//! npc 目标机：每名名册成员的目标状态机——步间停顿、决策梯、目标层
//! 抽签与目的地解算，全链消费 `moly_law::objective` 的律。
//!
//! 源的形状（方法体逐段核过）：
//! - 主循环每轮「步间停顿（TryRest）→ 决策梯（DecideObjective）→ 执行
//!   目标 → 丢弃」；停顿在决策**之前**无条件插入，时长 = 1000 ×
//!   trunc(角色表 pauseSeconds 列)，对话态保持续停；「立即可执行下一
//!   目标」旗一次性跳过整轮停顿。**AI 起动时装配（SetUp）先立这面旗**
//!   ——首判不落 15 秒停顿，出生即决策。
//! - 决策梯是一列谓词短路。产品可触发的档只有三档：槽位空（补数据立
//!   对话目标）、槽位无对话（立无对话目标）、全部落空（目标层百分比
//!   抽签）。其余档要产品没有的输入（摆拍、打断标记、换站/过场保持、
//!   问候、摆放后反应），按构造不可达，快照里按恒假带入。
//! - 目标层抽签：一张 [0,100) 抽签分三窗（家具道 / 已读窗 / 通用对话）；
//!   家具道内第二张分无对话行为与家具对话，家具对话再分第三张（常设 /
//!   已读重温）。
//! - 无对话目标的**收场补位**（源无对话目标执行完的 ForceUpdateObjective）
//!   是断环器：无对话槽位不补位，下一轮梯子又立无对话目标、永不换道。
//! - **家具目标动作点优先**：无对话道与家具对话道的目标解算先走动作点
//!   （座位表 → 挂点条目 → 挂点 x/z 上 0.3 可行走面门，[`fixture_target`]），
//!   落空回落环带（GetLittleFarPosition 同形）；落点交给执行层前过一遍
//!   身份判定（落点与挂点比 x/z），命中先导航到接近点，再执行局部
//!   FitTurning/FitWalking。
//!
//! 替身，具名：
//! * **内容绑定**：普通工厂保存所选master、既有preAction投影、目标位
//!   与self成员。其它尚缺完整工厂的道保留pending绑定，而非空Current。
//!   Previous只在AI Reset沿迁移；玩家窗口结束不更新它。
//! * **道可得性**：补位级联（未读 → 已读 → 通用，上游有货才落下一级）
//!   的「有货」替身 = 站点有锚定对话家具（配对语料锚在摆放表非 0 的
//!   fixtureId 上）。锚定表非空 ⇒ 未读道恒可得，级联首档即中。
//! * **目标家具抽签**：源在合格动作表上随机取（RandomPick）；产品在
//!   锚定摆放表上均匀抽——合格集的替身。
//! * **动作点座位表**：无对话道的 (家具, 座位) 行为表与对话道的先行动作
//!   入座行（经 talkId 居中联结 timeline 组）都烘成常量表（`NOTALK_*`
//!   / `TALK_SEATS`）——座位本该从 master 镜像族提取产物读，本单先烘入；
//!   提取侧落表后换装载是后续单的活。
//! * **无对话道座位查表**：行为表按 (角色, 家具) 行，恒常行占绝大多数；
//!   产品按家具查（角色覆写行保留）——角色维的行筛选是抽签域的近似，
//!   具名。
//! * **对话道先行入座选行**：同一 (角色, 家具) 的多行入座，产品取语料
//!   序首行（选行归抽签域，首行是具名替身）。
//! * **贴合身份判定（b__1）**：源比 AITalkData.TargetPosition 与挂点
//!   StartLoc 的 x/z；产品的落点面是全部已摆放实例的组合挂点世界位
//!   （`fixture_attach`，含未锚定摆放）——环带落点恰好撞上挂点坐标也
//!   命中判定，与源同形（执行层不问目标从哪条道来）。
//! * **入场目标**：源装配先立打断标记 14（入场站目标）再开主循环；产品
//!   不建模入场位（成员直接落座在可行走面上），只迁它的时序承重件——
//!   起动旗（首判立即执行）原样迁。
//! * **可行格与验路**：出生、目标与执行共用侵蚀后格场。高度几何只给
//!   已验证的导航位置落高；格场仍是原生导航网格的近似。
//! * **站定收场的无对话目标**：补位照常（断环），跳过旗不立——旗的源
//!   行为是「执行完毕后立即可执行下一目标」，站定者没有执行过。
//!
//! 目标面（[`ObjectiveFace`]）与玩家的约束面同源网格（站点域解析的
//! navmesh 面），但保留高度：目标域的采样要的是面上的三维最近点
//! （源 SamplePosition 语义），投影面裁不了高度。站点换装后面按代数
//! 重建；构建端带格桶索引，逐格最近点查询不扫全网格。

use crate::client_config::{
    ClientConfigs, KEY_CHARACTER_FIXTURE_MOVE_OFFSET, KEY_CHARACTER_OVERLAP_DISTANCE,
    KEY_CHARACTER_OVERLAP_TIME, KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
    KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ, KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
    KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT, KEY_NPC_RANDOM_MOVE_IN_ROOM_MAX_DISTANCE,
    KEY_NPC_RANDOM_MOVE_IN_ROOM_MIN_DISTANCE, KEY_NPC_RANDOM_MOVE_MAX_DISTANCE,
    KEY_NPC_RANDOM_MOVE_MIN_DISTANCE,
};
use crate::fixture::FixturePlacements;
use crate::npc::{
    depart, CharacterUnitId, FitCandidate, MotionPhase, MoveTarget, PathSlot, PauseSeconds,
    RouteStops, WalkState,
};
use crate::site::{GroundEpoch, SiteSelection, WalkFaceMeshes};
use bevy::prelude::*;
use moly_law::objective::{
    self, Cell, ObjectiveType, SurfaceProbe, TalkType, APPROACH_SEARCH_RANGE, TILE_SCALE,
};
use moly_law::talk::select::{self, LotteryPercents, PercentDraw, TalkLane, UniformDraw};
use std::collections::HashMap;

/// 目标面的格桶外扩余量（米）：三角按 XZ 包围盒外扩这一段后登记进格桶，
/// 采样查询只查落点所在格的桶。必须 ≥ 全部采样容差的上界（游走容差 =
/// 格距 0.25 是最大者）。
const BUCKET_MARGIN: f32 = 0.5;

/// 目标面：世界系三角网（保留高度）+ 格桶索引 + 可行走格集。站点域换装
/// 后由 [`build_face`] 按新面重建（代数戳记在 `epoch` 上）。
#[derive(Resource, Clone)]
pub struct ObjectiveFace {
    /// 世界系三角（三顶点，含高度）。
    tris: Vec<[[f32; 3]; 3]>,
    /// 格 → 覆盖该格（外扩余量后）的三角下标。
    buckets: HashMap<Cell, Vec<u32>>,
    /// 面包围盒折算的格界（含端点）。
    grid_min: Cell,
    grid_max: Cell,
    /// 格角映射的参考平面高度（面顶点均值）：格 → 世界的 y 档。
    ref_y: f32,
    /// 可行走格（格角在侵蚀场内且采到高度面），升序。
    walkable: Vec<Cell>,
    /// 构建时的站点代数（换站后过期）。
    epoch: u64,
    field: std::sync::Arc<moly_law::carve::WalkField>,
    generation: u64,
}

impl ObjectiveFace {
    /// 当前面是否对得上站点代数（换站后旧面只作过渡，不供决策）。
    pub(crate) fn is_fresh(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    pub(crate) fn navigation_generation(&self) -> u64 {
        self.generation
    }

    /// A source-navigation-validated central staging point, not the NPC patrol
    /// seed at the room perimeter. Ties retain the authored cell ordering.
    pub(crate) fn preview_center(&self) -> Option<[f32; 3]> {
        let x = (self.grid_min.0 as f32 + self.grid_max.0 as f32) * 0.5;
        let z = (self.grid_min.1 as f32 + self.grid_max.1 as f32) * 0.5;
        self.walkable
            .iter()
            .min_by(|a, b| {
                let dist = |cell: &Cell| (cell.0 as f32 - x).powi(2) + (cell.1 as f32 - z).powi(2);
                dist(a).total_cmp(&dist(b))
            })
            .and_then(|cell| self.sample(self.world_of(*cell), 0.25))
    }

    /// 格角世界位：x/z 按格距换算，y 取参考平面（采样负责落到真高度）。
    pub(crate) fn world_of(&self, cell: Cell) -> [f32; 3] {
        [
            cell.0 as f32 * TILE_SCALE,
            self.ref_y,
            cell.1 as f32 * TILE_SCALE,
        ]
    }

    /// 参考平面高度（面顶点均值）：一切「这一点在不在面上」的采样探针
    /// 用它做查询高度（同 [`Self::world_of`] 的基准）——高度未知的目标
    /// 点拿别的 y 去量，量出来的是那个 y 的影子，不是目标点本身。
    pub(crate) fn ref_y(&self) -> f32 {
        self.ref_y
    }

    /// Host reattachment using this existing WalkField and its surface mesh.
    /// Show keeps a valid current point; only an invalid x/z is moved to the
    /// nearest available point. This is not Unity NavMeshAgent reattachment.
    pub(crate) fn reattach_after_layout(&self, current: [f32; 3]) -> Option<[f32; 3]> {
        if current.iter().any(|value| !value.is_finite()) {
            return None;
        }
        if self.field.walkable_at([current[0], current[2]]) {
            return self.navigation_point_at([current[0], current[2]]);
        }
        let xz = self
            .field
            .nearest_walkable([current[0], current[2]], None)?;
        self.navigation_point_at(xz)
    }

    /// 面上最近点与距离（含高度的三角最近点；无三角覆盖处距离无穷）。
    fn closest(&self, target: [f32; 3]) -> ([f32; 3], f32) {
        let mut best = (target, f32::MAX);
        let Some(bucket) = self.buckets.get(&cell_of(target[0], target[2])) else {
            return best;
        };
        for &index in bucket {
            let tri = self.tris[index as usize];
            let point = closest_on_tri(target, tri[0], tri[1], tri[2]);
            let distance = dist3(point, target);
            if distance < best.1 {
                best = (point, distance);
            }
        }
        best
    }

    /// 表面采样（源 `SamplePosition` 语义）：面上最近点在容差内即命中，
    /// 返回面上点；否则未命中。
    pub(crate) fn sample(&self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
        let xz = self
            .field
            .nearest_walkable([target[0], target[2]], Some(tolerance))?;
        let (point, _) = self.closest([xz[0],
            target[1] - self.field.height_offset(xz), xz[1]]);
        let point = self.navigation_point(point);
        (dist3(point, target) <= tolerance && self.field.walkable_at([point[0], point[2]]))
            .then_some(point)
    }

    /// 高度几何探针；不提供导航资格，只给已经验证的路线/挂点落高度。
    pub(crate) fn surface_sample(&self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
        let (point, distance) = self.closest(target);
        (distance <= tolerance).then_some(point)
    }

    /// Height for an already navigation-qualified point. Explicit animation
    /// locators retain `surface_sample`, which probes the raw site geometry.
    pub(crate) fn navigation_surface_sample(&self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
        let point = self.navigation_point_at([target[0], target[2]])?;
        (dist3(point, target) <= tolerance).then_some(point)
    }

    fn navigation_point(&self, mut point: [f32; 3]) -> [f32; 3] {
        point[1] += self.field.height_offset([point[0], point[2]]);
        point
    }

    /// Navigation already owns x/z. A 3-D nearest-point projection would move
    /// them on slopes, then repeatedly convert a step's height into terrain.
    /// Use the same highest covering site surface as the single-layer bake.
    pub(crate) fn navigation_point_at(&self, xz: [f32; 2]) -> Option<[f32; 3]> {
        if !xz.into_iter().all(f32::is_finite) {
            return None;
        }
        let bucket = self.buckets.get(&cell_of(xz[0], xz[1]))?;
        let mut height: Option<f32> = None;
        for &index in bucket {
            let [a, b, c] = self.tris[index as usize];
            let determinant = (b[2] - c[2]) * (a[0] - c[0])
                + (c[0] - b[0]) * (a[2] - c[2]);
            if determinant.abs() < 1e-12 {
                continue;
            }
            let u = ((b[2] - c[2]) * (xz[0] - c[0])
                + (c[0] - b[0]) * (xz[1] - c[2])) / determinant;
            let v = ((c[2] - a[2]) * (xz[0] - c[0])
                + (a[0] - c[0]) * (xz[1] - c[2])) / determinant;
            if u >= -1e-5 && v >= -1e-5 && u + v <= 1.00001 {
                let y = u * a[1] + v * b[1] + (1.0 - u - v) * c[1];
                height = Some(height.map_or(y, |prior| prior.max(y)));
            }
        }
        height.map(|y| [xz[0], y + self.field.height_offset(xz), xz[1]])
    }

    /// 与执行器共用严格完整路径，目标不拉回到其他可达位置。
    pub(crate) fn has_path(&self, source: [f32; 3], target: [f32; 3]) -> bool {
        self.field
            .path_exact([source[0], source[2]], [target[0], target[2]])
            .is_some()
    }

    /// The player furniture adapter uses this same carved geometry as normal
    /// movement. A failed exact path remains a failed query, never a straight
    /// line fabricated through furniture. Heights come from the real mesh.
    pub(crate) fn fixture_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        if !from.is_finite() || !to.is_finite() {
            return None;
        }
        let corners = self.field.path_exact([from.x, from.z], [to.x, to.z])?;
        corners
            .into_iter()
            .map(|point| self.navigation_point_at(point).map(Vec3::from))
            .collect()
    }

    pub(crate) fn fixture_move(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        if !from.is_finite() || !to.is_finite() {
            return None;
        }
        let point = self.field.constrain_move([from.x, from.z], [to.x, to.z]);
        self.navigation_point_at(point).map(Vec3::from)
    }

    /// 可行走格集。
    pub(crate) fn walkable(&self) -> &[Cell] {
        &self.walkable
    }

    /// 格界（含端点）。
    pub(crate) fn grid_bounds(&self) -> (Cell, Cell) {
        (self.grid_min, self.grid_max)
    }
}

/// 目标面的探测合同实现：把 [`ObjectiveFace`] 借给律的三条目的地链。
struct FaceProbe<'a> {
    face: &'a ObjectiveFace,
}

impl SurfaceProbe for FaceProbe<'_> {
    fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
        self.face.sample(target, tolerance)
    }

    fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool {
        self.face.has_path(source, target)
    }
}

/// 世界位 → 格坐标（格距换算，向下取整）。
pub(crate) fn cell_of(x: f32, z: f32) -> Cell {
    (
        (x / TILE_SCALE).floor() as i32,
        (z / TILE_SCALE).floor() as i32,
    )
}

/// Update：面网格柄齐备且站点场景展开后，把目标面提为世界系三角网并算
/// 格桶与可行走格。代数翻新（换站）时旧面过期重建——旧面在柄拆换的间隙
/// 里供过渡（成员重播种与决策都按 [`ObjectiveFace::is_fresh`] 让路，不会
/// 拿旧面走新路）。柄对不上实体或资产未到齐都整帧等：面宁可晚到不可
/// 残缺。
#[allow(clippy::type_complexity)]
pub(crate) fn build_face(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    handles: Option<Res<WalkFaceMeshes>>,
    face: Option<Res<ObjectiveFace>>,
    epoch: Option<Res<GroundEpoch>>,
    walk_face: Option<Res<crate::walk_face::WalkFace>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
) {
    let epoch = epoch.map(|epoch| epoch.0).unwrap_or(0);
    let Some(walk_face) = walk_face else {
        return;
    };
    if face.is_some_and(|face| face.epoch == epoch && face.generation == walk_face.generation()) {
        return;
    }
    let Some(handles) = handles else {
        return;
    };
    let mut by_handle = handles
        .0
        .iter()
        .map(|handle| (handle.id(), false))
        .collect::<Vec<_>>();
    let mut tris = Vec::new();
    for (mesh3d, global) in &parts {
        let Some(seen) = by_handle
            .iter_mut()
            .find(|(id, _)| *id == mesh3d.0.id())
            .map(|(_, seen)| seen)
        else {
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            return; // 柄在场而资产未到，等齐再说
        };
        *seen = true;
        let Some(verts) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            panic!("目标面网格没有 float3 的 POSITION 属性");
        };
        let index: Vec<usize> = mesh
            .indices()
            .map(|indices| indices.iter().collect())
            .unwrap_or_default();
        let triples: Vec<[usize; 3]> = match index.len() {
            0 => (0..verts.len() / 3)
                .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
                .collect(),
            _ => (0..index.len() / 3)
                .map(|i| [index[i * 3], index[i * 3 + 1], index[i * 3 + 2]])
                .collect(),
        };
        for [a, b, c] in triples {
            let world = |i: usize| {
                let v = global.transform_point(Vec3::from(verts[i]));
                [v.x, v.y, v.z]
            };
            tris.push([world(a), world(b), world(c)]);
        }
    }
    if by_handle.iter().any(|(_, seen)| !*seen) || tris.is_empty() {
        return; // 场景未展开完，下一帧再试
    }

    // 包围盒与格界（含端点）。
    let mut min = [f32::MAX; 3];
    let mut max = [-f32::MAX; 3];
    let mut sum_y = 0.0_f32;
    for tri in &tris {
        for point in tri {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
            sum_y += point[1];
        }
    }
    let ref_y = sum_y / (tris.len() as f32 * 3.0);
    let grid_min = cell_of(min[0], min[2]);
    let grid_max = cell_of(max[0], max[2]);

    // 格桶：三角按 XZ 包围盒外扩余量登记。
    let mut buckets: HashMap<Cell, Vec<u32>> = HashMap::new();
    for (index, tri) in tris.iter().enumerate() {
        let lo = cell_of(
            tri.iter().map(|p| p[0]).fold(f32::MAX, f32::min) - BUCKET_MARGIN,
            tri.iter().map(|p| p[2]).fold(f32::MAX, f32::min) - BUCKET_MARGIN,
        );
        let hi = cell_of(
            tri.iter().map(|p| p[0]).fold(f32::MIN, f32::max) + BUCKET_MARGIN,
            tri.iter().map(|p| p[2]).fold(f32::MIN, f32::max) + BUCKET_MARGIN,
        );
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                buckets.entry((x, z)).or_default().push(index as u32);
            }
        }
    }

    // 可行走格：格角在游走容差内采到面，且不在任何摆放盒内（烘焙刻洞
    // 的替身，见模块注释）。
    let face = ObjectiveFace {
        tris: tris.clone(),
        buckets,
        grid_min,
        grid_max,
        ref_y,
        walkable: Vec::new(),
        epoch,
        field: walk_face.field.clone(),
        generation: walk_face.generation(),
    };
    let mut walkable = Vec::new();
    for x in grid_min.0..=grid_max.0 {
        for z in grid_min.1..=grid_max.1 {
            let cell = (x, z);
            if !walk_face.walkable_at([x as f32 * TILE_SCALE, z as f32 * TILE_SCALE]) {
                continue;
            }
            if face.sample(face.world_of(cell), TILE_SCALE).is_some() {
                walkable.push(cell);
            }
        }
    }
    let walkable_count = walkable.len();
    let face = ObjectiveFace { walkable, ..face };
    info!(
        "[npc-objective] 目标面就绪：三角 {}，格界 x [{},{}] z [{},{}]，可行走 {} 格（导航代数 {}），参考高度 {:.3}，站点代数 {}",
        tris.len(),
        grid_min.0,
        grid_max.0,
        grid_min.1,
        grid_max.1,
        walkable_count,
        walk_face.generation(),
        ref_y,
        epoch
    );
    commands.insert_resource(face);
}

/// A prepared master reference keeps its parser/backend identity. A missing
/// factory result is not an empty source AITalkData and must not take that route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TalkBackend {
    General,
    Fixture,
}

#[derive(Debug, Clone)]
pub(crate) struct TalkContent {
    pub(crate) master_id: i32,
    pub(crate) backend: TalkBackend,
    pub(crate) is_general: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct AiTalkData {
    pub(crate) kind: TalkType,
    pub(crate) content: Option<TalkContent>,
    pub(crate) target_fixture: Option<Entity>,
    pub(crate) target_position: [f32; 3],
    pub(crate) main_character: u32,
    pub(crate) characters: Vec<u32>,
    /// The ordinary data product's pre-action projection, not a new Lua IR.
    pub(crate) pre_action: Option<moly_law::talk::TweetRef>,
    pub(crate) pending_factory: Option<&'static str>,
}

impl AiTalkData {
    fn pending(kind: TalkType, unit: u32, position: [f32; 3]) -> Self {
        Self {
            kind,
            content: None,
            target_fixture: None,
            target_position: position,
            main_character: unit,
            characters: vec![unit],
            pre_action: None,
            pending_factory: Some("activity factory has not supplied its master and participants"),
        }
    }
}

/// Current/previous content belongs to the AI instance, not to either player
/// script backend. Set and reset are different source operations.
#[derive(Component, Default)]
pub struct TalkSlot {
    pub(crate) current: Option<AiTalkData>,
    pub(crate) previous: Option<AiTalkData>,
}

impl TalkSlot {
    pub(crate) fn kind(&self) -> Option<TalkType> {
        self.current.as_ref().map(|data| data.kind)
    }

    pub(crate) fn previous_id(&self) -> Option<i32> {
        self.previous
            .as_ref()?
            .content
            .as_ref()
            .map(|content| content.master_id)
    }

    pub(crate) fn set_current(&mut self, data: AiTalkData) {
        self.current = Some(data);
    }

    /// Call only at an AI reset/release edge, never when a player window ends.
    pub(crate) fn reset_ai_talk_data(&mut self) {
        self.previous = self.current.take();
    }
}

/// 成员抽签引擎：跨帧线性同余（与配对域同款——律只约束分布与求值次数，
/// 引擎序列不在律内）。种子按 unit id 定推，逐成员可复算、不受查询序
/// 影响。
#[derive(Component, Clone)]
pub struct MemberRng(u64);

impl MemberRng {
    /// 种子：unit id 经两轮固定扰动（不与任何面板值混源）。
    pub(crate) fn seeded(unit: u32) -> Self {
        Self(0x9E37_79B9_7F4A_7C15_u64 ^ (unit as u64).wrapping_mul(0x2545_F491_4F6C_DD1D))
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

/// 目标机（成员的思维相位）：停顿计时、当前目标与一次性起动旗。
#[derive(Component)]
pub struct ObjectiveMind {
    /// `true` = 执行目标中（等到达站收场）；`false` = 步间停顿中。
    pub executing: bool,
    /// 当前目标类型；`None` = 尚未有过目标（出生态）。
    pub current: Option<ObjectiveType>,
    /// 「立即可执行下一目标」旗（一次性）：跳过下一轮停顿。出生时由
    /// 起动装配立起（源 SetUp 同形），此后只由无对话目标的收场补位立。
    pub skip_next_rest: bool,
    /// 停顿剩余秒数（0 = 停完待判）。
    pub rest_remaining: f32,
    /// Identifies each newly armed objective Rest interval, not each frame.
    pub(crate) rest_revision: u64,
    overlap_seconds: f32,
    /// A cancelled ordinary target completes its coroutine, then the AI loop
    /// enters its next TryRest after Show. Cancelling a running Rest instead
    /// continues past that Rest. Neither operation resets talk data.
    edit_rest_after: Option<ObjectiveType>,
}

impl moly_law::path::WaypointDraw for MemberRng {
    fn index(&mut self, upper_exclusive: usize) -> usize {
        ((self.next() >> 33) as usize) % upper_exclusive
    }
}

impl ObjectiveMind {
    /// 出生态：起动旗立起（首判立即执行），停顿零，无当前目标。
    pub(crate) fn at_spawn() -> Self {
        Self {
            executing: false,
            current: None,
            skip_next_rest: true,
            rest_remaining: 0.0,
            rest_revision: 0,
            overlap_seconds: 0.0,
            edit_rest_after: None,
        }
    }
}

/// Presenter cancellation condition is action != Talk(4). The installed
/// RandomWalk/Rest objectives have CanCancel=true and ChangeStatus(Idle) on
/// cancel; Talk objective's pre-talk movement is cancellable too. Stop only
/// those ordinary continuations. Do not recreate the AI instance, reseed its
/// RNG, clear Previous, or invoke the unrelated ResetAITalkData operation.
pub(crate) fn cancel_ordinary_for_layout_edit(world: &mut World, actor: Entity) -> bool {
    let mut query = world.query::<(&crate::npc::NpcActions, &mut ObjectiveMind)>();
    let Ok((actions, mut mind)) = query.get_mut(world, actor) else {
        return false;
    };
    if actions.current == crate::npc::NpcAction::Talk {
        return false;
    }
    let resting = !mind.executing
        && (mind.rest_remaining > 0.0 || actions.current == crate::npc::NpcAction::Rest);
    let ordinary = mind.executing
        && matches!(
            mind.current,
            Some(ObjectiveType::RandomMove | ObjectiveType::Talk)
        );
    if !resting && !ordinary {
        return false;
    }
    mind.edit_rest_after = if ordinary { mind.current } else { None };
    mind.executing = false;
    mind.rest_remaining = 0.0;
    mind.overlap_seconds = 0.0;
    true
}

/// 百分比抽签的引擎适配（带账目：逐次落点记录成日志行）。
struct PercentSource<'a> {
    rng: &'a mut MemberRng,
    draws: Vec<u32>,
}

impl<'a> PercentSource<'a> {
    fn new(rng: &'a mut MemberRng) -> Self {
        Self {
            rng,
            draws: Vec::new(),
        }
    }

    /// 账目串：按抽签次序列出（`r0=37 r1=82`）。
    fn account(&self) -> String {
        self.draws
            .iter()
            .enumerate()
            .map(|(index, value)| format!("r{index}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl PercentDraw for PercentSource<'_> {
    fn draw_percent(&mut self) -> u32 {
        let value = ((self.rng.next() >> 33) % 100) as u32;
        self.draws.push(value);
        value
    }
}

/// 均匀抽签的引擎适配（带账目：落点与池大小）。
struct UniformSource<'a> {
    rng: &'a mut MemberRng,
    draws: Vec<(usize, usize)>,
}

impl<'a> UniformSource<'a> {
    fn new(rng: &'a mut MemberRng) -> Self {
        Self {
            rng,
            draws: Vec::new(),
        }
    }

    fn account(&self) -> String {
        self.draws
            .iter()
            .map(|(index, len)| format!("{index}/{len}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl UniformDraw for UniformSource<'_> {
    fn draw(&mut self, len: usize) -> usize {
        let index = ((self.rng.next() >> 33) as usize) % len.max(1);
        self.draws.push((index, len));
        index
    }
}

/// 均匀置换的引擎适配（源 `OrderBy(Guid.NewGuid())`：每格一枚新键、
/// 全部被排序消费）。
struct PermuteSource<'a> {
    rng: &'a mut MemberRng,
    keys: usize,
}

impl<'a> PermuteSource<'a> {
    fn new(rng: &'a mut MemberRng) -> Self {
        Self { rng, keys: 0 }
    }
}

impl objective::Permute for PermuteSource<'_> {
    fn permutation(&mut self, len: usize) -> Vec<usize> {
        self.keys += len;
        let mut keyed: Vec<(u64, usize)> = (0..len).map(|index| (self.rng.next(), index)).collect();
        keyed.sort_by_key(|&(key, _)| key);
        keyed.into_iter().map(|(_, index)| index).collect()
    }
}

/// 锚定目标家具：fixtureId、摆放包与摆放盒（包围盒格界）的联结——
/// 靠近家具链的环带按盒算，抽签按 fixtureId 记账，动作点支按包取
/// 挂点条目。
struct AnchoredFixture {
    uid: String,
    fixture_id: i32,
    package: String,
    min: Cell,
    max: Cell,
}

/// Rebuild only when the placement owner changes its rows. This cache does
/// not include moving NPC positions, eligibility, RNG results, or paths.
#[derive(Default)]
pub(crate) struct AnchoredFixtureCache {
    rows: Option<Vec<AnchoredFixture>>,
}

/// Preserve each placed instance's footprint, including repeated furniture IDs.
fn anchored_fixtures(placements: &FixturePlacements) -> Vec<AnchoredFixture> {
    placements
        .occupancy_rows()
        .into_iter()
        .zip(placements.fixture_ids())
        .filter(|(_, id)| *id != 0)
        .map(|(row, fixture_id)| AnchoredFixture {
            uid: row.uid,
            fixture_id,
            package: row.package,
            min: (row.min.x as i32, row.min.z as i32),
            max: (row.max.x as i32, row.max.z as i32),
        })
        .collect()
}

/// 道别 → 槽位类型（替身映射，见模块注释）。
fn slot_type_of(lane: TalkLane) -> TalkType {
    match lane {
        TalkLane::GeneralTalk => TalkType::Common,
        TalkLane::YetUnreadFixtureTalk
        | TalkLane::FixtureCommonTalk
        | TalkLane::AlreadyReadTalkFixtureTalk => TalkType::CommonFixture,
    }
}

/// 无对话道的座位表（行为表「(角色, 家具) 行 → 座位 id」的烘入切片）：
/// 座位对家具恒常（64 件带行的家具里 63 件如此），例外按角色覆写
/// （[`NOTALK_OVERRIDES`]）。座位 id 是挂点条目名里的数字——决策侧
/// 拿它取挂点条目。抽签面替身：真源在合格动作表上随机取，产品在
/// 锚定摆放表上均匀抽（见模块注释「目标家具抽签」），表按家具查即
/// 与抽签面同形。
const NOTALK_ASSIGNMENTS: &[(i32, i32)] = &[
    (8, 13),
    (10, 13),
    (23, 13),
    (25, 13),
    (38, 13),
    (40, 13),
    (53, 13),
    (55, 13),
    (68, 13),
    (70, 13),
    (83, 13),
    (85, 13),
    (143, 13),
    (147, 13),
    (148, 13),
    (150, 13),
    (151, 13),
    (163, 13),
    (164, 13),
    (175, 13),
    (177, 13),
    (178, 13),
    (184, 13),
    (189, 13),
    (202, 13),
    (219, 13),
    (227, 13),
    (236, 13),
    (243, 11),
    (255, 13),
    (259, 13),
    (269, 13),
    (272, 13),
    (286, 13),
    (294, 13),
    (303, 13),
    (307, 13),
    (309, 13),
    (311, 13),
    (321, 13),
    (323, 13),
    (327, 13),
    (337, 13),
    (339, 13),
    (460, 13),
    (462, 13),
    (470, 13),
    (524, 13),
    (531, 13),
    (532, 13),
    (586, 13),
    (587, 13),
    (588, 13),
    (660, 13),
    (661, 13),
    (747, 13),
    (749, 23),
    (760, 13),
    (805, 14),
    (814, 13),
    (984, 13),
    (986, 13),
    (988, 13),
    (1025, 13),
];

/// 无对话道的按角色覆写行（家具 227 的例外：角色 18 的座位不是该家具
/// 的恒常值）。
const NOTALK_OVERRIDES: &[(u32, i32, i32)] = &[(18, 227, 43)];

/// 对话道的座位表（先行动作表的烘入切片）：(家具, 角色) → 座位。
/// 对话道的入座不走对话主表——对话先查先行动作行，行带 timeline 组
/// 才进时间轴拿座位槽（配对道没有这条链，照环带走）。产品已摆放的
/// 家具里带座位行的三家：227（15 员）、243（1 员）、695（30 员）；
/// 其余家具的对话道环带（对话无先行入座行）。选取替身：同 (角色,
/// 家具) 多行取语料序首行——真源在合格动作表上随机取，随机面归
/// 抽签域，这里按首行具名。
const TALK_SEATS: &[(i32, u32, i32)] = &[
    (227, 2, 13),
    (227, 5, 13),
    (227, 7, 13),
    (227, 9, 13),
    (227, 10, 13),
    (227, 12, 13),
    (227, 13, 13),
    (227, 14, 13),
    (227, 16, 43),
    (227, 18, 23),
    (227, 20, 13),
    (227, 30, 13),
    (227, 31, 43),
    (227, 39, 13),
    (227, 55, 13),
    (243, 12, 13),
    (695, 1, 33),
    (695, 2, 43),
    (695, 3, 33),
    (695, 4, 43),
    (695, 5, 33),
    (695, 6, 43),
    (695, 7, 33),
    (695, 8, 33),
    (695, 9, 33),
    (695, 10, 43),
    (695, 11, 53),
    (695, 12, 53),
    (695, 13, 53),
    (695, 14, 33),
    (695, 15, 33),
    (695, 16, 43),
    (695, 17, 33),
    (695, 18, 43),
    (695, 19, 33),
    (695, 20, 33),
    (695, 27, 33),
    (695, 28, 43),
    (695, 29, 33),
    (695, 30, 33),
    (695, 31, 43),
    (695, 33, 33),
    (695, 39, 53),
    (695, 42, 33),
    (695, 49, 53),
    (695, 55, 33),
];

/// 无对话道座位：覆写行优先，其次家具恒常行；无行 [`None`]。
fn notalk_seat(fixture_id: i32, unit: u32) -> Option<i32> {
    if let Some(&(_, _, seat)) = NOTALK_OVERRIDES
        .iter()
        .find(|&&(u, f, _)| u == unit && f == fixture_id)
    {
        return Some(seat);
    }
    NOTALK_ASSIGNMENTS
        .iter()
        .find(|&&(f, _)| f == fixture_id)
        .map(|&(_, seat)| seat)
}

/// 对话道座位（先行动作表切片）：(家具, 角色) 直查。
fn talk_seat(fixture_id: i32, unit: u32) -> Option<i32> {
    TALK_SEATS
        .iter()
        .find(|&&(f, u, _)| f == fixture_id && u == unit)
        .map(|&(_, _, seat)| seat)
}

/// 动作点支的面采样门容差（米）：挂点 x/z 上的可行走面采样半径
/// （源动作点门里采样调用的同值）。
pub(crate) const SAMPLE_GATE_TOLERANCE: f32 = 0.3;

/// 家具目标解算结果：动作点支命中 / 环带兜底命中（带落空理由词）/
/// 全未命中（理由词串）。
enum FixtureOutcome {
    /// 动作点支命中：落点 = 挂点 x/z + 面 0.3 门采到的 y。
    ActionPoint {
        fixture_id: i32,
        seat: i32,
        package: String,
        landing: [f32; 3],
    },
    /// 环带兜底命中：`reason` 是动作点支落空的理由（无行 / 挂点条目
    /// 缺 / 面门未中）。
    Ring {
        fixture_id: i32,
        ring: usize,
        landing: [f32; 3],
        reason: &'static str,
    },
    /// 动作点支与环带都未命中。
    Missed { reason: String },
}

/// 家具目标解算（动作点优先、环带兜底）：座位查表 → 挂点条目取件 →
/// 挂点 x/z 上 0.3 可行走面门 → 命中即动作点落点；任一环落空回落环带
/// （靠近家具链同形），环带也未命中按未命中站定。落空各档的理由词
/// 具名——「走了环带」在目标裁决行里必须读得出为什么。
fn fixture_target(
    anchored: &[AnchoredFixture],
    face: &ObjectiveFace,
    attach: &crate::fixture_attach::AttachWorlds,
    from: [f32; 3],
    world_of: impl Fn(Cell) -> [f32; 3] + Copy,
    probe: &mut FaceProbe<'_>,
    uniform: &mut UniformSource<'_>,
    move_offset: f32,
    seat_of: impl Fn(i32) -> Option<i32> + Copy,
    seat_miss_word: &'static str,
) -> FixtureOutcome {
    if anchored.is_empty() {
        return FixtureOutcome::Missed {
            reason: "无锚定家具".to_owned(),
        };
    }
    let fixture = &anchored[uniform.draw(anchored.len())];
    // —— 动作点支：座位查表 → 挂点条目 → 面 0.3 门；命中即返回，任一
    // 环落空回落环带（理由词具名，随环带行报出）。
    let fallback_reason: &'static str = match seat_of(fixture.fixture_id) {
        Some(seat) => match attach.entry(&fixture.uid, seat) {
            Some(world) => {
                // 面采样门：查询点 = 挂点 x/z + 参考高度，容差 0.3；命中
                // 取采样 y（挂点 local 的 y 恒零，落点高度走面——同全部
                // 落点的形状）。
                let query = [world.position[0], face.ref_y, world.position[2]];
                match face.sample(query, SAMPLE_GATE_TOLERANCE) {
                    Some(landed) if face.has_path(from, landed) => {
                        return FixtureOutcome::ActionPoint {
                            fixture_id: fixture.fixture_id,
                            seat,
                            package: fixture.package.clone(),
                            landing: [world.position[0], landed[1], world.position[2]],
                        };
                    }
                    Some(_) => "动作点接近位置无完整路径",
                    None => "面 0.3 门未中",
                }
            }
            None => "挂点条目缺",
        },
        None => seat_miss_word,
    };
    // —— 环带兜底 ——
    let (grid_min, grid_max) = face.grid_bounds();
    let cells = objective::approach_ring_cells(
        fixture.min,
        fixture.max,
        APPROACH_SEARCH_RANGE,
        grid_min,
        grid_max,
    );
    match objective::approach_target(&cells, from, world_of, probe, move_offset) {
        Some(landing) => FixtureOutcome::Ring {
            fixture_id: fixture.fixture_id,
            ring: cells.len(),
            landing,
            reason: fallback_reason,
        },
        None => FixtureOutcome::Missed {
            reason: format!("{fallback_reason}，环带未命中"),
        },
    }
}

/// 动作点支命中的冒烟行（独立一行，方便对账）：家具、包尾名、座位与
/// 落点 x/z——「目标动作点」从这行直接读出。
fn action_point_line(unit: u32, fixture_id: i32, seat: i32, package: &str, landing: [f32; 3]) {
    let tail = package.rsplit("__").next().unwrap_or(package);
    info!(
        "[npc unit={unit}] 目标动作点 = 家具 {fixture_id}（{tail}）的座 {seat} @ ({:.2},{:.2})",
        landing[0], landing[2]
    );
}

/// 道别的账目词。
fn lane_word(lane: TalkLane) -> &'static str {
    match lane {
        TalkLane::YetUnreadFixtureTalk => "talk:unread",
        TalkLane::GeneralTalk => "talk:general",
        TalkLane::FixtureCommonTalk => "talk:common",
        TalkLane::AlreadyReadTalkFixtureTalk => "talk:read",
    }
}

/// 成员快照（社交链的候选输入与游走链的占格都按它算）。
struct MemberSnap {
    entity: Entity,
    target_fixture: Option<Entity>,
    unit: u32,
    position: [f32; 3],
    /// 当前导航目的地：执行中 = 目标目的地，停顿中 = 原地（无路径）。
    destination: [f32; 3],
}

/// Update：目标机推进——停顿计时（对话态冻结）、路线尽收场（无对话目标
/// 的断环补位）、停顿尽后的决策（梯子 → 抽签 → 目的地解算 → 出发）。
/// 出发经 [`depart`] 一次算完整条路线（可行走面折线 → 路点表），无路
/// （折线全档未命中）按未命中收场。
///
/// 一次决策的抽签账目一行日志：抽签落点、门占比、目标家具与环、落点
/// 坐标——「每员目标选取可从日志复算」。
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn decide(
    time: Res<Time>,
    editor: Res<crate::fixture_edit::EditSessionActive>,
    face: Option<Res<ObjectiveFace>>,
    epoch: Option<Res<GroundEpoch>>,
    selection: Option<Res<SiteSelection>>,
    placements: Res<FixturePlacements>,
    mut anchored_cache: Local<AnchoredFixtureCache>,
    configs: Option<Res<ClientConfigs>>,
    walk_face: Option<Res<crate::walk_face::WalkFace>>,
    attach_worlds: Option<Res<crate::fixture_attach::AttachWorlds>>,
    players: Query<&Transform, With<crate::player::PlayerControlled>>,
    initialized: Query<
        (&CharacterUnitId, &InheritedVisibility),
        With<crate::character::MotionDriver>,
    >,
    catalog: crate::player_talk::TalkCatalog,
    mut fixture_activities: crate::npc_fixture_activity::Factory,
    mut npcs: Query<(
        Entity,
        &CharacterUnitId,
        &PauseSeconds,
        Option<&crate::talk::TalkHold>,
        &mut TalkSlot,
        &mut ObjectiveMind,
        &mut MemberRng,
        &mut PathSlot,
        &mut WalkState,
        &mut MoveTarget,
        &mut RouteStops,
        &mut MotionPhase,
        &mut crate::npc::NpcActions,
        &mut crate::npc::RestLifecycle,
    )>,
) {
    // Invalidate before readiness returns so a placement change cannot be
    // missed while another input (catalog/geometry) is temporarily absent.
    if placements.is_changed() {
        anchored_cache.rows = None;
    }
    // Keep change-tick invalidation above, but consume no ordinary AI timer
    // or RNG while Hide owns the actors or saved geometry is still reloading.
    if editor.is_active() {
        return;
    }
    let Some(face) = face else {
        return;
    };
    let Some(config) = configs.as_deref() else {
        return;
    };
    // 代数资源在场才算「面已定案」：首站定案（inactiveNodes 清扫）之前
    // 的面可能还含即将被撤的实例，不在它上面做决策。
    let Some(epoch) = epoch else {
        return;
    };
    let face: &ObjectiveFace = &*face;
    if !face.is_fresh(epoch.0) {
        return;
    }
    // 可行走面（执行侧的折线来源）：不在场时整帧让路——决策写了落点也
    // 算不出路线，等面与目标面同帧链齐（站点链先建面再建目标面）。
    let Some(walk_face) = walk_face else {
        return;
    };
    let walk_face: &crate::walk_face::WalkFace = &*walk_face;
    if face.generation != walk_face.generation() {
        return;
    }
    // 动作点世界位组合表：不在场时整帧让路（组合晚解析一帧落地）——
    // 缺表时动作点支读不出挂点，静默落环带会把「表没到」伪装成「该
    // 家具没有动作点」。
    let attach = attach_worlds.as_deref();
    if !catalog.ready() {
        return;
    }
    let dt = time.delta_secs();

    // 成员快照：社交链的候选（位置 + 导航目的地）与游走链的占格都从
    // 同一帧的全员位置算。
    let snaps: Vec<MemberSnap> = npcs
        .iter()
        .map(
            |(entity, unit, _, _, slot, mind, _, _, state, target, _, _, _, _)| MemberSnap {
                entity,
                target_fixture: slot.current.as_ref().and_then(|data| data.target_fixture),
                unit: unit.0,
                position: state.0.position,
                destination: if mind.executing {
                    target.0
                } else {
                    state.0.position
                },
            },
        )
        .collect();
    let fixture_targets: Vec<_> = snaps
        .iter()
        .map(|snap| (snap.entity, snap.target_fixture))
        .collect();
    let mut occupied: std::collections::HashSet<Cell> = snaps
        .iter()
        .map(|snap| cell_of(snap.position[0], snap.position[2]))
        .collect();
    for player in &players {
        let translation = player.translation;
        occupied.insert(cell_of(translation.x, translation.z));
    }

    // 面板占比与游走距离档（房间类站点切室内档）。
    let percents = LotteryPercents {
        fixture_talk: config.float(KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT),
        already_read_fixture_talk: config.float(KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT),
        none_talk_fixture_action: config.float(KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT),
        already_read_when_has_not_read: config
            .float(KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ),
    };
    let is_room = selection.as_deref().is_some_and(|site| site.is_room());
    let (wander_min, wander_max) = if is_room {
        (
            config.int(KEY_NPC_RANDOM_MOVE_IN_ROOM_MIN_DISTANCE),
            config.int(KEY_NPC_RANDOM_MOVE_IN_ROOM_MAX_DISTANCE),
        )
    } else {
        (
            config.int(KEY_NPC_RANDOM_MOVE_MIN_DISTANCE),
            config.int(KEY_NPC_RANDOM_MOVE_MAX_DISTANCE),
        )
    };
    let move_offset = config.float(KEY_CHARACTER_FIXTURE_MOVE_OFFSET);
    let anchored = anchored_cache
        .rows
        .get_or_insert_with(|| anchored_fixtures(&placements))
        .as_slice();
    if !anchored.is_empty() && attach.is_none() {
        return;
    }
    // 道可得性替身：锚定对话家具在 ⇒ 未读道可得（级联首档即中）。
    let yet_unread_available = !anchored.is_empty();

    for (
        entity,
        unit,
        pause_seconds,
        talk_hold,
        mut slot,
        mut mind,
        mut rng,
        mut path,
        mut state,
        mut target,
        mut route,
        mut phase,
        mut actions,
        mut rest,
    ) in &mut npcs
    {
        if !actions.ready() {
            continue;
        }
        // The activity owner consumes arrival and completion separately. A
        // route reaching its last corner must not skip the furniture body.
        if fixture_activities.owns_actor(entity) {
            continue;
        }
        if talk_hold.is_some() || actions.current == crate::npc::NpcAction::Talk {
            // RestObjective's delay runs before its WaitWhile(Talk). Ending
            // the Rest action disposes its script, not this independent delay.
            if !mind.executing && mind.rest_remaining > 0.0 {
                mind.rest_remaining = (mind.rest_remaining - dt).max(0.0);
            }
            mind.overlap_seconds = 0.0;
            continue;
        }
        if let Some(cancelled) = mind.edit_rest_after.take() {
            // Respect a later owner that installed another objective during
            // the hidden interval. The Talk/hold guards above also still apply.
            if !mind.executing && mind.current == Some(cancelled) {
                finish_objective(
                    unit.0,
                    &mut mind,
                    &mut slot,
                    pause_seconds.0,
                    false,
                    &mut actions,
                    &mut rest,
                );
                if mind.rest_remaining > 0.0 {
                    continue;
                }
            }
        }
        let overlaps = snaps.iter().any(|other| {
            other.unit != unit.0
                && dist3(state.0.position, other.position)
                    < config.float(KEY_CHARACTER_OVERLAP_DISTANCE)
        });
        // 当前移动执行器运行于 AutoMove；转体相位在源是 Rotate。
        // 本域未承载 FixtureAction/PhotoShot/通信状态，不能把它们猜成常态。
        let state_type = actions.current as u8;
        let group_talk = objective::overlap::has_group_talk(mind.current, slot.kind());
        let cancellable = mind.executing || mind.rest_remaining > 0.0;
        let visible = initialized
            .iter()
            .any(|(id, visibility)| id.0 == unit.0 && visibility.get());
        if visible
            && objective::overlap::should_cancel(
                &mut mind.overlap_seconds,
                dt,
                config.float(KEY_CHARACTER_OVERLAP_TIME),
                overlaps,
                group_talk,
                state_type,
                cancellable,
            )
        {
            mind.executing = false;
            mind.current = None;
            mind.rest_remaining = 0.0;
            mind.skip_next_rest = true;
            slot.reset_ai_talk_data();
            route.cancel();
            path.0 = moly_law::path::NpcPathWalkSlot::from_corners(Vec::new());
            state.0.next_corner = 0;
            *phase = MotionPhase::Dwelling { remaining: None };
            actions.change(crate::npc::NpcAction::Idle, &mut rest);
            info!(
                "[npc unit={}] 角色重叠超过源时限，取消目标并立即重选",
                unit.0
            );
        }
        // 执行中且路线已尽（推进系统末站驻留尽的收场位）：目标收场。路点
        // 中途的驻留（Some）不收场——目标还没走完，不换乘。
        let outcome = if mind.executing {
            route.outcome.take()
        } else {
            None
        };
        if outcome == Some(crate::npc::RouteOutcome::Stopped) {
            // The movement task ended without OnArrive. Do not invent arrival,
            // release the AI content, or resume an interrupted Rest timer.
            mind.executing = false;
            mind.rest_remaining = 0.0;
            continue;
        }
        let completed_now = outcome == Some(crate::npc::RouteOutcome::Arrived);
        if completed_now {
            let grounded = false;
            finish_objective(
                unit.0,
                &mut mind,
                &mut slot,
                pause_seconds.0,
                grounded,
                &mut actions,
                &mut rest,
            );
        }
        if !mind.executing && mind.rest_remaining > 0.0 {
            if !completed_now {
                mind.rest_remaining = (mind.rest_remaining - dt).max(0.0);
            }
            continue;
        }
        if mind.executing {
            continue; // 在走向目的地的路上，无事可判
        }
        // 停顿门已过（源 TryRest 的产品面）：能走到这里的成员要么停顿计
        // 完（收场时 arm、逐帧倒数到零），要么带旗——出生装配的旗在这
        // 里一次性消费（出生没有收场帧，旗由首判自取）；无对话收场的
        // 旗在收场帧已当帧消费。到此带旗只剩出生后首判一种形态。
        mind.skip_next_rest = false;
        actions.change(crate::npc::NpcAction::Idle, &mut rest);

        // —— 决策梯 ——
        let view = objective::LadderView {
            talk_type: slot.kind(),
            photo_shot: false,
            interrupt: None,
            current: mind.current,
            first_talk_complete: false,
        };
        let mut percent = PercentSource::new(&mut rng);
        let decision = objective::decide(&view, &percents, yet_unread_available, &mut percent);
        // 账目串在这取走（借用就地结束）：后面的目的地链还要借 rng。
        let draws_word = percent.account();
        let (objective_type, lane): (ObjectiveType, Option<TalkLane>) = match decision {
            objective::Decision::RefuelAndTalk => {
                // 补位级联：未读可得即取未读道（替身语义，见模块注释）。
                let lane = if yet_unread_available {
                    TalkLane::YetUnreadFixtureTalk
                } else {
                    TalkLane::GeneralTalk
                };
                (ObjectiveType::Talk, Some(lane))
            }
            objective::Decision::NoneTalk => (ObjectiveType::NoneTalk, None),
            objective::Decision::Select(select::Objective::Talk(lane)) => {
                (ObjectiveType::Talk, Some(lane))
            }
            objective::Decision::Select(select::Objective::NoneTalk) => {
                // 无对话道立无对话目标，槽位同步立无对话（源 ForceUpdate
                // NoneTalkObjective 的补位侧）。
                (ObjectiveType::NoneTalk, None)
            }
            other => unreachable!(
                "决策梯落到了产品不可达的档（{other:?}）：快照里摆拍/打断/\
                 保持档的输入按构造恒假，槽位与当前目标的值域里没有它们"
            ),
        };

        // The ordinary factory selects its master before calculating a target.
        // Keep the same member RNG and do not defer this selection until a click.
        let ordinary = if lane == Some(TalkLane::GeneralTalk) {
            // ForceUpdateGeneralTalkObjective resets its old binding before
            // LotteryGeneralTalkId. This is an AI factory edge, not window end.
            slot.reset_ai_talk_data();
            let mut uniform = UniformSource::new(&mut rng);
            match catalog.lottery(unit.0, &actions.site_type, slot.previous_id(), |len| {
                uniform.draw(len)
            }) {
                Ok(selection) => match if selection.replayed {
                    slot.previous
                        .as_ref()
                        .and_then(|data| data.content.as_ref())
                        .and_then(|content| catalog.resolve_content(content))
                } else {
                    catalog.resolve(selection.master_id)
                } {
                    Some(content) => Some(content),
                    None => {
                        warn!(
                            "[npc unit={}] selected master {} is not loaded",
                            unit.0, selection.master_id
                        );
                        continue;
                    }
                },
                Err(reason) => {
                    trace!("[npc unit={}] ordinary factory: {reason}", unit.0);
                    continue;
                }
            }
        } else {
            None
        };

        // —— 目的地解算 ——
        let mut probe = FaceProbe { face };
        let world_of = |cell: Cell| face.world_of(cell);
        let from = route.navigation_origin(state.0.position, walk_face);
        let detail;
        let mut fixture_selection = None;
        let destination = if let Some(lane) = lane {
            match lane {
                TalkLane::GeneralTalk => {
                    // 社交链：候选 = 同站其余成员（全员同站；对话能力是
                    // 名册的输入合同）。未命中回退游走链（源补位级联里
                    // 「就近他人未命中 → 随机位」的同形级联）。
                    let candidates: Vec<objective::SocialCandidate> = snaps
                        .iter()
                        .filter(|snap| snap.unit != unit.0)
                        .map(|snap| objective::SocialCandidate {
                            position: snap.position,
                            destination: snap.destination,
                            talk_target: snap.destination,
                        })
                        .collect();
                    let npc_positions: Vec<[f32; 3]> =
                        snaps.iter().map(|snap| snap.position).collect();
                    let mut uniform = UniformSource::new(&mut rng);
                    let social = objective::social_target(
                        &candidates,
                        from,
                        &npc_positions,
                        &mut probe,
                        &mut uniform,
                    );
                    if let Some(spot) = social {
                        detail = format!(
                            "社交链 池抽 {}（候选 {} 员）",
                            uniform.account(),
                            candidates.len()
                        );
                        Some(spot)
                    } else {
                        let origin = cell_of(from[0], from[2]);
                        let eligible: Vec<Cell> = face
                            .walkable()
                            .iter()
                            .copied()
                            .filter(|cell| !occupied.contains(cell))
                            .collect();
                        let mut permute = PermuteSource::new(&mut rng);
                        let wander = objective::wander_target(
                            origin,
                            wander_min,
                            wander_max,
                            &eligible,
                            world_of,
                            &mut permute,
                            &mut probe,
                        );
                        detail = format!(
                            "社交未命中→游走 环 {} 格（键 {}）",
                            eligible.len(),
                            permute.keys
                        );
                        wander
                    }
                }
                TalkLane::YetUnreadFixtureTalk
                | TalkLane::FixtureCommonTalk
                | TalkLane::AlreadyReadTalkFixtureTalk => {
                    // 家具道：均匀抽目标家具（源 RandomPick 的替身）；目
                    // 标解算动作点优先（对话道经先行动作行入座），环带兜
                    // 底。座位查对话道切片（先行动作表）。
                    let mut uniform = UniformSource::new(&mut rng);
                    match fixture_target(
                        anchored,
                        face,
                        attach.expect("fixture destinations wait for attachment data"),
                        from,
                        world_of,
                        &mut probe,
                        &mut uniform,
                        move_offset,
                        |fixture_id| talk_seat(fixture_id, unit.0),
                        "对话无先行入座行",
                    ) {
                        FixtureOutcome::ActionPoint {
                            fixture_id,
                            seat,
                            package,
                            landing,
                        } => {
                            action_point_line(unit.0, fixture_id, seat, &package, landing);
                            detail = format!(
                                "动作点 家具 {fixture_id} 座 {seat} 池抽 {}",
                                uniform.account()
                            );
                            Some(landing)
                        }
                        FixtureOutcome::Ring {
                            fixture_id,
                            ring,
                            landing,
                            reason,
                        } => {
                            detail = format!(
                                "家具 {fixture_id} 环 {ring} 格（{reason}）池抽 {}",
                                uniform.account()
                            );
                            Some(landing)
                        }
                        FixtureOutcome::Missed { reason } => {
                            detail = format!("{reason} 池抽 {}", uniform.account());
                            None
                        }
                    }
                }
            }
        } else {
            match fixture_activities.select_none_talk(
                entity,
                unit.0,
                &actions.site_type,
                epoch.0,
                from,
                &placements,
                &fixture_targets,
                face,
                &mut rng,
            ) {
                Ok(Some(selected)) => {
                    detail = format!(
                        "source no-talk action on {:?}/{}",
                        selected.target.entity, selected.target.uid
                    );
                    let position = selected.position;
                    fixture_selection = Some(selected);
                    Some(position)
                }
                Ok(None) => {
                    detail = "no eligible source no-talk fixture action".into();
                    None
                }
                Err(reason) => {
                    fixture_activities.report_pending(entity, unit.0, reason);
                    continue;
                }
            }
        };

        let target_position = destination.unwrap_or(state.0.position);
        if let Some(selected) = &fixture_selection {
            slot.set_current(selected.ai_data());
        } else if let Some(ordinary) = ordinary {
            slot.set_current(AiTalkData {
                kind: TalkType::Common,
                content: Some(ordinary.content()),
                target_fixture: None,
                target_position,
                main_character: unit.0,
                characters: vec![unit.0],
                pre_action: Some(ordinary.pre_action()),
                pending_factory: None,
            });
        } else {
            slot.set_current(AiTalkData::pending(
                lane.map(slot_type_of).unwrap_or(TalkType::NoneTalk),
                unit.0,
                target_position,
            ));
        }
        mind.current = Some(objective_type);
        let source_word = match decision {
            objective::Decision::RefuelAndTalk => "空槽补位",
            objective::Decision::NoneTalk => "无对话槽",
            _ => "梯末抽签",
        };
        let objective_word = match lane {
            Some(lane) => lane_word(lane),
            None => "nonetalk",
        };
        match destination {
            Some(landing) => {
                target.0 = landing;
                mind.executing = true;
                // 身份判定过滤器（b__1）：落点与全表挂点比 x/z 两维
                // （引擎逐分量近似式，见 `fixture_attach`）。命中即贴合
                // 支——目标位 + 挂点朝向交执行层。纯位置比对，不看落点
                // 从哪条链来：动作点落点按构造命中自己的挂点，环带落点
                // 撞上挂点坐标的同样命中。
                let fit = if let Some(selected) = &fixture_selection {
                    Some(FitCandidate {
                        position: selected.position,
                        rotation: selected.rotation,
                    })
                } else {
                    attach
                        .and_then(|attach| attach.matching(landing))
                        .map(|world| FitCandidate {
                            position: landing,
                            rotation: world.rotation,
                        })
                };
                match depart(
                    unit,
                    &mut state.0,
                    &mut path.0,
                    &mut route,
                    walk_face,
                    face,
                    landing,
                    fit,
                    &mut *rng,
                ) {
                    Some(depart_phase) => {
                        if let Some(selected) = fixture_selection.take() {
                            if !fixture_activities.begin(selected, actions.enable_talk) {
                                route.cancel();
                                path.0 = moly_law::path::NpcPathWalkSlot::from_corners(Vec::new());
                                *phase = MotionPhase::Dwelling { remaining: None };
                                finish_objective(
                                    unit.0,
                                    &mut mind,
                                    &mut slot,
                                    pause_seconds.0,
                                    true,
                                    &mut actions,
                                    &mut rest,
                                );
                                continue;
                            }
                        }
                        *phase = depart_phase;
                        crate::npc::declare_navigation_action(
                            &mut actions,
                            &mut rest,
                            &phase,
                            &route,
                        );
                        info!(
                            "[npc unit={}] 目标裁决 {source_word}：{draws_word} 门[{}]={:.0} [{}]={:.0} [{}]={:.0} [{}]={:.0} → {objective_word}（{detail}）→ 落点 ({:.2},{:.2},{:.2})",
                            unit.0,
                            KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
                            percents.fixture_talk,
                            KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
                            percents.already_read_fixture_talk,
                            KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT,
                            percents.none_talk_fixture_action,
                            KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ,
                            percents.already_read_when_has_not_read,
                            landing[0],
                            landing[1],
                            landing[2],
                        );
                    }
                    None => {
                        // 折线全档未命中（无路可达落点）：按未命中收场（与
                        // 三条链全未命中同形——不立跳过旗，进停顿等下一轮）。
                        let grounded = true;
                        finish_objective(
                            unit.0,
                            &mut mind,
                            &mut slot,
                            pause_seconds.0,
                            grounded,
                            &mut actions,
                            &mut rest,
                        );
                        info!(
                            "[npc unit={}] 目标裁决 {source_word}：{draws_word} 门[{}]={:.0} [{}]={:.0} [{}]={:.0} [{}]={:.0} → {objective_word}（{detail}）→ 落点 ({:.2},{:.2},{:.2}) 无路（折线全档未命中），站定",
                            unit.0,
                            KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
                            percents.fixture_talk,
                            KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
                            percents.already_read_fixture_talk,
                            KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT,
                            percents.none_talk_fixture_action,
                            KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ,
                            percents.already_read_when_has_not_read,
                            landing[0],
                            landing[1],
                            landing[2],
                        );
                    }
                }
            }
            None => {
                // 三条链全未命中：站定收场（无对话目标照常补位断环，但不
                // 立跳过旗——见模块注释），进停顿等下一轮。
                let grounded = true;
                finish_objective(
                    unit.0,
                    &mut mind,
                    &mut slot,
                    pause_seconds.0,
                    grounded,
                    &mut actions,
                    &mut rest,
                );
                info!(
                    "[npc unit={}] 目标裁决 {source_word}：{draws_word} 门[{}]={:.0} [{}]={:.0} [{}]={:.0} [{}]={:.0} → {objective_word}（{detail}）→ 未命中，站定",
                    unit.0,
                    KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
                    percents.fixture_talk,
                    KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
                    percents.already_read_fixture_talk,
                    KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT,
                    percents.none_talk_fixture_action,
                    KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ,
                    percents.already_read_when_has_not_read,
                );
            }
        }
    }
}

/// 目标收场：无对话目标补位断环（跳过旗只对执行过的收场立——站定收场
/// 不立，见模块注释），然后过停顿门（旗立起即跳过整轮停顿、当帧可再判；
/// 旗未立进入停顿，时长走停顿律）。
pub(crate) fn finish_objective(
    unit: u32,
    mind: &mut ObjectiveMind,
    slot: &mut TalkSlot,
    pause_seconds: f32,
    grounded: bool,
    actions: &mut crate::npc::NpcActions,
    rest: &mut crate::npc::RestLifecycle,
) {
    mind.executing = false;
    if mind.current == Some(ObjectiveType::NoneTalk) {
        // 断环补位：未读道可得即取未读道（与空槽补位同一条级联替身）。
        let position = slot
            .current
            .as_ref()
            .expect("NoneTalk goal retains its factory record")
            .target_position;
        slot.set_current(AiTalkData::pending(
            slot_type_of(TalkLane::YetUnreadFixtureTalk),
            unit,
            position,
        ));
        if !grounded {
            mind.skip_next_rest = true;
            info!("[npc unit={unit}] 无对话目标收场：补位 → talk:unread，跳过下一轮停顿");
        } else {
            info!("[npc unit={unit}] 无对话目标站定收场：补位 → talk:unread，进入停顿");
        }
    }
    if objective::try_rest_gate(mind.skip_next_rest) == objective::RestGateOutcome::Skipped {
        mind.skip_next_rest = false;
        mind.rest_remaining = 0.0;
        actions.change(crate::npc::NpcAction::Idle, rest);
    } else {
        mind.skip_next_rest = false;
        let milliseconds = objective::rest_delay_milliseconds(pause_seconds).unwrap_or_else(|| {
            panic!("角色 {unit} 的 pauseSeconds 列产不出停顿时长：{pause_seconds}")
        });
        mind.rest_remaining = milliseconds as f32 / 1000.0;
        mind.rest_revision = mind.rest_revision.wrapping_add(1);
        actions.begin_objective_rest(rest, mind.rest_revision);
    }
}

/// 出生/重播种落位：可行走格集上按名册位次等距取格（`index × 总格数 /
/// count`），格角采样落高度。清空与在面由构造承载——可行走格在构建时
/// 已验「容差内采到面」且**不含任何摆放盒内的格**（烘焙刻洞的替身），
/// 落位不再逐点复核。确定性：同名册同一面复算同位。
///
/// 替身，具名：真源的出生点/出生人数未取证（服务端站点管理按进度下
/// 发），这里以「面内等距散布」替身——分散在可行走面上的初始分布，
/// 首判（出生当帧）立刻把每员派往各自的目标。
pub(crate) fn seed_position(face: &ObjectiveFace, index: usize, count: usize) -> [f32; 3] {
    let walkable = face.walkable();
    let cell = walkable[index * walkable.len() / count];
    face.sample(face.world_of(cell), TILE_SCALE)
        .unwrap_or_else(|| {
            panic!("可行走格 {cell:?} 构建时在面上，此刻采样落空——面在决策窗内被换代")
        })
}

/// 三角形上最近点（Ericson《Real-Time Collision Detection》5.1.5）。
fn closest_on_tri(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return add(a, scale(ab, v));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return add(a, scale(ac, w));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(sub(c, b), w));
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    add(a, add(scale(ab, v), scale(ac, w)))
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(a: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] * t, a[1] * t, a[2] * t]
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

#[cfg(test)]
mod navigation_height_tests {
    use super::*;
    use moly_law::carve::{ColliderPolygon, WalkField};

    fn low_step_face() -> ObjectiveFace {
        let tris = vec![
            [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
            [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
        ];
        let step = ColliderPolygon {
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            min_y: 0.0, max_y: 0.05, triangles: Vec::new(), solid: true, carve: false,
        };
        let field = std::sync::Arc::new(WalkField::bake_colliders(&tris, &[step], 0.05));
        let mut buckets = HashMap::new();
        for x in -16..=16 {
            for z in -16..=16 {
                buckets.insert((x, z), vec![0, 1]);
            }
        }
        ObjectiveFace { tris, buckets, grid_min: (-16, -16), grid_max: (16, 16),
            ref_y: 0.0, walkable: Vec::new(), epoch: 1, field, generation: 1 }
    }

    #[test]
    fn navigation_uses_step_height_but_animation_surface_probe_stays_raw() {
        let face = low_step_face();
        assert_eq!(face.surface_sample([0.0, 0.0, 0.0], 0.25).unwrap()[1], 0.0);
        for point in [face.sample([0.0, 0.0, 0.0], 0.25).unwrap(),
            face.navigation_surface_sample([0.0, 0.0, 0.0], 0.25).unwrap(),
            face.reattach_after_layout([0.0, 0.0, 0.0]).unwrap()] {
            assert!((point[1] - 0.05).abs() < 1e-5);
        }
    }

    #[test]
    fn fixture_navigation_move_does_not_accumulate_step_height() {
        let face = low_step_face();
        let start = Vec3::new(0.0, 0.05, 0.0);
        let next = face.fixture_move(start, Vec3::new(0.1, 0.05, 0.0)).unwrap();
        assert!((next.y - 0.05).abs() < 1e-5);
        let outside = face.fixture_move(next, Vec3::new(2.0, 0.05, 0.0)).unwrap();
        assert!(outside.y.abs() < 1e-5);
        let path = face.fixture_path(Vec3::new(-2.0, 0.0, 0.0), start).unwrap();
        assert!((path.last().unwrap().y - 0.05).abs() < 1e-5);
    }

    #[test]
    fn navigation_height_on_a_sloped_rug_never_moves_valid_xz() {
        let mut face = low_step_face();
        for triangle in &mut face.tris {
            for point in triangle {
                point[1] = point[0] * 0.25;
            }
        }
        let top = |x: f32, z: f32| [x, x * 0.25 + 0.05, z];
        let rug = ColliderPolygon {
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            min_y: -0.2, max_y: 0.3,
            triangles: vec![[top(-1.0, -1.0), top(-1.0, 1.0), top(1.0, 1.0)],
                [top(-1.0, -1.0), top(1.0, 1.0), top(1.0, -1.0)]],
            solid: false, carve: false,
        };
        face.field = std::sync::Arc::new(WalkField::bake_colliders(&face.tris, &[rug], 0.05));
        let xz = [0.025, 0.025];
        assert!(face.field.walkable_at(xz));
        let y = xz[0] * 0.25 + face.field.height_offset(xz);
        assert!(face.field.height_offset(xz) > 0.0);
        let original = [xz[0], y, xz[1]];
        for result in [face.navigation_surface_sample(original, 0.25).unwrap(),
            face.reattach_after_layout(original).unwrap(),
            face.fixture_move(Vec3::from(original), Vec3::from(original)).unwrap().to_array()] {
            assert_eq!([result[0], result[2]], xz);
            assert!((result[1] - y).abs() < 1e-5);
        }
        let raw = face.surface_sample(original, 0.25).unwrap();
        assert!((raw[1] - raw[0] * 0.25).abs() < 1e-5,
            "the raw animation probe still samples the site slope");
    }
}
