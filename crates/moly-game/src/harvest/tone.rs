//! Tone objects: the HarvestTone camera (state 14) and the tone view's
//! sounds.
//!
//! `ChangeCameraModeFromFixtureType` (step 5 of every press) maps the fixture
//! type through a ten-entry table: tone (6) -> HarvestTone (14), every other
//! type -> Normal (1); the action's end sets Normal again.
//!
//! `HarvestToneCameraState`:
//! - OnEnter: the camera locks, the previous model is inherited, the start
//!   distance and pitch are remembered, the elapsed time is 0, the harvest
//!   head-up display hides (the UI lane's);
//! - OnUpdate, while animating: the look-at moves toward the player's view
//!   position by `k = dt * 5` clamped to [0, 1] (0 when negative); the elapsed
//!   time grows by the frame time and `t = elapsed / 4.5` (forward) or
//!   `/ 0.89` (reverse). A forward `t >= 1` starts the reverse (elapsed 0,
//!   `t` = 1); a reverse `t >= 1` ends the animation (unlocks, no write that
//!   frame); a reverse `t` below 1 becomes `1 - t`. `e = EaseInOutQuad(t)`
//!   clamped to [0, 1]; `Distance = start + e (1.7 - start)`,
//!   `Pitch = start + e (8.0 - start)`; then the camera position and its
//!   look at the look-at point plus the offset. When not animating the state
//!   writes nothing;
//! - OnExit: the head-up display shows again.
//!
//! The tone view: Setup names its SE `"se_" + assetbundleName`;
//! `OnPlayerActionStart` plays it (`PlayEnvironmentSE(name, 0, 1, 6)`);
//! `PlayDamageEffect` stops its field effect; `ChangeAfterObject` waits
//! `GetHarvestActionTime(null, tone)` (the listen clip's length), then stops
//! the field effect and that SE and fades the BGM back to its initial
//! volume.
//!
//! Named gaps: `ResetCameraSetting` (the 0.5 s tween back to the previous
//! model when the state is left) is not ported: the reverse phase has brought
//! the distance and pitch back to within the last frame's step of the start;
//! the BGM fade inside the tone's radius (`StartFade(0)` on enter, back to
//! the initial volume on exit and after the listen) is not ported: the BGM
//! channel has no manager fade; the field effect's particles are not drawn;
//! the SE plays through the ordinary one-shot path (the environment SE's own
//! parameters are logged).

use bevy::prelude::*;

use crate::camera::{CameraStateType, FieldCameraModel, FieldCameraState};
use crate::player::PlayerControlled;

/// `CAMERA_ANIMATION_TIME`.
const FORWARD: f32 = 4.5;
/// `REVERSE_ANIMATION_TIME`.
const REVERSE: f32 = 0.89;
/// `END_DISTANCE`.
const END_DISTANCE: f32 = 1.7;
/// `END_PITCH` (degrees).
const END_PITCH: f32 = 8.0;

/// `CP.Easing.Quint.EaseInOutQuad`: below 0.5 `(t + t) t`, else
/// `1 - 0.5 (2 - (t + t))^2`.
pub(crate) fn ease_in_out_quad(t: f32) -> f32 {
    let twice = t + t;
    if t < 0.5 {
        twice * t
    } else {
        let u = 2.0 - twice;
        u * u * -0.5 + 1.0
    }
}

/// The animation clock of `HarvestToneCameraState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ToneClock {
    pub(crate) elapsed: f32,
    pub(crate) reversing: bool,
    pub(crate) animating: bool,
}

/// One OnUpdate after the look-at step: the blend `e` to write, or `None`
/// (not animating, or the frame that ends the reverse).
pub(crate) fn tone_step(clock: &mut ToneClock, dt: f32) -> Option<f32> {
    if !clock.animating {
        return None;
    }
    clock.elapsed += dt;
    let duration = if clock.reversing { REVERSE } else { FORWARD };
    let mut t = clock.elapsed / duration;
    if t >= 1.0 {
        if clock.reversing {
            clock.animating = false;
            clock.reversing = false;
            return None;
        }
        clock.elapsed = 0.0;
        clock.reversing = true;
        t = 1.0;
    } else if clock.reversing {
        t = 1.0 - t;
    }
    let e = ease_in_out_quad(t);
    Some(if e >= 0.0 { e.min(1.0) } else { 0.0 })
}

/// The state's own fields.
#[derive(Resource, Default)]
pub(crate) struct HarvestToneCamera {
    entered: bool,
    clock: ToneClock,
    start_distance: f32,
    start_pitch: f32,
    last_logged: f32,
}

/// PostUpdate, after the field camera's follow: HarvestTone's OnEnter,
/// OnUpdate and OnExit (the follow leaves this state to it).
pub(crate) fn advance_tone_camera(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    mut tone: ResMut<HarvestToneCamera>,
    models: Option<ResMut<FieldCameraModel>>,
    players: Query<&GlobalTransform, With<PlayerControlled>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(mut models) = models else {
        return;
    };
    let in_state = state.0 == CameraStateType::HarvestTone;
    if in_state && !tone.entered {
        tone.entered = true;
        tone.start_distance = models.distance;
        tone.start_pitch = models.pitch;
        tone.clock = ToneClock {
            elapsed: 0.0,
            reversing: false,
            animating: true,
        };
        tone.last_logged = -1.0;
        info!(
            "[harvest-tone] HarvestToneCameraState.OnEnter: camera locked, start distance {:.3} pitch {:.2}, the harvest head-up display hidden (UI lane)",
            models.distance, models.pitch
        );
    } else if !in_state && tone.entered {
        tone.entered = false;
        info!(
            "[harvest-tone] HarvestToneCameraState.OnExit -> {:?}: the head-up display shown; distance {:.3} pitch {:.2} as the state left them (ResetCameraSetting not ported)",
            state.0, models.distance, models.pitch
        );
    }
    if !in_state || !tone.clock.animating {
        return;
    }
    let dt = time.delta_secs();
    if let Ok(player) = players.single() {
        let v = dt * 5.0;
        let k = if v >= 0.0 { v.min(1.0) } else { 0.0 };
        let target = player.translation();
        let look_at = models.look_at;
        models.look_at = look_at + (target - look_at) * k;
    }
    let before = tone.clock;
    let Some(e) = tone_step(&mut tone.clock, dt) else {
        info!(
            "[harvest-tone] reverse done: animation over, camera unlocked; distance {:.3} pitch {:.2} (the last written frame)",
            models.distance, models.pitch
        );
        return;
    };
    let (start_distance, start_pitch) = (tone.start_distance, tone.start_pitch);
    models.distance = start_distance + e * (END_DISTANCE - start_distance);
    models.pitch = start_pitch + e * (END_PITCH - start_pitch);
    // UpdatePosition, then the view looks at the look-at point plus offset.
    let pivot = models.look_at + models.offset;
    let eye = pivot + crate::camera::view_dir(models.pitch, models.yaw) * models.distance;
    if let Ok(mut camera) = cameras.single_mut() {
        *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
    }
    let phase_time = tone.clock.elapsed + if tone.clock.reversing { FORWARD } else { 0.0 };
    let started_reverse = tone.clock.reversing && !before.reversing;
    if started_reverse || (phase_time / 0.5).floor() != (tone.last_logged / 0.5).floor() {
        tone.last_logged = phase_time;
        info!(
            "[harvest-tone] {} t {:.4}: e {:.4}, distance {:.3} pitch {:.2} (read back from the model)",
            if tone.clock.reversing {
                "reverse"
            } else {
                "forward"
            },
            tone.clock.elapsed,
            e,
            models.distance,
            models.pitch
        );
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// The schedule as read, at dt 0.125: forward t = elapsed / 4.5 eased
    /// (0.125 at t 0.25, 0.5 at t 0.5, 0.875 at t 0.75), the forward end
    /// writes e 1 and starts the reverse, the reverse eases 1 - elapsed / 0.89
    /// and its end writes nothing. Values by hand.
    #[test]
    fn tone_camera_schedule() {
        let mut clock = ToneClock {
            elapsed: 0.0,
            reversing: false,
            animating: true,
        };
        let mut forward = Vec::new();
        for _ in 0..36 {
            forward.push(tone_step(&mut clock, 0.125).unwrap());
        }
        assert_eq!(forward[8], 0.125);
        assert_eq!(forward[17], 0.5);
        assert_eq!(forward[26], 0.875);
        assert_eq!(forward[35], 1.0);
        assert!(clock.reversing && clock.elapsed == 0.0);
        let first = tone_step(&mut clock, 0.125).unwrap();
        // 1 - 2 (0.125 / 0.89)^2.
        assert!((first - 0.960_548).abs() < 1e-5, "{first}");
        let mut last = first;
        for _ in 0..6 {
            last = tone_step(&mut clock, 0.125).unwrap();
        }
        // elapsed 0.875: 2 (1 - 0.875 / 0.89)^2.
        assert!((last - 0.000_568).abs() < 1e-5, "{last}");
        assert_eq!(tone_step(&mut clock, 0.125), None);
        assert!(!clock.animating);
        assert_eq!(tone_step(&mut clock, 0.125), None);
    }
}
