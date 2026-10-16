//! Moving a resource directory to another place.
//!
//! On one volume the store's folders and record are renamed into the new
//! directory. Across volumes every file is copied and checked (each store file
//! is named by the SHA-256 of its bytes), the record is copied last, and the
//! old files are removed only after `commit` has switched to the new
//! directory: cancelling or failing before that leaves the old directory
//! complete and in use. Files already copied by an earlier, interrupted move
//! are kept.

use crate::install::{parallel, Phase, Progress, CANCELLED};
use crate::record::RECORD_FILE;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The folders a resource directory holds besides its record.
const FOLDERS: [&str; 3] = ["catalogs", "packages", "blobs"];

/// Move the resources in `from` to `to`, then call `commit` to start using
/// `to`. `jobs` files are copied at a time when the move crosses volumes.
pub fn migrate(
    from: &Path,
    to: &Path,
    jobs: usize,
    progress: &Progress,
    cancel: &AtomicBool,
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let outcome = run(from, to, jobs, progress, cancel, commit);
    progress.finish(&outcome);
    outcome
}

fn run(
    from: &Path,
    to: &Path,
    jobs: usize,
    progress: &Progress,
    cancel: &AtomicBool,
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    for dir in [from, to] {
        if !dir.is_absolute() {
            return Err(format!("{} is not a full path", dir.display()));
        }
    }
    let from = std::path::absolute(from).map_err(|error| format!("{}: {error}", from.display()))?;
    let to = std::path::absolute(to).map_err(|error| format!("{}: {error}", to.display()))?;
    check_target(&from, &to)?;
    let mut names: Vec<&str> = FOLDERS.iter().copied().filter(|name| from.join(name).is_dir()).collect();
    if from.join(RECORD_FILE).is_file() {
        names.push(RECORD_FILE);
    }
    if names.is_empty() {
        return Err(format!("{} holds no resources", from.display()));
    }
    std::fs::create_dir_all(&to).map_err(|error| format!("{}: {error}", to.display()))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    let free = names.iter().all(|name| !to.join(name).exists());
    if free && same_volume(&from, &to) {
        rename_all(&from, &to, &names, progress, commit)
    } else {
        copy_all(&from, &to, &names, jobs, progress, cancel, commit)
    }
}

/// The new directory must differ from the old one and lie outside its folders.
fn check_target(from: &Path, to: &Path) -> Result<(), String> {
    // Windows paths compare without case.
    let key = |path: &Path| {
        if cfg!(windows) {
            PathBuf::from(path.to_string_lossy().to_lowercase())
        } else {
            path.to_path_buf()
        }
    };
    let (from_key, to_key) = (key(from), key(to));
    if from_key == to_key {
        return Err("the new resource directory is the current one".into());
    }
    if FOLDERS.iter().any(|name| to_key.starts_with(from_key.join(name))) {
        return Err(format!("{} lies inside the current resource directory's files", to.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn same_volume(a: &Path, b: &Path) -> bool {
    use std::path::Component;
    let prefix = |path: &Path| match path.components().next() {
        Some(Component::Prefix(prefix)) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
        _ => None,
    };
    prefix(a).is_some() && prefix(a) == prefix(b)
}

#[cfg(unix)]
fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

#[cfg(not(any(windows, unix)))]
fn same_volume(_: &Path, _: &Path) -> bool {
    false
}

fn rename_all(
    from: &Path,
    to: &Path,
    names: &[&str],
    progress: &Progress,
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    progress.enter(Phase::Moving, names.len() as u64);
    let undo = |moved: &[&str]| {
        for name in moved.iter().rev() {
            let _ = std::fs::rename(to.join(name), from.join(name));
        }
    };
    let mut moved = Vec::new();
    for name in names {
        if let Err(error) = std::fs::rename(from.join(name), to.join(name)) {
            undo(&moved);
            return Err(format!("{}: {error}", from.join(name).display()));
        }
        moved.push(*name);
        progress.files_done.fetch_add(1, Ordering::Relaxed);
    }
    if let Err(error) = commit() {
        undo(&moved);
        return Err(error);
    }
    let _ = std::fs::remove_dir(from);
    Ok(())
}

fn copy_all(
    from: &Path,
    to: &Path,
    names: &[&str],
    jobs: usize,
    progress: &Progress,
    cancel: &AtomicBool,
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let mut files = Vec::new();
    for name in names.iter().filter(|name| **name != RECORD_FILE) {
        list(from, Path::new(name), &mut files)?;
    }
    let record = names.contains(&RECORD_FILE);
    progress.enter(Phase::Moving, files.len() as u64 + u64::from(record));
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(files.iter().map(|(_, size)| size).sum(), Ordering::Relaxed);
    parallel(jobs, files.len(), cancel, |index| {
        let (relative, size) = &files[index];
        copy_checked(from, to, relative, *size, progress)
    })?;
    if record {
        let source = from.join(RECORD_FILE);
        let bytes = std::fs::read(&source).map_err(|error| format!("{}: {error}", source.display()))?;
        write_new(&to.join(RECORD_FILE), &bytes)?;
        progress.files_done.fetch_add(1, Ordering::Relaxed);
    }
    commit()?;
    let mut leftover = Vec::new();
    for name in names {
        let path = from.join(name);
        let removed = if *name == RECORD_FILE { std::fs::remove_file(&path) } else { std::fs::remove_dir_all(&path) };
        if let Err(error) = removed {
            leftover.push(format!("{}: {error}", path.display()));
        }
    }
    let _ = std::fs::remove_dir(from);
    if !leftover.is_empty() {
        progress.set_note(format!("moved; some old files could not be removed: {}", leftover.join("; ")));
    }
    Ok(())
}

/// Every regular file below `dir/relative`, with its size. Links are skipped,
/// so nothing outside the directory is read; so are unfinished `.part` files.
fn list(dir: &Path, relative: &Path, out: &mut Vec<(PathBuf, u64)>) -> Result<(), String> {
    let path = dir.join(relative);
    let entries = std::fs::read_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
        let kind = entry.file_type().map_err(|error| format!("{}: {error}", entry.path().display()))?;
        let child = relative.join(entry.file_name());
        if kind.is_dir() {
            list(dir, &child, out)?;
        } else if kind.is_file() && !entry.file_name().to_string_lossy().ends_with(".part") {
            let size = entry.metadata().map_err(|error| format!("{}: {error}", entry.path().display()))?.len();
            out.push((child, size));
        }
    }
    Ok(())
}

/// The SHA-256 a store file is named by.
fn named_digest(relative: &Path) -> Option<String> {
    let name = relative.file_name()?.to_str()?;
    let stem = name.strip_suffix(".bin").or_else(|| name.strip_suffix(".json"))?;
    crate::is_sha256(stem).then(|| stem.to_owned())
}

fn copy_checked(from: &Path, to: &Path, relative: &Path, size: u64, progress: &Progress) -> Result<(), String> {
    let (source, target) = (from.join(relative), to.join(relative));
    let done = |progress: &Progress| {
        progress.files_done.fetch_add(1, Ordering::Relaxed);
    };
    if std::fs::metadata(&target).is_ok_and(|meta| meta.is_file() && meta.len() == size) {
        progress.bytes_done.fetch_add(size, Ordering::Relaxed);
        done(progress);
        return Ok(());
    }
    let parent = target.parent().ok_or_else(|| format!("{} has no parent", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let part = part_path(&target);
    let copied = (|| -> std::io::Result<String> {
        let mut input = std::fs::File::open(&source)?;
        let mut output = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read])?;
            progress.bytes_done.fetch_add(read as u64, Ordering::Relaxed);
            progress.bytes_fetched.fetch_add(read as u64, Ordering::Relaxed);
        }
        output.flush()?;
        Ok(hex(&hasher.finalize()))
    })();
    let digest = match copied {
        Ok(digest) => digest,
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{}: {error}", source.display()));
        }
    };
    if named_digest(relative).is_some_and(|expected| expected != digest) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: the file does not match its SHA-256", source.display()));
    }
    std::fs::rename(&part, &target).map_err(|error| format!("{}: {error}", target.display()))?;
    done(progress);
    Ok(())
}

fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let part = part_path(path);
    std::fs::write(&part, bytes)
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) {
        let blob = b"blob bytes";
        let sha = crate::sha256_hex(blob);
        let blob_path = dir.join("blobs").join(&sha[..2]).join(format!("{sha}.bin"));
        std::fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
        std::fs::write(&blob_path, blob).unwrap();
        let manifest = b"{}";
        let digest = crate::sha256_hex(manifest);
        std::fs::create_dir_all(dir.join("packages")).unwrap();
        std::fs::write(dir.join("packages").join(format!("{digest}.json")), manifest).unwrap();
        std::fs::write(dir.join(RECORD_FILE), b"{\"schema\":1,\"snapshots\":[]}").unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moly-store-migrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copying_moves_every_file_and_commits_before_removing() {
        let root = scratch("copy");
        let (from, to) = (root.join("old"), root.join("new"));
        store(&from);
        let mut files = Vec::new();
        for name in ["blobs", "packages"] {
            list(&from, Path::new(name), &mut files).unwrap();
        }
        // The target already holds one folder, so this takes the copy path.
        std::fs::create_dir_all(to.join("blobs")).unwrap();
        let progress = Progress::default();
        let committed = std::cell::Cell::new(false);
        migrate(&from, &to, 2, &progress, &AtomicBool::new(false), || {
            assert!(from.join(RECORD_FILE).is_file(), "the old record stays until commit");
            assert!(to.join(RECORD_FILE).is_file(), "the new record is written before commit");
            committed.set(true);
            Ok(())
        })
        .unwrap();
        assert!(committed.get());
        for (relative, _) in &files {
            assert!(to.join(relative).is_file(), "{} was copied", relative.display());
        }
        assert!(!from.exists(), "the old directory is removed once empty");
        assert!(matches!(progress.phase(), Phase::Done));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_commit_keeps_the_old_directory() {
        let root = scratch("commit");
        let (from, to) = (root.join("old"), root.join("new"));
        store(&from);
        std::fs::create_dir_all(to.join("blobs")).unwrap();
        let error = migrate(&from, &to, 2, &Progress::default(), &AtomicBool::new(false), || Err("no".into()));
        assert_eq!(error, Err("no".into()));
        assert!(from.join(RECORD_FILE).is_file() && from.join("blobs").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_store_file_stops_the_copy() {
        let root = scratch("corrupt");
        let (from, to) = (root.join("old"), root.join("new"));
        store(&from);
        let bad = "ab".repeat(32);
        std::fs::write(from.join("packages").join(format!("{bad}.json")), b"not it").unwrap();
        std::fs::create_dir_all(to.join("packages")).unwrap();
        let outcome = migrate(&from, &to, 1, &Progress::default(), &AtomicBool::new(false), || Ok(()));
        assert!(outcome.unwrap_err().contains("does not match its SHA-256"));
        assert!(from.join(RECORD_FILE).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_on_one_volume_moves_the_folders() {
        let root = scratch("rename");
        let (from, to) = (root.join("old"), root.join("new"));
        store(&from);
        migrate(&from, &to, 2, &Progress::default(), &AtomicBool::new(false), || Ok(())).unwrap();
        assert!(to.join("blobs").is_dir() && to.join(RECORD_FILE).is_file());
        assert!(!from.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_target_must_differ_and_lie_outside_the_store_folders() {
        let from = Path::new("/x/moly");
        assert!(check_target(from, Path::new("/x/moly")).is_err());
        assert!(check_target(from, Path::new("/x/moly/blobs/inner")).is_err());
        assert!(check_target(from, Path::new("/y/moly")).is_ok());
    }

    #[test]
    fn relative_directories_are_refused() {
        let absolute = std::env::temp_dir();
        let outcome = migrate(Path::new("old"), &absolute, 1, &Progress::default(), &AtomicBool::new(false), || Ok(()));
        assert!(outcome.unwrap_err().contains("is not a full path"));
    }
}
