//! `MysekaiMissionService`: the mission screen's one receive request.
//!
//! The service holds one request (the mission ids, and the sheet the screen
//! named) and sends it as `PostUserMysekaiNormalMissionRewardApi`
//! (`Call::NormalMissionReward`). `MysekaiMissionService.Receive(sheetId,
//! missionId, callback)` is a cell's receive and `.BulkReceive(sheetId,
//! missionIds, callback)` the bulk button's; both write the one request and
//! both are sent by `MysekaiMissionService.ExecuteRewardApi` through
//! `APIExecutor.Execute(caller, null, null, RetryCancel, Stay)`.
//!
//! The answer follows `MysekaiMissionService`'s own handlers: the success
//! callback runs only when the answer is the API's success state and its
//! reply carries `updatedResources` and `obtainedRewards`
//! (`MysekaiMissionService.cs`'s `(ApiStatus.SuccessComplete, response)`
//! branch); otherwise the failure callback runs with no response, and its
//! `APIExecutor`'s RetryCancel shows the retry-or-cancel dialog, which this
//! tree does not build (named in the log). In either case the request is
//! released, so a second receive can be sent.
//!
//! One request is in flight at a time: the source's `Receive` and
//! `BulkReceive` each overwrite the service's one request, so a second call
//! before the first answer would replace it; here the second call is
//! refused by name ([`MysekaiMissionService::send`]).
//!
//! Named refusals:
//! - The tutorial API variant (`PostUserMysekaiTutorialNormalMissionRewardApi`,
//!   sent while `MysekaiTutorialManager.IsTutorial`) is refused by name: this
//!   tree has no tutorial domain.
//! - `IsCompleteSheet` (`MysekaiMissionService.cs`) is the model's
//!   ([`crate::mission::model::ReceiveRequest::is_complete_sheet`]) and is
//!   read from the request kept while it is in flight.
//! - The reward chain (`ShowRewards`) is not built here (a later unit): a
//!   success is published as [`MissionReceived`] and the screen logs the
//!   chain's call.

use bevy::prelude::*;
use moly_link::ServerLink;
use moly_protocol::api::{UserMysekaiNormalMissionRewardRequest, UserMysekaiNormalMissionRewardResponse};
use moly_protocol::{Call, CallKind, Refusal, Reply};

use super::model::ReceiveRequest;
use crate::userdata::mission::ClientMission;
use crate::userdata::ServerPending;

/// The call the receive sends.
const CALL: CallKind = CallKind::NormalMissionReward;

/// What the screen is waiting for: the request it sent and the call's
/// sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionRequest {
    /// `_missionSheetId` and `mysekaiNormalMissionIds` as the screen asked.
    pub request: ReceiveRequest,
    pub sent: ServerPending,
}

impl MissionRequest {
    /// The request as the API's body (`UserMysekaiNormalMissionRewardRequest`).
    pub fn body(&self) -> UserMysekaiNormalMissionRewardRequest {
        UserMysekaiNormalMissionRewardRequest {
            mysekai_normal_mission_ids: self.request.mission_ids.clone(),
        }
    }
}

/// The request and its answer, kept in the order they happened.
#[derive(Resource, Debug, Default)]
pub struct MysekaiMissionService {
    in_flight: Option<MissionRequest>,
    /// The last request, whether it succeeded or not (`_missionSheetId`
    /// survives an answer in the source).
    last: Option<MissionRequest>,
    /// Why the last call was refused before it was sent.
    refused: Option<String>,
}

impl MysekaiMissionService {
    /// The request in flight.
    pub fn in_flight(&self) -> Option<&MissionRequest> {
        self.in_flight.as_ref()
    }

    /// The newest request (the one in flight, or the last answered).
    pub fn last(&self) -> Option<&MissionRequest> {
        self.in_flight.as_ref().or(self.last.as_ref())
    }

    /// The server call this service waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub fn server_pending(&self) -> Option<ServerPending> {
        self.in_flight.as_ref().map(|in_flight| in_flight.sent)
    }

    /// Why the last call was not sent.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// `MysekaiMissionService.Receive`: one request (the cell's or the bulk
    /// button's) sent on the link. `Err` names the request that was not sent:
    /// a tutorial session (the tutorial variant is refused), a request with
    /// no mission id, or a request while another is answered.
    pub fn send(
        &mut self,
        link: &mut ServerLink,
        sheet_id: i32,
        mission_ids: Vec<i32>,
        tutorial: bool,
        caller: &str,
    ) -> Result<u32, String> {
        if tutorial {
            let reason = format!(
                "{caller}: a receive in the tutorial sends PostUserMysekaiTutorialNormalMissionRewardApi, which this tree does not have (no tutorial domain); nothing is sent"
            );
            self.refused = Some(reason.clone());
            return Err(reason);
        }
        if mission_ids.is_empty() {
            let reason = format!("{caller}: the request names no mission; nothing is sent");
            self.refused = Some(reason.clone());
            return Err(reason);
        }
        if let Some(in_flight) = &self.in_flight {
            let reason = format!(
                "{caller}: the receive of missions {:?} (seq {:?}) is still unanswered; the source's service holds one request, so this one is not sent",
                in_flight.request.mission_ids, in_flight.sent.seq
            );
            self.refused = Some(reason.clone());
            return Err(reason);
        }
        self.refused = None;
        let request = ReceiveRequest {
            sheet_id,
            mission_ids,
        };
        let call = Call::NormalMissionReward(UserMysekaiNormalMissionRewardRequest {
            mysekai_normal_mission_ids: request.mission_ids.clone(),
        });
        let requested = request.mission_ids.clone();
        let seq = link.call(call);
        self.in_flight = Some(MissionRequest {
            request,
            sent: ServerPending {
                seq: Some(seq),
                call: CALL,
            },
        });
        info!(
            "{caller}: PostUserMysekaiNormalMissionRewardApi sent as seq {seq} (sheet {sheet_id}, missions {requested:?}); one request in flight until its answer"
        );
        Ok(seq)
    }

    /// Takes the answer of this service's call: the request it answers (and
    /// its release), or `None` for another call's answer (which then stays
    /// in flight).
    pub fn answer(&mut self, seq: u32) -> Option<MissionRequest> {
        let in_flight = self.in_flight.take()?;
        if in_flight.sent.seq != Some(seq) {
            self.in_flight = Some(in_flight);
            return None;
        }
        self.last = Some(in_flight.clone());
        Some(in_flight)
    }

    /// Takes the request in flight whatever answer arrives: the lost-session
    /// answer for a number the link no longer waits on (another send took
    /// the request's answer away), so the request finishes on the failure
    /// path instead of waiting forever.
    pub fn take_unmatched(&mut self) -> Option<MissionRequest> {
        let in_flight = self.in_flight.take()?;
        self.last = Some(in_flight.clone());
        Some(in_flight)
    }
}

/// The refusal code the link answers a call with when its session is lost
/// (`moly_link`): no receive is sent while the link is lost.
const LINK_LOST: &str = "link_lost";

/// The success of a receive as the screen's onFinish receives it: the
/// answer's `updatedResources` and `obtainedRewards` (`MysekaiMissionService`'s
/// success branch), and whether the reply raised a level or the rank,
/// compared with the copies before it.
#[derive(Message, Debug, Clone)]
pub struct MissionReceived {
    pub sheet_id: i32,
    pub mission_ids: Vec<i32>,
    /// `response.obtainedRewards`.
    pub obtained_rewards: Vec<moly_protocol::rows::UserResource>,
    /// `IsTracdeMachineLevelUp`: `mysekaiConvertFixtureLevel` went up.
    pub convert_level_up: bool,
    /// `IsMaterialPossessionLevelUp`.
    pub material_possession_level_up: bool,
    /// `IsFixturePossessionLevelUp`.
    pub fixture_possession_level_up: bool,
    /// `IsSekaiRankUp`: `mysekaiRank` went up.
    pub rank_up: bool,
}

/// The failure of a receive, as the failure callback receives it: the
/// refusal the link answered, or the name of another answer than the API's
/// success state.
#[derive(Message, Debug, Clone)]
pub struct MissionReceiveFailed {
    pub sheet_id: i32,
    pub mission_ids: Vec<i32>,
    /// The refusal; `Ok(other reply)` names an answer that is not the
    /// receive's.
    pub refusal: Result<Refusal, String>,
}

/// The copies before the answer's sections were applied, to compare the
/// levels the reply raised.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MissionLevelsBefore {
    pub gamedata: Option<moly_protocol::rows::Gamedata>,
    pub inventory: Option<moly_protocol::rows::UserMysekaiGamedata>,
}

fn is_level_up(before: Option<i32>, after: Option<i32>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => after > before,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// PreUpdate, right after the reader applied the arrival's sections: the
/// copy of the levels the reply's own sections would change.
pub fn remember_levels(
    client: Option<Res<crate::userdata::ClientUserData>>,
    inventory: Option<Res<crate::userdata::inventory::ClientMysekaiInventory>>,
    mut before: ResMut<MissionLevelsBefore>,
) {
    *before = MissionLevelsBefore {
        gamedata: client.as_deref().map(|client| client.gamedata),
        inventory: inventory.as_deref().map(|inventory| inventory.gamedata()).flatten(),
    };
}

/// The receive's own call: the one call kind this service answers.
pub const RECEIVE: CallKind = CALL;

/// Update, after the reader: a receive's answer runs its callback. The
/// reply's `updatedResources` tables are applied to the copies here, after
/// the arrival's own sections, so the reply's tables win.
///
/// A link that cannot send the call answers it with a `link_lost` refusal,
/// and the features read that answer against the calls they remember
/// ([`crate::userdata::SentCalls`]): the last receive of that kind is the
/// one this service holds, so a `link_lost` answer of the receive's kind,
/// which does not match the request in flight (another send took its
/// number), still finishes the request rather than leaving it unanswered.
#[allow(clippy::too_many_arguments)]
pub fn answers(
    mut answers: MessageReader<crate::userdata::Answered>,
    mut service: ResMut<MysekaiMissionService>,
    sent: Res<crate::userdata::SentCalls>,
    before: Res<MissionLevelsBefore>,
    client: Option<Res<crate::userdata::ClientUserData>>,
    inventory: Option<Res<crate::userdata::inventory::ClientMysekaiInventory>>,
    mut mission: Option<ResMut<ClientMission>>,
    mut received: MessageWriter<MissionReceived>,
    mut failed: MessageWriter<MissionReceiveFailed>,
) {
    for answer in answers.read() {
        // Not this service's call: its own handler reads it.
        if answer.call != CALL {
            continue;
        }
        // A lost-session answer for a receive that is not the request in
        // flight (another send took its number): the request finishes on
        // the failure path rather than waiting for an answer that never
        // comes. Every other mismatched answer leaves the request waiting.
        let lost_for_this_request = matches!(&answer.outcome, Err(refusal) if refusal.code == LINK_LOST)
            && matches!(sent.0.get(&answer.seq), Some(Call::NormalMissionReward(_)));
        let request = if service.in_flight().is_some_and(|in_flight| in_flight.sent.seq == Some(answer.seq)) {
            service.answer(answer.seq)
        } else if lost_for_this_request {
            service.take_unmatched()
        } else {
            None
        };
        let Some(request) = request else {
            // Not this request's answer, or no request waits: nothing to
            // finish, and the answer is not swallowed from its own sender
            // (the receive is the only sender of this kind).
            continue;
        };
        let sheet_id = request.request.sheet_id;
        let mission_ids = request.request.mission_ids.clone();
        // The success branch: `ApiStatus.SuccessComplete` and both reply
        // fields.
        let reply: Option<&UserMysekaiNormalMissionRewardResponse> = match &answer.outcome {
            Ok(Reply::NormalMissionReward(reply)) => Some(reply),
            _ => None,
        };
        let Some(reply) = reply else {
            let refusal = match &answer.outcome {
                Err(refusal) => Ok(refusal.clone()),
                Ok(other) => Err(format!("the answer is {other:?}, not the receive's reply")),
            };
            error!(
                "PostUserMysekaiNormalMissionRewardApi (seq {:?}, missions {mission_ids:?}): not the success state; the failure callback runs ({refusal:?}); APIExecutor's RetryCancel dialog is not built, so no receive is repeated",
                answer.seq
            );
            failed.write(MissionReceiveFailed {
                sheet_id,
                mission_ids,
                refusal,
            });
            continue;
        };
        // `UserDataManager.UpdateAll(updatedResources)` runs after the
        // reply's own sections: the reply's mission tables win over the
        // arrival's.
        if let Some(copy) = mission.as_mut() {
            copy.apply_reply(reply);
        }
        // The reply's `updatedResources.userMysekaiGamedata` is the user-wide
        // row; the reader has seated it in the client copy already. Its
        // fields are the rank, the experience and the convert machine's
        // level (the possession levels are the inventory copy's own).
        let gamedata_after = client.as_deref().map(|client| client.gamedata);
        let inventory_after = inventory.as_deref().and_then(|inventory| inventory.gamedata());
        let convert_level_up = is_level_up(
            before.gamedata.and_then(|row| row.mysekai_convert_fixture_level),
            gamedata_after.and_then(|row| row.mysekai_convert_fixture_level),
        );
        let rank_up = is_level_up(
            before.gamedata.and_then(|row| row.mysekai_rank),
            gamedata_after.and_then(|row| row.mysekai_rank),
        );
        let material_possession_level_up = is_level_up(
            before.inventory.map(|row| row.mysekai_material_possession_level),
            inventory_after.map(|row| row.mysekai_material_possession_level),
        );
        let fixture_possession_level_up = is_level_up(
            before.inventory.map(|row| row.mysekai_fixture_possession_level),
            inventory_after.map(|row| row.mysekai_fixture_possession_level),
        );
        info!(
            "PostUserMysekaiNormalMissionRewardApi (seq {:?}, missions {mission_ids:?}): success; {} obtained reward(s); convert level up {convert_level_up}, material possession up {material_possession_level_up}, fixture possession up {fixture_possession_level_up}, rank up {rank_up}",
            answer.seq,
            reply.obtained_rewards.len()
        );
        received.write(MissionReceived {
            sheet_id,
            mission_ids,
            obtained_rewards: reply.obtained_rewards.clone(),
            convert_level_up,
            material_possession_level_up,
            fixture_possession_level_up,
            rank_up,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_masters::mission::MissionMasters;

    fn masters() -> MissionMasters {
        use moly_masters::mission::{Mission, MissionCategory, MissionSheet};
        MissionMasters {
            sheets: vec![MissionSheet {
                id: 1,
                seq: 1,
                name: "sheet".to_owned(),
            }],
            missions: vec![Mission {
                id: 10,
                sheet_id: 1,
                seq: 0,
                sentence: String::new(),
                category: MissionCategory::Complete,
                requirement1: 1,
            }],
            ..MissionMasters::empty()
        }
    }

    /// The service keeps one request: `answer` releases only the request in
    /// flight and only for its own sequence number.
    #[test]
    fn one_request_in_flight_and_its_own_answer_releases_it() {
        let mut service = MysekaiMissionService::default();
        let request = MissionRequest {
            request: ReceiveRequest {
                sheet_id: 1,
                mission_ids: vec![10],
            },
            sent: ServerPending {
                seq: Some(7),
                call: CALL,
            },
        };
        service.in_flight = Some(request.clone());
        assert!(service.answer(8).is_none());
        assert_eq!(service.in_flight(), Some(&request));
        assert_eq!(service.answer(7), Some(request.clone()));
        assert!(service.in_flight().is_none());
        assert_eq!(service.last(), Some(&request));
        assert_eq!(request.body().mysekai_normal_mission_ids, vec![10]);
        assert!(request.request.is_complete_sheet(&masters()));
        assert_eq!(service.server_pending(), None);
    }

    /// `IsSekaiRankUp` and the level checks: only a rise counts, and a level
    /// the copy held none of before counts as a rise.
    #[test]
    fn a_rise_is_a_rise() {
        assert!(is_level_up(Some(1), Some(2)));
        assert!(!is_level_up(Some(2), Some(2)));
        assert!(!is_level_up(Some(2), Some(1)));
        assert!(is_level_up(None, Some(1)));
        assert!(!is_level_up(None, None));
        assert!(!is_level_up(Some(1), None));
    }
}
