//! `MysekaiMissionCell` on the mission screen's list, with the two list cells
//! of the mission dialogs alongside it.
//!
//! Source structure:
//! - `MysekaiMissionView.CreateMissionList`: the page's own cell is the
//!   sheet's completion mission (`_completeCell`, which the screen unit
//!   draws); the list holds one cell per normal mission whose status is not
//!   received.
//! - `MysekaiMissionView.Refresh`: `ListView.CreateViewItem(count)` then
//!   `SetContentFirstIndex(0)`.
//! - `MysekaiMissionCell.Setup`: the top reward thumbnail under
//!   `RewardThumbnailRoot`, `SetProgress`, `SetMissionContent`,
//!   `SetCompleteLabel`, then `_otherLabel` hidden.
//! - `MissionCellView.SetProgress`: the gauge `Setup(min(current, max), max)`
//!   with the progress clamped at 0, and the text `"{current}/{max}"`.
//! - `MissionCellView.SetMissionContent`: `ChangeNonBreakingSpace` of the
//!   mission's sentence.
//! - `MissionCellView.SetCompleteLabel`: the achievement label on with the
//!   mission complete, and `cellBaceImage.color` `#ffffee` when complete,
//!   `Color.white` otherwise.
//! - `MysekaiMissionView.OnClickMissionCell`: a tapped row reaches
//!   `ScreenLayerMysekaiMission.OnClickMysekaiMissionCell`.
//! - `ListView.SetListViewItemVertical` / `ListView.SetContentSize`: a row's
//!   anchored position is the list's left padding and `-row * (cell height +
//!   spacing) - top padding`, and the content's height is the rows' height
//!   plus the padding, less the last spacing.
//!
//! Host mapping: the list's `listViewItemPrefab` is the
//! `MysekaiMissionScreenCell` prefab, which this root has no document for.
//! The screen document's own `MysekaiMissionCell` (the same cell, holding the
//! completion mission) is the template: `UiPrefab::instantiate_children`
//! copies its subtree once per row under each page's
//! `ListView/Viewport/Content`, each copy names its own button, and the
//! composed document is drawn by [`crate::mission::screen`]'s view, whose
//! `UiPrefabView` accepts the copies' `@<identity>` selectors. A copy's gauge
//! fill keeps its source's serialized reference, so the fill is drawn on the
//! copy's own Mask, which [`GaugeFill`] remaps through the instance. Rows past
//! the page's count stay hidden.
//!
//! The list dialog's `MysekaiMissionSheetCell` is a prefab with no document
//! in this root: [`refuse_document`] names it once, and that list draws no
//! row. The detail dialog's `MysekaiMissionRewardCell` has a document of its
//! own, which that dialog reads for itself.
//!
//! Named gaps:
//! - The reward thumbnails: neither cell's thumbnail prefab nor the reward
//!   art is in this root, and the masters carry no `resourceBoxes` table to
//!   resolve a box reward's contents; `Reward/Base` stands as the cell's one
//!   reward slot. The completion cell's `icon_complete_mission` texture is
//!   not on the root either.
//! - The other-kinds label (`SetupOtherCount`) counts a mission's reward
//!   detail rows, which need those box rows.
//! - A list text with a character the root's documents never draw is not in
//!   the shared glyph atlas (it is baked from those texts, while a mission's
//!   sentence is runtime data): the copy keeps its node's serialized text and
//!   the character is named once.
//! - The list's drag scroll has no inertia or snapping (`CustomScrollRect`'s
//!   velocity and `ListView`'s snap).

use std::collections::BTreeSet;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiNode, UiPrefab};
use serde_json::Value;

use super::model;
use super::screen;
use crate::canvas::RootCanvas;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::ui_inputs;
use crate::ui_layers::ScreenManager;
use crate::ui_layout::{tmp_layout, UiLayouts, UiPrefabView};
use crate::userdata::mission::ClientMission;

/// One cell per listed mission. This branch's sheets hold at most nine normal
/// missions, so twelve rows cover every page.
pub const POOL_ROWS: usize = 12;
/// `ColorUtility.Create("#ffffee")`, `SetCompleteLabel`'s complete colour.
const COMPLETE_COLOR: [f32; 4] = [1.0, 1.0, 238.0 / 255.0, 1.0];
/// `Color.white`, `SetCompleteLabel`'s incomplete colour.
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TEXT_CLASS: &str = "Sekai.UI.CustomTextMesh";
const IMAGE_CLASS: &str = "Sekai.UI.CustomImage";
const BUTTON_CLASS: &str = "Sekai.UI.CustomButton";
const GAUGE_CLASS: &str = "Sekai.UIPartsGauge";
const LIST_VIEW_CLASS: &str = "Sekai.UI.ListView";

/// `MysekaiMissionSheetCell`, the list dialog's row cell.
pub const SHEET_CELL: &str = "MysekaiMissionSheetCell";
/// `MysekaiMissionRewardCell`, the detail dialog's reward row cell.
pub const REWARD_CELL: &str = "MysekaiMissionRewardCell";

/// The two mission dialogs, as the screen's taps ask for them.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionDialogRequest {
    /// `OnClickMissionListButton`:
    /// `Show1ButtonDialog<MysekaiMissionListDialog>(340)` then `Setup`.
    List,
    /// `OnClickMysekaiMissionCell`'s detail path:
    /// `Show2ButtonDialog<MysekaiMissionDetailDialog>(339).SetupProgress` for
    /// this mission's master id.
    Detail(i32),
    /// `MysekaiMissionSheetCell.OnClickInfoButton`:
    /// `Show2ButtonDialog<MysekaiMissionDetailDialog>(339).SetupInformation`
    /// for this mission's master id. Its only caller is the sheet cell, whose
    /// document the root does not carry, so nothing writes it yet.
    #[allow(dead_code)] // Written by the sheet cell's info button once it exists.
    Information(i32),
}

/// Logs a cell prefab's one named refusal: `<cell> document is not in the
/// root`.
pub fn refuse_document(seen: &mut BTreeSet<String>, cell: &str) {
    let message = format!("{cell} document is not in the root");
    if seen.insert(message.clone()) {
        error!("mission: {message}");
    }
}

// ---------------------------------------------------------------------------
// Reading a cell
// ---------------------------------------------------------------------------

/// The serialized component at `path` whose class is `class`.
pub(crate) fn component_at<'a>(doc: &'a UiPrefab, path: &str, class: &str) -> Option<&'a UiComponent> {
    let index = doc.find(path).ok()?;
    doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class == class)
}

/// The `CustomImage` component identity at `path`.
pub(crate) fn image_id(doc: &UiPrefab, path: &str) -> Option<i64> {
    Some(component_at(doc, path, IMAGE_CLASS)?.path_id)
}

/// A `CustomTextMesh`'s serialized width scale fit option.
fn width_fit(doc: &UiPrefab, path: &str) -> bool {
    component_at(doc, path, TEXT_CLASS)
        .and_then(|component| component.fields.get("enableWidthScaleFit"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A TMP text's preferred width through the root's line rules
/// (`CustomTextMesh.preferredWidth`); `None` without the rules.
fn preferred_width(layouts: &UiLayouts, doc: &UiPrefab, path: &str, text: &str) -> Option<f32> {
    let rules = layouts.text_rules.as_ref()?;
    let component = component_at(doc, path, TEXT_CLASS)?;
    let node: &UiNode = &doc.nodes[doc.find(path).ok()?];
    let mut internal = Vec::new();
    tmp_layout::preferred_axis(
        text,
        component,
        0,
        Vec2::from_array(node.rect.size_delta),
        rules,
        false,
        &mut internal,
    )
    .ok()
}

/// `CustomTextMesh.UpdateWordingText`'s rect: a label with width scale fit
/// takes its text's preferred width plus `margin`. A label without the fit,
/// a missing TMP line rule or a missing width keeps the authored rect.
pub(crate) fn widen_to_text(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    doc: &UiPrefab,
    path: &str,
    text: &str,
    margin: f32,
) {
    if !width_fit(doc, path) {
        return;
    }
    let Ok(index) = doc.find(path) else { return };
    let height = doc.nodes[index].rect.size_delta[1];
    if let Some(width) = preferred_width(layouts, doc, path, text) {
        view.set_size_delta(path, Vec2::new(width + margin, height));
    }
}

/// One gauge's fill in a composed document: the Mask's `GameObject` identity
/// and the selector that names it.
#[derive(Debug, Clone)]
pub(crate) struct GaugeFill {
    pub node: i64,
    pub path: String,
}

impl GaugeFill {
    /// The fill of the `CustomImage` at `path` of `doc`, by its `GameObject`.
    pub fn of_image(doc: &UiPrefab, path: &str) -> Option<Self> {
        let path = object_at(doc, path, IMAGE_CLASS)?;
        Some(Self {
            node: path[1..].parse().ok()?,
            path,
        })
    }
}

/// `UIPartsGauge.fillImage` of the gauge component `gauge` read against
/// `source` (the template): the fill object it names.
fn fill_target(source: &UiPrefab, gauge: &UiComponent) -> Option<i64> {
    let pair = gauge.fields.get("fillImage")?.as_array()?;
    if pair.first().and_then(Value::as_i64) != Some(0) {
        return None;
    }
    let id = pair.get(1).and_then(Value::as_i64).filter(|id| *id != 0)?;
    let index = source.find(&format!("@{id}")).ok()?;
    source.nodes[index]
        .components
        .iter()
        .any(|component| component.class == IMAGE_CLASS)
        .then_some(id)
}

/// The `GameObject` reference at `path` whose class is `class`, as a
/// selector of the document it is read from.
pub(crate) fn object_at(doc: &UiPrefab, path: &str, class: &str) -> Option<String> {
    let index = doc.find(path).ok()?;
    doc.nodes[index]
        .components
        .iter()
        .any(|component| component.class == class)
        .then(|| format!("@{}", doc.nodes[index].game_object_id))
}

/// The gauge fills of the copies of a template's gauge, read from `source`
/// (the template) and remapped through each instance: the copy's Mask, the
/// node the fill is drawn from (its `GaugeFill` lies in the Mask, whose reach
/// bounds it as the root's other gauges are drawn).
pub(crate) fn gauge_fills(
    source: &UiPrefab,
    path: &str,
    instances: &[UiInstance],
) -> Result<Vec<Option<GaugeFill>>, String> {
    let target = source
        .find(path)
        .ok()
        .and_then(|index| {
            source.nodes[index]
                .components
                .iter()
                .find(|component| component.class == GAUGE_CLASS)
        })
        .and_then(|gauge| fill_target(source, gauge));
    let Some(target) = target else {
        return Ok(instances.iter().map(|_| None).collect());
    };
    // `<gauge>/GaugeBase/Mask/GaugeFill`: the Mask is the fill's parent.
    let target_index = source.find(&format!("@{target}"))?;
    let mask = source.nodes[target_index].parent_transform_id;
    let mut fills = Vec::with_capacity(instances.len());
    for instance in instances {
        let node = instance.identity(mask)?;
        fills.push(Some(GaugeFill {
            node,
            path: format!("@{node}"),
        }));
    }
    Ok(fills)
}

/// `UIPartsGauge.Setup(fill)`: the gauge's fill is masked to its fraction.
pub(crate) fn set_gauge(view: &mut UiPrefabView, fill: Option<GaugeFill>, value: f32) {
    if let Some(fill) = fill {
        view.set_fill(&fill.path, value);
    }
}

/// The background colour `SetCompleteLabel` gives a cell.
pub(crate) fn complete_color(complete: bool) -> [f32; 4] {
    if complete {
        COMPLETE_COLOR
    } else {
        WHITE
    }
}

// ---------------------------------------------------------------------------
// The screen's list
// ---------------------------------------------------------------------------

/// One page's list of normal mission cells.
#[derive(Debug, Clone)]
pub(crate) struct PageRows {
    /// `ListView/Viewport/Content`.
    pub content: String,
    /// `ListView/Viewport`.
    pub viewport: String,
    /// `ListView/ScrollbarVertical`.
    pub scrollbar: String,
    /// `ListView/NoneInfo`.
    pub none_info: String,
    /// `ListView.padding`, left/right/top/bottom.
    pub padding: [f32; 4],
    pub spacing: f32,
    /// The row step: the cell's height plus the list's spacing.
    pub step: f32,
    /// The content's authored anchor offset.
    pub source: Vec2,
    /// One per copy, at the copy's place in the document.
    pub rows: Vec<ScreenRow>,
    /// The paths, relative to a copy's root, that one row writes.
    pub writes: RowWrites,
    /// The page's own cell's `cellBaceImage` (the completion cell, which
    /// `MysekaiMissionView.Refresh` also passes through `SetCompleteLabel`).
    pub completion_background: i64,
}

/// The paths one copied cell writes, below the copy's root.
#[derive(Debug, Clone)]
pub(crate) struct RowWrites {
    /// `MissionCellView/Root/Mission` (`SetMissionContent`).
    pub mission: String,
    /// `MissionCellView/Root/Progress` (`SetProgress`).
    pub progress: String,
    /// `MissionCellView/Root/AchieveLabel` (`SetCompleteLabel`).
    pub achieve: String,
    /// `MysekaiMissionCell/Others` (`_otherLabel`, hidden by `Setup`).
    pub others: String,
}

/// One copied cell of the screen's list, addressed in the composed document.
#[derive(Debug, Clone)]
pub(crate) struct ScreenRow {
    pub root: String,
    /// `MissionCellView/Root`'s `CustomImage` (`cellBaceImage`).
    pub background: i64,
    /// The instance's `MissionCellView` `CustomButton` identity.
    pub button: i64,
    /// `MissionCellView/Root/UIPartsGauge`'s `fillImage`.
    pub fill: Option<GaugeFill>,
}

/// `ListView.padding` as left, right, top, bottom.
fn padding_of(value: &Value) -> [f32; 4] {
    let mut padding = [0.0; 4];
    for (slot, part) in padding.iter_mut().zip(value.as_array().into_iter().flatten()) {
        *slot = part.as_f64().unwrap_or(0.0) as f32;
    }
    padding
}

/// The `ListView` component of the node at `list`.
fn list_view<'a>(doc: &'a UiPrefab, list: &str) -> Option<&'a UiComponent> {
    let index = doc.find(list).ok()?;
    doc.nodes[index].components.iter().find(|component| {
        component.class == LIST_VIEW_CLASS || component.fields.get("padding").is_some()
    })
}

/// The cell root above a `MissionCellView` path.
fn cell_root_of<'a>(cell_view: &'a str) -> Option<&'a str> {
    let (root, _) = cell_view.split_once("/MissionCellView")?;
    Some(root)
}

/// One copy per row under `page`'s list content, copied from the page's own
/// cell. `Err` names what the document lacks.
pub(crate) fn compose_rows(
    doc: &mut UiPrefab,
    template: &UiPrefab,
    page: &screen::PageBinding,
) -> Result<PageRows, String> {
    let fields = list_view(template, &page.list)
        .ok_or_else(|| format!("{} has no {LIST_VIEW_CLASS}", page.list))?
        .fields
        .clone();
    let padding = padding_of(&fields["padding"]);
    let spacing = fields["verticalSpacing"].as_f64().unwrap_or(0.0) as f32;
    let cell_root = cell_root_of(&page.cell.mission)
        .ok_or_else(|| format!("{} is not below a cell root", page.cell.mission))?
        .to_owned();
    let size = Vec2::from_array(template.nodes[template.find(&cell_root)?].rect.size_delta);
    let content = format!("{}/Viewport/Content", page.list);
    let source = Vec2::from_array(template.nodes[template.find(&content)?].rect.anchored_position);
    let viewport = format!("{}/Viewport", page.list);
    let scrollbar = format!("{}/ScrollbarVertical", page.list);
    for path in [
        &page.cell.mission,
        &page.cell.progress,
        &page.cell.achieve,
        &page.cell.others,
        &page.cell.button.0,
        &page.none_info,
        &viewport,
        &scrollbar,
    ] {
        template
            .find(path)
            .map_err(|_| format!("{path}: the document has no such node"))?;
    }
    let background_path = format!("{cell_root}/MissionCellView/Root");
    let completion_background = image_id(template, &background_path)
        .ok_or_else(|| format!("{background_path} has no {IMAGE_CLASS}"))?;
    let gauge_path = format!("{cell_root}/MissionCellView/Root/UIPartsGauge");
    let names: Vec<String> = (0..POOL_ROWS)
        .map(|index| format!("Mission{index}"))
        .collect();
    let instances = doc.instantiate_children(
        &content,
        template,
        &cell_root,
        names.iter().map(String::as_str),
    )?;
    let fills = gauge_fills(template, &gauge_path, &instances)?;
    let mut rows = Vec::with_capacity(instances.len());
    for (index, (instance, fill)) in instances.iter().zip(fills).enumerate() {
        let root = format!("{content}/Mission{index}");
        let suffix = |path: &str| format!("{root}{}", &path[cell_root.len()..]);
        rows.push(ScreenRow {
            background: image_id(doc, &suffix(&background_path))
                .ok_or_else(|| format!("row {index} has no {IMAGE_CLASS} background"))?,
            button: instance.identity(page.cell.button.1)?,
            fill,
            root,
        });
    }
    Ok(PageRows {
        writes: RowWrites {
            mission: cell_copy(&page.cell.mission, &cell_root),
            progress: cell_copy(&page.cell.progress, &cell_root),
            achieve: cell_copy(&page.cell.achieve, &cell_root),
            others: cell_copy(&page.cell.others, &cell_root),
        },
        content,
        viewport,
        scrollbar,
        none_info: page.none_info.clone(),
        source,
        padding,
        spacing,
        step: size.y + spacing,
        rows,
        completion_background,
    })
}

/// A path of the template's cell below `cell_root`, as this copy's path.
fn cell_copy(path: &str, cell_root: &str) -> String {
    path.strip_prefix(cell_root)
        .map(|rest| rest.to_owned())
        .unwrap_or_else(|| path.to_owned())
}

impl PageRows {
    /// `SetContentSize`: the content's height for `count` rows.
    pub fn content_height(&self, count: usize) -> f32 {
        if count == 0 {
            0.0
        } else {
            self.step * count as f32 + self.padding[2] + self.padding[3] - self.spacing
        }
    }

    /// `SetListViewItemVertical`: row `index`'s anchored position.
    pub fn row_position(&self, index: usize) -> Vec2 {
        Vec2::new(self.padding[0], -(index as f32) * self.step - self.padding[2])
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// The characters the root's documents draw, and with them the shared glyph
/// atlas's coverage: TMP layouts these without the open font.
fn glyph_chars(layouts: &UiLayouts) -> std::collections::HashSet<char> {
    layouts.text_chars().into_iter().collect()
}

/// A cell's text, as a copy draws it: the text when the glyph set covers it,
/// and the `CustomTextMesh`'s own serialized text otherwise — a runtime
/// sentence is not in the set. A keyed text has no serialized text to keep.
fn cell_text(doc: &UiPrefab, path: &str, text: &str, glyphs: &std::collections::HashSet<char>) -> (String, Option<char>) {
    let missing = text
        .chars()
        .find(|ch| !ch.is_whitespace() && !ui_inputs::atlas::tmp_zero_metric(*ch) && !glyphs.contains(ch));
    let Some(missing) = missing else {
        return (text.to_owned(), None);
    };
    let component = component_at(doc, path, TEXT_CLASS);
    let keyed = component
        .and_then(|component| component.fields.get("useWordingKey"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && component
            .and_then(|component| component.fields.get("wordingKey"))
            .and_then(Value::as_str)
            .is_some_and(|key| !key.is_empty());
    let keep = if keyed {
        String::new()
    } else {
        component
            .and_then(|component| component.fields.get("m_text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    (keep, Some(missing))
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The screen's list cells.
#[derive(Resource, Default)]
pub struct MissionCells {
    pub(crate) pages: [Option<PageRows>; 2],
    pub(crate) composed: bool,
    pub(crate) failed: bool,
    /// Each page's scroll from the top, in canvas units.
    scroll: [f32; 2],
    /// Each page's largest scroll.
    max_scroll: [f32; 2],
    dragging: [bool; 2],
    seen: BTreeSet<String>,
}

impl MissionCells {
    /// The row whose `MissionCellView` button is the component `id` on `page`.
    pub fn row_at(&self, page: usize, id: i64) -> Option<usize> {
        let page = self.pages[page & 1].as_ref()?;
        page.rows.iter().position(|row| row.button == id)
    }

    /// The rows the page's list holds.
    #[allow(dead_code)] // Read by a host that measures the list.
    pub fn rows(&self, page: usize) -> usize {
        self.pages[page & 1].as_ref().map_or(0, |page| page.rows.len())
    }

    /// `CreateViewItem` / `SetContentFirstIndex` on a page turned in: the
    /// list starts at its top.
    pub(crate) fn shown(&mut self, page: usize) {
        let page = page & 1;
        self.scroll[page] = 0.0;
        self.dragging[page] = false;
    }
}

// ---------------------------------------------------------------------------
// Composition and drawing
// ---------------------------------------------------------------------------

/// Update: the document with its list rows, once the screen's view exists.
pub fn spawn_cells(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    screen: Res<screen::MissionScreen>,
    mut cells: ResMut<MissionCells>,
    roots: Query<(), With<screen::MissionScreenRoot>>,
) {
    if cells.composed || cells.failed || screen.absent().is_some() || roots.is_empty() {
        return;
    }
    let Some(template) = layouts.document(screen::KEY).cloned() else {
        return;
    };
    let composed = (|| -> Result<[PageRows; 2], String> {
        let bindings = screen::bind(&template)?;
        let mut document = template.clone();
        let first = compose_rows(&mut document, &template, &bindings.pages[0])?;
        let second = compose_rows(&mut document, &template, &bindings.pages[1])?;
        layouts.replace_runtime_document(screen::KEY, document, &server)?;
        Ok([first, second])
    })();
    match composed {
        Ok(pages) => {
            cells.pages = pages.map(Some);
            cells.composed = true;
            info!(
                "mission: {POOL_ROWS} list cells per page, copied from the document's own MysekaiMissionCell subtree (the list's MysekaiMissionScreenCell prefab is outside the document)"
            );
        }
        Err(reason) => {
            cells.failed = true;
            error!("mission: the list cells are not composed: {reason}");
        }
    }
}

/// Update: the current page's rows, their scroll and the list's empty state.
#[allow(clippy::too_many_arguments)]
pub fn advance(
    layouts: Res<UiLayouts>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    user: Option<Res<ClientMission>>,
    screens: Res<ScreenManager>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    screen: Res<screen::MissionScreen>,
    mut cells: ResMut<MissionCells>,
    mut roots: Query<&mut UiPrefabView, With<screen::MissionScreenRoot>>,
) {
    if screens.current_screen() != Some(screen::SCREEN) || screen.absent().is_some() {
        return;
    }
    let Some(model) = tables
        .as_deref()
        .and_then(|tables| tables.mission.as_ref())
        .zip(user.as_deref())
        .and_then(|(masters, user)| screen::read_model(masters, user, screen.sheet()))
    else {
        return;
    };
    let current = screen.current & 1;
    if cells.pages[current].is_none() {
        return;
    }
    // `CreateMissionList`'s order: the completion cell is the page's own and
    // `Refresh`'s `CreateViewItem` lists the normal missions after it.
    let rows: Vec<screen::CompleteCell> = model
        .rows()
        .iter()
        .filter(|row| matches!(row, model::MissionRow::Mission(_)))
        .map(screen::CompleteCell::of)
        .collect();
    let complete = model.parent_view();
    let Ok(mut view) = roots.single_mut() else {
        return;
    };
    let canvas = windows
        .single()
        .ok()
        .zip(root_canvas.as_deref())
        .map(|(window, canvas)| canvas.size(window));
    let MissionCells {
        pages,
        scroll,
        max_scroll,
        seen,
        ..
    } = &mut *cells;
    let Some(page) = pages[current].as_ref() else {
        return;
    };
    // `MysekaiMissionView.Refresh` -> `_completeCell.Setup` -> `SetCompleteLabel`.
    if let Some(complete) = complete {
        view.set_graphic_color(page.completion_background, complete_color(complete.is_complete()));
    }
    let count = rows.len().min(page.rows.len());
    let height = page.content_height(count);
    view.set_size_delta(&page.content, Vec2::new(0.0, height));
    let viewport_height = canvas
        .and_then(|canvas| view.rect(&layouts, &page.viewport, canvas))
        .map_or(0.0, |rect| rect.size.y);
    let max = (height - viewport_height).max(0.0);
    max_scroll[current] = max;
    let offset = scroll[current].clamp(0.0, max);
    scroll[current] = offset;
    view.set_anchored_position(&page.content, page.source + Vec2::Y * offset);
    view.set_visible(&page.scrollbar, max > 0.5);
    view.set_visible(&page.none_info, count == 0);
    let writes = page.writes.clone();
    let glyphs = glyph_chars(&layouts);
    let Some(document) = layouts.document(screen::KEY) else {
        return;
    };
    let mut missing = BTreeSet::new();
    for (index, row) in page.rows.iter().enumerate() {
        view.set_visible(&row.root, index < count);
        view.set_anchored_position(&row.root, page.row_position(index));
        // `_otherLabel` hides in every `Setup`.
        view.set_visible(&format!("{}{}", row.root, writes.others), false);
        let Some(mission) = rows.get(index) else {
            continue;
        };
        // `SetMissionContent`: `ChangeNonBreakingSpace(sentence)`.
        let sentence = moly_law::text::custom_text_mesh::set_text(&mission.mission, false);
        let path = format!("{}{}", row.root, writes.mission);
        let (text, lack) = cell_text(document, &path, &sentence, &glyphs);
        missing.extend(lack);
        view.set_text(&path, text);
        // `SetProgress`.
        let path = format!("{}{}", row.root, writes.progress);
        let (text, lack) = cell_text(document, &path, &mission.progress, &glyphs);
        missing.extend(lack);
        view.set_text(&path, text);
        // `SetProgress`'s gauge, `UIPartsGauge.Setup(min(current, max), max)`.
        set_gauge(&mut view, row.fill.clone(), mission.fill);
        // `SetCompleteLabel`.
        view.set_visible(&format!("{}{}", row.root, writes.achieve), mission.achieved);
        view.set_graphic_color(row.background, complete_color(mission.achieved));
    }
    for ch in missing {
        if seen.insert(format!("glyph {ch:?}")) {
            warn!(
                "mission: a list text has the character {ch:?}, which the shared glyph atlas lacks; the cell keeps the document's text"
            );
        }
    }
    let gaps = [
        (
            "reward art",
            "the mission reward thumbnails are not on this root: their cell prefab and art are not carried; the cells' reward slot keeps its base image",
        ),
        (
            "other kinds",
            "the other-kinds label needs the reward boxes' rows, which the masters do not carry",
        ),
        (
            "complete label",
            "the completion cell's icon_complete_mission texture is not on this root",
        ),
    ];
    for (topic, message) in gaps {
        if seen.insert(topic.to_owned()) {
            info!("mission: {message}");
        }
    }
}

// ---------------------------------------------------------------------------
// Scrolling
// ---------------------------------------------------------------------------

/// Update: the list's wheel and drag scroll while the screen takes input.
pub fn scroll(
    mut wheel: MessageReader<MouseWheel>,
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    screens: Res<ScreenManager>,
    dialog: Res<ShellDialogState>,
    root_canvas: Option<Res<RootCanvas>>,
    roots: Query<&UiPrefabView, With<screen::MissionScreenRoot>>,
    screen: Res<screen::MissionScreen>,
    mut cells: ResMut<MissionCells>,
) {
    let (Ok((window_entity, window)), Some(root_canvas), Ok(view)) = (
        windows.single(),
        root_canvas.as_deref(),
        roots.single(),
    ) else {
        cells.dragging = [false; 2];
        return;
    };
    if screens.current_screen() != Some(screen::SCREEN)
        || screen.absent().is_some()
        || screen.turning()
        || screen.covered()
        || dialog.blocks_field_input()
        || screens.exists_dialog()
    {
        cells.dragging = [false; 2];
        return;
    }
    let current = screen.current & 1;
    let MissionCells {
        pages,
        scroll,
        max_scroll,
        dragging,
        ..
    } = &mut *cells;
    let Some(page) = pages[current].as_ref() else {
        return;
    };
    let size = root_canvas.size(window);
    let scale = root_canvas.scale(window);
    let inside = |point: Vec2| {
        view.rect(&layouts, &page.viewport, size)
            .is_some_and(|rect| rect.active && rect.contains(point))
    };
    let point = |position: Vec2| {
        Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale
    };
    let limit = max_scroll[current];
    if window
        .cursor_position()
        .is_some_and(|cursor| inside(point(cursor)))
    {
        for event in wheel.read().filter(|event| event.window == window_entity) {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            scroll[current] = (scroll[current] - delta).clamp(0.0, limit);
        }
    } else {
        wheel.clear();
    }
    for event in gestures.read() {
        if event.kind != GestureKind::Drag {
            continue;
        }
        match event.state {
            GestureState::Began => dragging[current] = inside(point(event.position)),
            GestureState::Moved if dragging[current] => {
                scroll[current] = (scroll[current] - event.delta.y / scale).clamp(0.0, limit);
            }
            GestureState::Moved => {}
            GestureState::End => dragging[current] = false,
        }
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// `ColorUtility.Create("#ffffee")` divides each 8-bit channel by 255:
    /// 255/255, 255/255 and 238/255.
    #[test]
    fn the_complete_colour_is_ffffee() {
        assert_eq!(COMPLETE_COLOR[0], 255.0 / 255.0);
        assert_eq!(COMPLETE_COLOR[1], 255.0 / 255.0);
        assert_eq!(COMPLETE_COLOR[2], 238.0 / 255.0);
        assert_eq!(complete_color(false), WHITE);
        assert_eq!(complete_color(true), COMPLETE_COLOR);
    }

    /// `ListView.SetListViewItemVertical` and `SetContentSize` with the
    /// screen's own list values (`padding` [40, 0, 8, 0], `verticalSpacing`
    /// 24) and the cell's authored height 198: the step is 222, row 2 sits at
    /// (40, -452), and three rows make 222 * 3 + 8 - 24 = 650.
    #[test]
    fn the_list_rows_are_laid_out_vertically() {
        let page = PageRows {
            content: String::new(),
            viewport: String::new(),
            scrollbar: String::new(),
            none_info: String::new(),
            padding: [40.0, 0.0, 8.0, 0.0],
            spacing: 24.0,
            step: 198.0 + 24.0,
            source: Vec2::ZERO,
            rows: Vec::new(),
            writes: RowWrites {
                mission: String::new(),
                progress: String::new(),
                achieve: String::new(),
                others: String::new(),
            },
            completion_background: 0,
        };
        assert_eq!(page.row_position(0), Vec2::new(40.0, -8.0));
        assert_eq!(page.row_position(2), Vec2::new(40.0, -452.0));
        assert_eq!(page.content_height(0), 0.0);
        assert_eq!(page.content_height(3), 650.0);
    }
}
