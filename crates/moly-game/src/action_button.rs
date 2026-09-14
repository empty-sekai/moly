//! Proximity action buttons use the existing oriented collision boxes and target
//! stack. NPC appearance requires active site/identity/visibility, not proximity
//! to an unrelated fixture or a current talk-state whitelist.
//!
//! A Talk click publishes the existing request once. Safe player positioning,
//! EnableTalk/action/ownership checks and the talk-specific input interval belong
//! to the sole dispatcher. Other action buttons retain their existing throttle.
//! The NPC ChangeSite14+tweet exception is not the ordinary Tweet9 state.
//!
//! Fixture button classification and source icons retain their existing owners.
//! The common talk/fixture button skin and geometry come from the field prefab.
//! Tutorial/registered NPC timeline
//! producers still need their own source-backed inputs; no nearby-furniture
//! pairing flag is a substitute for them.

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;

use moly_law::action_button::{
    character_box, fixture_box, ButtonStack, ButtonType, CollisionBox2D,
    FixtureType, TargetId, ACTION_BUTTON_INPUT_INTERVAL, PLAYER_ADDITIONAL_HALF_EXTEND,
};

use crate::balloon::{canvas_scale, BALLOON_LAYER};
use crate::fixture::{FixturePlacement, FixtureRoot, FixtureSource};
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureTarget};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::joystick::{JoystickState, HANDLE_SIZE};
use crate::npc::{CharacterUnitId, WalkState};
use crate::player::PlayerControlled;
use crate::player_talk::PlayerTalkRequest;
use crate::ui_layers::{LayerCommand, LayerId};
use crate::ui_layout::{UiLayouts, UiPrefabView};

// The Home and MyRoom talk/fixture buttons share this authored skin. Keep the
// complete ancestor geometry: the controller is anchored to the bottom-right,
// while its button Image and the smaller RawImage icon have separate rects.
const SKIN_LAYOUT: &str = "ShellHome";
const BUTTON_NODE: &str = "ActionButtonController/TalkActionButton";
const ICON_NODE: &str = "ActionButtonController/TalkActionButton/Icon";

/// 按钮根（图标件的父，可见性随栈首）。
#[derive(Component)]
pub(crate) struct ActionButtonRoot;

/// 图标件。
#[derive(Component)]
pub(crate) struct ActionButtonIcon;

#[derive(Component)]
pub(crate) struct ActionButtonBackground;

/// 图标纹源：按文件名存一份句柄。文件名由按钮类型给出（真源的那张
/// 二十五格表），所以这里不是按「用途」而是按真源文件名索引。
#[derive(Resource, Default)]
pub(crate) struct ActionButtonArt {
    icons: HashMap<&'static str, Handle<Image>>,
    skin: Option<ActionButtonSkin>,
}

struct ActionButtonSkin {
    background: Handle<Image>,
    background_color: Color,
    icon_color: Color,
    geometry: UiPrefabView,
}

impl ActionButtonSkin {
    fn from_layouts(layouts: &UiLayouts, server: &AssetServer) -> Option<Self> {
        let doc = layouts.document(SKIN_LAYOUT)?;
        let background_node = &doc.nodes[doc.find(BUTTON_NODE).expect("source action button")];
        let icon_node = &doc.nodes[doc.find(ICON_NODE).expect("source action icon")];
        let background = background_node.components.iter().find(|component| {
            component.enabled && component.class.ends_with("Image") && component.sprite.is_some()
        }).expect("source action button Image");
        let icon = icon_node.components.iter().find(|component| {
            component.enabled && component.class.ends_with("RawImage")
        }).expect("source action icon RawImage");
        let image = background.sprite.as_ref().and_then(|sprite| sprite["image"].as_str())
            .expect("source action button Sprite image");
        let color = |component: &moly_assets::ui_layout::UiComponent| {
            let values = component.fields["m_Color"].as_array().expect("source UI color");
            let channel = |i: usize| values[i].as_f64().expect("source UI color channel") as f32;
            Color::srgba(channel(0), channel(1), channel(2), channel(3))
        };
        // The root Image is the always-present button background. The separate
        // Cover is a press/disabled overlay: MysekaiActionInternalButton.OnEnable
        // calls HideCover, so it must not be mistaken for the normal gray base.
        let mut geometry = UiPrefabView::new(SKIN_LAYOUT, BALLOON_LAYER);
        geometry.set_visible(BUTTON_NODE, true);
        Some(Self {
            background: server.load(format!("moly://ui-layout-v2/{image}")),
            background_color: color(background),
            icon_color: color(icon),
            geometry,
        })
    }
}

/// Keep the existing input systems within the system-parameter limit while
/// sharing exactly the same prefab geometry with rendering and smoke input.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct ActionButtonScreen<'w, 's> {
    windows: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
    art: Option<Res<'w, ActionButtonArt>>,
    layouts: Option<Res<'w, UiLayouts>>,
}

impl ActionButtonScreen<'_, '_> {
    fn rects(&self, size: Vec2) -> Option<(moly_assets::ui_layout::UiRect, moly_assets::ui_layout::UiRect)> {
        let skin = self.art.as_deref()?.skin.as_ref()?;
        let layouts = self.layouts.as_deref()?;
        let canvas = size / canvas_scale(size.x, size.y);
        Some((skin.geometry.rect(layouts, BUTTON_NODE, canvas)?, skin.geometry.rect(layouts, ICON_NODE, canvas)?))
    }

    fn button_position(&self, size: Vec2) -> Option<Vec2> {
        let (button, _) = self.rects(size)?;
        let center = button.center() * canvas_scale(size.x, size.y);
        Some(Vec2::new(size.x * 0.5 + center.x, size.y * 0.5 - center.y))
    }

    fn hit(&self, position: Vec2, size: Vec2) -> bool {
        let Some((button, _)) = self.rects(size) else { return false; };
        let canvas_point = Vec2::new(position.x - size.x * 0.5, size.y * 0.5 - position.y)
            / canvas_scale(size.x, size.y);
        button.active && button.contains(canvas_point)
    }
}

/// 装载中的两份 json 句柄。家具模型清单只用来把 glb 文件名反查回包名
/// （家具域已装载同一份，但没有把这张反查表放出来，而本单边界不改那
/// 个文件）；主表切片给出「这件家具有没有按钮」。
#[derive(Resource)]
pub(crate) struct ActionButtonTables {
    model_index: Handle<JsonAsset>,
    fixture_master: Handle<JsonAsset>,
}

/// 家具主表里本模块要读的三样：类别、动作类别、格占。
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixtureRow {
    pub fixture_type: FixtureType,
    /// 主表那一列的整数值：0 无动作 · 1 循环 · 2 演出 · 3 单次。
    /// 源的两个谓词直接比这个整数，所以这里存整数而不是枚举。
    pub action_value: i32,
    pub grid_width: i32,
    pub grid_depth: i32,
}

/// 解析好的家具面：glb 文件名 → 包名 → 主表行。
#[derive(Resource, Default)]
pub(crate) struct FixtureFacts {
    /// glb 文件名到包名。清单里一个包一条。
    package_by_glb: HashMap<String, String>,
    by_package: HashMap<String, FixtureRow>,
    parsed: bool,
}

impl FixtureFacts {
    /// 源侧「这是机关家具吗」：`(值 & ~2) == 1`，即循环与单次都算。
    fn is_gimmick(action_value: i32) -> bool {
        (action_value & !2) == 1
    }

    /// 源侧「这是演出家具吗」：`值 == 2`。
    fn is_timeline(action_value: i32) -> bool {
        action_value == 2
    }

    fn row_for_glb(&self, glb: &str) -> Option<&FixtureRow> {
        let package = self.package_by_glb.get(glb)?;
        self.by_package.get(package)
    }
}

/// 本模块的运行态。
#[derive(Resource)]
pub(crate) struct ActionButtonState {
    /// 玩家的附加碰撞盒，每帧被重建。
    player_box: CollisionBox2D,
    stack: ButtonStack,
    /// Opaque stack handles bind the actual candidate, never a world/grid position.
    fixture_candidates: HashMap<i32, FixtureButtonCandidate>,
    next_fixture_key: i32,
    /// 上一次接受输入的时刻（秒）。源用一个常量间隔节流。
    last_input: f32,
    /// 已上报过一次的栈首，用来只在变化时打日志。
    reported: Option<(ButtonType, TargetId)>,
    /// 场上家具的一次性对账是否已报。
    surveyed: bool,
}

struct FixtureButtonCandidate {
    entity: Entity,
    /// Identity may arrive after appearance; that must not hide/reorder the button.
    uid: Option<String>,
}

impl Default for ActionButtonState {
    fn default() -> Self {
        ActionButtonState {
            player_box: CollisionBox2D::new([0.0, 0.0], PLAYER_ADDITIONAL_HALF_EXTEND, 0.0),
            stack: ButtonStack::new(),
            fixture_candidates: HashMap::new(),
            next_fixture_key: 0,
            last_input: f32::NEG_INFINITY,
            reported: None,
            surveyed: false,
        }
    }
}

impl ActionButtonState {
    /// 当前栈首（决定屏幕上显示哪个按钮）。
    pub(crate) fn current(&self) -> Option<(ButtonType, TargetId)> {
        self.stack.first()
    }

    fn fixture_key(&mut self, entity: Entity, identity: Option<&FixtureActivityIdentity>) -> i32 {
        let uid = identity.map(|identity| identity.uid.as_str());
        if let Some((&key, candidate)) = self.fixture_candidates.iter_mut().find(|(_, candidate)| {
            candidate.entity == entity
                && candidate.uid.as_deref().zip(uid).map_or(true, |(previous, current)| previous == current)
        }) {
            // Bind a late identity to this same entity. Once known, retain it
            // through temporary absence; the existing activity owner rejects
            // stale Entity/UID pairs. A different known UID gets a new handle.
            if candidate.uid.is_none() {
                candidate.uid = uid.map(str::to_owned);
            }
            return key;
        }
        let key = self.next_fixture_key;
        self.next_fixture_key = key.checked_add(1).expect("fixture button handle space exhausted");
        self.fixture_candidates.insert(key, FixtureButtonCandidate {
            entity,
            uid: uid.map(str::to_owned),
        });
        key
    }

    fn fixture_target(&self, key: i32) -> Option<FixtureTarget> {
        let candidate = self.fixture_candidates.get(&key)?;
        Some(FixtureTarget {
            entity: candidate.entity,
            uid: candidate.uid.clone()?,
        })
    }
}

/// 本帧点按是否已被按钮吃掉。世界射线拾取读它，读到就不再打射线——
/// 真源里屏幕按钮在 UI 事件系统里，本来就在世界射线之前拦下点按。
#[derive(Resource, Default)]
pub(crate) struct ActionTapConsumed(pub(crate) bool);

/// Startup：请求全部图标与两张表。
///
/// 图标按真源那张表里出现过的全部文件名一次装齐（每张两三 KB），
/// 而不是只装当前接得出的两种——按钮类型是闭集，装齐以后新接一种
/// 不需要再改装载面。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    let mut art = ActionButtonArt::default();
    for button in ALL_BUTTON_TYPES {
        if let Some(name) = button.icon_file_name() {
            art.icons
                .entry(name)
                .or_insert_with(|| server.load::<Image>(moly_assets::ui_action_icon(name)));
        }
    }
    let count = art.icons.len();
    commands.insert_resource(art);
    commands.insert_resource(ActionButtonTables {
        model_index: server.load::<JsonAsset>(moly_assets::fixture_model_index()),
        fixture_master: server.load::<JsonAsset>(moly_assets::mysekai_fixtures()),
    });
    commands.init_resource::<ActionButtonState>();
    commands.init_resource::<FixtureFacts>();
    commands.init_resource::<ActionTapConsumed>();
    info!("[action_button] 图标装载请求 {count} 件（真源二十五格表里的去重文件名）");
}

/// 按钮类型闭集。源枚举取值不连续，这里逐个列出而不是数值区间遍历。
const ALL_BUTTON_TYPES: [ButtonType; 23] = [
    ButtonType::Talk,
    ButtonType::GimmickFixture,
    ButtonType::TimelineFixture,
    ButtonType::HouseEntry,
    ButtonType::OpenChest,
    ButtonType::GateEdit,
    ButtonType::OpenCraftTool,
    ButtonType::OpenMysekaiInfo,
    ButtonType::OpenMysekaiBgmSelect,
    ButtonType::OpenMysekaiConvert,
    ButtonType::ChangeActionTarget,
    ButtonType::OpenOtherMysekai,
    ButtonType::OpenAvatarDressUp,
    ButtonType::Dash,
    ButtonType::OpenSecretShop,
    ButtonType::GoHomeSite,
    ButtonType::LeaveMysekai,
    ButtonType::Sketch,
    ButtonType::Delivery,
    ButtonType::BirthdayCutScene,
    ButtonType::GateInvitation,
    ButtonType::OpenMysekaiCompetition,
    ButtonType::DeliveryInformation,
];

/// Update：两张表到齐即解析。清单缺件或主表缺列都响亮拒绝——一个静默
/// 空表会让所有家具都「没有按钮」，而那与「普通摆件本来就没有按钮」
/// 长得一模一样。
pub(crate) fn parse_tables(
    server: Res<AssetServer>,
    tables: Option<Res<ActionButtonTables>>,
    jsons: Res<Assets<JsonAsset>>,
    mut facts: ResMut<FixtureFacts>,
) {
    if facts.parsed {
        return;
    }
    let Some(tables) = tables else { return };
    for handle in [&tables.model_index, &tables.fixture_master] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("[action_button] 表装载失败（{err:?}）");
        }
    }
    let Some(index_json) = jsons.get(&tables.model_index) else {
        return;
    };
    let Some(master_json) = jsons.get(&tables.fixture_master) else {
        return;
    };

    let index: serde_json::Value = serde_json::from_str(&index_json.0)
        .unwrap_or_else(|err| panic!("[action_button] 家具模型清单不是合法 json：{err}"));
    // 清单的 packages 是一张按包名索引的字典（不是数组），glb 文件名在
    // 每条里。这里要的反查方向与它相反，所以逐条翻过来。
    let packages = index
        .get("packages")
        .and_then(|v| v.as_object())
        .expect("[action_button] 家具模型清单缺 packages 字典");
    for (name, entry) in packages {
        let Some(glb) = entry.get("glb").and_then(|v| v.as_str()) else {
            continue;
        };
        facts
            .package_by_glb
            .insert(glb.to_owned(), name.to_owned());
    }

    let master: serde_json::Value = serde_json::from_str(&master_json.0)
        .unwrap_or_else(|err| panic!("[action_button] 家具主表切片不是合法 json：{err}"));
    let rows = master
        .get("fixtures")
        .and_then(|v| v.as_array())
        .expect("[action_button] 家具主表切片缺 fixtures 数组");
    for row in rows {
        let bundle = row
            .get("assetbundleName")
            .and_then(|v| v.as_str())
            .expect("[action_button] 主表行缺 assetbundleName");
        let type_value = row
            .get("fixtureTypeValue")
            .and_then(|v| v.as_i64())
            .expect("[action_button] 主表行缺 fixtureTypeValue") as i32;
        let action_value = row
            .get("playerActionTypeValue")
            .and_then(|v| v.as_i64())
            .expect("[action_button] 主表行缺 playerActionTypeValue") as i32;
        let fixture_type = FixtureType::from_i32(type_value)
            .unwrap_or_else(|| panic!("[action_button] 主表行 {bundle} 的家具类别越界：{type_value}"));
        let grid_width = row.get("gridWidth").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        let grid_depth = row.get("gridDepth").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        // 摆放侧的包名是主表 assetbundleName 加一个固定前缀。
        //
        // ⚠ 主表里 assetbundleName **不唯一**：952 行落在 912 个包名上，
        // 25 个包名被多行共用（同一个模型的多个存档条目，例如植物的
        // 生长阶段）。而本模块只能按包名认场上的家具，所以一行覆盖
        // 另一行是必然的——**只要被覆盖的两行在本模块读的那几列上
        // 一致，覆盖就无害**。不一致时响亮拒绝：那意味着同一个模型
        // 会因为选了哪一行而有没有按钮，静默取一行等于掷硬币。
        let row_facts = FixtureRow {
            fixture_type,
            action_value,
            grid_width,
            grid_depth,
        };
        let package = format!("mysekai__fixture__{bundle}");
        if let Some(prev) = facts.by_package.get(&package) {
            let same_button = prev.fixture_type as i32 == row_facts.fixture_type as i32
                && prev.action_value == row_facts.action_value;
            assert!(
                same_button,
                "[action_button] 主表包名 {bundle} 被多行共用，且它们的家具类别或动作类别不同（{:?}/{} 与 {:?}/{}）——按包名认不出该用哪一行",
                prev.fixture_type, prev.action_value, row_facts.fixture_type, row_facts.action_value
            );
            // 格占不同只影响碰撞盒大小，取较大的那个：源按实例的存档
            // 格算，本仓按包名认，取大者不会漏掉本该能碰上的实例。
            let merged = FixtureRow {
                fixture_type: prev.fixture_type,
                action_value: prev.action_value,
                grid_width: prev.grid_width.max(row_facts.grid_width),
                grid_depth: prev.grid_depth.max(row_facts.grid_depth),
            };
            facts.by_package.insert(package, merged);
            continue;
        }
        facts.by_package.insert(package, row_facts);
    }

    let gimmick = facts
        .by_package
        .values()
        .filter(|row| FixtureFacts::is_gimmick(row.action_value))
        .count();
    let timeline = facts
        .by_package
        .values()
        .filter(|row| FixtureFacts::is_timeline(row.action_value))
        .count();
    let gated = facts
        .by_package
        .values()
        .filter(|row| {
            !FixtureFacts::is_gimmick(row.action_value)
                && !FixtureFacts::is_timeline(row.action_value)
                && row.fixture_type.can_action()
        })
        .count();
    facts.parsed = true;
    info!(
        "[action_button] 家具面解析完：主表 {} 行落在 {} 个包名上（包名不唯一，共用行在按钮那两列上一致）· 机关 {gimmick} · 演出 {timeline} · 类别放行 {gated} · glb 反查 {} 条",
        rows.len(),
        facts.by_package.len(),
        facts.package_by_glb.len()
    );
}

/// Update：源底图与图标到齐后铺同一个按钮根，栈首统一控制整棵显隐。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    art: Option<ResMut<ActionButtonArt>>,
    layouts: Option<Res<UiLayouts>>,
    roots: Query<(), With<ActionButtonRoot>>,
) {
    if !roots.is_empty() {
        return;
    }
    let (Some(mut art), Some(layouts)) = (art, layouts) else { return };
    if art.skin.is_none() {
        art.skin = ActionButtonSkin::from_layouts(&layouts, &server);
    }
    let Some(skin) = art.skin.as_ref() else { return; };
    match server.load_state(&skin.background) {
        LoadState::Loaded => {}
        LoadState::Failed(err) => panic!("[action_button] source button background failed: {err:?}"),
        _ => return,
    }
    // 对话与机关两张是当前接得出的两种按钮，等它们到齐即可铺件；
    // 其余图标晚到不挡这一步（换按钮类型时按句柄取当帧那张）。
    for name in ["icon_action_talk_wh", "icon_action_gimmick_wh"] {
        let Some(handle) = art.icons.get(name) else {
            return;
        };
        match server.load_state(handle) {
            LoadState::Loaded => {}
            LoadState::Failed(err) => panic!("[action_button] 图标 {name} 装载失败（{err:?}）"),
            _ => return,
        }
    }
    let background = commands.spawn((
        ActionButtonBackground,
        Sprite { image: skin.background.clone(), color: skin.background_color, ..default() },
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
    )).id();
    let icon = commands
        .spawn((
            ActionButtonIcon,
            Sprite {
                image: art.icons["icon_action_talk_wh"].clone(),
                color: skin.icon_color,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
            // Camera layers belong to renderable entities, not their parents.
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands
        .spawn((
            ActionButtonRoot,
            Visibility::Hidden,
            Transform::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .add_children(&[background, icon]);
    info!("[action_button] source button background and icon installed under one hidden root");
}

/// Update：每帧重建玩家的碰撞盒、与场上目标求交、把进出边沿写进栈。
///
/// 源侧这一步分在两处：碰撞管理器算进出、屏幕层的回调把它转成压栈与
/// 出栈。本仓没有那两层，于是在这里一次算完——等价的地方是**边沿**：
/// 只在「本帧相交且上帧不相交」时压栈，反之出栈；不是每帧重建整个栈。
#[allow(clippy::type_complexity)]
pub(crate) fn advance(
    mut state: ResMut<ActionButtonState>,
    eligibility: crate::interaction::InteractionEligibility,
    facts: Res<FixtureFacts>,
    players: Query<&Transform, With<PlayerControlled>>,
    npcs: Query<(&Transform, &CharacterUnitId, &WalkState), Without<PlayerControlled>>,
    fixtures: Query<
        (Entity, &Transform, &FixtureSource, &FixturePlacement, &GlobalTransform, Option<&FixtureActivityIdentity>),
        With<FixtureRoot>,
    >,
    server: Res<AssetServer>,
) {
    let Ok(player) = players.single() else {
        return;
    };
    if !facts.parsed {
        return;
    }
    state.player_box = crate::interaction::player_box(player);
    let player_box = state.player_box;

    // 本帧相交的目标全集，连带它该出哪个按钮。
    let mut touching: Vec<(ButtonType, TargetId)> = Vec::new();

    // 场上家具的一次性对账：家具铺完之后报一次「这个站点上哪几件有
    // 交互按钮、在哪」。没有这一条，「走了半天没看到家具按钮」分不出
    // 是这条链没接上、还是这个站点上本来就没有可交互家具——两者在
    // 屏幕上长得一样。数是每次运行现算的，不是写死的清单。
    let survey = !state.surveyed && fixtures.iter().len() > 0;
    let mut placed = 0usize;
    let mut joined = 0usize;
    let mut with_button: Vec<(ButtonType, [f32; 3], String)> = Vec::new();

    // Appearance reads proximity/site/visibility. Talk state and EnableTalk are
    // click-time checks in the sole dispatcher, not reasons to hide the button.
    for (transform, unit, _) in &npcs {
        if player_box.collides(&character_box(transform.translation.to_array()))
            && eligibility.for_unit(unit.0)
        {
            touching.push((ButtonType::Talk, TargetId::Character(unit.0)));
        }
    }

    for (entity, transform, source, _, _, identity) in &fixtures {
        placed += 1;
        let Some(path) = server.get_path(&source.0) else {
            continue;
        };
        let glb = path
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        let Some(row) = facts.row_for_glb(&glb) else {
            continue;
        };
        joined += 1;
        let Some(button) = fixture_button(row) else {
            continue;
        };
        if survey {
            with_button.push((button, transform.translation.to_array(), glb));
        }
        let target_box = fixture_box(
            transform.translation.to_array(),
            row.grid_width,
            row.grid_depth,
        );
        if player_box.collides(&target_box) {
            // Preserve the exact collision candidate. Missing activity identity
            // is not a new appearance gate and never selects another root.
            let key = state.fixture_key(entity, identity);
            touching.push((button, TargetId::Fixture(key)));
        }
    }

    if survey {
        state.surveyed = true;
        info!(
            "[action_button] 站点家具对账：铺了 {placed} 件 · 认回主表 {joined} 件 · 其中 {} 件有交互按钮",
            with_button.len()
        );
        for (button, position, glb) in &with_button {
            info!(
                "[action_button]   {button:?} @ ({:.2},{:.2},{:.2})  {glb}",
                position[0], position[1], position[2]
            );
        }
    }

    // 进沿：本帧相交而栈里没有 → 压栈（非优先，接队尾）。
    for (button, target) in &touching {
        if !state.stack.contains(*button, *target) {
            state.stack.push(*button, *target, false);
        }
    }
    // 出沿：栈里有而本帧不相交 → 出栈。
    let alive: Vec<TargetId> = touching.iter().map(|(_, t)| *t).collect();
    state
        .stack
        .retain_targets(&|target| alive.contains(&target));
    state.fixture_candidates.retain(|key, _| alive.contains(&TargetId::Fixture(*key)));

    let head = state.stack.first();
    if head != state.reported {
        // 玩家那只盒子的中心与角度每帧现算，随边沿一起报出来：光有
        // 「栈首变了」看不出它是被什么位置与朝向碰上的。
        match head {
            Some((button, target)) => info!(
                "[action_button] 栈首 {button:?} ← {target:?}（栈深 {}；玩家盒心 ({:.2},{:.2}) 角 {:.1}° 半长 ({:.2},{:.2})）",
                state.stack.len(),
                player_box.center[0],
                player_box.center[1],
                player_box.rotation,
                player_box.half_extend[0],
                player_box.half_extend[1],
            ),
            None => info!(
                "[action_button] 栈空，按钮收起（玩家盒心 ({:.2},{:.2}) 角 {:.1}°）",
                player_box.center[0], player_box.center[1], player_box.rotation
            ),
        }
        state.reported = head;
    }
}

/// 一件家具该出哪个按钮。三支照源的次序：机关 → 演出 → 按类别。
///
/// 生日演出那一支源侧还在前面插一道「当前是否在生日档期内」，本仓的
/// 生日面在别的域，这里不判——所以生日家具当前按它自己的动作类别走。
fn fixture_button(row: &FixtureRow) -> Option<ButtonType> {
    if FixtureFacts::is_gimmick(row.action_value) {
        return Some(ButtonType::GimmickFixture);
    }
    if FixtureFacts::is_timeline(row.action_value) {
        // 源侧这一支还要过「动作点位可用」与「玩家能否演出」两道运行时
        // 门，本仓没有那两个状态；见模块头的缺口条目。
        return Some(ButtonType::TimelineFixture);
    }
    if !row.fixture_type.can_action() {
        return None;
    }
    // 无动作类别的家具走另一张表：家具视图上那个「玩家能做什么」的
    // 字段，源从系统家具的建立处写入，不在家具主表里。本仓没有那个
    // 字段，所以这一支目前只能到这里——`system` 与 `gate` 两类家具
    // 会被上面的类别门放行，但出哪个按钮定不下来。
    None
}

/// Update（advance 之后）：按栈首摆件、换图标。
pub(crate) fn place_ui(
    state: Res<ActionButtonState>,
    screen: ActionButtonScreen,
    mut roots: Query<(&mut Visibility, &mut Transform), (With<ActionButtonRoot>, Without<Sprite>)>,
    mut parts: Query<(&mut Transform, &mut Sprite, Option<&ActionButtonIcon>), Or<(With<ActionButtonIcon>, With<ActionButtonBackground>)>>,
) {
    let Ok((_, window)) = screen.windows.single() else {
        return;
    };
    let head = state.current();
    let size = Vec2::new(window.width(), window.height());
    let rects = screen.rects(size);
    let scale = canvas_scale(size.x, size.y);
    for (mut visibility, mut transform) in &mut roots {
        *visibility = if head.is_some() && rects.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::new(scale, scale, 1.0);
    }
    let Some((button, _)) = head else { return };
    let (Some(art), Some((background_rect, icon_rect))) = (screen.art.as_deref(), rects) else { return; };
    for (mut transform, mut sprite, icon) in &mut parts {
        let rect = if icon.is_some() { &icon_rect } else { &background_rect };
        let (local_scale, rotation, _) = rect.world.to_scale_rotation_translation();
        *transform = Transform {
            translation: rect.center().extend(if icon.is_some() { 0.5 } else { 0.0 }),
            rotation,
            scale: local_scale,
        };
        sprite.custom_size = Some(rect.size);
        if let Some(name) = button.icon_file_name().filter(|_| icon.is_some()) {
            if let Some(handle) = art.icons.get(name) {
                if sprite.image != *handle {
                    sprite.image = handle.clone();
                }
            }
        }
    }
}

/// Update（拾取之前）：点按落在按钮上就分派动作并吃掉这一帧的点按。
///
/// 节流照源：两次输入之间至少隔 [`ACTION_BUTTON_INPUT_INTERVAL`] 秒。
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut commands: Commands,
    mut state: ResMut<ActionButtonState>,
    mut consumed: ResMut<ActionTapConsumed>,
    eligibility: crate::interaction::InteractionEligibility,
    time: Res<Time>,
    screen: ActionButtonScreen,
    roots: Query<&Visibility, With<ActionButtonRoot>>,
    npcs: Query<(Entity, &CharacterUnitId, &WalkState), Without<PlayerControlled>>,
    mut talk_requests: MessageWriter<PlayerTalkRequest>,
    mut fixture_requests: MessageWriter<crate::player_fixture_action::PlayerFixtureRequest>,
    mut layer_commands: MessageWriter<LayerCommand>,
) {
    consumed.0 = false;
    let Ok((_, window)) = screen.windows.single() else {
        return;
    };
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind == GestureKind::Tap && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() || !eligibility.available()
        || !roots.iter().any(|visibility| *visibility != Visibility::Hidden) {
        return;
    }
    let Some((button, target)) = state.current() else {
        return;
    };
    let fixture_target = match target {
        TargetId::Fixture(key) => state.fixture_target(key),
        TargetId::Character(_) => None,
    };
    let size = Vec2::new(window.width(), window.height());
    for position in taps {
        if !screen.hit(position, size) {
            continue;
        }
        let now = time.elapsed_secs();
        if button != ButtonType::Talk && now - state.last_input < ACTION_BUTTON_INPUT_INTERVAL {
            info!(
                "[action_button] 点按被节流（距上次 {:.3}s < {ACTION_BUTTON_INPUT_INTERVAL}s）",
                now - state.last_input
            );
            consumed.0 = true;
            continue;
        }
        if button != ButtonType::Talk { state.last_input = now; }
        consumed.0 = true;
        if let TargetId::Character(unit) = target {
            if !eligibility.for_unit(unit) { continue; }
        }
        dispatch(
            button,
            target,
            &npcs,
            &mut talk_requests,
            &mut fixture_requests,
            &mut commands,
            &mut layer_commands,
            fixture_target.as_ref(),
        );
    }
}

/// 按按钮类型分派。
///
/// **多个目标同时在范围内时按哪一个：栈首。** 这不是推断——源侧的对话
/// 按钮子类在点下时读的就是 `_onShowButtonStack[0]`，拿它的目标号去找
/// NPC。普通项入栈接队尾，所以「先进入范围的那个」胜出。
///
/// Talk这里只发布意图。目标状态/EnableTalk、玩家安全避让位、输入节流
/// 与内容路由由唯一player_talk dispatcher处理；不能再按附近家具选段。
/// 不能交谈的源气泡/选择音尚须UI反馈供给，本入口不假造通用文字弹框。
///
/// 分派目标三族：**真域**（玩家对话 · 站点切换请求——与小地图点站同一条
/// 路）· **具名层槽**（压层命令进屏幕层栈；层视图未建由层栈响亮记账，
/// 压层梯本身是真的）· **响亮未建**（按下沿到此，日志具名）。
///
/// ⛔ 源的 switch 里有四格**根本不在那张表上**（速写 · 投递 · 造景比赛 ·
/// 投递信息）——源里走 default 抛异常，处理在各自的按钮子类里（投递、
/// 采集、传送门邀请都是独立类）。本仓同样不把它们写进这张表：它们落
/// 兜底臂记「未接」，往下补的时候先读这句：那四格补进来就是错的。
/// 同理 `OpenMysekaiBgmSelect` 源侧前面还有一道教程门（教程未完不弹
/// 选曲层）——本仓无教程域，直接压层，具名挂账。
fn dispatch(
    button: ButtonType,
    target: TargetId,
    npcs: &Query<(Entity, &CharacterUnitId, &WalkState), Without<PlayerControlled>>,
    talk_requests: &mut MessageWriter<PlayerTalkRequest>,
    fixture_requests: &mut MessageWriter<crate::player_fixture_action::PlayerFixtureRequest>,
    commands: &mut Commands,
    layer_commands: &mut MessageWriter<LayerCommand>,
    fixture_target: Option<&FixtureTarget>,
) {
    match (button, target) {
        (ButtonType::Talk, TargetId::Character(unit)) => {
            let Some((entity, _, _)) = npcs.iter().find(|(_, id, _)| id.0 == unit) else {
                info!("[action_button] 对话按钮按下但 unit {unit} 已离场，丢弃");
                return;
            };
            talk_requests.write(PlayerTalkRequest { entity, unit });
            info!("[action_button] 对话按钮按下 → unit {unit}（{entity:?}）玩家对话请求入队");
        }
        (ButtonType::GimmickFixture | ButtonType::TimelineFixture, TargetId::Fixture(key)) => {
            if let Some(target) = fixture_target {
                let request = if button == ButtonType::GimmickFixture {
                    crate::player_fixture_action::PlayerFixtureRequest::Gimmick(target.clone())
                } else {
                    crate::player_fixture_action::PlayerFixtureRequest::Timeline(target.clone())
                };
                fixture_requests.write(request);
                info!("[action_button] {button:?} → typed furniture request（候选键 {key}，实体 {:?}，UID {}）", target.entity, target.uid);
            } else {
                info!("[action_button] {button:?} 按下但该候选的实例身份未就绪（候选键 {key}）");
            }
        }
        (ButtonType::HouseEntry, _) => {
            // 源侧进屋换层（房子家具 → 室内场地屏）。本仓室内是站点
            // （first_floor），走与小地图点站同一条站点切换请求。
            commands.insert_resource(crate::site::SiteChangeRequest(
                "first_floor".to_owned(),
            ));
            info!("[action_button] 进屋按钮按下 → 站点切换请求 first_floor（与小地图点站同一条路）");
        }
        (ButtonType::GoHomeSite, _) => {
            commands.insert_resource(crate::site::SiteChangeRequest(
                "home_site".to_owned(),
            ));
            info!("[action_button] 回家按钮按下 → 站点切换请求 home_site");
        }
        (ButtonType::OpenChest, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiInventory));
            info!("[action_button] 宝箱按钮按下 → PushUIScreen(库存 607) → 层栈压层");
        }
        (ButtonType::GateEdit, _) => {
            // 源侧带启动参数（门位等）——本仓层栈元素暂只记层身份，
            // 参数格在层视图补齐时扩栈元素。
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiGate));
            info!("[action_button] 门编辑按钮按下 → PushUIScreen(传送门编辑 628，源带启动参数，参数格未建) → 层栈压层");
        }
        (ButtonType::GateInvitation, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiGateInvitation));
            info!("[action_button] 门邀请按钮按下 → PushUIScreen(传送门邀请 645，源带启动参数) → 层栈压层");
        }
        (ButtonType::OpenCraftTool, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiCraft));
            info!("[action_button] 工作台按钮按下 → PushUIScreen(工坊 615) → 层栈压层");
        }
        (ButtonType::OpenMysekaiInfo, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiInfo));
            info!("[action_button] 情报按钮按下 → PushUIScreen(家具情报 622，源带启动参数) → 层栈压层");
        }
        (ButtonType::OpenMysekaiBgmSelect, _) => {
            // 教程门（教程未完不弹层）本仓无对应域，直接压层——具名挂账
            // 见本函数头。
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiBgmSelect));
            info!("[action_button] 选曲按钮按下 → PushUIScreen(选曲 626，教程门未建直接压层) → 层栈压层");
        }
        (ButtonType::OpenMysekaiConvert, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiConvert));
            info!("[action_button] 变换按钮按下 → PushUIScreen(家具变换 627) → 层栈压层");
        }
        (ButtonType::OpenOtherMysekai, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiVisitTop));
            info!("[action_button] 访问按钮按下 → PushUIScreen(访客一览 658) → 层栈压层");
        }
        (ButtonType::OpenAvatarDressUp, _) => {
            layer_commands.write(LayerCommand::Push(
                LayerId::MysekaiAvatarCostumeSetting,
            ));
            info!("[action_button] 换装按钮按下 → PushUIScreen(换装 629) → 层栈压层");
        }
        (ButtonType::OpenSecretShop, _) => {
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiSecretShop));
            info!("[action_button] 秘密商店按钮按下 → PushUIScreen(秘密商店 625) → 层栈压层");
        }
        (ButtonType::BirthdayCutScene, target) => {
            info!("[action_button] 生日演出按钮按下（目标 {target:?}；生日演出域未建，具名挂账）——按下沿到此");
        }
        (ButtonType::Dash, _) => {
            info!("[action_button] 冲刺按钮按下 → 冲刺域未建（具名挂账）——按下沿到此");
        }
        (ButtonType::ChangeActionTarget, _) => {
            info!("[action_button] 换目标按钮按下 → 目标切换域未建（具名挂账）——按下沿到此");
        }
        (other, target) => {
            info!("[action_button] {other:?} ← {target:?}：本仓未接这一支");
        }
    }
}

/// 走位目标：名册成员逐个路过，锚定家具逐个驻足（锚旁等着 wandering
/// 的角色路过 ⇒ 采到配对那一栏）。
#[derive(Debug, Clone, Copy)]
enum WalkTarget {
    Npc(Entity),
    Anchor(i32, Vec3),
}

/// 配对门运行的冒烟口（`MOLY_ACTION_BUTTON_AUTOWALK_SECS`）：把玩家
/// 依次开到每个角色与每个锚定家具跟前。触摸注入进 `TouchInput` 消息流
/// ——与引擎触摸同一条流，走真实的摇杆链（捕获 → 方向 → 移动向量），
/// 不绕任何一行。两臂：
///
/// * **走位臂**（第一根指，摇杆区内）：量按钮**出现**的两栏对照
///   （配对入栈 / 未配对被拦），与逐 10 秒的配对面普查互为印证。
/// * **点按臂**（第二根指，按钮屏位＝区外）：栈首是对话按钮时隔
///   2s 起落一次，走真实的 起落 → 手势层 TAP → 点按 → 命中 → 分派
///   链——配对且空闲的入队、参演自主对话的被门二拒，两条日志都是
///   运行时证据。0.1s 松开保持 TAP 形（长按阈值 0.25s 之前收场）。
///
/// 目标轮转：角色按实体序逐个路过（到点水平距 < 0.9m 即换下一个），
/// 一圈之后到锚定家具处驻足 15 秒再换。40 秒到不了就放弃换下一个。
/// armed 窗口收尾与让位门落下时两根指都松——不留按着的幽灵方向。
pub(crate) fn smoke_autowalk(
    mut touches: MessageWriter<TouchInput>,
    screen: ActionButtonScreen,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    fixtures: Query<(&FixturePlacement, &GlobalTransform), With<FixtureRoot>>,
    players: Query<&Transform, With<PlayerControlled>>,
    npcs: Query<(Entity, &Transform), (With<CharacterUnitId>, Without<PlayerControlled>)>,
    joystick: Res<JoystickState>,
    button_state: Res<ActionButtonState>,
    time: Res<Time>,
    mut pressing: Local<bool>,
    mut tap_pressing: Local<bool>,
    mut tap_pressed_at: Local<f32>,
    mut next_tap: Local<f32>,
    mut index: Local<usize>,
    mut target_since: Local<Option<f32>>,
    mut linger_until: Local<f32>,
) {
    let armed = env_secs("MOLY_ACTION_BUTTON_AUTOWALK_SECS");
    if armed <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    let Ok((window_entity, window)) = screen.windows.single() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    const FINGER: u64 = 99007;
    const TAP_FINGER: u64 = 99008;
    let base = Vec2::new(width * 0.15, height * 0.75);
    let write = |touches: &mut MessageWriter<TouchInput>, phase: TouchPhase, position: Vec2| {
        touches.write(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: FINGER,
        });
    };
    let write_tap =
        |touches: &mut MessageWriter<TouchInput>, phase: TouchPhase, position: Vec2| {
            touches.write(TouchInput {
                phase,
                position,
                window: window_entity,
                force: None,
                id: TAP_FINGER,
            });
        };
    // 按钮的屏位（顶原点）：与摆件同一式——覆盖相机世界心翻回屏坐标。
    let Some(button_pos) = screen.button_position(Vec2::new(width, height)) else { return; };
    // 收尾：armed 窗口一过两根指都松（走指若还按着，摇杆会拖着最后
    // 那个方向一直走）。
    if now >= armed {
        if *pressing {
            write(&mut touches, TouchPhase::Ended, base);
            *pressing = false;
        }
        if *tap_pressing {
            write_tap(&mut touches, TouchPhase::Ended, button_pos);
            *tap_pressing = false;
        }
        return;
    }
    // 让位门落下（编辑 / 玩家自己的对话会话）：触摸区此刻归手势层，
    // 走指注入只会点出别的东西；两指都松，等门再开。
    if !joystick.enabled {
        if *pressing {
            write(&mut touches, TouchPhase::Ended, base);
            *pressing = false;
        }
        if *tap_pressing {
            write_tap(&mut touches, TouchPhase::Ended, button_pos);
            *tap_pressing = false;
        }
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    // 点按臂：栈首是对话按钮时隔 ≥2s 在按钮屏位起落一次。0.1s 收场
    // 在长按阈值（0.25s）之前、点按节流（0.3s）之外，都是真链上的门。
    if *tap_pressing {
        if now - *tap_pressed_at >= 0.1 {
            write_tap(&mut touches, TouchPhase::Ended, button_pos);
            *tap_pressing = false;
            *next_tap = now + 2.0;
        }
    } else if now >= *next_tap
        && button_state
            .current()
            .is_some_and(|(button, _)| button == ButtonType::Talk)
    {
        let (_, target) = button_state.current().expect("上面刚查过栈首");
        write_tap(&mut touches, TouchPhase::Started, button_pos);
        *tap_pressing = true;
        *tap_pressed_at = now;
        info!(
            "[autowalk] 点按对话按钮屏位 ({:.0},{:.0})，栈首 {target:?}",
            button_pos.x, button_pos.y
        );
    }
    // 目标集：本帧快照。角色按实体序、锚按 fixture id 序——次序确定，
    // 轮转的步进只在到点/超时发生。
    let mut targets: Vec<WalkTarget> = npcs
        .iter()
        .map(|(entity, _)| WalkTarget::Npc(entity))
        .collect();
    let mut anchors: Vec<(i32, Vec2)> = fixtures.iter()
        .filter(|(placement, _)| placement.fixture_id != 0)
        .map(|(placement, global)| (placement.fixture_id, global.translation().xz()))
        .collect();
    anchors.sort_by_key(|(id, _)| *id);
    targets.extend(
        anchors
            .iter()
            .map(|(id, position)| WalkTarget::Anchor(*id, position.extend(0.0))),
    );
    if targets.is_empty() {
        return;
    }
    // 驻足中：站在锚旁等，触摸回底盘心（方向零 → 站定）。
    if now < *linger_until {
        if *pressing {
            write(&mut touches, TouchPhase::Moved, base);
        }
        return;
    }
    if *linger_until > 0.0 {
        // 驻足刚结束：清掉时间戳，换下一个目标。
        *linger_until = 0.0;
        *index += 1;
        *target_since = Some(now);
    }
    // 目标推进：到点或超时就换。换目标不松指——底盘不变，方向下一帧
    // 自然转向新目标。跳步每帧封顶一圈（全部目标都到点/离场时不再打转）。
    let mut hops = 0usize;
    loop {
        if hops > targets.len() {
            return;
        }
        let target_position = match targets[*index % targets.len()] {
            WalkTarget::Npc(entity) => match npcs.get(entity) {
                Ok((_, transform)) => transform.translation,
                // 目标已离场（换站/重播种）：跳下一个。
                Err(_) => {
                    *index += 1;
                    *target_since = Some(now);
                    hops += 1;
                    continue;
                }
            },
            WalkTarget::Anchor(_, position) => position,
        };
        let delta = target_position - player.translation;
        let distance = (delta.x * delta.x + delta.z * delta.z).sqrt();
        let since = *target_since.get_or_insert(now);
        if distance < 0.9 {
            match targets[*index % targets.len()] {
                WalkTarget::Npc(entity) => {
                    info!("[autowalk] 到点角色 {entity:?}（距 {distance:.2}m）→ 下一个");
                    *index += 1;
                }
                WalkTarget::Anchor(id, _) => {
                    info!("[autowalk] 到点锚 fixture {id}，驻足 15s 等配对");
                    *linger_until = now + 15.0;
                }
            }
            *target_since = Some(now);
            if *linger_until > now {
                return;
            }
            hops += 1;
            continue;
        }
        if now - since > 40.0 {
            let label = match targets[*index % targets.len()] {
                WalkTarget::Npc(entity) => format!("角色 {entity:?}"),
                WalkTarget::Anchor(id, _) => format!("锚 fixture {id}"),
            };
            info!("[autowalk] 40s 未到 {label}（距 {distance:.1}m）→ 放弃换下一个");
            *index += 1;
            *target_since = Some(now);
            hops += 1;
            continue;
        }
        // 转向：世界方向反解回摇杆方向（与摇杆层的烘基式互逆）——
        // joy.x = 世界向 · 相机右，joy.y = 世界向 · 相机前（去 y 归一）。
        let mut dir_world = Vec2::new(delta.x, delta.z);
        if dir_world.length_squared() < f32::EPSILON {
            dir_world = Vec2::X;
        }
        let dir_world = dir_world.normalize();
        let forward = camera.forward();
        let horizontal = (forward.x * forward.x + forward.z * forward.z).sqrt();
        let forward_flat = if horizontal <= 0.00001 {
            Vec2::ZERO
        } else {
            Vec2::new(forward.x / horizontal, forward.z / horizontal)
        };
        let right = camera.right();
        let right_flat = Vec2::new(right.x, right.z);
        let joy = Vec2::new(
            dir_world.dot(right_flat),
            dir_world.dot(forward_flat),
        );
        let radius = HANDLE_SIZE * canvas_scale(width, height);
        // 触点 = 底盘 + (joy.x, -joy.y) × 半径（屏坐标 y 向下）。
        let position = base + Vec2::new(joy.x, -joy.y) * radius;
        if !*pressing {
            write(&mut touches, TouchPhase::Started, base);
            *pressing = true;
            let label = match targets[*index % targets.len()] {
                WalkTarget::Npc(entity) => format!("角色 {entity:?}"),
                WalkTarget::Anchor(id, _) => format!("锚 fixture {id}"),
            };
            info!(
                "[autowalk] 起手 @({:.0},{:.0})，目标 {label}（距 {distance:.1}m，摇杆半径 {:.0}px）",
                base.x, base.y, radius
            );
            return;
        }
        write(&mut touches, TouchPhase::Moved, position);
        return;
    }
}

/// 环境变量秒数（缺省 0）：与各域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
