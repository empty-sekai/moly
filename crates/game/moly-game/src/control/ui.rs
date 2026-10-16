//! `moly.ui.views`: the prefab views shown, and the controls on each with the
//! path a director `ui.tap` step takes and the window point that step taps.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde_json::{json, Value};

use crate::ui_layout::{UiLayouts, UiPrefabView};

pub(super) fn views(world: &mut World) -> Result<Value, String> {
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .map_err(|_| "no primary window".to_owned())?
        .clone();
    let root = *world.get_resource::<crate::canvas::RootCanvas>().ok_or("the root canvas is not set up")?;
    let canvas = root.size(&window);
    let mut query = world.query::<(&UiPrefabView, Option<&InheritedVisibility>)>();
    let layouts = world.get_resource::<UiLayouts>().ok_or("the UI layouts are not loaded")?;
    let mut views = Vec::new();
    for (view, visibility) in query.iter(world) {
        if visibility.is_some_and(|visibility| !visibility.get()) {
            continue;
        }
        let Some(controls) = view.controls(layouts, canvas) else {
            views.push(json!({ "view": view.key, "laidOut": false, "controls": [] }));
            continue;
        };
        let controls: Vec<Value> = controls
            .into_iter()
            .map(|(path, class, interactable, rect)| {
                let at = root.to_window(&window, rect.center());
                json!({
                    "path": path,
                    "class": class.rsplit('.').next().unwrap_or(&class),
                    "active": rect.active,
                    "interactable": interactable,
                    "at": [at.x.round(), at.y.round()],
                })
            })
            .collect();
        views.push(json!({ "view": view.key, "laidOut": true, "controls": controls }));
    }
    views.sort_by(|a, b| a["view"].as_str().cmp(&b["view"].as_str()));
    Ok(json!({ "window": [window.width(), window.height()], "views": views }))
}
