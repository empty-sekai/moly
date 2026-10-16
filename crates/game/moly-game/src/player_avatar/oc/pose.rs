//! The per-frame pose pass.
//!
//! After the animation systems have written the avatar skeleton's pose for
//! this frame and before transforms propagate, the OC's driven bones are
//! posed from it ([`super::rig::solve`]) and the avatar's socket joints are
//! moved onto the OC, so that everything mounted on or looking up an avatar
//! joint (tools, held items, the camera, emoticons) follows the OC this
//! frame. At the end of the frame the socket joints get the clips' own
//! translations back: the avatar's local pose stays exactly what its clips
//! and actions wrote, and the next frame starts from it.

use super::bind::OcBinding;
use super::rig::{self, Frame};
use bevy::prelude::*;

fn is_finite(frame: &Frame) -> bool {
    frame.pos.is_finite() && frame.rot.is_finite() && frame.scale.is_finite()
}

/// PostUpdate, after the animation systems and before transform
/// propagation.
pub(crate) fn retarget(mut bindings: Query<&mut OcBinding>, mut transforms: Query<&mut Transform>) {
    for mut binding in &mut bindings {
        let binding = &mut *binding;
        let mut complete = true;
        for (local, joint) in binding.locals.iter_mut().zip(&binding.proxy) {
            match transforms.get(*joint) {
                Ok(transform) => {
                    *local = Frame::new(
                        transform.translation,
                        transform.rotation,
                        (transform.scale.x + transform.scale.y + transform.scale.z) / 3.0,
                    );
                }
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete {
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
        for (bone, local) in binding.driven.iter().zip(&solution.driven_local) {
            if let Ok(mut transform) = transforms.get_mut(*bone) {
                transform.translation = local.pos;
                transform.rotation = local.rot;
                transform.scale = Vec3::splat(local.scale);
            }
        }
        super::constraint::apply(&binding.constraints, &mut transforms);
        binding.written.clear();
        for &(joint, translation) in &solution.sockets {
            if let Ok(mut transform) = transforms.get_mut(binding.proxy[joint]) {
                binding.written.push((joint, translation, transform.translation));
                transform.translation = translation;
            }
        }
    }
}

/// Last: the socket joints get the clips' translations back, unless
/// something else has written them since.
pub(crate) fn restore_sockets(mut bindings: Query<&mut OcBinding>, mut transforms: Query<&mut Transform>) {
    for mut binding in &mut bindings {
        let OcBinding { written, proxy, .. } = &mut *binding;
        for (joint, ours, source) in written.drain(..) {
            if let Ok(mut transform) = transforms.get_mut(proxy[joint]) {
                if transform.translation == ours {
                    transform.translation = source;
                }
            }
        }
    }
}
