//! Leaf modules the game's domains share: the local settings document, the
//! downloaded client configuration, the world seed, per-frame holds, the
//! frame and pipeline instruments and the developer scaffolding switch.
//!
//! Nothing here reads a game domain: a reader of one of these modules either
//! takes what it needs as an argument or reads it out of the world itself.

/// Assigns `$value` to `$place` only when they differ. A write through `Mut`
/// marks the component changed even when the value is equal, and the
/// change-driven systems (UI layout and text measurement, transform and
/// visibility propagation) then redo their work for it on that frame.
#[macro_export]
macro_rules! set_if_neq {
    ($place:expr, $value:expr) => {{
        let value = $value;
        if $place != value {
            $place = value;
        }
    }};
}

extern crate self as moly_core;

pub mod animation_time;
pub mod asset_cache;
#[cfg(target_arch = "wasm32")]
pub mod browser_log;
pub mod client_config;
pub mod dev_tools;
pub mod frame_capture;
pub mod harness_io;
pub mod hold;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_graphics_diagnostics;
pub mod pipeline_report;
pub mod run_mode;
pub mod saved_total_exp;
pub mod settings_store;
pub mod world_seed;

pub mod canvas;
pub mod env;
pub mod inactive_nodes;
pub mod input_claims;
pub mod light;
pub mod markers;
pub mod material_order;
pub mod mesh_buffer_release;
pub mod overlay_camera;
pub mod player_data_io;
pub mod render;
pub mod system_access;
pub mod ui_font;
pub mod weather_transition;

/// Product version shared by the settings panel and application entry points.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(target_arch = "wasm32")]
pub mod render_backend;
#[cfg(all(target_arch = "wasm32", feature = "browser-heap"))]
mod wasm_heap;
