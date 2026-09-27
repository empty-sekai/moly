//! SD角色上屏：名册成员的角色包/骨架装配，共享动作库。玩家的身体不走
//! 这里（见 [`crate::player_avatar::body`]）。
//!
//! 装载门形状与站点一致：请求 → 依赖到齐 → 展开 → 装配。名册成员
//! （npc 域的实体）各自挂自己的角色包场景为子实体——成员实体持位置与
//! 朝向，模型场景是它的孩子，位置的唯一写点因此不破：推进律仍是 npc
//! 实体 Transform 的唯一写者，这里不碰成员的变换。
//!
//! 动画的绑定基础：剪辑按「骨名链」哈希（[`AnimationTargetId`]）寻的。
//! 共享动作库的参考骨架与各角色包骨架在**参考根以下**的名链逐一相等
//! （被库驱动的每一个骨都如此），但参考根自己的名字是建库那份基准
//! 模型的，别的包各有各的根名。装配时把链首替换成库的参考根名，剪辑
//! 即直接驱动各包，无需重定向——根名在运行时从库的 [`GltfNode`]
//! （`is_animation_root`，恰一个）读出，不硬编码。glTF 装载器只给
//! 「文件内含动画」的场景装目标组件，角色包不含动画，这里按装载器的
//! 装配约定补齐：播放器落在动画根节点上，每个骨的目标指回播放器。
//!
//! 段的选择：移动相位（npc 域推进律的输出）在走播走姿族、驻留播待机
//! 族、转体播转身族。族名：位移族取名册 locomotion 的两列；转身族取
//! 转身律选出的基名（恒 `mov_cw_normal` 族——选段常量如此，与角色自身
//! 的位移族无关）。
//!
//! 换段照源的换段调用（`ChangeAnimationAsync(族名, hasExitTime, …,
//! playEndMotion, 0.5)`）的段链：
//! - 同族（段名去掉 `_S`/`_L`/`_E`/`_O` 后缀后相同）直接返回——在播的段
//!   不重起；
//! - 否则先播起始段 `族_S`，等 `UniTask.Delay(段长 / 速度)`（毫秒取整、
//!   DeltaTime、Update 时点：创建帧不计，此后逐帧累加帧间隔，累计达到段
//!   长的那一帧接段），再播循环段 `族_L`；起始段不在控制器里时两步都不
//!   做（换段不发、等待立即返回），直接到 `_L`；
//! - 每一步都是 `Animator.CrossFadeInFixedTime(段名, 0.5)`：固定秒数的
//!   过渡、目标段从 0 起播；与上一次播的段同名即不发。
//! - 新的换段调用取消上一条段链：还在等起始段的旧链不再接它的 `_L`。
//!
//! `hasExitTime` 是这个换段调用的实参，不是控制器配置：它为假且
//! `playEndMotion` 为真时，先播上一族的结束段 `_E` 并等它播完；为真则
//! 跳过。位移的换段调用全部不播结束段——走姿与待机换段不是
//! `hasExitTime` 为真、就是 `playEndMotion` 为假，转身同样——所以这里没有
//! `_E`。SD 角色的控制器没有资产侧过渡：31 个控制器每个一层、1179 个
//! 状态，状态过渡与 AnyState 过渡全为 0、状态速度全为 1，exit time、过渡
//! 时长、offset、打断设置无处可读，也无处可用——源的过渡只有上面这条
//! 代码发起的动态过渡。
//!
//! 过渡的权重：引擎的 [`AnimationTransitions`] 把旧段权重从 1 线性降到
//! 0、新段拿余量，与 CrossFade 的线性权重同形，这里沿用它。未移植的是
//! 过渡中途再次 CrossFade 时引擎的打断语义（引擎本体行为）：本仓按
//! [`AnimationTransitions`] 把在淡出的段逐层叠放。

use crate::entry::law::UniTaskDelay;
use crate::npc::{CharacterUnitId, MotionClips, MotionPhase};
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::{AnimatedBy, AnimationClip, AnimationTargetId};
use bevy::asset::{AssetPath, Assets, LoadState, RecursiveDependencyLoadState};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::observer::On;
use bevy::gltf::{Gltf, GltfNode};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneRoot};
use moly_assets::character as schema;
use moly_assets::json::JsonAsset;
use moly_law::path::TurnMotion;
use std::collections::HashMap;
use std::time::Duration;

/// 段间过渡时长：真源换段调用逐字面量传的过渡参数（0.5 秒），所有换段
/// 共用——待机/走姿/转身之间的每一次切换都是它。
pub(crate) const SEGMENT_BLEND: Duration = Duration::from_millis(500);

/// 位移换段调用传的播放速度：走姿、待机、转身的调用点都传 1。起始段的
/// 等待是 `段长 / 速度`。
const LOCOMOTION_SPEED: f32 = 1.0;

/// 已请求装载的角色清单（代号 → 包文件）。常驻：名册成员随 spawn 到齐，
/// 计划系统逐名现读。
#[derive(Resource)]
pub struct ManifestAssets {
    json: Handle<JsonAsset>,
}

/// 已请求装载的共享动作库。常驻：装配与换段都从它取段。
#[derive(Resource)]
pub struct MotionLibrary {
    pub gltf: Handle<Gltf>,
}

/// 相机跟随的那名成员。相机域读它的世界变换逐帧取景——真源站点相机追
/// 的是 avatar 视变换。挂载由玩家域决定（玩家实体持它；名册成员不再
/// 插——玩家域接管时把「清单首行」的旧默认撤了）。
#[derive(Component)]
pub struct AvatarRoot;

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

/// Imported local pose before any locomotion or content clip is evaluated.
/// Sparse source clips must not inherit transforms left by a previous clip.
#[derive(Component, Clone, Copy)]
pub(crate) struct CharacterRestTransform(pub Transform);

/// 角色包已挂载：装载门的一次性闩，防重复挂子场景。
#[derive(Component)]
pub struct ModelAttached;

/// 模型场景已展开；由 [`on_model_scene_ready`] 置位，装配消费后即撤。
#[derive(Component)]
pub struct ModelSceneReady;

/// 位移段种：静止帧待机、移动帧走姿、转体帧转身段，每族起始/循环两相。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// 待机起始段（`_S`）：播一遍，等满段长后接 [`Self::Idle`]。
    IdleS,
    /// 待机循环段（`_L`）。
    Idle,
    /// 走姿起始段（`_S`）：播一遍，等满段长后接 [`Self::Walk`]。
    WalkS,
    /// 走姿循环段（`_L`）。
    Walk,
    /// 转身起始段（`_S`）：播一遍，等满段长后接 [`Self::TurnL`]。
    TurnS(TurnMotion),
    /// 转身循环段（`_L`）。
    TurnL(TurnMotion),
}

/// 换段调用的族：段名去掉 `_S`/`_L`/`_E`/`_O` 后缀的那个原名。源的换段
/// 调用对同一原名直接返回，所以族相同即不换段。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MotionFamily {
    Idle,
    Walk,
    Turn(TurnMotion),
}

impl MotionKind {
    fn family(self) -> MotionFamily {
        match self {
            MotionKind::IdleS | MotionKind::Idle => MotionFamily::Idle,
            MotionKind::WalkS | MotionKind::Walk => MotionFamily::Walk,
            MotionKind::TurnS(motion) | MotionKind::TurnL(motion) => MotionFamily::Turn(motion),
        }
    }

    /// 起始段等满段长后接的循环段；循环段返回 `None`。
    fn loop_segment(self) -> Option<MotionKind> {
        match self {
            MotionKind::IdleS => Some(MotionKind::Idle),
            MotionKind::WalkS => Some(MotionKind::Walk),
            MotionKind::TurnS(motion) => Some(MotionKind::TurnL(motion)),
            MotionKind::Idle | MotionKind::Walk | MotionKind::TurnL(_) => None,
        }
    }
}

/// 一段起始段：图节点与源的等待秒数（`TimeSpan.FromSeconds(段长 / 速度)`
/// 的毫秒取整值）。
#[derive(Clone, Copy)]
struct LeadIn {
    node: AnimationNodeIndex,
    wait: f32,
}

/// 一名成员的动画驱动：播放器实体、图里各段的节点与段名、当前在播
/// 段。`playing` 装配时为 `None`——播放器经 commands 插上，装配当帧读
/// 不到，首段起播交给驱动系统。
///
/// 待机动作演出期间播放器归演出侧（[`Self::alone_holds`]）：位移驱动
/// 让位不换段，演出播完即交还；图句柄与 `playing`/`idle`/`player` 对
/// 演出侧开放（它按段名往图里补节点、经同款过渡换段、归位待机）。
#[derive(Component)]
pub struct MotionDriver {
    pub(crate) player: Entity,
    pub(crate) idle: AnimationNodeIndex,
    /// 待机起始段；库里没有该段时为 `None`（源：段不在控制器里，换段与
    /// 等待都不做，直接到循环段）。
    idle_s: Option<LeadIn>,
    walk: AnimationNodeIndex,
    /// 走姿起始段，同上。
    walk_s: Option<LeadIn>,
    /// 六个转身段基名的起始段，下标 = [`TurnMotion::index`]。
    turn_s: Vec<LeadIn>,
    /// 六个转身段基名的循环节点，下标 = [`TurnMotion::index`]。
    turn_l: Vec<AnimationNodeIndex>,
    pub(crate) playing: Option<MotionKind>,
    /// 在播起始段的等待（源 `UniTask.Delay`，DeltaTime 型）：起始段起播
    /// 时建、换段时换掉；只在 `playing` 是起始段时被读。
    lead_in_wait: Option<UniTaskDelay>,
    /// 该成员自己的动画图（图与播放器都是实体局部状态，不共享）。
    pub(crate) graph: Handle<AnimationGraph>,
    /// 待机动作演出占住播放器：true 时本驱动不换段。
    pub(crate) alone_holds: bool,
    idle_clip: String,
    walk_clip: String,
    installed_at: f32,
    probed: bool,
}

impl MotionDriver {
    fn lead_in(&self, kind: MotionKind) -> Option<LeadIn> {
        match kind {
            MotionKind::IdleS => self.idle_s,
            MotionKind::WalkS => self.walk_s,
            MotionKind::TurnS(motion) => Some(self.turn_s[motion.index()]),
            MotionKind::Idle | MotionKind::Walk | MotionKind::TurnL(_) => None,
        }
    }

    fn node(&self, kind: MotionKind) -> AnimationNodeIndex {
        match kind {
            MotionKind::Idle => self.idle,
            MotionKind::Walk => self.walk,
            MotionKind::TurnL(motion) => self.turn_l[motion.index()],
            MotionKind::IdleS | MotionKind::WalkS | MotionKind::TurnS(_) => {
                self.lead_in(kind)
                    .expect("起始段种只在库里有该段时进入")
                    .node
            }
        }
    }

    /// 换段调用的第一段：族有起始段就从起始段起，否则直接循环段。
    fn start(&self, family: MotionFamily) -> MotionKind {
        match family {
            MotionFamily::Idle if self.idle_s.is_some() => MotionKind::IdleS,
            MotionFamily::Idle => MotionKind::Idle,
            MotionFamily::Walk if self.walk_s.is_some() => MotionKind::WalkS,
            MotionFamily::Walk => MotionKind::Walk,
            MotionFamily::Turn(motion) => MotionKind::TurnS(motion),
        }
    }

    fn clip(&self, kind: MotionKind) -> String {
        let stem = |loop_clip: &str| loop_clip.strip_suffix("_L").unwrap_or(loop_clip).to_owned();
        match kind {
            MotionKind::IdleS => format!("{}_S", stem(&self.idle_clip)),
            MotionKind::Idle => self.idle_clip.clone(),
            MotionKind::WalkS => format!("{}_S", stem(&self.walk_clip)),
            MotionKind::Walk => self.walk_clip.clone(),
            MotionKind::TurnS(motion) => format!("{}_S", motion.base_name()),
            MotionKind::TurnL(motion) => format!("{}_L", motion.base_name()),
        }
    }
}

/// 源起始段的等待秒数：`TimeSpan.FromSeconds(段长 / 速度)` 取整到毫秒后
/// 由 `UniTask.Delay` 存成 float。
fn source_lead_in_seconds(clip_seconds: f32) -> f32 {
    crate::site_move::timeline::delay_seconds(f64::from(clip_seconds / LOCOMOTION_SPEED))
}

/// 角色网格外壳（米，装配帧的世界系快照）：相机取景偏移的现算量。
#[derive(Component)]
pub struct CharacterShell {
    pub height: f32,
    pub lowest: f32,
}

/// Startup：请求装载角色清单（包索引）与共享动作库。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ManifestAssets {
        json: server.load::<JsonAsset>(moly_assets::character_manifest()),
    });
    commands.insert_resource(MotionLibrary {
        gltf: server.load::<Gltf>(moly_assets::motion_library()),
    });
}

/// Update：名册成员到齐后逐名发角色包装载。清单的 `unit` 字段是代号
/// （成员 unitId 加 100——两份资产各持一半，清单持包文件、名册持身份），
/// 代码里把两侧折到同一坐标系再对。清单里没有某成员的包即响亮失败。
pub(crate) fn plan_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    manifest_files: Res<ManifestAssets>,
    npcs: Query<(Entity, &CharacterUnitId), (With<MotionClips>, Without<CharacterPack>)>,
) {
    if npcs.is_empty() {
        return; // 没有待装配的成员：名册未铺，或已全部装配
    }
    if let LoadState::Failed(err) = server.load_state(&manifest_files.json) {
        panic!("角色清单装载失败：{err:?}");
    }
    if !server.is_loaded_with_dependencies(&manifest_files.json) {
        return;
    }
    let manifest =
        schema::CharacterManifest::parse(&jsons.get(&manifest_files.json).expect("清单已到齐").0)
            .expect("角色清单解析失败");
    let rows: HashMap<u32, (&str, &str)> = manifest
        .units
        .iter()
        .map(|entry| {
            (
                entry.unit.parse::<u32>().expect("清单代号不是数字"),
                (entry.glb.as_str(), entry.rig.as_str()),
            )
        })
        .collect();
    let mut planned = 0usize;
    for (entity, unit) in &npcs {
        let code = unit
            .0
            .checked_add(100)
            .expect("成员 unitId 加 100 溢出：不是 unitId+100 形");
        let (glb, rig) = rows
            .get(&code)
            .copied()
            .unwrap_or_else(|| panic!("清单里没有成员 unit {} 的角色包（代号 {code}）", unit.0));
        commands.entity(entity).insert(CharacterPack {
            // 包内嵌贴图只在换装之前被画、只由 GPU 采样，没有读纹素的 CPU 读者。
            gltf: moly_assets::residency::load_gltf(
                &server,
                moly_assets::character_glb(glb),
                moly_assets::residency::GltfResidency::Character,
            ),
            file: glb.to_string(),
            rig: server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig}"))),
            rig_file: rig.to_string(),
        });
        planned += 1;
    }
    info!(
        "角色装配计划：{planned} 名（清单分母 {}）；相机跟随契约归玩家域（AvatarRoot 由玩家实体持）",
        manifest.units.len()
    );
}

/// Update：某名成员的角色包到齐即挂载——包场景作为成员实体的子实体
/// 展开，模型的局部原点与成员重合，成员的 Transform 带它走。每名独立
/// 过门：一名没到齐不挡别人。装载失败响亮失败。
pub fn attach_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    npcs: Query<(Entity, &CharacterUnitId, &CharacterPack), Without<ModelAttached>>,
) {
    for (npc, unit, pack) in &npcs {
        if let LoadState::Failed(err) = server.load_state(&pack.gltf) {
            panic!("unit {} 的角色包 {} 装载失败：{err:?}", unit.0, pack.file);
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(&pack.gltf)
        {
            panic!(
                "unit {} 的角色包 {} 依赖装载失败：{err:?}",
                unit.0, pack.file
            );
        }
        if !server.is_loaded_with_dependencies(&pack.gltf) {
            continue; // 这名等下一帧，不挡别人
        }
        let character = gltfs.get(&pack.gltf).expect("装载门已过");
        let Some(scene) = character.default_scene.clone() else {
            panic!("unit {} 的角色包 {} 没有默认 scene", unit.0, pack.file);
        };
        commands
            .entity(npc)
            .with_child((SceneRoot(scene), CharacterModel));
        commands.entity(npc).insert(ModelAttached);
        info!("[npc unit={}] 角色包挂载 {}", unit.0, pack.file);
    }
}

/// 全局观察者：角色模型的场景展开完毕，在成员实体上置位
/// [`ModelSceneReady`]（站点同款闩，装配消费后即撤）。事件实体是挂
/// [`SceneRoot`] 的子实体，沿父链一步回到成员。
pub fn on_model_scene_ready(
    trigger: On<SceneInstanceReady>,
    models: Query<&CharacterModel>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let model = trigger.event().entity;
    if models.get(model).is_err() {
        return; // 别人的场景（站点）
    }
    let npc = parents.get(model).expect("角色模型实体必有父").0;
    commands.entity(npc).insert(ModelSceneReady);
}

/// Update：模型场景展开后补动画目标、装播放器、把两段挂进动画图。
///
/// 逐名装配，每人一份动画图与播放器（图与播放器都是实体局部状态，
/// 不共享）。参考根见模块注释。包围盒扫网格实体的世界系顶点——外壳
/// 高与最低点是相机取景和「真的有模型吗」的现算量。
#[allow(clippy::type_complexity)]
pub(crate) fn wire_when_ready(
    mut commands: Commands,
    time: Res<Time>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    clip_assets: Res<Assets<AnimationClip>>,
    mesh_assets: Res<Assets<Mesh>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    npcs: Query<(Entity, &CharacterUnitId, &MotionClips), With<ModelSceneReady>>,
    children: Query<&Children>,
    names: Query<&Name>,
    models: Query<&CharacterModel>,
    meshes_3d: Query<&Mesh3d>,
    skinned_meshes: Query<(), With<SkinnedMesh>>,
    globals: Query<&GlobalTransform>,
    locals: Query<&Transform>,
) {
    if npcs.is_empty() {
        return;
    }
    let Some(lib) = libraries.get(&library.gltf) else {
        return; // 动作库还没到齐；闩保持，下帧再试
    };
    // 库参考根：剪辑骨名链的链首名，运行时从库读出（恰一个）。
    let mut reference_root = None;
    let mut roots = 0usize;
    for handle in &lib.nodes {
        let Some(node) = gltf_nodes.get(handle) else {
            continue;
        };
        if node.is_animation_root {
            roots += 1;
            if reference_root.is_none() {
                reference_root = Some(node.name.clone());
            }
        }
    }
    let reference_root = match (reference_root, roots) {
        (Some(name), 1) => Name::new(name),
        (None, _) => panic!("共享动作库没有动画根节点"),
        (Some(_), count) => panic!("共享动作库有 {count} 个动画根节点：参考根必须唯一"),
    };

    for (npc, unit, clips) in &npcs {
        // 模型子实体：CharacterModel 所在的那个孩子。
        let kids = children.get(npc).expect("模型挂载后有子链");
        let mut model = None;
        for kid in kids.iter() {
            if models.get(kid).is_ok() {
                model = Some(kid);
                break;
            }
        }
        let model = model.expect("ModelSceneReady 的置位者必有 CharacterModel 子实体");
        // 动画根：装载器建的场景实体不带名字，沿链下探到第一个带名字的节点。
        let mut cursor = model;
        let root = loop {
            if names.get(cursor).is_ok() {
                break cursor;
            }
            let kids = children
                .get(cursor)
                .expect("角色场景链断裂：实体无子且无名");
            if kids.len() != 1 {
                panic!("角色场景根应恰一条节点链，实际 {} 条", kids.len());
            }
            cursor = kids[0];
        };
        // 沿骨名链装配目标；栈存 (实体, 链长)，链长含该节点自己的名字。
        // 链首（depth 1）替换成库参考根名，其余照实——见模块注释。
        let mut chain: Vec<Name> = Vec::new();
        let mut stack: Vec<(Entity, usize)> = vec![(root, 1)];
        let mut targets = 0usize;
        let mut mesh_entities = 0usize;
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(-f32::MAX);
        while let Some((entity, depth)) = stack.pop() {
            // 只有带名实体装目标：无名实体是装载器派生的辅助件（如 primitive
            // 网格），不是 glTF 节点，装载器装配时也不会给它们目标。
            if let Ok(name) = names.get(entity) {
                chain.truncate(depth - 1);
                if depth == 1 {
                    chain.push(reference_root.clone());
                } else {
                    chain.push(name.clone());
                }
                commands.entity(entity).insert((
                    AnimationTargetId::from_names(chain.iter()),
                    AnimatedBy(root),
                ));
                if let Ok(pose) = locals.get(entity) {
                    commands
                        .entity(entity)
                        .insert(CharacterRestTransform(*pose));
                }
                targets += 1;
            } else {
                chain.truncate(depth);
            }
            if let Ok(mesh3d) = meshes_3d.get(entity) {
                mesh_entities += 1;
                // glTF stores a bind-pose AABB. Skinned face/hair meshes can
                // leave that box while an animation or fixture bridge moves
                // their joints, so let the shader decide visibility. Static
                // accessory meshes retain normal frustum culling.
                if skinned_meshes.get(entity).is_ok() {
                    commands.entity(entity).insert(NoFrustumCulling);
                }
                let mesh = mesh_assets
                    .get(&mesh3d.0)
                    .expect("网格实体引用的 Mesh 不在 Assets 里：装载门已过，不应发生");
                let positions = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|values| values.as_float3())
                    .expect("角色网格没有 float3 的 POSITION 属性");
                let global = globals.get(entity).expect("网格实体缺 GlobalTransform");
                for position in positions {
                    let world = global.transform_point(Vec3::from(*position));
                    min = min.min(world);
                    max = max.max(world);
                }
            }
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    let kid_depth = depth + usize::from(names.get(kid).is_ok());
                    stack.push((kid, kid_depth));
                }
            }
        }
        if mesh_entities == 0 {
            panic!("unit {} 的角色包里没有网格实体", unit.0);
        }
        let height = max.y - min.y;
        // 段并存一图，同一时刻只播一段（驱动系统负责换段过渡）。
        let idle_clip_name = format!("{}_L", clips.idle);
        let walk_clip_name = format!("{}_L", clips.walk);
        let idle = lib
            .named_animations
            .get(idle_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {idle_clip_name}"));
        let walk = lib
            .named_animations
            .get(walk_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {walk_clip_name}"));
        // 转身段：六个基名各 `_S`/`_L` 两节点。选段是运行时的（换腿帧才
        // 定），六个全进图；缺段即响亮失败——转身律选出的段必须全在图里，
        // 运行时选到不在图里的段就是断线。
        let mut turn_s = Vec::with_capacity(TurnMotion::ALL.len());
        let mut turn_l = Vec::with_capacity(TurnMotion::ALL.len());
        for motion in TurnMotion::ALL {
            let base = motion.base_name();
            let s_name = format!("{base}_S");
            let l_name = format!("{base}_L");
            let s = lib
                .named_animations
                .get(s_name.as_str())
                .unwrap_or_else(|| panic!("共享动作库没有转身段 {s_name}"));
            let l = lib
                .named_animations
                .get(l_name.as_str())
                .unwrap_or_else(|| panic!("共享动作库没有转身段 {l_name}"));
            turn_s.push(s.clone());
            turn_l.push(l.clone());
        }
        let mut graph = AnimationGraph::new();
        let idle_node = graph.add_clip(idle.clone(), 1.0, graph.root);
        let walk_node = graph.add_clip(walk.clone(), 1.0, graph.root);
        // 起始段进图，连同源等待的秒数（段长 / 速度，毫秒取整）。
        let clip_seconds = |clip: &Handle<AnimationClip>| -> f32 {
            clip_assets
                .get(clip)
                .expect("段句柄在库的 named_animations 里")
                .duration()
        };
        let mut lead_in = |clip: &Handle<AnimationClip>| LeadIn {
            node: graph.add_clip(clip.clone(), 1.0, graph.root),
            wait: source_lead_in_seconds(clip_seconds(clip)),
        };
        // 位移族的起始段可缺：源在段不在控制器里时换段与等待都不做。
        let idle_s = lib
            .named_animations
            .get(format!("{}_S", clips.idle).as_str())
            .map(&mut lead_in);
        let walk_s = lib
            .named_animations
            .get(format!("{}_S", clips.walk).as_str())
            .map(&mut lead_in);
        let turn_s_nodes: Vec<LeadIn> = turn_s.iter().map(&mut lead_in).collect();
        let turn_l_nodes: Vec<AnimationNodeIndex> = turn_l
            .iter()
            .map(|clip| graph.add_clip(clip.clone(), 1.0, graph.root))
            .collect();
        let graph_handle = graphs.add(graph);
        // 播放器按装载器约定落在动画根上；过渡组件与播放器同实体（引擎
        // 的换段过渡原语，驱动系统经它换段）。
        commands.entity(root).insert((
            AnimationPlayer::default(),
            AnimationTransitions::new(),
            AnimationGraphHandle(graph_handle.clone()),
        ));
        // 待机段一个循环的长度：装配账目行用（资产量，不发明默认值）。
        let idle_seconds = clip_assets
            .get(idle)
            .expect("段句柄在库的 named_animations 里")
            .duration();
        commands.entity(npc).insert((
            MotionDriver {
                player: root,
                idle: idle_node,
                idle_s,
                walk: walk_node,
                walk_s,
                turn_s: turn_s_nodes,
                turn_l: turn_l_nodes,
                playing: None,
                lead_in_wait: None,
                graph: graph_handle.clone(),
                alone_holds: false,
                idle_clip: idle_clip_name.clone(),
                walk_clip: walk_clip_name.clone(),
                installed_at: time.elapsed_secs(),
                probed: false,
            },
            CharacterShell {
                height,
                lowest: min.y,
            },
        ));
        commands.entity(npc).remove::<ModelSceneReady>();
        info!(
            "[npc unit={}] 装配完成：骨架目标 {targets} 网格实体 {mesh_entities} 外壳高 {height:.3} 待机段 {idle_clip_name}（{idle_seconds:.2}s）走姿段 {walk_clip_name}；起始段等待 待机 {:?} 走姿 {:?}",
            unit.0,
            idle_s.map(|lead| lead.wait),
            walk_s.map(|lead| lead.wait),
        );
    }
}

/// Transfer a completed source Director back to ordinary idle before a new
/// talk clip is admitted. The caller must release the Director first so its
/// sparse Root tracks cannot survive under a Hips-only shared animation.
pub(crate) fn resume_idle_after_fixture(world: &mut World, actor: Entity) -> Result<(), String> {
    let driver = world
        .get::<MotionDriver>(actor)
        .ok_or("completed fixture actor animator is missing")?;
    let (animator, idle) = (driver.player, driver.idle);
    let mut query = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
    let (mut player, mut transitions) = query
        .get_mut(world, animator)
        .map_err(|_| "completed fixture animation player is missing")?;
    transitions.play(&mut player, idle, SEGMENT_BLEND).repeat();
    world.get_mut::<MotionDriver>(actor).unwrap().playing = Some(MotionKind::Idle);
    Ok(())
}

/// Update：移动相位驱动动画段——在走播走姿族、驻留播待机族、转体播
/// 转身族。相位换族即发一条换段调用：起始段起播，等满段长接循环段，每
/// 步 0.5 秒过渡（段链语义见模块注释）；相位不换族不发。
///
/// 待机动作演出占住播放器时（`alone_holds`）本系统整名让位：不换段、
/// 不做段族衔接——那段期间的播放器归演出侧，它交还时已把 `playing`
/// 簿记归位（idle），本系统读到「已在播」自然不重触发。
///
/// 换段一律经引擎过渡组件，且**只在段种变化时调用**：过渡原语每次调用
/// 都把段重置到头，同段重复换等于永不播放——`playing` 的簿记不是装饰，
/// 是调用的前置条件。
pub fn drive(
    clock: Res<crate::npc_clock::NpcClock>,
    mut npcs: Query<
        (
            &CharacterUnitId,
            &MotionPhase,
            &mut MotionDriver,
            Option<&crate::talk::fixture_action::TalkFixtureActorLease>,
            Option<&crate::npc_look_at::NpcLookAt>,
        ),
        (
            Without<crate::npc_fixture_activity::NpcFixtureAnimationOwner>,
            Without<crate::npc::change_site_state::ChangeSiteClip>,
            // A cut-scene's cast member: its director owns the animator.
            Without<crate::cutscene::CutSceneCastLease>,
        ),
    >,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    let dt = clock.delta();
    for (unit, phase, mut driver, talk_lease, look_at) in &mut npcs {
        if driver.alone_holds || talk_lease.is_some_and(|lease| !lease.approaching) {
            continue; // 演出侧占着播放器：位移相位换段让位
        }
        // A look-at in flight on a standing character: the presenter's turn
        // clip (started with the tween); its end hands back to the idle clip.
        let family = match (look_at, phase) {
            (Some(look), MotionPhase::Dwelling { .. }) => MotionFamily::Turn(look.clip()),
            (_, MotionPhase::Walking | MotionPhase::FitWalking { .. }) => MotionFamily::Walk,
            (_, MotionPhase::Dwelling { .. }) => MotionFamily::Idle,
            (_, MotionPhase::Turning { motion, .. } | MotionPhase::FitTurning { motion, .. }) => {
                MotionFamily::Turn(*motion)
            }
        };
        // 同族：源的换段调用对同一原名直接返回，在播的段（起始段或循环
        // 段）照播——族内段链单调，接上 `_L` 就停在 `_L`，不回推 `_S`。
        let mut kind = match driver.playing {
            Some(playing) if playing.family() == family => playing,
            _ => driver.start(family),
        };
        // 起始段接循环段：起始段起播的那一帧建等待，此后逐帧累加 NPC
        // 帧间隔，累计达到等待秒数的那一帧换到循环段（源 `UniTask.Delay`
        // 的 DeltaTime 型：创建帧不计）。相位没变：这是段链的衔接。
        if driver.playing == Some(kind) {
            if let Some(next) = kind.loop_segment() {
                let wait = driver
                    .lead_in_wait
                    .as_mut()
                    .expect("起始段在播时必有它起播时建的等待");
                if wait.advance(dt) {
                    kind = next;
                }
            }
        }
        if driver.playing == Some(kind) {
            continue;
        }
        let old = driver.playing;
        let mut player = players
            .get_mut(driver.player)
            .expect("装配系统应已插上播放器");
        let player = &mut *player;
        let transitions = transitions
            .get_mut(driver.player)
            .expect("装配系统应已插上过渡组件")
            .into_inner();
        let animation = transitions.play(player, driver.node(kind), SEGMENT_BLEND);
        match driver.lead_in(kind) {
            // 起始段播一遍（不循环），等待从这一帧起建。
            Some(lead) => driver.lead_in_wait = Some(UniTaskDelay::new(lead.wait)),
            None => {
                animation.repeat();
                driver.lead_in_wait = None;
            }
        }
        driver.playing = Some(kind);
        let word = |kind: MotionKind| match kind {
            MotionKind::IdleS => "idle_s".to_owned(),
            MotionKind::Idle => "idle".to_owned(),
            MotionKind::WalkS => "walk_s".to_owned(),
            MotionKind::Walk => "walk".to_owned(),
            MotionKind::TurnS(motion) => format!("turn_s({})", motion.label()),
            MotionKind::TurnL(motion) => format!("turn_l({})", motion.label()),
        };
        let from = old
            .map(|kind| word(kind))
            .unwrap_or_else(|| "未起播".to_owned());
        info!(
            "[npc unit={}] 动画段 {from} -> {}（{}）",
            unit.0,
            word(kind),
            driver.clip(kind)
        );
    }
}

/// Update：每名装配后两秒报一次播放器实况——「动画真的在播」的现算
/// 证据；没有一段在播即响亮失败。
pub fn probe_playback(
    time: Res<Time>,
    mut npcs: Query<(&CharacterUnitId, &mut MotionDriver)>,
    players: Query<&AnimationPlayer>,
) {
    for (unit, mut driver) in &mut npcs {
        if driver.probed || time.elapsed_secs() - driver.installed_at < 2.0 {
            continue;
        }
        let player = players.get(driver.player).expect("装配后播放器应仍在");
        if player.playing_animations().count() == 0 {
            panic!("unit {} 起播两秒后没有任何动画在播：动画图装配断了", unit.0);
        }
        for (node, animation) in player.playing_animations() {
            info!(
                "[npc unit={}] 动画在播：node={node:?} 权重 {:.2} 已播 {:.2}s 完成 {} 次",
                unit.0,
                animation.weight(),
                animation.elapsed(),
                animation.completions()
            );
        }
        driver.probed = true;
    }
}

#[cfg(test)]
mod visibility_regressions {
    use super::*;

    #[test]
    fn animated_face_is_not_rejected_by_its_offscreen_bind_pose() {
        use bevy::camera::{
            primitives::{Aabb, Frustum, HalfSpace},
            visibility::{check_visibility, VisibleEntities},
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, check_visibility);
        // Screen edge at x=0: the bind-pose face is entirely outside, but its
        // animated joint has moved the face across the edge into the view.
        app.world_mut().spawn((
            Camera::default(),
            VisibleEntities::default(),
            Frustum {
                half_spaces: [HalfSpace::new(Vec4::new(1.0, 0.0, 0.0, 0.0)); 6],
            },
        ));
        let mut spawn = || {
            app.world_mut()
                .spawn((
                    Aabb::from_min_max(Vec3::new(-1.1, -0.1, -0.1), Vec3::new(-0.9, 0.1, 0.1)),
                    GlobalTransform::IDENTITY,
                    InheritedVisibility::VISIBLE,
                    ViewVisibility::HIDDEN,
                ))
                .id()
        };
        let face = spawn();
        let static_prop = spawn();
        app.world_mut().entity_mut(face).insert(NoFrustumCulling);
        app.update();
        assert!(app.world().get::<ViewVisibility>(face).unwrap().get());
        assert!(!app
            .world()
            .get::<ViewVisibility>(static_prop)
            .unwrap()
            .get());
    }
}
