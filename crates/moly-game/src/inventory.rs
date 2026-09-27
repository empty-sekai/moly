//! The MySekai inventory screen (`ScreenLayerMysekaiInventory`, screen 607):
//! the owned fixtures, materials, items and tools, laid out from the
//! screen's own prefab and the list and cell prefabs its content-list
//! selector instantiates.
//!
//! ## Entry
//!
//! The menu's chest button (`MysekaiMenuDialog.OnClickChestButton`: when
//! `ScreenManager.IsActiveScreen(607)` it only closes; else
//! `PushUIScreen(607)` with no boot argument, then `Close`) pushes the screen
//! through the screen manager. The header's back button
//! (`OnClickBackUIScreen`) and the back key pop it. Without a boot argument
//! the screen selects the first cell of each list.
//!
//! ## What the screen shows (the source's order)
//!
//! `OnInitComponent`: `_contentListSelector.Setup(ContentListTypes)` with
//! `ContentListTypes` = Fixture (6), Material (7), Item (8), Tool (9) and
//! `isCalcColumnCount` false, so each list keeps its ListView's serialized
//! column count. The selector instantiates one tab cell per type from
//! `_tabCellPrefab` under `_tabCellRoot`, its label the wording
//! `MysekaiUtility.GetContentListTabWordingKey(type)` (WORD_FIXTURE,
//! WORD_ELEMENT, WORD_ITEM, WORD_TOOL), and the list prefab of each type from
//! `_listSettingDataList` under `_contentListBaseRoot`. Then
//! `UpdateLimitText(Fixture)` and the first item of every list.
//!
//! - **Fixture list** (`FixtureInventoryContentList`, view type Default):
//!   every `UserMysekaiFixture` row with quantity at least 1 and a master
//!   row, as a `FixtureSelectCell` whose thumbnail count is `×{quantity}`
//!   (`UIPartsThumbnail.SetQuantity`, format `×{0}`).
//! - **Material and item lists** (`UserResourceContentList`): the owned rows
//!   with quantity above 0, as `UIPartsItemThumbnailListViewItem` cells with
//!   the same count format. `UIPartsItemThumbnail.Setup` loads the cell image
//!   from `AssetBundleNames.GetMysekaiMaterialThumbnail` /
//!   `GetMysekaiItemThumbnail` (`mysekai/thumbnail/material/` or
//!   `mysekai/thumbnail/item/` followed by the master's
//!   `iconAssetbundleName`).
//! - **Chest limit** (`UpdateLimitText`): the root is active for Fixture and
//!   Material only. Fixture: `WORD_CHEST_FURNITURE_NUM`, have =
//!   `GetFixtureTotalCount` (the sum of every fixture row's quantity), limit
//!   = `GetFixtureMaxCount`; Material: `WORD_CHEST_MATERIAL_NUM`, have =
//!   `GetTotalMaterialPossession`, limit = `GetMaterialMaxCount`. The have
//!   count goes to `_chestLimitDenomText`, coloured by `GetDenomTextColor`
//!   (have below limit: `ColorUtility.WHITE_ALPHA_1`; else
//!   `ColorUtility.COLOR_PINK` = `#ff55AA`); the limit goes to
//!   `_chestLimitNumerText`.
//! - **Preview** (`InventoryPreview`): the selected row's master name
//!   (`RefreshNameText`); for a material or an item (`SetMaterial` /
//!   `SetItem`) the item object with the image of
//!   `GetMysekaiMaterialPreview` / `GetMysekaiItemPreview`
//!   (`mysekai/item_preview/material/` or `mysekai/item_preview/item/`
//!   followed by `iconAssetbundleName`); for a fixture the fixture object.
//!   `ShowEmpty` when the list has no rows: the preview object, the recycle
//!   button and the cannot-recycle info off.
//!
//! The owned rows and the possession levels are the client's copy of the
//! server data (`crate::server::client::inventory`); the possession limits
//! are master data the server model installs. The thumbnail and preview
//! images are resolved by their client load path and resource through the
//! shared item icon index ([`crate::item_icon`]); a path the index does not
//! carry, or a root without the index, is refused by name and the image stays
//! off.
//!
//! The screen's own prefab, its four list prefabs, its tab cell and its item
//! cell are documents a root may lack (the CN roots carry none of them). A
//! root without one of them names the absent documents once; the screen then
//! refuses by name in the frame it is pushed and asks the screen manager to
//! pop it.
//!
//! ## Named gaps (not built here)
//!
//! - The genre tab list, the sort dropdown, the search button, the hashtag
//!   balloon and the fixture filter toggle are hidden: rows keep the order of
//!   the client's copy, and the fixture list stays in its Default view.
//! - Recycle and disassemble: the recycle button stands disabled and the
//!   cannot-recycle info is hidden.
//! - The 3D preview, the system fixture icon, the description pages and the
//!   grid size text (`MysekaiFixtureUtility.GetGridSizeText`) are hidden.
//! - Canvas rows (`UserMysekaiCanvas`) are not listed; the client copy has
//!   no tool rows, so the tool list shows its nothing text.
//! - The tab cell's icon is hidden (the selector's cell settings carry no
//!   sprite for these four types) and its label keeps the serialized colour.
//! - The header shows its back button only.

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::item_icon::ItemIcons;
use crate::server::client::inventory::{ClientMysekaiInventory, PossessionMasters};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The composed screen document (the screen with its tab cells, the selected
/// list and its cells).
const RUNTIME: &str = "InventoryRuntime";
const SCREEN: &str = "Inventory";
const TAB_CELL: &str = "InventoryTabCell";
const ITEM_CELL: &str = "InventoryItemCell";
/// The fixture list's cell (`FixtureSelectCell`), shared with the editor.
const FIXTURE_CELL: &str = "EditorCell";
/// The shared header (`ScreenLayerHeader`), shared with the editor.
const HEADER: &str = "EditorHeader";

/// `ScreenLayerMysekaiInventory.ContentListTypes`.
const CONTENT_LIST_TYPES: [ContentListType; 4] = [
    ContentListType::Fixture,
    ContentListType::Material,
    ContentListType::Item,
    ContentListType::Tool,
];

/// The characters the screen writes besides the master names and wordings.
pub(crate) const FIXED_TEXTS: &[&str] = &["×0123456789"];

/// The wordings the screen writes.
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_FIXTURE",
    "WORD_ELEMENT",
    "WORD_ITEM",
    "WORD_TOOL",
    "WORD_CHEST_FURNITURE_NUM",
    "WORD_CHEST_MATERIAL_NUM",
];

/// `Sekai.Mysekai.ContentList.ContentListType` (the four the screen lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentListType {
    Fixture = 6,
    Material = 7,
    Item = 8,
    Tool = 9,
}

impl ContentListType {
    /// `MysekaiUtility.GetContentListTabWordingKey`.
    fn tab_wording(self) -> &'static str {
        match self {
            Self::Fixture => "WORD_FIXTURE",
            Self::Material => "WORD_ELEMENT",
            Self::Item => "WORD_ITEM",
            Self::Tool => "WORD_TOOL",
        }
    }

    /// The layout document of the type's list prefab.
    fn list_document(self) -> &'static str {
        match self {
            Self::Fixture => "InventoryFixtureList",
            Self::Material => "InventoryMaterialList",
            Self::Item => "InventoryItemList",
            Self::Tool => "InventoryToolList",
        }
    }

    /// The load-path folder of the type's resource icons.
    fn icon_folder(self) -> Option<&'static str> {
        match self {
            Self::Material => Some("material"),
            Self::Item => Some("item"),
            Self::Fixture | Self::Tool => None,
        }
    }
}

/// One listed row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Fixture {
        id: i32,
        texture: i32,
        quantity: i32,
    },
    Resource {
        id: i32,
        quantity: i32,
    },
}

impl Row {
    fn quantity(self) -> i32 {
        match self {
            Row::Fixture { quantity, .. } | Row::Resource { quantity, .. } => quantity,
        }
    }
}

/// A master row's name and icon (`iconAssetbundleName`; none for fixtures).
struct MasterRow {
    name: String,
    icon: Option<String>,
}

/// The master rows the screen reads (fixtures, materials, items).
#[derive(Default)]
struct Masters {
    fixtures: HashMap<i32, MasterRow>,
    materials: HashMap<i32, MasterRow>,
    items: HashMap<i32, MasterRow>,
}

impl Masters {
    fn row(&self, kind: ContentListType, row: Row) -> Option<&MasterRow> {
        match (kind, row) {
            (ContentListType::Fixture, Row::Fixture { id, .. }) => self.fixtures.get(&id),
            (ContentListType::Material, Row::Resource { id, .. }) => self.materials.get(&id),
            (ContentListType::Item, Row::Resource { id, .. }) => self.items.get(&id),
            _ => None,
        }
    }

    /// The client load of a row's cell image (`GetMysekaiMaterialThumbnail`
    /// / `GetMysekaiItemThumbnail`) or preview image (`GetMysekaiMaterialPreview`
    /// / `GetMysekaiItemPreview`): the load path (the folder then the master's
    /// icon name) and the resource, the icon name itself.
    fn icon_path(&self, kind: ContentListType, row: Row, family: &str) -> Option<IconLoad> {
        let folder = kind.icon_folder()?;
        let icon = self.row(kind, row)?.icon.as_deref()?;
        Some(IconLoad {
            path: format!("mysekai/{family}/{folder}/{icon}"),
            resource: icon.to_owned(),
        })
    }
}

const FIXTURE_MASTER: &str = "moly://mysekai-fixtures.json";
const MATERIAL_MASTER: &str = "moly://mysekai-materials.json";
const ITEM_MASTER: &str = "moly://mysekai-items.json";

/// The master tables, requested at start-up; their names reach the shared
/// text atlas through `texts`.
#[derive(Resource)]
pub(crate) struct InventoryGlyphs {
    handles: Option<[Handle<JsonAsset>; 3]>,
    masters: Option<Masters>,
    pub(crate) texts: Option<Vec<String>>,
}

/// A client texture load: `UITextureLoader.LoadAsync(path, resource)`.
#[derive(Clone)]
struct IconLoad {
    path: String,
    resource: String,
}

/// Registers the images of the rows' cell and preview loads the shared icon
/// index answers, keyed by the load path, before a view binds them.
fn register_icons(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: &ItemIcons,
    masters: &Masters,
    kind: ContentListType,
    rows: &[Row],
) {
    for row in rows {
        for family in ["thumbnail", "item_preview"] {
            let Some(load) = masters.icon_path(kind, *row, family) else {
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

pub(crate) fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    commands.init_resource::<InventoryState>();
    // A stage draws no screens: nothing to request, nothing to add.
    let handles = stage
        .is_none()
        .then(|| [FIXTURE_MASTER, MATERIAL_MASTER, ITEM_MASTER].map(|path| server.load(path)));
    let texts = handles.is_none().then(Vec::new);
    commands.insert_resource(InventoryGlyphs {
        handles,
        masters: None,
        texts,
    });
}

fn id_of(row: &Value) -> Result<i32, String> {
    row["id"]
        .as_i64()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| "a master row has no 32-bit id".to_owned())
}

/// `mysekai-fixtures.json`: the `fixtures` array, each row with its name.
fn fixture_rows(value: &Value) -> Result<HashMap<i32, MasterRow>, String> {
    let rows = value["fixtures"]
        .as_array()
        .ok_or("the fixture master has no fixtures array")?;
    rows.iter()
        .map(|row| {
            let id = id_of(row)?;
            let name = row["name"]
                .as_str()
                .ok_or_else(|| format!("fixture master row {id} has no name"))?;
            Ok((
                id,
                MasterRow {
                    name: name.to_owned(),
                    icon: None,
                },
            ))
        })
        .collect()
}

/// A keyed master table (`entries` by id), each row with its name and
/// `iconAssetbundleName`.
fn keyed_rows(value: &Value, table: &str) -> Result<HashMap<i32, MasterRow>, String> {
    if value["semantics"]["table"].as_str() != Some(table) {
        return Err(format!("the document is not the {table} master"));
    }
    let entries = value["entries"]
        .as_object()
        .ok_or_else(|| format!("{table} has no entries"))?;
    entries
        .values()
        .map(|row| {
            let id = id_of(row)?;
            let text = |field: &str| {
                row[field]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{table} row {id} has no {field}"))
            };
            Ok((
                id,
                MasterRow {
                    name: text("name")?,
                    icon: Some(text("iconAssetbundleName")?),
                },
            ))
        })
        .collect()
}

pub(crate) fn parse_glyphs(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut glyphs: ResMut<InventoryGlyphs>,
) {
    if glyphs.texts.is_some() {
        return;
    }
    let Some(handles) = glyphs.handles.clone() else {
        return;
    };
    let paths = [FIXTURE_MASTER, MATERIAL_MASTER, ITEM_MASTER];
    let mut values = Vec::with_capacity(handles.len());
    for (handle, path) in handles.iter().zip(paths) {
        if let bevy::asset::LoadState::Failed(error) = server.load_state(handle) {
            error!(
                "[inventory] {path} failed to load ({error:?}): the screen lists no rows and writes no names"
            );
            glyphs.masters = Some(Masters::default());
            glyphs.texts = Some(Vec::new());
            return;
        }
        let Some(asset) = jsons.get(handle) else {
            return;
        };
        values.push(
            serde_json::from_str::<Value>(&asset.0)
                .unwrap_or_else(|e| panic!("[inventory] {path}: {e}")),
        );
    }
    let masters = Masters {
        fixtures: fixture_rows(&values[0])
            .unwrap_or_else(|e| panic!("[inventory] {FIXTURE_MASTER}: {e}")),
        materials: keyed_rows(&values[1], "mysekaiMaterials")
            .unwrap_or_else(|e| panic!("[inventory] {MATERIAL_MASTER}: {e}")),
        items: keyed_rows(&values[2], "mysekaiItems")
            .unwrap_or_else(|e| panic!("[inventory] {ITEM_MASTER}: {e}")),
    };
    let mut texts: Vec<String> = masters
        .fixtures
        .values()
        .chain(masters.materials.values())
        .chain(masters.items.values())
        .map(|row| row.name.clone())
        .collect();
    texts.extend(FIXED_TEXTS.iter().map(|text| (*text).to_owned()));
    info!(
        "[inventory] masters: {} fixtures, {} materials, {} items",
        masters.fixtures.len(),
        masters.materials.len(),
        masters.items.len()
    );
    glyphs.masters = Some(masters);
    glyphs.texts = Some(texts);
}

// ---------------------------------------------------------------------------
// Composition from the source prefabs
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

/// Non-null references of the screen document itself (not cloned).
fn optional_local(fields: &Value, names: &[&str]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for name in names {
        match pointer(&fields[*name])? {
            0 => {}
            id => out.push(format!("@{id}")),
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

struct TabBinding {
    button: String,
    label: String,
    selected: String,
}

struct CellBinding {
    root: String,
    button: String,
    image: String,
    quantity: String,
    selected: String,
}

struct Bindings {
    kind: ContentListType,
    rows: Vec<Row>,
    tabs: Vec<TabBinding>,
    cells: Vec<CellBinding>,
    /// Nodes the host hides: parts without a presenter here, the cell
    /// thumbnails' loading, label and cover graphics.
    hidden: Vec<String>,
    /// Frame masks `SetupMysekaiFixture` disables.
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
    recycle_button: String,
    limit_root: String,
    limit_text: String,
    limit_numer: String,
    limit_denom: String,
    preview_object: String,
    name_text: String,
    fixture_object: String,
    item_object: String,
    item_preview: String,
}

fn template<'a>(layouts: &'a UiLayouts, key: &str) -> Result<&'a UiPrefab, String> {
    layouts
        .document(key)
        .ok_or_else(|| format!("source UI template not loaded: {key}"))
}

fn compose(
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

    // The tab cells, one per content type, in ContentListTypes order.
    let tab_source = template(layouts, TAB_CELL)?;
    let tab_fields = component(tab_source, &tab_source.prefab, ".ContentListSelectorCell")?
        .fields
        .clone();
    let tab_names: Vec<String> = CONTENT_LIST_TYPES
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
    // Not built here: sorting, searching, the genre tabs and hashtags.
    let not_built: &[&str] = if kind == ContentListType::Fixture {
        &[
            "_sortDropdown",
            "_tabListController",
            "_searchButton",
            "_hashTagFilteredBalloon",
        ]
    } else {
        &["_sortDropdown", "_tabListController"]
    };
    hidden.extend(optional_cloned(&list_instance, &list_fields, not_built)?);

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
            hidden.extend(optional_cloned(
                instance,
                &thumb_fields,
                &["_subThumbnailImage"],
            )?);
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

    // The fixture filter toggle (Placeable / InPlaced) is not built.
    hidden.push(field(&selector, "_fixtureFilterToggle")?);
    for name in [
        "_blueprint3DPreview",
        "_systemFixtureIcon",
        "_descriptionView",
        "_canNotRecycleInfo",
        "_gridSizeText",
    ] {
        hidden.push(field(&preview, name)?);
    }
    // The grid size row's caption goes with its text.
    hidden.push("PreviewObject/FixtureObject/GridText".to_owned());
    hidden.extend(optional_local(
        &preview_loader,
        &["loadingObject", "notFoundObject"],
    )?);
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
    };
    Ok((doc, bindings))
}

/// A CustomButton's cover graphics: shown while the button is disabled.
fn set_enabled(view: &mut UiPrefabView, doc: &UiPrefab, selector: &str, enabled: bool) {
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

// ---------------------------------------------------------------------------
// State, spawn and the per-frame view
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub(crate) struct InventoryState {
    /// Index into [`CONTENT_LIST_TYPES`].
    tab: usize,
    /// The selected cell of each list (`SelectFixtureFirstItem` /
    /// `SelectUserResourceFirstItem` select index 0).
    selected: [usize; 4],
    scroll: f32,
    dragging_list: bool,
    was_open: bool,
    /// The tap map is written after the next placement.
    map_pending: bool,
    /// Load paths already refused by name (each is named once).
    refused_icons: BTreeSet<String>,
    /// Why the screen cannot be drawn: the root lacks some of its documents.
    absent: Option<String>,
    /// The refused screen's pop has been requested (reset once it is gone).
    refusal_popped: bool,
}

#[derive(Component)]
pub(crate) struct InventoryRoot {
    bindings: Bindings,
}

/// The header view with its back button.
#[derive(Component)]
pub(crate) struct InventoryHeader {
    back: String,
}

#[derive(Resource)]
pub(crate) struct InventorySpawned;

/// The listed rows of a type: fixtures with quantity at least 1 and a master
/// row (`UserFixtureArrayToFixtureThumbnailData`, view type Default); owned
/// materials and items with quantity above 0.
fn rows(
    kind: ContentListType,
    owned: Option<&ClientMysekaiInventory>,
    masters: &Masters,
) -> Vec<Row> {
    let Some(owned) = owned else {
        return Vec::new();
    };
    match kind {
        ContentListType::Fixture => owned
            .fixtures()
            .iter()
            .filter(|row| {
                row.quantity >= 1 && masters.fixtures.contains_key(&row.mysekai_fixture_id)
            })
            .map(|row| Row::Fixture {
                id: row.mysekai_fixture_id,
                texture: row.texture_id,
                quantity: row.quantity,
            })
            .collect(),
        ContentListType::Material => owned
            .materials()
            .iter()
            .filter(|row| row.quantity > 0)
            .map(|row| Row::Resource {
                id: row.mysekai_material_id,
                quantity: row.quantity,
            })
            .collect(),
        ContentListType::Item => owned
            .items()
            .iter()
            .filter(|row| row.quantity > 0)
            .map(|row| Row::Resource {
                id: row.mysekai_item_id,
                quantity: row.quantity,
            })
            .collect(),
        ContentListType::Tool => Vec::new(),
    }
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
    set_enabled(view, doc, &selector, true);
    Ok(selector)
}

/// Sets an image from its client load path; a path the runtime root does not
/// provide leaves the image off and is named once.
fn set_icon(
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

fn apply_static(
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
    set_enabled(view, doc, &bindings.recycle_button, false);
    for (tab, kind) in bindings.tabs.iter().zip(CONTENT_LIST_TYPES) {
        // SetWordingKey(GetContentListTabWordingKey(type)).
        if let Some(text) = layouts.wordings.get(kind.tab_wording()) {
            view.set_text(&tab.label, text.clone());
        }
        set_enabled(view, doc, &tab.button, true);
    }
    let mut missing_fixtures = Vec::new();
    for (cell, row) in bindings.cells.iter().zip(&bindings.rows) {
        // UIPartsThumbnail.SetQuantity: "×{0}".
        view.set_text(&cell.quantity, format!("×{}", row.quantity()));
        match *row {
            Row::Fixture { id, texture, .. } => match icons.variant(id, texture) {
                Some(alias) => view.set_texture(&cell.image, alias),
                None => {
                    view.set_visible(&cell.image, false);
                    missing_fixtures.push((id, texture));
                }
            },
            Row::Resource { .. } => set_icon(
                view,
                layouts,
                &cell.image,
                masters.icon_path(bindings.kind, *row, "thumbnail"),
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

fn name_refusals(state: &mut InventoryState, icons: &ItemIcons, refused: Vec<IconLoad>) {
    let new: Vec<IconLoad> = refused
        .into_iter()
        .filter(|load| state.refused_icons.insert(load.path.clone()))
        .collect();
    for load in new {
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

/// The documents the screen draws that this root lacks, once the root's
/// documents have settled (the shared header, a required document, has
/// loaded); `None` while they are still loading.
fn absent_documents(layouts: &UiLayouts) -> Option<Vec<&'static str>> {
    layouts.document(HEADER)?;
    Some(
        [SCREEN, TAB_CELL, ITEM_CELL]
            .into_iter()
            .chain(CONTENT_LIST_TYPES.map(ContentListType::list_document))
            .filter(|key| layouts.document(key).is_none())
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    glyphs: Res<InventoryGlyphs>,
    icons: Res<crate::fixture_edit_ui::EditorIcons>,
    owned: Option<Res<ClientMysekaiInventory>>,
    mut state: ResMut<InventoryState>,
    icon_index: Option<Res<ItemIcons>>,
    spawned: Option<Res<InventorySpawned>>,
) {
    if spawned.is_some() || state.absent.is_some() {
        return;
    }
    match absent_documents(&layouts) {
        None => return,
        Some(absent) if !absent.is_empty() => {
            let reason = format!("this root lacks the inventory documents {absent:?}");
            error!("[inventory] {reason}; the screen refuses by name when it is pushed");
            state.absent = Some(reason);
            return;
        }
        Some(_) => {}
    }
    let Some(icon_index) = icon_index else {
        return;
    };
    if !icons.is_ready() {
        return;
    }
    let Some(masters) = glyphs.masters.as_ref() else {
        return;
    };
    let keys = [SCREEN, TAB_CELL, ITEM_CELL, FIXTURE_CELL, HEADER];
    if keys
        .into_iter()
        .chain(CONTENT_LIST_TYPES.map(ContentListType::list_document))
        .any(|key| !layouts.ready(key, &server))
    {
        return;
    }
    let missing: Vec<&str> = WORDINGS
        .iter()
        .copied()
        .filter(|key| !layouts.wordings.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        warn!(
            "[inventory] wordings missing from this root: {missing:?}; the texts that write them keep the prefab's text"
        );
    }
    let kind = CONTENT_LIST_TYPES[0];
    let listed = rows(kind, owned.as_deref(), masters);
    register_icons(&mut layouts, &server, &icon_index, masters, kind, &listed);
    let (document, bindings) = compose(&layouts, kind, listed)
        .unwrap_or_else(|error| panic!("inventory screen composition: {error}"));
    layouts
        .replace_runtime_document(RUNTIME, document, &server)
        .expect("inventory screen installation");
    let mut view = UiPrefabView::new(RUNTIME, SITEMAP_LAYER);
    let mut refused = Vec::new();
    apply_static(
        &mut view,
        layouts.document(RUNTIME).unwrap(),
        &layouts,
        &bindings,
        masters,
        &icons,
        &mut refused,
    );
    name_refusals(&mut state, &icon_index, refused);
    commands.spawn((
        InventoryRoot { bindings },
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    let mut header = UiPrefabView::new(HEADER, SITEMAP_LAYER);
    let back = bind_header(layouts.document(HEADER).unwrap(), &mut header)
        .unwrap_or_else(|error| panic!("inventory header: {error}"));
    commands.spawn((
        InventoryHeader { back },
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 20.),
        RenderLayers::layer(SITEMAP_LAYER),
        header,
    ));
    commands.insert_resource(InventorySpawned);
}

/// The chest limit of a type: (wording, have, limit); `None` for the types
/// whose limit root `UpdateLimitText` turns off. A limit the masters cannot
/// give is `None` inside.
fn limit(
    kind: ContentListType,
    owned: Option<&ClientMysekaiInventory>,
    masters: Option<&PossessionMasters>,
) -> Option<(&'static str, i32, Option<i32>)> {
    let empty = ClientMysekaiInventory::default();
    let owned = owned.unwrap_or(&empty);
    match kind {
        ContentListType::Fixture => Some((
            "WORD_CHEST_FURNITURE_NUM",
            owned.fixture_total_count(),
            masters.and_then(|m| owned.fixture_max_count(m)),
        )),
        ContentListType::Material => Some((
            "WORD_CHEST_MATERIAL_NUM",
            owned.total_material_possession(),
            masters.and_then(|m| owned.material_max_count(m)),
        )),
        ContentListType::Item | ContentListType::Tool => None,
    }
}

/// `GetDenomTextColor(have, limit)` written into the have text. The text's
/// serialized colour is white (`WHITE_ALPHA_1`), rich text is on, it has no
/// vertex gradient and does not override tag colours, so a colour tag gives
/// the vertices the same colour as setting `TMP_Text.color`.
fn denom_text(have: i32, limit: i32) -> String {
    if have < limit {
        have.to_string()
    } else {
        format!("<color=#FF55AA>{have}</color>")
    }
}

/// The custom ListView's placement of the cells (row-major from the top left,
/// padding then cell size plus spacing), the content height it sets and the
/// scroll clamp.
fn layout_cells(
    bindings: &Bindings,
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
    scroll: &mut f32,
) {
    let step = bindings.cell_size + bindings.spacing;
    let rows = (bindings.cells.len() as f32 / bindings.columns as f32).ceil();
    let height = step.y * rows + bindings.padding[2] + bindings.padding[3]
        - if rows > 0. { bindings.spacing.y } else { 0. };
    view.set_size_delta(&bindings.content, Vec2::new(0., height));
    if let Some(viewport) = view.rect(layouts, &bindings.viewport, canvas) {
        *scroll = scroll.clamp(0., (height - viewport.size.y).max(0.));
    }
    view.set_anchored_position(
        &bindings.content,
        bindings.source_content_position + Vec2::Y * *scroll,
    );
    for (index, cell) in bindings.cells.iter().enumerate() {
        view.set_anchored_position(
            &cell.root,
            Vec2::new(
                step.x * (index % bindings.columns) as f32 + bindings.padding[0],
                -step.y * (index / bindings.columns) as f32 - bindings.padding[2],
            ),
        );
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    mut state: ResMut<InventoryState>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    glyphs: Res<InventoryGlyphs>,
    icons: Res<crate::fixture_edit_ui::EditorIcons>,
    owned: Option<Res<ClientMysekaiInventory>>,
    possession: Option<Res<PossessionMasters>>,
    icon_index: Option<Res<ItemIcons>>,
    mut roots: Query<
        (
            &mut InventoryRoot,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<InventoryHeader>,
    >,
    mut headers: Query<
        (
            &InventoryHeader,
            &UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<InventoryRoot>,
    >,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let open = stack.current() == LayerId::MysekaiInventory;
    if open && !state.was_open {
        // OnBoot (no boot argument), OnInitComponent: the Fixture list first,
        // the first item of every list.
        state.tab = 0;
        state.selected = [0; 4];
        state.scroll = 0.;
        state.map_pending = true;
        let copy = owned.as_deref();
        info!(
            "[inventory] OnBoot: no boot argument; OnInitComponent: ContentListTypes Fixture, Material, Item, Tool; \
             UpdateLimitText(Fixture); the first item of each list. Client copy: {} fixture rows ({} in total), \
             {} material rows (possession {}), {} item rows{}{}",
            copy.map_or(0, |o| o.fixtures().len()),
            copy.map_or(0, |o| o.fixture_total_count()),
            copy.map_or(0, |o| o.materials().len()),
            copy.map_or(0, |o| o.total_material_possession()),
            copy.map_or(0, |o| o.items().len()),
            match copy {
                None => "; the server model keeps no inventory copy in this build",
                Some(o) if o.revision == 0 =>
                    "; no server response has carried an inventory table yet",
                Some(_) => "",
            },
            if possession.is_none() {
                "; the possession masters are not installed (no limit text)"
            } else {
                ""
            },
        );
    }
    if !open && state.was_open {
        info!("[inventory] OnExitStart / OnExited: the screen closes");
    }
    state.was_open = open;
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    for (_, _, mut visibility, mut transform) in &mut headers {
        *visibility = if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
    }
    let Ok((mut root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    *visibility = if open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    transform.scale = Vec3::splat(scale);
    if !open {
        return;
    }
    let (Some(masters), Some(icon_index)) = (glyphs.masters.as_ref(), icon_index.as_deref()) else {
        return;
    };
    let kind = CONTENT_LIST_TYPES[state.tab];
    let next_rows = rows(kind, owned.as_deref(), masters);
    if root.bindings.kind != kind || root.bindings.rows != next_rows {
        register_icons(&mut layouts, &server, icon_index, masters, kind, &next_rows);
        let (document, bindings) = compose(&layouts, kind, next_rows)
            .unwrap_or_else(|error| panic!("inventory list update: {error}"));
        layouts
            .replace_runtime_document(RUNTIME, document, &server)
            .expect("inventory list installation");
        view.clear_overrides();
        root.bindings = bindings;
        let mut refused = Vec::new();
        apply_static(
            &mut view,
            layouts.document(RUNTIME).unwrap(),
            &layouts,
            &root.bindings,
            masters,
            &icons,
            &mut refused,
        );
        name_refusals(&mut state, icon_index, refused);
        let count = root.bindings.rows.len();
        let tab = state.tab;
        if state.selected[tab] >= count {
            state.selected[tab] = 0;
        }
        info!("[inventory] {kind:?} list: {count} rows");
        state.map_pending = true;
    }
    let bindings = &root.bindings;
    for (index, tab) in bindings.tabs.iter().enumerate() {
        view.set_visible(&tab.selected, index == state.tab);
    }
    // UpdateLimitText(selectedContent).
    match limit(kind, owned.as_deref(), possession.as_deref()) {
        Some((wording, have, max)) => {
            view.set_visible(&bindings.limit_root, true);
            if let Some(text) = layouts.wordings.get(wording) {
                view.set_text(&bindings.limit_text, text.clone());
            }
            match max {
                Some(max) => {
                    view.set_text(&bindings.limit_denom, denom_text(have, max));
                    view.set_text(&bindings.limit_numer, max.to_string());
                }
                None => {
                    view.set_text(&bindings.limit_denom, have.to_string());
                    view.set_text(&bindings.limit_numer, String::new());
                }
            }
        }
        None => view.set_visible(&bindings.limit_root, false),
    }
    // The selected cell and the preview.
    let selected = state.selected[state.tab];
    for (index, cell) in bindings.cells.iter().enumerate() {
        view.set_visible(&cell.selected, index == selected);
    }
    let mut refused = Vec::new();
    match bindings.rows.get(selected) {
        Some(row) => {
            view.set_visible(&bindings.preview_object, true);
            view.set_visible(&bindings.recycle_button, true);
            view.set_text(
                &bindings.name_text,
                masters
                    .row(kind, *row)
                    .map_or_else(String::new, |m| m.name.clone()),
            );
            view.set_visible(&bindings.fixture_object, kind == ContentListType::Fixture);
            view.set_visible(&bindings.item_object, kind != ContentListType::Fixture);
            if kind != ContentListType::Fixture {
                let path = masters.icon_path(kind, *row, "item_preview");
                set_icon(
                    &mut view,
                    &layouts,
                    &bindings.item_preview,
                    path,
                    &mut refused,
                );
            }
        }
        // InventoryPreview.ShowEmpty.
        None => {
            view.set_visible(&bindings.preview_object, false);
            view.set_visible(&bindings.recycle_button, false);
        }
    }
    name_refusals(&mut state, icon_index, refused);
    let mut scroll = state.scroll;
    layout_cells(bindings, &mut view, &layouts, canvas, &mut scroll);
    state.scroll = scroll;
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
            .take(5)
            .map(|cell| pixel(&*view, &cell.button))
            .collect();
        info!(
            "[inventory] tap map (window px, window {}x{}): back {back:?}, tabs Fixture/Material/Item/Tool {tabs:?}, first cells {cells:?}",
            size.x, size.y
        );
    }
}

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

/// Taps while the screen is open: the header's back button pops it, a tab
/// selects its list (`OnSelectContentType`), a cell selects its row. The
/// full-screen layer takes every tap (the source's raycast blocking).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    layouts: Res<UiLayouts>,
    mut state: ResMut<InventoryState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    roots: Query<(&InventoryRoot, &UiPrefabView)>,
    headers: Query<(&InventoryHeader, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let scrolls: Vec<MouseWheel> = wheel.read().copied().collect();
    if stack.current() != LayerId::MysekaiInventory {
        state.dragging_list = false;
        state.refusal_popped = false;
        return;
    }
    if let Some(reason) = &state.absent {
        // The screen has no view on this root: refused by name and popped
        // once its push has settled; its taps go nowhere.
        consumed.0 |= !events.is_empty();
        if !state.refusal_popped && !stack.is_ui_layer_working() {
            error!(
                "[inventory] PushUIScreen(MysekaiInventory) refused: {reason}; the screen is popped"
            );
            layer_commands.write(LayerCommand::Pop);
            state.refusal_popped = true;
        }
        return;
    }
    let (Ok((window_entity, window)), Some(root_canvas)) =
        (windows.single(), root_canvas.as_deref())
    else {
        return;
    };
    let (Ok((root, view)), Ok((header, header_view))) = (roots.single(), headers.single()) else {
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
        // The full-screen layer takes it (world rays do not get it).
        consumed.0 = true;
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
        if hit(header_view, &layouts, &header.back, point, canvas) {
            sounds.source_button(&layouts, header_view.key, &header.back);
            info!("[inventory] header back button → BackUIScreen");
            layer_commands.write(LayerCommand::Pop);
            continue;
        }
        if let Some(index) = bindings
            .tabs
            .iter()
            .position(|tab| hit(view, &layouts, &tab.button, point, canvas))
        {
            if index != state.tab {
                sounds.source_button(&layouts, view.key, &bindings.tabs[index].button);
                info!(
                    "[inventory] tab {:?} → OnSelectContentType",
                    CONTENT_LIST_TYPES[index]
                );
                state.tab = index;
                state.scroll = 0.;
            }
            continue;
        }
        if let Some(index) = bindings
            .cells
            .iter()
            .position(|cell| hit(view, &layouts, &cell.button, point, canvas))
        {
            sounds.source_button(&layouts, view.key, &bindings.cells[index].button);
            let tab = state.tab;
            state.selected[tab] = index;
        }
    }
}
