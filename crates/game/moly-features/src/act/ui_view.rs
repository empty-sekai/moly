//! The window point of a node of a shown UI view: the view's own press
//! raycast point, which a tap sends so that the game's own pointer
//! conversion and raycast decide.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::{canvas::RootCanvas, menu_shell, ui_layout::{UiLayouts, UiPrefabView}};

/// The window point of the node at `path` on the shown UI view `view` (its
/// document key), in logical pixels with a top-left origin, that the view's
/// own press raycast gives to the node: its centre when the raycast there
/// reaches the node, else another point of its rect. Several views can share
/// a document (a host's screen and a layout read for geometry only); the
/// first whose node is active is taken. A view that is not shown is named
/// with the views that are; a node active on none is refused; a node whose
/// whole rect is covered by other controls is refused with their paths.
pub fn point(world: &mut World, view: &str, path: &str) -> Result<Vec2, String> {
    let (_, window) = world
        .query_filtered::<(Entity, &Window), With<PrimaryWindow>>()
        .single(world)
        .map_err(|_| "no primary window".to_owned())?;
    let window = window.clone();
    let root = *world.get_resource::<RootCanvas>().ok_or("the root canvas is not set up")?;
    let canvas = root.size(&window);
    let mut views = world.query::<(&UiPrefabView, Option<&InheritedVisibility>)>();
    let layouts = world.get_resource::<UiLayouts>().ok_or("the UI layouts are not loaded")?;
    let shown: Vec<&UiPrefabView> = views
        .iter(world)
        .filter(|(_, visibility)| visibility.is_none_or(|visibility| visibility.get()))
        .map(|(view, _)| view)
        .collect();
    let candidates: Vec<&UiPrefabView> = shown.iter().filter(|candidate| candidate.key == view).copied().collect();
    if candidates.is_empty() {
        let mut keys: Vec<&str> = shown.iter().map(|candidate| candidate.key).collect();
        keys.sort_unstable();
        keys.dedup();
        return Err(format!("no view {view} is shown (shown: {})", keys.join(", ")));
    }
    let (mut found, mut refused) = (None, None);
    for candidate in candidates {
        match candidate.try_rect(layouts, path, canvas) {
            Ok(rect) if rect.active => {
                found = Some((candidate, rect));
                break;
            }
            Ok(_) => {}
            Err(reason) => refused = refused.or(Some(reason)),
        }
    }
    let Some((view, rect)) = found else {
        return Err(refused.unwrap_or_else(|| format!("{view} {path} is not active")));
    };
    let doc = layouts.document(view.key).ok_or_else(|| format!("view {} has no document", view.key))?;
    let index = doc.find(path)?;
    press_point(view, doc, layouts, &window, &root, canvas, &rect, path, index)
}

/// The point of `rect` the view's press raycast gives to `index`: the centre
/// first, then a grid across the rect's canvas bounds; every tested point is
/// the window point the tap sends. The covering paths are collected for the
/// refusal.
fn press_point(
    view: &UiPrefabView,
    doc: &moly_assets::ui_layout::UiPrefab,
    layouts: &UiLayouts,
    window: &Window,
    root: &RootCanvas,
    canvas: Vec2,
    rect: &moly_assets::ui_layout::UiRect,
    path: &str,
    index: usize,
) -> Result<Vec2, String> {
    let mut covering = Vec::new();
    if let Some(point) = press_at(view, doc, layouts, window, root, canvas, rect, index, rect.center(), &mut covering) {
        return Ok(point);
    }
    // The centre is the usual covered point; a grid across the rect's bounds
    // finds a free part of it (a fine grid so a small uncovered strip is still
    // found). A point the raycast cannot reach, such as one off the window,
    // simply wins nothing.
    let min = -rect.size * rect.pivot;
    let max = min + rect.size;
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for corner in [Vec2::new(min.x, min.y), Vec2::new(max.x, min.y), Vec2::new(max.x, max.y), Vec2::new(min.x, max.y)] {
        let at = rect.world.transform_point3(corner.extend(0.0)).truncate();
        lo = lo.min(at);
        hi = hi.max(at);
    }
    let mut steps = 4u32;
    while steps <= 32 {
        for iy in 0..steps {
            for ix in 0..steps {
                let fx = (ix as f32 + 1.0) / (steps as f32 + 1.0);
                let fy = (iy as f32 + 1.0) / (steps as f32 + 1.0);
                let at = Vec2::new(lo.x + (hi.x - lo.x) * fx, lo.y + (hi.y - lo.y) * fy);
                if let Some(point) = press_at(view, doc, layouts, window, root, canvas, rect, index, at, &mut covering) {
                    return Ok(point);
                }
            }
        }
        steps *= 2;
    }
    covering.sort_unstable();
    covering.dedup();
    covering.truncate(8);
    let listed = if covering.is_empty() {
        "no control is hit there".to_owned()
    } else {
        format!("covering controls: {}", covering.join(", "))
    };
    Err(format!("view {} path {path}: no point of its rect gives the press to it ({listed})", view.key))
}

/// Whether the press at `at` (a point of this view's canvas units) reaches
/// `index`: the raycast winner, or the selectable the raycast hands the press
/// to. The covering winner's path is kept for the refusal.
fn press_at(
    view: &UiPrefabView,
    doc: &moly_assets::ui_layout::UiPrefab,
    layouts: &UiLayouts,
    window: &Window,
    root: &RootCanvas,
    canvas: Vec2,
    rect: &moly_assets::ui_layout::UiRect,
    index: usize,
    at: Vec2,
    covering: &mut Vec<String>,
) -> Option<Vec2> {
    if !rect.contains(at) {
        return None;
    }
    let point = root.to_window(window, at);
    if point.x < 0.0 || point.y < 0.0 || point.x > window.width() || point.y > window.height() {
        return None;
    }
    let pointer = menu_shell::event_pointer(window, root, point);
    let winner = view.raycast_winner(layouts, pointer, canvas);
    if winner == Some(index) {
        return Some(point);
    }
    if view.press_target(layouts, pointer, canvas).is_some_and(|(node, _)| node == index) {
        return Some(point);
    }
    if let Some(node) = winner.and_then(|winner| doc.nodes.get(winner)) {
        covering.push(node.path.clone());
    }
    None
}
