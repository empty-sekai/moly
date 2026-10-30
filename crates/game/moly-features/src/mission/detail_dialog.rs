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
//! The reward rows: the list's `listViewItemPrefab` is the
//! `MysekaiMissionRewardCell` prefab, which the UI root carries as a document
//! of its own. The document is not one of the layouts the loader requests, so
//! this unit reads it from the root the runtime draws from and admits it
//! through that root's manifest ([`load_cell`]); the rows are copies of it
//! under `ListView/Viewport/Content`, made by `UiPrefab::instantiate_children`
//! on the runtime copy each `Setup` installs (a replaced document drops the
//! subtrees copied into it, so the copies are made with it).
//!
//! `SetupContentElement` takes
//! `MysekaiMissionUtility.ConvertRewardDataToDetailData(Rewards).Count`,
//! which is not the master's reward-row count: the conversion is
//! [`reward_rows`], its count sizes the content and draws one copy per row,
//! and each copy runs `MysekaiMissionRewardCell.Setup` ([`draw_rows`]).
//!
//! Rendering limits:
//! - A row's label: a resource row names its resource as
//!   `ResourceBoxUtility.GetResourceName` does from the `materials` table
//!   ([`resource_name`]); a status row names its type with
//!   `MysekaiResourceUtility.GetPlayerStatusWord`.
//! - A row's thumbnail comes from `ResourceBoxUtility.CreateThumbnail` or
//!   `MysekaiResourceUtility.CreatePlayerStatusThumbnail`: their prefab and
//!   art (an item's icon bundle, the player status icon bundles) are not in
//!   this root, so the cell's `ThumbnailRoot` stands empty and the gap is
//!   named once.
//! - The list's wheel and drag scroll is not drawn, and `NoneInfo` is not
//!   turned off; the rows are placed as `ListView` places them
//!   ([`row_position`], [`rows_height`]) and the scrollbar object follows
//!   `SetupContentElement` alone.
//! - The root's layout solves the authored rects: the writes reach the
//!   dialog's view, and a lowered list top or a widened label background
//!   shows where the document's own layout groups let the rect follow.
//!
//! Refused by name: `SetupInformation` opens from `MysekaiMissionSheetCell`'s
//! info button, and that cell has no document in this root, so no request
//! reaches it; one that does is refused and no slot is pushed.
//!
//! Named gaps: the window scale tween of `DialogBase` is not drawn and the
//! `DialogSizeFitter` window height is not recomputed, so the document's own
//! layout groups take the content root's new preferred height; the footer
//! close label is the document's serialized text.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiNode, UiPrefab, UiRootManifest};
use moly_masters::convert::ResourceBox;
use moly_masters::mission::{MissionReward, RewardType};
use serde_json::Value;

use super::cell::{component_at, widen_to_text, MissionDialogRequest, REWARD_CELL};
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
const TEXT_CLASS: &str = "Sekai.UI.CustomTextMesh";
const LIST_VIEW_CLASS: &str = "Sekai.UI.ListView";
/// `MysekaiMissionRewardCell`'s document, below the root's directory.
const CELL_PATH: &str = "missioncell/MysekaiMissionRewardCell.json";
/// `MasterResourceBox.resourceBoxPurpose` of a mission's box rewards
/// (`MasterDataManager.GetMasterResourceBox` in
/// `ConvertRewardDataToDetailData`).
const REWARD_BOX_PURPOSE: &str = "mysekai_normal_mission_reward";
/// `MasterResourceBox.resourceBoxType`'s two values
/// `ConvertRewardDataToDetailData` compares: a `list` box becomes its first
/// detail, an `expand` box one row per detail. The spellings are the
/// method's comparisons.
const BOX_LIST: &str = "list";
const BOX_EXPAND: &str = "expand";
/// The brief name of the list's copies of the cell document (`Mysekai0`,
/// `Mysekai1`, ...).
const ROW_NAME: &str = "Mysekai";


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
    /// The list's `padding` (left, right, top, bottom) and
    /// `verticalSpacing`, as `ListView` lays its rows out, and the cell
    /// prefab's name.
    pub(crate) list_padding: [f32; 4],
    pub(crate) list_spacing: f32,
    pub(crate) list_cell: String,
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
        let list_view = doc.nodes[doc.find(&list)?]
            .components
            .iter()
            .find(|component| component.class == LIST_VIEW_CLASS)
            .ok_or_else(|| format!("{list} has no {LIST_VIEW_CLASS}"))?;
        let fields = &list_view.fields;
        // `listViewItemPrefab` names the cell prefab in the list's asset
        // bundle; that prefab is the document [`load_cell`] requests.
        if !names_cell(&fields["listViewItemPrefab"]) {
            return Err(format!("{list}'s listViewItemPrefab is null, so it names no {REWARD_CELL}"));
        }
        let mut list_padding = [0.0; 4];
        for (slot, part) in list_padding
            .iter_mut()
            .zip(fields["padding"].as_array().into_iter().flatten())
        {
            *slot = part.as_f64().unwrap_or(0.0) as f32;
        }
        let gauge_mask = format!("@{}", mask_of(doc, &gauge)?);
        Ok(Self {
            list_padding,
            list_spacing: fields["verticalSpacing"].as_f64().unwrap_or(0.0) as f32,
            list_cell: REWARD_CELL.to_owned(),
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

/// One row of `_rewardDetails` (`MysekaiMissionRewardDetailData`), which
/// `MysekaiMissionRewardCell.Setup` switches on by its `Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RewardRow {
    /// `Type.UserResource` (0): the `UserResource` a box detail names.
    Resource(UserResource),
    /// `Type.ResourceBox` (1): a box the conversion adds whole.
    Box(ResourceBox),
    /// `Type.PlayerStatus` (2): a status reward's master row.
    Status(MissionReward),
}

/// The `resourceType` and `resourceId` of a `resourceBoxes` detail, as
/// `UserResource` reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserResource {
    pub(crate) resource_type: String,
    pub(crate) resource_id: i32,
}

/// `ConvertRewardDataToDetailData`, whose count `SetupContentElement` takes:
/// a box reward is the `mysekai_normal_mission_reward` box its external id
/// names (no row when the table lacks it or the box has no detail), which is
/// a `list` box as its first detail, an `expand` box as one row per detail
/// in `seq` order, and any other type as the box itself; any other reward is
/// one status row. Both the switch and the box the conversion reads are
/// confirmed by the method's source.
pub(crate) fn reward_rows(rewards: &[&MissionReward], boxes: Option<&[ResourceBox]>) -> Vec<RewardRow> {
    let mut rows = Vec::new();
    for reward in rewards {
        match reward.reward_type {
            RewardType::ResourceBox => {
                let Some(box_) = boxes.and_then(|boxes| {
                    boxes.iter().find(|box_| {
                        box_.id == reward.external_id && box_.purpose == REWARD_BOX_PURPOSE
                    })
                }) else {
                    continue;
                };
                if box_.details.is_empty() {
                    continue;
                }
                match box_.box_type.as_deref() {
                    Some(BOX_LIST) => rows.push(RewardRow::Resource(UserResource {
                        resource_type: box_.details[0].resource_type.clone(),
                        resource_id: box_.details[0].resource_id,
                    })),
                    Some(BOX_EXPAND) => rows.extend(box_.details.iter().map(|detail| {
                        RewardRow::Resource(UserResource {
                            resource_type: detail.resource_type.clone(),
                            resource_id: detail.resource_id,
                        })
                    })),
                    Some(_) | None => rows.push(RewardRow::Box(box_.clone())),
                }
            }
            _ => rows.push(RewardRow::Status((*reward).clone())),
        }
    }
    rows
}

/// `MysekaiMissionRewardCell.Setup`'s label for a row.
fn row_label(layouts: &UiLayouts, materials: &Value, row: &RewardRow) -> String {
    match row {
        RewardRow::Resource(resource) => resource_name(materials, resource),
        RewardRow::Box(box_) => box_.name.clone().unwrap_or_default(),
        RewardRow::Status(status) => status
            .reward_type
            .status_word_key()
            .map(|key| layouts.wording(key))
            .unwrap_or_default(),
    }
}

/// `ResourceBoxUtility.GetResourceName(resourceId, resourceType)` for a
/// `materials` resource: the row whose `id` is the resource id, named by its
/// `name`. The client's switch on the resource type (which picks the
/// `materials` table among several) is not read here: a resource of another
/// type, or one the table lacks, draws an empty label rather than a guess.
fn resource_name(materials: &Value, resource: &UserResource) -> String {
    materials["entries"]
        .get(resource.resource_id.to_string())
        .and_then(|row| row["name"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The dialog's state.
#[derive(Resource, Default)]
pub struct MissionDetailDialog {
    pub(crate) shown: Option<DialogId>,
    pub(crate) mode: Mode,
    /// The mission `SetupProgress` / `SetupInformation` shows.
    pub(crate) mission: Option<i32>,
    /// `_rewardDetails`: `ConvertRewardDataToDetailData` of the mission's
    /// rewards, the list's rows.
    pub(crate) rows: Vec<RewardRow>,
    /// The count `SetupContentElement` took (`_rewardDetails.Count`).
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

/// The reward cell document and the manifest that admits it. The handles are
/// kept here: a handle dropped at the end of a frame cancels its load.
#[derive(Resource, Default)]
pub struct RewardCellSource {
    cell: Option<Handle<JsonAsset>>,
    manifest: Option<Handle<JsonAsset>>,
}

/// The cell prefab of the dialog's list, which the top-level loader does not
/// request: read from the root the runtime draws from and admitted through
/// that root's manifest, as the loader admits its own documents. `Err` names
/// what is missing or refused; `Ok(None)` while the documents load.
fn load_cell(
    layouts: &UiLayouts,
    server: &AssetServer,
    json: &Assets<JsonAsset>,
    source: &mut RewardCellSource,
) -> Result<Option<UiPrefab>, String> {
    let Some(root) = layouts.root().map(str::to_owned) else {
        return Err("the UI root is not chosen".into());
    };
    let path = format!("{root}{CELL_PATH}");
    let missing = || format!("{REWARD_CELL} document is not in the root ({path} did not load)");
    let handle = source
        .cell
        .get_or_insert_with(|| server.load::<JsonAsset>(path.clone()))
        .clone();
    match server.load_state(&handle) {
        LoadState::Failed(error) => {
            let missing = missing();
            return Err(format!("{missing}: {error}"));
        }
        LoadState::Loaded => {}
        _ => return Ok(None),
    }
    let Some(document) = json.get(&handle) else {
        return Ok(None);
    };
    let mut cell = UiPrefab::parse(&document.0)
        .map_err(|reason| format!("{REWARD_CELL} document is refused: {reason}"))?;
    if cell.prefab != REWARD_CELL {
        return Err(format!(
            "{path} holds the {} prefab, not {REWARD_CELL}",
            cell.prefab
        ));
    }
    // A region root admits every file it carries through its manifest.
    let manifest = source
        .manifest
        .get_or_insert_with(|| server.load::<JsonAsset>(format!("{root}ui-manifest.json")))
        .clone();
    match server.load_state(&manifest) {
        LoadState::Failed(_) => {}
        LoadState::Loaded => {
            let Some(manifest) = json.get(&manifest) else {
                return Ok(None);
            };
            let identity = UiRootManifest::parse(&manifest.0, &cell.source.region.clone().unwrap_or_default())
                .and_then(|manifest| manifest.admit_prefab(CELL_PATH, &document.0, &cell));
            let identity = identity.map_err(|reason| {
                format!("the root refuses the {REWARD_CELL} document: {reason}")
            })?;
            info!(
                "mission: UI prefab {REWARD_CELL}: {} source nodes from the {} UI root {CELL_PATH}; client {} unity {}; document sha256 {}",
                cell.nodes.len(),
                identity.region,
                identity.client_version,
                identity.unity_version,
                identity.document_sha256,
            );
        }
        _ => return Ok(None),
    }
    cell.rebase_images(&root);
    cell.find(&REWARD_CELL)
        .map_err(|_| format!("{REWARD_CELL} document has no {REWARD_CELL} root"))?;
    Ok(Some(cell))
}

/// `resourceBoxes`, which the convert screen's tables hold; `None` while
/// that table is not in (a box reward then draws no row, as a box the table
/// lacks).
fn boxes_of(convert: Option<&moly_masters::convert::ConvertMasters>) -> Option<&[ResourceBox]> {
    convert?.boxes.as_deref()
}

/// Whether a `listViewItemPrefab` reference names a prefab (rather than
/// being null); which prefab it is cannot be read from the reference, and
/// the document the dialog asks for is the one the list's row has.
fn names_cell(reference: &Value) -> bool {
    reference
        .as_array()
        .filter(|pair| pair.len() == 2)
        .and_then(|pair| pair.get(1))
        .and_then(Value::as_i64)
        .is_some_and(|id| id != 0)
}

/// One copy of the cell template under the list's content, per reward row:
/// copy `row` is named `Mysekai{row}`. The copies keep the template's root
/// position until [`draw_rows`] places them.
fn compose_rows(
    document: &mut UiPrefab,
    cell: &UiPrefab,
    content: &str,
    count: usize,
) -> Result<(), String> {
    let root = cell
        .nodes
        .first()
        .map(|node| node.path.clone())
        .ok_or("the cell document has no root")?;
    let item_name = format!("{root}/ItemName");
    if cell.find(&item_name).is_err() {
        return Err(format!("{REWARD_CELL} has no ItemName text"));
    }
    let names: Vec<String> = (0..count).map(|row| format!("{ROW_NAME}{row}")).collect();
    document
        .instantiate_children(content, cell, &root, names.iter().map(String::as_str))
        .map(|_| ())
}

/// A copy's `ItemName` text, below the copy's root.
fn item_name_of(row: usize) -> String {
    format!("/{ROW_NAME}{row}/ItemName")
}

/// Update: the dialog's runtime document and view once the root's document is
/// in.
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut cell_source: ResMut<RewardCellSource>,
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
    let cell = match load_cell(&layouts, &server, &json, &mut cell_source) {
        Ok(Some(cell)) => cell,
        Ok(None) => return,
        Err(reason) => {
            error!("mission: {KEY} is refused: {reason}");
            return;
        }
    };
    let mut document = match with_height(&template, progress_height(0)) {
        Ok(document) => document,
        Err(reason) => {
            error!("mission: {KEY} is refused: {reason}");
            return;
        }
    };
    // `_rewardListView`'s one placeholder instance: every `Setup` draws its
    // rows into a fresh copy of the document, and a copy cannot be moved into
    // a replaced one.
    if let Err(reason) = compose_rows(&mut document, &cell, &bindings.list_content, 1) {
        error!("mission: {KEY} is refused: {reason}");
        return;
    }
    if let Err(reason) = layouts.replace_runtime_document(VIEW, document, &server) {
        error!("mission: {KEY} is not installed: {reason}");
        return;
    }
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
/// the dialog's own view; this step only resolves the mission and the rows.
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
        // `_rewardListView.CreateViewItem(_rewardDetails.Count)` and
        // `SetupContentElement(..., _rewardDetails.Count)`: the rows
        // `ConvertRewardDataToDetailData(viewData.Rewards)` makes.
        let boxes = boxes_of(tables.as_deref().and_then(|tables| tables.convert.as_ref()));
        let rows = reward_rows(&masters.rewards_of(mission), boxes);
        let count = rows.len();
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
        dialog.rows = rows;
        dialog.count = count;
        dialog.set_up = false;
        info!(
            "mission: MysekaiMissionDetailDialog: Show2ButtonDialog<MysekaiMissionDetailDialog>(339) is open for mission {mission} ({mode:?}); ConvertRewardDataToDetailData makes {count} reward row(s)"
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
    /// `SetupContentElement`'s count (`_rewardDetails.Count`).
    pub count: usize,
    /// `_rewardDetails`, the list's rows.
    pub rows: Vec<RewardRow>,
    /// The UI root's `materials` file, as it holds it: its `entries` object
    /// keyed by `id`.
    pub materials: Value,
}

/// `MysekaiMissionRewardCell.Setup`'s label and row text: the label when the
/// shared glyph set draws it, its empty fallback text otherwise.
fn row_text(
    cell: &UiPrefab,
    path: &str,
    text: &str,
    glyphs: &BTreeSet<char>,
    seen: &mut BTreeSet<String>,
) -> String {
    let missing = text
        .chars()
        .find(|ch| !ch.is_whitespace() && !glyphs.contains(ch));
    let Some(missing) = missing else {
        return text.to_owned();
    };
    let keyed = component_at(cell, path, TEXT_CLASS)
        .and_then(|component| component.fields.get("useWordingKey"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if seen.insert(format!("row glyph {missing:?}")) {
        warn!(
            "mission: a reward row's text has the character {missing:?}, which the shared glyph atlas lacks; the row draws the cell's own text"
        );
    }
    if keyed {
        String::new()
    } else {
        serialized_text(cell, path)
    }
}

/// A `CustomTextMesh`'s serialized text.
fn serialized_text(doc: &UiPrefab, path: &str) -> String {
    component_at(doc, path, TEXT_CLASS)
        .and_then(|component| component.fields.get("m_text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `ListView.GetContentPosition(0)` and the row placement `CreateViewItem`
/// makes, below the content's anchor: row `row` sits one row step
/// (`REWARD_HEIGHT + verticalSpacing`) under the row above it and the first
/// row `padding.top` into the content. How the content's transform maps that
/// onto a RectTransform's anchor (the anchor and pivot the list document
/// authored) is not read from the client here.
fn row_position(bindings: &Bindings, row: usize) -> Vec2 {
    Vec2::new(
        -bindings.list_padding[0],
        -(bindings.list_padding[2] + row as f32 * (REWARD_HEIGHT + bindings.list_spacing)),
    )
}

/// `ListView.SetContentSize`'s height for `count` rows. A snapping list
/// sizes its content to its viewport, which the authored rect carries; this
/// is the other lists' formula.
fn rows_height(bindings: &Bindings, count: usize) -> f32 {
    if count == 0 {
        return bindings.list_height;
    }
    // The authored padding and spacing. A plain list's vertical padding is
    // not read from the client: it is taken as the authored top and the
    // authored bottom.
    count as f32 * (REWARD_HEIGHT + bindings.list_spacing)
        + bindings.list_padding[2]
        + bindings.list_padding[3]
        - bindings.list_spacing
}

/// `MysekaiMissionRewardCell.Setup` on each copy, and the list's placement of
/// the copies (`ListView.CreateViewItem(count)`): the rows are the copies
/// `compose_rows` made, numbered by their names.
fn draw_rows(
    view: &mut UiPrefabView,
    cell: &UiPrefab,
    bindings: &Bindings,
    data: &SetupData,
    layouts: &UiLayouts,
    glyphs: &BTreeSet<char>,
    seen: &mut BTreeSet<String>,
) {
    let item_name = format!(
        "{}/ItemName",
        cell.nodes.first().map(|node| node.path.as_str()).unwrap_or_default()
    );
    for (row, reward) in data.rows.iter().enumerate() {
        let path = format!("{}{}", bindings.list_content, item_name_of(row));
        let label = row_label(layouts, &data.materials, reward);
        let text = row_text(cell, &item_name, &label, glyphs, seen);
        view.set_text(&path, text);
        view.set_anchored_position(
            &format!("{}/{ROW_NAME}{row}", bindings.list_content),
            row_position(bindings, row),
        );
    }
    view.set_size_delta(
        &bindings.list_content,
        Vec2::new(0.0, rows_height(bindings, data.count)),
    );
}

/// `SetupProgress` / `SetupInformation` on the dialog's own view: the copy
/// with `SetupContentElement`'s height and its reward row copies is installed,
/// then the mode's nodes, label, list top, rows and scrollbar are written.
pub fn setup_view(
    view: &mut UiPrefabView,
    layouts: &mut UiLayouts,
    server: &AssetServer,
    template: &UiPrefab,
    cell: &UiPrefab,
    bindings: &Bindings,
    data: &SetupData,
    seen: &mut BTreeSet<String>,
) -> Result<(), String> {
    let mut document = with_height(template, content_height_for(data.mode, data.count))?;
    compose_rows(&mut document, cell, &bindings.list_content, data.count)?;
    layouts.replace_runtime_document(VIEW, document, &server)?;
    // The glyph set covers the documents installed before the copy went in,
    // so it is read here, after the rows' copy.
    let glyphs = layouts.text_chars().into_iter().collect();
    draw_rows(view, cell, bindings, data, layouts, &glyphs, seen);
    // `MysekaiMissionRewardCell.Setup` draws each row's thumbnail under
    // `ThumbnailRoot` (`ThumbnailRoot` stays empty; see the module notes).
    if !data.rows.is_empty() && seen.insert("thumbnail".to_owned()) {
        info!(
            "mission: the reward rows' thumbnails ({REWARD_CELL}'s ThumbnailRoot) are not drawn: their prefab and art are not on this root"
        );
    }
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
    json: Res<Assets<JsonAsset>>,
    mut cell_source: ResMut<RewardCellSource>,
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
    // The cell document the list rows are copied from, read the same way the
    // view's installation reads it; a refusal here names it as it is.
    let cell = match load_cell(&layouts, &server, &json, &mut cell_source) {
        Ok(Some(cell)) => cell,
        Ok(None) => return,
        Err(reason) => {
            error!("mission: MysekaiMissionDetailDialog's Setup is refused: {reason}");
            return;
        }
    };
    // `materials`, let in with the cell: a label that waits for it would
    // never be drawn, since `Setup` runs once.
    let Some(materials) = materials(&layouts, &server, &json) else {
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
        rows: std::mem::take(&mut dialog.rows),
        materials,
    };
    let bindings = bindings.clone();
    let mut seen = std::mem::take(&mut dialog.seen);
    let result = setup_view(
        &mut view,
        &mut layouts,
        &server,
        &template,
        &cell,
        &bindings,
        &data,
        &mut seen,
    );
    dialog.rows = data.rows.clone();
    dialog.seen = seen;
    match result {
        Ok(()) => info!(
            "mission: MysekaiMissionDetailDialog({mission}, {mode:?}): content height {height}, list top {top}, {count} reward row(s) copied from {REWARD_CELL}, scroll {scroll}",
            mode = dialog.mode,
            height = content_height_for(dialog.mode, dialog.count),
            top = list_top(dialog.mode),
            count = dialog.count,
            scroll = if scrollable(dialog.count) { "on" } else { "off" },
        ),
        Err(reason) => error!("mission: MysekaiMissionDetailDialog's Setup is refused: {reason}"),
    }
}

/// `materials`, the table `ResourceBoxUtility.GetResourceName` names a
/// resource from. Its loader parses the table for itself and keeps only the
/// rows it reads, so the resource rows are read from the file the root
/// carries; `None` while it is not in, and `Setup` then waits a frame.
fn materials(layouts: &UiLayouts, server: &AssetServer, json: &Assets<JsonAsset>) -> Option<Value> {
    let handle = server.load::<JsonAsset>(format!("{}tables/materials.json", layouts.root()?));
    match server.load_state(&handle) {
        LoadState::Failed(error) => {
            let reason = format!("the materials table did not load: {error}");
            error!("mission: {reason}");
            None
        }
        LoadState::Loaded => json
            .get(&handle)
            .and_then(|source| serde_json::from_str(&source.0).ok()),
        _ => None,
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
    dialog.rows.clear();
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
