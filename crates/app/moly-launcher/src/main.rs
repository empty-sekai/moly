//! The moly launcher: picks the resource CDN and directory, downloads and
//! verifies resource snapshots, moves the folders of the home directory,
//! installs program updates, and starts the game with them.
//!
//! The page in `ui/` draws everything; this program answers its commands.
//! Settings live in the data directory's `launcher.json`, which the game also
//! reads when it is started without configuration.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use moly_store::config::LauncherConfig;
use moly_store::install::{self, Job, Phase, Progress};
use moly_store::location::Locations;
use moly_store::publication::Publication;
use moly_store::record::{Installed, Record, RECORD_FILE};
use moly_store::{relocate, update};
use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::http::Response;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// The page's font: the game's UI font, served on the `molyfont` scheme.
static FONT: &[u8] = include_bytes!("../../../game/moly-core/assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

#[cfg(windows)]
const GAME_FILE: &str = "moly-app.exe";
#[cfg(not(windows))]
const GAME_FILE: &str = "moly-app";

/// The game plays as two processes: this one runs beside `GAME_FILE` and the
/// game is started with the address it announces.
#[cfg(windows)]
const SERVER_FILE: &str = "moly-server.exe";
#[cfg(not(windows))]
const SERVER_FILE: &str = "moly-server";

/// How long the server may take to announce the address it listens on.
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);

/// The launcher program's file name, beside the other program files of a version.
#[cfg(windows)]
const LAUNCHER_FILE: &str = "moly-launcher.exe";
#[cfg(not(windows))]
const LAUNCHER_FILE: &str = "moly-launcher";

/// The launcher program as it was started. An update may replace the file; a restart
/// starts what is at this path.
static LAUNCHER_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Program versions kept after an update: the newest complete ones.
const KEPT_VERSIONS: usize = 2;

/// The folders that sit in the home directory unless they are set elsewhere.
const MOVED_FOLDERS: [&str; 3] = ["data", "logs", "versions"];

/// A download, a move of the resource directory or a move of a folder; one runs at a time.
struct Download {
    kind: &'static str,
    snapshot: String,
    started: Instant,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
}

impl Download {
    fn running(&self) -> bool {
        self.outcome.lock().unwrap().is_none()
    }

    fn spawn(
        kind: &'static str,
        snapshot: String,
        work: impl FnOnce(&Progress, &AtomicBool) -> Result<(), String> + Send + 'static,
    ) -> Self {
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let outcome = Arc::new(Mutex::new(None));
        std::thread::spawn({
            let (progress, cancel, outcome) = (progress.clone(), cancel.clone(), outcome.clone());
            move || {
                let result = work(&progress, &cancel);
                *outcome.lock().unwrap() = Some(result);
            }
        });
        Download { kind, snapshot, started: Instant::now(), progress, cancel, outcome }
    }
}

/// A program update: the newest release is installed into the versions folder, and
/// the launcher program is replaced when the release carries a different one. One
/// update runs at a time, in the `update` slot of [`Launcher`].
struct Update {
    progress: Arc<update::Progress>,
    cancel: Arc<AtomicBool>,
    /// The version being installed; empty until the release has been read.
    version: Arc<Mutex<String>>,
    launcher_replaced: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
}

impl Update {
    fn running(&self) -> bool {
        self.outcome.lock().unwrap().is_none()
    }

    /// Starts the update on a thread. `current` is the running launcher program.
    fn spawn(source: String, versions: PathBuf, current: PathBuf, config: LauncherConfig) -> Self {
        let job = Update {
            progress: Arc::new(update::Progress::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            version: Arc::new(Mutex::new(String::new())),
            launcher_replaced: Arc::new(AtomicBool::new(false)),
            outcome: Arc::new(Mutex::new(None)),
        };
        std::thread::spawn({
            let (progress, cancel, version, launcher_replaced, outcome) = (
                job.progress.clone(),
                job.cancel.clone(),
                job.version.clone(),
                job.launcher_replaced.clone(),
                job.outcome.clone(),
            );
            move || {
                let result = install_update(
                    &source,
                    &versions,
                    &current,
                    &config,
                    &progress,
                    &cancel,
                    &version,
                    &launcher_replaced,
                );
                if result.as_ref().is_err_and(|error| install::is_cancelled(error)) {
                    *progress.stage.lock().unwrap() = "cancelled".to_owned();
                }
                *outcome.lock().unwrap() = Some(result);
            }
        });
        job
    }
}

/// The server of the running game.
struct Server {
    child: Child,
    #[cfg(windows)]
    job: ServerJob,
}

impl Server {
    /// Ends the server: the game exited or could not start.
    fn stop(&mut self) {
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A Windows job whose processes end when the launcher's own handle closes, so
/// a killed launcher does not leave a server behind.
#[cfg(windows)]
struct ServerJob(isize);

#[cfg(windows)]
impl ServerJob {
    fn new() -> Result<Self, String> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: no attributes and no name; the handle is closed by `Drop`.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let job = ServerJob(handle as isize);
        // SAFETY: a zeroed limits record is valid; only the flags field is set.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        // SAFETY: the pointer and its length describe the record above.
        let set = unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const std::ffi::c_void,
                size,
            )
        };
        if set == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(job)
    }

    fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0 as windows_sys::Win32::Foundation::HANDLE
    }

    /// Puts `child` in this job, so ending the job also ends it.
    fn assign(&self, child: &Child) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: both handles are live for the call.
        let assigned =
            unsafe { AssignProcessToJobObject(self.handle(), child.as_raw_handle() as _) };
        if assigned == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: the handle is live until `Drop`.
        unsafe { TerminateJobObject(self.handle(), 1) };
    }
}

#[cfg(windows)]
impl Drop for ServerJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        // SAFETY: this value owns the handle and closes it once.
        unsafe { CloseHandle(self.handle()) };
    }
}

/// The address in the server's announcing line, if this is that line.
fn announced_address(line: &str) -> Option<String> {
    let (_, address) = line.split_once(" listening on ")?;
    let address = address.trim();
    address.starts_with("http://").then(|| address.to_owned())
}

/// Copies one of the server's streams into the log line by line and reports
/// every line to `each`. The stream is always read: a pipe that fills stops
/// the server. The optional buffer keeps the beginning of its stderr for a
/// start failure.
fn drain(
    stream: impl std::io::Read + Send + 'static,
    mut log: std::fs::File,
    kept: Option<Arc<Mutex<String>>>,
    mut each: impl FnMut(&str) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = line.trim_end();
                    let _ = writeln!(log, "[server] {line}");
                    if let Some(kept) = &kept {
                        let mut kept = kept.lock().unwrap();
                        if kept.len() < 8192 {
                            kept.push_str(line);
                            kept.push('\n');
                        }
                    }
                    each(line);
                }
            }
        }
    })
}

/// Starts the server beside the game, gives its streams to the log and returns
/// it with the address it announced. The server reads the same asset root and
/// developer flag as the game; its documents live below `MOLY_SERVER_DATA`.
fn start_server(
    game: &Path,
    log: &std::fs::File,
    assets: &Path,
    catalog: &str,
    developer: bool,
    data: &Path,
) -> Result<(Server, String), String> {
    let exe = game.with_file_name(SERVER_FILE);
    if !exe.is_file() {
        return Err(format!(
            "the game's server program is missing: {}",
            exe.display()
        ));
    }
    #[cfg(windows)]
    let job = ServerJob::new()
        .map_err(|error| format!("the server cannot be tied to the launcher: {error}"))?;
    let mut command = Command::new(&exe);
    command
        .current_dir(exe.parent().unwrap_or_else(|| Path::new(".")))
        // Port 0 asks the system for a free port; the server announces it.
        .env("MOLY_SERVER_ADDR", "127.0.0.1:0")
        .env("MOLY_ASSET_ROOT", assets)
        .env("MOLY_ASSET_CATALOG", catalog)
        .env("MOLY_DEV", if developer { "1" } else { "0" })
        .env("MOLY_SERVER_DATA", data.join("server"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("{}: {error}", exe.display()))?;
    #[cfg(windows)]
    {
        if let Err(error) = job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("the server cannot be tied to the launcher: {error}"));
        }
    }
    let stdout = child.stdout.take().expect("the server's stdout is piped");
    let stderr = child.stderr.take().expect("the server's stderr is piped");
    let diagnostics = Arc::new(Mutex::new(String::new()));
    let (ready, ready_rx) = mpsc::channel::<String>();
    let out_log = log
        .try_clone()
        .map_err(|error| format!("{}: {error}", exe.display()))?;
    // The server announces its address once; the first line settles the start.
    let mut announced = false;
    let out = drain(stdout, out_log, None, move |line| {
        if let Some(url) = announced_address(line) {
            if !announced {
                announced = true;
                let _ = ready.send(url);
            }
        }
    });
    let err_log = log
        .try_clone()
        .map_err(|error| format!("{}: {error}", exe.display()))?;
    let err = drain(stderr, err_log, Some(diagnostics.clone()), |_| {});
    let failure = |mut child: Child, reason: String| -> String {
        let _ = child.kill();
        let _ = child.wait();
        let _ = out.join();
        let _ = err.join();
        let details = diagnostics.lock().unwrap().trim().to_owned();
        if details.is_empty() {
            reason
        } else {
            format!("{reason}: {details}")
        }
    };
    match ready_rx.recv_timeout(SERVER_START_TIMEOUT) {
        Ok(url) => {
            #[cfg(windows)]
            let server = Server { child, job };
            #[cfg(not(windows))]
            let server = Server { child };
            Ok((server, url))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(failure(
            child,
            "the server exited before it announced its address".to_owned(),
        )),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(failure(
            child,
            format!(
                "the server did not announce its address within {} s",
                SERVER_START_TIMEOUT.as_secs()
            ),
        )),
    }
}

#[derive(Default)]
struct Launcher {
    publication: Mutex<Option<(String, Publication)>>,
    download: Mutex<Option<Download>>,
    update: Mutex<Option<Update>>,
    game: Mutex<Option<(Child, PathBuf)>>,
    server: Mutex<Option<Server>>,
    last_exit: Mutex<Option<String>>,
}

impl Launcher {
    /// Ends the server of the game that is gone. Lock the game first, then the
    /// server: the poll that sees the game exit holds both in that order.
    fn stop_server(&self) {
        if let Some(mut server) = self.server.lock().unwrap().take() {
            server.stop();
        }
    }
}

#[derive(Serialize)]
struct Overview {
    version: &'static str,
    config: LauncherConfig,
    resource_dir: String,
    default_resource_dir: String,
    /// The home directory, which holds the folders that are not set elsewhere.
    home_dir: String,
    data_dir: String,
    /// Set when `MOLY_DATA_DIR` fixes the data directory.
    data_dir_locked: bool,
    logs_dir: String,
    versions_dir: String,
    /// The installed program versions, newest first.
    versions: Vec<String>,
    installed: Vec<Installed>,
    selected: Option<String>,
    game: String,
    game_found: bool,
    /// Set on the first start, when an earlier install was taken over.
    imported: Option<moly_store::upgrade::Imported>,
}

/// The game program: the one set in the configuration, else the program of the newest
/// installed version, else the one beside the launcher.
fn game_path(config: &LauncherConfig) -> PathBuf {
    if let Some(game) = &config.game {
        return game.clone();
    }
    let newest = moly_store::location::versions_dir()
        .ok()
        .and_then(|dir| update::installed(&dir).into_iter().next());
    if let Some((_, folder)) = newest {
        return folder.join(GAME_FILE);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(GAME_FILE)))
        .unwrap_or_else(|| PathBuf::from(GAME_FILE))
}

fn overview_of(config: LauncherConfig) -> Result<Overview, String> {
    let dir = config.resource_dir()?;
    let record = Record::read(&dir)?;
    let selected = record.select(config.snapshot.as_deref()).map(|item| item.id.clone());
    let game = game_path(&config);
    let versions_dir = moly_store::location::versions_dir()?;
    let versions = update::installed(&versions_dir).into_iter().map(|(version, _)| version).collect();
    Ok(Overview {
        version: env!("CARGO_PKG_VERSION"),
        resource_dir: dir.display().to_string(),
        default_resource_dir: moly_store::location::default_resource_dir()?.display().to_string(),
        home_dir: Locations::load()?.home()?.display().to_string(),
        data_dir: moly_store::location::data_dir()?.display().to_string(),
        data_dir_locked: data_dir_locked(),
        logs_dir: moly_store::location::logs_dir()?.display().to_string(),
        versions_dir: versions_dir.display().to_string(),
        versions,
        installed: record.snapshots,
        selected,
        game_found: game.is_file(),
        game: game.display().to_string(),
        config,
        imported: None,
    })
}

/// The launcher's state. The first time, the proposed resource directory is
/// stored, so it stays put when free space on the drives changes. On the first
/// start, the settings, saves and resource directory of an earlier install are
/// taken over when one is found.
#[tauri::command]
fn overview() -> Result<Overview, String> {
    let imported = if moly_store::location::launcher_config_path()?.is_file() {
        None
    } else {
        moly_store::upgrade::find_previous().and_then(|from| moly_store::upgrade::import_previous(&from).ok())
    };
    let mut config = LauncherConfig::load()?;
    if config.resource_dir.is_none() {
        config.resource_dir = Some(moly_store::location::default_resource_dir()?);
        config.save()?;
    }
    let _ = moly_store::upgrade::remember_install();
    let mut view = overview_of(config)?;
    view.imported = imported;
    Ok(view)
}

#[tauri::command]
fn save_config(config: LauncherConfig) -> Result<Overview, String> {
    if config.jobs == 0 || config.jobs > 64 {
        return Err("parallel downloads must be between 1 and 64".into());
    }
    config.save()?;
    overview_of(config)
}

#[derive(Serialize)]
struct SnapshotView {
    id: String,
    region: String,
    version: String,
}

/// The installable snapshots `cdn` publishes.
#[tauri::command]
async fn publication(cdn: String, state: State<'_, Launcher>) -> Result<Vec<SnapshotView>, String> {
    let fetched = tauri::async_runtime::spawn_blocking({
        let cdn = cdn.clone();
        move || install::fetch_publication(&cdn)
    })
    .await
    .map_err(|error| error.to_string())??;
    let views = fetched
        .installable()
        .map(|s| SnapshotView { id: s.id.clone(), region: s.region.clone(), version: s.version.clone() })
        .collect();
    *state.publication.lock().unwrap() = Some((cdn, fetched));
    Ok(views)
}

fn job_for(state: &Launcher, snapshot: &str, config: &LauncherConfig) -> Result<Job, String> {
    let guard = state.publication.lock().unwrap();
    let (cdn, publication) = guard.as_ref().ok_or("the CDN's snapshot list has not been read yet")?;
    let found = publication
        .installable()
        .find(|item| item.id == snapshot)
        .ok_or_else(|| format!("{cdn} does not publish {snapshot}"))?;
    Ok(Job {
        cdn: cdn.clone(),
        prefix: publication.prefix.clone(),
        snapshot: found.clone(),
        dir: config.resource_dir()?,
        jobs: config.jobs,
    })
}

#[derive(Serialize)]
struct Size {
    packages: usize,
    bytes: u64,
    kinds: Vec<(String, u64)>,
}

/// The size of a snapshot, from its catalog.
#[tauri::command]
async fn snapshot_size(snapshot: String, state: State<'_, Launcher>) -> Result<Size, String> {
    let job = job_for(&state, &snapshot, &LauncherConfig::load()?)?;
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = install::catalog(&job.cdn, &job.prefix, &job.snapshot, &job.dir)?;
        let kinds: Vec<_> = catalog.bytes_by_kind().into_iter().collect();
        Ok(Size { packages: catalog.packages.len(), bytes: kinds.iter().map(|(_, b)| b).sum(), kinds })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn start_download(snapshot: String, state: State<'_, Launcher>) -> Result<(), String> {
    let mut slot = state.download.lock().unwrap();
    if slot.as_ref().is_some_and(Download::running) {
        return Err("a download or move is already running".into());
    }
    let job = job_for(&state, &snapshot, &LauncherConfig::load()?)?;
    *slot = Some(Download::spawn("download", snapshot, move |progress, cancel| {
        install::install(&job, progress, cancel).map(|_| ())
    }));
    Ok(())
}

/// Move the resource directory to `to` (the proposed directory when `None`)
/// and switch the configuration to it.
#[tauri::command]
fn start_migrate(to: Option<String>, state: State<'_, Launcher>) -> Result<(), String> {
    let mut slot = state.download.lock().unwrap();
    if slot.as_ref().is_some_and(Download::running) {
        return Err("a download or move is already running".into());
    }
    if let Some((child, _)) = state.game.lock().unwrap().as_mut() {
        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
            return Err("close the game before moving its resources".into());
        }
    }
    let config = LauncherConfig::load()?;
    let from = config.resource_dir()?;
    let to = match to {
        Some(to) => PathBuf::from(to),
        None => moly_store::location::default_resource_dir()?,
    };
    let jobs = config.jobs;
    *slot = Some(Download::spawn("migrate", String::new(), move |progress, cancel| {
        moly_store::migrate::migrate(&from, &to, jobs, progress, cancel, || {
            let mut config = LauncherConfig::load()?;
            config.resource_dir = Some(to.clone());
            config.save()
        })
    }));
    Ok(())
}

#[tauri::command]
fn cancel_download(state: State<'_, Launcher>) {
    if let Some(download) = state.download.lock().unwrap().as_ref() {
        download.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Serialize)]
struct DownloadView {
    kind: &'static str,
    snapshot: String,
    phase: &'static str,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    bytes_fetched: u64,
    retries: u64,
    note: String,
    elapsed_ms: u128,
    finished: bool,
    error: Option<String>,
}

#[tauri::command]
fn download_status(state: State<'_, Launcher>) -> Option<DownloadView> {
    let guard = state.download.lock().unwrap();
    let download = guard.as_ref()?;
    let p = &download.progress;
    let outcome = download.outcome.lock().unwrap();
    // A relocate job has no phase of its own in the store; its state is the job's outcome.
    let phase = match (download.kind, outcome.as_ref()) {
        ("relocate", None) => "moving",
        ("relocate", Some(Ok(()))) => "done",
        ("relocate", Some(Err(error))) if install::is_cancelled(error) => "cancelled",
        ("relocate", Some(Err(_))) => "failed",
        _ => match p.phase() {
            Phase::Idle => "idle",
            Phase::Catalog => "catalog",
            Phase::Manifests => "manifests",
            Phase::Blobs => "blobs",
            Phase::Done => "done",
            Phase::Failed => "failed",
            Phase::Cancelled => "cancelled",
            Phase::Moving => "moving",
        },
    };
    Some(DownloadView {
        kind: download.kind,
        snapshot: download.snapshot.clone(),
        phase,
        files_done: p.files_done.load(Ordering::Relaxed),
        files_total: p.files_total.load(Ordering::Relaxed),
        bytes_done: p.bytes_done.load(Ordering::Relaxed),
        bytes_total: p.bytes_total.load(Ordering::Relaxed),
        bytes_fetched: p.bytes_fetched.load(Ordering::Relaxed),
        retries: p.retries.load(Ordering::Relaxed),
        note: p.note(),
        elapsed_ms: download.started.elapsed().as_millis(),
        finished: outcome.is_some(),
        error: match outcome.as_ref() {
            Some(Err(error)) if !install::is_cancelled(error) => Some(error.clone()),
            _ => None,
        },
    })
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("选择资源目录")
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| moly_store::location::complete_resource_dir(&path).display().to_string())
}

/// A folder picked for one of the folders that can be moved, as picked.
#[tauri::command]
async fn pick_any_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("选择文件夹")
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.display().to_string())
}

#[tauri::command]
fn open_folder(path: String, app: AppHandle) -> Result<(), String> {
    let path = PathBuf::from(path);
    std::fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    app.opener().open_path(path.display().to_string(), None::<&str>).map_err(|error| error.to_string())
}

/// Opens an https address in the system browser.
#[tauri::command]
fn open_url(url: String, app: AppHandle) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("only https addresses can be opened: {url}"));
    }
    app.opener().open_url(url, None::<&str>).map_err(|error| error.to_string())
}

/// Start the game with one installed snapshot.
#[tauri::command]
fn launch(snapshot: String, state: State<'_, Launcher>) -> Result<(), String> {
    // The download and update slots are taken before the game, in the order the other
    // commands take them, so that a move or an update cannot start while the game starts.
    let download = state.download.lock().unwrap();
    if download.as_ref().is_some_and(|job| job.running() && job.kind != "download") {
        return Err("wait for the move to finish before starting the game".into());
    }
    if state.update.lock().unwrap().as_ref().is_some_and(Update::running) {
        return Err("wait for the update to finish before starting the game".into());
    }
    // The game slot is taken before the download slot is released: a move or an update
    // checks the game after taking that slot, so it waits for this start.
    let mut game = state.game.lock().unwrap();
    drop(download);
    if let Some((child, _)) = game.as_mut() {
        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
            return Err("the game is already running".into());
        }
    }
    let config = LauncherConfig::load()?;
    let dir = config.resource_dir()?;
    let record = Record::read(&dir)?;
    let installed = record
        .snapshots
        .iter()
        .find(|item| item.id == snapshot)
        .ok_or_else(|| format!("{snapshot} is not installed in {}", dir.display()))?;
    let exe = game_path(&config);
    if !exe.is_file() {
        return Err(format!("the game program is missing: {}", exe.display()));
    }
    let data = moly_store::location::data_dir()?;
    let logs = moly_store::location::logs_dir()?;
    std::fs::create_dir_all(&logs).map_err(|error| format!("{}: {error}", logs.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let log_path = logs.join(format!("game-{stamp}.log"));
    let log = std::fs::File::create(&log_path).map_err(|error| format!("{}: {error}", log_path.display()))?;
    let err = log.try_clone().map_err(|error| error.to_string())?;
    // A server left by the last game ends before this one starts.
    state.stop_server();
    let (mut server, url) = start_server(
        &exe,
        &log,
        &dir,
        &installed.catalog,
        config.developer,
        &data,
    )?;
    let mut command = Command::new(&exe);
    command
        .current_dir(exe.parent().unwrap_or(&dir))
        .env(moly_store::location::DATA_DIR_ENV, &data)
        .env("MOLY_ASSET_ROOT", &dir)
        .env("MOLY_ASSET_CATALOG", &installed.catalog)
        .env("MOLY_SERVER_URL", &url)
        .env("MOLY_DEV", if config.developer { "1" } else { "0" })
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(err);
    // The game's output goes to the log; a console build gets no window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            server.stop();
            return Err(format!("{}: {error}", exe.display()));
        }
    };
    *state.server.lock().unwrap() = Some(server);
    *game = Some((child, log_path));
    *state.last_exit.lock().unwrap() = None;
    if config.snapshot.as_deref() != Some(snapshot.as_str()) {
        LauncherConfig { snapshot: Some(snapshot), ..config }.save()?;
    }
    Ok(())
}

#[derive(Serialize)]
struct GameView {
    running: bool,
    log: Option<String>,
    exit: Option<String>,
}

#[tauri::command]
fn game_status(state: State<'_, Launcher>) -> GameView {
    let mut game = state.game.lock().unwrap();
    let mut last = state.last_exit.lock().unwrap();
    let Some((child, log)) = game.as_mut() else {
        return GameView { running: false, log: None, exit: last.clone() };
    };
    let log_text = Some(log.display().to_string());
    match child.try_wait() {
        Ok(None) => GameView { running: true, log: log_text, exit: None },
        Ok(Some(status)) => {
            // The server exists for the game; it ends when the game does.
            state.stop_server();
            let exit = if status.success() { None } else { Some(format!("{status}")) };
            *last = exit.clone();
            GameView { running: false, log: log_text, exit }
        }
        Err(error) => GameView { running: false, log: log_text, exit: Some(error.to_string()) },
    }
}

/// Moves the home directory, or the data, logs or versions folder, to `to`. The move
/// runs in the background in the download slot as a `relocate` job, whose `snapshot`
/// names the folder that moves.
#[tauri::command]
fn set_location(kind: String, to: String, state: State<'_, Launcher>) -> Result<(), String> {
    let mut slot = state.download.lock().unwrap();
    if slot.as_ref().is_some_and(Download::running) {
        return Err("a download or move is already running".into());
    }
    refuse_during_update(&state)?;
    if game_running(&state)? {
        return Err("close the game before moving its folders".into());
    }
    let to = PathBuf::from(to);
    if !to.is_absolute() {
        return Err(format!("the folder must be a full path: {}", to.display()));
    }
    let locations = Locations::load()?;
    let from = match kind.as_str() {
        "home" => locations.home()?,
        "data" if data_dir_locked() => {
            return Err("the data directory is fixed by MOLY_DATA_DIR and cannot be moved here".into());
        }
        "data" | "logs" | "versions" => place(&locations, &kind)?,
        _ => return Err(format!("unknown folder: {kind}")),
    };
    if kind == "home" && from == to {
        return Err(format!("{} is already the home directory", to.display()));
    }
    // A folder that holds the running launcher cannot be moved under it.
    if launcher_inside(&from) {
        return Err(format!(
            "the launcher runs from inside {}; start it from another folder to move this one",
            from.display()
        ));
    }
    let jobs = LauncherConfig::load()?.jobs;
    *slot = Some(Download::spawn("relocate", kind.clone(), move |progress, cancel| {
        if kind == "home" {
            move_home(&to, jobs, progress, cancel)
        } else {
            relocate::move_dir(&from, &to)?;
            set_place(&kind, to)
        }
    }));
    Ok(())
}

/// Moves the home directory to `new`. The data, logs and versions folders that sit in the
/// old home move with it, and so do the resources when the launcher configuration keeps
/// them there. The locations file is saved after each step, so a failed step leaves every
/// folder where the file says it is.
fn move_home(new: &Path, jobs: usize, progress: &Progress, cancel: &AtomicBool) -> Result<(), String> {
    // A data directory fixed by MOLY_DATA_DIR is not a folder of the home directory.
    let moved: Vec<&str> = MOVED_FOLDERS
        .iter()
        .copied()
        .filter(|kind| *kind != "data" || !data_dir_locked())
        .collect();
    // The folders in the old home get explicit places first, so that the home directory
    // can change last without moving any of them.
    let mut pinned = Locations::load()?;
    let old_home = pinned.home()?;
    pinned.home = Some(old_home.clone());
    for &kind in &moved {
        let current = place(&pinned, kind)?;
        *field(&mut pinned, kind)? = Some(current);
    }
    pinned.save()?;

    for &kind in &moved {
        let from = place(&Locations::load()?, kind)?;
        if from != old_home.join(kind) {
            continue;
        }
        let to = new.join(kind);
        relocate::move_dir(&from, &to)?;
        set_place(kind, to)?;
    }

    let config = LauncherConfig::load()?;
    let resources = config.resource_dir()?;
    if resources == old_home.join("resources") {
        let target = new.join("resources");
        if holds_resources(&resources) {
            moly_store::migrate::migrate(&resources, &target, jobs, progress, cancel, || {
                let mut config = LauncherConfig::load()?;
                config.resource_dir = Some(target.clone());
                config.save()
            })?;
        } else {
            LauncherConfig { resource_dir: Some(target), ..config }.save()?;
        }
    }

    let mut finished = Locations::load()?;
    finished.home = Some(new.to_path_buf());
    for kind in MOVED_FOLDERS {
        let slot = field(&mut finished, kind)?;
        if slot.as_deref() == Some(new.join(kind).as_path()) {
            *slot = None;
        }
    }
    finished.save()
}

/// The folder of one of the data, logs or versions kinds, as the locations file gives it.
fn place(locations: &Locations, kind: &str) -> Result<PathBuf, String> {
    match kind {
        "data" => locations.data_dir(),
        "logs" => locations.logs_dir(),
        "versions" => locations.versions_dir(),
        _ => Err(format!("unknown folder: {kind}")),
    }
}

/// The field of the locations that holds the folder of one of the data, logs or
/// versions kinds.
fn field<'a>(locations: &'a mut Locations, kind: &str) -> Result<&'a mut Option<PathBuf>, String> {
    match kind {
        "data" => Ok(&mut locations.data),
        "logs" => Ok(&mut locations.logs),
        "versions" => Ok(&mut locations.versions),
        _ => Err(format!("unknown folder: {kind}")),
    }
}

/// Records the folder of one of the data, logs or versions kinds in the locations file.
fn set_place(kind: &str, to: PathBuf) -> Result<(), String> {
    let mut locations = Locations::load()?;
    *field(&mut locations, kind)? = Some(to);
    locations.save()
}

/// Whether the game started by this launcher is still running.
fn game_running(state: &Launcher) -> Result<bool, String> {
    match state.game.lock().unwrap().as_mut() {
        Some((child, _)) => Ok(child.try_wait().map_err(|error| error.to_string())?.is_none()),
        None => Ok(false),
    }
}

/// Refuses while a program update runs. The update slot is taken after the download slot.
fn refuse_during_update(state: &Launcher) -> Result<(), String> {
    if state.update.lock().unwrap().as_ref().is_some_and(Update::running) {
        return Err("an update is running; wait for it to finish".into());
    }
    Ok(())
}

/// The release source of the program updates: `update_source` of the configuration, else
/// the default repository.
fn update_source(config: &LauncherConfig) -> String {
    config.update_source.clone().unwrap_or_else(|| update::DEFAULT_SOURCE.to_owned())
}

/// Whether `MOLY_DATA_DIR` fixes the data directory, which the launcher then cannot move.
fn data_dir_locked() -> bool {
    std::env::var_os(moly_store::location::DATA_DIR_ENV).is_some_and(|value| !value.is_empty())
}

/// Whether the running launcher program lies inside `folder`.
fn launcher_inside(folder: &Path) -> bool {
    let exe = std::env::current_exe().and_then(|exe| std::fs::canonicalize(exe));
    let folder = std::fs::canonicalize(folder);
    matches!((exe, folder), (Ok(exe), Ok(folder)) if exe.starts_with(&folder))
}

/// Whether `dir` holds resources, which a move carries along with the folder.
fn holds_resources(dir: &Path) -> bool {
    ["catalogs", "packages", "blobs"].iter().any(|name| dir.join(name).is_dir())
        || dir.join(RECORD_FILE).is_file()
}

/// Whether two files hold the same bytes. A file that cannot be read counts as different.
fn same_bytes(a: &Path, b: &Path) -> bool {
    let same_size = match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.len() == b.len(),
        _ => return false,
    };
    same_size && matches!((std::fs::read(a), std::fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

/// Installs the newest release into `versions`, replaces the launcher program when the
/// installed one differs from `current`, and keeps the newest program versions.
fn install_update(
    source: &str,
    versions: &Path,
    current: &Path,
    config: &LauncherConfig,
    progress: &update::Progress,
    cancel: &AtomicBool,
    version: &Mutex<String>,
    launcher_replaced: &AtomicBool,
) -> Result<(), String> {
    *progress.stage.lock().unwrap() = "checking".to_owned();
    let release = update::latest(source)?;
    if !update::is_newer(&release.version, env!("CARGO_PKG_VERSION")) {
        return Err(format!("release {} is not newer than this launcher", release.version));
    }
    *version.lock().unwrap() = release.version.clone();
    let folder = update::install(&release, versions, progress, cancel)?;
    let launcher = folder.join(LAUNCHER_FILE);
    if launcher.is_file() && !same_bytes(&launcher, current) {
        update::replace_launcher(&launcher)?;
        launcher_replaced.store(true, Ordering::Relaxed);
    }
    let game = game_path(config);
    update::prune(versions, KEPT_VERSIONS, &[current, game.as_path()])?;
    Ok(())
}

#[derive(Serialize)]
struct UpdateView {
    current: &'static str,
    latest: String,
    newer: bool,
    notes: String,
    page: String,
    /// Size of the full package the update downloads.
    download_bytes: u64,
}

/// Reads the newest program release of the update source and compares it with this
/// launcher.
#[tauri::command]
async fn check_update() -> Result<UpdateView, String> {
    let source = update_source(&LauncherConfig::load()?);
    let release = tauri::async_runtime::spawn_blocking(move || update::latest(&source))
        .await
        .map_err(|error| error.to_string())??;
    let download_bytes = release.package.size;
    let current = env!("CARGO_PKG_VERSION");
    Ok(UpdateView {
        current,
        newer: update::is_newer(&release.version, current),
        latest: release.version,
        notes: release.notes,
        page: release.page,
        download_bytes,
    })
}

/// Starts the program update, unless a download or move, another update or the game is
/// running.
#[tauri::command]
fn start_update(state: State<'_, Launcher>) -> Result<(), String> {
    let download = state.download.lock().unwrap();
    if download.as_ref().is_some_and(Download::running) {
        return Err("a download or move is already running".into());
    }
    let mut slot = state.update.lock().unwrap();
    if slot.as_ref().is_some_and(Update::running) {
        return Err("an update is already running".into());
    }
    if game_running(&state)? {
        return Err("close the game before updating".into());
    }
    let config = LauncherConfig::load()?;
    let source = update_source(&config);
    let versions = moly_store::location::versions_dir()?;
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    *slot = Some(Update::spawn(source, versions, current, config));
    Ok(())
}

#[derive(Serialize)]
struct UpdateStatus {
    stage: String,
    bytes_done: u64,
    bytes_total: u64,
    finished: bool,
    /// Set when the update failed; a cancelled update reports `stage` as `cancelled`.
    error: Option<String>,
    version: String,
    launcher_replaced: bool,
}

/// The program update in progress or the last one; `None` when none has run.
#[tauri::command]
fn update_status(state: State<'_, Launcher>) -> Option<UpdateStatus> {
    let guard = state.update.lock().unwrap();
    let job = guard.as_ref()?;
    let outcome = job.outcome.lock().unwrap();
    let status = UpdateStatus {
        stage: job.progress.stage.lock().unwrap().clone(),
        bytes_done: job.progress.bytes_done.load(Ordering::Relaxed),
        bytes_total: job.progress.bytes_total.load(Ordering::Relaxed),
        finished: outcome.is_some(),
        error: match outcome.as_ref() {
            Some(Err(error)) if !install::is_cancelled(error) => Some(error.clone()),
            _ => None,
        },
        version: job.version.lock().unwrap().clone(),
        launcher_replaced: job.launcher_replaced.load(Ordering::Relaxed),
    };
    Some(status)
}

/// Asks the running program update to stop at its next chunk.
#[tauri::command]
fn cancel_update(state: State<'_, Launcher>) {
    if let Some(job) = state.update.lock().unwrap().as_ref() {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// Starts the launcher program again and ends this process. Refused while the game runs.
#[tauri::command]
fn restart_launcher(app: AppHandle, state: State<'_, Launcher>) -> Result<(), String> {
    if game_running(&state)? {
        return Err("close the game before restarting the launcher".into());
    }
    let exe = LAUNCHER_PATH.get().cloned().ok_or("the launcher's own path is unknown")?;
    Command::new(&exe)
        .spawn()
        .map_err(|error| format!("{}: {error}", exe.display()))?;
    app.exit(0);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Launcher::default())
        .setup(|app| {
            if let Ok(exe) = std::env::current_exe() {
                let _ = LAUNCHER_PATH.set(exe);
            }
            // The locations are settled first: the folders used below follow them.
            moly_store::location::settle()?;
            // The program files this launcher runs from become a version folder when no
            // complete one exists for this version yet, so that the first update has a base.
            let program_dir = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf));
            let versions = moly_store::location::versions_dir();
            if let (Ok(versions), Some(program_dir)) = (versions, program_dir) {
                let _ = update::bootstrap(&versions, &program_dir, env!("CARGO_PKG_VERSION"));
            }
            // The window is built here rather than from the configuration so that the
            // webview keeps its profile beside the locations file, outside the folders
            // that can be moved.
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .cloned()
                .ok_or("the main window is not configured")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .data_directory(moly_store::location::webview_dir()?)
                .build()?;
            Ok(())
        })
        .register_uri_scheme_protocol("molyfont", |_context, _request| {
            Response::builder()
                .header("Content-Type", "font/ttf")
                .header("Access-Control-Allow-Origin", "*")
                .body(FONT.to_vec())
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            save_config,
            publication,
            snapshot_size,
            start_download,
            start_migrate,
            cancel_download,
            download_status,
            pick_folder,
            pick_any_folder,
            open_folder,
            open_url,
            set_location,
            launch,
            game_status,
            check_update,
            start_update,
            update_status,
            cancel_update,
            restart_launcher
        ])
        .run(tauri::generate_context!())
        .expect("the launcher window could not start");
}
