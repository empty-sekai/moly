//! User data rows the server sends and the game keeps copies of.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// `UserMysekaiStamina`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamina {
    pub normal: i32,
    pub enhance: i32,
    pub boost: i32,
}

/// `UserMysekaiGamedata`, the fields the client reads here.
/// `mysekai_rank` is `None` until the server derives it from `totalExp`
/// through the master rank table (a default or migrated document).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gamedata {
    pub mysekai_rank: Option<i32>,
    pub total_exp: i32,
    pub refreshed_at: i64,
    pub is_mysekai_tutorial_end: bool,
}

/// `UserMysekaiColorfulPass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorfulPass {
    pub id: i32,
    pub expired_at: i64,
}

/// `MysekaiPhenomenaSchedule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRow {
    pub refresh_time_period_id: i32,
    pub schedule_date: i64,
    pub phenomena_id: i32,
}

/// `UserMysekaiGate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub gate_id: i32,
    pub skin_id: i32,
    pub level: i32,
    pub visit_count: i32,
    pub last_refreshed_at: i64,
    pub is_setting_at_home_site: bool,
}

/// `UserMysekaiGateCharacter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCharacter {
    pub gate_id: i32,
    pub unit_group_id: i32,
    pub is_reservation: bool,
    pub visit_count: i32,
}

/// `UserMysekaiFixture`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiFixture {
    pub mysekai_fixture_id: i32,
    pub texture_id: i32,
    pub quantity: i32,
    pub last_obtained_at: i64,
}

/// `UserMysekaiCanvas`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiCanvas {
    pub mysekai_fixture_id: i32,
    pub card_id: i32,
    pub is_special_training: bool,
    pub quantity: i32,
    pub last_obtained_at: i64,
}

/// `UserMysekaiBlueprint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiBlueprint {
    pub mysekai_blueprint_id: i32,
    pub obtained_at: i64,
}

/// `UserMysekaiItem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiItem {
    pub mysekai_item_id: i32,
    pub quantity: i32,
    pub last_obtained_at: i64,
}

/// `UserMysekaiMaterial` (id and quantity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiMaterial {
    pub mysekai_material_id: i32,
    pub quantity: i32,
}

/// `UserMysekaiMaterialPossession`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiMaterialPossession {
    pub quantity: i32,
}

/// The `UserMysekaiGamedata` fields the inventory and craft screens read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiGamedata {
    /// `None` until the server has seated the rank from `totalExp`.
    pub mysekai_rank: Option<i32>,
    pub total_exp: i32,
    pub mysekai_material_possession_level: i32,
    pub mysekai_fixture_possession_level: i32,
}

/// The `SuiteUser` tables a response or reply carries; `None` for a table it
/// does not carry. Applying it replaces each carried table whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SuiteUserSections {
    pub user_mysekai_character_talks: Option<Vec<UserMysekaiCharacterTalk>>,
    pub user_mysekai_materials: Option<Vec<UserMysekaiMaterial>>,
    pub user_mysekai_material_possession: Option<UserMysekaiMaterialPossession>,
    pub user_mysekai_fixtures: Option<Vec<UserMysekaiFixture>>,
    pub user_mysekai_canvases: Option<Vec<UserMysekaiCanvas>>,
    pub user_mysekai_blueprints: Option<Vec<UserMysekaiBlueprint>>,
    pub user_mysekai_items: Option<Vec<UserMysekaiItem>>,
    pub user_mysekai_gamedata: Option<UserMysekaiGamedata>,
}

impl SuiteUserSections {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `UserMysekaiCharacterTalk`: whether a talk has been read (the character
/// archive reads these rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiCharacterTalk {
    pub mysekai_character_talk_id: i32,
    pub is_read: bool,
}

/// `UserResource`, the fields a reply's `obtainedResources` row carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserResource {
    pub resource_type: String,
    pub resource_id: i32,
    pub resource_level: i32,
    pub quantity: i32,
}

/// `UserMysekaiMusicPlayFixtureSetting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicPlaySetting {
    pub mysekai_site_id: i32,
    pub mysekai_music_record_id: i32,
    pub music_vocal_id: i32,
    pub is_instrumental: bool,
}

/// `UserMysekaiMusicRecord`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedMusicRecord {
    pub mysekai_music_record_id: i32,
    pub obtained_at: i64,
}

/// `UserAvatar`. All null is the source's unset wear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserAvatar {
    pub costume: Option<i32>,
    pub accessory: Option<i32>,
    pub skin_color: Option<i32>,
    pub coordinate: Option<i32>,
}

/// `UserMysekaiSystemFixtureAction` (the user id is the session's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiSystemFixtureAction {
    pub mysekai_system_fixture_id: i32,
}

/// `UserMysekaiTool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiTool {
    pub mysekai_tool_id: i32,
    pub quantity: i32,
    pub durability: i32,
    pub last_obtained_at: i64,
}

/// `MasterAvatarCoordinate`: the bundle and colour columns an avatar
/// coordinate row carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateRow {
    pub costume_assetbundle_name: Option<String>,
    pub accessory_assetbundle_name: Option<String>,
    pub skin_color_code: Option<String>,
}

/// `UserBirthdayParty` (the one user's `userId` is left out).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BirthdayPartyRow {
    pub birthday_party_id: i32,
    pub delivery_total_point: i32,
    pub dropped_mysekai_material_count: i32,
    pub obtained_mysekai_material_count: i32,
}

/// `UserHonor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonorRow {
    pub honor_id: i32,
    pub level: i32,
    pub obtained_at: i64,
}

/// One resource of a reply or a granted reward box.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reward {
    pub resource_type: String,
    pub resource_id: i64,
    pub quantity: i32,
    pub level: Option<i64>,
}

/// The delivery sections a response carries: each carried table replaces the
/// game's copy whole, a `None` leaves it as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryUpdate {
    /// `userBirthdayParties`.
    pub parties: Option<Vec<BirthdayPartyRow>>,
    /// `userMaterials`: material id -> quantity.
    pub materials: Option<BTreeMap<i32, i32>>,
    /// `userMysekaiMaterials`: mysekai material id -> quantity.
    pub mysekai_materials: Option<BTreeMap<i32, i32>>,
    /// `userCards`: the owned card ids.
    pub cards: Option<BTreeSet<i64>>,
    /// `userHonors`.
    pub honors: Option<Vec<HonorRow>>,
    /// `masterConfigs`: the master configs the delivery reads.
    pub configs: Option<BTreeMap<String, i32>>,
}

impl DeliveryUpdate {
    /// Whether the update carries no table.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}
