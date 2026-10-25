//! Browser ABI: source admission and a single canvas/event loop.
use bevy::prelude::*;
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

fn error(message: String) -> JsValue {
    JsValue::from_str(&message)
}

#[wasm_bindgen]
pub fn install_ui_font(bytes: Vec<u8>) -> Result<(), JsValue> {
    moly_core::ui_font::install(bytes).map_err(error)
}

#[wasm_bindgen]
pub fn install_text_font(bytes: Vec<u8>, sha256: &str) -> Result<(), JsValue> {
    moly_world::text_localization::install_font(bytes, sha256).map_err(error)
}

#[wasm_bindgen]
pub fn start_stage(backend: &str) -> Result<(), JsValue> {
    moly_core::ui_font::require().map_err(error)?;
    let settings =
        moly_core::render_backend::settings(backend, cfg!(feature = "webgpu")).map_err(error)?;
    let source = moly_assets::browser_source::resolve().map_err(error)?;
    let remote = moly_assets::browser_source::resolve_remote().map_err(error)?;
    let site = moly_features::site_request::resolve().map_err(error)?;
    moly_core::settings_store::set_browser_storage_writable(false);
    std::panic::set_hook(Box::new(|info| {
        if let Some(window) = web_sys::window() {
            let message = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("Unexpected stage error");
            let init = web_sys::CustomEventInit::new();
            init.set_detail(&message.into());
            if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("moly-error", &init) {
                let _ = window.dispatch_event(&event);
            }
        }
        console_error_panic_hook::hook(info);
    }));
    let mut app = crate::app(source, site, settings);
    moly_assets::remote::admit(&mut app, remote);
    moly_features::player_data_input::configure(&mut app).map_err(error)?;
    // Weather schedules, talk qualification and imported-layout exploration
    // consume the local server's client copies. It has no remote write target.
    app.add_plugins(moly_link::LocalLinkPlugin {
        user: moly_protocol::UserId::local(),
        client: format!("moly-stage {}", crate::VERSION),
    });
    let world = app.world_mut();
    let mut windows = world.query_filtered::<&mut Window, With<bevy::window::PrimaryWindow>>();
    let mut window = windows.single_mut(world).expect("exactly one stage canvas");
    window.canvas = Some("#app-canvas".into());
    window.fit_canvas_to_parent = true;
    app.add_systems(Startup, || {
        if let Some(window) = web_sys::window() {
            if let Ok(event) = web_sys::Event::new("moly-ready") {
                let _ = window.dispatch_event(&event);
            }
        }
    });
    app.run();
    Ok(())
}

#[wasm_bindgen]
pub fn library_command(command: &str) -> Result<(), JsValue> {
    moly_features::content_library::bridge::library_command(command).map_err(error)
}
#[wasm_bindgen]
pub fn library_snapshot() -> String {
    moly_features::content_library::bridge::library_snapshot()
}
#[wasm_bindgen]
pub fn library_catalog() -> String {
    moly_features::content_library::bridge::library_catalog()
}
#[wasm_bindgen]
pub fn library_diagnostics() -> String {
    moly_features::content_library::library_diagnostics()
}
#[wasm_bindgen]
pub fn player_data_command(operation: &str, region: &str, json: &str) -> Result<(), JsValue> {
    moly_features::player_data::browser_command(operation, region, json).map_err(error)
}
#[wasm_bindgen]
pub fn player_data_snapshot() -> String {
    moly_features::player_data::browser_snapshot()
}
#[wasm_bindgen]
pub fn perf_snapshot(frames: Option<u32>) -> String {
    moly_perf::snapshot_json(frames.map_or(moly_perf::DEFAULT_RECENT, |frames| frames as usize))
}
#[wasm_bindgen]
pub fn configure_text(
    source: &str,
    official: &str,
    community: &str,
    locale: &str,
) -> Result<(), JsValue> {
    moly_world::text_localization::configure(source, official, community, locale).map_err(error)
}

/// Called at the stage's idle boundary after all publication fonts download.
#[wasm_bindgen]
pub fn configure_text_session(
    source: &str,
    official: &str,
    community: &str,
    locale: &str,
    font_bytes: Vec<u8>,
    font_references: &str,
) -> Result<(), JsValue> {
    moly_core::ui_font::with_text_font_session(font_bytes, font_references, || {
        if source.is_empty() && official.is_empty() && community.is_empty() {
            moly_world::text_localization::clear()
        } else {
            moly_world::text_localization::configure(source, official, community, locale)
        }
    }).map_err(error)
}
#[wasm_bindgen]
pub fn clear_text() -> Result<(), JsValue> {
    moly_world::text_localization::clear().map_err(error)
}
#[wasm_bindgen]
pub fn text_ready() -> bool {
    moly_world::text_localization::ready()
}
