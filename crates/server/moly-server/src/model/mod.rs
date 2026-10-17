//! The server model: one document per user, the server clock, the server's
//! named policies and the sections a response carries.
//!
//! **The document.** [`document`] (schemaVersion 2) holds only server-decided
//! values: the gamedata, the stamina, the pass, the schedules, the home gate
//! and its characters, the owned MySekai tables, the music settings and
//! records, the system fixture actions, the tools, the layouts and the named
//! policies. The game keeps copies of the tables a response carries; this
//! crate keeps none of them.
//!
//! **Responses.** Every request a user makes runs one method here, which
//! records a [`ServerResponse`] on the model: the sections changed since the
//! last response, with the document's current values. A response of
//! [`ResponseKind::Join`] or [`ResponseKind::Refresh`] also carries the
//! schedule rows. The transport answers the request with the recorded
//! sections; pushes carry the responses the clock and the seated parties make
//! on their own.
//!
//! **Named policies** (server rules with no client code):
//! - *Stamina refresh*: [`document::REFILL_POLICY_TEXT`]. The windows are the
//!   master refresh time periods of the phenomena index; the server stamps
//!   `refreshedAt` with its clock at the refresh.
//! - *Initial stamina*: a user whose document has no stamina gets the normal
//!   pool at its master maximum, enhance 0 and the boost of one recovery (the
//!   first stamina-recovery row's `recoveryBoostStamina`).
//! - *Rank from experience*: `mysekaiRank` is the master rank of `totalExp`
//!   (the highest rank row whose `totalExp` is at most it); an edit of either
//!   field sets the other.
//! - *Harvest stamina*: the harvest reply's stamina is the rests the client
//!   sent, unless an edit of the stamina is pending; then the edit is the
//!   reply.
//! - *Gate reserve*: the reserved unit group joins the characters of the home
//!   gate (`isReservation` true, `visitCount` 1). *Gate change*: the home gate
//!   takes the new gate and skin and the stated unit groups become its
//!   characters (`isReservation` false, `visitCount` 1).
//! - *Daily schedule*: see [`document::SchedulePolicy::Daily`].
//! - *Site level releases*: [`document::RANK_RELEASE_POLICY_TEXT`]. The
//!   extension `everyAuthoredLevel` appends to `mysekaiRankReleases` one
//!   `mysekai_site_level` row for every authored level of a housing site that
//!   lies above the level its release rows name, at one rank each, the
//!   highest level at the highest master rank.
//!
//! **Instruments.** A developer start may overlay a fixed clock, the screen
//! pools and the avatar/music/harvest instruments onto the document; every
//! instrument is read through [`instrument_env`], which is silent unless the
//! process runs as a developer build.

pub(crate) mod avatar;
pub(crate) mod clock;
pub(crate) mod craft;
pub(crate) mod delivery;
pub(crate) mod document;
pub(crate) mod edit;
pub(crate) mod home_action;
pub(crate) mod housing_layout;
pub(crate) mod inventory;
pub(crate) mod music;
pub(crate) mod music_play;
pub(crate) mod system_fixture_action;
pub(crate) mod talk_read;
pub(crate) mod tools;

use bevy::prelude::*;
use serde_json::{json, Value};

pub(crate) use crate::instrument::instrument_env;
pub(crate) use crate::masters::{int32, master_rows, Deferred, Masters};
pub(crate) use document::{ScheduleRow, Stamina};
pub(crate) use moly_protocol::gate::{GateReply, ReplyTalkList};
pub(crate) use moly_protocol::json::{SECTION_GAMEDATA, SECTION_PASS, SECTION_STAMINA};

use document::{SchedulePolicy, ServerDocument, SiteLevelReleases, StaminaRefresh};

const MAX_ERRORS: usize = 32;

/// `MysekaiRankReleaseType.mysekai_site_level` as the master names it.
const SITE_LEVEL_RELEASE: &str = "mysekai_site_level";

/// The rows the extended site-level release policy appends, in master order:
/// for every authored level of a housing site that lies above the highest
/// level that site's release rows name, one `mysekai_site_level` row, the
/// highest level at the highest master rank and each lower one a rank below
/// it (`max_rank - n + 1` to `max_rank`), so that no two of them share a rank
/// and the greatest rank is the greatest level's.
///
/// An authored level a release row leaves unnamed but that lies *below* the
/// level the site's rows release gets no row: the site's greatest rank is the
/// one the expansion unlocks, so a row for it would take that rank and lower
/// the site's level. The second value names the levels left out, for the
/// model to report. `Err` names the master tables the rows are read from that
/// the model has not seated.
fn appended_release_rows(masters: &Masters) -> Result<(Vec<Value>, Vec<String>), String> {
    let (Some(categories), Some(levels), Some(releases), Some(ranks)) = (
        &masters.site_categories,
        &masters.site_levels,
        &masters.rank_releases,
        &masters.ranks,
    ) else {
        return Err(
            "policies.siteLevelReleases everyAuthoredLevel reads the mysekaiSites, mysekaiSiteLevels, mysekaiRankReleases and mysekaiRanks master tables, and this root has not seated them all; no release row is appended"
                .to_owned(),
        );
    };
    let Some(max_rank) = ranks.iter().map(|(rank, _)| *rank).max() else {
        return Err(
            "the mysekaiRanks master table has no row, so the extended site-level release policy has no rank to release at; no release row is appended"
                .to_owned(),
        );
    };
    let mut appended = Vec::new();
    let mut notes = Vec::new();
    for (site_id, category) in categories {
        if !is_housing_category(category) {
            continue;
        }
        // The site's authored levels no release row names, the level its rows
        // release and the greatest rank they carry.
        let mut unnamed = Vec::new();
        let mut released_level = 0;
        let mut released_rank = 0;
        for (id, _, level) in levels.iter().filter(|(_, site, _)| site == site_id) {
            let mut named = false;
            for (rank, _, _) in releases
                .iter()
                .filter(|(_, kind, external)| kind == SITE_LEVEL_RELEASE && external == id)
            {
                named = true;
                released_rank = released_rank.max(*rank);
                released_level = released_level.max(*level);
            }
            if !named {
                unnamed.push((*id, *level));
            }
        }
        let mut above: Vec<(i32, i32)> = unnamed
            .iter()
            .copied()
            .filter(|(_, level)| *level > released_level)
            .collect();
        above.sort_by_key(|(_, level)| *level);
        let below: Vec<i32> = unnamed
            .iter()
            .filter(|(_, level)| *level < released_level)
            .map(|(id, _)| *id)
            .collect();
        if !below.is_empty() {
            notes.push(format!(
                "policies.siteLevelReleases everyAuthoredLevel: housing site {site_id} leaves the levels {below:?} unnamed, below its released level {released_level}; they get no release row, because the greatest rank is the one the expansion unlocks and a row for one of them would lower the site's level"
            ));
        }
        // One rank per level, the highest level at the highest master rank;
        // the ranks above the site's released rank hold that many rows.
        let free = (max_rank - released_rank).max(0) as usize;
        if above.len() > free {
            let dropped: Vec<i32> = above[..above.len() - free]
                .iter()
                .map(|(id, _)| *id)
                .collect();
            notes.push(format!(
                "policies.siteLevelReleases everyAuthoredLevel: housing site {site_id} has {} levels above its released level {released_level}, and the {free} master rank(s) above its released rank {released_rank} hold only {free} of them; the levels {dropped:?} get no release row",
                above.len()
            ));
            above = above.split_off(above.len() - free);
        }
        for (index, (id, _)) in above.iter().enumerate() {
            appended.push(json!({
                "mysekaiRank": max_rank - (above.len() as i32 - 1 - index as i32),
                "mysekaiRankRelaseType": SITE_LEVEL_RELEASE,
                "externalId": id,
            }));
        }
    }
    Ok((appended, notes))
}

/// Whether the master's site category is a housing one (`housing_home`,
/// `housing_room`). Every other site is a harvest map or the festival garden,
/// whose levels no housing expansion reads.
fn is_housing_category(category: &str) -> bool {
    matches!(category, "housing_home" | "housing_room")
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

/// One response as the client receives it: its kind and the user data
/// sections it carries.
#[derive(Clone, Debug)]
pub(crate) struct ServerResponse {
    pub(crate) kind: ResponseKind,
    pub(crate) sections: moly_protocol::Sections,
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
    pub(crate) fn name(self) -> &'static str {
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
    pub(super) joined: bool,
    /// Counts document changes.
    pub(super) revision: u64,
    /// The document changed since it was last written to the store.
    pub(super) changed: bool,
    /// The rows the client last received.
    pub(crate) served: Vec<ScheduleRow>,
    pub(crate) responses: Vec<ServerResponse>,
    pub(crate) gate_replies: Vec<GateReply>,
    pub(super) errors: Vec<String>,
    /// The birthday parties in session at the server clock, as the server
    /// last seated them.
    pub(super) party_masters: Vec<delivery::PartyMaster>,
    /// Why no party can be in session: the birthday parties table is
    /// missing.
    pub(super) parties_missing: Option<String>,
    /// Home-action requests answered (craft, canvas, sketch).
    pub(super) home_action_replies: [u64; 3],
    /// The developer avatar instrument, applied once the masters resolve.
    pub(super) avatar_instrument: avatar::AvatarInstrument,
    /// Music player requests answered (set, eject).
    pub(super) music_play_replies: [u64; 2],
}

impl ServerModel {
    pub(crate) fn new(doc: ServerDocument, origin: Origin, pending: Vec<String>) -> Self {
        Self {
            doc,
            origin,
            pending,
            joined: false,
            revision: 0,
            changed: false,
            served: Vec::new(),
            responses: Vec::new(),
            gate_replies: Vec::new(),
            errors: Vec::new(),
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

    /// The server panel's NPC slice (schemaVersion 1): the rows served at the
    /// join and the client's `refreshedAt`.
    #[allow(dead_code)] // Read by the server panel's seam.
    pub(crate) fn npc_slice(&self, served: &[ScheduleRow], refreshed_at: i64) -> Option<String> {
        Some(self.doc.npc_slice(served, refreshed_at).to_string())
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The rows the extended site-level release policy appends to
    /// `mysekaiRankReleases` ([`SiteLevelReleases::EveryAuthoredLevel`]).
    /// The source's own policy appends none; a model that has not seated a
    /// table the rows are read from refuses by name and appends none.
    pub(crate) fn overlay_rank_releases(&mut self, masters: &Masters) -> Vec<Value> {
        if self.doc.site_level_releases != SiteLevelReleases::EveryAuthoredLevel {
            return Vec::new();
        }
        match appended_release_rows(masters) {
            Ok((rows, notes)) => {
                for note in notes {
                    self.push_error(note);
                }
                rows
            }
            Err(reason) => {
                self.push_error(reason);
                Vec::new()
            }
        }
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

    /// After every document change: count it and mark the document for the
    /// store.
    pub(super) fn commit(&mut self) {
        self.revision += 1;
        self.changed = true;
    }

    pub(super) fn mark_pending(&mut self, section: &str) {
        if !self.pending.iter().any(|known| known == section) {
            self.pending.push(section.to_owned());
        }
    }

    /// The rows the schedule is served with at `now`.
    pub(super) fn served_rows(&mut self, now: i64, masters: &Masters) -> Vec<ScheduleRow> {
        match &self.doc.schedule_policy {
            SchedulePolicy::Stated => self.doc.schedules.clone(),
            SchedulePolicy::Daily(map) => match &masters.periods {
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
    pub(super) fn refresh_due(&self, now: i64, masters: &Masters) -> bool {
        let Some(periods) = &masters.periods else {
            return false;
        };
        now < self.doc.gamedata.refreshed_at
            || clock::window_start(now, periods, clock::device_utc_offset_ms)
                .is_some_and(|start| self.doc.gamedata.refreshed_at < start)
    }

    /// The refresh: `refreshedAt` := now; the stamina refresh policy.
    pub(super) fn refresh(&mut self, now: i64, masters: &Masters) {
        let before = self.doc.gamedata.refreshed_at;
        self.doc.gamedata.refreshed_at = now;
        let refill = match (
            self.doc.stamina_refresh,
            masters.stamina_max,
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
    fn seat(&mut self, masters: &Masters) {
        let mut changed = false;
        if self.doc.stamina.is_none() {
            match masters.stamina_max {
                Some(max) => {
                    let boost = masters.boost_grant.unwrap_or(0);
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
            match masters.rank_of(self.doc.gamedata.total_exp) {
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
    pub(super) fn respond(
        &mut self,
        kind: ResponseKind,
        is_refreshed: bool,
        carries: &[&str],
        masters: &Masters,
    ) {
        let now = self.now_ms();
        let mut sections: Vec<String> = carries.iter().map(|s| (*s).to_owned()).collect();
        for section in self.pending.drain(..) {
            if !sections.contains(&section) {
                sections.push(section);
            }
        }
        let has = |name: &str| sections.iter().any(|section| section == name);
        let schedules = matches!(kind, ResponseKind::Join | ResponseKind::Refresh).then(|| {
            let rows = self.served_rows(now, masters);
            self.served = rows.clone();
            rows
        });
        let response = ServerResponse {
            kind,
            sections: moly_protocol::Sections {
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
                delivery: self.delivery_update(&sections, masters),
                music: has(music::SECTION).then(|| self.doc.music_settings.clone()),
                avatar: has(avatar::SECTION).then_some(self.doc.avatar),
                inventory: self.inventory_update(&sections, masters),
                music_records: has(music_play::RECORDS_SECTION)
                    .then(|| self.doc.music_records.clone()),
                system_fixture_actions: has(system_fixture_action::SECTION)
                    .then(|| self.doc.system_fixture_actions.rows.clone()),
                tools: has(tools::SECTION).then(|| self.doc.tools.clone().unwrap_or_default()),
            },
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
    pub(super) fn join(&mut self, masters: &Masters) {
        self.joined = true;
        let now = self.now_ms();
        self.seat(masters);
        if self.seat_tools(masters) {
            self.commit();
        }
        let refreshed = self.refresh_due(now, masters);
        if refreshed {
            self.refresh(now, masters);
        }
        if let Err(error) = edit::check_masters(&self.doc, masters) {
            self.push_error(format!(
                "the stored server document disagrees with the masters: {error} (kept as stored)"
            ));
        }
        if !self.avatar_instrument.is_empty() {
            let instrument = std::mem::take(&mut self.avatar_instrument);
            let (avatar, unresolved) = instrument.resolve(&masters.avatar);
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
        carries.push(tools::SECTION);
        self.respond(ResponseKind::Join, refreshed, &carries, masters);
    }

    /// The running clock's check.
    pub(super) fn tick(&mut self, masters: &Masters) {
        let now = self.now_ms();
        if self.refresh_due(now, masters) {
            self.refresh(now, masters);
            self.respond(
                ResponseKind::Refresh,
                true,
                &[SECTION_GAMEDATA, SECTION_STAMINA],
                masters,
            );
        }
    }

    /// The served values changed: serve the schedule again.
    pub(super) fn live_update(&mut self, masters: &Masters) {
        let now = self.now_ms();
        if self.joined && self.refresh_due(now, masters) {
            self.refresh(now, masters);
            self.respond(
                ResponseKind::Refresh,
                true,
                &[SECTION_GAMEDATA, SECTION_STAMINA],
                masters,
            );
            return;
        }
        self.served = self.served_rows(now, masters);
    }

    /// The harvest reply's stamina (harvest stamina policy), recorded as a
    /// response. The sent rests come back when an edit of the stamina is
    /// pending.
    pub(super) fn harvest(&mut self, rests: [i32; 3], masters: &Masters) -> Stamina {
        let sent = Stamina {
            normal: rests[0],
            enhance: rests[1],
            boost: rests[2],
        };
        if !self.pending.iter().any(|section| section == SECTION_STAMINA) {
            if self.doc.stamina != Some(sent) {
                self.doc.stamina = Some(sent);
                self.commit();
            }
        } else {
            info!("[server] harvest reply: a stamina edit is pending; the reply carries it instead of the sent rests {sent:?}");
        }
        self.respond(
            ResponseKind::Harvest,
            false,
            &[SECTION_STAMINA],
            masters,
        );
        self.doc.stamina.unwrap_or(sent)
    }

    /// The gather reply's stamina, recorded as a response.
    pub(super) fn gather(&mut self, masters: &Masters) -> Option<Stamina> {
        self.respond(
            ResponseKind::Gather,
            false,
            &[SECTION_STAMINA],
            masters,
        );
        self.doc.stamina
    }
}
