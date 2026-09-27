//! Admission of one particle row under a UIParticle: the row's system runs
//! on the shared particle runtime (its law parse, curve dispatch and the
//! native birth owner the fixture host installs at Play), and its renderer
//! draws one of the three UI particle programs through the canvas bake:
//! `Mysekai/Effect/UI-Uber`, `Sekai/Particles/UI-Default` or
//! `Sekai/Particles/UI/Additive`, selected by the material's shader name.
//!
//! Every control this path does not carry is refused by name; nothing is
//! read as a default. The gates are the shared runtime's own entry points,
//! with the choices this host makes written next to them:
//! - the native birth path is required (a row the native installer would
//!   leave on the legacy step is refused with the installer's reason);
//! - the orthographic baking camera's size limits are checked per frame by
//!   the host, so the renderer's minimum must be zero here;
//! - the renderer's particle sort must be None: the sort against the
//!   orthographic baking camera is not wired;
//! - no sub-emitter, trail, collision, ring buffer or start delay: this host
//!   installs none of their owners.

use bevy::prelude::*;
use moly_assets::material_passes::SourceRenderState;
use moly_assets::particle_source::ParticleSourceModules;
use moly_law::particle::schema::{ShapeTexture, SimulationSpace};
use moly_law::particle::{
    Effects, EmissionState, EmitterParams, LimitVelocity, RotationOverLifetime,
};
use serde_json::Value;

use crate::particle_runtime::{EffectKind, Geometry, Rng, Runtime, SourceRoute};

/// The programs this path draws, by shader name.
pub(crate) const UI_UBER: &str = "Mysekai/Effect/UI-Uber";
pub(crate) const UI_DEFAULT: &str = "Sekai/Particles/UI-Default";
pub(crate) const UI_ADDITIVE: &str = "Sekai/Particles/UI/Additive";

/// The program a UI particle material draws with. The values are the
/// selector the wgsl reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiProgram {
    Uber = 0,
    Default = 1,
    Additive = 2,
}

/// `unity_GUIZTestMode`, the global the engine's canvas manager sets before
/// it draws a canvas and that `Sekai/Particles/UI-Default`'s ZTest reads:
/// `UI::InitializeDeviceForOverlay` (a screen-space overlay canvas) sets 8
/// (Always), `UI::CanvasManager::EmitGeometryForCamera` (a canvas drawn by a
/// camera) sets 4 (LessEqual).
pub(crate) const GUI_ZTEST_OVERLAY: u8 = 8;
pub(crate) const GUI_ZTEST_CAMERA: u8 = 4;

/// Birth stream of this path, distinct from the weather, fixture and harvest
/// streams (only the legacy step reads it; admitted rows run the native
/// birth owner).
const RNG_SEED: u64 = 0x7569_7061_0001_0001;

/// How a UI particle material draws: its program, texture, `_MainTex_ST`,
/// the program's own material values and the pass state of the pass the UI
/// draw takes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UiDraw {
    pub(crate) program: UiProgram,
    /// `_MainTex`, relative to the document's directory.
    pub(crate) texture: String,
    pub(crate) main_tex_st: [f32; 4],
    /// UI-Uber's `_BlendMode` (0 and 1 keep the colour, 2 multiplies it by
    /// alpha); 0 for the other programs, which have no such property.
    pub(crate) blend_mode: u32,
    /// UI-Default's `_Color` (the vertex program multiplies the vertex colour
    /// by all four channels); `None` for the other programs, which have none.
    pub(crate) color: Option<[f32; 4]>,
    pub(crate) state: SourceRenderState,
    /// The material as exported; equal materials merge into one submesh the
    /// way `GetMaterialHash` merges them.
    pub(crate) identity: String,
}

/// One admitted row.
pub(crate) struct Admitted {
    pub(crate) runtime: Runtime,
    pub(crate) route: SourceRoute,
    pub(crate) draw: UiDraw,
    pub(crate) space: super::bake::Space,
    /// The renderer's `maxParticleSize`, checked against the baking camera.
    pub(crate) max_particle_size: f32,
}

/// The pass the UI draw takes. The UI camera's draw passes build their
/// drawing settings from the one tag `SRPDefaultUnlit`, the tag of a pass
/// with no LightMode (`SekaiDrawUIPass.Setup` takes `SekaiShaderTag.GetTag(0)`,
/// whose default arm is that literal); the second UI-Uber pass (LightMode
/// `SekaiUI`) is drawn only by the custom render loop that MySekai's URP
/// asset does not run, so each batch is drawn exactly once.
fn ui_pass<'a>(material: &'a Value) -> Result<&'a Value, String> {
    let passes = material["shader"]["shaderPasses"]
        .as_array()
        .ok_or("shader passes not exported; re-extract")?;
    let untagged: Vec<&Value> = passes
        .iter()
        .filter(|pass| pass["lightMode"].is_null())
        .collect();
    match untagged.as_slice() {
        [pass] => Ok(pass),
        other => Err(format!(
            "{} passes without a LightMode; the UI draw takes exactly one",
            other.len()
        )),
    }
}

/// A render-state field: a property-bound one reads the material float (or
/// the shader's default), a fixed one its compiled value, and one bound to
/// `unity_GUIZTestMode` the value the canvas manager sets for the host's
/// canvas (`gui_ztest`).
fn state_value(field: &Value, floats: &Value, gui_ztest: u8, label: &str) -> Result<u8, String> {
    let name = field["name"]
        .as_str()
        .ok_or_else(|| format!("pass {label} has no binding name"))?;
    let value = if name.is_empty() || name == "<noninit>" {
        field["val"].as_f64()
    } else if name == "unity_GUIZTestMode" {
        Some(f64::from(gui_ztest))
    } else {
        floats
            .get(name)
            .and_then(Value::as_f64)
            .or_else(|| field["default"].as_f64())
    }
    .ok_or_else(|| format!("pass {label} has no value"))?;
    if !(0.0..=255.0).contains(&value) || value.fract() != 0.0 {
        return Err(format!("pass {label} value {value} is not a state enum"));
    }
    Ok(value as u8)
}

/// The draw of a UI particle material. Each of the three programs has one
/// variant, compiled without keywords, so a material with keywords is
/// refused. `gui_ztest` is the canvas manager's `unity_GUIZTestMode` for the
/// host's canvas.
pub(crate) fn ui_draw(material: &Value, gui_ztest: u8) -> Result<UiDraw, String> {
    let shader = material["shader"]["name"].as_str().unwrap_or("<unnamed>");
    let program = match shader {
        UI_UBER => UiProgram::Uber,
        UI_DEFAULT => UiProgram::Default,
        UI_ADDITIVE => UiProgram::Additive,
        _ => {
            return Err(format!(
                "material shader {shader}: only {UI_UBER}, {UI_DEFAULT} and {UI_ADDITIVE} draw on a canvas bake here"
            ))
        }
    };
    if material["keywords"]
        .as_array()
        .is_none_or(|k| !k.is_empty())
    {
        return Err(format!(
            "material keywords {}: the {shader} program has one variant, without keywords",
            material["keywords"]
        ));
    }
    let floats = &material["floats"];
    let pass = ui_pass(material)?;
    let state = &pass["renderState"];
    let blend = &state["blend"];
    let read = |field: &Value, label: &str| state_value(field, floats, gui_ztest, label);
    let state = SourceRenderState {
        cull: read(&state["culling"], "culling")?,
        depth_test: read(&state["zTest"], "zTest")?,
        depth_write: read(&state["zWrite"], "zWrite")? == 1,
        color_mask: read(&blend["colMask"], "colMask")?,
        src_color: read(&blend["srcBlend"], "srcBlend")?,
        dst_color: read(&blend["destBlend"], "destBlend")?,
        src_alpha: read(&blend["srcBlendAlpha"], "srcBlendAlpha")?,
        dst_alpha: read(&blend["destBlendAlpha"], "destBlendAlpha")?,
        color_op: read(&blend["blendOp"], "blendOp")?,
        alpha_op: read(&blend["blendOpAlpha"], "blendOpAlpha")?,
    };
    if state.cull > 2
        || state.depth_test > 8
        || state.color_mask > 15
        || [
            state.src_color,
            state.dst_color,
            state.src_alpha,
            state.dst_alpha,
        ]
        .iter()
        .any(|v| *v > 10)
        || state.color_op > 4
        || state.alpha_op > 4
    {
        return Err(format!("pass state {state:?} is outside the engine enums"));
    }
    let blend_mode = if program == UiProgram::Uber {
        let blend_mode = floats["_BlendMode"]
            .as_f64()
            .ok_or("material lacks _BlendMode")?;
        // The program switches on the int: 0 and 1 keep the colour, 2
        // multiplies it by alpha, any other value keeps it (the default arm).
        if blend_mode.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&blend_mode) {
            return Err(format!("_BlendMode {blend_mode} is not an int"));
        }
        blend_mode as u32
    } else {
        0
    };
    let color = if program == UiProgram::Default {
        let value = material["colors"]["_Color"]
            .as_array()
            .filter(|v| v.len() == 4)
            .ok_or("material has no four-channel _Color")?;
        let mut color = [0.0f32; 4];
        for (slot, channel) in color.iter_mut().zip(value) {
            *slot = channel
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("_Color channel not finite")? as f32;
        }
        Some(color)
    } else {
        None
    };
    let texture = material["textures"]["_MainTex"]
        .as_str()
        .ok_or("material has no _MainTex texture")?
        .to_owned();
    let st = material["textureScaleOffset"]["_MainTex"]
        .as_array()
        .filter(|v| v.len() == 4)
        .ok_or("material has no _MainTex scale and offset")?;
    let mut main_tex_st = [0.0f32; 4];
    for (slot, value) in main_tex_st.iter_mut().zip(st) {
        *slot = value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("_MainTex scale and offset not finite")? as f32;
    }
    Ok(UiDraw {
        program,
        texture,
        main_tex_st,
        blend_mode,
        color,
        state,
        identity: material.to_string(),
    })
}

fn triple(value: &Value) -> Option<[f32; 3]> {
    let list = value.as_array().filter(|list| list.len() == 3)?;
    let mut out = [0.0f32; 3];
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value.as_f64().filter(|v| v.is_finite())? as f32;
    }
    Some(out)
}

/// Admits one row whose renderer has a material. `local_scale` is the
/// system node's own local scale (the Local scaling mode reads it);
/// `has_rigidbody` whether the prefab carries a Rigidbody (the Rigidbody
/// emitter velocity mode falls back to the Transform mode without one, as the
/// weather host resolves it); `gui_ztest` the canvas manager's
/// `unity_GUIZTestMode` for the host's canvas.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit(
    row: &Value,
    local_scale: Vec3,
    has_rigidbody: bool,
    gui_ztest: u8,
    mesh: Handle<Mesh>,
    host: Entity,
    ordinal: u64,
) -> Result<Admitted, String> {
    let node = row["node"].as_str().unwrap_or("").to_owned();
    let renderer = row
        .get("renderer")
        .filter(|v| v.is_object())
        .ok_or("no renderer")?;
    let draw = ui_draw(&renderer["material"], gui_ztest)?;
    // The renderer's sorting layer, sorting order and sprite-mask interaction
    // are not read on this path. The UIParticle turns the renderer off at
    // OnEnable, so the renderer's own draw (where the engine picks its render
    // function by mask interaction and sorts by layer and order) never runs;
    // the engine's BakeMesh does not read the mask interaction field, and no
    // managed code can (the build has no accessor for it on any renderer).
    // The one managed reader of the sorting layer and order is the
    // UIParticle's SortForRendering, which runs only when OnEnable finds
    // `m_Particles` empty; the host refuses an empty list.
    let mode = match renderer["renderMode"].as_str().unwrap_or("") {
        "Billboard" => crate::source_billboard::Mode::Billboard,
        "HorizontalBillboard" => crate::source_billboard::Mode::Horizontal,
        "VerticalBillboard" => crate::source_billboard::Mode::Vertical,
        other => return Err(format!("render mode {other} is not baked on this path")),
    };
    let alignment_id = renderer["alignment"].as_i64().unwrap_or(-1);
    let alignment = crate::particle_geometry::Alignment::from_source(alignment_id)
        .ok_or_else(|| format!("render alignment {alignment_id}"))?;
    if matches!(
        alignment,
        crate::particle_geometry::Alignment::Facing | crate::particle_geometry::Alignment::Velocity
    ) {
        // Facing reads the particle's world position against the camera,
        // which the bake takes without the system's position; Velocity reads
        // the simulation-space velocity basis. Neither is wired here.
        return Err(format!(
            "render alignment {alignment:?} is not baked on this path"
        ));
    }
    if renderer["flip"]
        .as_array()
        .is_none_or(|v| v.len() != 3 || v.iter().any(|x| x.as_f64() != Some(0.0)))
    {
        return Err("particle flip is not consumed".into());
    }
    let pivot = triple(&renderer["pivot"]).ok_or("renderer pivot")?;
    let max_size = renderer["maxParticleSize"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f32::MAX as f64);
    let min_size = renderer["minParticleSize"]
        .as_f64()
        .filter(|v| v.is_finite());
    let allow_roll = renderer["allowRoll"].as_bool();
    let (Some(max_size), Some(min_size), Some(allow_roll)) = (max_size, min_size, allow_roll)
    else {
        return Err("renderer size limits or roll flag missing".into());
    };
    if min_size != 0.0 {
        return Err(format!(
            "minParticleSize {min_size}: the orthographic bake's lower limit is not wired"
        ));
    }
    let sort_mode = renderer["sortMode"]
        .as_u64()
        .and_then(|mode| u32::try_from(mode).ok())
        .and_then(moly_law::particle::sort::ParticleSort::from_source)
        .ok_or("particle sort mode absent or unsupported")?;
    if sort_mode != moly_law::particle::sort::ParticleSort::None {
        return Err(format!(
            "particle sort {sort_mode:?} against the orthographic baking camera is not wired"
        ));
    }

    let system = row
        .get("system")
        .filter(|v| v.is_object())
        .ok_or("no system block")?;
    ParticleSourceModules::from_system(system)?;
    let scaling = match system["scalingMode"].as_u64() {
        Some(0) => crate::particle_geometry::Scaling::Hierarchy,
        // The chain above a UIParticle system carries the root canvas scale
        // and the UIParticle's driven scale, never all ones; only a World
        // system with the Local scaling mode reads the flag.
        Some(1) => crate::particle_geometry::Scaling::Local {
            scale: local_scale,
            unit_chain: false,
        },
        other => return Err(format!("scalingMode {other:?}")),
    };
    let archive = serde_json::json!({
        "effects": { "ui": { "particles": [{ "node": node, "system": system.clone() }] } }
    });
    let mut effects =
        Effects::from_json_str(archive.to_string().as_bytes()).map_err(|err| format!("{err}"))?;
    if effects.emitters.len() != 1 {
        panic!(
            "particle law parse of one row returned {} emitters ({node})",
            effects.emitters.len()
        );
    }
    let mut emitter: EmitterParams = effects.emitters.remove(0);
    if emitter.use_unscaled_time.is_none() {
        return Err("useUnscaledTime not exported; re-extract".into());
    }
    let space = match emitter.simulation_space {
        SimulationSpace::Local => super::bake::Space::Local,
        SimulationSpace::World => super::bake::Space::World,
        other => {
            return Err(format!(
                "simulation space {other:?} is not baked on this path"
            ))
        }
    };
    crate::weather_fx::resolve_velocity_mode(
        &mut emitter,
        &if has_rigidbody {
            Err("the prefab carries a Rigidbody".to_owned())
        } else {
            Ok(())
        },
    );
    if !emitter.play_on_awake {
        return Err(
            "a system without playOnAwake waits for an explicit Play, which no host sends yet"
                .into(),
        );
    }
    if !emitter.sub_emitters.is_empty()
        || crate::particle_runtime::has_real_sub_emitter_edges(&emitter)
    {
        return Err("sub-emitters: this host installs no sub-emitter target".into());
    }
    if emitter.trails.is_some() {
        return Err("TrailModule: the trail's second bake is not wired".into());
    }
    if emitter.collision.is_some() {
        return Err("CollisionModule: a canvas bake has no collision scene".into());
    }
    if emitter.ring_buffer_mode != moly_law::particle::RingBufferMode::Disabled {
        return Err("ring buffer mode: not verified on this host".into());
    }
    // A constant start delay: the first Play writes it to the system state's
    // start delay word and the native update counts it down (the native birth
    // path is required below). A random one is evaluated with the system
    // seed's hash, which is not transcribed.
    if crate::particle_runtime::has_start_delay(&emitter)
        && crate::particle_runtime::play_start_delay(&emitter).is_none()
    {
        return Err("random start delay: Play's seed-hash evaluation is not transcribed".into());
    }
    if crate::particle_runtime::has_distance_emission(&emitter) {
        return Err(
            "emission over distance: the canvas host reads no emitter translation history".into(),
        );
    }
    if let Some(shape) = &emitter.shape {
        match &shape.controls.texture {
            Some(ShapeTexture::None) => {}
            None => return Err("missing source shape texture reference; re-extract".into()),
            Some(ShapeTexture::Reference { .. }) => {
                return Err("source shape texture (ApplyTexture) not transcribed".into())
            }
        }
    }
    if let Some(sheet) = &emitter.texture_sheet {
        moly_law::particle::texture_sheet::TextureSheet::from_params(sheet)
            .map_err(|e| e.to_string())?;
    }
    if let Some(noise) = &emitter.noise {
        moly_law::particle::noise::NoiseLaw::from_params(noise).map_err(|e| e.to_string())?;
    }
    if let Some(force) = &emitter.force {
        moly_law::particle::force::ForceOverLifetime::from_params(force)
            .map_err(|e| e.to_string())?;
    }
    let route = crate::particle_runtime::source_route(system);
    let evidence = crate::particle_runtime::ShapeEmitterEvidence {
        scaling,
        mesh_renderer: false,
    };
    crate::particle_runtime::native_birth_path(&emitter, &route, Some(evidence))
        .map_err(|reason| format!("the native birth path is required here: {reason}"))?;
    let storage = || crate::particle_runtime::custom_data_storage_eligible(&emitter, &route);
    let size_storage = || crate::particle_runtime::size_storage_eligible(&emitter, &route);
    crate::particle_runtime::curve_admission_with(&emitter, Some(&storage), Some(&size_storage))?;
    let rol = emitter
        .rotation_over_lifetime
        .as_ref()
        .map(|p| {
            RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve)
        })
        .transpose()
        .map_err(|e| format!("rotationOverLifetime: {e}"))?;
    let limit = emitter
        .limit_velocity
        .as_ref()
        .map(|p| {
            LimitVelocity::from_parts(
                p.separate_axis,
                &p.magnitude,
                p.dampen,
                p.drag.as_ref(),
                p.multiply_drag_by_size,
                p.multiply_drag_by_velocity,
            )
        })
        .transpose()
        .map_err(|e| format!("limitVelocity: {e}"))?;
    if let Some(params) = &emitter.rotation_by_speed {
        moly_law::particle::rotation_by_speed::RotationBySpeed::from_params(params)
            .map_err(|e| format!("rotationBySpeed: {e}"))?;
    }
    let cone_angle = match emitter.shape.as_ref().map(|s| s.shape_type.as_str()) {
        Some("Cone" | "ConeVolume") => Some(
            emitter
                .shape
                .as_ref()
                .and_then(|s| s.controls.angle)
                .ok_or("cone without an angle")?,
        ),
        _ => None,
    };
    let geometry = Geometry::SourceBillboard(crate::source_billboard::Draw {
        mode,
        alignment,
        pivot: Vec3::from_array(pivot),
        // The bake camera is orthographic; the host checks its size limits
        // each frame (they do not bind for any admitted particle) and hands
        // the geometry a disabled upper limit, so the perspective body of the
        // limit is never read.
        screen_size: Vec2::new(0.0, -1.0),
        allow_roll,
        scaling,
    });
    let runtime = Runtime {
        node,
        effect: "UIParticle".to_owned(),
        kind: EffectKind::Site,
        camera_rotation: false,
        node_affine: GlobalTransform::IDENTITY,
        mesh,
        anchor: Some(host),
        geometry,
        emission_surface: None,
        ring_cursor: 0,
        pool: Vec::new(),
        side: Vec::new(),
        emission: EmissionState::default(),
        playback_head: 0.0,
        previous_head: 0.0,
        pending: 0.0,
        emission_started: false,
        native_birth: None,
        noise: None,
        trail: None,
        collision: None,
        rng: Rng(RNG_SEED ^ ordinal.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        prewarmed: false,
        sub_emitter_max_lifetime: 0.0,
        cone_angle,
        rol,
        limit,
        velocity_law: emitter.velocity_over_lifetime.as_ref().map(|p| {
            moly_law::particle::velocity::VelocityOverLifetime::from_params(p)
                .expect("curves validated during admission")
        }),
        force_law: emitter.force.as_ref().map(|p| {
            moly_law::particle::force::ForceOverLifetime::from_params(p)
                .expect("force validated during admission")
        }),
        gravity_law: moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier)
            .expect("curves validated during admission"),
        custom_law: emitter.custom_data.as_ref().map(|p| {
            moly_law::particle::custom_data::CustomData::from_params(p)
                .expect("curves validated during admission")
        }),
        texture_sheet: emitter.texture_sheet.as_ref().map(|p| {
            moly_law::particle::texture_sheet::TextureSheet::from_params(p)
                .expect("sheet validated during admission")
        }),
        sort_mode,
        size_law: emitter.size_over_lifetime.as_ref().map(|p| {
            moly_law::particle::size::SizeOverLifetime::from_params(p)
                .expect("curves validated during admission")
        }),
        color_law: emitter
            .color_over_lifetime
            .as_ref()
            .map(moly_law::particle::color::ColorOverLifetime::from_params),
        emitter,
        born_total: 0,
        died_total: 0,
        full_total: 0,
        refused_total: 0,
    };
    Ok(Admitted {
        runtime,
        route,
        draw,
        space,
        max_particle_size: max_size as f32,
    })
}
