//! 洗牌序列（序列类型 2）的选轨律与它的抽签源——纯函数，不依赖引擎。
//!
//! 音频中间件的 cue 引用一条**序列**：序列列出若干轨，每轨是「起播延迟 →
//! noteOn」的一小段事件程序。序列类型 2（洗牌）每起一轮只放一轨：
//!
//! - **位置**：有符号 16 位，cue 表装载时每条序列置 -1；每起一轮先推进——
//!   `pos + 1 < n` 则 +1，否则归 0。位置写回 cue 表自己的那一行，所以同一
//!   cue 停了再放，位置接着走（直到 cue 表被卸载重装才回 -1）。
//! - **顺序表**：紧跟在 cue 表里那 n 个轨号之后的同宽空间。首轮（推进前的
//!   位置为负）先把作者序的轨号整段抄进去；推进后位置为 0 时重洗：逐格
//!   `i = 0..n` 与 `range(0, n-1)` 抽中的格对换（不是 Fisher-Yates）。随后若
//!   `n >= 3`、不是首轮、且新表首格等于**重洗前**表的末格，再把首格与
//!   `range(1, n-1)` 抽中的格对换——跨轮不连放同一轨。放的是 `顺序表[位置]`。
//! - **抽签源**：播放器自己的 xorshift128（Marsaglia：`t = x ^ x<<11`，
//!   `w' = w ^ w>>19 ^ t ^ t>>8`）。种子按 Mersenne Twister 的初始化递推铺成
//!   四个状态字（`s0 = 1812433253·(seed ^ seed>>30)`，
//!   `s_i = 1812433253·(s_{i-1} ^ s_{i-1}>>30) + i`）。区间抽签 =
//!   `lo + next() mod (top - lo + 1)`，`top = max(lo, hi)` 按**有符号**比较
//!   取，取模是无符号的。区间宽回绕成 0 时按处理器的除零语义，余数就是
//!   被除数本身。播放器建立时以单调时钟的纳秒数（低 32 位）起种，所以种子
//!   是运行时状态：本仓按自己的时钟起种，与真源同分布、不同值。
//!
//! 这里只放律本身。谁调、何时起下一轮（本轮那一轨的声音放完、没有被叫停、
//! 序列带重放标志）在消费侧（区域环境音通道）。

/// 序列类型码：洗牌（中间件 cue 类型枚举里的 2）。
pub(crate) const SEQUENCE_TYPE_SHUFFLE: u8 = 2;

/// 播放器的抽签源：xorshift128，四个 32 位状态字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SequenceRng {
    state: [u32; 4],
}

impl SequenceRng {
    /// 以 32 位种子起种（Mersenne Twister 初始化递推的前四项）。
    pub(crate) fn seeded(seed: u32) -> Self {
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
    pub(crate) fn state(&self) -> [u32; 4] {
        self.state
    }

    /// 下一个 32 位数。
    pub(crate) fn next(&mut self) -> u32 {
        let [x, y, z, w] = self.state;
        let t = x ^ (x << 11);
        let next = w ^ (w >> 19) ^ t ^ (t >> 8);
        self.state = [y, z, w, next];
        next
    }

    /// 闭区间 `[lo, max(lo, hi)]` 里抽一个（上界按有符号比较取）。
    pub(crate) fn range(&mut self, lo: u32, hi: u32) -> u32 {
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
pub(crate) struct ShuffleWork {
    pub(crate) position: i16,
    pub(crate) order: Vec<u16>,
}

impl ShuffleWork {
    /// cue 表刚装载的样子：位置 -1。顺序表的初值用不到：首轮先整段抄入。
    pub(crate) fn loaded(tracks: usize) -> Self {
        Self {
            position: -1,
            order: vec![0; tracks],
        }
    }
}

/// 起一轮：推进位置、按需重洗，返回这一轮放的轨号；序列没有轨时返回
/// `None`（位置不动）。`tracks` 是作者序的轨号表。
pub(crate) fn advance_shuffle(
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
