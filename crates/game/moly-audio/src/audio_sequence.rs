//! Track selection for shuffle sequences (sequence type 2) and their draw
//! source — pure functions, independent of the engine.
//!
//! A cue of the audio middleware references a **sequence**: a sequence lists
//! several tracks, each a short event program of "start delay -> noteOn".
//! Sequence type 2 (shuffle) plays one track per round:
//!
//! - **Position**: signed 16-bit, set to -1 for every sequence when the cue
//!   table loads; every round first advances it — if `pos + 1 < n` then +1,
//!   otherwise wrap to 0. The position is written back into the cue table's own
//!   row, so the same cue stopped and started again continues from its
//!   position (it returns to -1 only when the cue table is unloaded and
//!   reloaded).
//! - **Order table**: the same-width space right after the cue table's n track
//!   numbers. The first round (position negative before advancing) copies the
//!   authored track numbers in whole; when the advanced position is 0 the
//!   table is reshuffled: for every cell `i = 0..n`, swap it with the cell drawn
//!   from `range(0, n-1)` (not Fisher-Yates). Then, if `n >= 3`, this is not
//!   the first round, and the new table's first cell equals the **pre-shuffle**
//!   table's last cell, swap the first cell with the cell drawn from
//!   `range(1, n-1)` — the same track never plays in two consecutive rounds.
//!   What plays is `order table[position]`.
//! - **Draw source**: the player's own xorshift128 (Marsaglia:
//!   `t = x ^ x<<11`, `w' = w ^ w>>19 ^ t ^ t>>8`). The seed is spread into
//!   four state words by the Mersenne Twister initialization recurrence
//!   (`s0 = 1812433253·(seed ^ seed>>30)`,
//!   `s_i = 1812433253·(s_{i-1} ^ s_{i-1}>>30) + i`). A range draw =
//!   `lo + next() mod (top - lo + 1)`, with `top = max(lo, hi)` taken by a
//!   **signed** comparison and the modulo unsigned. When the range width wraps
//!   to 0, the remainder follows the processor's divide-by-zero semantics and
//!   is the dividend itself. The player seeds it with the low 32 bits of the
//!   monotonic clock in nanoseconds, so the seed is run-time state: this crate
//!   seeds it from its own clock, with the same distribution as the original
//!   game but different values.
//!
//! Only the rules live here. Who calls them and when the next round starts
//! (this round's track has played out, was not stopped, and the sequence
//! carries the repeat flag) is on the consumption side (the area ambience
//! channel).

/// 序列类型码：洗牌（中间件 cue 类型枚举里的 2）。
pub const SEQUENCE_TYPE_SHUFFLE: u8 = 2;

/// 播放器的抽签源：xorshift128，四个 32 位状态字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceRng {
    state: [u32; 4],
}

impl SequenceRng {
    /// 以 32 位种子起种（Mersenne Twister 初始化递推的前四项）。
    pub fn seeded(seed: u32) -> Self {
        let mut state = [0u32; 4];
        state[0] = 1_812_433_253u32.wrapping_mul(seed ^ (seed >> 30));
        for i in 1..4 {
            let previous = state[i - 1];
            state[i] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        Self { state }
    }

    /// 四个状态字（账目与比对用）。
    pub fn state(&self) -> [u32; 4] {
        self.state
    }

    /// 下一个 32 位数。
    pub fn next(&mut self) -> u32 {
        let [x, y, z, w] = self.state;
        let t = x ^ (x << 11);
        let next = w ^ (w >> 19) ^ t ^ (t >> 8);
        self.state = [y, z, w, next];
        next
    }

    /// 闭区间 `[lo, max(lo, hi)]` 里抽一个（上界按有符号比较取）。
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        let top = if (lo as i32) > (hi as i32) { lo } else { hi };
        let width = top.wrapping_sub(lo).wrapping_add(1);
        let drawn = self.next();
        // 除数为 0 时处理器的无符号除法得 0，余数即被除数。
        let remainder = if width == 0 { drawn } else { drawn % width };
        remainder.wrapping_add(lo)
    }
}

/// 一条洗牌序列在 cue 表里的工作区：位置 + 顺序表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShuffleWork {
    pub position: i16,
    pub order: Vec<u16>,
}

impl ShuffleWork {
    /// cue 表刚装载的样子：位置 -1。顺序表的初值用不到：首轮先整段抄入。
    pub fn loaded(tracks: usize) -> Self {
        Self {
            position: -1,
            order: vec![0; tracks],
        }
    }
}

/// 起一轮：推进位置、按需重洗，返回这一轮放的轨号；序列没有轨时返回
/// `None`（位置不动）。`tracks` 是作者序的轨号表。
pub fn advance_shuffle(
    work: &mut ShuffleWork,
    tracks: &[u16],
    rng: &mut SequenceRng,
) -> Option<u16> {
    let n = tracks.len();
    if n == 0 || n > u16::MAX as usize {
        return None;
    }
    if work.order.len() != n {
        // 工作区与轨表同宽：同一条序列的轨数在 cue 表里是定值。
        panic!("洗牌序列工作区宽 {} 与轨数 {n} 不一致", work.order.len());
    }
    let old = work.position;
    let next = (old as i32).wrapping_add(1) as i16;
    let new = if (n as i32) > next as i32 { next } else { 0 };
    work.position = new;
    let last = work.order[n - 1];
    if old < 0 {
        work.order.copy_from_slice(tracks);
    }
    if new == 0 {
        let top = (n - 1) as u32;
        for slot in 0..n {
            let other = rng.range(0, top) as usize;
            work.order.swap(slot, other);
        }
        if n >= 3 && old >= 0 && work.order[0] == last {
            let other = rng.range(1, top) as usize;
            work.order.swap(0, other);
        }
    }
    Some(work.order[new as usize])
}
