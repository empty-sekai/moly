//! The captured texture stays owned by this presenter through frame selection.
//! The extracted animator drives presentation. LocalPhotoAdapter exports the
//! original JPEG and frame ID together and never reports an album API result.
use super::*;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use moly_assets::json::JsonAsset;
use serde::Deserialize;
use std::sync::Arc;

const CAPTURE_TEXTURE: &str = "photo-captured-image";
const SHOT_EFFECT: &str = "photo-shot/shot/shot.json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Presentation {
    version: u32,
    duration: f32,
    targets: Vec<Target>,
    curves: Vec<Curve>,
    events: Vec<ClipEvent>,
    frames: Vec<Frame>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    transform_id: i64,
    selector_id: i64,
    graphic_id: Option<i64>,
    position: [f32; 2],
    scale: [f32; 3],
    color: [f32; 4],
}
#[derive(Deserialize)]
struct Frame {
    id: i32,
    name: String,
    preview: String,
    thumbnail: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipEvent {
    time: f32,
    function_name: String,
    data: String,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Property {
    Position,
    Scale,
    Euler,
    Color,
    Alpha,
    Active,
}
#[derive(Deserialize)]
struct Curve {
    target: usize,
    property: Property,
    channel: usize,
    kind: String,
    points: Vec<(f32, serde_json::Value)>,
}

impl Curve {
    fn value(&self, time: f32) -> f32 {
        let index = self
            .points
            .partition_point(|(key, _)| *key <= time)
            .saturating_sub(1);
        let (key, value) = &self.points[index];
        if self.kind == "cubic" {
            let coefficients = value.as_array().expect("validated cubic coefficients");
            let c: [f32; 4] = std::array::from_fn(|i| {
                coefficients[i].as_f64().expect("validated coefficient") as f32
            });
            let dt = (time - *key).max(0.);
            ((c[0] * dt + c[1]) * dt + c[2]) * dt + c[3]
        } else {
            let value = value.as_f64().expect("validated scalar key") as f32;
            if self.kind == "linear" {
                if let Some((next, next_value)) = self.points.get(index + 1) {
                    return value
                        + (next_value.as_f64().expect("validated scalar key") as f32 - value)
                            * ((time - *key) / (*next - *key)).clamp(0., 1.);
                }
            }
            value
        }
    }
}

/// Frame names and dynamically assigned labels enter the shared glyph atlas
/// before the menu renderer bakes it.
#[derive(Resource, Default)]
pub struct PhotoCharset(pub Option<Vec<char>>);

#[derive(Resource)]
struct Source {
    document: Handle<JsonAsset>,
    effect: Handle<JsonAsset>,
    data: Option<Arc<Presentation>>,
    textures: Vec<Handle<Image>>,
    effect_node: Option<String>,
    base: Option<UiPrefab>,
}

struct Bindings {
    save: String,
    skip: String,
    name: String,
    photo: String,
    frame: String,
    shadow: String,
    fx_root: String,
    content: String,
    viewport: String,
    sensitivity: f32,
    elasticity: f32,
    deceleration: f32,
    inertia: bool,
    spacing: Vec2,
    padding: [f32; 4],
    split: usize,
}
struct Cell {
    id: i32,
    root: String,
    button: String,
    image: String,
    quantity: String,
    selected: i64,
    position: Vec2,
}

pub(super) struct Selection {
    pub playing: bool,
    pub header_fade: f32,
    scroll: f32,
    velocity: f32,
    elapsed: f32,
    frame_visible: bool,
    selected: i32,
    source: Arc<Presentation>,
    bindings: Bindings,
    cells: Vec<Cell>,
    content_height: f32,
    effect: Option<Entity>,
    sound_owner: Entity,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<crate::camera::PhotoGestureHeld>()
        .init_resource::<PhotoCharset>()
        .add_systems(
            Update,
            load_source
                .after(super::spawn_when_ready)
                .before(super::click),
        )
        .add_systems(
            Update,
            scroll
                .after(super::click)
                .before(super::dispatch)
                .run_if(crate::game_settings::scene_input_enabled),
        )
        .add_systems(
            Update,
            place.after(super::place).before(crate::ui_layout::render),
        );
}

fn load_source(world: &mut World) {
    if world.resource::<UiLayouts>().document(KEY).is_none() {
        if world.resource::<UiLayouts>().root().is_some()
            && world.resource::<PhotoCharset>().0.is_none()
        {
            world.resource_mut::<PhotoCharset>().0 = Some(Vec::new());
        }
        return;
    }
    if !world.contains_resource::<Source>() {
        let root = world
            .resource::<UiLayouts>()
            .root()
            .expect("photo root")
            .trim_end_matches('/')
            .to_owned();
        let server = world.resource::<AssetServer>();
        let document = server.load(format!("{root}/photo-shot/presentation.json"));
        let effect = server.load(format!("{root}/{SHOT_EFFECT}"));
        world.insert_resource(Source {
            document,
            effect,
            data: None,
            textures: Vec::new(),
            effect_node: None,
            base: None,
        });
    }
    if world.resource::<Source>().data.is_some() {
        return;
    }
    let source = world.resource::<Source>();
    let assets = world.resource::<Assets<JsonAsset>>();
    let (Some(raw), Some(effect)) = (assets.get(&source.document), assets.get(&source.effect))
    else {
        return;
    };
    let result = (|| -> Result<_, String> {
        let data: Presentation = serde_json::from_str(&raw.0).map_err(|e| e.to_string())?;
        if data.version != 1 || !data.duration.is_finite() || data.duration <= 0. {
            return Err("invalid photo animation header".into());
        }
        let layout = world
            .resource::<UiLayouts>()
            .document(KEY)
            .ok_or("photo layout missing")?;
        for target in &data.targets {
            if target
                .position
                .iter()
                .chain(&target.scale)
                .chain(&target.color)
                .any(|v| !v.is_finite())
            {
                return Err("non-finite photo animation target".into());
            }
            layout
                .find(&format!("@{}", target.selector_id))
                .map_err(|e| e.to_string())?;
            if let Some(id) = target.graphic_id {
                bound_component(layout, &format!("@{id}"))?;
            }
        }
        for curve in &data.curves {
            let channels = match curve.property {
                Property::Position => 2,
                Property::Scale | Property::Euler => 3,
                Property::Color => 4,
                _ => 1,
            };
            if curve.target >= data.targets.len()
                || curve.channel >= channels
                || curve.points.is_empty()
                || !["cubic", "linear", "const"].contains(&curve.kind.as_str())
            {
                return Err("invalid photo animation curve".into());
            }
            for (index, (time, value)) in curve.points.iter().enumerate() {
                if !time.is_finite() || index > 0 && curve.points[index - 1].0 >= *time {
                    return Err("unordered photo curve keys".into());
                }
                let valid = if curve.kind == "cubic" {
                    value.as_array().is_some_and(|v| {
                        v.len() == 4 && v.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))
                    })
                } else {
                    value.as_f64().is_some_and(f64::is_finite)
                };
                if !valid {
                    return Err("non-finite photo curve value".into());
                }
            }
        }
        if data.events.iter().any(|event| {
            event.function_name != "OnAction"
                || event.data != "PlayEffect"
                || !event.time.is_finite()
        }) {
            return Err("unsupported photo clip event".into());
        }
        let effect: serde_json::Value =
            serde_json::from_str(&effect.0).map_err(|e| e.to_string())?;
        let nodes = effect["components"]["UIParticle"]["instances"]
            .as_array()
            .ok_or("shot UIParticle missing")?;
        if nodes.len() != 1 {
            return Err("ambiguous shot UIParticle".into());
        }
        let node = nodes[0]["node"]
            .as_str()
            .ok_or("shot particle path missing")?
            .to_owned();
        let textures = effect["particles"]
            .as_array()
            .ok_or("shot particles missing")?
            .iter()
            .flat_map(|row| {
                row["renderer"]["material"]["textures"]
                    .as_object()
                    .into_iter()
                    .flat_map(|map| map.values())
            })
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("shot texture path missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((data, node, textures))
    })();
    match result {
        Ok((data, node, paths)) => {
            let root = world
                .resource::<UiLayouts>()
                .root()
                .expect("photo root")
                .trim_end_matches('/')
                .to_owned();
            let server = world.resource::<AssetServer>().clone();
            let mut images = paths
                .into_iter()
                .map(|path| {
                    moly_assets::residency::load_image(
                        &server,
                        format!("{root}/photo-shot/shot/{path}"),
                    )
                })
                .collect::<Vec<_>>();
            for frame in &data.frames {
                for (role, path) in [("preview", &frame.preview), ("thumbnail", &frame.thumbnail)] {
                    images.push(world.resource_mut::<UiLayouts>().register_runtime_texture(
                        &frame_alias(frame.id, role),
                        &format!("{root}/{path}"),
                        &server,
                    ));
                }
            }
            let mut chars = data
                .frames
                .iter()
                .flat_map(|frame| frame.name.chars())
                .collect::<Vec<_>>();
            for key in [
                "FORMAT_MYSEKAI_PHOTO_FILM_QUANTITY",
                "MSG_PHOTOSHOT_UNSAVE",
                "WORD_GO_BACK",
                "WORD_CANCEL",
            ] {
                chars.extend(world.resource::<UiLayouts>().wording(key).chars());
            }
            chars.push('1');
            chars.sort_unstable();
            chars.dedup();
            world.resource_mut::<PhotoCharset>().0 = Some(chars);
            let base = world
                .resource::<UiLayouts>()
                .document(KEY)
                .expect("photo document")
                .clone();
            let mut source = world.resource_mut::<Source>();
            source.data = Some(Arc::new(data));
            source.effect_node = Some(node);
            source.textures = images;
            source.base = Some(base);
        }
        Err(error) => report(world, error),
    }
}

fn frame_alias(id: i32, role: &str) -> String {
    format!("photo-frame-{id}-{role}")
}
fn bound_component<'a>(doc: &'a UiPrefab, selector: &str) -> Result<&'a UiComponent, String> {
    let id = selector
        .trim_start_matches('@')
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    doc.nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == id)
        .ok_or("photo component is missing".into())
}
fn loader_target(
    doc: &UiPrefab,
    fields: &serde_json::Value,
    field: &str,
) -> Result<String, String> {
    let loader = reference(doc, fields, field)?;
    reference(doc, &bound_component(doc, &loader)?.fields, "targetGraphic")
}
impl Bindings {
    fn read(doc: &UiPrefab) -> Result<Self, String> {
        let select = &component(doc, "Sekai.Mysekai.MysekaiSelectFrameView")?.fields;
        let animation = &component(doc, "Sekai.Mysekai.MysekaiPhotoShotAnimation")?.fields;
        let preview = &component(doc, "Sekai.UIPartsPhotoFramePreview")?.fields;
        let scroller = &bound_component(doc, &reference(doc, select, "_scroller")?)?.fields;
        let scroll = &bound_component(doc, &reference(doc, scroller, "scrollRect")?)?.fields;
        let split = scroller["layoutInfo"]["splitCount"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or("photo split count missing")? as usize;
        if scroll["m_MovementType"].as_i64() != Some(1) {
            return Err("photo frame scroll is not elastic".into());
        }
        if scroller["baseDirectionType"].as_i64() != Some(0) {
            return Err("photo frame list is not vertical".into());
        }
        let spacing =
            serde_json::from_value::<[f32; 2]>(scroller["layoutInfo"]["itemSpacing"].clone())
                .map_err(|e| e.to_string())?;
        let padding = serde_json::from_value::<[f32; 4]>(scroller["padding"].clone())
            .map_err(|e| e.to_string())?;
        let frame = loader_target(doc, preview, "_frameLoader")?;
        let frame_group = reference(doc, preview, "_frameCanvasGroup")?;
        if doc.find(&frame).map_err(|e| e.to_string())?
            != doc.find(&frame_group).map_err(|e| e.to_string())?
        {
            return Err("photo frame and canvas group are on different nodes".into());
        }
        Ok(Self {
            save: reference(doc, select, "_savePhotoButton")?,
            skip: reference(doc, select, "_photoShotSkipButton")?,
            name: reference(doc, select, "_selectedPhotoFrameNameText")?,
            photo: loader_target(doc, preview, "_photoLoader")?,
            frame,
            shadow: reference(doc, animation, "_photoFrameShadowCanvasGroup")?,
            fx_root: reference(doc, animation, "_fxPhotoShotRoot")?,
            content: reference(doc, scroll, "m_Content")?,
            viewport: reference(doc, scroll, "m_Viewport")?,
            sensitivity: scroll["m_ScrollSensitivity"]
                .as_f64()
                .ok_or("photo scroll sensitivity missing")? as f32,
            elasticity: scroll["m_Elasticity"]
                .as_f64()
                .ok_or("photo scroll elasticity missing")? as f32,
            deceleration: scroll["m_DecelerationRate"]
                .as_f64()
                .ok_or("photo scroll deceleration missing")? as f32,
            inertia: scroll["m_Inertia"]
                .as_bool()
                .ok_or("photo scroll inertia missing")?,
            spacing: Vec2::from_array(spacing),
            padding,
            split,
        })
    }
}

/// Named local adapter input. The master inventory never grants possession.
/// A host may supply this field in its normal NPC panel document.
fn owned_frames(world: &World) -> Result<Vec<i32>, String> {
    let Some(text) = world
        .get_resource::<crate::server_panel_data::ClientNpcPanel>()
        .and_then(|panel| panel.text.as_deref())
    else {
        return Ok(Vec::new());
    };
    let document: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(adapter) = document.get("photoLocalAdapter") else {
        return Ok(Vec::new());
    };
    let mut ids = adapter["ownedFrameIds"]
        .as_array()
        .ok_or("PhotoLocalAdapter.ownedFrameIds must be an array")?
        .iter()
        .map(|id| {
            id.as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .filter(|id| *id > 0)
                .ok_or("invalid owned photo frame ID".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate owned photo frame ID".into());
    }
    Ok(ids)
}

fn begin(world: &mut World) -> Result<(), String> {
    let source = world.resource::<Source>();
    let data = source
        .data
        .as_ref()
        .ok_or("photo presentation is loading")?
        .clone();
    if source.textures.iter().any(|handle| {
        !world
            .resource::<AssetServer>()
            .is_loaded_with_dependencies(handle)
    }) {
        return Err("photo frame textures are loading".into());
    }
    let mut document = source
        .base
        .as_ref()
        .ok_or("photo source document missing")?
        .clone();
    let bindings = Bindings::read(&document)?;
    let owned = owned_frames(world)?;
    if owned
        .iter()
        .any(|id| !data.frames.iter().any(|frame| frame.id == *id))
    {
        return Err("owned photo frame is absent from source inventory".into());
    }
    let template = world
        .resource::<UiLayouts>()
        .document("PhotoFrameCell")
        .ok_or("photo frame template missing")?;
    let fields = &component(template, "Sekai.UIPartsPhotoFrameThumbnail")?.fields;
    let image = loader_target(template, fields, "_frameLoader")?
        .trim_start_matches('@')
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    let field_id = |name| -> Result<i64, String> {
        reference(template, fields, name)?
            .trim_start_matches('@')
            .parse::<i64>()
            .map_err(|e| e.to_string())
    };
    let button = field_id("_selectedButton")?;
    let selected = field_id("_selectedImage")?;
    let quantity = field_id("_quantityText")?;
    let rect = &template.nodes[0].rect;
    let size = Vec2::from_array(rect.size_delta);
    let origin = Vec2::from_array(rect.anchored_position);
    let names = owned
        .iter()
        .map(|id| format!("Frame{id}"))
        .collect::<Vec<_>>();
    let instances = document.instantiate_children(
        &bindings.content,
        template,
        &template.prefab,
        names.iter().map(String::as_str),
    )?;
    let mut cells = Vec::new();
    for (index, (instance, id)) in instances.into_iter().zip(owned).enumerate() {
        let column = (index % bindings.split) as f32;
        let row = (index / bindings.split) as f32;
        let position = origin
            + Vec2::new(
                column * (size.x + bindings.spacing.x) + bindings.padding[0],
                -row * (size.y + bindings.spacing.y) - bindings.padding[2],
            );
        cells.push(Cell {
            id,
            root: instance.root_selector(),
            button: instance.selector(button)?,
            image: instance.selector(image)?,
            quantity: instance.selector(quantity)?,
            selected: instance.identity(selected)?,
            position,
        });
    }
    let rows = cells.len().div_ceil(bindings.split);
    let content_height = size.y * (rows.max(1) as f32) + bindings.spacing.y * rows as f32
        - origin.y
        + bindings.padding[2]
        + bindings.padding[3];
    let selected = if world.resource::<PhotoState>().previous_frame != 0 {
        world.resource::<PhotoState>().previous_frame
    } else {
        cells.first().map_or(0, |cell| cell.id)
    };
    let server = world.resource::<AssetServer>().clone();
    world
        .resource_mut::<UiLayouts>()
        .replace_runtime_document(KEY, document, &server)?;
    let image = world
        .resource_mut::<PhotoState>()
        .pending
        .take()
        .ok_or("photo capture texture missing")?;
    let handle = if let Some(handle) = world.resource::<PhotoState>().image.as_ref() {
        let handle = handle.clone();
        world
            .resource_mut::<Assets<Image>>()
            .insert(handle.id(), image)
            .map_err(|e| e.to_string())?;
        handle
    } else {
        let handle = world.resource_mut::<Assets<Image>>().add(image);
        world
            .resource_mut::<UiLayouts>()
            .bind_runtime_texture(CAPTURE_TEXTURE, handle.clone());
        handle
    };
    let sound_owner = world.spawn_empty().id();
    world
        .resource_mut::<crate::audio::SeRequests>()
        .0
        .push(crate::audio::SeRequest {
            owner: Some(sound_owner),
            cue: "se_camera".into(),
            class: crate::audio::SeClass::Ui,
            source: "PhotoShotAnimation",
        });
    stop_call_effect(world);
    for mut view in world
        .query_filtered::<&mut UiPrefabView, With<PhotoView>>()
        .iter_mut(world)
    {
        view.clear_overrides();
    }
    let mut state = world.resource_mut::<PhotoState>();
    state.image = Some(handle);
    state.selection = Some(Selection {
        playing: true,
        header_fade: 0.,
        scroll: 0.,
        velocity: 0.,
        elapsed: 0.,
        frame_visible: false,
        selected,
        source: data,
        bindings,
        cells,
        content_height,
        effect: None,
        sound_owner,
    });
    state.last_error = None;
    Ok(())
}

pub(super) fn advance(world: &mut World, delta: f32) {
    if world.resource::<PhotoState>().pending.is_some()
        && world.contains_resource::<PhotoBindings>()
        && world.get_resource::<Source>().is_some_and(|source| {
            source.data.is_some()
                && source.textures.iter().all(|handle| {
                    world
                        .resource::<AssetServer>()
                        .is_loaded_with_dependencies(handle)
                })
        })
    {
        if let Err(error) = begin(world) {
            report(world, error);
        }
    }
    if let Some(dialog) = world.resource::<PhotoState>().discard {
        if let Some(ok) = crate::common_dialog::answer(world, dialog) {
            world.resource_mut::<PhotoState>().discard = None;
            if ok {
                dispose(world);
            }
        }
    }
    let mut stop = None;
    if let Some(selection) = world.resource_mut::<PhotoState>().selection.as_mut() {
        if !selection.playing {
            selection.header_fade = (selection.header_fade + delta).min(0.1);
        }
        if selection.playing {
            let previous = selection.elapsed;
            selection.elapsed = (previous + delta).min(selection.source.duration);
            let event = selection
                .source
                .events
                .iter()
                .any(|event| event.time > previous && event.time <= selection.elapsed);
            if event {
                selection.frame_visible = true;
            }
            if selection.elapsed >= selection.source.duration {
                selection.playing = false;
                stop = Some(selection.sound_owner);
            }
        }
    }
    let start_particle = world
        .resource::<PhotoState>()
        .selection
        .as_ref()
        .is_some_and(|selection| {
            selection.effect.is_none()
                && selection.source.curves.iter().any(|curve| {
                    matches!(curve.property, Property::Active)
                        && format!("@{}", selection.source.targets[curve.target].transform_id)
                            == selection.bindings.fx_root
                        && curve.value(selection.elapsed) != 0.
                })
        });
    if start_particle {
        if let Err(error) = start_effect(world) {
            report(world, error);
        }
    }
    if let Some(owner) = stop {
        crate::audio::dispose_scoped_se(world, owner);
    }
    let state = world.resource::<PhotoState>();
    let held = state.entered
        && (state.capture.is_some() || state.pending.is_some() || state.selection.is_some());
    world.resource_mut::<crate::camera::PhotoGestureHeld>().0 = held;
}

fn report(world: &mut World, error: String) {
    if world.resource::<PhotoState>().last_error.as_deref() != Some(&error) {
        error!("[photo] {error}");
    }
    world.resource_mut::<PhotoState>().last_error = Some(error);
}

pub(super) fn back(world: &mut World) -> Result<(), String> {
    if world.resource::<PhotoState>().selection.is_none() {
        world
            .resource_mut::<ScreenManager>()
            .back_ui_screen(false, "PhotoShot header BackUI");
        return Ok(());
    }
    let body = world
        .resource::<UiLayouts>()
        .wording("MSG_PHOTOSHOT_UNSAVE");
    let dialog = crate::common_dialog::open(
        world,
        "PhotoShot.ShowUnSaveDialog",
        body,
        crate::common_dialog::Buttons::Two {
            ok: "WORD_GO_BACK",
            cancel: "WORD_CANCEL",
        },
        None,
    )
    .ok_or("photo unsaved dialog could not open")?;
    world.resource_mut::<PhotoState>().discard = Some(dialog);
    Ok(())
}

pub(super) fn skip(world: &mut World) -> Result<(), String> {
    let owner = if let Some(selection) = world.resource_mut::<PhotoState>().selection.as_mut() {
        // PlayFromStartAsync cancellation writes normalizedTime=1; the caller
        // explicitly reveals the frame. Seeking does not replay crossed events.
        selection.elapsed = selection.source.duration;
        selection.playing = false;
        selection.frame_visible = true;
        Some(selection.sound_owner)
    } else {
        None
    };
    if let Some(owner) = owner {
        crate::audio::dispose_scoped_se(world, owner);
    }
    Ok(())
}

pub(super) fn choose(world: &mut World, id: i32) -> Result<(), String> {
    if let Some(selection) = world.resource_mut::<PhotoState>().selection.as_mut() {
        if !selection.playing && selection.cells.iter().any(|cell| cell.id == id) {
            selection.selected = id;
        }
    }
    Ok(())
}

pub(super) fn dispose(world: &mut World) {
    if let Some(capture) = world.resource_mut::<PhotoState>().capture.take() {
        if let Ok(mut camera) = world.get_entity_mut(capture.camera) {
            camera.insert(capture.previous);
        }
        if let Ok(mut ready) = world.resource::<CaptureReadiness>().0.lock() {
            *ready = None;
        }
    }
    if let Some(dialog) = world.resource_mut::<PhotoState>().discard.take() {
        crate::common_dialog::dismiss(world, dialog);
    }
    let selection = world.resource_mut::<PhotoState>().selection.take();
    if let Some(selection) = selection {
        crate::audio::dispose_scoped_se(world, selection.sound_owner);
        if let Ok(entity) = world.get_entity_mut(selection.sound_owner) {
            entity.despawn();
        }
        if let Some(effect) = selection.effect {
            if let Ok(entity) = world.get_entity_mut(effect) {
                entity.despawn();
            }
        }
    }
    world.resource_mut::<PhotoState>().pending = None;
    world.resource_mut::<crate::camera::PhotoGestureHeld>().0 = false;
    // The alias retains one image for the presenter's lifetime. Release the
    // pixels after Save/discard, then replace this same asset on the next shot.
    let handle = world.resource::<PhotoState>().image.clone();
    if let Some(handle) = handle {
        if let Some(image) = world.resource_mut::<Assets<Image>>().get_mut(&handle) {
            *image = Image::new_fill(
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0, 0, 0, 0],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
        }
    }
}

pub(super) fn tap(
    selection: &Selection,
    layouts: &UiLayouts,
    view: &UiPrefabView,
    position: Vec2,
    size: Vec2,
    actions: &mut MessageWriter<PhotoAction>,
    sounds: &mut crate::audio::SeRequests,
) -> bool {
    let hit = |path: &str| {
        view.rect(layouts, path, size)
            .is_some_and(|rect| rect.active && rect.contains(position))
    };
    let control = if selection.playing {
        hit(&selection.bindings.skip).then_some((&selection.bindings.skip, PhotoAction::Skip))
    } else if hit(&selection.bindings.save) {
        Some((&selection.bindings.save, PhotoAction::Save))
    } else if hit(&selection.bindings.viewport) {
        selection
            .cells
            .iter()
            .find(|cell| hit(&cell.button))
            .map(|cell| (&cell.button, PhotoAction::Frame(cell.id)))
    } else {
        None
    };
    if let Some((button, action)) = control {
        layouts.queue_button_se(sounds, KEY, button);
        actions.write(action);
        true
    } else {
        false
    }
}

#[derive(Default)]
struct ScrollDrag {
    active: bool,
    cursor: f32,
    content: f32,
}

fn scroll(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    mut state: ResMut<PhotoState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<UiLayouts>,
    views: Query<&UiPrefabView, With<PhotoView>>,
    time: Res<Time<Real>>,
    mut drag: Local<ScrollDrag>,
) {
    let events = gestures.read().copied().collect::<Vec<_>>();
    let wheels = wheel.read().copied().collect::<Vec<_>>();
    if state.discard.is_some() {
        drag.active = false;
        return;
    }
    let Some(selection) = state
        .selection
        .as_mut()
        .filter(|selection| !selection.playing)
    else {
        drag.active = false;
        return;
    };
    let (Ok(window), Some(canvas), Ok(view)) =
        (windows.single(), canvas.as_deref(), views.single())
    else {
        return;
    };
    let scale = canvas.scale(window);
    let size = canvas.size(window);
    let Some(viewport) = view.rect(&layouts, &selection.bindings.viewport, size) else {
        return;
    };
    if viewport.size.y <= 0. {
        return;
    }
    let limit = (selection.content_height - viewport.size.y).max(0.);
    let local = |point: Vec2| {
        Vec2::new(
            point.x - window.width() / 2.,
            window.height() / 2. - point.y,
        ) / scale
    };
    let hit = |point: Vec2| viewport.active && viewport.contains(local(point));
    let previous = selection.scroll;
    let mut scrolling = false;
    if window.cursor_position().is_some_and(hit) {
        for event in wheels {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            selection.scroll -= delta * selection.bindings.sensitivity;
            scrolling = true;
        }
    }
    for event in events {
        if event.kind != crate::gesture::GestureKind::Drag {
            continue;
        }
        match event.state {
            GestureState::Began => {
                drag.active = hit(event.position);
                drag.cursor = local(event.position).y;
                drag.content = selection.scroll;
                selection.velocity = 0.;
            }
            GestureState::Moved if drag.active => {
                let position = drag.content + local(event.position).y - drag.cursor;
                let offset = position.clamp(0., limit) - position;
                let rubber = (1. - 1. / (offset.abs() * 0.55 / viewport.size.y + 1.))
                    * viewport.size.y
                    * offset.signum();
                selection.scroll = position + offset - rubber;
            }
            GestureState::End => drag.active = false,
            GestureState::Moved => {}
        }
    }
    let delta = time.delta_secs();
    if delta <= 0. {
        return;
    }
    if drag.active && selection.bindings.inertia {
        selection.velocity += ((selection.scroll - previous) / delta - selection.velocity)
            * (delta * 10.).clamp(0., 1.);
    } else if !drag.active {
        let target = selection.scroll.clamp(0., limit);
        if target != selection.scroll {
            let elasticity = selection.bindings.elasticity * if scrolling { 3. } else { 1. };
            selection.scroll = smooth_damp(
                selection.scroll,
                target,
                &mut selection.velocity,
                elasticity,
                delta,
            );
            if selection.velocity.abs() < 1. {
                selection.velocity = 0.;
            }
        } else if selection.bindings.inertia {
            selection.velocity *= selection.bindings.deceleration.powf(delta);
            if selection.velocity.abs() < 1. {
                selection.velocity = 0.;
            }
            selection.scroll += selection.velocity * delta;
        } else {
            selection.velocity = 0.;
        }
    }
}

// ScrollRect's elastic return uses Mathf.SmoothDamp with unlimited maxSpeed.
fn smooth_damp(current: f32, target: f32, velocity: &mut f32, smooth: f32, delta: f32) -> f32 {
    let omega = 2. / smooth.max(0.0001);
    let x = omega * delta;
    let decay = 1. / (1. + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = current - target;
    let temporary = (*velocity + omega * change) * delta;
    *velocity = (*velocity - omega * temporary) * decay;
    let value = target + (change + temporary) * decay;
    if (target - current > 0.) == (value > target) {
        *velocity = 0.;
        target
    } else {
        value
    }
}

fn start_effect(world: &mut World) -> Result<(), String> {
    let source = world.resource::<Source>();
    let document = source.effect.clone();
    let node = source
        .effect_node
        .as_ref()
        .ok_or("shot particle is loading")?
        .clone();
    let directory = format!(
        "{}/photo-shot/shot",
        world
            .resource::<UiLayouts>()
            .root()
            .ok_or("photo root missing")?
            .trim_start_matches("moly://")
            .trim_end_matches('/')
    );
    let canvas = world
        .query_filtered::<Entity, With<PhotoView>>()
        .single(world)
        .map_err(|_| "photo canvas missing")?;
    let host = world
        .spawn((
            Transform::default(),
            Visibility::Inherited,
            crate::ui_particle::UiParticlePlay,
            crate::ui_particle::UiParticleHost {
                document,
                directory,
                root: "fx_mysekai_photo_shot_pt".into(),
                node,
                canvas,
                layer: crate::sitemap::SITEMAP_LAYER,
                alpha: 1.,
            },
        ))
        .id();
    world.entity_mut(canvas).add_child(host);
    if let Some(selection) = world.resource_mut::<PhotoState>().selection.as_mut() {
        selection.effect = Some(host);
    }
    Ok(())
}

fn place(
    mut state: ResMut<PhotoState>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut views: Query<&mut UiPrefabView, With<PhotoView>>,
    mut effects: Query<(&mut Transform, &mut Visibility), With<crate::ui_particle::UiParticleHost>>,
) {
    let Some(selection) = state.selection.as_mut() else {
        return;
    };
    let (Ok(window), Some(canvas), Ok(mut view)) =
        (windows.single(), canvas.as_deref(), views.single_mut())
    else {
        return;
    };
    let size = canvas.size(window);
    let elapsed = selection.elapsed;
    let data = &selection.source;
    for (index, target) in data.targets.iter().enumerate() {
        let mut position = target.position;
        let mut scale = target.scale;
        let mut euler = [0.; 3];
        let mut color = target.color;
        let mut has_euler = false;
        let mut has_color = false;
        let path = format!("@{}", target.selector_id);
        for curve in data.curves.iter().filter(|curve| curve.target == index) {
            let value = curve.value(elapsed);
            match curve.property {
                Property::Position => position[curve.channel] = value,
                Property::Scale => scale[curve.channel] = value,
                Property::Euler => {
                    euler[curve.channel] = value;
                    has_euler = true;
                }
                Property::Color => {
                    color[curve.channel] = value;
                    has_color = true;
                }
                Property::Alpha => view.set_alpha(&path, value),
                Property::Active => view.set_visible(&path, value != 0.),
            }
        }
        if has_euler {
            view.set_transform(
                &path,
                Vec2::from_array(position),
                Quat::from_array(moly_law::animator::euler_zxy_degrees_to_quaternion(euler)),
            );
        } else {
            view.set_anchored_position(&path, Vec2::from_array(position));
        }
        view.set_local_scale(&path, Vec3::from_array(scale));
        if has_color {
            view.set_graphic_color(target.graphic_id.expect("validated graphic binding"), color);
        }
    }
    let bindings = &selection.bindings;
    view.set_alpha(SELECT_ROOT, 1.);
    view.set_visible(&bindings.skip, selection.playing);
    view.set_texture(&bindings.photo, CAPTURE_TEXTURE);
    view.set_behaviour_enabled(
        bindings
            .photo
            .trim_start_matches('@')
            .parse()
            .expect("photo graphic identity"),
        true,
    );
    let frame = selection
        .source
        .frames
        .iter()
        .find(|frame| frame.id == selection.selected);
    view.set_text(
        &bindings.name,
        frame.map_or_else(String::new, |frame| frame.name.clone()),
    );
    if let Some(frame) = frame {
        view.set_texture(&bindings.frame, &frame_alias(frame.id, "preview"));
    }
    let show = selection.frame_visible && !selection.cells.is_empty() && frame.is_some();
    view.set_alpha(&bindings.frame, if show { 1. } else { 0. });
    view.set_alpha(&bindings.shadow, if show { 1. } else { 0. });
    let quantity = layouts
        .wording("FORMAT_MYSEKAI_PHOTO_FILM_QUANTITY")
        .replace("{0}", "1");
    for cell in &selection.cells {
        view.set_visible(&cell.root, true);
        view.set_anchored_position(&cell.root, cell.position);
        view.set_texture(&cell.image, &frame_alias(cell.id, "thumbnail"));
        view.set_text(&cell.quantity, quantity.clone());
        view.set_behaviour_enabled(cell.selected, cell.id == selection.selected);
    }
    view.set_size_delta(&bindings.content, Vec2::new(0., selection.content_height));
    view.set_anchored_position(&bindings.content, Vec2::new(0., selection.scroll));
    let animation = &component(
        layouts.document(KEY).expect("photo document"),
        "Sekai.Mysekai.MysekaiPhotoShotAnimation",
    )
    .expect("photo animation bindings")
    .fields;
    for pointer in animation["_photoCameraImages"]
        .as_array()
        .expect("photo camera images")
    {
        let path = format!("@{}", pointer[1].as_i64().expect("photo camera image id"));
        view.set_texture(&path, "tex_mysekai_photo_shot_camera");
    }
    for (field, texture) in [
        ("_photoFramePartsImage", "tex_mysekai_photo_shot_parts"),
        ("_photoFilmImage", "img_film_default"),
    ] {
        view.set_texture(
            &reference(
                layouts.document(KEY).expect("photo document"),
                animation,
                field,
            )
            .expect("photo texture binding"),
            texture,
        );
    }
    if let Some(effect) = selection.effect {
        if let Some(rect) = view.rect(&layouts, &bindings.fx_root, size) {
            let position = rect.world.transform_point3(Vec3::ZERO);
            let depth = layouts
                .document(KEY)
                .expect("photo document")
                .find(&bindings.fx_root)
                .expect("photo effect root") as f32
                * 0.02
                + 20.019;
            if let Ok((mut transform, mut visibility)) = effects.get_mut(effect) {
                transform.translation = Vec3::new(position.x, position.y, depth);
                *visibility = if rect.active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

pub(super) fn save(world: &mut World) -> Result<(), String> {
    let state = world.resource::<PhotoState>();
    let Some(selection) = state
        .selection
        .as_ref()
        .filter(|selection| !selection.playing)
    else {
        return Ok(());
    };
    let frame_id = selection.selected;
    let image = state
        .image
        .as_ref()
        .and_then(|handle| world.resource::<Assets<Image>>().get(handle))
        .ok_or("photo texture missing")?;
    let image = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| e.to_string())?
        .to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 75)
        .encode_image(&image)
        .map_err(|e| e.to_string())?;
    let metadata=serde_json::to_vec_pretty(&serde_json::json!({"schemaVersion":1,"adapter":"PhotoLocalAdapter",
        "image":"photo.jpg","photoFrameId":frame_id,"width":image.width(),"height":image.height(),"jpegQuality":75,
        "albumServerResult":null,"filmConsumed":false})).map_err(|e|e.to_string())?;
    let bytes = archive(&jpeg, &metadata)?;
    export(&bytes)?;
    info!(
        "[photo] PhotoLocalAdapter exported original JPEG and frame {frame_id}; no album request or film consumption"
    );
    world.resource_mut::<PhotoState>().previous_frame = frame_id;
    dispose(world);
    Ok(())
}

fn archive(jpeg: &[u8], metadata: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [("photo.jpg", jpeg), ("metadata.json", metadata)] {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    zip.finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn export(bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let directory = std::env::var_os("MOLY_SCREENSHOT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "screenshots".into());
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let output = directory.join(format!("mysekai-photo-{timestamp}.zip"));
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(output)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn export(bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::{JsCast, closure::Closure};
    let js_error = |error: wasm_bindgen::JsValue| format!("photo download: {error:?}");
    let window = web_sys::window().ok_or("photo browser window missing")?;
    let document = window.document().ok_or("photo browser document missing")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/zip");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(js_error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_error)?;
    let result = (|| {
        let anchor = document.create_element("a").map_err(js_error)?;
        anchor.set_attribute("href", &url).map_err(js_error)?;
        anchor
            .set_attribute(
                "download",
                &format!("mysekai-photo-{}.zip", js_sys::Date::now() as u64),
            )
            .map_err(js_error)?;
        let anchor = anchor
            .dyn_into::<web_sys::HtmlElement>()
            .map_err(|_| "photo download anchor unavailable".to_owned())?;
        document
            .body()
            .ok_or("photo document body missing")?
            .append_child(&anchor)
            .map_err(js_error)?;
        anchor.click();
        anchor.remove();
        Ok(())
    })();
    let revoke_url = url.clone();
    let revoke = Closure::once_into_js(move || {
        if let Err(error) = web_sys::Url::revoke_object_url(&revoke_url) {
            warn!("[photo] could not release download URL: {error:?}");
        }
    });
    if window
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 1000)
        .is_err()
    {
        web_sys::Url::revoke_object_url(&url).map_err(js_error)?;
    }
    result
}
