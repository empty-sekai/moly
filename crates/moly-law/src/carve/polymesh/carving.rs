//! Runtime carving of the navigation cells: the engine's
//! `DynamicMesh::ClipPolys` step of `CarveNavMeshTile`, on this mesh.
//!
//! For every cell a hull's bounds touch, the part of the cell inside the hull
//! is removed. The hull's planes are applied in order with
//! `DynamicMesh::SplitPoly` (weld tolerance voxel / 64, the `CarveNavMeshTile`
//! argument): a cell entirely in front of one plane is not touched; otherwise
//! what lies in front of each plane is kept and the remainder (the cell's
//! intersection with the hull) is dropped. The kept boundary therefore runs
//! through the same crossing points the engine's intersection polygon has.
//!
//! Decomposition (named): the engine rebuilds the kept part with
//! `DynamicMesh::Subtract` and `MergePolygons` into convex polygons; this mesh
//! stays a triangle mesh (it has no polygon merge, see the module notes), so
//! each kept convex piece is fanned from its centroid. Cells that share an
//! edge with a carved cell get the carve's points on that edge too, so the
//! shared edge still pairs vertex for vertex. The polygons' height is the
//! carve's floor sample (this mesh has no detail height).

use super::super::grid::Grid;
use super::super::obstacle::{crossing, plane_distance, Plane};
use super::{connected_components, PolyMesh, CARVED_VERTEX, NO_NEIGHBOUR};
use std::collections::HashMap;

/// One carve in the moly frame: its hull's planes, its x/z bounds, and the
/// height the navigation polygons are tested at.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCarve {
    pub planes: Vec<Plane>,
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub floor: f32,
}

/// A piece vertex: its position, the base vertex it is, or the base edge it
/// lies on.
#[derive(Clone, Copy, Debug)]
struct Pv {
    p: [f32; 3],
    base: Option<u32>,
    edge: Option<(u32, u32)>,
}

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// The base edge of `tri` both vertices lie on.
fn common_edge(a: &Pv, b: &Pv, tri: [u32; 3]) -> Option<(u32, u32)> {
    let edges = |v: &Pv| -> Vec<(u32, u32)> {
        if let Some(id) = v.base {
            tri.iter()
                .filter(|other| **other != id)
                .map(|other| key(id, *other))
                .collect()
        } else {
            v.edge.into_iter().collect()
        }
    };
    let ea = edges(a);
    edges(b).into_iter().find(|e| ea.contains(e))
}

enum Cut {
    Inside,
    Outside,
    Both { inside: Vec<Pv>, outside: Vec<Pv> },
}

/// `obstacle::split_poly` carrying the vertex tags.
fn split(poly: &[Pv], plane: &Plane, eps: f32, tri: [u32; 3]) -> Cut {
    let d: Vec<f32> = poly
        .iter()
        .map(|v| plane_distance(plane, v.p, eps))
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
        return Cut::Inside;
    }
    if low > 0.0 {
        return Cut::Outside;
    }
    let n = poly.len();
    let (mut inside, mut outside) = (Vec::with_capacity(n + 2), Vec::with_capacity(n + 2));
    let mut prev = poly[n - 1];
    let mut dp = d[n - 1];
    for i in 0..n {
        let cur = poly[i];
        let dc = d[i];
        if let Some(p) = crossing(prev.p, dp, cur.p, dc) {
            let v = Pv {
                p,
                base: None,
                edge: common_edge(&prev, &cur, tri),
            };
            inside.push(v);
            outside.push(v);
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
    Cut::Both { inside, outside }
}

/// The pieces of `piece` outside the carve, or `None` when the piece does not
/// meet the hull (some plane has it entirely in front).
fn clip(piece: &[Pv], tri: [u32; 3], carve: &RuntimeCarve, eps: f32) -> Option<Vec<Vec<Pv>>> {
    let mut rest: Vec<Pv> = piece
        .iter()
        .map(|v| Pv {
            p: [v.p[0], carve.floor, v.p[2]],
            ..*v
        })
        .collect();
    let mut kept = Vec::new();
    for plane in &carve.planes {
        match split(&rest, plane, eps, tri) {
            Cut::Inside => {}
            Cut::Outside => return None,
            Cut::Both { inside, outside } => {
                kept.push(outside);
                rest = inside;
            }
        }
    }
    Some(kept)
}

/// Twice the signed x/z area.
fn area2(poly: &[Pv]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i].p, poly[(i + 1) % n].p);
            a[0] * b[2] - b[0] * a[2]
        })
        .sum()
}

/// A piece this small in area is dropped (`RemoveDegeneratePolygons`).
const DEGENERATE_AREA2: f32 = 1e-9;

impl PolyMesh {
    /// This (uncarved) mesh with `carves` applied. The world vertices must
    /// be cached for `grid`.
    pub(crate) fn carved(&self, grid: &Grid, carves: &[RuntimeCarve]) -> PolyMesh {
        let eps = grid.voxel * 0.015625;
        let n = self.tris.len();
        let world = |v: u32| self.vertex(grid, v);
        let corner = |v: u32| {
            let w = world(v);
            Pv {
                p: [w[0], 0.0, w[1]],
                base: Some(v),
                edge: None,
            }
        };
        let mut pieces: Vec<Option<Vec<Vec<Pv>>>> = vec![None; n];
        for cell in 0..n {
            let tri = self.tris[cell];
            let w = tri.map(world);
            let lo = [
                w[0][0].min(w[1][0]).min(w[2][0]),
                w[0][1].min(w[1][1]).min(w[2][1]),
            ];
            let hi = [
                w[0][0].max(w[1][0]).max(w[2][0]),
                w[0][1].max(w[1][1]).max(w[2][1]),
            ];
            let mut current = vec![tri.map(corner).to_vec()];
            let mut changed = false;
            for carve in carves {
                if carve.max[0] < lo[0]
                    || carve.min[0] > hi[0]
                    || carve.max[1] < lo[1]
                    || carve.min[1] > hi[1]
                {
                    continue;
                }
                let mut next = Vec::with_capacity(current.len());
                for piece in current {
                    match clip(&piece, tri, carve, eps) {
                        None => next.push(piece),
                        Some(kept) => {
                            changed = true;
                            next.extend(kept);
                        }
                    }
                }
                current = next;
            }
            if changed {
                pieces[cell] = Some(current);
            }
        }
        // The carve's points on every base edge, so both cells of a shared
        // edge carry them.
        let mut on_edge: HashMap<(u32, u32), Vec<[f32; 3]>> = HashMap::new();
        for cell_pieces in pieces.iter().flatten() {
            for piece in cell_pieces {
                for v in piece {
                    if let Some(e) = v.edge {
                        let list = on_edge.entry(e).or_default();
                        if !list.iter().any(|q| q[0] == v.p[0] && q[2] == v.p[2]) {
                            list.push(v.p);
                        }
                    }
                }
            }
        }
        if pieces.iter().all(Option::is_none) {
            return self.clone();
        }
        for cell in 0..n {
            let tri = self.tris[cell];
            let touches = (0..3).any(|k| on_edge.contains_key(&key(tri[k], tri[(k + 1) % 3])));
            if !touches {
                continue;
            }
            let mut cell_pieces = pieces[cell]
                .take()
                .unwrap_or_else(|| vec![tri.map(corner).to_vec()]);
            for piece in &mut cell_pieces {
                self.insert_edge_points(grid, piece, tri, &on_edge);
            }
            pieces[cell] = Some(cell_pieces);
        }
        self.assemble(grid, pieces)
    }

    /// Inserts into `piece` every point of `on_edge` lying on one of its
    /// edges that runs along a base edge, between that edge's ends.
    fn insert_edge_points(
        &self,
        grid: &Grid,
        piece: &mut Vec<Pv>,
        tri: [u32; 3],
        on_edge: &HashMap<(u32, u32), Vec<[f32; 3]>>,
    ) {
        let mut out = Vec::with_capacity(piece.len() + 4);
        let n = piece.len();
        for i in 0..n {
            let (u, w) = (piece[i], piece[(i + 1) % n]);
            out.push(u);
            let Some(e) = common_edge(&u, &w, tri) else {
                continue;
            };
            let Some(points) = on_edge.get(&e) else {
                continue;
            };
            let a = self.vertex(grid, e.0);
            let b = self.vertex(grid, e.1);
            let along = |p: [f32; 3]| (p[0] - a[0]) * (b[0] - a[0]) + (p[2] - a[1]) * (b[1] - a[1]);
            let (tu, tw) = (along(u.p), along(w.p));
            let same = |p: [f32; 3], q: [f32; 3]| p[0] == q[0] && p[2] == q[2];
            let mut between: Vec<(f32, [f32; 3])> = points
                .iter()
                .filter(|p| !same(**p, u.p) && !same(**p, w.p))
                .map(|p| (along(*p), *p))
                .filter(|(t, _)| (tu < *t && *t < tw) || (tw < *t && *t < tu))
                .collect();
            if tu < tw {
                between.sort_by(|x, y| x.0.total_cmp(&y.0));
            } else {
                between.sort_by(|x, y| y.0.total_cmp(&x.0));
            }
            out.extend(between.into_iter().map(|(_, p)| Pv {
                p,
                base: None,
                edge: Some(e),
            }));
        }
        *piece = out;
    }

    /// Builds the mesh: untouched cells as they were, every piece of a touched
    /// cell as one triangle (three vertices) or a fan around its centroid.
    fn assemble(&self, grid: &Grid, pieces: Vec<Option<Vec<Vec<Pv>>>>) -> PolyMesh {
        let mut verts = self.verts.clone();
        let mut world: Vec<[f32; 2]> = (0..self.verts.len() as u32)
            .map(|v| self.vertex(grid, v))
            .collect();
        let mut index: HashMap<(u32, u32), u32> = HashMap::new();
        for (i, w) in world.iter().enumerate() {
            index
                .entry((w[0].to_bits(), w[1].to_bits()))
                .or_insert(i as u32);
        }
        let mut id_of =
            |p: [f32; 2], verts: &mut Vec<[i32; 2]>, world: &mut Vec<[f32; 2]>| -> u32 {
                *index
                    .entry((p[0].to_bits(), p[1].to_bits()))
                    .or_insert_with(|| {
                        verts.push(CARVED_VERTEX);
                        world.push(p);
                        (world.len() - 1) as u32
                    })
            };
        let mut tris = Vec::with_capacity(self.tris.len());
        let mut cell_region = Vec::with_capacity(self.tris.len());
        for (cell, piece_list) in pieces.into_iter().enumerate() {
            let region = self.cell_region.get(cell).copied().unwrap_or(0);
            let Some(piece_list) = piece_list else {
                tris.push(self.tris[cell]);
                cell_region.push(region);
                continue;
            };
            for piece in piece_list {
                if piece.len() < 3 || area2(&piece).abs() <= DEGENERATE_AREA2 {
                    continue;
                }
                let ids: Vec<u32> = piece
                    .iter()
                    .map(|v| match v.base {
                        Some(base) => base,
                        None => id_of([v.p[0], v.p[2]], &mut verts, &mut world),
                    })
                    .collect();
                if ids.len() == 3 {
                    tris.push([ids[0], ids[1], ids[2]]);
                    cell_region.push(region);
                    continue;
                }
                let count = piece.len() as f32;
                let centre = [
                    piece.iter().map(|v| v.p[0]).sum::<f32>() / count,
                    piece.iter().map(|v| v.p[2]).sum::<f32>() / count,
                ];
                let c = id_of(centre, &mut verts, &mut world);
                for i in 0..ids.len() {
                    let (a, b) = (ids[i], ids[(i + 1) % ids.len()]);
                    if a == b || a == c || b == c {
                        continue;
                    }
                    tris.push([c, a, b]);
                    cell_region.push(region);
                }
            }
        }
        let mut edges: HashMap<(u32, u32), (u32, usize)> = HashMap::new();
        let mut neighbours = vec![[NO_NEIGHBOUR; 3]; tris.len()];
        for (i, t) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if let Some((other, other_k)) = edges.remove(&(b, a)) {
                    neighbours[i][k] = other;
                    neighbours[other as usize][other_k] = i as u32;
                } else {
                    edges.insert((a, b), (i as u32, k));
                }
            }
        }
        let mut by_region: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, region) in cell_region.iter().enumerate() {
            by_region.entry(*region).or_default().push(i as u32);
        }
        let components = connected_components(&neighbours);
        let mut mesh = PolyMesh {
            verts,
            world,
            cell_region,
            tris,
            neighbours,
            by_region,
            components,
            centres: Vec::new(),
            centre_grid: None,
        };
        mesh.cache_centres(grid);
        mesh
    }
}

/// Marks the walk cells whose centre lies inside a carve (every plane at most
/// zero, at the carve's floor) unwalkable; returns how many it changed.
pub(crate) fn carve_cells(grid: &mut Grid, carves: &[RuntimeCarve]) -> usize {
    let eps = grid.voxel * 0.015625;
    let mut changed = 0;
    for carve in carves {
        let (x0, z0) = grid.cell_of(carve.min[0], carve.min[1]);
        let (x1, z1) = grid.cell_of(carve.max[0], carve.max[1]);
        for cz in z0.max(0)..=z1.min(grid.rows as isize - 1) {
            for cx in x0.max(0)..=x1.min(grid.cols as isize - 1) {
                let i = cz as usize * grid.cols + cx as usize;
                if !grid.walkable[i] {
                    continue;
                }
                let c = grid.cell_center(cx as usize, cz as usize);
                let p = [c[0], carve.floor, c[1]];
                if carve
                    .planes
                    .iter()
                    .all(|plane| plane_distance(plane, p, eps) <= 0.0)
                {
                    grid.walkable[i] = false;
                    changed += 1;
                }
            }
        }
    }
    changed
}
