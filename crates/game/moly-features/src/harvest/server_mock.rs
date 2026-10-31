//! Compatibility path for harvest row adapters. The server document owns
//! every map, tool, stamina pool, and gathered resource.

pub use moly_protocol::harvest::{
    UserDrop, UserFixture, UserHarvestMap, UserTreasureBox, BOX_BEFORE_SPAWNED, BOX_SPAWNED,
    DROP_BEFORE, DROP_DROPPED, FIXTURE_HARVESTED, FIXTURE_SPAWNED,
};
pub const TRANSPORT_SECONDS: i32 = 10;
pub use crate::userdata::harvest::{user_tools, UserTool};

pub use moly_masters::offline_harvest::{
    generate_map, MapRules, MockFixtureRow, MockInputs, MockMaterialRow, MOCK_EXCLUDED,
};
/// A request of `PostUserMysekaiHarvestApi` (one merged run of stacks).
#[derive(Clone, Debug)]
pub struct HarvestRequest {
    pub site_id: u32,
    pub position_x: i32,
    pub position_z: i32,
    pub fixture_id: i32,
    pub hp: i32,
    pub is_last_attack: bool,
    /// (tool id, use count, the durability the client holds after them).
    pub tool: Option<(i64, i32, i32)>,
    /// `[{type, rest}]` in the order normal, enhance, boost.
    pub stamina_rests: [i32; 3],
}

/// A request of `PostUserMysekaiGatherApi`.
#[derive(Clone, Debug)]
pub struct GatherRequest {
    pub site_id: u32,
    pub drops: Vec<UserDrop>,
}
