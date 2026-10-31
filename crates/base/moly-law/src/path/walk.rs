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
    /// Heading while walking. Frames that do not walk (arrived, no path,
    /// refused) keep it unchanged — the heading code returns early when the
    /// agent is stopped, so "no walking means no turning" holds there too.
    pub forward: [f32; 3],
    /// Index of the next corner not yet reached. This is cross-frame
    /// progress memory: a corner already passed is never walked again,
    /// otherwise a walker mid-route would turn back toward the first corner
    /// every frame. The progress must live in the state and not in the input
    /// — a single move sends its destination once and the path is
    /// not re-fed every frame. Note that the fixed index read by the heading
    /// does **not** advance with it; that quirk is kept on purpose (see the
    /// module comment).
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
    /// Refused: the input carries a value the move loop can never produce;
    /// state bytes stay unchanged and the distance is unknown.
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

    // A single predicate `!(x >= 0.0)` catches both negative values and NaN:
    // NaN compares false against everything and cannot slip past `>=`. The dt
    // gate matches the speed gate: the frame step in the move
    // loop comes from integer microseconds, so negative and non-numeric
    // values cannot occur; letting a NaN through would write it straight into
    // the position and destroy the determinism of every later frame.
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

    // Arrival is checked at the start of the frame, matching the move loop's
    // "check first, then advance" structure: a walker that enters the band
    // mid-frame still reports walking for that frame and arrival only on the
    // next one. Once inside the band it holds still — the loop has exited,
    // so this implementation does not move the walker further within the
    // band, and the residual stays within the arrival distance by
    // construction.
    let distance_to_end = distance_to_last(state.position, corners);
    if distance_to_end < ARRIVAL_DISTANCE {
        return WalkVerdict::Arrived(distance_to_end);
    }

    let budget = speed * dt;

    // Only walk the corners not yet consumed. The corner progress blocks the
    // way back: a walker mid-route does not turn around every frame to walk
    // the corners already passed again. (The heading below deliberately still
    // points at corner 0 of the row — that fixed index reading is pinned by a
    // test; do not "fix" it in passing.)
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

/// Straight-line distance from `position` to the last corner — this is
/// exactly what the arrival check compares. The caller guarantees that
/// `corners` is non-empty.
fn distance_to_last(position: [f32; 3], corners: &[[f32; 3]]) -> f32 {
    let last = corners[corners.len() - 1];
    let dx = last[0] - position[0];
    let dy = last[1] - position[1];
    let dz = last[2] - position[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Advances `position` along the polyline `position -> corners[0] -> ...` by
/// at most `budget` world units; every corner whose whole segment the budget
/// covers is recorded in `consumed`. It never overshoots: when the budget
/// covers the remaining path, the position lands exactly on the last corner
/// (one assignment, byte-identical to the corner). A fully covered intermediate
/// corner is likewise reached by assignment — on the frame whose step lands on
/// (or crosses over) a corner, the exact value is touched and the caller's
/// corner progress moves past it.
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
            // Reaching here means `remaining < seg` and `seg > 0` (a
            // zero-length segment takes the branch above), so `t` lies in
            // `[0, 1)` and there is no division by zero. The corner being
            // approached is not consumed: this step stops short of it and the
            // next frame continues toward it.
            let t = remaining / seg;
            position[0] += dx * t;
            position[1] += dy * t;
            position[2] += dz * t;
            return;
        }
    }
}

/// The heading of the current frame: points at the given corner and is
/// normalized; when the corner is on (or, within the caller's threshold, near)
/// the walker's own position, the fallback is taken. The corner index is
/// fixed, as in the move loop: the walker standing on the first corner faces
/// the fallback direction rather than the next corner, and a walker past the
/// first corner faces back at it.
///
/// Public for the departure decision of a leg-change frame: the target
/// direction of the turn must be **the same source** as the heading computed
/// by feeding the walking rule the leg's first frame — the same function with
/// the same pair of inputs — so that the heading does not jump when the turn
/// hands over to walking.
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

    /// The test speeds in this file deliberately avoid 1.0: with speed exactly
    /// 1.0, "walked speed*dt" is indistinguishable from "walked dt", and a
    /// passing test would not show
    /// which one is measured. 0.4 is the table walk speed for the first
    /// character; the others are arbitrary but keep the same property.
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
        // Check: the agent's speed means the maximum movement speed, and the
        // start writes the selected speed. 0.4 units/second for half a second
        // must advance 0.2 along the path — and it must move at all, since if
        // the advance integrated an empty set every assertion in this file
        // would pass vacuously.
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
        // Check: the stop routine clears the agent's speed, so zero is a speed
        // the move loop really produces, and no dt may move the walker. Using
        // several frame lengths makes "any dt" more than one example.
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
        // Check: the threshold itself is 0.1 — the literal of the move loop.
        // A walker starting 0.0625 from the target (inside 0.1) must report
        // arrival on the first frame while the position bytes stay unchanged.
        // A walker approaching 0.5 away with exact power-of-two steps (0.125
        // per frame) must still be walking on the frame where it stands at
        // 0.125 — 0.125 is outside 0.1 — so if the threshold were wrongly
        // taken as the other literal, 0.3 (the automatic-braking switch), this
        // test fails. Every distance is a power of two, so no step loses
        // precision.
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
        // Check: the move loop has exited (that is what arrival means here),
        // and the heading code returns early when the agent is stopped — so
        // after entering the band neither position nor heading may change.
        // Seventeen frames bring the walker to the end (sixteen walking, one
        // noticing); every frame after that must change nothing.
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
        // Check: remainingDistance measures along the current path, and the
        // agent's steeringTarget is the next corner — walking proceeds segment
        // by segment, and a corner reached by one step must be **touched**,
        // not approximated. Speed 2.0 for half a second is a budget of 1.0; the
        // first corner is just outside 1.0, so that frame takes the assignment
        // branch: byte-identical to the corner, and the overshoot deviation
        // reported is exactly 0.0.
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
        // The next leg keeps walking, then arrives inside the band (the band
        // of 0.1 is where the move loop itself stops — the residual is
        // reported and pinned below the band).
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
        // Check: the heading code reads one fixed corner and normalizes the
        // current position toward it. A walker at the origin heading toward
        // (3, 4, 0) (length 5) must face [0.6, 0.8, 0]. Pointing at the last
        // corner instead, or not normalizing, gives a different result.
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
        // Check: when the target corner is less than 1e-05 from the walker,
        // the unit forward is used instead of dividing by a near-zero
        // magnitude — and the index read is fixed, so a walker standing
        // exactly on the first corner takes the fallback even when a second
        // corner lies ahead. The landing point here is provable: the corner is
        // 0.5 away and the step is exactly 0.5 (speed 2.0, quarter second), so
        // the assignment branch applies and the walker ends the frame standing
        // byte-for-byte on the corner.
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
        // The other half of the fixed-index rule, pinned so nobody "fixes" it:
        // from the second frame on, the walker has passed the corner the
        // heading code reads (the index never advances), so the rule makes it
        // face **backwards** along the path. The rest of the heading behavior
        // (rotation, avoidance gating) is not modelled here; this rule
        // recomputes the look-at only, quirks included.
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
        // Check: the move loop's speed writes only ever produce zero (cleared
        // on stop) or non-negative table values; a negative speed cannot
        // occur, so there is no defined semantics to integrate. Refusal is
        // loud (verdict flag, unknown distance, no arrival) and the state
        // bytes stay unchanged — silent guessing is the one outcome never
        // allowed.
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
        // dt is an explicit parameter here rather than the move loop's integer
        // microseconds, so non-negativity is no longer guaranteed by the type;
        // the gate moves into the refusal predicate: a negative dt runs time
        // backwards and a NaN dt writes a non-finite value into the position —
        // both are refused loudly with the state bytes unchanged. dt = 0 is
        // legal (tested separately).
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
