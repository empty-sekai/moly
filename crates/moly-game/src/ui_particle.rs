//! UIParticle: particle systems simulated by the shared particle runtime and
//! drawn in a canvas, through the source's canvas bake.
//!
//! The source draws UI particles with `Coffee.UIExtensions.UIParticle`
//! (a `MaskableGraphic`). Its `OnEnable` turns every listed system's
//! `ParticleSystemRenderer` off, so the systems simulate as ordinary
//! particle systems and draw only through the UIParticle: once a frame, on
//! `Canvas.willRenderCanvases`, `UIParticleUpdater.Refresh` runs
//! `ModifyScale`, bakes every system of `m_Particles` into one mesh in the
//! UIParticle's local space and hands it to the UIParticle's
//! `CanvasRenderer`. The mesh is therefore drawn as any Graphic is: in the
//! canvas' hierarchy order, with the CanvasRenderer's inherited alpha. The
//! bake itself is [`bake`].
//!
//! Per system, in `m_Particles` order: a system that is not alive or holds
//! no particle, or whose renderer has no material, bakes nothing; while the
//! inherited alpha is approximately zero nothing is baked (the systems still
//! simulate). A baked mesh with vertices is pushed at index `2 i` (its trail
//! at `2 i + 1`); consecutive pushes with the same material hash
//! (`GetMaterialHash`: the material and, in Sprites mode, the sheet texture)
//! merge into the earlier push's submesh and keep its index. `UpdateMaterial`
//! sets one material per submesh in that order, at most eight
//! (`materialCount = min(CountFast(activeMeshIndices), 8)`), so submeshes
//! past the eighth are not drawn. The material is the renderer's own:
//! `GetModifiedMaterial` adds a stencil material only below a Mask
//! (`m_StencilValue >= 1`) and a copy only for a sprite texture or animatable
//! properties.
//!
//! Masking: a maskable UIParticle below a `Mask` is refused here (the
//! stencil material is not ported); none of the packaged UIParticles is
//! below one. The UI-Uber program has no clip-rect term, so a `RectMask2D`
//! clips nothing of it.
//!
//! The draw: the UI camera draws the pass tagged `SRPDefaultUnlit` (the
//! UI-Uber pass without a LightMode) once per batch; see [`admission`]. The
//! fragment is the UI-Uber program's: the texture times the vertex colour, the
//! rgb times alpha for `_BlendMode` 2, with the pass blend
//! `[_SrcBlend] [_DstBlend]`, `ColorMask RGB`, `Cull Off`, `ZTest LEqual`,
//! `ZWrite Off` (`shaders/ui_particle.wgsl`).
//!
//! Host API ([`UiParticleHost`]): a screen that owns a UIParticle node spawns
//! an entity with this component where the node is, as a descendant of the
//! entity whose local space is the root canvas (in canvas units). Its
//! Transform is the node's placement: its x and y, its rotation, and a z that
//! orders it after the graphics drawn before it and before the ones drawn
//! after it; its scale must be one, because the source drives the UIParticle
//! node's own scale (`ModifyScale`; the serialized scale is zero). The
//! systems play when the host becomes visible (every packaged system plays on
//! awake) and are cleared when it is hidden or despawned; `alpha` is the
//! node's inherited CanvasGroup alpha, written by the screen. The draws are
//! spawned under the canvas entity, `(k + 1) * DRAW_STEP` above the host in
//! canvas z for the k-th system.
//!
//! A screen that took over systems which played before it (the entry's
//! cover particle, played in the previous scene and paused there) adds
//! [`UiParticleHeadStart`]: the first baked frame steps the systems through
//! that time first. [`UiParticlePaused`] is `ParticleSystem.Pause(true)`:
//! while the host carries it its systems are not stepped and their
//! particles are baked as they stand (a paused system still renders);
//! removing it resumes them.
//!
//! Named gaps: trails, the particle sort, the Facing and Velocity
//! alignments and the orthographic camera's size clamp are not wired (each
//! refused by name); a hidden host that shows again plays afresh, the
//! convention of the fixture host, not verified for UI particles.

mod admission;
pub(crate) mod bake;
#[cfg(test)]
mod bake_samples;

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use bevy::asset::uuid::Uuid;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::mesh::{MeshVertexBufferLayoutRef, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::material_passes::SourceRenderState;
use serde_json::Value;

use bake::{Node, Space};

/// Canvas z between one UIParticle's successive system draws.
pub(crate) const DRAW_STEP: f32 = 1.0e-5;

/// `UpdateMaterial` sets at most this many materials.
const MAX_MATERIALS: usize = 8;

const SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x7569_7061_7274_6963_6c65_0000_0000_0001),
    PhantomData,
);

/// A UIParticle node of a prefab document, placed by the screen that owns it.
#[derive(Component, Clone, Debug)]
pub(crate) struct UiParticleHost {
    /// The prefab document: its `roots`, `components.RectTransform` and
    /// `components.UIParticle` instances and its `particles` rows.
    pub(crate) document: Handle<JsonAsset>,
    /// The document's directory under the asset root; material textures are
    /// relative to it.
    pub(crate) directory: String,
    /// The document root the node is in.
    pub(crate) root: String,
    /// The UIParticle node's path in that root.
    pub(crate) node: String,
    /// The entity whose local space is the root canvas.
    pub(crate) canvas: Entity,
    /// The render layer of the screen's camera.
    pub(crate) layer: usize,
    /// The node's inherited CanvasGroup alpha.
    pub(crate) alpha: f32,
}

/// `ParticleSystem.Pause(withChildren: true)` on a host's systems.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct UiParticlePaused;

/// The simulated time a host's systems ran before it was installed, stepped
/// on its first baked frame in frames of `frame` seconds.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct UiParticleHeadStart {
    pub(crate) seconds: f32,
    pub(crate) frame: f32,
}

/// The serialized UIParticle fields the bake reads.
#[derive(Clone, Copy, Debug)]
struct Fields {
    ignore_canvas_scaler: bool,
    scale3d: Vec3,
    /// The serialized local scale (driven while the canvas scaler is
    /// ignored, read as it is otherwise).
    local_scale: Vec3,
}

#[derive(Component)]
enum HostState {
    Installed(Installed),
    Refused,
}

struct Installed {
    fields: Fields,
    draws: Vec<Entity>,
    /// The UIParticle node's local scale as `ModifyScale` leaves it.
    driven_scale: Vec3,
    /// `cachedPosition`.
    cached_position: Vec3,
    /// `activeMeshIndices.CountFast() != 0` after the last bake.
    drawn_before: bool,
    /// Whether the host's [`UiParticleHeadStart`] has been stepped.
    head_started: bool,
}

/// One installed system of a UIParticle.
#[derive(Component)]
struct UiParticleSystem {
    host: Entity,
    /// Its index in `m_Particles`.
    order: usize,
    /// Local TRS from the UIParticle node (exclusive) down to the system's
    /// node; empty for a system on the UIParticle node itself.
    chain: Vec<(Vec3, Quat, Vec3)>,
    space: Space,
    /// The render scale of a Local system's bake: its local scale under the
    /// Local scaling mode, `None` (its lossy scale) under Hierarchy.
    local_scale: Option<Vec3>,
    max_particle_size: f32,
    identity: String,
    runtime: crate::particle_runtime::Runtime,
    played: crate::weather_fx::fixture::Played,
    /// A refused step retires the system; it draws nothing more.
    retired: bool,
}

#[derive(Debug, Clone, Copy, ShaderType)]
struct UiUberUniform {
    main_tex_st: Vec4,
    /// x: `_BlendMode`.
    blend_mode: UVec4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct UiUberKey {
    state: SourceRenderState,
}

/// `Mysekai/Effect/UI-Uber` on a canvas.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(UiUberKey)]
pub(crate) struct UiUberMaterial {
    #[uniform(0)]
    value: UiUberUniform,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
    key: UiUberKey,
}

impl From<&UiUberMaterial> for UiUberKey {
    fn from(material: &UiUberMaterial) -> Self {
        material.key
    }
}

impl Material2d for UiUberMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(4),
        ])?];
        crate::source_render_state::apply(descriptor, key.bind_group_data.state);
        Ok(())
    }
}

pub(crate) struct UiParticlePlugin;

impl Plugin for UiParticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<UiUberMaterial>::default());
        app.world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(
                SHADER.id(),
                Shader::from_wgsl(
                    include_str!("shaders/ui_particle.wgsl"),
                    "moly_game/shaders/ui_particle.wgsl",
                ),
            )
            .expect("UI particle shader installation");
        app.add_systems(Update, install).add_systems(
            PostUpdate,
            bake_frame.after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

fn vec3(value: &Value) -> Option<Vec3> {
    let n = |axis: &str| {
        value[axis]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v as f32)
    };
    Some(Vec3::new(n("x")?, n("y")?, n("z")?))
}

fn vec2(value: &Value) -> Option<Vec2> {
    let n = |axis: &str| {
        value[axis]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v as f32)
    };
    Some(Vec2::new(n("x")?, n("y")?))
}

fn quat(value: &Value) -> Option<Quat> {
    let n = |axis: &str| {
        value[axis]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v as f32)
    };
    Some(Quat::from_xyzw(n("x")?, n("y")?, n("z")?, n("w")?))
}

/// One document root: its scene index (the particle rows of its systems
/// carry it) and its RectTransforms by node path. RectTransform instances are
/// listed root by root, each root holding `nodes` of them. Node paths repeat
/// across roots (two roots can both hold `cloud/cloud (1)`), so a component
/// is placed in a root by a link, never by its path alone.
fn document_root<'a>(
    doc: &'a Value,
    root: &str,
) -> Result<(u64, HashMap<&'a str, &'a Value>), String> {
    let roots = doc["roots"].as_array().ok_or("document has no roots")?;
    let rects = doc["components"]["RectTransform"]["instances"]
        .as_array()
        .ok_or("document has no RectTransform instances")?;
    let mut at = 0usize;
    let mut found = None;
    for r in roots {
        let count = r["nodes"].as_u64().ok_or("root without a node count")? as usize;
        let slice = rects
            .get(at..at + count)
            .ok_or("RectTransform instances fewer than the roots' node counts")?;
        if r["name"].as_str() == Some(root) {
            let scene = r["scene"]
                .as_u64()
                .ok_or_else(|| format!("root {root} has no scene index"))?;
            let map: HashMap<&str, &Value> = slice
                .iter()
                .map(|inst| (inst["node"].as_str().unwrap_or(""), &inst["fields"]))
                .collect();
            if found.replace((scene, map)).is_some() {
                return Err(format!("two roots named {root}"));
            }
        }
        at += count;
    }
    if at != rects.len() {
        return Err("RectTransform instances differ from the roots' node counts".into());
    }
    found.ok_or_else(|| format!("document has no root {root}"))
}

/// The rect fields of one node.
struct Rect {
    anchor_min: Vec2,
    anchor_max: Vec2,
    anchored: Vec2,
    size_delta: Vec2,
    pivot: Vec2,
    position_z: f32,
    rotation: Quat,
    scale: Vec3,
}

fn rect(fields: &Value, node: &str) -> Result<Rect, String> {
    let bad = |what: &str| format!("RectTransform {node}: {what} missing or not finite");
    Ok(Rect {
        anchor_min: vec2(&fields["m_AnchorMin"]).ok_or_else(|| bad("m_AnchorMin"))?,
        anchor_max: vec2(&fields["m_AnchorMax"]).ok_or_else(|| bad("m_AnchorMax"))?,
        anchored: vec2(&fields["m_AnchoredPosition"]).ok_or_else(|| bad("m_AnchoredPosition"))?,
        size_delta: vec2(&fields["m_SizeDelta"]).ok_or_else(|| bad("m_SizeDelta"))?,
        pivot: vec2(&fields["m_Pivot"]).ok_or_else(|| bad("m_Pivot"))?,
        position_z: vec3(&fields["m_LocalPosition"])
            .ok_or_else(|| bad("m_LocalPosition"))?
            .z,
        rotation: quat(&fields["m_LocalRotation"]).ok_or_else(|| bad("m_LocalRotation"))?,
        scale: vec3(&fields["m_LocalScale"]).ok_or_else(|| bad("m_LocalScale"))?,
    })
}

/// The local TRS of each node from below `from` down to `to`, from their
/// RectTransforms: the local position is the anchor reference point in the
/// parent's rect plus the anchored position (x, y) and the serialized z. Only
/// point anchors are read (a stretched anchor's size needs the parent's
/// resolved rect).
fn rect_chain(
    rects: &HashMap<&str, &Value>,
    from: &str,
    to: &str,
) -> Result<Vec<(Vec3, Quat, Vec3)>, String> {
    if to == from {
        return Ok(Vec::new());
    }
    let rest = if from.is_empty() {
        to
    } else {
        to.strip_prefix(from)
            .and_then(|r| r.strip_prefix('/'))
            .ok_or_else(|| format!("system node {to} is not below the UIParticle node {from}"))?
    };
    let mut parent_path = from.to_owned();
    let mut out = Vec::new();
    for part in rest.split('/') {
        let path = if parent_path.is_empty() {
            part.to_owned()
        } else {
            format!("{parent_path}/{part}")
        };
        let parent = rect(
            rects
                .get(parent_path.as_str())
                .ok_or_else(|| format!("no RectTransform for {parent_path}"))?,
            &parent_path,
        )?;
        let child = rect(
            rects.get(path.as_str()).ok_or_else(|| {
                format!("no RectTransform for {path} (a plain Transform is not read here)")
            })?,
            &path,
        )?;
        for (name, r) in [(&parent_path, &parent), (&path, &child)] {
            if r.anchor_min != r.anchor_max {
                return Err(format!(
                    "RectTransform {name} has stretched anchors, which are not read here"
                ));
            }
        }
        let reference = (child.anchor_min - parent.pivot) * parent.size_delta;
        let position = (reference + child.anchored).extend(child.position_z);
        out.push((position, child.rotation, child.scale));
        parent_path = path;
    }
    Ok(out)
}

/// Update: resolve each visible host once and install its systems (the Play
/// of its play-on-awake systems); clear them when it is hidden, and clear the
/// draws of a despawned host.
#[allow(clippy::too_many_arguments)]
fn install(
    mut commands: Commands,
    mut hosts: Query<(
        Entity,
        &UiParticleHost,
        &InheritedVisibility,
        Option<&mut HostState>,
    )>,
    live: Query<(), With<UiParticleHost>>,
    systems: Query<(Entity, &UiParticleSystem)>,
    json: Res<Assets<JsonAsset>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<UiUberMaterial>>,
    mut seeds: ResMut<crate::particle_runtime::seed::SystemSeedManager>,
    mut state: Local<(u64, HashMap<AssetId<JsonAsset>, Arc<Value>>)>,
) {
    let (ordinal, documents) = &mut *state;
    for (entity, system) in &systems {
        if !live.contains(system.host) {
            commands.entity(entity).try_despawn();
        }
    }
    for (entity, host, visible, host_state) in &mut hosts {
        match (visible.get(), host_state) {
            (false, Some(host_state)) => {
                if let HostState::Installed(installed) = &*host_state {
                    for &draw in &installed.draws {
                        commands.entity(draw).try_despawn();
                    }
                }
                commands.entity(entity).remove::<HostState>();
            }
            (true, None) => {
                let id = host.document.id();
                let doc = match documents.get(&id) {
                    Some(doc) => doc.clone(),
                    None => {
                        let Some(asset) = json.get(id) else { continue };
                        match serde_json::from_str::<Value>(&asset.0) {
                            Ok(doc) => documents.entry(id).or_insert(Arc::new(doc)).clone(),
                            Err(err) => {
                                error!(node = %host.node, "[ui-particle] the document is not JSON: {err}");
                                commands.entity(entity).insert(HostState::Refused);
                                continue;
                            }
                        }
                    }
                };
                match install_host(
                    &mut commands,
                    entity,
                    host,
                    &doc,
                    &server,
                    &mut meshes,
                    &mut materials,
                    &mut seeds,
                    ordinal,
                ) {
                    Ok(installed) => {
                        commands
                            .entity(entity)
                            .insert(HostState::Installed(installed));
                    }
                    Err(reason) => {
                        warn!(root = %host.root, node = %host.node, "[ui-particle] refused: {reason}");
                        commands.entity(entity).insert(HostState::Refused);
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn install_host(
    commands: &mut Commands,
    entity: Entity,
    host: &UiParticleHost,
    doc: &Value,
    server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<UiUberMaterial>,
    seeds: &mut crate::particle_runtime::seed::SystemSeedManager,
    ordinal: &mut u64,
) -> Result<Installed, String> {
    let (scene, rects) = document_root(doc, &host.root)?;
    let rows = doc["particles"]
        .as_array()
        .ok_or("document has no particles")?;
    let row_of = |reference: &Value| {
        let path_id = reference["pathId"].as_i64()?;
        (reference["fileId"].as_i64() == Some(0))
            .then(|| {
                rows.iter()
                    .find(|row| row["pathId"].as_i64() == Some(path_id))
            })
            .flatten()
    };
    // A UIParticle is in this root when the particle rows it lists are of
    // this root's scene.
    let matching: Vec<&Value> = doc["components"]["UIParticle"]["instances"]
        .as_array()
        .ok_or("document has no UIParticle instances")?
        .iter()
        .filter(|inst| inst["node"].as_str() == Some(host.node.as_str()))
        .filter(|inst| {
            inst["fields"]["m_Particles"]
                .as_array()
                .is_some_and(|list| {
                    !list.is_empty()
                        && list.iter().all(|reference| {
                            row_of(reference)
                                .is_some_and(|row| row["scene"].as_u64() == Some(scene))
                        })
                })
        })
        .collect();
    let [instance] = matching.as_slice() else {
        return Err(format!(
            "{} UIParticle components on {}/{} (placed by their particle rows' scene); one is placed",
            matching.len(),
            host.root,
            host.node
        ));
    };
    let f = &instance["fields"];
    if f["m_Enabled"].as_i64() != Some(1) {
        return Err(
            "the UIParticle component is disabled (its systems then draw as plain renderers)"
                .into(),
        );
    }
    if f["m_IsTrail"].as_i64() != Some(0) {
        return Err("m_IsTrail: the component disables itself".into());
    }
    if f["m_AnimatableProperties"]
        .as_array()
        .is_none_or(|a| !a.is_empty())
    {
        return Err("animatable material properties are not ported".into());
    }
    // A Mask whose node path this root holds, on the UIParticle node or above
    // it. Masks carry no scene link, so a same-path Mask of another root
    // counts too: this can only refuse more.
    let under_mask = doc["components"]["Mask"]["instances"]
        .as_array()
        .is_some_and(|masks| {
            masks.iter().any(|inst| {
                let mask = inst["node"].as_str().unwrap_or("");
                rects.contains_key(mask)
                    && (mask.is_empty()
                        || host.node == mask
                        || host.node.starts_with(&format!("{mask}/")))
            })
        });
    if under_mask && f["m_Maskable"].as_i64() != Some(0) {
        return Err("a Mask above the UIParticle: the stencil material is not ported".into());
    }
    let fields = Fields {
        ignore_canvas_scaler: f["m_IgnoreCanvasScaler"]
            .as_i64()
            .ok_or("m_IgnoreCanvasScaler")?
            == 1,
        scale3d: vec3(&f["m_Scale3D"]).ok_or("m_Scale3D")?,
        local_scale: rect(
            rects
                .get(host.node.as_str())
                .ok_or("no RectTransform for the UIParticle node")?,
            &host.node,
        )?
        .scale,
    };
    let has_rigidbody = doc["inventory"]["types"].get("Rigidbody").is_some();
    let references = f["m_Particles"].as_array().ok_or("m_Particles")?;
    let mut draws = Vec::new();
    let mut report = Vec::new();
    for (order, reference) in references.iter().enumerate() {
        let path_id = reference["pathId"]
            .as_i64()
            .ok_or("m_Particles entry without a pathId")?;
        if reference["fileId"].as_i64() != Some(0) {
            return Err(format!("m_Particles[{order}] is outside the prefab"));
        }
        let row = rows
            .iter()
            .find(|row| row["pathId"].as_i64() == Some(path_id))
            .ok_or_else(|| format!("m_Particles[{order}] ({path_id}) is not a particle row"))?;
        let node = row["node"].as_str().unwrap_or("");
        if !row["renderer"]["material"].is_object() {
            // No material and no trail material: the bake skips the system.
            report.push(format!("{order}:{node} (no material)"));
            continue;
        }
        let chain = rect_chain(&rects, &host.node, node)?;
        if chain.is_empty() && row["system"]["scalingMode"].as_u64() == Some(1) {
            return Err(format!(
                "m_Particles[{order}] {node}: a system on the UIParticle node under the Local scaling mode reads the node's driven scale, which is not wired"
            ));
        }
        let local_scale = chain.last().map_or(fields.local_scale, |(_, _, s)| *s);
        let mesh = meshes.add(crate::billboard::empty_mesh());
        let admission::Admitted {
            mut runtime,
            route,
            draw,
            space,
            max_particle_size,
        } = admission::admit(
            row,
            local_scale,
            has_rigidbody,
            mesh.clone(),
            entity,
            *ordinal,
        )
        .map_err(|reason| format!("m_Particles[{order}] {node}: {reason}"))?;
        *ordinal += 1;
        let played = crate::weather_fx::fixture::install_played(&mut runtime, &route, seeds)
            .map_err(|reason| format!("m_Particles[{order}] {node}: {reason}"))?;
        let local = match runtime.geometry.shape_evidence().map(|e| e.scaling) {
            Some(crate::particle_geometry::Scaling::Local { scale, .. }) => Some(scale),
            _ => None,
        };
        let texture = moly_assets::residency::load_image(
            server,
            format!("moly://{}/{}", host.directory, draw.texture),
        );
        let material = materials.add(UiUberMaterial {
            value: UiUberUniform {
                main_tex_st: Vec4::from_array(draw.main_tex_st),
                blend_mode: UVec4::new(draw.blend_mode, 0, 0, 0),
            },
            texture,
            key: UiUberKey { state: draw.state },
        });
        report.push(format!("{order}:{node} ({})", draw.texture));
        let entity_draw = commands
            .spawn((
                Mesh2d(mesh),
                MeshMaterial2d(material),
                Transform::default(),
                Visibility::Inherited,
                NoFrustumCulling,
                RenderLayers::layer(host.layer),
                UiParticleSystem {
                    host: entity,
                    order,
                    chain,
                    space,
                    local_scale: local,
                    max_particle_size,
                    identity: draw.identity,
                    runtime,
                    played,
                    retired: false,
                },
            ))
            .id();
        commands.entity(host.canvas).add_child(entity_draw);
        draws.push(entity_draw);
    }
    info!(
        root = %host.root,
        node = %host.node,
        "[ui-particle] installed {} of {} systems: {:?}",
        draws.len(),
        references.len(),
        report
    );
    Ok(Installed {
        fields,
        draws,
        driven_scale: fields.local_scale,
        cached_position: Vec3::ZERO,
        drawn_before: false,
        head_started: false,
    })
}

/// The UIParticle node's frame: the root canvas, then the host's ancestors
/// below the canvas entity and the host, each at its local x and y (z is the
/// draw order, not a source position), rotation and scale; the host node
/// takes `own_scale`.
fn uiparticle_node(
    canvas_root: &Node,
    chain: &[Transform],
    host: &Transform,
    own_scale: Vec3,
) -> Node {
    let mut node = *canvas_root;
    for t in chain {
        node = node.child(t.translation.truncate().extend(0.0), t.rotation, t.scale);
    }
    node.child(
        host.translation.truncate().extend(0.0),
        host.rotation,
        own_scale,
    )
}

/// PostUpdate: one frame of every installed UIParticle, as
/// `UIParticleUpdater.Refresh` runs it after the frame's particle update:
/// each system's frame on the shared runtime, then the bake into canvas space.
#[allow(clippy::too_many_arguments)]
fn bake_frame(
    mut hosts: Query<(
        Entity,
        &UiParticleHost,
        &mut HostState,
        Option<&UiParticleHeadStart>,
        Has<UiParticlePaused>,
    )>,
    mut draws: Query<(&mut UiParticleSystem, &mut Transform, &Mesh2d)>,
    locals: Query<(&Transform, Option<&ChildOf>), Without<UiParticleSystem>>,
    globals: Query<&GlobalTransform>,
    mut meshes: ResMut<Assets<Mesh>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    unscaled: Option<Res<crate::particle_runtime::UnscaledFrameClock>>,
) {
    if hosts.is_empty() {
        return;
    }
    let (Some(root_canvas), Ok(window)) = (root_canvas, windows.single()) else {
        return;
    };
    let pixels = [
        window.physical_width() as f32,
        window.physical_height() as f32,
    ];
    if pixels[0] <= 0.0 || pixels[1] <= 0.0 {
        return;
    }
    let canvas = root_canvas.for_pixels(pixels);
    let Some((camera, root_pose)) = root_canvas.event_camera(pixels) else {
        warn_once!("[ui-particle] the host canvas carries no root canvas camera pose: the baking camera's rotation is unknown, nothing is baked");
        return;
    };
    let q = |r: [f32; 4]| Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    let canvas_scale = Vec3::from_array(root_pose.scale);
    let canvas_root = Node::root(
        Vec3::from_array(root_pose.position),
        q(root_pose.rotation),
        canvas_scale,
    );
    // The root canvas renders with a camera (the host canvas reader admits
    // only that), so the baking camera turns with the camera.
    let baking = bake::baking_camera(
        canvas.size,
        canvas.scale_factor,
        Some(q(camera.world_rotation)),
    );
    let camera_transform = GlobalTransform::from(bake::runtime_camera(&baking));
    // The baking camera is orthographic: the geometry's upper size limit is
    // disabled (see admission) and its lower limit is zero, so the field of
    // view and aspect of this basis are not read.
    let basis = crate::billboard::basis_from_matrix(
        camera_transform.affine().matrix3.into(),
        camera_transform.translation(),
        std::f32::consts::FRAC_PI_3,
        pixels[0] / pixels[1],
        0.3,
    );
    let clocks = crate::weather_fx::fixture::FrameClocks {
        scaled: crate::particle_runtime::source_delta_time(time.delta()),
        unscaled: unscaled.as_deref().map(|clock| clock.delta()),
        now: time.elapsed_secs_f64(),
    };
    for (host_entity, host, mut state, head_start, paused) in &mut hosts {
        let HostState::Installed(installed) = &mut *state else {
            continue;
        };
        let head_steps = match head_start {
            Some(head) if !installed.head_started && head.frame > 0.0 => {
                (head.seconds / head.frame).round().max(0.0) as u32
            }
            _ => 0,
        };
        let (Ok(canvas_global), Ok(host_global)) =
            (globals.get(host.canvas), globals.get(host_entity))
        else {
            continue;
        };
        let Ok((host_local, parent)) = locals.get(host_entity) else {
            continue;
        };
        if host_local.scale != Vec3::ONE {
            warn_once!(node = %host.node, "[ui-particle] the host's scale is not one (the source drives the node's own scale): nothing is baked");
            continue;
        }
        let mut chain = Vec::new();
        let mut current = parent.map(ChildOf::parent);
        let mut reached = false;
        while let Some(entity) = current {
            if entity == host.canvas {
                reached = true;
                break;
            }
            let Ok((local, parent)) = locals.get(entity) else {
                break;
            };
            chain.push(*local);
            current = parent.map(ChildOf::parent);
        }
        if !reached {
            warn_once!(node = %host.node, "[ui-particle] the host is not below its canvas entity: nothing is baked");
            continue;
        }
        chain.reverse();
        let own = if installed.fields.ignore_canvas_scaler {
            installed.driven_scale = bake::driven_scale(installed.driven_scale, canvas_scale);
            installed.driven_scale
        } else {
            installed.fields.local_scale
        };
        let root = uiparticle_node(&canvas_root, &chain, host_local, own);
        let host_z = canvas_global
            .affine()
            .inverse()
            .transform_point3(host_global.translation())
            .z;
        let scale = bake::bake_scale(
            installed.fields.ignore_canvas_scaler,
            canvas_scale,
            installed.fields.scale3d,
        );
        let position = root.position();
        let displacement = bake::world_displacement(
            position,
            installed.cached_position,
            scale,
            installed.drawn_before,
        );
        installed.cached_position = position;
        let bake_alpha = !bake::approximately(host.alpha, 0.0);
        let mut groups: Vec<String> = Vec::new();
        for &draw in &installed.draws {
            let Ok((mut system, mut transform, mesh)) = draws.get_mut(draw) else {
                continue;
            };
            transform.translation =
                Vec3::new(0.0, 0.0, host_z + (system.order + 1) as f32 * DRAW_STEP);
            let node = system
                .chain
                .iter()
                .fold(root, |node, (t, r, s)| node.child(*t, *r, *s));
            if !system.retired {
                let anchor = GlobalTransform::from(bake::to_runtime(node.matrix));
                let ctx = crate::particle_runtime::Context {
                    site: anchor,
                    sky: GlobalTransform::IDENTITY,
                    camera: camera_transform,
                };
                let UiParticleSystem {
                    runtime,
                    played,
                    retired,
                    ..
                } = &mut *system;
                let mut stepped = Ok(());
                if let Some(head) = head_start.filter(|_| head_steps > 0) {
                    // The frames the systems ran before this host: each at
                    // `head.frame`, ending one frame before this one.
                    for step in 0..head_steps {
                        let before = f64::from(head_steps - step) * f64::from(head.frame);
                        let frame = crate::weather_fx::fixture::FrameClocks {
                            scaled: head.frame,
                            unscaled: Some(head.frame),
                            now: clocks.now - before,
                        };
                        stepped =
                            crate::weather_fx::fixture::step_played(runtime, played, &frame, &ctx);
                        if stepped.is_err() {
                            break;
                        }
                    }
                }
                if stepped.is_ok() && !paused {
                    stepped =
                        crate::weather_fx::fixture::step_played(runtime, played, &clocks, &ctx);
                }
                if let Err(reason) = stepped {
                    error!(%reason, node = %runtime.node, "[ui-particle] step refused: the system is retired");
                    runtime.pool.clear();
                    runtime.side.clear();
                    *retired = true;
                }
            }
            if system.space == Space::World && displacement != Vec3::ZERO {
                let shift = bake::reflect(displacement).to_array();
                for particle in &mut system.runtime.pool {
                    for (p, d) in particle.position.iter_mut().zip(shift) {
                        *p += d;
                    }
                }
            }
            let Some(mesh) = meshes.get_mut(&mesh.0) else {
                continue;
            };
            let render_scale = system.local_scale.unwrap_or_else(|| node.lossy_scale());
            let owner = GlobalTransform::from(bake::to_runtime(bake::local_render_matrix(
                &node,
                render_scale,
            )));
            let to_world = match system.space {
                Space::Local => owner,
                Space::World => GlobalTransform::IDENTITY,
            };
            // Nothing to bake: no particle, or the inherited alpha is
            // approximately zero.
            if system.runtime.pool.is_empty() || !bake_alpha {
                if mesh.count_vertices() != 0 {
                    *mesh = crate::billboard::empty_mesh();
                }
                continue;
            }
            // The orthographic camera's size limit is not wired; a particle
            // within half of the smallest reading of it (the viewport height
            // or width fraction at the orthographic size, over the far
            // distance) is not drawn, loudly.
            let bound = baking.orthographic_size
                * 2.0
                * system.max_particle_size
                * (pixels[0] / pixels[1]).min(1.0)
                * (bake::ORTHO_POSITION.z.abs() / baking.far).min(1.0);
            let largest = render_scale.abs().max_element();
            let near_limit =
                crate::particle_runtime::geometry_instances(&system.runtime, &to_world)
                    .iter()
                    .any(|p| p.size.x.max(p.size.y) * largest >= 0.5 * bound);
            if near_limit {
                warn_once!(node = %system.runtime.node, "[ui-particle] a particle near the baking camera's size limit, whose clamp is not wired: not drawn");
                if mesh.count_vertices() != 0 {
                    *mesh = crate::billboard::empty_mesh();
                }
                continue;
            }
            crate::particle_runtime::write_geometry(
                mesh,
                &system.runtime,
                &to_world,
                &owner,
                &camera_transform,
                basis,
            );
            if mesh.count_vertices() == 0 {
                continue;
            }
            if groups.last() != Some(&system.identity) {
                groups.push(system.identity.clone());
            }
            if groups.len() > MAX_MATERIALS {
                // Past the eighth material the CanvasRenderer has no material
                // for the submesh.
                *mesh = crate::billboard::empty_mesh();
                continue;
            }
            let system_node = (!system.chain.is_empty()).then_some(&node);
            let matrix = bake::to_canvas(&canvas_root, &root, system_node, system.space, scale);
            if let Some(VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for p in positions.iter_mut() {
                    let v = matrix.transform_point3(bake::reflect(Vec3::from_array(*p)));
                    *p = [v.x, v.y, 0.0];
                }
            }
            if let Some(VertexAttributeValues::Float32x4(colours)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
            {
                for c in colours.iter_mut() {
                    c[3] *= host.alpha;
                }
            }
        }
        installed.drawn_before = !groups.is_empty();
        installed.head_started = true;
    }
}
