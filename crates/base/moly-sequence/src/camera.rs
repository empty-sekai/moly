//! Camera shots: where a shot puts the camera, and how shots blend.

use bevy_math::{EulerRot, Mat3, Quat, Vec3};
use serde_json::Value;

use crate::doc::Shake;
use crate::ease::Ease;
use crate::lane::Presence;
use crate::noise;
use crate::sample::{CameraPose, Host, Point};
use crate::spline::Spline;
use crate::Compiled;

#[derive(Clone, Debug)]
pub(crate) struct Shot {
    pub place: Place,
    pub roll: Option<Curve>,
    pub fov: Option<Curve>,
    pub near: Option<f32>,
    pub far: Option<f32>,
    pub focus: Option<Focus>,
    pub aperture: Option<f32>,
    pub shake: Option<Shake>,
}

#[derive(Clone, Debug)]
pub(crate) enum Place {
    Eye { eye: Eye, aim: Aim },
    Rig { name: String, params: Value },
}

#[derive(Clone, Debug)]
pub(crate) enum Eye {
    At(Vec3),
    Keys(Vec<(f32, Vec3, Ease)>),
    Path { spline: Spline, ease: Ease },
    Orbit { around: Point, radius: f32, height: f32, from: f32, to: f32, ease: Ease },
    Follow { actor: usize, offset: Vec3, lag: f32 },
}

#[derive(Clone, Debug)]
pub(crate) enum Aim {
    At { target: Point, lag: f32 },
    /// A fixed forward direction.
    Dir(Vec3),
    /// Along the travel of a path eye.
    Travel,
}

/// A number, or numbers at clip-local times.
#[derive(Clone, Debug)]
pub(crate) enum Curve {
    Value(f32),
    Keys(Vec<(f32, f32, Ease)>),
}

#[derive(Clone, Debug)]
pub(crate) enum Focus {
    Distance(f32),
    At(Point),
}

impl Curve {
    pub fn at(&self, local: f32) -> f32 {
        match self {
            Curve::Value(value) => *value,
            Curve::Keys(keys) => keyed(keys, local, |a, b, s| a + (b - a) * s),
        }
    }
}

/// The value of time-ordered keys at `local`: the first value before the
/// first key, the last after the last, and between two keys the ease of the
/// arriving key.
pub(crate) fn keyed<T: Copy>(keys: &[(f32, T, Ease)], local: f32, mix: impl Fn(T, T, f32) -> T) -> T {
    let next = keys.partition_point(|(t, _, _)| *t <= local);
    if next == 0 {
        return keys[0].1;
    }
    if next == keys.len() {
        return keys[next - 1].1;
    }
    let (t0, a, _) = keys[next - 1];
    let (t1, b, ease) = keys[next];
    mix(a, b, ease.apply((local - t0) / (t1 - t0)))
}

/// The forward direction for a yaw and pitch in degrees; yaw 0 faces +Z.
pub(crate) fn forward(yaw: f32, pitch: f32) -> Vec3 {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
}

/// A camera rotation looking along `forward` with +Y up, turned by `roll`
/// degrees about the view direction.
fn look_rotation(forward: Vec3, roll: f32) -> Quat {
    let back = -forward.try_normalize().unwrap_or(Vec3::NEG_Z);
    let up = if back.y.abs() > 0.9999 { Vec3::Z } else { Vec3::Y };
    let right = up.cross(back).normalize();
    let up = back.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, back)) * Quat::from_rotation_z(roll.to_radians())
}

impl CameraPose {
    /// This pose moved `weight` of the way to `other`.
    pub fn blend(&self, other: &CameraPose, weight: f32) -> CameraPose {
        if weight >= 1.0 {
            return *other;
        }
        if weight <= 0.0 {
            return *self;
        }
        let lerp = |a: f32, b: f32| a + (b - a) * weight;
        let either = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(a), Some(b)) => Some(lerp(a, b)),
            _ if weight >= 0.5 => b,
            _ => a,
        };
        CameraPose {
            eye: self.eye.lerp(other.eye, weight),
            rotation: self.rotation.slerp(other.rotation, weight),
            fov: lerp(self.fov, other.fov),
            near: lerp(self.near, other.near),
            far: lerp(self.far, other.far),
            focus: either(self.focus, other.focus),
            aperture: either(self.aperture, other.aperture),
        }
    }
}

impl Compiled {
    /// The pose a shot gives at sequence time `t`.
    pub(crate) fn shot_pose(&self, shot: &Shot, presence: &Presence, t: f32, host: &dyn Host, live: &CameraPose) -> CameraPose {
        let (eye, rotation) = match &shot.place {
            Place::Rig { name, params } => match host.rig(name, params, presence.local) {
                Some(pose) => (pose.eye, pose.rotation),
                None => (live.eye, live.rotation),
            },
            Place::Eye { eye, aim } => {
                let position = self.eye_position(eye, presence, t, host);
                let direction = match aim {
                    Aim::At { target, lag } => self.point(target, t - lag, host) - position,
                    Aim::Dir(direction) => *direction,
                    Aim::Travel => match eye {
                        Eye::Path { spline, ease } => spline.direction(ease.apply(presence.progress)),
                        _ => live.rotation * Vec3::NEG_Z,
                    },
                };
                let roll = shot.roll.as_ref().map_or(0.0, |roll| roll.at(presence.local));
                (position, look_rotation(direction, roll))
            }
        };
        let (eye, rotation) = match &shot.shake {
            Some(shake) => {
                let (offset, angles) = noise::offsets(shake, presence.elapsed);
                let tremble = Quat::from_euler(EulerRot::YXZ, angles.x.to_radians(), angles.y.to_radians(), angles.z.to_radians());
                (eye + rotation * offset, rotation * tremble)
            }
            None => (eye, rotation),
        };
        let focus = match &shot.focus {
            Some(Focus::Distance(distance)) => Some(*distance),
            Some(Focus::At(point)) => Some(self.point(point, t, host).distance(eye)),
            None => live.focus,
        };
        CameraPose {
            eye,
            rotation,
            fov: shot.fov.as_ref().map_or(live.fov, |fov| fov.at(presence.local)),
            near: shot.near.unwrap_or(live.near),
            far: shot.far.unwrap_or(live.far),
            focus,
            aperture: shot.aperture.or(live.aperture),
        }
    }

    fn eye_position(&self, eye: &Eye, presence: &Presence, t: f32, host: &dyn Host) -> Vec3 {
        match eye {
            Eye::At(position) => *position,
            Eye::Keys(keys) => keyed(keys, presence.local, |a, b, s| a.lerp(b, s)),
            Eye::Path { spline, ease } => spline.at(ease.apply(presence.progress)),
            Eye::Orbit { around, radius, height, from, to, ease } => {
                let yaw = from + (to - from) * ease.apply(presence.progress);
                let centre = self.point(around, t, host);
                let flat = forward(yaw, 0.0) * *radius;
                centre + Vec3::new(flat.x, *height, flat.z)
            }
            Eye::Follow { actor, offset, lag } => {
                let at = t - lag;
                let ahead = forward(self.actor_yaw(*actor, at), 0.0);
                let right = ahead.cross(Vec3::Y);
                self.actor_position(*actor, at) + right * offset.x + Vec3::Y * offset.y + ahead * offset.z
            }
        }
    }

    /// The camera at `t`: every present shot laid over the live camera in
    /// start order, each at its own weight. `None` when no shot is present.
    pub fn camera(&self, t: f32, host: &dyn Host) -> Option<CameraPose> {
        let layers = self.camera.layered(t);
        if layers.is_empty() {
            return None;
        }
        let live = host.live_camera();
        let mut pose = live;
        let covered = layers.iter().map(|(_, _, share)| share).sum::<f32>() >= 1.0 - 1e-4;
        for (index, (span, presence, _)) in layers.iter().enumerate() {
            let shot = self.shot_pose(&span.payload, presence, t, host, &live);
            // The bottom layer of a fully covered stack replaces the live camera.
            let weight = if index == 0 && covered { 1.0 } else { presence.weight };
            pose = pose.blend(&shot, weight);
        }
        Some(pose)
    }
}
