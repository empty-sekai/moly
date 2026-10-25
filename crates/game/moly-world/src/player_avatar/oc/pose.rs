//! The per-frame pose pass.
//!
//! After the animation systems have written the avatar skeleton's pose for
//! this frame and before transforms propagate, the OC's driven bones are
//! posed from it ([``super::rig::solve``]) and the avatar's socket joints are
//! moved onto the OC, so that everything mounted on or looking up an avatar
//! joint (tools, held items, the camera, emoticons) follows the OC this
//! frame. At the end of the frame the socket joints get the clips' own
//! translations back: the avatar's local pose stays exactly what its clips
//! and actions wrote, and the next frame starts from it.
//!
//! # A body's own motion
//!
//! A body whose own animator owns it ([``OcBinding::own``]) is drawn by its own
//! clips alone, as a cast member's body is: this pass writes none of its
//! bones. The ownership's first frame is its calibration ([`calibrate`]).
//!
//! When the ownership ends ([`end`]) the body blends out of the pose its
//! clips left and into the retarget pose of the avatar skeleton, which plays
//! on through every owned frame: each driven bone's local translation and
//! rotation move from the outgoing pose to this frame's retarget pose over
//! [`OWN_BLEND`], and the retarget drives the body again when it is over.
//! The outgoing pose is read at the end of the last owned frame: the blend is
//! the only writer of a blending body's driven bones.

use super::bind::OcBinding;
use super::own::OwnAnimator;
use super::rig::{self, Frame};
use bevy::prelude::*;
use std::time::Duration;

/// The blend a body's own motion ends with: into the avatar skeleton's own
/// pose, the player locomotion states' fade.
pub const OWN_BLEND: Duration = Duration::from_millis(250);

/// The blend out of an own pose: the outgoing pose of each driven bone
/// (translation, rotation, scale) and its seconds so far.
#[derive(Component, Clone, Debug)]
pub struct OwnBlend {
    from: Vec<(Vec3, Quat, Vec3)>,
    elapsed: f32,
}

fn is_finite(frame: &Frame) -> bool {
    frame.pos.is_finite() && frame.rot.is_finite() && frame.scale.is_finite()
}

fn frame_of(transform: &Transform) -> Frame {
    Frame::new(
        transform.translation,
        transform.rotation,
        (transform.scale.x + transform.scale.y + transform.scale.z) / 3.0,
    )
}

fn set(transform: &mut Transform, frame: &Frame) {
    transform.translation = frame.pos;
    transform.rotation = frame.rot;
    transform.scale = Vec3::splat(frame.scale);
}

/// The avatar's current pose, from its joints' transforms; `false` when a
/// joint is gone.
fn read_proxy(binding: &mut OcBinding, transforms: &mut Query<&mut Transform>) -> bool {
    for (local, joint) in binding.locals.iter_mut().zip(&binding.proxy) {
        match transforms.get(*joint) {
            Ok(transform) => *local = frame_of(transform),
            Err(_) => return false,
        }
    }
    true
}

/// The body's own motion owns it: its own clips pose it, and this pass keeps
/// the bones they leave at the calibration over the avatar pose.
#[derive(Component)]
pub struct OcOwn;

/// The ownership is in its last frame: the next step blends out of the pose
/// the own clips left and back to the retarget.
#[derive(Component)]
pub struct OcOwnEnd;

/// The offer's outcome: the body keeps the avatar's motion.
#[derive(Component)]
pub struct OcOwnResolved;

/// PostUpdate, after the animation systems and before transform
/// propagation.
pub fn retarget(
    mut bindings: Query<(&mut OcBinding, Has<OwnBlend>)>,
    mut transforms: Query<&mut Transform>,
) {
    for (mut binding, blending) in &mut bindings {
        let binding = &mut *binding;
        // The body's own clips pose it.
        if binding.own {
            continue;
        }
        if !read_proxy(binding, &mut transforms) {
            continue;
        }
        rig::solve(&binding.plan, &binding.locals, &mut binding.solution);
        let solution = &binding.solution;
        let finite = solution.driven_local.iter().all(is_finite)
            && solution.sockets.iter().all(|(_, translation)| translation.is_finite());
        if !finite {
            // A degenerate avatar pose (a joint scaled to zero): the OC keeps
            // its last pose this frame.
            if !binding.warned {
                binding.warned = true;
                warn!("[oc] the avatar pose is degenerate this frame; the OC keeps its last pose");
            }
            continue;
        }
        // A blending body's driven bones are the blend's to write, toward
        // this frame's solution.
        if !blending {
            for (bone, local) in binding.driven.iter().zip(&solution.driven_local) {
                if let Ok(mut transform) = transforms.get_mut(*bone) {
                    set(&mut transform, local);
                }
            }
            super::constraint::apply(&binding.constraints, &mut transforms);
        }
        binding.written.clear();
        for &(joint, translation) in &solution.sockets {
            if let Ok(mut transform) = transforms.get_mut(binding.proxy[joint]) {
                binding.written.push((joint, translation, transform.translation));
                transform.translation = translation;
            }
        }
    }
}

/// The ownership's first frame: the own clips, which start their
/// transitions on that frame, pose the body from the next animation pass on.
pub fn calibrate(
    mut commands: Commands,
    mut starting: Query<(Entity, &mut OcBinding, Option<&mut OwnAnimator>), Added<OcOwn>>,
) {
    for (entity, mut binding, animator) in &mut starting {
        let binding = &mut *binding;
        let Some(mut animator) = animator else {
            // Without an animator there is nothing to own: the body keeps
            // the avatar's motion.
            commands.entity(entity).remove::<OcOwn>();
            commands.entity(entity).insert(OcOwnResolved);
            continue;
        };
        binding.own = true;
        animator.playing_body = true;
        info!("[oc] {entity:?}: the body's own motion plays");
    }
}

/// PostUpdate, after the pose pass (which wrote the last owned frame's pose
/// and the node constraints' turns on it): the end of an ownership reads
/// the pose its own clips left as the blend's outgoing pose.
pub fn end(
    mut commands: Commands,
    mut owners: Query<(Entity, &mut OcBinding, Option<&mut OwnAnimator>), Added<OcOwnEnd>>,
    driven: Query<&Transform>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (entity, mut binding, animator) in &mut owners {
        let binding = &mut *binding;
        if !binding.own {
            commands.entity(entity).remove::<OcOwnEnd>();
            continue;
        }
        // The own clips stop posing the body: the retarget pose of this
        // frame is what the blend goes to, and stays once it has arrived.
        if let Some(mut animator) = animator {
            animator.playing_body = false;
            if let Ok(mut player) = players.get_mut(animator.animator) {
                player.pause_all();
            }
        }
        let mut from = Vec::with_capacity(binding.driven.len());
        for bone in &binding.driven {
            if let Ok(transform) = driven.get(*bone) {
                from.push((transform.translation, transform.rotation, transform.scale));
            }
        }
        if from.len() != binding.driven.len() {
            from.clear();
        }
        binding.own = false;
        commands
            .entity(entity)
            .insert(OwnBlend {
                from,
                elapsed: 0.0,
            })
            .remove::<OcOwnEnd>();
        info!(
            "[oc] {:?}: the body's own motion ends; the avatar motion blends back over {:.2}s",
            entity,
            OWN_BLEND.as_secs_f32()
        );
    }
}

/// PostUpdate, after the pose pass: a blending body's driven bones move from
/// the own pose to the retarget pose, and the retarget drives them again
/// once the blend is over.
pub fn blend(
    time: Res<Time>,
    mut commands: Commands,
    mut bindings: Query<(Entity, &mut OcBinding, Option<&mut OwnBlend>)>,
    mut transforms: Query<&mut Transform>,
) {
    for (entity, mut binding, blend) in &mut bindings {
        let Some(mut blend) = blend else {
            continue;
        };
        blend.elapsed += time.delta_secs();
        let t = (blend.elapsed / OWN_BLEND.as_secs_f32()).clamp(0.0, 1.0);
        let binding = &mut *binding;
        let incoming = &binding.solution.driven_local;
        let from = &blend.from;
        if from.len() == binding.driven.len() && incoming.len() == binding.driven.len() {
            for ((bone, target), (pos, rot, scale)) in
                binding.driven.iter().zip(incoming).zip(from.iter())
            {
                if let Ok(mut transform) = transforms.get_mut(*bone) {
                    transform.translation = pos.lerp(target.pos, t);
                    transform.rotation = rot.slerp(target.rot, t);
                    transform.scale = scale.lerp(Vec3::splat(target.scale), t);
                }
            }
        }
        if t >= 1.0 {
            commands.entity(entity).remove::<OwnBlend>();
        }
    }
}
