//! `SceneMysekai.RestoreNpcAsync` (the birthday party's `RestoreCharacters`):
//! the visiting characters set up again after `DisposeNPCAll`.
//!
//! The source calls `SetupNpcVisitingAsync(visitingFromGate: false, the boot
//! data's gate characters, onCameFirstTime, hasReturnedFromOtherRoom: true)`,
//! the entry the scene's own `CreateNPCs` uses, with two differences: the
//! characters do not come in through the gate, and every one of them has its
//! first talk completed at the end (`SetCompleteFirstTalk`), so no greeting
//! is owed.
//!
//! Here the entry is the roster's own creation ([`super::spawn_when_ready`])
//! and the gate's random appearance ([`crate::npc_gate`]): [`begin`] lets both
//! run once more, and the roster created after it starts with the first talk
//! completed. [`done`] answers the scene's await.

use bevy::prelude::*;

use super::{CharacterUnitId, Registry, Spawned};

/// The next roster creation is a restore (`hasReturnedFromOtherRoom: true`).
#[derive(Resource)]
pub struct ReturnedFromOtherRoom;

/// Starts `SetupNpcVisitingAsync` again. `Err` when the roster was never
/// created (there is nothing the scene would restore from).
pub fn begin(world: &mut World) -> Result<(), String> {
    if world.remove_resource::<Spawned>().is_none() {
        return Err("the roster has not been created in this scene".into());
    }
    world.remove_resource::<Registry>();
    world
        .resource_mut::<crate::npc_gate::GateAppearance>()
        .restart();
    world.insert_resource(ReturnedFromOtherRoom);
    Ok(())
}

/// The scene's await: the roster exists, and on the home site the
/// appearance has run (on another site the members wait away from it, as at
/// the scene's start); every member on the loaded site is assembled.
pub fn done(world: &mut World) -> bool {
    if !world.contains_resource::<Spawned>() {
        return false;
    }
    let Some(ids) = world
        .get_resource::<Registry>()
        .map(|registry| registry.character_unit_ids.clone())
    else {
        return false;
    };
    let home = world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == super::residency::SITE_TYPES[0]);
    if !home || ids.is_empty() {
        return true;
    }
    if !world
        .get_resource::<crate::npc_gate::GateAppearance>()
        .is_some_and(crate::npc_gate::GateAppearance::settled)
    {
        return false;
    }
    let mut members = world.query::<(&CharacterUnitId, Has<crate::character::MotionDriver>)>();
    let members: Vec<(u32, bool)> = members
        .iter(world)
        .map(|(unit, driven)| (unit.0, driven))
        .collect();
    ids.iter()
        .all(|id| members.iter().any(|(unit, driven)| unit == id && *driven))
}
