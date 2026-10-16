//! The presenter's objective calls that decide by state alone: the cancel
//! condition, the objective's cancel report, the AI loop's run condition and
//! the two objective waits the gate and the cut-scene use.
//!
//! - [`cancel_condition`]: a cancel is refused while the character talks.
//! - [`try_cancel`]: the objective's own cancel. An objective that cannot be
//!   cancelled reports false; one cancelled before reports true again; a
//!   disposed one reports false; otherwise it is cancelled now (its OnCancel
//!   and its token) and reports true. The caller disposes the objective
//!   after the call whatever it reported.
//! - [`can_running_ai`]: the loop runs its rest and its decision only for a
//!   character outside a fixture timeline, not talking, and either visible
//!   or without an objective, or on the entry-site objective (which hides
//!   the character itself). Otherwise the loop yields again.
//! - [`in_fixture_action_state`]: the immediately-played fixture timeline
//!   objective waits for the fixture-action or fixture-action-idle state.
//! - [`cut_scene_holds`]: the cut-scene objective waits while the game
//!   state is the cut-scene state.

use super::ObjectiveType;

/// The talk state (`NPCActionStateType` 4).
pub const TALK_STATE: i32 = 4;
/// The fixture-action state (11); the fixture-action idle state follows it.
pub const FIXTURE_ACTION_STATE: i32 = 11;
/// The cut-scene game state (`GameStateType` 5).
pub const CUT_SCENE_GAME_STATE: i32 = 5;
/// The cut-scene action state the cut-scene objective changes to (13).
pub const CUT_SCENE_STATE: i32 = 13;

/// The presenter's cancel condition: any state but Talk.
pub fn cancel_condition(state: i32) -> bool {
    state != TALK_STATE
}

/// The objective flags its cancel reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CancelFlags {
    /// The objective's CanCancel (set by its constructor or by the ladder
    /// row that built it; false when neither writes it).
    pub can_cancel: bool,
    /// Cancelled before.
    pub canceled: bool,
    /// Disposed (completed, or disposed by an earlier cancel).
    pub disposed: bool,
}

/// What a cancel reports and does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelOutcome {
    /// The reported value.
    pub reported: bool,
    /// Whether the cancel ran now: the cancelled flag set, OnCancel and the
    /// token's cancel.
    pub cancels_now: bool,
}

/// The objective's cancel over its flags, in the order it tests them.
pub fn try_cancel(flags: CancelFlags) -> CancelOutcome {
    if !flags.can_cancel {
        return CancelOutcome {
            reported: false,
            cancels_now: false,
        };
    }
    if flags.canceled {
        return CancelOutcome {
            reported: true,
            cancels_now: false,
        };
    }
    if flags.disposed {
        return CancelOutcome {
            reported: false,
            cancels_now: false,
        };
    }
    CancelOutcome {
        reported: true,
        cancels_now: true,
    }
}

/// Whether the objective of `kind` can be cancelled. The cut-scene
/// objective's constructor never sets the flag, so it cannot be cancelled
/// (its OnCancel, which raises, is never reached through a cancel); the
/// immediately-played fixture timeline objective sets it.
pub fn objective_can_cancel(kind: ObjectiveType) -> bool {
    kind != ObjectiveType::CutScene
}

/// The AI loop's run condition, tested after each Yield.
pub fn can_running_ai(
    in_fixture_timeline: bool,
    state: i32,
    visible: bool,
    current: Option<ObjectiveType>,
) -> bool {
    if in_fixture_timeline || state == TALK_STATE {
        return false;
    }
    if visible {
        return true;
    }
    match current {
        None => true,
        Some(kind) => kind == ObjectiveType::EntrySite,
    }
}

/// The immediately-played fixture timeline objective's wait: the state is
/// fixture action (11) or fixture-action idle (12), compared unsigned after
/// the subtraction.
pub fn in_fixture_action_state(state: i32) -> bool {
    (state.wrapping_sub(FIXTURE_ACTION_STATE) as u32) < 2
}

/// The cut-scene objective's wait holds while the game state is the
/// cut-scene state.
pub fn cut_scene_holds(game_state: i32) -> bool {
    game_state == CUT_SCENE_GAME_STATE
}
