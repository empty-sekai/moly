//! Frame time of the player: Time.deltaTime, Time.unscaledDeltaTime and the
//! slices one particle system's frame is simulated in.
//!
//! The player's TimeManager never changes at run time (the game assembly calls
//! only Time getters) and its time scale is 1, so every law here is written for
//! time scale 1. The TimeManager values themselves live in
//! [`TimeManagerSnapshot`].
use super::prewarm::TimeManagerSnapshot;
use core::time::Duration;

/// TimeManager::Update's minimum frame and Update1b's minimum particle step.
const MINIMUM_STEP: f32 = f32::from_bits(0x3727_c5ac);
/// Update1Incremental keeps slicing while at least this much time is pending.
const INCREMENTAL_EPSILON: f32 = f32::from_bits(0x3586_37bd);
/// Longest slice schedule one call may produce before it is refused.
const SLICE_BUDGET: usize = 1_000_256;

/// Source Time.deltaTime for a frame whose real duration is `elapsed` seconds.
/// TimeManager::Update: a frame longer than Time.maximumDeltaTime advances game
/// time by exactly that maximum and the rest is dropped, never caught up; a
/// frame shorter than 1e-5 s advances it by 1e-5 s; any other frame by its
/// elapsed time rounded once to f32. Both comparisons are strict and made in
/// double precision against the widened float limits. A NaN falls through to
/// the last arm and stays NaN, as in the engine.
pub fn source_delta_seconds(elapsed: f64, maximum_delta_time: f32) -> f32 {
    if elapsed > f64::from(maximum_delta_time) {
        maximum_delta_time
    } else if elapsed < f64::from(MINIMUM_STEP) {
        MINIMUM_STEP
    } else {
        elapsed as f32
    }
}

/// [`source_delta_seconds`] for a real frame duration. The duration is
/// converted to seconds in double precision and rounded to f32 once, as the
/// engine rounds its double-precision clock difference once.
pub fn source_delta_time(elapsed: Duration, time: TimeManagerSnapshot) -> f32 {
    source_delta_seconds(elapsed.as_secs_f64(), time.maximum_delta_time)
}

/// One frame of Time.unscaledDeltaTime: the value and whether the frame
/// consumed the real time accumulated since the last consuming frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnscaledDelta {
    pub value: f32,
    pub consumed: bool,
}

/// Source Time.unscaledDeltaTime for `accumulated` seconds of real time since
/// the last frame that consumed its time. TimeManager::Update does not clamp
/// this delta by Time.maximumDeltaTime. A value below 1e-5 s reports 1e-5 s and
/// leaves the reference where it was, so that short frame's time carries into
/// the next one.
pub fn unscaled_delta_seconds(accumulated: f64) -> UnscaledDelta {
    let value = accumulated as f32;
    if value < MINIMUM_STEP {
        UnscaledDelta {
            value: MINIMUM_STEP,
            consumed: false,
        }
    } else {
        UnscaledDelta {
            value,
            consumed: true,
        }
    }
}

/// The engine keeps one Time.unscaledDeltaTime holder for the whole player; this
/// is that holder over an exact real clock, advanced once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnscaledClock {
    pending: Duration,
}

impl UnscaledClock {
    pub const fn new() -> Self {
        Self {
            pending: Duration::ZERO,
        }
    }

    /// Advance by one frame of `real` duration and return its unscaled delta.
    pub fn advance(&mut self, real: Duration) -> f32 {
        let total = self.pending.saturating_add(real);
        let delta = unscaled_delta_seconds(total.as_secs_f64());
        self.pending = if delta.consumed {
            Duration::ZERO
        } else {
            total
        };
        delta.value
    }
}

/// Update1b's `FMAX(simulationSpeed, 0)`: a NaN stays NaN, anything not above
/// +0 (negative zero included) becomes +0.
pub fn fmax_zero(speed: f32) -> f32 {
    if speed.is_nan() || speed > 0.0 {
        speed
    } else {
        0.0
    }
}

/// GetTimeStep with UpdateData flags bit 0 clear, the player's per-frame and
/// explicit-dt value: a scaled frame longer than the maximum particle step is
/// cut into equal pieces, `dt / FRINTP(dt / max)`, each an f32 operation; any
/// other value, a NaN included, is returned as it is.
pub fn time_step(scaled: f32, maximum_particle_timestep: f32) -> f32 {
    if maximum_particle_timestep < scaled {
        let step = scaled / (scaled / maximum_particle_timestep).ceil();
        // An infinite dt divides infinity by infinity: the ARM result is the
        // default NaN (positive quiet), not the host's.
        if step.is_nan() { f32::from_bits(0x7fc0_0000) } else { step }
    } else {
        scaled
    }
}

/// How the incremental update was entered. The per-frame update passes
/// UpdateData flags 0 and Play's explicit-dt update passes 8; the Update1b,
/// GetTimeStep and Update1Incremental bodies test only bits 0, 1 and 2, which
/// are clear in both, so both run the same slice loop. `ParticleSystem.Simulate`
/// from script passes flags 4 to its time update (bit 2 set; bit 1 as well on
/// a restart, whose time update is zero and skips): Update1Incremental then
/// takes every slice as `min(pending, step)`, without the 5 s / 10 s backlog
/// widening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncrementalEntry {
    PerFrame,
    ExplicitDt,
    ScriptSimulate,
}

#[derive(Clone, Copy, Debug)]
pub struct Slice {
    pub remaining_before: f32,
    pub duration: f32,
}

impl Slice {
    /// StartParticles' backtrack argument for the time births of this slice:
    /// FMAXNM((remaining / slice) + -1, 0), the slices still to come in the
    /// frame (fractional when a remainder below the loop threshold is carried).
    pub fn births_ahead(self) -> f32 {
        // f32::max returns the non-NaN operand, as FMAXNM does.
        ((self.remaining_before / self.duration) + -1.0).max(0.0)
    }
}

/// ParticleSystem::Update1Incremental's slice loop over the pending time.
pub struct IncrementalSlices {
    remaining: f32,
    base_step: f32,
    prior_step: f32,
    duration: f32,
    /// UpdateData flags bit 2 clear: the backlog widening applies.
    widen: bool,
    budget: usize,
}

impl IncrementalSlices {
    /// `total` is the pending time after Update1b added the scaled frame and
    /// `step` the GetTimeStep result. Inputs outside a bounded finite schedule
    /// are refused, never clamped.
    pub fn new(
        total: f32,
        step: f32,
        entry: IncrementalEntry,
        duration: f32,
    ) -> Result<Self, &'static str> {
        if !(total.is_finite()
            && total > 0.0
            && step.is_finite()
            && step > 0.0
            && duration.is_finite()
            && duration > 0.0)
        {
            return Err("unqualified incremental input");
        }
        if !(total / step).is_finite() || total / step > 1_000_000.0 {
            return Err("incremental slice budget outside bounded replay");
        }
        Ok(Self {
            remaining: total,
            base_step: step,
            prior_step: step,
            duration,
            widen: entry != IncrementalEntry::ScriptSimulate,
            budget: SLICE_BUDGET,
        })
    }

    /// Time still pending: below 1e-6 s once the loop has ended, carried into
    /// the next update.
    pub fn remaining(&self) -> f32 {
        self.remaining
    }
    pub fn base_step(&self) -> f32 {
        self.base_step
    }
}

impl Iterator for IncrementalSlices {
    type Item = Result<Slice, &'static str>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining < INCREMENTAL_EPSILON {
            return None;
        }
        if self.budget == 0 {
            self.remaining = 0.0;
            return Some(Err("incremental budget exhausted"));
        }
        let before = self.remaining;
        let mut step = before.min(self.base_step);
        // With flags bit 2 clear the selected slice is kept when it already
        // exceeds the 1 s (above 10 s pending) or 0.2 s (above 5 s pending)
        // threshold; otherwise the duration capped at that threshold is taken.
        // The two caps are IEEE minNum, which keeps the threshold for a NaN
        // duration. With bit 2 set neither test is made.
        if self.widen && before > 10.0 {
            step = if self.prior_step > 1.0 {
                self.prior_step
            } else {
                self.duration.min(1.0)
            };
        } else if self.widen && before > 5.0 {
            step = if self.prior_step > 0.2 {
                self.prior_step
            } else {
                self.duration.min(0.2)
            };
        }
        if !(step.is_finite() && step > 0.0 && step <= before) || before - step == before {
            self.budget = 0;
            self.remaining = 0.0;
            return Some(Err("incremental slice does not advance"));
        }
        self.prior_step = step;
        self.remaining = before - step;
        self.budget -= 1;
        Some(Ok(Slice {
            remaining_before: before,
            duration: step,
        }))
    }
}

/// What one frame of a playing system does.
pub enum FrameStep {
    /// GetTimeStep gave less than 1e-5 s, or NaN: the whole update is skipped
    /// and the frame's time is not added to the pending time.
    Skipped,
    Slices(IncrementalSlices),
}

/// ParticleSystem::Update1b's frame part and the slices of
/// Update1Incremental. `pending` is the time left over from the system's
/// previous update, `dt` the frame's delta from the clock the system reads and
/// `duration` the main-module duration the backlog widening reads.
pub fn frame_step(
    pending: f32,
    dt: f32,
    simulation_speed: f32,
    time: TimeManagerSnapshot,
    duration: f32,
) -> Result<FrameStep, &'static str> {
    frame_step_entry(pending, dt, simulation_speed, time, duration, IncrementalEntry::PerFrame)
}

/// [`frame_step`] for an update entered as `entry` (the per-frame update, or
/// a script `Simulate` time update, which skips the backlog widening).
pub fn frame_step_entry(
    pending: f32,
    dt: f32,
    simulation_speed: f32,
    time: TimeManagerSnapshot,
    duration: f32,
    entry: IncrementalEntry,
) -> Result<FrameStep, &'static str> {
    let scaled = dt * fmax_zero(simulation_speed);
    let step = time_step(scaled, time.maximum_particle_timestep);
    if !(step >= MINIMUM_STEP) {
        return Ok(FrameStep::Skipped);
    }
    IncrementalSlices::new(pending + scaled, step, entry, duration).map(FrameStep::Slices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

    fn read(path: &std::path::Path) -> Value {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    fn hex32(v: &Value) -> u32 {
        u32::from_str_radix(v.as_str().expect("hex word"), 16).expect("hex word")
    }
    fn hex64(v: &Value) -> u64 {
        u64::from_str_radix(v.as_str().expect("hex word"), 16).expect("hex word")
    }
    fn int(v: &Value) -> u32 {
        let n = v.as_f64().expect("integer");
        assert!(n >= 0.0 && n <= u32::MAX as f64 && n.fract() == 0.0, "{n}");
        n as u32
    }
    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    /// TimeManager::CheckConsistency over the serialized words the receipt
    /// cases load: a NaN fixed step becomes 0, then it is held to [1e-4, 10];
    /// a NaN maximum becomes 0 and the maximum is raised to the fixed step.
    fn consistent_maximum(fixed: f32, maximum: f32) -> f32 {
        let fixed = if fixed.is_nan() { 0.0 } else { fixed };
        let fixed = if fixed < 1e-4 { 1e-4 } else { fixed.min(10.0) };
        let maximum = if maximum.is_nan() { 0.0 } else { maximum };
        if maximum < fixed {
            fixed
        } else {
            maximum
        }
    }

    #[test]
    #[ignore = "MOLY_TIMESTEP_RECEIPT must name the native TimeManager and GetTimeStep receipt"]
    fn frame_deltas_and_time_step_match_native_receipt() {
        let path = std::env::var_os("MOLY_TIMESTEP_RECEIPT").expect("MOLY_TIMESTEP_RECEIPT");
        let receipt = read(std::path::Path::new(&path));
        assert_eq!(
            receipt.get("sha256").and_then(Value::as_str),
            Some(SOURCE_SHA256)
        );
        let cases = receipt
            .get("updateCases")
            .and_then(Value::as_array)
            .expect("updateCases");
        let (mut scaled, mut scaled_miss) = (0usize, 0usize);
        let (mut unscaled, mut unscaled_miss, mut separating) = (0usize, 0usize, 0usize);
        for case in cases {
            let name = case.get("name").and_then(Value::as_str).unwrap_or("?");
            let words = case
                .get("serialized")
                .and_then(Value::as_str)
                .expect("serialized");
            assert_eq!(words.len(), 32, "{name}");
            let word = |i: usize| {
                let b = u32::from_str_radix(&words[i * 8..i * 8 + 8], 16).expect("word");
                f32::from_bits(b.swap_bytes())
            };
            let (fixed, maximum, time_scale) = (word(0), word(1), word(2));
            let start = case.get("start").and_then(Value::as_f64).expect("start");
            let capture_null = matches!(case.get("capture"), Some(Value::Null));
            let playing = case.get("worldPlaying").and_then(Value::as_bool) == Some(true);
            let frames = case
                .get("frames")
                .and_then(Value::as_array)
                .expect("frames");
            // Scaled delta, time scale 1: the frame's elapsed real time is the
            // difference of consecutive clock readings. The first two frames
            // after the reset and every pause frame take the engine's reset
            // literal instead. The case whose clock starts near 3e7 s is left
            // out: the engine forms elapsed through its zero offset there.
            let scaled_case =
                time_scale.to_bits() == 0x3f80_0000 && capture_null && playing && start < 1e6;
            let maximum = consistent_maximum(fixed, maximum);
            let mut previous = start;
            // Unscaled delta: the reference moves only on a consuming frame.
            let mut reference = start;
            for frame in frames {
                let index = int(frame.get("frame").expect("frame"));
                let now = f64::from_bits(hex64(frame.get("nowBits").expect("nowBits")));
                let getters = frame.get("getters").expect("getters");
                let elapsed = now - previous;
                previous = now;
                let paused = frame.get("paused").and_then(Value::as_bool) == Some(true);
                if scaled_case && index >= 2 && !paused {
                    let native =
                        f32::from_bits(hex32(getters.get("deltaTime").expect("deltaTime")));
                    let ours = source_delta_seconds(elapsed, maximum);
                    scaled += 1;
                    if !same(ours, native) {
                        scaled_miss += 1;
                        eprintln!(
                            "scaled {name} frame {index}: native {:08x} ours {:08x}",
                            native.to_bits(),
                            ours.to_bits()
                        );
                    }
                }
                let delta = unscaled_delta_seconds(now - reference);
                if delta.consumed {
                    reference = now;
                }
                // The getter reads the active holder, which the first frame
                // after a reset without capture does not refresh.
                if index == 0 && capture_null {
                    continue;
                }
                let native = f32::from_bits(hex32(
                    getters.get("unscaledDeltaTime").expect("unscaledDeltaTime"),
                ));
                unscaled += 1;
                if !same(delta.value, native) {
                    unscaled_miss += 1;
                    eprintln!(
                        "unscaled {name} frame {index}: native {:08x} ours {:08x}",
                        native.to_bits(),
                        delta.value.to_bits()
                    );
                }
                if maximum.to_bits() == 0x3eaa_aaab && delta.value > maximum {
                    separating += 1;
                }
            }
        }
        let rows = receipt
            .get("getTimeStep")
            .and_then(|g| g.get("rows"))
            .and_then(Value::as_array)
            .expect("getTimeStep rows");
        let (mut steps, mut step_miss) = (0usize, 0usize);
        for row in rows {
            // Flags bit 0 clear with the world playing: the player's per-frame
            // and explicit-dt call.
            if row.get("fixed").and_then(Value::as_f64) != Some(0.0)
                || row.get("world").and_then(Value::as_f64) != Some(1.0)
            {
                continue;
            }
            let dt = f32::from_bits(hex32(row.get("dt").expect("dt")));
            let maximum = f32::from_bits(hex32(row.get("maxp").expect("maxp")));
            let native = f32::from_bits(hex32(row.get("native").expect("native")));
            let ours = time_step(dt, maximum);
            steps += 1;
            if !same(ours, native) {
                step_miss += 1;
                eprintln!(
                    "time step dt {:08x} max {:08x}: native {:08x} ours {:08x}",
                    dt.to_bits(),
                    maximum.to_bits(),
                    native.to_bits(),
                    ours.to_bits()
                );
            }
        }
        println!("timestep receipt: scaled frames {scaled} mismatched {scaled_miss}; unscaled frames {unscaled} mismatched {unscaled_miss}, above the player maximum {separating}; GetTimeStep rows {steps} mismatched {step_miss}");
        assert!(
            scaled > 0 && unscaled > 0 && steps > 0,
            "receipt compared nothing"
        );
        // The frames where the unscaled clock runs past the player maximum are
        // the ones that separate it from the clamped clock; they must be there.
        assert!(
            separating > 0,
            "no frame separates the unscaled clock from the clamped one"
        );
        assert_eq!((scaled_miss, unscaled_miss, step_miss), (0, 0, 0));
    }

    #[test]
    #[ignore = "MOLY_DISTANCE_RECEIPT must name the native Update1b slicing receipt files"]
    fn frame_slices_match_native_update1b() {
        let paths = std::env::var_os("MOLY_DISTANCE_RECEIPT").expect("MOLY_DISTANCE_RECEIPT");
        let time = TimeManagerSnapshot {
            fixed_timestep: f32::from_bits(0x3ca3_d70a),
            maximum_particle_timestep: f32::from_bits(0x3cf5_c28f),
            maximum_delta_time: f32::from_bits(0x3eaa_aaab),
        };
        let (mut frames, mut slices, mut skipped, mut files) = (0usize, 0usize, 0usize, 0usize);
        for path in std::env::split_paths(&paths) {
            let receipt = read(&path);
            files += 1;
            assert_eq!(
                receipt.get("sourceSha256").and_then(Value::as_str),
                Some(SOURCE_SHA256),
                "{}",
                path.display()
            );
            for group in ["s1Cases", "s11Cases", "controls"] {
                let Some(cases) = receipt.get(group).and_then(Value::as_array) else {
                    continue;
                };
                for case in cases {
                    let name = case.get("name").and_then(Value::as_str).unwrap_or("?");
                    let config = case.get("config").expect("config");
                    // The receipt writes the NaN speed case as the string "nan".
                    let speed = match config.get("speed") {
                        Some(Value::Number(v)) => *v as f32,
                        Some(Value::Str(s)) if s == "nan" => f32::NAN,
                        other => panic!("{name}: speed {other:?}"),
                    };
                    let duration = config
                        .get("duration")
                        .and_then(Value::as_f64)
                        .expect("duration") as f32;
                    for (i, frame) in case
                        .get("frames")
                        .and_then(Value::as_array)
                        .expect("frames")
                        .iter()
                        .enumerate()
                    {
                        let bits = |v: Option<&Value>| f32::from_bits(int(v.expect("bits")));
                        let input = frame.get("input").expect("input");
                        let dt = bits(input.get("dtBits"));
                        let pending = bits(frame.get("before").and_then(|b| b.get("pendingBits")));
                        let native_step = bits(frame.get("stepBits"));
                        let after = bits(frame.get("after").and_then(|a| a.get("pendingBits")));
                        let native_slices =
                            frame.get("slices").and_then(Value::as_array).unwrap_or(&[]);
                        let incremental = frame
                            .get("incremental")
                            .filter(|v| !matches!(v, Value::Null));
                        let at = format!("{} {name} frame {i}", path.display());
                        assert!(
                            same(
                                time_step(dt * fmax_zero(speed), time.maximum_particle_timestep),
                                native_step
                            ),
                            "{at}: step"
                        );
                        frames += 1;
                        match frame_step(pending, dt, speed, time, duration)
                            .unwrap_or_else(|e| panic!("{at}: {e}"))
                        {
                            FrameStep::Skipped => {
                                assert!(
                                    incremental.is_none(),
                                    "{at}: native ran the incremental update"
                                );
                                assert!(
                                    native_slices.is_empty(),
                                    "{at}: native sliced a skipped frame"
                                );
                                assert_eq!(
                                    pending.to_bits(),
                                    after.to_bits(),
                                    "{at}: skipped frame changed the pending time"
                                );
                                skipped += 1;
                            }
                            FrameStep::Slices(mut ours) => {
                                let incremental = incremental
                                    .unwrap_or_else(|| panic!("{at}: native skipped the frame"));
                                assert_eq!(
                                    ours.remaining().to_bits(),
                                    int(incremental.get("pendingBits").expect("pendingBits")),
                                    "{at}: pending at entry"
                                );
                                assert_eq!(
                                    ours.base_step().to_bits(),
                                    int(incremental.get("stepBits").expect("stepBits")),
                                    "{at}: step argument"
                                );
                                for (k, pair) in native_slices.iter().enumerate() {
                                    let pair = pair.as_array().expect("slice pair");
                                    let slice = ours
                                        .next()
                                        .unwrap_or_else(|| panic!("{at}: native slice {k} missing"))
                                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                                    assert_eq!(
                                        slice.remaining_before.to_bits(),
                                        int(&pair[0]),
                                        "{at}: remaining before slice {k}"
                                    );
                                    assert_eq!(
                                        slice.duration.to_bits(),
                                        int(&pair[1]),
                                        "{at}: slice {k}"
                                    );
                                    slices += 1;
                                }
                                assert!(ours.next().is_none(), "{at}: extra slice");
                                assert_eq!(
                                    ours.remaining().to_bits(),
                                    after.to_bits(),
                                    "{at}: pending after the frame"
                                );
                            }
                        }
                    }
                }
            }
        }
        println!("frame slicing receipt: files {files}, frames {frames}, skipped {skipped}, slices {slices}, mismatched 0");
        assert!(frames > 0 && slices > 0, "receipt compared nothing");
    }

    /// Research instrument: every law above over random finite and edge inputs
    /// must return without panicking. `MOLY_TIMESTEP_FUZZ` is the iteration count.
    #[test]
    #[ignore = "MOLY_TIMESTEP_FUZZ must give the iteration count"]
    fn frame_laws_are_total() {
        let n: u64 = std::env::var("MOLY_TIMESTEP_FUZZ")
            .expect("MOLY_TIMESTEP_FUZZ")
            .parse()
            .expect("count");
        let edges = [
            0.0f32,
            -0.0,
            1e-45,
            1e-6,
            1e-5,
            0.03,
            0.3333,
            1.0,
            5.0,
            10.0,
            12.0,
            1e6,
            3e38,
            f32::MAX,
            -1.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let time = TimeManagerSnapshot {
            fixed_timestep: f32::from_bits(0x3ca3_d70a),
            maximum_particle_timestep: f32::from_bits(0x3cf5_c28f),
            maximum_delta_time: f32::from_bits(0x3eaa_aaab),
        };
        let mut clock = UnscaledClock::new();
        let (mut ran, mut refused, mut sliced) = (0u64, 0u64, 0u64);
        for _ in 0..n {
            let pick = |r: u64| {
                if r % 4 == 0 {
                    edges[(r >> 8) as usize % edges.len()]
                } else {
                    f32::from_bits((r >> 16) as u32)
                }
            };
            let (pending, dt, speed, duration) =
                (pick(next()), pick(next()), pick(next()), pick(next()));
            let _ = source_delta_seconds(f64::from(dt), time.maximum_delta_time);
            let _ = source_delta_time(Duration::from_nanos(next() % 2_000_000_000), time);
            let _ = clock.advance(Duration::from_nanos(next() % 400_000_000));
            let _ = time_step(dt, pick(next()));
            match frame_step(pending, dt, speed, time, duration) {
                Ok(FrameStep::Skipped) => {}
                Ok(FrameStep::Slices(slices)) => {
                    for slice in slices.take(4096) {
                        if slice.is_err() {
                            break;
                        }
                        sliced += 1;
                    }
                }
                Err(_) => refused += 1,
            }
            ran += 1;
        }
        println!("frame laws: {ran} inputs, {refused} refused, {sliced} slices, 0 panics");
    }
}
