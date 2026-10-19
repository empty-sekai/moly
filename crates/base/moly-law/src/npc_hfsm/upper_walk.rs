//! `RandomWalkUpperState`'s position pick: the avoidance direction and the
//! ten-attempt target search.
//!
//! The pick has no navigation of its own: a surface sample comes from the
//! caller, one `NavMesh.SamplePosition` per attempt. The five first attempts
//! avoid the neighbours within the band and use a direction plus a
//! `Range(2.5, 5)` length; the later attempts, and every attempt while no
//! neighbour is in the band, use a point on the unit circle scaled by the
//! walk radius. A sampled point is accepted at once when the character has no
//! neighbour to avoid, otherwise only when it lies at least a metre away; the
//! walk falls back to the character's own position when no attempt lands.
//!
//! `GetAvoidDirection` sums the unit away-vectors of every other character
//! whose squared distance is inside `(0.001, 2.25)`, x and z only.

use super::timing::{
    AVOID_NEAR_MAX, AVOID_NEAR_MIN, RANDOM_WALK_ATTEMPTS, RANDOM_WALK_AVOID_ATTEMPTS,
    RANDOM_WALK_RADIUS,
};

/// The surface sample distance of every attempt (`NavMesh.SamplePosition`).
pub const RANDOM_WALK_SAMPLE_TOLERANCE: f32 = 1.0;
/// The squared avoid direction below which the character counts as having
/// nobody to avoid.
pub const AVOID_DIRECTION_EPSILON: f32 = 1.0e-10;
/// The distance a sampled point must have from the character for the avoid
/// branch to accept it.
pub const RANDOM_WALK_MIN_ACCEPT_DISTANCE: f32 = 1.0;
/// The direction term's scale inside the sphere sample.
pub const AVOID_SAMPLE_SCALE: f32 = 0.5;
/// `Vector3.normalize`'s length below which the direction is zero.
pub const NORMALIZE_EPSILON: f32 = 0.00001;

/// The engine draws the pick spends.
pub trait RandomWalkDraw {
    /// `Random.insideUnitCircle`.
    fn unit_circle(&mut self) -> [f32; 2];
    /// `Random.insideUnitSphere`.
    fn unit_sphere(&mut self) -> [f32; 3];
    /// `Random.Range(min, max)`.
    fn range(&mut self, min: f32, max: f32) -> f32;
}

/// `GetAvoidDirection`: the summed away direction of the other characters in
/// the near band, x and z only.
pub fn avoid_direction(position: [f32; 3], others: &[[f32; 3]]) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    for other in others {
        let dx = position[0] - other[0];
        let dy = position[1] - other[1];
        let dz = position[2] - other[2];
        let squared = dx * dx + dy * dy + dz * dz;
        if squared < AVOID_NEAR_MAX * AVOID_NEAR_MAX && squared > AVOID_NEAR_MIN * AVOID_NEAR_MIN
        {
            let length = squared.sqrt();
            if length > NORMALIZE_EPSILON {
                sum[0] += dx / length;
                sum[2] += dz / length;
            }
        }
    }
    sum
}

/// Whether the character has somebody to avoid: the direction's xz squared
/// length is at or above the epsilon.
pub fn is_avoiding(direction: [f32; 3]) -> bool {
    direction[0] * direction[0] + direction[2] * direction[2] >= AVOID_DIRECTION_EPSILON
}

/// `PickRandomPosition`: up to ten samples around `position`, the accepted
/// hit, else `position`.
pub fn pick_target(
    draw: &mut dyn RandomWalkDraw,
    position: [f32; 3],
    others: &[[f32; 3]],
    sample: &mut dyn FnMut([f32; 3], f32) -> Option<[f32; 3]>,
) -> [f32; 3] {
    let direction = avoid_direction(position, others);
    let avoiding = is_avoiding(direction);
    let mut accepted: Option<[f32; 3]> = None;
    for attempt in 0..RANDOM_WALK_ATTEMPTS {
        let target = if attempt >= RANDOM_WALK_AVOID_ATTEMPTS || !avoiding {
            let unit = draw.unit_circle();
            [
                position[0] + unit[0] * RANDOM_WALK_RADIUS,
                position[1],
                position[2] + unit[1] * RANDOM_WALK_RADIUS,
            ]
        } else {
            let sphere = draw.unit_sphere();
            let combined = [
                direction[0] + sphere[0] * AVOID_SAMPLE_SCALE,
                0.0,
                direction[2] + sphere[2] * AVOID_SAMPLE_SCALE,
            ];
            let length = (combined[0] * combined[0] + combined[2] * combined[2]).sqrt();
            let unit = if length <= NORMALIZE_EPSILON {
                [0.0, 0.0, 0.0]
            } else {
                [combined[0] / length, 0.0, combined[2] / length]
            };
            let distance = draw.range(2.5, 5.0);
            [
                position[0] + unit[0] * distance,
                position[1],
                position[2] + unit[2] * distance,
            ]
        };
        if let Some(hit) = sample(target, RANDOM_WALK_SAMPLE_TOLERANCE) {
            if !avoiding {
                accepted = Some(hit);
                break;
            }
            let dx = hit[0] - position[0];
            let dy = hit[1] - position[1];
            let dz = hit[2] - position[2];
            if (dx * dx + dy * dy + dz * dz).sqrt() >= RANDOM_WALK_MIN_ACCEPT_DISTANCE {
                accepted = Some(hit);
                break;
            }
        }
    }
    accepted.unwrap_or(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The draws the pick spends, recorded in order.
    struct Draws {
        circle: [f32; 2],
        sphere: [f32; 3],
        range: f32,
        calls: Vec<&'static str>,
    }

    impl Draws {
        fn new() -> Self {
            Self {
                circle: [0.6, 0.8],
                sphere: [0.0, 0.0, 0.0],
                range: 4.0,
                calls: Vec::new(),
            }
        }
    }

    impl RandomWalkDraw for Draws {
        fn unit_circle(&mut self) -> [f32; 2] {
            self.calls.push("circle");
            self.circle
        }

        fn unit_sphere(&mut self) -> [f32; 3] {
            self.calls.push("sphere");
            self.sphere
        }

        fn range(&mut self, _min: f32, _max: f32) -> f32 {
            self.calls.push("range");
            self.range
        }
    }

    #[test]
    fn the_avoid_band_is_the_source_square_band() {
        // 0.001 < d² < 2.25, y included; the away vector is x/z only.
        let centre = [1.0, 0.0, 1.0];
        let inside = [[1.0, 0.0, 1.5], [1.0, 0.5, 1.0]];
        let near = avoid_direction(centre, &inside);
        // d = (0,0,-0.5) gives (0,0,-1); d = (0,-0.5,0) has no xz term.
        assert_eq!(near, [0.0, 0.0, -1.0]);
        // d² = 0.001 is the open edge, 2.25 the far one.
        let edge = [[1.0 - 0.031_622_776, 0.0, 1.0]];
        assert!(!is_avoiding(avoid_direction(centre, &edge)));
        let far = [[1.0 - 1.5, 0.0, 1.0]];
        assert!(!is_avoiding(avoid_direction(centre, &far)));
        let band = [[1.0 - 1.0, 0.0, 1.0]];
        assert!(is_avoiding(avoid_direction(centre, &band)));
    }

    #[test]
    fn nobody_in_the_band_takes_the_circle_branch_and_accepts_the_first_hit() {
        let mut draw = Draws::new();
        let mut samples = 0;
        let mut sample = |_point: [f32; 3], tolerance: f32| {
            samples += 1;
            assert_eq!(tolerance, RANDOM_WALK_SAMPLE_TOLERANCE);
            Some([4.0, 0.0, 5.0])
        };
        let target = pick_target(&mut draw, [1.0, 0.0, 1.0], &[], &mut sample);
        assert_eq!(draw.calls, vec!["circle"]);
        assert_eq!(samples, 1);
        assert_eq!(target, [4.0, 0.0, 5.0]);
    }

    #[test]
    fn the_avoid_branch_spends_two_draws_and_needs_a_metre() {
        let mut draw = Draws::new();
        let mut calls = 0;
        let mut sample = |_point: [f32; 3], _tolerance: f32| {
            calls += 1;
            // The first sample lands at the character: inside the metre, so
            // the attempt is not accepted.
            if calls == 1 {
                Some([1.0, 0.0, 1.0])
            } else {
                Some([5.0, 0.0, 1.0])
            }
        };
        let target = pick_target(&mut draw, [1.0, 0.0, 1.0], &[[1.0, 0.0, 1.5]], &mut sample);
        assert_eq!(target, [5.0, 0.0, 1.0]);
        assert_eq!(calls, 2);
        // Two draws per avoid attempt: the sphere sample and the length.
        assert_eq!(draw.calls, vec!["sphere", "range", "sphere", "range"]);
    }

    #[test]
    fn the_avoid_branch_is_the_first_five_attempts() {
        let mut draw = Draws::new();
        let mut sample = |_point: [f32; 3], _tolerance: f32| None;
        let target = pick_target(&mut draw, [0.0, 0.0, 0.0], &[[0.0, 0.0, 1.0]], &mut sample);
        assert_eq!(target, [0.0, 0.0, 0.0]);
        // Five avoid attempts (two draws each), then five plain attempts
        // (one draw each).
        assert_eq!(
            draw.calls,
            vec![
                "sphere", "range", "sphere", "range", "sphere", "range", "sphere", "range",
                "sphere", "range", "circle", "circle", "circle", "circle", "circle"
            ]
        );
    }

    #[test]
    fn a_hit_below_the_metre_is_never_accepted_and_the_position_stands() {
        let mut draw = Draws::new();
        let mut sample = |_point: [f32; 3], _tolerance: f32| Some([0.5, 0.0, 0.0]);
        let target = pick_target(&mut draw, [0.0, 0.0, 0.0], &[[0.0, 0.0, 1.0]], &mut sample);
        assert_eq!(target, [0.0, 0.0, 0.0]);
        assert_eq!(draw.calls.len(), 15);
    }
}
