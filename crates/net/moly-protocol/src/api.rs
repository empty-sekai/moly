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

/// `PostUserMysekaiHarvestApi`: the rests the client sends. The reply's
/// stamina is these, unless a server-side stamina edit is pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestRequest {
    pub normal: i32,
    pub enhance: i32,
    pub boost: i32,
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
