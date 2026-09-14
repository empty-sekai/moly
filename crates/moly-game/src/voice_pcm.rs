//! Per-playback observation of the decoder used by the real voice player.
//!
//! This adapter is for the existing one-shot Vorbis dialogue sources. It keeps
//! their sample type, channel count, rate and samples unchanged; only channel
//! zero is converted to float for the level meter, before sink gain/resampling.
//! Decoder-buffer partitioning and pull-time publication are host adaptations,
//! not a claim that transcoded Vorbis has another codec's callback boundaries.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bevy::asset::{AssetId, LoadState};
use bevy::audio::{
    AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, CpalSample,
    Decodable, SeekError, Source,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use moly_law::facial::ChannelRms;

use crate::audio::{VoiceChannel, VoicePlaybackIdentity};

/// Configuration count, not a request to resample the file to this rate.
const ANALYZER_CONFIGURED_RATE: u32 = 48_000;

#[derive(Default)]
struct PublishedRms {
    value_bits: AtomicU32,
    observed_frames: AtomicUsize,
}

/// The same request token as the identity on the actual player entity.
/// The initial cached value is zero until real decoded buffers publish a value.
#[derive(Component, Clone)]
pub(crate) struct VoiceRmsMeter {
    pub(crate) request: Entity,
    published: Arc<PublishedRms>,
}

impl VoiceRmsMeter {
    fn new(request: Entity) -> Self {
        Self { request, published: Arc::new(PublishedRms::default()) }
    }

    pub(crate) fn latest(&self) -> f32 {
        f32::from_bits(self.published.value_bits.load(Ordering::Acquire))
    }

    /// Diagnostic count of actual channel-zero frames in completed host buffers.
    pub(crate) fn observed_frames(&self) -> usize {
        self.published.observed_frames.load(Ordering::Relaxed)
    }
}

/// One asset per actual playback. AudioSource::clone shares its encoded bytes;
/// it does not fetch a second file or create a second decoder for the meter.
#[derive(Asset, TypePath)]
pub(crate) struct MeteredVoiceSource {
    source: AudioSource,
    meter: VoiceRmsMeter,
}

impl Decodable for MeteredVoiceSource {
    type DecoderItem = <AudioSource as Decodable>::DecoderItem;
    type Decoder = MeteredVoiceDecoder;

    fn decoder(&self) -> Self::Decoder {
        MeteredVoiceDecoder {
            inner: self.source.decoder(),
            meter: self.meter.clone(),
            accumulator: ChannelRms::new(ANALYZER_CONFIGURED_RATE)
                .expect("the configured analyzer count must be nonzero"),
            packet_remaining: 0,
            channels: 0,
            channel: 0,
            channel_zero: Vec::new(),
        }
    }
}

pub(crate) struct MeteredVoiceDecoder {
    inner: <AudioSource as Decodable>::Decoder,
    meter: VoiceRmsMeter,
    accumulator: ChannelRms,
    packet_remaining: usize,
    channels: usize,
    channel: usize,
    channel_zero: Vec<f32>,
}

impl Iterator for MeteredVoiceDecoder {
    type Item = <AudioSource as Decodable>::DecoderItem;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        if self.packet_remaining == 0 {
            // The pinned Vorbis decoder exposes current_data.len(), not the
            // remaining sample count. Its first next() after exhaustion fills
            // the next buffer, so read the length AFTER obtaining that sample.
            let count = self.inner.current_frame_len()
                .expect("the voice Vorbis decoder must expose its buffer length");
            self.channels = usize::from(self.inner.channels());
            assert!(count != 0 && self.channels != 0 && count % self.channels == 0,
                "the voice Vorbis buffer must contain complete interleaved frames");
            self.packet_remaining = count;
            self.channel = 0;
        }
        if self.channel == 0 {
            self.channel_zero.push(<f32 as CpalSample>::from_sample(sample));
        }
        self.channel = (self.channel + 1) % self.channels;
        self.packet_remaining -= 1;
        if self.packet_remaining == 0 {
            // Submit this complete actual buffer, including any quota overshoot.
            // No fixed-sized slicing, padding, EOF flush or video-clock samples.
            if let Some(value) = self.accumulator.submit_channel_zero(&self.channel_zero)
                .expect("a decoded voice buffer must fit the analyzer counter")
            {
                self.meter.published.value_bits.store(value.to_bits(), Ordering::Release);
            }
            self.meter.published.observed_frames
                .fetch_add(self.channel_zero.len(), Ordering::Relaxed);
            self.channel_zero.clear();
        }
        // Preserve the original item type as well as the value, so the existing
        // sink still applies volume in the same sample representation.
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl Source for MeteredVoiceDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        // The pinned Vorbis decoder returns NotSupported without moving its
        // cursor. Forward that result; do not invent seeking or reset its meter.
        self.inner.try_seek(position)
    }
}

/// The channel owns this same entity throughout real loading and playback.
#[derive(Component)]
pub(crate) struct VoiceSource {
    raw: Handle<AudioSource>,
    prepared: Option<AssetId<MeteredVoiceSource>>,
}

impl VoiceSource {
    pub(crate) fn new(raw: Handle<AudioSource>) -> Self {
        Self { raw, prepared: None }
    }

    /// Before prepare has installed the typed player, absence of its meter is
    /// an expected raw-asset loading state, not a broken analyzer connection.
    pub(crate) fn awaiting_raw_source(&self) -> bool {
        self.prepared.is_none()
    }
}

/// Run after serve_voice and its command flush. Only the active channel entity
/// can prepare; intermediate requests stopped in the same frame never decode.
pub(crate) fn prepare(
    mut commands: Commands,
    channel: Res<VoiceChannel>,
    raw_sources: Res<Assets<AudioSource>>,
    mut metered_sources: ResMut<Assets<MeteredVoiceSource>>,
    mut sources: Query<(&mut VoiceSource, &VoicePlaybackIdentity)>,
) {
    let Some(entity) = channel.active_sink() else { return; };
    let Ok((mut source, identity)) = sources.get_mut(entity) else { return; };
    if source.prepared.is_some() { return; }
    let Some(raw) = raw_sources.get(&source.raw) else { return; };
    let meter = VoiceRmsMeter::new(identity.request);
    let handle = metered_sources.add(MeteredVoiceSource {
        source: raw.clone(),
        meter: meter.clone(),
    });
    source.prepared = Some(handle.id());
    commands.entity(entity).insert((AudioPlayer(handle), meter));
}

/// Read-only lifetime classification for the existing channel owner. It alone
/// clears channel bookkeeping and despawns; this module has no stop/Dispose bus.
pub(crate) fn finished_or_failed(
    entity: Entity,
    server: &AssetServer,
    sources: &Query<&VoiceSource>,
    players: &Query<&AudioPlayer<MeteredVoiceSource>>,
    assets: &Assets<MeteredVoiceSource>,
    sinks: &Query<&AudioSink>,
) -> bool {
    let Ok(source) = sources.get(entity) else { return true; };
    if let Some(expected) = source.prepared {
        let Ok(player) = players.get(entity) else { return true; };
        if player.0.id() != expected {
            warn!("voice player {entity:?} no longer owns its prepared source; releasing player");
            return true;
        }
        if let Ok(sink) = sinks.get(entity) {
            return sink.empty();
        }
        // This asset was inserted synchronously, not scheduled for loading.
        if assets.get(expected).is_none() {
            warn!("voice player {entity:?} lost its prepared source; releasing player");
            return true;
        }
        // Bevy may still be preparing, or have no output device. Neither is a
        // fabricated completion event. The prepared asset is already real.
        return false;
    }
    if let Some(LoadState::Failed(error)) = server.get_load_state(source.raw.id()) {
        warn!("voice player {entity:?} failed loading {:?}: {error}; releasing player",
            source.raw.path());
        return true;
    }
    // A pending raw load intentionally has no typed AudioPlayer yet.
    false
}

/// Register only after Bevy's asset/audio plugins. The caller owns scheduling
/// of prepare between the existing voice dispatcher and the mouth consumer.
pub(crate) fn install(app: &mut App) {
    app.add_audio_source::<MeteredVoiceSource>();
}
