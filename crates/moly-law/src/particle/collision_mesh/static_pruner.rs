//! The static pruner of the physics scene, as the engine's PhysX 4.1 build
//! keeps it: the order in which one scene overlap meets the static shapes.
//!
//! The scene is created with an incrementally rebuilt dynamic AABB tree for
//! its static shapes. Its query callback collects every shape the pruner
//! visits, in visit order, and stops each before any narrowphase, so that
//! order is the order the CollisionModule's candidates arrive in. The pruner
//! holds:
//!
//! - a pool of objects (payload and box, in pool index order). Adding
//!   appends; removing moves the last object into the removed slot, and the
//!   slot past the live count keeps the box it last held. Handles recycle
//!   last-removed-first;
//! - the main tree over the pool, at most four objects per leaf: each node
//!   takes the enclosing box of its objects and splits at the centre of that
//!   box along the axis of greatest variance of the objects' box centres
//!   (objects above the split move to the front, in place, and form the
//!   first child), or half and half when one side would be empty. A commit
//!   with no tree builds it at once; after that, adds and removes rebuild it
//!   one physics step at a time (`step`) from a copy of the boxes taken when
//!   the rebuild starts, and the rebuilt tree replaces the old one at the
//!   commit after it finishes;
//! - the bucket: two incremental trees holding the objects added since the
//!   current tree was built, one for adds before the rebuild in flight
//!   started and one for adds after. The first is dropped when the rebuilt
//!   tree comes in;
//! - the removals made while a rebuild runs, replayed onto the rebuilt tree.
//!
//! An overlap visits the main tree depth first, the first child before the
//! second, and in a leaf its objects in stored order (in a leaf of more than
//! one object each is tested against the query by its own box); then the
//! bucket's two trees in the same way. The query box is the query's centre
//! and half extents grown by 1.01, and a box meets it when on every axis the
//! distance of the centres is at most the sum of the half extents.
//!
//! What the model does not hold refuses by name (`Unmodeled`) and keeps
//! refusing: non-finite boxes, an object added twice, the removal of an
//! object the pruner does not hold. The pruner has no bounds update here:
//! static site and fixture colliders do not move.
use super::arms;
use std::collections::{HashMap, VecDeque};

/// Objects per leaf, main tree and bucket trees.
const LEAF_LIMIT: usize = 4;
/// A leaf left without objects takes this box at a refit: minimum at plus
/// this value and maximum at minus it, so no query meets it.
const EMPTY_EXTENT_BITS: u32 = 0x5a60_b17f;
/// The query box grows by this factor.
const QUERY_INFLATION: f32 = 1.01;
/// The scene's rebuild rate hint; the pruner keeps it less 3.
const REBUILD_RATE_HINT: u32 = 100;
const INVALID: u32 = u32::MAX;

/// A box: minimum x, y, z, maximum x, y, z.
pub type Bounds = [f32; 6];

/// Why the model cannot give the engine's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmodeled(pub &'static str);

/// The state words of the pruner, for comparison with the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub has_tree: bool,
    pub progress: u32,
    pub hint: u32,
    pub adaptive: i32,
    pub uncommitted: bool,
    pub needs_new_tree: bool,
    pub pool_count: u32,
    pub bucket: [u32; 3],
}

// The vector minimum and maximum: a pair of zeros gives the negative zero
// for the minimum and the positive zero for the maximum.
fn fmin(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else if b < a {
        b
    } else if a.is_sign_negative() {
        a
    } else {
        b
    }
}

fn fmax(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else if b > a {
        b
    } else if a.is_sign_positive() {
        a
    } else {
        b
    }
}

fn min3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [fmin(a[0], b[0]), fmin(a[1], b[1]), fmin(a[2], b[2])]
}

fn max3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [fmax(a[0], b[0]), fmax(a[1], b[1]), fmax(a[2], b[2])]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn lo(b: &Bounds) -> [f32; 3] {
    [b[0], b[1], b[2]]
}

fn hi(b: &Bounds) -> [f32; 3] {
    [b[3], b[4], b[5]]
}

fn join(mn: [f32; 3], mx: [f32; 3]) -> Bounds {
    [mn[0], mn[1], mn[2], mx[0], mx[1], mx[2]]
}

fn union(a: &Bounds, b: &Bounds) -> Bounds {
    join(min3(lo(a), lo(b)), max3(hi(a), hi(b)))
}

fn empty_bounds() -> Bounds {
    let e = f32::from_bits(EMPTY_EXTENT_BITS);
    [e, e, e, -e, -e, -e]
}

/// The largest axis, ties to the lower axis.
fn largest_axis(v: [f32; 3]) -> usize {
    let m = if v[1] > v[0] || (arms::on("axis-tie-up") && v[1] >= v[0]) { 1 } else { 0 };
    if v[2] > v[m] {
        2
    } else {
        m
    }
}

/// The query test of an overlap: centre and half extents of the grown box.
#[derive(Clone, Copy, Debug)]
struct Query {
    centre: [f32; 3],
    extents: [f32; 3],
}

impl Query {
    fn new(centre: [f32; 3], half: [f32; 3]) -> Self {
        let inflation = if arms::on("query-not-inflated") { 1.0 } else { QUERY_INFLATION };
        let grown = half.map(|h| h * inflation);
        let (mn, mx) = (sub3(centre, grown), add3(centre, grown));
        Self {
            centre: add3(mn, mx).map(|v| v * 0.5),
            extents: sub3(mx, mn).map(|v| v * 0.5),
        }
    }

    fn meets(&self, mn: [f32; 3], mx: [f32; 3]) -> bool {
        let c = add3(mx, mn).map(|v| v * 0.5);
        let e = sub3(mx, mn).map(|v| v * 0.5);
        (0..3).all(|k| self.extents[k] + e[k] >= (c[k] - self.centre[k]).abs())
    }

    fn meets_box(&self, b: &Bounds) -> bool {
        self.meets(lo(b), hi(b))
    }
}

// ---------------------------------------------------------------- pool

#[derive(Clone, Debug, PartialEq)]
struct Pool {
    count: usize,
    objects: Vec<u64>,
    boxes: Vec<Bounds>,
    index_to_handle: Vec<u32>,
    handle_to_index: Vec<u32>,
    first_recycled: u32,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            count: 0,
            objects: Vec::new(),
            boxes: Vec::new(),
            index_to_handle: Vec::new(),
            handle_to_index: Vec::new(),
            first_recycled: INVALID,
        }
    }
}

impl Pool {
    fn add(&mut self, id: u64, b: Bounds) -> u32 {
        let index = self.count;
        self.count += 1;
        let handle = if self.first_recycled != INVALID {
            let h = self.first_recycled;
            self.first_recycled = self.handle_to_index[h as usize];
            h
        } else {
            index as u32
        };
        if self.boxes.len() == index {
            self.boxes.push(b);
            self.objects.push(id);
            self.index_to_handle.push(handle);
        } else {
            self.boxes[index] = b;
            self.objects[index] = id;
            self.index_to_handle[index] = handle;
        }
        if self.handle_to_index.len() <= handle as usize {
            self.handle_to_index.resize(handle as usize + 1, INVALID);
        }
        self.handle_to_index[handle as usize] = index as u32;
        handle
    }

    /// Removes a handle's object; returns its pool index and the old last
    /// index, whose object moved into it.
    fn remove(&mut self, handle: u32) -> (u32, u32) {
        let index = self.handle_to_index[handle as usize] as usize;
        self.count -= 1;
        let last = self.count;
        if last != index {
            let moved = self.index_to_handle[last];
            self.boxes[index] = self.boxes[last];
            self.objects[index] = self.objects[last];
            self.index_to_handle[index] = moved;
            self.handle_to_index[moved as usize] = index as u32;
        }
        if arms::on("stale-box-cleared") {
            self.boxes[last] = [0.0; 6];
        }
        self.handle_to_index[handle as usize] = self.first_recycled;
        self.first_recycled = handle;
        (index as u32, last as u32)
    }
}

// ---------------------------------------------------------------- main tree

#[derive(Clone, Debug, PartialEq)]
struct Node {
    bounds: Bounds,
    leaf: bool,
    /// A leaf's first entry in `indices`; an inner node's first child (the
    /// second follows it).
    first: u32,
    /// A leaf's live object count.
    count: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Tree {
    nodes: Vec<Node>,
    indices: Vec<u32>,
    parents: Vec<u32>,
    marked: Vec<bool>,
    total_prims: u32,
}

/// One node of a build: its box, its object range, and its first child.
#[derive(Clone, Debug, PartialEq)]
struct BuildNode {
    bounds: Bounds,
    first: u32,
    count: u32,
    child: Option<u32>,
}

/// A build over a copy of the pool boxes. Nodes are numbered in allocation
/// order, children in pairs after every node allocated before them.
#[derive(Clone, Debug, PartialEq)]
struct Build {
    boxes: Vec<Bounds>,
    cache: Vec<[f32; 3]>,
    indices: Vec<u32>,
    nodes: Vec<BuildNode>,
    queue: VecDeque<u32>,
    total_prims: u32,
}

impl Build {
    fn new(boxes: &[Bounds]) -> Self {
        let n = boxes.len();
        Self {
            boxes: boxes.to_vec(),
            cache: boxes
                .iter()
                .map(|b| add3(hi(b), lo(b)).map(|v| v * 0.5))
                .collect(),
            indices: (0..n as u32).collect(),
            nodes: vec![BuildNode {
                bounds: [0.0; 6],
                first: 0,
                count: n as u32,
                child: None,
            }],
            queue: VecDeque::new(),
            total_prims: 0,
        }
    }

    fn subdivide(&mut self, k: u32) {
        let (first, count) = (
            self.nodes[k as usize].first as usize,
            self.nodes[k as usize].count as usize,
        );
        let prims = &mut self.indices[first..first + count];
        let (boxes, cache) = (&self.boxes, &self.cache);
        let mut bv = boxes[prims[0] as usize];
        let mut sum = cache[prims[0] as usize];
        for &p in &prims[1..] {
            sum = add3(sum, cache[p as usize]);
            bv = union(&bv, &boxes[p as usize]);
        }
        self.nodes[k as usize].bounds = bv;
        self.total_prims += count as u32;
        if count <= LEAF_LIMIT {
            return;
        }
        let coeff = 1.0f32 / count as f32;
        let means = sum.map(|v| v * coeff);
        let mut vars = [0.0f32; 3];
        for &p in prims.iter() {
            let d = sub3(cache[p as usize], means);
            vars = add3(vars, [d[0] * d[0], d[1] * d[1], d[2] * d[2]]);
        }
        let coeff = 1.0f32 / (count - 1) as f32;
        let axis = largest_axis(vars.map(|v| v * coeff));
        let split = (bv[axis] + bv[axis + 3]) * 0.5;
        let mut pos = 0usize;
        for i in 0..count {
            let index = prims[i];
            if cache[index as usize][axis] > split || (arms::on("split-ge") && cache[index as usize][axis] >= split) {
                prims[i] = prims[pos];
                prims[pos] = index;
                pos += 1;
            }
        }
        if pos == 0 || pos == count {
            pos = count >> 1;
        }
        let c = self.nodes.len() as u32;
        self.nodes.push(BuildNode {
            bounds: [0.0; 6],
            first: first as u32,
            count: pos as u32,
            child: None,
        });
        self.nodes.push(BuildNode {
            bounds: [0.0; 6],
            first: (first + pos) as u32,
            count: (count - pos) as u32,
            child: None,
        });
        self.nodes[k as usize].child = Some(c);
    }

    fn full(boxes: &[Bounds]) -> Tree {
        fn rec(b: &mut Build, k: u32) {
            b.subdivide(k);
            if let Some(c) = b.nodes[k as usize].child {
                rec(b, c);
                rec(b, c + 1);
            }
        }
        let mut b = Self::new(boxes);
        rec(&mut b, 0);
        b.finish()
    }

    /// One step of the progressive build with a budget of `limit` objects;
    /// false when it found nothing left to split.
    fn progress(&mut self, limit: u32) -> bool {
        if self.queue.is_empty() {
            return false;
        }
        let mut total = 0u32;
        while total < limit {
            let next = if arms::on("progressive-lifo") { self.queue.pop_back() } else { self.queue.pop_front() };
            let Some(k) = next else {
                break;
            };
            self.subdivide(k);
            if let Some(c) = self.nodes[k as usize].child {
                let (first, second) = if arms::on("progressive-pos-first") { (c, c + 1) } else { (c + 1, c) };
                self.queue.push_back(first);
                self.queue.push_back(second);
            }
            total += self.nodes[k as usize].count;
        }
        true
    }

    fn finish(self) -> Tree {
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .map(|n| match n.child {
                Some(c) => Node {
                    bounds: n.bounds,
                    leaf: false,
                    first: c,
                    count: 0,
                },
                None => Node {
                    bounds: n.bounds,
                    leaf: true,
                    first: n.first,
                    count: n.count,
                },
            })
            .collect();
        let mut parents = vec![0u32; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            if !n.leaf {
                parents[n.first as usize] = i as u32;
                parents[n.first as usize + 1] = i as u32;
            }
        }
        let marked = vec![false; nodes.len()];
        Tree {
            nodes,
            indices: self.indices,
            parents,
            marked,
            total_prims: self.total_prims,
        }
    }
}

impl Tree {
    fn refit_node(&mut self, i: usize, boxes: &[Bounds]) {
        let n = &self.nodes[i];
        let b = if n.leaf {
            if n.count == 0 {
                empty_bounds()
            } else {
                let prims = &self.indices[n.first as usize..(n.first + n.count) as usize];
                prims[1..].iter().fold(boxes[prims[0] as usize], |acc, &p| {
                    union(&acc, &boxes[p as usize])
                })
            }
        } else {
            union(
                &self.nodes[n.first as usize].bounds,
                &self.nodes[n.first as usize + 1].bounds,
            )
        };
        self.nodes[i].bounds = b;
    }

    fn full_refit(&mut self, boxes: &[Bounds]) {
        for i in (0..self.nodes.len()).rev() {
            self.refit_node(i, boxes);
        }
    }

    /// Marks a node and its ancestors for the next refit.
    fn mark(&mut self, node: u32) {
        let mut i = node as usize;
        while !self.marked[i] {
            self.marked[i] = true;
            if i == 0 {
                break;
            }
            i = self.parents[i] as usize;
        }
    }

    fn refit_marked(&mut self, boxes: &[Bounds]) {
        for i in (0..self.nodes.len()).rev() {
            if self.marked[i] {
                self.marked[i] = false;
                self.refit_node(i, boxes);
            }
        }
    }

    fn overlap(&self, q: &Query, boxes: &[Bounds], visit: &mut dyn FnMut(u32)) {
        let mut stack = vec![0usize];
        while let Some(mut node) = stack.pop() {
            while q.meets_box(&self.nodes[node].bounds) {
                let n = &self.nodes[node];
                if n.leaf {
                    let prims = &self.indices[n.first as usize..(n.first + n.count) as usize];
                    for &p in prims {
                        if prims.len() > 1 && !arms::on("no-leaf-box-test") && !q.meets_box(&boxes[p as usize]) {
                            continue;
                        }
                        visit(p);
                    }
                    break;
                }
                let (later, now) = if arms::on("neg-first") {
                    (n.first as usize, n.first as usize + 1)
                } else {
                    (n.first as usize + 1, n.first as usize)
                };
                stack.push(later);
                node = now;
            }
        }
    }
}

/// Pool index to leaf node of a main tree.
#[derive(Clone, Debug, Default, PartialEq)]
struct TreeMap(Vec<u32>);

impl TreeMap {
    fn init(size: usize, tree: &Tree) -> Self {
        let mut m = vec![INVALID; size];
        for (i, n) in tree.nodes.iter().enumerate() {
            if n.leaf {
                for &p in &tree.indices[n.first as usize..(n.first + n.count) as usize] {
                    m[p as usize] = i as u32;
                }
            }
        }
        Self(m)
    }

    fn get(&self, i: u32) -> u32 {
        self.0.get(i as usize).copied().unwrap_or(INVALID)
    }

    /// Takes the removed index out of its leaf (the leaf's last entry moves
    /// into its place) and renames the moved object's index.
    fn invalidate(&mut self, removed: u32, relocated: u32, tree: &mut Tree) {
        let n0 = self.get(removed);
        let n1 = self.get(relocated);
        if n0 != INVALID {
            let (base, count) = (
                tree.nodes[n0 as usize].first as usize,
                tree.nodes[n0 as usize].count as usize,
            );
            if let Some(k) = (0..count).find(|&k| tree.indices[base + k] == removed) {
                let last = count - 1;
                tree.nodes[n0 as usize].count = last as u32;
                tree.indices[base + k] = INVALID;
                self.0[removed as usize] = INVALID;
                if last != k {
                    if arms::on("leaf-remove-shift") {
                        tree.indices[base + k..=base + last].rotate_left(1);
                    } else {
                        tree.indices.swap(base + k, base + last);
                    }
                }
            }
        }
        if n1 != INVALID && removed != relocated {
            let (base, count) = (
                tree.nodes[n1 as usize].first as usize,
                tree.nodes[n1 as usize].count as usize,
            );
            if let Some(k) = (0..count).find(|&k| tree.indices[base + k] == relocated) {
                tree.indices[base + k] = removed;
                self.0[removed as usize] = n1;
                self.0[relocated as usize] = INVALID;
            }
        }
    }
}

// ---------------------------------------------------------------- bucket trees

#[derive(Clone, Debug, PartialEq)]
struct INode {
    min: [f32; 3],
    max: [f32; 3],
    parent: Option<usize>,
    /// The two children of an inner node; `None` for a leaf.
    children: Option<(usize, usize)>,
    indices: Vec<u32>,
}

/// An incremental tree: objects go in one by one, down the child whose box
/// centre is nearer, into a leaf that splits when full. An insert that
/// passes a node whose children differ in volume more than threefold moves
/// a leaf from the larger side to the smaller.
#[derive(Clone, Debug, Default, PartialEq)]
struct ITree {
    nodes: Vec<INode>,
    root: Option<usize>,
}

impl ITree {
    fn leaf(&self, n: usize) -> bool {
        self.nodes[n].children.is_none()
    }

    fn child(&self, n: usize, k: u32) -> usize {
        let (a, b) = self.nodes[n].children.expect("inner node");
        if k == 0 {
            a
        } else {
            b
        }
    }

    fn alloc(&mut self, n: INode) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    fn leaf_bounds(indices: &[u32], boxes: &[Bounds]) -> ([f32; 3], [f32; 3]) {
        let b0 = &boxes[indices[0] as usize];
        indices[1..].iter().fold((lo(b0), hi(b0)), |(mn, mx), &i| {
            let b = &boxes[i as usize];
            (min3(mn, lo(b)), max3(mx, hi(b)))
        })
    }

    fn refit_from_children(&mut self, n: usize) {
        let (a, b) = self.nodes[n].children.expect("inner node");
        self.nodes[n].min = min3(self.nodes[a].min, self.nodes[b].min);
        self.nodes[n].max = max3(self.nodes[a].max, self.nodes[b].max);
    }

    fn update_after_insert(&mut self, node: usize) {
        let mut test = node;
        let mut parent = self.nodes[node].parent;
        while let Some(p) = parent {
            let (t, pn) = (&self.nodes[test], &self.nodes[p]);
            if !(0..3).any(|k| pn.min[k] > t.min[k] || t.max[k] > pn.max[k]) {
                break;
            }
            self.refit_from_children(p);
            test = p;
            parent = self.nodes[p].parent;
        }
    }

    fn update_after_remove(&mut self, node: usize, boxes: &[Bounds]) {
        if self.leaf(node) {
            let (mn, mx) = Self::leaf_bounds(&self.nodes[node].indices, boxes);
            self.nodes[node].min = mn;
            self.nodes[node].max = mx;
        } else {
            self.refit_from_children(node);
        }
        let mut parent = self.nodes[node].parent;
        while let Some(p) = parent {
            let (a, b) = self.nodes[p].children.expect("inner node");
            let mn = min3(self.nodes[a].min, self.nodes[b].min);
            let mx = max3(self.nodes[a].max, self.nodes[b].max);
            if mn == self.nodes[p].min && mx == self.nodes[p].max {
                break;
            }
            self.nodes[p].min = mn;
            self.nodes[p].max = mx;
            parent = self.nodes[p].parent;
        }
    }

    fn add_into(&mut self, node: usize, index: u32, mn: [f32; 3], mx: [f32; 3]) {
        self.nodes[node].indices.push(index);
        self.nodes[node].min = min3(self.nodes[node].min, mn);
        self.nodes[node].max = max3(self.nodes[node].max, mx);
        self.update_after_insert(node);
    }

    fn split(&mut self, node: usize, index: u32, mn: [f32; 3], mx: [f32; 3], boxes: &[Bounds]) {
        let new_min = min3(self.nodes[node].min, mn);
        let new_max = max3(self.nodes[node].max, mx);
        let centre = add3(new_max, new_min).map(|v| v * 0.5);
        let axis = largest_axis(sub3(new_max, new_min));
        let side = |i: u32| {
            let b = &boxes[i as usize];
            let mid = (b[axis] + b[axis + 3]) * 0.5;
            if arms::on("bucket-split-gt") {
                centre[axis] > mid
            } else {
                centre[axis] >= mid
            }
        };
        // from the back: an object on the lower side moves to the second
        // leaf and the first leaf's last object takes its slot
        let mut c0 = std::mem::take(&mut self.nodes[node].indices);
        let mut c1 = Vec::new();
        let mut n0 = c0.len();
        for i in (0..c0.len()).rev() {
            if side(c0[i]) {
                c1.push(c0[i]);
                n0 -= 1;
                c0[i] = c0[n0];
            }
        }
        c0.truncate(n0);
        if c0.is_empty() || c1.len() == LEAF_LIMIT {
            c0 = vec![index];
        } else if c0.len() == LEAF_LIMIT {
            c1 = vec![index];
        } else if side(index) {
            c1.push(index);
        } else {
            c0.push(index);
        }
        let (mn0, mx0) = Self::leaf_bounds(&c0, boxes);
        let (mn1, mx1) = Self::leaf_bounds(&c1, boxes);
        let a = self.alloc(INode {
            min: mn0,
            max: mx0,
            parent: Some(node),
            children: None,
            indices: c0,
        });
        let b = self.alloc(INode {
            min: mn1,
            max: mx1,
            parent: Some(node),
            children: None,
            indices: c1,
        });
        self.nodes[node].children = Some((a, b));
        self.nodes[node].min = new_min;
        self.nodes[node].max = new_max;
        self.update_after_insert(node);
    }

    /// The child nearer the test centre (0 or 1). When rotation is tested,
    /// sets `rotate` and `largest` if one child's volume exceeds three times
    /// the other's; neither is cleared here.
    fn direction(
        &self,
        n: usize,
        test: [f32; 3],
        test_rotation: bool,
        rotate: &mut bool,
        largest: &mut u32,
    ) -> u32 {
        let (a, b) = self.nodes[n].children.expect("inner node");
        let (a, b) = (&self.nodes[a], &self.nodes[b]);
        let da = sub3(test, add3(a.max, a.min));
        let db = sub3(test, add3(b.max, b.min));
        if test_rotation {
            let sa = sub3(a.max, a.min);
            let sb = sub3(b.max, b.min);
            let va = sa[0] * sa[1] * sa[2];
            let vb = sb[0] * sb[1] * sb[2];
            let ratio = if arms::on("rotation-ratio-2") { 2.0 } else { 3.0 };
            if va * ratio < vb || vb * ratio < va {
                *largest = if va > vb { 0 } else { 1 };
                *rotate = true;
            }
        }
        let dot = |d: [f32; 3]| (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
        if dot(da) > dot(db) || (arms::on("bucket-nearer-tie") && dot(da) >= dot(db)) {
            1
        } else {
            0
        }
    }

    /// Walks down from the inner node `start` toward `test` to a leaf; with
    /// rotation tested, also the first node found unbalanced toward an inner
    /// child. The flag and the larger side carry from node to node.
    fn descend(
        &self,
        start: usize,
        test: [f32; 3],
        rotate_test: bool,
        largest: &mut u32,
    ) -> (usize, Option<usize>) {
        let (mut rotation_node, mut rotate, mut test_rotation, mut node) =
            (None, false, rotate_test, start);
        while !self.leaf(node) {
            let t = self.direction(node, test, test_rotation, &mut rotate, largest);
            if rotation_node.is_none() && rotate && !self.leaf(self.child(node, *largest)) {
                rotation_node = Some(node);
                test_rotation = false;
            }
            node = self.child(node, t);
        }
        (node, rotation_node)
    }

    fn insert(&mut self, index: u32, boxes: &[Bounds]) {
        let b = &boxes[index as usize];
        let (mn, mx) = (lo(b), hi(b));
        let Some(root) = self.root else {
            let r = self.alloc(INode {
                min: mn,
                max: mx,
                parent: None,
                children: None,
                indices: vec![index],
            });
            self.root = Some(r);
            return;
        };
        let mut largest = 0u32;
        let (base, rotation_node) = if self.leaf(root) {
            (root, None)
        } else {
            self.descend(root, add3(mx, mn), !arms::on("no-rotation"), &mut largest)
        };
        if self.nodes[base].indices.len() < LEAF_LIMIT {
            self.add_into(base, index, mn, mx);
        } else {
            self.split(base, index, mn, mx, boxes);
        }
        if let Some(r) = rotation_node {
            self.rotate(r, largest, boxes, true);
        }
    }

    /// Moves the leaf of the larger child nearest the smaller child into the
    /// smaller child's nearest leaf: merged, or paired under it when the two
    /// hold more than a leaf's worth.
    fn rotate(&mut self, node: usize, largest_in: u32, boxes: &[Bounds], rotate_again: bool) {
        let smaller = self.child(node, if largest_in == 0 { 1 } else { 0 });
        let larger = self.child(node, largest_in);
        let test = add3(self.nodes[smaller].max, self.nodes[smaller].min);
        let (closest, _) = self.descend(larger, test, false, &mut 0);
        let parent = self.nodes[closest]
            .parent
            .expect("a leaf below the larger child");
        let (a, b) = self.nodes[parent].children.expect("inner node");
        let remaining = self.nodes[if a == closest { b } else { a }].clone();
        self.nodes[parent].min = remaining.min;
        self.nodes[parent].max = remaining.max;
        match remaining.children {
            None => {
                self.nodes[parent].indices = remaining.indices;
                self.nodes[parent].children = None;
            }
            Some((g0, g1)) => {
                self.nodes[parent].children = Some((g0, g1));
                self.nodes[g0].parent = Some(parent);
                self.nodes[g1].parent = Some(parent);
            }
        }
        if let Some(pp) = self.nodes[parent].parent {
            self.update_after_remove(pp, boxes);
        }
        let moved = self.nodes[closest].clone();
        let mut largest = 0u32;
        let (spot, rotation_node) = if self.leaf(smaller) {
            (smaller, None)
        } else {
            self.descend(
                smaller,
                add3(moved.max, moved.min),
                rotate_again,
                &mut largest,
            )
        };
        if self.nodes[spot].indices.len() + moved.indices.len() <= LEAF_LIMIT {
            self.nodes[spot].indices.extend_from_slice(&moved.indices);
            self.nodes[spot].min = min3(self.nodes[spot].min, moved.min);
            self.nodes[spot].max = max3(self.nodes[spot].max, moved.max);
        } else {
            let s = self.nodes[spot].clone();
            let a = self.alloc(INode {
                min: s.min,
                max: s.max,
                parent: Some(spot),
                children: None,
                indices: s.indices,
            });
            let b = self.alloc(INode {
                min: moved.min,
                max: moved.max,
                parent: Some(spot),
                children: None,
                indices: moved.indices,
            });
            self.nodes[spot].indices = Vec::new();
            self.nodes[spot].children = Some((a, b));
            self.refit_from_children(spot);
        }
        self.update_after_insert(spot);
        if let Some(r) = rotation_node {
            self.rotate(r, largest, boxes, false);
        }
    }

    /// Removes `index` from leaf `node`.
    fn remove(&mut self, node: usize, index: u32, boxes: &[Bounds]) {
        if self.nodes[node].indices.len() > 1 {
            let ind = &mut self.nodes[node].indices;
            if let Some(i) = ind.iter().rposition(|&v| v == index) {
                ind.swap_remove(i);
            }
            self.update_after_remove(node, boxes);
            return;
        }
        if Some(node) == self.root {
            *self = Self::default();
            return;
        }
        let parent = self.nodes[node].parent.expect("a leaf below the root");
        let (a, b) = self.nodes[parent].children.expect("inner node");
        let remaining = self.nodes[if a == node { b } else { a }].clone();
        self.nodes[parent].min = remaining.min;
        self.nodes[parent].max = remaining.max;
        match remaining.children {
            None => {
                self.nodes[parent].indices = remaining.indices;
                self.nodes[parent].children = None;
            }
            Some((g0, g1)) => {
                self.nodes[parent].children = Some((g0, g1));
                self.nodes[g0].parent = Some(parent);
                self.nodes[g1].parent = Some(parent);
            }
        }
        if let Some(pp) = self.nodes[parent].parent {
            self.update_after_remove(pp, boxes);
        }
    }

    fn rename(&mut self, node: usize, index: u32, new_index: u32) {
        if let Some(slot) = self.nodes[node].indices.iter_mut().find(|v| **v == index) {
            *slot = new_index;
        }
    }

    /// The leaf of every index, walked from the root.
    fn leaves(&self) -> HashMap<u32, usize> {
        let mut m = HashMap::new();
        let mut stack: Vec<usize> = self.root.into_iter().collect();
        while let Some(n) = stack.pop() {
            match self.nodes[n].children {
                None => m.extend(self.nodes[n].indices.iter().map(|&i| (i, n))),
                Some((a, b)) => stack.extend([a, b]),
            }
        }
        m
    }

    fn overlap(&self, q: &Query, boxes: &[Bounds], visit: &mut dyn FnMut(u32)) {
        let Some(root) = self.root else { return };
        let mut stack = vec![root];
        while let Some(mut node) = stack.pop() {
            while q.meets(self.nodes[node].min, self.nodes[node].max) {
                let n = &self.nodes[node];
                match n.children {
                    None => {
                        for &p in &n.indices {
                            if n.indices.len() > 1 && !q.meets_box(&boxes[p as usize]) {
                                continue;
                            }
                            visit(p);
                        }
                        break;
                    }
                    Some((a, b)) => {
                        let (later, now) = if arms::on("bucket-neg-first") { (a, b) } else { (b, a) };
                        stack.push(later);
                        node = now;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CoreTree {
    tree: ITree,
    mapping: HashMap<u32, usize>,
}

impl CoreTree {
    fn remap(&mut self) {
        self.mapping = self.tree.leaves();
    }
}

/// The bucket: two incremental trees, the one receiving adds and the last.
#[derive(Clone, Debug, PartialEq)]
struct Bucket {
    trees: [CoreTree; 2],
    current: usize,
    last: usize,
}

impl Default for Bucket {
    fn default() -> Self {
        Self {
            trees: [CoreTree::default(), CoreTree::default()],
            current: 1,
            last: 0,
        }
    }
}

impl Bucket {
    fn count(&self) -> usize {
        self.trees[0].mapping.len() + self.trees[1].mapping.len()
    }

    fn add(&mut self, index: u32, boxes: &[Bounds]) {
        let t = &mut self.trees[self.current];
        t.tree.insert(index, boxes);
        t.remap();
    }

    /// Removes a bucket object (the last tree is searched first); the moved
    /// object's index is renamed.
    fn remove(&mut self, index: u32, relocated: u32, boxes: &[Bounds]) {
        let ti = if self.trees[self.last].mapping.contains_key(&index) {
            self.last
        } else {
            self.current
        };
        let t = &mut self.trees[ti];
        if let Some(&node) = t.mapping.get(&index) {
            t.tree.remove(node, index, boxes);
            t.remap();
        }
        if index != relocated {
            self.rename(index, relocated);
        }
    }

    /// The pool moved the object at `relocated` to `index` (the current
    /// tree is searched first).
    fn rename(&mut self, index: u32, relocated: u32) {
        let ti = if self.trees[self.current].mapping.contains_key(&relocated) {
            self.current
        } else {
            self.last
        };
        let t = &mut self.trees[ti];
        if let Some(node) = t.mapping.remove(&relocated) {
            t.mapping.insert(index, node);
            t.tree.rename(node, relocated, index);
        }
    }

    fn drop_last(&mut self) {
        self.trees[self.last] = CoreTree::default();
    }

    fn swap_trees(&mut self) {
        self.last = (self.last + 1) % 2;
        self.current = (self.current + 1) % 2;
    }

    fn overlap(&self, q: &Query, boxes: &[Bounds], visit: &mut dyn FnMut(u32)) {
        for t in &self.trees {
            t.tree.overlap(q, boxes, visit);
        }
    }
}

// ---------------------------------------------------------------- pruner

/// The rebuild in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    NotStarted = 0,
    Init = 1,
    InProgress = 2,
    NewMapping = 3,
    FullRefit = 4,
    LastFrame = 5,
    Finished = 6,
}

/// The static pruner.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticPruner {
    pool: Pool,
    handles: HashMap<u64, u32>,
    tree: Option<Tree>,
    tree_map: TreeMap,
    /// The rebuild: the build while it runs, the rebuilt tree after.
    building: Option<Build>,
    built: Option<Tree>,
    bucket: Bucket,
    progress: Progress,
    hint: u32,
    adaptive: i32,
    nb_calls: u32,
    total_work_units: u32,
    nb_cached: usize,
    uncommitted: bool,
    needs_new_tree: bool,
    fixups: Vec<(u32, u32)>,
    unmodeled: Option<Unmodeled>,
}

impl Default for StaticPruner {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticPruner {
    pub fn new() -> Self {
        Self {
            pool: Pool::default(),
            handles: HashMap::new(),
            tree: None,
            tree_map: TreeMap::default(),
            building: None,
            built: None,
            bucket: Bucket::default(),
            progress: Progress::NotStarted,
            hint: if arms::on("hint-not-less-3") { REBUILD_RATE_HINT } else { REBUILD_RATE_HINT - 3 },
            adaptive: 0,
            nb_calls: 0,
            total_work_units: 0,
            nb_cached: 0,
            uncommitted: false,
            needs_new_tree: false,
            fixups: Vec::new(),
            unmodeled: None,
        }
    }

    fn refuse(&mut self, why: &'static str) {
        self.unmodeled.get_or_insert(Unmodeled(why));
    }

    /// Adds a static shape with its pruner box (the shape's world bounds
    /// grown by 1.01); returns its pruner handle.
    pub fn add(&mut self, id: u64, bounds: Bounds) -> Option<u32> {
        if !bounds.iter().all(|v| v.is_finite()) {
            self.refuse("a non-finite pruner box");
            return None;
        }
        if self.handles.contains_key(&id) {
            self.refuse("an object added twice");
            return None;
        }
        self.uncommitted = true;
        let handle = self.pool.add(id, bounds);
        self.handles.insert(id, handle);
        if self.tree.is_some() {
            self.needs_new_tree = true;
            let index = self.pool.handle_to_index[handle as usize];
            self.bucket.add(index, &self.pool.boxes);
        }
        Some(handle)
    }

    /// Removes a static shape.
    pub fn remove(&mut self, id: u64) {
        let Some(handle) = self.handles.remove(&id) else {
            self.refuse("the removal of an object the pruner does not hold");
            return;
        };
        self.uncommitted = true;
        let (index, relocated) = self.pool.remove(handle);
        let rebuilding = self.building.is_some() || self.built.is_some();
        if let Some(tree) = self.tree.as_mut() {
            self.needs_new_tree = true;
            let node = self.tree_map.get(index);
            if node != INVALID {
                tree.mark(node);
                if index != relocated {
                    self.bucket.rename(index, relocated);
                }
            } else {
                self.bucket.remove(index, relocated, &self.pool.boxes);
            }
            self.tree_map.invalidate(index, relocated, tree);
            if rebuilding {
                if !arms::on("no-rebuild-fixups") {
                    self.fixups.push((index, relocated));
                }
            }
        }
        if self.pool.count == 0 {
            self.release();
            self.uncommitted = true;
        }
    }

    fn release(&mut self) {
        self.bucket = Bucket::default();
        self.tree_map = TreeMap::default();
        self.building = None;
        self.built = None;
        self.tree = None;
        self.nb_cached = 0;
        self.progress = Progress::NotStarted;
        self.fixups.clear();
        self.uncommitted = false;
    }

    /// The commit a query's flush and every physics step make.
    pub fn commit(&mut self) {
        if !self.uncommitted && self.progress != Progress::Finished {
            return;
        }
        self.uncommitted = false;
        if self.tree.is_none() {
            let n = self.pool.count;
            if n > 0 {
                let tree = Build::full(&self.pool.boxes[..n]);
                self.tree_map = TreeMap::init(n.max(self.nb_cached), &tree);
                self.tree = Some(tree);
            }
            return;
        }
        if self.progress != Progress::Finished {
            if self.pool.count > 0 {
                let boxes = &self.pool.boxes;
                self.tree.as_mut().expect("tree").refit_marked(boxes);
            }
            return;
        }
        self.progress = Progress::NotStarted;
        if self.nb_calls > self.hint {
            self.adaptive += 1;
        } else if self.nb_calls < self.hint {
            self.adaptive -= 1;
        }
        let mut tree = self.built.take().expect("finished rebuild");
        self.tree_map = TreeMap::init(self.pool.count.max(self.nb_cached), &tree);
        for (removed, relocated) in std::mem::take(&mut self.fixups) {
            let node = self.tree_map.get(removed);
            if node != INVALID {
                tree.mark(node);
            }
            self.tree_map.invalidate(removed, relocated, &mut tree);
        }
        if self.pool.count > 0 {
            tree.refit_marked(&self.pool.boxes);
        }
        self.tree = Some(tree);
        if !arms::on("no-bucket-drop") {
            self.bucket.drop_last();
        }
        self.needs_new_tree = self.bucket.count() > 0;
    }

    fn prepare_build(&mut self) -> bool {
        if !self.needs_new_tree {
            return false;
        }
        if self.progress == Progress::NotStarted {
            let n = self.pool.count;
            if n == 0 {
                return false;
            }
            self.nb_cached = n;
            self.building = Some(Build::new(&self.pool.boxes[..n]));
            self.built = None;
            if !arms::on("no-bucket-tree-swap") {
                self.bucket.swap_trees();
            }
            self.progress = Progress::Init;
        }
        true
    }

    /// One physics step: a rebuild step, then the commit. True when the
    /// rebuild finished on this step.
    pub fn step(&mut self) -> bool {
        let finished = self.build_step();
        self.commit();
        finished
    }

    fn build_step(&mut self) -> bool {
        if !self.needs_new_tree {
            return false;
        }
        match self.progress {
            Progress::NotStarted => {
                if !self.prepare_build() {
                    return false;
                }
            }
            Progress::Init => {
                self.building.as_mut().expect("build").queue.push_back(0);
                self.progress = Progress::InProgress;
                self.nb_calls = 0;
                // the work budget: the old tree's when within a factor two of
                // a balanced tree's, corrected by the adaptive term
                let n = self.nb_cached as u32;
                let depth = if n < 2 { 0 } else { 31 - n.leading_zeros() };
                let estimate = depth.wrapping_mul(n);
                let old = self.tree.as_ref().map_or(0, |t| t.total_prims);
                let base = if estimate <= old << 1 && estimate >= old >> 1 {
                    old
                } else {
                    self.adaptive = 0;
                    estimate
                };
                let total = if arms::on("no-adaptive-term") {
                    base as i32
                } else {
                    (base as i32).wrapping_add(self.adaptive.wrapping_mul(n as i32))
                };
                self.total_work_units = total.max(0) as u32;
            }
            Progress::InProgress => {
                self.nb_calls += 1;
                let limit = 1 + self.total_work_units / self.hint;
                if !self.building.as_mut().expect("build").progress(limit) {
                    self.built = self.building.take().map(Build::finish);
                    self.progress = Progress::NewMapping;
                }
            }
            Progress::NewMapping => {
                self.nb_calls += 1;
                self.progress = Progress::FullRefit;
                if !self.fixups.is_empty() {
                    let tree = self.built.as_mut().expect("rebuilt tree");
                    let mut map = TreeMap::init(self.pool.count.max(self.nb_cached), tree);
                    for (removed, relocated) in std::mem::take(&mut self.fixups) {
                        map.invalidate(removed, relocated, tree);
                    }
                }
            }
            Progress::FullRefit => {
                self.nb_calls += 1;
                self.progress = if arms::on("rebuild-one-step-short") { Progress::Finished } else { Progress::LastFrame };
                self.built
                    .as_mut()
                    .expect("rebuilt tree")
                    .full_refit(&self.pool.boxes);
            }
            Progress::LastFrame => self.progress = Progress::Finished,
            Progress::Finished => {}
        }
        self.uncommitted = true;
        self.progress == Progress::Finished
    }

    /// The shapes one box query meets, in the engine's visit order, or why
    /// the model cannot give it. Commits first, as the query's flush does.
    pub fn overlap(&mut self, centre: [f32; 3], half: [f32; 3]) -> Result<Vec<u64>, Unmodeled> {
        self.commit();
        if let Some(e) = self.unmodeled {
            return Err(e);
        }
        if !centre.iter().chain(&half).all(|v| v.is_finite()) {
            return Err(Unmodeled("a non-finite query box"));
        }
        let q = Query::new(centre, half);
        let mut out = Vec::new();
        let (objects, boxes) = (&self.pool.objects, &self.pool.boxes);
        let bucket_first = arms::on("bucket-before-tree");
        if bucket_first && self.bucket.count() > 0 {
            self.bucket
                .overlap(&q, boxes, &mut |p| out.push(objects[p as usize]));
        }
        if let Some(t) = &self.tree {
            t.overlap(&q, boxes, &mut |p| out.push(objects[p as usize]));
        }
        if !bucket_first && self.bucket.count() > 0 {
            self.bucket
                .overlap(&q, boxes, &mut |p| out.push(objects[p as usize]));
        }
        Ok(out)
    }

    /// The objects in pool order.
    pub fn pool_order(&self) -> &[u64] {
        &self.pool.objects[..self.pool.count]
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            has_tree: self.tree.is_some(),
            progress: self.progress as u32,
            hint: self.hint,
            adaptive: self.adaptive,
            uncommitted: self.uncommitted,
            needs_new_tree: self.needs_new_tree,
            pool_count: self.pool.count as u32,
            bucket: [
                self.bucket.trees[0].mapping.len() as u32,
                self.bucket.trees[1].mapping.len() as u32,
                0,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    //! Replay of the engine's own pruner code run on the ops of each case:
    //! every add's handle, every step's return, every query's visit order
    //! and the state words after every op. Each named variant must differ
    //! from the engine somewhere.
    use super::*;
    use crate::particle::json::{parse, Value};

    const ARMS: [&str; 21] = [
        "split-ge", "axis-tie-up", "neg-first", "bucket-before-tree", "leaf-remove-shift",
        "rebuild-one-step-short", "no-bucket-tree-swap", "no-bucket-drop", "progressive-lifo",
        "progressive-pos-first", "no-leaf-box-test", "hint-not-less-3", "no-adaptive-term",
        "query-not-inflated", "no-rebuild-fixups", "stale-box-cleared", "bucket-neg-first",
        "bucket-split-gt", "bucket-nearer-tie", "no-rotation", "rotation-ratio-2",
    ];

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn items(v: &Value) -> &[Value] {
        v.as_array().expect("row array")
    }
    fn int(v: &Value) -> i64 {
        let x = v.as_f64().expect("row number");
        assert!(x.fract() == 0.0 && x.abs() <= u32::MAX as f64, "row number {x}");
        x as i64
    }
    fn uint(v: &Value) -> u64 {
        let x = int(v);
        assert!(x >= 0, "row word {x}");
        x as u64
    }
    fn bits(v: &Value) -> f32 {
        f32::from_bits(uint(v) as u32)
    }
    fn vec3(v: &Value) -> [f32; 3] {
        let a = items(v);
        [bits(&a[0]), bits(&a[1]), bits(&a[2])]
    }
    fn native_state(s: &Value) -> Snapshot {
        let u = |k: &str| uint(field(s, k)) as u32;
        let b = items(field(s, "bucket"));
        Snapshot {
            has_tree: field(s, "tree").as_bool().expect("tree"),
            progress: u("progress"),
            hint: u("hint"),
            adaptive: int(field(s, "adaptive")) as i32,
            uncommitted: u("uncommitted") != 0,
            needs_new_tree: u("needsNewTree") != 0,
            pool_count: u("poolCount"),
            bucket: [0, 1, 2].map(|i| uint(&b[i]) as u32),
        }
    }
    /// The query's meeting test on a pool box (for the pool-order control).
    fn meets(b: &Bounds, c: [f32; 3], h: [f32; 3]) -> bool {
        (0..3).all(|k| {
            let g = h[k] * 1.01;
            let (qmin, qmax) = (c[k] - g, c[k] + g);
            let (bc, be) = ((b[k + 3] + b[k]) * 0.5, (b[k + 3] - b[k]) * 0.5);
            let (qc, qe) = ((qmax + qmin) * 0.5, (qmax - qmin) * 0.5);
            qe + be >= (bc - qc).abs()
        })
    }

    #[derive(Default, Debug)]
    struct Counts {
        cases: usize,
        ops: usize,
        adds: usize,
        steps: usize,
        finished: usize,
        queries: usize,
        visits: usize,
        multi: usize,
        bucket_queries: usize,
        pool_order_differs: usize,
        m_handle: usize,
        m_step: usize,
        m_visits: usize,
        m_state: usize,
        refused: usize,
    }

    impl Counts {
        fn mismatches(&self) -> usize {
            self.m_handle + self.m_step + self.m_visits + self.m_state + self.refused
        }
    }

    fn replay(rows: &[&Value], notes: &mut Vec<String>) -> Counts {
        let mut n = Counts::default();
        for row in rows {
            let name = field(row, "name").as_str().expect("name");
            let (ops, native) = (items(field(row, "ops")), items(field(row, "native")));
            assert_eq!(ops.len(), native.len(), "{name}: ops and records");
            n.cases += 1;
            let mut p = StaticPruner::new();
            let mut boxes = HashMap::new();
            for (i, (op, rec)) in ops.iter().zip(native).enumerate() {
                n.ops += 1;
                let mut note = |kind: &str, text: String| {
                    if notes.len() < 12 {
                        notes.push(format!("{name} op {i} {kind}: {text}"));
                    }
                };
                match field(op, "op").as_str().expect("op") {
                    "add" => {
                        let b = items(field(op, "bounds"));
                        let bounds: Bounds = std::array::from_fn(|k| bits(&b[k]));
                        let id = uint(field(op, "id"));
                        boxes.insert(id, bounds);
                        let got = p.add(id, bounds).map(u64::from);
                        n.adds += 1;
                        let want = rec.get("handle").and_then(Value::as_f64).map(|h| h as u64);
                        if got != want {
                            n.m_handle += 1;
                            note("handle", format!("model {got:?} native {want:?}"));
                        }
                    }
                    "remove" => p.remove(uint(field(op, "id"))),
                    "commit" => p.commit(),
                    "step" => {
                        let got = p.step();
                        let want = field(rec, "finished").as_bool().expect("finished");
                        n.steps += 1;
                        n.finished += usize::from(want);
                        if got != want {
                            n.m_step += 1;
                            note("step", format!("model {got} native {want}"));
                        }
                    }
                    "query" => {
                        let (c, h) = (vec3(field(op, "centre")), vec3(field(op, "half")));
                        let want: Vec<u64> = items(field(rec, "visits")).iter().map(uint).collect();
                        n.queries += 1;
                        n.visits += want.len();
                        n.multi += usize::from(want.len() > 1);
                        let s = native_state(field(rec, "state"));
                        n.bucket_queries += usize::from(s.bucket.iter().sum::<u32>() > 0);
                        let got = p.overlap(c, h);
                        let baseline: Vec<u64> =
                            p.pool_order().iter().copied().filter(|id| meets(&boxes[id], c, h)).collect();
                        n.pool_order_differs += usize::from(baseline != want);
                        match got {
                            Ok(got) if got == want => {}
                            Ok(got) => {
                                n.m_visits += 1;
                                note("visits", format!("model {got:?} native {want:?}"));
                            }
                            Err(e) => {
                                n.refused += 1;
                                note("refused", e.0.to_string());
                            }
                        }
                    }
                    other => panic!("{name}: op {other}"),
                }
                let want = native_state(field(rec, "state"));
                let got = p.snapshot();
                if got != want {
                    n.m_state += 1;
                    note("state", format!("model {got:?} native {want:?}"));
                }
            }
        }
        n
    }

    /// The engine's pruner rows (`MOLY_PRUNER_ROWS`, comma-separated files):
    /// random and lattice-tied boxes, pools past a rebuild's work budget,
    /// removals during a rebuild, crafted nearer-child ties in the bucket,
    /// and the site add and remove sequences. The port must equal every
    /// handle, step return, visit order and state word; each named variant
    /// must differ somewhere; the same hits in pool order must differ too.
    #[test]
    #[ignore = "needs MOLY_PRUNER_ROWS"]
    fn pruner_rows_match_native() {
        let paths = std::env::var("MOLY_PRUNER_ROWS").expect("MOLY_PRUNER_ROWS");
        let docs: Vec<Value> = paths
            .split(',')
            .map(|path| parse(&std::fs::read(path).expect("read rows")).expect("parse rows"))
            .collect();
        let rows: Vec<&Value> = docs.iter().flat_map(|d| items(field(d, "rows"))).collect();
        arms::set(None);
        let mut notes = Vec::new();
        let n = replay(&rows, &mut notes);
        for s in &notes {
            println!("{s}");
        }
        println!("pruner replay {n:?} mismatches {}", n.mismatches());
        let mut red = Vec::new();
        for arm in ARMS {
            arms::set(Some(arm));
            let m = replay(&rows, &mut Vec::new());
            red.push((arm, m.mismatches(), m.m_visits, m.m_state));
        }
        arms::set(None);
        println!("arms (mismatches, visit orders, state words): {red:?}");
        assert!(n.queries > 0 && n.multi > 0);
        assert_eq!(n.mismatches(), 0, "the model differs from the engine's pruner");
        assert!(n.pool_order_differs > 0, "pool order equals the engine's on every query");
        for (arm, m, _, _) in red {
            assert!(m > 0, "arm {arm} never differs from the engine");
        }
    }
}
