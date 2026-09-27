//! The birthday-party delivery: how fast the held button spends the
//! delivery item, how the spent items turn into points and the points into
//! reward drops, where the drops land, and the clocks around the hold.
//!
//! Every value here is client law: the delivery site model and the party's
//! site data compute them on the client between two API calls. What the
//! server decides (the have-quantity, the synchronized points, the member
//! bonus rates, the base point, the drop upper limit) comes in as inputs.
//!
//! Frames: positions are in the source frame (x not reflected); the host
//! converts at its boundary.

use core::f32::consts::PI;

/// `BirthdayDeliveryStartWaitTime`: the delay between the pre-action and the
/// first loop step is a frame count from the server config divided by this
/// frame rate (a single-precision division).
pub const START_WAIT_FRAMES_PER_SECOND: f32 = 60.0;

/// The start wait for a frame count, as the site model stores it.
pub fn start_wait_time(frames: i32) -> f32 {
    frames as f32 / START_WAIT_FRAMES_PER_SECOND
}

/// `BUTTON_INPUT_RESUME_TIME`: after the button is released the loop keeps
/// delivering until the time since the release is no longer below this.
/// The comparison reads the time accumulated on earlier frames, before this
/// frame's delta is added.
pub const RELEASE_WINDOW: f32 = 0.25;

/// `PrePlayerHarvestMotion`: the AutoMove destination threshold.
pub const APPROACH_THRESHOLD: f32 = 0.3;
/// `PrePlayerHarvestMotion`: the linear look-at toward the place after the
/// AutoMove, in seconds.
pub const APPROACH_FACE_SECONDS: f32 = 0.3;
/// A direction shorter than this is the zero vector.
pub const DIRECTION_EPSILON: f32 = 1e-5;

/// `PlayTotalRewardAnimation`: the wait before the honor reward state, in
/// whole milliseconds.
pub const HONOR_REWARD_DELAY_MS: i32 = 500;
/// `PlayerAvatarDeliveryHonorRewardState.Initialize`: the look-at toward the
/// camera, in seconds (eased OutCubic).
pub const HONOR_FACE_SECONDS: f32 = 1.5;
/// `DeliveryHonorRewardCameraState`: both tweens of its sequence last this
/// long (eased OutQuint).
pub const HONOR_CAMERA_SECONDS: f32 = 3.0;
/// `DeliveryHonorRewardCameraState`: the pitch the sequence ends at, in
/// degrees.
pub const HONOR_CAMERA_END_PITCH: f32 = 40.0;
/// `DeliveryHonorRewardCameraState.OnUpdate`: the look-at moves toward the
/// player by this fraction per sixtieth of a second.
pub const HONOR_CAMERA_FOLLOW_PER_FRAME: f32 = 0.1;

/// `ChangeDropItemRangeData`: the near edge of the drop annulus once a
/// delivery has a base angle.
pub const DROP_MIN_DISTANCE: f32 = 1.5;
/// `UpdateDropItemRangeData`: the widening span, 17 pi / 18, as the
/// single-precision word the law multiplies by (one unit in the last place
/// above the rounded quotient).
pub const DROP_ANGLE_SPAN: f32 = f32::from_bits(0x403d_e44f);
/// `UpdateDropItemRangeData`: the narrowest half-width, pi / 18, as the
/// word the law adds (one unit above the rounded quotient).
pub const DROP_ANGLE_MIN: f32 = f32::from_bits(0x3e32_b8c3);

/// The delivery rate of the site model (`CurrentDeliveryCostCountPerSecond`)
/// and the fractional carry between loop steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeliveryRate {
    /// Items per second.
    pub current: f32,
    pub min: f32,
    pub max: f32,
    /// Items per second, per second.
    pub acceleration: f32,
    /// `_carryOverCostCountPerExecuteLoop`.
    pub carry: f32,
}

impl DeliveryRate {
    /// `SetupDeliveryData`: the three server config values.
    pub fn new(min: f32, max: f32, acceleration: f32) -> Self {
        Self {
            current: 0.0,
            min,
            max,
            acceleration,
            carry: 0.0,
        }
    }

    /// `SetExecuteDeliveryData`: the carry clears and the rate starts at its
    /// minimum.
    pub fn start(&mut self) {
        self.carry = 0.0;
        self.current = self.min;
    }

    /// `UpdateDeliveryCostCountPerSecond`: below the maximum the rate grows
    /// by acceleration times the frame time and is capped at the maximum; at
    /// or above it nothing changes.
    pub fn accelerate(&mut self, dt: f32) {
        if self.current < self.max {
            let next = self.current + self.acceleration * dt;
            self.current = if next < self.max { next } else { self.max };
        }
    }

    /// `DeliverySiteModel.CalculateDeliveryData`: the item count this loop
    /// step asks for. While nothing is spent yet in this delivery the first
    /// step asks for one item; afterwards the rate times the frame time is
    /// added to the carry, the count is its floor and the carry keeps the
    /// fraction.
    pub fn step_count(&mut self, unsynchronized_cost: i32, dt: f32) -> i32 {
        if unsynchronized_cost < 1 {
            return 1;
        }
        let x = self.carry + self.current * dt;
        let floor = x.floor();
        let count = if floor == f32::INFINITY {
            i32::MIN
        } else {
            floor as i32
        };
        self.carry = x - count as f32;
        count
    }

    /// `ExecuteDeliveryEndAction`: the rate reads zero once the delivery
    /// ends.
    pub fn stop(&mut self) {
        self.current = 0.0;
    }
}

/// `Math.Round(double)`: halves go to the even neighbour.
pub fn round_half_even(x: f64) -> f64 {
    let floor = x.floor();
    let fraction = x - floor;
    if fraction > 0.5 {
        floor + 1.0
    } else if fraction < 0.5 {
        floor
    } else if floor % 2.0 == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

/// `BirthdayPartySiteData`: one party's tally on this site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartyTally {
    /// The have-quantity of the delivery item the last sync read.
    pub synchronized_cost_quantity: i32,
    /// Items spent since the last sync.
    pub unsynchronized_cost: i32,
    /// The party's delivery points as the server last replied them.
    pub synchronized_points: i32,
    /// Points earned since the last sync.
    pub unsynchronized_points: i32,
    /// `birthday_party_delivery_base_point`.
    pub base_point: i32,
    /// `MemberBonus`: the sum of the bonus rates the user's cards earn.
    pub member_bonus: f32,
    /// The last reward row's requirement, or 1 without rows.
    pub reward_loop_requirement: i32,
    /// `birthday_party_delivery_reward_drop_upper_limit`.
    pub max_drop_item_count: i32,
}

impl PartyTally {
    /// `CurrentCostItemQuantity`.
    pub fn remaining(&self) -> i32 {
        self.synchronized_cost_quantity - self.unsynchronized_cost
    }

    /// `CanDelivery`: some item is left to spend.
    pub fn can_delivery(&self) -> bool {
        self.remaining() > 0
    }

    /// `CurrentDeliveryPoint`.
    pub fn current_points(&self) -> i32 {
        self.unsynchronized_points + self.synchronized_points
    }

    /// `TotalDropMysekaiMaterialCount`: whole reward loops the points reach
    /// (integer division).
    pub fn total_drop_count(&self) -> i32 {
        self.current_points() / self.reward_loop_requirement
    }

    /// `BirthdayPartySiteData.CalculateDeliveryData(count)`: the count is
    /// capped at what is left; nothing happens below one item. The spent
    /// count grows by it, and the points by the base point times the count
    /// (an integer product), times one plus the member bonus in single
    /// precision, rounded half to even as a double and added as an integer.
    /// Returns the items spent.
    pub fn spend(&mut self, count: i32) -> i32 {
        let remaining = self.remaining();
        let n = if remaining > count { count } else { remaining };
        if n < 1 {
            return 0;
        }
        self.unsynchronized_cost += n;
        let points = (self.member_bonus + 1.0) * (self.base_point.wrapping_mul(n)) as f32;
        let rounded = round_half_even(points as f64);
        let added = if rounded == f64::INFINITY {
            i32::MIN
        } else {
            rounded as i32
        };
        self.unsynchronized_points = self.unsynchronized_points.wrapping_add(added);
        n
    }
}

/// `MemberBonus`: each bonus row whose card the user owns adds its rate
/// divided by 100, summed in single precision in row order.
pub fn member_bonus(owned_rates: &[i32]) -> f32 {
    owned_rates
        .iter()
        .fold(0.0f32, |sum, rate| sum + *rate as f32 / 100.0)
}

/// The reward loop requirement: the last reward row's requirement, or 1
/// when the party has no reward rows.
pub fn reward_loop_requirement(requirements_in_row_order: &[i32]) -> i32 {
    requirements_in_row_order.last().copied().unwrap_or(1)
}

/// `IsWithinDropItemLimit`: the party's drop count (synchronized and not)
/// is below the upper limit.
pub fn is_within_drop_item_limit(drop_count: i32, max_drop_item_count: i32) -> bool {
    drop_count < max_drop_item_count
}

/// `DeliverySiteDropRangeData`: the annulus sector the drops land in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropRange {
    /// Radians.
    pub base_angle: f32,
    /// Radians, each side of the base angle.
    pub angle_range: f32,
    pub min_distance: f32,
    pub max_distance: f32,
}

impl DropRange {
    /// `CreateAllRangeData`: the full circle (base 0,
    /// range pi on each side), from 1.5 to 4.0 m.
    pub const ALL: DropRange = DropRange {
        base_angle: 0.0,
        angle_range: PI,
        min_distance: 1.5,
        max_distance: 4.0,
    };

    /// `ChangeDropItemRangeData(base)`: the base angle and the near edge,
    /// then the width and the far edge by [`Self::update`].
    pub fn change(
        &mut self,
        base_angle: f32,
        max_drop_item_count: i32,
        synchronized_drops: i32,
        unsynchronized_drops: i32,
    ) {
        self.base_angle = base_angle;
        self.min_distance = DROP_MIN_DISTANCE;
        self.update(
            max_drop_item_count,
            synchronized_drops,
            unsynchronized_drops,
        );
    }

    /// `UpdateDropItemRangeData`: the free slots are the upper limit minus
    /// the synchronized drops. With free slots, `t` = unsynchronized drops
    /// over free slots (single precision), clamped to [0, 1]; the sector
    /// widens from pi/18 to pi on each side and the far edge from 3 to 4 m
    /// as `t` grows. Without free slots the sector is pi wide and 4 m deep.
    pub fn update(
        &mut self,
        max_drop_item_count: i32,
        synchronized_drops: i32,
        unsynchronized_drops: i32,
    ) {
        let free = max_drop_item_count - synchronized_drops;
        if free > 0 {
            let ratio = unsynchronized_drops as f32 / free as f32;
            let t = if ratio >= 0.0 { ratio.min(1.0) } else { 0.0 };
            self.angle_range = t * DROP_ANGLE_SPAN + DROP_ANGLE_MIN;
            self.max_distance = t + 3.0;
        } else {
            self.angle_range = PI;
            self.max_distance = 4.0;
        }
    }

    /// `GetDropPosition`: the angle bounds `[base - range, base + range]`.
    pub fn angle_bounds(&self) -> (f32, f32) {
        (
            self.base_angle - self.angle_range,
            self.base_angle + self.angle_range,
        )
    }
}

/// `Random.Range(min, max)` for floats, fed a uniform `u` in [0, 1).
pub fn random_range(min: f32, max: f32, u: f32) -> f32 {
    min + (max - min) * u
}

/// `GetDropPosition`: from the place, `distance` along `angle` on x/z.
/// Returns the source-frame (x, z) before the walk-field snap.
pub fn drop_point(place_xz: [f32; 2], angle: f32, distance: f32) -> [f32; 2] {
    [
        place_xz[0] + angle.cos() * distance,
        place_xz[1] + angle.sin() * distance,
    ]
}

/// `Vector2.SignedAngle(Vector2.right, v)` in degrees: the unsigned angle
/// from the dot product (zero when the length product is below 1e-15), its
/// sign that of `v.y` (non-negative counts as positive).
pub fn signed_angle_from_right_deg(v: [f32; 2]) -> f32 {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    let unsigned = if length < 1e-15 {
        0.0
    } else {
        let cos = (v[0] / length).clamp(-1.0, 1.0);
        (cos as f64).acos() as f32 * 57.295_78
    };
    if v[1] >= 0.0 {
        unsigned
    } else {
        -unsigned
    }
}

/// `ExecuteDeliveryPreAction`: the base angle of the annulus, from the
/// place to the player on x/z, in radians wrapped into [0, 2 pi): the
/// signed angle divided by 180, times pi, plus 2 pi when negative.
pub fn base_angle(player_xz: [f32; 2], place_xz: [f32; 2]) -> f32 {
    let deg = signed_angle_from_right_deg([player_xz[0] - place_xz[0], player_xz[1] - place_xz[1]]);
    let rad = deg / 180.0 * PI;
    if rad < 0.0 {
        rad + 2.0 * PI
    } else {
        rad
    }
}

/// `PrePlayerHarvestMotion`: the AutoMove destination, `radius` from the
/// place toward the player (3D; the zero direction when the player stands
/// within 1e-5 of the place).
pub fn approach_target(place: [f32; 3], player: [f32; 3], radius: f32) -> [f32; 3] {
    let d = [
        player[0] - place[0],
        player[1] - place[1],
        player[2] - place[2],
    ];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let n = if length > DIRECTION_EPSILON {
        [d[0] / length, d[1] / length, d[2] / length]
    } else {
        [0.0, 0.0, 0.0]
    };
    [
        place[0] + n[0] * radius,
        place[1] + n[1] * radius,
        place[2] + n[2] * radius,
    ]
}

/// DOTween `Ease.OutCubic` on the normalised time.
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = t - 1.0;
    u * u * u + 1.0
}

/// DOTween `Ease.OutQuint` on the normalised time.
pub fn ease_out_quint(t: f32) -> f32 {
    let u = t - 1.0;
    u * u * u * u * u + 1.0
}

/// `DeliveryHonorRewardCameraState.OnUpdate`: the look-at blend of one
/// frame, `dt / (1/60) * 0.1` clamped to [0, 1] (0 when negative).
pub fn honor_follow_factor(dt: f32) -> f32 {
    let v = dt / (1.0 / 60.0) * HONOR_CAMERA_FOLLOW_PER_FRAME;
    if v < 0.0 {
        0.0
    } else {
        v.min(1.0)
    }
}

/// A DOTween rotation change in degrees with `RotateMode.Fast`: the end
/// angle minus the start, taken the short way round.
pub fn fast_change_deg(from: f32, to: f32) -> f32 {
    let to = to.rem_euclid(360.0);
    let from = from.rem_euclid(360.0);
    let change = to - from;
    let abs = change.abs();
    if abs > 180.0 {
        if change > 0.0 {
            -(360.0 - abs)
        } else {
            360.0 - abs
        }
    } else {
        change
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    /// Start wait: the server's 30 frames make half a second; the rate law
    /// on sampled frames, worked by hand from `min(rate + a dt, max)` with
    /// (1, 180, 50): 1 -> 1.8333334 -> 2.6666667 at 1/60 s; one step of 4 s
    /// caps at 180; at the cap nothing changes.
    #[test]
    fn start_wait_and_rate_law() {
        assert_eq!(start_wait_time(30), 0.5);
        assert_eq!(start_wait_time(0), 0.0);
        let mut rate = DeliveryRate::new(1.0, 180.0, 50.0);
        rate.carry = 0.7;
        rate.start();
        assert_eq!((rate.current, rate.carry), (1.0, 0.0));
        rate.accelerate(1.0 / 60.0);
        assert!(
            close(rate.current, 1.0 + 50.0 / 60.0, 1e-6),
            "{}",
            rate.current
        );
        rate.accelerate(1.0 / 60.0);
        assert!(
            close(rate.current, 1.0 + 100.0 / 60.0, 1e-5),
            "{}",
            rate.current
        );
        rate.accelerate(4.0);
        assert_eq!(rate.current, 180.0);
        rate.accelerate(1.0);
        assert_eq!(rate.current, 180.0);
        rate.stop();
        assert_eq!(rate.current, 0.0);
    }

    /// The count and carry: the first step asks for one item whatever the
    /// rate; afterwards floor(carry + rate dt). At 50 items/s and 1/60 s:
    /// 0.8333 -> 0 (carry 0.8333), then 1.6667 -> 1 (carry 0.6667), then
    /// 1.5 -> 1 (carry 0.5).
    #[test]
    fn step_count_and_carry() {
        let mut rate = DeliveryRate::new(1.0, 180.0, 50.0);
        rate.start();
        rate.current = 50.0;
        assert_eq!(rate.step_count(0, 1.0 / 60.0), 1);
        assert_eq!(rate.carry, 0.0);
        assert_eq!(rate.step_count(1, 1.0 / 60.0), 0);
        assert!(close(rate.carry, 50.0 / 60.0, 1e-6));
        assert_eq!(rate.step_count(1, 1.0 / 60.0), 1);
        assert!(close(rate.carry, 100.0 / 60.0 - 1.0, 1e-5));
        assert_eq!(rate.step_count(2, 1.0 / 60.0), 1);
        assert!(
            close(rate.carry, 150.0 / 60.0 - 2.0, 1e-5),
            "{}",
            rate.carry
        );
    }

    /// Rounding half to even, the `Math.Round` default: 0.5 -> 0, 1.5 -> 2,
    /// 2.5 -> 2, 2.51 -> 3, 7.49 -> 7.
    #[test]
    fn round_half_even_samples() {
        for (x, want) in [
            (0.5, 0.0),
            (1.5, 2.0),
            (2.5, 2.0),
            (2.51, 3.0),
            (7.49, 7.0),
            (10.0, 10.0),
        ] {
            assert_eq!(round_half_even(x), want, "{x}");
        }
    }

    fn tally() -> PartyTally {
        PartyTally {
            synchronized_cost_quantity: 7,
            unsynchronized_cost: 0,
            synchronized_points: 9_950,
            unsynchronized_points: 0,
            base_point: 10,
            member_bonus: 0.25,
            reward_loop_requirement: 10_000,
            max_drop_item_count: 30,
        }
    }

    /// Accrual: 3 items at base 10 and bonus 0.25 give round(1.25 * 30 =
    /// 37.5) = 38 (37.5 is a half, 38 even); the count is capped at what is
    /// left (7 - 3 = 4 of a request for 9); nothing below one item; the
    /// reward count is the integer quotient of the points by the
    /// requirement.
    #[test]
    fn accrual_cap_and_reward_count() {
        let mut t = tally();
        assert!(t.can_delivery());
        assert_eq!(t.total_drop_count(), 0);
        assert_eq!(t.spend(3), 3);
        assert_eq!((t.unsynchronized_cost, t.unsynchronized_points), (3, 38));
        assert_eq!(t.current_points(), 9_988);
        assert_eq!(t.spend(9), 4);
        // 1.25 * 40 = 50.
        assert_eq!((t.unsynchronized_cost, t.unsynchronized_points), (7, 88));
        assert_eq!(t.total_drop_count(), 1);
        assert!(!t.can_delivery());
        assert_eq!(t.spend(1), 0);
        assert_eq!(t.unsynchronized_points, 88);
        // A half that rounds down: 1.25 * 10 * 1 = 12.5 -> 12.
        let mut u = tally();
        u.base_point = 10;
        u.spend(1);
        assert_eq!(u.unsynchronized_points, 12);
    }

    /// Member bonus rows add rate / 100; the requirement is the last row's,
    /// 1 without rows; the drop limit test is strict.
    #[test]
    fn bonus_requirement_and_limit() {
        assert!(close(member_bonus(&[50, 15, 15]), 0.8, 1e-6));
        assert_eq!(member_bonus(&[]), 0.0);
        assert_eq!(reward_loop_requirement(&[5_000, 10_000]), 10_000);
        assert_eq!(reward_loop_requirement(&[]), 1);
        assert!(is_within_drop_item_limit(29, 30));
        assert!(!is_within_drop_item_limit(30, 30));
    }

    /// The annulus law on sampled counts: all-range at start; a delivery
    /// sets the base and the 1.5 m edge; t = 0 gives pi/18 and 3 m, t = 0.5
    /// gives pi/18 + 17 pi/36 and 3.5 m, t >= 1 clamps to pi and 4 m; a
    /// full limit gives pi and 4 m.
    #[test]
    fn drop_range_law() {
        let mut range = DropRange::ALL;
        assert_eq!(
            range,
            DropRange {
                base_angle: 0.0,
                angle_range: PI,
                min_distance: 1.5,
                max_distance: 4.0
            }
        );
        range.change(1.0, 30, 10, 0);
        assert_eq!((range.base_angle, range.min_distance), (1.0, 1.5));
        assert!(close(range.angle_range, PI / 18.0, 1e-6));
        assert_eq!(range.angle_range, DROP_ANGLE_MIN);
        assert_eq!(range.max_distance, 3.0);
        range.update(30, 10, 10);
        assert!(close(
            range.angle_range,
            PI / 18.0 + 0.5 * 17.0 * PI / 18.0,
            1e-5
        ));
        assert_eq!(range.angle_range, 0.5 * DROP_ANGLE_SPAN + DROP_ANGLE_MIN);
        assert!(close(range.max_distance, 3.5, 1e-6));
        range.update(30, 10, 40);
        assert!(close(range.angle_range, PI, 1e-5));
        assert_eq!(range.max_distance, 4.0);
        range.update(30, 30, 0);
        assert_eq!((range.angle_range, range.max_distance), (PI, 4.0));
        let (lo, hi) = range.angle_bounds();
        assert_eq!((lo, hi), (1.0 - PI, 1.0 + PI));
    }

    /// The base angle for each quadrant (player relative to the place):
    /// +x 0, +z pi/2, -x pi, -z 3 pi/2; the drop point at that angle and
    /// distance; the approach target 1.4 m toward the player, and the zero
    /// direction on top of the place.
    #[test]
    fn angle_point_and_approach() {
        let place = [2.0, 5.0];
        assert!(close(base_angle([3.0, 5.0], place), 0.0, 1e-6));
        assert!(close(base_angle([2.0, 7.0], place), PI / 2.0, 1e-5));
        assert!(close(base_angle([0.0, 5.0], place), PI, 1e-5));
        assert!(close(base_angle([2.0, 4.0], place), 1.5 * PI, 1e-5));
        assert_eq!(base_angle(place, place), 0.0);
        let p = drop_point(place, PI / 2.0, 3.0);
        assert!(close(p[0], 2.0, 1e-5) && close(p[1], 8.0, 1e-5), "{p:?}");
        let target = approach_target([0.0, 0.0, 0.0], [3.0, 0.0, 4.0], 1.4);
        assert!(close(target[0], 0.84, 1e-6) && close(target[2], 1.12, 1e-6));
        assert_eq!(
            approach_target([1.0, 0.0, 1.0], [1.0, 0.0, 1.0], 1.4),
            [1.0, 0.0, 1.0]
        );
    }

    /// Eases at t = 0, 0.5, 1; the follow blend at 60 fps is 0.1 and caps at
    /// 1; the short-way rotation change.
    #[test]
    fn eases_follow_and_rotation() {
        assert_eq!((ease_out_cubic(0.0), ease_out_cubic(1.0)), (0.0, 1.0));
        assert!(close(ease_out_cubic(0.5), 0.875, 1e-6));
        assert!(close(ease_out_quint(0.5), 0.968_75, 1e-6));
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert!(close(honor_follow_factor(1.0 / 60.0), 0.1, 1e-6));
        assert_eq!(honor_follow_factor(1.0), 1.0);
        assert_eq!(honor_follow_factor(-0.1), 0.0);
        assert_eq!(fast_change_deg(350.0, 10.0), 20.0);
        assert_eq!(fast_change_deg(10.0, 350.0), -20.0);
        assert_eq!(fast_change_deg(0.0, 90.0), 90.0);
        assert_eq!(fast_change_deg(-90.0, 180.0), -90.0);
    }

    /// `Random.Range(float)` stand-in: the ends of the unit interval.
    #[test]
    fn random_range_ends() {
        assert_eq!(random_range(1.5, 4.0, 0.0), 1.5);
        assert!(close(random_range(1.5, 4.0, 0.5), 2.75, 1e-6));
    }
}
