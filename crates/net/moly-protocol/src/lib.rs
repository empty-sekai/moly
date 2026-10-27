//! The messages the game and its server exchange.
//!
//! The game sends one HTTP `POST` to [`RPC_PATH`] per request. The request
//! body is the encoded list of [`ClientMessage`] it carries; every request
//! opens with a [`ClientMessage::Hello`], naming the protocol version and
//! the user:
//!
//! 1. A request that carries a call is `[Hello, Call]`; a request with no
//!    call is `[Hello]` alone.
//! 2. The response body is the encoded list of [`ServerMessage`]: the
//!    [`ServerMessage::Welcome`], or a [`ServerMessage::Refused`], first;
//!    then the [`ServerMessage::Reply`] to the call, when there was one;
//!    then every [`ServerMessage::Push`] the server queued for the user.
//! 3. Each [`ClientMessage::Call`] carries a sequence number that its
//!    [`ServerMessage::Reply`] repeats. Replies arrive in call order.
//! 4. [`ServerMessage::Push`] carries user data the server sends on its own,
//!    such as the sections of a new refresh window.
//!
//! Every reply and push may carry [`Sections`]: the user data tables the
//! game keeps copies of. The game writes its copies from these and from
//! nothing else.
//!
//! Messages are binary ([`encode`] / [`decode`]); [`to_json`] renders any
//! message as JSON for logs and tools.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::api::{
    BirthdayPartyDeliveryRequest, BirthdayPartyGatherRequest, CharacterTalkReadRequest,
    ConvertPost, GateChangeRequest, GateReserveRequest, HarvestRequest, HomeActionPost,
    MusicPlayEjectRequest, MusicPlaySetRequest, MysekaiInventoryDisassemble, ReleaseRequest,
    SystemFixtureActionRequest, UserMysekaiConvertReceiveResponse, UserMysekaiCraftRequest,
    UserMysekaiHousingEditLayoutRequest, UserMysekaiHousingSketchRequest,
    UserMysekaiNormalMissionRewardRequest, UserMysekaiNormalMissionRewardResponse,
    UserMysekaiStaminaRequest,
};
use crate::rows::{
    ColorfulPass, ConvertItemHistory, ConvertSlot, DeliveryUpdate, Gamedata, Gate,
    MusicPlaySetting, OwnedMusicRecord, RankReleaseRow, ScheduleRow, Stamina, SuiteUserSections,
    UserAvatar, UserMysekaiNormalMission, UserMysekaiNormalMissionSheet,
    UserMysekaiPhenomena, UserMysekaiSystemFixtureAction, UserMysekaiTool,
};

pub mod api;
pub mod gate;
pub mod json;
pub mod rows;

/// The path of the server's one endpoint: every request is a `POST` here.
pub const RPC_PATH: &str = "/rpc";

/// Raised whenever a message changes shape. A peer with another version is
/// refused at the handshake.
///
/// 2: the sections carry the home gate row and the appended rank releases,
/// and the reserve reply carries every gate character.
/// 3: the harvest request carries the hit, the tool the run used and the
/// stamina that is left.
/// 4: the sections carry the server panel's NPC slice.
/// 5: the inventory screen's recycle and disassemble call and its reply.
/// 6: the boost stamina recovery call and its reply.
/// 7: the craft request carries the blueprint term, and the sections carry
/// the blueprint terms' craft counters.
/// 8: the convert screen's three calls and their replies, and the sections
/// that carry the convert slots, the convert item histories and the convert
/// machine's level.
/// 9: the normal mission reward call and its reply, and the sections that
/// carry the user's mission rows and the sheet it is on.
/// 10: the release call, and the section that carries the phenomena the user
/// has learned.
/// 11: the housing save carries its fixed phenomenon.
pub const PROTOCOL_VERSION: u32 = 11;

/// The account a session belongs to. The server keeps one document per user.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UserId(pub String);

impl UserId {
    /// The user a single-player game plays as.
    pub fn local() -> Self {
        Self("local".to_owned())
    }

    /// The longest user id a server accepts.
    pub const MAX_LEN: usize = 64;

    /// Whether a server accepts this id: 1 to [`Self::MAX_LEN`] ASCII
    /// letters, digits, `_` or `-`. A server names its stored documents
    /// after the id, so no other character is admitted.
    pub fn is_valid(&self) -> bool {
        (1..=Self::MAX_LEN).contains(&self.0.len())
            && self
                .0
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A message from the game to the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// The first message of every request.
    Hello {
        protocol: u32,
        user: UserId,
        /// The game's name and version, for the server's log.
        client: String,
    },
    /// A request. `seq` is the game's own counter; the reply repeats it.
    Call { seq: u32, call: Call },
}

/// A message from the server to the game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// The session of this request is open.
    Welcome {
        protocol: u32,
        user: UserId,
        /// The server's name and version, for the game's log.
        server: String,
    },
    /// The request is refused; nothing follows it in the response.
    Refused { reason: String },
    /// The answer to the call numbered `seq`.
    Reply {
        seq: u32,
        outcome: Result<Reply, Refusal>,
        sections: Option<Box<Sections>>,
    },
    /// User data the server sends without a call.
    Push { kind: PushKind, sections: Box<Sections> },
}

/// Why the server did not carry out a call. The game shows or logs `reason`;
/// `code` is the server's own name for the case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: String,
    pub reason: String,
}

impl Refusal {
    pub fn new(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.reason)
    }
}

/// What a push answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushKind {
    /// The server clock entered a new refresh window.
    Refresh,
    /// The server's document was edited and the edit is delivered now.
    Sync,
    /// Rows of the birthday parties now in session.
    BirthdayPartySeat,
}

/// A request. One variant per endpoint the game calls, carrying the request
/// the endpoint takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Call {
    /// `PostUserMysekaiApi`: the server seats the user and answers with every
    /// section.
    Join,
    /// `PostUserMysekaiHarvestApi`.
    Harvest(HarvestRequest),
    /// `PostUserMysekaiGatherApi`.
    Gather,
    /// `PostUserMysekaiGateReserveApi`.
    GateReserve(GateReserveRequest),
    /// `PostUserMysekaiGateChangeApi`.
    GateChange(GateChangeRequest),
    /// `PutUserMysekaiBirthdayPartyDeliveryApi`.
    BirthdayPartyDelivery(BirthdayPartyDeliveryRequest),
    /// `PutUserMysekaiBirthdayPartyGatherApi`.
    BirthdayPartyGather(BirthdayPartyGatherRequest),
    /// The craft, canvas and sketch screens' requests.
    HomeAction(HomeActionPost),
    /// `PostUserMysekaiCraftApi`.
    Craft(UserMysekaiCraftRequest),
    /// `PostUserMysekaiHousingSketchApi`.
    HousingSketch(UserMysekaiHousingSketchRequest),
    /// `PostUserMysekaiHousingLayoutApi`.
    HousingLayout(UserMysekaiHousingEditLayoutRequest),
    /// `PutUserMysekaiCharacterTalkReadApi`.
    CharacterTalkRead(CharacterTalkReadRequest),
    /// `PutUserMysekaiMusicPlaySetApi`.
    MusicPlaySet(MusicPlaySetRequest),
    /// `PutUserMysekaiMusicPlayEjectApi`.
    MusicPlayEject(MusicPlayEjectRequest),
    /// `PostUserMysekaiSystemFixtureActionApi`.
    SystemFixtureAction(SystemFixtureActionRequest),
    /// `PostUserMysekaiReleaseApi`: a site's phenomenon is learned.
    Release(ReleaseRequest),
    /// `PostUserMysekaiDisassembleFixtureApi`, `PostUserMysekaiRecycleMaterialApi`
    /// and `PostUserMysekaiRecycleToolApi`.
    InventoryRecycle(MysekaiInventoryDisassemble),
    /// `PostUserMysekaiBoostStaminaRecoveryApi`: the recover dialog's
    /// confirmation.
    BoostStaminaRecovery(UserMysekaiStaminaRequest),
    /// The convert screen's requests, with the slot they name.
    Convert { slot_id: i32, post: ConvertPost },
    /// `PostUserMysekaiNormalMissionRewardApi`: the mission screen's receive,
    /// one mission's reward or a bulk receive in one call.
    NormalMissionReward(UserMysekaiNormalMissionRewardRequest),
    /// The control plane's calls: the document and the schema, an edit, and
    /// the browser game's seed install.
    Admin(AdminCall),
}

impl Call {
    /// The call's fieldless kind, for the game's ordered arrivals.
    pub fn kind(&self) -> CallKind {
        match self {
            Self::Join => CallKind::Join,
            Self::Harvest(_) => CallKind::Harvest,
            Self::Gather => CallKind::Gather,
            Self::GateReserve(_) => CallKind::GateReserve,
            Self::GateChange(_) => CallKind::GateChange,
            Self::BirthdayPartyDelivery(_) => CallKind::BirthdayPartyDelivery,
            Self::BirthdayPartyGather(_) => CallKind::BirthdayPartyGather,
            Self::HomeAction(_) => CallKind::HomeAction,
            Self::Craft(_) => CallKind::Craft,
            Self::HousingSketch(_) => CallKind::HousingSketch,
            Self::HousingLayout(_) => CallKind::HousingLayout,
            Self::CharacterTalkRead(_) => CallKind::CharacterTalkRead,
            Self::MusicPlaySet(_) => CallKind::MusicPlaySet,
            Self::MusicPlayEject(_) => CallKind::MusicPlayEject,
            Self::SystemFixtureAction(_) => CallKind::SystemFixtureAction,
            Self::Release(_) => CallKind::Release,
            Self::InventoryRecycle(_) => CallKind::InventoryRecycle,
            Self::BoostStaminaRecovery(_) => CallKind::BoostStaminaRecovery,
            Self::Convert { .. } => CallKind::Convert,
            Self::NormalMissionReward(_) => CallKind::NormalMissionReward,
            Self::Admin(_) => CallKind::Admin,
        }
    }

    /// The call's name, for logs and for a refused link's notice.
    pub fn name(&self) -> &'static str {
        self.kind().name()
    }
}

/// A call without its request, so the game can name the call a reply or a
/// push belongs to without copying the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallKind {
    Join,
    Harvest,
    Gather,
    GateReserve,
    GateChange,
    BirthdayPartyDelivery,
    BirthdayPartyGather,
    HomeAction,
    Craft,
    HousingSketch,
    HousingLayout,
    CharacterTalkRead,
    MusicPlaySet,
    MusicPlayEject,
    SystemFixtureAction,
    Release,
    InventoryRecycle,
    BoostStaminaRecovery,
    Convert,
    NormalMissionReward,
    Admin,
}

impl CallKind {
    /// The call's name, for logs and for a refused link's notice.
    pub fn name(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Harvest => "harvest",
            Self::Gather => "gather",
            Self::GateReserve => "gate reserve",
            Self::GateChange => "gate change",
            Self::BirthdayPartyDelivery => "birthday party delivery",
            Self::BirthdayPartyGather => "birthday party gather",
            Self::HomeAction => "home action",
            Self::Craft => "craft",
            Self::HousingSketch => "housing sketch",
            Self::HousingLayout => "housing layout",
            Self::CharacterTalkRead => "character talk read",
            Self::MusicPlaySet => "music play set",
            Self::MusicPlayEject => "music play eject",
            Self::SystemFixtureAction => "system fixture action",
            Self::Release => "release",
            Self::InventoryRecycle => "inventory recycle",
            Self::BoostStaminaRecovery => "boost stamina recovery",
            Self::Convert => "convert",
            Self::NormalMissionReward => "normal mission reward",
            Self::Admin => "admin",
        }
    }
}

/// The control plane's requests. Their replies carry JSON text, which the
/// caller parses: a document or schema view, an edit's receipt, or the seed
/// install's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AdminCall {
    /// Apply one `server.edit` command, as JSON text.
    Edit { command_json: String },
    /// The editable fields of the document.
    Schema,
    /// The document with its clock, the pending sections and the errors.
    Document,
    /// Install the browser game's seed documents into the server: the server
    /// document replaces the user's, and the local document is the page's.
    Install {
        server_json: Option<String>,
        local_json: Option<String>,
    },
}

/// A request's own answer, beside the sections it carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    /// The join's answer.
    Joined,
    /// The harvest reply's stamina.
    Harvest(Stamina),
    /// The gather reply's stamina (`None` while the pools are unseated).
    Gather(Option<Stamina>),
    /// A gate reserve or gate change reply, for the gate presenter.
    Gate(crate::gate::GateReply),
    /// The birthday-party delivery reply.
    Delivery(crate::api::DeliveryResponse),
    /// The birthday-party gather reply.
    GatherDelivery(crate::api::GatherResponse),
    /// A home action's reply.
    HomeAction(crate::api::HomeActionReply),
    /// A `SuiteUser` reply: a craft, canvas, sketch or layout save.
    SuiteUser(crate::api::SuiteUserReply),
    /// The talk read report's reply.
    TalkRead(crate::api::TalkReadReply),
    /// The music player's reply.
    MusicPlay(crate::api::MusicPlayReply),
    /// The system fixture action's reply.
    SystemFixtureAction(crate::api::SystemFixtureActionReply),
    /// The release reply is a `SuiteUser` answer: the phenomenon it learned
    /// is in the reply's sections.
    Release,
    /// The inventory screen's recycle and disassemble reply.
    InventoryRecycle(crate::api::MysekaiInventoryRecycleResponse),
    /// The boost stamina recovery reply (a `SuiteUser` answer); the stamina
    /// it restored is in the reply's sections.
    BoostStaminaRecovery(crate::api::SuiteUserReply),
    /// A convert start or shorten reply (a `SuiteUser` answer).
    Convert(crate::api::SuiteUserReply),
    /// The convert receive reply: the resources received and the tables the
    /// request changed.
    ConvertReceive(UserMysekaiConvertReceiveResponse),
    /// The normal mission reward reply: the reward resources and the tables
    /// the receive changed.
    NormalMissionReward(UserMysekaiNormalMissionRewardResponse),
    /// An administration answer, as JSON text.
    Admin(String),
}

/// The user data tables a response carries. A `None` field is not carried and
/// leaves the game's copy as it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sections {
    /// The server time the sections were taken at, in milliseconds since the
    /// Unix epoch.
    pub server_date_ms: i64,
    /// Whether the server entered a new refresh window for this response.
    pub is_refreshed: bool,
    /// `userMysekaiGamedata`.
    pub gamedata: Option<Gamedata>,
    /// `userMysekaiStamina`.
    pub stamina: Option<Stamina>,
    /// `userMysekaiColorfulPass`; `Some(None)` clears the pass the game holds.
    pub colorful_pass: Option<Option<ColorfulPass>>,
    /// `mysekaiPhenomenaSchedules` (the rows the schedule write reads).
    pub schedules: Option<Vec<ScheduleRow>>,
    /// `userMysekaiGate`: the home gate row, whose gate and skin ids select
    /// the home gate model.
    pub gate: Option<Gate>,
    /// The `mysekaiRankReleases` rows this server appends to the master table
    /// for the extended site-level release policy (empty when the document
    /// states the source's own policy).
    pub rank_releases: Option<Vec<RankReleaseRow>>,
    /// The birthday-party delivery sections.
    pub delivery: DeliveryUpdate,
    /// `userMysekaiMusicPlayFixtureSettings`.
    pub music: Option<Vec<MusicPlaySetting>>,
    /// `userAvatar`.
    pub avatar: Option<UserAvatar>,
    /// The owned MySekai tables (materials, their possession, fixtures,
    /// canvases, blueprints, items, the gamedata's levels).
    pub inventory: SuiteUserSections,
    /// `userMysekaiMusicRecords`.
    pub music_records: Option<Vec<OwnedMusicRecord>>,
    /// `userMysekaiSystemFixtureActions`.
    pub system_fixture_actions: Option<Vec<UserMysekaiSystemFixtureAction>>,
    /// `userMysekaiTools`.
    pub tools: Option<Vec<UserMysekaiTool>>,
    /// `userMysekaiPhenomenas`.
    pub phenomena: Option<Vec<UserMysekaiPhenomena>>,
    /// `userMysekaiBlueprintLendCraftCounts`: the crafts each blueprint term
    /// has used (the craft screen's remaining-count and term-cap source).
    pub blueprint_lend_craft_counts:
        Option<Vec<crate::rows::UserMysekaiBlueprintLendCraftCount>>,
    /// `userMysekaiConvertSlots`.
    pub convert_slots: Option<Vec<ConvertSlot>>,
    /// `userMysekaiConvertItemHistories`.
    pub convert_item_histories: Option<Vec<ConvertItemHistory>>,
    /// `userMysekaiNormalMissions`: the user's mission rows.
    pub normal_missions: Option<Vec<UserMysekaiNormalMission>>,
    /// `userMysekaiNormalMissionSheet`: the sheet the user is on.
    pub normal_mission_sheet: Option<UserMysekaiNormalMissionSheet>,
    /// `mysekaiConvertItems`: the master table the convert screen's item list
    /// is built from, with the `convertLimit` and `onceConvertLimit` its
    /// document does not carry.
    #[serde(rename = "mysekaiConvertItems")]
    pub mysekai_convert_items: Option<Vec<crate::rows::UserMysekaiConvertItem>>,
    /// The developer server panel's NPC slice as JSON text: the gate and its
    /// visitors, the schedule, the talk list, the layouts and the controls the
    /// response changed.
    #[serde(rename = "npcPanel")]
    pub npc_panel: Option<String>,
}

/// A frame could not be read.
#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unreadable frame: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// A message as one binary frame.
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    bitcode::serialize(message).expect("protocol messages are always serializable")
}

/// A message from one binary frame.
pub fn decode<T: DeserializeOwned>(frame: &[u8]) -> Result<T, DecodeError> {
    bitcode::deserialize(frame).map_err(|error| DecodeError(error.to_string()))
}

/// A message as JSON, for logs and tools.
pub fn to_json<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).expect("protocol messages are always serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{
        BirthdayPartyRow, ConvertItemHistory, ConvertObtainResource, ConvertSlot, CoordinateRow,
        Gate, HonorRow, MusicPlaySetting, RankReleaseRow, Reward, UserMysekaiBlueprint,
        UserMysekaiBlueprintLendCraftCount, UserMysekaiCanvas, UserMysekaiCharacterTalk,
        UserMysekaiFixture, UserMysekaiGamedata, UserMysekaiItem, UserMysekaiMaterial,
        UserMysekaiMaterialPossession,
    };

    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn a_session_round_trips_through_frames() {
        let messages = [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                user: UserId::local(),
                client: "moly-app 0.0.3".to_owned(),
            },
            ClientMessage::Call {
                seq: 7,
                call: Call::Join,
            },
            ClientMessage::Call {
                seq: 8,
                call: Call::Craft(UserMysekaiCraftRequest {
                    blueprint_id: 1,
                    texture_id: Some(2),
                    card_id: None,
                    is_special_training: None,
                    quantity: 3,
                    mysekai_blueprint_term_id: Some(4),
                }),
            },
            ClientMessage::Call {
                seq: 9,
                call: Call::Admin(AdminCall::Install {
                    server_json: Some("{}".to_owned()),
                    local_json: None,
                }),
            },
            ClientMessage::Call {
                seq: 10,
                call: Call::BoostStaminaRecovery(crate::api::UserMysekaiStaminaRequest {
                    mysekai_boost_stamina_recovery_type: crate::api::BoostStaminaRecoveryType::BoostItem,
                    user_mysekai_stamina_recovery_consume_resources: vec![
                        crate::api::StaminaRecoveryConsume {
                            resource_id: 1201,
                            quantity: 1,
                        },
                    ],
                }),
            },
        ];
        for message in messages {
            assert_eq!(decode::<ClientMessage>(&encode(&message)).unwrap(), message);
        }
        let reply = ServerMessage::Reply {
            seq: 7,
            outcome: Err(Refusal::new("not_joined", "join first")),
            sections: Some(Box::new(Sections {
                server_date_ms: 1_760_000_000_000,
                is_refreshed: true,
                phenomena: Some(vec![UserMysekaiPhenomena {
                    mysekai_phenomena_id: 1,
                    obtained_at: 1_760_000_000_000,
                }]),
                ..Sections::default()
            })),
        };
        assert_eq!(decode::<ServerMessage>(&encode(&reply)).unwrap(), reply);
    }

    /// Every carried table has a row, so a shape the binary codec cannot
    /// read shows up here rather than on the wire.
    fn full_sections() -> Sections {
        let material = UserMysekaiMaterial {
            mysekai_material_id: 3,
            quantity: 4,
        };
        let mut delivery = DeliveryUpdate::default();
        delivery.parties = Some(vec![BirthdayPartyRow {
            birthday_party_id: 1,
            delivery_total_point: 2,
            dropped_mysekai_material_count: 3,
            obtained_mysekai_material_count: 4,
        }]);
        delivery.materials = Some(BTreeMap::from([(5, 6)]));
        delivery.mysekai_materials = Some(BTreeMap::from([(7, 8)]));
        delivery.cards = Some(BTreeSet::from([9]));
        delivery.honors = Some(vec![HonorRow {
            honor_id: 10,
            level: 11,
            obtained_at: 12,
        }]);
        delivery.configs = Some(BTreeMap::from([("a".to_owned(), 13)]));
        Sections {
            server_date_ms: 1_760_000_000_000,
            is_refreshed: true,
            gamedata: Some(Gamedata {
                mysekai_rank: Some(1),
                total_exp: 2,
                refreshed_at: 3,
                is_mysekai_tutorial_end: true,
                mysekai_convert_fixture_level: Some(1),
            }),
            stamina: Some(Stamina {
                normal: 4,
                enhance: 5,
                boost: 6,
            }),
            colorful_pass: Some(Some(ColorfulPass {
                id: 7,
                expired_at: 8,
            })),
            schedules: Some(vec![ScheduleRow {
                refresh_time_period_id: 9,
                schedule_date: 10,
                phenomena_id: 11,
            }]),
            gate: Some(Gate {
                gate_id: 12,
                skin_id: 13,
                level: 1,
                visit_count: 2,
                last_refreshed_at: 14,
                is_setting_at_home_site: true,
            }),
            rank_releases: Some(vec![RankReleaseRow {
                mysekai_rank: 15,
                mysekai_rank_release_type: "mysekai_site_level".to_owned(),
                external_id: 16,
            }]),
            delivery,
            music: Some(vec![MusicPlaySetting {
                mysekai_site_id: 12,
                mysekai_music_record_id: 13,
                music_vocal_id: 14,
                is_instrumental: true,
            }]),
            avatar: Some(UserAvatar {
                costume: Some(15),
                accessory: Some(16),
                skin_color: Some(17),
                coordinate: Some(18),
            }),
            inventory: SuiteUserSections {
                user_mysekai_character_talks: Some(vec![UserMysekaiCharacterTalk {
                    mysekai_character_talk_id: 19,
                    is_read: true,
                }]),
                user_mysekai_materials: Some(vec![material]),
                user_mysekai_material_possession: Some(UserMysekaiMaterialPossession {
                    quantity: 20,
                }),
                user_mysekai_fixtures: Some(vec![UserMysekaiFixture {
                    mysekai_fixture_id: 21,
                    texture_id: 22,
                    quantity: 23,
                    last_obtained_at: 24,
                }]),
                user_mysekai_canvases: Some(vec![UserMysekaiCanvas {
                    mysekai_fixture_id: 25,
                    card_id: 26,
                    is_special_training: true,
                    quantity: 27,
                    last_obtained_at: 28,
                }]),
                user_mysekai_blueprints: Some(vec![UserMysekaiBlueprint {
                    mysekai_blueprint_id: 29,
                    obtained_at: 30,
                }]),
                user_mysekai_items: Some(vec![UserMysekaiItem {
                    mysekai_item_id: 31,
                    quantity: 32,
                    last_obtained_at: 33,
                }]),
                user_mysekai_gamedata: Some(UserMysekaiGamedata {
                    mysekai_rank: Some(34),
                    total_exp: 35,
                    mysekai_material_possession_level: 36,
                    mysekai_fixture_possession_level: 37,
                }),
                user_mysekai_tools: Some(vec![UserMysekaiTool {
                    mysekai_tool_id: 41,
                    quantity: 42,
                    durability: 43,
                    last_obtained_at: 44,
                }]),
                user_mysekai_music_records: Some(vec![OwnedMusicRecord {
                    mysekai_music_record_id: 38,
                    obtained_at: 39,
                }]),
                user_mysekai_convert_slots: Some(vec![ConvertSlot {
                    slot_id: 0,
                    mysekai_convert_item_id: 47,
                    choice_id: 0,
                    convert_start_at: 48,
                    once_convert_minutes: 49,
                    convert_count: 50,
                    received_progress: 0,
                    mysekai_convert_slot_status: crate::rows::CONVERT_SLOT_PROCESSING.to_owned(),
                    obtain_resources: Vec::new(),
                }]),
                user_mysekai_convert_item_histories: Some(vec![ConvertItemHistory {
                    mysekai_convert_item_id: 47,
                    convert_count: 1,
                    total_convert_count: 1,
                    last_convert_start_at: 48,
                }]),
                user_mysekai_normal_missions: Some(vec![UserMysekaiNormalMission {
                    mysekai_normal_mission_id: 53,
                    progress: 2,
                    mysekai_mission_status: crate::rows::MISSION_ACHIEVED.to_owned(),
                }]),
                user_mysekai_normal_mission_sheet: Some(UserMysekaiNormalMissionSheet {
                    mysekai_normal_mission_sheet_id: 54,
                }),
            },
            music_records: Some(vec![OwnedMusicRecord {
                mysekai_music_record_id: 38,
                obtained_at: 39,
            }]),
            system_fixture_actions: Some(vec![UserMysekaiSystemFixtureAction {
                mysekai_system_fixture_id: 40,
            }]),
            phenomena: Some(vec![UserMysekaiPhenomena {
                mysekai_phenomena_id: 2,
                obtained_at: 55,
            }]),
            tools: Some(vec![UserMysekaiTool {
                mysekai_tool_id: 41,
                quantity: 42,
                durability: 43,
                last_obtained_at: 44,
            }]),
            blueprint_lend_craft_counts: Some(vec![UserMysekaiBlueprintLendCraftCount {
                mysekai_blueprint_term_id: 45,
                craft_count: 46,
            }]),
            convert_slots: Some(vec![ConvertSlot {
                slot_id: 0,
                mysekai_convert_item_id: 47,
                choice_id: 0,
                convert_start_at: 48,
                once_convert_minutes: 49,
                convert_count: 50,
                received_progress: 0,
                mysekai_convert_slot_status: crate::rows::CONVERT_SLOT_PROCESSING.to_owned(),
                obtain_resources: vec![ConvertObtainResource {
                    slot_id: 0,
                    progress: 1,
                    seq: 1,
                    resource_type: "mysekai_material".to_owned(),
                    resource_id: 51,
                    quantity: 52,
                }],
            }]),
            convert_item_histories: Some(vec![ConvertItemHistory {
                mysekai_convert_item_id: 47,
                convert_count: 1,
                total_convert_count: 1,
                last_convert_start_at: 48,
            }]),
            mysekai_convert_items: Some(vec![crate::rows::UserMysekaiConvertItem {
                mysekai_convert_item_id: 47,
                obtain_type: "resource_box".to_owned(),
                external_id: 10047,
                required_convert_fixture_level: 1,
                convert_minutes: 49,
                convert_limit: 0,
                once_convert_limit: 50,
                refresh_cycle: "none".to_owned(),
                start_at: 0,
                end_at: 0,
            }]),
            npc_panel: Some("{\"version\":1}".to_owned()),
            normal_missions: Some(vec![UserMysekaiNormalMission {
                mysekai_normal_mission_id: 53,
                progress: 2,
                mysekai_mission_status: crate::rows::MISSION_ACHIEVED.to_owned(),
            }]),
            normal_mission_sheet: Some(UserMysekaiNormalMissionSheet {
                mysekai_normal_mission_sheet_id: 54,
            }),
        }
    }

    /// The mission section set round-trips through a frame, and carries both
    /// mission tables.
    #[test]
    fn the_mission_sections_round_trip() {
        let sections = SuiteUserSections {
            user_mysekai_normal_missions: Some(vec![UserMysekaiNormalMission {
                mysekai_normal_mission_id: 3,
                progress: 1,
                mysekai_mission_status: crate::rows::MISSION_RECEIVED.to_owned(),
            }]),
            user_mysekai_normal_mission_sheet: Some(UserMysekaiNormalMissionSheet {
                mysekai_normal_mission_sheet_id: 2,
            }),
            ..SuiteUserSections::default()
        };
        assert_eq!(
            decode::<SuiteUserSections>(&encode(&sections)).unwrap(),
            sections
        );
        assert!(!sections.is_empty());
        assert!(SuiteUserSections::default().is_empty());
    }

    #[test]
    fn every_section_round_trips_through_frames() {
        let push = ServerMessage::Push {
            kind: PushKind::Refresh,
            sections: Box::new(full_sections()),
        };
        assert_eq!(decode::<ServerMessage>(&encode(&push)).unwrap(), push);
        // The row types beside the sections are frames of their own.
        let reward = Reward {
            resource_type: "honor".to_owned(),
            resource_id: 45,
            quantity: 46,
            level: Some(47),
        };
        assert_eq!(decode::<Reward>(&encode(&reward)).unwrap(), reward);
        let reply = crate::api::DeliveryResponse {
            dropped_reward_count: 48,
            obtained_delivery_total_rewards: vec![reward],
            is_refreshed: true,
        };
        assert_eq!(
            decode::<crate::api::DeliveryResponse>(&encode(&reply)).unwrap(),
            reply
        );
        let coordinate = CoordinateRow {
            costume_assetbundle_name: Some("a".to_owned()),
            accessory_assetbundle_name: None,
            skin_color_code: Some("#ffffff".to_owned()),
        };
        assert_eq!(
            decode::<CoordinateRow>(&encode(&coordinate)).unwrap(),
            coordinate
        );
    }

    #[test]
    fn every_call_knows_its_fieldless_kind() {
        assert_eq!(Call::Join.kind(), CallKind::Join);
        assert_eq!(
            Call::Harvest(crate::api::HarvestRequest {
                mysekai_site_id: 5,
                mysekai_site_harvest_fixture_target:
                    crate::api::MysekaiSiteHarvestFixtureTarget {
                        position_x: 3,
                        position_z: -4,
                        mysekai_site_harvest_fixture_id: 9,
                        hp: 0,
                        is_last_attack: true,
                    },
                mysekai_tool_used: None,
                mysekai_stamina_useds: vec![crate::api::MysekaiStaminaUsed {
                    mysekai_stamina_type: "normal".to_owned(),
                    rest: 7,
                }],
            })
            .kind(),
            CallKind::Harvest
        );
        assert_eq!(Call::Gather.kind(), CallKind::Gather);
        assert_eq!(
            Call::GateReserve(crate::api::GateReserveRequest {
                mysekai_game_character_unit_group_id: 4,
            })
            .kind(),
            CallKind::GateReserve
        );
        assert_eq!(
            Call::GateChange(crate::api::GateChangeRequest {
                mysekai_gate_id: 1,
                mysekai_gate_skin_id: 2,
                mysekai_game_character_unit_group_ids: vec![3],
            })
            .kind(),
            CallKind::GateChange
        );
        assert_eq!(
            Call::Admin(AdminCall::Schema).kind(),
            CallKind::Admin
        );
        assert_eq!(
            Call::BoostStaminaRecovery(crate::api::UserMysekaiStaminaRequest {
                mysekai_boost_stamina_recovery_type: crate::api::BoostStaminaRecoveryType::Jewel,
                user_mysekai_stamina_recovery_consume_resources: vec![
                    crate::api::StaminaRecoveryConsume {
                        resource_id: 1,
                        quantity: 30,
                    },
                ],
            })
            .kind(),
            CallKind::BoostStaminaRecovery
        );
        // The convert calls: start, receive and shorten, each with its slot.
        let convert = Call::Convert {
            slot_id: 1,
            post: crate::api::ConvertPost::Start(crate::api::UserMysekaiConvertStartRequest {
                mysekai_convert_item_id: 3,
                convert_count: 2,
                choice_id: 0,
            }),
        };
        assert_eq!(convert.kind(), CallKind::Convert);
        assert_eq!(convert.name(), "convert");
        assert_eq!(
            decode::<ClientMessage>(&encode(&ClientMessage::Call { seq: 11, call: convert }))
                .unwrap(),
            ClientMessage::Call {
                seq: 11,
                call: Call::Convert {
                    slot_id: 1,
                    post: crate::api::ConvertPost::Start(
                        crate::api::UserMysekaiConvertStartRequest {
                            mysekai_convert_item_id: 3,
                            convert_count: 2,
                            choice_id: 0,
                        }
                    ),
                }
            }
        );
        // The mission screen's receive: its ids, its kind and its reply.
        let reward = Call::NormalMissionReward(UserMysekaiNormalMissionRewardRequest {
            mysekai_normal_mission_ids: vec![100, 101],
        });
        assert_eq!(reward.kind(), CallKind::NormalMissionReward);
        assert_eq!(reward.name(), "normal mission reward");
        let answered = ServerMessage::Reply {
            seq: 12,
            outcome: Ok(Reply::NormalMissionReward(
                UserMysekaiNormalMissionRewardResponse {
                    obtained_rewards: vec![crate::rows::UserResource {
                        resource_type: "mysekai_material".to_owned(),
                        resource_id: 7,
                        resource_level: 0,
                        quantity: 3,
                    }],
                    updated_resources: SuiteUserSections {
                        user_mysekai_normal_missions: Some(vec![
                            crate::rows::UserMysekaiNormalMission {
                                mysekai_normal_mission_id: 101,
                                progress: 2,
                                mysekai_mission_status: crate::rows::MISSION_RECEIVED.to_owned(),
                            },
                        ]),
                        ..SuiteUserSections::default()
                    },
                },
            )),
            sections: None,
        };
        assert_eq!(decode::<ServerMessage>(&encode(&answered)).unwrap(), answered);
    }

    /// The release call round-trips through a frame and names itself; the
    /// learned phenomena it replies with are in `full_sections`.
    #[test]
    fn the_release_call_round_trips() {
        let call = Call::Release(crate::api::ReleaseRequest { mysekai_site_id: 5 });
        assert_eq!(call.kind(), CallKind::Release);
        assert_eq!(call.name(), "release");
        let message = ClientMessage::Call { seq: 13, call };
        assert_eq!(decode::<ClientMessage>(&encode(&message)).unwrap(), message);
    }

    #[test]
    fn a_truncated_frame_is_refused() {
        let frame = encode(&ServerMessage::Refused {
            reason: "protocol 2 is not 1".to_owned(),
        });
        assert!(decode::<ServerMessage>(&frame[..frame.len() - 1]).is_err());
    }
}
