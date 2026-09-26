//! Browser game transport. The page starts one game from a seed, enqueues
//! validated intents and polls a versioned projection; it never borrows the
//! world. The settings document lives in the page backend of
//! `settings_store`, and the page persists the revisions it takes.
//!
//! Input capture and persist acknowledgements take effect when
//! `game_command` returns: a hidden page renders no frames, so nothing the
//! page needs while hidden may wait for the next schedule. Every settings
//! writer commits synchronously into the page backend, so the newest
//! revision is takeable at once and a hidden page flushes by taking it.
//! Lifecycle and input intents are also queued for PreUpdate, where the
//! world applies its part of them.

use bevy::prelude::*;
use serde_json::{json, Map, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

const SCHEMA: u64 = 1;
const ABI: u64 = 1;
const MAX_COMMAND_BYTES: usize = 1024;
const MAX_PENDING_COMMANDS: usize = 128;
const MAX_ERRORS: usize = 32;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static COMMANDS: Mutex<Vec<GameCommand>> = Mutex::new(Vec::new());
static PUBLISHED: Mutex<Option<Published>> = Mutex::new(None);
// Set synchronously by the page, before the next input schedule.
static INPUT_CAPTURED: AtomicBool = AtomicBool::new(false);

/// One validated seed. `documents.settings` moves into the settings page
/// backend when the storage is installed; the resource keeps the rest.
#[derive(Resource, Clone, Debug)]
pub struct GameSeed {
    pub region: String,
    pub version: String,
    pub snapshot_id: String,
    pub release_id: String,
    pub assets: String,
    pub packs: bool,
    pub asset_catalog: Option<String>,
    pub resource_base: Option<String>,
    pub resource_origin: Option<String>,
    pub writable: bool,
    settings: Option<String>,
}

const SEED_FIELDS: [&str; 13] = [
    "schemaVersion",
    "abi",
    "region",
    "version",
    "snapshotId",
    "releaseId",
    "assets",
    "packs",
    "assetCatalog",
    "resourceBase",
    "resourceOrigin",
    "writable",
    "documents",
];

fn field<'a>(seed: &'a Map<String, Value>, name: &str) -> Result<&'a Value, String> {
    seed.get(name)
        .ok_or_else(|| format!("seed field {name} is missing"))
}

fn text(seed: &Map<String, Value>, name: &str) -> Result<String, String> {
    match field(seed, name)? {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!("seed field {name} must be a non-empty string")),
    }
}

/// Present, and either a non-empty string or null.
fn optional_text(seed: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    match field(seed, name)? {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(format!(
            "seed field {name} must be a non-empty string or null"
        )),
    }
}

fn flag(seed: &Map<String, Value>, name: &str) -> Result<bool, String> {
    field(seed, name)?
        .as_bool()
        .ok_or_else(|| format!("seed field {name} must be true or false"))
}

fn version(seed: &Map<String, Value>, name: &str, supported: u64) -> Result<(), String> {
    let value = field(seed, name)?;
    if value.as_u64() == Some(supported) {
        return Ok(());
    }
    Err(format!(
        "seed {name} {value} is not supported (this engine reads {supported})"
    ))
}

/// Validates the seed's shape. Asset locations are admitted by the caller
/// with the same rules as the page query parameters.
pub fn parse_game_seed(input: &str) -> Result<GameSeed, String> {
    let value: Value =
        serde_json::from_str(input).map_err(|error| format!("seed is not JSON: {error}"))?;
    let seed = value.as_object().ok_or("seed must be a JSON object")?;
    version(seed, "schemaVersion", SCHEMA)?;
    version(seed, "abi", ABI)?;
    if let Some(unknown) = seed.keys().find(|key| !SEED_FIELDS.contains(&key.as_str())) {
        return Err(format!("seed field {unknown} is unknown"));
    }
    let region = text(seed, "region")?;
    if !matches!(region.as_str(), "cn" | "jp") {
        return Err(format!("seed region {region} is not cn or jp"));
    }
    let documents = field(seed, "documents")?
        .as_object()
        .ok_or("seed field documents must be an object")?;
    if let Some(unknown) = documents.keys().find(|key| key.as_str() != "settings") {
        return Err(format!("seed document {unknown} is unknown"));
    }
    let settings = match field(documents, "settings")? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        _ => return Err("seed document settings must be a string or null".into()),
    };
    Ok(GameSeed {
        region,
        version: text(seed, "version")?,
        snapshot_id: text(seed, "snapshotId")?,
        release_id: text(seed, "releaseId")?,
        assets: text(seed, "assets")?,
        packs: flag(seed, "packs")?,
        asset_catalog: optional_text(seed, "assetCatalog")?,
        resource_base: optional_text(seed, "resourceBase")?,
        resource_origin: optional_text(seed, "resourceOrigin")?,
        writable: flag(seed, "writable")?,
        settings,
    })
}

/// Before the app is built: some plugins read the settings document while
/// they are added. Selects the page backend and the page's storage lease.
pub fn install_game_storage(seed: &mut GameSeed) {
    crate::settings_store::install_page_document(seed.settings.take());
    #[cfg(target_arch = "wasm32")]
    crate::settings_store::set_browser_storage_writable(seed.writable);
    ACTIVE.store(true, Ordering::Relaxed);
}

/// True once a browser game owns this module instance; library and
/// player-data intents are refused from then on.
pub fn game_mode_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    Visible,
    Hidden,
    PageHide,
}

impl Lifecycle {
    fn name(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::PageHide => "pagehide",
        }
    }
}

#[derive(Debug)]
enum GameCommand {
    Lifecycle(Lifecycle),
    Input(bool),
    PersistAck(u32),
}

fn only_fields(command: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match command.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(unknown) => Err(format!("game command field {unknown} is unknown")),
        None => Ok(()),
    }
}

fn parse_command(input: &str) -> Result<GameCommand, String> {
    if input.len() > MAX_COMMAND_BYTES {
        return Err("game command exceeds its length limit".into());
    }
    let value: Value =
        serde_json::from_str(input).map_err(|_| "game command is not JSON".to_owned())?;
    let command = value
        .as_object()
        .ok_or("game command must be a JSON object")?;
    let kind = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or("game command has no string type")?;
    match kind {
        "lifecycle" => {
            only_fields(command, &["type", "value"])?;
            Ok(GameCommand::Lifecycle(
                match command.get("value").and_then(Value::as_str) {
                    Some("hidden") => Lifecycle::Hidden,
                    Some("visible") => Lifecycle::Visible,
                    Some("pagehide") => Lifecycle::PageHide,
                    _ => return Err("lifecycle value must be hidden, visible or pagehide".into()),
                },
            ))
        }
        "input" => {
            only_fields(command, &["type", "captured"])?;
            Ok(GameCommand::Input(
                command
                    .get("captured")
                    .and_then(Value::as_bool)
                    .ok_or("input captured must be true or false")?,
            ))
        }
        "persist.ack" => {
            only_fields(command, &["type", "revision"])?;
            Ok(GameCommand::PersistAck(
                command
                    .get("revision")
                    .and_then(Value::as_u64)
                    .and_then(|revision| u32::try_from(revision).ok())
                    .ok_or("persist.ack revision must be a revision number")?,
            ))
        }
        _ => Err(format!("game command type {kind} is unknown")),
    }
}

pub fn game_command(input: &str) -> Result<(), String> {
    if !game_mode_active() {
        return Err("the browser game has not started".into());
    }
    match parse_command(input)? {
        // The acknowledgement has no world part.
        GameCommand::PersistAck(revision) => {
            crate::settings_store::with_page(|page| page.ack(revision))
                .ok_or("the settings page backend is not installed")?
        }
        command => enqueue(command),
    }
}

fn enqueue(command: GameCommand) -> Result<(), String> {
    let mut queue = COMMANDS
        .lock()
        .map_err(|_| "game command queue is unavailable".to_owned())?;
    if queue.len() >= MAX_PENDING_COMMANDS {
        return Err("game command queue is full; retry after the next frame".into());
    }
    // Nothing is buffered in the engine for a lifecycle: a hidden page
    // flushes by taking.
    if let GameCommand::Input(captured) = command {
        INPUT_CAPTURED.store(captured, Ordering::Relaxed);
    }
    queue.push(command);
    Ok(())
}

/// `""` when nothing newer than `after_revision` exists.
pub fn game_take_persist(after_revision: u32) -> String {
    crate::settings_store::with_page(|page| {
        page.take(after_revision).map(|(revision, settings)| {
            json!({"revision": revision, "documents": {"settings": settings}}).to_string()
        })
    })
    .flatten()
    .unwrap_or_default()
}

/// The world's part of the projection, written by [`publish`].
#[derive(Clone, PartialEq)]
struct Published {
    cover: &'static str,
    step: &'static str,
    control_open: bool,
    hud_open: bool,
    errors: Vec<String>,
}

impl Default for Published {
    fn default() -> Self {
        Self {
            cover: "opaque",
            step: "starting",
            control_open: false,
            hud_open: false,
            errors: Vec::new(),
        }
    }
}

/// Null until a game has started (and on native, which has no linear
/// memory); then the linear memory size, at most 4 GiB on wasm32.
fn wasm_bytes() -> Value {
    if !game_mode_active() {
        return Value::Null;
    }
    #[cfg(target_arch = "wasm32")]
    return json!(core::arch::wasm32::memory_size::<0>() as u64 * 65_536);
    #[cfg(not(target_arch = "wasm32"))]
    Value::Null
}

/// Entry state from the last frame; storage state as of this call.
pub fn game_snapshot() -> String {
    let mut world = PUBLISHED
        .lock()
        .ok()
        .and_then(|published| published.clone())
        .unwrap_or_default();
    let (revision, acked, refused) =
        crate::settings_store::with_page(|page| (page.revision(), page.acked(), page.refused()))
            .unwrap_or_default();
    if refused > 0 {
        world.errors.push(format!(
            "persist: {refused} settings writes refused because this tab is read-only"
        ));
    }
    json!({
        "schemaVersion": SCHEMA,
        "abi": ABI,
        "entry": {
            "cover": world.cover,
            "step": world.step,
            "controlOpen": world.control_open,
            "hudOpen": world.hud_open,
        },
        "writable": crate::settings_store::browser_storage_writable(),
        "persist": {"revision": revision, "acked": acked},
        "memory": {"wasmBytes": wasm_bytes()},
        "errors": world.errors,
    })
    .to_string()
}

/// Named refusals seen by the world, oldest first, without repeats.
#[derive(Resource, Default)]
struct GameErrors(Vec<String>);

impl GameErrors {
    fn push(&mut self, error: String) {
        if self.0.len() < MAX_ERRORS && !self.0.contains(&error) {
            error!("[game] {error}");
            self.0.push(error);
        }
    }
}

/// After the app is built and before it runs. Installs no library or stage
/// configuration and no developer scaffolding.
pub fn configure_browser_game(app: &mut App, seed: GameSeed) {
    let mut errors = GameErrors::default();
    // A malformed stored document refuses every writer; the page must see it.
    if let Err(error) = crate::settings_store::read_document() {
        errors.push(format!("settings document: {error}"));
    }
    app.insert_resource(seed)
        .insert_resource(errors)
        .add_systems(Startup, announce)
        .add_systems(
            PreUpdate,
            (gate_input, drain_commands)
                .chain()
                .after(bevy::input::InputSystems)
                .before(crate::game_settings::input)
                .before(crate::content_library::input),
        )
        .add_systems(
            Last,
            (close_orphan_panel, check_source_region, publish).chain(),
        );
}

fn announce(seed: Res<GameSeed>) {
    info!(
        "[game] seed {} {} snapshot {} release {}; developer tools absent, gated: {}",
        seed.region,
        seed.version,
        seed.snapshot_id,
        seed.release_id,
        crate::dev_tools::GATED.join(" · ")
    );
}

fn gate_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    if INPUT_CAPTURED.load(Ordering::Relaxed) {
        // Also clears held movement when a DOM surface takes focus.
        keys.reset_all();
        buttons.reset_all();
    }
}

fn drain_commands(mut library: ResMut<crate::content_library::ContentLibrary>) {
    let commands = COMMANDS
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default();
    for command in commands {
        match command {
            GameCommand::Lifecycle(value) => info!("[game] page lifecycle {}", value.name()),
            GameCommand::Input(captured) => library.set_host_input_capture(captured),
            GameCommand::PersistAck(_) => {}
        }
    }
}

/// The settings panel is developer UI and is not spawned in the game. A
/// writer that opens it (a colour or import failure) would otherwise hold
/// world input behind an invisible modal.
fn close_orphan_panel(
    mut panel: ResMut<crate::game_settings::SettingsPanel>,
    mut errors: ResMut<GameErrors>,
) {
    if panel.open {
        panel.close_after_import();
        errors.push("settings panel was requested; it is not part of the game".into());
    }
}

fn check_source_region(
    seed: Res<GameSeed>,
    region: Option<Res<crate::site::NavMeshSourceRegion>>,
    mut errors: ResMut<GameErrors>,
    mut checked: Local<bool>,
) {
    let Some(region) = region.filter(|_| !*checked) else {
        return;
    };
    *checked = true;
    let loaded = match region.0 {
        moly_law::carve::NavMeshRegion::Cn => "cn",
        moly_law::carve::NavMeshRegion::Jp => "jp",
    };
    if loaded != seed.region {
        errors.push(format!(
            "seed region {} does not match the snapshot's source region {loaded}",
            seed.region
        ));
    }
}

/// The cover and the await as the entry itself holds them; "starting" until
/// the entry exists.
fn publish(entry: Option<Res<crate::entry::EntrySequence>>, errors: Res<GameErrors>) {
    let seq = entry.as_deref();
    let cover = seq.map_or("opaque", crate::entry::EntrySequence::cover_state);
    let step = seq.map_or("starting", crate::entry::EntrySequence::phase_name);
    let control_open = seq.is_some() && crate::entry::control_open(seq);
    let hud_open = seq.is_some() && crate::entry::hud_open(seq);
    let next = Published {
        cover,
        step,
        control_open,
        hud_open,
        errors: errors.0.clone(),
    };
    if let Ok(mut slot) = PUBLISHED.lock() {
        if slot.as_ref() != Some(&next) {
            *slot = Some(next);
        }
    }
}
