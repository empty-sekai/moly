//! 游戏本体：Bevy plugin，一个域一个模块。判据是用户眼睛，零测试。
//!
//! 帧次序：0 层输入/决策 → 1 层聚合 → 2 层快照（依据记在 `schedule.rs`）。

pub mod action_button;
pub mod alone_action_runtime;
pub mod audio;
mod audio_sequence;
mod asset_cache;
mod audio_startup;
pub mod avatar_material;
pub mod avatar_wear;
pub mod balloon;
pub mod billboard;
pub mod birthday;
mod browser_game;
mod browser_stage;
#[cfg(target_arch = "wasm32")]
mod browser_log;
pub use browser_stage::configure_browser_stage;
pub mod camera;
pub mod canvas;
pub mod character;
pub mod character_material;
mod character_silhouette;
pub mod client_config;
pub mod cloth_runtime;
mod content_library;
mod cutscene;
mod cutscene_camera;
mod delayed_faces;
pub mod delivery;
mod delivery_camera;
mod dev_tools;
pub mod emoticon;
mod entry;
pub mod env;
pub mod fixture;
mod fixture_collision;
mod fixture_activity_data;
mod fixture_activity_provider;
mod fixture_activity_state;
mod fixture_activity_timeline;
mod fixture_timeline_particles;
pub mod fixture_attach;
mod fixture_colors;
pub mod fixture_edit;
mod fixture_edit_ui;
mod floor_edit_camera;
pub mod fixture_emission;
mod fixture_clock;
mod fixture_gimmick;
pub mod fixture_material;
mod fixture_player_navigation;
mod fixture_scene_inputs;
pub mod fixture_talk;
mod fixture_tiles;
mod footstep;
mod frame_capture;
pub mod game_settings;
mod game_state;
mod gate_flow;
pub mod gesture;
pub mod get_resource;
pub mod learn_phenomena_dialog;
mod gpu_image_release;
pub mod harvest;
pub mod harvest_material;
mod home_action;
mod harvest_particles;
pub mod inactive_nodes;
pub mod info;
mod interaction;
pub mod joystick;
pub mod light;
mod material_order;
mod mesh_buffer_release;
pub mod menu_dialog;
pub mod menu_shell;
pub mod notice_banner;
mod mysekai_rank;
#[cfg(not(target_arch = "wasm32"))]
mod native_graphics_diagnostics;
pub mod npc;
mod npc_clock;
mod npc_dither;
mod npc_fixture_activity;
mod npc_fixture_talk;
mod npc_gate;
mod npc_look_at;
#[cfg(test)]
mod npc_harness;
pub mod npc_objective;
mod npc_presenter;
mod npc_state;
mod npc_talk_lottery;
mod npc_tweet;
mod npc_view;
pub mod option_dialog;
mod particle_runtime;
mod particle_geometry;
mod source_billboard;
mod particle_mesh_emission;
pub mod pick;
pub mod player;
pub mod player_avatar;
pub mod player_data;
#[cfg(not(target_arch = "wasm32"))]
pub mod portraits;
mod player_data_io;
mod player_data_ui;
mod player_fixture_action;
pub mod player_state;
pub mod player_talk;
mod plain_background;
mod render;
mod room_appearance;
mod room_shell;
pub mod schedule;
mod screen_fade;
mod server;
mod server_panel;
mod settings_store;
pub mod shadowmap;
pub mod site;
pub mod site_material;
pub mod site_sound;
mod site_expansion;
pub(crate) mod site_move;
pub mod sitemap;
pub mod sitemap_phenomena;
pub mod sky;
pub mod site_extension;
mod source_curve;
mod weather_animation;
#[cfg(test)]
mod weather_animation_replay;
pub mod talk;
pub mod talk_camera;
mod zoom_player_camera;
mod talk_ingest;
pub mod talk_window;
pub mod uber_particle;
pub mod ui_layers;
pub mod ui_layout;
mod voice_mouth;
mod voice_pcm;
pub mod walk_face;
pub mod weather;
pub mod weather_fx;
mod weather_stock_post;
mod weather_depth;
mod weather_transition;
mod source_render_state;
mod source_shader;
mod source_color;
mod source_camera;
mod source_particle;
mod source_particle_streams;
mod source_particle_render;

use bevy::prelude::*;

pub use browser_game::{
    configure_browser_game, game_command, game_mode_active, game_server_document,
    game_server_schema, game_snapshot, game_take_persist, install_game_storage, parse_game_seed,
    GameSeed,
};
pub use content_library::bridge::{configure_browser_library, library_catalog, library_command, library_snapshot};
pub use content_library::library_diagnostics;
pub use dev_tools::{insert_dev_tools, DevTools};

/// Product version shared by the settings panel and application entry points.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(target_arch = "wasm32")]
pub use settings_store::set_browser_storage_writable;

/// Shared native/browser assembly. Only renderer initialization and native
/// diagnostics vary by platform; every gameplay plugin is installed here.
pub fn app(
    source: moly_assets::AssetSource,
    site: site::SiteRequest,
    #[cfg(target_arch = "wasm32")] web_render_settings: bevy::render::settings::WgpuSettings,
) -> App {
    let mut app = App::new();
    #[cfg(target_arch = "wasm32")]
    browser_log::install();
    // Asset sources must be registered before AssetPlugin is built.
    moly_assets::install(&mut app, source);
    let plugins = DefaultPlugins.set(bevy::window::WindowPlugin {
        primary_window: Some(Window {
            title: format!("moly v{VERSION}"),
            ..default()
        }),
        ..default()
    });
    #[cfg(target_arch = "wasm32")]
    // The browser host reports failures to its retry page. Keep that hook.
    let plugins =
        plugins
            .disable::<bevy::app::PanicHandlerPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(
                    web_render_settings,
                ),
                ..default()
            });
    app.add_plugins(plugins);
    // Every Time reader sees the engine's Time.maximumDeltaTime as the longest
    // frame, not the virtual clock's own 250 ms default.
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(particle_runtime::PLAYER_MAXIMUM_DELTA);
    app.add_plugins(game_settings::perf_plugin());
    app.add_plugins(mesh_buffer_release::MeshBufferReleasePlugin);
    gpu_image_release::install(&mut app);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(native_graphics_diagnostics::NativeGraphicsDiagnosticsPlugin);

    moly_assets::json::register(&mut app);
    moly_assets::source_shader::register(&mut app);
    app.add_plugins(source_shader::SourceShaderPlugin);
    moly_assets::material_textures::register(&mut app);
    sky::install(&mut app);
    emoticon::install(&mut app);
    app.add_plugins(weather::WeatherPlugin);
    uber_particle::install(&mut app);
    audio::install(&mut app);
    ui_layout::install(&mut app);
    app.add_plugins(site::SitePlugin(site));
    schedule::install(&mut app);
    app.add_plugins(server::ServerPlugin);
    app.add_plugins(server_panel::ServerPanelPlugin);
    app.add_plugins(npc_state::NpcStatePlugin);
    site_move::install(&mut app);
    screen_fade::install(&mut app);
    game_state::install(&mut app);
    site_expansion::install(&mut app);
    cutscene_camera::install(&mut app);
    cutscene::install(&mut app);
    gate_flow::install(&mut app);
    footstep::install(&mut app);
    app.add_plugins(site_material::SiteMaterialPlugin);
    room_appearance::install(&mut app);
    plain_background::install(&mut app);
    app.add_plugins(material_order::MaterialOrderPlugin);
    app.add_plugins(character_material::CharacterMaterialPlugin);
    app.add_plugins(avatar_material::AvatarMaterialPlugin);
    app.add_plugins(shadowmap::ShadowmapPlugin);
    app.add_plugins((
        fixture::FixturePlugin,
        fixture_material::FixtureMaterialPlugin,
        fixture_emission::FixtureEmissionPlugin,
    ));
    app.add_plugins(character_silhouette::CharacterSilhouettePlugin);
    app.add_plugins(fixture_edit::FixtureEditPlugin);
    app.add_plugins((source_color::SourceColorPlugin, source_particle_render::SourceParticlePlugin));
    app.add_plugins((
        harvest::HarvestPlugin,
        harvest_material::HarvestMaterialPlugin,
        harvest_particles::HarvestParticlePlugin,
    ));
    app.add_plugins(delivery::DeliveryPlugin);
    home_action::install(&mut app);
    app
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod weather_gpu_tests;
