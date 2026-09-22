//! Bounded JP 6.8.1 ParticleSystem prewarm plan. Source lifecycle admission remains separate.
//! Evidence: prewarm-current.json, current libunity SHA256 937c6d28...75badd9,
//! ComputePrewarmStartParameters 0xd8316c, GetTimeStep 0xd80a9c,
//! Update1b 0xd7e108, Update1Incremental 0xd8aa00.
//! Only first Play at elapsed zero, ordinary nonprocedural, no real child,
//! positive finite constant/two-constant lifetime, source flags=8 is qualified.

#[derive(Clone, Copy, Debug)]
pub enum Lifetime {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
}

#[derive(Clone, Copy, Debug)]
pub struct TimeManagerSnapshot {
    pub fixed_timestep: f32,
    pub maximum_particle_timestep: f32,
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

#[derive(Clone, Copy, Debug)]
pub struct Slice {
    pub remaining_before: f32,
    pub duration: f32,
}

pub struct PrewarmPlan {
    remaining: f32,
    base_step: f32,
    prior_step: f32,
    duration: f32,
    budget: usize,
}

impl PrewarmPlan {
    pub fn from_source(
        lifetime: Lifetime,
        time: TimeManagerSnapshot,
        play: PlayState,
    ) -> Result<Self, &'static str> {
        // These are admission requirements, not convenient substitutes for
        // the unreplayed Play/Compute branches.
        if !play.native_play_bool_argument
            || !play.world_playing
            || !play.ordinary_incremental
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
            && play.simulation_speed.to_bits() == 1.0f32.to_bits())
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
        let max_lifetime = match lifetime {
            Lifetime::Constant(v) => v,
            Lifetime::TwoConstants { min, max } if min.is_finite() && max.is_finite() => {
                min.max(max)
            }
            Lifetime::TwoConstants { .. } => return Err("nonfinite two-constant lifetime"),
        };
        if !(max_lifetime.is_finite() && max_lifetime > 0.0) {
            return Err("unqualified maximum lifetime");
        }
        // ComputePrewarmStartParameters: elapsed=0, no child maximum and
        // looping prewarm -> maximum lifetime / simulation speed. The current
        // ARM body writes s1/s0 at 0xd832ec-0xd832f0.
        let remaining = max_lifetime / play.simulation_speed;
        if !(remaining.is_finite() && remaining > 0.0) {
            return Err("prewarm total overflows f32");
        }
        // GetTimeStep nonfixed: total / ceil(total / maximum particle step),
        // f32 at each ARM fdiv/frintp/fdiv (0xd80af4..0xd80afc).
        let ratio = remaining / time.maximum_particle_timestep;
        if !(ratio.is_finite() && ratio.ceil() <= 1_000_000.0) {
            return Err("prewarm slice budget outside bounded replay");
        }
        let base_step = if remaining > time.maximum_particle_timestep {
            remaining / ratio.ceil()
        } else {
            remaining
        };
        if !(base_step.is_finite() && base_step > 0.0) {
            return Err("unqualified nonfixed timestep");
        }
        Self::from_incremental_input(remaining, base_step, 8, play.duration)
    }

    /// Boundary after native Compute/GetTimeStep. Input step and flags must
    /// come from those source paths; the extra current-native matrix probes
    /// this function without pretending each step came from GetTimeStep.
    pub fn from_incremental_input(
        total: f32,
        step: f32,
        flags: u32,
        duration: f32,
    ) -> Result<Self, &'static str> {
        if flags != 8 {
            return Err("unqualified incremental flags");
        }
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
            budget: 1_000_256,
        })
    }

    pub fn remaining(&self) -> f32 {
        self.remaining
    }
    pub fn base_step(&self) -> f32 {
        self.base_step
    }
}

impl Iterator for PrewarmPlan {
    type Item = Result<Slice, &'static str>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining < f32::from_bits(0x3586_37bd) {
            return None;
        }
        if self.budget == 0 {
            self.remaining = 0.0;
            return Some(Err("incremental budget exhausted"));
        }
        let before = self.remaining;
        let mut step = before.min(self.base_step);
        // Update1Incremental 0xd8ab00..0xd8ab48, flags bit 2 clear:
        // the old selected slice is retained when it exceeds the 1/.2
        // threshold. Hence the observed seven 1s calls (including backlog
        // values 10..6), not merely two calls while remaining >10.
        if before > 10.0 {
            step = if self.prior_step > 1.0 {
                self.prior_step
            } else {
                self.duration.min(1.0)
            };
        } else if before > 5.0 {
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
                8,
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
                simulation_speed: 2.0,
                ..qualified
            },
        ] {
            assert!(PrewarmPlan::from_source(Lifetime::Constant(12.0), time(), play).is_err());
        }
        assert!(
            PrewarmPlan::from_source(Lifetime::Constant(f32::INFINITY), time(), qualified).is_err()
        );
        assert!(PrewarmPlan::from_incremental_input(12.0, 0.03, 4, 1.0).is_err());
    }
}
