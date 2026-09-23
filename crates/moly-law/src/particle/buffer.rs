//! Storage order from current native StartParticles, CopyParticlesToUnalignedDst
//! and KillParticles. Parallel payloads move together; index is not identity.
use crate::particle::step::Particle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingBufferMode { Disabled, PauseUntilReplaced, LoopUntilReplaced }
impl RingBufferMode {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v { 0 => Some(Self::Disabled), 1 => Some(Self::PauseUntilReplaced),
            2 => Some(Self::LoopUntilReplaced), _ => None }
    }
}

/// Capacity before a whole birth batch. Ring modes reserve a second span for
/// newborns and, in Loop mode, the replaced particles finishing their lives.
pub fn birth_capacity(current: usize, mode: RingBufferMode, maximum: usize, requested: usize) -> usize {
    let capacity = maximum.saturating_mul(if mode == RingBufferMode::Disabled { 1 } else { 2 });
    requested.min(capacity.saturating_sub(current))
}

/// Finish a batch already appended to both arrays. Native births start at
/// align_up4(old_count); packing copies the last min(gap, births) elements into
/// the alignment gap. This rotates the birth batch even in Disabled mode.
///
/// Pause records each victim's death before overwriting it, regardless of age.
/// Loop swaps *every* newborn with the cursor when total count > maximum;
/// displaced particles stay in the overflow span and stop looping.
pub fn finish_births<T: Clone, U: Clone>(
    pool: &mut Vec<T>, side: &mut Vec<U>, cursor: &mut usize,
    mode: RingBufferMode, maximum: usize, old_count: usize,
    mut on_death: impl FnMut(&T, &U),
) {
    assert_eq!(pool.len(), side.len());
    let born = pool.len() - old_count;
    let gap = ((4 - (old_count & 3)) & 3).min(born);
    pool[old_count..].rotate_right(gap);
    side[old_count..].rotate_right(gap);
    if mode == RingBufferMode::Disabled || pool.len() <= maximum { return; }
    if maximum == 0 {
        if mode == RingBufferMode::PauseUntilReplaced {
            for (p, s) in pool.iter().zip(side.iter()) { on_death(p, s); }
            pool.clear(); side.clear();
        }
        *cursor = 0; return;
    }
    assert!(*cursor < maximum, "ring cursor must belong to its authored capacity");
    match mode {
        RingBufferMode::PauseUntilReplaced => {
            for source in maximum..pool.len() {
                on_death(&pool[*cursor], &side[*cursor]);
                pool[*cursor] = pool[source].clone();
                side[*cursor] = side[source].clone();
                *cursor = (*cursor + 1) % maximum;
            }
            pool.truncate(maximum); side.truncate(maximum);
        }
        RingBufferMode::LoopUntilReplaced => {
            for source in old_count..pool.len() {
                pool.swap(*cursor, source); side.swap(*cursor, source);
                *cursor = (*cursor + 1) % maximum;
            }
        }
        RingBufferMode::Disabled => unreachable!(),
    }
}

/// Native death compaction scans four-wide groups, removes flagged lanes high
/// to low, and retests the group after swap removal. A reverse scan over the
/// entire pool gives a different order. Loop protects indices < maximum even
/// when explicitly killed. Death never changes the ring cursor.
pub fn compact_with_side<U>(
    pool: &mut Vec<Particle>, side: &mut Vec<U>, mode: RingBufferMode, maximum: usize,
    mut on_death: impl FnMut(&Particle, &U),
) -> usize {
    assert_eq!(pool.len(), side.len());
    if mode == RingBufferMode::PauseUntilReplaced { return 0; }
    let protected = if mode == RingBufferMode::LoopUntilReplaced { maximum } else { 0 };
    let mut group = protected & !3;
    let mut removed = 0;
    while group < pool.len() {
        let end = (group + 4).min(pool.len());
        let dead: [bool; 4] = std::array::from_fn(|lane| {
            let index = group + lane;
            index < end && index >= protected && pool[index].age_percent > 100.0
        });
        if !dead.iter().any(|&value| value) { group += 4; continue; }
        for lane in (0..4).rev() {
            if dead[lane] {
                let index = group + lane;
                on_death(&pool[index], &side[index]);
                pool.swap_remove(index); side.swap_remove(index); removed += 1;
            }
        }
    }
    removed
}

pub fn compact(pool: &mut Vec<Particle>) -> usize {
    let mut side = vec![(); pool.len()];
    compact_with_side(pool, &mut side, RingBufferMode::Disabled, 0, |_, _| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_rotation_and_ring_victims_match_native() {
        for (mode, expected, deaths, cursor) in [
            (RingBufferMode::Disabled, vec![0,1,2,102,100,101], vec![], 0),
            (RingBufferMode::PauseUntilReplaced, vec![100,101,2,102], vec![0,1], 2),
            (RingBufferMode::LoopUntilReplaced, vec![102,100,101,0,1,2], vec![], 3),
        ] {
            let mut pool = vec![0,1,2,100,101,102];
            let mut side = pool.clone(); let mut actual = Vec::new(); let mut c = 0;
            finish_births(&mut pool, &mut side, &mut c, mode, 4, 3, |p,s| {
                assert_eq!(p,s); actual.push(*p);
            });
            assert_eq!(pool, expected); assert_eq!(pool, side);
            assert_eq!(actual, deaths); assert_eq!(c, cursor);
        }
    }
    #[test]
    fn capacity_is_per_batch_and_loop_overflow_uses_it() {
        assert_eq!(birth_capacity(4, RingBufferMode::Disabled, 4, 20), 0);
        assert_eq!(birth_capacity(4, RingBufferMode::PauseUntilReplaced, 4, 20), 4);
        assert_eq!(birth_capacity(7, RingBufferMode::LoopUntilReplaced, 4, 20), 1);
        assert_eq!(birth_capacity(0, RingBufferMode::LoopUntilReplaced, 0, 20), 0);
    }
    #[test]
    fn exact_endpoint_survives_and_loop_protects_only_inner_span() {
        let mut pool: Vec<_> = [110.,110.,100.,110.,110.,90.].map(|age| {
            let mut p = Particle::born([0.;3],[0.;3],1.); p.age_percent=age; p
        }).to_vec();
        let mut side: Vec<_> = (0..pool.len()).collect();
        let mut deaths=Vec::new();
        assert_eq!(compact_with_side(&mut pool, &mut side, RingBufferMode::LoopUntilReplaced,
            1, |_,s| deaths.push(*s)), 3);
        assert_eq!(side, vec![0,5,2]);
        assert_eq!(deaths,vec![3,1,4]);
    }
}
