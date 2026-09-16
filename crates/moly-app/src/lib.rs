//! Shared application startup. The browser supplies its preflighted renderer
//! and the lifetime of its exclusive storage lease.

pub mod asset_source;
pub mod player_data_input;
pub mod site_request;

#[cfg(target_arch = "wasm32")]
mod render_backend;

#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

/// 起一次 app。资产源与站点选择在这里解析、经 `moly_game::app` 装上——
/// 消费方读资源，不要再各自去读 env 或 URL（拒绝点全树只在 `asset_source`
/// 与 `site_request` 两处入口）。
fn run_app(
    #[cfg(target_arch = "wasm32")] web_render_settings: bevy::render::settings::WgpuSettings,
    #[cfg(target_arch = "wasm32")] stage: bool,
) {
    let source = match asset_source::resolve() {
        Ok(source) => source,
        Err(message) => fail_loud(&message),
    };
    let site = match site_request::resolve() {
        Ok(site) => site,
        Err(message) => fail_loud(&message),
    };
    // 渲染后端只在 wasm 分支选（取舍在 `render_backend`）；native 展开后
    // 与双后端升级前逐行一致。
    #[cfg(target_arch = "wasm32")]
    let mut app = moly_game::app(source, site, web_render_settings);
    #[cfg(not(target_arch = "wasm32"))]
    let mut app = moly_game::app(source, site);
    if let Err(message) = player_data_input::configure(&mut app) {
        fail_loud(&message);
    }
    #[cfg(target_arch = "wasm32")]
    {
        moly_game::configure_browser_library(&mut app);
        if stage { moly_game::configure_browser_stage(&mut app); }
        attach_canvas(&mut app);
        app.add_systems(Startup, || {
            if let Some(window) = web_sys::window() {
                if let Ok(event) = web_sys::Event::new("moly-ready") {
                    let _ = window.dispatch_event(&event);
                }
            }
        });
    }
    app.run();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    run_app();
}

#[cfg(not(target_arch = "wasm32"))]
fn fail_loud(message: &str) -> ! {
    eprintln!("moly-app: {message}");
    std::process::exit(1);
}

#[cfg(target_arch = "wasm32")]
fn fail_loud(message: &str) -> ! {
    panic!("moly-app: {message}");
}

/// 认领 `index.html` 里的 `#app-canvas`。主窗实体由 DefaultPlugins 在
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

/// Starts one browser application synchronously inside the trusted click.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start(backend: &str, writable: bool) -> Result<(), wasm_bindgen::JsValue> {
    start_browser(backend, writable, false)
}

/// Read-only stage entry; no second player/runtime is installed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_stage(backend: &str) -> Result<(), wasm_bindgen::JsValue> {
    start_browser(backend, false, true)
}

#[cfg(target_arch = "wasm32")]
fn start_browser(backend: &str, writable: bool, stage: bool) -> Result<(), wasm_bindgen::JsValue> {
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
    let settings = render_backend::settings(backend)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error))?;
    set_storage_writable(writable);
    run_app(settings, stage);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_storage_writable(writable: bool) {
    moly_game::set_browser_storage_writable(writable);
}

/// Enqueue one validated browser intent; the normal Bevy input phase owns it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn library_command(command: &str) -> Result<(), wasm_bindgen::JsValue> {
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
