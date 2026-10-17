//! Requests and their replies.

use serde::{Deserialize, Serialize};

use crate::rows::{SuiteUserSections, UserResource};

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
