//! Which feature currently holds the field: the layout editor's session, a site
//! move, the entry sequence, held game states and the movement holds. The
//! features set these; the world's systems read them.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct EditSessionActive {
    pub(crate) active: bool,
}

impl EditSessionActive {
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[derive(Debug, Clone, Copy, Message)]
pub(crate) struct LayoutSaved;

/// `GameState.SiteMove`: present from the move's admission until the
/// executor sets Normal. Its readers stop what the source stops in this
/// state (manual movement and input, the action-button scan, site commands).
#[derive(Resource)]
pub(crate) struct SiteMoveActive;

/// Queued by `site::read_switch` for a switch between two different sites
/// (`OnChangeSite`); the executor answers it with `GetActionState`.
#[derive(Resource)]
pub(crate) struct SiteMoveRequest {
    pub(crate) from: String,
    pub(crate) next: crate::site::SiteSelection,
    /// The move admitted when this one ends: the source's nested
    /// `ChangeSiteEventData`, or the second leg of a product composition.
    pub(crate) then: Option<crate::site::SiteSelection>,
    /// Names the product composition this chain serves (`None` for a source
    /// move or the source's own nesting).
    pub(crate) composition: Option<&'static str>,
}

/// The Core's `ChangeEnvironment` has not run yet: the destination
/// environment is requested but its cross-fade does not start.
#[derive(Resource)]
pub(crate) struct EnvironmentHold;

/// On the player from the press's approach until the end action's Idle, and
/// again through the honor reward state: the player state machine owns the
/// avatar (states AutoMove, 8 and 9 with the intercept gate closed), so the
/// input moves nothing (`player::advance` skips the player).
#[derive(Component)]
pub(crate) struct DeliveryHold;

/// Movement hold of the join (`NavMeshAgent.enabled = false`,
/// `updatePosition = false` in `MoveToEntrance`, undone in `Finish`).
#[derive(Component)]
pub(crate) struct EntryHold;

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
    pub(crate) fn has_state(self) -> bool {
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

/// The held state.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HeldGameState(pub(crate) GameStateType);

/// On the player while the harvest AutoMove holds it: the AutoMove state owns
/// the agent and a Move request is dropped by the closed intercept gate, so
/// joystick input moves nothing (`player::advance` skips the player).
#[derive(Component)]
pub(crate) struct HarvestAutoMoveHeld {
    pub(crate) dash: bool,
}

/// GameState LearnSiteEnvironment (6) is current: the joystick is hidden and
/// the gesture layer is off.
#[derive(Resource)]
pub(crate) struct LearnSiteEnvironmentActive;

/// Holds the site loader until the rank change has set the housing
/// sites' displayed levels.
#[derive(Resource)]
pub(crate) struct LevelHold;

/// The entry's gates, published by the entry sequence so the world's input
/// routes read one small resource instead of the entry itself. An app without
/// an entry has every gate open, which is the default.
#[derive(Resource)]
pub(crate) struct EntryGates {
    /// `MysekaiUtility.EnableJoyStick` / `EnableGestureLayer`: false until the
    /// join's `Finish`.
    pub(crate) control_open: bool,
    /// The home HUD (`ScreenLayerMysekaiHome`) exists only from
    /// `HomeSiteController.OnFinishEnterAsync`.
    pub(crate) hud_open: bool,
    /// Site-change input: the back key is enabled after `JoinMysekai` returns.
    pub(crate) site_input_open: bool,
}

impl Default for EntryGates {
    fn default() -> Self {
        Self {
            control_open: true,
            hud_open: true,
            site_input_open: true,
        }
    }
}

impl EntryGates {
    /// The entry has opened movement and gestures.
    pub(crate) fn control_open(&self) -> bool {
        self.control_open
    }

    /// The home HUD exists.
    pub(crate) fn hud_open(&self) -> bool {
        self.hud_open
    }

    /// Site-change input is enabled.
    pub(crate) fn site_input_open(&self) -> bool {
        self.site_input_open
    }
}
