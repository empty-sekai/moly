//! Pure laws of the MySekai entry, each a port of one source routine.
//!
//! Every constant here is read from the game's native code; the method it
//! belongs to is named beside it. Timing helpers reproduce the
//! frame semantics of the source's two clocks: the cover's coroutine counts
//! the frame that started it, a UniTask delay does not.

use bevy::prelude::*;

use crate::camera::{to_rotation, wrap180, CameraSetting, CameraTween, FieldCameraModel};

/// `ColorUtility.WHITE_ALPHA_1`: the static constructor stores four 1.0s.
pub(crate) const WHITE_ALPHA_1: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// `ColorUtility.WHITE_ALPHA_0`: the static constructor loads (1, 1, 1, 0).
pub(crate) const WHITE_ALPHA_0: [f32; 4] = [1.0, 1.0, 1.0, 0.0];
/// `LiveTransitioner.Finish`: `coverFader.Play(WHITE_ALPHA_0, delay, duration)`.
pub(crate) const COVER_FADE_DELAY: f32 = 1.0;
pub(crate) const COVER_FADE_DURATION: f32 = 1.0;
/// `LiveTransitioner.Finish`: the transitioner destroys itself 5 s later.
pub(crate) const COVER_DESTROY_DELAY: f32 = 5.0;
/// `UIUtility.PlayLiveTransition(..., timeout: 300)`: `DestroyMySelf(300)`,
/// the safety net for a transition that is never finished.
pub(crate) const COVER_SAFETY_TIMEOUT: f32 = 300.0;
/// `JoinMysekaiActionState.EXIT_HOME_ANIMATION_DELAY_TIME`, awaited by
/// `StartMysekaiTransition` after `LiveTransitioner.SafeFinish`.
pub(crate) const EXIT_HOME_ANIMATION_DELAY_TIME: f32 = 0.5;
/// `JoinMysekaiActionState.EXIT_HOME_ANIMATION_TIME`, awaited by
/// `PlayExitMyRoomAction` after `ChangeState(ExitMoveHouse)`.
pub(crate) const EXIT_HOME_ANIMATION_TIME: f32 = 1.5;
/// `SceneMysekai.WaitCharacterSpawnedAsync`: the linked timeout.
pub(crate) const CHARACTER_SPAWN_TIMEOUT: f32 = 3.0;
/// `JoinMysekaiActionState.Finish`: `PlayAnimation("c_000_mov_idle_00", 0.1)`.
pub(crate) const FINISH_IDLE_CROSSFADE: f32 = 0.1;
/// `ScreenLayerMysekaiNotice.ShowSiteEnvironmentInfo(0.8, 1)` from
/// `JoinMysekaiActionState.ShowSiteEnvironment`.
pub(crate) const BANNER_SHOW_DELAY: f32 = 0.8;
pub(crate) const BANNER_SHOW_DURATION: f32 = 1.0;
/// `JoinMysekaiEndAction` (not refreshed): `DelayCall(1, HideSiteEnvironment)`,
/// which calls `HideSiteEnvironmentInfo(0, 0.5)`.
pub(crate) const BANNER_HIDE_CALL_DELAY: f32 = 1.0;
pub(crate) const BANNER_HIDE_DURATION: f32 = 0.5;
/// `HouseEntryCameraState` constructor: its model copy gets `Distance = 5`.
pub(crate) const HOUSE_ENTRY_DISTANCE: f32 = 5.0;
/// `HouseEntryCameraState.OnEnter`: `Pitch = 25` and the tween duration.
pub(crate) const HOUSE_ENTRY_PITCH: f32 = 25.0;
pub(crate) const HOUSE_ENTRY_TWEEN_SECONDS: f32 = 0.03;
/// `NormalCameraState.TransferCameraSettings`: `_doTweenCameraTime = 1.0`.
pub(crate) const NORMAL_TRANSFER_TWEEN_SECONDS: f32 = 1.0;

/// `Mathf.Approximately`.
fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (0.000_001 * a.abs().max(b.abs())).max(f32::from_bits(1) * 8.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FadeStage {
    /// Inside the nested `CP_SystemUtility.WaitForSeconds(delay)`.
    Delay,
    /// The fade loop of `ColorFader.PlaySimpleFade`.
    Fade,
    /// The target colour is written; the coroutine has ended.
    Done,
}

/// `ColorFader.Play(target, delay, duration)`: not a DOTween call. It starts
/// the coroutine `PlaySimpleFade`, which keeps the current colour as base,
/// yields `CP_SystemUtility.WaitForSeconds(delay)` (a loop that adds
/// `Time.deltaTime` and yields null while `time < duration`), then on every
/// frame either writes the target (once `time >= duration`) or writes
/// `base + (target - base) * clamp(time / duration, 0, 1)` and only then adds
/// this frame's delta. The first fade frame therefore writes the base colour.
#[derive(Clone, Debug)]
pub(crate) struct ColorFade {
    from: [f32; 4],
    to: [f32; 4],
    delay: f32,
    duration: f32,
    waited: f32,
    time: f32,
    stage: FadeStage,
}

impl ColorFade {
    /// The frame `Play` is called: `StartCoroutine` runs the body up to its
    /// first yield, and the nested wait already adds this frame's delta.
    pub(crate) fn start(
        from: [f32; 4],
        to: [f32; 4],
        delay: f32,
        duration: f32,
        dt: f32,
    ) -> (Self, Option<[f32; 4]>) {
        let mut fade = Self {
            from,
            to,
            delay,
            duration,
            waited: 0.0,
            time: 0.0,
            stage: FadeStage::Delay,
        };
        if approximately(delay, 0.0) && approximately(duration, 0.0) {
            fade.stage = FadeStage::Done;
            return (fade, Some(to));
        }
        let written = fade.run(dt);
        (fade, written)
    }

    /// One later frame's resumption. Returns the colour written this frame.
    pub(crate) fn resume(&mut self, dt: f32) -> Option<[f32; 4]> {
        self.run(dt)
    }

    pub(crate) fn stage(&self) -> FadeStage {
        self.stage
    }

    fn run(&mut self, dt: f32) -> Option<[f32; 4]> {
        if self.stage == FadeStage::Delay {
            if self.waited < self.delay {
                self.waited += dt;
                return None;
            }
            // The nested routine ends; the outer coroutine resumes this frame.
            self.stage = FadeStage::Fade;
        }
        if self.stage != FadeStage::Fade {
            return None;
        }
        if self.time >= self.duration {
            self.stage = FadeStage::Done;
            return Some(self.to);
        }
        let mut u = (self.time / self.duration).min(1.0);
        if u < 0.0 {
            u = 0.0;
        }
        let mut colour = [0.0; 4];
        for (channel, value) in colour.iter_mut().enumerate() {
            *value = self.from[channel] + (self.to[channel] - self.from[channel]) * u;
        }
        self.time += dt;
        Some(colour)
    }
}

/// `UniTask.Delay(seconds)` in the default delta-time mode: the delay promise
/// does nothing on the frame it was created, then adds `Time.deltaTime` each
/// frame and completes on the frame where `elapsed >= delay`; the awaiting
/// continuation runs in that same frame. [`Self::advance`] is called from the
/// frame after creation on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UniTaskDelay {
    delay: f32,
    elapsed: f32,
}

impl UniTaskDelay {
    pub(crate) fn new(delay: f32) -> Self {
        Self {
            delay,
            elapsed: 0.0,
        }
    }

    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed >= self.delay
    }
}

/// `Quaternion.eulerAngles.y` of a rotation in the source (Unity) frame:
/// the engine's matrix-to-Euler (Y from `atan2(m02, m22)`) in degrees, then
/// `Internal_MakePositive` with its flip bounds of -0.0001 rad.
pub(crate) fn unity_euler_y_degrees(q: Quat) -> f32 {
    const RAD_TO_DEG: f32 = 57.295_78;
    let m02 = 2.0 * (q.x * q.z + q.w * q.y);
    let m22 = 1.0 - 2.0 * (q.x * q.x + q.y * q.y);
    let mut y = m02.atan2(m22) * RAD_TO_DEG;
    let negative_flip = -0.0001 * RAD_TO_DEG;
    let positive_flip = 360.0 + negative_flip;
    if y < negative_flip {
        y += 360.0;
    } else if y > positive_flip {
        y -= 360.0;
    }
    y
}

/// `HouseEntryCameraState.OnEnter`: `Yaw = Mathf.Repeat(eulerY + 180, 360)`,
/// which is `x - floor(x / 360) * 360` clamped to `[0, 360]`.
pub(crate) fn house_entry_yaw(euler_y: f32) -> f32 {
    let x = euler_y + 180.0;
    let v = (x - (x / 360.0).floor() * 360.0).min(360.0);
    if v < 0.0 {
        0.0
    } else {
        v
    }
}

/// `DoTweenCameraSetting(lookAt, (pitch, yaw), fov, distance, duration)`:
/// every channel starts from the current value; the rotation starts from
/// `ConvertAngle180` of the current angle and ends at `GetToRotation`.
fn camera_setting_tween(
    model: &FieldCameraModel,
    camera_fov: f32,
    look_at: Vec3,
    pitch: f32,
    yaw: f32,
    fov: f32,
    distance: f32,
    duration: f32,
) -> CameraTween {
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    CameraTween {
        look_at: (model.look_at, look_at),
        fov: (camera_fov, fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
        distance: (model.distance, distance),
        duration,
        elapsed: 0.0,
    }
}

/// The Normal state's private values that `TransferCameraSettings` tweens
/// back to. `NormalCameraState.OnExit` copies the owner's pitch and distance
/// into them when HouseEntry takes over; the FOV is never written after the
/// constructor copied the camera setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NormalPrivate {
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
}

/// `HouseEntryCameraState.OnEnter` on the owner model: the distance bounds
/// come from the state's model (a copy of the owner made at boot, so the
/// camera setting's bounds), then LookAt = player, Pitch = 25,
/// Yaw = `house_entry_yaw`, and a 0.03 s tween to (FOV of the copy, 5).
pub(crate) fn enter_house_entry(
    model: &mut FieldCameraModel,
    setting: &CameraSetting,
    player_position: Vec3,
    player_rotation: Quat,
    camera_fov: f32,
) -> CameraTween {
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    let euler_y = unity_euler_y_degrees(moly_assets::coordinates::source_rotation(player_rotation));
    let yaw = house_entry_yaw(euler_y);
    camera_setting_tween(
        model,
        camera_fov,
        player_position,
        HOUSE_ENTRY_PITCH,
        yaw,
        setting.fov,
        HOUSE_ENTRY_DISTANCE,
        HOUSE_ENTRY_TWEEN_SECONDS,
    )
}

/// `NormalCameraState.OnEnter` then `TransferCameraSettings` with the
/// previous state HouseEntry (8): the bounds and offset come back from the
/// Normal model (the camera setting), the private Yaw and LookAt take the
/// owner's, and one 1.0 s tween goes to the private pitch, FOV and distance.
pub(crate) fn transfer_from_house_entry(
    model: &mut FieldCameraModel,
    setting: &CameraSetting,
    private: NormalPrivate,
    camera_fov: f32,
) -> CameraTween {
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    let (look_at, yaw) = (model.look_at, model.yaw);
    camera_setting_tween(
        model,
        camera_fov,
        look_at,
        private.pitch,
        yaw,
        private.fov,
        private.distance,
        NORMAL_TRANSFER_TWEEN_SECONDS,
    )
}

#[cfg(test)]
mod tests {
    //! Research instruments: each expected value is recomputed from the
    //! source routine named above it, not read back from this module.
    use super::*;

    /// `PlaySimpleFade` evaluated by hand in f32 from its loop, for a 0.1 s
    /// and a 1/60 s frame. The wait adds the Play frame's delta and checks
    /// before adding; the fade writes, then adds. Frame numbers count the
    /// frames after the SafeFinish frame.
    #[test]
    fn cover_fade_follows_the_play_simple_fade_loop() {
        let run = |dt: f32| {
            let (mut fade, first) = ColorFade::start(WHITE_ALPHA_1, WHITE_ALPHA_0, 1.0, 1.0, dt);
            assert_eq!(first, None, "the Play frame only waits");
            let written: Vec<(u32, f32)> = (1..200)
                .filter_map(|frame| {
                    fade.resume(dt).map(|colour| {
                        assert_eq!(&colour[..3], &[1.0, 1.0, 1.0], "the fade keeps white");
                        (frame, colour[3])
                    })
                })
                .collect();
            assert_eq!(fade.stage(), FadeStage::Done);
            written
        };
        // 0.1 s: ten deltas reach 1.0000001 on frame 9's add, so frame 10
        // writes the base; eleven writes, the last one the exact target.
        let tenth = run(0.1);
        assert_eq!(tenth.len(), 11);
        assert_eq!(tenth[0], (10, 1.0));
        assert_eq!(tenth[1], (11, 0.9));
        assert_eq!(tenth[9], (19, 0.099_999_905));
        assert_eq!(tenth[10], (20, 0.0));
        // 1/60 s: sixty deltas sum to 0.99999994 in f32, so the wait takes a
        // sixty-first; the base is written on frame 61, the target on 122.
        let sixtieth = run(1.0 / 60.0);
        assert_eq!(sixtieth.len(), 62);
        assert_eq!(sixtieth[0], (61, 1.0));
        assert_eq!(sixtieth[1], (62, 0.983_333_35));
        assert_eq!(sixtieth[60], (121, 2.980_232_2e-7));
        assert_eq!(sixtieth[61], (122, 0.0));
    }

    /// For equal durations the coroutine wait and the UniTask delay end on
    /// the same frame: the wait's extra Play-frame delta is spent by its
    /// check-before-add. With 0.1 s frames both finish on frame 5.
    #[test]
    fn coroutine_wait_and_unitask_delay_end_on_the_same_frame() {
        let dt = 0.1;
        let (mut fade, _) = ColorFade::start(WHITE_ALPHA_1, WHITE_ALPHA_0, 0.5, 1.0, dt);
        let mut delay = UniTaskDelay::new(0.5);
        let mut fade_frame = None;
        let mut delay_frame = None;
        for frame in 1..20 {
            if fade.resume(dt).is_some() && fade_frame.is_none() {
                fade_frame = Some(frame);
            }
            if delay_frame.is_none() && delay.advance(dt) {
                delay_frame = Some(frame);
            }
        }
        assert_eq!(delay_frame, Some(5));
        assert_eq!(fade_frame, Some(5));
        // The join's 1.5 s at 60 fps: the delay completes on the 91st frame
        // (91 deltas = 1.5166659 in f32; 90 deltas are 1.4999999).
        let mut exit = UniTaskDelay::new(1.5);
        let frames = (1..200).find(|_| exit.advance(1.0 / 60.0));
        assert_eq!(frames, Some(91));
    }

    /// `HouseEntryCameraState.OnEnter` yaw: a Bevy yaw b is the Unity yaw
    /// -b in this coordinate contract; `Mathf.Repeat(y + 180, 360)` of the
    /// `MakePositive` euler, the expected values worked by hand.
    #[test]
    fn house_entry_yaw_wraps_the_player_euler_like_the_native_on_enter() {
        let cases = [
            // (Bevy yaw of the player in degrees, expected camera yaw)
            // Unity 0 -> 180.
            (0.0f32, 180.0f32),
            // Unity -90 -> MakePositive 270 -> 450 -> Repeat 90.
            (90.0, 90.0),
            // Unity 90 -> 270.
            (-90.0, 270.0),
            // Unity -30 -> 330 -> 510 -> 150.
            (30.0, 150.0),
            // Unity -179 -> 181 -> 361 -> 1.
            (179.0, 1.0),
            // Unity -0.001 stays above the -0.0057 flip bound -> 179.999.
            (0.001, 179.999),
        ];
        for (bevy_yaw, expected) in cases {
            let q = Quat::from_rotation_y(bevy_yaw.to_radians());
            let euler = unity_euler_y_degrees(moly_assets::coordinates::source_rotation(q));
            let yaw = house_entry_yaw(euler);
            assert!(
                (yaw - expected).abs() < 1.0e-3,
                "bevy yaw {bevy_yaw}: euler {euler} camera yaw {yaw}, expected {expected}"
            );
        }
        // The exact seat of the entry house (door facing +Z): identity.
        assert_eq!(
            house_entry_yaw(unity_euler_y_degrees(Quat::IDENTITY)),
            180.0
        );
        // MakePositive: below the flip bound the euler turns positive.
        let q =
            moly_assets::coordinates::source_rotation(Quat::from_rotation_y(0.5f32.to_radians()));
        assert!((unity_euler_y_degrees(q) - 359.5).abs() < 1.0e-3);
    }

    fn model() -> (FieldCameraModel, CameraSetting) {
        // The extracted field camera setting (fieldcamerasetting asset).
        let setting = CameraSetting {
            offset: Vec3::new(0.0, 0.5, 0.0),
            distance: 8.0,
            min_distance: 1.7,
            max_distance: 8.0,
            init_yaw: 180.0,
            init_pitch: 32.0,
            min_pitch: 8.0,
            max_pitch: 70.0,
            rot_sensitivity: 0.075,
            fov: 35.0,
        };
        let model = FieldCameraModel {
            look_at: Vec3::new(1.0, 0.0, 2.0),
            offset: setting.offset,
            fov: setting.fov,
            distance: setting.distance,
            min_distance: setting.min_distance,
            max_distance: setting.max_distance,
            yaw: setting.init_yaw,
            pitch: setting.init_pitch,
            rot_sensitivity: setting.rot_sensitivity,
            min_pitch: setting.min_pitch,
            max_pitch: setting.max_pitch,
            gestured_distance: setting.distance,
            look_at_bounds: crate::camera::BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
            max_look_at_bounds: crate::camera::BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
        };
        (model, setting)
    }

    /// OnEnter's tween endpoints and the 1 s transfer's, from the boot model
    /// (8 / 32 / 180 / 35): HouseEntry goes to (player, 25, 180, 35, 5) in
    /// 0.03 s; the transfer returns to (32, 35, 8) keeping yaw and LookAt.
    #[test]
    fn house_entry_and_transfer_tween_endpoints() {
        let (mut model, setting) = model();
        let player = Vec3::new(3.04, 0.0, -0.25);
        let private = NormalPrivate {
            pitch: model.pitch,
            distance: model.distance,
            fov: setting.fov,
        };
        let enter = enter_house_entry(&mut model, &setting, player, Quat::IDENTITY, 35.0);
        assert_eq!(enter.look_at, (Vec3::new(1.0, 0.0, 2.0), player));
        assert_eq!(enter.pitch, (32.0, 25.0));
        assert_eq!(enter.yaw, (180.0, 180.0));
        assert_eq!(enter.fov, (35.0, 35.0));
        assert_eq!(enter.distance, (8.0, 5.0));
        assert_eq!(enter.duration, 0.03);
        // After the tween: the owner model holds the HouseEntry values.
        model.look_at = player;
        model.pitch = 25.0;
        model.yaw = 180.0;
        model.distance = 5.0;
        let transfer = transfer_from_house_entry(&mut model, &setting, private, 35.0);
        assert_eq!(transfer.look_at, (player, player));
        assert_eq!(transfer.pitch, (25.0, 32.0));
        assert_eq!(transfer.yaw, (180.0, 180.0));
        assert_eq!(transfer.fov, (35.0, 35.0));
        assert_eq!(transfer.distance, (5.0, 8.0));
        assert_eq!(transfer.duration, 1.0);
        // A yaw across the seam takes the short way (GetToRotation).
        model.yaw = 350.0;
        let short = enter_house_entry(
            &mut model,
            &setting,
            player,
            Quat::from_rotation_y((-190.0f32).to_radians()),
            35.0,
        );
        // Bevy -190 -> Unity 190 -> +180 -> 10; from -10 the short way is +20.
        assert!((short.yaw.0 - (-10.0)).abs() < 1.0e-3);
        assert!((short.yaw.1 - 10.0).abs() < 1.0e-3, "{:?}", short.yaw);
    }

    /// DOTween's OutQuad (`-c * (t /= d) * (t - 2) + b`), the ease that
    /// `DoTweenCameraSetting` sets on all four tweens, against the product's
    /// ease function, sampled across both entry tweens.
    #[test]
    fn camera_tweens_use_dotween_out_quad() {
        for (duration, from, to) in [
            (0.03f32, 8.0f32, 5.0f32),
            (1.0, 5.0, 8.0),
            (1.0, 25.0, 32.0),
        ] {
            for step in 0..=10 {
                let elapsed = duration * step as f32 / 10.0;
                let t = elapsed / duration;
                let reference = -(to - from) * t * (t - 2.0) + from;
                let product = from + (to - from) * crate::camera::out_quad(t);
                assert!(
                    (reference - product).abs() < 1.0e-5,
                    "{duration} {elapsed}: {reference} vs {product}"
                );
            }
        }
    }
}
