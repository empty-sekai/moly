//! Where the OC model comes from, and its load.
//!
//! The player wears a character body of the cast or a model file of their
//! own. A character body is a unit of the character manifest: its model
//! file loads as a cast member's does, and its rig profile gives it the
//! character material ([`super::dress`]).
//!
//! The player chooses a model file on their own device: by dropping it on
//! the game's window or page, through the settings panel's file picker, or,
//! in native builds, by naming it in the `MOLY_OC_MODEL` environment
//! variable. The file is read into memory and never sent anywhere or stored:
//! its humanoid map is read ([`super::vrm::read_model`]) and the loader's copy
//! is put into an in-memory asset source (`oc://`), from which the engine's
//! glTF loader reads it once; the copy is dropped from that source when the
//! load completes. A new file replaces the current model; choosing the avatar
//! body removes it.

use super::constraint::NodeConstraint;
use super::vrm::{self, MapSource, ModelFile, MAX_MODEL_BYTES};
use crate::player_data_io::{self, FileKind, ReadTask};
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::AssetSourceBuilder;
use bevy::asset::{AssetApp, AssetPath, LoadState, RecursiveDependencyLoadState};
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

/// The in-memory asset source the model copy is read from.
pub(crate) const SOURCE: &str = "oc";

/// The picker's file family.
pub(crate) static MODEL_FILE: FileKind = FileKind {
    label: "Humanoid model (VRM, glTF)",
    extensions: &["vrm", "glb", "gltf"],
    input_id: "moly-oc-model-file",
};

/// Native builds: a model file to wear from the start.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MODEL_ENV: &str = "MOLY_OC_MODEL";

/// Native builds: a character body to wear from the start, by unit id.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const UNIT_ENV: &str = "MOLY_OC_UNIT";

/// The in-memory directory behind [`SOURCE`].
#[derive(Resource, Clone)]
pub(crate) struct OcFiles(Dir);

/// Registers the in-memory source. Asset sources are registered before the
/// asset plugin is built.
pub(crate) fn register_source(app: &mut App) {
    let dir = Dir::default();
    let reader = dir.clone();
    app.register_asset_source(
        SOURCE,
        AssetSourceBuilder::new(move || {
            Box::new(MemoryAssetReader {
                root: reader.clone(),
            })
        }),
    );
    app.insert_resource(OcFiles(dir));
}

/// What the player wears, for the settings panel and the log.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum OcStatus {
    #[default]
    Avatar,
    Reading(String),
    Loading(String),
    Worn(String),
    Refused(String),
}

impl OcStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            OcStatus::Avatar => "Player model: avatar".into(),
            OcStatus::Reading(name) => format!("Player model: reading {name}"),
            OcStatus::Loading(name) => format!("Player model: loading {name}"),
            OcStatus::Worn(what) => format!("Player model: {what}"),
            OcStatus::Refused(why) => format!("Player model: avatar ({why})"),
        }
    }
}

/// A request from the settings panel.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) enum OcRequest {
    /// Choose a model file with the file picker.
    Pick,
    /// Wear the avatar body.
    Avatar,
    /// Wear a character body.
    Character(CharacterChoice),
}

/// Which character body to wear: a unit, or the character manifest's unit
/// after or before the one worn (in unit order, wrapping around).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharacterChoice {
    Unit(u32),
    Next,
    Previous,
}

/// A character body: the unit and its rig profile.
#[derive(Clone, Debug)]
pub(crate) struct CharacterBody {
    pub(crate) unit: u32,
    pub(crate) rig_file: String,
    pub(crate) rig: Handle<JsonAsset>,
}

/// A read model file (`None`: the picker was cancelled).
struct Delivery {
    name: String,
    result: Result<Option<ModelFile>, String>,
}

#[derive(Default)]
struct InboxState {
    ticket: u64,
    delivery: Option<Delivery>,
}

type Inbox = Arc<Mutex<InboxState>>;

/// Parses a read file and leaves it for [`receive`], unless a newer request
/// has taken the inbox.
fn deliver(inbox: &Inbox, ticket: u64, name: String, result: Result<Option<Vec<u8>>, String>) {
    let result = result.and_then(|bytes| bytes.map(|bytes| vrm::read_model(&bytes)).transpose());
    let mut state = inbox.lock().unwrap_or_else(PoisonError::into_inner);
    if state.ticket == ticket {
        state.delivery = Some(Delivery { name, result });
    }
}

/// The model being loaded or worn.
pub(crate) struct OcLoad {
    pub(crate) generation: u64,
    pub(crate) name: String,
    pub(crate) source: MapSource,
    pub(crate) gltf: Handle<Gltf>,
    /// The file's node constraints, in evaluation order.
    pub(crate) constraints: Vec<NodeConstraint>,
    /// The copy's path in [`OcFiles`] until the load completes (model
    /// files only).
    path: Option<PathBuf>,
    /// Loaded with its dependencies.
    pub(crate) ready: bool,
    pub(crate) character: Option<CharacterBody>,
}

/// The player's model choice.
#[derive(Resource, Default)]
pub(crate) struct OcSelection {
    pub(crate) status: OcStatus,
    inbox: Inbox,
    reading: Option<ReadTask>,
    generations: u64,
    pub(crate) model: Option<OcLoad>,
    /// A character body waiting for the character manifest.
    wanted: Option<CharacterChoice>,
    manifest: Option<Handle<JsonAsset>>,
}

impl OcSelection {
    /// A new request: reads still running for older ones are cancelled and
    /// their results dropped.
    fn ticket(&mut self) -> (Inbox, u64) {
        self.reading = None;
        self.wanted = None;
        let mut state = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        state.ticket += 1;
        state.delivery = None;
        let ticket = state.ticket;
        drop(state);
        (self.inbox.clone(), ticket)
    }

    pub(crate) fn refuse(&mut self, name: &str, reason: &str) {
        error!("[oc] {name} refused: {reason}; the avatar body stays");
        self.status = OcStatus::Refused(format!("{name}: {reason}"));
    }

    fn start(&mut self, name: String, task: Result<ReadTask, String>) {
        match task {
            Ok(task) => {
                info!("[oc] reading {name}");
                self.status = OcStatus::Reading(name);
                self.reading = Some(task);
            }
            Err(reason) => self.refuse(&name, &reason),
        }
    }

    /// Reads a local file (native builds).
    #[cfg(not(target_arch = "wasm32"))]
    fn read_path(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        let (inbox, ticket) = self.ticket();
        let reported = name.clone();
        let task = player_data_io::read_bytes(path, MAX_MODEL_BYTES, move |result| {
            deliver(&inbox, ticket, reported, result.map(Some));
        });
        self.start(name, task);
    }

    /// Reads a browser file (one dropped on the page).
    #[cfg(target_arch = "wasm32")]
    fn read_web_file(&mut self, file: web_sys::File) {
        let name = file.name();
        let (inbox, ticket) = self.ticket();
        let reported = name.clone();
        let task = player_data_io::read_web_file(file, MAX_MODEL_BYTES, move |result| {
            deliver(&inbox, ticket, reported, result.map(Some));
        });
        self.start(name, task);
    }

    fn pick(&mut self) {
        let (inbox, ticket) = self.ticket();
        let task = player_data_io::choose_bytes(&MODEL_FILE, MAX_MODEL_BYTES, move |result| {
            match result {
                Ok(Some((name, bytes))) => deliver(&inbox, ticket, name, Ok(Some(bytes))),
                Ok(None) => deliver(&inbox, ticket, String::new(), Ok(None)),
                Err(reason) => deliver(&inbox, ticket, "The chosen file".into(), Err(reason)),
            }
        });
        match task {
            Ok(task) => self.reading = Some(task),
            Err(reason) => self.refuse("The file picker", &reason),
        }
    }

    fn want_character(&mut self, choice: CharacterChoice) {
        let _ = self.ticket();
        self.wanted = Some(choice);
    }

    fn wear_avatar(&mut self) {
        let _ = self.ticket();
        self.model = None;
        self.status = OcStatus::Avatar;
        info!("[oc] the avatar body is worn");
    }

    /// Puts the copy into the in-memory source and starts its load.
    fn load(&mut self, name: String, model: ModelFile, files: &OcFiles, server: &AssetServer) {
        self.generations += 1;
        let generation = self.generations;
        let path = PathBuf::from(format!("model-{generation}.{}", model.extension));
        let bytes = model.bytes.len();
        files.0.insert_asset(&path, model.bytes);
        let asset_path = AssetPath::from(format!("{SOURCE}://model-{generation}.{}", model.extension));
        let gltf = server.load_with_settings(asset_path, |settings: &mut GltfLoaderSettings| {
            settings.load_materials = moly_assets::residency::GPU_ONLY;
            settings.load_cameras = false;
            settings.load_lights = false;
            settings.load_animations = false;
        });
        let mapped: Vec<String> = model
            .original_names
            .iter()
            .filter(|(_, original)| !original.is_empty())
            .take(6)
            .map(|(bone, original)| format!("{bone}={original}"))
            .collect();
        info!(
            "[oc] {name}: {} humanoid map, {} bones ({}...), {bytes} bytes to the loader",
            model.source.label(),
            model.bones.len(),
            mapped.join(", ")
        );
        self.status = OcStatus::Loading(name.clone());
        self.model = Some(OcLoad {
            generation,
            name,
            source: model.source,
            gltf,
            constraints: model.constraints,
            path: Some(path),
            ready: false,
            character: None,
        });
    }

    /// Starts loading a character body: its model file as a cast member's,
    /// and its rig profile.
    fn load_character(&mut self, unit: u32, glb: &str, rig: &str, server: &AssetServer) {
        self.generations += 1;
        let gltf = moly_assets::residency::load_gltf(
            server,
            moly_assets::character_glb(glb),
            moly_assets::residency::GltfResidency::Character,
        );
        let name = glb.strip_suffix(".glb").unwrap_or(glb).to_owned();
        info!("[oc] unit {unit}: character body {glb} + {rig}");
        self.status = OcStatus::Loading(name.clone());
        self.model = Some(OcLoad {
            generation: self.generations,
            name,
            source: MapSource::Character,
            gltf,
            constraints: Vec::new(),
            path: None,
            ready: false,
            character: Some(CharacterBody {
                unit,
                rig_file: rig.to_owned(),
                rig: server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig}"))),
            }),
        });
    }
}

/// The character manifest's units in unit order: (unit, model file, rig
/// profile). A manifest code is the unit id plus 100.
fn manifest_units(raw: &str) -> Result<Vec<(u32, String, String)>, String> {
    let manifest = moly_assets::character::CharacterManifest::parse(raw).map_err(|error| error.to_string())?;
    let mut units = manifest
        .units
        .into_iter()
        .map(|row| {
            row.unit
                .parse::<u32>()
                .ok()
                .and_then(|code| code.checked_sub(100))
                .map(|unit| (unit, row.glb, row.rig))
                .ok_or_else(|| format!("code {:?} is not a unit id plus 100", row.unit))
        })
        .collect::<Result<Vec<_>, _>>()?;
    units.sort_by_key(|row| row.0);
    Ok(units)
}

/// The manifest index a choice names, given the unit worn.
fn choose(units: &[(u32, String, String)], choice: CharacterChoice, worn: Option<u32>) -> Option<usize> {
    let count = units.len();
    let at = worn.and_then(|unit| units.iter().position(|row| row.0 == unit));
    match choice {
        CharacterChoice::Unit(unit) => units.iter().position(|row| row.0 == unit),
        _ if count == 0 => None,
        CharacterChoice::Next => Some(at.map_or(0, |at| (at + 1) % count)),
        CharacterChoice::Previous => Some(at.map_or(count - 1, |at| (at + count - 1) % count)),
    }
}

/// Update: a wanted character body is looked up in the character manifest
/// and starts loading.
pub(crate) fn choose_character(
    mut selection: ResMut<OcSelection>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
) {
    let Some(choice) = selection.wanted else {
        return;
    };
    let manifest = selection
        .manifest
        .get_or_insert_with(|| server.load::<JsonAsset>(moly_assets::character_manifest()))
        .clone();
    if let LoadState::Failed(error) = server.load_state(&manifest) {
        selection.wanted = None;
        selection.model = None;
        return selection.refuse("The character manifest", &error.to_string());
    }
    let Some(json) = jsons.get(&manifest) else {
        return;
    };
    selection.wanted = None;
    let units = match manifest_units(&json.0) {
        Ok(units) => units,
        Err(reason) => {
            selection.model = None;
            return selection.refuse("The character manifest", &reason);
        }
    };
    let worn = selection
        .model
        .as_ref()
        .and_then(|model| model.character.as_ref())
        .map(|body| body.unit);
    let Some(index) = choose(&units, choice, worn) else {
        selection.model = None;
        let what = match choice {
            CharacterChoice::Unit(unit) => format!("Unit {unit}"),
            _ => "The character body".into(),
        };
        return selection.refuse(&what, "the character manifest has no body for it");
    };
    let (unit, glb, rig) = &units[index];
    selection.load_character(*unit, glb, rig, &server);
}

/// Native builds: the model named by [`MODEL_ENV`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn from_environment(mut selection: ResMut<OcSelection>) {
    if let Some(path) = std::env::var_os(MODEL_ENV).filter(|value| !value.is_empty()) {
        selection.read_path(PathBuf::from(path));
    }
    if let Ok(value) = std::env::var(UNIT_ENV) {
        match value.trim().parse::<u32>() {
            Ok(unit) => selection.want_character(CharacterChoice::Unit(unit)),
            Err(_) => selection.refuse(UNIT_ENV, &format!("{value:?} is not a unit id")),
        }
    }
}

/// Whether a file name is one of [`MODEL_FILE`]'s.
pub(crate) fn is_model_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    MODEL_FILE
        .extensions
        .iter()
        .any(|extension| lower.ends_with(&format!(".{extension}")))
}

/// Update: settings-panel requests and model files dropped on the window.
pub(crate) fn requests(
    mut selection: ResMut<OcSelection>,
    mut requests: MessageReader<OcRequest>,
    #[cfg(not(target_arch = "wasm32"))] mut drops: MessageReader<bevy::window::FileDragAndDrop>,
) {
    for request in requests.read() {
        match request {
            OcRequest::Pick => selection.pick(),
            OcRequest::Avatar => selection.wear_avatar(),
            OcRequest::Character(choice) => selection.want_character(*choice),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    for drop in drops.read() {
        if let bevy::window::FileDragAndDrop::DroppedFile { path_buf, .. } = drop {
            if is_model_name(&path_buf.to_string_lossy()) {
                selection.read_path(path_buf.clone());
            } else {
                info!("[oc] dropped file {} is not a .vrm, .glb or .gltf file", path_buf.display());
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    for file in web_drop::take() {
        if is_model_name(&file.name()) {
            selection.read_web_file(file);
        } else {
            info!("[oc] dropped file {} is not a .vrm, .glb or .gltf file", file.name());
        }
    }
}

/// Update: a read and parsed file starts its load.
pub(crate) fn receive(mut selection: ResMut<OcSelection>, files: Res<OcFiles>, server: Res<AssetServer>) {
    let delivery = selection
        .inbox
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .delivery
        .take();
    let Some(Delivery { name, result }) = delivery else {
        return;
    };
    selection.reading = None;
    match result {
        Ok(None) => info!("[oc] model choice cancelled"),
        Ok(Some(model)) => selection.load(name, model, &files, &server),
        Err(reason) => {
            selection.model = None;
            selection.refuse(&name, &reason);
        }
    }
}

/// Update: the load's outcome. The copy leaves memory either way.
pub(crate) fn poll_load(mut selection: ResMut<OcSelection>, files: Res<OcFiles>, server: Res<AssetServer>) {
    let Some(model) = selection.model.as_ref().filter(|model| !model.ready) else {
        return;
    };
    let failure = match server.load_state(&model.gltf) {
        LoadState::Failed(error) => Some(error.to_string()),
        _ => match server.recursive_dependency_load_state(&model.gltf) {
            RecursiveDependencyLoadState::Failed(error) => Some(error.to_string()),
            _ => None,
        },
    };
    if let Some(reason) = failure {
        if let Some(path) = &model.path {
            files.0.remove_asset(path);
        }
        let name = model.name.clone();
        selection.model = None;
        selection.refuse(&name, &reason);
        return;
    }
    if server.is_loaded_with_dependencies(&model.gltf) {
        if let Some(path) = &model.path {
            files.0.remove_asset(path);
        }
        let model = selection.model.as_mut().expect("checked above");
        model.ready = true;
        info!("[oc] {} loaded", model.name);
    }
}

/// The browser page's drop target: the window takes model files dropped
/// anywhere on the page.
#[cfg(target_arch = "wasm32")]
pub(crate) mod web_drop {
    use std::cell::RefCell;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    thread_local! {
        static DROPPED: RefCell<Vec<web_sys::File>> = const { RefCell::new(Vec::new()) };
    }

    /// Startup: listens for drops on the window. A drag over the page is
    /// accepted (so that the browser does not open the file in place of the
    /// game), and the first file of a drop is queued.
    pub(crate) fn listen() {
        let Some(window) = web_sys::window() else {
            bevy::log::warn!("[oc] no browser window: model files cannot be dropped");
            return;
        };
        let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|event: web_sys::DragEvent| {
            event.prevent_default();
        });
        let dropped = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|event: web_sys::DragEvent| {
            event.prevent_default();
            let file = event
                .data_transfer()
                .and_then(|transfer| transfer.files())
                .and_then(|files| files.get(0));
            if let Some(file) = file {
                DROPPED.with(|queue| queue.borrow_mut().push(file));
            }
        });
        let listening = window
            .add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref())
            .and_then(|()| {
                window.add_event_listener_with_callback("drop", dropped.as_ref().unchecked_ref())
            });
        if listening.is_err() {
            bevy::log::warn!("[oc] the page refused the drop listeners: model files cannot be dropped");
            return;
        }
        // The listeners live as long as the page.
        over.forget();
        dropped.forget();
    }

    pub(super) fn take() -> Vec<web_sys::File> {
        DROPPED.with(|queue| std::mem::take(&mut *queue.borrow_mut()))
    }
}
