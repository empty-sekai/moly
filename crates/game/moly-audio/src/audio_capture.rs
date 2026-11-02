//! Rendering the mix at the game's pace instead of the sound card's.
//!
//! With `dev.record_audio` (`MOLY_RECORD_AUDIO`) on, the default output
//! device is replaced, before the engine opens its audio output, by a device
//! that plays nothing. The engine's mixer becomes that device's callback, and
//! each frame, in `Last`, renders it for the sample frames the real clock
//! advanced by: `round(elapsed × 48 000)` frames in total since start. A
//! recording steps that clock by a fixed `1/fps`, so every recorded frame
//! carries exactly its own share of the mix, however long encoding takes.
//!
//! While a recording is open ([`AudioCapture::start`]) the rendered samples
//! are appended to its WAV (32-bit float, stereo, 48 kHz);
//! [`AudioCapture::stop`] completes the file. Without the setting the sound
//! card plays the mix and nothing here is installed.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bevy::prelude::*;
use cpal::traits::{DeviceTrait, StreamTrait};

/// The sample rate the mix is rendered at.
pub const SAMPLE_RATE: u32 = 48_000;
/// The channels the mix is rendered in.
pub const CHANNELS: u16 = 2;
/// The most sample frames one game frame renders; a longer stall of the
/// clock is skipped rather than caught up in one frame.
const MOST_FRAMES: u64 = SAMPLE_RATE as u64;

/// `dev.record_audio` (`MOLY_RECORD_AUDIO`).
pub fn recording_audio() -> bool {
    match moly_config::global().flag(&moly_config::keys::DEV_RECORD_AUDIO) {
        Ok(flag) => flag.unwrap_or(false),
        Err(error) => panic!("[audio-capture] {error}"),
    }
}

type Callback = Box<dyn FnMut(&mut cpal::Data, &cpal::OutputCallbackInfo) + Send>;

/// What the device, its stream and the frame loop share: the callback the
/// engine's output gave the stream, and whether the stream plays.
#[derive(Default)]
struct Shared {
    callback: Mutex<Option<Callback>>,
    playing: AtomicBool,
}

/// The output device that plays nothing; its stream is rendered by
/// [`render`].
#[derive(Clone)]
struct Device(Arc<Shared>);

struct Stream(Arc<Shared>);

impl StreamTrait for Stream {
    fn play(&self) -> Result<(), cpal::PlayStreamError> {
        self.0.playing.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn pause(&self) -> Result<(), cpal::PauseStreamError> {
        self.0.playing.store(false, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.0.playing.store(false, Ordering::Relaxed);
        *self.0.callback.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn config() -> cpal::SupportedStreamConfig {
    cpal::SupportedStreamConfig::new(CHANNELS, SAMPLE_RATE, cpal::SupportedBufferSize::Unknown, cpal::SampleFormat::F32)
}

impl DeviceTrait for Device {
    type SupportedInputConfigs = std::iter::Empty<cpal::SupportedStreamConfigRange>;
    type SupportedOutputConfigs = std::iter::Once<cpal::SupportedStreamConfigRange>;
    type Stream = Stream;

    fn description(&self) -> Result<cpal::DeviceDescription, cpal::DeviceNameError> {
        Ok(cpal::DeviceDescriptionBuilder::new("moly frame-paced output".to_owned()).build())
    }

    fn id(&self) -> Result<cpal::DeviceId, cpal::DeviceIdError> {
        Err(cpal::DeviceIdError::UnsupportedPlatform)
    }

    fn supported_input_configs(&self) -> Result<Self::SupportedInputConfigs, cpal::SupportedStreamConfigsError> {
        Ok(std::iter::empty())
    }

    fn supported_output_configs(&self) -> Result<Self::SupportedOutputConfigs, cpal::SupportedStreamConfigsError> {
        Ok(std::iter::once(cpal::SupportedStreamConfigRange::new(
            CHANNELS,
            SAMPLE_RATE,
            SAMPLE_RATE,
            cpal::SupportedBufferSize::Unknown,
            cpal::SampleFormat::F32,
        )))
    }

    fn default_input_config(&self) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Err(cpal::DefaultStreamConfigError::StreamTypeNotSupported)
    }

    fn default_output_config(&self) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Ok(config())
    }

    fn build_input_stream_raw<D, E>(
        &self,
        _: &cpal::StreamConfig,
        _: cpal::SampleFormat,
        _: D,
        _: E,
        _: Option<std::time::Duration>,
    ) -> Result<Self::Stream, cpal::BuildStreamError>
    where
        D: FnMut(&cpal::Data, &cpal::InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        Err(cpal::BuildStreamError::StreamConfigNotSupported)
    }

    fn build_output_stream_raw<D, E>(
        &self,
        config: &cpal::StreamConfig,
        format: cpal::SampleFormat,
        callback: D,
        _: E,
        _: Option<std::time::Duration>,
    ) -> Result<Self::Stream, cpal::BuildStreamError>
    where
        D: FnMut(&mut cpal::Data, &cpal::OutputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        if format != cpal::SampleFormat::F32 || config.channels != CHANNELS || config.sample_rate != SAMPLE_RATE {
            return Err(cpal::BuildStreamError::StreamConfigNotSupported);
        }
        let mut slot = self.0.callback.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            // One engine output renders through this device at a time.
            return Err(cpal::BuildStreamError::DeviceNotAvailable);
        }
        *slot = Some(Box::new(callback));
        Ok(Stream(self.0.clone()))
    }
}

/// An open WAV of rendered samples.
struct Wav {
    path: PathBuf,
    file: BufWriter<File>,
    samples: u64,
}

/// A 32-bit float WAV header for `samples` interleaved samples.
fn wav_header(samples: u64) -> Result<[u8; 44], String> {
    let data = u32::try_from(samples * 4).map_err(|_| "the recorded sound is longer than a WAV file holds".to_owned())?;
    let block = CHANNELS * 4;
    let mut header = [0u8; 44];
    let mut at = 0;
    let mut put = |bytes: &[u8]| {
        header[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    };
    put(b"RIFF");
    put(&(36 + data).to_le_bytes());
    put(b"WAVEfmt ");
    put(&16u32.to_le_bytes());
    // Format 3: IEEE float.
    put(&3u16.to_le_bytes());
    put(&CHANNELS.to_le_bytes());
    put(&SAMPLE_RATE.to_le_bytes());
    put(&(SAMPLE_RATE * u32::from(block)).to_le_bytes());
    put(&block.to_le_bytes());
    put(&32u16.to_le_bytes());
    put(b"data");
    put(&data.to_le_bytes());
    Ok(header)
}

/// The frame-paced mix, present when `dev.record_audio` is on.
#[derive(Resource)]
pub struct AudioCapture {
    shared: Arc<Shared>,
    /// Sample frames rendered since start.
    rendered: u64,
    wav: Option<Wav>,
    /// The first write error of the open WAV.
    failed: Option<String>,
}

impl AudioCapture {
    /// Starts appending the rendered mix to a WAV at `path`.
    pub fn start(&mut self, path: &Path) -> Result<(), String> {
        if self.wav.is_some() {
            return Err("a recording of the sound is already open".to_owned());
        }
        let mut file = File::create(path).map(BufWriter::new).map_err(|error| format!("{}: {error}", path.display()))?;
        file.write_all(&wav_header(0)?).map_err(|error| format!("{}: {error}", path.display()))?;
        self.wav = Some(Wav { path: path.to_owned(), file, samples: 0 });
        self.failed = None;
        Ok(())
    }

    /// Completes the open WAV and answers its path; `None` when none was open.
    pub fn stop(&mut self) -> Result<Option<PathBuf>, String> {
        let Some(mut wav) = self.wav.take() else {
            return Ok(None);
        };
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let header = wav_header(wav.samples)?;
        let path = wav.path.clone();
        let written = (|| {
            wav.file.seek(SeekFrom::Start(0))?;
            wav.file.write_all(&header)?;
            wav.file.flush()
        })();
        written.map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Some(path))
    }

    /// Renders `frames` sample frames of the mix; silence while the engine's
    /// output has no stream or has paused it.
    fn render_frames(&mut self, frames: usize) -> Vec<f32> {
        let mut buffer = vec![0.0f32; frames * usize::from(CHANNELS)];
        let elapsed = std::time::Duration::from_nanos(self.rendered * 1_000_000_000 / u64::from(SAMPLE_RATE));
        self.rendered += frames as u64;
        if !self.shared.playing.load(Ordering::Relaxed) {
            return buffer;
        }
        let mut slot = self.shared.callback.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(callback) = slot.as_mut() else {
            return buffer;
        };
        let instant = cpal::StreamInstant::new(elapsed.as_secs() as i64, elapsed.subsec_nanos());
        let info = cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp { callback: instant, playback: instant });
        // SAFETY: the pointer and length describe `buffer`, which outlives the
        // call, holds `f32` samples and is not otherwise borrowed meanwhile.
        let mut data = unsafe { cpal::Data::from_parts(buffer.as_mut_ptr().cast(), buffer.len(), cpal::SampleFormat::F32) };
        callback(&mut data, &info);
        drop(slot);
        buffer
    }
}

/// Replaces the default output device with the frame-paced one when
/// `dev.record_audio` is on. It must run before the engine's audio plugin
/// opens its output, that is before the default plugins are added.
pub fn install_before_output(app: &mut App) {
    if !recording_audio() {
        return;
    }
    let shared = Arc::new(Shared::default());
    cpal::set_default_output_device(Some(cpal::platform::CustomDevice::from_device(Device(shared.clone()))));
    app.insert_resource(AudioCapture { shared, rendered: 0, wav: None, failed: None });
    app.add_systems(Startup, || {
        info!("[audio-capture] the mix is rendered by the frame loop at {SAMPLE_RATE} Hz; the sound card is not used");
    });
    app.add_systems(Last, render);
}

/// Renders the mix up to the real clock and appends it to the open WAV.
fn render(time: Res<Time<Real>>, mut capture: ResMut<AudioCapture>) {
    let target = (time.elapsed_secs_f64() * f64::from(SAMPLE_RATE)).round() as u64;
    if target <= capture.rendered {
        return;
    }
    let behind = target - capture.rendered;
    if behind > MOST_FRAMES {
        capture.rendered = target - MOST_FRAMES;
    }
    let frames = (target - capture.rendered) as usize;
    let samples = capture.render_frames(frames);
    let capture = &mut *capture;
    let (Some(wav), None) = (capture.wav.as_mut(), capture.failed.as_ref()) else {
        return;
    };
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in &samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    match wav.file.write_all(&bytes) {
        Ok(()) => wav.samples += samples.len() as u64,
        Err(error) => capture.failed = Some(format!("{}: {error}", wav.path.display())),
    }
}
