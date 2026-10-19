//! The client side of the server model: the copies of server data the client
//! reads, and the requests it sends. It stands where the source's
//! `UserDataManager` copies and API calls stand, apart from the server.
//!
//! The copies and the requests depend on std, bevy, serde_json and the
//! protocol rows; a copy that mirrors a server row reads that row's type and
//! its value form from the server. [`apply_arrivals`] is the one place here
//! that reads what the server sent: it applies the arrived sections to the
//! copies, emits the domain events, and then writes [`Answered`], which the
//! features continue on. The link is the only source of these values: there
//! is no in-process model fallback.
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
//! - [`convert`]: the convert slots, their item histories and the convert
//!   items the convert screen's list is built from.
//! - [`mission`]: the mission sheet the user is on and the user's mission
//!   rows, which the mission screen reads.
//! - [`inventory`]: the inventory screen's recycle and disassemble request.

pub mod avatar;
pub mod birthday_party;
pub mod client;
pub mod clock;
pub mod convert;
pub mod craft;
pub mod home_action;
pub mod housing_layout;
pub mod inventory;
pub mod local;
pub mod mission;
pub mod music;
pub mod music_play;
pub use crate::userdata_snapshot as snapshot;
pub mod stamina;
pub mod stamina_recover;
pub mod system_fixture_action;
pub mod talk_read;
pub mod tools;

pub use local::check_seed_local;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use moly_link::{Arrival, LinkState, ServerArrived, ServerLink};
use moly_masters::craft::{is_first_craft, CraftMasters, CraftOwned};
use moly_protocol::api::{
    MysekaiCraftType, MysekaiInventoryDisassemble, UserMysekaiCraftRequest,
};
use moly_protocol::{AdminCall, Call, CallKind, PushKind, Refusal, Reply, Sections};
use serde_json::{json, Value};

pub use moly_protocol::gate::{GateReply, ReplyTalkList};
pub use moly_protocol::rows::{
    ColorfulPass, Gamedata, GateCharacter, ScheduleRow, Stamina,
};

pub use moly_core::dev_tools::{enter_game_mode, instrument_env};

/// The seed's text the game keeps client-side: the local document (the
/// client-local lists) and the server document a page seed carries. A native
/// start carries no server document; the server's own store is authoritative.
#[derive(Resource, Clone, Default)]
pub struct SeedDocuments {
    pub server: Option<String>,
    pub local: Option<String>,
}

pub static LOCAL_SEED: Mutex<Option<String>> = Mutex::new(None);
static SEED: Mutex<SeedDocuments> = Mutex::new(SeedDocuments {
    server: None,
    local: None,
});

/// Before the app is built: the seed's documents, and the page's game mode.
pub fn install_seed(server: Option<String>, local: Option<String>) {
    enter_game_mode();
    if let Ok(mut slot) = LOCAL_SEED.lock() {
        *slot = local.clone();
    }
    if let Ok(mut slot) = SEED.lock() {
        *slot = SeedDocuments { server, local };
    }
}

/// Gate replies waiting for the gate presenter, oldest first.
#[derive(Resource, Default, Debug)]
pub struct ServerGateReplies(pub VecDeque<GateReply>);

pub use crate::server_panel_data::{
    client_update_talk_list, ClientNpcPanel, ClientTalkListUpdates, LiveSchedule, TalkListUpdate,
};

/// A rank the client received: `UserMysekaiGamedata.mysekaiRank` of the
/// previous reply (`None` for the join, which has none) and of this one.
#[derive(Message, Clone, Copy, Debug)]
pub struct RankDelivered {
    pub previous: Option<i32>,
    pub current: i32,
    pub response: &'static str,
}

/// A call's answer, written once the arrived sections are in the copies: a
/// feature that waits for an answer therefore reads the copies after it.
#[derive(Message, Clone, Debug)]
pub struct Answered {
    pub seq: u32,
    pub call: CallKind,
    pub outcome: Result<Reply, Refusal>,
}

/// The server call a feature waits for: its sequence number and kind. A
/// feature keeps one for its screen; the director reads it through the
/// feature's own accessor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerPending {
    pub seq: Option<u32>,
    pub call: CallKind,
}

/// The last answers, for the director and the control plane, which poll by
/// sequence number rather than reading a stream.
#[derive(Resource, Default, Debug)]
pub struct AnsweredLog {
    entries: Vec<Answered>,
}

impl AnsweredLog {
    pub fn push(&mut self, answered: Answered) {
        if self.entries.len() >= MAX_ANSWERED {
            self.entries.remove(0);
        }
        self.entries.push(answered);
    }

    pub fn get(&self, seq: u32) -> Option<&Answered> {
        self.entries.iter().find(|answered| answered.seq == seq)
    }

    /// The newest answer of `call` at or after `since` (an index in this log).
    pub fn newest_since(&self, call: CallKind, since: usize) -> Option<&Answered> {
        self.entries
            .iter()
            .skip(since)
            .filter(|answered| answered.call == call)
            .next_back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Where the session is, from the link opening to the join's answer.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum ServerJoin {
    /// Not open, or waiting to open.
    #[default]
    Idle,
    /// The page's seed document is being installed.
    Installing(u32),
    /// The join is waiting for its answer.
    Joining { seq: u32, retry_at: Option<f32> },
    Joined,
    /// The server refused the join or the install; no fallback.
    Failed(String),
}

/// The last link state the notice reported.
#[derive(Resource, Default)]
pub struct LinkNoticed(pub Option<LinkState>);

/// The control plane's cached answers, filled from the administrative calls'
/// replies.
#[derive(Resource, Default, Debug)]
pub struct ServerViewCache {
    pub schema: Option<String>,
    pub document: Option<String>,
    pub revision: u64,
    pub errors: Vec<String>,
    /// The game's local part of the document (its copies), which the server
    /// does not hold.
    pub client: Value,
    /// The newest edit's answer: (seq, receipt or refusal).
    pub last_edit: Option<(u32, Result<String, Refusal>)>,
}

impl ServerViewCache {
    pub fn schema_text(&self) -> String {
        self.schema
            .clone()
            .unwrap_or_else(|| json!({"schemaVersion": 2, "pending": true}).to_string())
    }

    pub fn document_text(&self) -> String {
        let base = self.document.clone().unwrap_or_else(|| {
            json!({"schemaVersion": 2, "document": null, "joined": false, "pending": true, "errors": []})
                .to_string()
        });
        merge_client(base, &self.client)
    }
}

/// The document the server sent with the game's own `client` part inserted.
pub fn merge_client(text: String, client: &Value) -> String {
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(mut object)) => {
            object.insert("client".into(), client.clone());
            Value::Object(object).to_string()
        }
        _ => text,
    }
}

/// The calls the game sent and has not answered yet, with their requests:
/// the request's fields are what the exchange's events and the reply's
/// client-side bookkeeping read.
#[derive(Resource, Default)]
pub struct SentCalls(pub BTreeMap<u32, Call>);

impl SentCalls {
    fn remember(&mut self, seq: u32, call: Call) {
        if self.0.len() >= MAX_SENT {
            if let Some(oldest) = self.0.keys().next().copied() {
                self.0.remove(&oldest);
            }
        }
        self.0.insert(seq, call);
    }
}

const MAX_ANSWERED: usize = 512;
const MAX_SENT: usize = 256;
const JOIN_RETRY_SECONDS: f32 = 1.0;
/// The base texture the craft screen sends when no colour is chosen.
const BASE_TEXTURE: i32 = 1;

/// The client copies one arrived response is applied to. Bundled so the
/// reader stays under the system parameter limit.
#[derive(SystemParam)]
pub struct ClientCopies<'w> {
    pub client: Option<ResMut<'w, ClientUserData>>,
    pub live: ResMut<'w, LiveSchedule>,
    pub replies: ResMut<'w, ServerGateReplies>,
    pub ranks: MessageWriter<'w, RankDelivered>,
    pub local: Option<ResMut<'w, local::MysekaiLocalSettings>>,
    pub total_exp: Option<ResMut<'w, crate::mysekai_rank::UserTotalExp>>,
    pub birthday: ResMut<'w, birthday_party::ClientBirthdayPartyData>,
    pub music: ResMut<'w, music::ClientMusicPlaySettings>,
    pub avatar: ResMut<'w, avatar::ClientUserAvatar>,
    pub inventory: ResMut<'w, inventory::ClientMysekaiInventory>,
    pub records: ResMut<'w, music_play::ClientMusicRecords>,
    pub actions: ResMut<'w, system_fixture_action::ClientSystemFixtureActions>,
    pub tools: ResMut<'w, tools::ClientMysekaiTools>,
    pub convert: ResMut<'w, convert::ClientConvertSlots>,
    pub mission: ResMut<'w, mission::ClientMission>,
    pub npc_panel: ResMut<'w, ClientNpcPanel>,
    pub cache: ResMut<'w, ServerViewCache>,
    pub answered: MessageWriter<'w, Answered>,
}


// ---------------------------------------------------------------------------
// The client copies
// ---------------------------------------------------------------------------

/// The client's copies of the user data (`UserDataManager`), set by
/// responses only. The menu, the harvest login, the rank gauges and the
/// gate presenter read these.
#[derive(Resource, Debug, Clone)]
pub struct ClientUserData {
    pub gamedata: Gamedata,
    /// `None` when the server could not seat the pools (stamina masters
    /// absent); named in the model's errors.
    pub stamina: Option<Stamina>,
    pub colorful_pass: Option<ColorfulPass>,
    /// `MasterDataManager.LastGotServerDate`.
    last_got_server_date: i64,
    /// `MasterDataManager.ApplicationTimeAtLastGotServerDate`
    /// (`Time.realtimeSinceStartup` at that response).
    application_time_at_last_got_server_date: f32,
}

impl ClientUserData {
    /// `TimeUtility.GetCurrentTimestamp`: the last server date plus the real
    /// time since it, as whole milliseconds of the float seconds.
    pub fn current_timestamp(&self, realtime_since_startup: f32) -> i64 {
        let elapsed_ms =
            (realtime_since_startup - self.application_time_at_last_got_server_date) * 1000.0;
        self.last_got_server_date.saturating_add(elapsed_ms as i64)
    }

    /// `MysekaiUtility.HasMysekaiColorfulPass`: a pass whose `expiredAt` is
    /// after the current timestamp.
    pub fn has_mysekai_colorful_pass(&self, realtime_since_startup: f32) -> bool {
        self.colorful_pass
            .is_some_and(|pass| self.current_timestamp(realtime_since_startup) < pass.expired_at)
    }

    /// `StaminaData.IsEmptyStamina` of the copy (an unseated copy is empty).
    pub fn stamina_empty(&self) -> bool {
        self.stamina
            .is_none_or(|s| s.normal <= 0 && s.boost <= 0 && s.enhance < 1)
    }
}

// ---------------------------------------------------------------------------
// The reader
// ---------------------------------------------------------------------------

/// PreUpdate: the calls made since the last frame enter the event stream and
/// are remembered for their replies.
fn emit_link_events(mut link: ResMut<ServerLink>, mut sent: ResMut<SentCalls>) {
    for (seq, call) in link.take_sent() {
        let kind = call.kind();
        crate::bus::emit("server.call", json!({ "seq": seq, "call": kind.name() }));
        sent.remember(seq, call);
    }
}

/// PreUpdate: a lost or refused session is named on screen and in the log,
/// once per state.
fn note_link_state(
    mut commands: Commands,
    link: Res<ServerLink>,
    mut noticed: ResMut<LinkNoticed>,
) {
    let state = link.state().clone();
    if noticed.0.as_ref() == Some(&state) {
        return;
    }
    noticed.0 = Some(state.clone());
    match &state {
        LinkState::Connecting => {
            crate::bus::emit("server.link", json!({ "state": "connecting" }));
        }
        LinkState::Open { server } => {
            info!("[link] session open: {server}");
            crate::bus::emit("server.link", json!({ "state": "open", "server": server }));
        }
        LinkState::Refused { reason } => {
            let body = format!("the server refused the session: {reason}");
            error!("[link] {body}");
            crate::bus::emit("server.link", json!({ "state": "refused", "reason": reason }));
            commands.queue(move |world: &mut World| notice(world, &body));
        }
        LinkState::Lost { reason } => {
            let body = format!("the server is not reachable: {reason}");
            error!("[link] {body}");
            crate::bus::emit("server.link", json!({ "state": "lost", "reason": reason }));
            commands.queue(move |world: &mut World| notice(world, &body));
        }
    }
}

/// Names the reason on screen (a common dialog once the UI exists) and in the
/// page's error list.
fn notice(world: &mut World, body: &str) {
    if let Some(mut errors) = world.get_resource_mut::<crate::browser_game::GameErrors>() {
        errors.push(body.to_owned());
    }
    if world
        .get_resource::<crate::ui_layers::ScreenManager>()
        .is_none()
        || world.get_resource::<crate::ui_layout::UiLayouts>().is_none()
    {
        return;
    }
    crate::common_dialog::open_detached(
        world,
        "ServerLink",
        body.to_owned(),
        crate::common_dialog::Buttons::One { ok: "WORD_OK" },
    );
}

/// PreUpdate: send the page seed's install and the join once the session is
/// open, and retry a join the server answered `masters_loading`.
fn drive_join(
    mut link: ResMut<ServerLink>,
    seed: Res<SeedDocuments>,
    mut phase: ResMut<ServerJoin>,
    time: Res<Time<Real>>,
) {
    if !link.state().is_open() {
        if !matches!(*phase, ServerJoin::Idle) {
            *phase = ServerJoin::Idle;
        }
        return;
    }
    let now = time.elapsed_secs();
    match phase.clone() {
        ServerJoin::Idle => {
            let seq = match &seed.server {
                Some(text) => link.call(Call::Admin(AdminCall::Install {
                    server_json: Some(text.clone()),
                    local_json: seed.local.clone(),
                })),
                None => link.call(Call::Join),
            };
            *phase = if seed.server.is_some() {
                ServerJoin::Installing(seq)
            } else {
                ServerJoin::Joining {
                    seq,
                    retry_at: None,
                }
            };
        }
        ServerJoin::Joining {
            retry_at: Some(at), ..
        } if now >= at => {
            let seq = link.call(Call::Join);
            *phase = ServerJoin::Joining {
                seq,
                retry_at: None,
            };
        }
        _ => {}
    }
}

/// PreUpdate, after the reader: the join's answer moves the phase on.
fn track_join(
    mut link: ResMut<ServerLink>,
    mut phase: ResMut<ServerJoin>,
    mut answered: MessageReader<Answered>,
    time: Res<Time<Real>>,
) {
    for answer in answered.read() {
        match phase.clone() {
            ServerJoin::Installing(seq) if seq == answer.seq => match &answer.outcome {
                Ok(Reply::Admin(_)) => {
                    let seq = link.call(Call::Join);
                    *phase = ServerJoin::Joining {
                        seq,
                        retry_at: None,
                    };
                    info!("[link] seed installed; the join is sent");
                }
                Ok(_) => {}
                Err(refusal) => {
                    error!(
                        "[link] the seed install was refused ({refusal}); the game is not joined"
                    );
                    *phase = ServerJoin::Failed(refusal.to_string());
                }
            },
            ServerJoin::Joining { seq, .. } if seq == answer.seq => match &answer.outcome {
                Ok(Reply::Joined) => {
                    info!("[link] joined the server");
                    *phase = ServerJoin::Joined;
                }
                Ok(_) => {}
                Err(refusal) if refusal.code == "masters_loading" => {
                    info!(
                        "[link] the join waits for the server's masters ({}); it is retried",
                        refusal.reason
                    );
                    *phase = ServerJoin::Joining {
                        seq: answer.seq,
                        retry_at: Some(time.elapsed_secs() + JOIN_RETRY_SECONDS),
                    };
                }
                Err(refusal) => {
                    error!("[link] the join was refused ({refusal}); the game is not joined");
                    *phase = ServerJoin::Failed(refusal.to_string());
                }
            },
            _ => {}
        }
    }
}

/// PreUpdate: the arrived sections are applied to the copies in the server's
/// order, then each answer is published to the features.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_arrivals(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut arrived: MessageReader<ServerArrived>,
    mut copies: ClientCopies,
    mut log: ResMut<AnsweredLog>,
    mut sent: ResMut<SentCalls>,
    craft_masters: Option<Res<CraftMasters>>,
    (mode, saved): (
        Res<crate::run_mode::RunMode>,
        Option<Res<crate::player_data::SavedTotalExp>>,
    ),
) {
    let realtime = time.elapsed_secs();
    for arrival in arrived.read() {
        match &arrival.event {
            Arrival::Push { kind, sections } => {
                crate::bus::emit("server.push", json!({ "kind": push_name(*kind) }));
                apply_sections(
                    &mut commands,
                    &mut copies,
                    None,
                    sections.clone(),
                    realtime,
                    &mode,
                    saved.as_deref(),
                );
            }
            Arrival::Reply {
                call,
                outcome,
                sections,
            } => {
                let seq = arrival.seq.unwrap_or_default();
                if let Ok(Reply::Gate(reply)) = outcome {
                    copies.replies.0.push_back(reply.clone());
                }
                crate::bus::emit(
                    "server.reply",
                    match outcome {
                        Ok(_) => json!({ "seq": seq, "call": call.name(), "ok": true }),
                        Err(refusal) => json!({
                            "seq": seq,
                            "call": call.name(),
                            "ok": false,
                            "refusal": refusal.code,
                        }),
                    },
                );
                let request = sent.0.remove(&seq);
                // A refusal, and an administration answer, carry no user
                // data; its empty `Sections` must not overwrite the copies.
                let carried = sections.server_date_ms != 0;
                let before = match (&request, *call) {
                    (Some(Call::Craft(request)), CallKind::Craft) => {
                        Some(craft_before(&copies, request, craft_masters.as_deref()))
                    }
                    _ => None,
                };
                if carried {
                    apply_sections(
                        &mut commands,
                        &mut copies,
                        Some(*call),
                        sections.clone(),
                        realtime,
                        &mode,
                        saved.as_deref(),
                    );
                }
                fill_cache(&mut *copies.cache, seq, &request, outcome);
                if let (Some(Call::Craft(request)), Some(before)) = (&request, before) {
                    emit_craft(outcome, request, before, &copies);
                }
                if let (Some(Call::InventoryRecycle(request)), reply) = (&request, outcome) {
                    emit_recycle(request, reply, &copies);
                }
                if let (Some(Call::BirthdayPartyDelivery(request)), Ok(Reply::Delivery(_))) =
                    (&request, outcome)
                {
                    crate::bus::emit(
                        "delivery_done",
                        json!({
                            "party_id": request.birthday_party_id,
                            "consumed": request.consumed_delivery_material_count,
                        }),
                    );
                }
                let answer = Answered {
                    seq,
                    call: *call,
                    outcome: outcome.clone(),
                };
                copies.answered.write(answer.clone());
                log.push(answer);
            }
        }
    }
}

/// Applies one response's sections, as the in-process reader did.
#[allow(clippy::too_many_arguments)]
fn apply_sections(
    commands: &mut Commands,
    copies: &mut ClientCopies,
    kind: Option<CallKind>,
    mut sections: Sections,
    realtime: f32,
    mode: &crate::run_mode::RunMode,
    saved: Option<&crate::player_data::SavedTotalExp>,
) {
    clock::seat_server_date(sections.server_date_ms);
    if let Some(gate) = sections.gate {
        snapshot::seat_gate(gate);
    }
    if let Some(rows) = sections.rank_releases.take() {
        snapshot::seat_rank_releases(rows);
    }
    if let Some(text) = sections.npc_panel.take() {
        copies.npc_panel.apply(text);
    }
    copies.birthday.apply(std::mem::take(&mut sections.delivery));
    if let Some(rows) = sections.music.take() {
        copies.music.apply(rows);
    }
    if let Some(avatar) = sections.avatar.take() {
        copies.avatar.apply(avatar);
    }
    copies.inventory.apply(std::mem::take(&mut sections.inventory));
    if let Some(rows) = sections.music_records.take() {
        copies.records.apply(rows);
    }
    if let Some(rows) = sections.system_fixture_actions.take() {
        copies.actions.apply(rows);
    }
    if let Some(rows) = sections.tools.take() {
        copies.tools.apply(rows);
    }
    // The convert screen's three sections: its slots, the items' histories,
    // and the convert items the response carries beside the user sections.
    copies.convert.apply(
        sections.convert_slots.take(),
        sections.convert_item_histories.take(),
        sections.mysekai_convert_items.take(),
    );
    // The mission screen's two tables: the sheet the user is on and the
    // user's mission rows (`UpdateMysekaiMissionSheet` / `UpdateMysekaiMission`).
    {
        let sheet = sections.normal_mission_sheet.take();
        let missions = sections.normal_missions.take();
        copies.mission.apply(mission::MissionSections { sheet, missions });
    }
    let previous_rank = copies
        .client
        .as_ref()
        .and_then(|copy| copy.gamedata.mysekai_rank);
    let mut copy = copies.client.as_deref().cloned();
    let mut schedule: Option<Vec<ScheduleRow>> = None;
    let next = match copy.take() {
        None => ClientUserData {
            gamedata: sections.gamedata.unwrap_or(Gamedata {
                mysekai_rank: None,
                total_exp: 0,
                refreshed_at: 0,
                is_mysekai_tutorial_end: true,
                mysekai_convert_fixture_level: None,
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
        if let Some(local) = copies.local.as_deref_mut() {
            if !local.0.topics.contains(&local::TOPIC_STAMINA_REFRESH) {
                local.0.topics.push(local::TOPIC_STAMINA_REFRESH);
                info!(
                    "[server] {}: isRefreshed -> MysekaiTopicsManager.AddTopicIfNeeded adds StaminaRefresh",
                    kind.map_or("push", CallKind::name)
                );
            }
        }
    }
    if let Some(gamedata) = sections.gamedata {
        // UserMysekaiGamedata.totalExp is what both rank gauges read;
        // imported player data keeps its own value.
        if let Some(total) = copies.total_exp.as_deref_mut() {
            if total.0 != gamedata.total_exp && saved.and_then(|saved| saved.0).is_none() {
                total.0 = gamedata.total_exp;
            }
        }
        if let Some(current) = gamedata.mysekai_rank {
            let join = kind == Some(CallKind::Join);
            let response = kind.map_or("push", CallKind::name);
            if join && mode.game {
                copies.ranks.write(RankDelivered {
                    previous: None,
                    current,
                    response,
                });
            } else if !join && previous_rank.is_some_and(|previous| previous != current) {
                copies.ranks.write(RankDelivered {
                    previous: previous_rank,
                    current,
                    response,
                });
            }
        }
        if schedule.is_none()
            && sections.schedules.is_none()
            && copies.live.refreshed_at != gamedata.refreshed_at
        {
            schedule = Some(copies.live.schedules.clone());
        }
    }
    if let Some(rows) = sections.schedules {
        schedule = Some(rows);
    }
    copy = Some(next);
    let Some(copy) = copy else {
        return;
    };
    if let Some(rows) = schedule {
        copies.live.revision += 1;
        copies.live.schedules = rows;
        copies.live.refreshed_at = copy.gamedata.refreshed_at;
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
        "birthdayParty": copies.birthday.view(),
        "userMysekaiMusicPlayFixtureSettings": copies.music.view(),
        "userAvatar": avatar::value(&copies.avatar.avatar),
        "mysekaiInventory": copies.inventory.view(),
        "userMysekaiMusicRecords": copies.records.view(),
        "userMysekaiTools": copies.tools.view(),
        "mysekaiConvert": copies.convert.view(),
        // The mission copy's sheet name: the mission masters are not among
        // this reader's parameters, so only the keys and rows are shown
        // there (the name is null).
        "userMysekaiMission": copies.mission.view(None),
    });
    copies.cache.client = view;
    match copies.client.as_deref_mut() {
        Some(slot) => *slot = copy,
        None => {
            commands.insert_resource(copy);
        }
    }
}

fn push_name(kind: PushKind) -> &'static str {
    match kind {
        PushKind::Refresh => "refresh",
        PushKind::Sync => "sync",
        PushKind::BirthdayPartySeat => "birthday_party_seat",
    }
}

/// Fills the control plane's cache from an administrative answer.
fn fill_cache(
    cache: &mut ServerViewCache,
    seq: u32,
    request: &Option<Call>,
    outcome: &Result<Reply, Refusal>,
) {
    match (request, outcome) {
        (Some(Call::Admin(AdminCall::Schema)), Ok(Reply::Admin(text))) => {
            cache.schema = Some(text.clone());
        }
        (Some(Call::Admin(AdminCall::Document)), Ok(Reply::Admin(text))) => {
            cache.document = Some(text.clone());
            if let Ok(Value::Object(document)) = serde_json::from_str::<Value>(text) {
                cache.revision = document
                    .get("revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(cache.revision);
                cache.errors = document
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(|errors| {
                        errors
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
            }
        }
        (Some(Call::Admin(AdminCall::Edit { .. })), outcome) => {
            let receipt = match outcome {
                Ok(Reply::Admin(text)) => Ok(text.clone()),
                Ok(other) => Ok(format!("{other:?}")),
                Err(refusal) => Err(refusal.clone()),
            };
            cache.last_edit = Some((seq, receipt));
        }
        _ => {}
    }
}

/// The request's craft data as it stands in the copies before the reply's
/// sections are applied.
struct CraftBefore {
    texture_id: Option<i32>,
    craft_type: Option<&'static str>,
    target_id: Option<i32>,
    first: bool,
    spent: Vec<(i32, i32)>,
    materials: BTreeMap<i32, i32>,
    total_exp: i32,
}

fn craft_before(
    copies: &ClientCopies,
    request: &UserMysekaiCraftRequest,
    masters: Option<&CraftMasters>,
) -> CraftBefore {
    let owned = CraftOwned {
        materials: copies.inventory.materials(),
        fixtures: copies.inventory.fixtures(),
        canvases: copies.inventory.canvases(),
        fixture_max_count: None,
    };
    let blueprint = masters.and_then(|masters| masters.blueprint(request.blueprint_id));
    let tool = blueprint.is_some_and(|blueprint| blueprint.craft_type == MysekaiCraftType::MysekaiTool);
    let texture_id = (!tool).then_some(request.texture_id.unwrap_or(BASE_TEXTURE));
    let spent = masters
        .and_then(|masters| masters.blueprint_costs(request.blueprint_id))
        .map(|costs| {
            costs
                .iter()
                .map(|cost| {
                    (
                        cost.mysekai_material_id,
                        cost.quantity.wrapping_mul(request.quantity),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    CraftBefore {
        texture_id,
        craft_type: blueprint.map(|blueprint| blueprint.craft_type.name()),
        target_id: blueprint.map(|blueprint| blueprint.craft_target_id),
        first: blueprint.is_some_and(|blueprint| is_first_craft(blueprint, &owned)),
        spent,
        materials: copies.birthday.mysekai_materials().clone(),
        total_exp: copies
            .client
            .as_ref()
            .map(|copy| copy.gamedata.total_exp)
            .unwrap_or(0),
    }
}

/// The recycle or disassemble reply's domain event: the request, whether the
/// server carried it out, and the resources it gave back.
fn emit_recycle(
    request: &MysekaiInventoryDisassemble,
    outcome: &Result<Reply, Refusal>,
    copies: &ClientCopies,
) {
    let (api, id, quantity) = match request {
        MysekaiInventoryDisassemble::Fixture(row) => {
            ("fixture", row.mysekai_fixture_id, row.quantity)
        }
        MysekaiInventoryDisassemble::Material(row) => {
            ("material", row.mysekai_material_id, row.quantity)
        }
        MysekaiInventoryDisassemble::Tool(row) => ("tool", row.mysekai_tool_id, row.quantity),
    };
    let resources = |rows: &[moly_protocol::rows::UserResource]| {
        rows.iter()
            .map(|row| {
                json!({
                    "resource_type": row.resource_type,
                    "resource_id": row.resource_id,
                    "quantity": row.quantity,
                })
            })
            .collect::<Vec<_>>()
    };
    match outcome {
        Ok(Reply::InventoryRecycle(reply)) => crate::bus::emit(
            "recycle_done",
            json!({
                "api": request.api_name(),
                "kind": api,
                "id": id,
                "quantity": quantity,
                "obtained": resources(&reply.obtained_resources),
                "inventory_revision": copies.inventory.revision,
                "tools_revision": copies.tools.revision,
            }),
        ),
        Ok(_) => {}
        Err(refusal) => crate::bus::emit(
            "recycle_refused",
            json!({
                "api": request.api_name(),
                "kind": api,
                "id": id,
                "quantity": quantity,
                "reason": refusal.reason.clone(),
            }),
        ),
    }
}

/// The craft reply's domain event, its fields computed from the client's own
/// copies: the recipe master and the count for `spent`, the copies before and
/// after for `first` and `exp`.
fn emit_craft(
    outcome: &Result<Reply, Refusal>,
    request: &UserMysekaiCraftRequest,
    before: CraftBefore,
    copies: &ClientCopies,
) {
    match outcome {
        Ok(Reply::SuiteUser(reply)) if reply.success => {
            let after = copies
                .client
                .as_ref()
                .map(|copy| copy.gamedata.total_exp)
                .unwrap_or(before.total_exp);
            let spent = if before.spent.is_empty() {
                let now = copies.birthday.mysekai_materials();
                let mut spent: Vec<(i32, i32)> = before
                    .materials
                    .iter()
                    .filter_map(|(material, was)| {
                        let is = now.get(material).copied().unwrap_or(0);
                        (is < *was).then_some((*material, *was - is))
                    })
                    .collect();
                spent.sort_unstable();
                spent
            } else {
                before.spent
            };
            crate::bus::emit(
                "craft_done",
                json!({
                    "blueprint_id": request.blueprint_id,
                    "craft_type": before.craft_type,
                    "target_id": before.target_id,
                    "texture_id": before.texture_id,
                    "quantity": request.quantity,
                    "spent": spent
                        .iter()
                        .map(|(material, quantity)| json!({ "material_id": material, "quantity": quantity }))
                        .collect::<Vec<_>>(),
                    "first": before.first,
                    "exp": (after - before.total_exp).max(0),
                }),
            );
        }
        Ok(Reply::SuiteUser(reply)) => {
            crate::bus::emit(
                "craft_refused",
                json!({
                    "blueprint_id": request.blueprint_id,
                    "quantity": request.quantity,
                    "reason": reply.refusal.clone().unwrap_or_default(),
                }),
            );
        }
        Err(refusal) => {
            crate::bus::emit(
                "craft_refused",
                json!({
                    "blueprint_id": request.blueprint_id,
                    "quantity": request.quantity,
                    "reason": refusal.reason.clone(),
                }),
            );
        }
        Ok(_) => {}
    }
}

// ---------------------------------------------------------------------------
// The seed and the local lists
// ---------------------------------------------------------------------------

/// Startup: the seed's documents into their resources.
fn seat_seed(mut commands: Commands) {
    let seed = SEED.lock().map(|slot| slot.clone()).unwrap_or_default();
    commands.insert_resource(seed);
}

/// Startup: the local lists from the seed's local document (the browser
/// game); natively they start empty, as on a fresh install.
fn seat_local(local: Option<ResMut<local::MysekaiLocalSettings>>) {
    let Some(text) = LOCAL_SEED.lock().ok().and_then(|mut slot| slot.take()) else {
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

// ---------------------------------------------------------------------------
// The public helpers
// ---------------------------------------------------------------------------

/// Sends one administrative call on the server link, when the game has one.
pub fn admin_call(world: &mut World, call: AdminCall) -> Option<u32> {
    world
        .get_resource_mut::<ServerLink>()
        .map(|mut link| link.call(Call::Admin(call)))
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
pub struct UserDataPlugin;

impl Plugin for UserDataPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(moly_masters::load::MasterTablesPlugin)
            .add_plugins(moly_masters::load::MasterRowsPlugin)
            .add_message::<RankDelivered>()
            .add_message::<Answered>()
            .init_resource::<LiveSchedule>()
            .init_resource::<ServerGateReplies>()
            .init_resource::<ClientTalkListUpdates>()
            .init_resource::<ClientNpcPanel>()
            .init_resource::<AnsweredLog>()
            .init_resource::<ServerJoin>()
            .init_resource::<LinkNoticed>()
            .init_resource::<ServerViewCache>()
            .init_resource::<SentCalls>()
            .init_resource::<birthday_party::ClientBirthdayPartyData>()
            .init_resource::<music::ClientMusicPlaySettings>()
            .init_resource::<avatar::ClientUserAvatar>()
            .init_resource::<inventory::ClientMysekaiInventory>()
            .init_resource::<system_fixture_action::ClientSystemFixtureActions>()
            .init_resource::<music_play::ClientMusicRecords>()
            .init_resource::<tools::ClientMysekaiTools>()
            .init_resource::<convert::ClientConvertSlots>()
            .init_resource::<mission::ClientMission>()
            .add_systems(PreStartup, (seat_seed, seat_local))
            .add_systems(
                PreUpdate,
                (
                    note_link_state,
                    emit_link_events,
                    drive_join,
                    apply_arrivals,
                    track_join,
                )
                    .chain(),
            )
            .add_systems(
                PreUpdate,
                inventory::mirror_materials.after(apply_arrivals),
            )
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
                mysekai_convert_fixture_level: None,
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
