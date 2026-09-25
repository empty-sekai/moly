//! Source values of the three door moves (`HomeToMyRoomActionState`,
//! `MyRoomToHomeActionState`, `MyRoomToMyRoomActionState`) and of the circle
//! wipe they drive (`DoorTransitioner` / `SiteWipeController`), as pure
//! functions. Nothing here touches the ECS.
//!
//! Where the values come from, in words:
//! - `DoorTransitioner.FadeOut` calls `SiteWipeController.FadeOut(1.0,
//!   Vector2.zero, callback, 15, 0.75)`: the material's `_Scale` goes from
//!   0.75 to 15 in one second with DOTween's `InCirc`. `FadeIn` is the mirror
//!   call (15 to 0.75, `OutCirc`). `SetPosition` writes `_OffsetX/_OffsetY`
//!   as the position divided by the wipe rect's size (6000 by 6000).
//! - The wipe program (`Sekai/Area/WipeCircle`) samples no texture: the
//!   vertex stage writes `p = _Scale^3 * ((uv - offset) * 2 - 1)` and the
//!   fragment writes alpha 1 where `length(p) >= 1`, else 0, with the
//!   material colour. On the 6000-unit rect the hole's radius is therefore
//!   `3000 / _Scale^3` canvas units.
//! - Every wait is a `UniTask.Delay(TimeSpan.FromSeconds(x))`, so it goes
//!   through the millisecond rounding of [`super::timeline::delay_seconds`].
//! - The four house and room player states: their `Initialize` closes the
//!   intercept gate and plays the state clip through
//!   `PlayerAvatarPresenter.PlayAnimation`, which always crossfades 0.25 s;
//!   the float the states pass there is the animation speed, and
//!   `AvatarBase.ChangeMotion` resets the speed to 1 before it plays. Their
//!   `UpdateState` adds the frame delta to the state's elapsed time and acts
//!   once it is no longer below the state's threshold.

use bevy::prelude::*;

use super::timeline::{delay_seconds, TweenClock};

/// `SiteWipeController.FadeOut/FadeIn` duration argument.
pub(crate) const WIPE_DURATION: f32 = 1.0;
/// `_Scale` with the hole wider than any screen (the FadeOut start and the
/// FadeIn end).
pub(crate) const WIPE_OPEN_SCALE: f32 = 0.75;
/// `_Scale` with the hole smaller than a pixel (the FadeOut end and the
/// FadeIn start).
pub(crate) const WIPE_CLOSED_SCALE: f32 = 15.0;
/// The `wipe` RawImage's size (both axes) and `SetPosition`'s divisor.
pub(crate) const WIPE_RECT: f32 = 6000.0;

/// The DOTween ease each wipe direction sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WipeEase {
    InCirc,
    OutCirc,
}

/// DOTween `InCirc`: `-(sqrt(1 - t * t) - 1)`, t already normalised.
pub(crate) fn in_circ(t: f32) -> f32 {
    -((1.0 - t * t).sqrt() - 1.0)
}

/// DOTween `OutCirc`: `sqrt(1 - (t - 1)^2)`, t already normalised.
pub(crate) fn out_circ(t: f32) -> f32 {
    let t = t - 1.0;
    (1.0 - t * t).sqrt()
}

/// One `DOTween.To` over `_Scale` (a float plugin: `start + change * ease`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WipeTween {
    pub(crate) from: f32,
    pub(crate) to: f32,
    pub(crate) ease: WipeEase,
    pub(crate) clock: TweenClock,
}

impl WipeTween {
    /// `FadeOut(1, zero, callback, 15, 0.75)`: InCirc from open to closed.
    pub(crate) fn fade_out() -> Self {
        Self {
            from: WIPE_OPEN_SCALE,
            to: WIPE_CLOSED_SCALE,
            ease: WipeEase::InCirc,
            clock: TweenClock::new(WIPE_DURATION),
        }
    }

    /// `FadeIn(1, zero, callback, 0.75, 15)`: OutCirc from closed to open.
    pub(crate) fn fade_in() -> Self {
        Self {
            from: WIPE_CLOSED_SCALE,
            to: WIPE_OPEN_SCALE,
            ease: WipeEase::OutCirc,
            clock: TweenClock::new(WIPE_DURATION),
        }
    }

    /// `_Scale` at normalised time `e`.
    pub(crate) fn value_at(&self, e: f32) -> f32 {
        let eased = match self.ease {
            WipeEase::InCirc => in_circ(e),
            WipeEase::OutCirc => out_circ(e),
        };
        self.from + (self.to - self.from) * eased
    }

    /// Advance by one frame and return this frame's `_Scale`.
    pub(crate) fn advance(&mut self, dt: f32) -> f32 {
        let e = self.clock.advance(dt);
        self.value_at(e)
    }

    pub(crate) fn done(&self) -> bool {
        self.clock.done()
    }
}

/// The hole's radius in canvas units for a `_Scale` (the program itself is
/// the product's; this and [`coverage`] transcribe it for the value checks).
#[cfg(test)]
pub(crate) fn hole_radius(scale: f32) -> f32 {
    (WIPE_RECT * 0.5) / (scale * scale * scale)
}

/// `SetPosition(position)`: `_OffsetX = x / 6000`, `_OffsetY = y / 6000`.
pub(crate) fn wipe_offset(position: Vec2) -> Vec2 {
    position / WIPE_RECT
}

/// The wipe program at one canvas point (relative to the canvas centre,
/// y up): `None` outside the 6000-unit rect (no fragment), otherwise the
/// alpha the fragment writes.
#[cfg(test)]
pub(crate) fn coverage(canvas: Vec2, scale: f32, offset: Vec2) -> Option<f32> {
    let uv = canvas / WIPE_RECT + Vec2::splat(0.5);
    if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
        return None;
    }
    let s3 = scale * scale * scale;
    let p = ((uv - offset) * 2.0 - Vec2::ONE) * s3;
    Some(if p.dot(p).sqrt() >= 1.0 { 1.0 } else { 0.0 })
}

/// Half the screen in canvas units: the wipe canvas sits under the screen
/// manager's layer, whose root canvas scales by `balloon::canvas_scale`.
pub(crate) fn canvas_half_extent(width: f32, height: f32) -> Vec2 {
    Vec2::new(width, height) * 0.5 / crate::balloon::canvas_scale(width, height)
}

// --- The waits the three move states hand to `UniTask.Delay` ------------

/// `HomeToMyRoomActionState.PlayEnterMyRoomAction`: after `EnterMoveHouse`.
pub(crate) const ENTER_HOUSE_WAIT: f64 = 1.1;
/// `HomeToMyRoomActionState.MoveMyRoom`: after the room door starts opening.
pub(crate) const ENTER_ROOM_DOOR_WAIT: f64 = 1.0;
/// `MoveMyRoom` of home-to-room and room-to-room: after `FadeIn`.
pub(crate) const AFTER_FADE_IN_WAIT: f64 = 1.0;
/// `AutoMoveEntrance` of both room exits: after `se_door_open`.
pub(crate) const EXIT_ROOM_SE_WAIT: f64 = 0.2;
/// `OnSiteMovePreAction` of both room exits: after `ShowFadeOutAnimation`.
pub(crate) const AFTER_FADE_OUT_WAIT: f64 = 1.0;
/// `MyRoomToHomeActionState.StartMysekaiTransition`: after the two
/// `SafeFinish` calls.
pub(crate) const START_TRANSITION_WAIT: f64 = 0.1;
/// `MyRoomToHomeActionState.PlayExitMyRoomAction`: after `ExitMoveHouse`.
pub(crate) const EXIT_HOUSE_WAIT: f64 = 1.0;
/// `MyRoomToMyRoomActionState.MoveMyRoom`: before `ChangeSite`.
pub(crate) const ROOM_TO_ROOM_WAIT: f64 = 1.0;
/// `HomeToMyRoomActionState.MoveMyRoom`: `DelayCall(0.03, CullingWallFixture)`.
pub(crate) const CULLING_WALL_DELAY: f32 = 0.03;

/// A wait as the delay holds it.
pub(crate) fn wait(seconds: f64) -> f32 {
    delay_seconds(seconds)
}

// --- Player states -------------------------------------------------------

/// `PlayerAvatarEnterMoveHouseState.UpdateState`: at 2.0 s the gate opens,
/// the next state is Idle and the avatar is hidden.
pub(crate) const ENTER_MOVE_HOUSE_TIME: f32 = 2.0;
/// `PlayerAvatarEnterMoveMyRoomState.UpdateState`: at 1.0 s the gate opens
/// and the next state is Idle.
pub(crate) const ENTER_MOVE_MY_ROOM_TIME: f32 = 1.0;
/// `PlayerAvatarExitMoveMyRoomState.UpdateState`: the literal word
/// 0x3FB9999A (1.45 in single precision); the avatar is hidden, the gate
/// opens and the next state is Idle.
pub(crate) const EXIT_MOVE_MY_ROOM_TIME: f32 = f32::from_bits(0x3FB9_999A);
/// `PlayerAvatarPresenter.PlayAnimation`'s fade: every state clip.
pub(crate) const STATE_CLIP_FADE: f32 = 0.25;
/// `MyRoomToHomeActionState.Finish`: `PlayAnimation("c_000_mov_idle_00", 0.1, 1)`
/// straight on the view.
pub(crate) const FINISH_IDLE_FADE: f32 = 0.1;

/// `UpdateState` of one timed state: the elapsed time after this frame and
/// whether the state acts this frame. The elapsed time only grows while it
/// is below `float.MaxValue`, and saturates there.
pub(crate) fn state_timer(elapsed: f32, dt: f32, threshold: f32) -> (f32, bool) {
    let elapsed = if elapsed < f32::MAX {
        (elapsed + dt).min(f32::MAX)
    } else {
        elapsed
    };
    (elapsed, !(elapsed < threshold))
}

// --- AutoMove (`PlayerAvatarAutoMoveState`) --------------------------------

/// Arrival distance to the move target (strictly less).
pub(crate) const AUTO_MOVE_ARRIVAL: f32 = 0.1;
/// Arrival speed: the squared velocity at or below this.
pub(crate) const AUTO_MOVE_STOP_SPEED_SQ: f32 = 0.01;
/// A steering vector at or below this length gives no velocity.
pub(crate) const AUTO_MOVE_STEER_EPSILON: f32 = 1.0e-5;

/// The velocity toward the steering corner at the agent's speed.
pub(crate) fn auto_move_velocity(position: Vec3, steering: Vec3, speed: f32) -> Vec3 {
    let delta = steering - position;
    if delta.length() <= AUTO_MOVE_STEER_EPSILON {
        Vec3::ZERO
    } else {
        delta.normalize() * speed
    }
}

/// The requested next position: one frame of velocity, never past the corner.
pub(crate) fn auto_move_request(position: Vec3, steering: Vec3, velocity: Vec3, dt: f32) -> Vec3 {
    let step = velocity * dt;
    if step.length_squared() >= (steering - position).length_squared() {
        steering
    } else {
        position + step
    }
}

/// The state's arrival test.
pub(crate) fn auto_move_arrived(position: Vec3, target: Vec3, velocity: Vec3) -> bool {
    velocity.length_squared() <= AUTO_MOVE_STOP_SPEED_SQ
        && position.distance(target) < AUTO_MOVE_ARRIVAL
}

// --- Admission -------------------------------------------------------------

/// `CanMoveDoorActionPoint`: the last corner's horizontal distance to the
/// house's inside-door point must be below this.
pub(crate) const HOUSE_ENTRY_REACH: f32 = 0.03;

// --- Step schedule --------------------------------------------------------

/// The frame a step is timed from. The walk to a door and the site load take
/// as long as they take, so each segment is timed from where it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anchor {
    /// `OnChangeSite`: the request is admitted.
    Admission,
    /// The `WaitUntil(Idle)` after the walk to the door continues.
    Arrival,
    /// `ChangeSite` returned: the destination stands.
    SiteReady,
}

impl Anchor {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Arrival => "arrival",
            Self::SiteReady => "site-ready",
        }
    }
}

/// The three door move states, as `GetActionState` answers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoorKind {
    HomeToRoom,
    RoomToHome,
    RoomToRoom,
}

impl DoorKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::HomeToRoom => "HomeToMyRoom",
            Self::RoomToHome => "MyRoomToHome",
            Self::RoomToRoom => "MyRoomToMyRoom",
        }
    }
}

/// One ordered step of a door timeline, for the step log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoorStep {
    PreAction,
    AutoMove,
    Arrival,
    FadeOut,
    FadeOutDone,
    StateTimer,
    EnterRoomIdle,
    Core,
    ChangeSite,
    SiteReady,
    EnterRoom,
    ChangeEnvironment,
    ExitHouse,
    FadeInDone,
    CameraNormal,
    CloseDoor,
    EndAction,
    Normal,
}

impl DoorStep {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::PreAction => "pre-action",
            Self::AutoMove => "auto-move",
            Self::Arrival => "arrival",
            Self::FadeOut => "fade-out+se_transition_end",
            Self::FadeOutDone => "fade-out-done",
            Self::StateTimer => "state-timer",
            Self::EnterRoomIdle => "enter-my-room-idle",
            Self::Core => "core",
            Self::ChangeSite => "change-site",
            Self::SiteReady => "site-ready",
            Self::EnterRoom => "enter-room+fade-in",
            Self::ChangeEnvironment => "change-environment",
            Self::ExitHouse => "exit-house",
            Self::FadeInDone => "fade-in-done",
            Self::CameraNormal => "camera-normal",
            Self::CloseDoor => "close-door",
            Self::EndAction => "end-action",
            Self::Normal => "normal",
        }
    }
}

/// The source time of a step: its anchor and the offset the source's waits
/// and tweens put between them. `None` for a step whose time is not fixed
/// by the source (it waits on a load or a walk).
pub(crate) fn source_offset(kind: DoorKind, step: DoorStep) -> Option<(Anchor, f64)> {
    use Anchor as A;
    use DoorStep::*;
    let w = |x: f64| wait(x) as f64;
    let tween = WIPE_DURATION as f64;
    Some(match (kind, step) {
        (_, PreAction) => (A::Admission, 0.0),
        (_, Arrival) => (A::Arrival, 0.0),
        (_, SiteReady) => (A::SiteReady, 0.0),
        // Home to room: PlayEnterMyRoomAction's wait, then the FadeOut
        // tween, whose callback the WaitUntil before ChangeSite reads.
        (DoorKind::HomeToRoom, FadeOut) => (A::Arrival, w(ENTER_HOUSE_WAIT)),
        (DoorKind::HomeToRoom, FadeOutDone | ChangeSite) => {
            (A::Arrival, w(ENTER_HOUSE_WAIT) + tween)
        }
        (DoorKind::HomeToRoom, StateTimer) => (A::Arrival, ENTER_MOVE_HOUSE_TIME as f64),
        (DoorKind::HomeToRoom, EnterRoom | ChangeEnvironment) => {
            (A::SiteReady, w(ENTER_ROOM_DOOR_WAIT))
        }
        (DoorKind::HomeToRoom, FadeInDone) => (A::SiteReady, w(ENTER_ROOM_DOOR_WAIT) + tween),
        (DoorKind::HomeToRoom, EnterRoomIdle) => (
            A::SiteReady,
            w(ENTER_ROOM_DOOR_WAIT) + ENTER_MOVE_MY_ROOM_TIME as f64,
        ),
        (DoorKind::HomeToRoom, CloseDoor | CameraNormal | EndAction | Normal) => (
            A::SiteReady,
            w(ENTER_ROOM_DOOR_WAIT) + w(AFTER_FADE_IN_WAIT),
        ),
        // Both room exits: the SE wait, then FadeOut and the wait after it.
        (DoorKind::RoomToHome | DoorKind::RoomToRoom, FadeOut) => {
            (A::Arrival, w(EXIT_ROOM_SE_WAIT))
        }
        (DoorKind::RoomToHome | DoorKind::RoomToRoom, FadeOutDone) => {
            (A::Arrival, w(EXIT_ROOM_SE_WAIT) + tween)
        }
        (DoorKind::RoomToHome | DoorKind::RoomToRoom, Core) => {
            (A::Arrival, w(EXIT_ROOM_SE_WAIT) + w(AFTER_FADE_OUT_WAIT))
        }
        (DoorKind::RoomToHome | DoorKind::RoomToRoom, StateTimer) => {
            (A::Arrival, EXIT_MOVE_MY_ROOM_TIME as f64)
        }
        (DoorKind::RoomToHome, ChangeSite) => {
            (A::Arrival, w(EXIT_ROOM_SE_WAIT) + w(AFTER_FADE_OUT_WAIT))
        }
        (DoorKind::RoomToHome, ChangeEnvironment) => (A::SiteReady, w(START_TRANSITION_WAIT)),
        (DoorKind::RoomToHome, ExitHouse) => (A::SiteReady, w(START_TRANSITION_WAIT)),
        (DoorKind::RoomToHome, FadeInDone) => (A::SiteReady, tween),
        (DoorKind::RoomToHome, CameraNormal | EndAction | Normal) => {
            (A::SiteReady, w(START_TRANSITION_WAIT) + w(EXIT_HOUSE_WAIT))
        }
        (DoorKind::RoomToRoom, ChangeSite) => (
            A::Arrival,
            w(EXIT_ROOM_SE_WAIT) + w(AFTER_FADE_OUT_WAIT) + w(ROOM_TO_ROOM_WAIT),
        ),
        (DoorKind::RoomToRoom, EnterRoom) => (A::SiteReady, 0.0),
        (DoorKind::RoomToRoom, FadeInDone) => (A::SiteReady, tween),
        (DoorKind::RoomToRoom, EnterRoomIdle) => (A::SiteReady, ENTER_MOVE_MY_ROOM_TIME as f64),
        (DoorKind::RoomToRoom, CameraNormal | CloseDoor | EndAction | Normal) => {
            (A::SiteReady, w(AFTER_FADE_IN_WAIT))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Research instrument: the expected values were evaluated outside this
    //! module in single precision from the source's own expressions (the
    //! DOTween ease formulas, the wipe program's vertex and fragment lines,
    //! the move states' waits), so a changed constant or formula here turns
    //! them red.
    use super::*;

    /// `_Scale` of both wipe directions at sampled times (f32 evaluation of
    /// `start + (end - start) * ease(t)`).
    #[test]
    fn wipe_scale_follows_the_dotween_circ_eases() {
        let out = WipeTween::fade_out();
        let inn = WipeTween::fade_in();
        let cases: [(f32, f32, f32); 5] = [
            (0.1, 0.821_429_13, 8.788_568_5),
            (0.25, 1.202_496_4, 5.574_511_5),
            (0.5, 2.659_138_2, 2.659_138_7),
            (0.75, 5.574_511, 1.202_496_5),
            (0.9, 8.788_568_5, 0.821_429_25),
        ];
        for (t, closing, opening) in cases {
            assert!(
                (out.value_at(t) - closing).abs() < 2e-6,
                "FadeOut {t}: {}",
                out.value_at(t)
            );
            assert!(
                (inn.value_at(t) - opening).abs() < 2e-6,
                "FadeIn {t}: {}",
                inn.value_at(t)
            );
        }
        // The ends are the stored values exactly.
        assert_eq!(out.value_at(0.0), WIPE_OPEN_SCALE);
        assert_eq!(out.value_at(1.0), WIPE_CLOSED_SCALE);
        assert_eq!(inn.value_at(0.0), WIPE_CLOSED_SCALE);
        assert_eq!(inn.value_at(1.0), WIPE_OPEN_SCALE);
        // Closing covers a 1920x1080 screen's corners (1101.45 canvas units
        // from the centre) at t = 0.2978 and opening uncovers them at 0.7022.
        assert!(hole_radius(out.value_at(0.297)) > 1101.45);
        assert!(hole_radius(out.value_at(0.299)) < 1101.45);
        assert!(hole_radius(inn.value_at(0.701)) < 1101.45);
        assert!(hole_radius(inn.value_at(0.703)) > 1101.45);
    }

    /// The wipe program transcribed line by line (vertex: `uv - offset`,
    /// `* 2 - 1`, `* _Scale^3`; fragment: `sqrt(dot) >= 1`) against
    /// [`coverage`], at pixels either side of the hole's edge.
    #[test]
    fn coverage_matches_the_wipe_program_at_sampled_pixels() {
        let glsl = |uv: Vec2, scale: f32, offset: Vec2| {
            let a = uv - offset;
            let a = a * 2.0 + Vec2::splat(-1.0);
            let s = scale * scale;
            let s = s * scale;
            let p = a * s;
            if p.dot(p).sqrt() >= 1.0 {
                1.0
            } else {
                0.0
            }
        };
        // _Scale 2: radius 375 canvas units. At 1920x1080 a canvas unit is
        // one pixel; at 1280x720 it is 2/3 of one.
        for (width, height, radius_px) in [(1920.0, 1080.0, 375.0), (1280.0, 720.0, 250.0)] {
            let half = canvas_half_extent(width, height);
            let px_to_canvas = half.x / (width * 0.5);
            for (dx, want) in [(radius_px - 1.0, 0.0), (radius_px + 1.0, 1.0), (0.0, 0.0)] {
                let canvas = Vec2::new(dx * px_to_canvas, 0.0);
                let uv = canvas / WIPE_RECT + Vec2::splat(0.5);
                let reference = glsl(uv, 2.0, Vec2::ZERO);
                assert_eq!(reference, want, "{width}x{height} {dx}px");
                assert_eq!(coverage(canvas, 2.0, Vec2::ZERO), Some(want));
            }
        }
        // Open: no pixel of a 1920x1080 screen is covered; closed: every one.
        let corner = Vec2::new(960.0, 540.0);
        assert_eq!(coverage(corner, WIPE_OPEN_SCALE, Vec2::ZERO), Some(0.0));
        assert_eq!(
            coverage(Vec2::new(1.0, 0.0), WIPE_CLOSED_SCALE, Vec2::ZERO),
            Some(1.0)
        );
        assert!(hole_radius(WIPE_CLOSED_SCALE) < 1.0);
        // Outside the 6000-unit rect there is no fragment.
        assert_eq!(
            coverage(Vec2::new(3001.0, 0.0), WIPE_CLOSED_SCALE, Vec2::ZERO),
            None
        );
        // SetPosition(zero) keeps the centre.
        assert_eq!(wipe_offset(Vec2::ZERO), Vec2::ZERO);
        assert_eq!(wipe_offset(Vec2::new(600.0, -300.0)), Vec2::new(0.1, -0.05));
    }

    #[test]
    fn state_thresholds_are_the_stored_words() {
        assert_eq!(EXIT_MOVE_MY_ROOM_TIME, 1.45_f32);
        assert_eq!(EXIT_MOVE_MY_ROOM_TIME.to_bits(), 0x3FB9_999A);
        // The comparison is `elapsed >= threshold` in single precision: at
        // 60 fps 1.45 s is reached on the 88th frame (87 deltas give 1.4499993).
        let mut elapsed = 0.0;
        let frame = (1..200)
            .find(|_| {
                let (next, act) = state_timer(elapsed, 1.0 / 60.0, EXIT_MOVE_MY_ROOM_TIME);
                elapsed = next;
                act
            })
            .unwrap();
        assert_eq!(frame, 88);
        assert_eq!(state_timer(f32::MAX, 1.0, 2.0), (f32::MAX, true));
    }

    #[test]
    fn schedule_follows_the_source_waits() {
        let at = |kind, step| source_offset(kind, step).unwrap();
        use DoorKind::*;
        use DoorStep::*;
        assert_eq!(at(HomeToRoom, FadeOut), (Anchor::Arrival, 1.1_f32 as f64));
        assert!((at(HomeToRoom, ChangeSite).1 - 2.1).abs() < 1e-6);
        assert!((at(HomeToRoom, EnterRoom).1 - 1.0).abs() < 1e-9);
        assert!((at(HomeToRoom, EndAction).1 - 2.0).abs() < 1e-9);
        assert!((at(RoomToHome, FadeOut).1 - 0.2).abs() < 1e-6);
        assert!((at(RoomToHome, Core).1 - 1.2).abs() < 1e-6);
        assert!((at(RoomToHome, ChangeEnvironment).1 - 0.1).abs() < 1e-6);
        assert!((at(RoomToHome, EndAction).1 - 1.1).abs() < 1e-6);
        assert!((at(RoomToRoom, ChangeSite).1 - 2.2).abs() < 1e-6);
        assert_eq!(at(RoomToRoom, EndAction), (Anchor::SiteReady, 1.0));
        assert_eq!(source_offset(HomeToRoom, AutoMove), None);
    }

    #[test]
    fn auto_move_stops_at_the_corner_and_arrives_within_its_bounds() {
        let from = Vec3::ZERO;
        let corner = Vec3::new(1.0, 0.0, 0.0);
        let v = auto_move_velocity(from, corner, 2.0);
        assert_eq!(v, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(
            auto_move_request(from, corner, v, 0.1),
            Vec3::new(0.2, 0.0, 0.0)
        );
        assert_eq!(auto_move_request(from, corner, v, 1.0), corner);
        assert_eq!(auto_move_velocity(corner, corner, 2.0), Vec3::ZERO);
        // 0.1 squared is 0.010000001 in single precision, above the stored
        // 0.01: the arrival speed sits just below it.
        assert!(auto_move_arrived(
            Vec3::new(0.099, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.09, 0.0, 0.0)
        ));
        assert!(!auto_move_arrived(
            Vec3::new(0.099, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.1, 0.0, 0.0)
        ));
        assert!(!auto_move_arrived(
            Vec3::new(0.1, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::ZERO
        ));
        assert!(!auto_move_arrived(
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::new(0.11, 0.0, 0.0)
        ));
    }
}
