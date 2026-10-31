//! Site framing and camera follow: the site camera rules of the `FieldCamera` family.
//!
//! Two layers:
//! - **Loading**: `camera/out/mysekai__camera.json` (JsonAsset channel) parses into
//!   `CameraSetting` (static camera placement parameters) and `CameraParamAsset`
//!   (seven AnimationCurve curves).
//!   The curves have no consumer in this repository: the only name references to
//!   `CameraParam` are inside FieldCamera/FieldCameraModel itself, and the model
//!   fields have zero read sites across every file that holds the model (a
//!   three-round exhaustive search by offset and by name, with a positive
//!   control) ⇒ camera motion is **not** driven by these curves: normal follow
//!   runs the per-frame Normal-state rules (no curves), scripted sequences use
//!   the clip curves carried by Timeline's FieldCamera*Behaviour, and transport
//!   uses constant interpolation dispatched by animation name. The load layer
//!   still read them and the curve values go to the log for recomputation; if a
//!   consumer ever exists (for example through a runtime patch), the data is
//!   already on disk.
//! - **Follow rules**: `follow_avatar` advances a mirrored model
//!   ([`FieldCameraModel`]) with the per-frame rules of NormalCameraState.OnUpdate;
//!   the eye formula and the clamp order follow that routine line for line (the
//!   formulas are commented next to the methods).
//! - **FPS state**: pinching in again at the minimum distance enters first person
//!   (the whole `CanSwitchToFpsMode` chain plus the FPS-state OnEnter/OnUpdate/OnDrag
//!   trio); pinching outward in the FPS state exits back to Normal (the inherit
//!   branch / case 16 branch, dispatched by site category).
//!   Transitions always use four-component OutQuad tweens (matching
//!   `DoTweenCameraSetting`).
//!   ⚠ The UI and dither-family calls in OnEnter/OnExit are **not** carried over
//!   (this repository has no such systems); each one is accounted for in the
//!   [`apply_input`] comment.
//!
//! The comments on the near/far clip planes and the view frustum live in
//! [`spawn`]. The curve evaluation formula is documented on
//! [`CurveAsset::evaluate`]; it is the measurement that can be recomputed value
//! by value, and expressing the beat rules goes to the next change.

use crate::character::{AvatarRoot, CharacterShell};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::inactive_nodes::SiteSettled;
use crate::site::{GroundMeshes, SiteActive, SiteSelection};
use bevy::asset::{AssetPath, LoadState};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::ecs::system::{SystemParam, SystemState};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

/// Every compositing helper must match the current single-sample field target.
/// Bevy keys per-view colour allocations by MSAA as well as output/format; a
/// default 4x no-clear overlay is therefore NOT the same target as the 1x scene.
/// Actual GLES readbacks observed valid scene/post pixels replaced at that
/// overlay boundary. This is a renderer-stack compatibility invariant, not a
/// claim that current source runtime quality writes have all been excluded.
pub const MYSEKAI_CAMERA_MSAA: Msaa = Msaa::Off;

/// 取景中心附近的地表高度采样半径：取「脚下的地面」。
const SURFACE_RADIUS: f32 = 2.0;

/// Frame-duration normalisation base of NormalCameraState.OnUpdate (the literal
/// 0.016667, not the exact value of 1/60; every literal is matched).
pub const FRAME_BASE: f32 = 0.016667;

/// 源 NormalCameraState.OnUpdate 的跟随插值系数：t = clamp((dt/基)·0.1, 0, 1)。
const FOLLOW_RATE: f32 = 0.1;

/// 源 Normal 态构造体的三个字面量：拖拽俯仰联动的保底距离，与
/// 「俯仰下限随缩放插值」的两端（角度制）。
const GESTURED_MIN_DISTANCE: f32 = 4.5;
const MIN_PITCH_AT_MAX_ZOOM_FROM: f32 = 8.0;
const MIN_PITCH_AT_MAX_ZOOM_TO: f32 = 24.0;

/// Scroll-wheel one notch in pinch pixels. The gesture layer is touch based and
/// measured in pixels; the wheel is a PC-only input, so this value is a
/// hand-feel constant, not a quantity of the game.
pub const WHEEL_PINCH_PIXELS: f32 = 50.0;

/// 截图用的自由机位：设着时 [`apply_free_camera`] 每帧覆盖场地相机的
/// `Transform` 与 `Projection`。`moly.camera.free` 写入，`moly.camera.reset`
/// 清掉；下一帧玩家跟随相机自然接管（跟随系统在 PostUpdate 已写好）。
#[derive(Resource, Default)]
pub struct FreeCameraPose(pub Option<FreeCameraPoseValue>);

#[derive(Clone, Copy, Debug)]
pub struct FreeCameraPoseValue {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_degrees: f32,
}

/// `Last`：在所有相机态（跟随、过场、编辑、交付）都写完场地相机之后执行，
/// 因此自由机位对这一帧的渲染生效，而不被同帧的跟随律覆盖。
pub fn apply_free_camera(
    pose: Res<FreeCameraPose>,
    mut camera: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let Some(free) = &pose.0 else { return };
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    *transform = Transform::from_translation(free.eye).looking_at(free.target, free.up);
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = free.fov_degrees.to_radians();
    }
}
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

const PHOTO_MIN_PITCH: f32 = -15.0;
const PHOTO_MAX_PITCH: f32 = 75.0;
const PHOTO_MIN_FOV: f32 = 30.0;
const PHOTO_MAX_FOV: f32 = 60.0;

#[derive(Resource)]
struct PhotoCameraMemory { model: FieldCameraModel, near_planes: Vec<(Entity, Vec4)>, player: Entity, visibility: Visibility }

fn photo_enter(world: &mut World) {
    let player = world.query_filtered::<(Entity,&Transform,&Visibility), With<AvatarRoot>>().iter(world).next().map(|(e,t,v)|(e,*t,*v));
    let Some((player_entity,player,visibility)) = player else { return; };
    let Some(mut model) = world.get_resource::<FieldCameraModel>().cloned() else { return; };
    let previous = model.clone();
    let forward = player.rotation * Vec3::Z;
    model.yaw = (-forward.x).atan2(forward.z).to_degrees().rem_euclid(360.0);
    model.pitch = 0.0; model.fov = 45.0; model.offset = Vec3::ZERO;
    model.distance = 2.0; model.min_distance = 1.0; model.max_distance = 5.0;
    model.min_pitch = PHOTO_MIN_PITCH; model.max_pitch = PHOTO_MAX_PITCH;
    model.look_at = player.translation + Vec3::new(forward.x,0.5,forward.z);
    let candidate = model.look_at + view_dir(model.pitch,model.yaw) * model.distance;
    if let Some(face) = world.get_resource::<crate::npc_objective::ObjectiveFace>() {
        let origin=Vec3::new(player.translation.x,0.5,player.translation.z);
        if let Some(hit)=face.navigation_ray_hit(origin,candidate) {model.distance=origin.distance(hit);}
    }
    let mut near_planes=Vec::new();
    let mut cameras=world.query_filtered::<(Entity,&mut Projection),With<Camera3d>>();
    for (entity,mut projection) in cameras.iter_mut(world) {
        if let Projection::Perspective(p)=&mut *projection {
            near_planes.push((entity,p.near_clip_plane)); p.near_clip_plane=Vec4::new(0.0,0.0,-1.0,-0.01); p.fov=model.fov.to_radians();
        }
    }
    world.entity_mut(player_entity).insert(Visibility::Hidden);
    world.insert_resource(PhotoCameraMemory {model:previous,near_planes,player:player_entity,visibility});
    world.insert_resource(model);
    world.remove_resource::<CameraTween>();
    world.resource_mut::<FieldCameraState>().0=CameraStateType::PhotoShot;
}

fn photo_exit(world: &mut World) {
    let Some(memory)=world.remove_resource::<PhotoCameraMemory>() else {return;};
    if let Ok(mut player)=world.get_entity_mut(memory.player) {player.insert(memory.visibility);}
    for (entity,near) in memory.near_planes {
        if let Some(mut projection)=world.get_mut::<Projection>(entity) {
            if let Projection::Perspective(p)=&mut *projection {p.near_clip_plane=near;}
        }
    }
    world.insert_resource(memory.model);
}

fn photo_drag(model:&mut FieldCameraModel,delta:Vec2) {
    let eye=model.look_at+model.offset+view_dir(model.pitch,model.yaw)*model.distance;
    model.yaw=(model.yaw+delta.x*model.rot_sensitivity).rem_euclid(360.0);
    model.pitch=(model.pitch-delta.y*model.rot_sensitivity).clamp(PHOTO_MIN_PITCH,PHOTO_MAX_PITCH);
    model.look_at=eye-view_dir(model.pitch,model.yaw)*model.distance;
}

/// 转场补间时长（源字面量）：FPS 进入 0.2s；退出是 Normal 的 OnEnter——
/// 继承支 0.5s（[`INHERIT_TWEEN_SECS`]），否则 TransferCameraSettings case
/// 16 支 0.1s。缓动一律 OutQuad（源 EASE_BASIC = 6，DG.Tweening.Ease 枚举
/// 位序）。
const FPS_ENTER_TWEEN_SECS: f32 = 0.2;
const FPS_EXIT_TWEEN_SECS_CASE16: f32 = 0.1;

/// Field camera states (the type of `FieldCamera.CurrentState`: a closed set of
/// 21 values, discriminant values kept as-is; the spellings FPS /
/// ScreenshotCaptureFPS are written Fps / ScreenshotCaptureFps by Rust
/// convention). State transitions are driven by the game states and scripted
/// sequences; this repository implements one of them: Normal ↔ FPS is driven by
/// the pinch gesture (pinching in further at the minimum distance enters, and
/// an outward pinch in the FPS state exits, see [`apply_input`]); the other
/// states have no writer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[derive(Default)]
pub enum CameraStateType {
    None = 0,
    #[default]
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
    BulkHarvest = 21,
}

/// Current field camera state (the read/write surface of
/// `FieldCamera.CurrentState`). The constructor starts it at `None` and the
/// inbound-site flow switches it to `Normal`; this repository starts at
/// `Normal` (the equivalent of an always-resident "inbound complete" state),
/// and the writer is the FPS enter/exit transition of the pinch gesture. The
/// movement rules read it: in the FPS state the player walk speed is halved
/// (the `player` domain samples it every frame, the same for walk and sprint).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FieldCameraState(pub CameraStateType);


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

/// The transfer entry the last `NormalCameraState.OnExit` wrote (with the
/// live LookAt), under the site it left. Normal's own state is
/// [`NormalCameraPrivate`], [`NormalTransferData`] (every site's entry) and
/// [`NormalCameraTweenComplete`]; this single slot stays for its readers
/// outside the camera (the first-person lens baseline, the talk and entry
/// flows, the library's camera snapshot).
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
    /// The Sequence's OnComplete callback.
    pub on_complete: TweenCompletion,
}

/// What a camera tween's completion callback does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TweenCompletion {
    /// No callback (the transitions pass none).
    None,
    /// `NormalCameraState.ResetCameraSetting`'s callback: the state's own
    /// model mirror takes the setting distance and the reset flag clears.
    NormalReset { distance: f32 },
    /// `HarvestToneCameraState.ResetCameraSetting`'s callback: the tone
    /// state's private model ([`HarvestToneModel`]) takes the setting
    /// distance. It writes neither the shared model nor Normal's mirror, and
    /// the tone state has no reset flag.
    HarvestToneReset { distance: f32 },
    /// The callback of `NormalCameraState.OnEnter`'s inherit branch: Normal's
    /// private distance ([`FieldCameraModel::gestured_distance`]) takes the
    /// transfer entry's distance.
    NormalInherit { distance: f32 },
    /// The callback of `NormalCameraState.TransferCameraSettings`:
    /// `_isCompleteCameraTween` is set again ([`NormalCameraTweenComplete`]).
    NormalTransfer,
}

/// `HarvestToneCameraState._model`: the tone state's private copy of the
/// field camera's model, taken when the camera's state machine is built,
/// right after the camera builds its model from the setting (so it holds the
/// setting's values and a zero LookAt). The state's OnUpdate looks at the
/// shared LookAt plus this copy's offset; the camera reset writes this copy's
/// pitch bounds and its completion this copy's distance. Nothing reads those
/// three afterwards.
#[derive(Resource, Debug, Clone)]
pub struct HarvestToneModel(pub FieldCameraModel);

/// `HarvestToneCameraState._preCameraModel`: the copy of the field camera's
/// model that the tone state's OnEnter takes (the copy constructor carries
/// LookAt, offset, FOV, the distances, yaw, pitch and the pitch bounds). The
/// tone state's camera reset tweens back to it. Written on the frame the
/// state becomes HarvestTone and removed when the state is left.
#[derive(Resource, Debug, Clone)]
pub struct HarvestTonePreModel(pub FieldCameraModel);

impl CameraTween {
    /// `NormalCameraState._isResetAnimation`: set when the reset starts,
    /// cleared by its completion (or by leaving the Normal state, which here
    /// removes the tween).
    pub fn is_normal_reset(&self) -> bool {
        matches!(self.on_complete, TweenCompletion::NormalReset { .. })
    }
}

/// 相机 JSON 的装载请求；解析成功后即撤。内部形参带私有资源，故
/// pub。
#[derive(Resource)]
pub struct CameraJsonHandle(Handle<JsonAsset>);

/// Mirror of `CameraSetting` (serialized fields): static camera placement parameters.
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

/// Loaded mirror of `CameraParam` (a ScriptableObject with seven AnimationCurve curves).
/// Each curve's keys and values are kept unchanged; evaluation is in
/// [`CurveAsset::evaluate`].
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

/// Field-by-field mirror of `FieldCameraModel` (all state of the follow rules).
///
/// The initial values come from `FieldCameraModel..ctor(CameraSetting, CameraParam)`:
/// offset / pitch / the distance bounds / the pitch bounds / sensitivity / FOV
/// are all read straight from the serialized values; in the constructor
/// `LookAt` starts as the Vector3.zero static field (the constructor does not
/// write it) — on site load the `SetupLockAtCameraBounds` family writes its
/// coordinates and bounds directly, and once the Normal state is entered the
/// follow rules advance it every frame.
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

/// Startup: requests loading of the camera JSON; a single camera, fov is the
/// vertical field of view (same meaning).
/// Site colours are final values written by the material programs: both
/// tonemapping and debanding are off, no second processing layer is applied.
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
            // 3D audio listener: the ear pair lies on the listener's local X axis; its
            // position/orientation follow this entity's global transform every frame
            // (engine-side semantics). The camera carries an unconditional, always
            // resident 3D listener here — the current two audio channels (BGM, area
            // ambience) play flat 2D, do not consume it; it is the listening surface
            // reserved for the future family of 3D one-shot SEs.
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
/// 带私有资源，故 pub。
pub fn parse(
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
    // Frustum update: the SetupModel family applies the FOV to the camera.
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
    // The state machine is built right after SetupModel: the tone state's
    // and Normal's private models are copies of the model the setting just
    // built. The product starts in Normal as if its OnEnter had run.
    commands.insert_resource(HarvestToneModel(FieldCameraModel::from_setting(&setting)));
    commands.insert_resource(NormalCameraPrivate::from_setting(&setting));
    commands.insert_resource(NormalCameraTweenComplete(true));
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
///   navMesh 场变换位置（站点原点），界半径 = navMesh 场 `Bounds` 缩不可见
///   格数×格值；`Bounds` 是烘焙体积减 agent 半径，见
///   [`crate::site::Sites::nav_mesh_volume`]，格值 0.25 与源常量同值）。
pub fn frame_site(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    ground: Option<Res<GroundMeshes>>,
    settled: Option<Res<SiteSettled>>,
    active: Option<Res<SiteActive>>,
    setting: Option<Res<CameraSetting>>,
    config: Option<Res<crate::client_config::ClientConfigs>>,
    sites: Option<Res<crate::site::Sites>>,
    parts: Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
    entities: Query<Entity>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    // A cannon move keeps the camera model it flies with (SiteMoveAction
    // owns it); the new site only contributes its tracking bounds.
    (site_move, mut existing): (
        Option<Res<crate::field_modes::SiteMoveActive>>,
        Option<ResMut<FieldCameraModel>>,
    ),
) {
    let (Some(ground), Some(_), Some(active), Some(setting), Some(config), Some(sites)) =
        (ground, settled, active, setting, config, sites)
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
    // 半径 = fmax(全径 − 格值×不可见格数, 0)·0.5；全径 = 2 × navMesh 场
    // `Bounds` 半径，远界半径即 `Bounds` 半径）。不可见格数是面板键
    // （InvisibleGridCount，IntConfigs 键 69）。
    let mut model = FieldCameraModel::from_setting(&setting);
    let invisible_grid_count = config.int(crate::client_config::KEY_INVISIBLE_GRID_COUNT) as f32;
    let volume = sites
        .nav_mesh_volume(&active)
        .unwrap_or_else(|reason| panic!("[camera] 取景界无法确定（{site}）：{reason}"));
    let bounds_center = Vec2::from(volume.centre());
    let full_extents = Vec2::from(volume.field_half_extents()) * 2.0;
    let shrink = moly_law::fixture::position::TILE_SIZE * invisible_grid_count;
    let near_extents = Vec2::new(
        (full_extents.x - shrink).max(0.0) * 0.5,
        (full_extents.y - shrink).max(0.0) * 0.5,
    );
    model.look_at_bounds = BoundsXz {
        center: bounds_center,
        extents: near_extents,
    };
    model.max_look_at_bounds = BoundsXz {
        center: bounds_center,
        extents: full_extents * 0.5,
    };
    if site_move.is_some() {
        if let Some(existing) = existing.as_deref_mut() {
            // SetupLockAtCameraBounds of the destination; the model itself
            // is the one the move is flying (Normal re-entry restores the
            // rest from the setting when the move ends).
            existing.look_at_bounds = model.look_at_bounds;
            existing.max_look_at_bounds = model.max_look_at_bounds;
            info!(
                "[camera] {site} tracking bounds set during a cannon move: centre ({:.2},{:.2}) near {:?} far {:?}",
                bounds_center.x, bounds_center.y, near_extents, full_extents * 0.5
            );
            commands.remove_resource::<SiteSettled>();
            return;
        }
    }
    // 初始取景点：站点中心（LookAt 起零后第一次跟随会拉向角色，这里给
    // 首帧一个站点内的合法起点）。
    model.look_at = Vec3::new(center.x, surface, center.y);
    // Placement form: yaw/pitch turn into the view direction, the target point is
    // raised by offset, and the eye position steps back distance along it.
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
        bounds_center.x,
        bounds_center.y,
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
pub fn view_dir(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
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

/// Update: PC input feeds the follow model — left-button drag = one-finger touch
/// drag (`OnDrag` semantics), wheel = touch pinch (`OnPinch` semantics). Drags
/// come from the gesture layer (`gesture.rs`: DRAG activates only after the
/// accumulated movement passes the drag threshold, and then emits Moved every
/// frame — the gesture manager forwards only the UPDATE phase, so small
/// movements before activation are swallowed). Both input families are
/// dispatched by the current camera state (`FieldCamera.OnDrag/OnPinch` forward
/// to the same-named method of `CurrentState`).
/// While a reset tween is playing both families are swallowed entirely (the
/// `_isResetAnimation` gate at the entry of `OnDrag`/`OnPinch` in the Normal
/// state, checked before `UpdateAngle` and `CanSwitchToFpsMode` — during a reset
/// a pinch neither zooms nor enters FPS. The gate reads whether the in-flight
/// [`CameraTween`] carries a reset completion callback
/// ([`CameraTween::is_normal_reset`]); a reset starts only from the Normal state
/// ([`CameraReset::reset_camera_setting`]), and `ResetCameraSetting` is an empty
/// method in the other states).
/// The Normal-state rules, one by one:
///
/// - **Rotation** (from `FieldCamera.UpdateAngle`):
///   ```text
///   yaw'  = yaw + sensitivity·dx      // fmodf 360 only when leaving ±360
///   pitch'= pitch − sensitivity·dy
///   pitch = clamp to upper bound, then raise to lower bound
///                                     // asymmetric order: min(v, max) then max(·, min)
///   ```
/// - **Drag-pitch coupling** (tail of `OnDrag` in the Normal state): `t` uses the
///   pitch **just written**, effective distance = min(4.5 + t·(max distance−4.5),
///   gesture-remembered distance) — a low pitch (looking down) forces a close
///   pull, a bird's-eye view releases it; only the effective distance is
///   clamped, the gesture-remembered distance is not touched.
/// - **Zoom** (`OnPinch` in the Normal state): the zoom delta v13 = ratio·(−pinch)
///   is computed first and fed to [`can_switch_to_fps`] — **checked on every
///   pinch event**; if the distance is already at the lower bound (bit-exact
///   equality) and the direction is zoom-in, switch to the FPS state and **stop
///   zooming this frame**; otherwise distance = clamp(distance + v13, distance
///   bounds), with the gesture-remembered distance and the effective distance
///   **written together**; then the pitch lower bound is raised to
///   `max(8 + t₂·16 − ratio₂, 8, model pitch lower bound)`, where `t₂` is
///   normalised by the new distance.
///
/// FPS state: drags go through [`fps_drag`] (yaw floor-mod normalisation + pitch
/// clamping, from `FPSCameraState.OnDrag`); a pinch has **only the exit branch**
/// (v13 > 0 = pinch-out direction, i.e. back to Normal; `FPSCameraState.OnPinch`
/// has no zoom branch — "zoom in further" in the FPS state is the entry
/// transition itself: the position moves forward from distance 1.7 to 0.15 while
/// the FOV stays fixed).
/// The Moly close-inspection extension allows the wheel to narrow the FOV after
/// entering FPS; the FOV is restored first, then the exit happens.
/// Enter/exit transitions go through [`change_state`]: Normal's OnExit +
/// [`fps_on_enter`], then [`fps_on_exit`] + [`normal_on_enter`]
/// (TransferCameraSettings case 16 or the inherit branch).
///
/// Not carried over (each accounted for): the two-finger drag is an empty method
/// in both states; FPS OnEnter's `SetLock(0)` / `SetActiveUI(0)` /
/// `SiteObjectManager.ShowAll(0)` / `ResetHouseDither(1)`, OnExit's
/// `SetActiveUI(1)`, Normal OnEnter's `FadeInPlayerIfNeeded` /
/// `HideObstacleListIfNeeded` and OnExit's `SetDitherValue` — this repository has
/// no lock, UI-surface, house-hiding or dither systems, so player visibility is
/// switched by writing `Visibility` directly instead of a dither fade. The player
/// behaviour states in the entry guard (UseTimelineFixture/Harvest) have no state
/// machine here and always pass — a talk blocks entry only by switching the
/// camera state to Talk, and the talk camera is not part of this repository's
/// state set. Site switching during the FPS state is undefined (site transport
/// goes through camera states 6/7, which structurally leave FPS first; here
/// `frame_site` rebuilds the model without reverting the state — left for a
/// separate change, named here).
/// The inputs of the FPS enter/exit transition (player · camera projection ·
/// player behaviour state — the second item the entry guard reads) are packed
/// into one parameter:
/// `apply_input`'s bare parameters have reached the limit of the `SystemParam`
/// tuple (16), and listing them out would silently make the whole system
/// lose `IntoSystem` (the error only surfaces at the schedule's `.chain()`).
/// Only the enter/exit branches consume it; the regular drag/zoom path does not.
/// The photo presenter owns gestures while capturing or selecting a frame.
#[derive(Resource, Default)]
pub struct PhotoGestureHeld(pub bool);

#[derive(SystemParam)]
pub struct FpsTransitionCtx<'w, 's> {
    dialogs: Res<'w, crate::ui_inputs::ShellDialogState>,
    library: Res<'w, crate::field_modes::LibraryMode>,
    layers: Res<'w, crate::ui_layers::UiLayerStack>,
    avatar: Res<'w, crate::player_state::PlayerAvatarStates>,
    talk_camera: Option<Res<'w, crate::talk_camera::TalkCamera>>,
    photo_input: Option<Res<'w, PhotoGestureHeld>>,
    bulk_input: Option<Res<'w, crate::field_modes::BulkHarvestActive>>,
    players: Query<'w, 's, (&'static GlobalTransform, &'static mut Visibility), With<AvatarRoot>>,
    cameras: Query<'w, 's, &'static mut Projection, With<Camera3d>>,
}

pub fn apply_input(
    mut commands: Commands,
    mut gestures: MessageReader<GestureEvent>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut models: Option<ResMut<FieldCameraModel>>,
    edits: Res<crate::field_modes::EditSessionActive>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    tween: Option<Res<CameraTween>>,
    state: Res<FieldCameraState>,
    mut fps_view: ResMut<FpsViewMemory>,
    memory: Option<Res<NormalCameraMemory>>,
    mut transition: FpsTransitionCtx,
    mut smoke_entered: Local<bool>,
    mut smoke_exited: Local<bool>,
    mut smoke_pinched: Local<bool>,
) {
    // While the placement editor owns input (in edit mode the gesture layer belongs
    // to the editor), camera drag/zoom yields entirely — input routing is settled
    // inside edit mode.
    if transition.bulk_input.is_some() || transition.photo_input.as_ref().is_some_and(|held| held.0) || edits.is_active() || transition.dialogs.blocks_field_input() || (!transition.layers.on_field()
        && !(state.0 == CameraStateType::PhotoShot && transition.layers.is_active(crate::ui_layers::MenuScreenType::MysekaiPhotoShot)))
    {
        gestures.clear();
        return;
    }
    // The camera consumes only the per-frame Moved of DRAG (the camera domain
    // dispatches gestures to OnDrag only for the drag family's UPDATE state; the
    // tap/drag distinction threshold lives in the gesture layer). The increment is
    // the per-frame delta carried by the event itself. Events are drained even when
    // no model exists yet, so stale deltas do not accumulate.
    let mut drag = Vec2::ZERO;
    let mut ui_drag = Vec2::ZERO;
    for event in gestures.read() {
        if event.kind == GestureKind::Drag && event.state == GestureState::Moved
        {
            if event.ui_owned { ui_drag += event.delta } else { drag += event.delta }
        }
    }
    // A conversation's window captures the pointer at press time, so a drag
    // started anywhere over it is marked UI-owned and the lens would never
    // turn while a conversation plays. In the talk state the same drag turns
    // the camera too: the window consumes taps, not the viewer's hand.
    if state.0 == CameraStateType::Talk { drag += ui_drag; }
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
    if tween.as_deref().is_some_and(CameraTween::is_normal_reset) {
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
            CameraStateType::PhotoShot => photo_drag(model,source_drag),
            CameraStateType::Fps => fps_drag(model, &mut fps_view, source_drag.x, source_drag.y),
            // Talk deliberately has no Normal pitch/distance coupling. The
            // lens keeps turning while a conversation plays: only the shot's
            // own framing is fixed, not the viewer's hand.
            CameraStateType::Talk => update_angle(model, source_drag.x, source_drag.y),
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
            CameraStateType::PhotoShot => {
                model.fov=(model.fov-ratio*pinch*5.0).clamp(PHOTO_MIN_FOV,PHOTO_MAX_FOV);
                for mut projection in &mut transition.cameras {
                    if let Projection::Perspective(p)=&mut *projection { p.fov=model.fov.to_radians(); }
                }
            }
            CameraStateType::Normal if playback_inspection => {
                // Playback inspection must not replace the authored shot with
                // an FPS transition or Normal's pitch/distance coupling.
                playback_zoom(model, v13);
            }
            CameraStateType::Normal => {
                if can_switch_to_fps(model, player.is_some(), v13, &transition.avatar) {
                    // 进入支：当帧不再缩放（源 ChangeState 后直接 return）。
                    // ChangeState(FPS): Normal's OnExit, then FPS's OnEnter
                    // ([`change_state`]), applied before this frame's follow.
                    commands.queue(|world: &mut World| {
                        change_state(world, CameraStateType::Fps, "camera-fps");
                    });
                    return;
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
                    // 退出支（捏合张开方向）。FPS OnPinch 仅此一支:
                    // ChangeState(Normal), FPS's OnExit then Normal's OnEnter
                    // (TransferCameraSettings case 16 or the inherit branch).
                    commands.queue(|world: &mut World| {
                        change_state(world, CameraStateType::Normal, "camera-fps");
                    });
                    return;
                } else {
                    info!(
                        "[camera-fps] FPS 态捏合 Δ{:.1}px：无缩放支（源 OnPinch 仅反向退出支），距离保持 {:.2}",
                        pinch, model.distance
                    );
                }
            }
            CameraStateType::Talk
                if transition
                    .talk_camera
                    .as_deref()
                    .is_some_and(|camera| camera.accepts_input())
                => {
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
pub fn wrap180(deg: f32) -> f32 {
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
pub fn to_rotation(from: f32, to: f32) -> f32 {
    from + wrap180(to - from)
}

pub use crate::ui_inputs::out_quad;

/// 读相机本体当前的垂直视场（角度制）。源 `DoTweenCameraSetting` 的
/// prevFov 从 `Camera.fieldOfView` 取——**相机本体，不是模型字段**；本仓
/// 等价位是投影件。非透视投影响亮拒绝（场地相机按构造恒透视）。
pub fn perspective_fov_deg(projection: &Projection) -> f32 {
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

/// `NormalCameraState._model`: Normal's private copy of the field camera's
/// model, taken when the camera's state machine is built (the setting's
/// values, LookAt zero). Its distance is the shared model's
/// [`FieldCameraModel::gestured_distance`] (the pinch writes it there and the
/// drag coupling reads it); the other fields the state writes back and reads
/// are here. Its distance and pitch bounds and its offset are never written
/// after the copy, so Normal's OnEnter restores them from the setting.
#[derive(Resource, Debug, Clone, Copy)]
pub struct NormalCameraPrivate {
    pub look_at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
}

impl NormalCameraPrivate {
    fn from_setting(setting: &CameraSetting) -> Self {
        Self {
            look_at: Vec3::ZERO,
            yaw: setting.init_yaw,
            pitch: setting.init_pitch,
            fov: setting.fov,
        }
    }
}

/// `NormalCameraState._isCompleteCameraTween`: set by Normal's OnEnter,
/// cleared when `TransferCameraSettings` starts its tween and set again by
/// that tween's callback. A newer camera tween kills the transfer tween
/// without its callback, so the flag then stays clear until the next OnEnter.
/// Normal's OnExit keeps the private model while it is clear, and Normal's
/// follow runs at the fixed per-frame factor while it is clear.
#[derive(Resource, Debug, Clone, Copy)]
pub struct NormalCameraTweenComplete(pub bool);

/// `NormalCameraState._cameraTransferData`: Normal's OnExit stores the live
/// pitch, yaw, FOV and distance under the current site; the inherit branch
/// of its OnEnter reads the current site's entry. Keyed by site type (one
/// site per type here; the source keys by site id).
#[derive(Resource, Default)]
pub struct NormalTransferData(pub std::collections::HashMap<String, NormalCameraMemory>);

fn camera_fov(world: &mut World) -> Option<f32> {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    cameras.single(world).ok().map(perspective_fov_deg)
}

fn player_view_position(world: &mut World) -> Option<Vec3> {
    let mut players = world.query_filtered::<&GlobalTransform, With<AvatarRoot>>();
    players.single(world).ok().map(GlobalTransform::translation)
}

/// `NormalCameraState.IsInheritCameraSetting` with the previous camera
/// state: a harvest or delivery site inherits unless the camera comes back
/// from LearnSiteEnvironment (17); a housing site inherits after one of the
/// five outdoor harvest site types ([`is_inherit_camera_setting`]).
pub fn is_inherit(category: &str, prev_site_type: &str, prev: CameraStateType) -> bool {
    if prev == CameraStateType::BulkHarvest { return false; }
    if matches!(category, "harvest" | "delivery") {
        prev != CameraStateType::LearnSiteEnvironment
    } else {
        is_inherit_camera_setting(category, prev_site_type)
    }
}

/// `NormalCameraState.IsInheritCameraSetting` for a caller whose previous
/// camera state is never LearnSiteEnvironment (17): a harvest or delivery
/// site inherits; a housing site inherits after one of the five outdoor
/// harvest site types (the source compares 4 <= PrevSiteType <= 8). The
/// category and site type strings are the source enum names. [`is_inherit`]
/// takes the previous state as well.
pub fn is_inherit_camera_setting(category: &str, prev_site_type: &str) -> bool {
    if matches!(category, "harvest" | "delivery") {
        true
    } else {
        matches!(
            prev_site_type,
            "grassland" | "shore" | "flower_garden" | "memorial_place" | "festival_garden"
        )
    }
}

/// `NormalCameraState.OnExit` at `site`: while `_isCompleteCameraTween` is
/// set, the private model takes the live LookAt, distance, yaw and pitch;
/// the transfer entry of the site takes the live pitch, yaw, FOV and
/// distance (always). The reset flag it clears is the reset tween's kind
/// here ([`CameraTween::is_normal_reset`]); the player's dither value has no
/// counterpart. False without a camera model.
pub fn normal_on_exit(world: &mut World, site: &str) -> bool {
    let complete = world
        .get_resource::<NormalCameraTweenComplete>()
        .is_none_or(|flag| flag.0);
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        return false;
    };
    if complete {
        model.gestured_distance = model.distance;
    }
    let model = model.clone();
    if complete {
        if let Some(mut private) = world.get_resource_mut::<NormalCameraPrivate>() {
            private.look_at = model.look_at;
            private.yaw = model.yaw;
            private.pitch = model.pitch;
        }
    }
    let snapshot = NormalCameraMemory {
        site: site.to_owned(),
        look_at: model.look_at,
        distance: model.distance,
        yaw: model.yaw,
        pitch: model.pitch,
        fov: model.fov,
    };
    info!(
        "[camera] Normal.OnExit at {site}: private model {} (tween complete {complete}); transfer entry pitch {:.2} yaw {:.2} FOV {:.2} distance {:.3}",
        if complete { "takes the live LookAt, distance, yaw and pitch" } else { "kept" },
        model.pitch, model.yaw, model.fov, model.distance
    );
    world.insert_resource(snapshot.clone());
    world
        .get_resource_or_insert_with(NormalTransferData::default)
        .0
        .insert(site.to_owned(), snapshot);
    true
}

/// `FieldCamera.ChangeState(Normal)` after `prev`'s OnExit:
/// `NormalCameraState.OnEnter`. The flag is set, the distance and pitch
/// bounds and the offset come back from the private copy (the setting's),
/// then either the inherit branch (0.5 s to the site's transfer entry, or to
/// the setting's initial pitch with the live yaw, FOV and distance; its
/// callback writes the entry's distance into the private model) or
/// [`transfer_camera_settings`]. The tutorial branch is not in the product
/// (tutorials are out of scope). `FadeInPlayerIfNeeded` (a 0.2 s dither
/// fade-in after the first-person state) has no dither to drive: FPS's exit
/// shows the player at once.
pub fn normal_on_enter(world: &mut World, prev: CameraStateType, tag: &str) {
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    world.insert_resource(NormalCameraTweenComplete(true));
    let Some(setting) = world.get_resource::<CameraSetting>().copied() else {
        warn!("[{tag}] Normal.OnEnter from {prev:?} without the camera setting: no tween");
        return;
    };
    let site = world.get_resource::<SiteActive>().cloned();
    let prev_site = world.resource::<PrevSiteType>().0.clone();
    let fov_now = camera_fov(world);
    let player = player_view_position(world);
    let mut private = world
        .get_resource::<NormalCameraPrivate>()
        .copied()
        .unwrap_or_else(|| NormalCameraPrivate::from_setting(&setting));
    let entry = site.as_ref().and_then(|site| {
        world
            .get_resource::<NormalTransferData>()
            .and_then(|table| table.0.get(&site.site_type).cloned())
    });
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!("[{tag}] Normal.OnEnter from {prev:?} without a camera model: no tween");
        return;
    };
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    let fov_now = fov_now.unwrap_or(model.fov);
    let site_name = site.as_ref().map_or("?", |site| site.site_type.as_str());
    let inherit = site
        .as_ref()
        .is_some_and(|site| is_inherit(&site.category, &prev_site, prev));
    let (tween, transfer) = if inherit {
        let (pitch, yaw, fov, distance, hit) = match &entry {
            Some(m) => (m.pitch, m.yaw, m.fov, m.distance, true),
            None => (setting.init_pitch, model.yaw, model.fov, model.distance, false),
        };
        let prev_pitch = wrap180(model.pitch);
        let prev_yaw = wrap180(model.yaw);
        let tween = CameraTween {
            look_at: (model.look_at, model.look_at),
            fov: (fov_now, fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, pitch)),
            yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
            distance: (model.distance, distance),
            duration: INHERIT_TWEEN_SECS,
            elapsed: 0.0,
            on_complete: TweenCompletion::NormalInherit { distance },
        };
        info!(
            "[{tag}] ChangeState({prev:?} -> Normal) at {site_name} after {prev_site}: inherit branch ({}), {INHERIT_TWEEN_SECS}s: distance {:.3} -> {:.3}, pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, FOV {:.2} -> {:.2}",
            if hit { "site transfer entry" } else { "fallback: initPitch + live yaw, FOV, distance" },
            tween.distance.0, tween.distance.1, tween.pitch.0, tween.pitch.1,
            tween.yaw.0, tween.yaw.1, tween.fov.0, tween.fov.1
        );
        (Some(tween), false)
    } else {
        let (tween, case) = transfer_camera_settings(&mut model, &mut private, prev, player, fov_now);
        match &tween {
            Some(tween) => info!(
                "[{tag}] ChangeState({prev:?} -> Normal) at {site_name} after {prev_site}: TransferCameraSettings {case}, {}s to Normal's private model: LookAt {:.3} -> {:.3}, distance {:.3} -> {:.3}, pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, FOV {:.2} -> {:.2}",
                tween.duration, tween.look_at.0, tween.look_at.1, tween.distance.0, tween.distance.1,
                tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1, tween.fov.0, tween.fov.1
            ),
            None => info!(
                "[{tag}] ChangeState({prev:?} -> Normal) at {site_name}: TransferCameraSettings {case}: the live model takes Normal's private model at once (LookAt {:.3}, distance {:.3}, pitch {:.2}, yaw {:.2}), no tween",
                model.look_at, model.distance, model.pitch, model.yaw
            ),
        }
        let transfer = tween.is_some();
        (tween, transfer)
    };
    drop(model);
    world.insert_resource(private);
    if transfer {
        world.insert_resource(NormalCameraTweenComplete(false));
    }
    if let Some(tween) = tween {
        world.insert_resource(tween);
    }
}

/// `NormalCameraState.OnEnter`'s inherit branch passes this duration.
pub const INHERIT_TWEEN_SECS: f32 = 0.5;

/// `NormalCameraState.TransferCameraSettings`: `_doTweenCameraTime` = 1.0,
/// then the previous-state switch:
/// - FloorEdit, WallEdit: the private yaw takes the live yaw;
/// - LookAtPlayer: the live distance, yaw and pitch take the private ones,
///   and it returns without a tween;
/// - SiteMoveAction: 0.6 s, the private LookAt takes the live LookAt;
/// - HouseEntry: the private yaw and LookAt take the live ones;
/// - SomeCharacterTalk, ScreenshotCapture, ScreenshotCaptureFPS: the live
///   LookAt, distance, yaw and pitch take the private ones, no tween;
/// - FPS: 0.1 s, the private LookAt is the player's view position and the
///   private yaw the live yaw;
/// - LearnSiteEnvironment: 0.3 s, the private LookAt takes the live one and
///   the private distance is 8.0;
/// - every other state: nothing.
///
/// Then the flag clears and the camera tweens to the private model (LookAt,
/// pitch and yaw, FOV, distance) over `_doTweenCameraTime`; the callback
/// sets the flag again. Returns the tween (None for the no-tween cases) and
/// the case taken, for the log.
fn transfer_camera_settings(
    model: &mut FieldCameraModel,
    private: &mut NormalCameraPrivate,
    prev: CameraStateType,
    player: Option<Vec3>,
    fov_now: f32,
) -> (Option<CameraTween>, &'static str) {
    use CameraStateType as S;
    let mut seconds = TRANSFER_TWEEN_SECS;
    let case = match prev {
        S::FloorEdit | S::WallEdit => {
            private.yaw = model.yaw;
            "case 2/3 (private yaw = live yaw)"
        }
        S::BulkHarvest => {
            model.gestured_distance = model.distance;
            private.yaw = model.yaw;
            private.pitch = model.pitch;
            return (None, "case 21 (private distance and rotation = live)");
        }
        S::LookAtPlayer => {
            model.distance = model.gestured_distance;
            model.yaw = private.yaw;
            model.pitch = private.pitch;
            return (None, "case 5");
        }
        S::SiteMoveAction => {
            seconds = 0.6;
            private.look_at = model.look_at;
            "case 6 (private LookAt = live LookAt)"
        }
        S::HouseEntry => {
            private.yaw = model.yaw;
            private.look_at = model.look_at;
            "case 8 (private yaw and LookAt = live)"
        }
        S::SomeCharacterTalk | S::ScreenshotCapture | S::ScreenshotCaptureFps => {
            model.look_at = private.look_at;
            model.distance = model.gestured_distance;
            model.yaw = private.yaw;
            model.pitch = private.pitch;
            return (None, "case 11/18/19");
        }
        S::Fps => {
            match player {
                Some(player) => private.look_at = player,
                None => warn!("[camera] TransferCameraSettings case 16 without a player: private LookAt kept"),
            }
            private.yaw = model.yaw;
            seconds = FPS_EXIT_TWEEN_SECS_CASE16;
            "case 16 (private LookAt = player, private yaw = live yaw)"
        }
        S::LearnSiteEnvironment => {
            private.look_at = model.look_at;
            seconds = 0.3;
            model.gestured_distance = 8.0;
            "case 17 (private LookAt = live LookAt, private distance 8)"
        }
        _ => "default path",
    };
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let tween = CameraTween {
        look_at: (model.look_at, private.look_at),
        fov: (fov_now, private.fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, private.pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, private.yaw)),
        distance: (model.distance, model.gestured_distance),
        duration: seconds,
        elapsed: 0.0,
        on_complete: TweenCompletion::NormalTransfer,
    };
    (Some(tween), case)
}

/// `NormalCameraState.TransferCameraSettings`' `_doTweenCameraTime` before
/// the previous-state switch.
const TRANSFER_TWEEN_SECS: f32 = 1.0;

/// `FPSCameraState.OnEnter` after `prev`'s OnExit: the camera unlocks, the
/// player is hidden (the product has visibility where the source has
/// `SetVisible`), the private yaw takes the camera's euler yaw (the camera
/// looks along the model's yaw, folded into [0, 360)), the shared LookAt is
/// the player's position plus `CAMERA_HEIGHT_OFFSET`, the private minimum
/// pitch is set by the site (indoor floors -17, elsewhere -8), and the
/// shared distance bounds (both 0.15) and pitch bounds take the private
/// ones. Then 0.2 s to (shared LookAt, private pitch and yaw, shared FOV,
/// 0.15). The private pitch is not reset on entry: it is what the last
/// first-person drag left. `SetActiveUI(false)` (the screenshot mode),
/// `SiteObjectManager.ShowAll` and `ResetHouseDither` have no counterpart.
pub fn fps_on_enter(world: &mut World, prev: CameraStateType) {
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Fps;
    let is_room = world
        .get_resource::<SiteSelection>()
        .is_some_and(|site| site.is_room());
    let fov_now = camera_fov(world);
    let player = {
        let mut players =
            world.query_filtered::<(&GlobalTransform, &mut Visibility), With<AvatarRoot>>();
        match players.single_mut(world) {
            Ok((global, mut visibility)) => {
                visibility.set_if_neq(Visibility::Hidden);
                Some(global.translation())
            }
            Err(_) => None,
        }
    };
    let Some(player) = player else {
        warn!("[camera-fps] FPS.OnEnter from {prev:?} without a player: no tween");
        return;
    };
    let mut fps = world
        .get_resource::<FpsViewMemory>()
        .copied()
        .unwrap_or_default();
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!("[camera-fps] FPS.OnEnter from {prev:?} without a camera model: no tween");
        return;
    };
    let before = (
        model.min_distance,
        model.max_distance,
        model.min_pitch,
        model.max_pitch,
    );
    fps.yaw = normalize360(model.yaw);
    model.look_at = player + FPS_CAMERA_HEIGHT_OFFSET;
    model.min_distance = FPS_DISTANCE;
    model.max_distance = FPS_DISTANCE;
    model.min_pitch = if is_room {
        FPS_INDOOR_MIN_PITCH
    } else {
        FPS_MIN_PITCH
    };
    model.max_pitch = FPS_MAX_PITCH;
    let camera_fov_deg = fov_now.unwrap_or(model.fov);
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let tween = CameraTween {
        look_at: (model.look_at, model.look_at),
        fov: (camera_fov_deg, model.fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, fps.pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, fps.yaw)),
        distance: (model.distance, FPS_DISTANCE),
        duration: FPS_ENTER_TWEEN_SECS,
        elapsed: 0.0,
        on_complete: TweenCompletion::None,
    };
    info!(
        "[camera-fps] ChangeState({prev:?} -> FPS): distance {:.2}->{:.2} (bounds [{:.2},{:.2}]->[{:.2},{:.2}], pitch bounds [{:.1},{:.1}]->[{:.1},{:.1}]), LookAt {}, yaw {:.1}->{:.1} (pitch target {:.1} left by the last drag), FOV {:.1}->{:.1}, player hidden",
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
    drop(model);
    world.insert_resource(fps);
    world.insert_resource(tween);
}

/// `FPSCameraState.OnExit`: the player is shown again; `SetActiveUI(true)`
/// (leaving the screenshot mode) has no counterpart.
pub fn fps_on_exit(world: &mut World) {
    let mut players = world.query_filtered::<&mut Visibility, With<AvatarRoot>>();
    if let Ok(mut visibility) = players.single_mut(world) {
        visibility.set_if_neq(Visibility::Visible);
    }
    info!("[camera-fps] FPS.OnExit: player shown");
}

/// The OnExit of `state` as `FieldCamera.ChangeState` runs it before the
/// next state's OnEnter. Normal and FPS exits are here. ZoomPlayer's and
/// PreSiteMoveAction's are empty, FloorEdit's only removes its event
/// registrations, HarvestTone's (showing its head-up display) runs in the
/// tone module when it sees the state change. Any other state's exit belongs
/// to its own module; leaving it from here is logged and its exit is not
/// run.
pub fn exit_state(world: &mut World, state: CameraStateType, tag: &str) {
    use CameraStateType as S;
    match state {
        S::Normal => {
            let site = world
                .get_resource::<SiteActive>()
                .map(|active| active.site_type.clone());
            match site {
                Some(site) => {
                    normal_on_exit(world, &site);
                }
                None => warn!("[{tag}] Normal.OnExit without an active site: no transfer entry"),
            }
        }
        S::Fps => fps_on_exit(world),
        S::PhotoShot => photo_exit(world),
        S::ZoomPlayer | S::PreSiteMoveAction | S::FloorEdit | S::SiteEnvironmentEdit | S::HarvestTone | S::None => {}
        other => warn!("[{tag}] {other:?}.OnExit is not run from here (its own module runs it)"),
    }
}

/// The OnEnter of `next` after `prev`'s OnExit, for the states whose entry
/// is here (Normal and FPS). Any other state is set without its OnEnter,
/// which belongs to its own module, and that is logged.
pub fn enter_state(
    world: &mut World,
    next: CameraStateType,
    prev: CameraStateType,
    tag: &str,
) {
    match next {
        CameraStateType::Normal => normal_on_enter(world, prev, tag),
        CameraStateType::Fps => fps_on_enter(world, prev),
        CameraStateType::PhotoShot => photo_enter(world),
        other => {
            warn!("[{tag}] {other:?}.OnEnter is not run from here (its own module runs it); the state is set");
            world.resource_mut::<FieldCameraState>().0 = other;
        }
    }
}

/// `FieldCamera.ChangeState(next)`: nothing when `next` is the current
/// state; otherwise the current state's OnExit, then `next`'s OnEnter.
pub fn change_state(world: &mut World, next: CameraStateType, tag: &str) {
    let prev = world.resource::<FieldCameraState>().0;
    if prev == next {
        return;
    }
    exit_state(world, prev, tag);
    enter_state(world, next, prev, tag);
}

/// The field camera's reset inputs, bundled so a UI system stays within the
/// system-parameter limit.
#[derive(SystemParam)]
pub struct CameraReset<'w, 's> {
    state: Res<'w, FieldCameraState>,
    model: Option<ResMut<'w, FieldCameraModel>>,
    setting: Option<Res<'w, CameraSetting>>,
    tone_pre: Option<Res<'w, HarvestTonePreModel>>,
    tone_model: Option<ResMut<'w, HarvestToneModel>>,
    cameras: Query<'w, 's, &'static Projection, With<Camera3d>>,
}

/// `NormalCameraState.ResetCameraSetting` passes this duration.
const NORMAL_RESET_TWEEN_SECS: f32 = 0.25;
/// `HarvestToneCameraState.ResetCameraSetting` passes this duration.
const HARVEST_TONE_RESET_TWEEN_SECS: f32 = 0.5;

impl CameraReset<'_, '_> {
    /// `FieldCamera.ResetCameraSetting`: the current state's
    /// `ResetCameraSetting` (no lock check on the way). Normal and
    /// HarvestTone have bodies; every other state's is empty.
    pub fn reset_camera_setting(&mut self, commands: &mut Commands) {
        match self.state.0 {
            CameraStateType::Normal => self.normal_reset(commands),
            CameraStateType::HarvestTone => self.harvest_tone_reset(commands),
            _ => {}
        }
    }

    /// `HarvestToneCameraState.ResetCameraSetting`: the tone state's private
    /// model takes the setting's pitch bounds at once (one 8-byte copy of
    /// minPitch and maxPitch), then `DoTweenCameraSetting` runs for 0.5 s
    /// towards the model its OnEnter copied: LookAt, rotation (pitch, yaw),
    /// FOV and distance all from that copy. The callback writes the setting
    /// distance into the private model. The shared model's pitch bounds are
    /// not touched. While the tone animation runs, its OnUpdate still writes
    /// distance and pitch after the tween each frame; the tween's yaw and
    /// FOV stand, and its LookAt is where the tone's follow step starts.
    fn harvest_tone_reset(&mut self, commands: &mut Commands) {
        let (Some(model), Some(setting)) = (self.model.as_deref(), self.setting.as_deref()) else {
            warn!("[camera] harvest-tone reset pressed before the field camera model exists");
            return;
        };
        let Ok(projection) = self.cameras.single() else {
            warn!("[camera] harvest-tone reset pressed without a single field camera");
            return;
        };
        // The copy is taken on the first frame the state reads HarvestTone.
        // A press on the very frame the state changed comes before that
        // frame's copy; nothing has moved the model since the change, so the
        // live model is what OnEnter copied.
        let (pre, taken) = match self.tone_pre.as_deref() {
            Some(pre) => (pre.0.clone(), "OnEnter copy"),
            None => (model.clone(), "live model, entered this frame"),
        };
        match self.tone_model.as_deref_mut() {
            Some(private) => {
                private.0.min_pitch = setting.min_pitch;
                private.0.max_pitch = setting.max_pitch;
            }
            None => warn!("[camera] harvest-tone private model not built (no camera setting parse)"),
        }
        let prev_pitch = wrap180(model.pitch);
        let prev_yaw = wrap180(model.yaw);
        let tween = CameraTween {
            look_at: (model.look_at, pre.look_at),
            fov: (perspective_fov_deg(projection), pre.fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, pre.pitch)),
            yaw: (prev_yaw, to_rotation(prev_yaw, pre.yaw)),
            distance: (model.distance, pre.distance),
            duration: HARVEST_TONE_RESET_TWEEN_SECS,
            elapsed: 0.0,
            on_complete: TweenCompletion::HarvestToneReset { distance: setting.distance },
        };
        info!(
            "[camera] harvest-tone reset: private pitch bounds [{:.2}, {:.2}]; tween {}s to the pre-camera model ({taken}): LookAt {:.3} -> {:.3}, pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, distance {:.3} -> {:.3}, FOV {:.2} -> {:.2}; callback: private distance = {:.3}",
            setting.min_pitch, setting.max_pitch, tween.duration,
            tween.look_at.0, tween.look_at.1, tween.pitch.0, tween.pitch.1,
            tween.yaw.0, tween.yaw.1, tween.distance.0, tween.distance.1,
            tween.fov.0, tween.fov.1, setting.distance,
        );
        commands.insert_resource(tween);
    }

    /// `NormalCameraState.ResetCameraSetting`: the state's model mirror takes
    /// the setting's pitch bounds at once, the reset flag is set, and
    /// `DoTweenCameraSetting` runs for 0.25 s towards (LookAt as it is, pitch
    /// = setting initPitch, yaw as it is, FOV = model FOV, distance = setting
    /// distance). Its callback writes the setting distance into the mirror
    /// and clears the flag.
    fn normal_reset(&mut self, commands: &mut Commands) {
        let (Some(model), Some(setting)) = (self.model.as_deref_mut(), self.setting.as_deref()) else {
            warn!("[camera] reset pressed before the field camera model exists");
            return;
        };
        let Ok(projection) = self.cameras.single() else {
            warn!("[camera] reset pressed without a single field camera");
            return;
        };
        model.min_pitch = setting.min_pitch;
        model.max_pitch = setting.max_pitch;
        // DoTweenCameraSetting: start values captured now (LookAt, the camera
        // body's FOV, distance, and pitch/yaw folded into [-180, 180]); the
        // rotation end goes the shortest signed way from the folded start.
        let prev_pitch = wrap180(model.pitch);
        let prev_yaw = wrap180(model.yaw);
        let tween = CameraTween {
            look_at: (model.look_at, model.look_at),
            fov: (perspective_fov_deg(projection), model.fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, setting.init_pitch)),
            yaw: (prev_yaw, to_rotation(prev_yaw, model.yaw)),
            distance: (model.distance, setting.distance),
            duration: NORMAL_RESET_TWEEN_SECS,
            elapsed: 0.0,
            on_complete: TweenCompletion::NormalReset { distance: setting.distance },
        };
        info!(
            "[camera] reset: pitch {:.2}->{:.2}, yaw {:.2}->{:.2}, distance {:.2}->{:.2}, FOV {:.2}->{:.2} over {}s",
            tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1,
            tween.distance.0, tween.distance.1, tween.fov.0, tween.fov.1, tween.duration,
        );
        commands.insert_resource(tween);
    }
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

/// PostUpdate: the Normal-state follow rules (matching
/// `NormalCameraState.OnUpdate` line for line); advances [`FieldCameraModel`]
/// every frame and writes the camera transform.
///
/// The rule (after the IFix patch gate):
/// ```text
/// v13 = deltaTime / 0.016667                 // frame-duration normalisation (literal as is)
/// t   = clamp(v13 * 0.1, 0, 1)               // follow interpolation factor
/// LookAt += (playerPos - LookAt) * t          // all three axes interpolated (x/y/z)
/// t2  = MinDistance != MaxDistance
///       ? clamp((Distance-Min)/(Max-Min), 0, 1) : 0
/// xRange = InterpRange(near.min.x, far.min.x, near.max.x, far.max.x, t2)
///          // inner formula: x = (minAtMin - minAtMax)·t2 + minAtMax
///          //                y = (maxAtMin - maxAtMax)·t2 + maxAtMax
/// zRange = the same formula on the z components
/// LookAt.x = clamp(LookAt.x, xRange)          // clamp max first, then min; Y is not clamped
/// LookAt.z = clamp(LookAt.z, zRange)
/// eye = GetPosition(pitch, yaw, LookAt, Offset, Distance)   // see view_dir
/// camera forward = LookAt + Offset (Transform.LookAt)
/// ```
/// The follow target is the view-transform position of the player avatar
/// (`GetFollowPlayer` → `AvatarDataStore` slot → `GetViewTransform().position`;
/// the equivalent here is the world transform of the first member in the roster —
/// the same semantic slot).
///
/// Transition tweens (the four parallel tweens of `DoTweenCameraSetting`) advance
/// at the head of this function: all four components use OutQuad, LookAt/pitch/
/// yaw/distance are written to the model every frame, FOV is written to the
/// **camera projection** (the setter writes `Camera.fieldOfView` directly), and
/// the resource is removed at the end. While a tween is in flight the follow
/// coefficient is v13 = 1.0 (t is the constant 0.1/frame); the dt-normalisation
/// branch runs only after the tween has finished and the camera is idle (the
/// OnUpdate gate: `_isCompleteCameraTween && !CurrentAction.IsPlaying`).
/// The follow rules and the tween **stack within the same frame** rather than
/// yielding — the tween writes LookAt and OnUpdate still pulls it 10% toward the
/// player.
///
/// FPS-state rules (`FPSCameraState.OnUpdate`, line for line): the look-at point
/// is written directly as player position + height offset (no interpolation and
/// no clamping — following is the Normal state's job); the eye steps back along
/// the view direction by the FPS distance, and the camera looks at the look-at
/// point (UpdatePosition + UpdateRotation; `looking_at` here is the exact form,
/// and the 0.017453 literal truncation of FromEulerRad is not carried over);
/// player facing = the 3D LookRotation of the camera forward (it pitches with
/// looking down/up; the player is hidden in FPS and keeps its last orientation
/// after exit).
///
/// The FPS follow is not another chain but the `FieldCamera.UpdatePosition`
/// track **shared** by both states (the method body has no state branch: read
/// model Pitch/Yaw/LookAt/Offset/Distance → GetPosition → write the camera
/// position). Both states differ only **before/after** entering that track: how
/// LookAt is obtained (Normal: smoothly pulled toward the player with factor
/// `min(δt/0.016667·0.1, 1)` + interpolated bounds clamp on x/z within the
/// distance range; FPS: player position+0.1 written directly, no smoothing and
/// no clamping) and how the camera forward is written (Normal:
/// `Transform.LookAt(LookAt+Offset)`; FPS: pitch/yaw quaternion).
/// Normal state only: house dither and harvest-station camera collision; the
/// 16 state has neither.
///
/// There is no head-bone or player-facing anchor on the camera side: the
/// look-at point reads the player view **root** position
/// (`PlayerAvatarPresenter.get_Position` = the view transform position, not any
/// bone), and no camera-domain writer touches the head/spine fields anywhere in
/// the tree. The player→camera direction is not written either: the camera
/// forward never reads the player facing.
///
/// The player←camera write (the facing LookRotation above) is the one that wins
/// in the same-frame double write: one view transform is written by two places
/// every frame — the move state in the Update phase
/// (`AvatarDataStore.Update` → `PlayerAvatarPresenter.OnUpdate` →
/// `PlayerAvatarStateMachine.OnUpdate` → `PlayerAvatarMoveState.UpdateState`
/// writes `Euler(0, atan2(input.x, input.z), 0)` every frame), and the camera
/// state's tail write in the LateUpdate phase (`FieldCamera.LateUpdate` → state
/// machine → OnUpdate tail).
/// In Unity's frame order, Update runs before LateUpdate ⇒ the camera write lands
/// later: in the 16 state the torso facing always follows the camera and
/// overrides the input facing even while walking (this contradicts earlier notes
/// that guessed the player facing keeps its own direction while moving — that
/// write exists, but the phase order overrides it, so it never shows on screen).
/// This repository uses the same order: `player::advance` writes the move
/// facing in Update and this system writes the camera forward in PostUpdate, so
/// the same side wins.
pub fn follow_avatar(
    time: Res<Time>,
    mut commands: Commands,
    state: Res<FieldCameraState>,
    models: Option<ResMut<FieldCameraModel>>,
    mut tween: Option<ResMut<CameraTween>>,
    // Both queries hold &mut Transform: the mutually exclusive Without
    // declarations keep them disjoint (a camera is not an avatar and an avatar is
    // not a camera), otherwise Bevy's borrow check rejects them as "possibly the
    // same entity".
    mut avatars: Query<(&GlobalTransform, &mut Transform), (With<AvatarRoot>, Without<Camera3d>)>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<Camera3d>, Without<AvatarRoot>)>,
    mut announced: Local<bool>,
    mut fps_announced: Local<bool>,
    talk: Option<Res<crate::talk::ActiveTalk>>,
    player_talk: Option<Res<crate::player_talk::PlayerTalkSession>>,
    talk_camera: Option<Res<crate::talk_camera::TalkCamera>>,
    // 16 态面朝追迹的 0.5s 累计器（AUTOFPS 冒烟窗口内用）。
    mut face_trace: Local<f32>,
    // The state this system saw last frame (HarvestTone's entry copy, the
    // reset instrument).
    mut watch: Local<StateWatch>,
    // Normal's `_isCompleteCameraTween` (its OnUpdate's follow-rate gate).
    normal_complete: Option<Res<NormalCameraTweenComplete>>,
) {
    let Some(mut models) = models else {
        return; // 站点未取景（或 JSON 未装）：模型未立，等下一帧
    };
    // HarvestToneCameraState.OnEnter copies the field camera's model into
    // `_preCameraModel` (its InheritPreCameraModel then copies it back, which
    // changes nothing). The state is written in Update and this is the first
    // camera system after it, before any tween or follow write this frame
    // (the harvest camera shake before it moves only the offset, which the
    // tone reset does not read from the copy).
    if watch.seen != Some(state.0) {
        watch.entered_at = time.elapsed_secs();
        watch.reset_pressed = false;
        if state.0 == CameraStateType::HarvestTone {
            info!(
                "[camera] HarvestTone entered: pre-camera model copied (LookAt {:.3}, pitch {:.2}, yaw {:.2}, distance {:.3}, FOV {:.2})",
                models.look_at, models.pitch, models.yaw, models.distance, models.fov
            );
            commands.insert_resource(HarvestTonePreModel(models.clone()));
        } else if watch.seen == Some(CameraStateType::HarvestTone) {
            commands.remove_resource::<HarvestTonePreModel>();
        }
        if state.0 == CameraStateType::Normal {
            // Normal.OnEnter sets `_isCompleteCameraTween`; only its
            // TransferCameraSettings clears it, together with starting its
            // tween. Entries that other modules run without
            // [`normal_on_enter`] get the same flag from the tween in flight.
            let transfer = tween
                .as_deref()
                .is_some_and(|tween| tween.on_complete == TweenCompletion::NormalTransfer);
            commands.insert_resource(NormalCameraTweenComplete(!transfer));
        }
        watch.seen = Some(state.0);
    }
    if !watch.reset_pressed
        && autoreset_due(state.0, time.elapsed_secs() - watch.entered_at)
    {
        watch.reset_pressed = true;
        let current = state.0;
        commands.queue(move |world: &mut World| {
            let mut system_state: SystemState<(Commands, CameraReset)> = SystemState::new(world);
            let (mut commands, mut reset) = system_state.get_mut(world).expect("system params");
            info!("[camera] instrument MOLY_CAMERA_AUTORESET: the camera reset pressed in {current:?}");
            reset.reset_camera_setting(&mut commands);
            system_state.apply(world);
        });
    }
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
        // DOTween's Vector3 and float plugins write start + change * eased,
        // with change = end - start, per component.
        models.look_at = tw.look_at.0 + (tw.look_at.1 - tw.look_at.0) * k;
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
            match tw.on_complete {
                TweenCompletion::NormalReset { distance } => models.gestured_distance = distance,
                TweenCompletion::NormalInherit { distance } => models.gestured_distance = distance,
                TweenCompletion::NormalTransfer => {
                    commands.insert_resource(NormalCameraTweenComplete(true));
                }
                TweenCompletion::HarvestToneReset { distance } => {
                    commands.queue(move |world: &mut World| {
                        if let Some(mut private) = world.get_resource_mut::<HarvestToneModel>() {
                            private.0.distance = distance;
                        }
                    });
                    info!("[camera] harvest-tone reset complete: the tone state's private distance = {distance:.3}");
                }
                TweenCompletion::None => {}
            }
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
        CameraStateType::PhotoShot => {
            let mut eye=models.look_at+models.offset+view_dir(models.pitch,models.yaw)*models.distance;
            eye.y=player.y+0.5;
            if let Ok((mut camera,mut projection))=cameras.single_mut() {
                *camera=Transform::from_translation(eye).looking_at(models.look_at,Vec3::Y);
                if let Projection::Perspective(p)=&mut *projection {p.fov=models.fov.to_radians();}
            }
        }
        CameraStateType::HouseEntry | CameraStateType::ZoomPlayer => {
            // HouseEntryCameraState.OnUpdate: LookAt = the player's view
            // position, UpdatePosition, view.LookAt(LookAt + Offset). The
            // entry owns entering and leaving this state. ZoomPlayer's
            // OnUpdate is the same body (`zoom_player_camera`).
            models.look_at = player;
            let pivot = models.look_at + models.offset;
            let eye = pivot + view_dir(models.pitch, models.yaw) * models.distance;
            if let Ok((mut camera, _)) = cameras.single_mut() {
                *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
            }
        }
        // The cannon move's camera states: the tween above is their
        // DoTweenCameraSetting; the per-frame write is
        // `site_move::camera::update`, which runs right after this system.
        CameraStateType::SiteMoveAction | CameraStateType::PreSiteMoveAction => {}
        // The door moves switch to None while the player walks out of a
        // room: no state updates the camera, which stays where it is.
        CameraStateType::None => {}
        // HarvestToneCameraState: its OnUpdate is `harvest::tone`, which
        // runs right after this system.
        CameraStateType::HarvestTone => {}
        // LearnSiteEnvironmentCameraState: its OnUpdate is `harvest::learn`,
        // which runs right after this system.
        CameraStateType::LearnSiteEnvironment => {}
        // DeliveryHonorRewardCameraState: its OnUpdate is `delivery_camera`,
        // which runs right after this system.
        CameraStateType::DeliveryHonorReward | CameraStateType::BulkHarvest => {}
        // FloorEditCameraState.OnUpdate: FieldCamera.UpdatePosition places
        // the eye from the model, then the camera looks at LookAt without
        // the offset. Its drag and pinch are `floor_edit_camera::input`.
        CameraStateType::FloorEdit | CameraStateType::SiteEnvironmentEdit => {
            let pivot = models.look_at + models.offset;
            let eye = pivot + view_dir(models.pitch, models.yaw) * models.distance;
            if let Ok((mut camera, _)) = cameras.single_mut() {
                *camera = Transform::from_translation(eye).looking_at(models.look_at, Vec3::Y);
            }
        }
        // CutSceneCameraState: the brain drives the camera
        // (`cutscene_camera`, right after this system).
        CameraStateType::CutScene => {}
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
            // 走完且空闲才 dt / 0.016667，再乘 0.1 钳 [0,1]。The source gate
            // is `_isCompleteCameraTween && !CurrentAction.IsPlaying`: a
            // transfer tween killed before its callback keeps the fixed
            // factor until Normal's next OnEnter.
            let complete = normal_complete.as_deref().is_none_or(|flag| flag.0);
            let raw = if tween_active || !complete {
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
                // One-time status line: the follow rules started here, with the live
                // values of the first step (advances every frame afterwards, no more
                // logging).
                if !*announced {
                    *announced = true;
                    info!(
                        "[camera] follow camera active: pivot {pivot:.2} eye {eye:.2} (target = avatar view transform, t factor 0.1 per frame base {FRAME_BASE})"
                    );
                }
            }
        }
    }
}

/// What [`follow_avatar`] remembers about the camera state between frames.
#[derive(Default)]
pub struct StateWatch {
    seen: Option<CameraStateType>,
    entered_at: f32,
    reset_pressed: bool,
}

/// Instrument (`MOLY_CAMERA_AUTORESET=<state>:<seconds>`, off by default,
/// e.g. `HarvestTone:1`): once per entry into that camera state, `seconds`
/// after it, the camera reset runs as the reset button's click runs it.
fn autoreset_due(state: CameraStateType, since_entry: f32) -> bool {
    let Ok(spec) = std::env::var("MOLY_CAMERA_AUTORESET") else {
        return false;
    };
    let Some((name, secs)) = spec.split_once(':') else {
        return false;
    };
    let Ok(secs) = secs.trim().parse::<f32>() else {
        return false;
    };
    format!("{state:?}") == name.trim() && since_entry >= secs
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
