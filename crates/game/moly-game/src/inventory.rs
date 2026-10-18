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
//! the screen selects the first cell of every list (`OnInitComponent`:
//! `SelectFixtureFirstItem`, `SelectUserResourceFirstItem` for Material, Item
//! and Tool).
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
//! `UpdateLimitText(Fixture)`.
//!
//! - **Fixture list** (`FixtureInventoryContentList`, view type Default):
//!   every `UserMysekaiFixture` row with quantity at least 1 and a master
//!   row, as a `FixtureSelectCell` whose thumbnail count is `×{quantity}`
//!   (`UIPartsThumbnail.SetQuantity`, format `×{0}`). The list's tabs are the
//!   inventory's `mysekaiFixtureTabViews` rows (`edit_layout_home`); the
//!   filter toggle (`_fixtureFilterToggle`, `OnFixtureFilterChanged`) swaps
//!   between Default and `InPlaceable`.
//! - **Material, item and tool lists** (`UserResourceContentList`): the
//!   owned rows with quantity above 0, as `UIPartsItemThumbnailListViewItem`
//!   cells with the same count format. The tool rows are `UserMysekaiTools`
//!   (`ToUserResources`: the resource id is `mysekaiToolId`).
//!   `UIPartsItemThumbnail.Setup` loads the cell image from
//!   `AssetBundleNames.GetMysekaiMaterialThumbnail` /
//!   `GetMysekaiItemThumbnail` / `GetMysekaiToolThumbnail`
//!   (`mysekai/thumbnail/material/`, `mysekai/thumbnail/item/` or
//!   `mysekai/thumbnail/tool/` followed by the master's
//!   `iconAssetbundleName`, or the tool master's `assetbundleName`). The
//!   material list's tabs are one per material type.
//! - **Order** ([`order`]): the selected genre or type tab filters the rows;
//!   the sort dropdown orders them by its type (Get, Sequence, Count,
//!   AlphabeticalOrder and, for fixtures, LayoutCost) through
//!   `SortCellDataList`; the order button (`UIPartsSortOrder`) reverses that
//!   order (`SortOrderBy.Desc` is the ascending list reversed).
//! - **Search**: the search button (`OnSearch`) opens the search dialog
//!   (`MysekaiFixtureSearchDialog`, 333). Its word field is not built, so
//!   only the stored filter's word exists, and no carried master names
//!   searchable fixture text: a word filters nothing and is named once.
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
//!   followed by `iconAssetbundleName`); for a fixture the fixture object;
//!   for a tool (`SetTool`) neither: the item object and the fixture object
//!   off. `ShowEmpty` when the list has no rows: the preview object, the
//!   recycle button and the cannot-recycle info off.
//! - **Description** (`_descriptionView`, `DescriptionView`): a fixture's
//!   flavour text and grid size, a material's or an item's flavour text, a
//!   tool's description ([`recycle`]).
//! - **Recycle** (`InventoryPreview.OpenDisassembleDialog`): the source
//!   shows `MysekaiInventoryScrapFixtureDialog` (dialog type 328) with the
//!   item, its name and the selected count; that prefab is not served by the
//!   UI roots here, so the shared two-button dialog stands in with the name
//!   and the count as its message. OK sends
//!   `PostUserMysekaiDisassembleFixtureApi`,
//!   `PostUserMysekaiRecycleMaterialApi` or `PostUserMysekaiRecycleToolApi`
//!   ([`recycle`]).
//!
//! The owned rows and the possession levels are the client's copy of the
//! server data (`crate::userdata::client::inventory`, and
//! `crate::userdata::client::tools` for the tools); the possession limits
//! are master data the server model installs. The thumbnail and preview
//! images are resolved by their client load path and resource through the
//! shared item icon index ([`crate::item_icon`]); a path the index does not
//! carry, or a root without the index, is refused by name and the image stays
//! off.
//!
//! The screen's own prefab, its four list prefabs, its tab cell and its item
//! cell are documents a root may lack. A root without one of them names the
//! absent documents once; the screen then refuses by name in the frame it is
//! pushed and asks the screen manager to pop it. A root without the shared
//! two-button dialog keeps the recycle button off and names it once.
//!
//! ## Named gaps
//!
//! - The 3D preview (`MysekaiBlueprint3DPreview`) is hidden: the crate draws
//!   a fixture's model in the field but has no offscreen preview camera, so
//!   the item preview image and the name stand in for it.
//! - The recycle dialog's own layout (`MysekaiInventoryScrapFixtureDialog`)
//!   and the count selector inside it (`RecycleView`,
//!   `UIPartsSelectNumberItemCell`) are not drawn: the confirming dialog is
//!   the shared two-button one and recycles one unit
//!   (`MysekaiInventoryUtility.SelectedNum` is 1).
//! - The search field (`CustomInputFieldTextMesh`) is not built.
//! - The tags (hashtag balloon and `HashTagListView` pages), the "not
//!   created" filter, the system fixture icon and the plant preview are not
//!   built.
//! - The canvas rows (`UserMysekaiCanvas`) are not listed.
//! - The item list has no recycle request of its own: `SendDisassembleApi`
//!   builds one for a fixture, a material or a tool only, so the item list
//!   shows no recycle button.
//! - The tab cell's icon is hidden (the selector's cell settings carry no
//!   sprite for these four types) and its label keeps the serialized colour.
//! - The header shows its back button only.
//!
//! ## Files
//!
//! - [`catalog`]: the master tables besides the rows' masters, and the stored
//!   fixture filters.
//! - [`order`]: the rows' sort keys, the tabs and their filters.
//! - [`view`]: the prefab composition and its bindings.
//! - [`recycle`]: the recycle confirmation, its request and the description
//!   page.
//!
//! `view::compose` binds a control by the field name its serialized list
//! carries (`listView`, `_sortDropDown`, `_tabListController`); a list prefab
//! that names none of them keeps that control hidden and names it once.

mod catalog;
mod order;
mod recycle;
mod view;

use std::collections::{BTreeSet, HashMap};

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::master::MasterData;
use moly_assets::json::JsonAsset;
use moly_link::ServerLink;
use moly_protocol::Call;
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::fixture_edit_ui::EditorIcons;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::item_icon::ItemIcons;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{DialogBackKeyEvent, LayerCommand, LayerId, ScreenManager, UiLayerStack};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::client::inventory::{ClientMysekaiInventory, PossessionMasters};
use crate::userdata::client::tools::ClientMysekaiTools;

use catalog::Catalog;
use order::{FixtureFilter, SortOrder, SortType, Tab};
use recycle::Recycle;
use view::{Bindings, HEADER, RUNTIME};

/// `ScreenLayerMysekaiInventory.ContentListTypes`.
pub(crate) const CONTENT_LIST_TYPES: [ContentListType; 4] = [
    ContentListType::Fixture,
    ContentListType::Material,
    ContentListType::Item,
    ContentListType::Tool,
];

/// The characters the screen writes besides the master names and wordings.
pub(crate) const FIXED_TEXTS: &[&str] = &["×0123456789"];

/// The wordings the screen writes: the type tabs, the chest limits and the
/// sort types. The sort dropdown's label and the dialog's buttons are the
/// prefabs' own.
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_FIXTURE",
    "WORD_ELEMENT",
    "WORD_ITEM",
    "WORD_TOOL",
    "WORD_CHEST_FURNITURE_NUM",
    "WORD_CHEST_MATERIAL_NUM",
    "WORD_DEFAULT",
    "WORD_CATEGORY_ORDER",
    "WORD_GET_ORDER",
    "WORD_POSSESSION_COUNT_ORDER",
    "WORD_ALPHABETICAL_ORDER",
    "WORD_MYSEKAI_LAYOUT_COST_ORDER",
];

/// `Sekai.Mysekai.ContentList.ContentListType` (the four the screen lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ContentListType {
    Fixture = 6,
    Material = 7,
    Item = 8,
    Tool = 9,
}

impl ContentListType {
    /// The list's place in [`CONTENT_LIST_TYPES`].
    pub(crate) fn index(self) -> usize {
        self as usize - CONTENT_LIST_TYPES[0] as usize
    }

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
            Self::Tool => Some("tool"),
            Self::Fixture => None,
        }
    }

    /// The icon families a row of the type loads: the cell's thumbnail, and
    /// the preview's item image for the types whose preview shows the item
    /// object (`SetMaterial` / `SetItem`; `SetTool` turns it off).
    fn icon_families(self) -> &'static [&'static str] {
        match self {
            Self::Material | Self::Item => &["thumbnail", "item_preview"],
            Self::Tool => &["thumbnail"],
            Self::Fixture => &[],
        }
    }

    /// `ContentListTypeToMysekaiFixtureTabViewType`'s view type the list's
    /// tab rows carry: `edit_layout_home` for the fixture list only.
    fn carries_fixture_tabs(self) -> bool {
        self == Self::Fixture
    }
}

/// A master row's columns the screen reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MasterRow {
    pub(crate) name: String,
    /// `iconAssetbundleName` (a tool's `assetbundleName`).
    pub(crate) icon: Option<String>,
    /// `seq`, the Sequence sort's key.
    pub(crate) seq: Option<i32>,
    /// `pronunciation`, the AlphabeticalOrder sort's key.
    pub(crate) pronunciation: String,
    /// The fixture's layout cost (`mysekaiFixtureIPC.putCost`).
    pub(crate) layout_cost: i32,
    /// The fixture's main and sub genre ids.
    pub(crate) main_genre: i32,
    pub(crate) sub_genre: i32,
    /// The fixture's grid (`width`, `depth`), the grid size text's values.
    pub(crate) grid: Option<(i32, i32)>,
    /// The fixture's `flavorText` or a tool's `description` (the materials
    /// master carries no description).
    pub(crate) description: String,
}

/// One listed row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: i32,
    /// The fixture's colour (`textureId`; 0 for a resource row).
    pub(crate) texture: i32,
    pub(crate) quantity: i32,
    /// The owned row's `lastObtainedAt`, the Get sort's key (the material
    /// copy carries none).
    pub(crate) last_obtained_at: Option<i64>,
    pub(crate) master: Option<MasterRow>,
}

impl Row {
    pub(crate) fn quantity(&self) -> i32 {
        self.quantity
    }

    /// Whether both are the same listed row with the same drawn columns.
    fn same_identity(&self, other: &Row) -> bool {
        self.id == other.id && self.texture == other.texture && self.master == other.master
    }
}

/// The master rows the screen reads (fixtures, materials, items, tools).
#[derive(Default)]
pub(crate) struct Masters {
    pub(crate) fixtures: HashMap<i32, MasterRow>,
    pub(crate) materials: HashMap<i32, MasterRow>,
    pub(crate) items: HashMap<i32, MasterRow>,
    pub(crate) tools: HashMap<i32, MasterRow>,
}

impl Masters {
    pub(crate) fn table(&self, kind: ContentListType) -> &HashMap<i32, MasterRow> {
        match kind {
            ContentListType::Fixture => &self.fixtures,
            ContentListType::Material => &self.materials,
            ContentListType::Item => &self.items,
            ContentListType::Tool => &self.tools,
        }
    }

    /// The master row of a listed row: the master table's columns, or the
    /// row's own copy where the table has none.
    pub(crate) fn row<'a>(&'a self, kind: ContentListType, row: &'a Row) -> Option<&'a MasterRow> {
        self.table(kind).get(&row.id).or(row.master.as_ref())
    }

    /// The client load of a row's cell image (`GetMysekaiMaterialThumbnail`
    /// / `GetMysekaiItemThumbnail` / `GetMysekaiToolThumbnail`) or preview
    /// image (`GetMysekaiMaterialPreview` / `GetMysekaiItemPreview`): the load
    /// path (the folder then the master's icon name) and the resource, the
    /// icon name itself.
    pub(crate) fn icon_path(
        &self,
        kind: ContentListType,
        row: &Row,
        family: &str,
    ) -> Option<view::IconLoad> {
        let folder = kind.icon_folder()?;
        let icon = self.row(kind, row)?.icon.clone()?;
        Some(view::IconLoad {
            path: format!("mysekai/{family}/{folder}/{icon}"),
            resource: icon,
        })
    }
}

const FIXTURE_MASTER: &str = "moly://mysekai-fixtures.json";
const MATERIAL_MASTER: &str = "moly://mysekai-materials.json";
const ITEM_MASTER: &str = "moly://mysekai-items.json";
const TOOL_MASTER: &str = "moly://mysekai-tools.json";
/// `mysekaiFixtureFilteredDatas`: the stored fixture filters.
const FILTER_MASTER: &str = "moly://mysekai-fixture-filtered-datas.json";
const MASTER_PATHS: [&str; 5] = [
    FIXTURE_MASTER,
    MATERIAL_MASTER,
    ITEM_MASTER,
    TOOL_MASTER,
    FILTER_MASTER,
];

/// The row masters, requested at start-up; their names reach the shared text
/// atlas through `texts`.
#[derive(Resource)]
pub(crate) struct InventoryGlyphs {
    handles: Option<[Handle<JsonAsset>; 5]>,
    masters: Option<Masters>,
    pub(crate) texts: Option<Vec<String>>,
}

pub(crate) fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut data: ResMut<MasterData>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    commands.init_resource::<InventoryState>();
    commands.init_resource::<Recycle>();
    commands.init_resource::<catalog::Filters>();
    // A stage draws no screens: nothing to request, nothing to read.
    let live = stage.is_none();
    if live {
        catalog::request(&mut data);
    } else {
        commands.insert_resource(Catalog::answered());
        commands.insert_resource(catalog::Filters::default());
    }
    let handles = live.then(|| MASTER_PATHS.map(|path| server.load::<JsonAsset>(path)));
    let texts = (!live).then(Vec::new);
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

fn int_of(row: &Value, field: &str) -> i32 {
    row[field]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn seq_of(row: &Value) -> Option<i32> {
    row["seq"].as_i64().and_then(|seq| i32::try_from(seq).ok())
}

fn text_of(row: &Value, field: &str) -> String {
    row[field].as_str().unwrap_or_default().to_owned()
}

/// The fixture master's grid (`gridWidth` × `gridDepth`), when the row
/// states one.
fn grid_of(row: &Value) -> Option<(i32, i32)> {
    let width = row["gridWidth"].as_i64().and_then(|value| i32::try_from(value).ok())?;
    let depth = row["gridDepth"].as_i64().and_then(|value| i32::try_from(value).ok())?;
    Some((width, depth))
}

/// `mysekai-fixtures.json`: the `fixtures` array, each row with its name, its
/// sequence, its pronunciation, its genres, its grid and its flavour text.
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
                    seq: seq_of(row),
                    pronunciation: text_of(row, "pronunciation"),
                    layout_cost: 0,
                    main_genre: int_of(row, "mysekaiFixtureMainGenreId"),
                    sub_genre: int_of(row, "mysekaiFixtureSubGenreId"),
                    grid: grid_of(row),
                    description: text_of(row, "flavorText"),
                },
            ))
        })
        .collect()
}

/// A keyed master table (`entries` by id), each row with its name, its icon
/// name (the column `icon`), its sequence, its pronunciation and its
/// description.
fn keyed_rows(value: &Value, table: &str, icon: &str) -> Result<HashMap<i32, MasterRow>, String> {
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
                    icon: Some(text(icon)?),
                    seq: seq_of(row),
                    pronunciation: text_of(row, "pronunciation"),
                    layout_cost: 0,
                    main_genre: 0,
                    sub_genre: 0,
                    grid: None,
                    description: text_of(row, "description"),
                },
            ))
        })
        .collect()
}

/// Reads the four row masters and the stored filters once they load; the
/// names join the shared text atlas.
pub(crate) fn parse_glyphs(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut glyphs: ResMut<InventoryGlyphs>,
    mut filters: ResMut<catalog::Filters>,
) {
    if glyphs.texts.is_some() {
        return;
    }
    let Some(handles) = glyphs.handles.clone() else {
        return;
    };
    let mut values = Vec::with_capacity(handles.len());
    for (handle, path) in handles.iter().zip(MASTER_PATHS) {
        if let LoadState::Failed(error) = server.load_state(handle) {
            if path == FILTER_MASTER {
                warn!("[inventory] {path} failed to load ({error:?}): no fixture filter is stored");
                values.push(Value::Null);
                continue;
            }
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
        materials: keyed_rows(&values[1], "mysekaiMaterials", "iconAssetbundleName")
            .unwrap_or_else(|e| panic!("[inventory] {MATERIAL_MASTER}: {e}")),
        items: keyed_rows(&values[2], "mysekaiItems", "iconAssetbundleName")
            .unwrap_or_else(|e| panic!("[inventory] {ITEM_MASTER}: {e}")),
        tools: keyed_rows(&values[3], "mysekaiTools", "assetbundleName")
            .unwrap_or_else(|e| panic!("[inventory] {TOOL_MASTER}: {e}")),
    };
    if !values[4].is_null() {
        filters.rows = catalog::filters_from(&values[4]);
    }
    let mut texts: Vec<String> = masters
        .fixtures
        .values()
        .chain(masters.materials.values())
        .chain(masters.items.values())
        .chain(masters.tools.values())
        .map(|row| row.name.clone())
        .collect();
    texts.extend(FIXED_TEXTS.iter().map(|text| (*text).to_owned()));
    info!(
        "[inventory] masters: {} fixtures, {} materials, {} items, {} tools",
        masters.fixtures.len(),
        masters.materials.len(),
        masters.items.len(),
        masters.tools.len()
    );
    glyphs.masters = Some(masters);
    glyphs.texts = Some(texts);
}

/// Takes the catalogue tables as they resolve, names the absent ones and
/// builds each list's tabs once the catalogue has settled.
pub(crate) fn poll_catalog(
    mut catalog: ResMut<Catalog>,
    mut data: ResMut<MasterData>,
    mut state: ResMut<InventoryState>,
) {
    if catalog.settled() {
        return;
    }
    catalog.poll(&mut data);
    if !catalog.settled() {
        return;
    }
    catalog.name_absent();
    state.tabs = [
        catalog.fixture_tabs(),
        catalog.material_tabs(),
        Vec::new(),
        Vec::new(),
    ];
    info!(
        "[inventory] tabs: {} fixture genre tabs, {} material type tabs",
        state.tabs[0].len(),
        state.tabs[1].len()
    );
}

/// The dialog's answer: the confirmation's OK sends its request
/// (`OnClickOKMysekaiInventoryScrapFixtureDialog`), its cancel drops it, and
/// the recycle reply's answer shows the resources it returned
/// (`OnDisassemble`; the copies already carry its tables).
pub(crate) fn read_answer(
    mut commands: Commands,
    mut answers: MessageReader<crate::userdata::Answered>,
    mut state: ResMut<InventoryState>,
    spawned: Option<Res<InventorySpawned>>,
) {
    let answers: Vec<crate::userdata::Answered> = answers.read().cloned().collect();
    if spawned.is_none() {
        return;
    }
    // The link is read inside the command, so no resource borrow escapes it.
    // The confirmation's answer is read first: the common dialog's own `click`
    // reads its buttons, and only an OK sends the request.
    commands.queue(move |world: &mut World| {
        let (answer, request) = world.resource_scope(|world, mut recycle: Mut<Recycle>| {
            (recycle.answer(world), recycle.take_request())
        });
        if !matches!(answer, Some(true)) {
            return;
        }
        let Some(request) = request else {
            return;
        };
        let seq = world
            .resource_mut::<ServerLink>()
            .call(Call::InventoryRecycle(request));
        info!(
            "[inventory] OnClickOKMysekaiInventoryScrapFixtureDialog: {} sent as seq {seq}",
            request.api_name()
        );
    });
    let tab = state.tab;
    for answer in answers {
        if answer.call != moly_protocol::CallKind::InventoryRecycle {
            continue;
        }
        let outcome = answer.outcome.clone();
        commands.queue(move |world: &mut World| recycle::reward(world, &outcome));
        // `InventoryPreview` re-selects the list's first item.
        state.selected[tab] = 0;
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The fixture list's view type (`FixtureInventoryContentList.ViewType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FixtureView {
    /// `ViewType.Default`: every owned fixture.
    #[default]
    Default,
    /// `ViewType.InPlaceable`.
    InPlaceable,
}

/// One list's selected tab, sort and order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct ListView {
    pub(crate) tab: usize,
    pub(crate) sort: SortType,
    pub(crate) order: SortOrder,
}

#[derive(Resource, Default)]
pub(crate) struct InventoryState {
    /// Index into [`CONTENT_LIST_TYPES`].
    tab: usize,
    /// The selected cell of each list (each list keeps its own selection).
    selected: [usize; 4],
    /// Each list's tab, sort and order.
    view: [ListView; 4],
    /// The fixture list's view type (`_viewType`, Default).
    fixture_view: FixtureView,
    /// The tabs each list offers, once the catalogue has settled.
    tabs: [Vec<Tab>; 4],
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

impl InventoryState {
    fn list(&self, kind: ContentListType) -> &ListView {
        &self.view[kind.index()]
    }

    fn list_mut(&mut self, kind: ContentListType) -> &mut ListView {
        &mut self.view[kind.index()]
    }

    fn tabs_of(&self, kind: ContentListType) -> &[Tab] {
        &self.tabs[kind.index()]
    }
}

#[derive(Component)]
pub(crate) struct InventoryRoot {
    bindings: Bindings,
    kind: ContentListType,
    /// The rows drawn, for the change check.
    drawn: Vec<Row>,
}

/// The header view with its back button.
#[derive(Component)]
pub(crate) struct InventoryHeader {
    back: String,
}

#[derive(Resource)]
pub(crate) struct InventorySpawned;

// ---------------------------------------------------------------------------
// The rows
// ---------------------------------------------------------------------------

/// The owned rows of a type before the tab, the filter and the order.
pub(crate) fn rows(
    kind: ContentListType,
    owned: Option<&ClientMysekaiInventory>,
    tools: Option<&ClientMysekaiTools>,
    masters: &Masters,
    catalog: &Catalog,
) -> Vec<Row> {
    let empty = ClientMysekaiInventory::default();
    let owned = owned.unwrap_or(&empty);
    let no_tools = ClientMysekaiTools::default();
    let tools = tools.unwrap_or(&no_tools);
    let mut out = Vec::new();
    match kind {
        ContentListType::Fixture => {
            for row in owned.fixtures() {
                let Some(master) = masters.fixtures.get(&row.mysekai_fixture_id) else {
                    continue;
                };
                if row.quantity < 1 {
                    continue;
                }
                out.push(Row {
                    id: row.mysekai_fixture_id,
                    texture: row.texture_id,
                    quantity: row.quantity,
                    last_obtained_at: Some(row.last_obtained_at),
                    master: Some(master.clone()),
                });
            }
        }
        ContentListType::Material => {
            for row in owned.materials() {
                if row.quantity <= 0 {
                    continue;
                }
                out.push(Row {
                    id: row.mysekai_material_id,
                    texture: 0,
                    quantity: row.quantity,
                    last_obtained_at: None,
                    master: masters.materials.get(&row.mysekai_material_id).cloned(),
                });
            }
        }
        ContentListType::Item => {
            for row in owned.items() {
                if row.quantity <= 0 {
                    continue;
                }
                out.push(Row {
                    id: row.mysekai_item_id,
                    texture: 0,
                    quantity: row.quantity,
                    last_obtained_at: Some(row.last_obtained_at),
                    master: masters.items.get(&row.mysekai_item_id).cloned(),
                });
            }
        }
        ContentListType::Tool => {
            for row in tools.rows() {
                if row.quantity <= 0 {
                    continue;
                }
                out.push(Row {
                    id: row.mysekai_tool_id,
                    texture: 0,
                    quantity: row.quantity,
                    last_obtained_at: Some(row.last_obtained_at),
                    master: masters.tools.get(&row.mysekai_tool_id).cloned(),
                });
            }
        }
    }
    catalog.decorate(kind, &mut out);
    out
}

/// The rows the list draws: the tab's filter, the fixture view type and the
/// order.
pub(crate) fn drawn_rows(
    kind: ContentListType,
    base: &[Row],
    tabs: &[Tab],
    view: &ListView,
    fixture_view: FixtureView,
    catalog: &Catalog,
    filters: &[FixtureFilter; 3],
) -> Vec<Row> {
    let kept: Vec<Row> = match kind {
        ContentListType::Fixture => {
            let filter = &filters[order::SEARCH_INVENTORY];
            base.iter()
                .filter(|row| order::passes_genre(tabs, view.tab, row))
                // `InPlaceable` keeps the rows whose placeable count
                // (`GetFixturePlaceableCount`) is above 0; the sites' layouts
                // are not read here, and every owned copy is placeable.
                .filter(|row| fixture_view == FixtureView::Default || row.quantity > 0)
                // A search word keeps no row (`IsFiltered`): no carried
                // master names searchable fixture text.
                .filter(|_| filter.word.is_empty())
                .filter(|_| !filter.is_not_created_filter)
                .filter(|_| !filter.is_filtered_tag())
                .cloned()
                .collect()
        }
        ContentListType::Material => {
            let selected = tabs.get(view.tab);
            base.iter()
                .filter(|row| {
                    order::passes_material_type(
                        view.tab,
                        selected.and_then(|tab| tab.material_type.as_deref()),
                        catalog
                            .material_types()
                            .and_then(|types| types.get(&row.id))
                            .map(String::as_str),
                    )
                })
                .cloned()
                .collect()
        }
        ContentListType::Item | ContentListType::Tool => base.to_vec(),
    };
    order::sorted(kind, &kept, view.sort, view.order)
}

// ---------------------------------------------------------------------------
// The chest limit
// ---------------------------------------------------------------------------

/// The chest limit of a type: (wording, have, limit); `None` for the types
/// whose limit root `UpdateLimitText` turns off. A limit the masters cannot
/// give is `None` inside.
pub(crate) fn limit(
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

// ---------------------------------------------------------------------------
// Spawn and frame
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    glyphs: Res<InventoryGlyphs>,
    catalog: Res<Catalog>,
    icons: Res<EditorIcons>,
    owned: Option<Res<ClientMysekaiInventory>>,
    tools: Option<Res<ClientMysekaiTools>>,
    filters: Res<catalog::Filters>,
    mut state: ResMut<InventoryState>,
    icon_index: Option<Res<ItemIcons>>,
    spawned: Option<Res<InventorySpawned>>,
) {
    if spawned.is_some() || state.absent.is_some() {
        return;
    }
    // The tab cells, the sort dropdown's label and the search dialog's
    // documents are not part of the screen; only the composition's own
    // documents are required, and each is named when the root lacks it.
    let absent: Vec<&'static str> = [view::SCREEN, view::TAB_CELL, view::ITEM_CELL]
        .into_iter()
        .chain(CONTENT_LIST_TYPES.map(ContentListType::list_document))
        .filter(|key| layouts.document(key).is_none())
        .collect();
    if layouts.document(HEADER).is_none() {
        // The root's documents are still coming in.
        return;
    }
    if !absent.is_empty() {
        let reason = format!("this root lacks the inventory documents {absent:?}");
        error!("[inventory] {reason}; the screen refuses by name when it is pushed");
        state.absent = Some(reason);
        return;
    }
    let Some(icon_index) = icon_index else {
        return;
    };
    if !icons.is_ready() || !catalog.settled() {
        return;
    }
    let Some(masters) = glyphs.masters.as_ref() else {
        return;
    };
    let keys = [
        view::SCREEN,
        view::TAB_CELL,
        view::ITEM_CELL,
        view::FIXTURE_CELL,
        HEADER,
    ];
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
    let list = state.list(kind).clone();
    let base = rows(kind, owned.as_deref(), tools.as_deref(), masters, &catalog);
    let drawn = drawn_rows(
        kind,
        &base,
        state.tabs_of(kind),
        &list,
        state.fixture_view,
        &catalog,
        &filters.rows,
    );
    view::register_icons(&mut layouts, &server, &icon_index, masters, kind, &drawn);
    let (document, bindings) = view::compose(&layouts, kind, drawn.clone())
        .unwrap_or_else(|error| panic!("inventory screen composition: {error}"));
    layouts
        .replace_runtime_document(RUNTIME, document, &server)
        .expect("inventory screen installation");
    if layouts.document(view::CONFIRM).is_none() {
        warn!(
            "[inventory] this root lacks {}; the recycle button stays off",
            view::CONFIRM
        );
    }
    let mut view = UiPrefabView::new(RUNTIME, SITEMAP_LAYER);
    let mut refused = Vec::new();
    view::apply_static(
        &mut view,
        layouts.document(RUNTIME).unwrap(),
        &layouts,
        &bindings,
        masters,
        &icons,
        &mut refused,
    );
    view::name_refusals(&mut state, &icon_index, refused);
    commands.spawn((
        InventoryRoot {
            bindings,
            kind,
            drawn,
        },
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    let mut header = UiPrefabView::new(HEADER, SITEMAP_LAYER);
    let back = view::bind_header(layouts.document(HEADER).unwrap(), &mut header)
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

/// The scroll list's placement of the cells (row-major from the top left,
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
    catalog: Res<Catalog>,
    icons: Res<EditorIcons>,
    owned: Option<Res<ClientMysekaiInventory>>,
    tools: Option<Res<ClientMysekaiTools>>,
    possession: Option<Res<PossessionMasters>>,
    filters: Res<catalog::Filters>,
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
             {} material rows (possession {}), {} item rows, {} tool rows{}{}",
            copy.map_or(0, |o| o.fixtures().len()),
            copy.map_or(0, |o| o.fixture_total_count()),
            copy.map_or(0, |o| o.materials().len()),
            copy.map_or(0, |o| o.total_material_possession()),
            copy.map_or(0, |o| o.items().len()),
            tools.as_deref().map_or(0, |t| t.rows().len()),
            match copy {
                None => "; the server model keeps no inventory copy in this build",
                Some(o) if o.revision == 0 => "; no server response has carried an inventory table yet",
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
        visibility.set_if_neq(if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        crate::canvas::set_scale(&mut transform, scale);
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
    let (Some(masters), Some(icon_index)) = (glyphs.masters.as_ref(), icon_index.as_deref()) else {
        return;
    };
    let kind = CONTENT_LIST_TYPES[state.tab];
    let list = state.list(kind).clone();
    let base = rows(kind, owned.as_deref(), tools.as_deref(), masters, &catalog);
    let next_rows = drawn_rows(
        kind,
        &base,
        state.tabs_of(kind),
        &list,
        state.fixture_view,
        &catalog,
        &filters.rows,
    );
    let same = root.kind == kind
        && root.drawn.len() == next_rows.len()
        && root.drawn.iter().zip(&next_rows).all(|(a, b)| {
            a.same_identity(b) && a.quantity == b.quantity && a.last_obtained_at == b.last_obtained_at
        });
    if !same {
        view::register_icons(&mut layouts, &server, icon_index, masters, kind, &next_rows);
        let (document, bindings) = view::compose(&layouts, kind, next_rows.clone())
            .unwrap_or_else(|error| panic!("inventory list update: {error}"));
        layouts
            .replace_runtime_document(RUNTIME, document, &server)
            .expect("inventory list installation");
        view.clear_overrides();
        root.bindings = bindings;
        root.kind = kind;
        root.drawn = next_rows;
        let mut refused = Vec::new();
        view::apply_static(
            &mut view,
            layouts.document(RUNTIME).unwrap(),
            &layouts,
            &root.bindings,
            masters,
            &icons,
            &mut refused,
        );
        view::name_refusals(&mut state, icon_index, refused);
        let count = root.bindings.rows.len();
        if state.selected[kind.index()] >= count {
            state.selected[kind.index()] = 0;
        }
        info!(
            "[inventory] {kind:?} list: {count} rows (tab {}, sort {:?}, {:?})",
            list.tab, list.sort, list.order
        );
        state.map_pending = true;
    }
    let bindings = &root.bindings;
    for (index, tab) in bindings.tabs.iter().enumerate() {
        view.set_visible(&tab.selected, index == state.tab);
    }
    let tabs = state.tabs_of(kind);
    let mut sub_of: Option<&Vec<Tab>> = None;
    for (index, tab) in bindings.strip.tabs.iter().enumerate() {
        view.set_visible(&tab.selected, index == list.tab);
        if let Some(entry) = tabs.get(index) {
            if !entry.name.is_empty() {
                view.set_text(&tab.label, entry.name.clone());
            }
            if index == list.tab {
                sub_of = Some(&entry.subs);
            }
        }
    }
    // The sub tab list (the selected parent tab's sub genres).
    let subs: &[Tab] = sub_of.map_or(&[][..], |subs| subs.as_slice());
    for (index, tab) in bindings.strip.sub_tabs.iter().enumerate() {
        view.set_visible(&tab.selected, index < subs.len());
        if let Some(sub) = subs.get(index) {
            view.set_text(&tab.label, sub.name.clone());
        }
    }
    if let Some(path) = bindings.strip.sub_root.as_ref() {
        view.set_visible(path, !subs.is_empty());
    }
    view.set_visible(&bindings.strip.order[0], list.order == SortOrder::Asc);
    view.set_visible(&bindings.strip.order[1], list.order == SortOrder::Desc);
    if let (Some(word), Some(path)) = (
        layouts.wordings.get(list.sort.wording()),
        bindings.strip.sort_label.as_ref(),
    ) {
        view.set_text(path, word.clone());
    }
    if let Some(toggle) = bindings.strip.filter_toggle.as_ref() {
        if let Some(check) = bindings.strip.filter_check.as_ref() {
            view.set_visible(check, state.fixture_view == FixtureView::InPlaceable);
        }
        view.set_visible(toggle, kind == ContentListType::Fixture);
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
    // The selected cell, the preview and the description.
    let selected = state.selected[kind.index()];
    for (index, cell) in bindings.cells.iter().enumerate() {
        view.set_visible(&cell.selected, index == selected);
    }
    let mut refused = Vec::new();
    match bindings.rows.get(selected) {
        Some(row) => {
            view.set_visible(&bindings.preview_object, true);
            view.set_text(
                &bindings.name_text,
                masters
                    .row(kind, row)
                    .map_or_else(String::new, |m| m.name.clone()),
            );
            // SetFixture: the fixture object; SetMaterial / SetItem: the item
            // object; SetTool: neither.
            let item = matches!(kind, ContentListType::Material | ContentListType::Item);
            view.set_visible(&bindings.fixture_object, kind == ContentListType::Fixture);
            view.set_visible(&bindings.item_object, item);
            if item {
                let path = masters.icon_path(kind, row, "item_preview");
                view::set_icon(
                    &mut view,
                    &layouts,
                    &bindings.item_preview,
                    path,
                    &mut refused,
                );
            }
            recycle::show_description(&mut view, bindings, kind, row, masters);
            recycle::set_button(&mut view, bindings, row);
        }
        // InventoryPreview.ShowEmpty.
        None => {
            view.set_visible(&bindings.preview_object, false);
            view.set_visible(&bindings.recycle_button, false);
            view.set_visible(&bindings.description.root, false);
            view.set_visible(&bindings.description.sites, false);
        }
    }
    view::name_refusals(&mut state, icon_index, refused);
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
        let type_tabs: Vec<_> = bindings
            .tabs
            .iter()
            .map(|tab| pixel(&*view, &tab.button))
            .collect();
        let list_tabs: Vec<_> = bindings
            .strip
            .tabs
            .iter()
            .map(|tab| pixel(&*view, &tab.button))
            .collect();
        let controls: Vec<_> = [
            bindings.strip.sort_button.clone(),
            bindings.strip.search.clone(),
            bindings.recycle_button.clone(),
        ]
        .iter()
        .map(|path| pixel(&*view, path))
        .collect();
        let cells: Vec<_> = bindings
            .cells
            .iter()
            .take(5)
            .map(|cell| pixel(&*view, &cell.button))
            .collect();
        info!(
            "[inventory] tap map (window px, window {}x{}): back {back:?}, types Fixture/Material/Item/Tool {type_tabs:?}, \
             list tabs {list_tabs:?}, sort/search/recycle {controls:?}, first cells {cells:?}",
            size.x, size.y
        );
    }
}

// ---------------------------------------------------------------------------
// Taps
// ---------------------------------------------------------------------------

fn canvas_position(position: Vec2, window_size: Vec2, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window_size.x * 0.5,
        window_size.y * 0.5 - position.y,
    ) / scale
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    !path.is_empty()
        && view
            .rect(layouts, path, canvas)
            .is_some_and(|rect| rect.active && rect.contains(point))
}

/// Taps while the screen is open. The header's back button pops it; a type
/// tab selects its list (`OnSelectContentType`); a genre tab selects the tab
/// (`OnSelectTab`); a cell selects its row; the sort dropdown chooses the
/// next sort type (`OnSortDropDown`); the order button reverses the order
/// (`OnSortOrder`); the fixture filter toggle switches the view
/// (`OnFixtureFilterChanged`); the search button opens the search dialog
/// (`OnSearch`); the recycle button opens the recycle confirmation
/// (`OpenDisassembleDialog`). While the confirmation is open every tap
/// belongs to it (the common dialog's own `click`). The full-screen layer
/// takes every tap (the source's raycast blocking).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut commands: Commands,
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut recycle: ResMut<Recycle>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    mut state: ResMut<InventoryState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    mut screens: ResMut<ScreenManager>,
    roots: Query<(&InventoryRoot, &UiPrefabView)>,
    headers: Query<(&InventoryHeader, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let scrolls: Vec<MouseWheel> = wheel.read().copied().collect();
    // The confirmation answers its own back key (`CommonDialog.click`).
    back_keys.clear();
    if screens.current() != LayerId::MysekaiInventory {
        state.dragging_list = false;
        state.refusal_popped = false;
        // The confirmation the screen showed closes with it
        // (`SetEnableRecycleButton`).
        recycle.dismiss();
        return;
    }
    if let Some(reason) = &state.absent {
        // The screen has no view on this root: refused by name and popped
        // once its push has settled; its taps go nowhere.
        consumed.0 |= !events.is_empty();
        if !state.refusal_popped && !screens.is_ui_layer_working() {
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
    // The confirmation is modal while it is shown: it answers its own taps
    // and the back key (`CommonDialog.click`).
    if recycle.confirming() {
        consumed.0 |= !events.is_empty();
        return;
    }
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
    let kind = CONTENT_LIST_TYPES[state.tab];
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
                    "[inventory] type tab {:?} → OnSelectContentType",
                    CONTENT_LIST_TYPES[index]
                );
                state.tab = index;
                state.scroll = 0.;
            }
            continue;
        }
        if let Some(index) = bindings
            .strip
            .tabs
            .iter()
            .position(|tab| hit(view, &layouts, &tab.button, point, canvas))
        {
            if index != state.list(kind).tab {
                sounds.source_button(&layouts, view.key, &bindings.strip.tabs[index].button);
                info!("[inventory] {kind:?} tab {index} → OnSelectTab");
                state.list_mut(kind).tab = index;
                state.selected[kind.index()] = 0;
                state.scroll = 0.;
            }
            continue;
        }
        if hit(view, &layouts, &bindings.strip.sort_button, point, canvas) {
            // `ContentListSortDropDown.OnSortDropDownValueChanged`: the
            // dropdown's next option, the first one after the last.
            sounds.source_button(&layouts, view.key, &bindings.strip.sort_button);
            let sorts = order::sorts(kind);
            let list = state.list_mut(kind);
            let position = sorts.iter().position(|sort| *sort == list.sort).unwrap_or(0);
            list.sort = sorts[(position + 1) % sorts.len()];
            info!("[inventory] {kind:?} sort → OnSortDropDown: {:?}", list.sort);
            continue;
        }
        if hit(view, &layouts, &bindings.strip.order[0], point, canvas) {
            sounds.source_button(&layouts, view.key, &bindings.strip.order[0]);
            let list = state.list_mut(kind);
            list.order = list.order.reversed();
            info!("[inventory] {kind:?} order → OnSortOrder: {:?}", list.order);
            continue;
        }
        if let Some(toggle) = bindings.strip.filter_toggle.as_ref() {
            if kind.carries_fixture_tabs() && hit(view, &layouts, toggle, point, canvas) {
                sounds.source_button(&layouts, view.key, toggle);
                state.fixture_view = match state.fixture_view {
                    FixtureView::Default => FixtureView::InPlaceable,
                    FixtureView::InPlaceable => FixtureView::Default,
                };
                info!(
                    "[inventory] fixture filter toggle → OnFixtureFilterChanged: {:?}",
                    state.fixture_view
                );
                continue;
            }
        }
        if hit(view, &layouts, &bindings.strip.search, point, canvas) {
            sounds.source_button(&layouts, view.key, &bindings.strip.search);
            recycle::open_search(&mut screens);
            continue;
        }
        if let Some(index) = bindings
            .cells
            .iter()
            .position(|cell| hit(view, &layouts, &cell.button, point, canvas))
        {
            sounds.source_button(&layouts, view.key, &bindings.cells[index].button);
            state.selected[kind.index()] = index;
            continue;
        }
        if hit(view, &layouts, &bindings.recycle_button, point, canvas) {
            sounds.source_button(&layouts, view.key, &bindings.recycle_button);
            let Some(row) = bindings.rows.get(state.selected[kind.index()]) else {
                continue;
            };
            if !bindings.dialog.available {
                warn!(
                    "[inventory] OpenDisassembleDialog: this root lacks {}; the confirmation does not open",
                    view::CONFIRM
                );
                continue;
            }
            let row = row.clone();
            commands.queue(move |world: &mut World| {
                let name = row
                    .master
                    .as_ref()
                    .map_or_else(String::new, |master| master.name.clone());
                world.resource_scope(|world, mut recycle: Mut<Recycle>| {
                    recycle.open(world, kind, &row, &name);
                });
            });
            continue;
        }
    }
}
