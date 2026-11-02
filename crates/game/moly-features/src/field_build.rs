//! The layout editor's field build, as a step reaches it.
//!
//! A field build puts a fixture through the editor's own put search, save
//! checks, layout save and reload, with no editor session open. The crate
//! that holds the editor installs a [`FieldBuilder`] with those calls, and a
//! `companion.build` step calls them through it, so the step's crate needs
//! no edge to the editor's.

use bevy::prelude::*;
use moly_law::fixture::{Direction, GridPosition};

/// Where a field build stands.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldBuildState {
    /// The housing layout call is out.
    Saving,
    /// Saved; the layout is reloading and the walk cells rebaking.
    Reloading,
    /// The fixture `uid` is in the layout, spawned, with the walk cells
    /// baked around it.
    Placed { uid: String },
    Refused(String),
}

/// Where and how a field build puts its fixture.
#[derive(Clone, Copy, Debug)]
pub struct FieldPut {
    /// A world point: the put search starts at the tile nearest it, or, when
    /// `exact`, the fixture goes on the cell it falls in or the put is
    /// refused.
    pub at: Vec3,
    pub exact: bool,
    /// The direction number the put gets (0 front, 1 left, 2 back, 3 right,
    /// the number a camera yaw of 90 × n degrees gives); the camera's yaw
    /// decides it when absent.
    pub direction: Option<Direction>,
}

/// The editor's field build calls, installed by the editor's crate.
#[derive(Resource, Clone, Copy)]
pub struct FieldBuilder {
    /// Whether a field build's layout save is in flight; one is out at a
    /// time.
    pub saving: fn(&World) -> bool,
    /// Puts an owned copy of a fixture (of a texture, or any) where the put
    /// says, and sends the layout with it: the build's ticket and the new
    /// row's uid, answered before the save so the caller can prepare how the
    /// instance is drawn, or why it was not put.
    pub start: fn(&mut World, i32, Option<u32>, FieldPut) -> Result<(u64, String), String>,
    /// How the build of a ticket stands.
    pub state: fn(&World, u64) -> Option<FieldBuildState>,
    /// The cell and direction a placed fixture was saved with, by its uid.
    pub placed_cell: fn(&World, &str) -> Option<(GridPosition, Direction)>,
}
