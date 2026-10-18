//! The tap-ladder probe: a host instrument that writes the Talk button's own
//! request for the character under the stack's head, standing in for a press
//! the gesture layer cannot make in a headless run. It reads the action
//! button, so it lives in the devtools crate; the dispatcher it feeds is
//! `crate::player_talk`.

use bevy::prelude::*;

use crate::npc::CharacterUnitId;
use crate::player::PlayerControlled;
use crate::player_talk::{PlayerTalkRequest, PlayerTalkSession};

/// Host probe of the tap ladder (`MOLY_PLAYER_TALK_STACK_TAP_SECS`, a window
/// `<until>` or `<from>:<until>` in seconds, with the action-button autowalk bringing the player to each
/// character): while the action stack's head is the Talk button of a
/// character and no talk plays, the request the Talk button's dispatch
/// writes for that character is written: at most every 15 s (so the
/// characters get back to their own activities between taps), and at once
/// (2 s after the last tap at the earliest) when the character has entered
/// a fixture-activity state it was not tapped in (the tweet, the fixture
/// action idle, the multiple-character timeline states, the communication
/// wait), so the taps land in the states whose ladder rows differ. It stands in for
/// the press itself, which a headless run at a few frames per second cannot
/// make (the injected release lands a frame later, past the long-press
/// threshold, so the gesture layer reports no tap). The request then takes
/// the dispatcher's own path: the stack check, the ladder, the session.
pub(crate) fn probe_stack_taps(
    button_state: Res<crate::action_button::ActionButtonState>,
    npcs: Query<(Entity, &CharacterUnitId, &crate::npc::NpcActions), Without<PlayerControlled>>,
    session: Option<Res<PlayerTalkSession>>,
    active_talk: Option<Res<crate::talk::ActiveTalk>>,
    time: Res<Time>,
    mut requests: MessageWriter<PlayerTalkRequest>,
    mut last: Local<Option<(f32, u32, crate::npc::NpcAction)>>,
) {
    use crate::npc::NpcAction;
    let now = time.elapsed_secs();
    if !crate::dev_tools::instrument_open("MOLY_PLAYER_TALK_STACK_TAP_SECS", now) {
        return;
    }
    if session.is_some() || active_talk.is_some() {
        return;
    }
    let Some((
        moly_law::action_button::ButtonType::Talk,
        moly_law::action_button::TargetId::Character(unit),
    )) = button_state.current()
    else {
        return;
    };
    let Some((entity, _, actions)) = npcs.iter().find(|(_, id, _)| id.0 == unit) else {
        return;
    };
    let action = actions.current;
    let activity = matches!(
        action,
        NpcAction::Tweet
            | NpcAction::FixtureActionIdle
            | NpcAction::MainSomeFixtureAction
            | NpcAction::SomeFixtureAction
            | NpcAction::Communication
    );
    let due = match *last {
        None => true,
        Some((at, last_unit, last_action)) => {
            now >= at + 15.0
                || (now >= at + 2.0 && activity && (last_unit, last_action) != (unit, action))
        }
    };
    if !due {
        return;
    }
    requests.write(PlayerTalkRequest {
        entity,
        unit,
        exact: None,
        target_fixture: None,
    });
    *last = Some((now, unit, action));
    info!(
        "[player-talk-smoke] stack tap: the Talk button's request for unit {unit} at {now:.1}s (state {action:?})"
    );
}
