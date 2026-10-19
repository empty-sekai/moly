//! Materials, shadows, sky, roads and the particle system the game draws with.

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

pub mod billboard;
pub mod character_material;
pub mod fixture_timeline_particles;
pub mod fixture_colors;
pub mod fixture_emission;
pub mod fixture_material;
pub mod gpu_image_release;
pub mod particle_runtime;
pub mod particle_geometry;
pub mod render_inputs;
pub mod source_billboard;
pub mod particle_mesh_emission;
pub mod shadowmap;
pub mod site_material;
pub mod sky;
pub mod site_extension;
pub mod source_curve;
pub mod weather_animation;
#[cfg(test)]
pub mod weather_animation_replay;
pub mod uber_particle;
pub mod ui_particle;
pub mod weather_fx;
pub mod weather_depth;
pub mod source_render_state;
pub mod source_shader;
pub mod source_color;
pub mod source_camera;
pub mod source_particle;
pub mod source_particle_streams;
pub mod source_particle_render;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub mod weather_gpu_tests;
