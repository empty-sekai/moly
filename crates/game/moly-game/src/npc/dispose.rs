//! `AvatarDataStore.DisposeNPC` / `DisposeNPCAll` (JP client): the avatar
//! store removes the presenter from its list and disposes it.
//!
//! The presenter's dispose, in order: the model is marked disposed; the
//! some-character communication ends; the AI controller is disposed (its
//! current objective is cancelled without the presenter's cancel condition,
//! so even while talking, and then disposed); the state machine and the view
//! are disposed; the move and presenter tokens are cancelled and disposed;
//! the tweet listener is removed; the avatar's bundles are released.
//! `DisposeNPCAll` does this for every NPC of a copy of the list, then clears
//! the list.
//!
//! Here, before the entity goes:
//! - the objective's fixture session (a no-talk action) is disposed, which
//!   releases its fixture claims, its timeline and its animation lease;
//! - a group talk reads its members from their entities on each pass: a main
//!   that is gone ends its group (its claim is released) and a member that
//!   is gone leaves it, on the next pass;
//! - the change-site state kept for the NPC goes;
//! - the roster entry goes;
//! - the entity and its model hierarchy are despawned (route, objective,
//!   state machine, look-at and view with it).
//!
//! Named: a player talk with the NPC keeps the player's own end (the talk
//! screen is not closed here); the cut-scene avatars are the cut-scene cast's
//! (disposed there) and are not in this list.

use bevy::prelude::*;

use crate::npc::{CharacterUnitId, NpcActions};
use crate::npc_objective::ObjectiveMind;

fn frame(world: &World) -> u32 {
    world
        .get_resource::<bevy::diagnostic::FrameCount>()
        .map_or(0, |count| count.0)
}

/// The NPC list of the avatar store: every character with an AI, not the
/// player's avatar and not a cut-scene avatar, in creation order.
pub(crate) fn npc_list(world: &mut World) -> Vec<(u32, Entity)> {
    let mut npcs: Vec<(u32, Entity)> = world
        .query_filtered::<(Entity, &CharacterUnitId), (
            With<ObjectiveMind>,
            Without<crate::player::PlayerControlled>,
            Without<crate::cutscene::CutSceneAvatar>,
        )>()
        .iter(world)
        .map(|(entity, unit)| (unit.0, entity))
        .collect();
    npcs.sort_by_key(|(_, entity)| entity.index());
    npcs
}

/// `AvatarDataStore.DisposeNPC(npc)`. Returns the disposed unit.
pub(crate) fn dispose_npc(world: &mut World, entity: Entity, caller: &str) -> Result<u32, String> {
    let frame = frame(world);
    if world
        .get::<crate::player::PlayerControlled>(entity)
        .is_some()
    {
        return Err(format!("{entity:?} is the player's avatar"));
    }
    let unit = world
        .get::<CharacterUnitId>(entity)
        .map(|unit| unit.0)
        .ok_or_else(|| format!("{entity:?} is not a character"))?;
    let (objective, state, talk_owner) = {
        let mind = world.get::<ObjectiveMind>(entity);
        let actions = world.get::<NpcActions>(entity);
        (
            mind.and_then(|mind| mind.current),
            actions.map(|actions| actions.current),
            actions.and_then(|actions| actions.talk_owner),
        )
    };
    // The AI controller's dispose: the current objective's cancel and
    // dispose. A no-talk objective's fixture session releases its claims.
    let session = world
        .get_resource::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .is_some_and(|runtime| crate::npc_fixture_activity::owns_actor(runtime, entity));
    crate::npc_fixture_activity::cancel_for_greeting(world, entity);
    if let Some(mut runs) =
        world.get_resource_mut::<crate::npc::change_site_state::ChangeSiteRuns>()
    {
        runs.forget(entity);
    }
    if let Some(mut registry) = world.get_resource_mut::<crate::npc::Registry>() {
        registry.character_unit_ids.retain(|id| *id != unit);
    }
    world.despawn(entity);
    info!(
        "[npc-dispose] unit={unit} frame={frame} {caller}: DisposeNPC({entity:?}): objective {objective:?} cancelled and disposed, state {state:?}, fixture session {}; some-character communication ends with the entity; despawned{}",
        if session { "disposed (claims released)" } else { "none" },
        if talk_owner.is_some() { " (a player talk was open: its screen keeps its own end)" } else { "" },
    );
    Ok(unit)
}

/// `AvatarDataStore.DisposeNPCAll()`: every NPC of a copy of the list, in
/// list order. Returns the disposed units.
pub(crate) fn dispose_npc_all(world: &mut World, caller: &str) -> Vec<u32> {
    let list = npc_list(world);
    let mut disposed = Vec::with_capacity(list.len());
    for (unit, entity) in list {
        match dispose_npc(world, entity, caller) {
            Ok(unit) => disposed.push(unit),
            Err(reason) => warn!("[npc-dispose] unit={unit} {caller}: DisposeNPCAll: {reason}"),
        }
    }
    info!(
        "[npc-dispose] frame={} {caller}: DisposeNPCAll: {disposed:?}",
        frame(world)
    );
    disposed
}
