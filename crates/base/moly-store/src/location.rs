//! Where moly keeps its files on this machine.
//!
//! Files sit under one home directory, which holds the folders `data` (the
//! launcher's configuration, the game settings and the saves), `logs` and
//! `versions`. The resource directory is `resources` in the home directory
//! unless the launcher configuration names another one
//! ([`default_resource_dir`]). The locations file ([`locations_path`]) can give
//! the home directory and each of the data, logs and versions folders an
//! absolute path of its own; a folder without one sits in the home directory.
//! [`default_home`] chooses the home directory when none is saved, and
//! [`settle`] saves that choice. `MOLY_DATA_DIR` overrides the data directory
//! when it is set.

use std::path::{Path, PathBuf};

/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "MOLY_DATA_DIR";

/// The moly data directory: `MOLY_DATA_DIR` when it is set and not empty,
/// otherwise the data folder of the saved [`Locations`].
pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(DATA_DIR_ENV) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    Locations::load()?.data_dir()
}

/// The folders of a moly installation as saved in the locations file. A folder
/// that is not set sits in the home directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Locations {
    /// The layout version: 1 when saved; a document without the field reads as 0.
    pub schema: u32,
    /// The home directory; [`default_home`] when not set.
    pub home: Option<PathBuf>,
    /// The data directory; `data` in the home directory when not set.
    pub data: Option<PathBuf>,
    /// The logs directory; `logs` in the home directory when not set.
    pub logs: Option<PathBuf>,
    /// The versions directory; `versions` in the home directory when not set.
    pub versions: Option<PathBuf>,
}

impl Locations {
    /// The saved locations, or the defaults when no locations file exists.
    pub fn load() -> Result<Self, String> {
        let path = locations_path()?;
        match std::fs::read(&path) {
            Ok(bytes) => Self::parse(&bytes).map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// Writes the locations file as schema 1, through a `.part` sibling that is
    /// renamed into place. Every path that is set must be absolute.
    pub fn save(&self) -> Result<(), String> {
        self.check_paths()?;
        let path = locations_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let saved = Locations { schema: 1, ..self.clone() };
        let text = serde_json::to_string_pretty(&saved).map_err(|error| error.to_string())?;
        let part = path.with_extension("json.part");
        let written = std::fs::write(&part, text + "\n").and_then(|()| std::fs::rename(&part, &path));
        written.map_err(|error| {
            let _ = std::fs::remove_file(&part);
            format!("{}: {error}", path.display())
        })
    }

    /// The home directory: the saved one, else [`default_home`].
    pub fn home(&self) -> Result<PathBuf, String> {
        match &self.home {
            Some(home) => Ok(home.clone()),
            None => default_home(),
        }
    }

    /// The data directory: the saved one, else `data` in the home directory.
    pub fn data_dir(&self) -> Result<PathBuf, String> {
        self.folder(self.data.as_deref(), "data")
    }

    /// The logs directory: the saved one, else `logs` in the home directory.
    pub fn logs_dir(&self) -> Result<PathBuf, String> {
        self.folder(self.logs.as_deref(), "logs")
    }

    /// The versions directory: the saved one, else `versions` in the home directory.
    pub fn versions_dir(&self) -> Result<PathBuf, String> {
        self.folder(self.versions.as_deref(), "versions")
    }

    /// The folder that is set explicitly, else `name` in the home directory.
    fn folder(&self, explicit: Option<&Path>, name: &str) -> Result<PathBuf, String> {
        match explicit {
            Some(dir) => Ok(dir.to_path_buf()),
            None => Ok(self.home()?.join(name)),
        }
    }

    /// Fails when a path that is set is relative.
    fn check_paths(&self) -> Result<(), String> {
        let named = [
            ("home", &self.home),
            ("data", &self.data),
            ("logs", &self.logs),
            ("versions", &self.versions),
        ];
        for (name, path) in named {
            if let Some(path) = path.as_deref().filter(|path| !path.is_absolute()) {
                return Err(format!("the {name} directory must be a full path: {}", path.display()));
            }
        }
        Ok(())
    }

    /// Reads a locations document. Its schema must be 0 or 1, and its paths absolute.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let locations: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if !matches!(locations.schema, 0 | 1) {
            return Err(format!("unsupported schema {}", locations.schema));
        }
        locations.check_paths()?;
        Ok(locations)
    }
}

/// The locations file: `moly\locations.json` under `%LOCALAPPDATA%` on Windows,
/// and `moly/locations.json` under `$XDG_CONFIG_HOME` (or `~/.config`) elsewhere.
pub fn locations_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        Ok(local_app_data()?.join("moly").join("locations.json"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        xdg_base("XDG_CONFIG_HOME", ".config")
            .map(|base| base.join("moly").join("locations.json"))
            .ok_or_else(|| "User configuration directory is unavailable".to_owned())
    }
}

/// The launcher's webview profile: the `webview` folder beside the locations file,
/// outside the folders that can be moved.
pub fn webview_dir() -> Result<PathBuf, String> {
    let path = locations_path()?;
    path.parent()
        .map(|dir| dir.join("webview"))
        .ok_or_else(|| format!("{} has no parent folder", path.display()))
}

/// Loads the locations at launcher startup. When no locations file exists yet,
/// the home directory is set to [`default_home`] and saved, so that it stays the
/// same when free space on the drives changes.
pub fn settle() -> Result<Locations, String> {
    let path = locations_path()?;
    let exists = path.try_exists().map_err(|error| format!("{}: {error}", path.display()))?;
    if exists {
        return Locations::load();
    }
    let locations = Locations { schema: 1, home: Some(default_home()?), ..Locations::default() };
    locations.save()?;
    Ok(locations)
}

/// The home directory used when none is saved. On Windows it is `moly` at the
/// root of the fixed drive, other than the system drive, with the most free
/// space, or `%LOCALAPPDATA%\moly` when there is no such drive. Elsewhere it is
/// `$XDG_DATA_HOME/moly`, or `~/.local/share/moly`.
pub fn default_home() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(root) = drives::roomiest_other_fixed_drive() {
            return Ok(root.join("moly"));
        }
        Ok(local_app_data()?.join("moly"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        xdg_base("XDG_DATA_HOME", ".local/share")
            .map(|base| base.join("moly"))
            .ok_or_else(|| "User data directory is unavailable".to_owned())
    }
}

/// The resource directory used when the launcher configuration names none:
/// `resources` in the home directory.
pub fn default_resource_dir() -> Result<PathBuf, String> {
    Ok(Locations::load()?.home()?.join("resources"))
}

/// The resource directory to use for a folder the user picked: the folder
/// itself when it is empty, already holds moly resources or is named `moly`;
/// otherwise a `moly` folder inside it, so picking `E:\` gives `E:\moly`.
pub fn complete_resource_dir(picked: &Path) -> PathBuf {
    let named = picked.file_name().is_some_and(|name| name.eq_ignore_ascii_case("moly"));
    let holds = picked.join(crate::record::RECORD_FILE).is_file() || picked.join("catalogs").is_dir();
    let empty = std::fs::read_dir(picked).map(|mut entries| entries.next().is_none()).unwrap_or(true);
    if named || holds || (empty && picked.parent().is_some()) {
        picked.to_path_buf()
    } else {
        picked.join("moly")
    }
}

#[cfg(target_os = "windows")]
mod drives {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};

    const DRIVE_FIXED: u32 = 3;

    fn system_letter() -> u8 {
        std::env::var("SystemDrive")
            .ok()
            .and_then(|value| value.bytes().next())
            .map(|letter| letter.to_ascii_uppercase())
            .unwrap_or(b'C')
    }

    /// The root of the fixed drive, other than the system drive, with the most
    /// free space; `None` when there is no such drive.
    pub fn roomiest_other_fixed_drive() -> Option<PathBuf> {
        let system = system_letter();
        // SAFETY: no arguments; returns a bit mask of the present drives.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|index| mask & (1 << index) != 0)
            .map(|index| b'A' + index)
            .filter(|letter| *letter != system)
            .filter_map(|letter| {
                let root = format!("{}:\\", letter as char);
                let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain([0]).collect();
                // SAFETY: `wide` is a NUL-terminated path that outlives both calls,
                // and the out pointer is a live local.
                let fixed = unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_FIXED;
                let mut free = 0u64;
                let null = std::ptr::null_mut();
                let known = fixed && unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, null, null) } != 0;
                known.then(|| (free, PathBuf::from(root)))
            })
            .max_by_key(|(free, _)| *free)
            .map(|(_, root)| root)
    }
}

/// The launcher's configuration file, in the data directory.
pub fn launcher_config_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("launcher.json"))
}

/// The logs directory of the saved [`Locations`].
pub fn logs_dir() -> Result<PathBuf, String> {
    Locations::load()?.logs_dir()
}

/// The versions directory of the saved [`Locations`].
pub fn versions_dir() -> Result<PathBuf, String> {
    Locations::load()?.versions_dir()
}

/// The data folder of the layout that kept data beside the program: on Windows,
/// the `data` folder beside the running program; elsewhere `$XDG_DATA_HOME/moly`
/// (or `~/.local/share/moly`). `None` when that location cannot be determined.
pub fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join("data"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        xdg_base("XDG_DATA_HOME", ".local/share").map(|base| base.join("moly"))
    }
}

/// `%LOCALAPPDATA%`, which must be set and not empty.
#[cfg(target_os = "windows")]
fn local_app_data() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "The local application data directory is unavailable".to_owned())
}

/// `$variable` when it is set and not empty, else `$HOME/fallback`.
#[cfg(not(target_os = "windows"))]
fn xdg_base(variable: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(fallback))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_without_an_explicit_place_sit_in_the_home_directory() {
        let home = PathBuf::from("moly-home");
        let locations = Locations { home: Some(home.clone()), ..Locations::default() };
        assert_eq!(locations.data_dir().unwrap(), home.join("data"));
        assert_eq!(locations.logs_dir().unwrap(), home.join("logs"));
        assert_eq!(locations.versions_dir().unwrap(), home.join("versions"));
    }

    #[test]
    fn an_explicit_folder_takes_precedence_over_the_home_directory() {
        let locations = Locations {
            home: Some(PathBuf::from("moly-home")),
            data: Some(PathBuf::from("elsewhere")),
            ..Locations::default()
        };
        assert_eq!(locations.data_dir().unwrap(), PathBuf::from("elsewhere"));
        assert_eq!(locations.logs_dir().unwrap(), PathBuf::from("moly-home").join("logs"));
    }

    #[test]
    fn a_locations_document_has_schema_zero_or_one_and_absolute_paths() {
        let absolute = if cfg!(windows) { r"C:\moly" } else { "/srv/moly" };
        let document = serde_json::json!({ "schema": 1, "home": absolute }).to_string();
        assert!(Locations::parse(document.as_bytes()).is_ok());
        assert!(Locations::parse(b"{}").is_ok());
        assert!(Locations::parse(br#"{"schema": 2}"#).is_err());
        assert!(Locations::parse(b"not json").is_err());
        assert!(Locations::parse(br#"{"data": "relative/data"}"#).is_err());
    }
}
