//! The gate's random appearance of the visiting characters, once per scene
//! setup.
//!
//! Every NPC starts on the entry-site objective: hidden, waiting to be
//! cancelled. At the first environment load of a scene the gate controller
//! places every visiting unit on the home site, shows it and cancels that
//! objective:
//!
//! 1. `n = Random.Range(2, count + 1)`, one engine draw, before any other
//!    key; the gathering is the unit ids ordered by a fresh `Guid` each and
//!    cut to `n` (all of them when `n` is larger);
//! 2. the smallest id of the gathering appears first, at a random walkable
//!    cell corner (the walkable list is rebuilt, one engine pick over it);
//! 3. every other member of the gathering, in the gathering's order, takes a
//!    floor position near the first one ([`target_range_cells`], then the
//!    wander search); an answer of (almost) zero length means none, and the
//!    member takes a random cell instead;
//! 4. the unit ids not in the gathering (distinct, in list order) each take
//!    a random cell.
//!
//! The first one and every random-cell member also raise the flag that
//! skips the next Rest; a member placed near the first one does not.
//!
//! [`EngineRand`] is the engine's scripting generator and its integer
//! range; the ordering and the placement draws are the caller's.

use super::{Cell, TILE_SCALE};

/// The smallest gathering (`Random.Range(2, count + 1)`).
pub const GATHER_MIN: i32 = 2;

/// Squared length under which the near-floor answer counts as none (f32
/// bits `0x2edbe6fe`).
pub const NOT_PLACED_ZERO_BOUND: f32 = 9.9999994e-11;

/// The engine's scripting random generator (a 128-bit xorshift) and its
/// integer range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineRand {
    pub state: [u32; 4],
}

impl EngineRand {
    pub fn from_state(state: [u32; 4]) -> Self {
        Self { state }
    }

    /// One step: the new word, which the state keeps as its last word.
    pub fn next_u32(&mut self) -> u32 {
        let [x, y, z, w] = self.state;
        let mut t = x ^ (x << 11);
        t ^= t >> 8;
        t ^= w;
        t ^= w >> 19;
        self.state = [y, z, w, t];
        t
    }

    /// `Random.Range(int min, int max)`: `min + r % (max - min)` when
    /// `min < max`, `min - r % (min - max)` when `min > max` (the span taken
    /// unsigned), and `min` without a draw when they are equal.
    pub fn range_int(&mut self, min: i32, max: i32) -> i32 {
        if min < max {
            let span = (max as u32).wrapping_sub(min as u32);
            let r = self.next_u32();
            min.wrapping_add((r % span) as i32)
        } else if max < min {
            let span = (min as u32).wrapping_sub(max as u32);
            let r = self.next_u32();
            min.wrapping_sub((r % span) as i32)
        } else {
            min
        }
    }
}

/// Who appears where, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatherPlan {
    /// The gathering: the ordered ids cut to the draw.
    pub gather: Vec<u32>,
    /// The gathering's smallest id: it appears first, at a random cell.
    pub first: u32,
    /// The gathering's other ids (every id not equal to the first), in the
    /// gathering's order: each is placed near the first one.
    pub near: Vec<u32>,
    /// The ids not in the gathering, distinct, in list order: each at a
    /// random cell.
    pub rest: Vec<u32>,
}

/// The plan for `unit_ids` given the gathering size the range draw gave
/// (`take`) and the ordering (`order[k]` = index into `unit_ids` of the
/// k-th id after the `Guid` ordering). `None` when the gathering is empty:
/// its minimum raises, and nothing appears.
pub fn gather_plan(unit_ids: &[u32], take: i32, order: &[usize]) -> Option<GatherPlan> {
    let count = if take <= 0 {
        0
    } else {
        (take as usize).min(order.len())
    };
    let gather: Vec<u32> = order[..count]
        .iter()
        .map(|&index| unit_ids[index])
        .collect();
    let first = *gather.iter().min()?;
    let near = gather.iter().copied().filter(|&id| id != first).collect();
    let mut rest: Vec<u32> = Vec::new();
    for &id in unit_ids {
        if !gather.contains(&id) && !rest.contains(&id) {
            rest.push(id);
        }
    }
    Some(GatherPlan {
        gather,
        first,
        near,
        rest,
    })
}

/// A distance in world units to whole cells, rounded up: an infinite
/// quotient gives `i32::MIN`, anything else the saturating conversion.
fn ceil_cells(distance: f32, tile_scale: f32) -> i32 {
    let cells = (distance / tile_scale).ceil();
    if cells == f32::INFINITY {
        i32::MIN
    } else {
        cells as i32
    }
}

/// `FieldPosition.ToGridPosition` of a site-relative position: each axis
/// divided by the tile size and floored (the height floored from zero up);
/// an infinite quotient gives `i32::MIN`, anything else the saturating
/// conversion. The grid position keeps the low byte of each.
pub fn grid_position(relative: [f32; 3]) -> [i32; 3] {
    let floor_cells = |value: f32| {
        let cells = (value / TILE_SCALE).floor();
        if cells == f32::INFINITY {
            i32::MIN
        } else {
            cells as i32
        }
    };
    [
        floor_cells(relative[0]),
        floor_cells(relative[1].max(0.0)),
        floor_cells(relative[2]),
    ]
}

/// The near-floor search's inputs for a position and distances in world
/// units: the grid cell of `position` minus the site's master origin, and
/// the minimum and maximum ring radii in cells.
pub fn target_range_cells(
    position: [f32; 3],
    origin: [i32; 3],
    min_distance: f32,
    max_distance: f32,
) -> ([i32; 3], i32, i32) {
    let relative = [
        position[0] - origin[0] as f32,
        position[1] - origin[1] as f32,
        position[2] - origin[2] as f32,
    ];
    (
        grid_position(relative),
        ceil_cells(min_distance, TILE_SCALE),
        ceil_cells(max_distance, TILE_SCALE),
    )
}

/// The near-floor answer: the found position, or zero when none.
pub fn not_placed_floor_answer(found: Option<[f32; 3]>) -> [f32; 3] {
    found.unwrap_or([0.0; 3])
}

/// Whether a member takes a random cell instead of the near-floor answer.
pub fn answer_is_none(answer: [f32; 3]) -> bool {
    (answer[0] * answer[0] + answer[1] * answer[1]) + answer[2] * answer[2] < NOT_PLACED_ZERO_BOUND
}

/// `GridPosition.ToFieldPosition` of a floor cell (height byte 0): the
/// cell corner, each axis (a signed byte) times the tile size.
pub fn field_position(cell: Cell) -> [f32; 3] {
    [
        TILE_SCALE * f32::from(cell.0 as i8),
        TILE_SCALE * 0.0,
        TILE_SCALE * f32::from(cell.1 as i8),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zero_bound_is_the_source_word() {
        assert_eq!(NOT_PLACED_ZERO_BOUND.to_bits(), 0x2edb_e6fe);
    }
}
