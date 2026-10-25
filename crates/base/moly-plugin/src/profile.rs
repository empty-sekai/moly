//! The profile: which plugins are enabled, their settings, and which plugin
//! wins a contested exclusive seam.
//!
//! ```toml
//! [plugins."moly.companion"]
//! enabled = true
//! [plugins."moly.companion".config]
//! unit = 5
//!
//! [seams]
//! "companion.brain" = "moly.companion"            # the whole seam
//! "companion.brain@unit:5" = "example.emu-brain"   # one target
//! ```
//!
//! A plugin absent from `[plugins]` is disabled. Unknown keys are refused.
//! Disabling a plugin keeps its entry (and its save data elsewhere); only
//! `enabled` changes.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::manifest::{ManifestError, PluginId};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub plugins: BTreeMap<PluginId, PluginEntry>,
    /// Keys are `seam` or `seam@target`.
    pub seams: BTreeMap<String, PluginId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginEntry {
    pub enabled: bool,
    pub config: toml::Table,
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let file: ProfileFile =
            toml::from_str(text).map_err(|err| ManifestError::Syntax(err.to_string()))?;
        let mut plugins = BTreeMap::new();
        for (id, entry) in file.plugins {
            plugins.insert(
                PluginId::new(&id)?,
                PluginEntry {
                    enabled: entry.enabled,
                    config: entry.config,
                },
            );
        }
        let mut seams = BTreeMap::new();
        for (seam, plugin) in file.seams {
            seams.insert(seam, PluginId::new(&plugin)?);
        }
        Ok(Self { plugins, seams })
    }

    /// Read the profile; a missing file is the empty profile.
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(ManifestError::Io {
                path: path.display().to_string(),
                error: err.to_string(),
            }),
        }
    }

    /// Serialise in the documented layout (round-trips through `parse`).
    pub fn to_toml(&self) -> String {
        let file = ProfileFile {
            plugins: self
                .plugins
                .iter()
                .map(|(id, entry)| {
                    (
                        id.to_string(),
                        PluginFile {
                            enabled: entry.enabled,
                            config: entry.config.clone(),
                        },
                    )
                })
                .collect(),
            seams: self
                .seams
                .iter()
                .map(|(seam, plugin)| (seam.clone(), plugin.to_string()))
                .collect(),
        };
        toml::to_string(&file).expect("a profile always serialises to TOML")
    }

    pub fn enabled(&self) -> impl Iterator<Item = &PluginId> {
        self.plugins
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .map(|(id, _)| id)
    }
}

/// The on-disk shape of the profile. Unknown keys are refused at every level.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    plugins: BTreeMap<String, PluginFile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    seams: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PluginFile {
    #[serde(default)]
    enabled: bool,
    #[serde(default, skip_serializing_if = "is_empty_table")]
    config: toml::Table,
}

fn is_empty_table(table: &toml::Table) -> bool {
    table.is_empty()
}
