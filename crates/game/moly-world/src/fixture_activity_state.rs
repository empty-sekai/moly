//! Shared instance identity and reservations for furniture activity owners.
//!
//! A future NPC use, an active action slot, and a player's locate-index use
//! have different lifetimes. They deliberately remain separate maps. Every
//! entry is owned by an actor generation, so delayed cleanup cannot release
//! a seat acquired by a later activity.

use bevy::prelude::*;
use std::collections::HashMap;

/// Supplied by the placed-instance/save owner, never inferred from a model
/// name, a dialogue anchor id, or a quantized world position.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct FixtureActivityIdentity {
    pub uid: String,
    pub master_id: i32,
    pub model_package: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixtureTarget {
    pub entity: Entity,
    pub uid: String,
}

impl FixtureTarget {
    pub fn matches(&self, identity: &FixtureActivityIdentity) -> bool {
        !self.uid.is_empty() && self.uid == identity.uid
    }
}

/// The business activity's generation, shared with its timeline ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FixtureActivityOwner {
    pub actor: Entity,
    pub generation: u64,
}

/// The cut-scene holds this object for its director: a cast member (its
/// animator, root pose and activity) or the fixture a track binds. The world's
/// character and timeline systems read it to yield the object, so it lives
/// with the activity owner it carries.
#[derive(Component, Clone, Copy, Debug)]
pub struct CutSceneCastLease(pub FixtureActivityOwner);

#[derive(Resource, Default)]
pub struct FixtureActivityReservations {
    npc_future: HashMap<FixtureTarget, FixtureActivityOwner>,
    action_slots: HashMap<(FixtureTarget, i32), FixtureActivityOwner>,
    player_points: HashMap<(FixtureTarget, usize), FixtureActivityOwner>,
}

impl FixtureActivityReservations {
    pub fn npc_target_in_use(&self, target: &FixtureTarget) -> bool {
        self.npc_future.contains_key(target)
    }

    pub fn npc_target_owner(&self, target: &FixtureTarget) -> Option<FixtureActivityOwner> {
        self.npc_future.get(target).copied()
    }

    pub fn reserve_npc_target(
        &mut self,
        target: &FixtureTarget,
        owner: FixtureActivityOwner,
    ) -> bool {
        if self
            .npc_future
            .get(target)
            .is_some_and(|held| *held != owner)
        {
            return false;
        }
        self.npc_future.insert(target.clone(), owner);
        true
    }

    pub fn action_slot_in_use(&self, target: &FixtureTarget, slot_id: i32) -> bool {
        self.action_slots.contains_key(&(target.clone(), slot_id))
    }

    pub fn action_slot_owner(
        &self,
        target: &FixtureTarget,
        slot_id: i32,
    ) -> Option<FixtureActivityOwner> {
        self.action_slots.get(&(target.clone(), slot_id)).copied()
    }

    pub fn reserve_action_slot(
        &mut self,
        target: &FixtureTarget,
        slot_id: i32,
        owner: FixtureActivityOwner,
    ) -> bool {
        let key = (target.clone(), slot_id);
        if self
            .action_slots
            .get(&key)
            .is_some_and(|held| *held != owner)
        {
            return false;
        }
        self.action_slots.insert(key, owner);
        true
    }

    pub fn player_point_in_use(&self, target: &FixtureTarget, locate_index: usize) -> bool {
        self.player_points
            .contains_key(&(target.clone(), locate_index))
    }

    /// Atomically reserve the action slot and the player's original array
    /// index. Neither numeric identity can substitute for the other.
    pub fn reserve_player_slot(
        &mut self,
        target: &FixtureTarget,
        slot_id: i32,
        locate_index: usize,
        owner: FixtureActivityOwner,
    ) -> bool {
        let action_key = (target.clone(), slot_id);
        let point_key = (target.clone(), locate_index);
        if self
            .action_slots
            .get(&action_key)
            .is_some_and(|held| *held != owner)
            || self
                .player_points
                .get(&point_key)
                .is_some_and(|held| *held != owner)
        {
            return false;
        }
        self.action_slots.insert(action_key, owner);
        self.player_points.insert(point_key, owner);
        true
    }

    pub fn owns_player_slot(
        &self,
        target: &FixtureTarget,
        slot_id: i32,
        locate_index: usize,
        owner: FixtureActivityOwner,
    ) -> bool {
        self.action_slots.get(&(target.clone(), slot_id)) == Some(&owner)
            && self.player_points.get(&(target.clone(), locate_index)) == Some(&owner)
    }

    pub fn release_owner(&mut self, owner: FixtureActivityOwner) {
        self.npc_future.retain(|_, held| *held != owner);
        self.action_slots.retain(|_, held| *held != owner);
        self.player_points.retain(|_, held| *held != owner);
    }
}
