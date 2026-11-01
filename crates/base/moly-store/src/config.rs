//! The launcher's configuration: its keys in the configuration file
//! (`moly.toml`, see [`moly_config`]), and the resource selection a game
//! started without configuration uses.
//!
//! A data directory that still holds the earlier `launcher.json` and whose
//! `moly.toml` has none of the launcher's keys has the file's values imported
//! into `moly.toml` on the first load; the old file is then renamed to
//! `launcher.json.imported`.

use crate::location;
use crate::record::{Installed, Record};
use moly_config::{keys, Config, Key, Value};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LauncherConfig {
    pub schema: u32,
    /// The resource CDN base, ending with `/`; required to download or list
    /// resources (`resources.base`).
    pub cdn: Option<String>,
    /// Where resources are downloaded; the default directory when absent
    /// (`resources.dir`).
    pub resource_dir: Option<PathBuf>,
    /// The snapshot id the game starts with; the newest install when absent
    /// (`resources.snapshot`).
    pub snapshot: Option<String>,
    /// Parallel transfers while downloading (`launcher.jobs`).
    pub jobs: usize,
    /// Start the game's developer window (`dev.window`).
    pub developer: bool,
    /// The game program; the one beside the launcher when absent
    /// (`launcher.game`).
    pub game: Option<PathBuf>,
    /// The GitHub API address of the repository that publishes program releases;
    /// the default repository (`update::DEFAULT_SOURCE`) when absent
    /// (`update.source`).
    pub update_source: Option<String>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            cdn: None,
            resource_dir: None,
            snapshot: None,
            jobs: 16,
            developer: false,
            game: None,
            update_source: None,
        }
    }
}

/// The keys this configuration owns in `moly.toml`.
const LAUNCHER_KEYS: [&Key; 7] = [
    &keys::RESOURCE_BASE,
    &keys::RESOURCE_DIR,
    &keys::RESOURCE_SNAPSHOT,
    &keys::LAUNCHER_JOBS,
    &keys::DEV_WINDOW,
    &keys::LAUNCHER_GAME,
    &keys::UPDATE_SOURCE,
];

impl LauncherConfig {
    /// The saved configuration, or the defaults when none is saved.
    pub fn load() -> Result<Self, String> {
        let path = location::config_path()?;
        let config = Config::load(&path).map_err(|error| error.to_string())?;
        if !Self::has_any(&config) {
            if let Some(imported) = Self::import(&path)? {
                return Ok(imported);
            }
        }
        Self::from_config(&config)
    }

    fn has_any(config: &Config) -> bool {
        LAUNCHER_KEYS.iter().any(|key| config.has(key))
    }

    fn from_config(config: &Config) -> Result<Self, String> {
        let defaults = Self::default();
        let jobs = match config.file_integer(&keys::LAUNCHER_JOBS) {
            Some(jobs) => usize::try_from(jobs).map_err(|_| format!("`{}` must not be negative", keys::LAUNCHER_JOBS.path))?,
            None => defaults.jobs,
        };
        Ok(Self {
            schema: defaults.schema,
            cdn: non_empty(config.file_text(&keys::RESOURCE_BASE)),
            resource_dir: config.file_text(&keys::RESOURCE_DIR).map(PathBuf::from),
            snapshot: config.file_text(&keys::RESOURCE_SNAPSHOT),
            jobs,
            developer: config.file_flag(&keys::DEV_WINDOW).unwrap_or(defaults.developer),
            game: config.file_text(&keys::LAUNCHER_GAME).map(PathBuf::from),
            update_source: config.file_text(&keys::UPDATE_SOURCE),
        })
    }

    /// Imports `launcher.json` into `path` when it exists. `None` when there
    /// is nothing to import.
    fn import(path: &Path) -> Result<Option<Self>, String> {
        let old = location::launcher_config_path()?;
        let bytes = match std::fs::read(&old) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", old.display())),
        };
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", old.display()))?;
        config.write(path)?;
        let mut imported = old.as_os_str().to_owned();
        imported.push(".imported");
        std::fs::rename(&old, PathBuf::from(imported)).map_err(|error| format!("{}: {error}", old.display()))?;
        Ok(Some(config))
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(cdn) = self.cdn.as_deref() {
            crate::check_cdn(cdn)?;
        }
        if let Some(dir) = self.resource_dir.as_deref().filter(|dir| !dir.is_absolute()) {
            return Err(format!("the resource directory must be a full path: {}", dir.display()));
        }
        self.write(&location::config_path()?)
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let text = |value: Option<&str>| value.map(|value| Value::Text(value.to_owned()));
        let path_text = |value: &Option<PathBuf>| value.as_deref().map(|value| Value::Text(value.display().to_string()));
        let jobs = i64::try_from(self.jobs).map_err(|_| format!("`{}` is too large", keys::LAUNCHER_JOBS.path))?;
        moly_config::set_values(
            path,
            &[
                (&keys::RESOURCE_BASE, text(self.cdn.as_deref())),
                (&keys::RESOURCE_DIR, path_text(&self.resource_dir)),
                (&keys::RESOURCE_SNAPSHOT, text(self.snapshot.as_deref())),
                (&keys::LAUNCHER_JOBS, Some(Value::Integer(jobs))),
                (&keys::DEV_WINDOW, Some(Value::Flag(self.developer))),
                (&keys::LAUNCHER_GAME, path_text(&self.game)),
                (&keys::UPDATE_SOURCE, text(self.update_source.as_deref())),
            ],
        )
        .map_err(|error| error.to_string())
    }

    /// The resource directory in effect.
    pub fn resource_dir(&self) -> Result<PathBuf, String> {
        match &self.resource_dir {
            Some(dir) => Ok(dir.clone()),
            None => location::default_resource_dir(),
        }
    }
}

/// `value`, unless it is empty, as the value of an optional setting.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

/// The resources a game started without configuration reads: the launcher's
/// directory and its selected (or newest) complete install. `None` when
/// nothing has been installed there.
pub fn default_selection() -> Result<Option<(PathBuf, Installed)>, String> {
    let config = LauncherConfig::load()?;
    let dir = config.resource_dir()?;
    let record = Record::read(&dir)?;
    Ok(record.select(config.snapshot.as_deref()).cloned().map(|installed| (dir, installed)))
}

/// Loads the configuration file and makes it this program's configuration
/// ([`moly_config::global`]). Each program calls it once, first thing; an
/// unreadable or malformed file is an error that names the file and the key.
pub fn install_program_config() -> Result<(), String> {
    let path = location::config_path()?;
    let config = Config::load(&path).map_err(|error| error.to_string())?;
    moly_config::install(config).map_err(|_| "the configuration was installed twice".to_owned())
}
