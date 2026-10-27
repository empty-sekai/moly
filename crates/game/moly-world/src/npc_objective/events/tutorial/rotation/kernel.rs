use crate::npc_objective::events::SourceAnimationMap;
use bevy::prelude::*;

pub(super) const MALE: [[&str; 3]; 2] = [
    [
        "mov_cm_normal_turn45_l",
        "mov_cm_normal_turn90_l",
        "mov_cm_normal_turn180_l",
    ],
    [
        "mov_cm_normal_turn45_r",
        "mov_cm_normal_turn90_r",
        "mov_cm_normal_turn180_r",
    ],
];
const FEMALE: [[&str; 3]; 2] = [
    [
        "mov_cw_normal_turn45_l",
        "mov_cw_normal_turn90_l",
        "mov_cw_normal_turn180_l",
    ],
    [
        "mov_cw_normal_turn45_r",
        "mov_cw_normal_turn90_r",
        "mov_cw_normal_turn180_r",
    ],
];

pub(super) fn is_turn_clip(name: &str) -> bool {
    ["_S", "_L", "_E", "_O"]
        .into_iter()
        .filter_map(|suffix| name.strip_suffix(suffix))
        .any(|family| {
            MALE.iter()
                .chain(FEMALE.iter())
                .flatten()
                .any(|known| *known == family)
        })
}

pub(super) struct Plan {
    pub family: Option<&'static str>,
    pub duration: f32,
}

pub(super) fn plan(angle: f32, requested: f32, male: bool, clips: &SourceAnimationMap) -> Plan {
    if angle.abs() < 5.0 {
        return Plan {
            family: None,
            duration: requested,
        };
    }
    let magnitude = angle.abs();
    let step = if magnitude <= 60.0 {
        0
    } else if magnitude <= 120.0 {
        1
    } else {
        2
    };
    let table = if male { &MALE } else { &FEMALE };
    let family = table[usize::from(angle >= 0.0)][step];
    let fallback = if requested == 0.0 {
        magnitude / 60.0
    } else {
        requested
    };
    let duration = clips
        .get(&format!("{family}_S"))
        .map_or(fallback, |clip| clip.length);
    Plan {
        family: Some(family),
        duration,
    }
}

pub(super) fn delta_angle(current: f32, target: f32) -> f32 {
    let delta = target - current;
    let repeated = (delta - (delta / 360.0).floor() * 360.0).clamp(0.0, 360.0);
    if repeated > 180.0 {
        repeated - 360.0
    } else {
        repeated
    }
}

/// The asset pipeline reflects X. Quaternion reflection is its own inverse.
pub(super) fn reflect(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

pub(super) fn euler(q: Quat) -> Vec3 {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()).map(moly_law::path::make_positive_yaw)
}

pub(super) struct LocalTween {
    target: Vec3,
    start: Option<(Vec3, Vec3)>,
    duration: f32,
    elapsed: f32,
}

impl LocalTween {
    pub fn new(target: Vec3, duration: f32) -> Self {
        Self {
            target,
            start: None,
            duration,
            elapsed: 0.0,
        }
    }
    /// Source-space local pose. DOTween captures its getter on startup.
    pub fn step(&mut self, local: Quat, dt: f32) -> (Quat, bool) {
        let (start, change) = *self.start.get_or_insert_with(|| {
            let start = euler(local);
            let fast = moly_law::path::look_at::fast_change;
            (
                start,
                Vec3::new(
                    fast(start.x, self.target.x),
                    fast(start.y, self.target.y),
                    fast(start.z, self.target.z),
                ),
            )
        });
        self.elapsed = (self.elapsed + dt).min(self.duration);
        let complete = self.duration <= 0.0 || self.elapsed >= self.duration;
        let ease = if complete {
            1.0
        } else {
            moly_law::ui::dotween::Ease::OutQuad.evaluate(self.elapsed, self.duration)
        };
        let angle = (start + change * ease).map(f32::to_radians);
        (
            Quat::from_euler(EulerRot::YXZ, angle.y, angle.x, angle.z),
            complete,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_objective::events::SourceAnimationClip;

    #[test]
    // The presenter's DeltaAngle and 5/60/120 gates, with both authored sides.
    fn source_delta_and_all_boundaries_choose_authored_sides() {
        assert_eq!(delta_angle(0., -180.), 180.);
        assert_eq!(delta_angle(0., 180.), 180.);
        assert_eq!(delta_angle(359., 1.), 2.);
        assert_eq!(delta_angle(1., 359.), -2.);
        let clips = SourceAnimationMap::new();
        for (a, step) in [
            (5., 45),
            (60., 45),
            (60.001, 90),
            (120., 90),
            (120.001, 180),
            (180., 180),
        ] {
            for (sign, side) in [(1., "r"), (-1., "l")] {
                assert_eq!(
                    plan(sign * a, 0.75, true, &clips).family.unwrap(),
                    format!("mov_cm_normal_turn{step}_{side}")
                );
                assert_eq!(
                    plan(sign * a, 0.75, false, &clips).family.unwrap(),
                    format!("mov_cw_normal_turn{step}_{side}")
                );
            }
        }
        assert!(plan(4.999, 0.75, true, &clips).family.is_none());
    }
    #[test]
    // The view's clip.length is read only for |angle|>=5; a requested time of
    // zero falls back to |angle|/60 before looking up that source clip.
    fn small_rotation_does_not_sync_and_missing_start_uses_supplied_fallback() {
        let mut clips = SourceAnimationMap::new();
        clips.insert(
            "mov_cw_normal_turn45_r_S".into(),
            SourceAnimationClip {
                exported_name: "turn".into(),
                length: 1.0000004768371582,
            },
        );
        assert_eq!(plan(4., 0.75, true, &clips).duration, 0.75);
        assert_eq!(plan(45., 0.75, false, &clips).duration, 1.0000004768371582);
        assert_eq!(plan(90., 0.75, true, &clips).duration, 0.75);
        assert_eq!(plan(90., 0., true, &clips).duration, 1.5);
    }
    #[test]
    // DOLocalRotate(Fast), DOTween OutQuad. Final quaternion is
    // the independently executed Unity FromEulerRad(order4) result at 30/40/20.
    fn tween_restores_all_axes_with_fast_outquad_and_reflection() {
        let target = Vec3::new(30., 40., 20.);
        let mut tween = LocalTween::new(target, 1.0);
        let (half, done) = tween.step(Quat::IDENTITY, 0.5);
        assert!(!done);
        let expected = Quat::from_euler(
            EulerRot::YXZ,
            30_f32.to_radians(),
            22.5_f32.to_radians(),
            15_f32.to_radians(),
        );
        assert!(half.dot(expected).abs() > 0.999999);
        let end = tween.step(half, 0.5).0;
        let expected = Quat::from_xyzw(0.2968829, 0.283114, 0.07043934, 0.9092553);
        assert!(end.dot(expected).abs() > 0.999999);
        let canonical = reflect(end);
        assert!(reflect(canonical).dot(end).abs() > 0.999999);
        assert!(delta_angle(0., euler(reflect(Quat::from_rotation_y(0.5))).y) < 0.);
    }
}
