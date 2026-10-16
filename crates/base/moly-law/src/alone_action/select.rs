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
use crate::objective::DelayPromise;

/// One integer draw in the inclusive range 0..=99.
pub trait PercentDraw {
    fn percent(&mut self) -> u32;
}

/// The script VM's `math.random(m, n)`: every call takes exactly one value
/// `x` of the C library's `rand()` (uniform on `0..2^31`, before any argument
/// check) and maps it as `m + trunc(x * 2^-31 * ((n - m) + 1.0))` in double
/// precision. For `(0, 99)` every step is exact, so the map is the integer
/// identity `(100 * x) >> 31`: 48 outcomes cover 21,474,837 values of `x`
/// and 52 cover 21,474,836. It is a different map from an exact uniform
/// 0..99, which gives every outcome the same share.
pub fn lua_math_random(x: u32, low: i64, high: i64) -> i64 {
    assert!(x < 1 << 31, "rand() returns values below 2^31");
    let scaled = f64::from(x) * (1.0 / 2_147_483_648.0) * ((high - low) as f64 + 1.0);
    low + scaled as i64
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
    /// The host's wait: a scaled-time delay of the wait's milliseconds.
    wait: Option<DelayPromise>,
    /// A wait whose milliseconds are negative: the host's delay raises, and
    /// the script makes no further step.
    failed_wait: Option<i32>,
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
            wait: None,
            failed_wait: None,
            memory: HashMap::new(),
            loops: 0,
            selected_blocks: 0,
            draws: 0,
        }
    }

    pub fn current_scenario(&self) -> Option<usize> {
        self.active.as_ref().and_then(|active| active.scenario)
    }

    /// The negative milliseconds of a wait the host could not start.
    pub fn failed_wait(&self) -> Option<i32> {
        self.failed_wait
    }

    /// Resume until the next coroutine wait, on one frame whose delta time
    /// is `delta_time`. A wait is the host's scaled-time delay: made when the
    /// script reaches it, it adds each later frame's delta time in single
    /// precision and ends on the first frame its milliseconds are reached;
    /// excess time is not credited to a later wait.
    pub fn advance<'a>(
        &mut self,
        program: &Program,
        scenarios: &'a [Scenario],
        tail: &'a [Step],
        wall_second: i64,
        delta_time: f32,
        rng: &mut impl PercentDraw,
    ) -> Vec<&'a Step> {
        if self.failed_wait.is_some() {
            return Vec::new();
        }
        if let Some(wait) = self.wait.as_mut() {
            if !wait.advance(delta_time) {
                return Vec::new();
            }
            self.wait = None;
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
                        match DelayPromise::from_milliseconds(wait_milliseconds(*seconds)) {
                            Ok(delay) => self.wait = Some(delay),
                            Err(milliseconds) => self.failed_wait = Some(milliseconds),
                        }
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

#[cfg(test)]
mod lua_random_tests {
    use super::lua_math_random;

    /// Source-value instrument: `math.random` of the game's own script VM,
    /// executed as compiled code with the C library's `rand()` injected
    /// (`x`, low, high, result): every bucket edge and edge - 1 of `(0, 99)`,
    /// both ends of `x`, and the other integer forms that were run.
    const EXECUTED: [(u32, i64, i64, i64); 206] = [
        (0, 0, 99, 0),
        (21474837, 0, 99, 1),
        (21474836, 0, 99, 0),
        (42949673, 0, 99, 2),
        (42949672, 0, 99, 1),
        (64424510, 0, 99, 3),
        (64424509, 0, 99, 2),
        (85899346, 0, 99, 4),
        (85899345, 0, 99, 3),
        (107374183, 0, 99, 5),
        (107374182, 0, 99, 4),
        (128849019, 0, 99, 6),
        (128849018, 0, 99, 5),
        (150323856, 0, 99, 7),
        (150323855, 0, 99, 6),
        (171798692, 0, 99, 8),
        (171798691, 0, 99, 7),
        (193273529, 0, 99, 9),
        (193273528, 0, 99, 8),
        (214748365, 0, 99, 10),
        (214748364, 0, 99, 9),
        (236223202, 0, 99, 11),
        (236223201, 0, 99, 10),
        (257698038, 0, 99, 12),
        (257698037, 0, 99, 11),
        (279172875, 0, 99, 13),
        (279172874, 0, 99, 12),
        (300647711, 0, 99, 14),
        (300647710, 0, 99, 13),
        (322122548, 0, 99, 15),
        (322122547, 0, 99, 14),
        (343597384, 0, 99, 16),
        (343597383, 0, 99, 15),
        (365072221, 0, 99, 17),
        (365072220, 0, 99, 16),
        (386547057, 0, 99, 18),
        (386547056, 0, 99, 17),
        (408021894, 0, 99, 19),
        (408021893, 0, 99, 18),
        (429496730, 0, 99, 20),
        (429496729, 0, 99, 19),
        (450971567, 0, 99, 21),
        (450971566, 0, 99, 20),
        (472446403, 0, 99, 22),
        (472446402, 0, 99, 21),
        (493921240, 0, 99, 23),
        (493921239, 0, 99, 22),
        (515396076, 0, 99, 24),
        (515396075, 0, 99, 23),
        (536870912, 0, 99, 25),
        (536870911, 0, 99, 24),
        (558345749, 0, 99, 26),
        (558345748, 0, 99, 25),
        (579820585, 0, 99, 27),
        (579820584, 0, 99, 26),
        (601295422, 0, 99, 28),
        (601295421, 0, 99, 27),
        (622770258, 0, 99, 29),
        (622770257, 0, 99, 28),
        (644245095, 0, 99, 30),
        (644245094, 0, 99, 29),
        (665719931, 0, 99, 31),
        (665719930, 0, 99, 30),
        (687194768, 0, 99, 32),
        (687194767, 0, 99, 31),
        (708669604, 0, 99, 33),
        (708669603, 0, 99, 32),
        (730144441, 0, 99, 34),
        (730144440, 0, 99, 33),
        (751619277, 0, 99, 35),
        (751619276, 0, 99, 34),
        (773094114, 0, 99, 36),
        (773094113, 0, 99, 35),
        (794568950, 0, 99, 37),
        (794568949, 0, 99, 36),
        (816043787, 0, 99, 38),
        (816043786, 0, 99, 37),
        (837518623, 0, 99, 39),
        (837518622, 0, 99, 38),
        (858993460, 0, 99, 40),
        (858993459, 0, 99, 39),
        (880468296, 0, 99, 41),
        (880468295, 0, 99, 40),
        (901943133, 0, 99, 42),
        (901943132, 0, 99, 41),
        (923417969, 0, 99, 43),
        (923417968, 0, 99, 42),
        (944892806, 0, 99, 44),
        (944892805, 0, 99, 43),
        (966367642, 0, 99, 45),
        (966367641, 0, 99, 44),
        (987842479, 0, 99, 46),
        (987842478, 0, 99, 45),
        (1009317315, 0, 99, 47),
        (1009317314, 0, 99, 46),
        (1030792152, 0, 99, 48),
        (1030792151, 0, 99, 47),
        (1052266988, 0, 99, 49),
        (1052266987, 0, 99, 48),
        (1073741824, 0, 99, 50),
        (1073741823, 0, 99, 49),
        (1095216661, 0, 99, 51),
        (1095216660, 0, 99, 50),
        (1116691497, 0, 99, 52),
        (1116691496, 0, 99, 51),
        (1138166334, 0, 99, 53),
        (1138166333, 0, 99, 52),
        (1159641170, 0, 99, 54),
        (1159641169, 0, 99, 53),
        (1181116007, 0, 99, 55),
        (1181116006, 0, 99, 54),
        (1202590843, 0, 99, 56),
        (1202590842, 0, 99, 55),
        (1224065680, 0, 99, 57),
        (1224065679, 0, 99, 56),
        (1245540516, 0, 99, 58),
        (1245540515, 0, 99, 57),
        (1267015353, 0, 99, 59),
        (1267015352, 0, 99, 58),
        (1288490189, 0, 99, 60),
        (1288490188, 0, 99, 59),
        (1309965026, 0, 99, 61),
        (1309965025, 0, 99, 60),
        (1331439862, 0, 99, 62),
        (1331439861, 0, 99, 61),
        (1352914699, 0, 99, 63),
        (1352914698, 0, 99, 62),
        (1374389535, 0, 99, 64),
        (1374389534, 0, 99, 63),
        (1395864372, 0, 99, 65),
        (1395864371, 0, 99, 64),
        (1417339208, 0, 99, 66),
        (1417339207, 0, 99, 65),
        (1438814045, 0, 99, 67),
        (1438814044, 0, 99, 66),
        (1460288881, 0, 99, 68),
        (1460288880, 0, 99, 67),
        (1481763718, 0, 99, 69),
        (1481763717, 0, 99, 68),
        (1503238554, 0, 99, 70),
        (1503238553, 0, 99, 69),
        (1524713391, 0, 99, 71),
        (1524713390, 0, 99, 70),
        (1546188227, 0, 99, 72),
        (1546188226, 0, 99, 71),
        (1567663064, 0, 99, 73),
        (1567663063, 0, 99, 72),
        (1589137900, 0, 99, 74),
        (1589137899, 0, 99, 73),
        (1610612736, 0, 99, 75),
        (1610612735, 0, 99, 74),
        (1632087573, 0, 99, 76),
        (1632087572, 0, 99, 75),
        (1653562409, 0, 99, 77),
        (1653562408, 0, 99, 76),
        (1675037246, 0, 99, 78),
        (1675037245, 0, 99, 77),
        (1696512082, 0, 99, 79),
        (1696512081, 0, 99, 78),
        (1717986919, 0, 99, 80),
        (1717986918, 0, 99, 79),
        (1739461755, 0, 99, 81),
        (1739461754, 0, 99, 80),
        (1760936592, 0, 99, 82),
        (1760936591, 0, 99, 81),
        (1782411428, 0, 99, 83),
        (1782411427, 0, 99, 82),
        (1803886265, 0, 99, 84),
        (1803886264, 0, 99, 83),
        (1825361101, 0, 99, 85),
        (1825361100, 0, 99, 84),
        (1846835938, 0, 99, 86),
        (1846835937, 0, 99, 85),
        (1868310774, 0, 99, 87),
        (1868310773, 0, 99, 86),
        (1889785611, 0, 99, 88),
        (1889785610, 0, 99, 87),
        (1911260447, 0, 99, 89),
        (1911260446, 0, 99, 88),
        (1932735284, 0, 99, 90),
        (1932735283, 0, 99, 89),
        (1954210120, 0, 99, 91),
        (1954210119, 0, 99, 90),
        (1975684957, 0, 99, 92),
        (1975684956, 0, 99, 91),
        (1997159793, 0, 99, 93),
        (1997159792, 0, 99, 92),
        (2018634630, 0, 99, 94),
        (2018634629, 0, 99, 93),
        (2040109466, 0, 99, 95),
        (2040109465, 0, 99, 94),
        (2061584303, 0, 99, 96),
        (2061584302, 0, 99, 95),
        (2083059139, 0, 99, 97),
        (2083059138, 0, 99, 96),
        (2104533976, 0, 99, 98),
        (2104533975, 0, 99, 97),
        (2126008812, 0, 99, 99),
        (2126008811, 0, 99, 98),
        (2147483647, 0, 99, 99),
        (0, -5, 5, -5),
        (0, 7, 7, 7),
        (1073741824, -5, 5, 0),
        (1073741824, 7, 7, 7),
        (2147483647, -5, 5, 5),
        (2147483647, 7, 7, 7),
    ];

    #[test]
    fn lua_math_random_equals_the_executed_vm() {
        let mut outcomes = std::collections::HashSet::new();
        for (x, low, high, result) in EXECUTED {
            assert_eq!(lua_math_random(x, low, high), result, "x={x} ({low}, {high})");
            outcomes.insert(result);
        }
        // Positive arm: the compared dimension carries many outcomes.
        assert!(outcomes.len() >= 90);
    }

    #[test]
    fn lua_percent_buckets_have_the_executed_sizes() {
        // The executed kernel's bucket edges are ceil(k * 2^31 / 100); 48
        // buckets hold 21,474,837 values of x and 52 hold 21,474,836.
        let mut heavy = 0;
        for k in 0..100i64 {
            let low = (k * (1 << 31) + 99) / 100;
            let high = ((k + 1) * (1 << 31) + 99) / 100;
            assert_eq!(lua_math_random(low as u32, 0, 99), k);
            assert_eq!(lua_math_random((high - 1) as u32, 0, 99), k);
            if high - low == 21_474_837 {
                heavy += 1;
            }
        }
        assert_eq!(heavy, 48);
    }
}
