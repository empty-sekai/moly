//! The field camera held at a pose: a screenshot's eye, a shot's lens, a
//! conversation's framing. Whoever holds the camera lease writes the pose;
//! [`apply_hold`] is the only place the pose reaches the camera, and it runs
//! after every camera state of the game has written its own.

use bevy::prelude::*;

use crate::lease::{Channel, Holder, Leases, Refusal};

/// The pose the camera is held at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldPose {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_degrees: f32,
}

/// The held pose, and the holder of the camera lease that wrote it.
#[derive(Resource, Default)]
pub struct CameraHold {
    pub pose: Option<HeldPose>,
    pub holder: Option<String>,
}

impl CameraHold {
    /// Holds the camera at `pose` for `holder`, who is given the camera
    /// lease: refused by name when a holder of the same or a higher rank
    /// has it.
    pub fn take(&mut self, leases: &mut Leases, holder: &Holder, pose: HeldPose) -> Result<(), Refusal> {
        leases.take(Channel::Camera, holder)?;
        self.pose = Some(pose);
        self.holder = Some(holder.id.clone());
        Ok(())
    }

    /// Holds the camera for `holder`, leaving the pose to write frame by
    /// frame; refused by name as [`CameraHold::take`] is.
    pub fn claim(&mut self, leases: &mut Leases, holder: &Holder) -> Result<(), Refusal> {
        leases.take(Channel::Camera, holder)?;
        self.holder = Some(holder.id.clone());
        Ok(())
    }

    /// Lets the camera go for `holder_id`, clearing the hold when it was
    /// its.
    pub fn release(&mut self, leases: &mut Leases, holder_id: &str) {
        leases.release(&Channel::Camera, holder_id);
        if self.holder.as_deref() == Some(holder_id) {
            self.pose = None;
            self.holder = None;
        }
    }
}

/// `Last`: after every camera state (the follow, the cutscenes, the editor,
/// the delivery) has written the field camera, so the held pose is what the
/// frame renders.
pub fn apply_hold(hold: Res<CameraHold>, mut camera: Query<(&mut Transform, &mut Projection), With<Camera3d>>) {
    let Some(held) = &hold.pose else { return };
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    *transform = Transform::from_translation(held.eye).looking_at(held.target, held.up);
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = held.fov_degrees.to_radians();
    }
}

/// `Last`: the held camera's global transform, after [`apply_hold`] has set
/// its local one: transform propagation has already run for this frame.
pub fn apply_hold_global(hold: Res<CameraHold>, mut camera: Query<(&Transform, &mut GlobalTransform), With<Camera3d>>) {
    if hold.pose.is_none() {
        return;
    }
    if let Ok((transform, mut global)) = camera.single_mut() {
        *global = GlobalTransform::from(*transform);
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<CameraHold>()
        .add_systems(Last, (apply_hold, apply_hold_global).chain());
}

/// The name a program's camera steps take their lease under.
pub fn lease_name(owner: &str) -> String {
    format!("{owner}:camera")
}
