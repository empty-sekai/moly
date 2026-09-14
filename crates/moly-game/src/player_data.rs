//! Player-data import, preview, durable replacement and recovery.

use bevy::{asset::LoadState, prelude::*};
use moly_assets::{
    json::JsonAsset,
    player_data::{ImportedPlayerData, PlayerDataCatalog, MAX_IMPORT_BYTES},
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    fixture::layouts::{self, SiteFixtureLayouts},
    settings_store,
    site::{SiteSelection, Sites},
};

pub const DEFAULT_PLAYER_API: &str =
    "https://haruki-api.menardi.top/api/{region}/mysekai/{target_user_id}";
const PROFILE: &str = "ImportedPlayerData";
const BACKUP: &str = "PlayerDataImportBackup";

#[derive(Clone, Copy, Component, Debug)]
pub(crate) enum ImportAction {
    Fetch,
    File,
    Paste,
    ClearUid,
    Region,
    Apply,
    Restore,
    Cancel,
}

struct Payload {
    text: String,
    region: String,
    uid: Option<String>,
    apply: bool,
}
enum Incoming {
    Data(u64, Result<Option<Payload>, String>),
    Paste(Result<String, String>),
}
type Inbox = Arc<Mutex<Vec<Incoming>>>;

struct Preview {
    data: ImportedPlayerData,
    section: Value,
    before_layouts: Option<Value>,
    before_profile: Option<Value>,
}

#[derive(Resource)]
pub struct PlayerDataImport {
    pub(crate) uid: String,
    pub(crate) region: String,
    pub(crate) status: String,
    pub(crate) busy: bool,
    pub(crate) action: Option<ImportAction>,
    api: String,
    inbox: Inbox,
    generation: u64,
    pending: Option<Payload>,
    preview: Option<Preview>,
    requests: Option<(Handle<JsonAsset>, Handle<JsonAsset>)>,
    catalog: Option<PlayerDataCatalog>,
}

impl Default for PlayerDataImport {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let api = DEFAULT_PLAYER_API;
        #[cfg(target_arch = "wasm32")]
        let api = "/player-api/{region}/{target_user_id}";
        Self { uid: String::new(), region: "cn".into(),
            status: "Enter a player UID or choose a Mysekai JSON file. Import keeps a recoverable layout backup.".into(),
            busy: false, action: None, api: api.into(), inbox: Default::default(),
            generation: 0, pending: None, preview: None, requests: None, catalog: None }
    }
}

impl PlayerDataImport {
    /// Entry-point configuration; the game does not read env or URL parameters.
    pub fn configure(&mut self, api: Option<String>, region: Option<String>) -> Result<(), String> {
        if let Some(api) = api {
            if !(api.starts_with("https://") || api.starts_with("http://") || api.starts_with('/'))
                || !api.contains("{region}")
                || !api.contains("{target_user_id}")
            {
                return Err("Player API must be an HTTP(S) URL template containing {region} and {target_user_id}".into());
            }
            self.api = api;
        }
        if let Some(region) = region {
            if !matches!(region.as_str(), "cn" | "jp") {
                return Err("Player region must be cn or jp".into());
            }
            self.region = region;
        }
        Ok(())
    }

    pub fn request_uid(&mut self, uid: String) {
        self.uid = uid;
        self.action = Some(ImportAction::Fetch);
    }

    pub fn request_json(&mut self, text: String, apply: bool) {
        self.cancel();
        self.pending = Some(Payload {
            text,
            region: self.region.clone(),
            uid: None,
            apply,
        });
        self.busy = true;
        self.status = "Checking player data and matching assets...".into();
    }

    pub fn request_apply(&mut self) {
        self.action = Some(ImportAction::Apply);
    }

    pub fn request_restore(&mut self) {
        self.action = Some(ImportAction::Restore);
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn preview_summary(&self) -> Option<(u32, usize, usize)> {
        self.preview.as_ref().map(|preview| {
            (
                preview.data.rank,
                preview.data.sites.len(),
                preview.data.fixture_count(),
            )
        })
    }

    pub(crate) fn has_preview(&self) -> bool {
        self.preview.is_some()
    }

    pub(crate) fn invalidate_preview(&mut self) {
        self.cancel();
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.preview = None;
        self.busy = false;
    }

    fn fetch(&mut self) -> Result<(), String> {
        let uid = self.uid.trim();
        if uid.is_empty()
            || uid.len() > 20
            || !uid.bytes().all(|b| b.is_ascii_digit())
            || uid.bytes().all(|b| b == b'0')
        {
            return Err("Player UID must contain 1–20 digits and cannot be zero".into());
        }
        let uid = uid.to_owned();
        self.cancel();
        self.busy = true;
        self.status = "Fetching player data...".into();
        let url = self
            .api
            .replace("{region}", &self.region)
            .replace("{target_user_id}", &uid);
        let request = ehttp::Request::get(url).with_timeout(Some(Duration::from_secs(60)));
        let (inbox, ticket, region) = (self.inbox.clone(), self.generation, self.region.clone());
        ehttp::fetch(request, move |response| {
            let result = response.and_then(|response| {
                match response.status {
                    200 => {}
                    404 => {
                        return Err(
                            "Player does not exist or their Mysekai home is unavailable (HTTP 404)"
                                .into(),
                        )
                    }
                    401 | 403 => {
                        return Err(format!(
                            "Player API denied access (HTTP {})",
                            response.status
                        ))
                    }
                    429 => {
                        return Err("Player API is rate limited; try again later (HTTP 429)".into())
                    }
                    code => return Err(format!("Player API returned HTTP {code}")),
                }
                decode_bytes(response.bytes).map(|text| {
                    Some(Payload {
                        text,
                        region,
                        uid: Some(uid),
                        apply: false,
                    })
                })
            });
            inbox.lock().unwrap().push(Incoming::Data(ticket, result));
        });
        Ok(())
    }

    fn choose_file(&mut self) {
        self.cancel();
        self.busy = true;
        self.status = "Choose a player-data JSON file...".into();
        choose_file(self.inbox.clone(), self.generation, self.region.clone());
    }

    fn receive(&mut self) {
        let messages = std::mem::take(&mut *self.inbox.lock().unwrap());
        for message in messages {
            match message {
                Incoming::Data(ticket, result) if ticket == self.generation => match result {
                    Ok(Some(payload)) => {
                        self.pending = Some(payload);
                        self.status = "Checking player data and matching assets...".into();
                    }
                    Ok(None) => {
                        self.busy = false;
                        self.status = "File selection cancelled.".into();
                    }
                    Err(error) => {
                        self.busy = false;
                        self.status = error;
                    }
                },
                Incoming::Paste(Ok(text)) => {
                    let text = text.trim();
                    if text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()) {
                        self.cancel();
                        self.uid = text.into();
                    } else {
                        self.status = "The clipboard does not contain a player UID.".into();
                    }
                }
                Incoming::Paste(Err(error)) => self.status = error,
                _ => {}
            }
        }
    }

    fn prepare(&mut self, world: &World) -> Result<bool, String> {
        let Some(payload) = self.pending.as_ref() else {
            return Ok(false);
        };
        if self.catalog.is_none() {
            let server = world.resource::<AssetServer>();
            let (master, models) = self
                .requests
                .get_or_insert_with(|| {
                    (
                        server.load("moly://fixture-models/player-data.json"),
                        server.load("moly://fixture-models/index.json"),
                    )
                })
                .clone();
            for handle in [&master, &models] {
                if matches!(server.load_state(handle), LoadState::Failed(_)) {
                    self.requests = None;
                    return Err("Player import catalog is unavailable. Update the supplied assets, including fixture-models/player-data.json.".into());
                }
            }
            let jsons = world.resource::<Assets<JsonAsset>>();
            let (Some(master), Some(models)) = (jsons.get(&master), jsons.get(&models)) else {
                return Ok(false);
            };
            self.catalog = Some(PlayerDataCatalog::from_json(&master.0, &models.0)?);
        }
        let Some(sites) = world.get_resource::<Sites>() else {
            return Ok(false);
        };
        let mut data = self
            .catalog
            .as_ref()
            .unwrap()
            .import(&payload.text, Some(&payload.region))?;
        if let Some(uid) = &payload.uid {
            if data.player_id.as_ref().is_some_and(|id| id != uid) {
                return Err("The API returned a different player ID; no data was imported.".into());
            }
            data.player_id = Some(uid.clone());
        }
        let section = layouts::import_section(&data, sites)?;
        let document = settings_store::read_document()?;
        layouts::validate_document(&document, sites)?;
        self.status = format!(
            "Ready for {} {}: {} sites, {} furniture instances, Mysekai rank {}.{}",
            data.region.to_uppercase(),
            data.player_id
                .as_deref()
                .or(data.player_name.as_deref())
                .unwrap_or("selected file"),
            data.sites.len(),
            data.fixture_count(),
            data.rank,
            if data.notices.is_empty() {
                String::new()
            } else {
                format!("\n{}", data.notices.join("\n"))
            }
        );
        self.preview = Some(Preview {
            data,
            section,
            before_layouts: document.get(layouts::SECTION).cloned(),
            before_profile: document.get(PROFILE).cloned(),
        });
        let apply = payload.apply;
        self.pending = None;
        self.busy = false;
        Ok(apply)
    }

    fn apply(&mut self, world: &mut World) -> Result<(), String> {
        require_clean_editor(world)?;
        let preview = self
            .preview
            .as_ref()
            .ok_or("Fetch or choose player data before importing")?;
        let selection = SiteSelection::for_player_data(&preview.data);
        let backup_selection = world.resource::<SiteSelection>().snapshot();
        let document = settings_store::update_document_checked(|document| {
            if document.get(layouts::SECTION) != preview.before_layouts.as_ref()
                || document.get(PROFILE) != preview.before_profile.as_ref()
            {
                return Err("Saved player layouts changed after preview. Fetch or choose the data again; nothing was overwritten.".into());
            }
            document[BACKUP] = json!({ "version": 1, "layouts": preview.before_layouts,
                "profile": preview.before_profile, "selection": backup_selection,
                "hasLayouts": preview.before_layouts.is_some(), "hasProfile": preview.before_profile.is_some() });
            document[layouts::SECTION] = preview.section.clone();
            document[PROFILE] = json!({ "version": 1, "region": preview.data.region,
                "rank": preview.data.rank, "playerName": preview.data.player_name,
                "playerId": preview.data.player_id, "source": preview.data.source });
            Ok(())
        })?;
        self.status = format!(
            "Imported {} sites and {} furniture instances. The previous layout is backed up.",
            preview.data.sites.len(),
            preview.data.fixture_count()
        );
        info!(
            "[player-data] imported {} sites, {} fixtures, rank {}",
            preview.data.sites.len(),
            preview.data.fixture_count(),
            preview.data.rank
        );
        replace_live(world, document, selection);
        self.preview = None;
        Ok(())
    }

    fn restore(&mut self, world: &mut World) -> Result<(), String> {
        require_clean_editor(world)?;
        let sites = world
            .get_resource::<Sites>()
            .ok_or("Scene catalog is still loading")?;
        let mut restored_selection = None;
        let document = settings_store::update_document_checked(|document| {
            let backup = document
                .get(BACKUP)
                .cloned()
                .ok_or("No player import backup is available")?;
            if backup["version"].as_u64() != Some(1) {
                return Err("Unsupported player import backup version".into());
            }
            let mut candidate = document.clone();
            for (key, value, present) in [
                (layouts::SECTION, &backup["layouts"], &backup["hasLayouts"]),
                (PROFILE, &backup["profile"], &backup["hasProfile"]),
            ] {
                if !present.as_bool().unwrap_or(!value.is_null()) {
                    candidate.as_object_mut().unwrap().remove(key);
                } else {
                    candidate[key] = value.clone();
                }
            }
            layouts::validate_document(&candidate, sites)?;
            let mut selection = SiteSelection::from_snapshot(&backup["selection"])?;
            selection
                .resolve_level(sites, &SiteFixtureLayouts::from_document(candidate.clone()))?;
            candidate.as_object_mut().unwrap().remove(BACKUP);
            *document = candidate;
            restored_selection = Some(selection);
            Ok(())
        })?;
        self.cancel();
        self.status = "Restored the layout from before the import.".into();
        replace_live(world, document, restored_selection.unwrap());
        Ok(())
    }
}

fn require_clean_editor(world: &World) -> Result<(), String> {
    if world
        .get_resource::<crate::fixture_edit::EditSession>()
        .is_some_and(crate::fixture_edit::has_unsaved_layout)
    {
        Err(
            "Save or discard your furniture edits before importing or restoring player data."
                .into(),
        )
    } else {
        Ok(())
    }
}

fn replace_live(world: &mut World, document: Value, selection: SiteSelection) {
    let rank = profile_rank(&document);
    if let Some(mut panel) = world.get_resource_mut::<crate::game_settings::SettingsPanel>() {
        panel.close_after_import();
    }
    let roots = world
        .query_filtered::<Entity, With<crate::site::SiteRoot>>()
        .iter(world)
        .collect();
    let mut commands = world.commands();
    crate::site::queue_transition(&mut commands, roots, selection);
    commands.queue(move |world: &mut World| {
        if let Some(mut menu) = world.get_resource_mut::<crate::menu_dialog::MenuMock>() {
            menu.set_player_rank(rank);
        }
        if let Some(mut info) = world.get_resource_mut::<crate::info::InfoMock>() {
            info.set_player_rank(rank);
        }
        world.insert_resource(SiteFixtureLayouts::from_document(document));
    });
}

fn profile_rank(document: &Value) -> Option<u32> {
    if document[PROFILE]["version"].as_u64() != Some(1) {
        return None;
    }
    document[PROFILE]["rank"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
}

pub(crate) fn saved_rank() -> Option<u32> {
    settings_store::read_document()
        .ok()
        .as_ref()
        .and_then(profile_rank)
}

/// Prefer the imported home on restart when the entry point has no explicit site.
pub fn preferred_site() -> Option<String> {
    let document = settings_store::read_document().ok()?;
    if document[PROFILE]["version"].as_u64() != Some(1) {
        return None;
    }
    let sites = document[layouts::SECTION]["sites"].as_object()?;
    [
        (1, "home_site"),
        (2, "first_floor"),
        (3, "second_floor"),
        (4, "third_floor"),
    ]
    .into_iter()
    .find_map(|(id, kind)| {
        sites
            .get(&id.to_string())
            .filter(|site| {
                site["mysekaiSiteId"].as_u64() == Some(id)
                    && site["siteType"].as_str() == Some(kind)
            })
            .map(|_| kind.to_owned())
    })
}

pub(crate) fn update(world: &mut World) {
    let Some(mut state) = world.remove_resource::<PlayerDataImport>() else {
        return;
    };
    state.receive();
    let result = match state.action.take() {
        Some(ImportAction::Fetch) => {
            if let Some(mut panel) = world.get_resource_mut::<crate::game_settings::SettingsPanel>()
            {
                panel.open = true;
                panel.player_data = true;
            }
            state.fetch()
        }
        Some(ImportAction::File) => {
            state.choose_file();
            Ok(())
        }
        Some(ImportAction::Apply) => state.apply(world),
        Some(ImportAction::Restore) => state.restore(world),
        Some(ImportAction::Paste) => {
            paste(state.inbox.clone());
            Ok(())
        }
        Some(ImportAction::ClearUid) => {
            state.cancel();
            state.uid.clear();
            Ok(())
        }
        Some(ImportAction::Region) => {
            state.cancel();
            state.region = if state.region == "cn" { "jp" } else { "cn" }.into();
            Ok(())
        }
        Some(ImportAction::Cancel) => {
            state.cancel();
            state.status = "Import cancelled.".into();
            Ok(())
        }
        None => Ok(()),
    }
    .and_then(|_| {
        state
            .prepare(world)
            .and_then(|apply| if apply { state.apply(world) } else { Ok(()) })
    });
    if let Err(error) = result {
        state.busy = false;
        state.pending = None;
        state.status = error;
        if let Some(mut panel) = world.get_resource_mut::<crate::game_settings::SettingsPanel>() {
            panel.open = true;
            panel.player_data = true;
        }
    }
    world.insert_resource(state);
}

fn decode_bytes(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("Player data exceeds 32 MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Player data must be a UTF-8 JSON file".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_json_file(path: &std::path::Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Open player data: {e}"))?;
    if !file
        .metadata()
        .map_err(|e| format!("Read player data: {e}"))?
        .is_file()
    {
        return Err("Choose a player-data file".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Read player data: {e}"))?;
    decode_bytes(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn choose_file(inbox: Inbox, ticket: u64, region: String) {
    std::thread::spawn(move || {
        let result = rfd::FileDialog::new()
            .add_filter("Mysekai player data", &["json"])
            .pick_file()
            .map(|path| {
                read_json_file(&path).map(|text| Payload {
                    text,
                    region,
                    uid: None,
                    apply: false,
                })
            })
            .transpose();
        inbox.lock().unwrap().push(Incoming::Data(ticket, result));
    });
}

#[cfg(target_arch = "wasm32")]
fn choose_file(inbox: Inbox, ticket: u64, region: String) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = match rfd::AsyncFileDialog::new()
            .add_filter("Mysekai player data", &["json"])
            .pick_file()
            .await
        {
            Some(file) => decode_bytes(file.read().await).map(|text| {
                Some(Payload {
                    text,
                    region,
                    uid: None,
                    apply: false,
                })
            }),
            None => Ok(None),
        };
        inbox.lock().unwrap().push(Incoming::Data(ticket, result));
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn paste(inbox: Inbox) {
    let result = arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .map_err(|e| format!("Read clipboard: {e}"));
    inbox.lock().unwrap().push(Incoming::Paste(result));
}

#[cfg(target_arch = "wasm32")]
fn paste(inbox: Inbox) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = match web_sys::window() {
            Some(window) => {
                wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().read_text())
                    .await
                    .map_err(|_| "Clipboard access failed; type the UID instead".to_owned())
                    .and_then(|value| {
                        value
                            .as_string()
                            .ok_or_else(|| "Clipboard is not text".into())
                    })
            }
            None => Err("Browser window is unavailable".into()),
        };
        inbox.lock().unwrap().push(Incoming::Paste(result));
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn drop_files(
    mut files: MessageReader<bevy::window::FileDragAndDrop>,
    mut state: ResMut<PlayerDataImport>,
    mut panel: ResMut<crate::game_settings::SettingsPanel>,
) {
    for event in files.read() {
        if let bevy::window::FileDragAndDrop::DroppedFile { path_buf, .. } = event {
            panel.open = true;
            panel.player_data = true;
            match read_json_file(path_buf) {
                Ok(text) => state.request_json(text, false),
                Err(error) => {
                    state.cancel();
                    state.status = error;
                }
            }
        }
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<PlayerDataImport>();
    app.add_systems(Update, update.before(crate::fixture::FixtureLayoutSet));
    app.add_systems(
        PreUpdate,
        crate::player_data_ui::input.after(crate::game_settings::input),
    );
    app.add_systems(Update, crate::player_data_ui::refresh.after(update));
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Update, drop_files.before(update));
}
