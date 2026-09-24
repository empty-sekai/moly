//! Research instrument: the values this crate's UI path computes on one
//! extracted field-menu host layout, written as f32 bit patterns so that an
//! external comparison can set them against values produced by the game's
//! own code or by a cited transcription of the engine source.
//!
//! It drives the product's own paths, not copies of them. The host canvas,
//! TMP settings and wordings are applied by `UiLayouts::apply_host_sources`,
//! the root's TMP font assets (when the input names them) by the loader's
//! `UiLayouts::set_tmp_fonts`;
//! the layout is installed into `UiLayouts`; the host view is set up by
//! `menu_shell::settled_host_view` (the bindings' visibility, the break-time
//! gauge after its screen setup, the chrome at rest). Rects are that view's
//! drawn rects (host overrides and layout controllers applied); image meshes
//! come from the renderer's doc-to-mesh function; text from the renderer's
//! text layout call; presses from the view's press target at canvas points,
//! from the camera-reset press chain fed engine screen points (the event
//! camera's ray) and fed window pixel positions; the event camera's matrices
//! from the root canvas, and its ray on a given control camera.
//!
//! It asserts nothing about the values and pins no behaviour of this crate;
//! it only reports. Ignored by default: it reads the input description named
//! by `MOLY_UI_COMPARE_IN` and writes the report to `MOLY_UI_COMPARE_OUT`.

use super::{group_alphas, image_path, image_rule_mesh_data, serialized_rgba, tmp_layout, ImageDraw, ImageMeshData, Override, Pointer, UiLayouts};
use crate::canvas::RootCameraFields;
use bevy::math::{Mat4, Vec2, Vec3};
use bevy::window::{Window, WindowResolution};
use moly_assets::ui_layout::{UiPrefab, UiRect};
use moly_law::ui::{canvas as canvas_rule, image as rule, screen_ray};
use serde_json::{json, Value};
use std::collections::HashMap;

fn bits(value: f32) -> u32 {
    value.to_bits()
}

fn from_bits(value: &Value) -> f32 {
    f32::from_bits(value.as_u64().expect("f32 bit pattern") as u32)
}

fn ray_bits(ray: &screen_ray::Ray) -> Vec<u32> {
    ray.origin.iter().chain(ray.direction.iter()).map(|v| v.to_bits()).collect()
}

fn pointer_json(pointer: Pointer) -> Value {
    match pointer {
        Pointer::Ray { ray, root } => json!({
            "rayBits": ray_bits(&ray),
            "rootPositionBits": root.position.map(bits),
            "rootRotationBits": root.rotation.map(bits),
            "rootScaleBits": root.scale.map(bits),
        }),
        Pointer::Canvas(point) => json!({"canvasBits": point.to_array().map(bits)}),
    }
}

fn numbers<const N: usize>(value: &Value, what: &str) -> [f32; N] {
    let items = value.as_array().filter(|a| a.len() == N).unwrap_or_else(|| panic!("{what}: {N} numbers"));
    std::array::from_fn(|k| items[k].as_f64().unwrap_or_else(|| panic!("{what}: not a number")) as f32)
}

/// The event camera's matrices, aspect and pose for a camera on a screen.
fn camera_json(camera: &screen_ray::OrthographicCamera, target: [f32; 2]) -> Value {
    let (world_to_camera, projection, world_to_clip) = screen_ray::matrices(camera, target);
    let rect = screen_ray::camera_pixel_rect(
        screen_ray::Rect { x: 0.0, y: 0.0, width: target[0], height: target[1] },
        camera.viewport,
    );
    json!({
        "worldToCamera": world_to_camera.map(bits),
        "projection": projection.map(bits),
        "worldToClip": world_to_clip.map(bits),
        "aspectBits": bits(screen_ray::implicit_aspect(rect)),
        "viewportInt": screen_ray::rect_to_rect_int(rect),
        "worldPositionBits": camera.world_position.map(bits),
        "worldRotationBits": camera.world_rotation.map(bits),
    })
}

fn read_json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `RectTransform.GetWorldCorners` order: (xMin,yMin), (xMin,yMax), (xMax,yMax), (xMax,yMin).
fn corners(rect: &UiRect) -> Vec<[u32; 3]> {
    let local = rule::Rect::from_size_pivot(rect.size.to_array(), rect.pivot.to_array());
    let (x0, y0) = (local.x, local.y);
    let (x1, y1) = (local.x + local.width, local.y + local.height);
    [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]
        .iter()
        .map(|&(x, y)| {
            let p = rect.world.transform_point3(Vec3::new(x, y, 0.0));
            [bits(p.x), bits(p.y), bits(p.z)]
        })
        .collect()
}

fn all_corners(rects: &[UiRect]) -> Value {
    json!(rects.iter().map(corners).collect::<Vec<_>>())
}

fn mesh_json(mesh: Result<ImageMeshData, ImageDraw>) -> Value {
    match mesh {
        Ok(mesh) => json!({
            "positions": mesh.positions.iter().map(|p| p.map(bits)).collect::<Vec<_>>(),
            "uvs": mesh.uvs.iter().map(|uv| uv.map(bits)).collect::<Vec<_>>(),
            "indices": mesh.indices,
            "ruleColors": mesh.rule_colors,
            "cropBits": mesh.crop.map(bits),
            "textureSizeBits": mesh.texture_size.map(bits),
        }),
        Err(ImageDraw::OtherPath) => json!({"otherPath": true}),
        Err(ImageDraw::Fallback(reason)) => json!({"fallback": reason}),
    }
}

/// A node rect of the given size at the node's own pivot, at the origin.
/// The RectTransform values the layout controllers produce, by node.
fn layout_values(values: &HashMap<usize, moly_assets::ui_layout::RectTransform>) -> Value {
    let mut nodes: Vec<_> = values.iter().collect();
    nodes.sort_unstable_by_key(|(index, _)| **index);
    json!(nodes
        .into_iter()
        .map(|(index, r)| json!({
            "node": index,
            "anchorMin": r.anchors_min.map(bits),
            "anchorMax": r.anchors_max.map(bits),
            "anchoredPosition": r.anchored_position.map(bits),
            "sizeDelta": r.size_delta.map(bits),
            "pivot": r.pivot.map(bits),
        }))
        .collect::<Vec<_>>())
}

fn bare_rect(size: Vec2, pivot: Vec2) -> UiRect {
    UiRect { size, pivot, world: Mat4::IDENTITY, active: true, clipping: Default::default() }
}

#[test]
#[ignore = "research instrument: needs MOLY_UI_COMPARE_IN and MOLY_UI_COMPARE_OUT"]
fn ui_values_for_source_comparison() {
    let input_path = std::env::var("MOLY_UI_COMPARE_IN").expect("MOLY_UI_COMPARE_IN");
    let output_path = std::env::var("MOLY_UI_COMPARE_OUT").expect("MOLY_UI_COMPARE_OUT");
    let input = read_json(&input_path);
    let doc_text = std::fs::read_to_string(input["doc"].as_str().expect("doc")).expect("doc bytes");
    let source_doc = UiPrefab::parse(&doc_text).expect("doc parses");
    let camera_fields = match input["hostCanvasRoot"].as_str() {
        Some("region") => RootCameraFields::Required,
        Some("shared") => RootCameraFields::SharedRootMayLack,
        other => panic!("hostCanvasRoot must be region or shared, not {other:?}"),
    };
    let mut layouts = UiLayouts::default();
    let root_canvas = layouts.apply_host_sources(
        &read_json(input["hostCanvas"].as_str().expect("hostCanvas")),
        &read_json(input["textSettings"].as_str().expect("textSettings")),
        &read_json(input["wordings"].as_str().expect("wordings")),
        camera_fields,
    );
    if let Some(path) = input["tmpFontAssets"].as_str() {
        let region = source_doc.source.region.as_deref().expect("region document");
        let client = source_doc.source.client_version.as_deref().expect("region document client version");
        let fonts = super::tmp_font::TmpFonts::parse(&read_json(path), region, client).expect("TMP font document");
        layouts.set_tmp_fonts(Some(fonts));
    }
    let key: &'static str = match input["hostKey"].as_str().expect("hostKey") {
        "ShellHome" => "ShellHome",
        "ShellMyRoom" => "ShellMyRoom",
        "ShellHarvest" => "ShellHarvest",
        "ShellDelivery" => "ShellDelivery",
        other => panic!("{other} is not a field-menu host key"),
    };
    layouts.install_for_measurement(key, source_doc.clone());
    let view = crate::menu_shell::settled_host_view(&source_doc, key);
    let doc = layouts.document(key).expect("installed layout");
    let button = crate::menu_shell::camera_reset_button(doc).expect("camera reset CustomButton");
    let button_node = doc.find(&format!("@{button}")).expect("camera reset button node");
    let changes: HashMap<usize, &Override> = view
        .overrides
        .iter()
        .map(|(path, value)| (doc.find(path).expect("view override path"), value))
        .collect();
    let mut view_hidden: Vec<usize> = changes
        .iter()
        .filter(|(_, change)| change.visible == Some(false))
        .map(|(index, _)| *index)
        .collect();
    view_hidden.sort_unstable();
    let mut view_moved: Vec<usize> = changes
        .iter()
        .filter(|(_, change)| change.anchored_position.is_some() || change.size_delta.is_some())
        .map(|(index, _)| *index)
        .collect();
    view_moved.sort_unstable();
    let alphas = group_alphas(doc, &changes);

    // The canvas scaler over the scale grid, through the root canvas rule.
    let scaler = root_canvas.scaler();
    let mut scale_grid = Vec::new();
    for wh in input["scaleGrid"].as_array().expect("scaleGrid") {
        let (w, h) = (wh[0].as_f64().unwrap() as f32, wh[1].as_f64().unwrap() as f32);
        let root = root_canvas.for_pixels([w, h]);
        let m0 = canvas_rule::scale_with_screen_size([w, h], scaler.reference_resolution, scaler.mode, 0.0);
        let m1 = canvas_rule::scale_with_screen_size([w, h], scaler.reference_resolution, scaler.mode, 1.0);
        scale_grid.push(json!({
            "W": wh[0], "H": wh[1],
            "matchBits": bits(root.match_width_or_height),
            "scaleBits": bits(root.scale_factor),
            "sizeBits": root.size.map(bits),
            "scaleAtMatch0Bits": bits(m0), "scaleAtMatch1Bits": bits(m1),
        }));
    }

    let mut resolutions = serde_json::Map::new();
    let mut text_rects = None;
    for res in input["resolutions"].as_array().expect("resolutions") {
        let res_key = res["key"].as_str().expect("key").to_owned();
        let (w, h) = (res["W"].as_u64().expect("W") as u32, res["H"].as_u64().expect("H") as u32);
        // A window of this many physical pixels at device pixel ratio 1.
        let window = Window { resolution: WindowResolution::new(w, h), ..Default::default() };
        let canvas = root_canvas.size(&window);
        let scale = root_canvas.scale(&window);
        let oracle_root = Vec2::new(from_bits(&res["oracleRootBits"][0]), from_bits(&res["oracleRootBits"][1]));
        let bare = doc.resolve(canvas, &HashMap::new());
        let drawn = view.drawn_rects(&layouts, canvas).expect("drawn rects");
        let drawn_oracle_root = view.drawn_rects(&layouts, oracle_root).expect("drawn rects at the oracle root");
        let driven_bare = layouts.layout_driven_nodes(key, canvas).expect("layout-driven nodes");
        let driven_drawn = view.layout_driven(&layouts, canvas).expect("layout-driven nodes of the view");
        let check = view.solve_check(&layouts, canvas).expect("solve check");
        // The prefab's own activity with the layout controllers applied, and
        // the values the controllers write in both states.
        let bare_layout = layouts.resolved(key, canvas).expect("bare rects with the layout");
        let values_bare = layouts.layout_overrides(doc, canvas, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let (host_visibility, host_rects, host_changes) = view.host_state(doc);
        let values_drawn = layouts.layout_overrides(doc, canvas, &host_visibility, &host_rects, &host_changes);

        // Images drawn by the Image rules: at the drawn rect, and at the
        // oracle's rect size (same input on both sides).
        let oracle_rects = res["oracleRectBits"].as_array().expect("oracleRectBits");
        let mut images = Vec::new();
        for (i, node) in doc.nodes.iter().enumerate() {
            for c in &node.components {
                if c.fields.get("m_Sprite").is_none() || !matches!(c.fields["m_Type"].as_i64(), Some(0 | 1)) {
                    continue;
                }
                let change = changes.get(&i).copied();
                let r = &oracle_rects[i];
                let oracle_rect = bare_rect(Vec2::new(from_bits(&r[2]), from_bits(&r[3])), Vec2::from_array(node.rect.pivot));
                let tint = serialized_rgba(&c.fields, "m_Color").to_srgba();
                let tint_bits = [tint.red, tint.green, tint.blue, tint.alpha].map(bits);
                images.push(json!({
                    "node": i, "pathId": c.path_id, "class": c.class, "type": c.fields["m_Type"],
                    "drawnActive": drawn[i].active,
                    "drawnSizeBits": drawn[i].size.to_array().map(bits),
                    "pivotBits": drawn[i].pivot.to_array().map(bits),
                    // The renderer draws an Image without an image by its plain sprite path.
                    "hasImage": image_path(&layouts, c, change).is_some(),
                    "drawn": mesh_json(image_rule_mesh_data(&layouts, doc, i, c, change, &drawn[i])),
                    "oracleRect": mesh_json(image_rule_mesh_data(&layouts, doc, i, c, change, &oracle_rect)),
                    "materialTintBits": tint_bits,
                    "groupAlphaBits": bits(alphas[i]),
                }));
            }
        }

        // Presses: the view's press target at the oracle's canvas points; the
        // camera-reset press chain at the engine screen point (origin
        // bottom-left) whose image under the canvas mapping is that point,
        // through the event camera's ray; and the same chain at the window
        // pixel position (origin top-left) of that point.
        let pixels = [w as f32, h as f32];
        let (event_camera, event_root) = root_canvas.event_camera(pixels).expect("the host canvas carries the event camera pose");
        let mut presses = Vec::new();
        for p in res["points"].as_array().expect("points") {
            let point = Vec2::new(from_bits(&p[0]), from_bits(&p[1]));
            let target = view.press_target(&layouts, Pointer::Canvas(point), canvas);
            let engine_screen = [
                (f64::from(w) / 2. + f64::from(point.x) * f64::from(scale)) as f32,
                (f64::from(h) / 2. + f64::from(point.y) * f64::from(scale)) as f32,
            ];
            let pointer = crate::menu_shell::screen_pointer(&window, &root_canvas, engine_screen)
                .expect("the host canvas carries the event camera pose");
            let ray_error = root_canvas.event_ray(pixels, engine_screen).and_then(|event| event.error);
            let ray_hit = crate::menu_shell::camera_reset_press(&view, &layouts, canvas, pointer, button_node, button);
            let screen = Vec2::new(
                (f64::from(w) / 2. + f64::from(point.x) * f64::from(scale)) as f32,
                (f64::from(h) / 2. - f64::from(point.y) * f64::from(scale)) as f32,
            );
            let hit = crate::menu_shell::camera_reset_hit(&view, &layouts, &window, &root_canvas, screen, button_node, button);
            presses.push(json!({
                "canvasTarget": target.map(|(n, id)| json!([n, id])),
                "engineScreenBits": engine_screen.map(bits),
                "enginePointer": pointer_json(ray_hit.pointer),
                "engineRayError": ray_error,
                "engineTarget": ray_hit.target.map(|(n, id)| json!([n, id])),
                "engineOverButton": ray_hit.over_button,
                "engineHovered": ray_hit.hovered,
                "screenBits": screen.to_array().map(bits),
                "screenPointer": pointer_json(hit.pointer),
                "screenTarget": hit.target.map(|(n, id)| json!([n, id])),
                "screenOverButton": hit.over_button,
                "buttonActive": hit.active, "buttonInteractable": hit.interactable,
            }));
        }
        resolutions.insert(res_key.clone(), json!({
            "rootSizeBits": canvas.to_array().map(bits),
            "windowScaleBits": bits(scale),
            "eventCamera": camera_json(&event_camera, pixels),
            "eventRootScaleBits": event_root.scale.map(bits),
            "bare": all_corners(&bare),
            "drawn": all_corners(&drawn),
            "drawnOracleRoot": all_corners(&drawn_oracle_root),
            "bareLayout": all_corners(&bare_layout),
            "layoutValuesBare": layout_values(&values_bare),
            "layoutValuesDrawn": layout_values(&values_drawn),
            "drawnActive": drawn.iter().map(|r| r.active).collect::<Vec<_>>(),
            "layoutDrivenBare": driven_bare,
            "layoutDrivenDrawn": driven_drawn,
            "solveCheck": {"nodes": check.nodes, "nonFinite": check.non_finite, "negativeSize": check.negative_size},
            "images": images,
            "presses": presses,
        }));
        if res_key == "1920x1080" {
            text_rects = Some((drawn, oracle_rects.clone()));
        }
    }

    // Every TMP component laid out as the renderer lays it out, at the drawn
    // rect size and at the oracle's rect size, at 1920x1080.
    let (drawn, oracle_rects) = text_rects.expect("1920x1080 among the resolutions");
    let rules = layouts.text_rules.as_ref().expect("TMP line rules applied");
    let mut texts = Vec::new();
    for (i, node) in doc.nodes.iter().enumerate() {
        for c in &node.components {
            if c.fields.get("m_fontSize").is_none() {
                continue;
            }
            let change = changes.get(&i).copied();
            let text = change.and_then(|v| v.text.clone()).unwrap_or_else(|| layouts.text(c));
            let alignment = change.and_then(|v| v.text_alignment);
            let r = &oracle_rects[i];
            let oracle_size = Vec2::new(from_bits(&r[2]), from_bits(&r[3]));
            let run = |size: Vec2| -> Value {
                // Pens are relative to the rect's pivot (the drawn rect's).
                match tmp_layout::layout(&text, c, size, drawn[i].pivot, rules, alignment) {
                    Ok(layout) => json!(layout.glyphs.iter().map(|g| json!({
                        "ch": g.ch.to_string(), "sourceIndex": g.source_index,
                        "fontSizeBits": bits(g.font_size), "scaleBits": bits(g.scale),
                        "widthScaleBits": bits(g.width_scale),
                        "penBits": [bits(g.pen.x), bits(g.pen.y)],
                    })).collect::<Vec<_>>()),
                    Err(e) => json!({"error": e}),
                }
            };
            // The open font's advances (em units) of every character of the
            // text, and the kerning pairs between neighbours once rich-text
            // tags are removed, as the renderer's layout reads them.
            let mut advances = serde_json::Map::new();
            for ch in text.chars() {
                advances.insert((ch as u32).to_string(), json!(tmp_layout::open_font_advance(ch).map(bits)));
            }
            let mut plain = Vec::new();
            let mut in_tag = false;
            for ch in text.chars() {
                match ch {
                    '<' => in_tag = true,
                    '>' if in_tag => in_tag = false,
                    _ if !in_tag => plain.push(ch),
                    _ => {}
                }
            }
            let pairs: Vec<Value> = plain
                .windows(2)
                .map(|w| match tmp_layout::open_font_pair(w[0], w[1]) {
                    Ok((advance, offset)) => json!({
                        "left": w[0] as u32, "right": w[1] as u32,
                        "advanceBits": advance.map(bits),
                        "offsetBits": offset.map(|o| o.map(bits)),
                    }),
                    Err(e) => json!({"left": w[0] as u32, "right": w[1] as u32, "error": e}),
                })
                .collect();
            texts.push(json!({
                "node": i, "pathId": c.path_id, "active": node.active, "drawnActive": drawn[i].active,
                "text": text,
                "openFontAdvanceBits": advances,
                "openFontPairs": pairs,
                "oracleRect": run(oracle_size),
                "drawnRect": run(drawn[i].size),
                "drawnSizeBits": [bits(drawn[i].size.x), bits(drawn[i].size.y)],
            }));
        }
    }

    // Positive control of the solve check: a copy of the layout with one
    // node's x scale planted at 3e38 (finite, so the loader and the layout
    // controllers accept it, but its world corners overflow) and another
    // node's width driven negative must be reported by the same check.
    let control = input.get("solveCheckControl").map(|control| {
        let non_finite = control["nonFiniteNode"].as_u64().expect("nonFiniteNode") as usize;
        let negative = control["negativeSizeNode"].as_u64().expect("negativeSizeNode") as usize;
        let mut planted = source_doc.clone();
        planted.nodes[non_finite].rect.local_scale[0] = 3.0e38;
        planted.nodes[negative].rect.size_delta[0] = -1.0e4;
        layouts.install_for_measurement("SolveCheckControl", planted);
        let view = super::UiPrefabView::new("SolveCheckControl", 0);
        let window = Window { resolution: WindowResolution::new(1920, 1080), ..Default::default() };
        let check = view.solve_check(&layouts, root_canvas.size(&window)).expect("control solve check");
        json!({
            "nonFiniteNode": non_finite, "negativeSizeNode": negative,
            "nodes": check.nodes, "nonFinite": check.non_finite, "negativeSize": check.negative_size,
        })
    });

    // Control of the screen-point ray on a camera the host does not use: a
    // rotated, off-centre camera under a moved and scaled parent, over a
    // partial viewport, through the same pose and ray functions.
    let ray_control = input.get("rayControl").map(|control| {
        let screen: [f32; 2] = numbers(&control["screen"], "rayControl screen");
        let parent = screen_ray::Pose {
            position: numbers(&control["rootPos"], "rayControl rootPos"),
            rotation: numbers(&control["rootRot"], "rayControl rootRot"),
            scale: numbers(&control["rootScale"], "rayControl rootScale"),
        };
        let (world_position, world_rotation) = screen_ray::child_world_pose(
            &parent,
            numbers(&control["localPos"], "rayControl localPos"),
            numbers(&control["localRot"], "rayControl localRot"),
        );
        let viewport: [f32; 4] = numbers(&control["viewport"], "rayControl viewport");
        let camera = screen_ray::OrthographicCamera {
            orthographic_size: control["orthoSize"].as_f64().expect("rayControl orthoSize") as f32,
            near: control["near"].as_f64().expect("rayControl near") as f32,
            far: control["far"].as_f64().expect("rayControl far") as f32,
            viewport: screen_ray::Rect { x: viewport[0], y: viewport[1], width: viewport[2], height: viewport[3] },
            world_position,
            world_rotation,
        };
        let rays: Vec<Value> = control["pixels"]
            .as_array()
            .expect("rayControl pixels")
            .iter()
            .map(|p| {
                let point: [f32; 2] = numbers(p, "rayControl pixel");
                let (ray, error) = screen_ray::screen_point_to_ray(&camera, screen, point);
                json!({"pixelBits": point.map(bits), "rayBits": ray_bits(&ray), "error": error})
            })
            .collect();
        json!({"camera": camera_json(&camera, screen), "rays": rays})
    });

    let report = json!({
        "doc": input["doc"], "hostKey": key, "hostCanvasRoot": input["hostCanvasRoot"],
        "solveCheckControl": control,
        "rayControl": ray_control,
        "cameraResetButton": [button_node, button],
        "viewHidden": view_hidden, "viewMoved": view_moved,
        "scaleGrid": scale_grid, "resolutions": resolutions, "texts": texts,
    });
    std::fs::write(&output_path, serde_json::to_string(&report).expect("report json")).expect("write report");
    println!("UI source comparison values written: {output_path}");
}

/// The field camera's reset as the camera-reset button's click runs it.
fn press_camera_reset(mut commands: bevy::prelude::Commands, mut reset: crate::camera::CameraReset) {
    reset.reset_camera_setting(&mut commands);
}

fn vec3_bits(v: Vec3) -> [u32; 3] {
    v.to_array().map(bits)
}

fn f32_field(case: &Value, key: &str) -> f32 {
    from_bits(&case[key])
}

/// The camera reset on given field camera states: the reset pressed once in
/// the Normal state, then the tween stepped frame by frame by the camera
/// follow system with no avatar spawned, so nothing but the tween writes the
/// model. Reports the captured tween and the model after every frame.
#[test]
#[ignore = "research instrument: needs MOLY_CAMERA_COMPARE_IN and MOLY_CAMERA_COMPARE_OUT"]
fn camera_reset_values_for_source_comparison() {
    use crate::camera::{
        BoundsXz, CameraSetting, CameraStateType, CameraTween, FieldCameraModel, FieldCameraState,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::{Camera3d, PerspectiveProjection, Projection, Time, Transform, World};
    let input = read_json(&std::env::var("MOLY_CAMERA_COMPARE_IN").expect("MOLY_CAMERA_COMPARE_IN"));
    let output_path = std::env::var("MOLY_CAMERA_COMPARE_OUT").expect("MOLY_CAMERA_COMPARE_OUT");
    let mut cases = Vec::new();
    for case in input["cases"].as_array().expect("cases") {
        let setting_in = &case["setting"];
        let setting = CameraSetting {
            offset: Vec3::from_array([0, 1, 2].map(|k| from_bits(&setting_in["offsetBits"][k]))),
            distance: f32_field(setting_in, "distanceBits"),
            min_distance: f32_field(setting_in, "minDistanceBits"),
            max_distance: f32_field(setting_in, "maxDistanceBits"),
            init_yaw: f32_field(setting_in, "initYawBits"),
            init_pitch: f32_field(setting_in, "initPitchBits"),
            min_pitch: f32_field(setting_in, "minPitchBits"),
            max_pitch: f32_field(setting_in, "maxPitchBits"),
            rot_sensitivity: f32_field(setting_in, "rotSensitivityBits"),
            fov: f32_field(setting_in, "fovBits"),
        };
        let model_in = &case["model"];
        let unbounded = BoundsXz { center: Vec2::ZERO, extents: Vec2::splat(f32::MAX) };
        let model = FieldCameraModel {
            look_at: Vec3::from_array([0, 1, 2].map(|k| from_bits(&model_in["lookAtBits"][k]))),
            offset: setting.offset,
            fov: f32_field(model_in, "fovBits"),
            distance: f32_field(model_in, "distanceBits"),
            min_distance: setting.min_distance,
            max_distance: setting.max_distance,
            yaw: f32_field(model_in, "yawBits"),
            pitch: f32_field(model_in, "pitchBits"),
            rot_sensitivity: setting.rot_sensitivity,
            min_pitch: f32_field(model_in, "minPitchBits"),
            max_pitch: f32_field(model_in, "maxPitchBits"),
            gestured_distance: f32_field(model_in, "gesturedDistanceBits"),
            look_at_bounds: unbounded,
            max_look_at_bounds: unbounded,
        };
        let mut world = World::new();
        world.insert_resource(FieldCameraState(CameraStateType::Normal));
        world.insert_resource(model);
        world.insert_resource(setting);
        world.insert_resource(Time::<()>::default());
        // The camera body's field of view in degrees, installed as radians
        // of the degree value, the way site framing installs it.
        let camera_fov_deg = f32_field(case, "cameraFovDegBits");
        world.spawn((
            Camera3d::default(),
            Transform::default(),
            Projection::Perspective(PerspectiveProjection { fov: camera_fov_deg.to_radians(), ..Default::default() }),
        ));
        world.run_system_once(press_camera_reset).expect("reset system runs");
        let tween = *world.get_resource::<CameraTween>().expect("the reset inserts a camera tween");
        let after_press = world.resource::<FieldCameraModel>().clone();
        let start = json!({
            "lookAtBits": [vec3_bits(tween.look_at.0), vec3_bits(tween.look_at.1)],
            "fovBits": [bits(tween.fov.0), bits(tween.fov.1)],
            "pitchBits": [bits(tween.pitch.0), bits(tween.pitch.1)],
            "yawBits": [bits(tween.yaw.0), bits(tween.yaw.1)],
            "distanceBits": [bits(tween.distance.0), bits(tween.distance.1)],
            "durationBits": bits(tween.duration),
            "isNormalReset": tween.is_normal_reset(),
            "modelMinPitchBits": bits(after_press.min_pitch),
            "modelMaxPitchBits": bits(after_press.max_pitch),
            "modelGesturedDistanceBits": bits(after_press.gestured_distance),
            "modelPitchBits": bits(after_press.pitch),
            "modelYawBits": bits(after_press.yaw),
            "modelDistanceBits": bits(after_press.distance),
        });
        let mut frames = Vec::new();
        for dt in case["deltaNanos"].as_array().expect("deltaNanos") {
            let nanos = dt.as_u64().expect("delta nanoseconds");
            world.resource_mut::<Time>().advance_by(std::time::Duration::from_nanos(nanos));
            let delta = world.resource::<Time>().delta_secs();
            world.run_system_once(crate::camera::follow_avatar).expect("follow system runs");
            let model = world.resource::<FieldCameraModel>().clone();
            let tween_after = world.get_resource::<CameraTween>().copied();
            let fov = world
                .query::<&Projection>()
                .iter(&world)
                .map(|p| match p {
                    Projection::Perspective(p) => p.fov,
                    _ => panic!("field camera projection is perspective"),
                })
                .next()
                .expect("camera");
            frames.push(json!({
                "deltaBits": bits(delta),
                "elapsedBits": tween_after.map(|t| bits(t.elapsed)),
                "tweenLive": tween_after.is_some(),
                "isNormalReset": tween_after.map(|t| t.is_normal_reset()),
                "lookAtBits": vec3_bits(model.look_at),
                "pitchBits": bits(model.pitch),
                "yawBits": bits(model.yaw),
                "distanceBits": bits(model.distance),
                "gesturedDistanceBits": bits(model.gestured_distance),
                "minPitchBits": bits(model.min_pitch),
                "maxPitchBits": bits(model.max_pitch),
                "cameraFovRadBits": bits(fov),
            }));
        }
        cases.push(json!({"name": case["name"], "start": start, "frames": frames}));
    }
    std::fs::write(&output_path, serde_json::to_string(&json!({"cases": cases})).expect("report json"))
        .expect("write report");
    println!("camera reset comparison values written: {output_path}");
}
