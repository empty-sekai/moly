//! The presenter's `CreateLowerState`: the lower state a character gets when
//! an upper state's result completes.
//!
//! The switch reads the result's state type: `RandomWalk`, `MoveToNPC`,
//! `MoveToPosition` and `Stay` build a general talk, `MoveToPlayer` a
//! greeting, `Rest` a rest, `ChangeSite` the change-site sequence and
//! `MoveToFixture` the fixture action; the default arm is the idle fallback.
//!
//! `CreateFixtureLowerState` puts the fixture result's target first: without
//! a fixture the fallback is the idle state, whatever the result's type. The
//! fixture's talk-data choice and the three-way pre-action dispatch
//! (`CreateFixtureLowerStateFromTalkData`) belong to the fixture lower
//! factories; this switch answers the single action for a fixture move,
//! which those factories replace once they are ported.

use super::timing::REST_AFTER_UPPER_SECONDS;
use super::{LowerState, UpperResult, UpperType};

/// `NPCAvatarPresenter.CreateLowerState` without the fixture talk-data
/// choice. The caller builds the idle fallback with its drawn pause when this
/// answers `None` or an idle.
pub fn create_lower(result: &UpperResult) -> Option<LowerState> {
    if result.state == UpperType::MoveToFixture && result.target_fixture.is_none() {
        return Some(LowerState::Idle { duration: super::IDLE_DURATION_MIN });
    }
    Some(match result.state {
        UpperType::RandomWalk
        | UpperType::MoveToNPC
        | UpperType::MoveToPosition
        | UpperType::Stay => LowerState::GeneralTalk,
        UpperType::MoveToFixture => LowerState::SingleFixtureAction,
        UpperType::MoveToPlayer => LowerState::Greeting,
        UpperType::Rest => LowerState::Rest { duration: REST_AFTER_UPPER_SECONDS },
        UpperType::ChangeSite => LowerState::ChangeSite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_hfsm::AvatarMoveStatus;

    fn result(state: UpperType, target_fixture: Option<u32>) -> UpperResult {
        UpperResult {
            state,
            arrival: [0.0; 3],
            target_fixture,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        }
    }

    #[test]
    fn a_fixture_move_without_a_fixture_is_the_idle() {
        // CreateFixtureLowerState tests the target fixture before the switch.
        assert_eq!(
            create_lower(&result(UpperType::MoveToFixture, None)),
            Some(LowerState::Idle { duration: super::super::IDLE_DURATION_MIN })
        );
        // Every other type keeps its arm without a fixture target.
        assert_eq!(create_lower(&result(UpperType::Rest, None)), Some(LowerState::Rest { duration: 5.0 }));
        assert_eq!(create_lower(&result(UpperType::ChangeSite, None)), Some(LowerState::ChangeSite));
    }

    #[test]
    fn the_switch_is_the_source_table() {
        for state in [
            UpperType::RandomWalk,
            UpperType::MoveToNPC,
            UpperType::MoveToPosition,
            UpperType::Stay,
        ] {
            assert_eq!(create_lower(&result(state, None)), Some(LowerState::GeneralTalk));
        }
        assert_eq!(
            create_lower(&result(UpperType::MoveToFixture, Some(1))),
            Some(LowerState::SingleFixtureAction)
        );
        assert_eq!(
            create_lower(&result(UpperType::MoveToPlayer, None)),
            Some(LowerState::Greeting)
        );
        assert_eq!(create_lower(&result(UpperType::Rest, None)), Some(LowerState::Rest { duration: 5.0 }));
        assert_eq!(create_lower(&result(UpperType::ChangeSite, None)), Some(LowerState::ChangeSite));
    }
}
