//! Shared local settings document. Every writer merges its own fields into the
//! latest document, so an audio save cannot erase graphics preferences.
//!
//! The browser game keeps the document in the page backend ([`PageDocument`])
//! beside the server model's two documents (`server` and `local`): the page
//! seeds all three, every committed write of any of them bumps the one
//! revision, and the page stores the revision it takes, all three together.
//! The native game uses the same backend and stores each revision in three
//! files ([`open_native_game`]). Settings writers are the same in every
//! backend.

use bevy::prelude::*;
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Resource, Default)]
pub(crate) struct SettingsStore {
    pub(crate) last_error: Option<String>,
}

impl SettingsStore {
    pub(crate) fn load(&mut self) -> Option<Value> {
        match read_document() {
            Ok(document) => {
                self.last_error = None;
                Some(document)
            }
            Err(error) => {
                self.last_error = Some(error);
                None
            }
        }
    }

    pub(crate) fn save(&mut self, sections: &[(&str, Value)]) -> bool {
        match save_sections(sections) {
            Ok(()) => {
                self.last_error = None;
                true
            }
            Err(error) => {
                self.last_error = Some(error);
                false
            }
        }
    }
}

/// Missing storage is a first run. Malformed existing data is an error and is
/// never silently overwritten by an unrelated settings domain.
pub(crate) fn read_document() -> Result<Value, String> {
    match read_text()? {
        None => Ok(Value::Object(Map::new())),
        Some(text) => {
            let value: Value =
                serde_json::from_str(&text).map_err(|error| format!("Settings JSON: {error}"))?;
            if !value.is_object() {
                return Err("Settings root must be an object".into());
            }
            Ok(value)
        }
    }
}

/// Also callable by the legacy Option audio writer without a World borrow.
/// A fresh read on each synchronous write preserves changes from other domains.
pub(crate) fn save_sections(sections: &[(&str, Value)]) -> Result<(), String> {
    save_sections_checked(|_| {
        Ok(sections
            .iter()
            .map(|(name, patch)| ((*name).to_owned(), patch.clone()))
            .collect())
    })
    .map(|_| ())
}

/// One synchronous read -> caller preflight/patch preparation -> merge ->
/// encode -> write. The caller's validation and unknown-field preservation
/// both observe this same document; there is no second unchecked fresh read.
/// The successful receipt is exactly the document passed to write_text, not
/// a fallible reread after a write has already committed.
///
/// The transaction guard excludes other native writers. Browser writes require
/// the exclusive Web Lock held by the page for its lifetime.
pub(crate) fn save_sections_checked(
    prepare: impl FnOnce(&Value) -> Result<Vec<(String, Value)>, String>,
) -> Result<Value, String> {
    update_document_checked(|document| {
        let sections = prepare(document)?;
        let object = document
            .as_object_mut()
            .expect("read_document returns an object");
        for (name, patch) in sections {
            merge(object.entry(name).or_insert(Value::Null), &patch);
        }
        Ok(())
    })
}

/// Replace complete domain records after validation against the fresh document.
/// Other domains are retained, and no in-memory state changes before this returns.
pub(crate) fn update_document_checked(
    update: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<Value, String> {
    let _guard = transaction_guard()?;
    let mut document = read_document()?;
    update(&mut document)?;
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("Encode settings: {error}"))?;
    write_text(&text)?;
    Ok(document)
}

fn merge(target: &mut Value, patch: &Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            merge(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    } else {
        *target = patch.clone();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> Result<std::path::PathBuf, String> {
    use std::path::PathBuf;
    if let Ok(value) = std::env::var("MOLY_SETTINGS_FILE") {
        if !value.trim().is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    Ok(moly_store::location::data_dir()?.join("settings.json"))
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_read_text() -> Result<Option<String>, String> {
    match std::fs::read_to_string(path()?) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Read settings: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_guard() -> Result<std::fs::File, String> {
    let destination = path()?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Create settings directory: {error}"))?;
    let mut lock_path = destination.into_os_string();
    lock_path.push(".lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| format!("Open settings lock: {error}"))?;
    file.try_lock()
        .map_err(|error| format!("Another process is saving these settings; retry: {error}"))?;
    Ok(file)
}

static BROWSER_WRITABLE: AtomicBool = AtomicBool::new(false);

const READ_ONLY: &str = "This tab is read-only. Exclusive browser storage access is unavailable; close other moly tabs and reload to save.";

/// The host sets this only while it holds the exclusive storage lock (the
/// page's Web Lock, or the native game's lease file).
pub fn set_browser_storage_writable(writable: bool) {
    BROWSER_WRITABLE.store(writable, Ordering::Relaxed);
}

pub(crate) fn browser_storage_writable() -> bool {
    BROWSER_WRITABLE.load(Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
fn platform_guard() -> Result<(), String> {
    if browser_storage_writable() {
        Ok(())
    } else {
        Err(READ_ONLY.into())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_write_text(text: &str) -> Result<(), String> {
    write_file(&path()?, text)
}

/// Replaces `destination` with `text` through a synced temporary file in the
/// same directory.
#[cfg(not(target_arch = "wasm32"))]
fn write_file(destination: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".moly-settings-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| format!("Create temporary settings: {error}"))?;
    temporary
        .write_all(text.as_bytes())
        .map_err(|error| format!("Write settings: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Sync settings: {error}"))?;
    temporary
        .persist(destination)
        .map_err(|error| format!("Replace settings: {error}"))?;
    Ok(())
}

/// The native game's files: the settings document at the settings path and
/// the server model's documents beside it (`<name>.server.json` and
/// `<name>.local.json`), in [`PAGE_DOCUMENTS`] order, with the write lease
/// (`<name>.game.lock`) when this process holds it.
#[cfg(not(target_arch = "wasm32"))]
struct NativeGameFiles {
    paths: [std::path::PathBuf; 3],
    lease: Option<std::fs::File>,
}

#[cfg(not(target_arch = "wasm32"))]
static NATIVE_GAME: Mutex<Option<NativeGameFiles>> = Mutex::new(None);

/// Opens the native game's files: whether this process holds their write
/// lease, and each document's text (`None` when its file does not exist).
/// Called once, before the page backend is installed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_native_game() -> Result<(bool, [Option<String>; 3]), String> {
    let settings = path()?;
    let parent = settings
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&parent)
        .map_err(|error| format!("Create settings directory: {error}"))?;
    let name = settings
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("settings path {} has no file name", settings.display()))?
        .to_owned();
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(format!("{name}.game.lock")))
        .map_err(|error| format!("Open the game lease: {error}"))?;
    let lease = lease.try_lock().is_ok().then_some(lease);
    let paths = [
        settings,
        parent.join(format!("{name}.server.json")),
        parent.join(format!("{name}.local.json")),
    ];
    let mut texts = [None, None, None];
    for (text, path) in texts.iter_mut().zip(&paths) {
        *text = match std::fs::read_to_string(path) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("Read {}: {error}", path.display())),
        };
    }
    let writable = lease.is_some();
    if let Ok(mut slot) = NATIVE_GAME.lock() {
        *slot = Some(NativeGameFiles { paths, lease });
    }
    Ok((writable, texts))
}

/// Writes the page backend's newest unacknowledged revision to the native
/// game's files and acknowledges it. Nothing to do without a lease or a
/// newer revision.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn persist_native_game() -> Result<(), String> {
    let native = NATIVE_GAME
        .lock()
        .map_err(|_| "the native game files are unavailable".to_owned())?;
    let Some(files) = native.as_ref().filter(|files| files.lease.is_some()) else {
        return Ok(());
    };
    let Some((revision, texts)) = with_page(|page| {
        page.take(page.acked())
            .map(|(revision, texts)| (revision, texts.map(|text| text.map(str::to_owned))))
    })
    .flatten() else {
        return Ok(());
    };
    for (path, text) in files.paths.iter().zip(texts) {
        if let Some(text) = text {
            write_file(path, &text)?;
        }
    }
    with_page(|page| page.ack(revision)).unwrap_or(Ok(()))
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "Browser window is unavailable".to_owned())?
        .local_storage()
        .map_err(|error| format!("Open localStorage: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled".to_owned())
}

#[cfg(target_arch = "wasm32")]
fn platform_read_text() -> Result<Option<String>, String> {
    storage()?
        .get_item("moly-settings")
        .map_err(|error| format!("Read localStorage: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn platform_write_text(text: &str) -> Result<(), String> {
    storage()?
        .set_item("moly-settings", text)
        .map_err(|error| format!("Write localStorage: {error:?}"))
}

pub(crate) fn location() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(location) = NATIVE_GAME.lock().ok().and_then(|native| {
        native.as_ref().map(|files| {
            files
                .paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
    }) {
        return location;
    }
    if page_installed() {
        return "page settings document".into();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        path()
            .map(|value| value.display().to_string())
            .unwrap_or_else(|error| error)
    }
    #[cfg(target_arch = "wasm32")]
    {
        "localStorage[moly-settings]".into()
    }
}

/// The page backend's document, or the platform store when none is installed.
pub(crate) fn read_text() -> Result<Option<String>, String> {
    match with_page(|page| page.texts[0].clone()) {
        Some(text) => Ok(text),
        None => platform_read_text(),
    }
}

fn write_text(text: &str) -> Result<(), String> {
    match with_page(|page| page.write(text)) {
        Some(result) => result,
        None => platform_write_text(text),
    }
}

#[cfg(not(target_arch = "wasm32"))]
type PlatformGuard = std::fs::File;
#[cfg(target_arch = "wasm32")]
type PlatformGuard = ();

/// The page backend keeps the exclusive-writer rule of the browser store: a
/// tab without the storage lease commits nothing and counts the refusal.
fn transaction_guard() -> Result<Option<PlatformGuard>, String> {
    let writable = browser_storage_writable();
    match with_page(|page| {
        if !writable {
            page.refused = page.refused.saturating_add(1);
        }
        writable
    }) {
        Some(true) => Ok(None),
        Some(false) => Err(READ_ONLY.into()),
        None => platform_guard().map(Some),
    }
}

/// The documents of the page backend, in the order the page stores them.
pub(crate) const PAGE_DOCUMENTS: [&str; 3] = ["settings", "server", "local"];

/// In-memory documents of the browser game: the settings document and the
/// server model's `server` and `local` documents. Revision 0 is the seed;
/// each committed write of any document is the next revision. `acked` is
/// the newest revision the page reported as stored.
#[derive(Debug, Default)]
pub(crate) struct PageDocument {
    texts: [Option<String>; 3],
    revision: u32,
    acked: u32,
    refused: u32,
}

impl PageDocument {
    pub(crate) fn seeded(
        settings: Option<String>,
        server: Option<String>,
        local: Option<String>,
    ) -> Self {
        Self {
            texts: [settings, server, local],
            ..Self::default()
        }
    }

    fn bump(&mut self) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Settings revision is exhausted")?;
        Ok(())
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        self.bump()?;
        self.texts[0] = Some(text.to_owned());
        Ok(())
    }

    /// A named document's new text; an unchanged text is not a revision.
    fn write_named(&mut self, name: &str, text: String) -> Result<(), String> {
        let index = PAGE_DOCUMENTS
            .iter()
            .position(|known| *known == name)
            .ok_or_else(|| format!("page document {name} is unknown"))?;
        if self.texts[index].as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        self.bump()?;
        self.texts[index] = Some(text);
        Ok(())
    }

    /// Every document (null when never written) when the newest revision is
    /// newer than `after`.
    pub(crate) fn take(&self, after: u32) -> Option<(u32, [Option<&str>; 3])> {
        if self.revision <= after {
            return None;
        }
        Some((
            self.revision,
            [
                self.texts[0].as_deref(),
                self.texts[1].as_deref(),
                self.texts[2].as_deref(),
            ],
        ))
    }

    /// The page may only acknowledge a revision it was offered.
    pub(crate) fn ack(&mut self, revision: u32) -> Result<(), String> {
        if revision > self.revision {
            return Err(format!(
                "persist.ack revision {revision} was never offered (newest is {})",
                self.revision
            ));
        }
        self.acked = self.acked.max(revision);
        Ok(())
    }

    pub(crate) fn revision(&self) -> u32 {
        self.revision
    }

    pub(crate) fn acked(&self) -> u32 {
        self.acked
    }

    /// Writes refused because this tab holds no storage lease.
    pub(crate) fn refused(&self) -> u32 {
        self.refused
    }
}

static PAGE: Mutex<Option<PageDocument>> = Mutex::new(None);

/// Selects the page backend before the app is built; the platform store is
/// never read or written afterwards.
pub(crate) fn install_page_document(
    settings: Option<String>,
    server: Option<String>,
    local: Option<String>,
) {
    if let Ok(mut page) = PAGE.lock() {
        *page = Some(PageDocument::seeded(settings, server, local));
    }
}

/// Commits the new text of the page's `server` or `local` document, under
/// the same exclusive-writer rule as the settings document. `Err` without a
/// page backend.
pub(crate) fn commit_page_document(name: &str, text: String) -> Result<(), String> {
    let writable = browser_storage_writable();
    with_page(|page| {
        if !writable {
            page.refused = page.refused.saturating_add(1);
            return Err(READ_ONLY.to_owned());
        }
        page.write_named(name, text)
    })
    .unwrap_or_else(|| Err("the page backend is not installed".into()))
}

fn page_installed() -> bool {
    with_page(|_| ()).is_some()
}

/// `None` when no page backend is installed.
pub(crate) fn with_page<R>(operation: impl FnOnce(&mut PageDocument) -> R) -> Option<R> {
    PAGE.lock().ok()?.as_mut().map(operation)
}

