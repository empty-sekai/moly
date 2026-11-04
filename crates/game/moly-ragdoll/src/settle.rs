//! At rest: when a body has stopped moving long enough for the character to
//! get up.
//!
//! A body is at rest when every part moves slower than the settings' settle
//! speed, read per metre of the character's own height, both in the distance it
//! covers and in the way it turns. It has settled when it has stayed at rest
//! for the settle time, or when it has fallen for longer than the longest time;
//! the choice to get up is made after the frame's step ([`crate::recover`]).

use std::collections::HashMap;

use bevy::prelude::*;

/// How long each falling body has stayed at rest.
#[derive(Resource, Default)]
pub struct Settling {
    held: HashMap<Entity, f32>,
}

impl Settling {
    /// Adds the frame to a body's time at rest, or starts it again at nothing
    /// when the body is moving.
    pub fn hold(&mut self, member: Entity, still: bool, delta: f32) {
        let held = self.held.entry(member).or_insert(0.0);
        *held = if still { *held + delta } else { 0.0 };
    }

    /// How long a body has stayed at rest.
    pub fn held(&self, member: Entity) -> f32 {
        self.held.get(&member).copied().unwrap_or(0.0)
    }

    /// Forgets a body that has got up.
    pub fn forget(&mut self, member: Entity) {
        self.held.remove(&member);
    }
}
