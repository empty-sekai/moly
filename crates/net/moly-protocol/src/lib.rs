//! The messages the game and its server exchange.
//!
//! One connection carries one user's session:
//!
//! 1. The game opens with [`ClientMessage::Hello`], naming the protocol
//!    version and the user.
//! 2. The server answers [`ServerMessage::Welcome`], or
//!    [`ServerMessage::Refused`] and closes.
//! 3. Each [`ClientMessage::Call`] carries a sequence number that its
//!    [`ServerMessage::Reply`] repeats. Replies arrive in call order.
//! 4. [`ServerMessage::Push`] carries user data the server sends on its own,
//!    such as the sections of a new refresh window.
//!
//! Every reply and push may carry [`Sections`]: the user data tables the
//! game keeps copies of. The game writes its copies from these and from
//! nothing else.
//!
//! Frames are binary ([`encode`] / [`decode`]); [`to_json`] renders any
//! message as JSON for logs and tools.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::api::{
    BirthdayPartyDeliveryRequest, BirthdayPartyGatherRequest, CharacterTalkReadRequest,
    GateChangeRequest, GateReserveRequest, HarvestRequest, HomeActionPost, MusicPlayEjectRequest,
    MusicPlaySetRequest, SystemFixtureActionRequest, UserMysekaiCraftRequest,
    UserMysekaiHousingEditLayoutRequest, UserMysekaiHousingSketchRequest,
};
use crate::rows::{
    ColorfulPass, DeliveryUpdate, Gamedata, MusicPlaySetting, OwnedMusicRecord, ScheduleRow,
    Stamina, SuiteUserSections, UserAvatar, UserMysekaiSystemFixtureAction, UserMysekaiTool,
};

pub mod api;
pub mod gate;
pub mod json;
pub mod rows;

/// Raised whenever a message changes shape. A peer with another version is
/// refused at the handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// The account a session belongs to. The server keeps one document per user.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UserId(pub String);

impl UserId {
    /// The user a single-player game plays as.
    pub fn local() -> Self {
        Self("local".to_owned())
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
    /// The first message of a session.
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
    /// The session is open.
    Welcome {
        protocol: u32,
        user: UserId,
        /// The server's name and version, for the game's log.
        server: String,
    },
    /// The session is refused; the server closes the connection after this.
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
    /// The control plane's calls: the document and the schema, an edit, and
    /// the browser game's seed install.
    Admin(AdminCall),
}

impl Call {
    /// The call's name, for logs and for a refused link's notice.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Harvest(_) => "harvest",
            Self::Gather => "gather",
            Self::GateReserve(_) => "gate reserve",
            Self::GateChange(_) => "gate change",
            Self::BirthdayPartyDelivery(_) => "birthday party delivery",
            Self::BirthdayPartyGather(_) => "birthday party gather",
            Self::HomeAction(_) => "home action",
            Self::Craft(_) => "craft",
            Self::HousingSketch(_) => "housing sketch",
            Self::HousingLayout(_) => "housing layout",
            Self::CharacterTalkRead(_) => "character talk read",
            Self::MusicPlaySet(_) => "music play set",
            Self::MusicPlayEject(_) => "music play eject",
            Self::SystemFixtureAction(_) => "system fixture action",
            Self::Admin(_) => "admin",
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
        BirthdayPartyRow, CoordinateRow, HonorRow, MusicPlaySetting, Reward, UserMysekaiBlueprint,
        UserMysekaiCanvas, UserMysekaiCharacterTalk, UserMysekaiFixture, UserMysekaiGamedata,
        UserMysekaiItem, UserMysekaiMaterial, UserMysekaiMaterialPossession,
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
                }),
            },
            ClientMessage::Call {
                seq: 9,
                call: Call::Admin(AdminCall::Install {
                    server_json: Some("{}".to_owned()),
                    local_json: None,
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
            },
            music_records: Some(vec![OwnedMusicRecord {
                mysekai_music_record_id: 38,
                obtained_at: 39,
            }]),
            system_fixture_actions: Some(vec![UserMysekaiSystemFixtureAction {
                mysekai_system_fixture_id: 40,
            }]),
            tools: Some(vec![UserMysekaiTool {
                mysekai_tool_id: 41,
                quantity: 42,
                durability: 43,
                last_obtained_at: 44,
            }]),
        }
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
    fn a_truncated_frame_is_refused() {
        let frame = encode(&ServerMessage::Refused {
            reason: "protocol 2 is not 1".to_owned(),
        });
        assert!(decode::<ServerMessage>(&frame[..frame.len() - 1]).is_err());
    }
}
