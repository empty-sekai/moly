//! User harvest map rows shared by the server and its client mirror.
use serde::{Deserialize, Serialize};

pub const FIXTURE_SPAWNED: i32 = 1;
pub const FIXTURE_HARVESTED: i32 = 2;
pub const DROP_BEFORE: i32 = 0;
pub const DROP_DROPPED: i32 = 1;
pub const BOX_BEFORE_SPAWNED: i32 = 0;
pub const BOX_SPAWNED: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserFixture {
    #[serde(rename = "mysekaiSiteHarvestFixtureId")]
    pub fixture_id: i32,
    pub position_x: i32,
    pub position_z: i32,
    pub hp: i32,
    pub status: i32,
    pub group_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserDrop {
    pub resource_type: i32,
    pub resource_id: i64,
    pub position_x: i32,
    pub position_z: i32,
    pub hp: i32,
    pub seq: i32,
    pub status: i32,
    pub quantity: i32,
    pub group_id: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserHarvestMap {
    #[serde(rename = "mysekaiSiteId")]
    pub site_id: u32,
    #[serde(rename = "userMysekaiSiteHarvestFixtures")]
    pub fixtures: Vec<UserFixture>,
    #[serde(rename = "userMysekaiSiteHarvestResourceDrops")]
    pub drops: Vec<UserDrop>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserTreasureBox {
    pub refresh_type: String,
    pub seq: i32,
    pub transport_seconds: i32,
    pub status: i32,
    #[serde(rename = "mysekaiSiteId")]
    pub site_id: u32,
    pub position_x: i32,
    pub position_z: i32,
}

/// The original request carries strings, while its map decoder uses the
/// source ResourceType integer. Keep the conversion at this one boundary.
pub fn resource_word(value: i32) -> Option<&'static str> {
    match value {
        2 => Some("material"),
        39 => Some("mysekai_fixture"),
        40 => Some("mysekai_blueprint"),
        41 => Some("mysekai_material"),
        42 => Some("mysekai_item"),
        43 => Some("mysekai_tool"),
        44 => Some("mysekai_music_record"),
        _ => None,
    }
}
