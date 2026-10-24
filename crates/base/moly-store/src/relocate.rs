//! Moving a folder tree to another place.
//!
//! Within one volume the folder is renamed. Across volumes it is copied into a
//! `.part` sibling of the target, which is then renamed into place; the source
//! is removed only once the copy is complete. Symbolic links are never
//! followed: the copy refuses them, and a rename moves them unchanged.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Moves the folder tree at `from` to `to`.
///
/// `to` must be absent or an empty folder. Neither folder may lie inside the
/// other, and paths may not contain `..`. When `from` does not exist, `to` is
/// created empty. A failed copy leaves `from` in place. When `from` cannot be
/// removed after its copy is complete, the error names both folders.
pub fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    let (from_key, to_key) = (key(&physical(from)?), key(&physical(to)?));
    if from_key == to_key {
        return Err(format!("{} and {} are the same folder", from.display(), to.display()));
    }
    if to_key.starts_with(&from_key) {
        return Err(format!("{} lies inside {}", to.display(), from.display()));
    }
    if from_key.starts_with(&to_key) {
        return Err(format!("{} lies inside {}", from.display(), to.display()));
    }
    match std::fs::read_dir(to) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(format!("{} is not empty", to.display()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("{}: {error}", to.display())),
    }
    let meta = match std::fs::symlink_metadata(from) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return std::fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()));
        }
        Err(error) => return Err(format!("{}: {error}", from.display())),
    };
    if meta.file_type().is_symlink() {
        return Err(format!("{}: symbolic links are not moved", from.display()));
    }
    if !meta.is_dir() {
        return Err(format!("{} is not a folder", from.display()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    // The target is absent or empty at this point, so it can be replaced.
    let _ = std::fs::remove_dir(to);
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_then_remove(from, to)
}

/// Copies the tree at `from` into a `.part` sibling of `to`, renames that sibling
/// to `to` and then removes `from`. A failed copy removes the sibling and leaves
/// `from` in place.
fn copy_then_remove(from: &Path, to: &Path) -> Result<(), String> {
    let part = part_path(to);
    remove_tree(&part);
    if let Err(error) = copy_tree(from, &part) {
        remove_tree(&part);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&part, to) {
        remove_tree(&part);
        return Err(format!("{}: {error}", to.display()));
    }
    std::fs::remove_dir_all(from).map_err(|error| {
        format!("{}: copied to {}, but not removed: {error}", from.display(), to.display())
    })
}

/// Copies the folder tree at `from` to `to`, which must not exist. Symbolic links
/// are refused, as are entries that are neither files nor folders.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir(to).map_err(|error| format!("{}: {error}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", from.display()))?;
        let source = entry.path();
        let kind = entry.file_type().map_err(|error| format!("{}: {error}", source.display()))?;
        if kind.is_symlink() {
            return Err(format!("{}: symbolic links are not moved", source.display()));
        }
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target).map_err(|error| format!("{}: {error}", source.display()))?;
        } else {
            return Err(format!("{}: only files and folders are moved", source.display()));
        }
    }
    Ok(())
}

/// The `.part` sibling that a folder is copied into before it is renamed to `path`.
fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Removes the file or folder at `path`, when there is one.
fn remove_tree(path: &Path) {
    let _ = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
}

/// The absolute form of `path`, with the symbolic links in its existing part
/// resolved, so that two paths compare by the folder they name.
fn physical(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if absolute.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(format!("{} must not contain '..'", path.display()));
    }
    let mut missing: Vec<OsString> = Vec::new();
    let mut base = absolute.as_path();
    loop {
        if let Ok(real) = base.canonicalize() {
            return Ok(missing.iter().rev().fold(real, |folder, name| folder.join(name)));
        }
        match (base.file_name(), base.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name.to_os_string());
                base = parent;
            }
            _ => return Ok(absolute.clone()),
        }
    }
}

/// The form in which resolved paths are compared: lowercase on Windows, where
/// file names ignore case.
fn key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A fresh folder under the system temporary directory, unique to this run.
    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("moly-relocate-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn moves_a_tree_into_an_absent_target() {
        let root = scratch("move");
        let (from, to) = (root.join("old"), root.join("new").join("deeper"));
        write(&from.join("a.txt"), "a");
        write(&from.join("sub").join("b.txt"), "b");
        move_dir(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("sub").join("b.txt")).unwrap(), "b");
        assert!(!from.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_a_target_that_is_not_empty() {
        let root = scratch("occupied");
        let (from, to) = (root.join("old"), root.join("new"));
        write(&from.join("a.txt"), "a");
        write(&to.join("keep.txt"), "keep");
        assert!(move_dir(&from, &to).is_err());
        assert!(from.join("a.txt").is_file());
        assert!(to.join("keep.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_folders_nested_in_each_other() {
        let root = scratch("nested");
        let from = root.join("old");
        write(&from.join("a.txt"), "a");
        assert!(move_dir(&from, &from.join("inner")).is_err());
        assert!(move_dir(&from.join("inner"), &from).is_err());
        assert!(from.join("a.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_source_only_creates_the_target() {
        let root = scratch("missing");
        let to = root.join("made").join("here");
        move_dir(&root.join("absent"), &to).unwrap();
        assert!(to.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_copy_path_moves_the_tree() {
        let root = scratch("copy");
        let (from, to) = (root.join("old"), root.join("new"));
        write(&from.join("sub").join("b.txt"), "b");
        copy_then_remove(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("sub").join("b.txt")).unwrap(), "b");
        assert!(!from.exists());
        assert!(!part_path(&to).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_copy_path_keeps_the_source_when_the_target_cannot_be_replaced() {
        let root = scratch("copy-blocked");
        let (from, to) = (root.join("old"), root.join("new"));
        write(&from.join("a.txt"), "a");
        write(&to.join("keep.txt"), "keep");
        assert!(copy_then_remove(&from, &to).is_err());
        assert!(from.join("a.txt").is_file());
        assert!(!part_path(&to).exists());
        assert!(to.join("keep.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
