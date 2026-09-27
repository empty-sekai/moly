//! The learn-phenomenon dialog (`LearnPhenomenaSubWindowDialog`, dialog type
//! 381), opened by the harvest site's learn run with
//! [`crate::harvest::LearnPhenomenaDialogRequest`] and answered with
//! [`crate::harvest::LearnPhenomenaDialogClosed`] (its onClose).
//!
//! The source, in order:
//! - `ScreenManager.ShowSubWindowDialog` instantiates the prefab, runs
//!   `Initialize` (the close buttons of its `SubWindowComponent` call
//!   `Close`) and `Open`: `DialogBase.Open` plays the open SE of the
//!   serialized `openSE` from the dialog's SE table and starts
//!   `SubWindowFadeAnimation.OpenAnimation`, which activates the window, sets
//!   its CanvasGroup's alpha to 0 and `blocksRaycasts` to false, fades the
//!   alpha to 1 over `OPEN_ANIMATION_DURATION` (0.2 s, the tween settings'
//!   default ease) and then sets `blocksRaycasts` back to true. The window
//!   takes no raycast before that; `OnFinishOpenAnimation` marks it opened.
//! - `Setup(name, thumbnail)`: `_balloon.Initialize()` (with
//!   `isAllowedClose` its TouchController reports each press, and a press
//!   whose raycast does not include the balloon object hides it; then
//!   `Hide(false)`: alpha 0, `blocksRaycasts` false), the thumbnail button's
//!   listeners become `OnClick`, the raw image takes the thumbnail, the
//!   message body becomes `WordingManager.GetFormat("MSG_LEARN_PHENOMENA",
//!   name)` and `se_get_blueprint` plays.
//! - `OnClick` is `_balloon.Setup(name)`: `SetText`, `AdjustSize(padding)`
//!   when `adjustSize`, then `Show(true)`: `blocksRaycasts` true and a 0.1 s
//!   fade to alpha 1. `Hide(true)` fades to 0 over 0.1 s and sets
//!   `blocksRaycasts` false when that fade completes.
//! - `AdjustSize(padding)`: a canvas update fits the text to its new string;
//!   with the text's size fitter then off, the text rect is measured; a
//!   width above `maxSize.x - padding.horizontal` sets the
//!   text LayoutElement's preferred width to that bound, a height above
//!   `maxSize.y - padding.vertical` its preferred height; with the fitter
//!   back on the text is measured again, each side part gets the horizontal
//!   size `((width + left) + right - center width) * 0.5` and the balloon
//!   `(width + left) + right` by `(height + top) + bottom`.
//! - A tap on a close button (`CloseArea`, which covers the screen under the
//!   panel, or the panel `Base`) calls `Close`: the onClose callback, then
//!   the close animation (alpha to 0 over `CLOSE_ANIMATION_DURATION`, 0.2 s)
//!   and the dialog's destruction (`closeBehavior` Destroy). The tapped
//!   button plays its own serialized SE.
//!
//! Opener: the screen manager ([`crate::ui_layers`]) shows the dialog
//! (`ShowDialog` with its `Dialog/` prefab), and this view reports `Open`,
//! the open animation's end, `Close` and the destruction back to it; the
//! manager hands it the back key when it is the topmost dialog.
//!
//! Host mapping: the dialog occupies the dialog input layer from the request
//! to the end of its close animation (`ShellDialogState`); a tap the window
//! does not take while it is not raycastable goes to the background cover,
//! whose tap listener is installed only after the open animation, so it does
//! nothing. The press the balloon's TouchController sees and the click are
//! the same tap event here. A tween created in a frame takes its first step
//! in that frame, with that frame's delta.
//!
//! Named gaps: the background fill cover is not drawn (its colour is set by
//! code from a static table not read here); the result-pop particle on the
//! thumbnail is not drawn; the external close (allowCloseExternal) has no
//! source here besides the cover, which the close area covers.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::UiPrefab;
use moly_law::ui::dotween::{Ease, FloatTween};
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::harvest::{LearnPhenomenaDialogClosed, LearnPhenomenaDialogRequest};
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager};
use crate::ui_layout::{Pointer, UiLayouts, UiPrefabView};

pub(crate) const KEY: &str = "LearnPhenomena";
const PRESENTER: &str = "Sekai.Mysekai.LearnPhenomenaSubWindowDialog";
const MESSAGE_KEY: &str = "MSG_LEARN_PHENOMENA";
/// `SubWindowFadeAnimation`'s constructor: the open and close durations.
const OPEN_ANIMATION_DURATION: f32 = 0.2;
const CLOSE_ANIMATION_DURATION: f32 = 0.2;
/// `UIPartsCommonBalloon.Show` / `Hide`: the fade duration.
const BALLOON_FADE: f32 = 0.1;
/// `DialogBase`'s SE table, indexed by the serialized `openSE`; the last
/// value (`None`) plays nothing.
const OPEN_SE_TABLE: [&str; 8] = [
    "SE_UI_DIALOG_OPEN",
    "SE_REWARD_DIALOG_OPEN",
    "SE_UI_COSTUME_RESULT",
    "SE_RANKUP",
    "SE_UI_CHOICES_OPEN",
    "SE_SUBWINDOW_OPEN",
    "se_recycling",
    "se_get_blueprint",
];
const OPEN_SE_NONE: i64 = 8;
/// `LearnPhenomenaSubWindowDialog.Setup`'s SE.
const SETUP_SE: &str = "se_get_blueprint";
const THUMBNAIL_PREFIX: &str = "mysekai/thumbnail/phenomena/";

/// The dialog's view (the dialog slot's layer).
#[derive(Component)]
pub(crate) struct LearnPhenomenaRoot;

/// The serialized references the dialog code reads, as view paths.
struct Bindings {
    pristine: UiPrefab,
    open_se: Option<&'static str>,
    close_buttons: Vec<i64>,
    thumbnail_button: i64,
    fade_group: String,
    item_image: String,
    message_text: String,
    balloon: BalloonBindings,
}

struct BalloonBindings {
    root: String,
    group: String,
    text: String,
    text_element: i64,
    center: String,
    sides: Vec<String>,
    adjust_size: bool,
    allowed_close: bool,
    /// left, right, top, bottom.
    padding: [i32; 4],
    max_size: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Opening,
    Open,
    Closing,
}

struct Run {
    /// The dialog the screen manager shows.
    id: DialogId,
    phenomena_id: i32,
    name: String,
    stage: Stage,
    alpha: f32,
    fade: FloatTween,
    opened_at: f64,
    balloon: BalloonState,
}

#[derive(Default)]
struct BalloonState {
    /// `canvasGroup.blocksRaycasts`: the balloon is hit-testable and drawn.
    blocks: bool,
    alpha: f32,
    /// The running fade and whether it is a hide.
    tween: Option<(FloatTween, bool)>,
}

#[derive(Resource, Default)]
pub(crate) struct LearnPhenomenaDialog {
    bindings: Option<Bindings>,
    run: Option<Run>,
    edited_document: bool,
    ease_warned: bool,
}

/// The phenomenon names the dialog and the notice banner print, for the text
/// atlas. The atlas is baked once, before the weather chain resolves its
/// catalogue, so the names are read from the phenomenon index as soon as it
/// loads.
#[derive(Resource)]
pub(crate) struct PhenomenonGlyphs {
    handle: Handle<JsonAsset>,
    pub(crate) texts: Option<Vec<String>>,
}

pub(crate) fn load_glyphs(mut commands: Commands, server: Res<AssetServer>) {
    commands.init_resource::<LearnPhenomenaDialog>();
    commands.insert_resource(PhenomenonGlyphs {
        handle: server.load("moly://phenomena/index.json"),
        texts: None,
    });
}

pub(crate) fn parse_glyphs(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut glyphs: ResMut<PhenomenonGlyphs>,
) {
    if glyphs.texts.is_some() {
        return;
    }
    if let bevy::asset::LoadState::Failed(error) = server.load_state(&glyphs.handle) {
        error!("[learn-phenomena] the phenomenon index failed to load ({error:?}): no phenomenon names reach the text atlas");
        glyphs.texts = Some(Vec::new());
        return;
    }
    let Some(asset) = jsons.get(&glyphs.handle) else {
        return;
    };
    let mut texts = Vec::new();
    match serde_json::from_str::<Value>(&asset.0) {
        Ok(doc) => {
            for entry in doc["phenomena"].as_object().into_iter().flat_map(|rows| rows.values()) {
                for field in ["name", "englishName"] {
                    if let Some(text) = entry["master"][field].as_str() {
                        texts.push(text.to_owned());
                    }
                }
            }
        }
        Err(error) => error!("[learn-phenomena] the phenomenon index is not JSON ({error}): no phenomenon names reach the text atlas"),
    }
    info!(
        "[learn-phenomena] {} phenomenon master names and English names join the text atlas",
        texts.len()
    );
    glyphs.texts = Some(texts);
}

fn pointer(value: &Value, what: &str) -> i64 {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .unwrap_or_else(|| panic!("{KEY}: {what} is not a serialized reference"));
    assert_eq!(
        pair[0].as_i64(),
        Some(0),
        "{KEY}: {what} points outside the prefab"
    );
    let id = pair[1]
        .as_i64()
        .unwrap_or_else(|| panic!("{KEY}: {what} has no identity"));
    assert_ne!(id, 0, "{KEY}: {what} is null");
    id
}

/// The view path of the node holding identity `id`: one path per node, so
/// every override of a node lands in the same entry.
fn node_path(doc: &UiPrefab, id: i64) -> String {
    let index = doc
        .find(&format!("@{id}"))
        .unwrap_or_else(|e| panic!("{e}"));
    format!("@{}", doc.nodes[index].game_object_id)
}

fn component<'a>(doc: &'a UiPrefab, id: i64) -> &'a moly_assets::ui_layout::UiComponent {
    doc.nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == id)
        .unwrap_or_else(|| panic!("{KEY}: component {id} is not in the prefab"))
}

fn bind(doc: &UiPrefab) -> Bindings {
    let presenter = doc
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.class == PRESENTER)
        .unwrap_or_else(|| panic!("{KEY}: no {PRESENTER}"));
    let f = &presenter.fields;
    let open_se = match f["openSE"]
        .as_i64()
        .unwrap_or_else(|| panic!("{KEY}: openSE"))
    {
        OPEN_SE_NONE => None,
        index => Some(
            *OPEN_SE_TABLE
                .get(usize::try_from(index).expect("openSE range"))
                .unwrap_or_else(|| panic!("{KEY}: openSE {index} is outside the SE table")),
        ),
    };
    let window = component(doc, pointer(&f["subWindowComponent"], "subWindowComponent"));
    let close_buttons: Vec<i64> = window.fields["closeButtons"]
        .as_array()
        .unwrap_or_else(|| panic!("{KEY}: closeButtons"))
        .iter()
        .map(|button| pointer(button, "closeButtons[]"))
        .collect();
    let animation = component(doc, pointer(&f["animation"], "animation"));
    assert_eq!(
        animation.class, "Sekai.SubWindowFadeAnimation",
        "{KEY}: the open animation is not a fade"
    );
    let fade_group = pointer(&animation.fields["canvasGroup"], "animation.canvasGroup");
    let balloon = component(doc, pointer(&f["_balloon"], "_balloon"));
    let b = &balloon.fields;
    let padding = b["padding"]
        .as_array()
        .filter(|p| p.len() == 4)
        .map(|p| [0, 1, 2, 3].map(|k| p[k].as_i64().expect("balloon padding") as i32))
        .unwrap_or_else(|| panic!("{KEY}: balloon padding"));
    let max_size = b["maxSize"]
        .as_array()
        .filter(|m| m.len() == 2)
        .map(|m| {
            Vec2::new(
                m[0].as_f64().expect("maxSize") as f32,
                m[1].as_f64().expect("maxSize") as f32,
            )
        })
        .unwrap_or_else(|| panic!("{KEY}: balloon maxSize"));
    let text_element = pointer(&b["textLayoutElement"], "balloon textLayoutElement");
    assert_eq!(
        component(doc, text_element).class,
        "UnityEngine.UI.LayoutElement",
        "{KEY}: textLayoutElement"
    );
    let root = node_path(doc, balloon.path_id);
    // The TouchController hit target is the balloon's own object: a press
    // hits it when the object's raycast graphic contains the point.
    let hit = doc.nodes[doc.find(&root).expect("balloon node")]
        .components
        .iter()
        .find(|c| c.fields.get("m_RaycastTarget").is_some())
        .unwrap_or_else(|| panic!("{KEY}: the balloon object has no raycast graphic"));
    assert!(
        hit.fields["m_RaycastPadding"]
            .as_array()
            .is_some_and(|p| p.iter().all(|v| v.as_f64() == Some(0.0))),
        "{KEY}: the balloon's raycast graphic is padded"
    );
    Bindings {
        pristine: doc.clone(),
        open_se,
        close_buttons,
        thumbnail_button: pointer(&f["_button"], "_button"),
        fade_group: node_path(doc, fade_group),
        item_image: node_path(doc, pointer(&f["_itemImage"], "_itemImage")),
        message_text: node_path(doc, pointer(&f["messageBodyText"], "messageBodyText")),
        balloon: BalloonBindings {
            group: node_path(doc, pointer(&b["canvasGroup"], "balloon canvasGroup")),
            text: node_path(doc, pointer(&b["mainText"], "balloon mainText")),
            text_element,
            center: node_path(doc, pointer(&b["baseCenterRt"], "balloon baseCenterRt")),
            sides: b["baseSideRts"]
                .as_array()
                .expect("balloon baseSideRts")
                .iter()
                .map(|side| node_path(doc, pointer(side, "balloon baseSideRts[]")))
                .collect(),
            adjust_size: b["adjustSize"].as_bool().expect("balloon adjustSize"),
            allowed_close: b["isAllowedClose"]
                .as_bool()
                .expect("balloon isAllowedClose"),
            padding,
            max_size,
            root,
        },
    }
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut dialog: ResMut<LearnPhenomenaDialog>,
) {
    if dialog.bindings.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let doc = layouts
        .document(KEY)
        .expect("ready learn-phenomenon prefab");
    let bindings = bind(doc);
    let mut view = UiPrefabView::new(KEY, SITEMAP_LAYER);
    // Initialize: Hide(false) leaves the balloon without raycasts.
    view.set_visible(&bindings.balloon.root, false);
    view.set_alpha(&bindings.balloon.group, 0.0);
    info!(
        "[learn-phenomena] {KEY} view: open SE {:?}, close buttons {:?}, thumbnail button @{}, fade group {}, balloon {} (padding {:?}, max {:?}, adjust {}, allowed close {})",
        bindings.open_se, bindings.close_buttons, bindings.thumbnail_button, bindings.fade_group,
        bindings.balloon.root, bindings.balloon.padding, bindings.balloon.max_size,
        bindings.balloon.adjust_size, bindings.balloon.allowed_close
    );
    commands.spawn((
        LearnPhenomenaRoot,
        Visibility::Hidden,
        Transform::from_xyz(0.0, 0.0, 100.0),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    dialog.bindings = Some(bindings);
}

fn default_ease(layouts: &UiLayouts, warned: &mut bool) -> Ease {
    if layouts.tween_defaults().is_none() && !*warned {
        *warned = true;
        warn!("[learn-phenomena] this UI root carries no tween defaults: the fades take OutQuad, the default of the JP root's tween settings");
    }
    Ease::OutQuad
}

/// `ShowSubWindowDialog` + `Setup(name, thumbnail)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn open(
    mut requests: MessageReader<LearnPhenomenaDialogRequest>,
    time: Res<Time>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut dialog: ResMut<LearnPhenomenaDialog>,
    mut shell: ResMut<ShellDialogState>,
    mut sounds: ResMut<SeRequests>,
    mut views: Query<&mut UiPrefabView, With<LearnPhenomenaRoot>>,
    mut screens: ResMut<ScreenManager>,
) {
    for request in requests.read() {
        let dialog = &mut *dialog;
        if dialog.run.is_some() {
            error!("[learn-phenomena] a second dialog request for phenomenon {} while the dialog is open: dropped", request.phenomena_id);
            continue;
        }
        let (Some(bindings), Ok(mut view)) = (dialog.bindings.as_ref(), views.single_mut()) else {
            error!("[learn-phenomena] dialog requested for phenomenon {} before its prefab is ready: the dialog does not open", request.phenomena_id);
            continue;
        };
        let Some(option) = catalogue.0.iter().find(|option| {
            option
                .metadata
                .as_ref()
                .is_some_and(|m| m.id == request.phenomena_id)
        }) else {
            error!("[learn-phenomena] no phenomenon master row {} in the catalogue: the dialog does not open", request.phenomena_id);
            continue;
        };
        let master = option.metadata.as_ref().expect("catalogue row with master");
        let icon = master.icon_assetbundle_name.as_deref().unwrap_or_else(|| {
            panic!(
                "[learn-phenomena] phenomenon {} has no iconAssetbundleName",
                master.id
            )
        });
        assert_eq!(
            request.thumbnail,
            format!("{THUMBNAIL_PREFIX}{icon}"),
            "[learn-phenomena] the request's thumbnail is not the master row's icon bundle"
        );
        let source = option
            .icon_source
            .as_ref()
            .unwrap_or_else(|| panic!("[learn-phenomena] the thumbnail {icon} was not extracted"));
        // ShowSubWindowDialog: InstantiateDialog("Dialog/" + type) and Initialize.
        let id = match screens.show_dialog(
            DialogType::LearnPhenomenaSubWindowDialog,
            DisplayLayerType::LayerDialog,
            DialogBackKey::Close,
            "HarvestUtility (learn phenomenon)",
        ) {
            Ok(id) => id,
            Err(error) => {
                error!("[learn-phenomena] {error}: the dialog does not open");
                continue;
            }
        };
        let alias = format!("learn-phenomena/{icon}");
        layouts.register_runtime_texture(
            &alias,
            &format!("moly://phenomena/{}", source.file),
            &server,
        );
        if dialog.edited_document {
            // A fresh instance of the prefab: its text LayoutElement is the serialized one.
            layouts
                .replace_runtime_document(KEY, bindings.pristine.clone(), &server)
                .unwrap_or_else(|e| panic!("[learn-phenomena] {e}"));
            dialog.edited_document = false;
        }
        let wording = layouts.wording(MESSAGE_KEY);
        let message =
            moly_law::text::custom_text_mesh::format_wording(&wording, &[master.name.clone()])
                .unwrap_or_else(|e| panic!("UI wording {MESSAGE_KEY}: {e}"));
        view.set_texture(&bindings.item_image, &alias);
        view.set_text(&bindings.message_text, message.clone());
        view.set_visible(&bindings.balloon.root, false);
        view.set_alpha(&bindings.balloon.group, 0.0);
        view.set_alpha(&bindings.fade_group, 0.0);
        let ease = default_ease(&layouts, &mut dialog.ease_warned);
        for cue in bindings.open_se.into_iter().chain([SETUP_SE]) {
            sounds.0.push(SeRequest {
                owner: None,
                cue: cue.to_owned(),
                class: SeClass::Ui,
                source: "learn_phenomena_dialog",
            });
        }
        shell.learn_phenomena_open = true;
        screens.open_dialog(id);
        info!(
            "[learn-phenomena] t {:.4}: ShowSubWindowDialog(type 381): Open plays {:?}; OpenAnimation: alpha 0, no raycasts, fade to 1 over {OPEN_ANIMATION_DURATION}s {ease:?}; Setup(\"{}\", {}): balloon Initialize (hidden), message \"{message}\", {SETUP_SE}",
            time.elapsed_secs_f64(), bindings.open_se, master.name, request.thumbnail
        );
        dialog.run = Some(Run {
            id,
            phenomena_id: master.id,
            name: master.name.clone(),
            stage: Stage::Opening,
            alpha: 0.0,
            fade: FloatTween::new(1.0, OPEN_ANIMATION_DURATION, ease),
            opened_at: time.elapsed_secs_f64(),
            balloon: BalloonState::default(),
        });
    }
}

fn to_canvas(position: Vec2, window: &Window, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window.width() / 2.0,
        window.height() / 2.0 - position.y,
    ) / scale
}

/// `RectTransform.SetSizeWithCurrentAnchors(axis, size)`.
fn size_with_current_anchors(
    doc: &UiPrefab,
    view: &UiPrefabView,
    layouts: &UiLayouts,
    path: &str,
    canvas: Vec2,
    size: Vec2,
    axes: [bool; 2],
) -> Vec2 {
    let index = doc.find(path).unwrap_or_else(|e| panic!("{e}"));
    let node = &doc.nodes[index];
    let parent = doc
        .parent(index)
        .map(|p| format!("@{}", doc.nodes[p].game_object_id));
    let parent_size = parent
        .and_then(|p| view.rect(layouts, &p, canvas))
        .map_or(canvas, |r| r.size);
    let current = view.rect(layouts, path, canvas).expect("balloon part rect");
    let anchors = Vec2::from_array(node.rect.anchors_max) - Vec2::from_array(node.rect.anchors_min);
    // The current sizeDelta: the size minus the anchored span of the parent.
    let mut delta = current.size - parent_size * anchors;
    for axis in 0..2 {
        if axes[axis] {
            delta[axis] = size[axis] - parent_size[axis] * anchors[axis];
        }
    }
    delta
}

/// `UIPartsCommonBalloon.AdjustSize(padding)` on the view.
fn adjust_size(
    view: &mut UiPrefabView,
    layouts: &mut UiLayouts,
    server: &AssetServer,
    b: &BalloonBindings,
    canvas: Vec2,
) -> bool {
    let [left, right, top, bottom] = b.padding;
    let measured = view
        .rect(layouts, &b.text, canvas)
        .expect("balloon text rect")
        .size;
    let max_width = b.max_size.x - (left + right) as f32;
    let max_height = b.max_size.y - (top + bottom) as f32;
    let mut edits = Vec::new();
    if measured.x > max_width {
        edits.push(("m_PreferredWidth", max_width));
    }
    if measured.y > max_height {
        edits.push(("m_PreferredHeight", max_height));
    }
    let edited = !edits.is_empty();
    if edited {
        let mut doc = layouts
            .document(KEY)
            .expect("learn-phenomenon prefab")
            .clone();
        let element = doc
            .nodes
            .iter_mut()
            .flat_map(|node| node.components.iter_mut())
            .find(|c| c.path_id == b.text_element)
            .expect("balloon text LayoutElement");
        for (field, value) in &edits {
            element.fields[*field] = Value::from(*value);
        }
        layouts
            .replace_runtime_document(KEY, doc, server)
            .unwrap_or_else(|e| panic!("[learn-phenomena] {e}"));
    }
    let text = view
        .rect(layouts, &b.text, canvas)
        .expect("balloon text rect")
        .size;
    let center = view
        .rect(layouts, &b.center, canvas)
        .expect("balloon center rect")
        .size
        .x;
    let width = (text.x + left as f32) + right as f32;
    let height = (text.y + top as f32) + bottom as f32;
    let side = (width - center) * 0.5;
    let doc = layouts
        .document(KEY)
        .expect("learn-phenomenon prefab")
        .clone();
    for path in &b.sides {
        let delta = size_with_current_anchors(
            &doc,
            view,
            layouts,
            path,
            canvas,
            Vec2::new(side, 0.0),
            [true, false],
        );
        view.set_size_delta(path, delta);
    }
    let delta = size_with_current_anchors(
        &doc,
        view,
        layouts,
        &b.root,
        canvas,
        Vec2::new(width, height),
        [true, true],
    );
    view.set_size_delta(&b.root, delta);
    info!(
        "[learn-phenomena] balloon AdjustSize: text measured {measured:.3} (bounds {max_width} x {max_height}; LayoutElement edits {edits:?}), text {text:.3}, center width {center:.3} -> sides {side:.3}, balloon {width:.3} x {height:.3}"
    );
    edited
}

/// One tap on the open dialog: the balloon's TouchController press, then the
/// click of the selectable the raycast gives the tap.
#[allow(clippy::too_many_arguments)]
fn tap(
    point: Vec2,
    canvas: Vec2,
    ease: Ease,
    run: &mut Run,
    bindings: &Bindings,
    view: &mut UiPrefabView,
    layouts: &mut UiLayouts,
    server: &AssetServer,
    sounds: &mut SeRequests,
    now: f64,
    closed: &mut MessageWriter<LearnPhenomenaDialogClosed>,
    screens: &mut ScreenManager,
) -> bool {
    let b = &bindings.balloon;
    if b.allowed_close {
        let hit = run.balloon.blocks
            && view
                .rect(layouts, &b.root, canvas)
                .is_some_and(|r| r.active && r.contains(point));
        if !hit && run.balloon.blocks {
            // CheckClose(false): Hide(true).
            run.balloon.tween = Some((FloatTween::new(0.0, BALLOON_FADE, ease), true));
            info!("[learn-phenomena] press outside the balloon: Hide(true), fade to 0 over {BALLOON_FADE}s");
        }
    }
    let Some((_, id)) = view.press_target(layouts, Pointer::Canvas(point), canvas) else {
        return false;
    };
    let path = format!("@{id}");
    if id == bindings.thumbnail_button {
        sounds.source_button(layouts, KEY, &path);
        view.set_text(&b.text, run.name.clone());
        // Hide leaves the balloon object active (alpha 0, no raycasts); the
        // view's visibility stands in for its blocksRaycasts, so the balloon
        // is shown before AdjustSize: the size fitter only drives an active
        // text, and the measurement reads the fitted rect.
        view.set_visible(&b.root, true);
        let mut edited = false;
        if b.adjust_size {
            edited = adjust_size(view, layouts, server, b, canvas);
        }
        run.balloon.blocks = true;
        run.balloon.tween = Some((FloatTween::new(1.0, BALLOON_FADE, ease), false));
        info!("[learn-phenomena] thumbnail button: OnClick -> balloon Setup(\"{}\"): Show(true), fade to 1 over {BALLOON_FADE}s", run.name);
        return edited;
    }
    if bindings.close_buttons.contains(&id) {
        sounds.source_button(layouts, KEY, &path);
        close(run, ease, now, &format!("close button {path}"), closed, screens);
    }
    false
}

/// `Close`: the onClose callback, then the close animation.
fn close(
    run: &mut Run,
    ease: Ease,
    now: f64,
    by: &str,
    closed: &mut MessageWriter<LearnPhenomenaDialogClosed>,
    screens: &mut ScreenManager,
) {
    closed.write(LearnPhenomenaDialogClosed);
    run.stage = Stage::Closing;
    run.fade = FloatTween::new(0.0, CLOSE_ANIMATION_DURATION, ease);
    screens.close_dialog(run.id);
    info!(
        "[learn-phenomena] t {now:.4}: {by}: Close -> onClose (LearnPhenomenaDialogClosed), close animation fade to 0 over {CLOSE_ANIMATION_DURATION}s ({:.4}s after the open)",
        now - run.opened_at
    );
}

/// The back key the screen manager hands the topmost dialog:
/// `SubWindowDialog.OnHardwareBackKeyProcess` is `CloseProcess`, which is
/// `Close`.
pub(crate) fn back_key(
    mut keys: MessageReader<DialogBackKeyEvent>,
    time: Res<Time>,
    layouts: Res<UiLayouts>,
    mut dialog: ResMut<LearnPhenomenaDialog>,
    mut screens: ResMut<ScreenManager>,
    mut closed: MessageWriter<LearnPhenomenaDialogClosed>,
) {
    for key in keys.read() {
        let dialog = &mut *dialog;
        let Some(run) = dialog.run.as_mut().filter(|run| run.id == key.id) else {
            continue;
        };
        if run.stage == Stage::Closing {
            info!("[learn-phenomena] back key while the close animation runs: Close again changes nothing");
            continue;
        }
        let ease = default_ease(&layouts, &mut dialog.ease_warned);
        close(
            run,
            ease,
            time.elapsed_secs_f64(),
            "back key (SubWindowDialog.OnHardwareBackKeyProcess -> CloseProcess)",
            &mut closed,
            &mut screens,
        );
    }
}

/// Taps while the dialog is open (the dialog slot is modal).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut dialog: ResMut<LearnPhenomenaDialog>,
    mut views: Query<&mut UiPrefabView, With<LearnPhenomenaRoot>>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut sounds: ResMut<SeRequests>,
    mut closed: MessageWriter<LearnPhenomenaDialogClosed>,
    mut screens: ResMut<ScreenManager>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    let dialog = &mut *dialog;
    let (Some(run), Some(bindings)) = (dialog.run.as_mut(), dialog.bindings.as_ref()) else {
        return;
    };
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(root_canvas), Ok(mut view)) =
        (windows.single(), root_canvas.as_deref(), views.single_mut())
    else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let ease = default_ease(&layouts, &mut dialog.ease_warned);
    for position in taps {
        consumed.0 = true;
        if run.stage != Stage::Open {
            info!("[learn-phenomena] tap during the {:?} animation: the window takes no raycast, the cover has no listener", run.stage);
            continue;
        }
        let point = to_canvas(position, window, scale);
        if tap(
            point,
            canvas,
            ease,
            run,
            bindings,
            &mut view,
            &mut layouts,
            &server,
            &mut sounds,
            time.elapsed_secs_f64(),
            &mut closed,
            &mut screens,
        ) {
            dialog.edited_document = true;
        }
    }
}

/// The tweens, the window visibility and the end of the close animation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut dialog: ResMut<LearnPhenomenaDialog>,
    mut shell: ResMut<ShellDialogState>,
    mut roots: Query<
        (&mut Visibility, &mut Transform, &mut UiPrefabView),
        With<LearnPhenomenaRoot>,
    >,
    mut screens: ResMut<ScreenManager>,
) {
    let Ok((mut visibility, mut transform, mut view)) = roots.single_mut() else {
        return;
    };
    if let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) {
        transform.scale = Vec3::splat(root_canvas.scale(window));
    }
    let dialog = &mut *dialog;
    let (Some(run), Some(bindings)) = (dialog.run.as_mut(), dialog.bindings.as_ref()) else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Inherited;
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    if let Some(alpha) = run.fade.update(run.alpha, dt) {
        run.alpha = alpha;
        view.set_alpha(&bindings.fade_group, alpha);
    }
    let balloon = &mut run.balloon;
    if let Some((tween, hide)) = balloon.tween.as_mut() {
        if let Some(alpha) = tween.update(balloon.alpha, dt) {
            balloon.alpha = alpha;
            view.set_alpha(&bindings.balloon.group, alpha);
        }
        if tween.is_complete() {
            if *hide {
                balloon.blocks = false;
                view.set_visible(&bindings.balloon.root, false);
            }
            balloon.tween = None;
        }
    }
    if run.fade.is_complete() {
        let stage = run.stage;
        match stage {
            Stage::Opening => {
                run.stage = Stage::Open;
                screens.dialog_open_finished(run.id);
                info!(
                    "[learn-phenomena] t {now:.4}: OpenAnimation done ({:.4}s after the open): blocksRaycasts true, OnFinishOpenAnimation",
                    now - run.opened_at
                );
            }
            Stage::Closing => {
                info!(
                    "[learn-phenomena] t {now:.4}: close animation done ({:.4}s after the open): the dialog of phenomenon {} is destroyed",
                    now - run.opened_at, run.phenomena_id
                );
                screens.dialog_destroyed(run.id);
                dialog.run = None;
                shell.learn_phenomena_open = false;
                *visibility = Visibility::Hidden;
            }
            Stage::Open => {}
        }
    }
}

/// Instrument for headless runs, off by default:
/// `MOLY_LEARN_PHENOMENA_AUTOTAP_SECS` = seconds after the open animation
/// ends; then one tap on the thumbnail and, one second later, one on the
/// screen's top-left corner (the close area), through the tap events.
pub(crate) fn autotap(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<UiLayouts>,
    dialog: Res<LearnPhenomenaDialog>,
    views: Query<&UiPrefabView, With<LearnPhenomenaRoot>>,
    mut taps: MessageWriter<GestureEvent>,
    mut plan: Local<Option<Option<f32>>>,
    mut open_since: Local<Option<f64>>,
    mut step: Local<u8>,
) {
    let secs = *plan.get_or_insert_with(|| {
        let secs = std::env::var("MOLY_LEARN_PHENOMENA_AUTOTAP_SECS").ok().and_then(|v| v.parse::<f32>().ok());
        if let Some(secs) = secs {
            warn!("[learn-phenomena] MOLY_LEARN_PHENOMENA_AUTOTAP_SECS instrument on: taps {secs}s and {}s after the open animation", secs + 1.0);
        }
        secs
    });
    let Some(secs) = secs else {
        return;
    };
    let open = dialog
        .run
        .as_ref()
        .is_some_and(|run| run.stage == Stage::Open);
    if !open {
        if dialog.run.is_none() {
            *open_since = None;
            *step = 0;
        }
        return;
    }
    let now = time.elapsed_secs_f64();
    let since = *open_since.get_or_insert(now);
    let (Ok(window), Some(root_canvas), Ok(view), Some(bindings)) = (
        windows.single(),
        root_canvas.as_deref(),
        views.single(),
        dialog.bindings.as_ref(),
    ) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let at = |step: u8| since + f64::from(secs) + f64::from(step);
    let position = match *step {
        0 if now >= at(0) => {
            let doc = layouts.document(KEY).expect("learn-phenomenon prefab");
            let node = doc
                .find(&format!("@{}", bindings.thumbnail_button))
                .expect("thumbnail button node");
            let rect = view
                .rect(
                    &layouts,
                    &format!("@{}", doc.nodes[node].game_object_id),
                    canvas,
                )
                .expect("thumbnail rect");
            let c = rect.center();
            Some(Vec2::new(
                c.x * scale + window.width() / 2.0,
                window.height() / 2.0 - c.y * scale,
            ))
        }
        1 if now >= at(1) => Some(Vec2::new(8.0, 8.0)),
        _ => None,
    };
    if let Some(position) = position {
        info!(
            "[learn-phenomena] instrument tap {} at window {position:.1}",
            *step + 1
        );
        taps.write(GestureEvent {
            kind: GestureKind::Tap,
            state: GestureState::End,
            position,
            delta: Vec2::ZERO,
            ui_owned: true,
        });
        *step += 1;
    }
}
