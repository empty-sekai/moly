//! The single server model: one document the player edits, one server
//! clock, the server's named policies, and the responses that carry the
//! server's values to the client's copies.
//!
//! **What is server and what is client.** The document (schemaVersion 2,
//! [`document`]) holds only server-decided values. The client side of the
//! newer sections ([`client`]: the music record settings, the avatar wear and
//! its masters, the home actions' requests, the native instruments) depends
//! on no server internals, so it can sit below the server in the crate
//! graph; the model here is its only writer. The client's copies are
//! [`ClientUserData`] (`UserDataManager`'s `UserMysekaiGamedata`,
//! `UserMysekaiStamina`, `UserMysekaiColorfulPass` and the server date it
//! last got) and the application's local lists (the local document,
//! [`local`]). The client copies change only when a response arrives:
//!
//! - **Join** (`PostUserMysekaiApi`, the entry's request): every section.
//!   Its `isRefreshed` adds the `StaminaRefresh` topic to the local lists
//!   (`MysekaiTopicsManager.AddTopicIfNeeded(UserMysekaiResponse)`); the
//!   entry's refresh performance reads that topic.
//! - **Harvest and gather replies** (`PostUserMysekaiHarvestApi`,
//!   `PostUserMysekaiGatherApi`): the stamina, through the harvest owner's
//!   calls into [`harvest_api_stamina`] and [`gather_api_stamina`].
//! - **Gate replies** (reserve and change): the panel's gate actions stand
//!   in for the requests the gate screens make (those screens are not built);
//!   the reply is queued in [`ServerGateReplies`] for the gate presenter.
//! - **Birthday-party delivery replies** (`PutUserMysekaiBirthdayPartyDeliveryApi`,
//!   `PutUserMysekaiBirthdayPartyGatherApi`): the delivery site calls
//!   [`delivery::put_birthday_party_delivery`] and
//!   [`delivery::put_birthday_party_gather`]; their `updatedResources` merge
//!   into [`delivery::ClientBirthdayPartyData`] at once, as the API executor
//!   merges them before the caller reads the user data. The rows of a party
//!   that comes into session are a response of their own
//!   ([`ResponseKind::BirthdayPartySeat`], a named adaptation of the login's
//!   user data).
//! - **Refresh** (named adaptation): when the running server clock enters a
//!   new master refresh window, the server refreshes and the client gets the
//!   refreshed values at once, standing in for the join request the client
//!   makes again after a refresh.
//! - **Sync** (named adaptation): the panel's `sync` action is a response
//!   with nothing but the pending sections, standing in for any request the
//!   client would make next.
//!
//! Every response carries the sections edited since the last response
//! (`pending`), so a panel edit of the stamina, the game data or the pass
//! applies as the next server response. The clock and the schedule apply
//! live: the phenomenon of the day is chosen again at once
//! ([`LiveSchedule`]).
//!
//! **Named policies** (server rules with no client code):
//! - *Stamina refresh*: [`document::REFILL_POLICY_TEXT`]. The windows are
//!   the master refresh time periods of the phenomena index; the server
//!   stamps `refreshedAt` with its clock at the refresh.
//! - *Initial stamina*: a user whose document has no stamina gets the normal
//!   pool at its master maximum, enhance 0 and the boost of one recovery
//!   (the first stamina-recovery row's `recoveryBoostStamina`).
//! - *Rank from experience*: `mysekaiRank` is the master rank of `totalExp`
//!   (the highest rank row whose `totalExp` is at most it); a panel edit of
//!   either field sets the other.
//! - *Harvest stamina*: the harvest reply's stamina is the rests the client
//!   sent, unless a panel edit of the stamina is pending; then the edit is
//!   the reply.
//! - *Gate reserve*: the reserved unit group joins the gate characters of the
//!   home gate (`isReservation` true, `visitCount` 1). *Gate change*: the
//!   home gate takes the new gate and skin and the stated unit groups become
//!   its characters (`isReservation` false, `visitCount` 1).
//! - *Daily schedule*: see [`document::SchedulePolicy::Daily`].
//!
//! **Persistence.** In the browser game the document and the local lists
//! live in the settings page backend beside the settings document and are
//! persisted with it. The native host keeps them in memory only, so a
//! harness run starts from its own state every time.
//!
//! **Sources of the document.** The browser game's seed (its `server`
//! document, or the default on a first run). Natively: the asset source's
//! `server-panel/npc.json` when present (a schemaVersion 1 slice, migrated
//! with its fixed clock and stated rows), else the checked-in slice migrated
//! as the product default. A native-only overlay applies the menu mock's
//! instruments (`MOLY_MENU_MOCK_TOTAL_EXP`, `MOLY_MENU_MOCK_STAMINA_NORMAL`,
//! `_ENHANCE`, `_BOOST`) and the birthday gate's `MOLY_BIRTHDAY_NOW_MS` (a
//! fixed server clock) to the native document; game mode reads no
//! environment variable ([`instrument_env`]).

pub(crate) mod avatar;
pub(crate) mod client;
pub(crate) mod clock;
pub(crate) mod craft;
pub(crate) mod delivery;
pub(crate) mod document;
pub(crate) mod edit;
pub(crate) mod home_action;
pub(crate) mod housing_layout;
pub(crate) mod inventory;
pub(crate) mod local;
pub(crate) mod music;
pub(crate) mod music_play;
pub(crate) mod system_fixture_action;
pub(crate) mod talk_read;

use std::collections::VecDeque;
use std::sync::Mutex;

use bevy::asset::io::{AssetReaderError, AssetSourceId, Reader};
use bevy::asset::AssetPath;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, IoTaskPool, Task};
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_assets::remote::RemoteRegion;
use serde_json::{json, Value};

pub(crate) use document::{ColorfulPass, Gamedata, GateCharacter, ScheduleRow, Stamina};

use clock::RefreshPeriod;
use document::{Migration, SchedulePolicy, ServerDocument, StaminaRefresh};

/// Asset source and path of a native slice that replaces the checked-in one.
const SLICE_SOURCE: &str = "moly";
const SLICE_FILE: &str = "server-panel/npc.json";
/// The checked-in slice (the server panel's default).
const CHECKED_IN_SLICE: &str = include_str!("../server_panel/npc.json");

const PHENOMENA_INDEX: &str = "moly://phenomena/index.json";

/// Seconds between the running clock's refresh checks.
const TICK_SECONDS: f32 = 1.0;
const MAX_ERRORS: usize = 32;

/// The document sections a response carries.
pub(crate) const SECTION_GAMEDATA: &str = "userMysekaiGamedata";
pub(crate) const SECTION_STAMINA: &str = "userMysekaiStamina";
pub(crate) const SECTION_PASS: &str = "userMysekaiColorfulPass";

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

/// The master pools' maxima (`mysekaiStaminas.maxStamina` per type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StaminaMax {
    pub(crate) normal: i32,
    pub(crate) enhance: i32,
    pub(crate) boost: i32,
}

/// The master data the model reads. Each is `None` while loading or when
/// its table is missing ([`Masters::missing`] names those).
#[derive(Default, Debug)]
pub(crate) struct Masters {
    pub(crate) periods: Option<Vec<RefreshPeriod>>,
    /// Phenomenon id and master name.
    pub(crate) phenomena: Option<Vec<(i32, String)>>,
    pub(crate) stamina_max: Option<StaminaMax>,
    pub(crate) boost_grant: Option<i32>,
    /// (rank, cumulative totalExp), master order.
    pub(crate) ranks: Option<Vec<(i32, i32)>>,
    /// Gate ids and gate skin ids.
    pub(crate) gates: Option<(Vec<i32>, Vec<i32>)>,
    /// `mysekaiGates` ids, until the skin ids join them in `gates`.
    gate_ids: Option<Vec<i32>>,
    /// `mysekaiGateSkins` ids, until they join the gate ids in `gates`.
    gate_skin_ids: Option<Vec<i32>>,
    /// The birthday-party delivery tables.
    pub(crate) delivery: Option<delivery::DeliveryTables>,
    /// The delivery tables as they resolve, until all three are in.
    delivery_parts: delivery::DeliveryParts,
    /// The total reward boxes (deferred): what each granted box holds.
    pub(crate) reward_boxes: Deferred<Vec<(i64, delivery::Reward)>>,
    /// The master configs (`configs`): configKey -> value.
    pub(crate) configs: Option<std::collections::BTreeMap<String, String>>,
    /// The music record ids (`mysekaiMusicRecords`).
    pub(crate) music_records: Option<Vec<i32>>,
    /// The avatar wear masters.
    pub(crate) avatar: client::avatar::AvatarMasters,
    /// The craft masters (blueprints, costs, terms, first-craft bonus,
    /// material types, the white blueprint item).
    pub(crate) craft: client::craft::CraftMasters,
    /// The fixture and material possession masters.
    pub(crate) possession: client::inventory::PossessionMasters,
    /// The cards master (deferred): card id -> its character and rarity.
    pub(crate) cards: Deferred<std::collections::BTreeMap<i32, craft::CardRow>>,
    pub(crate) missing: Vec<String>,
}

/// A master table the model loads in the background once the join's tables
/// are in (the two large ones: the total reward boxes and the cards), and
/// why it is not there yet. The join does not wait for it; a request that
/// needs it before it arrives is refused by name.
#[derive(Debug)]
pub(crate) enum Deferred<T> {
    /// Not requested yet: the join's tables are still loading.
    Waiting,
    /// Requested (the upstream table name); not arrived yet.
    Loading(&'static str),
    /// The master layer could not give it (named once there).
    Failed(String),
    Ready(T),
}

impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self::Waiting
    }
}

impl<T> Deferred<T> {
    /// The table, or the refusal: `refused` says what cannot be done, and
    /// the reason says why the table is not there.
    pub(crate) fn get(&self, refused: &str) -> Result<&T, String> {
        match self {
            Self::Waiting => Err(format!(
                "{refused}: its master table is requested once the join's tables are in"
            )),
            Self::Loading(table) => Err(format!(
                "{refused}: the {table} master table is still loading"
            )),
            Self::Failed(reason) => Err(format!("{refused}: {reason}")),
            Self::Ready(value) => Ok(value),
        }
    }
}

impl Masters {
    /// Joins the tables that resolve apart: the gate and gate skin ids, and
    /// the three delivery tables.
    fn join_parts(&mut self) {
        if self.gates.is_none() {
            if let (Some(gates), Some(skins)) = (&self.gate_ids, &self.gate_skin_ids) {
                self.gates = Some((gates.clone(), skins.clone()));
            }
        }
        if self.delivery.is_none() {
            self.delivery = self.delivery_parts.complete();
        }
    }

    /// `MysekaiRankModel`'s rank of a total experience.
    pub(crate) fn rank_of(&self, total_exp: i32) -> Option<i32> {
        self.ranks
            .as_ref()?
            .iter()
            .filter(|(_, exp)| *exp <= total_exp)
            .map(|(rank, _)| *rank)
            .max()
    }

    pub(crate) fn exp_of(&self, rank: i32) -> Option<i32> {
        self.ranks
            .as_ref()?
            .iter()
            .find(|(row, _)| *row == rank)
            .map(|(_, exp)| *exp)
    }
}

// ---------------------------------------------------------------------------
// Responses and replies
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) enum ResponseKind {
    Join,
    Harvest,
    Gather,
    GateReserve,
    GateChange,
    Refresh,
    Sync,
    BirthdayPartyDelivery,
    BirthdayPartyGather,
    BirthdayPartySeat,
    HomeActionCraft,
    HomeActionCanvas,
    HomeActionSketch,
    HousingLayout,
    CharacterTalkRead,
    MusicPlaySet,
    MusicPlayEject,
    SystemFixtureAction,
}

impl ResponseKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Join => "PostUserMysekaiApi (join)",
            Self::Harvest => "PostUserMysekaiHarvestApi",
            Self::Gather => "PostUserMysekaiGatherApi",
            Self::GateReserve => "PostUserMysekaiGateReserveApi (panel action)",
            Self::GateChange => "PostUserMysekaiGateChangeApi (panel action)",
            Self::Refresh => "refresh (the server clock entered a new window)",
            Self::Sync => "sync (panel action)",
            Self::BirthdayPartyDelivery => "PutUserMysekaiBirthdayPartyDeliveryApi",
            Self::BirthdayPartyGather => "PutUserMysekaiBirthdayPartyGatherApi",
            Self::HomeActionCraft => "PostUserMysekaiCraftApi (craft)",
            Self::HomeActionCanvas => "PostUserMysekaiCraftApi (canvas)",
            Self::HomeActionSketch => "PostUserMysekaiHousingSketchApi",
            Self::HousingLayout => "PostUserMysekaiHousingLayoutApi",
            Self::CharacterTalkRead => "PutUserMysekaiCharacterTalkReadApi",
            Self::BirthdayPartySeat => {
                "user data (the rows of the birthday parties now in session)"
            }
            Self::MusicPlaySet => "PutUserMysekaiMusicPlaySetApi",
            Self::MusicPlayEject => "PutUserMysekaiMusicPlayEjectApi",
            Self::SystemFixtureAction => "PostUserMysekaiSystemFixtureActionApi",
        }
    }
}

/// One response as the client receives it: the server date and the user
/// data sections it carries.
#[derive(Clone, Debug)]
pub(crate) struct ServerResponse {
    pub(crate) kind: ResponseKind,
    pub(crate) server_date_ms: i64,
    pub(crate) is_refreshed: bool,
    pub(crate) gamedata: Option<Gamedata>,
    pub(crate) stamina: Option<Stamina>,
    pub(crate) colorful_pass: Option<Option<ColorfulPass>>,
    /// The rows the schedule write reads (join and refresh).
    pub(crate) schedules: Option<Vec<ScheduleRow>>,
    /// The birthday-party delivery sections it carries.
    pub(crate) delivery: delivery::DeliveryUpdate,
    /// `userMysekaiMusicPlayFixtureSettings` when it carries them.
    pub(crate) music: Option<Vec<client::music::MusicPlaySetting>>,
    /// `userAvatar` when it carries it.
    pub(crate) avatar: Option<client::avatar::UserAvatar>,
    /// The owned MySekai tables it carries (materials, their possession,
    /// fixtures, canvases, blueprints, items, the gamedata's levels).
    pub(crate) inventory: client::inventory::SuiteUserSections,
    /// `userMysekaiMusicRecords` when it carries them.
    pub(crate) music_records: Option<Vec<client::music_play::OwnedMusicRecord>>,
    /// `userMysekaiSystemFixtureActions` when it carries them.
    pub(crate) system_fixture_actions:
        Option<Vec<client::system_fixture_action::UserMysekaiSystemFixtureAction>>,
}

/// A reply's talk list (`mysekaiCharacterTalkWithReadHistories`): the rows
/// the document states, or the talk-list policy's list, which the server
/// panel builds for the new visitors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReplyTalkList {
    Stated(Vec<(i32, bool)>),
    Policy,
}

/// A gate reply for the gate presenter.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) enum GateReply {
    /// The reserve reply: the reserved row and the reply's talk list.
    Reserve {
        row: GateCharacter,
        talks: ReplyTalkList,
    },
    /// The change reply: the home gate's new gate and skin, its characters
    /// and the reply's talk list.
    Change {
        gate_id: i32,
        skin_id: i32,
        rows: Vec<GateCharacter>,
        talks: ReplyTalkList,
    },
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

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Seed,
    SeedDefault,
    AssetSource,
    CheckedInDefault,
}

impl Origin {
    fn name(self) -> &'static str {
        match self {
            Self::Seed => "the seed's server document",
            Self::SeedDefault => "the product default (first run)",
            Self::AssetSource => "the asset source's schemaVersion 1 slice",
            Self::CheckedInDefault => "the checked-in slice as the product default",
        }
    }
}

pub(crate) struct ServerModel {
    pub(super) doc: ServerDocument,
    pub(super) origin: Origin,
    /// Sections the next response carries.
    pub(super) pending: Vec<String>,
    pub(super) masters: Masters,
    pub(super) masters_ready: bool,
    pub(super) joined: bool,
    /// Counts document changes.
    pub(super) revision: u64,
    /// The rows the client last received.
    pub(super) served: Vec<ScheduleRow>,
    pub(super) live_changed: bool,
    pub(super) responses: Vec<ServerResponse>,
    pub(super) gate_replies: Vec<GateReply>,
    pub(super) errors: Vec<String>,
    /// The client copies as last delivered (for the panel's document view).
    pub(super) client: Value,
    /// The page backend persists the document.
    pub(super) persist: bool,
    /// The birthday parties in session at the server clock, as the server
    /// last seated them.
    pub(super) party_masters: Vec<delivery::PartyMaster>,
    /// Why no party can be in session: the birthday parties table is
    /// missing.
    pub(super) parties_missing: Option<String>,
    /// Home-action requests answered (craft, canvas, sketch).
    pub(super) home_action_replies: [u64; 3],
    /// The native avatar instrument, applied once the masters resolve.
    pub(super) avatar_instrument: avatar::AvatarInstrument,
    /// Music player requests answered (set, eject).
    pub(super) music_play_replies: [u64; 2],
}

static MODEL: Mutex<Option<ServerModel>> = Mutex::new(None);
/// The seed's local document text, taken once by the local lists' seat.
static LOCAL_SEED: Mutex<Option<String>> = Mutex::new(None);

/// `None` when no model is installed.
pub(crate) fn with_model<R>(operation: impl FnOnce(&mut ServerModel) -> R) -> Option<R> {
    MODEL.lock().ok()?.as_mut().map(operation)
}

fn install(model: ServerModel) {
    info!(
        "[server] document installed from {} (clock {}, schedule policy {}, stamina {:?}, rank {:?}, totalExp {})",
        model.origin.name(),
        document::clock_value(model.doc.clock),
        match &model.doc.schedule_policy {
            SchedulePolicy::Stated => "stated".to_owned(),
            SchedulePolicy::Daily(map) => format!("daily {map:?}"),
        },
        model.doc.stamina,
        model.doc.gamedata.mysekai_rank,
        model.doc.gamedata.total_exp,
    );
    if let Ok(mut slot) = MODEL.lock() {
        *slot = Some(model);
    }
}

impl ServerModel {
    fn new(doc: ServerDocument, origin: Origin, pending: Vec<String>, persist: bool) -> Self {
        Self {
            doc,
            origin,
            pending,
            masters: Masters::default(),
            masters_ready: false,
            joined: false,
            revision: 0,
            served: Vec::new(),
            live_changed: false,
            responses: Vec::new(),
            gate_replies: Vec::new(),
            errors: Vec::new(),
            client: Value::Null,
            persist,
            party_masters: Vec::new(),
            parties_missing: None,
            home_action_replies: [0; 3],
            avatar_instrument: avatar::AvatarInstrument::default(),
            music_play_replies: [0; 2],
        }
    }

    pub(crate) fn document(&self) -> &ServerDocument {
        &self.doc
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn errors(&self) -> &[String] {
        &self.errors
    }

    pub(super) fn push_error(&mut self, error: String) {
        if self.errors.len() < MAX_ERRORS && !self.errors.contains(&error) {
            error!("[server] {error}");
            self.errors.push(error);
        }
    }

    pub(crate) fn now_ms(&self) -> i64 {
        clock::server_now_ms(self.doc.clock)
    }

    /// The document text the page persists.
    pub(crate) fn text(&self) -> String {
        self.doc.to_value(&self.pending).to_string()
    }

    /// After every document change: count it and hand it to the page.
    pub(super) fn commit(&mut self) {
        self.revision += 1;
        if self.persist {
            if let Err(error) = crate::settings_store::commit_page_document("server", self.text()) {
                warn!("[server] the server document is not persisted: {error}");
            }
        }
    }

    pub(super) fn mark_pending(&mut self, section: &str) {
        if !self.pending.iter().any(|known| known == section) {
            self.pending.push(section.to_owned());
        }
    }

    /// The rows the schedule is served with at `now`.
    pub(super) fn served_rows(&mut self, now: i64) -> Vec<ScheduleRow> {
        match &self.doc.schedule_policy {
            SchedulePolicy::Stated => self.doc.schedules.clone(),
            SchedulePolicy::Daily(map) => match &self.masters.periods {
                Some(periods) => clock::daily_rows(now, periods, map, clock::device_utc_offset_ms),
                None => {
                    self.push_error(
                        "the daily schedule needs the master refresh windows (phenomena index); no rows are served".into(),
                    );
                    Vec::new()
                }
            },
        }
    }

    /// The server's refresh check at `now`: a new refresh window since
    /// `refreshedAt` refreshes the user.
    /// A clock set behind the last refresh counts as entering a new window
    /// too (the server's record is never in its own future).
    pub(super) fn refresh_due(&self, now: i64) -> bool {
        let Some(periods) = &self.masters.periods else {
            return false;
        };
        now < self.doc.gamedata.refreshed_at
            || clock::window_start(now, periods, clock::device_utc_offset_ms)
                .is_some_and(|start| self.doc.gamedata.refreshed_at < start)
    }

    /// The refresh: `refreshedAt` := now; the stamina refresh policy.
    pub(super) fn refresh(&mut self, now: i64) {
        let before = self.doc.gamedata.refreshed_at;
        self.doc.gamedata.refreshed_at = now;
        let refill = match (
            self.doc.stamina_refresh,
            self.masters.stamina_max,
            &mut self.doc.stamina,
        ) {
            (StaminaRefresh::RefillNormal, Some(max), Some(stamina)) => {
                let from = stamina.normal;
                stamina.normal = max.normal;
                format!(
                    "normal {from} -> {} ({})",
                    max.normal,
                    document::REFILL_POLICY_TEXT
                )
            }
            (StaminaRefresh::RefillNormal, None, _) => {
                "no refill: the stamina masters are absent".to_owned()
            }
            (StaminaRefresh::RefillNormal, _, None) => {
                "no refill: the pools are not seated".to_owned()
            }
            (StaminaRefresh::None, _, _) => "no refill (policy none)".to_owned(),
        };
        info!("[server] refresh at {now}: refreshedAt {before} -> {now}; {refill}");
        self.commit();
    }

    /// Seats what a new user lacks: the pools (initial stamina policy) and
    /// the rank (rank from experience policy).
    fn seat(&mut self) {
        let mut changed = false;
        if self.doc.stamina.is_none() {
            match self.masters.stamina_max {
                Some(max) => {
                    let boost = self.masters.boost_grant.unwrap_or(0);
                    self.doc.stamina = Some(Stamina {
                        normal: max.normal,
                        enhance: 0,
                        boost,
                    });
                    info!("[server] initial stamina: normal {} (master maximum), enhance 0, boost {boost} (one recovery grant)", max.normal);
                    changed = true;
                }
                None => self.push_error(
                    "userMysekaiStamina is not seated: the stamina masters are absent".into(),
                ),
            }
        }
        if self.doc.gamedata.mysekai_rank.is_none() {
            match self.masters.rank_of(self.doc.gamedata.total_exp) {
                Some(rank) => {
                    self.doc.gamedata.mysekai_rank = Some(rank);
                    info!(
                        "[server] mysekaiRank {rank} from totalExp {} through the master rank table",
                        self.doc.gamedata.total_exp
                    );
                    changed = true;
                }
                None => self.push_error(format!(
                    "userMysekaiGamedata.mysekaiRank is not seated: no master rank table row for totalExp {}",
                    self.doc.gamedata.total_exp
                )),
            }
        }
        if changed {
            self.commit();
        }
    }

    /// A response of `kind`: the sections it carries by its kind plus the
    /// pending ones, with the document's current values.
    pub(super) fn respond(&mut self, kind: ResponseKind, is_refreshed: bool, carries: &[&str]) {
        let now = self.now_ms();
        let mut sections: Vec<String> = carries.iter().map(|s| (*s).to_owned()).collect();
        for section in self.pending.drain(..) {
            if !sections.contains(&section) {
                sections.push(section);
            }
        }
        let has = |name: &str| sections.iter().any(|section| section == name);
        let schedules = matches!(kind, ResponseKind::Join | ResponseKind::Refresh).then(|| {
            let rows = self.served_rows(now);
            self.served = rows.clone();
            rows
        });
        let response = ServerResponse {
            kind,
            server_date_ms: now,
            is_refreshed,
            gamedata: has(SECTION_GAMEDATA).then_some(self.doc.gamedata),
            stamina: if has(SECTION_STAMINA) {
                self.doc.stamina
            } else {
                None
            },
            colorful_pass: has(SECTION_PASS).then_some(self.doc.colorful_pass),
            schedules,
            delivery: self.delivery_update(&sections),
            music: has(music::SECTION).then(|| self.doc.music_settings.clone()),
            avatar: has(avatar::SECTION).then_some(self.doc.avatar),
            inventory: self.inventory_update(&sections),
            music_records: has(music_play::RECORDS_SECTION).then(|| self.doc.music_records.clone()),
            system_fixture_actions: has(system_fixture_action::SECTION)
                .then(|| self.doc.system_fixture_actions.rows.clone()),
        };
        info!(
            "[server] response {}: sections {sections:?}, isRefreshed {is_refreshed}, server date {now}",
            kind.name()
        );
        self.responses.push(response);
        // The pending list is part of the persisted text.
        self.commit();
    }

    /// The join: masters are in. Refresh when due, seat a new user, check
    /// the stored values against the masters, respond with every section.
    fn join(&mut self) {
        self.joined = true;
        let now = self.now_ms();
        self.seat();
        let refreshed = self.refresh_due(now);
        if refreshed {
            self.refresh(now);
        }
        if let Err(error) = edit::check_masters(&self.doc, &self.masters) {
            self.push_error(format!(
                "the stored server document disagrees with the masters: {error} (kept as stored)"
            ));
        }
        if !self.avatar_instrument.is_empty() {
            let instrument = std::mem::take(&mut self.avatar_instrument);
            let (avatar, unresolved) = instrument.resolve(&self.masters.avatar);
            for line in unresolved {
                self.push_error(format!("native avatar instrument: {line}"));
            }
            info!("[server] native overlay: MOLY_AVATAR_MOCK_* -> userAvatar {avatar:?}");
            self.doc.avatar = avatar;
            self.commit();
        }
        let mut carries = vec![SECTION_GAMEDATA, SECTION_STAMINA, SECTION_PASS];
        carries.extend(delivery::SECTIONS);
        carries.extend([music::SECTION, avatar::SECTION]);
        carries.extend(inventory::SECTIONS);
        carries.push(music_play::RECORDS_SECTION);
        carries.push(system_fixture_action::SECTION);
        self.respond(ResponseKind::Join, refreshed, &carries);
    }

    /// The running clock's check.
    fn tick(&mut self) {
        let now = self.now_ms();
        if self.refresh_due(now) {
            self.refresh(now);
            self.respond(
                ResponseKind::Refresh,
                true,
                &[SECTION_GAMEDATA, SECTION_STAMINA],
            );
        }
    }

    /// The panel's live sections changed: serve the schedule again.
    pub(super) fn live_update(&mut self) {
        let now = self.now_ms();
        if self.joined && self.refresh_due(now) {
            self.refresh(now);
            self.respond(
                ResponseKind::Refresh,
                true,
                &[SECTION_GAMEDATA, SECTION_STAMINA],
            );
            return;
        }
        self.served = self.served_rows(now);
        self.live_changed = true;
    }
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

/// The seed's server document, checked before anything is installed: a
/// newer or unknown schemaVersion, or a malformed document, is refused.
pub(crate) fn check_seed_document(text: &str) -> Result<(), String> {
    document::parse_v2(text).map(|_| ())
}

/// The seed's local document, checked likewise.
pub(crate) fn check_seed_local(text: &str) -> Result<(), String> {
    local::LocalDocument::parse(text).map(|_| ())
}

/// The browser game: the seed's documents (checked by the seed parser),
/// persisted through the page backend.
pub(crate) fn install_from_seed(server: Option<String>, local_text: Option<String>) {
    // The page's game mode: every native instrument is off from here on.
    client::enter_game_mode();
    let model = match server {
        Some(text) => {
            let (doc, pending) = document::parse_v2(&text).unwrap_or_else(|reason| {
                panic!("server document passed the seed check but is refused: {reason}")
            });
            ServerModel::new(doc, Origin::Seed, pending, true)
        }
        None => {
            let doc = document::migrate_v1(CHECKED_IN_SLICE, Migration::CheckedInDefault)
                .unwrap_or_else(|reason| panic!("checked-in server panel slice: {reason}"));
            ServerModel::new(doc, Origin::SeedDefault, Vec::new(), true)
        }
    };
    install(model);
    if let Ok(mut slot) = LOCAL_SEED.lock() {
        *slot = local_text;
    }
}

/// Whether a model is installed.
pub(crate) fn installed() -> bool {
    with_model(|_| ()).is_some()
}

/// The native instruments' environment variables (read by the owners'
/// seams as `crate::server::instrument_env`; the function lives with the
/// client side).
pub(crate) use client::instrument_env;

/// The native overlay of the groups' instruments onto the document; the
/// avatar instrument names bundles and a colour, so it waits for the masters.
fn native_overlay(doc: &mut ServerDocument) -> avatar::AvatarInstrument {
    let int = |name: &str| -> Option<i32> {
        let raw = instrument_env(name)?;
        match raw.trim().parse::<i32>() {
            Ok(value) => Some(value),
            Err(_) => {
                warn!("[server] {name}={raw:?} is not an integer; the document keeps its value");
                None
            }
        }
    };
    if let Some(exp) = int("MOLY_MENU_MOCK_TOTAL_EXP") {
        info!("[server] native overlay: MOLY_MENU_MOCK_TOTAL_EXP -> userMysekaiGamedata.totalExp {exp} (the rank follows it)");
        doc.gamedata.total_exp = exp;
        doc.gamedata.mysekai_rank = None;
    }
    let pools = [
        int("MOLY_MENU_MOCK_STAMINA_NORMAL"),
        int("MOLY_MENU_MOCK_STAMINA_ENHANCE"),
        int("MOLY_MENU_MOCK_STAMINA_BOOST"),
    ];
    if pools.iter().any(Option::is_some) {
        let base = doc.stamina.unwrap_or(Stamina {
            normal: 0,
            enhance: 0,
            boost: 0,
        });
        let stamina = Stamina {
            normal: pools[0].unwrap_or(base.normal),
            enhance: pools[1].unwrap_or(base.enhance),
            boost: pools[2].unwrap_or(base.boost),
        };
        info!(
            "[server] native overlay: MOLY_MENU_MOCK_STAMINA_* -> userMysekaiStamina {stamina:?}"
        );
        doc.stamina = Some(stamina);
    }
    if let Some(raw) = instrument_env("MOLY_BIRTHDAY_NOW_MS") {
        let at_ms = raw
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|at| *at > 0)
            .unwrap_or_else(|| {
                panic!("MOLY_BIRTHDAY_NOW_MS={raw:?} is not a positive epoch millisecond")
            });
        info!(
            "[server] native overlay: MOLY_BIRTHDAY_NOW_MS -> the server clock is fixed at {at_ms}"
        );
        doc.clock = document::Clock::Fixed { at_ms };
    }
    if instrument_env("MOLY_MENU_MOCK_STAMINA_MAX").is_some() {
        warn!("[server] MOLY_MENU_MOCK_STAMINA_MAX is not applied: the gauge maximum is the master maxStamina");
    }
    const MUSIC: &str = "MOLY_AUDIO_MOCK_MUSIC_RECORD";
    if let Some(raw) = instrument_env(MUSIC) {
        doc.music_settings = music::parse_instrument(MUSIC, &raw);
        info!(
            "[server] native overlay: {MUSIC} -> userMysekaiMusicPlayFixtureSettings ({} sites)",
            doc.music_settings.len()
        );
    }
    inventory::native_overlay(doc);
    music_play::native_overlay(doc);
    let text = |name: &str| {
        instrument_env(name)
            .map(|raw| raw.trim().to_owned())
            .filter(|raw| !raw.is_empty())
    };
    avatar::AvatarInstrument {
        coordinate: text("MOLY_AVATAR_MOCK_COORDINATE"),
        costume: text("MOLY_AVATAR_MOCK_COSTUME"),
        accessory: text("MOLY_AVATAR_MOCK_ACCESSORY"),
        skin_color: text("MOLY_AVATAR_MOCK_SKIN_COLOR"),
    }
}

// ---------------------------------------------------------------------------
// Reads for the other owners
// ---------------------------------------------------------------------------

/// The server panel's NPC slice (schemaVersion 1) once the client has
/// joined: the rows served at the join and the client's `refreshedAt`.
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn npc_slice(client: &ClientUserData, live: &LiveSchedule) -> Option<String> {
    with_model(|model| {
        model
            .doc
            .npc_slice(&live.schedules, client.gamedata.refreshed_at)
            .to_string()
    })
}

/// The server clock's epoch millisecond, once a model is installed.
pub(crate) fn server_now_ms() -> Option<i64> {
    with_model(|model| model.now_ms())
}

/// The home gate as the document holds it (gate id, skin id).
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn home_gate() -> Option<(i32, i32)> {
    with_model(|model| (model.doc.gate.gate_id, model.doc.gate.skin_id))
}

/// The master normal pool maximum (the menu gauge's maximum).
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn stamina_max() -> Option<StaminaMax> {
    with_model(|model| model.masters.stamina_max).flatten()
}

/// The harvest reply's stamina (harvest stamina policy); a response. The
/// sent rests come back when no model is installed.
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn harvest_api_stamina(rests: [i32; 3]) -> Stamina {
    let sent = Stamina {
        normal: rests[0],
        enhance: rests[1],
        boost: rests[2],
    };
    with_model(|model| {
        if !model.pending.iter().any(|section| section == SECTION_STAMINA) {
            if model.doc.stamina != Some(sent) {
                model.doc.stamina = Some(sent);
                model.commit();
            }
        } else {
            info!("[server] harvest reply: a panel edit of the stamina is pending; the reply carries it instead of the sent rests {sent:?}");
        }
        model.respond(ResponseKind::Harvest, false, &[SECTION_STAMINA]);
        model.doc.stamina.unwrap_or(sent)
    })
    .unwrap_or(sent)
}

/// The gather reply's stamina; a response. `None` without a model or with
/// unseated pools.
#[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
pub(crate) fn gather_api_stamina() -> Option<Stamina> {
    with_model(|model| {
        model.respond(ResponseKind::Gather, false, &[SECTION_STAMINA]);
        model.doc.stamina
    })
    .flatten()
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// The native document read, while it runs.
#[derive(Resource)]
struct SliceRead(Task<Result<Option<String>, String>>);

type Parser = fn(&str, &mut Masters) -> Result<(), String>;

/// A root document's read, while it runs: `Ok(None)` when the asset source
/// does not hold the file.
type RootRead = Task<Result<Option<String>, String>>;

/// The root documents still reading: name, parser and the read.
#[derive(Resource)]
struct RootReads(Vec<(&'static str, Parser, RootRead)>);

/// A root document's text through the asset source. Reading it here rather
/// than through an asset load lets a document the runtime root lacks be only
/// the model's named missing entry, with no load error for the absent file.
async fn read_root_document(
    server: AssetServer,
    path: AssetPath<'static>,
) -> Result<Option<String>, String> {
    let source = server
        .get_source(path.source().clone_owned())
        .map_err(|error| error.to_string())?;
    let mut reader = match source.reader().read(path.path()).await {
        Ok(reader) => reader,
        Err(AssetReaderError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// A master table the model reads. The master layer parses it into a fresh
/// [`Masters`] (the parser fills one field) and `seat` moves that field into
/// the model's.
struct ServerTable {
    table: MasterTable<Masters>,
    seat: fn(&mut Masters, Masters),
    /// The region whose master carries the table; `None`: every region.
    region: Option<RemoteRegion>,
    /// A deferred table's field markers; `None`: the join waits for it.
    deferred: Option<DeferredSeat>,
}

/// How a deferred table's field in the model says where it is.
struct DeferredSeat {
    /// Requested: the field reads as loading.
    requested: fn(&mut Masters, &'static str),
    /// The master layer could not give it: the field carries the reason.
    failed: fn(&mut Masters, String),
}

/// A [`ServerTable`] whose parser fills the given field of [`Masters`].
macro_rules! server_table {
    ($table:literal, $name:literal, $parse:path, $($field:ident).+) => {
        ServerTable {
            table: server_table!(@table $table, $name, $parse),
            seat: |masters, partial| masters.$($field).+ = partial.$($field).+,
            region: None,
            deferred: None,
        }
    };
    (@table $table:literal, $name:literal, $parse:path) => {
        MasterTable {
            table: $table,
            name: $name,
            parse: |text| {
                let mut partial = Masters::default();
                $parse(text, &mut partial)?;
                Ok(partial)
            },
        }
    };
}

/// A deferred [`ServerTable`]: its parser fills the given [`Deferred`] field.
macro_rules! deferred_table {
    ($table:literal, $name:literal, $parse:path, $field:ident, $region:expr) => {
        ServerTable {
            table: server_table!(@table $table, $name, $parse),
            seat: |masters, partial| masters.$field = partial.$field,
            region: $region,
            deferred: Some(DeferredSeat {
                requested: |masters, table| masters.$field = Deferred::Loading(table),
                failed: |masters, reason| masters.$field = Deferred::Failed(reason),
            }),
        }
    };
}

/// The model's master tables. The join waits for all but the deferred ones,
/// which are requested once the others are in and seated when they arrive.
static SERVER_TABLES: &[ServerTable] = &[
    server_table!(
        "mysekaiStaminas",
        "mysekaiStaminas (the server's pool maxima)",
        parse_staminas,
        stamina_max
    ),
    server_table!(
        "mysekaiStaminaRecovery",
        "mysekaiStaminaRecovery (the server's boost grant)",
        parse_recovery,
        boost_grant
    ),
    server_table!(
        "mysekaiRanks",
        "mysekaiRanks (the server's rank table)",
        parse_ranks,
        ranks
    ),
    server_table!(
        "mysekaiGates",
        "mysekaiGates (the server's gate ids)",
        parse_gates,
        gate_ids
    ),
    server_table!(
        "mysekaiGateSkins",
        "mysekaiGateSkins (the server's gate skin ids)",
        parse_gate_skins,
        gate_skin_ids
    ),
    server_table!(
        "configs",
        "configs (master configs; without it the delivery reads policies.masterConfigsStandIn)",
        parse_configs,
        configs
    ),
    server_table!(
        "mysekaiMusicRecords",
        "mysekaiMusicRecords (the server's music records)",
        music::parse_records,
        music_records
    ),
    server_table!(
        "avatarCostumes",
        "avatarCostumes (the server's avatar costumes)",
        avatar::parse_costumes,
        avatar.costumes
    ),
    server_table!(
        "avatarAccessories",
        "avatarAccessories (the server's avatar accessories)",
        avatar::parse_accessories,
        avatar.accessories
    ),
    server_table!(
        "avatarSkinColors",
        "avatarSkinColors (the server's avatar skin colours)",
        avatar::parse_skin_colors,
        avatar.skin_colors
    ),
    server_table!(
        "avatarCoordinates",
        "avatarCoordinates (the server's avatar coordinates)",
        avatar::parse_coordinates,
        avatar.coordinates
    ),
    server_table!(
        "mysekaiBlueprints",
        "mysekaiBlueprints (the server's blueprints)",
        craft::parse_blueprints,
        craft.blueprints
    ),
    server_table!(
        "mysekaiBlueprintMysekaiMaterialCosts",
        "mysekaiBlueprintMysekaiMaterialCosts (the server's blueprint material costs)",
        craft::parse_costs,
        craft.costs
    ),
    server_table!(
        "mysekaiBlueprintTerms",
        "mysekaiBlueprintTerms (the server's blueprint craft terms)",
        craft::parse_terms,
        craft.terms
    ),
    server_table!(
        "mysekaiRankObtainedExps",
        "mysekaiRankObtainedExps (the server's first-craft bonus)",
        craft::parse_rank_obtained_exps,
        craft.first_craft_bonus
    ),
    server_table!(
        "mysekaiFixturePossessions",
        "mysekaiFixturePossessions (the server's fixture possession limits)",
        inventory::parse_fixture_possessions,
        possession.fixture
    ),
    server_table!(
        "mysekaiMaterialPossessions",
        "mysekaiMaterialPossessions (the server's material possession limits)",
        inventory::parse_material_possessions,
        possession.material
    ),
    server_table!(
        "mysekaiMaterials",
        "mysekaiMaterials (the server's material types)",
        inventory::parse_materials,
        craft.material_types
    ),
    server_table!(
        "mysekaiItems",
        "mysekaiItems (the server's white blueprint item)",
        inventory::parse_items_master,
        craft.white_blueprint_item
    ),
    server_table!(
        "birthdayPartyDeliveryRewards",
        "birthdayPartyDeliveryRewards (the delivery's reward rows)",
        delivery::parse_rewards,
        delivery_parts.rewards
    ),
    server_table!(
        "birthdayPartyDeliveryPointBonuses",
        "birthdayPartyDeliveryPointBonuses (the delivery's point bonus rows)",
        delivery::parse_point_bonuses,
        delivery_parts.bonuses
    ),
    server_table!(
        "birthdayPartyDeliveryTotalRewards",
        "birthdayPartyDeliveryTotalRewards (the delivery's total reward rows)",
        delivery::parse_total_rewards,
        delivery_parts.totals
    ),
    // The total reward boxes: the CN master carries their rows as a table of
    // their own; the JP master nests them in each box's `details`.
    deferred_table!(
        "resourceBoxDetails",
        "resourceBoxDetails (the delivery's total reward boxes)",
        delivery::parse_box_details,
        reward_boxes,
        Some(RemoteRegion::Cn)
    ),
    deferred_table!(
        "resourceBoxes",
        "resourceBoxes (the delivery's total reward boxes)",
        delivery::parse_nested_boxes,
        reward_boxes,
        Some(RemoteRegion::Jp)
    ),
    deferred_table!(
        "cards",
        "cards (the canvas craft's card character and rarity)",
        craft::parse_cards,
        cards,
        None
    ),
];

/// The model's master tables not seated yet, and whether the deferred ones
/// have been requested.
#[derive(Resource)]
struct MasterSeats {
    pending: Vec<&'static ServerTable>,
    deferred_requested: bool,
}

async fn read_slice(server: AssetServer) -> Result<Option<String>, String> {
    let source = server
        .get_source(AssetSourceId::from(SLICE_SOURCE))
        .map_err(|error| error.to_string())?;
    let mut reader = match source.reader().read(std::path::Path::new(SLICE_FILE)).await {
        Ok(reader) => reader,
        Err(AssetReaderError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn load(mut commands: Commands, server: Res<AssetServer>, mut masters: ResMut<MasterData>) {
    if !installed() {
        let reader = server.clone();
        commands.insert_resource(SliceRead(
            IoTaskPool::get().spawn(async move { read_slice(reader).await }),
        ));
    }
    let reader = server.clone();
    commands.insert_resource(RootReads(vec![(
        "phenomena/index.json (refresh windows, phenomena)",
        parse_phenomena as Parser,
        IoTaskPool::get().spawn(async move {
            read_root_document(reader, AssetPath::from(PHENOMENA_INDEX)).await
        }),
    )]));
    // The deferred tables are requested once these are in.
    for table in SERVER_TABLES
        .iter()
        .filter(|table| table.deferred.is_none())
    {
        masters.request(&table.table);
    }
    commands.insert_resource(MasterSeats {
        pending: SERVER_TABLES.iter().collect(),
        deferred_requested: false,
    });
}

/// Native: the asset source's slice when present, else the checked-in one.
fn install_native(mut commands: Commands, read: Option<ResMut<SliceRead>>) {
    let Some(mut read) = read else {
        return;
    };
    let Some(result) = block_on(future::poll_once(&mut read.0)) else {
        return;
    };
    commands.remove_resource::<SliceRead>();
    let (text, migration, origin) = match result {
        Ok(Some(text)) => (text, Migration::AssetSource, Origin::AssetSource),
        Ok(None) => (
            CHECKED_IN_SLICE.to_owned(),
            Migration::CheckedInDefault,
            Origin::CheckedInDefault,
        ),
        Err(error) => {
            panic!("server document {SLICE_SOURCE}://{SLICE_FILE} failed to load: {error}")
        }
    };
    let mut doc = document::migrate_v1(&text, migration).unwrap_or_else(|reason| {
        panic!("server document ({}) is refused: {reason}", origin.name())
    });
    let instrument = native_overlay(&mut doc);
    let mut model = ServerModel::new(doc, origin, Vec::new(), false);
    model.avatar_instrument = instrument;
    install(model);
}

/// The rows of an upstream master table, in id order.
pub(super) fn master_rows(value: &Value, table: &str) -> Result<Vec<Value>, String> {
    let mut rows = value
        .as_array()
        .ok_or_else(|| format!("{table} is not an array of rows"))?
        .clone();
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    Ok(rows)
}

pub(super) fn int32(value: &Value, key: &str) -> Result<i32, String> {
    value[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("{key} of {value} is not an int"))
}

/// `configs`: configKey -> value (the value is a string in the master).
fn parse_configs(text: &str, masters: &mut Masters) -> Result<(), String> {
    let configs = master::rows(text)?
        .iter()
        .map(|row| {
            let key = master::text(row, "configKey")?;
            let text = match &row["value"] {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                other => {
                    return Err(format!(
                        "configs {key} has a value {other} that is not text"
                    ))
                }
            };
            Ok((key.to_owned(), text))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?;
    masters.configs = Some(configs);
    Ok(())
}

fn parse_phenomena(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let periods = value["refreshTimePeriods"]
        .as_array()
        .ok_or("no refreshTimePeriods")?
        .iter()
        .map(|row| {
            Ok(RefreshPeriod {
                id: int32(row, "id")?,
                start_hour: row["startHour"].as_i64().ok_or("startHour is not an int")?,
                end_hour: row["endHour"].as_i64().ok_or("endHour is not an int")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut phenomena = value["phenomena"]
        .as_object()
        .ok_or("no phenomena")?
        .values()
        .map(|row| {
            Ok((
                int32(row, "id")?,
                row["master"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    phenomena.sort_by_key(|(id, _)| *id);
    masters.periods = Some(periods);
    masters.phenomena = Some(phenomena);
    Ok(())
}

fn parse_staminas(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let rows = master_rows(&value, "mysekaiStaminas")?;
    let max = |kind: &str| -> Result<i32, String> {
        rows.iter()
            .find(|row| row["mysekaiStaminaType"].as_str() == Some(kind))
            .ok_or_else(|| format!("no {kind} stamina row"))
            .and_then(|row| int32(row, "maxStamina"))
    };
    masters.stamina_max = Some(StaminaMax {
        normal: max("normal")?,
        enhance: max("enhance")?,
        boost: max("boost")?,
    });
    Ok(())
}

/// `mysekaiStaminaRecovery`: one object.
fn parse_recovery(text: &str, masters: &mut Masters) -> Result<(), String> {
    let row = Value::Object(master::object(text)?);
    masters.boost_grant = Some(int32(&row, "recoveryBoostStamina")?);
    Ok(())
}

/// `mysekaiRanks`: (rank, cumulative totalExp), master order.
fn parse_ranks(text: &str, masters: &mut Masters) -> Result<(), String> {
    let ranks = master::rows(text)?
        .iter()
        .map(|row| Ok((int32(row, "mysekaiRank")?, int32(row, "totalExp")?)))
        .collect::<Result<Vec<_>, String>>()?;
    masters.ranks = Some(ranks);
    Ok(())
}

/// The ids of a master table, master order.
fn master_ids(text: &str) -> Result<Vec<i32>, String> {
    master::rows(text)?
        .iter()
        .map(|row| int32(row, "id"))
        .collect()
}

fn parse_gates(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.gate_ids = Some(master_ids(text)?);
    Ok(())
}

fn parse_gate_skins(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.gate_skin_ids = Some(master_ids(text)?);
    Ok(())
}

/// Each master as it resolves, once the model is installed; a missing or
/// malformed one (named once by the master layer, or here for a root
/// document) is listed and the model goes on without it.
fn resolve_masters(
    mut commands: Commands,
    reads: Option<ResMut<RootReads>>,
    seats: Option<ResMut<MasterSeats>>,
    mut masters: ResMut<MasterData>,
) {
    if !installed() {
        return;
    }
    let mut outcomes: Vec<(&'static str, Parser, Result<Option<String>, String>)> = Vec::new();
    let mut reads_done = true;
    if let Some(mut reads) = reads {
        reads.0.retain_mut(
            |(name, parser, read)| match block_on(future::poll_once(read)) {
                Some(outcome) => {
                    outcomes.push((*name, *parser, outcome));
                    false
                }
                None => true,
            },
        );
        if reads.0.is_empty() {
            commands.remove_resource::<RootReads>();
        } else {
            reads_done = false;
        }
    }
    let mut seated: Vec<(&'static ServerTable, _)> = Vec::new();
    let mut requested: Vec<&'static ServerTable> = Vec::new();
    let mut join_waits = false;
    if let Some(mut seats) = seats {
        seats
            .pending
            .retain(|table| match masters.take(&table.table) {
                Some(result) => {
                    seated.push((*table, result));
                    false
                }
                None => true,
            });
        join_waits = seats.pending.iter().any(|table| table.deferred.is_none());
        if !seats.deferred_requested && reads_done && !join_waits {
            // The join's tables are in: the deferred ones load in the
            // background now. Without a region (named by the master layer)
            // every candidate is requested and each fails with that reason.
            seats.deferred_requested = true;
            let region = masters.region();
            seats.pending.retain(|table| match (table.region, region) {
                (Some(only), Some(region)) => only == region,
                _ => true,
            });
            for table in &seats.pending {
                masters.request(&table.table);
                requested.push(*table);
            }
        }
        if seats.pending.is_empty() {
            commands.remove_resource::<MasterSeats>();
        }
    }
    if outcomes.is_empty() && seated.is_empty() && requested.is_empty() {
        return;
    }
    let requested_names: Vec<&str> = requested.iter().map(|table| table.table.table).collect();
    with_model(|model| {
        for (name, parser, outcome) in outcomes {
            let error = match outcome {
                Ok(Some(text)) => match parser(&text, &mut model.masters) {
                    Ok(()) => continue,
                    Err(error) => format!("malformed: {error}"),
                },
                Ok(None) => {
                    let error = "not in the runtime root".to_owned();
                    info!("[server] root document {name} is {error}");
                    model.masters.missing.push(format!("{name}: {error}"));
                    continue;
                }
                Err(error) => format!("unreadable: {error}"),
            };
            warn!("[server] root document {name} is {error}");
            model.masters.missing.push(format!("{name}: {error}"));
        }
        for (table, result) in seated {
            match result {
                Ok(partial) => (table.seat)(&mut model.masters, partial),
                Err(error) => {
                    if let Some(deferred) = &table.deferred {
                        (deferred.failed)(&mut model.masters, error.to_string());
                    }
                    model.masters.missing.push(error.to_string());
                }
            }
            if table.deferred.is_some() {
                info!("[server] deferred master {} seated", table.table.name);
            }
        }
        let had_delivery = model.masters.delivery.is_some();
        model.masters.join_parts();
        if !had_delivery {
            // The client reads the same delivery tables.
            if let Some(tables) = model.masters.delivery.clone() {
                commands.insert_resource(tables);
            }
        }
        if !model.masters_ready && reads_done && !join_waits {
            info!(
                "[server] masters resolved; missing: {:?}",
                model.masters.missing
            );
            model.masters_ready = true;
            // The client reads the same master tables.
            commands.insert_resource(model.masters.avatar.clone());
            commands.insert_resource(model.masters.craft.clone());
            commands.insert_resource(model.masters.possession.clone());
        }
        for table in requested {
            if let Some(deferred) = &table.deferred {
                (deferred.requested)(&mut model.masters, table.table.table);
            }
        }
        if !requested_names.is_empty() {
            info!("[server] deferred masters requested: {requested_names:?}");
        }
    });
}

/// The birthday parties in session at the server clock: the new party row
/// and delivery item stock policies, answered as a response when they seat
/// a row.
fn seat_birthday_parties(parties: Option<Res<crate::birthday::BirthdayParties>>) {
    let Some(parties) = parties else {
        return;
    };
    with_model(|model| {
        model.parties_missing = parties.missing().map(str::to_owned);
        if !model.joined {
            return;
        }
        let now = model.now_ms();
        let mut in_session = Vec::new();
        for row in parties.in_session(now) {
            let ids = (
                i32::try_from(row.id),
                i32::try_from(row.delivery_item_material_id),
                i32::try_from(row.delivery_reward_material_id),
            );
            match ids {
                (Ok(id), Ok(item), Ok(reward)) => in_session.push(delivery::PartyMaster {
                    birthday_party_id: id,
                    delivery_item_material_id: item,
                    delivery_reward_mysekai_material_id: reward,
                }),
                _ => model.push_error(format!(
                    "birthday party {} has an id that does not fit a 32-bit integer",
                    row.label
                )),
            }
        }
        if in_session == model.party_masters {
            return;
        }
        let changed = model.seat_parties(&in_session);
        info!(
            "[server] birthday parties in session at {now}: {:?}",
            in_session
                .iter()
                .map(|party| party.birthday_party_id)
                .collect::<Vec<_>>()
        );
        if !changed.is_empty() {
            model.commit();
            model.respond(ResponseKind::BirthdayPartySeat, false, &changed);
        }
    });
}

/// The join, once the model and its masters are in.
fn join() {
    with_model(|model| {
        if model.masters_ready && !model.joined {
            model.join();
        }
    });
}

fn tick(time: Res<Time<Real>>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed < TICK_SECONDS {
        return;
    }
    *elapsed = 0.0;
    with_model(|model| {
        if model.joined {
            model.tick();
        }
    });
}

/// The client side of the responses: the copies, the refresh topic, the
/// rank and the schedule the phenomenon of the day reads.
#[allow(clippy::too_many_arguments)]
fn deliver(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut client: Option<ResMut<ClientUserData>>,
    mut live: ResMut<LiveSchedule>,
    mut replies: ResMut<ServerGateReplies>,
    mut ranks: MessageWriter<RankDelivered>,
    mut local: Option<ResMut<crate::site_expansion::MysekaiLocalSettings>>,
    mut total_exp: Option<ResMut<crate::mysekai_rank::UserTotalExp>>,
    mut birthday: ResMut<delivery::ClientBirthdayPartyData>,
    mut music_copy: ResMut<client::music::ClientMusicPlaySettings>,
    mut avatar_copy: ResMut<client::avatar::ClientUserAvatar>,
    mut inventory_copy: ResMut<client::inventory::ClientMysekaiInventory>,
    mut records_copy: ResMut<client::music_play::ClientMusicRecords>,
    mut actions_copy: ResMut<client::system_fixture_action::ClientSystemFixtureActions>,
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
    for mut response in responses {
        birthday.apply(std::mem::take(&mut response.delivery));
        if let Some(rows) = response.music.take() {
            music_copy.apply(rows);
        }
        if let Some(avatar) = response.avatar.take() {
            avatar_copy.apply(avatar);
        }
        inventory_copy.apply(std::mem::take(&mut response.inventory));
        if let Some(rows) = response.music_records.take() {
            records_copy.apply(rows);
        }
        if let Some(rows) = response.system_fixture_actions.take() {
            actions_copy.apply(rows);
        }
        let previous_rank = copy.as_ref().and_then(|copy| copy.gamedata.mysekai_rank);
        let next = match copy.take() {
            None => ClientUserData {
                gamedata: response.gamedata.unwrap_or(Gamedata {
                    mysekai_rank: None,
                    total_exp: 0,
                    refreshed_at: 0,
                    is_mysekai_tutorial_end: true,
                }),
                stamina: response.stamina,
                colorful_pass: response.colorful_pass.flatten(),
                last_got_server_date: response.server_date_ms,
                application_time_at_last_got_server_date: realtime,
            },
            Some(mut copy) => {
                if let Some(gamedata) = response.gamedata {
                    copy.gamedata = gamedata;
                }
                if let Some(stamina) = response.stamina {
                    copy.stamina = Some(stamina);
                }
                if let Some(pass) = response.colorful_pass {
                    copy.colorful_pass = pass;
                }
                copy.last_got_server_date = response.server_date_ms;
                copy.application_time_at_last_got_server_date = realtime;
                copy
            }
        };
        if response.is_refreshed {
            if let Some(local) = local.as_deref_mut() {
                if !local.0.topics.contains(&local::TOPIC_STAMINA_REFRESH) {
                    local.0.topics.push(local::TOPIC_STAMINA_REFRESH);
                    info!("[server] {}: isRefreshed -> MysekaiTopicsManager.AddTopicIfNeeded adds StaminaRefresh", response.kind.name());
                }
            }
        }
        if let Some(gamedata) = response.gamedata {
            // UserMysekaiGamedata.totalExp is what both rank gauges read;
            // imported player data keeps its own value.
            if let Some(total) = total_exp.as_deref_mut() {
                if total.0 != gamedata.total_exp && crate::player_data::saved_total_exp().is_none()
                {
                    total.0 = gamedata.total_exp;
                }
            }
            if let Some(current) = gamedata.mysekai_rank {
                let join = response.kind == ResponseKind::Join;
                if join && crate::browser_game::game_mode_active() {
                    ranks.write(RankDelivered {
                        previous: None,
                        current,
                        response: response.kind.name(),
                    });
                } else if !join && previous_rank.is_some_and(|previous| previous != current) {
                    ranks.write(RankDelivered {
                        previous: previous_rank,
                        current,
                        response: response.kind.name(),
                    });
                }
            }
            if schedule.is_none()
                && response.schedules.is_none()
                && live.refreshed_at != gamedata.refreshed_at
            {
                schedule = Some(live.schedules.clone());
            }
        }
        if let Some(rows) = response.schedules {
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
        "userMysekaiStamina": copy.stamina.map(document::stamina_value),
        "hasMysekaiColorfulPass": copy.has_mysekai_colorful_pass(realtime),
        "currentTimestamp": copy.current_timestamp(realtime),
        "birthdayParty": birthday.view(),
        "userMysekaiMusicPlayFixtureSettings": music_copy.view(),
        "userAvatar": client::avatar::value(&avatar_copy.avatar),
        "mysekaiInventory": inventory_copy.view(),
        "userMysekaiMusicRecords": records_copy.view(),
    });
    with_model(|model| model.client = view);
    match client.as_deref_mut() {
        Some(slot) => *slot = copy,
        None => {
            commands.insert_resource(copy);
        }
    }
}

/// Startup: the local lists from the seed's local document (the browser
/// game); natively they start empty, as on a fresh install.
fn seat_local(local: Option<ResMut<crate::site_expansion::MysekaiLocalSettings>>) {
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

/// Last: a changed local list is written to the page backend.
fn persist_local(
    local: Option<Res<crate::site_expansion::MysekaiLocalSettings>>,
    mut written: Local<Option<String>>,
) {
    if !crate::browser_game::game_mode_active() {
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

pub(crate) struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RankDelivered>()
            .init_resource::<LiveSchedule>()
            .init_resource::<ServerGateReplies>()
            .init_resource::<ClientTalkListUpdates>()
            .init_resource::<delivery::ClientBirthdayPartyData>()
            .init_resource::<client::music::ClientMusicPlaySettings>()
            .init_resource::<client::avatar::ClientUserAvatar>()
            .init_resource::<client::music_play::ClientMusicRecords>()
            .add_systems(PreStartup, seat_local)
            .add_systems(Startup, load)
            .add_systems(
                PreUpdate,
                (
                    install_native,
                    resolve_masters,
                    join,
                    seat_birthday_parties,
                    tick,
                    deliver,
                )
                    .chain(),
            )
            .add_systems(Last, persist_local);
        let endpoint = app.world_mut().register_system(home_action::handle);
        app.insert_resource(client::home_action::HomeActionEndpoint(endpoint));
        let endpoint = app.world_mut().register_system(housing_layout::handle);
        app.insert_resource(client::housing_layout::HousingLayoutEndpoint(endpoint));
        craft::install(app);
        let endpoint = app.world_mut().register_system(music_play::handle);
        app.insert_resource(client::music_play::MusicPlayEndpoint(endpoint));
        system_fixture_action::install(app);
    }
}

// ---------------------------------------------------------------------------
// The page's views
// ---------------------------------------------------------------------------

/// `game_server_document()`: the document, the server clock, the client's
/// copies, the pending sections, the masters and the named refusals.
pub(crate) fn document_view() -> String {
    with_model(|model| {
        json!({
            "schemaVersion": document::SCHEMA_VERSION,
            "origin": model.origin.name(),
            "revision": model.revision,
            "serverNowMs": model.now_ms(),
            "document": model.doc.to_value(&model.pending),
            "pending": model.pending,
            "client": model.client,
            "joined": model.joined,
            "missingMasters": model.masters.missing,
            "homeActionReplies": {
                "craft": model.home_action_replies[0],
                "canvas": model.home_action_replies[1],
                "sketch": model.home_action_replies[2],
            },
            "musicPlayReplies": {
                "set": model.music_play_replies[0],
                "eject": model.music_play_replies[1],
            },
            "errors": model.errors,
        })
        .to_string()
    })
    .unwrap_or_else(|| json!({"schemaVersion": document::SCHEMA_VERSION, "document": null, "errors": ["the server model is not installed"]}).to_string())
}

/// `game_server_schema()`.
pub(crate) fn schema_view() -> String {
    with_model(|model| edit::schema(model).to_string())
        .unwrap_or_else(|| edit::schema_without_model().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn model() -> ServerModel {
        let doc = document::migrate_v1(CHECKED_IN_SLICE, Migration::CheckedInDefault).unwrap();
        let mut model = ServerModel::new(doc, Origin::CheckedInDefault, Vec::new(), false);
        model.masters.periods = Some(vec![
            RefreshPeriod {
                id: 1,
                start_hour: 5,
                end_hour: 17,
            },
            RefreshPeriod {
                id: 2,
                start_hour: 17,
                end_hour: 29,
            },
        ]);
        model.masters.stamina_max = Some(StaminaMax {
            normal: 1000,
            enhance: 1000,
            boost: 10000,
        });
        model.masters.boost_grant = Some(100);
        model.masters.ranks = Some(vec![(1, 0), (2, 48_000), (3, 172_300)]);
        model.masters.phenomena = Some(vec![(1, "a".into()), (2, "b".into())]);
        model.masters_ready = true;
        model
    }

    #[test]
    fn join_seats_a_new_user_and_refreshes() {
        let mut model = model();
        model.doc.clock = document::Clock::Fixed {
            at_ms: 1_790_300_000_000,
        };
        model.join();
        assert_eq!(
            model.doc.stamina,
            Some(Stamina {
                normal: 1000,
                enhance: 0,
                boost: 100
            })
        );
        assert_eq!(model.doc.gamedata.mysekai_rank, Some(3));
        assert_eq!(model.doc.gamedata.refreshed_at, 1_790_300_000_000);
        let response = model.responses.last().unwrap();
        assert!(response.is_refreshed);
        assert_eq!(response.schedules.as_ref().unwrap().len(), 6);
    }

    #[test]
    fn a_fixed_clock_at_refreshed_at_does_not_refresh() {
        let masters = model().masters;
        let doc = document::migrate_v1(CHECKED_IN_SLICE, Migration::AssetSource).unwrap();
        let mut server = ServerModel::new(doc, Origin::AssetSource, Vec::new(), false);
        server.masters = masters;
        server.masters_ready = true;
        server.join();
        assert!(!server.responses.last().unwrap().is_refreshed);
        assert_eq!(server.doc.gamedata.refreshed_at, 1_790_218_800_000);
    }

    #[test]
    fn a_clock_set_behind_the_last_refresh_refreshes() {
        let mut model = model();
        model.doc.clock = document::Clock::Fixed {
            at_ms: 1_790_000_000_000,
        };
        model.join();
        assert!(model.responses.last().unwrap().is_refreshed);
        assert_eq!(model.doc.gamedata.refreshed_at, 1_790_000_000_000);
        model.responses.clear();
        model.tick();
        assert!(model.responses.is_empty());
    }

    #[test]
    fn the_refill_policy_refills_normal_only() {
        let mut model = model();
        model.doc.clock = document::Clock::Fixed {
            at_ms: 1_790_300_000_000,
        };
        model.doc.stamina = Some(Stamina {
            normal: 10,
            enhance: 5,
            boost: 7,
        });
        model.join();
        assert_eq!(
            model.doc.stamina,
            Some(Stamina {
                normal: 1000,
                enhance: 5,
                boost: 7
            })
        );
    }

    #[test]
    fn a_pending_stamina_edit_wins_over_the_sent_rests() {
        let mut model = model();
        model.join();
        model.doc.stamina = Some(Stamina {
            normal: 500,
            enhance: 0,
            boost: 0,
        });
        model.mark_pending(SECTION_STAMINA);
        let pending = model.pending.iter().any(|s| s == SECTION_STAMINA);
        assert!(pending);
        model.respond(ResponseKind::Harvest, false, &[SECTION_STAMINA]);
        assert!(model.pending.is_empty());
        assert_eq!(
            model.responses.last().unwrap().stamina,
            Some(Stamina {
                normal: 500,
                enhance: 0,
                boost: 0
            })
        );
    }

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
