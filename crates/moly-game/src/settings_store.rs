//! Shared local settings document. Every writer merges its own fields into the
//! latest document, so an audio save cannot erase graphics preferences.
//!
//! The browser game keeps the document in the page backend ([`PageDocument`]):
//! the page seeds it, every committed write bumps its revision, and the page
//! stores the revision it takes. Writers are the same in every backend.

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
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable".to_owned())?;
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".local/share")))
        .ok_or_else(|| "User data directory is unavailable".to_owned())?;
    Ok(base.join("moly/settings.json"))
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

/// The browser host sets this only while it holds the exclusive storage lock.
#[cfg(target_arch = "wasm32")]
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
    use std::io::Write;
    let destination = path()?;
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
        .persist(&destination)
        .map_err(|error| format!("Replace settings: {error}"))?;
    Ok(())
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
    match with_page(|page| page.text.clone()) {
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

/// In-memory settings document of the browser game. Revision 0 is the seed;
/// each committed write is the next revision. `acked` is the newest revision
/// the page reported as stored.
#[derive(Debug, Default)]
pub(crate) struct PageDocument {
    text: Option<String>,
    revision: u32,
    acked: u32,
    refused: u32,
}

impl PageDocument {
    pub(crate) fn seeded(text: Option<String>) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Settings revision is exhausted")?;
        self.text = Some(text.to_owned());
        Ok(())
    }

    /// The newest document when it is newer than `after`.
    pub(crate) fn take(&self, after: u32) -> Option<(u32, &str)> {
        if self.revision <= after {
            return None;
        }
        Some((self.revision, self.text.as_deref()?))
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
pub(crate) fn install_page_document(text: Option<String>) {
    if let Ok(mut page) = PAGE.lock() {
        *page = Some(PageDocument::seeded(text));
    }
}

fn page_installed() -> bool {
    with_page(|_| ()).is_some()
}

/// `None` when no page backend is installed.
pub(crate) fn with_page<R>(operation: impl FnOnce(&mut PageDocument) -> R) -> Option<R> {
    PAGE.lock().ok()?.as_mut().map(operation)
}

