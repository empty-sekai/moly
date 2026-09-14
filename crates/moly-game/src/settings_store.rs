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
    save_sections_checked(|_| Ok(sections.iter()
        .map(|(name, patch)| ((*name).to_owned(), patch.clone())).collect()))
        .map(|_| ())
}

/// One synchronous read -> caller preflight/patch preparation -> merge ->
/// encode -> write. The caller's validation and unknown-field preservation
/// both observe this same document; there is no second unchecked fresh read.
/// The successful receipt is exactly the document passed to write_text, not
/// a fallible reread after a write has already committed.
///
/// This is not compare-and-swap: another process/tab can still write between
/// this read and write. No file lock, Web Lock or cross-process CAS is claimed.
pub(crate) fn save_sections_checked(
    prepare: impl FnOnce(&Value) -> Result<Vec<(String, Value)>, String>,
) -> Result<Value, String> {
    let mut document = read_document()?;
    let sections = prepare(&document)?;
    let object = document
        .as_object_mut()
        .expect("read_document returns an object");
    for (name, patch) in sections {
        merge(
            object.entry(name).or_insert(Value::Null),
            &patch,
        );
    }
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
fn write_text(text: &str) -> Result<(), String> {
    let destination = path()?;
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Create settings directory: {error}"))?;
    }
    let temporary = destination.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|error| format!("Write settings: {error}"))?;
    std::fs::rename(&temporary, &destination).map_err(|error| format!("Replace settings: {error}"))
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
