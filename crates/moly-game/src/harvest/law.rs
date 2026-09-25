//! Pure rules of the harvest flow, each a transcription of one client method.
//!
//! Nothing here touches the ECS. Every function names the method it follows;
//! the value checks at the end compute the rule on sampled inputs and compare
//! with values worked from the same source reading.

use bevy::math::{Vec2, Vec3};

/// `Mathf.Rad2Deg` as the priority method multiplies it (the f32 word
/// 0x42652EE1 loaded next to the `acos` call).
const RAD2DEG: f32 = f32::from_bits(0x4265_2EE1);
/// The `Vector3.Angle` degenerate-length threshold loaded by the same method.
const ANGLE_EPSILON: f32 = f32::from_bits(0x2690_1D7D);

/// `HarvestPresenter.GetHarvestTargetPriority`: with `d` the object minus the
/// player and `f` the player's forward, both taken on x/z,
/// `|d|^2 * angle(f, d)` in degrees; the angle is 0 when `|f| |d|` is below
/// the threshold. The lowest value is the target. The angle's `acos` runs in
/// double on the single-precision cosine and is narrowed back before the two
/// single-precision products.
pub(crate) fn target_priority(player: Vec3, forward: Vec3, object: Vec3) -> f32 {
    let dx = object.x - player.x;
    let dz = object.z - player.z;
    let d2 = dx * dx + dz * dz;
    let length = ((forward.x * forward.x + forward.z * forward.z) * d2).sqrt();
    let mut angle = 0.0;
    if length >= ANGLE_EPSILON {
        let cosine = (forward.x * dx + forward.z * dz) / length;
        let clamped = if cosine < -1.0 { -1.0 } else { cosine.min(1.0) };
        angle = ((clamped as f64).acos() as f32) * RAD2DEG;
    }
    d2 * angle
}

/// `ObjectCollisionManager.IsInSideCircle`: the owner's position against the
/// target's position in three dimensions, inside when the distance is not
/// more than the target's radius.
pub(crate) fn inside_circle(owner: Vec3, target: Vec3, radius: f32) -> bool {
    owner.distance(target) <= radius
}

/// Tool type (`MysekaiToolType`): the two values the tool master uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ToolType {
    Pickaxe,
    Axe,
}

impl ToolType {
    pub(crate) fn from_word(word: &str) -> Option<Self> {
        match word {
            "pickaxe" => Some(Self::Pickaxe),
            "axe" => Some(Self::Axe),
            _ => None,
        }
    }

    /// The fragment `GetHarvestAnimationName` builds the clip name from.
    pub(crate) fn clip_word(self) -> &'static str {
        match self {
            Self::Pickaxe => "pickax",
            Self::Axe => "ax",
        }
    }
}

/// `HarvestUtility.MaterialTypeToToolType`: the table of ten entries in the
/// binary maps wood (0) to the axe, mineral (1) to the pickaxe and the other
/// eight fixture types to none; a type of ten or more throws.
pub(crate) fn tool_type_for(fixture_type: i32) -> Option<ToolType> {
    match fixture_type {
        0 => Some(ToolType::Axe),
        1 => Some(ToolType::Pickaxe),
        2..=9 => None,
        other => panic!("fixture type {other} is outside the tool-type table"),
    }
}

/// `harvestToolState`: 0 Start, 1 Loop, 2 End, 3 None.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolState {
    Start,
    Loop,
    End,
    None,
}

impl ToolState {
    fn suffix(self) -> &'static str {
        match self {
            Self::Start => "_s",
            Self::Loop => "_l",
            Self::End => "_e",
            Self::None => "_o",
        }
    }
}

/// `HarvestUtility.IsSustainableTool`: true only for the two level-five tools
/// (ids 5 and 10); they start with the Start state, every other tool with
/// Loop.
pub(crate) fn is_sustainable_tool(tool_id: i64) -> bool {
    matches!(tool_id, 5 | 10)
}

/// The tool fields the clip and the clock read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolClock {
    pub(crate) tool_type: ToolType,
    pub(crate) level: i32,
    /// Master `coolTimeMicroSeconds` (the field holds milliseconds).
    pub(crate) cool_time: f32,
}

/// `HarvestUtility.GetHarvestAnimationName`, lower case as the client builds
/// it. Wood and mineral with a tool: `mov_u000_site_` + ax / pickax + the
/// level in two digits + the state suffix; the other kinds name one clip.
pub(crate) fn harvest_clip_name(fixture_type: i32, tool: Option<ToolClock>, state: ToolState) -> Option<String> {
    Some(match fixture_type {
        0 | 1 => {
            let tool = tool?;
            format!(
                "mov_u000_site_{}{:02}{}",
                tool.tool_type.clip_word(),
                tool.level,
                state.suffix()
            )
        }
        2 => "mov_u000_site_pick001_o".to_owned(),
        3 | 4 => "mov_u000_site_treasure01_o".to_owned(),
        5 => "mov_u000_site_pick04_o".to_owned(),
        6 => "mov_u000_site_listen01_o".to_owned(),
        7 => "mov_u000_site_toolbox01_o".to_owned(),
        8 => "mov_u000_site_barrel01_o".to_owned(),
        9 => "mov_u000_site_birthday_plant01_o".to_owned(),
        _ => return None,
    })
}

/// `HarvestUtility.GetHarvestActionTime`: with a tool in the Loop state the
/// cool-down `coolTimeMicroSeconds / 1000` (a value not above zero logs and
/// becomes 1.0); otherwise the length of the state's clip.
pub(crate) fn harvest_action_time(tool: Option<ToolClock>, state: ToolState, clip_length: f32) -> f32 {
    match tool {
        Some(tool) if state == ToolState::Loop => {
            let value = tool.cool_time / 1000.0;
            if value <= 0.0 {
                1.0
            } else {
                value
            }
        }
        _ => clip_length,
    }
}

/// The client's stamina triple (`StaminaData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamina {
    pub(crate) normal: i32,
    pub(crate) enhance: i32,
    pub(crate) boost: i32,
}

impl Stamina {
    /// `StaminaData.Sum`: enhance + normal + boost.
    pub(crate) fn sum(self) -> i32 {
        self.enhance + self.normal + self.boost
    }

    /// `StaminaData.IsEmptyStamina`.
    pub(crate) fn is_empty(self) -> bool {
        self.normal <= 0 && self.boost <= 0 && self.enhance < 1
    }

    /// `StaminaData.HasBoostOrEnhance`.
    pub(crate) fn has_boost_or_enhance(self) -> bool {
        self.boost > 0 || self.enhance > 0
    }

    /// `HarvestPlayerModel.DecreaseStamina`: the amount comes out of boost
    /// first; what boost cannot cover (a non-positive remainder) goes to the
    /// normal pool while enhance is below one, else to enhance, and boost
    /// becomes zero.
    pub(crate) fn decrease(&mut self, amount: i32) {
        let remainder = self.boost - amount;
        if remainder <= 0 {
            if self.enhance < 1 {
                self.normal += remainder;
            } else {
                self.enhance += remainder;
            }
            self.boost = 0;
        } else {
            self.boost = remainder;
        }
    }
}

/// `HarvestPlayerPresenter.GetAnimationSpeed`: the boost speed (FloatConfigs
/// key 91) while enhance is positive or boost is at least one, else 1.0.
pub(crate) fn animation_speed(stamina: Stamina, boost_speed: f32) -> f32 {
    if stamina.enhance > 0 || stamina.boost >= 1 {
        boost_speed
    } else {
        1.0
    }
}

/// `HarvestPresenter.CanAttackRemainStamina`: with a tool and a target that
/// is on its last attack or still has hp, the sum must cover the tool's power
/// or the remaining hp; otherwise it must cover the last-attack stamina.
pub(crate) fn can_attack_remain_stamina(
    stamina: Stamina,
    attack_power: Option<i32>,
    is_last_attack: bool,
    hp: i32,
    last_attack_stamina: i32,
) -> bool {
    let sum = stamina.sum();
    match attack_power {
        Some(power) if is_last_attack || hp > 0 => sum >= power || sum >= hp,
        _ => sum >= last_attack_stamina,
    }
}

/// The inputs `HarvestPresenter.CanContinueAction` reads, in its order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContinueInputs {
    pub(crate) long_tap: bool,
    pub(crate) sustain: bool,
    pub(crate) has_target: bool,
    pub(crate) target_harvested: bool,
    pub(crate) auto_changed_tool: bool,
    /// The selected tool's quantity; `None` without a tool.
    pub(crate) tool_quantity: Option<i32>,
    pub(crate) stamina_empty: bool,
    pub(crate) can_attack: bool,
}

/// `HarvestPresenter.CanContinueAction`: a held button or a press queued
/// during the cool-down (the queued press is consumed here); then a live,
/// unharvested target, no automatic tool change, a selected tool with a
/// quantity of at least one, stamina that is not empty and enough for the
/// next attack. Returns the answer and whether the queued press remains.
pub(crate) fn can_continue_action(inputs: ContinueInputs) -> (bool, bool) {
    if !(inputs.long_tap || inputs.sustain) {
        return (false, inputs.sustain);
    }
    let sustain = false;
    let ok = inputs.has_target
        && !inputs.target_harvested
        && !inputs.auto_changed_tool
        && inputs.tool_quantity.is_some_and(|quantity| quantity >= 1)
        && !inputs.stamina_empty
        && inputs.can_attack;
    (ok, sustain)
}

/// `HarvestObjectModel.UpdateHp` on a copy of the three fields it writes;
/// see `HarvestObject::update_hp`, which carries the same body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HpModel {
    pub(crate) hp: i32,
    pub(crate) prev_hp: i32,
    pub(crate) harvested: bool,
    pub(crate) is_last_attack: bool,
    pub(crate) last_attack_stamina: i32,
}

impl HpModel {
    pub(crate) fn update_hp(&mut self, damage: i32) -> i32 {
        let prev = self.hp;
        self.prev_hp = prev;
        if prev < 1 {
            if prev == 0 && !self.harvested {
                self.is_last_attack = true;
                self.harvested = true;
                return self.last_attack_stamina;
            }
        } else if !self.harvested {
            if 0 < prev - damage {
                self.hp = prev - damage;
                return damage;
            }
            self.hp = 0;
            return prev;
        }
        0
    }
}

/// DOTween `Punch` point list: `(int)(vibrato * duration)` iterations (at
/// least two); segment `i` lasts `duration * (i + 1) / n`, all scaled so they
/// sum to the duration; the first point is the direction, then alternating
/// negated and positive clamps of it to a strength that decays by
/// `|direction| / n` per point, the last point zero. Points are relative to
/// the start value.
pub(crate) fn punch_points(direction: Vec3, duration: f32, vibrato: i32, elasticity: f32) -> (Vec<Vec3>, Vec<f32>) {
    let elasticity = elasticity.clamp(0.0, 1.0);
    let product = vibrato as f32 * duration;
    let mut count = if product == f32::INFINITY { i32::MIN } else { product as i32 };
    if count < 3 {
        count = 2;
    }
    let n = count as usize;
    let mut strength = direction.length();
    let decay = strength / count as f32;
    let (durations, _) = iteration_durations(n, duration, true);
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        if i + 1 < n {
            let point = if i == 0 {
                direction
            } else if i % 2 == 1 {
                -clamp_magnitude(direction, strength * elasticity)
            } else {
                clamp_magnitude(direction, strength)
            };
            points.push(point);
            strength -= decay;
        } else {
            points.push(Vec3::ZERO);
        }
    }
    (points, durations)
}

/// DOTween `Shake` with a scalar strength, `ignoreZAxis` on and the Full
/// randomness mode (the only mode of the game's DOTween build): the first
/// angle is one draw in [0, 360); each later point turns by 180 minus a draw
/// in [-randomness, randomness). With `fade_out` the durations grow like the
/// punch's and the magnitude decays by `strength / n` per point.
pub(crate) fn shake_points(
    duration: f32,
    strength: f32,
    vibrato: i32,
    randomness: f32,
    fade_out: bool,
    mut draw: impl FnMut(f32, f32) -> f32,
) -> (Vec<Vec3>, Vec<f32>) {
    let mut count = (vibrato as f32 * duration) as i32;
    if count < 2 {
        count = 2;
    }
    let n = count as usize;
    let mut magnitude = strength;
    let decay = magnitude / count as f32;
    let (durations, _) = iteration_durations(n, duration, fade_out);
    let mut angle = draw(0.0, 360.0);
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        if i + 1 < n {
            if i > 0 {
                angle = angle - 180.0 + draw(-randomness, randomness);
            }
            // `DOTweenUtils.Vector3FromAngle`: degrees times `Mathf.Deg2Rad`.
            let radians = angle * 0.017_453_292;
            points.push(Vec3::new(magnitude * radians.cos(), magnitude * radians.sin(), 0.0));
            if fade_out {
                magnitude -= decay;
            }
        } else {
            points.push(Vec3::ZERO);
        }
    }
    (points, durations)
}

fn iteration_durations(n: usize, duration: f32, grow: bool) -> (Vec<f32>, f32) {
    let mut durations = Vec::with_capacity(n);
    let mut sum = 0.0f32;
    for i in 0..n {
        let part = if grow {
            ((i + 1) as f32 / n as f32) * duration
        } else {
            duration / n as f32
        };
        sum += part;
        durations.push(part);
    }
    let multiplier = duration / sum;
    for part in &mut durations {
        *part *= multiplier;
    }
    (durations, sum)
}

fn clamp_magnitude(v: Vec3, max: f32) -> Vec3 {
    if v.length_squared() > max * max {
        v.normalize() * max
    } else {
        v
    }
}

/// The segment ease of a point-list tween: punches force OutQuad, shakes
/// Linear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentEase {
    OutQuad,
    Linear,
}

/// A running DOTween point-list tween (`Vector3ArrayPlugin`). Startup adds
/// the start value to every point; segment `i` runs from the previous point
/// (the start value for the first) to point `i`.
#[derive(Clone, Debug)]
pub(crate) struct PointTween {
    relative: Vec<Vec3>,
    durations: Vec<f32>,
    ease: SegmentEase,
    start: Option<Vec3>,
    pub(crate) elapsed: f32,
}

impl PointTween {
    pub(crate) fn new(points: (Vec<Vec3>, Vec<f32>), ease: SegmentEase) -> Self {
        Self {
            relative: points.0,
            durations: points.1,
            ease,
            start: None,
            elapsed: 0.0,
        }
    }

    pub(crate) fn duration(&self) -> f32 {
        self.durations.iter().sum()
    }

    pub(crate) fn done(&self) -> bool {
        self.elapsed >= self.duration()
    }

    /// Advance and return the value to set. The first call captures the
    /// start value (the tween's startup).
    pub(crate) fn advance(&mut self, current: Vec3, dt: f32) -> Vec3 {
        let start = *self.start.get_or_insert(current);
        self.elapsed += dt;
        let elapsed = self.elapsed.min(self.duration());
        let mut from = start;
        let mut before = 0.0f32;
        let mut count = 0.0f32;
        for (index, (point, segment)) in self.relative.iter().zip(&self.durations).enumerate() {
            count += *segment;
            let to = start + *point;
            if elapsed > count && index + 1 < self.relative.len() {
                before += *segment;
                from = to;
                continue;
            }
            let t = if *segment > 0.0 { ((elapsed - before) / segment).clamp(0.0, 1.0) } else { 1.0 };
            let eased = match self.ease {
                SegmentEase::OutQuad => -t * (t - 2.0),
                SegmentEase::Linear => t,
            };
            return from + (to - from) * eased;
        }
        start
    }
}

/// `TimeSpan.FromSeconds` as a `UniTask.Delay` holds it (whole milliseconds,
/// half away from zero, back to a float).
pub(crate) fn delay_seconds(x: f64) -> f32 {
    crate::site_move::timeline::delay_seconds(x)
}

/// Yaw that looks along `d` in the product frame (the player's facing law:
/// `atan2(x, z)`).
pub(crate) fn facing_yaw(d: Vec2) -> f32 {
    d.x.atan2(d.y)
}

/// `DORotate(..., RotateMode.Fast)` on the yaw alone: the change is wrapped
/// into [-180, 180) degrees, then eased linearly.
pub(crate) fn fast_yaw(from: f32, to: f32, t: f32) -> f32 {
    let mut change = (to - from).rem_euclid(std::f32::consts::TAU);
    if change > std::f32::consts::PI {
        change -= std::f32::consts::TAU;
    }
    from + change * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// Priority on sampled poses. The method: `|d|^2` times the angle in
    /// degrees between the forward and `d` on x/z; values worked by hand.
    #[test]
    fn target_priority_on_sampled_poses() {
        let player = Vec3::ZERO;
        let forward = Vec3::Z;
        // Straight ahead: angle 0.
        assert_eq!(target_priority(player, forward, Vec3::new(0.0, 5.0, 2.0)), 0.0);
        // At 90 degrees, 1 m away: 1 * 90.
        let side = target_priority(player, forward, Vec3::new(1.0, 0.0, 0.0));
        assert!((side - 90.0).abs() < 1e-3, "{side}");
        // Behind, 2 m: 4 * 180.
        let behind = target_priority(player, forward, Vec3::new(0.0, 0.0, -2.0));
        assert!((behind - 720.0).abs() < 1e-2, "{behind}");
        // A near object at 90 degrees wins over a far one straight ahead
        // only while |d|^2 * angle is lower: 0.25 * 90 = 22.5 vs 0.
        let near = target_priority(player, forward, Vec3::new(0.5, 0.0, 0.0));
        assert!((near - 22.5).abs() < 1e-3);
        // The degenerate case: player on the object.
        assert_eq!(target_priority(player, forward, player), 0.0);
        // y is ignored.
        assert_eq!(
            target_priority(player, forward, Vec3::new(1.0, 9.0, 0.0)),
            target_priority(player, forward, Vec3::new(1.0, -9.0, 0.0))
        );
    }

    /// The radius test is three-dimensional: a height step counts.
    #[test]
    fn inside_circle_is_three_dimensional() {
        assert!(inside_circle(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0));
        assert!(!inside_circle(Vec3::ZERO, Vec3::new(1.0, 0.1, 0.0), 1.0));
    }

    /// Clip names and the swing clock per CN tool row (master tool levels
    /// 1, 2, 4, 5 with cool times 1200, 1200, 1000, 500).
    #[test]
    fn cool_down_and_clip_schedule_per_tool() {
        let tool = |tool_type, level, cool_time| ToolClock { tool_type, level, cool_time };
        let ax1 = tool(ToolType::Axe, 1, 1200.0);
        assert_eq!(harvest_clip_name(0, Some(ax1), ToolState::Loop).unwrap(), "mov_u000_site_ax01_l");
        assert_eq!(harvest_clip_name(0, Some(ax1), ToolState::End).unwrap(), "mov_u000_site_ax01_e");
        let pick5 = tool(ToolType::Pickaxe, 5, 500.0);
        assert_eq!(harvest_clip_name(1, Some(pick5), ToolState::Start).unwrap(), "mov_u000_site_pickax05_s");
        assert_eq!(harvest_clip_name(2, None, ToolState::None).unwrap(), "mov_u000_site_pick001_o");
        assert_eq!(harvest_clip_name(5, None, ToolState::None).unwrap(), "mov_u000_site_pick04_o");
        assert_eq!(harvest_clip_name(8, None, ToolState::None).unwrap(), "mov_u000_site_barrel01_o");
        assert!(harvest_clip_name(0, None, ToolState::None).is_none());
        // Loop: the cool time; other states: the clip length.
        assert_eq!(harvest_action_time(Some(ax1), ToolState::Loop, 9.0), 1.2);
        assert_eq!(harvest_action_time(Some(tool(ToolType::Axe, 4, 1000.0)), ToolState::Loop, 9.0), 1.0);
        assert_eq!(harvest_action_time(Some(pick5), ToolState::Loop, 9.0), 0.5);
        assert_eq!(harvest_action_time(Some(ax1), ToolState::End, 0.466_666_7), 0.466_666_7);
        assert_eq!(harvest_action_time(None, ToolState::None, 1.25), 1.25);
        assert_eq!(harvest_action_time(Some(tool(ToolType::Axe, 1, 0.0)), ToolState::Loop, 9.0), 1.0);
        // The cool-down Delay divides by the speed, the animation wait not.
        assert_eq!(delay_seconds((1.2f32 / 1.5f32) as f64), 0.8);
        assert_eq!(delay_seconds(1.2f32 as f64), 1.2);
        assert!(is_sustainable_tool(5) && is_sustainable_tool(10));
        assert!(!is_sustainable_tool(1) && !is_sustainable_tool(6) && !is_sustainable_tool(9));
    }

    /// `UpdateHp` for each CN tool power on a 90-hp tree: damaging hits, then
    /// the last attack that returns the last-attack stamina (10).
    #[test]
    fn update_hp_sequences_per_tool_power() {
        for (power, expected) in [
            (20, vec![20, 20, 20, 20, 10, 10]),
            (25, vec![25, 25, 25, 15, 10]),
            (35, vec![35, 35, 20, 10]),
            (40, vec![40, 40, 10, 10]),
        ] {
            let mut model = HpModel { hp: 90, prev_hp: 90, harvested: false, is_last_attack: false, last_attack_stamina: 10 };
            let mut returned = Vec::new();
            while !model.harvested {
                returned.push(model.update_hp(power));
            }
            assert_eq!(returned, expected, "power {power}");
            assert!(model.is_last_attack);
            assert_eq!(model.update_hp(power), 0, "a harvested model returns 0");
        }
        // hp 0 kinds: the first hit is the last attack.
        let mut plant = HpModel { hp: 0, prev_hp: 0, harvested: false, is_last_attack: false, last_attack_stamina: 20 };
        assert_eq!(plant.update_hp(1), 20);
        assert!(plant.harvested);
    }

    /// `CanContinueAction` in its order; the queued press is consumed only
    /// when a press (held or queued) is there.
    #[test]
    fn can_continue_action_rules() {
        let base = ContinueInputs {
            long_tap: true,
            sustain: false,
            has_target: true,
            target_harvested: false,
            auto_changed_tool: false,
            tool_quantity: Some(1),
            stamina_empty: false,
            can_attack: true,
        };
        assert_eq!(can_continue_action(base), (true, false));
        assert_eq!(can_continue_action(ContinueInputs { long_tap: false, ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { long_tap: false, sustain: true, ..base }), (true, false));
        assert_eq!(can_continue_action(ContinueInputs { tool_quantity: None, sustain: true, ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { tool_quantity: Some(0), ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { target_harvested: true, ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { auto_changed_tool: true, ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { stamina_empty: true, ..base }), (false, false));
        assert_eq!(can_continue_action(ContinueInputs { can_attack: false, ..base }), (false, false));
    }

    /// Stamina debit: boost first, then normal (enhance below one) or
    /// enhance.
    #[test]
    fn stamina_debit_and_gate() {
        let mut s = Stamina { normal: 1000, enhance: 0, boost: 0 };
        s.decrease(20);
        assert_eq!(s, Stamina { normal: 980, enhance: 0, boost: 0 });
        let mut s = Stamina { normal: 100, enhance: 0, boost: 15 };
        s.decrease(20);
        assert_eq!(s, Stamina { normal: 95, enhance: 0, boost: 0 });
        let mut s = Stamina { normal: 100, enhance: 50, boost: 30 };
        s.decrease(20);
        assert_eq!(s, Stamina { normal: 100, enhance: 50, boost: 10 });
        let mut s = Stamina { normal: 100, enhance: 50, boost: 0 };
        s.decrease(20);
        assert_eq!(s, Stamina { normal: 100, enhance: 30, boost: 0 });
        assert!(Stamina { normal: 0, enhance: 0, boost: 0 }.is_empty());
        assert!(!Stamina { normal: 0, enhance: 1, boost: 0 }.is_empty());
        // The gate: a 20-power axe on hp 90 needs 20 or 90; on hp 0 before
        // the last attack the last-attack stamina.
        let low = Stamina { normal: 19, enhance: 0, boost: 0 };
        assert!(!can_attack_remain_stamina(low, Some(20), false, 90, 10));
        assert!(can_attack_remain_stamina(low, Some(20), false, 15, 10));
        assert!(can_attack_remain_stamina(low, Some(20), false, 0, 10));
        assert!(!can_attack_remain_stamina(Stamina { normal: 9, ..low }, Some(20), false, 0, 10));
        assert!(can_attack_remain_stamina(low, None, false, 0, 19));
        assert_eq!(animation_speed(Stamina { normal: 5, enhance: 0, boost: 1 }, 1.5), 1.5);
        assert_eq!(animation_speed(Stamina { normal: 5, enhance: 0, boost: 0 }, 1.5), 1.0);
    }

    /// `DOPunchPosition((0.01, 0, 0), 0.7, 6, 1)`: 4 points, durations
    /// 0.07 / 0.14 / 0.21 / 0.28 (0.175 i scaled by 0.7 / 1.75).
    #[test]
    fn punch_points_for_the_harvest_punch() {
        let (points, durations) = punch_points(Vec3::new(0.01, 0.0, 0.0), 0.7, 6, 1.0);
        assert_eq!(points.len(), 4);
        let expected = [0.01, -0.0075, 0.005, 0.0];
        for (point, x) in points.iter().zip(expected) {
            assert!((point.x - x).abs() < 1e-7, "{points:?}");
            assert_eq!(point.y, 0.0);
        }
        for (d, e) in durations.iter().zip([0.07, 0.14, 0.21, 0.28]) {
            assert!((d - e).abs() < 1e-6, "{durations:?}");
        }
        let mut tween = PointTween::new((points, durations), SegmentEase::OutQuad);
        let start = Vec3::new(3.0, 0.0, 0.0);
        // Half of the first segment, OutQuad: 0.75 of 0.01.
        let v = tween.advance(start, 0.035);
        assert!((v.x - (3.0 + 0.0075)).abs() < 1e-6, "{v}");
        let end = tween.advance(start, 1.0);
        assert_eq!(end, start);
        assert!(tween.done());
    }

    /// `ShakeCamera(0.03, 0.2, 3, fadeOut)`: DOTween shake of 0.2 / 3 s,
    /// two iterations (3 * 0.0667 truncates to 0), the first point at the
    /// drawn angle with the full magnitude, the second zero.
    #[test]
    fn shake_points_for_the_harvest_shake() {
        let (points, durations) = shake_points(0.2 / 3.0, 0.03, 3, 90.0, true, |_, _| 90.0);
        assert_eq!(points.len(), 2);
        assert!(points[0].x.abs() < 1e-6 && (points[0].y - 0.03).abs() < 1e-7);
        assert_eq!(points[1], Vec3::ZERO);
        let total: f32 = durations.iter().sum();
        assert!((total - 0.2 / 3.0).abs() < 1e-6);
        assert!((durations[0] * 2.0 - durations[1]).abs() < 1e-6);
    }

    #[test]
    fn fast_yaw_takes_the_short_way() {
        let a = 170f32.to_radians();
        let b = (-170f32).to_radians();
        let mid = fast_yaw(a, b, 0.5);
        assert!((mid.to_degrees() - 180.0).abs() < 1e-3, "{}", mid.to_degrees());
    }
}
