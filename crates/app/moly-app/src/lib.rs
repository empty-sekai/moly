//! Shared application startup. The browser supplies its preflighted renderer
//! and the lifetime of its exclusive storage lease.

pub mod asset_source;
pub mod player_data_input;
pub mod site_request;

#[cfg(target_arch = "wasm32")]
mod render_backend;
#[cfg(target_arch = "wasm32")]
mod wasm_heap;

#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

/// What one start installs. The developer window (`MOLY_DEV=1`) and the
/// embedded stage read their inputs from env or the page URL and carry
/// developer tools; the game, native or in a page, reads only its seed and
/// carries none.
enum StartMode {
    #[cfg(not(target_arch = "wasm32"))]
    Library,
    #[cfg(target_arch = "wasm32")]
    Stage,
    #[cfg(target_arch = "wasm32")]
    Game(GameStart),
}

/// A seed already admitted by `start_game`, with its asset source.
#[cfg(target_arch = "wasm32")]
struct GameStart {
    seed: moly_game::GameSeed,
    source: moly_assets::AssetSource,
    remote: moly_assets::remote::RemoteBases,
}

/// 起一次 app。资产源与站点选择在这里解析、经 `moly_game::app` 装上——
/// 消费方读资源，不要再各自去读 env 或 URL（拒绝点全树只在 `asset_source`
/// 与 `site_request` 两处入口）。
fn run_app(
    #[cfg(target_arch = "wasm32")] web_render_settings: bevy::render::settings::WgpuSettings,
    mode: StartMode,
) {
    #[cfg(target_arch = "wasm32")]
    if let StartMode::Game(start) = mode {
        return run_game(web_render_settings, start);
    }
    let source = match asset_source::resolve() {
        Ok(source) => source,
        Err(message) => fail_loud(&message),
    };
    let site = match site_request::resolve() {
        Ok(site) => site,
        Err(message) => fail_loud(&message),
    };
    let remote = match asset_source::resolve_remote() {
        Ok(remote) => remote,
        Err(message) => fail_loud(&message),
    };
    // 渲染后端只在 wasm 分支选（取舍在 `render_backend`）；native 展开后
    // 与双后端升级前逐行一致。
    #[cfg(target_arch = "wasm32")]
    let mut app = moly_game::app(source, site, web_render_settings);
    #[cfg(not(target_arch = "wasm32"))]
    let mut app = moly_game::app(source, site);
    moly_assets::remote::admit(&mut app, remote);
    moly_game::insert_dev_tools(&mut app);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(directory) = std::env::var_os("MOLY_PORTRAITS_OUT") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("create portrait output directory");
        // This opt-in export process never writes the user's normal settings.
        std::env::set_var("MOLY_SETTINGS_FILE", directory.join("capture-settings.json"));
        let only = std::env::var("MOLY_PORTRAIT_UNIT").ok()
            .map(|value| value.parse::<u32>().expect("MOLY_PORTRAIT_UNIT must be a source unit ID"));
        moly_game::configure_browser_stage(&mut app);
        moly_game::configure_browser_library(&mut app);
        moly_game::portraits::configure(&mut app, directory, only);
    }
    if let Err(message) = player_data_input::configure(&mut app) {
        fail_loud(&message);
    }
    #[cfg(target_arch = "wasm32")]
    {
        moly_game::configure_browser_library(&mut app);
        if matches!(mode, StartMode::Stage) { moly_game::configure_browser_stage(&mut app); }
        attach_canvas(&mut app);
        announce_ready(&mut app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let StartMode::Library = mode;
        install_link(&mut app);
    }
    app.run();
}

/// The browser game: seed inputs only, no library or stage configuration,
/// no player-data import parameters and no developer tools.
#[cfg(target_arch = "wasm32")]
fn run_game(web_render_settings: bevy::render::settings::WgpuSettings, start: GameStart) {
    let mut app = moly_game::app(start.source, site_request::game(), web_render_settings);
    moly_assets::remote::admit(&mut app, start.remote);
    moly_game::configure_browser_game(&mut app, start.seed);
    attach_canvas(&mut app);
    announce_ready(&mut app);
    app.run();
}

#[cfg(target_arch = "wasm32")]
fn announce_ready(app: &mut App) {
    app.add_systems(Startup, || {
        if let Some(window) = web_sys::window() {
            if let Ok(event) = web_sys::Event::new("moly-ready") {
                let _ = window.dispatch_event(&event);
            }
        }
    });
}

/// The native program starts the game, as the page does, unless
/// `MOLY_DEV=1` asks for the developer window.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    match std::env::var("MOLY_DEV").ok().as_deref() {
        None | Some("") | Some("0") => run_native_game(),
        Some("1") => run_app(StartMode::Library),
        Some(other) => fail_loud(&format!("MOLY_DEV must be 1 or 0, not {other:?}")),
    }
}

/// The native game: the page game's start with the native host's documents.
/// The asset root and the remote hosts are read as the developer window
/// reads them; nothing else in the environment changes the game.
#[cfg(not(target_arch = "wasm32"))]
fn run_native_game() {
    let source = asset_source::resolve().unwrap_or_else(|message| fail_loud(&message));
    let remote = asset_source::resolve_remote().unwrap_or_else(|message| fail_loud(&message));
    let mut seed = moly_game::native_game_seed().unwrap_or_else(|message| fail_loud(&message));
    moly_game::install_game_storage(&mut seed);
    let mut app = moly_game::app(source, site_request::game());
    moly_assets::remote::admit(&mut app, remote);
    moly_game::configure_browser_game(&mut app, seed);
    install_link(&mut app);
    app.run();
}

/// The server address a native start uses when `MOLY_SERVER_URL` is unset.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_SERVER_URL: &str = "ws://127.0.0.1:15800";

/// Gives the game its session with the server named by `MOLY_SERVER_URL`.
/// A launcher reads the address of the server it started and passes it here;
/// a native start without one connects to [`DEFAULT_SERVER_URL`].
#[cfg(not(target_arch = "wasm32"))]
fn install_link(app: &mut bevy::prelude::App) {
    let config = link_config().unwrap_or_else(|message| fail_loud(&message));
    app.add_plugins(moly_link::LinkPlugin(config));
}

#[cfg(not(target_arch = "wasm32"))]
fn link_config() -> Result<moly_link::LinkConfig, String> {
    let address = match std::env::var("MOLY_SERVER_URL") {
        Ok(value) if value.trim().is_empty() => {
            return Err("MOLY_SERVER_URL is empty; it names the server's ws:// address".into())
        }
        Ok(value) if !is_server_address(value.trim()) => {
            return Err(format!(
                "MOLY_SERVER_URL must be a ws://host:port address, not {value:?}"
            ))
        }
        Ok(value) => value.trim().to_owned(),
        Err(std::env::VarError::NotPresent) => DEFAULT_SERVER_URL.to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("MOLY_SERVER_URL is not valid text".into())
        }
    };
    Ok(moly_link::LinkConfig {
        address,
        user: moly_protocol::UserId::local(),
        client: format!("moly-app {}", moly_game::VERSION),
        connect_window: moly_link::LinkConfig::CONNECT_WINDOW,
    })
}

/// `ws://` or `wss://`, then a host with no credentials and no whitespace.
#[cfg(not(target_arch = "wasm32"))]
fn is_server_address(value: &str) -> bool {
    let authority = value
        .strip_prefix("ws://")
        .or_else(|| value.strip_prefix("wss://"))
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or(""));
    authority.is_some_and(|authority| {
        !authority.is_empty()
            && !authority.contains('@')
            && !value.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn fail_loud(message: &str) -> ! {
    eprintln!("moly-app: {message}");
    #[cfg(windows)]
    show_error_without_stderr(message);
    std::process::exit(1);
}

/// A program started without standard error (the release build opened from
/// the file manager) also shows the message in a dialog.
#[cfg(windows)]
fn show_error_without_stderr(message: &str) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    if !std::io::stderr().as_raw_handle().is_null() {
        return;
    }
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("moly"));
    // SAFETY: both strings are NUL-terminated and outlive the call; the
    // dialog has no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR) };
}

#[cfg(target_arch = "wasm32")]
fn fail_loud(message: &str) -> ! {
    panic!("moly-app: {message}");
}

/// 认领宿主页面里的 `#app-canvas`。主窗实体由 DefaultPlugins 在
/// build 时生成，但裸窗到 runner 的 `resumed()` 才创建、届时才读 `canvas`
/// 选择器（找不到元素会 panic）——所以在 `run()` 之前改主窗仍然生效，
/// 不必为 canvas 给 moly-game 开配置口。
#[cfg(target_arch = "wasm32")]
fn attach_canvas(app: &mut App) {
    // 0.18 的 prelude 不含这个标记组件，真身在 bevy::window。
    use bevy::window::PrimaryWindow;
    let mut windows = app
        .world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>();
    // DefaultPlugins 只建一个主窗；多了少了都是组装层坏了，值得在这里炸。
    let mut window = windows
        .single_mut(app.world_mut())
        .expect("exactly one primary window");
    window.canvas = Some("#app-canvas".into());
    // 画布跟随父元素尺寸（body 定为视口高）；父元素不能反过来随子元素
    // 撑开，否则每次 resize 反馈放大一圈。
    window.fit_canvas_to_parent = true;
}

/// Installs the release's verified UI font before a browser entry starts.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn install_ui_font(bytes: Vec<u8>) -> Result<(), wasm_bindgen::JsValue> {
    moly_game::ui_font::install(bytes)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// Read-only stage entry; no second player/runtime is installed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_stage(backend: &str) -> Result<(), wasm_bindgen::JsValue> {
    moly_game::ui_font::require().map_err(|error| wasm_bindgen::JsValue::from_str(&error))?;
    install_panic_hook();
    let settings = render_backend::settings(backend)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error))?;
    moly_game::set_browser_storage_writable(false);
    run_app(settings, StartMode::Stage);
    Ok(())
}

/// Starts the browser game from its seed, synchronously inside the trusted
/// click. A seed refusal returns before anything is installed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_game(backend: &str, seed: &str) -> Result<(), wasm_bindgen::JsValue> {
    let refuse = |error: String| wasm_bindgen::JsValue::from_str(&error);
    moly_game::ui_font::require().map_err(refuse)?;
    let mut seed = moly_game::parse_game_seed(seed).map_err(refuse)?;
    let source = asset_source::resolve_seed(&seed).map_err(refuse)?;
    let remote = asset_source::resolve_remote_seed(&seed).map_err(refuse)?;
    let settings = render_backend::settings(backend).map_err(refuse)?;
    install_panic_hook();
    moly_game::install_game_storage(&mut seed);
    run_app(settings, StartMode::Game(GameStart { seed, source, remote }));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // Disable writes before notifying JavaScript; the host may release its lease.
        moly_game::set_browser_storage_writable(false);
        if let Some(window) = web_sys::window() {
            let message = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("Unexpected game error");
            let init = web_sys::CustomEventInit::new();
            init.set_detail(&message.into());
            if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("moly-error", &init) {
                let _ = window.dispatch_event(&event);
            }
        }
        console_error_panic_hook::hook(info);
    }));
}

/// Library and player-data intents belong to the embedded stage.
#[cfg(target_arch = "wasm32")]
fn refuse_in_game() -> Result<(), wasm_bindgen::JsValue> {
    if moly_game::game_mode_active() {
        return Err(wasm_bindgen::JsValue::from_str(
            "This command belongs to the content library pages, not to the game",
        ));
    }
    Ok(())
}

/// Enqueue one validated browser intent; the normal Bevy input phase owns it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn library_command(command: &str) -> Result<(), wasm_bindgen::JsValue> {
    refuse_in_game()?;
    moly_game::library_command(command).map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// Read the last published, versioned projection without borrowing the world.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn library_snapshot() -> String {
    moly_game::library_snapshot()
}

/// Deployment tooling requests a source-scoped catalogue, never a live entity.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn library_catalog() -> String { moly_game::library_catalog() }

/// Read-only native/browser QA projection. Deliberately absent from the host
/// postMessage intention protocol; it cannot mutate a world or inject scripts.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn library_diagnostics() -> String { moly_game::library_diagnostics() }

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn player_data_command(operation: &str, region: &str, json: &str) -> Result<(), wasm_bindgen::JsValue> {
    refuse_in_game()?;
    moly_game::player_data::browser_command(operation, region, json)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn player_data_snapshot() -> String { moly_game::player_data::browser_snapshot() }

/// Frame counters of the running application as JSON: percentiles over the
/// last 600 frames and the newest `frames` frames (120 when omitted). The
/// browser measurement probe calls it on the imported module, as the QA
/// harness calls `library_snapshot`; the page itself does not.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn perf_snapshot(frames: Option<u32>) -> String {
    moly_perf::snapshot_json(frames.map_or(moly_perf::DEFAULT_RECENT, |frames| frames as usize))
}

/// Enqueue one validated game intent (lifecycle, input capture, persist
/// acknowledgement); refused before `start_game`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn game_command(json: &str) -> Result<(), wasm_bindgen::JsValue> {
    moly_game::game_command(json).map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// The game's versioned projection: entry, storage lease, persistence,
/// memory and named refusals.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn game_snapshot() -> String {
    moly_game::game_snapshot()
}

/// The settings document of the newest revision above `after_revision`, or
/// an empty string when there is none.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn game_take_persist(after_revision: u32) -> String {
    moly_game::game_take_persist(after_revision)
}

/// Validates a director command (a script to run, stop, pause or resume)
/// and queues it for the next frame; available on every page mode.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn director_command(json: &str) -> Result<(), wasm_bindgen::JsValue> {
    moly_game::director_command(json).map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// The director's state: status, the steps running now and the run's log.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn director_snapshot() -> String {
    moly_game::director_snapshot()
}

/// Sends one JSON-RPC 2.0 request to the control surface (the methods of
/// `rpc.discover`); resolves to its JSON-RPC 2.0 response, handled in the
/// next frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn control(request: String) -> js_sys::Promise {
    wasm_bindgen_futures::future_to_promise(async move {
        Ok(wasm_bindgen::JsValue::from_str(&moly_game::control_request(&request).await))
    })
}

/// The editable fields of the server document, for the page's panel.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn game_server_schema() -> String {
    moly_game::game_server_schema()
}

/// The server document with its clock, the client's copies, the pending
/// sections and the named refusals.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn game_server_document() -> String {
    moly_game::game_server_document()
}
