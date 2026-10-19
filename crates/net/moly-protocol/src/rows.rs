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
    /// `mysekaiConvertFixtureLevel`: the convert machine's level, which the
    /// convert screen reads to unlock its items and the shorten button.
    /// `None` until the server seats it.
    pub mysekai_convert_fixture_level: Option<i32>,
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

/// One `mysekaiRankReleases` row: a rank, the release type and the id of the
/// released row (a site level for `mysekai_site_level`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankReleaseRow {
    pub mysekai_rank: i32,
    pub mysekai_rank_release_type: String,
    pub external_id: i32,
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

/// `UserMysekaiBlueprintLendCraftCount`: how many crafts a blueprint term has
/// used, the client's only source for a term's remaining count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiBlueprintLendCraftCount {
    pub mysekai_blueprint_term_id: i32,
    pub craft_count: i32,
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
    pub user_mysekai_tools: Option<Vec<UserMysekaiTool>>,
    pub user_mysekai_music_records: Option<Vec<OwnedMusicRecord>>,
    pub user_mysekai_gamedata: Option<UserMysekaiGamedata>,
    /// `userMysekaiConvertSlots`.
    pub user_mysekai_convert_slots: Option<Vec<ConvertSlot>>,
    /// `userMysekaiConvertItemHistories`.
    pub user_mysekai_convert_item_histories: Option<Vec<ConvertItemHistory>>,
}

impl SuiteUserSections {
    /// The convert slots this section set carries.
    #[allow(dead_code)] // Read by the convert screen's seam.
    pub fn convert_slots(&self) -> Option<&Vec<ConvertSlot>> {
        self.user_mysekai_convert_slots.as_ref()
    }
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

/// `UserMysekaiConvertObtainResource`: one resource a convert slot's draw
/// produced, at the completion step `progress`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertObtainResource {
    pub slot_id: i32,
    pub progress: i32,
    pub seq: i32,
    pub resource_type: String,
    pub resource_id: i32,
    pub quantity: i32,
}

impl ConvertObtainResource {
    /// The row as the `UserResource` a receiver receives.
    pub fn to_resource(&self) -> UserResource {
        UserResource {
            resource_type: self.resource_type.clone(),
            resource_id: self.resource_id,
            resource_level: 1,
            quantity: self.quantity,
        }
    }
}

/// One convert item a response carries: the `mysekaiConvertItems` row's
/// fields the convert screen reads, as a section of its own because the
/// client builds its item list from them and its document keeps none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiConvertItem {
    pub mysekai_convert_item_id: i32,
    /// `mysekaiConvertItemObtainType`.
    pub obtain_type: String,
    pub external_id: i32,
    pub required_convert_fixture_level: i32,
    pub convert_minutes: i32,
    /// 0 when the item has no convert limit.
    pub convert_limit: i32,
    pub once_convert_limit: i32,
    /// `refreshCycle`: `none`, `weekly` or `monthly`.
    pub refresh_cycle: String,
    pub start_at: i64,
    pub end_at: i64,
}

/// `UserMysekaiConvertSlot`: one convert slot's state. `convert_start_at` and
/// `once_convert_minutes` place the slot's progress on the server clock; the
/// obtain rows are the slot's draws.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertSlot {
    pub slot_id: i32,
    /// The `mysekaiConvertItems` row the slot converts.
    pub mysekai_convert_item_id: i32,
    /// The draw variant of that row (0 when it has none).
    pub choice_id: i32,
    /// Epoch milliseconds the slot started converting.
    pub convert_start_at: i64,
    /// The minutes one convert of the item takes, as the server decided them.
    pub once_convert_minutes: i32,
    /// How many converts the slot holds.
    pub convert_count: i32,
    /// How many completed converts the user has received.
    pub received_progress: i32,
    /// `mysekaiConvertSlotStatus`: `processing` or `complete`.
    pub mysekai_convert_slot_status: String,
    pub obtain_resources: Vec<ConvertObtainResource>,
}

/// `MysekaiConvertSlotStatus` words.
pub const CONVERT_SLOT_PROCESSING: &str = "processing";
pub const CONVERT_SLOT_COMPLETE: &str = "complete";

impl ConvertSlot {
    /// Whether the slot is still converting.
    pub fn is_processing(&self) -> bool {
        self.mysekai_convert_slot_status == CONVERT_SLOT_PROCESSING
    }

    /// The epoch millisecond at which the slot's last convert completes
    /// (`convertStartAt + onceConvertMinutes * convertCount`).
    pub fn end_at(&self) -> i64 {
        self.convert_start_at
            .saturating_add(saturating_minutes_ms(self.once_convert_minutes, self.convert_count))
    }

    /// How many converts have completed at `now`, at least 1 and at most
    /// `convert_count`; a slot with nothing started has completed none, and a
    /// slot the server marked `complete` has completed them all
    /// (`MysekaiConvertFurnitureSlotModel.SetupInComplete` sets every convert
    /// complete, a skip included).
    pub fn completed_count(&self, now: i64) -> i32 {
        if self.mysekai_convert_slot_status == CONVERT_SLOT_COMPLETE {
            return self.convert_count.max(0);
        }
        if self.convert_start_at <= 0 || self.once_convert_minutes < 1 {
            return 0;
        }
        let each = minutes_ms(self.once_convert_minutes);
        let elapsed = now.saturating_sub(self.convert_start_at);
        let completed = (elapsed / each.max(1)) as i64;
        completed.clamp(0, self.convert_count.max(0) as i64) as i32
    }

    /// The slot whose converts have all completed, at `end_at`.
    pub fn completed(&self) -> Self {
        Self {
            mysekai_convert_slot_status: CONVERT_SLOT_COMPLETE.to_owned(),
            convert_start_at: self.end_at(),
            ..self.clone()
        }
    }

    /// The rows drawn at the completion step `progress`, as the resources a
    /// receiver receives (the source's `GetObtainResources`).
    pub fn obtain_resources_at(&self, progress: i32) -> Vec<UserResource> {
        self.obtain_resources
            .iter()
            .filter(|row| row.progress == progress)
            .map(|row| ConvertObtainResource::to_resource(row))
            .collect()
    }
}

/// `UserMysekaiConvertItemHistory`: how often a convert item has been
/// converted, for the items whose master row carries a convert limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertItemHistory {
    pub mysekai_convert_item_id: i32,
    /// The converts since the row's last refresh.
    pub convert_count: i32,
    pub total_convert_count: i32,
    /// Epoch milliseconds of the last convert start.
    pub last_convert_start_at: i64,
}

fn minutes_ms(minutes: i32) -> i64 {
    i64::from(minutes).saturating_mul(60_000)
}

fn saturating_minutes_ms(minutes: i32, count: i32) -> i64 {
    i64::from(minutes).saturating_mul(i64::from(count)).saturating_mul(60_000)
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
