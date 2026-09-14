//! 粒子池的环形缓冲与压实。
//!
//! 模式语义 `C# 可读`（`ParticleSystemRingBufferMode` 枚注释逐条）：
//! `Disabled = 0`；`PauseUntilReplaced = 1`（到寿命终点暂停，直到被
//! 替换）；`LoopUntilReplaced = 2`（到淡出时刻回卷到淡入时刻；被替换
//! 时先走完剩余寿命）。
//!
//! 替换游标的存在 `C# 可读（存在性）`：`PlaybackState.m_RingBufferIndex`。
//! 谁在哪个模式被替换、何时替换，在 extern 墙后，行为口径给测量方案。
//!
//! 池的表示是**本仓自己的选择**（`Vec` + 交换删除），不是引擎律：引擎
//! 原生池的内存布局不可见，也不必可见——本模块钉的是**替换与移除的
//! 裁决语义**。

use crate::particle::step::Particle;

/// 环形缓冲模式。数值与名字 `C# 可读`（枚声明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingBufferMode {
    /// 死亡即移除；池满时停止出生（不覆写）。
    Disabled,
    /// 到寿命终点暂停等替换（见 `step::advance_lifetime`）。
    PauseUntilReplaced,
    /// 归一化寿命在 loop range 内回卷；池满时最老者被覆写。
    LoopUntilReplaced,
}

impl RingBufferMode {
    /// 序列化数值（schema 层用 0/1/2 解析，失败响亮拒绝）。
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(RingBufferMode::Disabled),
            1 => Some(RingBufferMode::PauseUntilReplaced),
            2 => Some(RingBufferMode::LoopUntilReplaced),
            _ => None,
        }
    }
}

/// 池满时一次出生尝试的裁决。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RingPushVerdict {
    /// 池未满：新粒子追加在末尾。
    Appended,
    /// 池满且按模式允许替换：新粒子落在 `index` 槽位。
    Replaced { index: usize },
    /// 池满且无处可替换：本帧这个粒子**没有出生**。发射侧应停止
    /// 呼叫（不是静默丢弃——调用方要能看见「满」）。
    Full,
}

/// 往池里出生一个粒子。
///
/// `cursor` 是替换游标（对齐 `m_RingBufferIndex`，`C# 可读（存在性）`），
/// 调用方跨帧携带。档位：**行为口径**。实现口径：
///
/// - 未满：一律追加（三种模式相同），游标不动；
/// - 模式 0 满：`Full`——出生停止，等压实腾位；
/// - 模式 1 满：替换**已到寿命终点**（`remaining_lifetime <= 0`，
///   即 PausedAtEnd 的粒子）的最老者；一个都没有则 `Full`
///   （枚注释的「系统暂停」）；游标不参与（真源在此模式用不用游标
///   读不出来，具名留白）；
/// - 模式 2 满：覆写游标处的槽位、游标 +1 回卷——最老者循环覆写。
///
/// Editor 测量方案：`maxParticles=4`、模式 1、寿命 1s、rate 10/s、
/// 跑 3s 后 `particleCount` 应为 4 且系统 `isEmitting` 应为假
/// （全部暂停在终点、无处替换）；同设置模式 2，粒子年龄分布应持续
/// 均匀回卷而无暂停。模式 2 的「被替换者先走完剩余寿命」是渲染侧
/// 的淡出宽限，不在本律（具名在 REPORT）。
pub fn ring_push(
    pool: &mut Vec<Particle>,
    cursor: &mut usize,
    mode: RingBufferMode,
    max_particles: usize,
    new: Particle,
) -> RingPushVerdict {
    if pool.len() < max_particles {
        pool.push(new);
        return RingPushVerdict::Appended;
    }
    if max_particles == 0 {
        return RingPushVerdict::Full;
    }
    match mode {
        RingBufferMode::Disabled => RingPushVerdict::Full,
        RingBufferMode::PauseUntilReplaced => {
            // 替换最老的「停在终点」者：Vec 保序，扫描第一个即最老。
            // 一个没有 -> 系统暂停（Full），不是错误。
            match pool.iter().position(|p| p.remaining_lifetime <= 0.0) {
                Some(index) => {
                    pool[index] = new;
                    RingPushVerdict::Replaced { index }
                }
                None => RingPushVerdict::Full,
            }
        }
        RingBufferMode::LoopUntilReplaced => {
            let index = (*cursor).min(max_particles.saturating_sub(1));
            pool[index] = new;
            *cursor = (index + 1) % max_particles;
            RingPushVerdict::Replaced { index }
        }
    }
}

/// 压实（模式 0 的移除侧）：交换删除全部死者，返回移除数。
///
/// 交换删除不保序（死者与末位活者互换）——渲染读全池不依赖序，
/// 测试钉住该行为。这是**本仓的表示选择**；引擎原生池的移除机制
/// 不可见，移除的**裁决**（模式 0 下死亡即移除）才是律。
pub fn compact(pool: &mut Vec<Particle>) -> usize {
    let mut removed = 0;
    let mut i = 0;
    while i < pool.len() {
        if pool[i].remaining_lifetime <= 0.0 {
            pool.swap_remove(i);
            removed += 1;
            // swap_remove 把末位换到 i：不前进，重扫本位（换上来的
            // 可能也是死者）。
        } else {
            i += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(lifetime: f32) -> Particle {
        Particle::born([0.0; 3], [0.0; 3], lifetime)
    }

    fn dead() -> Particle {
        let mut p = live(0.5);
        p.remaining_lifetime = 0.0;
        p
    }

    #[test]
    fn append_until_full_then_gate_by_mode() {
        // 三模式共享：未满追加。满了之后模式 0 -> Full。
        let mut pool = Vec::new();
        let mut cursor = 0;
        let verdicts: Vec<RingPushVerdict> = (0..3)
            .map(|_| ring_push(&mut pool, &mut cursor, RingBufferMode::Disabled, 2, live(1.0)))
            .collect();
        assert_eq!(
            verdicts,
            vec![RingPushVerdict::Appended, RingPushVerdict::Appended, RingPushVerdict::Full]
        );
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn mode_one_replaces_oldest_held_particle() {
        // 池满：两个活 + 一个停在终点。替换发生在最老的停终者（下标 2）。
        let mut pool = vec![live(1.0), live(1.0), dead()];
        let mut cursor = 0;
        let v = ring_push(&mut pool, &mut cursor, RingBufferMode::PauseUntilReplaced, 3, live(9.0));
        assert_eq!(v, RingPushVerdict::Replaced { index: 2 });
        assert!((pool[2].remaining_lifetime - 9.0).abs() < 1e-6);
        // 全活且满 -> 系统暂停（Full），池不动。
        let mut all_alive = vec![live(1.0), live(1.0), live(1.0)];
        let v = ring_push(
            &mut all_alive,
            &mut cursor,
            RingBufferMode::PauseUntilReplaced,
            3,
            live(9.0),
        );
        assert_eq!(v, RingPushVerdict::Full);
        assert_eq!(all_alive.len(), 3);
    }

    #[test]
    fn mode_two_overwrites_at_rotating_cursor() {
        // 池满 max=3：覆写游标 0 -> 1 -> 2 -> 0，游标回卷。
        let mut pool = vec![live(1.0), live(2.0), live(3.0)];
        let mut cursor = 0;
        let first = ring_push(&mut pool, &mut cursor, RingBufferMode::LoopUntilReplaced, 3, dead());
        assert_eq!(first, RingPushVerdict::Replaced { index: 0 });
        let second = ring_push(&mut pool, &mut cursor, RingBufferMode::LoopUntilReplaced, 3, dead());
        assert_eq!(second, RingPushVerdict::Replaced { index: 1 });
        let third = ring_push(&mut pool, &mut cursor, RingBufferMode::LoopUntilReplaced, 3, dead());
        assert_eq!(third, RingPushVerdict::Replaced { index: 2 });
        let fourth = ring_push(&mut pool, &mut cursor, RingBufferMode::LoopUntilReplaced, 3, dead());
        assert_eq!(fourth, RingPushVerdict::Replaced { index: 0 });
        // 被覆写的槽位换上了新粒子（此处用 dead() 当标记：剩 0）。
        assert_eq!(pool[0].remaining_lifetime, 0.0);
        assert_eq!(pool[1].remaining_lifetime, 0.0);
    }

    #[test]
    fn zero_capacity_never_admits() {
        let mut pool = Vec::new();
        let mut cursor = 0;
        assert_eq!(
            ring_push(&mut pool, &mut cursor, RingBufferMode::Disabled, 0, live(1.0)),
            RingPushVerdict::Full
        );
        assert!(pool.is_empty());
    }

    #[test]
    fn compact_swap_removes_all_dead_including_chained() {
        // [活A, 死, 死, 活B]：swap_remove 两次；末位换上来的可能又是
        // 死者，while 不前进重扫——两个死者都清掉，活者保留（不保序）。
        let mut a = live(1.0);
        a.position = [1.0, 0.0, 0.0];
        let mut b = live(2.0);
        b.position = [2.0, 0.0, 0.0];
        let mut pool = vec![a, dead(), dead(), b];
        assert_eq!(compact(&mut pool), 2);
        assert_eq!(pool.len(), 2);
        let positions: Vec<[f32; 3]> = pool.iter().map(|p| p.position).collect();
        assert!(positions.contains(&[1.0, 0.0, 0.0]));
        assert!(positions.contains(&[2.0, 0.0, 0.0]));
    }

    #[test]
    fn compact_keeps_live_order_when_no_dead() {
        let mut pool = vec![live(1.0), live(2.0)];
        assert_eq!(compact(&mut pool), 0);
        assert_eq!(pool[0].remaining_lifetime, 1.0);
        assert_eq!(pool[1].remaining_lifetime, 2.0);
    }

    #[test]
    fn ring_mode_from_serialized_value() {
        assert_eq!(RingBufferMode::from_u32(0), Some(RingBufferMode::Disabled));
        assert_eq!(
            RingBufferMode::from_u32(1),
            Some(RingBufferMode::PauseUntilReplaced)
        );
        assert_eq!(
            RingBufferMode::from_u32(2),
            Some(RingBufferMode::LoopUntilReplaced)
        );
        assert_eq!(RingBufferMode::from_u32(3), None);
    }
}
