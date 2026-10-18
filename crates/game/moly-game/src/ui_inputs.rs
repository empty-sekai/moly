//! State and vocabulary the UI layer shares with the layers above it: the
//! screen manager's input state, the layout editor's published view, the
//! player's action state, the phenomenon catalogue and the UI render layers.
//! The world and the game write these; the UI reads them.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy::window::Window;
use moly_law::fixture::{Direction, GridPosition, Vector3Int};
use moly_law::ui::custom_button as button_rule;
use moly_layout::validate as tile_rules;

pub(crate) mod atlas;
mod fade;

pub(crate) use atlas::{ascii_or, BalloonArt, BAKE_PPEM, BALLOON_LAYER, SCREEN_UI_LAYER};
pub(crate) use fade::{ColorFade, FadeStage, UniTaskDelay};

use crate::canvas::RootCanvas;
use crate::ui_layout::{Pointer, UiLayouts, UiPrefabView};

/// The site map's render layer: apart from the balloon layer (1), so each
/// camera sees only its own half; the map's visibility moves this layer only.
pub(crate) const SITEMAP_LAYER: usize = 2;

/// OutQuad (`Ease.Basic` 6 in the source's `DG.Tweening.Ease` order):
/// `EaseManager.Evaluate` compiles it as `(t / d - 2) * -(t / d)`; `t` here is
/// that quotient, already clamped to [0, 1] as the tween's position is.
pub(crate) fn out_quad(t: f32) -> f32 {
    (t - 2.0) * -t
}

/// A window position (logical pixels, origin top-left) in root canvas units.
pub(crate) fn to_canvas(position: Vec2, window: &Window, root: &RootCanvas) -> Vec2 {
    Vec2::new(position.x - window.width() / 2., window.height() / 2. - position.y) / root.scale(window)
}

/// A window position (logical pixels, origin top-left) as the engine's
/// screen point (physical pixels, origin bottom-left).
pub(crate) fn to_screen_point(position: Vec2, window: &Window) -> [f32; 2] {
    let ratio = window.scale_factor();
    [position.x * ratio, window.physical_height() as f32 - position.y * ratio]
}

/// The event camera's ray through an engine screen point, as the raycast
/// pointer; None when the host canvas carries no camera pose. A point the
/// camera cannot see gets the engine's fallback ray and its error is logged.
pub(crate) fn screen_pointer(window: &Window, root: &RootCanvas, screen: [f32; 2]) -> Option<Pointer> {
    let pixels = [window.physical_width() as f32, window.physical_height() as f32];
    let event = root.event_ray(pixels, screen)?;
    if let Some(error) = &event.error {
        warn!("{error}");
    }
    Some(Pointer::Ray { ray: event.ray, root: event.root })
}

/// The raycast pointer of a window position: the event camera's ray, or the
/// point in canvas units when the host canvas carries no camera pose.
pub(crate) fn event_pointer(window: &Window, root: &RootCanvas, position: Vec2) -> Pointer {
    screen_pointer(window, root, to_screen_point(position, window))
        .unwrap_or_else(|| Pointer::Canvas(to_canvas(position, window, root)))
}

/// The press chain of the field camera reset button for one pointer: the
/// pointer, the selectable the source raycast gives a press there to,
/// whether the button's node is the pointer's enter target or an ancestor
/// of it, and the button's `IsActive()` and `IsInteractable()`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraResetHit {
    // The pointer and the target are read by the research instrument only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) pointer: Pointer,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) target: Option<(usize, i64)>,
    pub(crate) over_button: bool,
    pub(crate) hovered: bool,
    pub(crate) active: bool,
    pub(crate) interactable: bool,
}

/// The camera reset press chain for a pointer at a window position.
pub(crate) fn camera_reset_hit(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    window: &Window,
    root: &RootCanvas,
    position: Vec2,
    node: usize,
    button: i64,
) -> CameraResetHit {
    camera_reset_press(view, layouts, root.size(window), event_pointer(window, root, position), node, button)
}

/// The camera reset press chain for a raycast pointer over a root canvas of
/// `canvas` units.
pub(crate) fn camera_reset_press(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
    pointer: Pointer,
    node: usize,
    button: i64,
) -> CameraResetHit {
    let target = view.press_target(layouts, pointer, canvas);
    let hovered = view.hovers(layouts, pointer, canvas, node);
    let (active, interactable) = match view.selectable_live(layouts, canvas, node, button) {
        Some(live) => live,
        None => {
            // A canvas without area resolves and raycasts nothing, so the
            // pointer is over no button and these two are never read.
            assert!(target.is_none() && !hovered, "a canvas without area produced a raycast target");
            (false, false)
        }
    };
    CameraResetHit { pointer, target, over_button: target.is_some_and(|(_, id)| id == button), hovered, active, interactable }
}

/// The CustomButton behind `MysekaiMenuUIContent._cameraResetButton`: the
/// field holds the MySekai button wrapper, whose `_button` is the
/// CustomButton.
pub(crate) fn camera_reset_button(doc: &moly_assets::ui_layout::UiPrefab) -> Option<i64> {
    let content = doc.nodes.iter()
        .flat_map(|node| node.components.iter())
        .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuUIContent")?;
    let reference = content.fields["_cameraResetButton"].as_array()?;
    if reference.first()?.as_i64() != Some(0) {
        return None;
    }
    let id = reference.get(1)?.as_i64()?;
    let component = doc.nodes.iter().flat_map(|node| node.components.iter()).find(|c| c.path_id == id)?;
    if component.fields.get("se").is_some() {
        return Some(id);
    }
    let inner = component.fields["_button"].as_array()?;
    (inner.first()?.as_i64() == Some(0)).then(|| inner.get(1)?.as_i64()).flatten()
}



/// The layout key of the furniture editor's runtime prefab.
pub(crate) const EDITOR_RUNTIME_KEY: &str = "EditorRuntime";

/// Whether a point of a drawn prefab belongs to the furniture editor's
/// content: only the left panel and the selected HUD's measured content are
/// opaque regions. The full-screen editor root is deliberately not one.
pub(crate) fn captures_background(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    point: Vec2,
    canvas: Vec2,
) -> bool {
    view.key == EDITOR_RUNTIME_KEY
        && [
            "ContentRoot/ExpansionFixtureSelecter/BaseContent",
            "ContentRoot/FixtureEditHeadUpDisplay/Content",
        ]
        .iter()
        .any(|path| hit(view, layouts, path, point, canvas))
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

/// Whether this frame's tap was already taken by a screen button. The world
/// raycast reads it and casts no ray when it is set: the source's screen
/// buttons sit in the UI event system, ahead of the world ray.
#[derive(Resource, Default)]
pub(crate) struct ActionTapConsumed(pub(crate) bool);

/// The field shell's dialog state: which shell dialogs hold the field input.
#[derive(Resource, Default)]
pub(crate) struct ShellDialogState {
    pub(crate) leave_confirm: bool,
    pub(crate) menu_open: bool,
    /// The closing window still occupies the dialog input layer until its slide ends.
    pub(crate) menu_closing: bool,
    pub(crate) option_open: bool,
    pub(crate) get_resource_open: bool,
    /// The learn-phenomenon dialog, from its request to the end of its close animation.
    pub(crate) learn_phenomena_open: bool,
    pub(crate) leave_context: Option<LeaveContext>,
    /// The screen manager's instance of the leave confirm.
    pub(crate) leave_dialog: Option<crate::ui_layers::DialogId>,
}

impl ShellDialogState {
    pub(crate) fn blocks_field_input(&self) -> bool {
        self.leave_confirm || self.menu_open || self.menu_closing || self.option_open || self.get_resource_open
            || self.learn_phenomena_open
    }
}

/// The leave confirmation's owner and the join operation its OK runs.
pub(crate) struct LeaveContext {
    pub(crate) owner_name: String,
    pub(crate) on_confirm: SystemId,
}

/// The game's input manager state shared by source buttons.
#[derive(Resource, Default)]
pub(crate) struct SourceInputManager(pub(crate) button_rule::InputManager);

/// The site map's overlay root, whose visibility the screen manager sets.
#[derive(Component)]
pub(crate) struct SitemapRoot;

/// UI and platform input may request operations, never mutate draft fields.
#[derive(Message, Clone, Debug)]
pub(crate) enum EditCommand {
    Enter,
    /// A cell of the owned fixture list (`EditView::catalog`), by its index.
    SelectCatalog { index: usize },
    SelectPlaced { uid: String },
    SelectInventory { uid: String },
    /// A drag's touched tile (product frame, `GetSelectTile`): the drag puts
    /// the selection's source center there, clamped into the grid and
    /// raised by the column.
    MoveTo { center: GridPosition },
    /// The desktop arrow keys: one tile from the source center.
    Nudge { x: i8, z: i8 },
    Rotate,
    Decide,
    Cancel,
    ReturnToInventory,
    Save,
    RequestExit,
    KeepEditing,
    SaveAndExit,
    DiscardAndExit,
    /// The camera rotate button (`LayoutAction` 13).
    RotateCamera,
    /// The change-look button (`LayoutAction` 14).
    ChangeLookCamera,
    /// The remove-all button (`SiteEditView.OnRemoveFixtureAll`): opens the
    /// clean-up confirmation.
    RequestCleanUp,
    /// The confirmation's clean-up button (`CleanUpFixture`).
    CleanUpAll,
    /// The confirmation's cancel or close button, or a tap outside it.
    CancelCleanUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EditPhase {
    #[default]
    Idle,
    Browsing,
    Placing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditItemView {
    pub uid: String,
    pub piece: Option<u32>,
    pub package: String,
    pub texture_id: u32,
    pub fixture_id: i32,
    pub center: GridPosition,
    pub direction: Direction,
    pub grid_size: Vector3Int,
    pub layout: u8,
    pub editable: bool,
}

/// `SiteLayoutUtility.CanPutFloor` of the selection at its draft place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PutStatus {
    Ok,
    /// The first check it failed.
    Refused(tile_rules::Refusal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditSelectionView {
    pub item: EditItemView,
    pub from_inventory: bool,
    /// A new placement of an owned fixture.
    pub is_new: bool,
    /// The store (return to inventory) action is offered.
    pub can_clean_up: bool,
    pub put_status: PutStatus,
}

/// One owned fixture row of the list: `GetFixturePlaceableCount` above 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditCatalogView {
    pub index: usize,
    pub package: String,
    pub fixture_id: i32,
    pub texture_id: u32,
    pub grid_size: Vector3Int,
    /// `GetFixturePlaceableCount(id, texture)`: the owned quantity less the
    /// layouted count.
    pub placeable: i32,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditView {
    pub revision: u64,
    pub active: bool,
    pub phase: EditPhase,
    pub site_id: u32,
    pub site_type: String,
    pub dirty: bool,
    pub exit_dialog: bool,
    /// The remove-all confirmation is open.
    pub clean_up_dialog: bool,
    /// A save dialog (common dialog) is shown and takes the input.
    pub save_dialog: bool,
    pub selected: Option<EditSelectionView>,
    pub placed_rows: Vec<EditItemView>,
    pub inventory: Vec<EditItemView>,
    pub catalog: Vec<EditCatalogView>,
    /// The owned copy's revision the catalog was built from.
    pub owned_revision: u64,
    pub can_save: bool,
    pub feedback: String,
}

/// The player avatar's action state: the source's `PlayerActionState` values
/// this product writes and reads, 0 (Idle) through 34 (None).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerActionState {
    /// 0: idle. `InitializeStatus` starts the state machine here.
    #[default]
    Idle,
    /// 1: joystick movement (`MoveTo`'s non-dash branch).
    Move,
    /// 2: automatic pathfinding (`ChangeStateAutoMove`). This product enters
    /// it nowhere; the value is kept for the finish set, which reads
    /// {Move, AutoMove, Dash}.
    AutoMove,
    /// 6: talking (`ChangeAvatarsStatusToTalk`'s player branch).
    Talk,
    /// 7: harvesting (`HarvestPresenter`'s hit and finish branches).
    Harvest,
    /// 8: the birthday-party delivery (`PlayerAvatarDeliveryState`), entered
    /// by the delivery pre-action after the approach; the delivery closes
    /// the intercept gate for it and opens it at its end action.
    Delivery,
    /// 9: the honor reward (`PlayerAvatarDeliveryHonorRewardState`); its
    /// camera state and look-at are the delivery's.
    DeliveryHonorReward,
    /// 10: the player's one-second fixture switch interval.
    SwitchGimmick,
    /// 11: walking into the house (`PlayerAvatarEnterMoveHouseState`, the
    /// home-to-room move). Initialize closes the gate; at 2.0 s UpdateState
    /// opens it, hides the avatar and goes Idle.
    EnterMoveHouse,
    /// 13: leaving the house (`PlayerAvatarExitMoveHouseState`, the entry and
    /// the room-to-home move). Initialize closes the intercept gate; its
    /// UpdateState only counts time; the move's Finish opens the gate.
    ExitMoveHouse,
    /// 14: stepping into a room (`PlayerAvatarEnterMoveMyRoomState`).
    /// Initialize closes the gate; at 1.0 s UpdateState opens it and goes Idle.
    EnterMoveMyRoom,
    /// 15: walking out of a room (`PlayerAvatarExitMoveMyRoomState`).
    /// Initialize closes the gate; at 1.45 s UpdateState hides the avatar,
    /// opens the gate and goes Idle.
    ExitMoveMyRoom,
    /// 23: dash (`MoveTo`'s dash branch; the dash bit is toggled by the
    /// substitute key).
    Dash,
    /// 24: crafting at a workbench (`PlayerAvatarCraftMotionState`). The
    /// home action enters it with the gate open and closes the gate; its
    /// Initialize and Dispose run in `home_action` (the state's clip, facing
    /// the target fixture, then the idle clip at fade 0 and ClearAvatarItem).
    Craft,
    /// 25: painting a canvas (`PlayerAvatarDrawMotionState`), as Craft.
    Draw,
    /// 26: sketching (`PlayerAvatarSketchMotionState`): its Initialize also
    /// plays `se_sketch`; it does not turn the player.
    Sketch,
    /// 27: choosing a fixture to sketch (`PlayerAvatarSelectSketchItemState`):
    /// Initialize only clears the next state; UpdateState and Dispose are
    /// empty. The gate stays open: input moves the player out of it, and
    /// without input it stays (the input writer's finish set is {Move,
    /// AutoMove, Dash}).
    SelectSketchItem,
    /// 28: a performed fixture (`ChangeStateUseTimelineFixture`). The fixture
    /// session holds this state and the intercept gate, and releases both
    /// when it finishes or is cancelled.
    UseTimelineFixture,
    /// 30: leaving a harvest site by cannon. `Initialize` closes the
    /// intercept gate; `UpdateState` reopens it once 1.0 s has elapsed
    /// (`ANIMATION_TIME`). The cannon's `OnMoveFinish` (0.817 s after entry
    /// on a successful landing) is therefore dropped by the closed gate.
    ExitHarvestSite,
    /// 31: entering a harvest site by cannon. Its `Initialize` only shows the
    /// warp effect for other players' avatars; `UpdateState` is empty.
    EnterHarvestSite,
    /// 32: leaving the delivery site; same body as 30.
    ExitDeliverySite,
    /// 33: entering the delivery site; same body as 31.
    EnterDeliverySite,
    /// 34: none. The sentinel of the source's
    /// `PlayerAvatarStateBase.ClearNextState`.
    None,
}

/// The player avatar state machine's resident state (the source's
/// `currentStatus` and `PlayerStateActionData.CanIntercept`; the source's
/// `CurrentState` reader is this resource's `current` field).
#[derive(Resource, Debug)]
pub struct PlayerAvatarStates {
    /// The current action state. Starts Idle (the source's
    /// `InitializeStatus` writes 0).
    pub current: PlayerActionState,
    /// The intercept gate: while it is closed [`Self::change_status`] drops
    /// every request silently (the source's `ChangeStatus` third step). Open
    /// by default (the source opens it from the state constructor). The
    /// harvest and fixture sessions close and restore it around their own
    /// exclusive flows.
    pub(crate) can_intercept: bool,
    /// `PlayerAvatarStateBase.ElapsedTime` of the current state, for the
    /// states whose `UpdateState` reads it (the site exit states).
    pub(crate) state_elapsed: f32,
}

impl Default for PlayerAvatarStates {
    fn default() -> Self {
        Self {
            current: PlayerActionState::Idle,
            can_intercept: true,
            state_elapsed: 0.0,
        }
    }
}

impl PlayerAvatarStates {
    /// The product's `ChangeStatus(status)` (steps ①③⑤; the missing steps
    /// and their reasons are in `player_state`). A request the gate drops is
    /// not reported: the source returns silently.
    pub(crate) fn change_status(&mut self, status: PlayerActionState) {
        // ① Same-state early return: before the gate, as in the source.
        if self.current == status {
            return;
        }
        // ③ The intercept gate: while closed every change is dropped.
        if !self.can_intercept {
            return;
        }
        // ⑤ The change. The old state's disposal and the new state's
        // initialisation belong to their domains, not to this marker.
        info!(
            "[player-state] ChangeStatus {:?} -> {:?}",
            self.current, status
        );
        self.current = status;
        // New state's Initialize.
        self.state_elapsed = 0.0;
        if matches!(
            status,
            PlayerActionState::ExitHarvestSite
                | PlayerActionState::ExitDeliverySite
                | PlayerActionState::EnterMoveHouse
                | PlayerActionState::ExitMoveHouse
                | PlayerActionState::EnterMoveMyRoom
                | PlayerActionState::ExitMoveMyRoom
        ) {
            self.can_intercept = false;
        }
    }
}

/// Source identity, display metadata and resolved icon path for product UI.
#[derive(Clone, Debug)]
pub struct PhenomenonOption {
    pub id: i32,
    pub name: String,
    pub icon: Option<String>,
    pub icon_source: Option<moly_assets::weather_icons::WeatherIconArtifact>,
    pub metadata: Option<moly_assets::weather_index::PhenomenonMetadata>,
}

/// The phenomenon catalogue (id ascending, in run order), filled by the
/// weather chain in one resolution. The browser and the host UI only read it
/// to draw the weather button, so the UI cannot select a level that does not
/// exist; a bad request is only refused once.
#[derive(Resource, Default)]
pub struct PhenomenonCatalogue(pub Vec<PhenomenonOption>);
