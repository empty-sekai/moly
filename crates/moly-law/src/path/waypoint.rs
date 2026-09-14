//! Ordered navigation checkpoints and authored rest-point generation.
//!
//! A navigation corner is not a rest instruction. The random side candidates
//! are the only Rest points inserted by this path-building operation. Surface
//! sampling and integer draws are supplied by the host; sampling admits the
//! candidate without replacing it with the sampled position.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WaypointKind {
    Rest = 0,
    CheckPoint = 1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub position: [f32; 3],
    pub kind: WaypointKind,
}

/// One integer draw in [0, upper_exclusive). Called only with a nonempty range.
pub trait WaypointDraw {
    fn index(&mut self, upper_exclusive: usize) -> usize;
}

pub const WAYPOINT_SAMPLE_DISTANCE: f32 = 0.25;

/// Generate the complete route before execution. `corners` is the navigation
/// query's unmodified path, including its first corner; `start` is inserted
/// separately even when those two points coincide.
pub fn build_waypoints(
    start: [f32; 3],
    corners: &[[f32; 3]],
    forward: [f32; 3],
    draw: &mut impl WaypointDraw,
    mut is_navigable: impl FnMut([f32; 3]) -> bool,
) -> Vec<Waypoint> {
    if corners.is_empty() {
        return Vec::new();
    }
    let mut path = Vec::with_capacity(corners.len() + 1);
    path.push(start);
    path.extend_from_slice(corners);
    let lengths: Vec<f32> = path.windows(2).map(|pair| distance(pair[0], pair[1])).collect();
    let total: f32 = lengths.iter().sum();
    let spacing = (3 + draw.index(2)) as f32;
    let divisions = (total / spacing).round_ties_even() as i32;
    let divided = divide_path(&path, &lengths, total, divisions);
    let directions = [
        rotate_y(forward, -core::f32::consts::FRAC_PI_4),
        rotate_y(forward, core::f32::consts::FRAC_PI_4),
    ];
    let mut rests = Vec::new();
    for position in divided {
        let mut candidates = Vec::new();
        // Repeated f32 addition is significant at the inclusive upper bound.
        let mut scale = 0.2_f32;
        while scale <= 0.5 {
            for direction in directions {
                let candidate = [
                    position[0] + direction[0] * scale,
                    position[1] + direction[1] * scale,
                    position[2] + direction[2] * scale,
                ];
                if is_navigable(candidate) {
                    candidates.push(candidate);
                }
            }
            scale += 0.1;
        }
        if candidates.is_empty() {
            // The empty-list branch adds the original split point, then also
            // adds RandomPick's default vector. Empty RandomPick draws nothing.
            rests.push(position);
            rests.push([0.0; 3]);
        } else {
            rests.push(candidates[draw.index(candidates.len())]);
        }
    }
    let first_corner_distance = distance(start, path[1]);
    let mut waypoints: Vec<_> = path.into_iter().map(|position| Waypoint {
        position,
        kind: WaypointKind::CheckPoint,
    }).collect();
    waypoints.extend(rests.into_iter()
        .filter(|position| distance(start, *position) > first_corner_distance)
        .map(|position| Waypoint { position, kind: WaypointKind::Rest }));
    // Stable radial ordering, not cumulative route-distance ordering. Original
    // checkpoints precede side candidates when their distance keys tie.
    waypoints.sort_by(|a, b| distance(start, a.position)
        .partial_cmp(&distance(start, b.position)).unwrap_or(core::cmp::Ordering::Equal));
    waypoints
}

fn divide_path(path: &[[f32; 3]], lengths: &[f32], total: f32, divisions: i32) -> Vec<[f32; 3]> {
    let mut points = Vec::new();
    for division in 1..divisions {
        let required = (total / divisions as f32) * division as f32;
        let mut preceding = 0.0;
        for (index, &length) in lengths.iter().enumerate() {
            if required <= preceding + length {
                let t = ((required - preceding) / length).clamp(0.0, 1.0);
                let from = path[index];
                let to = path[index + 1];
                points.push([
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                    from[2] + (to[2] - from[2]) * t,
                ]);
                break;
            }
            preceding += length;
        }
    }
    points
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let x = b[0] - a[0];
    let y = b[1] - a[1];
    let z = b[2] - a[2];
    (x * x + y * y + z * z).sqrt()
}

fn rotate_y(vector: [f32; 3], angle: f32) -> [f32; 3] {
    let (y, w) = (angle * 0.5).sin_cos();
    let twice_y = y * 2.0;
    let yy = y * twice_y;
    let wy = w * twice_y;
    [
        (1.0 - yy) * vector[0] + wy * vector[2],
        vector[1],
        -wy * vector[0] + (1.0 - yy) * vector[2],
    ]
}
