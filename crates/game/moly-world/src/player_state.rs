//! Player avatar behaviour state machine (first batch: gate-dependent states).
//!
//! Skeleton (`PlayerAvatarStateMachine`):
//!
//! - **Every frame**: `UpdateController` first advances `OnUpdate` (calls
//!   `UpdateState` of the current state, then, if the state set its own
//!   `nextStatus`, performs one `ChangeStatus` for it), then takes the input
//!   vector: input present → `MoveTo` (write `InputVec`, then `ChangeStatus`, with
//!   target 23 when the dash bit is set and 1 otherwise); no input and current ∈
//!   {Move, AutoMove, Dash} → `OnMoveFinish` (= `ChangeStatus(Idle)`); no input and
//!   any other state → no write (the finishing flag is 0 only for those three
//!   states).
//! - **`ChangeStatus(status)` in five stages**: ① same state, early return;
//!   ② empty motion data, throw; ③ **intercept gate** — silently dropped when
//!   `CanIntercept` is false and this is the local avatar; ④ when leaving Move for
//!   anything other than Move/AutoMove, the NavMeshAgent stays at the current
//!   position; ⑤ Dispose the old state → swap `currentStatus` → Initialize the new
//!   state → multi-player broadcast for the local avatar.
//! - **The gate is open by default**: the `PlayerAvatarStateBase` constructor calls
//!   `SetInterceptFlag(true)` right after the motion data is attached — all 31
//!   state objects are built during Setup, so the gate is open as soon as the
//!   state machine is installed. Every exclusive flow follows the same step: **open
//!   gate, change state into the flow → close the gate → wait for the finish → open
//!   the gate → change state out** (harvest enter/exit, auto pathfinding and
//!   timeline-fixture playback all do this). A talk does not close this gate: what
//!   stops the player from moving is the input being taken away by the dialogue
//!   surface, not the state machine.
//!
//! The equivalent here (the same shape as `camera::FieldCameraState`: a resident
//! Resource, and the state is a passive marker — the engine's state objects are
//! markers too, and the behaviour lives in each domain's own coroutines):
//!
//! - [`PlayerAvatarStates`]: current state + intercept gate.
//! - [`PlayerAvatarStates::change_status`]: stages ①③⑤ of the five. Stage ②'s
//!   motion data is true by construction in a Resource form; stage ④'s agent
//!   parking has no counterpart — movement here is input-driven kinematics
//!   (`player::advance`), and yielding is handled by each domain's own hold (talk
//!   `TalkHold`, fixture sessions `PlayerFixtureHeld`; harvest see below), so the
//!   state machine does not add another layer; stage ⑤'s Dispose / Initialize and
//!   multi-player broadcast: states are markers, enter/exit behaviour belongs to
//!   each domain, and a single-player session has no broadcast.
//! - **No `nextStatus` channel**: none of the first batch's four states writes it
//!   in `UpdateState` (Idle only accumulates time; Harvest, UseTimelineFixture and
//!   Talk have empty bodies) — the channel is added when the first state that
//!   actually uses it joins the batch.
//!
//! Writers: the harvest action (`harvest::action`: `PlayHarvestAction` enters 7 and
//! closes the gate, the finish reopens it and returns to 0) · the talk session
//! ([`drive_from_talk`]) · input ([`drive_from_input`]), plus the enter/exit of
//! fixture sessions. While a fixture session holds its control lease, all three
//! writers in this module yield; the intercept gate is restored by that session on
//! its final release and must not be opened early by another domain's delayed
//! finish. Readers of the gate: the camera's close-range entry guard
//! (`camera::can_switch_to_fps`, which refuses in {Harvest, UseTimelineFixture}).

use bevy::prelude::*;

use crate::player::{DashMode, PlayerControlled, PlayerInput};
use crate::player_fixture_action::PlayerFixtureControlOwner;
use crate::player_talk::PlayerTalkSession;
use crate::talk::TalkHold;

pub use crate::ui_inputs::{PlayerActionState, PlayerAvatarStates};

/// Update, after the site-move executor (a state the move enters this frame
/// counts this frame's delta, as the state machine's Update runs after the
/// move's continuation): `UpdateState` of the four house and room states.
/// The avatar is hidden through the player's visibility; the move shows it
/// again.
pub fn update_house_states(
    time: Res<Time>,
    mut states: ResMut<PlayerAvatarStates>,
    mut players: Query<&mut Visibility, With<PlayerControlled>>,
) {
    use crate::door_law::{
        state_timer, ENTER_MOVE_HOUSE_TIME, ENTER_MOVE_MY_ROOM_TIME, EXIT_MOVE_MY_ROOM_TIME,
    };
    let current = states.current;
    let threshold = match current {
        PlayerActionState::EnterMoveHouse => ENTER_MOVE_HOUSE_TIME,
        PlayerActionState::EnterMoveMyRoom => ENTER_MOVE_MY_ROOM_TIME,
        PlayerActionState::ExitMoveMyRoom => EXIT_MOVE_MY_ROOM_TIME,
        // ExitMoveHouse only counts its time.
        PlayerActionState::ExitMoveHouse => f32::INFINITY,
        _ => return,
    };
    let (elapsed, act) = state_timer(states.state_elapsed, time.delta_secs(), threshold);
    states.state_elapsed = elapsed;
    if !act {
        return;
    }
    let hide = matches!(
        current,
        PlayerActionState::EnterMoveHouse | PlayerActionState::ExitMoveMyRoom
    );
    if hide {
        for mut visibility in &mut players {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
    states.can_intercept = true;
    info!(
        "[player-state] {current:?}: {elapsed:.4}s >= {threshold}: gate open{}, next state Idle",
        if hide { ", avatar hidden" } else { "" }
    );
    states.change_status(PlayerActionState::Idle);
}

/// Update: `UpdateState` of the two site exit states. `ElapsedTime` grows by
/// `Time.deltaTime`; once it is no longer below 1.0 (compared in double) a
/// closed gate reopens.
pub fn update_site_exit_states(time: Res<Time>, mut states: ResMut<PlayerAvatarStates>) {
    if !matches!(
        states.current,
        PlayerActionState::ExitHarvestSite | PlayerActionState::ExitDeliverySite
    ) {
        return;
    }
    let elapsed = states.state_elapsed + time.delta_secs();
    states.state_elapsed = elapsed;
    if !((elapsed as f64) < 1.0) && !states.can_intercept {
        states.can_intercept = true;
        info!("[player-state] {:?}: intercept gate reopened after {elapsed:.3}s", states.current);
    }
}

/// Talk writer: while a player talk session is playing ⇒ the player is in Talk.
///
/// Entry is `ChangeAvatarsStatusToTalk` (the player `ChangeState(6)`, participating
/// NPCs `ChangeState(4)` — the NPC side belongs to the talk domain and is not
/// repeated here). **There is no dedicated exit writer**: the Talk state remains
/// and the next `MoveTo`/`OnMoveFinish` carries the state away.
/// Equivalently, this writer only reasserts Talk while the session is playing (same
/// state, early return at ①); it writes nothing when the session ends, and waits for
/// the input writer to take over. The gate stays open during a session (a talk does
/// not close CanIntercept — what stops the player from moving is the input being
/// taken away by the dialogue surface, whose counterpart here is `TalkHold` on the
/// player, see the yielding in [`drive_from_input`]). A paired segment the player
/// joins by clicking also enters Talk; pure NPC performances and synthetic probes do
/// not change the player state.
pub fn drive_from_talk(
    session: Option<Res<PlayerTalkSession>>,
    pair: Option<Res<crate::talk::ActiveTalk>>,
    fixture_owner: Option<Res<PlayerFixtureControlOwner>>,
    mut states: ResMut<PlayerAvatarStates>,
) {
    if fixture_owner.is_some() {
        return;
    }
    if session.is_some() || pair.is_some_and(|talk| talk.includes_player()) {
        states.change_status(PlayerActionState::Talk);
    }
}

/// Input writer: the equivalent of `UpdateController`'s per-frame dispatch.
///
/// Input present → the tail of `MoveTo`: dash bit set ⇒ Dash, otherwise Move. No
/// input and current ∈ {Move, AutoMove, Dash} → `OnMoveFinish` (= Idle); no input
/// and any other state → no write.
///
/// Two cases where input yields (the engine takes the input away one level up in
/// `GetMoveDirection`, and the state machine only sees "no input"): while the
/// placement editor holds input; while the player participates in a talk
/// (`TalkHold` — the same marker `player::advance` uses to yield movement).
/// A fixture session holding the control lease does not consume movement input, but
/// does not clear it either; that session still observes click and joystick stop
/// requests, and the actual movement advances on its own lifecycle.
pub fn drive_from_input(
    edits: Res<crate::field_modes::EditSessionActive>,
    players: Query<(&PlayerInput, &DashMode, Option<&TalkHold>), With<PlayerControlled>>,
    fixture_owner: Option<Res<PlayerFixtureControlOwner>>,
    mut states: ResMut<PlayerAvatarStates>,
) {
    if fixture_owner.is_some() {
        return;
    }
    let held = players.single().ok();
    let is_input =
        !edits.is_active() && matches!(held, Some((input, _, None)) if input.active);
    if is_input {
        // `MoveTo` 尾部：dash 位决定目标态。
        let dash = matches!(held, Some((_, dash, _)) if dash.0);
        states.change_status(if dash {
            PlayerActionState::Dash
        } else {
            PlayerActionState::Move
        });
    } else if matches!(
        states.current,
        PlayerActionState::Move | PlayerActionState::AutoMove | PlayerActionState::Dash
    ) {
        states.change_status(PlayerActionState::Idle);
    }
}
