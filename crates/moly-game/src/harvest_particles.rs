//! Harvest stay particles: the particle systems a stone view plays on its
//! own prefab, drawn with the Hidden/particle_circle program.
//!
//! Source behaviour: `MysekaiAreaStoneView.Setup` plays `_objectParticle`
//! for every stone, and `RefreshParticle` activates and plays
//! `rareParticleSystem` for a rare stone (a common stone deactivates and
//! stops it). `Play()` plays the named system and every particle system
//! under its GameObject. Both fields name ParticleSystem components on the
//! prefab; the prop document lists every particle system of the prefab
//! (`particles[]`: node path, component id, renderer, system), and each glb
//! node carries the component ids of its GameObject, so a row binds to its
//! scene node by component id.
//!
//! Drawn here: rows whose material shader is Hidden/particle_circle (the
//! `fake_light` glow under the rare system of the five common stone
//! packages). The fragment is the source program's: a radial falloff
//! `max(1 - 2|uv - 0.5|, 0)^1.25` times the vertex alpha, and the vertex
//! colour blended toward the phenomena directional light colour by its w
//! when the material's `_UsePhenomenaLighting` is above 0.5. No harvest
//! view writes that float (the source's SetPhenomenaLighting callers are
//! the fixture, road, room and preview paths), so the authored value is
//! read. Blend SrcAlpha/One (alpha One/One), no depth write, back-face
//! cull, as the pass states. The second colour target (a constant black
//! with alpha 1) is not drawn, as for every site family.
//!
//! Not drawn, and logged by name on every install: the other played rows
//! (Mysekai/Effect/UberUnlit: `pt`, `sekai_circle`, `ray`, `sekai_01`,
//! `rainbow`), because the prop document carries no program catalogue for
//! them; rows without a material are system roots that draw nothing.
//!
//! Each installed system is played as the fixture host plays its systems
//! (`weather_fx::fixture::Played`): the Play installs the birth owner where
//! the route and modules qualify, and each frame reads the clock its
//! useUnscaledTime selects. A row that does not carry that flag is refused by
//! name rather than read as either clock (site prop documents exported before
//! the extractor wrote it carry none).
//!
//! The draw follows the anchor's visibility: a harvested stone is hidden
//! with its whole hierarchy, and so is its glow.
//!
//! Reachability: harvest objects are placed only by the development preview
//! gallery (`MOLY_HARVEST_PREVIEW=1`); ordinary play places none, so this
//! glow is drawn only there.

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::{lifetimeless::SRes, SystemParamItem};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_assets::particle_source::ParticleSourceModules;
use moly_assets::source_navigation::{SourceHarvestView, SourceObjectIdentity};
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::{EmissionState, Effects, EmitterParams, MinMaxCurve};
use serde_json::Value;

use crate::env::SiteEnvGpuBuffer;
use crate::harvest::{HarvestDocs, HarvestObject, HarvestRoot, HarvestScenesReady};
use crate::particle_runtime::{EffectKind, Geometry, Rng, Runtime};
use crate::uber_particle::FixtureParticleLive;

const CIRCLE_SHADER: &str = "Hidden/particle_circle";
const STONE_VIEW: &str = "MysekaiAreaStoneView";
/// Birth stream of this path, distinct from the weather and fixture streams.
const RNG_SEED: u64 = 0x6861_7276_0001_0001;

/// Hidden/particle_circle material: the one material float the program reads.
#[derive(Asset, TypePath, Clone)]
pub(crate) struct ParticleCircleMaterial {
    use_phenomena_lighting: f32,
}

impl AsBindGroup for ParticleCircleMaterial {
    type Data = ();
    type Param = SRes<SiteEnvGpuBuffer>;
    fn label() -> &'static str {
        "particle_circle_material"
    }
    fn bind_group_data(&self) -> Self::Data {}
    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _device: &RenderDevice,
        env: &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let bytes = [self.use_phenomena_lighting, 0.0, 0.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        Ok(UnpreparedBindGroup {
            bindings: BindingResources(vec![
                (0, OwnedBindingResource::Data(OwnedData(bytes))),
                (1, OwnedBindingResource::Buffer(env.buffer.clone())),
            ]),
        })
    }
    fn bind_group_layout_entries(
        _device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![uniform(0), uniform(1)]
    }
}

impl Material for ParticleCircleMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/particle_circle.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Slots are the contract with `CircleVertex` in the shader.
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?];
        // Pass state: Cull Back, ZTest LEqual (reversed depth here), ZWrite Off.
        descriptor.primitive.cull_mode = Some(Face::Back);
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = false;
            depth.depth_compare = CompareFunction::GreaterEqual;
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::SrcAlpha,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                });
            }
        }
        Ok(())
    }
}

/// The stay particles of this placement were resolved (installed or refused).
#[derive(Component)]
struct HarvestStayResolved;

/// One installed stay particle draw and the scene node it follows.
#[derive(Component)]
struct HarvestStayDraw {
    anchor: Entity,
}

/// One admitted Hidden/particle_circle row.
struct Admitted {
    node: String,
    emitter: EmitterParams,
    /// The exported system block: its route decision ([`source_route`]) and,
    /// with it, whether the native birth owner is installable.
    system: Value,
    draw: crate::source_billboard::Draw,
    sort_mode: moly_law::particle::sort::ParticleSort,
    use_phenomena_lighting: f32,
}

/// Update: after every harvest scene is spawned, resolve each stone view's
/// played particle systems once and install the particle_circle rows.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn install_stay_particles(
    mut commands: Commands,
    ready: Option<Res<HarvestScenesReady>>,
    docs: Option<Res<HarvestDocs>>,
    json: Res<Assets<JsonAsset>>,
    roots: Query<(Entity, &HarvestObject), (With<HarvestRoot>, Without<HarvestStayResolved>)>,
    children: Query<&Children>,
    views: Query<&SourceHarvestView>,
    identities: Query<&SourceObjectIdentity>,
    transforms: Query<&Transform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut circles: ResMut<Assets<ParticleCircleMaterial>>,
    mut seeds: ResMut<crate::particle_runtime::seed::SystemSeedManager>,
    mut ordinal: Local<u64>,
) {
    if ready.is_none() {
        return;
    }
    let Some(docs) = docs else {
        return;
    };
    for (root, object) in &roots {
        // Stone views live on the prefab root node; component ids name the
        // node of each particle system's GameObject.
        let mut stone_views = Vec::new();
        let mut by_component: HashMap<i64, Vec<Entity>> = HashMap::new();
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            if let Ok(view) = views.get(entity) {
                if view.class == STONE_VIEW {
                    stone_views.push(view);
                }
            }
            if let Ok(identity) = identities.get(entity) {
                for component in &identity.components {
                    by_component.entry(*component).or_default().push(entity);
                }
            }
        }
        let view = match stone_views.as_slice() {
            [] => {
                commands.entity(root).insert(HarvestStayResolved);
                continue;
            }
            [view] => *view,
            many => panic!(
                "harvest placement {}#{} has {} stone views; one prefab holds one",
                object.leaf,
                object.fixture_id,
                many.len()
            ),
        };
        let Some(handle) = docs.0.get(&object.package) else {
            panic!("harvest stay particles: no document handle for {}", object.package);
        };
        let Some(doc) = json.get(handle) else {
            continue; // not loaded yet; the material swap waits on the same handle
        };
        let doc: Value = serde_json::from_str(&doc.0)
            .unwrap_or_else(|err| panic!("prop document of {} is not JSON: {err}", object.leaf));
        let rows = doc["particles"]
            .as_array()
            .unwrap_or_else(|| panic!("prop document of {} has no particles list", object.leaf));
        let fields: Value = serde_json::from_str(&view.fields_json)
            .unwrap_or_else(|err| panic!("stone view fields of {} are not JSON: {err}", object.leaf));

        // Setup plays _objectParticle only when it is not null (a null reference
        // is valid source input and simply plays nothing); RefreshParticle(Rare)
        // dereferences rareParticleSystem without a null check.
        let mut played = Vec::new();
        if let Some(component) = system_reference(&fields, "_objectParticle", &object.leaf) {
            played.push(("_objectParticle", component));
        }
        if object.is_rare {
            let component = system_reference(&fields, "rareParticleSystem", &object.leaf).unwrap_or_else(|| {
                panic!("{}: stone view rareParticleSystem is null, which RefreshParticle(Rare) dereferences", object.leaf)
            });
            played.push(("rareParticleSystem", component));
        }
        let mut installed = Vec::new();
        let mut not_drawn: Vec<String> = Vec::new();
        let mut refused: Vec<String> = Vec::new();
        let mut material_less = 0usize;
        for (field, component) in played {
            let root_row = rows
                .iter()
                .find(|row| row["pathId"].as_i64() == Some(component))
                .unwrap_or_else(|| {
                    panic!(
                        "{}: stone view {field} names particle system {component}, which is not a particle row of the document",
                        object.leaf
                    )
                });
            let prefix = root_row["node"].as_str().expect("particle row node path").to_owned();
            let subtree = rows.iter().filter(|row| {
                row["node"]
                    .as_str()
                    .is_some_and(|node| node == prefix || node.starts_with(&format!("{prefix}/")))
            });
            for row in subtree {
                let node = row["node"].as_str().unwrap_or("").to_owned();
                let material = &row["renderer"]["material"];
                if !material.is_object() {
                    material_less += 1;
                    continue;
                }
                let shader = material["shader"]["name"].as_str().unwrap_or("<unnamed>");
                if shader != CIRCLE_SHADER {
                    not_drawn.push(format!("{node} ({shader})"));
                    continue;
                }
                let path_id = row["pathId"].as_i64().expect("particle row component id");
                let anchor = match by_component.get(&path_id).map(Vec::as_slice) {
                    Some([anchor]) => *anchor,
                    other => {
                        refused.push(format!(
                            "{node}: component {path_id} binds {} scene nodes, needs exactly one",
                            other.map_or(0, <[Entity]>::len)
                        ));
                        continue;
                    }
                };
                let local_scale = transforms.get(anchor).map(|t| t.scale).unwrap_or_else(|_| {
                    panic!("{}: stay particle node {node} has no Transform", object.leaf)
                });
                match admit(row, local_scale) {
                    Ok(admitted) => installed.push((anchor, admitted)),
                    Err(reason) => refused.push(format!("{node}: {reason}")),
                }
            }
        }
        let names: Vec<String> = installed.iter().map(|(_, a)| a.node.clone()).collect();
        for (anchor, admitted) in installed {
            let mesh = meshes.add(crate::billboard::empty_mesh());
            let material = circles.add(ParticleCircleMaterial {
                use_phenomena_lighting: admitted.use_phenomena_lighting,
            });
            let route = route_of(&admitted);
            let mut runtime = runtime(admitted, anchor, mesh.clone(), *ordinal);
            *ordinal += 1;
            // `Setup` plays `_objectParticle` (and `RefreshParticle` the rare
            // one): `ParticleSystem.Play()` on the prefab's root system with
            // its children, which installs the birth owner of every system it
            // plays.
            let played = match crate::weather_fx::fixture::install_played(&mut runtime, &route, &mut seeds) {
                Ok(played) => played,
                Err(reason) => {
                    refused.push(format!("{}: {reason}", runtime.node));
                    continue;
                }
            };
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::default(),
                NoFrustumCulling,
                crate::shadowmap::NoShadowCast,
                FixtureParticleLive(runtime),
                played,
                HarvestStayDraw { anchor },
            ));
        }
        info!(
            "[harvest-stay] {}#{} rare {}: {} particle_circle installed {:?}; not drawn {} {:?}; system roots without material {}",
            object.leaf,
            object.fixture_id,
            object.is_rare,
            names.len(),
            names,
            not_drawn.len(),
            not_drawn,
            material_less,
        );
        if !refused.is_empty() {
            warn!(
                "[harvest-stay] {}#{} refused {}: {:?}",
                object.leaf,
                object.fixture_id,
                refused.len(),
                refused
            );
        }
        commands.entity(root).insert(HarvestStayResolved);
    }
}

/// A stone view field that names a ParticleSystem in the prefab's own file,
/// or `None` for a null reference (serialized `id` 0, or null).
fn system_reference(fields: &Value, field: &str, leaf: &str) -> Option<i64> {
    let reference = fields
        .get(field)
        .unwrap_or_else(|| panic!("{leaf}: stone view has no field {field}"));
    if reference.is_null() || reference["id"].as_str() == Some("0") || reference["id"].as_i64() == Some(0) {
        return None;
    }
    if reference["file"].as_i64() != Some(0) {
        panic!("{leaf}: stone view {field} is not a component of the prefab itself: {reference}");
    }
    let id = reference["id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("{leaf}: stone view {field} names no particle system: {reference}"));
    Some(id)
}

/// Admission of one Hidden/particle_circle row. Every consumed control is
/// checked; anything this path does not install is a named refusal.
fn admit(row: &Value, local_scale: Vec3) -> Result<Admitted, String> {
    let node = row["node"].as_str().unwrap_or("").to_owned();
    let renderer = row.get("renderer").filter(|v| v.is_object()).ok_or("no renderer")?;
    if renderer["enabled"].as_bool() != Some(true) {
        return Err("renderer disabled".into());
    }
    let material = &renderer["material"];
    if material["keywords"].as_array().is_none_or(|k| !k.is_empty()) {
        return Err(format!("material keywords {} are not read by this program", material["keywords"]));
    }
    let use_phenomena_lighting = material["floats"]["_UsePhenomenaLighting"]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or("material lacks _UsePhenomenaLighting")? as f32;
    pass_state(material)?;
    if renderer["sortingLayerId"].as_i64() != Some(0) || renderer["maskInteraction"].as_i64() != Some(0) {
        return Err("sorting layer or sprite mask needs its renderer owner".into());
    }
    let render_mode = renderer["renderMode"].as_str().unwrap_or("");
    let mode = match render_mode {
        "Billboard" => crate::source_billboard::Mode::Billboard,
        "HorizontalBillboard" => crate::source_billboard::Mode::Horizontal,
        "VerticalBillboard" => crate::source_billboard::Mode::Vertical,
        other => return Err(format!("render mode {other} is not installed on this path")),
    };
    let alignment_id = renderer["alignment"].as_i64().unwrap_or(-1);
    let alignment = crate::particle_geometry::Alignment::from_source(alignment_id)
        .ok_or_else(|| format!("render alignment {alignment_id}"))?;
    if renderer["normalDirection"].as_f64() != Some(1.0) {
        return Err("billboard normalDirection other than one is not verified".into());
    }
    if renderer["flip"].as_array().is_none_or(|v| v.len() != 3 || v.iter().any(|x| x.as_f64() != Some(0.0))) {
        return Err("particle flip is not consumed".into());
    }
    let pivot = triple(&renderer["pivot"]).ok_or("renderer pivot")?;
    let max_size = renderer["maxParticleSize"].as_f64().filter(|v| v.is_finite());
    let min_size = renderer["minParticleSize"].as_f64().filter(|v| v.is_finite());
    let allow_roll = renderer["allowRoll"].as_bool();
    let (Some(max_size), Some(min_size), Some(allow_roll)) = (max_size, min_size, allow_roll) else {
        return Err("renderer size limits or roll flag missing".into());
    };
    if min_size < 0.0 || max_size < min_size || max_size > f32::MAX as f64 {
        return Err(format!("renderer size limits {min_size}..{max_size}"));
    }
    let sort_mode = renderer["sortMode"]
        .as_u64()
        .and_then(|mode| u32::try_from(mode).ok())
        .and_then(moly_law::particle::sort::ParticleSort::from_source)
        .ok_or("particle sort mode absent or unsupported")?;

    let system = row.get("system").filter(|v| v.is_object()).ok_or("no system block")?;
    let modules = ParticleSourceModules::from_system(system)?;
    for module in &modules.enabled {
        if !matches!(module.as_str(), "InitialModule" | "EmissionModule" | "ColorModule" | "SizeModule") {
            return Err(format!("enabled module {module} is not installed on this path"));
        }
    }
    if system["ringBufferMode"].as_u64() != Some(0) {
        return Err("ring buffer lifecycle is not verified".into());
    }
    if system["start"]["randomizeRotationDirection"].as_f64() != Some(0.0) {
        return Err("birth rotation direction randomization is not consumed".into());
    }
    if system["start"]["rotation3D"].as_bool() != Some(false) {
        return Err("three-axis start rotation is not installed on this path".into());
    }
    if system["subEmitters"].as_array().is_some_and(|entries| !entries.is_empty()) {
        return Err("sub-emitters are not installed on this path".into());
    }
    if system.get("shape").is_some_and(|shape| shape.as_object().is_some_and(|o| !o.is_empty()))
        || system["shapeEnabled"].as_bool() != Some(false)
    {
        return Err("Shape module is not installed on this path".into());
    }
    let emission = system.get("emission").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        .ok_or("no emission")?;
    if raw_const(&emission["rateOverDistance"]) != Some(0.0) {
        return Err("rate over distance is not installed".into());
    }
    let bursts = emission["bursts"].as_array().map_or(0, Vec::len);
    if bursts > 0 {
        return Err("bursts are not installed on this path".into());
    }
    if raw_const(&emission["rateOverTime"]).is_none_or(|rate| rate <= 0.0) {
        return Err("rate over time is not a positive constant".into());
    }
    if !matches!(system["simulationSpace"].as_str(), Some("Local") | Some("World")) {
        return Err(format!("simulation space {}", system["simulationSpace"]));
    }
    let scaling = match system["scalingMode"].as_u64() {
        Some(0) => crate::particle_geometry::Scaling::Hierarchy,
        // Local: this node's own scale. unit_chain only gates the native
        // birth path, which this path does not install.
        Some(1) => crate::particle_geometry::Scaling::Local { scale: local_scale, unit_chain: false },
        other => return Err(format!("scalingMode {other:?}")),
    };

    let archive = serde_json::json!({
        "effects": { "harvest": { "particles": [{ "node": node, "system": system.clone() }] } }
    });
    let mut effects = Effects::from_json_str(archive.to_string().as_bytes()).map_err(|err| format!("{err}"))?;
    if effects.emitters.len() != 1 {
        panic!("particle law parse of one row returned {} emitters ({node})", effects.emitters.len());
    }
    let emitter = effects.emitters.remove(0);
    let uninstalled = [
        ("velocityOverLifetime", emitter.velocity_over_lifetime.is_some()),
        ("rotationOverLifetime", emitter.rotation_over_lifetime.is_some()),
        ("limitVelocity", emitter.limit_velocity.is_some()),
        ("customData", emitter.custom_data.is_some()),
        ("forceOverLifetime", emitter.force.is_some()),
        ("textureSheet", emitter.texture_sheet.is_some()),
        ("noise", emitter.noise.is_some()),
        ("collision", emitter.collision.is_some()),
        ("trails", emitter.trails.is_some()),
        ("inheritVelocity", emitter.inherit_velocity.is_some()),
        ("shape", emitter.shape.is_some()),
    ];
    if let Some((name, _)) = uninstalled.iter().find(|(_, present)| *present) {
        return Err(format!("law parse carries {name}, which this path does not install"));
    }
    if emitter.shape_enabled != Some(false) {
        return Err("missing shape requires explicit disabled-module evidence".into());
    }
    match emitter.start_delay {
        MinMaxCurve::Constant(v) if v == 0.0 => {}
        _ => return Err("start delay other than zero".into()),
    }
    if !matches!(emitter.simulation_space, SimulationSpace::Local | SimulationSpace::World) {
        return Err("simulation space".into());
    }
    // Every curve the runtime evaluates goes through the engine's curve
    // dispatch; a lane outside the transcribed evaluator refuses the system
    // here, before any law is installed (the runtime below relies on it).
    crate::particle_runtime::curve_admission(&emitter)?;
    // ParticleSystem::BeginUpdate reads Time.unscaledDeltaTime for a system
    // with useUnscaledTime and Time.deltaTime otherwise; a row that does not
    // carry the flag is refused rather than read as either clock, as in the
    // weather and fixture hosts.
    if emitter.use_unscaled_time.is_none() {
        return Err("useUnscaledTime not exported; re-extract".into());
    }
    Ok(Admitted {
        node,
        emitter,
        system: system.clone(),
        draw: crate::source_billboard::Draw {
            mode,
            alignment,
            pivot: Vec3::from_array(pivot),
            screen_size: Vec2::new(min_size as f32, max_size as f32),
            allow_roll,
            scaling,
        },
        sort_mode,
        use_phenomena_lighting,
    })
}

/// The pass render state the fragment port assumes, fixed in the shader
/// (not bound to material properties).
fn pass_state(material: &Value) -> Result<(), String> {
    let passes = material["shader"]["shaderPasses"].as_array().ok_or("shader passes absent")?;
    let [pass] = passes.as_slice() else {
        return Err(format!("{} shader passes, the port has one", passes.len()));
    };
    let state = &pass["renderState"];
    let blend = &state["blend"];
    for (label, value, expected) in [
        ("srcBlend", &blend["srcBlend"], 5.0),
        ("destBlend", &blend["destBlend"], 1.0),
        ("srcBlendAlpha", &blend["srcBlendAlpha"], 1.0),
        ("destBlendAlpha", &blend["destBlendAlpha"], 1.0),
        ("blendOp", &blend["blendOp"], 0.0),
        ("blendOpAlpha", &blend["blendOpAlpha"], 0.0),
        ("colMask", &blend["colMask"], 15.0),
        ("culling", &state["culling"], 2.0),
        ("zTest", &state["zTest"], 4.0),
        ("zWrite", &state["zWrite"], 0.0),
    ] {
        if value["name"].as_str() != Some("<noninit>") {
            return Err(format!("pass {label} is bound to property {}", value["name"]));
        }
        if value["val"].as_f64() != Some(expected) {
            return Err(format!("pass {label} {} (port assumes {expected})", value["val"]));
        }
    }
    Ok(())
}

fn raw_const(value: &Value) -> Option<f64> {
    match value["mode"].as_str() {
        Some("constant") => value["value"].as_f64(),
        Some("twoConstants") => {
            let (min, max) = (value["min"].as_f64()?, value["max"].as_f64()?);
            (min == max).then_some(min)
        }
        _ => None,
    }
}

fn triple(value: &Value) -> Option<[f32; 3]> {
    let list = value.as_array().filter(|list| list.len() == 3)?;
    let mut out = [0.0f32; 3];
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value.as_f64().filter(|v| v.is_finite())? as f32;
    }
    Some(out)
}

fn runtime(admitted: Admitted, anchor: Entity, mesh: Handle<Mesh>, ordinal: u64) -> Runtime {
    let Admitted { node, emitter, draw, sort_mode, system, .. } = admitted;
    Runtime {
        node,
        effect: STONE_VIEW.to_owned(),
        kind: EffectKind::Site,
        camera_rotation: false,
        node_affine: GlobalTransform::IDENTITY,
        mesh,
        anchor: Some(anchor),
        geometry: Geometry::SourceBillboard(draw),
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
        cone_angle: None,
        rol: None,
        limit: None,
        velocity_law: None,
        force_law: None,
        gravity_law: moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier)
            .expect("curves validated during admission"),
        custom_law: None,
        texture_sheet: None,
        sort_mode,
        size_law: emitter
            .size_over_lifetime
            .as_ref()
            .map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission")),
        color_law: emitter
            .color_over_lifetime
            .as_ref()
            .map(moly_law::particle::color::ColorOverLifetime::from_params),
        emitter,
        born_total: 0,
        died_total: 0,
        full_total: 0,
        refused_total: 0,
    }
}

/// The exported system block of one admitted row, for its route decision.
fn route_of(admitted: &Admitted) -> crate::particle_runtime::SourceRoute {
    crate::particle_runtime::source_route(&admitted.system)
}

/// Update: a draw is visible exactly when its anchor node is (a harvested
/// placement hides its whole hierarchy). The draw holds world-space
/// vertices, so it cannot be parented under the placement.
fn follow_anchor_visibility(
    mut draws: Query<(&HarvestStayDraw, &mut Visibility)>,
    anchors: Query<&InheritedVisibility>,
) {
    for (draw, mut visibility) in &mut draws {
        let shown = anchors.get(draw.anchor).is_ok_and(|v| v.get());
        let wanted = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Harvest stay particle plugin: the particle_circle material pipeline and
/// the install and visibility systems. The particle runtime advance is the
/// shared entity-level one (`advance_fixture_particles`).
pub struct HarvestParticlePlugin;

impl Plugin for HarvestParticlePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/particle_circle.wgsl");
        app.add_plugins(MaterialPlugin::<ParticleCircleMaterial>::default())
            .add_systems(Update, (install_stay_particles, follow_anchor_visibility).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coverage diagnostic, not a correctness test: every particle row of the
    /// release root's site prop documents drawn with the program this host
    /// installs (Hidden/particle_circle), through this host's admission, with
    /// the refusal reason, then counts by reason. The host installs only the
    /// rows under a stone view's played systems; every circle row is judged
    /// here, and the other shaders are counted, not judged. The denominator is
    /// every `site/props` document that carries a `particles` list.
    #[test]
    #[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing a release root's site/props"]
    fn harvest_host_refusal_census() {
        let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
        let mut stack = vec![root.join("site/props")];
        let mut files = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() { stack.push(path); } else if path.extension().is_some_and(|e| e == "json") { files.push(path); }
            }
        }
        files.sort();
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        let (mut documents, mut rows, mut circle, mut unscaled_exported) = (0usize, 0usize, 0usize, 0usize);
        for path in files {
            let Ok(doc) = serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()) else { continue; };
            let Some(particles) = doc.get("particles").and_then(Value::as_array) else { continue; };
            documents += 1;
            for row in particles {
                rows += 1;
                if row.pointer("/system/useUnscaledTime").is_some() { unscaled_exported += 1; }
                if row.pointer("/renderer/material/shader/name").and_then(Value::as_str) != Some(CIRCLE_SHADER) { continue; }
                circle += 1;
                let reason = match admit(row, Vec3::ONE) { Ok(_) => "admitted".to_owned(), Err(reason) => reason };
                println!("harvest-host-census | {} | {} | {reason}", path.file_name().unwrap().to_string_lossy(),
                    row["node"].as_str().unwrap_or(""));
                *reasons.entry(reason).or_default() += 1;
            }
        }
        for (reason, count) in &reasons { println!("harvest-host-census count {count} | {reason}"); }
        println!("harvest-host-census documents {documents} rows {rows} circle-rows {circle} useUnscaledTime-exported {unscaled_exported}");
        assert!(circle > 0, "no Hidden/particle_circle row in the supplied site props");
    }
}
