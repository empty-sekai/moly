//! The launcher's configuration (`launcher.json` in the data directory), and
//! the resource selection a game started without configuration uses.

use crate::location;
use crate::record::{Installed, Record};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LauncherConfig {
    pub schema: u32,
    /// The resource CDN base, ending with `/`.
    pub cdn: String,
    /// Where resources are downloaded; the default directory when absent.
    pub resource_dir: Option<PathBuf>,
    /// The snapshot id the game starts with; the newest install when absent.
    pub snapshot: Option<String>,
    /// Parallel transfers while downloading.
    pub jobs: usize,
    /// Start the game's developer window (`MOLY_DEV=1`).
    pub developer: bool,
    /// The game program; the one beside the launcher when absent.
    pub game: Option<PathBuf>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            cdn: crate::DEFAULT_CDN.into(),
            resource_dir: None,
            snapshot: None,
            jobs: 16,
            developer: false,
            game: None,
        }
    }
}

impl LauncherConfig {
    /// The saved configuration, or the defaults when none is saved.
    pub fn load() -> Result<Self, String> {
        let path = location::launcher_config_path()?;
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        crate::check_cdn(&self.cdn)?;
        if let Some(dir) = self.resource_dir.as_deref().filter(|dir| !dir.is_absolute()) {
            return Err(format!("the resource directory must be a full path: {}", dir.display()));
        }
        let path = location::launcher_config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let part = path.with_extension("json.part");
        std::fs::write(&part, text + "\n")
            .and_then(|()| std::fs::rename(&part, &path))
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The resource directory in effect.
    pub fn resource_dir(&self) -> Result<PathBuf, String> {
        match &self.resource_dir {
            Some(dir) => Ok(dir.clone()),
            None => location::default_resource_dir(),
        }
    }
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
