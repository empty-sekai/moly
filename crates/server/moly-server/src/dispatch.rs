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
use crate::masters::Masters;
use crate::model::{ResponseKind, ServerModel};
use crate::users::Users;

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

/// The dispatch system.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_calls(
    mut calls: MessageReader<CallReceived>,
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    sessions: Res<Sessions>,
    defaults: Option<Res<Defaults>>,
) {
    for call in calls.read() {
        let session = call.session;
        let user = call.user.clone();
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
                &masters,
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
                        answer_call(call.call.clone(), model, &masters)
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
pub(crate) fn answer_call(call: Call, model: &mut ServerModel, masters: &Masters) -> Answered {
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

fn run(call: Call, model: &mut ServerModel, masters: &Masters) -> Result<Reply, Refusal> {
    match call {
        Call::Join => {
            // The join seats the user and answers with every section; a
            // repeated join answers again, as the source's login does.
            model.join(masters);
            Ok(Reply::Joined)
        }
        Call::Harvest(request) => Ok(Reply::Harvest(model.harvest(
            [request.normal, request.enhance, request.boost],
            masters,
        ))),
        Call::Gather => Ok(Reply::Gather(model.gather(masters))),
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
            let reply = model.gate_replies.pop().ok_or_else(|| {
                Refusal::new("gate_refused", "the gate change recorded no reply")
            })?;
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
            model.delivery_reply(
                ResponseKind::BirthdayPartyDelivery,
                &changed,
                masters,
            );
            Ok(Reply::Delivery(response))
        }
        Call::BirthdayPartyGather(request) => {
            let contents: &[GatherContent] = &request.contents;
            let (response, changed) = model
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
            let (resources, changed) = model
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
        Call::Admin(call) => admin::answer(call, model, masters).map(Reply::Admin),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::craft::{
        parse_blueprints, parse_costs, parse_rank_obtained_exps, parse_terms, BASE_TEXTURE_ID,
    };
    use crate::model::document::{self, Clock, Migration};
    use crate::model::{inventory, Origin, SECTION_STAMINA};
    use moly_protocol::api::{
        BirthdayPartyDeliveryRequest, BirthdayPartyGatherRequest, CharacterTalkReadRequest,
        GateChangeRequest, GateReserveRequest, HarvestRequest, HomeActionPost, MusicPlayEjectRequest,
        MusicPlayReply, MusicPlaySetRequest, SystemFixtureActionRequest, UserMysekaiCraftRequest,
        UserMysekaiHousingEditLayoutRequest, UserMysekaiHousingSketchRequest,
    };
    use moly_protocol::gate::GateReply;
    use moly_protocol::rows::{MusicPlaySetting, OwnedMusicRecord, Stamina, UserMysekaiFixture};
    use moly_protocol::AdminCall;

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

    fn joined_model(masters: &Masters) -> ServerModel {
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

    #[test]
    fn the_join_seats_the_user_and_carries_every_section() {
        let masters = fixture_masters();
        let mut model = fixture_model();
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
        assert_eq!(model.doc.gamedata.refreshed_at, 1_790_300_000_000);
    }

    #[test]
    fn the_harvest_reply_is_the_sent_rests_unless_an_edit_is_pending() {
        let masters = fixture_masters();
        let mut model = joined_model(&masters);
        let sent = Stamina {
            normal: 7,
            enhance: 2,
            boost: 1,
        };
        let answered = answer_call(
            Call::Harvest(HarvestRequest {
                normal: sent.normal,
                enhance: sent.enhance,
                boost: sent.boost,
            }),
            &mut model,
            &masters,
        );
        assert_eq!(ok(answered.clone()), Reply::Harvest(sent));
        assert_eq!(
            answered.sections.expect("the stamina is carried").stamina,
            Some(sent)
        );
        assert_eq!(model.doc.stamina, Some(sent));

        // A pending stamina edit is the reply, not the sent rests.
        let edited = Stamina {
            normal: 500,
            enhance: 0,
            boost: 0,
        };
        model.doc.stamina = Some(edited);
        model.mark_pending(SECTION_STAMINA);
        let answered = answer_call(
            Call::Harvest(HarvestRequest {
                normal: 9,
                enhance: 9,
                boost: 9,
            }),
            &mut model,
            &masters,
        );
        assert_eq!(ok(answered), Reply::Harvest(edited));
    }

    #[test]
    fn the_gather_reply_carries_the_pools() {
        let masters = fixture_masters();
        let mut model = joined_model(&masters);
        let answered = answer_call(Call::Gather, &mut model, &masters);
        assert_eq!(ok(answered.clone()), Reply::Gather(model.doc.stamina));
        assert_eq!(
            answered.sections.expect("the stamina is carried").stamina,
            model.doc.stamina
        );
    }

    #[test]
    fn the_gate_reserve_and_change_answer_with_the_gate_rows() {
        let masters = fixture_masters();
        let mut model = joined_model(&masters);
        let answered = answer_call(
            Call::GateReserve(GateReserveRequest {
                mysekai_game_character_unit_group_id: 4,
            }),
            &mut model,
            &masters,
        );
        match ok(answered) {
            Reply::Gate(GateReply::Reserve { row, .. }) => {
                assert_eq!(row.unit_group_id, 4);
                assert!(row.is_reservation);
                assert_eq!(row.visit_count, 1);
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
    fn craft_masters(mut masters: Masters) -> Masters {
        let upstream = |rows: serde_json::Value| rows.to_string();
        parse_blueprints(
            &upstream(json!([
                {"id": 1, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 1, "isEnableSketch": true, "isObtainedByConvert": true, "craftCountLimit": 1, "isAvailableWithoutPossession": false},
                {"id": 4, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 4, "isEnableSketch": true, "isObtainedByConvert": true, "isAvailableWithoutPossession": false},
            ])),
            &mut masters,
        )
        .unwrap();
        parse_costs(
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
            &mut masters,
        )
        .unwrap();
        parse_terms(
            &upstream(json!([
                {"id": 1, "mysekaiBlueprintId": 844, "startAt": 1_759_330_800_000_i64, "endAt": 1_759_849_199_000_i64},
            ])),
            &mut masters,
        )
        .unwrap();
        parse_rank_obtained_exps(
            &upstream(json!([
                {"id": 1, "mysekaiRankObtainedExpType": "craft_mysekai_fixture_first_bonus", "quantity": 1000},
            ])),
            &mut masters,
        )
        .unwrap();
        inventory::parse_fixture_possessions(
            &upstream(json!([
                {"id": 1, "level": 1, "possessionLimit": 1000},
            ])),
            &mut masters,
        )
        .unwrap();
        masters
    }

    #[test]
    fn the_craft_spends_its_cost_and_earns_the_first_craft_bonus() {
        let masters = craft_masters(fixture_masters());
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
        assert_eq!(
            model.doc.inventory.fixtures,
            vec![UserMysekaiFixture {
                mysekai_fixture_id: 4,
                texture_id: BASE_TEXTURE_ID,
                quantity: 3,
                last_obtained_at: model.now_ms(),
            }]
        );
        assert_eq!(model.doc.gamedata.total_exp, 1000);
    }

    #[test]
    fn the_sketch_refusal_keeps_its_api_name() {
        let masters = fixture_masters();
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
    }

    #[test]
    fn the_talk_read_sets_the_row_and_replies_with_the_table() {
        let masters = fixture_masters();
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
}
