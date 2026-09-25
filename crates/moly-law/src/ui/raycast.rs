//! Pointer raycast selection: which graphic receives a pointer event.
//!
//! Per raycaster (one per canvas), a graphic is a candidate when it is a
//! raycast target, not culled, has been given a draw depth (depth -1 means
//! the canvas never drew it), contains the pointer in its padded rect, and
//! passes `Graphic.Raycast`'s walk up the hierarchy. Candidates are ordered
//! by draw depth, highest first. The event system then orders the results of
//! all raycasters with its comparer and the first result wins.
//!
//! The padded rect test is the engine's native one: the rect transform's rect
//! shrunk by the padding (left, bottom, right, top; negative values enlarge
//! it), its four corners taken into the event space, and a ray through the
//! pointer intersected with the two triangles (c0, c1, c2) and (c0, c2, c3),
//! the corners ordered min-min, min-max, max-max, max-min. The triangle test
//! is the engine's `IntersectRayTriangle` with its tolerances, so the hit
//! area reaches slightly past the exact edges.
//!
//! The engine casts the event camera's screen-point ray (`screen_ray`)
//! against the corners taken into world space; `quad_hit_by_ray` is that
//! test. `quad_contains` is the same test in the canvas plane (z = 0) for a
//! point already in canvas units: a ray from one unit in front of the plane
//! along +z through the point, which is not the engine's world-space
//! arithmetic.

use super::image::Rect;
use std::cmp::Ordering;

/// The padded local rect's corners in the engine's order.
pub fn padded_local_quad(rect: Rect, padding: [f32; 4]) -> [[f32; 2]; 4] {
    let x_min = rect.x + padding[0];
    let y_min = rect.y + padding[1];
    let x_max = (rect.x + rect.width) - padding[2];
    let y_max = (rect.y + rect.height) - padding[3];
    [[x_min, y_min], [x_min, y_max], [x_max, y_max], [x_max, y_min]]
}

/// The engine's smallest accepted |determinant| (float bits 0x358637bd).
const RAY_TRIANGLE_DETERMINANT_EPSILON: f32 = 1e-6;
/// The engine's lower bound for the barycentric u, v and the ray distance
/// (float bits 0xb48fe1a4).
const RAY_TRIANGLE_BARYCENTRIC_FLOOR: f32 = -2.68e-7;

/// The engine's `IntersectRayTriangle(ray, a, b, c, out t)` (Moller-Trumbore)
/// in its operation order: the distance along the ray, or None.
///
/// `pvec = dir x e2`, `det = e1 . pvec` (rejected when |det| < 1e-6; NaN
/// passes), `u = (tvec . pvec) / det`, `qvec = tvec x e1`,
/// `v = (dir . qvec) / det`, `t = (e2 . qvec) / det`, each division a
/// multiplication by `1 / det`. Rejected when u < -2.68e-7, u > 1,
/// v < -2.68e-7, u + v > 1 or t < -2.68e-7.
pub fn intersect_ray_triangle(
    origin: [f32; 3],
    direction: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) -> Option<f32> {
    let [dx, dy, dz] = direction;
    let (e1x, e1y, e1z) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (e2x, e2y, e2z) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    let px = dy * e2z - e2y * dz;
    let py = e2x * dz - e2z * dx;
    let pz = e2y * dx - e2x * dy;
    let det = e1z * pz + (e1x * px + e1y * py);
    let magnitude = if det < 0.0 { -det } else { det };
    if magnitude < RAY_TRIANGLE_DETERMINANT_EPSILON {
        return None;
    }
    let (tx, ty, tz) = (origin[0] - a[0], origin[1] - a[1], origin[2] - a[2]);
    let inverse = 1.0 / det;
    let u = inverse * ((px * tx + py * ty) + pz * tz);
    if u < RAY_TRIANGLE_BARYCENTRIC_FLOOR || u > 1.0 {
        return None;
    }
    let qx = e1z * ty - e1y * tz;
    let qy = e1x * tz - e1z * tx;
    let qz = e1y * tx - e1x * ty;
    let v = inverse * (dz * qz + (dx * qx + dy * qy));
    if v < RAY_TRIANGLE_BARYCENTRIC_FLOOR || u + v > 1.0 {
        return None;
    }
    let t = inverse * (e2z * qz + (e2x * qx + e2y * qy));
    if t < RAY_TRIANGLE_BARYCENTRIC_FLOOR {
        return None;
    }
    Some(t)
}

/// The native `PointInRectangle` after its screen-point ray: the ray
/// against the triangles (c0, c1, c2), then (c0, c2, c3) of the padded
/// corners in world space.
pub fn quad_hit_by_ray(corners: [[f32; 3]; 4], origin: [f32; 3], direction: [f32; 3]) -> bool {
    let [c0, c1, c2, c3] = corners;
    intersect_ray_triangle(origin, direction, c0, c1, c2).is_some()
        || intersect_ray_triangle(origin, direction, c0, c2, c3).is_some()
}

/// `RectTransformUtility.RectangleContainsScreenPoint(rect, point, cam, padding)`
/// given the padded corners and the point in the canvas plane.
pub fn quad_contains(corners: [[f32; 2]; 4], point: [f32; 2]) -> bool {
    quad_hit_by_ray(corners.map(|c| [c[0], c[1], 0.0]), [point[0], point[1], -1.0], [0.0, 0.0, 1.0])
}

/// One component on one transform, as `Graphic.Raycast` sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LevelComponent {
    /// A Canvas: one with override sorting ends the walk after this transform.
    Canvas { override_sorting: bool },
    /// A CanvasGroup (its raycast filter answers `blocksRaycasts`).
    CanvasGroup { enabled: bool, blocks_raycasts: bool, ignore_parent_groups: bool },
    /// Any other raycast filter with its answer for this pointer.
    Filter { valid: bool },
}

/// `Graphic.Raycast(sp, eventCamera)`. `levels` lists the components of the
/// graphic's own transform first, then each parent's, in component order.
pub fn graphic_raycast(active_and_enabled: bool, levels: &[Vec<LevelComponent>]) -> bool {
    if !active_and_enabled {
        return false;
    }
    let mut ignore_parent_groups = false;
    for components in levels {
        let mut continue_traversal = true;
        for component in components {
            let valid = match *component {
                LevelComponent::Canvas { override_sorting } => {
                    if override_sorting {
                        continue_traversal = false;
                    }
                    continue;
                }
                LevelComponent::CanvasGroup { enabled, blocks_raycasts, ignore_parent_groups: ignores } => {
                    if !enabled {
                        continue;
                    }
                    if !ignore_parent_groups && ignores {
                        ignore_parent_groups = true;
                        blocks_raycasts
                    } else if !ignore_parent_groups {
                        blocks_raycasts
                    } else {
                        true
                    }
                }
                LevelComponent::Filter { valid } => valid,
            };
            if !valid {
                return false;
            }
        }
        if !continue_traversal {
            break;
        }
    }
    true
}

/// CanvasGroup flags `Selectable.ParentGroupAllowsInteraction` reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupInteraction {
    pub enabled: bool,
    pub interactable: bool,
    pub ignore_parent_groups: bool,
}

/// `Selectable.ParentGroupAllowsInteraction`: own transform first, then parents.
pub fn parent_group_allows_interaction(levels: &[Vec<GroupInteraction>]) -> bool {
    for groups in levels {
        for group in groups {
            if group.enabled && !group.interactable {
                return false;
            }
            if group.ignore_parent_groups {
                return true;
            }
        }
    }
    true
}

/// `Selectable.IsInteractable`.
pub fn is_interactable(interactable: bool, groups_allow_interaction: bool) -> bool {
    groups_allow_interaction && interactable
}

/// One graphic registered to a raycaster's canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphicCandidate {
    pub raycast_target: bool,
    /// `canvasRenderer.cull`.
    pub culled: bool,
    /// `canvasRenderer.absoluteDepth`; -1 when the canvas has not drawn it.
    pub depth: i32,
    /// The padded rect test for this pointer.
    pub contains_point: bool,
    /// `Graphic.Raycast` for this pointer.
    pub raycast: bool,
}

/// The candidates one `GraphicRaycaster` returns, highest depth first.
#[derive(Debug, Clone, PartialEq)]
pub struct RaycasterHits {
    /// Indices into the candidate list.
    pub order: Vec<usize>,
    /// True when two hits share a depth: the engine sorts with an unstable
    /// sort, so their relative order is not defined by the source.
    pub depth_tie: bool,
}

/// The static `GraphicRaycaster.Raycast` filter and depth sort.
pub fn raycaster_hits(candidates: &[GraphicCandidate]) -> RaycasterHits {
    let mut order: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, g)| g.raycast_target && !g.culled && g.depth != -1)
        .filter(|(_, g)| g.contains_point && g.raycast)
        .map(|(i, _)| i)
        .collect();
    order.sort_by(|a, b| candidates[*b].depth.cmp(&candidates[*a].depth));
    let depth_tie = order.windows(2).any(|w| candidates[w[0]].depth == candidates[w[1]].depth);
    RaycasterHits { order, depth_tie }
}

/// The fields of a `RaycastResult` the event system's comparer reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResultKey {
    /// Identity of the raycaster module.
    pub module: usize,
    /// The module's event camera depth (None without a camera).
    pub camera_depth: Option<f32>,
    pub sort_order_priority: i32,
    pub render_order_priority: i32,
    /// `SortingLayer.GetLayerValueFromID(sortingLayer)`.
    pub sorting_layer_value: i32,
    pub sorting_order: i32,
    pub depth: i32,
    /// Identity of the module's root raycaster.
    pub root_raycaster: usize,
    pub distance: f32,
    pub index: usize,
}

/// `EventSystem.RaycastComparer`.
pub fn compare(lhs: &ResultKey, rhs: &ResultKey) -> Ordering {
    if lhs.module != rhs.module {
        if let (Some(l), Some(r)) = (lhs.camera_depth, rhs.camera_depth) {
            if l != r {
                return if l < r { Ordering::Greater } else { Ordering::Less };
            }
        }
        if lhs.sort_order_priority != rhs.sort_order_priority {
            return rhs.sort_order_priority.cmp(&lhs.sort_order_priority);
        }
        if lhs.render_order_priority != rhs.render_order_priority {
            return rhs.render_order_priority.cmp(&lhs.render_order_priority);
        }
    }
    if lhs.sorting_layer_value != rhs.sorting_layer_value {
        return rhs.sorting_layer_value.cmp(&lhs.sorting_layer_value);
    }
    if lhs.sorting_order != rhs.sorting_order {
        return rhs.sorting_order.cmp(&lhs.sorting_order);
    }
    if lhs.depth != rhs.depth && lhs.root_raycaster == rhs.root_raycaster {
        return rhs.depth.cmp(&lhs.depth);
    }
    if lhs.distance != rhs.distance {
        return lhs.distance.partial_cmp(&rhs.distance).unwrap_or(Ordering::Equal);
    }
    lhs.index.cmp(&rhs.index)
}

/// `EventSystem.RaycastAll` ordering: the winner is the first key.
pub fn winner(results: &[ResultKey]) -> Option<usize> {
    let mut order: Vec<usize> = (0..results.len()).collect();
    order.sort_by(|a, b| compare(&results[*a], &results[*b]));
    order.first().copied()
}
