//! The plugins' store: one JSON object per plugin, under `store` in the data
//! directory's plugins directory.
//!
//! A plugin's data is read once, when its scripts load, and written when it
//! changed: the file is written as a temporary sibling first and renamed over
//! the plugin's file, so a reader never sees half a document.

use std::path::{Path, PathBuf};

use bevy::prelude::warn;
use moly_plugin::manifest::PluginId;
use serde_json::{Map, Value};

/// One plugin's saved data, and whether it is newer than its file.
#[derive(Default)]
pub(super) struct File {
    values: Map<String, Value>,
    changed: bool,
}

/// The store directory of a plugins directory.
fn store_dir(home: &Path) -> PathBuf {
    home.join("store")
}

/// One plugin's file in `home` (`<data>/plugins`).
fn file_path(home: &Path, plugin: &PluginId) -> PathBuf {
    store_dir(home).join(format!("{plugin}.json"))
}

impl File {
    /// The plugin's saved data: its file, or an empty object when it has
    /// none or the file cannot be read as one.
    pub(super) fn read(home: &Path, plugin: &PluginId) -> Self {
        let path = file_path(home, plugin);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => return Self::default(),
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(values)) => Self { values, changed: false },
            Ok(_) => {
                warn!("[plugins] {plugin}: {} is not a JSON object; its data starts empty", path.display());
                Self::default()
            }
            Err(error) => {
                warn!("[plugins] {plugin}: {} is not JSON ({error}); its data starts empty", path.display());
                Self::default()
            }
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<Value> {
        self.values.get(key).cloned()
    }

    /// Sets `key`, or removes it when `value` is `None`. Answers whether the
    /// data changed.
    pub(super) fn set(&mut self, key: &str, value: Option<Value>) -> bool {
        match value {
            Some(value) if self.values.get(key) == Some(&value) => false,
            Some(value) => {
                self.values.insert(key.to_owned(), value);
                self.changed = true;
                true
            }
            None if !self.values.contains_key(key) => false,
            None => {
                self.values.remove(key);
                self.changed = true;
                true
            }
        }
    }

    /// Writes the data when it changed since the last write. An empty store
    /// keeps no file: one that emptied removes it.
    pub(super) fn write(&mut self, home: &Path, plugin: &PluginId) -> Result<(), String> {
        if !self.changed {
            return Ok(());
        }
        let path = file_path(home, plugin);
        self.changed = false;
        if self.values.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("{}: {error}", path.display())),
            };
        }
        let dir = store_dir(home);
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let text = serde_json::to_string_pretty(&Value::Object(self.values.clone()))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let part = dir.join(format!("{plugin}.json.part"));
        let written = std::fs::write(&part, text + "\n").and_then(|()| std::fs::rename(&part, &path));
        written.map_err(|error| {
            let _ = std::fs::remove_file(&part);
            format!("{}: {error}", path.display())
        })
    }
}
