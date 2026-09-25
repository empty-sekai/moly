//! The cover's loading indicator: `LiveTransitioner`'s `LoadingContent`, one
//! `UIPartsLoadingCircleOutLine` of eight circles, each pulsing through its
//! own `TweenScale` and drawn over a dark outline disc (`UIPartsImageOutline`).
//!
//! Source behaviour: `LiveTransitioner.Play` (with `showsLoading` true on the
//! MySekai join) activates `LoadingContent` once the cover is white; each
//! `TweenScale` (start timing OnEnable) then sets its circle's scale to
//! `from` and starts a DOTween tween to `to` with the circle's animation
//! curve as ease, its own delay and endless Restart loops.
//! `LiveTransitioner.Finish` deactivates `LoadingContent` again.
//!
//! Product adaptation, named: in the source the indicator was switched on
//! in the previous scene and has run through the whole scene load, so its
//! phase on the MySekai scene's first frame is whatever that load took.
//! Here every tween starts when the indicator appears. The tween is created
//! in a coroutine callback, after DOTween's update of that frame, so its
//! first update is on the next frame; that order is kept.
//!
//! Named gaps: the circles' colour comes from the uPalette asset (entry
//! `base_wh`, written by `UpdateView` and by each circle's
//! `GraphicColorSynchronizer`), and the pack carries no palette document;
//! [`BASE_WH`] stands in for it. Whether `UIPartsImageOutline.Update` runs
//! before or after DOTween's update in a frame is not fixed by the source
//! (script order); here the outline reads the tween value of the same frame.

use std::collections::HashMap;

use bevy::{
    camera::visibility::RenderLayers, image::ImageLoaderSettings, prelude::*, window::PrimaryWindow,
};
use moly_assets::json::JsonAsset;
use moly_law::particle::{curve::eval_curve, value::CurveKey};
use serde_json::Value;

/// The extracted `LiveTransitioner` prefab (Resources
/// `Common/Transition/LiveTransitioner`) in the asset root.
const DOCUMENT_DIR: &str = "live-transitioner/LiveTransitioner";
const DOCUMENT: &str = "LiveTransitioner.json";
const PREFAB_ROOT: &str = "LiveTransitioner";
const INDICATOR_NODE: &str = "ContentRoot/ButtomRight/LoadingContent";
/// `UIPartsLoadingCircle.UpdateView` writes this sprite name into every
/// circle image (a string literal of the method).
const CIRCLE_SPRITE: &str = "bg_base_circle_h96_wh";
/// `UIPartsLoadingCircle.UpdateView`: `colorType` indexes this table of
/// palette entries (`ColorEntry` base_gn, base_dbl, base_wh, base_gr); a
/// type above 3 takes entry 2.
const COLOR_TYPE_ENTRIES: [u32; 4] = [3, 0, 2, 8];
const BASE_WH_ENTRY: u32 = 2;
/// The palette asset's `base_wh` in its active theme: a named mock of the
/// palette document the pack does not carry.
const BASE_WH: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// Same overlay layer as the cover; the indicator sorts above the quad
/// (`ContentRoot` draws `Cover` before `ButtomRight`).
const LAYER: usize = 29;

/// The fields of one `RectTransform` the placement reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RectFields {
    pub anchor_min: Vec2,
    pub anchor_max: Vec2,
    pub anchored: Vec2,
    pub size_delta: Vec2,
    pub pivot: Vec2,
    pub scale: Vec2,
}

/// A child `RectTransform` inside its parent's rect: the child's pivot in
/// the parent's local space (origin at the parent's pivot) and its size.
/// The anchor reference point is `lerp(anchorMin, anchorMax, pivot)` of the
/// parent rect, and `anchoredPosition` is the pivot's offset from it.
pub(super) fn place(parent_size: Vec2, parent_pivot: Vec2, child: &RectFields) -> (Vec2, Vec2) {
    let min = -parent_pivot * parent_size;
    let a0 = min + child.anchor_min * parent_size;
    let a1 = min + child.anchor_max * parent_size;
    let size = (a1 - a0) + child.size_delta;
    let pivot = a0 + (a1 - a0) * child.pivot + child.anchored;
    (pivot, size)
}

/// `CanvasScaler` in Scale With Screen Size / Match Width Or Height:
/// `2 ^ lerp(log2(w / refW), log2(h / refH), match)`.
pub(super) fn canvas_scale_factor(screen: Vec2, reference: Vec2, matching: f32) -> f32 {
    let log_w = (f64::from(screen.x) / f64::from(reference.x)).log2() as f32;
    let log_h = (f64::from(screen.y) / f64::from(reference.y)).log2() as f32;
    let t = matching.clamp(0.0, 1.0);
    2f64.powf(f64::from(log_w + (log_h - log_w) * t)) as f32
}

/// `UIPartsImageOutline.Update`: the outline takes the target's anchored
/// position, and `sizeDelta * localScale + outline` per axis as its size.
pub(super) fn outline_size(target_size: Vec2, target_scale: Vec2, outline: f32) -> Vec2 {
    Vec2::new(
        outline + target_size.x * target_scale.x,
        target_size.y * target_scale.y + outline,
    )
}

/// One `TweenScale` as `TweenBase.SetTweenOptionCommon` configures it:
/// ease = the animation curve, `SetDelay(delay)`, loops -1 with Restart.
#[derive(Clone, Debug)]
pub(super) struct TweenScaleLaw {
    pub from: Vec3,
    pub to: Vec3,
    pub delay: f32,
    pub duration: f32,
    pub keys: Vec<CurveKey>,
}

/// The DOTween tweener of one circle.
#[derive(Clone, Debug)]
pub(super) struct ScaleTween {
    law: TweenScaleLaw,
    delay_complete: bool,
    elapsed_delay: f32,
    startup_done: bool,
    start: Vec3,
    change: Vec3,
    position: f32,
    completed_loops: i32,
    value: Vec3,
}

impl ScaleTween {
    /// `TweenScale.PlayCore(Forward)`: the target's scale is set to `from`
    /// and the tweener is created; `SetDelay` marks the delay complete when
    /// it is not above 0.
    pub(super) fn play(law: TweenScaleLaw) -> Self {
        Self {
            delay_complete: law.delay <= 0.0,
            elapsed_delay: 0.0,
            startup_done: false,
            start: law.from,
            change: Vec3::ZERO,
            position: 0.0,
            completed_loops: 0,
            value: law.from,
            law,
        }
    }

    pub(super) fn value(&self) -> Vec3 {
        self.value
    }

    /// One frame of `TweenManager.Update` for this tweener (Normal update,
    /// time scale 1, endless loops). Returns whether a value was applied.
    pub(super) fn update(&mut self, dt: f32) -> bool {
        let mut delta = dt;
        if !self.delay_complete {
            // Tweener.DoUpdateDelay(elapsedDelay + delta): complete only once
            // the elapsed time exceeds the delay; the excess carries on.
            let elapsed = self.elapsed_delay + delta;
            if self.law.delay < elapsed {
                self.elapsed_delay = self.law.delay;
                self.delay_complete = true;
                delta = elapsed - self.law.delay;
            } else {
                self.elapsed_delay = elapsed;
                delta = 0.0;
            }
            if delta <= 0.0 {
                return false;
            }
        }
        if !self.startup_done {
            // DoStartup: the start value is the getter's (the scale PlayCore
            // set), the change is `to - start`.
            self.startup_done = true;
            self.start = self.value;
            self.change = self.law.to - self.start;
        }
        let duration = self.law.duration;
        let mut to_position = delta + self.position;
        let mut loops = self.completed_loops;
        while duration <= to_position {
            to_position -= duration;
            loops += 1;
        }
        if duration <= self.position {
            loops -= 1;
        }
        // Tween.DoGoto: a position at or below 0 after a completed loop is
        // the loop's end; above the duration it is clamped.
        self.completed_loops = loops;
        self.position = if to_position <= duration {
            if to_position > 0.0 {
                to_position
            } else if loops > 0 {
                duration
            } else {
                0.0
            }
        } else {
            duration
        };
        // EaseCurve.Evaluate: `curve.Evaluate(time / duration * lastKeyTime)`;
        // Vector3Plugin: `start + change * ease` per axis.
        let last = self.law.keys.last().map_or(0.0, |key| key.time);
        let ease = eval_curve(&self.law.keys, (self.position / duration) * last);
        self.value = self.start + self.change * ease;
        true
    }
}

#[derive(Component)]
pub(crate) struct EntryIndicatorPart;

struct Circle {
    name: String,
    rect: RectFields,
    tween: ScaleTween,
    entity: Entity,
    outline: Option<Outline>,
}

struct Outline {
    rect: RectFields,
    width: f32,
    entity: Entity,
}

struct Running {
    /// `ContentRoot` down to `CircleRoot`.
    chain: Vec<RectFields>,
    reference: Vec2,
    matching: f32,
    circles: Vec<Circle>,
    updates: u32,
}

enum State {
    Requested(Handle<JsonAsset>),
    /// Created this frame; DOTween's first update is on the next frame.
    Created(Running),
    Running(Running),
    Off,
}

#[derive(Resource)]
pub(crate) struct EntryIndicator(State);

/// Startup: request the prefab document with the cover.
pub(crate) fn request(mut commands: Commands, server: Res<AssetServer>) {
    let handle = server.load::<JsonAsset>(format!("moly://{DOCUMENT_DIR}/{DOCUMENT}"));
    commands.insert_resource(EntryIndicator(State::Requested(handle)));
}

/// `LiveTransitioner.Finish`: `loadingContent.SetActive(false)`; the
/// tweens die with it (`dontKillIfDisable` is off).
pub(crate) fn hide(world: &mut World) {
    let previous = {
        let Some(mut indicator) = world.get_resource_mut::<EntryIndicator>() else {
            return;
        };
        std::mem::replace(&mut indicator.0, State::Off)
    };
    if let State::Created(running) | State::Running(running) = previous {
        info!(
            "[entry-indicator] LoadingContent off after {} tween updates",
            running.updates
        );
        let mut parts = world.query_filtered::<Entity, With<EntryIndicatorPart>>();
        let parts: Vec<_> = parts.iter(world).collect();
        for part in parts {
            world.despawn(part);
        }
    }
}

/// One frame while the cover is opaque.
pub(crate) fn advance(world: &mut World, dt: f32) {
    let Some(mut indicator) = world.remove_resource::<EntryIndicator>() else {
        return;
    };
    indicator.0 = match std::mem::replace(&mut indicator.0, State::Off) {
        State::Requested(handle) => requested(world, handle),
        State::Created(running) => {
            let mut running = running;
            step(world, &mut running, dt);
            State::Running(running)
        }
        State::Running(mut running) => {
            step(world, &mut running, dt);
            State::Running(running)
        }
        State::Off => State::Off,
    };
    world.insert_resource(indicator);
}

fn requested(world: &mut World, handle: Handle<JsonAsset>) -> State {
    let load = world.resource::<AssetServer>().load_state(&handle);
    match load {
        bevy::asset::LoadState::Loaded => {}
        bevy::asset::LoadState::Failed(_) => {
            warn!(
                "[entry-indicator] {DOCUMENT_DIR}/{DOCUMENT} is not in the asset root: \
                 the loading indicator is not drawn"
            );
            return State::Off;
        }
        _ => return State::Requested(handle),
    }
    let text = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|asset| asset.0.clone());
    let parsed = text
        .ok_or_else(|| "the loaded document is not in the asset table".to_owned())
        .and_then(|text| {
            serde_json::from_str::<Value>(&text).map_err(|error| format!("not JSON: {error}"))
        })
        .and_then(|doc| Indicator::read(&doc));
    match parsed {
        Ok(read) => State::Created(spawn(world, read)),
        Err(error) => {
            error!("[entry-indicator] {DOCUMENT_DIR}/{DOCUMENT}: {error}; the loading indicator is not drawn");
            State::Off
        }
    }
}

/// What the document says, checked against the subset ported here.
struct Indicator {
    chain: Vec<RectFields>,
    reference: Vec2,
    matching: f32,
    alpha: f32,
    /// In `CircleRoot`'s child order: (sibling index, circle).
    circles: Vec<ReadCircle>,
    textures: HashMap<String, (String, bool)>,
}

struct ReadCircle {
    name: String,
    sibling: usize,
    rect: RectFields,
    law: TweenScaleLaw,
    colour: [f32; 4],
    sprite: String,
    outline: Option<ReadOutline>,
}

struct ReadOutline {
    sibling: usize,
    rect: RectFields,
    width: f32,
    colour: [f32; 4],
    sprite: String,
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what} is not a finite number"))
}

fn vec2(value: &Value, what: &str) -> Result<Vec2, String> {
    Ok(Vec2::new(
        number(&value["x"], what)?,
        number(&value["y"], what)?,
    ))
}

fn vec3(value: &Value, what: &str) -> Result<Vec3, String> {
    Ok(Vec3::new(
        number(&value["x"], what)?,
        number(&value["y"], what)?,
        number(&value["z"], what)?,
    ))
}

fn colour(value: &Value, what: &str) -> Result<[f32; 4], String> {
    Ok([
        number(&value["r"], what)?,
        number(&value["g"], what)?,
        number(&value["b"], what)?,
        number(&value["a"], what)?,
    ])
}

fn path_id(value: &Value) -> Option<String> {
    match &value["pathId"] {
        Value::String(id) => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

struct Prefab<'a> {
    doc: &'a Value,
    /// Component path id -> node path, within the prefab root.
    nodes: HashMap<String, String>,
    active: HashMap<String, bool>,
}

impl<'a> Prefab<'a> {
    fn new(doc: &'a Value) -> Result<Self, String> {
        let mut nodes = HashMap::new();
        let mut active = HashMap::new();
        for row in doc["nodeTable"].as_array().ok_or("no nodeTable")? {
            if row["root"].as_str() != Some(PREFAB_ROOT) {
                continue;
            }
            let node = row["node"].as_str().ok_or("nodeTable row without a node")?;
            active.insert(
                node.to_owned(),
                row["active"]
                    .as_bool()
                    .ok_or("nodeTable row without active")?,
            );
            for component in row["components"]
                .as_array()
                .ok_or("nodeTable row without components")?
            {
                let id = path_id(component).ok_or("nodeTable component without a path id")?;
                nodes.insert(id, node.to_owned());
            }
        }
        Ok(Self { doc, nodes, active })
    }

    fn node_of(&self, pointer: &Value) -> Result<&str, String> {
        let id = path_id(pointer).ok_or("a pointer without a path id")?;
        self.nodes
            .get(&id)
            .map(String::as_str)
            .ok_or_else(|| format!("path id {id} is not a component of the prefab"))
    }

    /// The single enabled instance of `class` on `node`.
    fn fields(&self, class: &str, node: &str) -> Result<&'a Value, String> {
        let found: Vec<&Value> = self.doc["components"][class]["instances"]
            .as_array()
            .map(|all| {
                all.iter()
                    .filter(|i| i["node"].as_str() == Some(node))
                    .collect()
            })
            .unwrap_or_default();
        let [instance] = found.as_slice() else {
            return Err(format!(
                "{} {class} instances on {node:?}, expected one",
                found.len()
            ));
        };
        let instance: &'a Value = *instance;
        let fields = &instance["fields"];
        if fields
            .get("m_Enabled")
            .is_some_and(|enabled| enabled.as_i64() != Some(1))
        {
            return Err(format!("{class} on {node:?} is disabled"));
        }
        Ok(fields)
    }

    fn rect(&self, node: &str) -> Result<RectFields, String> {
        let f = self.fields("RectTransform", node)?;
        let rotation = &f["m_LocalRotation"];
        let identity = ["x", "y", "z"]
            .iter()
            .all(|axis| rotation[*axis].as_f64() == Some(0.0))
            && rotation["w"].as_f64() == Some(1.0);
        if !identity {
            return Err(format!(
                "{node:?} is rotated; only unrotated rects are placed"
            ));
        }
        let scale = vec3(&f["m_LocalScale"], "m_LocalScale")?;
        Ok(RectFields {
            anchor_min: vec2(&f["m_AnchorMin"], "m_AnchorMin")?,
            anchor_max: vec2(&f["m_AnchorMax"], "m_AnchorMax")?,
            anchored: vec2(&f["m_AnchoredPosition"], "m_AnchoredPosition")?,
            size_delta: vec2(&f["m_SizeDelta"], "m_SizeDelta")?,
            pivot: vec2(&f["m_Pivot"], "m_Pivot")?,
            scale: scale.truncate(),
        })
    }

    fn group_alpha(&self, node: &str) -> Result<f32, String> {
        let count = self.doc["components"]["CanvasGroup"]["instances"]
            .as_array()
            .map_or(0, |all| {
                all.iter()
                    .filter(|i| i["node"].as_str() == Some(node))
                    .count()
            });
        if count == 0 {
            return Ok(1.0);
        }
        number(&self.fields("CanvasGroup", node)?["m_Alpha"], "m_Alpha")
    }

    fn image(&self, node: &str) -> Result<([f32; 4], String), String> {
        let f = self.fields("CustomImage", node)?;
        if f["m_Type"].as_i64() != Some(0) || !f["m_Material"].is_null() {
            return Err(format!(
                "{node:?} is not a simple image with the default material"
            ));
        }
        let sprite = f["spriteName"]
            .as_str()
            .ok_or("image without a sprite name")?;
        Ok((colour(&f["m_Color"], "m_Color")?, sprite.to_owned()))
    }
}

impl Indicator {
    fn read(doc: &Value) -> Result<Self, String> {
        let prefab = Prefab::new(doc)?;
        let canvas = prefab.fields("Canvas", "")?;
        if canvas["m_RenderMode"].as_i64() != Some(0) {
            return Err("the root canvas is not a screen-space overlay".into());
        }
        let scaler = prefab.fields("ScreenCanvasScaler", "")?;
        if scaler["m_UiScaleMode"].as_i64() != Some(1)
            || scaler["m_ScreenMatchMode"].as_i64() != Some(0)
        {
            return Err(
                "the canvas scaler is not Scale With Screen Size / Match Width Or Height".into(),
            );
        }
        let reference = vec2(&scaler["m_ReferenceResolution"], "m_ReferenceResolution")?;
        let matching = number(&scaler["m_MatchWidthOrHeight"], "m_MatchWidthOrHeight")?;

        let loading = prefab.fields(
            "UIPartsLoadingCircle",
            &format!("{INDICATOR_NODE}/UIPartsLoadingCircleOutLine"),
        )?;
        let circle_root = format!("{INDICATOR_NODE}/UIPartsLoadingCircleOutLine/CircleRoot");
        let color_type = loading["colorType"].as_i64().ok_or("colorType missing")?;
        let entry = usize::try_from(color_type)
            .ok()
            .and_then(|index| COLOR_TYPE_ENTRIES.get(index).copied())
            .unwrap_or(BASE_WH_ENTRY);
        if entry != BASE_WH_ENTRY {
            return Err(format!(
                "colorType {color_type} selects palette entry {entry}; only base_wh has a stand-in value"
            ));
        }

        // The chain from ContentRoot to CircleRoot; everything but
        // LoadingContent (activated by Play) must be active in the prefab.
        let mut chain = Vec::new();
        let mut alpha = prefab.group_alpha("")?;
        let mut path = String::new();
        for part in circle_root.split('/') {
            path = if path.is_empty() {
                part.to_owned()
            } else {
                format!("{path}/{part}")
            };
            let active = *prefab
                .active
                .get(&path)
                .ok_or_else(|| format!("{path:?} is not in the prefab"))?;
            if active != (path != INDICATOR_NODE) {
                return Err(format!("{path:?} has an unexpected active flag {active}"));
            }
            chain.push(prefab.rect(&path)?);
            alpha *= prefab.group_alpha(&path)?;
        }
        let children: Vec<String> = prefab.fields("RectTransform", &circle_root)?["m_Children"]
            .as_array()
            .ok_or("CircleRoot has no children list")?
            .iter()
            .map(|child| prefab.node_of(child).map(str::to_owned))
            .collect::<Result<_, _>>()?;
        let sibling = |node: &str| {
            children
                .iter()
                .position(|child| child == node)
                .ok_or_else(|| format!("{node:?} is not a child of CircleRoot"))
        };

        // Outline discs by the image they follow.
        let mut outlines = HashMap::new();
        for instance in doc["components"]["UIPartsImageOutline"]["instances"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let node = instance["node"].as_str().ok_or("outline without a node")?;
            let fields = prefab.fields("UIPartsImageOutline", node)?;
            let target = prefab.node_of(&fields["_targetImage"])?.to_owned();
            let (colour, sprite) = prefab.image(node)?;
            let rect = prefab.rect(node)?;
            let read = ReadOutline {
                sibling: sibling(node)?,
                rect,
                width: number(&fields["_outline"], "_outline")?,
                colour,
                sprite,
            };
            if outlines.insert(target, (node.to_owned(), read)).is_some() {
                return Err("two outlines follow one image".into());
            }
        }

        let images = loading["circleImages"]
            .as_array()
            .ok_or("circleImages missing")?;
        let tweens = loading["circleTweens"]
            .as_array()
            .ok_or("circleTweens missing")?;
        if images.len() != tweens.len() || images.is_empty() {
            return Err("circleImages and circleTweens differ in length".into());
        }
        let mut synchronizer_entry = None;
        let mut circles = Vec::new();
        for (image, tween) in images.iter().zip(tweens) {
            let node = prefab.node_of(image)?.to_owned();
            if prefab.node_of(tween)? != node {
                return Err(format!("{node:?}: its tween sits on another object"));
            }
            let (serialized, sprite) = prefab.image(&node)?;
            if sprite != CIRCLE_SPRITE {
                return Err(format!(
                    "{node:?} names sprite {sprite}, UpdateView writes {CIRCLE_SPRITE}"
                ));
            }
            if serialized != BASE_WH {
                return Err(format!(
                    "{node:?}: serialized colour {serialized:?} is not the palette stand-in"
                ));
            }
            let sync = prefab.fields("GraphicColorSynchronizer", &node)?;
            if prefab.node_of(&sync["_component"])? != node {
                return Err(format!(
                    "{node:?}: its colour synchronizer drives another graphic"
                ));
            }
            let entry = sync["_entryId"]["_value"]
                .as_str()
                .ok_or("synchronizer without an entry id")?;
            if synchronizer_entry.get_or_insert_with(|| entry.to_owned()) != entry {
                return Err(
                    "the circles' colour synchronizers name different palette entries".into(),
                );
            }
            let t = prefab.fields("TweenScale", &node)?;
            let checks = [
                ("behaviour", 0),
                ("componentType", 1),
                ("direction", 0),
                ("startTiming", 2),
                ("loopType", 1),
                ("loopCount", 0),
                ("syncGameTime", 0),
                ("dontKillIfDisable", 0),
                ("isReflesh", 0),
            ];
            for (field, wanted) in checks {
                if t[field].as_i64() != Some(wanted) {
                    return Err(format!(
                        "{node:?}: TweenScale {field} is {}, the port covers {wanted}",
                        t[field]
                    ));
                }
            }
            let curve = &t["curve"];
            if curve["m_PreInfinity"].as_i64() != Some(2)
                || curve["m_PostInfinity"].as_i64() != Some(2)
            {
                return Err(format!(
                    "{node:?}: the ease curve does not clamp at both ends"
                ));
            }
            let keys = curve["m_Curve"]
                .as_array()
                .ok_or("ease curve without keys")?
                .iter()
                .map(|key| {
                    Ok(CurveKey {
                        time: number(&key["time"], "key time")?,
                        value: number(&key["value"], "key value")?,
                        in_slope: number(&key["inSlope"], "key inSlope")?,
                        out_slope: number(&key["outSlope"], "key outSlope")?,
                        weighted_mode: u8::try_from(
                            key["weightedMode"].as_u64().ok_or("key weightedMode")?,
                        )
                        .map_err(|_| "key weightedMode out of range".to_owned())?,
                        in_weight: number(&key["inWeight"], "key inWeight")?,
                        out_weight: number(&key["outWeight"], "key outWeight")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if keys.is_empty() || keys.windows(2).any(|pair| pair[1].time < pair[0].time) {
                return Err(format!(
                    "{node:?}: the ease curve keys are empty or unordered"
                ));
            }
            let duration = number(&t["duration"], "duration")?;
            if duration <= 0.0 {
                return Err(format!("{node:?}: tween duration {duration}"));
            }
            let law = TweenScaleLaw {
                from: vec3(&t["from"], "from")?,
                to: vec3(&t["to"], "to")?,
                delay: number(&t["delay"], "delay")?,
                duration,
                keys,
            };
            let rect = prefab.rect(&node)?;
            let outline = outlines.remove(&node).map(|(_, read)| read);
            if let Some(outline) = &outline {
                if outline.rect.anchor_min != rect.anchor_min
                    || outline.rect.anchor_max != rect.anchor_max
                    || outline.rect.pivot != rect.pivot
                {
                    return Err(format!("{node:?}: its outline has other anchors or pivot"));
                }
            }
            circles.push(ReadCircle {
                name: node.rsplit('/').next().unwrap_or(&node).to_owned(),
                sibling: sibling(&node)?,
                rect,
                law,
                colour: BASE_WH,
                sprite,
                outline,
            });
        }
        if !outlines.is_empty() {
            return Err("an outline follows an image outside the circle list".into());
        }

        let mut textures = HashMap::new();
        for texture in doc["textures"].as_array().ok_or("no texture list")? {
            let file = texture.as_str().ok_or("texture entry is not a path")?;
            let srgb = doc["textureColourSpace"][file]
                .as_bool()
                .ok_or_else(|| format!("{file}: no colour space"))?;
            let stem = file.rsplit('/').next().unwrap_or(file);
            let stem = stem
                .strip_suffix(".png")
                .ok_or_else(|| format!("{file}: not a png"))?;
            if let Some((name, _)) = stem.rsplit_once('-') {
                if textures
                    .insert(name.to_owned(), (file.to_owned(), srgb))
                    .is_some()
                {
                    return Err(format!("two textures are named {name}"));
                }
            }
        }
        Ok(Self {
            chain,
            reference,
            matching,
            alpha,
            circles,
            textures,
        })
    }
}

fn sprite_image(
    world: &mut World,
    read: &Indicator,
    sprite: &str,
) -> Result<Handle<Image>, String> {
    let (file, srgb) = read
        .textures
        .get(sprite)
        .cloned()
        .ok_or_else(|| format!("no texture named {sprite} in the document"))?;
    Ok(moly_assets::residency::load_image_with(
        world.resource::<AssetServer>(),
        format!("moly://{DOCUMENT_DIR}/{file}"),
        move |settings: &mut ImageLoaderSettings| settings.is_srgb = srgb,
    ))
}

fn spawn_part(
    world: &mut World,
    image: Handle<Image>,
    colour: [f32; 4],
    alpha: f32,
    z: f32,
) -> Entity {
    world
        .spawn((
            Sprite {
                image,
                color: Color::srgba(colour[0], colour[1], colour[2], colour[3] * alpha),
                custom_size: Some(Vec2::ZERO),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, z),
            RenderLayers::layer(LAYER),
            EntryIndicatorPart,
        ))
        .id()
}

fn spawn(world: &mut World, read: Indicator) -> Running {
    let mut circles = Vec::new();
    let mut failures = Vec::new();
    // Sibling order is draw order: z 1 + index / 100 above the cover.
    let z = |sibling: usize| 1.0 + sibling as f32 / 100.0;
    for circle in &read.circles {
        let image = match sprite_image(world, &read, &circle.sprite) {
            Ok(image) => image,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        let entity = spawn_part(world, image, circle.colour, read.alpha, z(circle.sibling));
        let outline = match &circle.outline {
            Some(outline) => match sprite_image(world, &read, &outline.sprite) {
                Ok(image) => Some(Outline {
                    rect: outline.rect,
                    width: outline.width,
                    entity: spawn_part(
                        world,
                        image,
                        outline.colour,
                        read.alpha,
                        z(outline.sibling),
                    ),
                }),
                Err(error) => {
                    failures.push(error);
                    None
                }
            },
            None => None,
        };
        circles.push(Circle {
            name: circle.name.clone(),
            rect: circle.rect,
            tween: ScaleTween::play(circle.law.clone()),
            entity,
            outline,
        });
    }
    for failure in failures {
        error!("[entry-indicator] {failure}");
    }
    info!(
        "[entry-indicator] LoadingContent on: {} circles, {} outlines, canvas reference {}x{} match {}; tweens created (first update next frame)",
        circles.len(),
        circles.iter().filter(|c| c.outline.is_some()).count(),
        read.reference.x,
        read.reference.y,
        read.matching
    );
    let mut running = Running {
        chain: read.chain,
        reference: read.reference,
        matching: read.matching,
        circles,
        updates: 0,
    };
    layout(world, &mut running);
    running
}

/// Places every part for the current window size.
fn layout(world: &mut World, running: &mut Running) {
    let mut windows = world.query_filtered::<&Window, With<PrimaryWindow>>();
    let Ok(window) = windows.single(world) else {
        return;
    };
    let screen = Vec2::new(window.width(), window.height());
    if screen.x <= 0.0 || screen.y <= 0.0 {
        return;
    }
    let factor = canvas_scale_factor(screen, running.reference, running.matching);
    // The overlay canvas: centred on the screen, pivot at its centre.
    let mut origin = Vec2::ZERO;
    let mut accumulated = Vec2::ONE;
    let mut parent = (screen / factor, Vec2::splat(0.5));
    for node in &running.chain {
        let (pivot, size) = place(parent.0, parent.1, node);
        origin += accumulated * pivot;
        accumulated *= node.scale;
        parent = (size, node.pivot);
    }
    let mut placements = Vec::new();
    for circle in &running.circles {
        let scale = circle.tween.value().truncate();
        let mut rect = circle.rect;
        rect.scale = scale;
        placements.push((circle.entity, quad(origin, accumulated, parent, &rect)));
        if let Some(outline) = &circle.outline {
            let mut rect = outline.rect;
            rect.anchored = circle.rect.anchored;
            rect.size_delta = outline_size(circle.rect.size_delta, scale, outline.width);
            placements.push((outline.entity, quad(origin, accumulated, parent, &rect)));
        }
    }
    for (entity, (centre, size)) in placements {
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation.x = centre.x * factor;
            transform.translation.y = centre.y * factor;
        }
        if let Some(mut sprite) = world.get_mut::<Sprite>(entity) {
            sprite.custom_size = Some(size * factor);
        }
    }
}

/// A rect's drawn quad in canvas units: centre and size, under the parent
/// chain's accumulated scale and its own scale.
fn quad(origin: Vec2, accumulated: Vec2, parent: (Vec2, Vec2), rect: &RectFields) -> (Vec2, Vec2) {
    let (pivot, size) = place(parent.0, parent.1, rect);
    let own = size * rect.scale;
    let centre = origin + accumulated * (pivot + (Vec2::splat(0.5) - rect.pivot) * own);
    (centre, own * accumulated)
}

fn step(world: &mut World, running: &mut Running, dt: f32) {
    let mut applied = 0;
    for circle in &mut running.circles {
        if circle.tween.update(dt) {
            applied += 1;
        }
    }
    running.updates += 1;
    layout(world, running);
    let scales: Vec<String> = running
        .circles
        .iter()
        .map(|c| format!("{}={:.4}", c.name, c.tween.value().x))
        .collect();
    info!(
        "[entry-indicator] update {} dt={dt:.4} applied={applied} {}",
        running.updates,
        scales.join(" ")
    );
}

#[cfg(test)]
mod tests {
    //! Research instruments: expected values come from an independent
    //! float32 recomputation of DOTween's update (delay, Restart loops,
    //! `DoGoto` clamp, curve ease) and the keyframe Hermite, fed with the
    //! extracted curve; the placement values are worked by hand from the
    //! extracted rects.
    use super::*;

    fn keys() -> Vec<CurveKey> {
        // The extracted TweenScale curve (all eight circles share it),
        // verbatim; weightedMode 0, so the stored weights are not read.
        let key = |time: f32, value: f32, slope: f32, in_weight: f32, out_weight: f32| CurveKey {
            time,
            value,
            in_slope: slope,
            out_slope: slope,
            weighted_mode: 0,
            in_weight,
            out_weight,
        };
        vec![
            key(0.0, 0.0, 0.0, 0.0, 0.079_673_864),
            key(
                0.103_379_32,
                0.997_887_97,
                0.011_925_893,
                0.333_333_34,
                0.378_937_66,
            ),
            key(
                0.200_774_1,
                0.002_552_330_5,
                0.0,
                0.333_333_34,
                0.333_333_34,
            ),
            key(0.979_156_5, -0.002_357_483, 0.0, 0.063_912_91, 0.0),
        ]
    }

    fn law(delay: f32) -> TweenScaleLaw {
        TweenScaleLaw {
            from: Vec3::new(0.666_666_7, 0.666_666_7, 1.0),
            to: Vec3::ONE,
            delay,
            duration: 1.0,
            keys: keys(),
        }
    }

    fn run(delay: f32, dts: &[f32]) -> Vec<Option<f32>> {
        let mut tween = ScaleTween::play(law(delay));
        dts.iter()
            .map(|dt| tween.update(*dt).then(|| tween.value().x))
            .collect()
    }

    fn check(values: &[Option<f32>], expected: &[(usize, Option<f32>)]) {
        for (frame, want) in expected {
            let got = values[frame - 1];
            match (got, want) {
                (Some(got), Some(want)) => assert!(
                    (got - want).abs() < 2.0e-6,
                    "frame {frame}: {got} vs {want}"
                ),
                _ => assert_eq!(got, *want, "frame {frame}"),
            }
        }
    }

    /// Circle1 (delay 0) and Circle3 (delay 0.25) at a steady 1/60 s: the
    /// pulse, the plateau, the loop restart (frame 61 / 76), and the delay
    /// ending with its remainder (Circle3's first applied frame is 16).
    #[test]
    fn scale_tween_follows_dotween_at_sixty_frames() {
        let dts = vec![1.0f32 / 60.0; 130];
        check(
            &run(0.0, &dts),
            &[
                (1, Some(0.688_907_74)),
                (2, Some(0.745_169_6)),
                (14, Some(0.667_511_4)),
                (30, Some(0.667_008_76)),
                (60, Some(0.665_880_86)),
                (61, Some(0.688_907_1)),
                (62, Some(0.745_168_5)),
                (70, Some(0.777_629_2)),
                (121, Some(0.688_906_5)),
            ],
        );
        check(
            &run(0.25, &dts),
            &[
                (15, None),
                (16, Some(0.688_907_74)),
                (20, Some(0.961_161_1)),
                (21, Some(0.996_587_45)),
                (22, Some(0.987_883_5)),
                (30, Some(0.667_502_3)),
                (75, Some(0.665_880_86)),
                (76, Some(0.688_907_1)),
                (77, Some(0.745_168_5)),
            ],
        );
    }

    /// Uneven frames (a 0.4 s hitch first): the delay remainder and the
    /// loop wrap carry the excess time.
    #[test]
    fn scale_tween_carries_the_excess_through_delay_and_loops() {
        let dts = [0.4f32, 0.05, 0.05, 0.05, 0.1, 0.3, 0.2, 0.2, 0.2, 0.25];
        check(
            &run(0.0, &dts),
            &[
                (1, Some(0.667_270_5)),
                (6, Some(0.665_899_46)),
                (7, Some(0.859_945_5)),
                (8, Some(0.667_374_07)),
                (10, Some(0.666_139_5)),
            ],
        );
        check(
            &run(0.25, &dts),
            &[
                (1, Some(0.859_946)),
                (2, Some(0.669_995_2)),
                (7, Some(0.665_952_03)),
                (8, Some(0.996_587_4)),
                (9, Some(0.667_453)),
            ],
        );
    }

    fn rect(
        anchor_min: Vec2,
        anchor_max: Vec2,
        anchored: Vec2,
        size: Vec2,
        scale: f32,
    ) -> RectFields {
        RectFields {
            anchor_min,
            anchor_max,
            anchored,
            size_delta: size,
            pivot: Vec2::splat(0.5),
            scale: Vec2::splat(scale),
        }
    }

    /// The extracted chain on a 1280 x 720 screen (match width: canvas
    /// 1920 x 1080): Circle1 sits at the ButtomRight corner + (-24 - 64,
    /// 13.1 + 75 + 42), all scaled by ContentRoot's 0.999 about the centre.
    #[test]
    fn circle_one_lands_at_the_bottom_right_corner_offset() {
        let factor = canvas_scale_factor(Vec2::new(1280.0, 720.0), Vec2::new(1920.0, 1080.0), 0.0);
        assert!((factor - 2.0 / 3.0).abs() < 1.0e-6);
        let canvas = Vec2::new(1280.0, 720.0) / factor;
        let chain = [
            rect(Vec2::ZERO, Vec2::ONE, Vec2::ZERO, Vec2::ZERO, 0.999),
            rect(
                Vec2::X,
                Vec2::X,
                Vec2::new(-6.103_515_6e-5, 6.103_515_6e-5),
                Vec2::splat(100.0),
                1.0,
            ),
            rect(
                Vec2::splat(0.5),
                Vec2::splat(0.5),
                Vec2::new(-24.0, 13.1),
                Vec2::splat(100.0),
                1.0,
            ),
            rect(
                Vec2::splat(0.5),
                Vec2::splat(0.5),
                Vec2::new(-64.0, 75.0),
                Vec2::splat(100.0),
                1.0,
            ),
            rect(
                Vec2::splat(0.5),
                Vec2::splat(0.5),
                Vec2::ZERO,
                Vec2::splat(100.0),
                1.0,
            ),
        ];
        let mut origin = Vec2::ZERO;
        let mut accumulated = Vec2::ONE;
        let mut parent = (canvas, Vec2::splat(0.5));
        for node in &chain {
            let (pivot, size) = place(parent.0, parent.1, node);
            origin += accumulated * pivot;
            accumulated *= node.scale;
            parent = (size, node.pivot);
        }
        let circle = rect(
            Vec2::splat(0.5),
            Vec2::splat(0.5),
            Vec2::new(0.0, 42.0),
            Vec2::splat(18.0),
            0.666_666_7,
        );
        let (centre, size) = quad(origin, accumulated, parent, &circle);
        let want = Vec2::new(
            960.0 - 6.103_515_6e-5 - 88.0,
            -540.0 + 6.103_515_6e-5 + 88.1 + 42.0,
        ) * 0.999;
        assert!(
            (centre - want).abs().max_element() < 1.0e-3,
            "{centre} vs {want}"
        );
        assert!(
            (size - Vec2::splat(18.0 * 0.666_666_7 * 0.999))
                .abs()
                .max_element()
                < 1.0e-5
        );
        // UIPartsImageOutline: 18 * s + 8 per axis, 20 at the rest scale.
        let outline = outline_size(Vec2::splat(18.0), Vec2::splat(0.666_666_7), 8.0);
        assert!((outline - Vec2::splat(20.0)).abs().max_element() < 1.0e-5);
        assert_eq!(
            outline_size(Vec2::splat(18.0), Vec2::ONE, 8.0),
            Vec2::splat(26.0)
        );
    }
}
