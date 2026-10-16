//! Blink cycles the game's own code produced: the cycle's eye writes and
//! integer draws in order (one delay completed per frame), and the frame of
//! every eye write when the cycle runs under the game's own delay and
//! frame order at a given frame time. Generated; do not edit by hand.

use super::{Blink, EyePattern, RangeDraw};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    Eye(i32),
    Range(i32, i32, i32),
}

struct StepCase {
    name: &'static str,
    enabled: bool,
    pattern: EyePattern,
    cancel_at_entry: bool,
    /// Cancel after this many completed awaits (the token throws there).
    cancel_after: Option<usize>,
    events: &'static [Ev],
    /// The playing flag right after the cycle starts.
    playing_after_start: bool,
}

const STEP_CASES: &[StepCase] = &[
    StepCase {
        name: "disabled",
        enabled: false,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: None,
        events: &[Ev::Eye(1)],
        playing_after_start: false,
    },
    StepCase {
        name: "pattern_no_blink",
        enabled: true,
        pattern: EyePattern { open: 18, close: 6, blink_enabled: false },
        cancel_at_entry: false,
        cancel_after: None,
        events: &[Ev::Eye(18), Ev::Range(3000, 5000, 3777)],
        playing_after_start: true,
    },
    StepCase {
        name: "single",
        enabled: true,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: None,
        events: &[Ev::Eye(5), Ev::Range(100, 150, 120), Ev::Eye(1), Ev::Range(100, 150, 130), Ev::Range(0, 2, 0), Ev::Range(3000, 5000, 4000)],
        playing_after_start: true,
    },
    StepCase {
        name: "double",
        enabled: true,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: None,
        events: &[Ev::Eye(5), Ev::Range(100, 150, 120), Ev::Eye(1), Ev::Range(100, 150, 130), Ev::Range(0, 2, 1), Ev::Eye(5), Ev::Range(100, 150, 110), Ev::Eye(1), Ev::Range(100, 150, 140), Ev::Range(3000, 5000, 4000)],
        playing_after_start: true,
    },
    StepCase {
        name: "double_edges",
        enabled: true,
        pattern: EyePattern { open: 19, close: 7, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: None,
        events: &[Ev::Eye(7), Ev::Range(100, 150, 100), Ev::Eye(19), Ev::Range(100, 150, 149), Ev::Range(0, 2, 1), Ev::Eye(7), Ev::Range(100, 150, 149), Ev::Eye(19), Ev::Range(100, 150, 100), Ev::Range(3000, 5000, 4999)],
        playing_after_start: true,
    },
    StepCase {
        name: "cancel_at_entry",
        enabled: true,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: true,
        cancel_after: None,
        events: &[],
        playing_after_start: false,
    },
    StepCase {
        name: "cancel_during_close",
        enabled: true,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: Some(0),
        events: &[Ev::Eye(5), Ev::Range(100, 150, 120)],
        playing_after_start: true,
    },
    StepCase {
        name: "cancel_during_long_wait",
        enabled: true,
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        cancel_at_entry: false,
        cancel_after: Some(2),
        events: &[Ev::Eye(5), Ev::Range(100, 150, 120), Ev::Eye(1), Ev::Range(100, 150, 130), Ev::Range(0, 2, 0), Ev::Range(3000, 5000, 3000)],
        playing_after_start: true,
    },
];

/// Frame time of frame `f`.
#[derive(Clone, Copy)]
enum Dt {
    Fixed(u32),
    /// 60 Hz, every seventh frame 1/20 s, one frame clamped to 1/3 s.
    Variable,
}

fn dt_of(dt: Dt, frame: u32) -> f32 {
    match dt {
        Dt::Fixed(bits) => f32::from_bits(bits),
        Dt::Variable if frame == 200 => f32::from_bits(0x3EAAAAAB),
        Dt::Variable if frame % 7 == 0 => f32::from_bits(0x3D4CCCCD),
        Dt::Variable => f32::from_bits(0x3C888889),
    }
}

struct CadenceCase {
    name: &'static str,
    pattern: EyePattern,
    draws: &'static [i32],
    dt: Dt,
    /// (frame, eye cell), frame 0 = the first cycle's start, over two cycles.
    writes: &'static [(u32, i32)],
}

const CADENCE_CASES: &[CadenceCase] = &[
    CadenceCase {
        name: "double_then_single_60Hz",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 1, 110, 140, 4000, 101, 149, 0, 3000],
        dt: Dt::Fixed(0x3C888889),
        writes: &[(0, 5), (8, 1), (16, 5), (23, 1), (273, 5), (280, 1)],
    },
    CadenceCase {
        name: "double_then_single_30Hz",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 1, 110, 140, 4000, 101, 149, 0, 3000],
        dt: Dt::Fixed(0x3D088889),
        writes: &[(0, 5), (4, 1), (8, 5), (12, 1), (138, 5), (142, 1)],
    },
    CadenceCase {
        name: "double_then_single_var",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 1, 110, 140, 4000, 101, 149, 0, 3000],
        dt: Dt::Variable,
        writes: &[(0, 5), (7, 1), (14, 5), (21, 1), (202, 5), (207, 1)],
    },
    CadenceCase {
        name: "long_3000_60Hz",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 0, 3000, 120, 130, 0, 3000],
        dt: Dt::Fixed(0x3C888889),
        writes: &[(0, 5), (8, 1), (197, 5), (205, 1)],
    },
    CadenceCase {
        name: "long_4999_60Hz",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 0, 4999, 120, 130, 0, 4999],
        dt: Dt::Fixed(0x3C888889),
        writes: &[(0, 5), (8, 1), (316, 5), (324, 1)],
    },
    CadenceCase {
        name: "no_blink_pattern_60Hz",
        pattern: EyePattern { open: 18, close: 6, blink_enabled: false },
        draws: &[3777, 3000],
        dt: Dt::Fixed(0x3C888889),
        writes: &[(0, 18), (227, 18)],
    },
    CadenceCase {
        name: "double_then_single_4Hz",
        pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
        draws: &[120, 130, 1, 110, 140, 3000, 101, 149, 0, 3000],
        dt: Dt::Fixed(0x3E800000),
        writes: &[(0, 5), (1, 1), (2, 5), (3, 1), (16, 5), (17, 1)],
    },
];

/// Draws from a fixed list, recording each call.
struct Scripted<'a> {
    values: &'a [i32],
    next: usize,
    log: Vec<Ev>,
}

impl RangeDraw for Scripted<'_> {
    fn range(&mut self, min: i32, max: i32) -> i32 {
        let value = self.values[self.next];
        self.next += 1;
        self.log.push(Ev::Range(min, max, value));
        value
    }
}

#[test]
fn step_cases_match_the_game() {
    for case in STEP_CASES {
        let values: Vec<i32> = case
            .events
            .iter()
            .filter_map(|event| match event {
                Ev::Range(_, _, value) => Some(*value),
                Ev::Eye(_) => None,
            })
            .collect();
        let mut draw = Scripted { values: &values, next: 0, log: Vec::new() };
        let mut blink = Blink::default();
        let mut eyes = Vec::new();
        // The start, then one runner pass per await with a frame time that
        // completes any delay.
        eyes.extend(blink.update(case.enabled, case.cancel_at_entry, case.pattern, &mut draw).map(Ev::Eye));
        assert_eq!(blink.is_playing(), case.playing_after_start, "{}: flag after start", case.name);
        let mut completed = 0;
        while blink.is_playing() {
            if case.cancel_after == Some(completed) {
                blink.cancel();
                break;
            }
            let (cell, ended) = blink.resume(10.0, case.pattern, &mut draw);
            eyes.extend(cell.map(Ev::Eye));
            completed += 1;
            if ended {
                break;
            }
        }
        assert!(!blink.is_playing(), "{}: flag at the end", case.name);
        // The eye writes in order and the draws in order. Within one
        // continuation the order of a write and a draw is not observable.
        let want_eyes: Vec<Ev> =
            case.events.iter().copied().filter(|event| matches!(event, Ev::Eye(_))).collect();
        let want_draws: Vec<Ev> =
            case.events.iter().copied().filter(|event| matches!(event, Ev::Range(..))).collect();
        assert_eq!(eyes, want_eyes, "{}: eye writes", case.name);
        assert_eq!(draw.log, want_draws, "{}: draws", case.name);
        assert_eq!(draw.next, values.len(), "{}: draws left", case.name);
    }
}

#[test]
fn cadence_matches_the_game() {
    for case in CADENCE_CASES {
        let mut draw = Scripted { values: case.draws, next: 0, log: Vec::new() };
        let mut blink = Blink::default();
        let mut writes = Vec::new();
        let mut ended = 0;
        let mut frame = 0_u32;
        while ended < 2 {
            let (cell, done) = blink.resume(dt_of(case.dt, frame), case.pattern, &mut draw);
            writes.extend(cell.map(|cell| (frame, cell)));
            ended += u32::from(done);
            // The game's run stops after two cycles: no third start.
            if ended < 2 {
                let cell = blink.update(true, false, case.pattern, &mut draw);
                writes.extend(cell.map(|cell| (frame, cell)));
            }
            frame += 1;
            assert!(frame < 100_000, "{}: runaway", case.name);
        }
        assert_eq!(writes, case.writes, "{}", case.name);
        assert_eq!(draw.next, case.draws.len(), "{}: draws left", case.name);
    }
}

/// The interval after a cycle follows its Range(3000, 5000) draw.
#[test]
fn interval_varies_with_the_draw() {
    let start = |long: i32| {
        let draws = [120, 130, 0, long, 120, 130, 0, long];
        let case = CadenceCase {
            name: "arm",
            pattern: EyePattern { open: 1, close: 5, blink_enabled: true },
            draws: Box::leak(Box::new(draws)),
            dt: Dt::Fixed(0x3C888889),
            writes: &[],
        };
        let mut draw = Scripted { values: case.draws, next: 0, log: Vec::new() };
        let mut blink = Blink::default();
        let mut starts = Vec::new();
        for frame in 0..400_u32 {
            let out = blink.frame(dt_of(case.dt, frame), true, false, case.pattern, &mut draw);
            if out.started.is_some() {
                starts.push(frame);
            }
            if starts.len() == 2 {
                break;
            }
        }
        starts[1]
    };
    assert_eq!(start(3000), 197);
    assert_eq!(start(4999), 316);
}
