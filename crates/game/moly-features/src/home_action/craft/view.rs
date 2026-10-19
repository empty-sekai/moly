//! The craft screen's view: `ScreenLayerMysekaiCraft` (screen 615) drawn from
//! its own prefab, with its content-list selector's tab cells
//! (`ContentListSelectorCell` under `_tabCellRoot`, one per
//! `ContentListTypes` entry, labelled by `GetContentListTabWordingKey`), the
//! selected type's list prefab (`_listSettingDataList`:
//! `CraftFixtureContentList`, `CraftToolContentList` or
//! `LimitedCraftFixtureContentList`) under `_contentListBaseRoot`, the list's
//! cells (`CraftThumbnailViewItem`), and the shared header. Switching the tab
//! composes the screen again with that type's list prefab.
//!
//! What it shows (the source's order):
//! - **Tab cells**: the selected type's `selectedObj` on; the LimitedCraft
//!   cell active only while the screen's `isActiveLimitedTabCell` holds.
//! - **Cells** (`CraftContentList.OnCreateCell`): per row a
//!   `CraftThumbnailViewItem.Setup(UserResource(target, count), onClick,
//!   isSetDisableCover = !IsCanCraftAnyColor(blueprint), isMission)`. The
//!   count is `GetCraftViewData`: the summed quantity of the target's fixture
//!   rows (a canvas blueprint: its canvas rows; a tool blueprint: the first
//!   `UserMysekaiTools` row of the tool, else 0). `IsCanCraftAnyColor`:
//!   `IsCanCraft(texture 1)`, or any other colour's `IsCanCraft`, where
//!   `IsCanCraft` is not at the create count limit, not at the possession
//!   limit, and `IsEnoughMaterialToCraft(blueprint, 1, dummy material)`. The
//!   thumbnail (`UIPartsItemThumbnail`) hides its base and frame (the fixture
//!   path's `VisibleFrame = false`) and shows the count; a tool's image is
//!   `GetMysekaiToolThumbnail(assetbundleName)`. `SetSelected` marks the
//!   selected row.
//! - **Preview** (`CraftPreview`): the target's name, the owned quantity
//!   (`SetQuantityText`: the selected colour's fixture row, a canvas's
//!   canvas rows summed, a tool's first tool row) with `_quantityObject` on,
//!   the grid size of a fixture or canvas
//!   (`MysekaiFixtureUtility.GetGridSizeText`: floor, rug and road layouts
//!   `FORMAT_MYSEKAI_SITE_LAYOUT_SIZE(width, depth)`, the wall layout
//!   `(width, height)`, any other none, and `_sizeObject` on only for a
//!   text; a tool has none), the material cost cells
//!   (`MaterialCostList.SetupMaterialCostList`:
//!   up to five cells, each with the material's thumbnail, the need quantity
//!   `quantity * count` and the owned quantity; the cells beyond the costs
//!   off; the canvas's stand-in character row as `WORD_THREE_HYPHEN`), the
//!   apply button's label and state and the cannot-craft balloon
//!   (`SetupApplyButton`, the craft screen's). `SetupEmpty` without a
//!   selection: the preview content, the balloon, the quantity and size
//!   objects off, the button disabled.
//! - **Fades**: the home action's screen alpha and raycast switch (the
//!   screen's `_screenCanvasGroup` and the header), `DOFade` with the root's
//!   default ease (OutQuad).
//!
//! Taps (the full-screen layer takes every tap; a shown dialog takes them
//! first): the header's back button pops the screen (`OnClickBackUIScreen`);
//! an active tab cell plays the tab SE and selects its type; a cell selects
//! its row; the apply button, while enabled, is `OnClickApplyButton`. A drag
//! in the list's viewport or the mouse wheel scrolls it.
//!
//! The list is the source ListView's recycling form: a pool of cells covers
//! the visible rows and takes the rows under the scroll position, so a list
//! of a thousand blueprints lays out a few dozen cells.
//!
//! Instrument (off by default; game mode reads none):
//! `MOLY_CRAFT_TAP_TAB=<index>` taps that tab cell at its drawn centre one
//! second after the screen opens, `MOLY_CRAFT_TAP_CELL=<slot>` taps that
//! recycled cell two seconds after, and `MOLY_CRAFT_TAP_APPLY=1` taps the
//! apply button one second later: the view's own tap path.
//!
//! Named gaps: the 3D preview, the colour selector (its cell prefab is not
//! exported), the fixture bonus view, the reaction count view, the system
//! fixture icon and the layout cost (the fixture master carries no put
//! costs) are hidden; the count selector keeps its prefab text; the owned
//! quantity's short-of-material colour is not decoded (the text keeps its
//! serialized colour); the mission labels are hidden; the term text
//! (`SetupCraftTermText`, `MSG_DATE_LIMIT` of the term's end) stays off; the
//! tab cells' separator lines, badges, banners and selected text colour are
//! not built.

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use moly_law::ui::dotween::Ease;
use serde_json::Value;
use std::collections::BTreeSet;

use super::{CRAFT_LIST_TYPES, CraftListType, CraftScreen, SE_TAB_CLICK, Stage, ViewRequest};
use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest};
use crate::fixture_edit_ui::EditorIcons;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::home_action::HomeActionPresentation;
use crate::item_icon::ItemIcons;
use crate::userdata::client::craft::{
    CANVAS_MATERIAL_ID, CraftMasters, CraftOwned, MasterBlueprint, MysekaiCraftType,
    is_create_count_limit, is_enough_material_to_craft, is_reached_possession_limit_to_craft,
};
use crate::userdata::client::inventory::{ClientMysekaiInventory, PossessionMasters};
use crate::userdata::client::tools::ClientMysekaiTools;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The composed screen document.
const RUNTIME: &str = "CraftRuntime";
/// The screen's prefab, the three list prefabs, the tab cell and the list
/// cell (optional documents).
pub const SCREEN: &str = "CraftScreen";
pub const FIXTURE_LIST: &str = "CraftFixtureList";
pub const TOOL_LIST: &str = "CraftToolList";
pub const LIMITED_LIST: &str = "CraftLimitedList";
pub const TAB_CELL: &str = "CraftTabCell";
pub const CELL: &str = "CraftCell";
/// The shared header (`ScreenLayerHeader`).
const HEADER: &str = "EditorHeader";
/// The recycled cells: this many rows of cells cover the viewport (a row is
/// 180 units; the viewport is shorter than eight rows).
const POOL_ROWS: usize = 8;
/// `MaterialCostList.MaxMaterialCostCapacity`.
const MAX_COSTS: usize = 5;

// ---------------------------------------------------------------------------
// Source prefab helpers
// ---------------------------------------------------------------------------

fn component<'a>(
    doc: &'a UiPrefab,
    selector: &str,
    suffix: &str,
) -> Result<&'a UiComponent, String> {
    let node = &doc.nodes[doc.find(selector)?];
    node.components
        .iter()
        .find(|c| c.class.ends_with(suffix))
        .ok_or_else(|| format!("{} lacks source component {suffix}", node.path))
}

/// A local PPtr's path id (0 for null); an external one is an error.
fn pointer(value: &Value) -> Result<i64, String> {
    let p = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or("expected a decoded PPtr")?;
    if p[0].as_i64() != Some(0) {
        return Err("the PPtr is external".into());
    }
    p[1].as_i64().ok_or_else(|| "invalid PPtr path id".into())
}

fn field(fields: &Value, name: &str) -> Result<String, String> {
    match pointer(&fields[name])? {
        0 => Err(format!("required source reference {name} is null")),
        id => Ok(format!("@{id}")),
    }
}

fn optional_field(fields: &Value, name: &str) -> Result<Option<String>, String> {
    if fields.get(name).is_none() {
        return Ok(None);
    }
    Ok(match pointer(&fields[name])? {
        0 => None,
        id => Some(format!("@{id}")),
    })
}

fn cloned(instance: &UiInstance, fields: &Value, name: &str) -> Result<String, String> {
    instance.selector(pointer(&fields[name])?)
}

fn optional_cloned(
    instance: &UiInstance,
    fields: &Value,
    names: &[&str],
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for name in names {
        let id = pointer(&fields[*name])?;
        if id != 0 {
            out.push(instance.selector(id)?);
        }
    }
    Ok(out)
}

fn f32_field(fields: &Value, name: &str) -> Result<f32, String> {
    fields[name]
        .as_f64()
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("source float missing: {name}"))
}

fn template<'a>(layouts: &'a UiLayouts, key: &str) -> Result<&'a UiPrefab, String> {
    layouts
        .document(key)
        .ok_or_else(|| format!("source UI template not loaded: {key}"))
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

/// The list prefab of a type (`_listSettingDataList`).
fn list_document(kind: CraftListType) -> &'static str {
    match kind {
        CraftListType::Craft => FIXTURE_LIST,
        CraftListType::Tool => TOOL_LIST,
        CraftListType::LimitedCraft => LIMITED_LIST,
    }
}

struct TabBinding {
    root: String,
    button: String,
    label: String,
    selected: String,
}

struct CellBinding {
    root: String,
    button: String,
    image: String,
    count: String,
    disabled: String,
    selected: String,
}

struct CostBinding {
    root: String,
    image: String,
    need: String,
    have: String,
}

struct Bindings {
    /// The list type this composition holds.
    kind: CraftListType,
    /// The tab cells, in [`CRAFT_LIST_TYPES`] order.
    tabs: Vec<TabBinding>,
    cells: Vec<CellBinding>,
    hidden: Vec<String>,
    frame_masks: Vec<i64>,
    viewport: String,
    content: String,
    source_content_position: Vec2,
    padding: [f32; 4],
    cell_size: Vec2,
    spacing: Vec2,
    columns: usize,
    scroll_sensitivity: f32,
    nothing_text: String,
    preview_content: String,
    name_text: String,
    quantity_object: String,
    quantity_text: String,
    size_object: String,
    size_text: String,
    term_root: String,
    costs: Vec<CostBinding>,
    apply_button: String,
    apply_text: String,
    apply_cover: Option<String>,
    balloon: String,
    balloon_text: String,
    screen_group: String,
}

fn compose(layouts: &UiLayouts, kind: CraftListType) -> Result<(UiPrefab, Bindings), String> {
    let mut doc = template(layouts, SCREEN)?.clone();
    let screen = component(&doc, &doc.prefab, ".ScreenLayerMysekaiCraft")?
        .fields
        .clone();
    let selector = component(
        &doc,
        &field(&screen, "_contentListSelector")?,
        ".CraftContentListSelector",
    )?
    .fields
    .clone();
    let preview = component(&doc, &field(&screen, "_craftPreview")?, ".CraftPreview")?
        .fields
        .clone();
    let mut hidden = Vec::new();

    // The tab cells, one per content type, in ContentListTypes order.
    let tab_source = template(layouts, TAB_CELL)?;
    let tab_fields = component(tab_source, &tab_source.prefab, ".ContentListSelectorCell")?
        .fields
        .clone();
    let tab_names: Vec<String> = CRAFT_LIST_TYPES
        .iter()
        .map(|kind| format!("Tab{}", *kind as i32))
        .collect();
    let tab_instances = doc.instantiate_children(
        &field(&selector, "_tabCellRoot")?,
        tab_source,
        &tab_source.prefab,
        tab_names.iter().map(String::as_str),
    )?;
    let mut tabs = Vec::new();
    for instance in &tab_instances {
        hidden.extend(optional_cloned(
            instance,
            &tab_fields,
            &[
                "badgeObj",
                "_missionIcon",
                "selectBannerObj",
                "selectBannerCover",
                "lineObj",
                "_iconImage",
            ],
        )?);
        tabs.push(TabBinding {
            root: instance.root_selector(),
            button: cloned(instance, &tab_fields, "button")?,
            label: cloned(instance, &tab_fields, "numberText")?,
            selected: cloned(instance, &tab_fields, "selectedObj")?,
        });
    }

    // The selected type's list prefab under _contentListBaseRoot.
    let list = template(layouts, list_document(kind))?;
    let list_fields = component(list, &list.prefab, ".CraftContentList")?
        .fields
        .clone();
    let list_view = component(list, &field(&list_fields, "listView")?, ".ListView")?
        .fields
        .clone();
    let scroll = component(list, &field(&list_view, "scrollRect")?, ".CustomScrollRect")?
        .fields
        .clone();
    let list_instance = doc.instantiate_subtree(
        &field(&selector, "_contentListBaseRoot")?,
        list,
        &list.prefab,
        "ContentList",
    )?;
    let content = cloned(&list_instance, &scroll, "m_Content")?;
    let source_content_position = Vec2::from_array(
        list.nodes[list.find(&field(&scroll, "m_Content")?)?]
            .rect
            .anchored_position,
    );
    // Not built: sorting, the genre tabs, searching and the hashtags.
    hidden.extend(optional_cloned(
        &list_instance,
        &list_fields,
        &[
            "_sortDropdown",
            "_tabListController",
            "_searchButton",
            "_hashTagFilteredBalloon",
        ],
    )?);

    // The recycled cells under the list's scroll content.
    let columns = list_view["columnCount"]
        .as_i64()
        .filter(|c| *c >= 1)
        .ok_or("ListView columnCount")? as usize;
    let cell_source = template(layouts, CELL)?;
    let cell_fields = component(cell_source, &cell_source.prefab, ".CraftThumbnailViewItem")?
        .fields
        .clone();
    let thumb_fields = component(
        cell_source,
        &field(&cell_fields, "_thumbnail")?,
        ".UIPartsItemThumbnail",
    )?
    .fields
    .clone();
    let loader = component(
        cell_source,
        &field(&thumb_fields, "textureLoader")?,
        ".UITextureLoader",
    )?
    .fields
    .clone();
    let names: Vec<String> = (0..columns * POOL_ROWS)
        .map(|i| format!("Item{i}"))
        .collect();
    let instances = doc.instantiate_children(
        &content,
        cell_source,
        &cell_source.prefab,
        names.iter().map(String::as_str),
    )?;
    let mut cells = Vec::new();
    let mut frame_masks = Vec::new();
    for instance in &instances {
        hidden.extend(optional_cloned(instance, &cell_fields, &["_missionLabel"])?);
        // UIPartsItemThumbnail.Setup: thumbnailBase off; the fixture path's
        // VisibleFrame false (frame image off, frame mask disabled).
        hidden.extend(optional_cloned(
            instance,
            &thumb_fields,
            &["thumbnailBase", "frameImage"],
        )?);
        match pointer(&thumb_fields["frameMask"])? {
            0 => {}
            id => frame_masks.push(instance.identity(id)?),
        }
        hidden.extend(optional_cloned(
            instance,
            &loader,
            &["loadingObject", "notFoundObject"],
        )?);
        cells.push(CellBinding {
            root: instance.root_selector(),
            button: cloned(instance, &thumb_fields, "button")?,
            image: cloned(instance, &thumb_fields, "thumbnailImage")?,
            count: cloned(instance, &thumb_fields, "innerText")?,
            disabled: cloned(instance, &thumb_fields, "disableCover")?,
            selected: cloned(instance, &cell_fields, "_selected")?,
        });
    }
    let padding_values = list_view["padding"]
        .as_array()
        .filter(|p| p.len() == 4)
        .ok_or("ListView padding")?;
    let mut padding = [0.; 4];
    for (out, value) in padding.iter_mut().zip(padding_values) {
        *out = value.as_i64().ok_or("ListView padding integer")? as f32;
    }

    // The preview's parts without a presenter here.
    for name in [
        "_blueprint3DPreview",
        "_colorSelector",
        "_fixtureBonusView",
        "_reactionCountView",
        "_systemFixtureIcon",
    ] {
        hidden.push(field(&preview, name)?);
    }
    if let Some(root) = optional_field(&preview, "_layoutCostRoot")? {
        hidden.push(root);
    }
    // The material cost cells.
    let cost_list = component(
        &doc,
        &field(&preview, "_materialCostList")?,
        ".MaterialCostList",
    )?
    .fields
    .clone();
    hidden.push(field(&cost_list, "_additionalMasterialLine")?);
    let mut costs = Vec::new();
    for cell in cost_list["craftNeedItemCells"]
        .as_array()
        .ok_or("MaterialCostList craftNeedItemCells")?
    {
        let root = match pointer(cell)? {
            0 => return Err("a null material cost cell".into()),
            id => format!("@{id}"),
        };
        let cell_fields = component(&doc, &root, ".MaterialCostCell")?.fields.clone();
        let thumb = component(
            &doc,
            &field(&cell_fields, "thumbnail")?,
            ".UIPartsItemThumbnail",
        )?
        .fields
        .clone();
        let loader = component(&doc, &field(&thumb, "textureLoader")?, ".UITextureLoader")?
            .fields
            .clone();
        // VisibleInnerText false; the loader's indicator is not shown.
        hidden.push(field(&thumb, "innerText")?);
        if let Some(loading) = optional_field(&loader, "loadingObject")? {
            hidden.push(loading);
        }
        costs.push(CostBinding {
            root,
            image: field(&thumb, "thumbnailImage")?,
            need: field(&cell_fields, "needQuantityText")?,
            have: field(&cell_fields, "hasQuantityText")?,
        });
    }
    if costs.len() != MAX_COSTS {
        return Err(format!(
            "the material cost list has {} cells, not {MAX_COSTS}",
            costs.len()
        ));
    }
    let apply_button = field(&preview, "_applyButton")?;
    let apply = component(&doc, &apply_button, ".UIPartsCommonButton")?
        .fields
        .clone();
    let balloon = field(&preview, "_cantCraftBalloon")?;
    let balloon_fields = component(&doc, &balloon, ".UIPartsBalloon")?.fields.clone();
    let bindings = Bindings {
        kind,
        tabs,
        cells,
        hidden,
        frame_masks,
        viewport: cloned(&list_instance, &scroll, "m_Viewport")?,
        content,
        source_content_position,
        padding,
        cell_size: Vec2::new(
            f32_field(&cell_fields, "sizeX")?,
            f32_field(&cell_fields, "sizeY")?,
        ),
        spacing: Vec2::new(
            f32_field(&list_view, "horizontalSpacing")?,
            f32_field(&list_view, "verticalSpacing")?,
        ),
        columns,
        scroll_sensitivity: f32_field(&scroll, "m_ScrollSensitivity")?,
        nothing_text: cloned(&list_instance, &list_fields, "_nothingText")?,
        preview_content: field(&preview, "_previewContent")?,
        name_text: field(&preview, "_nameText")?,
        quantity_object: field(&preview, "_quantityObject")?,
        quantity_text: field(&preview, "_quantityText")?,
        size_object: field(&preview, "_sizeObject")?,
        size_text: field(&preview, "_sizeText")?,
        term_root: field(&preview, "_craftTermTextRoot")?,
        costs,
        apply_text: field(&apply, "customTextMesh")?,
        apply_cover: optional_field(&apply, "coverImage")?,
        apply_button,
        balloon_text: field(&balloon_fields, "message")?,
        balloon,
        screen_group: field(&preview, "_screenCanvasGroup")?,
    };
    Ok((doc, bindings))
}

/// The shared header's back button (`OnClickBackUIScreen`); every other
/// header node is hidden.
fn bind_header(doc: &UiPrefab, view: &mut UiPrefabView) -> Result<String, String> {
    let mut buttons = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .filter(|c| {
            c.class.ends_with(".CustomButton")
                && c.fields["m_OnClick"]["m_Calls"]
                    .as_array()
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call["m_MethodName"].as_str() == Some("OnClickBackUIScreen")
                        })
                    })
        });
    let button = buttons
        .next()
        .ok_or("the header has no OnClickBackUIScreen button")?;
    if buttons.next().is_some() {
        return Err("the header's OnClickBackUIScreen button is ambiguous".into());
    }
    let selector = format!("@{}", button.path_id);
    let back = &doc.nodes[doc.find(&selector)?].path;
    for node in &doc.nodes {
        let is_ancestor = back == &node.path || back.starts_with(&format!("{}/", node.path));
        let is_descendant = node.path.starts_with(&format!("{back}/"));
        if !is_ancestor && !is_descendant {
            view.set_visible(&format!("@{}", node.transform_id), false);
        }
    }
    Ok(selector)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One row's cell data (`GetCraftViewData` and `IsCanCraftAnyColor`).
#[derive(Clone, Copy, Debug)]
struct CellData {
    count: i32,
    can_craft: bool,
}

/// The screen alpha tween (`DOFade`).
#[derive(Clone, Copy, Debug)]
struct Fade {
    from: f32,
    to: f32,
    duration: f32,
    elapsed: f32,
}

impl Fade {
    fn value(&self) -> f32 {
        if self.duration <= 0.0 || self.elapsed >= self.duration {
            return self.to;
        }
        self.from + (self.to - self.from) * Ease::OutQuad.evaluate(self.elapsed, self.duration)
    }
}

#[derive(Resource)]
pub struct CraftViewState {
    scroll: f32,
    dragging_list: bool,
    was_open: bool,
    /// The rows, inventory and tools the cell data was computed from.
    cells_for: Option<(u64, u64, u64)>,
    cell_data: Vec<CellData>,
    /// The first row the pool shows.
    first_row: usize,
    fade: Fade,
    map_pending: bool,
    /// Load paths and gaps already named.
    named: BTreeSet<String>,
    /// Why the screen has no view on this root.
    absent: Option<String>,
    /// The instrument's taps of this opening.
    opened_at: f32,
    tab_tapped: bool,
    tapped: u8,
}

impl Default for CraftViewState {
    fn default() -> Self {
        Self {
            scroll: 0.0,
            dragging_list: false,
            was_open: false,
            cells_for: None,
            cell_data: Vec::new(),
            first_row: 0,
            fade: Fade {
                from: 1.0,
                to: 1.0,
                duration: 0.0,
                elapsed: 0.0,
            },
            map_pending: false,
            named: BTreeSet::new(),
            absent: None,
            opened_at: 0.0,
            tab_tapped: false,
            tapped: 0,
        }
    }
}

impl CraftViewState {
    fn name_once(&mut self, key: String) -> bool {
        self.named.insert(key)
    }
}

#[derive(Component)]
pub struct CraftViewRoot {
    bindings: Bindings,
}

/// The craft screen's document key and its apply button's node path, for a
/// tap at the button's drawn centre.
pub fn apply_button_node(world: &mut World) -> Option<(&'static str, String)> {
    let mut roots = world.query::<&CraftViewRoot>();
    roots
        .iter(world)
        .next()
        .map(|root| (RUNTIME, root.bindings.apply_button.clone()))
}

#[derive(Component)]
pub struct CraftHeader {
    back: String,
}

// ---------------------------------------------------------------------------
// The preview's data
// ---------------------------------------------------------------------------

fn owned<'a>(
    inventory: &'a ClientMysekaiInventory,
    possession: Option<&PossessionMasters>,
) -> CraftOwned<'a> {
    CraftOwned {
        materials: inventory.materials(),
        fixtures: inventory.fixtures(),
        canvases: inventory.canvases(),
        fixture_max_count: possession.and_then(|masters| inventory.fixture_max_count(masters)),
    }
}

/// `IsCanCraft(blueprint, texture)`; a missing input answers false.
fn is_can_craft(
    blueprint: &MasterBlueprint,
    texture: i32,
    masters: &CraftMasters,
    owned: &CraftOwned,
) -> bool {
    let limited = is_create_count_limit(blueprint, texture, owned).unwrap_or(true)
        || is_reached_possession_limit_to_craft(blueprint, texture, owned).unwrap_or(true);
    !limited
        && masters
            .blueprint_costs(blueprint.id)
            .is_some_and(|costs| is_enough_material_to_craft(costs, 1, true, owned.materials))
}

/// `GetCraftViewData` and `IsCanCraftAnyColor` of one row.
fn cell_data(
    blueprint: &MasterBlueprint,
    colors: Option<&[i32]>,
    masters: &CraftMasters,
    owned: &CraftOwned,
    tools: &ClientMysekaiTools,
) -> CellData {
    let count = match blueprint.craft_type {
        MysekaiCraftType::MysekaiCanvas => owned
            .canvases
            .iter()
            .filter(|row| row.mysekai_fixture_id == blueprint.craft_target_id)
            .map(|row| row.quantity)
            .sum(),
        MysekaiCraftType::MysekaiTool => tools.quantity(blueprint.craft_target_id),
        _ => owned
            .fixtures
            .iter()
            .filter(|row| row.mysekai_fixture_id == blueprint.craft_target_id)
            .map(|row| row.quantity)
            .sum(),
    };
    let can_craft = is_can_craft(blueprint, 1, masters, owned)
        || colors.is_some_and(|colors| {
            colors
                .iter()
                .filter(|color| **color != 1)
                .any(|color| is_can_craft(blueprint, *color, masters, owned))
        });
    CellData { count, can_craft }
}

/// `GetGridSizeText`: the layout's size format, `None` for a layout without
/// one.
fn grid_size_text(
    layouts: &UiLayouts,
    layout: &str,
    grid: [i32; 3],
) -> Result<Option<String>, String> {
    let args = match layout {
        "floor" | "rug" | "road" => [grid[0], grid[1]],
        "wall" => [grid[0], grid[2]],
        _ => return Ok(None),
    };
    let format = layouts
        .wordings
        .get("FORMAT_MYSEKAI_SITE_LAYOUT_SIZE")
        .ok_or("FORMAT_MYSEKAI_SITE_LAYOUT_SIZE is not on the root")?;
    moly_law::text::custom_text_mesh::format_wording(
        format,
        &[args[0].to_string(), args[1].to_string()],
    )
    .map(Some)
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// The documents the view draws.
const DOCUMENTS: [&str; 6] = [SCREEN, FIXTURE_LIST, TOOL_LIST, LIMITED_LIST, TAB_CELL, CELL];

/// The documents the view draws that this root lacks, once the root's
/// documents have settled (the shared header, a required document, has
/// loaded); `None` while they are still loading.
fn absent_documents(layouts: &UiLayouts) -> Option<Vec<&'static str>> {
    layouts.document(HEADER)?;
    Some(
        DOCUMENTS
            .into_iter()
            .filter(|key| layouts.document(key).is_none())
            .collect(),
    )
}

/// What a composition sets once: the hidden parts, the disabled frame
/// masks and the tab labels (`SetWordingKey(GetContentListTabWordingKey)`);
/// the label wordings this root lacks are returned (their texts keep the
/// prefab's).
fn apply_static(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    bindings: &Bindings,
) -> Vec<&'static str> {
    for path in &bindings.hidden {
        view.set_visible(path, false);
    }
    for mask in &bindings.frame_masks {
        view.set_behaviour_enabled(*mask, false);
    }
    let mut missing = Vec::new();
    for (tab, kind) in bindings.tabs.iter().zip(CRAFT_LIST_TYPES) {
        match layouts.wordings.get(kind.tab_wording()) {
            Some(text) => view.set_text(&tab.label, text.clone()),
            None => missing.push(kind.tab_wording()),
        }
    }
    missing
}

pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut state: ResMut<CraftViewState>,
    mut spawned: Local<bool>,
) {
    if *spawned || state.absent.is_some() {
        return;
    }
    match absent_documents(&layouts) {
        None => return,
        Some(absent) if !absent.is_empty() => {
            let reason = format!("this root lacks the craft screen documents {absent:?}");
            error!(
                "[craft] {reason}; the craft screen has no view (its logic and instruments still run)"
            );
            state.absent = Some(reason);
            return;
        }
        Some(_) => {}
    }
    if DOCUMENTS
        .into_iter()
        .chain([HEADER])
        .any(|key| !layouts.ready(key, &server))
    {
        return;
    }
    *spawned = true;
    // Every type's list composes (a tab switch composes the screen again).
    for kind in &CRAFT_LIST_TYPES[1..] {
        if let Err(reason) = compose(&layouts, *kind) {
            let reason = format!("the {kind:?} list: {reason}");
            error!("[craft] the craft screen could not be composed ({reason}); it has no view");
            state.absent = Some(reason);
            return;
        }
    }
    let (document, bindings) = match compose(&layouts, CRAFT_LIST_TYPES[0]) {
        Ok(composed) => composed,
        Err(reason) => {
            error!("[craft] the craft screen could not be composed ({reason}); it has no view");
            state.absent = Some(reason);
            return;
        }
    };
    if let Err(reason) = layouts.replace_runtime_document(RUNTIME, document, &server) {
        error!("[craft] the craft screen could not be installed ({reason}); it has no view");
        state.absent = Some(reason);
        return;
    }
    let mut view = UiPrefabView::new(RUNTIME, SITEMAP_LAYER);
    let missing = apply_static(&mut view, &layouts, &bindings);
    if !missing.is_empty() {
        warn!(
            "[craft] this root lacks the tab wordings {missing:?}; those labels keep the prefab's text"
        );
    }
    info!(
        "[craft] the craft screen composed: {} tab cells, {} recycled cells ({} columns), {} material cost cells",
        bindings.tabs.len(),
        bindings.cells.len(),
        bindings.columns,
        bindings.costs.len()
    );
    commands.spawn((
        CraftViewRoot { bindings },
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    let mut header = UiPrefabView::new(HEADER, SITEMAP_LAYER);
    match bind_header(layouts.document(HEADER).expect("ready header"), &mut header) {
        Ok(back) => {
            commands.spawn((
                CraftHeader { back },
                Visibility::Hidden,
                Transform::from_xyz(0., 0., 20.),
                RenderLayers::layer(SITEMAP_LAYER),
                header,
            ));
        }
        Err(reason) => {
            error!("[craft] the header has no back button ({reason}); the back key pops the screen")
        }
    }
}

// ---------------------------------------------------------------------------
// Place
// ---------------------------------------------------------------------------

/// Registers a material's thumbnail (`GetMysekaiMaterialThumbnail`) by its
/// load path; `Err` names why it has none.
fn material_thumbnail(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: Option<&ItemIcons>,
    inputs: Option<&crate::get_resource::GetResourceInputs>,
    material: i32,
) -> Result<String, String> {
    let icon = inputs
        .ok_or("the acquisition module's masters are not installed")?
        .material_icon(material)?;
    let path = format!("mysekai/thumbnail/material/{icon}");
    if layouts.has_runtime_texture(&path) {
        return Ok(path);
    }
    let image = icons
        .ok_or("the item icon index is not installed")?
        .texture(&path, icon)?;
    layouts.register_runtime_texture(&path, &image, server);
    Ok(path)
}

/// Registers a tool's thumbnail (`GetMysekaiToolThumbnail(assetbundleName)`,
/// resource `assetbundleName`) by its load path; `Err` names why it has none.
fn tool_thumbnail(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: Option<&ItemIcons>,
    inputs: Option<&crate::get_resource::GetResourceInputs>,
    tool: i32,
) -> Result<String, String> {
    let bundle = inputs
        .ok_or("the acquisition module's masters are not installed")?
        .tool_bundle(tool)?;
    let path = format!("mysekai/thumbnail/tool/{bundle}");
    if layouts.has_runtime_texture(&path) {
        return Ok(path);
    }
    let image = icons
        .ok_or("the item icon index is not installed")?
        .texture(&path, bundle)?;
    layouts.register_runtime_texture(&path, &image, server);
    Ok(path)
}

/// Draws the screen while 615 is the current screen.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    screen: Res<CraftScreen>,
    mut state: ResMut<CraftViewState>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    (editor_icons, item_icons, inputs): (
        Res<EditorIcons>,
        Option<Res<ItemIcons>>,
        Option<Res<crate::get_resource::GetResourceInputs>>,
    ),
    (masters, inventory, possession, tools): (
        Option<Res<CraftMasters>>,
        Option<Res<ClientMysekaiInventory>>,
        Option<Res<PossessionMasters>>,
        Option<Res<ClientMysekaiTools>>,
    ),
    (presentation, time): (Res<HomeActionPresentation>, Res<Time>),
    mut roots: Query<
        (
            &mut CraftViewRoot,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<CraftHeader>,
    >,
    mut headers: Query<
        (
            &CraftHeader,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<CraftViewRoot>,
    >,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut injected: MessageWriter<GestureEvent>,
) {
    let open = stack.current() == LayerId::MysekaiCraft;
    if open && !state.was_open {
        state.scroll = 0.0;
        state.opened_at = time.elapsed_secs();
        state.tab_tapped = false;
        state.tapped = 0;
        state.map_pending = true;
        state.fade = Fade {
            from: 1.0,
            to: 1.0,
            duration: 0.0,
            elapsed: 0.0,
        };
        if let Some(reason) = &state.absent {
            error!("[craft] PushUIScreen(MysekaiCraft) has no view: {reason}");
        }
    }
    state.was_open = open;
    // DOFade of the screens (the home action's hook).
    if (presentation.screen_alpha - state.fade.to).abs() > f32::EPSILON {
        let from = state.fade.value();
        state.fade = Fade {
            from,
            to: presentation.screen_alpha,
            duration: presentation.fade_seconds,
            elapsed: 0.0,
        };
    } else {
        state.fade.elapsed += time.delta_secs();
    }
    let alpha = state.fade.value();
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    for (_, mut header_view, mut visibility, mut transform) in &mut headers {
        visibility.set_if_neq(if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        crate::canvas::set_scale(&mut transform, scale);
        if let Some(root) = layouts
            .document(HEADER)
            .and_then(|doc| doc.nodes.first())
            .map(|node| format!("@{}", node.transform_id))
        {
            header_view.set_alpha(&root, alpha);
        }
    }
    let Ok((mut root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    visibility.set_if_neq(if open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    crate::canvas::set_scale(&mut transform, scale);
    if !open {
        return;
    }
    // The selected type's list (`OnSelectorCell`: that list shown).
    let kind = CRAFT_LIST_TYPES[screen.list];
    if root.bindings.kind != kind {
        let composed = compose(&layouts, kind).and_then(|(document, bindings)| {
            layouts.replace_runtime_document(RUNTIME, document, &server)?;
            Ok(bindings)
        });
        match composed {
            Ok(bindings) => {
                view.clear_overrides();
                root.bindings = bindings;
                apply_static(&mut view, &layouts, &root.bindings);
                state.scroll = 0.0;
                state.cells_for = None;
                state.map_pending = true;
                info!("[craft] the {kind:?} list shown");
            }
            Err(reason) => {
                if state.name_once(format!("compose {kind:?}")) {
                    error!(
                        "[craft] the {kind:?} list could not be composed ({reason}); the {:?} list stays shown",
                        root.bindings.kind
                    );
                }
            }
        }
    }
    let bindings = &root.bindings;
    view.set_alpha(&bindings.screen_group, alpha);
    for (index, (tab, kind)) in bindings.tabs.iter().zip(CRAFT_LIST_TYPES).enumerate() {
        view.set_visible(
            &tab.root,
            kind != CraftListType::LimitedCraft || screen.limited_tab,
        );
        view.set_visible(&tab.selected, index == screen.list);
    }

    // The rows' cell data, again when the rows or the client copies change.
    let revision = inventory
        .as_deref()
        .map_or(0, |inventory| inventory.revision);
    let tools_revision = tools.as_deref().map_or(0, |tools| tools.revision);
    if state.cells_for != Some((screen.rows_revision, revision, tools_revision)) {
        state.cells_for = Some((screen.rows_revision, revision, tools_revision));
        let empty_tools = ClientMysekaiTools::default();
        let tool_rows = tools.as_deref().unwrap_or(&empty_tools);
        state.cell_data = match (masters.as_deref(), inventory.as_deref()) {
            (Some(masters), Some(inventory)) => {
                let owned = owned(inventory, possession.as_deref());
                screen
                    .rows
                    .iter()
                    .map(|blueprint| {
                        // `IsCanCraftAnyColor` reads the fixture master by the
                        // target id whatever the blueprint's type.
                        let colors = inputs
                            .as_deref()
                            .and_then(|inputs| {
                                inputs.craft_target(false, blueprint.craft_target_id).ok()
                            })
                            .map(|(_, colors)| colors);
                        cell_data(blueprint, colors.as_deref(), masters, &owned, tool_rows)
                    })
                    .collect()
            }
            _ => screen
                .rows
                .iter()
                .map(|_| CellData {
                    count: 0,
                    can_craft: false,
                })
                .collect(),
        };
        state.map_pending = true;
    }

    // The recycled cells: the rows under the scroll position.
    let step = bindings.cell_size + bindings.spacing;
    let rows = screen.rows.len();
    let row_lines = rows.div_ceil(bindings.columns);
    let height = step.y * row_lines as f32 + bindings.padding[2] + bindings.padding[3]
        - if row_lines > 0 {
            bindings.spacing.y
        } else {
            0.
        };
    view.set_size_delta(&bindings.content, Vec2::new(0., height));
    let mut scroll = state.scroll;
    if let Some(viewport) = view.rect(&layouts, &bindings.viewport, canvas) {
        scroll = scroll.clamp(0., (height - viewport.size.y).max(0.));
    }
    state.scroll = scroll;
    view.set_anchored_position(
        &bindings.content,
        bindings.source_content_position + Vec2::Y * scroll,
    );
    let first_line = (((scroll - bindings.padding[2]) / step.y).floor().max(0.0)) as usize;
    let first_row = (first_line * bindings.columns).min(rows.saturating_sub(1));
    let first_row = first_row - first_row % bindings.columns;
    state.first_row = first_row;
    let mut missing = Vec::new();
    for (slot, cell) in bindings.cells.iter().enumerate() {
        let index = first_row + slot;
        let Some(blueprint) = screen.rows.get(index) else {
            view.set_visible(&cell.root, false);
            continue;
        };
        view.set_visible(&cell.root, true);
        view.set_anchored_position(
            &cell.root,
            Vec2::new(
                step.x * (index % bindings.columns) as f32 + bindings.padding[0],
                -step.y * (index / bindings.columns) as f32 - bindings.padding[2],
            ),
        );
        let data = state.cell_data.get(index).copied().unwrap_or(CellData {
            count: 0,
            can_craft: false,
        });
        // UIPartsItemThumbnail.SetupCountText: "{0}".
        view.set_text(&cell.count, data.count.to_string());
        view.set_visible(&cell.disabled, !data.can_craft);
        view.set_visible(&cell.selected, screen.selected == Some(index));
        if blueprint.craft_type == MysekaiCraftType::MysekaiTool {
            match tool_thumbnail(
                &mut layouts,
                &server,
                item_icons.as_deref(),
                inputs.as_deref(),
                blueprint.craft_target_id,
            ) {
                Ok(path) => {
                    view.set_visible(&cell.image, true);
                    view.set_texture(&cell.image, &path);
                }
                Err(reason) => {
                    view.set_visible(&cell.image, false);
                    if state.name_once(format!("tool {}", blueprint.craft_target_id)) {
                        warn!(
                            "[craft] tool {} thumbnail refused: {reason}; the cell shows no image",
                            blueprint.craft_target_id
                        );
                    }
                }
            }
            continue;
        }
        match editor_icons.variant(blueprint.craft_target_id, 1) {
            Some(alias) => {
                view.set_visible(&cell.image, true);
                view.set_texture(&cell.image, alias);
            }
            None => {
                view.set_visible(&cell.image, false);
                missing.push(blueprint.craft_target_id);
            }
        }
    }
    if !missing.is_empty() && state.name_once(format!("cells {missing:?}")) {
        warn!(
            "[craft] the fixture thumbnail catalogue lacks fixtures {missing:?} (texture 1); those cells show no image"
        );
    }
    view.set_visible(&bindings.nothing_text, rows == 0);

    // The preview.
    view.set_visible(&bindings.term_root, false);
    let enabled = screen.button.is_some_and(|button| button.enabled);
    if let Some(cover) = &bindings.apply_cover {
        view.set_visible(cover, !enabled);
    }
    match (screen.selection.as_ref(), screen.button) {
        (Some(selection), button) => {
            view.set_visible(&bindings.preview_content, true);
            view.set_text(&bindings.name_text, selection.name.clone());
            let blueprint = &selection.blueprint;
            // SetupCraftTermText: on for a blueprint with a term row.
            if masters
                .as_deref()
                .is_some_and(|masters| masters.term(blueprint.id).is_some())
                && state.name_once("term text".into())
            {
                warn!(
                    "[craft] SetupCraftTermText: the term's end date (MSG_DATE_LIMIT) is not built; the term text stays off"
                );
            }
            // SetQuantityText.
            let quantity: i32 = match (blueprint.craft_type, inventory.as_deref()) {
                (MysekaiCraftType::MysekaiTool, _) => tools
                    .as_deref()
                    .map_or(0, |tools| tools.quantity(blueprint.craft_target_id)),
                (_, None) => 0,
                (MysekaiCraftType::MysekaiCanvas, Some(inventory)) => {
                    inventory.canvas_quantity(blueprint.craft_target_id)
                }
                (_, Some(inventory)) => inventory
                    .fixture_of_texture(blueprint.craft_target_id, selection.texture().unwrap_or(1))
                    .map_or(0, |row| row.quantity),
            };
            view.set_text(&bindings.quantity_text, quantity.to_string());
            view.set_visible(&bindings.quantity_object, true);
            // SetSizeText: a fixture or canvas blueprint's.
            let size = if blueprint.craft_type == MysekaiCraftType::MysekaiTool {
                Ok(None)
            } else {
                inputs
                    .as_deref()
                    .ok_or_else(|| "the acquisition module's masters are not installed".to_owned())
                    .and_then(|inputs| {
                        let (layout, grid) = inputs.fixture_layout(blueprint.craft_target_id)?;
                        grid_size_text(&layouts, layout, grid)
                    })
            };
            match size {
                Ok(Some(text)) => {
                    view.set_text(&bindings.size_text, text);
                    view.set_visible(&bindings.size_object, true);
                }
                Ok(None) => view.set_visible(&bindings.size_object, false),
                Err(reason) => {
                    view.set_visible(&bindings.size_object, false);
                    if state.name_once(format!("size {reason}")) {
                        error!("[craft] GetGridSizeText: {reason}; the size object is off");
                    }
                }
            }
            // MaterialCostList.SetupMaterialCostList.
            let costs = masters
                .as_deref()
                .and_then(|masters| masters.blueprint_costs(blueprint.id))
                .unwrap_or(&[]);
            for (index, cell) in bindings.costs.iter().enumerate() {
                let Some(cost) = costs.get(index) else {
                    view.set_visible(&cell.root, false);
                    continue;
                };
                view.set_visible(&cell.root, true);
                if cost.id == CANVAS_MATERIAL_ID {
                    // SetupCommonCharacterMaterial.
                    let hyphens = layouts.wordings.get("WORD_THREE_HYPHEN").cloned();
                    view.set_text(&cell.need, hyphens.clone().unwrap_or_default());
                    view.set_text(&cell.have, hyphens.unwrap_or_default());
                    view.set_visible(&cell.image, false);
                    if state.name_once("canvas material".into()) {
                        error!(
                            "[craft] the canvas's stand-in character material image (CanvasMaterialAssetbundleName) is not resolved; its cell shows no image"
                        );
                    }
                    continue;
                }
                let have = inventory.as_deref().map_or(0, |inventory| {
                    inventory.material_quantity(cost.mysekai_material_id)
                });
                view.set_text(&cell.need, (cost.quantity * selection.count).to_string());
                view.set_text(&cell.have, have.to_string());
                match material_thumbnail(
                    &mut layouts,
                    &server,
                    item_icons.as_deref(),
                    inputs.as_deref(),
                    cost.mysekai_material_id,
                ) {
                    Ok(path) => {
                        view.set_visible(&cell.image, true);
                        view.set_texture(&cell.image, &path);
                    }
                    Err(reason) => {
                        view.set_visible(&cell.image, false);
                        if state.name_once(format!("material {}", cost.mysekai_material_id)) {
                            warn!(
                                "[craft] material {} thumbnail refused: {reason}; the image stays off",
                                cost.mysekai_material_id
                            );
                        }
                    }
                }
            }
            if state.name_once("have colour".into()) {
                warn!(
                    "[craft] MaterialCostCell.Setup's owned-quantity colour (short of the need) is not decoded; the text keeps its serialized colour"
                );
            }
            // SetupApplyButton's label and balloon.
            if let Some(button) = button {
                if let Some(label) = layouts.wordings.get(button.label) {
                    view.set_text(&bindings.apply_text, label.clone());
                }
                match button.balloon {
                    Some(key) => {
                        view.set_visible(&bindings.balloon, true);
                        match layouts.wordings.get(key) {
                            Some(text) => view.set_text(&bindings.balloon_text, text.clone()),
                            None => {
                                if state.name_once(format!("balloon {key}")) {
                                    error!(
                                        "[craft] {key} is not on the root; the balloon keeps its prefab text"
                                    );
                                }
                            }
                        }
                    }
                    None => view.set_visible(&bindings.balloon, false),
                }
            } else {
                view.set_visible(&bindings.balloon, false);
            }
        }
        // SetupEmpty.
        (None, _) => {
            view.set_visible(&bindings.preview_content, false);
            view.set_visible(&bindings.balloon, false);
            view.set_visible(&bindings.quantity_object, false);
            view.set_visible(&bindings.size_object, false);
        }
    }

    // The instrument's taps (the view's own tap path).
    let since = time.elapsed_secs() - state.opened_at;
    let target = if !state.tab_tapped && since >= 1.0 {
        state.tab_tapped = true;
        crate::userdata::client::instrument_env("MOLY_CRAFT_TAP_TAB")
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .and_then(|index| bindings.tabs.get(index))
            .map(|tab| ("tab cell", tab.button.clone()))
    } else {
        let target = match state.tapped {
            0 if since >= 2.0 => crate::userdata::client::instrument_env("MOLY_CRAFT_TAP_CELL")
                .and_then(|raw| raw.trim().parse::<usize>().ok())
                .and_then(|slot| bindings.cells.get(slot))
                .map(|cell| ("cell", cell.button.clone())),
            1 if since >= 3.0 => crate::userdata::client::instrument_env("MOLY_CRAFT_TAP_APPLY")
                .map(|_| ("apply button", bindings.apply_button.clone())),
            _ => None,
        };
        if state.tapped < 2 && since >= 2.0 + state.tapped as f32 {
            state.tapped += 1;
        }
        target
    };
    if let Some((what, path)) = target {
        if let Some(rect) = view.rect(&layouts, &path, canvas) {
            let centre = rect.center();
            let position = Vec2::new(
                centre.x * scale + window.width() / 2.0,
                window.height() / 2.0 - centre.y * scale,
            );
            info!(
                "[craft] instrument: tap injected at ({:.0},{:.0}) on the {what}",
                position.x, position.y
            );
            injected.write(GestureEvent {
                kind: GestureKind::Tap,
                state: GestureState::End,
                position,
                delta: Vec2::ZERO,
                ui_owned: false,
            });
        }
    }
    if std::mem::take(&mut state.map_pending) {
        // Window-pixel centres of the controls, for harness taps.
        let size = Vec2::new(window.width(), window.height());
        let pixel = |view: &UiPrefabView, path: &str| {
            view.rect(&layouts, path, canvas).map(|rect| {
                let centre = rect.center() * scale;
                (
                    (centre.x + size.x * 0.5).round() as i32,
                    (size.y * 0.5 - centre.y).round() as i32,
                )
            })
        };
        let back = headers
            .single()
            .ok()
            .and_then(|(header, header_view, _, _)| pixel(header_view, &header.back));
        let tabs: Vec<_> = bindings
            .tabs
            .iter()
            .map(|tab| pixel(&*view, &tab.button))
            .collect();
        let cells: Vec<_> = bindings
            .cells
            .iter()
            .take(3)
            .map(|cell| pixel(&*view, &cell.button))
            .collect();
        let apply = pixel(&*view, &bindings.apply_button);
        info!(
            "[craft] tap map (window px, window {}x{}): back {back:?}, tabs Craft/Tool/LimitedCraft {tabs:?}, first cells {cells:?}, apply {apply:?}; {:?} list, {} rows",
            size.x, size.y, bindings.kind, rows
        );
    }
}

// ---------------------------------------------------------------------------
// Click
// ---------------------------------------------------------------------------

fn canvas_position(position: Vec2, window_size: Vec2, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window_size.x * 0.5,
        window_size.y * 0.5 - position.y,
    ) / scale
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

/// Taps while the screen is open (after the common dialogs, which take them
/// first while shown).
#[allow(clippy::too_many_arguments)]
pub fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    layouts: Res<UiLayouts>,
    (mut screen, mut state, presentation): (
        ResMut<CraftScreen>,
        ResMut<CraftViewState>,
        Res<HomeActionPresentation>,
    ),
    mut consumed: ResMut<ActionTapConsumed>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    roots: Query<(&CraftViewRoot, &UiPrefabView)>,
    headers: Query<(&CraftHeader, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let scrolls: Vec<MouseWheel> = wheel.read().copied().collect();
    if stack.current() != LayerId::MysekaiCraft {
        state.dragging_list = false;
        return;
    }
    // The full-screen layer takes every tap (world rays do not get them).
    consumed.0 |= !events.is_empty();
    if !matches!(screen.stage, Stage::Idle) || !presentation.raycasts {
        return;
    }
    let (Ok((window_entity, window)), Some(root_canvas)) =
        (windows.single(), root_canvas.as_deref())
    else {
        return;
    };
    let Ok((root, view)) = roots.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let bindings = &root.bindings;
    if window.cursor_position().is_some_and(|p| {
        hit(
            view,
            &layouts,
            &bindings.viewport,
            canvas_position(p, size, scale),
            canvas,
        )
    }) {
        for event in scrolls.iter().filter(|event| event.window == window_entity) {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            state.scroll -= delta * bindings.scroll_sensitivity;
        }
    }
    for event in events {
        let point = canvas_position(event.position, size, scale);
        if event.kind == GestureKind::Drag {
            match event.state {
                GestureState::Began => {
                    state.dragging_list = hit(view, &layouts, &bindings.viewport, point, canvas)
                }
                GestureState::Moved if state.dragging_list => state.scroll -= event.delta.y / scale,
                GestureState::Moved => {}
                GestureState::End => state.dragging_list = false,
            }
            continue;
        }
        if !event.kind.is_tap_family() || event.state != GestureState::End {
            continue;
        }
        if let Ok((header, header_view)) = headers.single() {
            if hit(header_view, &layouts, &header.back, point, canvas) {
                layouts.queue_button_se(&mut sounds, header_view.key, &header.back);
                info!("[craft] header back button: BackUIScreen");
                layer_commands.write(LayerCommand::Pop);
                continue;
            }
        }
        // The tab group's cells (`ChangeOtherSE`: each plays the tab SE);
        // an inactive cell is not drawn and takes no tap.
        if let Some(index) = bindings
            .tabs
            .iter()
            .position(|tab| hit(view, &layouts, &tab.button, point, canvas))
        {
            if CRAFT_LIST_TYPES[index] != CraftListType::LimitedCraft || screen.limited_tab {
                sounds.0.push(SeRequest {
                    owner: None,
                    cue: SE_TAB_CLICK.to_owned(),
                    class: SeClass::Ui,
                    source: "craft-tab",
                });
                screen.requests.push(ViewRequest::Tab(index));
                continue;
            }
        }
        if let Some(slot) = bindings
            .cells
            .iter()
            .position(|cell| hit(view, &layouts, &cell.button, point, canvas))
        {
            let index = state.first_row + slot;
            if index < screen.rows.len() {
                layouts.queue_button_se(&mut sounds, view.key, &bindings.cells[slot].button);
                screen.requests.push(ViewRequest::Select(index));
            }
            continue;
        }
        if hit(view, &layouts, &bindings.apply_button, point, canvas) {
            if screen.button.is_some_and(|button| button.enabled) {
                layouts.queue_button_se(&mut sounds, view.key, &bindings.apply_button);
                screen.requests.push(ViewRequest::Apply);
            } else {
                info!("[craft] apply button tapped while disabled: nothing");
            }
        }
    }
}
