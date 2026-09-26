//! The `GameStateManager` states the expansion performances enter:
//! `LevelUpMyRoomSite` (11) and `CutScene` (5).
//!
//! The product has no game-state object; each performance holds its state
//! as this one resource, and its readers stop what the source's
//! `ChangeGameState` subscribers stop: the joystick (its table disables on
//! 5 and 11) and the keyboard stand-in that follows it.
//!
//! Leaving: `CutScene` ends in `CutScenePresenter.ResetCameraState`
//! (`ChangeState(Normal)`). `LevelUpMyRoomSite` has no exit of its own: the
//! current room's performance never returns the game to Normal, so the state
//! holds until the next `ChangeState` call. Here that is the next product
//! game state that begins (a site move, an edit session, a player talk, the
//! harvest learning), each of which the source enters through `ChangeState`.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// `GameStateType` values these performances enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GameStateType {
    CutScene = 5,
    LevelUpMyRoomSite = 11,
}

impl GameStateType {
    fn name(self) -> &'static str {
        match self {
            Self::CutScene => "CutScene",
            Self::LevelUpMyRoomSite => "LevelUpMyRoomSite",
        }
    }
}

/// The held state.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HeldGameState(pub(crate) GameStateType);

/// A performance's hold on the field UI: `SetLayerCanvasGroup(UI,
/// interactable false)` and `RemoveCollisionSensorEvent`, until its
/// `AddCollisionSensorEvent` and `SetLayerCanvasGroup(UI, interactable
/// true)`. The name is the performance that holds it.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct UiHold(pub(crate) &'static str);

/// What the field's input readers see of the performances.
#[derive(SystemParam)]
pub(crate) struct PerformanceHolds<'w> {
    held: Option<Res<'w, HeldGameState>>,
    ui: Option<Res<'w, UiHold>>,
}

impl PerformanceHolds<'_> {
    /// `ObjectCollisionManager.IsCanUpdate` holds in Normal, Harvest and
    /// Sketch only, so a held CutScene or LevelUpMyRoomSite state stops the
    /// collision scan; a removed collision sensor event stops it as well.
    pub(crate) fn collision_stopped(&self) -> bool {
        self.held.is_some() || self.ui.is_some()
    }

    /// The UI canvas group is not interactable.
    pub(crate) fn ui_blocked(&self) -> bool {
        self.ui.is_some()
    }
}

/// `GameStateManager.ChangeState(state)` by one of the performances.
pub(crate) fn enter(world: &mut World, state: GameStateType, caller: &str) {
    let previous = world.get_resource::<HeldGameState>().map(|held| held.0);
    world.insert_resource(HeldGameState(state));
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

/// `ChangeState(Normal)` or another state that replaces the held one.
pub(crate) fn leave(world: &mut World, caller: &str) {
    if let Some(held) = world.remove_resource::<HeldGameState>() {
        info!(
            "[game-state] {caller}: {} {} left",
            held.0 as i32,
            held.0.name()
        );
    }
}

/// Update: the next product game state that begins replaces a held
/// `LevelUpMyRoomSite`.
pub(crate) fn release_level_up(world: &mut World) {
    let Some(held) = world.get_resource::<HeldGameState>().copied() else {
        return;
    };
    if held.0 != GameStateType::LevelUpMyRoomSite {
        return;
    }
    let next = if world.contains_resource::<crate::site_move::SiteMoveActive>() {
        Some("ChangeState(10 SiteMove) at the site move's admission")
    } else if world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|edit| edit.is_active())
    {
        Some("ChangeState(2 Edit)")
    } else if world.contains_resource::<crate::player_talk::PlayerTalkSession>() {
        Some("ChangeState(3 Talk)")
    } else if world.contains_resource::<crate::harvest::LearnSiteEnvironmentActive>() {
        Some("ChangeState(6 LearnSiteEnvironment)")
    } else {
        None
    };
    if let Some(next) = next {
        leave(world, next);
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(Update, release_level_up);
}
