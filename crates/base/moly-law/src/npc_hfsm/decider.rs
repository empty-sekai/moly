//! `NPCObjectiveDecider`: the two lottery entries, `Decide`, and the
//! already-read entry's sub-tree.
//!
//! `Decide` rings **one** engine integer draw `Random.Range(0, 100)` before
//! it looks at any entry, then walks the entries in registration order. For
//! each entry it adds the entry's percent read at that moment and tests the
//! running sum against the draw with a strict `>`: a draw equal to a
//! cumulative boundary goes to the next entry. The entry whose test holds is
//! asked for a plan; a plan ends the call, and a failing entry **ends the
//! walk too** (later entries are not tried). When no entry produced a plan
//! the decider answers none, and the controller falls through to its upper
//! roulette.
//!
//! The already-read entry's `CreateFixtureTalkSubTreePlan` draws `r1` in
//! `[0, 100)` and takes the none-talk branch when `P143 <= r1` (a high roll);
//! the other branch draws `r2` and takes the common-fixture plan when
//! `P148 <= r2`, the already-read fixture plan otherwise. Every branch that
//! yields no plan answers no plan from `Decide` as well.
//!
//! Draws per call: one always; two when the already-read entry is taken and
//! its sub-tree's first branch yields none-talk; three when the sub-tree
//! runs both draws.

use crate::talk::select::{LotteryPercents, PercentDraw};

/// The registration order of the two entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeciderEntry {
    /// `FixtureTalkEntry`: the fixture-talk lottery (key 142).
    FixtureTalk,
    /// `AlreadyReadFixtureTalkEntry`: the already-read sub-tree (key 149).
    AlreadyReadFixtureTalk,
}

/// The registered entries, in order.
pub const ENTRIES: [DeciderEntry; 2] = [DeciderEntry::FixtureTalk, DeciderEntry::AlreadyReadFixtureTalk];

impl DeciderEntry {
    /// `get_Percent`: the entry's percent, read when the walk reaches it.
    pub fn percent(self, percents: &LotteryPercents) -> f32 {
        match self {
            DeciderEntry::FixtureTalk => percents.fixture_talk,
            DeciderEntry::AlreadyReadFixtureTalk => percents.already_read_fixture_talk,
        }
    }
}

/// What one entry answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryPlan {
    /// The fixture-talk plan: `CreateFixtureTalkPlan`.
    FixtureTalk,
    /// The already-read sub-tree answered the none-talk plan.
    NoneTalk,
    /// The sub-tree answered the common-fixture plan.
    CommonFixtureTalk,
    /// The sub-tree answered the already-read fixture plan.
    AlreadyReadTalk,
    /// `TryCreatePlan` failed (or the plan factory answered none).
    NoPlan,
}

/// The already-read entry's sub-tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubTree {
    /// `P143 <= r1`: the none-talk branch.
    NoneTalk,
    /// `P148 <= r2`: the common-fixture plan.
    CommonFixtureTalk,
    /// Otherwise the already-read fixture plan.
    AlreadyReadTalk,
}

/// The draw ledger of one `Decide` call, in call order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeciderDraws {
    /// The turnstile draw `r`.
    pub turnstile: Option<u32>,
    /// The sub-tree's first draw.
    pub subtree_r1: Option<u32>,
    /// The sub-tree's second draw.
    pub subtree_r2: Option<u32>,
}

impl DeciderDraws {
    /// Every draw made, in order.
    pub fn values(&self) -> Vec<u32> {
        let mut values = Vec::with_capacity(3);
        values.extend(self.turnstile);
        values.extend(self.subtree_r1);
        values.extend(self.subtree_r2);
        values
    }
}

/// What `Decide` answered, and the draws it spent.
#[derive(Debug, Clone, PartialEq)]
pub struct DeciderOutcome {
    /// The entry whose test held, and the plan it answered.
    pub entry: Option<(DeciderEntry, EntryPlan)>,
    /// The sub-tree branch, when the already-read entry ran one.
    pub subtree: Option<SubTree>,
    /// The draw ledger.
    pub draws: DeciderDraws,
}

impl DeciderOutcome {
    /// The plan the decider answered, when it answered one.
    pub fn plan(&self) -> Option<EntryPlan> {
        match self.entry {
            Some((_, plan @ (EntryPlan::FixtureTalk | EntryPlan::NoneTalk | EntryPlan::CommonFixtureTalk | EntryPlan::AlreadyReadTalk))) => Some(plan),
            _ => None,
        }
    }
}

/// `CreateFixtureTalkSubTreePlan`: `r1` always, `r2` only on the low `r1`.
pub fn fixture_talk_subtree(percents: &LotteryPercents, rand: &mut (impl PercentDraw + ?Sized)) -> (SubTree, Option<u32>, Option<u32>) {
    let r1 = rand.draw_percent();
    if percents.none_talk_fixture_action <= r1 as f32 {
        return (SubTree::NoneTalk, Some(r1), None);
    }
    let r2 = rand.draw_percent();
    let branch = if percents.already_read_when_has_not_read <= r2 as f32 {
        SubTree::CommonFixtureTalk
    } else {
        SubTree::AlreadyReadTalk
    };
    (branch, Some(r1), Some(r2))
}

/// `NPCObjectiveDecider.Decide`, with each entry's plan factory given as
/// its answer.
pub fn decide(
    percents: &LotteryPercents,
    rand: &mut impl PercentDraw,
    fixture_talk_plan: &mut dyn FnMut() -> EntryPlan,
    already_read_plan: &mut dyn FnMut(&LotteryPercents, &mut dyn PercentDraw) -> (EntryPlan, Option<SubTree>),
) -> DeciderOutcome {
    let turnstile = rand.draw_percent();
    let mut outcome = DeciderOutcome {
        entry: None,
        subtree: None,
        draws: DeciderDraws {
            turnstile: Some(turnstile),
            ..DeciderDraws::default()
        },
    };
    let mut acc = 0.0f32;
    for entry in ENTRIES {
        acc += entry.percent(percents);
        if !(acc > turnstile as f32) {
            continue;
        }
        let plan = match entry {
            DeciderEntry::FixtureTalk => fixture_talk_plan(),
            DeciderEntry::AlreadyReadFixtureTalk => {
                let (plan, subtree) = already_read_plan(percents, rand);
                if let Some(subtree) = subtree {
                    outcome.subtree = Some(subtree);
                }
                plan
            }
        };
        // A failing entry keeps its answer in the outcome; the walk ends
        // there, and the later entries are not tried.
        outcome.entry = Some((entry, plan));
        break;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draw stand-in: hands out the given values in turn.
    struct Queue {
        values: Vec<u32>,
        taken: usize,
    }

    impl Queue {
        fn at(values: &[u32]) -> Self {
            Self {
                values: values.to_vec(),
                taken: 0,
            }
        }
    }

    impl PercentDraw for Queue {
        fn draw_percent(&mut self) -> u32 {
            let value = self.values.get(self.taken).copied().unwrap_or(0);
            self.taken += 1;
            value
        }
    }

    /// The panel used by the cases: 50 / 20 / 25 / 50.
    fn percents() -> LotteryPercents {
        LotteryPercents {
            fixture_talk: 50.0,
            already_read_fixture_talk: 20.0,
            none_talk_fixture_action: 25.0,
            already_read_when_has_not_read: 50.0,
        }
    }

    /// The real sub-tree plan function, with the two factory answers fixed.
    fn subtree(plan_for: [EntryPlan; 2]) -> impl FnMut(&LotteryPercents, &mut dyn PercentDraw) -> (EntryPlan, Option<SubTree>) {
        move |percents, rand| {
            let (branch, _, _) = fixture_talk_subtree(percents, rand);
            let plan = match branch {
                SubTree::NoneTalk => plan_for[0],
                _ => plan_for[1],
            };
            (plan, Some(branch))
        }
    }

    fn run(percents: &LotteryPercents, draws: &[u32], fixture: EntryPlan, sub: [EntryPlan; 2]) -> (DeciderOutcome, Queue) {
        let mut queue = Queue::at(draws);
        let mut fixture = fixture;
        let mut sub = subtree(sub);
        let outcome = decide(
            percents,
            &mut queue,
            &mut || fixture,
            &mut sub,
        );
        (outcome, queue)
    }

    #[test]
    fn one_draw_always_and_the_strict_boundary_goes_to_the_next_entry() {
        let percents = percents();
        // r just under 50 takes the fixture entry; r = 50 is not > 50 and
        // falls to the already-read entry (cut at 70).
        for (r, want_entry, want_plan) in [
            (0, Some(DeciderEntry::FixtureTalk), Some(EntryPlan::FixtureTalk)),
            (49, Some(DeciderEntry::FixtureTalk), Some(EntryPlan::FixtureTalk)),
            (50, Some(DeciderEntry::AlreadyReadFixtureTalk), None),
            (69, Some(DeciderEntry::AlreadyReadFixtureTalk), None),
            (70, None, None),
            (99, None, None),
        ] {
            let (outcome, queue) = match want_entry {
                Some(DeciderEntry::FixtureTalk) => run(&percents, &[r], EntryPlan::FixtureTalk, [EntryPlan::NoneTalk, EntryPlan::CommonFixtureTalk]),
                _ => run(&percents, &[r, 90, 10], EntryPlan::FixtureTalk, [EntryPlan::NoneTalk, EntryPlan::CommonFixtureTalk]),
            };
            assert_eq!(outcome.entry.map(|(entry, _)| entry), want_entry, "r={r}");
            if let Some(plan) = want_plan {
                assert_eq!(outcome.plan(), Some(plan), "r={r}");
            }
            // The turnstile always; the already-read entry adds its `r1` (90,
            // the none-talk branch, so no `r2`); the fixture entry and a walk
            // that reaches no entry spend the turnstile alone.
            let expected = match want_entry {
                Some(DeciderEntry::AlreadyReadFixtureTalk) => 2,
                _ => 1,
            };
            assert_eq!(queue.taken, expected, "r={r}");
            assert_eq!(outcome.draws.turnstile, Some(r));
        }
    }

    #[test]
    fn the_subtree_branches_on_its_two_draws() {
        let percents = percents();
        // Already-read entry (turnstile 60 enters it): `P143 <= r1` (25) is the
        // none-talk branch — one draw, no `r2`; below it the code draws `r2`
        // and takes the common plan when `P148 <= r2` (50), the already-read
        // plan otherwise.
        let plans = [EntryPlan::NoneTalk, EntryPlan::CommonFixtureTalk];
        let cases = [
            (25, 90, SubTree::NoneTalk, Some(2)),
            (0, 50, SubTree::CommonFixtureTalk, Some(3)),
            (24, 50, SubTree::CommonFixtureTalk, Some(3)),
            (24, 49, SubTree::AlreadyReadTalk, Some(3)),
            (10, 0, SubTree::AlreadyReadTalk, Some(3)),
        ];
        for (r1, r2, want_sub, want_taken) in cases {
            let mut queue = Queue::at(&[60, r1, r2]);
            let mut sub = subtree(plans);
            let outcome = decide(&percents, &mut queue, &mut || EntryPlan::FixtureTalk, &mut sub);
            assert_eq!(outcome.subtree, Some(want_sub), "r1={r1} r2={r2}");
            assert_eq!(Some(queue.taken), want_taken, "r1={r1} r2={r2}");
            // The plan is the factory's: the sub-tree answers the none-talk
            // plan alone, and the other two branches share the one fixture
            // answer this stand-in gives.
            match want_sub {
                SubTree::NoneTalk => assert_eq!(outcome.plan(), Some(EntryPlan::NoneTalk)),
                _ => assert_eq!(outcome.plan(), Some(EntryPlan::CommonFixtureTalk)),
            }
        }
    }

    #[test]
    fn a_failing_entry_ends_the_walk() {
        // The fixture entry's plan fails: the already-read entry is not
        // tried, no plan is answered, and no further draw is made.
        let (outcome, queue) = run(&percents(), &[0, 60, 10], EntryPlan::NoPlan, [EntryPlan::NoneTalk, EntryPlan::CommonFixtureTalk]);
        assert_eq!(outcome.entry, Some((DeciderEntry::FixtureTalk, EntryPlan::NoPlan)));
        assert_eq!(queue.taken, 1);
        assert_eq!(outcome.plan(), None);
        // The sub-tree's answer fails too, whichever branch it took.
        for r1 in [90, 10] {
            let mut queue = Queue::at(&[60, r1, 90]);
            let mut sub = subtree([EntryPlan::NoPlan, EntryPlan::NoPlan]);
            let outcome = decide(&percents(), &mut queue, &mut || EntryPlan::FixtureTalk, &mut sub);
            assert_eq!(outcome.plan(), None);
            assert_eq!(queue.taken, if r1 >= 25 { 2 } else { 3 });
        }
    }

    #[test]
    fn an_empty_panel_still_draws_once() {
        // Both percents zero: the turnstile draw happens before any test, no
        // entry is taken, and no plan answers.
        let empty = LotteryPercents {
            fixture_talk: 0.0,
            already_read_fixture_talk: 0.0,
            none_talk_fixture_action: 0.0,
            already_read_when_has_not_read: 0.0,
        };
        let mut queue = Queue::at(&[0, 90, 10]);
        let mut sub = subtree([EntryPlan::NoneTalk, EntryPlan::CommonFixtureTalk]);
        let outcome = decide(&empty, &mut queue, &mut || EntryPlan::FixtureTalk, &mut sub);
        assert_eq!(queue.taken, 1);
        assert_eq!(outcome.entry, None);
        assert_eq!(outcome.draws.values(), vec![0]);
    }
}
