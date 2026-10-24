//! Taking over an earlier install's settings, saves and resource directory.
//!
//! On Windows the launcher records the data directories it runs from in a
//! per-user list. [`find_previous`] reads that list, the `data` folders beside
//! the program folder on Windows, and the directory of
//! [`location::legacy_data_dir`].
//! [`import_previous`] copies the launcher configuration, the game settings and
//! the saves of an earlier data directory into the current one. Nothing under
//! the earlier data directory is changed.

use crate::config::LauncherConfig;
use crate::location;
use crate::record::RECORD_FILE;
use std::path::{Path, PathBuf};

/// The launcher's configuration file in a data directory.
const CONFIG_FILE: &str = "launcher.json";
/// The game settings file in a data directory.
const SETTINGS_FILE: &str = "settings.json";
/// The folder of saves: the server's player documents, in a data directory.
const SAVES_DIR: &str = "server";
/// The most data directories the per-user list holds.
#[cfg(any(target_os = "windows", test))]
const MAX_INSTALLS: usize = 16;

/// Records the current data directory in the per-user list that
/// [`find_previous`] reads. Only Windows keeps the list; elsewhere the data
/// directory already belongs to the user, so nothing is recorded.
pub fn remember_install() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let Some(file) = installs_file() else {
            return Ok(());
        };
        let current = location::data_dir()?;
        let old = read_installs(&file)?;
        let list = updated_list(&old, &current, |dir: &Path| dir.is_dir());
        if list != old {
            write_installs(&file, list)?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// The data directory of an earlier install, or `None` when there is none.
/// Candidates are the directories in the per-user list, the `data` folders beside
/// the program folder on Windows, and [`location::legacy_data_dir`]. A candidate
/// qualifies when it is not the current data directory and holds a
/// `launcher.json`; the qualifying candidate whose `launcher.json` was modified
/// most recently is returned.
pub fn find_previous() -> Option<PathBuf> {
    let current = location::data_dir().ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        candidates.extend(
            installs_file()
                .and_then(|file| read_installs(&file).ok())
                .unwrap_or_default(),
        );
        candidates.extend(sibling_data_dirs());
    }
    candidates.extend(location::legacy_data_dir());
    candidates
        .into_iter()
        .filter(|dir| !same_dir(dir, &current))
        .filter_map(|dir| {
            let meta = std::fs::metadata(dir.join(CONFIG_FILE)).ok().filter(|meta| meta.is_file())?;
            Some((meta.modified().ok()?, dir))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, dir)| dir)
}

/// What [`import_previous`] took over from an earlier install.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Imported {
    /// The earlier data directory.
    pub from: PathBuf,
    /// The resource directory the new configuration uses, when it has one.
    pub resource_dir: Option<PathBuf>,
    /// Whether `settings.json` was copied.
    pub settings: bool,
    /// Whether the saves (the `server` folder) were copied.
    pub saves: bool,
}

/// Takes over the configuration, the game settings and the saves of the earlier
/// data directory `from`, into the current data directory, which must not hold a
/// `launcher.json` yet.
///
/// The resource directory comes from the earlier configuration; when that names
/// none, `from/resources` is used if it holds installed resources. A game program
/// inside the earlier program folder is dropped. The settings and the saves are
/// copied only when the current data directory lacks them, and each copy is
/// renamed into place once complete. The configuration is written last, so a
/// failed import leaves no `launcher.json` and can run again.
pub fn import_previous(from: &Path) -> Result<Imported, String> {
    let here = location::data_dir()?;
    let config_path = location::launcher_config_path()?;
    if config_path.exists() {
        return Err(format!("{} already exists", config_path.display()));
    }
    let source = from.join(CONFIG_FILE);
    let bytes = std::fs::read(&source).map_err(|error| format!("{}: {error}", source.display()))?;
    let mut config: LauncherConfig =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", source.display()))?;
    if config.resource_dir.is_none() {
        let resources = from.join("resources");
        if resources.join(RECORD_FILE).is_file() {
            config.resource_dir = Some(resources);
        }
    }
    let inside_old_program = match (config.game.as_deref(), from.parent()) {
        (Some(game), Some(program)) => game.starts_with(program),
        _ => false,
    };
    if inside_old_program {
        config.game = None;
    }

    std::fs::create_dir_all(&here).map_err(|error| format!("{}: {error}", here.display()))?;
    let settings_source = from.join(SETTINGS_FILE);
    let settings_target = here.join(SETTINGS_FILE);
    let settings = settings_source.is_file() && !settings_target.exists();
    if settings {
        stage(&settings_target, |part| std::fs::copy(&settings_source, part).map(|_| ()))?;
    }
    let saves_source = from.join(SAVES_DIR);
    let saves_target = here.join(SAVES_DIR);
    let saves = saves_source.is_dir() && !saves_target.exists();
    if saves {
        stage(&saves_target, |part| copy_tree(&saves_source, part))?;
    }

    config.save()?;
    Ok(Imported {
        from: from.to_path_buf(),
        resource_dir: config.resource_dir,
        settings,
        saves,
    })
}

/// The `data` folders beside the program folder: for each folder in the parent
/// of the program folder, its `data` folder and the `data` folder of each folder
/// directly inside it.
#[cfg(target_os = "windows")]
fn sibling_data_dirs() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(parent) = exe.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for dir in subdirectories(parent) {
        found.push(dir.join("data"));
        for child in subdirectories(&dir) {
            found.push(child.join("data"));
        }
    }
    found
}

/// The folders directly inside `dir`; empty when it cannot be read.
#[cfg(target_os = "windows")]
fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// The per-user list, `installs.json` under `%LOCALAPPDATA%\moly`.
#[cfg(target_os = "windows")]
fn installs_file() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(local).join("moly").join("installs.json"))
}

/// The file layout of the per-user list.
#[cfg(target_os = "windows")]
#[derive(serde::Serialize, serde::Deserialize)]
struct InstallList {
    schema: u32,
    data_dirs: Vec<PathBuf>,
}

/// The data directories in the per-user list; empty when there is no list.
#[cfg(target_os = "windows")]
fn read_installs(file: &Path) -> Result<Vec<PathBuf>, String> {
    match std::fs::read(file) {
        Ok(bytes) => {
            let list: InstallList =
                serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;
            if list.schema != 1 {
                return Err(format!("{} has schema {}", file.display(), list.schema));
            }
            Ok(list.data_dirs)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}: {error}", file.display())),
    }
}

/// Writes the per-user list, creating its folder when needed.
#[cfg(target_os = "windows")]
fn write_installs(file: &Path, data_dirs: Vec<PathBuf>) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&InstallList { schema: 1, data_dirs })
        .map_err(|error| error.to_string())?;
    stage(file, |part| std::fs::write(part, text + "\n"))
}

/// The list after `current` is recorded: `current` first, then the entries of
/// `old` that still exist, without repeats, and at most `MAX_INSTALLS` in all.
#[cfg(any(target_os = "windows", test))]
fn updated_list(old: &[PathBuf], current: &Path, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut list = vec![current.to_path_buf()];
    for dir in old {
        if list.len() >= MAX_INSTALLS {
            break;
        }
        if exists(dir.as_path()) && !list.iter().any(|kept| same_dir(kept, dir)) {
            list.push(dir.clone());
        }
    }
    list
}

/// Whether two directory paths name the same folder, ignoring case and trailing
/// separators.
fn same_dir(a: &Path, b: &Path) -> bool {
    dir_key(a) == dir_key(b)
}

fn dir_key(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches(|c: char| c == '\\' || c == '/').to_lowercase()
}

/// Builds `to` with `build`, which writes to a `.part` sibling. That sibling is
/// renamed to `to` when `build` succeeds and removed when it fails.
fn stage(to: &Path, build: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<(), String> {
    let part = part_path(to);
    remove_path(&part);
    let built = build(part.as_path()).and_then(|()| std::fs::rename(&part, to));
    built.map_err(|error| {
        remove_path(&part);
        format!("{}: {error}", to.display())
    })
}

/// The `.part` sibling that a file or folder is built in before it is renamed.
fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Removes the file or folder at `path`, when there is one.
fn remove_path(path: &Path) {
    let _ = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
}

/// Copies the folder tree at `from` to `to`, which must not exist yet.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_comparison_ignores_case_and_trailing_separators() {
        assert!(same_dir(Path::new(r"C:\Games\Moly\data\"), Path::new(r"c:\games\moly\data")));
        assert!(!same_dir(Path::new(r"C:\Games\moly\data"), Path::new(r"C:\Games\moly\data2")));
    }

    #[test]
    fn installs_list_starts_with_the_current_directory() {
        let current = PathBuf::from(r"C:\new\data");
        let old = vec![
            PathBuf::from(r"C:\old\data"),
            PathBuf::from(r"C:\NEW\data\"),
            PathBuf::from(r"C:\gone\data"),
            PathBuf::from(r"C:\old\data"),
        ];
        let list = updated_list(&old, &current, |dir: &Path| !dir.to_string_lossy().contains("gone"));
        assert_eq!(list, vec![PathBuf::from(r"C:\new\data"), PathBuf::from(r"C:\old\data")]);
    }

    #[test]
    fn installs_list_holds_at_most_sixteen_directories() {
        let current = PathBuf::from(r"C:\current\data");
        let old: Vec<PathBuf> = (0..30).map(|index| PathBuf::from(format!(r"C:\old{index}\data"))).collect();
        let list = updated_list(&old, &current, |_: &Path| true);
        assert_eq!(list.len(), 16);
        assert_eq!(list[0], current);
    }
}
