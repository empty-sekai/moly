//! Per-frame performance counters for the application.
//!
//! [`PerfPlugin`] records one [`FrameSample`] for every application update:
//! the Bevy frame delta, the wall time of the whole `Main` pass and of its
//! `PreUpdate`, `Update` and `PostUpdate` schedules, the wall time of the
//! render sub-app's extraction and of its `Render` schedule, the live entity
//! count and the number of `AssetEvent::Modified` messages of each tracked
//! asset type. The newest [`CAPACITY`] samples stay in a fixed ring, and
//! [`snapshot_json`] summarises them for a host page.
//!
//! The per-frame path only writes preallocated storage: it neither allocates
//! nor logs. Main-schedule times come from marker schedules placed around the
//! measured schedules in `MainScheduleOrder`; extraction is timed by wrapping
//! the render sub-app's extract function, and `Render` by running it from a
//! timing root schedule. The counters observe the frame; they never change
//! what a schedule runs or in which order.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bevy::app::MainScheduleOrder;
use bevy::asset::{Asset, AssetEvent};
use bevy::diagnostic::FrameCount;
use bevy::ecs::archetype::Archetypes;
use bevy::ecs::message::{MessageCursor, Messages};
use bevy::ecs::schedule::{ExecutorKind, InternedScheduleLabel, ScheduleLabel};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp};

/// Number of frames kept in the ring (about ten seconds at 60 fps).
pub const CAPACITY: usize = 600;
/// Upper bound of asset types one plugin can track.
pub const MAX_TRACKED_ASSETS: usize = 16;
/// Frames listed individually by a snapshot when the caller names no count.
pub const DEFAULT_RECENT: usize = 120;
/// A duration that was not measured for this frame (no render sub-app, or the
/// frame's render has not finished yet).
const UNKNOWN: u32 = u32::MAX;

/// One application update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSample {
    /// Bevy's `FrameCount` when the frame's `PreUpdate` began.
    pub frame: u32,
    /// `Time<Real>` delta of this update, microseconds.
    pub delta_us: u32,
    /// Whole `Main` pass (`First` through `Last`), microseconds.
    pub main_us: u32,
    pub pre_update_us: u32,
    pub update_us: u32,
    pub post_update_us: u32,
    /// Render sub-app extraction, including `ExtractSchedule`.
    pub extract_us: u32,
    /// The `Render` schedule that consumed this frame's extraction.
    pub render_us: u32,
    /// Spawned entities in the main world at the end of the frame.
    pub entities: u32,
    /// `AssetEvent::Modified` messages per tracked asset type, in the order
    /// the types were given to [`PerfPlugin::track`].
    pub modified: [u32; MAX_TRACKED_ASSETS],
}

impl FrameSample {
    const EMPTY: Self = Self {
        frame: 0,
        delta_us: 0,
        main_us: 0,
        pre_update_us: 0,
        update_us: 0,
        post_update_us: 0,
        extract_us: UNKNOWN,
        render_us: UNKNOWN,
        entities: 0,
        modified: [0; MAX_TRACKED_ASSETS],
    };
}

/// Fixed ring of the newest samples, oldest overwritten first.
struct Ring {
    samples: [FrameSample; CAPACITY],
    next: usize,
    len: usize,
    /// Completed render runs already attributed to a sample.
    renders_seen: u32,
}

impl Ring {
    fn new() -> Self {
        Self {
            samples: [FrameSample::EMPTY; CAPACITY],
            next: 0,
            len: 0,
            renders_seen: 0,
        }
    }

    fn push(&mut self, sample: FrameSample) {
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % CAPACITY;
        self.len = (self.len + 1).min(CAPACITY);
    }

    fn newest_mut(&mut self) -> Option<&mut FrameSample> {
        (self.len > 0).then(|| &mut self.samples[(self.next + CAPACITY - 1) % CAPACITY])
    }

    /// Samples oldest first.
    fn ordered(&self) -> impl Iterator<Item = &FrameSample> {
        let start = (self.next + CAPACITY - self.len) % CAPACITY;
        (0..self.len).map(move |offset| &self.samples[(start + offset) % CAPACITY])
    }
}

/// State shared by the main world, the extract wrapper, the render timing
/// root (which runs on the render thread when rendering is pipelined) and the
/// host snapshot.
struct Shared {
    ring: Mutex<Ring>,
    assets: Box<[&'static str]>,
    /// Wall time of the most recent completed `Render` run.
    render_us: AtomicU32,
    /// Count of completed `Render` runs.
    renders: AtomicU32,
}

impl Shared {
    /// Called after each extraction. The render run that finished before this
    /// extraction consumed the previous frame's extraction, so its time
    /// belongs to the newest stored sample; then the current frame's sample
    /// is stored with its own extraction time.
    fn commit(&self, sample: Option<FrameSample>, extract_us: u32) {
        let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let renders = self.renders.load(Ordering::Acquire);
        if renders != ring.renders_seen {
            ring.renders_seen = renders;
            let render_us = self.render_us.load(Ordering::Relaxed);
            if let Some(newest) = ring.newest_mut() {
                if newest.render_us == UNKNOWN {
                    newest.render_us = render_us;
                }
            }
        }
        if let Some(mut sample) = sample {
            sample.extract_us = extract_us;
            ring.push(sample);
        }
    }

    fn push_without_render(&self, sample: FrameSample) {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sample);
    }
}

/// The most recently built plugin's counters, read by [`snapshot_json`].
static LATEST: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

/// Main-world handle of this app's counters, written by the frame markers.
#[derive(Resource, Clone)]
struct PerfStore(Arc<Shared>);

/// Render-world handle for the render timing root and the schedule it runs.
#[derive(Resource)]
struct RenderPerf {
    shared: Arc<Shared>,
    schedule: InternedScheduleLabel,
}

/// JSON summary of the counters of the most recently built [`PerfPlugin`]:
/// percentiles over the stored window plus the newest `recent` frames.
/// Returns `{"available":false}` before any plugin was built.
pub fn snapshot_json(recent: usize) -> String {
    let latest = LATEST.lock().unwrap_or_else(PoisonError::into_inner).clone();
    match latest {
        Some(shared) => snapshot(&shared, recent),
        None => "{\"available\":false}".to_owned(),
    }
}

/// Marker schedules placed at the boundaries of the measured main schedules.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PerfMark {
    MainStart,
    PreUpdateStart,
    PreUpdateEnd,
    UpdateStart,
    UpdateEnd,
    PostUpdateStart,
    PostUpdateEnd,
    MainEnd,
}

const MAIN_START: usize = 0;
const PRE_UPDATE_START: usize = 1;
const PRE_UPDATE_END: usize = 2;
const UPDATE_START: usize = 3;
const UPDATE_END: usize = 4;
const POST_UPDATE_START: usize = 5;
const POST_UPDATE_END: usize = 6;
const MARKS: usize = 7;

/// The render sub-app's update schedule; it runs and times the schedule the
/// render sub-app ran before (the engine's `Render`).
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PerfRenderRoot;

/// The frame being assembled in the main world.
#[derive(Resource)]
struct FrameClock {
    marks: [Option<Instant>; MARKS],
    frame: u32,
    delta_us: u32,
    modified: [u32; MAX_TRACKED_ASSETS],
    /// Main-world part of the finished frame, waiting for its extraction.
    ready: Option<FrameSample>,
    /// The render sub-app's extraction stores each frame; otherwise the end
    /// of the main pass does.
    stored_at_extract: bool,
}

type Register = fn(&mut App, usize);

/// Installs the counters. Add it after the default plugins, so the render
/// sub-app already exists; tracked asset types may be registered later.
#[derive(Default)]
pub struct PerfPlugin {
    tracked: Vec<(&'static str, Register)>,
}

impl PerfPlugin {
    /// Count `AssetEvent::Modified` messages of `A` under `label`.
    pub fn track<A: Asset>(mut self, label: &'static str) -> Self {
        assert!(
            self.tracked.len() < MAX_TRACKED_ASSETS,
            "moly-perf tracks at most {MAX_TRACKED_ASSETS} asset types"
        );
        self.tracked.push((label, count_modified::<A>));
        self
    }
}

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring::new()),
            assets: self.tracked.iter().map(|(label, _)| *label).collect(),
            render_us: AtomicU32::new(0),
            renders: AtomicU32::new(0),
        });
        *LATEST.lock().unwrap_or_else(PoisonError::into_inner) = Some(shared.clone());
        // One tiny system per marker: a multi-threaded executor would cost
        // far more than the system itself.
        for label in [
            PerfMark::MainStart,
            PerfMark::PreUpdateStart,
            PerfMark::PreUpdateEnd,
            PerfMark::UpdateStart,
            PerfMark::UpdateEnd,
            PerfMark::PostUpdateStart,
            PerfMark::PostUpdateEnd,
            PerfMark::MainEnd,
        ] {
            app.add_schedule(single_threaded(label));
        }
        app.insert_resource(PerfStore(shared.clone()))
            .insert_resource(FrameClock {
                marks: [None; MARKS],
                frame: 0,
                delta_us: 0,
                modified: [0; MAX_TRACKED_ASSETS],
                ready: None,
                stored_at_extract: false,
            })
            .add_systems(PerfMark::MainStart, mark_main_start)
            .add_systems(PerfMark::PreUpdateStart, mark_pre_update_start)
            .add_systems(PerfMark::PreUpdateEnd, mark::<PRE_UPDATE_END>)
            .add_systems(PerfMark::UpdateStart, mark::<UPDATE_START>)
            .add_systems(PerfMark::UpdateEnd, mark::<UPDATE_END>)
            .add_systems(PerfMark::PostUpdateStart, mark::<POST_UPDATE_START>)
            .add_systems(PerfMark::PostUpdateEnd, mark::<POST_UPDATE_END>)
            .add_systems(PerfMark::MainEnd, mark_main_end);
        for (slot, (_, register)) in self.tracked.iter().enumerate() {
            register(app, slot);
        }
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        let Some(mut extract) = render_app.take_extract() else {
            return;
        };
        let extract_shared = shared.clone();
        render_app.set_extract(move |main_world, render_world| {
            let start = Instant::now();
            extract(main_world, render_world);
            let extract_us = micros(start.elapsed());
            let ready = main_world
                .get_resource_mut::<FrameClock>()
                .and_then(|mut clock| clock.ready.take());
            extract_shared.commit(ready, extract_us);
        });
        let schedule = render_app
            .update_schedule
            .replace(PerfRenderRoot.intern())
            .unwrap_or_else(|| Render.intern());
        render_app
            .add_schedule(single_threaded(PerfRenderRoot))
            .insert_resource(RenderPerf { shared, schedule })
            .add_systems(PerfRenderRoot, run_render);
        app.world_mut().resource_mut::<FrameClock>().stored_at_extract = true;
    }

    /// Markers are placed once every plugin has built, so a schedule another
    /// plugin inserts right after `PreUpdate` (the state transitions) stays
    /// outside the `PreUpdate` measurement.
    fn finish(&self, app: &mut App) {
        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.labels.insert(0, PerfMark::MainStart.intern());
        order.insert_before(PreUpdate, PerfMark::PreUpdateStart);
        order.insert_after(PreUpdate, PerfMark::PreUpdateEnd);
        order.insert_before(Update, PerfMark::UpdateStart);
        order.insert_after(Update, PerfMark::UpdateEnd);
        order.insert_before(PostUpdate, PerfMark::PostUpdateStart);
        order.insert_after(PostUpdate, PerfMark::PostUpdateEnd);
        order.labels.push(PerfMark::MainEnd.intern());
    }
}

fn single_threaded(label: impl ScheduleLabel) -> Schedule {
    let mut schedule = Schedule::new(label);
    schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    schedule
}

fn micros(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(UNKNOWN - 1)
}

fn span(marks: &[Option<Instant>; MARKS], start: usize, end: Option<Instant>) -> u32 {
    match (marks[start], end) {
        (Some(start), Some(end)) => micros(end.saturating_duration_since(start)),
        _ => 0,
    }
}

fn mark_main_start(mut clock: ResMut<FrameClock>) {
    clock.marks = [None; MARKS];
    clock.marks[MAIN_START] = Some(Instant::now());
}

/// `Time` and `FrameCount` are current once `First` has run.
fn mark_pre_update_start(
    mut clock: ResMut<FrameClock>,
    time: Res<Time<Real>>,
    frames: Option<Res<FrameCount>>,
) {
    clock.marks[PRE_UPDATE_START] = Some(Instant::now());
    clock.delta_us = micros(time.delta());
    clock.frame = frames.map_or(0, |frames| frames.0);
}

fn mark<const INDEX: usize>(mut clock: ResMut<FrameClock>) {
    clock.marks[INDEX] = Some(Instant::now());
}

fn mark_main_end(
    mut clock: ResMut<FrameClock>,
    archetypes: &Archetypes,
    store: Res<PerfStore>,
) {
    let end = Some(Instant::now());
    let marks = clock.marks;
    let sample = FrameSample {
        frame: clock.frame,
        delta_us: clock.delta_us,
        main_us: span(&marks, MAIN_START, end),
        pre_update_us: span(&marks, PRE_UPDATE_START, marks[PRE_UPDATE_END]),
        update_us: span(&marks, UPDATE_START, marks[UPDATE_END]),
        post_update_us: span(&marks, POST_UPDATE_START, marks[POST_UPDATE_END]),
        extract_us: UNKNOWN,
        render_us: UNKNOWN,
        entities: archetypes.iter().map(|archetype| archetype.len()).sum(),
        modified: clock.modified,
    };
    clock.modified = [0; MAX_TRACKED_ASSETS];
    if clock.stored_at_extract {
        clock.ready = Some(sample);
    } else {
        store.0.push_without_render(sample);
    }
}

/// Runs in `Last`, after the asset systems of `PostUpdate` wrote this
/// frame's events.
fn count_modified<A: Asset>(app: &mut App, slot: usize) {
    app.add_systems(
        Last,
        move |messages: Option<Res<Messages<AssetEvent<A>>>>,
              mut cursor: Local<MessageCursor<AssetEvent<A>>>,
              mut clock: ResMut<FrameClock>| {
            let Some(messages) = messages else {
                return;
            };
            let modified = cursor
                .read(&messages)
                .filter(|event| matches!(event, AssetEvent::Modified { .. }))
                .count();
            let slot = &mut clock.modified[slot];
            *slot = slot.saturating_add(u32::try_from(modified).unwrap_or(u32::MAX));
        },
    );
}

fn run_render(world: &mut World) {
    let schedule = world
        .get_resource::<RenderPerf>()
        .expect("the render timing root is installed with its handle")
        .schedule;
    let start = Instant::now();
    world.run_schedule(schedule);
    let render_us = micros(start.elapsed());
    let perf = world.resource::<RenderPerf>();
    perf.shared.render_us.store(render_us, Ordering::Relaxed);
    perf.shared.renders.fetch_add(1, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// Nearest-rank percentiles of one duration series, microseconds.
struct Stats {
    count: usize,
    p50: u32,
    p90: u32,
    p99: u32,
    max: u32,
    mean: u64,
}

fn stats(values: &mut [u32]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = |q: usize| values[(values.len() * q).div_ceil(100).max(1) - 1];
    let sum: u64 = values.iter().map(|&value| u64::from(value)).sum();
    Some(Stats {
        count: values.len(),
        p50: rank(50),
        p90: rank(90),
        p99: rank(99),
        max: values[values.len() - 1],
        mean: sum / values.len() as u64,
    })
}

/// Microseconds as a JSON millisecond number with microsecond precision.
fn write_ms(out: &mut String, micros: u64) {
    let _ = write!(out, "{}.{:03}", micros / 1000, micros % 1000);
}

fn write_duration(out: &mut String, micros: u32) {
    if micros == UNKNOWN {
        out.push_str("null");
    } else {
        write_ms(out, u64::from(micros));
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            character if u32::from(character) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(character));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn write_stats(out: &mut String, name: &str, samples: &[FrameSample], field: fn(&FrameSample) -> u32) {
    let mut values: Vec<u32> = samples
        .iter()
        .map(field)
        .filter(|&value| value != UNKNOWN)
        .collect();
    write_string(out, name);
    out.push(':');
    match stats(&mut values) {
        None => out.push_str("null"),
        Some(stats) => {
            let _ = write!(out, "{{\"frames\":{},\"p50\":", stats.count);
            write_ms(out, u64::from(stats.p50));
            out.push_str(",\"p90\":");
            write_ms(out, u64::from(stats.p90));
            out.push_str(",\"p99\":");
            write_ms(out, u64::from(stats.p99));
            out.push_str(",\"max\":");
            write_ms(out, u64::from(stats.max));
            out.push_str(",\"mean\":");
            write_ms(out, stats.mean);
            out.push('}');
        }
    }
}

/// Main pass plus extraction plus render, when all three are known.
fn work_us(sample: &FrameSample) -> u32 {
    if sample.extract_us == UNKNOWN || sample.render_us == UNKNOWN {
        return UNKNOWN;
    }
    sample
        .main_us
        .saturating_add(sample.extract_us)
        .saturating_add(sample.render_us)
        .min(UNKNOWN - 1)
}

fn snapshot(shared: &Shared, recent: usize) -> String {
    let samples: Vec<FrameSample> = {
        let ring = shared.ring.lock().unwrap_or_else(PoisonError::into_inner);
        ring.ordered().copied().collect()
    };
    let tracked = shared.assets.len();
    let mut out = String::with_capacity(4096 + recent.min(CAPACITY) * 160);
    let _ = write!(
        out,
        "{{\"available\":true,\"capacity\":{CAPACITY},\"frames\":{},\"assets\":[",
        samples.len()
    );
    for (index, label) in shared.assets.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_string(&mut out, label);
    }
    out.push_str("],\"summary\":{");
    let series: [(&str, fn(&FrameSample) -> u32); 8] = [
        ("deltaMs", |sample| sample.delta_us),
        ("mainMs", |sample| sample.main_us),
        ("preUpdateMs", |sample| sample.pre_update_us),
        ("updateMs", |sample| sample.update_us),
        ("postUpdateMs", |sample| sample.post_update_us),
        ("extractMs", |sample| sample.extract_us),
        ("renderMs", |sample| sample.render_us),
        ("workMs", work_us),
    ];
    for (index, (name, field)) in series.into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_stats(&mut out, name, &samples, field);
    }
    let entities = samples.iter().map(|sample| sample.entities);
    let _ = write!(
        out,
        ",\"entities\":{{\"last\":{},\"min\":{},\"max\":{}}}",
        samples.last().map_or(0, |sample| sample.entities),
        entities.clone().min().unwrap_or(0),
        entities.max().unwrap_or(0)
    );
    out.push_str(",\"modified\":{");
    for (slot, label) in shared.assets.iter().enumerate() {
        if slot > 0 {
            out.push(',');
        }
        let counts = samples.iter().map(|sample| sample.modified[slot]);
        let total: u64 = counts.clone().map(u64::from).sum();
        let frames = counts.clone().filter(|&count| count > 0).count();
        let max = counts.max().unwrap_or(0);
        write_string(&mut out, label);
        let _ = write!(
            out,
            ":{{\"total\":{total},\"framesWithAny\":{frames},\"maxPerFrame\":{max}}}"
        );
    }
    out.push_str("}},\"recent\":[");
    let skip = samples.len().saturating_sub(recent);
    for (index, sample) in samples[skip..].iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"frame\":{},\"deltaMs\":", sample.frame);
        write_duration(&mut out, sample.delta_us);
        out.push_str(",\"mainMs\":");
        write_duration(&mut out, sample.main_us);
        out.push_str(",\"preUpdateMs\":");
        write_duration(&mut out, sample.pre_update_us);
        out.push_str(",\"updateMs\":");
        write_duration(&mut out, sample.update_us);
        out.push_str(",\"postUpdateMs\":");
        write_duration(&mut out, sample.post_update_us);
        out.push_str(",\"extractMs\":");
        write_duration(&mut out, sample.extract_us);
        out.push_str(",\"renderMs\":");
        write_duration(&mut out, sample.render_us);
        let _ = write!(out, ",\"entities\":{},\"modified\":[", sample.entities);
        for (slot, count) in sample.modified[..tracked].iter().enumerate() {
            if slot > 0 {
                out.push(',');
            }
            let _ = write!(out, "{count}");
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}
