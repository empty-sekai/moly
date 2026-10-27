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
//! runtime) and installed as one runtime document. Each cell is instantiated
//! from its qualified prefab under its list's content node. A root whose
//! document carries the Common1 chain is
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
use moly_assets::ui_layout::{UiComponent, UiPrefab, UiRootManifest};
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
use crate::server_panel_data::LiveSchedule;
use crate::userdata::ClientUserData;

/// The runtime document's key and the listed view name.
const KEY: &str = "MysekaiWeatherDialog";

/// Where the source document sits under a UI root.
const DOCUMENT_PATH: &str = "hud/MysekaiWeatherDialog.json";
const CELL_DOCUMENT_PATH: &str = "hud/MysekaiWeatherCell.json";

// JP7's serialized file external table maps the dialog's file 7 to this
// exact cell file. These are source identities, not runtime document hashes:
// a decoder upgrade may enrich the JSON without changing the Unity objects.
const DIALOG_FILE: &str = "57f29419b821f46fabb5cc48a3337f7e";
const DIALOG_SOURCE_SHA: &str = "e30634b2449c0e04aece70abb0ea2279866bf84113ecd25ebb7d1841d0c1f8bb";
const CELL_FILE: &str = "434bbf70d1c9142f09df58997f80b738";
const CELL_SOURCE_SHA: &str = "4a6172198b85a1432e6be54b4df5e774fda9d69d33df450467e042d90bc8cb5f";
const JP6_DIALOG_SOURCE_SHA: &str = "23dddf257506fdf0e1bdea65df3e285c98ed68c7dcc02a03a46b48dbb62922a4";
const JP6_CELL_SOURCE_SHA: &str = "5a9e34e071348e3496485f4fce98a363f18ab21a1b9bcf54a50215e302a927aa";

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
pub struct WeatherDialogRequest;

#[derive(Component)]
pub struct WeatherDialogRoot;

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
    lists: Vec<String>,
}

/// A cell drawn for the open, with its slot paths inside the cell document.
struct CellShow {
    slots: Vec<String>,
    phenomenon: Option<i32>,
}

/// Every cell has unique runtime identities inside the drawn dialog document.
struct Installed {
    document: UiPrefab,
    cells: Vec<Vec<Vec<String>>>,
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
    cell: Option<Handle<JsonAsset>>,
    manifest: Option<Handle<JsonAsset>>,
    region: String,
    root: String,
}

/// The dialog as the current open shows it.
struct Shown {
    id: DialogId,
    core: Core,
    cells: Vec<Vec<CellShow>>,
    close_node: String,
    /// How many first-list cells show their cover.
    cover: usize,
    current: Option<usize>,
    tomorrow: String,
}

/// Install the dialog's state, its request message and its systems.
pub fn install(app: &mut App) {
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

fn install_document(
    mut commands: Commands,
    mut weather: ResMut<WeatherDialog>,
    source: Option<Res<WeatherDialogSource>>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut layouts: ResMut<UiLayouts>,
) {
    if weather.installed.is_some() || weather.warned.contains(&"document") {
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
        let Some(root) = layouts.root().map(str::to_owned) else { return };
        let region_root = root == format!("moly://ui-{region}/");
        dialog.source = Some(Source {
            handle: server.load(format!("{root}{DOCUMENT_PATH}")),
            cell: None,
            manifest: region_root.then(|| server.load(format!("{root}ui-manifest.json"))),
            region: region.to_owned(),
            root,
        });
        return;
    }
    let source = dialog.source.as_mut().expect("the document request");
    let root = source.root.as_str();
    for (handle, name) in std::iter::once((&source.handle, DOCUMENT_PATH))
        .chain(source.manifest.as_ref().map(|h| (h, "ui-manifest.json"))) {
        match server.load_state(handle) {
        LoadState::Failed(error) => {
            // A root published without this document: the weather button
            // opens nothing, and the rest of the game runs.
            warn!(
                "[weather] {root}{name}: the weather source did not load ({error})"
            );
            dialog.warned.push("document");
            return;
        }
        LoadState::Loaded => {}
        _ => return,
        }
    }
    let Some(asset) = json.get(&source.handle) else { return };
    let mut document = UiPrefab::parse(&asset.0).unwrap_or_else(|e| panic!("[weather] {e}"));
    let needs_external = match needs_external_cell(&document) {
        Ok(external) => external,
        Err(reason) => {
            warn!("[weather] {root}{DOCUMENT_PATH} has no usable weather cell references: {reason}");
            dialog.warned.push("document");
            return;
        }
    };
    if needs_external && source.cell.is_none() {
        source.cell = Some(server.load(format!("{root}{CELL_DOCUMENT_PATH}")));
        return;
    }
    let cell = if needs_external {
        let handle = source.cell.as_ref().expect("external cell requested");
        match server.load_state(handle) {
            LoadState::Failed(error) => {
                error!("[weather] external cell failed to load: {error}");
                dialog.warned.push("document");
                return;
            }
            LoadState::Loaded => {}
            _ => return,
        }
        let Some(cell) = json.get(handle) else { return };
        Some(cell)
    } else { None };
    let mut template = cell.map(|cell| UiPrefab::parse(&cell.0).unwrap_or_else(|e| panic!("[weather] {e}")));
    let qualified = (|| -> Result<(), String> {
        if let Some(handle) = source.manifest.as_ref() {
            let manifest = json.get(handle).ok_or("weather root manifest unavailable")?;
            let manifest = UiRootManifest::parse(&manifest.0, &source.region)?;
            manifest.admit_prefab(DOCUMENT_PATH, &asset.0, &document)?;
            if let (Some(cell), Some(template)) = (cell, template.as_ref()) {
                manifest.admit_prefab(CELL_DOCUMENT_PATH, &cell.0, template)?;
            }
        }
        if let Some(cell) = cell {
            qualify_external_cell(&serde_json::from_str(&asset.0).map_err(|e| e.to_string())?,
                &serde_json::from_str(&cell.0).map_err(|e| e.to_string())?)?;
        }
        Ok(())
    })();
    if let Err(reason) = qualified {
        error!("[weather] source cell refused: {reason}");
        dialog.warned.push("document");
        return;
    }
    document.rebase_images(root);
    if let Some(template) = &mut template { template.rebase_images(root); }
    let cells = match compose_cells(&mut document, template.as_ref()) {
        Ok(cells) => cells,
        Err(reason) => {
            error!("[weather] cell composition refused: {reason}");
            dialog.warned.push("document");
            return;
        }
    };
    if let Err(reason) = layouts.replace_runtime_document(KEY, document.clone(), &server) {
        info!("[weather] {KEY} waits for the UI sources: {reason}");
        return;
    }
    info!(
        "[weather] {KEY} installed from {root}{DOCUMENT_PATH}: {} nodes, chain {}, {} cells",
        document.nodes.len(),
        carries_chain(&document),
        cells.iter().map(Vec::len).sum::<usize>()
    );
    let mut view = UiPrefabView::new(KEY, SITEMAP_LAYER);
    view.set_visible(&format!("@{}", document.nodes[0].game_object_id), true);
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
        lists.push(content);
    }
    Ok(Core {
        tomorrow_label,
        time_labels,
        time_highlight,
        lists,
    })
}

fn qualify_external_cell(dialog: &Value, cell: &Value) -> Result<(), String> {
    let version = dialog["source"]["clientVersion"].as_str();
    let (dialog_sha, cell_sha) = match version {
        Some("7.0.0") => (DIALOG_SOURCE_SHA, CELL_SOURCE_SHA),
        Some("6.8.1") => (JP6_DIALOG_SOURCE_SHA, JP6_CELL_SOURCE_SHA),
        _ => return Err("weather external cell source version has no verified relationship".into()),
    };
    for (document, file, sha) in [(dialog, DIALOG_FILE, dialog_sha), (cell, CELL_FILE, cell_sha)] {
        let source = &document["source"];
        if source["region"].as_str() != Some("jp")
            || source["clientVersion"].as_str() != version
            || source["serializedFile"].as_str() != Some(file)
            || source["sha256"].as_str() != Some(sha)
        {
            return Err("weather cell's serialized source does not match its verified external table".into());
        }
    }
    Ok(())
}

fn cell_pointer(value: &Value) -> Result<(i64, i64), String> {
    let pair = value.as_array().filter(|pair| pair.len() == 2).ok_or("weather cell PPtr is not a pair")?;
    let file = pair[0].as_i64().filter(|file| *file >= 0).ok_or("weather cell PPtr file is invalid")?;
    let id = pair[1].as_i64().filter(|id| *id != 0).ok_or("weather cell PPtr is null")?;
    Ok((file, id))
}

fn needs_external_cell(document: &UiPrefab) -> Result<bool, String> {
    let (_, dialog) = find(document, DIALOG).ok_or("no MysekaiWeatherDialog component")?;
    let lists = dialog.fields["_weatherCellList"].as_array().filter(|lists| !lists.is_empty()).ok_or("_weatherCellList")?;
    let mut external = false;
    for reference in lists {
        let list = list_component(document, pointer(reference, "cell list")?)?;
        external |= cell_pointer(&list.fields["_weatherCellPrefab"])?.0 != 0;
    }
    Ok(external)
}

/// Instantiate the external source object, retaining an independent identity
/// map for each cell. All four are in the document the one dialog view draws;
/// its authored GridLayoutGroups drive their positions.
fn compose_cells(document: &mut UiPrefab, external: Option<&UiPrefab>) -> Result<Vec<Vec<Vec<String>>>, String> {
    let local = document.clone();
    let (_, dialog) = find(document, DIALOG).ok_or("no MysekaiWeatherDialog component")?;
    let lists = dialog.fields["_weatherCellList"].as_array().ok_or("_weatherCellList")?
        .iter().map(|value| pointer(value, "cell list")).collect::<Result<Vec<_>, _>>()?;
    let mut result = Vec::new();
    for (list_index, list_id) in lists.into_iter().enumerate() {
        let list = list_component(document, list_id)?;
        let (file, id) = cell_pointer(&list.fields["_weatherCellPrefab"])?;
        let template = if file == 0 { &local } else {
            if (file, id) != (7, 19) {
                return Err("weather cell PPtr differs from its qualified external source (7,19)".into());
            }
            external.ok_or("qualified external weather cell is absent")?
        };
        let content = format!("@{}", pointer(&list.fields["_content"], "cell content")?);
        let selector = format!("@{id}");
        let cell_index = template.find(&selector)?;
        let cell = template.nodes[cell_index].components.iter()
            .find(|component| component.path_id == id && component.class == CELL)
            .ok_or("referenced source object is not MysekaiWeatherCell")?;
        let slots = ["_weatherIcon", "_cover", "_weatherLabel"].into_iter()
            .map(|field| {
                let id = pointer(&cell.fields[field], field)?;
                template.find(&format!("@{id}"))?;
                Ok(id)
            }).collect::<Result<Vec<i64>, String>>()?;
        let mut cells = Vec::new();
        if file == 0 {
            let index = document.find(&selector)?;
            document.nodes[index].active = false;
        }
        for index in 0..CELLS_PER_LIST {
            let instance = document.instantiate_subtree(&content, template, &selector,
                &format!("WeatherCell{list_index}{index}"))?;
            let root = document.find(&instance.root_selector())?;
            document.nodes[root].active = true;
            cells.push(slots.iter().map(|id| instance.selector(*id)).collect::<Result<Vec<_>, _>>()?);
        }
        result.push(cells);
    }
    Ok(result)
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
    let (_, dialog) = find(doc, DIALOG)?;
    let id = pointer(&dialog.fields["closeButton"], "closeButton").ok()?;
    let index = doc.find(&format!("@{id}")).ok()?;
    doc.nodes[index].components.iter()
        .any(|component| component.path_id == id && component.class == BUTTON)
        .then_some(index)
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// `Show1ButtonDialog(369)` then `Setup`.
pub fn open(
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
        let mut refused = false;
        for list in 0..core.lists.len() {
            let start = list * CELLS_PER_LIST;
            let Some(slice) = rows.get(start..start + CELLS_PER_LIST).filter(|s| !s.is_empty()) else {
                error!("[weather] Setup: weatherList is null for list {list}: the dialog is not set up");
                refused = true;
                break;
            };
            let mut shown = Vec::new();
            for (cell, row) in slice.iter().enumerate() {
                let Some(slots) = installed.cells.get(list).and_then(|cells| cells.get(cell)) else {
                    error!("[weather] cell {list}/{cell} is not installed: the dialog is not set up");
                    refused = true;
                    break;
                };
                // A phenomenon the catalogue does not hold has no master row:
                // the cell keeps its placeholder (the source logs it).
                let phenomenon = catalogue
                    .0
                    .iter()
                    .any(|option| option.metadata.as_ref().is_some_and(|m| m.id == row.2))
                    .then_some(row.2);
                shown.push(CellShow {
                    slots: slots.clone(),
                    phenomenon,
                });
            }
            if refused {
                break;
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
pub fn place(
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
            let slots = &shown_cell.slots;
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
pub fn click(
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
            layouts.queue_button_se(&mut sounds, KEY, &shown.close_node);
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
pub fn on_back_key(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    fn component(id: i64, class: &str, fields: Value, pointers: &[&str]) -> Value {
        json!({"pathId":id,"class":class,"enabled":true,"fields":fields,"pointerFields":pointers})
    }

    fn node(id: i64, transform: i64, parent: i64, path: &str, components: Vec<Value>) -> Value {
        json!({"gameObjectId":id,"transformId":transform,"parentTransformId":parent,
            "path":path,"name":path.rsplit('/').next().unwrap(),"active":true,"components":components,
            "rect":{"anchorsMin":[0,1],"anchorsMax":[0,1],"anchoredPosition":[0,0],
                "sizeDelta":[712,184],"pivot":[0,1],"localScale":[1,1,1],"localRotation":[0,0,0,1]}})
    }

    fn documents() -> (UiPrefab, UiPrefab) {
        let grid = |id| component(id, "UnityEngine.UI.GridLayoutGroup",
            json!({"m_CellSize":[348,184],"m_Spacing":[16,16],"m_Padding":[0,0,0,0],
                "m_ChildAlignment":0,"m_Constraint":1,"m_ConstraintCount":2,"m_StartAxis":0,"m_StartCorner":0}), &[]);
        let dialog = json!({"version":1,"prefab":"Dialog","nodes":[
            node(100,101,0,"Dialog",vec![component(102,DIALOG,json!({"_weatherCellList":[[0,113],[0,123]]}), &["/_weatherCellList/0","/_weatherCellList/1"])]),
            node(110,111,101,"Dialog/Today",vec![component(113,CELL_LIST,json!({"_content":[0,111],"_weatherCellPrefab":[7,19]}), &["/_content","/_weatherCellPrefab"]),grid(114)]),
            node(120,121,101,"Dialog/Tomorrow",vec![component(123,CELL_LIST,json!({"_content":[0,121],"_weatherCellPrefab":[7,19]}), &["/_content","/_weatherCellPrefab"]),grid(124)])]});
        let cell = json!({"version":1,"prefab":"Cell","nodes":[
            node(6,17,0,"Cell",vec![component(19,CELL,json!({"_weatherIcon":[0,21],"_cover":[0,1],"_weatherLabel":[0,23]}), &["/_weatherIcon","/_cover","/_weatherLabel"])]),
            node(8,18,17,"Cell/Icon",vec![component(21,IMAGE,json!({}), &[])]),
            node(1,2,17,"Cell/Cover",vec![]),
            node(9,24,17,"Cell/Label",vec![component(23,TEXT,json!({}), &[])])]});
        (UiPrefab::parse(&dialog.to_string()).unwrap(), UiPrefab::parse(&cell.to_string()).unwrap())
    }

    #[test]
    fn external_cells_have_independent_ids_and_use_the_source_grid() {
        let (mut dialog, template) = documents();
        let slots = compose_cells(&mut dialog, Some(&template)).unwrap();
        assert_eq!(slots.iter().map(Vec::len).collect::<Vec<_>>(), vec![2,2]);
        let unique: HashSet<_> = slots.iter().flatten().flatten().cloned().collect();
        assert_eq!(unique.len(), 12);
        assert!(unique.iter().all(|selector| dialog.find(selector).is_ok()));
        let empty = HashMap::new();
        let geometry = moly_assets::ui_layout::auto_layout::compute_overrides(
            &dialog, Vec2::new(1280.,720.), &empty, &HashMap::new(), |_,_,_| None).unwrap();
        let rects = dialog.resolve_with(Vec2::new(1280.,720.), &empty, &geometry);
        for list in ["Today", "Tomorrow"] {
            let list_index = if list == "Today" { 0 } else { 1 };
            let a = dialog.find(&format!("{list}/WeatherCell{list_index}0")).unwrap();
            let b = dialog.find(&format!("{list}/WeatherCell{list_index}1")).unwrap();
            assert_eq!(rects[a].size, Vec2::new(348.,184.));
            assert_eq!(rects[b].center().x - rects[a].center().x, 364.);
            assert_eq!(rects[b].center().y, rects[a].center().y);
            assert!(rects[a].active && rects[b].active);
        }
        assert_eq!(template.nodes[0].components[0].path_id, 19);
    }

    #[test]
    fn matching_object_id_from_another_external_file_is_refused() {
        let (mut dialog, template) = documents();
        dialog.nodes[1].components[0].fields["_weatherCellPrefab"] = json!([8,19]);
        assert!(compose_cells(&mut dialog,Some(&template)).unwrap_err().contains("PPtr"));
    }

    #[test]
    fn source_file_domain_is_required_even_when_prefab_names_match() {
        let mut dialog = json!({"source":{"region":"jp","clientVersion":"7.0.0",
            "serializedFile":DIALOG_FILE,"sha256":DIALOG_SOURCE_SHA}});
        let mut cell = json!({"source":{"region":"jp","clientVersion":"7.0.0",
            "serializedFile":CELL_FILE,"sha256":CELL_SOURCE_SHA}});
        qualify_external_cell(&dialog,&cell).unwrap();
        cell["source"]["serializedFile"] = json!("unrelated-file");
        assert!(qualify_external_cell(&dialog,&cell).is_err());
        cell["source"]["serializedFile"] = json!(CELL_FILE);
        dialog["source"]["sha256"] = json!("different-source-bytes");
        assert!(qualify_external_cell(&dialog,&cell).is_err());
    }

    #[test]
    fn local_weather_cells_need_no_jp_source_or_external_document() {
        let (mut dialog, template) = documents();
        let instance = dialog.instantiate_subtree("@101", &template, "@19", "LocalTemplate").unwrap();
        let id = instance.identity(19).unwrap();
        for node in &mut dialog.nodes {
            for component in &mut node.components {
                if component.class == CELL_LIST {
                    component.fields["_weatherCellPrefab"] = json!([0,id]);
                }
            }
        }
        assert!(!needs_external_cell(&dialog).unwrap());
        let slots = compose_cells(&mut dialog,None).unwrap();
        assert_eq!(slots.iter().map(Vec::len).collect::<Vec<_>>(), vec![2,2]);
        let unique: HashSet<_> = slots.iter().flatten().flatten().cloned().collect();
        assert_eq!(unique.len(),12);
        assert!(!dialog.nodes[dialog.find(&instance.root_selector()).unwrap()].active);
    }

    #[test]
    fn verified_jp_681_external_relationship_remains_supported() {
        let dialog = json!({"source":{"region":"jp","clientVersion":"6.8.1",
            "serializedFile":DIALOG_FILE,"sha256":JP6_DIALOG_SOURCE_SHA}});
        let cell = json!({"source":{"region":"jp","clientVersion":"6.8.1",
            "serializedFile":CELL_FILE,"sha256":JP6_CELL_SOURCE_SHA}});
        qualify_external_cell(&dialog,&cell).unwrap();
    }
}
