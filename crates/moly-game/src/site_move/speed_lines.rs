//! `FieldCameraEffectController.SetConcentrationLineActive`: the speed lines
//! over the screen during the cannon flight.
//!
//! The FieldCamera prefab carries a screen-space overlay canvas with one
//! RawImage stretched over it (anchors 0..1, no size delta), drawn with the
//! material `ConcentrationLine` (shader `Mysekai/ConcentrationLine`). The
//! image starts disabled; the setter is `_concentrationLineImage.enabled =
//! isActive`, and the Core calls it with true after the firing sound and
//! with false at the reveal step.
//!
//! The prefab is requested only when the release root lists the site-move
//! products (see `products`); otherwise the move has no speed lines.
//!
//! Everything the draw depends on is read from the extracted prefab: the
//! material's six floats and its colour, the pass blend state, the image's
//! material and uvRect, the canvas render mode and the image's rect. The
//! program itself is transcribed in `shaders/concentration_line.wgsl`. A
//! missing or different prefab refuses the overlay with one WARN; the move
//! continues without it.
//!
//! The quad lives on the overlay camera (`balloon::overlay_camera`, after
//! the 3D view and its post effects, like an overlay canvas after every
//! camera), sorted under the other overlay draws: the canvas's sorting order
//! is 0 and the order against the product's other overlay content was not
//! read. `_TimeParameters.x` is fed from the product's game clock.
//!
//! Named differences:
//! - The source blends the encoded colour in a Gamma-space player; this draw
//!   blends in linear light like the product's other overlay draws. The
//!   colour is white, so only the blend curve differs.
//! - From the move's admission until the fire step the quad is drawn at zero
//!   coverage so its pipeline is compiled before the first enabled frame (a
//!   disabled RawImage issues no draw; a zero-alpha draw with this blend
//!   leaves the target as it was). Outside a move it is hidden.

use bevy::asset::uuid::Uuid;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::marker::PhantomData;

const DOC: &str = "moly://field-camera/fieldcamera/fieldcamera.json";
const SHADER_NAME: &str = "Mysekai/ConcentrationLine";
const MATERIAL_NAME: &str = "ConcentrationLine";
const SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x636f_6e63_656e_7472_6174_696f_6e00_0001),
    PhantomData,
);
/// The overlay camera sorts its transparent draws by translation z; the
/// canvas draws under the other overlay content.
const SORT_Z: f32 = -1000.0;

/// The material floats, by their shader-graph property names.
const FLOATS: [&str; 6] = [
    "Vector1_2",
    "Vector1_603d0dab08414d719cd581f940fbdbc8",
    "Vector1_1",
    "Vector1_faf012929d054e78b256dae299677873",
    "Vector1_266bcaed2966410aa10f41605fd8de44",
    "Vector1_21a8a39812064880bc8182238f00af1b",
];
const COLOR: &str = "Color_28639f00d0534a889342fdf1a9d89b8a";

#[derive(Debug, Clone, Copy, ShaderType)]
struct ConcentrationLineUniform {
    time_parameters: Vec4,
    a: Vec4,
    b: Vec4,
    color: Vec4,
    uv_rect: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct ConcentrationLineMaterial {
    #[uniform(0)]
    value: ConcentrationLineUniform,
}

impl Material2d for ConcentrationLineMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    /// The pass blends SrcAlpha/OneMinusSrcAlpha on colour and
    /// One/OneMinusSrcAlpha on alpha with no depth write, which is the
    /// blend state this mode selects (checked against the prefab on load).
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// What the prefab says about the draw.
#[derive(Clone, Copy, Debug)]
struct Prefab {
    floats: [f32; 6],
    color: Vec4,
    uv_rect: Vec4,
}

enum State {
    Loading(Handle<JsonAsset>),
    Ready {
        quad: Entity,
        material: Handle<ConcentrationLineMaterial>,
    },
    Refused,
}

#[derive(Resource)]
pub(crate) struct SpeedLines {
    state: State,
    pending_warned: bool,
}

pub(crate) fn install(app: &mut App) {
    app.add_plugins(Material2dPlugin::<ConcentrationLineMaterial>::default());
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            SHADER.id(),
            Shader::from_wgsl(
                include_str!("../shaders/concentration_line.wgsl"),
                "moly_game/src/shaders/concentration_line.wgsl".to_owned(),
            ),
        )
        .expect("speed-line shader installation");
    app.add_systems(
        PostUpdate,
        advance_time.run_if(resource_exists::<SpeedLines>),
    );
}

/// Request the prefab (the release root lists the site-move products).
pub(crate) fn request(world: &mut World) {
    let doc = world.resource::<AssetServer>().load(DOC);
    world.insert_resource(SpeedLines {
        state: State::Loading(doc),
        pending_warned: false,
    });
}

/// The move is admitted: build the quad if the prefab is ready, and draw it
/// at zero coverage until the Core enables it.
pub(crate) fn arm(world: &mut World) {
    if !resolve(world) {
        return;
    }
    set_enabled(world, false);
    set_visible(world, true);
}

/// `SetConcentrationLineActive(active)`.
pub(crate) fn set_active(world: &mut World, active: bool) {
    if !resolve(world) {
        return;
    }
    set_enabled(world, active);
    info!("[site-move] SetConcentrationLineActive({active})");
}

/// The move is over: hide the quad.
pub(crate) fn disarm(world: &mut World) {
    if !resolve(world) {
        return;
    }
    set_enabled(world, false);
    set_visible(world, false);
}

/// Turn a loaded prefab into the quad (once); false while it cannot draw.
fn resolve(world: &mut World) -> bool {
    let Some(lines) = world.get_resource::<SpeedLines>() else {
        return false;
    };
    let handle = match &lines.state {
        State::Ready { .. } => return true,
        State::Refused => return false,
        State::Loading(handle) => handle.clone(),
    };
    let server = world.resource::<AssetServer>();
    if server.load_state(&handle).is_failed() {
        return refuse(world, format!("{DOC} failed to load"));
    }
    let Some(text) = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|json| json.0.clone())
    else {
        // Not loaded yet: the move runs without the overlay until it is.
        let mut lines = world.resource_mut::<SpeedLines>();
        if !lines.pending_warned {
            lines.pending_warned = true;
            warn!("[site-move] speed-line prefab {DOC} not loaded yet; no speed lines until it is");
        }
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
        .resource_mut::<Assets<ConcentrationLineMaterial>>()
        .add(ConcentrationLineMaterial {
            value: ConcentrationLineUniform {
                time_parameters: Vec4::ZERO,
                a: Vec4::new(
                    prefab.floats[0],
                    prefab.floats[1],
                    prefab.floats[2],
                    prefab.floats[3],
                ),
                b: Vec4::new(prefab.floats[4], prefab.floats[5], 0.0, 0.0),
                color: prefab.color,
                uv_rect: prefab.uv_rect,
            },
        });
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(1.0, 1.0));
    let quad = world
        .spawn((
            Name::new("SpeedLines"),
            Mesh2d(mesh),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, SORT_Z),
            Visibility::Hidden,
            NoFrustumCulling,
            RenderLayers::layer(crate::balloon::BALLOON_LAYER),
        ))
        .id();
    info!(
        "[site-move] speed-line overlay ready: {SHADER_NAME} floats {:?} colour {:?} uvRect {:?}",
        prefab.floats, prefab.color, prefab.uv_rect
    );
    world.resource_mut::<SpeedLines>().state = State::Ready { quad, material };
    true
}

fn refuse(world: &mut World, reason: String) -> bool {
    warn!("[site-move] speed-line overlay refused: {reason}");
    world.resource_mut::<SpeedLines>().state = State::Refused;
    false
}

fn set_enabled(world: &mut World, enabled: bool) {
    let State::Ready { material, .. } = &world.resource::<SpeedLines>().state else {
        return;
    };
    let material = material.clone();
    if let Some(asset) = world
        .resource_mut::<Assets<ConcentrationLineMaterial>>()
        .get_mut(&material)
    {
        asset.value.b.z = if enabled { 1.0 } else { 0.0 };
    }
}

fn set_visible(world: &mut World, visible: bool) {
    let State::Ready { quad, .. } = world.resource::<SpeedLines>().state else {
        return;
    };
    if let Some(mut visibility) = world.get_mut::<Visibility>(quad) {
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// `_TimeParameters` while the quad is drawn.
fn advance_time(
    time: Res<Time>,
    lines: Res<SpeedLines>,
    quads: Query<&Visibility>,
    mut materials: ResMut<Assets<ConcentrationLineMaterial>>,
) {
    let State::Ready { quad, material } = &lines.state else {
        return;
    };
    if quads
        .get(*quad)
        .is_ok_and(|visibility| *visibility == Visibility::Hidden)
    {
        return;
    }
    let t = time.elapsed_secs();
    if let Some(asset) = materials.get_mut(material) {
        asset.value.time_parameters = Vec4::new(t, t.sin(), t.cos(), 0.0);
    }
}

/// Read the prefab's canvas, image and material; refuse anything this draw
/// does not reproduce.
fn read_prefab(doc: &Value) -> Result<Prefab, String> {
    let components = &doc["components"];
    let canvas = &components["Canvas"]["instances"][0]["fields"];
    // RenderMode 0: screen-space overlay.
    if canvas["m_RenderMode"].as_i64() != Some(0) {
        return Err(format!(
            "canvas render mode {} is not the screen-space overlay",
            canvas["m_RenderMode"]
        ));
    }
    let image = components["RawImage"]["instances"]
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or("the prefab has no RawImage")?;
    let fields = &image["fields"];
    if fields["m_Material"]["name"].as_str() != Some(MATERIAL_NAME) {
        return Err(format!(
            "the RawImage material is {}, not {MATERIAL_NAME}",
            fields["m_Material"]["name"]
        ));
    }
    if !fields["m_Texture"].is_null() {
        return Err("the RawImage has a texture; this shader reads none".into());
    }
    let node = image["node"].as_str().unwrap_or_default();
    let rect = components["RectTransform"]["instances"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["node"].as_str() == Some(node)))
        .map(|row| &row["fields"])
        .ok_or_else(|| format!("no RectTransform for {node}"))?;
    let v2 = |value: &Value| -> Option<Vec2> {
        Some(Vec2::new(
            value["x"].as_f64()? as f32,
            value["y"].as_f64()? as f32,
        ))
    };
    let stretched = v2(&rect["m_AnchorMin"]) == Some(Vec2::ZERO)
        && v2(&rect["m_AnchorMax"]) == Some(Vec2::ONE)
        && v2(&rect["m_SizeDelta"]) == Some(Vec2::ZERO)
        && v2(&rect["m_AnchoredPosition"]) == Some(Vec2::ZERO);
    if !stretched {
        return Err(format!("{node} is not stretched over its canvas"));
    }
    let uv = &fields["m_UVRect"];
    let f = |value: &Value| value.as_f64().map(|v| v as f32);
    let uv_rect = Vec4::new(
        f(&uv["x"]).ok_or("uvRect x")?,
        f(&uv["y"]).ok_or("uvRect y")?,
        f(&uv["width"]).ok_or("uvRect width")?,
        f(&uv["height"]).ok_or("uvRect height")?,
    );
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
    // The first sub-shader's pass: Unity BlendMode SrcAlpha (5) and
    // OneMinusSrcAlpha (10) on colour, One (1) and OneMinusSrcAlpha (10) on
    // alpha, BlendOp Add (0) on both, all four channels written, no depth
    // write.
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
    let expected = [
        ("srcBlend", 5.0),
        ("destBlend", 10.0),
        ("srcBlendAlpha", 1.0),
        ("destBlendAlpha", 10.0),
        ("blendOp", 0.0),
        ("blendOpAlpha", 0.0),
        ("colMask", 15.0),
    ];
    for (key, want) in expected {
        if val(&blend[key]) != Some(want) {
            return Err(format!(
                "pass {key} is {}, the overlay blends with {want}",
                blend[key]
            ));
        }
    }
    if val(&state["zWrite"]) != Some(0.0) {
        return Err(format!(
            "pass zWrite is {}, the overlay writes no depth",
            state["zWrite"]
        ));
    }
    let floats = &material["floats"];
    let mut values = [0.0_f32; 6];
    for (slot, key) in values.iter_mut().zip(FLOATS) {
        *slot = f(&floats[key]).ok_or_else(|| format!("{MATERIAL_NAME} float {key} missing"))?;
    }
    let color = material["colors"][COLOR]
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
        .ok_or_else(|| format!("{MATERIAL_NAME} colour {COLOR} missing"))?;
    Ok(Prefab {
        floats: values,
        color,
        uv_rect,
    })
}
