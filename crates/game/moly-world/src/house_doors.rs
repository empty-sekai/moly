//! The placed house's door points, as the entry button reads them: the
//! `HouseView.InsideDoorActionPoint` on the house's fixture root and the site
//! generation it was looked up for. The lookup and the tag write are the site
//! move's (`crate::site_move::door`).

use bevy::prelude::*;

/// `HouseView.InsideDoorActionPoint` of the placed house, on its fixture
/// root: the house-entry button's availability reads it. The second field is
/// `OutsideDoorActionPoint`, which places the house's collision box.
#[derive(Component, Clone, Copy, Debug)]
pub struct HouseEntryPoint(pub Entity, pub Option<Entity>);

/// The site generation the house-entry point was looked up for.
#[derive(Resource)]
pub struct HouseEntryTagged(pub u64);
