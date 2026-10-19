//! Cue playback as the middleware runs it: one start of a cue plays the
//! tracks its sequence type picks ([`super::cue_law`]); each picked track
//! waits for its own start delay, then plays its waveform to the end; the
//! round ends when every picked track has ended, and a sequence with the
//! repeat command starts its next round in the same update. A plain cue is
//! the one-track case. Every playback is an entity with [`CuePlayback`]; its
//! sounding tracks are child entities, so despawning the playback stops them
//! all. A finished playback despawns itself.
//!
//! The volume of a sounding track is the product the middleware forms (see
//! [`CueVolume`]): the player's volume (the user's system volume of the BGM,
//! SE or voice player), the volume of every category the cue belongs to, the
//! cue's own volume commands and the track's and synth's volume commands.

use super::cue_law;
use super::*;
use crate::audio_sequence::{SequenceRng, ShuffleWork};
use bevy::prelude::*;

/// One of the three players the user's system volume addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Player {
    Bgm,
    Se,
    Voice,
    /// A call that passes its own volume instead of the player's (the option
    /// page's fixed-volume previews); see [`CuePlayback::scaled`].
    Unscaled,
}

/// The categories a sounding track is attenuated by. `Cue` holds category
/// indices of the sound-configuration table as the cue sheet names them;
/// `Class` is the named stand-in for a cue whose facts were not exported (the
/// consumer's family, see [`CueGain`]).
#[derive(Clone, Debug, PartialEq)]
pub enum CategoryRefs {
    Cue(Vec<u32>),
    Class(usize),
}

/// The volume of one sounding track: `player × Π category × gain`.
#[derive(Clone, Debug, PartialEq)]
pub struct CueVolume {
    pub player: Player,
    pub categories: CategoryRefs,
    /// Cue, track and synth volume commands multiplied (1.0 without facts).
    pub gain: f32,
}

impl CueVolume {
    pub fn linear(&self, bus: &VolumeBus) -> f32 {
        self.linear_at(bus, bus.player_volume(self.player))
    }

    /// The same product with the player's volume given (the BGM manager's
    /// fade writes the BGM player's volume).
    pub fn linear_at(&self, bus: &VolumeBus, player_volume: f32) -> f32 {
        let category: f32 = match &self.categories {
            CategoryRefs::Cue(indices) => {
                indices.iter().map(|index| bus.acf_volume(*index)).product()
            }
            CategoryRefs::Class(slot) => bus.slot_volume(*slot),
        };
        player_volume * category * self.gain
    }
}

/// Gain facts of one cue: the categories its sequence command list names and
/// the volume commands (value / 100 each, multiplied) of the sequence and of
/// every track with its synth, in authored track order.
#[derive(Clone, Debug, PartialEq)]
pub struct CueGain {
    pub categories: Vec<u32>,
    pub sequence: f32,
    pub tracks: Vec<f32>,
    /// A volume command with a random spread was found (base value used).
    pub random_spread: bool,
}

impl CueGain {
    /// Volume of one sounding track of this cue.
    pub fn volume(&self, player: Player, slot: usize) -> CueVolume {
        CueVolume {
            player,
            categories: CategoryRefs::Cue(self.categories.clone()),
            gain: self.sequence * self.tracks.get(slot).copied().unwrap_or(1.0),
        }
    }
}

/// Volume of a track: from the cue's facts, or the consumer's class when the
/// export carries none.
pub fn track_volume(
    gain: Option<&CueGain>,
    player: Player,
    fallback_slot: usize,
    slot: usize,
) -> CueVolume {
    match gain {
        Some(gain) => gain.volume(player, slot),
        None => CueVolume {
            player,
            categories: CategoryRefs::Class(fallback_slot),
            gain: 1.0,
        },
    }
}

/// The generators of the players that start cues; each is seeded on first
/// use from the clock (the middleware seeds a player's generator when the
/// player is created). The ambient channel, one-shot SE and timeline SE are
/// separate players here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RngSlot {
    Ambient,
    OneShot,
    /// Timeline SE clips (see `start_timeline_cue`).
    #[allow(dead_code)]
    Timeline,
}

#[derive(Resource, Default)]
pub struct CueRngs(HashMap<RngSlot, SequenceRng>);

impl CueRngs {
    fn get(&mut self, slot: RngSlot) -> &mut SequenceRng {
        self.0
            .entry(slot)
            .or_insert_with(|| seeded_sequence_rng(slot))
    }
}

/// One track of the current round.
enum RoundVoice {
    Waiting { slot: usize, due: f64 },
    Sounding { entity: Entity },
    Done,
}

/// A cue that is playing.
#[derive(Component)]
pub struct CuePlayback {
    pub cue: String,
    package: String,
    plan: CuePlan,
    /// Every track's waveform, requested at the start and held (a dropped
    /// handle cancels its load).
    handles: Vec<Handle<AudioSource>>,
    player: Player,
    fallback_slot: usize,
    rng: RngSlot,
    round: u64,
    voices: Vec<RoundVoice>,
    channel: &'static str,
    /// The call's own volume factor (1.0 unless the call passes one).
    scale: f32,
}

impl CuePlayback {
    /// The call's own volume: multiplies every track's volume.
    pub fn scaled(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

/// Start a cue: request its waveforms and run the first start of its
/// sequence; spawn the result with [`spawn_playback`]. The first tracks sound
/// from the next update of [`advance_cue_playbacks`] on (a track with no
/// delay starts there).
#[allow(clippy::too_many_arguments)]
pub fn start_cue(
    server: &AssetServer,
    work: &mut SequenceWorkAreas,
    rngs: &mut CueRngs,
    now: f64,
    cue: &str,
    package: &str,
    plan: &CuePlan,
    player: Player,
    fallback_slot: usize,
    rng: RngSlot,
    channel: &'static str,
) -> CuePlayback {
    let handles = plan
        .voices
        .iter()
        .map(|voice| {
            server.load::<AudioSource>(AssetPath::from(format!("moly://{}", voice.stream.ogg)))
        })
        .collect();
    start_cue_with_handles(
        work,
        rngs,
        now,
        cue,
        package,
        plan,
        handles,
        player,
        fallback_slot,
        rng,
        channel,
    )
}

/// One sounding track as spawned: player, settings, volume, parent.
pub type TrackBundle = (
    AudioPlayer<AudioSource>,
    PlaybackSettings,
    BusVolume,
    ChildOf,
);

/// Spawn a started cue; the entity is the playback's handle (despawn stops
/// it). Tracks whose delay has already passed start at once, in the update of
/// the call (the middleware starts a cue's undelayed tracks in the call).
pub fn spawn_playback(
    commands: &mut Commands,
    mut playback: CuePlayback,
    bus: &VolumeBus,
    gate: &AudioGate,
    now: f64,
) -> Entity {
    let entity = commands.spawn_empty().id();
    start_due_tracks(&mut playback, entity, bus, gate, now, &mut |track| {
        commands.spawn(track).id()
    });
    commands.entity(entity).insert(playback);
    entity
}

/// [`spawn_playback`] on the world (a caller that holds the world).
pub fn spawn_playback_in_world(
    world: &mut World,
    mut playback: CuePlayback,
    now: f64,
) -> Entity {
    let bus = world.resource::<VolumeBus>().clone();
    let gate = *world.resource::<AudioGate>();
    let entity = world.spawn_empty().id();
    start_due_tracks(&mut playback, entity, &bus, &gate, now, &mut |track| {
        world.spawn(track).id()
    });
    world.entity_mut(entity).insert(playback);
    entity
}

/// Start every waiting track of the round whose delay has passed.
fn start_due_tracks(
    playback: &mut CuePlayback,
    entity: Entity,
    bus: &VolumeBus,
    gate: &AudioGate,
    now: f64,
    spawn: &mut dyn FnMut(TrackBundle) -> Entity,
) {
    for index in 0..playback.voices.len() {
        let RoundVoice::Waiting { slot, due } = playback.voices[index] else {
            continue;
        };
        if now < due {
            continue;
        }
        let voice = &playback.plan.voices[slot];
        let settings = if voice.stream.loops {
            PlaybackSettings::LOOP
                .with_start_position(Duration::from_secs_f64(voice.stream.loop_start))
                .with_duration(Duration::from_secs_f64(
                    voice.stream.loop_end - voice.stream.loop_start,
                ))
        } else {
            PlaybackSettings::ONCE
        };
        let mut volume = track_volume(
            playback.plan.gain.as_ref(),
            playback.player,
            playback.fallback_slot,
            slot,
        );
        volume.gain *= playback.scale;
        let linear = volume.linear(bus) * gate.factor();
        let sound = spawn((
            AudioPlayer::new(playback.handles[slot].clone()),
            settings.with_volume(Volume::Linear(linear)),
            BusVolume::Cue(volume),
            ChildOf(entity),
        ));
        if playback.plan.work_key.is_some() {
            debug!(
                "[audio-seq] 序列轨起声：{} · cue={} round={} track={} due={due:.6} t={now:.6} 音量 {linear:.3}{}",
                playback.channel,
                label(&playback.cue),
                playback.round,
                voice.track_index,
                if voice.stream.loops { "（波形带循环：这一轮不会自己收）" } else { "" }
            );
        }
        playback.voices[index] = RoundVoice::Sounding { entity: sound };
    }
}

/// [`start_cue`] with waveforms the caller already requested (in plan track
/// order), e.g. a timeline that prepares its sounds before it starts.
#[allow(clippy::too_many_arguments)]
pub fn start_cue_with_handles(
    work: &mut SequenceWorkAreas,
    rngs: &mut CueRngs,
    now: f64,
    cue: &str,
    package: &str,
    plan: &CuePlan,
    handles: Vec<Handle<AudioSource>>,
    player: Player,
    fallback_slot: usize,
    rng: RngSlot,
    channel: &'static str,
) -> CuePlayback {
    assert_eq!(
        handles.len(),
        plan.voices.len(),
        "cue {} needs one waveform per track",
        label(cue)
    );
    let mut playback = CuePlayback {
        cue: cue.to_owned(),
        package: package.to_owned(),
        plan: plan.clone(),
        handles,
        player,
        fallback_slot,
        rng,
        round: 0,
        voices: Vec::new(),
        channel,
        scale: 1.0,
    };
    start_round(&mut playback, work, rngs, now);
    playback
}

/// One start of the sequence: pick the tracks, schedule their delays.
fn start_round(
    playback: &mut CuePlayback,
    work: &mut SequenceWorkAreas,
    rngs: &mut CueRngs,
    now: f64,
) {
    let plan = &playback.plan;
    let slots = match &plan.work_key {
        None => (0..plan.voices.len()).collect::<Vec<_>>(),
        Some(key) => {
            let area = work
                .0
                .entry(key.clone())
                .or_insert_with(|| ShuffleWork::loaded(plan.track_ids.len()));
            let before = area.position;
            let slots = cue_law::start(
                plan.kind,
                &plan.track_ids,
                plan.weights.as_deref(),
                area,
                rngs.get(playback.rng),
            );
            debug!(
                "[audio-seq] 序列轮起：{} · cue={} @ {} type={} round={} pos_before={} pos={} slots={:?} tracks={:?} t={:.6}",
                playback.channel,
                label(&playback.cue),
                label(&playback.package),
                plan.kind.label(),
                playback.round + 1,
                before,
                area.position,
                slots,
                slots.iter().map(|slot| plan.voices[*slot].track_index).collect::<Vec<_>>(),
                now
            );
            slots
        }
    };
    playback.round += 1;
    playback.voices = slots
        .into_iter()
        .map(|slot| RoundVoice::Waiting {
            slot,
            due: now + plan.voices[slot].start_delay_us as f64 / 1.0e6,
        })
        .collect();
}

/// Update: run every playback. A track whose delay has passed starts (flat
/// 2D, once, or looped over its loop interval when the waveform loops); a
/// track that has played out (or whose waveform failed to load) is done; when
/// every track of the round is done the round ends, and a repeating sequence
/// starts its next round at once, otherwise the playback despawns. A round in
/// which no track was picked ends at once; a repeating one then starts again
/// in the next update, not in a loop within this one.
///
/// Starts and ends land on the first update after the moment (the middleware
/// steps on its own audio clock). Without an audio output device no sink
/// appears and a track stays sounding: a wall clock is not taken for its end.
#[allow(clippy::too_many_arguments)]
pub fn advance_cue_playbacks(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time<Real>>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    mut work: ResMut<SequenceWorkAreas>,
    mut rngs: ResMut<CueRngs>,
    mut playbacks: Query<(Entity, &mut CuePlayback)>,
    players: Query<&AudioPlayer<AudioSource>>,
    sinks: Query<&AudioSink>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, mut playback) in &mut playbacks {
        let playback = &mut *playback;
        let mut restarted = false;
        loop {
            start_due_tracks(playback, entity, &bus, &gate, now, &mut |track| {
                commands.spawn(track).id()
            });
            for index in 0..playback.voices.len() {
                match playback.voices[index] {
                    RoundVoice::Sounding { entity: sound, .. } => {
                        if one_shot_finished_or_failed(sound, &server, &players, &sinks) {
                            if let Ok(mut sound_commands) = commands.get_entity(sound) {
                                sound_commands.despawn();
                            }
                            playback.voices[index] = RoundVoice::Done;
                        }
                    }
                    _ => {}
                }
            }
            if !playback
                .voices
                .iter()
                .all(|voice| matches!(voice, RoundVoice::Done))
            {
                break;
            }
            let empty_round = playback.voices.is_empty();
            if playback.plan.work_key.is_some() {
                debug!(
                    "[audio-seq] 序列轮终：{} · cue={} round={} t={now:.6} repeat={}",
                    playback.channel,
                    label(&playback.cue),
                    playback.round,
                    u8::from(playback.plan.repeat)
                );
            }
            if !playback.plan.repeat {
                commands.entity(entity).despawn();
                break;
            }
            if restarted && empty_round {
                break; // next update starts the next round
            }
            start_round(playback, &mut work, &mut rngs, now);
            restarted = true;
        }
    }
}

/// Clock seed: the low 32 bits of the epoch in nanoseconds (JS milliseconds
/// on wasm). The middleware seeds from its own clock, so the values differ
/// from the source's and the distribution is the same.
#[cfg(not(target_arch = "wasm32"))]
fn clock_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时钟早于 Unix 纪元")
        .as_nanos() as u32
}

#[cfg(target_arch = "wasm32")]
fn clock_seed() -> u32 {
    (js_sys::Date::now() * 1.0e6) as u64 as u32
}

/// Seed environment variable (decimal u32), for replaying one session's
/// picks value by value; each player adds its slot number so the players do
/// not draw the same sequence.
const SEQUENCE_SEED_ENV: &str = "MOLY_AUDIO_SEQUENCE_SEED";

fn seeded_sequence_rng(slot: RngSlot) -> SequenceRng {
    let offset = slot as u32;
    let (seed, source) = match std::env::var(SEQUENCE_SEED_ENV) {
        Ok(raw) => (
            raw.trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("{SEQUENCE_SEED_ENV} 不是十进制 u32：{raw:?}"))
                .wrapping_add(offset),
            "环境变量",
        ),
        Err(_) => match crate::world_seed::seed() {
            Some(_) => (
                crate::world_seed::next_u64(crate::world_seed::Stream::Audio, "the audio sequence")
                    as u32,
                "world seed",
            ),
            None => (clock_seed(), "时钟"),
        },
    };
    let rng = SequenceRng::seeded(seed);
    info!(
        "[audio-seq] 播放器抽签源起种：{slot:?} seed={seed} source={source} state={:?}",
        rng.state()
    );
    rng
}

/// Gain facts from a cue's command lists, in the shape of a sequence block
/// (every export version carries them for structured cues; version 2 adds
/// the same shape for every listed cue under `cueCommands`): command 65 lists
/// the category indices as 32-bit big-endian numbers; command 87 is a 16-bit
/// volume in hundredths; command 89 is a volume in hundredths with a random
/// spread (second 16-bit value, hundredths) drawn per start from the player's
/// generator. The spread is not ported: its base value is used, and the cues
/// that carry it are counted at parse. `Err` names the command list that
/// does not have the exported shape.
pub fn gain_from_block(block: &serde_json::Value) -> Result<CueGain, String> {
    let records = |value: &serde_json::Value| -> Result<Vec<(u64, Vec<u8>)>, String> {
        let Some(list) = value.as_array() else {
            return Ok(Vec::new());
        };
        list.iter()
            .map(|record| {
                let code = record["code"]
                    .as_u64()
                    .ok_or("a command code is not an integer")?;
                let hex = record["bytes"]
                    .as_str()
                    .ok_or_else(|| format!("the bytes of command {code} are not a string"))?;
                Ok((code, decode_hex(hex)?))
            })
            .collect()
    };
    let volume_of = |list: &[(u64, Vec<u8>)]| -> Result<f32, String> {
        let mut volume = 1.0f32;
        for (code, bytes) in list {
            match (*code, bytes.as_slice()) {
                (COMMAND_VOLUME, [hi, lo]) | (COMMAND_RANDOM_VOLUME, [hi, lo, _, _]) => {
                    volume *= f32::from(u16::from_be_bytes([*hi, *lo])) / 100.0;
                }
                (COMMAND_VOLUME | COMMAND_RANDOM_VOLUME, other) => {
                    return Err(format!(
                        "volume command {code} is {} bytes long",
                        other.len()
                    ));
                }
                _ => {}
            }
        }
        Ok(volume)
    };
    let sequence = records(&block["commands"])?;
    let mut random_spread = sequence
        .iter()
        .any(|(code, _)| *code == COMMAND_RANDOM_VOLUME);
    let mut categories = Vec::new();
    for (code, bytes) in &sequence {
        if *code == COMMAND_CATEGORY {
            if bytes.len() % 4 != 0 {
                return Err(format!(
                    "category command is {} bytes long, not a multiple of 4",
                    bytes.len()
                ));
            }
            categories.extend(
                bytes
                    .chunks(4)
                    .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
    }
    let mut tracks = Vec::new();
    for track in block["tracks"].as_array().into_iter().flatten() {
        let mut synth = 1.0f32;
        for note in track["noteOns"].as_array().into_iter().flatten() {
            let list = records(&note["synth"]["commands"])?;
            random_spread |= list.iter().any(|(code, _)| *code == COMMAND_RANDOM_VOLUME);
            synth *= volume_of(&list)?;
        }
        let list = records(&track["commands"])?;
        random_spread |= list.iter().any(|(code, _)| *code == COMMAND_RANDOM_VOLUME);
        tracks.push(volume_of(&list)? * synth);
    }
    Ok(CueGain {
        categories,
        sequence: volume_of(&sequence)?,
        tracks,
        random_spread,
    })
}

/// Sequence command: the categories the cue belongs to.
const COMMAND_CATEGORY: u64 = 65;
/// Sequence, track and synth command: volume in hundredths.
const COMMAND_VOLUME: u64 = 87;
/// Volume in hundredths with a random spread.
pub const COMMAND_RANDOM_VOLUME: u64 = 89;

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return Err(format!("command bytes {hex:?} are not whole bytes"));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| format!("command bytes {hex:?} are not hexadecimal"))
        })
        .collect()
}
