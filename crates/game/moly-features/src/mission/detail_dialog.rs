//! `MysekaiMissionDetailDialog` (dialog type 339, Common2): one mission's
//! progress, or one sheet's information.
//!
//! Source structure:
//! - `ScreenLayerMysekaiMission.OnClickMysekaiMissionCell` opens the detail
//!   with `Show2ButtonDialog<MysekaiMissionDetailDialog>(339).SetupProgress(viewData)`
//!   when the tapped cell is not an achieved child and is not a complete
//!   sheet — that is, for a normal mission still in progress, and for the
//!   completion cell of a sheet that is not complete.
//! - `SetupProgress`: `_progressContent` on and `_infomationContent` off; the
//!   gauge `Setup(min(current, max), max)` with the progress clamped at 0; the
//!   text `"{current}/{max}"`; `_missionText` the master's sentence;
//!   `_rewardLabelText` `WORD_REWARD` with its background widened to
//!   `preferredWidth + 32`; `SetupContentElement(480, count)`; the reward
//!   list's `offsetMax.y` set to -240; `_buttonOK` off, because the
//!   `_transitionMethod` it would run is null.
//! - `SetupInformation` (`MysekaiMissionSheetCell.OnClickInfoButton`):
//!   `_progressContent` off and `_infomationContent` on; `_infoMissionText`
//!   the sheet's completion mission's sentence; `_rewardLabelText`
//!   `WORD_COMPLETE_REWARD` with the same widened background; `_missionClearObj`
//!   on with the mission complete; `SetupContentElement(390, count)`; the
//!   reward list's `offsetMax.y` set to -150; `_buttonOK` off.
//! - `SetupContentElement`: `_contentElement.preferredHeight = base +
//!   min(count, 3) * 176 - 176`, `SetContentFirstIndex(0)`, and — one frame
//!   later — `ScrollRect.enabled` and the vertical scrollbar's object active
//!   exactly when the count is not 3.
//! - `OnClickOK` (the footer's positive button) closes, as do `OnClickCancel`
//!   (the footer's close button), `OnCloseExternal` (the window's close
//!   button) and the hardware back key.
//!
//! Host mapping: the dialog takes the dialog slot
//! (`ScreenManager::show_dialog` for dialog type 339) and draws a runtime copy
//! of the root's `MissionDetail` document (`VIEW`), because the content height
//! is written into the `ContentRoot`'s `LayoutElement` — the layout reads that
//! serialized preferred height — so each opening installs a copy with the
//! height `SetupContentElement` computes, while the published document stays
//! as the root carries it. Both modes share that copy, its node set and its
//! entity; the mode picks the content nodes, the label, the list's top and the
//! height. The caller reads the request a cell's tap wrote and pushes the
//! slot; the dialog's own view reads it and runs `SetupProgress` /
//! `SetupInformation` there.
//!
//! Rendering limits:
//! - The reward rows are `MysekaiMissionRewardCell`s, a prefab with no
//!   document in this root: [`refuse_document`] names it once and the list
//!   draws no row. The count `SetupContentElement` takes is the mission's
//!   reward-row count, the one count a host can make without the reward rows'
//!   types (the masters carry no `resourceBoxes` table to convert them).
//! - The root's layout solves the authored rects: the writes reach the
//!   dialog's view, and a lowered list top or a widened label background
//!   shows where the document's own layout groups let the rect follow.
//! - The document's `NoneInfo` (useWordingKey on, empty key) keeps its
//!   serialized text: the list draws no row there.
//!
//! Refused by name: `SetupInformation` opens from `MysekaiMissionSheetCell`'s
//! info button, and that cell has no document in this root, so no request
//! reaches it; one that does is refused and no slot is pushed.
//!
//! Named gaps: the window scale tween of `DialogBase` is not drawn and the
//! `DialogSizeFitter` window height is not recomputed, so the document's own
//! layout groups take the content root's new preferred height; the footer
//! close label is the document's serialized text.

use std::collections::BTreeSet;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiNode, UiPrefab};
use serde_json::Value;

use super::cell::{refuse_document, widen_to_text, MissionDialogRequest, REWARD_CELL};
use super::screen;
use crate::action_button::ActionTapConsumed;
use crate::canvas::RootCanvas;
use crate::gesture::{GestureEvent, GestureState};
use crate::ui_inputs::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::mission::ClientMission;

/// The root's document (`dialogs/MysekaiMissionDetailDialog.json`).
pub const KEY: &str = "MissionDetail";
/// The runtime copy the dialog draws.
pub const VIEW: &str = "MissionDetailView";
/// `DialogType.MysekaiMissionDetailDialog`.
pub const DIALOG: DialogType = DialogType(339);
/// `OnClickMysekaiMissionCell`'s detail path.
pub const CALLER_PROGRESS: &str = "ScreenLayerMysekaiMission.OnClickMysekaiMissionCell";
/// `MysekaiMissionSheetCell.OnClickInfoButton`.
pub const CALLER_INFORMATION: &str = "MysekaiMissionSheetCell.OnClickInfoButton";
/// `PROGRESS_BASE_CONENT_ELEMENT_HEIGHT` (the source's spelling).
pub const PROGRESS_BASE_HEIGHT: f32 = 480.0;
/// `INFORMATION_BASE_CONENT_ELEMENT_HEIGHT`.
pub const INFORMATION_BASE_HEIGHT: f32 = 390.0;
/// `PROGRESS_LIST_VIEW_TOP`.
pub const PROGRESS_LIST_TOP: f32 = -240.0;
/// `INFORMATION_LIST_VIEW_TOP`.
pub const INFORMATION_LIST_TOP: f32 = -150.0;
/// `REWARD_HEIGHT`: one reward row's height.
pub const REWARD_HEIGHT: f32 = 176.0;
/// `REWARD_VIEW_MAX`: the rows a content root is sized for.
pub const REWARD_VIEW_MAX: usize = 3;
/// `LABEL_MARGIN_WIDTH`.
pub const LABEL_MARGIN_WIDTH: f32 = 32.0;
/// `WORD_REWARD`.
pub const WORD_REWARD: &str = "WORD_REWARD";
/// `WORD_COMPLETE_REWARD`.
pub const WORD_COMPLETE_REWARD: &str = "WORD_COMPLETE_REWARD";
const DIALOG_Z: f32 = 100.0;
const LAYOUT_ELEMENT: &str = "UnityEngine.UI.LayoutElement";
const SCROLL_RECT_CLASS: &str = "Sekai.UI.CustomScrollRect";

/// The dialog's nodes, on the dialog's entity.
#[derive(Debug, Clone, Component)]
pub(crate) struct Bindings {
    /// `ContentRoot/Content/ProgressInformation` (`_progressContent`).
    pub(crate) progress_content: String,
    /// `ProgressInformation/MissionText` (`_missionText`).
    pub(crate) mission_text: String,
    /// `ProgressInformation/ProgressText` (`_progressText`).
    pub(crate) progress_text: String,
    /// `ProgressInformation/UIPartsGauge` (`_progresGauge`).
    pub(crate) gauge: String,
    /// `ContentRoot/Content/MissionInformation` (`_infomationContent`).
    pub(crate) info_content: String,
    /// `MissionInformation/MissionText` (`_infoMissionText`).
    pub(crate) info_mission: String,
    /// `MissionInformation/Clear` (`_missionClearObj`).
    pub(crate) info_clear: String,
    /// `ContentRoot/Content/RewardBg/RewardTitle` (the node whose rect
    /// `_rewardLabelBg.position.x + width + LABEL_MARGIN_WIDTH` widens).
    pub(crate) reward_label_bg: String,
    /// `RewardBg/RewardTitle/TitleText` (`_rewardLabelText`).
    pub(crate) reward_label: String,
    /// `Content/RewardBg/ListView`'s node (`_listViewRectTransform`).
    pub(crate) list: String,
    /// `ListView/Viewport/Content`.
    pub(crate) list_content: String,
    /// `ListView/Viewport`, the list's clip.
    pub(crate) list_viewport: String,
    /// `RewardBg/ScrollbarVertical` (the vertical scrollbar's object).
    pub(crate) scrollbar: String,
    /// `ListView`'s `CustomScrollRect`, whose `enabled` the source sets.
    pub(crate) scroll_rect: i64,
    /// The list's authored pivot and height.
    pub(crate) list_pivot_y: f32,
    pub(crate) list_height: f32,
    /// The Mask above the gauge's `fillImage`, the node a fill is drawn on.
    pub(crate) gauge_mask: String,
    /// `WindowRoot/FooterButtons/UIPartsCommonButton_close` (`OnClickCancel`).
    pub(crate) footer_close: String,
    /// `WindowRoot/FooterButtons/UIPartsCommonButton_ToScreen` (`_buttonOK`).
    pub(crate) to_screen: String,
    /// `WindowRoot/UIPartsCloseButton` (`DialogBase`'s close button,
    /// `OnCloseExternal` -> `OnClickCancel`).
    pub(crate) close_button: String,
}

impl Bindings {
    pub(crate) fn read(doc: &UiPrefab) -> Result<Self, String> {
        let root = doc
            .nodes
            .first()
            .map(|node| node.path.clone())
            .ok_or("the document has no root")?;
        let window = format!("{root}/WindowRoot");
        let content_root = format!("{window}/ContentRoot");
        // Every node below is resolved now: a document missing one of them is
        // refused by name instead of failing later inside the frame's draw.
        let path = |suffix: String| -> Result<String, String> { doc.find(&suffix).map(|_| suffix) };
        let content = path(format!("{content_root}/Content"))?;
        let progress = path(format!("{content}/ProgressInformation"))?;
        let info = path(format!("{content}/MissionInformation"))?;
        let gauge = path(format!("{progress}/UIPartsGauge"))?;
        let list = path(format!("{content}/RewardBg/ListView"))?;
        let reward_label_bg = path(format!("{content}/RewardBg/RewardTitle"))?;
        let reward_label = path(format!("{reward_label_bg}/TitleText"))?;
        let list_node: &UiNode = &doc.nodes[doc.find(&list)?];
        let scroll_rect = doc.nodes[doc.find(&list)?]
            .components
            .iter()
            .find(|component| component.class == SCROLL_RECT_CLASS)
            .map(|component| component.path_id)
            .ok_or_else(|| format!("{list} has no {SCROLL_RECT_CLASS}"))?;
        let gauge_mask = format!("@{}", mask_of(doc, &gauge)?);
        Ok(Self {
            mission_text: path(format!("{progress}/MissionText"))?,
            progress_text: path(format!("{progress}/ProgressText"))?,
            info_mission: path(format!("{info}/MissionText"))?,
            info_clear: path(format!("{info}/Clear"))?,
            list_content: format!("{list}/Viewport/Content"),
            list_viewport: format!("{list}/Viewport"),
            scrollbar: path(format!("{content}/RewardBg/ScrollbarVertical"))?,
            footer_close: path(format!("{window}/FooterButtons/UIPartsCommonButton_close"))?,
            to_screen: path(format!("{window}/FooterButtons/UIPartsCommonButton_ToScreen"))?,
            close_button: path(format!("{window}/UIPartsCloseButton"))?,
            list_pivot_y: list_node.rect.pivot[1],
            list_height: list_node.rect.size_delta[1],
            scroll_rect,
            gauge_mask,
            progress_content: progress,
            info_content: info,
            gauge,
            reward_label,
            reward_label_bg,
            list,
        })
    }
}

#[derive(Component)]
pub struct MissionDetailDialogRoot;

/// Which of the dialog's two modes is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// `SetupProgress`: one mission's progress and its own rewards.
    #[default]
    Progress,
    /// `SetupInformation`: one sheet's information and its completion
    /// mission's rewards.
    Information,
}

/// The dialog's state.
#[derive(Resource, Default)]
pub struct MissionDetailDialog {
    pub(crate) shown: Option<DialogId>,
    pub(crate) mode: Mode,
    /// The mission `SetupProgress` / `SetupInformation` shows.
    pub(crate) mission: Option<i32>,
    /// The count `SetupContentElement` took.
    pub(crate) count: usize,
    /// Whether `Setup` ran on the dialog's own view.
    pub(crate) set_up: bool,
    seen: BTreeSet<String>,
}

impl MissionDetailDialog {
    /// The dialog is up.
    pub fn shown(&self) -> bool {
        self.shown.is_some()
    }

    /// `SetupProgress` / `SetupInformation`.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// `SetupContentElement`'s reward row count.
    pub fn count(&self) -> usize {
        self.count
    }
}

/// `SetupContentElement`: the `ContentRoot` preferred height for a row count.
pub fn content_height(base: f32, count: usize) -> f32 {
    base + count.min(REWARD_VIEW_MAX) as f32 * REWARD_HEIGHT - REWARD_HEIGHT
}

/// `SetupProgress`'s content height (`PROGRESS_BASE_CONENT_ELEMENT_HEIGHT`).
pub fn progress_height(count: usize) -> f32 {
    content_height(PROGRESS_BASE_HEIGHT, count)
}

/// `SetupInformation`'s content height
/// (`INFORMATION_BASE_CONENT_ELEMENT_HEIGHT`).
pub fn information_height(count: usize) -> f32 {
    content_height(INFORMATION_BASE_HEIGHT, count)
}

/// `SetupContentElement`'s list top, the mode's `offsetMax.y`.
pub fn list_top(mode: Mode) -> f32 {
    match mode {
        Mode::Progress => PROGRESS_LIST_TOP,
        Mode::Information => INFORMATION_LIST_TOP,
    }
}

/// `SetupContentElement`'s scrollbar rule: `ScrollRect.enabled` and the
/// scrollbar are active exactly when the count is not [`REWARD_VIEW_MAX`].
pub fn scrollable(count: usize) -> bool {
    count != REWARD_VIEW_MAX
}

/// A copy of the document with `ContentRoot`'s `LayoutElement` preferred
/// height set to `height`.
fn with_height(template: &UiPrefab, height: f32) -> Result<UiPrefab, String> {
    let mut doc = template.clone();
    let content_root = format!("{}/WindowRoot/ContentRoot", doc.nodes[0].path);
    let index = doc.find(&content_root)?;
    let element = doc.nodes[index]
        .components
        .iter_mut()
        .find(|component| component.class == LAYOUT_ELEMENT)
        .ok_or_else(|| format!("{content_root} has no {LAYOUT_ELEMENT}"))?;
    element.fields["m_PreferredHeight"] = Value::from(height);
    Ok(doc)
}

/// Update: the dialog's runtime document and view once the root's document is
/// in.
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    roots: Query<(), With<MissionDetailDialogRoot>>,
) {
    if !roots.is_empty() {
        return;
    }
    let Some(template) = layouts.document(KEY).cloned() else {
        return;
    };
    let bindings = match Bindings::read(&template) {
        Ok(bindings) => bindings,
        Err(reason) => {
            error!("mission: {KEY} is refused: {reason}");
            return;
        }
    };
    let document = match with_height(&template, progress_height(0)) {
        Ok(document) => document,
        Err(reason) => {
            error!("mission: {KEY} is refused: {reason}");
            return;
        }
    };
    if let Err(reason) = layouts.replace_runtime_document(VIEW, document, &server) {
        error!("mission: {KEY} is not installed: {reason}");
        return;
    }
    // The list's cell prefab is not in this root: no reward row is drawn.
    let mut seen = BTreeSet::new();
    refuse_document(&mut seen, REWARD_CELL);
    let mut view = UiPrefabView::new(VIEW, SITEMAP_LAYER);
    // `_transitionMethod` is null in both modes, so `_buttonOK` is off.
    view.set_visible(&bindings.to_screen, false);
    view.set_visible(&bindings.progress_content, false);
    view.set_visible(&bindings.info_content, false);
    commands.spawn((
        MissionDetailDialogRoot,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., DIALOG_Z),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
        bindings,
    ));
    info!("mission: MysekaiMissionDetailDialog's document ({KEY}) is installed as a runtime copy");
}

/// The dialog's reads for the input paths.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DialogWorld<'w, 's> {
    pub windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    pub root_canvas: Option<Res<'w, RootCanvas>>,
    pub views: Query<'w, 's, (&'static Bindings, &'static UiPrefabView), With<MissionDetailDialogRoot>>,
}

/// Update: the dialog's entity visibility and canvas scale while it is up.
pub fn place(
    dialog: Res<MissionDetailDialog>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut roots: Query<(&mut Visibility, &mut Transform), With<MissionDetailDialogRoot>>,
) {
    let Ok((mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    if let (Ok(window), Some(canvas)) = (windows.single(), root_canvas.as_deref()) {
        crate::canvas::set_scale(&mut transform, canvas.scale(window));
    }
    // The document root is the window's own node: an override hiding it
    // would hide the window, so the entity's visibility alone hides the
    // dialog.
    visibility.set_if_neq(if dialog.shown.is_none() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
}

/// `Show2ButtonDialog<MysekaiMissionDetailDialog>(339)`: the slot, `Open`, and
/// the open animation's end (`DialogBase`'s tween is not drawn).
pub fn show(screens: &mut ScreenManager, caller: &str) -> Result<DialogId, String> {
    let id = screens.show_dialog(
        DIALOG,
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        caller,
    )?;
    screens.open_dialog(id);
    screens.dialog_open_finished(id);
    Ok(id)
}

/// Update: the cell tap's request. The pushed slot is read by [`setup`] with
/// the dialog's own view; this step only resolves the mission and the count.
pub fn open(
    mut requests: MessageReader<MissionDialogRequest>,
    mut dialog: ResMut<MissionDetailDialog>,
    mut screens: ResMut<ScreenManager>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    roots: Query<(), With<MissionDetailDialogRoot>>,
    layouts: Res<UiLayouts>,
) {
    let asked: Vec<(i32, Mode)> = requests
        .read()
        .filter_map(|request| match request {
            MissionDialogRequest::Detail(id) => Some((*id, Mode::Progress)),
            MissionDialogRequest::Information(id) => Some((*id, Mode::Information)),
            MissionDialogRequest::List => None,
        })
        .collect();
    for (mission, mode) in asked {
        let caller = match mode {
            Mode::Progress => CALLER_PROGRESS,
            Mode::Information => CALLER_INFORMATION,
        };
        if dialog.shown.is_some() {
            error!("mission: {caller} while the detail dialog is shown: the shown one stays");
            continue;
        }
        let Some(masters) = tables
            .as_deref()
            .and_then(|tables| tables.mission.as_ref())
        else {
            error!(
                "mission: {caller} before the mission masters are in: nothing"
            );
            continue;
        };
        if roots.is_empty() || layouts.document(VIEW).is_none() {
            error!("mission: {KEY} is not installed: the mission detail dialog does not open");
            continue;
        }
        if mode == Mode::Information {
            error!(
                "mission: {CALLER_INFORMATION} (SetupInformation, mission {mission}): the sheet cell that opens the information mode is not built; the request is refused"
            );
            continue;
        }
        if masters.mission(mission).is_none() {
            error!("mission: {caller} names mission {mission}, which the masters lack: nothing");
            continue;
        }
        // `ConvertRewardDataToDetailData(viewData.Rewards)`: the reward rows
        // the list would hold; its cells are not in the root.
        let count = masters.rewards_of(mission).len();
        let id = match show(&mut screens, caller) {
            Ok(id) => id,
            Err(reason) => {
                error!("mission: {reason}: the mission detail dialog does not open");
                continue;
            }
        };
        dialog.shown = Some(id);
        dialog.mode = mode;
        dialog.mission = Some(mission);
        dialog.count = count;
        dialog.set_up = false;
        info!(
            "mission: MysekaiMissionDetailDialog: Show2ButtonDialog<MysekaiMissionDetailDialog>(339) is open for mission {mission} ({mode:?}); SetupContentElement counts {count} reward row(s)"
        );
    }
}

/// `SetupContentElement`'s height for a mode and count.
pub fn content_height_for(mode: Mode, count: usize) -> f32 {
    match mode {
        Mode::Progress => progress_height(count),
        Mode::Information => information_height(count),
    }
}

/// One mission's `MysekaiMissionViewData` as a `Setup` reads it.
pub(crate) struct SetupData {
    pub mode: Mode,
    /// `_missionText` / `_infoMissionText`: the master's sentence with
    /// `ChangeNonBreakingSpace`.
    pub sentence: String,
    /// `SetupProgress`'s `"{current}/{max}"` and the gauge's fill.
    pub progress: Option<(String, f32)>,
    /// `SetupInformation`'s `IsComplete` (`_missionClearObj`).
    pub complete: bool,
    /// `SetupContentElement`'s count.
    pub count: usize,
}

/// `SetupProgress` / `SetupInformation` on the dialog's own view: the copy
/// with `SetupContentElement`'s height is installed, then the mode's nodes,
/// label, list top and scrollbar are written.
#[allow(clippy::too_many_arguments)]
pub fn setup_view(
    view: &mut UiPrefabView,
    layouts: &mut UiLayouts,
    server: &AssetServer,
    template: &UiPrefab,
    bindings: &Bindings,
    data: &SetupData,
    seen: &mut BTreeSet<String>,
) -> Result<(), String> {
    let glyphs: BTreeSet<char> = layouts.text_chars().into_iter().collect();
    let document = with_height(template, content_height_for(data.mode, data.count))?;
    layouts.replace_runtime_document(VIEW, document, &server)?;
    match data.mode {
        Mode::Progress => {
            view.set_visible(&bindings.progress_content, true);
            view.set_visible(&bindings.info_content, false);
            put_text(view, template, &bindings.mission_text, &data.sentence, &glyphs, seen);
            if let Some((progress, fill)) = &data.progress {
                put_text(view, template, &bindings.progress_text, progress, &glyphs, seen);
                view.set_fill(&bindings.gauge_mask, *fill);
            }
        }
        Mode::Information => {
            view.set_visible(&bindings.progress_content, false);
            view.set_visible(&bindings.info_content, true);
            put_text(view, template, &bindings.info_mission, &data.sentence, &glyphs, seen);
            view.set_visible(&bindings.info_clear, data.complete);
        }
    }
    let key = match data.mode {
        Mode::Progress => WORD_REWARD,
        Mode::Information => WORD_COMPLETE_REWARD,
    };
    let label = layouts.wording(key);
    view.set_text(&bindings.reward_label, label.clone());
    widen_to_text(
        view,
        layouts,
        template,
        &bindings.reward_label_bg,
        &label,
        LABEL_MARGIN_WIDTH,
    );
    // `offsetMax.y = <mode's top>`: the bottom edge stays, so the anchored y
    // takes the top minus the height above the pivot.
    let top = list_top(data.mode);
    let y = top - bindings.list_height * (1.0 - bindings.list_pivot_y);
    view.set_anchored_position(&bindings.list, Vec2::new(0.0, y));
    view.set_visible(&bindings.list_content, true);
    let scroll = scrollable(data.count);
    view.set_visible(&bindings.scrollbar, scroll);
    view.set_behaviour_enabled(bindings.scroll_rect, scroll);
    // `_transitionMethod` is null in both modes: `_buttonOK` stays off.
    view.set_visible(&bindings.to_screen, false);
    Ok(())
}

/// The Mask on or above a gauge's `fillImage`.
fn mask_of(doc: &UiPrefab, gauge: &str) -> Result<i64, String> {
    let pair = doc.nodes[doc.find(gauge)?]
        .components
        .iter()
        .find(|component| component.class == "Sekai.UIPartsGauge")
        .and_then(|component| component.fields.get("fillImage"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{gauge} has no fillImage"))?;
    let id = pair
        .get(1)
        .and_then(Value::as_i64)
        .filter(|id| *id != 0)
        .ok_or_else(|| format!("{gauge}'s fillImage is null"))?;
    let index = doc.find(&format!("@{id}"))?;
    // The fillImage may itself carry the Mask; GaugeFill is its child.
    let mut cursor = Some(index);
    while let Some(parent) = cursor {
        if doc.nodes[parent]
            .components
            .iter()
            .any(|component| component.class.ends_with("Mask"))
        {
            return Ok(doc.nodes[parent].game_object_id);
        }
        cursor = doc.parent(parent);
    }
    Err(format!("{gauge}'s fillImage has no Mask above it"))
}

/// A text the shared glyph set cannot draw keeps the document's own text.
fn put_text(
    view: &mut UiPrefabView,
    doc: &UiPrefab,
    path: &str,
    text: &str,
    glyphs: &BTreeSet<char>,
    seen: &mut BTreeSet<String>,
) {
    let missing = text
        .chars()
        .find(|ch| !ch.is_whitespace() && !glyphs.contains(ch));
    let Some(missing) = missing else {
        view.set_text(path, text.to_owned());
        return;
    };
    let component: Option<&UiComponent> = doc.find(path).ok().and_then(|index| {
        doc.nodes[index]
            .components
            .iter()
            .find(|component| component.class == "Sekai.UI.CustomTextMesh")
    });
    let keep = component
        .and_then(|component| component.fields.get("m_text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    view.set_text(path, keep);
    if seen.insert(format!("glyph {missing:?}")) {
        warn!(
            "mission: the detail dialog's text has the character {missing:?}, which the shared glyph atlas lacks; the node keeps the document's text"
        );
    }
}

/// Update: `Setup` on the dialog's own view, once its slot is open: the
/// mission's `SetupProgress` / `SetupInformation` node by node.
pub fn setup(
    mut dialog: ResMut<MissionDetailDialog>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    screens: Res<ScreenManager>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    user: Option<Res<ClientMission>>,
    mut views: Query<(&Bindings, &mut UiPrefabView), With<MissionDetailDialogRoot>>,
) {
    let Some(id) = dialog.shown else {
        return;
    };
    if dialog.set_up || !screens.dialogs().any(|(open, _, _)| open == id) {
        return;
    }
    let (Some(masters), Some(user)) = (
        tables.as_deref().and_then(|tables| tables.mission.as_ref()),
        user.as_deref(),
    ) else {
        return;
    };
    let Some(template) = layouts.document(KEY).cloned() else {
        return;
    };
    let Some(mission) = dialog.mission else {
        return;
    };
    let Some(master) = masters.mission(mission) else {
        return;
    };
    let Ok((bindings, mut view)) = views.single_mut() else {
        return;
    };
    dialog.set_up = true;
    let mut state = user.state();
    state.current_sheet = screen::current_sheet(masters, user);
    let view_data = super::model::mission_state(masters, &state, master);
    let data = SetupData {
        mode: dialog.mode,
        sentence: moly_law::text::custom_text_mesh::set_text(&master.sentence, false),
        progress: view_data.map(|view| (view.progress_text(), view.gauge_fill())),
        complete: view_data.is_some_and(|view| view.is_complete()),
        count: dialog.count,
    };
    let bindings = bindings.clone();
    let mut seen = std::mem::take(&mut dialog.seen);
    let result = setup_view(&mut view, &mut layouts, &server, &template, &bindings, &data, &mut seen);
    dialog.seen = seen;
    match result {
        Ok(()) => info!(
            "mission: MysekaiMissionDetailDialog({mission}, {mode:?}): content height {height}, list top {top}, {count} reward row(s), scroll {scroll}; the {REWARD_CELL} document is not in the root, so the list draws no row",
            mode = dialog.mode,
            height = content_height_for(dialog.mode, dialog.count),
            top = list_top(dialog.mode),
            count = dialog.count,
            scroll = if scrollable(dialog.count) { "on" } else { "off" },
        ),
        Err(reason) => error!("mission: MysekaiMissionDetailDialog's Setup is refused: {reason}"),
    }
}

/// `OnClickOK` / `OnClickCancel` / `OnCloseExternal` / the back key: the
/// close.
fn close(by: &str, dialog: &mut MissionDetailDialog, screens: &mut ScreenManager) {
    let Some(id) = dialog.shown.take() else { return };
    info!("mission: MysekaiMissionDetailDialog {id:?}: {by}; DialogBase.Close, the dialog is destroyed");
    screens.close_dialog(id);
    screens.dialog_destroyed(id);
    dialog.set_up = false;
    dialog.mission = None;
    dialog.count = 0;
}

/// Update: `OnClickOK` (the footer's positive button), `OnClickCancel` (the
/// footer's close button) and `OnCloseExternal` (the window's close button);
/// any other tap is the dialog's (its slot is modal).
pub fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut dialog: ResMut<MissionDetailDialog>,
    mut screens: ResMut<ScreenManager>,
    mut consumed: ResMut<ActionTapConsumed>,
    layouts: Res<UiLayouts>,
    world: DialogWorld,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    if dialog.shown.is_none() {
        gestures.clear();
        return;
    }
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(canvas), Ok((bindings, view))) = (
        world.windows.single(),
        world.root_canvas.as_deref(),
        world.views.single(),
    ) else {
        return;
    };
    let size = canvas.size(window);
    let scale = canvas.scale(window);
    for position in taps {
        // The dialog slot is modal: every tap is the dialog's.
        consumed.0 = true;
        let point = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        let hit = |path: &str| {
            view.rect(&layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(point))
        };
        if hit(&bindings.close_button) {
            layouts.queue_button_se(&mut sounds, VIEW, &bindings.close_button);
            close(
                "tap on the window's close button (OnCloseExternal -> OnClickCancel -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
            return;
        }
        if hit(&bindings.footer_close) {
            layouts.queue_button_se(&mut sounds, VIEW, &bindings.footer_close);
            close(
                "tap on the footer's close button (OnClickCancel -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
            return;
        }
        if hit(&bindings.to_screen) {
            layouts.queue_button_se(&mut sounds, VIEW, &bindings.to_screen);
            close(
                "tap on the footer's positive button (OnClickOK -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
            return;
        }
    }
}

/// Update: `Common2ButtonDialog.OnHardwareBackKeyProcess` -> `OnClickCancel`.
pub fn back_key(
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut dialog: ResMut<MissionDetailDialog>,
    mut screens: ResMut<ScreenManager>,
) {
    for event in back_keys.read() {
        if dialog.shown == Some(event.id) {
            close(
                "hardware back key (OnHardwareBackKeyProcess -> OnClickCancel -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
        }
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// `SetupContentElement`'s height (`base + min(count, 3) * 176 - 176` with
    /// `PROGRESS_BASE_CONENT_ELEMENT_HEIGHT` 480 and
    /// `INFORMATION_BASE_CONENT_ELEMENT_HEIGHT` 390), the list tops and the
    /// scrollbar rule (`count != 3`).
    #[test]
    fn the_content_height_and_scrollbar() {
        assert_eq!(progress_height(0), 480.0 - 176.0);
        assert_eq!(progress_height(1), 480.0);
        assert_eq!(progress_height(2), 480.0 + 176.0);
        assert_eq!(progress_height(3), 480.0 + 2.0 * 176.0);
        assert_eq!(progress_height(5), 480.0 + 2.0 * 176.0);
        assert_eq!(information_height(1), 390.0);
        assert_eq!(information_height(4), 390.0 + 2.0 * 176.0);
        assert!(scrollable(0));
        assert!(scrollable(2));
        assert!(!scrollable(3));
        assert!(scrollable(4));
        assert_eq!(list_top(Mode::Progress), -240.0);
        assert_eq!(list_top(Mode::Information), -150.0);
    }
}
