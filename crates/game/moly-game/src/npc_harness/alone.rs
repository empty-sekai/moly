//! Acceptance harness, idle scripts: the product's script loader and script
//! state driven on the streams the game's own script VM produced under
//! injected random draws and clocks. See the parent harness module for the
//! protocol.
//!
//! Per recorded run: the product parses the idle-script document the product
//! loads, starts a script state at the run's first clock read, and resumes it
//! once per recorded wait. Each resume is fed the recorded draws in order,
//! the clock the recorded run read in that segment, and a frame delta equal
//! to the recorded wait's delay; before the real resume a copy is resumed
//! with the next lower delta, which must not end the wait. The output is the
//! product's step stream per segment; the comparison is made outside.

use serde_json::{json, Value};

use super::{law, parse_actions};
use crate::npc_harness::{case, emit};
use moly_law::objective::DelayPromise;

#[derive(Clone)]
struct Draws {
    values: Vec<u32>,
    next: usize,
    overrun: usize,
}

impl law::PercentDraw for Draws {
    fn percent(&mut self) -> u32 {
        match self.values.get(self.next) {
            Some(value) => {
                self.next += 1;
                *value
            }
            None => {
                self.overrun += 1;
                0
            }
        }
    }
}

fn step_json(unit: u32, step: &law::Step) -> Value {
    match step {
        law::Step::ChangeEye { pattern, .. } => json!({"op": "eye", "unit": unit, "pattern": pattern}),
        law::Step::ChangeMouth { pattern, .. } => json!({"op": "mouth", "unit": unit, "pattern": pattern}),
        law::Step::ChangeAnimation { motion, speed, playback_speed, play_end_motion, blend, phase, .. } => json!({
            "op": "animation", "unit": unit, "motion": motion, "speed": speed, "playback_speed": playback_speed,
            "play_end_motion": play_end_motion, "blend": blend, "phase": phase.map(|s| s.index())}),
        law::Step::ShowEmoticon { name, show_seconds, time_arg, not_play_se_arg, host_not_play_se, .. } => json!({
            "op": "emoticon", "unit": unit, "name": name, "show_seconds": show_seconds, "time_arg": time_arg,
            "not_play_se_arg": not_play_se_arg, "host_not_play_se": host_not_play_se}),
        law::Step::HideEmoticon { .. } => json!({"op": "hide_emoticon", "unit": unit}),
        law::Step::Wait { seconds, .. } => json!({"op": "wait", "unit": unit, "seconds": seconds}),
    }
}

fn clock_of(segment: &Value, carry: i64) -> i64 {
    segment["times"]
        .as_array()
        .and_then(|times| times.last())
        .and_then(Value::as_i64)
        .unwrap_or(carry)
}

#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR and MOLY_ASSET_ROOT"]
fn a010_idle_script_streams() {
    let input = case("a010");
    let root = std::env::var("MOLY_ASSET_ROOT").expect("MOLY_ASSET_ROOT");
    let text = std::fs::read_to_string(format!("{root}/alone-actions.json")).expect("alone-actions.json");
    let actions = parse_actions(&text);
    let mut runs = Vec::new();
    for run in input["runs"].as_array().expect("runs") {
        let unit = run["unit"].as_u64().unwrap() as u32;
        let Some(pool) = actions.units.get(&unit) else {
            runs.push(json!({"unit": unit, "run": run["run"], "missing_unit": true}));
            continue;
        };
        let segments = run["segments"].as_array().unwrap();
        let values: Vec<u32> = segments
            .iter()
            .flat_map(|s| s["draws"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32))
            .collect();
        let mut draws = Draws { values, next: 0, overrun: 0 };
        // A script that never reads the clock: any start value.
        let start = run["start"].as_i64().unwrap_or(0);
        let mut state = law::ScriptState::new(start);
        let mut clock = start;
        let mut out = Vec::new();
        for (index, segment) in segments.iter().enumerate() {
            clock = clock_of(segment, clock);
            let mut early = Value::Null;
            let delta = if index == 0 {
                0.0
            } else {
                let ms = segments[index - 1]["wait_ms"].as_i64().unwrap() as i32;
                let delay = DelayPromise::from_milliseconds(ms).map(|d| d.delay()).unwrap_or(f32::NAN);
                if delay > 0.0 {
                    let mut probe = state.clone();
                    let mut probe_draws = draws.clone();
                    let before = (probe.loops, probe.selected_blocks, probe.draws);
                    let lower = f32::from_bits(delay.to_bits() - 1);
                    let events = probe.advance(&pool.program, &pool.scenarios, &pool.tail, clock, lower, &mut probe_draws);
                    let moved = !events.is_empty()
                        || probe_draws.next != draws.next
                        || (probe.loops, probe.selected_blocks, probe.draws) != before;
                    early = json!(moved);
                }
                delay
            };
            let before = draws.next;
            let events = state.advance(&pool.program, &pool.scenarios, &pool.tail, clock, delta, &mut draws);
            out.push(json!({
                "steps": events.iter().map(|step| step_json(unit, step)).collect::<Vec<_>>(),
                "draws": draws.next - before,
                "early_end_at_lower_delta": early,
                "clock": clock,
                "failed_wait": state.failed_wait(),
            }));
        }
        runs.push(json!({"unit": unit, "run": run["run"], "segments": out, "overrun": draws.overrun,
                         "loops": state.loops, "selected_blocks": state.selected_blocks}));
    }
    emit("a010", &Value::Array(runs));
}
