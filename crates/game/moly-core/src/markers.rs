//! Markers and shared state that every layer of the game reads: which site is
//! active, which entity the player controls, the roots of character avatars.
//! The domains above define the behaviour; this module holds only the types
//! they share.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

/// 房间类站点：真源按 siteType 名切到 MyRoomSiteController 的三行。
pub const ROOM_TYPES: [&str; 3] = ["first_floor", "second_floor", "third_floor"];

/// Whether a site type is one of the indoor floors.
pub fn is_room(site_type: &str) -> bool {
    ROOM_TYPES.contains(&site_type)
}

/// `NPCAvatarPresenter.SetBirthdayLua`: the script a character's birthday
/// wait plays when the player talks to it, with its scenario row. The birthday
/// party's own module (`crate::cutscene::birthday_party`) writes it; the field's
/// talk chain reads it.
#[derive(Component, Clone, Debug)]
pub struct BirthdayLua {
    pub scenario: i64,
    pub lua: String,
}

/// GameStateManager.HasContext(MysekaiContextType.Birthday): set while the
/// birthday party runs (AddContext), and the field's button and talk chain
/// read it. The party's own module writes it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BirthdayContext {
    pub party: i64,
}

/// The order the field's readers keep against the action button: the button's
/// per-frame stack write runs before this name. The name belongs to the world
/// so its systems need not name the button's system.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionButtonOrder;

/// The order the field's loaders keep against the layout editor's input: the
/// editor's Input set runs before this name, and a loader that must not
/// interleave with a draft command runs after it. The name belongs to the
/// world so its systems need not name the editor's set.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EditInputOrder;

/// An avatar the cut-scene presenter created for a character unit that was
/// not present in the field. The world's systems skip it: its director owns
/// its animator, and the field's own walkers and disposers must not touch it.
#[derive(Component)]
pub struct CutSceneAvatar;

/// The component that marks the site map's tap view, so the map's visibility
/// pass can hide it with the screen.
#[derive(Component)]
pub struct SitemapTapView;

/// 当前站点的账目面：锚行与日志的名字、位点、房间拼装来源都读它。
#[derive(Clone, Resource)]
pub struct SiteActive {
    pub site_id: u32,
    pub level: u32,
    pub site_type: String,
    pub category: String,
    /// 场景目录名：inactiveNodes 路径以它为前缀（glTF 主根同名）。
    pub scene: String,
    /// 环境站名（主表 `assetbundleName` 列）：天气粒子链按它挑现象档案
    /// 的 `unique__<站>` 变体。
    pub env_site: String,
    pub position: [f32; 3],
    /// 房间类站点的拼装账目；场景包站点为 None。
    pub room: Option<RoomInfo>,
    /// 场景包站点的可行走面账目名（锚行报它，面从哪来逐站可对账）；
    /// 房间站的寻路面账目在 [`RoomInfo`] 里，此列为 None。
    pub nav_face: Option<String>,
}

impl SiteActive {
    /// The source's indoor predicate: `MysekaiSiteModel.IsIndoor` is
    /// `SiteType - 1 < 3` (first, second and third floor), and the load path
    /// tests the site name against the same three floors. The environment view
    /// hides the weather effects and shows the plain background on it.
    pub fn is_indoor(&self) -> bool {
        is_room(&self.site_type)
    }
}

/// 房间等级与寻路面来源（锚行报它：寻路面从哪来是可对账的）。
#[derive(Clone)]
pub struct RoomInfo {
    pub level: u32,
    /// Some = 室内导航包按等级落盘的面文件；None = 盘上 authored 无几何，
    /// 模块地板兼寻路面。
    pub walkable_file: Option<String>,
}

/// 站点场景实体标记（场景包站一个；房间站的壳、模块两面、寻路面各一）。
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct SiteRoot;

/// A renderable scene root is withheld until its source materials are installed.
/// Navigation-only roots never carry this marker and stay hidden permanently.
/// Scene instantiation and navigation do not depend on presentation visibility.
#[derive(Component)]
pub struct SiteVisualPending;

/// scene 实例已展开完毕（全部站点 scene 就绪）；由 `on_scene_ready` 计数
/// 置位。
#[derive(Resource)]
pub struct SiteReady;

/// 全部站点 scene 就绪的常驻闩：站点域外按它对齐时机（站点材质换装的
/// 门）。与 [`SiteReady`] 同时置位，但不被清扫链撤下——换站拆资源时才撤，
/// 新一代 scene 全就绪再立。
#[derive(Resource)]
pub struct SiteScenesReady;

/// 站点内容定案的代数：每次定案（inactiveNodes 清扫完成）自增，跨站单调。
/// 换站后的重播种面与锚行以它为门——`SiteSettled` 同帧被取景行撤下，
/// Update 侧等不到（时机全解在 inactive_nodes 模块注释）。
#[derive(Clone, Copy, Resource)]
pub struct GroundEpoch(pub u64);

/// 玩家标记：NPC 独占的域（待机动作演出、tweet 气泡、表情编排）用它把
/// 玩家排除在外——玩家不是名册成员，那些域的触发器（驻留计时、编排表）
/// 对玩家没有真源对应物。
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct PlayerControlled;

/// 相机跟随的那名成员。相机域读它的世界变换逐帧取景——真源站点相机追
/// 的是 avatar 视变换。挂载由玩家域决定（玩家实体持它；名册成员不再
/// 插——玩家域接管时把「清单首行」的旧默认撤了）。
#[derive(Component)]
pub struct AvatarRoot;

/// 摇杆区谓词：窗口左下 35% 宽 × 50% 高。Bevy 屏坐标顶原点、y 向下
/// ⇒ x < 0.35W 且 y ≥ 0.5H。手势层读同一谓词复现射线分流。
pub fn in_zone(position: Vec2, width: f32, height: f32) -> bool {
    const ZONE_W: f32 = 0.35;
    const ZONE_H: f32 = 0.5;
    position.x < width * ZONE_W && position.y >= height * ZONE_H
}

/// 摇杆层状态：CustomJoyStick 字段面的移植子集 + presenter 侧的
/// 活跃沿。单实例。
#[derive(Resource, Default)]
pub struct JoystickState {
    /// 本帧让位门求值结果（编辑/对话在播 → false）。手势层读它复现
    /// 区内吞没。
    pub enabled: bool,
    /// 捕获中的手指 id（真源 _handlingPointerId 的 Nullable<int>）。
    /// 手势层读它跳过捕获指的事件。
    pub captured: Option<u64>,
    /// 底盘心（屏坐标、顶原点——捕获时等于触点）。也是拖动阈值的
    /// 按下点（PointerEventData.pressPosition）。
    pub base: Vec2,
    /// The captured finger's drag has started (PointerEventData.dragging):
    /// its moves reach OnDrag.
    pub dragging: bool,
    /// 最近一次 UPDATE 的方向（真源屏坐标语义：y 向上；已按上限夹）。
    pub direction: Vec2,
    /// presenter 的 _isJoyStickInput：收到过 UPDATE 且未收 END。
    pub active: bool,
    /// UPDATE 时刻烘好的世界移动向量（真源 _joyStickMoveDirection）。
    pub move_vector: Vec3,
}

/// 站点相机的多重采样档（页面合成要求关闭）。
pub const MYSEKAI_CAMERA_MSAA: Msaa = Msaa::Off;

/// The player body's scene instance has been attached (the loader's model is
/// in the tree); the physics scene reports whether the avatar it sees is
/// fully loaded with it.
#[derive(Component)]
pub struct BodyAttached;

/// Navigation uses the actual loaded snapshot's region, independently from the
/// page locale, player-save region, fixture catalog, or shared-weather donor.
/// The offline HOME starter layout reads the same identity: its mock rows must
/// name packages of this snapshot's own fixture-model index.
#[derive(Clone, Copy, Resource)]
pub struct NavMeshSourceRegion(pub moly_law::carve::NavMeshRegion);

impl NavMeshSourceRegion {
    pub fn parse(value: &serde_json::Value) -> Result<Self, String> {
        if value.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
            return Err("source.json has an unsupported or missing identity version".into());
        }
        let region = value
            .get("source")
            .and_then(|source| source.get("region"))
            .and_then(serde_json::Value::as_str);
        match region {
            Some("cn") => Ok(Self(moly_law::carve::NavMeshRegion::Cn)),
            Some("jp") => Ok(Self(moly_law::carve::NavMeshRegion::Jp)),
            _ => Err(format!(
                "source.json has unsupported source region {region:?} (source.region must be cn or jp; navigation and the offline HOME starter layout are keyed by it)"
            )),
        }
    }
}

/// 一张表情的绘制位：属于哪个实例（报告侧数「活着的有几个绘制实体」用，
/// 网格/材质句柄在实例自己的 draws 表里）。The shadow pass excludes these
/// draws: an emote stands in the world as a billboard, not as a caster.
#[derive(Component)]
pub struct EmoteDraw {
    pub instance: u64,
}

/// A skinned renderer carrying the player's own body material: the avatar's
/// dressed skin or the player's own model. The shadow pass counts these as the
/// player's body; every other character renderer is an NPC's.
#[derive(Component)]
pub struct PlayerBodyRenderer;

/// The camera has framed the site it holds this frame: the camera model and
/// the tracking bounds are written, so the global table can read them. Declared
/// here so a lower layer orders against it without naming the camera module.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SiteFramed;

/// 该成员的角色包句柄与包文件名（日志对账用）。骨架档案（材质值来源）
/// 与 glb 同清单行，一起装载。
#[derive(Component)]
pub struct CharacterPack {
    pub gltf: Handle<Gltf>,
    pub file: String,
    pub rig: Handle<JsonAsset>,
    pub rig_file: String,
}

/// 模型场景的挂载点子实体（持 [`SceneRoot`]），挂在成员实体下。
#[derive(Component)]
pub struct CharacterModel;

/// 名册成员的身份：真源 `npcRegistry.characterUnitIds` 身份列的一行。
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct CharacterUnitId(pub u32);

/// The unit an entity animates, as the animation-time check names it.
impl crate::animation_time::UnitId for CharacterUnitId {
    fn unit_id(&self) -> u32 {
        self.0
    }
}

/// The member carries an animation driver: the field NPC's motion driver or
/// the player's avatar driver. The render side materialises a member once its
/// driver is in place.
#[derive(Component)]
pub struct CharacterDriver;
