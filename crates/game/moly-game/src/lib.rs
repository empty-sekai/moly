//! 游戏本体：Bevy plugin，一个域一个模块。判据是用户眼睛，零测试。
//!
//! 帧次序：0 层输入/决策 → 1 层聚合 → 2 层快照（依据记在 `schedule.rs`）。


#[macro_use]
extern crate moly_core;

// The layer crates' modules resolve as `crate::<module>` here as well.
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
pub use moly_render::billboard;
pub use moly_render::character_material;
pub use moly_render::fixture_timeline_particles;
pub use moly_render::fixture_colors;
pub use moly_render::fixture_emission;
pub use moly_render::fixture_material;
pub use moly_render::gpu_image_release;
pub use moly_render::particle_runtime;
pub use moly_render::particle_geometry;
pub use moly_render::render_inputs;
pub use moly_render::source_billboard;
pub use moly_render::particle_mesh_emission;
pub use moly_render::shadowmap;
pub use moly_render::site_material;
pub use moly_render::sky;
pub use moly_render::site_extension;
pub use moly_render::source_curve;
pub use moly_render::weather_animation;
pub use moly_render::uber_particle;
pub use moly_render::ui_particle;
pub use moly_render::weather_fx;
pub use moly_render::weather_depth;
pub use moly_render::source_render_state;
pub use moly_render::source_shader;
pub use moly_render::source_color;
pub use moly_render::source_camera;
pub use moly_render::source_particle;
pub use moly_render::source_particle_streams;
pub use moly_render::source_particle_render;
pub use moly_audio::audio;
pub use moly_audio::audio_inputs;
pub use moly_audio::audio_sequence;
pub use moly_audio::audio_startup;
pub use moly_audio::site_sound;
pub use moly_audio::voice_pcm;
pub use moly_ui::browser_stage;
pub use moly_ui::gesture;
pub use moly_ui::notice_banner;
pub use moly_ui::item_icon;
pub use moly_ui::screen_fade;
pub use moly_ui::ui_charsets;
pub use moly_ui::ui_inputs;
pub use moly_ui::two_button_dialog;
pub use moly_ui::common_dialog;
pub use moly_ui::ui_layers;
pub use moly_ui::ui_layout;
pub use moly_world::alone_action_runtime;
pub use moly_world::avatar_material;
pub use moly_world::avatar_wear_data;
pub use moly_world::avatar_wear;
pub use moly_world::balloon;
pub use moly_world::bus;
pub use moly_world::camera;
pub use moly_world::camera_site_move;
pub use moly_world::character;
pub use moly_world::character_silhouette;
pub use moly_world::cloth_runtime;
pub use moly_world::cutscene_camera;
pub use moly_world::delayed_faces;
pub use moly_world::delivery_camera;
pub use moly_world::door_law;
pub use moly_world::emoticon;
pub use moly_world::clock;
pub use moly_world::field_modes;
pub use moly_world::fixture;
pub use moly_world::fixture_collision;
pub use moly_world::fixture_activity_data;
pub use moly_world::fixture_activity_provider;
pub use moly_world::fixture_activity_state;
pub use moly_world::fixture_activity_timeline;
pub use moly_world::fixture_attach;
pub use moly_world::fixture_gate_model;
pub use moly_world::fixture_clock;
pub use moly_world::fixture_gimmick;
pub use moly_world::fixture_locator;
pub use moly_world::fixture_player_navigation;
pub use moly_world::fixture_scene_inputs;
pub use moly_world::fixture_talk;
pub use moly_world::fixture_tiles;
pub use moly_world::house;
pub use moly_world::house_doors;
pub use moly_world::interaction;
pub use moly_world::joystick_law;
pub use moly_world::npc;
pub use moly_world::npc_clock;
pub use moly_world::npc_dither;
pub use moly_world::npc_fixture_activity;
pub use moly_world::npc_fixture_talk;
pub use moly_world::npc_gate;
pub use moly_world::npc_look_at;
pub use moly_world::npc_objective;
pub use moly_world::npc_presenter;
pub use moly_world::npc_state;
pub use moly_world::npc_talk_lottery;
pub use moly_world::npc_tweet;
pub use moly_world::npc_view;
pub use moly_world::system_fixture;
pub use moly_world::player;
pub use moly_world::player_avatar;
pub use moly_world::player_fixture_action;
pub use moly_world::player_state;
pub use moly_world::player_talk;
pub use moly_world::plain_background;
pub use moly_world::rank_table;
pub use moly_world::room_appearance;
pub use moly_world::room_door_port;
pub use moly_world::room_shell;
pub use moly_world::userdata_snapshot;
pub use moly_world::server_panel;
pub use moly_world::server_panel_data;
pub use moly_world::stage_layout;
pub use moly_world::site;
pub use moly_world::talk;
pub use moly_world::talk_camera;
pub use moly_world::zoom_player_camera;
pub use moly_world::talk_ingest;
pub use moly_world::talk_window;
pub use moly_world::tween_clock;
pub use moly_world::voice_mouth;
pub use moly_world::walk_face;
pub use moly_world::weather;
pub use moly_world::weather_stock_post;
pub use moly_features::action_button;
pub use moly_features::bgm_select;
pub use moly_features::birthday;
pub use moly_features::browser_game;
pub use moly_features::content_library;
pub use moly_features::cutscene;
pub use moly_features::delivery;
pub use moly_features::entry;
pub use moly_features::fixture_edit;
pub use moly_features::fixture_edit_ui;
pub use moly_features::floor_edit_camera;
pub use moly_features::footstep;
pub use moly_features::game_settings;
pub use moly_features::game_state;
pub use moly_features::gate_flow;
pub use moly_features::get_resource;
pub use moly_features::learn_phenomena_dialog;
pub use moly_features::harvest;
pub use moly_features::harvest_material;
pub use moly_features::home_action;
pub use moly_features::info;
pub use moly_features::inventory;
pub use moly_features::joystick;
pub use moly_features::menu_dialog;
pub use moly_features::menu_shell;
pub use moly_features::collect_notice;
pub use moly_features::mysekai_rank;
pub use moly_features::option_dialog;
pub use moly_features::pick;
pub use moly_features::player_input;
pub use moly_features::player_data;
pub use moly_features::player_data_ui;
pub use moly_features::userdata;
pub use moly_features::site_expansion;
pub use moly_features::site_move;
pub use moly_features::site_object_dither;
pub use moly_features::sitemap;
pub use moly_features::sitemap_phenomena;
pub use moly_features::stamina_recover_dialog;
pub use moly_features::weather_dialog;
pub use moly_devtools::control;
pub use moly_devtools::control_panel;
pub use moly_devtools::director;
pub use moly_devtools::fixture_walk;
#[cfg(not(target_arch = "wasm32"))]
pub use moly_devtools::portraits;
pub use moly_devtools::talk_probe;
pub use browser_stage::configure_browser_stage;
pub mod schedule;



// The shared leaf modules live in `moly-core`; the re-export keeps every
// `crate::<module>` path in this crate resolving.
pub(crate) use moly_core::animation_time;
pub(crate) use moly_core::asset_cache;
#[cfg(target_arch = "wasm32")]
pub(crate) use moly_core::browser_log;
pub use moly_core::client_config;
pub(crate) use moly_core::dev_tools;
pub(crate) use moly_core::frame_capture;
pub(crate) use moly_core::hold;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use moly_core::native_graphics_diagnostics;
pub(crate) use moly_core::pipeline_report;
pub(crate) use moly_core::run_mode;
pub(crate) use moly_core::settings_store;
pub(crate) use moly_core::world_seed;

use bevy::prelude::*;

pub use browser_game::{
    configure_browser_game, game_command, game_mode_active, game_server_document,
    game_server_schema, game_snapshot, game_take_persist, install_game_storage, parse_game_seed,
    GameHost, GameSeed,
};
#[cfg(not(target_arch = "wasm32"))]
pub use browser_game::native_game_seed;
pub use content_library::bridge::{configure_browser_library, library_catalog, library_command, library_snapshot};
pub use content_library::library_diagnostics;
pub use control::control_request;
pub use moly_core::dev_tools::{insert_dev_tools, DevTools};
pub use director::{director_command, director_snapshot};

/// Product version shared by the settings panel and application entry points.
pub use moly_core::VERSION;

pub use moly_core::settings_store::set_browser_storage_writable;

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
    player_avatar::oc::source::register_source(&mut app);
    let plugins = DefaultPlugins
        .set(bevy::window::WindowPlugin {
            primary_window: Some(Window {
                title: format!("moly v{VERSION}"),
                // A native run may start with the window hidden
                // (`MOLY_WINDOW_HIDDEN`) for headless batches.
                #[cfg(not(target_arch = "wasm32"))]
                visible: std::env::var_os("MOLY_WINDOW_HIDDEN").is_none(),
                ..default()
            }),
            ..default()
        });
    // On wasm the web asset plugin would add an `https` source that reaches
    // any host; remote resources go only through the admitted bases of
    // `moly_assets::remote`, which use the same reader. Native builds have no
    // such plugin.
    #[cfg(target_arch = "wasm32")]
    let plugins = plugins.disable::<bevy::asset::io::web::WebAssetPlugin>();
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
    // A host that seats no documents leaves the instance a non-game one; the
    // game's host replaces this before the first frame.
    app.init_resource::<run_mode::RunMode>();
    // Every Time reader sees the engine's Time.maximumDeltaTime as the longest
    // frame, not the virtual clock's own 250 ms default.
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(particle_runtime::PLAYER_MAXIMUM_DELTA);
    app.add_plugins(game_settings::perf_plugin());
    app.add_plugins(mesh_buffer_release::MeshBufferReleasePlugin);
    gpu_image_release::install(&mut app);
    pipeline_report::install(&mut app);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(native_graphics_diagnostics::NativeGraphicsDiagnosticsPlugin);

    moly_assets::json::register(&mut app);
    moly_assets::source_shader::register(&mut app);
    app.add_plugins(source_shader::SourceShaderPlugin);
    moly_assets::material_textures::register(&mut app);
    sky::install(&mut app);
    emoticon::install(&mut app);
    collect_notice::install(&mut app);
    system_fixture::install(&mut app);
    bgm_select::install(&mut app);
    item_icon::install(&mut app);
    app.add_plugins(weather::WeatherPlugin);
    uber_particle::install(&mut app);
    audio::install(&mut app);
    input_claims::install(&mut app);
    ui_layout::install(&mut app);
    app.add_plugins(site::SitePlugin(site));
    talk_window::install(&mut app);
    schedule::install(&mut app);
    app.add_plugins(userdata::UserDataPlugin);
    app.add_plugins(server_panel::ServerPanelPlugin);
    app.add_plugins(npc_state::NpcStatePlugin);
    site_move::install(&mut app);
    screen_fade::install(&mut app);
    overlay_camera::install(&mut app);
    game_state::install(&mut app);
    site_expansion::install(&mut app);
    cutscene_camera::install(&mut app);
    cutscene::install(&mut app);
    gate_flow::install(&mut app);
    footstep::install(&mut app);
    app.add_plugins(site_material::SiteMaterialPlugin);
    site_object_dither::install(&mut app);
    room_appearance::install(&mut app);
    plain_background::install(&mut app);
    app.add_plugins(material_order::MaterialOrderPlugin);
    app.add_plugins(character_material::CharacterMaterialPlugin);
    app.add_plugins(avatar_material::AvatarMaterialPlugin);
    player_avatar::oc::install(&mut app);
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
    ));
    app.add_plugins(delivery::DeliveryPlugin);
    app.add_plugins(moly_features::convert::ConvertPlugin);
    app.add_plugins(moly_features::mission::MissionPlugin);
    app.add_plugins(ui_particle::UiParticlePlugin);
    common_dialog::install(&mut app);
    moly_ui::subwindow_dialog::install(&mut app);
    home_action::install(&mut app);
    app
}

