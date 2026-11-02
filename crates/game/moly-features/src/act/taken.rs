//! The characters the actions took, as the run that took one sees it, and
//! the fixture activities they started.

use bevy::prelude::*;

use crate::npc_fixture_activity::preview::{self, PreviewPhase};

/// The characters the actions took. A character is taken once, by the run
/// that took it (its ticket): its AI makes no decision again until the run
/// ends or a step gives it back, and a second take of it is the same take.
#[derive(Resource, Default)]
pub struct Taken {
    taken: Vec<Entry>,
}

#[derive(Clone, Debug)]
struct Entry {
    unit: u32,
    /// The run that took the character.
    ticket: u64,
}

impl Taken {
    /// The body of `unit` when the world has it.
    pub fn body(world: &mut World, unit: u32) -> Result<Entity, String> {
        world
            .query_filtered::<(Entity, &crate::npc::CharacterUnitId), (
                With<crate::npc_objective::ObjectiveMind>,
                Without<crate::player::PlayerControlled>,
                Without<crate::cut_scene::CutSceneAvatar>,
            )>()
            .iter(world)
            .find(|(_, id)| id.0 == unit)
            .map(|(entity, _)| entity)
            .ok_or_else(|| format!("character {unit} is not on this site"))
    }

    /// Takes `unit` for the run `ticket`; `Ok(false)` when it is taken
    /// already, and the body is held no further. `Err` names what refuses the
    /// body (its AI's loop has ended, it is in a conversation).
    pub fn take(world: &mut World, ticket: u64, unit: u32) -> Result<(Entity, bool), String> {
        if let Some(entity) = Self::taken(world, unit) {
            return Ok((entity, false));
        }
        let entity = crate::npc_take::take(world, unit)?;
        world.resource_mut::<Taken>().taken.push(Entry { unit, ticket });
        Ok((entity, true))
    }

    /// The body of `unit`, when any run took it.
    pub fn taken(world: &mut World, unit: u32) -> Option<Entity> {
        if !world.resource::<Taken>().taken.iter().any(|entry| entry.unit == unit) {
            return None;
        }
        Self::body(world, unit).ok()
    }

    /// Whether `ticket`'s run took `unit`.
    pub fn counts(&self, ticket: u64, unit: u32) -> bool {
        self.taken.iter().any(|entry| entry.unit == unit && entry.ticket == ticket)
    }

    /// Whether any run took `unit`.
    pub fn counts_any(&self, unit: u32) -> bool {
        self.taken.iter().any(|entry| entry.unit == unit)
    }

    /// Gives the character of `unit` back to its AI and forgets every take of
    /// it. Answers whether it was taken at all.
    pub fn untake(world: &mut World, unit: u32) -> bool {
        let known = {
            let mut taken = world.resource_mut::<Taken>();
            let known = taken.taken.iter().any(|entry| entry.unit == unit);
            taken.taken.retain(|entry| entry.unit != unit);
            known
        };
        if known {
            if let Ok(entity) = Self::body(world, unit) {
                crate::npc_take::release(world, entity);
            }
        }
        known
    }

    /// Hands the body `entity` back to its AI, whatever run took it: a
    /// character the AI now plays a talk with is the AI's own again.
    pub fn hand_back(world: &mut World, entity: Entity) {
        let unit = world.get::<crate::npc::CharacterUnitId>(entity).map(|unit| unit.0);
        if let Some(unit) = unit {
            world.resource_mut::<Taken>().taken.retain(|entry| entry.unit != unit);
        }
        crate::npc_take::release(world, entity);
    }

    /// Gives every character the run `ticket`'s steps still count back to its
    /// AI.
    pub fn release(world: &mut World, ticket: u64) {
        let units: Vec<u32> = {
            let mut taken = world.resource_mut::<Taken>();
            let mine: Vec<u32> = taken.taken.iter().filter(|entry| entry.ticket == ticket).map(|entry| entry.unit).collect();
            taken.taken.retain(|entry| entry.ticket != ticket);
            mine
        };
        for unit in units {
            if let Ok(entity) = Self::body(world, unit) {
                crate::npc_take::release(world, entity);
            }
        }
    }

    /// The units the run `ticket` took, in the order they were taken.
    pub fn units(&self, ticket: u64) -> Vec<u32> {
        self.taken.iter().filter(|entry| entry.ticket == ticket).map(|entry| entry.unit).collect()
    }
}

/// The fixture activities the actions started, by the ticket of their record
/// in [`crate::npc_fixture_activity::preview`].
#[derive(Resource, Default)]
pub struct Activities {
    started: Vec<u64>,
}

impl Activities {
    /// Remembers the activity `ticket` a step started.
    pub fn keep(world: &mut World, ticket: u64) {
        world.resource_mut::<Activities>().started.push(ticket);
    }

    /// The phase of the activity `ticket`, with its last note.
    pub fn phase(world: &World, ticket: u64) -> Option<(PreviewPhase, Option<String>)> {
        world.get_resource::<preview::DirectorPreviews>()?.0.get(&ticket).cloned()
    }

    /// Ends the activity `ticket` (and its session when `cancel`) and forgets
    /// it.
    pub fn end(world: &mut World, ticket: u64, cancel: bool) {
        if cancel {
            preview::cancel(world, ticket);
        }
        if let Some(mut ledger) = world.get_resource_mut::<preview::DirectorPreviews>() {
            ledger.0.remove(&ticket);
        }
        world.resource_mut::<Activities>().started.retain(|known| *known != ticket);
    }
}
