//! Where moly keeps its files on this machine.
//!
//! Everything lives in one data directory. `MOLY_DATA_DIR` names it when set.
//! Otherwise, on Windows it is the `data` folder beside the running program,
//! so the launcher and the game, shipped in one folder, share it and keep it
//! on whichever drive that folder is on; elsewhere it is `$XDG_DATA_HOME/moly`
//! (or `~/.local/share/moly`). The game's settings documents and the
//! launcher's configuration and logs sit below it; where the downloaded
//! resources go is [`default_resource_dir`] until another directory is chosen.

use std::path::{Path, PathBuf};

/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "MOLY_DATA_DIR";

/// The moly data directory.
pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(DATA_DIR_ENV) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe().map_err(|error| format!("The program location is unavailable: {error}"))?;
        let dir = exe.parent().ok_or_else(|| "The program location has no parent directory".to_owned())?;
        Ok(dir.join("data"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".local/share")))
            .ok_or_else(|| "User data directory is unavailable".to_owned())?;
        Ok(base.join("moly"))
    }
}

/// The resource directory proposed when none is configured: `resources` in
/// the data directory, unless that lies on the Windows system drive; then
/// `moly` at the root of the other fixed drive with the most free space, when
/// there is one. Free space changes, so the launcher stores the directory it
/// proposes once instead of asking again.
pub fn default_resource_dir() -> Result<PathBuf, String> {
    let beside = data_dir()?.join("resources");
    #[cfg(target_os = "windows")]
    if drives::on_system_drive(&beside) {
        if let Some(root) = drives::roomiest_other_fixed_drive() {
            return Ok(root.join("moly"));
        }
    }
    Ok(beside)
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
    use std::path::{Component, Path, PathBuf, Prefix};
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};

    const DRIVE_FIXED: u32 = 3;

    fn letter(path: &Path) -> Option<u8> {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter.to_ascii_uppercase()),
                _ => None,
            },
            _ => None,
        }
    }

    fn system_letter() -> u8 {
        std::env::var("SystemDrive")
            .ok()
            .and_then(|value| value.bytes().next())
            .map(|letter| letter.to_ascii_uppercase())
            .unwrap_or(b'C')
    }

    pub fn on_system_drive(path: &Path) -> bool {
        letter(path) == Some(system_letter())
    }

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

/// The launcher's configuration file.
pub fn launcher_config_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("launcher.json"))
}
