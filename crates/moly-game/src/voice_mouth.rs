//! A real voice player's channel-zero meter drives its resolved NPC instance.
//!
//! The actor owns the continuation; the instance-local material owns only the
//! last full pattern input. Audio completion disables new cycles, not an
//! already scheduled source delay. Texture writes reuse the existing atlas path.
//! Timeline LipGate assigns this same NPCAvatarView's enable flag with a null
//! analyzer. It overrides ordinary metering on an evaluated timeline frame;
//! fixture-character and player-avatar speech still require separate adapters.

use std::collections::HashSet;

use bevy::prelude::*;
use moly_law::facial::{LipSync, LipTimes};

use crate::{
    alone_action_runtime::{apply_mouth, apply_mouth_pattern, FacialTables, Lcg},
    audio::{VoiceChannel, VoicePlaybackIdentity, VoiceSpeaker},
    character_material::{CharacterMaterial, ToonMaterials},
    npc::CharacterUnitId,
    player::PlayerControlled,
    voice_pcm::{VoiceRmsMeter, VoiceSource},
};

#[derive(Component)]
pub(crate) struct MouthController {
    handle: Handle<CharacterMaterial>,
    lip: LipSync,
    pattern_revision: u64,
    pattern_known: bool,
    remaining_seconds: Option<f32>,
    bound_request: Option<Entity>,
    warned_missing_meter: Option<Entity>,
    reported_pcm_request: Option<Entity>,
    reported_open_request: Option<Entity>,
    reported_close_request: Option<Entity>,
}

/// Reuse the existing bounded integer generator with a separate presentation
/// stream. Speech must not consume AloneRuntime's script-selection draws.
/// Unity's global PRNG sequence is not claimed equivalent to this host stream.
#[derive(Resource)]
pub(crate) struct MouthRandom(Lcg);

impl Default for MouthRandom {
    fn default() -> Self {
        Self(Lcg::seeded(0))
    }
}

pub(crate) fn attach(
    mut commands: Commands,
    tables: Option<Res<FacialTables>>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    npcs: Query<
        (Entity, &CharacterUnitId, &ToonMaterials),
        (Without<MouthController>, Without<PlayerControlled>),
    >,
    mut warned: Local<HashSet<Entity>>,
) {
    let Some(tables) = tables else { return; };
    warned.retain(|entity| npcs.get(*entity).is_ok());
    for (actor, unit, toon) in &npcs {
        let Some(handle) = toon.slot_handle("mouth").cloned() else {
            if warned.insert(actor) {
                warn!("[voice-mouth] actor={actor:?} unit={} has no mouth material", unit.0);
            }
            continue;
        };
        let Some(existing) = materials.get(&handle).map(|material| material.lip_pattern) else {
            continue;
        };
        let pattern = if let Some(pattern) = existing {
            // A script/timeline may have written before this component attached.
            pattern
        } else {
            let Some((_, name)) = tables.default_patterns(unit.0) else {
                if warned.insert(actor) {
                    warn!("[voice-mouth] actor={actor:?} unit={} has no authored default facial row; no mouth controller attached", unit.0);
                }
                continue;
            };
            let row = tables.lip_pattern(name);
            if row.is_none() && warned.insert(actor) {
                warn!("[voice-mouth] actor={actor:?} unit={} authored default lip row is absent; using source zero-valued FindBy result", unit.0);
            }
            let pattern = row.unwrap_or_default();
            apply_mouth_pattern(&mut materials, &handle, pattern);
            pattern
        };
        let pattern_revision = materials.get(&handle)
            .expect("mouth material was resolved above").lip_pattern_revision;
        commands.entity(actor).insert(MouthController {
            handle,
            lip: LipSync::new(pattern),
            pattern_revision,
            pattern_known: true,
            remaining_seconds: None,
            bound_request: None,
            warned_missing_meter: None,
            reported_pcm_request: None,
            reported_open_request: None,
            reported_close_request: None,
        });
    }
}

pub(crate) fn advance(
    time: Res<Time>,
    channel: Res<VoiceChannel>,
    players: Query<(&VoicePlaybackIdentity, Option<&VoiceRmsMeter>, Option<&VoiceSource>)>,
    mut actors: Query<
        (Entity, &ToonMaterials, &mut MouthController, Option<&crate::fixture_activity_timeline::TimelineFacialState>),
        Without<PlayerControlled>,
    >,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut random: ResMut<MouthRandom>,
    mut warned_identity: Local<Option<Entity>>,
    drawn_materials: Query<(
        Entity,
        &MeshMaterial3d<CharacterMaterial>,
        Option<&InheritedVisibility>,
        Option<&ViewVisibility>,
    )>,
) {
    let sink = channel.active_sink();
    let active = sink.and_then(|entity| players.get(entity).ok());
    if let Some(sink) = sink {
        if active.is_none() && *warned_identity != Some(sink) {
            warn!("[voice-mouth] active player={sink:?} has no resolved playback identity; no speaker guessed");
            *warned_identity = Some(sink);
        }
    }
    for (actor, toon, mut controller, timeline) in &mut actors {
        let Some(handle) = toon.slot_handle("mouth") else { continue; };
        let Some((revision, pattern)) = materials.get(handle)
            .map(|material| (material.lip_pattern_revision, material.lip_pattern)) else {
                continue;
            };
        if controller.handle != *handle || controller.pattern_revision != revision {
            controller.handle = handle.clone();
            controller.pattern_revision = revision;
            controller.pattern_known = pattern.is_some();
            if let Some(pattern) = pattern {
                // The originating writer already wrote Close. Do not repeat
                // that write here or cancel the old captured delayed index.
                controller.lip.change_pattern(pattern);
            } else {
                // A timeline can restore an older raw material that had no
                // full-row input yet. Preserve its ST; do not infer a pattern
                // from one cell or start new cycles with the timeline's row.
                // An already pending source continuation still finishes.
                warn!("[voice-mouth] actor={actor:?} restored a material without a full lip row; new cycles suspended until an authored pattern arrives");
            }
        }

        let actor_voice = active.filter(|(identity, _, _)|
            identity.speaker == Some(VoiceSpeaker::Npc(actor)));
        let (request, rms) = match actor_voice {
            Some((identity, Some(meter), _)) if meter.request == identity.request => {
                let rms = meter.latest();
                if rms >= 0.1 && controller.reported_pcm_request != Some(identity.request) {
                    // One observation per utterance, not a per-frame level log.
                    // The meter belongs to the decoder feeding this same player.
                    info!("[voice-mouth] actor={actor:?} player={sink:?} request={:?} real PCM reached lip threshold: frames={} rms={rms:.4} pattern={:?} material={:?}",
                        identity.request, meter.observed_frames(), controller.lip.pattern(), controller.handle.id());
                    controller.reported_pcm_request = Some(identity.request);
                }
                (Some(identity.request), Some(rms))
            },
            // Only a genuinely absent meter on an explicitly unprepared raw
            // source is expected. A wrong meter token must still warn, even
            // if somebody left the source marked unprepared.
            Some((_, None, Some(source))) if source.awaiting_raw_source() => (None, None),
            Some((identity, _, _)) => {
                if controller.warned_missing_meter != Some(identity.request) {
                    warn!("[voice-mouth] actor={actor:?} request={:?} has no matching real PCM meter outside the expected raw-loading state; new cycles disabled", identity.request);
                    controller.warned_missing_meter = Some(identity.request);
                }
                (None, None)
            }
            None => (None, None),
        };
        if controller.bound_request != request {
            controller.bound_request = request;
            trace!("[voice-mouth] actor={actor:?} bound request={request:?}");
        }
        let timeline_gate = timeline.and_then(|state| state.lip_gate);
        let enabled = timeline_gate.unwrap_or(rms.is_some()) && controller.pattern_known;
        controller.lip.set_enabled(enabled);

        debug_assert_eq!(controller.lip.is_playing(), controller.remaining_seconds.is_some());
        let directive = if let Some(remaining) = controller.remaining_seconds.take() {
            let remaining = remaining - time.delta_secs();
            if remaining > 0.0 {
                controller.remaining_seconds = Some(remaining);
                continue;
            }
            // Resume once. A newly created delay begins after this host Update
            // boundary and never consumes leftover dt from the preceding wait.
            controller.lip.resume_delay(LipTimes::CN_6, &mut random.0)
        } else if controller.pattern_known {
            // ChangeLipSyncStateMixerBehaviour passes null for BOTH enable
            // and disable. Null is an authored sound-independent cycle, not a
            // substitute for a missing meter on an ordinary playing voice.
            let analyzer = if timeline_gate.is_some() { None } else { rms };
            controller.lip.update_mouth(analyzer, LipTimes::CN_6, &mut random.0)
        } else {
            continue;
        };
        if let Some(index) = directive.texture {
            // Two bounded observations per actual voice request. Inspect the
            // existing writer and actual mesh handles; do not force a phase,
            // change any delay/RMS threshold, or mark an idle asset modified.
            let pattern = controller.lip.pattern();
            let phase = request.and_then(|request| {
                if pattern.open == pattern.close {
                    return None; // Authored lip01..lip16 are fixed-cell rows.
                }
                if index == pattern.open && controller.reported_open_request != Some(request) {
                    Some((request, "open"))
                } else if index == pattern.close
                    && controller.reported_open_request == Some(request)
                    && controller.reported_close_request != Some(request)
                {
                    Some((request, "close-after-open"))
                } else {
                    None
                }
            });
            let before = phase.and_then(|_| materials.get(&controller.handle)
                .map(|material| material.params.main_tex_st));
            apply_mouth(&mut materials, &controller.handle, Some(&index));
            if let Some((request, phase)) = phase {
                let after = materials.get(&controller.handle)
                    .map(|material| material.params.main_tex_st);
                let draws: Vec<_> = drawn_materials.iter()
                    .filter(|(_, material, _, _)| material.0 == controller.handle)
                    .map(|(mesh, _, inherited, view)| (
                        mesh,
                        inherited.map(|state| state.get()),
                        view.map(|state| state.get()),
                    ))
                    .collect();
                info!("[voice-mouth] actual atlas write actor={actor:?} current-request={request:?} phase={phase} index={index} pattern={pattern:?} material={:?} before={before:?} after={after:?} mesh-inherited-last-view={draws:?}",
                    controller.handle.id());
                if phase == "open" {
                    controller.reported_open_request = Some(request);
                } else {
                    controller.reported_close_request = Some(request);
                }
            }
        }
        controller.remaining_seconds = directive.wait_millis.map(|ms| ms as f32 / 1000.0);
    }
}
