//! 站点取景与相机跟随：真源 FieldCamera 一族的客户端律。
//!
//! 两层：
//! - **装载面**：`camera/out/mysekai__camera.json`（JsonAsset 通道）解析成
//!   `CameraSetting`（静态机位参数）与 `CameraParamAsset`（七条 AnimationCurve）。
//!   曲线在本仓没有消费方——全反编译语料里 `CameraParam` 的名字引用只出现在
//!   FieldCamera/FieldCameraModel 自身，模型字段在所有持模文件的读取点为零
//!   （按偏移与按名字双查、带阳性对照的三轮穷举，结论记在本模块注释与
//!   研究仓报告里）⇒ 相机运动在真源里**不**由这批曲线驱动：常态跟随走
//!   Normal 态逐帧律（无曲线），演出走 Timeline 的 FieldCamera*Behaviour
//!   自带 clip 曲线，搬运走按动画名分道的常数插值。装载面照读、曲线值
//!   进日志可复算——曲线若真有消费者（例如经运行时补丁注入），数据已在盘。
//! - **跟随律**：`follow_avatar` 按源 NormalCameraState.OnUpdate 的逐帧律
//!   推进一个镜像模型（[`FieldCameraModel`]），眼位公式与钳界次序逐行对源
//!   （式子注释钉在方法旁）。
//! - **FPS 态**：距离拉到下界后继续捏合进入第一人称（源
//!   `CanSwitchToFpsMode` 全链 + FPS 态三件套 OnEnter/OnUpdate/OnDrag），FPS
//!   态反向捏合退出回 Normal（继承支 / case 16 支，按站点类目分道）。
//!   转场一律 OutQuad 四分量补间（源 `DoTweenCameraSetting` 镜像）。
//!   ⚠ 源 OnEnter/OnExit 里的 UI 与 dither 族调用**不迁**（本仓无那些
//!   系统），逐条挂账写在 [`apply_input`] 注释里。
//!
//! 近远裁剪面与视角锥的注释保留在 [`spawn`]。曲线求值式写在
//! [`CurveAsset::evaluate`]——它是「与真源逐值可复算」的量法，节拍律化归
//! 下一单。

use crate::character::{AvatarRoot, CharacterShell};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::inactive_nodes::SiteSettled;
use crate::site::{GroundMeshes, SiteActive, SiteSelection};
use bevy::asset::{AssetPath, LoadState};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

/// Every compositing helper must match the current single-sample field target.
/// Bevy keys per-view colour allocations by MSAA as well as output/format; a
/// default 4x no-clear overlay is therefore NOT the same target as the 1x scene.
/// Actual GLES readbacks observed valid scene/post pixels replaced at that
/// overlay boundary. This is a renderer-stack compatibility invariant, not a
/// claim that current source runtime quality writes have all been excluded.
pub(crate) const MYSEKAI_CAMERA_MSAA: Msaa = Msaa::Off;

/// 取景中心附近的地表高度采样半径：取「脚下的地面」。
const SURFACE_RADIUS: f32 = 2.0;

/// 源 NormalCameraState.OnUpdate 的帧时长归一基（字面量 0.016667，非 1/60
/// 的精确值；native 反编译逐字面量核过）。
const FRAME_BASE: f32 = 0.016667;

/// 源 NormalCameraState.OnUpdate 的跟随插值系数：t = clamp((dt/基)·0.1, 0, 1)。
const FOLLOW_RATE: f32 = 0.1;

/// 源 Normal 态构造体的三个字面量：拖拽俯仰联动的保底距离，与
/// 「俯仰下限随缩放插值」的两端（角度制）。
const GESTURED_MIN_DISTANCE: f32 = 4.5;
const MIN_PITCH_AT_MAX_ZOOM_FROM: f32 = 8.0;
const MIN_PITCH_AT_MAX_ZOOM_TO: f32 = 24.0;

/// 滚轮一档折算的捏合像素数。源手势层是触屏、单位是像素；滚轮是 PC
/// 独有输入，此值是手感常量，非真源量。
const WHEEL_PINCH_PIXELS: f32 = 50.0;
// Product extension requested for close inspection: after entering FPS the
// wheel narrows the lens down to 20 degrees. Reverse first restores the source
// field of view, then the next outward gesture exits to the source Normal view.
const INSPECTION_MIN_FOV_DEGREES: f32 = 20.0;
const INSPECTION_ZOOM_RATE: f32 = 0.0025;

/// 源 FPSCameraState 构造体的字面量：FPS 态距离三界同值 0.15（私有模型
/// Distance/MinDistance/MaxDistance 一并写 0.15），俯仰界户外 −8 / 楼层
/// 室内 −17 / 上界 75。
const FPS_DISTANCE: f32 = 0.15;
const FPS_MIN_PITCH: f32 = -8.0;
const FPS_INDOOR_MIN_PITCH: f32 = -17.0;
const FPS_MAX_PITCH: f32 = 75.0;

/// 源 FPSCameraState.CAMERA_HEIGHT_OFFSET = (0, 0.1, 0)：FPS 态取景点 =
/// 玩家位置 + 此偏移（构造体 8 字节连写 y=0.1/z=0，x 为字段默认 0）。
/// 与模型 offset（照常作用于取景点与眼位）是两个量。
const FPS_CAMERA_HEIGHT_OFFSET: Vec3 = Vec3::new(0.0, 0.1, 0.0);

/// 转场补间时长（源字面量）：FPS 进入 0.2s；退出按站点类目分两支——
/// 采集/配送走继承支 0.5s，住宅走搬运 case 16 支 0.1s。缓动一律
/// OutQuad（源 EASE_BASIC = 6，DG.Tweening.Ease 枚举位序）。
const FPS_ENTER_TWEEN_SECS: f32 = 0.2;
const FPS_EXIT_TWEEN_SECS_INHERIT: f32 = 0.5;
const FPS_EXIT_TWEEN_SECS_CASE16: f32 = 0.1;

/// 真源场地相机的态（`FieldCamera.CurrentState` 的类型，闭集 21 值，
/// 判别值照抄；源拼写 FPS / ScreenshotCaptureFPS 在此按 Rust 惯例作
/// Fps / ScreenshotCaptureFps）。态迁移由各游戏态与演出流程驱动；本仓
/// 落了其中一条：Normal ↔ FPS 由缩放手势驱动（最小距离上继续捏合进入、
/// FPS 态反向捏合退出，见 [`apply_input`]），其余态无写者。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraStateType {
    None = 0,
    Normal = 1,
    FloorEdit = 2,
    WallEdit = 3,
    Talk = 4,
    LookAtPlayer = 5,
    SiteMoveAction = 6,
    PreSiteMoveAction = 7,
    HouseEntry = 8,
    CutScene = 9,
    SiteEnvironmentEdit = 10,
    SomeCharacterTalk = 11,
    ZoomPlayer = 12,
    PhotoShot = 13,
    HarvestTone = 14,
    TripodCamera = 15,
    Fps = 16,
    LearnSiteEnvironment = 17,
    ScreenshotCapture = 18,
    ScreenshotCaptureFps = 19,
    DeliveryHonorReward = 20,
}

/// 场地相机当前态（真源 `FieldCamera.CurrentState` 的读写面）。真源构造
/// 起值是 `None`，入站流程把它切到 `Normal`；本仓起步即 `Normal`（等价于
/// 「入站完成」的常驻态），写者是缩放手势的 FPS 进出迁移。位移律是它的
/// 读者：FPS 态下玩家步速减半（`player` 域逐帧取值，走/冲两态同构）。
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FieldCameraState(pub CameraStateType);

impl Default for CameraStateType {
    fn default() -> Self {
        CameraStateType::Normal
    }
}

/// FPS 态私有模型的旋转镜像（源 `FPSCameraState._model` 的 {Yaw, Pitch} 两
/// 字段；它的距离三界等常量只在进入时直写共享模型，无需镜像）。
///
/// 源里状态机各态是常驻单例，私有模型构造一次、跨会话存续：Yaw 每次进入
/// 重捕获，**Pitch 不重置**——再次进入时转场补间的目标俯仰是上一会话最后
/// 一次拖拽后的残留值（源同形：构造体只在构造时写一次 0）。拖拽律读它、
/// 写它，并经 SetRotate 同步共享模型。
#[derive(Resource, Debug, Clone, Copy)]
pub struct FpsViewMemory {
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for FpsViewMemory {
    fn default() -> Self {
        // 源构造体：Pitch = 0（v12+60 直写 0），Yaw = 0（常量槽第四元）。
        Self {
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Normal 态的转场记忆（源 NormalCameraState 的两份存储并一）：Normal 态
/// OnExit 把私有镜像 {LookAt, Distance, Yaw, Pitch} 与站点转场表
/// `cameraTransferData[siteId]` 的 {pitch, yaw, fov, distance} **同刻同值**
/// 地写走，本仓单槽承载两语义。site 键等价源字典的站点分槽——继承支读侧
/// 按站点命中，miss 走源 fallback 形状（构造体 initPitch + 当前模型值）；
/// 住宅 case 16 支读的是不分站的私有镜像，同一槽不看键。
#[derive(Resource, Debug, Clone)]
pub struct NormalCameraMemory {
    pub site: String,
    pub look_at: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
}

/// 源 `DoTweenCameraSetting`（无 Ease 参的重载）的补间镜像：四条并行
/// OutQuad——LookAt/Pitch/Yaw/Distance 逐帧写共享模型，FOV 写**相机本体**
/// （源 setter 直写 `Camera.fieldOfView`，不写模型字段）。起值构造时捕获
/// （偏航经 GetToRotation 取最短有符号方向），新补间顶掉旧补间（源
/// KillTween 先行），到头即撤。
#[derive(Resource, Debug, Clone, Copy)]
pub struct CameraTween {
    pub look_at: (Vec3, Vec3),
    pub fov: (f32, f32),
    pub pitch: (f32, f32),
    pub yaw: (f32, f32),
    pub distance: (f32, f32),
    pub duration: f32,
    pub elapsed: f32,
}

/// 相机 JSON 的装载请求；解析成功后即撤。内部形参带私有资源，故
/// pub(crate)。
#[derive(Resource)]
pub(crate) struct CameraJsonHandle(Handle<JsonAsset>);

/// 真源 `CameraSetting`（序列化字段）的镜像：静态机位参数。
#[derive(Resource, Debug, Clone, Copy)]
pub struct CameraSetting {
    pub offset: Vec3,
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub init_yaw: f32,
    pub init_pitch: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    pub rot_sensitivity: f32,
    pub fov: f32,
}

/// 真源 `CameraParam`（七条 AnimationCurve 的 ScriptableObject）的装载镜像。
/// 每条曲线的键值原样保存；求值见 [`CurveAsset::evaluate`]。
#[derive(Resource, Debug, Default)]
pub struct CameraParamAsset {
    pub distance: CurveAsset,
    pub yaw: CurveAsset,
    pub pitch: CurveAsset,
    pub move_path_speed: CurveAsset,
    pub move_path_high: CurveAsset,
    pub move_camera_x_path: CurveAsset,
    pub move_camera_z_path: CurveAsset,
}

/// 一条 Unity `AnimationCurve` 的键序列（time/value/inSlope/outSlope）。
///
/// 只承载非权重 Hermite：源资产的 `weightedMode` 字段全部键为 0（三进位
/// 位标记，0 = 两端权重都不启用），本仓资产已核全 0。非 0 键在解析点
/// 响亮拒绝——静默降级成非权重式会得到一个看似合理的错值。
#[derive(Debug, Default, Clone)]
pub struct CurveAsset {
    pub keys: Vec<CurveKey>,
}

/// 单个关键帧：源 `Keyframe` 的四数值字段（time/value/inSlope/outSlope）。
#[derive(Debug, Clone, Copy)]
pub struct CurveKey {
    pub time: f32,
    pub value: f32,
    pub in_slope: f32,
    pub out_slope: f32,
}

impl CurveAsset {
    /// Unity `AnimationCurve.Evaluate(time)` 的 Hermite 求值，非权重式。
    ///
    /// 式子（与 Unity 文档的三次 Hermite 基一一对应）：
    /// ```text
    /// dt   = t1 - t0
    /// m0   = outSlope0 * dt      （斜率是值/秒，基函数吃的是值·dt）
    /// m1   = inSlope1  * dt
    /// u    = (time - t0) / dt    （段内归一参数）
    /// value(t) = h00·v0 + h10·m0 + h01·v1 + h11·m1
    ///   h00 = 2u³ - 3u² + 1      h01 = -2u³ + 3u²
    ///   h10 = u³ - 2u² + u       h11 = u³ - u²
    /// ```
    /// 时间轴语义（Unity 序列化行为）：time 在首键之前回首键值、末键之后
    /// 回末键值（PreInfinity/PostInfinity=2 即 `Constant`，资产全键同值）；
    /// 空曲线（`_moveCameraZPath` 就是一条）求值取 0。
    pub fn evaluate(&self, time: f32) -> f32 {
        let Some(&first) = self.keys.first() else {
            return 0.0;
        };
        if time <= first.time {
            return first.value;
        }
        let Some(&last) = self.keys.last() else {
            return 0.0;
        };
        if time >= last.time {
            return last.value;
        }
        // 折半找包含 time 的段：键按 time 升序（源资产序已核）。
        let idx = self
            .keys
            .partition_point(|key| key.time <= time)
            .saturating_sub(1);
        let (a, b) = (self.keys[idx], self.keys[idx + 1]);
        let dt = b.time - a.time;
        if dt <= 0.0 {
            return a.value;
        }
        let u = (time - a.time) / dt;
        let m0 = a.out_slope * dt;
        let m1 = b.in_slope * dt;
        let u2 = u * u;
        let u3 = u2 * u;
        let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h10 = u3 - 2.0 * u2 + u;
        let h01 = -2.0 * u3 + 3.0 * u2;
        let h11 = u3 - u2;
        h00 * a.value + h10 * m0 + h01 * b.value + h11 * m1
    }
}

/// 真源 `FieldCameraModel` 的逐字段镜像（跟随律的全部状态）。
///
/// 初值来自 `FieldCameraModel..ctor(CameraSetting, CameraParam)`：偏移/俯仰/
/// 距离三界/俯仰界/灵敏度/FOV 全部直读序列化值；`LookAt` 初值在源里是
/// Vector3.zero 静态字段（构造体不写它）——站点装载时由
/// `SetupLockAtCameraBounds` 一族直写坐标与界，进入 Normal 态后由跟随律
/// 逐帧推进。
#[derive(Resource, Debug, Clone)]
pub struct FieldCameraModel {
    /// 跟随目标点（源 `_LookAt`，偏移前的量）。
    pub look_at: Vec3,
    /// 眼位抬升偏移（源 `_Offset`，作用于取景点与眼位两处）。
    pub offset: Vec3,
    pub fov: f32,
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub rot_sensitivity: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    /// 手势记忆的距离（源 Normal 态自持镜像的 `_Distance`）：缩放手势写
    /// 它；拖拽俯仰联动只压当帧生效距离，不碰它。
    pub gestured_distance: f32,
    /// 最近缩放时的取景点活动界（源 `_LookAtBounds`，站点装载时直写）。
    pub look_at_bounds: BoundsXz,
    /// 最远缩放时的取景点活动界（源 `_MaxLookAtBounds`）。
    pub max_look_at_bounds: BoundsXz,
}

/// XZ 平面上的界（源 `Bounds` 的 X/Z 两分量；Y 界在源里恒写 0 且跟随律
/// 不钳 Y，不迁）。
#[derive(Debug, Clone, Copy)]
pub struct BoundsXz {
    pub center: Vec2,
    pub extents: Vec2,
}

impl BoundsXz {
    fn min(&self) -> Vec2 {
        self.center - self.extents
    }
    fn max(&self) -> Vec2 {
        self.center + self.extents
    }
}

impl FieldCameraModel {
    /// 源 `FieldCameraModel..ctor(CameraSetting, CameraParam)`：字段直读
    /// 序列化值，`LookAt` 起零（源 Vector3.zero 静态字段，构造体不写）。
    fn from_setting(setting: &CameraSetting) -> Self {
        Self {
            look_at: Vec3::ZERO,
            offset: setting.offset,
            fov: setting.fov,
            distance: setting.distance,
            min_distance: setting.min_distance,
            max_distance: setting.max_distance,
            yaw: setting.init_yaw,
            pitch: setting.init_pitch,
            rot_sensitivity: setting.rot_sensitivity,
            min_pitch: setting.min_pitch,
            max_pitch: setting.max_pitch,
            gestured_distance: setting.distance,
            look_at_bounds: BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
            max_look_at_bounds: BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
        }
    }
}

/// Startup：请求装载相机 JSON；单相机，fov 是垂直视场（真源同义）。
/// 站点的颜色是材质程序自己写好的终值：tonemapping 与去色带都关，不叠
/// 第二层处理。
pub fn spawn(mut commands: Commands, server: Res<AssetServer>) {
    let camera = commands
        .spawn((
            Camera3d {
                depth_texture_usages: (bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                    | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING).into(),
                ..Default::default()
            },
            crate::weather_depth::WeatherCameraRole::Base,
            // Supported shared-depth path: source Effect color is single-sample.
            // This is a runtime compatibility constraint, NOT evidence of the
            // current game's URP MSAA setting. Do not silently synthesize a resolve.
            MYSEKAI_CAMERA_MSAA,
            // Copy the actual opaque attachment; an engine geometry prepass
            // is neither the source producer nor a portable depth-copy carrier.
            crate::weather_depth::WeatherDepthSnapshot,
            // The empty loading view uses the engine default. The actual prefab
            // projection is installed with its bound setting before site framing.
            Projection::Perspective(PerspectiveProjection::default()),
            Tonemapping::None,
            DebandDither::Disabled,
            // 3D 音频收听者：耳朵对在听者局部 X 轴上，位置/朝向每帧随本实体的
            // 全局变换走（引擎侧语义）。真源相机挂的是无条件常驻的 3D 监听器，
            // 这里照挂——当前两条音频通道（BGM、区域环境音）按真源走平铺 2D，
            // 不消费它；它是给未来的 3D 一次性 SE 那一族预留的收听面。
            SpatialListener::new(4.0),
        ))
        .id();
    // Native wgpu/DX12 indirect offsets fail validation; keep that workaround
    // scoped to native. Browser material reloads are handled at asset import.
    #[cfg(not(target_arch = "wasm32"))]
    commands
        .entity(camera)
        .insert(bevy::render::view::NoIndirectDrawing);
    #[cfg(target_arch = "wasm32")]
    let _ = camera;
    let handle = server.load::<JsonAsset>(AssetPath::from(
        "moly://camera/out/mysekai__camera.json".to_owned(),
    ));
    commands.insert_resource(CameraJsonHandle(handle));
}

/// Update：相机 JSON 到位后解析一次并改写视角锥。装载失败或字段缺失
/// 响亮 panic（资产边界的一次性拒绝点），未到齐静默等下一帧。内部形参
/// 带私有资源，故 pub(crate)。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<CameraJsonHandle>>,
    mut cameras: Query<&mut Projection, With<Camera3d>>,
) {
    let Some(handle) = handle else {
        return; // 已解析并撤下
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("相机 JSON 装载失败：{err:?}");
    }
    let Some(asset) = jsons.get(&handle.0) else {
        return; // 还在装
    };
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|err| panic!("相机 JSON 不是合法 JSON：{err}"));
    let field_cameras = value["fieldCameras"]
        .as_array()
        .expect("bound FieldCamera instances");
    assert_eq!(
        field_cameras.len(),
        1,
        "the field-camera package must identify its sole camera"
    );
    let field = &field_cameras[0];
    assert_eq!(field["class"].as_str(), Some("FieldCamera"));
    let setting = parse_setting(linked_asset(&value, "cameraSettings", &field["setting"]));
    let params = parse_camera_params(linked_asset(&value, "cameraParams", &field["cameraParam"]));
    let view = &field["view"];
    assert_eq!(
        view["orthographic"].as_bool(),
        Some(false),
        "field camera must be perspective"
    );
    let plane = |name: &str| {
        view[name]
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| n.is_finite())
            .unwrap_or_else(|| panic!("FieldCamera view lacks {name}"))
    };
    let near = plane("nearClipPlane");
    let far = plane("farClipPlane");
    assert!(
        near > 0. && far > near,
        "invalid field camera projection planes"
    );
    // 视角锥改写：真源 SetupModel 一族把 FOV 应用到相机。
    let fov = setting.fov.to_radians();
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = fov;
            perspective.near = near;
            perspective.far = far;
            // Keep the explicit clipping plane in sync; leaving the engine
            // default here would silently retain a different projection near.
            perspective.near_clip_plane = Vec4::new(0., 0., -1., -near);
        }
    }
    info!("[camera] bound source projection: near={near}, far={far}");
    // 装载面状态行：曲线关键帧值逐条打印——装载结果可从这行复算
    // （键数、端点值、端点时刻；逐键全量在 JSON 里，本行是抽样锚）。
    for (name, curve) in [
        ("_distance", &params.distance),
        ("_yaw", &params.yaw),
        ("_pitch", &params.pitch),
        ("_movePathSpeed", &params.move_path_speed),
        ("_movePathHigh", &params.move_path_high),
        ("_moveCameraXPath", &params.move_camera_x_path),
        ("_moveCameraZPath", &params.move_camera_z_path),
    ] {
        match (curve.keys.first(), curve.keys.last()) {
            (Some(first), Some(last)) => info!(
                "[camera] 曲线 {name} 装载：{} 键，t∈[{:.4},{:.4}]，端值 {:.4}/{:.4}",
                curve.keys.len(),
                first.time,
                last.time,
                first.value,
                last.value
            ),
            _ => info!("[camera] 曲线 {name} 装载：0 键（空曲线，求值恒 0）"),
        }
    }
    // 求值点抽样：整点时刻各条曲线的手算对照值（与
    // `CurveAsset::evaluate` 同式，日志可推导）。
    for t in [0.5_f32, 1.5, 2.5] {
        info!(
            "[camera] 曲线求值点 t={t}：distance={:.4} yaw={:.4} pitch={:.4} speed={:.4} high={:.4} xPath={:.4} zPath={:.4}",
            params.distance.evaluate(t),
            params.yaw.evaluate(t),
            params.pitch.evaluate(t),
            params.move_path_speed.evaluate(t),
            params.move_path_high.evaluate(t),
            params.move_camera_x_path.evaluate(t),
            params.move_camera_z_path.evaluate(t),
        );
    }
    info!(
        "[camera] FieldCameraSetting 装载：offset {:?} distance {} initYaw {} initPitch {} fov {}（min/max 距离 {}/{}，俯仰界 {}/{}，灵敏度 {}）",
        setting.offset,
        setting.distance,
        setting.init_yaw,
        setting.init_pitch,
        setting.fov,
        setting.min_distance,
        setting.max_distance,
        setting.min_pitch,
        setting.max_pitch,
        setting.rot_sensitivity
    );
    commands.insert_resource(setting);
    commands.insert_resource(params);
    commands.remove_resource::<CameraJsonHandle>();
}

fn linked_asset<'a>(
    document: &'a serde_json::Value,
    table: &str,
    identity: &serde_json::Value,
) -> &'a serde_json::Value {
    assert!(
        identity["file"].as_str().is_some() && identity["pathId"].as_str().is_some(),
        "FieldCamera must supply a complete {table} identity"
    );
    let mut matches = document[table]
        .as_array()
        .expect("camera asset table")
        .iter()
        .filter(|row| row.get("asset") == Some(identity));
    let row = matches
        .next()
        .unwrap_or_else(|| panic!("FieldCamera {table} reference is unresolved"));
    assert!(
        matches.next().is_none(),
        "duplicate bound camera asset identity"
    );
    row
}

/// 解析实际绑定的 CameraSetting 字段。字段名与源的序列化名
/// 一一对应；缺列即响亮失败。
fn parse_setting(row: &serde_json::Value) -> CameraSetting {
    let fields = row
        .get("fields")
        .unwrap_or_else(|| panic!("绑定的 CameraSetting 缺 fields"));
    let vec3 = |name: &str| {
        let node = fields
            .get(name)
            .unwrap_or_else(|| panic!("CameraSetting 缺 {name}"));
        Vec3::new(
            node.get("x")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("CameraSetting.{name} 缺 x")) as f32,
            node.get("y")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("CameraSetting.{name} 缺 y")) as f32,
            node.get("z")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("CameraSetting.{name} 缺 z")) as f32,
        )
    };
    let scalar = |name: &str| {
        fields
            .get(name)
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("CameraSetting 缺 {name}")) as f32
    };
    CameraSetting {
        offset: vec3("offset"),
        distance: scalar("distance"),
        min_distance: scalar("minDistance"),
        max_distance: scalar("maxDistance"),
        init_yaw: scalar("initYaw"),
        init_pitch: scalar("initPitch"),
        min_pitch: scalar("minPitch"),
        max_pitch: scalar("maxPitch"),
        rot_sensitivity: scalar("rotSensitivity"),
        fov: scalar("fov"),
    }
}

/// 解析实际绑定的 CameraParam 的七条曲线。缺列响亮失败；`weightedMode`
/// 非 0 的键同样响亮失败（求值式只覆盖非权重 Hermite，见 `CurveAsset`）。
fn parse_camera_params(value: &serde_json::Value) -> CameraParamAsset {
    let curves = value
        .get("curves")
        .unwrap_or_else(|| panic!("绑定的 CameraParam 缺 curves"));
    let curve = |name: &str| {
        let node = curves
            .get(name)
            .unwrap_or_else(|| panic!("CameraParam 缺 {name}"));
        let keys = node
            .get("keys")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("CameraParam.{name} 缺 keys 数组"));
        let mut parsed = Vec::with_capacity(keys.len());
        for key in keys {
            let weighted = key
                .get("weightedMode")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| panic!("CameraParam.{name} 键缺 weightedMode"));
            if weighted != 0 {
                panic!(
                    "CameraParam.{name} 出现 weightedMode={weighted} 的键：\
                     非权重 Hermite 求值不覆盖该形态，拒绝静默降级"
                );
            }
            let f = |field: &str| {
                key.get(field)
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| panic!("CameraParam.{name} 键缺 {field}"))
                    as f32
            };
            parsed.push(CurveKey {
                time: f("time"),
                value: f("value"),
                in_slope: f("inSlope"),
                out_slope: f("outSlope"),
            });
        }
        CurveAsset { keys: parsed }
    };
    CameraParamAsset {
        distance: curve("_distance"),
        yaw: curve("_yaw"),
        pitch: curve("_pitch"),
        move_path_speed: curve("_movePathSpeed"),
        move_path_high: curve("_movePathHigh"),
        move_camera_x_path: curve("_moveCameraXPath"),
        move_camera_z_path: curve("_moveCameraZPath"),
    }
}

/// PostUpdate：站点源状态就绪（休眠实例保留，见 `inactive_nodes` 模块注释）
/// 完成后一次性取景并布置模型，取完撤 `SiteSettled`。
///
/// 排在 TransformPropagate 之后：scene 实体当帧在 SpawnScene 展开，
/// 这里读的是已传播完全局变换的地表。门吃 `SiteSettled` 不吃
/// `SiteReady`：锚数必须反映移除后的站点——`SiteReady` 在 Update 之后
/// 才置位，等不到 Update 侧系统的同帧处理。锚点取地表包围盒的 XZ 中心
/// 加脚下地表高度——全场景包围盒的中心落在半空或地里，曾把相机埋进
/// 地形。
///
/// 本步同时做两件源里站点装载时做的事：
/// - 初始化 [`FieldCameraModel`]（源 `SetupModel` → `FieldCameraModel..ctor`）；
/// - 布置取景活动界（源站点控制器 `SetupLockAtCameraBounds`：界中心 =
///   navMesh 场变换位置，界半径 = navMesh 界缩不可见格数×格值——我们的
///   navMesh 等价物是地面网格包围盒，格值 0.25 与源常量同值，不可见格数
///   取具名 mock）。
pub fn frame_site(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    ground: Option<Res<GroundMeshes>>,
    settled: Option<Res<SiteSettled>>,
    active: Option<Res<SiteActive>>,
    setting: Option<Res<CameraSetting>>,
    config: Option<Res<crate::client_config::ClientConfigs>>,
    parts: Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
    entities: Query<Entity>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let (Some(ground), Some(_), Some(active), Some(setting), Some(config)) =
        (ground, settled, active, setting, config)
    else {
        return;
    };
    let site = active.site_type.as_str();
    let mut verts = Vec::new();
    let mut ground_entities = 0;
    for (mesh3d, global) in &parts {
        if !ground.0.contains(&mesh3d.0) {
            continue;
        }
        ground_entities += 1;
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            panic!("地表网格实体引用的 Mesh 不在 Assets 里：装载门已过，不应发生");
        };
        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            panic!("地表网格没有 float3 的 POSITION 属性");
        };
        for position in positions {
            let world = global.transform_point(Vec3::from(*position));
            verts.push(world);
        }
    }
    if verts.is_empty() {
        panic!("默认 scene 里没有地表网格实体：{site}");
    }
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(-f32::MAX);
    for world in &verts {
        min = min.min(*world);
        max = max.max(*world);
    }
    let center = Vec2::new((min.x + max.x) * 0.5, (min.z + max.z) * 0.5);
    // 局部最高点，不是全场景最高点：后者在远处山体上，会把相机抬到半空。
    // 中心 {SURFACE_RADIUS} 米内可能没有顶点——地表网格中心是空洞
    // （memorialplace：中央碑座，最近顶点 2.8m）。此时退最近顶点的高：
    // 同一个「脚下地面」量的宽网兜底，不另发明值。
    let surface = verts
        .iter()
        .filter(|v| Vec2::new(v.x, v.z).distance_squared(center) <= SURFACE_RADIUS * SURFACE_RADIUS)
        .map(|v| v.y)
        .reduce(f32::max)
        .unwrap_or_else(|| {
            let nearest = verts
                .iter()
                .min_by(|a, b| {
                    let d = |v: &Vec3| Vec2::new(v.x, v.z).distance_squared(center);
                    d(a).total_cmp(&d(b))
                })
                .expect("顶点表非空，最近顶点必在");
            info!(
                "[camera] 取景中心 {SURFACE_RADIUS}m 内没有地表顶点（{site}）——地表网格中心空洞，退最近顶点高 {:+.3}",
                nearest.y
            );
            nearest.y
        });
    // 模型初始化 + 取景活动界（源 SetupLockAtCameraBounds 的式子，界
    // 半径 = fmax(全径 − 格值×不可见格数, 0)·0.5；全径即包围盒 x/z 跨度）。
    // 不可见格数是面板键（InvisibleGridCount，IntConfigs 键 69）。
    let mut model = FieldCameraModel::from_setting(&setting);
    let invisible_grid_count = config.int(crate::client_config::KEY_INVISIBLE_GRID_COUNT) as f32;
    let full_extents = Vec2::new(max.x - min.x, max.z - min.z);
    let shrink = moly_law::fixture::position::TILE_SIZE * invisible_grid_count;
    let near_extents = Vec2::new(
        (full_extents.x - shrink).max(0.0) * 0.5,
        (full_extents.y - shrink).max(0.0) * 0.5,
    );
    model.look_at_bounds = BoundsXz {
        center,
        extents: near_extents,
    };
    model.max_look_at_bounds = BoundsXz {
        center,
        extents: full_extents * 0.5,
    };
    // 初始取景点：站点中心（LookAt 起零后第一次跟随会拉向角色，这里给
    // 首帧一个站点内的合法起点）。
    model.look_at = Vec3::new(center.x, surface, center.y);
    // 真源式：偏航/俯仰转视线方向，目标点上抬 offset，眼位沿方向退 distance。
    let dir = view_dir(model.pitch, model.yaw);
    let pivot = model.look_at + model.offset;
    let eye = pivot + dir * model.distance;
    let mut camera = cameras.single_mut().expect("应恰有一台相机");
    *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
    // 一次性状态行：实体数与机位是「站点真的展开了吗」的现算证据。
    info!(
        "{site} 取景完成：实体 {}，网格实体 {}，地表实体 {}，眼位 {eye:.2}，取景点 {pivot:.2}",
        entities.iter().count(),
        parts.iter().count(),
        ground_entities,
    );
    info!(
        "[camera] 取景界：中心 ({:.2},{:.2}) 近界 {:#?} 远界 {:#?}（不可见格数 {invisible_grid_count}，面板 IntConfigs {}）",
        center.x,
        center.y,
        near_extents,
        full_extents * 0.5,
        crate::client_config::KEY_INVISIBLE_GRID_COUNT
    );
    commands.insert_resource(model);
    // 只撤 SiteSettled；GroundMeshes 常驻——它是站点装载闩，撤了会无限重铺整树。
    commands.remove_resource::<SiteSettled>();
}

/// 俯仰/偏航角（角度制）转视线方向（眼位在取景点反方向上）。
///
/// 源 `FieldCamera.GetPosition`：`eye = lookAt + offset +
///     Quaternion.Euler(pitch, yaw, 0) * (0, 0, -distance)`。
/// Unity 的 Euler 乘向量 = 先绕 X 轴转 pitch、再绕 Y 轴转 yaw 的合成旋转
/// 作用到 (0,0,-distance) 上。Unity 中 x=-cos(p)·sin(y)·d，y=sin(p)·d，
/// z=-cos(p)·cos(y)·d；资产导入以(-x,y,z)转换至Bevy右手系，因此这里只有
/// x项反号。屏幕Y翻转在输入边界处理，不能再用反转世界yaw补偿它。
fn view_dir(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let pitch = pitch_deg.to_radians();
    let yaw = yaw_deg.to_radians();
    Vec3::new(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        -(pitch.cos() * yaw.cos()),
    )
}

/// 源两条联动共用的归一式：`t = clamp((v−from)/(to−from), 0, 1)`，
/// `from == to` 时取 0（源先判不等再算，等值路径落 0）。
fn ratio01(value: f32, from: f32, to: f32) -> f32 {
    if from == to {
        0.0
    } else {
        ((value - from) / (to - from)).clamp(0.0, 1.0)
    }
}

/// Update：PC 输入接进跟随模型——左键拖 = 触屏单指拖（源 `OnDrag` 语义），
/// 滚轮 = 触屏捏合（源 `OnPinch` 语义）。拖拽从手势层来（`gesture.rs`：
/// 位移累计过拖拽阈值才激活 DRAG、激活后逐帧发 Moved——真源手势管理器
/// 只转发 UPDATE 相，激活前的小幅位移被吞）。两族输入都按当前相机态分派
/// （源 `FieldCamera.OnDrag/OnPinch` 转发给 `CurrentState` 的同名方法）。
/// 复位补间播着时两族整表吞掉（源 Normal 态 `OnDrag`/`OnPinch` 入口的
/// `_isResetAnimation` 门，先于 `UpdateAngle` 与 `CanSwitchToFpsMode`
/// 检查——复位期间捏合既不缩放也进不了 FPS。补间资源是
/// [`crate::menu_shell::CameraResetTween`]，门按其在场与否判、不分相机态：
/// 源 FPS 态的 `ResetCameraSetting` 是空方法，真源里 FPS 态按复位钮不
/// 触发任何补间；本仓复位钮不分态插补间，输入统一由这道门兜住）。
/// Normal 态律逐条对源：
///
/// - **旋转**（源 `FieldCamera.UpdateAngle`，原生体与伪码双树互证）：
///   ```text
///   yaw'  = yaw + 灵敏度·dx          // 越出 ±360 才 fmodf 360
///   pitch'= pitch − 灵敏度·dy
///   pitch = 先钳上界再抬下界         // 非对称序：min(v, max) 后 max(·, min)
///   ```
/// - **拖拽俯仰联动**（源 Normal 态 `OnDrag` 尾段）：`t` 取**刚写完的**
///   pitch，生效距离 = min(4.5 + t·(最大距离−4.5), 手势记忆距离)——低头
///   （pitch 低）强制拉近，鸟瞰放开；只压生效距离，不碰手势记忆。
/// - **缩放**（源 Normal 态 `OnPinch`）：先把缩放增量 v13 = 比例·(−捏合)
///   算好喂 [`can_switch_to_fps`]——**每个捏合事件都查**，距离已在下界
///   （位精确相等）且方向为拉近即切 FPS 态并**当帧不再缩放**；否则距离 =
///   clamp(距离 + v13, 距离界)，手势记忆与生效距离**同写**；随后俯仰下限
///   抬到 `max(8 + t₂·16 − 比例₂, 8, 模型俯仰下界)`，`t₂` 按新距离归一。
///
/// FPS 态：拖拽走 [`fps_drag`]（偏航 floor-mod 归一 + 俯仰钳界，源
/// `FPSCameraState.OnDrag`）；捏合**只有退出支**（v13 > 0 = 捏合张开方向
/// 即回 Normal，源 `FPSCameraState.OnPinch` 没有缩放支——FPS 态下的
/// 「继续放大」就是进入转场本身：距离 1.7→0.15 的位置前移，FOV 不动）。
/// Moly 的近距查看扩展允许进入 FPS 后继续滚轮缩小 FOV，先还原 FOV 再退出。
/// 进出迁移仍逐行对源（[`enter_fps`] / [`exit_fps`]）。
///
/// 不迁（逐条挂账）：双拖手势源两态本身就是空方法；源 FPS OnEnter 的 `SetLock(0)` /
/// `SetActiveUI(0)` / `SiteObjectManager.ShowAll(0)` / `ResetHouseDither(1)`
/// 与 OnExit 的 `SetActiveUI(1)`、Normal OnEnter 的
/// `FadeInPlayerIfNeeded` / `HideObstacleListIfNeeded` / OnExit 的
/// `SetDitherValue`——本仓没有锁、UI 面、房屋隐藏与 dither 系统，玩家显隐
/// 用可见性直写替代 dither 淡入淡出；源进入守卫里的玩家行为态
/// （UseTimelineFixture/Harvest）本仓无对应状态机，恒放行——真源里对话挡
/// 进入靠的是相机态自身切到 Talk，而对话相机不在本仓的态集里。FPS 态下
/// 站点切换未定义（源里站点搬运走相机态 6/7 结构性地先退出 FPS；本仓
/// `frame_site` 会重建模型而态不回退——本单不处理，具名在案）。
/// FPS 进出迁移的取数面（相机面板 · 站点选/当前站 · 玩家可见性 · 相机
/// 投影 · 玩家行为态——进入守卫的第二条读它）打包成一个参数：
/// `apply_input` 的裸参数已到 `SystemParam` 元组的
/// 上限（16），逐个展开会让整个系统静默失去 `IntoSystem`（错误只在
/// schedule 的 `.chain()` 处冒出来）。仅进入/退出支消费；拖拽/缩放的
/// 常规路径不走它。
#[derive(SystemParam)]
pub struct FpsTransitionCtx<'w, 's> {
    dialogs: Res<'w, crate::menu_shell::ShellDialogState>,
    layers: Res<'w, crate::ui_layers::UiLayerStack>,
    library: Res<'w, crate::content_library::ContentLibrary>,
    setting: Option<Res<'w, CameraSetting>>,
    site: Option<Res<'w, SiteSelection>>,
    active: Option<Res<'w, SiteActive>>,
    avatar: Res<'w, crate::player_state::PlayerAvatarStates>,
    talk_camera: Option<Res<'w, crate::talk_camera::TalkCamera>>,
    players: Query<'w, 's, (&'static GlobalTransform, &'static mut Visibility), With<AvatarRoot>>,
    cameras: Query<'w, 's, &'static mut Projection, With<Camera3d>>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_input(
    mut commands: Commands,
    mut gestures: MessageReader<GestureEvent>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut models: Option<ResMut<FieldCameraModel>>,
    edits: Res<crate::fixture_edit::EditSessionActive>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    reset: Option<Res<crate::menu_shell::CameraResetTween>>,
    mut state: ResMut<FieldCameraState>,
    mut fps_view: ResMut<FpsViewMemory>,
    prev_site: Res<PrevSiteType>,
    memory: Option<Res<NormalCameraMemory>>,
    mut transition: FpsTransitionCtx,
    mut smoke_entered: Local<bool>,
    mut smoke_exited: Local<bool>,
    mut smoke_pinched: Local<bool>,
) {
    // 摆放编辑面持有输入期间（真源编辑模式下手势层归编辑面），相机
    // 拖拽/缩放整表让位——编辑模式内的输入分配收口。
    if edits.is_active() || transition.dialogs.blocks_field_input() || !transition.layers.on_field()
    {
        gestures.clear();
        return;
    }
    // 相机只吃 DRAG 的逐帧 Moved（真源相机域的手势分派只对拖拽族的
    // UPDATE 态走 OnDrag；tap/drag 的区分阈值在手势层）。增量即事件
    // 自带的当帧增量。模型未立也把事件读掉，别攒陈账。
    let mut drag = Vec2::ZERO;
    for event in gestures.read() {
        if event.kind == GestureKind::Drag && event.state == GestureState::Moved && !event.ui_owned
        {
            drag += event.delta;
        }
    }
    let Some(model) = models.as_deref_mut() else {
        return; // 站点未取景：模型未立，输入无处落
    };
    let playback_inspection = transition.library.playback_camera_active();
    // FPS 冒烟钩子（无头验收用，玩家域 MOLY_PLAYER_AUTOWALK_SECS 同款）：
    // 窗口期内按当前态喂合成输入，走与真实输入同一条分派——Normal 持续
    // 捏合拉近（到底后继续捏 → 进入 FPS）、FPS 内拖拽、窗口后段反向捏合
    // 一次性退出。进入帧/FPS 无缩放支/退出支的实测值全部可从日志现算。
    let autofps = env_secs("MOLY_CAMERA_AUTOFPS_SECS");
    let mut syn_drag = Vec2::ZERO;
    let mut syn_pinch = 0.0;
    if autofps > 0.0 && time.elapsed_secs() < autofps {
        let phase = time.elapsed_secs() / autofps;
        match state.0 {
            CameraStateType::Normal if !*smoke_entered => syn_pinch = 60.0,
            CameraStateType::Fps if !*smoke_exited && phase < 0.7 => {
                syn_drag = Vec2::new(6.0, 3.0);
                // FPS 态继续捏合（拉近向）：应打「无缩放支」行且距离保持
                // ——「FPS 下继续放大」的实测证据，一次性喂入。
                if phase >= 0.4 && !*smoke_pinched {
                    *smoke_pinched = true;
                    syn_pinch = 60.0;
                }
            }
            CameraStateType::Fps if !*smoke_exited => {
                syn_pinch = -60.0;
                *smoke_exited = true;
            }
            _ => {}
        }
        if state.0 == CameraStateType::Fps {
            *smoke_entered = true;
        }
    }
    // 复位补间期间吞掉拖拽/缩放输入（上面门律的执行点）：事件与合成输入
    // 都先读掉再吞（读数沿纪律，别攒陈账），之后任何态分派——含 FPS
    // 进入支——整表让位。合成输入在这一步被并掉，让无头冒烟能在复位
    // 窗口里造出「被吞的那一下」并留下日志行。
    if reset.is_some() {
        let drag_total = drag + syn_drag;
        if drag_total != Vec2::ZERO || syn_pinch != 0.0 {
            info!(
                "[camera-fps] 复位补间期间吞掉输入：拖拽 ({:.0},{:.0}) 捏合 {:.0}px（态 {:?}）",
                drag_total.x, drag_total.y, syn_pinch, state.0
            );
        }
        return;
    }
    // --- 拖拽族分派（不需要面板）---
    let drag = drag + syn_drag;
    if drag != Vec2::ZERO {
        // Host screen coordinates grow down; source screen deltas grow up.
        // Hit testing remains in host coordinates. Convert at this consumer.
        let source_drag = Vec2::new(drag.x, -drag.y);
        match state.0 {
            CameraStateType::Fps => fps_drag(model, &mut fps_view, source_drag.x, source_drag.y),
            // Talk deliberately has no Normal pitch/distance coupling.
            CameraStateType::Talk => {
                if transition
                    .talk_camera
                    .as_deref()
                    .is_some_and(|camera| camera.accepts_input())
                {
                    update_angle(model, source_drag.x, source_drag.y);
                }
            }
            CameraStateType::Normal => {
                normal_drag(model, source_drag, playback_inspection);
            }
            _ => {}
        }
    }
    // 缩放族的两份比例来自 ClientConfig 面板（FloatConfigs 65/66）。面板
    // 未立（装载窗内）时缩放输入让位——装载闩语义，回落散写常量的路径
    // 不存在（面板缺席在解析点已经响亮 panic）。
    let Some(config) = configs.as_deref() else {
        return;
    };
    // 滚轮：向上为正 = 等价捏合张开（正增量→距离减→拉近）。像素单位的
    // 触摸板滚动本身就是像素，逐档单位才折算。
    let mut pinch = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y * WHEEL_PINCH_PIXELS,
        MouseScrollUnit::Pixel => scroll.delta.y,
    };
    pinch += syn_pinch;
    if pinch != 0.0 {
        let player = transition.players.single_mut().ok();
        // 源 Normal 态 OnPinch 开头算好的缩放增量，进入判定与缩放支共用。
        let ratio = config.float(crate::client_config::KEY_FIELD_CAMERA_ADD_DISTANCE_RATIO);
        let v13 = -ratio * pinch;
        match state.0 {
            CameraStateType::Normal if playback_inspection => {
                // Playback inspection must not replace the authored shot with
                // an FPS transition or Normal's pitch/distance coupling.
                playback_zoom(model, v13);
            }
            CameraStateType::Normal => {
                if can_switch_to_fps(model, player.is_some(), v13, &transition.avatar) {
                    // 进入支：当帧不再缩放（源 ChangeState 后直接 return）。
                    // 写面 = 当前站（快照键 + 室内判）· 玩家可见性 · 相机投影
                    // （FOV 起值捕获）；进入界来自 FPS 常量，不读面板。
                    let (Some(site), Some(active), Some((pg, pv)), Some(projection)) = (
                        transition.site.as_deref(),
                        transition.active.as_deref(),
                        player,
                        transition.cameras.single_mut().ok(),
                    ) else {
                        return; // 站点/相机未立：FPS 迁移的写面不全，让位下帧
                    };
                    enter_fps(
                        &mut commands,
                        model,
                        &mut state,
                        active,
                        site.is_room(),
                        pg.translation(),
                        pv,
                        &mut fps_view,
                        perspective_fov_deg(&projection),
                    );
                } else {
                    pinch_zoom(model, pinch, config);
                }
            }
            CameraStateType::Fps => {
                if let Ok(mut projection) = transition.cameras.single_mut() {
                    let current = perspective_fov_deg(&projection);
                    let baseline = memory
                        .as_deref()
                        .map(|memory| memory.fov)
                        .unwrap_or(model.fov);
                    if let Some(next) = inspection_fov(current, baseline, pinch) {
                        if let Projection::Perspective(projection) = &mut *projection {
                            projection.fov = next.to_radians();
                        }
                        // An unfinished entry tween must not overwrite the
                        // user's newer lens setting on the next follow frame.
                        commands.queue(move |world: &mut World| {
                            if let Some(mut tween) = world.get_resource_mut::<CameraTween>() {
                                tween.fov = (next, next);
                            }
                        });
                        return;
                    }
                }
                if v13 > 0.0 {
                    // 退出支（捏合张开方向）。FPS OnPinch 仅此一支。
                    let (Some(setting), Some(active), Some((pg, pv)), Some(projection)) = (
                        transition.setting.as_deref(),
                        transition.active.as_deref(),
                        player,
                        transition.cameras.single_mut().ok(),
                    ) else {
                        return;
                    };
                    exit_fps(
                        &mut commands,
                        model,
                        &mut state,
                        setting,
                        active,
                        &prev_site.0,
                        pg.translation(),
                        pv,
                        memory.as_deref(),
                        perspective_fov_deg(&projection),
                    );
                } else {
                    info!(
                        "[camera-fps] FPS 态捏合 Δ{:.1}px：无缩放支（源 OnPinch 仅反向退出支），距离保持 {:.2}",
                        pinch, model.distance
                    );
                }
            }
            CameraStateType::Talk => {
                if transition
                    .talk_camera
                    .as_deref()
                    .is_some_and(|camera| camera.accepts_input())
                {
                    // Talk.OnPinch calls AddDistance only. Normal's gesture
                    // memory, pitch coupling and FPS entry do not run here.
                    let before = model.distance;
                    model.distance =
                        (model.distance + v13).clamp(model.min_distance, model.max_distance);
                    info!(
                        "[talk-cam] 对话缩放：距离 {before:.2}→{:.2}，无 Normal 俯仰联动或 FPS 切换",
                        model.distance,
                    );
                }
            }
            // 其余态无写者，捏合整表让位（fail-closed，不猜律）。
            _ => {}
        }
    }
    // 冒烟脚本（无头验收用）：窗口期内按段喂合成手势，走与真实输入同一
    // 条律函数。四段分别压到俯仰下界、距离上界（连带俯仰下限抬升）、
    // 俯仰上界、距离下界——全部触发行与联动行都能在日志里现算。
    // 仅 Normal 态喂（FPS 态有自己的 AUTOFPS 钩子），且让位给 AUTOFPS。
    let autogesture = env_secs("MOLY_CAMERA_AUTOGESTURE_SECS");
    if autogesture > 0.0
        && autofps <= 0.0
        && state.0 == CameraStateType::Normal
        && !playback_inspection
        && time.elapsed_secs() < autogesture
    {
        let phase = time.elapsed_secs() / autogesture;
        if phase < 0.25 {
            update_angle(model, 12.0, 8.0);
            drag_distance_coupling(model);
        } else if phase < 0.5 {
            pinch_zoom(model, -60.0, config);
        } else if phase < 0.75 {
            update_angle(model, 12.0, -8.0);
            drag_distance_coupling(model);
        } else {
            pinch_zoom(model, 60.0, config);
        }
    }
}

fn inspection_fov(current: f32, baseline: f32, pinch: f32) -> Option<f32> {
    if !current.is_finite() || !baseline.is_finite() || !pinch.is_finite() || pinch == 0. {
        return None;
    }
    let minimum = INSPECTION_MIN_FOV_DEGREES.min(baseline);
    if pinch > 0. || current < baseline - 0.01 {
        Some((current * (-pinch * INSPECTION_ZOOM_RATE).exp()).clamp(minimum, baseline))
    } else {
        None
    }
}

#[cfg(test)]
mod inspection_zoom_tests {
    use super::*;
    fn playback_model() -> FieldCameraModel {
        FieldCameraModel {
            look_at: Vec3::new(1., 2., 3.), offset: Vec3::new(0., 0.5, 0.),
            fov: 35., distance: 6., min_distance: 1.7, max_distance: 8.,
            yaw: 0., pitch: 30., rot_sensitivity: 1., min_pitch: 8., max_pitch: 80.,
            gestured_distance: 7.,
            look_at_bounds: BoundsXz { center: Vec2::ZERO, extents: Vec2::ONE },
            max_look_at_bounds: BoundsXz { center: Vec2::ZERO, extents: Vec2::ONE },
        }
    }
    #[test]
    fn playback_orbit_preserves_distance_and_authored_camera_target() {
        let mut model = playback_model();
        normal_drag(&mut model, Vec2::new(20., 200.), true);
        assert_eq!(model.yaw, 20.);
        assert_eq!(model.pitch, model.min_pitch);
        assert_eq!(model.distance, 6.);
        assert_eq!(model.gestured_distance, 7.);
        assert_eq!(model.look_at, Vec3::new(1., 2., 3.));
        assert_eq!(model.fov, 35.);
        normal_drag(&mut model, Vec2::ZERO, false);
        assert_ne!(model.distance, 6.); // Normal exploration keeps its source coupling.
    }
    #[test]
    fn playback_zoom_stays_bounded_without_touching_pitch_or_lens() {
        let mut model = playback_model();
        for _ in 0..3 { playback_zoom(&mut model, -100.); }
        assert_eq!(model.distance, 1.7);
        assert_eq!(model.gestured_distance, 1.7);
        playback_zoom(&mut model, 100.);
        assert_eq!(model.distance, 8.);
        assert_eq!(model.pitch, 30.);
        assert_eq!(model.min_pitch, 8.);
        assert_eq!(model.fov, 35.);
        assert_eq!(model.look_at, Vec3::new(1., 2., 3.));
    }
    #[test]
    fn fps_inspection_zoom_is_bounded_and_unzooms_before_exiting() {
        let closer = inspection_fov(50., 50., 50.).unwrap();
        assert!(closer < 50. && closer > 20.);
        assert_eq!(inspection_fov(20., 50., 50000.), Some(20.));
        assert_eq!(inspection_fov(25., 50., -50000.), Some(50.));
        assert_eq!(inspection_fov(50., 50., -50.), None);
        assert_eq!(inspection_fov(f32::NAN, 50., 50.), None);
    }
}

/// 环境变量秒数（缺省 0）：与玩家域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}

/// 源 `FieldCamera.UpdateAngle(x, y)` 的逐行迁移，含两条界钳制触发行。
fn update_angle(model: &mut FieldCameraModel, dx: f32, dy: f32) {
    let raw_yaw = model.yaw + model.rot_sensitivity * dx;
    let yaw = if raw_yaw.abs() <= 360.0 {
        raw_yaw
    } else {
        raw_yaw % 360.0
    };
    model.yaw = yaw;
    let raw_pitch = model.pitch - model.rot_sensitivity * dy;
    let mut pitch = if raw_pitch <= model.max_pitch {
        raw_pitch
    } else {
        model.max_pitch
    };
    if raw_pitch < model.min_pitch {
        pitch = model.min_pitch;
    }
    model.pitch = pitch;
    info!(
        "[camera] 旋转输入 Δ({:.1},{:.1})px → yaw {:.1}° pitch {:.1}°（灵敏度 {}）",
        dx, dy, yaw, pitch, model.rot_sensitivity
    );
    if pitch != raw_pitch {
        info!(
            "[camera] 俯仰触界：raw {:.1}° → {:.1}°（界 [{:.1},{:.1}]）",
            raw_pitch, pitch, model.min_pitch, model.max_pitch
        );
    }
}

fn normal_drag(model: &mut FieldCameraModel, delta: Vec2, playback_inspection: bool) {
    update_angle(model, delta.x, delta.y);
    if !playback_inspection {
        drag_distance_coupling(model);
    }
}

fn playback_zoom(model: &mut FieldCameraModel, delta: f32) {
    model.distance = (model.distance + delta).clamp(model.min_distance, model.max_distance);
    model.gestured_distance = model.distance;
}

/// 源 Normal 态 `OnDrag` 尾段：按（刚更新的）pitch 压当帧生效距离。
fn drag_distance_coupling(model: &mut FieldCameraModel) {
    let t = ratio01(
        model.pitch,
        MIN_PITCH_AT_MAX_ZOOM_FROM,
        MIN_PITCH_AT_MAX_ZOOM_TO,
    );
    let cap = GESTURED_MIN_DISTANCE + t * (model.max_distance - GESTURED_MIN_DISTANCE);
    let before = model.distance;
    model.distance = cap.min(model.gestured_distance);
    if model.distance != before {
        info!(
            "[camera] 俯仰联动距离：{:.2} → {:.2}（联动上限 {:.2}，记忆 {:.2}）",
            before, model.distance, cap, model.gestured_distance
        );
    }
}

/// 源 Normal 态 `OnPinch(delta)` 的逐行迁移：距离双写 + 俯仰下限抬升。
/// 两份比例（换算比例与俯仰减项）是 ClientConfig 面板键（FloatConfigs
/// 65/66），随面板取值。
fn pinch_zoom(
    model: &mut FieldCameraModel,
    pinch: f32,
    config: &crate::client_config::ClientConfigs,
) {
    let add_distance_ratio =
        config.float(crate::client_config::KEY_FIELD_CAMERA_ADD_DISTANCE_RATIO);
    let raw = model.distance + (-add_distance_ratio * pinch);
    let mut distance = if raw <= model.max_distance {
        raw
    } else {
        model.max_distance
    };
    if raw < model.min_distance {
        distance = model.min_distance;
    }
    model.gestured_distance = distance;
    model.distance = distance;
    info!(
        "[camera] 缩放输入 Δ{:.1}px → 距离 {:.2}（界 [{:.2},{:.2}]，比例 {add_distance_ratio}，面板 FloatConfigs {}）",
        pinch, distance, model.min_distance, model.max_distance,
        crate::client_config::KEY_FIELD_CAMERA_ADD_DISTANCE_RATIO
    );
    if distance != raw {
        info!("[camera] 距离触界：raw {:.2} → {:.2}", raw, distance);
    }
    // 俯仰下限：插值端点来自构造体常量，减项是面板键（FloatConfigs 66）。
    let move_look_at_ratio =
        config.float(crate::client_config::KEY_FIELD_CAMERA_MOVE_LOOK_AT_RATIO);
    let t2 = ratio01(distance, GESTURED_MIN_DISTANCE, model.max_distance);
    let floor = (MIN_PITCH_AT_MAX_ZOOM_FROM
        + t2 * (MIN_PITCH_AT_MAX_ZOOM_TO - MIN_PITCH_AT_MAX_ZOOM_FROM)
        - move_look_at_ratio)
        .max(MIN_PITCH_AT_MAX_ZOOM_FROM)
        .max(model.min_pitch);
    let raw_pitch = model.pitch;
    let mut pitch = if raw_pitch <= model.max_pitch {
        raw_pitch
    } else {
        model.max_pitch
    };
    if raw_pitch < floor {
        pitch = floor;
    }
    model.pitch = pitch;
    if pitch != raw_pitch {
        info!(
            "[camera] 缩放联动俯仰：{:.1}° → {:.1}°（下限 {:.1}°，缩放归一 {:.2}）",
            raw_pitch, pitch, floor, t2
        );
    }
}

/// 角度归一到 [0, 360)（floor-mod）。源 FPS 态 `OnDrag` 的偏航归一式。
fn normalize360(deg: f32) -> f32 {
    deg - (deg / 360.0).floor() * 360.0
}

/// 角度归一到 [−180, 180]（源 `FieldCameraStateBase.ConvertAngle180`：
/// 先 fmodf 360，>180 减 360，<−180 加 360）。
fn wrap180(deg: f32) -> f32 {
    let mut v = deg % 360.0;
    if v > 180.0 {
        v -= 360.0;
    } else if v < -180.0 {
        v += 360.0;
    }
    v
}

/// 最短有向角（源 `GetToRotation(from, to)` = from + ConvertAngle180(to−from)）：
/// 转场补间的旋转终点都经它取「朝哪个方向转多少」，不是裸差。
fn to_rotation(from: f32, to: f32) -> f32 {
    from + wrap180(to - from)
}

/// OutQuad 缓动（源 `EASE_BASIC = 6`，DG.Tweening.Ease 位序：OutQuad）。
fn out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// 读相机本体当前的垂直视场（角度制）。源 `DoTweenCameraSetting` 的
/// prevFov 从 `Camera.fieldOfView` 取——**相机本体，不是模型字段**；本仓
/// 等价位是投影件。非透视投影响亮拒绝（场地相机按构造恒透视）。
fn perspective_fov_deg(projection: &Projection) -> f32 {
    match projection {
        Projection::Perspective(p) => p.fov.to_degrees(),
        _ => panic!("场地相机应是透视投影（FOV 捕获只对透视定义）"),
    }
}

/// 源 `NormalCameraState.CanSwitchToFpsMode(player, value)` 全链四条：
/// 玩家在位 ∧ 玩家行为态 ∉ {UseTimelineFixture, Harvest} ∧
/// |距离 − 最小距离| < float.Epsilon ∧ value < 0。
///
/// 第三条的源比较对象是 `COERCE_FLOAT(1)`——位型 1 是最小非正规数，对
/// 正常量级的距离值「|Δ| 小于它」当且仅当**位级相等**；而 [`pinch_zoom`]
/// 钳在下界时写的就是 `min_distance` 本身 ⇒ 位精确相等成立。第四条的
/// value 是缩放增量（v13 = 比例·(−捏合)，拉近方向为负）。第二条守卫读
/// [`crate::player_state::PlayerAvatarStates`] 的当前态（源读
/// `CurrentState`）：采集段与演出家具段拒入——本仓 28 暂无写者，该支
/// 由 7 单独挡着，演出流程进批后自动生效。
fn can_switch_to_fps(
    model: &FieldCameraModel,
    player_present: bool,
    value: f32,
    avatar: &crate::player_state::PlayerAvatarStates,
) -> bool {
    player_present
        && !matches!(
            avatar.current,
            crate::player_state::PlayerActionState::UseTimelineFixture
                | crate::player_state::PlayerActionState::Harvest
        )
        && model.distance == model.min_distance
        && value < 0.0
}

/// 源 `FPSCameraState.OnDrag(delta)` 逐行：读写**私有镜像**，随后经
/// `SetRotate(pitch, yaw)` 同步共享模型（SetRotate 体：Model.Pitch = x、
/// Model.Yaw = y，双树核过）。式子：
/// ```text
/// yaw'  = 私有.yaw + dx·灵敏度          // 灵敏度取私有模型（=共享模型值）
/// pitch'= 私有.pitch − dy·灵敏度
/// yaw   = floor-mod [0,360)；<0 抬 0，再 min(·,360)   // 与旋转律不同的归一
/// pitch = 先钳上界再抬下界（非对称序，与 UpdateAngle 同形）
/// ```
/// 界取共享模型（进入时已写 FPS 界，私有/共享同值）。
fn fps_drag(model: &mut FieldCameraModel, fps: &mut FpsViewMemory, dx: f32, dy: f32) {
    let raw_yaw = fps.yaw + model.rot_sensitivity * dx;
    let raw_pitch = fps.pitch - model.rot_sensitivity * dy;
    let floored = raw_yaw - (raw_yaw / 360.0).floor() * 360.0;
    let yaw = if floored < 0.0 {
        0.0
    } else {
        floored.min(360.0)
    };
    let mut pitch = if raw_pitch <= model.max_pitch {
        raw_pitch
    } else {
        model.max_pitch
    };
    if raw_pitch < model.min_pitch {
        pitch = model.min_pitch;
    }
    let (before_yaw, before_pitch) = (fps.yaw, fps.pitch);
    fps.yaw = yaw;
    fps.pitch = pitch;
    model.yaw = yaw;
    model.pitch = pitch;
    info!(
        "[camera-fps] FPS 旋转输入 Δ({:.1},{:.1})px → yaw {:.1}°→{:.1}° pitch {:.1}°→{:.1}°（界 [{:.1},{:.1}]，灵敏度 {}）",
        dx, dy, before_yaw, yaw, before_pitch, pitch, model.min_pitch, model.max_pitch,
        model.rot_sensitivity
    );
    if pitch != raw_pitch {
        info!(
            "[camera-fps] 俯仰触界：raw {:.1}° → {:.1}°（界 [{:.1},{:.1}]）",
            raw_pitch, pitch, model.min_pitch, model.max_pitch
        );
    }
}

/// 源 `FPSCameraState.OnEnter`（配上前一刻的 `NormalCameraState.OnExit`）
/// 的进入帧全序：快照双写 → 玩家隐 → 私有偏航捕获 → 取景点直写 →
/// 界钳 → 进入补间 → 换态。
///
/// 不迁（挂账，逐条见 [`apply_input`] 文档）：SetLock、SetActiveUI、
/// ShowAll、ResetHouseDither；玩家隐用可见性直写替代 dither 淡出。
#[allow(clippy::too_many_arguments)]
fn enter_fps(
    commands: &mut Commands,
    model: &mut FieldCameraModel,
    state: &mut FieldCameraState,
    active: &SiteActive,
    is_room: bool,
    player_pos: Vec3,
    mut player_visibility: Mut<Visibility>,
    fps: &mut FpsViewMemory,
    camera_fov_deg: f32,
) {
    // 进入帧快照 = 源 OnExit 的两份同刻双写：私有镜像 {LookAt, Distance,
    // Yaw, Pitch}（flag 置位时同步）与转场表 {pitch, yaw, fov, distance}
    // （无条件写）。距离此刻已钳在下界（进入条件的位精确相等）。
    commands.insert_resource(NormalCameraMemory {
        site: active.site_type.clone(),
        look_at: model.look_at,
        distance: model.distance,
        yaw: model.yaw,
        pitch: model.pitch,
        fov: model.fov,
    });
    let before = (
        model.min_distance,
        model.max_distance,
        model.min_pitch,
        model.max_pitch,
    );
    *player_visibility = Visibility::Hidden;
    // 偏航捕获：源读**相机本体** eulerAngles.y——相机朝向由模型偏航派生，
    // 取其 [0,360) 归一形在方向上是恒等操作（补间终点经最短有向角，差值
    // 被 floor-mod 归零）。俯仰**不重置**：源常驻单例的私有模型只在构造
    // 时写一次 0，再次进入的目标俯仰是上一会话拖拽后的残留值。
    fps.yaw = normalize360(model.yaw);
    // 取景点直写玩家位 + 高度偏移；界钳 = FPS 私有模型的四界（距离三界
    // 同值 0.15，俯仰按楼层室内/户外分 −17/−8，上界 75）。
    model.look_at = player_pos + FPS_CAMERA_HEIGHT_OFFSET;
    model.min_distance = FPS_DISTANCE;
    model.max_distance = FPS_DISTANCE;
    model.min_pitch = if is_room {
        FPS_INDOOR_MIN_PITCH
    } else {
        FPS_MIN_PITCH
    };
    model.max_pitch = FPS_MAX_PITCH;
    // 进入补间（源 DoTweenCameraSetting，0.2s 四分量 OutQuad）：起值构造
    // 时捕获——LookAt 已直写故首尾同值（不动），FOV 起自相机本体、终于
    // 模型 FOV（本次不动），旋转经最短有向角到私有镜像，距离 →0.15。
    // 这就是「FPS 态下继续放大」的全部：位置前移，FOV 不变。
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    commands.insert_resource(CameraTween {
        look_at: (model.look_at, model.look_at),
        fov: (camera_fov_deg, model.fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, fps.pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, fps.yaw)),
        distance: (model.distance, FPS_DISTANCE),
        duration: FPS_ENTER_TWEEN_SECS,
        elapsed: 0.0,
    });
    state.0 = CameraStateType::Fps;
    info!(
        "[camera-fps] 进入 FPS：距离 {:.2}→{:.2}（界 [{:.2},{:.2}]→[{:.2},{:.2}]，俯仰界 [{:.1},{:.1}]→[{:.1},{:.1}]），取景点 {}, 捕获偏航 {:.1}°→{:.1}°（残俯仰目标 {:.1}°），FOV {:.1}°→{:.1}°（不动），玩家隐",
        model.distance,
        FPS_DISTANCE,
        before.0,
        before.1,
        FPS_DISTANCE,
        FPS_DISTANCE,
        before.2,
        before.3,
        model.min_pitch,
        model.max_pitch,
        model.look_at,
        prev_yaw,
        fps.yaw,
        fps.pitch,
        camera_fov_deg,
        model.fov,
    );
}

/// 源 `NormalCameraState.IsInheritCameraSetting`（退出分道判据）：
/// 站点类目 > 1（harvest/delivery）⇒ 前态 ≠ 17；类目 ≤ 1（住宅）⇒
/// 前站点类型 ∈ {grassland, shore, flower_garden, memorial_place,
/// festival_garden}（源判 4 ≤ PrevSiteType ≤ 8，即五户外采集类站）。
///
/// 两条化简都由构造保证：本仓进入 Normal 的唯一写者是 FPS 退出（前态
/// 恒 16 ≠ 17 ⇒ 类目 > 1 时恒继承）；类目与站点类型串就是源枚举名
/// （主表照抄，产物核过）。住宅分叉不化简——它读的是站点搬迁史。
pub(crate) fn is_inherit_camera_setting(category: &str, prev_site_type: &str) -> bool {
    if matches!(category, "harvest" | "delivery") {
        true
    } else {
        matches!(
            prev_site_type,
            "grassland" | "shore" | "flower_garden" | "memorial_place" | "festival_garden"
        )
    }
}

/// 源 `FPSCameraState.OnExit`（配 `NormalCameraState.OnEnter`）的退出帧
/// 全序：玩家显 → 界钳/offset 从面板恢复 → 按站点分道的退出补间 →
/// 换态。继承支（0.5s）到转场表快照或 fallback；case 16 支（住宅常规，
/// 0.1s）取景点到玩家位、俯仰/距离回进入帧快照、偏航保持当前、FOV 回
/// 面板值。源教程分岔（IsTutorial）不迁——本仓无教程态，构造上恒走
/// TransferCameraSettings 等价支。
///
/// ⚠ 单槽快照的已知边界：源 OnExit 的私有镜像同步是 flag 门的（case 16
/// 转场补间在飞时 flag=0，跳过同步），转场表写则无条件——本仓单槽承载
/// 两语义、恒写。退出补间未完就再次进入的窗口（≤0.5s）内，下一次 case
/// 16 支读到的是「当刻模型值」而非「上一补间的目标值」，具名在案。
#[allow(clippy::too_many_arguments)]
fn exit_fps(
    commands: &mut Commands,
    model: &mut FieldCameraModel,
    state: &mut FieldCameraState,
    setting: &CameraSetting,
    active: &SiteActive,
    prev_site_type: &str,
    player_pos: Vec3,
    mut player_visibility: Mut<Visibility>,
    memory: Option<&NormalCameraMemory>,
    camera_fov_deg: f32,
) {
    *player_visibility = Visibility::Visible;
    // 界钳与 offset 恢复：源读 Normal 私有模型的界——该模型构造时整份
    // 拷贝共享模型后再无写点 ⇒ 恒为面板值（Setting）。
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let inherit = is_inherit_camera_setting(&active.category, prev_site_type);
    if inherit {
        // 继承支：站点命中的快照，否则源 fallback 形状（俯仰回构造体
        // initPitch，偏航/FOV/距离保持当前——不猜值）。取景点目标 = 当前
        // 值（源取 OnEnter 时刻的模型值，恒等）。
        let (pitch, yaw, fov, distance, hit) = match memory {
            Some(m) if m.site == active.site_type => (m.pitch, m.yaw, m.fov, m.distance, true),
            _ => (
                setting.init_pitch,
                model.yaw,
                model.fov,
                model.distance,
                false,
            ),
        };
        commands.insert_resource(CameraTween {
            look_at: (model.look_at, model.look_at),
            fov: (camera_fov_deg, fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, pitch)),
            yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
            distance: (model.distance, distance),
            duration: FPS_EXIT_TWEEN_SECS_INHERIT,
            elapsed: 0.0,
        });
        info!(
            "[camera-fps] 退出 FPS（继承支 {}s）：距离 {:.2}→{:.2}，俯仰 {:.1}°→{:.1}°，偏航 {:.1}°→{:.1}°，FOV {:.1}°→{:.1}°（{}），取景点不动 {}",
            FPS_EXIT_TWEEN_SECS_INHERIT,
            model.distance,
            distance,
            prev_pitch,
            to_rotation(prev_pitch, pitch),
            prev_yaw,
            to_rotation(prev_yaw, yaw),
            camera_fov_deg,
            fov,
            if hit { "转场表快照" } else { "fallback：initPitch+当前值" },
            model.look_at
        );
    } else {
        // case 16 支（住宅常规）：偏航目标 = 当前 FPS 偏航（源 private.Yaw
        // ← owner.Model.Yaw，恒等不动），取景点到玩家位（无高度偏移），
        // 俯仰/距离回进入帧快照（源私有镜像的 OnExit 双写），FOV 回面板
        // 值（源私有 FOV 自构造起未再写 = Setting）。快照缺席在构造上不可
        // 达（进入必写；站点切换在 Normal 态才发生）——响亮拒绝，不静默
        // 取默认。
        let Some(m) = memory else {
            panic!("FPS 退出缺转场快照：进入必写、站点切换不进 FPS，不可达");
        };
        commands.insert_resource(CameraTween {
            look_at: (model.look_at, player_pos),
            fov: (camera_fov_deg, setting.fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, m.pitch)),
            yaw: (prev_yaw, to_rotation(prev_yaw, model.yaw)),
            distance: (model.distance, m.distance),
            duration: FPS_EXIT_TWEEN_SECS_CASE16,
            elapsed: 0.0,
        });
        info!(
            "[camera-fps] 退出 FPS（case16 支 {}s）：距离 {:.2}→{:.2}，俯仰 {:.1}°→{:.1}°，偏航 {:.1}°（保持），FOV {:.1}°→{:.1}°，取景点 →玩家位 {}",
            FPS_EXIT_TWEEN_SECS_CASE16,
            model.distance,
            m.distance,
            prev_pitch,
            to_rotation(prev_pitch, m.pitch),
            prev_yaw,
            camera_fov_deg,
            setting.fov,
            player_pos
        );
    }
    state.0 = CameraStateType::Normal;
}

/// 前站点类型（源 `SiteManager.PrevSiteType` 的镜像）。站点域零跟踪、
/// 消费者只有相机域的退出分道，由相机域顺带维护。零初始化 = home_site
/// （源字段零值 0 = home_site）。
#[derive(Resource, Debug, Clone)]
pub struct PrevSiteType(pub String);

impl Default for PrevSiteType {
    fn default() -> Self {
        Self("home_site".to_owned())
    }
}

/// PreUpdate：站点类型变化时记下前值（源换站时 PrevSiteType ← 旧
/// CurrentSiteType）。SiteActive 经命令在 Update 内插入，PreUpdate 读到
/// 的是已落定值——prev 是跨站慢变量，滞后一帧不影响语义。首次观测只
/// 记当前，prev 保持零初始化（源零值语义）。
pub fn track_prev_site(
    active: Option<Res<SiteActive>>,
    mut prev: ResMut<PrevSiteType>,
    mut cache: Local<Option<String>>,
) {
    let Some(active) = active else {
        return;
    };
    let Some(seen) = cache.clone() else {
        *cache = Some(active.site_type.clone());
        return;
    };
    if seen != active.site_type {
        prev.0 = seen;
        *cache = Some(active.site_type.clone());
        info!(
            "[camera-fps] 站点搬迁：{} → {}（前站类型记账）",
            prev.0, active.site_type
        );
    }
}

/// PostUpdate：Normal 态跟随律（源 `NormalCameraState.OnUpdate` 逐行迁移），
/// 每帧推进 [`FieldCameraModel`] 并写相机变换。
///
/// 源式（原生反编译，IFix 补丁门之后的原生体）：
/// ```text
/// v13 = deltaTime / 0.016667                 // 帧时长归一（字面量如此）
/// t   = clamp(v13 * 0.1, 0, 1)               // 跟随插值系数
/// LookAt += (playerPos - LookAt) * t          // 三轴同插（x/y/z）
/// t2  = MinDistance != MaxDistance
///       ? clamp((Distance-Min)/(Max-Min), 0, 1) : 0
/// xRange = InterpRange(近界.min.x, 远界.min.x, 近界.max.x, 远界.max.x, t2)
///          // 内式：x = (minAtMin - minAtMax)·t2 + minAtMax
///          //      y = (maxAtMin - maxAtMax)·t2 + maxAtMax
/// zRange = 同式 z 分量
/// LookAt.x = clamp(LookAt.x, xRange)          // 先钳 max 再钳 min，Y 不钳
/// LookAt.z = clamp(LookAt.z, zRange)
/// eye = GetPosition(pitch, yaw, LookAt, Offset, Distance)   // 见 view_dir
/// 相机朝向 = LookAt + Offset（Transform.LookAt）
/// ```
/// 跟随目标是玩家 avatar 的视变换位置（源 `GetFollowPlayer` →
/// `AvatarDataStore` 槽 → `GetViewTransform().position`；我们的等价物是
/// 名册首行成员的世界变换——同一语义位）。
///
/// 转场补间（源 `DoTweenCameraSetting` 的四条并行 tween）在本函数头部推
/// 进：四分量一律 OutQuad，LookAt/俯仰/偏航/距离逐帧写模型，FOV 写
/// **相机投影**（源 setter 直写 `Camera.fieldOfView`），到头撤资源。补间
/// 在飞 ⇒ 跟随系数 v13 = 1.0（t 恒 0.1/帧）；补间走完且空闲才走 dt 归一
/// 支（源 OnUpdate 的门：`_isCompleteCameraTween && !CurrentAction.IsPlaying`）。
/// 跟随律与补间**同帧叠加**而非让位——源同形（tween 写 LookAt 后 OnUpdate
/// 仍把取景点往玩家拉 10%）。
///
/// FPS 态律（源 `FPSCameraState.OnUpdate` 逐行）：取景点直写玩家位 + 高度
/// 偏移（无插值无钳界——跟随是 Normal 态的事）；眼位沿视线退 FPS 距离，
/// 相机朝向取景点（源 UpdatePosition + UpdateRotation；我们的 `looking_at`
/// 是精确式，源 FromEulerRad 的字面量 0.017453 截断不迁移）；玩家朝向 =
/// 相机前向的三维 LookRotation（随俯仰低头抬头；FPS 内玩家隐藏、退出后
/// 保留残姿态）。
///
/// FPS 的 follow 不是另一条链，是两态**共享**的 `FieldCamera.UpdatePosition`
/// 轨道（方法体无态分支：读 model 的 Pitch/Yaw/LookAt/Offset/Distance →
/// GetPosition → 写相机位）。两态的差别全在进轨道**之前/之后**：LookAt
/// 怎么来（Normal：系数 `min(δt/0.016667·0.1, 1)` 平滑拉向玩家 + 视距内
/// 插界钳 x/z；FPS：玩家位+0.1 硬直写，无平滑无钳界）与相机朝向怎么写
/// （Normal：`Transform.LookAt(LookAt+Offset)`；FPS：pitch/yaw 四元数）。
/// Normal 态另有的房屋 dither 与采集站相机碰撞，16 态都没有。
///
/// 相机侧不存在头骨/玩家朝向锚点：取景点读的是玩家视图**根**位置
/// （`PlayerAvatarPresenter.get_Position` = 视变换 position，非任何骨骼），
/// 头/脊柱字段全树无相机域写者。玩家→相机方向也无写：相机朝向从不读
/// 玩家面朝。
///
/// 玩家←相机的那支写（上面的面朝 LookRotation）是同帧双写里注定赢的一
/// 支：源里同一视图变换每帧被两处写——移动态在 Update 相位
/// （`AvatarDataStore.Update` → `PlayerAvatarPresenter.OnUpdate` →
/// `PlayerAvatarStateMachine.OnUpdate` → `PlayerAvatarMoveState.UpdateState`
/// 每帧写 `Euler(0, atan2(输入.x, 输入.z), 0)`），相机态尾写在
/// LateUpdate 相位（`FieldCamera.LateUpdate` → 状态机 → OnUpdate 尾段）。
/// Unity 帧序 Update 先于 LateUpdate ⇒ 相机写后落：16 态下躯干朝向恒跟
/// 相机走，行走中也压住输入朝向（对不上前几版记录里「移动时玩家朝向照
/// 常走自己的」的猜测——那支写在源里存在、但被相位压住，不出现在画面
/// 上）。本仓同一次序：`player::advance` 在 Update 写移动朝向，本系统在
/// PostUpdate 写相机前向，同一赢家。
pub(crate) fn follow_avatar(
    time: Res<Time>,
    mut commands: Commands,
    state: Res<FieldCameraState>,
    models: Option<ResMut<FieldCameraModel>>,
    mut tween: Option<ResMut<CameraTween>>,
    // 两查询都持 &mut Transform：互加 Without 声明不相交（相机不是
    // avatar、avatar 不是相机），否则 Bevy 借用检查按「可能同实体」拒绝。
    mut avatars: Query<(&GlobalTransform, &mut Transform), (With<AvatarRoot>, Without<Camera3d>)>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<Camera3d>, Without<AvatarRoot>)>,
    mut announced: Local<bool>,
    mut fps_announced: Local<bool>,
    talk: Option<Res<crate::talk::ActiveTalk>>,
    player_talk: Option<Res<crate::player_talk::PlayerTalkSession>>,
    talk_camera: Option<Res<crate::talk_camera::TalkCamera>>,
    // 16 态面朝追迹的 0.5s 累计器（AUTOFPS 冒烟窗口内用）。
    mut face_trace: Local<f32>,
) {
    let Some(mut models) = models else {
        return; // 站点未取景（或 JSON 未装）：模型未立，等下一帧
    };
    // A player conversation owns the camera, including its entry tween.
    // Normal following resumes on the actual session's completion frame;
    // it must not tug at a conversation target before the Talk writer runs.
    if talk
        .as_deref()
        .is_some_and(crate::talk::ActiveTalk::includes_player)
        || player_talk.is_some()
    {
        return;
    }
    // --- 转场补间推进（任意态在飞）---
    let mut tween_active = talk_camera.is_some();
    if let Some(tw) = tween.as_deref_mut() {
        tw.elapsed += time.delta_secs();
        let e = (tw.elapsed / tw.duration).clamp(0.0, 1.0);
        let k = out_quad(e);
        models.look_at = tw.look_at.0.lerp(tw.look_at.1, k);
        models.pitch = tw.pitch.0 + (tw.pitch.1 - tw.pitch.0) * k;
        models.yaw = tw.yaw.0 + (tw.yaw.1 - tw.yaw.0) * k;
        models.distance = tw.distance.0 + (tw.distance.1 - tw.distance.0) * k;
        let fov_deg = tw.fov.0 + (tw.fov.1 - tw.fov.0) * k;
        if let Ok((_, mut projection)) = cameras.single_mut() {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = fov_deg.to_radians();
            }
        }
        if e >= 1.0 {
            info!(
                "[camera] 转场补间完成（{:.2}s）：距离 {:.2}→{:.2}，俯仰 {:.1}°，偏航 {:.1}°，FOV {:.1}°，取景点 {}",
                tw.duration,
                tw.distance.0,
                tw.distance.1,
                models.pitch,
                models.yaw,
                fov_deg,
                models.look_at
            );
            commands.remove_resource::<CameraTween>();
        } else {
            tween_active = true;
        }
    }
    let Ok((avatar, mut avatar_transform)) = avatars.single_mut() else {
        return;
    };
    let player = avatar.translation();
    match state.0 {
        CameraStateType::Fps => {
            // FPS 态律：取景点直写玩家位+高度偏移（无插值无钳界），眼位
            // 沿视线退 FPS 距离，相机朝向取景点——两态共享同一条轨道，
            // 详见函数头「FPS 的 follow 不是另一条链」段。
            models.look_at = player + FPS_CAMERA_HEIGHT_OFFSET;
            let pivot = models.look_at + models.offset;
            let eye = pivot + view_dir(models.pitch, models.yaw) * models.distance;
            if let Ok((mut camera, _)) = cameras.single_mut() {
                *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
                if !*fps_announced {
                    *fps_announced = true;
                    info!(
                        "[camera-fps] FPS 态跟随律启动：取景点 {pivot:.2} 眼位 {eye:.2}（距离 {:.2}，俯仰界 [{:.1},{:.1}]°）",
                        models.distance, models.min_pitch, models.max_pitch
                    );
                }
            }
            // 玩家朝向 = 相机前向的 LookRotation（源 OnUpdate 尾段；Unity
            // 基构造：z = forward、x = up×z 归一、y = z×x；本仓 avatar
            // 面朝局部 +Z，同位）。⚠ 别把这支写当「盖掉移动律的错」摘掉：
            // 它是 Update/LateUpdate 双写里注定赢的一支（证据与次序见函数
            // 头注释），摘掉后行走面朝会钉死在输入方向上，恰与源相反。
            let forward = (pivot - eye).normalize_or_zero();
            if forward != Vec3::ZERO {
                let right = Vec3::Y.cross(forward).normalize_or_zero();
                let up = forward.cross(right);
                avatar_transform.rotation = Quat::from_mat3(&Mat3::from_cols(right, up, forward));
                // 面朝追迹（AUTOFPS 冒烟窗口内每 0.5s 一行，无头验收
                // 「移动中拖拽」拍）：相机偏航与玩家面朝应当**同变**
                // （本仓约定面朝 = −相机偏航），位置应当照常走（移动写
                // 没被摘、只是被相位压住）。
                let trace_window = env_secs("MOLY_CAMERA_AUTOFPS_SECS");
                if trace_window > 0.0 && time.elapsed_secs() < trace_window {
                    *face_trace += time.delta_secs();
                    if *face_trace >= 0.5 {
                        *face_trace = 0.0;
                        let facing_yaw = forward.x.atan2(forward.z).to_degrees();
                        info!(
                            "[camera-fps] 16 态面朝追迹：相机 yaw {:.1}° → 玩家面朝 {:.1}°，位置 ({:.2},{:.2})，距离 {:.2}",
                            models.yaw, facing_yaw, player.x, player.z, models.distance
                        );
                    }
                }
            }
        }
        _ => {
            // 源式的时间归一：补间在飞 ⇒ v13 = 1.0（t 恒 0.1/帧），补间
            // 走完且空闲才 dt / 0.016667，再乘 0.1 钳 [0,1]。
            let raw = if tween_active {
                1.0
            } else {
                time.delta_secs() / FRAME_BASE
            };
            let t = (raw * FOLLOW_RATE).clamp(0.0, 1.0);
            // 先取插值界与缩放参数（避免与 LookAt 的可变借用交叉）。
            let (min_distance, max_distance, distance) =
                (models.min_distance, models.max_distance, models.distance);
            let near_min = models.look_at_bounds.min();
            let near_max = models.look_at_bounds.max();
            let far_min = models.max_look_at_bounds.min();
            let far_max = models.max_look_at_bounds.max();
            // 缩放插值参数 t2（源式；等距时 0）。
            let t2 = if min_distance != max_distance {
                ((distance - min_distance) / (max_distance - min_distance)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            // X/Z 界按 t2 在近界与远界之间插值（源 GetInterpolatedTrackingRange）。
            let x_range = Vec2::new(
                (near_min.x - far_min.x) * t2 + far_min.x,
                (near_max.x - far_max.x) * t2 + far_max.x,
            );
            let z_range = Vec2::new(
                (near_min.y - far_min.y) * t2 + far_min.y,
                (near_max.y - far_max.y) * t2 + far_max.y,
            );
            let look_at = &mut models.look_at;
            look_at.x += (player.x - look_at.x) * t;
            look_at.y += (player.y - look_at.y) * t;
            look_at.z += (player.z - look_at.z) * t;
            // 钳界：源序是先 max 后 min（fmin/fmax 链），Y 不钳。
            look_at.x = look_at.x.min(x_range.y).max(x_range.x);
            look_at.z = look_at.z.min(z_range.y).max(z_range.x);
            // 眼位（源 GetPosition，式子见 view_dir 注释）。
            let pivot = *look_at + models.offset;
            let eye = pivot + view_dir(models.pitch, models.yaw) * models.distance;
            if let Ok((mut camera, _)) = cameras.single_mut() {
                *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
                // 一次性状态行：真源跟随律启动的现算证据（此后每帧推进，不再刷屏）。
                if !*announced {
                    *announced = true;
                    info!(
                        "[camera] 相机改真源跟随律：取景点 {pivot:.2} 眼位 {eye:.2}（目标=avatar 视变换，t 系数 0.1/帧基 {FRAME_BASE}）"
                    );
                }
            }
        }
    }
}

/// PostUpdate（定时）：跟随模式的周期状态行。取景点、眼位与界半径随帧
/// 推进是「相机真的在按源律跑」的现算证据——眼位可用 `view_dir` 手算对回。
pub fn report_follow(
    models: Option<Res<FieldCameraModel>>,
    avatars: Query<(&GlobalTransform, &CharacterShell), With<AvatarRoot>>,
    cameras: Query<&Transform, With<Camera3d>>,
) {
    let Some(models) = models else {
        return; // 模型未立（站点未取景）：无账可报
    };
    let Ok((avatar, shell)) = avatars.single() else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    // 手算眼位（与 follow_avatar 同式）：报告行可从模型量复算。
    let pivot = models.look_at + models.offset;
    let expected = pivot + view_dir(models.pitch, models.yaw) * models.distance;
    info!(
        "[camera] 跟随：目标 {:.2} 取景点 {pivot:.2} 眼位 {:.2}（手算 {expected:.2}）界半径 {:#?} 外壳高 {:.2}",
        avatar.translation(),
        camera.translation,
        models.look_at_bounds.extents,
        shell.height
    );
}

#[cfg(test)]
mod overlay_sample_tests {
    use super::*;
    #[test]
    fn source_sprite_overlay_cameras_share_the_field_sample_count() {
        let mut app = App::new();
        app.add_systems(Startup, (crate::balloon::overlay_camera, crate::sitemap::overlay_camera));
        app.update();
        let mut cameras = app.world_mut().query_filtered::<(&Camera, &Msaa), With<Camera2d>>();
        let cameras = cameras.iter(app.world()).collect::<Vec<_>>();
        assert_eq!(cameras.len(), 2);
        for (camera, msaa) in cameras {
            assert!(matches!(camera.clear_color, ClearColorConfig::None));
            assert_eq!(*msaa, MYSEKAI_CAMERA_MSAA);
            assert_eq!(msaa.samples(), 1);
        }
    }
}
