//! `ScreenManager`: the one consumer that opens screens and dialogs.
//!
//! Every screen and dialog of the field opens through this module, the way
//! the source opens them through `ScreenManager`: screens by
//! `AddScreen` / `ChangeUIScreen` / `PushUIScreen` / `BackUIScreen` /
//! `RemoveScreen`, dialogs by `ShowDialog` (`InstantiateDialog` + `Initialize`)
//! and the `DialogBase` open/close lifecycle. Views keep their own drawing and
//! follow the manager: a screen view draws while its layer is active, a dialog
//! view reports its animation ends back to the manager.
//!
//! Registry ([`registry`]): the 39 registered MySekai screen ids with the
//! region root's `ScreenLayerData`, and the dialog types with their
//! `Dialog/` prefabs.
//!
//! ## The source's order, rule by rule
//!
//! **Requests.** `PushUIScreen` and `ChangeUIScreen` return silently while a
//! transition holds the single coroutine slot (`changeUILayerCoroutine`), log
//! an error and return when the target is the current screen, then start the
//! transition in the slot. `BackUIScreen` logs a warning and returns while the
//! screen cover fades or the slot is busy; with `OnBackUIScreenOverride` set
//! it calls the override instead (and clears it when
//! `IsAutoClearBackUIScreenOverride`); else it starts the pop in the slot.
//! `ForceBackUIScreen` stops the running transition first and skips both
//! guards. `AddScreen` and `RemoveScreen` start their own coroutines, outside
//! the slot.
//!
//! **The exit gate.** Push, pop and change call the current screen's
//! `OnWillExit` and read its `WillExitBehaviour` each frame: None waits a
//! frame, Cancel rolls back (push and change restore `nextUI`, pop pushes the
//! popped entry back, change clears the boot object when asked). The base
//! `OnWillExit` answers OK at once. `RemoveScreen` runs the same gate inside
//! its exit; the three transitions pass `executeOnWillExit = false` to theirs.
//!
//! **Push.** Gate; the entry (current screen, its `GetStackObject`, the boot
//! argument of this push); `ExitScreen(current, isForward = true)` (awaited
//! only with `isWaitExitAnimation`); stack push; `prevUI = currentUI`,
//! `currentUI = target`; `AddScreenCore(target, bootArg, isForward = true)`;
//! `OnChangeUILayer(target)`; wait until the target plays; slot cleared.
//!
//! **Pop.** `IsPopUIScreen = true`; the stack pops; an empty stack ends the
//! pop with nothing changed. Gate; `ExitScreen(current, isForward = false)`;
//! `AddScreenCore(entry.screen, entry.bootArg, isForward = false,
//! entry.stackObject)`; `OnChangeUILayer`; wait for playing; slot cleared,
//! `IsPopUIScreen = false`.
//!
//! **Change.** As push with no stack change, the exit taking `isForward` and
//! the mount taking `nextScreenAnimationIsForward`.
//!
//! **AddScreenCore.** Not instantiated: instantiate (a failure ends it).
//! Already active: nothing. Else `SetActive(true)`, the layer stack object,
//! `SetAsLastSibling`, `ScreenInsertDirection`, then `OnScreenLayerBoot`:
//! `DisableTapScreen` unless the data enables tap animation, state Booting,
//! the animation types from the data, `HideComponentRoot`,
//! `ReceiveBootData` when a boot argument came, `OnBoot`; wait for the boot to
//! be done. Then the boot object is cleared, a backward mount reverses its
//! start animation, the layer BGM and background play, the header takes the
//! data (`HeaderUtility.SetDisplayHeader`), and `OnScreenLayerInitComponent`
//! (state InitComponent, `ShowComponentRoot`, `OnInitComponent`,
//! `PlayStartAnimation`) and `OnScreenLayerStart` (`OnScreenStart`) run.
//! `PlayStartAnimation` sets Starting and, with no animation, finishes at
//! once: state Playing, `OnFinishStartAnimation`, `EnableTapScreen`. So with
//! no animation the hooks run `OnInitComponent`, `OnFinishStartAnimation`,
//! `OnScreenStart`.
//!
//! **ExitScreen.** Not instantiated: nothing. A backward exit reverses its
//! exit animation; `OnScreenLayerExitStart` (`DisableTapScreen` unless tap
//! animation, state Exiting, `OnExitStart`, `PlayExitAnimation`); then
//! `SetActive(false)` and `OnScreenLayerExited` (state Exit, `OnExited`).
//! **ExitScene** calls `OnExitScene` on every instantiated layer and leaves
//! the stack as it is.
//!
//! **The stack** is `LimitedStack(16)`: a push over 16 entries drops the
//! oldest, a pop of an empty stack returns nothing.
//!
//! **Back key** (`BackKeyChecker.Check`: Escape, after the input manager's
//! interval gate). The topmost dialog takes it (`ExecuteBackKeyProcess`); else
//! the header's back button, when the header shows it and
//! `EnableBackUIScreen` holds, calls `BackUIScreen`; then the scene's handler
//! runs: in MySekai `OnExecuteMysekaiBackKeyProcess` while
//! `EnableMysekaiBackKeyProcess` holds. The field screen presenter adds
//! `MysekaiUtility.OnBackKey` to that handler outside the birthday context;
//! the scene enables it at its setup. `OnBackKey` leaves MySekai
//! (`TransitionToOutGame`) when the player is idle and owns the room. A
//! single player owns the room; the exit out of MySekai is not available in
//! this product and says so.
//!
//! ## Product mapping (read before changing)
//!
//! - Views have no screen transition animations yet, so every wait point
//!   passes in the frame it is reached, unless a view holds its layer
//!   ([`ScreenManager::hold`]); a transition started in a frame then ends in
//!   that frame, and the hook order above is kept.
//! - The site controllers: the product has one site module, so this module
//!   issues each controller's `ChangeUIScreen` of its base screen when a site
//!   becomes active with no screen current, and resolves a change to
//!   [`MenuScreenType::HomeField`] by the current site's category.
//! - The scene setup's `AddScreen` of the common, HUD and notice layers runs
//!   on the first frame the manager runs.
//! - The site map's view writes its own visibility (its toggle, its M key,
//!   its tap on the current site); the manager treats that view as the truth
//!   of its layer and reconciles once a frame: an open view the stack does not
//!   record is pushed, a closed view the stack still records is backed.
//! - The layout editor's back override (`ScreenLayerSiteEditMode` sets
//!   `OnBackUIScreenOverride`): while the editor is active a back request asks
//!   the editor to leave (it resolves its dirty-work dialog first); the
//!   editor's own Escape stays the editor's input while it is active.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::sitemap::SitemapRoot;

pub(crate) mod registry;

use registry::HeaderDisplay;
pub(crate) use registry::{DialogType, MenuScreenType, ScreenRegistry};

/// The earlier slot table's name for a screen id.
pub(crate) type LayerId = MenuScreenType;
/// The earlier name of the manager.
pub(crate) type UiLayerStack = ScreenManager;

// ---------------------------------------------------------------------------
// Display layers (`Sekai.DisplayLayerType`, `DisplayOrderInLayer`)
// ---------------------------------------------------------------------------

/// `Sekai.DisplayLayerType`. The unconstructed values are layers with no view
/// in this product yet.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DisplayLayerType {
    LayerBg = 0,
    LayerUi = 1,
    LayerHeader = 2,
    LayerScenario = 3,
    LayerDialog = 4,
    LayerLoading = 5,
    LayerScreenEffect = 6,
    LayerOverlay = 7,
}

#[allow(dead_code)]
impl DisplayLayerType {
    pub(crate) fn label(self) -> &'static str {
        match self {
            DisplayLayerType::LayerBg => "BG",
            DisplayLayerType::LayerUi => "UI",
            DisplayLayerType::LayerHeader => "Header",
            DisplayLayerType::LayerScenario => "Scenario",
            DisplayLayerType::LayerDialog => "Dialog",
            DisplayLayerType::LayerLoading => "Loading",
            DisplayLayerType::LayerScreenEffect => "ScreenEffect",
            DisplayLayerType::LayerOverlay => "Overlay",
        }
    }

    fn from_value(value: i64) -> Option<Self> {
        Some(match value {
            0 => Self::LayerBg,
            1 => Self::LayerUi,
            2 => Self::LayerHeader,
            3 => Self::LayerScenario,
            4 => Self::LayerDialog,
            5 => Self::LayerLoading,
            6 => Self::LayerScreenEffect,
            7 => Self::LayerOverlay,
            _ => return None,
        })
    }

    /// `DisplayOrderInLayer`: BG 0, UI 100, Dialog 150, Loading 200, Overlay
    /// 300; the other layers have no named order.
    pub(crate) fn order_in_layer(self) -> Option<i32> {
        match self {
            DisplayLayerType::LayerBg => Some(0),
            DisplayLayerType::LayerUi => Some(100),
            DisplayLayerType::LayerDialog => Some(150),
            DisplayLayerType::LayerLoading => Some(200),
            DisplayLayerType::LayerOverlay => Some(300),
            DisplayLayerType::LayerHeader
            | DisplayLayerType::LayerScenario
            | DisplayLayerType::LayerScreenEffect => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Layer state
// ---------------------------------------------------------------------------

/// `ScreenLayer.State`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenLayerState {
    None = 0,
    Booting = 1,
    InitComponent = 2,
    Starting = 3,
    Playing = 4,
    Exiting = 5,
    Exit = 6,
}

#[allow(dead_code)]
impl ScreenLayerState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ScreenLayerState::None => "None",
            ScreenLayerState::Booting => "Booting",
            ScreenLayerState::InitComponent => "InitComponent",
            ScreenLayerState::Starting => "Starting",
            ScreenLayerState::Playing => "Playing",
            ScreenLayerState::Exiting => "Exiting",
            ScreenLayerState::Exit => "Exit",
        }
    }
}

/// `ScreenLayer.WillExitBehaviourStatus`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WillExitBehaviour {
    None,
    Ok,
    Cancel,
}

/// The screen layer hooks, in the order the manager calls them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenHook {
    OnBoot,
    OnInitComponent,
    OnFinishStartAnimation,
    OnScreenStart,
    OnWillExit,
    OnExitStart,
    OnExited,
    OnExitScene,
}

/// One hook call on one screen layer, for the views that follow their layer.
#[allow(dead_code)]
#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct ScreenLayerEvent {
    pub(crate) screen: MenuScreenType,
    pub(crate) hook: ScreenHook,
}

/// A view's hold on its layer: the wait points it keeps open until it
/// releases them. No view holds a layer yet; the holds keep the source's wait
/// points where a view with an animation or a veto will stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayerHold {
    /// `OnWillExit` answers None until released.
    pub(crate) will_exit: bool,
    /// `OnBoot` does not set the boot done.
    pub(crate) boot: bool,
    /// The start animation runs until released.
    pub(crate) start: bool,
}

#[derive(Debug, Clone, Default)]
struct LayerRuntime {
    instantiated: bool,
    active: bool,
    state: Option<ScreenLayerState>,
    will_exit: Option<WillExitBehaviour>,
    boot_done: bool,
    hold: LayerHold,
    /// `LayerStackObject`.
    stack_object: Option<String>,
    /// `ScreenInsertDirection`: Back for a backward mount.
    inserted_back: bool,
}

/// One `uiScreenStack` entry: (layer, its stack object, a boot argument).
#[derive(Debug, Clone, PartialEq, Eq)]
struct StackEntry {
    screen: MenuScreenType,
    stack_object: Option<String>,
    boot_arg: Option<String>,
}

/// `CP.LimitedStack<T>`: a push beyond the limit removes the oldest entry.
#[derive(Debug, Clone)]
struct LimitedStack<T> {
    list: Vec<T>,
    max: usize,
}

impl<T> LimitedStack<T> {
    fn new(max: usize) -> Self {
        Self {
            list: Vec::with_capacity(max),
            max,
        }
    }

    fn push(&mut self, item: T) -> Option<T> {
        self.list.push(item);
        (self.list.len() > self.max).then(|| self.list.remove(0))
    }

    fn pop(&mut self) -> Option<T> {
        self.list.pop()
    }

    fn peek(&self) -> Option<&T> {
        self.list.last()
    }

    fn clear(&mut self) {
        self.list.clear();
    }

    fn len(&self) -> usize {
        self.list.len()
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.list.iter()
    }
}

/// `ScreenManager`'s stack capacity (`new LimitedStack<...>(16)` in `Awake`).
const UI_SCREEN_STACK_CAPACITY: usize = 16;

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TransitionKind {
    Push {
        boot_arg: Option<String>,
    },
    Change {
        boot_arg: Option<String>,
        is_forward: bool,
        next_forward: bool,
        clear_boot_on_cancel: bool,
    },
    Pop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionStage {
    /// The current screen's `OnWillExit` was called; its answer is read.
    Gate,
    /// The target's boot waits for its boot done.
    Boot,
    /// The target's start waits for Playing.
    Playing,
}

#[derive(Debug, Clone)]
struct Transition {
    kind: TransitionKind,
    target: Option<MenuScreenType>,
    tmp_next: Option<MenuScreenType>,
    entry: Option<StackEntry>,
    stage: TransitionStage,
    /// The mount's `isForward` (push true, pop false, change its
    /// `nextScreenAnimationIsForward`).
    mount_forward: bool,
    caller: String,
}

/// A mount running outside the slot (`AddScreen`) or inside a transition.
#[derive(Debug, Clone)]
struct Mount {
    screen: MenuScreenType,
    is_forward: bool,
    apply_header: bool,
}

/// `RemoveScreen`: the exit with `executeOnWillExit`.
#[derive(Debug, Clone)]
struct Removal {
    screen: MenuScreenType,
    gated: bool,
}

/// `OnBackUIScreenOverride`, by the screen that set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackOverride {
    /// `ScreenLayerSiteEditMode`: the layout editor's leave request.
    SiteEdit,
}

/// The header's state the back key reads (`HeaderUtility.SetDisplayHeader`).
#[derive(Debug, Clone, Copy, Default)]
struct HeaderState {
    displayed: bool,
    back_button_shown: bool,
    enable_back: bool,
}

// ---------------------------------------------------------------------------
// Dialogs (`DialogBase`)
// ---------------------------------------------------------------------------

/// `DialogState`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogState {
    Instantiated = 0,
    Initialized = 1,
    PlayOpenAnimation = 2,
    Show = 3,
    PlayCloseAnimation = 4,
    Closed = 5,
}

/// A shown dialog's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DialogId(u64);

/// What the dialog's class does on the hardware back key
/// (`OnHardwareBackKeyProcess`).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogBackKey {
    /// `DialogBase`'s body: nothing.
    Nothing,
    /// `SubWindowDialog`: `CloseProcess` then `Close`.
    Close,
}

#[derive(Debug, Clone)]
struct DialogInstance {
    id: DialogId,
    dialog: DialogType,
    layer: DisplayLayerType,
    state: DialogState,
    back_key: DialogBackKey,
    caller: String,
}

/// A back key the topmost dialog takes (`ExecuteBackKeyProcess`); the view of
/// that dialog answers it.
#[allow(dead_code)]
#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct DialogBackKeyEvent {
    pub(crate) id: DialogId,
    pub(crate) dialog: DialogType,
    pub(crate) back_key: DialogBackKey,
}

// ---------------------------------------------------------------------------
// The manager
// ---------------------------------------------------------------------------

/// `ScreenManager`'s screen and dialog state.
#[derive(Resource)]
pub(crate) struct ScreenManager {
    current: Option<MenuScreenType>,
    next: Option<MenuScreenType>,
    prev: Option<MenuScreenType>,
    stack: LimitedStack<StackEntry>,
    layers: HashMap<MenuScreenType, LayerRuntime>,
    /// Sibling order of the active layers (`SetAsLastSibling`).
    siblings: Vec<MenuScreenType>,
    transition: Option<Transition>,
    mounts: Vec<Mount>,
    removals: Vec<Removal>,
    back_override: Option<BackOverride>,
    auto_clear_back_override: bool,
    is_pop: bool,
    boot_arg_object: Option<String>,
    header: HeaderState,
    /// `DisableTapScreen` command objects on the UI layer.
    tap_disablers: Vec<MenuScreenType>,
    dialogs: Vec<DialogInstance>,
    next_dialog: u64,
    /// `EnableMysekaiBackKeyProcess`.
    mysekai_back_key_enabled: bool,
    /// `MysekaiUtility.OnBackKey` in `OnExecuteMysekaiBackKeyProcess`.
    mysekai_back_key_handler: bool,
    scene_set_up: bool,
    events: Vec<ScreenLayerEvent>,
    edges: Vec<String>,
}

impl Default for ScreenManager {
    fn default() -> Self {
        Self {
            current: None,
            next: None,
            prev: None,
            stack: LimitedStack::new(UI_SCREEN_STACK_CAPACITY),
            layers: HashMap::new(),
            siblings: Vec::new(),
            transition: None,
            mounts: Vec::new(),
            removals: Vec::new(),
            back_override: None,
            auto_clear_back_override: false,
            is_pop: false,
            boot_arg_object: None,
            header: HeaderState::default(),
            tap_disablers: Vec::new(),
            dialogs: Vec::new(),
            next_dialog: 1,
            mysekai_back_key_enabled: false,
            mysekai_back_key_handler: false,
            scene_set_up: false,
            events: Vec::new(),
            edges: Vec::new(),
        }
    }
}

/// The frame's inputs every step may read.
struct Ctx<'a> {
    registry: &'a mut ScreenRegistry,
}

impl ScreenManager {
    // ---- read side ----

    /// `currentUI`, or the site's field screen before any screen mounted.
    pub(crate) fn current(&self) -> MenuScreenType {
        self.current.unwrap_or(MenuScreenType::HomeField)
    }

    /// `GetCurrentUIScreenType`.
    #[allow(dead_code)]
    pub(crate) fn current_screen(&self) -> Option<MenuScreenType> {
        self.current
    }

    /// The current screen's layer state.
    #[allow(dead_code)]
    pub(crate) fn current_state(&self) -> ScreenLayerState {
        self.current
            .and_then(|screen| self.layers.get(&screen))
            .and_then(|layer| layer.state)
            .unwrap_or(ScreenLayerState::None)
    }

    /// The current screen is a site's field screen.
    pub(crate) fn on_field(&self) -> bool {
        self.current.is_some_and(MenuScreenType::is_field)
    }

    /// `IsActiveScreen`: instantiated and its GameObject active.
    pub(crate) fn is_active(&self, screen: MenuScreenType) -> bool {
        self.layers
            .get(&screen)
            .is_some_and(|layer| layer.instantiated && layer.active)
    }

    /// `IsUILayerWorking`: a transition holds the slot.
    #[allow(dead_code)]
    pub(crate) fn is_ui_layer_working(&self) -> bool {
        self.transition.is_some()
    }

    /// `GetScreenStackList`: bottom first.
    pub(crate) fn stack_list(&self) -> Vec<MenuScreenType> {
        self.stack.iter().map(|entry| entry.screen).collect()
    }

    /// `GetPeekScreenType`.
    #[allow(dead_code)]
    pub(crate) fn peek_screen(&self) -> Option<MenuScreenType> {
        self.stack.peek().map(|entry| entry.screen)
    }

    /// `GetPrevUIScreenType`.
    #[allow(dead_code)]
    pub(crate) fn prev_screen(&self) -> Option<MenuScreenType> {
        self.prev
    }

    /// `IsPopUIScreen`.
    #[allow(dead_code)]
    pub(crate) fn is_pop_ui_screen(&self) -> bool {
        self.is_pop
    }

    /// `CanTapByDisplayLayerType(Layer_UI)`: no command object disables taps.
    #[allow(dead_code)]
    pub(crate) fn can_tap_ui(&self) -> bool {
        self.tap_disablers.is_empty()
    }

    /// The active dialogs, oldest first.
    #[allow(dead_code)]
    pub(crate) fn dialogs(&self) -> impl Iterator<Item = (DialogId, DialogType, DialogState)> + '_ {
        self.dialogs
            .iter()
            .map(|dialog| (dialog.id, dialog.dialog, dialog.state))
    }

    /// `ExistsDialog(Layer_Dialog)`.
    #[allow(dead_code)]
    pub(crate) fn exists_dialog(&self) -> bool {
        self.dialogs
            .iter()
            .any(|dialog| dialog.layer == DisplayLayerType::LayerDialog)
    }

    /// A dialog's state.
    #[allow(dead_code)]
    pub(crate) fn dialog_state(&self, id: DialogId) -> Option<DialogState> {
        self.dialogs
            .iter()
            .find(|dialog| dialog.id == id)
            .map(|dialog| dialog.state)
    }

    // ---- view holds ----

    /// Set a view's hold on its layer; releasing a hold lets the waiting
    /// transition continue on its next step.
    #[allow(dead_code)]
    pub(crate) fn hold(&mut self, screen: MenuScreenType, hold: LayerHold) {
        self.layers.entry(screen).or_default().hold = hold;
    }

    /// `ScreenWillExitDone(status)`.
    #[allow(dead_code)]
    pub(crate) fn will_exit_done(&mut self, screen: MenuScreenType, status: WillExitBehaviour) {
        self.layers.entry(screen).or_default().will_exit = Some(status);
    }

    // ---- requests ----

    /// `PushUIScreen(screenType, bootArg, isWaitExitAnimation)`.
    pub(crate) fn push_ui_screen(
        &mut self,
        screen: MenuScreenType,
        boot_arg: Option<String>,
        caller: &str,
    ) {
        if self.transition.is_some() {
            info!(
                "[screen] {caller}: PushUIScreen({screen:?}) while a transition holds the slot: returns"
            );
            return;
        }
        if self.current == Some(screen) {
            error!("[screen] {caller}: PushUIScreen Same Screen not change. ({screen:?})");
            return;
        }
        self.edges.push(format!("{caller} Push {screen:?}"));
        info!(
            "[screen] {caller}: PushUIScreen({screen:?}, bootArg {boot_arg:?}): CheckModuleMaintenance allows; PushUIScreenCore in the slot"
        );
        self.transition = Some(Transition {
            kind: TransitionKind::Push { boot_arg },
            target: Some(screen),
            tmp_next: None,
            entry: None,
            stage: TransitionStage::Gate,
            mount_forward: true,
            caller: caller.to_owned(),
        });
        self.transition_begin();
    }

    /// `ChangeUIScreen(screenType, bootArg, isWaitExitAnimation, isForward,
    /// nextScreenAnimationIsForward, isClearBootDataOnExitCancel)`.
    pub(crate) fn change_ui_screen(
        &mut self,
        screen: MenuScreenType,
        boot_arg: Option<String>,
        is_forward: bool,
        next_forward: bool,
        caller: &str,
    ) {
        if self.transition.is_some() {
            info!(
                "[screen] {caller}: ChangeUIScreen({screen:?}) while a transition holds the slot: returns"
            );
            return;
        }
        if self.current == Some(screen) {
            error!("[screen] {caller}: ChangeUIScreen Same Screen not change.{screen:?}");
            return;
        }
        self.edges.push(format!("{caller} Change {screen:?}"));
        info!(
            "[screen] {caller}: ChangeUIScreen({screen:?}, isForward {is_forward}, next forward {next_forward}): ChangeUIScreenCore in the slot"
        );
        self.transition = Some(Transition {
            kind: TransitionKind::Change {
                boot_arg,
                is_forward,
                next_forward,
                clear_boot_on_cancel: false,
            },
            target: Some(screen),
            tmp_next: None,
            entry: None,
            stage: TransitionStage::Gate,
            mount_forward: true,
            caller: caller.to_owned(),
        });
        self.transition_begin();
    }

    /// `BackUIScreen(isWaitExitAnimation)`; `fading` is the screen cover's
    /// `IsPlayFading`.
    pub(crate) fn back_ui_screen(&mut self, fading: bool, caller: &str) -> Option<BackOverride> {
        if fading {
            warn!("[screen] {caller}: [BackUIScreen]Layer is fading");
            return None;
        }
        if self.transition.is_some() {
            warn!("[screen] {caller}: [BackUIScreen]Layer is working");
            return None;
        }
        self.back_or_override(caller)
    }

    /// `ForceBackUIScreen`: the running transition stops first.
    #[allow(dead_code)]
    pub(crate) fn force_back_ui_screen(&mut self, caller: &str) -> Option<BackOverride> {
        if let Some(transition) = self.transition.take() {
            info!(
                "[screen] {caller}: ForceBackUIScreen stops the running transition ({:?} to {:?})",
                transition.kind, transition.target
            );
        }
        self.back_or_override(caller)
    }

    fn back_or_override(&mut self, caller: &str) -> Option<BackOverride> {
        if let Some(back) = self.back_override {
            info!(
                "[screen] {caller}: BackUIScreen: OnBackUIScreenOverride ({back:?}) runs instead of the pop"
            );
            if self.auto_clear_back_override {
                self.back_override = None;
                info!("[screen]   IsAutoClearBackUIScreenOverride: the override is cleared");
            }
            return Some(back);
        }
        self.edges.push(format!("{caller} Back"));
        info!("[screen] {caller}: BackUIScreen: PopUIScreen in the slot");
        self.transition = Some(Transition {
            kind: TransitionKind::Pop,
            target: None,
            tmp_next: None,
            entry: None,
            stage: TransitionStage::Gate,
            mount_forward: true,
            caller: caller.to_owned(),
        });
        self.transition_begin();
        None
    }

    /// `OnBackUIScreenOverride = back` with `IsAutoClearBackUIScreenOverride`.
    pub(crate) fn set_back_override(
        &mut self,
        back: Option<BackOverride>,
        auto_clear: bool,
        caller: &str,
    ) {
        info!("[screen] {caller}: OnBackUIScreenOverride = {back:?}, IsAutoClear {auto_clear}");
        self.back_override = back;
        self.auto_clear_back_override = auto_clear;
    }

    /// `AddScreen(screenType)`: a coroutine outside the slot.
    pub(crate) fn add_screen(&mut self, screen: MenuScreenType, caller: &str) {
        self.edges.push(format!("{caller} Add {screen:?}"));
        info!("[screen] {caller}: AddScreen({screen:?})");
        self.mounts.push(Mount {
            screen,
            is_forward: true,
            apply_header: true,
        });
    }

    /// `RemoveScreen(screenType)`: `ExitScreen(layer, executeOnWillExit =
    /// true)` in its own coroutine.
    pub(crate) fn remove_screen(&mut self, screen: MenuScreenType, caller: &str) {
        self.edges.push(format!("{caller} Remove {screen:?}"));
        if !screen.registered() {
            error!("[screen] {caller}: removeScreen:Not Register ScreenLayer;{screen:?}");
            return;
        }
        info!("[screen] {caller}: RemoveScreen({screen:?})");
        self.removals.push(Removal {
            screen,
            gated: false,
        });
    }

    /// `ExitScene`.
    pub(crate) fn exit_scene(&mut self, events: &mut Vec<ScreenLayerEvent>, caller: &str) {
        info!(
            "[screen] {caller}: ExitScene: OnExitScene on every instantiated layer; the stack stays"
        );
        let mut screens: Vec<_> = self
            .layers
            .iter()
            .filter(|(_, layer)| layer.instantiated)
            .map(|(screen, _)| *screen)
            .collect();
        screens.sort();
        for screen in screens {
            info!("[screen]   {screen:?}: OnScreenLayerExitScene -> OnExitScene");
            events.push(ScreenLayerEvent {
                screen,
                hook: ScreenHook::OnExitScene,
            });
        }
        self.mysekai_back_key_enabled = false;
        info!("[screen]   SceneMysekai exit: EnableMysekaiBackKeyProcess = false");
    }

    /// `ClearScreenStack`.
    #[allow(dead_code)]
    pub(crate) fn clear_screen_stack(&mut self, caller: &str) {
        info!(
            "[screen] {caller}: ClearScreenStack ({} entries)",
            self.stack.len()
        );
        self.stack.clear();
    }

    // ---- dialogs ----

    /// `ShowDialog<T>(dialogType, layerType)`: `InstantiateDialog` loads
    /// `"Dialog/" + dialogType`; a missing prefab returns null. `Initialize`
    /// sets the state Initialized.
    pub(crate) fn show_dialog(
        &mut self,
        dialog: DialogType,
        layer: DisplayLayerType,
        back_key: DialogBackKey,
        caller: &str,
    ) -> Result<DialogId, String> {
        self.edges.push(format!("{caller} Dialog {dialog:?}"));
        let Some(prefab) = dialog.prefab() else {
            return Err(format!(
                "{caller}: ShowDialog({dialog:?}): Resources.Load has no prefab for this dialog type; InstantiateDialog returns null"
            ));
        };
        let id = DialogId(self.next_dialog);
        self.next_dialog += 1;
        info!(
            "[screen] {caller}: ShowDialog({dialog:?}, {}): InstantiateDialog loads {prefab} under the {} layer (state Instantiated), hooks OnOpenPreprocess/OnOpen/OnClose; Initialize: state Initialized",
            layer.label(),
            layer.label()
        );
        self.dialogs.push(DialogInstance {
            id,
            dialog,
            layer,
            state: DialogState::Initialized,
            back_key,
            caller: caller.to_owned(),
        });
        Ok(id)
    }

    fn dialog_mut(&mut self, id: DialogId, step: &str) -> &mut DialogInstance {
        self.dialogs
            .iter_mut()
            .find(|dialog| dialog.id == id)
            .unwrap_or_else(|| panic!("[screen] {step}: dialog {id:?} is not shown"))
    }

    /// `DialogBase.Open`: `OpenPreprocess` (`ToFront`, background on),
    /// `OpenAnimation`, state PlayOpenAnimation.
    pub(crate) fn open_dialog(&mut self, id: DialogId) {
        let index = self
            .dialogs
            .iter()
            .position(|dialog| dialog.id == id)
            .unwrap_or_else(|| panic!("[screen] Open: dialog {id:?} is not shown"));
        let dialog = self.dialogs.remove(index);
        self.dialogs.push(dialog);
        let dialog = self.dialog_mut(id, "Open");
        assert_eq!(
            dialog.state,
            DialogState::Initialized,
            "[screen] Open of {:?} from {:?}",
            dialog.dialog,
            dialog.state
        );
        dialog.state = DialogState::PlayOpenAnimation;
        info!(
            "[screen] {:?}: DialogBase.Open: OpenPreprocess (ToFront, SetActiveBackground(true), OnOpenPreprocess), OpenAnimation, state PlayOpenAnimation",
            dialog.dialog
        );
    }

    /// `OnFinishOpenAnimation`: state Show, `OnOpen`.
    pub(crate) fn dialog_open_finished(&mut self, id: DialogId) {
        let dialog = self.dialog_mut(id, "OnFinishOpenAnimation");
        assert_eq!(
            dialog.state,
            DialogState::PlayOpenAnimation,
            "[screen] open end of {:?} from {:?}",
            dialog.dialog,
            dialog.state
        );
        dialog.state = DialogState::Show;
        info!(
            "[screen] {:?}: OnFinishOpenAnimation: state Show, OnFinishOpenAnimationCallback (OnOpen)",
            dialog.dialog
        );
    }

    /// `DialogBase.Close`: state PlayCloseAnimation, `CloseAnimation`.
    pub(crate) fn close_dialog(&mut self, id: DialogId) {
        let dialog = self.dialog_mut(id, "Close");
        dialog.state = DialogState::PlayCloseAnimation;
        info!(
            "[screen] {:?}: DialogBase.Close: state PlayCloseAnimation, CloseAnimation",
            dialog.dialog
        );
    }

    /// `Destroy` after the close animation: `OnClose`, `OnClosed`, state
    /// Closed, the object destroyed.
    pub(crate) fn dialog_destroyed(&mut self, id: DialogId) {
        let dialog = self.dialog_mut(id, "Destroy");
        dialog.state = DialogState::Closed;
        info!(
            "[screen] {:?}: Destroy: DisableAllColliders, OnFinishCloseAnimationCallback (OnClose), OnClosed, state Closed; the dialog is destroyed (opened by {})",
            dialog.dialog, dialog.caller
        );
        self.dialogs.retain(|dialog| dialog.id != id);
    }

    // ---- the transition machine ----

    fn layer(&mut self, screen: MenuScreenType) -> &mut LayerRuntime {
        self.layers.entry(screen).or_default()
    }

    fn hook(&mut self, screen: MenuScreenType, hook: ScreenHook) {
        self.events.push(ScreenLayerEvent { screen, hook });
    }

    fn set_state(&mut self, screen: MenuScreenType, state: ScreenLayerState) {
        self.layer(screen).state = Some(state);
    }

    /// The first step of a transition, in the frame it starts.
    fn transition_begin(&mut self) {
        let Some(mut transition) = self.transition.take() else {
            return;
        };
        if let Some(target) = transition.target {
            if !target.registered() {
                error!(
                    "[screen] {}: Not Register ScreenLayer;{target:?}",
                    transition.caller
                );
                return;
            }
            transition.tmp_next = self.next;
            self.next = Some(target);
        } else {
            // PopUIScreen.
            self.is_pop = true;
            let Some(entry) = self.stack.pop() else {
                info!(
                    "[screen] {}: PopUIScreen: the stack is empty; nothing changes",
                    transition.caller
                );
                self.is_pop = false;
                return;
            };
            transition.tmp_next = self.next;
            self.next = Some(entry.screen);
            transition.entry = Some(entry);
        }
        if let Some(current) = self.current {
            self.call_will_exit(current);
        }
        self.transition = Some(transition);
    }

    fn call_will_exit(&mut self, screen: MenuScreenType) {
        let hold = self.layer(screen).hold.will_exit;
        let answer = if hold {
            WillExitBehaviour::None
        } else {
            WillExitBehaviour::Ok
        };
        self.layer(screen).will_exit = Some(answer);
        info!("[screen]   {screen:?}: OnWillExit -> WillExitBehaviour {answer:?}");
        self.hook(screen, ScreenHook::OnWillExit);
    }

    /// Step the slot's transition and the coroutines outside it.
    fn step(&mut self, ctx: &mut Ctx) {
        for _ in 0..4 {
            let before = (
                self.transition.as_ref().map(|t| t.stage),
                self.mounts.len(),
                self.removals.len(),
            );
            self.step_transition(ctx);
            self.step_mounts(ctx);
            self.step_removals(ctx);
            let after = (
                self.transition.as_ref().map(|t| t.stage),
                self.mounts.len(),
                self.removals.len(),
            );
            if before == after {
                break;
            }
        }
    }

    fn step_transition(&mut self, ctx: &mut Ctx) {
        let Some(mut transition) = self.transition.take() else {
            return;
        };
        loop {
            match transition.stage {
                TransitionStage::Gate => {
                    if let Some(current) = self.current {
                        match self
                            .layer(current)
                            .will_exit
                            .unwrap_or(WillExitBehaviour::Ok)
                        {
                            WillExitBehaviour::None => {
                                if self.layer(current).hold.will_exit {
                                    self.transition = Some(transition);
                                    return;
                                }
                                self.layer(current).will_exit = Some(WillExitBehaviour::Ok);
                                continue;
                            }
                            WillExitBehaviour::Cancel => {
                                self.next = transition.tmp_next;
                                match &transition.kind {
                                    TransitionKind::Pop => {
                                        let entry = transition.entry.take().expect("popped entry");
                                        info!(
                                            "[screen] {}: OnWillExit Cancel: the popped {:?} goes back on the stack",
                                            transition.caller, entry.screen
                                        );
                                        self.stack.push(entry);
                                    }
                                    TransitionKind::Change {
                                        clear_boot_on_cancel,
                                        ..
                                    } => {
                                        if *clear_boot_on_cancel {
                                            self.boot_arg_object = None;
                                        }
                                        info!(
                                            "[screen] {}: OnWillExit Cancel: the change is dropped",
                                            transition.caller
                                        );
                                    }
                                    TransitionKind::Push { .. } => {
                                        info!(
                                            "[screen] {}: OnWillExit Cancel: the push is dropped",
                                            transition.caller
                                        );
                                    }
                                }
                                return;
                            }
                            WillExitBehaviour::Ok => {}
                        }
                    }
                    let (target, boot_arg, exit_forward, mount_forward, stack_object) =
                        match &transition.kind {
                            TransitionKind::Push { boot_arg } => (
                                transition.target.expect("push target"),
                                boot_arg.clone(),
                                true,
                                true,
                                None,
                            ),
                            TransitionKind::Change {
                                boot_arg,
                                is_forward,
                                next_forward,
                                ..
                            } => (
                                transition.target.expect("change target"),
                                boot_arg.clone(),
                                *is_forward,
                                *next_forward,
                                None,
                            ),
                            TransitionKind::Pop => {
                                let entry = transition.entry.as_ref().expect("popped entry");
                                (
                                    entry.screen,
                                    entry.boot_arg.clone(),
                                    false,
                                    false,
                                    entry.stack_object.clone(),
                                )
                            }
                        };
                    let old = self.current;
                    if let Some(old) = old {
                        if let TransitionKind::Push { boot_arg } = &transition.kind {
                            // The entry: (currentUI, GetStackObject(), this push's boot argument).
                            let entry = StackEntry {
                                screen: old,
                                stack_object: self.layer(old).stack_object.clone(),
                                boot_arg: boot_arg.clone(),
                            };
                            self.exit_screen(ctx, old, exit_forward);
                            if let Some(dropped) = self.stack.push(entry) {
                                info!(
                                    "[screen]   LimitedStack(16) is full: the oldest entry {:?} is dropped",
                                    dropped.screen
                                );
                            }
                        } else {
                            self.exit_screen(ctx, old, exit_forward);
                        }
                    }
                    transition.mount_forward = mount_forward;
                    self.prev = old;
                    self.current = Some(target);
                    info!(
                        "[screen] {}: prevUI {old:?}, currentUI {target:?}; stack {:?}",
                        transition.caller,
                        self.stack_list()
                    );
                    if !self.mount_start(ctx, target, boot_arg, mount_forward, stack_object, true) {
                        info!(
                            "[screen] {}: the mount of {target:?} ended without playing",
                            transition.caller
                        );
                        self.transition = None;
                        return;
                    }
                    transition.stage = TransitionStage::Boot;
                }
                TransitionStage::Boot => {
                    let target = self.current.expect("mounting target");
                    if !self.layer(target).boot_done {
                        self.transition = Some(transition);
                        return;
                    }
                    self.mount_finish(ctx, target, transition.mount_forward, true);
                    info!(
                        "[screen] {}: OnChangeUILayer({target:?})",
                        transition.caller
                    );
                    transition.stage = TransitionStage::Playing;
                }
                TransitionStage::Playing => {
                    let target = self.current.expect("mounted target");
                    if self.layer(target).state != Some(ScreenLayerState::Playing) {
                        self.transition = Some(transition);
                        return;
                    }
                    if matches!(transition.kind, TransitionKind::Pop) {
                        self.is_pop = false;
                    }
                    info!(
                        "[screen] {}: {target:?} plays; the slot is free",
                        transition.caller
                    );
                    return;
                }
            }
        }
    }

    fn step_mounts(&mut self, ctx: &mut Ctx) {
        let mounts = std::mem::take(&mut self.mounts);
        let mut waiting = Vec::new();
        for mount in mounts {
            let layer = self.layer(mount.screen).clone();
            if layer.state == Some(ScreenLayerState::Booting) && !layer.boot_done {
                waiting.push(mount);
                continue;
            }
            if layer.state == Some(ScreenLayerState::Booting) {
                self.mount_finish(ctx, mount.screen, mount.is_forward, mount.apply_header);
                continue;
            }
            if self.mount_start(
                ctx,
                mount.screen,
                None,
                mount.is_forward,
                None,
                mount.apply_header,
            ) {
                if self.layer(mount.screen).boot_done {
                    self.mount_finish(ctx, mount.screen, mount.is_forward, mount.apply_header);
                } else {
                    waiting.push(mount);
                }
            }
        }
        self.mounts.extend(waiting);
    }

    fn step_removals(&mut self, ctx: &mut Ctx) {
        let removals = std::mem::take(&mut self.removals);
        let mut waiting = Vec::new();
        for mut removal in removals {
            if !self.layer(removal.screen).instantiated {
                info!(
                    "[screen]   RemoveScreen({:?}): the layer is not instantiated; nothing to exit",
                    removal.screen
                );
                continue;
            }
            if !removal.gated {
                removal.gated = true;
                self.call_will_exit(removal.screen);
            }
            match self
                .layer(removal.screen)
                .will_exit
                .unwrap_or(WillExitBehaviour::Ok)
            {
                WillExitBehaviour::None if self.layer(removal.screen).hold.will_exit => {
                    waiting.push(removal);
                    continue;
                }
                WillExitBehaviour::Cancel => {
                    info!(
                        "[screen]   RemoveScreen({:?}): OnWillExit Cancel; the layer stays",
                        removal.screen
                    );
                    continue;
                }
                _ => {}
            }
            self.exit_screen(ctx, removal.screen, true);
        }
        self.removals.extend(waiting);
    }

    /// `AddScreenCore` up to the boot wait. False when it ends there.
    fn mount_start(
        &mut self,
        ctx: &mut Ctx,
        screen: MenuScreenType,
        boot_arg: Option<String>,
        is_forward: bool,
        stack_object: Option<String>,
        _apply_header: bool,
    ) -> bool {
        if !screen.registered() {
            error!("[screen]   Not Register ScreenLayer;{screen:?}");
            return false;
        }
        let layer = self.layer(screen).clone();
        if !layer.instantiated {
            let prefab = ctx
                .registry
                .data(screen)
                .map(|data| data.prefab.clone())
                .unwrap_or_else(|| {
                    format!("Screen/Prefabs/{}", screen.data_asset().unwrap_or("-"))
                });
            info!("[screen]   {screen:?}: InstantiateLayerScreen ({prefab}), inactive");
            let layer = self.layer(screen);
            layer.instantiated = true;
            layer.active = false;
        } else if layer.active {
            info!("[screen]   {screen:?}: AddScreenCore: already active; nothing");
            return false;
        }
        let data = ctx.registry.data(screen).cloned();
        {
            let layer = self.layer(screen);
            layer.active = true;
            layer.stack_object = stack_object;
            layer.inserted_back = !is_forward;
            layer.boot_done = false;
        }
        self.siblings.retain(|sibling| *sibling != screen);
        self.siblings.push(screen);
        let tap_animation = data.as_ref().map(|data| data.enable_tap_screen_animation);
        if tap_animation == Some(false) {
            self.tap_disablers.push(screen);
        }
        self.set_state(screen, ScreenLayerState::Booting);
        info!(
            "[screen]   {screen:?}: SetActive(true), SetAsLastSibling, ScreenInsertDirection {}; OnScreenLayerBoot: {}state Booting, animations in {:?} out {:?}, HideComponentRoot{}, OnBoot",
            if is_forward { "Forward" } else { "Back" },
            match tap_animation {
                Some(false) => "DisableTapScreen, ",
                Some(true) => "",
                None => "(no data: EnableTapScreenAnimation unread) ",
            },
            data.as_ref().map(|d| d.start_animation),
            data.as_ref().map(|d| d.exit_animation),
            if boot_arg.is_some() {
                ", ReceiveBootData"
            } else {
                ""
            },
        );
        self.boot_arg_object = boot_arg;
        self.hook(screen, ScreenHook::OnBoot);
        let hold = self.layer(screen).hold.boot;
        self.layer(screen).boot_done = !hold;
        true
    }

    /// `AddScreenCore` after the boot: init component and start.
    fn mount_finish(
        &mut self,
        ctx: &mut Ctx,
        screen: MenuScreenType,
        is_forward: bool,
        apply_header: bool,
    ) {
        let data = ctx.registry.data(screen).cloned();
        self.boot_arg_object = None;
        let mut steps = Vec::new();
        if !is_forward {
            steps.push("ReverseStartAnimationType".to_owned());
        }
        match &data {
            Some(data) => {
                steps.push(format!(
                    "bgm Play({}), background Show({})",
                    data.bgm_type, data.background_type
                ));
                if apply_header {
                    self.set_display_header(screen, data);
                    steps.push(format!(
                        "SetDisplayHeader(header {:?}, back {:?}, enable back {})",
                        data.display_header, data.display_back, data.enable_back
                    ));
                }
            }
            None => steps.push("no data: bgm, background and header unread".to_owned()),
        }
        self.set_state(screen, ScreenLayerState::InitComponent);
        self.hook(screen, ScreenHook::OnInitComponent);
        self.set_state(screen, ScreenLayerState::Starting);
        let held = self.layer(screen).hold.start;
        if !held {
            self.finish_start(screen, data.as_ref().map(|d| d.enable_tap_screen_animation));
        }
        self.hook(screen, ScreenHook::OnScreenStart);
        info!(
            "[screen]   {screen:?}: {}; OnScreenLayerInitComponent: state InitComponent, ShowComponentRoot, OnInitComponent, PlayStartAnimation (state Starting{}); OnScreenLayerStart: OnScreenStart",
            steps.join(", "),
            if held {
                ", held by its view"
            } else {
                "; no animation: state Playing, OnFinishStartAnimation, EnableTapScreen"
            }
        );
        if screen == MenuScreenType::SiteEditMode {
            self.set_back_override(
                Some(BackOverride::SiteEdit),
                false,
                "ScreenLayerSiteEditMode",
            );
        }
        if screen.is_field() && !self.mysekai_back_key_handler {
            self.mysekai_back_key_handler = true;
            info!(
                "[screen]   {screen:?} presenter: OnExecuteMysekaiBackKeyProcess += MysekaiUtility.OnBackKey (not the birthday context)"
            );
        }
    }

    fn finish_start(&mut self, screen: MenuScreenType, tap_animation: Option<bool>) {
        self.set_state(screen, ScreenLayerState::Playing);
        self.hook(screen, ScreenHook::OnFinishStartAnimation);
        if tap_animation == Some(false) {
            self.tap_disablers.retain(|disabler| *disabler != screen);
        }
    }

    /// A view's start animation ended (`StartAnimationDone`).
    #[allow(dead_code)]
    pub(crate) fn start_animation_done(
        &mut self,
        screen: MenuScreenType,
        registry: &mut ScreenRegistry,
    ) {
        self.layer(screen).hold.start = false;
        if self.layer(screen).state == Some(ScreenLayerState::Starting) {
            let tap = registry.data(screen).map(|d| d.enable_tap_screen_animation);
            self.finish_start(screen, tap);
            info!(
                "[screen]   {screen:?}: StartAnimationDone: state Playing, OnFinishStartAnimation"
            );
        }
    }

    /// `ExitScreen` with `executeOnWillExit = false` (the gate already ran).
    fn exit_screen(&mut self, ctx: &mut Ctx, screen: MenuScreenType, is_forward: bool) {
        if !self.layer(screen).instantiated {
            return;
        }
        let data = ctx.registry.data(screen).cloned();
        let tap_animation = data.as_ref().map(|d| d.enable_tap_screen_animation);
        self.set_state(screen, ScreenLayerState::Exiting);
        self.hook(screen, ScreenHook::OnExitStart);
        {
            let layer = self.layer(screen);
            layer.active = false;
        }
        self.siblings.retain(|sibling| *sibling != screen);
        self.set_state(screen, ScreenLayerState::Exit);
        self.hook(screen, ScreenHook::OnExited);
        info!(
            "[screen]   {screen:?}: ExitScreen{}: OnScreenLayerExitStart ({}state Exiting, OnExitStart, PlayExitAnimation: none), SetActive(false), OnScreenLayerExited (state Exit, OnExited)",
            if is_forward {
                ""
            } else {
                " backward (ReverseExitAnimationType)"
            },
            match tap_animation {
                Some(false) => "DisableTapScreen then EnableTapScreen at the end, ",
                _ => "",
            },
        );
        if screen == MenuScreenType::SiteEditMode
            && self.back_override == Some(BackOverride::SiteEdit)
        {
            self.set_back_override(None, false, "ScreenLayerSiteEditMode exit");
        }
    }

    fn set_display_header(&mut self, _screen: MenuScreenType, data: &registry::ScreenLayerData) {
        if data.display_header == HeaderDisplay::Unrelated {
            return;
        }
        self.header.displayed = data.display_header.shows();
        if data.display_back != HeaderDisplay::Unrelated {
            self.header.back_button_shown = data.display_back.shows();
        }
        self.header.enable_back = data.enable_back;
    }

    /// `OnBackKey` after the dialog branch: the header's back button, when the
    /// header shows it and back is enabled.
    fn header_back_enabled(&self) -> bool {
        self.header.displayed && self.header.back_button_shown && self.header.enable_back
    }

    /// Recorded request edges (caller, verb, target), for the navigation account.
    #[allow(dead_code)]
    pub(crate) fn edges(&self) -> &[String] {
        &self.edges
    }
}

// ---------------------------------------------------------------------------
// Commands and systems
// ---------------------------------------------------------------------------

/// The product's request messages.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub(crate) enum LayerCommand {
    /// `PushUIScreen`.
    Push(MenuScreenType),
    /// `BackUIScreen`.
    Pop,
    /// `ExitScene`.
    ExitScene,
    /// `ChangeUIScreen`; a change to [`MenuScreenType::HomeField`] goes to the
    /// current site's field screen.
    Change(MenuScreenType),
    /// `AddScreen`.
    Add(MenuScreenType),
    /// `RemoveScreen`.
    Remove(MenuScreenType),
}

/// Update: the scene setup, the site controller, the view reconciliations,
/// the requests, then one step of the running coroutines.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: MessageReader<LayerCommand>,
    mut stack: ResMut<ScreenManager>,
    mut screen_registry: ResMut<ScreenRegistry>,
    mut sitemap_roots: Query<&mut Visibility, With<SitemapRoot>>,
    editor: Res<crate::fixture_edit::EditView>,
    mut editor_commands: MessageWriter<crate::fixture_edit::EditCommand>,
    site: Option<Res<crate::site::SiteActive>>,
    mut events: MessageWriter<ScreenLayerEvent>,
) {
    let stack = &mut *stack;
    let mut ctx = Ctx {
        registry: &mut *screen_registry,
    };
    if !stack.scene_set_up {
        stack.scene_set_up = true;
        let (total, live) = registry::screen_counts();
        let [mysekai, shared] = registry::dialog_counts();
        info!(
            "[screen] ScreenManager: {live} of {total} MySekai screens registered; dialog types {} MySekai ({} with a Dialog/ prefab) + {} shared ({} with a prefab); uiScreenStack LimitedStack({UI_SCREEN_STACK_CAPACITY})",
            mysekai.0, mysekai.1, shared.0, shared.1
        );
        for screen in [
            MenuScreenType::MysekaiCommon,
            MenuScreenType::MysekaiHUD,
            MenuScreenType::MysekaiNotice,
        ] {
            stack.add_screen(screen, "SceneMysekai setup");
        }
        stack.mysekai_back_key_enabled = true;
        info!("[screen] SceneMysekai setup: EnableMysekaiBackKeyProcess = true");
    }
    if stack.current.is_none() && stack.transition.is_none() {
        if let Some(site) = site.as_deref() {
            if let Some(field) = MenuScreenType::field_for_category(&site.category) {
                let caller = match field {
                    MenuScreenType::MysekaiMyRoom => "MyRoomSiteController",
                    MenuScreenType::MysekaiHarvest => "HarvestSiteController",
                    MenuScreenType::MysekaiDelivery => "DeliverySiteController",
                    _ => "HomeSiteController",
                };
                stack.change_ui_screen(field, None, true, true, caller);
                stack.step(&mut ctx);
            }
        }
    }
    // The layout editor's draft owner is authoritative for entry and exit.
    let editor_mounted = stack.current == Some(MenuScreenType::SiteEditMode)
        || stack
            .stack
            .iter()
            .any(|entry| entry.screen == MenuScreenType::SiteEditMode);
    if editor.active && !editor_mounted && stack.on_field() {
        stack.push_ui_screen(MenuScreenType::SiteEditMode, None, "SiteEditModeUtility");
    } else if !editor.active && stack.current == Some(MenuScreenType::SiteEditMode) {
        stack.set_back_override(None, false, "the layout editor leaves");
        stack.back_ui_screen(false, "SiteEditModeUtility (the editor left)");
    }
    stack.step(&mut ctx);
    // The site map view is the truth of its layer.
    let view_open = match sitemap_roots.single() {
        Ok(visible) => *visible != Visibility::Hidden,
        Err(_) => false,
    };
    let slot_open = stack.current == Some(MenuScreenType::MysekaiSiteMap);
    if view_open && !slot_open && stack.transition.is_none() {
        stack.push_ui_screen(
            MenuScreenType::MysekaiSiteMap,
            None,
            "MysekaiMenuUIContent (site map view opened)",
        );
    } else if !view_open && slot_open {
        stack.back_ui_screen(false, "ScreenLayerMysekaiSiteMap (site map view closed)");
    }
    stack.step(&mut ctx);
    for command in commands.read() {
        match command {
            LayerCommand::Push(screen) => stack.push_ui_screen(*screen, None, "LayerCommand::Push"),
            LayerCommand::Pop => {
                if let Some(BackOverride::SiteEdit) =
                    stack.back_ui_screen(false, "LayerCommand::Pop")
                {
                    if editor.active {
                        editor_commands.write(crate::fixture_edit::EditCommand::RequestExit);
                    }
                }
            }
            LayerCommand::Change(screen) => {
                let target = if *screen == MenuScreenType::HomeField {
                    match site.as_deref() {
                        Some(site) => MenuScreenType::field_for_category(&site.category)
                            .unwrap_or_else(|| {
                                panic!(
                                    "[screen] site category {} has no site controller screen",
                                    site.category
                                )
                            }),
                        None => {
                            error!(
                                "[screen] a change to the field screen with no active site: the home site's screen"
                            );
                            MenuScreenType::MysekaiHome
                        }
                    }
                } else {
                    *screen
                };
                stack.change_ui_screen(target, None, true, true, "LayerCommand::Change");
            }
            LayerCommand::ExitScene => {
                let mut exits = Vec::new();
                stack.exit_scene(&mut exits, "LayerCommand::ExitScene");
                stack.events.extend(exits);
            }
            LayerCommand::Add(screen) => stack.add_screen(*screen, "LayerCommand::Add"),
            LayerCommand::Remove(screen) => stack.remove_screen(*screen, "LayerCommand::Remove"),
        }
        // With no animation or hold every wait point passes in this frame, so
        // one request ends before the next is read.
        stack.step(&mut ctx);
    }
    stack.step(&mut ctx);
    for event in stack.events.drain(..) {
        events.write(event);
    }
    if let Ok(mut visible) = sitemap_roots.single_mut() {
        *visible = if stack.current == Some(MenuScreenType::MysekaiSiteMap) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Update: `BackKeyChecker.Check` and `ScreenManager.OnBackKey`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn back_key(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut input: ResMut<crate::menu_shell::SourceInputManager>,
    stack: Res<ScreenManager>,
    editor: Res<crate::fixture_edit::EditView>,
    player: Option<Res<crate::player_state::PlayerAvatarStates>>,
    mut dialog_keys: MessageWriter<DialogBackKeyEvent>,
    mut layer_commands: MessageWriter<LayerCommand>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    // The layout editor's own input reads Escape while it is active.
    if editor.active {
        return;
    }
    // `Time.realtimeSinceStartup`, read live at each call.
    let startup = time.startup();
    let mut clock = move || startup.elapsed().as_secs_f32();
    if input.0.check_and_reset_interval_time(
        moly_law::ui::custom_button::IntervalUseType::Use,
        &mut clock,
    ) {
        info!("[screen] back key: InputManager.CheckAndResetIntervalTime(Use) blocks it");
        return;
    }
    if let Some(dialog) = stack
        .dialogs
        .iter()
        .rev()
        .find(|dialog| dialog.state != DialogState::Closed)
    {
        info!(
            "[screen] back key: the topmost dialog {:?} takes it: ExecuteBackKeyProcess -> OnHardwareBackKeyProcess ({:?})",
            dialog.dialog, dialog.back_key
        );
        dialog_keys.write(DialogBackKeyEvent {
            id: dialog.id,
            dialog: dialog.dialog,
            back_key: dialog.back_key,
        });
        return;
    }
    if stack.header_back_enabled() {
        info!(
            "[screen] back key: the header back button of {:?} (EnableBackUIScreen): BackUIScreen",
            stack.current
        );
        layer_commands.write(LayerCommand::Pop);
    }
    if stack.mysekai_back_key_enabled && stack.mysekai_back_key_handler {
        let idle = player
            .as_deref()
            .is_some_and(|player| player.current == crate::player_state::PlayerActionState::Idle);
        if idle {
            info!(
                "[screen] back key: OnExecuteMysekaiBackKeyProcess -> MysekaiUtility.OnBackKey: the player is idle and owns the room -> TransitionToOutGame: leaving MySekai is not available in this product"
            );
        } else {
            info!(
                "[screen] back key: OnExecuteMysekaiBackKeyProcess -> MysekaiUtility.OnBackKey: the player is not idle; nothing"
            );
        }
    }
}

/// The manager's resources, messages and systems besides [`advance`], which
/// the schedule orders with the views.
pub(crate) fn install(app: &mut App) {
    app.add_message::<ScreenLayerEvent>()
        .add_message::<DialogBackKeyEvent>()
        .add_systems(
            Startup,
            |mut commands: Commands, server: Res<AssetServer>| {
                commands.insert_resource(ScreenRegistry::new(&server));
            },
        )
        .add_systems(Update, registry::load.before(advance))
        .add_systems(Update, back_key.before(advance));
}
