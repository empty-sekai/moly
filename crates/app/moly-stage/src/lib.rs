//! Embedded field runtime. The host owns the catalogue and page controls.
//! This assembly does not depend on moly-game or install its progression,
//! inventory, layout-editor, developer-console or full-game lifecycle plugins.

pub use moly_audio::audio;
pub use moly_audio::audio_startup;
pub use moly_audio::site_sound;
pub use moly_audio::voice_pcm;
pub(crate) use moly_core::animation_time;
#[cfg(target_arch = "wasm32")]
pub(crate) use moly_core::browser_log;
pub use moly_core::client_config;
pub use moly_core::env;
pub use moly_core::inactive_nodes;
pub use moly_core::input_claims;
pub use moly_core::light;
pub use moly_core::markers;
pub use moly_core::material_order;
pub use moly_core::mesh_buffer_release;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use moly_core::native_graphics_diagnostics;
pub use moly_core::overlay_camera;
pub use moly_core::render;
pub(crate) use moly_core::run_mode;
pub use moly_core::ui_font;
pub use moly_core::weather_transition;
pub use moly_features::action_button;
pub use moly_features::birthday;
pub use moly_features::content_library;
pub use moly_features::cutscene;
pub use moly_features::entry;
pub use moly_features::fixture_edit;
pub use moly_features::footstep;
pub use moly_features::game_settings;
pub use moly_features::game_state;
pub use moly_features::info;
pub use moly_features::joystick;
pub use moly_features::mysekai_rank;
pub use moly_features::pick;
pub use moly_features::player_data;
pub use moly_features::player_input;
pub use moly_features::site_move;
pub use moly_features::site_object_dither;
pub use moly_features::sitemap;
pub use moly_features::userdata;
pub use moly_render::character_material;
pub use moly_render::fixture_colors;
pub use moly_render::fixture_emission;
pub use moly_render::fixture_material;
pub use moly_render::gpu_image_release;
pub use moly_render::particle_runtime;
pub use moly_render::shadowmap;
pub use moly_render::site_material;
pub use moly_render::sky;
pub use moly_render::source_color;
pub use moly_render::source_particle_render;
pub use moly_render::source_shader;
pub use moly_render::uber_particle;
pub use moly_render::ui_particle;
pub use moly_render::weather_fx;
pub use moly_ui::browser_stage;
pub use moly_ui::common_dialog;
pub use moly_ui::gesture;
pub use moly_ui::screen_fade;
pub use moly_ui::ui_charsets;
pub use moly_ui::ui_inputs;
pub use moly_ui::ui_layers;
pub use moly_ui::ui_layout;
pub use moly_world::alone_action_runtime;
pub use moly_world::avatar_material;
pub use moly_world::balloon;
pub use moly_world::camera;
pub use moly_world::character;
pub use moly_world::character_silhouette;
pub use moly_world::cloth_runtime;
pub use moly_world::cutscene_camera;
pub use moly_world::emoticon;
pub use moly_world::field_modes;
pub use moly_world::fixture;
pub use moly_world::fixture_activity_data;
pub use moly_world::fixture_activity_provider;
pub use moly_world::fixture_activity_state;
pub use moly_world::fixture_activity_timeline;
pub use moly_world::fixture_attach;
pub use moly_world::fixture_clock;
pub use moly_world::fixture_collision;
pub use moly_world::fixture_gimmick;
pub use moly_world::fixture_player_navigation;
pub use moly_world::fixture_scene_inputs;
pub use moly_world::fixture_talk;
pub use moly_world::house;
pub use moly_world::npc;
pub use moly_world::npc_fixture_activity;
pub use moly_world::npc_gate;
pub use moly_world::npc_look_at;
pub use moly_world::npc_objective;
pub use moly_world::npc_presenter;
pub use moly_world::npc_state;
pub use moly_world::npc_tweet;
pub use moly_world::plain_background;
pub use moly_world::player;
pub use moly_world::player_avatar;
pub use moly_world::player_fixture_action;
pub use moly_world::player_state;
pub use moly_world::player_talk;
pub use moly_world::room_appearance;
pub use moly_world::server_panel;
pub use moly_world::site;
pub use moly_world::system_fixture;
pub use moly_world::talk;
pub use moly_world::talk_camera;
pub use moly_world::talk_window;
pub use moly_world::voice_mouth;
pub use moly_world::walk_face;
pub use moly_world::weather;

use bevy::prelude::*;
pub use moly_core::VERSION;
#[cfg(target_arch = "wasm32")]
mod browser;
pub mod schedule;
#[cfg(target_arch = "wasm32")]
pub use browser::*;

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
    let plugins = DefaultPlugins.set(bevy::window::WindowPlugin {
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
    let plugins = plugins
        .disable::<bevy::app::PanicHandlerPlugin>()
        .disable::<bevy::log::LogPlugin>()
        .set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::RenderCreation::Automatic(web_render_settings),
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
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(native_graphics_diagnostics::NativeGraphicsDiagnosticsPlugin);

    moly_assets::json::register(&mut app);
    moly_assets::source_shader::register(&mut app);
    app.add_plugins(source_shader::SourceShaderPlugin);
    moly_assets::material_textures::register(&mut app);
    sky::install(&mut app);
    emoticon::install(&mut app);
    system_fixture::install(&mut app);
    app.add_plugins(weather::WeatherPlugin);
    uber_particle::install(&mut app);
    audio::install(&mut app);
    input_claims::install(&mut app);
    ui_layout::install(&mut app);
    app.add_plugins(site::SitePlugin(site));
    talk_window::install(&mut app);
    schedule::install(&mut app);
    browser_stage::configure_browser_stage(&mut app);
    content_library::bridge::configure_browser_library(&mut app);
    app.add_plugins(userdata::UserDataPlugin);
    app.add_plugins(server_panel::ServerPanelPlugin);
    app.add_plugins(npc_state::NpcStatePlugin);
    site_move::install(&mut app);
    screen_fade::install(&mut app);
    overlay_camera::install(&mut app);
    game_state::install(&mut app);
    cutscene_camera::install(&mut app);
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
    app.add_plugins((
        source_color::SourceColorPlugin,
        source_particle_render::SourceParticlePlugin,
    ));
    app.add_plugins(ui_particle::UiParticlePlugin);
    common_dialog::install(&mut app);
    // Door transitions can publish this lifecycle notification; stage has no home menu.
    app.add_message::<cutscene::HomeScreenStartAnimation>();
    app
}
