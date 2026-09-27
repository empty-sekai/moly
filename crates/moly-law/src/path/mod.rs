//! 行走律：把「路径拐点 + 速度 + dt」一帧一帧复算成一个位置和一个朝向，
//! 一次一个 NPC。`dt` 是显式入参——帧间移动的「平滑」完全由它决定。
//!
//! 真源不自己积分位置：它配置寻路代理（`SetupNavMeshAgent`：半径、高度、
//! 停止距离、速度等），起步（`SetupMoveNavMeshParameter`：清 `isStopped`、
//! 写加速度与所选速度），交出目的地（`SetDestinationIfMoving`），之后每帧
//! 的推进发生在代理的原生实现里——`velocity`、`steeringTarget`、
//! `remainingDistance` 全是没有函数体的 extern 绑定。那面墙是边界，本模块
//! 不复算墙后的传递函数；复算的是四周可读的合同：
//!
//! * 起步把所选速度写进代理的 `speed`，而 `speed` 的语义是**最大**移动
//!   速度、`remainingDistance` 沿当前路径计量——最强可读陈述是「沿拐点
//!   折线、每秒至多 `speed`」，本模块按恰好 `speed` 走。
//! * 到达是真源移动循环自己的逐帧判断：与移动目标的直线距离低于
//!   [`ARRIVAL_DISTANCE`] 即跳出循环。判定放在帧首：帧中才进带的行走者
//!   本帧仍在走、下一帧才报到达；进带后原地保持——带内最终停在哪在墙后，
//!   保持是诚实答案，残差按构造留在带内。
//! * 行走中的朝向按 `LookAtNavMeshCorner` 复算：读路径拐点表里固定的
//!   一位（行走者自身条目的下一位；折算到 [`NpcPathWalkSlot`] 的行即
//!   第 0 拐点，下标从不前进），从当前位置归一化指向它；该拐点距自身不足
//!   [`DIRECTION_EPSILON`] 时回退单位朝向 [`FORWARD_FALLBACK`]。固定下标
//!   的怪癖一并保留：站上首拐点取回退而不看前方，越过首拐点后倒退着
//!   朝向它。
//!
//! # 加速度与刹车：代理的逐帧传递函数（[`crowd`]）
//!
//! 起步写过一个加速度（8.0），移动循环每帧按「剩余距离 < 0.3」改写自动
//! 刹车开关。消费这些输入的是引擎的人群更新：它的函数体（转向、积分、沿
//! 导航单元移动、剩余距离）已经可读，移植在 [`crowd`] 与 `carve` 的走廊里；
//! NPC 的逐帧移动走那一条。本文件的 [`advance`]（恒速折线）不再是 NPC 的
//! 移动律，只留给仍按折线复算的调用方。
//!
//! 朝向也不按避障类型门控：真源的朝向代码只在避障类型非零时运行，而该
//! 类型被两处以两个不同的值条件写入，任一时刻立着的是哪个，没有可读来源
//! 能裁决。按它门控等于发明它的状态——朝向规则在行走中无条件生效，门控
//! 留给消费朝向的那一侧。
//!
//! # 拒绝是响的
//!
//! 速度为负或非数、拐点或持久状态含非有限值、dt 为负或非数——该 NPC
//! 本帧被拒绝：位置与朝向字节不动、剩余距离报未知（无穷，与引擎侧属性在
//! 距离未知时的读数一致）、不给到达、立拒绝标记。真源的速度写入只产得出
//! 零（停机清零）或非负的表值，其余值无从发生；非有限值一旦进位置状态，
//! 之后的快照不再有字节一致的确定性与可复算性。静默修数等于替没人写下过
//! 的语义作主，所以宁可响。

pub mod crowd;
pub mod look_at;
pub mod slot;
pub mod turn;
pub mod walk;
pub mod waypoint;

pub use slot::NpcPathWalkSlot;
pub use turn::{
    TurnMotion, ROTATE_DEGREES_PER_SECOND, angle_between, heading_yaw, make_positive_yaw,
    rotate_time, turn_angle, turn_motion,
};
pub use walk::{WalkState, WalkVerdict, advance, facing_direction};
pub use waypoint::{Waypoint, WaypointDraw, WaypointKind, WAYPOINT_SAMPLE_DISTANCE, build_waypoints};

/// 到达阈：真源移动循环自己的字面量——与移动目标的直线距离低于它即跳出
/// 循环。数值上与配置代理时写的停止距离相同，但那是两处不同位置的两个
/// 字面量（一处喂原生代理的刹车，一处是循环自己的比较），不合并成一个
/// 公共常数，免得漂移出虚假的「单一常数」声明。
pub const ARRIVAL_DISTANCE: f32 = 0.1;

/// 近零回退阈：目标拐点距自身不超过它时，朝向取回退值而不做除以近零
/// 模长的归一化。
pub const DIRECTION_EPSILON: f32 = 1e-5;

/// 回退朝向：真源在目标拐点近零时使用的单位前向 `[0, 0, 1]`。
pub const FORWARD_FALLBACK: [f32; 3] = [0.0, 0.0, 1.0];
