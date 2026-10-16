//! Overlay cameras with nothing to draw are switched off.
//!
//! An overlay camera marked [`IdleWhenEmpty`] clears nothing and draws only
//! the entities on its render layers and the UI roots that name it with
//! `UiTargetCamera`. While none of them is visible in the hierarchy (a UI root
//! also counts as hidden while its display is `None`), its passes add nothing
//! to the image and its output copy repeats the one written below it, so the
//! camera is made inactive; it is active again on the first frame one of them
//! is visible.
//!
//! The engine's default UI camera is the window camera with the highest
//! order, active or not, so a marked camera can be that camera (the screen
//! fade's, once the entry cover is gone). UI roots here name their camera
//! with `UiTargetCamera` and do not depend on the default one.

use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::prelude::*;

/// An overlay camera switched by [`switch_idle_overlays`].
#[derive(Component)]
pub(crate) struct IdleWhenEmpty;

/// PostUpdate, between visibility propagation and the per-camera visibility
/// check: a marked camera is active exactly when an entity on its layers or a
/// UI root that targets it is visible in the hierarchy.
fn switch_idle_overlays(
    mut cameras: Query<(Entity, &mut Camera, &RenderLayers), With<IdleWhenEmpty>>,
    content: Query<(&RenderLayers, &InheritedVisibility), Without<Camera>>,
    ui_roots: Query<(&UiTargetCamera, &Node, &InheritedVisibility)>,
) {
    if cameras.is_empty() {
        return;
    }
    // The layers of every visible entity, one bit per layer.
    let mut visible: Vec<u64> = Vec::new();
    for (layers, visibility) in &content {
        if !visibility.get() {
            continue;
        }
        let bits = layers.bits();
        if visible.len() < bits.len() {
            visible.resize(bits.len(), 0);
        }
        for (word, bits) in visible.iter_mut().zip(bits) {
            *word |= bits;
        }
    }
    for (entity, mut camera, layers) in &mut cameras {
        let drawn = layers
            .bits()
            .iter()
            .zip(&visible)
            .any(|(camera_bits, visible_bits)| camera_bits & visible_bits != 0)
            || ui_roots.iter().any(|(target, node, visibility)| {
                target.entity() == entity && node.display != Display::None && visibility.get()
            });
        if camera.is_active != drawn {
            camera.is_active = drawn;
        }
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(
        PostUpdate,
        switch_idle_overlays
            .after(VisibilitySystems::VisibilityPropagate)
            .before(VisibilitySystems::CheckVisibility),
    );
}
