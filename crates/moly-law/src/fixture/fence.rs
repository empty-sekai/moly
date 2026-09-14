//! FenceConnectData + FenceController (JP 6.7.0 source, CN prefab enums).
//! Neighbors are in source ConnectType order: UR, UL, RU, RD, DR, DL, LU, LD.
//! Grid lookup preserves the first joint's type/direction, while UID overlap
//! considers every joint in the two tiles, as IsSameMysekaiUid does.

use super::{Direction, GridPosition};

#[derive(Clone, Default)]
pub struct Neighbor {
    /// None means no first joint of this fixture kind; otherwise its direction.
    pub direction: Option<Direction>,
    pub joint_uids: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub poles: [bool; 9],
    pub wings: [bool; 9],
}

pub fn connected_parts(direction: Direction, neighbors: &[Neighbor; 8]) -> Parts {
    let horizontal = |d| matches!(d, Direction::Front | Direction::Back);
    let exists = |i: usize| neighbors[i].direction.is_some();
    let parallel = |i: usize| neighbors[i].direction
        .is_some_and(|d| horizontal(d) == horizontal(direction));
    let same = |a: usize, b: usize| neighbors[a].joint_uids.iter()
        .any(|uid| neighbors[b].joint_uids.contains(uid));
    let up = exists(1) && exists(0) && (same(1, 0) || parallel(1) || parallel(0));
    let down = exists(5) && exists(4) && (same(5, 4) || parallel(5) || parallel(4));
    let right = exists(2) && exists(3) && (same(2, 3) || !parallel(2) || !parallel(3));
    let left = exists(6) && exists(7) && (same(6, 7) || !parallel(6) || !parallel(7));
    let right_up = exists(0) && exists(2) && exists(3) && !same(1, 0) && !parallel(2);
    let left_up = exists(1) && exists(6) && exists(7) && !same(1, 0);
    let right_down = exists(4) && exists(3) && exists(2) && !same(5, 4);
    let left_down = exists(5) && exists(7) && exists(6) && !same(5, 4) && !parallel(7);

    // Controller's center predicate differs from CanConnectCenter: it uses XOR
    // for the up/down arms, so a four-way crossing can retain the long wing.
    let center = (up ^ down) || !(right && left);
    let reversed = matches!(direction, Direction::Back | Direction::Right);
    Parts {
        poles: [center, up, down, reversed && right, right_up, right_down,
            !reversed && left, left_up, left_down],
        wings: [!center, right && (up || down || !left), left && (up || down || !right),
            up, down, right_up, right_down, left_up, left_down],
    }
}

/// Exact getters and direction permutation in GetConnectTypeToPosition.
/// The source packed signed bytes wrap at the byte boundary.
pub fn neighbor_positions(min: GridPosition, max: GridPosition, direction: Direction) -> [GridPosition; 8] {
    let at = |x, z| GridPosition { x, y: min.y, z };
    let ur = at(max.x, max.z.wrapping_add(1));
    let ul = at(max.x.wrapping_sub(1), max.z.wrapping_add(1));
    let dr = at(min.x.wrapping_add(1), min.z.wrapping_sub(1));
    let dl = at(min.x, min.z.wrapping_sub(1));
    let ru = at(max.x.wrapping_add(1), max.z);
    let rd = at(max.x.wrapping_add(1), max.z.wrapping_sub(1));
    let lu = at(min.x.wrapping_sub(1), min.z.wrapping_add(1));
    let ld = at(min.x.wrapping_sub(1), min.z);
    match direction {
        Direction::Front => [ur, ul, ru, rd, dr, dl, lu, ld],
        Direction::Left => [rd, ru, dr, dl, ld, lu, ur, ul],
        Direction::Back => [dl, dr, ld, lu, ul, ur, rd, ru],
        Direction::Right => [lu, ld, ul, ur, ru, rd, dl, dr],
    }
}
