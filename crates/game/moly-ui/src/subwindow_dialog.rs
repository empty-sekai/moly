//! The sub-window dialogs' shared presentation: a `SubWindowDialog`'s window,
//! its fade, its close areas and its open sound, for any sub-window document a
//! caller shows from its own module.
//!
//! A sub-window dialog is a `DialogBase` whose window is a `SubWindowComponent`
//! animated by a `SubWindowFadeAnimation`:
//!
//! - **Opening.** `SubWindowDialog.Initialize(messageBody, onClose,
//!   allowCloseExternal)` gives the component the close callback, sizes its
//!   back panel, stores the caller's `onClose` and sets the message body;
//!   `DialogBase.Open` calls `OpenAnimation`, which starts
//!   `SubWindowDialog.ShowAsync`: the window's canvas group turns interactable
//!   and raycastable, the `SubWindowFadeAnimation` brings its canvas group's
//!   alpha to 1 over 0.2 s and plays the dialog's open SE (`DialogBase.openSE`
//!   through the SE table). When the fade ends the dialog reaches its shown
//!   state and `OnFinishOpenAnimation` runs: a dialog's own animation starts
//!   there ([`SubWindows::opened`]).
//! - **Closing.** `SubWindowComponent.SetCallback` makes every serialized
//!   close button run the component's callback (`CloseProcess`), which closes
//!   the dialog; `OnCloseExternal` and the hardware back key
//!   (`OnHardwareBackKeyProcess`) run `CloseProcess` too. `DialogBase.Close`
//!   calls `CloseAnimation`, which starts `SubWindowDialog.CloseAnimationAsync`:
//!   the window's canvas group stops being interactable and raycastable, the
//!   fade brings the alpha to 0 over 0.2 s, and the dialog is then destroyed.
//!   `SubWindowDialog.Close` runs the caller's `onClose` before that, so the
//!   caller learns of the close as it begins.
//!
//! The close buttons do not read `disableActionType`: `SubWindowComponent`'s
//! callback runs whatever that flag says. What a document's own fields change
//! is the rest of `OnCloseExternal`: `DialogBase.closeButton` (the dialog's own
//! close button) and a tap outside the window when the dialog needs the
//! background cover (`needBackgroundCover`). A dialog class that overrides
//! `OnCloseExternal` keeps it for its own use; its caller passes
//! `closes_externally: false` to [`open`] then.
//!
//! A dialog's own code may need its close held back (an animator dialog
//! disables its close button until its clip ends): it calls
//! [`request_close`] when it is done, and the close runs then.
//!
//! One sub-window is shown at a time; another [`open`] while one is shown is
//! refused by name.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::Value;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::gesture::{GestureEvent, GestureState};
use crate::ui_inputs::{ActionTapConsumed, SITEMAP_LAYER};
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The dialog's own component (`SubWindowDialog` or a class deriving from it).
const DIALOG: &str = "Sekai.SubWindowDialog";
/// The window component.
const COMPONENT: &str = "Sekai.SubWindowComponent";
/// The fade component.
const FADE: &str = "Sekai.SubWindowFadeAnimation";

/// `SubWindowFadeAnimation.OPEN_ANIMATION_DURATION`.
pub const OPEN_ANIMATION_DURATION: f32 = 0.2;
/// `SubWindowFadeAnimation.CLOSE_ANIMATION_DURATION`.
pub const CLOSE_ANIMATION_DURATION: f32 = 0.2;

/// `DialogBase.OpenSE`, in the enum's order; its last value plays nothing.
pub const OPEN_SE_TABLE: [&str; 8] = [
    "SE_UI_DIALOG_OPEN",
    "SE_REWARD_DIALOG_OPEN",
    "SE_UI_COSTUME_RESULT",
    "SE_RANKUP",
    "SE_UI_CHOICES_OPEN",
    "SE_SUBWINDOW_OPEN",
    "se_recycling",
    "se_get_blueprint",
];
/// `DialogBase.OpenSE.None`.
const OPEN_SE_NONE: i64 = 8;

/// The dialog slot's depth above the screens.
const DIALOG_Z: f32 = 100.0;

/// The SE a serialized `openSE` plays (`None` for `OpenSE.None` and for a
/// value outside the table).
pub fn open_se_name(open_se: i64) -> Option<&'static str> {
    if open_se < 0 || open_se == OPEN_SE_NONE {
        return None;
    }
    OPEN_SE_TABLE.get(usize::try_from(open_se).ok()?).copied()
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// What a sub-window document's own code reads, as view paths and component
/// identities.
#[derive(Clone, Debug)]
pub struct Bindings {
    /// The dialog component's class.
    pub class: String,
    /// `SubWindowDialog.animation`'s `canvasGroup`.
    pub fade_group: String,
    /// `SubWindowComponent.contentTransform`: the window a tap is inside or
    /// outside of.
    pub content: String,
    /// `SubWindowComponent.closeButtons`, as `@<id>` paths with their
    /// identities.
    pub close_buttons: Vec<(String, i64)>,
    /// `DialogBase.openSE`, as its cue (`None` when the dialog opens
    /// silently).
    pub open_se: Option<&'static str>,
    /// `DialogBase.closeButton`, when the document serializes it.
    pub close_button: Option<(String, i64)>,
    /// `DialogBase.needBackgroundCover`.
    pub need_background_cover: bool,
}

impl Bindings {
    /// Every close path the source's handlers read: the window's close
    /// buttons and the dialog's own close button.
    pub fn close_paths(&self) -> Vec<&str> {
        self.close_buttons
            .iter()
            .map(|(path, _)| path.as_str())
            .chain(self.close_button.iter().map(|(path, _)| path.as_str()))
            .collect()
    }
}

/// Read a sub-window document's bindings; `Err` names what the document
/// lacks, and its dialog then refuses to open by name.
pub fn bindings(document: &UiPrefab) -> Result<Bindings, String> {
    let dialog = document
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|component| {
            component.class == DIALOG || component.fields.get("subWindowComponent").is_some()
        })
        .ok_or("it has no sub-window dialog component")?;
    let fields = &dialog.fields;
    let window = component(document, pointer(&fields["subWindowComponent"], "subWindowComponent")?)?;
    if window.class != COMPONENT {
        return Err(format!(
            "its window is {} @{}, not {COMPONENT}",
            window.class, window.path_id
        ));
    }
    let close_buttons = window.fields["closeButtons"]
        .as_array()
        .ok_or("its window's closeButtons is not a list")?
        .iter()
        .map(|button| {
            let id = pointer(button, "closeButtons[]")?;
            Ok((node_path(document, id)?, id))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let content = node_path(
        document,
        pointer(&window.fields["contentTransform"], "contentTransform")?,
    )?;
    let animation = component(document, pointer(&fields["animation"], "animation")?)?;
    if animation.class != FADE {
        return Err(format!(
            "its animation is {} @{}, not {FADE}",
            animation.class, animation.path_id
        ));
    }
    let fade_group = node_path(
        document,
        pointer(&animation.fields["canvasGroup"], "animation.canvasGroup")?,
    )?;
    let open_se = match fields.get("openSE").and_then(Value::as_i64) {
        Some(cue) if cue != OPEN_SE_NONE && open_se_name(cue).is_none() => {
            return Err(format!("openSE {cue} is outside the SE table"));
        }
        Some(cue) => open_se_name(cue),
        None => None,
    };
    let close_button = match fields.get("closeButton") {
        Some(value) => {
            let id = pointer(value, "closeButton")?;
            Some((node_path(document, id)?, id))
        }
        None => None,
    };
    let need_background_cover = fields
        .get("needBackgroundCover")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Bindings {
        class: dialog.class.clone(),
        fade_group,
        content,
        close_buttons,
        open_se,
        close_button,
        need_background_cover,
    })
}

fn component(document: &UiPrefab, id: i64) -> Result<&UiComponent, String> {
    document
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|candidate| candidate.path_id == id)
        .ok_or_else(|| format!("component {id} is not in the document"))
}

fn node_path(document: &UiPrefab, id: i64) -> Result<String, String> {
    document
        .find(&format!("@{id}"))
        .map(|index| document.nodes[index].path.clone())
        .map_err(|_| format!("component {id} is not in the document"))
}

fn pointer(value: &Value, what: &str) -> Result<i64, String> {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .ok_or_else(|| format!("{what} is not a serialized reference"))?;
    if pair[0].as_i64() != Some(0) {
        return Err(format!("{what} points outside its prefab"));
    }
    pair[1]
        .as_i64()
        .filter(|id| *id != 0)
        .ok_or_else(|| format!("{what} is null"))
}

// ---------------------------------------------------------------------------
// One shown sub-window
// ---------------------------------------------------------------------------

/// A fade in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    /// DOTween's `timeRatio` of the 0.2 s tween, 0 to 1.
    pub progress: f32,
    /// `DOFade(0)`: a close; `DOFade(1)` otherwise: an open.
    pub closing: bool,
    /// The dialog was still opening when the close started.
    pub interrupted_open: bool,
}

/// One sub-window shown by this module.
#[derive(Clone, Debug)]
pub struct SubWindowShown {
    pub id: DialogId,
    /// The document's optional key.
    pub key: &'static str,
    pub layer: DisplayLayerType,
    pub bindings: Bindings,
    /// The fade in progress, while one runs; `None` once it has ended.
    pub fade: Option<Fade>,
    /// The open fade ended (`OnFinishOpenAnimation`).
    pub opened: bool,
    /// A close the dialog's own code asked for ([`request_close`]).
    pub close_requested: bool,
}

impl SubWindowShown {
    /// The alpha the fade draws: 0 before the open fade starts, 1 after it
    /// ends, and back to 0 on a close.
    pub fn alpha(&self) -> f32 {
        match self.fade {
            Some(fade) if fade.closing => {
                if fade.progress >= 1.0 {
                    0.0
                } else {
                    1.0 - ease(fade.progress)
                }
            }
            Some(fade) => ease(fade.progress),
            None if self.opened => 1.0,
            None => 0.0,
        }
    }

    /// Whether no fade is in progress.
    pub fn fade_finished(&self) -> bool {
        self.fade.is_none()
    }

    /// Whether a close is under way (its fade runs; the dialog is destroyed
    /// when it ends).
    pub fn closing(&self) -> bool {
        self.fade.is_some_and(|fade| fade.closing)
    }
}

/// DOTween's `EaseManager.Evaluate`: a tween created without an ease takes
/// the settings asset's default, `OutQuad`.
fn ease(time_ratio: f32) -> f32 {
    moly_law::ui::dotween::Ease::OutQuad.evaluate(time_ratio, 1.0)
}

/// The view of the shown sub-window: its document, its layer and the alpha
/// the fade draws into its canvas group.
#[derive(Component)]
pub struct SubWindowView {
    key: &'static str,
    layer: DisplayLayerType,
    fade_group: String,
    alpha: f32,
}

impl SubWindowView {
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// The dialog layer the view draws.
    pub fn layer(&self) -> DisplayLayerType {
        self.layer
    }

    /// The alpha the view drew last.
    pub fn alpha(&self) -> f32 {
        self.alpha
    }
}

/// The sub-window documents a root carries, by key, with their bindings (or
/// why they were refused), and the one shown now.
#[derive(Resource, Default)]
pub struct SubWindows {
    registered: Vec<(&'static str, Result<Bindings, String>)>,
    shown: Option<SubWindowShown>,
    view: Option<Entity>,
    /// The open dialog's `closes_externally`, from its open.
    closes_externally: bool,
}

impl SubWindows {
    pub fn shown(&self) -> Option<&SubWindowShown> {
        self.shown.as_ref()
    }

    /// Whether the open fade has ended (`OnFinishOpenAnimation` has run).
    pub fn opened(&self) -> bool {
        self.shown.as_ref().is_some_and(|shown| shown.opened)
    }

    /// The registered document's bindings; `None` for a key not registered.
    pub fn document(&self, key: &str) -> Option<&Result<Bindings, String>> {
        self.registered
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, bindings)| bindings)
    }
}

/// Registration: the documents' state and the view's spawn.
pub fn install(app: &mut App) {
    app.init_resource::<SubWindows>()
        .add_systems(Update, spawn_when_ready);
}

/// Registers the keys a caller shows (its optional documents): each key's
/// document is read from the layouts; a root that lacks it, or a document
/// this module cannot read, is remembered with the reason, so that the key's
/// dialog refuses by name.
pub fn register(world: &mut World, keys: &[(&'static str, &'static str)]) {
    let entries = {
        let Some(layouts) = world.get_resource::<UiLayouts>() else {
            return;
        };
        let known: Vec<&'static str> = world
            .resource::<SubWindows>()
            .registered
            .iter()
            .map(|(name, _)| *name)
            .collect();
        let mut entries: Vec<(&'static str, Result<Bindings, String>)> = Vec::new();
        let mut seen: Vec<&'static str> = Vec::new();
        for (key, _) in keys {
            if known.contains(key) || seen.contains(key) {
                continue;
            }
            let entry = match layouts.document(key) {
                Some(document) => match bindings(document) {
                    Ok(bindings) => {
                        info!(
                            "[subwindow] {key}: {} close buttons, open SE {:?}, background cover {}",
                            bindings.close_buttons.len(),
                            bindings.open_se,
                            bindings.need_background_cover
                        );
                        Ok(bindings)
                    }
                    Err(reason) => {
                        error!("[subwindow] {key}: {reason}; its dialog refuses to open by name");
                        Err(reason)
                    }
                },
                None => {
                    error!(
                        "[subwindow] {key}: this root carries no such document; its dialog refuses to open by name"
                    );
                    Err("this root carries no such document".to_owned())
                }
            };
            entries.push((*key, entry));
            seen.push(*key);
        }
        entries
    };
    world.resource_mut::<SubWindows>().registered.extend(entries);
}

// ---------------------------------------------------------------------------
// Opening and closing
// ---------------------------------------------------------------------------

/// The ways this module closes a sub-window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseSource {
    /// A tap at one of the window's close buttons (`CloseProcess`).
    CloseButton,
    /// `OnCloseExternal`: the dialog's own close button or outside the
    /// window.
    External,
    /// The hardware back key (`OnHardwareBackKeyProcess`).
    BackKey,
}

impl CloseSource {
    fn label(self) -> &'static str {
        match self {
            Self::CloseButton => "a tap at a close button (CloseProcess)",
            Self::External => "OnCloseExternal",
            Self::BackKey => "the back key (OnHardwareBackKeyProcess)",
        }
    }
}

/// Shows a registered sub-window document: `dialog` is its `DialogType`
/// (`ScreenManager::show_dialog`), `key` its optional document's key, and
/// `closes_externally` whether its class closes on `OnCloseExternal` (a class
/// that overrides it passes `false`). `None` when the document is not
/// registered or was refused, or while another sub-window is shown; the
/// reason is logged.
pub fn open(
    world: &mut World,
    dialog: DialogType,
    key: &'static str,
    back_key: DialogBackKey,
    caller: &'static str,
    closes_externally: bool,
) -> Option<DialogId> {
    let bindings = {
        let subwindows = world.resource::<SubWindows>();
        if let Some(shown) = &subwindows.shown {
            error!(
                "[subwindow] {caller}: {key} is refused while {} ({:?}) is shown",
                shown.key, shown.id
            );
            return None;
        }
        match subwindows.document(key) {
            Some(Ok(bindings)) => bindings.clone(),
            Some(Err(reason)) => {
                error!("[subwindow] {caller}: {key} refuses to open: {reason}");
                return None;
            }
            None => {
                error!("[subwindow] {caller}: {key} is not registered");
                return None;
            }
        }
    };
    let opened = {
        let mut screens = world.resource_mut::<ScreenManager>();
        match screens.show_dialog(dialog, DisplayLayerType::LayerDialog, back_key, caller) {
            Ok(id) => {
                screens.open_dialog(id);
                id
            }
            Err(reason) => {
                error!("[subwindow] {caller}: {reason}");
                return None;
            }
        }
    };
    info!(
        "[subwindow] {caller}: {key} ({opened:?}) opens: alpha 0 to 1 over {OPEN_ANIMATION_DURATION} s, SE {:?}, closes externally: {closes_externally}",
        bindings.open_se
    );
    let mut subwindows = world.resource_mut::<SubWindows>();
    subwindows.view = None;
    subwindows.closes_externally = closes_externally;
    subwindows.shown = Some(SubWindowShown {
        id: opened,
        key,
        layer: DisplayLayerType::LayerDialog,
        bindings,
        fade: Some(Fade {
            progress: 0.0,
            closing: false,
            interrupted_open: false,
        }),
        opened: false,
        close_requested: false,
    });
    Some(opened)
}

/// The dialog's own code asks for its close (`SubWindowDialog.Close` at the
/// end of its own animation): it starts once the open fade has ended. A
/// close already under way keeps its request. `true` when the close started.
pub fn request_close(world: &mut World) -> bool {
    let mut subwindows = world.resource_mut::<SubWindows>();
    let Some(shown) = subwindows.shown.as_mut() else {
        return false;
    };
    shown.close_requested = true;
    start_requested(&mut subwindows)
}

/// Starts a requested close once no fade holds the dialog back.
fn start_requested(subwindows: &mut SubWindows) -> bool {
    let Some(shown) = subwindows.shown.as_mut() else {
        return false;
    };
    if !shown.close_requested || !shown.fade_finished() {
        return false;
    }
    close(shown, CloseSource::External);
    true
}

/// Starts the close: the caller's `onClose` runs, the slot closes, and the
/// window's canvas group stops being live for the fade's length.
fn close(shown: &mut SubWindowShown, source: CloseSource) {
    info!(
        "[subwindow] {} ({:?}): closed by {}; the caller's onClose, alpha to 0 over {CLOSE_ANIMATION_DURATION} s",
        shown.key,
        shown.id,
        source.label()
    );
    shown.close_requested = false;
    match shown.fade.as_mut() {
        Some(fade) if fade.closing => {}
        Some(fade) => {
            fade.closing = true;
            fade.progress = 0.0;
            fade.interrupted_open = !shown.opened;
        }
        None => {
            shown.fade = Some(Fade {
                progress: 0.0,
                closing: true,
                interrupted_open: false,
            });
        }
    }
}

/// Whether the shown dialog's close fade has ended (it is destroyed then).
fn closing_ended(subwindows: &SubWindows) -> bool {
    subwindows
        .shown
        .as_ref()
        .is_some_and(|shown| shown.fade.is_some_and(|fade| fade.closing && fade.progress >= 1.0))
}

/// Destroys the shown dialog whose close fade ended: `ScreenManager` reaches
/// `Destroy` after the close animation.
fn finish(subwindows: &mut SubWindows, screens: &mut ScreenManager) -> bool {
    if !closing_ended(subwindows) {
        return false;
    }
    if let Some(shown) = subwindows.shown.take() {
        screens.dialog_destroyed(shown.id);
    }
    true
}

/// Spawns the shown document's view once its images are in, and drops the
/// view of a dialog no longer shown.
pub fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut subwindows: ResMut<SubWindows>,
    views: Query<&SubWindowView>,
) {
    let Some(shown) = subwindows.shown.as_ref() else {
        if let Some(entity) = subwindows.view.take() {
            if views.get(entity).is_ok() {
                commands.entity(entity).despawn();
            }
        }
        return;
    };
    if subwindows.view.is_some() || !layouts.ready(shown.key, &server) {
        return;
    }
    let key = shown.key;
    let mut view = UiPrefabView::new(key, SITEMAP_LAYER);
    view.set_alpha(&shown.bindings.fade_group, shown.alpha());
    let entity = commands
        .spawn((
            SubWindowView {
                key,
                layer: shown.layer,
                fade_group: shown.bindings.fade_group.clone(),
                alpha: shown.alpha(),
            },
            Visibility::Inherited,
            Transform::from_xyz(0., 0., DIALOG_Z),
            RenderLayers::layer(SITEMAP_LAYER),
            view,
        ))
        .id();
    subwindows.view = Some(entity);
}

/// Advances the shown sub-window's fade by this frame's delta, draws its
/// alpha, and finishes what the fade ends: the open reaches `Show`
/// (`dialog_open_finished`, where `OnFinishOpenAnimation` runs) with its SE;
/// the close destroys the dialog.
#[allow(clippy::too_many_arguments)]
pub fn fade(
    mut commands: Commands,
    time: Res<Time>,
    mut subwindows: ResMut<SubWindows>,
    mut screens: ResMut<ScreenManager>,
    mut views: Query<(&mut SubWindowView, &mut UiPrefabView)>,
    mut sounds: ResMut<SeRequests>,
) {
    let dt = time.delta_secs();
    let subwindows = &mut *subwindows;
    let Some(shown) = subwindows.shown.as_mut() else {
        return;
    };
    let mut opened_now: Option<(&'static str, Option<&'static str>)> = None;
    if let Some(fade) = shown.fade.as_mut() {
        let duration = if fade.closing {
            CLOSE_ANIMATION_DURATION
        } else {
            OPEN_ANIMATION_DURATION
        };
        fade.progress = (fade.progress + dt / duration).min(1.0);
        if fade.progress >= 1.0 && fade.closing {
            let key = shown.key;
            if let Some(closed) = subwindows.shown.take() {
                screens.dialog_destroyed(closed.id);
                info!(
                    "[subwindow] {key} ({:?}): the close fade ended; the dialog is destroyed",
                    closed.id
                );
            }
            if let Some(entity) = subwindows.view.take() {
                commands.entity(entity).despawn();
            }
            return;
        }
        if fade.progress >= 1.0 {
            shown.fade = None;
            shown.opened = true;
            opened_now = Some((shown.key, shown.bindings.open_se));
        }
    }
    if let Some((key, cue)) = opened_now {
        if let Some(cue) = cue {
            sounds.0.push(SeRequest {
                owner: None,
                cue: cue.to_owned(),
                class: SeClass::Ui,
                source: "subwindow_dialog",
            });
        }
        screens.dialog_open_finished(shown.id);
        info!("[subwindow] {key} ({:?}): the open fade ended (state Show, OnFinishOpenAnimation), SE {cue:?}", shown.id);
    }
    let alpha = shown.alpha();
    let fade_group = shown.bindings.fade_group.clone();
    let Some(entity) = subwindows.view else {
        return;
    };
    if let Ok((mut view, mut prefab)) = views.get_mut(entity) {
        view.alpha = alpha;
        prefab.set_alpha(&fade_group, alpha);
    }
}

// ---------------------------------------------------------------------------
// Taps and the back key
// ---------------------------------------------------------------------------

/// The view's pointer in its canvas units.
fn canvas_point(window: &Window, scale: f32, position: Vec2) -> Vec2 {
    Vec2::new(position.x - window.width() / 2.0, window.height() / 2.0 - position.y) / scale
}

/// A selectable of the view at `path` with `id`: active and interactable.
fn selectable(layouts: &UiLayouts, key: &str, view: &UiPrefabView, path: &str, id: i64, size: Vec2) -> bool {
    layouts
        .document(key)
        .and_then(|document| document.find(path).ok())
        .and_then(|index| view.selectable_live(layouts, size, index, id))
        .is_some_and(|(active, interactable)| active && interactable)
}

/// The first tap of `taps` that closes the shown dialog: a close button of
/// the window, the dialog's own close button, or (when the dialog closes
/// externally) a tap outside the window while it needs the background cover.
fn closing_tap(
    layouts: &UiLayouts,
    window: &Window,
    scale: f32,
    size: Vec2,
    view: (&SubWindowView, &UiPrefabView),
    shown: &SubWindowShown,
    closes_externally: bool,
    taps: &[Vec2],
) -> Option<CloseSource> {
    let (marker, prefab) = view;
    for position in taps {
        for (path, id) in &shown.bindings.close_buttons {
            if selectable(layouts, marker.key, prefab, path, *id, size) {
                return Some(CloseSource::CloseButton);
            }
        }
        if !closes_externally {
            continue;
        }
        if let Some((path, id)) = &shown.bindings.close_button {
            if selectable(layouts, marker.key, prefab, path, *id, size) {
                return Some(CloseSource::External);
            }
        }
        if shown.bindings.need_background_cover {
            let point = canvas_point(window, scale, *position);
            let inside = prefab
                .rect(layouts, &shown.bindings.content, size)
                .is_some_and(|rect| rect.active && rect.contains(point));
            if !inside {
                return Some(CloseSource::External);
            }
        }
    }
    None
}

/// Taps and the back key on the shown sub-window (modal: every tap is its).
/// A tap on the window off its close areas reaches nothing.
#[allow(clippy::too_many_arguments)]
pub fn click(
    mut subwindows: ResMut<SubWindows>,
    mut screens: ResMut<ScreenManager>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<(&SubWindowView, &UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let Some(shown) = subwindows.shown.as_ref() else {
        gestures.clear();
        back_keys.clear();
        return;
    };
    let (id, key) = (shown.id, shown.key);
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        gestures.clear();
        return;
    };
    let mut taps = Vec::new();
    for event in gestures.read() {
        if !event.kind.is_tap_family() {
            continue;
        }
        consumed.0 = true;
        if event.state == GestureState::End {
            taps.push(event.position);
        }
    }
    let back = back_keys.read().any(|event| event.id == id);
    let source = if back {
        Some(CloseSource::BackKey)
    } else if let Some(view) = views.iter().find(|(view, _)| view.key == key) {
        let size = root_canvas.size(window);
        let scale = root_canvas.scale(window);
        closing_tap(
            &layouts,
            window,
            scale,
            size,
            view,
            shown,
            subwindows.closes_externally,
            &taps,
        )
    } else {
        None
    };
    let Some(source) = source else {
        if !taps.is_empty() {
            info!("[subwindow] {key} ({id:?}): a tap on the window off its close areas: nothing");
        }
        return;
    };
    if let Some(shown) = subwindows.shown.as_mut() {
        close(shown, source);
    }
    finish(&mut subwindows, &mut screens);
}
