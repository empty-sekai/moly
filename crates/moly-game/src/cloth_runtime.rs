//! 头发骨布上屏：把 rig 的 `cloth` 节装成每名成员的二级物理，逐帧
//! 消费 `moly_law::cloth` 的解算器并写回骨变换。
//!
//! # 装配（[`plan_when_wired`]）
//!
//! 骨架装配完成后（动画目标已插、播放器已装、名册成员的模型场景已
//! 展开），解析 rig 的 cloth 节：链拓扑/参数/约束表走律的 schema
//! （缺键与非法值响亮拒绝，不造默认值），绑定姿态取顶点的
//! `worldPosition`，碰撞体的骨绑定与 team 消费表按 pathId 自行解引用
//! （schema 只搬几何，绑定归本层）。每链 [`build_chain`] 一次，链骨与
//! 碰撞骨按节点名绑到场景实体——骨名缺失即响亮失败；同名多实体时
//! （源资产允许骨名重名）按档案记录的绑定姿态世界位消歧，见
//! [`WORLD_MATCH_TOL`]。链内
//! 父子关系与场景父子关系做一致性检查：写回的局部化按链内父算，场景
//! 父不一致时直写会错位，必须在装配期就响。
//!
//! # 帧次序（[`advance`]）
//!
//! 动画在 PostUpdate 的 `AnimationSystems` 里写骨的局部变换；本系统
//! 排在其后、变换传播（`TransformSystems::Propagate`）之前，推进并写
//! 回——渲染与蒙皮消费的是同一帧的布料结果。
//!
//! # 「动画姿势」的读法
//!
//! 律的 [`FrameInput`] 要每粒子的当帧动画世界位。动作库的目标链不含
//! 头发/饰品骨，这些骨的局部变换静止在绑定姿态，而布料写回又会改
//! 它们——所以动画位**不读**布料骨的现行局部变换，而是沿
//! 成员 → 模型 → … → 骨手工合成世界变换：路径上落在链骨集合里的
//! 实体取装配时快照的绑定局部姿态，其余取当帧局部变换。成员实体是
//! 场景根（无父），它的 Transform 由推进律在 Update 写好，当帧即是
//! 世界。这与对照实现「每帧先还原布料骨到绑定姿态、更新矩阵、再读
//! 动画位」逐帧等价，但不触碰场景状态。模拟在世界空间进行——世界
//! 运动惯性（律的 move/rotation influence 块）读的就是世界系的锚点
//! 位移。
//!
//! # 写回
//!
//! 位置直写 + 父→子方向反解旋转（律侧把拓扑备好给这一步）：有子者取
//! 「自身→诸子」的方向和（动画向 → 布料向），叶子取自身父边；旋转 =
//! 对齐四元数 × 动画世界旋转，方向退化时保持动画旋转；fixed 粒子
//! 位置钉在动画位、旋转仍随子线；局部化在拓扑序里做（父先于子），
//! 链内父用其**新**世界（位置与旋转），链外父用其**动画**世界，尺度
//! 一律不动。
//!
//! 风：律已具名「无风」——rig 的风参数在、风源不在，本层不造风。

use crate::character::{CharacterModel, CharacterPack, MotionDriver};
use crate::npc::CharacterUnitId;
use bevy::asset::{Assets, LoadState};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::cloth::math;
use moly_law::cloth::schema::{
    cloth_from_value, json_parse, params_from_value, ClothParamsDef, JsonValue, Selection,
};
use moly_law::cloth::solver::{
    advance_with_scratch as solve, build_chain, ChainTopology, ClothState, EvaluatedParams,
    FrameInput, SolverScratch,
};
use moly_law::cloth::{ColliderDef, WorldCollider};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// 推进统计的汇报周期。
pub const REPORT_PERIOD: Duration = Duration::from_secs(2);

/// 同名消歧的世界位容差（米）。档案为每个绑定点存了提取器绑定的那个
/// 实例的绑定姿态世界位（链骨 `vertices.worldPosition`、碰撞体
/// `boneWorld.position`），装配帧场景停在绑定姿态（链骨未被写过、
/// 动画未起帧），候选实体的合成世界位与档案值逐值比：差只许是浮点
/// 圆整（全批档案实测最大 1.76e-7），容差放在圆整噪声之上四个数量级、
/// 在相邻骨间距（厘米级）之下两个数量级。恰一命中才收，零命中或多命中
/// 都响亮失败——不猜。
const WORLD_MATCH_TOL: f32 = 1e-3;

/// 布料骨标记：链内骨实体在装配时挂上。读查询（`Without`）与写查询
/// （`With`）以此分界——链骨的 Transform 由本模块独占写，动作库不
/// 驱动它们，两者互不踩脚。动画位对链骨不用现行变换（用绑定姿态快照
/// 合成，见 [`world_transform`]）；碰撞几何是**例外读者**：碰撞骨本身
/// 可以是链骨（真源同形，随骨走），此时读链骨的现行——上一帧写回的
/// 变换（见 [`collider_world_transform`]）。
#[derive(Component)]
pub struct ClothBone;

/// 一名成员的骨布运行时（装配闩，装好即永驻）。
#[derive(Component)]
pub struct ClothRuntime {
    chains: Vec<ChainRuntime>,
    /// 链骨实体集：动画位合成时，路径上落在集合里的实体改用绑定姿态。
    chain_bones: HashSet<Entity>,
    /// 链骨的绑定局部姿态快照（实体 → 局部 Transform）。
    rest: HashMap<Entity, Transform>,
    /// 顶点总数（对账：rig 顶点数）。
    particles: usize,
    /// team 解引用后的碰撞体绑定总数（链间可重复计数）。
    collider_bindings: usize,
    /// 帧计数：advance 系统跑过的帧数。
    frames: u64,
    /// 推进子步累计。
    substeps: u64,
    /// 骨写回累计（每链每帧写 n 个骨）。
    writes: u64,
    /// 历史最大 `|pos - anim|`（米）。
    max_disp: f32,
}

/// 一条链的运行时。
struct ChainRuntime {
    name: String,
    topo: ChainTopology,
    /// 从 `topo.children` 反推的父表（`-1` = 链根）。
    parent: Vec<i32>,
    params: EvaluatedParams,
    state: ClothState,
    scratch: ChainScratch,
    /// 顶点 → 骨绑定（fixed 的位置由律钉回动画位，但旋转照写）。
    bones: Vec<BoneBinding>,
    /// team 碰撞体（pathId 解引用、停用项剔除后）。
    colliders: Vec<ColliderBinding>,
    /// 链根的链外父路径（成员不含 → 父含尾）：链根局部化的基准。
    root_parent_path: Vec<Entity>,
}

/// Capacity is owned by this chain and released with its runtime. All active
/// values are rewritten each frame; the buffers never replace live bone reads.
#[derive(Default)]
struct ChainScratch {
    anim: Vec<[f32; 3]>,
    anim_q: Vec<Quat>,
    colliders: Vec<WorldCollider>,
    new_p: Vec<Vec3>,
    new_q: Vec<Quat>,
    solver: SolverScratch,
}

/// 顶点绑定。
struct BoneBinding {
    entity: Entity,
    /// 成员（不含）→ 本骨（含）的实体路径。
    path: Vec<Entity>,
    fixed: bool,
}

/// 碰撞体绑定。
struct ColliderBinding {
    def: ColliderDef,
    /// 成员（不含）→ 碰撞骨（含）的实体路径。
    path: Vec<Entity>,
}

// ---------------------------------------------------------------- rig 解析

/// 解析产物：律侧结构与本层自行解引用的绑定信息平行存放。
struct ParsedRig {
    rig: moly_law::cloth::schema::ClothRig,
    /// 与 `rig.chains` 平行。
    chains: Vec<ParsedChain>,
    /// 与 `rig.colliders` 平行。
    colliders: Vec<ParsedCollider>,
}

struct ParsedChain {
    params: ClothParamsDef,
    /// 顶点绑定世界位（rest）。
    rest: Vec<[f32; 3]>,
    /// team 消费的碰撞体 pathId 表。
    team: Vec<i64>,
    /// 组件停用开关。
    enabled: bool,
}

struct ParsedCollider {
    path_id: i64,
    bone: String,
    /// 提取器绑定的碰撞骨的绑定姿态世界位（`boneWorld.position`）：
    /// 同名骨多实体时的消歧依据。
    bone_world: [f32; 3],
    enabled: bool,
    is_global: bool,
}

fn err_at(what: &str, key: &str) -> String {
    format!("cloth 节 {what}（键 `{key}`）")
}

/// rig 的布尔两形并存（`1` 与 `true`），与律的 schema 同口径都收。
fn bool_of(v: &JsonValue, key: &str) -> Result<bool, String> {
    let x = v.get(key).ok_or_else(|| err_at("缺 bool", key))?;
    match x {
        JsonValue::Bool(b) => Ok(*b),
        JsonValue::Num(n) => Ok(*n != 0.0),
        _ => Err(err_at("bool 形状不对", key)),
    }
}

fn i64_of(v: &JsonValue, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as i64)
        .ok_or_else(|| err_at("缺整数", key))
}

fn str_of(v: &JsonValue, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| err_at("缺字符串", key))
}

fn vec3_of(v: &JsonValue, key: &str) -> Result<[f32; 3], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| err_at("世界位数组不是 3 数组", key))?;
    let mut out = [0.0f32; 3];
    for (i, e) in a.iter().enumerate() {
        out[i] = e.as_f64().ok_or_else(|| err_at("世界位非数", key))? as f32;
    }
    Ok(out)
}

/// rig 根文本 → 律侧结构 + 绑定信息。schema 翻译链/参数/碰撞几何；
/// 绑定面（骨名、pathId、team、停用开关）由本层读同一棵 JSON 树。
fn parse_rig(raw: &str) -> Result<ParsedRig, String> {
    let root = json_parse(raw.as_bytes())?;
    let rig = cloth_from_value(&root)?;
    let cloth = root.get("cloth").unwrap_or(&root);
    let empty = [];
    let comps = cloth
        .get("components")
        .and_then(|v| v.as_array())
        .ok_or_else(|| err_at("缺 components", "cloth"))?;
    if comps.len() != rig.chains.len() {
        return Err("cloth 节 components 与 schema 链表长度不一致".to_string());
    }
    let mut chains = Vec::with_capacity(comps.len());
    for comp in comps {
        let params = params_from_value(comp)?;
        let verts = comp
            .get("vertices")
            .ok_or_else(|| err_at("缺 vertices", "component"))?;
        let world = verts
            .get("worldPosition")
            .and_then(|v| v.as_array())
            .ok_or_else(|| err_at("缺 worldPosition", "vertices"))?;
        let mut rest = Vec::with_capacity(world.len());
        for p in world {
            rest.push(vec3_of(p, "worldPosition")?);
        }
        let team = comp
            .get("team")
            .and_then(|t| t.get("colliderPathIds"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| err_at("缺 team.colliderPathIds", "component"))?
            .iter()
            .map(|v| v.as_f64().map(|n| n as i64))
            .collect::<Option<Vec<i64>>>()
            .ok_or_else(|| err_at("team.colliderPathIds 非数", "component"))?;
        chains.push(ParsedChain {
            params,
            rest,
            team,
            enabled: bool_of(comp, "enabled")?,
        });
    }
    let cols = cloth
        .get("colliders")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if cols.len() != rig.colliders.len() {
        return Err("cloth 节 colliders 与 schema 碰撞体表长度不一致".to_string());
    }
    let mut colliders = Vec::with_capacity(cols.len());
    for c in cols {
        let bone_world = c
            .get("boneWorld")
            .and_then(|b| b.get("position"))
            .ok_or_else(|| err_at("缺 boneWorld.position", "collider"))?;
        colliders.push(ParsedCollider {
            path_id: i64_of(c, "pathId")?,
            bone: str_of(c, "bone")?,
            bone_world: vec3_of(bone_world, "boneWorld.position")?,
            enabled: bool_of(c, "enabled")?,
            is_global: bool_of(c, "isGlobal")?,
        });
    }
    Ok(ParsedRig {
        rig,
        chains,
        colliders,
    })
}

// ---------------------------------------------------------------- 装配

/// Update：骨架装配完成的成员解析 cloth 节、绑骨、建链。闩在
/// NPC的[`MotionDriver`]或SD玩家的AvatarDriver上（场景已展开、动画目标已插）；档案装载失败或
/// 任何数据面缺口响亮失败——装载面按 schema 的拒绝语义走，缺键即
/// 停，不造默认值。
#[allow(clippy::type_complexity)]
pub fn plan_when_wired(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    npcs: Query<
        (Entity, &CharacterUnitId, &CharacterPack),
        (Or<(With<MotionDriver>, With<crate::player_avatar::AvatarDriver>)>, Without<ClothRuntime>),
    >,
    children: Query<&Children>,
    child_of: Query<&ChildOf>,
    models: Query<&CharacterModel>,
    names: Query<&Name>,
    transforms: Query<&Transform>,
) {
    for (npc, unit, pack) in &npcs {
        match server.load_state(&pack.rig) {
            LoadState::Failed(err) => {
                panic!("unit {} 的骨架档案 {} 装载失败：{err:?}", unit.0, pack.rig_file)
            }
            LoadState::Loaded => {}
            _ => continue, // 这名等下一帧，不挡别人
        }
        let Some(json) = jsons.get(&pack.rig) else {
            continue;
        };
        let parsed = match parse_rig(&json.0) {
            Ok(p) => p,
            Err(reason) => panic!(
                "unit {} 的骨架档案 {} cloth 节解析失败：{reason}",
                unit.0, pack.rig_file
            ),
        };
        // 世界合成的假设：成员实体是场景根，其当帧 Transform 即世界。
        if child_of.get(npc).is_ok() {
            panic!(
                "unit {} 的成员实体挂在别的实体下：布料的世界合成假设被破坏",
                unit.0
            );
        }
        let Some(model) = children
            .get(npc)
            .expect("模型挂载后有子链")
            .iter()
            .find(|kid| models.get(*kid).is_ok())
        else {
            panic!("unit {} 的成员实体下没有 CharacterModel 子实体", unit.0);
        };
        // 名字表：模型子树全量走一遍。链骨与碰撞骨按名绑定；同名多实体
        // 时按档案存的绑定姿态世界位消歧（见 [`WORLD_MATCH_TOL`]）。
        let mut by_name: HashMap<&str, Vec<Entity>> = HashMap::new();
        let mut stack = vec![model];
        while let Some(entity) = stack.pop() {
            if let Ok(name) = names.get(entity) {
                by_name.entry(name.as_str()).or_default().push(entity);
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        // 实体路径：从骨沿父链上溯到成员（不含），返回自上而下序。
        let path_to = |bone: Entity| -> Vec<Entity> {
            let mut reversed = Vec::new();
            let mut cursor = bone;
            for _ in 0..10_000 {
                if cursor == npc {
                    reversed.reverse();
                    return reversed;
                }
                let parent = child_of
                    .get(cursor)
                    .unwrap_or_else(|e| panic!("unit {} 的场景实体 {cursor:?} 没有父：{e:?}", unit.0))
                    .0;
                reversed.push(cursor);
                cursor = parent;
            }
            panic!("unit {} 的场景树过深：路径上溯没有回到成员实体", unit.0);
        };
        // 按名绑骨：名字唯一即收；同名多实体时逐一合成候选的世界位
        // （装配帧即绑定姿态），与档案为这个绑定点存的世界位比，恰一
        // 命中才收。源资产允许骨名重名（本批 31 份档案里 3 份有：链骨
        // 的环尾骨与链根同名、同名定位碰撞体挂在身体不同部位），档案
        // 记的绑定姿态世界位是提取器绑定的那个实例的直接痕迹。
        let world_resolved = Cell::new(0usize);
        let bind = |chain: &str, kind: &str, name: &str, expected: [f32; 3]| -> Entity {
            let candidates = by_name.get(name).unwrap_or_else(|| {
                panic!("unit {} 链 {chain} 的{kind}骨名 {name}：角色包里没有", unit.0)
            });
            if candidates.len() == 1 {
                return candidates[0];
            }
            let tol2 = WORLD_MATCH_TOL * WORLD_MATCH_TOL;
            let mut hits = Vec::with_capacity(candidates.len());
            for &candidate in candidates {
                let mut world = Transform::IDENTITY;
                for entity in path_to(candidate) {
                    let local = *transforms.get(entity).unwrap_or_else(|_| {
                        panic!("unit {} 链 {chain} 的{kind}骨 {name} 候选缺 Transform", unit.0)
                    });
                    world = world.mul_transform(local);
                }
                if world.translation.distance_squared(Vec3::from(expected)) <= tol2 {
                    hits.push(candidate);
                }
            }
            match hits.as_slice() {
                [only] => {
                    world_resolved.set(world_resolved.get() + 1);
                    *only
                }
                [] => panic!(
                    "unit {} 链 {chain} 的{kind}骨 {name} 同名 {} 处，无一世界位命中档案的绑定姿态 {expected:?}",
                    unit.0,
                    candidates.len()
                ),
                many => panic!(
                    "unit {} 链 {chain} 的{kind}骨 {name} 同名 {} 处，世界位命中 {} 个：消歧不足",
                    unit.0,
                    candidates.len(),
                    many.len()
                ),
            }
        };

        // 第一遍：全部启用链的骨绑定（链骨集合要全集成立后才解碰撞体）。
        struct Built {
            chain_index: usize,
            topo: ChainTopology,
            parent: Vec<i32>,
            params: EvaluatedParams,
            bones: Vec<BoneBinding>,
            root_parent_path: Vec<Entity>,
        }
        let mut built = Vec::new();
        let mut chain_bones: HashSet<Entity> = HashSet::new();
        let mut rest: HashMap<Entity, Transform> = HashMap::new();
        let mut particles = 0usize;
        let mut disabled_chains = 0usize;
        for (i, pchain) in parsed.chains.iter().enumerate() {
            let def = &parsed.rig.chains[i];
            if !pchain.enabled {
                disabled_chains += 1;
                continue; // 组件停用是数据语义，跳过并计数
            }
            let ev = EvaluatedParams::evaluate(&pchain.params, &def.depth);
            let topo = match build_chain(def, &ev, &pchain.rest) {
                Ok(topo) => topo,
                Err(reason) => panic!(
                    "unit {} 链 {} 建链被拒：{reason}",
                    unit.0, def.name
                ),
            };
            let n = def.bones.len();
            let mut bones = Vec::with_capacity(n);
            for (vi, bone_name) in def.bones.iter().enumerate() {
                let entity = bind(&def.name, "链", bone_name, pchain.rest[vi]);
                let local = *transforms.get(entity).unwrap_or_else(|_| {
                    panic!("unit {} 链 {} 骨 {bone_name} 缺 Transform", unit.0, def.name)
                });
                bones.push(BoneBinding {
                    entity,
                    path: path_to(entity),
                    fixed: def.selection[vi] == Selection::Fixed,
                });
                chain_bones.insert(entity);
                rest.insert(entity, local);
            }
            // 一致性：链内父 = 场景父。写回按链内父做局部化，场景父
            // 不一致时位置直写会错位——装配期必须响。
            for (vi, &p) in def.parent.iter().enumerate() {
                if p >= 0 {
                    let actual = child_of
                        .get(bones[vi].entity)
                        .expect("链骨必有父")
                        .0;
                    if actual != bones[p as usize].entity {
                        panic!(
                            "unit {} 链 {} 骨 {} 的场景父不是链内父顶点：写回局部化会错位",
                            unit.0,
                            def.name,
                            def.bones[vi]
                        );
                    }
                }
            }
            // 父表从 `topo.children` 反推（律只对上屏公开 children/order）。
            let mut parent = vec![-1i32; n];
            for p in 0..n {
                for &c in &topo.children[p] {
                    parent[c as usize] = p as i32;
                }
            }
            // 链根的链外父：链根局部化的基准（空路径 = 父即成员实体）。
            let root_entity = bones
                .iter()
                .enumerate()
                .find(|(vi, _)| def.parent[*vi] < 0)
                .map(|(_, b)| b.entity)
                .unwrap_or_else(|| panic!("unit {} 链 {} 无链根", unit.0, def.name));
            let root_parent_path = path_to(
                child_of
                    .get(root_entity)
                    .expect("链根必有场景父")
                    .0,
            );
            particles += n;
            built.push(Built {
                chain_index: i,
                topo,
                parent,
                params: ev,
                bones,
                root_parent_path,
            });
        }
        // 第二遍：team 碰撞体解引用（链骨集合已全集）。
        let mut collider_bindings = 0usize;
        let mut disabled_colliders = 0usize;
        let mut chain_bone_colliders = 0usize;
        let mut chains = Vec::with_capacity(built.len());
        for b in built {
            let pchain = &parsed.chains[b.chain_index];
            let chain_name = &parsed.rig.chains[b.chain_index].name;
            let mut colliders = Vec::with_capacity(pchain.team.len());
            for pid in &pchain.team {
                let idx = parsed
                    .colliders
                    .iter()
                    .position(|c| c.path_id == *pid)
                    .unwrap_or_else(|| {
                        panic!(
                            "unit {} 的 team 碰撞体 pathId {pid} 不在 cloth.colliders 里",
                            unit.0
                        )
                    });
                let pc = &parsed.colliders[idx];
                if !pc.enabled {
                    disabled_colliders += 1; // 停用是数据语义，剔除并计数
                    continue;
                }
                if pc.is_global {
                    panic!(
                        "unit {} 的碰撞体 pathId {pid} 是全局绑定：本批数据没有 \
                         这种碰撞体，行为未转录，不猜",
                        unit.0
                    );
                }
                let entity = bind(chain_name, "碰撞体", &pc.bone, pc.bone_world);
                // 碰撞骨同时是链骨：真源的合法形态，不是结构冲突。真源的
                // 骨读（ReadBoneFromTransform）在帧首一次读全部登记骨的
                // **当前**变换、先于模拟（UpdateStartSimulation，两条更新
                // 路同序）——链骨被布料写回之后，绑在它上面的碰撞体读到的
                // 就是上一帧的解算姿势（首帧装配态即绑定姿势，两者同一）。
                // 几何读法走 [`collider_world_transform`]（链骨取现行局部
                // 变换）。本批 31 份 rig 恰一例：unit 10 的饰品链中段骨自
                // 带一枚平面碰撞体（selection=2 钉在动画位，位置恒绑定
                // 姿，旋转随子线）。
                if chain_bones.contains(&entity) {
                    chain_bone_colliders += 1;
                }
                colliders.push(ColliderBinding {
                    def: parsed.rig.colliders[idx].clone(),
                    path: path_to(entity),
                });
            }
            collider_bindings += colliders.len();
            chains.push(ChainRuntime {
                name: chain_name.clone(),
                topo: b.topo,
                parent: b.parent,
                params: b.params,
                state: ClothState::new(b.bones.len()),
                scratch: ChainScratch::default(),
                bones: b.bones,
                colliders,
                root_parent_path: b.root_parent_path,
            });
        }
        if disabled_chains > 0 || disabled_colliders > 0 {
            warn!(
                "[npc unit={}] 骨布装配剔除停用项：链 {} 碰撞体 {}",
                unit.0, disabled_chains, disabled_colliders
            );
        }
        for chain in &chains {
            for bone in &chain.bones {
                commands.entity(bone.entity).insert(ClothBone);
            }
        }
        commands.entity(npc).insert(ClothRuntime {
            chains,
            chain_bones,
            rest,
            particles,
            collider_bindings,
            frames: 0,
            substeps: 0,
            writes: 0,
            max_disp: 0.0,
        });
        info!(
            "[npc unit={}] 骨布装配：{} 链 {} 顶点，team 碰撞体绑定 {}（链骨碰撞体 {}，同名按绑定姿态世界位解 {}）",
            unit.0,
            parsed.chains.len() - disabled_chains,
            particles,
            collider_bindings,
            chain_bone_colliders,
            world_resolved.get()
        );
    }
}

// ---------------------------------------------------------------- 推进

/// 路径上实体的世界变换：链骨取绑定姿态，其余取当帧局部变换。
/// `base` 是成员实体的当帧世界（成员无父，Transform 即世界）。
fn world_transform(
    base: &Transform,
    path: &[Entity],
    chain_bones: &HashSet<Entity>,
    rest: &HashMap<Entity, Transform>,
    live: &Query<&Transform, Without<ClothBone>>,
) -> Transform {
    let mut world = *base;
    for entity in path {
        let local = if chain_bones.contains(entity) {
            *rest
                .get(entity)
                .expect("链骨必有绑定姿态快照")
        } else {
            *live
                .get(*entity)
                .expect("路径实体缺 Transform：场景结构在装配后被改动")
        };
        world = world.mul_transform(local);
    }
    world
}

/// 碰撞骨的世界变换：路径上的链骨取**现行**局部变换（上一帧布料写回的
/// 姿势），其余取当帧动画局部。与 [`world_transform`]（动画位合成，链骨
/// 用绑定姿态快照）是两条语义：真源的骨读在帧首一次读全部登记骨的当前
/// 变换、先于模拟推进，碰撞体绑在链骨上时读到的就是它上一次被写回的
/// 姿势（首帧装配态两者同为绑定姿势）。
///
/// 边界：本系统逐链「解算→写回」，链 N 的碰撞体若引用**更早链**的骨，
/// 读到的是本帧姿势而非上一帧（真源全帧首读、恒上一帧）。本批 31 份
/// rig 的链骨碰撞体恰一例且**自引**（unit 10，碰撞体与骨同链，构建在
/// 本链写回之前），读数与真源一致；跨链引用出现时这一差要重新核。
fn collider_world_transform(
    base: &Transform,
    path: &[Entity],
    live: &Query<&Transform, Without<ClothBone>>,
    cloth_bones: &Query<&mut Transform, With<ClothBone>>,
) -> Transform {
    let mut world = *base;
    for entity in path {
        let local = match cloth_bones.get(*entity) {
            Ok(current) => *current,
            Err(_) => *live
                .get(*entity)
                .expect("路径实体缺 Transform：场景结构在装配后被改动"),
        };
        world = world.mul_transform(local);
    }
    world
}

/// PostUpdate（动画之后、变换传播之前）：逐链读当帧动画位与世界碰撞
/// 体，推进律，写回骨变换。世界合成从成员实体起（其 Transform 由推进
/// 律当帧写好），链骨用绑定姿态、链外用当帧姿势——见模块注释。
pub fn advance(
    time: Res<Time>,
    mut npcs: Query<(Entity, &CharacterUnitId, &mut ClothRuntime)>,
    live: Query<&Transform, Without<ClothBone>>,
    mut cloth_bones: Query<&mut Transform, With<ClothBone>>,
) {
    let dt = time.delta_secs();
    for (npc, unit, mut runtime) in &mut npcs {
        let base = *live
            .get(npc)
            .expect("成员实体必有 Transform");
        // 统计先攒在本地，链循环结束后一次落账（链引用与账本字段
        // 分开借，见下面的分字段借用）。
        let mut substeps = 0u64;
        let mut writes = 0u64;
        let mut max_disp = runtime.max_disp;
        let rt = &mut *runtime;
        for ci in 0..rt.chains.len() {
            // 分字段借用：链表可变、骨集合与绑定快照只读，互不冲突。
            let chain_bones = &rt.chain_bones;
            let rest = &rt.rest;
            let chain = &mut rt.chains[ci];
            let n = chain.topo.n;
            let scratch = &mut chain.scratch;
            // 当帧动画位（世界）与动画世界旋转。
            let anim = &mut scratch.anim;
            let anim_q = &mut scratch.anim_q;
            anim.clear();
            anim_q.clear();
            for bone in &chain.bones {
                let world = world_transform(&base, &bone.path, chain_bones, rest, &live);
                anim.push(world.translation.to_array());
                anim_q.push(world.rotation);
            }
            // 世界碰撞基元：碰撞骨当帧世界矩阵搬入律（链骨碰撞体读现行
            // 姿势，见 collider_world_transform）。
            let colliders = &mut scratch.colliders;
            colliders.clear();
            for c in &chain.colliders {
                let world = collider_world_transform(&base, &c.path, &live, &cloth_bones);
                colliders.push(WorldCollider::from_def(
                    &c.def,
                    world.translation.to_array(),
                    [
                        world.rotation.x,
                        world.rotation.y,
                        world.rotation.z,
                        world.rotation.w,
                    ],
                    world.scale.to_array(),
                ));
            }
            // 推进（律内部处理首帧播种、累加器、teleport 与 NaN 复位）。
            let anchor_q = anim_q[chain.topo.anchor as usize];
            let input = FrameInput {
                anim: anim.as_slice(),
                anchor_rot: [anchor_q.x, anchor_q.y, anchor_q.z, anchor_q.w],
                colliders: colliders.as_slice(),
            };
            let stats = match solve(
                &mut chain.state, &chain.topo, &chain.params, &input, dt, &mut scratch.solver,
            ) {
                Ok(stats) => stats,
                Err(reason) => panic!("unit {} 链 {} 推进被拒：{reason}", unit.0, chain.name),
            };
            // 写回：位置直写 + 父→子方向反解旋转。拓扑序父先于子，
            // 链内父的新世界边算边用。
            let pos = &chain.state.pos;
            let new_p = &mut scratch.new_p;
            let new_q = &mut scratch.new_q;
            new_p.resize(n, Vec3::ZERO);
            new_q.resize(n, Quat::IDENTITY);
            new_p.fill(Vec3::ZERO);
            new_q.fill(Quat::IDENTITY);
            for &v in &chain.topo.order {
                let vi = v as usize;
                let p = chain.parent[vi];
                // 方向：有子者取「自身→诸子」方向和；叶子取自身父边。
                let mut dir_anim = Vec3::ZERO;
                let mut dir_pos = Vec3::ZERO;
                let mut dir_ok = false;
                if !chain.topo.children[vi].is_empty() {
                    for &k in &chain.topo.children[vi] {
                        let ki = k as usize;
                        dir_anim += Vec3::from(anim[ki]) - Vec3::from(anim[vi]);
                        dir_pos += Vec3::from(pos[ki]) - Vec3::from(pos[vi]);
                    }
                    dir_ok = true;
                } else if p >= 0 {
                    let pi = p as usize;
                    dir_anim = Vec3::from(anim[vi]) - Vec3::from(anim[pi]);
                    dir_pos = Vec3::from(pos[vi]) - Vec3::from(pos[pi]);
                    dir_ok = true;
                }
                // 旋转 = 对齐四元数（动画向 → 布料向）× 动画世界旋转；
                // 方向退化时保持动画旋转。
                let wq = if dir_ok
                    && dir_anim.length_squared() > 1e-18
                    && dir_pos.length_squared() > 1e-18
                {
                    let q = math::quat_normalize(math::from_unit_vectors(
                        dir_anim.normalize().to_array(),
                        dir_pos.normalize().to_array(),
                    ));
                    Quat::from_xyzw(q[0], q[1], q[2], q[3]) * anim_q[vi]
                } else {
                    anim_q[vi]
                };
                // 位置：fixed 钉在动画位（旋转仍随子线），move 直写布料位。
                let wp = if chain.bones[vi].fixed {
                    Vec3::from(anim[vi])
                } else {
                    Vec3::from(pos[vi])
                };
                // 局部化基准：链内父用新世界；链外父用动画世界。
                let (pp, pq, ps) = if p >= 0 {
                    let pi = p as usize;
                    let scale = rest
                        .get(&chain.bones[pi].entity)
                        .expect("链骨必有绑定姿态快照")
                        .scale;
                    (new_p[pi], new_q[pi], scale)
                } else {
                    let world = world_transform(
                        &base,
                        &chain.root_parent_path,
                        chain_bones,
                        rest,
                        &live,
                    );
                    (world.translation, world.rotation, world.scale)
                };
                let inv = pq.inverse();
                let mut bone = cloth_bones
                    .get_mut(chain.bones[vi].entity)
                    .expect("链骨实体必有 Transform");
                bone.translation = (inv * (wp - pp)) / ps;
                bone.rotation = inv * wq;
                // 尺度不动（绑定姿态的尺度保持）。
                new_p[vi] = wp;
                new_q[vi] = wq;
            }
            substeps += stats.substeps as u64;
            writes += n as u64;
            if stats.max_disp > max_disp {
                max_disp = stats.max_disp;
            }
        }
        rt.substeps += substeps;
        rt.writes += writes;
        rt.max_disp = max_disp;
        rt.frames += 1;
    }
}

/// PostUpdate 周期汇报：每成员的链数/顶点数/推进子步/写回次数与复位
/// 计数——「布料真的在跑」的现算证据。
pub fn report(npcs: Query<(&CharacterUnitId, &ClothRuntime)>) {
    if !bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
        return;
    }
    for (unit, runtime) in &npcs {
        let nan = runtime.chains.iter().map(|c| c.state.nan_resets).sum::<u32>();
        let teleport = runtime
            .chains
            .iter()
            .map(|c| c.state.teleport_resets)
            .sum::<u32>();
        debug!(
            "[npc unit={}] 骨布推进：{} 链 {} 顶点 {} 碰撞体绑定 · 帧 {} 子步累计 {} 写回累计 {} max|pos-anim| {:.3}m NaN复位 {} teleport复位 {}",
            unit.0,
            runtime.chains.len(),
            runtime.particles,
            runtime.collider_bindings,
            runtime.frames,
            runtime.substeps,
            runtime.writes,
            runtime.max_disp,
            nan,
            teleport
        );
    }
}
