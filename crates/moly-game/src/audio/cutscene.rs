//! The cut-scene screen's sound calls (`Sekai.Mysekai.Timeline`):
//! the BGM track's `SoundManager.PlayBGM(cue)` and the voice track's
//! `SoundManager.PlayVoiceForMysekai` / `StopVoice`. The cut-scene's screen
//! tracks call [`play_bgm`], [`play_voice`] and [`stop_voice`]; the cut-scene's
//! `EndAsync` calls [`release_bgm`].
//!
//! **BGM.** `BGMBehaviour.PlayBGM(cueName)`: an empty cue is an error and
//! plays nothing; otherwise `SoundManager.PlayBGM(cueName, fadeTime 0.25,
//! isSameBGMRewind false, callback null, startTime 0, priority 0)`, the cue
//! alone (the clip's `_assetBundleName` names the cue sheet the cut-scene
//! loads, `LoadBgms`, so it names the stream's package here: the bundle path
//! with `/` as `__`). The BGM player keeps a cue it already holds, and
//! otherwise crossfades to the new one over the fade time, from its start,
//! looping as every BGM does. The behaviour has no pause: the cue plays on
//! after its clip, until the next BGM call.
//!
//! Named rule of the port: the BGM channel chooses its music every frame
//! from the site, the phenomenon, the site's music record setting and the
//! BGM select screen's choice ([`super::record`]). A cut-scene's call stands
//! over all of them from the call until the cut-scene's `EndAsync`
//! ([`release_bgm`]); then the channel's own choice plays again (crossfaded
//! with its own fade time). For the callers whose `EndAsync` restores the
//! BGM that is the source's order. The birthday party passes `restoreBGM
//! false` and its party sequence (`CacheBirthdayBgm`, the party's talks and
//! the site BGM it plays at the end) takes the music over, which is not run
//! here: its cut-scene's music is handed back at the end, named.
//!
//! A cue whose stream is not in the root's stream table is refused loudly
//! once per cue and changes nothing (the current music keeps playing).
//!
//! **Voice.** `VoiceBehaviour.PlayVoice(cue)`: an empty cue is an error;
//! when `ExistsCueName(cue)` is false the call returns silently (named here:
//! logged); otherwise `PlayVoiceForMysekai(cue, startTime, volume 1, onPlay,
//! onFinish, onNotFound)` with `startTime = (float)((double)(float)rootTime
//! - clip.start)`; `onPlay` turns the bound `NPCAvatarView`'s lip sync on
//! with the playback's analyzer, `onFinish` and `onNotFound` turn it off.
//! `StopVoice` stops that playback. The cue sheet is the one the caller
//! downloaded for the cut-scene (the birthday party's `DownloadVoiceAsync`:
//! `mysekai/talk/birthday/voice/<timeline>`).
//!
//! Product mapping: the voice plays on the voice bus the dialogue voice uses
//! ([`super::VoiceChannel`]), so the lip sync is the same meter-driven mouth
//! ([`crate::voice_mouth`]) bound to the track's character. Named
//! differences: the source's voice player has several playback slots, the
//! bus one (a cut-scene voice replaces a dialogue voice; none plays in a
//! cut-scene); the start offset is taken when the play is commanded, so the
//! waveform's load time is not added to it.

use super::*;

/// `BGMBehaviour.PlayBGM`'s fade time (a literal of the method).
pub(crate) const PLAY_BGM_FADE_SECONDS: f32 = 0.25;

/// A cut-scene's BGM call standing over the channel's choice.
struct Standing {
    cue: String,
    package: String,
    caller: String,
}

/// The cut-scene's sound state.
#[derive(Resource, Default)]
pub(crate) struct CutSceneAudio {
    bgm: Option<Standing>,
    /// The cue last refused (a stream not in the table), reported once.
    refused: Option<String>,
}

/// A cue sheet's bundle path as the stream table's package name.
pub(crate) fn bundle_package(bundle: &str) -> String {
    bundle.replace('/', "__")
}

/// `BGMBehaviour.PlayBGM(cueName)` with the clip's cue sheet bundle.
pub(crate) fn play_bgm(world: &mut World, cue: &str, bundle: &str, caller: &str) {
    if cue.is_empty() {
        error!("[audio] cut-scene BGM ({caller}): BGMBehaviour.PlayBGM with an empty cue: LogError, nothing plays");
        return;
    }
    let package = bundle_package(bundle);
    info!("[audio] cut-scene BGM ({caller}): SoundManager.PlayBGM({cue}, fade {PLAY_BGM_FADE_SECONDS} s, no rewind, start 0) from {package}; it stands over the site's BGM choice until the cut-scene ends");
    world.resource_mut::<CutSceneAudio>().bgm = Some(Standing {
        cue: cue.to_owned(),
        package,
        caller: caller.to_owned(),
    });
}

/// The cut-scene's `EndAsync`: the channel's own choice plays again.
pub(crate) fn release_bgm(world: &mut World, why: &str) {
    let mut audio = world.resource_mut::<CutSceneAudio>();
    audio.refused = None;
    if let Some(standing) = audio.bgm.take() {
        info!(
            "[audio] cut-scene BGM ({}): {} released ({why}); the site's BGM choice plays again",
            standing.caller, standing.cue
        );
    }
}

/// The BGM channel's frame while a cut-scene's call stands: `true` when it
/// does (the channel's own choice waits).
#[allow(clippy::too_many_arguments)]
pub(super) fn stands(
    audio: &mut CutSceneAudio,
    commands: &mut Commands,
    server: &AssetServer,
    routing: &Routing,
    channel: &mut BgmChannel,
    bus: &VolumeBus,
    bgm_player: f32,
    gate: &AudioGate,
) -> bool {
    let Some(standing) = audio.bgm.as_ref() else {
        return false;
    };
    // isSameBGMRewind false: the cue the channel holds keeps playing.
    if channel
        .voice
        .as_ref()
        .is_some_and(|voice| voice.cue == standing.cue)
    {
        return true;
    }
    let key = (standing.cue.clone(), standing.package.clone());
    let Some(stream) = routing.streams.0.get(&key) else {
        if audio.refused.as_deref() != Some(standing.cue.as_str()) {
            error!(
                "[audio] cut-scene BGM ({}): {} @ {} is not in the stream table; the current music keeps playing",
                standing.caller,
                label(&standing.cue),
                label(&standing.package)
            );
            audio.refused = Some(standing.cue.clone());
        }
        return true;
    };
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
            seconds: PLAY_BGM_FADE_SECONDS,
        });
    }
    let cold = channel.fading.is_empty();
    let cue_volume = track_volume(
        routing.cue_gain(&standing.cue, &standing.package),
        Player::Bgm,
        SLOT_BGM,
        0,
    );
    let target = BGM_VOLUME_FACTOR * cue_volume.linear_at(bus, bgm_player) * gate.factor();
    let now = Volume::Linear(if cold { target } else { 0.0 });
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    // The BGM player forces the loop: the loop region after the intro, or
    // the whole waveform when it has no loop points.
    let (intro, loop_sink, handoff_done) = if stream.loops && stream.loop_start > 0.0 {
        let intro = commands
            .spawn((
                AudioPlayer::new(handle.clone()),
                PlaybackSettings::ONCE
                    .with_duration(Duration::from_secs_f64(stream.loop_start))
                    .with_volume(now),
            ))
            .id();
        let loop_sink = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::LOOP
                    .with_start_position(Duration::from_secs_f64(stream.loop_start))
                    .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
                    .with_volume(now)
                    .paused(),
            ))
            .id();
        (Some(intro), loop_sink, false)
    } else {
        let settings = if stream.loops {
            PlaybackSettings::LOOP.with_duration(Duration::from_secs_f64(stream.loop_end))
        } else {
            PlaybackSettings::LOOP
        };
        let loop_sink = commands
            .spawn((AudioPlayer::new(handle), settings.with_volume(now)))
            .id();
        (None, loop_sink, true)
    };
    channel.voice = Some(BgmVoice {
        cue: standing.cue.clone(),
        intro,
        loop_sink: Some(loop_sink),
        loop_start: stream.loop_start,
        loop_end: stream.loop_end,
        handoff_at: stream.loop_start,
        handoff_done,
        fade_in: if cold { None } else { Some(0.0) },
        fade_seconds: PLAY_BGM_FADE_SECONDS,
        applied_volume: if cold { target } else { 0.0 },
        volume: cue_volume,
    });
    info!(
        "[audio] cut-scene BGM ({}): {} @ {} starts ({}, volume {target:.3}; {})",
        standing.caller,
        label(&standing.cue),
        label(&standing.package),
        if cold {
            "cold start".to_owned()
        } else {
            format!("crossfade {PLAY_BGM_FADE_SECONDS} s")
        },
        if stream.loops {
            format!(
                "intro {:.3} s, loop [{:.3}, {:.3}]",
                stream.loop_start, stream.loop_start, stream.loop_end
            )
        } else {
            "no loop points: the whole waveform loops".to_owned()
        }
    );
    true
}

/// `VoiceBehaviour.PlayVoice(cue)` -> `PlayVoiceForMysekai(cue, start, 1)`
/// from the caller's voice bank, with `speaker`'s lip sync. Returns the
/// playback (for [`stop_voice`]), or why nothing plays.
pub(crate) fn play_voice(
    world: &mut World,
    cue: &str,
    bank: &str,
    start: f32,
    speaker: Option<Entity>,
) -> Result<Entity, String> {
    if cue.is_empty() {
        return Err("VoiceBehaviour.PlayVoice with an empty cue: LogError".into());
    }
    let (path, cue_volume, duration) = {
        let Some(routing) = world.get_resource::<Routing>() else {
            return Err("the audio routing is not ready".into());
        };
        let key = (cue.to_owned(), bank.to_owned());
        let Some(stream) = routing.streams.0.get(&key) else {
            return Err(format!(
                "ExistsCueName({}) is false: the voice bank {} is not in the stream table (the source returns silently)",
                label(cue),
                label(bank)
            ));
        };
        (
            format!("moly://{}", stream.ogg),
            track_volume(
                routing.cue_gain(cue, bank),
                Player::Voice,
                SLOT_VOX_SCENARIO,
                0,
            ),
            stream.duration,
        )
    };
    let volume =
        cue_volume.linear(world.resource::<VolumeBus>()) * world.resource::<AudioGate>().factor();
    let handle = world
        .resource::<AssetServer>()
        .load::<AudioSource>(AssetPath::from(path));
    let ready = matches!(
        world.resource::<AssetServer>().get_load_state(handle.id()),
        Some(LoadState::Loaded)
    );
    let now = world.resource::<Time>().elapsed_secs_f64();
    let old = world.resource_mut::<VoiceChannel>().sink.take();
    if let Some(old) = old {
        world.despawn(old);
    }
    let mut settings = PlaybackSettings::ONCE.with_volume(Volume::Linear(volume));
    if start > 0.0 {
        settings = settings.with_start_position(Duration::from_secs_f32(start));
    }
    let sink = world
        .spawn((
            VoiceSource::new(handle, now, false, ready),
            settings,
            BusVolume::Cue(cue_volume),
        ))
        .id();
    // The playback is its own request token (the mouth reads the meter of
    // this request only).
    world.entity_mut(sink).insert(VoicePlaybackIdentity {
        request: sink,
        speaker: speaker.map(VoiceSpeaker::Npc),
    });
    let mut channel = world.resource_mut::<VoiceChannel>();
    channel.sink = Some(sink);
    channel.cue = Some(cue.to_owned());
    info!(
        "[audio] cut-scene voice: PlayVoiceForMysekai({}, start {start:.4}, volume 1) from {} ({duration:.2} s, volume {volume:.2}); lip sync on {speaker:?}",
        label(cue),
        label(bank)
    );
    Ok(sink)
}

/// `VoiceBehaviour.StopVoice`: stops the playback when it still sounds.
pub(crate) fn stop_voice(world: &mut World, playback: Entity) {
    let mut channel = world.resource_mut::<VoiceChannel>();
    if channel.sink != Some(playback) {
        return; // played out (reaped) or replaced
    }
    channel.sink = None;
    channel.cue = None;
    world.despawn(playback);
    info!("[audio] cut-scene voice: StopVoice({playback:?})");
}
