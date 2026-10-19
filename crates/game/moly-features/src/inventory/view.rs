//! The inventory screen's composition from its source prefabs: the four
//! content-type tab cells, the selected type's list, its genre or type tab
//! strip and sort controls, its cells, the header's back button, the
//! description page and the recycle dialog's bindings.
//!
//! Every part is read from the source's serialized references (the screen's
//! `_contentListSelector`, `_inventoryPreview` and `_chestLimitText` fields,
//! the list's `listView`, `_sortDropdown`, `_searchButton` and
//! `_tabListController`, and the cells' `thumbnail` and `textureLoader`), so
//! the parts sit where the prefab puts them. Parts without a presenter here
//! (the 3D preview, the system fixture icon, the hashtag balloon) are hidden
//! by name.

use bevy::prelude::*;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiNode, UiPrefab};
use serde_json::Value;

use super::{ContentListType, Masters, Row};
use crate::item_icon::ItemIcons;
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The composed screen document (the screen with its tab cells, the selected
/// list and its cells).
pub const RUNTIME: &str = "InventoryRuntime";
pub const SCREEN: &str = "Inventory";
pub const TAB_CELL: &str = "InventoryTabCell";
pub const ITEM_CELL: &str = "InventoryItemCell";
/// The fixture list's cell (`FixtureSelectCell`), shared with the editor.
pub const FIXTURE_CELL: &str = "EditorCell";
/// The shared header (`ScreenLayerHeader`), shared with the editor.
pub const HEADER: &str = "EditorHeader";
/// The shared two-button dialog the recycle button's confirmation shows.
pub const CONFIRM: &str = "Common2";

/// A client texture load: `UITextureLoader.LoadAsync(path, resource)`.
#[derive(Clone, Debug)]
pub struct IconLoad {
    pub path: String,
    pub resource: String,
}

pub struct TabBinding {
    pub button: String,
    pub label: String,
    pub selected: String,
}

pub struct CellBinding {
    pub root: String,
    pub button: String,
    pub image: String,
    pub quantity: String,
    pub selected: String,
}

/// The list's genre or type tabs and its sort controls.
pub struct TabStripBinding {
    /// `_tabListController`'s cells: the parent tabs, and the cells of the
    /// sub tab list (shown while a parent tab with sub genre tabs is
    /// selected).
    pub tabs: Vec<TabBinding>,
    pub sub_tabs: Vec<TabBinding>,
    /// The sub tab list's root (`UIPartsLeftTabListController`'s sub tab
    /// cells, hidden while no parent tab with sub tabs is selected).
    pub sub_root: Option<String>,
    /// The sort dropdown's button and the text of its current option.
    pub sort_button: String,
    pub sort_label: Option<String>,
    /// The sort order button's ascending and descending graphics
    /// (`UIPartsSortOrder`).
    pub order: [String; 2],
    /// The search button.
    pub search: String,
    /// The fixture filter toggle (`_fixtureFilterToggle`) and its check
    /// graphic.
    pub filter_toggle: Option<String>,
    pub filter_check: Option<String>,
}

/// The description page's root, its text and its site page.
pub struct DescriptionBinding {
    pub root: String,
    pub text: String,
    pub sites: String,
}

/// The recycle button's confirmation (`Common2`, the shared two-button
/// dialog).
pub struct DialogBinding {
    /// The root carries the confirmation (`Common2`).
    pub available: bool,
}

pub struct Bindings {
    pub kind: ContentListType,
    pub rows: Vec<Row>,
    pub tabs: Vec<TabBinding>,
    pub cells: Vec<CellBinding>,
    /// Nodes the host hides: parts without a presenter here and the cell
    /// thumbnails' loading, label and cover graphics.
    pub hidden: Vec<String>,
    /// Frame masks `SetupMysekaiFixture` disables.
    pub frame_masks: Vec<i64>,
    pub viewport: String,
    pub content: String,
    pub source_content_position: Vec2,
    pub padding: [f32; 4],
    pub cell_size: Vec2,
    pub spacing: Vec2,
    pub columns: usize,
    pub scroll_sensitivity: f32,
    pub nothing_text: String,
    pub recycle_button: String,
    pub limit_root: String,
    pub limit_text: String,
    pub limit_numer: String,
    pub limit_denom: String,
    pub preview_object: String,
    pub name_text: String,
    pub fixture_object: String,
    pub item_object: String,
    pub item_preview: String,
    pub strip: TabStripBinding,
    pub description: DescriptionBinding,
    pub dialog: DialogBinding,
}

// ---------------------------------------------------------------------------
// Serialized references
// ---------------------------------------------------------------------------

pub fn component<'a>(
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

/// A decoded PPtr's path id (fileId 0: the document's own objects).
pub fn pointer(value: &Value) -> Result<i64, String> {
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

fn optional_pointer(fields: &Value, name: &str) -> Option<i64> {
    fields
        .get(name)
        .and_then(|value| pointer(value).ok())
        .filter(|id| *id != 0)
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
        let id = pointer(&fields[name])?;
        if id != 0 {
            out.push(instance.selector(id)?);
        }
    }
    Ok(out)
}

/// The local nodes of the screen document under `root` whose path ends with
/// `suffix`, as selectors.
fn under(doc: &UiPrefab, root: &str, suffix: &str) -> Vec<String> {
    doc.nodes
        .iter()
        .filter(|node| node.path == root || node.path.starts_with(&format!("{root}/")))
        .filter(|node| node.path.ends_with(suffix))
        .map(|node| format!("@{}", node.game_object_id))
        .collect()
}

fn text_of_node(node: &UiNode) -> Option<i64> {
    node.components
        .iter()
        .find(|c| c.class.ends_with(".CustomTextMesh"))
        .map(|c| c.path_id)
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

pub fn compose(
    layouts: &UiLayouts,
    kind: ContentListType,
    rows: Vec<Row>,
) -> Result<(UiPrefab, Bindings), String> {
    let mut doc = template(layouts, SCREEN)?.clone();
    let screen = component(&doc, &doc.prefab, ".ScreenLayerMysekaiInventory")?
        .fields
        .clone();
    let selector = component(
        &doc,
        &field(&screen, "_contentListSelector")?,
        ".InventoryContentListSelector",
    )?
    .fields
    .clone();
    let preview = component(
        &doc,
        &field(&screen, "_inventoryPreview")?,
        ".InventoryPreview",
    )?
    .fields
    .clone();
    let preview_loader = component(
        &doc,
        &field(&preview, "_itemPreviewTextureLoader")?,
        ".UITextureLoader",
    )?
    .fields
    .clone();
    let mut hidden = Vec::new();

    // The content-type tab cells, one per type, in ContentListTypes order.
    let tab_source = template(layouts, TAB_CELL)?;
    let tab_fields = component(tab_source, &tab_source.prefab, ".ContentListSelectorCell")?
        .fields
        .clone();
    let tab_names: Vec<String> = super::CONTENT_LIST_TYPES
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
            button: cloned(instance, &tab_fields, "button")?,
            label: cloned(instance, &tab_fields, "numberText")?,
            selected: cloned(instance, &tab_fields, "selectedObj")?,
        });
    }

    // The selected type's list prefab under _contentListBaseRoot.
    let list = template(layouts, kind.list_document())?;
    let list_class = if kind == ContentListType::Fixture {
        ".FixtureInventoryContentList"
    } else {
        ".UserResourceContentList"
    };
    let list_fields = component(list, &list.prefab, list_class)?.fields.clone();
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
    let strip = bind_strip(list, &list_instance, &list_fields, &mut hidden)?;

    // The cells under the list's scroll content.
    let (cell_source, cell_class, thumb_class) = if kind == ContentListType::Fixture {
        (
            template(layouts, FIXTURE_CELL)?,
            ".FixtureSelectCell",
            ".UIPartsFixtureThumbnail",
        )
    } else {
        (
            template(layouts, ITEM_CELL)?,
            ".UIPartsItemThumbnailListViewItem",
            ".UIPartsItemThumbnail",
        )
    };
    let cell_fields = component(cell_source, &cell_source.prefab, cell_class)?
        .fields
        .clone();
    let thumb_fields = component(cell_source, &field(&cell_fields, "thumbnail")?, thumb_class)?
        .fields
        .clone();
    let loader = component(
        cell_source,
        &field(&thumb_fields, "textureLoader")?,
        ".UITextureLoader",
    )?
    .fields
    .clone();
    let cell_names: Vec<String> = (0..rows.len()).map(|i| format!("Item{i}")).collect();
    let instances = doc.instantiate_children(
        &content,
        cell_source,
        &cell_source.prefab,
        cell_names.iter().map(String::as_str),
    )?;
    let mut cells = Vec::new();
    let mut frame_masks = Vec::new();
    for instance in &instances {
        if kind == ContentListType::Fixture {
            hidden.extend(optional_cloned(
                instance,
                &cell_fields,
                &["_missionLabel", "_inPlacedLabel"],
            )?);
            hidden.extend(optional_cloned(instance, &thumb_fields, &["_subThumbnailImage"])?);
            // UIPartsItemThumbnail.SetupMysekaiFixture: thumbnailBase off,
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
        }
        hidden.extend(optional_cloned(
            instance,
            &thumb_fields,
            &["disableCover", "labelImage"],
        )?);
        hidden.extend(optional_cloned(
            instance,
            &loader,
            &["loadingObject", "notFoundObject"],
        )?);
        cells.push(CellBinding {
            root: instance.root_selector(),
            button: cloned(instance, &thumb_fields, "button")?,
            image: cloned(instance, &thumb_fields, "thumbnailImage")?,
            quantity: cloned(instance, &thumb_fields, "innerText")?,
            selected: cloned(instance, &cell_fields, "selected")?,
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
    let columns = list_view["columnCount"]
        .as_i64()
        .filter(|c| *c >= 1)
        .ok_or("ListView columnCount")? as usize;

    // The parts of the preview without a presenter here.
    let mut grid_size_text = None;
    for name in [
        "_blueprint3DPreview",
        "_systemFixtureIcon",
        "_canNotRecycleInfo",
        "_gridSizeText",
    ] {
        if let Some(id) = optional_pointer(&preview, name) {
            if name == "_gridSizeText" {
                // The grid size caption (`GridText`) goes with its text.
                grid_size_text = Some(id);
            }
            hidden.push(format!("@{id}"));
        }
    }
    if let Some(id) = grid_size_text {
        if let Some(parent) = doc.parent(doc.find(&format!("@{id}"))?) {
            hidden.push(format!("@{}", doc.nodes[parent].transform_id));
        }
    }
    for name in ["loadingObject", "notFoundObject"] {
        hidden.extend(optional_pointer(&preview_loader, name).map(|id| format!("@{id}")));
    }

    let description = bind_description(&doc, &preview)?;
    let dialog = bind_dialog(layouts);

    let bindings = Bindings {
        kind,
        rows,
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
        recycle_button: field(&preview, "_recycleButton")?,
        limit_root: field(&screen, "_chestLimitTextRoot")?,
        limit_text: field(&screen, "_chestLimitText")?,
        limit_numer: field(&screen, "_chestLimitNumerText")?,
        limit_denom: field(&screen, "_chestLimitDenomText")?,
        preview_object: field(&preview, "_previewObject")?,
        name_text: field(&preview, "_nameText")?,
        fixture_object: field(&preview, "_fixtureObject")?,
        item_object: field(&preview, "_itemObject")?,
        item_preview: field(&preview, "_itemPreview")?,
        strip,
        description,
        dialog,
    };
    Ok((doc, bindings))
}

fn f32_field(fields: &Value, name: &str) -> Result<f32, String> {
    fields[name]
        .as_f64()
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("source float missing: {name}"))
}

/// The list's sort controls and its genre or type tabs.
///
/// - The sort dropdown (`_sortDropdown` on both list classes) is a
///   `ContentListSortDropDown`: its `_sortDropdown` is the `TMP_Dropdown`,
///   whose `Base/Label` text names the current option, and its
///   `_sortOrderButton` a `UIPartsSortOrder` whose `ascImage` and
///   `descImage` are the order's graphics.
/// - The genre or type tabs (`_tabListController`) are a
///   `UIPartsLeftTabListController`: its `tabList` is the parent tab list
///   (`UIPartsLeftTabList`), its `subTabList` the sub genre list.
/// - The search button (`_searchButton`) and, for the fixture list only, the
///   filter toggle (`_fixtureFilterToggle`) beside the selector.
fn bind_strip(
    list: &UiPrefab,
    instance: &UiInstance,
    fields: &Value,
    hidden: &mut Vec<String>,
) -> Result<TabStripBinding, String> {
    let (sort_button, sort_label, order) = match optional_pointer(fields, "_sortDropdown") {
        Some(id) => {
            let selector = instance.selector(id)?;
            let dropdown = component(list, &format!("@{id}"), ".ContentListSortDropDown")?;
            // The `TMP_Dropdown` object answers the click (`CustomDropdown`
            // draws it); the list's `_sortDropdown` is its host.
            let control = optional_pointer(&dropdown.fields, "_sortDropdown")
                .map(|id| instance.selector(id))
                .transpose()?
                .unwrap_or_else(|| selector.clone());
            let label = dropdown_label(list, instance, id)?;
            let order = match optional_pointer(&dropdown.fields, "_sortOrderButton") {
                Some(button) => {
                    let fields = component(list, &format!("@{button}"), ".UIPartsSortOrder")?
                        .fields
                        .clone();
                    [
                        optional_pointer(&fields, "ascImage")
                            .map(|id| instance.selector(id))
                            .transpose()?,
                        optional_pointer(&fields, "descImage")
                            .map(|id| instance.selector(id))
                            .transpose()?,
                    ]
                }
                None => [None, None],
            };
            if order[0].is_none() || order[1].is_none() {
                hidden.push(selector.clone());
            }
            (control, label, order)
        }
        None => (String::new(), None, [None, None]),
    };
    // `UIPartsLeftTabList` keeps its cells in `cells`; the sub list
    // (`subTabList`) its sub genre cells in its own `cells`.
    let (tabs, sub_tabs, sub_root) = match optional_pointer(fields, "_tabListController") {
        Some(id) => {
            let controller = component(list, &format!("@{id}"), ".UIPartsLeftTabListController")?;
            let parent = optional_pointer(&controller.fields, "tabList")
                .map(|id| tab_cells(list, instance, id, ".UIPartsLeftTabList"))
                .transpose()?
                .unwrap_or_default();
            let sub = optional_pointer(&controller.fields, "subTabList");
            let sub_tabs = sub
                .map(|id| tab_cells(list, instance, id, ".UIPartsLeftTabSubList"))
                .transpose()?
                .unwrap_or_default();
            let sub_root = sub.map(|id| instance.selector(id)).transpose()?;
            (parent, sub_tabs, sub_root)
        }
        None => (Vec::new(), Vec::new(), None),
    };
    let search = match optional_pointer(fields, "_searchButton") {
        Some(id) => instance.selector(id)?,
        None => String::new(),
    };
    let filter = optional_pointer(fields, "_fixtureFilterToggle")
        .map(|id| instance.selector(id))
        .transpose()?;
    let filter_check = match filter.as_ref() {
        Some(selector) => {
            let toggle = component(list, selector, ".CustomToggle")?;
            optional_pointer(&toggle.fields, "graphic")
                .map(|id| instance.selector(id))
                .transpose()?
        }
        None => None,
    };
    Ok(TabStripBinding {
        tabs,
        sub_tabs,
        sub_root,
        sort_button,
        sort_label,
        order: [
            order[0].clone().unwrap_or_default(),
            order[1].clone().unwrap_or_default(),
        ],
        search,
        filter_toggle: filter,
        filter_check,
    })
}

/// The `TMP_Dropdown`'s current option label: its `Base/Label` text.
fn dropdown_label(
    list: &UiPrefab,
    instance: &UiInstance,
    dropdown: i64,
) -> Result<Option<String>, String> {
    let root = list.nodes[list.find(&format!("@{dropdown}"))?].path.clone();
    let label = list
        .nodes
        .iter()
        .find(|node| node.path == format!("{root}/Base/Label"))
        .and_then(text_of_node)
        .map(|id| instance.selector(id))
        .transpose()?;
    Ok(label)
}

/// A `UIPartsLeftTabList` or `UIPartsLeftTabListSubList`'s cells (`cells`),
/// each with its first text as its label.
fn tab_cells(
    list: &UiPrefab,
    instance: &UiInstance,
    id: i64,
    class: &str,
) -> Result<Vec<TabBinding>, String> {
    let holder = component(list, &format!("@{id}"), class)?;
    let mut cells = Vec::new();
    for cell in holder.fields.get("cells").and_then(Value::as_array).into_iter().flatten() {
        let Ok(source) = pointer(cell) else { continue };
        if source == 0 {
            continue;
        }
        let Ok(selector) = instance.selector(source) else {
            continue;
        };
        let path = list.nodes[list.find(&selector)?].path.clone();
        let label = list
            .nodes
            .iter()
            .filter(|node| node.path.starts_with(&format!("{path}/")))
            .filter_map(text_of_node)
            .next()
            .map(|id| instance.selector(id))
            .transpose()?;
        cells.push(TabBinding {
            button: selector.clone(),
            label: label.unwrap_or_else(|| selector.clone()),
            selected: selector,
        });
    }
    Ok(cells)
}

/// `InventoryPreview`'s description page (`_descriptionView`): the page's
/// `DescriptionTextView` and its `SiteNameGroup` (the site names).
fn bind_description(doc: &UiPrefab, preview: &Value) -> Result<DescriptionBinding, String> {
    let root = field(preview, "_descriptionView")?;
    let root_path = doc.nodes[doc.find(&root)?].path.clone();
    let first = |suffix: &str| under(doc, &root_path, suffix).into_iter().next();
    // `DescriptionCell`'s text: the `DescriptionTextView` object (its
    // `CustomTextMesh` sits below it) and its `SiteNameGroup` object.
    let text = first("/DescriptionTextView/CustomTextMesh")
        .ok_or("the description view has no DescriptionTextView")?;
    let sites = first("/SiteNameGroup").unwrap_or_default();
    Ok(DescriptionBinding {
        root,
        text,
        sites,
    })
}

/// The confirmation (the shared two-button dialog), when the root carries
/// it.
fn bind_dialog(layouts: &UiLayouts) -> DialogBinding {
    let mut binding = DialogBinding { available: false };
    let Some(doc) = layouts.document(CONFIRM) else {
        return binding;
    };
    match crate::two_button_dialog::resolve(doc) {
        Ok(_) => binding.available = true,
        Err(reason) => warn!("[inventory] the recycle confirmation is not composed: {reason}"),
    }
    binding
}

fn template<'a>(layouts: &'a UiLayouts, key: &str) -> Result<&'a UiPrefab, String> {
    layouts
        .document(key)
        .ok_or_else(|| format!("source UI template not loaded: {key}"))
}

// ---------------------------------------------------------------------------
// The view's static content
// ---------------------------------------------------------------------------

/// A CustomButton's cover graphics: shown while the button is disabled.
pub fn set_enabled(view: &mut UiPrefabView, doc: &UiPrefab, selector: &str, enabled: bool) {
    let Ok(button) = component(doc, selector, ".CustomButton") else {
        return;
    };
    let covers = std::iter::once(&button.fields["coverImage"]).chain(
        button.fields["optionalCoverImages"]
            .as_array()
            .into_iter()
            .flatten(),
    );
    for cover in covers {
        match pointer(cover) {
            Ok(0) | Err(_) => {}
            Ok(id) => view.set_visible(&format!("@{id}"), !enabled),
        }
    }
}

/// The shared header's back button (`OnClickBackUIScreen`); every other
/// header node is hidden.
pub fn bind_header(doc: &UiPrefab, view: &mut UiPrefabView) -> Result<String, String> {
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
    set_enabled(view, doc, &selector, true);
    Ok(selector)
}

/// Registers the images of the rows' cell and preview loads the shared icon
/// index answers, keyed by the load path, before a view binds them.
pub fn register_icons(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: &ItemIcons,
    masters: &Masters,
    kind: ContentListType,
    rows: &[Row],
) {
    for row in rows {
        for family in kind.icon_families() {
            let Some(load) = masters.icon_path(kind, row, family) else {
                continue;
            };
            if layouts.has_runtime_texture(&load.path) {
                continue;
            }
            if let Ok(image) = icons.texture(&load.path, &load.resource) {
                layouts.register_runtime_texture(&load.path, &image, server);
            }
        }
    }
}

/// Sets an image from its client load path; a path the runtime root does not
/// provide leaves the image off and is named once.
pub fn set_icon(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    node: &str,
    load: Option<IconLoad>,
    refused: &mut Vec<IconLoad>,
) {
    match load {
        Some(load) if layouts.has_runtime_texture(&load.path) => {
            view.set_visible(node, true);
            view.set_texture(node, &load.path);
        }
        Some(load) => {
            view.set_visible(node, false);
            refused.push(load);
        }
        None => view.set_visible(node, false),
    }
}

/// The texts that do not change with the selection.
#[allow(clippy::too_many_arguments)]
pub fn apply_static(
    view: &mut UiPrefabView,
    doc: &UiPrefab,
    layouts: &UiLayouts,
    bindings: &Bindings,
    masters: &Masters,
    icons: &crate::fixture_edit_ui::EditorIcons,
    refused: &mut Vec<IconLoad>,
) {
    for path in &bindings.hidden {
        view.set_visible(path, false);
    }
    for mask in &bindings.frame_masks {
        view.set_behaviour_enabled(*mask, false);
    }
    // SetRecycleButtonEnable: the button answers for a selected row of a
    // list that recycles (`recycle::set_button`).
    set_enabled(view, doc, &bindings.recycle_button, true);
    for (tab, kind) in bindings.tabs.iter().zip(super::CONTENT_LIST_TYPES) {
        // SetWordingKey(GetContentListTabWordingKey(type)).
        if let Some(text) = layouts.wordings.get(kind.tab_wording()) {
            view.set_text(&tab.label, text.clone());
        }
        set_enabled(view, doc, &tab.button, true);
    }
    // The fixture tab strip and the filter toggle belong to the fixture
    // list; the search button and the sort controls to every list.
    if let Some(toggle) = bindings.strip.filter_toggle.as_ref() {
        view.set_visible(toggle, bindings.kind == ContentListType::Fixture);
    }
    let mut missing_fixtures = Vec::new();
    for (cell, row) in bindings.cells.iter().zip(&bindings.rows) {
        // UIPartsThumbnail.SetQuantity: "×{0}".
        view.set_text(&cell.quantity, format!("×{}", row.quantity()));
        match bindings.kind {
            ContentListType::Fixture => match icons.variant(row.id, row.texture) {
                Some(alias) => view.set_texture(&cell.image, alias),
                None => {
                    view.set_visible(&cell.image, false);
                    missing_fixtures.push((row.id, row.texture));
                }
            },
            ContentListType::Material | ContentListType::Item | ContentListType::Tool => set_icon(
                view,
                layouts,
                &cell.image,
                masters.icon_path(bindings.kind, row, "thumbnail"),
                refused,
            ),
        }
    }
    if !missing_fixtures.is_empty() {
        warn!(
            "[inventory] the fixture thumbnail catalogue lacks (fixture id, texture id) {missing_fixtures:?}; those cells show no image"
        );
    }
    view.set_visible(&bindings.nothing_text, bindings.rows.is_empty());
}

/// Names each refused icon once.
pub fn name_refusals(
    state: &mut super::InventoryState,
    icons: &ItemIcons,
    refused: Vec<IconLoad>,
) {
    for load in refused {
        if !state.refused_icons.insert(load.path.clone()) {
            continue;
        }
        let reason = icons
            .texture(&load.path, &load.resource)
            .err()
            .unwrap_or_else(|| "the icon index answers it but it is not registered".into());
        warn!(
            "[inventory] icon {} refused: {reason}; the image stays off",
            load.path
        );
    }
}
