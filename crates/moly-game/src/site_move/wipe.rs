//! `DoorTransitioner` and its `SiteWipeController`: the circle wipe that
//! closes over the screen when the player goes through a door and opens
//! again on the other side.
//!
//! `DoorTransitionerManager.Init` loads the `SiteTransitionerNormal` prefab
//! of the house-transition bundle once and instantiates it under the screen
//! manager's layer 6; its `Awake` hides the wipe. `FadeOut` shows the wipe,
//! plays `se_transition_end` and tweens the material's `_Scale` from 0.75 to
//! 15 with `InCirc` (the hole shrinks to nothing); its callback runs at the
//! tween's completion and the wipe stays covering the screen. `FadeIn` plays
//! `se_transition_start` and tweens back to 0.75 with `OutCirc`; at its
//! completion the wipe is hidden and the callback runs. The laws are in
//! [`super::door_law`], the program in `shaders/site_wipe_circle.wgsl`.
//!
//! The prefab is requested at startup (the release root carries it or not);
//! everything the draw depends on is checked against it: the canvas render
//! mode, the `wipe` image's rect, material and texture, the controller's
//! material, the material's shader, colour and pass state. A missing or
//! different prefab refuses the wipe with one WARN (a missing file also
//! gets the asset server's own ERROR line); the door moves then run without
//! it and their fade callbacks complete at once.
//!
//! Named differences:
//! - The prefab's `TweenRotation` (0 to -720 degrees about z over 1.5 s,
//!   start timing 3) has no caller in the door moves; a rotation about the
//!   centre of a centred circle would not show anyway. Not played.
//! - The wipe is drawn on its own overlay camera (order 150: above the
//!   balloon, site map, settings and library cameras, below the entry
//!   cover). The canvas sorts at 500 over the screen layers it covers; its
//!   order against the product's other overlays was not read.
//! - From the move's admission until the first `FadeOut` the quad is drawn
//!   fully open (`_Scale` 0.75: the hole is wider than any screen, so no
//!   pixel is written) so its pipeline is compiled before the first
//!   covering frame; the source's wipe is inactive until `FadeOut`.

use bevy::asset::uuid::Uuid;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::marker::PhantomData;

use super::door_law::{self, WipeTween, WIPE_OPEN_SCALE};

const DOC: &str = "moly://house-transition/sitetransitionernormal/sitetransitionernormal.json";
const SHADER_NAME: &str = "Sekai/Area/WipeCircle";
const MATERIAL_NAME: &str = "AreaWipeCircle";
const WIPE_NODE: &str = "wipe";
const SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x7369_7465_7769_7065_6369_7263_6c65_0001),
    PhantomData,
);
const WIPE_LAYER: usize = 28;
const WIPE_ORDER: isize = 150;

#[derive(Debug, Clone, Copy, ShaderType)]
struct WipeUniform {
    /// (_Scale, _OffsetX, _OffsetY, 0)
    params: Vec4,
    /// Half the screen in canvas units.
    half_extent: Vec4,
    /// _Color as stored.
    color: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct WipeCircleMaterial {
    #[uniform(0)]
    value: WipeUniform,
}

impl Material2d for WipeCircleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    /// The pass blends SrcAlpha/OneMinusSrcAlpha on colour and alpha with no
    /// depth write; the program's alpha is 0 or 1, so this blend state
    /// writes the same values (checked against the prefab on load).
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

enum State {
    Loading(Handle<JsonAsset>),
    Ready {
        camera: Entity,
        quad: Entity,
        material: Handle<WipeCircleMaterial>,
    },
    Refused,
}

/// The direction of a running or finished wipe tween.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fade {
    Out,
    In,
}

/// `DoorTransitioner.Init` as the door moves see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Init {
    Pending,
    Ready,
    Refused,
}

#[derive(Resource)]
pub(crate) struct SiteWipe {
    state: State,
    tween: Option<(Fade, WipeTween)>,
    /// A tween completed and its callback has not been read yet: the fade,
    /// the completing frame, and the tween clock at completion.
    finished: Option<(Fade, u64, f32)>,
    scale: f32,
    offset: Vec2,
    /// The wipe object is active (`EnableWipe`).
    shown: bool,
    armed: bool,
    warned: bool,
}

pub(crate) fn install(app: &mut App) {
    app.add_plugins(Material2dPlugin::<WipeCircleMaterial>::default());
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            SHADER.id(),
            Shader::from_wgsl(
                include_str!("../shaders/site_wipe_circle.wgsl"),
                "moly_game/src/shaders/site_wipe_circle.wgsl".to_owned(),
            ),
        )
        .expect("site wipe shader installation");
    app.add_systems(Startup, request);
}

fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SiteWipe {
        state: State::Loading(server.load(DOC)),
        tween: None,
        finished: None,
        scale: WIPE_OPEN_SCALE,
        offset: Vec2::ZERO,
        shown: false,
        armed: false,
        warned: false,
    });
}

/// `DoorTransitionerManager.Init`: the transitioner exists once the prefab
/// has loaded; a refused or absent prefab answers `Refused`.
pub(crate) fn init(world: &mut World) -> Init {
    if !world.contains_resource::<SiteWipe>() {
        return Init::Refused;
    }
    if resolve(world) {
        Init::Ready
    } else if matches!(world.resource::<SiteWipe>().state, State::Refused) {
        Init::Refused
    } else {
        Init::Pending
    }
}

/// The move is admitted: draw the quad fully open so its pipeline exists
/// before the first covering frame.
pub(crate) fn arm(world: &mut World) {
    if !resolve(world) {
        return;
    }
    let mut wipe = world.resource_mut::<SiteWipe>();
    if wipe.tween.is_none() && !wipe.shown {
        wipe.scale = WIPE_OPEN_SCALE;
        wipe.armed = true;
    }
}

/// `DoorTransitioner.FadeOut(callback)`: returns false without a
/// transitioner (the caller completes the callback at once).
pub(crate) fn fade_out(world: &mut World) -> bool {
    start(world, Fade::Out)
}

/// `DoorTransitioner.FadeIn(callback)`.
pub(crate) fn fade_in(world: &mut World) -> bool {
    start(world, Fade::In)
}

fn start(world: &mut World, fade: Fade) -> bool {
    if !resolve(world) {
        return false;
    }
    let tween = match fade {
        Fade::Out => WipeTween::fade_out(),
        Fade::In => WipeTween::fade_in(),
    };
    {
        let mut wipe = world.resource_mut::<SiteWipe>();
        // EnableWipe(true), SetPosition(Vector2.zero), the current scale.
        wipe.shown = true;
        wipe.armed = false;
        wipe.offset = door_law::wipe_offset(Vec2::ZERO);
        wipe.scale = tween.from;
        wipe.tween = Some((fade, tween));
        wipe.finished = None;
    }
    let cue = match fade {
        Fade::Out => "se_transition_end",
        Fade::In => "se_transition_start",
    };
    super::push_se(world, cue);
    info!(
        "[site-move] DoorTransitioner.{}: _Scale {} -> {} over {}s {:?}, {cue}",
        match fade {
            Fade::Out => "FadeOut",
            Fade::In => "FadeIn",
        },
        tween.from,
        tween.to,
        door_law::WIPE_DURATION,
        tween.ease
    );
    true
}

/// The callback of `fade`: `Some((frame, clock))` once, on the first read
/// after the tween completed.
pub(crate) fn take_finished(world: &mut World, fade: Fade) -> Option<(u64, f32)> {
    let mut wipe = world.get_resource_mut::<SiteWipe>()?;
    match wipe.finished {
        Some((done, frame, clock)) if done == fade => {
            wipe.finished = None;
            Some((frame, clock))
        }
        _ => None,
    }
}

/// Update, after the executor: the DOTween update of the wipe tween (the
/// creating frame's delta counts, the product's tween convention), then the
/// material and the quad's visibility.
pub(crate) fn advance(world: &mut World) {
    if !world.contains_resource::<SiteWipe>() || !resolve(world) {
        return;
    }
    let dt = world.resource::<Time>().delta_secs();
    let frame = u64::from(world.resource::<FrameCount>().0);
    let half = {
        let mut windows = world.query_filtered::<&Window, With<PrimaryWindow>>();
        windows
            .single(world)
            .ok()
            .map(|window| door_law::canvas_half_extent(window.width(), window.height()))
    };
    let (scale, offset, visible, material, quad, camera) = {
        let mut wipe = world.resource_mut::<SiteWipe>();
        if let Some((fade, tween)) = wipe.tween.as_mut() {
            let fade = *fade;
            let scale = tween.advance(dt);
            let done = tween.done();
            let clock = tween.clock.elapsed;
            wipe.scale = scale;
            if done {
                wipe.tween = None;
                wipe.finished = Some((fade, frame, clock));
                if fade == Fade::In {
                    // OnFinishedFadeIn: EnableWipe(false).
                    wipe.shown = false;
                }
                info!(
                    "[site-move] wipe {fade:?} tween complete at clock {clock:.4}s (_Scale {scale})"
                );
            }
        }
        let State::Ready {
            camera,
            quad,
            material,
        } = &wipe.state
        else {
            return;
        };
        (
            wipe.scale,
            wipe.offset,
            wipe.shown || wipe.armed,
            material.clone(),
            *quad,
            *camera,
        )
    };
    if let Some(asset) = world
        .resource_mut::<Assets<WipeCircleMaterial>>()
        .get_mut(&material)
    {
        asset.value.params = Vec4::new(scale, offset.x, offset.y, 0.0);
        if let Some(half) = half {
            asset.value.half_extent = Vec4::new(half.x, half.y, 0.0, 0.0);
        }
    }
    let wanted = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if let Some(mut visibility) = world.get_mut::<Visibility>(quad) {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if let Some(mut camera) = world.get_mut::<Camera>(camera) {
        if camera.is_active != visible {
            camera.is_active = visible;
        }
    }
}

/// The move is over: an armed but unused quad goes away. A wipe that is
/// still covering stays (the source leaves a FadeOut's wipe active).
pub(crate) fn disarm(world: &mut World) {
    if let Some(mut wipe) = world.get_resource_mut::<SiteWipe>() {
        wipe.armed = false;
    }
}

/// Turn a loaded prefab into the camera and quad (once); false while the
/// wipe cannot draw.
fn resolve(world: &mut World) -> bool {
    let Some(wipe) = world.get_resource::<SiteWipe>() else {
        return false;
    };
    let handle = match &wipe.state {
        State::Ready { .. } => return true,
        State::Refused => return false,
        State::Loading(handle) => handle.clone(),
    };
    let server = world.resource::<AssetServer>();
    if server.load_state(&handle).is_failed() {
        return refuse(
            world,
            format!("{DOC} is not in the release root (the house-transition product)"),
        );
    }
    let Some(text) = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|json| json.0.clone())
    else {
        return false;
    };
    let prefab = match serde_json::from_str::<Value>(&text)
        .map_err(|error| error.to_string())
        .and_then(|doc| read_prefab(&doc))
    {
        Ok(prefab) => prefab,
        Err(error) => return refuse(world, error),
    };
    let material = world
        .resource_mut::<Assets<WipeCircleMaterial>>()
        .add(WipeCircleMaterial {
            value: WipeUniform {
                params: Vec4::new(WIPE_OPEN_SCALE, 0.0, 0.0, 0.0),
                half_extent: Vec4::new(960.0, 540.0, 0.0, 0.0),
                color: prefab.color,
            },
        });
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(1.0, 1.0));
    let camera = world
        .spawn((
            Name::new("SiteWipeCamera"),
            Camera2d,
            crate::camera::MYSEKAI_CAMERA_MSAA,
            Camera {
                order: WIPE_ORDER,
                clear_color: ClearColorConfig::None,
                is_active: false,
                ..default()
            },
            // The wipe colour is a final value: no tonemapping or dither.
            Tonemapping::None,
            DebandDither::Disabled,
            RenderLayers::layer(WIPE_LAYER),
        ))
        .id();
    let quad = world
        .spawn((
            Name::new("SiteWipe"),
            Mesh2d(mesh),
            MeshMaterial2d(material.clone()),
            Transform::default(),
            Visibility::Hidden,
            NoFrustumCulling,
            RenderLayers::layer(WIPE_LAYER),
        ))
        .id();
    info!(
        "[site-move] door wipe ready: {SHADER_NAME} colour {:?}, rect {WIPE_NODE} {}x{}",
        prefab.color,
        door_law::WIPE_RECT,
        door_law::WIPE_RECT
    );
    world.resource_mut::<SiteWipe>().state = State::Ready {
        camera,
        quad,
        material,
    };
    true
}

fn refuse(world: &mut World, reason: String) -> bool {
    let mut wipe = world.resource_mut::<SiteWipe>();
    if !wipe.warned {
        wipe.warned = true;
        warn!("[site-move] door wipe refused: {reason}; the door moves run without the wipe");
    }
    wipe.state = State::Refused;
    false
}

struct Prefab {
    color: Vec4,
}

fn read_prefab(doc: &Value) -> Result<Prefab, String> {
    let components = &doc["components"];
    let canvas = &components["Canvas"]["instances"][0]["fields"];
    if canvas["m_RenderMode"].as_i64() != Some(0) {
        return Err(format!(
            "canvas render mode {} is not the screen-space overlay",
            canvas["m_RenderMode"]
        ));
    }
    let on_wipe = |component: &str| -> Result<&Value, String> {
        components[component]["instances"]
            .as_array()
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row["node"].as_str() == Some(WIPE_NODE))
            })
            .map(|row| &row["fields"])
            .ok_or_else(|| format!("no {component} on {WIPE_NODE}"))
    };
    let image = on_wipe("CustomRawImage")?;
    if image["m_Material"]["name"].as_str() != Some(MATERIAL_NAME) {
        return Err(format!(
            "the {WIPE_NODE} image material is {}, not {MATERIAL_NAME}",
            image["m_Material"]["name"]
        ));
    }
    if !image["m_Texture"].is_null() {
        return Err("the wipe image has a texture; this program reads none".into());
    }
    if image["m_Enabled"].as_i64() != Some(1) {
        return Err("the wipe image is disabled".into());
    }
    let controller = on_wipe("SiteWipeController")?;
    if controller["_mat"]["name"].as_str() != Some(MATERIAL_NAME) {
        return Err(format!(
            "SiteWipeController drives {}, not {MATERIAL_NAME}",
            controller["_mat"]["name"]
        ));
    }
    let rect = on_wipe("RectTransform")?;
    let v2 = |value: &Value| -> Option<Vec2> {
        Some(Vec2::new(
            value["x"].as_f64()? as f32,
            value["y"].as_f64()? as f32,
        ))
    };
    let centred = v2(&rect["m_AnchorMin"]) == Some(Vec2::splat(0.5))
        && v2(&rect["m_AnchorMax"]) == Some(Vec2::splat(0.5))
        && v2(&rect["m_Pivot"]) == Some(Vec2::splat(0.5))
        && v2(&rect["m_AnchoredPosition"]) == Some(Vec2::ZERO)
        && v2(&rect["m_SizeDelta"]) == Some(Vec2::splat(door_law::WIPE_RECT));
    if !centred {
        return Err(format!(
            "{WIPE_NODE} is not the centred {0}x{0} rect",
            door_law::WIPE_RECT
        ));
    }
    let material = doc["materials"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["name"].as_str() == Some(MATERIAL_NAME))
        })
        .ok_or_else(|| format!("material {MATERIAL_NAME} not exported"))?;
    let shader = &material["shader"];
    if shader["name"].as_str() != Some(SHADER_NAME) {
        return Err(format!(
            "{MATERIAL_NAME} uses {}, not {SHADER_NAME}",
            shader["name"]
        ));
    }
    let pass = shader["shaderPasses"]
        .as_array()
        .and_then(|passes| {
            passes
                .iter()
                .find(|pass| pass["subShaderIndex"].as_i64() == Some(0))
        })
        .ok_or("no pass of the first sub-shader")?;
    let state = &pass["renderState"];
    let blend = &state["blend"];
    let val = |value: &Value| value["val"].as_f64();
    // Unity BlendMode SrcAlpha (5) / OneMinusSrcAlpha (10) on colour and
    // alpha, BlendOp Add (0).
    for (key, want) in [
        ("srcBlend", 5.0),
        ("destBlend", 10.0),
        ("srcBlendAlpha", 5.0),
        ("destBlendAlpha", 10.0),
        ("blendOp", 0.0),
        ("blendOpAlpha", 0.0),
    ] {
        if val(&blend[key]) != Some(want) {
            return Err(format!(
                "pass {key} is {}, the wipe blends with {want}",
                blend[key]
            ));
        }
    }
    // The colour mask is the material's `_ColorMask` property: all four.
    let mask = &blend["colMask"];
    let floats = &material["floats"];
    let mask_value = match mask["name"].as_str() {
        Some(property) if property != "<noninit>" => floats[property].as_f64(),
        _ => val(mask),
    };
    if mask_value != Some(15.0) {
        return Err(format!(
            "pass colour mask is {mask}, the wipe writes all channels"
        ));
    }
    if val(&state["zWrite"]) != Some(0.0) {
        return Err(format!(
            "pass zWrite is {}, the wipe writes no depth",
            state["zWrite"]
        ));
    }
    let f = |value: &Value| value.as_f64().map(|v| v as f32);
    let color = material["colors"]["_Color"]
        .as_array()
        .filter(|rgba| rgba.len() == 4)
        .and_then(|rgba| {
            Some(Vec4::new(
                f(&rgba[0])?,
                f(&rgba[1])?,
                f(&rgba[2])?,
                f(&rgba[3])?,
            ))
        })
        .ok_or_else(|| format!("{MATERIAL_NAME} colour _Color missing"))?;
    Ok(Prefab { color })
}
