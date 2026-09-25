//! Bounded JP 6.8.1 ParticleSystem prewarm plan. Source lifecycle admission remains separate.
//! Transcribed from the current JP 6.8.1 libunity: ParticleSystem::GetTimeStep
//! and ParticleSystem::Update1Incremental; the native prewarm observations are
//! replayed by opt-in tests that read them from outside the repository. First
//! Play runs ParticleSystem::ComputePrewarmStartParameters, then
//! BeginUpdate(dt=out) and ParticleSystem::Update1b, which scales that dt by
//! the simulation speed.
//! Only first Play at elapsed zero, ordinary nonprocedural, constant/two-constant
//! lifetime, looping prewarm and source flags=8 are qualified. The warm length
//! adds the sub-emitter child term ([`sub_emitter_maximum_lifetime`]).
//! Every later frame goes through the same Update1b head ([`FrameStep`]) and the
//! same incremental slice loop, with flags 0 (BeginUpdate while the world plays).

use super::frame_time::{IncrementalEntry, IncrementalSlices};
pub use super::frame_time::Slice;

#[derive(Clone, Copy, Debug)]
pub enum Lifetime {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
}

impl Lifetime {
    /// The upper lane of the lifetime range, as both the warm length and its
    /// sub-emitter term read it: constant mode stores (0, v) only for v > 0,
    /// else (v, 0); two constants are ordered by one `max > min` compare
    /// (unordered keeps the min field in the upper lane).
    pub fn upper(self) -> f32 {
        match self {
            Lifetime::Constant(v) => {
                if v > 0.0 {
                    v
                } else {
                    0.0
                }
            }
            Lifetime::TwoConstants { min, max } => {
                if max > min {
                    max
                } else {
                    min
                }
            }
        }
    }

    /// The system's own term of the first-Play warm length: the upper lane,
    /// an upper lane of exactly +Infinity replaced by the main-module
    /// duration before any warm arithmetic. The sub-emitter term is computed
    /// from this value too.
    pub fn first_play_upper(self, duration: f32) -> f32 {
        let upper = self.upper();
        if upper == f32::INFINITY {
            duration
        } else {
            upper
        }
    }
}

/// One system of an authored sub-emitter graph, as the warm length's child
/// term reads it.
#[derive(Clone, Debug)]
pub struct SubEmitterNode {
    /// Whether the system's SubModule is enabled. A disabled module hands out
    /// no child, so its edges are not read.
    pub sub_module_enabled: bool,
    /// The child of every edge, in authored order, of every trigger type
    /// (birth, collision, death, trigger and manual alike), as an index into
    /// the graph; `None` for a null pointer.
    pub children: Vec<Option<usize>>,
    /// The start lifetime; `None` for a curve-mode lifetime, whose range
    /// (CalculateCurveRangesValue) is not transcribed.
    pub lifetime: Option<Lifetime>,
}

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

/// CalculateSubEmitterMaximumLifeTime of `node` with `life`, the
/// time the chain has taken so far: 0 when the node's SubModule is disabled;
/// otherwise, for each edge in authored order whose child is neither null nor
/// the node itself, the child's upper lifetime plus `life` (one f32 add; the
/// child's simulation speed is not read), then the child's own term from
/// that sum, each kept only when the running best is ordered less than it.
/// The int argument the engine passes down unchanged is read nowhere, so the
/// recursion has no depth bound: a chain that comes back to a system it is
/// still inside would not end, and is refused here.
pub fn sub_emitter_maximum_lifetime(nodes: &[SubEmitterNode], node: usize, life: f32) -> Result<f32, &'static str> {
    let mut path = vec![node];
    maximum_lifetime(nodes, node, life, &mut path)
}

fn maximum_lifetime(nodes: &[SubEmitterNode], node: usize, life: f32, path: &mut Vec<usize>)
    -> Result<f32, &'static str> {
    let this = nodes.get(node).ok_or("sub-emitter graph index outside the graph")?;
    if !this.sub_module_enabled {
        return Ok(0.0);
    }
    let mut best = 0.0_f32;
    for &child in &this.children {
        let Some(child) = child else { continue };
        if child == node {
            continue;
        }
        if path.contains(&child) {
            return Err("sub-emitter chain revisits a system: the maximum-lifetime recursion does not end");
        }
        let upper = nodes
            .get(child)
            .ok_or("sub-emitter graph index outside the graph")?
            .lifetime
            .ok_or("curve-mode child lifetime: its range is not transcribed")?
            .upper();
        let sum = if arms::on("maxInsteadOfSum") { upper } else { upper + life };
        if best < sum {
            best = sum;
        }
        if arms::on("noRecursion") {
            continue;
        }
        path.push(child);
        let deeper = maximum_lifetime(nodes, child, sum, path)?;
        path.pop();
        if best < deeper {
            best = deeper;
        }
    }
    Ok(best)
}

/// The player's serialized TimeManager. The game assembly calls only Time
/// getters, so these values hold for the whole session; the time scale is 1
/// and is therefore not carried.
#[derive(Clone, Copy, Debug)]
pub struct TimeManagerSnapshot {
    pub fixed_timestep: f32,
    pub maximum_particle_timestep: f32,
    /// Serialized Maximum Allowed Timestep: Time.maximumDeltaTime.
    pub maximum_delta_time: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PlayState {
    pub elapsed: f32,
    pub live_count: usize,
    /// Bool at the native Play boundary. The managed withChildren mapping is
    /// not established by this scheduler replay.
    pub native_play_bool_argument: bool,
    pub world_playing: bool,
    pub ordinary_incremental: bool,
    /// The sub-emitter term of the warm length
    /// ([`sub_emitter_maximum_lifetime`] of the system with its
    /// [`Lifetime::first_play_upper`]); 0 when the SubModule is off or no
    /// edge has a live child.
    pub sub_emitter_max_lifetime: f32,
    pub prewarm: bool,
    pub looping: bool,
    pub simulation_speed: f32,
    pub duration: f32,
}

/// Update1b returns before the pending sum, the emission, the slices and the
/// clock when GetTimeStep gives less than this (or NaN).
pub const MIN_UPDATE_STEP: f32 = f32::from_bits(0x3727_c5ac);

/// Head of Update1b on the non-fixed route, shared by first-Play prewarm and
/// every later frame: the frame dt scaled by the simulation speed, then
/// GetTimeStep's slice length. The arithmetic is `frame_time`'s (`fmax_zero`,
/// `time_step`); this keeps the scaled dt, which the frame head's emission over
/// distance reads.
#[derive(Clone, Copy, Debug)]
pub struct FrameStep {
    /// dt * FMAX(speed, +0): the system seconds this frame adds to the pending
    /// time. FMAX keeps a NaN speed, so the frame then skips.
    pub scaled_dt: f32,
    /// GetTimeStep: the scaled dt up to the maximum particle timestep, else
    /// scaled / ceil(scaled / maximum), each an f32 fdiv / frintp / fdiv.
    pub step: f32,
}

impl FrameStep {
    pub fn new(frame_dt: f32, simulation_speed: f32, time: TimeManagerSnapshot) -> Self {
        let scaled_dt = frame_dt * super::frame_time::fmax_zero(simulation_speed);
        Self { scaled_dt, step: super::frame_time::time_step(scaled_dt, time.maximum_particle_timestep) }
    }

    /// A skipped frame adds nothing to the pending time and runs no slice.
    pub fn skips(self) -> bool {
        !(self.step >= MIN_UPDATE_STEP)
    }

    /// Slices of an unskipped frame: this frame's scaled dt plus the pending
    /// time earlier frames left below the loop threshold. The pending sum
    /// adds the scaled dt, never the step.
    pub fn plan(self, pending: f32, duration: f32) -> Result<PrewarmPlan, &'static str> {
        if self.skips() {
            return Err("skipped frame has no incremental update");
        }
        PrewarmPlan::from_incremental_input(self.scaled_dt + pending, self.step, IncrementalEntry::PerFrame, duration)
    }
}

pub struct PrewarmPlan {
    slices: IncrementalSlices,
    /// ComputePrewarmStartParameters out value: the explicit dt Play hands to
    /// BeginUpdate. Update1b scales it by the simulation speed.
    compute_out: f32,
    /// System clock of the particle system state, written by
    /// ComputePrewarmStartParameters:
    /// the warm window starts mid-cycle so that it ends on a cycle boundary.
    initial_clock: f32,
}

/// First-Play warm window shared by the ordinary (BeginUpdate) and procedural
/// (Update flags=3) routes: ComputePrewarmStartParameters, then Update1b's
/// speed scale. Only the incremental slicing after this differs by route.
#[derive(Clone, Copy, Debug)]
pub struct FirstPlayWarm {
    /// Compute out value, the explicit dt Play hands to the update entry.
    pub compute_out: f32,
    /// Update1b `dt * max(simulationSpeed, 0)`: system seconds added to the
    /// remaining incremental time.
    pub total: f32,
    /// System clock of the particle system state, written by Compute before
    /// any update.
    pub initial_clock: f32,
}

impl FirstPlayWarm {
    pub fn from_source(
        lifetime: Lifetime,
        time: TimeManagerSnapshot,
        play: PlayState,
    ) -> Result<Self, &'static str> {
        // These are admission requirements, not convenient substitutes for
        // the unreplayed Play/Compute branches.
        if !play.native_play_bool_argument
            || !play.world_playing
            || !play.prewarm
            || !play.looping
            || play.live_count != 0
            || play.elapsed.to_bits() != 0
        {
            return Err("Play/Compute branch outside qualified first-prewarm path");
        }
        if !(play.duration.is_finite()
            && play.duration > 0.0
            && play.simulation_speed.is_finite()
            && play.simulation_speed > 0.0)
        {
            return Err("unqualified duration or simulation speed");
        }
        if !(time.fixed_timestep.is_finite()
            && time.fixed_timestep > 0.0
            && time.maximum_particle_timestep.is_finite()
            && time.maximum_particle_timestep > 0.0)
        {
            return Err("unqualified TimeManager snapshot");
        }
        // ComputePrewarmStartParameters: the lifetime range upper lane, an
        // upper lane of exactly +Infinity replaced by the main-module duration.
        let upper = lifetime.first_play_upper(play.duration);
        // Then the sub-emitter term (0 with the SubModule off or no live
        // child), kept by one ordered less-than. An infinite term fails the
        // window check below, which is the engine's no-warm path.
        let sub = play.sub_emitter_max_lifetime;
        let lifetime = if sub < upper { upper } else { sub };
        // With the prewarm flag set, Compute's out = (fmod(elapsed, fixed) + L) /
        // max(speed, 0.001); start = elapsed - L - fmod(elapsed, fixed).
        let phase = play.elapsed % time.fixed_timestep;
        let warm = phase + lifetime;
        let compute_out = warm / play.simulation_speed.max(f32::from_bits(0x3a83_126f));
        let mut start = (play.elapsed - lifetime) - phase;
        let magnitude = if start < 0.0 { -start } else { start };
        // Compute moves a negative start forward by whole
        // durations (fcvtps rounds toward +Infinity, then back to f32).
        if start < 0.0 {
            let cycles = ((-start) / play.duration).ceil() as i32 as f32;
            start += play.duration * cycles;
        }
        let end = magnitude + start;
        let initial_clock = start % play.duration;
        // Compute's last test: both window ends must still advance by the fixed
        // step; otherwise Compute logs and Play performs no prewarm update.
        if !(time.fixed_timestep + start > start && time.fixed_timestep + end > end) {
            return Err("Compute prewarm window does not advance by the fixed step");
        }
        // Play passes out as the explicit dt of the update entry. Update1b
        // scales it by max(simulationSpeed, 0) before
        // GetTimeStep and the remaining sum, on both routes.
        let total = compute_out * play.simulation_speed.max(0.0);
        if !(total.is_finite() && total > 0.0) {
            return Err("prewarm total overflows f32");
        }
        Ok(Self {
            compute_out,
            total,
            initial_clock,
        })
    }
}

impl PrewarmPlan {
    pub fn from_source(
        lifetime: Lifetime,
        time: TimeManagerSnapshot,
        play: PlayState,
    ) -> Result<Self, &'static str> {
        // DetermineSupportsProcedural true takes Update(flags=3) instead of
        // BeginUpdate; its incremental slicing differs from this plan.
        if !play.ordinary_incremental {
            return Err("Play/Compute branch outside qualified first-prewarm path");
        }
        let warm = FirstPlayWarm::from_source(lifetime, time, play)?;
        let remaining = warm.total;
        // GetTimeStep nonfixed: total / ceil(total / maximum particle step),
        // f32 at each of its ARM fdiv, frintp and fdiv.
        let ratio = remaining / time.maximum_particle_timestep;
        if !(ratio.is_finite() && ratio.ceil() <= 1_000_000.0) {
            return Err("prewarm slice budget outside bounded replay");
        }
        let base_step = if remaining > time.maximum_particle_timestep {
            remaining / ratio.ceil()
        } else {
            remaining
        };
        // Update1b skips the incremental update entirely
        // below this step.
        if !(base_step.is_finite() && base_step >= MIN_UPDATE_STEP) {
            return Err("unqualified nonfixed timestep");
        }
        let mut plan =
            Self::from_incremental_input(remaining, base_step, IncrementalEntry::ExplicitDt, play.duration)?;
        plan.compute_out = warm.compute_out;
        plan.initial_clock = warm.initial_clock;
        Ok(plan)
    }

    /// Boundary after native Compute/GetTimeStep. Input step and entry must
    /// come from those source paths; the extra current-native matrix probes
    /// this function without pretending each step came from GetTimeStep.
    /// Flags 8 is Play's explicit dt and 0 the per-frame BeginUpdate while the
    /// world plays; the loop reads only bits 0 to 2, clear in both.
    pub fn from_incremental_input(
        total: f32,
        step: f32,
        entry: IncrementalEntry,
        duration: f32,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            slices: IncrementalSlices::new(total, step, entry, duration)?,
            compute_out: total,
            initial_clock: 0.0,
        })
    }

    pub fn remaining(&self) -> f32 {
        self.slices.remaining()
    }
    pub fn base_step(&self) -> f32 {
        self.slices.base_step()
    }
    pub fn compute_out(&self) -> f32 {
        self.compute_out
    }
    pub fn initial_clock(&self) -> f32 {
        self.initial_clock
    }
}

/// Update1Incremental's slices: the old selected slice is retained when it
/// exceeds the 1 s / 0.2 s threshold, hence the observed seven 1 s calls
/// (including backlog values 10..6), not merely two calls while remaining > 10.
impl Iterator for PrewarmPlan {
    type Item = Result<Slice, &'static str>;
    fn next(&mut self) -> Option<Self::Item> {
        self.slices.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn initial_play() -> PlayState {
        PlayState {
            elapsed: 0.0,
            live_count: 0,
            native_play_bool_argument: true,
            world_playing: true,
            ordinary_incremental: true,
            sub_emitter_max_lifetime: 0.0,
            prewarm: true,
            looping: true,
            simulation_speed: 1.0,
            duration: 1.0,
        }
    }
    fn time() -> TimeManagerSnapshot {
        TimeManagerSnapshot {
            fixed_timestep: 0.02,
            maximum_particle_timestep: 0.03,
            maximum_delta_time: 1.0 / 3.0,
        }
    }
    fn read(key: &str) -> Value {
        let path = std::env::var_os(key).expect(key);
        parse(&std::fs::read(path).unwrap()).unwrap()
    }
    fn number(v: &Value) -> f32 {
        v.as_f64().unwrap() as f32
    }
    fn compare(mut plan: PrewarmPlan, rows: &[Value], final_remaining: &Value, pairs: bool) {
        for (i, row) in rows.iter().enumerate() {
            let actual = plan.next().expect("native row has no Rust slice").unwrap();
            let (remaining, step) = if pairs {
                let pair = row.as_array().unwrap();
                assert_eq!(pair.len(), 2);
                (&pair[0], &pair[1])
            } else {
                (row.get("remaining").unwrap(), row.get("slice").unwrap())
            };
            assert_eq!(
                actual.remaining_before.to_bits(),
                number(remaining).to_bits(),
                "remaining[{i}]"
            );
            assert_eq!(
                actual.duration.to_bits(),
                number(step).to_bits(),
                "slice[{i}]"
            );
        }
        assert!(plan.next().is_none(), "extra Rust slice");
        assert_eq!(
            plan.remaining().to_bits(),
            number(final_remaining).to_bits(),
            "remaining after native return"
        );
    }

    #[test]
    #[ignore = "MOLY_CURRENT_PREWARM_JSON must identify current JP native prewarm receipt"]
    fn current_native_prewarm_all_slices_match_bits() {
        let receipt = read("MOLY_CURRENT_PREWARM_JSON");
        assert_eq!(
            receipt.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let replay = receipt.get("prewarmSchedulerReplay").unwrap();
        let plan = PrewarmPlan::from_source(
            Lifetime::TwoConstants {
                min: 12.0,
                max: 8.0,
            },
            time(),
            initial_play(),
        )
        .unwrap();
        let rows = replay.get("slices").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 174);
        compare(plan, rows, replay.get("remaining").unwrap(), false);
    }

    #[test]
    #[ignore = "MOLY_CURRENT_PREWARM_MATRIX must identify current ARM64 incremental matrix"]
    fn current_native_incremental_matrix_matches_bits() {
        let receipt = read("MOLY_CURRENT_PREWARM_MATRIX");
        assert_eq!(
            receipt.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let cases = receipt.get("cases").unwrap().as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let flags = number(case.get("flags").unwrap());
            assert_eq!(flags, 8.0);
            let plan = PrewarmPlan::from_incremental_input(
                number(case.get("total").unwrap()),
                number(case.get("stepArgument").unwrap()),
                IncrementalEntry::ExplicitDt,
                1.0,
            )
            .unwrap();
            compare(
                plan,
                case.get("slices").unwrap().as_array().unwrap(),
                case.get("remaining").unwrap(),
                true,
            );
        }
    }

    #[test]
    #[ignore = "MOLY_CURRENT_PREWARM_LONG must identify current JP long-lifetime native receipt"]
    fn current_native_half_speed_long_prewarm_matches_bits() {
        let receipt = read("MOLY_CURRENT_PREWARM_LONG");
        assert_eq!(
            receipt.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let compute = receipt.get("computeHalfSpeed").unwrap();
        assert_eq!(compute.get("valid").unwrap().as_bool(), Some(true));
        assert_eq!(number(compute.get("warmupSeconds").unwrap()).to_bits(), 130.0f32.to_bits());
        // This receipt feeds 130 s straight into Update1Incremental. It is an
        // incremental-scheduler probe only: first Play scales Compute's 130 s
        // by the 0.5 speed in Update1b (see the first-Play receipt test).
        let timestep = receipt.get("timeStepHalfSpeed").unwrap();
        let observed = number(timestep.get("result").unwrap());
        let source = receipt.get("sourceDerived130").unwrap();
        assert_eq!(observed.to_bits(), number(source.get("stepArgument").unwrap()).to_bits());
        let plan = PrewarmPlan::from_incremental_input(130.0, observed, IncrementalEntry::ExplicitDt, 1.0).unwrap();
        compare(plan, source.get("slices").unwrap().as_array().unwrap(),
            source.get("remaining").unwrap(), true);
        for case in receipt.get("cases").unwrap().as_array().unwrap() {
            let plan = PrewarmPlan::from_incremental_input(
                number(case.get("total").unwrap()),
                number(case.get("stepArgument").unwrap()),
                IncrementalEntry::ExplicitDt,
                1.0,
            ).unwrap();
            compare(plan, case.get("slices").unwrap().as_array().unwrap(),
                case.get("remaining").unwrap(), true);
        }
    }

    #[test]
    #[ignore = "MOLY_CURRENT_PREWARM_FIRST_PLAY must identify current JP first-Play Update1b receipt"]
    fn current_native_first_play_prewarm_matches_bits() {
        let receipt = read("MOLY_CURRENT_PREWARM_FIRST_PLAY");
        assert_eq!(
            receipt.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let cases = receipt.get("cases").unwrap();
        // Same serialized inputs as the native probe cases.
        let inputs = [
            ("cloudSunInfinite24", Lifetime::Constant(f32::INFINITY), 24.0, 1.0),
            ("snowTwoConstants12x8", Lifetime::TwoConstants { min: 12.0, max: 8.0 }, 1.0, 1.0),
            ("halfSpeedTwoConstants65x35", Lifetime::TwoConstants { min: 65.0, max: 35.0 }, 1.0, 0.5),
            ("clockPhase12over5", Lifetime::Constant(12.0), 5.0, 1.0),
            ("clockPhase12over5HalfSpeed", Lifetime::Constant(12.0), 5.0, 0.5),
            ("doubleSpeed12over5", Lifetime::Constant(12.0), 5.0, 2.0),
            ("thirdSpeedTwoConstants7x3over0p7", Lifetime::TwoConstants { min: 7.0, max: 3.0 }, 0.7, 1.0 / 3.0),
        ];
        assert_eq!(cases.as_object().unwrap().len(), inputs.len());
        for (name, lifetime, duration, speed) in inputs {
            let case = cases.get(name).expect(name);
            let compute = case.get("compute").unwrap();
            assert_eq!(compute.get("valid").unwrap().as_bool(), Some(true), "{name}");
            let plan = PrewarmPlan::from_source(
                lifetime,
                time(),
                PlayState { simulation_speed: speed, duration, ..initial_play() },
            )
            .unwrap();
            assert_eq!(plan.compute_out().to_bits(),
                number(compute.get("warmupSeconds").unwrap()).to_bits(), "{name} Compute out");
            assert_eq!(plan.compute_out().to_bits(),
                number(case.get("update1bDtArgument").unwrap()).to_bits(), "{name} BeginUpdate dt");
            assert_eq!(plan.initial_clock().to_bits(),
                number(compute.get("clock").unwrap()).to_bits(), "{name} Compute clock");
            let entry = case.get("incrementalEntry").unwrap();
            assert_eq!(plan.remaining().to_bits(),
                number(entry.get("stateRemaining").unwrap()).to_bits(), "{name} Update1b total");
            assert_eq!(plan.base_step().to_bits(),
                number(entry.get("stepArgument").unwrap()).to_bits(), "{name} GetTimeStep");
            compare(plan, case.get("slices").unwrap().as_array().unwrap(),
                case.get("remainingAfter").unwrap(), true);
        }
    }

    fn bits(v: &Value) -> f32 {
        f32::from_bits(v.as_f64().unwrap() as u32)
    }

    /// One frame of the per-frame head and slice loop, or of a one-rule arm.
    /// Returns (step, entry pending, slices as (remaining, slice), births
    /// ahead per slice, pending after); `None` for the entry of a skipped frame.
    #[allow(clippy::type_complexity)]
    fn frame_law(
        dt: f32,
        speed: f32,
        pending: f32,
        duration: f32,
        arm: Option<&str>,
    ) -> (f32, Option<f32>, Vec<(f32, f32)>, Vec<f32>, f32) {
        let head = FrameStep::new(dt, speed, time());
        if head.skips() {
            return (head.step, None, Vec::new(), Vec::new(), pending);
        }
        let (total, mut plan) = match arm {
            Some("accumulateStep") => (head.step + pending,
                PrewarmPlan::from_incremental_input(head.step + pending, head.step, IncrementalEntry::PerFrame, duration).unwrap()),
            Some("singleSlice") => (head.scaled_dt + pending, PrewarmPlan::from_incremental_input(
                head.scaled_dt + pending, head.scaled_dt + pending, IncrementalEntry::PerFrame, duration).unwrap()),
            _ => (head.scaled_dt + pending, head.plan(pending, duration).unwrap()),
        };
        let mut slices = Vec::new();
        let mut ahead = Vec::new();
        for slice in plan.by_ref() {
            let slice = slice.unwrap();
            slices.push((slice.remaining_before, slice.duration));
            ahead.push(slice.births_ahead());
        }
        (head.step, Some(total), slices, ahead, plan.remaining())
    }

    /// Replays every frame of the native per-frame Update1b receipts through
    /// the head and slice law; returns (frames, mismatched frames).
    fn replay_frames(receipts: &[Value], arm: Option<&str>) -> (usize, usize) {
        let (mut frames, mut mismatched) = (0, 0);
        for receipt in receipts {
            for group in ["s11Cases", "s1Cases", "controls"] {
                let Some(cases) = receipt.get(group).and_then(Value::as_array) else { continue };
                for case in cases {
                    if case.get("patched").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let config = case.get("config").unwrap();
                    let speed = match config.get("speed").unwrap() {
                        Value::Str(text) if text == "nan" => f32::NAN,
                        value => number(value),
                    };
                    let duration = number(config.get("duration").unwrap());
                    let mut pending = 0.0_f32;
                    for frame in case.get("frames").unwrap().as_array().unwrap() {
                        frames += 1;
                        let input = frame.get("input").unwrap();
                        let flags = number(input.get("flags").unwrap());
                        assert!(flags == 0.0 || flags == 8.0);
                        let dt = bits(input.get("dtBits").unwrap());
                        let (step, entry, slices, ahead, after) = frame_law(dt, speed, pending, duration, arm);
                        let mut same = step.to_bits() == bits(frame.get("stepBits").unwrap()).to_bits();
                        match (entry, frame.get("incremental").unwrap()) {
                            (None, Value::Null) => {}
                            (Some(total), native) if !matches!(native, Value::Null) => {
                                same &= total.to_bits() == bits(native.get("pendingBits").unwrap()).to_bits()
                                    && step.to_bits() == bits(native.get("stepBits").unwrap()).to_bits();
                            }
                            _ => same = false,
                        }
                        let native_slices = frame.get("slices").unwrap().as_array().unwrap();
                        same &= native_slices.len() == slices.len()
                            && native_slices.iter().zip(&slices).all(|(pair, (remaining, slice))| {
                                let pair = pair.as_array().unwrap();
                                bits(&pair[0]).to_bits() == remaining.to_bits()
                                    && bits(&pair[1]).to_bits() == slice.to_bits()
                            });
                        // The per-slice StartParticles rows carry the births-ahead argument.
                        let per_slice: Vec<f32> = frame.get("starts").unwrap().as_array().unwrap().iter()
                            .filter(|row| number(row.get("lr").unwrap()) == 14_200_180.0)
                            .map(|row| bits(&row.get("argsBits").unwrap().as_array().unwrap()[2]))
                            .collect();
                        same &= per_slice.len() == ahead.len()
                            && per_slice.iter().zip(&ahead).all(|(native, rust)| native.to_bits() == rust.to_bits());
                        same &= after.to_bits()
                            == bits(frame.get("after").unwrap().get("pendingBits").unwrap()).to_bits();
                        if !same {
                            mismatched += 1;
                        }
                        // Continue from the native state so one arm error is counted per frame.
                        pending = bits(frame.get("after").unwrap().get("pendingBits").unwrap());
                    }
                }
            }
        }
        (frames, mismatched)
    }

    #[test]
    #[ignore = "MOLY_UPDATE1B_FRAMES and MOLY_UPDATE1B_FRAMES_EXTRA must identify the current JP per-frame Update1b receipts"]
    fn current_native_per_frame_head_and_slices_match_bits() {
        let receipts = [read("MOLY_UPDATE1B_FRAMES"), read("MOLY_UPDATE1B_FRAMES_EXTRA")];
        for receipt in &receipts {
            assert_eq!(
                receipt.get("sourceSha256").unwrap().as_str(),
                Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
            );
        }
        let (frames, mismatched) = replay_frames(&receipts, None);
        assert_eq!(frames, 667);
        assert_eq!(mismatched, 0);
        // One-rule arms read against the same native rows must fail.
        for arm in ["singleSlice", "accumulateStep"] {
            let (_, wrong) = replay_frames(&receipts, Some(arm));
            assert!(wrong > 0, "{arm} arm matched every native frame");
            println!("arm {arm}: {wrong} of {frames} frames differ");
        }
    }

    #[test]
    fn uncovered_lifecycle_branches_are_refused() {
        let qualified = initial_play();
        for play in [
            PlayState {
                elapsed: 0.35,
                ..qualified
            },
            // An infinite sub-emitter term leaves the Compute window without
            // a fixed step: the engine logs and Play does no warm.
            PlayState {
                sub_emitter_max_lifetime: f32::INFINITY,
                ..qualified
            },
            PlayState {
                live_count: 1,
                ..qualified
            },
            PlayState {
                ordinary_incremental: false,
                ..qualified
            },
            PlayState {
                simulation_speed: 0.0,
                ..qualified
            },
        ] {
            assert!(PrewarmPlan::from_source(Lifetime::Constant(12.0), time(), play).is_err());
        }
    }

    /// One native Compute case as the normalized rows hold it: the system's
    /// warm inputs, its sub-emitter graph (node 0 the system itself, each
    /// edge's trigger kept for the one-rule arms) and the native outcome.
    struct WarmLengthCase {
        name: String,
        lifetime: Option<Lifetime>,
        duration: f32,
        speed: f32,
        elapsed: f32,
        looping: bool,
        prewarm: bool,
        nodes: Vec<SubEmitterNode>,
        triggers: Vec<Vec<u32>>,
        speeds: Vec<f32>,
        valid: bool,
        out: u32,
        clock: u32,
        compare_clock: bool,
    }

    fn row_lifetime(value: &Value) -> Option<Lifetime> {
        match value.get("mode").and_then(Value::as_str) {
            Some("constant") => Some(Lifetime::Constant(bits(value.get("bits").unwrap()))),
            Some("twoConstants") => Some(Lifetime::TwoConstants {
                min: bits(value.get("minBits").unwrap()),
                max: bits(value.get("maxBits").unwrap()),
            }),
            _ => None,
        }
    }

    fn warm_length_cases(receipt: &Value) -> Vec<WarmLengthCase> {
        receipt.get("rows").unwrap().as_array().unwrap().iter().map(|row| {
            let parent = row.get("parent").unwrap();
            let native = row.get("native").unwrap();
            let rows = row.get("nodes").unwrap().as_array().unwrap();
            let edges = |node: &Value| node.get("children").unwrap().as_array().unwrap().to_vec();
            WarmLengthCase {
                name: row.get("name").unwrap().as_str().unwrap().to_owned(),
                lifetime: row_lifetime(parent.get("lifetime").unwrap()),
                duration: bits(parent.get("durationBits").unwrap()),
                speed: bits(parent.get("speedBits").unwrap()),
                elapsed: bits(parent.get("elapsedBits").unwrap()),
                looping: parent.get("looping").unwrap().as_bool().unwrap(),
                prewarm: parent.get("prewarm").unwrap().as_bool().unwrap(),
                nodes: rows.iter().map(|node| SubEmitterNode {
                    sub_module_enabled: node.get("sub").unwrap().as_bool().unwrap(),
                    children: edges(node).iter().map(|edge| {
                        let pair = edge.as_array().unwrap();
                        pair[1].as_f64().map(|index| index as usize)
                    }).collect(),
                    lifetime: node.get("lifetime").and_then(row_lifetime),
                }).collect(),
                triggers: rows.iter().map(|node| edges(node).iter()
                    .map(|edge| number(&edge.as_array().unwrap()[0]) as u32).collect()).collect(),
                speeds: rows.iter().map(|node| bits(node.get("speedBits").unwrap())).collect(),
                valid: number(native.get("valid").unwrap()) == 1.0,
                out: native.get("outBits").unwrap().as_f64().unwrap() as u32,
                clock: native.get("clockBits").unwrap().as_f64().unwrap() as u32,
                compare_clock: native.get("compareClock").unwrap().as_bool().unwrap(),
            }
        }).collect()
    }

    /// The case under one arm: the law's own arms, or the named input change.
    fn arm_inputs(case: &WarmLengthCase, arm: Option<&str>) -> (Option<Lifetime>, Vec<SubEmitterNode>, bool) {
        let mut lifetime = case.lifetime;
        let mut nodes = case.nodes.clone();
        // The max field in place of the ordered maximum, wherever an upper
        // lane is read: a two-constant pair below every finite value keeps it.
        let max_field = |lifetime: Lifetime| match lifetime {
            Lifetime::TwoConstants { max, .. } => Lifetime::TwoConstants { min: f32::NEG_INFINITY, max },
            other => other,
        };
        match arm {
            Some("birthOnly") => for (node, triggers) in nodes.iter_mut().zip(&case.triggers) {
                node.children = node.children.iter().zip(triggers).filter(|(_, t)| **t == 0).map(|(c, _)| *c).collect();
            },
            Some("childSpeedDivides") => for (node, speed) in nodes.iter_mut().zip(&case.speeds).skip(1) {
                node.lifetime = node.lifetime.map(|l| Lifetime::TwoConstants { min: f32::NEG_INFINITY, max: l.upper() / *speed });
            },
            Some("scalarFieldNotOrderedMax") => {
                lifetime = lifetime.map(max_field);
                for node in &mut nodes {
                    node.lifetime = node.lifetime.map(max_field);
                }
            }
            Some("ignoreParentSubEnabled") => nodes[0].sub_module_enabled = true,
            Some("ignoreChildSubEnabled") => for node in nodes.iter_mut().skip(1) { node.sub_module_enabled = true },
            _ => {}
        }
        (lifetime, nodes, arm == Some("noChildTerm"))
    }

    enum WarmLengthOutcome {
        /// valid, then out and clock bits of a valid window.
        Compared(bool, Option<(u32, u32)>),
        /// A refusal by name: the law does not transcribe the input.
        Refused(&'static str),
    }

    fn warm_length(case: &WarmLengthCase, fixed: f32, arm: Option<&'static str>) -> WarmLengthOutcome {
        let (lifetime, nodes, no_child_term) = arm_inputs(case, arm);
        let Some(lifetime) = lifetime else {
            return WarmLengthOutcome::Refused("curve start lifetime");
        };
        arms::set(arm);
        let sub = sub_emitter_maximum_lifetime(&nodes, 0, lifetime.first_play_upper(case.duration));
        arms::set(None);
        let sub = match sub {
            Ok(sub) => if no_child_term { 0.0 } else { sub },
            Err(reason) => return WarmLengthOutcome::Refused(reason),
        };
        let time = TimeManagerSnapshot { fixed_timestep: fixed, maximum_particle_timestep: 0.03,
            maximum_delta_time: 1.0 / 3.0 };
        let play = PlayState { elapsed: case.elapsed, simulation_speed: case.speed, duration: case.duration,
            looping: case.looping, prewarm: case.prewarm, sub_emitter_max_lifetime: sub, ..initial_play() };
        match FirstPlayWarm::from_source(lifetime, time, play) {
            Ok(warm) => WarmLengthOutcome::Compared(true, Some((warm.compute_out.to_bits(), warm.initial_clock.to_bits()))),
            Err("Compute prewarm window does not advance by the fixed step") => WarmLengthOutcome::Compared(false, None),
            Err(reason) => WarmLengthOutcome::Refused(reason),
        }
    }

    /// Compared cases, mismatched fields (valid, out, clock), the mismatched
    /// cases and the refused ones by name, under one arm.
    fn replay_warm_lengths(cases: &[WarmLengthCase], fixed: f32, arm: Option<&'static str>)
        -> (usize, usize, Vec<String>, Vec<String>) {
        let (mut compared, mut fields) = (0, 0);
        let (mut mismatched, mut refused) = (Vec::new(), Vec::new());
        for case in cases {
            match warm_length(case, fixed, arm) {
                WarmLengthOutcome::Refused(reason) => refused.push(format!("{}: {reason}", case.name)),
                WarmLengthOutcome::Compared(valid, window) => {
                    compared += 1;
                    let mut bad = usize::from(valid != case.valid);
                    if let (Some((out, clock)), true) = (window, case.valid) {
                        bad += usize::from(out != case.out);
                        bad += usize::from(case.compare_clock && clock != case.clock);
                    }
                    fields += bad;
                    if bad > 0 {
                        mismatched.push(case.name.clone());
                    }
                }
            }
        }
        (compared, fields, mismatched, refused)
    }

    #[test]
    #[ignore = "MOLY_WARM_LENGTH_ROWS must identify the normalized rows of the current JP native Compute receipt"]
    fn warm_length_with_the_sub_emitter_term_matches_native_bits() {
        let receipt = read("MOLY_WARM_LENGTH_ROWS");
        assert_eq!(receipt.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let fixed = bits(receipt.get("fixedBits").unwrap());
        let cases = warm_length_cases(&receipt);
        assert_eq!(cases.len(), 45);
        let (compared, fields, mismatched, refused) = replay_warm_lengths(&cases, fixed, None);
        println!("warm length: {} cases, {compared} compared, {fields} mismatched fields {mismatched:?}; refused {refused:?}",
            cases.len());
        assert_eq!(fields, 0, "mismatched cases {mismatched:?}");
        // The law's own refusal is the curve-mode child; the rest are the
        // warm's admission requirements (Play warms a looping prewarm system
        // with a positive speed only).
        assert!(refused.iter().any(|r| r.contains("curve-mode child lifetime")));
        for arm in ["noChildTerm", "maxInsteadOfSum", "noRecursion", "birthOnly", "childSpeedDivides",
            "scalarFieldNotOrderedMax", "ignoreParentSubEnabled", "ignoreChildSubEnabled"] {
            let (_, wrong, cases, _) = replay_warm_lengths(&cases, fixed, Some(arm));
            println!("arm {arm}: {wrong} mismatched fields over {} cases {:?}", cases.len(),
                cases.iter().take(4).collect::<Vec<_>>());
            assert!(wrong > 0, "{arm} arm matched every native case");
        }
    }
}
