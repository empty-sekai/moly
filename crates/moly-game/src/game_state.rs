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

/// `Sekai.Mysekai.GameStateType`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GameStateType {
    None = 0,
    Normal = 1,
    Edit = 2,
    Talk = 3,
    Harvest = 4,
    CutScene = 5,
    LearnSiteEnvironment = 6,
    SomeCharacterTalk = 7,
    PhotoShot = 8,
    Sketch = 9,
    SiteMove = 10,
    LevelUpMyRoomSite = 11,
    Delivery = 12,
}

impl GameStateType {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Normal => "Normal",
            Self::Edit => "Edit",
            Self::Talk => "Talk",
            Self::Harvest => "Harvest",
            Self::CutScene => "CutScene",
            Self::LearnSiteEnvironment => "LearnSiteEnvironment",
            Self::SomeCharacterTalk => "SomeCharacterTalk",
            Self::PhotoShot => "PhotoShot",
            Self::Sketch => "Sketch",
            Self::SiteMove => "SiteMove",
            Self::LevelUpMyRoomSite => "LevelUpMyRoomSite",
            Self::Delivery => "Delivery",
        }
    }

    /// The machine has a state object for the value (`GameStateMachine.SetUp`).
    fn has_state(self) -> bool {
        self != Self::None
    }

    /// The UI calls of the state's `OnEnter` / `OnExit`.
    pub(crate) fn screen_call(self) -> &'static str {
        match self {
            Self::Normal | Self::Harvest => {
                "OnEnter: MysekaiCommon.SetClickDetectorActive(true); OnExit: (false)"
            }
            Self::SiteMove => {
                "OnEnter: MysekaiCommon.EnableGestureLayer(false), ChangeUIScreen(MysekaiSiteMove), DisableTapScreen; OnExit: EnableGestureLayer(true), EnableTapScreen"
            }
            Self::LearnSiteEnvironment => {
                "OnEnter: MysekaiCommon.EnableGestureLayer(false); OnExit: (true)"
            }
            Self::CutScene => "OnEnter: PushUIScreen(MysekaiCutScene) unless active",
            Self::Sketch => "OnEnter: PushUIScreen(MysekaiSketchUI)",
            Self::PhotoShot => "OnEnter: PushUIScreen(MysekaiPhotoShot, bootArg)",
            Self::Edit => {
                "OnEnter: ShowGrid, HideTweets, FieldCamera FloorEdit; OnExit: HideGrid, ShowTweets"
            }
            Self::Talk
            | Self::SomeCharacterTalk
            | Self::LevelUpMyRoomSite
            | Self::Delivery
            | Self::None => "no screen call",
        }
    }
}

/// `GameStateManager`: the machine's current key.
#[derive(Resource, Debug, Default)]
pub(crate) struct GameStateManager {
    current: Option<GameStateType>,
}

impl GameStateManager {
    /// `CurrentType` (None before any change).
    pub(crate) fn current_type(&self) -> GameStateType {
        self.current.unwrap_or(GameStateType::None)
    }

    /// `ChangeState(type)`. Returns false when the machine is already there.
    pub(crate) fn change_state(&mut self, state: GameStateType, caller: &str) -> bool {
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
pub(crate) fn leave(world: &mut World, caller: &str) {
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
pub(crate) fn release_level_up(world: &mut World) {
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
        .get_resource::<crate::fixture_edit::EditSessionActive>()
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
pub(crate) fn scene_normal(
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

pub(crate) fn install(app: &mut App) {
    app.init_resource::<GameStateManager>()
        .add_systems(Update, (release_level_up, scene_normal));
}
