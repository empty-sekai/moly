//! One system answers every call: it finds the session's user model, runs the
//! matching model method, and answers with the sections that method queued.
//!
//! A call before the masters are in is refused with `masters_loading` and the
//! tables still missing; a stored document that cannot be read is refused
//! with `document_invalid`. A refusal keeps the model's own text and names
//! its case in `code`.

use bevy::prelude::*;
use moly_protocol::api::{
    GatherContent, HomeActionReply, MusicPlayPost, SystemFixtureActionReply, TalkReadReply,
};
use moly_protocol::{AdminCall, Call, Refusal, Reply, Sections, ServerMessage};
use serde_json::{json, Map};

use crate::admin;
use crate::defaults::Defaults;
use crate::hub::{CallReceived, Sessions};
use crate::masters::{Masters, Tables};
use crate::model::{ResponseKind, ServerModel};
use crate::users::Users;
use crate::{AdminClosed, ListenAddress};
use moly_masters::load::MasterTables;

/// One call's answer.
#[derive(Clone)]
pub(crate) struct Answered {
    pub(crate) outcome: Result<Reply, Refusal>,
    /// The sections the model queued while answering.
    pub(crate) sections: Option<Box<Sections>>,
}

impl Answered {
    fn refused(refusal: Refusal) -> Self {
        Self {
            outcome: Err(refusal),
            sections: None,
        }
    }
}

/// Why an administration call cannot run here: the host closed
/// administration, the player socket is not a loopback address, or the server
/// did not report one.
pub(crate) fn admin_gate(
    closed: Option<&AdminClosed>,
    listen: Option<&ListenAddress>,
) -> Result<(), Refusal> {
    if closed.is_some() {
        return Err(Refusal::new(
            "admin_not_allowed",
            "administration is closed on this server",
        ));
    }
    match listen {
        Some(listen) if listen.0.ip().is_loopback() => Ok(()),
        Some(listen) => Err(Refusal::new(
            "admin_not_allowed",
            format!(
                "administration is only accepted on a loopback address; this server listens on {}",
                listen.0
            ),
        )),
        None => Err(Refusal::new(
            "admin_not_allowed",
            "administration needs the server's listen address, which was not reported",
        )),
    }
}

/// The dispatch system.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_calls(
    mut calls: MessageReader<CallReceived>,
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    sessions: Res<Sessions>,
    defaults: Option<Res<Defaults>>,
    listen: Option<Res<ListenAddress>>,
    closed: Option<Res<AdminClosed>>,
    shared: Option<Res<MasterTables>>,
) {
    let tables = Tables::new(&masters, shared.as_deref());
    for call in calls.read() {
        let session = call.session;
        let user = call.user.clone();
        // The administration calls run only on the loopback player socket.
        if matches!(&call.call, Call::Admin(_)) {
            if let Err(refusal) = admin_gate(closed.as_deref(), listen.as_deref()) {
                sessions.send(
                    session,
                    &ServerMessage::Reply {
                        seq: call.seq,
                        outcome: Err(refusal),
                        sections: None,
                    },
                );
                continue;
            }
        }
        let Some(entry) = users.0.get_mut(&user) else {
            sessions.send(
                session,
                &ServerMessage::Reply {
                    seq: call.seq,
                    outcome: Err(Refusal::new(
                        "no_user",
                        format!("user {user} has no session documents"),
                    )),
                    sections: None,
                },
            );
            continue;
        };
        // The join is what seats the user; it is not gated on having joined.
        let is_join = matches!(&call.call, Call::Join);
        let answered = match &call.call {
            // The seed install replaces the user's document whole; it is
            // also the call that repairs a stored document that was refused.
            Call::Admin(AdminCall::Install {
                server_json,
                local_json,
            }) => match admin::install(
                entry,
                server_json.clone(),
                local_json.as_deref(),
                defaults.as_deref(),
                &tables,
            ) {
                Ok(text) => Answered {
                    outcome: Ok(Reply::Admin(text)),
                    sections: None,
                },
                Err(refusal) => Answered::refused(refusal),
            },
            _ => {
                if let Some(reason) = &entry.load_error {
                    Answered::refused(Refusal::new("document_invalid", reason.clone()))
                } else if !masters.ready {
                    Answered::refused(Refusal::new("masters_loading", masters.lacking()))
                } else if entry.model.is_none() {
                    Answered::refused(Refusal::new(
                        "masters_loading",
                        "the user's document is still being seated",
                    ))
                } else {
                    let model = entry.model.as_mut().expect("checked above");
                    if !model.joined && !is_join {
                        Answered::refused(Refusal::new(
                            "not_joined",
                            "the server has not joined this user yet",
                        ))
                    } else {
                        answer_call(call.call.clone(), model, &tables)
                    }
                }
            }
        };
        sessions.send(
            session,
            &ServerMessage::Reply {
                seq: call.seq,
                outcome: answered.outcome,
                sections: answered.sections,
            },
        );
    }
}

/// One call answered by a joined model. The sections are the ones the model
/// queued for this answer.
pub(crate) fn answer_call(call: Call, model: &mut ServerModel, masters: &Tables<'_>) -> Answered {
    let before = model.responses.len();
    // An administration call's sections are pushes, not a reply.
    let admin = matches!(call, Call::Admin(_));
    let outcome = run(call, model, masters);
    let queued = model.responses.split_off(before);
    if admin {
        model.responses.extend(queued);
        return Answered {
            outcome,
            sections: None,
        };
    }
    Answered {
        outcome,
        sections: queued
            .into_iter()
            .next_back()
            .map(|response| Box::new(response.sections)),
    }
}

fn run(call: Call, model: &mut ServerModel, masters: &Tables<'_>) -> Result<Reply, Refusal> {
    match call {
        Call::Join => {
            // The join seats the user and answers with every section; a
            // repeated join answers again, as the source's login does.
            model.join(masters);
            Ok(Reply::Joined)
        }
        Call::Harvest(request) => Ok(Reply::Harvest(model.harvest(&request, masters)?)),
        Call::Gather(request) => Ok(Reply::Gather(model.gather(&request, masters)?)),
        Call::HarvestMap { mysekai_site_id } => {
            model.harvest_map(mysekai_site_id, masters)?;
            Ok(Reply::HarvestMap)
        }
        Call::BulkHarvest(request) => {
            Ok(Reply::BulkHarvest(model.bulk_harvest(&request, masters)?))
        }
        Call::TreasureBoxSpawn(request) => {
            model.harvest_spawn_box(&request, masters)?;
            Ok(Reply::TreasureBoxSpawn)
        }
        Call::GateReserve(request) => {
            let mut command = Map::new();
            command.insert("type".into(), json!("server.edit"));
            command.insert("action".into(), json!("gate.reserve"));
            command.insert(
                "mysekaiGameCharacterUnitGroupId".into(),
                json!(request.mysekai_game_character_unit_group_id),
            );
            model
                .edit(&command, masters)
                .map_err(|reason| Refusal::new("gate_refused", reason))?;
            let reply = model.gate_replies.pop().ok_or_else(|| {
                Refusal::new("gate_refused", "the gate reserve recorded no reply")
            })?;
            Ok(Reply::Gate(reply))
        }
        Call::GateChange(request) => {
            let mut command = Map::new();
            command.insert("type".into(), json!("server.edit"));
            command.insert("action".into(), json!("gate.change"));
            command.insert("mysekaiGateId".into(), json!(request.mysekai_gate_id));
            command.insert(
                "mysekaiGateSkinId".into(),
                json!(request.mysekai_gate_skin_id),
            );
            command.insert(
                "mysekaiGameCharacterUnitGroupIds".into(),
                json!(request.mysekai_game_character_unit_group_ids),
            );
            model
                .edit(&command, masters)
                .map_err(|reason| Refusal::new("gate_refused", reason))?;
            let reply = model
                .gate_replies
                .pop()
                .ok_or_else(|| Refusal::new("gate_refused", "the gate change recorded no reply"))?;
            Ok(Reply::Gate(reply))
        }
        Call::BirthdayPartyDelivery(request) => {
            let (response, changed) = model
                .birthday_party_delivery(
                    request.birthday_party_id,
                    request.consumed_delivery_material_count,
                    masters,
                )
                .map_err(|reason| {
                    Refusal::new(
                        "delivery_refused",
                        format!("PutUserMysekaiBirthdayPartyDeliveryApi: {reason}"),
                    )
                })?;
            model.delivery_reply(ResponseKind::BirthdayPartyDelivery, &changed, masters);
            Ok(Reply::Delivery(response))
        }
        Call::BirthdayPartyGather(request) => {
            let contents: &[GatherContent] = &request.contents;
            let (response, changed) =
                model
                    .birthday_party_gather(contents, masters)
                    .map_err(|reason| {
                        Refusal::new(
                            "gather_refused",
                            format!("PutUserMysekaiBirthdayPartyGatherApi: {reason}"),
                        )
                    })?;
            model.delivery_reply(ResponseKind::BirthdayPartyGather, &changed, masters);
            Ok(Reply::GatherDelivery(response))
        }
        Call::HomeAction(request) => {
            let reply: HomeActionReply =
                model.home_action(request.api, request.target.as_deref(), masters);
            Ok(Reply::HomeAction(reply))
        }
        Call::Craft(request) => {
            let answer = model.craft_reply(&request, masters);
            Ok(Reply::SuiteUser(answer.reply))
        }
        Call::HousingSketch(request) => {
            let answer = model.sketch_reply(&request, masters);
            Ok(Reply::SuiteUser(answer.reply))
        }
        Call::HousingLayout(request) => {
            let reply = model.housing_layout(request, masters);
            Ok(Reply::SuiteUser(reply))
        }
        Call::CharacterTalkRead(request) => {
            let (resources, changed) =
                model
                    .talk_read(request.mysekai_character_talk_id)
                    .map_err(|reason| {
                        Refusal::new(
                            "talk_read_refused",
                            format!("PutUserMysekaiCharacterTalkReadApi: {reason}"),
                        )
                    })?;
            model.respond(ResponseKind::CharacterTalkRead, false, &changed, masters);
            let updated = model
                .responses
                .last()
                .expect("the response just recorded")
                .sections
                .inventory
                .clone();
            Ok(Reply::TalkRead(TalkReadReply {
                success: true,
                refusal: None,
                obtained_resources: resources,
                updated,
            }))
        }
        Call::MusicPlaySet(request) => Ok(Reply::MusicPlay(
            model.music_play(MusicPlayPost::Set(request), masters),
        )),
        Call::MusicPlayEject(request) => Ok(Reply::MusicPlay(
            model.music_play(MusicPlayPost::Eject(request), masters),
        )),
        Call::SystemFixtureAction(request) => {
            let obtained = model
                .system_fixture_action(request.mysekai_system_fixture_id, masters)
                .map_err(|reason| {
                    Refusal::new(
                        "system_fixture_action_refused",
                        format!("PostUserMysekaiSystemFixtureActionApi: {reason}"),
                    )
                })?;
            Ok(Reply::SystemFixtureAction(SystemFixtureActionReply {
                success: true,
                refusal: None,
                obtained_resources: obtained,
            }))
        }
        Call::Release(request) => {
            model
                .release(request.mysekai_site_id, masters)
                .map_err(|reason| {
                    Refusal::new(
                        "release_refused",
                        format!("{}: {reason}", crate::model::phenomena::API),
                    )
                })?;
            Ok(Reply::Release)
        }
        Call::InventoryRecycle(request) => {
            let reply = model
                .inventory_disassemble(request, masters)
                .map_err(|reason| {
                    Refusal::new(
                        "recycle_refused",
                        format!("{}: {reason}", request.api_name()),
                    )
                })?;
            Ok(Reply::InventoryRecycle(reply))
        }
        Call::BoostStaminaRecovery(request) => {
            let reply = model
                .boost_stamina_recovery(&request, masters)
                .map_err(|refusal| {
                    Refusal::new(
                        format!("stamina_recovery_{}", refusal.code),
                        format!("PostUserMysekaiBoostStaminaRecoveryApi: {}", refusal.reason),
                    )
                })?;
            Ok(Reply::BoostStaminaRecovery(reply))
        }
        Call::Convert { slot_id, post } => {
            model
                .convert_reply(slot_id, post, masters)
                .map_err(|reason| {
                    Refusal::new("convert_refused", format!("{}: {reason}", post.api_name()))
                })
        }
        Call::NormalMissionReward(request) => model
            .normal_mission_reward(&request.mysekai_normal_mission_ids, masters)
            .map(Reply::NormalMissionReward)
            .map_err(|reason| {
                Refusal::new(
                    "normal_mission_refused",
                    format!("PostUserMysekaiNormalMissionRewardApi: {reason}"),
                )
            }),
        Call::Admin(call) => admin::answer(call, model, masters).map(Reply::Admin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::craft::BASE_TEXTURE_ID;
    use crate::model::document::{self, Clock, Migration};
    use crate::model::{Origin, SECTION_STAMINA};
    use moly_protocol::api::{
        BirthdayPartyDeliveryRequest, BirthdayPartyGatherRequest, CharacterTalkReadRequest,
        GateChangeRequest, GateReserveRequest, HarvestRequest, HomeActionPost,
        MusicPlayEjectRequest, MusicPlayReply, MusicPlaySetRequest,
        MysekaiSiteHarvestFixtureTarget, MysekaiStaminaUsed, MysekaiToolUsed,
        SystemFixtureActionRequest, UserMysekaiCraftRequest, UserMysekaiHousingEditLayoutRequest,
        UserMysekaiHousingSketchRequest,
    };
    use moly_protocol::gate::GateReply;
    use moly_protocol::rows::{
        MusicPlaySetting, OwnedMusicRecord, Stamina, UserMysekaiFixture, UserMysekaiNormalMission,
        UserMysekaiTool, MISSION_ACHIEVED, MISSION_NOT_ACHIEVED, MISSION_RECEIVED,
    };
    use moly_protocol::{AdminCall, UserId};

    /// The shared synthetic tables, plus the gate ids the gate requests are
    /// checked against.
    fn fixture_masters() -> Masters {
        let mut masters = Masters::fixture();
        masters.gates = Some((vec![1], vec![1]));
        masters
    }

    /// The checked-in product default with a fixed clock, as the game's model
    /// tests build it.
    fn fixture_model() -> ServerModel {
        let doc = document::migrate_v1(
            crate::defaults::CHECKED_IN_SLICE,
            Migration::CheckedInDefault,
        )
        .expect("the checked-in slice migrates");
        let mut model = ServerModel::new(doc, Origin::CheckedInDefault, Vec::new());
        model.doc.clock = Clock::Fixed {
            at_ms: 1_790_300_000_000,
        };
        model
    }

    fn joined_model(masters: &Tables<'_>) -> ServerModel {
        let mut model = fixture_model();
        model.join(masters);
        model.responses.clear();
        model
    }

    fn ok(answered: Answered) -> Reply {
        match answered.outcome {
            Ok(reply) => reply,
            Err(refusal) => panic!("refused: {refusal:?}"),
        }
    }

    /// `mysekaiBoostItems` for the recovery test: id 7 recovers 5 boost
    /// stamina.
    fn recovery_masters() -> Masters {
        let mut masters = fixture_masters();
        masters.boost_items = Some(vec![moly_masters::BoostItemRow {
            id: 7,
            recovery_value: 5,
        }]);
        masters
    }

    /// `PostUserMysekaiBoostStaminaRecoveryApi` with the recovery master row
    /// (1 live bonus per 100 boost stamina): one live bonus restores 100 and
    /// one boost item row restores 5 · quantity · 100, the pool stops at its
    /// maximum, and a request that restores nothing is refused.
    #[test]
    fn the_boost_stamina_recovery_restores_by_the_recovery_row() {
        let masters = recovery_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.stamina = Some(Stamina {
            normal: 1000,
            enhance: 0,
            boost: 100,
        });
        let call = |consumes: Vec<(i32, i32)>| {
            Call::BoostStaminaRecovery(moly_protocol::api::UserMysekaiStaminaRequest {
                mysekai_boost_stamina_recovery_type:
                    moly_protocol::api::BoostStaminaRecoveryType::Boost,
                user_mysekai_stamina_recovery_consume_resources: consumes
                    .into_iter()
                    .map(
                        |(resource_id, quantity)| moly_protocol::api::StaminaRecoveryConsume {
                            resource_id,
                            quantity,
                        },
                    )
                    .collect(),
            })
        };
        let answered = answer_call(call(vec![(0, 2)]), &mut model, &masters);
        match ok(answered.clone()) {
            Reply::BoostStaminaRecovery(reply) => assert!(reply.success),
            other => panic!("answered {other:?}"),
        }
        let sections = answered.sections.expect("the reply carries the stamina");
        assert_eq!(sections.stamina.map(|stamina| stamina.boost), Some(300));
        // The pool stops at the master maximum (10000).
        let answered = answer_call(call(vec![(0, 500)]), &mut model, &masters);
        let sections = answered.sections.expect("the reply carries the stamina");
        assert_eq!(sections.stamina.map(|stamina| stamina.boost), Some(10000));
        // A full pool refuses by name.
        let answered = answer_call(call(vec![(0, 1)]), &mut model, &masters);
        assert!(answered.outcome.is_err());
        let mut items = call(vec![(7, 3)]);
        if let Call::BoostStaminaRecovery(request) = &mut items {
            request.mysekai_boost_stamina_recovery_type =
                moly_protocol::api::BoostStaminaRecoveryType::BoostItem;
        }
        model.doc.stamina = Some(Stamina {
            normal: 1000,
            enhance: 0,
            boost: 100,
        });
        let answered = answer_call(items, &mut model, &masters);
        let sections = answered.sections.expect("the reply carries the stamina");
        assert_eq!(sections.stamina.map(|stamina| stamina.boost), Some(1600));
        // Nothing spent: refused.
        let answered = answer_call(call(Vec::new()), &mut model, &masters);
        assert!(answered.outcome.is_err());
    }

    #[test]
    fn the_join_seats_the_user_and_carries_every_section() {
        let masters = fixture_masters();
        let mut model = fixture_model();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let answered = answer_call(Call::Join, &mut model, &masters);
        assert_eq!(ok(answered.clone()), Reply::Joined);
        let sections = answered.sections.expect("the join carries its sections");
        assert!(sections.is_refreshed);
        assert_eq!(
            sections.stamina,
            Some(Stamina {
                normal: 1000,
                enhance: 0,
                boost: 100
            })
        );
        assert_eq!(sections.gamedata.expect("gamedata").mysekai_rank, Some(3));
        assert_eq!(
            sections.schedules.as_ref().map(Vec::len),
            Some(6),
            "three days of both refresh windows"
        );
        let panel: serde_json::Value = serde_json::from_str(
            sections
                .npc_panel
                .as_deref()
                .expect("the join carries the panel slice"),
        )
        .expect("the panel slice is JSON");
        assert_eq!(panel["version"].as_u64(), Some(1));
        assert_eq!(
            panel["userMysekaiGamedata"]["refreshedAt"].as_i64(),
            Some(1_790_300_000_000)
        );
        assert_eq!(model.doc.gamedata.refreshed_at, 1_790_300_000_000);
    }

    /// One merged harvest run on site 5, target (3, -4), fixture 9.
    fn harvest_request(rests: [i32; 3], tool: Option<(i32, i32, i32)>) -> HarvestRequest {
        HarvestRequest {
            mysekai_site_id: 5,
            mysekai_site_harvest_fixture_target: MysekaiSiteHarvestFixtureTarget {
                position_x: 3,
                position_z: -4,
                mysekai_site_harvest_fixture_id: 9,
                hp: 0,
                is_last_attack: true,
            },
            mysekai_tool_used: tool.map(|(mysekai_tool_id, use_count, durability)| {
                MysekaiToolUsed {
                    mysekai_tool_id,
                    use_count,
                    durability,
                }
            }),
            mysekai_stamina_useds: vec![
                MysekaiStaminaUsed {
                    mysekai_stamina_type: "normal".to_owned(),
                    rest: rests[0],
                },
                MysekaiStaminaUsed {
                    mysekai_stamina_type: "enhance".to_owned(),
                    rest: rests[1],
                },
                MysekaiStaminaUsed {
                    mysekai_stamina_type: "boost".to_owned(),
                    rest: rests[2],
                },
            ],
        }
    }

    // Harvest/gather settlement is covered in model::harvest::tests with
    // authoritative fixture/drop rows and exact pool/tool debits.

    #[test]
    fn the_gate_reserve_and_change_answer_with_the_gate_rows() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::GateReserve(GateReserveRequest {
                mysekai_game_character_unit_group_id: 4,
            }),
            &mut model,
            &masters,
        );
        let panel: serde_json::Value = serde_json::from_str(
            answered
                .sections
                .as_ref()
                .and_then(|sections| sections.npc_panel.as_deref())
                .expect("the reserve carries the panel slice"),
        )
        .expect("the panel slice is JSON");
        assert_eq!(
            panel["userMysekaiGateCharacterVisit"]["userMysekaiGateCharacters"]
                .as_array()
                .map(Vec::len),
            Some(model.doc.gate_characters.len())
        );
        match ok(answered) {
            Reply::Gate(GateReply::Reserve { row, rows, .. }) => {
                assert_eq!(row.unit_group_id, 4);
                assert!(row.is_reservation);
                assert_eq!(row.visit_count, 1);
                // The reply lists every gate character the server holds.
                assert_eq!(rows, model.doc.gate_characters);
                assert!(rows
                    .iter()
                    .any(|known| known.unit_group_id == 4 && known.is_reservation));
            }
            other => panic!("not the reserve's reply: {other:?}"),
        }
        assert!(model
            .doc
            .gate_characters
            .iter()
            .any(|row| row.unit_group_id == 4 && row.is_reservation));

        let answered = answer_call(
            Call::GateChange(GateChangeRequest {
                mysekai_gate_id: 1,
                mysekai_gate_skin_id: 1,
                mysekai_game_character_unit_group_ids: vec![1, 2],
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::Gate(GateReply::Change { gate_id, rows, .. }) => {
                assert_eq!(gate_id, 1);
                assert_eq!(rows.len(), 2);
                assert!(rows.iter().all(|row| !row.is_reservation));
            }
            other => panic!("not the change's reply: {other:?}"),
        }
    }

    #[test]
    fn the_delivery_refusal_keeps_the_models_text() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::BirthdayPartyDelivery(BirthdayPartyDeliveryRequest {
                birthday_party_id: 7,
                consumed_delivery_material_count: 1,
            }),
            &mut model,
            &masters,
        );
        assert_eq!(
            answered.outcome,
            Err(Refusal::new(
                "delivery_refused",
                "PutUserMysekaiBirthdayPartyDeliveryApi: party 7 is not in session at the server clock",
            ))
        );
        assert!(answered.sections.is_none());
    }

    #[test]
    fn the_gather_of_no_party_answers_with_no_rewards() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::BirthdayPartyGather(BirthdayPartyGatherRequest { contents: vec![] }),
            &mut model,
            &masters,
        );
        match ok(answered.clone()) {
            Reply::GatherDelivery(reply) => {
                assert!(reply.obtained_delivery_total_rewards.is_empty())
            }
            other => panic!("not the gather's reply: {other:?}"),
        }
        assert!(answered.sections.is_some());
    }

    #[test]
    fn the_home_action_answers_and_carries_the_pending_sections() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::HomeAction(HomeActionPost {
                api: moly_protocol::api::HomeActionApi::Craft,
                target: Some("fixture".to_owned()),
            }),
            &mut model,
            &masters,
        );
        assert_eq!(
            ok(answered.clone()),
            Reply::HomeAction(HomeActionReply { success: true })
        );
        assert!(answered.sections.is_some());
    }

    /// The blueprints, costs, first-craft bonus, fixture possession row and
    /// term row the game's craft tests use.
    fn craft_masters(tables: &mut MasterTables) {
        let upstream = |rows: serde_json::Value| rows.to_string();
        let craft = tables
            .craft
            .as_mut()
            .expect("the fixture seats the craft masters");
        moly_masters::load::parse_blueprints(
            &upstream(json!([
                {"id": 1, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 1, "isEnableSketch": true, "isObtainedByConvert": true, "craftCountLimit": 1, "isAvailableWithoutPossession": false},
                {"id": 4, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 4, "isEnableSketch": true, "isObtainedByConvert": true, "isAvailableWithoutPossession": false},
            ])),
            craft,
        )
        .unwrap();
        moly_masters::load::parse_costs(
            &upstream(json!([
                {"id": 1, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 1, "seq": 1, "quantity": 100},
                {"id": 2, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 2, "seq": 2, "quantity": 20},
                {"id": 3, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 14, "seq": 3, "quantity": 20},
                {"id": 4, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 15, "seq": 4, "quantity": 20},
                {"id": 5, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 8, "seq": 5, "quantity": 10},
                {"id": 10, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 1, "seq": 1, "quantity": 30},
                {"id": 11, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 21, "seq": 2, "quantity": 3},
                {"id": 12, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 10, "seq": 3, "quantity": 3},
            ])),
            craft,
        )
        .unwrap();
        moly_masters::load::parse_terms(
            &upstream(json!([
                {"id": 1, "mysekaiBlueprintId": 844, "startAt": 1_759_330_800_000_i64, "endAt": 1_759_849_199_000_i64},
                {"id": 2, "mysekaiBlueprintId": 4, "startAt": 1_790_000_000_000_i64, "endAt": 1_790_400_000_000_i64, "mysekaiBlueprintTermTabType": "birthday_anniversary", "mysekaiBlueprintTermMysekaiMaterialCostGroupId": 900, "craftLimit": 2},
                {"id": 3, "mysekaiBlueprintId": 4, "startAt": 1_700_000_000_000_i64, "endAt": 1_700_100_000_000_i64, "mysekaiBlueprintTermTabType": "limited_term"},
            ])),
            craft,
        )
        .unwrap();
        moly_masters::load::parse_term_costs(
            &upstream(json!([
                {"id": 900, "groupId": 900, "mysekaiMaterialId": 1, "seq": 1, "quantity": 5},
                {"id": 901, "groupId": 900, "mysekaiMaterialId": 21, "seq": 2, "quantity": 1},
            ])),
            craft,
        )
        .unwrap();
        moly_masters::load::parse_rank_obtained_exps(
            &upstream(json!([
                {"id": 1, "mysekaiRankObtainedExpType": "craft_mysekai_fixture_first_bonus", "quantity": 1000},
            ])),
            craft,
        )
        .unwrap();
        let possession = tables
            .possession
            .as_mut()
            .expect("the fixture seats the possession masters");
        moly_masters::load::parse_fixture_possessions(
            &upstream(json!([
                {"id": 1, "level": 1, "possessionLimit": 1000},
            ])),
            possession,
        )
        .unwrap();
    }

    #[test]
    fn the_craft_spends_its_cost_and_earns_the_first_craft_bonus() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.gamedata.total_exp = 0;
        model.doc.gamedata.mysekai_rank = Some(1);
        model.doc.delivery.mysekai_materials = [(1, 130), (21, 13), (10, 13)].into_iter().collect();
        let answered = answer_call(
            Call::Craft(UserMysekaiCraftRequest {
                blueprint_id: 4,
                texture_id: None,
                card_id: None,
                is_special_training: None,
                quantity: 3,
                mysekai_blueprint_term_id: None,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(reply.success, "{reply:?}");
                let rows = reply
                    .updated
                    .user_mysekai_fixtures
                    .expect("the fixtures are carried");
                assert_eq!(rows[0].quantity, 3);
            }
            other => panic!("not the craft's reply: {other:?}"),
        }
        let left = &model.doc.delivery.mysekai_materials;
        assert_eq!((left[&1], left[&21], left[&10]), (40, 4, 4));
        let crafted = [UserMysekaiFixture {
            mysekai_fixture_id: 4,
            texture_id: BASE_TEXTURE_ID,
            quantity: 3,
            last_obtained_at: model.now_ms(),
        }];
        let mut expected = fixture_model().doc.inventory.fixtures;
        expected.extend(crafted);
        assert_eq!(model.doc.inventory.fixtures, expected);
        assert_eq!(model.doc.gamedata.total_exp, 1000);
    }

    /// A craft of blueprint 4 in term 2 (birthday_anniversary, cost group 900,
    /// craftLimit 2) with the given quantity and term id.
    fn term_craft_request(quantity: i32, term: Option<i32>) -> Call {
        Call::Craft(UserMysekaiCraftRequest {
            blueprint_id: 4,
            texture_id: None,
            card_id: None,
            is_special_training: None,
            quantity,
            mysekai_blueprint_term_id: term,
        })
    }

    // The server resolves mysekaiBlueprintTermId to a row of the request's
    // blueprint (spec 4 item 1): an id of another blueprint is refused.
    #[test]
    fn a_craft_term_of_another_blueprint_is_refused() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.delivery.mysekai_materials = [(1, 100), (21, 10)].into_iter().collect();
        // Term 1 belongs to blueprint 844, not 4.
        let answered = answer_call(term_craft_request(1, Some(1)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(refusal.contains("not blueprint 4's"), "{refusal}");
            }
            other => panic!("not a craft reply: {other:?}"),
        }
        // An id that is no master term at all is refused too.
        let answered = answer_call(term_craft_request(1, Some(404)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(
                    refusal.contains("is not a master blueprint term"),
                    "{refusal}"
                );
            }
            other => panic!("not a craft reply: {other:?}"),
        }
    }

    // The term must hold the server clock (spec 4 items 1 and 7): term 3
    // ended long before the fixed clock, so a request naming it is refused.
    #[test]
    fn a_craft_of_an_expired_term_is_refused_at_the_server_clock() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.delivery.mysekai_materials = [(1, 100), (21, 10)].into_iter().collect();
        let answered = answer_call(term_craft_request(1, Some(3)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(refusal.contains("outside its session"), "{refusal}");
                assert!(refusal.contains("MSG_CRAFT_TERM_LIMIT"), "{refusal}");
            }
            other => panic!("not a craft reply: {other:?}"),
        }
    }

    // A term with a positive cost group charges that group's rows in place of
    // the blueprint's, and the lend counter gains the quantity and comes back
    // in the reply (spec 4 items 2 and 4).
    #[test]
    fn a_term_craft_charges_the_terms_cost_group_and_counts_the_craft() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        // Group 900: 5 x material 1 and 1 x material 21 per craft; the
        // blueprint's own rows (3 x material 1, 3 x 21, 3 x 10) are not
        // charged.
        model.doc.delivery.mysekai_materials = [(1, 100), (21, 10), (10, 10)].into_iter().collect();
        let answered = answer_call(term_craft_request(2, Some(2)), &mut model, &masters);
        match ok(answered.clone()) {
            Reply::SuiteUser(reply) => assert!(reply.success, "{reply:?}"),
            other => panic!("not the craft's reply: {other:?}"),
        }
        let left = &model.doc.delivery.mysekai_materials;
        assert_eq!((left[&1], left[&21], left[&10]), (90, 8, 10));
        assert_eq!(
            model.doc.inventory.blueprint_lend_craft_counts,
            vec![moly_protocol::rows::UserMysekaiBlueprintLendCraftCount {
                mysekai_blueprint_term_id: 2,
                craft_count: 2,
            }]
        );
        let sections = answered.sections.expect("the reply carries its sections");
        assert_eq!(
            sections.blueprint_lend_craft_counts,
            Some(vec![
                moly_protocol::rows::UserMysekaiBlueprintLendCraftCount {
                    mysekai_blueprint_term_id: 2,
                    craft_count: 2,
                }
            ]),
            "the term counter comes back with the reply"
        );
    }

    // craftLimit is a second, independent cap against the lend counter
    // (spec 4 items 2 and 3): term 2 allows 2 crafts.
    #[test]
    fn a_term_craft_past_its_craft_limit_is_refused() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.delivery.mysekai_materials = [(1, 100), (21, 10)].into_iter().collect();
        // Two crafts used already; the cap is 2, so the third is refused.
        model.doc.inventory.blueprint_lend_craft_counts =
            vec![moly_protocol::rows::UserMysekaiBlueprintLendCraftCount {
                mysekai_blueprint_term_id: 2,
                craft_count: 2,
            }];
        let answered = answer_call(term_craft_request(1, Some(2)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(refusal.contains("IsTermCraftCountLimit"), "{refusal}");
            }
            other => panic!("not a craft reply: {other:?}"),
        }
        // One craft left: a quantity of 2 is above the term-capped selector
        // maximum, a quantity of 1 goes through.
        model.doc.inventory.blueprint_lend_craft_counts =
            vec![moly_protocol::rows::UserMysekaiBlueprintLendCraftCount {
                mysekai_blueprint_term_id: 2,
                craft_count: 1,
            }];
        let answered = answer_call(term_craft_request(2, Some(2)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(refusal.contains("count selector's maximum 1"), "{refusal}");
            }
            other => panic!("not a craft reply: {other:?}"),
        }
        let answered = answer_call(term_craft_request(1, Some(2)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(reply.success, "{reply:?}");
                assert_eq!(
                    model.doc.inventory.blueprint_lend_craft_counts,
                    vec![moly_protocol::rows::UserMysekaiBlueprintLendCraftCount {
                        mysekai_blueprint_term_id: 2,
                        craft_count: 2,
                    }]
                );
            }
            other => panic!("not the craft's reply: {other:?}"),
        }
    }

    // A blueprint with a live term is craftable without possession (spec 4
    // item 5): with the owned rows policy `stated` and no owned row, a craft
    // of a term blueprint in session is accepted, while a blueprint without a
    // live term is still refused for not being owned.
    #[test]
    fn a_term_craft_needs_no_owned_blueprint() {
        let masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        craft_masters(&mut shared);
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.inventory.owned_blueprints = crate::model::inventory::OwnedBlueprints::Stated;
        model.doc.inventory.blueprints.clear();
        model.doc.delivery.mysekai_materials = [(1, 100), (21, 10), (10, 10)].into_iter().collect();
        // Blueprint 1 has no term row: not owned.
        let answered = answer_call(
            Call::Craft(UserMysekaiCraftRequest {
                blueprint_id: 1,
                texture_id: None,
                card_id: None,
                is_special_training: None,
                quantity: 1,
                mysekai_blueprint_term_id: None,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                let refusal = reply.refusal.unwrap_or_default();
                assert!(refusal.contains("is not owned"), "{refusal}");
            }
            other => panic!("not a craft reply: {other:?}"),
        }
        // Blueprint 4 has live term 2: accepted, its term counter written.
        let answered = answer_call(term_craft_request(1, Some(2)), &mut model, &masters);
        match ok(answered) {
            Reply::SuiteUser(reply) => assert!(reply.success, "{reply:?}"),
            other => panic!("not a craft reply: {other:?}"),
        }
    }

    #[test]
    fn the_sketch_refusal_keeps_its_api_name() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::HousingSketch(UserMysekaiHousingSketchRequest {
                mysekai_owner_user_id: 1,
                mysekai_site_id: 1,
                mysekai_blueprint_id: 1,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::SuiteUser(reply) => {
                assert!(!reply.success);
                assert!(reply
                    .refusal
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("PostUserMysekaiHousingSketchApi")));
            }
            other => panic!("not the sketch's reply: {other:?}"),
        }
    }

    #[test]
    fn the_housing_layout_save_stores_the_served_row() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let row = moly_protocol::api::HousingLayoutFixture {
            package: "mysekai__fixture__mdl_test".to_owned(),
            mysekai_fixture_id: 8,
            minimum: moly_protocol::api::HousingLayoutCorner { x: -1, y: 0, z: 0 },
            maximum: moly_protocol::api::HousingLayoutCorner { x: 0, y: 1, z: 1 },
            center_y: 0,
            layout_type: 2,
            rotation: 0,
            texture_id: 1,
        };
        let answered = answer_call(
            Call::HousingLayout(UserMysekaiHousingEditLayoutRequest {
                mysekai_site_id: 1,
                mysekai_phenomena_id: Some(2),
                site_type: "home_site".to_owned(),
                fixtures: vec![row],
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::SuiteUser(reply) => assert!(reply.success),
            other => panic!("not the layout's reply: {other:?}"),
        }
        let served = model
            .doc
            .layouts
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["siteType"] == "home_site"))
            .and_then(|row| row["mysekaiFixtures"].as_array())
            .expect("the served home-site row");
        assert_eq!(served.len(), 1);
        assert_eq!(served[0]["package"], "mysekai__fixture__mdl_test");
        assert_eq!(model.doc.layouts[0]["mysekaiPhenomenaId"], 2);
        let before = model.doc.layouts.clone();
        let refused = answer_call(
            Call::HousingLayout(UserMysekaiHousingEditLayoutRequest {
                mysekai_site_id: 1,
                site_type: "home_site".to_owned(),
                fixtures: vec![],
                mysekai_phenomena_id: Some(999),
            }),
            &mut model,
            &masters,
        );
        assert!(matches!(ok(refused), Reply::SuiteUser(reply) if !reply.success));
        assert_eq!(model.doc.layouts, before);
        let reset = answer_call(
            Call::HousingLayout(UserMysekaiHousingEditLayoutRequest {
                mysekai_site_id: 1,
                site_type: "home_site".to_owned(),
                fixtures: vec![],
                mysekai_phenomena_id: None,
            }),
            &mut model,
            &masters,
        );
        assert!(matches!(ok(reset), Reply::SuiteUser(reply) if reply.success));
        assert_eq!(model.doc.layouts[0]["mysekaiPhenomenaId"], 0);
    }

    #[test]
    fn the_talk_read_sets_the_row_and_replies_with_the_table() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::CharacterTalkRead(CharacterTalkReadRequest {
                mysekai_character_talk_id: 42,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::TalkRead(reply) => {
                assert!(reply.success);
                assert!(reply.obtained_resources.is_empty());
                assert!(reply
                    .updated
                    .user_mysekai_character_talks
                    .expect("the talks are carried")
                    .iter()
                    .any(|row| row.mysekai_character_talk_id == 42 && row.is_read));
            }
            other => panic!("not the talk read's reply: {other:?}"),
        }
    }

    #[test]
    fn the_music_play_set_and_eject_change_the_site_row() {
        let mut masters = fixture_masters();
        masters.music_records = Some(vec![12]);
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.doc.music_records = vec![OwnedMusicRecord {
            mysekai_music_record_id: 12,
            obtained_at: 7,
        }];
        let answered = answer_call(
            Call::MusicPlaySet(MusicPlaySetRequest {
                mysekai_site_id: 5,
                mysekai_music_record_id: 12,
                music_vocal_id: Some(3),
                is_instrumental: false,
                mysekai_fixture_id: 70,
                texture_id: 1,
            }),
            &mut model,
            &masters,
        );
        assert_eq!(
            ok(answered),
            Reply::MusicPlay(MusicPlayReply { success: true })
        );
        assert_eq!(
            model.doc.music_settings,
            vec![MusicPlaySetting {
                mysekai_site_id: 5,
                mysekai_music_record_id: 12,
                music_vocal_id: 3,
                is_instrumental: false,
            }]
        );
        let answered = answer_call(
            Call::MusicPlayEject(MusicPlayEjectRequest {
                mysekai_site_id: 5,
                mysekai_fixture_id: 70,
                texture_id: 1,
            }),
            &mut model,
            &masters,
        );
        assert_eq!(
            ok(answered),
            Reply::MusicPlay(MusicPlayReply { success: true })
        );
        assert!(model.doc.music_settings.is_empty());
    }

    #[test]
    fn the_system_fixture_action_records_the_row() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::SystemFixtureAction(SystemFixtureActionRequest {
                mysekai_system_fixture_id: 3,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::SystemFixtureAction(reply) => {
                assert!(reply.success);
                assert!(reply.obtained_resources.is_empty());
            }
            other => panic!("not the action's reply: {other:?}"),
        }
        assert!(model
            .doc
            .system_fixture_actions
            .rows
            .iter()
            .any(|row| row.mysekai_system_fixture_id == 3));
    }

    #[test]
    fn the_admin_views_and_edit_answer_with_json_text() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        match ok(answer_call(
            Call::Admin(AdminCall::Schema),
            &mut model,
            &masters,
        )) {
            Reply::Admin(text) => assert!(text.contains("\"sections\"")),
            other => panic!("not the schema: {other:?}"),
        }
        match ok(answer_call(
            Call::Admin(AdminCall::Document),
            &mut model,
            &masters,
        )) {
            Reply::Admin(text) => {
                assert!(text.contains("\"schemaVersion\":2"), "{text}");
                assert!(text.contains("\"serverNowMs\""));
            }
            other => panic!("not the document: {other:?}"),
        }
        let command = json!({
            "type": "server.edit",
            "path": "userMysekaiGamedata.totalExp",
            "value": 100,
        })
        .to_string();
        match ok(answer_call(
            Call::Admin(AdminCall::Edit {
                command_json: command,
            }),
            &mut model,
            &masters,
        )) {
            Reply::Admin(text) => assert!(text.contains("\"revision\"")),
            other => panic!("not the edit's receipt: {other:?}"),
        }
        assert_eq!(model.doc.gamedata.total_exp, 100);
    }

    #[test]
    fn a_live_edit_queues_a_sync_push_with_the_served_schedule() {
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let masters = Tables::new(&masters, Some(&shared));
        let mut model = joined_model(&masters);
        model.responses.clear();
        // The default document serves a daily schedule; the phenomenon of
        // the first refresh window is served live.
        let mut command = Map::new();
        command.insert("type".into(), json!("server.edit"));
        command.insert(
            "path".into(),
            json!("phenomenaSchedulePolicy.byRefreshTimePeriod.1"),
        );
        command.insert("value".into(), json!(2));
        model
            .edit(&command, &masters)
            .expect("the schedule policy edit applies");
        let push = model
            .responses
            .iter()
            .find(|response| response.kind == ResponseKind::Sync)
            .expect("the live edit queued a sync response");
        let rows = push
            .sections
            .schedules
            .as_ref()
            .expect("the sync carries the served schedule");
        assert_eq!(rows.len(), 6);
        assert!(rows
            .iter()
            .any(|row| row.refresh_time_period_id == 1 && row.phenomena_id == 2));
    }

    /// One administration call over a session, with the given listen address,
    /// answered by the dispatch system.
    fn admin_over(listen: Option<std::net::SocketAddr>) -> Result<Reply, Refusal> {
        use crate::hub::{CallReceived, HubEvent, SessionId};
        use crate::users::UserEntry;
        use crate::ListenAddress;

        let session = SessionId(7);
        let (out, replies) = async_channel::unbounded::<Vec<u8>>();
        let masters = fixture_masters();
        let shared = MasterTables::fixture();
        let tables = Tables::new(&masters, Some(&shared));
        let model = joined_model(&tables);
        let mut users = Users::default();
        users.0.insert(
            UserId::local(),
            UserEntry {
                model: Some(model),
                load_error: None,
            },
        );
        let mut app = App::new();
        app.insert_resource(users)
            .insert_resource(masters)
            .add_plugins(MinimalPlugins)
            .add_plugins(crate::ServerPlugin);
        if let Some(listen) = listen {
            app.insert_resource(ListenAddress(listen));
        }
        let hub = crate::hub(&app);
        hub.send(HubEvent::Open { session, out }).unwrap();
        app.update();
        app.world_mut().write_message(CallReceived {
            session,
            user: UserId::local(),
            seq: 5,
            call: Call::Admin(AdminCall::Schema),
        });
        app.update();
        // The first frame that is not the end of an update.
        let frame = std::iter::from_fn(|| replies.try_recv().ok())
            .find(|frame| !frame.is_empty())
            .expect("the server answered");
        let ServerMessage::Reply { outcome, .. } =
            moly_protocol::decode(&frame).expect("a protocol frame")
        else {
            panic!("not a reply");
        };
        outcome
    }

    #[test]
    fn the_admin_calls_are_refused_on_a_non_loopback_address() {
        match admin_over(Some("192.0.2.1:15800".parse().unwrap())) {
            Err(refusal) => {
                assert_eq!(refusal.code, "admin_not_allowed");
                assert!(
                    refusal.reason.contains("192.0.2.1:15800"),
                    "{}",
                    refusal.reason
                );
            }
            other => panic!("not a refusal: {other:?}"),
        }
        assert!(
            admin_over(None).is_err(),
            "a server that does not report its address accepts none"
        );
        match admin_over(Some("127.0.0.1:15800".parse().unwrap())) {
            Ok(Reply::Admin(text)) => assert!(text.contains("\"sections\"")),
            other => panic!("not the loopback's answer: {other:?}"),
        }
    }

    /// Two sheets: sheet 1 (seq 1) with completion mission 100 (two normal
    /// missions), missions 101 and 102; sheet 2 (seq 2) with completion
    /// mission 200, mission 201. Mission 100 rewards box 900 (material 7 x2
    /// and item 5 x1) and rank experience 1 (7000); mission 101 rewards the
    /// material possession level 2; mission 102 has no reward row.
    fn mission_masters() -> moly_masters::mission::MissionMasters {
        use moly_masters::mission::{
            Mission, MissionCategory, MissionMasters, MissionReward, RewardType,
        };
        let mission =
            |id: i32, sheet: i32, seq: i32, category: MissionCategory, requirement1: i32| Mission {
                id,
                sheet_id: sheet,
                seq,
                sentence: format!("mission {id}"),
                category,
                requirement1,
            };
        let reward = |id: i32, mission: i32, seq: i32, reward_type: RewardType, external: i32| {
            MissionReward {
                id,
                mission_id: mission,
                seq,
                reward_type,
                external_id: external,
            }
        };
        MissionMasters {
            sheets: vec![
                moly_masters::mission::MissionSheet {
                    id: 2,
                    seq: 2,
                    name: "second".to_owned(),
                },
                moly_masters::mission::MissionSheet {
                    id: 1,
                    seq: 1,
                    name: "first".to_owned(),
                },
            ],
            missions: vec![
                mission(100, 1, 1, MissionCategory::Complete, 2),
                mission(101, 1, 2, MissionCategory::Normal, 10),
                mission(102, 1, 3, MissionCategory::Normal, 3),
                mission(200, 2, 1, MissionCategory::Complete, 1),
                mission(201, 2, 2, MissionCategory::Normal, 1),
            ],
            rewards: vec![
                reward(1, 100, 1, RewardType::ResourceBox, 900),
                reward(2, 100, 2, RewardType::RankObtainedExp, 1),
                reward(3, 101, 1, RewardType::MaterialPossession, 2),
            ],
            rank_exps: vec![moly_masters::mission::RankExpRow {
                id: 1,
                quantity: 7000,
            }],
        }
    }

    /// The mission masters seated beside the shared fixture's other tables,
    /// with the mission box 900: a `mysekai_material` 7 x2 and a
    /// `mysekai_item` 5 x1. The rank table is the fixture's (rank 2 at
    /// 48,000, rank 3 at 172,300), and the possession masters hold the
    /// levels 1 (limit 1000) and 2 (limit 2000).
    fn mission_tables(masters: &mut Masters, shared: &mut MasterTables) {
        let missions = mission_masters();
        shared.mission = Some(missions);
        let limit = moly_masters::inventory::PossessionRow {
            level: 1,
            possession_limit: 1000,
        };
        let raised = moly_masters::inventory::PossessionRow {
            level: 2,
            possession_limit: 2000,
        };
        shared.possession = Some(moly_masters::inventory::PossessionMasters {
            fixture: Some(vec![limit, raised]),
            material: Some(vec![limit, raised]),
        });
        let mut convert = masters
            .convert
            .clone()
            .expect("the fixture's convert tables");
        convert.resource_boxes = Some(vec![crate::model::convert::ResourceBox {
            purpose: "mysekai_normal_mission_reward".to_owned(),
            id: 900,
            details: vec![
                ("mysekai_material".to_owned(), 7, 0, 2),
                ("mysekai_item".to_owned(), 5, 0, 1),
            ],
        }]);
        masters.convert = Some(convert);
    }

    /// `userMysekaiNormalMissions` of the test document: sheet 1, its
    /// completion mission at its two normal missions (progress 2, achieved),
    /// mission 101 (progress 10, achieved) and mission 102 (not achieved).
    fn mission_rows() -> Vec<UserMysekaiNormalMission> {
        vec![
            UserMysekaiNormalMission {
                mysekai_normal_mission_id: 100,
                progress: 2,
                mysekai_mission_status: MISSION_ACHIEVED.to_owned(),
            },
            UserMysekaiNormalMission {
                mysekai_normal_mission_id: 101,
                progress: 10,
                mysekai_mission_status: MISSION_ACHIEVED.to_owned(),
            },
            UserMysekaiNormalMission {
                mysekai_normal_mission_id: 102,
                progress: 0,
                mysekai_mission_status: MISSION_NOT_ACHIEVED.to_owned(),
            },
        ]
    }

    fn mission_model() -> ServerModel {
        let mut model = fixture_model();
        model.doc.normal_mission =
            crate::model::normal_mission::NormalMissionDoc::new(1, mission_rows());
        model
    }

    /// The mission model joined on the given tables; the session's own seating
    /// is undone so each test states the sheet and rows it reads.
    fn joined_mission_model(tables: &Tables<'_>) -> ServerModel {
        let mut model = mission_model();
        model.join(tables);
        model.doc.normal_mission =
            crate::model::normal_mission::NormalMissionDoc::new(1, mission_rows());
        model.doc.gamedata.total_exp = 0;
        model.doc.gamedata.mysekai_rank = Some(1);
        model.responses.clear();
        model
    }

    /// One receive of an achieved mission: the row reads received, the
    /// material reward is granted and reported, the rank experience moves
    /// totalExp by the rank-exp row's quantity and the rank follows it.
    /// The cell that sends such a call is a normal mission's
    /// (`ScreenLayerMysekaiMission.OnClickMysekaiMissionCell` sends the
    /// completion mission's id through the bulk list only), so the call is
    /// mission 101's.
    #[test]
    fn a_normal_mission_receive_grants_its_reward_and_experience() {
        let mut masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        mission_tables(&mut masters, &mut shared);
        let tables = Tables::new(&masters, Some(&shared));
        let mut model = joined_mission_model(&tables);
        model.doc.delivery.mysekai_materials.clear();
        let answered = answer_call(
            Call::NormalMissionReward(moly_protocol::api::UserMysekaiNormalMissionRewardRequest {
                mysekai_normal_mission_ids: vec![101],
            }),
            &mut model,
            &tables,
        );
        let reply = match ok(answered.clone()) {
            Reply::NormalMissionReward(reply) => reply,
            other => panic!("answered {other:?}"),
        };
        // Mission 101 rewards the material possession level 2 and nothing
        // else: no resource, no rank experience.
        assert!(reply.obtained_rewards.is_empty());
        assert_eq!(model.doc.inventory.material_possession_level, 2);
        assert_eq!(model.doc.gamedata.total_exp, 0);
        assert_eq!(model.doc.gamedata.mysekai_rank, Some(1));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&7), None);
        let rows = reply
            .updated_resources
            .user_mysekai_normal_missions
            .as_ref()
            .expect("the rows");
        assert!(rows.iter().any(|row| row.mysekai_normal_mission_id == 101
            && row.mysekai_mission_status == MISSION_RECEIVED));
        // The completion row is not named by the call: its count is 1
        // (mission 101), below its requirement1 2, so the completion step
        // reads it not_achieved; its stored progress 2 is above the count
        // and stays.
        assert!(rows.iter().any(|row| row.mysekai_normal_mission_id == 100
            && row.mysekai_mission_status == MISSION_NOT_ACHIEVED
            && row.progress == 2));
        // The sheet does not advance: mission 102 is not received.
        assert_eq!(
            reply
                .updated_resources
                .user_mysekai_normal_mission_sheet
                .expect("the sheet")
                .mysekai_normal_mission_sheet_id,
            1
        );
        assert_eq!(model.doc.normal_mission.sheet_id, 1);
    }

    /// A receive of a mission that is not achieved is refused whole: the
    /// rows, the experience and the materials do not change.
    #[test]
    fn a_normal_mission_receive_of_a_not_achieved_mission_is_refused() {
        let mut masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        mission_tables(&mut masters, &mut shared);
        let tables = Tables::new(&masters, Some(&shared));
        let mut model = joined_mission_model(&tables);
        let before = model.doc.normal_mission.clone();
        let answered = answer_call(
            Call::NormalMissionReward(moly_protocol::api::UserMysekaiNormalMissionRewardRequest {
                // 100 is achieved, 102 is not: one bad id refuses both.
                mysekai_normal_mission_ids: vec![100, 102],
            }),
            &mut model,
            &tables,
        );
        match answered.outcome {
            Err(refusal) => {
                assert_eq!(refusal.code, "normal_mission_refused");
                assert!(refusal.reason.contains("102"), "{}", refusal.reason);
            }
            other => panic!("not a refusal: {other:?}"),
        }
        assert_eq!(model.doc.normal_mission, before);
        assert_eq!(model.doc.gamedata.total_exp, 0);
        assert_eq!(model.doc.delivery.mysekai_materials.get(&7), None);
        assert!(model.responses.is_empty());
    }

    /// A bulk receive takes the achieved missions in one call, keeps the
    /// completion mission's progress at the counted normal missions, and
    /// advances to the next sheet once the sheet is complete.
    #[test]
    fn a_bulk_receive_advances_the_sheet() {
        let mut masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        mission_tables(&mut masters, &mut shared);
        let tables = Tables::new(&masters, Some(&shared));
        let mut model = joined_mission_model(&tables);
        model.doc.delivery.mysekai_materials.clear();
        // 101 and 102 are the sheet's normal missions; 100 the completion
        // one. The bulk receive takes the whole receivable set: 102 must
        // read achieved, as the panel made it before the button.
        for row in &mut model.doc.normal_mission.rows {
            if row.mysekai_normal_mission_id == 102 {
                row.progress = 3;
                row.mysekai_mission_status = MISSION_ACHIEVED.to_owned();
            }
        }
        let answered = answer_call(
            Call::NormalMissionReward(moly_protocol::api::UserMysekaiNormalMissionRewardRequest {
                mysekai_normal_mission_ids: vec![101, 102, 100],
            }),
            &mut model,
            &tables,
        );
        let reply = match ok(answered) {
            Reply::NormalMissionReward(reply) => reply,
            other => panic!("answered {other:?}"),
        };
        // Box 900 once (two details), the rank experience 7000.
        assert_eq!(reply.obtained_rewards.len(), 2);
        assert_eq!(model.doc.gamedata.total_exp, 7000);
        // Mission 102 has no reward row: the box's material is granted once.
        assert_eq!(model.doc.delivery.mysekai_materials.get(&7), Some(&2));
        // Mission 101 raises the material possession level to 2; the
        // possession is not carried (no materials master in the fixture),
        // but the gamedata the reply carries states the level.
        assert_eq!(model.doc.inventory.material_possession_level, 2);
        assert_eq!(
            reply
                .updated_resources
                .user_mysekai_gamedata
                .expect("the gamedata")
                .mysekai_material_possession_level,
            2
        );
        assert_eq!(reply.obtained_rewards[1].resource_type, "mysekai_item");
        assert_eq!(model.doc.inventory.items[0].mysekai_item_id, 5);
        assert_eq!(model.doc.inventory.items[0].quantity, 1);
        // The completion step (`MysekaiMissionUtility.CreateRecommendViewData`)
        // runs after the receive marks its rows, and the completion mission
        // was named: its row still reads received, at the count (2).
        let rows = &model.doc.normal_mission.rows;
        let parent = rows
            .iter()
            .find(|row| row.mysekai_normal_mission_id == 100)
            .expect("the completion mission's row");
        assert_eq!(parent.progress, 2);
        assert_eq!(parent.mysekai_mission_status, MISSION_RECEIVED);
        // The sheet advanced to sheet 2, whose missions were seated.
        assert_eq!(model.doc.normal_mission.sheet_id, 2);
        assert!(rows.iter().any(|row| row.mysekai_normal_mission_id == 200
            && row.mysekai_mission_status == MISSION_NOT_ACHIEVED));
        assert!(rows.iter().any(|row| row.mysekai_normal_mission_id == 201
            && row.mysekai_mission_status == MISSION_NOT_ACHIEVED));
        assert_eq!(
            reply
                .updated_resources
                .user_mysekai_normal_mission_sheet
                .expect("the sheet")
                .mysekai_normal_mission_sheet_id,
            2
        );
    }

    /// The experience moves the rank exactly at the master table's threshold:
    /// 7000 keeps rank 1, another 41,000 reaches rank 2 (48,000). Only the
    /// completion mission rewards rank experience, so the call names it (as
    /// the bulk list does).
    #[test]
    fn the_normal_mission_experience_recomputes_the_rank() {
        let mut masters = fixture_masters();
        let mut shared = MasterTables::fixture();
        mission_tables(&mut masters, &mut shared);
        let tables = Tables::new(&masters, Some(&shared));
        let mut model = joined_mission_model(&tables);
        model.doc.gamedata.total_exp = 41_000;
        model.doc.gamedata.mysekai_rank = Some(1);
        let answered = answer_call(
            Call::NormalMissionReward(moly_protocol::api::UserMysekaiNormalMissionRewardRequest {
                mysekai_normal_mission_ids: vec![100],
            }),
            &mut model,
            &tables,
        );
        let reply = match ok(answered) {
            Reply::NormalMissionReward(reply) => reply,
            other => panic!("answered {other:?}"),
        };
        assert_eq!(model.doc.gamedata.total_exp, 48_000);
        assert_eq!(model.doc.gamedata.mysekai_rank, Some(2));
        assert_eq!(
            reply
                .updated_resources
                .user_mysekai_gamedata
                .expect("the gamedata")
                .mysekai_rank,
            Some(2)
        );
        // The completion step runs after the receive's marks: the row the
        // call named still reads received.
        let rows = &model.doc.normal_mission.rows;
        assert!(rows.iter().any(|row| row.mysekai_normal_mission_id == 100
            && row.mysekai_mission_status == MISSION_RECEIVED
            && row.progress == 2));
    }
}
