//! npc 目标机：每名名册成员的目标状态机——步间停顿、决策梯、目标层
//! 抽签与目的地解算，全链消费 `moly_law::objective` 的律。
//!
//! 源的形状（方法体逐段核过）：
//! - 主循环每轮「步间停顿（TryRest）→ 决策梯（DecideObjective）→ 执行
//!   目标 → 丢弃」；停顿在决策**之前**无条件插入，时长 = 1000 ×
//!   trunc(角色表 pauseSeconds 列)，对话态保持续停；「立即可执行下一
//!   目标」旗一次性跳过整轮停顿。**AI 起动时装配（SetUp）先立这面旗**
//!   ——首判不落 15 秒停顿，出生即决策。
//! - 决策梯是一列谓词短路。产品可触发的档有四档：槽位空（两次复位后
//!   建对话数据立对话目标）、槽位无对话（立无对话目标）、对话打断标记
//!   （同一对话数据重立对话目标，随后清可打断位）、全部落空（目标层
//!   百分比抽签，四条对话道各走自己的工厂）。其余档要产品没有的输入
//!   （摆拍、换站/过场保持、问候、摆放后反应），按构造不可达，快照里
//!   按恒假带入。
//! - 目标层抽签：一张 [0,100) 抽签分三窗（家具道 / 已读窗 / 通用对话）；
//!   家具道内第二张分无对话行为与家具对话，家具对话再分第三张（常设 /
//!   已读重温）。
//! - 无对话目标的**收场补位**（ForceUpdateObjective：复位 + 空槽档的建数）
//!   是断环器：无对话槽位不补位，下一轮梯子又立无对话目标、永不换道。
//!   时间轴放完时补两次（取消仍在等待的目标补一次、时间轴自己再补一次），
//!   目标本身在下一帧见到取消才结束；移动失败补一次、当帧结束。补位都在
//!   同一帧做完，两次复位都清掉「立即执行下一目标」旗，所以其后是完整
//!   停顿，停顿后的决策读最后一次补位的数据（通常落到梯末抽签）。
//! - **对话内容来自对话列表**：空槽补位与四条对话道各调源的对话工厂
//!   （`npc_talk_lottery`），抽签读客户端对话列表，不读 master 目录。
//!   通用对话的目标位先走社交链、未命中走游走链，两链都从角色自己的
//!   位置出发，游走档按角色自己所在站点的类型切室内档。落点交给执行层
//!   前过一遍身份判定（落点与挂点比 x/z），命中先导航到接近点，再执行
//!   局部 FitTurning/FitWalking。
//! - **其它角色读现值**：成员按名册序逐个决策，每名成员轮完后从它的组件
//!   回读社交链候选（位置、导航目的地、对话目标位）与家具对话门的视图
//!   （对话类型、当前目标、动作状态、上一条对话），同帧后决策的成员读到
//!   先决策者的新数据。
//! - 普通对话社交池只含同站、不同角色且有真实 Current TalkData 的成员。
//!   导航 destination 与 Current.TargetPosition 是独立输入；后者用于
//!   最终 0.7m 排斥，不能拿前者代替。未完成工厂的占位不算真实 Current。
//! - **选取不验达，出发验一次**：选取只采样；社交链与游走链的验路取静态
//!   CalculatePath 的布尔（部分路径也算成功）。执行层出发前按 MoveAsync
//!   的门对选中目标判一次：对话数据带目标家具的走
//!   IfMoveTargetFixtureActionPosition，失败把该摆放记入本成员的不可达表
//!   （UnmovableFixtureList，两条家具道此后都跳过它，保存布局或换站清空）；
//!   其余走 IfMoveTargetPosition。过门后的路线取 GeneratePath。门未过 =
//!   移动失败，目标收场进停顿。
//! - **决策前的保持**：无对话工厂的 master 表、挂点表与本站本代的站点
//!   快照还在装载时整条决策不抽签（同 CanRunningAI 的位置）。快照已建成
//!   但留有缺口（摆放/地块数据未闭合）不算装载：只让无对话道这次选不出
//!   行为，这一轮按工厂空结果收场，缺口只报一次；工厂读到的其它宿主数据
//!   缺口同样处理——重试改变不了它们，保持就等于让成员永远站住。
//! - **工厂空结果**：普通工厂找不到 master、无对话工厂没有合格行时对话
//!   数据为空，返回的对话目标即刻结束，照常停顿，下一轮走空槽档补位。
//!
//! 替身，具名：
//! * **内容绑定**：通用对话数据保存所选 master、既有 preAction 投影、
//!   目标位与 self 成员。Previous 只在 AI Reset 沿迁移；玩家窗口结束
//!   不更新它。
//! * **未移植的工厂**：家具对话的三个建数工厂（带 timeline 组、带等待
//!   通信、带目标家具）、家具可用性一对谓词、家具标签条件、角色初始化
//!   以来的时长与常设家具对话表不在宿主里；抽签走到它们时按具名缺口
//!   收场（对话数据为空，进停顿），不静默换道。源异常（抽签池空且无
//!   上一条、成员数桶空、行类型越界、缺 master 的解引用）让该角色的
//!   AI 循环终止。
//! * **抽签引擎**：整数档与浮点档都在成员引擎上抽（整数 `[0,n)`、浮点
//!   `[0,总和]` 两端可达），引擎序列本身不在律内。序列上的抽取每次新建
//!   一个运行时随机数生成器，种子取线程种子器的下一个值；种子器由进程级
//!   生成器播种，那个种子来自操作系统随机数，没有回放能复现，宿主取一个
//!   固定根。
//! * **无对话道座位查表**：当前 NoTalk 工厂从真实角色行为行及 timeline
//!   组读取动作点。
//! * **贴合身份判定（b__1）**：源比 AITalkData.TargetPosition 与挂点
//!   StartLoc 的 x/z；产品的落点面是全部已摆放实例的组合挂点世界位
//!   （`fixture_attach`，含未锚定摆放）——环带落点恰好撞上挂点坐标也
//!   命中判定，与源同形（执行层不问目标从哪条道来）。
//! * **入场目标**：源装配先立打断标记 14（入场站目标）再开主循环；产品
//!   不建模入场位（成员直接落座在可行走面上），只迁它的时序承重件——
//!   起动旗（首判立即执行）原样迁。
//! * **可行格与验路**：出生、目标与执行共用侵蚀后格场。高度几何只给
//!   已验证的导航位置落高；格场仍是原生导航网格的近似，引擎路径查询的
//!   查询盒映射、部分路径与末拐点判定在它上面转写。
//! * **移动失败收场的无对话目标**：补位照常（断环），不跳过停顿——源在
//!   执行开始时立的旗，被移动失败分支的 ForceUpdateObjective（先 Reset）
//!   清掉。
//!
//! 目标面（[`ObjectiveFace`]）与玩家的约束面同源网格（站点域解析的
//! navmesh 面），但保留高度：目标域的采样要的是面上的三维最近点
//! （源 SamplePosition 语义），投影面裁不了高度。站点换装后面按代数
//! 重建；构建端带格桶索引，逐格最近点查询不扫全网格。

use crate::client_config::{
    ClientConfigs, KEY_CHARACTER_FIXTURE_MOVE_OFFSET, KEY_CHARACTER_GATE_ACTION_ELAPSED_TIME,
    KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
    KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ, KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
    KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT, KEY_NPC_LOTTERY_TALK1_WIGHT,
    KEY_NPC_LOTTERY_TALK2_WIGHT, KEY_NPC_LOTTERY_TALK3_WIGHT, KEY_NPC_LOTTERY_TALK4_WIGHT,
    KEY_NPC_RANDOM_MOVE_IN_ROOM_MAX_DISTANCE, KEY_NPC_RANDOM_MOVE_IN_ROOM_MIN_DISTANCE,
    KEY_NPC_RANDOM_MOVE_MAX_DISTANCE, KEY_NPC_RANDOM_MOVE_MIN_DISTANCE,
};
use crate::fixture::FixturePlacements;
use crate::npc::{
    depart_along, CharacterUnitId, FitCandidate, MotionPhase, MoveTarget, PathSlot, PauseSeconds,
    RouteStops, WalkState,
};
use crate::npc_talk_lottery;
use crate::site::{GroundEpoch, SiteSelection, WalkFaceMeshes};
use bevy::prelude::*;
use moly_law::objective::{self, Cell, ObjectiveType, SurfaceProbe, TalkType, TILE_SCALE};
use moly_law::talk::select::{
    self, LotteryPercents, LotteryWeights, PercentDraw, TalkLane, UniformDraw,
};
use moly_law::talk::{EnumerablePick, NetThreadSeeder};
use std::collections::{HashMap, HashSet};

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
    /// SitePosition.y of the site this face was built for. The site renders
    /// at the origin, so this is the one world height the face's local
    /// coordinates drop; MoveAsync's IsCompleted reads it.
    site_height: f32,
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

    /// SitePosition.y of this face's site.
    pub(crate) fn site_height(&self) -> f32 {
        self.site_height
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

    /// The social IsNavigable predicate and the wander search both take the
    /// boolean of the static `NavMesh.CalculatePath`, which also succeeds for
    /// a partial path. Whether the target itself is reached is decided later,
    /// by the objective's departure gate.
    fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool {
        self.face.field.can_calculate_path(
            [source[0], source[2]],
            [target[0], target[2]],
            moly_law::carve::STATIC_QUERY_HALF_EXTENT,
        )
    }
}

/// A fixture talk the factories built and this host executes: its data, its
/// loaded script, the fixture entity, and for a pre-action with a timeline
/// the fixture session that runs its window and timeline.
struct PreparedFixtureTalk {
    data: npc_talk_lottery::FixtureTalkData,
    resolved: crate::player_talk::ResolvedTalk,
    /// The master reference with IsGeneralTalk read from its conditions.
    content: TalkContent,
    fixture: Entity,
    timeline: Option<crate::npc_fixture_activity::Selection>,
    /// The fixture's StartLoc names in action-point array order.
    names: Vec<String>,
    /// A group talk's other members as PassTalkDataToSubCharacters met them.
    drafts: Vec<crate::npc_fixture_talk::MemberDraft>,
}

impl PreparedFixtureTalk {
    fn is_group(&self) -> bool {
        matches!(
            self.data.kind,
            TalkType::CommunicationWhileDoingWait | TalkType::MultipleCharacterFixture
        )
    }
}

/// One other member of a group talk at the main's decision.
struct RawDraft {
    entity: Entity,
    unit: u32,
    state: u8,
    drafted: bool,
    cancel: bool,
    data: Option<npc_talk_lottery::FixtureTalkData>,
    steps: Vec<serde_json::Value>,
}

/// PassTalkDataToSubCharacters' members, in the data's order, as the main's
/// decision sees them: eligible when its state is a fixture-action state
/// (11, 12, 17, 18, 19) or TryCancel on its current objective would report
/// true while it is not talking. The data a while-doing-wait member would
/// get (TryCreateWaitingWithCommunicationTalkData with itself as the
/// character) draws nothing and is built for every member; the drafts land
/// after the frame's decisions, where eligibility and the cancel are read
/// again.
///
/// For the multiple-character talk only an eligible member is handed data,
/// and building it draws one sequence pick (in the members' order, after the
/// main's factory draws): the eligibility is the one the main's decision
/// sees.
fn member_drafts(
    scene: &npc_talk_lottery::LotteryScene<'_>,
    draws: &mut npc_talk_lottery::Draws<'_>,
    snaps: &[MemberSnap],
    views: &[npc_talk_lottery::NpcView],
    main_unit: u32,
    data: &npc_talk_lottery::FixtureTalkData,
) -> Result<Vec<RawDraft>, npc_talk_lottery::Halt> {
    let mut drafts = Vec::new();
    for &member in data.members.iter().filter(|unit| **unit != main_unit) {
        let Some(snap) = snaps.iter().find(|snap| snap.unit == member) else {
            continue;
        };
        let fixture_state = matches!(snap.state, 11 | 12 | 17 | 18 | 19);
        let cancellable = snap.state != 4 && snap.cancel_reports;
        let drafted = fixture_state || cancellable;
        let cancel = match data.kind {
            TalkType::CommunicationWhileDoingWait => true,
            _ => !fixture_state,
        };
        let (member_data, steps) = match views.iter().find(|view| view.unit == member) {
            Some(view) if data.kind == TalkType::CommunicationWhileDoingWait => {
                npc_talk_lottery::waiting_data_for_member(scene, view, data.talk_id, &data.fixture)?
            }
            Some(_) if data.kind == TalkType::MultipleCharacterFixture && drafted => {
                let (member_data, steps) =
                    npc_talk_lottery::some_character_data_for_member(scene, draws, member, data)?;
                if member_data.is_none() {
                    // The caller indexes the locate list at the member's
                    // position before the factory runs (argument out of
                    // range without a row); a missing presenter is a null
                    // dereference; a null factory result goes to
                    // SetAITalkData, which logs and builds the member general
                    // data from one more general-talk lottery and the
                    // general target chain, not evaluated here.
                    let result = steps
                        .iter()
                        .rev()
                        .find_map(|step| step.get("result").and_then(|value| value.as_str()))
                        .unwrap_or("none");
                    return Err(match result {
                        "not_in_list" | "locate_short" => npc_talk_lottery::Halt::Fault(format!(
                            "member {member}'s position in the character list has no locate row (argument out of range)"
                        )),
                        "no_presenter" => npc_talk_lottery::Halt::Fault(format!(
                            "member {member} has no presenter (null dereference)"
                        )),
                        _ => npc_talk_lottery::Halt::Gap(format!(
                            "member {member}'s data factory returned null ({result}); SetAITalkData(null) gives it general data from a general-talk lottery and the general target chain, which a member's hand-over does not evaluate"
                        )),
                    });
                }
                (member_data, steps)
            }
            _ => (None, Vec::new()),
        };
        drafts.push(RawDraft {
            entity: snap.entity,
            unit: member,
            state: snap.state,
            drafted,
            cancel,
            data: member_data,
            steps,
        });
    }
    Ok(drafts)
}

/// A face probe that counts its samples (GetLittleFarPosition's walk).
struct CountingProbe<'a> {
    face: &'a ObjectiveFace,
    samples: usize,
}

impl SurfaceProbe for CountingProbe<'_> {
    fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
        self.samples += 1;
        self.face.sample(target, tolerance)
    }

    fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool {
        self.face.has_path(source, target)
    }
}

type TalkGeometry = crate::npc_fixture_activity::TalkFixtureGeometry;

/// The fixture-talk factories' host for one decision: the avatar list's
/// live positions, each placed fixture's geometry (resolved once), the
/// objective face and the configured fixture move offset (key 153). Every
/// GetLittleFarPosition walk goes into `walks` for the decision record.
struct TalkFixtureHost<'a> {
    positions: Vec<(u32, [f32; 3])>,
    geometry: &'a dyn Fn(&str) -> Result<TalkGeometry, String>,
    resolved: std::cell::RefCell<HashMap<String, Result<std::rc::Rc<TalkGeometry>, String>>>,
    face: &'a ObjectiveFace,
    move_offset: f32,
    walks: std::cell::RefCell<Vec<serde_json::Value>>,
}

impl TalkFixtureHost<'_> {
    fn fixture(&self, uid: &str) -> Result<std::rc::Rc<TalkGeometry>, npc_talk_lottery::Halt> {
        self.resolved
            .borrow_mut()
            .entry(uid.to_owned())
            .or_insert_with(|| (self.geometry)(uid).map(std::rc::Rc::new))
            .clone()
            .map_err(|reason| npc_talk_lottery::Halt::Gap(format!("fixture {uid}: {reason}")))
    }
}

impl npc_talk_lottery::FixtureTalkHost for TalkFixtureHost<'_> {
    fn npc_position(&self, unit: u32) -> Option<[f32; 3]> {
        self.positions
            .iter()
            .find(|(member, _)| *member == unit)
            .map(|(_, position)| *position)
    }

    fn action_point_names(&self, fixture: &str) -> Result<Vec<String>, npc_talk_lottery::Halt> {
        Ok(self
            .fixture(fixture)?
            .locators
            .iter()
            .map(|(name, _)| name.clone())
            .collect())
    }

    fn start_loc(
        &self,
        fixture: &str,
        index: usize,
    ) -> Result<([f32; 3], [f32; 4]), npc_talk_lottery::Halt> {
        let geometry = self.fixture(fixture)?;
        let (_, pair) = geometry.locators.get(index).ok_or_else(|| {
            npc_talk_lottery::Halt::Fault(format!(
                "locator index {index} of {fixture} is outside its array (argument out of range)"
            ))
        })?;
        Ok((pair.start.position, pair.start.rotation.to_array()))
    }

    fn site_height(&self, fixture: &str) -> Result<Option<f32>, npc_talk_lottery::Halt> {
        Ok(Some(self.fixture(fixture)?.site_origin[1]))
    }

    fn little_far_position(
        &self,
        position: [f32; 3],
        fixture: &str,
    ) -> Result<Option<[f32; 3]>, npc_talk_lottery::Halt> {
        let geometry = self.fixture(fixture)?;
        let (box_min, box_max) = geometry.footprint;
        let (grid_min, grid_max) = geometry.floor_bounds;
        let cells = objective::approach_ring_cells(
            box_min,
            box_max,
            objective::APPROACH_SEARCH_RANGE,
            grid_min,
            grid_max,
        );
        let origin = geometry.site_origin;
        // TILE_SIZE * (x, 0, z) + the site origin.
        let world_of = |cell: Cell| {
            [
                cell.0 as f32 * TILE_SCALE + origin[0],
                origin[1],
                cell.1 as f32 * TILE_SCALE + origin[2],
            ]
        };
        let mut probe = CountingProbe {
            face: self.face,
            samples: 0,
        };
        let hit = objective::approach_target(&cells, position, world_of, &mut probe, self.move_offset);
        self.walks.borrow_mut().push(serde_json::json!({
            "fixture": fixture,
            "from": position,
            "footprint": [box_min, box_max],
            "floor": [grid_min, grid_max],
            "origin": origin,
            "ring": cells.len(),
            "samples": probe.samples,
            "tolerance": self.move_offset,
            "hit": hit,
        }));
        Ok(hit)
    }

    fn sample_hit(&self, point: [f32; 3], radius: f32) -> bool {
        self.face.sample(point, radius).is_some()
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
    site: Option<Res<crate::site::SiteActive>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
) {
    let epoch = epoch.map(|epoch| epoch.0).unwrap_or(0);
    let Some(walk_face) = walk_face else {
        return;
    };
    let Some(site_height) = site.as_deref().map(|site| site.position[1]) else {
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
        site_height,
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
    /// FixtureNpcActionLocateDataList: the fixture-talk factories' list,
    /// `None` for data without one.
    pub(crate) locate: Option<Vec<crate::npc_talk_lottery::LocateRow>>,
    pub(crate) pending_factory: Option<&'static str>,
}

impl AiTalkData {
    #[cfg(test)]
    fn pending(kind: TalkType, unit: u32, position: [f32; 3]) -> Self {
        Self {
            kind,
            content: None,
            target_fixture: None,
            target_position: position,
            main_character: unit,
            characters: vec![unit],
            pre_action: None,
            locate: None,
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
    /// The AI model's interrupt marker. The general and fixture-common
    /// factories raise the Talk marker; the ladder's interrupt row clears its
    /// interrupt flag; a reset clears the marker.
    pub(crate) interrupt: Option<objective::InterruptMarker>,
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
        self.interrupt = None;
    }
}

/// 成员抽签引擎：跨帧线性同余（与配对域同款——律只约束分布与求值次数，
/// 引擎序列不在律内）。种子按 unit id 定推，逐成员可复算、不受查询序
/// 影响。
#[derive(Component, Clone)]
pub struct MemberRng {
    state: u64,
    /// Generator calls so far; a decision record reports its own share.
    calls: u64,
}

/// Eight bytes from the platform's random source. The source's generators
/// are seeded from operating-system randomness and the clock, so every launch
/// draws a different sequence; a platform without a random source is refused.
pub(crate) fn platform_seed() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)
        .unwrap_or_else(|error| panic!("the platform random source seeds the NPC generators: {error}"));
    u64::from_le_bytes(bytes)
}

impl MemberRng {
    /// A spawned NPC's generator, seeded from the platform (see [`platform_seed`]).
    pub(crate) fn from_platform() -> Self {
        Self {
            state: platform_seed(),
            calls: 0,
        }
    }

    /// 种子：unit id 经两轮固定扰动（不与任何面板值混源）。
    pub(crate) fn seeded(unit: u32) -> Self {
        Self {
            state: 0x9E37_79B9_7F4A_7C15_u64 ^ (unit as u64).wrapping_mul(0x2545_F491_4F6C_DD1D),
            calls: 0,
        }
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.calls += 1;
        self.state
    }

    pub(crate) fn calls(&self) -> u64 {
        self.calls
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
    /// 起动装配立起（源 SetUp 同形）；问候的执行也立它。
    pub skip_next_rest: bool,
    /// The objective body's current wait, once the body is past its move.
    pub(crate) body: Option<BodyWait>,
    /// The loop's Yield: the objective ended (or was cancelled) on this
    /// frame, and the next iteration (TryRest) runs on a later frame.
    pub(crate) yield_since: Option<u32>,
    /// The Rest objective: its delay, then its wait while talking.
    pub(crate) rest: Option<RestPhase>,
    /// ForceUpdateObjective calls a no-talk objective's end still owes, each
    /// a reset and a row-1 cascade, made before the next TryRest.
    pub(crate) force_updates: u8,
    /// The frame on which that no-talk objective itself ends: the frame of a
    /// failed move, or the frame after its timeline ended (its wait sees the
    /// cancel on its next poll).
    pub(crate) force_update_end: u32,
    /// The frame on which the overlap cancel (presenter call 8) fired; the
    /// greeting gate does not run on that frame.
    pub(crate) overlap_cancel_frame: Option<u32>,
    /// A named greeting failure (a source exception in the gate frame): no
    /// further greeting is attempted for this character.
    pub(crate) greeting_halt: Option<String>,
    /// Identifies each newly armed objective Rest interval, not each frame.
    pub(crate) rest_revision: u64,
    /// TryCancelIfCharacterOverlap's timer (presenter call 8).
    pub(crate) overlap_seconds: f32,
    /// A cancelled ordinary target completes its coroutine, then the AI loop
    /// enters its next TryRest after Show. Cancelling a running Rest instead
    /// continues past that Rest. Neither operation resets talk data.
    edit_rest_after: Option<ObjectiveType>,
    /// UnmovableFixtureList: placement UIDs whose departure gate failed.
    /// Both fixture lanes skip them until a saved layout or a new site
    /// setup clears the list.
    pub(crate) unmovable_fixtures: Vec<String>,
    /// Set when a source exception ended this character's AI loop (the loop
    /// rethrows everything but cancellation and its own cannot-decide
    /// exception); no further decision is made.
    pub(crate) ai_stopped: Option<String>,
    /// The current objective was cancelled (NPCObjectiveBase +0x28); a
    /// later TryCancel on it reports true again. A new objective clears it.
    pub(crate) cancelled: bool,
}

impl ObjectiveMind {
    /// A new current objective (the AI model's _currentObjective write).
    pub(crate) fn begin_objective(&mut self, objective: ObjectiveType) {
        self.current = Some(objective);
        self.cancelled = false;
    }

    /// What NPCObjectiveBase.TryCancel on the current objective reports:
    /// no current objective, or one that completed (disposed), reports
    /// false; one cancelled before reports true; a running one is cancelled
    /// (true). CanCancel is 1 for every objective this host runs.
    pub(crate) fn cancel_reports(&self) -> bool {
        self.current.is_some()
            && (self.cancelled || self.executing || self.rest.is_some() || self.body.is_some())
    }
}

impl moly_law::path::WaypointDraw for MemberRng {
    fn index(&mut self, upper_exclusive: usize) -> usize {
        ((self.next() >> 33) as usize) % upper_exclusive
    }
}

/// A wait inside an objective body (each created on `since` and first
/// tested on a later frame, as the source's WaitUntil and Delay are).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BodyWait {
    /// The talk objective's tweet: until the tweet state is Done.
    TweetDone { since: u32, step: TweetStep },
    /// The talk objective's tweet: until the character is not talking.
    NotTalking { since: u32, step: TweetStep },
    /// The greeting objective: until the first talk is complete.
    FirstTalk { since: u32 },
    /// A Stacked move: the scaled 1.0 s delay before the move fails.
    Stacked {
        since: u32,
        delay: objective::DelayPromise,
    },
}

/// Which tweet of the talk objective a wait belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TweetStep {
    /// The common-fixture tweet (type 5); the pre-action follows it.
    CommonFixture,
    /// The pre-action tweet; the pre-action's timeline follows it.
    PreAction,
}

/// The Rest objective's two waits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RestPhase {
    Delay {
        since: u32,
        delay: objective::DelayPromise,
    },
    /// WaitWhile(Talk) after the delay.
    WaitTalk { since: u32 },
}

impl ObjectiveMind {
    /// Whether the Rest objective (or the loop's Yield before TryRest) holds
    /// the character.
    pub(crate) fn resting(&self) -> bool {
        self.rest.is_some()
    }

    /// Seconds of the Rest delay still to run (0 outside the delay).
    pub(crate) fn rest_seconds_left(&self) -> f32 {
        match self.rest {
            Some(RestPhase::Delay { delay, .. }) => (delay.delay() - delay.elapsed()).max(0.0),
            _ => 0.0,
        }
    }

    /// 出生态：起动旗立起（首判立即执行），停顿零，无当前目标。
    pub(crate) fn at_spawn() -> Self {
        Self {
            executing: false,
            current: None,
            skip_next_rest: true,
            body: None,
            yield_since: None,
            rest: None,
            force_updates: 0,
            force_update_end: 0,
            overlap_cancel_frame: None,
            greeting_halt: None,
            rest_revision: 0,
            overlap_seconds: 0.0,
            edit_rest_after: None,
            unmovable_fixtures: Vec::new(),
            ai_stopped: None,
            cancelled: false,
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
        && (mind.resting() || actions.current == crate::npc::NpcAction::Rest);
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
    mind.body = None;
    mind.rest = None;
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

/// One committed decision as one machine-readable log line,
/// `[npc-decision] {json}`, so a replay can recompute it from its inputs:
/// the ladder row taken, every draw with the source draw it stands for
/// (overload and generator), the four objective percents as passed to the
/// ladder, the unread input as passed together with the talk list's own
/// answer and digest, the path the talk data took, the chosen talk and the
/// outcome. `rng_calls` counts every product generator call the decision
/// made, including the ones inside the no-talk factory, which draws through
/// the same generator but is not itemised here. A retry that discards its
/// draws while inputs are still loading is not a decision and has no line.
struct DecisionRecord {
    fields: serde_json::Map<String, serde_json::Value>,
    draws: Vec<serde_json::Value>,
}

/// Decision lines written so far; a character whose turn wrote one has its
/// live view logged before the next character's turn.
static DECISIONS_EMITTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The source's integer engine range, max exclusive.
const DRAW_ENGINE_INT_RANGE: &str = "UnityEngine.Random.Range(int,int)";
/// A fresh sort key per element (a uniform permutation).
const DRAW_GUID_KEYS: &str = "Guid.NewGuid";

impl DecisionRecord {
    #[allow(clippy::too_many_arguments)]
    fn new(
        unit: u32,
        site_type: &str,
        seconds: f64,
        row: &str,
        percents: &LotteryPercents,
        weights: &LotteryWeights,
        unread_passed: bool,
        talk_list: &crate::server_panel::TalkDataStore,
        percent_draws: &[u32],
    ) -> Self {
        let (rows, unread, digest) = talk_list.digest();
        let mut fields = serde_json::Map::new();
        fields.insert("v".into(), 1.into());
        fields.insert("unit".into(), unit.into());
        fields.insert("site".into(), site_type.into());
        fields.insert("t".into(), seconds.into());
        fields.insert("row".into(), row.into());
        fields.insert(
            "percents".into(),
            serde_json::json!({
                KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT.to_string(): percents.fixture_talk,
                KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT.to_string(): percents.already_read_fixture_talk,
                KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT.to_string(): percents.none_talk_fixture_action,
                KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ.to_string(): percents.already_read_when_has_not_read,
            }),
        );
        fields.insert(
            "weights".into(),
            serde_json::json!({
                KEY_NPC_LOTTERY_TALK1_WIGHT.to_string(): weights.talk1,
                KEY_NPC_LOTTERY_TALK2_WIGHT.to_string(): weights.talk2,
                KEY_NPC_LOTTERY_TALK3_WIGHT.to_string(): weights.talk3,
                KEY_NPC_LOTTERY_TALK4_WIGHT.to_string(): weights.talk4,
            }),
        );
        fields.insert(
            "unread".into(),
            serde_json::json!({
                "passed": unread_passed,
                "passed_from": "talk list: any unread row",
                "talk_list_any_unread": talk_list.any_unread(),
            }),
        );
        fields.insert(
            "talk_list".into(),
            serde_json::json!({
                "rows": rows,
                "unread": unread,
                "digest": format!("{digest:016x}"),
                "revision": talk_list.revision(),
            }),
        );
        let mut record = Self {
            fields,
            draws: Vec::new(),
        };
        for (index, value) in percent_draws.iter().enumerate() {
            record.draws.push(serde_json::json!({
                "use": if index == 0 { "select_objective" } else { "select_fixture_talk" },
                "value": value,
                "range": [0, 100],
                "source": DRAW_ENGINE_INT_RANGE,
            }));
        }
        record
    }

    fn set(&mut self, key: &str, value: impl Into<serde_json::Value>) {
        self.fields.insert(key.into(), value.into());
    }

    fn uniform(&mut self, use_word: &str, source: &str, draws: &[(usize, usize)]) {
        for (index, len) in draws {
            self.draws.push(serde_json::json!({
                "use": use_word,
                "value": index,
                "range": [0, len],
                "source": source,
            }));
        }
    }

    fn extend_draws(&mut self, draws: Vec<serde_json::Value>) {
        self.draws.extend(draws);
    }

    fn keys(&mut self, use_word: &str, keys: usize) {
        if keys > 0 {
            self.draws.push(serde_json::json!({
                "use": use_word,
                "keys": keys,
                "source": DRAW_GUID_KEYS,
            }));
        }
    }

    fn emit(mut self, outcome: &str, rng_calls: u64) {
        DECISIONS_EMITTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.fields.insert("outcome".into(), outcome.into());
        self.fields.insert("rng_calls".into(), rng_calls.into());
        self.fields
            .insert("draws".into(), serde_json::Value::Array(self.draws));
        info!("[npc-decision] {}", serde_json::Value::Object(self.fields));
    }
}

/// The talk factory a decision row calls (see `npc_talk_lottery`).
#[derive(Debug, Clone, Copy)]
enum TalkFactory {
    /// Row 1: ForceUpdateObjective's CreateAITalkData.
    CreateAiTalkData,
    YetUnreadFixtureTalk,
    GeneralTalk,
    AlreadyReadFixtureTalk,
    FixtureCommonTalk,
}

impl TalkFactory {
    fn of_lane(lane: TalkLane) -> Self {
        match lane {
            TalkLane::YetUnreadFixtureTalk => TalkFactory::YetUnreadFixtureTalk,
            TalkLane::GeneralTalk => TalkFactory::GeneralTalk,
            TalkLane::AlreadyReadTalkFixtureTalk => TalkFactory::AlreadyReadFixtureTalk,
            TalkLane::FixtureCommonTalk => TalkFactory::FixtureCommonTalk,
        }
    }

    fn word(self) -> &'static str {
        match self {
            TalkFactory::CreateAiTalkData => "talk:create",
            TalkFactory::YetUnreadFixtureTalk => "talk:unread",
            TalkFactory::GeneralTalk => "talk:general",
            TalkFactory::AlreadyReadFixtureTalk => "talk:read",
            TalkFactory::FixtureCommonTalk => "talk:common",
        }
    }

    fn run(
        self,
        scene: &npc_talk_lottery::LotteryScene<'_>,
        seeker: &npc_talk_lottery::NpcView,
        draws: &mut npc_talk_lottery::Draws<'_>,
    ) -> Result<npc_talk_lottery::TalkPlan, npc_talk_lottery::Halt> {
        match self {
            TalkFactory::CreateAiTalkData => {
                npc_talk_lottery::create_ai_talk_data(scene, seeker, draws)
            }
            TalkFactory::YetUnreadFixtureTalk => {
                npc_talk_lottery::force_update_yet_read_talk_fixture_talk(scene, seeker, draws)
            }
            TalkFactory::GeneralTalk => {
                npc_talk_lottery::force_update_general_talk_objective(scene, seeker, draws)
            }
            TalkFactory::AlreadyReadFixtureTalk => {
                npc_talk_lottery::force_update_already_read_talk_fixture_talk(scene, seeker, draws)
            }
            TalkFactory::FixtureCommonTalk => {
                npc_talk_lottery::change_fixture_common_talk_objective(scene, seeker, draws)
            }
        }
    }
}

/// What a decision builds: talk data from a factory, the same talk data
/// again (the Talk interrupt), or a no-talk objective.
#[derive(Debug, Clone, Copy)]
enum DecisionRoute {
    Factory(TalkFactory),
    SameTalkData,
    /// A drafted member's sub objective (9 or 18) on its existing data.
    SubObjective(ObjectiveType),
    NoneTalk,
    /// The greeting objective on the greeting data the gate wrote.
    Greeting,
    /// The change-site objective on the change-site data (row 8, or the
    /// change-site interrupt); its body is `npc::change_site_state`'s.
    ChangeSite,
}

impl DecisionRoute {
    fn word(self) -> &'static str {
        match self {
            DecisionRoute::Factory(factory) => factory.word(),
            DecisionRoute::SameTalkData => "talk:interrupt",
            DecisionRoute::SubObjective(_) => "sub:interrupt",
            DecisionRoute::NoneTalk => "nonetalk",
            DecisionRoute::Greeting => "greeting",
            DecisionRoute::ChangeSite => "change_site",
        }
    }
}

/// The Talk interrupt marker the general and fixture-common factories raise.
const TALK_INTERRUPT: objective::InterruptMarker = objective::InterruptMarker {
    marker_type: 4,
    can_interrupt: true,
};

/// The main thread's seeder of parameterless System.Random generators, which
/// every sequence pick of the decisions and the greeting pick builds from.
#[derive(Resource, Default, Clone)]
pub(crate) struct SequencePickSeeder(Option<NetThreadSeeder>);

impl SequencePickSeeder {
    fn seeder(&mut self) -> &mut NetThreadSeeder {
        self.0
            // The source seeds this seeder from the process-wide generator,
            // whose seed comes from operating-system random bytes.
            .get_or_insert_with(|| NetThreadSeeder::from_root(platform_seed() as i32))
    }

    /// A pick over a sequence of `count` elements (see [`EnumerablePick`]).
    pub(crate) fn pick(&mut self, count: usize) -> Option<usize> {
        self.seeder().pick(count)
    }

    /// `new Random()` alone (a pick that raises after building it).
    pub(crate) fn fresh(&mut self) {
        self.seeder().fresh();
    }
}

/// The engine integer range `[0, len)` on the member generator.
pub(crate) fn engine_int_draw(rng: &mut MemberRng, len: usize) -> usize {
    ((rng.next() >> 33) as usize) % len.max(1)
}

/// The engine float range `[0, total]` on the member generator, both ends
/// reachable: 23 bits scaled onto the range.
pub(crate) fn engine_float_draw(rng: &mut MemberRng, total: f32) -> f32 {
    let bits = (rng.next() >> 41) as u32;
    bits as f32 / 8_388_607.0 * total
}

/// 动作点支的面采样门容差（米）：挂点 x/z 上的可行走面采样半径
/// （源动作点门里采样调用的同值）。
pub(crate) const SAMPLE_GATE_TOLERANCE: f32 = 0.3;

/// 成员快照（社交链的候选输入与游走链的占格都按它算）。
struct MemberSnap {
    entity: Entity,
    target_fixture: Option<Entity>,
    unit: u32,
    site_type: String,
    position: [f32; 3],
    /// 当前导航目的地：执行中 = 目标目的地，停顿中 = 原地（无路径）。
    destination: [f32; 3],
    /// A source Current may be a valid NoTalk record without a master story.
    /// Only a missing Current or our explicit unfinished factory is excluded.
    talk_target: Option<[f32; 3]>,
    /// The action state value.
    state: u8,
    /// What TryCancel on its current objective would report.
    cancel_reports: bool,
}

fn prepared_social_target(slot: &TalkSlot) -> Option<[f32; 3]> {
    slot.current.as_ref()
        .filter(|data| data.pending_factory.is_none())
        .map(|data| data.target_position)
}

impl MemberSnap {
    fn social_candidate(&self, unit: u32, site_type: &str) -> Option<objective::SocialCandidate> {
        if self.unit == unit || self.site_type != site_type {
            return None;
        }
        Some(objective::SocialCandidate {
            position: self.position,
            destination: self.destination,
            talk_target: self.talk_target?,
        })
    }
}

/// Update：目标机推进——停顿计时（对话态冻结）、路线尽收场（无对话目标
/// 的断环补位）、停顿尽后的决策（梯子 → 抽签 → 目的地解算 → 出发）。
/// 出发先过 MoveAsync 的门，再经 [`depart_along`] 把 GeneratePath 的拐点
/// 一次排成路点表；门未过或没有拐点按移动失败收场。
///
/// 一次决策的抽签账目一行日志：抽签落点、门占比、目标家具与环、落点
/// 坐标——「每员目标选取可从日志复算」。
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn decide(
    (time, frame_count, clock): (
        Res<Time>,
        Res<bevy::diagnostic::FrameCount>,
        Res<crate::npc_clock::NpcClock>,
    ),
    editor: Res<crate::fixture_edit::EditSessionActive>,
    mut saved_layouts: MessageReader<crate::fixture_edit::LayoutSaved>,
    face: Option<Res<ObjectiveFace>>,
    epoch: Option<Res<GroundEpoch>>,
    selection: Option<Res<SiteSelection>>,
    placements: Res<FixturePlacements>,
    (mut seeder, mut reported_gaps, mut last_placed, mut groups): (
        ResMut<SequencePickSeeder>,
        Local<HashSet<String>>,
        Local<String>,
        ResMut<crate::npc_fixture_talk::FixtureTalkGroups>,
    ),
    (configs, talk_list, together): (
        Option<Res<ClientConfigs>>,
        Option<Res<crate::server_panel::TalkDataStore>>,
        Option<Res<crate::fixture_activity_data::TogetherCommunicationTable>>,
    ),
    (walk_face, mut change_site): (
        Option<Res<crate::walk_face::WalkFace>>,
        ResMut<crate::npc::change_site_state::ChangeSiteRuns>,
    ),
    attach_worlds: Option<Res<crate::fixture_attach::AttachWorlds>>,
    players: Query<&Transform, With<crate::player::PlayerControlled>>,
    catalog: crate::player_talk::TalkCatalog,
    mut fixture_activities: crate::npc_fixture_activity::Factory,
    mut npcs: Query<
        (
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
        ),
        Without<crate::npc::residency::Away>,
    >,
) {
    // The site controllers clear every NPC's UnmovableFixtureList when a
    // layout edit is saved. Read before the editor return below: the save is
    // written while the editor still owns the actors.
    if saved_layouts.read().count() > 0 {
        for (_, _, _, _, _, mut mind, ..) in &mut npcs {
            mind.unmovable_fixtures.clear();
        }
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
    // The talk list is set before any NPC exists; hold without it.
    let Some(talk_list) = talk_list.as_deref() else {
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
    // The together-communication table loads on its own; its loader always
    // installs a result (the rows, or why this asset source lacks them).
    let Some(together) = together.as_deref() else {
        return;
    };
    // No decision before the engine's frame clock runs (and none at all if
    // its settings were refused).
    if !clock.ready() {
        return;
    }
    let dt = clock.delta();
    let frame = frame_count.0;

    // 成员快照：社交链的候选（位置 + 导航目的地 + 对话目标位）与游走链的
    // 占格都从全员位置算。每名成员决策后按它的现值刷新自己那一格，后决策
    // 的成员读到的是先决策者的新数据（源逐个续跑各成员的 AI、读现值）。
    let mut snaps: Vec<MemberSnap> = npcs
        .iter()
        .map(
            |(entity, unit, _, _, slot, mind, _, _, state, target, _, _, actions, _)| MemberSnap {
                entity,
                target_fixture: slot.current.as_ref().and_then(|data| data.target_fixture),
                unit: unit.0,
                site_type: actions.site_type.clone(),
                talk_target: prepared_social_target(slot),
                state: actions.current as u8,
                cancel_reports: mind.cancel_reports(),
                position: state.0.position,
                destination: if mind.executing {
                    target.0
                } else {
                    state.0.position
                },
            },
        )
        .collect();
    let mut occupied: std::collections::HashSet<Cell> = snaps
        .iter()
        .map(|snap| cell_of(snap.position[0], snap.position[2]))
        .collect();
    for player in &players {
        let translation = player.translation;
        occupied.insert(cell_of(translation.x, translation.z));
    }

    // 面板占比与成员数权重。
    let percents = LotteryPercents {
        fixture_talk: config.float(KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT),
        already_read_fixture_talk: config.float(KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT),
        none_talk_fixture_action: config.float(KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT),
        already_read_when_has_not_read: config
            .float(KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ),
    };
    let weights = LotteryWeights {
        talk1: config.float(KEY_NPC_LOTTERY_TALK1_WIGHT),
        talk2: config.float(KEY_NPC_LOTTERY_TALK2_WIGHT),
        talk3: config.float(KEY_NPC_LOTTERY_TALK3_WIGHT),
        talk4: config.float(KEY_NPC_LOTTERY_TALK4_WIGHT),
    };
    // The landing identity test reads the attachment table: wait for it
    // while anchored fixtures stand, so a table still loading never reads as
    // a fixture without action points.
    if attach.is_none() && placements.fixture_ids().iter().any(|id| *id != 0) {
        return;
    }
    // Branch A's input: any unread row in the whole client talk list.
    let yet_unread_available = talk_list.any_unread();
    // The avatar store's NPC list as the talk lotteries read it.
    let mut views: Vec<npc_talk_lottery::NpcView> = npcs
        .iter()
        .map(
            |(_, unit, _, _, slot, mind, _, _, _, _, _, _, actions, _)| npc_talk_lottery::NpcView {
                unit: unit.0,
                site_type: catalog.site_type_value(&actions.site_type),
                previous_talk_id: slot.previous_id().unwrap_or(0),
                talk_type: slot.kind(),
                objective: mind.current,
                state: actions.current as u8,
                since_initialized: actions.since_initialized,
            },
        )
        .collect();
    // The placed fixtures, built at the first talk factory of the frame.
    let mut placed: Option<Vec<npc_talk_lottery::PlacedFixture>> = None;

    // The avatar list's order; each character, once past its turn, is read
    // back from its live components before the next one runs.
    let order: Vec<Entity> = snaps.iter().map(|snap| snap.entity).collect();
    let mut passed: Option<Entity> = None;
    let mut emitted_before_turn = 0u64;
    'npc: for &entity in &order {
        if let Some(previous) = passed.take() {
            let decided = DECISIONS_EMITTED.load(std::sync::atomic::Ordering::Relaxed)
                != emitted_before_turn;
            if let Ok((_, unit, _, _, slot, mind, _, _, state, target, _, _, actions, _)) =
                npcs.get(previous)
            {
                if let Some(snap) = snaps.iter_mut().find(|snap| snap.entity == previous) {
                    snap.target_fixture = slot.current.as_ref().and_then(|data| data.target_fixture);
                    snap.site_type = actions.site_type.clone();
                    snap.talk_target = prepared_social_target(&slot);
                    snap.state = actions.current as u8;
                    snap.cancel_reports = mind.cancel_reports();
                    snap.position = state.0.position;
                    snap.destination = if mind.executing {
                        target.0
                    } else {
                        state.0.position
                    };
                }
                if let Some(view) = views.iter_mut().find(|view| view.unit == unit.0) {
                    view.site_type = catalog.site_type_value(&actions.site_type);
                    view.previous_talk_id = slot.previous_id().unwrap_or(0);
                    view.talk_type = slot.kind();
                    view.objective = mind.current;
                    view.state = actions.current as u8;
                    view.since_initialized = actions.since_initialized;
                }
                if decided {
                    info!(
                        "[npc-member-view] {}",
                        serde_json::json!({
                            "v": 1,
                            "unit": unit.0,
                            "t": time.elapsed_secs_f64(),
                            "site": actions.site_type,
                            "talk_target": prepared_social_target(&slot),
                            "executing": mind.executing,
                            "position": state.0.position,
                            "destination": if mind.executing { target.0 } else { state.0.position },
                        })
                    );
                }
            }
        }
        passed = Some(entity);
        emitted_before_turn = DECISIONS_EMITTED.load(std::sync::atomic::Ordering::Relaxed);
        let Ok((
            _,
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
        )) = npcs.get_mut(entity)
        else {
            continue;
        };
        if !actions.ready() {
            continue;
        }
        // A member whose own site is not the selected site is not on it (see
        // `npc::residency`), even on the frame before its suspension lands.
        if selection
            .as_deref()
            .is_some_and(|selection| selection.site_type() != actions.site_type)
        {
            continue;
        }
        if mind.ai_stopped.is_some() {
            continue;
        }
        // The activity owner consumes arrival and completion separately. A
        // route reaching its last corner must not skip the furniture body.
        if fixture_activities.owns_actor(entity) {
            continue;
        }
        // A group talk (types 3 and 6) drives its main and the members'
        // sub objectives.
        if groups.owns(entity) {
            continue;
        }
        // IsTalking: the talk state's enter sets it and its exit clears it.
        let talking = actions.current == crate::npc::NpcAction::Talk;
        // The objective body's waits are polled on every frame after the one
        // that created them, talking or not.
        if let Some(wait) = mind.body {
            match wait {
                BodyWait::TweetDone { since, step } => {
                    if frame != since && actions.tweet_state == crate::npc_state::TweetState::Done
                    {
                        mind.body = Some(BodyWait::NotTalking { since: frame, step });
                    }
                }
                BodyWait::NotTalking { since, step } => {
                    if frame != since && !talking {
                        mind.body = None;
                        info!(
                            "[npc unit={}] frame={frame} tweet wait ended; call=TryRotateBeforeRotation",
                            unit.0
                        );
                        let tweeting = step == TweetStep::CommonFixture
                            && run_pre_action(unit.0, frame, &slot, &mut mind, &mut actions, &mut rest);
                        if !tweeting {
                            // The pre-action's timeline: data from the general
                            // factory carries no fixture timeline.
                            finish_objective(unit.0, &mut mind, &mut slot, frame, false);
                        }
                    }
                }
                BodyWait::FirstTalk { since } => {
                    if frame != since && actions.first_talk_complete {
                        info!(
                            "[npc unit={}] frame={frame} greeting objective ended: first talk complete",
                            unit.0
                        );
                        finish_objective(unit.0, &mut mind, &mut slot, frame, false);
                    }
                }
                BodyWait::Stacked { since, mut delay } => {
                    if frame != since {
                        if delay.advance(dt) {
                            // The move fails: the sub members (none for the
                            // single-character talks) stop and the objective
                            // ends, keeping its talk data.
                            info!(
                                "[npc unit={}] frame={frame} stacked move: 1.0 s delay done, objective ends without arrival",
                                unit.0
                            );
                            finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                        } else {
                            mind.body = Some(BodyWait::Stacked { since, delay });
                        }
                    }
                }
            }
        }
        // RestObjective's delay runs before its WaitWhile(Talk): talking
        // does not hold the delay.
        if let Some(RestPhase::Delay { since, mut delay }) = mind.rest {
            if frame != since {
                mind.rest = Some(if delay.advance(dt) {
                    RestPhase::WaitTalk { since: frame }
                } else {
                    RestPhase::Delay { since, delay }
                });
            }
        }
        if talk_hold.is_some() || talking {
            continue;
        }
        if let Some(cancelled) = mind.edit_rest_after.take() {
            // Respect a later owner that installed another objective during
            // the hidden interval. The Talk/hold guards above also still apply.
            if !mind.executing && mind.current == Some(cancelled) {
                finish_objective(unit.0, &mut mind, &mut slot, frame, false);
            }
        }
        // TryCancelIfCharacterOverlap is the presenter's call 8
        // (npc_presenter::try_cancel_if_overlap), after this loop.
        // 执行中且路线已尽（推进系统末站驻留尽的收场位）：目标收场。路点
        // 中途的驻留（Some）不收场——目标还没走完，不换乘。
        let outcome = if mind.executing && mind.body.is_none() {
            route.outcome.take()
        } else {
            None
        };
        if outcome == Some(crate::npc::RouteOutcome::Stopped) {
            if route.take_stalled() {
                // Source Stacked: the main character stops, then a scaled
                // 1.0 s delay, then the move fails. A talk objective raises
                // no ImmediatelyExecuteNextObjective, so the next TryRest
                // runs in full.
                let delay = objective::DelayPromise::from_milliseconds(1000)
                    .expect("one second is a valid delay");
                mind.body = Some(BodyWait::Stacked { since: frame, delay });
                info!(
                    "[npc unit={}] frame={frame} 移动按卡住结束（路线末端距目标超出完成容差）：停下，1.0s 延迟后目标失败",
                    unit.0
                );
                continue;
            }
            // Every other stop is the move's failure: MoveAsync returns false
            // (a missing route and a fixture in use stop the character first;
            // the other statuses return without a stop) and the objective
            // completes without OnArrive. The loop then yields and runs
            // TryRest: a talk objective raises no
            // ImmediatelyExecuteNextObjective, so the Rest runs in full; a
            // no-talk objective's failed move owes its one
            // ForceUpdateObjective first. The AI content is kept.
            info!(
                "[npc unit={}] frame={frame} move failed without arrival: objective {:?} ends, then Yield and TryRest",
                unit.0, mind.current
            );
            mind.rest = None;
            finish_objective(unit.0, &mut mind, &mut slot, frame, true);
            continue;
        }
        if outcome == Some(crate::npc::RouteOutcome::Arrived) {
            if mind.current == Some(ObjectiveType::Talk) {
                on_talk_arrived(unit.0, frame, &mut slot, &mut mind, &mut actions, &mut rest);
            } else {
                finish_objective(unit.0, &mut mind, &mut slot, frame, false);
            }
        }
        // The AI loop: after an objective ends, one Yield frame, then
        // TryRest. The flag skips the Rest once; otherwise the Rest objective
        // runs its delay and then waits while the character talks. The
        // ForceUpdateObjective calls a no-talk end owes come first.
        if mind.force_updates == 0 {
            if let Some(since) = mind.yield_since {
                if frame == since {
                    continue;
                }
                mind.yield_since = None;
                if mind.skip_next_rest {
                    mind.skip_next_rest = false;
                    info!("[npc unit={}] frame={frame} TryRest: flag set, rest skipped", unit.0);
                } else {
                    let milliseconds = objective::rest_objective_milliseconds(pause_seconds.0);
                    match objective::DelayPromise::from_milliseconds(milliseconds) {
                        Ok(delay) => {
                            mind.rest = Some(RestPhase::Delay { since: frame, delay });
                            // RestAsync makes the Rest objective the current one.
                            mind.begin_objective(ObjectiveType::Rest);
                            mind.rest_revision = mind.rest_revision.wrapping_add(1);
                            actions.begin_objective_rest(&mut rest, mind.rest_revision);
                            info!(
                                "[npc unit={}] frame={frame} rest: {milliseconds} ms (delay {:#x})",
                                unit.0,
                                delay.delay().to_bits()
                            );
                        }
                        Err(milliseconds) => {
                            let reason = format!(
                                "rest delay of {milliseconds} ms is out of range (the delay raises)"
                            );
                            error!("[npc unit={}] {reason}; this character's AI loop ends", unit.0);
                            mind.ai_stopped = Some(reason);
                            continue;
                        }
                    }
                }
            }
        }
        if let Some(RestPhase::WaitTalk { since }) = mind.rest {
            if frame != since {
                // The Rest objective's end: Idle, then CompleteRest; TryRest
                // clears the flag after it.
                mind.rest = None;
                mind.skip_next_rest = false;
                actions.change(crate::npc::NpcAction::Idle, &mut rest);
                info!("[npc unit={}] frame={frame} rest ended", unit.0);
            }
        }
        if mind.rest.is_some() || mind.yield_since.is_some() {
            continue;
        }
        if mind.executing {
            continue; // 在走向目的地的路上，无事可判
        }
        // The source AI starts only after its site and master data exist.
        // Hold in front of every draw while this host is still installing
        // them (as CanRunningAI holds the loop) instead of drawing and
        // discarding a decision each frame. Gaps inside a built snapshot are
        // not loading: they only keep the no-talk lane from selecting.
        if let Err(reason) = fixture_activities.loading(epoch.0, &actions.site_type) {
            fixture_activities.report_pending(entity, unit.0, reason);
            continue;
        }
        // The start-up flag (the AI's set-up raises it before its first
        // TryRest) is consumed here on the first decision; after an objective
        // the Yield step above has already consumed it.
        mind.skip_next_rest = false;
        'cascade: loop {
            // A no-talk end owes ForceUpdateObjective calls: each is a reset and
            // UpdateObjective's reset and row-1 cascade, with no ladder draw and no
            // departure. All of them are made on this pass; the loop's Yield
            // before TryRest follows the objective's own end.
            let force_update = mind.force_updates > 0;

            // —— 决策梯 ——
            // One decision draws on copies of the member RNG and of the
            // sequence-pick seeder. The copies replace the originals only when
            // the decision commits; a factory that cannot evaluate yet leaves
            // both sequences untouched, so the retry makes exactly the draws the
            // source makes once per DecideObjective.
            let mut trial = (*rng).clone();
            let mut trial_seeder = seeder.seeder().clone();
            let calls_before = rng.calls();
            let view = objective::LadderView {
                talk_type: slot.kind(),
                photo_shot: false,
                interrupt: slot.interrupt,
                current: mind.current,
                first_talk_complete: actions.first_talk_complete,
            };
            let mut percent = PercentSource::new(&mut trial);
            let decision = if force_update {
                objective::Decision::RefuelAndTalk
            } else {
                objective::decide(&view, &percents, yet_unread_available, &mut percent)
            };
            // 账目串在这取走（借用就地结束）：后面的目的地链还要借 rng。
            let draws_word = percent.account();
            let mut record = DecisionRecord::new(
                unit.0,
                &actions.site_type,
                time.elapsed_secs_f64(),
                match decision {
                    _ if force_update => "force_update_objective",
                    objective::Decision::RefuelAndTalk => "empty_talk_data",
                    objective::Decision::Greeting => "greeting",
                    objective::Decision::NoneTalk => "none_talk_data",
                    objective::Decision::Interrupt { .. } => "interrupt",
                    _ => "select_objective",
                },
                &percents,
                &weights,
                yet_unread_available,
                talk_list,
                &percent.draws,
            );
            let decision_route = match decision {
                // Row 1: ForceUpdateObjective resets, then UpdateObjective resets
                // again before it builds the data, so Previous ends empty.
                objective::Decision::RefuelAndTalk => {
                    slot.reset_ai_talk_data();
                    slot.reset_ai_talk_data();
                    DecisionRoute::Factory(TalkFactory::CreateAiTalkData)
                }
                objective::Decision::Select(select::Objective::Talk(lane)) => {
                    // Each of the four talk factories resets before its lottery.
                    slot.reset_ai_talk_data();
                    DecisionRoute::Factory(TalkFactory::of_lane(lane))
                }
                // The Talk interrupt: a new talk objective on the same data; the
                // marker's interrupt flag is then cleared.
                objective::Decision::Interrupt {
                    dispatch:
                        objective::InterruptDispatch::Direct(
                            kind @ (ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub
                            | ObjectiveType::SubCharacterFixtureAction),
                        ),
                } => {
                    // A drafted member's sub objective on the data the main
                    // gave it; the marker's interrupt flag is then cleared.
                    if let Some(marker) = slot.interrupt.as_mut() {
                        marker.can_interrupt = false;
                    }
                    DecisionRoute::SubObjective(kind)
                }
                objective::Decision::Interrupt {
                    dispatch: objective::InterruptDispatch::Direct(ObjectiveType::Talk),
                } => {
                    if let Some(marker) = slot.interrupt.as_mut() {
                        marker.can_interrupt = false;
                    }
                    DecisionRoute::SameTalkData
                }
                objective::Decision::NoneTalk
                | objective::Decision::Select(select::Objective::NoneTalk) => DecisionRoute::NoneTalk,
                objective::Decision::Greeting => DecisionRoute::Greeting,
                objective::Decision::ChangeSite => DecisionRoute::ChangeSite,
                objective::Decision::Interrupt {
                    dispatch: objective::InterruptDispatch::Direct(ObjectiveType::ChangeSite),
                } => {
                    if let Some(marker) = slot.interrupt.as_mut() {
                        marker.can_interrupt = false;
                    }
                    DecisionRoute::ChangeSite
                }
                other => unreachable!(
                    "决策梯落到了产品不可达的档（{other:?}）：快照里摆拍/保持档的输入按构造恒假，\
                     打断标记只写对话一种，槽位与当前目标的值域里没有它们"
                ),
            };
            let objective_type = match decision_route {
                DecisionRoute::NoneTalk => ObjectiveType::NoneTalk,
                DecisionRoute::Greeting => ObjectiveType::Greeting,
                DecisionRoute::ChangeSite => ObjectiveType::ChangeSite,
                DecisionRoute::SubObjective(kind) => kind,
                _ => ObjectiveType::Talk,
            };
            record.set(
                "objective",
                match objective_type {
                    ObjectiveType::NoneTalk => "none_talk",
                    ObjectiveType::Greeting => "greeting",
                    ObjectiveType::ChangeSite => "change_site",
                    ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub => {
                        "sub_while_doing_wait"
                    }
                    ObjectiveType::SubCharacterFixtureAction => "sub_fixture_action",
                    _ => "talk",
                },
            );
            record.set("lane", decision_route.word());
            record.set("phenomenon", catalog.phenomena_id());
            if let DecisionRoute::ChangeSite = decision_route {
                // The change-site objective draws nothing; its body runs in
                // the change-site state's steps.
                let calls = trial.calls() - calls_before;
                *rng = trial;
                *seeder.seeder() = trial_seeder;
                record.set("path", "change_site_objective");
                record.emit("change_site", calls);
                fixture_activities.clear_pending(entity);
                mind.begin_objective(ObjectiveType::ChangeSite);
                mind.executing = true;
                change_site.decided(entity, unit.0, frame);
                continue 'npc;
            }
            if let DecisionRoute::Greeting = decision_route {
                // The greeting objective: the greeting state, then a wait until
                // the first talk is complete. It draws nothing.
                let calls = trial.calls() - calls_before;
                *rng = trial;
                *seeder.seeder() = trial_seeder;
                record.set("path", "greeting_objective");
                if let Some(content) = slot.current.as_ref().and_then(|data| data.content.as_ref()) {
                    record.set("talk_id", content.master_id);
                }
                record.emit("greeting", calls);
                fixture_activities.clear_pending(entity);
                mind.begin_objective(ObjectiveType::Greeting);
                mind.executing = true;
                actions.change(crate::npc::NpcAction::Greeting, &mut rest);
                mind.body = Some(BodyWait::FirstTalk { since: frame });
                info!("[npc unit={}] frame={frame} greeting objective: change to Greeting", unit.0);
                continue 'npc;
            }

            // The talk factory. It leaves a general talk to build, or empty data
            // (the talk objective then completes at once), or a source exception
            // (this character's AI loop ends), or a host gap (reported by name;
            // the decision ends as empty data does).
            let mut general_talk: Option<crate::player_talk::ResolvedTalk> = None;
            let mut fixture_talk: Option<PreparedFixtureTalk> = None;
            let mut null_talk: Option<String> = None;
            let mut halt: Option<npc_talk_lottery::Halt> = None;
            match decision_route {
                DecisionRoute::Factory(factory) => {
                    record.set("path", "talk_factory");
                    let own = views
                        .iter()
                        .position(|view| view.unit == unit.0)
                        .expect("every deciding NPC is in the avatar list");
                    views[own].previous_talk_id = slot.previous_id().unwrap_or(0);
                    views[own].talk_type = slot.kind();
                    views[own].objective = mind.current;
                    views[own].since_initialized = actions.since_initialized;
                    let seeker = views[own].clone();
                    // The avatar list as the lotteries read it (enum values as
                    // numbers), so a replay evaluates the member gates.
                    record.set(
                        "npc_views",
                        views
                            .iter()
                            .map(|view| {
                                serde_json::json!({
                                    "unit": view.unit,
                                    "site_type": view.site_type,
                                    "previous_talk_id": view.previous_talk_id,
                                    "talk_type": view.talk_type.map(|kind| kind as u8),
                                    "objective": view.objective.map(|kind| kind as u8),
                                    "state": view.state,
                                    "since_initialized": view.since_initialized,
                                })
                            })
                            .collect::<Vec<_>>(),
                    );
                    let fixtures: &[npc_talk_lottery::PlacedFixture] = placed.get_or_insert_with(|| {
                        let fixtures = fixture_activities.talk_lottery_fixtures(
                            &placements,
                            selection
                                .as_deref()
                                .and_then(|site| catalog.site_type_value(site.site_type())),
                        );
                        // The placed fixtures the talk gates read, logged when
                        // they change, so a replay evaluates the same gates.
                        let line = serde_json::json!({
                            "v": 1,
                            "fixtures": fixtures
                                .iter()
                                .map(|fixture| serde_json::json!({
                                    "uid": fixture.uid,
                                    "fixture_id": fixture.fixture_id,
                                    "site_type": fixture.located_site_type,
                                    "is_gate": fixture.is_gate,
                                    "motion_overlap": match &fixture.motion_overlap {
                                        Ok(overlap) => serde_json::json!(overlap),
                                        Err(reason) => serde_json::json!({ "error": reason }),
                                    },
                                }))
                                .collect::<Vec<_>>(),
                        })
                        .to_string();
                        if *last_placed != line {
                            info!("[npc-placed] {line}");
                            *last_placed = line;
                        }
                        fixtures
                    });
                    let admissions = std::cell::RefCell::new(Vec::new());
                    let (result, factory_draws, lottery_rows, fixture_walks, drafts) = {
                        let tables = fixture_activities
                            .tables()
                            .expect("the decision holds until the master tables are installed");
                        let site_type_of_site = |site: i32| catalog.site_type_of_site(site);
                        // The playable-fixture gate's admissibility pair for this
                        // character, through the host's fixture admission; every
                        // evaluation goes into the record.
                        let other_targets: Vec<(Entity, Option<Entity>)> = snaps
                            .iter()
                            .map(|snap| (snap.entity, snap.target_fixture))
                            .collect();
                        let position = state.0.position;
                        let unmovable: &[String] = &mind.unmovable_fixtures;
                        let admissible = |talk: i32, fixture: &npc_talk_lottery::PlacedFixture| {
                            let result = fixture_activities.talk_fixture_admissible(
                                entity,
                                position,
                                talk,
                                &fixture.uid,
                                &placements,
                                &other_targets,
                                unmovable,
                                face,
                            );
                            admissions.borrow_mut().push(serde_json::json!({
                                "talk_id": talk,
                                "uid": fixture.uid,
                                "result": match &result {
                                    Ok(value) => serde_json::json!(value),
                                    Err(reason) => serde_json::json!({ "gap": reason }),
                                },
                            }));
                            result
                        };
                        let geometry =
                            |uid: &str| fixture_activities.talk_fixture_geometry(uid, &placements);
                        let host = TalkFixtureHost {
                            // FindNPC(unit).Position: the deciding character's own
                            // live position, the others' as last read.
                            positions: snaps
                                .iter()
                                .map(|snap| {
                                    let own = snap.entity == entity;
                                    (snap.unit, if own { position } else { snap.position })
                                })
                                .collect(),
                            geometry: &geometry,
                            resolved: Default::default(),
                            face,
                            move_offset: config.float(KEY_CHARACTER_FIXTURE_MOVE_OFFSET),
                            walks: Default::default(),
                        };
                        let scene = npc_talk_lottery::LotteryScene {
                            tables,
                            talk_list: talk_list.talk_list(),
                            phenomena_id: catalog.phenomena_id(),
                            site_type_of_site: &site_type_of_site,
                            npcs: &views,
                            fixtures,
                            weights,
                            fixture_gates: Some(npc_talk_lottery::FixtureGateInputs {
                                gate_action_elapsed_seconds: config
                                    .int(KEY_CHARACTER_GATE_ACTION_ELAPSED_TIME),
                                admissible: &admissible,
                            }),
                            fixture_host: Some(&host),
                            together: Some(&together.0),
                        };
                        let engine = std::cell::RefCell::new(&mut trial);
                        let mut engine_int = |len: usize| {
                            let mut guard = engine.borrow_mut();
                            engine_int_draw(&mut **guard, len)
                        };
                        let mut engine_float = |total: f32| {
                            let mut guard = engine.borrow_mut();
                            engine_float_draw(&mut **guard, total)
                        };
                        let mut sequence_pick = |count: usize| trial_seeder.pick(count);
                        let mut draws = npc_talk_lottery::Draws {
                            engine_int: &mut engine_int,
                            engine_float: &mut engine_float,
                            sequence_pick: &mut sequence_pick,
                            record: Vec::new(),
                        };
                        let mut result = factory.run(&scene, &seeker, &mut draws);
                        // PassTalkDataToSubCharacters runs at the start of the
                        // talk objective, before any other draw: the members'
                        // data is built here, in the decision's draw order.
                        let drafts = match &result {
                            // A ForceUpdateObjective builds data only; no talk
                            // objective runs.
                            Ok(npc_talk_lottery::TalkPlan::Fixture(data))
                                if !force_update
                                    && matches!(
                                        data.kind,
                                        TalkType::CommunicationWhileDoingWait
                                            | TalkType::MultipleCharacterFixture
                                    ) =>
                            {
                                member_drafts(&scene, &mut draws, &snaps, &views, unit.0, data)
                            }
                            _ => Ok(Vec::new()),
                        };
                        let drafts = match drafts {
                            Ok(drafts) => drafts,
                            Err(stop) => {
                                result = Err(stop);
                                Vec::new()
                            }
                        };
                        let counts = npc_talk_lottery::list_counts(&scene);
                        (result, draws.record, counts, host.walks.into_inner(), drafts)
                    };
                    if !drafts.is_empty() {
                        record.set(
                            "member_drafts",
                            drafts
                                .iter()
                                .map(|draft| {
                                    serde_json::json!({
                                        "unit": draft.unit,
                                        "state": draft.state,
                                        "drafted": draft.drafted,
                                        "cancel": draft.cancel,
                                        "target": draft.data.as_ref().map(|data| data.target_position),
                                        "steps": draft.steps,
                                    })
                                })
                                .collect::<Vec<_>>(),
                        );
                    }
                    record.set("fixture_lottery_rows", lottery_rows);
                    if !fixture_walks.is_empty() {
                        record.set("fixture_walks", fixture_walks);
                    }
                    record.set("fixture_admissible", admissions.into_inner());
                    record.extend_draws(factory_draws);
                    match result {
                        Ok(npc_talk_lottery::TalkPlan::General { talk_id, interrupt }) => {
                            record.set("talk_id", talk_id);
                            match catalog.resolve(talk_id) {
                                Some(resolved) => {
                                    if interrupt {
                                        slot.interrupt = Some(TALK_INTERRUPT);
                                    }
                                    general_talk = Some(resolved);
                                }
                                None => {
                                    halt = Some(npc_talk_lottery::Halt::Gap(format!(
                                        "general talk {talk_id} is not in the loaded talk scripts"
                                    )));
                                }
                            }
                        }
                        Ok(npc_talk_lottery::TalkPlan::Null { reason, interrupt }) => {
                            if interrupt {
                                slot.interrupt = Some(TALK_INTERRUPT);
                            }
                            null_talk = Some(reason);
                        }
                        Ok(npc_talk_lottery::TalkPlan::Fixture(data)) => {
                            record.set("talk_id", data.talk_id);
                            record.set(
                                "fixture_talk",
                                serde_json::json!({
                                    "kind": data.kind as u8,
                                    "fixture": data.fixture,
                                    "timeline": data.timeline,
                                    "target": data.target_position,
                                    "target_found": data.target_found,
                                    "rotation": data.rotation,
                                    "members": data.members,
                                }),
                            );
                            match data.kind {
                                _ => match catalog.resolve(data.talk_id) {
                                    Some(resolved) => {
                                        let prepared = fixture_activities
                                            .talk_fixture_geometry(&data.fixture, &placements)
                                            .and_then(|geometry| {
                                                let timeline = match data.timeline {
                                                    Some(_) if data.kind == TalkType::SingleCharacterFixture => {
                                                        let pre_action = fixture_activities
                                                            .tables()
                                                            .and_then(|tables| tables.pre_action_of(data.talk_id))
                                                            .map(|pre| pre.id)
                                                            .ok_or("the fixture talk has no pre-action row")?;
                                                        Some(fixture_activities.talk_timeline_selection(
                                                            entity,
                                                            unit.0,
                                                            epoch.0,
                                                            &placements,
                                                            &data,
                                                            pre_action,
                                                            resolved.pre_action(),
                                                        )?)
                                                    }
                                                    _ => None,
                                                };
                                                let names = geometry
                                                    .locators
                                                    .into_iter()
                                                    .map(|(name, _)| name)
                                                    .collect::<Vec<_>>();
                                                Ok((geometry.entity, timeline, names))
                                            });
                                        let prepared = prepared.and_then(|(fixture, timeline, names)| {
                                            let tables = fixture_activities
                                                .tables()
                                                .ok_or("the talk tables are not installed")?;
                                            let is_general = npc_talk_lottery::is_general_talk(tables, data.talk_id)?;
                                            Ok((fixture, timeline, names, is_general))
                                        });
                                        match prepared {
                                            Ok((fixture, timeline, names, is_general)) => {
                                                let content = crate::npc_objective::TalkContent {
                                                    is_general: Some(is_general),
                                                    ..resolved.content()
                                                };
                                                // The members' data carries this
                                                // master, fixture and pre-action.
                                                let drafts = drafts
                                                    .iter()
                                                    .map(|draft| crate::npc_fixture_talk::MemberDraft {
                                                        entity: draft.entity,
                                                        unit: draft.unit,
                                                        drafted: draft.drafted,
                                                        cancel: draft.cancel,
                                                        data: draft.data.as_ref().map(|member| AiTalkData {
                                                            kind: member.kind,
                                                            content: Some(content.clone()),
                                                            target_fixture: Some(fixture),
                                                            target_position: member.target_position,
                                                            // A while-doing-wait member is its own
                                                            // data's main; a multiple-character
                                                            // member keeps the main.
                                                            main_character: if member.kind
                                                                == TalkType::CommunicationWhileDoingWait
                                                            {
                                                                draft.unit
                                                            } else {
                                                                unit.0
                                                            },
                                                            characters: member.members.clone(),
                                                            pre_action: Some(resolved.pre_action()),
                                                            locate: member.locate.clone(),
                                                            pending_factory: None,
                                                        }),
                                                    })
                                                    .collect();
                                                fixture_talk = Some(PreparedFixtureTalk {
                                                    data,
                                                    resolved,
                                                    content,
                                                    fixture,
                                                    timeline,
                                                    names,
                                                    drafts,
                                                });
                                            }
                                            Err(reason) => {
                                                halt = Some(npc_talk_lottery::Halt::Gap(format!(
                                                    "fixture talk {}: {reason}",
                                                    data.talk_id
                                                )));
                                            }
                                        }
                                    }
                                    None => {
                                        halt = Some(npc_talk_lottery::Halt::Gap(format!(
                                            "fixture talk {} is not in the loaded talk scripts",
                                            data.talk_id
                                        )));
                                    }
                                },
                            }
                        }
                        Err(stop) => halt = Some(stop),
                    }
                }
                DecisionRoute::SameTalkData => {
                    record.set("path", "same_talk_data");
                    match slot.current.as_ref() {
                        Some(data) if data.target_fixture.is_none() => {
                            if let Some(content) = &data.content {
                                record.set("talk_id", content.master_id);
                            }
                        }
                        _ => {
                            halt = Some(npc_talk_lottery::Halt::Gap(
                                "the talk interrupt re-runs fixture-targeted data this host does not build"
                                    .into(),
                            ));
                        }
                    }
                }
                DecisionRoute::SubObjective(_) => {
                    record.set("path", "sub_objective");
                    if let Some(content) = slot.current.as_ref().and_then(|data| data.content.as_ref()) {
                        record.set("talk_id", content.master_id);
                    }
                }
                DecisionRoute::NoneTalk => {}
                DecisionRoute::Greeting => unreachable!("the greeting objective returned above"),
                DecisionRoute::ChangeSite => unreachable!("the change-site objective returned above"),
            }
            match halt {
                Some(npc_talk_lottery::Halt::Fault(reason)) => {
                    // The source's AI loop catches only cancellation and its own
                    // cannot-decide exception; any other exception ends this
                    // character's AI.
                    let calls = trial.calls() - calls_before;
                    *rng = trial;
                    *seeder.seeder() = trial_seeder;
                    record.set("path", "source_exception");
                    record.set("reason", reason.as_str());
                    record.emit("ai_stopped", calls);
                    fixture_activities.clear_pending(entity);
                    error!(
                        "[npc unit={}] 目标裁决：{draws_word} → 源异常，该角色 AI 循环终止：{reason}",
                        unit.0
                    );
                    mind.ai_stopped = Some(reason);
                    continue 'npc;
                }
                Some(npc_talk_lottery::Halt::Gap(reason)) => {
                    record.set("path", "host_data_gap");
                    if reported_gaps.insert(reason.clone()) {
                        warn!(
                            "[npc-talk] talk factory not evaluated (first met by unit={}): {reason}",
                            unit.0
                        );
                    }
                    null_talk = Some(format!("host gap: {reason}"));
                }
                None => {}
            }
            if let (Some(reason), true) = (null_talk.as_ref(), force_update) {
                let calls = trial.calls() - calls_before;
                *rng = trial;
                *seeder.seeder() = trial_seeder;
                record.set("reason", reason.as_str());
                record.emit("empty_talk_data", calls);
                fixture_activities.clear_pending(entity);
                mind.force_updates -= 1;
                info!(
                    "[npc unit={}] frame={frame} ForceUpdateObjective: {} empty ({reason})",
                    unit.0,
                    decision_route.word()
                );
                if mind.force_updates > 0 {
                    continue 'cascade;
                }
                mind.yield_since = Some(mind.force_update_end.max(frame));
                continue 'npc;
            }
            if let Some(reason) = null_talk {
                let calls = trial.calls() - calls_before;
                *rng = trial;
                *seeder.seeder() = trial_seeder;
                record.set("reason", reason.as_str());
                record.emit("empty_talk_data", calls);
                fixture_activities.clear_pending(entity);
                mind.begin_objective(ObjectiveType::Talk);
                finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                info!(
                    "[npc unit={}] 目标裁决：{draws_word} → {} 空数据（{reason}），对话目标即刻结束，进入停顿",
                    unit.0,
                    decision_route.word()
                );
                continue 'npc;
            }

            // —— 目的地解算 ——
            let mut probe = FaceProbe { face };
            let world_of = |cell: Cell| face.world_of(cell);
            let from = route.navigation_origin(state.0.position, walk_face);
            let detail;
            let mut fixture_selection = None;
            // Placement UID of the talk data's TargetFixture: MoveAsync gates such
            // a target with IfMoveTargetFixtureActionPosition, others with
            // IfMoveTargetPosition.
            let mut gate_fixture: Option<String> = None;
            let mut none_talk_null = false;
            let mut none_talk_gap = false;
            // Row 5 on no-talk data a forced objective set: the no-talk
            // objective runs on that data as it is (no factory, no reset).
            let mut forced_data = false;
            let destination = if let Some(data) = fixture_talk.as_ref().map(|prepared| &prepared.data) {
                // MoveAsync goes to the data's target position; a talk data
                // TargetFixture takes IfMoveTargetFixtureActionPosition.
                detail = format!("fixture talk {} on {}", data.talk_id, data.fixture);
                gate_fixture = Some(data.fixture.clone());
                Some(data.target_position)
            } else {
                match decision_route {
                    DecisionRoute::SameTalkData => {
                        let data = slot
                            .current
                            .as_ref()
                            .expect("the interrupt row re-runs existing talk data");
                        detail = "同一对话数据".to_owned();
                        Some(data.target_position)
                    }
                    DecisionRoute::SubObjective(_) => {
                        let data = slot
                            .current
                            .as_ref()
                            .expect("the interrupt row runs on existing talk data");
                        detail = "sub objective to its data target".to_owned();
                        gate_fixture = data
                            .target_fixture
                            .and_then(|fixture| fixture_activities.uid_of(fixture));
                        Some(data.target_position)
                    }
                    DecisionRoute::Factory(_) => {
                        // CreateGeneralTalkData: the spot near another character,
                        // else a random floor position. Both chains start from the
                        // character's own position, not from its navigation origin.
                        let position = state.0.position;
                        let near: Vec<(u32, objective::SocialCandidate)> = snaps
                            .iter()
                            .filter_map(|snap| {
                                snap.social_candidate(unit.0, &actions.site_type)
                                    .map(|candidate| (snap.unit, candidate))
                            })
                            .collect();
                        // The candidates as this decision read them, so a replay
                        // can check them against the others' live state.
                        record.set(
                            "near_candidates",
                            near.iter()
                                .map(|(other, candidate)| {
                                    serde_json::json!({
                                        "unit": other,
                                        "position": candidate.position,
                                        "destination": candidate.destination,
                                        "talk_target": candidate.talk_target,
                                    })
                                })
                                .collect::<Vec<_>>(),
                        );
                        let candidates: Vec<objective::SocialCandidate> =
                            near.into_iter().map(|(_, candidate)| candidate).collect();
                        let npc_positions: Vec<[f32; 3]> =
                            snaps.iter().map(|snap| snap.position).collect();
                        let mut uniform = UniformSource::new(&mut trial);
                        let social = objective::social_target(
                            &candidates,
                            position,
                            &npc_positions,
                            &mut probe,
                            &mut uniform,
                        );
                        record.uniform("near_character_pick", DRAW_ENGINE_INT_RANGE, &uniform.draws);
                        if let Some(spot) = social {
                            detail = format!(
                                "社交链 池抽 {}（候选 {} 员）",
                                uniform.account(),
                                candidates.len()
                            );
                            Some(spot)
                        } else {
                            // The move range follows the site type of this
                            // character's own site.
                            let in_room = catalog
                                .site_type_value(&actions.site_type)
                                .is_some_and(objective::uses_in_room_move_range);
                            let (wander_min, wander_max) = if in_room {
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
                            // The source stores the grid origin as signed bytes; the
                            // ring filter wraps each axis difference to a signed byte,
                            // so the unwrapped cell gives the same ring.
                            let origin = cell_of(position[0], position[2]);
                            let eligible: Vec<Cell> = face
                                .walkable()
                                .iter()
                                .copied()
                                .filter(|cell| !occupied.contains(cell))
                                .collect();
                            let mut permute = PermuteSource::new(&mut trial);
                            let wander = objective::wander_target(
                                origin,
                                wander_min,
                                wander_max,
                                &eligible,
                                world_of,
                                &mut permute,
                                &mut probe,
                            );
                            record.keys("floor_position_order", permute.keys);
                            detail = format!(
                                "社交未命中→游走 环 {} 格（键 {}，档 {wander_min}..{wander_max}）",
                                eligible.len(),
                                permute.keys
                            );
                            wander
                        }
                    }
                    DecisionRoute::Greeting => unreachable!("the greeting objective returned above"),
                    DecisionRoute::ChangeSite => unreachable!("the change-site objective returned above"),
                    DecisionRoute::NoneTalk
                        if matches!(decision, objective::Decision::NoneTalk)
                            && forced_none_talk(&mut fixture_activities, entity, &slot) =>
                    {
                        let selected = fixture_activities
                            .take_forced(entity)
                            .expect("the guard found the forced data");
                        record.set("path", "none_talk_existing_data");
                        if let Some(row) = selected.no_talk_row() {
                            record.set("no_talk_row", row);
                        }
                        record.set("timeline", selected.timeline_id());
                        detail = format!(
                            "existing no-talk data on {:?}/{}",
                            selected.target.entity, selected.target.uid
                        );
                        gate_fixture = Some(selected.target.uid.clone());
                        forced_data = true;
                        let position = selected.position;
                        fixture_selection = Some(selected);
                        Some(position)
                    }
                    DecisionRoute::NoneTalk => {
                        record.set("path", "none_talk_factory");
                        let fixture_targets: Vec<(Entity, Option<Entity>)> = snaps
                            .iter()
                            .map(|snap| (snap.entity, snap.target_fixture))
                            .collect();
                        let mut none_talk_draws = crate::npc_fixture_activity::NoneTalkDraws::default();
                        let selected = fixture_activities.select_none_talk(
                            entity,
                            unit.0,
                            &actions.site_type,
                            epoch.0,
                            from,
                            &placements,
                            &fixture_targets,
                            &mind.unmovable_fixtures,
                            face,
                            &mut trial,
                            &mut none_talk_draws,
                        );
                        // CreateNoneTalkData: one fresh sort key per no-talk row
                        // (OrderBy Guid), then on a hit one engine pick of the
                        // timeline in its group.
                        record.keys("none_talk_order", none_talk_draws.keys);
                        if let Some((value, len)) = none_talk_draws.timeline_pick {
                            record.uniform("none_talk_timeline_pick", DRAW_ENGINE_INT_RANGE, &[(value, len)]);
                        }
                        match selected {
                            Ok(Some(selected)) => {
                                detail = format!(
                                    "source no-talk action on {:?}/{}",
                                    selected.target.entity, selected.target.uid
                                );
                                gate_fixture = Some(selected.target.uid.clone());
                                let position = selected.position;
                                fixture_selection = Some(selected);
                                Some(position)
                            }
                            Ok(None) => {
                                detail = "no eligible source no-talk fixture action".into();
                                none_talk_null = true;
                                None
                            }
                            Err(crate::npc_fixture_activity::FactoryIssue::Pending(reason)) => {
                                // Inputs the loop waits for are not installed yet. Drop
                                // the trial draws and retry the same decision later.
                                fixture_activities.report_pending(entity, unit.0, reason);
                                continue 'npc;
                            }
                            Err(crate::npc_fixture_activity::FactoryIssue::Gap(reason)) => {
                                // Host data the factory reads is incomplete and a retry
                                // cannot be relied on to change it; holding could stop
                                // this NPC for good. The cycle ends as the factory's empty
                                // result does; the gap itself is reported once.
                                detail = "host data gap, factory not evaluated".into();
                                record.set("path", "host_data_gap");
                                fixture_activities.report_gap(unit.0, reason);
                                none_talk_gap = true;
                                none_talk_null = true;
                                None
                            }
                        }
                    }
                }
            };
            let calls = trial.calls() - calls_before;
            *rng = trial;
            *seeder.seeder() = trial_seeder;
            fixture_activities.clear_pending(entity);
            if none_talk_null {
                record.emit("empty_talk_data", calls);
                // ForceUpdateNoneTalkObjective resets the AI talk data before
                // CreateNoneTalkData returns null; SelectFixtureTalk then returns a
                // TalkObjective whose null data completes at once. TryRest runs
                // normally and the next decision takes the null-data arm. A host
                // data gap takes the same path without evaluating the factory.
                slot.reset_ai_talk_data();
                mind.begin_objective(ObjectiveType::Talk);
                finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                let outcome_word = if none_talk_gap { "工厂未求值" } else { "工厂空" };
                info!(
                    "[npc unit={}] 目标裁决：{draws_word} → nonetalk {outcome_word}（{detail}），对话目标即刻结束，进入停顿",
                    unit.0
                );
                continue 'npc;
            }

            let target_position = destination.unwrap_or(state.0.position);
            if general_talk.is_some() || fixture_talk.is_some() {
                record.set("talk_target", serde_json::json!(target_position));
            }
            if forced_data {
                // The slot already holds this data.
            } else if let Some(selected) = &fixture_selection {
                // ForceUpdateNoneTalkObjective: Reset, then SetAITalkData.
                slot.reset_ai_talk_data();
                slot.set_current(selected.ai_data());
            } else if let Some(resolved) = &general_talk {
                // CreateCharacterGeneralTalkData: the master, the target
                // position, the pre-action, the main character and the
                // one-member list; the tweet id is the talk's pre-action tweet.
                actions.tweet_id = resolved.pre_action().id;
                slot.set_current(AiTalkData {
                    kind: TalkType::Common,
                    content: Some(resolved.content()),
                    target_fixture: None,
                    target_position,
                    main_character: unit.0,
                    characters: vec![unit.0],
                    pre_action: Some(resolved.pre_action()),
                    locate: None,
                    pending_factory: None,
                });
            } else if let Some(prepared) = &fixture_talk {
                // CreateCharacterTalkData, then SetAITalkData: the master, the
                // fixture, the target position, the pre-action and the
                // factory's character list; the tweet id is the talk's
                // pre-action tweet.
                actions.tweet_id = prepared.resolved.pre_action().id;
                slot.set_current(AiTalkData {
                    kind: prepared.data.kind,
                    content: Some(prepared.content.clone()),
                    target_fixture: Some(prepared.fixture),
                    target_position,
                    main_character: unit.0,
                    characters: prepared.data.members.clone(),
                    pre_action: Some(prepared.resolved.pre_action()),
                    locate: prepared.data.locate.clone(),
                    pending_factory: None,
                });
            }
            if force_update {
                record.emit("force_update_objective", calls);
                mind.force_updates -= 1;
                info!(
                    "[npc unit={}] frame={frame} ForceUpdateObjective: {} -> data kept ({} left)",
                    unit.0,
                    decision_route.word(),
                    mind.force_updates
                );
                if mind.force_updates > 0 {
                    continue 'cascade;
                }
                // The objective's end: on this frame after a failed move, on
                // the next one after its timeline (its wait sees the cancel
                // then). The loop yields after it.
                mind.yield_since = Some(mind.force_update_end.max(frame));
                continue 'npc;
            }
            mind.begin_objective(objective_type);
            let source_word = match decision {
                objective::Decision::RefuelAndTalk => "空槽补位",
                objective::Decision::NoneTalk => "无对话槽",
                objective::Decision::Interrupt { .. } => "对话打断",
                _ => "梯末抽签",
            };
            let objective_word = decision_route.word();
            // The group talk's drafts and claims run after this pass; its
            // gathering runs whatever the main's own move does.
            if let Some(prepared) = fixture_talk.as_mut().filter(|prepared| prepared.is_group()) {
                let pre_action = prepared.resolved.pre_action();
                groups.start(crate::npc_fixture_talk::GroupStart {
                    main: entity,
                    main_unit: unit.0,
                    kind: prepared.data.kind,
                    talk_id: prepared.data.talk_id,
                    fixture: prepared.fixture,
                    fixture_uid: prepared.data.fixture.clone(),
                    members: prepared.data.members.clone(),
                    locate: prepared.data.locate.clone().unwrap_or_default(),
                    timeline: prepared.data.timeline,
                    names: std::mem::take(&mut prepared.names),
                    drafts: std::mem::take(&mut prepared.drafts),
                    tweet: (pre_action.id != 0).then(|| (pre_action.id, pre_action.text.clone())),
                });
            }
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
                    // MoveAsync's gate, once, for the chosen target only. A talk
                    // data TargetFixture takes IfMoveTargetFixtureActionPosition
                    // and a failure enters that placement into this NPC's
                    // UnmovableFixtureList (TryAddUnmovableFixture); any other
                    // target takes IfMoveTargetPosition. Either failure ends the
                    // move as NoneRoute: the objective fails and TryRest runs.
                    let field = &walk_face.field;
                    let from_xz = [from[0], from[2]];
                    let landing_xz = [landing[0], landing[2]];
                    let gate_open = if gate_fixture.is_some() {
                        field.if_move_target_fixture_action_position(from_xz, landing_xz)
                    } else {
                        field.if_move_target_position(from_xz, landing_xz)
                    };
                    if !gate_open {
                        record.emit("move_gate_closed", calls);
                        let unmovable = gate_fixture.filter(|uid| !mind.unmovable_fixtures.contains(uid));
                        if let Some(uid) = &unmovable {
                            mind.unmovable_fixtures.push(uid.clone());
                        }
                        if groups.owns(entity) {
                            // MoveAsync ends NoneRoute; the gathering still
                            // runs to its end first.
                            groups.main_departure_failed(entity);
                            continue 'npc;
                        }
                        if let DecisionRoute::SubObjective(kind) = decision_route {
                            sub_objective_move_failed(kind, &mut mind, &mut actions, &mut rest, frame);
                            continue 'cascade;
                        }
                        finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                        info!(
                            "[npc unit={}] 目标裁决 {source_word}：{draws_word} → {objective_word}（{detail}）→ 落点 ({:.2},{:.2},{:.2}) 出发门未过{}",
                            unit.0,
                            landing[0],
                            landing[1],
                            landing[2],
                            unmovable
                                .map(|uid| format!("，{uid} 记入不可达表"))
                                .unwrap_or_default(),
                        );
                        if mind.force_updates > 0 {
                            continue 'cascade;
                        }
                        continue 'npc;
                    }
                    // GeneratePath: the agent's own query, with its sampled retry.
                    let polyline = field.generate_path(from_xz, landing_xz);
                    match depart_along(
                        unit,
                        &mut state.0,
                        &mut path.0,
                        &mut route,
                        walk_face,
                        face,
                        landing,
                        fit,
                        &mut *rng,
                        polyline,
                    ) {
                        Some(depart_phase) => {
                            // A fixture talk whose pre-action carries a timeline:
                            // its move, window and timeline run as one fixture
                            // session from here.
                            let talk_timeline = fixture_talk
                                .as_mut()
                                .and_then(|prepared| prepared.timeline.take());
                            if let Some(selected) = fixture_selection.take().or(talk_timeline) {
                                if !fixture_activities.begin(selected, actions.enable_talk) {
                                    record.emit("activity_refused", calls);
                                    route.cancel();
                                    path.0 = moly_law::path::NpcPathWalkSlot::from_corners(Vec::new());
                                    *phase = MotionPhase::Dwelling { remaining: None };
                                    finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                                    if mind.force_updates > 0 {
                                        continue 'cascade;
                                    }
                                    continue 'npc;
                                }
                            }
                            record.emit("departed", calls);
                            if let DecisionRoute::SubObjective(kind) = decision_route {
                                let members = slot
                                    .current
                                    .as_ref()
                                    .map(|data| data.characters.clone())
                                    .unwrap_or_default();
                                groups.begin_sub(entity, unit.0, kind, members);
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
                        None if groups.owns(entity) => {
                            record.emit("no_route", calls);
                            groups.main_departure_failed(entity);
                            continue 'npc;
                        }
                        None if matches!(decision_route, DecisionRoute::SubObjective(_)) => {
                            record.emit("no_route", calls);
                            if let DecisionRoute::SubObjective(kind) = decision_route {
                                sub_objective_move_failed(kind, &mut mind, &mut actions, &mut rest, frame);
                            }
                            continue 'cascade;
                        }
                        None => {
                            record.emit("no_route", calls);
                            // GeneratePath 两次查询都没给出拐点（TryGeneratePath
                            // 失败 ⇒ NoneRoute）：移动失败收场，进停顿等下一轮。
                            finish_objective(unit.0, &mut mind, &mut slot, frame, true);
                            info!(
                                "[npc unit={}] 目标裁决 {source_word}：{draws_word} 门[{}]={:.0} [{}]={:.0} [{}]={:.0} [{}]={:.0} → {objective_word}（{detail}）→ 落点 ({:.2},{:.2},{:.2}) GeneratePath 无拐点，移动失败",
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
                    record.emit("no_destination", calls);
                    // 三条链全未命中：站定收场（无对话目标照常补位断环，但不
                    // 立跳过旗——见模块注释），进停顿等下一轮。
                    finish_objective(unit.0, &mut mind, &mut slot, frame, true);
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
            // A no-talk objective whose move failed on this pass owes its
            // ForceUpdateObjective on this same frame.
            if mind.force_updates > 0 {
                continue 'cascade;
            }
            break 'cascade;
        }
    }
}

/// Whether `actor` carries forced no-talk data that its slot still holds
/// (the same fixture in no-talk data); stale forced data is dropped.
fn forced_none_talk(
    activities: &mut crate::npc_fixture_activity::Factory<'_, '_>,
    actor: Entity,
    slot: &TalkSlot,
) -> bool {
    let Some(selected) = activities.take_forced(actor) else {
        return false;
    };
    let holds = slot.current.as_ref().is_some_and(|data| {
        data.kind == TalkType::NoneTalk && data.target_fixture == Some(selected.target.entity)
    });
    if holds {
        activities.keep_forced(actor, selected);
    }
    holds
}

/// The objective ended on `frame` (`failed`: it ended without its body, for
/// example a failed move). The AI loop yields one frame before its next
/// TryRest.
///
/// A no-talk objective's end owes ForceUpdateObjective calls instead, all
/// made by the next decision pass: the end of its fixture timeline makes two
/// (the cancel of the waiting objective runs one, then the timeline runs one
/// more) and the objective itself ends on the following frame, when its wait
/// sees the cancel; a failed move makes one and ends the objective on its own
/// frame. Each call resets the AI model, so the flag is clear and the Rest
/// after the Yield runs in full; the decision after the Rest reads the last
/// call's data.
pub(crate) fn finish_objective(
    unit: u32,
    mind: &mut ObjectiveMind,
    slot: &TalkSlot,
    frame: u32,
    failed: bool,
) {
    mind.executing = false;
    mind.body = None;
    if mind.current == Some(ObjectiveType::NoneTalk) && slot.kind() == Some(TalkType::NoneTalk) {
        mind.force_updates = if failed { 1 } else { 2 };
        mind.force_update_end = if failed { frame } else { frame.wrapping_add(1) };
        mind.skip_next_rest = false;
        info!(
            "[npc unit={unit}] frame={frame} 无对话目标收场（{}）：{} 次 ForceUpdateObjective，随后完整停顿",
            if failed { "移动失败" } else { "时间轴结束" },
            mind.force_updates
        );
        return;
    }
    mind.yield_since = Some(frame);
}

/// A member's sub objective whose move failed at its start: the sub
/// objective 18 makes Model.ForceUpdateObjective and ends (no cancel); the
/// sub objective 9's failed move throws its cancellation, whose OnCancel
/// changes to Idle and makes one ForceUpdateObjective. Both calls are made on
/// the same pass.
fn sub_objective_move_failed(
    kind: ObjectiveType,
    mind: &mut ObjectiveMind,
    actions: &mut crate::npc::NpcActions,
    rest: &mut crate::npc::RestLifecycle,
    frame: u32,
) {
    if kind == ObjectiveType::SubCharacterFixtureAction {
        crate::npc_fixture_talk::model_force_update(mind, frame);
    } else {
        actions.change(crate::npc::NpcAction::Idle, rest);
        owe_force_updates(mind, frame, 1);
    }
}

/// presenter.ForceUpdateObjective's owed part on a character that is not
/// talking: its current objective is cancelled here, and `count`
/// ForceUpdateObjective calls (each a reset and a row-1 cascade) are made by
/// its next decision pass; the objective ends on `end`, then the loop yields
/// and runs TryRest (a reset clears the flag that would skip it).
pub(crate) fn owe_force_updates(mind: &mut ObjectiveMind, end: u32, count: u8) {
    mind.cancelled = true;
    mind.executing = false;
    mind.body = None;
    mind.rest = None;
    mind.yield_since = None;
    mind.skip_next_rest = false;
    mind.force_updates = mind.force_updates.saturating_add(count);
    mind.force_update_end = end;
}

/// The talk objective after its move succeeded: OnArrive, the common-fixture
/// tweet, then the pre-action (its tweet, then its timeline). A tweet leaves
/// the body waiting; otherwise the objective ends on this frame.
fn on_talk_arrived(
    unit: u32,
    frame: u32,
    slot: &mut TalkSlot,
    mind: &mut ObjectiveMind,
    actions: &mut crate::npc::NpcActions,
    rest: &mut crate::npc::RestLifecycle,
) {
    let Some((kind, has_fixture)) = slot
        .current
        .as_ref()
        .map(|data| (data.kind, data.target_fixture.is_some()))
    else {
        finish_objective(unit, mind, slot, frame, false);
        return;
    };
    // OnArrive: a member waiting with a communication skips it; without a
    // target fixture there is nothing to look at.
    if kind != TalkType::CommunicationWhileDoingWait && has_fixture {
        info!("[npc unit={unit}] frame={frame} OnArrive: call=DoLookAt(fixture)");
    }
    // The common-fixture tweet reads the AI model's tweet id.
    if kind == TalkType::CommonFixture {
        let id = actions.tweet_id;
        if try_play_tweet(unit, frame, kind, id, TweetStep::CommonFixture, mind, actions, rest) {
            return;
        }
    }
    if !run_pre_action(unit, frame, slot, mind, actions, rest) {
        // The pre-action's timeline: data from the general factory carries
        // no fixture timeline.
        finish_objective(unit, mind, slot, frame, false);
    }
}

/// The pre-action: without one, nothing; its wait with a communication needs
/// a together-communication id, which no general talk carries and this
/// host's pre-action projection does not hold; then its tweet. Returns
/// whether a tweet wait started.
fn run_pre_action(
    unit: u32,
    frame: u32,
    slot: &TalkSlot,
    mind: &mut ObjectiveMind,
    actions: &mut crate::npc::NpcActions,
    rest: &mut crate::npc::RestLifecycle,
) -> bool {
    let Some(data) = slot.current.as_ref() else {
        return false;
    };
    let Some(pre_action) = data.pre_action.as_ref() else {
        return false;
    };
    try_play_tweet(
        unit,
        frame,
        data.kind,
        pre_action.id,
        TweetStep::PreAction,
        mind,
        actions,
        rest,
    )
}

/// TryPlayTweetAsync: skipped for a multiple-character fixture talk or a
/// zero id; otherwise the tweet id, the tweet state Pending and the change to
/// the tweet state, then the wait for Done.
#[allow(clippy::too_many_arguments)]
fn try_play_tweet(
    unit: u32,
    frame: u32,
    kind: TalkType,
    tweet_id: i32,
    step: TweetStep,
    mind: &mut ObjectiveMind,
    actions: &mut crate::npc::NpcActions,
    rest: &mut crate::npc::RestLifecycle,
) -> bool {
    if kind == TalkType::MultipleCharacterFixture || tweet_id == 0 {
        return false;
    }
    actions.tweet_id = tweet_id;
    actions.tweet_state = crate::npc_state::TweetState::Pending;
    actions.change(crate::npc::NpcAction::Tweet, rest);
    mind.body = Some(BodyWait::TweetDone { since: frame, step });
    info!("[npc unit={unit}] frame={frame} tweet {tweet_id}: Pending, change to Tweet");
    true
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
mod social_candidate_tests {
    use super::*;

    #[test]
    fn current_is_required_but_a_prepared_no_talk_needs_no_master() {
        let mut slot = TalkSlot::default();
        assert_eq!(prepared_social_target(&slot), None);
        slot.current = Some(AiTalkData::pending(TalkType::NoneTalk, 11, [3., 0., 4.]));
        assert_eq!(prepared_social_target(&slot), None);
        let current = slot.current.as_mut().unwrap();
        current.pending_factory = None;
        assert!(current.content.is_none());
        assert_eq!(prepared_social_target(&slot), Some([3., 0., 4.]));
    }

    #[test]
    fn social_pool_filters_identity_site_and_current_and_keeps_two_targets() {
        let mut snap = MemberSnap {
            entity: Entity::PLACEHOLDER, target_fixture: None, unit: 11,
            site_type: "garden".into(), position: [1., 0., 2.],
            destination: [9., 0., 8.], talk_target: Some([3., 0., 4.]),
            state: 0, cancel_reports: false,
        };
        assert!(snap.social_candidate(11, "garden").is_none());
        assert!(snap.social_candidate(12, "myroom").is_none());
        let candidate = snap.social_candidate(12, "garden").unwrap();
        assert_eq!(candidate.position, snap.position);
        assert_eq!(candidate.destination, [9., 0., 8.]);
        assert_eq!(candidate.talk_target, [3., 0., 4.]);
        snap.talk_target = None;
        assert!(snap.social_candidate(12, "garden").is_none());
    }
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
            ref_y: 0.0, walkable: Vec::new(), epoch: 1, field, generation: 1, site_height: 0.0 }
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
