//! NPC 名册、动作生命周期与导航路线执行。
//!
//! 每名角色的身份、速度、路径与动作状态随实体持有。目标机提供目的地，
//! 本模块将导航 corners 与侧偏休息候选组成带类型的路点表，再逐腿查询导航。
//! 原始 corners（包括插入的自身点）是 CheckPoint，通过后继续；只有侧偏
//! Rest 点到达后等待角色表的 PauseSeconds，并进入独立的 Rest 脚本生命周期。
//! 表中的径距排序不保证相邻点直通，不能拿两点间直线代替每腿的导航路径。
//!
//! 路线完成与停止分别发布 Arrived/Stopped。几何相位只供移动与动画使用，
//! 目标执行者读明确结果，不把任意 Dwelling 相位当成完成。卡住时停止路线，
//! 不伪造到达；玩家谈话显式停止移动，不清 AI Current/Previous。
//!
//! 现有导航后端仍为侵蚀格场与恒速折线，并非原生代理的完整动力学；每帧
//! 校验实际经过的各段，朝向取该腿当前剩余 corner。普通导航不统一插入
//! 原地预转；独立转身与家具局部贴合保留各自相位。
//!
//! 常规路线按源调用选步行，速度来自角色表的 walkSpeedMetersPerSecond。
//! 出生名册/分布仍为离线点名与面内等距散布的具名替身；家具 Fit 的
//! 旋转次序、完整动作退出与导航再附着仍待收口，不能当作已还原的生命周期。
//! 换站保留实体/装配，由 reseed 在新面定案后重置落位与目标机。

use crate::site::GroundMeshes;
use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::objective::rest_delay_milliseconds;
use moly_law::path::WalkState as LawWalkState;
use moly_law::path::{
    angle_between, build_waypoints, facing_direction, heading_yaw, rotate_time, turn_angle,
    turn_motion, NpcPathWalkSlot, TurnMotion, WalkVerdict, Waypoint, WaypointKind,
    ARRIVAL_DISTANCE, FORWARD_FALLBACK, WAYPOINT_SAMPLE_DISTANCE,
};

/// 离线名册点名（native `MOLY_ROSTER_UNITS` / web `roster_units`）。
/// Compact 默认只生成三名NPC；Full 或显式 `all` 保留全角色清单。
/// 只减少实例，不缩减角色资产目录，也不改变成员自己的模拟。名册覆盖到谁会翻转
/// 下游分支（如 tweet after-edit 池为空的角色静默跳过），验收要能点名
/// 铺到那一行。只做形状解析；点名是否在清单里由消费点连同清单一起校验。
fn roster_units(content: crate::site::OfflineSceneContent) -> Option<Vec<u32>> {
    #[cfg(not(target_arch = "wasm32"))]
    let raw = std::env::var("MOLY_ROSTER_UNITS").ok();
    #[cfg(target_arch = "wasm32")]
    let raw = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("roster_units"));
    let Some(raw) = raw else {
        return (content == crate::site::OfflineSceneContent::Compact).then(|| vec![1, 5, 13]);
    };
    if raw.trim() == "all" {
        return None;
    }
    let ids: Vec<u32> = raw
        .split(',')
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("MOLY_ROSTER_UNITS 的项不是 unit id：{token:?}"))
        })
        .collect();
    if ids.is_empty() {
        panic!("MOLY_ROSTER_UNITS 是空的：要么删掉它走默认名册，要么点名至少一名");
    }
    Some(ids)
}

/// 名册成员身位距站点中心的半径，替身值（真源出生点的来源未接）。
/// 只喂玩家出生环（`player` 域与 [`ring_scale`]）；名册成员的落位走
/// 目标面的可行走格（见 `npc_objective::seed_position`）。
const SEED_RING_RADIUS: f32 = 3.0;

/// 地表高度采样半径：取「脚下的地面」，与站点取景同款。
const SURFACE_RADIUS: f32 = 2.0;

/// 名册成员的身份：真源 `npcRegistry.characterUnitIds` 身份列的一行。
#[derive(Component)]
pub struct CharacterUnitId(pub u32);

/// 常规路线的步行速度，来自角色清单的运行时米/秒列。
#[derive(Component)]
pub struct WalkSpeed(pub f32);

/// 步间停顿时长（秒）：真源角色表的 `locomotion.pauseSeconds` 列（目标
/// 机主循环每轮决策前的停顿时长，见 `npc_objective` 的停顿律）。
#[derive(Component)]
pub struct PauseSeconds(pub f32);

/// Source action identity is independent of the rendered motion phase. Scripted
/// turns inside a conversation must not overwrite Talk or its Before state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum NpcAction {
    Idle = 0,
    AutoMove = 1,
    Rotate = 2,
    Talk = 4,
    Walk = 5,
    Rest = 7,
    None = 8,
    Tweet = 9,
    FixtureAction = 11,
    FixtureActionIdle = 12,
    ChangeSite = 14,
    Communication = 20,
}

#[derive(Component)]
pub(crate) struct NpcActions {
    pub(crate) current: NpcAction,
    pub(crate) before: NpcAction,
    pub(crate) enable_talk: bool,
    pub(crate) is_tweeting: bool,
    pub(crate) talk_owner: Option<Entity>,
    pub(crate) site_type: String,
    registered: bool,
    scene: u64,
}

impl Default for NpcActions {
    fn default() -> Self {
        Self {
            current: NpcAction::None,
            before: NpcAction::None,
            enable_talk: false,
            is_tweeting: false,
            talk_owner: None,
            site_type: String::new(),
            registered: false,
            scene: 0,
        }
    }
}

impl NpcActions {
    pub(crate) fn ready(&self) -> bool {
        self.registered
    }

    pub(crate) fn change(&mut self, next: NpcAction, rest: &mut RestLifecycle) {
        if self.current == next {
            return;
        }
        if self.current == NpcAction::Rest {
            rest.leave();
        }
        self.before = self.current;
        self.current = next;
    }

    pub(crate) fn begin_objective_rest(&mut self, rest: &mut RestLifecycle, revision: u64) {
        self.change(NpcAction::Rest, rest);
        rest.enter(RestSource::Objective {
            scene: self.scene,
            revision,
        });
    }

    fn begin_waypoint_rest(&mut self, rest: &mut RestLifecycle, route: &RouteStops) {
        self.change(NpcAction::Rest, rest);
        rest.enter(RestSource::Waypoint {
            scene: self.scene,
            index: route.next,
            goal: route.goal.map(|point| point.map(f32::to_bits)),
        });
    }
}

/// One actual Rest interval. MotionPhase::Dwelling alone is not a Rest state:
/// idle, fixture waits and player conversations also use that motion shape.
#[derive(Component, Default)]
pub(crate) struct RestLifecycle {
    epoch: u64,
    source: Option<RestSource>,
}

#[derive(Clone, PartialEq, Eq)]
enum RestSource {
    Objective {
        scene: u64,
        revision: u64,
    },
    Waypoint {
        scene: u64,
        index: usize,
        goal: Option<[u32; 3]>,
    },
}

impl RestLifecycle {
    pub(crate) fn active_epoch(&self) -> Option<u64> {
        self.source.as_ref().map(|_| self.epoch)
    }

    fn enter(&mut self, source: RestSource) {
        if self.source.is_none() {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.source = Some(source);
    }

    fn leave(&mut self) {
        if self.source.take().is_some() {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }
}

/// Registration/retirement edges only. The route and objective producers enter
/// Rest explicitly; an old timer cannot re-enter it after a player conversation.
pub(crate) fn sync_rest_lifecycle(
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    mut npcs: Query<(
        &mut NpcActions,
        Option<&crate::character::MotionDriver>,
        Option<&crate::character_material::ToonMaterials>,
        Option<&crate::talk::TalkHold>,
        Option<&crate::balloon::AfterEditHold>,
        &mut RestLifecycle,
    )>,
) {
    // SiteActive removal is the current host's scene-retirement signal.
    // Visibility and navigation readiness are not Rest state transitions.
    let scene = epoch
        .as_deref()
        .filter(|_| site.is_some())
        .map(|epoch| epoch.0);
    for (mut actions, driver, toon, talk, reaction, mut rest) in &mut npcs {
        let Some(scene) = scene else {
            rest.leave();
            continue;
        };
        if !actions.registered && driver.is_some() && toon.is_some() {
            actions.registered = true;
            actions.scene = scene;
            actions.site_type = site.as_ref().unwrap().site_type.clone();
            actions.enable_talk = true;
            actions.change(NpcAction::Idle, &mut rest);
        }
        if actions.current != NpcAction::Rest || talk.is_some() || reaction.is_some() {
            rest.leave();
        }
    }
}

/// Applied before the Rest/script hand-off. Stop movement, not the AI content
/// or goal: the goal executor observes a distinct stopped result afterwards.
pub(crate) fn enter_player_talk(commands: &mut Commands, entity: Entity, player: Entity) {
    commands.queue(move |world: &mut World| {
        let mut query = world.query::<(
            &mut NpcActions,
            &mut RestLifecycle,
            &mut RouteStops,
            &mut PathSlot,
            &mut WalkState,
            &mut MotionPhase,
        )>();
        if let Ok((mut actions, mut rest, mut route, mut path, mut walk, mut phase)) =
            query.get_mut(world, entity)
        {
            actions.change(NpcAction::Talk, &mut rest);
            actions.talk_owner = Some(player);
            route.stop();
            path.0 = NpcPathWalkSlot::from_corners(Vec::new());
            walk.0.next_corner = 0;
            *phase = MotionPhase::Dwelling { remaining: None };
        }
    });
}

/// General/Set read the live Before state at their normal end. This is not an
/// AI reset and does not write the shared previous-content slot.
pub(crate) fn leave_player_talk(commands: &mut Commands, entity: Entity) {
    commands.queue(move |world: &mut World| {
        let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
        if let Ok((mut actions, mut rest)) = query.get_mut(world, entity) {
            if actions.current != NpcAction::Talk {
                return;
            }
            let next = match actions.before {
                NpcAction::FixtureAction | NpcAction::FixtureActionIdle => {
                    NpcAction::FixtureActionIdle
                }
                _ => NpcAction::Idle,
            };
            actions.change(next, &mut rest);
            actions.talk_owner = None;
        }
    });
}

/// The ordinary objective owner has accepted cancellation. Dispose only its
/// movement/Rest execution; Current/Previous content and MemberRng stay owned
/// by the retained AI instance.
pub(crate) use stop_for_external_activity as stop_for_layout_edit;

pub(crate) fn stop_for_external_activity(world: &mut World, actor: Entity) {
    let mut query = world.query::<(
        &mut NpcActions,
        &mut RestLifecycle,
        &mut RouteStops,
        &mut PathSlot,
        &mut WalkState,
        &mut StuckBaseline,
        &mut MotionPhase,
    )>();
    let Ok((mut actions, mut rest, mut route, mut path, mut walk, mut stuck, mut phase)) =
        query.get_mut(world, actor)
    else {
        return;
    };
    if actions.current == NpcAction::Talk {
        return;
    }
    route.cancel();
    path.0 = NpcPathWalkSlot::from_corners(Vec::new());
    walk.0.next_corner = 0;
    stuck.0 = None;
    *phase = MotionPhase::Dwelling { remaining: None };
    actions.change(NpcAction::Idle, &mut rest);
}

/// Show synchronizes the retained actor with the rebuilt field. Unhandled
/// objective branches keep their route/content and the existing generation
/// mover replans them; it must not blindly follow a stale path after Save.
pub(crate) fn reattach_after_layout_edit(
    world: &mut World,
    actor: Entity,
    point: Vec3,
    generation: u64,
) {
    let mut query = world.query::<(
        &mut Transform,
        &mut WalkState,
        &mut RouteStops,
        &PathSlot,
        &mut StuckBaseline,
    )>();
    let Ok((mut transform, mut walk, mut route, path, mut stuck)) = query.get_mut(world, actor)
    else {
        return;
    };
    transform.translation = point;
    walk.0.position = point.to_array();
    walk.0.forward = (transform.rotation * Vec3::Z).to_array();
    stuck.0 = None;
    route.generation = if route.goal.is_none() && path.0.corners().is_empty() {
        generation
    } else {
        0
    };
}

/// 当前 waypoint 的导航拐点。与包含 Rest/CheckPoint 身份的整条路点表分开。
#[derive(Component)]
pub struct PathSlot(pub NpcPathWalkSlot);

/// 跨帧行走状态原形：位置、朝向、拐点进度。位置与朝向每帧由此翻译进
/// [`Transform`]，进度由 [`advance`] 消费。
#[derive(Component)]
pub struct WalkState(pub LawWalkState);

/// 当前目标的目的地（世界坐标）：目标机的决策写入（出发时），状态行与
/// 目标域读它。执行器的逐站喂腿读路点表（[`RouteStops`]），不读它。
#[derive(Component)]
pub struct MoveTarget(pub [f32; 3]);

/// 整条带类型路线：原导航 corners 为 CheckPoint，侧偏候选为 Rest，合并后
/// 按距起点的三维直线距离稳定排序。`next` 指向当前前往/正在休息的路点；
/// CheckPoint 到达后推进，Rest 到达且等待结束后推进。两者都不擅自跳过导航。
#[derive(Component, Default)]
pub struct RouteStops {
    /// 世界坐标与行为身份。侧偏点保留原候选位置，不替换为采样命中位置。
    stops: Vec<Waypoint>,
    /// 下一步要去的站。
    next: usize,
    generation: u64,
    goal: Option<[f32; 3]>,
    fit: Option<FitCandidate>,
    /// 最后贴合前的导航落点；下一次开启代理时用于从局部挂点返回面。
    reentry: Option<[f32; 3]>,
    pub(crate) outcome: Option<RouteOutcome>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteOutcome {
    Arrived,
    Stopped,
}

impl RouteStops {
    fn stop(&mut self) {
        let generation = self.generation;
        self.cancel();
        self.generation = generation;
        self.outcome = Some(RouteOutcome::Stopped);
    }
    pub(crate) fn cancel(&mut self) {
        let reentry = self.reentry;
        *self = Self::default();
        self.reentry = reentry;
    }

    pub(crate) fn navigation_origin(
        &self,
        position: [f32; 3],
        face: &crate::walk_face::WalkFace,
    ) -> [f32; 3] {
        if face.walkable_at([position[0], position[2]]) {
            return position;
        }
        self.reentry
            .filter(|p| face.walkable_at([p[0], p[2]]))
            .unwrap_or(position)
    }

    /// Pause route bookkeeping while the activity owns local movement.
    pub(crate) fn hold_for_fixture(&mut self) {
        let generation = self.generation;
        self.cancel();
        self.generation = generation;
    }

    /// A completed furniture exit already chose its current surface position.
    /// Reusing the old pre-seat checkpoint would undo the real EndLoc exit.
    pub(crate) fn finish_fixture(&mut self) {
        let generation = self.generation;
        *self = Self::default();
        self.generation = generation;
    }
}

/// 卡死基线（源卡死上报的跨帧状态）：上次记下的位置与时刻。`None` = 未立
/// （行走首帧立、上报后清零——可再触发）。
#[derive(Component, Default)]
pub struct StuckBaseline(Option<([f32; 3], f32)>);

/// 卡死距离阈（源字面量）：位移不小于它即「在动」，基线前移。
const STUCK_DISTANCE: f32 = 0.09;

/// 卡死时间窗（源字面量，秒）：窗内位移不足距离阈 ⇒ 卡死上报。
const STUCK_SECONDS: f32 = 5.0;

/// 位移段族名（真源 locomotion 的 idleMotion / walkMotion 两列）：共享
/// 动作库在族名上加段后缀成剪辑名。角色装配系统消费。
#[derive(Component)]
pub struct MotionClips {
    pub idle: String,
    pub walk: String,
}

/// 几何/动画相位。导航写普通移动，持留期间的转身由会话生产者推进；
/// 它不是完整的 NPC 动作态，也不能替代明确的路线结束结果。
#[derive(Component, Clone, Copy)]
pub enum MotionPhase {
    /// 在走：本帧位置由推进律移动。
    Walking,
    /// 在转体：位置冻结（路径槽为空、律每帧报 Idle），朝向在 `duration`
    /// 秒内从 `from` 插值到 `to`。`motion` 是转身律选中的段（动画域读它
    /// 选剪辑），`elapsed` 是已转秒数。
    Turning {
        motion: TurnMotion,
        from: Quat,
        to: Quat,
        duration: f32,
        elapsed: f32,
    },
    /// `Some(remaining)` 只代表 Rest 路点到达后的剩余秒数，CheckPoint 不写它。
    /// `None` 也用于未起步、已停止与会话持留，目标机应读 RouteOutcome，
    /// 不能从此形状推断成功到达。
    Dwelling { remaining: Option<f32> },
    /// 贴合支的先转到位（目标位是某条家具挂点时的第一步）：位置冻结
    /// （路径槽为空、律每帧报 Idle），朝向在 `duration` 秒内从 `from` 插
    /// 到 `to`——`to` 是挂点朝向。选段角喂欧拉 y 差折算到 [0,360)。
    /// 这是现有局部 Fit 实现的第一阶段；其旋转/位移先后仍待按活动链收口。
    /// 转完接 `FitWalking`（`leg` 随身带），不用于普通导航起步。
    FitTurning {
        motion: TurnMotion,
        from: Quat,
        to: Quat,
        duration: f32,
        elapsed: f32,
        leg: FitLeg,
    },
    /// 贴合支的直线插值（关代理后的唯一位移）：位置从 `leg.start` 线性
    /// 到 `leg.target`（匀速 0.3 m/s，末帧精确落位），朝向逐帧向
    /// `leg.move_quat` 收敛（插值率 t·1.5，逐帧锁 yaw-only）。无路点
    /// 驻留——到位即收场位，目标机的停顿照常接手。
    FitWalking { leg: FitLeg, elapsed: f32 },
}

/// 贴合腿的常量（起步一次算定，两相位随身携带）。
#[derive(Clone, Copy)]
pub struct FitLeg {
    /// 目标位（决策写的动作点目标：x/z 即挂点，y 是可行走面采样值）。
    pub target: [f32; 3],
    /// 起步时的自身位。
    pub start: [f32; 3],
    /// 行程秒数 = 3D 距离 / 0.3。
    pub travel: f32,
    /// 行进朝向（面朝水平移动向的旋转，起步一次算定）。
    pub move_quat: Quat,
}

impl FitLeg {
    /// Existing host local-fit arithmetic, shared by approach and exit.
    /// Call only on an in-progress positive-duration leg. The activity owner
    /// handles zero travel and the final position-only completion separately.
    pub(crate) fn sample_position(&self, elapsed: f32) -> Vec3 {
        let t = (elapsed / self.travel).clamp(0.0, 1.0);
        Vec3::from(self.start).lerp(Vec3::from(self.target), t)
    }

    pub(crate) fn sample_rotation(&self, current: Quat, elapsed: f32) -> Quat {
        let t = (elapsed / self.travel).clamp(0.0, 1.0);
        let blended = current.lerp(self.move_quat, (t * 1.5).clamp(0.0, 1.0));
        yaw_only(blended)
    }
}

/// Existing host turn sampler. This keeps its documented engine-tween
/// approximation in one place; an activity must not create a second easing.
pub(crate) fn sample_turn_rotation(from: Quat, to: Quat, t: f32) -> Quat {
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    from.slerp(to, eased)
}

/// 贴合支的触发件（身份判定过滤器的命中输出）：目标位 + 命中的那条
/// 挂点的世界朝向。目标域在决策时算好交给 [`depart`]。
#[derive(Clone, Copy)]
pub(crate) struct FitCandidate {
    pub position: [f32; 3],
    pub rotation: Quat,
}

/// 贴合支的步速（源字面量，米/秒）：全程匀速直线插值。
const FIT_SPEED: f32 = 0.3;

/// 唯一名册：真源的身份列叫 `characterUnitIds`，npc.* 族按它索引；
/// npc.* 族的数据活在实体 component 上，实体就是名册的行，不再有第二份
/// 按行数组。
#[derive(Resource)]
pub struct Registry {
    pub character_unit_ids: Vec<u32>,
}

/// 角色清单的原始文本走共享的 [`JsonAsset`]（moly-assets 的 json 装载器，
/// 一个扩展名一个装载器），字段解析在 [`Catalog::parse`]。

/// 角色清单的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct CharacterListHandle(Handle<JsonAsset>);

/// 解析完成的角色清单，每行 `(unitId, 运行时步行速, 步间停顿秒数, 待机
/// 段族名, 走姿段族名)`；常驻作为独立体验临时名册的真实角色蓝图。
#[derive(Resource)]
pub(crate) struct Catalog(Vec<(u32, f32, f32, String, String)>);

/// spawn 一次性闩：名册只铺一次。
#[derive(Resource)]
pub(crate) struct Spawned;

impl Catalog {
    /// 解析角色清单，只取名册消费的五列。缺列即响亮失败——资产加载边界
    /// 的一次性拒绝；报错只带字段名与 unitId，不带清单文本（清单含角色名）。
    /// 停顿列要能产出停顿时长（停顿律拒负值/NaN），产不出同样在这里响。
    fn parse(text: &str) -> Self {
        let value: serde_json::Value =
            serde_json::from_str(text).unwrap_or_else(|err| panic!("角色清单不是合法 JSON：{err}"));
        let characters = value
            .get("characters")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("角色清单缺 characters 对象"));
        let mut rows: Vec<_> = characters
            .values()
            .map(|row| {
                let unit_id =
                    row.get("unitId")
                        .and_then(|v| v.as_u64())
                        .unwrap_or_else(|| panic!("角色行缺 unitId")) as u32;
                let speed = row
                    .pointer("/locomotion/walkSpeedMetersPerSecond")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!("角色 {unit_id} 缺 locomotion.walkSpeedMetersPerSecond")
                    }) as f32;
                let pause = row
                    .pointer("/locomotion/pauseSeconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| panic!("角色 {unit_id} 缺 locomotion.pauseSeconds"))
                    as f32;
                if !pause.is_finite() || pause < 0.0 {
                    panic!("角色 {unit_id} 的 pauseSeconds 产不出停顿时长：{pause}");
                }
                let motion = |field: &str| {
                    row.pointer(&format!("/locomotion/{field}"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| panic!("角色 {unit_id} 缺 locomotion.{field}"))
                        .to_owned()
                };
                (
                    unit_id,
                    speed,
                    pause,
                    motion("idleMotion"),
                    motion("walkMotion"),
                )
            })
            .collect();
        // 行序按 unitId 定序：出生位按名册位次等距取格（见
        // `npc_objective::seed_position`），顺序要不受清单键的字典序影响
        // ——重播种按身份列排序，两处同序才能同一员拿到同一出生位。
        rows.sort_by_key(|(unit_id, ..)| *unit_id);
        Self(rows)
    }
}

/// Startup：请求装载角色清单。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let handle = server.load::<JsonAsset>(moly_assets::character_registry());
    commands.insert_resource(CharacterListHandle(handle));
}

/// Update：清单装载完成后解析一次。装载失败响亮 panic（资产边界的唯一
/// 拒绝点），未到齐静默等下一帧。内部形参带私有资源，故 pub(crate)。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    lists: Res<Assets<JsonAsset>>,
    handle: Option<Res<CharacterListHandle>>,
) {
    let Some(handle) = handle else {
        return; // 未请求，或已解析并撤下
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("角色清单装载失败：{err:?}");
    }
    let Some(list) = lists.get(&handle.0) else {
        return; // 还在装
    };
    commands.insert_resource(Catalog::parse(&list.0));
    commands.remove_resource::<CharacterListHandle>();
}

/// Update：清单与目标面都定案后，铺一次名册。内部形参带私有资源，
/// 故 pub(crate)。
///
/// 名册是具名离线输入（真源由站点管理按进度下发）。默认Compact只点名
/// 少量成员，Full保留全员；成员落位走目标面的可行走格等距散布（构造上
/// 不在家具脚印内、在面上）。目标机以出生态插入——起动旗立起（源起动
/// 装配的同形），首判在名册铺开的当帧由目标机执行：出生即决策、即出发。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    catalog: Option<Res<Catalog>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    face: Option<Res<crate::npc_objective::ObjectiveFace>>,
    placements: Res<crate::fixture::FixturePlacements>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    spawned: Option<Res<Spawned>>,
    selection: Res<crate::site::SiteSelection>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    if spawned.is_some() {
        return;
    }
    let Some(catalog) = catalog else {
        return;
    };
    // 三道门：站点定案代数在场（面不再含将被清扫的实例）、目标面对得上
    // 代数且可行走格非空、面板已立（目标层抽签门要读它）。
    let Some(epoch) = epoch else {
        return;
    };
    let Some(face_res) = face else {
        return;
    };
    let face: &crate::npc_objective::ObjectiveFace = &*face_res;
    if !face.is_fresh(epoch.0) || face.walkable().is_empty() {
        return;
    }
    let Some(config) = configs.as_deref() else {
        return;
    };

    // 名册行：按离线预设或显式点名集过滤（保持清单序，落位
    // 随人数摊）。点名不在清单里即响亮失败——静默丢一名会让冒烟覆盖
    // 悄悄变短。
    // No demonstration cast in the embedded stage: independent playback must
    // obtain every required member through spawn_temporary_units. Standalone
    // continues to use its explicit offline roster unchanged.
    let requested = if stage.is_some() { Some(Vec::new()) } else { roster_units(selection.content()) };
    let roster: Vec<&(u32, f32, f32, String, String)> = match requested {
        Some(ids) => {
            let set: std::collections::HashSet<u32> = ids.iter().copied().collect();
            if set.len() != ids.len() {
                panic!("MOLY_ROSTER_UNITS 有重复点名：{ids:?}");
            }
            for id in &ids {
                if !catalog.0.iter().any(|(unit_id, ..)| unit_id == id) {
                    panic!("MOLY_ROSTER_UNITS 点名的 unit {id} 不在角色清单里");
                }
            }
            catalog
                .0
                .iter()
                .filter(|(unit_id, ..)| set.contains(unit_id))
                .collect()
        }
        None => catalog.0.iter().collect(),
    };
    let count = roster.len();
    let mut ids = Vec::with_capacity(count);
    let (mut pause_min, mut pause_max) = (f32::INFINITY, f32::NEG_INFINITY);
    for (i, (unit_id, walk_speed, pause_seconds, idle, walk)) in roster.into_iter().enumerate() {
        let seed = crate::npc_objective::seed_position(face, i, count);
        pause_min = pause_min.min(*pause_seconds);
        pause_max = pause_max.max(*pause_seconds);
        let unit = CharacterUnitId(*unit_id);
        commands.spawn((
            unit,
            Transform::from_translation(Vec3::from(seed)),
            // 成员是渲染层级的节点：模型子实体的可见性沿父链向上查到本实体。
            // 不带它时子实体的可见性只能靠引擎的回退（视为可见）并告警。
            Visibility::default(),
            PathSlot(NpcPathWalkSlot::from_corners(Vec::new())),
            WalkSpeed(*walk_speed),
            (
                PauseSeconds(*pause_seconds),
                RestLifecycle::default(),
                NpcActions::default(),
            ),
            WalkState(LawWalkState::new(seed, FORWARD_FALLBACK)),
            MoveTarget(seed),
            RouteStops::default(),
            StuckBaseline::default(),
            MotionClips {
                idle: idle.clone(),
                walk: walk.clone(),
            },
            crate::npc_objective::TalkSlot::default(),
            // 出生态：起动旗立起（源起动装配「立即可执行下一目标」的同
            // 形），首判不等停顿。
            crate::npc_objective::ObjectiveMind::at_spawn(),
            crate::npc_objective::MemberRng::seeded(*unit_id),
            MotionPhase::Dwelling { remaining: None },
        ));
        ids.push(*unit_id);
    }
    commands.insert_resource(Registry {
        character_unit_ids: ids.clone(),
    });
    commands.insert_resource(Spawned);
    use crate::client_config::{
        KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
        KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ, KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
        KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT,
    };
    let pause_word = if count == 0 {
        "none".to_owned()
    } else if pause_min == pause_max {
        format!("{pause_min:.1}s/员")
    } else {
        format!("{pause_min:.1}-{pause_max:.1}s/员")
    };
    info!(
        "npc 名册就绪：{count} 名（unit {ids:?}），目标驱动：目标层抽签门[{}]={:.0} [{}]={:.0} [{}]={:.0} [{}]={:.0}（面板 FloatConfigs），停顿 {pause_word}，锚定对话家具 {} 件，可行走 {} 格，出生即首判",
        KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT,
        config.float(KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT),
        KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT,
        config.float(KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT),
        KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT,
        config.float(KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT),
        KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ,
        config.float(KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ),
        placements.anchored().len(),
        face.walkable().len(),
    );
}

/// Add exact cast members for a scoped independent experience. These are
/// ordinary NPC entities, so character assembly, animation, facial, talk and
/// cleanup remain owned by their existing systems.
pub(crate) fn spawn_temporary_units(
    world: &mut World,
    requested: &[u32],
) -> Result<Vec<Entity>, String> {
    let existing: std::collections::HashSet<u32> = world
        // The player avatar may share a unit id with a requested independent
        // cast member. It is not an NPC and has no NpcActions readiness state;
        // counting it here makes the staging gate wait forever for that actor.
        .query_filtered::<&CharacterUnitId, Without<crate::player::PlayerControlled>>()
        .iter(world)
        .map(|unit| unit.0)
        .collect();
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .ok_or_else(|| "独立场景还没有定案".to_owned())?
        .0;
    let face = world
        .get_resource::<crate::npc_objective::ObjectiveFace>()
        .filter(|face| face.is_fresh(epoch) && !face.walkable().is_empty())
        .ok_or_else(|| "独立场景的行走区域还在准备".to_owned())?;
    let rows = world
        .get_resource::<Catalog>()
        .ok_or_else(|| "角色蓝图尚未载入".to_owned())?;
    let missing: Vec<u32> = requested
        .iter()
        .copied()
        .filter(|unit| !existing.contains(unit))
        .collect();
    let blueprints: Vec<_> = missing
        .iter()
        .enumerate()
        .map(|(index, unit)| {
            let (_, speed, pause, idle, walk) = rows
                .0
                .iter()
                .find(|(id, ..)| id == unit)
                .ok_or_else(|| format!("角色 {unit} 不在当前来源的角色清单中"))?;
            Ok((
                *unit,
                *speed,
                *pause,
                idle.clone(),
                walk.clone(),
                crate::npc_objective::seed_position(face, index, missing.len().max(1)),
            ))
        })
        .collect::<Result<_, String>>()?;
    let mut spawned = Vec::with_capacity(blueprints.len());
    for (unit_id, speed, pause, idle, walk, seed) in blueprints {
        let entity = world
            .spawn((
                CharacterUnitId(unit_id),
                Transform::from_translation(Vec3::from(seed)),
                Visibility::default(),
                PathSlot(NpcPathWalkSlot::from_corners(Vec::new())),
                WalkSpeed(speed),
                (
                    PauseSeconds(pause),
                    RestLifecycle::default(),
                    NpcActions::default(),
                ),
                WalkState(LawWalkState::new(seed, FORWARD_FALLBACK)),
                MoveTarget(seed),
                RouteStops::default(),
                StuckBaseline::default(),
                MotionClips { idle, walk },
                crate::npc_objective::TalkSlot::default(),
                crate::npc_objective::ObjectiveMind::at_spawn(),
                crate::npc_objective::MemberRng::seeded(unit_id),
                MotionPhase::Dwelling { remaining: None },
            ))
            .id();
        spawned.push(entity);
    }
    if let Some(mut registry) = world.get_resource_mut::<Registry>() {
        registry.character_unit_ids.extend(missing.iter().copied());
        registry.character_unit_ids.sort_unstable();
        registry.character_unit_ids.dedup();
    }
    Ok(spawned)
}

pub(crate) fn remove_temporary_units(world: &mut World, entities: &[Entity]) {
    let units: std::collections::HashSet<u32> = entities
        .iter()
        .filter_map(|entity| world.get::<CharacterUnitId>(*entity).map(|unit| unit.0))
        .collect();
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(*entity) {
            entity.despawn();
        }
    }
    if let Some(mut registry) = world.get_resource_mut::<Registry>() {
        registry
            .character_unit_ids
            .retain(|unit| !units.contains(unit));
    }
}

/// Update：换站后的名册重播种。吃站点定案代数（见 `site::GroundEpoch`）：
/// 首代只记录不重排——名册刚以同式落位；代数翻新（换站）时成员实体原样
/// 保留（角色装配链不参与换站），按新目标面重新落位：路径清空进驻留、
/// 位置落新面的可行走格、目标机复位到出生态（换站 = 新一轮起动装配，
/// 首判旗再立）。目标面没跟上代数（重建窗）时本帧不记代数，下一帧整个
/// 重来——代数记了却不重排，这一代的重播种会被永久跳过。
pub(crate) fn reseed(
    epoch: Option<Res<crate::site::GroundEpoch>>,
    mut last: Local<u64>,
    face: Option<Res<crate::npc_objective::ObjectiveFace>>,
    mut npcs: Query<(
        &CharacterUnitId,
        &mut PathSlot,
        &mut WalkState,
        &mut MoveTarget,
        &mut RouteStops,
        &mut StuckBaseline,
        &mut Transform,
        &mut MotionPhase,
        &mut crate::npc_objective::TalkSlot,
        &mut crate::npc_objective::ObjectiveMind,
        &mut NpcActions,
        &mut RestLifecycle,
    )>,
) {
    let Some(epoch) = epoch else {
        return;
    };
    let Some(face_res) = face else {
        return;
    };
    let face: &crate::npc_objective::ObjectiveFace = &*face_res;
    if !face.is_fresh(epoch.0) {
        return;
    }
    if epoch.0 == *last {
        return;
    }
    let first = *last == 0;
    *last = epoch.0;
    if first {
        return;
    }
    // 落位按身份列稳定编号（查询序不定，重播种的位置要可复算）。
    let mut members: Vec<_> = npcs.iter_mut().collect();
    members.sort_by_key(|(unit, ..)| unit.0);
    let count = members.len();
    for (
        index,
        (
            unit,
            mut slot,
            mut state,
            mut target,
            mut route,
            mut stuck,
            mut transform,
            mut phase,
            mut talk_slot,
            mut mind,
            mut actions,
            mut rest,
        ),
    ) in members.into_iter().enumerate()
    {
        let seed = crate::npc_objective::seed_position(face, index, count);
        *slot = PathSlot(NpcPathWalkSlot::from_corners(Vec::new()));
        state.0 = LawWalkState::new(seed, FORWARD_FALLBACK);
        *target = MoveTarget(seed);
        *route = RouteStops::default();
        *stuck = StuckBaseline::default();
        *transform = Transform::from_translation(Vec3::from(seed));
        *phase = MotionPhase::Dwelling { remaining: None };
        talk_slot.reset_ai_talk_data();
        rest.leave();
        *actions = NpcActions::default();
        // 目标机复位到出生态：换站对成员是新一次起动（旗再立、停顿清零、
        // 槽位清空）。抽签引擎不复位——成员自己的跨站连续性没有真源
        // 依据可断，保留序列比假装重抽更诚实。
        *mind = crate::npc_objective::ObjectiveMind::at_spawn();
        info!(
            "[npc unit={}] 换站重播种：位次 {index}，落位 ({:.2},{:.2},{:.2})，出生即首判",
            unit.0, seed[0], seed[1], seed[2]
        );
    }
    info!("npc 名册重播种：{count} 名（换站代数 {}）", epoch.0);
}

/// 地表网格的世界顶点集合（站点 scene 未展开完成为空表）。
pub(crate) fn ground_verts(
    meshes: &Res<Assets<Mesh>>,
    ground: &GroundMeshes,
    parts: &Query<(&Mesh3d, &GlobalTransform)>,
) -> Vec<Vec3> {
    let mut verts = Vec::new();
    for (mesh3d, global) in parts {
        if !ground.0.contains(&mesh3d.0) {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            panic!("地表网格没有 float3 的 POSITION 属性");
        };
        for position in positions {
            verts.push(global.transform_point(Vec3::from(*position)));
        }
    }
    verts
}

/// 地表包围盒的 XZ 中心与中心脚下的地表高度。
pub(crate) fn center_of(verts: &[Vec3]) -> (Vec2, f32) {
    let (min, max) = bounds_of(verts);
    let center = Vec2::new((min.x + max.x) * 0.5, (min.z + max.z) * 0.5);
    (center, surface_y(verts, center.x, center.y, min.y))
}

/// 玩家出生环的收缩系数（玩家域复用；名册成员的落位走目标面的可行走
/// 格）：出生环半径对地表包围盒最小半跨的余量收缩。室外大地上得 1
/// （替身值原样，行为不变）；房间级地面上缩到出生点留在寻路面内（寻路
/// 面即站点地表网格）。
pub(crate) fn ring_scale(verts: &[Vec3]) -> f32 {
    const MARGIN: f32 = 0.6;
    let (min, max) = bounds_of(verts);
    let half = ((max.x - min.x) * 0.5).min((max.z - min.z) * 0.5);
    (MARGIN * half / SEED_RING_RADIUS).min(1.0)
}

fn bounds_of(verts: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(-f32::MAX);
    for world in verts {
        min = min.min(*world);
        max = max.max(*world);
    }
    (min, max)
}

/// 驻留秒数：停顿列走「秒 → 毫秒」律（源的路点驻留与步间停顿共用同一条
/// 换算，整数毫秒、无穷 ⇒ 0 秒立进立出）。
fn dwell_seconds(pause_seconds: f32) -> f32 {
    let milliseconds = rest_delay_milliseconds(pause_seconds)
        .unwrap_or_else(|| panic!("pauseSeconds 列产不出驻留时长：{pause_seconds}"));
    milliseconds as f32 / 1000.0
}

/// 构建整条 typed 路线并提交第一个 CheckPoint。普通目标复用可行走面查询，
/// 家具 Fit 目标先严格导航到接近点，路线完成后才进入局部贴合。
/// 初始/重复 corner 保持 CheckPoint，不额外驻留；无路返回 None。
/// 高度几何只为已查询到的导航点落高，侧偏候选由单独的三维采样门准入。
pub(crate) fn depart(
    unit: &CharacterUnitId,
    state: &mut LawWalkState,
    slot: &mut NpcPathWalkSlot,
    route: &mut RouteStops,
    walk_face: &crate::walk_face::WalkFace,
    objective_face: &crate::npc_objective::ObjectiveFace,
    corner: [f32; 3],
    fit: Option<FitCandidate>,
    rng: &mut crate::npc_objective::MemberRng,
) -> Option<MotionPhase> {
    route.outcome = None;
    let mut start = [state.position[0], state.position[2]];
    if !walk_face.walkable_at(start) {
        // 源下次 MoveAsync 先重新开启 NavMeshAgent。这里只恢复已完成
        // 局部 Fit 的已验证接近点；禁止任意无效起点用一条直线连入场。
        let reentry = route.reentry?;
        if !walk_face.walkable_at([reentry[0], reentry[2]]) {
            return None;
        }
        state.position = reentry;
        start = [reentry[0], reentry[2]];
    }
    // 出生与重烘修复由各自生命周期处理；不把不可走起点插进路线制造穿洞首腿。
    if !walk_face.walkable_at(start) {
        return None;
    }
    let goal = [corner[0], corner[2]];
    let polyline = if fit.is_some() {
        // MoveExecutor 先 CalculatePath，再严验末拐点可达（0.01）；
        // AutoMove 完成后才 NoUseNavmeshMoveAsync。0.3 来自动作点采样门。
        let approach = walk_face.sample(goal, crate::npc_objective::SAMPLE_GATE_TOLERANCE)?;
        walk_face.path_exact(start, approach)?
    } else {
        walk_face.path(start, goal)?
    };
    let corners = lift_navigation_path(unit, objective_face, polyline);
    route.stops = build_waypoints(state.position, &corners, state.forward, rng, |candidate| {
        objective_face
            .sample(candidate, WAYPOINT_SAMPLE_DISTANCE)
            .is_some()
    });
    route.next = 0;
    route.generation = walk_face.generation();
    route.goal = Some(corner);
    route.fit = fit;
    route.reentry = None;
    *slot = NpcPathWalkSlot::from_corners(Vec::new());
    let last = route.stops.last()?.position;
    let rests = route
        .stops
        .iter()
        .filter(|point| point.kind == WaypointKind::Rest)
        .count();
    info!(
        "[npc unit={}] 路线起步：{} CheckPoint / {rests} Rest，末站 ({:.2},{:.2},{:.2})",
        unit.0,
        route.stops.len() - rests,
        last[0],
        last[1],
        last[2]
    );
    // Ordinary navigation starts by submitting its first checkpoint. Explicit
    // turn actions and the separate local furniture-fit path retain their owners.
    start_waypoint(unit, state, slot, route, walk_face, objective_face)
}

fn lift_navigation_path(
    unit: &CharacterUnitId,
    face: &crate::npc_objective::ObjectiveFace,
    points: Vec<[f32; 2]>,
) -> Vec<[f32; 3]> {
    points
        .into_iter()
        .map(|point| {
            face.surface_sample([point[0], face.ref_y(), point[1]], WAYPOINT_SAMPLE_DISTANCE)
                .unwrap_or_else(|| {
                    panic!(
                        "[npc unit={}] 路点 ({:.2},{:.2}) 在目标面上无采样（两份面数据不一致）",
                        unit.0, point[0], point[1]
                    )
                })
        })
        .collect()
}

/// Each ordered waypoint is a navigation destination, not a permission to
/// traverse the straight chord. Side candidates and radial sorting can place
/// an obstacle between adjacent waypoints, so the same navigation field supplies
/// each leg. The unsnapped waypoint remains the actual arrival predicate.
fn start_waypoint(
    unit: &CharacterUnitId,
    state: &mut LawWalkState,
    slot: &mut NpcPathWalkSlot,
    route: &mut RouteStops,
    walk_face: &crate::walk_face::WalkFace,
    objective_face: &crate::npc_objective::ObjectiveFace,
) -> Option<MotionPhase> {
    let Some(waypoint) = route.stops.get(route.next).copied() else {
        let fit = route.fit.take();
        route.stops.clear();
        route.next = 0;
        route.goal = None;
        let phase = fit
            .and_then(|fit| fit_depart(unit, state, slot, route, fit))
            .unwrap_or(MotionPhase::Dwelling { remaining: None });
        if matches!(phase, MotionPhase::Dwelling { remaining: None }) {
            route.outcome = Some(RouteOutcome::Arrived);
        }
        return Some(phase);
    };
    let corners =
        if Vec3::from(waypoint.position).distance(Vec3::from(state.position)) < ARRIVAL_DISTANCE {
            // Even a coincident CheckPoint is an execution leg; the next update
            // observes arrival without inventing a Rest delay.
            vec![state.position]
        } else {
            let sampled = objective_face.sample(waypoint.position, WAYPOINT_SAMPLE_DISTANCE)?;
            let points = walk_face.path_exact(
                [state.position[0], state.position[2]],
                [sampled[0], sampled[2]],
            )?;
            lift_navigation_path(unit, objective_face, points)
        };
    // Keep nearby corners, including the query's start. Removing every corner
    // inside the arrival radius can cut a short but necessary obstacle turn.
    *slot = NpcPathWalkSlot::from_corners(corners);
    state.next_corner = 0;
    info!(
        "[npc unit={}] 路点 {}/{} {:?} 出发 → ({:.2},{:.2},{:.2})",
        unit.0,
        route.next + 1,
        route.stops.len(),
        waypoint.kind,
        waypoint.position[0],
        waypoint.position[1],
        waypoint.position[2]
    );
    Some(MotionPhase::Walking)
}

fn next_waypoint_or_stop(
    unit: &CharacterUnitId,
    state: &mut LawWalkState,
    slot: &mut NpcPathWalkSlot,
    route: &mut RouteStops,
    walk_face: &crate::walk_face::WalkFace,
    objective_face: &crate::npc_objective::ObjectiveFace,
) -> MotionPhase {
    route.next += 1;
    start_waypoint(unit, state, slot, route, walk_face, objective_face).unwrap_or_else(|| {
        route.stop();
        *slot = NpcPathWalkSlot::from_corners(Vec::new());
        state.next_corner = 0;
        warn!(
            "[npc unit={}] waypoint has no navigation route; movement stopped",
            unit.0
        );
        MotionPhase::Dwelling { remaining: None }
    })
}

/// 贴合支起步：目标位是某条家具挂点时的出发——先原地转到挂点朝向
/// （[`MotionPhase::FitTurning`]），转完进直线插值腿
/// （[`MotionPhase::FitWalking`]，由转体完成帧移交）。已面向目标则直
/// 接起步插值；零行程（已站在挂点上）直接落位收场。
///
/// 当前选段使用正值欧拉 y 差，时长使用最短夹角/60；局部 Fit 的次序与
/// 完整退出生命周期仍未收口。这条局部插值不代替前置导航，也不用于普通
/// CheckPoint/Rest 之间的位移。
fn fit_depart(
    unit: &CharacterUnitId,
    state: &mut LawWalkState,
    slot: &mut NpcPathWalkSlot,
    route: &mut RouteStops,
    fit: FitCandidate,
) -> Option<MotionPhase> {
    // 贴合不走路径：清旧路线与路径槽（源形状：关代理，在途路线作废）。
    route.stops.clear();
    route.next = 0;
    route.goal = None;
    route.fit = None;
    route.reentry = Some(state.position);
    *slot = NpcPathWalkSlot::from_corners(Vec::new());
    state.next_corner = 0;

    let start = state.position;
    let delta = Vec3::from(fit.position) - Vec3::from(start);
    let distance = delta.length();
    let leg = FitLeg {
        target: fit.position,
        start,
        travel: distance / FIT_SPEED,
        move_quat: {
            // 行进朝向 = 面朝水平移动向。竖直分量近零的行程（同一面上
            // 不应出现）退回当前朝向——插值锁会维持它的 yaw。
            let horizontal = Vec3::new(delta.x, 0.0, delta.z);
            if horizontal.length_squared() < 1e-12 {
                Quat::from_rotation_arc(Vec3::Z, Vec3::from(state.forward))
            } else {
                Quat::from_rotation_arc(Vec3::Z, horizontal.normalize())
            }
        },
    };
    info!(
        "[npc unit={}] 贴合起步：目标 ({:.2},{:.2},{:.2})，行程 {:.2}m，直线匀速 {:.1} m/s",
        unit.0, fit.position[0], fit.position[1], fit.position[2], distance, FIT_SPEED
    );

    let attach_forward = fit.rotation * Vec3::Z;
    let heading = angle_between(
        state.forward,
        [attach_forward.x, attach_forward.y, attach_forward.z],
    );
    let duration = rotate_time(heading);
    if duration <= 0.0 {
        if leg.travel <= 0.0 {
            // 已面向且零行程：末帧精确落位语义下即已完成。
            state.position = leg.target;
            return Some(MotionPhase::Dwelling { remaining: None });
        }
        return Some(MotionPhase::FitWalking { leg, elapsed: 0.0 });
    }
    let angle = turn_angle(
        heading_yaw(state.forward),
        heading_yaw([attach_forward.x, attach_forward.y, attach_forward.z]),
    );
    let motion = turn_motion(angle);
    let from = Quat::from_rotation_arc(Vec3::Z, Vec3::from(state.forward));
    let mut to = fit.rotation;
    // 最短角路径：挂点朝向按构造是纯绕 Y 旋转（与 from 同为「Z 到前向」
    // 约定的四元数），点积为负则翻到同叶短弧——与行走前转身同一处理。
    if from.dot(to) < 0.0 {
        to = Quat::from_xyzw(-to.x, -to.y, -to.z, -to.w);
    }
    info!(
        "[npc unit={}] 贴合转身选段 {} 时长 {:.2}s（差角 {:.1}°）",
        unit.0,
        motion.label(),
        duration,
        angle
    );
    Some(MotionPhase::FitTurning {
        motion,
        from,
        to,
        duration,
        elapsed: 0.0,
        leg,
    })
}

/// 锁 yaw-only（源贴合插值的每帧清 pitch/roll）：取朝向前向的水平航向，
/// 重造纯绕 Y 朝向。前向水平分量近零（纯竖直朝向）时原样返回——贴合
/// 链的朝向都从水平面来，这个角实际到不了。
fn yaw_only(rotation: Quat) -> Quat {
    let forward = rotation * Vec3::Z;
    if forward.x * forward.x + forward.z * forward.z < 1e-12 {
        return rotation;
    }
    Quat::from_rotation_y(heading_yaw([forward.x, forward.y, forward.z]).to_radians())
}

/// 两点的水平行进方向（单位向量）。水平分量近零（纯竖直行程）返回
/// [`None`]——调用方各自决定退路。
fn horizontal_direction(start: [f32; 3], target: [f32; 3]) -> Option<[f32; 3]> {
    let dx = target[0] - start[0];
    let dz = target[2] - start[2];
    let length_squared = dx * dx + dz * dz;
    if length_squared < 1e-12 {
        return None;
    }
    let length = length_squared.sqrt();
    Some([dx / length, 0.0, dz / length])
}

/// Called only by the movement producer when it starts a new execution leg.
/// Conversation-script turns never use this entry point.
pub(crate) fn declare_navigation_action(
    actions: &mut NpcActions,
    rest: &mut RestLifecycle,
    phase: &MotionPhase,
    route: &RouteStops,
) {
    match phase {
        MotionPhase::Turning { .. } | MotionPhase::FitTurning { .. } => {
            actions.change(NpcAction::Rotate, rest)
        }
        MotionPhase::Walking | MotionPhase::FitWalking { .. } => {
            actions.change(NpcAction::AutoMove, rest)
        }
        MotionPhase::Dwelling { remaining: Some(_) } => actions.begin_waypoint_rest(rest, route),
        MotionPhase::Dwelling { remaining: None } => actions.change(NpcAction::Idle, rest),
    }
}

/// Update：每帧把每名 NPC 沿其路径槽推进一帧，并驱动移动相位与路点表。
///
/// 普通导航由 path::advance 积分，局部 Fit 由自己的插值相位推进。
/// 每个 CheckPoint 到达后交给下一腿；只有 Rest 点才进入 PauseSeconds 倒数。
/// 同帧跨过多个底层 corner 时逐段校验实际轨迹，不能以首尾弦线替代路径。
/// 行走（含导航终点未能接近原 waypoint）使用 .09m/5秒 watchdog；停止与
/// 正常路线耗尽发布不同结果。真正 Rest 不把等待时长记作卡住。
///
/// 对话入场显式停止移动并保留Stopped结果，持留时本移动写者让位。
/// 目标自己的Rest delay由目标拥有者推进，动作Rest脚本已被出沿Dispose；
/// 不能在撤持留后恢复旧路线/把停止当到达。朝向归正文的转体帧写。
#[allow(clippy::type_complexity)]
pub fn advance(
    time: Res<Time>,
    editor: Res<crate::fixture_edit::EditSessionActive>,
    walk_face: Option<Res<crate::walk_face::WalkFace>>,
    objective_face: Option<Res<crate::npc_objective::ObjectiveFace>>,
    mut npcs: Query<
        (
            &CharacterUnitId,
            &WalkSpeed,
            &PauseSeconds,
            &mut crate::npc_objective::MemberRng,
            &mut PathSlot,
            &mut WalkState,
            &mut RouteStops,
            &mut StuckBaseline,
            &mut Transform,
            &mut MotionPhase,
            Option<&crate::talk::TalkHold>,
            &mut NpcActions,
            &mut RestLifecycle,
        ),
        Without<crate::npc_fixture_activity::NpcFixtureMotionOwner>,
    >,
) {
    if editor.is_active() {
        return;
    }
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for (
        unit,
        speed,
        pause,
        mut rng,
        mut slot,
        mut state,
        mut route,
        mut stuck,
        mut transform,
        mut phase,
        talk_hold,
        mut actions,
        mut rest,
    ) in &mut npcs
    {
        // 外部截断的自愈：收场位（`Dwelling::None`）与非空路点表/路径槽
        // 并存，只可能来自相位被外部改写（对话域对参演者的收场写入）。
        // 本域的状态机里收场位必然伴随空表空槽；这里把残表清掉——被谈话
        // 截断的成员谈话后站定等目标域再判，而不是把旧路线剩下的站走完
        // （源形状：谈话取消移动任务，不是暂停它）。
        if matches!(*phase, MotionPhase::Dwelling { remaining: None })
            && (!route.stops.is_empty() || !slot.0.corners().is_empty())
        {
            route.stop();
            slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
            state.0.next_corner = 0;
            stuck.0 = None;
            info!(
                "[npc unit={}] t={now:.1} 路点表外部截断，清空（对话收场写入）",
                unit.0
            );
        }
        if talk_hold.is_some() || actions.current == NpcAction::Talk {
            continue; // 对话持留：位移推进整名让位（相位与变换都不写）
        }
        let (Some(walk_face), Some(objective_face)) =
            (walk_face.as_deref(), objective_face.as_deref())
        else {
            continue;
        };
        if objective_face.navigation_generation() != walk_face.generation() {
            continue;
        }
        if route.generation != walk_face.generation() {
            if !walk_face.walkable_at([state.0.position[0], state.0.position[2]]) {
                let (x, z) = walk_face.seat(state.0.position[0], state.0.position[2]);
                state.0.position = objective_face
                    .surface_sample(
                        [x, objective_face.ref_y(), z],
                        moly_law::objective::TILE_SCALE,
                    )
                    .unwrap_or([x, state.0.position[1], z]);
                transform.translation = Vec3::from(state.0.position);
            }
            let goal = route.goal;
            let fit = route.fit;
            *route = RouteStops::default();
            slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
            state.0.next_corner = 0;
            stuck.0 = None;
            *phase = if let Some(goal) = goal {
                match depart(
                    unit,
                    &mut state.0,
                    &mut slot.0,
                    &mut route,
                    walk_face,
                    objective_face,
                    goal,
                    fit,
                    &mut rng,
                ) {
                    Some(phase) => phase,
                    None => {
                        // A failed replacement path terminates this move, not
                        // the AI content. Its executor must receive a result
                        // instead of waiting for an arrival that cannot happen.
                        route.stop();
                        warn!(
                            "[npc unit={}] navigation replan has no route; movement stopped",
                            unit.0
                        );
                        MotionPhase::Dwelling { remaining: None }
                    }
                }
            } else {
                MotionPhase::Dwelling { remaining: None }
            };
            route.generation = walk_face.generation();
            declare_navigation_action(&mut actions, &mut rest, &phase, &route);
        }
        // 关闭代理的局部贴合结束后，先恢复已验证的导航接近点再让目标机查询。
        // 原生代理的再附着由本地格场的记录点承担，不能猜一条穿洞的首腿。
        if matches!(*phase, MotionPhase::Dwelling { remaining: None }) {
            if let Some(reentry) = route.reentry.take() {
                if !walk_face.walkable_at([state.0.position[0], state.0.position[2]])
                    && walk_face.walkable_at([reentry[0], reentry[2]])
                {
                    state.0.position = reentry;
                    transform.translation = Vec3::from(reentry);
                }
            }
        }
        let prior = state.0.position;
        let prior_forward = state.0.forward;
        let prior_corner = state.0.next_corner;
        let mut verdict = moly_law::path::advance(&mut state.0, &slot.0, speed.0, dt);
        if matches!(*phase, MotionPhase::Walking) {
            // Validate the travelled polyline, not the chord spanning several
            // valid corners crossed in one frame. A chord can cross a hole even
            // though every actually traversed segment stays on the field.
            let corners = slot.0.corners();
            let begin = (prior_corner as usize).min(corners.len());
            let end = (state.0.next_corner as usize).min(corners.len());
            let mut from = prior;
            let mut valid = true;
            for to in corners[begin..end]
                .iter()
                .copied()
                .chain(std::iter::once(state.0.position))
            {
                valid &= walk_face.segment_walkable([from[0], from[2]], [to[0], to[2]]);
                from = to;
            }
            if !valid {
                state.0.position = prior;
                state.0.forward = prior_forward;
                state.0.next_corner = prior_corner;
                route.generation = 0;
                continue;
            }
            if let WalkVerdict::Walking(_) = verdict {
                if let Some(corner) = corners.get(end).or_else(|| corners.last()) {
                    state.0.forward = facing_direction(state.0.position, *corner);
                }
            }
            if let WalkVerdict::Arrived(_) = verdict {
                let waypoint = route
                    .stops
                    .get(route.next)
                    .expect("active navigation leg retains its waypoint");
                let distance = Vec3::from(waypoint.position).distance(Vec3::from(state.0.position));
                // A sampled endpoint is not proof that the unsnapped waypoint
                // was reached. Keep the movement/watchdog alive when it was not.
                verdict = if distance < ARRIVAL_DISTANCE {
                    WalkVerdict::Arrived(distance)
                } else {
                    WalkVerdict::Walking(0.0)
                };
            }
        }
        // 转体完成帧的旋转：完成时相位已离开 Turning、`forward` 还是旧值，
        // 尾写需要一个不回跳的终点朝向。
        let mut finished_turn: Option<Quat> = None;
        match verdict {
            WalkVerdict::Walking(_) => {
                *phase = MotionPhase::Walking;
                actions.change(NpcAction::AutoMove, &mut rest);
                match stuck.0 {
                    None => stuck.0 = Some((state.0.position, now)),
                    Some((last, since)) => {
                        if Vec3::from(state.0.position).distance(Vec3::from(last)) >= STUCK_DISTANCE
                        {
                            stuck.0 = Some((state.0.position, now));
                        } else if now - since > STUCK_SECONDS {
                            stuck.0 = None;
                            // Terminate only this movement; the objective owns
                            // its next decision and the shared AI content.
                            route.stop();
                            slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
                            state.0.next_corner = 0;
                            *phase = MotionPhase::Dwelling { remaining: None };
                            actions.change(NpcAction::Idle, &mut rest);
                            warn!(
                                "[npc unit={}] t={now:.1} 卡死上报：{:.1}s 位移不足 {:.2}m，路线中止",
                                unit.0,
                                now - since,
                                STUCK_DISTANCE
                            );
                        }
                    }
                }
            }
            WalkVerdict::Arrived(distance) => {
                let waypoint = route.stops[route.next];
                slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
                state.0.next_corner = 0;
                stuck.0 = None;
                match waypoint.kind {
                    WaypointKind::Rest => {
                        let dwell = dwell_seconds(pause.0);
                        *phase = MotionPhase::Dwelling {
                            remaining: Some(dwell),
                        };
                        actions.begin_waypoint_rest(&mut rest, &route);
                        info!("[npc unit={}] t={now:.1} 路点 {}/{} Rest 到站，距 {distance:.3}m，驻留 {dwell:.1}s",
                            unit.0, route.next + 1, route.stops.len());
                    }
                    WaypointKind::CheckPoint => {
                        info!("[npc unit={}] t={now:.1} 路点 {}/{} CheckPoint 通过，距 {distance:.3}m",
                            unit.0, route.next + 1, route.stops.len());
                        *phase = next_waypoint_or_stop(
                            unit,
                            &mut state.0,
                            &mut slot.0,
                            &mut route,
                            walk_face,
                            objective_face,
                        );
                        declare_navigation_action(&mut actions, &mut rest, &phase, &route);
                    }
                }
            }
            WalkVerdict::Idle => match &mut *phase {
                MotionPhase::Turning {
                    to,
                    duration,
                    elapsed,
                    ..
                } => {
                    *elapsed += dt;
                    if *elapsed >= *duration {
                        let end = *to;
                        stuck.0 = None;
                        *phase = start_waypoint(
                            unit,
                            &mut state.0,
                            &mut slot.0,
                            &mut route,
                            walk_face,
                            objective_face,
                        )
                        .unwrap_or_else(|| {
                            route.stop();
                            MotionPhase::Dwelling { remaining: None }
                        });
                        declare_navigation_action(&mut actions, &mut rest, &phase, &route);
                        finished_turn = Some(end);
                    }
                }
                MotionPhase::Dwelling {
                    remaining: Some(remaining),
                } => {
                    *remaining -= dt;
                    if *remaining <= 0.0 {
                        stuck.0 = None;
                        *phase = next_waypoint_or_stop(
                            unit,
                            &mut state.0,
                            &mut slot.0,
                            &mut route,
                            walk_face,
                            objective_face,
                        );
                        declare_navigation_action(&mut actions, &mut rest, &phase, &route);
                    }
                }
                MotionPhase::FitTurning {
                    to,
                    duration,
                    elapsed,
                    leg,
                    ..
                } => {
                    *elapsed += dt;
                    if *elapsed >= *duration {
                        // 贴合转身完成：进直线插值腿（终点朝向先取出再写相位，
                        // 写相位会断开这个借）。零行程的腿跳过插值直接落位。
                        let end = *to;
                        let leg = *leg;
                        if leg.travel <= 0.0 {
                            state.0.position = leg.target;
                            *phase = MotionPhase::Dwelling { remaining: None };
                            route.outcome = Some(RouteOutcome::Arrived);
                            actions.change(NpcAction::Idle, &mut rest);
                            finished_turn = Some(end);
                            info!(
                                "[npc unit={}] t={now:.1} 贴合完成：转身后零行程，落位 ({:.2},{:.2},{:.2})",
                                unit.0, leg.target[0], leg.target[1], leg.target[2]
                            );
                        } else {
                            *phase = MotionPhase::FitWalking { leg, elapsed: 0.0 };
                            actions.change(NpcAction::AutoMove, &mut rest);
                            finished_turn = Some(end);
                            info!(
                                "[npc unit={}] t={now:.1} 贴合转身完成，起步直线插值",
                                unit.0
                            );
                        }
                    }
                }
                MotionPhase::FitWalking { leg, elapsed } => {
                    // 腿先拷出（写相位会断开模式借，后面还要用腿的常量）。
                    let leg = *leg;
                    *elapsed += dt;
                    if *elapsed >= leg.travel {
                        // 末帧精确落位：位置直写目标，forward 换行进方向
                        // （下一轮决策的走前转身从行进末向起算，不回跳）。
                        state.0.position = leg.target;
                        let move_forward =
                            horizontal_direction(leg.start, leg.target).unwrap_or(state.0.forward);
                        state.0.forward = move_forward;
                        stuck.0 = None;
                        *phase = MotionPhase::Dwelling { remaining: None };
                        route.outcome = Some(RouteOutcome::Arrived);
                        actions.change(NpcAction::Idle, &mut rest);
                        info!(
                            "[npc unit={}] t={now:.1} 贴合完成：精确落位 ({:.2},{:.2},{:.2})",
                            unit.0, leg.target[0], leg.target[1], leg.target[2]
                        );
                    } else {
                        // 位置 = 匀速线性插值（t 是时间比例，帧率无关的落位）。
                        let position = leg.sample_position(*elapsed);
                        state.0.position = [position.x, position.y, position.z];
                    }
                }
                // 收场位：路线已尽或未起步，站定的成员由目标机收场与再出发。
                MotionPhase::Dwelling { remaining: None } => {}
                MotionPhase::Walking => {
                    unreachable!("空路径只写在到站帧、卡死帧与转体帧：行走者必在驻留或转体相位")
                }
            },
            WalkVerdict::Refused => {
                // 律的拒绝是响的：状态字节不动，但这里要有人听见。
                warn!(
                    "npc {} 本帧被推进律拒绝（状态保持）：速度 {:.3}，dt {:.3}",
                    unit.0, speed.0, dt
                );
            }
        }
        transform.translation = Vec3::from(state.0.position);
        transform.rotation = match (&*phase, finished_turn) {
            (
                MotionPhase::Turning {
                    from,
                    to,
                    duration,
                    elapsed,
                    ..
                }
                | MotionPhase::FitTurning {
                    from,
                    to,
                    duration,
                    elapsed,
                    ..
                },
                _,
            ) => {
                let t = (elapsed / duration).clamp(0.0, 1.0);
                // 缓动取补间库缺省（转体调用点不设 ease）：先快后慢的
                // 二次出线。`from`/`to` 已是最短弧同叶对，普通球面插值
                // 即最短路径。贴合转身同机制（同一补间族）。
                sample_turn_rotation(*from, *to, t)
            }
            // 转体完成帧的移交朝向（含贴合转身 → 插值腿的交接帧：插值
            // 首帧的「当前」就是它）。
            (_, Some(to)) => to,
            // 贴合插值的朝向律（源 CalcMoveLerp）：当前朝向向行进朝向按
            // t·1.5 插值（t 是腿的时间比例），逐帧锁 yaw-only。「当前」读
            // 上一帧的变换朝向（本帧尚未写）——源读的也是逐帧的当前
            // 变换。四元数 lerp 是线性插值后归一，与引擎 Quaternion.Lerp
            // 同式。
            (MotionPhase::FitWalking { leg, elapsed }, _) => {
                leg.sample_rotation(transform.rotation, *elapsed)
            }
            (_, None) => Quat::from_rotation_arc(Vec3::Z, Vec3::from(state.0.forward)),
        };
    }
}

/// Update（定时）：名册的周期状态行。位置随帧变化是「真的在走」的现算
/// 证据，间隔采样足以从相邻两行推出位移；逐帧打印是刷屏不是证据。相位
/// 让「在走/转体/驻留」与动画侧的换段行可对账；驻留分支带剩余秒数——
/// 路点驻留的计时现值。朝向角是转体连续性的采样证据——相邻行的角差
/// 不超过转体角速度乘采样间隔（60°/s × 2s），瞬跳（单帧换角）会以整角
/// 差出现。`wp` 是路点进度（当前站/总站数，只在路线中显示）。目标与
/// 槽位词让「目标机在动」可从状态行直接读出：obj 是当前目标
/// （talk/nonetalk）、slot 是槽位道别、rest 是步间停顿剩余秒、dest 是
/// 当前目的地。
pub fn report(
    time: Res<Time>,
    npcs: Query<(
        &CharacterUnitId,
        &WalkState,
        &WalkSpeed,
        &MotionPhase,
        &RouteStops,
        &Transform,
        &crate::npc_objective::TalkSlot,
        &crate::npc_objective::ObjectiveMind,
        &MoveTarget,
        &NpcActions,
    )>,
) {
    if !bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
        return;
    }
    for (unit, state, speed, phase, route, transform, slot, mind, target, actions) in &npcs {
        let p = state.0.position;
        let forward = transform.rotation * Vec3::Z;
        let yaw = heading_yaw([forward.x, forward.y, forward.z]);
        let phase_word = match phase {
            MotionPhase::Walking => "walk".to_owned(),
            MotionPhase::Dwelling {
                remaining: Some(remaining),
            } => {
                format!("dwell {:.1}s", remaining)
            }
            MotionPhase::Dwelling { remaining: None } => "dwell".to_owned(),
            MotionPhase::Turning {
                duration, elapsed, ..
            } => {
                format!(
                    "turn {:.0}%",
                    (elapsed / duration * 100.0).clamp(0.0, 100.0)
                )
            }
            MotionPhase::FitTurning {
                duration, elapsed, ..
            } => {
                format!(
                    "fitturn {:.0}%",
                    (elapsed / duration * 100.0).clamp(0.0, 100.0)
                )
            }
            MotionPhase::FitWalking { leg, elapsed } => {
                format!(
                    "fitwalk {:.0}%",
                    (elapsed / leg.travel * 100.0).clamp(0.0, 100.0)
                )
            }
        };
        let route_word = if route.stops.is_empty() {
            String::new()
        } else {
            format!(
                " wp={}/{}",
                (route.next + 1).min(route.stops.len()),
                route.stops.len()
            )
        };
        let obj_word = match mind.current {
            None => "none",
            Some(moly_law::objective::ObjectiveType::Talk) => "talk",
            Some(moly_law::objective::ObjectiveType::NoneTalk) => "nonetalk",
            Some(_) => "other",
        };
        let slot_word = match slot.kind() {
            None => "empty",
            Some(moly_law::objective::TalkType::Common) => "common",
            Some(moly_law::objective::TalkType::CommonFixture) => "fixture",
            Some(moly_law::objective::TalkType::NoneTalk) => "nonetalk",
            Some(_) => "other",
        };
        if let Some(current) = &slot.current {
            debug!("[npc unit={}] action={:?} before={:?} enabled={} current={:?} previous={:?} main={} members={:?} preAction={:?} pending={:?}",
                unit.0, actions.current, actions.before, actions.enable_talk,
                current.content.as_ref().map(|content| content.master_id), slot.previous_id(),
                current.main_character, current.characters, current.pre_action.as_ref().map(|row| row.id), current.pending_factory);
        }
        debug!(
            "npc unit={} t={:.1} pos=({:.3},{:.3},{:.3}) yaw={:.1} speed={:.3} phase={phase_word}{route_word} obj={obj_word} slot={slot_word} rest={:.1} dest=({:.1},{:.1})",
            unit.0,
            time.elapsed_secs(),
            p[0],
            p[1],
            p[2],
            yaw,
            speed.0,
            mind.rest_remaining,
            target.0[0],
            target.0[2],
        );
    }
}

/// 采样点附近的地表高度：半径内地形顶点的最高者；没有顶点时取 `fallback`
/// （玩家域读地表中心的脚下高度用；名册成员的高度走目标面采样）。
fn surface_y(verts: &[Vec3], x: f32, z: f32, fallback: f32) -> f32 {
    verts
        .iter()
        .filter(|v| {
            let dx = v.x - x;
            let dz = v.z - z;
            dx * dx + dz * dz <= SURFACE_RADIUS * SURFACE_RADIUS
        })
        .map(|v| v.y)
        .fold(fallback, f32::max)
}
