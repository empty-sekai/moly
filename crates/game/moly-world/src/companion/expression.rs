//! A companion's face: the eye pattern and the mouth (lip) pattern of its
//! worn character body, written the way the roster's tweet state writes
//! them ([`crate::npc_state`]'s `ChangeEyePattern` and
//! `ChangeLipSyncPattern`): the pattern row of the facial tables and its
//! atlas cell in the body's `eye` or `mouth` character material.
//!
//! The body owns the materials its rig profile planned
//! ([`crate::character_material::ToonMaterials`]), so a companion carries
//! the same `eye` and `mouth` slots a cast member does. A pattern the tables
//! do not hold takes the tables' zero-valued row, as the roster's does.
//!
//! A body whose materials are not in yet takes the face as pending: it is
//! written on the first frame the body is dressed ([`apply`]).

use crate::alone_action_runtime::FacialTables;
use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::player_avatar::oc::OcModel;
use bevy::prelude::*;
use moly_law::blink::EyePattern;
use moly_law::facial::LipPattern;

/// The face a companion shows: the eye and mouth patterns last written on
/// its body, or the one waiting for the body's materials.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct CompanionFace {
    pub eye: Option<String>,
    pub mouth: Option<String>,
    /// The face waits for the body's materials.
    pub pending: bool,
}

/// The pattern rows a face write holds: the eye row and the mouth row, each
/// `None` when that pattern is left as it is.
type Rows = (Option<EyePattern>, Option<LipPattern>);

/// The reason a face write is refused.
const NO_TABLES: &str = "the facial tables are not loaded";

/// The pattern rows of the eye pattern `eye` and the mouth pattern `mouth`
/// (each `None` left as it is), a name the tables do not hold reading as the
/// tables' zero-valued row.
fn rows(tables: &FacialTables, eye: Option<&str>, mouth: Option<&str>) -> Rows {
    (
        eye.map(|pattern| tables.eye_pattern(pattern).unwrap_or_default()),
        mouth.map(|pattern| tables.lip_pattern(pattern).unwrap_or_default()),
    )
}

/// Writes the pattern rows `rows` on the material slots of a companion's
/// body.
fn write(
    materials: &mut Assets<CharacterMaterial>,
    rows: Rows,
    eye_handle: &Handle<CharacterMaterial>,
    mouth_handle: &Handle<CharacterMaterial>,
) {
    if let Some(row) = rows.0 {
        crate::alone_action_runtime::apply_eye_pattern(materials, eye_handle, Some(row));
    }
    if let Some(row) = rows.1 {
        crate::alone_action_runtime::apply_mouth_pattern(materials, mouth_handle, row);
    }
}

/// Writes the eye pattern `eye` and the mouth pattern `mouth` (each `None`
/// left as it is) on the companion `entity`, and records them on it. A body
/// whose materials are not in yet takes them for the first frame it is
/// dressed. The reason names what refuses it: the facial tables are not
/// loaded, or the body's materials have no such slot.
pub fn set(
    world: &mut World,
    entity: Entity,
    eye: Option<&str>,
    mouth: Option<&str>,
) -> Result<(), String> {
    if world.get_resource::<FacialTables>().is_none() {
        return Err(NO_TABLES.to_owned());
    }
    if world.get::<CompanionFace>(entity).is_none() {
        world.entity_mut(entity).insert(CompanionFace::default());
    }
    // The materials are the character body's, on the model the companion
    // wears; a body not dressed yet takes the face once it is.
    let slots = match body_materials(world, entity) {
        Some(toon) => {
            let slots = (toon.slot_handle("eye").cloned(), toon.slot_handle("mouth").cloned());
            if eye.is_some() && slots.0.is_none() {
                return Err("the body has no eye material slot".to_owned());
            }
            if mouth.is_some() && slots.1.is_none() {
                return Err("the body has no mouth material slot".to_owned());
            }
            slots
        }
        None => (None, None),
    };
    let written = match slots {
        (Some(eye_handle), Some(mouth_handle)) => {
            let rows = {
                let tables = world.resource::<FacialTables>();
                rows(tables, eye, mouth)
            };
            write(
                &mut world.resource_mut::<Assets<CharacterMaterial>>(),
                rows,
                &eye_handle,
                &mouth_handle,
            );
            true
        }
        _ => false,
    };
    let mut face = world
        .get_mut::<CompanionFace>(entity)
        .ok_or_else(|| "the companion is gone".to_owned())?;
    if let Some(pattern) = eye {
        face.eye = Some(pattern.to_owned());
    }
    if let Some(pattern) = mouth {
        face.mouth = Some(pattern.to_owned());
    }
    face.pending = !written;
    Ok(())
}

/// Returns the companion `entity` to the default face of its unit in the
/// facial tables ([`FacialTables::default_patterns`]). A unit whose row the
/// tables do not hold is refused by name; its face is left as it is.
pub fn reset(world: &mut World, entity: Entity) -> Result<(), String> {
    let unit = world
        .get::<super::Companion>(entity)
        .map(|companion| companion.unit)
        .ok_or_else(|| "not a companion".to_owned())?;
    let tables = world
        .get_resource::<FacialTables>()
        .ok_or_else(|| NO_TABLES.to_owned())?;
    let defaults = tables
        .default_patterns(unit)
        .map(|(eye, mouth)| (eye.to_owned(), mouth.to_owned()))
        .ok_or_else(|| format!("the facial tables have no default face for unit {unit}"))?;
    set(world, entity, Some(&defaults.0), Some(&defaults.1))
}

/// Update: a face that waited for the body's materials is written on the
/// first frame the body is dressed.
pub fn apply(
    mut commands: Commands,
    faces: Query<(Entity, &CompanionFace)>,
    bodies: Query<(&OcModel, &ToonMaterials)>,
) {
    for (entity, face) in &faces {
        if !face.pending {
            continue;
        }
        let Some((_, toon)) = bodies.iter().find(|(model, _)| model.player == entity) else {
            continue;
        };
        let (Some(eye_handle), Some(mouth_handle)) = (
            toon.slot_handle("eye").cloned(),
            toon.slot_handle("mouth").cloned(),
        ) else {
            continue;
        };
        let (eye, mouth) = (face.eye.clone(), face.mouth.clone());
        commands.queue(move |world: &mut World| {
            let Some(rows) = world
                .get_resource::<FacialTables>()
                .map(|tables| rows(tables, eye.as_deref(), mouth.as_deref()))
            else {
                return;
            };
            write(
                &mut world.resource_mut::<Assets<CharacterMaterial>>(),
                rows,
                &eye_handle,
                &mouth_handle,
            );
            if let Some(mut face) = world.get_mut::<CompanionFace>(entity) {
                face.pending = false;
            }
            info!("[companion] {entity:?}: the face waited for the body's materials and is written");
        });
    }
}

/// The materials of the character body the companion `entity` wears, once
/// it is dressed.
fn body_materials(world: &mut World, entity: Entity) -> Option<&ToonMaterials> {
    let mut bodies = world.query::<(&OcModel, &ToonMaterials)>();
    bodies
        .iter(world)
        .find(|(model, _)| model.player == entity)
        .map(|(_, toon)| toon)
}
