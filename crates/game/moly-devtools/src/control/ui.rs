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
        let scroll_rects: Vec<Value> = layouts.document(view.key).into_iter()
            .flat_map(|doc| doc.nodes.iter())
            .flat_map(|node| node.components.iter().map(move |component| (node, component)))
            .filter(|(_, component)| component.class.ends_with("ScrollRect"))
            .map(|(node, component)| {
                let rectangle = |field: &str| {
                    let pointer = &component.fields[field];
                    if pointer[0].as_i64() != Some(0) { return Value::Null; }
                    let Some(id) = pointer[1].as_i64().filter(|id| *id != 0) else { return Value::Null; };
                    view.rect(layouts, &format!("@{id}"), canvas).map_or(Value::Null, |rect| {
                        json!({"size": rect.size.to_array(), "center": rect.center().to_array(), "active": rect.active})
                    })
                };
                json!({"path": node.path, "viewport": rectangle("m_Viewport"), "content": rectangle("m_Content")})
            }).collect();
        views.push(json!({ "view": view.key, "laidOut": true, "controls": controls, "scrollRects": scroll_rects }));
    }
    views.sort_by(|a, b| a["view"].as_str().cmp(&b["view"].as_str()));
    Ok(json!({ "window": [window.width(), window.height()], "canvas": canvas.to_array(), "scale": root.scale(&window), "views": views }))
}
