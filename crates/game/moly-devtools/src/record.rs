//! Recordings: the window's frames, encoded by ffmpeg and joined with the
//! game's mix into an MP4.
//!
//! `moly.record.start {path, fps, encoder, quality}` sets the game clock's
//! update strategy to a fixed step of `1/fps` seconds, and every recorded
//! frame sets it again. Each frame spawns one `Screenshot::primary_window()`
//! with its number; the screenshot holds the finished frame (the engine draws
//! the whole frame into the capture texture and copies it to the window
//! afterwards). The observer hands the pixels, as they were read back,
//! through a bounded channel of 8 frames to the writing thread, which feeds
//! the standard input of
//!
//! ```text
//! ffmpeg -f rawvideo -pix_fmt <rgba|bgra> -s WxH -r fps -i - -c:v <encoder> … <path>.video.mp4
//! ```
//!
//! The encoder starts with the first frame, at that frame's size and pixel
//! format. A full channel blocks the frame, so every frame reaches the
//! encoder, in order. With the frame-paced mix on (`dev.record_audio`) the
//! same frames' sound is written to `<path>.audio.wav`.
//!
//! `moly.record.stop` stops capturing and closes the sound at once. The
//! frames still being read back reach the encoder over the next frames; then
//! the clock gets its strategy back and the video and sound are joined:
//! `ffmpeg -i <video> -i <wav> -c:v copy -c:a aac <path>`. The parts are
//! removed once the join has succeeded. `moly.record.status` reads the
//! recording in progress, or the outcome of the last one.
//!
//! Only native runs record: the browser build has no ffmpeg and no files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::PrimaryWindow;
use bevy_remote::{error_codes, BrpError, BrpResult, RemoteMethodSystemId, RemoteMethods};
use serde_json::{json, Map, Value};

use crate::audio_capture::AudioCapture;

/// The frames a recording may run ahead of the encoder before the frame
/// blocks: the channel's capacity.
const BACKLOG: usize = 8;

/// Frames a stopping recording waits for its last screenshots before it
/// gives up on them.
const DRAIN_FRAMES: u32 = 30;

/// The encoders a request may name.
const ENCODERS: &[&str] = &["h264_amf", "hevc_amf", "h264_nvenc", "hevc_nvenc", "h264_qsv", "hevc_qsv", "libx264", "libx265"];

/// The encoder a request names when it names none.
const DEFAULT_ENCODER: &str = "h264_amf";

/// The frames per second a request names when it names none.
const DEFAULT_FPS: f64 = 60.0;

/// The pixel formats ffmpeg reads a frame in, by the order of the texture's
/// bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pixels {
    Rgba,
    Bgra,
}

impl Pixels {
    fn name(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::Bgra => "bgra",
        }
    }

    fn of_format(format: TextureFormat) -> Option<Self> {
        match format {
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some(Self::Rgba),
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => Some(Self::Bgra),
            _ => None,
        }
    }
}

/// One frame, as the encoder reads it.
struct Frame {
    number: u64,
    width: u32,
    height: u32,
    pixels: Pixels,
    bytes: Vec<u8>,
}

/// What the encoder is started with.
struct Encoding {
    program: String,
    output: PathBuf,
    fps: f64,
    encoder: String,
    quality: Option<u32>,
}

/// The encoder process and the thread that feeds it.
struct Writer {
    frames: SyncSender<Frame>,
    written: Arc<AtomicU64>,
    failed: Arc<Mutex<Option<String>>>,
    thread: std::thread::JoinHandle<()>,
}

impl Writer {
    /// Starts the program and the writing thread for frames of `width` by
    /// `height` in `pixels`.
    fn spawn(encoding: &Encoding, width: u32, height: u32, pixels: Pixels) -> Result<Self, String> {
        use std::process::{Command, Stdio};
        let program = &encoding.program;
        let mut command = Command::new(program);
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", pixels.name()])
            // The window presents full-range sRGB.
            .args(["-color_range", "pc", "-color_primaries", "bt709", "-color_trc", "iec61966-2-1", "-colorspace", "rgb"])
            .args(["-s", &format!("{width}x{height}"), "-r", &rate_text(encoding.fps), "-i", "-"])
            // Converted with the matrix and range the video is tagged with.
            .args(["-vf", "scale=out_color_matrix=bt709:out_range=tv", "-pix_fmt", "yuv420p"])
            .args(["-color_range", "tv", "-color_primaries", "bt709", "-colorspace", "bt709", "-color_trc", "bt709"])
            .args(["-c:v", &encoding.encoder]);
        if let Some(quality) = encoding.quality {
            let level = quality.to_string();
            match encoding.encoder.as_str() {
                "h264_amf" | "hevc_amf" => {
                    command.args(["-rc", "qvbr", "-qvbr_quality_level", &level, "-quality", "quality"]);
                }
                "libx264" | "libx265" => {
                    command.args(["-preset", "slow", "-crf", &level]);
                }
                "h264_qsv" | "hevc_qsv" => {
                    command.args(["-global_quality", &level]);
                }
                _ => {
                    command.args(["-rc", "vbr", "-cq", &level]);
                }
            }
        }
        command.arg(&encoding.output).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| format!("{program} cannot be started: {error}"))?;
        let stdin = child.stdin.take().ok_or_else(|| format!("{program} has no standard input"))?;
        let (frames, queue) = std::sync::mpsc::sync_channel::<Frame>(BACKLOG);
        let written = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(Mutex::new(None));
        let (thread_written, thread_failed) = (written.clone(), failed.clone());
        let thread = std::thread::Builder::new()
            .name("record-video".into())
            .spawn(move || write_frames(queue, stdin, child, (width, height, pixels), &thread_written, &thread_failed))
            .map_err(|error| format!("the frame writer cannot start: {error}"))?;
        Ok(Self { frames, written, failed, thread })
    }

    fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    /// Ends the frames, waits for the program, and answers the frames it took
    /// and the reason it failed, if it did.
    fn finish(self) -> (u64, Option<String>) {
        drop(self.frames);
        let joined = self.thread.join();
        let mut failure = self.failed.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if joined.is_err() && failure.is_none() {
            failure = Some("the frame writer stopped unexpectedly".to_owned());
        }
        (self.written.load(Ordering::Relaxed), failure)
    }
}

/// The writing thread: the frames, then the end of the program's input, then
/// its exit status.
fn write_frames(
    queue: std::sync::mpsc::Receiver<Frame>,
    input: std::process::ChildStdin,
    child: std::process::Child,
    (width, height, pixels): (u32, u32, Pixels),
    written: &AtomicU64,
    failed: &Mutex<Option<String>>,
) {
    use std::io::Write as _;
    let mut input = input;
    for frame in queue {
        let expected = written.load(Ordering::Relaxed) + 1;
        let refusal = if frame.number != expected {
            Some(format!("frame {} arrived where frame {expected} was expected", frame.number))
        } else if (frame.width, frame.height) != (width, height) {
            Some(format!(
                "frame {} is {}x{} and the video is {width}x{height}: the window changed size",
                frame.number, frame.width, frame.height
            ))
        } else if frame.pixels != pixels {
            Some(format!("frame {} is {} and the video reads {}", frame.number, frame.pixels.name(), pixels.name()))
        } else {
            input.write_all(&frame.bytes).err().map(|error| format!("the encoder stopped taking frame {}: {error}", frame.number))
        };
        if let Some(refusal) = refusal {
            note_failure(failed, refusal);
            break;
        }
        written.fetch_add(1, Ordering::Relaxed);
    }
    // The end of the frames is the end of the encoder's input.
    drop(input);
    match child.wait_with_output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => note_failure(
            failed,
            format!("the encoder failed ({}): {}", output.status, tail(&String::from_utf8_lossy(&output.stderr), 600)),
        ),
        Err(error) => note_failure(failed, format!("the encoder cannot be waited for: {error}")),
    }
}

/// Keeps the first reason a recording failed.
fn note_failure(failed: &Mutex<Option<String>>, reason: String) {
    failed.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(reason);
}

/// The last part of a program's report, cut at a character boundary.
fn tail(text: &str, most: usize) -> String {
    let text = text.trim();
    if text.len() <= most {
        return text.to_owned();
    }
    let mut start = text.len() - most;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

/// A rate as ffmpeg takes it: no exponent, at most three decimals.
fn rate_text(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The recording in progress.
struct Recording {
    path: PathBuf,
    video: PathBuf,
    audio: Option<PathBuf>,
    fps: f64,
    encoding: Encoding,
    /// The window's size in physical pixels when the recording started.
    window: (u32, u32),
    /// Started with the first frame.
    writer: Option<Writer>,
    /// Screenshots spawned so far; the next one is numbered `captured + 1`.
    captured: u64,
    /// Frames handed to the writer.
    delivered: u64,
    /// The game clock's update strategy before the recording started.
    previous: TimeUpdateStrategy,
    /// Frames left to wait for the last screenshots, once stopping.
    draining: Option<u32>,
    /// The first reason the recording failed.
    failure: Option<String>,
}

impl Recording {
    fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            error!("[record] {}: {reason}", self.path.display());
            self.failure = Some(reason);
        }
    }

    /// Hands one read-back frame to the writer, starting the writer with the
    /// first frame.
    fn deliver(&mut self, frame: Frame) {
        if self.failure.is_some() {
            return;
        }
        if self.writer.is_none() {
            match Writer::spawn(&self.encoding, frame.width, frame.height, frame.pixels) {
                Ok(writer) => self.writer = Some(writer),
                Err(error) => return self.fail(error),
            }
        }
        let number = frame.number;
        let sent = self.writer.as_ref().is_some_and(|writer| writer.frames.send(frame).is_ok());
        if sent {
            self.delivered += 1;
        } else {
            self.fail(format!("the encoder stopped before frame {number}"));
        }
    }

    fn status(&self) -> Value {
        json!({
            "recording": true,
            "stopping": self.draining.is_some(),
            "path": self.path,
            "fps": self.fps,
            "encoder": self.encoding.encoder,
            "window": [self.window.0, self.window.1],
            "captured": self.captured,
            "delivered": self.delivered,
            "frames": self.writer.as_ref().map_or(0, Writer::written),
            "failure": self.failure,
        })
    }
}

/// The recorder: the recording in progress, and the outcome of the last one.
#[derive(Resource, Default)]
struct Recorder {
    active: Option<Recording>,
    last: Option<Value>,
}

/// The methods are added to the remote protocol's table at `Startup`, once
/// the remote plugin has built it. The frame system runs in `Last`: a frame
/// is captured after it is drawn, and the clock's strategy is set for the
/// frames after it.
pub fn install(app: &mut App) {
    app.init_resource::<Recorder>().add_systems(Startup, register_methods).add_systems(Last, drive);
}

fn register_methods(world: &mut World) {
    if !world.contains_resource::<RemoteMethods>() {
        return;
    }
    let start = world.register_system(start);
    let stop = world.register_system(stop);
    let status = world.register_system(status);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("moly.record.start", RemoteMethodSystemId::Instant(start));
    methods.insert("moly.record.stop", RemoteMethodSystemId::Instant(stop));
    methods.insert("moly.record.status", RemoteMethodSystemId::Instant(status));
}

fn refuse(message: impl Into<String>) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message: message.into(), data: None }
}

fn refusal(message: impl Into<String>) -> BrpError {
    BrpError { code: error_codes::RESOURCE_NOT_PRESENT, message: message.into(), data: None }
}

/// The params as an object; none reads as an empty one. A key the method
/// does not take is refused rather than ignored.
fn object(params: Option<Value>, method: &str, takes: &[&str]) -> Result<Map<String, Value>, BrpError> {
    let params = match params {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(refuse(format!("{method}: params must be an object"))),
    };
    if let Some(key) = params.keys().find(|key| !takes.contains(&key.as_str())) {
        let takes = if takes.is_empty() { "no parameters".to_owned() } else { takes.join(", ") };
        return Err(refuse(format!("{method}: unknown parameter \"{key}\"; it takes {takes}")));
    }
    Ok(params)
}

/// The program a recording is encoded with: `dev.ffmpeg` (`MOLY_FFMPEG`),
/// else `ffmpeg` found on `PATH`.
fn ffmpeg() -> String {
    match moly_config::text(&moly_config::keys::DEV_FFMPEG) {
        Some(program) if !program.trim().is_empty() => program,
        _ => "ffmpeg".to_owned(),
    }
}

/// The path a recording writes to, made absolute; without an extension it
/// takes `.mp4`.
fn record_path(params: &Map<String, Value>) -> Result<PathBuf, BrpError> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| refuse("moly.record.start: path must be a file name or path"))?;
    let mut path = PathBuf::from(path);
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|error| refusal(format!("moly.record.start: the working directory: {error}")))
}

/// The file beside a recording's: `take.mp4` gives `take.mp4.video.mp4` and
/// `take.mp4.audio.wav`.
fn beside(path: &Path, extension: &str) -> PathBuf {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".");
    beside.push(extension);
    PathBuf::from(beside)
}

/// Starts a recording from the frame the request arrives on.
fn start(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.record.start", &["path", "fps", "encoder", "quality"])?;
    let path = record_path(&params)?;
    let fps = match params.get("fps") {
        None => DEFAULT_FPS,
        Some(fps) => fps
            .as_f64()
            .filter(|fps| (1.0..=240.0).contains(fps))
            .ok_or_else(|| refuse("moly.record.start: fps must be a number from 1 to 240"))?,
    };
    let encoder = match params.get("encoder") {
        None => DEFAULT_ENCODER.to_owned(),
        Some(encoder) => encoder
            .as_str()
            .filter(|encoder| ENCODERS.contains(encoder))
            .ok_or_else(|| refuse(format!("moly.record.start: encoder must be one of {}", ENCODERS.join(", "))))?
            .to_owned(),
    };
    let quality = match params.get("quality") {
        None => None,
        Some(quality) => Some(
            quality
                .as_u64()
                .and_then(|quality| u32::try_from(quality).ok())
                .filter(|quality| *quality <= 51)
                .ok_or_else(|| refuse("moly.record.start: quality must be a whole number from 0 to 51"))?,
        ),
    };
    if let Some(active) = world.resource::<Recorder>().active.as_ref() {
        return Err(refusal(format!("moly.record.start: {} is being recorded; stop it first", active.path.display())));
    }
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map(|window| (window.physical_width(), window.physical_height()))
        .ok_or_else(|| refusal("moly.record.start: there is no window to record"))?;
    // The encoder stores 4:2:0 video, which takes even dimensions.
    if window.0 == 0 || window.1 == 0 || window.0 % 2 != 0 || window.1 % 2 != 0 {
        return Err(refusal(format!(
            "moly.record.start: the window is {}x{}; recordings take even widths and heights",
            window.0, window.1
        )));
    }
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| refusal(format!("moly.record.start: cannot create {}: {error}", parent.display())))?;
    }
    let video = beside(&path, "video.mp4");
    let audio = match world.get_resource_mut::<AudioCapture>() {
        Some(mut capture) => {
            let wav = beside(&path, "audio.wav");
            capture.start(&wav).map_err(|error| refusal(format!("moly.record.start: {error}")))?;
            Some(wav)
        }
        None => None,
    };
    let step = TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / fps));
    let previous = std::mem::replace(&mut *world.resource_mut::<TimeUpdateStrategy>(), step);
    let encoding = Encoding { program: ffmpeg(), output: video.clone(), fps, encoder, quality };
    let recording = Recording {
        path,
        video,
        audio,
        fps,
        encoding,
        window,
        writer: None,
        captured: 0,
        delivered: 0,
        previous,
        draining: None,
        failure: None,
    };
    info!("[record] {} at {fps} fps, {}x{}, {}", recording.path.display(), window.0, window.1, recording.encoding.encoder);
    let mut answer = recording.status();
    if let Some(audio) = recording.audio.as_ref() {
        answer["audio"] = json!(audio);
    }
    world.resource_mut::<Recorder>().active = Some(recording);
    Ok(answer)
}

fn status(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    object(params, "moly.record.status", &[])?;
    let recorder = world.resource::<Recorder>();
    Ok(match (recorder.active.as_ref(), recorder.last.as_ref()) {
        (Some(active), _) => active.status(),
        (None, Some(last)) => json!({ "recording": false, "last": last }),
        (None, None) => json!({ "recording": false }),
    })
}

/// Stops capturing and closes the sound; the recording finishes once its
/// last frames have reached the encoder (see `moly.record.status`).
fn stop(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    object(params, "moly.record.stop", &[])?;
    let mut sound = None;
    if world.resource::<Recorder>().active.as_ref().is_some_and(|active| active.audio.is_some()) {
        sound = world.get_resource_mut::<AudioCapture>().map(|mut capture| capture.stop());
    }
    let mut recorder = world.resource_mut::<Recorder>();
    let Some(active) = recorder.active.as_mut().filter(|active| active.draining.is_none()) else {
        return Err(refusal("moly.record.stop: nothing is being recorded"));
    };
    match sound {
        Some(Err(error)) => active.fail(error),
        Some(Ok(None)) => active.fail("the sound of the recording was not open".to_owned()),
        Some(Ok(Some(_))) | None => {}
    }
    active.draining = Some(DRAIN_FRAMES);
    Ok(active.status())
}

/// Ends a recording: the video's frames end and the encoder is waited for,
/// the clock gets its strategy back, and the video and sound are joined.
/// A step that failed leaves the parts beside the recording.
fn finish(mut recording: Recording, world: &mut World) -> Value {
    let frames = match recording.writer.take() {
        Some(writer) => {
            let (frames, failure) = writer.finish();
            if let Some(failure) = failure {
                recording.fail(failure);
            }
            frames
        }
        None => 0,
    };
    if frames < recording.captured {
        recording.fail(format!("{} of {} frames reached the encoder", frames, recording.captured));
    }
    *world.resource_mut::<TimeUpdateStrategy>() = std::mem::replace(&mut recording.previous, TimeUpdateStrategy::Automatic);
    // A recording that failed with its sound open closes the sound here.
    if let Some(mut capture) = world.get_resource_mut::<AudioCapture>() {
        if let Err(error) = capture.stop() {
            recording.fail(error);
        }
    }
    if recording.failure.is_none() {
        match join(&recording.video, recording.audio.as_deref(), &recording.path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&recording.video);
                if let Some(audio) = recording.audio.as_deref() {
                    let _ = std::fs::remove_file(audio);
                }
                info!("[record] {}: {frames} frames", recording.path.display());
            }
            Err(error) => recording.fail(error),
        }
    }
    let mut answer = json!({ "path": recording.path, "frames": frames, "seconds": frames as f64 / recording.fps });
    if let Some(failure) = recording.failure {
        answer["failed"] = json!(failure);
    }
    answer
}

/// Joins the video and its sound into the recording's file: the video is
/// copied, the sound encoded to AAC. Success is the program's exit status
/// and the file's presence.
fn join(video: &Path, sound: Option<&Path>, out: &Path) -> Result<(), String> {
    let program = ffmpeg();
    let mut command = std::process::Command::new(&program);
    command.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(video);
    match sound {
        Some(sound) => {
            command.arg("-i").arg(sound).args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k"]);
        }
        None => {
            command.args(["-map", "0:v:0", "-c:v", "copy"]);
        }
    }
    command.args(["-movflags", "+faststart"]).arg(out);
    let output = command.output().map_err(|error| format!("{program} cannot be started: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} could not join the video and its sound ({}): {}",
            output.status,
            tail(&String::from_utf8_lossy(&output.stderr), 600)
        ));
    }
    if !out.exists() {
        return Err(format!("{program} ended without writing {}", out.display()));
    }
    Ok(())
}

/// `Last`: one screenshot per frame while recording; once stopping, the wait
/// for the frames still being read back, then the end.
fn drive(world: &mut World) {
    world.resource_scope(|world, mut recorder: Mut<Recorder>| drive_recorder(world, &mut recorder));
}

fn drive_recorder(world: &mut World, recorder: &mut Recorder) {
    let Some(active) = recorder.active.as_mut() else {
        return;
    };
    let done = match active.draining.as_mut() {
        Some(left) => {
            *left = left.saturating_sub(1);
            active.failure.is_some() || active.delivered >= active.captured || *left == 0
        }
        None if active.failure.is_some() => true,
        None => {
            active.captured += 1;
            let number = active.captured;
            let step = Duration::from_secs_f64(1.0 / active.fps);
            // Set on every frame, over anything that set another strategy.
            *world.resource_mut::<TimeUpdateStrategy>() = TimeUpdateStrategy::ManualDuration(step);
            world.spawn(Screenshot::primary_window()).observe(move |captured: On<ScreenshotCaptured>, mut recorder: ResMut<Recorder>| {
                let Some(active) = recorder.active.as_mut() else {
                    return;
                };
                let image = &captured.image;
                let Some(pixels) = Pixels::of_format(image.texture_descriptor.format) else {
                    return active.fail(format!("frame {number} is {:?}; the encoder reads RGBA or BGRA", image.texture_descriptor.format));
                };
                let Some(bytes) = image.data.clone() else {
                    return active.fail(format!("frame {number} has no pixels"));
                };
                let size = image.texture_descriptor.size;
                active.deliver(Frame { number, width: size.width, height: size.height, pixels, bytes });
            });
            false
        }
    };
    if done {
        let recording = recorder.active.take().expect("the recording was just read");
        let outcome = finish(recording, world);
        if outcome.get("failed").is_some() {
            warn!("[record] {outcome}");
        }
        recorder.last = Some(outcome);
    }
}
