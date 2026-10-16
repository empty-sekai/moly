//! The moly launcher: picks the resource CDN and directory, downloads and
//! verifies resource snapshots, and starts the game with them.
//!
//! The page in `ui/` draws everything; this program answers its commands.
//! Settings live in the data directory's `launcher.json`, which the game also
//! reads when it is started without configuration.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use moly_store::config::LauncherConfig;
use moly_store::install::{self, Job, Phase, Progress};
use moly_store::publication::Publication;
use moly_store::record::{Installed, Record};
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::http::Response;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// The page's font: the game's UI font, served on the `molyfont` scheme.
static FONT: &[u8] = include_bytes!("../../../game/moly-game/assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

#[cfg(windows)]
const GAME_FILE: &str = "moly-app.exe";
#[cfg(not(windows))]
const GAME_FILE: &str = "moly-app";

/// A download or a move of the resource directory; one runs at a time.
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

#[derive(Default)]
struct Launcher {
    publication: Mutex<Option<(String, Publication)>>,
    download: Mutex<Option<Download>>,
    game: Mutex<Option<(Child, PathBuf)>>,
    last_exit: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct Overview {
    version: &'static str,
    config: LauncherConfig,
    resource_dir: String,
    default_resource_dir: String,
    data_dir: String,
    logs_dir: String,
    installed: Vec<Installed>,
    selected: Option<String>,
    game: String,
    game_found: bool,
}

fn game_path(config: &LauncherConfig) -> PathBuf {
    if let Some(game) = &config.game {
        return game.clone();
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
    Ok(Overview {
        version: env!("CARGO_PKG_VERSION"),
        resource_dir: dir.display().to_string(),
        default_resource_dir: moly_store::location::default_resource_dir()?.display().to_string(),
        data_dir: moly_store::location::data_dir()?.display().to_string(),
        logs_dir: moly_store::location::data_dir()?.join("logs").display().to_string(),
        installed: record.snapshots,
        selected,
        game_found: game.is_file(),
        game: game.display().to_string(),
        config,
    })
}

/// The launcher's state. The first time, the proposed resource directory is
/// stored, so it stays put when free space on the drives changes.
#[tauri::command]
fn overview() -> Result<Overview, String> {
    let mut config = LauncherConfig::load()?;
    if config.resource_dir.is_none() {
        config.resource_dir = Some(moly_store::location::default_resource_dir()?);
        config.save()?;
    }
    overview_of(config)
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
    let phase = match p.phase() {
        Phase::Idle => "idle",
        Phase::Catalog => "catalog",
        Phase::Manifests => "manifests",
        Phase::Blobs => "blobs",
        Phase::Done => "done",
        Phase::Failed => "failed",
        Phase::Cancelled => "cancelled",
        Phase::Moving => "moving",
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

#[tauri::command]
fn open_folder(path: String, app: AppHandle) -> Result<(), String> {
    let path = PathBuf::from(path);
    std::fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    app.opener().open_path(path.display().to_string(), None::<&str>).map_err(|error| error.to_string())
}

/// Start the game with one installed snapshot.
#[tauri::command]
fn launch(snapshot: String, state: State<'_, Launcher>) -> Result<(), String> {
    let mut game = state.game.lock().unwrap();
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
    let logs = data.join("logs");
    std::fs::create_dir_all(&logs).map_err(|error| format!("{}: {error}", logs.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let log_path = logs.join(format!("game-{stamp}.log"));
    let log = std::fs::File::create(&log_path).map_err(|error| format!("{}: {error}", log_path.display()))?;
    let err = log.try_clone().map_err(|error| error.to_string())?;
    let mut command = Command::new(&exe);
    command
        .current_dir(exe.parent().unwrap_or(&dir))
        .env(moly_store::location::DATA_DIR_ENV, &data)
        .env("MOLY_ASSET_ROOT", &dir)
        .env("MOLY_ASSET_CATALOG", &installed.catalog)
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
    let child = command.spawn().map_err(|error| format!("{}: {error}", exe.display()))?;
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
            let exit = if status.success() { None } else { Some(format!("{status}")) };
            *last = exit.clone();
            GameView { running: false, log: log_text, exit }
        }
        Err(error) => GameView { running: false, log: log_text, exit: Some(error.to_string()) },
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Launcher::default())
        .setup(|app| {
            // The window is built here rather than from the configuration so
            // that the webview keeps its profile in the data directory.
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .cloned()
                .ok_or("the main window is not configured")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .data_directory(moly_store::location::data_dir()?.join("webview"))
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
            open_folder,
            launch,
            game_status
        ])
        .run(tauri::generate_context!())
        .expect("the launcher window could not start");
}
