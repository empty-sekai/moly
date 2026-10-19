//! Music, sound effects, voices and the site's ambient sound.

#[macro_use]
extern crate moly_core;

// Every lower layer's modules resolve as `crate::<module>`.
pub use moly_core::VERSION;
pub use moly_core::animation_time;
pub use moly_core::asset_cache;
#[cfg(target_arch = "wasm32")]
pub use moly_core::browser_log;
pub use moly_core::client_config;
pub use moly_core::dev_tools;
pub use moly_core::frame_capture;
pub use moly_core::hold;
#[cfg(not(target_arch = "wasm32"))]
pub use moly_core::native_graphics_diagnostics;
pub use moly_core::pipeline_report;
pub use moly_core::run_mode;
pub use moly_core::saved_total_exp;
pub use moly_core::settings_store;
pub use moly_core::world_seed;
pub use moly_core::canvas;
pub use moly_core::env;
pub use moly_core::inactive_nodes;
pub use moly_core::input_claims;
pub use moly_core::light;
pub use moly_core::markers;
pub use moly_core::material_order;
pub use moly_core::mesh_buffer_release;
pub use moly_core::overlay_camera;
pub use moly_core::player_data_io;
pub use moly_core::render;
pub use moly_core::system_access;
pub use moly_core::ui_font;
pub use moly_core::weather_transition;

pub mod audio;
pub mod audio_inputs;
pub mod audio_sequence;
pub mod audio_startup;
pub mod site_sound;
pub mod voice_pcm;
