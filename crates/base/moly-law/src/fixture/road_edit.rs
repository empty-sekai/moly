//! RoadEditPlanningState's grid planning arithmetic, in source Unity axes.
//! Selection and rendering stay with their owning hosts; this module does not
//! invent collider hits, user inventory or fixture costs.

use super::GridPosition;

fn checked_abs(value: i8) -> Result<i32, &'static str> {
    value
        .checked_abs()
        .map(i32::from)
        .ok_or("Road planning signed-byte Abs overflow")
}

/// SimulateHorizontal includes the endpoint, SimulateVertical excludes it.
/// Each road advances two grid units, independent of its master grid size.
fn axis(
    first: GridPosition,
    delta: GridPosition,
    horizontal: bool,
) -> Result<Vec<GridPosition>, &'static str> {
    let value = if horizontal { delta.x } else { delta.z };
    let length = checked_abs(value)?;
    let mut result = Vec::new();
    let mut step = 0;
    while if horizontal {
        step <= length
    } else {
        step < length
    } {
        let signed = if value > 0 { step } else { -step };
        result.push(
            first
                + if horizontal {
                    GridPosition::new(signed as i8, 0, 0)
                } else {
                    GridPosition::new(0, 0, signed as i8)
                },
        );
        step += 2;
    }
    Ok(result)
}

/// SimulateTapPutAsync uses the dominant axis; ties choose horizontal.
pub fn tap_candidates(
    first: GridPosition,
    last: GridPosition,
) -> Result<Vec<GridPosition>, &'static str> {
    let delta = last - first;
    axis(first, delta, checked_abs(delta.x)? >= checked_abs(delta.z)?)
}

/// UpdateDragTileList: returning inside the first 2x2 bound collapses the
/// history; backtracking removes samples further from the first point.
pub fn update_drag_history(history: &mut Vec<GridPosition>, next: GridPosition) {
    let Some(&first) = history.first() else {
        return;
    };
    if (0..2).contains(&(i16::from(next.x) - i16::from(first.x)))
        && (0..2).contains(&(i16::from(next.z) - i16::from(first.z)))
    {
        history.clear();
        history.push(first);
        return;
    }
    let length = |point: GridPosition| {
        let delta = point - first;
        let x = f32::from(delta.x);
        let y = f32::from(delta.y);
        let z = f32::from(delta.z);
        (x * x + y * y + z * z).sqrt()
    };
    if history.len() >= 2 {
        let distance = length(next);
        history.retain(|&point| length(point) <= distance);
    }
    if history.last() != Some(&next) && !history.contains(&next) {
        history.push(next);
    }
}

/// SimulateDragPutAsync sums the first five samples to choose the bend axis.
/// It then draws the two orthogonal legs, with the turning coordinate rounded
/// toward zero to an even grid displacement by the source's byte expression.
pub fn drag_candidates(history: &[GridPosition]) -> Result<Vec<GridPosition>, &'static str> {
    let Some(&first) = history.first() else {
        return Ok(Vec::new());
    };
    let last = *history.last().expect("history has a first");
    let delta = last - first;
    let accumulated = history
        .iter()
        .take(5)
        .fold(GridPosition::ZERO, |sum, &point| sum + (point - first));
    let horizontal_first = checked_abs(accumulated.x)? >= checked_abs(accumulated.z)?;
    let mut result = axis(first, delta, horizontal_first)?;
    let even = |n: i8| ((i32::from(n) / 2) * 2) as i8;
    let bend = first
        + if horizontal_first {
            GridPosition::new(even(delta.x), 0, 0)
        } else {
            GridPosition::new(0, 0, even(delta.z))
        };
    result.extend(axis(bend, delta, !horizontal_first)?);
    Ok(result)
}

/// Source GetSelectTileOnField after a verified RaycastTargetGroup hit has
/// been converted to site-local source coordinates. Y is clamped above zero
/// by FieldPosition.ToGridPosition; invalid byte-domain input is refused.
pub fn field_hit_grid(point: [f32; 3], tile: [f32; 3]) -> Option<GridPosition> {
    raw_field_hit_grid(
        [point[0] - tile[0] * 0.5, point[1], point[2] - tile[2] * 0.5],
        tile,
    )
}

/// SiteLayoutUtility.GetTouchedTile, used by selection rectangles, omits
/// the road planner's half-tile offset.
pub fn raw_field_hit_grid(point: [f32; 3], tile: [f32; 3]) -> Option<GridPosition> {
    if point.iter().any(|v| !v.is_finite()) || tile.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return None;
    }
    let v = [
        (point[0] / tile[0]).floor(),
        (point[1].max(0.) / tile[1]).floor(),
        (point[2] / tile[2]).floor(),
    ];
    if v.iter().any(|n| *n < i8::MIN as f32 || *n > i8::MAX as f32) {
        return None;
    }
    Some(GridPosition::new(v[0] as i8, v[1] as i8, v[2] as i8))
}

/// Contiguous source placement-effect segments. The next axis change starts
/// a fresh segment; the preceding segment retains its own size and mean.
pub fn effect_segments(
    rows: &[(GridPosition, super::Vector3Int)],
) -> Vec<(std::ops::Range<usize>, super::Vector3Int)> {
    let mut output = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let mut direction = 0;
        let mut size = rows[start].1;
        let mut end = start + 1;
        while end < rows.len() {
            let (previous, current) = (rows[end - 1].0, rows[end].0);
            let x = previous.x != current.x;
            let z = previous.z != current.z;
            let next = match (x, z) {
                (true, true) => 3 - direction,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            };
            if next == 3 || direction != 0 && direction != next {
                break;
            }
            if next == 1 {
                size.x += (i32::from(current.x) - i32::from(previous.x)).abs();
            }
            if next == 2 {
                size.z += (i32::from(current.z) - i32::from(previous.z)).abs();
            }
            direction = next;
            end += 1;
        }
        output.push((start..end, size));
        start = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: i8, z: i8) -> GridPosition {
        GridPosition::new(x, 0, z)
    }

    #[test]
    fn source_tap_axis_endpoint_asymmetry() {
        assert_eq!(
            tap_candidates(p(0, 0), p(4, 1)).unwrap(),
            vec![p(0, 0), p(2, 0), p(4, 0)]
        );
        assert_eq!(
            tap_candidates(p(0, 0), p(1, 4)).unwrap(),
            vec![p(0, 0), p(0, 2)]
        );
        assert_eq!(
            tap_candidates(p(0, 0), p(-3, 1)).unwrap(),
            vec![p(0, 0), p(-2, 0)]
        );
    }

    #[test]
    fn drag_bend_follows_early_samples_and_signed_even_step() {
        assert_eq!(
            drag_candidates(&[p(0, 0), p(2, 0), p(4, 4)]).unwrap(),
            vec![p(0, 0), p(2, 0), p(4, 0), p(4, 0), p(4, 2)]
        );
        assert_eq!(
            drag_candidates(&[p(0, 0), p(0, 2), p(-3, 4)]).unwrap(),
            vec![p(0, 0), p(0, 2), p(0, 4), p(-2, 4)]
        );
        // The repeated bend is rejected by CanPutRoad's planning-grid pass.
    }

    #[test]
    fn backtracking_and_field_hit_preserve_source_grid() {
        let mut history = vec![p(0, 0), p(2, 0), p(6, 0)];
        update_drag_history(&mut history, p(3, 0));
        assert_eq!(history, vec![p(0, 0), p(2, 0), p(3, 0)]);
        update_drag_history(&mut history, p(1, 1));
        assert_eq!(history, vec![p(0, 0)]);
        assert_eq!(field_hit_grid([0., -0.01, 0.5], [0.25; 3]), Some(p(-1, 1)));
    }
    #[test]
    fn effect_segments_split_turns_without_inflating_the_empty_corner() {
        let size = super::super::Vector3Int::new(2, 1, 2);
        let rows = [
            (p(0, 0), size),
            (p(2, 0), size),
            (p(2, 2), size),
            (p(2, 6), size),
        ];
        let segments = effect_segments(&rows);
        assert_eq!(
            segments,
            vec![
                (0..2, super::super::Vector3Int::new(4, 1, 2)),
                (2..4, super::super::Vector3Int::new(2, 1, 6))
            ]
        );
        assert_eq!(raw_field_hit_grid([0., 0., 0.5], [0.25; 3]), Some(p(0, 2)));
    }
}
