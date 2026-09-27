//! Cooking of a non-convex mesh with the MeshCollider cooking options 30
//! (mesh cleaning, vertex welding, cooking for faster simulation and the
//! fast midphase): the engine's weld, the mesh cleaner, the BV4 tree of
//! four-lane quantised nodes over an AABB tree of at most four triangles per
//! leaf, the triangle remap into leaf order, the local bounds and the
//! active edge flags.
//!
//! - the mesh cleaner keeps the used vertices, merges exact duplicates by
//!   float equality in first-occurrence order, drops triangles of zero area
//!   (by the corners before the merge) or with a repeated corner after it,
//!   and drops exact duplicate index triples;
//! - the AABB tree splits a node of more than four triangles at the centre
//!   of its bounds on the axis of largest centre variance (strictly greater
//!   wins, the earlier axis on ties), partitioning in place; a split that
//!   leaves a side empty retries on the axis whose split is closest to half
//!   (each trial split reorders the range in turn), then halves the range;
//!   the leaves are renumbered in pre-order and the triangles remapped to
//!   that order;
//! - over more than four triangles the BV4 tree: the deeper child of every
//!   node moves to the second slot, each node takes its children and
//!   grandchildren in one of four layouts with the sorting code of each
//!   pair (eight direction bits, highest first), node bounds widened by
//!   2e-4, the bounds quantised to 15 bits per axis by truncation with the
//!   engine's two fix loops, lanes swizzled by axis, children numbered
//!   depth first after their siblings.
use super::cook::{active_edge_flags, weld};
use super::vector::*;
use super::{arms, Refused};
use crate::particle::armf as a;

const INVALID: u32 = 0xffff_ffff;
/// The node bounds are widened by this, 2e-4.
const BOX_EPS: f32 = f32::from_bits(0x3951_b717);
/// Triangles per AABB leaf.
const PER_LEAF: usize = 4;
/// The quantisation range.
const QUANT: f32 = 32767.0;
/// The geometric epsilon's scale of the largest bound, 2^-22.
const GEOM_EPS_SCALE: f32 = f32::from_bits(0x3480_0000);

/// The cooked BV4 tree: sixteen words per four-lane node (per axis the four
/// lanes' `max << 16 | min` quantised bounds, then the four lanes' data
/// words); empty when the mesh has at most four triangles (brute force).
#[derive(Clone, Debug)]
pub(super) struct Bv4Tree {
    pub(super) words: Vec<u32>,
    pub(super) init_data: u32,
    pub(super) min_coeff: V3,
    pub(super) max_coeff: V3,
    // Header words the queries do not read; the replay compares them with
    // the engine's cooked mesh.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) nb_nodes: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) geom_epsilon: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) local_bounds: [f32; 4],
}

pub(super) struct Bv4Cook {
    pub(super) vertices: Vec<V3>,
    pub(super) triangles: Vec<[u32; 3]>,
    /// For each cooked triangle its index in the authored triangles.
    pub(super) face_remap: Vec<u32>,
    pub(super) extra: Vec<u8>,
    pub(super) tree: Bv4Tree,
    pub(super) center: V3,
    pub(super) extents: V3,
}

pub(super) fn cook(positions: &[V3], triangles: &[[u32; 3]]) -> Result<Bv4Cook, Refused> {
    let flat: Vec<u32> = triangles.iter().flatten().copied().collect();
    let (welded, index) = weld(positions, &flat);
    let (vertices, cleaned, cleaner_remap) = mesh_cleaner(&welded, &index);
    if cleaned.is_empty() {
        return Err(Refused("every triangle of the mesh is degenerate (the cleaned mesh is empty)"));
    }
    let (tree, order) = build(&vertices, &cleaned)?;
    let (tris, face_remap): (Vec<[u32; 3]>, Vec<u32>) = if arms::on("bv4NoRemap") {
        (cleaned.clone(), cleaner_remap.clone())
    } else {
        order.iter().map(|&o| (cleaned[o as usize], cleaner_remap[o as usize])).unzip()
    };
    let last = vertices[vertices.len() - 1];
    let (mut mn, mut mx) = (last, last);
    for p in &vertices[..vertices.len() - 1] {
        for k in 0..3 {
            mn[k] = a::min(mn[k], p[k]);
            mx[k] = a::max(mx[k], p[k]);
        }
    }
    let mut largest = 0.0f32;
    for k in 0..3 {
        let (x, n) = (a::abs(mx[k]), a::abs(mn[k]));
        let m = if n > x { n } else { x };
        if m > largest {
            largest = m;
        }
    }
    let extra = active_edge_flags(&vertices, &tris, true)?;
    let tree = Bv4Tree { geom_epsilon: a::mul(largest, GEOM_EPS_SCALE), ..tree };
    Ok(Bv4Cook {
        center: std::array::from_fn(|k| a::mul(a::add(mn[k], mx[k]), 0.5)),
        extents: std::array::from_fn(|k| a::mul(a::sub(mx[k], mn[k]), 0.5)),
        vertices,
        triangles: tris,
        face_remap,
        extra,
        tree,
    })
}

/// The mesh cleaner with weld tolerance zero. Returns the kept vertices, the
/// kept triangles and each kept triangle's input index.
fn mesh_cleaner(vertices: &[V3], index: &[u32]) -> (Vec<V3>, Vec<[u32; 3]>, Vec<u32>) {
    let n = vertices.len();
    let mut used = vec![false; n];
    for &v in index {
        if (v as usize) < n {
            used[v as usize] = true;
        }
    }
    // Float equality of finite values is bit equality with the zero signs
    // merged; the first kept equal vertex is the one found.
    let key = |v: V3| v.map(|c| if c == 0.0 { 0 } else { c.to_bits() });
    let mut first: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
    let mut clean: Vec<V3> = Vec::new();
    let mut remap = vec![INVALID; n];
    for i in 0..n {
        if !used[i] {
            continue;
        }
        let v = vertices[i];
        remap[i] = *first.entry(key(v)).or_insert_with(|| {
            clean.push(v);
            (clean.len() - 1) as u32
        });
    }
    let mut seen = std::collections::HashSet::new();
    let (mut tris, mut source) = (Vec::new(), Vec::new());
    for (i, t) in index.chunks_exact(3).enumerate() {
        if t.iter().any(|&v| v as usize >= n) {
            continue;
        }
        let [p0, p1, p2] = [t[0], t[1], t[2]].map(|v| vertices[v as usize]);
        if mag2(cross(sub(p0, p1), sub(p0, p2))) == 0.0 {
            continue;
        }
        let r = [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]];
        if r[0] == r[1] || r[1] == r[2] || r[2] == r[0] {
            continue;
        }
        if seen.insert(r) {
            tris.push(r);
            source.push(i as u32);
        }
    }
    (clean, tris, source)
}

#[derive(Clone, Copy, Debug)]
struct AabbNode {
    mn: V3,
    mx: V3,
    /// The first of the two children's slots; `None` for a leaf.
    pos: Option<usize>,
    start: usize,
    nb: usize,
    /// The depth of the subtree (a leaf is one), carried with the node.
    depth: u32,
}

struct AabbTree {
    boxes: Vec<(V3, V3)>,
    centers: Vec<V3>,
    ind: Vec<u32>,
    pool: Vec<AabbNode>,
}

impl AabbTree {
    fn new(vertices: &[V3], tris: &[[u32; 3]]) -> Self {
        let n = tris.len();
        let mut boxes = Vec::with_capacity(n);
        let mut centers = Vec::with_capacity(n);
        for t in tris {
            let [v0, v1, v2] = t.map(|i| vertices[i as usize]);
            let mn: V3 = std::array::from_fn(|k| a::min(a::min(v0[k], v1[k]), v2[k]));
            let mx: V3 = std::array::from_fn(|k| a::max(a::max(v0[k], v1[k]), v2[k]));
            boxes.push((mn, mx));
            centers.push(std::array::from_fn(|k| a::mul(a::add(mx[k], mn[k]), 0.5)));
        }
        let empty = AabbNode { mn: [0.0; 3], mx: [0.0; 3], pos: None, start: 0, nb: 0, depth: 1 };
        let mut pool = vec![empty; 2 * n - 1];
        pool[0].nb = n;
        let mut tree = Self { boxes, centers, ind: (0..n as u32).collect(), pool };
        // Depth first, the first child's subtree before the second's.
        let mut count = 1usize;
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            if tree.subdivide(ni, &mut count) {
                let c = tree.pool[ni].pos.expect("just subdivided");
                stack.push(c + 1);
                stack.push(c);
            }
        }
        // Subtree depths, children before parents (children sit at higher
        // slots than their parent).
        for ni in (0..count).rev() {
            if let Some(c) = tree.pool[ni].pos {
                tree.pool[ni].depth = 1 + tree.pool[c].depth.max(tree.pool[c + 1].depth);
            }
        }
        tree
    }

    fn split(&mut self, ni: usize, axis: usize) -> usize {
        let node = self.pool[ni];
        let split = a::mul(a::add(node.mn[axis], node.mx[axis]), 0.5);
        let mut nb_pos = 0;
        for i in node.start..node.start + node.nb {
            let index = self.ind[i];
            if self.centers[index as usize][axis] > split {
                self.ind[i] = self.ind[node.start + nb_pos];
                self.ind[node.start + nb_pos] = index;
                nb_pos += 1;
            }
        }
        nb_pos
    }

    fn subdivide(&mut self, ni: usize, count: &mut usize) -> bool {
        let (start, nb) = (self.pool[ni].start, self.pool[ni].nb);
        let first = self.ind[start] as usize;
        let (mut mn, mut mx) = self.boxes[first];
        let mut means = self.centers[first];
        for i in 1..nb {
            let p = self.ind[start + i] as usize;
            let (bmn, bmx) = self.boxes[p];
            for k in 0..3 {
                mn[k] = a::min(mn[k], bmn[k]);
                mx[k] = a::max(mx[k], bmx[k]);
                means[k] = a::add(means[k], self.centers[p][k]);
            }
        }
        let coeff = a::div(1.0, nb as f32);
        means = means.map(|m| a::mul(m, coeff));
        self.pool[ni].mn = mn;
        self.pool[ni].mx = mx;
        if nb <= PER_LEAF {
            return false;
        }
        let mut var = [0.0f32; 3];
        for i in 0..nb {
            let c = self.centers[self.ind[start + i] as usize];
            for k in 0..3 {
                let d = a::sub(c[k], means[k]);
                var[k] = a::add(var[k], a::mul(d, d));
            }
        }
        let coeff1 = a::div(1.0, (nb - 1) as f32);
        var = var.map(|v| a::mul(v, coeff1));
        let mut axis = 0;
        if var[1] > var[axis] {
            axis = 1;
        }
        if var[2] > var[axis] {
            axis = 2;
        }
        let mut nb_pos = self.split(ni, axis);
        if nb_pos == 0 || nb_pos == nb {
            let mut res = [0.0f32; 3];
            for (ax, slot) in res.iter_mut().enumerate() {
                let p = self.split(ni, ax);
                let r = a::sub(a::div(p as f32, nb as f32), 0.5);
                *slot = a::mul(r, r);
            }
            let mut m = 0;
            if res[1] < res[m] {
                m = 1;
            }
            if res[2] < res[m] {
                m = 2;
            }
            nb_pos = self.split(ni, m);
            if nb_pos == 0 || nb_pos == nb {
                nb_pos = nb >> 1;
            }
        }
        let c = *count;
        *count += 2;
        self.pool[ni].pos = Some(c);
        self.pool[c].start = start;
        self.pool[c].nb = nb_pos;
        self.pool[c + 1].start = start + nb_pos;
        self.pool[c + 1].nb = nb - nb_pos;
        true
    }

    /// The leaves' triangles in pre-order; each leaf entry renumbered to its
    /// position in that order.
    fn walk_reorder(&mut self) -> Vec<u32> {
        let mut order = Vec::with_capacity(self.ind.len());
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let node = self.pool[ni];
            match node.pos {
                None => {
                    for i in 0..node.nb {
                        order.push(self.ind[node.start + i]);
                        self.ind[node.start + i] = (order.len() - 1) as u32;
                    }
                }
                Some(c) => {
                    stack.push(c + 1);
                    stack.push(c);
                }
            }
        }
        order
    }
}

fn center_extents(mn: V3, mx: V3) -> (V3, V3) {
    (std::array::from_fn(|k| a::mul(a::add(mn[k], mx[k]), 0.5)), std::array::from_fn(|k| a::mul(a::sub(mx[k], mn[k]), 0.5)))
}

/// The eight sorting directions, normalised as `PxVec3::normalize` does.
fn directions() -> [V3; 8] {
    let s = [1.0f32, -1.0];
    std::array::from_fn(|i| normalize([1.0 * s[i >> 2], 1.0 * s[(i >> 1) & 1], 1.0 * s[i & 1]]).0)
}

/// The sorting code of a node pair: bit 7 down to bit 0 for the eight
/// directions, set when the centre difference is not behind it.
fn node_sorting(b0: (V3, V3), b1: (V3, V3), dirs: &[V3; 8]) -> u32 {
    let c0: V3 = std::array::from_fn(|k| a::mul(a::add(b0.0[k], b0.1[k]), 0.5));
    let c1: V3 = std::array::from_fn(|k| a::mul(a::add(b1.0[k], b1.1[k]), 0.5));
    let d = sub(c0, c1);
    let mut code = 0;
    for (i, dv) in dirs.iter().enumerate() {
        if !(dot(d, *dv) < 0.0) {
            code |= 1 << (7 - i);
        }
    }
    code
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    Invalid,
    Leaf(u32),
    Node(usize),
}

#[derive(Clone, Copy, Debug)]
struct Lane {
    c: V3,
    e: V3,
    slot: Slot,
    pns: u32,
}

const EMPTY_LANE: Lane = Lane { c: [0.0; 3], e: [-1.0; 3], slot: Slot::Invalid, pns: 0 };

fn node_type(node: &[Lane; 4]) -> u32 {
    node.iter().filter(|l| !matches!(l.slot, Slot::Invalid)).count() as u32
}

/// The AABB tree, the leaf order and, over four triangles, the BV4 tree.
fn build(vertices: &[V3], tris: &[[u32; 3]]) -> Result<(Bv4Tree, Vec<u32>), Refused> {
    let n = tris.len();
    let mut src = AabbTree::new(vertices, tris);
    let order = src.walk_reorder();
    let (root_mn, root_mx) = (src.pool[0].mn, src.pool[0].mx);
    let (lc, le) = center_extents(root_mn, root_mx);
    let local_bounds = [lc[0], lc[1], lc[2], a::sqrt(mag2(le))];
    let mut tree = Bv4Tree {
        words: Vec::new(),
        init_data: 0,
        min_coeff: [0.0; 3],
        max_coeff: [0.0; 3],
        nb_nodes: 0,
        geom_epsilon: 0.0,
        local_bounds,
    };
    if n <= PER_LEAF {
        return Ok((tree, order));
    }
    let pool = &mut src.pool;
    // The deeper child to the second slot.
    let mut stack = vec![0usize];
    while let Some(ni) = stack.pop() {
        let Some(p) = pool[ni].pos else { continue };
        if pool[p].depth > pool[p + 1].depth {
            pool.swap(p, p + 1);
        }
        stack.push(p);
        stack.push(p + 1);
    }
    let dirs = directions();
    let bv = |ni: usize| (pool[ni].mn, pool[ni].mx);
    let mut nodes: Vec<[Lane; 4]> = vec![[EMPTY_LANE; 4]];
    let mut built = 0u32;
    let widened = |ni: usize| {
        let (c, e) = center_extents(pool[ni].mn, pool[ni].mx);
        (c, e.map(|x| a::add(x, BOX_EPS)))
    };
    let mut work = vec![(0usize, 0usize)];
    while let Some((tmp, ni)) = work.pop() {
        built += 1;
        let pp = pool[ni].pos.expect("an inner node");
        let (p, q) = (pp, pp + 1);
        let leaf = |ni: usize| pool[ni].pos.is_none();
        let set = |nodes: &mut Vec<[Lane; 4]>, work: &mut Vec<(usize, usize)>, lane: usize, ni: usize| -> Result<(), Refused> {
            let (c, e) = widened(ni);
            let slot = match pool[ni].pos {
                None => {
                    let (start, nb) = (pool[ni].start, pool[ni].nb);
                    if (0..nb).any(|j| src.ind[start + j] as usize != start + j) {
                        return Err(Refused("a BV4 leaf whose triangles are not consecutive after the remap"));
                    }
                    Slot::Leaf(((((start as u32) << 4) | (nb as u32 & 15)) << 1) | 1)
                }
                Some(_) => {
                    nodes.push([EMPTY_LANE; 4]);
                    let child = nodes.len() - 1;
                    work.push((child, ni));
                    Slot::Node(child)
                }
            };
            nodes[tmp][lane] = Lane { c, e, slot, pns: 0 };
            Ok(())
        };
        match (leaf(p), leaf(q)) {
            (true, true) => {
                set(&mut nodes, &mut work, 0, p)?;
                set(&mut nodes, &mut work, 1, q)?;
                nodes[tmp][0].pns = node_sorting(bv(p), bv(q), &dirs);
            }
            (true, false) => {
                let (np, nn) = (pool[q].pos.expect("inner"), pool[q].pos.expect("inner") + 1);
                set(&mut nodes, &mut work, 0, p)?;
                set(&mut nodes, &mut work, 1, np)?;
                set(&mut nodes, &mut work, 2, nn)?;
                nodes[tmp][0].pns = node_sorting(bv(p), bv(q), &dirs);
                nodes[tmp][2].pns = node_sorting(bv(np), bv(nn), &dirs);
            }
            (false, true) => {
                let (pp2, pn) = (pool[p].pos.expect("inner"), pool[p].pos.expect("inner") + 1);
                set(&mut nodes, &mut work, 2, q)?;
                set(&mut nodes, &mut work, 0, pp2)?;
                set(&mut nodes, &mut work, 1, pn)?;
                nodes[tmp][0].pns = node_sorting(bv(p), bv(q), &dirs);
                nodes[tmp][1].pns = node_sorting(bv(pp2), bv(pn), &dirs);
            }
            (false, false) => {
                let (pp2, pn) = (pool[p].pos.expect("inner"), pool[p].pos.expect("inner") + 1);
                let (np, nn) = (pool[q].pos.expect("inner"), pool[q].pos.expect("inner") + 1);
                set(&mut nodes, &mut work, 0, pp2)?;
                set(&mut nodes, &mut work, 1, pn)?;
                set(&mut nodes, &mut work, 2, np)?;
                set(&mut nodes, &mut work, 3, nn)?;
                nodes[tmp][0].pns = node_sorting(bv(p), bv(q), &dirs);
                nodes[tmp][1].pns = node_sorting(bv(pp2), bv(pn), &dirs);
                nodes[tmp][2].pns = node_sorting(bv(np), bv(nn), &dirs);
            }
        }
    }
    let nb_needed = built as usize * 4;
    let init = match node_type(&nodes[0]) {
        2 => 0,
        3 => 2,
        4 => 4,
        _ => INVALID,
    };
    // The largest magnitudes of the node minima and maxima.
    let mut minmax = [-FLT_MAX; 3];
    let mut maxmax = [-FLT_MAX; 3];
    for node in &nodes {
        for lane in node.iter().filter(|l| !matches!(l.slot, Slot::Invalid)) {
            for k in 0..3 {
                let m = a::abs(a::sub(lane.c[k], lane.e[k]));
                if m > minmax[k] {
                    minmax[k] = m;
                }
            }
            for k in 0..3 {
                let m = a::abs(a::add(lane.c[k], lane.e[k]));
                if m > maxmax[k] {
                    maxmax[k] = m;
                }
            }
        }
    }
    let min_q: V3 = std::array::from_fn(|k| if minmax[k] != 0.0 { a::div(QUANT, minmax[k]) } else { 0.0 });
    let max_q: V3 = std::array::from_fn(|k| if maxmax[k] != 0.0 { a::div(QUANT, maxmax[k]) } else { 0.0 });
    let min_coeff: V3 = std::array::from_fn(|k| a::div(minmax[k], QUANT));
    let max_coeff: V3 = std::array::from_fn(|k| a::div(maxmax[k], QUANT));
    // Flattened entries: quantised minima (signed), maxima (as stored) and
    // the data word.
    let mut dest: Vec<([i16; 3], [u16; 3], u32)> = vec![([0; 3], [0; 3], INVALID); nb_needed];
    let mut cur_id = 4usize;
    let mut flat_stack = vec![(0usize, 0usize)];
    while let Some((box_id, ni)) = flat_stack.pop() {
        let node = nodes[ni];
        let ctype = node_type(&node) as usize;
        for (i, lane) in node.iter().take(ctype).enumerate() {
            let m: V3 = std::array::from_fn(|k| a::sub(lane.c[k], lane.e[k]));
            let mm: V3 = std::array::from_fn(|k| a::add(lane.c[k], lane.e[k]));
            let mut qmn: [i16; 3] = std::array::from_fn(|k| a::mul(m[k], min_q[k]) as i64 as i16);
            let mut qmx: [u16; 3] = std::array::from_fn(|k| a::mul(mm[k], max_q[k]) as i64 as i16 as u16);
            for j in 0..3 {
                loop {
                    let mut leave = true;
                    let qmin = a::mul(qmn[j] as f32, min_coeff[j]);
                    let qmax = a::mul(qmx[j] as i16 as f32, max_coeff[j]);
                    if qmax < mm[j] && qmx[j] != 0x7fff {
                        qmx[j] = qmx[j].wrapping_add(1);
                        leave = false;
                    }
                    if qmin > m[j] && qmn[j] != 0 {
                        qmn[j] = qmn[j].wrapping_sub(1);
                        leave = false;
                    }
                    if leave {
                        break;
                    }
                }
            }
            let data = match lane.slot {
                Slot::Leaf(word) => word,
                _ => INVALID,
            };
            dest[box_id + i] = (qmn, qmx, data);
        }
        let mut children = Vec::new();
        for (i, lane) in node.iter().enumerate() {
            match lane.slot {
                Slot::Node(child) => {
                    let nid = cur_id;
                    cur_id += 4;
                    let ctype2 = (node_type(&nodes[child]).wrapping_sub(2)) << 1;
                    let word = ctype2.wrapping_add((nid as u32) << 11) | (lane.pns << 3);
                    dest[box_id + i].2 = word;
                    children.push((nid, child));
                }
                Slot::Invalid => dest[box_id + i] = ([0; 3], [0; 3], INVALID),
                Slot::Leaf(_) => {}
            }
        }
        for child in children.into_iter().rev() {
            flat_stack.push(child);
        }
    }
    let mut words = Vec::with_capacity(nb_needed * 4);
    for group in dest.chunks_exact(4) {
        for axis in 0..3 {
            for lane in group {
                words.push(((lane.1[axis] as u32) << 16) | (lane.0[axis] as u16 as u32));
            }
        }
        for lane in group {
            words.push(lane.2);
        }
    }
    tree.words = words;
    tree.init_data = init;
    tree.nb_nodes = nb_needed as u32;
    tree.min_coeff = min_coeff;
    tree.max_coeff = max_coeff;
    Ok((tree, order))
}
