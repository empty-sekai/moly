//! Which feature currently holds the field: the layout editor's session, a site
//! move, the entry sequence, held game states and the movement holds. The
//! features set these; the world's systems read them.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct EditSessionActive {
    pub active: bool,
    /// The editor holds changes the player has not saved. The editor
    /// publishes it after its commands and before the site switch reads it.
    pub unsaved_layout: bool,
}

impl EditSessionActive {
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[derive(Debug, Clone, Copy, Message)]
pub struct LayoutSaved;

/// `GameState.SiteMove`: present from the move's admission until the
/// executor sets Normal. Its readers stop what the source stops in this
/// state (manual movement and input, the action-button scan, site commands).
#[derive(Resource)]
pub struct SiteMoveActive;

/// Queued by `site::read_switch` for a switch between two different sites
/// (`OnChangeSite`); the executor answers it with `GetActionState`.
#[derive(Resource)]
pub struct SiteMoveRequest {
    pub from: String,
    pub next: crate::site::SiteSelection,
    /// The move admitted when this one ends: the source's nested
    /// `ChangeSiteEventData`, or the second leg of a product composition.
    pub then: Option<crate::site::SiteSelection>,
    /// Names the product composition this chain serves (`None` for a source
    /// move or the source's own nesting).
    pub composition: Option<&'static str>,
}

/// The Core's `ChangeEnvironment` has not run yet: the destination
/// environment is requested but its cross-fade does not start.
#[derive(Resource)]
pub struct EnvironmentHold;

/// On the player from the press's approach until the end action's Idle, and
/// again through the honor reward state: the player state machine owns the
/// avatar (states AutoMove, 8 and 9 with the intercept gate closed), so the
/// input moves nothing (`player::advance` skips the player).
#[derive(Component)]
pub struct DeliveryHold;

/// Movement hold of the join (`NavMeshAgent.enabled = false`,
/// `updatePosition = false` in `MoveToEntrance`, undone in `Finish`).
#[derive(Component)]
pub struct EntryHold;

/// `Sekai.Mysekai.GameStateType`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameStateType {
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
    pub fn name(self) -> &'static str {
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
    pub fn has_state(self) -> bool {
        self != Self::None
    }

    /// The UI calls of the state's `OnEnter` / `OnExit`.
    pub fn screen_call(self) -> &'static str {
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
pub struct HeldGameState(pub GameStateType);

/// On the player while the harvest AutoMove holds it: the AutoMove state owns
/// the agent and a Move request is dropped by the closed intercept gate, so
/// joystick input moves nothing (`player::advance` skips the player).
#[derive(Component)]
pub struct HarvestAutoMoveHeld {
    pub dash: bool,
}

/// GameState LearnSiteEnvironment (6) is current: the joystick is hidden and
/// the gesture layer is off.
#[derive(Resource)]
pub struct LearnSiteEnvironmentActive;

/// Holds the site loader until the rank change has set the housing
/// sites' displayed levels.
#[derive(Resource)]
pub struct LevelHold;

/// The entry's gates, published by the entry sequence so the world's input
/// routes read one small resource instead of the entry itself. An app without
/// an entry has every gate open, which is the default.
#[derive(Resource)]
pub struct EntryGates {
    /// `MysekaiUtility.EnableJoyStick` / `EnableGestureLayer`: false until the
    /// join's `Finish`.
    pub control_open: bool,
    /// The home HUD (`ScreenLayerMysekaiHome`) exists only from
    /// `HomeSiteController.OnFinishEnterAsync`.
    pub hud_open: bool,
    /// Site-change input: the back key is enabled after `JoinMysekai` returns.
    pub site_input_open: bool,
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
    pub fn control_open(&self) -> bool {
        self.control_open
    }

    /// The home HUD exists.
    pub fn hud_open(&self) -> bool {
        self.hud_open
    }

    /// Site-change input is enabled.
    pub fn site_input_open(&self) -> bool {
        self.site_input_open
    }
}

/// The content library's gates on the field. The library publishes them in
/// `PreUpdate`, after its input systems, and again in `Update`, after preview
/// admission and its deferred writes and before the balloon lifetime check.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryMode {
    pub reads_availability: bool,
    pub blocks_world_input: bool,
    pub owns_scene: bool,
    pub playback_camera: bool,
    /// The ticket of the talk preview the library owns.
    pub talk_preview: Option<u64>,
}

impl LibraryMode {
    /// The library reads the furniture availability cache.
    pub fn reads_availability(&self) -> bool {
        self.reads_availability
    }

    /// The library takes the field's world input.
    pub fn blocks_world_input(&self) -> bool {
        self.blocks_world_input
    }

    /// The library owns the scene until its rollback completes.
    pub fn owns_scene(&self) -> bool {
        self.owns_scene
    }

    /// The camera may inspect the library's playback.
    pub fn playback_camera_active(&self) -> bool {
        self.playback_camera
    }

    /// The library owns the talk preview of `ticket`.
    pub fn owns_talk_preview(&self, ticket: u64) -> bool {
        self.talk_preview == Some(ticket)
    }

}

/// GameState Delivery (12): set by the pre-action of a visit's first press,
/// gone with the site.
#[derive(Resource, Debug)]
pub struct DeliveryGameState;

/// A performance's hold on the field UI: `SetLayerCanvasGroup(UI,
/// interactable false)` and `RemoveCollisionSensorEvent`, until its
/// `AddCollisionSensorEvent` and `SetLayerCanvasGroup(UI, interactable
/// true)`. The name is the performance that holds it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct UiHold(pub &'static str);

/// What the field's input readers see of the performances.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PerformanceHolds<'w> {
    held: Option<Res<'w, HeldGameState>>,
    ui: Option<Res<'w, UiHold>>,
}

impl PerformanceHolds<'_> {
    /// `ObjectCollisionManager.IsCanUpdate` holds in Normal, Harvest and
    /// Sketch only, so a held CutScene or LevelUpMyRoomSite state stops the
    /// collision scan; a removed collision sensor event stops it as well.
    pub fn collision_stopped(&self) -> bool {
        self.held.is_some() || self.ui.is_some()
    }

    /// The UI canvas group is not interactable.
    pub fn ui_blocked(&self) -> bool {
        self.ui.is_some()
    }
}

/// Stages the content library's observer for an independent fixture talk:
/// `(world, talk id, player, final anchor, planned cast positions)`, true when
/// the observer moved. The library fills it; without the library nothing is
/// staged.
#[derive(Resource, Clone, Copy)]
pub struct FixtureTalkObserverPort(
    pub fn(&mut World, i32, Entity, Vec3, &[Vec3]) -> Result<bool, String>,
);

/// Runs [`FixtureTalkObserverPort`]; `Ok(false)` without one.
pub fn stage_fixture_talk_observer(
    world: &mut World,
    talk_id: i32,
    player: Entity,
    final_anchor: Vec3,
    planned_cast_positions: &[Vec3],
) -> Result<bool, String> {
    match world.get_resource::<FixtureTalkObserverPort>().copied() {
        Some(port) => (port.0)(world, talk_id, player, final_anchor, planned_cast_positions),
        None => Ok(false),
    }
}
