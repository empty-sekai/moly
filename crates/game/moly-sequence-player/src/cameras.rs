//! The camera: the live camera as playback started, and the held pose a
//! shot lays over it.
//!
//! The camera lease is taken once, when playback starts; every frame with a
//! shot holds its pose under it ([`CameraHold::take`] writes the pose), and a
//! frame without a shot lets the hold go, so the camera is back at the live
//! one. A holder of the same or a higher rank takes the camera from the
//! player, and the refused take stops the playback by name.

use bevy::prelude::*;
use moly_features::camera_hold::{CameraHold, HeldPose};
use moly_features::lease::{Channel, Holder, Leases};
use moly_sequence::CameraPose;

use crate::Player;

/// The player's camera half: the holder its lease is under, and whether the
/// camera is held this frame.
#[derive(Default)]
pub struct Held {
    pub holder: Option<Holder>,
    /// The pose is held; the next frame without a shot lets it go.
    pub holding: bool,
}

/// Takes the camera for `holder`, kept on `player`.
pub fn claim(world: &mut World, player: &mut Player, holder: &Holder) -> Result<(), String> {
    world.resource_mut::<Leases>().take(Channel::Camera, holder).map_err(|refusal| refusal.to_string())?;
    player.camera.holder = Some(holder.clone());
    player.camera.holding = false;
    Ok(())
}

/// Lets the camera go: at a stop, or to frame a shot with the free camera
/// while the player is paused.
pub fn release(world: &mut World, player: &mut Player) {
    let Some(holder) = player.camera.holder.take() else {
        return;
    };
    let mut leases = std::mem::take(&mut *world.resource_mut::<Leases>());
    if let Some(mut hold) = world.get_resource_mut::<CameraHold>() {
        hold.release(&mut leases, &holder.id);
    } else {
        leases.release(&Channel::Camera, &holder.id);
    }
    *world.resource_mut::<Leases>() = leases;
    player.camera.holding = false;
}

/// Holds the frame's camera: a shot's pose over the live one, or the hold let
/// go when the frame has no shot. A refused lease names its holder.
pub fn lay(world: &mut World, player: &mut Player, pose: Option<CameraPose>) -> Result<(), String> {
    let Some(holder) = player.camera.holder.clone() else {
        return Ok(());
    };
    let Some(pose) = pose else {
        release(world, player);
        return Ok(());
    };
    let held = HeldPose { eye: pose.eye, target: pose.eye + pose.rotation * Vec3::NEG_Z, up: pose.rotation * Vec3::Y, fov_degrees: pose.fov };
    let mut hold = std::mem::take(&mut *world.resource_mut::<CameraHold>());
    let mut leases = std::mem::take(&mut *world.resource_mut::<Leases>());
    let taken = hold.take(&mut leases, &holder, held).map_err(|refusal| refusal.to_string());
    *world.resource_mut::<CameraHold>() = hold;
    *world.resource_mut::<Leases>() = leases;
    taken.map(|()| player.camera.holding = true)
}

/// The live camera now: the first 3D camera's place, its vertical field of
/// view and its clip planes. Its focus and aperture are unset.
pub fn live(world: &mut World) -> Option<CameraPose> {
    let mut cameras = world.query_filtered::<(&GlobalTransform, &Projection), With<Camera3d>>();
    let (global, projection) = cameras.iter(world).next()?;
    let (near, far) = match projection {
        Projection::Perspective(perspective) => (perspective.near, perspective.far),
        Projection::Orthographic(orthographic) => (orthographic.near, orthographic.far),
        Projection::Custom(_) => (0.1, 1000.0),
    };
    let fov = moly_features::camera::perspective_fov_deg(projection);
    Some(CameraPose { eye: global.translation(), rotation: global.rotation(), fov, near, far, focus: None, aperture: None })
}
