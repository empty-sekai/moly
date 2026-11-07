//! Audio domain, first batch: BGM switching, area ambience, the Set A
//! proximity ambience manager, one-shot SE, and the nine-category volume
//! panel.
//!
//! The two persistent channels follow the managers of the original game and
//! are kept separate, not merged:
//!
//! - **BGM** (track selection of the BGM manager `MysekaiBGMManager`):
//!   computed from the current site x current phenomenon. `PlayBGMAsync(site,
//!   phenomenon)` first asks the user's record setting (checked only at
//!   housing-type sites; the setting is server-side user state, written into
//!   the client-side copy [`crate::audio_inputs::ClientMusicPlaySettings`] by
//!   the server model's response, empty by default), then falls back to the
//!   default selection `PlayDefaultBGMAsync`: phenomenon id equal to the
//!   client configuration's delivery phenomenon id -> the site's normal
//!   variant; the phenomenon master table's brightness variant not none ->
//!   the site x brightness-variant row; none -> the phenomenon's own BGM row
//!   (the master data cache builds this dictionary by mysekaiPhenomenaId; a
//!   duplicate key is overridden by the later entry). A delivery-type site
//!   (the festival venue) has a site controller that always starts BGM with
//!   the delivery phenomenon id, regardless of the phenomenon shown on
//!   screen. When a branch finds no row, nothing starts and the current sound
//!   keeps playing. On a track switch the old sound fades out and the new one
//!   fades in (the linear curve of the CRI fader, see
//!   `CROSS_FADE_SECONDS`); the same cue does not restart. The BGM player
//!   starts with forced looping, so a BGM waveform without a loop point loops
//!   as a whole. Selection happens on site entry (each site controller's
//!   SetupBGM), on site changes (the teleport cannon and the home/room
//!   move-action states) and on the furniture-edit response; the in-site
//!   ambience crossfade (UpdateSite in multi-player furniture updates) does
//!   not re-select. The phenomenon switch button has no counterpart in the
//!   original game; this crate treats it as entering the site with that
//!   phenomenon: any change of site or phenomenon re-selects for the new
//!   "site x phenomenon".
//! - **Area ambience** (the phenomenon SE manager
//!   `MysekaiPhenomenaSEManager`): flat 2D — neither position nor orientation
//!   enters this chain; the volume is the SE player x the cue's categories and
//!   volume commands. The original game calls it once when the environment is
//!   built (site entry) and once when the crossfade ends: the site type first
//!   passes a gate (the four types grassland, shore, flower garden and
//!   memorial place play; home, the three room floors and the festival venue
//!   only stop), and a passing site takes the rows for this phenomenon, else
//!   the rows with the "other" condition. Every call stops the old sound
//!   before playing (the same cue also restarts from the top); a phenomenon
//!   without a master-table row logs an error and stops. Here the trigger is
//!   every real commit of the weather chain (the commit sequence advancing
//!   once equals one call in the original game). Cues are played by their
//!   **sequence structure**: the asset export writes the structure into the
//!   loop.json package entries (`sequenceExport` count + `sequences` blocks).
//!   A cue that is listed without a block is plain (one waveform per track,
//!   played from the cue start). Cues with a block are played by the rules of
//!   sequence types 0-4 (`cue_law`: all tracks, sequential, shuffle,
//!   random, no-repeat random); each chosen track waits for its own start
//!   delay and plays its waveform to the end, and once every track of a round
//!   has finished and the repeat flag is set the next round starts (`cue`).
//!   Positions and the order table live in the cue engine's work area, so a
//!   stop and restart continues where it left off. One-shot SE and timeline
//!   SE share the same engine. Every other structure (types 5-8, action
//!   tracks, undecoded events, ...), and keys that were never read yet carry
//!   several waveforms, are refused by name; one of their waveforms is not
//!   used as a substitute.
//!
//! **The Set A proximity ambience manager** (the sound source object manager)
//! is the third consumer: source components + distance tuning
//! `clamp(1 - d/max, 0, 1)` + nearest selection on a single channel. Sources
//! are fed by the site scene's sidecar component table (the `site_sound`
//! module attaches them to the expanded scene entities) — a sound source
//! object is not attached to a furniture prefab; it is a first-class entry on
//! the site scene side.
//!
//! **talk voice** (PlayVoice of MySekaiTalkEngine) is the fourth consumer: the
//! dialogue domain delivers a line request when a line advances and this
//! domain starts it in the same frame — in the per-line command order voice
//! precedes text, so voice and the text line are one beat, not the window
//! open. The channel is monophonic: every start first runs `StopVoiceAll`
//! (before the ExistsCueName check, so a missing cue also stops the old
//! sound); a missing cue means "no playback, the dialogue continues", which
//! this crate does and logs as a named skip. Volume = the voice player x the
//! cue's categories (talk voice cues belong to VOX_SCENARIO) and volume
//! commands.
//!
//! **The one-shot SE channel** (`SoundManager.PlaySEOneShot`'s event family)
//! is the fifth: the event side enqueues (gather hits, dialogue-window
//! skip/step, furniture-edit actions) and the channel side starts — flat 2D
//! (PlaySEOneShot uses a player without a 3D source, so position never enters
//! this chain).
//! Requests start individually after the scene's cue-name substitutions. The
//! ambient AudioSourceRepeater interval does not apply to button/one-shot sounds.
//! Every request starts its cue on the cue engine (plain or sequence types
//! 0-4); the volume is the SE player times the cue's own categories and
//! volume commands. The request's class (`se_ingame`/`se_ui`) only stands in
//! for a cue whose facts the export does not carry.
//!
//! A cue binds to an audio file by **(cue, the routing row's package)**, not
//! by the bare cue name: the same cue name can live both in a "multi-track
//! bundle" package and in a single-cue download (the same song with two
//! different loop ranges), and the routing row's package field is what picks
//! one. A talk voice routing row follows the mapping: cue =
//! `voice_<script>_<line>_<variant>` and the package follows the script
//! (`mysekai__talk__voice__<script>`), so the cue itself does not need the
//! package name.
//!
//! Volume has two layers, multiplied per sounding track (see
//! `cue::CueVolume`). The **player layer** is the user's system volume on
//! the middleware's three players: BGM (times 0.7, `BGM_VOLUME_FACTOR`),
//! SE and voice (`SetupVolume(1, bgm, se, voice)` at scene start and on every
//! option-page `UpdateVolume`, [`apply_system_volume`]; the Live group of the
//! local settings is the rhythm game's and is persisted, not applied). The
//! **category layer** is the sound configuration's nine categories
//! (SE_INGAME ... BGM): every cue belongs to the categories its sequence
//! command list names, each category has a volume in the configuration
//! (0.75-0.90, not 1), and MySekai never writes them (the only writers are in
//! the streaming live). So a cue sounds at `player x category volumes x the
//! cue's, track's and synth's volume commands`, whatever channel starts it.
//! The configuration's category table comes as
//! `phenomena/audio/categories.json` (name, index, volume; the index order is
//! the configuration's, not the order of the client's category enum), and
//! the cues' categories and volume commands come with the sequence export. A
//! cue whose facts are not exported plays in its channel's family category,
//! and the parse names how many keys do. The environment variables
//! `MOLY_AUDIO_VOLUME_<CLASS>` override a category's volume at load (mock
//! rule).

mod cue;
mod cue_law;
pub mod cutscene;
mod record;

pub use self::record::RecordChoice;
pub use self::cue::advance_cue_playbacks as advance_ambient_sequence;
use self::cue::{track_volume, CueGain, CueVolume};
pub use self::cue::{CuePlayback, CueRngs, Player, RngSlot};
use self::cue_law::{SequenceKind, NO_REPEAT_HISTORY_FORMAT};
use crate::audio_sequence::{ShuffleWork, SEQUENCE_TYPE_SHUFFLE};
use crate::markers::AvatarRoot;
use crate::client_config::ClientConfigs;
use crate::markers::SiteActive;
use crate::voice_pcm::{MeteredVoiceSource, VoiceSource};
use crate::audio_inputs::CurrentPhenomenon;
use crate::weather_transition::WeatherTransition;
use bevy::asset::{AssetPath, LoadState};
use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume,
};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

/// The order of the site type enum `MysekaiSiteType` (the site master table's
/// siteType names map onto it in order). The phenomenon ambience gate shifts
/// by this order.
const SITE_TYPE_ORDER: [&str; 9] = [
    "home_site",
    "first_floor",
    "second_floor",
    "third_floor",
    "grassland",
    "shore",
    "flower_garden",
    "memorial_place",
    "festival_garden",
];

/// 现象环境音门 `MysekaiUtility.IsPlaySitePhenomenaSound` 的位掩码：
/// `(0xF0 >> 站点类型序) & 1`——只有草原、海岸、花园、纪念地四类为真。
const SITE_PHENOMENA_SOUND_MASK: u32 = 0xF0;

/// 站点主表的站点类别名（`MysekaiSiteCategory`）。
const CATEGORY_HOUSING_HOME: &str = "housing_home";
const CATEGORY_HOUSING_ROOM: &str = "housing_room";
const CATEGORY_DELIVERY: &str = "delivery";

/// The phenomenon ambience gate. An out-of-range type order raises the
/// argument-out-of-range exception in the original game; this crate refuses
/// loudly in the same way.
fn plays_site_phenomena_sound(site_type: &str) -> bool {
    let order = SITE_TYPE_ORDER
        .iter()
        .position(|name| *name == site_type)
        .unwrap_or_else(|| panic!("站点类型 {site_type} 不在站点类型枚举里（现象环境音门）"));
    (SITE_PHENOMENA_SOUND_MASK >> order) & 1 != 0
}

/// BGM volume factor (a declaration-level constant of the original game;
/// multiplied onto the bus volume).
const BGM_VOLUME_FACTOR: f32 = 0.7;

/// Fade duration of a BGM track switch (seconds): the default fade time 0.25
/// set when the BGM controller is constructed. The BGM selection screen sets
/// 0.3 on Resume and 1.0 on Dispose (through
/// [`RecordChoice::set_fade_time`]); the new value applies to every later
/// switch until the manager is rebuilt — this product never leaves MySekai
/// and never rebuilds. Every fade runs to the end with the value it started
/// with. The original game hands the duration in milliseconds to the CRI
/// player's fader: the old sound falls linearly from its current level to 0
/// and the new sound rises linearly from 0 to 1; both start together and each
/// covers the same fade duration (the default linear curve, no start offset).
/// CRI advances on the audio service thread with each step capped at 67ms;
/// this crate advances per render frame, whose length is not that quantity, so
/// the cap is not implemented. The original game keeps the silent old sound
/// for 500ms after its level reaches 0 before stopping it; this crate stops
/// it at zero.
const CROSS_FADE_SECONDS: f32 = 0.25;

/// Lead (seconds) with which the BGM intro part hands over to the loop part:
/// once the one-shot intro plays up to "loop start - lead", the parked loop
/// part is released, and the overlap at the handover is capped by the lead.
/// The original game loops seamlessly sample by sample inside the cue player;
/// the vorbis decoder has no seek, so the two-part handover is approximate and
/// trades a short overlap for no silent gap.
const BGM_HANDOFF_LEAD_SECONDS: f32 = 0.05;

/// Unity `Mathf.Epsilon`：最小非规格化正数（Rust 的 `f32::EPSILON` 是机器
/// epsilon，不是这个数，别用混）。
const UNITY_MIN_FLOAT: f32 = 1.4e-45;

// ---- 用户音量档（本地档）与九类总线 ----------------------------------------

/// The user's volume as one set of three knobs (`VolumeSettingData`: fields
/// Bgm/Se/Voice, serialized under the same keys; range 0..=1 in steps of 0.01
/// — the UI side is an integer 0..100, and both ends multiply/divide by 0.01).
/// Default 1.0 (`ApplicationLocalSettings` constructs all three fields of both
/// groups as 1.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeSettingData {
    pub bgm: f32,
    pub se: f32,
    pub voice: f32,
}

impl Default for VolumeSettingData {
    fn default() -> Self {
        VolumeSettingData {
            bgm: 1.0,
            se: 1.0,
            voice: 1.0,
        }
    }
}

/// The volume half of the local settings (`ApplicationLocalSettings` has one
/// each for `LiveVolume` and `SystemVolume`): six floats, Live/System x
/// Bgm/Se/Voice. **Only the System group enters the MySekai application
/// chain** (the on-entry `SetupVolume(1.0, Bgm×0.7, Se, Voice)` reads only
/// SystemVolume; the 0.7 lands on the BGM consumer side as
/// `BGM_VOLUME_FACTOR`); the Live group belongs to the rhythm game — MySekai
/// does not apply it, but still persists it and reads/writes it on the option
/// page. Loaded in [`init_settings`].
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
#[derive(Default)]
pub struct LocalVolumeSettings {
    pub live: VolumeSettingData,
    pub system: VolumeSettingData,
}


/// Both volume layers. The nine named fields are the **category layer**, in
/// the order of the client's category enum (`SeCategoryType`): each holds
/// that category's volume from the sound configuration (1.0 until the
/// category table loads), then the load-time environment override.
/// `acf_slots` maps the configuration's own category index (what a cue sheet
/// names) to a field by name. `player` is the **player layer**, written by
/// [`apply_system_volume`].
#[derive(Resource, Clone)]
pub struct VolumeBus {
    pub se_ingame: f32,
    pub se_ui: f32,
    pub se_scenario: f32,
    pub se_sl: f32,
    pub se_sl_call: f32,
    pub se_area_ambient: f32,
    pub vox_scenario: f32,
    pub vox_ingame: f32,
    pub bgm: f32,
    /// The three players' volumes (the user's system settings).
    pub player: PlayerVolumes,
    /// Configuration category index -> field slot; empty until the category
    /// table has loaded.
    acf_slots: Vec<Option<usize>>,
}

/// The user's system volume per player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerVolumes {
    pub bgm: f32,
    pub se: f32,
    pub voice: f32,
}

/// Field slots of the category layer (client category enum order).
const SLOT_SE_INGAME: usize = 0;
const SLOT_SE_UI: usize = 1;
const SLOT_SE_AREA_AMBIENT: usize = 5;
const SLOT_VOX_SCENARIO: usize = 6;
const SLOT_BGM: usize = 8;

impl VolumeBus {
    /// The user's volume of one player.
    pub fn player_volume(&self, player: Player) -> f32 {
        match player {
            Player::Bgm => self.player.bgm,
            Player::Se => self.player.se,
            Player::Voice => self.player.voice,
            Player::Unscaled => 1.0,
        }
    }

    /// A category's volume by field slot.
    pub fn slot_volume(&self, slot: usize) -> f32 {
        [
            self.se_ingame,
            self.se_ui,
            self.se_scenario,
            self.se_sl,
            self.se_sl_call,
            self.se_area_ambient,
            self.vox_scenario,
            self.vox_ingame,
            self.bgm,
        ][slot]
    }

    /// A category's volume by the configuration's index. Before the category
    /// table loads the index cannot be named and the factor is 1.0 (the parse
    /// logs that the table is absent).
    pub fn acf_volume(&self, index: u32) -> f32 {
        match self.acf_slots.get(index as usize) {
            Some(Some(slot)) => self.slot_volume(*slot),
            Some(None) => 1.0,
            None if self.acf_slots.is_empty() => 1.0,
            None => panic!(
                "cue names category index {index}, the configuration has {}",
                self.acf_slots.len()
            ),
        }
    }
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioGate {
    pub enabled: bool,
}

impl Default for AudioGate {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl AudioGate {
    #[inline]
    fn factor(&self) -> f32 {
        if self.enabled {
            1.0
        } else {
            0.0
        }
    }
}

/// How a sink's volume follows the two layers. `Cue` is the full product of
/// one sounding track; `Se(class)` is a sound its starter plays outside the
/// cue player (SE player, the class's category, no cue facts).
#[derive(Component)]
pub enum BusVolume {
    Cue(CueVolume),
    Se(SeClass),
}

pub fn apply_sink_volumes(
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    mut sinks: Query<(&BusVolume, &mut AudioSink)>,
    mut spatial: Query<(&BusVolume, &mut bevy::audio::SpatialAudioSink)>,
) {
    for (source, mut sink) in &mut spatial {
        let volume = match source { BusVolume::Cue(v)=>v.linear(&bus),BusVolume::Se(v)=>v.volume(&bus) } * gate.factor();
        if sink.volume().to_linear()!=volume { sink.set_volume(Volume::Linear(volume)); }
    }
    for (source, mut sink) in &mut sinks {
        let volume = match source {
            BusVolume::Cue(volume) => volume.linear(&bus),
            BusVolume::Se(class) => class.volume(&bus),
        } * gate.factor();
        if sink.volume().to_linear() != volume {
            sink.set_volume(Volume::Linear(volume));
        }
    }
}

/// 总线类的环境变量名后缀（装载期初始覆写用；序同 [`volume_fields`]）。
const VOLUME_ENTRY_NAMES: [&str; 9] = [
    "SE_INGAME",
    "SE_UI",
    "SE_SCENARIO",
    "SE_SL",
    "SE_SL_CALL",
    "SE_AREA_AMBIENT",
    "VOX_SCENARIO",
    "VOX_INGAME",
    "BGM",
];

/// 九类字段的可变借用数组（初始覆写按下标寻址）。
fn volume_fields(bus: &mut VolumeBus) -> [&mut f32; 9] {
    let VolumeBus {
        se_ingame,
        se_ui,
        se_scenario,
        se_sl,
        se_sl_call,
        se_area_ambient,
        vox_scenario,
        vox_ingame,
        bgm,
        ..
    } = bus;
    [
        se_ingame,
        se_ui,
        se_scenario,
        se_sl,
        se_sl_call,
        se_area_ambient,
        vox_scenario,
        vox_ingame,
        bgm,
    ]
}

impl Default for VolumeBus {
    /// Neutral 1.0 everywhere. The player layer is applied by
    /// [`init_settings`]; the category layer by [`parse`] when the category
    /// table and the environment overrides are read.
    fn default() -> Self {
        VolumeBus {
            se_ingame: 1.0,
            se_ui: 1.0,
            se_scenario: 1.0,
            se_sl: 1.0,
            se_sl_call: 1.0,
            se_area_ambient: 1.0,
            vox_scenario: 1.0,
            vox_ingame: 1.0,
            bgm: 1.0,
            player: PlayerVolumes {
                bgm: 1.0,
                se: 1.0,
                voice: 1.0,
            },
            acf_slots: Vec::new(),
        }
    }
}

impl VolumeBus {
    /// One ledger line: the three players, then the nine categories.
    fn describe(&self) -> String {
        format!(
            "players bgm {:.2} · se {:.2} · voice {:.2} | categories se_ingame {:.2} · se_ui {:.2} · se_scenario {:.2} · se_sl {:.2} · se_sl_call {:.2} · \
             se_area_ambient {:.2} · vox_scenario {:.2} · vox_ingame {:.2} · bgm {:.2}",
            self.player.bgm,
            self.player.se,
            self.player.voice,
            self.se_ingame,
            self.se_ui,
            self.se_scenario,
            self.se_sl,
            self.se_sl_call,
            self.se_area_ambient,
            self.vox_scenario,
            self.vox_ingame,
            self.bgm,
        )
    }
}

/// The system group applied to the three players (`SetupVolume` at scene
/// start and the option page's `UpdateVolume` follow one rule, master 1.0).
/// The categories are not touched: MySekai never writes them. `reason` names
/// the writer; an unchanged value logs nothing.
pub fn apply_system_volume(bus: &mut VolumeBus, system: &VolumeSettingData, reason: &str) {
    let next = PlayerVolumes {
        bgm: system.bgm,
        se: system.se,
        voice: system.voice,
    };
    let changed = bus.player != next;
    bus.player = next;
    if changed {
        info!(
            "[option] 施加（{reason}）：SetupVolume(1.0, bgm={:.2}, se={:.2}, voice={:.2}) → 总线 \
             [{}]（bgm 消费侧再乘 {} 进 BGM 通道；LiveVolume 不进施加——节奏域律）",
            system.bgm,
            system.se,
            system.voice,
            bus.describe(),
            BGM_VOLUME_FACTOR
        );
    }
}

/// StopVoiceAll also cancels deferred line requests at this command boundary.
/// The active dialogue owners are exclusive and share this global voice bus.
pub fn stop_voice_all(channel: &mut VoiceChannel, commands: &mut Commands) {
    for old in [channel.sink.take(), channel.preview.take()]
        .into_iter()
        .flatten()
    {
        if let Ok(mut entity_commands) = commands.get_entity(old) {
            entity_commands.despawn();
        }
    }
    channel.cue = None;
    commands.queue(clear_pending_voice_lines);
}

/// Source PlayAsync disposes its talk engine after the Lua stream completes;
/// Dispose stops voice and cancels outstanding work. Queue at the same boundary
/// as the dialogue's final VoiceLine commands, before serve_voice's flush.
pub fn dispose_talk_voice(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        let old = {
            let mut channel = world.resource_mut::<VoiceChannel>();
            channel.cue = None;
            channel.sink.take()
        };
        if let Some(old) = old {
            world.despawn(old);
        }
        clear_pending_voice_lines(world);
    });
}

fn clear_pending_voice_lines(world: &mut World) {
    let pending: Vec<Entity> = world
        .query_filtered::<Entity, With<VoiceLine>>()
        .iter(world)
        .collect();
    for entity in pending {
        world.despawn(entity);
    }
}

// ---- 本地档的存取（ApplicationLocalSettings 音量半的持久化） ---------------

/// The original game goes through `PersistentDataUtility` (a local file under
/// Unity's persistentDataPath) and this crate stores locally as well. The
/// serializer details are not fixed, so **the file shape is this crate's
/// choice**: container keys follow the field names (LiveVolume/SystemVolume)
/// and field keys follow the serialization keys (Bgm/Se/Voice). On native the
/// file goes to the user data directory (the counterpart of
/// persistentDataPath) and `MOLY_SETTINGS_FILE` can override it (for
/// verification); on wasm it goes to localStorage.
pub fn settings_sections(
    settings: &LocalVolumeSettings,
) -> [(&'static str, serde_json::Value); 2] {
    let group = |value: &VolumeSettingData| serde_json::json!({"Bgm": value.bgm, "Se": value.se, "Voice": value.voice});
    [
        ("LiveVolume", group(&settings.live)),
        ("SystemVolume", group(&settings.system)),
    ]
}

/// JSON text -> settings. A document that is not a JSON object at all =>
/// `None` (the caller falls back to the defaults loudly — the same rule as the
/// original game, which uses the constructed defaults when the settings file
/// cannot be read); an absent group or field => that slot takes the default
/// (the settings are the union of six floats, so a partial file is still a
/// file); an out-of-range value (hand-edited file) => clamped back to 0..=1.
fn parse_volume_settings(text: &str) -> Option<LocalVolumeSettings> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    if !doc.is_object() {
        return None;
    }
    let group = |key: &str| -> VolumeSettingData {
        let mut data = VolumeSettingData::default();
        let Some(obj) = doc.get(key).filter(|v| v.is_object()) else {
            return data;
        };
        let clamp = |v: &serde_json::Value| -> Option<f32> {
            v.as_f64().map(|n| (n as f32).clamp(0.0, 1.0))
        };
        if let Some(v) = obj.get("Bgm").and_then(clamp) {
            data.bgm = v;
        }
        if let Some(v) = obj.get("Se").and_then(clamp) {
            data.se = v;
        }
        if let Some(v) = obj.get("Voice").and_then(clamp) {
            data.voice = v;
        }
        data
    };
    Some(LocalVolumeSettings {
        live: group("LiveVolume"),
        system: group("SystemVolume"),
    })
}

/// Startup：装载本地档 → 进场施加 → 环境变量初始覆写（三行日志：装载行 ·
/// 施加行 · 覆写行）。这是 [`VolumeBus`] 的首写者链——进场律照
/// `SceneMysekai.Start → SetupVolume`（只应用 System 组）。
pub fn init_settings(mut commands: Commands, mut bus: ResMut<VolumeBus>) {
    let (settings, origin) = match crate::settings_store::read_text() {
        Ok(Some(text)) => {
            match parse_volume_settings(&text) {
                Some(settings) => (settings, "本地档"),
                None => {
                    warn!("[option] settings text is not a JSON object; the whole settings fall back to the defaults (an unreadable settings file uses the constructed defaults)");
                    (LocalVolumeSettings::default(), "默认（档损坏）")
                }
            }
        }
        Ok(None) => (LocalVolumeSettings::default(), "默认（无档·首跑）"),
        Err(err) => {
            warn!("[option] 本地档读取失败：{err}——整档回默认，本次会话的改动将无法落盘（fail-closed，不静默换位落盘）");
            (LocalVolumeSettings::default(), "默认（读取失败）")
        }
    };
    info!(
        "[option] 本地档装载：来源={origin} · Live{{bgm {:.2}, se {:.2}, voice {:.2}}} · \
         System{{bgm {:.2}, se {:.2}, voice {:.2}}}（LiveVolume 是 mysekai 不应用的节奏域档，持久化照做）",
        settings.live.bgm,
        settings.live.se,
        settings.live.voice,
        settings.system.bgm,
        settings.system.se,
        settings.system.voice,
    );
    apply_system_volume(
        &mut bus,
        &settings.system,
        "进场施加（SceneMysekai.Start → SetupVolume 同律：只读 SystemVolume）",
    );
    // The category layer (configuration volumes, then the environment
    // overrides) is applied by `parse` when the category table is read.
    commands.insert_resource(settings);
}

/// Load-time environment overrides of the category layer (mock rule):
/// `MOLY_AUDIO_VOLUME_<CLASS>` in 0..=1 replaces that category's volume.
fn apply_volume_overrides(bus: &mut VolumeBus) {
    let mut overridden: Vec<&str> = Vec::new();
    for (name, slot) in VOLUME_ENTRY_NAMES.iter().zip(volume_fields(bus)) {
        let Ok(raw) = std::env::var(format!("MOLY_AUDIO_VOLUME_{name}")) else {
            continue;
        };
        match raw.parse::<f32>() {
            Ok(v) if (0.0..=1.0).contains(&v) => {
                *slot = v;
                overridden.push(name);
            }
            _ => warn!(
                "[option] MOLY_AUDIO_VOLUME_{name}={raw:?} 不是 0..=1 的数，不覆写（保留配置值）"
            ),
        }
    }
    if !overridden.is_empty() {
        info!(
            "[option] category volume overridden by environment for {}: {} (mock rule)",
            overridden.len(),
            overridden.join(" · ")
        );
    }
}

/// The sound configuration's category table (`categories.json`): every row
/// names a category, the configuration's index of it and its volume. The
/// names are the client's category enum names; the fields take the volumes
/// and `acf_slots` maps each index to its field.
fn apply_category_table(bus: &mut VolumeBus, doc: &serde_json::Value) {
    let rows = doc["categories"]
        .as_array()
        .unwrap_or_else(|| panic!("categories.json 缺 categories 数组"));
    let mut slots: Vec<Option<usize>> = Vec::new();
    for row in rows {
        let name = field_str(row, "name");
        let index = usize::try_from(field_i64(row, "index"))
            .unwrap_or_else(|_| panic!("categories.json 里类别 {name} 的序为负"));
        let volume = field_f64(row, "volume") as f32;
        let slot = VOLUME_ENTRY_NAMES
            .iter()
            .position(|entry| *entry == name)
            .unwrap_or_else(|| panic!("categories.json 的类别 {name} 不在客户端类别枚举里"));
        if slots.len() <= index {
            slots.resize(index + 1, None);
        }
        if slots[index].replace(slot).is_some() {
            panic!("categories.json 里类别序 {index} 出现两次");
        }
        *volume_fields(bus)[slot] = volume;
    }
    bus.acf_slots = slots;
}

/// 写档（选项页 OK 的 SaveToStorage 对应物）：序列化 → 落盘 → 读回解析
/// 校验，三步一行账。失败响亮（会话内档对象已改、盘上没改——下次装载
/// 回旧值，如实报）。
pub fn save_volume_settings(settings: &LocalVolumeSettings) {
    if let Err(err) = crate::settings_store::save_sections(&settings_sections(settings)) {
        warn!("[option] 本地档写盘失败：{err}——本次改动不持久（档对象在会话内已是新值）");
        return;
    }
    match crate::settings_store::read_text() {
        Ok(Some(stored)) => match parse_volume_settings(&stored) {
            Some(readback) => {
                let same = readback == *settings;
                info!(
                    "[option] 本地档落盘：{} → 读回解析{}（{}）",
                    stored_location(),
                    if same { "一致" } else { "不一致！" },
                    describe_settings(settings)
                );
                if !same {
                    warn!("[option] 读回值与写入值不一致——持久化层有问题，下次装载以盘上为准");
                }
            }
            None => warn!("[option] 落盘后读回解析失败（写入文本非 JSON 对象）"),
        },
        _ => warn!("[option] 落盘后读不回（写盘成功但读取失败）——持久化层有问题"),
    }
}

/// 档的六浮点一行账。
fn describe_settings(settings: &LocalVolumeSettings) -> String {
    format!(
        "Live{{bgm {:.2}, se {:.2}, voice {:.2}}} · System{{bgm {:.2}, se {:.2}, voice {:.2}}}",
        settings.live.bgm,
        settings.live.se,
        settings.live.voice,
        settings.system.bgm,
        settings.system.se,
        settings.system.voice,
    )
}

/// 档所在地的日志名（native 带路径，wasm 带 localStorage 键）。
fn stored_location() -> String {
    crate::settings_store::location()
}

// ---- 流表与路由表 ----------------------------------------------------------

/// Asset name label for logs and refusal messages: non-ASCII characters are
/// written as code points. The cue/package/phenomenon name spaces come from
/// game assets and a future re-extraction may bring in characters outside the
/// expected set; only this encoded form goes into the logs.
fn label(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("U+{:04X}", c as u32)
            }
        })
        .collect()
}

/// A decoded stream's asset-root-relative path and authored loop interval.
struct StreamRow {
    ogg: String,
    loops: bool,
    loop_start: f64,
    loop_end: f64,
    /// 整轨时长（秒）——talk voice 的起播账目用（行→cue→包→时长）。
    duration: f64,
    /// 这个 (cue, package) 键下的波形条数。大于 1 时这个 cue 是多轨序列：
    /// 选哪条、何时放由 cue 表的序列结构决定（客户端 cue 表数据，不是服务端
    /// 配置），结构见 [`SequenceIndex`]，洗牌序列的律见 [`crate::audio_sequence`]。
    waveforms: usize,
}

/// (cue, package) → 流。同一键带多条流（多 subsong）时那是一个多轨序列
/// cue，流表只留最小 subsong 一条并记下波形条数：cue 引擎（环境音、一次性
/// SE、时间线 SE 的新入口）见到没有序列导出的多波形键具名拒绝；仍按路径
/// 取流的 [`Routing::timeline_se_asset_path`] 取这一条，是具名的替身（单流
/// 键 subsong 为空值，不与编号争——空值折算为最大，必输给任何编号）。
#[derive(Default)]
struct Streams(HashMap<(String, String), StreamRow>);

/// 一条路由的指向：cue 名（换曲判定用）+ 路由行自带的 package（流表
/// 选择面——同一 cue 名可活在多个包里，见模块注释）。
#[derive(Clone)]
struct RouteCue {
    cue: String,
    package: String,
}

impl RouteCue {
    /// 从路由行取指向（siteBgms/bgms/siteSounds/siteSoundFallbacks 同构）。
    fn from_row(row: &serde_json::Value) -> Self {
        Self {
            cue: field_str(row, "cue").to_string(),
            package: field_str(row, "package").to_string(),
        }
    }
}

/// 现象主表的亮度档（`MysekaiPhenomenaBrightnessType`：none/normal/bright/dark）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Brightness {
    None,
    Normal,
    Bright,
    Dark,
}

impl Brightness {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "normal" => Some(Self::Normal),
            "bright" => Some(Self::Bright),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Normal => "normal",
            Self::Bright => "bright",
            Self::Dark => "dark",
        }
    }
}

/// 一档现象的音频面（现象清单一项）。
struct PhenomenonAudio {
    /// 现象 id（配送祭会场那档是客户端配置里的配送现象 id）。
    id: i64,
    /// 现象主表行的亮度档；`None` = 没有主表行（配送祭会场那档）。
    master_brightness: Option<Brightness>,
    /// 现象自己的 BGM 行（亮度档 none 的现象用）。
    own_bgm: Option<RouteCue>,
    /// siteId → 本现象在该站的环境音行（主表序，同站取第一行）。
    site_sounds: HashMap<i64, RouteCue>,
}

/// 环境音选行的结果。
enum AmbientRoute<'a> {
    Play(&'a RouteCue),
    /// The phenomenon has no master table row: the original game logs an
    /// error and stops.
    NoMaster,
    /// The site has neither a row for this phenomenon nor an "other" row: the
    /// original game gets an empty name and only stops, without playing.
    /// `site_has_rows` = the site has other rows in the table (the original
    /// game logs no error in that case).
    NoRow {
        site_has_rows: bool,
    },
}

/// Track routing: BGM and area ambience are independent (the two managers of
/// the original game each query their own table) and both are computed from
/// "site x phenomenon" at run time, not folded into a single-site table at
/// load.
#[derive(Resource)]
pub struct Routing {
    /// (siteId, 亮度档) → BGM 路由（站点 BGM 表）。换曲判定按 cue 名：
    /// 同曲的两档不重启。
    site_bgms: HashMap<(i64, Brightness), RouteCue>,
    /// siteId → 「其它」条件的环境音行（同站取第一行）。
    site_sound_fallbacks: HashMap<i64, RouteCue>,
    /// Sites with any row in the phenomenon ambience table (the original game
    /// logs an error when "the site has no row at all" and only gets an empty
    /// name, without an error, when rows exist but neither this phenomenon nor
    /// the "other" condition matches).
    sites_with_sounds: HashSet<i64>,
    /// 档名 → 该档的音频面。
    phenomena: HashMap<String, PhenomenonAudio>,
    streams: Streams,
    /// 提取侧导出的序列结构（只来自现象音频的 loop.json）。
    sequences: SequenceIndex,
}

/// talk voice 包的包名前缀（提取侧清单族 `mysekai/talk/voice/` 的扁平形）。
const TALK_VOICE_PACKAGE_PREFIX: &str = "mysekai__talk__voice__";
/// The talk voice bank under the Birthday context
/// (`mysekai/talk/birthday/voice/{0}`), flattened like the plain one.
const BIRTHDAY_TALK_VOICE_PACKAGE_PREFIX: &str = "mysekai__talk__birthday__voice__";
const TUTORIAL_TALK_VOICE_PACKAGE_PREFIX: &str = "mysekai__talk__tutorial__voice__";

impl Routing {
    fn phenomenon(&self, name: &str) -> &PhenomenonAudio {
        // 天气与音频读同一份现象清单：档不在表里是结构不变量破裂。
        self.phenomena
            .get(name)
            .unwrap_or_else(|| panic!("音频路由表里没有档 {}", label(name)))
    }

    /// The default BGM selection (`PlayDefaultBGMAsync`; the site controller
    /// supplies the input phenomenon id: delivery-type sites pass the delivery
    /// phenomenon id, other sites the site's current phenomenon). `Err` = this
    /// branch finds no row and nothing starts.
    fn bgm_route(
        &self,
        site: &SiteActive,
        phenomenon: &str,
        delivery_phenomenon_id: i64,
    ) -> Result<&RouteCue, String> {
        let entry = self.phenomenon(phenomenon);
        let site_id = i64::from(site.site_id);
        let phenomenon_id = if site.category == CATEGORY_DELIVERY {
            delivery_phenomenon_id
        } else {
            entry.id
        };
        let brightness = if phenomenon_id == delivery_phenomenon_id {
            Brightness::Normal
        } else {
            match entry.master_brightness {
                None => return Err(format!("现象 id {phenomenon_id} 没有现象主表行")),
                Some(Brightness::None) => {
                    return entry.own_bgm.as_ref().ok_or_else(|| {
                        format!("现象 id {phenomenon_id} 的亮度档是 none，却没有现象 BGM 行")
                    });
                }
                Some(brightness) => brightness,
            }
        };
        self.site_bgms.get(&(site_id, brightness)).ok_or_else(|| {
            format!(
                "站点 BGM 表没有 site {site_id} 的 {} 档",
                brightness.label()
            )
        })
    }

    /// Phenomenon ambience row selection (`GetSiteSEName`): if the site has
    /// rows for this phenomenon, take the first; otherwise take the first row
    /// with the site's "other" condition. The site-type gate is at the caller.
    fn ambient_route(&self, site_id: i64, phenomenon: &str) -> AmbientRoute<'_> {
        let entry = self.phenomenon(phenomenon);
        if entry.master_brightness.is_none() {
            return AmbientRoute::NoMaster;
        }
        match entry
            .site_sounds
            .get(&site_id)
            .or_else(|| self.site_sound_fallbacks.get(&site_id))
        {
            Some(route) => AmbientRoute::Play(route),
            None => AmbientRoute::NoRow {
                site_has_rows: self.sites_with_sounds.contains(&site_id),
            },
        }
    }

    /// The timeline names both package and cue. A same-name cue in another
    /// package or the public UI sound bank is not a substitute. One path per
    /// key: for a multi-track cue it is the lowest-numbered waveform, a named
    /// stand-in; [`Self::cue_asset_paths`] with [`start_timeline_cue`] plays
    /// the cue as the middleware does.
    pub fn timeline_se_asset_path(&self, package: &str, cue: &str) -> Option<&str> {
        self.streams
            .0
            .get(&(cue.to_owned(), package.to_owned()))
            .map(|stream| stream.ogg.as_str())
    }
    /// talk voice 包里的全部 (cue, 包) 对，按 (cue, 包) 字典序——合成对话
    /// 注入口的确定性选材面（取前几条即固定样本）。
    pub fn talk_voice_cues(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .streams
            .0
            .keys()
            .filter(|(_, package)| package.starts_with(TALK_VOICE_PACKAGE_PREFIX))
            .cloned()
            .collect();
        rows.sort();
        rows
    }
}

/// 装载请求；三份必达档案到达后由 `parse` 一次性吃掉。第四份 partvoice
/// 路由表**可缺席**：文件不在＝路由面缺席（消费侧 fail-closed 具名跳过），
/// 装载失败不拦三份必达档案的解析，只把 partvoice 面折成 `None`。
#[derive(Resource)]
pub struct AudioRequests {
    ui: Handle<JsonAsset>,
    index: Handle<JsonAsset>,
    loops: Handle<JsonAsset>,
    corpus: Handle<JsonAsset>,
    partvoice: Handle<JsonAsset>,
    /// The sound configuration's category table (may be absent: named).
    categories: Handle<JsonAsset>,
}

/// Startup：请求三份必达路由档案与 partvoice 路由表。index.json 与天气域
/// 共用同一份资产（AssetServer 按路径去重），loop.json 是音频自己的循环
/// 点表，corpus.json 是 talk voice 的语料账本（cue→包账面 + 上游无包具名
/// 清单），partvoice.json 是说话者/家具 → 变体语音包的查表面（可缺席）。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let index = server.load::<JsonAsset>(AssetPath::from("moly://phenomena/index.json".to_owned()));
    let loops = server.load::<JsonAsset>(AssetPath::from(
        "moly://phenomena/audio/loop.json".to_owned(),
    ));
    let corpus = server.load::<JsonAsset>(moly_assets::audio_corpus());
    let partvoice = server.load::<JsonAsset>(moly_assets::partvoice_routes());
    commands.insert_resource(AudioRequests {
        ui: server.load("moly://ui/audio/loop.json"),
        index,
        loops,
        corpus,
        partvoice,
        categories: server.load(CATEGORY_TABLE_PATH),
    });
}

/// Where the sound configuration's category table lives in the runtime root.
const CATEGORY_TABLE_PATH: &str = "moly://phenomena/audio/categories.json";

/// Update：两份档案到达后解出路由表。结构损伤（字段缺、区间倒挂、必有的
/// 行不在、路由指向的流不在）在此响亮 panic——路由表与流表出自同一份
/// 提取产物，对不上是提取缺陷，不是运行期分支。
pub fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    requests: Option<ResMut<AudioRequests>>,
    mut bus: ResMut<VolumeBus>,
) {
    let Some(requests) = requests else {
        return; // 已解析并撤下
    };
    for (name, handle) in [
        ("index", &requests.index),
        ("loop", &requests.loops),
        ("ui", &requests.ui),
        ("corpus", &requests.corpus),
    ] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("音频路由档案 {name} 装载失败：{err:?}");
        }
    }
    if !server.is_loaded(&requests.index)
        || !server.is_loaded(&requests.loops)
        || !server.is_loaded(&requests.corpus)
        || !server.is_loaded(&requests.ui)
    {
        return; // 还在装
    }
    // partvoice 路由表：三份必达之外的可缺席档案。文件不在＝路由面缺席
    // ——不 panic（消费侧按 fail-closed 具名跳过）；
    // Loaded 但结构损伤照样响亮（表在盘上却对不上形，是提取缺陷）。
    // 在途（Loading）则等它一起吃：请求资源一次性撤下，不留在半途。
    let partvoice_map = match server.load_state(&requests.partvoice) {
        LoadState::Failed(_) => {
            warn!(
                "partvoice 路由表缺席：partvoice 步全部具名跳过（不推默认包），voice_ 族不受影响"
            );
            None
        }
        LoadState::Loaded => {
            let doc = jsons
                .get(&requests.partvoice)
                .unwrap_or_else(|| panic!("partvoice.json 已装载但不在资产表里"));
            let doc: serde_json::Value = serde_json::from_str(&doc.0)
                .unwrap_or_else(|err| panic!("partvoice.json 不是合法 JSON：{err}"));
            Some(parse_partvoice(&doc))
        }
        _ => return, // 在途：下一帧再吃（本地文件一帧内必达）
    };
    let categories = match server.load_state(&requests.categories) {
        LoadState::Failed(_) => None,
        LoadState::Loaded => {
            let doc = jsons
                .get(&requests.categories)
                .unwrap_or_else(|| panic!("categories.json 已装载但不在资产表里"));
            Some(
                serde_json::from_str::<serde_json::Value>(&doc.0)
                    .unwrap_or_else(|err| panic!("categories.json 不是合法 JSON：{err}")),
            )
        }
        _ => return,
    };
    match &categories {
        Some(doc) => apply_category_table(&mut bus, doc),
        None => error!(
            "[audio] category table absent ({CATEGORY_TABLE_PATH}): every category volume stays 1.0 \
             (the configuration's are 0.75-0.90); extraction gap, the table is read from the sound configuration"
        ),
    }
    apply_volume_overrides(&mut bus);
    let Some(index) = jsons.get(&requests.index) else {
        panic!("音频 index.json 不在资产表里");
    };
    let Some(loops) = jsons.get(&requests.loops) else {
        panic!("音频 loop.json 不在资产表里");
    };
    let Some(corpus) = jsons.get(&requests.corpus) else {
        panic!("音频 corpus.json 不在资产表里");
    };
    let index: serde_json::Value = serde_json::from_str(&index.0)
        .unwrap_or_else(|err| panic!("音频 index.json 不是合法 JSON：{err}"));
    let loops: serde_json::Value = serde_json::from_str(&loops.0)
        .unwrap_or_else(|err| panic!("音频 loop.json 不是合法 JSON：{err}"));
    let corpus: serde_json::Value = serde_json::from_str(&corpus.0)
        .unwrap_or_else(|err| panic!("音频 corpus.json 不是合法 JSON：{err}"));
    let mut routing = parse_routing(&index, &loops);
    let ui: serde_json::Value =
        serde_json::from_str(&jsons.get(&requests.ui).expect("UI audio loaded").0)
            .expect("UI audio catalog");
    routing.streams.0.extend(parse_streams(&ui, "").0);
    let voice_corpus = parse_corpus(&corpus);
    info!(
        "音频路由表就绪：站点 BGM {} 行 · 站点「其它」环境音 {} 行 · 现象 {} 档 · {} 个流键（按站点 × 现象现算）",
        routing.site_bgms.len(),
        routing.site_sound_fallbacks.len(),
        routing.phenomena.len(),
        routing.streams.0.len(),
    );
    info!(
        "序列结构就绪：读过的 (cue, 包) {} 个 · 带结构块 {} 个（洗牌 {} · 其它类型 {}）· 按轨取的流 {} 条",
        routing.sequences.read.len(),
        routing.sequences.blocks.len(),
        routing
            .sequences
            .blocks
            .values()
            .filter(|block| block.kind == SEQUENCE_TYPE_SHUFFLE)
            .count(),
        routing
            .sequences
            .blocks
            .values()
            .filter(|block| block.kind != SEQUENCE_TYPE_SHUFFLE)
            .count(),
        routing.sequences.subsongs.len(),
    );
    if !routing.sequences.refused.is_empty() {
        info!(
            "sequence exports refused: {} package(s), each named in an ERROR line",
            routing.sequences.refused.len()
        );
    }
    info!(
        "talk voice 语料账本就绪：上游无包 {} 条具名（缺 cue 日志的分类依据）",
        voice_corpus.uncovered.len(),
    );
    info!(
        "音量两层当前值（播放器层＝本地档 System 组；类别层＝配置类别表 {} + 环境变量覆写）：{}",
        if categories.is_some() {
            "已读"
        } else {
            "缺席"
        },
        bus.describe()
    );
    info!(
        "cue gain facts: {} keys (categories and volume commands); {} of them carry a random volume spread (base value used, spread not ported)",
        routing.sequences.gains.len(),
        routing
            .sequences
            .gains
            .values()
            .filter(|gain| gain.random_spread)
            .count(),
    );
    commands.insert_resource(routing);
    commands.insert_resource(voice_corpus);
    if let Some(map) = partvoice_map {
        info!(
            "partvoice 路由表就绪：参与者 {} 员（门拒 {}）· 家具 {} 件",
            map.participants.len(),
            map.participants
                .values()
                .filter(|route| matches!(route, ParticipantRoute::Gated(_)))
                .count(),
            map.fixtures.len(),
        );
        commands.insert_resource(map);
    }
    commands.remove_resource::<AudioRequests>();
}

/// 解析两份档案为路由表。行重复、亮度档不在枚举里、循环区间倒挂、任何
/// 一条可达路由指向的流不在，都在此 panic——这些是提取产物的结构损伤，
/// 不是运行期分支。
fn parse_routing(index: &serde_json::Value, loops: &serde_json::Value) -> Routing {
    // (site, 亮度档) → BGM 路由（BGM 管理器的站点选曲表，含 none 档的死行）。
    let mut site_bgms: HashMap<(i64, Brightness), RouteCue> = HashMap::new();
    for row in index["siteBgms"]
        .as_array()
        .unwrap_or_else(|| panic!("siteBgms 不是数组"))
    {
        let site = field_i64(row, "siteId");
        let brightness_name = field_str(row, "brightnessType");
        let brightness = Brightness::parse(brightness_name).unwrap_or_else(|| {
            panic!("siteBgms 里 site {site} 的亮度档 {brightness_name:?} 不在枚举里")
        });
        if site_bgms
            .insert((site, brightness), RouteCue::from_row(row))
            .is_some()
        {
            panic!("siteBgms 里 site {site} 的 {brightness_name} 档重复");
        }
    }
    // 站点表里出现的每一站都要有三个亮度档：任何一档现象都可能落到它。
    let mut sites: Vec<i64> = site_bgms.keys().map(|(site, _)| *site).collect();
    sites.sort_unstable();
    sites.dedup();
    for site in &sites {
        for brightness in [Brightness::Normal, Brightness::Bright, Brightness::Dark] {
            if !site_bgms.contains_key(&(*site, brightness)) {
                panic!("siteBgms 里 site {site} 缺 {} 档", brightness.label());
            }
        }
    }
    // The site's "other" condition ambience rows (first row per site, the
    // same as FirstOrDefault in the original game).
    let mut site_sound_fallbacks: HashMap<i64, RouteCue> = HashMap::new();
    for row in index["siteSoundFallbacks"]
        .as_array()
        .unwrap_or_else(|| panic!("siteSoundFallbacks 不是数组"))
    {
        site_sound_fallbacks
            .entry(field_i64(row, "siteId"))
            .or_insert_with(|| RouteCue::from_row(row));
    }

    let entries = index["phenomena"]
        .as_object()
        .unwrap_or_else(|| panic!("phenomena 不是对象表"));
    let mut phenomena: HashMap<String, PhenomenonAudio> = HashMap::new();
    for (name, entry) in entries {
        // A null master is the delivery phenomenon of the festival venue (the
        // delivery id lives in the client configuration; in this table "no
        // master row" is its form).
        let master_brightness = entry["master"].as_object().map(|master| {
            let value = master["brightnessType"]
                .as_str()
                .unwrap_or_else(|| panic!("现象 {} 的 brightnessType 不是字符串", label(name)));
            Brightness::parse(value)
                .unwrap_or_else(|| panic!("现象 {} 的亮度档 {value:?} 不在枚举里", label(name)))
        });
        // 现象 BGM 字典按 mysekaiPhenomenaId 建、重复键后者覆盖前者：取最后一行。
        let own_bgm = entry["bgms"]
            .as_array()
            .and_then(|rows| rows.last())
            .map(RouteCue::from_row);
        if master_brightness == Some(Brightness::None) && own_bgm.is_none() {
            panic!("现象 {} 亮度档是 none 却没有 bgms 行", label(name));
        }
        let mut site_sounds: HashMap<i64, RouteCue> = HashMap::new();
        for row in entry["siteSounds"].as_array().into_iter().flatten() {
            site_sounds
                .entry(field_i64(row, "siteId"))
                .or_insert_with(|| RouteCue::from_row(row));
        }
        phenomena.insert(
            name.clone(),
            PhenomenonAudio {
                id: field_i64(entry, "id"),
                master_brightness,
                own_bgm,
                site_sounds,
            },
        );
    }

    let sites_with_sounds: HashSet<i64> = site_sound_fallbacks
        .keys()
        .copied()
        .chain(
            phenomena
                .values()
                .flat_map(|entry| entry.site_sounds.keys().copied()),
        )
        .collect();
    let routing = Routing {
        site_bgms,
        site_sound_fallbacks,
        sites_with_sounds,
        phenomena,
        streams: parse_streams(loops, "phenomena/"),
        sequences: parse_sequences(loops, "phenomena/"),
    };
    // 任何站点 × 现象可达的路由都必须落到一条解码好的流上。解析期全量核过
    // 之后，消费侧就不再需要「cue 无流」的运行期分支。
    let reachable = routing
        .site_bgms
        .values()
        .map(|route| ("BGM", route))
        .chain(
            routing
                .site_sound_fallbacks
                .values()
                .map(|route| ("环境音", route)),
        )
        .chain(routing.phenomena.values().flat_map(|entry| {
            entry
                .own_bgm
                .iter()
                .map(|route| ("BGM", route))
                .chain(entry.site_sounds.values().map(|route| ("环境音", route)))
        }));
    for (kind, route) in reachable {
        if !routing
            .streams
            .0
            .contains_key(&(route.cue.clone(), route.package.clone()))
        {
            panic!(
                "{kind}路由指向的流不在表里：{} @ {}",
                label(&route.cue),
                label(&route.package)
            );
        }
    }
    routing
}

/// 解 loop.json 为流表（见 [`Streams`]）。诊断行（提取侧对无波形 cue 写的
/// {cue, error}）具名跳过；可播放行字段缺失或循环区间倒挂响亮拒绝。
fn parse_streams(loops: &serde_json::Value, base: &str) -> Streams {
    // 流表：(cue, package) →（最小 subsong 的）一条流 + 该键的波形条数。
    let mut streams: HashMap<(String, String), (i64, StreamRow)> = HashMap::new();
    for package in loops["packages"]
        .as_array()
        .unwrap_or_else(|| panic!("loop.json 的 packages 不是数组"))
    {
        let package_name = field_str(package, "package").to_string();
        for stream in package["streams"]
            .as_array()
            .unwrap_or_else(|| panic!("loop.json 的 streams 不是数组"))
        {
            let cue = field_str(stream, "cue");
            // The extractor explicitly emits {cue, error} for a requested cue
            // with no source waveform (phenomena/audio.py, NO_CUE). It is a
            // diagnostic variant, not a playable stream with missing fields.
            // Do not invent loop=false, a path, or another cue as a substitute.
            if let Some(diagnostic) = stream.get("error") {
                let reason = diagnostic
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .expect("audio diagnostic error must be a nonempty string");
                assert_eq!(
                    stream.as_object().map(|row| row.len()),
                    Some(2),
                    "audio diagnostic cannot also contain partial playable fields"
                );
                warn!("[audio-source] unavailable cue={cue} package={package_name}: {reason}");
                continue;
            }
            let cue_label = label(cue);
            // 单流键 subsong 是空值（自成一包的下载件一族）；多流键里全是
            // 编号——空值折算为最大，必输给任何编号。
            let subsong = stream["subsong"].as_i64().unwrap_or(i64::MAX);
            let loops_flag = stream["loop"]
                .as_bool()
                .unwrap_or_else(|| panic!("流 {cue_label} 的 loop 不是布尔"));
            let (loop_start, loop_end) = if loops_flag {
                let start = field_f64(stream, "loopStartSeconds");
                let end = field_f64(stream, "loopEndSeconds");
                if end <= start {
                    panic!("流 {cue_label} 的循环区间倒挂：{start:.3}..{end:.3}");
                }
                (start, end)
            } else {
                (0.0, 0.0)
            };
            let source = stream["ogg"]
                .as_str()
                .or_else(|| stream["wav"].as_str())
                .unwrap_or_else(|| panic!("流 {cue_label} 既无 ogg 也无 wav 路径"))
                .to_string();
            let row = StreamRow {
                ogg: format!("{base}{source}"),
                loops: loops_flag,
                loop_start,
                loop_end,
                duration: field_f64(stream, "durationSeconds"),
                waveforms: 1,
            };
            let key = (cue.to_string(), package_name.clone());
            match streams.get_mut(&key) {
                // 同键多流（多 subsong）：多轨序列 cue。保留最小 subsong 一条
                // 并累计波形条数（环境音通道据此具名拒绝，见 StreamRow）。
                Some((existing_subsong, existing_row)) => {
                    let waveforms = existing_row.waveforms + 1;
                    if subsong < *existing_subsong {
                        *existing_subsong = subsong;
                        *existing_row = row;
                    }
                    existing_row.waveforms = waveforms;
                }
                None => {
                    streams.insert(key, (subsong, row));
                }
            }
        }
    }


    Streams(streams.into_iter().map(|(k, (_, v))| (k, v)).collect())
}

/// 提取侧序列导出的版本（`sequenceExport.version`）。别的版本号响亮拒绝：
/// 字段语义可能变了，按旧读法读会静默读错。
const SEQUENCE_EXPORT_VERSION: i64 = 1;
/// Version 2 adds, per entry, `cueCommands` (every listed cue's command
/// lists in the block shape, plain cues included) and `acbVersion` (the cue
/// sheet header's format number).
const SEQUENCE_EXPORT_VERSION_GAINS: i64 = 2;

/// 按轨取的一条流（多轨序列的一轨对应一个 subsong）。
#[derive(Clone, Debug)]
struct SubStream {
    ogg: String,
    loops: bool,
    loop_start: f64,
    loop_end: f64,
}

/// The noteOn target of one track (only the one shape the playback side
/// supports is distinguished).
#[derive(Clone, Debug)]
enum NoteOn {
    /// noteOn -> synth -> exactly one waveform reference; `subsong` is the
    /// decoder's stream number (stored waveforms have no stream number).
    Waveform { subsong: Option<i64> },
    /// Every other shape (a synth referencing several waveforms, a nested
    /// synth, a noteOn pointing at a sequence, ...): not supported.
    Other(String),
}

/// One track of a sequence.
#[derive(Clone, Debug)]
struct SequenceTrack {
    /// Track number (the cue sheet's track row index): this is what the order
    /// table stores.
    track_index: u16,
    /// Start delay in microseconds (millisecond term ×1000 + microsecond term,
    /// already combined by the asset export).
    start_delay_us: i64,
    /// Event codes the asset export did not decode from the event program.
    /// Non-empty = this track does more than is supported.
    events_not_decoded: Vec<i64>,
    note_ons: Vec<NoteOn>,
}

/// 一个 cue 的序列结构（提取侧 `sequences` 块里消费侧读的那部分）。
#[derive(Clone, Debug)]
struct SequenceBlock {
    /// 所在归档（同包多归档时区分工作区）。
    archive: String,
    /// 序列行号：工作区挂在 cue 表的序列行上，同一行的别名 cue 共用。
    sequence_index: u16,
    kind: u8,
    type_name: String,
    /// 重放标志（序列命令里的那一字节；无此命令 = `None`）。
    repeat: Option<u8>,
    playback_ratio: i64,
    action_tracks: i64,
    plugin_tracks: i64,
    tracks: Vec<SequenceTrack>,
    /// 消费侧不读的参数，原样记一行（总线送量、参数 69、未解码的命令码）：
    /// 本仓的混音器没有总线效果器，这些不进声音。
    not_consumed: String,
    /// Per-track weights of the random types (empty = the cue has none).
    track_values: Vec<u16>,
    /// The cue sheet's format number (export version 2), `None` before.
    acb_version: Option<u32>,
}

/// 提取侧导出的序列结构（loop.json 包条目的 `sequenceExport`/`sequences`）。
///
/// 读法：条目带 `sequenceExport` ＝ 提取侧读过这个归档的 cue 表。它列了、
/// 却没有块的 cue 是 plain（一轨一条波形、从 cue 起点放）。有块的 cue
/// 按块里的结构放或具名拒绝。条目不带 `sequenceExport` ＝ 从没读过，
/// 那时只能看波形条数：多于一条具名拒绝。
#[derive(Default)]
struct SequenceIndex {
    blocks: HashMap<(String, String), SequenceBlock>,
    read: HashSet<(String, String)>,
    subsongs: HashMap<(String, String, i64), SubStream>,
    /// Gain facts per (cue, package): every structured cue, and with export
    /// version 2 every listed cue.
    gains: HashMap<(String, String), CueGain>,
    /// Packages whose export this reader refused, with the reason: every cue
    /// of such a package is refused by name when started.
    refused: HashMap<String, String>,
}

/// An integer of the sequence export; `Err` names the field that is missing
/// or not an integer.
fn value_i64(value: &serde_json::Value, what: &str) -> Result<i64, String> {
    value
        .as_i64()
        .ok_or_else(|| format!("{what} is not an integer"))
}

/// An integer of the sequence export in a narrower type; `Err` names the
/// field and its value.
fn value_in<T: TryFrom<i64>>(value: &serde_json::Value, what: &str) -> Result<T, String> {
    let wide = value_i64(value, what)?;
    T::try_from(wide).map_err(|_| format!("{what} {wide} is out of range"))
}

/// One cue's sequence block; `Err` names the field of the block that does
/// not have the exported shape.
fn parse_sequence_block(
    block: &serde_json::Value,
    archive: &str,
    acb_version: Option<u32>,
) -> Result<SequenceBlock, String> {
    let kind = value_in::<u8>(&block["type"], "type")?;
    let repeat = match &block["repeat"] {
        serde_json::Value::Null => None,
        value => Some(value_in::<u8>(value, "repeat")?),
    };
    let sequence_index = value_in::<u16>(&block["sequenceIndex"], "sequenceIndex")?;
    let mut tracks = Vec::new();
    for track in block["tracks"]
        .as_array()
        .ok_or("tracks is not an array")?
    {
        let track_index = value_in::<u16>(&track["trackIndex"], "trackIndex")?;
        let mut note_ons = Vec::new();
        for note in track["noteOns"]
            .as_array()
            .ok_or_else(|| format!("noteOns of track {track_index} is not an array"))?
        {
            let note_kind = value_i64(&note["kind"], "noteOn kind")?;
            let references = note["synth"]["references"].as_array();
            note_ons.push(match references {
                Some(items)
                    if note_kind == 2
                        && items.len() == 1
                        && value_i64(&items[0]["kind"], "reference kind")? == 1 =>
                {
                    NoteOn::Waveform {
                        subsong: items[0]["waveform"]["subsong"].as_i64(),
                    }
                }
                Some(items) if note_kind == 2 => {
                    NoteOn::Other(format!("synth 引用 {} 条（要恰一条波形）", items.len()))
                }
                _ => NoteOn::Other(format!("noteOn 指向种类 {note_kind}（要 synth）")),
            });
        }
        let mut events_not_decoded = Vec::new();
        for code in track["eventCodesNotDecoded"].as_array().ok_or_else(|| {
            format!("eventCodesNotDecoded of track {track_index} is not an array")
        })? {
            events_not_decoded.push(value_i64(code, "event code")?);
        }
        tracks.push(SequenceTrack {
            track_index,
            start_delay_us: value_i64(
                &track["startDelayMicroseconds"],
                "startDelayMicroseconds",
            )?,
            events_not_decoded,
            note_ons,
        });
    }
    let not_consumed = format!(
        "总线送量 {} · 参数69 {} · 未解码命令码 {}",
        block["busSends"], block["parameter69"], block["commandCodesNotDecoded"]
    );
    let hex = block["trackValues"].as_str().unwrap_or("");
    if !hex.len().is_multiple_of(4) || !hex.is_ascii() {
        return Err(format!("trackValues {hex:?} is not whole 16-bit values"));
    }
    let track_values = (0..hex.len())
        .step_by(4)
        .map(|i| {
            u16::from_str_radix(&hex[i..i + 4], 16)
                .map_err(|_| format!("trackValues {hex:?} is not hexadecimal"))
        })
        .collect::<Result<_, _>>()?;
    Ok(SequenceBlock {
        archive: archive.to_string(),
        sequence_index,
        kind,
        type_name: block["typeName"].as_str().unwrap_or("未命名").to_string(),
        repeat,
        playback_ratio: value_i64(&block["playbackRatio"], "playbackRatio")?,
        action_tracks: value_i64(&block["actionTracks"]["count"], "actionTracks.count")?,
        plugin_tracks: value_i64(
            &block["instrumentPluginTracks"]["count"],
            "instrumentPluginTracks.count",
        )?,
        tracks,
        not_consumed,
        track_values,
        acb_version,
    })
}

/// What one package entry's sequence export adds to the index.
#[derive(Default)]
struct PackageExport {
    blocks: Vec<(String, SequenceBlock)>,
    read: Vec<String>,
    /// Gain facts of the `cueCommands` rows (export version 2).
    commands: Vec<(String, CueGain)>,
    /// Gain facts of the blocks of cues without a `cueCommands` row.
    block_gains: Vec<(String, CueGain)>,
}

/// One package entry's sequence export; `Err` names the first part of it
/// this reader does not take (an export version it does not know, a field
/// that does not have the exported shape, a table the version requires that
/// is absent).
fn parse_package_export(
    package: &serde_json::Value,
    export: &serde_json::Value,
    streams: &[serde_json::Value],
) -> Result<PackageExport, String> {
    let version = value_i64(&export["version"], "sequenceExport.version")?;
    if version != SEQUENCE_EXPORT_VERSION && version != SEQUENCE_EXPORT_VERSION_GAINS {
        return Err(format!(
            "export version {version} is not one this reader takes ({SEQUENCE_EXPORT_VERSION} or {SEQUENCE_EXPORT_VERSION_GAINS})"
        ));
    }
    let mut parsed = PackageExport::default();
    let no_rows = serde_json::Map::new();
    let (acb_version, rows) = if version >= SEQUENCE_EXPORT_VERSION_GAINS {
        let acb_version = value_in::<u32>(&export["acbVersion"], "sequenceExport.acbVersion")?;
        let rows = package["cueCommands"]
            .as_object()
            .ok_or_else(|| format!("export version {version} has no cueCommands table"))?;
        (Some(acb_version), rows)
    } else {
        (None, &no_rows)
    };
    for (cue, row) in rows {
        let gain = cue::gain_from_block(row)
            .map_err(|reason| format!("cueCommands of cue {}: {reason}", label(cue)))?;
        parsed.commands.push((cue.clone(), gain));
    }
    let archive = package["archive"]
        .as_str()
        .ok_or("archive is not a string")?;
    parsed.read = streams
        .iter()
        .filter_map(|stream| stream["cue"].as_str().map(str::to_string))
        .collect();
    let blocks = package["sequences"]
        .as_object()
        .ok_or("the entry has a sequence export but no sequences table")?;
    for (cue, block) in blocks {
        let named = |reason: String| format!("sequence block of cue {}: {reason}", label(cue));
        let parsed_block = parse_sequence_block(block, archive, acb_version).map_err(named)?;
        if !rows.contains_key(cue) {
            let gain = cue::gain_from_block(block).map_err(named)?;
            parsed.block_gains.push((cue.clone(), gain));
        }
        parsed.blocks.push((cue.clone(), parsed_block));
    }
    Ok(parsed)
}

/// 解 loop.json 的序列导出（见 [`SequenceIndex`]）。A package entry whose
/// export this reader does not take is refused by name: one ERROR line with
/// the package and the reason, and every cue of that package is refused when
/// it is started ([`Routing::cue_plan`]); the other packages are read as
/// usual.
fn parse_sequences(loops: &serde_json::Value, base: &str) -> SequenceIndex {
    let mut index = SequenceIndex::default();
    for package in loops["packages"]
        .as_array()
        .unwrap_or_else(|| panic!("loop.json 的 packages 不是数组"))
    {
        let package_name = field_str(package, "package").to_string();
        let streams = package["streams"]
            .as_array()
            .unwrap_or_else(|| panic!("loop.json 的 streams 不是数组"));
        for stream in streams {
            if stream.get("error").is_some() {
                continue; // 诊断行：parse_streams 已具名
            }
            let cue = field_str(stream, "cue").to_string();
            // 单流归档的流没有编号，在归档自己的波形容器里它是第 1 条。
            let subsong = stream["subsong"].as_i64().unwrap_or(1);
            let loops_flag = stream["loop"]
                .as_bool()
                .unwrap_or_else(|| panic!("流 {} 的 loop 不是布尔", label(&cue)));
            let (loop_start, loop_end) = if loops_flag {
                (
                    field_f64(stream, "loopStartSeconds"),
                    field_f64(stream, "loopEndSeconds"),
                )
            } else {
                (0.0, 0.0)
            };
            let source = stream["ogg"]
                .as_str()
                .or_else(|| stream["wav"].as_str())
                .unwrap_or_else(|| panic!("流 {} 既无 ogg 也无 wav 路径", label(&cue)));
            index.subsongs.insert(
                (cue, package_name.clone(), subsong),
                SubStream {
                    ogg: format!("{base}{source}"),
                    loops: loops_flag,
                    loop_start,
                    loop_end,
                },
            );
        }
        let Some(export) = package.get("sequenceExport") else {
            continue; // 从没读过这个归档的 cue 表
        };
        if index.refused.contains_key(&package_name) {
            continue; // another entry of this package was refused and named
        }
        let parsed = parse_package_export(package, export, streams).and_then(|parsed| {
            // A package's archives are separate entries; two blocks for one
            // cue of the package cannot both be the cue's structure.
            match parsed.blocks.iter().find(|(cue, _)| {
                index
                    .blocks
                    .contains_key(&(cue.clone(), package_name.clone()))
            }) {
                Some((cue, _)) => Err(format!(
                    "cue {} has a sequence block in two entries of the package",
                    label(cue)
                )),
                None => Ok(parsed),
            }
        });
        match parsed {
            Ok(parsed) => {
                for cue in parsed.read {
                    index.read.insert((cue, package_name.clone()));
                }
                for (cue, gain) in parsed.commands {
                    index.gains.insert((cue, package_name.clone()), gain);
                }
                for (cue, gain) in parsed.block_gains {
                    // A block's facts do not replace a cueCommands row.
                    index
                        .gains
                        .entry((cue, package_name.clone()))
                        .or_insert(gain);
                }
                for (cue, block) in parsed.blocks {
                    index.blocks.insert((cue, package_name.clone()), block);
                }
            }
            Err(reason) => {
                error!(
                    "[audio] sequence export of package {} refused: {reason}; every cue of the package is refused when started, and its cue gain facts are absent",
                    label(&package_name)
                );
                index.refused.insert(package_name, reason);
            }
        }
    }
    index
}

/// 一轨的起播计划：轨号、起播延迟、要放的流。
#[derive(Clone, Debug)]
struct PlanVoice {
    track_index: u16,
    start_delay_us: i64,
    stream: SubStream,
}

/// What one start of a cue plays from: the sequence type, the authored track
/// ids (the shuffle's order table holds them), the weights of the random
/// types, one voice per track, the repeat flag, the gain facts. A plain cue is
/// one polyphonic track without a work area.
#[derive(Clone, Debug)]
pub struct CuePlan {
    kind: SequenceKind,
    /// (package, archive, sequence row): where the position and order live.
    work_key: Option<(String, String, u16)>,
    track_ids: Vec<u16>,
    weights: Option<Vec<u16>>,
    voices: Vec<PlanVoice>,
    repeat: bool,
    gain: Option<CueGain>,
    /// A structured cue's parameters that do not reach the sound here (bus
    /// sends to the effect buses, parameter 69, command codes not decoded).
    not_consumed: Option<String>,
}

/// The plan of a structured cue; `Err` names what of its structure is not
/// supported.
fn structured_plan(
    index: &SequenceIndex,
    cue: &str,
    package: &str,
    block: &SequenceBlock,
) -> Result<CuePlan, String> {
    let kind = SequenceKind::from_code(block.kind).ok_or_else(|| {
        format!(
            "sequence type {} ({}) is not supported by this crate (types 0-4 are supported)",
            block.kind, block.type_name
        )
    })?;
    if kind == SequenceKind::RandomNoRepeat
        && block
            .acb_version
            .is_some_and(|version| version >= NO_REPEAT_HISTORY_FORMAT)
    {
        return Err(format!(
            "a no-repeat random sequence in cue sheet format {:#x} selects tracks from a history table, which is not supported (this crate supports position-based selection)",
            block.acb_version.unwrap_or_default()
        ));
    }
    if block.playback_ratio != 100 {
        return Err(format!(
            "序列的播放倍率是 {}（本仓只按 100 走时）",
            block.playback_ratio
        ));
    }
    if block.action_tracks != 0 || block.plugin_tracks != 0 {
        return Err(format!(
            "sequence has {} action track(s) and {} plugin track(s) (not supported)",
            block.action_tracks, block.plugin_tracks
        ));
    }
    if block.tracks.is_empty() {
        return Err("序列没有轨".to_string());
    }
    let mut voices = Vec::with_capacity(block.tracks.len());
    for track in &block.tracks {
        if !track.events_not_decoded.is_empty() {
            return Err(format!(
                "轨 {} 的事件程序里有提取侧没解码的事件码 {:?}",
                track.track_index, track.events_not_decoded
            ));
        }
        if track.start_delay_us < 0 {
            return Err(format!(
                "轨 {} 的起播延迟为负（{} 微秒）",
                track.track_index, track.start_delay_us
            ));
        }
        let [note] = track.note_ons.as_slice() else {
            return Err(format!(
                "轨 {} 有 {} 个 noteOn（本仓只放恰一个的轨）",
                track.track_index,
                track.note_ons.len()
            ));
        };
        let subsong = match note {
            NoteOn::Waveform {
                subsong: Some(subsong),
            } => *subsong,
            NoteOn::Waveform { subsong: None } => {
                return Err(format!(
                    "轨 {} 的波形没有流号（外存波形）",
                    track.track_index
                ))
            }
            NoteOn::Other(shape) => {
                return Err(format!("轨 {} 的 noteOn：{shape}", track.track_index))
            }
        };
        let stream = index
            .subsongs
            .get(&(cue.to_string(), package.to_string(), subsong))
            .ok_or_else(|| {
                format!(
                    "轨 {} 要的流 {subsong} 没有解码进运行时根",
                    track.track_index
                )
            })?;
        voices.push(PlanVoice {
            track_index: track.track_index,
            start_delay_us: track.start_delay_us,
            stream: stream.clone(),
        });
    }
    Ok(CuePlan {
        kind,
        work_key: Some((
            package.to_string(),
            block.archive.clone(),
            block.sequence_index,
        )),
        track_ids: block.tracks.iter().map(|track| track.track_index).collect(),
        weights: (!block.track_values.is_empty()).then(|| block.track_values.clone()),
        voices,
        repeat: block.repeat.is_some_and(|flag| flag != 0),
        not_consumed: Some(block.not_consumed.clone()),
        gain: index
            .gains
            .get(&(cue.to_string(), package.to_string()))
            .cloned(),
    })
}

impl Routing {
    /// The plan one start of `(cue, package)` plays. `Err` is a named reason:
    /// a structure this port does not play, a multi-waveform key whose entry
    /// the sequence export never read (an extraction gap: the export reads
    /// the archive's cue table), or a key with no decoded stream.
    pub fn cue_plan(&self, cue: &str, package: &str) -> Result<CuePlan, String> {
        if let Some(reason) = self.sequences.refused.get(package) {
            return Err(format!("the sequence export of the package was refused: {reason}"));
        }
        let key = (cue.to_string(), package.to_string());
        if let Some(block) = self.sequences.blocks.get(&key) {
            return structured_plan(&self.sequences, cue, package, block)
                .map_err(|reason| format!("{} 序列：{reason}", block.type_name));
        }
        let Some(stream) = self.streams.0.get(&key) else {
            return Err("不在流表里".to_string());
        };
        if self.sequences.read.contains(&key) && stream.waveforms != 1 {
            return Err(format!(
                "序列导出判为 plain，流表里却有 {} 条波形",
                stream.waveforms
            ));
        }
        if stream.waveforms > 1 {
            return Err(format!(
                "带 {} 条波形——这是多轨序列 cue，它所在的条目没有序列导出（提取缺口：提取侧没读过这个归档的 cue 表）",
                stream.waveforms
            ));
        }
        Ok(CuePlan {
            kind: SequenceKind::Polyphonic,
            work_key: None,
            track_ids: vec![0],
            weights: None,
            voices: vec![PlanVoice {
                track_index: 0,
                start_delay_us: 0,
                stream: SubStream {
                    ogg: stream.ogg.clone(),
                    loops: stream.loops,
                    loop_start: stream.loop_start,
                    loop_end: stream.loop_end,
                },
            }],
            repeat: false,
            gain: self.sequences.gains.get(&key).cloned(),
            not_consumed: None,
        })
    }

    /// Every stream the table holds, as its cue and package.
    pub fn stream_keys(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.streams.0.keys().cloned()
    }

    /// The whole length of a `(cue, package)` stream in seconds, when the
    /// table holds it.
    pub fn stream_length(&self, cue: &str, package: &str) -> Option<f32> {
        self.streams.0.get(&(cue.to_owned(), package.to_owned())).map(|stream| stream.duration as f32)
    }

    /// The gain facts of `(cue, package)`, when exported.
    fn cue_gain(&self, cue: &str, package: &str) -> Option<&CueGain> {
        self.sequences
            .gains
            .get(&(cue.to_string(), package.to_string()))
    }

    /// Timeline SE clips prepare their sounds through this.
    #[allow(dead_code)]
    /// The waveform paths one cue can play, in track order (what a caller
    /// that prepares its sounds before starting must request), or the named
    /// reason it cannot be played.
    pub fn cue_asset_paths(&self, package: &str, cue: &str) -> Result<Vec<String>, String> {
        self.cue_plan(cue, package).map(|plan| {
            plan.voices
                .iter()
                .map(|voice| voice.stream.ogg.clone())
                .collect()
        })
    }
}

/// JSON 行取字符串字段：缺字段是结构损伤，响亮拒绝。
fn field_str<'a>(row: &'a serde_json::Value, key: &str) -> &'a str {
    row[key]
        .as_str()
        .unwrap_or_else(|| panic!("音频表行缺字符串字段 {key}"))
}

/// JSON 行取整数字段：同上（subsong 允许空值，取值处自行处理）。
fn field_i64(row: &serde_json::Value, key: &str) -> i64 {
    row[key]
        .as_i64()
        .unwrap_or_else(|| panic!("音频表行缺整数字段 {key}"))
}

/// talk voice 语料账本（corpus.json 的消费切片）：上游无包 cue 的具名
/// 清单——缺 cue 日志拿它分类「具名上游缺口」与「未提取消费面」。
#[derive(Resource)]
pub struct VoiceCorpus {
    uncovered: Vec<String>,
}

/// 解析语料账本的 talk-voice 切片。结构损伤（数组缺、行缺键）在此响亮
/// panic——与路由表同一条资产边界纪律。
fn parse_corpus(corpus: &serde_json::Value) -> VoiceCorpus {
    let uncovered = corpus["families"]["talk-voice"]["uncovered"]
        .as_array()
        .unwrap_or_else(|| panic!("corpus.json 的 talk-voice.uncovered 不是数组"))
        .iter()
        .map(|row| {
            row["cue"]
                .as_str()
                .unwrap_or_else(|| panic!("corpus.json 的 uncovered 行缺 cue"))
                .to_string()
        })
        .collect();
    VoiceCorpus { uncovered }
}

/// talk voice cue 的路由行：cue = `voice_<script>_<行>_<变体>`，包随
/// script（提取侧 audio_corpus 的映射语义——「the consumer never asks
/// for a package; it plays cues」）；绑定键仍取 (cue, package)。
/// 非 `voice_` 前缀的 cue（partvoice 族等）不出包名，由调用方具名。
///
/// `MySekaiTalkEngine.LoadVoiceIfYetNotLoaded(lua)` picks the bank by the
/// game context, not by the script: under the Birthday context every talk
/// loads `mysekai/talk/birthday/voice/<lua>`, otherwise
/// `mysekai/talk/voice/<lua>`.
pub fn voice_package(cue: &str, birthday: bool) -> Option<String> {
    contextual_voice_package(cue, false, birthday)
}

fn contextual_voice_package(cue: &str, tutorial: bool, birthday: bool) -> Option<String> {
    let body = cue.strip_prefix("voice_")?;
    let stem = body.rsplit_once('_')?.0.rsplit_once('_')?.0;
    if stem.is_empty() {
        return None;
    }
    let prefix = if tutorial {
        TUTORIAL_TALK_VOICE_PACKAGE_PREFIX
    } else if birthday {
        BIRTHDAY_TALK_VOICE_PACKAGE_PREFIX
    } else {
        TALK_VOICE_PACKAGE_PREFIX
    };
    Some(format!("{prefix}{stem}"))
}

/// The partvoice routing table (the slice of partvoice.json consumed here):
/// speaker variant -> variant voice package, furniture character -> a
/// single-input package. In the original game's chain the package follows the
/// speaker: a main-chain participant that passes the variant gate **loads
/// both** (one on the scenario side and one on the MySekai side), while a
/// fixture character of the egg chain has no gate and loads one — only package
/// names are stored here, and whether a cue is in the package is decided by
/// the stream table (the counterpart of ExistsCueName).
#[derive(Resource)]
pub struct PartVoiceMap {
    /// characterUnitId -> main-chain route (gate passed = two packages; gate
    /// refused = a named reason — the original game loads no package for a
    /// refused participant).
    participants: HashMap<i32, ParticipantRoute>,
    /// mysekaiFixtureId → 蛋链单参包名。
    fixtures: HashMap<i32, String>,
}

/// 主链参与者的路由结果。
enum ParticipantRoute {
    /// 门过：两份包名（扁平形，与流表键同构；scenario 侧包在流表里可以
    /// 还没解出——提取侧族在途时 cue 查不到，走缺 cue 具名）。
    Packages { scenario: String, mysekai: String },
    /// Gate refused: the reason is named (the original game silently loads no
    /// package; here the reason goes into the skip log).
    Gated(String),
}

/// 解析 partvoice 路由表。结构损伤（键不是整数、行不是对象、双包形缺
/// 键、值不是字符串）响亮 panic——表在盘上就对得上形，对不上是提取缺陷。
fn parse_partvoice(doc: &serde_json::Value) -> PartVoiceMap {
    let participants = doc["participants"]
        .as_object()
        .unwrap_or_else(|| panic!("partvoice.json 缺 participants 对象"))
        .iter()
        .map(|(key, row)| {
            let id: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("partvoice.json 的 participants 键 {key} 不是整数 id"));
            let route = if let Some(reason) = row.get("skip").and_then(|v| v.as_str()) {
                ParticipantRoute::Gated(reason.to_owned())
            } else {
                let package = |name: &str| {
                    row[name]
                        .as_str()
                        .unwrap_or_else(|| panic!("partvoice.json 的参与者 {id} 缺 {name} 包名"))
                };
                ParticipantRoute::Packages {
                    scenario: package("scenarioPackage").to_owned(),
                    mysekai: package("mysekaiPackage").to_owned(),
                }
            };
            (id, route)
        })
        .collect();
    let fixtures = doc["fixtures"]
        .as_object()
        .unwrap_or_else(|| panic!("partvoice.json 缺 fixtures 对象"))
        .iter()
        .map(|(key, row)| {
            let id: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("partvoice.json 的 fixtures 键 {key} 不是整数 id"));
            let package = row["mysekaiPackage"]
                .as_str()
                .unwrap_or_else(|| panic!("partvoice.json 的家具 {id} 缺 mysekaiPackage 包名"))
                .to_owned();
            (id, package)
        })
        .collect();
    PartVoiceMap {
        participants,
        fixtures,
    }
}

/// JSON 行取浮点字段：同上。
fn field_f64(row: &serde_json::Value, key: &str) -> f64 {
    row[key]
        .as_f64()
        .unwrap_or_else(|| panic!("音频表行缺浮点字段 {key}"))
}

// ---- BGM 通道 ---------------------------------------------------------------

/// A sounding BGM voice. Two-part: a one-shot intro part (before the loop
/// start) + a resident loop part (created parked, released when the intro
/// reaches its point) — the cue player splices the cue's loop range
/// sample by sample, while this crate approximates the handover with two sinks
/// (see [`BGM_HANDOFF_LEAD_SECONDS`]).
struct BgmVoice {
    cue: String,
    intro: Option<Entity>,
    loop_sink: Option<Entity>,
    /// 循环区间（起播时从流表抄来；交接与账目用）。
    loop_start: f64,
    loop_end: f64,
    /// The intro sink's position (seconds from its own start) at which the
    /// loop sink takes over.
    handoff_at: f64,
    /// loop 段已放行（intro 已到交接点或已播空）。
    handoff_done: bool,
    /// 淡入计时：`None` = 冷启全量起播（首次放曲没有旧声可让），
    /// `Some(elapsed)` = 换曲淡入中。
    fade_in: Option<f32>,
    /// The crossfade time this voice started with.
    fade_seconds: f32,
    /// 当前请求的包络音量（淡出起点与账目）；实际 sink 音量逐个校准。
    applied_volume: f32,
    /// BGM player x the cue's categories and volume commands.
    volume: CueVolume,
}

/// 淡出中的一路旧声：换曲时从现声里搬出来，按换曲时的淡化时长线性归零后拆除。
struct FadingBgm {
    entities: Vec<Entity>,
    from_volume: f32,
    elapsed: f32,
    /// The crossfade time at the change.
    seconds: f32,
}

/// Progress of a linear fade of `seconds` after `elapsed` (a zero time is
/// already complete).
fn fade_progress(elapsed: f32, seconds: f32) -> f32 {
    if seconds > 0.0 {
        (elapsed / seconds).min(1.0)
    } else {
        1.0
    }
}

/// BGM 通道运行态。
#[derive(Resource, Default)]
pub struct BgmChannel {
    voice: Option<BgmVoice>,
    fading: Vec<FadingBgm>,
    /// 已具名拒绝的 (站点, 档)：同一组合只报一次，换了组合再报。
    refused_for: Option<(u32, String)>,
    /// 已具名拒绝唱片设定的站点（设定只随站点变：在这站停留期间只报一次，
    /// 离站即清，再进再报）。
    music_refused_for: Option<u32>,
}

// The user's music-record settings (the server's
// `UserMysekaiMusicPlayFixtureSetting` rows: per site, the record the jukebox
// plays and the vocal version) are server state: the server model holds them
// (document section `userMysekaiMusicPlayFixtureSettings`) and its responses
// write the client copy `crate::audio_inputs::ClientMusicPlaySettings`.
// It is empty by default (no site has a record set; the default choice plays).
// The native instrument `MOLY_AUDIO_MOCK_MUSIC_RECORD` (`site:record:vocal`,
// comma separated, refused loudly when malformed) writes the native server
// document; the game mode does not read it. Only housing sites (categories
// `housing_home` / `housing_room`) look the setting up; what a set record
// plays is the record path in [`record`].

/// A `MysekaiBGMManager.StartFade` target: a volume, or the manager's
/// `InitialVolume`.
#[derive(Clone, Copy, Debug)]
pub enum BgmFadeTarget {
    Volume(f32),
    Initial,
}

/// `MysekaiBGMManager`'s fade of the BGM player's volume (the value
/// `SoundManager.SetupVolume` sets from the volume setting and
/// `SoundManager.SetBGMVolume` overwrites; the manager is its only other
/// writer).
/// - Setup: `InitialVolume` = `GetCurrentBGMVolume()`, the player's volume.
/// - `StartFade(target)`: a call in the frame the running fade started
///   (`Mathf.Approximately(Time.time, _fadeStartTime)`) only replaces the
///   target; otherwise the running fade is cancelled, and the target, the
///   start time (this call's `Time.time`) and the current volume
///   (`GetCurrentBGMVolume()`) are taken, and `FadeBGM` runs.
/// - `FadeBGM`: from the current volume, once per frame (the first in the
///   call's frame): `t = (Time.time - start time) * 0.5` clamped to [0, 1]
///   (`FADE_TIME` 2 s), the volume `start + (target - start) t` clamped to
///   [0, 1] goes to `SetBGMVolume`; when it is approximately the target the
///   loop ends and, on the next frame, `SetBGMVolume(target)`.
///
/// Product mapping: the player's volume is the BGM player's system volume
/// ([`PlayerVolumes::bgm`]) until the
/// manager writes one; a changed volume setting (`SetupVolume`) replaces the
/// manager's value again. Named differences: a cancelled fade hands over at
/// once (the source waits for the cancelled loop's next frame, keeping the
/// start time of the call); `Mathf.Epsilon` is taken as the smallest normal
/// float.
#[derive(Resource, Default)]
pub struct MysekaiBgmFade {
    requests: Vec<BgmFadeTarget>,
    initial: Option<f32>,
    player: Option<f32>,
    setup_value: Option<f32>,
    start: f32,
    target: f32,
    fade_start: f32,
    fading: bool,
    final_write: bool,
}

/// `Mathf.Approximately`.
fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(f32::MIN_POSITIVE * 8.0)
}

impl MysekaiBgmFade {
    /// `StartFade(target)`, run in this frame's BGM update.
    pub fn start_fade(&mut self, target: BgmFadeTarget) {
        self.requests.push(target);
    }

    /// The BGM player's volume.
    fn player_volume(&self, setup: f32) -> f32 {
        self.player.unwrap_or(setup)
    }

    fn advance(&mut self, now: f32, setup: f32) {
        if self.setup_value != Some(setup) {
            if self.setup_value.is_some() {
                self.player = None;
            }
            self.setup_value = Some(setup);
        }
        let initial = *self.initial.get_or_insert(setup);
        if self.final_write {
            self.final_write = false;
            self.player = Some(self.target);
        }
        for request in std::mem::take(&mut self.requests) {
            let target = match request {
                BgmFadeTarget::Volume(volume) => volume,
                BgmFadeTarget::Initial => initial,
            };
            self.final_write = false;
            if self.fading && approximately(now, self.fade_start) {
                self.target = target;
                continue;
            }
            self.target = target;
            self.fade_start = now;
            self.start = self.player_volume(setup);
            self.fading = true;
            info!(
                "[bgm-manager] StartFade({target:.3}): from {:.3} over 2 s (InitialVolume {initial:.3})",
                self.start
            );
        }
        if self.fading {
            let t = ((now - self.fade_start) * 0.5).clamp(0.0, 1.0);
            let current = (self.start + (self.target - self.start) * t).clamp(0.0, 1.0);
            self.player = Some(current);
            if approximately(current, self.target) {
                self.fading = false;
                self.final_write = true;
            }
        }
    }
}

/// Update：BGM 逐帧——淡出旧声、推进淡入、intro→loop 交接、按档换曲。
pub fn advance_bgm(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    phenomenon: Res<CurrentPhenomenon>,
    site: Option<Res<SiteActive>>,
    configs: Option<Res<ClientConfigs>>,
    music: Res<crate::audio_inputs::ClientMusicPlaySettings>,
    routing: Option<Res<Routing>>,
    mut channel: ResMut<BgmChannel>,
    mut sinks: Query<&mut AudioSink>,
    bgm_hold: Option<Res<crate::audio_inputs::BgmHold>>,
    mut manager_fade: ResMut<MysekaiBgmFade>,
    mut record: record::RecordParams,
    mut cut_scene: ResMut<cutscene::CutSceneAudio>,
) {
    manager_fade.advance(time.elapsed_secs(), bus.player.bgm);
    let bgm_player = manager_fade.player_volume(bus.player.bgm);
    let Some(routing) = routing else {
        return; // 路由表未就绪
    };
    let dt = time.delta_secs();

    // 淡出推进：线性归零，到点拆除。
    channel.fading.retain_mut(|fading| {
        fading.elapsed += dt;
        let level = fading.from_volume
            * (1.0 - fade_progress(fading.elapsed, fading.seconds))
            * gate.factor();
        for entity in &fading.entities {
            if let Ok(mut sink) = sinks.get_mut(*entity) {
                sink.set_volume(Volume::Linear(level));
            }
        }
        if fading.elapsed >= fading.seconds {
            for entity in &fading.entities {
                if let Ok(mut entity_commands) = commands.get_entity(*entity) {
                    entity_commands.despawn();
                }
            }
            false
        } else {
            true
        }
    });

    // 当前声的音量与交接。
    if let Some(voice) = channel.voice.as_mut() {
        let target_volume =
            BGM_VOLUME_FACTOR * voice.volume.linear_at(&bus, bgm_player) * gate.factor();
        let envelope = match &mut voice.fade_in {
            Some(elapsed) => {
                *elapsed += dt;
                fade_progress(*elapsed, voice.fade_seconds)
            }
            None => 1.0,
        };
        let volume = target_volume * envelope;
        // A player can acquire its AudioSink after this fade has already ended.
        // Reconcile each actual sink, not just the previous requested envelope.
        for entity in voice.intro.iter().chain(&voice.loop_sink) {
            if let Ok(mut sink) = sinks.get_mut(*entity) {
                if (sink.volume().to_linear() - volume).abs() > 1e-6 {
                    sink.set_volume(Volume::Linear(volume));
                }
            }
        }
        voice.applied_volume = volume;

        // Keep the handoff retryable if the loop sink has not materialized yet.
        // An absent intro here means a previously empty intro was reclaimed.
        let intro_status = voice.intro.and_then(|intro| {
            sinks.get(intro).ok().map(|sink| {
                (
                    sink.position().as_secs_f32()
                        >= voice.handoff_at as f32 - BGM_HANDOFF_LEAD_SECONDS
                        || sink.empty(),
                    sink.empty(),
                )
            })
        });
        let handoff_due = voice.intro.is_none() || intro_status.is_some_and(|(due, _)| due);
        if handoff_due && !voice.handoff_done {
            if let Some(loop_sink) = voice.loop_sink {
                if let Ok(sink) = sinks.get(loop_sink) {
                    sink.play();
                    voice.handoff_done = true;
                    info!(
                        "BGM intro 收口：{} 进入循环段 [{:.3}, {:.3}]（intro 段 {:.3}s）",
                        label(&voice.cue),
                        voice.loop_start,
                        voice.loop_end,
                        voice.handoff_at
                    );
                }
            }
        }
        if intro_status.is_some_and(|(_, empty)| empty) {
            if let Some(intro) = voice.intro.take() {
                if let Ok(mut entity_commands) = commands.get_entity(intro) {
                    entity_commands.despawn();
                }
            }
        }
    }

    // A cut-scene's BGM call stands over the channel's own choice until the
    // cut-scene ends ([`cutscene`]).
    if cutscene::stands(
        &mut cut_scene,
        &mut commands,
        &server,
        &routing,
        &mut channel,
        &bus,
        bgm_player,
        &gate,
    ) {
        return;
    }
    // 选曲：站点或客户端配置未就绪时不动（现声照响，换站途中亦然）。
    let (Some(site), Some(configs)) = (site.as_deref(), configs.as_deref()) else {
        return;
    };
    // A cannon move calls PlayBGMAsync for its destination at its
    // ChangeEnvironment step; until then the current music keeps playing.
    if bgm_hold.is_some() {
        return;
    }
    let key = (site.site_id, phenomenon.0.clone());
    // The record setting is checked only at housing-type sites (the category
    // gate of TryPlayUserSettingBGM).
    let music_setting = matches!(
        site.category.as_str(),
        CATEGORY_HOUSING_HOME | CATEGORY_HOUSING_ROOM
    )
    .then(|| music.setting(site.site_id))
    .flatten();
    // The select screen's choice stands over the setting (see [`record`]).
    let request = match record::standing_choice(&mut record, &mut channel, site.site_id, &key.1) {
        Some(record::Choice::Record {
            record_id,
            vocal_id,
        }) => Some((record_id, vocal_id, record::Origin::Screen)),
        Some(record::Choice::Default) => None,
        None => music_setting
            .map(|(record_id, vocal_id)| (record_id, vocal_id, record::Origin::Setting)),
    };
    match request {
        Some((record_id, vocal_id, origin)) => match record::advance(
            &mut record,
            &mut commands,
            &server,
            &mut channel,
            &bus,
            bgm_player,
            &gate,
            site.site_id,
            record_id,
            vocal_id,
            origin,
        ) {
            record::Step::Default => {}
            record::Step::Hold | record::Step::Keep => return,
        },
        None => channel.music_refused_for = None,
    }
    let delivery_phenomenon_id =
        i64::from(configs.int(crate::client_config::KEY_DELIVERY_PHENOMENA_ID));
    let target = match routing.bgm_route(site, &phenomenon.0, delivery_phenomenon_id) {
        Ok(target) => {
            channel.refused_for = None;
            target
        }
        Err(reason) => {
            if channel.refused_for.as_ref() != Some(&key) {
                error!(
                    "[audio] BGM unchanged: site {} · phenomenon {}: {reason} (no playback on this branch; the current sound keeps playing)",
                    site.site_id,
                    label(&phenomenon.0)
                );
                channel.refused_for = Some(key);
            }
            return;
        }
    };
    // Switch decision: by cue name (a cue with the same name does not
    // restart).
    if channel
        .voice
        .as_ref()
        .is_some_and(|voice| voice.cue == target.cue)
    {
        return;
    }
    // 现声搬进淡出，按淡出起点的现值起算。
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
            seconds: record.fade_seconds(),
        });
    }
    // 冷启 = 此刻没有任何声在响（现声无、淡出也无）——首曲全量起播。
    let cold = channel.fading.is_empty();
    let Some(stream) = routing
        .streams
        .0
        .get(&(target.cue.clone(), target.package.clone()))
    else {
        // 解析期已全量核过路由都在表里；运行期到不了这支。真到了就是表
        // 在运行中被换了形状——响亮拒绝。
        panic!(
            "BGM 档 {} 的流不在表里：{} @ {}",
            label(&phenomenon.0),
            label(&target.cue),
            label(&target.package)
        );
    };
    let cue_volume = track_volume(
        routing.cue_gain(&target.cue, &target.package),
        Player::Bgm,
        SLOT_BGM,
        0,
    );
    let target_volume = BGM_VOLUME_FACTOR * cue_volume.linear_at(&bus, bgm_player) * gate.factor();
    let volume_now = if cold { target_volume } else { 0.0 };
    let settings_volume = Volume::Linear(volume_now);
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    let (intro, loop_sink, handoff_done) = if stream.loops {
        if stream.loop_start > 0.0 {
            // 两段式：intro 一次性段 + 驻停的 loop 段。
            let intro = commands
                .spawn((
                    AudioPlayer::new(handle.clone()),
                    PlaybackSettings::ONCE
                        .with_duration(Duration::from_secs_f64(stream.loop_start))
                        .with_volume(settings_volume),
                ))
                .id();
            let loop_sink = commands
                .spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings::LOOP
                        .with_start_position(Duration::from_secs_f64(stream.loop_start))
                        .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
                        .with_volume(settings_volume)
                        .paused(),
                ))
                .id();
            (Some(intro), Some(loop_sink), false)
        } else {
            // 循环起点在 0：整轨就是循环体。
            let loop_sink = commands
                .spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings::LOOP
                        .with_duration(Duration::from_secs_f64(stream.loop_end))
                        .with_volume(settings_volume),
                ))
                .id();
            (None, Some(loop_sink), true)
        }
    } else {
        // 波形没有循环点（家园 normal 档就是这样）：BGM 播放器起播前把循环
        // 限次设成强制循环，CRI 播放器对这个档位按波形全长取模推进播放位置
        // ⇒ 整轨循环。
        let loop_sink = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::LOOP.with_volume(settings_volume),
            ))
            .id();
        (None, Some(loop_sink), true)
    };
    channel.voice = Some(BgmVoice {
        cue: target.cue.clone(),
        intro,
        loop_sink,
        loop_start: stream.loop_start,
        loop_end: stream.loop_end,
        handoff_at: stream.loop_start,
        handoff_done,
        fade_in: if cold { None } else { Some(0.0) },
        fade_seconds: record.fade_seconds(),
        applied_volume: volume_now,
        volume: cue_volume.clone(),
    });
    info!(
        "BGM 换曲：站点 {} · 档 {} → {}（{}，音量 {:.3} = {BGM_VOLUME_FACTOR:.2} × BGM 播放器 {:.2} × {:?} × cue {:.2}，{}）",
        site.site_id,
        label(&phenomenon.0),
        label(&target.cue),
        if cold {
            "冷启".to_string()
        } else {
            format!("交叉淡化 {:.2}s", record.fade_seconds())
        },
        target_volume,
        bus.player.bgm,
        cue_volume.categories,
        cue_volume.gain,
        if stream.loops {
            format!(
                "intro {:.3}s → 循环 [{:.3}, {:.3}]",
                stream.loop_start, stream.loop_start, stream.loop_end
            )
        } else {
            "无循环点，整轨强制循环".to_string()
        },
    );
}

// ---- 区域环境音通道 ----------------------------------------------------------

/// Ambience channel state: one cue playback (the playback id the original
/// game's phenomenon SE manager keeps), stopped before every call and then
/// started again (the same cue also restarts from the top — Stop+Play without
/// distinction).
#[derive(Resource, Default)]
pub struct AmbientChannel {
    /// The cue playback entity (its sounding tracks are its children).
    playback: Option<Entity>,
    /// The weather commit sequence number processed so far (every real
    /// commit equals one phenomenon SE call in the original game).
    handled_commit: Option<u64>,
    /// The cue that is sounding (for accounting).
    playing: Option<String>,
}

/// Position and order of every structured cue, by (package, archive,
/// sequence row). The middleware keeps them in the loaded cue table's
/// sequence rows: -1 at load, written only by starts. The cue tables stay
/// loaded for the whole session here, so the map is never cleared: a cue
/// stopped and started again carries on.
#[derive(Resource, Default)]
pub struct SequenceWorkAreas(pub HashMap<(String, String, u16), ShuffleWork>);

/// Update：区域环境音——每次环境提交按「站点 × 现象」选行，平铺 2D，
/// 先停后放。cue 按它的结构放（plain 或序列类型 0-4，见 `cue`），音量
/// 是 SE 播放器 × cue 的类别 × cue 的音量命令。
pub fn advance_ambient(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time<Real>>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    transition: Option<Res<WeatherTransition>>,
    site: Option<Res<SiteActive>>,
    routing: Option<Res<Routing>>,
    mut channel: ResMut<AmbientChannel>,
    mut work: ResMut<SequenceWorkAreas>,
    mut rngs: ResMut<CueRngs>,
) {
    let (Some(routing), Some(transition), Some(site)) = (routing, transition, site) else {
        return; // 路由表、天气链或站点未就绪
    };
    let (Some(serial), Some(committed)) =
        (transition.committed_serial(), transition.committed.as_ref())
    else {
        return; // 还没有任何环境提交
    };
    if channel.handled_commit == Some(serial) {
        return; // 这次提交已经处理过
    }
    if committed.site_id != site.site_id {
        return; // 提交属于上一站（换站途中）：等本站自己的提交
    }
    channel.handled_commit = Some(serial);
    let now = time.elapsed_secs_f64();
    // Every branch stops the old sound first (a running sequence together
    // with the tracks still in their delay).
    if let Some(playback) = channel.playback.take() {
        if let Ok(mut entity_commands) = commands.get_entity(playback) {
            entity_commands.despawn();
        }
        info!(
            "[audio-seq] 环境音停：cue={} t={now:.6}",
            channel.playing.as_deref().map(label).unwrap_or_default()
        );
    }
    channel.playing = None;
    let site_id = i64::from(site.site_id);
    if !plays_site_phenomena_sound(&site.site_type) {
        info!(
            "ambience stop: site {} ({}) is outside the phenomenon ambience gate (only the grassland, shore, flower garden and memorial place site types play)",
            site.site_id, site.site_type
        );
        return;
    }
    let route = match routing.ambient_route(site_id, &committed.name) {
        AmbientRoute::Play(route) => route,
        AmbientRoute::NoMaster => {
            error!(
                "[audio] ambience stop: site {site_id} · phenomenon {} has no phenomenon master table row (the original game logs an error and stops)",
                label(&committed.name)
            );
            return;
        }
        AmbientRoute::NoRow {
            site_has_rows: false,
        } => {
            error!(
                "[audio] ambience stop: site {site_id} has neither a row for phenomenon {} nor an \"other\" row, and has no row at all in the phenomenon ambience table (the original game logs an error, gets an empty name and only stops, without playing)",
                label(&committed.name)
            );
            return;
        }
        AmbientRoute::NoRow {
            site_has_rows: true,
        } => {
            info!(
                "ambience stop: site {site_id} has neither a row for phenomenon {} nor an \"other\" row (the original game gets an empty name, only stops without playing, and logs no error)",
                label(&committed.name)
            );
            return;
        }
    };
    let plan = match routing.cue_plan(&route.cue, &route.package) {
        Ok(plan) => plan,
        Err(reason) => {
            error!(
                "[audio] ambience refused: site {site_id} · phenomenon {} cue {} @ {}: {reason}; one of its waveforms is not used as a substitute",
                label(&committed.name),
                label(&route.cue),
                label(&route.package),
            );
            return;
        }
    };
    info!(
        "环境音：站点 {site_id} · 档 {} → {}（{} {} 轨 · 重放 {} · 平铺 2D · {}）",
        label(&committed.name),
        label(&route.cue),
        plan.kind.label(),
        plan.voices.len(),
        if plan.repeat { "有" } else { "无" },
        match &plan.gain {
            Some(gain) => format!(
                "类别 {:?} · cue 音量 {:.2} · 轨音量 {:?}",
                gain.categories, gain.sequence, gain.tracks
            ),
            None => "cue 的类别与音量命令未导出：按 SE_AREA_AMBIENT 类".to_string(),
        },
    );
    if let Some(parameters) = &plan.not_consumed {
        info!(
            "ambience cue {} parameters that do not reach the sound: {parameters}",
            label(&route.cue)
        );
    }
    channel.playback = Some(cue::spawn_playback(
        &mut commands,
        cue::start_cue(
            &server,
            &mut work,
            &mut rngs,
            now,
            &route.cue,
            &route.package,
            &plan,
            Player::Se,
            SLOT_SE_AREA_AMBIENT,
            RngSlot::Ambient,
            "ambient",
        ),
        &bus,
        &gate,
        now,
    ));
    channel.playing = Some(route.cue.clone());
}

/// Timeline SE clips start their cue through this.
/// Start a timeline's SE clip as the middleware plays the cue (plain or a
/// sequence of type 0-4), with the waveforms the timeline prepared from
/// [`Routing::cue_asset_paths`] (track order). Returns the playback entity:
/// despawning it stops every track, and it despawns itself when played out.
pub fn start_timeline_cue(
    world: &mut World,
    package: &str,
    cue: &str,
    handles: Vec<Handle<AudioSource>>,
) -> Result<Entity, String> {
    start_timeline_cue_at(world,package,cue,handles,None)
}

pub fn start_timeline_cue_at(world: &mut World, package: &str, cue: &str, handles: Vec<Handle<AudioSource>>, position: Option<Vec3>) -> Result<Entity,String> {
    let plan = world
        .get_resource::<Routing>()
        .ok_or_else(|| "audio routing is not ready".to_string())?
        .cue_plan(cue, package)?;
    if handles.len() != plan.voices.len() {
        return Err(format!(
            "cue {} @ {} has {} tracks, {} waveforms were prepared",
            label(cue),
            label(package),
            plan.voices.len(),
            handles.len()
        ));
    }
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let playback = world.resource_scope(|world, mut work: Mut<SequenceWorkAreas>| {
        let mut rngs = world.resource_mut::<CueRngs>();
        cue::start_cue_with_handles(
            &mut work,
            &mut rngs,
            now,
            cue,
            package,
            &plan,
            handles,
            Player::Se,
            SLOT_SE_INGAME,
            RngSlot::Timeline,
            "timeline SE",
        )
    });
    Ok(cue::spawn_playback_in_world(world, playback.spatial(position), now))
}

/// Audio-only translation for an owner simulated in a different site's local frame.
#[derive(Component,Clone,Copy)]
pub struct SourcePositionOffset(pub Vec3);

// ---- A 套邻近环境音管理器 -----------------------------------------------------

/// A sound source on the site scene side (the counterpart of the original
/// game's sound source object): cue + package name + max distance. The
/// position is the entity's global transform. In the game content a sound
/// source is not attached to a furniture prefab (exactly one site in total,
/// the waterfall at the grassland site); the `site_sound` module attaches it
/// to the expanded scene entity from the site sidecar's component table. The
/// sidecar has no package name, so the shared SE package is filled in when
/// the component is attached.
#[derive(Component)]
pub struct SoundObject {
    pub cue: String,
    pub package: String,
    /// Cutoff distance of the distance tuning; the constructor default is
    /// 10.0.
    pub max_distance: f32,
}

impl Default for SoundObject {
    fn default() -> Self {
        Self {
            cue: String::new(),
            package: String::new(),
            max_distance: 10.0,
        }
    }
}

/// Set A state: a single channel + the two name fields + the current volume +
/// whether the ready line has been reported.
///
/// Meaning of the two name fields: `pre`/`current` both empty = not started;
/// equal = the same sound source continues (the volume updates when it passes
/// the epsilon check); different = the source changes (stop the old, play the
/// new). The built-in update does not write `current` back in its
/// source-change branch (the name then never changes again and, by
/// construction, a second source change never triggers — that manager's
/// method is replaced by a hotfix at runtime). This crate implements it in
/// readable form: the source-change branch writes `current` back; without
/// that write the source-change branch would fail permanently after its first
/// run.
#[derive(Resource, Default)]
pub struct ProximityState {
    pre: String,
    current: String,
    current_volume: f32,
    playback: Option<Entity>,
    announced: bool,
}

/// Update: the Set A nearest-source manager — every frame the source with the
/// largest distance-tuned volume plays on the single channel; the start
/// volume is 0 and from the next frame the epsilon check raises it to the real
/// value (the original game passes 0 to the player at start, so the volume
/// only arrives through the next frame's update; one frame of silence is the
/// original behaviour and is reproduced).
pub fn advance_proximity(
    mut commands: Commands,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    routing: Option<Res<Routing>>,
    mut state: ResMut<ProximityState>,
    sources: Query<
        (&SoundObject, &GlobalTransform,Option<&SourcePositionOffset>),
        Without<moly_assets::scene_state::SourceInactive>,
    >,
    avatars: Query<&GlobalTransform, With<AvatarRoot>>,
    mut sinks: Query<&mut AudioSink>,
) {
    if !state.announced {
        state.announced = true;
        info!(
            "A 套就近环境音就绪：声源 {} · 站点场景侧车喂入（site_sound 挂组件）· 距离律 clamp(1-d/max,0,1) · epsilon 门",
            sources.iter().count()
        );
    }
    let Some(routing) = routing else {
        return; // 路由表未就绪
    };
    if sources.is_empty() {
        return;
    }
    // Player viewpoint: the avatar's view transform, the same transform the
    // camera follows.
    let Ok(avatar) = avatars.single() else {
        return;
    };
    let position = avatar.translation();

    // Nearest selection: a source is taken only if its volume is strictly
    // larger (a source with volume 0 is still selected when it is the only
    // one — the initial value is the int minimum reinterpreted as a float,
    // i.e. f32::MIN).
    let mut best: Option<(&SoundObject, f32)> = None;
    let mut best_volume = f32::MIN;
    for (source, transform, offset) in &sources {
        let volume = proximity_volume(position, transform.translation()+offset.map_or(Vec3::ZERO,|o|o.0), source.max_distance);
        if volume > best_volume {
            best = Some((source, volume));
            best_volume = volume;
        }
    }
    let Some((nearest, volume)) = best else {
        return;
    };
    let audible_volume = volume * gate.factor();

    if state.pre.is_empty() || state.current.is_empty() {
        // Start branch: start volume 0 (the original game passes 0 to the
        // player and the volume argument is dropped); `current_volume` is not
        // touched (the next frame's epsilon check raises it to the real
        // value).
        state.playback = play_proximity(
            &mut commands,
            &server,
            &routing.streams,
            &nearest.cue,
            &nearest.package,
        );
        state.pre = nearest.cue.clone();
        state.current = nearest.cue.clone();
        info!(
            "A 套起播：{}（音量 0.000 起播，次帧 epsilon 门拉到 {:.3}）",
            label(&nearest.cue),
            volume
        );
    } else if state.pre == state.current {
        // Same-source branch: the epsilon check updates the volume (width =
        // max(ε×8, larger×1e-6); the absolute difference is written only when
        // it passes the check, and the player is updated with "user volume ×
        // distance volume").
        let epsilon = (UNITY_MIN_FLOAT * 8.0)
            .max(state.current_volume.abs().max(audible_volume.abs()) * 1e-6);
        if (audible_volume - state.current_volume).abs() >= epsilon {
            state.current_volume = audible_volume;
            if let Some(entity) = state.playback {
                if let Ok(mut sink) = sinks.get_mut(entity) {
                    let cue_volume = track_volume(
                        routing.cue_gain(&nearest.cue, &nearest.package),
                        Player::Se,
                        SLOT_SE_AREA_AMBIENT,
                        0,
                    );
                    sink.set_volume(Volume::Linear(cue_volume.linear(&bus) * audible_volume));
                }
            }
        }
    } else {
        // Source-change branch: stop the old, play the new (this
        // implementation writes `current` back; see the type comment).
        if let Some(old) = state.playback {
            if let Ok(mut entity_commands) = commands.get_entity(old) {
                entity_commands.despawn();
            }
        }
        state.pre = state.current.clone();
        state.playback = play_proximity(
            &mut commands,
            &server,
            &routing.streams,
            &nearest.cue,
            &nearest.package,
        );
        state.current = nearest.cue.clone();
        info!(
            "A 套换源：{} → {}（音量 0.000 起播）",
            label(&state.pre),
            label(&nearest.cue)
        );
    }
}

/// 距离调音律：`clamp(1 - d/max, 0, 1)`。
fn proximity_volume(listener: Vec3, source: Vec3, max_distance: f32) -> f32 {
    let distance = (listener - source).length();
    (1.0 - distance / max_distance).clamp(0.0, 1.0)
}

/// Set A plays one cue: start volume 0 (the original game passes 0 to the
/// player at start); a cue missing from the stream table is skipped silently
/// (the same as the original player — Set A's sources are fed by the site
/// scene, so a fed cue that is not in the stream table does not stop the
/// run).
fn play_proximity(
    commands: &mut Commands,
    server: &Res<AssetServer>,
    streams: &Streams,
    cue: &str,
    package: &str,
) -> Option<Entity> {
    let Some(stream) = streams.0.get(&(cue.to_string(), package.to_string())) else {
        warn!(
            "A 套：cue {} @ {} 不在流表里，跳过",
            label(cue),
            label(package)
        );
        return None;
    };
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    let settings = if stream.loops {
        PlaybackSettings::LOOP
            .with_start_position(Duration::from_secs_f64(stream.loop_start))
            .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
    } else {
        PlaybackSettings::ONCE
    };
    Some(
        commands
            .spawn((
                AudioPlayer::new(handle),
                settings.with_volume(Volume::Linear(0.0)),
            ))
            .id(),
    )
}

// ---- talk voice 通道 ---------------------------------------------------------

pub use crate::audio_inputs::VoiceWho;

/// Resolved actor instance, independent of the package-routing referent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceSpeaker {
    Npc(Entity),
    Fixture(Entity),
}

/// A voice line request delivered where the line advances: the dialogue domain
/// spawns it and this domain consumes and despawns it in the same frame (in
/// the in-line command order voice precedes text, so playback and the text
/// line are one beat, not separated by a frame).
#[derive(Component)]
pub struct VoiceLine {
    pub talk_id: i32,
    pub step: usize,
    pub cue: String,
    /// partvoice 的说话者指称（voice_ 族不读它：包随 cue 的 script 段）。
    pub who: Option<VoiceWho>,
    /// Current-cast instance; unresolved identity does not change audio routing.
    pub speaker: Option<VoiceSpeaker>,
}

pub use crate::audio_inputs::VoicePrefetch;

const VOICE_PREFETCH_CACHE_LIMIT: usize = 24;

/// Small bounded strong-handle cache. Keeping the handle alive is what makes an
/// AssetServer request a real preload rather than a dropped speculative load.
#[derive(Resource, Default)]
pub struct VoicePrefetchCache {
    handles: HashMap<String, Handle<AudioSource>>,
    order: VecDeque<String>,
}

impl VoicePrefetchCache {
    fn get(&mut self, path: &str) -> Option<Handle<AudioSource>> {
        let handle = self.handles.get(path)?.clone();
        if let Some(index) = self.order.iter().position(|key| key == path) {
            self.order.remove(index);
        }
        self.order.push_back(path.to_owned());
        Some(handle)
    }

    fn insert(&mut self, path: String, handle: Handle<AudioSource>) {
        if self.handles.contains_key(&path) {
            let _ = self.get(&path);
            return;
        }
        while self.handles.len() >= VOICE_PREFETCH_CACHE_LIMIT {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.handles.remove(&oldest);
        }
        self.order.push_back(path.clone());
        self.handles.insert(path, handle);
    }
}

/// Attached to the actual player, not to a parallel clock or a cue lookup.
/// The consumed request Entity remains a generation-bearing playback token;
/// it need not remain alive after the line request has been consumed.
#[derive(Component)]
pub struct VoicePlaybackIdentity {
    pub request: Entity,
    pub speaker: Option<VoiceSpeaker>,
}

/// Single dialogue-voice bus: a new Voice line stops the old one before lookup.
/// A live line can finish naturally while waiting for a manual click; disposing
/// the completed/skipped talk stops it and cancels deferred line requests.
#[derive(Resource, Default)]
pub struct VoiceChannel {
    sink: Option<Entity>,
    /// 在播 cue（起播时记下，账目行用）。
    cue: Option<String>,
    /// A volume preview on the voice player (`SoundManager.PlayVoice` /
    /// `PlayVoiceFixedVolume` from the option volume page): a cue playback
    /// entity. It is not a dialogue line: no playback identity, no mouth, no
    /// analyzer.
    preview: Option<Entity>,
}

impl VoiceChannel {
    /// Actual player entity, including while its asset is still loading.
    /// Presence alone is neither audible playback nor an analyzer sample.
    pub fn active_sink(&self) -> Option<Entity> {
        self.sink
    }
}

/// The routing decision for one voice line: the voice_ family fixes the
/// package from the cue (existing domain, behaviour unchanged); the partvoice
/// family looks up the routing table by speaker — the original game's chain is
/// "speaker variant -> both loaded packages, sounding only if the cue is in
/// one of them", mirrored here: the package is looked up in the table and the
/// cue in the stream table, with misses and gate refusals named by class.
enum VoiceRoute {
    /// Stream table hit: the cue is in this package, play it. `partvoice`
    /// marks the partvoice log face (the voice_ family's log face is kept
    /// as is; the accounting is not mixed).
    Play { package: String, partvoice: bool },
    /// Stream table miss of the voice_ family: named by corpus class (no
    /// package upstream / not extracted yet).
    TalkMiss { package: String },
    /// partvoice does not play by structure: routing table absent / speaker
    /// reference missing / speaker not in the table / gate refused — the
    /// original game loads no package in any of these cases (the dialogue
    /// continues); each is logged at info with its reason.
    Refuse { reason: String },
    /// The partvoice cue is not in the speaker's package: the original game's
    /// ExistsCueName miss is skipped silently; here a warn names the packages
    /// tried (the class of a missing cue for accounting).
    Miss { who: String, packages: Vec<String> },
}

/// Line -> decision. The voice_ family is decided first (the prefix yields the
/// package name directly); any other cue without the partvoice prefix is also
/// refused by name (a cue outside the vocabulary belongs to no family).
fn route_voice(
    line: &VoiceLine,
    partvoice: Option<&PartVoiceMap>,
    streams: &Streams,
    birthday: bool,
    tutorial: bool,
) -> VoiceRoute {
    if let Some(package) = contextual_voice_package(&line.cue, tutorial, birthday) {
        return if streams.0.contains_key(&(line.cue.clone(), package.clone())) {
            VoiceRoute::Play {
                package,
                partvoice: false,
            }
        } else {
            VoiceRoute::TalkMiss { package }
        };
    }
    if !line.cue.starts_with("partvoice_") {
        return VoiceRoute::Refuse {
            reason: "cue 非 voice_/partvoice_ 族的已知前缀".to_owned(),
        };
    }
    let Some(map) = partvoice else {
        return VoiceRoute::Refuse {
            reason: "partvoice 路由表缺席，fail-closed".to_owned(),
        };
    };
    // Speaker -> candidate package set: the main chain tries both packages
    // (both are loaded and the cue sounds from whichever holds it), the egg
    // chain has one; a gate refusal or an absent entry means no package is
    // loaded, so the line is refused.
    let (who, packages): (String, Vec<String>) = match line.who {
        Some(VoiceWho::Participant(id)) => match map.participants.get(&id) {
            None => {
                return VoiceRoute::Refuse {
                    reason: format!("说话者 {id} 不在 partvoice 路由表"),
                }
            }
            Some(ParticipantRoute::Gated(reason)) => {
                return VoiceRoute::Refuse {
                    reason: format!("speaker {id} refused by the gate ({reason}); no package is loaded"),
                }
            }
            Some(ParticipantRoute::Packages { scenario, mysekai }) => (
                format!("说话者 {id}"),
                vec![mysekai.clone(), scenario.clone()],
            ),
        },
        Some(VoiceWho::Fixture(fixture)) => match map.fixtures.get(&fixture) {
            None => {
                return VoiceRoute::Refuse {
                    reason: format!("家具 {fixture} 不在 partvoice 路由表"),
                }
            }
            Some(package) => (format!("家具 {fixture}"), vec![package.clone()]),
        },
        None => {
            return VoiceRoute::Refuse {
                reason: "步无 who/fixture 指称，说话者变体无从定位".to_owned(),
            }
        }
    };
    for package in &packages {
        if streams.0.contains_key(&(line.cue.clone(), package.clone())) {
            return VoiceRoute::Play {
                package: package.clone(),
                partvoice: true,
            };
        }
    }
    VoiceRoute::Miss { who, packages }
}

/// Request exact future authored voice assets while a conversation is already
/// known. This deliberately performs no playback and never touches VoiceChannel:
/// the source StopVoiceAll/ExistsCueName/PlayVoice ordering remains owned by
/// serve_voice when the actual command reaches the script cursor.
pub fn prefetch_voice(
    server: Res<AssetServer>,
    routing: Option<Res<Routing>>,
    partvoice: Option<Res<PartVoiceMap>>,
    talk: Option<Res<crate::audio_inputs::TalkVoicePrefetch>>,
    birthday: (Option<Res<crate::audio_inputs::BirthdayVoice>>, Option<Res<crate::audio_inputs::TutorialVoice>>),
    mut cache: ResMut<VoicePrefetchCache>,
) {
    let Some(routing) = routing else {
        return;
    };
    let mut requests = talk.map(|talk| talk.requests.clone()).unwrap_or_default();
    requests.truncate(VOICE_PREFETCH_CACHE_LIMIT);

    for request in requests {
        let line = VoiceLine {
            talk_id: 0,
            step: 0,
            cue: request.cue,
            who: request.who,
            speaker: None,
        };
        let VoiceRoute::Play { package, .. } =
            route_voice(&line, partvoice.as_deref(), &routing.streams, birthday.0.is_some(), birthday.1.is_some())
        else {
            continue;
        };
        let Some(stream) = routing.streams.0.get(&(line.cue.clone(), package)) else {
            continue;
        };
        let path = format!("moly://{}", stream.ogg);
        if cache.get(&path).is_some() {
            continue;
        }
        let handle = server.load::<AudioSource>(AssetPath::from(path.clone()));
        debug!("[voice-prefetch] requested {}", label(&line.cue));
        cache.insert(path, handle);
    }
}

/// Update (inside the dialogue chain, after a step): consume line requests —
/// stop the old sound, check the stream table, play or skip by name. A missing
/// cue is skipped and the dialogue continues (ExistsCueName has no else and
/// logs nothing); this crate does the same and logs every skip: a cue with no
/// package upstream is named through the corpus ledger, and any other cue
/// missing from the table is named as a not-yet-extracted consumption surface,
/// without silence and without panicking.
pub fn serve_voice(
    mut commands: Commands,
    time: Res<Time>,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    corpus: Option<Res<VoiceCorpus>>,
    routing: Option<Res<Routing>>,
    partvoice: Option<Res<PartVoiceMap>>,
    birthday: (Option<Res<crate::audio_inputs::BirthdayVoice>>, Option<Res<crate::audio_inputs::TutorialVoice>>),
    mut prefetch: ResMut<VoicePrefetchCache>,
    mut channel: ResMut<VoiceChannel>,
    lines: Query<(Entity, &VoiceLine)>,
    sinks: Query<&AudioSink>,
    players: Query<&AudioPlayer<MeteredVoiceSource>>,
    sources: Query<&VoiceSource>,
    metered_sources: Res<Assets<MeteredVoiceSource>>,
) {
    // ONCE is not automatically despawned. Reap completion, removed players and
    // terminal asset failures, while retaining real in-flight asset loads.
    if let Some(entity) = channel.sink {
        if crate::voice_pcm::finished_or_failed(
            entity,
            &server,
            &sources,
            &players,
            &metered_sources,
            &sinks,
        ) {
            if let Ok(mut entity_commands) = commands.get_entity(entity) {
                entity_commands.despawn();
            }
            channel.sink = None;
            channel.cue = None;
        }
    }
    let Some(routing) = routing else {
        return; // 流表未就绪：行请求留池等（对话能走到 voice 步时表早已就绪）
    };
    for (entity, line) in &lines {
        // Monophonic: stop the old sound before the table check (the order
        // is StopVoiceAll before the existence test).
        if let Some(old) = channel.sink.take() {
            if let Ok(mut entity_commands) = commands.get_entity(old) {
                entity_commands.despawn();
            }
        }
        match route_voice(line, partvoice.as_deref(), &routing.streams, birthday.0.is_some(), birthday.1.is_some()) {
            VoiceRoute::Play { package, partvoice } => {
                let Some(stream) = routing.streams.0.get(&(line.cue.clone(), package.clone()))
                else {
                    // 裁决刚判过在表：同帧不可能被撤（流表是解析后只读资源）。
                    unreachable!("route_voice 判定在表后流表被改写")
                };
                // talk voice 的流全部非循环（整轨一次性）；音量 = voice
                // 播放器 × cue 的类别与音量命令（PlayVoice 基量 1.0）。
                let cue_volume = track_volume(
                    routing.cue_gain(&line.cue, &package),
                    Player::Voice,
                    SLOT_VOX_SCENARIO,
                    0,
                );
                let volume = cue_volume.linear(&bus) * gate.factor();
                let path = format!("moly://{}", stream.ogg);
                let cached = prefetch.get(&path);
                let was_prefetched = cached.is_some();
                let handle = match cached {
                    Some(handle) => handle,
                    None => {
                        let handle = server.load::<AudioSource>(AssetPath::from(path.clone()));
                        prefetch.insert(path, handle.clone());
                        handle
                    }
                };
                let raw_ready_at_command =
                    matches!(server.get_load_state(handle.id()), Some(LoadState::Loaded));
                channel.sink = Some(
                    commands
                        .spawn((
                            VoiceSource::new(
                                handle,
                                time.elapsed_secs_f64(),
                                was_prefetched,
                                raw_ready_at_command,
                            ),
                            PlaybackSettings::ONCE.with_volume(Volume::Linear(volume)),
                            BusVolume::Cue(cue_volume),
                            VoicePlaybackIdentity {
                                request: entity,
                                speaker: line.speaker,
                            },
                        ))
                        .id(),
                );
                channel.cue = Some(line.cue.clone());
                if partvoice {
                    info!(
                        "partvoice 起播：段 {} 步 {} cue {} @ {}（时长 {:.2}s，整轨一次性，音量 {:.2} = 1.0 × 面板）",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                        stream.duration,
                        volume,
                    );
                } else {
                    info!(
                        "talk voice 起播：段 {} 步 {} cue {} @ {}（时长 {:.2}s，整轨一次性，音量 {:.2} = 1.0 × 面板）",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                        stream.duration,
                        volume,
                    );
                }
            }
            VoiceRoute::TalkMiss { package } => {
                let named = corpus
                    .as_deref()
                    .is_some_and(|corpus| corpus.uncovered.iter().any(|cue| cue == &line.cue));
                if named {
                    warn!(
                        "talk voice missing cue: talk {} step {} cue {} — no package upstream (named in the corpus ledger; skipped, the dialogue continues)",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                    );
                } else {
                    warn!(
                        "talk voice 缺 cue：段 {} 步 {} cue {} @ {} 不在流表（fixture-talks 语料面未提取），对话照走",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                    );
                }
            }
            VoiceRoute::Refuse { reason } => {
                info!(
                    "partvoice 跳过：段 {} 步 {} cue {}——{}，对话照走",
                    line.talk_id,
                    line.step,
                    label(&line.cue),
                    reason,
                );
            }
            VoiceRoute::Miss { who, packages } => {
                let tried: Vec<String> = packages.iter().map(|p| label(p)).collect();
                warn!(
                    "partvoice missing cue: talk {} step {} cue {} is not in the package of {} [{}] (ExistsCueName miss, skipped); the dialogue continues",
                    line.talk_id,
                    line.step,
                    label(&line.cue),
                    who,
                    tried.join(", "),
                );
            }
        }
        commands.entity(entity).despawn();
    }
    if channel.sink.is_none() {
        channel.cue = None; // 停旧声后没接上新声：账目不留尾巴
    }
}

// ---- 一次性 SE 通道（事件族） -------------------------------------------------

/// The field scene installs these three substitutions in SoundManager.
/// Unlisted cues retain their identity, including subwindow open/close cues.
fn themed_se_cue(cue: &str) -> &str {
    match cue {
        "SE_DECIDE1" => "se_mysekai_ui_decision",
        "SE_CANCEL" => "se_mysekai_ui_cancel",
        "SE_REWARD_DIALOG_OPEN" => "se_get_blueprint",
        other => other,
    }
}

/// The common MySekai SE package (the package key form in loop.json), from
/// the constant `MYSEKAI_COMMON_SE_NAME = "mysekai/sound/se/se_mysekai"`: all
/// one-shot event SE cues live in the shared package `se_mysekai` (Set A sound
/// sources use the same one: a source cue fed by the site scene carries no
/// package name, and this package is filled in when the component is
/// attached).
pub const SE_PACKAGE: &str = "mysekai__sound__se__se_mysekai";

/// The family a one-shot request names (scene action -> Ingame, interface
/// action -> Ui). The source's `PlaySE` takes no category: a cue's categories
/// are the cue sheet's, and the SE channel uses them when the export carries
/// them. The class is only the named stand-in for a cue whose facts are not
/// exported (then its category is this family's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeClass {
    Ingame,
    Ui,
}

impl SeClass {
    /// Field slot of the family's category.
    fn slot(self) -> usize {
        match self {
            Self::Ingame => SLOT_SE_INGAME,
            Self::Ui => SLOT_SE_UI,
        }
    }

    /// SE player times the family's category (no cue facts).
    fn volume(self, bus: &VolumeBus) -> f32 {
        bus.player.se * bus.slot_volume(self.slot())
    }
}

/// 一次一次性 SE 请求（事件侧入队的三元组）：cue + 音量类 + 日志标签。
#[derive(Debug, Clone)]
pub struct SeRequest {
    /// A generation-safe playback scope; None is an ordinary world/UI sound.
    pub owner: Option<Entity>,
    /// A plain cue, or an AnimationEvent's bundle path whose leaf is the cue.
    /// A supplied bundle path selects its own bank, never a same-name fallback.
    pub cue: String,
    pub class: SeClass,
    /// 事件来源标签：事件→cue→起播/拒发行的可推导链靠它拼接。
    pub source: &'static str,
}

/// 一次性 SE 请求队列。写者（harvest `on_damage`、talk 步进/窗体、
/// `fixture_edit` 动作）先入队，`advance_se` 后排空——链序在 schedule
/// 里显式约束。The second list holds source buttons'
/// `CustomSelectableDefine.PlaySE` calls; they resolve against the cue bank
/// when the queue drains.
#[derive(Resource, Default)]
pub struct SeRequests(
    pub Vec<SeRequest>,
    pub Vec<ButtonSe>,
    pub Vec<PreviewSound>,
);

/// One volume preview call of the option volume page.
#[derive(Debug, Clone)]
pub enum PreviewSound {
    /// `PlayVoice(cue, 1)`: the voice player at its player volume times 1.
    Voice(String),
    /// `SamplePlaySE(cue, volume)`: the SE player's sample player (index 5)
    /// at `volume`, without a player volume.
    SampleSe(String, f32),
    /// `PlayVoiceFixedVolume(cue, volume)`: the voice player at `volume`,
    /// without its player volume.
    VoiceFixed(String, f32),
}

/// One `CustomSelectableDefine.PlaySE(se, otherSeName)` call.
#[derive(Debug, Clone)]
pub struct ButtonSe {
    pub se: moly_law::ui::custom_button::SeType,
    pub other_se_name: String,
}

impl SeRequests {
    /// Queues an in-game one-shot `cue` with no owner; `source` names the
    /// writer in logs.
    pub fn ingame(&mut self, cue: &str, source: &'static str) {
        self.0.push(SeRequest {
            owner: None,
            cue: cue.to_owned(),
            class: SeClass::Ingame,
            source,
        });
    }

    /// Queues a `PlaySE(se, otherSeName)` call.
    pub fn button(
        &mut self,
        se: moly_law::ui::custom_button::SeType,
        other_se_name: String,
    ) {
        self.1.push(ButtonSe { se, other_se_name });
    }

    /// Queues a volume preview call.
    pub fn preview(&mut self, sound: PreviewSound) {
        self.2.push(sound);
    }
}

/// The bank a plain cue name plays from: the shared MySekai SE bank, then
/// the built-in and downloaded common menu banks.
fn plain_se_package(streams: &Streams, cue: &str) -> Option<&'static str> {
    [SE_PACKAGE, "MenuCommon_Built_in", "MenuCommon"]
        .into_iter()
        .find(|package| {
            streams
                .0
                .contains_key(&(cue.to_owned(), (*package).to_owned()))
        })
}

/// The stream a plain cue name plays from (see [`plain_se_package`]).
fn plain_se_stream<'a>(streams: &'a Streams, cue: &str) -> Option<&'a StreamRow> {
    plain_se_package(streams, cue)
        .and_then(|package| streams.0.get(&(cue.to_owned(), package.to_owned())))
}

/// Ordering anchor for draining one-shot SE: every writer system orders
/// itself `.before(Drain)` in its own plugin, which declares that it runs before
/// the reader (a writer's signature need not leave its module — some writers take
/// private types in their parameters, so ordering goes through the set, not
/// the system name).
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum SeDrainSet {
    Drain,
}

/// Live one-shot players, playback accounting and once-per-cue missing warnings.
#[derive(Resource, Default)]
pub struct SeChannel {
    live: Vec<Entity>,
    requests: usize,
    played: usize,
    warned_missing: HashSet<String>,
}

/// Drain accepted requests through the same one-shot player on both targets.
/// Every request starts its cue as the middleware does (plain, or a sequence
/// of type 0-4: see `cue`); a cue this port cannot play, or one not
/// decoded, is reported once and nothing else is played in its place. The
/// volume of each track is the SE player times the cue's categories and
/// volume commands.
pub fn advance_se(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time<Real>>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    routing: Option<Res<Routing>>,
    mut queue: ResMut<SeRequests>,
    mut channel: ResMut<SeChannel>,
    mut work: ResMut<SequenceWorkAreas>,
    mut rngs: ResMut<CueRngs>,
    playbacks: Query<(), With<CuePlayback>>,
    owners: Query<()>,
    mut voice: ResMut<VoiceChannel>,
) {
    // A playback despawns itself when it has played out.
    channel.live.retain(|entity| playbacks.contains(*entity));
    if voice
        .preview
        .is_some_and(|entity| !playbacks.contains(entity))
    {
        voice.preview = None;
    }
    let Some(routing) = routing else {
        return; // routing not ready: requests stay queued (drained once ready; the window still dedups)
    };
    // PlaySE: a table cue plays; Other plays its serialized name only when
    // that cue exists in a loaded bank (else the source logs an error and
    // nothing plays); None plays nothing.
    let buttons = std::mem::take(&mut queue.1);
    for button in buttons {
        let exists = |cue: &str| plain_se_stream(&routing.streams, cue).is_some();
        match moly_law::ui::custom_button::play_se(button.se, &button.other_se_name, exists) {
            Some(cue) => queue.0.push(SeRequest {
                owner: None,
                cue,
                class: SeClass::Ui,
                source: "source-button",
            }),
            None if button.se == moly_law::ui::custom_button::SeType::Other => error!(
                "SE: button cue {:?} is not in a loaded bank; nothing plays",
                button.other_se_name
            ),
            None => {}
        }
    }
    // The previews resolve like plain cue names and play as cues. The voice
    // ones replace the preview on the voice player (StopVoiceAll ran at the
    // call); the fixed ones play at the volume they were called with instead
    // of the player's (the cue's categories and volume commands still apply).
    for preview in std::mem::take(&mut queue.2) {
        let (cue, fixed, on_voice) = match &preview {
            PreviewSound::Voice(cue) => (cue.as_str(), None, true),
            PreviewSound::SampleSe(cue, volume) => (cue.as_str(), Some(*volume), false),
            PreviewSound::VoiceFixed(cue, volume) => (cue.as_str(), Some(*volume), true),
        };
        let plan = plain_se_package(&routing.streams, cue)
            .ok_or_else(|| "not in a loaded bank".to_string())
            .and_then(|package| routing.cue_plan(cue, package).map(|plan| (package, plan)));
        let (package, plan) = match plan {
            Ok(found) => found,
            Err(reason) => {
                if channel.warned_missing.insert(cue.to_owned()) {
                    warn!("SE: preview cue {}: {reason}; nothing plays", label(cue));
                }
                continue;
            }
        };
        let (player, slot) = match &preview {
            PreviewSound::Voice(_) => (Player::Voice, SLOT_VOX_SCENARIO),
            PreviewSound::VoiceFixed(..) => (Player::Unscaled, SLOT_VOX_SCENARIO),
            PreviewSound::SampleSe(..) => (Player::Unscaled, SLOT_SE_INGAME),
        };
        let playback = cue::start_cue(
            &server,
            &mut work,
            &mut rngs,
            time.elapsed_secs_f64(),
            cue,
            package,
            &plan,
            player,
            slot,
            RngSlot::OneShot,
            "option preview",
        )
        .scaled(fixed.unwrap_or(1.0));
        let entity = cue::spawn_playback(
            &mut commands,
            playback,
            &bus,
            &gate,
            time.elapsed_secs_f64(),
        );
        if on_voice {
            if let Some(old) = voice.preview.replace(entity) {
                if let Ok(mut entity_commands) = commands.get_entity(old) {
                    entity_commands.despawn();
                }
            }
        } else {
            channel.live.push(entity);
        }
        channel.played += 1;
        info!(
            "SE preview: {} ({preview:?}, player {player:?})",
            label(cue)
        );
    }
    for request in queue.0.drain(..) {
        if request
            .owner
            .is_some_and(|owner| owners.get(owner).is_err())
        {
            continue;
        }
        channel.requests += 1;
        let (cue, source_package) = match request.cue.rsplit_once('/') {
            Some((_, cue)) => (cue, Some(request.cue.replace('/', "__"))),
            None => (themed_se_cue(&request.cue), None),
        };
        let package = match source_package {
            Some(package) => Some(package),
            None => plain_se_package(&routing.streams, cue).map(str::to_owned),
        };
        let Some(package) = package.filter(|package| {
            routing
                .streams
                .0
                .contains_key(&(cue.to_owned(), package.clone()))
        }) else {
            if channel.warned_missing.insert(request.cue.to_string()) {
                warn!(
                    "SE skipped: cue {} is not in this snapshot's stream table (fail-closed skip; unless the package does not contain this cue either, this is an extraction/stream-table gap; warned once per cue)",
                    label(&request.cue)
                );
            }
            continue;
        };
        let plan = match routing.cue_plan(cue, &package) {
            Ok(plan) => plan,
            Err(reason) => {
                if channel.warned_missing.insert(request.cue.to_string()) {
                    error!(
                        "SE refused: cue {} @ {}: {reason} (one of its waveforms is not used as a substitute; reported once per cue)",
                        label(cue),
                        label(&package)
                    );
                }
                continue;
            }
        };
        let facts = plan.gain.is_some();
        let playback = cue::start_cue(
            &server,
            &mut work,
            &mut rngs,
            time.elapsed_secs_f64(),
            cue,
            &package,
            &plan,
            Player::Se,
            request.class.slot(),
            RngSlot::OneShot,
            "one-shot SE",
        );
        let entity = cue::spawn_playback(
            &mut commands,
            playback,
            &bus,
            &gate,
            time.elapsed_secs_f64(),
        );
        if let Some(owner) = request.owner {
            commands.entity(entity).insert(ScopedSe(owner));
        }
        channel.live.push(entity);
        channel.played += 1;
        debug!(
            "SE started: {} ({} · {} {} tracks · {} · flat 2D)",
            label(cue),
            request.source,
            plan.kind.label(),
            plan.voices.len(),
            if facts {
                "volume = SE player × cue categories × volume commands"
            } else {
                "no cue categories or volume commands in the asset data: the request class applies"
            },
        );
    }
}

#[derive(Component)]
pub struct ScopedSe(pub Entity);

/// Stop only the departing controller's queued and already-playing effects.
/// Other fixtures, UI clicks, BGM and ambient sound are intentionally untouched.
pub fn dispose_scoped_se(world: &mut World, owner: Entity) {
    if let Some(mut queue) = world.get_resource_mut::<SeRequests>() {
        queue.0.retain(|request| request.owner != Some(owner));
    }
    let entities: Vec<_> = world
        .query::<(Entity, &ScopedSe)>()
        .iter(world)
        .filter_map(|(entity, scope)| (scope.0 == owner).then_some(entity))
        .collect();
    for entity in &entities {
        if let Some(sink) = world.get::<AudioSink>(*entity) {
            sink.stop();
        }
        // A playback's sounding tracks are its children: despawn takes them.
        if let Ok(entity) = world.get_entity_mut(*entity) {
            entity.despawn();
        }
    }
    if let Some(mut channel) = world.get_resource_mut::<SeChannel>() {
        channel.live.retain(|entity| !entities.contains(entity));
    }
}

#[cfg(test)]
mod scoped_se_tests {
    use super::*;
    #[test]
    fn cancellation_retires_only_the_departing_owner() {
        let mut world = World::new();
        world.init_resource::<SeRequests>();
        world.init_resource::<SeChannel>();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let old = world.spawn(ScopedSe(first)).id();
        let newer = world.spawn(ScopedSe(second)).id();
        world.resource_mut::<SeChannel>().live = vec![old, newer];
        for owner in [Some(first), Some(second), None] {
            world.resource_mut::<SeRequests>().0.push(SeRequest {
                owner,
                cue: "test".into(),
                class: SeClass::Ingame,
                source: "test",
            });
        }
        dispose_scoped_se(&mut world, first);
        assert!(world.get_entity(old).is_err());
        assert!(world.get_entity(newer).is_ok());
        assert_eq!(world.resource::<SeRequests>().0.len(), 2);
        assert_eq!(world.resource::<SeChannel>().live, vec![newer]);
    }
}

/// AudioSink absence alone means neither "still loading" nor "finished".
/// Bevy only creates a sink once its AudioSource is present; a failed load
/// otherwise leaves an ONCE player and its channel bookkeeping alive forever.
fn one_shot_finished_or_failed(
    entity: Entity,
    server: &AssetServer,
    players: &Query<&AudioPlayer<AudioSource>>,
    sinks: &Query<&AudioSink>,
) -> bool {
    let Ok(player) = players.get(entity) else {
        return true;
    };
    if let Ok(sink) = sinks.get(entity) {
        return sink.empty();
    }
    if let Some(LoadState::Failed(error)) = server.get_load_state(player.0.id()) {
        warn!(
            "audio one-shot {entity:?} failed loading {:?}: {error}; releasing player",
            player.0.path()
        );
        return true;
    }
    // Loaded-without-sink can also mean unavailable output or a pending Bevy
    // update; do not turn a wall-clock timeout into a playback-end event.
    false
}

// ---- periodic accounting ----------------------------------------------------

/// Update (every 2s): one line per channel — phenomenon -> track (BGM),
/// phenomenon -> cue -> volume (ambience, flat), source count -> current
/// channel and volume (Set A), request/start counts (one-shot SE).
pub fn report(
    phenomenon: Res<CurrentPhenomenon>,
    site: Option<Res<SiteActive>>,
    routing: Option<Res<Routing>>,
    bgm: Res<BgmChannel>,
    ambient: Res<AmbientChannel>,
    proximity: Res<ProximityState>,
    sources: Query<(), With<SoundObject>>,
    voice: Res<VoiceChannel>,
    se: Res<SeChannel>,
    sinks: Query<&AudioSink>,
    children: Query<&Children>,
) {
    if routing.is_none() {
        return; // routing not ready: the parse's ready line is the first accounting line
    }
    let site_id = site.map_or_else(|| "未定".to_string(), |site| site.site_id.to_string());
    match &bgm.voice {
        Some(voice) => {
            let segment = if voice.loop_sink.is_some() && voice.handoff_done {
                "loop"
            } else if voice.loop_sink.is_some() {
                "intro"
            } else {
                "once"
            };
            info!(
                "BGM 账目：站点 {site_id} · 档 {} · 曲 {} · 段 {} · 音量 {:.2}",
                label(&phenomenon.0),
                label(&voice.cue),
                segment,
                voice.applied_volume
            );
        }
        None => info!(
            "BGM 账目：站点 {site_id} · 档 {} · 无曲",
            label(&phenomenon.0)
        ),
    }
    let ambient_volume = ambient
        .playback
        .and_then(|entity| children.get(entity).ok())
        .and_then(|tracks| tracks.iter().find_map(|track| sinks.get(track).ok()))
        .map(|sink| sink.volume().to_linear())
        .unwrap_or(0.0);
    match &ambient.playing {
        Some(cue) => info!(
            "环境音账目：站点 {site_id} · cue {} · 音量 {:.2}（平铺）",
            label(cue),
            ambient_volume
        ),
        None => info!("环境音账目：站点 {site_id} · 无"),
    }
    info!(
        "A 套账目：声源 {} · 当前 {} · 音量 {:.3}",
        sources.iter().count(),
        if proximity.current.is_empty() {
            "无".to_string()
        } else {
            label(&proximity.current)
        },
        proximity.current_volume
    );
    let voice_volume = voice
        .sink
        .and_then(|entity| sinks.get(entity).ok())
        .map(|sink| sink.volume().to_linear())
        .unwrap_or(0.0);
    match (&voice.cue, voice.sink) {
        (Some(cue), Some(entity)) if sinks.get(entity).map(|s| !s.empty()).unwrap_or(false) => {
            info!(
                "talk voice 账目：在播 {} · 音量 {:.2}",
                label(cue),
                voice_volume
            );
        }
        _ => info!("talk voice 账目：无在播"),
    }
    info!(
        "SE 账目：请求 {} · 起播 {} · 在响 {}",
        se.requests,
        se.played,
        se.live.len()
    );
}

/// Audio domain resource setup. The current-phenomenon resource gets a
/// fallback here (after both sides attach the weather plugin this fallback
/// stays: `init_resource` is idempotent, whichever runs first creates it, and
/// the value is the same default); the channel state is placed here as
/// well.
pub fn install(app: &mut App) {
    app.init_resource::<VolumeBus>()
        .init_resource::<AudioGate>()
        .init_resource::<LocalVolumeSettings>()
        .init_resource::<CurrentPhenomenon>()
        .init_resource::<crate::audio_inputs::ClientMusicPlaySettings>()
        .init_resource::<BgmChannel>()
        .init_resource::<record::RecordLibrary>()
        .init_resource::<RecordChoice>()
        .init_resource::<cutscene::CutSceneAudio>()
        .init_resource::<MysekaiBgmFade>()
        .init_resource::<AmbientChannel>()
        .init_resource::<SequenceWorkAreas>()
        .init_resource::<CueRngs>()
        .init_resource::<ProximityState>()
        .init_resource::<VoicePrefetchCache>()
        .init_resource::<crate::audio_inputs::TalkVoicePrefetch>()
        .init_resource::<VoiceChannel>()
        .init_resource::<SeRequests>()
        .init_resource::<SeChannel>();
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Update, record::autoplay_record_choice.before(advance_bgm));
}

#[cfg(test)]
mod stream_manifest_tests {
    use super::*;
    use serde_json::json;

    fn manifest(rows: serde_json::Value) -> serde_json::Value {
        json!({"packages":[{"package":"source-bank","streams":rows}]})
    }
    fn valid() -> serde_json::Value {
        json!({"cue":"available","subsong":1,"loop":false,
            "ogg":"audio/source.ogg","durationSeconds":1.25})
    }
    #[test]
    fn source_diagnostic_does_not_poison_unrelated_audio_or_fabricate_a_stream() {
        let streams = parse_streams(
            &manifest(json!([valid(), {
                "cue":"missing","error":"no waveform in this archive carries this cue name"
            }])),
            "phenomena/",
        );
        assert_eq!(streams.0.len(), 1);
        assert!(streams
            .0
            .get(&("missing".into(), "source-bank".into()))
            .is_none());
        assert_eq!(
            streams.0[&("available".into(), "source-bank".into())].ogg,
            "phenomena/audio/source.ogg"
        );
    }
    #[test]
    #[should_panic(expected = "loop")]
    fn malformed_playable_row_still_fails_closed() {
        let mut row = valid();
        row["loop"] = json!(null);
        parse_streams(&manifest(json!([row])), "");
    }
    #[test]
    #[should_panic(expected = "partial playable")]
    fn diagnostic_cannot_hide_corrupt_playable_data() {
        let mut row = valid();
        row["error"] = json!("failed");
        parse_streams(&manifest(json!([row])), "");
    }
    #[test]
    #[should_panic(expected = "nonempty string")]
    fn malformed_diagnostic_is_not_silently_ignored() {
        parse_streams(&manifest(json!([{"cue":"bad","error":false}])), "");
    }
}

#[cfg(test)]
mod sequence_export_tests {
    use super::*;
    use serde_json::json;

    fn entry(package: &str, export: serde_json::Value) -> serde_json::Value {
        json!({
            "package": package,
            "archive": format!("{package}.acb"),
            "streams": [{"cue": "c", "subsong": 1, "loop": false,
                "ogg": format!("audio/{package}.ogg"), "durationSeconds": 1.0}],
            "sequenceExport": export,
            "cueCommands": {"c": {"commands": [{"code": 87, "bytes": "0032"}]}},
            "sequences": {}
        })
    }

    fn routing(loops: &serde_json::Value) -> Routing {
        Routing {
            site_bgms: HashMap::new(),
            site_sound_fallbacks: HashMap::new(),
            sites_with_sounds: HashSet::new(),
            phenomena: HashMap::new(),
            streams: parse_streams(loops, ""),
            sequences: parse_sequences(loops, ""),
        }
    }

    #[test]
    fn a_package_export_the_reader_does_not_take_is_refused_by_name_and_the_rest_is_read() {
        let mut no_commands = entry("no-commands", json!({"version": 2, "acbVersion": 1}));
        no_commands.as_object_mut().unwrap().remove("cueCommands");
        let loops = json!({"packages": [
            entry("good", json!({"version": 2, "acbVersion": 0x0130_0000})),
            no_commands,
            entry("wide", json!({"version": 2, "acbVersion": 4_294_967_296_i64})),
            entry("future", json!({"version": 3})),
            entry("unversioned", json!({})),
        ]});
        let routing = routing(&loops);
        let refused = &routing.sequences.refused;
        let mut names: Vec<&str> = refused.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["future", "no-commands", "unversioned", "wide"]);
        assert!(refused["no-commands"].contains("no cueCommands table"));
        assert!(refused["wide"].contains("acbVersion 4294967296 is out of range"));
        assert!(refused["future"].contains("export version 3"));
        assert!(refused["unversioned"].contains("sequenceExport.version is not an integer"));
        for package in names {
            let error = routing.cue_plan("c", package).unwrap_err();
            assert!(error.contains("sequence export of the package was refused"), "{error}");
            assert!(routing.cue_gain("c", package).is_none());
        }
        let plan = routing.cue_plan("c", "good").expect("the readable package plays");
        assert_eq!(plan.voices.len(), 1);
        assert_eq!(routing.cue_gain("c", "good").unwrap().sequence, 0.5);
    }

    #[test]
    fn a_malformed_block_or_command_list_refuses_only_its_package() {
        let mut wide_type = entry("wide-type", json!({"version": 1}));
        wide_type["sequences"] = json!({"c": {"type": 300, "sequenceIndex": 0, "tracks": []}});
        let mut odd_bytes = entry("odd-bytes", json!({"version": 2, "acbVersion": 1}));
        odd_bytes["cueCommands"]["c"]["commands"][0]["bytes"] = json!("003");
        let mut short_volume = entry("short-volume", json!({"version": 2, "acbVersion": 1}));
        short_volume["cueCommands"]["c"]["commands"][0]["bytes"] = json!("32");
        let loops = json!({"packages": [
            wide_type,
            odd_bytes,
            short_volume,
            entry("good", json!({"version": 1})),
        ]});
        let routing = routing(&loops);
        let refused = &routing.sequences.refused;
        assert_eq!(refused.len(), 3);
        assert!(refused["wide-type"].contains("type 300 is out of range"));
        assert!(refused["odd-bytes"].contains("not whole bytes"));
        assert!(refused["short-volume"].contains("volume command 87 is 1 bytes long"));
        assert!(routing.cue_plan("c", "good").is_ok());
    }

    #[test]
    fn a_cue_command_row_keeps_precedence_over_its_block() {
        let mut both = entry("both", json!({"version": 2, "acbVersion": 1}));
        both["sequences"] = json!({"c": {
            "type": 0, "sequenceIndex": 0, "playbackRatio": 100,
            "actionTracks": {"count": 0}, "instrumentPluginTracks": {"count": 0},
            "commands": [{"code": 87, "bytes": "0019"}],
            "tracks": [{"trackIndex": 0, "startDelayMicroseconds": 0,
                "eventCodesNotDecoded": [],
                "noteOns": [{"kind": 2, "synth": {"references": [
                    {"kind": 1, "waveform": {"subsong": 1}}]}}]}]
        }});
        let routing = routing(&json!({"packages": [both]}));
        assert!(routing.sequences.refused.is_empty());
        assert_eq!(routing.cue_gain("c", "both").unwrap().sequence, 0.5);
        assert!(routing.cue_plan("c", "both").is_ok());
    }
}
