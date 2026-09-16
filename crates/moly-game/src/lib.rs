//! 游戏本体：Bevy plugin，一个域一个模块。判据是用户眼睛，零测试。
//!
//! 帧次序：0 层输入/决策 → 1 层聚合 → 2 层快照（依据记在 `schedule.rs`）。

pub mod action_button;
pub mod alone_action_runtime;
pub mod audio;
pub mod avatar_material;
pub mod avatar_wear;
pub mod balloon;
pub mod billboard;
pub mod birthday;
pub mod camera;
pub mod character;
pub mod character_material;
pub mod client_config;
pub mod cloth_runtime;
mod content_library;
mod delayed_faces;
pub mod emoticon;
pub mod env;
pub mod fixture;
mod fixture_activity_data;
mod fixture_activity_provider;
mod fixture_activity_state;
mod fixture_activity_timeline;
pub mod fixture_attach;
mod fixture_colors;
pub mod fixture_edit;
mod fixture_edit_ui;
pub mod fixture_emission;
mod fixture_gimmick;
pub mod fixture_material;
mod fixture_player_navigation;
mod fixture_scene_inputs;
pub mod fixture_talk;
mod fixture_tiles;
mod frame_capture;
pub mod game_settings;
pub mod gesture;
pub mod get_resource;
pub mod harvest;
pub mod harvest_material;
pub mod inactive_nodes;
pub mod info;
mod interaction;
pub mod joystick;
pub mod light;
mod material_order;
pub mod menu_dialog;
pub mod menu_shell;
#[cfg(not(target_arch = "wasm32"))]
mod native_graphics_diagnostics;
pub mod npc;
mod npc_fixture_activity;
pub mod npc_objective;
pub mod option_dialog;
mod particle_runtime;
pub mod pick;
pub mod player;
pub mod player_avatar;
pub mod player_data;
mod player_data_io;
mod player_data_ui;
mod player_fixture_action;
pub mod player_state;
pub mod player_talk;
mod room_appearance;
pub mod schedule;
mod settings_store;
pub mod shadowmap;
pub mod site;
pub mod site_material;
pub mod site_sound;
pub mod sitemap;
pub mod sitemap_phenomena;
pub mod sky;
mod source_curve;
pub mod talk;
pub mod talk_camera;
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

use bevy::prelude::*;

pub use content_library::bridge::{configure_browser_library, library_command, library_snapshot};

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
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(
                    web_render_settings,
                ),
                ..default()
            });
    app.add_plugins(plugins);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(native_graphics_diagnostics::NativeGraphicsDiagnosticsPlugin);

    moly_assets::json::register(&mut app);
    moly_assets::material_textures::register(&mut app);
    sky::install(&mut app);
    emoticon::install(&mut app);
    app.add_plugins(weather::WeatherPlugin);
    uber_particle::install(&mut app);
    audio::install(&mut app);
    ui_layout::install(&mut app);
    app.add_plugins(site::SitePlugin(site));
    schedule::install(&mut app);
    app.add_plugins(site_material::SiteMaterialPlugin);
    room_appearance::install(&mut app);
    app.add_plugins(material_order::MaterialOrderPlugin);
    app.add_plugins(character_material::CharacterMaterialPlugin);
    app.add_plugins(shadowmap::ShadowmapPlugin);
    app.add_plugins((
        fixture::FixturePlugin,
        fixture_material::FixtureMaterialPlugin,
        fixture_emission::FixtureEmissionPlugin,
    ));
    app.add_plugins(fixture_edit::FixtureEditPlugin);
    app.add_plugins((
        harvest::HarvestPlugin,
        harvest_material::HarvestMaterialPlugin,
    ));
    app
}
