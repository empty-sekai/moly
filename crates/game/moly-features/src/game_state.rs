//! `GameStateManager`: the MySekai game state machine.
//!
//! The source (`Sekai.Mysekai.GameStateManager`, `GameStateMachine`,
//! `Sekai.FSM.StateMachine<GameStateType>`):
//! - `GameStateType` has 13 values, None 0 to Delivery 12. The machine is
//!   built with one state object for each of the twelve values from Normal
//!   (1) to Delivery (12); None has no state.
//! - `ChangeState(type)` does nothing when the machine's current key is
//!   already `type`; otherwise `ChangeTo(type)`: the current state's `OnExit`,
//!   the lookup (an unknown key logs "Not find state" and leaves no state),
//!   the current key set to `type`, the new state's `OnEnter`. It is
//!   synchronous.
//! - `CurrentType` is the machine's current key.
//! - Every state's `OnEnter` publishes `ChangeGameState`; the screen calls a
//!   state makes on entering are listed by [`GameStateType::screen_call`].
//!
//! The product's performances enter `LevelUpMyRoomSite` (11) and `CutScene`
//! (5) through [`enter`]; their readers stop what the source's
//! `ChangeGameState` subscribers stop: the joystick (its table disables on 5
//! and 11) and the keyboard stand-in that follows it. `CutScene` ends in
//! `CutScenePresenter.ResetCameraState` (`ChangeState(Normal)`).
//! `LevelUpMyRoomSite` has no exit of its own: the state holds until the next
//! `ChangeState`, which here is the next product game state that begins (a
//! site move, an edit session, a player talk, the harvest learning).
//!
//! The states' screen calls are not issued by this module: each is made by
//! the flow that owns the screen (the cut-scene presenter pushes and backs its
//! screen, the site move changes to and from its screen), and this module
//! names them in its log.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

pub use crate::field_modes::{GameStateType, HeldGameState};

/// `GameStateManager`: the machine's current key.
#[derive(Resource, Debug, Default)]
pub struct GameStateManager {
    current: Option<GameStateType>,
}

impl GameStateManager {
    /// `CurrentType` (None before any change).
    pub fn current_type(&self) -> GameStateType {
        self.current.unwrap_or(GameStateType::None)
    }

    /// `IsEnableClickBackKey`: `CurrentType < Edit`, so only `None` and
    /// `Normal` let a presenter's back-key click run.
    pub fn allows_click_back_key(&self) -> bool {
        (self.current_type() as u32) < (GameStateType::Edit as u32)
    }

    /// `ChangeState(type)`. Returns false when the machine is already there.
    pub fn change_state(&mut self, state: GameStateType, caller: &str) -> bool {
        if self.current == Some(state) {
            info!(
                "[game-state] {caller}: ChangeState({} {}): already the current state; nothing",
                state as i32,
                state.name()
            );
            return false;
        }
        let previous = self.current_type();
        if previous.has_state() {
            info!(
                "[game-state]   {} OnExit ({})",
                previous.name(),
                previous.screen_call()
            );
        }
        self.current = Some(state);
        if state.has_state() {
            info!(
                "[game-state] {caller}: ChangeTo({} {}) from {}: OnEnter publishes ChangeGameState ({}; issued by the flow that owns the screen)",
                state as i32,
                state.name(),
                previous.name(),
                state.screen_call()
            );
        } else {
            error!("[game-state] {caller}: Not find state = {}", state.name());
        }
        true
    }
}

pub use crate::field_modes::{PerformanceHolds, UiHold};

/// `GameStateManager.ChangeState(state)` by one of the performances.
pub fn enter(world: &mut World, state: GameStateType, caller: &str) {
    let previous = world.get_resource::<HeldGameState>().map(|held| held.0);
    world.insert_resource(HeldGameState(state));
    world
        .resource_mut::<GameStateManager>()
        .change_state(state, caller);
    let joystick = world
        .get_resource::<crate::joystick::JoystickState>()
        .map(|joystick| joystick.enabled);
    info!(
        "[game-state] {caller}: ChangeState({} {}) (held before: {:?}); publish ChangeGameState: the joystick's table disables it (joystick enabled this frame: {:?})",
        state as i32,
        state.name(),
        previous.map(GameStateType::name),
        joystick
    );
}

/// `ChangeState(Normal)`: the held state is left.
pub fn leave(world: &mut World, caller: &str) {
    leave_to(world, GameStateType::Normal, caller);
}

/// `ChangeState(next)` replacing the held state.
fn leave_to(world: &mut World, next: GameStateType, caller: &str) {
    if let Some(held) = world.remove_resource::<HeldGameState>() {
        info!(
            "[game-state] {caller}: {} {} left",
            held.0 as i32,
            held.0.name()
        );
        world
            .resource_mut::<GameStateManager>()
            .change_state(next, caller);
    }
}

/// Update: the next product game state that begins replaces a held
/// `LevelUpMyRoomSite`.
pub fn release_level_up(world: &mut World) {
    let Some(held) = world.get_resource::<HeldGameState>().copied() else {
        return;
    };
    if held.0 != GameStateType::LevelUpMyRoomSite {
        return;
    }
    let next = if world.contains_resource::<crate::site_move::SiteMoveActive>() {
        Some((
            GameStateType::SiteMove,
            "ChangeState(10 SiteMove) at the site move's admission",
        ))
    } else if world
        .get_resource::<crate::field_modes::EditSessionActive>()
        .is_some_and(|edit| edit.is_active())
    {
        Some((GameStateType::Edit, "ChangeState(2 Edit)"))
    } else if world.contains_resource::<crate::player_talk::PlayerTalkSession>() {
        Some((GameStateType::Talk, "ChangeState(3 Talk)"))
    } else if world.contains_resource::<crate::harvest::LearnSiteEnvironmentActive>() {
        Some((
            GameStateType::LearnSiteEnvironment,
            "ChangeState(6 LearnSiteEnvironment)",
        ))
    } else {
        None
    };
    if let Some((state, caller)) = next {
        leave_to(world, state, caller);
    }
}

/// Update: the scene's first state. `SceneMysekai` changes to Normal when the
/// field is ready; here, when the field screen first mounts.
pub fn scene_normal(
    screens: Res<crate::ui_layers::ScreenManager>,
    mut manager: ResMut<GameStateManager>,
) {
    if manager.current.is_none() && screens.on_field() {
        manager.change_state(
            GameStateType::Normal,
            "SceneMysekai (the field screen mounted)",
        );
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<GameStateManager>()
        .add_systems(Update, (release_level_up, scene_normal));
}
