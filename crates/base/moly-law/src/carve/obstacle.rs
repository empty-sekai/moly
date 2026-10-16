//! Runtime NavMeshObstacle carving, as the engine does it after the bake.
//!
//! An enabled obstacle with carving on registers with the navigation
//! manager; every navigation update runs `NavMeshObstacle::UpdateState` and
//! then adds the obstacle's carve shape to the carving system while it is
//! stationary, or removes it while it moves ([`ObstacleState`]). The carving
//! system rebuilds each tile the shapes touch from the tile's baked data:
//! `CarveNavMeshTile` turns every shape into a convex hull of planes
//! ([`carve_points`] then [`carve_hull`]) and clips the tile's polygons
//! against it (`DynamicMesh::ClipPolys`: the part of each polygon inside the
//! hull is removed). Removing the obstacle restores the tile from the baked
//! data with the remaining shapes.
//!
//! Frames: everything here is the engine's left-handed frame. The caller
//! converts from the moly frame (x reflected) with [`CarveShape::from_moly`]
//! and converts the planes back with [`CarveHull::to_moly`]; negation is
//! exact, so the plane tests give the same bits in either frame.

/// `NavMeshObstacleShape`: capsule 0, box 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarveKind {
    Capsule,
    Box,
}

/// `NavMeshCarveShape` as `NavMeshObstacle::GetCarveShape` fills it: the
/// world extents (`GetWorldExtents`), the world centre and the rotation's
/// axes (`GetWorldCenterAndAxes`), and the world bounds (centre ± the
/// capsule's or box's world half size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarveShape {
    pub kind: CarveKind,
    pub center: [f32; 3],
    pub extents: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

/// A plane `normal · p + distance`; the hull's inside is where every plane
/// gives at most zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub distance: f32,
}

/// The carve hull of one shape: its planes in the engine's order (one
/// vertical plane per edge of the expanded outline, then the bottom and top
/// planes, then for a tilted shape the bounds' bottom and top) and its bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct CarveHull {
    pub planes: Vec<Plane>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

/// `Vector3f::epsilon`.
const EPSILON: f32 = 1e-5;
/// The outline corner rule's threshold, the literal `0xbae4c694`.
const SHARP_CORNER: f32 = f32::from_bits(0xbae4_c694);
/// `cos 10°` (`0x3f7c1c5c`): above it the up axis counts as vertical.
const COS_10: f32 = f32::from_bits(0x3f7c_1c5c);
/// `tan 10°` (`0x3e348f0f`).
const TAN_10: f32 = f32::from_bits(0x3e34_8f0f);
/// The capsule constants the carve loads: `√½` and the octagon's
/// circumscription factor `1/cos(π/8)`.
const HALF_SQRT2: f32 = f32::from_bits(0x3f35_04f3);
const OCTAGON: f32 = f32::from_bits(0x3f8a_8bd4);

impl CarveShape {
    /// `GetCarveShape` from the obstacle's serialized fields and its node's
    /// world transform, all in the engine's frame: `center` the world centre
    /// (`TransformPoint` of the local centre), `axes` the world rotation's
    /// columns, `lossy_scale` the node's lossy world scale, `extents` the
    /// serialized extents (a capsule reads x as its radius and y as its half
    /// height).
    pub fn new(
        kind: CarveKind,
        center: [f32; 3],
        axes: [[f32; 3]; 3],
        lossy_scale: [f32; 3],
        extents: [f32; 3],
    ) -> CarveShape {
        let scale = lossy_scale.map(f32::abs);
        let world = match kind {
            // GetWorldExtents: the capsule shares max(|x|, |z|) between its
            // two radii; the box scales each axis by its own.
            CarveKind::Capsule => {
                let radius = if scale[0] < scale[2] {
                    scale[2]
                } else {
                    scale[0]
                } * extents[0];
                [radius, scale[1] * extents[1], radius]
            }
            CarveKind::Box => [
                scale[0] * extents[0],
                scale[1] * extents[1],
                scale[2] * extents[2],
            ],
        };
        let half = match kind {
            CarveKind::Capsule => capsule_world_extents(world, axes[1]),
            CarveKind::Box => box_world_extents(world, axes),
        };
        CarveShape {
            kind,
            center,
            extents: world,
            axes,
            bounds_min: [
                center[0] - half[0],
                center[1] - half[1],
                center[2] - half[2],
            ],
            bounds_max: [
                center[0] + half[0],
                center[1] + half[1],
                center[2] + half[2],
            ],
        }
    }

    /// [`Self::new`] from a moly-frame centre, axes and scale: reflects x
    /// into the engine's frame first (the x axis column changes sign as a
    /// whole; every other column reflects its x component).
    pub fn from_moly(
        kind: CarveKind,
        center: [f32; 3],
        axes: [[f32; 3]; 3],
        lossy_scale: [f32; 3],
        extents: [f32; 3],
    ) -> CarveShape {
        let reflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let x = reflect(axes[0]).map(|c| -c);
        CarveShape::new(
            kind,
            reflect(center),
            [x, reflect(axes[1]), reflect(axes[2])],
            lossy_scale,
            extents,
        )
    }
}

/// `CalcCapsuleWorldExtents`: `r + |yAxis|·max(halfHeight − r, 0)`, with
/// `r = max(extents.x, extents.z)`.
fn capsule_world_extents(extents: [f32; 3], up: [f32; 3]) -> [f32; 3] {
    let radius = if extents[0] < extents[2] {
        extents[2]
    } else {
        extents[0]
    };
    let segment = engine_fmaxnm(extents[1] - radius, 0.0);
    [
        radius + up[0].abs() * segment,
        radius + up[1].abs() * segment,
        radius + segment * up[2].abs(),
    ]
}

/// `CalcBoxWorldExtents`: `|x|·e.x + |y|·e.y + |z|·e.z` per component.
fn box_world_extents(extents: [f32; 3], axes: [[f32; 3]; 3]) -> [f32; 3] {
    let [x, y, z] = axes;
    let mut out = [0.0; 3];
    for c in 0..2 {
        out[c] = x[c].abs() * extents[0] + y[c].abs() * extents[1] + z[c].abs() * extents[2];
    }
    out[2] = extents[0] * x[2].abs() + extents[1] * y[2].abs() + extents[2] * z[2].abs();
    out
}

/// ARM `fmaxnm`: the larger, a NaN operand loses to a number.
fn engine_fmaxnm(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        b
    } else if b.is_nan() || a > b {
        a
    } else {
        b
    }
}

/// The points `CarveNavMeshTile` hands to the hull, relative to the tile
/// position `pos`: a capsule gives 8 rings of 4 (an equator pair at the
/// cylinder's ends and a mid-latitude pair, on an octagon circumscribing the
/// radius) and the two poles; a box gives its 8 corners, corner `i` taking
/// `+` on axis `k` when bit `k` of `i` is set.
pub fn carve_points(shape: &CarveShape, pos: [f32; 3]) -> Vec<[f32; 3]> {
    let [ax, ay, az] = shape.axes;
    let c = [
        shape.center[0] - pos[0],
        shape.center[1] - pos[1],
        shape.center[2] - pos[2],
    ];
    let e = shape.extents;
    match shape.kind {
        CarveKind::Box => (0..8)
            .map(|i| {
                let sx = if i & 1 == 0 { -e[0] } else { e[0] };
                let sy = if i & 2 == 0 { -e[1] } else { e[1] };
                let sz = if i & 4 == 0 { -e[2] } else { e[2] };
                let mut p = [0.0; 3];
                for k in 0..3 {
                    p[k] = c[k] + ax[k] * sx + ay[k] * sy + az[k] * sz;
                }
                p
            })
            .collect(),
        CarveKind::Capsule => {
            let radius = if e[0] < e[2] { e[2] } else { e[0] };
            let segment = engine_fmaxnm(e[1] - radius, 0.0);
            let ring = radius * OCTAGON;
            let middle = (radius * HALF_SQRT2) * OCTAGON;
            let middle_reach = middle + segment;
            let up_segment = ay.map(|v| v * segment);
            let up_middle = ay.map(|v| v * middle_reach);
            let mut points = Vec::with_capacity(34);
            for step in 0..8 {
                let angle = ((step as f32 * 0.125) * std::f32::consts::PI) * 2.0;
                let (sin, cos) = angle.sin_cos();
                let mut equator = [0.0; 3];
                let mut mid = [0.0; 3];
                for k in 0..3 {
                    let x_cos = ax[k] * cos;
                    let z_sin = az[k] * sin;
                    equator[k] = ring * z_sin + (c[k] + ring * x_cos);
                    mid[k] = middle * z_sin + (c[k] + middle * x_cos);
                }
                points.push(std::array::from_fn(|k| equator[k] - up_segment[k]));
                points.push(std::array::from_fn(|k| up_segment[k] + equator[k]));
                points.push(std::array::from_fn(|k| mid[k] - up_middle[k]));
                points.push(std::array::from_fn(|k| up_middle[k] + mid[k]));
            }
            let pole = ring + segment;
            points.push(std::array::from_fn(|k| c[k] - pole * ay[k]));
            points.push(std::array::from_fn(|k| c[k] + pole * ay[k]));
            points
        }
    }
}

/// `CalculateConvexHull`: Andrew's monotone chain over points sorted by x
/// then y, clockwise in (x, z); a point is popped unless the turn is strictly
/// clockwise. The closing duplicate is dropped.
pub fn convex_hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| {
        if a[0] < b[0] {
            std::cmp::Ordering::Less
        } else if a[0] != b[0] {
            std::cmp::Ordering::Greater
        } else if a[1] < b[1] {
            std::cmp::Ordering::Less
        } else if b[1] < a[1] {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(sorted.len() + 1);
    if sorted.is_empty() {
        return hull;
    }
    let keeps = |hull: &[[f32; 2]], p: [f32; 2]| {
        let a = hull[hull.len() - 2];
        let b = hull[hull.len() - 1];
        let v1 = [b[0] - a[0], b[1] - a[1]];
        let v0 = [p[0] - a[0], p[1] - a[1]];
        v1[1] * v0[0] - v1[0] * v0[1] > 0.0
    };
    for &p in &sorted {
        while hull.len() >= 2 && !keeps(&hull, p) {
            hull.pop();
        }
        hull.push(p);
    }
    if sorted.len() >= 2 {
        let limit = hull.len() + 1;
        for i in (0..sorted.len() - 1).rev() {
            let p = sorted[i];
            while hull.len() >= limit && !keeps(&hull, p) {
                hull.pop();
            }
            hull.push(p);
        }
    }
    hull.pop();
    hull
}

/// `SimplifyPolyline`: drops every vertex closer than `tolerance` to the
/// segment joining its neighbours (the closed ring), re-testing the vertex
/// that moves into the dropped one's place, while more than two remain.
pub fn simplify_polyline(poly: &mut Vec<[f32; 2]>, tolerance: f32) {
    if poly.len() < 3 {
        return;
    }
    let limit = tolerance * tolerance;
    let mut i = 0usize;
    let mut n = poly.len();
    loop {
        let prev = poly[if i == 0 { n } else { i } - 1];
        let next = poly[if i + 1 == n { 0 } else { i + 1 }];
        let cur = poly[i];
        let d = [next[0] - prev[0], next[1] - prev[1]];
        let length2 = d[0] * d[0] + d[1] * d[1];
        let v = [cur[0] - prev[0], cur[1] - prev[1]];
        let distance2 = if length2 != 0.0 {
            let t = (v[0] * d[0] + v[1] * d[1]) / length2;
            let t = if t < 0.0 { 0.0 } else { engine_fmin(t, 1.0) };
            let off = [d[0] * t - v[0], d[1] * t - v[1]];
            off[0] * off[0] + off[1] * off[1]
        } else {
            v[0] * v[0] + v[1] * v[1]
        };
        if distance2 < limit {
            poly.remove(i);
            n -= 1;
        } else {
            i += 1;
        }
        if i >= n || n <= 2 {
            break;
        }
    }
}

/// ARM `fmin`: a NaN operand gives NaN.
fn engine_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

fn normalize2(v: [f32; 2]) -> [f32; 2] {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if length > EPSILON {
        [v[0] / length, v[1] / length]
    } else {
        [0.0, 0.0]
    }
}

/// `CalculateCarveHullFromPoints` for points relative to the tile position
/// `pos`, with the agent's height and radius: the (x, z) outline is the
/// points' convex hull, simplified at a tenth of the radius and pushed out by
/// the radius (a corner turning by more than about 90° is bevelled with two
/// points, others are mitred); every outline edge gives a vertical plane.
/// The shape's up axis (its axes weighted by their cross-section size, or y
/// when that vanishes) bounds the hull below at the lowest point less the
/// agent height and above at the highest point. A shape tilted by more than
/// 10° also gets its bounds' bottom (less the agent height) and top. `None`
/// when the outline has fewer than three points.
pub fn carve_hull(
    points: &[[f32; 3]],
    shape: &CarveShape,
    pos: [f32; 3],
    agent_height: f32,
    agent_radius: f32,
) -> Option<CarveHull> {
    let flat: Vec<[f32; 2]> = points.iter().map(|p| [p[0], p[2]]).collect();
    let mut hull = convex_hull(&flat);
    simplify_polyline(&mut hull, agent_radius * 0.1);
    let n = hull.len();
    let r = agent_radius;
    let mut outline: Vec<[f32; 2]> = Vec::with_capacity(n + 1);
    for i in 0..n {
        let cur = hull[i];
        let prev = hull[if i == 0 { n } else { i } - 1];
        let next = hull[if i == n - 1 { 0 } else { i + 1 }];
        let d0 = normalize2([cur[0] - prev[0], cur[1] - prev[1]]);
        let d1 = normalize2([next[0] - cur[0], next[1] - cur[1]]);
        let dot = d0[0] * d1[0] + d0[1] * d1[1];
        if dot < SHARP_CORNER {
            let weight = dot.abs() * 0.75 + 0.25;
            let cx = cur[0] + d0[0] * weight * r;
            let cz = cur[1] + d0[1] * weight * r;
            outline.push([cx - d0[1] * r, d0[0] * r + cz]);
            outline.push([cx - d1[1] * r, d1[0] * r + cz]);
        } else {
            let mut nx = (d0[0] + d1[0]) * 0.5;
            let mut nz = (-d1[1] - d0[1]) * 0.5;
            let length2 = nx * nx + nz * nz;
            if length2 > 0.0 {
                let inverse = 1.0 / length2;
                nz *= inverse;
                nx *= inverse;
            }
            outline.push([cur[0] + nz * r, cur[1] + nx * r]);
        }
    }
    if outline.len() < 3 {
        return None;
    }
    let mut min: [f32; 3] = std::array::from_fn(|k| shape.bounds_min[k] - pos[k]);
    let mut max: [f32; 3] = std::array::from_fn(|k| shape.bounds_max[k] - pos[k]);
    let mut planes = Vec::with_capacity(outline.len() + 4);
    for j in 0..outline.len() {
        let a = outline[j];
        let b = outline[if j + 1 == outline.len() { 0 } else { j + 1 }];
        let dz = b[1] - a[1];
        let dx = b[0] - a[0];
        let length = (dx * dx + (dz * dz + 0.0)).sqrt();
        let normal = if length > EPSILON {
            [-dz / length, 0.0 / length, dx / length]
        } else {
            [0.0, 0.0, 0.0]
        };
        let distance = -(a[0] * normal[0] + normal[1] * 0.0 + a[1] * normal[2]);
        planes.push(Plane { normal, distance });
        if a[0] < min[0] {
            min[0] = a[0];
        }
        if a[1] < min[2] {
            min[2] = a[1];
        }
        if max[0] < a[0] {
            max[0] = a[0];
        }
        if max[2] < a[1] {
            max[2] = a[1];
        }
    }
    let up = up_axis(shape);
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for p in points {
        let along = up[0] * p[0] + up[1] * p[1] + up[2] * p[2];
        if along < low {
            low = along;
        }
        if high < along {
            high = along;
        }
    }
    let h = low - agent_height;
    let bottom = (h * up[1]) * -up[1] - up[0] * (up[0] * h) - (h * up[2]) * up[2];
    planes.push(Plane {
        normal: [-up[0], -up[1], -up[2]],
        distance: -bottom,
    });
    let top = up[2] * (up[2] * high) + (up[0] * (up[0] * high) + up[1] * (up[1] * high));
    planes.push(Plane {
        normal: up,
        distance: -top,
    });
    let vertical = up[2] * 0.0 + (up[1] + up[0] * 0.0);
    if vertical.abs() > COS_10 {
        let dx = max[0] - min[0];
        let dz = max[2] - min[2];
        let reach = ((dx * dx + 0.0) + dz * dz).sqrt() * 0.5 * TAN_10;
        max[1] += reach;
        min[1] = (min[1] - reach) - agent_height;
    } else {
        let low = std::array::from_fn::<f32, 3, _>(|k| shape.bounds_min[k] - pos[k]);
        let high = std::array::from_fn::<f32, 3, _>(|k| shape.bounds_max[k] - pos[k]);
        // Vector3f::yAxis scaled by the agent height.
        let q = [
            low[0] - 0.0 * agent_height,
            low[1] - 1.0 * agent_height,
            low[2] - 0.0 * agent_height,
        ];
        planes.push(Plane {
            normal: [-0.0, -1.0, -0.0],
            distance: -((q[0] * -0.0 - q[1]) + q[2] * -0.0),
        });
        planes.push(Plane {
            normal: [0.0, 1.0, 0.0],
            distance: -((high[0] * 0.0 + high[1]) + high[2] * 0.0),
        });
        min[1] -= agent_height;
    }
    Some(CarveHull {
        planes,
        bounds_min: min,
        bounds_max: max,
    })
}

/// The shape's up axis: `Σ axis_k · axis_k.y · m_k` with `m_x = max(e.y, e.z)`,
/// `m_y = max(e.x, e.z)`, `m_z = max(e.x, e.y)`, normalised; y when it vanishes.
fn up_axis(shape: &CarveShape) -> [f32; 3] {
    let e = shape.extents;
    let [x, y, z] = shape.axes;
    let mx = if e[1] < e[2] { e[2] } else { e[1] };
    let my = if e[2] < e[0] { e[0] } else { e[2] };
    let mz = if e[0] < e[1] { e[1] } else { e[0] };
    let wx = ((x[0] * x[1]) * mx + 0.0) + (y[0] * y[1]) * my + (z[0] * z[1]) * mz;
    let wy = ((x[1] * x[1]) * mx + 0.0) + (y[1] * y[1]) * my + (z[1] * z[1]) * mz;
    let wz = ((x[2] * x[1]) * mx + 0.0) + (y[2] * y[1]) * my + (z[2] * z[1]) * mz;
    let length = (wz * wz + (wx * wx + wy * wy)).sqrt();
    let u = if length > EPSILON {
        [wx / length, wy / length, wz / length]
    } else {
        [0.0, 0.0, 0.0]
    };
    let d = [0.0 - u[0], 0.0 - u[1], 0.0 - u[2]];
    if (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2] <= EPSILON * EPSILON {
        [0.0, 1.0, 0.0]
    } else {
        u
    }
}

impl CarveHull {
    /// The same hull in the moly frame (x reflected, tile position added
    /// back as `pos`).
    pub fn to_moly(&self, pos: [f32; 3]) -> CarveHull {
        let planes = self
            .planes
            .iter()
            .map(|p| {
                let n = [-p.normal[0], p.normal[1], p.normal[2]];
                // n·(q − pos) + d with q the moly point reflected back.
                let shift = -(p.normal[0] * pos[0] + p.normal[1] * pos[1] + p.normal[2] * pos[2]);
                Plane {
                    normal: n,
                    distance: p.distance + shift,
                }
            })
            .collect();
        CarveHull {
            planes,
            bounds_min: [
                -(self.bounds_max[0] + pos[0]),
                self.bounds_min[1] + pos[1],
                self.bounds_min[2] + pos[2],
            ],
            bounds_max: [
                -(self.bounds_min[0] + pos[0]),
                self.bounds_max[1] + pos[1],
                self.bounds_max[2] + pos[2],
            ],
        }
    }
}

/// `DynamicMesh::SplitPoly` against one plane with the tile's weld tolerance
/// `eps` (the voxel size / 64): distances within `eps` of the plane count as
/// on it. A crossing edge from `prev` to `cur` gives
/// `prev·t + cur·(1−t)` with `t = −d_cur/(d_prev − d_cur)` when `prev` is in
/// front, `prev·(1−t) + cur·t` with `t = −d_prev/(d_cur − d_prev)` when it is
/// behind: the same bits from either side of a shared edge.
pub enum Split {
    /// Every vertex is behind or on the plane.
    Inside,
    /// Every vertex is in front of it.
    Outside,
    /// The part behind (`inside`) and the part in front (`outside`).
    Both {
        inside: Vec<[f32; 3]>,
        outside: Vec<[f32; 3]>,
    },
}

/// The plane distance with the weld snap.
pub fn plane_distance(plane: &Plane, p: [f32; 3], eps: f32) -> f32 {
    let n = plane.normal;
    let d = plane.distance + (n[0] * p[0] + n[1] * p[1] + n[2] * p[2]);
    if d.abs() < eps {
        0.0
    } else {
        d
    }
}

/// The point where the edge `prev`→`cur` crosses the plane, when its ends
/// (at plane distances `dp`, `dc`) lie strictly on opposite sides; see
/// [`Split`] for the two forms.
pub(crate) fn crossing(prev: [f32; 3], dp: f32, cur: [f32; 3], dc: f32) -> Option<[f32; 3]> {
    if dp > 0.0 && dc < 0.0 {
        let t = -dc / (dp - dc);
        let s = 1.0 - t;
        Some([
            prev[0] * t + cur[0] * s,
            prev[1] * t + cur[1] * s,
            prev[2] * t + s * cur[2],
        ])
    } else if dp < 0.0 && dc > 0.0 {
        let t = -dp / (dc - dp);
        let s = 1.0 - t;
        Some([
            prev[0] * s + cur[0] * t,
            prev[1] * s + cur[1] * t,
            prev[2] * s + t * cur[2],
        ])
    } else {
        None
    }
}

/// See [`Split`]. The front part is built with the same crossing points (it
/// is what `DynamicMesh` keeps of the polygon outside that plane).
pub fn split_poly(poly: &[[f32; 3]], plane: &Plane, eps: f32) -> Split {
    let d: Vec<f32> = poly
        .iter()
        .map(|p| plane_distance(plane, *p, eps))
        .collect();
    let high = d
        .iter()
        .copied()
        .fold(d[0], |m, v| if m < v { v } else { m });
    let low = d
        .iter()
        .copied()
        .fold(d[0], |m, v| if v < m { v } else { m });
    if !(high > 0.0) || poly.len() < 2 {
        return Split::Inside;
    }
    if low > 0.0 {
        return Split::Outside;
    }
    let mut inside = Vec::with_capacity(poly.len() + 2);
    let mut outside = Vec::with_capacity(poly.len() + 2);
    let n = poly.len();
    let mut prev = poly[n - 1];
    let mut dp = d[n - 1];
    for i in 0..n {
        let cur = poly[i];
        let dc = d[i];
        if let Some(c) = crossing(prev, dp, cur, dc) {
            inside.push(c);
            outside.push(c);
        }
        if dc <= 0.0 {
            inside.push(cur);
        }
        if dc >= 0.0 {
            outside.push(cur);
        }
        prev = cur;
        dp = dc;
    }
    Split::Both { inside, outside }
}

/// `NavMeshObstacle::UpdateState` for one obstacle, with the snapshot it
/// keeps of its transform: a fresh obstacle (or one just re-enabled) takes a
/// snapshot on its first update and starts stationary, so a carving obstacle
/// carves on its first navigation update. With `carve_only_stationary`, a
/// move beyond the move threshold makes it moving (uncarved) until it has
/// stayed within a tenth of that threshold for longer than the time to
/// stationary. Without it the obstacle is never moving.
#[derive(Clone, Debug, PartialEq)]
pub struct ObstacleState {
    snapshot: Option<Snapshot>,
    moving: bool,
    timer: f32,
    /// Bumped whenever the carve shape must be rebuilt.
    pub version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Snapshot {
    position: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
    extents2: f32,
}

/// The transform an obstacle reads each update, in the engine's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObstaclePose {
    pub position: [f32; 3],
    /// Unity quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub lossy_scale: [f32; 3],
    /// `GetWorldExtents` of the shape.
    pub world_extents: [f32; 3],
}

impl Default for ObstacleState {
    fn default() -> Self {
        ObstacleState {
            snapshot: None,
            moving: false,
            timer: 0.0,
            version: 0,
        }
    }
}

impl ObstacleState {
    /// One update with the frame's delta time; returns whether the obstacle
    /// carves after it (carving on and not moving).
    pub fn update(
        &mut self,
        pose: &ObstaclePose,
        dt: f32,
        carve_only_stationary: bool,
        move_threshold: f32,
        time_to_stationary: f32,
    ) -> bool {
        // A fresh obstacle snapshots first; the snapshot equals the pose,
        // so nothing has moved in the same update.
        let fresh = self.snapshot.is_none();
        if fresh {
            self.version += 1;
            self.snapshot = Some(Self::snap(pose));
        }
        let snapshot = self.snapshot.expect("snapshot taken");
        let moved = |threshold: f32| !fresh && has_moved(&snapshot, pose, threshold);
        let threshold = engine_fmaxnm(move_threshold, 1e-5);
        if !carve_only_stationary {
            if moved(threshold) {
                self.version += 1;
                self.snapshot = Some(Self::snap(pose));
            }
            self.moving = false;
            self.timer = 0.0;
        } else if !self.moving {
            if moved(threshold) {
                self.moving = true;
                self.timer = 0.0;
                self.version += 1;
                self.snapshot = Some(Self::snap(pose));
            }
        } else if moved(engine_fmaxnm(move_threshold * 0.1, 1e-5)) {
            self.timer = 0.0;
            self.snapshot = Some(Self::snap(pose));
        } else {
            self.timer += dt;
            if self.timer > time_to_stationary {
                self.moving = false;
                self.version += 1;
            }
        }
        !self.moving
    }

    fn snap(pose: &ObstaclePose) -> Snapshot {
        let e = pose.world_extents;
        Snapshot {
            position: pose.position,
            rotation: pose.rotation,
            scale: pose.lossy_scale,
            extents2: e[0] * e[0] + e[1] * e[1] + e[2] * e[2],
        }
    }
}

/// `NavMeshObstacle::HasMoved`: the position moved farther than the
/// threshold, the rotation's angular distance times the snapshot's extent
/// exceeds it, or the scale change times the extent does.
fn has_moved(s: &Snapshot, pose: &ObstaclePose, threshold: f32) -> bool {
    let t2 = threshold * threshold;
    let p = pose.position;
    let dp = (s.position[0] - p[0]) * (s.position[0] - p[0])
        + (s.position[1] - p[1]) * (s.position[1] - p[1])
        + (s.position[2] - p[2]) * (s.position[2] - p[2]);
    if dp > t2 {
        return true;
    }
    let q = pose.rotation;
    let r = s.rotation;
    let dot = (((r[0] * q[0] + r[1] * q[1]) + r[2] * q[2]) + r[3] * q[3]).abs();
    // AngularDistance: acos(fminnm(|dot|, 1)) doubled.
    let clamped = if dot < 1.0 { dot } else { 1.0 };
    let angle = clamped.acos() * 2.0;
    let sc = s.scale;
    let extent2 = sc[0] * sc[0] + sc[1] * sc[1] + sc[2] * sc[2];
    if angle * angle * (s.extents2 * extent2) > t2 {
        return true;
    }
    let ds = (sc[0] - pose.lossy_scale[0]) * (sc[0] - pose.lossy_scale[0])
        + (sc[1] - pose.lossy_scale[1]) * (sc[1] - pose.lossy_scale[1])
        + (sc[2] - pose.lossy_scale[2]) * (sc[2] - pose.lossy_scale[2]);
    s.extents2 * ds > t2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Research instrument: `CalculateCarveHullFromPoints` executed in an
    /// ARMv8 emulator on the current engine library (with its own static
    /// initializer run first for the `.bss` axis constants), over generated
    /// carve inputs: upright and tilted capsules and boxes at random yaw and
    /// position, harvest obstacle sizes, point clouds, tiny shapes and
    /// degenerate point sets, each at the origin or an offset tile position
    /// and with several agent heights and radii. Point MOLY_CARVE_HULL_ROWS
    /// at the recorded word rows; every row's success flag, plane words and
    /// bound words must match.
    #[test]
    #[ignore = "needs MOLY_CARVE_HULL_ROWS"]
    fn carve_hull_matches_native_rows() {
        let path = std::env::var("MOLY_CARVE_HULL_ROWS").expect("MOLY_CARVE_HULL_ROWS");
        let text = std::fs::read_to_string(&path).expect("read rows");
        let (mut rows, mut failures) = (0usize, Vec::new());
        let mut families: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        let mut sha = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("S ") {
                sha = Some(rest.to_owned());
                continue;
            }
            let Some(rest) = line.strip_prefix("R ") else {
                continue;
            };
            let (input, output) = rest.split_once(" | ").expect("row halves");
            let f: Vec<&str> = input.split_whitespace().collect();
            let o: Vec<u32> = output
                .split_whitespace()
                .map(|w| w.parse().unwrap())
                .collect();
            let family = f[0].to_owned();
            let word = |i: usize| f32::from_bits(f[i].parse::<u32>().unwrap());
            let v3 = |i: usize| [word(i), word(i + 1), word(i + 2)];
            let kind = if f[1] == "1" {
                CarveKind::Box
            } else {
                CarveKind::Capsule
            };
            let shape = CarveShape {
                kind,
                center: v3(2),
                extents: v3(5),
                axes: [v3(8), v3(11), v3(14)],
                bounds_min: v3(17),
                bounds_max: v3(20),
            };
            let pos = v3(23);
            let (height, radius) = (word(26), word(27));
            let count: usize = f[28].parse().unwrap();
            let points: Vec<[f32; 3]> = (0..count).map(|k| v3(29 + 3 * k)).collect();
            let ours = carve_hull(&points, &shape, pos, height, radius);
            let (ok, planes) = (o[0], o[1] as usize);
            let expected_planes = &o[2..2 + 4 * planes];
            let expected_bounds = &o[2 + 4 * planes..];
            let matched = match &ours {
                None => ok == 0 && planes == 0,
                Some(hull) => {
                    let words: Vec<u32> = hull
                        .planes
                        .iter()
                        .flat_map(|p| [p.normal[0], p.normal[1], p.normal[2], p.distance])
                        .map(f32::to_bits)
                        .collect();
                    let bounds: Vec<u32> = hull
                        .bounds_min
                        .iter()
                        .chain(&hull.bounds_max)
                        .map(|v| v.to_bits())
                        .collect();
                    ok == 1 && words == expected_planes && bounds == expected_bounds
                }
            };
            rows += 1;
            let entry = families.entry(family.clone()).or_default();
            entry.0 += 1;
            if !matched {
                entry.1 += 1;
                if failures.len() < 8 {
                    failures.push(format!(
                        "{family} row {rows}: ours {ours:?} native ok {ok} planes {planes}"
                    ));
                }
            }
        }
        eprintln!(
            "carve hull rows {rows} (library {sha:?}): per family (rows, mismatched) {families:?}"
        );
        for failure in &failures {
            eprintln!("{failure}");
        }
        assert!(rows > 0 && families.contains_key("tilted-box"));
        assert!(failures.is_empty(), "mismatched rows");
    }
}
