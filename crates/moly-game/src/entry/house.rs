//! The player's house on the offline HOME layout: a named server-decided mock.
//!
//! In the source the house is a placed system fixture in the user's saved
//! housing layout, which the server supplies. A new user's house choice is
//! server data too, so this mock takes the lowest `home` system fixture
//! (`mysekaiSystemFixtures` row 1 names fixture 1, `mdl_mis0001_house_house1`,
//! 12 x 12 x 13 cells, one colour). Its footprint is chosen free of every
//! starter row in both regions; the fixture layout owner appends it to the
//! HOME starter at restore time and checks that choice against the rows it
//! actually loaded. Facing Back puts the door toward +Z, so the camera yaw
//! the entry leaves behind equals the Normal camera's initial yaw.

use moly_law::fixture::{Direction, GridPosition};

pub(crate) struct StarterHouse {
    pub(crate) package: &'static str,
    pub(crate) fixture_id: i32,
    pub(crate) texture_id: u32,
    /// Unrotated (Front) footprint corners, as a saved layout row stores them.
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
