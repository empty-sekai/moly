//! Requests and their replies.

use serde::{Deserialize, Serialize};

use crate::rows::{Reward, SuiteUserSections, UserResource};

/// Which home-action API a request goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Constructed by the home actions' seam.
pub enum HomeActionApi {
    /// `PostUserMysekaiCraftApi` from the craft screen.
    Craft,
    /// `PostUserMysekaiCraftApi` from the canvas screen (a canvas blueprint).
    Canvas,
    /// `PostUserMysekaiHousingSketchApi` from the sketch screen.
    Sketch,
}

impl HomeActionApi {
    /// The API's name in the source.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Craft => "PostUserMysekaiCraftApi (craft)",
            Self::Canvas => "PostUserMysekaiCraftApi (canvas)",
            Self::Sketch => "PostUserMysekaiHousingSketchApi",
        }
    }
}

/// A request: the API and the fixture UID it names (craft and canvas name
/// the tool; the sketch names none).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeActionPost {
    pub api: HomeActionApi,
    pub target: Option<String>,
}

/// The reply the client reads before it starts the world sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeActionReply {
    pub success: bool,
}

/// `MysekaiCraftType`, in the source's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MysekaiCraftType {
    MysekaiFixture,
    MysekaiTool,
    MysekaiCanvas,
    Material,
}

impl MysekaiCraftType {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "mysekai_fixture" => Some(Self::MysekaiFixture),
            "mysekai_tool" => Some(Self::MysekaiTool),
            "mysekai_canvas" => Some(Self::MysekaiCanvas),
            "material" => Some(Self::Material),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::MysekaiFixture => "mysekai_fixture",
            Self::MysekaiTool => "mysekai_tool",
            Self::MysekaiCanvas => "mysekai_canvas",
            Self::Material => "material",
        }
    }
}

/// `UserMysekaiCraftRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiCraftRequest {
    pub blueprint_id: i32,
    pub texture_id: Option<i32>,
    pub card_id: Option<i32>,
    pub is_special_training: Option<bool>,
    pub quantity: i32,
    /// The blueprint term the craft consumes; `None` when the cell carries no
    /// term (the blueprint's own costs, no term cap).
    pub mysekai_blueprint_term_id: Option<i32>,
}

/// `UserMysekaiHousingSketchRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiHousingSketchRequest {
    pub mysekai_owner_user_id: i64,
    pub mysekai_site_id: i32,
    pub mysekai_blueprint_id: i32,
}

/// A `SuiteUser` reply: success with the tables the request changed, or a
/// refusal the client shows as the API error (it starts nothing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuiteUserReply {
    pub success: bool,
    /// Why the server refused (`None` on success).
    pub refusal: Option<String>,
    /// The tables the reply carries (already merged into the client copy).
    pub updated: SuiteUserSections,
}

impl SuiteUserReply {
    pub fn refused(reason: String) -> Self {
        Self {
            success: false,
            refusal: Some(reason),
            updated: SuiteUserSections::default(),
        }
    }
}

/// `UserMysekaiMusicPlaySetRequest`, as `MysekaiMusicPlaySetService.Execute`
/// fills it: the record's `SelectedId`, `isInstrumental` false, the model's
/// `MusicVocalId` (null when the model has none), the fixture id and the
/// fixture's colour id as `textureId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Constructed by the record list's seam.
pub struct MusicPlaySetRequest {
    pub mysekai_site_id: i32,
    pub mysekai_music_record_id: i32,
    pub music_vocal_id: Option<i32>,
    pub is_instrumental: bool,
    pub mysekai_fixture_id: i32,
    pub texture_id: i32,
}

/// `UserMysekaiMusicPlayEjectRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Constructed by the record list's seam.
pub struct MusicPlayEjectRequest {
    pub mysekai_site_id: i32,
    pub mysekai_fixture_id: i32,
    pub texture_id: i32,
}

/// A request to one of the two APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Constructed by the record list's seam.
pub enum MusicPlayPost {
    Set(MusicPlaySetRequest),
    Eject(MusicPlayEjectRequest),
}

impl MusicPlayPost {
    /// The API's name in the source.
    pub fn api_name(&self) -> &'static str {
        match self {
            Self::Set(_) => "PutUserMysekaiMusicPlaySetApi",
            Self::Eject(_) => "PutUserMysekaiMusicPlayEjectApi",
        }
    }

    pub fn site_id(&self) -> i32 {
        match self {
            Self::Set(request) => request.mysekai_site_id,
            Self::Eject(request) => request.mysekai_site_id,
        }
    }
}

/// The reply the caller reads (`MysekaiMusicPlaySetService` /
/// `MysekaiMusicPlayEjectService` status Success or Error).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicPlayReply {
    pub success: bool,
}

/// `UserMysekaiSystemFixtureActionResponse`, or a refusal (the source's
/// null reply: the API error, and the caller stops).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemFixtureActionReply {
    pub success: bool,
    /// Why the server refused (`None` on success).
    pub refusal: Option<String>,
    /// `obtainedResources`.
    pub obtained_resources: Vec<UserResource>,
}

/// `UserMysekaiFixtureDisassembleRequest`: the fixture and texture the
/// player disassembles, and the quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureDisassembleRequest {
    pub mysekai_fixture_id: i32,
    pub texture_id: i32,
    pub quantity: i32,
}

/// `UserMysekaiRecycleMaterialRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecycleMaterialRequest {
    pub mysekai_material_id: i32,
    pub quantity: i32,
}

/// `UserMysekaiRecycleToolRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecycleToolRequest {
    pub mysekai_tool_id: i32,
    pub quantity: i32,
}

/// A request to one of the three recycle and disassemble APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MysekaiInventoryDisassemble {
    /// `PostUserMysekaiDisassembleFixtureApi`.
    Fixture(FixtureDisassembleRequest),
    /// `PostUserMysekaiRecycleMaterialApi`.
    Material(RecycleMaterialRequest),
    /// `PostUserMysekaiRecycleToolApi`.
    Tool(RecycleToolRequest),
}

impl MysekaiInventoryDisassemble {
    /// The API's name in the source.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Fixture(_) => "PostUserMysekaiDisassembleFixtureApi",
            Self::Material(_) => "PostUserMysekaiRecycleMaterialApi",
            Self::Tool(_) => "PostUserMysekaiRecycleToolApi",
        }
    }
}

/// `UserMysekaiFixtureDisassembleResponse` and `UserMysekaiRecycleResponse`:
/// the resources the rows gave back and the tables the request changed
/// (already in the game's copies).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MysekaiInventoryRecycleResponse {
    pub obtained_resources: Vec<UserResource>,
    pub updated: SuiteUserSections,
}

/// `UserMysekaiCharacterTalkReadResponse`, or a refusal the client shows as
/// the API error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TalkReadReply {
    pub success: bool,
    /// Why the server refused (`None` on success).
    pub refusal: Option<String>,
    /// `obtainedResources`.
    pub obtained_resources: Vec<UserResource>,
    /// `updatedResources` (already merged into the client copy).
    pub updated: SuiteUserSections,
}

/// One row of the gather request (`UserBirthdayPartyGatherRequestContent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatherContent {
    pub birthday_party_id: i32,
    pub gathered_count: i32,
}

/// `UserBirthdayPartyDeliveryResponse`; its `updatedResources` are already in
/// the game's copies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Read by the delivery owner's seam.
pub struct DeliveryResponse {
    pub dropped_reward_count: i32,
    pub obtained_delivery_total_rewards: Vec<Reward>,
    pub is_refreshed: bool,
}

/// `UserBirthdayPartyGatherResponse`; its `updatedResources` are already in
/// the game's copies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Read by the delivery owner's seam.
pub struct GatherResponse {
    pub obtained_delivery_total_rewards: Vec<Reward>,
}

/// One corner of a housing layout fixture's grid footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HousingLayoutCorner {
    pub x: i8,
    pub y: i8,
    pub z: i8,
}

/// One placed fixture of a housing layout in the served record form: the
/// package, the master fixture id, the footprint corners, the centre height,
/// the layout type, the rotation and the texture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HousingLayoutFixture {
    pub package: String,
    pub mysekai_fixture_id: i32,
    pub minimum: HousingLayoutCorner,
    pub maximum: HousingLayoutCorner,
    pub center_y: i8,
    pub layout_type: u8,
    pub rotation: u8,
    pub texture_id: u32,
}

/// `UserMysekaiHousingEditLayoutRequest`: the layout editor's save, the site
/// and its whole layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiHousingEditLayoutRequest {
    pub mysekai_site_id: u32,
    /// The site's type (`home_site`, ...), which names its served layout row.
    pub site_type: String,
    /// `userMysekaiSiteHousingLayout.mysekaiSiteHousingLayouts`: every placed
    /// fixture of the site.
    pub fixtures: Vec<HousingLayoutFixture>,
}

/// `UserMysekaiHarvestRequest` (`PostUserMysekaiHarvestApi`): one merged run
/// of hits, the tool the run used (absent when it used none) and the stamina
/// that is left.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestRequest {
    pub mysekai_site_id: i32,
    pub mysekai_site_harvest_fixture_target: MysekaiSiteHarvestFixtureTarget,
    pub mysekai_tool_used: Option<MysekaiToolUsed>,
    pub mysekai_stamina_useds: Vec<MysekaiStaminaUsed>,
}

/// `MysekaiSiteHarvestFixtureTarget`: where the run hit, the target's hp
/// after the hits and whether the last of them destroyed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MysekaiSiteHarvestFixtureTarget {
    pub position_x: i32,
    pub position_z: i32,
    pub mysekai_site_harvest_fixture_id: i32,
    pub hp: i32,
    pub is_last_attack: bool,
}

/// `MysekaiToolUsed`: the tool a run used, how many times, and the durability
/// the client holds after them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MysekaiToolUsed {
    pub mysekai_tool_id: i32,
    pub use_count: i32,
    pub durability: i32,
}

/// `MysekaiStaminaUsed`: one stamina pool and what is left of it after the
/// run (`mysekaiStaminaType`: `normal`, `enhance` or `boost`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MysekaiStaminaUsed {
    pub mysekai_stamina_type: String,
    pub rest: i32,
}

/// `PostUserMysekaiGateReserveApi`: the selected character unit group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReserveRequest {
    pub mysekai_game_character_unit_group_id: i32,
}

/// `PostUserMysekaiGateChangeApi`: the home gate's new gate, skin and the
/// unit groups that then visit it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateChangeRequest {
    pub mysekai_gate_id: i32,
    pub mysekai_gate_skin_id: i32,
    pub mysekai_game_character_unit_group_ids: Vec<i32>,
}

/// `PutUserMysekaiBirthdayPartyDeliveryApi`: the party and the delivery
/// material the client spends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BirthdayPartyDeliveryRequest {
    pub birthday_party_id: i32,
    pub consumed_delivery_material_count: i32,
}

/// `PutUserMysekaiBirthdayPartyGatherApi`: the count gathered from each
/// party.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BirthdayPartyGatherRequest {
    pub contents: Vec<GatherContent>,
}

/// `PutUserMysekaiCharacterTalkReadApi`: the talk that was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterTalkReadRequest {
    pub mysekai_character_talk_id: i32,
}

/// `PostUserMysekaiSystemFixtureActionApi`: the system fixture that was
/// actioned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemFixtureActionRequest {
    pub mysekai_system_fixture_id: i32,
}

/// `UserMysekaiConvertStartRequest` (`PostUserMysekaiConvertStartApi`): the
/// convert item a slot starts converting, how many, and its draw variant.
/// The slot is the call's path parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiConvertStartRequest {
    pub mysekai_convert_item_id: i32,
    pub convert_count: i32,
    pub choice_id: i32,
}

/// `UserMysekaiConvertReceiveRequest` (`PostUserMysekaiConvertReceiveApi`):
/// how many of the slot's completed converts to receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiConvertReceiveRequest {
    pub receive_convert_count: i32,
}

/// `UserMysekaiConvertReceiveResponse`: the resources the received converts
/// produced and the tables the request changed (already in the game's
/// copies).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserMysekaiConvertReceiveResponse {
    pub obtained_resources: Vec<UserResource>,
    pub updated: SuiteUserSections,
}

/// `UserMysekaiConvertShortenRequest` (`PostUserMysekaiConvertShortenApi`):
/// the crystals the skip spends, as the slot priced them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiConvertShortenRequest {
    pub consume_jewel_quantity: i32,
}

/// A request to one of the three convert APIs; the slot is the call's path
/// parameter in each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConvertPost {
    /// `PostUserMysekaiConvertStartApi`.
    Start(UserMysekaiConvertStartRequest),
    /// `PostUserMysekaiConvertReceiveApi`.
    Receive(UserMysekaiConvertReceiveRequest),
    /// `PostUserMysekaiConvertShortenApi`.
    Shorten(UserMysekaiConvertShortenRequest),
}

impl ConvertPost {
    /// The API's name in the source.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Start(_) => "PostUserMysekaiConvertStartApi",
            Self::Receive(_) => "PostUserMysekaiConvertReceiveApi",
            Self::Shorten(_) => "PostUserMysekaiConvertShortenApi",
        }
    }
}

/// `MysekaiBoostStaminaRecoveryType`, spelled the way the client's
/// `ToString()` sends the enum value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoostStaminaRecoveryType {
    /// Live bonus spent on boost stamina (`boost`).
    Boost,
    /// Boost items spent (`boost_item`).
    BoostItem,
    /// Paid crystals spent (`jewel`).
    Jewel,
}

impl BoostStaminaRecoveryType {
    pub fn word(self) -> &'static str {
        match self {
            Self::Boost => "boost",
            Self::BoostItem => "boost_item",
            Self::Jewel => "jewel",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "boost" => Some(Self::Boost),
            "boost_item" => Some(Self::BoostItem),
            "jewel" => Some(Self::Jewel),
            _ => None,
        }
    }
}

/// `UserMysekaiStaminaRecoveryConsumeResource`: one resource a recovery
/// spends (the live bonus, the paid crystals, or a boost item) and how much.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaminaRecoveryConsume {
    pub resource_id: i32,
    pub quantity: i32,
}

/// `UserMysekaiStaminaRequest` (`PostUserMysekaiBoostStaminaRecoveryApi`):
/// the recovery type and the resources it spends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMysekaiStaminaRequest {
    pub mysekai_boost_stamina_recovery_type: BoostStaminaRecoveryType,
    pub user_mysekai_stamina_recovery_consume_resources: Vec<StaminaRecoveryConsume>,
}
