//! The system fixture action as the client holds it, and the starter house
//! the layout owner places.
//!
//! - `UserDataManager.UserMysekaiSystemFixtureActions`
//!   (`UserMysekaiSystemFixtureAction` {userId, mysekaiSystemFixtureId}):
//!   [`ClientSystemFixtureActions`], set by responses only.
//!   `UpdateUserMysekaiSystemFixtureActions(data)` replaces the whole array
//!   when a response carries one (a null keeps the copy), and
//!   `IsActionedSystemFixture(id)` is false for a null or empty array, else
//!   whether any row names the id. The user id is the session's and is not
//!   kept.
//! - `MysekaiSystemFixtureActionService.ExecuteAPI(systemFixtureId)` is one
//!   `Call::SystemFixtureAction` on the server link (see
//!   [`crate::cutscene::birthday`]); the reply's `updatedResources` reach
//!   this copy with the arrived sections.
//! - [`STARTER_HOUSE`]: the house the layout owner appends to the home site,
//!   with its unrotated footprint corners.

use bevy::prelude::*;
use moly_law::fixture::{Direction, GridPosition};

pub(crate) use moly_protocol::rows::UserMysekaiSystemFixtureAction;

/// The client's copy of `userMysekaiSystemFixtureActions`, set by responses
/// only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientSystemFixtureActions {
    rows: Vec<UserMysekaiSystemFixtureAction>,
    /// Responses that carried the section.
    pub(crate) revision: u64,
}

impl ClientSystemFixtureActions {
    /// `UserDataManager.IsActionedSystemFixture(systemFixtureId)`.
    pub(crate) fn is_actioned(&self, mysekai_system_fixture_id: i32) -> bool {
        self.rows
            .iter()
            .any(|row| row.mysekai_system_fixture_id == mysekai_system_fixture_id)
    }

    /// `UpdateUserMysekaiSystemFixtureActions`: a response's rows replace
    /// the copy.
    pub(crate) fn apply(&mut self, rows: Vec<UserMysekaiSystemFixtureAction>) {
        self.rows = rows;
        self.revision += 1;
    }
}

/// The house `MysekaiSystemFixtureAction` puts on the home site: the package,
/// its fixture and texture ids, and the unrotated (Front) footprint corners as
/// a saved layout row stores them.
pub(crate) struct StarterHouse {
    pub(crate) package: &'static str,
    pub(crate) fixture_id: i32,
    pub(crate) texture_id: u32,
    pub(crate) min: GridPosition,
    pub(crate) max: GridPosition,
    pub(crate) direction: Direction,
}

pub(crate) const STARTER_HOUSE: StarterHouse = StarterHouse {
    package: "mysekai__fixture__mdl_mis0001_house_house1",
    fixture_id: 1,
    texture_id: 1,
    min: GridPosition { x: 4, y: 0, z: -17 },
    max: GridPosition {
        x: 15,
        y: 12,
        z: -6,
    },
    direction: Direction::Back,
};
