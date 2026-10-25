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
/// pass. [`source::register_source`] runs earlier, before the asset plugin.
pub fn install(app: &mut App) {
    material::install(app);
    app.init_resource::<OcSelection>()
        .init_resource::<choice::OcChoiceBodies>()
        .add_message::<OcRequest>()
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
                dress::dress,
                dress::refuse_failed_textures,
                dress::show,
                bind::keep_avatar_hidden,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            pose::retarget
                .after(AnimationSystems)
                .before(TransformSystems::Propagate)
                .before(crate::cloth_runtime::advance),
        )
        .add_systems(Last, pose::restore_sockets);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Startup, source::from_environment);
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Startup, source::web_drop::listen);
}
