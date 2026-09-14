//! Shared local settings document. Every writer merges its own fields into the
//! latest document, so an audio save cannot erase graphics preferences.

use bevy::prelude::*;
use serde_json::{Map, Value};

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
pub(crate) fn read_text() -> Result<Option<String>, String> {
    match std::fs::read_to_string(path()?) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Read settings: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn transaction_guard() -> Result<std::fs::File, String> {
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

#[cfg(target_arch = "wasm32")]
static BROWSER_WRITABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The browser host sets this only while it holds the exclusive storage lock.
#[cfg(target_arch = "wasm32")]
pub fn set_browser_storage_writable(writable: bool) {
    BROWSER_WRITABLE.store(writable, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
fn transaction_guard() -> Result<(), String> {
    if BROWSER_WRITABLE.load(std::sync::atomic::Ordering::Relaxed) {
        Ok(())
    } else {
        Err("This tab is read-only. Exclusive browser storage access is unavailable; close other moly tabs and reload to save.".into())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_text(text: &str) -> Result<(), String> {
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
pub(crate) fn read_text() -> Result<Option<String>, String> {
    storage()?
        .get_item("moly-settings")
        .map_err(|error| format!("Read localStorage: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn write_text(text: &str) -> Result<(), String> {
    storage()?
        .set_item("moly-settings", text)
        .map_err(|error| format!("Write localStorage: {error:?}"))
}

pub(crate) fn location() -> String {
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
