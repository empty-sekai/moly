//! A humanoid model (an "OC") as the player's body, moved by the player
//! avatar's motion set: a character body of the cast, or a model file of the
//! player's own.
//!
//! The avatar body still loads, wires and plays every clip and action: its
//! skeleton is the animation target and carries every socket. While an OC
//! is worn, the avatar's body mesh is hidden and each frame the OC's bones
//! are posed from the avatar's joints by the retarget law in [`rig`]; the
//! avatar's socket joints are moved onto the OC's bones for that frame.
//!
//! - [`humanoid`]: the humanoid bone vocabulary and the avatar's sixteen
//!   joints;
//! - [`vrm`]: reading a model file's humanoid map;
//! - [`source`]: choosing the character body or the file, and its load;
//! - [`bind`]: fitting a loaded model to the avatar skeleton;
//! - [`dress`]: a character body's character material;
//! - [`pose`]: the per-frame pose pass;
//! - [`own`]: a body's own idle and walk segments, played on the body's own
//!   skeleton in place of the retarget, with the fade back into it;
//! - [`constraint`]: the model file's node constraints, applied after the
//!   pose;
//! - [`material`]: the OC's material.
//!
//! Without a chosen model, or when a model is refused, the avatar body is
//! drawn as before.

pub mod bind;
pub mod choice;
pub mod constraint;
pub mod dress;
pub mod humanoid;
pub mod material;
pub mod own;
pub mod pose;
pub mod rig;
pub mod source;
pub mod vrm;

pub use bind::{capture_proxy, OcModel};
pub use choice::{OcChoice, OcChoiceLoad};
pub use material::OcMaterial;
pub use source::{CharacterChoice, OcRequest, OcSelection};

use bevy::app::AnimationSystems;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

/// Registers the OC body: its material, the choice, binding and the pose
/// pass, and the bodies' own motions. [`source::register_source`] runs
/// earlier, before the asset plugin.
///
/// The own motion's frame order: the segment changes in Update
/// ([`own::OwnAnimator::drive`], their transition starts with them); the
/// start, ask and end commands in Update ([`own::offer`], after the
/// commands that write them); the animation systems pose both skeletons in
/// PostUpdate; the pose pass ([`pose::retarget`], after
/// [`AnimationSystems`]) keeps the bones the own clips leave; the
/// calibration of a first owned frame and the end of an ownership follow it
/// ([`pose::calibrate`], [`pose::end`]). The end pauses the own clips, so
/// no later frame of them overwrites the blend, which moves the driven bones
/// from the own pose to the retarget pose ([`pose::blend`], last in
/// PostUpdate before propagation); after it the retarget drives the body
/// again.
pub fn install(app: &mut App) {
    material::install(app);
    app.init_resource::<OcSelection>()
        .init_resource::<choice::OcChoiceBodies>()
        .init_resource::<own::CharacterProfiles>()
        .add_message::<OcRequest>()
        .add_message::<own::Retail>()
        .add_message::<own::PlayOwn>()
        .add_message::<own::EndOwn>()
        .add_observer(bind::on_scene_ready)
        .add_systems(
            Update,
            (
                source::requests,
                source::choose_character,
                source::receive,
                source::poll_load,
                choice::request,
                bind::attach,
                bind::bind,
                own::load_profiles,
                dress::dress,
                dress::refuse_failed_textures,
                dress::show,
                bind::keep_avatar_hidden,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                own::prepare.after(bind::bind),
                own::OwnAnimator::drive,
                own::offer,
            ),
        )
        .add_systems(
            PostUpdate,
            (
                pose::retarget
                    .after(AnimationSystems)
                    .before(TransformSystems::Propagate)
                    .before(crate::cloth_runtime::advance),
                pose::calibrate.after(pose::retarget),
                pose::end.after(pose::calibrate),
                pose::blend
                    .after(pose::end)
                    .before(TransformSystems::Propagate),
            )
                .chain(),
        )
        .add_systems(Last, restore_sockets);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Startup, source::from_environment);
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Startup, source::web_drop::listen);
}

/// Last: the socket joints get the clips' translations back, unless
/// something else has written them since.
pub fn restore_sockets(
    mut bindings: Query<&mut bind::OcBinding>,
    mut transforms: Query<&mut Transform>,
) {
    for mut binding in &mut bindings {
        let bind::OcBinding { written, proxy, .. } = &mut *binding;
        for (joint, ours, source) in written.drain(..) {
            if let Ok(mut transform) = transforms.get_mut(proxy[joint]) {
                if transform.translation == ours {
                    transform.translation = source;
                }
            }
        }
    }
}