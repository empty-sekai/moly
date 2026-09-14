//! 接近触发的动作按钮：碰撞判定、按钮类型解析、按钮栈。
//!
//! 源侧这条链是：碰撞管理器每帧重建玩家的二维有向盒、与场上每个
//! 碰撞对象求交，进出边沿分别转成「压栈」「出栈」两个调用；栈顶那
//! 一项决定屏幕上显示哪个动作按钮，按下时按按钮类型分派。
//!
//! 三件从源逐句转录的东西在这里：
//! 分离轴求交（`CollisionBox2D`）· 球形判定（`inside_circle`）·
//! 两张类型映射表（`ButtonType::from_action_type` 与 `icon_file_name`）。
//!
//! 三处刻意与「教科书写法」不同、但与源一致的地方，改动前先读这里：
//!
//! 1. 分离轴投影只取盒子的 Front 与 Back 两条边，四个角里
//!    `(+hx, +hy)` 那个从不参与投影。写全四角会改变斜置盒子的结果。
//! 2. 投影轴不做归一化。同一根轴上两段区间的比较不需要它，源也没做。
//! 3. 球形判定用的是三维距离（含高度），而盒判定在 XZ 平面上做，
//!    高度整个丢掉。两种判定的维度不同，不是笔误。

use crate::fixture::Direction;

/// 玩家的附加碰撞盒每帧被放到身前这么远。源常量。
pub const ADDITIONAL_COLLISION_DISTANCE: f32 = 0.5;

/// 玩家附加碰撞盒的默认半长。源在建立时写死这一对，另有一个按参数
/// 改写它的入口，所以这是默认值而不是唯一值。
pub const PLAYER_ADDITIONAL_HALF_EXTEND: [f32; 2] = [0.25, 0.5];

/// 玩家半径在拿不到导航体时的兜底值。源写死这个数。
/// 注意：动作按钮这条链上的盒判定**不读**玩家半径，只有球判定读
/// 目标自己的半径。这个常量留在这里是为了别人来找时不必再翻一遍。
pub const PLAYER_RADIUS_FALLBACK: f32 = 0.15;

/// 角色在 XZ 平面上的尺寸。源在静态构造里用一条把两个 32 位通道
/// 同时置 1.0 的浮点立即数指令写入，故两个分量都是 1.0。
/// 角色碰撞盒的半长是它的一半。
pub const CHARACTER_SIZE_XZ: [f32; 2] = [1.0, 1.0];

/// 连续两次动作按钮输入之间的最小间隔，秒。源里是一个常量字段。
pub const ACTION_BUTTON_INPUT_INTERVAL: f32 = 0.3;

/// 源在旋转二维向量时用的度转弧度系数，取它写下的那个字面量。
pub const DEG_TO_RAD: f32 = 0.017453;

/// 二维线段。分离轴既当轴、又当被投影的一段区间。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment2D {
    pub p1: [f32; 2],
    pub p2: [f32; 2],
}

/// 二维有向矩形。源把它挂在玩家、NPC 与家具上，三者共用同一套求交。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionBox2D {
    pub center: [f32; 2],
    pub half_extend: [f32; 2],
    /// 角度制。
    pub rotation: f32,
}

impl CollisionBox2D {
    pub fn new(center: [f32; 2], half_extend: [f32; 2], rotation: f32) -> Self {
        CollisionBox2D {
            center,
            half_extend,
            rotation,
        }
    }

    /// 源的 `SetCenter` 吃一个三维点，写进去的是它的 x 与 **z**。
    /// 高度在这一层被丢掉，盒判定完全在 XZ 平面上。
    pub fn set_center_world(&mut self, world: [f32; 3]) {
        self.center = [world[0], world[2]];
    }

    pub fn set_rotation(&mut self, degrees: f32) {
        self.rotation = degrees;
    }

    fn rotate_vector(&self, v: [f32; 2]) -> [f32; 2] {
        let radians = self.rotation * DEG_TO_RAD;
        let (sin, cos) = radians.sin_cos();
        [v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos]
    }

    /// 四条边的两个端点，先按盒子的角度旋转、再平移到中心。
    /// 端点顺序照抄源的分支，别改成「顺时针一圈」那种更整齐的排法：
    /// 分离轴取的是 `p1 - p2`，方向反了轴就反了。
    pub fn edge(&self, dir: Direction) -> Segment2D {
        let hx = self.half_extend[0];
        let hy = self.half_extend[1];
        let (a, b) = match dir {
            Direction::Front => ([-hx, -hy], [-hx, hy]),
            Direction::Left => ([-hx, hy], [hx, hy]),
            Direction::Back => ([hx, -hy], [-hx, -hy]),
            Direction::Right => ([hx, hy], [hx, -hy]),
        };
        let ra = self.rotate_vector(a);
        let rb = self.rotate_vector(b);
        Segment2D {
            p1: [ra[0] + self.center[0], ra[1] + self.center[1]],
            p2: [rb[0] + self.center[0], rb[1] + self.center[1]],
        }
    }

    /// 分离轴求交。四根轴：本盒的 Left/Right 两条边、对方盒的
    /// Left/Right 两条边。注意后两根轴上被投影的是**本盒**——源就是
    /// 这样传的，两次都把 `this` 当作被投影的一方。
    pub fn collides(&self, other: &CollisionBox2D) -> bool {
        if is_separating_axis(self.edge(Direction::Left), other) {
            return false;
        }
        if is_separating_axis(self.edge(Direction::Right), other) {
            return false;
        }
        if is_separating_axis(other.edge(Direction::Left), self) {
            return false;
        }
        !is_separating_axis(other.edge(Direction::Right), self)
    }
}

/// 一段线段在一根轴上的投影区间。轴不归一化。
fn project_segment(segment: Segment2D, axis: [f32; 2]) -> (f32, f32) {
    let d1 = segment.p1[0] * axis[0] + segment.p1[1] * axis[1];
    let d2 = segment.p2[0] * axis[0] + segment.p2[1] * axis[1];
    if d1 < d2 {
        (d1, d2)
    } else {
        (d2, d1)
    }
}

fn is_overlapping(a: (f32, f32), r: (f32, f32)) -> bool {
    a.1 >= r.0 && r.1 >= a.0
}

/// 轴由一条边给出，方向取 `p1 - p2`；被投影的盒子只取 Front 与 Back
/// 两条边的四个端点（去重后三个角），并集成一段区间。
fn is_separating_axis(axis: Segment2D, box2d: &CollisionBox2D) -> bool {
    let a = [axis.p1[0] - axis.p2[0], axis.p1[1] - axis.p2[1]];
    let axis_range = project_segment(axis, a);
    let front = project_segment(box2d.edge(Direction::Front), a);
    let back = project_segment(box2d.edge(Direction::Back), a);
    let r = (front.0.min(back.0), front.1.max(back.1));
    !is_overlapping(axis_range, r)
}

/// 球形判定。三维距离，含高度；比较的是**目标自己的半径**，玩家半径
/// 不参与。源用 `<=`，等于半径时算命中。
pub fn inside_circle(owner_position: [f32; 3], position: [f32; 3], radius: f32) -> bool {
    let dx = owner_position[0] - position[0];
    let dy = owner_position[1] - position[1];
    let dz = owner_position[2] - position[2];
    (dz * dz + (dx * dx + dy * dy)).sqrt() <= radius
}

/// 玩家的附加碰撞盒每帧被重建：中心放到身前 0.5 米，角度取偏航角的
/// 相反数。源在碰撞管理器的每帧更新里做这两件事。
pub fn update_player_box(box2d: &mut CollisionBox2D, position: [f32; 3], forward: [f32; 3], yaw_degrees: f32) {
    box2d.set_center_world([
        position[0] + forward[0] * ADDITIONAL_COLLISION_DISTANCE,
        position[1] + forward[1] * ADDITIONAL_COLLISION_DISTANCE,
        position[2] + forward[2] * ADDITIONAL_COLLISION_DISTANCE,
    ]);
    box2d.set_rotation(-yaw_degrees);
}

/// NPC 的碰撞盒：半长是角色尺寸的一半，中心在脚下，不带旋转。
pub fn character_box(position: [f32; 3]) -> CollisionBox2D {
    CollisionBox2D::new(
        [position[0], position[2]],
        [CHARACTER_SIZE_XZ[0] * 0.5, CHARACTER_SIZE_XZ[1] * 0.5],
        0.0,
    )
}

/// 家具的碰撞盒：半长是它占的格数乘格边长再折半，不带旋转。
/// 源用的格边长与摆放公式那一族同一个常量。
pub fn fixture_box(position: [f32; 3], grid_x: i32, grid_z: i32) -> CollisionBox2D {
    let tile = crate::fixture::position::TILE_SIZE;
    CollisionBox2D::new(
        [position[0], position[2]],
        [
            tile * grid_x as f32 * 0.5,
            tile * grid_z as f32 * 0.5,
        ],
        0.0,
    )
}

/// 碰撞对象的类别。源按它分派到三种判定：家具与角色走有向盒，
/// 其余走球。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionType {
    Fixture = 0,
    Harvest = 1,
    Character = 2,
    Gimmick = 3,
    Delivery = 4,
}

/// 家具在主数据里的类别。动作按钮只认其中两种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureType {
    System = 0,
    Custom = 1,
    Plant = 2,
    HousePlant = 3,
    SurfaceAppearance = 4,
    Gate = 5,
    Normal = 6,
    Canvas = 7,
}

impl FixtureType {
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => FixtureType::System,
            1 => FixtureType::Custom,
            2 => FixtureType::Plant,
            3 => FixtureType::HousePlant,
            4 => FixtureType::SurfaceAppearance,
            5 => FixtureType::Gate,
            6 => FixtureType::Normal,
            7 => FixtureType::Canvas,
            _ => return None,
        })
    }

    /// 源在筛「这件家具能不能出动作按钮」时，机关家具与演出家具各有
    /// 自己的分支，剩下的按类别判：只有 system 与 gate 放行。
    /// 普通摆件、自定义家具、植物一律没有按钮。
    pub fn can_action(self) -> bool {
        matches!(self, FixtureType::System | FixtureType::Gate)
    }
}

/// 家具上挂的「玩家能对它做什么」。源枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerActionType {
    None = 0,
    Furniture = 1,
    Home = 2,
    Gate = 3,
    Door = 4,
    Chest = 5,
    Entrance = 6,
    CraftTool = 7,
    MysekaiInfo = 8,
    MusicPlay = 9,
    Convert = 10,
    WarpPoint = 11,
    AvatarDressUp = 12,
    SecretShop = 13,
}

impl PlayerActionType {
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => PlayerActionType::None,
            1 => PlayerActionType::Furniture,
            2 => PlayerActionType::Home,
            3 => PlayerActionType::Gate,
            4 => PlayerActionType::Door,
            5 => PlayerActionType::Chest,
            6 => PlayerActionType::Entrance,
            7 => PlayerActionType::CraftTool,
            8 => PlayerActionType::MysekaiInfo,
            9 => PlayerActionType::MusicPlay,
            10 => PlayerActionType::Convert,
            11 => PlayerActionType::WarpPoint,
            12 => PlayerActionType::AvatarDressUp,
            13 => PlayerActionType::SecretShop,
            _ => return None,
        })
    }
}

/// 屏幕上那一组动作按钮的类型。取值不连续：源枚举里 4 与 13 不存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonType {
    None = 0,
    Talk = 1,
    GimmickFixture = 2,
    TimelineFixture = 3,
    HouseEntry = 5,
    OpenChest = 6,
    GateEdit = 7,
    OpenCraftTool = 8,
    OpenMysekaiInfo = 9,
    OpenMysekaiBgmSelect = 10,
    OpenMysekaiConvert = 11,
    ChangeActionTarget = 12,
    OpenOtherMysekai = 14,
    OpenAvatarDressUp = 15,
    Dash = 16,
    OpenSecretShop = 17,
    GoHomeSite = 18,
    LeaveMysekai = 19,
    Sketch = 20,
    Delivery = 21,
    BirthdayCutScene = 22,
    GateInvitation = 23,
    OpenMysekaiCompetition = 24,
    DeliveryInformation = 25,
}

impl ButtonType {
    /// 源把家具的动作类别换算成按钮类型。整张表照抄，包括两处会让人
    /// 想「顺手补齐」的地方：`Home` 与 `Entrance` 落在同一个按钮上，
    /// 而 `WarpPoint` 在源的跳转表里**没有条目**，落到默认分支抛异常。
    /// 这里把那一格表成 `None`，因为本仓不复刻异常。
    ///
    /// `Gate` 那一格看当前是不是在别人的世界里做客：做客时是「离开」，
    /// 在自己世界里是「编辑传送门」。
    pub fn from_action_type(action: PlayerActionType, is_visiting: bool) -> ButtonType {
        match action {
            PlayerActionType::None => ButtonType::None,
            PlayerActionType::Furniture => ButtonType::GimmickFixture,
            PlayerActionType::Home => ButtonType::HouseEntry,
            PlayerActionType::Gate => {
                if is_visiting {
                    ButtonType::LeaveMysekai
                } else {
                    ButtonType::GateEdit
                }
            }
            PlayerActionType::Door => ButtonType::GoHomeSite,
            PlayerActionType::Chest => ButtonType::OpenChest,
            PlayerActionType::Entrance => ButtonType::HouseEntry,
            PlayerActionType::CraftTool => ButtonType::OpenCraftTool,
            PlayerActionType::MysekaiInfo => ButtonType::OpenMysekaiInfo,
            PlayerActionType::MusicPlay => ButtonType::OpenMysekaiBgmSelect,
            PlayerActionType::Convert => ButtonType::OpenMysekaiConvert,
            PlayerActionType::WarpPoint => ButtonType::None,
            PlayerActionType::AvatarDressUp => ButtonType::OpenAvatarDressUp,
            PlayerActionType::SecretShop => ButtonType::OpenSecretShop,
        }
    }

    /// 图标文件名。源先用一个位掩码筛「这个类型有没有图标」，再查一张
    /// 二十五格的表；掩码里空着的三格是冲刺、回主站点、造景比赛，
    /// 它们的按钮自带图形、不走这条路。
    ///
    /// 表里 2 与 3 指向同一个文件——机关家具与演出家具共用一张图。
    pub fn icon_file_name(self) -> Option<&'static str> {
        Some(match self {
            ButtonType::Talk => "icon_action_talk_wh",
            ButtonType::GimmickFixture => "icon_action_gimmick_wh",
            ButtonType::TimelineFixture => "icon_action_gimmick_wh",
            ButtonType::HouseEntry => "icon_action_home_wh",
            ButtonType::OpenChest => "icon_action_chest_wh",
            ButtonType::GateEdit => "icon_action_gate_wh",
            ButtonType::OpenCraftTool => "icon_action_crafttool_wh",
            ButtonType::OpenMysekaiInfo => "icon_action_mysekaiinformation_wh",
            ButtonType::OpenMysekaiBgmSelect => "icon_action_bgm_wh",
            ButtonType::OpenMysekaiConvert => "icon_action_convert_wh",
            ButtonType::ChangeActionTarget => "btn_selectchange_h80_wh",
            ButtonType::OpenOtherMysekai => "icon_action_visit_wh",
            ButtonType::OpenAvatarDressUp => "icon_action_avatardressup_wh",
            ButtonType::OpenSecretShop => "icon_action_secretshop_wh",
            ButtonType::LeaveMysekai => "icon_exit_room_wh",
            ButtonType::Sketch => "icon_action_sketch_wh",
            ButtonType::Delivery => "icon_action_amatsuyu_wh",
            ButtonType::BirthdayCutScene => "icon_action_birthday_wh",
            ButtonType::GateInvitation => "icon_action_invitation_wh",
            ButtonType::DeliveryInformation => "icon_action_infomation_wh",
            ButtonType::None
            | ButtonType::Dash
            | ButtonType::GoHomeSite
            | ButtonType::OpenMysekaiCompetition => return None,
        })
    }
}

/// 场上一个可交互目标的身份。源用一个复合唯一号，本仓用同一个整数
/// 空间里的两族：角色按角色号、家具按摆放号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetId {
    Character(u32),
    Fixture(i32),
}

/// 按钮栈。源的模型持一列 `(按钮类型, 目标号)`，进入碰撞时压入、
/// 离开时按目标号移出，栈**首**那一项决定当前显示哪个按钮。
/// 优先项插到队首，普通项接在队尾。同一对 `(类型, 目标)` 不重复入栈。
#[derive(Debug, Default, Clone)]
pub struct ButtonStack {
    entries: Vec<(ButtonType, TargetId)>,
}

impl ButtonStack {
    pub fn new() -> Self {
        ButtonStack {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, button: ButtonType, target: TargetId, is_priority: bool) {
        if self.entries.iter().any(|e| *e == (button, target)) {
            return;
        }
        if is_priority {
            self.entries.insert(0, (button, target));
        } else {
            self.entries.push((button, target));
        }
    }

    /// 按目标号移出。源在离开碰撞时按同一对键移除。
    pub fn remove(&mut self, target: TargetId) {
        self.entries.retain(|(_, t)| *t != target);
    }

    /// 只留下仍然在场的目标。源每帧会清掉引用已失效的项。
    pub fn retain_targets(&mut self, alive: &dyn Fn(TargetId) -> bool) {
        self.entries.retain(|(_, t)| alive(*t));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 栈首。空栈时源给出的是一对默认值，等价于「没有按钮」。
    pub fn first(&self) -> Option<(ButtonType, TargetId)> {
        self.entries.first().copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, button: ButtonType, target: TargetId) -> bool {
        self.entries.iter().any(|e| *e == (button, target))
    }
}
