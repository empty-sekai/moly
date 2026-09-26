//! Cooking of a convex mesh collider as the engine runs it: the vertex weld
//! of the mesh extraction, then the quickhull path with the collider cooking
//! parameters (hull computed from the points, mesh validation on, no
//! plane shifting, no quantisation, no GPU data): the vertex cleanup, the
//! simplex fixup, the hull build and its polygon limit, the post merge of
//! near-coplanar faces, the hull descriptor, the polygon data with its edge
//! list and vertex-to-polygon map, the mass properties (binary64 volume
//! integrals about the vertex mean), the local bounds, the support-vertex
//! map of hulls past 32 vertices, and the internal radius and extents.
//!
//! Every step is one binary32 operation (binary64 in the volume integrals)
//! in the engine's order; nothing fuses. The engine's float `PxMax` is
//! FMAXNM, the cleanup bounds compare and select, and the local bounds take
//! FMIN/FMAX lanes starting from the last vertex.
//!
//! The collider's cooking options do not reach this path: the convex
//! descriptor's flags and limits are constants, and the extraction welds
//! whatever the options say. Where the engine's own checks cook nothing
//! (fewer than three distinct vertices, 256 or more hull vertices) the
//! refusal's text starts with `no convex mesh:` (the collider has no
//! shape); `UNSET_SIMPLEX` is an input whose engine result is not defined;
//! every other refusal names a path the engine's rows never reached.
use super::cook::weld;
use super::vector::*;
use super::{arms, GaussMap, HullSupport, Refused};
use crate::particle::armf as a;

/// The tolerance scale's length the physics object was created with (the
/// engine default).
const LENGTH: f32 = 1.0;
/// Cleanup: an axis thinner than this makes the input a box, 1e-6 · length.
const DISTANCE_EPSILON: f32 = f32::from_bits(0x3586_37bd);
/// Cleanup: the half-size of the box made from a point-like input, 0.01 · length.
const RESIZE_VALUE: f32 = f32::from_bits(0x3c23_d70a);
/// Cleanup: two normalised points closer than this on every axis are one, 1e-4.
const NORMAL_EPSILON: f32 = f32::from_bits(0x38d1_b717);
/// Cleanup: a thin axis becomes this fraction of the shortest other one, 0.05.
const THIN_FRACTION: f32 = f32::from_bits(0x3d4c_cccd);
/// Cleanup: the empty bounds, a quarter of FLT_MAX.
const BOUNDS_EXTENT: f32 = f32::from_bits(0x7e7f_ffff);
/// Points within this of a plane are on it: 3 · FLT_EPSILON.
const PLANE_THICKNESS: f32 = f32::from_bits(0x34c0_0000);
/// The cooking parameters' plane tolerance, 0.0007.
const PLANE_TOLERANCE: f32 = f32::from_bits(0x3a37_8034);
/// Post merge: two faces within 3 degrees merge, cos 3°.
const MAXDOT_MINANG: f32 = f32::from_bits(0x3f7f_a62f);
/// The descriptor's vertex limit and the builder's polygon limit.
const VERTEX_LIMIT: u32 = 255;
const POLYGON_LIMIT: u32 = 255;
/// Hulls with more vertices than this carry the support-vertex map.
const GAUSS_MAP_LIMIT: usize = 32;
/// The support-vertex map's cube-face subdivision.
const SUBDIV: u32 = 16;
/// The polygon check's plane tolerance, 0.02.
const CHECK_TOLERANCE: f32 = f32::from_bits(0x3ca3_d70a);
/// Internal extents: a plane normal component this small is parallel, 1e-7.
const INTERNAL_EPSILON: f32 = f32::from_bits(0x33d6_bf95);
const SQRT_3: f32 = f32::from_bits(0x3fdd_b3d7);

const NONE: usize = usize::MAX;

/// Every point exactly on the extreme pair's line: the engine's simplex
/// fixup then moves a vertex by its own unset stack slots, so what it cooks
/// is not defined by the input (the rows' engine cooked nothing).
pub const UNSET_SIMPLEX: Refused = Refused("exactly collinear points: the engine's simplex fixup reads an unset vertex");

/// One hull polygon: its plane (normal and `d`, with `n·p + d = 0` on it),
/// the offset of its corner list in `vertex_data`, its corner count, and the
/// hull vertex whose projection on the normal is least.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullPolygon {
    pub plane: [f32; 4],
    pub vref8: u16,
    pub nb_verts: u8,
    pub min_index: u8,
}

/// The support-vertex map of a hull past 32 vertices: per cube-face sample
/// the hill-climbed minimum and maximum vertex, and the vertex adjacency the
/// climb walks.
#[derive(Clone, Debug, PartialEq)]
pub struct BigConvex {
    pub subdiv: u16,
    pub nb_samples: u16,
    /// `2 * nb_samples` bytes: the minimum vertices, then the maxima.
    pub samples: Vec<u8>,
    /// Per hull vertex its neighbour count and offset into `adjacent_verts`.
    pub valencies: Vec<(u16, u16)>,
    pub nb_adj_verts: u32,
    pub adjacent_verts: Vec<u8>,
}

/// The engine's cooked convex mesh: the hull data as the mesh holds it, with
/// the mesh's mass and inertia.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvexHull {
    pub aabb_center: [f32; 3],
    pub aabb_extents: [f32; 3],
    pub center_of_mass: [f32; 3],
    pub nb_edges: u16,
    pub vertices: Vec<[f32; 3]>,
    pub polygons: Vec<HullPolygon>,
    /// Two polygon indices per edge.
    pub faces_by_edges: Vec<u8>,
    /// Three polygon indices per hull vertex.
    pub faces_by_vertices: Vec<u8>,
    /// The polygons' corner lists, concatenated.
    pub vertex_data: Vec<u8>,
    pub big: Option<BigConvex>,
    pub internal_radius: f32,
    pub internal_extents: [f32; 3],
    pub mass: f32,
    /// Column-major.
    pub inertia: [f32; 9],
    /// The build stopped at the polygon limit; the engine warns and keeps
    /// the partial hull.
    pub polygon_limit_reached: bool,
}

/// Whether a refusal is the engine's own failed cook (no shape) rather than
/// a path this port does not carry.
pub fn engine_cooks_nothing(r: &Refused) -> bool {
    r.0.starts_with("no convex mesh:")
}

/// A MeshCollider's convex cook: `positions` and `indices` as the mesh holds
/// them (source axes, source index order), `options` its cooking options.
/// Returns what the sweep reads.
pub fn cook_convex(positions: &[[f32; 3]], indices: &[u32], options: u32) -> Result<HullSupport, Refused> {
    if options & !0x1f != 0 {
        return Err(Refused("cooking options outside the collider's option bits"));
    }
    if indices.iter().any(|&i| i as usize >= positions.len()) {
        return Err(Refused("a triangle corner past the vertex count"));
    }
    Ok(from_cooked(&cook_convex_hull(positions)?))
}

/// What the sweep reads of a cooked hull.
pub(super) fn from_cooked(hull: &ConvexHull) -> HullSupport {
    HullSupport {
        vertices: hull.vertices.clone(),
        internal_extents: hull.internal_extents,
        gauss_map: hull.big.as_ref().map(|b| GaussMap {
            subdiv: b.subdiv,
            samples: b.samples.clone(),
            valencies: b.valencies.clone(),
            adjacent: b.adjacent_verts.clone(),
        }),
    }
}

/// The whole cooked mesh from the mesh's positions.
pub fn cook_convex_hull(positions: &[[f32; 3]]) -> Result<ConvexHull, Refused> {
    if positions.iter().any(|p| !p.iter().all(|v| v.is_finite())) {
        return Err(Refused("a non-finite convex collider vertex"));
    }
    let welded = if arms::on("convexNoWeld") { positions.to_vec() } else { weld(positions, &[]).0 };
    if welded.len() <= 2 {
        return Err(Refused("no convex mesh: fewer than three distinct vertices (the engine reports it and cooks nothing)"));
    }
    let mut points = cleanup_vertices(&welded)?;
    cleanup_for_simplex(&mut points)?;
    let mut hull = QuickHull::new(&points);
    let polygon_limit_reached = match hull.build()? {
        Built::Success => {
            hull.post_merge()?;
            false
        }
        Built::PolygonLimit => {
            if hull.output_vertices > VERTEX_LIMIT {
                return Err(Refused("a polygon-limited hull past 255 vertices (the engine's OBB expansion is not reached by the rows)"));
            }
            true
        }
        Built::VertexLimit => {
            return Err(Refused("a hull past 255 vertices (the engine's OBB expansion is not reached by the rows)"));
        }
    };
    let desc = hull.fill_desc()?;
    if desc.vertices.len() >= 256 {
        return Err(Refused("no convex mesh: a hull of 256 or more vertices (the engine reports it and cooks nothing)"));
    }
    if desc.polygons.len() < 4 {
        return Err(Refused("a hull of fewer than four polygons (the engine's descriptor check is not reached by the rows)"));
    }
    let (faces_by_edges, edge_data) = hull.edge_list(&desc)?;
    build_mesh(desc, faces_by_edges, edge_data, polygon_limit_reached)
}

// ---- vertex cleanup ------------------------------------------------------

/// `checkPointsAABBValidity`: the input's bounds; a thin or point-like input
/// (or fewer than three points) becomes the eight corners of a box about
/// its centre, written from `out`'s start when `restart`, else appended.
/// Returns whether it did; otherwise `scale` takes the bounds' size.
fn aabb_validity(points: &[V3], center: &mut V3, scale: &mut V3, out: &mut Vec<V3>, restart: bool) -> bool {
    let (mut mn, mut mx) = ([BOUNDS_EXTENT; 3], [-BOUNDS_EXTENT; 3]);
    for p in points {
        for k in 0..3 {
            mn[k] = if mn[k] < p[k] { mn[k] } else { p[k] };
            mx[k] = if mx[k] > p[k] { mx[k] } else { p[k] };
        }
    }
    let dim = sub(mx, mn);
    *center = std::array::from_fn(|k| a::mul(a::add(mn[k], mx[k]), 0.5));
    let eps = a::mul(LENGTH, DISTANCE_EPSILON);
    if !(dim[0] < eps || dim[1] < eps || points.len() < 3 || dim[2] < eps) {
        *scale = dim;
        return false;
    }
    let mut len = FLT_MAX;
    for d in dim {
        if d < len && d > eps {
            len = d;
        }
    }
    let half = if len == FLT_MAX {
        [a::mul(LENGTH, RESIZE_VALUE); 3]
    } else {
        dim.map(|d| if d < eps { a::mul(len, THIN_FRACTION) } else { a::mul(d, 0.5) })
    };
    let (p, n) = (add(*center, half), sub(*center, half));
    if restart {
        out.clear();
    }
    out.extend_from_slice(&[
        n,
        [p[0], n[1], n[2]],
        [p[0], p[1], n[2]],
        [n[0], p[1], n[2]],
        [n[0], n[1], p[2]],
        [p[0], n[1], p[2]],
        p,
        [n[0], p[1], p[2]],
    ]);
    true
}

/// `ConvexHullLib::cleanupVertices`: the box for a thin input; otherwise
/// the points normalised by the bounds' size, points within the normal
/// epsilon of a kept one merged into it (the one farther from the centre
/// stays), scaled back, and the box check again on the result.
fn cleanup_vertices(input: &[V3]) -> Result<Vec<V3>, Refused> {
    let mut center = [0.0; 3];
    let mut scale = [1.0; 3];
    let mut out = Vec::with_capacity(input.len().max(8));
    if aabb_validity(input, &mut center, &mut scale, &mut out, false) {
        return Ok(out);
    }
    let recip = scale.map(|s| a::div(1.0, s));
    let center = std::array::from_fn::<f32, 3, _>(|k| a::mul(recip[k], center[k]));
    for p in input {
        let np: V3 = std::array::from_fn(|k| a::mul(recip[k], p[k]));
        let mut found = false;
        for v in out.iter_mut() {
            let d: V3 = std::array::from_fn(|k| a::abs(a::sub(np[k], v[k])));
            if d[0] < NORMAL_EPSILON && d[1] < NORMAL_EPSILON && d[2] < NORMAL_EPSILON {
                if mag2(sub(np, center)) > mag2(sub(*v, center)) {
                    *v = np;
                }
                found = true;
                break;
            }
        }
        if !found {
            out.push(np);
        }
    }
    if out.len() < 4 {
        return Err(Refused("fewer than four distinct cleaned vertices (the engine's failed cook here is not reached by the rows)"));
    }
    for v in out.iter_mut() {
        *v = std::array::from_fn(|k| a::mul(v[k], scale[k]));
    }
    let mut center = [0.0; 3];
    let kept = out.clone();
    aabb_validity(&kept, &mut center, &mut scale, &mut out, true);
    Ok(out)
}

/// Half the bounds' size summed over the axes, in the engine's order.
fn half_size(mn: V3, mx: V3) -> f32 {
    a::mul(a::sub(a::add(a::sub(a::add(a::sub(mx[0], mn[0]), mx[1]), mn[1]), mx[2]), mn[2]), 0.5)
}

/// Per axis the first vertex with the greatest and with the least
/// coordinate (a vertex that raises the maximum is not tested against the
/// minimum), and the bounds.
fn extreme_vertices(points: &[V3]) -> ([usize; 3], [usize; 3], V3, V3) {
    let (mut imin, mut imax) = ([0usize; 3], [0usize; 3]);
    let (mut mn, mut mx) = (points[0], points[0]);
    for (i, p) in points.iter().enumerate().skip(1) {
        for k in 0..3 {
            if p[k] > mx[k] {
                mx[k] = p[k];
                imax[k] = i;
            } else if p[k] < mn[k] {
                mn[k] = p[k];
                imin[k] = i;
            }
        }
    }
    (imin, imax, mn, mx)
}

/// `QuickHullConvexHullLib::cleanupForSimplex`: when the farthest point
/// from the extreme pair's line is within the tolerance of it, that point
/// moves off the line by the tolerance; when the farthest point from that
/// plane is within the tolerance of it, it moves off the plane.
fn cleanup_for_simplex(vertices: &mut [V3]) -> Result<(), Refused> {
    let (imin, imax, mn, mx) = extreme_vertices(vertices);
    let tolerance = a::max_nm(a::mul(PLANE_THICKNESS, half_size(mn, mx)), PLANE_THICKNESS);
    let mut fmax = 0.0f32;
    let mut axis = 0;
    for i in 0..3 {
        let diff = a::sub(vertices[imax[i]][i], vertices[imin[i]][i]);
        if diff > fmax {
            fmax = diff;
            axis = i;
        }
    }
    let s0 = vertices[imax[axis]];
    let s1 = vertices[imin[axis]];
    let (u01, _) = normalize(sub(s1, s0));
    let mut normal = [0.0f32; 3];
    let mut max_dist = 0.0f32;
    let mut i2 = NONE;
    for (i, p) in vertices.iter().enumerate() {
        let xprod = cross(u01, sub(*p, s0));
        let len = mag2(xprod);
        if len > max_dist {
            max_dist = len;
            normal = xprod;
            i2 = i;
        }
    }
    if i2 == NONE {
        return Err(UNSET_SIMPLEX);
    }
    let mut s2 = vertices[i2];
    if a::sqrt(max_dist) < tolerance {
        let u02 = sub(s2, s0);
        let t = a::div(dot(u02, u01), mag2(u01));
        let (n, _) = normalize(sub(u02, scale(u01, t)));
        s2 = add(s2, scale(n, tolerance));
        vertices[i2] = s2;
    }
    let (normal, _) = normalize(normal);
    let d0 = dot(s2, normal);
    let mut max_dist = 0.0f32;
    let mut i3 = 0;
    for (i, p) in vertices.iter().enumerate() {
        let dist = a::abs(a::sub(dot(*p, normal), d0));
        if dist > max_dist {
            max_dist = dist;
            i3 = i;
        }
    }
    if a::abs(max_dist) < tolerance {
        let p = vertices[i3];
        vertices[i3] = if a::sub(dot(p, normal), d0) > 0.0 {
            add(p, scale(normal, tolerance))
        } else {
            sub(p, scale(normal, tolerance))
        };
    }
    Ok(())
}

// ---- quickhull -----------------------------------------------------------

/// A vertex as edges and the simplex copy it: its point and input index.
#[derive(Clone, Copy)]
struct Vtx {
    point: V3,
    index: u32,
}

/// An input vertex with its conflict-list link and distance.
#[derive(Clone, Copy)]
struct ListVertex {
    point: V3,
    index: u32,
    dist: f32,
    next: usize,
}

/// A half edge; it ends at its tail (it runs from `prev`'s tail to its own).
#[derive(Clone, Copy)]
struct HalfEdge {
    tail: Vtx,
    prev: usize,
    next: usize,
    twin: usize,
    face: usize,
    edge_index: u32,
}

impl HalfEdge {
    fn new(tail: Vtx, face: usize) -> Self {
        HalfEdge { tail, prev: NONE, next: NONE, twin: NONE, face, edge_index: u32::MAX }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    Deleted,
    NonConvex,
}

#[derive(Clone, Copy)]
struct Face {
    edge: usize,
    num_edges: u16,
    conflict: usize,
    normal: V3,
    area: f32,
    centroid: V3,
    plane_offset: f32,
    state: State,
    out_index: u8,
}

impl Face {
    fn fresh() -> Self {
        Face {
            edge: NONE,
            num_edges: 0,
            conflict: NONE,
            normal: [0.0; 3],
            area: 0.0,
            centroid: [0.0; 3],
            plane_offset: 0.0,
            state: State::Visible,
            out_index: 0,
        }
    }

    fn distance(&self, p: V3) -> f32 {
        plane_distance(self.normal, self.plane_offset, p)
    }
}

/// `n·p - offset`.
fn plane_distance(n: V3, offset: f32, p: V3) -> f32 {
    if arms::on("convexFusedPlaneDistance") {
        return n[0].mul_add(p[0], n[1].mul_add(p[1], n[2] * p[2])) - offset;
    }
    a::sub(dot(n, p), offset)
}

/// A face's plane: the Newell normal (normalised; its length is the area),
/// the corner mean, the offset and the corner count.
struct Plane {
    normal: V3,
    area: f32,
    centroid: V3,
    offset: f32,
    num_edges: u16,
}

/// `QuickHullFace::computeNormalAndCentroid`: the fan from the corner that
/// starts the longest of the first three edges.
fn face_plane(edges: &[HalfEdge], edge: usize) -> Result<Plane, Refused> {
    let mut test = edge;
    let mut start = NONE;
    let mut max_dist = 0.0f32;
    for _ in 0..3 {
        let d = mag2(sub(edges[test].tail.point, edges[edges[test].next].tail.point));
        if d > max_dist {
            max_dist = d;
            start = test;
        }
        test = edges[test].next;
    }
    if start == NONE {
        return Err(Refused("a hull face whose first three corners coincide (the engine dereferences no edge)"));
    }
    let p0 = edges[start].tail.point;
    let mut he = edges[start].next;
    let d = sub(edges[he].tail.point, p0);
    let mut centroid = p0;
    let mut normal = [0.0f32; 3];
    let mut n: u16 = 1;
    loop {
        n = n.wrapping_add(1);
        if n as usize > edges.len() + 1 {
            return Err(Refused("an open face ring"));
        }
        centroid = add(centroid, edges[he].tail.point);
        normal = add(normal, cross(d, sub(edges[edges[he].next].tail.point, p0)));
        he = edges[he].next;
        if he == start {
            break;
        }
    }
    let (normal, area) = normalize(normal);
    let centroid = if arms::on("convexCentroidDivide") {
        centroid.map(|c| a::div(c, n as f32))
    } else {
        scale(centroid, a::div(1.0, n as f32))
    };
    Ok(Plane { normal, area, offset: dot(normal, centroid), centroid, num_edges: n })
}

enum Built {
    Success,
    PolygonLimit,
    VertexLimit,
}

enum Added {
    Added,
    Failed,
    PolygonLimit,
}

/// The hull descriptor: vertices in first-visit order, the polygons' corner
/// indices, per polygon its plane, index base and corner count, and per
/// output polygon the build face it came from.
struct Desc {
    vertices: Vec<V3>,
    indices: Vec<u32>,
    polygons: Vec<([f32; 4], u16, u16)>,
    face_translate: Vec<u16>,
}

struct QuickHull {
    verts: Vec<ListVertex>,
    edges: Vec<HalfEdge>,
    faces: Vec<Face>,
    hull_faces: Vec<usize>,
    num_hull_faces: u32,
    tolerance: f32,
    plane_tolerance: f32,
    output_vertices: u32,
    unclaimed: Vec<usize>,
    horizon: Vec<usize>,
    new_faces: Vec<usize>,
    removed: Vec<usize>,
}

impl QuickHull {
    fn new(points: &[V3]) -> Self {
        QuickHull {
            verts: points
                .iter()
                .enumerate()
                .map(|(i, &point)| ListVertex { point, index: i as u32, dist: 0.0, next: NONE })
                .collect(),
            edges: Vec::new(),
            faces: Vec::new(),
            hull_faces: Vec::new(),
            num_hull_faces: 0,
            tolerance: 0.0,
            plane_tolerance: 0.0,
            output_vertices: 0,
            unclaimed: Vec::new(),
            horizon: Vec::new(),
            new_faces: Vec::new(),
            removed: Vec::new(),
        }
    }

    fn vtx(&self, i: usize) -> Vtx {
        Vtx { point: self.verts[i].point, index: self.verts[i].index }
    }

    fn opposite_face(&self, he: usize) -> usize {
        self.edges[self.edges[he].twin].face
    }

    /// `getOppositeFaceDistance`: the twin face's centroid above this face.
    fn opposite_distance(&self, he: usize) -> f32 {
        let f = &self.faces[self.edges[he].face];
        f.distance(self.faces[self.opposite_face(he)].centroid)
    }

    fn edge_at(&self, face: usize, i: usize) -> usize {
        let mut he = self.faces[face].edge;
        for _ in 0..i {
            he = self.edges[he].next;
        }
        he
    }

    fn set_twin(&mut self, x: usize, y: usize) {
        self.edges[x].twin = y;
        self.edges[y].twin = x;
    }

    fn recompute(&mut self, face: usize) -> Result<(), Refused> {
        let p = face_plane(&self.edges, self.faces[face].edge)?;
        let f = &mut self.faces[face];
        f.normal = p.normal;
        f.area = p.area;
        f.centroid = p.centroid;
        f.plane_offset = p.offset;
        f.num_edges = p.num_edges;
        Ok(())
    }

    /// The simplex vertices' search (as the fixup's) with the tolerances
    /// from half the bounds' size.
    fn min_max(&mut self) -> ([Vtx; 3], [Vtx; 3]) {
        let points: Vec<V3> = self.verts.iter().map(|v| v.point).collect();
        let (imin, imax, mn, mx) = extreme_vertices(&points);
        let size = half_size(mn, mx);
        self.tolerance = a::max_nm(a::mul(PLANE_THICKNESS, size), PLANE_THICKNESS);
        self.plane_tolerance = a::max_nm(a::mul(PLANE_TOLERANCE, size), PLANE_TOLERANCE);
        (imin.map(|i| self.vtx(i)), imax.map(|i| self.vtx(i)))
    }

    fn triangle(&mut self, v0: Vtx, v1: Vtx, v2: Vtx) -> Result<usize, Refused> {
        let face = self.faces.len();
        self.faces.push(Face::fresh());
        let e = self.edges.len();
        for v in [v0, v1, v2] {
            self.edges.push(HalfEdge::new(v, face));
        }
        for k in 0..3 {
            self.edges[e + k].prev = e + (k + 2) % 3;
            self.edges[e + k].next = e + (k + 1) % 3;
        }
        self.faces[face].edge = e;
        self.recompute(face)?;
        Ok(face)
    }

    /// `findSimplex`: the extreme pair on the widest axis, the farthest
    /// vertex from their line, the farthest from that plane.
    fn find_simplex(&mut self, minv: [Vtx; 3], maxv: [Vtx; 3]) -> Result<(), Refused> {
        let mut max = 0.0f32;
        let mut axis = 0;
        for i in 0..3 {
            let diff = a::sub(maxv[i].point[i], minv[i].point[i]);
            if diff > max {
                max = diff;
                axis = i;
            }
        }
        if max <= self.tolerance {
            return Err(Refused("coincident simplex points after the cleanup (the engine's failed cook here is not reached by the rows)"));
        }
        let (s0, s1) = (maxv[axis], minv[axis]);
        let (u01, _) = normalize(sub(s1.point, s0.point));
        let mut normal = [0.0f32; 3];
        let mut max_dist = 0.0f32;
        let mut s2 = None;
        for v in &self.verts {
            let xprod = cross(u01, sub(v.point, s0.point));
            let len = mag2(xprod);
            if len > max_dist && v.index != s0.index && v.index != s1.index {
                max_dist = len;
                s2 = Some(Vtx { point: v.point, index: v.index });
                normal = xprod;
            }
        }
        let s2 = match s2 {
            Some(s2) if a::sqrt(max_dist) > self.tolerance => s2,
            _ => return Err(Refused("collinear simplex points after the fixup (the engine's failed cook here is not reached by the rows)")),
        };
        let (normal, _) = normalize(normal);
        let d0 = dot(s2.point, normal);
        let mut max_dist = 0.0f32;
        let mut s3 = None;
        for v in &self.verts {
            let dist = a::abs(a::sub(dot(v.point, normal), d0));
            if dist > max_dist && v.index != s0.index && v.index != s1.index && v.index != s2.index {
                max_dist = dist;
                s3 = Some(Vtx { point: v.point, index: v.index });
            }
        }
        let s3 = match s3 {
            Some(s3) if a::abs(max_dist) > self.tolerance => s3,
            _ => return Err(Refused("coplanar simplex points after the cleanup (the engine's failed cook here is not reached by the rows)")),
        };
        let flip = a::sub(dot(s3.point, normal), d0) < 0.0;
        self.add_simplex([s0, s1, s2, s3], flip)
    }

    /// `addSimplex`: four triangles and their twins, then every other vertex
    /// into the conflict list of the face it is farthest above.
    fn add_simplex(&mut self, s: [Vtx; 4], flip: bool) -> Result<(), Refused> {
        let tris = if flip {
            let t = [
                self.triangle(s[0], s[1], s[2])?,
                self.triangle(s[3], s[1], s[0])?,
                self.triangle(s[3], s[2], s[1])?,
                self.triangle(s[3], s[0], s[2])?,
            ];
            for i in 0..3 {
                let k = (i + 1) % 3;
                let (x, y) = (self.edge_at(t[i + 1], 1), self.edge_at(t[k + 1], 0));
                self.set_twin(x, y);
                let (x, y) = (self.edge_at(t[i + 1], 2), self.edge_at(t[0], k));
                self.set_twin(x, y);
            }
            t
        } else {
            let t = [
                self.triangle(s[0], s[2], s[1])?,
                self.triangle(s[3], s[0], s[1])?,
                self.triangle(s[3], s[1], s[2])?,
                self.triangle(s[3], s[2], s[0])?,
            ];
            for i in 0..3 {
                let k = (i + 1) % 3;
                let (x, y) = (self.edge_at(t[i + 1], 0), self.edge_at(t[k + 1], 1));
                self.set_twin(x, y);
                let (x, y) = (self.edge_at(t[i + 1], 2), self.edge_at(t[0], (3 - i) % 3));
                self.set_twin(x, y);
            }
            t
        };
        self.hull_faces.extend_from_slice(&tris);
        self.num_hull_faces = 4;
        for i in 0..self.verts.len() {
            let v = self.verts[i];
            if s.iter().any(|x| x.index == v.index) {
                continue;
            }
            let mut max_dist = self.tolerance;
            let mut max_face = NONE;
            for &t in &tris {
                let dist = self.faces[t].distance(v.point);
                if dist > max_dist {
                    max_face = t;
                    max_dist = dist;
                }
            }
            if max_face != NONE {
                self.add_point_to_face(max_face, i, max_dist);
            }
        }
        Ok(())
    }

    /// `addPointToFace`: the farthest vertex heads the list; a nearer one
    /// goes second.
    fn add_point_to_face(&mut self, face: usize, v: usize, dist: f32) {
        self.verts[v].dist = dist;
        let c = self.faces[face].conflict;
        if c == NONE {
            self.faces[face].conflict = v;
            self.verts[v].next = NONE;
        } else if self.verts[c].dist > dist {
            self.verts[v].next = self.verts[c].next;
            self.verts[c].next = v;
        } else {
            self.verts[v].next = c;
            self.faces[face].conflict = v;
        }
    }

    /// `buildHull`. A merge failure restarts the build with the failing
    /// vertex as the terminal one; the rows never reach it.
    fn build(&mut self) -> Result<Built, Refused> {
        let (minv, maxv) = self.min_max();
        self.find_simplex(minv, maxv)?;
        let mut num_verts: u32 = 4;
        loop {
            let (eye, eye_face) = self.next_point();
            if eye == NONE {
                break;
            }
            match self.add_point(eye, eye_face)? {
                Added::PolygonLimit => {
                    self.output_vertices = num_verts;
                    return Ok(Built::PolygonLimit);
                }
                Added::Failed => {
                    return Err(Refused("a face merge failure that restarts the hull at a terminal vertex (not reached by the rows)"));
                }
                Added::Added => {}
            }
            num_verts += 1;
        }
        self.output_vertices = num_verts;
        Ok(if num_verts > VERTEX_LIMIT { Built::VertexLimit } else { Built::Success })
    }

    /// `nextPointToAdd`: the head of the visible face lists farthest above
    /// the plane tolerance.
    fn next_point(&self) -> (usize, usize) {
        let (mut eye, mut eye_face) = (NONE, NONE);
        let mut max = self.plane_tolerance;
        for &f in &self.hull_faces {
            let face = &self.faces[f];
            if face.state == State::Visible && face.conflict != NONE {
                let dist = self.verts[face.conflict].dist;
                if max < dist {
                    max = dist;
                    eye = face.conflict;
                    eye_face = f;
                }
            }
        }
        (eye, eye_face)
    }

    /// `addPointToHull`.
    fn add_point(&mut self, eye: usize, eye_face: usize) -> Result<Added, Refused> {
        self.faces[eye_face].conflict = self.verts[eye].next;
        let eye_point = self.verts[eye].point;
        self.horizon_from(eye_point, NONE, eye_face, 0)?;
        if self.num_hull_faces + self.horizon.len() as u32 > POLYGON_LIMIT {
            for i in 0..self.removed.len() {
                let f = self.removed[i];
                self.faces[f].state = State::Visible;
            }
            self.num_hull_faces += self.removed.len() as u32;
            return Ok(Added::PolygonLimit);
        }
        let eye_vtx = self.vtx(eye);
        self.new_faces_from_horizon(eye_vtx)?;
        let mut failed = false;
        for i in 0..self.new_faces.len() {
            let f = self.new_faces[i];
            if self.faces[f].state == State::Visible {
                while self.adjacent_merge(f, true, &mut failed)? {}
            }
        }
        if failed {
            return Ok(Added::Failed);
        }
        for i in 0..self.new_faces.len() {
            let f = self.new_faces[i];
            if self.faces[f].state == State::NonConvex {
                self.faces[f].state = State::Visible;
                while self.adjacent_merge(f, false, &mut failed)? {}
            }
        }
        if failed {
            return Ok(Added::Failed);
        }
        self.resolve_unclaimed();
        self.horizon.clear();
        self.new_faces.clear();
        self.removed.clear();
        Ok(Added::Added)
    }

    /// `calculateHorizon`: every face the eye sees is removed; the edges to
    /// the faces it does not see form the horizon, in walk order.
    fn horizon_from(&mut self, eye: V3, edge0: usize, face: usize, depth: usize) -> Result<(), Refused> {
        if depth > self.faces.len() {
            return Err(Refused("a horizon walk deeper than the face count"));
        }
        self.delete_face_points(face, NONE);
        self.faces[face].state = State::Deleted;
        self.removed.push(face);
        self.num_hull_faces = self.num_hull_faces.wrapping_sub(1);
        let (edge0, mut edge) = if edge0 == NONE {
            let e = self.faces[face].edge;
            (e, e)
        } else {
            (edge0, self.edges[edge0].next)
        };
        loop {
            let opp = self.opposite_face(edge);
            if self.faces[opp].state == State::Visible {
                if self.faces[opp].distance(eye) > self.tolerance {
                    let twin = self.edges[edge].twin;
                    self.horizon_from(eye, twin, opp, depth + 1)?;
                } else {
                    self.horizon.push(edge);
                }
            }
            edge = self.edges[edge].next;
            if edge == edge0 {
                break;
            }
        }
        Ok(())
    }

    /// `deleteFacePoints`: the face's list moves to the absorbing face where
    /// a vertex is above it by more than the tolerance, else to the
    /// unclaimed list.
    fn delete_face_points(&mut self, face: usize, absorbing: usize) {
        let mut u = self.faces[face].conflict;
        if u == NONE {
            return;
        }
        while u != NONE {
            let v = u;
            u = self.verts[u].next;
            self.verts[v].next = NONE;
            if absorbing == NONE {
                self.unclaimed.push(v);
            } else {
                let dist = self.faces[absorbing].distance(self.verts[v].point);
                if dist > self.tolerance {
                    self.add_point_to_face(absorbing, v, dist);
                } else {
                    self.unclaimed.push(v);
                }
            }
        }
        self.faces[face].conflict = NONE;
    }

    /// `addNewFacesFromHorizon`: a triangle from the eye to each horizon
    /// edge, twinned to the face beyond it and to its neighbours.
    fn new_faces_from_horizon(&mut self, eye: Vtx) -> Result<(), Refused> {
        let (mut side_prev, mut side_begin) = (NONE, NONE);
        for i in 0..self.horizon.len() {
            let h = self.horizon[i];
            let twin = self.edges[h].twin;
            let (head, tail) = (self.edges[twin].tail, self.edges[h].tail);
            let face = self.triangle(eye, head, tail)?;
            self.hull_faces.push(face);
            self.num_hull_faces += 1;
            let e2 = self.edge_at(face, 2);
            self.set_twin(e2, twin);
            let side = self.faces[face].edge;
            if side_prev != NONE {
                let n = self.edges[side].next;
                self.set_twin(n, side_prev);
            } else {
                side_begin = side;
            }
            self.new_faces.push(face);
            side_prev = side;
        }
        if side_begin == NONE {
            return Err(Refused("an empty horizon (the engine dereferences no edge)"));
        }
        let n = self.edges[side_begin].next;
        self.set_twin(n, side_prev);
        Ok(())
    }

    /// `doAdjacentMerge`. `failed` is reset on every call, as in the engine.
    fn adjacent_merge(&mut self, face: usize, wrt_large: bool, failed: &mut bool) -> Result<bool, Refused> {
        let mut hedge = self.faces[face].edge;
        *failed = false;
        let mut convex = true;
        let neg_tol = a::neg(self.tolerance);
        loop {
            let twin = self.edges[hedge].twin;
            let opp = self.edges[twin].face;
            let mut merge = false;
            if wrt_large {
                if self.faces[face].area > self.faces[opp].area {
                    if self.opposite_distance(hedge) > neg_tol {
                        merge = true;
                    } else if self.opposite_distance(twin) > neg_tol {
                        convex = false;
                    }
                } else if self.opposite_distance(twin) > neg_tol {
                    merge = true;
                } else if self.opposite_distance(hedge) > neg_tol {
                    convex = false;
                }
            } else if self.opposite_distance(hedge) > neg_tol || self.opposite_distance(twin) > neg_tol {
                merge = true;
            }
            if merge {
                let mut discarded = Vec::new();
                if !self.merge_adjacent(face, hedge, &mut discarded)? {
                    *failed = true;
                    return Ok(false);
                }
                self.num_hull_faces = self.num_hull_faces.wrapping_sub(discarded.len() as u32);
                for &d in &discarded {
                    self.delete_face_points(d, face);
                }
                return Ok(true);
            }
            hedge = self.edges[hedge].next;
            if hedge == self.faces[face].edge {
                break;
            }
        }
        if !convex {
            self.faces[face].state = State::NonConvex;
        }
        Ok(false)
    }

    /// `QuickHullFace::mergeAdjacentFace`.
    fn merge_adjacent(&mut self, this: usize, hedge_adj: usize, discarded: &mut Vec<usize>) -> Result<bool, Refused> {
        let hedge_opp = self.edges[hedge_adj].twin;
        let opp = self.edges[hedge_opp].face;
        discarded.push(opp);
        self.faces[opp].state = State::Deleted;
        let mut adj_prev = self.edges[hedge_adj].prev;
        let mut adj_next = self.edges[hedge_adj].next;
        let mut opp_prev = self.edges[hedge_opp].prev;
        let mut opp_next = self.edges[hedge_opp].next;
        let brk = adj_prev;
        while self.opposite_face(adj_prev) == opp {
            adj_prev = self.edges[adj_prev].prev;
            opp_next = self.edges[opp_next].next;
            if adj_prev == brk {
                return Ok(false);
            }
        }
        let brk = adj_next;
        while self.opposite_face(adj_next) == opp {
            opp_prev = self.edges[opp_prev].prev;
            adj_next = self.edges[adj_next].next;
            if adj_next == brk {
                return Ok(false);
            }
        }
        let stop = self.edges[opp_prev].next;
        let mut h = opp_next;
        let mut guard = 0;
        while h != stop {
            guard += 1;
            if guard > self.edges.len() {
                return Err(Refused("an open ring in a face merge"));
            }
            self.edges[h].face = this;
            h = self.edges[h].next;
        }
        if hedge_adj == self.faces[this].edge {
            self.faces[this].edge = adj_next;
        }
        if let Some(d) = self.connect(this, opp_prev, adj_next)? {
            discarded.push(d);
        }
        if let Some(d) = self.connect(this, adj_prev, opp_next)? {
            discarded.push(d);
        }
        self.recompute(this)?;
        Ok(true)
    }

    /// `QuickHullFace::connectHalfEdges`: two edges onto the same opposite
    /// face become one (a triangle beyond is dropped).
    fn connect(&mut self, this: usize, hedge_prev: usize, hedge: usize) -> Result<Option<usize>, Refused> {
        if self.opposite_face(hedge_prev) != self.opposite_face(hedge) {
            self.edges[hedge_prev].next = hedge;
            self.edges[hedge].prev = hedge_prev;
            return Ok(None);
        }
        let opp = self.opposite_face(hedge);
        let mut discarded = None;
        if hedge_prev == self.faces[this].edge {
            self.faces[this].edge = hedge;
        }
        let twin = self.edges[hedge].twin;
        let hedge_opp;
        if self.faces[opp].num_edges == 3 {
            hedge_opp = self.edges[self.edges[twin].prev].twin;
            self.faces[opp].state = State::Deleted;
            discarded = Some(opp);
        } else {
            hedge_opp = self.edges[twin].next;
            if self.faces[opp].edge == self.edges[hedge_opp].prev {
                self.faces[opp].edge = hedge_opp;
            }
            let pp = self.edges[self.edges[hedge_opp].prev].prev;
            self.edges[hedge_opp].prev = pp;
            self.edges[pp].next = hedge_opp;
        }
        let pp = self.edges[hedge_prev].prev;
        self.edges[hedge].prev = pp;
        self.edges[pp].next = hedge;
        self.edges[hedge].twin = hedge_opp;
        self.edges[hedge_opp].twin = hedge;
        self.recompute(opp)?;
        Ok(discarded)
    }

    /// `resolveUnclaimedPoints`: each unclaimed vertex to the new visible
    /// face it is farthest above by more than the tolerance.
    fn resolve_unclaimed(&mut self) {
        for i in 0..self.unclaimed.len() {
            let v = self.unclaimed[i];
            let mut max_dist = self.tolerance;
            let mut max_face = NONE;
            for j in 0..self.new_faces.len() {
                let f = self.new_faces[j];
                if self.faces[f].state == State::Visible {
                    let dist = self.faces[f].distance(self.verts[v].point);
                    if dist > max_dist {
                        max_dist = dist;
                        max_face = f;
                    }
                }
            }
            if max_face != NONE {
                self.add_point_to_face(max_face, v, max_dist);
            }
        }
        self.unclaimed.clear();
    }
}

impl QuickHull {
    /// `postMergeHull`: each visible face absorbs neighbours within three
    /// degrees while the merged polygon stays flat and convex.
    fn post_merge(&mut self) -> Result<(), Refused> {
        if arms::on("convexNoPostMerge") {
            return Ok(());
        }
        for i in 0..self.hull_faces.len() {
            let f = self.hull_faces[i];
            if self.faces[f].state == State::Visible {
                while self.post_adjacent_merge(f)? {}
            }
        }
        Ok(())
    }

    /// `doPostAdjacentMerge`; the merge's own result is not read, as in the
    /// engine.
    fn post_adjacent_merge(&mut self, face: usize) -> Result<bool, Refused> {
        let mut hedge = self.faces[face].edge;
        loop {
            let opp = self.opposite_face(hedge);
            let dot_p = dot(self.faces[face].normal, self.faces[opp].normal);
            if dot_p > MAXDOT_MINANG && self.faces[face].area >= self.faces[opp].area && self.can_merge(hedge)? {
                let mut discarded = Vec::new();
                self.merge_adjacent(face, hedge, &mut discarded)?;
                self.num_hull_faces = self.num_hull_faces.wrapping_sub(discarded.len() as u32);
                for &d in &discarded {
                    self.delete_face_points(d, face);
                }
                return Ok(true);
            }
            hedge = self.edges[hedge].next;
            if hedge == self.faces[face].edge {
                break;
            }
        }
        Ok(false)
    }

    /// `canMergeFaces`: the two faces' rings copied into one merged ring
    /// (the first face from the edge after `he`), its plane; no input vertex
    /// above it by more than the plane tolerance, every corner within the
    /// tolerance inside each edge, and no third face lined up on either end.
    fn can_merge(&self, he: usize) -> Result<bool, Refused> {
        let face1 = self.edges[he].face;
        let twin = self.edges[he].twin;
        let face2 = self.edges[twin].face;
        let n1 = self.faces[face1].num_edges as usize;
        let n2 = self.faces[face2].num_edges as usize;
        let blank = HalfEdge::new(Vtx { point: [0.0; 3], index: 0 }, 0);
        let mut tmp = vec![blank; n1 + n2];
        let f1 = self.faces[face1].edge;
        let start = if f1 != he { f1 } else { self.edges[f1].next };
        let (mut copy, mut cur) = (start, 0usize);
        let (mut twin_arena, mut twin_tmp, mut he_copy) = (NONE, NONE, NONE);
        loop {
            if cur >= n1 {
                return Err(Refused("a face ring longer than its edge count"));
            }
            tmp[cur].tail = self.edges[copy].tail;
            if copy == he {
                twin_arena = self.edges[copy].twin;
                he_copy = cur;
            }
            tmp[cur].next = if self.edges[copy].next == start { 0 } else { cur + 1 };
            tmp[cur].prev = if cur == 0 { n1 - 1 } else { cur - 1 };
            cur += 1;
            copy = self.edges[copy].next;
            if copy == start {
                break;
            }
        }
        let f2 = self.faces[face2].edge;
        copy = f2;
        loop {
            if cur >= n1 + n2 {
                return Err(Refused("a face ring longer than its edge count"));
            }
            tmp[cur].tail = self.edges[copy].tail;
            if twin_tmp == NONE && twin_arena == copy {
                twin_tmp = cur;
            }
            tmp[cur].next = if self.edges[copy].next == f2 { n1 } else { cur + 1 };
            tmp[cur].prev = if cur == n1 { n1 + n2 - 1 } else { cur - 1 };
            cur += 1;
            copy = self.edges[copy].next;
            if copy == f2 {
                break;
            }
        }
        if cur != n1 + n2 || he_copy == NONE || twin_tmp == NONE {
            return Err(Refused("a face ring shorter than its edge count"));
        }
        let (adj_prev, adj_next) = (tmp[he_copy].prev, tmp[he_copy].next);
        let (opp_prev, opp_next) = (tmp[twin_tmp].prev, tmp[twin_tmp].next);
        tmp[opp_prev].next = adj_next;
        tmp[adj_next].prev = opp_prev;
        tmp[adj_prev].next = opp_next;
        tmp[opp_next].prev = adj_prev;
        let merged = face_plane(&tmp, 0)?;
        for v in &self.verts {
            if plane_distance(merged.normal, merged.offset, v.point) > self.plane_tolerance {
                return Ok(false);
            }
        }
        let mut qhe = 0usize;
        let mut guard = 0;
        loop {
            let vertex = tmp[qhe].tail.point;
            let (edge_vector, _) = normalize(sub(tmp[tmp[qhe].next].tail.point, vertex));
            let out = neg(cross(merged.normal, edge_vector));
            let first = tmp[qhe].next;
            let mut test = first;
            loop {
                if dot(sub(tmp[test].tail.point, vertex), out) > self.tolerance {
                    return Ok(false);
                }
                test = tmp[test].next;
                if test == first {
                    break;
                }
            }
            qhe = tmp[qhe].next;
            guard += 1;
            if qhe == 0 {
                break;
            }
            if guard > tmp.len() {
                return Err(Refused("an open merged ring"));
            }
        }
        let opp = face2;
        let (mut adj_prev, mut adj_next) = (self.edges[he].prev, self.edges[he].next);
        let (mut opp_prev, mut opp_next) = (self.edges[twin].prev, self.edges[twin].next);
        let mut guard = 0;
        while self.opposite_face(adj_prev) == opp {
            adj_prev = self.edges[adj_prev].prev;
            opp_next = self.edges[opp_next].next;
            guard += 1;
            if guard > self.edges.len() {
                return Err(Refused("a face lined up all round (the engine loops)"));
            }
        }
        while self.opposite_face(adj_next) == opp {
            opp_prev = self.edges[opp_prev].prev;
            adj_next = self.edges[adj_next].next;
            guard += 1;
            if guard > 2 * self.edges.len() {
                return Err(Refused("a face lined up all round (the engine loops)"));
            }
        }
        if self.opposite_face(opp_prev) == self.opposite_face(adj_next) {
            return Ok(false);
        }
        if self.opposite_face(adj_prev) == self.opposite_face(opp_next) {
            return Ok(false);
        }
        Ok(true)
    }

    /// `fillConvexMeshDescFromQuickHull`: vertices in the order the visible
    /// faces first reach them; the polygons with the (first) largest face
    /// swapped to the front, each face's edges marked unvisited.
    fn fill_desc(&mut self) -> Result<Desc, Refused> {
        let n = self.hull_faces.len();
        let mut largest = 0usize;
        let mut num_indices = 0usize;
        for i in 0..n {
            let f = self.faces[self.hull_faces[i]];
            if f.state == State::Visible {
                num_indices += f.num_edges as usize;
                if f.num_edges > self.faces[self.hull_faces[largest]].num_edges {
                    largest = i;
                }
            }
        }
        if arms::on("convexLargestFaceNotFirst") {
            largest = 0;
        }
        let mut translate = vec![-1i32; self.verts.len()];
        let mut vertices = Vec::new();
        for i in 0..n {
            let f = self.hull_faces[i];
            if self.faces[f].state != State::Visible {
                continue;
            }
            let start = self.faces[f].edge;
            let mut he = start;
            loop {
                let t = self.edges[he].tail;
                if translate[t.index as usize] == -1 {
                    translate[t.index as usize] = vertices.len() as i32;
                    vertices.push(t.point);
                }
                he = self.edges[he].next;
                if he == start {
                    break;
                }
            }
        }
        let mut desc = Desc { vertices, indices: Vec::with_capacity(num_indices), polygons: Vec::new(), face_translate: Vec::new() };
        let mut index_offset: u16 = 0;
        for i in 0..n {
            let fi = if i == 0 {
                largest
            } else if i == largest {
                0
            } else {
                i
            };
            let f = self.hull_faces[fi];
            if self.faces[f].state != State::Visible {
                continue;
            }
            let start = self.faces[f].edge;
            let mut he = start;
            loop {
                self.edges[he].edge_index = u32::MAX;
                desc.indices.push(translate[self.edges[he].tail.index as usize] as u32);
                he = self.edges[he].next;
                if he == start {
                    break;
                }
            }
            let face = self.faces[f];
            let plane = [face.normal[0], face.normal[1], face.normal[2], a::neg(face.plane_offset)];
            desc.polygons.push((plane, index_offset, face.num_edges));
            index_offset = index_offset.wrapping_add(face.num_edges);
            desc.face_translate.push(fi as u16);
            self.faces[f].out_index = (desc.polygons.len() - 1) as u8;
        }
        if self.num_hull_faces as usize != desc.polygons.len() {
            return Err(Refused("the hull face count disagrees with the visible faces"));
        }
        Ok(desc)
    }

    /// `QuickHullConvexHullLib::createEdgeList`: edges numbered in the order
    /// the output polygons first walk them, each with its two polygons, and
    /// per polygon corner the edge that starts there.
    fn edge_list(&mut self, desc: &Desc) -> Result<(Vec<u8>, Vec<u16>), Refused> {
        let nb = desc.indices.len();
        let mut faces_by_edges = vec![0u8; nb];
        let mut edge_data = vec![0u16; nb];
        let mut edge_index: u16 = 0;
        let mut offset = 0usize;
        for i in 0..self.num_hull_faces as usize {
            let f = self.hull_faces[desc.face_translate[i] as usize];
            let start = self.faces[f].edge;
            let mut hedge = start;
            loop {
                if offset >= nb {
                    return Err(Refused("more polygon corners than the descriptor holds"));
                }
                if self.edges[hedge].edge_index == u32::MAX {
                    let e = edge_index as usize;
                    if 2 * e + 1 >= nb {
                        return Err(Refused("more edges than half the polygon corners"));
                    }
                    let next_twin = self.edges[self.edges[hedge].next].twin;
                    faces_by_edges[2 * e] = self.faces[self.edges[hedge].face].out_index;
                    faces_by_edges[2 * e + 1] = self.faces[self.edges[next_twin].face].out_index;
                    edge_data[offset] = edge_index;
                    self.edges[hedge].edge_index = edge_index as u32;
                    let p = self.edges[next_twin].prev;
                    self.edges[p].edge_index = edge_index as u32;
                    edge_index = edge_index.wrapping_add(1);
                } else {
                    edge_data[offset] = self.edges[hedge].edge_index as u16;
                }
                hedge = self.edges[hedge].next;
                offset += 1;
                if hedge == start {
                    break;
                }
            }
        }
        Ok((faces_by_edges, edge_data))
    }
}

// ---- the mesh builder ------------------------------------------------------

/// `ConvexMeshBuilder::build` after the hull library: the polygon data and
/// its checks, the mass properties, the local bounds, the support-vertex
/// map past 32 vertices, and the internal objects.
fn build_mesh(desc: Desc, faces_by_edges: Vec<u8>, edge_data: Vec<u16>, polygon_limit_reached: bool) -> Result<ConvexHull, Refused> {
    let verts = desc.vertices;
    let nv = verts.len();
    if desc.polygons.len() > POLYGON_LIMIT as usize {
        return Err(Refused("more than 255 hull polygons (the engine's failed cook here is not reached by the rows)"));
    }
    let vertex_data: Vec<u8> = desc.indices.iter().map(|&i| i as u8).collect();
    let mut polygons: Vec<HullPolygon> = desc
        .polygons
        .iter()
        .map(|&(plane, base, count)| HullPolygon { plane, vref8: base, nb_verts: count as u8, min_index: 0 })
        .collect();
    let faces_by_vertices = vertex_map(nv, &polygons, &vertex_data)?;
    let nb_edges = (vertex_data.len() / 2) as u16;
    for p in polygons.iter_mut() {
        let n = [p.plane[0], p.plane[1], p.plane[2]];
        let mut min = FLT_MAX;
        let mut min_index = 0xffu8;
        for (i, v) in verts.iter().enumerate() {
            let dp = dot(*v, n);
            if dp < min {
                min = dp;
                min_index = i as u8;
            }
        }
        p.min_index = min_index;
    }
    check_hull_polygons(&verts, &polygons, &vertex_data)?;
    let (mass, inertia, center_of_mass) = mass_properties(&verts, &polygons, &vertex_data)?;
    let last = verts[nv - 1];
    let (mut mn, mut mx) = (last, last);
    for p in &verts[..nv - 1] {
        for k in 0..3 {
            mn[k] = a::min(mn[k], p[k]);
            mx[k] = a::max(mx[k], p[k]);
        }
    }
    let aabb_center: V3 = std::array::from_fn(|k| a::mul(a::add(mn[k], mx[k]), 0.5));
    let aabb_extents: V3 = std::array::from_fn(|k| a::mul(a::sub(mx[k], mn[k]), 0.5));
    let big = if nv > GAUSS_MAP_LIMIT {
        Some(big_convex(&verts, &polygons, &vertex_data, &faces_by_edges, &edge_data, nb_edges)?)
    } else {
        None
    };
    let (internal_radius, internal_extents) = internal_objects(&polygons, center_of_mass, aabb_center, aabb_extents);
    Ok(ConvexHull {
        aabb_center,
        aabb_extents,
        center_of_mass,
        nb_edges,
        vertices: verts,
        polygons,
        faces_by_edges: faces_by_edges[..2 * nb_edges as usize].to_vec(),
        faces_by_vertices,
        vertex_data,
        big,
        internal_radius,
        internal_extents,
        mass,
        inertia,
        polygon_limit_reached,
    })
}

/// `ConvexHullBuilder::calculateVertexMapTable`: per vertex the first three
/// polygons that reach it; a vertex on fewer than three fails the cook.
fn vertex_map(nv: usize, polygons: &[HullPolygon], vertex_data: &[u8]) -> Result<Vec<u8>, Refused> {
    let mut map = vec![0u8; nv * 3];
    let mut marker = [0u8; 256];
    for (i, p) in polygons.iter().enumerate() {
        for k in 0..p.nb_verts as usize {
            let v = vertex_data[p.vref8 as usize + k] as usize;
            if marker[v] < 3 {
                map[v * 3 + marker[v] as usize] = i as u8;
                marker[v] += 1;
            }
        }
    }
    if marker[..nv].iter().any(|&m| m != 3) {
        return Err(Refused("a hull vertex on fewer than three polygons (the engine's failed cook here is not reached by the rows)"));
    }
    Ok(map)
}

/// `ConvexHullBuilder::checkHullPolygons`: each of the eight corners of the
/// bounds grown by 0.02 lies on or above some plane, and no vertex off a
/// polygon lies above its plane by more than 0.02 · (the summed largest
/// absolute coordinates, at least 1).
fn check_hull_polygons(verts: &[V3], polygons: &[HullPolygon], vertex_data: &[u8]) -> Result<(), Refused> {
    const FAILED: Refused = Refused("a hull the engine's polygon check rejects (its failed cook here is not reached by the rows)");
    if polygons.len() < 4 {
        return Err(FAILED);
    }
    let mut max = [-FLT_MAX; 3];
    let (mut hmax, mut hmin) = (verts[0], verts[0]);
    for v in verts {
        for k in 0..3 {
            if a::abs(v[k]) > max[k] {
                max[k] = a::abs(v[k]);
            }
        }
        for k in 0..3 {
            if v[k] > hmax[k] {
                hmax[k] = v[k];
            } else if v[k] < hmin[k] {
                hmin[k] = v[k];
            }
        }
    }
    let m = max.map(|x| a::add(x, CHECK_TOLERANCE));
    let (x, y, z) = (m[0], m[1], m[2]);
    let (nx, ny, nz) = (a::neg(x), a::neg(y), a::neg(z));
    let tests = [[x, y, z], [x, ny, nz], [x, y, nz], [x, ny, z], [nx, y, z], [nx, ny, z], [nx, y, nz], [nx, ny, nz]];
    let mut found = [false; 8];
    let span = a::add(
        a::add(a::max_nm(a::abs(hmax[0]), a::abs(hmin[0])), a::max_nm(a::abs(hmax[1]), a::abs(hmin[1]))),
        a::max_nm(a::abs(hmax[2]), a::abs(hmin[2])),
    );
    let test_eps = a::max_nm(a::mul(CHECK_TOLERANCE, span), CHECK_TOLERANCE);
    for p in polygons {
        let n = [p.plane[0], p.plane[1], p.plane[2]];
        for k in 0..8 {
            if !found[k] && a::add(dot(tests[k], n), p.plane[3]) >= 0.0 {
                found[k] = true;
            }
        }
        let corners = &vertex_data[p.vref8 as usize..p.vref8 as usize + p.nb_verts as usize];
        for (j, v) in verts.iter().enumerate() {
            if corners.contains(&(j as u8)) {
                continue;
            }
            if a::add(dot(*v, n), p.plane[3]) > test_eps {
                return Err(FAILED);
            }
        }
    }
    if found.iter().any(|f| !f) {
        return Err(FAILED);
    }
    Ok(())
}

// ---- mass properties -------------------------------------------------------

/// `computeMassInfo` with `computeVolumeIntegralsEberly` about the vertex
/// mean (binary64 integrals, binary32 inputs and results): the mass, the
/// inertia about the origin (column-major) and the centre of mass.
fn mass_properties(verts: &[V3], polygons: &[HullPolygon], vertex_data: &[u8]) -> Result<(f32, [f32; 9], V3), Refused> {
    let mut mean = [0.0f32; 3];
    for v in verts {
        mean = add(mean, *v);
    }
    let mean = scale(mean, a::div(1.0, verts.len() as f32));
    let (mass, tensor, com) = if arms::on("convexFloatEberly") {
        eberly::<f32>(verts, polygons, vertex_data, mean)
    } else {
        eberly::<f64>(verts, polygons, vertex_data, mean)
    };
    let mut inertia = [0.0f32; 9];
    for j in 0..3 {
        for i in 0..3 {
            inertia[j * 3 + i] = a::narrow(tensor[i][j]);
        }
    }
    let mass32 = a::narrow(mass);
    if !(inertia.iter().all(|x| x.is_finite()) && com.iter().all(|x| x.is_finite()) && mass32.is_finite()) {
        return Err(Refused("non-finite mass properties (the engine's error path is not reached by the rows)"));
    }
    if mass < 0.0 {
        return Err(Refused("a negative hull volume (the engine's absolute-value path is not reached by the rows)"));
    }
    Ok((mass32, inertia, com))
}

/// The integration's number type: binary64 in the engine; binary32 only for
/// the mutant.
trait Real: Copy + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::Mul<Output = Self> + std::ops::Div<Output = Self> + std::ops::Neg<Output = Self> {
    fn from32(x: f32) -> Self;
    fn from64(x: f64) -> Self;
    fn to64(self) -> f64;
}

impl Real for f64 {
    fn from32(x: f32) -> Self {
        x as f64
    }
    fn from64(x: f64) -> Self {
        x
    }
    fn to64(self) -> f64 {
        self
    }
}

impl Real for f32 {
    fn from32(x: f32) -> Self {
        x
    }
    fn from64(x: f64) -> Self {
        x as f32
    }
    fn to64(self) -> f64 {
        self as f64
    }
}

/// Eberly's subexpressions of one axis: f1, f2, f3, g0, g1, g2.
fn subexpressions<R: Real>(w0: R, w1: R, w2: R) -> [R; 6] {
    let temp0 = w0 + w1;
    let f1 = temp0 + w2;
    let temp1 = w0 * w0;
    let temp2 = temp1 + w1 * temp0;
    let f2 = temp2 + w2 * f1;
    let f3 = w0 * temp1 + w1 * temp2 + w2 * f2;
    [f1, f2, f3, f2 + w0 * (f1 + w0), f2 + w1 * (f1 + w1), f2 + w2 * (f1 + w2)]
}

/// `VolumeIntegratorEberly::computeVolumeIntegrals`: each polygon fanned
/// from its first corner (a fan triangle wound against the plane normal is
/// flipped), the vertices shifted by `origin`; the returned inertia is about
/// the world origin and the centre of mass shifted back.
fn eberly<R: Real>(verts: &[V3], polygons: &[HullPolygon], vertex_data: &[u8], origin: V3) -> (f64, [[f64; 3]; 3], V3) {
    let mult = [1.0 / 6.0, 1.0 / 24.0, 1.0 / 24.0, 1.0 / 24.0, 1.0 / 60.0, 1.0 / 60.0, 1.0 / 60.0, 1.0 / 120.0, 1.0 / 120.0, 1.0 / 120.0];
    let zero = R::from64(0.0);
    let mut intg = [zero; 10];
    for p in polygons {
        let data = &vertex_data[p.vref8 as usize..];
        let nv = p.nb_verts as usize;
        let n = [p.plane[0], p.plane[1], p.plane[2]];
        for j in 0..nv - 2 {
            let p0 = sub(verts[data[0] as usize], origin);
            let mut p1 = sub(verts[data[(j + 1) % nv] as usize], origin);
            let mut p2 = sub(verts[data[(j + 2) % nv] as usize], origin);
            let mut cp = cross(sub(p1, p0), sub(p2, p0));
            if dot(cp, n) < 0.0 {
                cp = neg(cp);
                std::mem::swap(&mut p1, &mut p2);
            }
            let w = |k: usize| (R::from32(p0[k]), R::from32(p1[k]), R::from32(p2[k]));
            let ((x0, x1, x2), (y0, y1, y2), (z0, z1, z2)) = (w(0), w(1), w(2));
            let (d0, d1, d2) = (R::from32(cp[0]), R::from32(cp[1]), R::from32(cp[2]));
            let [f1x, f2x, f3x, g0x, g1x, g2x] = subexpressions(x0, x1, x2);
            let [_, f2y, f3y, g0y, g1y, g2y] = subexpressions(y0, y1, y2);
            let [_, f2z, f3z, g0z, g1z, g2z] = subexpressions(z0, z1, z2);
            intg[0] = intg[0] + d0 * f1x;
            intg[1] = intg[1] + d0 * f2x;
            intg[2] = intg[2] + d1 * f2y;
            intg[3] = intg[3] + d2 * f2z;
            intg[4] = intg[4] + d0 * f3x;
            intg[5] = intg[5] + d1 * f3y;
            intg[6] = intg[6] + d2 * f3z;
            intg[7] = intg[7] + d0 * (y0 * g0x + y1 * g1x + y2 * g2x);
            intg[8] = intg[8] + d1 * (z0 * g0y + z1 * g1y + z2 * g2y);
            intg[9] = intg[9] + d2 * (x0 * g0z + x1 * g1z + x2 * g2z);
        }
    }
    let intg: [f64; 10] = std::array::from_fn(|i| (intg[i] * R::from64(mult[i])).to64());
    let mass = intg[0];
    let mut com = [a::narrow(intg[1] / mass), a::narrow(intg[2] / mass), a::narrow(intg[3] / mass)];
    let mut t = [[0.0f64; 3]; 3];
    t[0][0] = intg[5] + intg[6];
    t[1][1] = intg[4] + intg[6];
    t[2][2] = intg[4] + intg[5];
    t[0][1] = -intg[7];
    t[1][0] = t[0][1];
    t[1][2] = -intg[8];
    t[2][1] = t[1][2];
    t[0][2] = -intg[9];
    t[2][0] = t[0][2];
    if !(origin[0] == 0.0 && origin[1] == 0.0 && origin[2] == 0.0) {
        let s = add(com, origin);
        let c = com;
        let sq = |u: f32, v: f32| a::add(a::mul(u, u), a::mul(v, v));
        t[0][0] = t[0][0] - mass * a::sub(sq(c[1], c[2]), sq(s[1], s[2])) as f64;
        t[1][1] = t[1][1] - mass * a::sub(sq(c[2], c[0]), sq(s[2], s[0])) as f64;
        t[2][2] = t[2][2] - mass * a::sub(sq(c[0], c[1]), sq(s[0], s[1])) as f64;
        t[0][1] = t[0][1] + mass * a::sub(a::mul(c[0], c[1]), a::mul(s[0], s[1])) as f64;
        t[1][0] = t[0][1];
        t[1][2] = t[1][2] + mass * a::sub(a::mul(c[1], c[2]), a::mul(s[1], s[2])) as f64;
        t[2][1] = t[1][2];
        t[0][2] = t[0][2] + mass * a::sub(a::mul(c[2], c[0]), a::mul(s[2], s[0])) as f64;
        t[2][0] = t[0][2];
        com = s;
    }
    (mass, t, com)
}

// ---- support-vertex map ----------------------------------------------------

/// `BigConvexDataBuilder::computeValencies` then `precompute(16)`.
fn big_convex(verts: &[V3], polygons: &[HullPolygon], vertex_data: &[u8], faces_by_edges: &[u8], edge_data: &[u16], nb_edges: u16) -> Result<BigConvex, Refused> {
    let nv = verts.len();
    let mut valencies = vec![(0u16, 0u16); nv];
    for p in polygons {
        for j in 0..p.nb_verts as usize {
            let v = vertex_data[p.vref8 as usize + j] as usize;
            valencies[v].0 = valencies[v].0.wrapping_add(1);
        }
    }
    create_offsets(&mut valencies);
    let nb_adj = valencies[nv - 1].1 as u32 + valencies[nv - 1].0 as u32;
    let mut adjacent = vec![0u8; 2 * nb_edges as usize];
    let mut marker = [0u8; 256];
    const BROKEN: Refused = Refused("a vertex fan the adjacency walk cannot close");
    let mut put = |valencies: &mut Vec<(u16, u16)>, v: usize, x: u8| -> Result<(), Refused> {
        let slot = valencies[v].1 as usize;
        *adjacent.get_mut(slot).ok_or(BROKEN)? = x;
        valencies[v].1 = valencies[v].1.wrapping_add(1);
        Ok(())
    };
    let edge_faces = |slot: usize| -> Result<(u32, u32), Refused> {
        let e = (edge_data.get(slot).ok_or(BROKEN)?.wrapping_mul(2)) as usize;
        Ok((*faces_by_edges.get(e).ok_or(BROKEN)? as u32, *faces_by_edges.get(e + 1).ok_or(BROKEN)? as u32))
    };
    for (i, p) in polygons.iter().enumerate() {
        let i = i as u32;
        let nverts = p.nb_verts as usize;
        let data = &vertex_data[p.vref8 as usize..p.vref8 as usize + nverts];
        for j in 0..nverts {
            let vi = data[j];
            if marker[vi as usize] != 0 {
                continue;
            }
            let mut num_adj = 0u8;
            let mut prev_index = data[(j + 1) % nverts];
            put(&mut valencies, vi as usize, prev_index)?;
            num_adj = num_adj.wrapping_add(1);
            let (n0, n1) = edge_faces(p.vref8 as usize + j)?;
            let mut neighbor = if n0 == i { n1 } else { n0 };
            let mut guard = 0;
            while neighbor != i {
                guard += 1;
                if guard > polygons.len() {
                    return Err(BROKEN);
                }
                let q = polygons.get(neighbor as usize).ok_or(BROKEN)?;
                let nn = q.nb_verts as usize;
                let nd = &vertex_data[q.vref8 as usize..q.vref8 as usize + nn];
                let mut next_edge = 0usize;
                for k in 0..nn {
                    if nd[k] == vi {
                        let next = nd[(k + 1) % nn];
                        if next == prev_index {
                            prev_index = if k == 0 { nd[nn - 1] } else { nd[k - 1] };
                            next_edge = if k == 0 { nn - 1 } else { k - 1 };
                        } else {
                            prev_index = next;
                            next_edge = k;
                        }
                        put(&mut valencies, vi as usize, prev_index)?;
                        num_adj = num_adj.wrapping_add(1);
                        break;
                    }
                }
                let (n0, n1) = edge_faces(q.vref8 as usize + next_edge)?;
                neighbor = if n0 == neighbor { n1 } else { n0 };
            }
            marker[vi as usize] = num_adj;
        }
    }
    create_offsets(&mut valencies);
    let nb_samples = (6 * SUBDIV * SUBDIV) as u16;
    let samples = precompute(verts, &valencies, &adjacent, nb_samples as usize);
    Ok(BigConvex {
        subdiv: SUBDIV as u16,
        nb_samples,
        samples,
        valencies,
        nb_adj_verts: nb_adj,
        adjacent_verts: adjacent,
    })
}

/// `BigConvexData::CreateOffsets`.
fn create_offsets(valencies: &mut [(u16, u16)]) {
    valencies[0].1 = 0;
    for i in 1..valencies.len() {
        valencies[i].1 = valencies[i - 1].1.wrapping_add(valencies[i - 1].0);
    }
}

/// `BigConvexDataBuilder::precompute`: per cube-face sample direction the
/// vertex of least and of greatest projection, each climbed from the last
/// answer for the same direction slot.
fn precompute(verts: &[V3], valencies: &[(u16, u16)], adjacent: &[u8], nb: usize) -> Vec<u8> {
    let s = SUBDIV as usize;
    let mut samples = vec![0u8; 2 * nb];
    let mut start = [0u8; 12];
    let mut start2 = [0u8; 12];
    let half = a::mul((SUBDIV - 1) as f32, 0.5);
    for j in 0..s {
        for i in j..s {
            let isub = a::sub(1.0, a::div(i as f32, half));
            let jsub = a::sub(1.0, a::div(j as f32, half));
            let (t, _) = normalize([1.0, isub, jsub]);
            let (x, y, z) = (t[0], t[1], t[2]);
            let nx = a::neg(x);
            let dirs: [V3; 12] = [
                [nx, y, z],
                [x, y, z],
                [z, nx, y],
                [z, x, y],
                [y, z, nx],
                [y, z, x],
                [nx, z, y],
                [x, z, y],
                [y, nx, z],
                [y, x, z],
                [z, y, nx],
                [z, y, x],
            ];
            for d in 0..12 {
                start[d] = climb(verts, valencies, adjacent, dirs[d], start[d], 1.0);
                start2[d] = climb(verts, valencies, adjacent, dirs[d], start2[d], -1.0);
            }
            for k in 0..6 {
                let ksub = k * s * s;
                let offset = j + i * s + ksub;
                let offset2 = i + j * s + ksub;
                samples[offset] = start[k];
                samples[offset + nb] = start2[k];
                samples[offset2] = start[k + 6];
                samples[offset2 + nb] = start2[k + 6];
            }
        }
    }
    samples
}

/// `precomputeSample`: from `start`, step to a not-yet-visited neighbour
/// with a smaller signed projection until none is.
fn climb(verts: &[V3], valencies: &[(u16, u16)], adjacent: &[u8], dir: V3, start: u8, sign: f32) -> u8 {
    if arms::on("convexGlobalSupport") {
        let mut best = 0usize;
        for i in 1..verts.len() {
            if a::mul(sign, dot(verts[i], dir)) < a::mul(sign, dot(verts[best], dir)) {
                best = i;
            }
        }
        return best as u8;
    }
    let mut s = start;
    let mut visited = [0u32; 8];
    let mut minimum = a::mul(sign, dot(verts[s as usize], dir));
    loop {
        let initial = s;
        let (count, offset) = valencies[s as usize];
        for k in 0..count as usize {
            let n = adjacent[offset as usize + k];
            let dist = a::mul(sign, dot(verts[n as usize], dir));
            if dist < minimum {
                let (ind, mask) = ((n >> 5) as usize, 1u32 << (n & 31));
                if visited[ind] & mask == 0 {
                    visited[ind] |= mask;
                    minimum = dist;
                    s = n;
                }
            }
        }
        if s == initial {
            return s;
        }
    }
}

// ---- internal objects ------------------------------------------------------

/// `computeInternalObjects`: the least distance from the centre of mass to
/// a polygon plane, then `ComputeInternalExtent`'s box: the largest bounds
/// axis first (ray-plane distances from the centre of mass at the radius's
/// cube corners), then the next two from it.
fn internal_objects(polygons: &[HullPolygon], com: V3, center: V3, extents: V3) -> (f32, V3) {
    let mut radius = FLT_MAX;
    for p in polygons {
        let dist = a::abs(a::add(dot(com, [p.plane[0], p.plane[1], p.plane[2]]), p.plane[3]));
        if dist < radius {
            radius = dist;
        }
    }
    let e = sub(add(center, extents), sub(center, extents));
    let r = a::div(radius, SQRT_3);
    let m = usize::from(e[1] > e[0]);
    let largest = if e[2] > e[m] { 2 } else { m };
    let next3 = |i: usize| (i + 1 + (i >> 1)) & 3;
    let mut e0 = next3(largest);
    let mut e1 = next3(e0);
    if e[e0] < e[e1] {
        std::mem::swap(&mut e0, &mut e1);
    }
    let mut ext = [FLT_MAX; 3];
    let eps = INTERNAL_EPSILON;
    let neps = a::neg(eps);
    let base = |p: &HullPolygon| a::sub(a::neg(p.plane[3]), dot([p.plane[0], p.plane[1], p.plane[2]], com));
    for p in polygons {
        let d = p.plane[largest];
        if neps < d && d < eps {
            continue;
        }
        let num_base = base(p);
        let den = a::div(1.0, p.plane[largest]);
        let numn0 = a::mul(r, p.plane[e0]);
        let numn1 = a::mul(r, p.plane[e1]);
        for num in [
            a::sub(a::sub(num_base, numn0), numn1),
            a::add(a::sub(num_base, numn0), numn1),
            a::add(a::add(num_base, numn0), numn1),
            a::sub(a::add(num_base, numn0), numn1),
        ] {
            let x = a::max_nm(a::abs(a::mul(num, den)), r);
            if x < ext[largest] {
                ext[largest] = x;
            }
        }
    }
    for p in polygons {
        let add_d = a::add(p.plane[e0], p.plane[e1]);
        let sub_d = a::sub(p.plane[e0], p.plane[e1]);
        let num_base = base(p);
        let numn0 = a::mul(ext[largest], p.plane[largest]);
        for denom in [add_d, sub_d] {
            if neps < denom && denom < eps {
                continue;
            }
            for num in [a::sub(num_base, numn0), a::add(num_base, numn0)] {
                let q = if arms::on("convexInternalReciprocal") { a::mul(num, a::div(1.0, denom)) } else { a::div(num, denom) };
                let x = a::max_nm(a::abs(q), r);
                if x < ext[e0] {
                    ext[e0] = x;
                }
            }
        }
    }
    ext[e1] = ext[e0];
    (radius, ext)
}

#[cfg(test)]
mod tests {
    //! Replay against the engine's own cooks: every row's welded vertex
    //! array and every field of the cooked mesh (or the failed cook), bit
    //! for bit. Each mutant must differ on some row; the control (one
    //! flipped bit in each row's expected mass) must be caught on every
    //! cooked row.
    use super::*;
    use crate::particle::json::{parse, Value};

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn items(v: &Value) -> &[Value] {
        v.as_array().expect("row array")
    }
    fn word(v: &Value) -> u32 {
        let x = v.as_f64().expect("row word");
        assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
        x as u32
    }
    fn words(v: &Value) -> Vec<u32> {
        items(v).iter().map(word).collect()
    }
    fn bits(v: &[[f32; 3]]) -> Vec<u32> {
        v.iter().flatten().map(|x| x.to_bits()).collect()
    }

    struct Row {
        case: String,
        class: String,
        positions: Vec<[f32; 3]>,
        extracted: Vec<u32>,
        limit_warning: bool,
        native: Option<Vec<(&'static str, Vec<u32>)>>,
    }

    fn load(path: &str) -> Vec<Row> {
        let text = std::fs::read_to_string(path).expect("read rows");
        let mut out = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let r = parse(line.as_bytes()).expect("parse row");
            assert!(matches!(field(&r, "error"), Value::Null), "a harness error row");
            let positions: Vec<[f32; 3]> =
                items(field(&r, "positionBits")).iter().map(|p| { let w = words(p); [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])] }).collect();
            let extracted = match field(&r, "extractedBits") {
                Value::Null => bits(&positions),
                v => words(v),
            };
            let limit_warning = items(field(&r, "physxErrors"))
                .iter()
                .any(|e| items(e).first().and_then(|t| t.as_str()).is_some_and(|t| t.contains("maximum polygons limit")));
            let native = match field(&r, "cooked") {
                Value::Null => None,
                c => Some(native_fields(c)),
            };
            out.push(Row {
                case: field(&r, "case").as_str().unwrap().to_string(),
                class: field(&r, "class").as_str().unwrap().to_string(),
                positions,
                extracted,
                limit_warning,
                native,
            });
        }
        out
    }

    /// The engine's read-back, field by field.
    fn native_fields(c: &Value) -> Vec<(&'static str, Vec<u32>)> {
        assert_eq!(word(field(c, "type")), 2, "a convex mesh");
        assert!(matches!(field(c, "verticesByEdges16"), Value::Null), "no GPU edge data");
        let polygons: Vec<u32> = items(field(c, "polygons"))
            .iter()
            .flat_map(|p| {
                let mut w = words(field(p, "planeBits"));
                w.extend([word(field(p, "vref8")), word(field(p, "nbVerts")), word(field(p, "minIndex"))]);
                w
            })
            .collect();
        let big = match field(c, "big") {
            Value::Null => vec![0],
            b => {
                let mut w = vec![1, word(field(b, "subdiv")), word(field(b, "nbSamples")), word(field(b, "nbVerts")), word(field(b, "nbAdjVerts"))];
                w.extend(words(field(b, "samples")));
                w.extend(items(field(b, "valencies")).iter().flat_map(words));
                w.extend(words(field(b, "adjacentVerts")));
                w
            }
        };
        assert_eq!(big[0] == 1, field(c, "bigConvexData").as_bool().unwrap());
        vec![
            ("aabb", words(field(c, "aabbBits"))),
            ("centerOfMass", words(field(c, "centerOfMassBits"))),
            ("nbEdges", vec![word(field(c, "nbEdgesWord"))]),
            ("nbHullVertices", vec![word(field(c, "nbHullVertices"))]),
            ("nbPolygons", vec![word(field(c, "nbPolygons"))]),
            ("internalRadius", vec![word(field(c, "internalRadiusBits"))]),
            ("internalExtents", words(field(c, "internalExtentsBits"))),
            ("polygons", polygons),
            ("vertices", words(field(c, "vertexBits"))),
            ("facesByEdges", words(field(c, "facesByEdges8"))),
            ("facesByVertices", words(field(c, "facesByVertices8"))),
            ("vertexData", words(field(c, "vertexData8"))),
            ("nb", vec![word(field(c, "nb"))]),
            ("mass", vec![word(field(c, "massBits"))]),
            ("inertia", words(field(c, "inertiaBits"))),
            ("big", big),
        ]
    }

    /// The port's hull, in the same order.
    fn port_fields(h: &ConvexHull) -> Vec<(&'static str, Vec<u32>)> {
        let f = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
        let bytes = |v: &[u8]| v.iter().map(|&x| x as u32).collect::<Vec<u32>>();
        let polygons = h
            .polygons
            .iter()
            .flat_map(|p| {
                let mut w = f(&p.plane);
                w.extend([p.vref8 as u32, p.nb_verts as u32, p.min_index as u32]);
                w
            })
            .collect();
        let big = match &h.big {
            None => vec![0],
            Some(b) => {
                let mut w = vec![1, b.subdiv as u32, b.nb_samples as u32, b.valencies.len() as u32, b.nb_adj_verts];
                w.extend(bytes(&b.samples));
                w.extend(b.valencies.iter().flat_map(|&(c, o)| [c as u32, o as u32]));
                w.extend(bytes(&b.adjacent_verts));
                w
            }
        };
        let mut aabb = f(&h.aabb_center);
        aabb.extend(f(&h.aabb_extents));
        vec![
            ("aabb", aabb),
            ("centerOfMass", f(&h.center_of_mass)),
            ("nbEdges", vec![h.nb_edges as u32]),
            ("nbHullVertices", vec![h.vertices.len() as u32]),
            ("nbPolygons", vec![h.polygons.len() as u32]),
            ("internalRadius", vec![h.internal_radius.to_bits()]),
            ("internalExtents", f(&h.internal_extents)),
            ("polygons", polygons),
            ("vertices", bits(&h.vertices)),
            ("facesByEdges", bytes(&h.faces_by_edges)),
            ("facesByVertices", bytes(&h.faces_by_vertices)),
            ("vertexData", bytes(&h.vertex_data)),
            ("nb", vec![h.vertex_data.len() as u32]),
            ("mass", vec![h.mass.to_bits()]),
            ("inertia", f(&h.inertia)),
            ("big", big),
        ]
    }

    /// The first disagreement between the port and a row, if any; `Ok(false)`
    /// for a row whose engine result reads unset memory (the port refuses it).
    fn check(row: &Row, native: Option<&[(&'static str, Vec<u32>)]>) -> Result<bool, String> {
        let welded = if arms::on("convexNoWeld") { row.positions.clone() } else { weld(&row.positions, &[]).0 };
        if bits(&welded) != row.extracted {
            return Err("weld".into());
        }
        match (cook_convex_hull(&row.positions), native) {
            (Err(r), None) if engine_cooks_nothing(&r) => Ok(true),
            (Err(r), None) if r == UNSET_SIMPLEX => Ok(false),
            (Err(r), _) => Err(format!("refused: {}", r.0)),
            (Ok(_), None) => Err("cooked where the engine cooks nothing".into()),
            (Ok(h), Some(n)) => {
                for ((name, got), (_, want)) in port_fields(&h).iter().zip(n) {
                    if got != want {
                        return Err((*name).into());
                    }
                }
                if h.polygon_limit_reached != row.limit_warning {
                    return Err("polygonLimit".into());
                }
                Ok(true)
            }
        }
    }

    const ARMS: [&str; 8] = [
        "convexNoWeld",
        "convexFusedPlaneDistance",
        "convexCentroidDivide",
        "convexNoPostMerge",
        "convexFloatEberly",
        "convexGlobalSupport",
        "convexLargestFaceNotFirst",
        "convexInternalReciprocal",
    ];

    #[test]
    #[ignore = "needs MOLY_CONVEX_COOK_ROWS"]
    fn convex_cook_rows_match_native_bits() {
        let paths = std::env::var("MOLY_CONVEX_COOK_ROWS").expect("MOLY_CONVEX_COOK_ROWS");
        let rows: Vec<Row> = paths.split(',').flat_map(load).collect();
        assert!(!rows.is_empty());
        arms::set(None);
        let mut failures = Vec::new();
        let (mut cooked, mut nothing, mut limit, mut big, mut welded, mut undefined) = (0, 0, 0, 0, 0, 0);
        let mut classes = std::collections::BTreeMap::<&str, usize>::new();
        for row in &rows {
            *classes.entry(row.class.as_str()).or_default() += 1;
            welded += usize::from(row.extracted != bits(&row.positions));
            match &row.native {
                None => nothing += 1,
                Some(n) => {
                    cooked += 1;
                    limit += usize::from(row.limit_warning);
                    big += usize::from(n.last().unwrap().1[0] == 1);
                }
            }
            match check(row, row.native.as_deref()) {
                Ok(true) => {}
                Ok(false) => undefined += 1,
                Err(e) => failures.push(format!("{} ({}): {e}", row.case, row.class)),
            }
        }
        let mut control = 0;
        for row in &rows {
            if let Some(n) = &row.native {
                let mut flipped = n.clone();
                let mass = flipped.iter_mut().find(|(k, _)| *k == "mass").unwrap();
                mass.1[0] ^= 1;
                control += usize::from(check(row, Some(&flipped)).is_err());
            }
        }
        let mut red = Vec::new();
        for arm in ARMS {
            arms::set(Some(arm));
            let n = rows.iter().filter(|r| check(r, r.native.as_deref()).is_err()).count();
            red.push((arm, n));
        }
        arms::set(None);
        println!(
            "convex cook replay: {} rows {classes:?}; {cooked} cooked ({limit} at the polygon limit, {big} with the support map), \
             {nothing} the engine cooks nothing ({undefined} reading unset memory, refused), {welded} changed by the weld; mismatches {}; control caught {control} of {cooked}; \
             mutants (rows red) {red:?}",
            rows.len(),
            failures.len()
        );
        for f in failures.iter().take(30) {
            println!("  {f}");
        }
        assert!(failures.is_empty(), "{} rows differ", failures.len());
        assert_eq!(control, cooked, "the control must be caught on every cooked row");
        for (arm, n) in red {
            assert!(n > 0, "mutant {arm} is not red");
        }
    }
}
