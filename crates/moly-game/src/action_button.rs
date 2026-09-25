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
//!
//! Buttons follow the collision manager's edges, not the current overlap.
//! One ordered colliding list (collideObjList) holds the fixtures and NPCs
//! the player's box overlaps, and the room door's sensor while the player
//! stands inside its sphere, apart from the button stack: an object joining
//! it runs AddShowButtonStack once, an object leaving it runs
//! RemoveShowButtonStack, and nothing is re-tested while the player stays
//! inside. The scan stops in Edit and in a conversation with the player
//! (ObjectCollisionManager.IsCanUpdate), which produces no edges at all, and
//! during a site move. A timeline started from the button locks the stack
//! (SetLockActionButton) and hides the view until the player leaves
//! UseTimelineFixture; RemoveNotCollisionObject then prunes it. A Talk tap
//! prunes the same way once the talk action is done.
//!
//! The buttons belong to ScreenLayerMysekaiHome. While another screen is
//! current, a site move included, they are not shown; when the home screen
//! mounts again its new presenter starts from an empty, unlocked stack and
//! runs ObjectCollisionManager.ForceUpdate.

pub(crate) mod admission;

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;

use moly_law::action_button::{
    character_box, fixture_box, inside_circle, ButtonStack, ButtonType, CollisionBox2D,
    FixtureType, PlayerActionType, TargetId, ACTION_BUTTON_INPUT_INTERVAL,
    PLAYER_ADDITIONAL_HALF_EXTEND,
};

use admission::{
    AdmissionInputs, Availability, Enter, FixtureKind, InputRevision, Probe, Retry,
};

use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::fixture::{FixturePlacement, FixtureRoot, FixtureSource};
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureTarget};
use crate::fixture_edit::LayoutSaved;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::joystick::{JoystickState, HANDLE_SIZE};
use crate::npc::{CharacterUnitId, WalkState};
use crate::player::PlayerControlled;
use crate::player_state::PlayerActionState;
use crate::player_talk::PlayerTalkRequest;
use crate::ui_layers::{LayerCommand, LayerId};
use crate::ui_layout::{UiLayouts, UiPrefabView};

// The Home and MyRoom talk/fixture buttons share this authored skin. Keep the
// complete ancestor geometry: the controller is anchored to the bottom-right,
// while its button Image and the smaller RawImage icon have separate rects.
const SKIN_LAYOUT: &str = "ShellHome";
const BUTTON_NODE: &str = "ActionButtonController/TalkActionButton";
const ICON_NODE: &str = "ActionButtonController/TalkActionButton/Icon";
// The go-home button is its own view on the room screen (MyRoomSiteSelector
// on GoHomeButton): the prefab carries both its circle and its home icon,
// so it takes no icon from the button-type table.
const GO_HOME_LAYOUT: &str = "ShellMyRoom";
const GO_HOME_NODE: &str = "ActionButtonController/GoHomeButton";
const GO_HOME_ICON_NODE: &str = "ActionButtonController/GoHomeButton/CustomImage";
// The change-target button of the same controller: its own Image is fully
// transparent, the visible part is the icon the view loads by type.
const CHANGE_NODE: &str = "ActionButtonController/ChangeSelectTargetButton";
const CHANGE_ICON_NODE: &str = "ActionButtonController/ChangeSelectTargetButton/Icon";

/// 按钮根（图标件的父，可见性随栈首）。
#[derive(Component)]
pub(crate) struct ActionButtonRoot;

/// 图标件。
#[derive(Component)]
pub(crate) struct ActionButtonIcon;

/// The change-target button (SetChangeButton shows it).
#[derive(Component)]
pub(crate) struct ActionButtonChange;

#[derive(Component)]
pub(crate) struct ActionButtonBackground;

/// 图标纹源：按文件名存一份句柄。文件名由按钮类型给出（真源的那张
/// 二十五格表），所以这里不是按「用途」而是按真源文件名索引。
#[derive(Resource, Default)]
pub(crate) struct ActionButtonArt {
    icons: HashMap<&'static str, Handle<Image>>,
    skin: Option<ActionButtonSkin>,
    go_home: Option<ActionButtonSkin>,
}

impl ActionButtonArt {
    /// The view that shows this button: the go-home selector for its
    /// type, the shared talk and fixture button for the others.
    fn skin_for(&self, button: ButtonType) -> Option<&ActionButtonSkin> {
        if button == ButtonType::GoHomeSite {
            self.go_home.as_ref()
        } else {
            self.skin.as_ref()
        }
    }
}

struct ActionButtonSkin {
    background: Handle<Image>,
    background_color: Color,
    /// The icon the prefab itself carries, for a view whose icon is not
    /// chosen by button type.
    icon: Option<Handle<Image>>,
    icon_color: Color,
    geometry: UiPrefabView,
    button_node: &'static str,
    icon_node: &'static str,
}

impl ActionButtonSkin {
    fn from_layouts(
        layouts: &UiLayouts,
        server: &AssetServer,
        layout: &'static str,
        button_node: &'static str,
        icon_node: &'static str,
    ) -> Option<Self> {
        let doc = layouts.document(layout)?;
        let background_node = &doc.nodes[doc.find(button_node).expect("source action button")];
        let icon_node_data = &doc.nodes[doc.find(icon_node).expect("source action icon")];
        let background = background_node
            .components
            .iter()
            .find(|component| {
                component.enabled
                    && component.class.ends_with("Image")
                    && component.sprite.is_some()
            })
            .expect("source action button Image");
        // The shared button's icon is a RawImage whose texture the view sets
        // by button type; the go-home icon is an Image with its own sprite.
        let icon = icon_node_data
            .components
            .iter()
            .find(|component| component.enabled && component.class.ends_with("Image"))
            .expect("source action icon image");
        let image = background
            .sprite
            .as_ref()
            .and_then(|sprite| sprite["image"].as_str())
            .expect("source action button Sprite image");
        let icon_image = icon
            .sprite
            .as_ref()
            .and_then(|sprite| sprite["image"].as_str())
            .map(|image| {
                moly_assets::residency::load_image(server, crate::ui_layout::image_asset_path(image))
            });
        let color = |component: &moly_assets::ui_layout::UiComponent| {
            let values = component.fields["m_Color"]
                .as_array()
                .expect("source UI color");
            let channel = |i: usize| values[i].as_f64().expect("source UI color channel") as f32;
            Color::srgba(channel(0), channel(1), channel(2), channel(3))
        };
        // The root Image is the always-present button background. The separate
        // Cover is a press/disabled overlay: MysekaiActionInternalButton.OnEnable
        // calls HideCover, so it must not be mistaken for the normal gray base.
        let mut geometry = UiPrefabView::new(layout, BALLOON_LAYER);
        geometry.set_visible(button_node, true);
        Some(Self {
            // The prefab UI loads its images through the same asset path for
            // the GPU only; the first request's settings win, so this one agrees.
            background: moly_assets::residency::load_image(
                server,
                crate::ui_layout::image_asset_path(image),
            ),
            background_color: color(background),
            icon: icon_image,
            icon_color: color(icon),
            geometry,
            button_node,
            icon_node,
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
    root: Option<Res<'w, RootCanvas>>,
}

impl ActionButtonScreen<'_, '_> {
    /// Logical pixels per canvas unit, once the host canvas has loaded.
    fn scale(&self, window: &Window) -> Option<f32> {
        Some(self.root.as_deref()?.scale(window))
    }

    fn rects(
        &self,
        window: &Window,
        shown: ButtonType,
    ) -> Option<(
        moly_assets::ui_layout::UiRect,
        moly_assets::ui_layout::UiRect,
    )> {
        let skin = self.art.as_deref()?.skin_for(shown)?;
        let layouts = self.layouts.as_deref()?;
        let canvas = self.root.as_deref()?.size(window);
        Some((
            skin.geometry.rect(layouts, skin.button_node, canvas)?,
            skin.geometry.rect(layouts, skin.icon_node, canvas)?,
        ))
    }

    /// The change-target button's rect and its icon's, laid out in the head
    /// view's own layout.
    fn change_rects(
        &self,
        window: &Window,
        shown: ButtonType,
    ) -> Option<(
        moly_assets::ui_layout::UiRect,
        moly_assets::ui_layout::UiRect,
    )> {
        let skin = self.art.as_deref()?.skin_for(shown)?;
        let layouts = self.layouts.as_deref()?;
        let canvas = self.root.as_deref()?.size(window);
        Some((
            skin.geometry.rect(layouts, CHANGE_NODE, canvas)?,
            skin.geometry.rect(layouts, CHANGE_ICON_NODE, canvas)?,
        ))
    }

    fn change_position(&self, window: &Window, shown: ButtonType) -> Option<Vec2> {
        let (button, _) = self.change_rects(window, shown)?;
        let center = button.center() * self.scale(window)?;
        Some(Vec2::new(window.width() * 0.5 + center.x, window.height() * 0.5 - center.y))
    }

    fn change_hit(&self, position: Vec2, window: &Window, shown: ButtonType) -> bool {
        let (Some((button, _)), Some(scale)) =
            (self.change_rects(window, shown), self.scale(window))
        else {
            return false;
        };
        let canvas_point = Vec2::new(position.x - window.width() * 0.5, window.height() * 0.5 - position.y)
            / scale;
        button.active && button.contains(canvas_point)
    }

    fn button_position(&self, window: &Window, shown: ButtonType) -> Option<Vec2> {
        let (button, _) = self.rects(window, shown)?;
        let center = button.center() * self.scale(window)?;
        Some(Vec2::new(window.width() * 0.5 + center.x, window.height() * 0.5 - center.y))
    }

    fn hit(&self, position: Vec2, window: &Window, shown: ButtonType) -> bool {
        let (Some((button, _)), Some(scale)) = (self.rects(window, shown), self.scale(window))
        else {
            return false;
        };
        let canvas_point = Vec2::new(position.x - window.width() * 0.5, window.height() * 0.5 - position.y)
            / scale;
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

/// One placed fixture's table facts, resolved once. The root's model handle
/// is set at spawn and never replaced, and the parsed tables never change, so
/// the cached answer is the answer the per-frame lookup would give.
#[derive(Component, Clone)]
pub(crate) struct ActionButtonFixture {
    row: Option<FixtureRow>,
    button: Option<ButtonType>,
    glb: String,
}

/// One collision object the player's box overlaps.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Collider {
    /// A placed fixture, by instance and identity.
    Fixture(Entity, String),
    /// An NPC, by instance and character.
    Character(Entity, u32),
    /// The room door's sensor, by its attach point.
    Sensor(Entity),
}

impl Collider {
    fn entity(&self) -> Entity {
        match self {
            Collider::Fixture(entity, _)
            | Collider::Character(entity, _)
            | Collider::Sensor(entity) => *entity,
        }
    }
}

/// collideObjList: the overlapped objects in the order they joined.
/// ForceUpdate re-enters them in this order, after the newly overlapping.
#[derive(Default)]
struct CollideList(Vec<Collider>);

impl CollideList {
    fn contains(&self, entity: Entity) -> bool {
        self.0.iter().any(|collider| collider.entity() == entity)
    }

    fn fixture_uid(&self, entity: Entity) -> Option<&str> {
        self.0.iter().find_map(|collider| match collider {
            Collider::Fixture(joined, uid) if *joined == entity => Some(uid.as_str()),
            _ => None,
        })
    }

    /// IsCollisionObject for a Talk entry: the NPC instance of that character.
    fn character(&self, unit: u32) -> Option<Entity> {
        self.0.iter().find_map(|collider| match collider {
            Collider::Character(entity, joined) if *joined == unit => Some(*entity),
            _ => None,
        })
    }

    /// TryAddObjToCollideList: appended once.
    fn join(&mut self, collider: Collider) {
        if !self.contains(collider.entity()) {
            self.0.push(collider);
        }
    }

    fn leave(&mut self, entity: Entity) -> Option<Collider> {
        let index = self.0.iter().position(|collider| collider.entity() == entity)?;
        Some(self.0.remove(index))
    }

    fn retain(&mut self, keep: impl FnMut(&Collider) -> bool) {
        self.0.retain(keep);
    }

    fn take(&mut self) -> Vec<Collider> {
        std::mem::take(&mut self.0)
    }
}

/// 本模块的运行态。
#[derive(Resource)]
pub(crate) struct ActionButtonState {
    /// 玩家的附加碰撞盒，每帧被重建。
    player_box: CollisionBox2D,
    stack: ButtonStack,
    /// collideObjList. Admission runs only when an object joins it.
    colliding: CollideList,
    /// Rising edges the given navigation snapshot could not evaluate.
    deferred: HashMap<Entity, InputRevision>,
    /// Opaque stack handles bind the admitted instance, never a position.
    fixture_targets: HashMap<i32, FixtureTarget>,
    next_fixture_key: i32,
    /// SetLockActionButton on the current model (BlockStackChange): a
    /// timeline started from the button holds the stack.
    locked: bool,
    /// Counts the models: each rebuild of the home screen makes a new one.
    model: u64,
    /// TimelineFixtureProcess hid the buttons view; the model whose button
    /// started that timeline. TimelineFixtureFinishAsync shows the view again
    /// once the player leaves UseTimelineFixture.
    timeline: Option<u64>,
    /// A Talk tap reached the talk action last frame.
    talk_tapped: bool,
    /// ScreenLayerMysekaiHome was the current screen last frame.
    home_mounted: bool,
    /// The site generation the scan last saw.
    site_epoch: Option<u64>,
    /// The old site is torn down and the new generation has not arrived.
    awaiting_site: bool,
    /// The new generation arrived this frame; its actors are placed after
    /// this scan.
    arriving: bool,
    /// ObjectCollisionManager.ForceUpdate waits for the next scan.
    force_update: bool,
    /// This frame's player and world position, for the click's re-check.
    player: Option<(Entity, Vec3)>,
    /// 上一次接受输入的时刻（秒）。源用一个常量间隔节流。
    last_input: f32,
    /// 已上报过一次的栈首，用来只在变化时打日志。
    reported: Option<(ButtonType, TargetId)>,
    /// 场上家具的一次性对账是否已报。
    surveyed: bool,
    /// The door sensor entry's last next-frame wait, named once per reason.
    sensor_wait: Option<String>,
    /// The change-target button was tapped; OnChangeActionTarget runs on
    /// the next scan.
    change_tapped: bool,
    /// The house-entry candidate's overlap and last next-frame wait, named
    /// when they change.
    house_touching: Option<bool>,
    house_wait: Option<String>,
}

impl Default for ActionButtonState {
    fn default() -> Self {
        ActionButtonState {
            player_box: CollisionBox2D::new([0.0, 0.0], PLAYER_ADDITIONAL_HALF_EXTEND, 0.0),
            stack: ButtonStack::new(),
            colliding: CollideList::default(),
            deferred: HashMap::new(),
            fixture_targets: HashMap::new(),
            next_fixture_key: 0,
            locked: false,
            model: 0,
            timeline: None,
            talk_tapped: false,
            home_mounted: true,
            site_epoch: None,
            awaiting_site: false,
            arriving: false,
            force_update: false,
            player: None,
            last_input: f32::NEG_INFINITY,
            reported: None,
            surveyed: false,
            sensor_wait: None,
            change_tapped: false,
            house_touching: None,
            house_wait: None,
        }
    }
}

impl ActionButtonState {
    /// The home screen's new presenter: a MysekaiActionButtonsModel whose
    /// constructor writes only its two lists, so the stack starts empty and
    /// unlocked (only SetLockActionButton writes BlockStackChange); its
    /// Initialize runs ForceUpdate.
    fn rebuild(&mut self) {
        self.stack.clear();
        self.fixture_targets.clear();
        self.locked = false;
        self.model += 1;
        self.force_update = true;
    }

    /// 当前栈首（决定屏幕上显示哪个按钮）。
    pub(crate) fn current(&self) -> Option<(ButtonType, TargetId)> {
        self.stack.first()
    }

    /// SetChangeButton: shown unless the model hides it (the lock), with
    /// more than one entry stacked.
    pub(crate) fn change_shown(&self) -> bool {
        !self.locked && self.stack.len() > 1
    }

    /// An entry of this button type is stacked, head or not.
    fn stacked(&self, button: ButtonType) -> bool {
        self.stack.buttons().any(|stacked| stacked == button)
    }

    fn fixture_target(&self, key: i32) -> Option<FixtureTarget> {
        self.fixture_targets.get(&key).cloned()
    }

    /// Push when absent. One instance keeps one handle while it is stacked.
    fn push_fixture(&mut self, button: ButtonType, target: FixtureTarget) {
        let key = match self
            .fixture_targets
            .iter()
            .find(|(_, stacked)| **stacked == target)
        {
            Some((&key, _)) => key,
            None => {
                let key = self.next_fixture_key;
                self.next_fixture_key = key
                    .checked_add(1)
                    .expect("fixture button handle space exhausted");
                self.fixture_targets.insert(key, target);
                key
            }
        };
        self.stack.push(button, TargetId::Fixture(key), false);
    }

    /// RemoveShowButtonStack for one instance.
    fn remove_fixture(&mut self, entity: Entity, uid: &str) {
        let keys: Vec<i32> = self
            .fixture_targets
            .iter()
            .filter(|(_, target)| target.entity == entity && target.uid == uid)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.stack.remove(TargetId::Fixture(key));
            self.fixture_targets.remove(&key);
        }
    }
}

/// A fixture with a button, as the scan saw it this frame.
struct Candidate<'a> {
    entity: Entity,
    button: ButtonType,
    touching: bool,
    identity: Option<&'a FixtureActivityIdentity>,
    world: &'a GlobalTransform,
    /// The house's inside-door point, for a HouseEntry candidate.
    house_entry: Option<Vec3>,
}

/// A registered NPC, as the scan saw it this frame.
struct NpcCandidate {
    entity: Entity,
    unit: u32,
    touching: bool,
}

/// The room door's sensor, as the scan saw it this frame.
struct SensorCandidate {
    /// The attach point (`gimmick_door`).
    entity: Entity,
    /// IsInSideCircle: the player within the sensor's own radius, in 3D.
    touching: bool,
    /// The room's door action point, which the go-home button's
    /// availability reaches for.
    door: Vec3,
}

/// One object of this frame's scan, in the order its enter edge runs.
#[derive(Clone, Copy)]
enum Slot {
    Fixture(usize),
    Sensor,
    Npc(usize),
}

fn kind_of(button: ButtonType) -> FixtureKind {
    if button == ButtonType::TimelineFixture {
        FixtureKind::Timeline
    } else {
        FixtureKind::Gimmick
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
        facts.package_by_glb.insert(glb.to_owned(), name.to_owned());
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
        let action_value =
            row.get("playerActionTypeValue")
                .and_then(|v| v.as_i64())
                .expect("[action_button] 主表行缺 playerActionTypeValue") as i32;
        let fixture_type = FixtureType::from_i32(type_value).unwrap_or_else(|| {
            panic!("[action_button] 主表行 {bundle} 的家具类别越界：{type_value}")
        });
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
    let (Some(mut art), Some(layouts)) = (art, layouts) else {
        return;
    };
    if art.skin.is_none() {
        art.skin =
            ActionButtonSkin::from_layouts(&layouts, &server, SKIN_LAYOUT, BUTTON_NODE, ICON_NODE);
    }
    if art.go_home.is_none() {
        art.go_home = ActionButtonSkin::from_layouts(
            &layouts,
            &server,
            GO_HOME_LAYOUT,
            GO_HOME_NODE,
            GO_HOME_ICON_NODE,
        );
    }
    // The change-target button is inactive in both prefabs until
    // SetChangeButton shows it; its rect is read as shown.
    let fields = &mut *art;
    for skin in [fields.skin.as_mut(), fields.go_home.as_mut()].into_iter().flatten() {
        skin.geometry.set_visible(CHANGE_NODE, true);
    }
    let (Some(skin), Some(go_home)) = (art.skin.as_ref(), art.go_home.as_ref()) else {
        return;
    };
    for image in [Some(&skin.background), Some(&go_home.background), go_home.icon.as_ref()]
        .into_iter()
        .flatten()
    {
        match server.load_state(image) {
            LoadState::Loaded => {}
            LoadState::Failed(err) => {
                panic!("[action_button] source button image failed: {err:?}")
            }
            _ => return,
        }
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
    let background = commands
        .spawn((
            ActionButtonBackground,
            Sprite {
                image: skin.background.clone(),
                color: skin.background_color,
                ..default()
            },
            Transform::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
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
    let change_icon = ButtonType::ChangeActionTarget
        .icon_file_name()
        .and_then(|name| art.icons.get(name))
        .cloned()
        .unwrap_or_default();
    commands.spawn((
        ActionButtonChange,
        Sprite {
            image: change_icon,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.6),
        Visibility::Hidden,
        RenderLayers::layer(BALLOON_LAYER),
    ));
    info!("[action_button] source button background and icon installed under one hidden root");
}

/// Update：每帧重建玩家的碰撞盒、与场上目标求交、把进出边沿写进栈。
///
/// 源侧这一步分在两处：碰撞管理器算进出、屏幕层的回调把它转成压栈与
/// 出栈。本仓没有那两层，于是在这里一次算完。Fixtures and NPCs both
/// follow the colliding list's edges: admission runs once when an object
/// joins, removal when it leaves, and nothing is re-tested in between.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    mut state: ResMut<ActionButtonState>,
    eligibility: crate::interaction::InteractionEligibility,
    inputs: AdmissionInputs,
    facts: Res<FixtureFacts>,
    mut saved: MessageReader<LayoutSaved>,
    players: Query<(Entity, &Transform, Option<&ChildOf>), With<PlayerControlled>>,
    parents: Query<&GlobalTransform>,
    scenes_ready: Option<Res<crate::site::SiteScenesReady>>,
    npcs: Query<(Entity, &Transform, &CharacterUnitId), Without<PlayerControlled>>,
    fixtures: Query<
        (
            Entity,
            &Transform,
            &FixtureSource,
            &GlobalTransform,
            Option<&FixtureActivityIdentity>,
            Option<&ActionButtonFixture>,
            Option<&crate::site_move::door::HouseEntryPoint>,
        ),
        With<FixtureRoot>,
    >,
    server: Res<AssetServer>,
    site_move: Option<Res<crate::site_move::SiteMoveActive>>,
    homes: Option<Res<crate::entry::house::HomeFixtures>>,
    room_door: Option<Res<crate::site_move::room_door::RoomDoor>>,
) {
    let state = &mut *state;
    // A saved layout re-fires entry once the scan next runs.
    if saved.read().count() > 0 {
        state.force_update = true;
    }
    // A site move: SiteMoveGameState.OnEnter changes to the site-move screen
    // and GameState SiteMove stops the collision scan. It lasts from the old
    // site's teardown until the player and NPCs stand on the new site, whose
    // controller then changes back to the home screen (a new presenter).
    // npc::reseed and player::reseed place them for a new generation after
    // this system, in the same frame, so the move ends on the next scan.
    if scenes_ready.is_none() {
        state.awaiting_site = true;
    }
    let epoch = inputs.site_epoch();
    if state.site_epoch != epoch {
        state.site_epoch = epoch;
        state.awaiting_site = false;
        state.arriving = true;
    } else if state.arriving {
        state.arriving = false;
    }
    let site_moving = scenes_ready.is_none()
        || state.awaiting_site
        || state.arriving
        || site_move.is_some();
    // ScreenLayerMysekaiHome mounts again: OnBoot builds a new presenter,
    // and OnScreenStart's Initialize runs ForceUpdate.
    let mounted = eligibility.home_screen_current() && !site_moving;
    if mounted && !state.home_mounted {
        state.rebuild();
        info!("[action_button] home screen current again; stack restarts from ForceUpdate");
    }
    state.home_mounted = mounted;
    let Ok((player_entity, local, parent)) = players.single() else {
        state.player = None;
        return;
    };
    if !facts.parsed {
        return;
    }
    // The scan and admission read the player's world pose: a player timeline
    // parents the actor to its seat.
    let player = match parent {
        None => *local,
        Some(parent) => match parents.get(parent.parent()) {
            Ok(parent) => parent.mul_transform(*local).compute_transform(),
            Err(_) => return,
        },
    };
    state.player_box = crate::interaction::player_box(&player);
    let player_box = state.player_box;
    let position = player.translation;
    state.player = Some((player_entity, position));

    // 场上家具的一次性对账：家具铺完之后报一次「这个站点上哪几件有
    // 交互按钮、在哪」。没有这一条，「走了半天没看到家具按钮」分不出
    // 是这条链没接上、还是这个站点上本来就没有可交互家具——两者在
    // 屏幕上长得一样。数是每次运行现算的，不是写死的清单。
    let survey = !state.surveyed && !fixtures.is_empty();
    let mut placed = 0usize;
    let mut joined = 0usize;
    let mut with_button: Vec<(ButtonType, [f32; 3], String)> = Vec::new();

    let mut frame: Vec<Candidate> = Vec::new();
    for (entity, transform, source, world, identity, cached, house) in &fixtures {
        placed += 1;
        let resolved;
        let facts_of = match cached {
            Some(cached) => cached,
            None => {
                let Some(path) = server.get_path(&source.0) else {
                    continue;
                };
                let glb = path
                    .path()
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_owned();
                let row = facts.row_for_glb(&glb).copied();
                resolved = ActionButtonFixture {
                    button: row.as_ref().and_then(fixture_button),
                    row,
                    glb,
                };
                commands.entity(entity).try_insert(resolved.clone());
                &resolved
            }
        };
        let Some(row) = facts_of.row else {
            continue;
        };
        joined += 1;
        // A home system fixture's view carries PlayerActionType Home: the
        // house-entry button, once its inside-door point is known.
        let house_entry = house
            .and_then(|point| parents.get(point.0).ok())
            .map(GlobalTransform::translation);
        let home = || {
            let package = &identity?.model_package;
            let is_home = homes.as_deref()?.is_home(package).ok()?;
            (is_home && row.fixture_type.can_action() && house_entry.is_some())
                .then(|| ButtonType::from_action_type(PlayerActionType::Home, false))
        };
        let Some(button) = facts_of.button.or_else(home) else {
            continue;
        };
        if survey {
            with_button.push((button, transform.translation.to_array(), facts_of.glb.clone()));
        }
        let target_box = fixture_box(
            transform.translation.to_array(),
            row.grid_width,
            row.grid_depth,
        );
        frame.push(Candidate {
            entity,
            button,
            touching: player_box.collides(&target_box),
            identity,
            world,
            house_entry,
        });
    }

    if let Some(house) = frame
        .iter()
        .find(|candidate| candidate.button == ButtonType::HouseEntry)
    {
        if state.house_touching != Some(house.touching) {
            state.house_touching = Some(house.touching);
            info!(
                "[action_button] HouseEntry candidate {:?}: player box overlaps the house footprint = {}",
                house.entity, house.touching
            );
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

    // An NPC is a collision object once its avatar is registered.
    let npc_frame: Vec<NpcCandidate> = npcs
        .iter()
        .filter(|(entity, _, _)| eligibility.talk_registered(*entity))
        .map(|(entity, transform, unit)| NpcCandidate {
            entity,
            unit: unit.0,
            touching: player_box.collides(&character_box(transform.translation.to_array())),
        })
        .collect();

    // The room site's enter registers the door sensor and its exit removes
    // it: it is a collision object while its room is the active site.
    let sensor_frame: Option<SensorCandidate> =
        crate::site_move::room_door::sensor(room_door.as_deref(), inputs.site_epoch()).and_then(
            |(sensor, inside)| {
                let at = parents.get(sensor).ok()?.translation();
                Some(SensorCandidate {
                    entity: sensor,
                    touching: inside_circle(
                        position.to_array(),
                        at.to_array(),
                        crate::site_move::room_door::DOOR_SENSOR_RADIUS,
                    ),
                    door: parents.get(inside).ok()?.translation(),
                })
            },
        );
    let is_sensor = |entity: Entity| sensor_frame.as_ref().is_some_and(|s| s.entity == entity);

    // An object that left the scene, whose identity changed, or an NPC no
    // longer registered, leaves the colliding list and the stack.
    let index: HashMap<Entity, usize> = frame
        .iter()
        .enumerate()
        .map(|(i, candidate)| (candidate.entity, i))
        .collect();
    let npc_index: HashMap<Entity, usize> = npc_frame
        .iter()
        .enumerate()
        .map(|(i, npc)| (npc.entity, i))
        .collect();
    let live = |entity: Entity, uid: &str| {
        index
            .get(&entity)
            .and_then(|&i| frame[i].identity)
            .is_some_and(|identity| identity.uid == uid)
    };
    state.colliding.retain(|collider| match collider {
        Collider::Fixture(entity, uid) => live(*entity, uid),
        Collider::Character(entity, unit) => npc_index
            .get(entity)
            .is_some_and(|&i| npc_frame[i].unit == *unit),
        Collider::Sensor(entity) => is_sensor(*entity),
    });
    state
        .deferred
        .retain(|entity, _| index.contains_key(entity) || is_sensor(*entity));
    let stale: Vec<i32> = state
        .fixture_targets
        .iter()
        .filter(|(_, target)| !live(target.entity, &target.uid))
        .map(|(key, _)| *key)
        .collect();
    for key in stale {
        state.stack.remove(TargetId::Fixture(key));
        state.fixture_targets.remove(&key);
    }
    if sensor_frame.is_none() {
        state.stack.remove(TargetId::Sensor);
    }
    // For an NPC, RemoveCollisionObject runs RemoveShowButtonStack, which
    // returns at once while the stack is locked.
    if !state.locked {
        state.stack.retain_targets(&|target| match target {
            TargetId::Character(unit) => npc_frame.iter().any(|npc| npc.unit == unit),
            TargetId::Fixture(_) | TargetId::Sensor => true,
        });
    }

    // TimelineFixtureFinishAsync: once the player leaves UseTimelineFixture,
    // the presenter that started the timeline runs RemoveNotCollisionObject
    // and SetLockActionButton(false) on its model, then shows the view again.
    // The view outlives a home rebuild; that presenter's model does not, and
    // the new model was never locked.
    if let Some(owner) = state.timeline {
        if eligibility.player_action() != PlayerActionState::UseTimelineFixture {
            if owner == state.model {
                remove_not_colliding(
                    state,
                    &inputs,
                    &eligibility,
                    player_entity,
                    position,
                    &frame,
                    &index,
                    sensor_frame.as_ref(),
                );
                state.locked = false;
                info!("[action_button] player timeline ended; stack pruned and unlocked");
            }
            state.timeline = None;
        }
    }
    // OnChangeActionTarget, after its input interval passed in the click:
    // RemoveNotCollisionObject, then with two or more entries the head
    // moves to the back and the stack is shown again.
    if std::mem::take(&mut state.change_tapped) {
        remove_not_colliding(
            state,
            &inputs,
            &eligibility,
            player_entity,
            position,
            &frame,
            &index,
            sensor_frame.as_ref(),
        );
        if state.stack.rotate() {
            info!(
                "[action_button] change-target: head moves to the back; head now {:?} (stack depth {})",
                state.stack.first(),
                state.stack.len()
            );
        }
    }
    // The Talk button's OnClickPlayerTalkAction awaits the talk action and
    // then always runs RemoveNotCollisionObject. A talk that played pushed
    // MysekaiTalk, so that call reaches the model the home rebuild replaces;
    // a refused one reaches the current model now.
    if std::mem::take(&mut state.talk_tapped) && !eligibility.player_in_talk() {
        remove_not_colliding(
            state,
            &inputs,
            &eligibility,
            player_entity,
            position,
            &frame,
            &index,
            sensor_frame.as_ref(),
        );
    }

    // ObjectCollisionManager.IsCanUpdate: no edges at all while it is false.
    if eligibility.collision_updates() && !site_moving {
        // ForceUpdate returns the colliding list to the registered objects,
        // behind the others, and scans: the newly overlapping enter first,
        // then the previous list in its order. It produces no exit edge.
        let previous = if state.force_update {
            state.force_update = false;
            state.deferred.clear();
            Some(state.colliding.take())
        } else {
            None
        };
        let fresh = |entity: Entity| {
            previous
                .as_ref()
                .is_none_or(|previous| !previous.iter().any(|collider| collider.entity() == entity))
        };
        // Moly has no registration order across fixtures, the door sensor
        // and NPCs; the scan takes fixtures, then the sensor, then NPCs, each
        // in query order.
        let mut order: Vec<Slot> = frame
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.touching && fresh(candidate.entity))
            .map(|(i, _)| Slot::Fixture(i))
            .chain(
                sensor_frame
                    .as_ref()
                    .filter(|sensor| sensor.touching && fresh(sensor.entity))
                    .map(|_| Slot::Sensor),
            )
            .chain(
                npc_frame
                    .iter()
                    .enumerate()
                    .filter(|(_, npc)| npc.touching && fresh(npc.entity))
                    .map(|(i, _)| Slot::Npc(i)),
            )
            .collect();
        for collider in previous.iter().flatten() {
            let slot = match collider {
                Collider::Fixture(entity, _) => index
                    .get(entity)
                    .filter(|&&i| frame[i].touching)
                    .map(|&i| Slot::Fixture(i)),
                Collider::Character(entity, _) => npc_index
                    .get(entity)
                    .filter(|&&i| npc_frame[i].touching)
                    .map(|&i| Slot::Npc(i)),
                Collider::Sensor(entity) => sensor_frame
                    .as_ref()
                    .filter(|sensor| sensor.entity == *entity && sensor.touching)
                    .map(|_| Slot::Sensor),
            };
            order.extend(slot);
        }
        // CheckEnterCollide, then CheckExitCollide.
        let revision = inputs.revision();
        for slot in order {
            match slot {
                Slot::Fixture(i) => {
                    fixture_enter(state, &inputs, player_entity, position, &frame[i], revision)
                }
                Slot::Sensor => {
                    if let Some(sensor) = sensor_frame.as_ref() {
                        sensor_enter(state, &inputs, player_entity, position, sensor, revision);
                    }
                }
                Slot::Npc(i) => npc_enter(state, &eligibility, &npc_frame[i]),
            }
        }
        for candidate in frame.iter().filter(|candidate| !candidate.touching) {
            fixture_exit(state, candidate.entity);
        }
        if let Some(sensor) = sensor_frame.as_ref().filter(|sensor| !sensor.touching) {
            sensor_exit(state, sensor.entity);
        }
        for npc in npc_frame.iter().filter(|npc| !npc.touching) {
            npc_exit(state, npc.entity, npc.unit);
        }
    }

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

/// CheckExitCollide for a fixture that no longer overlaps.
fn fixture_exit(state: &mut ActionButtonState, entity: Entity) {
    state.deferred.remove(&entity);
    // RemoveShowButtonStack returns at once while the stack is locked.
    if let Some(Collider::Fixture(_, uid)) = state.colliding.leave(entity) {
        if !state.locked {
            state.remove_fixture(entity, &uid);
        }
    }
}

/// CheckEnterCollide for an overlapping fixture.
fn fixture_enter(
    state: &mut ActionButtonState,
    inputs: &AdmissionInputs,
    player: Entity,
    position: Vec3,
    candidate: &Candidate,
    revision: Option<InputRevision>,
) {
    let entity = candidate.entity;
    if state.colliding.contains(entity) {
        return;
    }
    let Some(identity) = candidate.identity else {
        return;
    };
    if revision.is_some() && state.deferred.get(&entity) == revision.as_ref() {
        return;
    }
    let target = FixtureTarget {
        entity,
        uid: identity.uid.clone(),
    };
    let joined = Collider::Fixture(entity, target.uid.clone());
    // TryAddObjToCollideList precedes AddShowButtonStack, whose lock check
    // then drops the entry.
    if state.locked {
        state.colliding.join(joined);
        return;
    }
    let probe = Probe {
        player,
        position,
        target: &target,
        identity,
        world: candidate.world,
        kind: kind_of(candidate.button),
    };
    let entered = match candidate.house_entry {
        // IsActionButtonTypeAvailable of the house entry.
        Some(door) if candidate.button == ButtonType::HouseEntry => {
            inputs.house_entry(player, position, door).map(|shown| {
                if shown {
                    Enter::Push
                } else {
                    Enter::Skip("CanShowHouseEntryButton is false")
                }
            })
        }
        _ => inputs.enter(&probe),
    };
    match entered {
        Ok(Enter::Push) => {
            state.deferred.remove(&entity);
            state.colliding.join(joined);
            state.push_fixture(candidate.button, target);
        }
        Ok(Enter::Remove) => {
            state.deferred.remove(&entity);
            state.colliding.join(joined);
            state.remove_fixture(entity, &target.uid);
            info!(
                "[action_button] {:?} {} entered; IsCanActionFixture is false, not stacked",
                candidate.button, target.uid
            );
        }
        Ok(Enter::Skip(reason)) => {
            state.deferred.remove(&entity);
            state.colliding.join(joined);
            info!(
                "[action_button] {:?} {} entered; {reason}, not stacked",
                candidate.button, target.uid
            );
        }
        Err(deferred) => match deferred.retry {
            Retry::NextFrame => {
                if candidate.button == ButtonType::HouseEntry
                    && state.house_wait.as_deref() != Some(deferred.reason.as_str())
                {
                    info!(
                        "[action_button] HouseEntry {} entry waits for {}",
                        target.uid, deferred.reason
                    );
                    state.house_wait = Some(deferred.reason);
                }
            }
            Retry::NewInputs(at) => {
                if state.deferred.insert(entity, at) != Some(at) {
                    warn!(
                        "[action_button] {:?} {} entry undecided with this navigation snapshot: {}",
                        candidate.button, target.uid, deferred.reason
                    );
                }
            }
            Retry::Never => {
                state.deferred.remove(&entity);
                state.colliding.join(joined);
                warn!(
                    "[action_button] {:?} {} entered without a button; attachment data cannot answer: {}",
                    candidate.button, target.uid, deferred.reason
                );
            }
        },
    }
}

/// CheckEnterCollide for the door sensor: TryAddObjToCollideList, then
/// AddShowButtonStack. The sensor's action type Door maps to the go-home
/// button, whose IsActionButtonTypeAvailable is CanShowHouseEntryButton on
/// the room's door action point. A sensor is neither a fixture view nor an
/// NPC, so IsCanActionFixture and IsCanActionNPC pass it; the entry is
/// appended, not a priority one.
fn sensor_enter(
    state: &mut ActionButtonState,
    inputs: &AdmissionInputs,
    player: Entity,
    position: Vec3,
    sensor: &SensorCandidate,
    revision: Option<InputRevision>,
) {
    let entity = sensor.entity;
    if state.colliding.contains(entity) {
        return;
    }
    if revision.is_some() && state.deferred.get(&entity) == revision.as_ref() {
        return;
    }
    let joined = Collider::Sensor(entity);
    if state.locked {
        state.colliding.join(joined);
        return;
    }
    let button = ButtonType::from_action_type(PlayerActionType::Door, false);
    match inputs.house_entry(player, position, sensor.door) {
        Ok(shown) => {
            state.deferred.remove(&entity);
            state.colliding.join(joined);
            if shown {
                state.stack.push(button, TargetId::Sensor, false);
                info!(
                    "[action_button] {button:?} door sensor entered; stacked (stack depth {})",
                    state.stack.len()
                );
            } else {
                info!("[action_button] {button:?} door sensor entered; CanShowHouseEntryButton is false, not stacked");
            }
        }
        Err(deferred) => match deferred.retry {
            Retry::NextFrame => {
                if state.sensor_wait.as_deref() != Some(deferred.reason.as_str()) {
                    info!(
                        "[action_button] {button:?} door sensor entry waits for {}",
                        deferred.reason
                    );
                    state.sensor_wait = Some(deferred.reason);
                }
            }
            Retry::NewInputs(at) => {
                if state.deferred.insert(entity, at) != Some(at) {
                    warn!(
                        "[action_button] {button:?} door sensor entry undecided with this navigation snapshot: {}",
                        deferred.reason
                    );
                }
            }
            Retry::Never => {
                state.deferred.remove(&entity);
                state.colliding.join(joined);
                warn!(
                    "[action_button] {button:?} door sensor entered without a button: {}",
                    deferred.reason
                );
            }
        },
    }
}

/// CheckExitCollide for the door sensor: RemoveShowButtonStack, which
/// returns at once while the stack is locked.
fn sensor_exit(state: &mut ActionButtonState, entity: Entity) {
    state.deferred.remove(&entity);
    if state.colliding.leave(entity).is_some() && !state.locked {
        state.stack.remove(TargetId::Sensor);
    }
}

/// CheckEnterCollide for an overlapping NPC: TryAddObjToCollideList, then
/// AddShowButtonStack for its Talk entry.
fn npc_enter(
    state: &mut ActionButtonState,
    eligibility: &crate::interaction::InteractionEligibility,
    npc: &NpcCandidate,
) {
    if state.colliding.contains(npc.entity) {
        return;
    }
    // Without an active site CheckTargetSite has nothing to compare with;
    // the edge waits for one.
    let Some(admitted) = eligibility.talk_admitted(npc.entity) else {
        return;
    };
    state
        .colliding
        .join(Collider::Character(npc.entity, npc.unit));
    if state.locked || !admitted {
        return;
    }
    state
        .stack
        .push(ButtonType::Talk, TargetId::Character(npc.unit), false);
}

/// CheckExitCollide for an NPC: RemoveShowButtonStack, which returns at
/// once while the stack is locked.
fn npc_exit(state: &mut ActionButtonState, entity: Entity, unit: u32) {
    if state.colliding.leave(entity).is_some() && !state.locked {
        state.stack.remove(TargetId::Character(unit));
    }
}

/// RemoveNotCollisionObject: an entry whose object left the colliding list
/// is removed; one still inside is removed when IsActionButtonTypeAvailable
/// is false. It has no lock check.
fn remove_not_colliding(
    state: &mut ActionButtonState,
    inputs: &AdmissionInputs,
    eligibility: &crate::interaction::InteractionEligibility,
    player: Entity,
    position: Vec3,
    frame: &[Candidate],
    index: &HashMap<Entity, usize>,
    sensor: Option<&SensorCandidate>,
) {
    let mut removed = Vec::new();
    for (&key, target) in &state.fixture_targets {
        if state.colliding.fixture_uid(target.entity) != Some(target.uid.as_str()) {
            removed.push(key);
            continue;
        }
        let Some(candidate) = index.get(&target.entity).map(|&i| &frame[i]) else {
            removed.push(key);
            continue;
        };
        let Some(identity) = candidate.identity else {
            continue;
        };
        let probe = Probe {
            player,
            position,
            target,
            identity,
            world: candidate.world,
            kind: kind_of(candidate.button),
        };
        let available = match candidate.house_entry {
            Some(door) if candidate.button == ButtonType::HouseEntry => {
                inputs.house_entry(player, position, door).map(|shown| {
                    if shown {
                        Availability::Available
                    } else {
                        Availability::Unavailable
                    }
                })
            }
            _ => inputs.availability(&probe),
        };
        match available {
            Ok(Availability::Available) => {}
            Ok(Availability::Unavailable) => removed.push(key),
            Ok(Availability::Raises) => warn!(
                "[action_button] {} is stacked although the source raises on its availability; kept",
                target.uid
            ),
            Err(deferred) => info!(
                "[action_button] {} availability undecided while pruning ({}); kept",
                target.uid, deferred.reason
            ),
        }
    }
    for key in removed {
        state.stack.remove(TargetId::Fixture(key));
        state.fixture_targets.remove(&key);
    }
    // The door sensor's entry: the sensor left the colliding list, or
    // CanShowHouseEntryButton is false.
    let keep_sensor = sensor
        .filter(|sensor| state.colliding.contains(sensor.entity))
        .is_some_and(|sensor| match inputs.house_entry(player, position, sensor.door) {
            Ok(shown) => shown,
            Err(deferred) => {
                info!(
                    "[action_button] door sensor availability undecided while pruning ({}); kept",
                    deferred.reason
                );
                true
            }
        });
    // A Talk entry: its NPC left the colliding list, or CheckTargetSite or
    // CanShowTalkActionButton is false. Without an active site it is kept.
    let colliding = &state.colliding;
    state.stack.retain_targets(&|target| match target {
        TargetId::Fixture(_) => true,
        TargetId::Sensor => keep_sensor,
        TargetId::Character(unit) => colliding
            .character(unit)
            .is_some_and(|entity| eligibility.talk_available(entity) != Some(false)),
    });
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
        // The action point and player gates (IsActionButtonTypeAvailable,
        // IsCanActionFixture) run on the collision edge, in admission.rs.
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
    mut parts: Query<
        (&mut Transform, &mut Sprite, Option<&ActionButtonIcon>),
        Or<(With<ActionButtonIcon>, With<ActionButtonBackground>)>,
    >,
) {
    let Ok((_, window)) = screen.windows.single() else {
        return;
    };
    let head = state.current();
    let scale = screen.scale(window);
    let rects = head.and_then(|(button, _)| screen.rects(window, button));
    for (mut visibility, mut transform) in &mut roots {
        // TimelineFixtureProcess hides the action buttons view until the
        // timeline's finish shows it again. They are part of the home
        // screen, which is not shown while another screen is current.
        *visibility = if head.is_some()
            && rects.is_some()
            && state.timeline.is_none()
            && state.home_mounted
        {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(scale) = scale {
            transform.scale = Vec3::new(scale, scale, 1.0);
        }
    }
    let Some((button, _)) = head else { return };
    let (Some(art), Some((background_rect, icon_rect))) = (screen.art.as_deref(), rects) else {
        return;
    };
    let Some(skin) = art.skin_for(button) else {
        return;
    };
    for (mut transform, mut sprite, icon) in &mut parts {
        let rect = if icon.is_some() {
            &icon_rect
        } else {
            &background_rect
        };
        let (local_scale, rotation, _) = rect.world.to_scale_rotation_translation();
        *transform = Transform {
            translation: rect.center().extend(if icon.is_some() { 0.5 } else { 0.0 }),
            rotation,
            scale: local_scale,
        };
        sprite.custom_size = Some(rect.size);
        let image = if icon.is_some() {
            skin.icon.as_ref().or_else(|| {
                button
                    .icon_file_name()
                    .and_then(|name| art.icons.get(name))
            })
        } else {
            Some(&skin.background)
        };
        if let Some(handle) = image {
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
        }
        sprite.color = if icon.is_some() {
            skin.icon_color
        } else {
            skin.background_color
        };
    }
}

/// The change-target button: visible with the buttons view while
/// SetChangeButton shows it, laid out in the head view's layout.
pub(crate) fn place_change_ui(
    state: Res<ActionButtonState>,
    screen: ActionButtonScreen,
    roots: Query<&Visibility, (With<ActionButtonRoot>, Without<ActionButtonChange>)>,
    mut change: Query<(&mut Visibility, &mut Transform, &mut Sprite), With<ActionButtonChange>>,
) {
    let Ok((_, window)) = screen.windows.single() else {
        return;
    };
    let Ok((mut visibility, mut transform, mut sprite)) = change.single_mut() else {
        return;
    };
    let view_shown = roots
        .iter()
        .any(|visibility| *visibility != Visibility::Hidden);
    let rects = state
        .current()
        .and_then(|(head, _)| screen.change_rects(window, head));
    let (Some((_, icon)), Some(scale), true, true) =
        (rects, screen.scale(window), view_shown, state.change_shown())
    else {
        *visibility = Visibility::Hidden;
        return;
    };
    let (local_scale, rotation, _) = icon.world.to_scale_rotation_translation();
    *transform = Transform {
        translation: (icon.center() * scale).extend(0.6),
        rotation,
        scale: local_scale * Vec3::new(scale, scale, 1.0),
    };
    sprite.custom_size = Some(icon.size);
    *visibility = Visibility::Visible;
}

/// Update（拾取之前）：点按落在按钮上就分派动作并吃掉这一帧的点按。
///
/// 节流照源：两次输入之间至少隔 [`ACTION_BUTTON_INPUT_INTERVAL`] 秒。
///
/// OnGimmickFixture reads only its input interval and the multiplayer dialog;
/// the player's own state is a later, separate step of the switch. The other
/// buttons keep the full interaction gate. OnTimelineFixture asks
/// IsCanActionFixture for the head again before it locks the stack.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut commands: Commands,
    mut state: ResMut<ActionButtonState>,
    mut consumed: ResMut<ActionTapConsumed>,
    eligibility: crate::interaction::InteractionEligibility,
    inputs: AdmissionInputs,
    views: Query<(&FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>,
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
        // A button takes every pointer click: the second of two quick taps
        // is a double tap to the field recognizer, still a click here.
        .filter(|event| {
            matches!(event.kind, GestureKind::Tap | GestureKind::DoubleTap)
                && event.state == GestureState::End
        })
        .map(|event| event.position)
        .collect();
    // Open gap for Talk: available() is closed while any conversation plays,
    // and the talk dispatcher refuses a request for the same reason. The
    // source has no such global gate: OnCharacterTalk hands the head's NPC to
    // MysekaiPlayerTalkAction.OnClickPlayerTalkAction, which routes on that
    // NPC's own state, and sends an NPC taking part in a conversation between
    // characters to PlaySomeCharacterTalkFixture, where the player joins it.
    // Moly does not join conversations, so while NPCs talk to each other the
    // Talk button stays visible but inert: the tap is not taken and no talk
    // starts.
    let open = match state.current() {
        Some((ButtonType::GimmickFixture, _)) => eligibility.field_input_open(),
        _ => eligibility.available(),
    };
    let shown = roots
        .iter()
        .any(|visibility| *visibility != Visibility::Hidden);
    if let (Some(&tap), Some((head, _))) = (taps.first(), state.current()) {
        let on_change = state.change_shown() && screen.change_hit(tap, window, head);
        if head != ButtonType::Talk
            && !on_change
            && (!open || !shown || !screen.hit(tap, window, head))
        {
            info!(
                "[action_button] tap at ({:.0},{:.0}) not taken for {head:?}: field open {open}, button shown {shown}, on the button {}",
                tap.x,
                tap.y,
                screen.hit(tap, window, head)
            );
        }
    }
    if taps.is_empty() || !open || !shown {
        return;
    }
    let Some((button, target)) = state.current() else {
        return;
    };
    let fixture_target = match target {
        TargetId::Fixture(key) => state.fixture_target(key),
        TargetId::Character(_) | TargetId::Sensor => None,
    };
    for position in taps {
        // The change-target button: IsCantActionButtonInput (the shared
        // input interval), then OnChangeActionTarget on the next scan.
        if state.change_shown() && screen.change_hit(position, window, button) {
            let now = time.elapsed_secs();
            consumed.0 = true;
            if now - state.last_input < ACTION_BUTTON_INPUT_INTERVAL {
                continue;
            }
            state.last_input = now;
            state.change_tapped = true;
            info!("[action_button] change-target button pressed (stack depth {})", state.stack.len());
            continue;
        }
        if !screen.hit(position, window, button) {
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
        if button != ButtonType::Talk {
            state.last_input = now;
        }
        consumed.0 = true;
        // Every Talk tap from here reaches the talk action, refused or not;
        // its RemoveNotCollisionObject runs on the next scan.
        if button == ButtonType::Talk {
            state.talk_tapped = true;
        }
        if let TargetId::Character(unit) = target {
            if !eligibility.for_unit(unit) {
                continue;
            }
        }
        if button == ButtonType::TimelineFixture {
            if let Some(target) = fixture_target.as_ref() {
                if let Err(reason) = timeline_ready(state.player, &inputs, &views, target) {
                    info!("[action_button] TimelineFixture tap on {} ignored: {reason}", target.uid);
                    continue;
                }
            }
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
        if button == ButtonType::TimelineFixture && fixture_target.is_some() {
            // TimelineFixtureProcess hides the view and runs
            // SetLockActionButton(true); the timeline's finish undoes both.
            state.locked = true;
            state.timeline = Some(state.model);
        }
    }
}

/// OnTimelineFixture's head check: the fixture and its view exist and
/// IsCanActionFixture holds now.
fn timeline_ready(
    player: Option<(Entity, Vec3)>,
    inputs: &AdmissionInputs,
    views: &Query<(&FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>,
    target: &FixtureTarget,
) -> Result<(), String> {
    let (player, position) = player.ok_or("player pose is not ready")?;
    let (identity, world) = views
        .get(target.entity)
        .map_err(|_| "fixture left the scene")?;
    if identity.uid != target.uid {
        return Err("fixture identity changed".into());
    }
    let probe = Probe {
        player,
        position,
        target,
        identity,
        world,
        kind: FixtureKind::Timeline,
    };
    match inputs.can_action(&probe) {
        Ok(true) => Ok(()),
        Ok(false) => Err("IsCanActionFixture is false".into()),
        Err(deferred) => Err(format!("IsCanActionFixture undecided: {}", deferred.reason)),
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
            talk_requests.write(PlayerTalkRequest {
                entity,
                unit,
                exact: None,
                target_fixture: None,
            });
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
            commands.insert_resource(crate::site::SiteChangeRequest("first_floor".to_owned()));
            info!(
                "[action_button] 进屋按钮按下 → 站点切换请求 first_floor（与小地图点站同一条路）"
            );
        }
        (ButtonType::GoHomeSite, _) => {
            commands.insert_resource(crate::site::SiteChangeRequest("home_site".to_owned()));
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
            layer_commands.write(LayerCommand::Push(LayerId::MysekaiAvatarCostumeSetting));
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
    mut touches: InjectedTouches,
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
    let write = |touches: &mut InjectedTouches, phase: TouchPhase, position: Vec2| {
        touches.touches.write(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: FINGER,
        });
    };
    // The joystick reads the touch messages; the gesture layer reads the
    // window's event stream, so the tap finger goes there.
    let write_tap = |touches: &mut InjectedTouches, phase: TouchPhase, position: Vec2| {
        touches
            .window_events
            .write(bevy::window::WindowEvent::TouchInput(TouchInput {
                phase,
                position,
                window: window_entity,
                force: None,
                id: TAP_FINGER,
            }));
    };
    // 按钮的屏位（顶原点）：与摆件同一式——覆盖相机世界心翻回屏坐标。
    // With an empty stack the tap arm has nothing to press; the walk still
    // needs a position, the shared button's.
    let shown = button_state
        .current()
        .map_or(ButtonType::Talk, |(button, _)| button);
    let Some(button_pos) = screen.button_position(window, shown) else {
        return;
    };
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
    let mut anchors: Vec<(i32, Vec2)> = fixtures
        .iter()
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
        let joy = Vec2::new(dir_world.dot(right_flat), dir_world.dot(forward_flat));
        let Some(scale) = screen.scale(window) else {
            return;
        };
        let radius = HANDLE_SIZE * scale;
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

/// The two streams an instrument writes: the walk finger's touch messages
/// (the joystick) and the tap finger's window events (the gesture layer).
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct InjectedTouches<'w> {
    touches: MessageWriter<'w, TouchInput>,
    window_events: MessageWriter<'w, bevy::window::WindowEvent>,
}

/// The door walk's progress on the current site.
#[derive(Default)]
pub(crate) struct DoorWalk {
    pressing: bool,
    /// The site generation the door button was pressed on.
    tapped: Option<u64>,
    /// The site generation this walk runs on, and when it began.
    since: Option<(u64, f32)>,
    /// The walk stopped on this generation (reached or timed out).
    stopped: Option<u64>,
    last_log: f32,
    /// The last change-target tap.
    last_change: f32,
}

/// Instrument (`MOLY_DOOR_WALK_SECS`, off by default; from
/// `MOLY_DOOR_WALK_AFTER` seconds): on the home site walk the player past
/// the house's inside-door point towards the house, in a room to the door
/// sensor, through the
/// joystick's touch stream; once the stack head is that door's button (the
/// house entry, the go-home button) tap it on its screen position, through
/// the gesture and click path. The press moves the site, and the walk
/// starts again on the next one. `MOLY_DOOR_WALK_OFFSET=x,z` shifts the
/// walk's goal by that many metres (to come at the sensor from another
/// side; the button still waits for the sensor's own sphere).
#[allow(clippy::too_many_arguments)]
pub(crate) fn smoke_door_walk(
    mut touches: MessageWriter<TouchInput>,
    mut window_events: MessageWriter<bevy::window::WindowEvent>,
    screen: ActionButtonScreen,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    players: Query<&Transform, With<PlayerControlled>>,
    houses: Query<(&crate::site_move::door::HouseEntryPoint, &GlobalTransform)>,
    points: Query<&GlobalTransform>,
    joystick: Res<JoystickState>,
    button_state: Res<ActionButtonState>,
    room_door: Option<Res<crate::site_move::room_door::RoomDoor>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    time: Res<Time>,
    mut walk: Local<DoorWalk>,
) {
    let armed = env_secs("MOLY_DOOR_WALK_SECS");
    if armed <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    let Ok((window_entity, window)) = screen.windows.single() else {
        return;
    };
    const FINGER: u64 = 99011;
    const TAP_FINGER: u64 = 99012;
    let base = Vec2::new(window.width() * 0.15, window.height() * 0.75);
    // The joystick reads the touch messages; the gesture layer reads the
    // window's event stream, so the tap finger goes there.
    let mut touch = |id: u64, phase: TouchPhase, position: Vec2| {
        let input = TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id,
        };
        if id == TAP_FINGER {
            window_events.write(bevy::window::WindowEvent::TouchInput(input));
        } else {
            touches.write(input);
        }
    };
    let walk = &mut *walk;
    let release = |walk: &mut DoorWalk, touch: &mut dyn FnMut(u64, TouchPhase, Vec2)| {
        if walk.pressing {
            touch(FINGER, TouchPhase::Ended, base);
            walk.pressing = false;
        }
    };
    // Hold the walk finger still at the pad's centre (direction zero) while
    // standing: a free touch would be the joystick's next capture, and the
    // tap finger must reach the gesture layer instead.
    let hold = |walk: &mut DoorWalk, touch: &mut dyn FnMut(u64, TouchPhase, Vec2)| {
        if walk.pressing {
            touch(FINGER, TouchPhase::Moved, base);
        }
    };
    if now >= armed || now < env_secs("MOLY_DOOR_WALK_AFTER") || !joystick.enabled {
        release(walk, &mut touch);
        return;
    }
    let Some(epoch) = epoch.map(|epoch| epoch.0) else {
        return;
    };
    if walk.tapped == Some(epoch) || walk.stopped == Some(epoch) {
        release(walk, &mut touch);
        return;
    }
    let (Ok(player), Ok(camera)) = (players.single(), cameras.single()) else {
        return;
    };
    let target = match crate::site_move::room_door::sensor(room_door.as_deref(), Some(epoch)) {
        Some((sensor, _)) => points
            .get(sensor)
            .ok()
            .map(|at| (at.translation(), ButtonType::GoHomeSite)),
        // The house: half a metre past the inside-door point towards the
        // house, so the player arrives facing it (the player box is ahead).
        None => houses.iter().next().and_then(|(point, house)| {
            let at = points.get(point.0).ok()?.translation();
            let inward = (house.translation() - at).with_y(0.0).normalize_or_zero();
            Some((at + inward * 0.5, ButtonType::HouseEntry))
        }),
    };
    let Some((target, expected)) = target else {
        return;
    };
    let offset = std::env::var("MOLY_DOOR_WALK_OFFSET")
        .ok()
        .and_then(|raw| {
            let (x, z) = raw.split_once(',')?;
            Some(Vec3::new(x.trim().parse().ok()?, 0.0, z.trim().parse().ok()?))
        })
        .unwrap_or(Vec3::ZERO);
    let target = target + offset;
    let since = match walk.since {
        Some((generation, since)) if generation == epoch => since,
        _ => {
            walk.since = Some((epoch, now));
            info!(
                "[door-walk] site generation {epoch}: walking to the {expected:?} door at ({:.2},{:.2},{:.2})",
                target.x, target.y, target.z
            );
            now
        }
    };
    let delta = target - player.translation;
    let distance = Vec2::new(delta.x, delta.z).length();
    let head = button_state.current();
    if now - walk.last_log >= 1.0 {
        walk.last_log = now;
        info!(
            "[door-walk] player ({:.2},{:.2},{:.2}) distance {distance:.2} m, 3D {:.2} m, head {head:?}",
            player.translation.x,
            player.translation.y,
            player.translation.z,
            delta.length()
        );
    }
    // The buttons share one input interval (0.3 s): after a change-target
    // tap the door button waits it out.
    if head.is_some_and(|(button, _)| button == expected)
        && now - walk.last_change >= ACTION_BUTTON_INPUT_INTERVAL + 0.05
    {
        hold(walk, &mut touch);
        let Some(position) = screen.button_position(window, expected) else {
            return;
        };
        touch(TAP_FINGER, TouchPhase::Started, position);
        touch(TAP_FINGER, TouchPhase::Ended, position);
        walk.tapped = Some(epoch);
        info!(
            "[door-walk] head {head:?} at distance {distance:.2} m (3D {:.2} m); tapping its button at ({:.0},{:.0})",
            delta.length(),
            position.x,
            position.y
        );
        return;
    }
    if now - since > 40.0 {
        release(walk, &mut touch);
        walk.stopped = Some(epoch);
        warn!(
            "[door-walk] stopped at distance {distance:.2} m after {:.1} s without the {expected:?} button; head {head:?}",
            now - since
        );
        return;
    }
    // The door's button is stacked behind another head: the change-target
    // button brings it forward, one tap at a time.
    if head.is_some_and(|(button, _)| button != expected)
        && button_state.stacked(expected)
        && button_state.change_shown()
        && now - walk.last_change >= 1.0
    {
        if let Some(position) = head.and_then(|(shown, _)| screen.change_position(window, shown)) {
            hold(walk, &mut touch);
            // Down and up in one frame: a slow frame must not turn the tap
            // into a long touch (0.25 s).
            touch(TAP_FINGER, TouchPhase::Started, position);
            touch(TAP_FINGER, TouchPhase::Ended, position);
            walk.last_change = now;
            info!(
                "[door-walk] {expected:?} is stacked behind {head:?}; tapping the change-target button at ({:.0},{:.0})",
                position.x, position.y
            );
            return;
        }
    }
    // At the door: stand and wait for the head (another entry that joined
    // first keeps it until its object leaves).
    if distance < 0.2 {
        hold(walk, &mut touch);
        return;
    }
    // The joystick's inverse, as the action button walk writes it.
    let dir_world = Vec2::new(delta.x, delta.z).normalize_or(Vec2::X);
    let forward = camera.forward();
    let forward_flat = Vec2::new(forward.x, forward.z).normalize_or_zero();
    let right = camera.right();
    let joy = Vec2::new(
        dir_world.dot(Vec2::new(right.x, right.z)),
        dir_world.dot(forward_flat),
    );
    let Some(scale) = screen.scale(window) else {
        return;
    };
    let position = base + Vec2::new(joy.x, -joy.y) * HANDLE_SIZE * scale;
    if !walk.pressing {
        touch(FINGER, TouchPhase::Started, base);
        walk.pressing = true;
        return;
    }
    touch(FINGER, TouchPhase::Moved, position);
}

/// 环境变量秒数（缺省 0）：与各域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
