//! Music records on housing sites: the BGM a record set on the site plays.
//!
//! The client rule, as `MysekaiBGMManager.PlayBGMAsync(site, phenomena)` runs
//! it: `TryPlayUserSettingBGM` first (only on housing sites, the category
//! gate the BGM channel applies before calling here), then the default
//! choice when it answers false.
//! - The site's setting row gives the record id and the vocal id, and a
//!   `MysekaiMusicRecordModel(recordId, vocalId)` is built. That constructor
//!   stores the vocal id before it reads any master, so the setting's vocal
//!   always wins over the locally remembered one.
//! - The model reads the record row. When the row is missing, the model's
//!   construction dereferences nothing and throws a null reference, so the
//!   whole BGM request fails and no BGM changes (the current music keeps
//!   playing; on a cold start nothing plays).
//! - Track type `music_sound_track`: the soundtrack row named by the record's
//!   `externalId`; its `assetbundleName` and `assetbundleFileName` are the
//!   package and the cue, start time 0. Without the row the model has no
//!   resource and the default choice plays.
//! - Any other track type (`music`): the music named by `externalId`. Without
//!   it the model logs an error and has no resource: the default choice
//!   plays. The music's composite also needs its difficulties, which this
//!   port takes as present with the music row (the composite is assembled
//!   outside the readable code). A music without any vocal throws (the
//!   default-vocal lookup takes the first of an empty list): no BGM change.
//!   The vocal is the one whose id is the setting's vocal id among this
//!   music's vocals; when none matches the model has no resource and the
//!   default choice plays. The package is `music/long/<assetbundleName>`
//!   (`AssetBundleNames.GetLiveMusicName`) and the cue is the vocal's
//!   `assetbundleName`; the start time is the music's `fillerSec`.
//! - Only when the model has a resource does `TryPlayUserSettingBGM` answer
//!   true; it then plays the record through the same player call the default
//!   choice uses (`OutGameBGMController.LoadAndPlayAsync`, no fade loop,
//!   priority 1 -> `SoundManager.PlayBGM` with the controller's fade time
//!   and the start time). `SoundManager.PlayBGM` skips a cue whose name the
//!   BGM channel already holds, and `CriCorePlayer.PlayCoreFade` sets the
//!   start time as whole milliseconds, `(long)(startTime * 1000f)`, and
//!   forces the loop: a waveform with loop points loops between them, one
//!   without loops as a whole, from its beginning; the start time only
//!   places the first pass.
//! - `isInstrumental` of the setting row is not read on this path.
//!
//! What this port reads where: the four master tables from the public master
//! host (the mirror when the host fails); the waveform facts the audio host's
//! files do not carry (length, loop region or its absence, cue commands, the
//! verified sha256 of the host's file) from the root's record sidecar; the
//! audio (MP3, sample-aligned with the source waveform, filler lead-in
//! included) from the public storage host (the mirror when it fails). A
//! package missing from the sidecar was not swept and is refused by name.
//! The downloaded bytes must hash to the sidecar's sha256, or the record is
//! refused by name. Named differences, all of the same kind (the source's
//! own download layer has no such outcome): unreachable masters, a failed or
//! mismatching download and an undecodable file are refused by name once per
//! site entry and the default choice plays.
//!
//! The BGM select screen reaches the BGM through [`RecordChoice`] only, with
//! the two calls its presenter makes on the BGM manager:
//! - [`RecordChoice::play_record`]: the screen plays a record model for its
//!   site (`BGMSelectPresenter.PlayBgm` -> `MysekaiBGMManager.PlayBGMAsync`
//!   with the model), both for the highlighted record (`PlaySelectBgm`) and
//!   for the screen's chosen one (`PlaySettingBgm`). The screen returns
//!   without a call when the model has no resource, so here a record without
//!   a resource (or without a record row) changes no BGM; otherwise the
//!   record plays as a set record does, with the same refusals.
//! - [`RecordChoice::play_default`]: the screen plays the site's default
//!   (`BGMSelectPresenter.PlayDefaultBGM` ->
//!   `MysekaiBGMManager.PlayDefaultBGMAsync`), which skips the site's setting.
//! - [`RecordChoice::set_fade_time`]: the screen's `MysekaiBGMManager.SetFadeTime`
//!   (0.3 when it resumes, 1.0 when it is disposed), the crossfade time of
//!   every later BGM change.
//!
//! In the source each call simply replaces what plays until the next call on
//! the manager. This port chooses the BGM every frame from the site, the
//! phenomenon and the setting, so a choice stands over the setting until the
//! active site or the phenomenon changes (where the port asks for the site's
//! own choice again). The screen's requests to the server (set or eject on
//! exit) are separate and change the setting, not the choice.

use super::cue::gain_from_block;
use super::*;
use crate::site::NavMeshSourceRegion;
use bevy::ecs::system::SystemParam;
use moly_assets::remote::{self, Remote, RemoteRegion};
use sha2::{Digest, Sha256};

/// The package directory of a song's audio (`AssetBundleNames.GetLiveMusicName`
/// formats `music/long/{0}`).
const SONG_PACKAGE_PREFIX: &str = "music/long/";
/// The record master's soundtrack track type (`MysekaiMusicTrackType` 1, the
/// branch `SetupMasterData` takes); every other value takes the music branch.
const TRACK_SOUNDTRACK: &str = "music_sound_track";
/// `CriCorePlayer.PlayCoreFade` hands the start time to the player as
/// `(long)(startTime * 1000f)` milliseconds.
const START_TIME_UNITS_PER_SECOND: f32 = 1000.0;
/// The root's record sidecar (waveform facts of every swept record package).
const RECORD_SIDECAR: &str = "moly://music-record-audio.json";
const SIDECAR_VERSION: u64 = 1;
/// The upstream master tables the record model reads.
const TABLES: [&str; 4] = [
    "mysekaiMusicRecords",
    "musics",
    "musicVocals",
    "musicSoundTracks",
];

/// One record row: track type and external id.
struct RecordRow {
    soundtrack: bool,
    external_id: i64,
}

/// The master rows the record model reads, by id.
pub(crate) struct RecordMasters {
    records: HashMap<i64, RecordRow>,
    /// `musics.fillerSec` by music id (the master's float).
    fillers: HashMap<i64, f32>,
    /// `musicVocals` by music id: (vocal id, `assetbundleName`), master order.
    vocals: HashMap<i64, Vec<(i64, String)>>,
    /// `musicSoundTracks`: (`assetbundleName`, `assetbundleFileName`) by id.
    soundtracks: HashMap<i64, (String, String)>,
}

/// Parses the four upstream master tables (arrays of rows) the record model
/// reads. The only reader of these tables.
fn parse_record_masters(tables: &[serde_json::Value; 4]) -> Result<RecordMasters, String> {
    let rows = |index: usize| -> Result<&Vec<serde_json::Value>, String> {
        tables[index]
            .as_array()
            .ok_or_else(|| format!("{} is not an array of rows", TABLES[index]))
    };
    let int = |row: &serde_json::Value, table: &str, field: &str| -> Result<i64, String> {
        row[field]
            .as_i64()
            .ok_or_else(|| format!("{table} row {}: {field} is not an integer", row["id"]))
    };
    let text = |row: &serde_json::Value, table: &str, field: &str| -> Result<String, String> {
        row[field]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{table} row {}: {field} is not a string", row["id"]))
    };
    let mut records = HashMap::new();
    for row in rows(0)? {
        let kind = text(row, TABLES[0], "mysekaiMusicTrackType")?;
        records.insert(
            int(row, TABLES[0], "id")?,
            RecordRow {
                soundtrack: kind == TRACK_SOUNDTRACK,
                external_id: int(row, TABLES[0], "externalId")?,
            },
        );
    }
    let mut fillers = HashMap::new();
    for row in rows(1)? {
        let filler = row["fillerSec"]
            .as_f64()
            .ok_or_else(|| format!("musics row {}: fillerSec is not a number", row["id"]))?;
        fillers.insert(int(row, TABLES[1], "id")?, filler as f32);
    }
    let mut vocals: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    for row in rows(2)? {
        vocals
            .entry(int(row, TABLES[2], "musicId")?)
            .or_default()
            .push((
                int(row, TABLES[2], "id")?,
                text(row, TABLES[2], "assetbundleName")?,
            ));
    }
    let mut soundtracks = HashMap::new();
    for row in rows(3)? {
        soundtracks.insert(
            int(row, TABLES[3], "id")?,
            (
                text(row, TABLES[3], "assetbundleName")?,
                text(row, TABLES[3], "assetbundleFileName")?,
            ),
        );
    }
    Ok(RecordMasters {
        records,
        fillers,
        vocals,
        soundtracks,
    })
}

/// What the record model resolves to.
#[derive(Debug, PartialEq)]
enum Resolution {
    Track(RecordTrack),
    /// The model has no resource: the default choice plays.
    NoResource(String),
    /// The model's construction throws: no BGM change.
    Throws(String),
}

#[derive(Clone, Debug, PartialEq)]
struct RecordTrack {
    /// `song vocal <id>` or `soundtrack <id>`, for the log.
    kind: String,
    package: String,
    cue: String,
    /// Whole milliseconds, as the player takes them.
    start_ms: i64,
}

impl RecordMasters {
    fn resolve(&self, record_id: i64, vocal_id: i64) -> Resolution {
        let Some(record) = self.records.get(&record_id) else {
            return Resolution::Throws(format!("record {record_id} is not in the record master"));
        };
        let external = record.external_id;
        if record.soundtrack {
            return match self.soundtracks.get(&external) {
                Some((package, cue)) => Resolution::Track(RecordTrack {
                    kind: format!("soundtrack {external}"),
                    package: package.clone(),
                    cue: cue.clone(),
                    start_ms: 0,
                }),
                None => Resolution::NoResource(format!(
                    "soundtrack {external} is not in the soundtrack master"
                )),
            };
        }
        let Some(filler) = self.fillers.get(&external) else {
            return Resolution::NoResource(format!("music {external} is not in the music master"));
        };
        let vocals = self.vocals.get(&external).map(Vec::as_slice).unwrap_or(&[]);
        if vocals.is_empty() {
            return Resolution::Throws(format!("music {external} has no vocal"));
        }
        match vocals.iter().find(|(id, _)| *id == vocal_id) {
            Some((_, bundle)) => Resolution::Track(RecordTrack {
                kind: format!("song {external} vocal {vocal_id}"),
                package: format!("{SONG_PACKAGE_PREFIX}{bundle}"),
                cue: bundle.clone(),
                start_ms: (filler * START_TIME_UNITS_PER_SECOND) as i64,
            }),
            None => Resolution::NoResource(format!(
                "vocal {vocal_id} is not a vocal of music {external}"
            )),
        }
    }
}

/// The sidecar's facts of one package.
struct PackageFacts {
    loops: bool,
    loop_start: f64,
    loop_end: f64,
    duration: f64,
    bundle_version: String,
    sha256: String,
    gain: Option<CueGain>,
}

fn parse_record_sidecar(doc: &serde_json::Value) -> Result<HashMap<String, PackageFacts>, String> {
    if doc["version"].as_u64() != Some(SIDECAR_VERSION) {
        return Err("not version 1 of the record sidecar".into());
    }
    let packages = doc["packages"].as_object().ok_or("no packages")?;
    let mut out = HashMap::new();
    for (name, row) in packages {
        let cue = row["cue"]
            .as_str()
            .ok_or_else(|| format!("{name}: no cue"))?;
        let stream = &row["stream"];
        let number = |field: &str| {
            stream[field]
                .as_f64()
                .ok_or_else(|| format!("{name}: {field} is not a number"))
        };
        let loops = stream["loop"]
            .as_bool()
            .ok_or_else(|| format!("{name}: loop is not a boolean"))?;
        let (loop_start, loop_end) = if loops {
            (number("loopStartSeconds")?, number("loopEndSeconds")?)
        } else {
            (0.0, 0.0)
        };
        if loops && loop_end <= loop_start {
            return Err(format!(
                "{name}: loop region {loop_start}..{loop_end} is empty"
            ));
        }
        if row["sequences"]
            .as_object()
            .is_some_and(|blocks| blocks.contains_key(cue))
        {
            return Err(format!("{name}: cue {cue} is a structured sequence"));
        }
        let Some(sha256) = row["storage"]["sha256"].as_str() else {
            // A row without a verified storage file is not playable.
            continue;
        };
        out.insert(
            name.clone(),
            PackageFacts {
                loops,
                loop_start,
                loop_end,
                duration: number("durationSeconds")?,
                bundle_version: row["bundleVersion"].as_str().unwrap_or("?").to_owned(),
                sha256: sha256.to_owned(),
                gain: row["cueCommands"]
                    .get(cue)
                    .map(|block| gain_from_block(block, &label(cue))),
            },
        );
    }
    Ok(out)
}

/// The downloaded audio against the hash the sidecar verified. `Err` carries
/// the hash of the bytes that arrived.
fn check_audio_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let got = format!("{:x}", Sha256::digest(bytes));
    if got == expected {
        Ok(())
    } else {
        Err(got)
    }
}

/// One remote file tried on the primary host, then on the mirror.
struct RemoteLoad<A: Asset> {
    handle: Handle<A>,
    mirror: bool,
    first_error: Option<String>,
}

enum LoadStep<A: Asset> {
    Pending,
    Ready(Handle<A>),
    Failed(String),
}

impl<A: Asset> RemoteLoad<A> {
    fn start(
        server: &AssetServer,
        path: impl Fn(Remote) -> AssetPath<'static>,
        primary: Remote,
    ) -> Self {
        Self {
            handle: server.load(path(primary)),
            mirror: false,
            first_error: None,
        }
    }

    fn step(
        &mut self,
        server: &AssetServer,
        path: impl Fn(Remote) -> AssetPath<'static>,
        mirror: Remote,
    ) -> LoadStep<A> {
        match server.load_state(&self.handle) {
            LoadState::Loaded => LoadStep::Ready(self.handle.clone()),
            LoadState::Failed(error) if !self.mirror => {
                self.first_error = Some(error.to_string());
                self.handle = server.load(path(mirror));
                self.mirror = true;
                LoadStep::Pending
            }
            LoadState::Failed(error) => LoadStep::Failed(format!(
                "host: {}; mirror: {error}",
                self.first_error.as_deref().unwrap_or("?")
            )),
            _ => LoadStep::Pending,
        }
    }
}

enum AudioState {
    Loading(RemoteLoad<AudioSource>),
    Ready(Handle<AudioSource>),
    Refused(String),
}

/// Loaded inputs and fetched audio of the record path.
#[derive(Resource, Default)]
pub(crate) struct RecordLibrary {
    region: Option<RemoteRegion>,
    tables: Vec<RemoteLoad<JsonAsset>>,
    sidecar: Option<Handle<JsonAsset>>,
    masters: Option<Result<RecordMasters, String>>,
    facts: Option<Result<HashMap<String, PackageFacts>, String>>,
    audio: HashMap<String, AudioState>,
}

#[derive(SystemParam)]
pub(crate) struct RecordParams<'w> {
    library: ResMut<'w, RecordLibrary>,
    jsons: Res<'w, Assets<JsonAsset>>,
    sources: Res<'w, Assets<AudioSource>>,
    region: Option<Res<'w, NavMeshSourceRegion>>,
    admission: Option<Res<'w, remote::RemoteAdmission>>,
    choice: ResMut<'w, RecordChoice>,
}

/// What the BGM channel does after the record path ran.
pub(super) enum Step {
    /// Inputs or audio still arriving: no change this frame.
    Hold,
    /// The record plays (started now or already playing), or the source
    /// changes no BGM: no change by the default route.
    Keep,
    /// The default choice plays.
    Default,
}

/// What asked for the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    /// The site's setting (`TryPlayUserSettingBGM`).
    Setting,
    /// The BGM select screen ([`RecordChoice::play_record`]).
    Screen,
}

/// A choice of the BGM select screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Record {
        record_id: i64,
        vocal_id: i64,
    },
    /// The site's default, skipping its setting.
    Default,
}

#[derive(Clone, Debug)]
struct Chosen {
    site_id: u32,
    choice: Choice,
    /// The phenomenon the choice was first read under.
    phenomenon: Option<String>,
}

/// The BGM select screen's way into the BGM (see the module notes).
#[derive(Resource, Default)]
pub struct RecordChoice {
    chosen: Option<Chosen>,
    fade_seconds: Option<f32>,
}

impl RecordChoice {
    /// Plays the record `record_id` with the vocal `vocal_id` as the BGM of
    /// `site_id` (a soundtrack record ignores the vocal). The next frame's
    /// BGM choice reads it.
    pub fn play_record(&mut self, site_id: u32, record_id: i32, vocal_id: i32) {
        self.chosen = Some(Chosen {
            site_id,
            choice: Choice::Record {
                record_id: i64::from(record_id),
                vocal_id: i64::from(vocal_id),
            },
            phenomenon: None,
        });
    }

    /// Plays the default BGM of `site_id`, whatever the site's setting names
    /// (the screen's choice was cleared).
    pub fn play_default(&mut self, site_id: u32) {
        self.chosen = Some(Chosen {
            site_id,
            choice: Choice::Default,
            phenomenon: None,
        });
    }

    /// Sets the BGM crossfade time, in seconds, for every later BGM change
    /// (`MysekaiBGMManager.SetFadeTime`: the controller's fade time, which
    /// the screen sets to 0.3 when it resumes and to 1.0 when it is disposed;
    /// the value holds until the manager is rebuilt, and nothing else writes
    /// it after the controller's construction default of 0.25). A fade
    /// already running keeps the time it started with. A negative or
    /// non-finite time stops loudly.
    pub fn set_fade_time(&mut self, seconds: f32) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "BGM crossfade time {seconds} is not a non-negative number of seconds"
        );
        self.fade_seconds = Some(seconds);
    }

    /// The crossfade time of the next BGM change.
    fn fade_seconds(&self) -> f32 {
        self.fade_seconds.unwrap_or(CROSS_FADE_SECONDS)
    }
}

impl RecordParams<'_> {
    /// The crossfade time of the next BGM change.
    pub(super) fn fade_seconds(&self) -> f32 {
        self.choice.fade_seconds()
    }
}

/// The native instrument of the select screen's two entries.
#[cfg(not(target_arch = "wasm32"))]
const CHOICE_AUTOPLAY: &str = "MOLY_RECORD_CHOICE_AUTOPLAY";

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
pub(super) enum AutoplayStep {
    Play { site: u32, record: i32, vocal: i32 },
    Default { site: u32 },
    Fade { seconds: f32 },
}

/// `MOLY_RECORD_CHOICE_AUTOPLAY`: comma-separated steps
/// `<seconds>=play:<site>:<record>:<vocal>`, `<seconds>=default:<site>` or
/// `<seconds>=fade:<crossfade seconds>`, in time order; a malformed step
/// stops loudly.
#[cfg(not(target_arch = "wasm32"))]
fn parse_autoplay(raw: &str) -> std::collections::VecDeque<(f32, AutoplayStep)> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            parse_autoplay_step(entry).unwrap_or_else(|| {
                panic!("{CHOICE_AUTOPLAY} step {entry:?} is not <seconds>=play:<site>:<record>:<vocal>, <seconds>=default:<site> or <seconds>=fade:<crossfade seconds>")
            })
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_autoplay_step(entry: &str) -> Option<(f32, AutoplayStep)> {
    let (at, step) = entry.split_once('=')?;
    let at = at.trim().parse::<f32>().ok()?;
    let parts: Vec<&str> = step.split(':').map(str::trim).collect();
    let step = match parts.as_slice() {
        ["play", site, record, vocal] => AutoplayStep::Play {
            site: site.parse().ok()?,
            record: record.parse().ok()?,
            vocal: vocal.parse().ok()?,
        },
        ["default", site] => AutoplayStep::Default {
            site: site.parse().ok()?,
        },
        ["fade", seconds] => AutoplayStep::Fade {
            seconds: seconds.parse().ok()?,
        },
        _ => return None,
    };
    Some((at, step))
}

/// Calls the select screen's entries at the instrument's times (seconds of
/// real time since the app started).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn autoplay_record_choice(
    time: Res<Time<bevy::time::Real>>,
    mut choice: ResMut<RecordChoice>,
    mut steps: Local<Option<std::collections::VecDeque<(f32, AutoplayStep)>>>,
) {
    let steps = steps.get_or_insert_with(|| {
        let steps = std::env::var(CHOICE_AUTOPLAY)
            .map(|raw| parse_autoplay(&raw))
            .unwrap_or_default();
        if !steps.is_empty() {
            warn!("[audio] {CHOICE_AUTOPLAY} instrument on: {steps:?}");
        }
        steps
    });
    while steps
        .front()
        .is_some_and(|(at, _)| time.elapsed_secs() >= *at)
    {
        let (at, step) = steps.pop_front().expect("checked above");
        info!("[audio] {CHOICE_AUTOPLAY}: {step:?} at {at:.1}s");
        match step {
            AutoplayStep::Play {
                site,
                record,
                vocal,
            } => choice.play_record(site, record, vocal),
            AutoplayStep::Default { site } => choice.play_default(site),
            AutoplayStep::Fade { seconds } => choice.set_fade_time(seconds),
        }
    }
}

/// The screen's choice standing for the active site and phenomenon, if any.
/// A new choice binds to the phenomenon it is first read under and clears
/// the refusal guard (each choice names its own refusal); a choice for
/// another site, or read under another phenomenon, is dropped.
pub(super) fn standing_choice(
    params: &mut RecordParams,
    channel: &mut BgmChannel,
    site_id: u32,
    phenomenon: &str,
) -> Option<Choice> {
    let chosen = params.choice.chosen.as_mut()?;
    if chosen.site_id != site_id {
        warn!(
            "[audio] record BGM: the select screen's choice for site {} is dropped: the active site is {site_id}",
            chosen.site_id
        );
        params.choice.chosen = None;
        return None;
    }
    match chosen.phenomenon.as_deref() {
        None => {
            chosen.phenomenon = Some(phenomenon.to_owned());
            channel.music_refused_for = None;
            info!(
                "[audio] record BGM: the select screen chose {:?} for site {site_id}",
                chosen.choice
            );
        }
        Some(bound) if bound != phenomenon => {
            params.choice.chosen = None;
            return None;
        }
        Some(_) => {}
    }
    Some(chosen.choice)
}

/// Logs once per site entry (the channel's record-refusal guard).
fn once(channel: &mut BgmChannel, site_id: u32, line: impl FnOnce() -> String, error: bool) {
    if channel.music_refused_for != Some(site_id) {
        if error {
            error!("{}", line());
        } else {
            warn!("{}", line());
        }
        channel.music_refused_for = Some(site_id);
    }
}

/// The record path for a site whose setting (or the select screen) names
/// `record_id` / `vocal_id`.
#[allow(clippy::too_many_arguments)]
pub(super) fn advance(
    params: &mut RecordParams,
    commands: &mut Commands,
    server: &AssetServer,
    channel: &mut BgmChannel,
    bus: &VolumeBus,
    bgm_player: f32,
    gate: &AudioGate,
    site_id: u32,
    record_id: i64,
    vocal_id: i64,
    origin: Origin,
) -> Step {
    let Some(region) = params.region.as_deref() else {
        return Step::Hold;
    };
    let region = match region.0 {
        moly_law::carve::NavMeshRegion::Jp => RemoteRegion::Jp,
        moly_law::carve::NavMeshRegion::Cn => RemoteRegion::Cn,
    };
    let library = &mut *params.library;
    if library.region != Some(region) {
        *library = RecordLibrary {
            region: Some(region),
            ..default()
        };
        library.tables = TABLES
            .iter()
            .map(|table| {
                RemoteLoad::start(
                    server,
                    |host| remote::master_table(host, region, table),
                    Remote::Master,
                )
            })
            .collect();
        library.sidecar = Some(server.load(RECORD_SIDECAR));
        info!("[audio] record BGM: reading the record masters from the master host and the record sidecar");
        return Step::Hold;
    }
    if library.masters.is_none() {
        let mut ready = Vec::new();
        let mut failed = None;
        for (index, load) in library.tables.iter_mut().enumerate() {
            let table = TABLES[index];
            match load.step(
                server,
                |host| remote::master_table(host, region, table),
                Remote::MasterMirror,
            ) {
                LoadStep::Ready(handle) => ready.push(handle),
                LoadStep::Pending => {}
                LoadStep::Failed(reason) => failed = Some(format!("{table}: {reason}")),
            }
        }
        if let Some(reason) = failed {
            library.masters = Some(Err(reason));
        } else if ready.len() == TABLES.len() {
            let parsed: Result<Vec<serde_json::Value>, String> = ready
                .iter()
                .map(|handle| {
                    let asset = params
                        .jsons
                        .get(handle)
                        .ok_or("loaded table is not in the assets")?;
                    serde_json::from_str(&asset.0).map_err(|error| error.to_string())
                })
                .collect();
            library.masters = Some(parsed.and_then(|tables| {
                let tables: [serde_json::Value; 4] =
                    tables.try_into().map_err(|_| "table count".to_string())?;
                parse_record_masters(&tables)
            }));
            if let Some(Ok(masters)) = &library.masters {
                info!(
                    "[audio] record BGM: masters read (records {}, musics {}, vocals of {} musics, soundtracks {})",
                    masters.records.len(),
                    masters.fillers.len(),
                    masters.vocals.len(),
                    masters.soundtracks.len()
                );
            }
        } else {
            return Step::Hold;
        }
    }
    if library.facts.is_none() {
        let handle = library.sidecar.clone().expect("requested with the masters");
        library.facts = match server.load_state(&handle) {
            LoadState::Loaded => Some(
                params
                    .jsons
                    .get(&handle)
                    .ok_or_else(|| "loaded sidecar is not in the assets".to_string())
                    .and_then(|asset| {
                        serde_json::from_str::<serde_json::Value>(&asset.0)
                            .map_err(|e| e.to_string())
                    })
                    .and_then(|doc| parse_record_sidecar(&doc)),
            ),
            LoadState::Failed(error) => Some(Err(format!("{RECORD_SIDECAR} is absent: {error}"))),
            _ => return Step::Hold,
        };
        if let Some(Ok(facts)) = &library.facts {
            info!(
                "[audio] record BGM: sidecar read ({} playable packages)",
                facts.len()
            );
        }
    }
    let masters = match library.masters.as_ref().expect("set above") {
        Ok(masters) => masters,
        Err(reason) => {
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM refused: site {site_id} record {record_id} (vocal {vocal_id}): the record masters are unreachable ({reason}); the default choice plays"
            )
                },
                true,
            );
            return Step::Default;
        }
    };
    let track = match masters.resolve(record_id, vocal_id) {
        Resolution::Track(track) => track,
        Resolution::NoResource(reason) if origin == Origin::Screen => {
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM: site {site_id} record {record_id} (vocal {vocal_id}) has no resource ({reason}); the select screen plays nothing for it, so no BGM changes"
            )
                },
                false,
            );
            return Step::Keep;
        }
        Resolution::NoResource(reason) => {
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM: site {site_id} record {record_id} (vocal {vocal_id}) has no resource ({reason}); the source plays the default choice here"
            )
                },
                false,
            );
            return Step::Default;
        }
        Resolution::Throws(reason) => {
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM refused: site {site_id} record {record_id} (vocal {vocal_id}): {reason}; {}, so no BGM changes",
                if origin == Origin::Screen {
                    "the select screen holds no model for it"
                } else {
                    "the source's record model throws here"
                }
            )
                },
                true,
            );
            return Step::Keep;
        }
    };
    if channel
        .voice
        .as_ref()
        .is_some_and(|voice| voice.cue == track.cue)
    {
        return Step::Keep;
    }
    let facts = match library.facts.as_ref().expect("set above") {
        Ok(facts) => facts.get(&track.package),
        Err(reason) => {
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM refused: site {site_id} record {record_id} ({}): {reason}; the default choice plays",
                track.kind
            )
                },
                true,
            );
            return Step::Default;
        }
    };
    let Some(facts) = facts else {
        once(
            channel,
            site_id,
            || {
                format!(
            "[audio] record BGM refused: site {site_id} record {record_id} ({}): package {} is not swept (no verified row in the record sidecar); the default choice plays",
            track.kind, track.package
        )
            },
            true,
        );
        return Step::Default;
    };
    let relative = format!("{}/{}.mp3", track.package, track.cue);
    let path = |host: Remote| remote::storage_asset(host, region, &relative);
    let admission = params.admission.as_deref();
    let url = |host: Remote| {
        let path = path(host);
        admission
            .and_then(|admission| admission.url(&path))
            .unwrap_or_else(|| path.to_string())
    };
    let state = library
        .audio
        .entry(track.package.clone())
        .or_insert_with(|| {
            info!("[audio] record BGM: fetching {}", url(Remote::Storage));
            AudioState::Loading(RemoteLoad::start(server, path, Remote::Storage))
        });
    if let AudioState::Loading(load) = state {
        match load.step(server, path, Remote::StorageMirror) {
            LoadStep::Pending => return Step::Hold,
            LoadStep::Failed(reason) => {
                *state = AudioState::Refused(format!("the download failed ({reason})"))
            }
            LoadStep::Ready(handle) => {
                let url = url(if load.mirror {
                    Remote::StorageMirror
                } else {
                    Remote::Storage
                });
                let source = params
                    .sources
                    .get(&handle)
                    .expect("a loaded audio is in the assets");
                *state = match check_audio_hash(&source.bytes, &facts.sha256) {
                    Err(got) => AudioState::Refused(format!(
                        "the audio's sha256 does not match: expected {} got {got} ({url})",
                        facts.sha256
                    )),
                    Ok(()) => match rodio::Decoder::new(std::io::Cursor::new(source.clone())) {
                        Err(error) => AudioState::Refused(format!(
                            "the audio does not decode: {error} ({url})"
                        )),
                        Ok(decoder) => {
                            use rodio::Source as _;
                            info!(
                                "[audio] record BGM: fetched {url}: {} bytes, sha256 {} matches the sidecar (bundle {}), decoded {} Hz {} ch {:?}",
                                source.bytes.len(),
                                facts.sha256,
                                facts.bundle_version,
                                decoder.sample_rate(),
                                decoder.channels(),
                                decoder.total_duration()
                            );
                            AudioState::Ready(handle)
                        }
                    },
                };
            }
        }
    }
    let handle = match state {
        AudioState::Ready(handle) => handle.clone(),
        AudioState::Refused(reason) => {
            let reason = reason.clone();
            once(
                channel,
                site_id,
                || {
                    format!(
                "[audio] record BGM refused: site {site_id} record {record_id} ({}): {reason}; the default choice plays",
                track.kind
            )
                },
                true,
            );
            return Step::Default;
        }
        AudioState::Loading(_) => return Step::Hold,
    };
    let fade_seconds = params.choice.fade_seconds();
    start_voice(
        commands,
        channel,
        handle,
        facts,
        &track,
        bus,
        bgm_player,
        gate,
        fade_seconds,
    );
    info!(
        "[audio] record BGM start: site {site_id} record {record_id} ({}, {}) -> package {} cue {} from {:.3}s ({}; crossfade {:.2}s)",
        track.kind,
        match origin {
            Origin::Setting => "the site's setting",
            Origin::Screen => "the select screen",
        },
        track.package,
        track.cue,
        track.start_ms as f64 / 1000.0,
        if facts.loops {
            format!("loop region [{:.3}, {:.3}]", facts.loop_start, facts.loop_end)
        } else {
            format!("no loop chunk: the whole {:.3}s waveform loops", facts.duration)
        },
        fade_seconds
    );
    Step::Keep
}

/// Starts the record as the channel's voice: the current voice fades out;
/// the first pass plays from the start time to the loop's end (or to the
/// waveform's end), then the loop region (or the whole waveform) repeats.
#[allow(clippy::too_many_arguments)]
fn start_voice(
    commands: &mut Commands,
    channel: &mut BgmChannel,
    handle: Handle<AudioSource>,
    facts: &PackageFacts,
    track: &RecordTrack,
    bus: &VolumeBus,
    bgm_player: f32,
    gate: &AudioGate,
    fade_seconds: f32,
) {
    if let Some(voice) = channel.voice.take() {
        let entities = voice
            .intro
            .iter()
            .chain(&voice.loop_sink)
            .copied()
            .collect();
        channel.fading.push(FadingBgm {
            entities,
            from_volume: voice.applied_volume,
            elapsed: 0.0,
            seconds: fade_seconds,
        });
    }
    let cold = channel.fading.is_empty();
    let volume = track_volume(facts.gain.as_ref(), Player::Bgm, SLOT_BGM, 0);
    let target = BGM_VOLUME_FACTOR * volume.linear_at(bus, bgm_player) * gate.factor();
    let now = Volume::Linear(if cold { target } else { 0.0 });
    let start = track.start_ms as f64 / 1000.0;
    let (region_start, region_end) = if facts.loops {
        (facts.loop_start, facts.loop_end)
    } else {
        (0.0, facts.duration)
    };
    let loop_settings = if facts.loops {
        PlaybackSettings::LOOP
            .with_start_position(Duration::from_secs_f64(region_start))
            .with_duration(Duration::from_secs_f64(region_end - region_start))
    } else {
        PlaybackSettings::LOOP
    }
    .with_volume(now);
    // The first pass ends where the loop hands over: at the loop start when
    // the start time is before it, otherwise at the loop's (or waveform's) end.
    let first_pass_end = if start < region_start {
        region_start
    } else {
        region_end
    };
    let (intro, loop_sink, handoff_at, handoff_done) =
        if start == region_start || first_pass_end <= start {
            let loop_sink = commands
                .spawn((AudioPlayer::new(handle), loop_settings))
                .id();
            (None, loop_sink, 0.0, true)
        } else {
            let mut first = PlaybackSettings::ONCE
                .with_duration(Duration::from_secs_f64(first_pass_end - start))
                .with_volume(now);
            if start > 0.0 {
                first = first.with_start_position(Duration::from_secs_f64(start));
            }
            let intro = commands
                .spawn((AudioPlayer::new(handle.clone()), first))
                .id();
            let loop_sink = commands
                .spawn((AudioPlayer::new(handle), loop_settings.paused()))
                .id();
            (Some(intro), loop_sink, first_pass_end - start, false)
        };
    channel.voice = Some(BgmVoice {
        cue: track.cue.clone(),
        intro,
        loop_sink: Some(loop_sink),
        loop_start: region_start,
        loop_end: region_end,
        handoff_at,
        handoff_done,
        fade_in: if cold { None } else { Some(0.0) },
        fade_seconds,
        applied_volume: if cold { target } else { 0.0 },
        volume,
    });
}
