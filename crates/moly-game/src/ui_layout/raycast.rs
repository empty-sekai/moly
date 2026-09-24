//! Source raycast over a prefab view: which graphic receives a pointer and
//! which selectable handles it, by the engine rules in `moly_law::ui::raycast`.
//!
//! A pointer is either the event camera's screen-point ray in world space,
//! against which the padded corners are taken from the view's canvas units
//! into the world through the root canvas pose (the views sit at the root
//! canvas), or a point already in the view's canvas units, tested in the
//! canvas plane.
//!
//! Graphics are the components that serialize the Graphic fields (a
//! `m_RaycastTarget` flag); selectables are the components that serialize
//! the Selectable fields (`m_Transition` with `m_Interactable`). Each graphic
//! belongs to the raycaster of its nearest enabled Canvas; a nested Canvas
//! without a GraphicRaycaster is never raycast. Graphics directly under the
//! view belong to the host layer canvas.
//!
//! Two inputs are this host's stand-ins, not engine values: the draw depth is
//! the graphic's order within its canvas (the engine's batching depth also
//! lets non-overlapping graphics share a depth), and culling is taken as off
//! (only RectMask2D culls, and its raycast filter is applied here). The
//! sorting order of a nested Canvas without override sorting is its parent
//! canvas's. None of these three matters where only one candidate contains
//! the pointer.

use bevy::math::{Vec2, Vec3};
use moly_assets::ui_layout::{UiComponent, UiPrefab, UiRect};
use moly_law::ui::{image::Rect, raycast as rule, screen_ray};

/// A pointer as the raycast tests it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pointer {
    /// The event camera's ray in world space and the root canvas' world pose.
    Ray { ray: screen_ray::Ray, root: screen_ray::Pose },
    /// A point in the view's canvas units.
    Canvas(Vec2),
}
use serde_json::Value;

fn vec4(value: &Value) -> Option<[f32; 4]> {
    let items = value.as_array().filter(|a| a.len() == 4)?;
    let mut out = [0.0; 4];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64()? as f32;
    }
    Some(out)
}

/// A serialized field every component of its class carries.
fn field<'a>(component: &'a UiComponent, name: &str) -> &'a Value {
    component.fields.get(name).unwrap_or_else(|| {
        panic!("UI raycast: {} @{} lacks its serialized {name}", component.class, component.path_id)
    })
}

fn flag(component: &UiComponent, name: &str) -> bool {
    field(component, name).as_bool().unwrap_or_else(|| {
        panic!("UI raycast: {} @{} {name} is not a bool", component.class, component.path_id)
    })
}

fn padding(component: &UiComponent, name: &str) -> [f32; 4] {
    vec4(field(component, name)).unwrap_or_else(|| {
        panic!("UI raycast: {} @{} {name} is not four numbers", component.class, component.path_id)
    })
}

/// The padded rect test for a pointer.
pub(crate) fn padded_contains(rect: &UiRect, padding: [f32; 4], pointer: Pointer) -> bool {
    let local = Rect::from_size_pivot(rect.size.to_array(), rect.pivot.to_array());
    let quad = rule::padded_local_quad(local, padding);
    let canvas = quad.map(|c| rect.world.transform_point3(Vec3::new(c[0], c[1], 0.0)));
    match pointer {
        Pointer::Ray { ray, root } => {
            let corners = canvas.map(|c| root.transform_point(c.to_array()));
            rule::quad_hit_by_ray(corners, ray.origin, ray.direction)
        }
        Pointer::Canvas(point) => rule::quad_contains(canvas.map(|c| c.truncate().to_array()), point.to_array()),
    }
}

fn is_graphic(component: &UiComponent) -> bool {
    component.fields.get("m_RaycastTarget").is_some()
}

fn is_selectable(component: &UiComponent) -> bool {
    component.fields.get("m_Transition").is_some() && component.fields.get("m_Interactable").is_some()
}

fn enabled_canvas(doc: &UiPrefab, node: usize) -> Option<&UiComponent> {
    doc.nodes[node].components.iter().find(|c| c.enabled && c.class == "UnityEngine.Canvas")
}

/// The node of the nearest enabled Canvas at or above `node` (None: host canvas).
fn canvas_of(doc: &UiPrefab, rects: &[UiRect], node: usize) -> Option<usize> {
    let mut cursor = Some(node);
    while let Some(i) = cursor {
        if rects[i].active && enabled_canvas(doc, i).is_some() {
            return Some(i);
        }
        cursor = doc.parent(i);
    }
    None
}

fn sorting_order(doc: &UiPrefab, rects: &[UiRect], canvas: Option<usize>, host_order: i32) -> i32 {
    let mut cursor = canvas;
    while let Some(i) = cursor {
        let component = enabled_canvas(doc, i).expect("canvas node carries a Canvas");
        if flag(component, "m_OverrideSorting") {
            return field(component, "m_SortingOrder").as_i64().unwrap_or_else(|| {
                panic!("UI raycast: Canvas @{} m_SortingOrder is not an integer", component.path_id)
            }) as i32;
        }
        cursor = doc.parent(i).and_then(|p| canvas_of(doc, rects, p));
    }
    host_order
}

/// `Graphic.Raycast`'s levels from `node` up to the view root.
fn levels(doc: &UiPrefab, rects: &[UiRect], node: usize, pointer: Pointer) -> Vec<Vec<rule::LevelComponent>> {
    let mut out = Vec::new();
    let mut cursor = Some(node);
    while let Some(i) = cursor {
        let mut level = Vec::new();
        for c in &doc.nodes[i].components {
            match c.class.as_str() {
                "UnityEngine.Canvas" => level.push(rule::LevelComponent::Canvas {
                    override_sorting: flag(c, "m_OverrideSorting"),
                }),
                "UnityEngine.CanvasGroup" => level.push(rule::LevelComponent::CanvasGroup {
                    enabled: c.enabled,
                    blocks_raycasts: flag(c, "m_BlocksRaycasts"),
                    ignore_parent_groups: flag(c, "m_IgnoreParentGroups"),
                }),
                "UnityEngine.UI.RectMask2D" => level.push(rule::LevelComponent::Filter {
                    valid: !(rects[i].active && c.enabled)
                        || padded_contains(&rects[i], padding(c, "m_Padding"), pointer),
                }),
                "UnityEngine.UI.Mask" => level.push(rule::LevelComponent::Filter {
                    valid: !(rects[i].active && c.enabled) || padded_contains(&rects[i], [0.0; 4], pointer),
                }),
                _ => {}
            }
        }
        out.push(level);
        cursor = doc.parent(i);
    }
    out
}

/// The node of the graphic that wins the pointer.
pub(crate) fn winner(doc: &UiPrefab, rects: &[UiRect], pointer: Pointer, host_sorting_order: i32) -> Option<usize> {
    // Raycasters in first-seen order: the host canvas, then nested ones.
    let mut modules: Vec<Option<usize>> = vec![None];
    let mut candidates: Vec<(Option<usize>, usize, rule::GraphicCandidate)> = Vec::new();
    let mut depth_in: Vec<(Option<usize>, i32)> = Vec::new();
    for (i, node) in doc.nodes.iter().enumerate() {
        if !rects[i].active {
            continue;
        }
        for c in node.components.iter().filter(|c| c.enabled && is_graphic(c)) {
            let module = canvas_of(doc, rects, i);
            if let Some(canvas) = module {
                let raycaster = doc.nodes[canvas].components.iter()
                    .any(|r| r.enabled && r.class == "UnityEngine.UI.GraphicRaycaster");
                if !raycaster {
                    continue;
                }
            }
            if !modules.contains(&module) {
                modules.push(module);
            }
            let depth = match depth_in.iter_mut().find(|(m, _)| *m == module) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.1
                }
                None => {
                    depth_in.push((module, 0));
                    0
                }
            };
            let contains_point = padded_contains(&rects[i], padding(c, "m_RaycastPadding"), pointer);
            let candidate = rule::GraphicCandidate {
                raycast_target: flag(c, "m_RaycastTarget"),
                culled: false,
                depth,
                contains_point,
                raycast: contains_point && rule::graphic_raycast(true, &levels(doc, rects, i, pointer)),
            };
            candidates.push((module, i, candidate));
        }
    }
    let mut keys = Vec::new();
    let mut nodes = Vec::new();
    for (module_id, module) in modules.iter().enumerate() {
        let list: Vec<&(Option<usize>, usize, rule::GraphicCandidate)> =
            candidates.iter().filter(|(m, _, _)| m == module).collect();
        let graphics: Vec<rule::GraphicCandidate> = list.iter().map(|(_, _, g)| *g).collect();
        let hits = rule::raycaster_hits(&graphics);
        let order = sorting_order(doc, rects, *module, host_sorting_order);
        for hit in hits.order {
            keys.push(rule::ResultKey {
                module: module_id,
                camera_depth: Some(0.0),
                sort_order_priority: i32::MIN,
                render_order_priority: i32::MIN,
                sorting_layer_value: 0,
                sorting_order: order,
                depth: graphics[hit].depth,
                root_raycaster: 0,
                distance: 0.0,
                index: keys.len(),
            });
            nodes.push(list[hit].1);
        }
    }
    rule::winner(&keys).map(|k| nodes[k])
}

/// The selectable that handles a press on `node`: the nearest at or above it.
/// Returns its node and component identity.
pub(crate) fn selectable_handler(doc: &UiPrefab, node: usize) -> Option<(usize, i64)> {
    let mut cursor = Some(node);
    while let Some(i) = cursor {
        if let Some(c) = doc.nodes[i].components.iter().find(|c| is_selectable(c)) {
            return Some((i, c.path_id));
        }
        cursor = doc.parent(i);
    }
    None
}

/// `Selectable.ParentGroupAllowsInteraction` for a selectable on `node`.
pub(crate) fn groups_allow_interaction(doc: &UiPrefab, node: usize) -> bool {
    let mut levels = Vec::new();
    let mut cursor = Some(node);
    while let Some(i) = cursor {
        levels.push(
            doc.nodes[i].components.iter()
                .filter(|c| c.class == "UnityEngine.CanvasGroup")
                .map(|c| rule::GroupInteraction {
                    enabled: c.enabled,
                    interactable: flag(c, "m_Interactable"),
                    ignore_parent_groups: flag(c, "m_IgnoreParentGroups"),
                })
                .collect(),
        );
        cursor = doc.parent(i);
    }
    rule::parent_group_allows_interaction(&levels)
}
