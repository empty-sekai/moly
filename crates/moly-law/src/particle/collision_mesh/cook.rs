//! Cooking of a non-convex mesh with the default options: the engine's
//! weld, the RTree of the BVH33 midphase, the triangle remap, the local
//! bounds and the active edge flags. The options 30 cook (the BV4
//! midphase) is in `bv4_cook`; the weld and the edge flags are shared.
use super::bv4_cook::{self, Bv4Tree};
use super::vector::*;
use super::Refused;
use crate::particle::armf as a;

/// Half the widening of each triangle's bounds in the RTree build, 5e-4.
const TRIANGLE_BOUNDS_EPS: f32 = f32::from_bits(0x3a03_126f);
/// The build's trade-off between traversal and build cost, 0.55.
const TRADE_OFF: f32 = f32::from_bits(0x3f0c_cccd);
/// Leaf sizes by trade-off step.
const MAX_LEAF: [usize; 9] = [16, 14, 12, 10, 8, 7, 6, 5, 4];
/// The split axis extent shrinks by these after each of the first three
/// clusters (0.8, 0.7, 0.6).
const REDUCTION: [f32; 3] = [f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3f33_3333), f32::from_bits(0x3f19_999a)];
/// An edge between two faces is convex when their angle passes 0.1.
const ACTIVE_ANGLE: f32 = f32::from_bits(0x3dcc_cccd);
/// Two faces folded back onto each other: normals' dot below -0.999.
const FOLDED_DOT: f32 = f32::from_bits(0xbf7f_be77);
/// The host computes the edge angle in binary64; an angle this close to the
/// threshold would need the engine's single-precision atan2, which is not
/// transcribed.
const ANGLE_MARGIN: f64 = 1e-6;

/// One four-lane RTree page: per axis the four lanes' minima and maxima, and
/// each lane's node word (`start << 5 | (count - 1) << 1 | 1` for a leaf,
/// the child page's byte offset for an inner node).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Page {
    pub(super) min: [[f32; 4]; 3],
    pub(super) max: [[f32; 4]; 3],
    pub(super) ptrs: [u32; 4],
}

/// The size of a page as the node words address it.
pub(super) const PAGE_BYTES: u32 = 112;

/// The BVH33 RTree: its four-lane pages and header words.
#[derive(Clone, Debug)]
pub(super) struct RTree {
    pub(super) pages: Vec<Page>,
    // The RTree header words: no query reads them (one root page), the
    // replay compares them with the engine's cooked mesh.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) num_levels: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) total_nodes: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) tree_min: V3,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) tree_max: V3,
}

/// The midphase a mesh was cooked with.
#[derive(Clone, Debug)]
pub(super) enum Midphase {
    Bvh33(RTree),
    Bv4(Bv4Tree),
}

/// A cooked mesh: welded vertices, triangles in leaf order, the midphase
/// tree and the per-triangle edge flags.
#[derive(Clone, Debug)]
pub struct CookedMesh {
    pub(super) vertices: Vec<V3>,
    pub(super) triangles: Vec<[u32; 3]>,
    /// For each cooked triangle its index in the authored triangles.
    pub(super) source_index: Vec<u32>,
    pub(super) extra: Vec<u8>,
    pub(super) midphase: Midphase,
    pub(super) center: V3,
    pub(super) extents: V3,
}

impl CookedMesh {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[inline]
    pub(super) fn corners(&self, triangle: u32) -> Option<[V3; 3]> {
        let t = self.triangles.get(triangle as usize)?;
        Some([self.vertices[t[0] as usize], self.vertices[t[1] as usize], self.vertices[t[2] as usize]])
    }
}

/// Cook the authored arrays (positions in vertex order, triangles in index
/// order) with the given MeshCollider cooking options.
pub fn cook(positions: &[[f32; 3]], triangles: &[[u32; 3]], options: u32) -> Result<CookedMesh, Refused> {
    let bv4 = match options {
        0 => false,
        30 => true,
        o if o & 16 != 0 => return Err(Refused("fast-midphase cooking options other than 30 are not transcribed")),
        _ => return Err(Refused("cooking options other than the default and 30 are not transcribed")),
    };
    if positions.is_empty() || triangles.is_empty() {
        return Err(Refused("an empty collision mesh"));
    }
    if positions.len() > 0xffff {
        return Err(Refused("more than 65,535 vertices (32-bit cooked indices are not transcribed)"));
    }
    if triangles.len() >= 1 << 20 {
        return Err(Refused("more triangles than the RTree node words can address here"));
    }
    if positions.iter().any(|p| !p.iter().all(|v| v.is_finite())) {
        return Err(Refused("a non-finite collision mesh vertex"));
    }
    if triangles.iter().flatten().any(|&i| i as usize >= positions.len()) {
        return Err(Refused("a triangle corner past the vertex count"));
    }
    if bv4 {
        if triangles.len() >= 1 << 19 {
            return Err(Refused("more triangles than the BV4 node words can address here"));
        }
        let c = bv4_cook::cook(positions, triangles)?;
        return Ok(CookedMesh {
            vertices: c.vertices,
            triangles: c.triangles,
            source_index: c.face_remap,
            extra: c.extra,
            midphase: Midphase::Bv4(c.tree),
            center: c.center,
            extents: c.extents,
        });
    }
    let flat: Vec<u32> = triangles.iter().flatten().copied().collect();
    let (vertices, index) = weld(positions, &flat);
    let welded: Vec<[u32; 3]> = index.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    let tree = RTreeBuild::run(&vertices, &welded);
    let source_index = tree.permute.clone();
    let cooked: Vec<[u32; 3]> = tree.permute.iter().map(|&p| welded[p as usize]).collect();
    let last = vertices[vertices.len() - 1];
    let (mut mn, mut mx) = (last, last);
    for p in &vertices[..vertices.len() - 1] {
        for k in 0..3 {
            mn[k] = a::min(mn[k], p[k]);
            mx[k] = a::max(mx[k], p[k]);
        }
    }
    let extra = active_edge_flags(&vertices, &cooked, false)?;
    Ok(CookedMesh {
        center: std::array::from_fn(|k| a::mul(a::add(mn[k], mx[k]), 0.5)),
        extents: std::array::from_fn(|k| a::mul(a::sub(mx[k], mn[k]), 0.5)),
        vertices,
        triangles: cooked,
        source_index,
        extra,
        midphase: Midphase::Bvh33(RTree {
            pages: tree.pages,
            num_levels: tree.num_levels,
            total_nodes: tree.total_nodes,
            tree_min: tree.bounds_min,
            tree_max: tree.bounds_max,
        }),
    })
}

/// The engine's vertex weld without bone weights: a hash of the position
/// bits, the bucket chain walked from the most recently inserted vertex,
/// equality by float compare, vertices kept in first-occurrence order and
/// the indices remapped.
pub(super) fn weld(positions: &[V3], index: &[u32]) -> (Vec<V3>, Vec<u32>) {
    let n = positions.len();
    let mut x = (n as u32).wrapping_sub(1);
    for s in [1, 2, 4, 8, 16] {
        x |= x >> s;
    }
    let carry = u32::from(x == u32::MAX);
    let heads = x.wrapping_add(1).wrapping_add(carry) as usize;
    let mask = x.wrapping_add(carry);
    let mut head = vec![usize::MAX; heads.max(1)];
    let mut next = vec![usize::MAX; n];
    let mut out: Vec<V3> = Vec::with_capacity(n);
    let mut remap = vec![0u32; n];
    for (i, p) in positions.iter().enumerate() {
        let [b1, b2, b3] = p.map(f32::to_bits);
        let mut h = b2.wrapping_mul(11).wrapping_add(b1).wrapping_sub(b3.wrapping_mul(17)) & 0x7fff_ffff;
        h = (h ^ (h >> 12)) ^ (h >> 22);
        let bucket = (h & mask) as usize % head.len();
        let mut e = head[bucket];
        let mut found = None;
        while e != usize::MAX {
            let q = out[e];
            if q[0] == p[0] && q[1] == p[1] && q[2] == p[2] {
                found = Some(e);
                break;
            }
            e = next[e];
        }
        remap[i] = match found {
            Some(e) => e as u32,
            None => {
                let k = out.len();
                out.push(*p);
                next[k] = head[bucket];
                head[bucket] = k;
                k as u32
            }
        };
    }
    (out, index.iter().map(|&i| remap[i as usize]).collect())
}

#[derive(Clone, Copy)]
struct Node {
    /// > 0: a leaf of that many triangles; 0: inner; -1: empty.
    leaf: i64,
    child: usize,
    mn: V3,
    mx: V3,
}

struct RTreeBuild {
    bounds: Vec<(V3, V3)>,
    centers: Vec<V3>,
    permute: Vec<u32>,
    tree: Vec<Option<Node>>,
    max_levels: u32,
    max_leaf: usize,
    pages: Vec<Page>,
    bounds_min: V3,
    bounds_max: V3,
    num_levels: u32,
    total_nodes: u32,
}

fn axis_of(me: V3) -> usize {
    if me[0] > me[1] && me[0] > me[2] {
        0
    } else if me[1] > me[2] {
        1
    } else {
        2
    }
}

/// Quick-select of the first `k` of `a[left..=right]` by `centre[axis] <=`.
fn quick_select_first_k(a: &mut [u32], mut left: usize, mut right: usize, mut k: usize, centers: &[V3], axis: usize) {
    loop {
        if left > right || right >= a.len() || k == 0 {
            return;
        }
        let pivot_index = (left + right) >> 1;
        let pivot_value = a[pivot_index];
        a.swap(pivot_index, right);
        let mut store = left;
        for i in left..right {
            if centers[a[i] as usize][axis] <= centers[pivot_value as usize][axis] {
                a.swap(i, store);
                store += 1;
            }
        }
        a.swap(store, right);
        let pivot_new = store;
        let pivot_dist = pivot_new - left + 1;
        if pivot_dist == k {
            return;
        } else if k < pivot_dist {
            // pivot_new > left here, since k >= 1.
            right = pivot_new - 1;
        } else {
            k -= pivot_dist;
            left = pivot_new + 1;
        }
    }
}

impl RTreeBuild {
    fn run(vertices: &[V3], tris: &[[u32; 3]]) -> Self {
        let mut bounds = Vec::with_capacity(tris.len());
        let mut all_mn = [FLT_MAX; 3];
        let mut all_mx = [-FLT_MAX; 3];
        for t in tris {
            let [v0, v1, v2] = t.map(|i| vertices[i as usize]);
            let mn: V3 = std::array::from_fn(|k| a::sub(a::min(a::min(v0[k], v1[k]), v2[k]), TRIANGLE_BOUNDS_EPS));
            let mx: V3 = std::array::from_fn(|k| a::add(a::max(a::max(v0[k], v1[k]), v2[k]), TRIANGLE_BOUNDS_EPS));
            for k in 0..3 {
                all_mn[k] = a::min(all_mn[k], mn[k]);
                all_mx[k] = a::max(all_mx[k], mx[k]);
            }
            bounds.push((mn, mx));
        }
        let centers = bounds.iter().map(|(mn, mx)| std::array::from_fn(|k| a::add(mn[k], mx[k]))).collect();
        let step = (a::mul(a::max(0.0, TRADE_OFF), 9.0) as i64).clamp(0, 8) as usize;
        let mut build = Self {
            bounds,
            centers,
            permute: (0..tris.len() as u32).collect(),
            tree: Vec::new(),
            max_levels: 0,
            max_leaf: MAX_LEAF[step],
            pages: Vec::new(),
            bounds_min: all_mn,
            bounds_max: all_mx,
            num_levels: 0,
            total_nodes: 0,
        };
        build.sort4(0, tris.len(), 0);
        // Flatten: an empty node points at the first empty node's slot with
        // the leaf bit and inverted bounds.
        let mut nodes: Vec<(V3, V3, u32)> = Vec::with_capacity(build.tree.len());
        let mut first_empty: Option<usize> = None;
        for node in build.tree.iter().flatten() {
            if node.leaf < 0 {
                let slot = *first_empty.get_or_insert(nodes.len());
                nodes.push(([FLT_MAX; 3], [-FLT_MAX; 3], ((slot * 28) as u32) | 1));
            } else if node.leaf > 0 {
                nodes.push((node.mn, node.mx, ((node.child as u32) << 5) | (((node.leaf - 1) as u32) << 1) | 1));
            } else {
                nodes.push((node.mn, node.mx, (node.child * 28) as u32));
            }
        }
        build.pages = nodes.chunks_exact(4).map(|lanes| Page {
            min: std::array::from_fn(|k| std::array::from_fn(|l| lanes[l].0[k])),
            max: std::array::from_fn(|k| std::array::from_fn(|l| lanes[l].1[k])),
            ptrs: std::array::from_fn(|l| lanes[l].2),
        }).collect();
        build.num_levels = build.max_levels;
        build.total_nodes = nodes.len() as u32;
        build
    }

    /// SubSortQuick: four clusters of the range by quick-select on the
    /// longest axis, a leaf for a cluster of at most `max_leaf`, else a
    /// child page. Returns the range's bound, empty clusters adding zero.
    fn sort4(&mut self, base: usize, size: usize, level: u32) -> (V3, V3) {
        if level == 0 {
            self.max_levels = 1;
        } else {
            self.max_levels = self.max_levels.max(level + 1);
        }
        let cluster4 = (size / 4).max(1);
        let (mut mn, mut mx) = self.bounds[self.permute[base] as usize];
        for i in 1..size {
            let b = self.bounds[self.permute[base + i] as usize];
            for k in 0..3 {
                mx[k] = a::max(mx[k], b.1[k]);
                mn[k] = a::min(mn[k], b.0[k]);
            }
        }
        let mut me: V3 = std::array::from_fn(|k| a::sub(mx[k], mn[k]));
        let mut axis = axis_of(me);
        let start = self.tree.len();
        self.tree.extend([None; 4]);
        let leftover = size.saturating_sub(cluster4 * 3);
        let mut total = 0usize;
        let mut sub: (V3, V3) = ([0.0; 3], [0.0; 3]);
        for i in 0..4 {
            let off = cluster4 * i;
            let count = if i < 3 {
                if off < size {
                    let view = &mut self.permute[base..base + size];
                    quick_select_first_k(view, off, size - 1, cluster4, &self.centers, axis);
                    me[axis] = a::mul(me[axis], REDUCTION[i]);
                    axis = axis_of(me);
                }
                cluster4
            } else {
                leftover
            };
            total += count;
            let (node, child_bound) = if count <= self.max_leaf {
                if count != 0 && total <= size {
                    let (mut cmn, mut cmx) = self.bounds[self.permute[base + off] as usize];
                    for j in 1..count {
                        let b = self.bounds[self.permute[base + off + j] as usize];
                        for k in 0..3 {
                            cmn[k] = a::min(cmn[k], b.0[k]);
                            cmx[k] = a::max(cmx[k], b.1[k]);
                        }
                    }
                    (Node { leaf: count as i64, child: off + base, mn: cmn, mx: cmx }, (cmn, cmx))
                } else {
                    (Node { leaf: -1, child: 0, mn: [0.0; 3], mx: [0.0; 3] }, ([0.0; 3], [0.0; 3]))
                }
            } else {
                let child = self.tree.len();
                let bound = self.sort4(base + cluster4 * i, count, level + 1);
                (Node { leaf: 0, child, mn: bound.0, mx: bound.1 }, bound)
            };
            self.tree[start + i] = Some(node);
            sub = if i == 0 {
                child_bound
            } else {
                (std::array::from_fn(|k| a::min(sub.0[k], child_bound.0[k])),
                    std::array::from_fn(|k| a::max(sub.1[k], child_bound.1[k])))
            };
        }
        sub
    }
}

/// The edge list of the cooked triangles (unique edges sorted by their
/// corner pair, each face's three edges, each edge's faces) and the convex
/// edge flags: bit 3 for corners 0-1, bit 4 for 1-2, bit 5 for 2-0. A
/// boundary edge is active; an edge of two faces is active when the second
/// face's plane has the first face's opposite corner behind it and the
/// faces' angle passes 0.1, or, with the corner in front, when the faces
/// fold back (normals' dot below -0.999). With `multi` (the options-30
/// cook's reading of the builder) an edge of more than two faces takes the
/// builder's third branch: the faces are compared with the first face by
/// corner membership (and, once a second distinct face is found, with it),
/// a face with the same corners and a normal folded back marks the pair
/// double sided; one distinct face is active, two are the angle test when
/// double sided or else the angle test behind the second face's plane,
/// more are active. Without it such an edge refuses.
pub(super) fn active_edge_flags(vertices: &[V3], tris: &[[u32; 3]], multi: bool) -> Result<Vec<u8>, Refused> {
    let mut keys: Vec<((u32, u32), usize, usize)> = Vec::with_capacity(tris.len() * 3);
    for (i, t) in tris.iter().enumerate() {
        for (j, (p, q)) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])].into_iter().enumerate() {
            keys.push(((p.min(q), p.max(q)), i, j));
        }
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&r| keys[r].0);
    let mut edges: Vec<(u32, u32)> = Vec::new();
    let mut links = vec![[0usize; 3]; tris.len()];
    let mut previous = None;
    for r in order {
        let (key, i, j) = keys[r];
        if previous != Some(key) {
            edges.push(key);
        }
        previous = Some(key);
        links[i][j] = edges.len() - 1;
    }
    let mut faces: Vec<Vec<usize>> = vec![Vec::new(); edges.len()];
    for (i, link) in links.iter().enumerate() {
        for &e in link {
            faces[e].push(i);
        }
    }
    let corner = |i: u32| vertices[i as usize];
    let normal = |t: [u32; 3]| get_normalized(cross(sub(corner(t[1]), corner(t[0])), sub(corner(t[2]), corner(t[0]))));
    let angle_gt = |n0: V3, n1: V3| -> Result<bool, Refused> {
        let c = cross(n0, n1);
        let s = a::sqrt(mag2(c));
        let angle = (s as f64).atan2(dot(n0, n1) as f64);
        if ((angle.abs()) - 0.1).abs() < ANGLE_MARGIN {
            return Err(Refused("an edge angle too close to the convexity threshold for the host atan2"));
        }
        Ok((angle as f32).abs() > ACTIVE_ANGLE)
    };
    let behind = |t0: [u32; 3], t1: [u32; 3], r0: u32, r1: u32| -> Result<bool, Refused> {
        let opposite = opposite_corner(t0, r0, r1).ok_or(Refused("an edge that its first face does not hold"))?;
        Ok(plane_distance(corner(t1[0]), corner(t1[1]), corner(t1[2]), corner(opposite)) < 0.0)
    };
    let not_in = |v: u32, t: [u32; 3]| v != t[0] && v != t[1] && v != t[2];
    let distinct = |t: [u32; 3], of: [u32; 3]| not_in(t[0], of) || not_in(t[1], of) || not_in(t[2], of);
    let mut active = vec![false; edges.len()];
    for (e, &(r0, r1)) in edges.iter().enumerate() {
        match faces[e].as_slice() {
            [_] => active[e] = true,
            &[f0, f1] => {
                let (t0, t1) = (tris[f0], tris[f1]);
                let (n0, n1) = (normal(t0), normal(t1));
                if behind(t0, t1, r0, r1)? {
                    active[e] = angle_gt(n0, n1)?;
                } else if dot(n0, n1) < FOLDED_DOT {
                    active[e] = true;
                }
            }
            _ if !multi => return Err(Refused("an edge shared by more than two triangles")),
            list => {
                let t0 = tris[list[0]];
                let mut t1 = [0u32; 3];
                let mut unique = 1;
                let (mut ds0, mut ds1) = (false, false);
                for &f in &list[1..] {
                    let t = tris[f];
                    if distinct(t, t0) {
                        if unique == 2 {
                            if distinct(t, t1) {
                                unique += 1;
                                break;
                            } else if dot(normal(t1), normal(t)) < FOLDED_DOT {
                                ds1 = true;
                            }
                        } else {
                            t1 = t;
                            unique += 1;
                        }
                    } else if dot(normal(t0), normal(t)) < FOLDED_DOT {
                        ds0 = true;
                    }
                }
                active[e] = match unique {
                    1 => true,
                    2 if ds0 || ds1 => angle_gt(normal(t0), normal(t1))?,
                    2 => behind(t0, t1, r0, r1)? && angle_gt(normal(t0), normal(t1))?,
                    _ => true,
                };
            }
        }
    }
    Ok(links.iter().map(|l| {
        (u8::from(active[l[0]]) << 3) | (u8::from(active[l[1]]) << 4) | (u8::from(active[l[2]]) << 5)
    }).collect())
}

/// The corner of `t` opposite the edge `(p, q)`, as the edge-list builder
/// finds it.
fn opposite_corner(t: [u32; 3], p: u32, q: u32) -> Option<u32> {
    let [r0, r1, r2] = t;
    if p == r0 {
        if q == r1 {
            Some(r2)
        } else if q == r2 {
            Some(r1)
        } else {
            None
        }
    } else if p == r1 {
        if q == r0 {
            Some(r2)
        } else if q == r2 {
            Some(r0)
        } else {
            None
        }
    } else if p == r2 {
        if q == r1 {
            Some(r0)
        } else if q == r0 {
            Some(r1)
        } else {
            None
        }
    } else {
        None
    }
}

/// `PxPlane(p0, p1, p2).distance(x)`.
fn plane_distance(p0: V3, p1: V3, p2: V3, x: V3) -> f32 {
    let n = get_normalized(cross(sub(p1, p0), sub(p2, p0)));
    let d = a::neg(dot(p0, n));
    a::add(dot(x, n), d)
}
