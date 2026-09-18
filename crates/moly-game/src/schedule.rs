//! schedule 的唯一住所。system 在表里就跑，不在表里就不存在。
//!
//! 层序的依据：全部律的无上游依赖集合算出来是 58 族（输入/决策层），
//! 其余按「写者先于读者」排成 1 层聚合、2 层快照。此处只编码结论，
//! 推导过程在私有研究仓，不在本仓。

use crate::{
    action_button, alone_action_runtime, audio, balloon, birthday, camera, character,
    character_material, client_config, cloth_runtime, content_library, emoticon, fixture_attach,
    fixture_edit, fixture_talk, gesture, get_resource, harvest, inactive_nodes, info, joystick,
    light, menu_dialog, menu_shell, npc, npc_objective, option_dialog, pick, player, player_avatar,
    player_state, player_talk, site, site_sound, sitemap, sky, talk, talk_camera, talk_window,
    uber_particle, ui_layers, walk_face, weather, weather_fx,
};
use bevy::app::{AnimationSystems, App, PostUpdate, PreUpdate, SpawnScene, Startup, Update};
use bevy::prelude::{ApplyDeferred, IntoScheduleConfigs};
use bevy::scene::SceneSpawnerSystems;
use bevy::time::common_conditions;
use bevy::transform::TransformSystems;
use std::time::Duration;

pub fn install(app: &mut App) {
    content_library::install(app);
    app.add_systems(Startup, content_library::load_capabilities);
    app.add_systems(
        PreUpdate,
        content_library::bridge::gate_input
            .after(bevy::input::InputSystems)
            .before(crate::game_settings::input)
            .before(content_library::input),
    );
    app.add_systems(
        Update,
        content_library::bridge::publish.after(content_library::refresh),
    );
    app.add_systems(
        Update,
        (
            content_library::parse_assets,
            content_library::parse_capabilities,
            content_library::build_talk_catalog,
            content_library::build_activity_catalog,
            content_library::refresh_context,
            content_library::prepare_pending,
            content_library::dispatch,
        )
            .chain()
            .after(crate::player_fixture_action::refresh_availability)
            .before(player_talk::consume_trigger)
            .before(crate::player_fixture_action::receive_requests),
    )
    .add_systems(
        Update,
        content_library::refresh
            .after(player_talk::advance_session)
            .after(talk::advance_talk)
            .after(crate::player_fixture_action::advance),
    )
    .add_systems(
        Update,
        content_library::qa_open
            .after(content_library::retire_scene)
            .after(content_library::build_talk_catalog)
            .before(content_library::refresh),
    )
    .add_systems(
        Update,
        content_library::observe_start
            .after(crate::npc_fixture_activity::advance)
            .after(crate::fixture_gimmick::advance)
            .after(player_talk::advance_session)
            .after(talk::advance_talk)
            .after(crate::player_fixture_action::advance)
            .before(content_library::refresh),
    );
    app.add_systems(
        Update,
        (
            content_library::reap_preview_owners,
            content_library::retire_scene,
        )
            .chain()
            .after(content_library::observe_start)
            .before(content_library::refresh),
    );
    // UI consumes a pointer before the editor/world rays. Commands finish
    // before gameplay input; the early fixture loader intentionally depends
    // only on keyboard Input and observes committed reloads next frame.
    app.configure_sets(
        Update,
        (
            // Flush the previous loader generation's deferred material writes
            // before Save can tear down its roots and readiness markers.
            crate::fixture_material::FixtureMaterialSet
                .before(fixture_edit::FixtureEditSystems::Commands),
            fixture_edit::FixtureEditSystems::Pointer
                .after(menu_shell::click)
                .before(pick::pick),
            fixture_edit::FixtureEditSystems::Commands
                .after(menu_shell::click)
                .before(npc_objective::decide)
                .before(npc::advance)
                .before(pick::pick),
            fixture_edit::FixtureEditSystems::View.before(ui_layers::advance),
        ),
    );
    // DefaultPlugins and audio::install are already present on native and wasm.
    // Register the existing voice player's metered source, not a second bus.
    crate::voice_pcm::install(app);
    app.init_resource::<crate::voice_mouth::MouthRandom>()
        .add_systems(
            Update,
            (
                ApplyDeferred,
                crate::voice_pcm::prepare,
                ApplyDeferred,
                crate::voice_pcm::report_start,
                crate::voice_mouth::attach,
                ApplyDeferred,
                crate::voice_mouth::advance,
            )
                .chain()
                .after(audio::serve_voice)
                .after(alone_action_runtime::attach)
                .after(alone_action_runtime::advance)
                .after(talk::advance_talk)
                .after(player_talk::advance_session)
                .after(crate::fixture_activity_timeline::advance),
        );
    app.init_resource::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .init_resource::<crate::npc_fixture_activity::NpcFixtureAreas>()
        .add_systems(
            Update,
            crate::npc_fixture_activity::advance
                .after(content_library::dispatch)
                .after(npc::advance)
                .after(crate::fixture_scene_inputs::advance)
                .after(crate::fixture_activity_provider::advance)
                .before(crate::fixture_activity_timeline::advance),
        );
    app.init_resource::<crate::fixture_scene_inputs::FixtureFloorAreas>()
        .init_resource::<crate::fixture_scene_inputs::FixtureSceneSupply>()
        .add_systems(
            Update,
            crate::fixture_scene_inputs::advance
                .after(player::reseed)
                .after(crate::fixture::refresh_activity_view)
                .after(crate::fixture_edit::FixtureEditSystems::Commands)
                .before(crate::player_fixture_action::refresh_availability),
        );
    app.add_systems(
        Update,
        crate::fixture_player_navigation::publish
            .after(crate::fixture_scene_inputs::advance)
            .after(npc_objective::build_face)
            .after(walk_face::rebake_on_save)
            .before(content_library::refresh_context)
            .before(crate::player_fixture_action::refresh_availability)
            .before(crate::player_fixture_action::receive_requests),
    );
    app.init_resource::<crate::fixture_activity_state::FixtureActivityReservations>()
        .init_resource::<crate::fixture_activity_timeline::FixtureActivityTimelines>()
        .init_resource::<crate::player_fixture_action::PlayerFixtureRuntime>()
        .init_resource::<crate::player_fixture_action::PlayerFixtureVisualProfiles>()
        .init_resource::<crate::fixture_gimmick::Gimmicks>()
        .add_systems(Startup, crate::fixture_activity_data::load)
        .add_systems(Startup, crate::fixture_gimmick::load)
        .add_systems(Startup, player_avatar::switch_gesture::load)
        .add_systems(
            Update,
            crate::fixture_gimmick::parse.before(crate::player_fixture_action::advance),
        )
        .add_systems(Update, crate::fixture_activity_data::parse)
        .add_systems(
            Update,
            crate::fixture_gimmick::catalog_stream::advance
                .after(crate::fixture_activity_data::parse)
                .after(crate::fixture::refresh_activity_view)
                .before(crate::player_fixture_action::refresh_availability),
        )
        .add_systems(
            Update,
            crate::fixture_activity_provider::advance
                .after(crate::fixture_activity_data::parse)
                .after(crate::fixture::refresh_activity_view)
                .after(character::wire_when_ready)
                .before(crate::player_fixture_action::refresh_availability),
        )
        .add_systems(
            Update,
            crate::fixture_activity_provider::report
                .after(crate::fixture_activity_provider::advance)
                .run_if(common_conditions::on_timer(Duration::from_secs(2))),
        );
    crate::game_settings::install(app);
    crate::player_data::install(app);
    crate::fixture_colors::install(app);
    app.add_message::<crate::frame_capture::CaptureFrame>()
        .add_systems(PostUpdate, crate::frame_capture::capture);
    app.add_systems(
        Startup,
        crate::game_settings::setup
            .after(audio::init_settings)
            .after(info::init)
            .after(camera::spawn),
    )
    .add_systems(Startup, content_library::setup.after(camera::spawn))
    .add_systems(
        PreUpdate,
        crate::game_settings::input
            .after(bevy::ui::UiSystems::Focus)
            .run_if(crate::browser_stage::standalone),
    )
    .add_systems(
        PreUpdate,
        content_library::input
            .after(bevy::ui::UiSystems::Focus)
            .after(crate::game_settings::input),
    )
    .add_systems(
        Update,
        (
            crate::game_settings::apply_graphics.after(info::click),
            crate::game_settings::refresh_ui,
        )
            .chain(),
    )
    .add_systems(PostUpdate, audio::apply_sink_volumes);
    app.add_systems(Startup, crate::ui_layout::load)
        .add_systems(Update, crate::ui_layout::parse.before(menu_shell::parse))
        .add_systems(
            Update,
            crate::ui_layout::render
                .after(menu_shell::place)
                .after(menu_dialog::place)
                .after(option_dialog::place)
                .after(info::place)
                .after(get_resource::place),
        )
        .add_systems(
            Update,
            (
                uber_particle::request_fixture_particles,
                uber_particle::plan_fixture_particles,
                uber_particle::spawn_fixture_particles,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            uber_particle::advance_fixture_particles
                .after(TransformSystems::Propagate)
                .after(camera::follow_avatar),
        );
    app.add_systems(Startup, crate::fixture_edit_ui::load)
        .add_systems(
            Update,
            (
                crate::fixture_edit_ui::parse,
                crate::fixture_edit_ui::spawn_when_ready,
            )
                .chain()
                .after(crate::ui_layout::parse),
        )
        .add_systems(
            Update,
            crate::fixture_edit_ui::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(action_button::click)
                .after(menu_shell::click)
                .before(fixture_edit::FixtureEditSystems::Pointer)
                .before(fixture_edit::FixtureEditSystems::Commands),
        )
        .add_systems(
            Update,
            crate::fixture_edit_ui::refresh
                .after(crate::fixture_edit_ui::spawn_when_ready)
                .after(fixture_edit::FixtureEditSystems::View)
                .after(ui_layers::advance)
                .before(crate::ui_layout::render),
        );
    app.add_message::<gesture::GestureEvent>()
        .add_message::<player_talk::PlayerTalkRequest>()
        .add_message::<crate::player_fixture_action::PlayerFixtureRequest>()
        // 摆设编辑的保存回执沿（保存动作 → tweet 域 after-edit 反应）。
        .add_message::<fixture_edit::LayoutSaved>()
        .init_resource::<gesture::GestureLayerState>()
        // 摇杆层状态（手势层与玩家输入都读它；缺资源会在系统参数校验
        // 处 panic，与手势层同款兜底）。
        .init_resource::<joystick::JoystickState>()
        // SitePlugin owns the site SceneInstanceReady observer. Registering it
        // again here would decrement SiteScenePending twice for one scene root.
        .add_observer(character::on_model_scene_ready)
        // 气泡链的选取门要读当前现象 id；wasm 分支不装天气插件，资源在
        // 这里兜底建（默认值 = 真源默认现象 1，与档名资源在音频侧兜底建
        // 同款故事；native 上与天气插件的 init 幂等重合）。
        .init_resource::<weather::CurrentPhenomenonId>()
        .init_resource::<alone_action_runtime::AloneExecutionGate>()
        // 家具时间轴可播集（常驻空表起步，装载期填充——步进系统按 Res
        // 读它，缺资源会在系统参数校验处 panic）。
        .init_resource::<fixture_talk::FixtureTimelines>()
        // 场地相机当前态（常驻 Normal 起步；位移律读它，缺资源同样在
        // 参数校验处 panic）。
        .init_resource::<camera::FieldCameraState>()
        // 玩家 avatar 状态机（常驻 Idle、截获门开起步——真源 Setup 期
        // 31 个状态构造把门开成；相机近距进入守卫读它，缺资源同样在
        // 参数校验处 panic）。
        .init_resource::<player_state::PlayerAvatarStates>()
        // 近距态私有旋转镜像（跨会话存续，源常驻单例的等价物）。
        .init_resource::<camera::FpsViewMemory>()
        // 前站点类型（近距态退出分道读它；零初始化 = home_site，源字段零值）。
        .init_resource::<camera::PrevSiteType>()
        // 前站点类型记账：PreUpdate 读上一帧落定的 SiteActive，换站即记
        // 前值（相机域独养，站点域零改动）。
        .add_systems(PreUpdate, camera::track_prev_site)
        .add_systems(
            Startup,
            (
                (
                    site::load,
                    character::load,
                    camera::spawn,
                    light::spawn,
                    npc::load,
                    player::load,
                    sky::load,
                    // 角色全局量表：中性 → 晴天常量 → 雾档案请求，链内依次
                    // （commands 在同步点落地，链保证下一行读得到）。
                    character_material::insert_neutral,
                    character_material::apply_sunny,
                    character_material::load_fog,
                )
                    .chain(),
                (
                    // ClientConfig 可下发面板与生日派对主表：装载请求放
                    // 最前——两份都是根级小 JSON，链上其他装载依赖的
                    // 大表没到齐之前它们就该落定。
                    client_config::load,
                    birthday::load,
                    uber_particle::load,
                    // 家具挂点档案（attach-points）的装载请求：动作点
                    // 世界位的数据面，与摆放表（fixture 域）在同一批
                    // Startup 请求里发出。
                    fixture_attach::load,
                    balloon::load,
                    // 待机动作两张表（编排 + facial）与表情件档案的装载请求。
                    alone_action_runtime::load,
                    emoticon::load,
                    // 对话剧本表（fixture-talks）的装载请求。
                    talk::load,
                    // 玩家对话剧本表（talks）与站点主表（站点门对照）的
                    // 装载请求。
                    player_talk::load,
                    content_library::load,
                    // 对话窗体的纹源（面板页 + 尾标替身）与常驻状态机。
                    talk_window::load,
                    // 摇杆两件（底盘 + 手柄）的纹源（UI atlas 整页，两件
                    // 共用一次装载）。
                    joystick::load,
                    // 接近触发的动作按钮：图标纹源（每种按钮类型一张独立
                    // 纹理，不在图集里）与两张家具面表的装载请求。
                    action_button::load,
                    // 音频路由档案（index 共用天气那份 + 循环点表）的装载请求。
                    audio::load,
                    balloon::overlay_camera,
                    // 小地图链的装载请求与覆盖相机（order 2 层，与气泡层平行）。
                    sitemap::load.run_if(crate::browser_stage::standalone),
                    sitemap::overlay_camera.run_if(crate::browser_stage::standalone),
                )
                    .chain(),
            )
                .chain(),
        )
        .add_systems(Update, character_material::apply_fog)
        // The player uses the shared SD body/rig/material installation. No
        // audience model, audience recolouring or spectator wear is registered.
        .add_systems(
            Update,
            (
                (
                    // ClientConfig 面板与生日派对主表的解析：每帧先于全部
                    // 消费者跑（面板的消费者横跨相机/对话/采集/小地图四
                    // 域，单独成链放最前）。装载失败在这里响亮 panic。
                    client_config::parse,
                    birthday::parse,
                ),
                (
                    // 站点域：主表解析 → 换站入口（拆站重选）→ 装载计划
                    // （首站与换站共一条路）→ 展开 → 侧车解析（inactiveNodes
                    // 名单与 A 套声源表）→ 声源挂上（清扫后的树）→ 每站
                    // 一条装载锚行（代数门：清扫落账的下一帧计数）。
                    // 约束面构建收尾站点域：玩家铺位与推进读它（面与地表
                    // 同批网格，地表顶点可查的当帧它也齐）。
                    site::parse_masters,
                    site::read_switch.after(site::parse_masters),
                    site::plan.after(site::read_switch),
                    site::spawn_when_ready.after(site::plan),
                    inactive_nodes::parse,
                    site_sound::parse,
                    site_sound::apply
                        .after(site_sound::parse)
                        .after(inactive_nodes::parse),
                    site::report_anchor.after(site::spawn_when_ready),
                    walk_face::build.after(site::spawn_when_ready),
                    // 目标面构建（保留高度的三角网 + 格桶 + 可行走格）：
                    // 与约束面同批网格、同批就绪；名册落位与目标机都读它。
                    npc_objective::build_face.after(walk_face::build),
                    // 摆放保存后整场重烘挖洞：保存回执落台账 → 重烘 → 对账行。
                    walk_face::rebake_on_save.after(walk_face::build),
                ),
                (
                    // 手势与摇杆链（0 层输入，先于相机与对话的消费）：
                    // 两个冒烟口各自注入（手势往真实输入资源按剧本写，
                    // 摇杆往触摸消息流按剧本写——消息面即真实输入面，
                    // 都不绕过层的任何一行）→ 摇杆层推进（触摸流先过
                    // 摇杆：区内起手归摇杆、让位门编辑/对话在播时禁用
                    // 并以 END 收场）→ 手势层推进（输入分配表落码：拖拽
                    // 阈值 min(宽,高)/100 冻结、长按 0.25s、连击窗 0.3s；
                    // 鼠标与触摸手指并进同一台单指针状态机——摇杆捕获
                    // 指与区内起手已被摇杆层拿走，两层读同一谓词分流）
                    // → 摇杆件铺装与摆位（页到齐一次铺两件，摆位读摇杆
                    // 层本帧的捕获态）→ 拾取冒烟合成口（事件面注入，
                    // 走同一条拾取链）→ 拾取（TAP 收场沿 + 右键：世界
                    // 射线 → 采集物/NPC/家具三族候选按射线路程取最近；
                    // 采集物入被击队，NPC 沿发玩家对话请求，家具预留沿
                    // 只记日志）。拾取显式排在采集物域的被击处理之前：
                    // 当帧入队当帧消费。
                    joystick::smoke_autojoystick,
                    // 动作按钮的走位冒烟口也注在摇杆层推进之前：它写入的
                    // 触摸要被同一帧的摇杆层读到。
                    action_button::smoke_autowalk,
                    joystick::advance,
                    gesture::advance,
                    joystick::spawn_when_ready,
                    joystick::place_ui,
                    // 接近触发的动作按钮：表解析 → 铺件 → 每帧求交写栈 →
                    // 摆件 → 点按分派。点按显式排在世界射线拾取之前：
                    // 真源里屏幕按钮在 UI 事件系统里，本来就先拦下点按，
                    // 落在按钮上的那一下不该再打一条世界射线。
                    action_button::parse_tables,
                    action_button::spawn_when_ready,
                    action_button::advance,
                    action_button::place_ui,
                    action_button::click,
                    pick::smoke_autotap,
                    // NPC 臂冒烟口（MOLY_PICK_NPC_TAP_SECS）：名册成员的
                    // 世界位投影点按，走同一条拾取链过配对门。
                    pick::smoke_npc_autotap,
                    // 玩家对话冒烟口同拍注入（候选成员的世界位投影点按，
                    // 请求当帧被对话链消费）。
                    player_talk::smoke_autotap,
                    pick::pick.before(harvest::on_damage),
                )
                    .chain(),
                (
                    (
                        // npc 链写者先于读者：解析清单 → 铺名册 → 换站重播种 →
                        // 逐帧推进（写移动相位）→ 周期状态行。排在站点域之后：
                        // 名册的脚下高度取自站点地表网格。
                        // 挂点档案解析（含与摆放表组合出的动作点世界位）放
                        // 链首：装载失败在此响亮 panic；组合表晚解析一帧落地
                        // （命令同步点），目标机的资源门挡那一帧的空窗。
                        fixture_attach::parse,
                        npc::parse,
                        npc::spawn_when_ready
                            .after(npc::parse)
                            .after(fixture_attach::parse),
                        npc::reseed.after(npc::spawn_when_ready),
                        // 目标机决策（停顿计时、决策梯、抽签、目的地解算、出发——
                        // 写路径槽与相位）。决策先于推进：当帧决策当帧起步。
                        npc_objective::decide
                            .after(npc::reseed)
                            .before(npc::advance),
                        npc::advance,
                        npc::report
                            .after(npc::advance)
                            .run_if(common_conditions::on_timer(Duration::from_secs(2))),
                    ),
                    (
                        // 相机 JSON 解析：装载面（七曲线 + 静态机位）。资产
                        // 独立于站点链，放在链前端尽早落定；frame_site 消费
                        // 其 CameraSetting（解析未到时取景暂缓）。
                        camera::parse,
                        // 相机手势输入：吃手势层的 DRAG 逐帧 Moved（激活阈值
                        // 在手势层）+ 滚轮捏合，跟随律在 PostUpdate 当帧消费
                        // ——输入当帧生效。
                        camera::apply_input
                            .after(camera::parse)
                            .run_if(crate::game_settings::camera_input_enabled),
                    ),
                    (
                        // 角色链：计划（发包装载）→ 挂载 → 装配（插播放器与驱动）→
                        // toon 计划（解析骨架档案建材质）→ toon 换装（等贴图到齐
                        // 摘 Standard 换 Character）。链内命令自动同步点逐级生效：
                        // 前一级插的组件，后一级当帧读得到。
                        player::parse,
                        player::spawn_when_ready.after(player::parse),
                        character::plan_when_ready
                            .after(player::spawn_when_ready)
                            .after(npc::spawn_when_ready),
                        character::attach_when_ready.after(character::plan_when_ready),
                        character::wire_when_ready.after(character::attach_when_ready),
                        character_material::plan_when_wired.after(character::wire_when_ready),
                        character_material::swap_when_planned
                            .after(character_material::plan_when_wired),
                        // 骨布装配：解析 rig 的 cloth 节、绑骨、建链（等装配闩
                        // MotionDriver——场景已展开、动画目标已插）。
                        cloth_runtime::plan_when_wired.after(character::wire_when_ready),
                        // 玩家逻辑仍在自己的根：换站重定位 → 输入 → 位移，
                        // 只将SD外观与角色共用，不加入NPC决策/路径名册。
                        player::reseed.after(player::spawn_when_ready),
                        // 读输入 → 推进位移这一对必须显式链上：两者对 PlayerInput
                        // 一写一读构成冲突，但冲突只保证串行、不保证次序——
                        // executor 让推进先跑时读到的是上一帧的输入，起手/收场
                        // 各晚一帧（真源 OnTouchJoyStick 在事件回调里烘好向量，
                        // 同一帧的 UpdateState 就消费它；键盘路同帧同形）。
                        (player::read_input.after(player::reseed), player::advance).chain(),
                        // SD body/model wiring is shared with character above;
                        // its player branch installs the sole AvatarDriver.
                    ),
                    (
                        // 待机动作链：两张表解析（装载失败在此响亮失败）→ 逐名挂
                        // 运行时（挂上即核数据面：facial 键与动作段全在表/库里）
                        // → 逐帧主循环（驻留帧选取、序列步进分发事件、播完回收
                        // 重选、播放器独占权维护）。排在换装之后（运行时要 toon
                        // 材质句柄）、位移驱动之前（先占后让：演出占住播放器时
                        // 位移驱动整名让位）。
                        alone_action_runtime::parse_alone_actions,
                        alone_action_runtime::parse_facial_tables,
                        alone_action_runtime::attach
                            .after(alone_action_runtime::parse_alone_actions)
                            .after(alone_action_runtime::parse_facial_tables)
                            .after(character_material::swap_when_planned),
                        alone_action_runtime::report.after(alone_action_runtime::attach),
                    ),
                    (
                        // 驱动（读移动相位换段）与播放探针；玩家速率律排在换段
                        // 之后：换段以缺省速率起新段，玩家域同帧把速率压回
                        // 真源式。其后气泡链：解析主表
                        // → 烘图集 → 驻留沿触发（读 npc 推进写好的相位）→ 存活
                        // 计时与淡坡。读相位所以排推进之后。
                        player_avatar::drive
                            .after(player::advance)
                            .after(character::wire_when_ready),
                        player::tune_animation_speed.after(player_avatar::drive),
                        character::probe_playback.after(character::drive),
                        player_avatar::probe_playback.after(player::tune_animation_speed),
                        player::report
                            .after(player::advance)
                            .run_if(common_conditions::on_timer(Duration::from_secs(2))),
                        balloon::parse_master,
                        balloon::bake_atlas
                            .after(balloon::parse_master)
                            .after(player_talk::parse),
                        // 问候触发 → 显式同步点 → after-edit 反应：同一根
                        // objective 槽位的两条写入沿。链式定序 + 同步点让
                        // 反应的让位门看得见问候触发本帧铺的气泡——不定序
                        // 时两系统并发（命令式写入不构成访问冲突），让位门
                        // 对同帧的问候气泡是盲的，成员头上会叠两只气泡
                        // （真源单状态字段，构造上不允许两只并存）。
                        (
                            balloon::trigger
                                .after(balloon::bake_atlas)
                                .after(npc::advance),
                            ApplyDeferred,
                            // after-edit 反应链：保存回执 → 池选取
                            // → 上屏 → 5.0s 驻留收场（驻留在上屏之后，真源
                            // objective 的序）。同一份表与图集资源，呈现复用。
                            balloon::after_edit_reaction,
                        )
                            .chain(),
                        balloon::tick.after(balloon::after_edit_reaction),
                        // 层序探针（MOLY_BALLOON_ORDER_SECS > 0 才活）：造一对
                        // 同屏点重叠的探针气泡，到秒数自动退出（无头验收）。
                        balloon::order_smoke.after(balloon::tick),
                    ),
                    (
                        // The Rest driver owns script selection; this chain only prepares visuals.
                        emoticon::parse,
                        emoticon::spawn_when_ready.after(emoticon::parse),
                    ),
                    (
                        // 对话链：剧本表解析 → 时间轴三件套装载计划（锚定
                        // 摆放 ∩ 语料点名）与解析（可播集落位）→ 名册/库/
                        // 表/档案全就绪后预筛候选（候选要点播的键装载期核
                        // 验）→ 配对检出与选取（读 npc 推进写好的位置，排
                        // 其后；开场即开下方对话窗，与玩家链共用窗体）→
                        // 点跳输入（两链各自的闩，无输入驻留合成点击）→
                        // 家具脸部件解析与动画执行面解析（换装闩后一次性，
                        // 步进要读它们）→ 步进主循环（触发步分发：文本进
                        // 对话窗体、眼/口写材质（角色与家具同表）、动作起
                        // 播、表情件折请求、家具转体起转、时间轴起播；合
                        // 成点击闩与 IsWaitClick 门在步进前定）→ 时间轴序
                        // 列逐帧（段终态接下一段/进循环）→ 家具转体逐帧
                        // （当帧可完成瞬时档）→ 转体逐帧推进（参演者持留
                        // 期间位移侧让位）→ 表情件请求消费（同一条出件/
                        // 收件通道）→ 窗体帧推进（打字机/淡变/字形揭示/α
                        // 写回）→ 周期状态行。
                        //
                        // 玩家对话链同拍同形：剧本表解析（含指称名表与文本字
                        // 符集——字符集并进气泡图集的烘焙门）→ 名册就绪后筛
                        // 候选 → 消费拾取链的玩家对话请求（六门求值 → 池 →
                        // 均匀抽/重播回退/Current覆写 → 会话开场）。只有一个
                        // 业务request reader；随后同步命令，让Rest出沿先于
                        // 两个既有脚本后端的正文/语音消费者。
                        (
                            talk::parse,
                            fixture_talk::plan_timelines,
                            fixture_talk::resolve_timelines,
                            talk::prepare,
                            player_talk::parse,
                            player_talk::prepare,
                            player_talk::consume_trigger,
                            ApplyDeferred,
                            // Dispose a departing Rest before a new conversation writes animation.
                            npc::sync_rest_lifecycle.after(npc::advance),
                            alone_action_runtime::advance,
                            (talk::fixture_action::drive, character::drive).chain(),
                            emoticon::serve_rest,
                            // 合成对话注入口（冒烟；无环境变量自关）——与配对同链
                            // 在步进前：注入的会话当帧即可步进。
                            // Keep the outer dialogue tuple below Bevy's tuple-system
                            // arity while preserving the probe -> prefetch order.
                            (
                                talk::voice_probe,
                                talk::partvoice_probe,
                                // Warm exact future authored voice assets without
                                // creating a player or advancing the script cursor.
                                audio::prefetch_voice,
                            )
                                .chain(),
                            talk_window::read_click_input
                                .run_if(crate::game_settings::talk_input_enabled),
                            fixture_talk::discover_faces,
                            fixture_talk::discover_animation,
                            talk_window::smoke_tap,
                            talk::advance_talk,
                            player_talk::advance_session,
                        )
                            .chain(),
                        (
                            // Both conversation drivers publish voice lines before this flush.
                            // Audio observes the same frame as the associated text steps.
                            ApplyDeferred,
                            // talk voice 起播（音频域消费行请求）：真源行内命令序
                            // voice 先于 text——行推进同帧起播。
                            audio::serve_voice,
                            fixture_talk::progress_timelines,
                            fixture_talk::progress_turns,
                            talk::progress_turns,
                            player_talk::progress_turns,
                            emoticon::serve_talk,
                            talk_window::tick_window,
                            talk::report
                                .run_if(common_conditions::on_timer(Duration::from_secs(5))),
                            player_talk::report
                                .run_if(common_conditions::on_timer(Duration::from_secs(5))),
                        )
                            .chain(),
                    )
                        .chain(),
                    (
                        // 小地图链：四份数据解析（fail-closed）→ 贴图到齐铺装
                        // （底图 + 图标列 + 云簇 + 天气钮 + 字形图集）→ 两根
                        // 缩放逐帧对窗 → 验证用的自动开云钩子 → 点击开云 →
                        // 入场/浮动/开云三条推进 → M 键显隐 → 天气钮跟随
                        // 现象 → 周期状态行。链内命令同步点逐级生效。
                        sitemap::parse,
                        sitemap::spawn_when_ready,
                        sitemap::fit_root,
                        sitemap::auto_unlock,
                        sitemap::smoke_autoclick,
                        sitemap::click.run_if(crate::game_settings::scene_input_enabled),
                        sitemap::tick_entries,
                        sitemap::tick_floats,
                        sitemap::tick_unlock,
                        sitemap::toggle.run_if(crate::game_settings::scene_input_enabled),
                        sitemap::refresh_weather,
                        sitemap::report.run_if(common_conditions::on_timer(Duration::from_secs(2))),
                    ),
                ),
            )
                .chain(),
        )
        // 玩家 avatar 状态机三写者（真源 UpdateController 每帧分派的等价
        // + 两个域写者）：采集段 → 对话会话 → 输入。单列而不进上面的大
        // tuple——那个 tuple 的名册/玩家组已经顶到 Bevy 调度元组的 20 元
        // 上限，再塞会整块失去 IntoScheduleConfigs（错误只在 .chain() 处
        // 冒「not an iterator」）。采集段排在整个采集链之后（段信号是被击
        // 队列与在飞击打演出的事后读数）；输入写者排在 read_input 之后——
        // 与那对同款，PlayerInput 一写一读的冲突只保证串行不保证次序。
        // 采集段放链首：同帧收场时先开门，输入写者当帧就能接上。
        .add_systems(
            Update,
            (
                player_state::drive_from_harvest,
                player_state::drive_from_talk,
                player_state::drive_from_input,
            )
                .chain()
                .after(harvest::advance_punch)
                .after(player::read_input),
        )
        .add_systems(
            Update,
            (
                crate::player_fixture_action::request_end_from_input,
                crate::player_fixture_action::refresh_availability,
                crate::player_fixture_action::receive_requests,
                crate::player_fixture_action::advance,
                crate::fixture_gimmick::advance,
                crate::fixture_activity_timeline::advance,
            )
                .chain()
                .after(action_button::click)
                .after(player_state::drive_from_input)
                .before(player::advance)
                .before(player_avatar::drive)
                .before(audio::advance_se),
        )
        // 清扫排引擎场景展开之后：SceneInstanceReady 在 Update 之后才触发，
        // SiteReady 同帧就会被取景行吃掉，Update 侧的清扫等不到它（时机
        // 全解在 inactive_nodes 模块注释）。
        .add_systems(
            SpawnScene,
            inactive_nodes::apply.after(SceneSpawnerSystems::Spawn),
        )
        // 天空链：认壳包 → 认材质 → 网格与渐变条齐了铺实体；渐变条独立成行。
        .add_systems(
            Update,
            (
                sky::parse_packages,
                sky::parse_sidecar,
                sky::spawn_when_ready,
            )
                .chain(),
        )
        .add_systems(Update, sky::parse_phenomena)
        // 站点侧 UberUnlit 粒子链：判读（sidecar 的 particles[] + 场景树的
        // 节点路径 join，逐档具名拒绝）→ 贴图到齐铺实体 → 周期状态行。
        // 帧推进不在这里，在 PostUpdate（局部空间仿真要读发射节点的当帧
        // 世界变换、四角展开要读当帧机位）。
        .add_systems(
            Update,
            (
                uber_particle::plan,
                uber_particle::spawn_when_ready,
                uber_particle::report.run_if(common_conditions::on_timer(Duration::from_secs(2))),
            )
                .chain(),
        )
        // 天气粒子链：锚点监视（档位 × 站点）→ 现象清单解析 → 判读 →
        // 贴图到齐铺实体 → 周期状态行。链内命令同步点逐级生效。帧推进
        // 不在这里，在 PostUpdate（局部空间仿真要读锚点的当帧世界变换、
        // 四角展开要读当帧机位）。
        .add_systems(
            Update,
            (
                weather_fx::watch,
                weather_fx::parse,
                weather_fx::plan,
                weather_fx::spawn_when_ready,
                weather_fx::report.run_if(common_conditions::on_timer(Duration::from_secs(2))),
            )
                .chain(),
        )
        // 音频链：路由表解析（两份档案到齐一次性吃掉）→ BGM（淡出/淡入/
        // intro→loop 交接/换曲）→ 环境音（切档停放）→ A 套就近管理（站点
        // 场景喂声源）→ 一次性 SE（排空事件队列）→ 周期账目。读天气
        // 写的当前档名，排在天气链之后是做不到的（天气系统在它自己的插件
        // 里），同帧读晚一档的值，换曲与画面淡化并行推进。SE 的写者
        // （采集被击、对话步进/窗体、摆放编辑输入）用 .after 显式排序——
        // 队列当帧入当帧排空，起播时刻就是事件帧。
        .add_systems(
            Update,
            (
                audio::parse,
                audio::advance_bgm,
                audio::advance_ambient,
                audio::advance_proximity,
                audio::advance_se
                    .in_set(audio::SeDrainSet::Drain)
                    .after(harvest::on_damage)
                    .after(talk::advance_talk)
                    .after(talk_window::tick_window),
                audio::report.run_if(common_conditions::on_timer(Duration::from_secs(2))),
            )
                .chain(),
        )
        // 骨布推进与写回：动画系统写好骨的当帧局部变换之后、变换传播
        // 之前——写回的布料姿势当帧传播到渲染与蒙皮。世界变换由布层
        // 自己沿父链合成（全局变换此刻还是上一帧的）。
        .add_systems(
            PostUpdate,
            (
                cloth_runtime::advance
                    .after(AnimationSystems)
                    .before(TransformSystems::Propagate),
                cloth_runtime::report
                    .after(cloth_runtime::advance)
                    .run_if(common_conditions::on_timer(cloth_runtime::REPORT_PERIOD)),
            ),
        )
        // TransformPropagate 之后：scene 实体当帧展开，取景要读已传播的全局变换。
        // 追角色再排在取景之后：角色就位当帧起，追角色每帧覆盖机位。
        // 天空钉位收尾：读当帧机位的水平坐标，别把上一帧的机位画进天空。
        .add_systems(
            PostUpdate,
            (
                camera::frame_site.after(TransformSystems::Propagate),
                camera::follow_avatar
                    .after(TransformSystems::Propagate)
                    .after(camera::frame_site),
                // 补间推进写相机投影的 FOV 分量与两个材质帧刷新的
                // near/far（只读常量）无行为交集，不排序；对话相机链
                // 对 follow_avatar 与 write_frame_state 各有先序约束，
                // 这里再挂 after 会与它成环。
                camera::report_follow
                    .after(camera::follow_avatar)
                    .run_if(common_conditions::on_timer(Duration::from_secs(2))),
                sky::follow_camera.after(camera::follow_avatar),
                // 粒子帧推进与属性池重建：变换传播之后（发射节点的世界
                // 变换是本帧的）、机位定好之后（四角朝的是本帧的相机）。
                uber_particle::advance
                    .after(TransformSystems::Propagate)
                    .after(camera::follow_avatar),
                // 天气粒子帧推进：与站点粒子链同一拍——天空/相机锚与
                // 机位都要当帧值。
                weather_fx::advance
                    .after(TransformSystems::Propagate)
                    .after(camera::follow_avatar),
                // 角色侧相机态（屏幕/投影参数）与站点取景同拍；头参考点在
                // TransformPropagate 之后回写——动画在 Propagate 之前推进，
                // 读到的是当帧姿势。
                character_material::write_frame_state
                    .after(TransformSystems::Propagate)
                    .after(camera::frame_site),
                character_material::update_head.after(TransformSystems::Propagate),
                character_material::report_head
                    .after(character_material::update_head)
                    .run_if(common_conditions::on_timer(Duration::from_secs(3))),
                // 气泡定位：主相机机位定好、变换传播完，投影才是本帧的。
                balloon::place
                    .after(TransformSystems::Propagate)
                    .after(camera::follow_avatar),
                // 表情链的帧推进与周期账目：在骨骼传播与相机之后——挂点、
                // 发射形状、billboard 基读到的都是本帧值。
                emoticon::advance
                    .after(TransformSystems::Propagate)
                    .after(camera::follow_avatar),
                emoticon::report
                    .after(emoticon::advance)
                    .run_if(common_conditions::on_timer(Duration::from_secs(2))),
            ),
        )
        // 对话相机（配对剧情与玩家对话共用）：常态机位快照抓在取景与
        // 跟随之间（那帧模型还是构造值）；入场/驻留/还原排在跟随之后、
        // 一切读机位的系统之前——本帧对话相机的机位就是渲染用的机位。
        .add_systems(
            PostUpdate,
            (
                talk_camera::capture_normal
                    .after(camera::frame_site)
                    .before(camera::follow_avatar),
                talk_camera::advance
                    .after(camera::follow_avatar)
                    .before(sky::follow_camera)
                    .before(character_material::write_frame_state)
                    .before(balloon::place)
                    .before(emoticon::advance),
            ),
        );
    // ---- 场地屏外壳 · 屏幕层栈（追加段：以下全部为新增，未动上面既有行） ----
    // 层栈命令与外壳对话框请求两条消息沿；层栈资源常驻（栈底场地屏起步，
    // 真源主场地屏常驻同形）。
    app.add_message::<ui_layers::LayerCommand>()
        .add_message::<menu_shell::ShellDialogRequest>()
        .init_resource::<ui_layers::UiLayerStack>()
        // 外壳的站点主表装载请求（站名列的读者；与小地图同一份表）。
        .add_systems(Startup, menu_shell::load)
        // 站名表解析与铺件：只读资产面 + 自有组件，无次序面。解析落
        // ShellTextCharset（气泡层图集的第四个字符集成员——烘制门四员
        // 到齐才开烘，外壳的字不会漏烘）。
        .add_systems(Update, (menu_shell::parse, menu_shell::spawn_when_ready))
        // 外壳点按链：冒烟注入 → 点按分派。排在动作按钮之后（同一份
        // 点按消费标志，后者覆盖）、世界射线拾取之前（按钮先手，落在外
        // 壳上的那一下不该再打世界射线——与动作按钮同一条次序律）。
        .add_systems(
            Update,
            menu_shell::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(action_button::click)
                .before(pick::pick),
        )
        // 层栈推进：读小地图根可见性做直通口对账 + 消费层命令 + 回写槽
        // 位视图。小地图的四条可见性写者（点击 · 自动点 · 解锁推进 ·
        // M 键）与外壳点按全排在它之前——它读的是当帧终值、当帧命令。
        .add_systems(
            Update,
            ui_layers::advance
                .after(menu_shell::click)
                .after(sitemap::click)
                .after(sitemap::smoke_autoclick)
                .after(sitemap::tick_unlock)
                .after(sitemap::toggle),
        )
        // 对话框请求消费（动作按钮的离开支走这条沿）与外壳摆位：层栈
        // 之后（摆位读层栈当前层定外壳收放）、外壳点按之后（点按写收放
        // 态/对话框态，摆位读终值）。
        .add_systems(
            Update,
            (menu_shell::advance_dialogs, menu_shell::place)
                .chain()
                .after(ui_layers::advance)
                .after(menu_shell::click),
        )
        // 相机复位补间推进：相机输入之后（补间期间输入被吞——真源
        // _isResetAnimation 门，吞门在 apply_input 里读本资源在场与否）。
        .add_systems(
            Update,
            menu_shell::advance_camera_reset.after(camera::apply_input),
        );
    // ---- 情报层视图（追加段：层栈 + 外壳之后的第一个带内容物的屏幕层） ----
    // 资源与启动施加（进场即按存量档全量施加；刷新率档当值取构造默认
    // high ⇒ 60fps 钳制）。
    app.add_systems(Startup, info::init)
        // 铺件：图集到齐一次铺成（外壳字符集已并情报层固定文案，烘制
        // 门的四员到齐条件不变——情报层的字随外壳成员一起进图集）。
        .add_systems(Update, info::spawn_when_ready)
        // 点按链：冒烟注入 → 点按分派。锚同外壳链（动作按钮之后、世界
        // 射线与层栈推进之前）：情报层开着时点按全被本层吃掉，层命令
        // 当帧进栈。
        .add_systems(
            Update,
            info::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(menu_shell::click)
                .before(pick::pick)
                .before(ui_layers::advance),
        )
        // 摆位与生命周期：层栈推进之后（读当帧终值：层开关沿、页态、
        // 对话框态）、外壳摆位之后（同为写可见性/精灵/变换的摆位系统，
        // 显式排序消歧义）、本层点按之后（点按写页态/对话框态/档位，
        // 摆位读终值）。
        .add_systems(
            Update,
            info::place
                .after(ui_layers::advance)
                .after(menu_shell::place)
                .after(info::click),
        );
    // ---- 菜单对话框（追加段：外壳菜单钮的目标，Dialog 槽） ----
    // mock 面板资源（服务端态具名：体力/等级两格 + 四个使能输入，环境
    // 变量覆写）。
    app.add_systems(Startup, menu_dialog::init)
        // 铺件：图集到齐一次铺成（外壳字符集已并菜单对话框固定文案，
        // 烘制门四员到齐条件不变——菜单对话框的字随外壳成员一起进图集）。
        .add_systems(Update, menu_dialog::spawn_when_ready)
        // 点按链：冒烟注入 → 点按分派。排在外壳点按**之前**（同一条
        // 模态律的先手：对话框开着时点按全归本系统，外壳件不吃——外壳
        // 的门读同一个 menu_open 位）、动作按钮之后（同一份点按消费标志，
        // 重置者在前）、世界射线与层栈推进之前（框内层命令当帧进栈）。
        .add_systems(
            Update,
            menu_dialog::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(action_button::click)
                .before(menu_shell::click)
                .before(pick::pick)
                .before(ui_layers::advance),
        )
        // 摆位与开关沿：本层点按之后（点按键关框，摆位读终值）、外壳
        // 摆位之后（同为写可见性/精灵/变换的摆位系统，显式排序消歧义）。
        .add_systems(
            Update,
            menu_dialog::place
                .after(menu_dialog::click)
                .after(menu_shell::place),
        );
    // ---- 获得子窗（追加段：Dialog 槽的第三件，四开门者共用的获得窗） ----
    // mock 面板资源（服务端态具名：开门者 + 资源队列，环境变量覆写）与
    // 链播放态（关一格出队开下一格）。
    app.add_systems(Startup, get_resource::init)
        // 铺件：图集到齐一次铺成（外壳字符集已并获得子窗固定文案，烘制
        // 门四员到齐条件不变——获得子窗的字随外壳成员一起进图集）。
        .add_systems(Update, get_resource::spawn_when_ready)
        // 点按链：冒烟注入 → 点按分派。锚同菜单对话框链（同一条 Dialog
        // 槽模态律的先手：子窗开着时点按全归本系统，外壳与菜单对话框都
        // 不吃——外壳的门读同一个 get_resource_open 位）、动作按钮之后
        // （同一份点按消费标志，重置者在前）、世界射线与层栈推进之前。
        .add_systems(
            Update,
            get_resource::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(action_button::click)
                .before(menu_shell::click)
                .before(menu_dialog::click)
                .before(pick::pick)
                .before(ui_layers::advance),
        )
        // 摆位与开关沿：本窗点按之后（点按键关窗/换格，摆位读终值）、
        // 外壳摆位之后（同为写可见性/精灵/变换的摆位系统，显式排序消
        // 歧义）。
        .add_systems(
            Update,
            get_resource::place
                .after(get_resource::click)
                .after(menu_shell::place),
        );
    // ---- 选项对话框（追加段：设置钮的目标，Dialog 槽——音量页） ----
    // 本地档装载与进场施加（SceneMysekai.Start → SetupVolume 同律：读档
    // → 施加系统组 → 环境变量只作进场覆写）。排 Startup 链后（无跨系统
    // 依赖：总线资源在插件安装时已建，装载行先于任何 Update 读者）。
    app.add_systems(Startup, audio::init_settings)
        // 运行态资源（草稿/在途延时预览/拖动）。
        .add_systems(Startup, option_dialog::init)
        // 铺件：图集到齐一次铺成（外壳字符集已并选项对话框固定文案，
        // 烘制门四员到齐条件不变——选项对话框的字随外壳成员一起进图集）。
        .add_systems(Update, option_dialog::spawn_when_ready)
        // 点按链：冒烟注入 → 点按分派。锚同菜单对话框（动作按钮之后、
        // 外壳与菜单对话框与世界射线与层栈推进之前）：选项框开着时点按
        // 全归本系统（外壳与菜单对话框的门读同一个 option_open 位）。
        .add_systems(
            Update,
            option_dialog::click
                .run_if(crate::game_settings::scene_input_enabled)
                .after(action_button::click)
                .before(menu_dialog::click)
                .before(menu_shell::click)
                .before(pick::pick)
                .before(ui_layers::advance),
        )
        // 摆位与开关沿：本层点按之后（点按键关框，摆位读终值）、外壳与
        // 菜单对话框摆位之后（同为写可见性/精灵/变换的摆位系统，显式
        // 排序消歧义）、SE 排空之前（延时预览入队当帧被音频链排空——
        // 写者先于读者的既有排序律）。voice 通道不再加边：serve_voice
        // 经层栈链（sitemap 链 → ui_layers::advance → 外壳摆位）传递序
        // 已在本系统之前，ResMut 冲突由那条序化解。
        .add_systems(
            Update,
            option_dialog::place
                .after(option_dialog::click)
                .after(menu_shell::place)
                .after(menu_dialog::place)
                .before(audio::SeDrainSet::Drain),
        );
}
