//! The collect-item notice: `ScreenLayerMysekaiNotice.NoticeCollectItem`, the
//! entry point every collect notice goes through, and its slide cells.
//!
//! The source: the openers (harvest pickups, delivery, crafting and talk
//! replies) call `MysekaiResourceUtility.NoticeCollectItem(resourceType,
//! resourceId, quantity)`, which resolves the item's name, its icon's bundle
//! and asset names and the limit, new and rare flags from the masters and the
//! user's data, then `MysekaiUtility.NoticeCollectItem` hands those seven
//! values to the notice layer. The layer's `NoticeCollectItem` takes a view
//! data object from its pool of 64, sets the seven values and a notice
//! duration of 2.5 s when the item is new or rare and 1.5 s otherwise, and
//! enqueues it on the collect-item slide notice controller, initialized with
//! 5 cells, the layer's `collectItemCellPrefab` under `collectItemRoot`,
//! sliding in from the left ([`slide`]).
//!
//! [`NoticeCollectItem`] is that layer call, with the same seven values. The
//! harvest opener raises its notice by id; [`bridge`] resolves the name and
//! the icon's bundle and asset names from the masters, as
//! `MysekaiResourceUtility.NoticeCollectItem` does, and makes the layer call.
//! This module draws the cells ([`cell`]) in the notice
//! layer's view ([`crate::notice_banner`]), which shows only while the notice
//! layer is active. A call while the layer is not active, or before the
//! cells are composed, is reported and not drawn.
//!
//! Glyphs: the one text atlas is baked once, so [`CollectNoticeCharset`]
//! gives it every name the masters can put in a cell (materials, items,
//! fixtures) and the count's characters. A name with a glyph outside the
//! atlas is refused by name, not drawn with holes.
//!
//! Native instrument: `MOLY_NOTICE_COLLECT_AUTOPLAY` (game mode reads none)
//! sends a scripted burst of calls built from the masters two seconds after
//! the cells are ready, so a product run shows notices without an opener:
//! plain, rare, new, limited, a rare and new one, an item, and two more
//! plain ones (eight calls: five drivers, three of them queue a second).
//! The value is how many of them to send; a value that is not a number sends
//! all of them.

mod cell;
pub(crate) mod slide;

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use crate::notice_banner::{NoticeRoot, KEY};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use cell::{CellBinding, LetterTween, RARE_DELAY};
use slide::{CellCall, NoticeData, SlideNotices, PARALLEL};

/// `ScreenLayerMysekaiNotice.NoticeCollectItem(itemName, assetBundleName,
/// resourceName, getCount, isLimit, isNew, isRare)`, one per collected item.
#[derive(Message, Clone, Debug, PartialEq)]
pub(crate) struct NoticeCollectItem {
    /// The item's display name.
    pub(crate) item_name: String,
    /// The bundle the icon is loaded from (for a material,
    /// `AssetBundleNames.GetMysekaiMaterialPreview(iconAssetbundleName)`).
    pub(crate) asset_bundle_name: String,
    /// The icon's asset name inside that bundle.
    pub(crate) resource_name: String,
    /// The quantity collected.
    pub(crate) get_count: i32,
    /// The inventory cannot take the item.
    pub(crate) is_limit: bool,
    /// The item is collected for the first time.
    pub(crate) is_new: bool,
    /// The item is rare. For a material the opener sets it from the master's
    /// `MysekaiMaterialRarityType` (rarity_1 = 0, rarity_2 = 1, rarity_3 = 2,
    /// rarity_4 = 3) with one unsigned compare, `rarity - 1 < 3`: rarity_2 to
    /// rarity_4 are rare, rarity_1 wraps and is not.
    pub(crate) is_rare: bool,
}

impl NoticeCollectItem {
    /// `NoticeViewDataBase.NoticeDuration` the layer sets: 2.5 s for a new or
    /// rare item, 1.5 s otherwise.
    pub(crate) fn notice_duration(&self) -> f32 {
        if self.is_new || self.is_rare {
            2.5
        } else {
            1.5
        }
    }
}

impl NoticeData for NoticeCollectItem {
    fn notice_duration(&self) -> f32 {
        NoticeCollectItem::notice_duration(self)
    }
}

/// The count text's characters (`SetCount` prints an `Int32`).
const COUNT_CHARS: &str = "\u{00D7}-0123456789";
/// The instrument's environment variable.
const AUTOPLAY: &str = "MOLY_NOTICE_COLLECT_AUTOPLAY";
/// The instrument's wait after the cells are ready.
const AUTOPLAY_DELAY: f32 = 2.0;
const INPUTS: [&str; 3] = [
    "mysekai-materials.json",
    "mysekai-items.json",
    "mysekai-fixtures.json",
];

/// The characters a collect-item cell can print: the atlas's fifth member.
#[derive(Resource, Default)]
pub(crate) struct CollectNoticeCharset {
    pub(crate) chars: Vec<char>,
}

#[derive(Resource)]
struct Inputs(Vec<(&'static str, Handle<JsonAsset>)>);

/// A material or item master row.
#[derive(Clone, Debug)]
struct MasterRow {
    id: i64,
    name: String,
    icon: String,
    /// `MysekaiMaterialRarityType` as its enum value; None for an item.
    rarity: Option<i64>,
}

/// A fixture master row.
#[derive(Clone, Debug)]
struct FixtureRow {
    name: String,
    assetbundle: String,
    /// `MysekaiFixtureType.plant`.
    plant: bool,
}

/// The masters' rows the opener's names and icons come from, by id order.
#[derive(Resource, Default)]
struct Masters {
    materials: Vec<MasterRow>,
    items: Vec<MasterRow>,
    fixtures: HashMap<i64, FixtureRow>,
}

struct Bound {
    cells: Vec<CellBinding>,
    slides: SlideNotices<NoticeCollectItem>,
    anchored_x: f32,
    placed: bool,
    /// Each cell's `_newObject` state from its last `Setup`.
    new_active: [bool; PARALLEL],
    letters: [Vec<LetterTween>; PARALLEL],
    /// Pending `DelayCall`s of `SetupEffect`: cell, counted time, started
    /// this frame.
    rare_calls: Vec<(usize, f32, bool)>,
    registered: HashSet<String>,
    refused_icons: HashSet<String>,
}

#[derive(Resource, Default)]
struct CollectNotices {
    bound: Option<Bound>,
    failed: bool,
}

#[derive(Resource)]
struct Autoplay {
    count: Option<usize>,
    waited: f32,
    sent: bool,
}

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Inputs(
        INPUTS
            .iter()
            .map(|path| (*path, server.load::<JsonAsset>(format!("moly://{path}"))))
            .collect(),
    ));
    let count = crate::server::client::instrument_env(AUTOPLAY).map(|raw| {
        let raw = raw.trim().to_owned();
        raw.parse::<usize>().unwrap_or(usize::MAX)
    });
    if let Some(count) = count {
        info!("[notice] instrument {AUTOPLAY}: {} scripted NoticeCollectItem calls after the cells are ready",
            if count == usize::MAX { "all".to_owned() } else { count.to_string() });
    }
    commands.insert_resource(Autoplay {
        count,
        waited: 0.0,
        sent: false,
    });
}

/// The rows of an exported master table (keyed entries, or a table key).
fn rows<'a>(doc: &'a Value, table: &str) -> Vec<&'a Value> {
    match &doc["entries"] {
        Value::Object(entries) => entries.values().collect(),
        Value::Array(entries) => entries.iter().collect(),
        _ => doc[table]
            .as_array()
            .map(|rows| rows.iter().collect())
            .unwrap_or_default(),
    }
}

fn rarity_value(word: &str) -> Option<i64> {
    match word {
        "rarity_1" => Some(0),
        "rarity_2" => Some(1),
        "rarity_3" => Some(2),
        "rarity_4" => Some(3),
        _ => None,
    }
}

/// Once every input is loaded or failed: the charset and the instrument's
/// rows.
fn settle(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    inputs: Option<Res<Inputs>>,
) {
    let Some(inputs) = inputs else { return };
    let mut docs: HashMap<&str, Value> = HashMap::new();
    let mut absent = Vec::new();
    for (path, handle) in &inputs.0 {
        match server.load_state(handle) {
            LoadState::Loaded => {
                let Some(asset) = json.get(handle) else {
                    return;
                };
                match serde_json::from_str(&asset.0) {
                    Ok(value) => {
                        docs.insert(*path, value);
                    }
                    Err(error) => absent.push(format!("{path}: {error}")),
                }
            }
            LoadState::Failed(error) => absent.push(format!("{path}: {error}")),
            _ => return,
        }
    }
    let mut chars: Vec<char> = COUNT_CHARS.chars().collect();
    let mut masters = Masters::default();
    for (path, table) in [
        ("mysekai-materials.json", "mysekaiMaterials"),
        ("mysekai-items.json", "mysekaiItems"),
        ("mysekai-fixtures.json", "fixtures"),
    ] {
        let Some(doc) = docs.get(path) else { continue };
        let mut listed = rows(doc, table);
        listed.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
        for row in listed {
            let Some(name) = row["name"].as_str() else {
                continue;
            };
            chars.extend(name.chars());
            let Some(id) = row["id"].as_i64() else {
                continue;
            };
            if path == "mysekai-fixtures.json" {
                if let Some(assetbundle) = row["assetbundleName"].as_str() {
                    masters.fixtures.insert(
                        id,
                        FixtureRow {
                            name: name.to_owned(),
                            assetbundle: assetbundle.to_owned(),
                            plant: row["fixtureType"].as_str() == Some("plant"),
                        },
                    );
                }
                continue;
            }
            let icon = row["iconAssetbundleName"].as_str().map(str::to_owned);
            match (path, icon) {
                ("mysekai-materials.json", Some(icon)) => masters.materials.push(MasterRow {
                    id,
                    name: name.to_owned(),
                    icon,
                    rarity: row["mysekaiMaterialRarityType"]
                        .as_str()
                        .and_then(rarity_value),
                }),
                ("mysekai-items.json", Some(icon)) => masters.items.push(MasterRow {
                    id,
                    name: name.to_owned(),
                    icon,
                    rarity: None,
                }),
                _ => {}
            }
        }
    }
    chars.sort_unstable();
    chars.dedup();
    for line in &absent {
        warn!("[notice] collect-item input absent: {line}");
    }
    info!(
        "[notice] collect-item charset: {} characters; master rows: {} materials, {} items, {} fixtures",
        chars.len(),
        masters.materials.len(),
        masters.items.len(),
        masters.fixtures.len()
    );
    commands.insert_resource(CollectNoticeCharset { chars });
    commands.insert_resource(masters);
    commands.remove_resource::<Inputs>();
}

/// `Initialize`: the five cells composed into the notice document once it is
/// loaded, then bound.
fn compose(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut state: ResMut<CollectNotices>,
) {
    if state.bound.is_some() || state.failed {
        return;
    }
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let (composed, instances, anchored_x, height) = match cell::compose(doc) {
        Ok(result) => result,
        Err(error) => {
            error!("[notice] the collect-item cells cannot be composed: {error}; collect notices are not drawn");
            state.failed = true;
            return;
        }
    };
    let cells = match instances
        .iter()
        .enumerate()
        .map(|(i, instance)| cell::bind(&composed, instance, i, height))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(cells) => cells,
        Err(error) => {
            error!("[notice] a collect-item cell cannot be bound: {error}; collect notices are not drawn");
            state.failed = true;
            return;
        }
    };
    if layouts
        .replace_runtime_document(KEY, composed, &server)
        .is_err()
    {
        // The source documents are not all parsed yet: next frame.
        return;
    }
    info!(
        "[notice] SlideNoticeContoller.Initialize: {} cells {} at x {anchored_x}, y 0 to {}, {} new-mark letters each; start x {}",
        cells.len(),
        cells.iter().map(|c| c.root.as_str()).collect::<Vec<_>>().join(" "),
        cells.last().map_or(0.0, |c| c.y),
        cells.first().map_or(0, |c| c.letters.len()),
        anchored_x + slide::SLIDE_IN_LEFT
    );
    state.bound = Some(Bound {
        cells,
        slides: SlideNotices::new(anchored_x),
        anchored_x,
        placed: false,
        new_active: [false; PARALLEL],
        letters: Default::default(),
        rare_calls: Vec::new(),
        registered: HashSet::new(),
        refused_icons: HashSet::new(),
    });
}

/// The glyphs of `text` the atlas lacks.
fn missing_glyphs(art: &crate::balloon::BalloonArt, text: &str) -> Vec<char> {
    let mut missing: Vec<char> = text
        .chars()
        .filter(|ch| *ch != '\n' && art.advance(*ch).is_none())
        .collect();
    missing.sort_unstable();
    missing.dedup();
    missing
}

/// `HarvestCollectItemNoticeCell.Setup` on cell `index`.
#[allow(clippy::too_many_arguments)]
fn setup_cell(
    bound: &mut Bound,
    index: usize,
    slot: usize,
    dt: f32,
    view: &mut UiPrefabView,
    layouts: &mut UiLayouts,
    server: &AssetServer,
    art: Option<&crate::balloon::BalloonArt>,
    icons: Option<&crate::item_icon::ItemIcons>,
) {
    let data = bound
        .slides
        .slot(slot)
        .expect("a set-up slot holds data")
        .clone();
    let cell = bound.cells[index].clone();
    for (path, text) in [
        (&cell.name, data.item_name.clone()),
        (&cell.count, cell::count_text(data.get_count)),
    ] {
        match art.map(|art| missing_glyphs(art, &text)) {
            Some(missing) if missing.is_empty() => view.set_text(path, text),
            Some(missing) => {
                error!(
                    "[notice] collect-item cell {index}: {} characters outside the atlas (code points {:?}): the text at {path} is not drawn",
                    missing.len(),
                    missing.iter().map(|ch| *ch as u32).collect::<Vec<_>>()
                );
                view.set_text(path, String::new());
            }
            None => {
                error!("[notice] collect-item cell {index}: the text atlas is not baked: the text at {path} is not drawn");
                view.set_text(path, String::new());
            }
        }
    }
    view.set_visible(&cell.new_mark, data.is_new);
    bound.new_active[index] = data.is_new;
    view.set_visible(&cell.limit, data.is_limit);
    view.set_visible(&cell.name, !data.is_limit);
    view.set_visible(&cell.rare, false);
    if data.is_rare {
        // The coroutine's first count runs in the calling frame.
        bound.rare_calls.push((index, dt, true));
    }
    view.set_visible(&cell.loading, false);
    let key = data.asset_bundle_name.clone();
    let found = match icons {
        Some(icons) => icons.texture(&key, &data.resource_name),
        None => Err("the icon index is not settled yet".to_owned()),
    };
    match found {
        Ok(asset) => {
            if bound.registered.insert(key.clone()) {
                layouts.register_runtime_texture(&key, &asset, server);
            }
            view.set_texture(&cell.icon, &key);
            view.set_visible(&cell.icon, true);
            info!(
                "[notice] collect-item cell {index} icon {}: {}",
                crate::balloon::ascii_or(&key),
                crate::balloon::ascii_or(&asset)
            );
        }
        Err(reason) => {
            if bound.refused_icons.insert(key.clone()) {
                error!(
                    "[notice] collect-item icon {} / {} refused ({reason}): the cell's icon is not drawn",
                    crate::balloon::ascii_or(&key),
                    crate::balloon::ascii_or(&data.resource_name)
                );
            }
            view.set_visible(&cell.icon, false);
        }
    }
}

/// The calls, the slide controller's frame, the rare delays and the new-mark
/// letters, drawn into the notice view.
#[allow(clippy::too_many_arguments)]
fn advance(
    mut calls: MessageReader<NoticeCollectItem>,
    time: Res<Time>,
    screens: Res<crate::ui_layers::ScreenManager>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    art: Option<Res<crate::balloon::BalloonArt>>,
    icons: Option<Res<crate::item_icon::ItemIcons>>,
    mut state: ResMut<CollectNotices>,
    mut views: Query<&mut UiPrefabView, With<NoticeRoot>>,
) {
    let incoming: Vec<NoticeCollectItem> = calls.read().cloned().collect();
    let layer = crate::ui_layers::MenuScreenType::MysekaiNotice;
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let describe = |call: &NoticeCollectItem| {
        format!(
            "NoticeCollectItem(bundle {}, resource {}, count {}, limit {}, new {}, rare {})",
            crate::balloon::ascii_or(&call.asset_bundle_name),
            crate::balloon::ascii_or(&call.resource_name),
            call.get_count,
            call.is_limit,
            call.is_new,
            call.is_rare
        )
    };
    let Some(bound) = state.bound.as_mut() else {
        for call in &incoming {
            error!(
                "[notice] {} before the collect-item cells are composed: not drawn",
                describe(call)
            );
        }
        return;
    };
    let Ok(mut view) = views.single_mut() else {
        for call in &incoming {
            error!(
                "[notice] {} before the notice view exists: not drawn",
                describe(call)
            );
        }
        return;
    };
    if !bound.placed {
        // `Initialize`: each cell asleep at its row.
        for cell in &bound.cells {
            view.set_visible(&cell.root, false);
            view.set_anchored_position(&cell.root, Vec2::new(bound.anchored_x, cell.y));
        }
        bound.placed = true;
    }
    let mut cell_calls: Vec<(usize, CellCall)> = Vec::new();
    for call in incoming {
        if !screens.is_active(layer) {
            error!("[notice] {}: GetLayerComponent<ScreenLayerMysekaiNotice> finds no active {layer:?} layer: not drawn", describe(&call));
            continue;
        }
        let text = describe(&call);
        let duration = call.notice_duration();
        let (slot, driver) = bound.slides.notice(call, dt, &mut cell_calls);
        info!(
            "[notice] t {now:.4}: {text}: slot {slot}, duration {duration} s, driver {driver}, queues {:?}",
            bound.slides.queue_lengths()
        );
    }
    bound.slides.step(dt, &mut cell_calls);
    // The coroutines: `WaitForSeconds` counts while below the delay.
    let mut fire = Vec::new();
    bound.rare_calls.retain_mut(|(index, counted, fresh)| {
        if *fresh {
            *fresh = false;
            return true;
        }
        if *counted < RARE_DELAY {
            *counted += dt;
            true
        } else {
            fire.push(*index);
            false
        }
    });
    for (index, call) in cell_calls {
        match call {
            CellCall::Setup { slot } => {
                setup_cell(
                    bound,
                    index,
                    slot,
                    dt,
                    &mut view,
                    &mut layouts,
                    &server,
                    art.as_deref(),
                    icons.as_deref(),
                );
            }
            CellCall::Wakeup => {
                let cell = &bound.cells[index];
                view.set_visible(&cell.root, true);
                bound.letters[index] = if bound.new_active[index] {
                    cell.letters
                        .iter()
                        .map(|letter| LetterTween::play(letter.law.clone()))
                        .collect()
                } else {
                    Vec::new()
                };
                for (letter, tween) in cell.letters.iter().zip(&bound.letters[index]) {
                    view.set_anchored_position(
                        &letter.selector,
                        cell::letter_position(tween.value()),
                    );
                }
                info!(
                    "[notice] t {now:.4}: cell {index} Wakeup, Open{}",
                    if bound.new_active[index] {
                        ", new mark letters play"
                    } else {
                        ""
                    }
                );
            }
            CellCall::Sleep => {
                view.set_visible(&bound.cells[index].root, false);
                bound.letters[index].clear();
                info!("[notice] t {now:.4}: cell {index} Close done, Sleep");
            }
            CellCall::X(x) => {
                let cell = &bound.cells[index];
                view.set_anchored_position(&cell.root, Vec2::new(x, cell.y));
            }
        }
    }
    for index in fire {
        view.set_visible(&bound.cells[index].rare, true);
        info!("[notice] t {now:.4}: cell {index} rare effect active (its UIParticle is not decoded: nothing is drawn under it)");
    }
    for (index, tweens) in bound.letters.iter_mut().enumerate() {
        for (letter, tween) in bound.cells[index].letters.iter().zip(tweens.iter_mut()) {
            if tween.update(dt) {
                view.set_anchored_position(&letter.selector, cell::letter_position(tween.value()));
            }
        }
    }
}

/// `MysekaiResourceUtility.NoticeCollectItem`'s name and icon arms, for the
/// harvest opener's notice (which carries the id and the flags):
/// - `mysekai_material`: the row's `name`, `AssetBundleNames.
///   GetMysekaiMaterialPreview(iconAssetbundleName)` and the icon name;
/// - `mysekai_item`: the row's `name`, `GetMysekaiItemPreview(
///   iconAssetbundleName)` and the icon name;
/// - `mysekai_fixture`: the row's `name`; a plant fixture takes
///   `GetMysekaiFixturePlantPreviewImage(assetbundleName, id)` with the
///   resource `{assetbundleName}_{id}`, any other fixture
///   `GetMysekaiFixtureThumbnail(assetbundleName)` with the resource
///   `assetbundleName`;
/// - any other type: an empty name and no icon names.
fn resolve(
    masters: &Masters,
    notice: &crate::harvest::notice::CollectNotice,
) -> Result<NoticeCollectItem, String> {
    use crate::harvest::notice::NoticeArm;
    let id = notice.resource_id;
    let (item_name, asset_bundle_name, resource_name) = match notice.arm {
        NoticeArm::MysekaiMaterial => {
            let row = masters
                .materials
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("mysekai material {id} has no master row here"))?;
            (
                row.name.clone(),
                format!("mysekai/item_preview/material/{}", row.icon),
                row.icon.clone(),
            )
        }
        NoticeArm::MysekaiItem => {
            let row = masters
                .items
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("mysekai item {id} has no master row here"))?;
            (
                row.name.clone(),
                format!("mysekai/item_preview/item/{}", row.icon),
                row.icon.clone(),
            )
        }
        NoticeArm::MysekaiFixture => {
            let row = masters
                .fixtures
                .get(&id)
                .ok_or_else(|| format!("mysekai fixture {id} has no master row here"))?;
            if row.plant {
                let resource = format!("{}_{id}", row.assetbundle);
                (
                    row.name.clone(),
                    format!("mysekai/item_preview/fixture/{resource}"),
                    resource,
                )
            } else {
                (
                    row.name.clone(),
                    format!("mysekai/thumbnail/fixture/{}", row.assetbundle),
                    row.assetbundle.clone(),
                )
            }
        }
        NoticeArm::Other => (String::new(), String::new(), String::new()),
    };
    Ok(NoticeCollectItem {
        item_name,
        asset_bundle_name,
        resource_name,
        get_count: notice.get_count,
        is_limit: notice.is_limit,
        is_new: notice.is_new,
        is_rare: notice.is_rare,
    })
}

/// The harvest opener's notices, resolved and handed to the notice layer.
fn bridge(
    mut raised: MessageReader<crate::harvest::notice::CollectNotice>,
    masters: Option<Res<Masters>>,
    mut writer: MessageWriter<NoticeCollectItem>,
) {
    for notice in raised.read() {
        let resolved = match masters.as_deref() {
            Some(masters) => resolve(masters, notice),
            None => Err("the masters are not loaded yet".to_owned()),
        };
        match resolved {
            Ok(call) => {
                writer.write(call);
            }
            Err(reason) => error!(
                "[notice] harvest notice {:?} {} x{}: {reason}: not drawn",
                notice.arm, notice.resource_id, notice.get_count
            ),
        }
    }
}

/// The opener's rare rule for a material: `rarity - 1 < 3` compared
/// unsigned on the `MysekaiMaterialRarityType` value.
fn material_is_rare(rarity: i64) -> bool {
    ((rarity as i32).wrapping_sub(1) as u32) < 3
}

/// The instrument's script: plain, rare, new, limited, rare and new, an
/// item, two more plain.
fn script(masters: &Masters) -> Vec<NoticeCollectItem> {
    let material = |row: &MasterRow, count: i32, limit: bool, new: bool| NoticeCollectItem {
        item_name: row.name.clone(),
        asset_bundle_name: format!("mysekai/item_preview/material/{}", row.icon),
        resource_name: row.icon.clone(),
        get_count: count,
        is_limit: limit,
        is_new: new,
        is_rare: row.rarity.is_some_and(material_is_rare),
    };
    let common: Vec<&MasterRow> = masters
        .materials
        .iter()
        .filter(|row| row.rarity == Some(0))
        .collect();
    let rare: Vec<&MasterRow> = masters
        .materials
        .iter()
        .filter(|row| row.rarity.is_some_and(|r| r >= 1))
        .collect();
    let mut out = Vec::new();
    if let Some(row) = common.first() {
        out.push(material(*row, 3, false, false));
    }
    if let Some(row) = rare.first() {
        out.push(material(*row, 1, false, false));
    }
    if let Some(row) = common.get(1) {
        out.push(material(*row, 2, false, true));
    }
    if let Some(row) = common.get(2) {
        out.push(material(*row, 5, true, false));
    }
    if let Some(row) = rare.last() {
        out.push(material(*row, 1, false, true));
    }
    if let Some(row) = masters.items.first() {
        // The item arm of `MysekaiResourceUtility.NoticeCollectItem`: never
        // new or limited, always rare.
        out.push(NoticeCollectItem {
            item_name: row.name.clone(),
            asset_bundle_name: format!("mysekai/item_preview/item/{}", row.icon),
            resource_name: row.icon.clone(),
            get_count: 1,
            is_limit: false,
            is_new: false,
            is_rare: true,
        });
    }
    for row in common.iter().skip(3).take(2) {
        out.push(material(*row, 1, false, false));
    }
    out
}

fn autoplay(
    time: Res<Time>,
    mut plan: ResMut<Autoplay>,
    masters: Option<Res<Masters>>,
    state: Res<CollectNotices>,
    screens: Res<crate::ui_layers::ScreenManager>,
    views: Query<(), With<NoticeRoot>>,
    mut writer: MessageWriter<NoticeCollectItem>,
) {
    let Some(count) = plan.count else { return };
    if plan.sent {
        return;
    }
    let (Some(masters), Some(_)) = (masters, state.bound.as_ref()) else {
        return;
    };
    if views.is_empty() || !screens.is_active(crate::ui_layers::MenuScreenType::MysekaiNotice) {
        return;
    }
    plan.waited += time.delta_secs();
    if plan.waited < AUTOPLAY_DELAY {
        return;
    }
    plan.sent = true;
    let calls: Vec<_> = script(&masters).into_iter().take(count).collect();
    info!(
        "[notice] instrument {AUTOPLAY}: sending {} NoticeCollectItem calls",
        calls.len()
    );
    for call in calls {
        writer.write(call);
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_message::<NoticeCollectItem>()
        .init_resource::<CollectNotices>()
        .add_systems(Startup, load)
        .add_systems(Update, (settle, compose, bridge, autoplay, advance).chain());
}
