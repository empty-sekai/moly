//! Installs the newest published program release into a versions folder.
//!
//! A release carries one program package per target, named
//! `moly-<version>-<target>.<ext>` (`.zip` on Windows, `.tar.gz` elsewhere), a
//! `SHA256SUMS` file with the SHA-256 of every asset.
//! Program packages keep their program files in one top-level folder named
//! after the package, `moly-<version>-<target>`; files at the archive root are
//! accepted too.
//! [`latest`] reads the release metadata from the GitHub REST API and [`install`]
//! places the files in `<versions_dir>/<version>`.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ureq::Agent;

use crate::install::CANCELLED;

/// The GitHub API address of the repository that publishes program releases.
pub const DEFAULT_SOURCE: &str = "https://api.github.com/repos/empty-sekai/moly";

/// Program executables that every installed version holds, for the current platform.
#[cfg(windows)]
pub const PROGRAMS: [&str; 3] = ["moly-app.exe", "moly-server.exe", "moly-launcher.exe"];
/// Program executables that every installed version holds, for the current platform.
#[cfg(not(windows))]
pub const PROGRAMS: [&str; 3] = ["moly-app", "moly-server", "moly-launcher"];

/// Documents copied along with the program files when a seed folder holds them.
const DOCUMENTS: [&str; 3] = ["LICENSE", "README.md", "THIRD_PARTY_NOTICES.md"];

/// Target triple and package extension of each published platform.
const TARGETS: [(&str, &str); 3] = [
    ("x86_64-pc-windows-msvc", "zip"),
    ("x86_64-unknown-linux-gnu", "tar.gz"),
    ("aarch64-apple-darwin", "tar.gz"),
];

const CHECKSUMS: &str = "SHA256SUMS";
/// Largest asset accepted for download.
const MAX_ASSET_BYTES: u64 = 2 << 30;
const MAX_CHECKSUMS_BYTES: u64 = 1 << 20;
const MAX_API_BYTES: u64 = 4 << 20;
const BUFFER_BYTES: usize = 1 << 16;

/// The target triple and package extension of this build's platform, or `None`
/// when no program package is published for it.
pub fn target() -> Option<(&'static str, &'static str)> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(TARGETS[0])
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(TARGETS[1])
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(TARGETS[2])
    } else {
        None
    }
}

/// A downloadable file attached to a release.
#[derive(Clone, Debug, Serialize)]
pub struct Asset {
    /// File name of the asset.
    pub name: String,
    /// Address the bytes are downloaded from.
    pub url: String,
    /// Size in bytes, as reported by the release.
    pub size: u64,
}

/// The newest published release.
#[derive(Clone, Debug, Serialize)]
pub struct Release {
    /// Version without the tag prefix, such as `0.1.0`.
    pub version: String,
    /// Tag name as published, such as `v0.1.0`.
    pub tag: String,
    /// Address of the release page.
    pub page: String,
    /// Release notes, empty when the release has none.
    pub notes: String,
    /// Full program package for the platform.
    pub package: Asset,
    /// The `SHA256SUMS` asset, which lists the SHA-256 of every other asset.
    pub checksums: Asset,
}

/// Progress of [`install`]. The counters describe the current stage. `bytes_total`
/// is 0 when the size of the stage is unknown.
#[derive(Default)]
pub struct Progress {
    /// Bytes handled in the current stage.
    pub bytes_done: AtomicU64,
    /// Size of the current stage in bytes, or 0 when unknown.
    pub bytes_total: AtomicU64,
    /// Current stage: `checking`, `downloading`, `extracting` or `done`.
    pub stage: Mutex<String>,
}

impl Progress {
    fn set_stage(&self, stage: &str, total: u64) {
        self.bytes_done.store(0, Ordering::Relaxed);
        self.bytes_total.store(total, Ordering::Relaxed);
        if let Ok(mut current) = self.stage.lock() {
            current.clear();
            current.push_str(stage);
        }
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Reads the newest release of the repository at `source`, such as
/// [`DEFAULT_SOURCE`], and selects the assets for this platform.
///
/// The asset names are part of the publishing contract: the package is
/// `moly-<version>-<target>.<ext>` and the checksums file is `SHA256SUMS`.
pub fn latest(source: &str) -> Result<Release, String> {
    let platform = target().ok_or_else(|| "no program package is published for this platform".to_string())?;
    if !source.starts_with("https://") {
        return Err(format!("release source must be an https address: {source}"));
    }
    let url = format!("{}/releases/latest", source.trim_end_matches('/'));
    let agent = agent();
    let mut response = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("release lookup failed: {e}"))?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_API_BYTES)
        .read_to_vec()
        .map_err(|e| format!("release lookup failed: {e}"))?;
    parse_release(&body, platform)
}

fn parse_release(body: &[u8], platform: (&str, &str)) -> Result<Release, String> {
    let (triple, ext) = platform;
    let api: ApiRelease =
        serde_json::from_slice(body).map_err(|e| format!("release metadata: {e}"))?;
    let version = api.tag_name.strip_prefix('v').unwrap_or(api.tag_name.as_str()).to_string();
    semver::Version::parse(&version)
        .map_err(|e| format!("release tag {} is not a version: {e}", api.tag_name))?;
    let package_name = format!("moly-{version}-{triple}.{ext}");
    let find = |name: &str| api.assets.iter().find(|asset| asset.name == name);
    let package = find(&package_name)
        .ok_or_else(|| format!("release {} has no {package_name}", api.tag_name))?;
    let checksums = find(CHECKSUMS)
        .ok_or_else(|| format!("release {} has no {CHECKSUMS}", api.tag_name))?;
    if checksums.size > MAX_CHECKSUMS_BYTES {
        return Err(format!("{CHECKSUMS} is larger than expected"));
    }
    Ok(Release {
        version,
        tag: api.tag_name.clone(),
        page: api.html_url.clone(),
        notes: api.body.clone().unwrap_or_default(),
        package: asset_of(package)?,
        checksums: asset_of(checksums)?,
    })
}

fn asset_of(asset: &ApiAsset) -> Result<Asset, String> {
    if !asset.browser_download_url.starts_with("https://") {
        return Err(format!("{} has no https download address", asset.name));
    }
    if asset.size > MAX_ASSET_BYTES {
        return Err(format!("{} is larger than the download limit", asset.name));
    }
    Ok(Asset {
        name: asset.name.clone(),
        url: asset.browser_download_url.clone(),
        size: asset.size,
    })
}

/// Whether `candidate` is a newer version than `current`, by semantic version
/// precedence. Returns `false` when either string is not a version.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (semver::Version::parse(candidate), semver::Version::parse(current)) {
        (Ok(candidate), Ok(current)) => candidate > current,
        _ => false,
    }
}

/// The complete version folders in `versions_dir`, newest first, as
/// (version, folder).
///
/// A folder counts when its name is a semantic version and it holds every file in
/// [`PROGRAMS`]. A missing directory yields an empty list.
pub fn installed(versions_dir: &Path) -> Vec<(String, PathBuf)> {
    version_folders(versions_dir)
        .into_iter()
        .filter(|(_, _, folder)| is_complete(folder))
        .map(|(_, name, folder)| (name, folder))
        .collect()
}

/// Every subfolder of `dir` whose name is a semantic version, newest first.
fn version_folders(dir: &Path) -> Vec<(semver::Version, String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders: Vec<(semver::Version, String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let version = semver::Version::parse(&name).ok()?;
            Some((version, name, entry.path()))
        })
        .collect();
    folders.sort_by(|a, b| b.0.cmp(&a.0));
    folders
}

fn is_complete(folder: &Path) -> bool {
    PROGRAMS.iter().all(|program| folder.join(program).is_file())
}

/// The top-level folder of the program package of `version` for `triple`.
fn package_root(version: &str, triple: &str) -> String {
    format!("moly-{version}-{triple}")
}

/// `name` without the leading folder `root`. A name outside `root` is returned
/// unchanged, and `root` itself becomes the empty name.
fn strip_root<'a>(name: &'a str, root: &str) -> &'a str {
    if name == root {
        return "";
    }
    name.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(name)
}

/// Checks that `folder` holds every program and marks the programs executable.
fn finish_programs(folder: &Path) -> Result<(), String> {
    check_programs(folder)?;
    #[cfg(unix)]
    for program in PROGRAMS {
        use std::os::unix::fs::PermissionsExt;
        let path = folder.join(program);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("mark {} executable: {e}", path.display()))?;
    }
    Ok(())
}

fn check_programs(folder: &Path) -> Result<(), String> {
    match PROGRAMS.iter().copied().find(|program| !folder.join(program).is_file()) {
        Some(missing) => Err(format!("the package has no {missing}")),
        None => Ok(()),
    }
}

/// Removes version folders that are no longer needed.
///
/// The `keep` newest complete version folders are kept, and so is every folder
/// that contains a path in `in_use`. All other complete folders and all incomplete
/// version folders are removed. Returns the removed folders, newest first.
pub fn prune(versions_dir: &Path, keep: usize, in_use: &[&Path]) -> Result<Vec<PathBuf>, String> {
    let mut kept = 0;
    let mut removed = Vec::new();
    for (_, _, folder) in version_folders(versions_dir) {
        if in_use.iter().any(|used| in_tree(used, &folder)) {
            continue;
        }
        if is_complete(&folder) && kept < keep {
            kept += 1;
            continue;
        }
        fs::remove_dir_all(&folder).map_err(|e| format!("remove {}: {e}", folder.display()))?;
        removed.push(folder);
    }
    Ok(removed)
}

/// Whether `used` lies inside `folder`. Both paths are canonicalized when possible.
fn in_tree(used: &Path, folder: &Path) -> bool {
    let used = fs::canonicalize(used).unwrap_or_else(|_| used.to_path_buf());
    let folder = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    used.starts_with(folder)
}

/// Seeds `<versions_dir>/<version>` with the program files found in `program_dir`,
/// the folder the launcher runs from, so that the installed versions have a first
/// complete folder.
///
/// Every file in [`PROGRAMS`] must be present in `program_dir`. The documents
/// `LICENSE`, `README.md` and `THIRD_PARTY_NOTICES.md` are copied when present.
/// Returns the new folder, or `None` when the version folder is already complete.
pub fn bootstrap(
    versions_dir: &Path,
    program_dir: &Path,
    version: &str,
) -> Result<Option<PathBuf>, String> {
    semver::Version::parse(version).map_err(|e| format!("{version} is not a version: {e}"))?;
    let folder = versions_dir.join(version);
    if is_complete(&folder) {
        return Ok(None);
    }
    check_programs(program_dir)?;
    fs::create_dir_all(versions_dir)
        .map_err(|e| format!("create {}: {e}", versions_dir.display()))?;
    let staging = versions_dir.join(format!(".{version}.partial"));
    reset_dir(&staging)?;
    if let Err(error) = copy_seed(program_dir, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if folder.exists() {
        fs::remove_dir_all(&folder)
            .map_err(|e| format!("remove incomplete {}: {e}", folder.display()))?;
    }
    fs::rename(&staging, &folder)
        .map_err(|e| format!("move {} into place: {e}", staging.display()))?;
    Ok(Some(folder))
}

fn copy_seed(program_dir: &Path, staging: &Path) -> Result<(), String> {
    for name in PROGRAMS.iter().copied().chain(DOCUMENTS) {
        let source = program_dir.join(name);
        if source.is_file() {
            fs::copy(&source, staging.join(name)).map_err(|e| format!("copy {name}: {e}"))?;
        }
    }
    Ok(())
}

/// Installs `release` into `<versions_dir>/<version>` and returns that folder.
///
/// When the folder already holds every file in [`PROGRAMS`], nothing is downloaded.
/// Otherwise the full package is installed. Every downloaded asset is checked
/// against `SHA256SUMS`, and the folder appears only once all files are in place.
///
/// `cancel` is polled between chunks. Setting it makes the call return
/// `Err("cancelled")`.
pub fn install(
    release: &Release,
    versions_dir: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let platform =
        target().ok_or_else(|| "no program package is published for this platform".to_string())?;
    let agent = agent();
    install_with(
        release,
        versions_dir,
        progress,
        cancel,
        platform,
        &|asset: &Asset, dest: &Path| download(&agent, asset, dest, progress, cancel),
    )
}

/// [`install`] with the platform and the asset fetcher supplied by the caller.
fn install_with(
    release: &Release,
    versions_dir: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
    platform: (&'static str, &'static str),
    fetch: &dyn Fn(&Asset, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    semver::Version::parse(&release.version)
        .map_err(|e| format!("release version {} is not a version: {e}", release.version))?;
    let folder = versions_dir.join(&release.version);
    if is_complete(&folder) {
        return Ok(folder);
    }
    let (triple, ext) = platform;
    let job = Job {
        release,
        versions_dir,
        progress,
        cancel,
        fetch,
        triple,
        format: format_of(ext)?,
    };
    fs::create_dir_all(versions_dir)
        .map_err(|e| format!("create {}: {e}", versions_dir.display()))?;
    let staging = versions_dir.join(format!(".{}.partial", release.version));
    let download_path = versions_dir.join(format!(".{}.download", release.version));
    progress.set_stage("checking", 0);
    let outcome = populate(&job, &staging, &download_path);
    let _ = fs::remove_file(&download_path);
    if let Err(error) = outcome {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if folder.exists() {
        fs::remove_dir_all(&folder)
            .map_err(|e| format!("remove incomplete {}: {e}", folder.display()))?;
    }
    fs::rename(&staging, &folder)
        .map_err(|e| format!("move {} into place: {e}", staging.display()))?;
    progress.set_stage("done", 0);
    Ok(folder)
}

struct Job<'a> {
    release: &'a Release,
    versions_dir: &'a Path,
    progress: &'a Progress,
    cancel: &'a AtomicBool,
    fetch: &'a dyn Fn(&Asset, &Path) -> Result<(), String>,
    triple: &'static str,
    format: Format,
}

/// Fills `staging` with the files of the release.
fn populate(job: &Job<'_>, staging: &Path, download: &Path) -> Result<(), String> {
    reset_dir(staging)?;
    job.progress.set_stage("downloading", job.release.checksums.size);
    (job.fetch)(&job.release.checksums, download)?;
    let text = fs::read_to_string(download).map_err(|e| format!("read {CHECKSUMS}: {e}"))?;
    let _ = fs::remove_file(download);
    let sums = parse_sums(&text)?;
    fetch_verified(job, &job.release.package, &sums, download)?;
    job.progress.set_stage("extracting", 0);
    let root = package_root(&job.release.version, job.triple);
    extract_archive(job.format, download, &root, staging, job.progress, job.cancel)?;
    finish_programs(staging)
}

/// Downloads `asset` to `dest` and checks it against its `SHA256SUMS` entry.
fn fetch_verified(
    job: &Job<'_>,
    asset: &Asset,
    sums: &BTreeMap<String, String>,
    dest: &Path,
) -> Result<(), String> {
    let expected = sums
        .get(&asset.name)
        .ok_or_else(|| format!("{CHECKSUMS} has no entry for {}", asset.name))?;
    job.progress.set_stage("downloading", asset.size);
    (job.fetch)(asset, dest)?;
    if hash_file(dest)? != *expected {
        let _ = fs::remove_file(dest);
        return Err(format!("{} failed its SHA-256 check", asset.name));
    }
    Ok(())
}

/// Extracts every entry of the package `archive` below `dest`, without the
/// package's top-level folder `root`.
fn extract_archive(
    format: Format,
    archive: &Path,
    root: &str,
    dest: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<(), String> {
    for_each_entry(archive, format, |name, kind, reader| {
        let name = strip_root(name, root);
        if name.is_empty() {
            return Ok(());
        }
        let target = dest.join(name);
        match kind {
            EntryKind::Dir => {
                fs::create_dir_all(&target).map_err(|e| format!("create {}: {e}", target.display()))
            }
            EntryKind::File => {
                create_parent(&target)?;
                let mut writer = File::create(&target)
                    .map_err(|e| format!("create {}: {e}", target.display()))?;
                copy_hashed(reader, &mut writer, progress, cancel).map(|_| ())
            }
        }
    })
}

/// Streams the asset at its download address into `dest`.
fn download(
    agent: &Agent,
    asset: &Asset,
    dest: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if !asset.url.starts_with("https://") {
        return Err(format!("{} has no https download address", asset.name));
    }
    let mut response = agent
        .get(&asset.url)
        .call()
        .map_err(|e| format!("download {}: {e}", asset.name))?;
    let mut body = response.body_mut().with_config().limit(MAX_ASSET_BYTES).reader();
    let mut file = File::create(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    copy_hashed(&mut body, &mut file, progress, cancel)?;
    file.sync_all().map_err(|e| format!("write {}: {e}", dest.display()))
}

/// Parses `SHA256SUMS` lines of the form `<sha256>  <name>` or `<sha256> *<name>`.
fn parse_sums(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut sums = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let malformed = || format!("malformed {CHECKSUMS} line: {line}");
        let (digest, rest) = line.split_once(' ').ok_or_else(malformed)?;
        let name = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*')).unwrap_or(rest);
        if !crate::is_sha256(digest) || name.is_empty() {
            return Err(malformed());
        }
        if sums.insert(name.to_string(), digest.to_string()).is_some() {
            return Err(format!("{CHECKSUMS} lists {name} twice"));
        }
    }
    Ok(sums)
}

/// The lowercase hex SHA-256 of the file at `path`.
fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if count == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

/// Copies `source` to `target` in chunks, checking `cancel` between chunks and adding
/// each chunk to `progress`. Returns the lowercase hex SHA-256 of the copied bytes.
fn copy_hashed(
    source: &mut dyn Read,
    target: &mut dyn Write,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        check_cancel(cancel)?;
        let count = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
        target.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
        progress.bytes_done.fetch_add(count as u64, Ordering::Relaxed);
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.to_string())
    } else {
        Ok(())
    }
}

fn create_parent(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(parent) => {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))
        }
        None => Ok(()),
    }
}

fn reset_dir(dir: &Path) -> Result<(), String> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))
}

fn format_of(ext: &str) -> Result<Format, String> {
    match ext {
        "zip" => Ok(Format::Zip),
        "tar.gz" => Ok(Format::TarGz),
        other => Err(format!("unsupported package format: {other}")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    TarGz,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
}

/// Calls `visit` for each entry of the archive, in archive order. Directory entries
/// are passed with [`EntryKind::Dir`]. The entry of the archive root is skipped.
fn for_each_entry(
    archive: &Path,
    format: Format,
    mut visit: impl FnMut(&str, EntryKind, &mut dyn Read) -> Result<(), String>,
) -> Result<(), String> {
    let open = || File::open(archive).map_err(|e| format!("open {}: {e}", archive.display()));
    match format {
        Format::Zip => {
            let mut reader = zip::ZipArchive::new(BufReader::new(open()?))
                .map_err(|e| format!("read {}: {e}", archive.display()))?;
            for index in 0..reader.len() {
                let mut entry = reader
                    .by_index(index)
                    .map_err(|e| format!("read {}: {e}", archive.display()))?;
                let path = entry
                    .enclosed_name()
                    .ok_or_else(|| format!("unsafe entry name in {}", archive.display()))?;
                let name = entry_name(&path)?;
                if name.is_empty() {
                    continue;
                }
                let kind = if entry.is_dir() { EntryKind::Dir } else { EntryKind::File };
                visit(&name, kind, &mut entry)?;
            }
        }
        Format::TarGz => {
            let decoder = flate2::read::GzDecoder::new(BufReader::new(open()?));
            let mut reader = tar::Archive::new(decoder);
            let entries = reader
                .entries()
                .map_err(|e| format!("read {}: {e}", archive.display()))?;
            for entry in entries {
                let mut entry = entry.map_err(|e| format!("read {}: {e}", archive.display()))?;
                let kind = match entry.header().entry_type() {
                    entry_type if entry_type.is_dir() => EntryKind::Dir,
                    entry_type if entry_type.is_file() => EntryKind::File,
                    _ => return Err(format!("unsupported entry type in {}", archive.display())),
                };
                let name = {
                    let path = entry
                        .path()
                        .map_err(|e| format!("read {}: {e}", archive.display()))?;
                    entry_name(&path)?
                };
                if name.is_empty() {
                    continue;
                }
                visit(&name, kind, &mut entry)?;
            }
        }
    }
    Ok(())
}

/// The archive name of `path`: its normal components joined with `/`. Current-directory
/// components are dropped and any other component is an error, so an empty name is the
/// archive root.
fn entry_name(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| format!("non-UTF-8 entry name: {}", path.display()))?,
            ),
            Component::CurDir => {}
            _ => return Err(format!("unsafe entry name: {}", path.display())),
        }
    }
    Ok(parts.join("/"))
}

fn zip_options(method: zip::CompressionMethod) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(method)
}

fn agent() -> Agent {
    Agent::config_builder()
        .tls_config(crate::tls::tls_config())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(3600)))
        .user_agent(concat!("moly-store/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// Replaces the running launcher executable with `new_launcher`.
///
/// The running process keeps its code, and the new file runs the next time the
/// launcher starts. The replacement is performed by the `self-replace` crate.
pub fn replace_launcher(new_launcher: &Path) -> Result<(), String> {
    self_replace::self_replace(new_launcher)
        .map_err(|e| format!("replace the launcher with {}: {e}", new_launcher.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const PLATFORM: (&str, &str) = ("x86_64-pc-windows-msvc", "zip");
    const NEW_PACKAGE: &str = "moly-0.0.2-x86_64-pc-windows-msvc.zip";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moly-update-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The program files of one version. `app` is the body of the first program.
    fn sample(app: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = PROGRAMS
            .iter()
            .map(|name| (name.to_string(), format!("{name} body").into_bytes()))
            .collect();
        files[0].1 = app.to_vec();
        files.push(("README.md".to_string(), b"readme".to_vec()));
        files
    }

    /// Writes a zip whose entries sit below `folder`, or at the root when it is empty.
    fn write_package_in(path: &Path, folder: &str, files: &[(String, Vec<u8>)]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        if !folder.is_empty() {
            writer
                .add_directory(folder, zip_options(zip::CompressionMethod::Stored))
                .unwrap();
        }
        for (name, bytes) in files {
            let name = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
            writer
                .start_file(name.as_str(), zip_options(zip::CompressionMethod::Deflated))
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn install_seed(versions: &Path, version: &str, files: &[(String, Vec<u8>)]) {
        let folder = versions.join(version);
        fs::create_dir_all(&folder).unwrap();
        for (name, bytes) in files {
            fs::write(folder.join(name), bytes).unwrap();
        }
    }

    fn asset(name: &str, path: &Path) -> Asset {
        Asset {
            name: name.to_string(),
            url: format!("https://example.invalid/{name}"),
            size: fs::metadata(path).unwrap().len(),
        }
    }

    /// A fetcher that serves the named files from disk and records each request.
    fn serve<'a>(
        served: &'a HashMap<String, PathBuf>,
        requests: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&Asset, &Path) -> Result<(), String> + 'a {
        move |asset: &Asset, dest: &Path| -> Result<(), String> {
            requests.borrow_mut().push(asset.name.clone());
            let source = served
                .get(&asset.name)
                .ok_or_else(|| format!("{} is not served", asset.name))?;
            fs::copy(source, dest).map(|_| ()).map_err(|e| e.to_string())
        }
    }

    /// Writes a `SHA256SUMS` file covering the given (name, file) pairs.
    fn checksums(dir: &Path, files: &[(&str, &Path)]) -> PathBuf {
        let mut text = String::new();
        for (name, path) in files {
            text.push_str(&format!("{}  {name}\n", hash_file(path).unwrap()));
        }
        let path = dir.join(CHECKSUMS);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn install_downloads_the_package_and_unpacks_its_folder() {
        let dir = scratch("install");
        let old = sample(b"old app");
        let mut new = sample(&[7u8; 4096]);
        new.push(("NOTICE.txt".to_string(), b"notice".to_vec()));
        let new_pkg = dir.join(NEW_PACKAGE);
        write_package_in(&new_pkg, "moly-0.0.2-x86_64-pc-windows-msvc", &new);
        let versions = dir.join("versions");
        install_seed(&versions, "0.0.1", &old);
        let sums = checksums(&dir, &[(NEW_PACKAGE, new_pkg.as_path())]);
        let served = HashMap::from([
            (NEW_PACKAGE.to_string(), new_pkg.clone()),
            (CHECKSUMS.to_string(), sums.clone()),
        ]);
        let requests = RefCell::new(Vec::new());
        let release = Release {
            version: "0.0.2".to_string(),
            tag: "v0.0.2".to_string(),
            page: String::new(),
            notes: String::new(),
            package: asset(NEW_PACKAGE, &new_pkg),
            checksums: asset(CHECKSUMS, &sums),
        };
        let folder = install_with(
            &release,
            &versions,
            &Progress::default(),
            &AtomicBool::new(false),
            PLATFORM,
            &serve(&served, &requests),
        )
        .unwrap();
        assert_eq!(folder, versions.join("0.0.2"));
        assert_eq!(*requests.borrow(), [CHECKSUMS.to_string(), NEW_PACKAGE.to_string()]);
        assert_eq!(fs::read(folder.join(PROGRAMS[0])).unwrap(), new[0].1);
        assert_eq!(fs::read(folder.join("NOTICE.txt")).unwrap(), b"notice");
        assert!(!folder.join("moly-0.0.2-x86_64-pc-windows-msvc").exists());
    }

    #[test]
    fn release_metadata_selects_the_platform_assets() {
        let body = br#"{
            "tag_name": "v0.0.2",
            "html_url": "https://example.invalid/release",
            "body": "notes",
            "assets": [
                {"name": "SHA256SUMS", "browser_download_url": "https://example.invalid/sums", "size": 10},
                {"name": "moly-0.0.2-x86_64-pc-windows-msvc.zip", "browser_download_url": "https://example.invalid/pkg", "size": 20}
            ]
        }"#;
        let release = parse_release(body, PLATFORM).unwrap();
        assert_eq!(release.version, "0.0.2");
        assert_eq!(release.package.size, 20);
        assert_eq!(release.checksums.name, CHECKSUMS);
    }

    #[test]
    fn checksum_lines_accept_text_and_binary_markers() {
        let text = format!("{}  a.zip\n{} *b.zip\n", "0".repeat(64), "1".repeat(64));
        let sums = parse_sums(&text).unwrap();
        assert_eq!(sums["a.zip"], "0".repeat(64));
        assert_eq!(sums["b.zip"], "1".repeat(64));
        assert!(parse_sums("zz  a.zip").is_err());
    }

    #[test]
    fn installed_versions_and_prune_follow_the_retention_rules() {
        let dir = scratch("prune");
        for version in ["0.0.2", "0.0.10", "0.0.9"] {
            install_seed(&dir, version, &sample(b"app"));
        }
        fs::create_dir_all(dir.join("0.0.11")).unwrap();
        let listed: Vec<String> = installed(&dir).into_iter().map(|(version, _)| version).collect();
        assert_eq!(listed, ["0.0.10", "0.0.9", "0.0.2"]);
        assert!(is_newer("0.0.10", "0.0.9"));
        assert!(!is_newer("0.0.9", "0.0.9"));
        let in_use = dir.join("0.0.2").join(PROGRAMS[0]);
        let removed = prune(&dir, 1, &[in_use.as_path()]).unwrap();
        assert_eq!(removed, vec![dir.join("0.0.11"), dir.join("0.0.9")]);
    }
}
