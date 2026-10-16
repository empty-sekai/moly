//! Leaf modules the game's domains share: the local settings document, the
//! downloaded client configuration, the world seed, per-frame holds, the
//! frame and pipeline instruments and the developer scaffolding switch.
//!
//! Nothing here reads a game domain: a reader of one of these modules either
//! takes what it needs as an argument or reads it out of the world itself.

pub mod animation_time;
pub mod asset_cache;
#[cfg(target_arch = "wasm32")]
pub mod browser_log;
pub mod client_config;
pub mod dev_tools;
pub mod frame_capture;
pub mod hold;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_graphics_diagnostics;
pub mod pipeline_report;
pub mod run_mode;
pub mod saved_total_exp;
pub mod settings_store;
pub mod world_seed;
