//! Rest script execution: ordered conditional blocks with a coroutine cursor.
//!
//! A loop always draws its integer random value. Sequential blocks then check
//! the script-start wall-clock window, their own probability draw, and optional
//! slot memory, in that order. A successful block resumes at the next block
//! after its waits; it does not restart the loop. Slot writes happen at the
//! block tail and store the wall second captured at the loop head.

use std::collections::HashMap;

use super::row::{Program, ProgramKind, Scenario, Step, Trigger};
use super::step::wait_milliseconds;

/// One integer draw in the inclusive range 0..=99.
pub trait PercentDraw {
    fn percent(&mut self) -> u32;
}

#[derive(Debug, Clone)]
struct ActiveBlock {
    scenario: Option<usize>,
    remember_slot: Option<String>,
    next_step: usize,
}

/// State belongs to one invocation of one character's Rest script.
#[derive(Debug, Clone)]
pub struct ScriptState {
    started_wall_second: i64,
    loop_wall_second: i64,
    loop_roll: u32,
    next_block: usize,
    loop_started: bool,
    active: Option<ActiveBlock>,
    wait_until: Option<f64>,
    memory: HashMap<String, i64>,
    pub loops: u64,
    pub selected_blocks: u64,
    pub draws: u64,
}

impl ScriptState {
    pub fn new(wall_second: i64) -> Self {
        Self {
            started_wall_second: wall_second,
            loop_wall_second: wall_second,
            loop_roll: 0,
            next_block: 0,
            loop_started: false,
            active: None,
            wait_until: None,
            memory: HashMap::new(),
            loops: 0,
            selected_blocks: 0,
            draws: 0,
        }
    }

    pub fn current_scenario(&self) -> Option<usize> {
        self.active.as_ref().and_then(|active| active.scenario)
    }

    /// Resume until the next coroutine wait. A wait starts when encountered;
    /// excess time from a late frame is not credited to a later wait.
    pub fn advance<'a>(
        &mut self,
        program: &Program,
        scenarios: &'a [Scenario],
        tail: &'a [Step],
        wall_second: i64,
        elapsed_seconds: f64,
        rng: &mut impl PercentDraw,
    ) -> Vec<&'a Step> {
        if let Some(until) = self.wait_until {
            if elapsed_seconds < until {
                return Vec::new();
            }
            self.wait_until = None;
        }
        let mut events = Vec::new();
        loop {
            if let Some(active) = self.active.as_mut() {
                let steps = active
                    .scenario
                    .map(|index| scenarios[index].steps.as_slice())
                    .unwrap_or(tail);
                if let Some(step) = steps.get(active.next_step) {
                    active.next_step += 1;
                    if let Step::Wait { seconds, .. } = step {
                        self.wait_until =
                            Some(elapsed_seconds + f64::from(wait_milliseconds(*seconds)) / 1000.0);
                        return events;
                    }
                    events.push(step);
                    continue;
                }
                let finished = self.active.take().expect("active block was present");
                if let Some(slot) = finished.remember_slot {
                    self.memory.insert(slot, self.loop_wall_second);
                }
                if finished.scenario.is_none() {
                    self.loop_started = false;
                    self.loops += 1;
                }
            }
            if !self.loop_started {
                self.loop_roll = rng.percent();
                self.draws += 1;
                self.loop_wall_second = wall_second;
                self.next_block = 0;
                self.loop_started = true;
            }
            if let Some(block) = program.blocks.get(self.next_block) {
                self.next_block += 1;
                let scenario = &scenarios[block.scenario];
                let selected = match &scenario.trigger {
                    Trigger::RandomBranch(trigger) => {
                        let draw = f64::from(self.loop_roll);
                        draw >= trigger.low && draw < trigger.high
                    }
                    Trigger::TimeGated(trigger) => {
                        if (wall_second - self.started_wall_second) as f64
                            >= trigger.time_limit_seconds
                        {
                            false
                        } else {
                            let draw = rng.percent();
                            self.draws += 1;
                            if f64::from(draw) >= trigger.probability * 100.0 {
                                false
                            } else {
                                trigger.motion_slot.as_ref().is_none_or(|slot| {
                                    self.memory.get(slot).is_none_or(|last| {
                                        (self.loop_wall_second - *last) as f64
                                            > trigger.slot_memory_seconds
                                    })
                                })
                            }
                        }
                    }
                };
                if selected {
                    self.selected_blocks += 1;
                    if program.kind == ProgramKind::RandomBranch {
                        self.next_block = program.blocks.len();
                    }
                    self.active = Some(ActiveBlock {
                        scenario: Some(block.scenario),
                        remember_slot: block.remember_slot.clone(),
                        next_step: 0,
                    });
                }
                continue;
            }
            self.active = Some(ActiveBlock {
                scenario: None,
                remember_slot: None,
                next_step: 0,
            });
        }
    }
}
