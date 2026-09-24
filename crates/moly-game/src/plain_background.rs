//! Room plain background: the environment view's `PlainBackground` quad.
//!
//! Source: `SiteEnvironmentViewController.Setup(camera)` calls
//! `PlainBackground.Setup(camera)` and deactivates the `PlainBG` object; at
//! each cross-fade start `ShowPlainBackground(afterSite.SiteType - 1 < 3)`
//! activates it for the three room floors and deactivates it everywhere else,
//! and it stays so until the next cross-fade. `PlainBackground.SetupVertices`
//! overwrites the four vertices of its mesh (the engine's built-in Quad) with
//! (-1,-1,0), (1,-1,0), (-1,1,0), (1,1,0), keeping the Quad's uv (0,0), (1,0),
//! (0,1), (1,1) and its triangles 0,3,1 / 3,0,2. Before every render of its
//! camera `OnBeginCameraRendering` writes `_PositionZ = _plainBackgroundDistance`
//! (serialized 30) into the material. The material is `plain_background`
//! (`Mysekai/Indoor/BG`, queue 2070, cull off, depth test less-equal with
//! depth write, no blending); its program puts the vertices straight into clip
//! space at that eye distance and paints a vertical two-colour gradient
//! (`shaders/plain_background.wgsl`). `ChangePlainBgTexture` sets a main
//! texture that this program never samples.
//!
//! Open: the program writes the depth-buffer value of that eye distance
//! (from `_ZBufferParams`) as clip z with w = 1. That lands at the distance
//! only where clip z reaches the depth buffer unchanged, the reversed-Z
//! reading (Vulkan); under OpenGL ES the same value maps to another depth.
//! The source's graphics API is not decided, and the product takes the
//! reversed-Z (Vulkan) reading.
//!
//! Inputs: the shell package whose inventory carries `PlainBackground` (the
//! same package the sky reads), its `components.PlainBackground` distance and
//! the one material drawn with `Mysekai/Indoor/BG`. A missing input is named
//! once and nothing is drawn.
use bevy::asset::{AssetPath, LoadState};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;

use crate::site::SiteActive;

const SHADER_NAME: &str = "Mysekai/Indoor/BG";

#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
#[bind_group_data(PlainBackgroundKey)]
pub(crate) struct PlainBackgroundMaterial {
    /// `_Color1` as stored (top of the gradient, uv.y = 1).
    #[uniform(0)]
    color1: Vec4,
    /// `_Color2` as stored (bottom, uv.y = 0).
    #[uniform(1)]
    color2: Vec4,
    /// (`_PositionZ`, 0, 0, 0).
    #[uniform(2)]
    params: Vec4,
    queue: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PlainBackgroundKey(u32);

impl From<&PlainBackgroundMaterial> for PlainBackgroundKey {
    fn from(value: &PlainBackgroundMaterial) -> Self {
        Self(value.queue)
    }
}

impl Material for PlainBackgroundMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/plain_background.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/plain_background.wgsl".into()
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
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The pass state is Cull Off, ZTest LEqual, ZWrite On, Blend One Zero;
        // the opaque default already tests and writes depth in this renderer's
        // reversed convention, so only the culling changes.
        descriptor.primitive.cull_mode = None;
        crate::material_order::set_queue(descriptor, key.bind_group_data.0);
        Ok(())
    }
}

/// The quad marker; its visibility follows the source's show rule.
#[derive(Component)]
pub(crate) struct PlainBackground;

#[derive(Resource)]
struct Listings(Handle<JsonAsset>);

#[derive(Resource)]
struct Sidecar(Handle<JsonAsset>);

#[derive(Resource)]
struct Spawned;

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Listings(
        server.load::<JsonAsset>(AssetPath::from("moly://site/packages.json".to_owned())),
    ));
}

fn parse_packages(
    mut commands: Commands,
    server: Res<AssetServer>,
    docs: Res<Assets<JsonAsset>>,
    listings: Option<Res<Listings>>,
) {
    let Some(listings) = listings else {
        return;
    };
    if let LoadState::Failed(error) = server.load_state(&listings.0) {
        panic!("site package listing failed to load (plain background): {error:?}");
    }
    let Some(doc) = docs.get(&listings.0) else {
        return;
    };
    commands.remove_resource::<Listings>();
    let value: serde_json::Value = match serde_json::from_str(&doc.0) {
        Ok(value) => value,
        Err(error) => {
            error!("[plain-background] site package listing is not JSON: {error}; not drawn");
            return;
        }
    };
    // The same selection as the sky: the first shell package (by key order)
    // that ships the environment view's scripts and its geometry.
    let hit = value
        .get("packages")
        .and_then(|v| v.as_object())
        .and_then(|packages| {
            packages.values().find(|package| {
                package.get("kind").and_then(|v| v.as_str()) == Some("shell")
                    && package
                        .pointer("/inventory/scripts/PlainBackground")
                        .is_some()
                    && package
                        .pointer("/artifacts/geometry")
                        .and_then(|v| v.as_str())
                        .is_some()
            })
        });
    let Some(package) = hit else {
        error!("[plain-background] no shell package carries PlainBackground; the room background is not drawn");
        return;
    };
    let (Some(directory), Some(geometry)) = (
        package.get("directory").and_then(|v| v.as_str()),
        package
            .pointer("/artifacts/geometry")
            .and_then(|v| v.as_str()),
    ) else {
        error!("[plain-background] shell package lacks directory or geometry; not drawn");
        return;
    };
    let stem = geometry.rsplit_once('.').map_or(geometry, |(stem, _)| stem);
    commands.insert_resource(Sidecar(server.load::<JsonAsset>(AssetPath::from(format!(
        "moly://site/{directory}/{stem}.json"
    )))));
}

fn read_color(record: &serde_json::Value, key: &str) -> Option<Vec4> {
    let items = record.pointer(&format!("/colors/{key}"))?.as_array()?;
    if items.len() != 4 {
        return None;
    }
    let mut out = [0.0f32; 4];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64().filter(|v| v.is_finite())? as f32;
    }
    Some(Vec4::from_array(out))
}

fn spawn(
    mut commands: Commands,
    server: Res<AssetServer>,
    docs: Res<Assets<JsonAsset>>,
    sidecar: Option<Res<Sidecar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PlainBackgroundMaterial>>,
) {
    let Some(sidecar) = sidecar else {
        return;
    };
    if let LoadState::Failed(error) = server.load_state(&sidecar.0) {
        panic!("shell sidecar failed to load (plain background): {error:?}");
    }
    let Some(doc) = docs.get(&sidecar.0) else {
        return;
    };
    commands.remove_resource::<Sidecar>();
    let value: serde_json::Value = match serde_json::from_str(&doc.0) {
        Ok(value) => value,
        Err(error) => {
            error!("[plain-background] shell sidecar is not JSON: {error}; not drawn");
            return;
        }
    };
    let instances = value
        .pointer("/components/PlainBackground/instances")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let [instance] = instances else {
        error!(
            "[plain-background] expected one PlainBackground component, found {}; not drawn",
            instances.len()
        );
        return;
    };
    let Some(distance) = instance
        .pointer("/fields/_plainBackgroundDistance")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
    else {
        error!("[plain-background] PlainBackground lacks a positive _plainBackgroundDistance; not drawn");
        return;
    };
    let records: Vec<_> = value
        .get("materials")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.pointer("/shader/name").and_then(|v| v.as_str()) == Some(SHADER_NAME))
        .collect();
    let [record] = records.as_slice() else {
        error!(
            "[plain-background] expected one {SHADER_NAME} material, found {}; not drawn",
            records.len()
        );
        return;
    };
    let (Some(color1), Some(color2), Some(queue)) = (
        read_color(record, "_Color1"),
        read_color(record, "_Color2"),
        record
            .get("renderQueue")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok()),
    ) else {
        error!("[plain-background] {SHADER_NAME} material lacks _Color1, _Color2 or its render queue; not drawn");
        return;
    };
    // The built-in Quad after SetupVertices (vertex order, uv and triangles
    // of the engine mesh; only the positions are replaced).
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-1.0f32, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [-1.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ],
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 3, 1, 3, 0, 2]));
    let material = materials.add(PlainBackgroundMaterial {
        color1,
        color2,
        params: Vec4::new(distance as f32, 0.0, 0.0, 0.0),
        queue,
    });
    commands.spawn((
        PlainBackground,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
        NoFrustumCulling,
        crate::shadowmap::NoShadowCast,
    ));
    commands.insert_resource(Spawned);
    info!(
        "[plain-background] ready: distance {distance} m, colour1 {:?}, colour2 {:?}, queue {queue}",
        color1.to_array(),
        color2.to_array()
    );
}

/// Shown exactly while the active site is a room floor.
fn show(site: Option<Res<SiteActive>>, mut quads: Query<&mut Visibility, With<PlainBackground>>) {
    let target = if site.as_deref().is_some_and(SiteActive::is_indoor) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut quads {
        if *visibility != target {
            *visibility = target;
        }
    }
}

pub(crate) fn install(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/plain_background.wgsl");
    crate::gpu_image_release::prepare_after_images::<PlainBackgroundMaterial>(app);
    app.add_plugins(MaterialPlugin::<PlainBackgroundMaterial>::default())
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (
                parse_packages,
                spawn.run_if(not(resource_exists::<Spawned>)),
                show,
            )
                .chain(),
        );
}
