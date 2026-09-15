//! Host world-pick mapping for harvest objects, NPCs and fixtures.
//!
//! NPC ray picking is a host input convenience, not the source NPC-button
//! collision implementation. It shares proximity/site/visibility qualification
//! with the existing action stack, then emits PlayerTalkRequest for the single
//! dispatcher. No general fixture-radius gate or story selection lives here.
//! UI consumption, drag exclusion and the existing harvest/fixture paths remain.

use bevy::prelude::*;

use crate::action_button::ActionTapConsumed;
use crate::fixture::{FixturePlacement, FixtureRoot};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::harvest::{HarvestHit, HarvestHits, HarvestObject, HarvestRoot, STATUS_HARVESTED};
use crate::npc::CharacterUnitId;
use crate::player::PlayerControlled;
use crate::player_talk::PlayerTalkRequest;

/// NPC 预留沿的拾取半径（我方选值，非真源——真源走物理碰撞体，胶囊
/// 尺寸未提取；按角色近似身宽取 0.5，具名挂账，提取到尺寸后替换）。
const NPC_PICK_RADIUS: f32 = 0.5;

/// 家具预留沿的拾取半径（同上，按一格摆件的近似半宽取 0.75）。
const FIXTURE_PICK_RADIUS: f32 = 0.75;

/// 冒烟合成的点按节奏（秒）。
const SMOKE_TAP_INTERVAL: f32 = 0.8;

/// 拾取候选（按路程排序前的具名形状）。
enum Candidate<'a> {
    Harvest(Entity, &'a str),
    Npc(Entity, u32),
    Fixture(Entity, i32),
}
/// Update（手势链尾）：TAP 收场沿 + 右键收起沿 → 世界射线 → 三族候选
/// 按路程取最近 → 采集物入被击队 / 预留沿记日志。
#[allow(clippy::type_complexity)]
pub(crate) fn pick(
    mut gestures: MessageReader<GestureEvent>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    harvests: Query<(Entity, &Transform, &HarvestObject), With<HarvestRoot>>,
    npcs: Query<(Entity, &Transform, &CharacterUnitId), Without<PlayerControlled>>,
    fixtures: Query<(Entity, &Transform, &FixturePlacement), With<FixtureRoot>>,
    mut hits: ResMut<HarvestHits>,
    mut talk_requests: MessageWriter<PlayerTalkRequest>,
    consumed: Res<ActionTapConsumed>,
    eligibility: crate::interaction::InteractionEligibility,
) {
    // 读沿先于消费门：被吃掉的点按也要把读数沿推过去——消息缓冲两天
    // 有效，不读的话下一帧这同一次点按会被当新事件重放成一条世界射线
    // （真源里 UI 吃掉的点按，世界射线本来就拿不到那一次——重放违反
    // 同一条次序）。
    let mut taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| {
            event.kind == GestureKind::Tap && event.state == GestureState::End && !event.ui_owned
        })
        .map(|event| event.position)
        .collect();
    // 屏幕按钮先手：接近触发的动作按钮与场地屏外壳都排在本系统之前，
    // 落在按钮上的那一下由它们吃掉。
    if consumed.0 || !eligibility.available() {
        return;
    }
    // 触发沿：手势层的 TAP 收场（真源只认 TAP；双击的第二次与长按都不
    // 拾取）+ 右键收起（PC 映射，取当帧光标位）。
    if buttons.just_released(MouseButton::Right) {
        if let Some(position) = windows
            .single()
            .ok()
            .and_then(|window| window.cursor_position())
        {
            taps.push(position);
        }
    }
    if taps.is_empty() {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return; // 相机没立（站点未取景）：这一帧的点按无处落，丢弃
    };
    for position in taps {
        let Some(ray) = camera.viewport_to_world(camera_transform, position).ok() else {
            info!(
                "[pick] 屏位 ({:.0},{:.0}) 出不了射线（在相机视口外）",
                position.x, position.y
            );
            continue;
        };
        // 三族候选各测命中；已收场的采集物不参与（真源收场即摘碰撞体，
        // 碰撞注册表挂账，这里按状态行等价排除）。
        let mut candidates: Vec<(f32, Candidate)> = Vec::new();
        for (entity, transform, object) in &harvests {
            if object.status == STATUS_HARVESTED {
                continue;
            }
            if let Some(distance) =
                ray_cylinder_entry(&ray, transform.translation.xz(), object.radius)
            {
                candidates.push((distance, Candidate::Harvest(entity, object.leaf.as_str())));
            }
        }
        for (entity, transform, unit) in &npcs {
            if let Some(distance) =
                ray_cylinder_entry(&ray, transform.translation.xz(), NPC_PICK_RADIUS)
            {
                candidates.push((distance, Candidate::Npc(entity, unit.0)));
            }
        }
        for (entity, transform, placement) in &fixtures {
            if let Some(distance) =
                ray_cylinder_entry(&ray, transform.translation.xz(), FIXTURE_PICK_RADIUS)
            {
                candidates.push((distance, Candidate::Fixture(entity, placement.fixture_id)));
            }
        }
        // 真源 RaycastAll 的距离升序 + 首个类型匹配：非候选命中（地面、
        // 建筑网格）不遮挡——它们本来就不在候选集里。
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("路程没有 NaN"));
        match candidates.first() {
            Some((distance, Candidate::Harvest(entity, leaf))) => {
                hits.0.push(HarvestHit {
                    target: *entity,
                    damage: 1,
                    tool_level: 0,
                    is_boost: false,
                });
                info!(
                    "[pick] 点按 ({:.0},{:.0}) → 射线 {distance:.2}m 命中采集物 {leaf}（空手 damage 1 入队）",
                    position.x, position.y
                );
            }
            Some((_, Candidate::Npc(entity, index))) => {
                if !eligibility.allows(*entity, *index) {
                    continue;
                }
                // This host mapping shares appearance qualification with the
                // proximity button. The dispatcher owns safe positioning and
                // click-time state checks; no nearby fixture selects a script.
                talk_requests.write(PlayerTalkRequest {
                    entity: *entity,
                    unit: *index,
                    exact: None,
                    target_fixture: None,
                });
                info!(
                    "[pick] 点按 ({:.0},{:.0}) → 命中 NPC {entity:?}（unit {}）→ 玩家对话请求入队",
                    position.x, position.y, index
                );
            }
            Some((_, Candidate::Fixture(entity, fixture_id))) => {
                info!(
                    "[pick] 点按 ({:.0},{:.0}) → 命中家具 {fixture_id}（{entity:?}）——预留沿：家具交互域未接",
                    position.x, position.y
                );
            }
            None => {
                info!(
                    "[pick] 点按 ({:.0},{:.0}) → 射线未命中任何候选（真源不命中即无事）",
                    position.x, position.y
                );
            }
        }
    }
}

/// Update（手势链内、拾取前）：冒烟口（`MOLY_PICK_AUTOTAP_SECS`，同仓
/// player/harvest 仪表同款）——窗口期内每隔 [`SMOKE_TAP_INTERVAL`] 秒
/// 对一个在场采集物合成一次 TAP：把它的世界位投影到屏面、按该屏位
/// 发布手势事件，走与真实输入**同一条**拾取链（投影 → 射线 → 圆柱
/// 测试 → 入队 → 被击处理）。事件面注入：绕过手势层内部账本（账本只
/// 服务连击/长按判定，冒烟不需要）；多击族目标也只点一下——链路验证
/// 而已，收场脚本归采集物域自己的仪表。
pub(crate) fn smoke_autotap(
    mut events: MessageWriter<GestureEvent>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    harvests: Query<(Entity, &Transform, &HarvestObject), With<HarvestRoot>>,
    time: Res<Time>,
    mut next_at: Local<f32>,
    mut index: Local<usize>,
) {
    let armed = env_secs("MOLY_PICK_AUTOTAP_SECS");
    if armed <= 0.0 || time.elapsed_secs() >= armed || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + SMOKE_TAP_INTERVAL;
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    // 在场目标按实体序（≈摆放表序）排定，循环点名。
    let mut targets: Vec<(Entity, Vec3, &str)> = harvests
        .iter()
        .filter(|(_, _, object)| object.status != STATUS_HARVESTED)
        .map(|(entity, transform, object)| (entity, transform.translation, object.leaf.as_str()))
        .collect();
    targets.sort_by(|a, b| a.0.cmp(&b.0));
    if targets.is_empty() {
        return; // 还没有在场目标：等下一拍
    }
    let Some((_, world, leaf)) = targets.get(*index % targets.len()) else {
        return;
    };
    *index += 1;
    let Some(position) = camera.world_to_viewport(camera_transform, *world).ok() else {
        info!("[pick-smoke] {} 的世界位投影失败（相机视口外），跳过", leaf);
        return;
    };
    events.write(GestureEvent {
        kind: GestureKind::Tap,
        state: GestureState::End,
        position,
        delta: Vec2::ZERO,
        ui_owned: false,
    });
    info!(
        "[pick-smoke] 合成 TAP @({:.0},{:.0})（投影 {} 的世界位，事件面注入）",
        position.x, position.y, leaf
    );
}

/// Update（手势链内、拾取前）：NPC 臂冒烟口（`MOLY_PICK_NPC_TAP_SECS`）
/// ——窗口期内每隔 [`SMOKE_TAP_INTERVAL`] 秒把一个在场角色的世界位投影
/// 到屏面、按该屏位合成一次 TAP，走与真实输入**同一条**拾取链（投影
/// → 射线 → 圆柱测试 → 配对门 → 入队/拒绝）。与采集物臂同款的事件面
/// 注入；目标按实体序循环点名——名册里配对与未配对混居，门的两列
/// 样本（拒绝/放行）都会自然攒出来。投影屏位恰好压在对话按钮上时
/// 那一拍会被按钮层吃掉，算无样本。
pub(crate) fn smoke_npc_autotap(
    mut events: MessageWriter<GestureEvent>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    npcs: Query<(Entity, &Transform, &CharacterUnitId), Without<PlayerControlled>>,
    time: Res<Time>,
    mut next_at: Local<f32>,
    mut index: Local<usize>,
) {
    let armed = env_secs("MOLY_PICK_NPC_TAP_SECS");
    if armed <= 0.0 || time.elapsed_secs() >= armed || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + SMOKE_TAP_INTERVAL;
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    // 名册按实体序排定，循环点名。
    let mut targets: Vec<(Entity, Vec3, u32)> = npcs
        .iter()
        .map(|(entity, transform, unit)| (entity, transform.translation, unit.0))
        .collect();
    targets.sort_by(|a, b| a.0.cmp(&b.0));
    if targets.is_empty() {
        return; // 还没有在场角色：等下一拍
    }
    let Some((_, world, unit)) = targets.get(*index % targets.len()) else {
        return;
    };
    *index += 1;
    let Some(position) = camera.world_to_viewport(camera_transform, *world).ok() else {
        info!("[pick-smoke] unit {unit} 的世界位投影失败（相机视口外），跳过");
        return;
    };
    events.write(GestureEvent {
        kind: GestureKind::Tap,
        state: GestureState::End,
        position,
        delta: Vec2::ZERO,
        ui_owned: false,
    });
    info!(
        "[pick-smoke] 合成 TAP @({:.0},{:.0})（投影 unit {unit} 的世界位，事件面注入）",
        position.x, position.y
    );
}

/// 射线对竖直圆柱（XZ 圆）的入口路程：解 |(o + t·d)_xz − c|² = r² 的
/// 最小非负根。d 是三维单位方向，其 XZ 投影自然不大于 1——t 仍是沿
/// 射线的真实路程（与真源 RaycastHit.distance 同口径）。起点在圆柱内
/// 视作路程 0（最近）；射线竖直（XZ 无前进量）按未命中处理。
fn ray_cylinder_entry(ray: &Ray3d, center: Vec2, radius: f32) -> Option<f32> {
    let offset = Vec2::new(ray.origin.x - center.x, ray.origin.z - center.y);
    let dir = Vec2::new(ray.direction.x, ray.direction.z);
    let a = dir.dot(dir);
    if a <= f32::EPSILON {
        return None;
    }
    let b = 2.0 * offset.dot(dir);
    let c = offset.dot(offset) - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let near = (-b - root) / (2.0 * a);
    let far = (-b + root) / (2.0 * a);
    if near >= 0.0 {
        Some(near)
    } else if far >= 0.0 {
        Some(0.0)
    } else {
        None
    }
}

/// 环境变量秒数（缺省 0）：与各域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
