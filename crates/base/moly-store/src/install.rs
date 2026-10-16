//! Mirror one published catalog into a local resource directory.
//!
//! The catalog, every package manifest and every blob are fetched to the
//! same relative paths they have in the store. A file already present is
//! kept when its SHA-256 matches, so an interrupted install resumes and a
//! repeated one verifies. A 403, 429 or 5xx answer, or a network failure,
//! waits and retries with a doubling delay. Files land under a `.part` name
//! and are renamed once complete.

use crate::publication::{Publication, Snapshot, PUBLICATION_PATH};
use crate::record::{Installed, Record};
use crate::store::{catalog_key, check_key, manifest_digest, Catalog, Manifest};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;

/// Documents are small; anything larger than this is refused.
const DOCUMENT_LIMIT: u64 = 64 << 20;
const ATTEMPTS: u32 = 12;
const FIRST_DELAY: Duration = Duration::from_secs(5);
const LONGEST_DELAY: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Idle = 0,
    Catalog = 1,
    Manifests = 2,
    Blobs = 3,
    Done = 4,
    Failed = 5,
    Cancelled = 6,
    /// Moving an installed directory to another place.
    Moving = 7,
}

/// Counters an install updates as it goes; read them from any thread.
#[derive(Default)]
pub struct Progress {
    phase: AtomicU8,
    /// Files of the current phase finished (present or fetched).
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    /// Bytes of blobs finished, out of `bytes_total`.
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    /// Bytes actually transferred.
    pub bytes_fetched: AtomicU64,
    pub retries: AtomicU64,
    last_note: Mutex<String>,
}

impl Progress {
    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Catalog,
            2 => Phase::Manifests,
            3 => Phase::Blobs,
            4 => Phase::Done,
            5 => Phase::Failed,
            6 => Phase::Cancelled,
            7 => Phase::Moving,
            _ => Phase::Idle,
        }
    }

    pub(crate) fn enter(&self, phase: Phase, files: u64) {
        self.files_done.store(0, Ordering::Relaxed);
        self.files_total.store(files, Ordering::Relaxed);
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    /// The most recent retry or failure note.
    pub fn note(&self) -> String {
        self.last_note.lock().map(|note| note.clone()).unwrap_or_default()
    }

    pub(crate) fn set_note(&self, note: String) {
        if let Ok(mut slot) = self.last_note.lock() {
            *slot = note;
        }
    }

    /// Record how a job ended.
    pub(crate) fn finish<T>(&self, outcome: &Result<T, String>) {
        match outcome {
            Ok(_) => self.enter(Phase::Done, 0),
            Err(error) if error == CANCELLED => self.phase.store(Phase::Cancelled as u8, Ordering::Relaxed),
            Err(error) => {
                self.set_note(error.clone());
                self.phase.store(Phase::Failed as u8, Ordering::Relaxed);
            }
        }
    }
}

fn agent() -> Agent {
    Agent::config_builder()
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(300)))
        .user_agent(concat!("moly-store/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

struct Fetcher<'a> {
    agent: Agent,
    base: &'a str,
    progress: &'a Progress,
    cancel: &'a AtomicBool,
}

impl Fetcher<'_> {
    fn sleep(&self, delay: Duration) -> Result<(), String> {
        let until = Instant::now() + delay;
        while Instant::now() < until {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }

    /// The bytes at `key`, retried on transient failures.
    fn get(&self, key: &str, limit: u64) -> Result<Vec<u8>, String> {
        let url = format!("{}{key}", self.base);
        let mut delay = FIRST_DELAY;
        for _ in 0..ATTEMPTS {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let failure = match self.agent.get(&url).header("Accept-Encoding", "identity").call() {
                Ok(mut response) => match response.body_mut().with_config().limit(limit).read_to_vec() {
                    Ok(bytes) => {
                        self.progress.bytes_fetched.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        return Ok(bytes);
                    }
                    Err(error) => format!("{key}: {error}"),
                },
                Err(ureq::Error::StatusCode(code)) if code == 403 || code == 429 || code >= 500 => {
                    format!("{key}: HTTP {code}")
                }
                Err(ureq::Error::StatusCode(code)) => return Err(format!("{url}: HTTP {code}")),
                Err(error) => format!("{key}: {error}"),
            };
            self.progress.retries.fetch_add(1, Ordering::Relaxed);
            self.progress
                .set_note(format!("{failure}; retrying in {} s", delay.as_secs()));
            self.sleep(delay)?;
            delay = (delay * 2).min(LONGEST_DELAY);
        }
        Err(format!("{url}: gave up after {ATTEMPTS} attempts"))
    }

    /// The file at `key` below `dir`: kept when its digest is `sha`,
    /// otherwise fetched, checked and written.
    fn file(&self, dir: &Path, key: &str, sha: &str, limit: u64, size: Option<u64>) -> Result<Vec<u8>, String> {
        check_key(key)?;
        let path = dir.join(key);
        if let Ok(meta) = std::fs::metadata(&path) {
            if size.is_none_or(|size| meta.len() == size) {
                if let Ok(bytes) = std::fs::read(&path) {
                    if crate::sha256_hex(&bytes) == sha {
                        return Ok(bytes);
                    }
                }
            }
        }
        let bytes = self.get(key, limit)?;
        if crate::sha256_hex(&bytes) != sha {
            return Err(format!("{key}: the downloaded bytes do not match their SHA-256"));
        }
        write_atomic(&path, &bytes)?;
        Ok(bytes)
    }
}

pub(crate) const CANCELLED: &str = "cancelled";

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| format!("{} has no parent", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    std::fs::write(&part, bytes)
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Read the publication document of `cdn`.
pub fn fetch_publication(cdn: &str) -> Result<Publication, String> {
    crate::check_cdn(cdn)?;
    let progress = Progress::default();
    let cancel = AtomicBool::new(false);
    let fetcher = Fetcher { agent: agent(), base: cdn, progress: &progress, cancel: &cancel };
    // A publication answers at once or not at all: one attempt, no backoff.
    let url = format!("{cdn}{PUBLICATION_PATH}");
    let mut response = fetcher
        .agent
        .get(&url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(DOCUMENT_LIMIT)
        .read_to_vec()
        .map_err(|error| format!("{url}: {error}"))?;
    Publication::parse(&bytes)
}

/// The catalog of `snapshot`: the copy in `dir` when it is intact, otherwise
/// downloaded into `dir` (where an install puts it anyway).
pub fn catalog(cdn: &str, prefix: &str, snapshot: &Snapshot, dir: &Path) -> Result<Catalog, String> {
    crate::check_cdn(cdn)?;
    let id = snapshot
        .asset_catalog
        .as_deref()
        .filter(|id| crate::is_sha256(id))
        .ok_or_else(|| format!("snapshot {} has no asset catalog", snapshot.id))?;
    let base = snapshot.store_base(cdn, prefix)?;
    let progress = Progress::default();
    let cancel = AtomicBool::new(false);
    let fetcher = Fetcher { agent: agent(), base: &base, progress: &progress, cancel: &cancel };
    Catalog::parse(&fetcher.file(dir, &catalog_key(id), id, DOCUMENT_LIMIT, None)?)
}

/// What to install and where.
pub struct Job {
    pub cdn: String,
    pub prefix: String,
    pub snapshot: Snapshot,
    pub dir: PathBuf,
    /// Parallel transfers.
    pub jobs: usize,
}

/// Install `job`, updating `progress`; setting `cancel` stops it at the next
/// file. On success the directory's record names the snapshot.
pub fn install(job: &Job, progress: &Progress, cancel: &AtomicBool) -> Result<Installed, String> {
    let outcome = run(job, progress, cancel);
    progress.finish(&outcome);
    outcome
}

fn run(job: &Job, progress: &Progress, cancel: &AtomicBool) -> Result<Installed, String> {
    crate::check_cdn(&job.cdn)?;
    let catalog_id = job
        .snapshot
        .asset_catalog
        .clone()
        .filter(|id| crate::is_sha256(id))
        .ok_or_else(|| format!("snapshot {} has no asset catalog", job.snapshot.id))?;
    let base = job.snapshot.store_base(&job.cdn, &job.prefix)?;
    std::fs::create_dir_all(&job.dir).map_err(|error| format!("{}: {error}", job.dir.display()))?;
    let fetcher = Fetcher { agent: agent(), base: &base, progress, cancel };

    progress.enter(Phase::Catalog, 1);
    let catalog = Catalog::parse(&fetcher.file(&job.dir, &catalog_key(&catalog_id), &catalog_id, DOCUMENT_LIMIT, None)?)?;
    progress.files_done.store(1, Ordering::Relaxed);

    progress.enter(Phase::Manifests, catalog.packages.len() as u64);
    // Manifests are small, so their time is round trips: twice the transfers.
    let manifests = parallel(job.jobs * 2, catalog.packages.len(), cancel, |index| {
        let key = &catalog.packages[index].manifest;
        let digest = manifest_digest(key)?;
        let manifest = Manifest::parse(&fetcher.file(&job.dir, key, digest, DOCUMENT_LIMIT, None)?)?;
        progress.files_done.fetch_add(1, Ordering::Relaxed);
        Ok(manifest)
    })?;

    let mut blobs = std::collections::BTreeMap::new();
    for entry in manifests.iter().flat_map(|manifest| &manifest.entries) {
        blobs.insert(entry.blob.as_str(), (entry.blob_sha256.as_str(), entry.blob_bytes));
    }
    let blobs: Vec<_> = blobs.into_iter().collect();
    let total: u64 = blobs.iter().map(|(_, (_, size))| size).sum();
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(total, Ordering::Relaxed);
    progress.enter(Phase::Blobs, blobs.len() as u64);
    parallel(job.jobs, blobs.len(), cancel, |index| {
        let (blob, (sha, size)) = blobs[index];
        fetcher.file(&job.dir, &format!("blobs/{blob}"), sha, size + 1, Some(size))?;
        progress.files_done.fetch_add(1, Ordering::Relaxed);
        progress.bytes_done.fetch_add(size, Ordering::Relaxed);
        Ok(())
    })?;

    let installed = Installed {
        id: job.snapshot.id.clone(),
        region: job.snapshot.region.clone(),
        version: job.snapshot.version.clone(),
        catalog: catalog_id,
        cdn: job.cdn.clone(),
        installed_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        bytes: total,
    };
    let mut record = Record::read(&job.dir)?;
    record.upsert(installed.clone());
    record.write(&job.dir)?;
    Ok(installed)
}

/// Run `work` for every index on `jobs` threads, in index order of results.
/// The first failure stops the remaining work.
pub(crate) fn parallel<T: Send>(
    jobs: usize,
    count: usize,
    cancel: &AtomicBool,
    work: impl Fn(usize) -> Result<T, String> + Sync,
) -> Result<Vec<T>, String> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let slots: Vec<Mutex<Option<T>>> = (0..count).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, 64).min(count.max(1)) {
            scope.spawn(|| loop {
                if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                    return;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= count {
                    return;
                }
                match work(index) {
                    Ok(value) => *slots[index].lock().unwrap() = Some(value),
                    Err(error) => {
                        stop.store(true, Ordering::Relaxed);
                        failure.lock().unwrap().get_or_insert(error);
                        return;
                    }
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().unwrap() {
        return Err(error);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    Ok(slots.into_iter().map(|slot| slot.into_inner().unwrap().expect("every index ran")).collect())
}

/// True when `error` is the cancellation of an install.
pub fn is_cancelled(error: &str) -> bool {
    error == CANCELLED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_keeps_index_order_and_stops_on_failure() {
        let cancel = AtomicBool::new(false);
        let out = parallel(4, 100, &cancel, |i| Ok(i * 2)).unwrap();
        assert_eq!(out, (0..100).map(|i| i * 2).collect::<Vec<_>>());
        let err = parallel(4, 100, &cancel, |i| if i == 37 { Err("no".to_owned()) } else { Ok(i) });
        assert_eq!(err.unwrap_err(), "no");
        cancel.store(true, Ordering::Relaxed);
        assert!(is_cancelled(&parallel(4, 10, &cancel, Ok).unwrap_err()));
    }

    #[test]
    fn present_files_with_their_digest_are_kept_without_a_request() {
        let dir = std::env::temp_dir().join(format!("moly-store-test-{}", std::process::id()));
        let body = b"blob bytes";
        let sha = crate::sha256_hex(body);
        let key = format!("blobs/{}/{sha}.bin", &sha[..2]);
        write_atomic(&dir.join(&key), body).unwrap();
        let progress = Progress::default();
        let cancel = AtomicBool::new(false);
        // An unroutable base: any request would fail.
        let fetcher = Fetcher { agent: agent(), base: "https://invalid.invalid/", progress: &progress, cancel: &cancel };
        assert_eq!(fetcher.file(&dir, &key, &sha, 100, Some(body.len() as u64)).unwrap(), body);
        assert_eq!(progress.bytes_fetched.load(Ordering::Relaxed), 0);
        cancel.store(true, Ordering::Relaxed);
        let other = format!("blobs/{}/{}.bin", &sha[..2], "0".repeat(64));
        assert!(fetcher.file(&dir, &other, &"0".repeat(64), 100, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
