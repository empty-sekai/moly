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
mod delayed_faces;
pub mod emoticon;
pub mod env;
pub mod fixture;
pub mod fixture_attach;
mod fixture_activity_data;
mod fixture_activity_provider;
mod fixture_gimmick;
mod fixture_tiles;
mod fixture_scene_inputs;
mod fixture_activity_state;
mod fixture_activity_timeline;
pub mod fixture_edit;
mod fixture_edit_ui;
pub mod fixture_emission;
pub mod fixture_material;
pub mod fixture_talk;
mod frame_capture;
pub mod gesture;
pub mod get_resource;
pub mod game_settings;
pub mod harvest;
pub mod harvest_material;
pub mod info;
pub mod inactive_nodes;
pub mod joystick;
pub mod light;
pub mod menu_dialog;
pub mod menu_shell;
#[cfg(not(target_arch = "wasm32"))]
mod native_graphics_diagnostics;
pub mod npc;
pub mod npc_objective;
mod npc_fixture_activity;
pub mod option_dialog;
pub mod pick;
pub mod player;
pub mod player_avatar;
mod player_fixture_action;
pub mod player_state;
pub mod player_talk;
pub mod schedule;
pub mod shadowmap;
pub mod site;
pub mod site_material;
pub mod site_sound;
pub mod sitemap;
pub mod sitemap_phenomena;
mod source_curve;
pub mod sky;
pub mod talk;
pub mod talk_camera;
pub mod talk_window;
pub mod uber_particle;
pub mod ui_layers;
pub mod ui_layout;
mod interaction;
mod particle_runtime;
mod settings_store;
mod material_order;
mod voice_mouth;
mod voice_pcm;
pub mod walk_face;
pub mod weather;
pub mod weather_fx;

use bevy::prelude::*;

/// 唯一的 App 组装点。schedule 细节在 `schedule.rs`；这里只挂 plugin。
/// 资产源与站点选择都在入口 crate 解析成值传入（env / URL 参数的拒绝
/// 点全树只在那一处）；wasm 版多收一份渲染后端设置（探测在 moly-app，
/// 取舍见其 `render_backend`）——0.18 的 `WgpuSettings` 只能从
/// `RenderPlugin.render_creation` 字段进（没有世界资源口），而字段的值在
/// `add_plugins` 时就固定，所以只能在这个组装点走 `DefaultPlugins.set`，
/// 不能 fork 插件。
#[cfg(not(target_arch = "wasm32"))]
pub fn app(source: moly_assets::AssetSource, site: site::SiteRequest) -> App {
    let mut app = App::new();
    // 先于 DefaultPlugins：AssetPlugin 构建时固化全部资产源（见 moly_assets::install）。
    moly_assets::install(&mut app, source);
    app.add_plugins(DefaultPlugins);
    app.add_plugins(native_graphics_diagnostics::NativeGraphicsDiagnosticsPlugin);
    // 清单类 JSON 装载器落在 AssetServer 上，只能在其创建之后注册。
    moly_assets::json::register(&mut app);
    moly_assets::material_textures::register(&mut app);
    // 天空材质的 Assets/管线集合：先于任何引用它的 system 安装。
    sky::install(&mut app);
    // 表情材质的管线集合（sprite 子矩形与粒子 billboard 共用）。
    emoticon::install(&mut app);
    // 天气系统：现象切换 + 后处理轴上屏（render graph 节点）。
    // 两侧同挂——wasm 分支的站点/天空/音频消费面早已补齐，注册面
    // 也同形（此前那边漏挂，三锚永不出现）。
    app.add_plugins(weather::WeatherPlugin);
    // 雨粒子材质同款：先于 schedule 里引用它的 system。
    // 站点侧 UberUnlit 粒子材质同款。
    uber_particle::install(&mut app);
    // 音频域：面板与通道资源先落位（当前档资源在这里兜底建——见该模块）。
    audio::install(&mut app);
    ui_layout::install(&mut app);
    // 站点选择资源由入口参数起值；站点场景观察者随插件挂。
    app.add_plugins(site::SitePlugin(site));
    schedule::install(&mut app);
    // 后于 DefaultPlugins：embedded_asset! 需要 AssetPlugin 的内嵌注册表。
    app.add_plugins(site_material::SiteMaterialPlugin);
    app.add_plugins(material_order::MaterialOrderPlugin);
    app.add_plugins(character_material::CharacterMaterialPlugin);
    // 主光阴影深度图（单图路线）：消费面在站点材质，native 与 wasm 都装。
    app.add_plugins(shadowmap::ShadowmapPlugin);
    // 家具链（装载与摆放 mock · Basic 族材质换装）：材质换装消费站点材质
    // 插件装的全局量 buffer（该插件两侧同挂）。自发光 pass（第二颜色目标
    // 直画进泛光输入缓冲）由换装侧给实体插组件，两侧都装——那边漏挂会
    // 让第二目标在那侧永远空。
    app.add_plugins((
        fixture::FixturePlugin,
        fixture_material::FixtureMaterialPlugin,
        fixture_emission::FixtureEmissionPlugin,
    ));
    // 摆放编辑面：摆放 mock 的交互化——输入与状态机
    // 面板，无材质依赖，native 与 wasm 同装。
    app.add_plugins(fixture_edit::FixtureEditPlugin);
    // 采集物链同款：装载/摆放/交互在本体插件，材质换装在材质插件（消费
    // 站点插件装的全局量 buffer）。
    app.add_plugins((harvest::HarvestPlugin, harvest_material::HarvestMaterialPlugin));
    app
}

#[cfg(target_arch = "wasm32")]
pub fn app(
    source: moly_assets::AssetSource,
    site: site::SiteRequest,
    web_render_settings: bevy::render::settings::WgpuSettings,
) -> App {
    let mut app = App::new();
    // 先于 DefaultPlugins：AssetPlugin 构建时固化全部资产源（见 moly_assets::install）。
    moly_assets::install(&mut app, source);
    app.add_plugins(DefaultPlugins.set(bevy::render::RenderPlugin {
        render_creation: bevy::render::settings::RenderCreation::Automatic(web_render_settings),
        // 其余字段与 `RenderPlugin::default()` 一致。
        ..Default::default()
    }));
    // 清单类 JSON 装载器落在 AssetServer 上，只能在其创建之后注册。
    moly_assets::json::register(&mut app);
    moly_assets::material_textures::register(&mut app);
    // 天空材质的 Assets/管线集合：先于 schedule 里引用它的 system。
    // wasm 侧此前漏挂（native 有）——schedule 的 spawn_when_ready 两分支
    // 共用，缺 Assets<SkyGradient> 即 panic，页面白屏。
    sky::install(&mut app);
    // 雨粒子材质的 Assets/管线集合：先于 schedule 里引用它的 system。
    // 站点侧 UberUnlit 粒子材质同款（两侧同挂：漏挂那侧粒子永不上屏）。
    uber_particle::install(&mut app);
    // 表情材质的管线集合（wasm 分支同样上屏）。
    emoticon::install(&mut app);
    // 天气系统：现象切换 + 后处理轴上屏（render graph 节点）。补挂——
    // 此前 wasm 分支漏挂（native 有），三锚（现象清单解析/天气系统就绪/
    // C 键切换）在 web 永不出现、无现象可切；清单 JSON 的装载本身两侧
    // 同形且正常（sky/audio 消费同一份在先），缺的只是这条注册。
    app.add_plugins(weather::WeatherPlugin);
    // 音频域：面板与通道资源先落位（当前档资源在这里兜底建——见该模块；
    // 天气插件两侧同挂后此兜底照旧，init_resource 幂等）。
    audio::install(&mut app);
    ui_layout::install(&mut app);
    // 站点选择资源由入口参数起值；站点场景观察者随插件挂。
    app.add_plugins(site::SitePlugin(site));
    schedule::install(&mut app);
    // 后于 DefaultPlugins：embedded_asset! 需要 AssetPlugin 的内嵌注册表。
    // 站点材质换装（含全局量 buffer 的建立）：wasm 此前未装，站点网格
    // 一直以 PBR 默认材质渲染（浏览器端「草地贴图丢失/素」的根因）。
    app.add_plugins(site_material::SiteMaterialPlugin);
    app.add_plugins(material_order::MaterialOrderPlugin);
    app.add_plugins(character_material::CharacterMaterialPlugin);
    // 主光阴影深度图同款：站点材质在 wasm 一样收影，深度 pass 两侧都装。
    app.add_plugins(shadowmap::ShadowmapPlugin);
    // 家具链同款：材质换装消费站点插件装的全局量 buffer。自发光 pass
    // 同装（第二颜色目标两侧都要上屏）。
    app.add_plugins((
        fixture::FixturePlugin,
        fixture_material::FixtureMaterialPlugin,
        fixture_emission::FixtureEmissionPlugin,
    ));
    // 摆放编辑面同款：输入与状态机面板，无材质依赖。
    app.add_plugins(fixture_edit::FixtureEditPlugin);
    // 采集物链同款。
    app.add_plugins((harvest::HarvestPlugin, harvest_material::HarvestMaterialPlugin));
    app
}
