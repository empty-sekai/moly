//! The engine's particle storage past the live count.
//!
//! The engine's module loops run in four-lane groups from the start of their
//! range while the group starts before its end, so the last group of a call
//! also reads, and SimulateParticles also writes, the slots past the live
//! count. CustomDataModule evaluates its curves there, and each such
//! evaluation reads and may rewrite the curve object's cache
//! ([`crate::particle::curve::CurveCache`]) that the next call reads.
//!
//! Those slots hold what the storage operations left, and nothing clears
//! them. KillParticle copies the particle in the last slot into the dead slot
//! and lowers the count, so the last slot keeps what it held. A birth writes
//! whole four-lane groups from the four-aligned end of the live particles;
//! the newborn death pass kills the same way inside that span; packing then
//! copies the last surviving newborns into the alignment gap before them.
//! Clear only resets the count. [`SlotTail`] carries these slots for a
//! runtime whose live particles follow the engine's slot order, in the order
//! the runtime applies those operations.
use std::collections::VecDeque;

use crate::particle::buffer::RingBufferMode;
use crate::particle::step::{advance_lifetime, Particle};

/// Play reserves the particle storage once, for the smaller of the authored
/// maximum (doubled in a ring mode) and `CalculateMaxActiveParticles`'
/// estimate (at most 50000), rounded up to a multiple of 32 particles. The
/// estimate is the ceiling of the largest start lifetime times the largest
/// emission rate, plus the largest burst count within one lifetime. With a
/// positive maximum and a positive estimate the first 32 slots always lie in
/// that reservation, so no birth writing below them grows the storage.
pub const RESERVED_SLOTS: usize = 32;

/// Slots `count, count + 1, ..` of the engine's particle storage, where
/// `count` is the runtime's live count. Only slots some operation wrote are
/// held; the storage a reservation leaves unwritten past them is not.
#[derive(Clone, Debug)]
pub struct SlotTail {
    slots: VecDeque<Particle>,
    /// The live count the operations left.
    live: usize,
    /// The slots known to lie in the storage Play reserved.
    capacity: usize,
}

/// Why the tail could not follow an operation. The caller must stop
/// simulating the system: the slots past the live count are then unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailRefused {
    /// A four-lane group reads a slot no operation wrote.
    Unwritten,
    /// The runtime's kills or packed newborns are not the order this model
    /// follows, or its live count is not the one the operations left (the
    /// caller changed its storage without reporting it).
    Order,
    /// A birth writes past the slots known to be reserved: the storage may
    /// grow there, and what a growth keeps past the live count is not read.
    Capacity,
}

fn same(a: &Particle, b: &Particle) -> bool {
    let bits = |p: &Particle| (p.position.map(f32::to_bits), p.velocity.map(f32::to_bits),
        p.start_lifetime.to_bits(), p.age_percent.to_bits(), p.inverse_lifetime.to_bits());
    bits(a) == bits(b)
}

fn same_all(a: &[Particle], b: &[Particle]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
}

impl SlotTail {
    /// An empty system whose storage reservation holds at least `capacity`
    /// slots ([`RESERVED_SLOTS`]).
    pub fn new(capacity: usize) -> Self {
        Self { slots: VecDeque::new(), live: 0, capacity }
    }

    /// The slot `count + k`, as far as it was written.
    pub fn slot(&self, k: usize) -> Option<&Particle> {
        self.slots.get(k)
    }

    /// The live count the operations left.
    pub fn live(&self) -> usize {
        self.live
    }

    /// The lanes the last four-lane group of a call over `[0, count)` reads past
    /// `count`: none when `count` is a multiple of four (zero included: the
    /// engine skips an empty range).
    pub fn padding(&self, count: usize) -> Result<Vec<Particle>, TailRefused> {
        if count != self.live {
            return Err(TailRefused::Order);
        }
        let lanes = (4 - count % 4) % 4;
        if self.slots.len() < lanes {
            return Err(TailRefused::Unwritten);
        }
        Ok(self.slots.iter().take(lanes).copied().collect())
    }

    /// SimulateParticles over those lanes: the age advances as the live lanes'
    /// ages do ([`advance_lifetime`] with the same mode and loop range).
    pub fn advance_padding(&mut self, count: usize, dt: f32, mode: RingBufferMode, loop_range: [f32; 2]) {
        let lanes = (4 - count % 4) % 4;
        for slot in self.slots.iter_mut().take(lanes) {
            let _ = advance_lifetime(slot, dt, mode, loop_range);
        }
    }

    /// A kill pass over the live particles. `removed` holds the slot of each
    /// killed particle in removal order, each removal a swap with the last live
    /// particle: that last slot then keeps its content past the new count.
    /// `after` is the runtime's live particles after the pass.
    pub fn kill(&mut self, before: &[Particle], removed: &[usize], after: &[Particle]) -> Result<(), TailRefused> {
        if before.len() != self.live {
            return Err(TailRefused::Order);
        }
        let mut live = before.to_vec();
        for &index in removed {
            if index >= live.len() {
                return Err(TailRefused::Order);
            }
            let last = live[live.len() - 1];
            live.swap_remove(index);
            self.slots.push_front(last);
        }
        if !same_all(&live, after) {
            return Err(TailRefused::Order);
        }
        self.live = after.len();
        Ok(())
    }

    /// One birth of `lanes.len()` lanes (whole four-lane groups, the padding
    /// lanes of the last group included) after `old` live particles, as the
    /// lanes are after their newborn modules and ages and before the newborn
    /// death pass. The engine writes them from the slot `old` rounded up to
    /// four. The death pass scans them forward, retesting a slot after each
    /// removal, and removes an age above 100 by a swap with the last lane; the
    /// `live` first survivors stay live, and packing copies the last
    /// `min(gap, live)` of them into the gap below that slot. `after` is the
    /// runtime's newborns after packing.
    pub fn birth(&mut self, old: usize, lanes: &[Particle], live: usize, after: &[Particle])
        -> Result<(), TailRefused> {
        if old != self.live {
            return Err(TailRefused::Order);
        }
        if old.next_multiple_of(4) + lanes.len() > self.capacity {
            return Err(TailRefused::Capacity);
        }
        let gap = old.next_multiple_of(4) - old;
        if self.slots.len() < gap {
            return Err(TailRefused::Unwritten);
        }
        let gap_slots: Vec<Particle> = self.slots.drain(..gap).collect();
        let overwritten = lanes.len().min(self.slots.len());
        self.slots.drain(..overwritten);
        let mut survivors = lanes.to_vec();
        let mut popped = Vec::new();
        let mut index = 0;
        while index < survivors.len() {
            if survivors[index].age_percent > 100.0 {
                popped.push(survivors[survivors.len() - 1]);
                survivors.swap_remove(index);
            } else {
                index += 1;
            }
        }
        if live > survivors.len() {
            return Err(TailRefused::Order);
        }
        let copied = gap.min(live);
        let mut packed = survivors[..live].to_vec();
        packed.rotate_right(copied);
        if !same_all(&packed, after) {
            return Err(TailRefused::Order);
        }
        let mut tail: VecDeque<Particle> = VecDeque::new();
        if live >= gap {
            tail.extend(&survivors[live - gap..live]);
        } else {
            tail.extend(&gap_slots[live..]);
            tail.extend(&survivors[..live]);
        }
        tail.extend(&survivors[live..]);
        tail.extend(popped.iter().rev());
        tail.extend(self.slots.drain(..));
        self.slots = tail;
        self.live = old + live;
        Ok(())
    }

    /// Clear: the count returns to zero and the live particles' slots keep
    /// their content.
    pub fn clear(&mut self, live: &[Particle]) -> Result<(), TailRefused> {
        if live.len() != self.live {
            return Err(TailRefused::Order);
        }
        for particle in live.iter().rev() {
            self.slots.push_front(*particle);
        }
        self.live = 0;
        Ok(())
    }
}
