//! The weather forecast dialog (`MysekaiWeatherDialog`, dialog type 369),
//! opened by the site map's weather button
//! (`ScreenLayerMysekaiSiteMap.OnClickWeatherButton`).
//!
//! The source, in order:
//! - `Setup`: the schedule rows of `UserMysekaiPhenomenaSchedules`, ordered by
//!   their date. The last row's date, read in the device's local time as
//!   `MM/dd`, goes into `_tomorrowLabel`; then the two cell lists
//!   (`_weatherCellList`, today and tomorrow) each take two consecutive rows
//!   from the start of that order and instantiate their cell prefab under
//!   their content transform. A cell shows the phenomenon master row of its
//!   schedule row: the icon texture `AssetBundleLoader.LoadAsset(
//!   iconAssetbundleName)` into `_weatherIcon`, whose object is turned on
//!   first, and the row's `name` into `_weatherLabel`. A range of no rows
//!   logs `weatherList is null` and returns.
//! - `SetTimeLabel`: the master refresh windows in table order; `_timeLabel[i]`
//!   becomes `new TimeSpan(startHour, 0, 0)` formatted `hh\:mm`. The window
//!   holding `UserMysekaiGamedata.refreshedAt` — its time of day moved past
//!   midnight when it is before 5 a.m. — takes the highlight palette entry
//!   (`text_wh`), the others the one their serialized colour is (`text_dbl`).
//!   `_timeHighlight` is moved onto the highlighted label (the source writes
//!   a zero vector, the finished code the label's own position) and
//!   `_currTimeIndex` becomes that label's index.
//! - `SetWeatherCellGreyOut`: once the second window is the current one, each
//!   cell of the first list below `_currTimeIndex` shows its cover.
//! - `OnClickWeatherButton`: `Show1ButtonDialog<MysekaiWeatherDialog>(369)`
//!   (`Common1ButtonDialog.Initialize` binds the footer button as OK and the
//!   `OnClickOK` reply), then `Setup`.
//! - `Common1ButtonDialog.OnClickOK` runs its (null reply) callback and
//!   closes; `OnCloseExternal` and `OnHardwareBackKeyProcess` both call it,
//!   and `DialogBase.SetCloseExternal` makes the close button call
//!   `OnCloseExternal`. The dialog holds no `SubWindowComponent` and plays no
//!   sub-window fade: it closes at once.
//!
//! Host mapping: the dialog takes the dialog slot
//! (`ScreenManager::show_dialog` for [`DialogType::MysekaiWeatherDialog`]).
//! Its prefab is not one of the UI root's requested documents, so it is read
//! from the root the runtime's region draws from (`moly://ui-jp/` in a JP
//! runtime) and installed as a runtime document; each cell is the document's
//! own cell prefab instantiated under its list's content node
//! ([`cell_document`]). A root whose document carries the Common1 chain is
//! drawn with it; the shared root's copy has none, and its dialog is drawn as
//! it is.
//!
//! Named gaps: `DialogBase`'s window scale tween and the background cover are
//! not drawn. A cell's cover is a prefab reference the document does not
//! carry, so a shown cover is the cell's own cover object with nothing in it;
//! a cell's icon is the phenomenon's site-map icon (the runtime Sprite
//! `site-map-phenomena/icon_` + `iconAssetbundleName`, the art the notice
//! banner and the map draw), because the thumbnail bundle
//! `mysekai/thumbnail/phenomena/` is not on this root.

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use moly_masters::clock::RefreshPeriod;
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::client::LiveSchedule;
use crate::userdata::ClientUserData;

/// The runtime document's key and the listed view name.
const KEY: &str = "MysekaiWeatherDialog";

/// Where the source document sits under a UI root.
const DOCUMENT_PATH: &str = "hud/MysekaiWeatherDialog.json";

/// The phenomena index: its refresh windows.
const PHENOMENA_INDEX: &str = "moly://phenomena/index.json";

/// `MysekaiWeatherCellList.CELL_COUNT`: the cells each list instantiates.
const CELLS_PER_LIST: usize = 2;

/// The classes the dialog's code reads.
const DIALOG: &str = "Sekai.Mysekai.MysekaiWeatherDialog";
const CELL_LIST: &str = "Sekai.Mysekai.MysekaiWeatherCellList";
const CELL: &str = "Sekai.Mysekai.MysekaiWeatherCell";
const TEXT: &str = "Sekai.UI.CustomTextMesh";
const BUTTON: &str = "Sekai.UI.CustomButton";
const COMMON_BUTTON: &str = "Sekai.UI.UIPartsCommonButton";
const IMAGE: &str = "Sekai.UI.CustomImage";

/// The chain the dialog's class descends from.
const CHAIN: &str = "Sekai.Common1ButtonDialog";

/// The value `DialogBase`'s SE table gives `OpenSE.Default`.
const OPEN_SE: &str = "SE_UI_DIALOG_OPEN";

/// `MysekaiWeatherUtility.IsInRefreshTime` reads a time of day before 5 a.m.
/// as the day before, after midnight. The windows are master rows.
const AFTER_MIDNIGHT_HOUR: i64 = 5;

/// The palette entries of the dialog's serialized text colours: white
/// (`text_wh`) for the highlighted window, `text_dbl` for the others.
const HIGHLIGHT_COLOR: usize = 18;
const NORMAL_COLOR: usize = 14;

/// The runtime sprites the weather chain extracts (`site-map-phenomena/`).
const ICON_PREFIX: &str = "site-map-phenomena/icon_";

const DIALOG_Z: f32 = 100.0;

/// `Show1ButtonDialog<MysekaiWeatherDialog>(369)` was called: it carries no
/// message, no OK label and a null reply.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct WeatherDialogRequest;

#[derive(Component)]
pub(crate) struct WeatherDialogRoot;

/// The master refresh windows, once the phenomena index arrives.
#[derive(Resource, Default)]
struct WeatherPeriods(Option<Result<Vec<RefreshPeriod>, String>>);

impl WeatherPeriods {
    fn get(&self) -> &[RefreshPeriod] {
        match self.0.as_ref() {
            Some(Ok(periods)) => periods.as_slice(),
            _ => &[],
        }
    }
}

/// The dialog's own code's targets.
struct Core {
    tomorrow_label: String,
    time_labels: Vec<String>,
    time_highlight: String,
    lists: Vec<CoreList>,
}

struct CoreList {
    /// The content node's path: its cell instances are named under it.
    content: String,
}

/// A cell drawn for the open, with its slot paths inside the cell document.
struct CellShow {
    key: &'static str,
    root: String,
    phenomenon: Option<i32>,
}

/// The document as it was read, and the per-cell documents installed.
struct Installed {
    document: UiPrefab,
    cells: Vec<(usize, usize)>,
}

#[derive(Resource, Default)]
struct WeatherDialog {
    source: Option<Source>,
    installed: Option<Installed>,
    shown: Option<Shown>,
    warned: Vec<&'static str>,
}

struct Source {
    handle: Handle<JsonAsset>,
    root: &'static str,
}

/// The dialog as the current open shows it.
struct Shown {
    id: DialogId,
    core: Core,
    cells: Vec<Vec<CellShow>>,
    /// The cell slots of each list, from the list's cell prefab.
    slots: Vec<Vec<String>>,
    close_node: String,
    /// How many first-list cells show their cover.
    cover: usize,
    current: Option<usize>,
    tomorrow: String,
}

/// Install the dialog's state, its request message and its systems.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<WeatherDialog>()
        .init_resource::<WeatherPeriods>()
        .add_message::<WeatherDialogRequest>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (parse_periods, install_document, open, place, click, on_back_key).chain(),
        );
}

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(WeatherDialogSource {
        source: server.load("moly://source.json"),
        periods: server.load(PHENOMENA_INDEX),
    });
}

#[derive(Resource)]
struct WeatherDialogSource {
    source: Handle<JsonAsset>,
    periods: Handle<JsonAsset>,
}

// ---------------------------------------------------------------------------
// The refresh windows
// ---------------------------------------------------------------------------

fn parse_periods(
    source: Option<Res<WeatherDialogSource>>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut periods: ResMut<WeatherPeriods>,
) {
    if periods.0.is_some() {
        return;
    }
    let Some(source) = source else { return };
    if let LoadState::Failed(error) = server.load_state(&source.periods) {
        let reason = format!("{PHENOMENA_INDEX} did not load ({error})");
        error!("[weather] the refresh windows are absent ({reason}): the time labels and the highlight stay off");
        periods.0 = Some(Err(reason));
        return;
    }
    let Some(asset) = jsons.get(&source.periods) else { return };
    let result: Result<Vec<RefreshPeriod>, String> = serde_json::from_str::<Value>(&asset.0)
        .map_err(|e| format!("not JSON: {e}"))
        .and_then(|value| {
            let rows = value["refreshTimePeriods"]
                .as_array()
                .ok_or_else(|| "no refreshTimePeriods".to_owned())?;
            moly_masters::clock::parse_periods(rows)
        });
    match &result {
        Ok(rows) => info!("[weather] {} refresh windows", rows.len()),
        Err(reason) => error!(
            "[weather] the refresh windows are absent ({reason}): the time labels and the highlight stay off"
        ),
    }
    periods.0 = Some(result);
}

// ---------------------------------------------------------------------------
// Loading the dialog
// ---------------------------------------------------------------------------

/// The UI root the runtime's region selects.
fn root_for_region(region: &str) -> Option<&'static str> {
    match region {
        "jp" => Some("moly://ui-jp/"),
        _ => None,
    }
}

fn install_document(
    mut commands: Commands,
    mut weather: ResMut<WeatherDialog>,
    source: Option<Res<WeatherDialogSource>>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut layouts: ResMut<UiLayouts>,
) {
    if weather.installed.is_some() {
        return;
    }
    let Some(source) = source else { return };
    let dialog = &mut *weather;
    if dialog.source.is_none() {
        if let LoadState::Failed(error) = server.load_state(&source.source) {
            panic!("[weather] the runtime identity failed to load ({error})");
        }
        let Some(asset) = json.get(&source.source) else { return };
        let value: Value = serde_json::from_str(&asset.0)
            .unwrap_or_else(|e| panic!("[weather] source.json: {e}"));
        let region = value["source"]["region"].as_str().unwrap_or_default();
        let Some(root) = root_for_region(region) else {
            if !dialog.warned.contains(&"shared-root") {
                dialog.warned.push("shared-root");
                info!(
                    "[weather] region {region} draws the shared root, which carries this \
                     dialog's document without its chain: the weather button opens it only \
                     where a region root carries it"
                );
            }
            return;
        };
        dialog.source = Some(Source {
            handle: server.load(format!("{root}{DOCUMENT_PATH}")),
            root,
        });
        return;
    }
    let source = dialog.source.as_ref().expect("the document request");
    let root = source.root;
    match server.load_state(&source.handle) {
        LoadState::Failed(error) => panic!(
            "[weather] {root}{DOCUMENT_PATH}: this UI root does not carry the weather dialog ({error})"
        ),
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&source.handle) else { return };
    let mut document = UiPrefab::parse(&asset.0).unwrap_or_else(|e| panic!("[weather] {e}"));
    document.rebase_images(root);
    let core = match core(&document) {
        Ok(core) => core,
        Err(reason) => panic!("[weather] {KEY}: {reason}"),
    };
    // One cell document per list and cell, installed before the first open.
    let mut cells = Vec::new();
    for list in 0..core.lists.len() {
        for cell in 0..CELLS_PER_LIST {
            let scope = format!("WeatherCell{list}{cell}");
            match cell_document(&document, list, &scope) {
                Ok(doc) => match layouts.replace_runtime_document(cell_key(list, cell), doc, &server) {
                    Ok(()) => cells.push((list, cell)),
                    Err(reason) => info!(
                        "[weather] cell {list}/{cell} waits for the UI sources: {reason}"
                    ),
                },
                Err(reason) => error!("[weather] cell {list}/{cell} is not composed ({reason})"),
            }
        }
    }
    if let Err(reason) = layouts.replace_runtime_document(KEY, document.clone(), &server) {
        info!("[weather] {KEY} waits for the UI sources: {reason}");
        return;
    }
    info!(
        "[weather] {KEY} installed from {root}{DOCUMENT_PATH}: {} nodes, chain {}, {} cells",
        document.nodes.len(),
        carries_chain(&document),
        cells.len()
    );
    let mut view = UiPrefabView::new(KEY, SITEMAP_LAYER);
    view.set_visible(&format!("@{}", document.nodes[0].game_object_id), false);
    commands.spawn((
        WeatherDialogRoot,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., DIALOG_Z),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    dialog.installed = Some(Installed { document, cells });
    dialog.source = None;
}

/// Two lists of two cells: four fixed keys.
fn cell_key(list: usize, cell: usize) -> &'static str {
    match (list, cell) {
        (0, 0) => "MysekaiWeatherDialog.Cell00",
        (0, 1) => "MysekaiWeatherDialog.Cell01",
        (1, 0) => "MysekaiWeatherDialog.Cell10",
        _ => "MysekaiWeatherDialog.Cell11",
    }
}

fn carries_chain(doc: &UiPrefab) -> bool {
    doc.nodes
        .iter()
        .flat_map(|node| &node.components)
        .any(|c| c.class == CHAIN)
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

fn find<'a>(doc: &'a UiPrefab, class: &str) -> Option<(usize, &'a UiComponent)> {
    doc.nodes
        .iter()
        .enumerate()
        .flat_map(|(index, node)| node.components.iter().map(move |c| (index, c)))
        .find(|(_, c)| c.class == class)
}

/// The node index and its `@` path.
fn node_path(doc: &UiPrefab, id: i64, what: &str) -> Result<(usize, String), String> {
    let index = doc.find(&format!("@{id}")).map_err(|e| format!("{what}: {e}"))?;
    Ok((index, format!("@{}", doc.nodes[index].game_object_id)))
}

fn list_component<'a>(doc: &'a UiPrefab, id: i64) -> Result<&'a UiComponent, String> {
    doc.nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == id && c.class == CELL_LIST)
        .ok_or_else(|| format!("list {id} is not a {CELL_LIST}"))
}

fn core(doc: &UiPrefab) -> Result<Core, String> {
    let (_, dialog) = find(doc, DIALOG).ok_or("no MysekaiWeatherDialog component")?;
    let f = &dialog.fields;
    let (_, tomorrow_label) = node_path(doc, pointer(&f["_tomorrowLabel"], "_tomorrowLabel")?, "tomorrow label")?;
    let time_labels = f["_timeLabel"]
        .as_array()
        .ok_or("_timeLabel")?
        .iter()
        .enumerate()
        .map(|(i, value)| {
            node_path(doc, pointer(value, &format!("_timeLabel[{i}]"))?, "time label").map(|(_, p)| p)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (_, time_highlight) = node_path(doc, pointer(&f["_timeHighlight"], "_timeHighlight")?, "highlight")?;
    let mut lists = Vec::new();
    for (index, value) in f["_weatherCellList"]
        .as_array()
        .ok_or("_weatherCellList")?
        .iter()
        .enumerate()
    {
        let list = list_component(doc, pointer(value, &format!("_weatherCellList[{index}]"))?)?;
        let (_, content) =
            node_path(doc, pointer(&list.fields["_content"], "cell list _content")?, "cell content")?;
        lists.push(CoreList { content });
    }
    Ok(Core {
        tomorrow_label,
        time_labels,
        time_highlight,
        lists,
    })
}

/// The cell prefab's three slots (its icon, cover and label objects), as the
/// cell document's own `@` paths.
fn cell_slots(document: &UiPrefab, list_index: usize) -> Result<Vec<String>, String> {
    let (_, dialog) = find(document, DIALOG).ok_or("no MysekaiWeatherDialog component")?;
    let list_id = pointer(&dialog.fields["_weatherCellList"][list_index], "cell list")?;
    let list = list_component(document, list_id)?;
    let template_id = pointer(&list.fields["_weatherCellPrefab"], "_weatherCellPrefab")?;
    let template = document
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == template_id)
        .ok_or("the cell list's cell prefab")?;
    if template.class != CELL {
        return Err("the cell list's cell prefab is not MysekaiWeatherCell".into());
    }
    ["_weatherIcon", "_cover", "_weatherLabel"]
        .iter()
        .map(|field| node_path(document, pointer(&template.fields[*field], field)?, field).map(|(_, p)| p))
        .collect()
}

// ---------------------------------------------------------------------------
// A cell drawn under its list
// ---------------------------------------------------------------------------

/// The cell prefab turned into a document of its own: the cell's subtree root
/// takes `scope` and is parented to the list's content node (the source's
/// `Object.Instantiate(cellPrefab, content)`, under a canvas of its own).
fn cell_document(document: &UiPrefab, list: usize, scope: &str) -> Result<UiPrefab, String> {
    let (_, dialog) = find(document, DIALOG).ok_or("no MysekaiWeatherDialog component")?;
    let list_id = pointer(&dialog.fields["_weatherCellList"][list], "cell list")?;
    let list_component = list_component(document, list_id)?;
    let template_id = pointer(&list_component.fields["_weatherCellPrefab"], "_weatherCellPrefab")?;
    let template_index = document
        .nodes
        .iter()
        .position(|node| node.components.iter().any(|c| c.path_id == template_id))
        .ok_or("the cell prefab's root")?;
    let (content_index, content_path) =
        node_path(document, pointer(&list_component.fields["_content"], "_content")?, "content")?;
    let content_transform = document.nodes[content_index].transform_id;
    let template_path = document.nodes[template_index].path.clone();
    let template_transform = document.nodes[template_index].transform_id;
    let root_path = format!("{content_path}/{scope}");
    let mut out = document.clone();
    let mut moved = false;
    for node in &mut out.nodes {
        if node.transform_id == template_transform {
            node.name = scope.to_owned();
            node.path = root_path.clone();
            node.parent_transform_id = content_transform;
            moved = true;
        } else if let Some(suffix) = node.path.strip_prefix(&format!("{template_path}/")) {
            node.path = format!("{root_path}/{suffix}");
        }
    }
    if !moved {
        return Err("the cell prefab's root is not in the document".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The schedule
// ---------------------------------------------------------------------------

/// Epoch milliseconds as the proleptic Gregorian (year, month, day), after
/// `CivilFromDays` (the days from 1970-01-01, shifted to an era of 400
/// years).
fn civil(epoch_ms: i64) -> (i64, i64, i64) {
    let days = epoch_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn local_ms(epoch_ms: i64) -> i64 {
    epoch_ms + crate::userdata::clock::device_utc_offset_ms(epoch_ms)
}

/// `DateTime.ToString("MM/dd")` of an epoch millisecond in local time.
fn month_day(epoch_ms: i64) -> String {
    let (_, month, day) = civil(local_ms(epoch_ms));
    format!("{month:02}/{day:02}")
}

/// `new TimeSpan(hour, 0, 0).ToString("hh\\:mm")`.
fn window_label(start_hour: i64) -> String {
    format!("{:02}:00", start_hour.rem_euclid(24))
}

/// The schedule as `LiveSchedule` holds it, ordered by date
/// (`MysekaiWeatherUtility.GetOrderWeatherList` sorts on the date).
fn ordered(schedule: &LiveSchedule) -> Vec<(i32, i64, i32)> {
    let mut rows: Vec<(i32, i64, i32)> = schedule
        .schedules
        .iter()
        .map(|row| (row.refresh_time_period_id, row.schedule_date, row.phenomena_id))
        .collect();
    rows.sort_by_key(|row| row.1);
    rows
}

/// The index of the window holding `refreshed_at`, read as
/// `MysekaiWeatherUtility.IsInRefreshTime` reads the clock against its two
/// edges (a time of day before 5 a.m. is after midnight).
fn current_window(periods: &[RefreshPeriod], refreshed_at: i64) -> Option<usize> {
    let hour = local_ms(refreshed_at).div_euclid(3_600_000).rem_euclid(24);
    let offset = if hour < AFTER_MIDNIGHT_HOUR { hour + 24 } else { hour };
    periods
        .iter()
        .position(|period| period.start_hour <= offset && offset < period.end_hour)
}

/// `DialogBase.SetCloseExternal`: the dialog's own close button — the button
/// that is not the footer's, over an image.
fn close_button_of(doc: &UiPrefab) -> Option<usize> {
    (0..doc.nodes.len()).rev().find(|index| {
        let node = &doc.nodes[*index];
        !node.components.iter().any(|c| c.class == COMMON_BUTTON)
            && node.components.iter().any(|c| c.class == IMAGE)
            && node
                .components
                .iter()
                .any(|c| c.class == BUTTON && c.fields.get("m_Interactable").is_some())
    })
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// `Show1ButtonDialog(369)` then `Setup`.
pub(crate) fn open(
    mut requests: MessageReader<WeatherDialogRequest>,
    mut weather: ResMut<WeatherDialog>,
    mut screens: ResMut<ScreenManager>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    periods: Res<WeatherPeriods>,
    schedule: Res<LiveSchedule>,
    user: Option<Res<ClientUserData>>,
) {
    for _ in requests.read() {
        let dialog = &mut *weather;
        let Some(installed) = dialog.installed.as_ref() else {
            error!("[weather] Show1ButtonDialog(369) before the dialog's document is installed: refused");
            continue;
        };
        if dialog.shown.is_some() {
            error!("[weather] Show1ButtonDialog(369) while the dialog is shown: the shown one stays");
            continue;
        }
        let Ok(core) = core(&installed.document) else {
            error!("[weather] the dialog names no cell list: it does not open");
            continue;
        };
        let Some(close_index) = close_button_of(&installed.document) else {
            error!("[weather] the dialog has no close button: it does not open");
            continue;
        };
        let rows = ordered(&schedule);
        let refreshed_at = user.as_deref().map_or(0, |user| user.gamedata.refreshed_at);
        let current = current_window(periods.get(), refreshed_at);
        let mut cells = Vec::new();
        let mut slots = Vec::new();
        let mut refused = false;
        for (list, list_core) in core.lists.iter().enumerate() {
            let start = list * CELLS_PER_LIST;
            let Some(slice) = rows.get(start..start + CELLS_PER_LIST).filter(|s| !s.is_empty()) else {
                error!("[weather] Setup: weatherList is null for list {list}: the dialog is not set up");
                refused = true;
                break;
            };
            let mut shown = Vec::new();
            for (cell, row) in slice.iter().enumerate() {
                if !installed.cells.contains(&(list, cell)) {
                    error!("[weather] cell {list}/{cell} is not installed: the dialog is not set up");
                    refused = true;
                    break;
                }
                // A phenomenon the catalogue does not hold has no master row:
                // the cell keeps its placeholder (the source logs it).
                let phenomenon = catalogue
                    .0
                    .iter()
                    .any(|option| option.metadata.as_ref().is_some_and(|m| m.id == row.2))
                    .then_some(row.2);
                shown.push(CellShow {
                    key: cell_key(list, cell),
                    root: format!("{}/WeatherCell{list}{cell}", list_core.content),
                    phenomenon,
                });
            }
            if refused {
                break;
            }
            match cell_slots(&installed.document, list) {
                Ok(list_slots) => slots.push(list_slots),
                Err(reason) => {
                    error!("[weather] list {list}: {reason}");
                    refused = true;
                    break;
                }
            }
            cells.push(shown);
        }
        if refused {
            continue;
        }
        let id = match screens.show_dialog(
            DialogType::MysekaiWeatherDialog,
            DisplayLayerType::LayerDialog,
            DialogBackKey::Close,
            "ScreenLayerMysekaiSiteMap.OnClickWeatherButton",
        ) {
            Ok(id) => id,
            Err(reason) => {
                error!("[weather] {reason}: the dialog does not open");
                continue;
            }
        };
        screens.open_dialog(id);
        screens.dialog_open_finished(id);
        let close_node = format!("@{}", installed.document.nodes[close_index].game_object_id);
        let cover = current.map_or(0, |index| index.min(CELLS_PER_LIST));
        let tomorrow = rows.last().map_or_else(String::new, |row| month_day(row.1));
        let phenomena: Vec<Vec<Option<i32>>> = cells
            .iter()
            .map(|list| list.iter().map(|cell| cell.phenomenon).collect())
            .collect();
        info!(
            "[weather] Show1ButtonDialog(369): Open plays {OPEN_SE}; Setup: tomorrow \
             \"{tomorrow}\", current window {current:?}, {cover} first-list covers, cells \
             {phenomena:?}, chain {}",
            carries_chain(&installed.document)
        );
        dialog.shown = Some(Shown {
            id,
            core,
            cells,
            slots,
            close_node,
            cover,
            current,
            tomorrow,
        });
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The dialog's texts and window colours once it opens.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    mut weather: ResMut<WeatherDialog>,
    periods: Res<WeatherPeriods>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<UiLayouts>,
    mut roots: Query<(&mut Visibility, &mut Transform, &mut UiPrefabView), With<WeatherDialogRoot>>,
    mut sounds: ResMut<SeRequests>,
    mut opened: Local<Option<DialogId>>,
) {
    let Ok((mut visibility, mut transform, mut view)) = roots.single_mut() else {
        return;
    };
    if let (Ok(window), Some(canvas)) = (windows.single(), root_canvas.as_deref()) {
        crate::canvas::set_scale(&mut transform, canvas.scale(window));
    }
    let weather = &mut *weather;
    let Some(shown) = weather.shown.as_ref() else {
        visibility.set_if_neq(Visibility::Hidden);
        *opened = None;
        return;
    };
    visibility.set_if_neq(Visibility::Inherited);
    if *opened != Some(shown.id) {
        *opened = Some(shown.id);
        sounds.0.push(SeRequest {
            owner: None,
            cue: OPEN_SE.to_owned(),
            class: SeClass::Ui,
            source: "weather_dialog",
        });
        // Setup: the last row's date, then the windows' labels and colours.
        view.set_text(&shown.core.tomorrow_label, shown.tomorrow.clone());
        let periods = periods.get();
        for (index, label) in shown.core.time_labels.iter().enumerate() {
            if let Some(period) = periods.get(index) {
                view.set_text(label, window_label(period.start_hour));
            }
        }
        if let Some(current) = shown.current {
            for (index, label) in shown.core.time_labels.iter().enumerate() {
                let entry = if index == current { HIGHLIGHT_COLOR } else { NORMAL_COLOR };
                let color = layouts.palette_color(entry);
                if let (Some(color), Some(id)) = (color, text_component(&layouts, label)) {
                    view.set_graphic_color(id, color);
                }
            }
        }
    }
    // The cells: each cell document's icon takes the phenomenon's runtime
    // sprite, its label the master's name, and a covered position shows the
    // cover object (first list only, below the current window).
    for (list, cells) in shown.cells.iter().enumerate() {
        for (cell, shown_cell) in cells.iter().enumerate() {
            // A cell that did not install keeps its placeholders.
            if layouts.document(shown_cell.key).is_none() {
                continue;
            }
            let slots = &shown.slots[list];
            if let Some(phenomenon) = shown_cell.phenomenon {
                if let Some(master) = catalogue.0.iter().find_map(|option| {
                    option.metadata.as_ref().filter(|m| m.id == phenomenon)
                }) {
                    if let Some(icon) = master.icon_assetbundle_name.as_deref() {
                        let alias = format!("{ICON_PREFIX}{icon}");
                        if layouts.has_runtime_texture(&alias) {
                            view.set_texture(&slots[0], &alias);
                        } else if !weather.warned.contains(&"icon") {
                            weather.warned.push("icon");
                            error!(
                                "[weather] this root carries no runtime Sprite {alias}: the cell icons stay empty"
                            );
                        }
                    }
                    view.set_text(&slots[2], master.name.clone());
                }
            }
            let covered = list == 0 && cell < shown.cover;
            view.set_visible(&slots[1], covered);
        }
    }
}

/// The text component of the node at `path`.
fn text_component(layouts: &UiLayouts, path: &str) -> Option<i64> {
    let document = layouts.document(KEY)?;
    let index = document.find(path).ok()?;
    document.nodes[index]
        .components
        .iter()
        .find(|c| c.class == TEXT)
        .map(|c| c.path_id)
}

// ---------------------------------------------------------------------------
// Taps and the back key
// ---------------------------------------------------------------------------

/// One tap while the dialog is shown: the close button closes it
/// (`OnCloseExternal` -> `OnClickOK`); any other tap is the dialog's (the
/// dialog slot is modal).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut weather: ResMut<WeatherDialog>,
    mut screens: ResMut<ScreenManager>,
    mut consumed: ResMut<ActionTapConsumed>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<&UiPrefabView, With<WeatherDialogRoot>>,
    mut sounds: ResMut<SeRequests>,
) {
    let weather = &mut *weather;
    let Some(shown) = weather.shown.as_ref() else {
        gestures.clear();
        return;
    };
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(canvas), Ok(view)) = (windows.single(), root_canvas.as_deref(), views.single()) else {
        return;
    };
    let size = canvas.size(window);
    let scale = canvas.scale(window);
    for position in taps {
        consumed.0 = true;
        let point = Vec2::new(position.x - window.width() / 2.0, window.height() / 2.0 - position.y) / scale;
        if view
            .rect(&layouts, &shown.close_node, size)
            .is_some_and(|rect| rect.active && rect.contains(point))
        {
            // The close button's `CustomSelectableDefine.PlaySE`, then the
            // close.
            sounds.source_button(&layouts, KEY, &shown.close_node);
            close_by(
                &mut weather.shown,
                &mut screens,
                "tap on the close button (OnCloseExternal -> OnClickOK)",
            );
            return;
        }
    }
}

/// `OnClickOK` and the close: the slot frees at once (no sub-window fade runs
/// here).
fn close_by(shown: &mut Option<Shown>, screens: &mut ScreenManager, by: &str) {
    let Some(current) = shown.take() else { return };
    info!("[weather] {id:?}: {by}", id = current.id);
    screens.close_dialog(current.id);
    screens.dialog_destroyed(current.id);
}

/// `Common1ButtonDialog.OnHardwareBackKeyProcess` -> `OnClickOK` for the
/// dialog the screen manager hands the key to.
pub(crate) fn on_back_key(
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut weather: ResMut<WeatherDialog>,
    mut screens: ResMut<ScreenManager>,
) {
    for event in back_keys.read() {
        let weather = &mut *weather;
        let Some(shown) = weather.shown.as_ref() else { continue };
        if shown.id == event.id {
            close_by(
                &mut weather.shown,
                &mut screens,
                "back key (OnHardwareBackKeyProcess -> OnClickOK)",
            );
        }
    }
}
