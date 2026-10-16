//! 单链解算器：90Hz 定步长累加器 → 世界运动惯性 → 子步
//! {Verlet 积分 → 约束迭代（碰撞→根距钳→结构距离→限角）→
//! 固定点回钉} → 回拉 → NaN 守卫。dt 显式入参，状态由调用方
//! 跨帧携带（与 [`crate::path`] 同款）。
//!
//! 式子来源对照（伪 C 反编译 → 本文件）：
//! * 子步数：`CalcMaxUpdateCount`（UpdateTimeManager 90Hz 档；
//!   demo SIM_HZ=90、子步上限 6、帧 dt 钳 [0,0.1]）；
//! * Verlet：`ForceAndVelocityJob`——阻尼 `(1-drag)^updatePower`
//!   （整子步 updatePower=1）、速度上限 `maxVelocity·dt`、
//!   重力/外力 `·dt²`；
//! * 根距钳制：`ClampDistanceJob`——目标 `clamp(d, minR·L, maxR·L)`、
//!   `L = len·scaleRatio`，修正按 `clamp01(1-velInfl·0.5)` 逐迭代
//!   逼近（velInfl = clampDistanceVelocityInfluence，sd 数据 0.2）；
//! * 结构距离：`RestoreDistanceJob`——刚度=逐源粒子深度求值，
//!   分摊比 `(mass_j+5·infl_j)/(5·infl_i+mass_i+mass_j+5·infl_j)`，
//!   逐源粒子求和后除以条目数均摊；
//! * 限角：`CompositeRotationJob`（clamp 半）——与动画方向夹角超
//!   `GetClampRotationAngle(algorithm)·π/180` 时钳到限、长度保持；
//! * 碰撞：`CollisionJob` 的三种 Detection（见 collider.rs）；
//! * 固定点：`FixPositionJob` 在迭代环后回钉。
//!
//! # 已知形差（诚实挂账，见 mod.rs 表与 REPORT）
//!
//! * 回拉（restoreRotation）：demo 形 = 每子步一次、向动画方向
//!   按 `power` slerp；反编译的 CompositeRotation（restore 半）
//!   注册在迭代环内、内部式（四元数链传播 + `pow(p/90,0.5)` 系
//!   权重）只部分可读——本律采 demo 形并在迭代环外执行一次，
//!   两读法的差异在 REPORT 具名。
//! * 约束修正的速度吸收：Magica 用独立速度缓冲按 `(1-velInfl)`
//!   记账；本律 pos/prev 形下修正全额进速度（PBD 惯例）。稳态
//!   等价、瞬态有差。
//! * 约束读位：反编译同迭代内并行读位置（job 系统序不保证）；
//!   本律顺序投影（Gauss-Seidel）。

use super::collider::WorldCollider;
use super::math;
use super::schema::{ChainDef, ClothParamsDef, Selection};

/// 90Hz 定步长（UpdateTimeManager 的 90 档；demo 同值）。
pub const SIM_HZ: f32 = 90.0;
/// 一次子步的物理 dt。
pub const SIM_DT: f32 = 1.0 / SIM_HZ;
/// 每帧子步上限（demo `cloth.js` 同值；超出的时间留在累加器里）。
pub const MAX_SUBSTEPS: u32 = 6;
/// 约束迭代次数（demo `iterations=4`；反编译 SolverIterationCount
/// 是 manager 配置值，demo 取 4 为用户验收形）。
pub const CONSTRAINT_ITERATIONS: u32 = 4;
/// 帧 dt 上限（demo：钳 [0, 0.1]）。
pub const MAX_FRAME_DT: f32 = 0.1;

/// `useResetTeleport=false` 链的兜底复位位移（米）。**我方选择**：
/// 反编译只读得出数据阈值臂，demo 以 1.0m 兜底挡超大跳变，本律
/// 沿用并具名。
pub const FALLBACK_TELEPORT_DISTANCE: f32 = 1.0;

/// 拓扑与约束表（[`build_chain`] 一次建好，跨帧复用）。
/// 侧表（fixed/parent/限角/回拉幂）由构建时折叠，只读。
#[derive(Debug, Clone, PartialEq)]
pub struct ChainTopology {
    /// 顶点数。
    pub n: usize,
    /// 拓扑序（父先于子）。
    pub order: Vec<u32>,
    /// 链根（anchor）顶点下标：拓扑序里第一个 parent<0 者。
    pub anchor: u32,
    /// 子表（上屏接线层写回旋转反解的输入）。
    pub children: Vec<Vec<u32>>,
    /// 父顶点下标（-1 = 根）。
    pub(crate) parent_of: Vec<i32>,
    /// 选择集位（true = fixed）。
    pub(crate) fixed_bits: Vec<bool>,
    /// 限角（弧度；π = 关）。
    pub(crate) clamp_angle_of: Vec<f32>,
    /// 回拉幂（0 = 关）。
    pub(crate) restore_power_of: Vec<f32>,
    /// 结构距离条目（directed 原样；`.0` 是被修的源粒子）。
    pub struct_edges: Vec<(u32, u32, f32)>,
    /// 每条结构边的刚度（按源粒子深度求值，clamp01）。
    pub struct_stiffness: Vec<f32>,
    /// 每源粒子的结构条目数（反编译 `/count` 均摊用）。
    pub struct_entry_count: Vec<u32>,
    /// 根距钳制对 `(v, r, len)`。
    pub root_pairs: Vec<(u32, u32, f32)>,
}

/// 每粒子参数（BezierParam 已按深度求值；`algorithm` 的 `-2` 槽
/// 分派已做掉）。
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedParams {
    /// 碰撞粒子半径。
    pub radius: Vec<f32>,
    /// 阻尼系数（速度乘 `(1-drag)`）。
    pub drag: Vec<f32>,
    /// 速度上限（米/秒；无穷 = 不设限）。
    pub max_velocity: Vec<f32>,
    /// 限角（弧度；π = 关）。
    pub clamp_angle: Vec<f32>,
    /// 回拉幂（0 = 关）。
    pub restore_power: Vec<f32>,
    /// 结构距离刚度（0..1）。
    pub struct_stiffness: Vec<f32>,
    /// 重力加速度向量（方向×幅值；sd 数据面全零）。
    pub gravity: Vec<[f32; 3]>,
    /// 世界位移影响（clamp01）。
    pub move_influence: Vec<f32>,
    /// 世界旋转影响（clamp01）。
    pub rotation_influence: Vec<f32>,
    /// 质量分摊用质量（结构距离）。
    pub mass: Vec<f32>,
    /// 质量影响标量（`massInfluence`）。
    pub mass_influence: f32,
    pub use_collision: bool,
    pub use_clamp_distance: bool,
    pub clamp_min_ratio: f32,
    pub clamp_max_ratio: f32,
    pub clamp_vel_influence: f32,
    /// 米/秒（世界惯性截流）。
    pub max_move_speed: f32,
    /// 弧度/秒。
    pub max_rotation_speed: f32,
    pub use_reset_teleport: bool,
    pub teleport_distance: f32,
    pub teleport_rotation: f32,
    /// 外力（加速度量纲，与重力同路 `·dt²`）。**站点具名无风**
    /// （未决②）：默认零，留给未来风源接线，本律不造风。
    pub external_force: [f32; 3],
    /// team 缩放比（骨缩放；默认 1）。
    pub scale_ratio: f32,
}

impl EvaluatedParams {
    /// 从 rig 参数定义 + 每粒子深度求值。曲线分派按
    /// `GetClampRotationAngle` / `GetRestoreRotationPower` 反编译：
    /// `algorithm==1` 取 `-2` 槽，否则取主槽。
    pub fn evaluate(def: &ClothParamsDef, depth: &[f32]) -> Self {
        let deg2rad = core::f32::consts::PI / 180.0;
        let count = depth.len();
        let eval_all = |p: &super::bezier::BezierParam| -> Vec<f32> {
            depth.iter().map(|&d| p.evaluate(d)).collect()
        };
        let angle_param = if def.algorithm == 1 {
            &def.clamp_rotation_angle_alt
        } else {
            &def.clamp_rotation_angle
        };
        let restore_param = if def.algorithm == 1 {
            &def.restore_rotation_alt
        } else {
            &def.restore_rotation
        };
        let gravity_mag = eval_all(&def.gravity);
        let gravity = gravity_mag
            .iter()
            .map(|&m| {
                if def.use_gravity {
                    [
                        def.gravity_direction[0] * m,
                        def.gravity_direction[1] * m,
                        def.gravity_direction[2] * m,
                    ]
                } else {
                    [0.0; 3]
                }
            })
            .collect();
        Self {
            radius: eval_all(&def.radius),
            drag: if def.use_drag {
                eval_all(&def.drag)
            } else {
                vec![0.0; count]
            },
            max_velocity: if def.use_max_velocity {
                eval_all(&def.max_velocity)
            } else {
                vec![f32::INFINITY; count]
            },
            clamp_angle: if def.use_clamp_rotation {
                eval_all(angle_param)
                    .into_iter()
                    .map(|a| a * deg2rad)
                    .collect()
            } else {
                vec![core::f32::consts::PI; count]
            },
            restore_power: if def.use_restore_rotation {
                eval_all(restore_param)
            } else {
                vec![0.0; count]
            },
            struct_stiffness: eval_all(&def.struct_distance_stiffness)
                .into_iter()
                .map(|s| s.clamp(0.0, 1.0))
                .collect(),
            gravity,
            move_influence: eval_all(&def.world_move_influence),
            rotation_influence: eval_all(&def.world_rotation_influence),
            mass: eval_all(&def.mass),
            mass_influence: def.mass_influence,
            use_collision: def.use_collision,
            use_clamp_distance: def.use_clamp_distance_ratio,
            clamp_min_ratio: def.clamp_distance_min_ratio,
            clamp_max_ratio: def.clamp_distance_max_ratio,
            clamp_vel_influence: def.clamp_distance_velocity_influence,
            max_move_speed: def.max_move_speed,
            max_rotation_speed: def.max_rotation_speed * deg2rad,
            use_reset_teleport: def.use_reset_teleport,
            teleport_distance: def.teleport_distance,
            teleport_rotation: def.teleport_rotation * deg2rad,
            external_force: [0.0; 3],
            scale_ratio: 1.0,
        }
    }
}

/// 一帧推进的统计回执。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StepStats {
    /// 本帧实际子步数。
    pub substeps: u32,
    /// 本帧发生了 teleport 复位。
    pub teleport_reset: bool,
    /// 本帧发生了 NaN 复位。
    pub nan_reset: bool,
    /// 末子步后 `max |pos - anim|`（米）。
    pub max_disp: f32,
}

/// 跨帧状态（调用方持有）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClothState {
    /// 粒子世界位置。
    pub pos: Vec<[f32; 3]>,
    /// 上一子步位置（Verlet 速度 = pos - prev）。
    pub prev: Vec<[f32; 3]>,
    /// 首帧播种闩。
    pub inited: bool,
    /// 链根上帧动画位置（世界运动惯性基准）。
    pub anchor_prev_pos: [f32; 3],
    /// 链根上帧世界旋转。
    pub anchor_prev_rot: [f32; 4],
    /// 定步长累加器残量。
    pub accumulator: f32,
    /// NaN 复位累计（诊断）。
    pub nan_resets: u32,
    /// teleport 复位累计（诊断）。
    pub teleport_resets: u32,
}

impl ClothState {
    pub fn new(n: usize) -> Self {
        Self {
            pos: vec![[0.0; 3]; n],
            prev: vec![[0.0; 3]; n],
            inited: false,
            anchor_prev_pos: [0.0; 3],
            anchor_prev_rot: math::quat_identity(),
            accumulator: 0.0,
            nan_resets: 0,
            teleport_resets: 0,
        }
    }
}

/// 一帧的输入（全为当帧值，不跨帧）。
#[derive(Debug, Clone)]
pub struct FrameInput<'a> {
    /// 本帧动画世界位置（每粒子）。
    pub anim: &'a [[f32; 3]],
    /// 链根骨本帧世界旋转。
    pub anchor_rot: [f32; 4],
    /// 世界碰撞基元（已由调用方按骨世界矩阵搬好）。
    pub colliders: &'a [WorldCollider],
}

/// 把 rig 链定义折成拓扑与侧表。
///
/// `rest` = 绑定姿态世界位置（每粒子）；只用于约束表缺席时的
/// 长度重建（demo 同形：表优先、长度不匹配照表继续）。
pub fn build_chain(
    def: &ChainDef,
    params: &EvaluatedParams,
    rest: &[[f32; 3]],
) -> Result<ChainTopology, String> {
    let n = def.bones.len();
    if rest.len() != n {
        return Err(format!(
            "cloth solver: 链 `{}` rest 长度 {} != 顶点数 {n}",
            def.name,
            rest.len()
        ));
    }
    let fixed_bits: Vec<bool> = def.selection.iter().map(|&s| s == Selection::Fixed).collect();

    // 拓扑序：反复放置「父已放」的顶点（demo 形）。
    let mut order = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    let mut guard = 0;
    while order.len() < n && guard <= n {
        guard += 1;
        for i in 0..n {
            if placed[i] {
                continue;
            }
            let p = def.parent[i];
            if p < 0 || placed[p as usize] {
                placed[i] = true;
                order.push(i as u32);
            }
        }
    }
    if order.len() != n {
        return Err(format!("cloth solver: 链 `{}` 拓扑成环", def.name));
    }
    let anchor = *order
        .iter()
        .find(|&&v| def.parent[v as usize] < 0)
        .ok_or_else(|| format!("cloth solver: 链 `{}` 无根顶点", def.name))?;

    let mut children = vec![Vec::new(); n];
    for i in 0..n {
        let p = def.parent[i];
        if p >= 0 {
            children[p as usize].push(i as u32);
        }
    }

    // 结构距离表：rig 有表用表（directed 原样）；缺表用 parent 边 +
    // 绑定长度重建（双向，与 rig 双向条目同形）。
    let struct_edges: Vec<(u32, u32, f32)> = if !def.struct_distance.is_empty() {
        def.struct_distance
            .iter()
            .filter(|&&(a, b, _)| (a as usize) < n && (b as usize) < n)
            .copied()
            .collect()
    } else {
        let mut out = Vec::new();
        for i in 0..n {
            let p = def.parent[i];
            if p >= 0 {
                let len = math::length(sub(rest[i], rest[p as usize]));
                out.push((i as u32, p as u32, len));
                out.push((p as u32, i as u32, len));
            }
        }
        out
    };
    let mut struct_entry_count = vec![0u32; n];
    for &(a, _, _) in &struct_edges {
        struct_entry_count[a as usize] += 1;
    }
    // 刚度按源粒子深度求值（RestoreDistanceJob：Evaluate(depth_源)·
    // updatePower 后 clamp01；整子步 updatePower=1）。
    let struct_stiffness = struct_edges
        .iter()
        .map(|&(a, _, _)| params.struct_stiffness[a as usize])
        .collect();

    // 根距钳制对：useClampDistanceRatio 门开才注册（ClothInit 条件）。
    let mut root_pairs = Vec::new();
    if params.use_clamp_distance {
        if !def.root_distance.is_empty() {
            for &(v, r, len) in def.root_distance.iter() {
                if (v as usize) < n && (r as usize) < n {
                    root_pairs.push((v, r, len));
                }
            }
        } else {
            for i in 0..n {
                let r = def.root[i];
                if r >= 0 && !fixed_bits[i] {
                    let len = math::length(sub(rest[i], rest[r as usize]));
                    root_pairs.push((i as u32, r as u32, len));
                }
            }
        }
    }

    Ok(ChainTopology {
        n,
        order,
        anchor,
        children,
        parent_of: def.parent.clone(),
        fixed_bits,
        clamp_angle_of: params.clamp_angle.clone(),
        restore_power_of: params.restore_power.clone(),
        struct_edges,
        struct_stiffness,
        struct_entry_count,
        root_pairs,
    })
}

/// Reusable working storage, separate from the simulated ClothState. Each
/// projection resets the active entries before accumulating the same edges.
#[derive(Debug, Default)]
pub struct SolverScratch {
    struct_acc: Vec<[f32; 3]>,
    struct_counts: Vec<u32>,
}

/// 一帧推进（dt 显式）。`dt` 非有限时响亮拒绝、状态不动。
pub fn advance(
    state: &mut ClothState,
    topo: &ChainTopology,
    params: &EvaluatedParams,
    input: &FrameInput,
    dt: f32,
) -> Result<StepStats, String> {
    let mut scratch = SolverScratch::default();
    advance_with_scratch(state, topo, params, input, dt, &mut scratch)
}

/// Same simulation as `advance`, with caller-owned reusable working buffers.
/// No clock, iteration count, constraint order, or particle state is cached.
pub fn advance_with_scratch(
    state: &mut ClothState,
    topo: &ChainTopology,
    params: &EvaluatedParams,
    input: &FrameInput,
    dt: f32,
    scratch: &mut SolverScratch,
) -> Result<StepStats, String> {
    if !dt.is_finite() || dt < 0.0 {
        return Err(format!("cloth solver: dt 非法（{dt}）"));
    }
    if input.anim.len() != topo.n || state.pos.len() != topo.n {
        return Err("cloth solver: 输入长度与链不一致".to_string());
    }
    if !state.inited {
        seed(state, input.anim, input.anchor_rot);
        return Ok(StepStats {
            substeps: 0,
            teleport_reset: false,
            nan_reset: false,
            max_disp: 0.0,
        });
    }

    // ---- 世界运动惯性（demo step() 的 inertia 块；数据结构 =
    // TeamData.WorldInfluence 的 now/old position/rotation）----
    let a = topo.anchor as usize;
    let a1 = input.anim[a];
    let a0 = state.anchor_prev_pos;
    let dq = math::quat_normalize(mul_quat(
        input.anchor_rot,
        math::quat_conjugate(state.anchor_prev_rot),
    ));
    let trans = sub(a1, a0);
    let t_len = math::length(trans);
    let rot_angle = 2.0 * dq[3].abs().clamp(0.0, 1.0).acos();

    // teleport 复位：useResetTeleport 链用数据阈值；其余 1.0m 兜底。
    let dist_thresh = if params.use_reset_teleport {
        params.teleport_distance
    } else {
        FALLBACK_TELEPORT_DISTANCE
    };
    let mut stats = StepStats::default();
    if dt > 0.0
        && (t_len > dist_thresh
            || (params.use_reset_teleport && rot_angle > params.teleport_rotation))
    {
        seed(state, input.anim, input.anchor_rot);
        state.teleport_resets += 1;
        stats.teleport_reset = true;
        return Ok(stats);
    }

    // 截流：maxMoveSpeed / maxRotationSpeed 是「每秒」量，乘帧 dt。
    let cap_t = params.max_move_speed * dt;
    let felt_t = if t_len > cap_t && t_len > 0.0 {
        mul(trans, cap_t / t_len)
    } else {
        trans
    };
    let cap_r = params.max_rotation_speed * dt;
    let felt_q = if rot_angle > cap_r && rot_angle > 1e-9 {
        math::quat_slerp(math::quat_identity(), dq, cap_r / rot_angle)
    } else {
        dq
    };
    for i in 0..topo.n {
        if topo.fixed_bits[i] {
            continue;
        }
        let mi = params.move_influence[i].clamp(0.0, 1.0);
        let ri = params.rotation_influence[i].clamp(0.0, 1.0);
        let rel = sub(state.pos[i], a0);
        // fullRigid = ΔR·(p-a0) - (p-a0) + trans：骨架刚性带动的那份
        let rigid_disp = {
            let r = math::quat_rotate(dq, rel);
            add(sub(r, rel), trans)
        };
        // felt = 截流平移·mi + 截流旋转·ri：粒子「感到」的那份
        let felt_rot = sub(math::quat_rotate(felt_q, rel), rel);
        let shift = sub(rigid_disp, add(mul(felt_t, mi), mul(felt_rot, ri)));
        state.pos[i] = add(state.pos[i], shift);
        state.prev[i] = add(state.prev[i], shift);
    }
    state.anchor_prev_pos = a1;
    state.anchor_prev_rot = input.anchor_rot;

    // ---- 定步长子步 ----
    let dt = dt.clamp(0.0, MAX_FRAME_DT);
    state.accumulator += dt;
    let mut n_sub = (state.accumulator / SIM_DT) as u32;
    if n_sub > 0 {
        state.accumulator -= n_sub as f32 * SIM_DT;
    }
    if n_sub > MAX_SUBSTEPS {
        n_sub = MAX_SUBSTEPS;
    }
    stats.substeps = n_sub;
    for _ in 0..n_sub {
        substep(state, topo, params, input, scratch);
    }

    // ---- NaN 守卫：任一非有限 → 整链复位并计数 ----
    if state.pos.iter().any(|p| p.iter().any(|c| !c.is_finite())) {
        seed(state, input.anim, input.anchor_rot);
        state.nan_resets += 1;
        stats.nan_reset = true;
        return Ok(stats);
    }

    let mut max_disp = 0.0f32;
    for i in 0..topo.n {
        let d = math::length(sub(state.pos[i], input.anim[i]));
        if d > max_disp {
            max_disp = d;
        }
    }
    stats.max_disp = max_disp;
    Ok(stats)
}

fn seed(state: &mut ClothState, anim: &[[f32; 3]], anchor_rot: [f32; 4]) {
    state.pos.clear();
    state.pos.extend_from_slice(anim);
    state.prev.clear();
    state.prev.extend_from_slice(anim);
    state.inited = true;
    if let Some(&p0) = anim.first() {
        state.anchor_prev_pos = p0;
    }
    state.anchor_prev_rot = anchor_rot;
}

/// 一次子步（ForceAndVelocityJob → 约束迭代 → FixPositionJob → 回拉）。
fn substep(
    state: &mut ClothState,
    topo: &ChainTopology,
    params: &EvaluatedParams,
    input: &FrameInput,
    scratch: &mut SolverScratch,
) {
    let dt = SIM_DT;
    // Verlet：fixed 先钉（prev=pos, pos=anim，速度清零）；move 走
    // 阻尼/限速/重力·dt²。
    for i in 0..topo.n {
        if topo.fixed_bits[i] {
            state.prev[i] = state.pos[i];
            state.pos[i] = input.anim[i];
            continue;
        }
        let damp = (1.0 - params.drag[i]).max(0.0);
        let mut v = mul(sub(state.pos[i], state.prev[i]), damp);
        let limit = params.max_velocity[i] * dt;
        let sp = math::length(v);
        if sp > limit && sp > 0.0 {
            v = mul(v, limit / sp);
        }
        state.prev[i] = state.pos[i];
        let accel = add(params.gravity[i], params.external_force);
        state.pos[i] = add(add(state.pos[i], v), mul(accel, dt * dt));
    }

    // 约束迭代：碰撞 → 根距钳 → 结构距离 → 限角（注册序）。
    for _ in 0..CONSTRAINT_ITERATIONS {
        if params.use_collision {
            project_collision(state, topo, params, input.colliders);
        }
        project_root_clamp(state, topo, params);
        project_struct_distance(state, topo, params, scratch);
        project_clamp_rotation(state, topo, input.anim);
    }

    // FixPositionJob 位：迭代环后回钉 fixed。
    for i in 0..topo.n {
        if topo.fixed_bits[i] {
            state.pos[i] = input.anim[i];
        }
    }

    // 回拉（restore）：demo 形——每子步一次、迭代环外、向动画方向
    // 按 power 旋转。见模块头「已知形差」。
    restore_rotation(state, topo, input.anim);
}

fn project_collision(
    state: &mut ClothState,
    topo: &ChainTopology,
    params: &EvaluatedParams,
    colliders: &[WorldCollider],
) {
    if colliders.is_empty() {
        return;
    }
    for i in 0..topo.n {
        if topo.fixed_bits[i] {
            continue;
        }
        let r = params.radius[i] * params.scale_ratio;
        for prim in colliders {
            prim.push_out(&mut state.pos[i], r);
        }
    }
}

fn project_root_clamp(state: &mut ClothState, topo: &ChainTopology, params: &EvaluatedParams) {
    if !params.use_clamp_distance {
        return;
    }
    // ClampDistanceJob：目标长 L = len·scaleRatio，夹到
    // [minR·L, maxR·L]；修正按 clamp01(1 - velInfl·0.5) 逐迭代逼近。
    let k = (1.0 - params.clamp_vel_influence * 0.5).clamp(0.0, 1.0);
    for &(v, r, len) in &topo.root_pairs {
        let vi = v as usize;
        let ri = r as usize;
        if topo.fixed_bits[vi] {
            continue;
        }
        let dir = sub(state.pos[vi], state.pos[ri]);
        let d = math::length(dir);
        if d <= 1e-6 {
            continue;
        }
        let target = len * params.scale_ratio;
        let lo = target * params.clamp_min_ratio;
        let hi = target * params.clamp_max_ratio;
        let t = d.clamp(lo, hi);
        if (t - d).abs() <= f32::EPSILON {
            continue;
        }
        let want = add(state.pos[ri], mul(dir, t / d));
        state.pos[vi] = add(state.pos[vi], mul(sub(want, state.pos[vi]), k));
    }
}

fn project_struct_distance(
    state: &mut ClothState,
    topo: &ChainTopology,
    params: &EvaluatedParams,
    scratch: &mut SolverScratch,
) {
    if topo.struct_edges.is_empty() {
        return;
    }
    let n = topo.n;
    // 反编译形：逐源粒子累加修正，除以该粒子的条目数均摊。
    scratch.struct_acc.resize(n, [0.0; 3]);
    scratch.struct_counts.resize(n, 0);
    scratch.struct_acc.fill([0.0; 3]);
    scratch.struct_counts.fill(0);
    let acc = &mut scratch.struct_acc;
    let counts = &mut scratch.struct_counts;
    for (e, &(a, b, len)) in topo.struct_edges.iter().enumerate() {
        let ai = a as usize;
        let bi = b as usize;
        if topo.fixed_bits[ai] {
            continue;
        }
        let dir = sub(state.pos[bi], state.pos[ai]);
        let d = math::length(dir);
        if d < 1e-5 {
            continue;
        }
        let target = len * params.scale_ratio;
        // 质量分摊：本粒子份额 = (mass_b + 5·infl_b) /
        //   (5·infl_a + mass_a + mass_b + 5·infl_b)
        let wi = 5.0 * params.mass_influence;
        let own = wi + params.mass[ai];
        let partner = params.mass[bi] + wi;
        let w = partner / (own + partner);
        let k = topo.struct_stiffness[e];
        let corr = mul(dir, (d - target) * k * w / d);
        acc[ai] = add(acc[ai], corr);
        counts[ai] += 1;
    }
    for i in 0..n {
        if counts[i] > 0 {
            state.pos[i] = add(state.pos[i], mul(acc[i], 1.0 / counts[i] as f32));
        }
    }
}

fn project_clamp_rotation(state: &mut ClothState, topo: &ChainTopology, anim: &[[f32; 3]]) {
    // CompositeRotationJob（clamp 半）：与动画方向夹角超限 → 钳到限
    // （长度保持当前值；limit≥π-1e-6 视作关）。
    for vi in 0..topo.n {
        let p = match parent_of(topo, vi) {
            Some(p) => p,
            None => continue,
        };
        if topo.fixed_bits[vi] {
            continue;
        }
        let limit = topo.clamp_angle_of[vi];
        if limit >= core::f32::consts::PI - 1e-6 {
            continue;
        }
        let cur_raw = sub(state.pos[vi], state.pos[p]);
        let anim_raw = sub(anim[vi], anim[p]);
        let lc = math::length(cur_raw);
        if lc < 1e-9 || math::length(anim_raw) < 1e-9 {
            continue;
        }
        let cur = mul(cur_raw, 1.0 / lc);
        let want = mul(anim_raw, 1.0 / math::length(anim_raw));
        let ang = math::angle_between(want, cur);
        if ang <= limit {
            continue;
        }
        let q = math::quat_normalize(math::from_unit_vectors(want, cur));
        let q = math::quat_slerp(math::quat_identity(), q, limit / ang);
        state.pos[vi] = add(state.pos[p], mul(math::quat_rotate(q, want), lc));
    }
}

fn restore_rotation(state: &mut ClothState, topo: &ChainTopology, anim: &[[f32; 3]]) {
    for vi in 0..topo.n {
        let p = match parent_of(topo, vi) {
            Some(p) => p,
            None => continue,
        };
        if topo.fixed_bits[vi] {
            continue;
        }
        let pow = topo.restore_power_of[vi];
        if pow <= 0.0 {
            continue;
        }
        let cur_raw = sub(state.pos[vi], state.pos[p]);
        let anim_raw = sub(anim[vi], anim[p]);
        let lc = math::length(cur_raw);
        if lc < 1e-9 || math::length(anim_raw) < 1e-9 {
            continue;
        }
        let cur = mul(cur_raw, 1.0 / lc);
        let want = mul(anim_raw, 1.0 / math::length(anim_raw));
        let q = math::quat_normalize(math::from_unit_vectors(cur, want));
        let q = math::quat_slerp(math::quat_identity(), q, pow.min(1.0));
        state.pos[vi] = add(state.pos[p], mul(math::quat_rotate(q, cur), lc));
    }
}

fn parent_of(topo: &ChainTopology, i: usize) -> Option<usize> {
    let p = topo.parent_of[i];
    if p >= 0 {
        Some(p as usize)
    } else {
        None
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// 四元数乘 `a·b`（旋转合成：先施 b 再施 a）。
fn mul_quat(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] + a[1] * b[3] + a[2] * b[0] - a[0] * b[2],
        a[3] * b[2] + a[2] * b[3] + a[0] * b[1] - a[1] * b[0],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloth::bezier::BezierParam;
    use crate::cloth::schema::ClothParamsDef;

    const DT: f32 = SIM_DT;

    /// 两顶点链：根 fixed、梢 move；约束/重力全关的最小律面。
    fn minimal_chain() -> (ChainDef, ClothParamsDef) {
        let def = ChainDef {
            name: "T".into(),
            class: "hair".into(),
            bones: vec!["root".into(), "tip".into()],
            parent: vec![-1, 0],
            root: vec![-1, 0],
            selection: vec![Selection::Fixed, Selection::Move],
            depth: vec![0.0, 1.0],
            struct_distance: vec![(0, 1, 1.0), (1, 0, 1.0)],
            root_distance: vec![(1, 0, 1.0)],
        };
        let p = ClothParamsDef {
            algorithm: 1,
            radius: BezierParam::constant(0.02),
            mass: BezierParam::constant(1.0),
            mass_influence: 0.0,
            use_gravity: false,
            gravity: BezierParam::constant(0.0),
            gravity_direction: [0.0, 1.0, 0.0],
            use_drag: false,
            drag: BezierParam::constant(0.0),
            use_max_velocity: false,
            max_velocity: BezierParam::constant(0.0),
            max_move_speed: 2.0,
            max_rotation_speed: 720.0,
            world_move_influence: BezierParam::constant(1.0),
            world_rotation_influence: BezierParam::constant(1.0),
            use_collision: false,
            use_clamp_distance_ratio: false,
            clamp_distance_min_ratio: 0.7,
            clamp_distance_max_ratio: 1.1,
            clamp_distance_velocity_influence: 0.2,
            use_clamp_rotation: false,
            clamp_rotation_angle: BezierParam::constant(20.0),
            clamp_rotation_angle_alt: BezierParam::constant(20.0),
            use_restore_rotation: false,
            restore_rotation: BezierParam::constant(0.0),
            restore_rotation_alt: BezierParam::constant(0.0),
            struct_distance_stiffness: BezierParam::constant(1.0),
            use_reset_teleport: false,
            teleport_distance: 0.2,
            teleport_rotation: 45.0,
            wind_influence: 1.0,
            wind_random_scale: 0.7,
            wind_synchronization: 0.6,
        };
        (def, p)
    }

    fn strip_constraints(def: &mut ChainDef) {
        def.struct_distance.clear();
        def.root_distance.clear();
    }

    /// 结构边表剥空后会触发 parent 重建回退（demo 同形），要真正
    /// 关掉结构距离得把刚度归零（k=0 → 修正恒零）。
    fn disable_struct_spring(p: &mut ClothParamsDef) {
        p.struct_distance_stiffness = BezierParam::constant(0.0);
    }

    fn setup(
        def: &ChainDef,
        p: &ClothParamsDef,
        anim: &[[f32; 3]],
    ) -> (ChainTopology, EvaluatedParams, ClothState) {
        let ev = EvaluatedParams::evaluate(p, &def.depth);
        let topo = build_chain(def, &ev, anim).expect("topo");
        let mut st = ClothState::new(def.bones.len());
        let input = FrameInput {
            anim,
            anchor_rot: math::quat_identity(),
            colliders: &[],
        };
        advance(&mut st, &topo, &ev, &input, DT).expect("seed");
        (topo, ev, st)
    }

    fn frame(
        st: &mut ClothState,
        topo: &ChainTopology,
        ev: &EvaluatedParams,
        anim: &[[f32; 3]],
        anchor_rot: [f32; 4],
        dt: f32,
    ) -> StepStats {
        let input = FrameInput {
            anim,
            anchor_rot,
            colliders: &[],
        };
        advance(st, topo, ev, &input, dt).expect("advance")
    }

    /// 首帧播种：不积分，位置=动画；次帧静止全零。
    #[test]
    fn first_frame_seeds() {
        let (def, p) = minimal_chain();
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        assert!(st.inited);
        assert_eq!(st.pos[1], [1.0, 0.0, 0.0]);
        assert_eq!(st.prev[1], st.pos[1]);
        let s = frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        assert_eq!(s.substeps, 1);
        assert_eq!(s.max_disp, 0.0);
        assert!(!s.teleport_reset && !s.nan_reset);
    }

    /// Verlet 重力项 `g·dt²`（ForceAndVelocityJob 力臂）：
    /// g=90 向下时一子步位移 90·(1/90)²。
    #[test]
    fn verlet_gravity_term() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_clamp_distance_ratio = false;
        p.use_gravity = true;
        p.gravity = BezierParam::constant(90.0);
        p.gravity_direction = [0.0, -1.0, 0.0];
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let expected = -90.0 * DT * DT;
        assert!((st.pos[1][1] - expected).abs() < 1e-7, "got {}", st.pos[1][1]);
    }

    /// Verlet 阻尼：v' = v·(1-drag)，drag=0.25 → 0.75 倍。
    #[test]
    fn verlet_damping() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_drag = true;
        p.drag = BezierParam::constant(0.25);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        // 手工注速度：prev=(0,0,0)、pos=(1,0,0) → v=1
        st.prev[1] = [0.0, 0.0, 0.0];
        st.pos[1] = [1.0, 0.0, 0.0];
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        assert!((st.pos[1][0] - 1.75).abs() < 1e-6, "got {}", st.pos[1][0]);
    }

    /// 速度上限：lim = maxVelocity·dt（3 m/s → 0.0333…/子步）。
    #[test]
    fn verlet_velocity_clamp() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_max_velocity = true;
        p.max_velocity = BezierParam::constant(3.0);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        st.prev[1] = [0.0, 0.0, 0.0];
        st.pos[1] = [1.0, 0.0, 0.0];
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let expected = 1.0 + 3.0 * DT;
        assert!((st.pos[1][0] - expected).abs() < 1e-6, "got {}", st.pos[1][0]);
    }

    /// fixed 每子步与迭代后都钉在动画位（带重力也钉）。
    #[test]
    fn fixed_particles_pinned() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_gravity = true;
        p.gravity = BezierParam::constant(90.0);
        p.gravity_direction = [0.0, -1.0, 0.0];
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        for _ in 0..10 {
            frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        }
        assert_eq!(st.pos[0], [0.0, 0.0, 0.0]);
        assert!(st.pos[1][1] < 0.0, "梢粒子应在掉");
    }

    /// 根距钳（ClampDistanceJob 形）：d=2 超上臂 → 每迭代按
    /// k=1-vi·0.5=0.9 逼近 1.1；4 迭代后 1.1+0.9·0.1⁴=1.10009。
    /// 下臂同形：0.5 < 0.7 → 逼近 0.7。
    #[test]
    fn root_clamp_converges() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_clamp_distance_ratio = true;
        def.root_distance = vec![(1, 0, 1.0)];
        let anim = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let expected = 1.1 + 0.9 * 1e-4;
        assert!((st.pos[1][0] - expected).abs() < 1e-4, "got {}", st.pos[1][0]);

        let anim2 = [[0.0, 0.0, 0.0], [0.5, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim2);
        frame(&mut st, &topo, &ev, &anim2, math::quat_identity(), DT);
        let e = 0.2 * 1e-4;
        assert!((st.pos[1][0] - (0.7 - e)).abs() < 1e-4, "got {}", st.pos[1][0]);
    }

    /// 结构距离（RestoreDistanceJob 形）：单条目 (1→0)，质量分摊
    /// w = (mass_0+5·infl)/(5·infl+mass_1+mass_0+5·infl)，
    /// mass=[5,1]、infl=0.3 → w=6.5/9；e 终 = (1-w)⁴。
    #[test]
    fn struct_distance_mass_split() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        p.mass = BezierParam {
            start_value: 5.0,
            end_value: 1.0,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        p.mass_influence = 0.3;
        def.struct_distance = vec![(1, 0, 1.0)];
        let anim = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let w: f32 = (5.0 + 5.0 * 0.3) / (5.0 * 0.3 + 1.0 + 5.0 + 5.0 * 0.3);
        let expected: f32 = 1.0 + (2.0 - 1.0) * (1.0 - w).powi(4);
        assert!((st.pos[1][0] - expected).abs() < 1e-4, "got {}", st.pos[1][0]);
    }

    /// 限角（CompositeRotation clamp 半）：30° 拉回限值 20°，长度保持。
    #[test]
    fn clamp_rotation_limits_angle() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_clamp_rotation = true;
        p.clamp_rotation_angle_alt = BezierParam::constant(20.0);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        let cur = [30f32.to_radians().cos(), 30f32.to_radians().sin(), 0.0];
        st.pos[1] = cur;
        st.prev[1] = cur;
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let d = math::normalize(sub(st.pos[1], st.pos[0]));
        let ang = d[1].atan2(d[0]).to_degrees();
        assert!((ang - 20.0).abs() < 1e-3, "got {ang}");
        assert!((math::length(st.pos[1]) - 1.0).abs() < 1e-5, "长度应保持");
    }

    /// 回拉（demo 形）：30°、power=0.1 → 一次子步后 27°。
    #[test]
    fn restore_pulls_toward_animated() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_restore_rotation = true;
        p.restore_rotation_alt = BezierParam::constant(0.1);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        let cur = [30f32.to_radians().cos(), 30f32.to_radians().sin(), 0.0];
        st.pos[1] = cur;
        st.prev[1] = cur;
        frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        let d = math::normalize(sub(st.pos[1], st.pos[0]));
        let ang = d[1].atan2(d[0]).to_degrees();
        assert!((ang - 27.0).abs() < 1e-3, "got {ang}");
    }

    /// 世界位移惯性：influence=1 → 粒子留世界系（感到全部运动）；
    /// influence=0 → 刚性随动。
    #[test]
    fn world_move_inertia() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim0 = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        // 位移 0.01 < 截流 cap（maxMoveSpeed·dt = 0.0222）→ 全量感到
        let anim1 = [[0.01, 0.0, 0.0], [1.01, 0.0, 0.0]];
        let s = frame(&mut st, &topo, &ev, &anim1, math::quat_identity(), DT);
        assert!((st.pos[1][0] - 1.0).abs() < 1e-6, "influence=1 应留原地");
        assert!((s.max_disp - 0.01).abs() < 1e-6);

        let mut p2 = p.clone();
        p2.world_move_influence = BezierParam::constant(0.0);
        let ev2 = EvaluatedParams::evaluate(&p2, &def.depth);
        let topo2 = build_chain(&def, &ev2, &anim0).expect("topo");
        let mut st2 = ClothState::new(2);
        frame(&mut st2, &topo2, &ev2, &anim0, math::quat_identity(), DT);
        frame(&mut st2, &topo2, &ev2, &anim1, math::quat_identity(), DT);
        assert!((st2.pos[1][0] - 1.01).abs() < 1e-6, "influence=0 应随动");
    }

    /// 世界旋转惯性：5°（在 720°/s·dt=8°/帧截流内）。
    /// influence=1 → 留世界系；influence=0 → 刚性随动到动画位。
    #[test]
    fn world_rotation_inertia() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim0 = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        let half = 2.5f32.to_radians();
        let q5 = [0.0, 0.0, half.sin(), half.cos()];
        let anim1 = [
            [0.0, 0.0, 0.0],
            [5f32.to_radians().cos(), 5f32.to_radians().sin(), 0.0],
        ];
        let s = frame(&mut st, &topo, &ev, &anim1, q5, DT);
        assert!((st.pos[1][0] - 1.0).abs() < 1e-5, "influence=1 应留原地");
        let expected = 2.0 * (2.5f32.to_radians()).sin();
        assert!((s.max_disp - expected).abs() < 1e-4);

        let mut p2 = p.clone();
        p2.world_rotation_influence = BezierParam::constant(0.0);
        let ev2 = EvaluatedParams::evaluate(&p2, &def.depth);
        let topo2 = build_chain(&def, &ev2, &anim0).expect("topo");
        let mut st2 = ClothState::new(2);
        frame(&mut st2, &topo2, &ev2, &anim0, math::quat_identity(), DT);
        frame(&mut st2, &topo2, &ev2, &anim1, q5, DT);
        let d = math::length(sub(st2.pos[1], anim1[1]));
        assert!(d < 1e-5, "influence=0 应刚性随动，残差 {d}");
    }

    /// teleport 兜底：根跳 2m > 1.0m 兜底（useResetTeleport=false）
    /// → 整链复位到动画位。
    #[test]
    fn teleport_reset_fallback() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim0 = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        st.pos[1] = [0.5, 0.3, 0.0];
        let anim1 = [[2.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
        let s = frame(&mut st, &topo, &ev, &anim1, math::quat_identity(), DT);
        assert!(s.teleport_reset);
        assert_eq!(st.teleport_resets, 1);
        assert_eq!(st.pos[1], anim1[1], "复位=动画位");
    }

    /// teleport 数据阈值臂：useResetTeleport=true 时 0.2m/45° 生效。
    #[test]
    fn teleport_reset_data_thresholds() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        p.use_reset_teleport = true;
        p.teleport_distance = 0.2;
        p.teleport_rotation = 45.0;
        let anim0 = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        let anim1 = [[0.3, 0.0, 0.0], [1.3, 0.0, 0.0]];
        let s = frame(&mut st, &topo, &ev, &anim1, math::quat_identity(), DT);
        assert!(s.teleport_reset, "位移 0.3 > 0.2 应复位");

        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        let half = 45f32.to_radians();
        let q90 = [0.0, 0.0, half.sin(), half.cos()];
        let s = frame(&mut st, &topo, &ev, &anim0, q90, DT);
        assert!(s.teleport_reset, "旋转 90° > 45° 应复位");

        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        let anim2 = [[0.1, 0.0, 0.0], [1.1, 0.0, 0.0]];
        let s = frame(&mut st, &topo, &ev, &anim2, math::quat_identity(), DT);
        assert!(!s.teleport_reset, "位移 0.1 < 0.2 不复位");
    }

    /// NaN 守卫：链上任一非有限 → 整链复位并计数。
    #[test]
    fn nan_guard_resets() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        st.pos[1] = [f32::NAN, 0.0, 0.0];
        let s = frame(&mut st, &topo, &ev, &anim, math::quat_identity(), DT);
        assert!(s.nan_reset);
        assert_eq!(st.nan_resets, 1);
        assert_eq!(st.pos[1], anim[1], "复位=动画位");
    }

    /// 累加器：dt=1/30 → 3 子步残零；dt=0.1 → 封顶 6 子步；
    /// 残量滚动补步。
    #[test]
    fn accumulator_and_substep_cap() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        let s = frame(&mut st, &topo, &ev, &anim, math::quat_identity(), 1.0 / 30.0);
        assert_eq!(s.substeps, 3);
        assert!(st.accumulator.abs() < 1e-6, "残量 {}", st.accumulator);

        let s = frame(&mut st, &topo, &ev, &anim, math::quat_identity(), 0.1);
        assert_eq!(s.substeps, MAX_SUBSTEPS);
        // demo 语义：先按未封顶 nSub 扣减累加器再封顶——超限的时间
        // 直接丢弃（防死亡螺旋），残量归零。
        assert!(st.accumulator.abs() < 1e-6, "残量 {}", st.accumulator);

        // 0.01 < 1/90 → 本帧 0 子步，时间留给下一帧
        let s = frame(&mut st, &topo, &ev, &anim, math::quat_identity(), 0.01);
        assert_eq!(s.substeps, 0, "不足一个定步长");
        assert!((st.accumulator - 0.01).abs() < 1e-9);
    }

    /// 碰撞：粒子嵌入球面时推出到 `R + r`。
    #[test]
    fn collision_pushes_out() {
        let (mut def, mut p) = minimal_chain();
        strip_constraints(&mut def);
        disable_struct_spring(&mut p);
        let anim = [[0.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
        let mut ev = EvaluatedParams::evaluate(&p, &def.depth);
        ev.use_collision = true;
        let topo = build_chain(&def, &ev, &anim).expect("topo");
        let mut st = ClothState::new(2);
        let cols = [WorldCollider::Sphere {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        }];
        let input = FrameInput {
            anim: &anim,
            anchor_rot: math::quat_identity(),
            colliders: &cols,
        };
        advance(&mut st, &topo, &ev, &input, DT).expect("seed");
        st.pos[1] = [0.0, -1.0, 0.0];
        st.prev[1] = [0.0, -1.0, 0.0];
        advance(&mut st, &topo, &ev, &input, DT).expect("step");
        let r = ev.radius[1];
        assert!(
            (math::length(st.pos[1]) - (1.0 + r)).abs() < 1e-4,
            "got {:?} r={r}",
            st.pos[1]
        );
    }

    /// dt 非法响亮拒绝（负 / NaN），状态字节不动。
    #[test]
    fn invalid_dt_rejected() {
        let (def, p) = minimal_chain();
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        let before = st.clone();
        let input = FrameInput {
            anim: &anim,
            anchor_rot: math::quat_identity(),
            colliders: &[],
        };
        assert!(advance(&mut st, &topo, &ev, &input, -1.0).is_err());
        assert!(advance(&mut st, &topo, &ev, &input, f32::NAN).is_err());
        assert_eq!(st, before);
    }

    /// 输入长度不一致响亮拒绝。
    #[test]
    fn mismatched_input_rejected() {
        let (def, p) = minimal_chain();
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim);
        let short = [[0.0, 0.0, 0.0]];
        let input = FrameInput {
            anim: &short,
            anchor_rot: math::quat_identity(),
            colliders: &[],
        };
        assert!(advance(&mut st, &topo, &ev, &input, DT).is_err());
    }

    /// `algorithm` 曲线分派：=1 取 `-2` 槽（GetClampRotationAngle /
    /// GetRestoreRotationPower 反编译），≠1 取主槽。
    #[test]
    fn algorithm_slot_dispatch() {
        let (def, mut p) = minimal_chain();
        p.use_clamp_rotation = true;
        p.use_restore_rotation = true;
        p.algorithm = 1;
        p.clamp_rotation_angle = BezierParam::constant(5.0);
        p.clamp_rotation_angle_alt = BezierParam::constant(20.0);
        p.restore_rotation = BezierParam::constant(0.5);
        p.restore_rotation_alt = BezierParam::constant(0.25);
        let ev = EvaluatedParams::evaluate(&p, &def.depth);
        assert!((ev.clamp_angle[1] - 20f32.to_radians()).abs() < 1e-6);
        assert_eq!(ev.restore_power[1], 0.25);

        let ev = {
            let mut q = p.clone();
            q.algorithm = 0;
            EvaluatedParams::evaluate(&q, &def.depth)
        };
        assert!((ev.clamp_angle[1] - 5f32.to_radians()).abs() < 1e-6);
        assert_eq!(ev.restore_power[1], 0.5);
    }

    /// 拓扑序与子表：父先于子；anchor=根；条目计数对。
    #[test]
    fn topology_order_and_children() {
        let (def, p) = minimal_chain();
        let anim = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let ev = EvaluatedParams::evaluate(&p, &def.depth);
        let topo = build_chain(&def, &ev, &anim).expect("topo");
        assert_eq!(topo.order, vec![0, 1]);
        assert_eq!(topo.anchor, 0);
        assert_eq!(topo.children[0], vec![1]);
        assert_eq!(topo.struct_entry_count, vec![1, 1]);
    }

    /// sd_101 chain0 原样参数跑 30 帧不炸：gravity 曲线全零 →
    /// 位置只受约束/惯性驱动；限角 3°→20°、回拉 0.10→0.02。
    #[test]
    fn sd101_like_params_smoke() {
        let (def, mut p) = minimal_chain();
        p.drag = BezierParam {
            start_value: 0.03,
            end_value: 0.02,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        p.use_drag = true;
        p.use_gravity = true;
        p.use_clamp_distance_ratio = true;
        p.use_clamp_rotation = true;
        p.use_restore_rotation = true;
        p.restore_rotation_alt = BezierParam {
            start_value: 0.10,
            end_value: 0.02,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        let anim0 = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let (topo, ev, mut st) = setup(&def, &p, &anim0);
        for i in 0..30 {
            let a = [
                [0.0, 0.0, 0.0],
                [
                    1.0 + 0.2 * (i as f32 * 0.3).sin(),
                    0.1 * (i as f32 * 0.7).cos(),
                    0.0,
                ],
            ];
            let s = frame(&mut st, &topo, &ev, &a, math::quat_identity(), DT);
            assert!(!s.nan_reset);
            assert!(!s.teleport_reset);
            assert!(s.max_disp.is_finite());
        }
    }
}
