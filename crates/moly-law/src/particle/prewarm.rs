//! Bounded JP 6.8.1 ParticleSystem prewarm plan. Source lifecycle admission remains separate.
//! Transcribed from the current JP 6.8.1 libunity: ParticleSystem::GetTimeStep
//! and ParticleSystem::Update1Incremental; the native prewarm observations are
//! replayed by opt-in tests that read them from outside the repository. First
//! Play runs ParticleSystem::ComputePrewarmStartParameters, then
//! BeginUpdate(dt=out) and ParticleSystem::Update1b, which scales that dt by
//! the simulation speed.
//! Only first Play at elapsed zero, ordinary nonprocedural, no real child,
//! constant/two-constant lifetime, looping prewarm and source flags=8 are qualified.

use super::frame_time::{IncrementalEntry, IncrementalSlices};
pub use super::frame_time::Slice;

#[derive(Clone, Copy, Debug)]
pub enum Lifetime {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
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
    pub no_real_subemitters: bool,
    pub prewarm: bool,
    pub looping: bool,
    pub simulation_speed: f32,
    pub duration: f32,
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
            || !play.no_real_subemitters
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
        // ComputePrewarmStartParameters first takes the lifetime range's
        // upper lane. Constant mode stores (0, v) only for v > 0, else (v, 0);
        // two constants are ordered by one `max > min` compare (unordered keeps
        // the min field in the upper lane).
        let upper = match lifetime {
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
        };
        // Compute then replaces an upper lane of exactly +Infinity by the
        // main-module duration before any warm arithmetic.
        let upper = if upper == f32::INFINITY {
            play.duration
        } else {
            upper
        };
        // No real child: CalculateSubEmitterMaximumLifeTime is not consulted
        // (disabled module) or resolves no live child, so the sub maximum is 0
        // and Compute's comparison with it keeps `upper` only when 0 < upper.
        let lifetime = if 0.0 < upper { upper } else { 0.0 };
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
        if !(base_step.is_finite() && base_step >= f32::from_bits(0x3727_c5ac)) {
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
            no_real_subemitters: true,
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

    #[test]
    fn uncovered_lifecycle_branches_are_refused() {
        let qualified = initial_play();
        for play in [
            PlayState {
                elapsed: 0.35,
                ..qualified
            },
            PlayState {
                no_real_subemitters: false,
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
}
