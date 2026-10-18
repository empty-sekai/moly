//! The BGM select screen's two dialogs: the change-vocal sub-window
//! (`MysekaiBGMSelectChangeVocalDialog`, 320) and the BGM filter
//! (`BGMFilterDialog`, 398).
//!
//! Both are `UiPrefabView`s shown through `ScreenManager`, so `moly.ui.views`
//! lists them and `moly.ui.tap` presses their controls. Their source
//! documents are not in the shared UI root's request list, so they are loaded
//! from the same region root the screen draws from and installed as runtime
//! documents.
//!
//! Change-vocal data: the record's `musicVocals`, ordered by `Seq`; a
//! `streaming_live` vocal and a special-season vocal need data this client
//! does not carry (named gaps at the bottom). Selecting a vocal remembers it
//! per record, updates the record model, and sends
//! `PutUserMysekaiMusicPlaySetApi` when the record is the chosen one — the
//! source's `OnChangedVocal`.
//!
//! Filter data: `BGMFilterData` from the local settings document (the
//! `ApplicationLocalSettings` member), with its three serialized keys. The
//! dialog's two music toggles (`None`, `NewlyWrittenMusic`) and the character
//! checkboxes mirror the prefab's own controls; OK saves the data and the
//! screen re-filters its list.

use std::collections::{BTreeSet, HashMap};

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::master::{self, MasterData, MasterError, MasterTable};
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiNode, UiPrefab};
use moly_link::ServerLink;
use moly_protocol::api::MusicPlaySetRequest;
use moly_protocol::Call;
use serde_json::{json, Value};

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::ServerPending;

use super::model::{self, BgmMasters, Presenter, RecordModel, VocalRow, VocalWordings};
use super::{words, BgmSelect};

/// Runtime document keys for the three source prefabs.
const CHANGE_VOCAL: &str = "BgmChangeVocal";
const CHANGE_VOCAL_CELL: &str = "BgmChangeVocalCell";
const FILTER: &str = "BgmFilter";
/// The characters' icons the dialog's checkboxes show (`UIPartsCharacterIcon`)
/// are UI sprites like the player view's images (the icon sprite the root's
/// runtime sprite inventory lists by `CharacterIconInfo`): their extracted
/// sprites are registered with the screen's own, and their authored metrics
/// make the Image rules draw them instead of the raw fallback.

/// Where the source documents sit under the region root.
const CHANGE_VOCAL_PATH: &str = "bgmselect/MysekaiBGMSelectChangeVocalDialog.json";
const CHANGE_VOCAL_CELL_PATH: &str = "bgmselect/MysekaiBGMSelectChangeVocalCell.json";
const FILTER_PATH: &str = "bgmselect/BGMFilterDialog.json";

/// The local settings key carrying `ApplicationLocalSettings`'s
/// `BGMFilterData`.
const FILTER_SETTINGS_KEY: &str = "BGMFilterData";

/// The cell prefab's authored height (`sizeDelta.y`).
const CELL_HEIGHT: f32 = 164.0;

// ---------------------------------------------------------------------------
// The filter data
// ---------------------------------------------------------------------------

/// `BGMFilterData`: the unchecked character ids, the music filter and the
/// outside-character flag. Defaults are the source constructor's:
/// `VirtualSingerOutSide = true`, no unchecked ids, `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BgmFilter {
    pub(crate) unchecked: BTreeSet<i32>,
    pub(crate) music: i32,
    pub(crate) other: bool,
}

impl Default for BgmFilter {
    fn default() -> Self {
        Self {
            unchecked: BTreeSet::new(),
            music: 0,
            other: true,
        }
    }
}

impl BgmFilter {
    /// `get_IsFiltered`.
    pub(crate) fn is_filtered(&self) -> bool {
        !self.unchecked.is_empty() || self.music != 0 || !self.other
    }

    fn checked(&self, character_id: i32) -> bool {
        !self.unchecked.contains(&character_id)
    }

    fn set_checked(&mut self, character_id: i32, checked: bool) {
        if checked {
            self.unchecked.remove(&character_id);
        } else {
            self.unchecked.insert(character_id);
        }
    }

    fn parse(value: &Value) -> Self {
        let mut filter = Self::default();
        if let Some(ids) = value["UnCheckedCharacterIds"].as_array() {
            for id in ids.iter().filter_map(Value::as_i64) {
                filter.unchecked.insert(id as i32);
            }
        }
        // MusicSelectMusicFilterType: None 0, NewlyWrittenMusic 1.
        filter.music = value["MusicSelectMusicFilterType"]
            .as_i64()
            .unwrap_or(0)
            .clamp(0, 1) as i32;
        if let Some(other) = value["VirtualSingerOutSide"].as_bool() {
            filter.other = other;
        }
        filter
    }

    fn value(&self) -> Value {
        json!({
            "UnCheckedCharacterIds": self.unchecked.iter().copied().collect::<Vec<_>>(),
            "MusicSelectMusicFilterType": self.music,
            "VirtualSingerOutSide": self.other,
        })
    }
}

fn read_filter() -> BgmFilter {
    match crate::settings_store::read_document() {
        Ok(document) => document
            .get(FILTER_SETTINGS_KEY)
            .map(BgmFilter::parse)
            .unwrap_or_default(),
        Err(error) => {
            warn!("[bgm-select] the local settings could not be read ({error}); the filter starts at its defaults");
            BgmFilter::default()
        }
    }
}

fn save_filter(filter: &BgmFilter) {
    if let Err(error) = crate::settings_store::save_sections(&[(FILTER_SETTINGS_KEY, filter.value())])
    {
        error!("[bgm-select] the filter could not be saved ({error})");
    }
}

// ---------------------------------------------------------------------------
// Detail master tables (fields the screen's model does not keep)
// ---------------------------------------------------------------------------

/// One `musicVocals` row: the cell's caption, release condition, special
/// season and the characters `IsCharacterExist` reads.
#[derive(Clone, Debug)]
pub(crate) struct VocalDetail {
    pub(crate) id: i32,
    pub(crate) music_id: i32,
    pub(crate) vocal_type: String,
    pub(crate) seq: i32,
    pub(crate) release_condition_id: i32,
    pub(crate) caption: String,
    pub(crate) special_season_id: i32,
    pub(crate) characters: Vec<(String, i32, i32)>,
}

fn parse_vocal_details(text: &str) -> Result<HashMap<i32, Vec<VocalDetail>>, String> {
    let rows = master::read_rows(text, |row| {
        let mut characters = Vec::new();
        for character in row["characters"].as_array().into_iter().flatten() {
            characters.push((
                master::text(character, "characterType")?.to_owned(),
                master::int32(character, "characterId")?,
                master::int32(character, "seq")?,
            ));
        }
        let vocal = VocalDetail {
            id: master::int32(row, "id")?,
            music_id: master::int32(row, "musicId")?,
            vocal_type: master::text(row, "musicVocalType")?.to_owned(),
            seq: master::int32(row, "seq")?,
            release_condition_id: master::int32(row, "releaseConditionId").unwrap_or(0),
            caption: row["caption"].as_str().unwrap_or_default().to_owned(),
            special_season_id: master::int32(row, "specialSeasonId").unwrap_or(0),
            characters,
        };
        Ok(Some((vocal.music_id, vocal)))
    })?;
    let mut by_music: HashMap<i32, Vec<VocalDetail>> = HashMap::new();
    for (music, vocal) in rows {
        by_music.entry(music).or_default().push(vocal);
    }
    for vocals in by_music.values_mut() {
        vocals.sort_by_key(|vocal| (vocal.seq, vocal.id));
    }
    Ok(by_music)
}

/// `musics.isNewlyWrittenMusic`, by music id.
fn parse_music_details(text: &str) -> Result<HashMap<i32, bool>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            row["isNewlyWrittenMusic"].as_bool().unwrap_or(false),
        )))
    })?;
    Ok(rows.into_iter().collect())
}

const VOCAL_DETAILS: MasterTable<HashMap<i32, Vec<VocalDetail>>> = MasterTable {
    table: "musicVocals",
    name: "musicVocals (the BGM select change-vocal dialog)",
    parse: parse_vocal_details,
};
const MUSIC_DETAILS: MasterTable<HashMap<i32, bool>> = MasterTable {
    table: "musics",
    name: "musics (the BGM select filter)",
    parse: parse_music_details,
};

/// The two detail tables, once they resolve.
#[derive(Resource, Default)]
pub(crate) struct Details {
    vocals: Option<Result<HashMap<i32, Vec<VocalDetail>>, MasterError>>,
    musics: Option<Result<HashMap<i32, bool>, MasterError>>,
    ready: bool,
    warned: bool,
}

impl Details {
    fn poll(&mut self, data: &mut MasterData) {
        if self.ready {
            return;
        }
        if self.vocals.is_none() {
            self.vocals = data.take(&VOCAL_DETAILS);
        }
        if self.musics.is_none() {
            self.musics = data.take(&MUSIC_DETAILS);
        }
        self.ready = self.vocals.is_some() && self.musics.is_some();
    }

    fn vocals(&self) -> Option<&HashMap<i32, Vec<VocalDetail>>> {
        self.vocals.as_ref().and_then(|result| result.as_ref().ok())
    }

    fn music(&self) -> Option<&HashMap<i32, bool>> {
        self.musics.as_ref().and_then(|result| result.as_ref().ok())
    }
}

fn request_details(mut data: ResMut<MasterData>) {
    data.request(&VOCAL_DETAILS);
    data.request(&MUSIC_DETAILS);
}

fn poll_details(mut details: ResMut<Details>, mut data: ResMut<MasterData>) {
    details.poll(&mut data);
    if details.ready && !details.warned {
        if let Some(Err(error)) = &details.vocals {
            let reason = error.to_string();
            details.warned = true;
            error!("[bgm-select] the change-vocal detail table is absent ({reason}); the dialog refuses when opened");
        }
    }
}

// ---------------------------------------------------------------------------
// The filter's effect on the record list
// ---------------------------------------------------------------------------

/// `CreateDisplayMusicData` for tab ALL and sort Default, with the filter
/// dialog's predicate (`IsFilterDialogExist`, `IsCharacterExist`).
pub(crate) fn display(
    presenter: &Presenter,
    filter: &BgmFilter,
    details: Option<&Details>,
) -> Vec<usize> {
    if !filter.is_filtered() {
        return presenter.display.clone();
    }
    presenter
        .display
        .iter()
        .copied()
        .filter(|&index| passes(&presenter.all[index], filter, details))
        .collect()
}

fn passes(record: &RecordModel, filter: &BgmFilter, details: Option<&Details>) -> bool {
    // Any active filter hides the soundtracks, and the character filter
    // never keeps one.
    if record.soundtrack {
        return false;
    }
    if filter.music == 1 {
        let Some(newly) = details
            .and_then(|details| details.music())
            .and_then(|musics| record.music_id.and_then(|id| musics.get(&id).copied()))
        else {
            return false;
        };
        if !newly {
            return false;
        }
    }
    let Some(details) = details else { return false };
    let Some(music_id) = record.music_id else { return false };
    let Some(vocals) = details.vocals().and_then(|vocals| vocals.get(&music_id)) else {
        return false;
    };
    // `<IsCharacterExist>b__23_0`: only the ordinary vocals.
    let mut characters: Vec<(String, i32, i32)> = vocals
        .iter()
        .filter(|vocal| vocal.special_season_id == 0)
        .flat_map(|vocal| vocal.characters.iter().cloned())
        .collect();
    // `VirtualSingerOutSide` (the dialog's other checkbox): a record with an
    // outside character passes regardless of the unchecked ids.
    if filter.other && characters.iter().any(|(kind, _, _)| kind == "outside_character") {
        return true;
    }
    if filter.unchecked.is_empty() {
        return true;
    }
    characters.sort_by_key(|(_, id, _)| *id);
    characters.dedup_by_key(|(_, id, _)| *id);
    let game_ids: BTreeSet<i32> = characters
        .iter()
        .filter(|(kind, _, _)| kind == "game_character")
        .map(|(_, id, _)| *id)
        .collect();
    // `!All(id => UnCheckedCharacterIds.Contains(id))`: at least one game
    // character must be checked.
    !game_ids.iter().all(|id| filter.unchecked.contains(id))
}

// ---------------------------------------------------------------------------
// Document loading
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DocState {
    Loading,
    Installed,
    Refused,
}

struct FilterBindings {
    none: String,
    none_check: String,
    newly: String,
    newly_check: String,
    all: String,
    all_check: String,
    units: Vec<(String, String, Vec<i32>)>,
    characters: Vec<(String, String, i32)>,
    other: String,
    other_check: String,
    positive: String,
    negative: String,
    close: String,
    window: String,
}

#[derive(Resource, Default)]
struct Documents {
    source: Option<Handle<JsonAsset>>,
    root: Option<String>,
    change: Option<Handle<JsonAsset>>,
    cell: Option<Handle<JsonAsset>>,
    filter: Option<Handle<JsonAsset>>,
    state: DocState,
    filter_bindings: Option<FilterBindings>,
}

impl Default for DocState {
    fn default() -> Self {
        DocState::Loading
    }
}

fn load_documents(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Documents {
        source: Some(server.load("moly://source.json")),
        ..Default::default()
    });
}

fn refuse(docs: &mut Documents, reason: String) {
    error!("[bgm-select] the BGM dialogs refuse by name: {reason}");
    docs.state = DocState::Refused;
}

fn install_documents(
    mut commands: Commands,
    mut docs: ResMut<Documents>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut layouts: ResMut<UiLayouts>,
) {
    if docs.state != DocState::Loading {
        return;
    }
    // Wait for the screen's own UI source set: the runtime documents are
    // installed through the same registry.
    if layouts.document(super::SCREEN_DOC).is_none() {
        return;
    }
    // 1. The region root, as `ui_layout` chooses it: a region with its own
    // root is `ui-jp`; every other region reads the shared donor root.
    if docs.root.is_none() {
        let handle = docs.source.clone().expect("the dialog loader's source handle");
        if let LoadState::Failed(error) = server.load_state(&handle) {
            refuse(&mut docs, format!("source.json did not load ({error})"));
            return;
        }
        let Some(asset) = json.get(&handle) else { return };
        let value: Value = serde_json::from_str(&asset.0)
            .unwrap_or_else(|e| panic!("[bgm-select] source.json: {e}"));
        let region = value["source"]["region"].as_str().unwrap_or_default();
        let root = if region == "jp" {
            "moly://ui-jp/"
        } else {
            "moly://ui-layout-v2/"
        };
        docs.root = Some(root.to_owned());
        docs.change = Some(server.load(format!("{root}{CHANGE_VOCAL_PATH}")));
        docs.cell = Some(server.load(format!("{root}{CHANGE_VOCAL_CELL_PATH}")));
        docs.filter = Some(server.load(format!("{root}{FILTER_PATH}")));
    }
    let root = docs.root.clone().expect("the dialog loader's root");
    // 2. All three documents, or a named refusal.
    for (handle, path) in [
        (docs.change.clone(), CHANGE_VOCAL_PATH),
        (docs.cell.clone(), CHANGE_VOCAL_CELL_PATH),
        (docs.filter.clone(), FILTER_PATH),
    ] {
        let handle = handle.expect("the dialog loader's document handle");
        if let LoadState::Failed(error) = server.load_state(&handle) {
            refuse(&mut docs, format!("{root}{path} did not load ({error})"));
            return;
        }
        if json.get(&handle).is_none() {
            return;
        }
    }
    // 3. Parse, rebase the images and install them as runtime documents.
    let parse = |name: &str, handle: &Handle<JsonAsset>| -> UiPrefab {
        let asset = json.get(handle).expect("the dialog document loaded");
        let mut doc =
            UiPrefab::parse(&asset.0).unwrap_or_else(|e| panic!("[bgm-select] {name}: {e}"));
        doc.rebase_images(&root);
        doc
    };
    let change = parse("change-vocal dialog", docs.change.as_ref().unwrap());
    let cell = parse("change-vocal cell", docs.cell.as_ref().unwrap());
    let filter = parse("filter dialog", docs.filter.as_ref().unwrap());
    let bindings = match filter_bindings(&filter) {
        Ok(bindings) => bindings,
        Err(reason) => {
            refuse(&mut docs, format!("the filter dialog is not composed: {reason}"));
            return;
        }
    };
    for (key, doc) in [(CHANGE_VOCAL, change), (CHANGE_VOCAL_CELL, cell), (FILTER, filter)] {
        if let Err(reason) = layouts.replace_runtime_document(key, doc, &server) {
            info!("[bgm-select] {key} waits for the UI sources: {reason}");
            return;
        }
    }
    commands.spawn((
        FilterView,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 200.),
        RenderLayers::layer(SITEMAP_LAYER),
        UiPrefabView::new(FILTER, SITEMAP_LAYER),
    ));
    docs.filter_bindings = Some(bindings);
    docs.state = DocState::Installed;
    info!("[bgm-select] change-vocal and filter dialogs installed from {root}");
}

// ---------------------------------------------------------------------------
// Bindings
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

/// One instantiated vocal cell's selectors.
struct VocalCell {
    root: String,
    caption: String,
    vocal_text: String,
    locked: String,
    selected: String,
    button: String,
}

fn change_vocal_bindings(
    cell_doc: &UiPrefab,
    instances: &[UiInstance],
) -> Result<Vec<VocalCell>, String> {
    let cell = component(cell_doc, &cell_doc.prefab, ".MysekaiBGMSelectChangeVocalListCell")?
        .fields
        .clone();
    let mut cells = Vec::new();
    for instance in instances {
        cells.push(VocalCell {
            root: instance.root_selector(),
            caption: instance.selector(pointer(&cell["_captionText"])?)?,
            vocal_text: instance.selector(pointer(&cell["_vocalText"])?)?,
            locked: instance.selector(pointer(&cell["_lockedObject"])?)?,
            selected: instance.selector(pointer(&cell["_selectedObject"])?)?,
            button: instance.selector(pointer(&cell["_selectedbutton"])?)?,
        });
    }
    Ok(cells)
}

fn change_vocal_content(doc: &UiPrefab) -> Result<String, String> {
    let fields = component(doc, &doc.prefab, ".MysekaiBGMSelectChangeVocalDialog")?
        .fields
        .clone();
    let list = field(&fields, "_listView")?;
    let index = doc.find(&list)?;
    Ok(format!("{}/Viewport/Content", doc.nodes[index].path))
}

struct ChangeVocalClose {
    /// `CloseArea`, behind the window: closes only outside it.
    area: Vec<String>,
    /// `UIPartsCloseButton`, inside the window: always closes.
    button: Vec<String>,
    window: String,
}

fn change_vocal_close(doc: &UiPrefab) -> Result<ChangeVocalClose, String> {
    let node = doc
        .nodes
        .iter()
        .find(|node| {
            node.components
                .iter()
                .any(|component| component.class.ends_with(".SubWindowComponent"))
        })
        .ok_or("the change-vocal dialog has no SubWindowComponent")?;
    let component = node
        .components
        .iter()
        .find(|component| component.class.ends_with(".SubWindowComponent"))
        .expect("the SubWindowComponent found");
    let buttons = component.fields["closeButtons"]
        .as_array()
        .ok_or("SubWindowComponent has no closeButtons")?;
    let window = format!("{}/Window", node.path);
    let mut area = Vec::new();
    let mut button = Vec::new();
    for close in buttons {
        let Ok(id) = pointer(close) else { continue };
        if id == 0 {
            continue;
        }
        let selector = format!("@{id}");
        let index = doc.find(&selector)?;
        if doc.nodes[index].path.contains("/UIPartsCloseButton") {
            button.push(selector);
        } else {
            area.push(selector);
        }
    }
    Ok(ChangeVocalClose { area, button, window })
}

fn check_image(node: &UiNode) -> Result<String, String> {
    let toggle = node
        .components
        .iter()
        .find(|c| c.class.ends_with(".CustomToggle"))
        .ok_or_else(|| format!("{} is not a toggle", node.path))?;
    match pointer(&toggle.fields["graphic"])? {
        0 => Err(format!("{} has no check graphic", node.path)),
        id => Ok(format!("@{id}")),
    }
}

fn character_id(node: &UiNode) -> Option<i32> {
    node.components
        .iter()
        .find(|c| c.class.ends_with(".UIPartsCharacterIcon"))
        .and_then(|c| c.fields["characterId"].as_i64())
        .map(|id| id as i32)
}

fn by_path<'a>(doc: &'a UiPrefab, path: &str) -> Result<&'a UiNode, String> {
    let index = doc.find(path)?;
    Ok(&doc.nodes[index])
}

fn filter_bindings(doc: &UiPrefab) -> Result<FilterBindings, String> {
    let radio = "BGMFilterDialog/WindowRoot/ContentRoot/Content/UIPartsGroupVerticalListView/ListView/Viewport/Content/UIPartsGroupFilterTextRadio";
    let content = "BGMFilterDialog/WindowRoot/ContentRoot/Content/UIPartsGroupVerticalListView/ListView/Viewport/Content/CharacterFilterContent";
    let none = by_path(doc, &format!("{radio}/None"))?;
    let newly = by_path(doc, &format!("{radio}/NewlyWrittenMusic"))?;
    let all = by_path(doc, &format!("{content}/AllMember/UIPartsCheckBox"))?;
    let other = by_path(doc, &format!("{content}/VS/CheckButtonRoot/Other/UIPartsCheckBox"))?;

    let mut units = Vec::new();
    let mut characters = Vec::new();
    for group in ["VS", "In", "MJ", "VB", "WS", "25"] {
        let prefix = format!("{content}/{group}");
        let unit = by_path(doc, &format!("{prefix}/UIPartsCheckBox"))?;
        let mut ids = Vec::new();
        for node in &doc.nodes {
            // `Other` sits among the VS character toggles but carries no
            // character icon; it is bound on its own above.
            if !node.path.starts_with(&format!("{prefix}/CheckButtonRoot/"))
                || !node.path.ends_with("/UIPartsCheckBox")
                || node.path == other.path
            {
                continue;
            }
            let parent = node.path.trim_end_matches("/UIPartsCheckBox");
            let icon = by_path(doc, &format!("{parent}/UIPartsCharacterIcon"))?;
            let id = character_id(icon).ok_or_else(|| format!("{} has no characterId", icon.path))?;
            ids.push(id);
            characters.push((node.path.clone(), check_image(node)?, id));
        }
        units.push((unit.path.clone(), check_image(unit)?, ids));
    }

    let footer = "BGMFilterDialog/WindowRoot/FooterButtons";
    let button_path = format!("{footer}/UIPartsCommonButton");
    let mut negative = None;
    let mut positive = None;
    for node in &doc.nodes {
        if node.path != button_path {
            continue;
        }
        let text = doc
            .nodes
            .iter()
            .find(|text| {
                text.parent_transform_id == node.transform_id && text.path.ends_with("/Text")
            })
            .and_then(|text| {
                text.components
                    .iter()
                    .find(|c| c.class.ends_with(".CustomTextMesh"))
                    .and_then(|c| c.fields["m_text"].as_str())
            })
            .unwrap_or_default();
        let selector = format!("@{}", node.transform_id);
        if text.contains("キャンセル") {
            negative = Some(selector);
        } else {
            positive = Some(selector);
        }
    }
    let close = by_path(doc, "BGMFilterDialog/WindowRoot/UIPartsCloseButton")?;
    Ok(FilterBindings {
        none: none.path.clone(),
        none_check: format!("{}/UIPartsRadioToggle/Radio/Checkmark", none.path),
        newly: newly.path.clone(),
        newly_check: format!("{}/UIPartsRadioToggle/Radio/Checkmark", newly.path),
        all: all.path.clone(),
        all_check: check_image(all)?,
        units,
        characters,
        other: other.path.clone(),
        other_check: check_image(other)?,
        positive: positive.ok_or("the filter footer has no confirm button")?,
        negative: negative.ok_or("the filter footer has no cancel button")?,
        close: close.path.clone(),
        window: "BGMFilterDialog/WindowRoot".to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Host state
// ---------------------------------------------------------------------------

/// What the screen asked for on its last click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogRequest {
    ChangeVocal(i32),
    Filter,
}

struct ChangeVocalRow {
    vocal_id: i32,
    caption: String,
    text: String,
}

struct OpenChangeVocal {
    id: DialogId,
    entity: Entity,
    record: i32,
    cells: Vec<VocalCell>,
    rows: Vec<ChangeVocalRow>,
    selected: i32,
    close: ChangeVocalClose,
}

#[derive(Resource, Default)]
struct Host {
    open: Option<OpenChangeVocal>,
    filter_open: bool,
    filter_dialog: Option<DialogId>,
    edit: BgmFilter,
}

#[derive(Component)]
struct ChangeVocalView;

#[derive(Component)]
struct FilterView;

// ---------------------------------------------------------------------------
// Open / close
// ---------------------------------------------------------------------------

fn close_dialog(screens: &mut ScreenManager, id: DialogId) {
    screens.close_dialog(id);
    screens.dialog_destroyed(id);
}

fn close_change_vocal(
    screens: &mut ScreenManager,
    host: &mut Host,
    state: &mut BgmSelect,
    commands: &mut Commands,
) {
    let Some(open) = host.open.take() else { return };
    close_dialog(screens, open.id);
    commands.entity(open.entity).despawn();
    state.dialog_open = false;
}

fn close_filter(screens: &mut ScreenManager, host: &mut Host, state: &mut BgmSelect) {
    if let Some(id) = host.filter_dialog.take() {
        close_dialog(screens, id);
    }
    host.filter_open = false;
    state.dialog_open = false;
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// `CreateBGMSelectChangeVocalListCellDataList` for the vocals this client can
/// place: ordinary vocals in `Seq` order. Special-season vocals need the
/// current season and `streaming_live` vocals need the user's owned vocal
/// rows; neither table is carried, so both are omitted and named.
fn change_vocal_rows(
    vocals: &[VocalDetail],
    masters: &BgmMasters,
    words: &VocalWordings,
) -> Vec<ChangeVocalRow> {
    let mut rows = Vec::new();
    for vocal in vocals.iter().filter(|vocal| vocal.special_season_id == 0) {
        if vocal.vocal_type == "streaming_live" {
            continue;
        }
        let source = VocalRow {
            id: vocal.id,
            vocal_type: vocal.vocal_type.clone(),
            characters: vocal.characters.clone(),
        };
        rows.push(ChangeVocalRow {
            vocal_id: vocal.id,
            caption: vocal.caption.clone(),
            text: model::vocal_text(&source, masters, words),
        });
    }
    rows
}

fn open_requests(
    mut commands: Commands,
    mut state: ResMut<BgmSelect>,
    mut host: ResMut<Host>,
    mut screens: ResMut<ScreenManager>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    docs: Res<Documents>,
    details: Res<Details>,
) {
    let Some(request) = state.dialog_request.take() else {
        return;
    };
    if host.open.is_some() || host.filter_open {
        return;
    }
    if docs.state != DocState::Installed {
        error!("[bgm-select] the dialog was requested but its document is not installed; nothing opens");
        return;
    }
    match request {
        DialogRequest::Filter => {
            let id = match screens.show_dialog(
                crate::ui_layers::DialogType::BGMFilterDialog,
                DisplayLayerType::LayerDialog,
                DialogBackKey::Close,
                "BGMSelectPresenter.OnClickFilter",
            ) {
                Ok(id) => id,
                Err(error) => {
                    error!("[bgm-select] {error}");
                    return;
                }
            };
            screens.open_dialog(id);
            screens.dialog_open_finished(id);
            host.edit = state.filter.clone();
            host.filter_open = true;
            host.filter_dialog = Some(id);
            state.dialog_open = true;
            info!(
                "[bgm-select] filter dialog -> Show2ButtonDialog(BGMFilterDialog, 398): music filter {}, {} unchecked characters, other {}",
                host.edit.music,
                host.edit.unchecked.len(),
                host.edit.other
            );
        }
        DialogRequest::ChangeVocal(record) => {
            let (music_id, selected) = {
                let Some(presenter) = state.presenter.as_ref() else { return };
                let Some(model) = presenter.model(Some(record)) else { return };
                (model.music_id, model.vocal_id.unwrap_or(0))
            };
            let Some(music_id) = music_id else {
                error!("[bgm-select] change vocal: record {record} is a soundtrack");
                return;
            };
            let rows = {
                let Some(masters) = state.masters.as_ref().and_then(|m| m.as_ref().ok()) else {
                    error!("[bgm-select] change vocal: the record masters are absent");
                    return;
                };
                let Some(vocals) = details
                    .vocals()
                    .and_then(|vocals| vocals.get(&music_id))
                else {
                    error!("[bgm-select] change vocal: the musicVocals detail table is absent");
                    return;
                };
                change_vocal_rows(vocals, masters, &words(&layouts))
            };
            if rows.is_empty() {
                error!("[bgm-select] change vocal: the record has no vocal this root can offer (named: streaming-live and special-season vocals need userMusicVocals)");
                return;
            }
            let Some(cell_doc) = layouts.document(CHANGE_VOCAL_CELL).cloned() else { return };
            let Some(mut doc) = layouts.document(CHANGE_VOCAL).cloned() else { return };
            let content = match change_vocal_content(&doc) {
                Ok(path) => path,
                Err(error) => {
                    error!("[bgm-select] change vocal: {error}");
                    return;
                }
            };
            let close = match change_vocal_close(&doc) {
                Ok(close) => close,
                Err(error) => {
                    error!("[bgm-select] change vocal: {error}");
                    return;
                }
            };
            let names: Vec<String> = (0..rows.len()).map(|i| format!("Vocal{i}")).collect();
            let instances = match doc.instantiate_children(
                &content,
                &cell_doc,
                &cell_doc.prefab,
                names.iter().map(String::as_str),
            ) {
                Ok(instances) => instances,
                Err(error) => {
                    error!("[bgm-select] change vocal: the cells are not instantiated: {error}");
                    return;
                }
            };
            let cells = match change_vocal_bindings(&cell_doc, &instances) {
                Ok(cells) => cells,
                Err(error) => {
                    error!("[bgm-select] change vocal: {error}");
                    return;
                }
            };
            if let Err(error) = layouts.replace_runtime_document(CHANGE_VOCAL, doc, &server) {
                error!("[bgm-select] change vocal: {error}");
                return;
            }
            let id = match screens.show_dialog(
                crate::ui_layers::DialogType::MysekaiBGMSelectChangeVocalDialog,
                DisplayLayerType::LayerDialog,
                DialogBackKey::Close,
                "BGMSelectPresenter.ShowChangeVocalDialog",
            ) {
                Ok(id) => id,
                Err(error) => {
                    error!("[bgm-select] {error}");
                    return;
                }
            };
            screens.open_dialog(id);
            screens.dialog_open_finished(id);
            let entity = commands
                .spawn((
                    ChangeVocalView,
                    Visibility::Inherited,
                    Transform::from_xyz(0., 0., 200.),
                    RenderLayers::layer(SITEMAP_LAYER),
                    UiPrefabView::new(CHANGE_VOCAL, SITEMAP_LAYER),
                ))
                .id();
            let count = rows.len();
            host.open = Some(OpenChangeVocal {
                id,
                entity,
                record,
                cells,
                rows,
                selected,
                close,
            });
            state.dialog_open = true;
            info!(
                "[bgm-select] change vocal dialog -> ShowSubWindowDialog(MysekaiBGMSelectChangeVocalDialog, 320): {count} vocals of record {record}, selected {selected}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Back key
// ---------------------------------------------------------------------------

fn on_back_key(
    mut events: MessageReader<DialogBackKeyEvent>,
    mut screens: ResMut<ScreenManager>,
    mut host: ResMut<Host>,
    mut state: ResMut<BgmSelect>,
    mut commands: Commands,
) {
    for event in events.read() {
        if host.open.as_ref().is_some_and(|open| open.id == event.id) {
            info!("[bgm-select] change vocal: back key -> Close");
            close_change_vocal(&mut screens, &mut host, &mut state, &mut commands);
        } else if host.filter_dialog == Some(event.id) {
            info!("[bgm-select] filter: back key -> cancel");
            close_filter(&mut screens, &mut host, &mut state);
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    host: Res<Host>,
    docs: Res<Documents>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut change: Query<
        (&mut UiPrefabView, &mut Visibility, &mut Transform),
        With<ChangeVocalView>,
    >,
    mut filter: Query<
        (&mut UiPrefabView, &mut Visibility, &mut Transform),
        (With<FilterView>, Without<ChangeVocalView>),
    >,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    if let Ok((mut view, mut visibility, mut transform)) = change.single_mut() {
        crate::canvas::set_scale(&mut transform, scale);
        visibility.set_if_neq(if host.open.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(open) = host.open.as_ref() {
            let count = open.rows.len() as f32;
            let top = (count - 1.) * 0.5 * CELL_HEIGHT;
            for (index, (cell, row)) in open.cells.iter().zip(open.rows.iter()).enumerate() {
                view.set_anchored_position(&cell.root, Vec2::new(0., top - index as f32 * CELL_HEIGHT));
                view.set_text(&cell.caption, row.caption.clone());
                view.set_text(&cell.vocal_text, row.text.clone());
                view.set_visible(&cell.selected, row.vocal_id == open.selected);
                view.set_visible(&cell.locked, false);
            }
        }
    }
    if let Ok((mut view, mut visibility, mut transform)) = filter.single_mut() {
        crate::canvas::set_scale(&mut transform, scale);
        visibility.set_if_neq(if host.filter_open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if host.filter_open {
            let Some(bindings) = docs.filter_bindings.as_ref() else { return };
            view.set_visible(&bindings.none_check, host.edit.music == 0);
            view.set_visible(&bindings.newly_check, host.edit.music == 1);
            let all_checked = bindings
                .characters
                .iter()
                .all(|(_, _, id)| host.edit.checked(*id));
            view.set_visible(&bindings.all_check, all_checked);
            for (_, check, ids) in &bindings.units {
                view.set_visible(check, ids.iter().all(|id| host.edit.checked(*id)));
            }
            for (_, check, id) in &bindings.characters {
                view.set_visible(check, host.edit.checked(*id));
            }
            view.set_visible(&bindings.other_check, host.edit.other);
        }
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
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn click(
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    mut state: ResMut<BgmSelect>,
    mut host: ResMut<Host>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut screens: ResMut<ScreenManager>,
    mut commands: Commands,
    mut link: ResMut<ServerLink>,
    details: Res<Details>,
    docs: Res<Documents>,
    change: Query<&UiPrefabView, With<ChangeVocalView>>,
    filter: Query<&UiPrefabView, (With<FilterView>, Without<ChangeVocalView>)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    // Drain the taps while no dialog is open, and while this screen is not
    // the current one (the dialogs live as long as the host): the tap that
    // opens a dialog must not be read again as a tap on it.
    if (!host.filter_open && host.open.is_none()) || screens.current() != super::SCREEN {
        gestures.clear();
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    for event in events {
        if !event.kind.is_tap_family() || event.state != GestureState::End {
            continue;
        }
        consumed.0 = true;
        let point = canvas_position(event.position, size, scale);

        if host.open.is_some() {
            let Ok(view) = change.single() else { continue };
            let mut close = false;
            let mut chosen = None;
            let mut record = 0;
            if let Some(open) = host.open.as_ref() {
                let inside = hit(view, &layouts, &open.close.window, point, canvas);
                close = open
                    .close
                    .button
                    .iter()
                    .any(|close| hit(view, &layouts, close, point, canvas))
                    || (!inside
                        && open
                            .close
                            .area
                            .iter()
                            .any(|close| hit(view, &layouts, close, point, canvas)));
                record = open.record;
                for (cell, row) in open.cells.iter().zip(open.rows.iter()) {
                    if hit(view, &layouts, &cell.button, point, canvas) {
                        chosen = Some(row.vocal_id);
                    }
                }
            }
            if close && chosen.is_none() {
                info!("[bgm-select] change vocal: close");
                close_change_vocal(&mut screens, &mut host, &mut state, &mut commands);
                return;
            }
            let Some(vocal_id) = chosen else { continue };
            select_vocal(&mut state, &details, &layouts, record, vocal_id, &mut link);
            if let Some(open) = host.open.as_mut() {
                open.selected = vocal_id;
            }
            return;
        }

        let Some(bindings) = docs.filter_bindings.as_ref() else { continue };
        let Ok(view) = filter.single() else { continue };
        // Footer.
        if hit(view, &layouts, &bindings.positive, point, canvas) {
            commit_filter(&mut state, &mut host, &details);
            save_filter(&state.filter);
            info!(
                "[bgm-select] filter: OK -> BGMFilterData(music {}, {} unchecked, other {}); the list re-filters",
                state.filter.music,
                state.filter.unchecked.len(),
                state.filter.other
            );
            close_filter(&mut screens, &mut host, &mut state);
            return;
        }
        if hit(view, &layouts, &bindings.negative, point, canvas)
            || hit(view, &layouts, &bindings.close, point, canvas)
        {
            info!("[bgm-select] filter: cancel, the saved data stays");
            close_filter(&mut screens, &mut host, &mut state);
            return;
        }
        if !hit(view, &layouts, &bindings.window, point, canvas) {
            info!("[bgm-select] filter: outside tap -> cancel");
            close_filter(&mut screens, &mut host, &mut state);
            return;
        }
        // Music filter.
        if hit(view, &layouts, &bindings.none, point, canvas) {
            host.edit.music = 0;
            return;
        }
        if hit(view, &layouts, &bindings.newly, point, canvas) {
            host.edit.music = 1;
            return;
        }
        // All.
        if hit(view, &layouts, &bindings.all, point, canvas) {
            let all_checked = bindings
                .characters
                .iter()
                .all(|(_, _, id)| host.edit.checked(*id));
            for (_, _, id) in &bindings.characters {
                host.edit.set_checked(*id, !all_checked);
            }
            return;
        }
        // Units.
        for (path, _, ids) in &bindings.units {
            if hit(view, &layouts, path, point, canvas) {
                let checked = ids.iter().all(|id| host.edit.checked(*id));
                for id in ids {
                    host.edit.set_checked(*id, !checked);
                }
                return;
            }
        }
        // Characters.
        for (path, _, id) in &bindings.characters {
            if hit(view, &layouts, path, point, canvas) {
                let checked = host.edit.checked(*id);
                host.edit.set_checked(*id, !checked);
                return;
            }
        }
        // The outside-character checkbox.
        if hit(view, &layouts, &bindings.other, point, canvas) {
            host.edit.other = !host.edit.other;
            return;
        }
    }
}

fn commit_filter(state: &mut BgmSelect, host: &mut Host, details: &Details) {
    state.filter = host.edit.clone();
    let Some(presenter) = state.presenter.as_mut() else { return };
    let visible = display(presenter, &state.filter, Some(details));
    let selected = presenter.select;
    if !visible
        .iter()
        .any(|&index| presenter.all[index].record_id == selected.unwrap_or(0))
    {
        presenter.select = visible.first().map(|&index| presenter.all[index].record_id);
    }
}

fn select_vocal(
    state: &mut BgmSelect,
    details: &Details,
    layouts: &UiLayouts,
    record: i32,
    vocal_id: i32,
    link: &mut ServerLink,
) {
    state.vocal_choice.insert(record, vocal_id);
    let (music_id, masters_present) = {
        let Some(presenter) = state.presenter.as_ref() else { return };
        let Some(model) = presenter.model(Some(record)) else { return };
        (model.music_id, state.masters.as_ref().is_some_and(|m| m.is_ok()))
    };
    if !masters_present {
        return;
    }
    let Some(music_id) = music_id else { return };
    let Some(vocal) = details
        .vocals()
        .and_then(|vocals| vocals.get(&music_id))
        .and_then(|vocals| vocals.iter().find(|vocal| vocal.id == vocal_id))
    else {
        return;
    };
    let wording = {
        let masters = state.masters.as_ref().and_then(|m| m.as_ref().ok()).expect("masters present");
        model::vocal_text(
            &VocalRow {
                id: vocal.id,
                vocal_type: vocal.vocal_type.clone(),
                characters: vocal.characters.clone(),
            },
            masters,
            &words(layouts),
        )
    };
    let Some(presenter) = state.presenter.as_mut() else { return };
    if let Some(model) = presenter.all.iter_mut().find(|model| model.record_id == record) {
        model.vocal_id = Some(vocal_id);
        model.vocal_name = wording;
    }
    info!("[bgm-select] change vocal: record {record} -> vocal {vocal_id}");
    // The source sends the set request at once when the record is the
    // chosen one.
    let (setting, select, boot) = (presenter.setting, presenter.select, presenter.boot);
    if setting == select && setting == Some(record) {
        let request = MusicPlaySetRequest {
            mysekai_site_id: boot.site_id as i32,
            mysekai_music_record_id: record,
            music_vocal_id: Some(vocal_id),
            is_instrumental: false,
            mysekai_fixture_id: boot.fixture_id,
            texture_id: boot.color_id,
        };
        let call = Call::MusicPlaySet(request);
        let kind = call.kind();
        let seq = link.call(call);
        state.music_call = Some(ServerPending {
            seq: Some(seq),
            call: kind,
        });
        info!("[bgm-select] change vocal: PutUserMysekaiMusicPlaySetApi sent as seq {seq}");
    }
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

fn load_filter(mut state: ResMut<BgmSelect>) {
    state.filter = read_filter();
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Details>()
        .init_resource::<Host>()
        .add_systems(Startup, (load_documents, request_details, load_filter))
        .add_systems(
            Update,
            (
                install_documents,
                poll_details,
                open_requests
                    .after(super::click)
                    .before(crate::ui_layers::advance),
                on_back_key.after(crate::ui_layers::advance),
                click
                    .run_if(crate::game_settings::scene_input_enabled)
                    .after(super::click)
                    .before(open_requests)
                    .before(crate::pick::pick)
                    .before(crate::ui_layers::advance),
                place
                    .after(super::place)
                    .after(click)
                    .before(crate::ui_layout::render),
            ),
        );
}
