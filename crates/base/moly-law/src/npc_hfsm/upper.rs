//! The upper-state roulette: the controller's `DecideUpperState` and its
//! `SetUpperStateWeight`.
//!
//! `SetUpperStateWeight` adds an entry or replaces the entry of the same
//! state type in place (the list keeps its order, a zero weight stays).
//! Only the presenter's tutorial branch pushes weights (`RandomWalk` 5,
//! `MoveToFixture` 5, `Rest` 2); a normal session's list is empty.
//!
//! `DecideUpperState` with an empty list, or with a weight sum at or below
//! zero, returns `RandomWalk` **without drawing**. Otherwise it takes
//! exactly one engine float draw `r` in `[0, total)`, scans a single-
//! precision running sum and takes the first entry with `r <= acc`; the
//! trailing fallback (reached only through rounding) is the last entry.
//!
//! The two marker upper states (`Rest`, `Stay`) and the base's entry clock
//! are also here: the markers have no fields, no mover and no timer, so the
//! inherited base `OnUpdate` completes them on their first update.

use super::{AvatarMoveStatus, UpperBase, UpperEntry, UpperResult, UpperState, UpperType};

/// `RestUpperState` and `StayUpperState`: no fields, no mover, no timer. The
/// enter is empty; the inherited `OnUpdate` sees the running flag clear and
/// completes the state on its first update; the result tags the state type,
/// the presenter's position and `Success`.
#[derive(Debug, Clone, PartialEq)]
pub struct RestUpper {
    kind: UpperType,
    base: UpperBase,
    entry: Option<UpperEntry>,
}

impl RestUpper {
    /// `Rest` for the controller's rest marker.
    pub fn rest() -> Self {
        Self::of(UpperType::Rest)
    }

    /// `Stay`, the same marker under its own type.
    pub fn stay() -> Self {
        Self::of(UpperType::Stay)
    }

    fn of(kind: UpperType) -> Self {
        Self {
            kind,
            base: UpperBase::default(),
            entry: None,
        }
    }

    /// The upper state object.
    pub fn state(&self) -> UpperState {
        match self.kind {
            UpperType::Stay => UpperState::Stay,
            _ => UpperState::Rest,
        }
    }

    /// `Enter()`: the marker carries no mover, so the entry is not moving.
    pub fn enter(&mut self) {
        self.entry = Some(self.base.enter(false));
    }

    /// `Update()`: one frame of the base; the marker completes on this same
    /// update.
    pub fn update(&mut self, dt: f32) {
        if let Some(entry) = self.entry.as_mut() {
            entry.update(dt);
        }
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.entry.map_or(false, |entry| entry.is_complete)
    }

    /// `GetResult()`: the marker's type, the presenter's position, no
    /// fixture or character target, `Success`.
    pub fn result(&self, position: [f32; 3]) -> UpperResult {
        UpperResult {
            state: self.kind,
            arrival: position,
            target_fixture: None,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        }
    }

    /// `Exit()`.
    pub fn exit(&mut self) {
        if let Some(entry) = self.entry.as_mut() {
            entry.exit();
        }
    }
}

/// `UpperStateWeightEntry`: one state type and its weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperStateWeight {
    /// The state type.
    pub state: UpperType,
    /// Its weight.
    pub weight: f32,
}

/// `SetUpperStateWeight`: adds `entry`, replacing an entry of the same state
/// type in place.
pub fn set_upper_weight(weights: &mut Vec<UpperStateWeight>, state: UpperType, weight: f32) {
    if let Some(existing) = weights.iter_mut().find(|entry| entry.state == state) {
        existing.weight = weight;
    } else {
        weights.push(UpperStateWeight { state, weight });
    }
}

/// What `DecideUpperState` answered, and whether it drew.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperChoice {
    /// The state type.
    pub state: UpperType,
    /// The draw, when one was made.
    pub draw: Option<f32>,
    /// The weight sum the draw was taken in.
    pub total: f32,
    /// The entry index the scan accepted (`None` on the no-draw paths and on
    /// the trailing fallback).
    pub entry: Option<usize>,
}

/// `DecideUpperState` over `weights`; `draw` is called only when the sum is
/// positive and receives the sum.
pub fn decide_upper(
    weights: &[UpperStateWeight],
    draw: &mut dyn FnMut(f32) -> f32,
) -> UpperChoice {
    if weights.is_empty() {
        return UpperChoice {
            state: UpperType::RandomWalk,
            draw: None,
            total: 0.0,
            entry: None,
        };
    }
    let mut total = 0.0f32;
    for entry in weights {
        total += entry.weight;
    }
    if total <= 0.0 {
        return UpperChoice {
            state: UpperType::RandomWalk,
            draw: None,
            total,
            entry: None,
        };
    }
    let drawn = draw(total);
    let mut acc = 0.0f32;
    for (index, entry) in weights.iter().enumerate() {
        acc += entry.weight;
        if drawn <= acc {
            return UpperChoice {
                state: entry.state,
                draw: Some(drawn),
                total,
                entry: Some(index),
            };
        }
    }
    UpperChoice {
        state: weights[weights.len() - 1].state,
        draw: Some(drawn),
        total,
        entry: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draw stand-in: hands out the given draws in turn and counts calls.
    /// The draws are absolute values, not fractions of the sum, so a draw
    /// past the running sums can be made on purpose.
    struct Queue {
        values: Vec<f32>,
        taken: usize,
    }

    impl Queue {
        fn at(values: &[f32]) -> Self {
            Self {
                values: values.to_vec(),
                taken: 0,
            }
        }
    }

    fn run(weights: &[UpperStateWeight], queue: &mut Queue) -> UpperChoice {
        decide_upper(weights, &mut |_total| {
            let value = queue.values.get(queue.taken).copied().unwrap_or(0.0);
            queue.taken += 1;
            value
        })
    }

    fn entry(state: UpperType, weight: f32) -> UpperStateWeight {
        UpperStateWeight { state, weight }
    }

    #[test]
    fn empty_and_non_positive_lists_return_random_walk_without_a_draw() {
        let mut queue = Queue::at(&[0.5]);
        let choice = run(&[], &mut queue);
        assert_eq!(choice.state, UpperType::RandomWalk);
        assert_eq!(choice.draw, None);
        assert_eq!(queue.taken, 0);
        for weights in [
            vec![entry(UpperType::Rest, 0.0)],
            vec![entry(UpperType::Rest, 0.0), entry(UpperType::Stay, 0.0)],
            vec![entry(UpperType::Rest, -1.0)],
        ] {
            let choice = run(&weights, &mut queue);
            assert_eq!(choice.state, UpperType::RandomWalk);
            assert_eq!(choice.draw, None);
        }
        assert_eq!(queue.taken, 0);
    }

    #[test]
    fn one_draw_over_the_sum_and_the_first_entry_at_or_above_it() {
        // The tutorial's three weights: 5, 5, 2 over 12.
        let weights = [
            entry(UpperType::RandomWalk, 5.0),
            entry(UpperType::MoveToFixture, 5.0),
            entry(UpperType::Rest, 2.0),
        ];
        // Draws at and just past each running sum: 5 → the first entry, 5.5 →
        // the second, 10.5 → the third; every draw is inside [0, 12).
        let cases = [
            (0.0, UpperType::RandomWalk),
            (4.999, UpperType::RandomWalk),
            (5.0, UpperType::RandomWalk),
            (5.5, UpperType::MoveToFixture),
            (9.999, UpperType::MoveToFixture),
            (10.0, UpperType::MoveToFixture),
            (10.5, UpperType::Rest),
            (11.999, UpperType::Rest),
        ];
        for (drawn, want) in cases {
            let mut queue = Queue::at(&[drawn]);
            let choice = run(&weights, &mut queue);
            assert_eq!(choice.state, want, "draw {drawn}");
            assert_eq!(choice.draw, Some(drawn));
            assert_eq!(choice.total, 12.0);
            assert!(choice.entry.is_some(), "draw {drawn} lands on an entry");
            assert_eq!(queue.taken, 1);
        }
    }

    #[test]
    fn zero_weight_entries_can_only_be_taken_at_an_exact_boundary() {
        // A zero-weight entry is taken only when the draw equals the running
        // sum before it (the scan accepts `r <= acc`).
        let weights = [entry(UpperType::Rest, 0.0), entry(UpperType::Stay, 3.0)];
        let mut queue = Queue::at(&[0.0]);
        assert_eq!(run(&weights, &mut queue).state, UpperType::Rest);
        let mut queue = Queue::at(&[0.5]);
        assert_eq!(run(&weights, &mut queue).state, UpperType::Stay);
    }

    #[test]
    fn a_draw_past_the_sum_falls_to_the_last_entry() {
        // The source scan leaves by `r <= acc` on every entry and answers the
        // last entry after the loop. A draw past the running sums (only
        // rounding can make one: Range(0, total) stops below total) takes
        // that exit. With the draws handed out as given, draw 7.5 of a
        // 7-unit sum passes 5 and 7 and still answers the last entry, so the
        // two paths are told apart by the entry they report.
        let weights = [entry(UpperType::RandomWalk, 5.0), entry(UpperType::Rest, 2.0)];
        let mut queue = Queue::at(&[7.5]);
        let choice = run(&weights, &mut queue);
        assert_eq!(choice.state, UpperType::Rest);
        assert_eq!(choice.draw, Some(7.5));
        assert_eq!(choice.entry, None);
        assert_eq!(queue.taken, 1);
    }

    #[test]
    fn rest_and_stay_need_their_first_update_to_complete() {
        // The upper base's OnUpdate alone: the running flag never sets, so
        // the first update completes the marker (the enter leaves it open).
        for (marker, kind) in [
            (RestUpper::rest(), UpperType::Rest),
            (RestUpper::stay(), UpperType::Stay),
        ] {
            let mut marker = marker;
            assert!(!marker.is_complete(), "not complete before the entry");
            marker.enter();
            assert!(!marker.is_complete(), "the entry does not complete");
            marker.update(1.0 / 60.0);
            assert!(marker.is_complete(), "one update completes");
            let result = marker.result([1.5, 2.0, 3.5]);
            assert_eq!(result.state, kind);
            assert_eq!(result.arrival, [1.5, 2.0, 3.5]);
            assert_eq!(result.move_status, AvatarMoveStatus::Success);
            assert_eq!((result.target_fixture, result.target_npc), (None, None));
            assert!(!result.is_move_failed());
            marker.exit();
        }
        assert_eq!(RestUpper::rest().state(), UpperState::Rest);
        assert_eq!(RestUpper::stay().state(), UpperState::Stay);
    }

    #[test]
    fn the_upper_base_keeps_the_frame_time() {
        // Update() adds Time.deltaTime before OnUpdate; a state with a mover
        // stays incomplete while the flag is set.
        let mut base = UpperBase::default();
        let mut entry = base.enter(true);
        assert!(!entry.is_complete);
        assert!(entry.is_moving);
        entry.update(0.5);
        entry.update(0.25);
        assert_eq!(entry.elapsed, 0.75);
        assert!(!entry.is_complete);
        assert!(entry.can_interrupt());
        entry.is_moving = false;
        entry.update(0.0);
        assert!(entry.is_complete);
        assert!(!UpperState::Rest.has_mover());
        assert!(!UpperState::Stay.has_mover());
        assert!(UpperState::MoveToPosition.has_mover());
    }

    #[test]
    fn set_upper_weight_replaces_in_place_or_appends() {
        // The source scans for the same state type and overwrites its
        // weight; a new type is appended (the order is kept).
        let mut weights = vec![entry(UpperType::RandomWalk, 5.0), entry(UpperType::Rest, 2.0)];
        set_upper_weight(&mut weights, UpperType::RandomWalk, 1.0);
        assert_eq!(
            weights,
            vec![entry(UpperType::RandomWalk, 1.0), entry(UpperType::Rest, 2.0)]
        );
        set_upper_weight(&mut weights, UpperType::MoveToFixture, 5.0);
        set_upper_weight(&mut weights, UpperType::Rest, 0.0);
        assert_eq!(
            weights,
            vec![
                entry(UpperType::RandomWalk, 1.0),
                entry(UpperType::Rest, 0.0),
                entry(UpperType::MoveToFixture, 5.0),
            ]
        );
    }
}
