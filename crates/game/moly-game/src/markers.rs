//! Markers and shared state that every layer of the game reads: which site is
//! active, which entity the player controls, the roots of character avatars.
//! The domains above define the behaviour; this module holds only the types
//! they share.

use bevy::prelude::*;
