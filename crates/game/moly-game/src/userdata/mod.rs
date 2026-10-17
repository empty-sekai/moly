//! The client side of the server model: the copies of server data the client
//! reads, and the requests it sends. It stands where the source's
//! `UserDataManager` copies and API calls stand, apart from the server.
//!
//! The copies and the requests depend on std, bevy, serde_json and the
//! protocol rows; a copy that mirrors a server row reads that row's type and
//! its value form from the server. [`deliver`] is the one place here that
//! reads the server model's queued responses and applies them to the copies.
//! The server model is their only writer: its responses set the copies, and
//! it installs the endpoints the requests run through. A consumer imports
//! from here only.
//!
//! - [`music`]: `UserMysekaiMusicPlayFixtureSetting` rows.
//! - [`avatar`]: `UserAvatar` and the avatar wear masters.
//! - [`birthday_party`]: the birthday-party delivery user data.
//! - [`home_action`]: the craft, canvas and sketch requests.
//! - [`craft`]: the craft masters, the client's craft checks and the craft
//!   and sketch requests with their `SuiteUser` replies.
//! - [`inventory`]: the owned MySekai tables and the possession masters.
//! - [`housing_layout`]: the layout editor's save request.
//! - [`local`]: the local document and the client-local lists.
//! - [`talk_read`]: the talk read report and its reply.
//! - [`instrument_env`]: the native instruments' environment variables,
//!   which game mode never reads.
//! - [`music_play`]: the owned `UserMysekaiMusicRecord` rows and the music
//!   player's set and eject requests.
//! - [`system_fixture_action`]: the `UserMysekaiSystemFixtureAction` rows
//!   and the system fixture action request.
//! - [`tools`]: the owned harvest tools.

pub(crate) mod avatar;
pub(crate) mod birthday_party;
pub(crate) mod craft;
pub(crate) mod home_action;
pub(crate) mod housing_layout;
pub(crate) mod inventory;
pub(crate) mod local;
pub(crate) mod music;
pub(crate) mod music_play;
pub(crate) mod system_fixture_action;
pub(crate) mod talk_read;
pub(crate) mod tools;

pub(crate) use local::check_seed_local;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use serde_json::json;

use crate::server::{with_model, ResponseKind};

pub(crate) use moly_protocol::gate::{GateReply, ReplyTalkList};
pub(crate) use moly_protocol::rows::{
    ColorfulPass, Gamedata, GateCharacter, ScheduleRow, Stamina,
};

static GAME_MODE: AtomicBool = AtomicBool::new(false);

/// The page's game mode begins: every native instrument is off from here on.
/// The browser game's storage install calls it through the server model's
/// seed install, before it marks the module instance as a game.
pub(crate) fn enter_game_mode() {
    GAME_MODE.store(true, Ordering::Relaxed);
}

/// The environment variable of a native instrument. Game mode reads none:
/// every instrument is off there, and the one server document is the only
/// input.
pub(crate) fn instrument_env(name: &str) -> Option<String> {
    if GAME_MODE.load(Ordering::Relaxed) {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(name).ok()
    }
}

/// Gate replies waiting for the gate presenter, oldest first.
#[derive(Resource, Default, Debug)]
pub(crate) struct ServerGateReplies(pub(crate) VecDeque<GateReply>);

/// `MysekaiTalkDataStore.UpdateTalkList` / `SetTalkList` calls of the gate
/// presenter, for the talk store's owner. `visitors` is the reply's gate
/// characters when the call also changes who visits.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) struct TalkListUpdate {
    pub(crate) caller: &'static str,
    pub(crate) talks: ReplyTalkList,
    pub(crate) visitors: Option<Vec<GateCharacter>>,
}

#[derive(Resource, Default, Debug)]
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) struct ClientTalkListUpdates(pub(crate) Vec<TalkListUpdate>);

/// The gate presenter's `UpdateTalkList` / `SetTalkList` call.
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn client_update_talk_list(world: &mut World, update: TalkListUpdate) {
    world
        .get_resource_or_insert_with(ClientTalkListUpdates::default)
        .0
        .push(update);
}

/// A rank the client received: `UserMysekaiGamedata.mysekaiRank` of the
/// previous reply (`None` for the join, which has none) and of this one.
/// The join's rank is delivered only in the browser game; natively the
/// sites keep their offline levels until a response changes the rank.
#[derive(Message, Clone, Copy, Debug)]
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) struct RankDelivered {
    pub(crate) previous: Option<i32>,
    pub(crate) current: i32,
    pub(crate) response: &'static str,
}

/// The schedule the phenomenon of the day is chosen from, as the client
/// holds it: the rows and its copy of `refreshedAt`. `revision` counts
/// changes; the server panel writes the schedule again on each.
#[derive(Resource, Default, Debug, Clone)]
pub(crate) struct LiveSchedule {
    pub(crate) revision: u64,
    pub(crate) schedules: Vec<ScheduleRow>,
    pub(crate) refreshed_at: i64,
}

// ---------------------------------------------------------------------------
// Client copies
// ---------------------------------------------------------------------------

/// The client's copies of the user data (`UserDataManager`), set by
/// responses only. The menu, the harvest login, the rank gauges and the
/// gate presenter read these.
#[derive(Resource, Debug, Clone)]
pub(crate) struct ClientUserData {
    pub(crate) gamedata: Gamedata,
    /// `None` when the server could not seat the pools (stamina masters
    /// absent); named in the model's errors.
    pub(crate) stamina: Option<Stamina>,
    pub(crate) colorful_pass: Option<ColorfulPass>,
    /// `MasterDataManager.LastGotServerDate`.
    last_got_server_date: i64,
    /// `MasterDataManager.ApplicationTimeAtLastGotServerDate`
    /// (`Time.realtimeSinceStartup` at that response).
    application_time_at_last_got_server_date: f32,
}

impl ClientUserData {
    /// `TimeUtility.GetCurrentTimestamp`: the last server date plus the real
    /// time since it, as whole milliseconds of the float seconds.
    pub(crate) fn current_timestamp(&self, realtime_since_startup: f32) -> i64 {
        let elapsed_ms =
            (realtime_since_startup - self.application_time_at_last_got_server_date) * 1000.0;
        self.last_got_server_date.saturating_add(elapsed_ms as i64)
    }

    /// `MysekaiUtility.HasMysekaiColorfulPass`: a pass whose `expiredAt` is
    /// after the current timestamp.
    pub(crate) fn has_mysekai_colorful_pass(&self, realtime_since_startup: f32) -> bool {
        self.colorful_pass
            .is_some_and(|pass| self.current_timestamp(realtime_since_startup) < pass.expired_at)
    }

    /// `StaminaData.IsEmptyStamina` of the copy (an unseated copy is empty).
    #[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
    pub(crate) fn stamina_empty(&self) -> bool {
        self.stamina
            .is_none_or(|s| s.normal <= 0 && s.boost <= 0 && s.enhance < 1)
    }
}

/// The client side of the responses: the copies, the refresh topic, the
/// rank and the schedule the phenomenon of the day reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deliver(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut client: Option<ResMut<ClientUserData>>,
    mut live: ResMut<LiveSchedule>,
    mut replies: ResMut<ServerGateReplies>,
    mut ranks: MessageWriter<RankDelivered>,
    mut local: Option<ResMut<local::MysekaiLocalSettings>>,
    mut total_exp: Option<ResMut<crate::mysekai_rank::UserTotalExp>>,
    mut birthday: ResMut<birthday_party::ClientBirthdayPartyData>,
    mut music_copy: ResMut<music::ClientMusicPlaySettings>,
    mut avatar_copy: ResMut<avatar::ClientUserAvatar>,
    mut inventory_copy: ResMut<inventory::ClientMysekaiInventory>,
    mut records_copy: ResMut<music_play::ClientMusicRecords>,
    mut actions_copy: ResMut<system_fixture_action::ClientSystemFixtureActions>,
    mut tools_copy: ResMut<tools::ClientMysekaiTools>,
    // Paired: this system is at the parameter-count limit.
    (mode, saved): (
        Res<crate::run_mode::RunMode>,
        Option<Res<crate::player_data::SavedTotalExp>>,
    ),
) {
    let taken = with_model(|model| {
        (
            std::mem::take(&mut model.responses),
            std::mem::take(&mut model.gate_replies),
            std::mem::replace(&mut model.live_changed, false).then(|| model.served.clone()),
        )
    });
    let Some((responses, gate_replies, live_rows)) = taken else {
        return;
    };
    replies.0.extend(gate_replies);
    let realtime = time.elapsed_secs();
    let mut copy = client.as_deref().cloned();
    let mut schedule = live_rows;
    for response in responses {
        let kind = response.kind;
        let mut sections = response.sections;
        birthday.apply(std::mem::take(&mut sections.delivery));
        if let Some(rows) = sections.music.take() {
            music_copy.apply(rows);
        }
        if let Some(avatar) = sections.avatar.take() {
            avatar_copy.apply(avatar);
        }
        inventory_copy.apply(std::mem::take(&mut sections.inventory));
        if let Some(rows) = sections.music_records.take() {
            records_copy.apply(rows);
        }
        if let Some(rows) = sections.system_fixture_actions.take() {
            actions_copy.apply(rows);
        }
        if let Some(rows) = sections.tools.take() {
            tools_copy.apply(rows);
        }
        let previous_rank = copy.as_ref().and_then(|copy| copy.gamedata.mysekai_rank);
        let next = match copy.take() {
            None => ClientUserData {
                gamedata: sections.gamedata.unwrap_or(Gamedata {
                    mysekai_rank: None,
                    total_exp: 0,
                    refreshed_at: 0,
                    is_mysekai_tutorial_end: true,
                }),
                stamina: sections.stamina,
                colorful_pass: sections.colorful_pass.flatten(),
                last_got_server_date: sections.server_date_ms,
                application_time_at_last_got_server_date: realtime,
            },
            Some(mut copy) => {
                if let Some(gamedata) = sections.gamedata {
                    copy.gamedata = gamedata;
                }
                if let Some(stamina) = sections.stamina {
                    copy.stamina = Some(stamina);
                }
                if let Some(pass) = sections.colorful_pass {
                    copy.colorful_pass = pass;
                }
                copy.last_got_server_date = sections.server_date_ms;
                copy.application_time_at_last_got_server_date = realtime;
                copy
            }
        };
        if sections.is_refreshed {
            if let Some(local) = local.as_deref_mut() {
                if !local.0.topics.contains(&local::TOPIC_STAMINA_REFRESH) {
                    local.0.topics.push(local::TOPIC_STAMINA_REFRESH);
                    info!("[server] {}: isRefreshed -> MysekaiTopicsManager.AddTopicIfNeeded adds StaminaRefresh", kind.name());
                }
            }
        }
        if let Some(gamedata) = sections.gamedata {
            // UserMysekaiGamedata.totalExp is what both rank gauges read;
            // imported player data keeps its own value.
            if let Some(total) = total_exp.as_deref_mut() {
                if total.0 != gamedata.total_exp
                    && saved.as_deref().and_then(|saved| saved.0).is_none()
                {
                    total.0 = gamedata.total_exp;
                }
            }
            if let Some(current) = gamedata.mysekai_rank {
                let join = kind == ResponseKind::Join;
                if join && mode.game {
                    ranks.write(RankDelivered {
                        previous: None,
                        current,
                        response: kind.name(),
                    });
                } else if !join && previous_rank.is_some_and(|previous| previous != current) {
                    ranks.write(RankDelivered {
                        previous: previous_rank,
                        current,
                        response: kind.name(),
                    });
                }
            }
            if schedule.is_none()
                && sections.schedules.is_none()
                && live.refreshed_at != gamedata.refreshed_at
            {
                schedule = Some(live.schedules.clone());
            }
        }
        if let Some(rows) = sections.schedules {
            schedule = Some(rows);
        }
        copy = Some(next);
    }
    let Some(copy) = copy else {
        return;
    };
    if let Some(rows) = schedule {
        live.revision += 1;
        live.schedules = rows;
        live.refreshed_at = copy.gamedata.refreshed_at;
    }
    let view = json!({
        "userMysekaiGamedata": {
            "mysekaiRank": copy.gamedata.mysekai_rank,
            "totalExp": copy.gamedata.total_exp,
            "refreshedAt": copy.gamedata.refreshed_at,
        },
        "userMysekaiStamina": copy.stamina.map(moly_protocol::json::stamina_value),
        "hasMysekaiColorfulPass": copy.has_mysekai_colorful_pass(realtime),
        "currentTimestamp": copy.current_timestamp(realtime),
        "birthdayParty": birthday.view(),
        "userMysekaiMusicPlayFixtureSettings": music_copy.view(),
        "userAvatar": avatar::value(&avatar_copy.avatar),
        "mysekaiInventory": inventory_copy.view(),
        "userMysekaiMusicRecords": records_copy.view(),
        "userMysekaiTools": tools_copy.view(),
    });
    with_model(|model| model.client = view);
    match client.as_deref_mut() {
        Some(slot) => *slot = copy,
        None => {
            commands.insert_resource(copy);
        }
    }
}

/// The server panel's NPC slice (schemaVersion 1) once the client has
/// joined: the rows served at the join and the client's `refreshedAt`.
#[allow(dead_code)] // Read by the server panel's seam.
pub(crate) fn npc_slice(client: &ClientUserData, live: &LiveSchedule) -> Option<String> {
    with_model(|model| model.npc_slice(&live.schedules, client.gamedata.refreshed_at)).flatten()
}

/// Startup: the local lists from the seed's local document (the browser
/// game); natively they start empty, as on a fresh install.
fn seat_local(local: Option<ResMut<local::MysekaiLocalSettings>>) {
    let Some(text) = crate::server::LOCAL_SEED.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    let Some(mut local) = local else {
        return;
    };
    match local::LocalDocument::parse(&text) {
        Ok(doc) => {
            info!(
                "[server] local document: topics {:?}, UnlockedMasterSiteLevelIds {:?}",
                doc.topics, doc.unlocked_master_site_level_ids
            );
            local.0.topics = doc.topics;
            local.0.unlocked_master_site_level_ids = doc.unlocked_master_site_level_ids;
        }
        Err(error) => panic!("local document passed the seed check but is refused: {error}"),
    }
}

/// Last: a changed local list is written to the page backend.
fn persist_local(
    local: Option<Res<local::MysekaiLocalSettings>>,
    mode: Res<crate::run_mode::RunMode>,
    mut written: Local<Option<String>>,
) {
    if !mode.game {
        return;
    }
    let Some(local) = local else {
        return;
    };
    let text = local::LocalDocument {
        topics: local.0.topics.clone(),
        unlocked_master_site_level_ids: local.0.unlocked_master_site_level_ids.clone(),
    }
    .to_text();
    if written.as_deref() == Some(text.as_str()) {
        return;
    }
    if written.is_none() {
        // The seeded state (or a first run's empty lists) needs no write.
        *written = Some(text);
        return;
    }
    match crate::settings_store::commit_page_document("local", text.clone()) {
        Ok(()) => *written = Some(text),
        Err(error) => {
            warn!("[server] the local document is not persisted: {error}");
            *written = Some(text);
        }
    }
}

/// The client-side copies of the server data and the systems that apply the
/// server's responses to them.
pub(crate) struct UserDataPlugin;

impl Plugin for UserDataPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RankDelivered>()
            .init_resource::<LiveSchedule>()
            .init_resource::<ServerGateReplies>()
            .init_resource::<ClientTalkListUpdates>()
            .init_resource::<birthday_party::ClientBirthdayPartyData>()
            .init_resource::<music::ClientMusicPlaySettings>()
            .init_resource::<avatar::ClientUserAvatar>()
            .init_resource::<music_play::ClientMusicRecords>()
            .init_resource::<tools::ClientMysekaiTools>()
            .add_systems(PreStartup, seat_local)
            .add_systems(PreUpdate, deliver.after(crate::server::tick))
            .add_systems(Last, persist_local);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_clock_adds_real_time_to_the_last_server_date() {
        let copy = ClientUserData {
            gamedata: Gamedata {
                mysekai_rank: Some(1),
                total_exp: 0,
                refreshed_at: 0,
                is_mysekai_tutorial_end: true,
            },
            stamina: None,
            colorful_pass: Some(ColorfulPass {
                id: 1,
                expired_at: 10_500,
            }),
            last_got_server_date: 10_000,
            application_time_at_last_got_server_date: 2.0,
        };
        assert_eq!(copy.current_timestamp(2.25), 10_250);
        assert!(copy.has_mysekai_colorful_pass(2.4));
        assert!(!copy.has_mysekai_colorful_pass(2.5));
    }
}
