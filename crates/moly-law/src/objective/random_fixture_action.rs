//! The home site's random fixture action: on the room-to-home door move (and
//! once when the home site is set up) the home controller picks about a
//! third of the visible home NPCs and asks each to take one fixture action.
//!
//! The source's shape, in order:
//!
//! - **Gate.** It runs only for a solo player who owns the room, outside the
//!   tutorial; otherwise it returns at once.
//! - **Wait.** It waits until the avatar store's NPC list is non-empty. The
//!   wait is an engine-loop wait, so its predicate is first tested on the
//!   frame after the call; with no NPC it never returns.
//! - **Candidates.** The unit ids of the NPCs that are visible and whose own
//!   site type is home (0), in NPC-list order.
//! - **Shuffle.** Fisher-Yates from the back: for `i` from `n - 1` down to 1,
//!   one engine draw `Range(0, i + 1)` (max exclusive), then the two entries
//!   swap. `n - 1` draws; none for fewer than two ids.
//! - **Target.** `ceil(n * 0.3f)` as an int (single-precision product).
//! - **Passes.** One pass walks the shuffled ids in order with a fresh
//!   excluded set: an excluded id or an id with no NPC is skipped; otherwise
//!   the NPC tries its action and a success counts; the pass stops once the
//!   count reaches its target. A pass whose target is below 1, or an empty
//!   list, runs nothing. When the first pass falls short, exactly one second
//!   pass runs over the same order for the remainder.
//! - **One try.** The NPC's own action reports success or failure. On
//!   success, when the talk it now carries is a multiple-character fixture
//!   talk and its own id is in that talk's cast, its own id and every other
//!   cast id join the excluded set (the source filters a one-element list
//!   holding its id by the cast and, when that empties it, excludes the
//!   character and its co-actors); any other talk, or no talk data, excludes
//!   nothing. A failure excludes nothing.
//!
//! The NPC's own action ([`NpcFixtureActionPick`]) chooses between one unread
//! fixture-action talk and one no-talk fixture action of that character:
//! with both available one engine `Random.value` above 0.5 takes the talk,
//! otherwise the no-talk action, and it reports **failure** in both of those
//! arms (the compiled body returns false after either force); with only one
//! of them available it runs that one and reports success; with neither it
//! reports failure.

use std::collections::HashSet;

/// The share of candidates the controller targets (the compiled constant is
/// the single-precision literal 0.3).
pub const TARGET_SHARE: f32 = 0.3;

/// Threshold of the talk-or-no-talk draw: the talk is taken when
/// `Random.value > 0.5`.
pub const TALK_OVER_NO_TALK_ABOVE: f32 = 0.5;

/// `CalculateTargetActionCount(n)`: `ceil(n * 0.3f)` converted to int; a
/// positive infinity converts to `i32::MIN` (the runtime's conversion), NaN
/// to 0 and out-of-range values saturate.
pub fn target_action_count(total: i32) -> i32 {
    let product = total as f32 * TARGET_SHARE;
    if product.ceil() == f32::INFINITY {
        i32::MIN
    } else {
        product.ceil() as i32
    }
}

/// `ShuffleCharacterIds`: Fisher-Yates from the back. `draw(lo, hi)` is the
/// engine's integer range (`hi` exclusive); it is called `len - 1` times with
/// `hi` = `len`, `len - 1`, ..., 2.
pub fn shuffle_character_ids(ids: &mut [i32], draw: &mut dyn FnMut(i32, i32) -> i32) {
    let len = ids.len() as i32;
    if len - 1 < 1 {
        return;
    }
    let mut bound = len;
    loop {
        let i = bound - 1;
        let j = draw(0, bound);
        assert!(
            (0..bound).contains(&j),
            "engine range draw {j} outside [0, {bound})"
        );
        ids.swap(i as usize, j as usize);
        if i <= 1 {
            break;
        }
        bound = i;
    }
}

/// The result of one NPC's try, as the pass reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryOutcome {
    /// `FindNPC` found no NPC for the id: skipped, nothing counted.
    NoNpc,
    /// The NPC's action reported failure: nothing counted or excluded.
    Failed,
    /// The NPC's action reported success. `cast` is the unit ids of the talk
    /// data it now carries (`None` without talk data); `multiple` is whether
    /// that talk is a multiple-character fixture talk.
    Succeeded {
        cast: Option<Vec<i32>>,
        multiple: bool,
    },
}

/// `TryExecuteFixtureActionForCharacter`'s bookkeeping after the NPC's action
/// reported success: which ids join the excluded set.
pub fn exclusions_after_success(unit: i32, cast: Option<&[i32]>, multiple: bool) -> Vec<i32> {
    // RemoveSomeCharacterFixtureActionCharacterList([unit]): without talk data
    // the source logs an error and returns the list as it was; a talk that is
    // not a multiple-character fixture talk returns it as it was too; a
    // multiple-character one keeps the ids that are not in its cast.
    let filtered: Vec<i32> = match cast {
        Some(cast) if multiple => [unit].into_iter().filter(|id| !cast.contains(id)).collect(),
        _ => vec![unit],
    };
    if !filtered.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if !filtered.contains(&unit) {
        out.push(unit);
    }
    if multiple {
        if let Some(cast) = cast.filter(|cast| !cast.is_empty()) {
            out.extend(cast.iter().copied().filter(|id| *id != unit));
        }
    }
    out
}

/// `ExecuteFixtureActionsForCharacters`: one pass; returns the success count.
pub fn run_pass(ids: &[i32], target: i32, try_one: &mut dyn FnMut(i32) -> TryOutcome) -> i32 {
    let mut excluded: HashSet<i32> = HashSet::new();
    let mut count = 0;
    if target < 1 || ids.is_empty() {
        return 0;
    }
    for &id in ids {
        if !excluded.contains(&id) {
            match try_one(id) {
                TryOutcome::NoNpc | TryOutcome::Failed => {}
                TryOutcome::Succeeded { cast, multiple } => {
                    count += 1;
                    for other in exclusions_after_success(id, cast.as_deref(), multiple) {
                        excluded.insert(other);
                    }
                }
            }
        }
        if count >= target {
            break;
        }
    }
    count
}

/// `ExecuteFixtureActionsWithRetry`: the first pass, and one more for the
/// remainder when it fell short. Returns both counts (the second is `None`
/// when it did not run).
pub fn run_with_retry(
    ids: &[i32],
    target: i32,
    try_one: &mut dyn FnMut(i32) -> TryOutcome,
) -> (i32, Option<i32>) {
    let first = run_pass(ids, target, try_one);
    if first < target {
        (first, Some(run_pass(ids, target - first, try_one)))
    } else {
        (first, None)
    }
}

/// What one NPC's `ExecuteRandomFixtureAction` does, given which of its two
/// candidates exist and (only when both do) the engine's `Random.value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcFixtureActionPick {
    /// Force the unread fixture-action talk; `reports_success` is the value
    /// the body returns.
    Talk { reports_success: bool },
    /// Force the no-talk fixture action.
    NoTalk { reports_success: bool },
    /// Neither candidate: nothing runs, failure.
    Nothing,
}

/// The branch of `ExecuteRandomFixtureAction`. `value` is drawn only when both
/// candidates exist; pass a closure that draws it.
pub fn pick_npc_fixture_action(
    has_talk: bool,
    has_no_talk: bool,
    value: &mut dyn FnMut() -> f32,
) -> NpcFixtureActionPick {
    match (has_talk, has_no_talk) {
        (true, true) => {
            if value() > TALK_OVER_NO_TALK_ABOVE {
                NpcFixtureActionPick::Talk {
                    reports_success: false,
                }
            } else {
                NpcFixtureActionPick::NoTalk {
                    reports_success: false,
                }
            }
        }
        (true, false) => NpcFixtureActionPick::Talk {
            reports_success: true,
        },
        (false, true) => NpcFixtureActionPick::NoTalk {
            reports_success: true,
        },
        (false, false) => NpcFixtureActionPick::Nothing,
    }
}
