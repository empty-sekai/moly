//! Which tracks one start of a structured cue plays: the middleware's
//! sequence types 0-4 (polyphonic, sequential, shuffle, random, random
//! without repeat). Pure functions over the cue table's own data; the
//! consumers (ambient channel, one-shot SE, timeline SE) decide when a start
//! happens and play what it returns.
//!
//! Every start of a sequence walks its tracks in authored order and asks the
//! type which of them to play:
//!
//! - **polyphonic**: every track;
//! - **sequential**: the position advances (`pos + 1 < n ? pos + 1 : 0`, the
//!   same advance the shuffle uses) and the track at the position plays;
//! - **shuffle**: [`crate::audio_sequence::advance_shuffle`];
//! - **random**: one draw `range(0, hi)` from the player's generator, then a
//!   running sum over the tracks' weights (a cue without a weight list adds 1
//!   per track); the first track whose running sum, as 16 bits, is at least the
//!   draw, as 16 bits, plays. The draw's upper bound is the sum of the weights,
//!   or `n - 1` when the cue has no weights or they sum to 0. The bound is
//!   inclusive, so track 0 wins a draw of 0 even at weight 0, and a draw past
//!   every running sum plays nothing;
//! - **random without repeat** (cue sheets written before format 1.42; a newer
//!   sheet keeps a history list this port does not read, see
//!   [`NO_REPEAT_HISTORY_FORMAT`]): as random, except that the track at the
//!   position is skipped (its weight is not added) while at least one other
//!   track has weight, and the track that plays is written back to the
//!   position. The draw's bound: before any start (position negative) the sum
//!   of all weights when it is over 99, else 100, or `n - 1` without weights;
//!   after, from the sum of the other tracks' weights `rest`, the sum of all
//!   weights `all` (16 bits) and the count of other tracks with weight: `rest`
//!   when `all >= 100` and `rest != 0`; the count when `all == 0` and
//!   `rest != 0`; otherwise `100 - all + rest` (32-bit wrap). Without weights
//!   `rest` and the count are `n - 1` and `all` is 0.
//!
//! Only random and random without repeat draw here (one draw per start);
//! polyphonic and sequential draw nothing, the shuffle draws in its own law.
//! The position lives with the cue table (see [`ShuffleWork`]): -1 when the
//! table loads, kept across stops.

use crate::audio_sequence::{advance_shuffle, SequenceRng, ShuffleWork};

/// Cue sheet format from which random-without-repeat reads a history list
/// instead of the position (header `Version` compared as a 32-bit number).
/// The port implements the older branch only.
pub(crate) const NO_REPEAT_HISTORY_FORMAT: u32 = 0x0142_0000;

/// The sequence types this port plays (cue type enum codes 0-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SequenceKind {
    Polyphonic,
    Sequential,
    Shuffle,
    Random,
    RandomNoRepeat,
}

impl SequenceKind {
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Polyphonic),
            1 => Some(Self::Sequential),
            2 => Some(Self::Shuffle),
            3 => Some(Self::Random),
            4 => Some(Self::RandomNoRepeat),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Polyphonic => "polyphonic",
            Self::Sequential => "sequential",
            Self::Shuffle => "shuffle",
            Self::Random => "random",
            Self::RandomNoRepeat => "randomNoRepeat",
        }
    }
}

/// The draw of the two random types. `count` receives the number of other
/// tracks with weight (random without repeat, after the first start) and is
/// what the selection reads to decide whether the position's track is
/// skipped. Returns the draw; other types return 0 without drawing.
pub(crate) fn random_draw(
    kind: SequenceKind,
    tracks: usize,
    position: i16,
    weights: Option<&[u16]>,
    count: &mut u16,
    rng: &mut SequenceRng,
) -> u32 {
    let n = (tracks & 0xffff) as u32;
    let hi = match kind {
        SequenceKind::Random => {
            let mut sum = 0u32;
            if let Some(weights) = weights.filter(|weights| !weights.is_empty()) {
                for &weight in weights {
                    if weight != 0 {
                        *count = count.wrapping_add(1);
                    }
                    sum = sum.wrapping_add(u32::from(weight));
                }
            }
            if sum != 0 {
                sum
            } else {
                n.wrapping_sub(1)
            }
        }
        SequenceKind::RandomNoRepeat if position < 0 => match weights {
            None => n.wrapping_sub(1),
            Some(weights) => {
                let mut sum = 0u32;
                for &weight in weights.iter().take(tracks) {
                    sum = (sum + u32::from(weight)) & 0xffff;
                }
                if sum > 99 {
                    sum
                } else {
                    100
                }
            }
        },
        SequenceKind::RandomNoRepeat => {
            let (all, rest) = match weights {
                None => {
                    let rest = n.wrapping_sub(1);
                    *count = rest as u16;
                    (0u32, rest)
                }
                Some(weights) => {
                    let (mut all, mut rest) = (0u32, 0u32);
                    for (index, &weight) in weights.iter().take(tracks).enumerate() {
                        if index as i32 != i32::from(position) {
                            rest = rest.wrapping_add(u32::from(weight));
                            if weight != 0 {
                                *count = count.wrapping_add(1);
                            }
                        }
                        all = (all + u32::from(weight)) & 0xffff;
                    }
                    (all, rest)
                }
            };
            let rest16 = rest & 0xffff;
            if all >= 100 && rest16 != 0 {
                rest16
            } else if all == 0 && rest16 != 0 {
                u32::from(*count)
            } else {
                100u32.wrapping_sub(all).wrapping_add(rest16)
            }
        }
        SequenceKind::Polyphonic | SequenceKind::Sequential | SequenceKind::Shuffle => return 0,
    };
    rng.range(0, hi)
}

/// One start of a sequence with authored track ids `tracks` (the ids the
/// shuffle's order table holds) and optional per-track weights. Returns the
/// authored slots that play, in authored order (empty when a random draw lands
/// past every running sum).
pub(crate) fn start(
    kind: SequenceKind,
    tracks: &[u16],
    weights: Option<&[u16]>,
    work: &mut ShuffleWork,
    rng: &mut SequenceRng,
) -> Vec<usize> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    match kind {
        SequenceKind::Polyphonic => (0..n).collect(),
        SequenceKind::Sequential => {
            let next = work.position.wrapping_add(1);
            work.position = if (n as i32) > i32::from(next) {
                next
            } else {
                0
            };
            vec![work.position as usize]
        }
        SequenceKind::Shuffle => {
            let Some(track) = advance_shuffle(work, tracks, rng) else {
                return Vec::new();
            };
            let slot = tracks
                .iter()
                .position(|id| *id == track)
                .unwrap_or_else(|| {
                    panic!("shuffle order holds track {track}, not in the authored list")
                });
            vec![slot]
        }
        SequenceKind::Random | SequenceKind::RandomNoRepeat => {
            let mut count = 0u16;
            let draw = random_draw(kind, n, work.position, weights, &mut count, rng) & 0xffff;
            let mut sum = 0u32;
            for slot in 0..n {
                if kind == SequenceKind::RandomNoRepeat
                    && count != 0
                    && slot as i32 == i32::from(work.position)
                {
                    continue;
                }
                sum = sum.wrapping_add(match weights {
                    Some(weights) => u32::from(weights.get(slot).copied().unwrap_or(0)),
                    None => 1,
                });
                if sum & 0xffff >= draw {
                    if kind == SequenceKind::RandomNoRepeat {
                        work.position = slot as i16;
                    }
                    return vec![slot];
                }
            }
            Vec::new()
        }
    }
}

#[cfg(test)]
mod source_receipt {
    //! Sampled starts computed by the middleware's own draw and position
    //! functions (executed on the shipped native library with its default
    //! generator; selection per type as read from the same code), against
    //! [`start`]. One line per case: type, track count, weights (`-` none),
    //! seed, then one symbol per start: the slot that played, `.` for none,
    //! `*` for every track.
    use super::*;

    const CASES: &str = include_str!("cue_law_receipt.txt");

    fn symbol(kind: SequenceKind, slots: &[usize]) -> char {
        match slots {
            [] => '.',
            _ if kind == SequenceKind::Polyphonic => '*',
            [slot] => char::from_digit(*slot as u32, 36).unwrap(),
            _ => '?',
        }
    }

    #[test]
    fn starts_match_the_middleware() {
        let mut cases = 0;
        let mut starts = 0;
        for line in CASES
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let fields: Vec<&str> = line.split(' ').collect();
            let [kind, n, weights, seed, expected] = fields.as_slice() else {
                panic!("receipt line {line:?} is not five fields");
            };
            let kind = SequenceKind::from_code(kind.parse().unwrap()).unwrap();
            let n: usize = n.parse().unwrap();
            let weights: Option<Vec<u16>> =
                (*weights != "-").then(|| weights.split(',').map(|w| w.parse().unwrap()).collect());
            let mut rng = SequenceRng::seeded(seed.parse().unwrap());
            let ids: Vec<u16> = (0..n as u16).map(|i| 0x100 + i).collect();
            let mut work = ShuffleWork::loaded(n);
            let mine: String = expected
                .chars()
                .map(|_| {
                    symbol(
                        kind,
                        &start(kind, &ids, weights.as_deref(), &mut work, &mut rng),
                    )
                })
                .collect();
            assert_eq!(&mine, expected, "{line}");
            cases += 1;
            starts += expected.len();
        }
        assert!(cases > 0 && starts > 0, "receipt table is empty");
    }
}
