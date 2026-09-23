//! SD角色上屏：名册成员和玩家外观复用角色包/骨架装配，共享动作库。
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
//! 段的选择：移动相位（npc 域推进律的输出）在走播走姿段、驻留播待机
//! 段、转体播转身段。段名 = 族名 + 段后缀：位移族取名册 locomotion 的
//! 两列加 `_L` 循环后缀；转身段取转身律选出的基名（恒 `mov_cw_normal`
//! 族——选段常量如此，与角色自身的位移族无关）加 `_S`/`_L`。相位变即
//! 换段，统一 0.5 秒过渡（真源换段调用的过渡参数）。起始段播完自动接
//! 循环段（`_S` → `_L`，与段族的游戏分段约定一致）；转体结束换走姿段
//! 是族间直换（真源换段调用不播结束段）。
//!
//! 引擎侧的过渡原语（[`AnimationTransitions`]）以旧段淡出、新段即刻
//! 顶上实现过渡——两端点与真源的 crossFade 一致，中间形状是引擎近似。
//! 真源换段调用里的 `hasExitTime`（等当前段到退出点再切）是动画状态机
//! 的资产侧配置，无源可读，未接：相位变即切。

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

/// 位移段种：静止帧待机、移动帧走姿、转体帧转身段（起始/循环两相）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    Idle,
    Walk,
    /// 转身起始段（`_S`）：播一遍，播完由驱动系统接 [`Self::TurnL`]。
    TurnS(TurnMotion),
    /// 转身循环段（`_L`）：起始段播完后循环。
    TurnL(TurnMotion),
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
    walk: AnimationNodeIndex,
    /// 六个转身段基名的起始节点，下标 = [`TurnMotion::index`]。
    turn_s: Vec<AnimationNodeIndex>,
    /// 六个转身段基名的循环节点，下标 = [`TurnMotion::index`]。
    turn_l: Vec<AnimationNodeIndex>,
    pub(crate) playing: Option<MotionKind>,
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
    fn node(&self, kind: MotionKind) -> AnimationNodeIndex {
        match kind {
            MotionKind::Idle => self.idle,
            MotionKind::Walk => self.walk,
            MotionKind::TurnS(motion) => self.turn_s[motion.index()],
            MotionKind::TurnL(motion) => self.turn_l[motion.index()],
        }
    }

    /// 该段是否循环（起始段播一遍后由衔接逻辑接管，不自身循环）。
    fn loops(kind: MotionKind) -> bool {
        matches!(
            kind,
            MotionKind::Idle | MotionKind::Walk | MotionKind::TurnL(_)
        )
    }

    fn clip(&self, kind: MotionKind) -> String {
        match kind {
            MotionKind::Idle => self.idle_clip.clone(),
            MotionKind::Walk => self.walk_clip.clone(),
            MotionKind::TurnS(motion) => format!("{}_S", motion.base_name()),
            MotionKind::TurnL(motion) => format!("{}_L", motion.base_name()),
        }
    }
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
/// 玩家外观显式带PlayerVisualClips，复用同一个角色包入口但不加入NPC名册。
pub(crate) fn plan_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    manifest_files: Res<ManifestAssets>,
    npcs: Query<
        (Entity, &CharacterUnitId),
        (
            Or<(
                With<MotionClips>,
                With<crate::player_avatar::PlayerVisualClips>,
            )>,
            Without<CharacterPack>,
        ),
    >,
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
            gltf: server.load::<Gltf>(moly_assets::character_glb(glb)),
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
    npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            Option<&MotionClips>,
            Option<&crate::player_avatar::PlayerVisualClips>,
        ),
        With<ModelSceneReady>,
    >,
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

    for (npc, unit, npc_clips, player_clips) in &npcs {
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
        let (idle_base, walk_base) = if let Some(clips) = player_clips {
            (clips.idle.as_str(), clips.walk.as_str())
        } else {
            let clips = npc_clips.expect("SD model must have NPC or player motion names");
            (clips.idle.as_str(), clips.walk.as_str())
        };
        let idle_clip_name = format!("{idle_base}_L");
        let walk_clip_name = format!("{walk_base}_L");
        let idle = lib
            .named_animations
            .get(idle_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {idle_clip_name}"));
        let walk = lib
            .named_animations
            .get(walk_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {walk_clip_name}"));
        if let Some(clips) = player_clips {
            let run_clip_name = format!("{}_L", clips.run);
            let run = lib
                .named_animations
                .get(run_clip_name.as_str())
                .unwrap_or_else(|| panic!("SD玩家共享动作库没有冲刺段 {run_clip_name}"));
            let mut graph = AnimationGraph::new();
            let idle_node = graph.add_clip(idle.clone(), 1.0, graph.root);
            let walk_node = graph.add_clip(walk.clone(), 1.0, graph.root);
            let run_node = graph.add_clip(run.clone(), 1.0, graph.root);
            let graph_handle = graphs.add(graph);
            commands.entity(root).insert((
                AnimationPlayer::default(),
                AnimationTransitions::new(),
                AnimationGraphHandle(graph_handle.clone()),
            ));
            commands.entity(npc).insert((
                crate::player_avatar::AvatarDriver::new_sd(
                    root,
                    model,
                    graph_handle,
                    [idle_node, walk_node, run_node],
                    [
                        idle_clip_name.clone(),
                        walk_clip_name.clone(),
                        run_clip_name.clone(),
                    ],
                    lib.named_animations
                        .iter()
                        .map(|(name, clip)| (name.to_string(), clip.clone()))
                        .collect(),
                    time.elapsed_secs(),
                ),
                CharacterShell {
                    height,
                    lowest: min.y,
                },
            ));
            commands.entity(npc).remove::<ModelSceneReady>();
            info!("[player] SD unit={} wired: animator={root:?} model={model:?}, targets={targets}, meshes={mesh_entities}, idle={idle_clip_name}, walk={walk_clip_name}, run={run_clip_name}", unit.0);
            continue;
        }
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
        let turn_s_nodes: Vec<AnimationNodeIndex> = turn_s
            .iter()
            .map(|clip| graph.add_clip(clip.clone(), 1.0, graph.root))
            .collect();
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
                walk: walk_node,
                turn_s: turn_s_nodes,
                turn_l: turn_l_nodes,
                playing: None,
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
            "[npc unit={}] 装配完成：骨架目标 {targets} 网格实体 {mesh_entities} 外壳高 {height:.3} 待机段 {idle_clip_name}（{idle_seconds:.2}s）走姿段 {walk_clip_name}",
            unit.0
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

/// Update：移动相位驱动动画段——在走播走姿、驻留播待机、转体播转身段；
/// 起始段播完自动接同段循环段（`_S` → `_L`，段族的衔接约定）。相位变即
/// 换段，统一 0.5 秒过渡（换段语义见模块注释）。
///
/// 待机动作演出占住播放器时（`alone_holds`）本系统整名让位：不换段、
/// 不做段族衔接——那段期间的播放器归演出侧，它交还时已把 `playing`
/// 簿记归位（idle），本系统读到「已在播」自然不重触发。
///
/// 换段一律经引擎过渡组件，且**只在段种变化时调用**：过渡原语每次调用
/// 都把段重置到头，同段重复换等于永不播放——`playing` 的簿记不是装饰，
/// 是调用的前置条件。
pub fn drive(
    mut npcs: Query<
        (
            &CharacterUnitId,
            &MotionPhase,
            &mut MotionDriver,
            Option<&crate::talk::fixture_action::TalkFixtureActorLease>,
        ),
        Without<crate::npc_fixture_activity::NpcFixtureAnimationOwner>,
    >,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    for (unit, phase, mut driver, talk_lease) in &mut npcs {
        if driver.alone_holds || talk_lease.is_some_and(|lease| !lease.approaching) {
            continue; // 演出侧占着播放器：位移相位换段让位
        }
        let mut kind = match phase {
            MotionPhase::Walking | MotionPhase::FitWalking { .. } => MotionKind::Walk,
            MotionPhase::Dwelling { .. } => MotionKind::Idle,
            MotionPhase::Turning { motion, .. } | MotionPhase::FitTurning { motion, .. } => {
                // 段链在转体相位内单调：已接上 `_L` 就停在 `_L`，不再按
                // 相位回推 `_S`——否则接段后下一帧又把起始段重置到头，段
                // 链在 S/L 之间弹跳（实测 5ms 一次）。换转体（不同的段）
                // 走 `_S` 重新起链。
                match driver.playing {
                    Some(MotionKind::TurnL(playing)) if playing == *motion => {
                        MotionKind::TurnL(*motion)
                    }
                    _ => MotionKind::TurnS(*motion),
                }
            }
        };
        let mut player = players
            .get_mut(driver.player)
            .expect("装配系统应已插上播放器");
        // 起始段接循环段：还在转体而 `_S` 已播完（完成数到 1——起始段
        // 不循环，播完即终态），段种自己前进到 `_L`。相位没变：这不是
        // 换腿，是段族的衔接，判据「还在转体却已换到循环段」从这里读。
        if driver.playing == Some(kind) {
            if let MotionKind::TurnS(motion) = kind {
                let s_node = driver.turn_s[motion.index()];
                if player
                    .playing_animations()
                    .any(|(node, animation)| node == &s_node && animation.is_finished())
                {
                    kind = MotionKind::TurnL(motion);
                }
            }
        }
        if driver.playing == Some(kind) {
            continue;
        }
        let old = driver.playing;
        let player = &mut *player;
        let transitions = transitions
            .get_mut(driver.player)
            .expect("装配系统应已插上过渡组件")
            .into_inner();
        let animation = transitions.play(player, driver.node(kind), SEGMENT_BLEND);
        if MotionDriver::loops(kind) {
            animation.repeat();
        }
        driver.playing = Some(kind);
        let word = |kind: MotionKind| match kind {
            MotionKind::Idle => "idle".to_owned(),
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
