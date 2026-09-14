//! 折线推进的纯数学：一帧预算内沿拐点走、帧首判到达、朝向读固定拐点。
//!
//! 逐 NPC 传参——没有行间数组，也就没有「第 i 行的持久状态缺一列」这种
//! 错位可发生。状态里的三样（位置、朝向、拐点进度）由调用方跨帧携带。

use super::{
    ARRIVAL_DISTANCE, DIRECTION_EPSILON, FORWARD_FALLBACK, NpcPathWalkSlot,
};

/// 一个行走 NPC 的跨帧状态：位置、朝向、拐点进度。
///
/// 位置与朝向用 `[f32; 3]` 裸数组——引擎的 Transform 归调用方翻译，
/// 本律不碰引擎类型。
#[derive(Debug, Clone, PartialEq)]
pub struct WalkState {
    /// 当前位置。调用方播种一次，此后只有 [`advance`] 推进它。
    pub position: [f32; 3],
    /// 行走中的朝向。不走的帧（到站、无路径、被拒）保持原样——真源的
    /// 朝向代码在代理停下时提前返回，同样是「不走就不转头」。
    pub forward: [f32; 3],
    /// 下一个尚未走到的拐点下标。这是跨帧进度记忆：已走过的拐点永不重走，
    /// 否则半路的行走者会每帧掉头去够第一个拐点。进度必须活在状态里而不
    /// 是输入里——真源一次移动只发一次目的地，路径不逐帧重喂。注意朝向
    /// 读的固定下标**不**随它前进，那是刻意保留的怪癖（见族注释）。
    pub next_corner: u32,
}

impl WalkState {
    /// 从播种位置与初始朝向起算，拐点进度归零。
    pub fn new(position: [f32; 3], forward: [f32; 3]) -> Self {
        Self {
            position,
            forward,
            next_corner: 0,
        }
    }
}

/// 一帧推进的裁决。三种「没走」的情形在类型上分开，避免用三个布尔
/// 互相猜：拒绝时状态字节不动，到站是帧首判定，无路径是常态不是错误。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WalkVerdict {
    /// 本帧在走：携带走完后沿路径的剩余距离。
    Walking(f32),
    /// 帧首已在到达阈内：原地保持，携带到末拐点的直线距离。
    Arrived(f32),
    /// 无活动路径：距离未知。
    Idle,
    /// 拒绝：输入带了真源产不出的值，状态字节不动，距离未知。
    Refused,
}

impl WalkVerdict {
    /// 沿路径的剩余距离；未知时为无穷，与引擎侧属性在距离未知时的读数
    /// 一致。
    pub fn remaining_distance(self) -> f32 {
        match self {
            WalkVerdict::Walking(d) | WalkVerdict::Arrived(d) => d,
            WalkVerdict::Idle | WalkVerdict::Refused => f32::INFINITY,
        }
    }

    /// 帧首到达判定。
    pub fn arrived_this_frame(self) -> bool {
        matches!(self, WalkVerdict::Arrived(_))
    }

    /// 本帧是否被拒绝。
    pub fn rejected(self) -> bool {
        matches!(self, WalkVerdict::Refused)
    }
}

/// 推进一个 NPC 一帧。
///
/// `path` 是该 NPC 的拐点行（不含自身位置，见 [`NpcPathWalkSlot`]）；
/// `speed` 是所选速度（单位/秒）；`dt` 是帧长（秒），显式入参——
/// 本帧预算为 `speed * dt`。行走中朝向更新为指向行的第 0 拐点。
pub fn advance(state: &mut WalkState, path: &NpcPathWalkSlot, speed: f32, dt: f32) -> WalkVerdict {
    let corners = path.corners();

    // `!(x >= 0.0)` 一个谓词同时抓负值与 NaN：NaN 与一切比较为假，混不过
    // `>=`。dt 的门槛与速度同款：真源的帧步长由整型微秒折算，负与非数
    // 无从发生；放行 NaN 会把它直接写进位置，毁掉此后所有帧的确定性。
    if !(speed >= 0.0)
        || !(dt >= 0.0)
        || !all_finite(state.position)
        || !all_finite(state.forward)
        || corners.iter().any(|&c| !all_finite(c))
    {
        return WalkVerdict::Refused;
    }

    // 无活动路径：不走、不转头、距离未知。这是常态，不是拒绝。
    if corners.is_empty() {
        return WalkVerdict::Idle;
    }

    // 到达是帧首判定，对齐真源循环「先判断、后让帧」的结构：帧中才进带
    // 的行走者本帧仍报行走，下一帧才报到站。进带后保持不动——循环已经
    // 跳出，带内最终停在哪在 extern 墙后，保持是诚实答案，残差按构造
    // 留在到达距离之内。
    let distance_to_end = distance_to_last(state.position, corners);
    if distance_to_end < ARRIVAL_DISTANCE {
        return WalkVerdict::Arrived(distance_to_end);
    }

    let budget = speed * dt;

    // 只走尚未消耗的拐点。拐点进度挡住回头路：半路的行走者不会每帧掉头
    // 去重走已过的拐点。（下面的朝向刻意仍指向行的第 0 拐点——那个固定
    // 下标的读法是真源自己的，由测试钉住，别顺手「修好」它。）
    let start = (state.next_corner as usize).min(corners.len());
    let tail = &corners[start..];
    let mut consumed = 0usize;
    walk_along(&mut state.position, tail, budget, &mut consumed);
    state.next_corner = (start + consumed) as u32;
    state.forward = facing_direction(state.position, corners[0]);

    WalkVerdict::Walking(remaining_distance(
        state.position,
        &corners[start + consumed..],
    ))
}

/// 从 `position` 出发按序穿过 `corners` 的折线总长。
fn remaining_distance(position: [f32; 3], corners: &[[f32; 3]]) -> f32 {
    let mut total = 0.0f32;
    let mut prev = position;
    for &corner in corners {
        let dx = corner[0] - prev[0];
        let dy = corner[1] - prev[1];
        let dz = corner[2] - prev[2];
        total += (dx * dx + dy * dy + dz * dz).sqrt();
        prev = corner;
    }
    total
}

/// `position` 到最后一个拐点的直线距离——真源到达判比的就是这个量。
/// 调用方保证 `corners` 非空。
fn distance_to_last(position: [f32; 3], corners: &[[f32; 3]]) -> f32 {
    let last = corners[corners.len() - 1];
    let dx = last[0] - position[0];
    let dy = last[1] - position[1];
    let dz = last[2] - position[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// 沿折线 `position -> corners[0] -> ...` 推进 `position`，至多 `budget`
/// 个世界单位；整段被预算覆盖的拐点记进 `consumed`。从不过冲：预算盖过
/// 剩余路径时，位置恰好落在末拐点上（一次赋值，与拐点字节一致）。整段
/// 被覆盖的中间拐点同样按赋值到站——一步恰好踩到（或跨过）拐点的帧，
/// 触到的是精确值，调用方的拐点进度随之越过它。
fn walk_along(position: &mut [f32; 3], corners: &[[f32; 3]], budget: f32, consumed: &mut usize) {
    let mut remaining = budget;
    for (k, &corner) in corners.iter().enumerate() {
        if remaining <= 0.0 {
            return;
        }
        let dx = corner[0] - position[0];
        let dy = corner[1] - position[1];
        let dz = corner[2] - position[2];
        let seg = (dx * dx + dy * dy + dz * dz).sqrt();
        if seg <= remaining {
            remaining -= seg;
            *position = corner;
            *consumed = k + 1;
        } else {
            // 走到这里时 `remaining < seg` 且 `seg > 0`（零长段走上分支），
            // 所以 `t` 落在 `[0, 1)`，不会除零。被逼近的拐点不消耗：这一步
            // 停在它跟前，下一帧朝它继续。
            let t = remaining / seg;
            position[0] += dx * t;
            position[1] += dy * t;
            position[2] += dz * t;
            return;
        }
    }
}

/// 本帧的朝向：指向给定拐点并归一化；拐点在（或在使用方阈内的）自身位置
/// 上时取回退。拐点下标是固定的，与真源一致：站上首拐点的行走者面对
/// 回退方向而非下一个拐点，越过首拐点的行走者倒退着面向它。
///
/// 公开给换腿帧的出发决策用：转身的目标方向必须与行走律喂腿首帧
/// 算出的朝向**同源**——同一个函数、同一对输入，转身结束移交行走时
/// 朝向才不跳。
pub fn facing_direction(position: [f32; 3], corner: [f32; 3]) -> [f32; 3] {
    let dx = corner[0] - position[0];
    let dy = corner[1] - position[1];
    let dz = corner[2] - position[2];
    let mag = (dx * dx + dy * dy + dz * dz).sqrt();
    if mag <= DIRECTION_EPSILON {
        FORWARD_FALLBACK
    } else {
        [dx / mag, dy / mag, dz / mag]
    }
}

/// 三个分量是否全为有限数。输入与持久状态同门槛：任何一处进一个非有限
/// 值，都会穿过后面的每一帧，毁掉字节级确定性，所以在门口拒绝。
fn all_finite(v: [f32; 3]) -> bool {
    v[0].is_finite() && v[1].is_finite() && v[2].is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本文件的测试速度刻意都不取 1.0：速度恰为 1.0 时「走了 speed*dt」
    /// 与「走了 dt」不可分辨，测试绿了也不知道量的是哪一个。0.4 是真源
    /// 为首个角色解出的表值步行速度；其余任取，但保持同一性质。
    const WALK_SPEED: f32 = 0.4;
    const RUN_SPEED: f32 = 0.7;

    fn walker(
        position: [f32; 3],
        forward: [f32; 3],
        corners: Vec<[f32; 3]>,
    ) -> (WalkState, NpcPathWalkSlot) {
        (
            WalkState::new(position, forward),
            NpcPathWalkSlot::from_corners(corners),
        )
    }

    // ---- 正臂先行：走必须真的走动 --------------------------------------

    #[test]
    fn a_walk_moves_the_walker_forward_by_speed_times_dt() {
        // 锚：代理的 speed 语义是最大移动速度，真源起步写入所选速度。
        // 0.4 单位/秒走半秒必须沿路径走 0.2——且必须走动，这是正臂：
        // 若推进积分了空集，本文件所有断言都会空转着绿。
        let (mut st, path) = walker([0.0; 3], [0.0, 0.0, 1.0], vec![[10.0, 0.0, 0.0]]);
        let v = advance(&mut st, &path, WALK_SPEED, 0.5);
        let x = st.position[0];
        assert!(x > 0.0, "walker did not move: x = {x}");
        assert!((x - 0.2).abs() < 1e-5, "moved {x}, expected 0.4 * 0.5 = 0.2");
        // 走的是路径，不是飘进空中：y 与 z 保持精确。
        assert_eq!(st.position[1], 0.0);
        assert_eq!(st.position[2], 0.0);
        // 走完 0.2，沿路径剩 9.8（与逐值锚一致）。
        assert_eq!(v.remaining_distance(), 9.8);
    }

    // ---- 成对：dt = 0 必须一字节不差地留在起点 -------------------------

    #[test]
    fn zero_dt_leaves_the_position_byte_identical_to_the_start() {
        // 锚：零长步进把位置推进 speed * 0 = 0；其余都是发明。取字节一致
        // 而非近似相等：一点近零漂移就已经是编造的一步。
        let start = [3.5, -2.25, 7.125];
        let (mut st, path) = walker(start, [0.0, 0.0, 1.0], vec![[10.0, 0.0, 0.0]]);
        advance(&mut st, &path, WALK_SPEED, 0.0);
        assert_eq!(st.position, start);
    }

    // ---- 成对：同一时刻，速度翻倍路程翻倍 ------------------------------

    #[test]
    fn doubling_the_speed_doubles_the_distance_walked() {
        // 锚：speed 按比例缩放推进。同路径同时长半秒、速度 0.4 与 0.7：
        // 后者的路程必须是前者的 0.7/0.4 倍——同时是绝对锚 0.7 * 0.5 = 0.35。
        // 两个速度都不是 1.0，所以能区分「乘了速度」与「根本没乘」。
        let (mut slow, path) = walker([0.0; 3], [0.0; 3], vec![[100.0, 0.0, 0.0]]);
        let (mut fast, path2) = walker([0.0; 3], [0.0; 3], vec![[100.0, 0.0, 0.0]]);
        advance(&mut slow, &path, WALK_SPEED, 0.5);
        advance(&mut fast, &path2, RUN_SPEED, 0.5);
        let slow_dx = slow.position[0];
        let fast_dx = fast.position[0];
        assert!((slow_dx - 0.2).abs() < 1e-5, "walk speed produced {slow_dx}");
        assert!((fast_dx - 0.35).abs() < 1e-5, "run speed produced {fast_dx}");
        assert!((fast_dx / slow_dx - RUN_SPEED / WALK_SPEED).abs() < 1e-4);
    }

    // ---- 成对：零速即原地 ----------------------------------------------

    #[test]
    fn zero_speed_holds_the_position_at_every_dt() {
        // 锚：真源的停机流程把代理速度清零，所以零是真源真的产得出的速度，
        // 任何 dt 都必须哪儿也不去。取多个帧长，「任何 dt」才不只是 dt 一例。
        let start = [1.0, 2.0, 3.0];
        let (mut st, path) = walker(start, [0.0, 0.0, 1.0], vec![[10.0, 0.0, 0.0]]);
        for dt in [0.0f32, 0.001, 0.5, 4.0] {
            advance(&mut st, &path, 0.0, dt);
            assert_eq!(st.position, start, "moved at dt {dt}");
        }
    }

    // ---- 成对：走完全程必须落在终点 ------------------------------------

    #[test]
    fn walking_the_full_path_lands_on_the_endpoint_and_reports_the_residual() {
        // 锚：到达阈是移动循环自己的比较（到目标直线距低于 0.1 即跳出），
        // 走得够久必须收进带内。速度 2.0、四分之一秒帧、每帧 0.5：十六帧
        // 盖完到拐点的八个单位，再下一帧报到站。残差是报出来的不是假设的：
        // 十六步插值累积的舍入按构造远低于到达带，断言把它钉在那——过冲
        // 或欠走的积分器给不出这么小的残差。
        let (mut st, path) = walker([0.0; 3], [0.0; 3], vec![[8.0, 0.0, 0.0]]);
        let mut v = WalkVerdict::Idle;
        for _ in 0..16 {
            v = advance(&mut st, &path, 2.0, 0.25);
        }
        // 第十六帧从半单位处起步：仍在走。
        assert!(!v.arrived_this_frame());
        // 第十七帧从约零处起步：它是到站帧。
        let v = advance(&mut st, &path, 2.0, 0.25);
        assert!(v.arrived_this_frame());
        let residual = remaining_distance(st.position, &[[8.0, 0.0, 0.0]]);
        assert!(residual < 1e-4, "endpoint residual {residual}");
        assert!(v.remaining_distance() < 1e-4);
    }

    #[test]
    fn one_frame_whose_step_covers_the_whole_path_lands_byte_exact_on_the_endpoint() {
        // 同一条路，一帧走完：预算（2.0 * 4.0s = 8.0）盖过八个单位的全程，
        // 落点走上赋值分支——与拐点字节一致、零残差，到站在下一帧报到
        // （判定是帧首检查）。这是终点成对测试的字节精确半边；上一条
        // 逐帧走是累积舍入半边。
        let (mut st, path) = walker([0.0; 3], [0.0; 3], vec![[8.0, 0.0, 0.0]]);
        advance(&mut st, &path, 2.0, 4.0);
        assert_eq!(st.position, [8.0, 0.0, 0.0]);
        let v = advance(&mut st, &path, 2.0, 0.25);
        assert!(v.arrived_this_frame());
    }

    #[test]
    fn arrival_fires_inside_the_band_and_not_just_outside_it() {
        // 锚：阈值本身 0.1——移动循环的字面量。从目标 0.0625 处（0.1 之内）
        // 起步的行走者必须在第一帧报到达，同时位置字节不动。从 0.5 处以
        // 精确的二的幂步长（每帧 0.125）逼近的行走者，站在 0.125 处的那帧
        // 必须仍在走——0.125 在 0.1 之外——所以若阈值误取真源的另一处
        // 字面量 0.3（自动刹车开关），此测试即红。每个距离都是二的幂，
        // 没有一步丢位。
        let (mut inside, path) = walker([0.0; 3], [0.0; 3], vec![[0.0625, 0.0, 0.0]]);
        let v = advance(&mut inside, &path, 2.0, 0.25);
        assert!(v.arrived_this_frame());
        assert_eq!(inside.position, [0.0; 3]);
        assert_eq!(v.remaining_distance(), 0.0625);

        // 速度 0.5、四分之一秒：每帧 0.125。
        let (mut outside, path) = walker([0.0; 3], [0.0; 3], vec![[0.5, 0.0, 0.0]]);
        for frames in 1..=3 {
            let v = advance(&mut outside, &path, 0.5, 0.25);
            assert!(!v.arrived_this_frame(), "arrived at frame {frames}");
        }
        // 第四帧从 0.125 处起步，恰好盖完。
        let v = advance(&mut outside, &path, 0.5, 0.25);
        assert!(!v.arrived_this_frame());
        assert!((outside.position[0] - 0.5).abs() < 1e-5);
        let v = advance(&mut outside, &path, 0.5, 0.25);
        assert!(v.arrived_this_frame());
    }

    #[test]
    fn an_arrived_walker_never_moves_again_and_holds_its_facing() {
        // 锚：移动循环已经跳出（到达在真源里就是这个意思），而真源的朝向
        // 代码在代理停下时提前返回——所以进带后位置与朝向都不得再变。
        // 十七帧把行走者送上终点（十六帧走、一帧察觉）；其后每一帧必须
        // 什么都不改。
        let (mut st, path) = walker([0.0; 3], [0.0; 3], vec![[8.0, 0.0, 0.0]]);
        for _ in 0..17 {
            advance(&mut st, &path, 2.0, 0.25);
        }
        let held_position = st.position;
        let held_facing = st.forward;
        advance(&mut st, &path, 2.0, 0.25);
        advance(&mut st, &path, 2.0, 4.0);
        assert_eq!(st.position, held_position);
        assert_eq!(st.forward, held_facing);
        assert!((held_position[0] - 8.0).abs() < 1e-4);
    }

    // ---- 拐点：路径必须真的被读 ----------------------------------------

    #[test]
    fn a_corner_whose_segment_the_step_covers_is_passed_byte_exact() {
        // 锚：remainingDistance 沿当前路径计量，真源的 steeringTarget 就是
        // 下一个拐点——走是一段一段走的，一步够到的拐点必须被**触到**，
        // 不是被近似。速度 2.0 走半秒是 1.0 的预算；首拐点恰在 1.0 外，
        // 该帧落在赋值分支：与拐点字节一致，掠过偏差报出来恰是 0.0。
        let (mut st, path) = walker(
            [0.0; 3],
            [0.0; 3],
            vec![[1.0, 0.0, 0.0], [1.0, 2.5, 0.0]],
        );
        let v = advance(&mut st, &path, 2.0, 0.5);
        let deviation = remaining_distance(st.position, &[[1.0, 0.0, 0.0]]);
        assert_eq!(deviation, 0.0, "passed the first corner {deviation} units away");
        assert_eq!(st.position, [1.0, 0.0, 0.0]);
        assert!(!v.arrived_this_frame());
        // 下一腿继续走，然后在到达带内到站（带 0.1 是真源循环自己停下的
        // 地方——残差报出来并钉在带下）。
        for _ in 0..4 {
            advance(&mut st, &path, 2.0, 0.5);
        }
        let v = advance(&mut st, &path, 2.0, 0.5);
        assert!(v.arrived_this_frame());
        let residual = remaining_distance(st.position, &[[1.0, 2.5, 0.0]]);
        assert!(residual < 0.1, "endpoint residual {residual} must sit inside the band");
    }

    #[test]
    fn every_sampled_position_lies_on_the_polyline_between_corners() {
        // 「真的沿拐点走」的另一半：步长跨过拐点而非恰好落在其上时，每个
        // 采样位置仍必须精确落在折线的某一段上。L 形路径先走 x 再走 y，
        // 于是第一腿上 y 必须恰为 0、第二腿上 x 必须恰为公共拐点的 x——
        // 都由构造保证（零分量乘步进比例仍是零；拐点本身按赋值到站）。
        // 全程最大偏差：0.0，作为断言报出。
        let (mut st, path) = walker(
            [0.0; 3],
            [0.0; 3],
            vec![[2.0, 0.0, 0.0], [2.0, 3.0, 0.0]],
        );
        let mut worst = 0.0_f32;
        let mut v = WalkVerdict::Idle;
        for _ in 0..44 {
            v = advance(&mut st, &path, 0.5, 0.25);
            assert!(!v.rejected(), "walker refused");
            let [x, y, _] = st.position;
            // 到折线的偏差：第一腿上 y 恰为 0（分量乘步进比例仍是 0），
            // 第二腿上 x 恰为公共拐点的 x（同理）。其余都算离开折线。
            let on_path = if y == 0.0 { 0.0 } else { (x - 2.0).abs() };
            worst = worst.max(on_path);
        }
        assert_eq!(worst, 0.0, "trajectory left the polyline by {worst}");
        // 四十四帧的 0.125 盖完五单位的 L 全程——上面的采样走完了整条路，
        // 且最后一帧的判定是到站。
        assert!(v.arrived_this_frame());
    }

    #[test]
    fn moving_a_corner_moves_the_trajectory_at_the_same_moment() {
        // 「路径被读、不是被假设」的锚对：同起点、同速度、同步数，只挪一个
        // 中间拐点。五帧的 0.5 把两个行走者在第四帧送上同一个首拐点；第五帧
        // 是被挪拐点第一次起作用的帧——轨迹从那里分岔。若路径没被真读，
        // 两个行走者会在此重合。
        let start = [0.0; 3];
        let (mut a, path_a) = walker(start, [0.0; 3], vec![[2.0, 0.0, 0.0], [2.0, 3.0, 0.0]]);
        let (mut b, path_b) = walker(start, [0.0; 3], vec![[2.0, 0.0, 0.0], [5.0, 0.0, 0.0]]);
        for _ in 0..5 {
            advance(&mut a, &path_a, 2.0, 0.25);
            advance(&mut b, &path_b, 2.0, 0.25);
        }
        assert_ne!(a.position, b.position);
        // 各自在第二腿上：a 的拐点把它拉向 y 轴，b 的沿 x 轴拉。
        assert_eq!(a.position[0], 2.0);
        assert_eq!(b.position[1], 0.0);
    }

    #[test]
    fn corner_progress_is_memorized_across_frames_and_never_rewalks() {
        // 拐点进度是跨帧记忆：首拐点被消耗后，后续帧从下一腿继续、不再
        // 重走。从首拐点正后方出发、首拐点在身后的行走者若不记进度，会
        // 每帧掉头；这里钉住它一往无前。
        let (mut st, path) = walker(
            [0.0; 3],
            [0.0; 3],
            vec![[1.0, 0.0, 0.0], [1.0, 3.0, 0.0]],
        );
        advance(&mut st, &path, 2.0, 0.5); // 恰好踩上首拐点，进度记 1
        assert_eq!(st.next_corner, 1);
        advance(&mut st, &path, 2.0, 0.5);
        // 第二帧后位置在首拐点上方 1.0，而不是掉回原点方向。
        assert_eq!(st.position, [1.0, 1.0, 0.0]);
        assert_eq!(st.next_corner, 1);
    }

    // ---- 朝向 -----------------------------------------------------------

    #[test]
    fn while_walking_the_walker_faces_the_first_corner_of_its_row() {
        // 锚：真源朝向代码读固定的一位拐点、从当前位置归一化指向它。原点
        // 处朝 (3, 4, 0)（长度 5）走的行走者必须面向 [0.6, 0.8, 0]。改指
        // 末拐点、或不归一化，落点都不同。
        let (mut st, path) = walker([0.0; 3], [0.0; 3], vec![[3.0, 4.0, 0.0]]);
        advance(&mut st, &path, WALK_SPEED, 0.25);
        let f = st.forward;
        assert!(
            (f[0] - 0.6).abs() < 1e-5 && (f[1] - 0.8).abs() < 1e-5,
            "facing {f:?}"
        );
        assert_eq!(f[2], 0.0);
    }

    #[test]
    fn a_walker_standing_on_its_first_corner_takes_the_unit_forward_fallback() {
        // 锚：真源在目标拐点距自身不足 1e-05 时换用单位前向，而不是除以
        // 近零模长——且它读的是固定下标，所以恰好站上首拐点的行走者取
        // 回退，哪怕前方还有第二个拐点。这里的落点是可证的：拐点在 0.5 外，
        // 步长恰为 0.5（速度 2.0、四分之一秒），赋值分支生效，帧末行走者
        // 逐字节站在拐点上。
        let (mut st, path) = walker(
            [0.0; 3],
            [9.0, 9.0, 9.0],
            vec![[0.5, 0.0, 0.0], [0.5, 5.0, 0.0]],
        );
        let v = advance(&mut st, &path, 2.0, 0.25);
        assert_eq!(st.position, [0.5, 0.0, 0.0]);
        assert_eq!(st.forward, [0.0, 0.0, 1.0]);
        assert!(!v.arrived_this_frame());
    }

    #[test]
    fn a_walker_past_its_first_corner_faces_back_at_it_the_fixed_index_demands() {
        // 固定下标规则的另一半，钉住它免得有人替真源「修好」：从第二帧起，
        // 行走者已经越过了朝向代码读的那位拐点（下标从不前进），可读规则
        // 于是让它沿路径**倒着**面向。真源朝向的其余部分（原生旋转、避障
        // 门控）在墙后；本律复算的是可读的 look-at，怪癖包括在内。
        let (mut st, path) = walker(
            [0.0; 3],
            [0.0; 3],
            vec![[0.5, 0.0, 0.0], [0.5, 5.0, 0.0]],
        );
        advance(&mut st, &path, 2.0, 0.25); // 踩上首拐点，取回退朝向
        advance(&mut st, &path, 2.0, 0.25); // 沿第二腿越过它
        let f = st.forward;
        assert_eq!(f[0], 0.0);
        assert_eq!(f[2], 0.0);
        assert!(f[1] < -0.99, "expected to face back at the corner, got {f:?}");
    }

    // ---- 无路径与响亮拒绝 ------------------------------------------------

    #[test]
    fn an_empty_path_is_idle_not_rejected_and_touches_nothing() {
        // 锚：引擎侧剩余距离属性在距离未知时报无穷；没有活动路径的 NPC 是
        // 常态，行合法、什么都不动、拒绝标记保持 false。
        let start = [1.0, 1.0, 1.0];
        let facing0 = [0.0, 0.0, 1.0];
        let (mut st, path) = walker(start, facing0, Vec::new());
        let v = advance(&mut st, &path, WALK_SPEED, 0.5);
        assert_eq!(st.position, start);
        assert_eq!(st.forward, facing0);
        assert_eq!(v, WalkVerdict::Idle);
        assert!(!v.rejected());
        assert!(!v.arrived_this_frame());
        assert_eq!(v.remaining_distance(), f32::INFINITY);
    }

    #[test]
    fn a_negative_speed_refuses_the_slot_without_touching_its_state() {
        // 锚：真源的速度写入只产得出零（停机清零）或非负的表值；负速度
        // 无从发生，故没有可读语义可供积分。拒绝是响的（裁决位、未知距离、
        // 不给到达），状态字节不动——唯绝不允许的结局是静默猜测。
        let start = [1.0, 2.0, 3.0];
        let (mut st, path) = walker(start, [0.0, 0.0, 1.0], vec![[10.0, 0.0, 0.0]]);
        let v = advance(&mut st, &path, -0.4, 0.5);
        assert_eq!(st.position, start);
        assert_eq!(st.forward, [0.0, 0.0, 1.0]);
        assert!(v.rejected());
        assert!(!v.arrived_this_frame());
        assert_eq!(v.remaining_distance(), f32::INFINITY);
    }

    #[test]
    fn a_non_finite_corner_refuses_the_slot() {
        // 锚：字节级确定性撑不住一个进位置状态的 NaN，含非有限拐点的路径
        // 在门口被拒，不被传播。
        let start = [1.0, 2.0, 3.0];
        let (mut st, path) = walker(start, [0.0, 0.0, 1.0], vec![[f32::NAN, 0.0, 0.0]]);
        let v = advance(&mut st, &path, WALK_SPEED, 0.5);
        assert_eq!(st.position, start);
        assert!(v.rejected());
        assert_eq!(v.remaining_distance(), f32::INFINITY);
    }

    #[test]
    fn a_non_finite_or_negative_dt_refuses_the_slot() {
        // dt 从真源的整型微秒改成本律的显式入参后，非负性失去了类型保票，
        // 门槛移进拒绝谓词：负 dt 是倒放时间、NaN dt 会把非有限值写进位置
        // ——两者都按响亮拒绝处理，状态字节不动。dt = 0 合法（另测）。
        let start = [1.0, 2.0, 3.0];
        let (mut st, path) = walker(start, [0.0, 0.0, 1.0], vec![[10.0, 0.0, 0.0]]);
        let v = advance(&mut st, &path, WALK_SPEED, -0.5);
        assert!(v.rejected());
        assert_eq!(st.position, start);
        let v = advance(&mut st, &path, WALK_SPEED, f32::NAN);
        assert!(v.rejected());
        assert_eq!(st.position, start);
    }
}
