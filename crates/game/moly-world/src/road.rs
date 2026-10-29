//! Road runtime material and RoadCell neighborhood adapter.
//! Source: CN Road Base/instancing shader, RoadNativeArrayCache and RoadView.

use super::{EditableFixture, FixturePlacements, fence::FixtureRow, layout_type};
use crate::env::SiteEnvGpuBuffer;
use bevy::{
    ecs::system::{SystemParamItem, lifetimeless::SRes},
    gltf::{GltfExtras, GltfMaterialExtras},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, StandardMaterial},
    prelude::*,
    render::{
        render_asset::RenderAssets, render_resource::*, renderer::RenderDevice, texture::GpuImage,
    },
    shader::ShaderRef,
};
use moly_assets::material_textures::SourcePrimitiveIndex;
use moly_law::fixture::GridPosition;
use moly_law::fixture::road::{CELL_OFFSETS, NEIGHBOR_OFFSETS, quarter};
use std::collections::{HashMap, HashSet};

const SHADER: &str = "Mysekai/Fixture/Road";

#[derive(Asset, TypePath, Clone)]
pub struct RoadMaterial {
    /// connect, alpha tiling/offset, options, shadow globals, edge globals.
    uniforms: [[f32; 4]; 5],
    main_tex: Handle<Image>,
}

impl AsBindGroup for RoadMaterial {
    type Data = ();
    type Param = (SRes<SiteEnvGpuBuffer>, SRes<RenderAssets<GpuImage>>);
    fn label() -> &'static str {
        "road_material"
    }
    fn bind_group_data(&self) -> Self::Data {}
    fn build_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _device: &RenderDevice,
        (env, images): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
        output: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        let image = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let bytes: Vec<u8> = self
            .uniforms
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let uniforms = crate::render::gpu::push_data(output, &bytes);
        output.binding_resources.0.extend([
                (0, uniforms),
                (1, UnpreparedBindingResource::Buffer(env.buffer.clone())),
                (
                    2,
                    UnpreparedBindingResource::TextureView(
                        TextureViewDimension::D2,
                        image.texture_view.clone(),
                    ),
                ),
                (
                    3,
                    UnpreparedBindingResource::Sampler(
                        SamplerBindingType::Filtering,
                        image.sampler.clone(),
                    ),
                ),
        ]);
        Ok(())
    }
    fn bind_group_layout_entries(
        _device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![
            uniform(0),
            uniform(1),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ]
    }
}

impl Material for RoadMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_world/shaders/road_material.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
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
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        crate::material_order::set_queue(descriptor, 2006);
        descriptor.primitive.cull_mode = Some(Face::Back);
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::Always);
        }
        Ok(())
    }
}

/// The editor publishes its entire private road projection. Committed
/// placement records and gameplay occupancy remain untouched until save.
#[derive(Resource, Clone)]
pub struct RoadDraft {
    pub revision: u64,
    pub rows: Vec<EditableFixture>,
}

/// A source road view created solely for the editor's private projection.
#[derive(Component)]
pub struct RoadPreview(pub String);

#[derive(Component)]
struct RoadBound;

#[derive(Component)]
struct RoadSurface {
    root: Entity,
    row: Option<usize>,
    uid: Option<String>,
    cell: usize,
    quadrant: usize,
    texture: Handle<Image>,
    local_mapping: f32,
    lighting: f32,
}

fn bind_surfaces(
    mut commands: Commands,
    roots: Query<
        (
            Entity,
            &super::FixtureVisualRoot,
            Option<&FixtureRow>,
            Option<&RoadPreview>,
            Option<&crate::fixture_scene_inputs::FixtureScenePlacement>,
        ),
        (
            With<crate::render_inputs::FixtureVisualSceneReady>,
            Without<RoadBound>,
        ),
    >,
    children: Query<&Children>,
    nodes: Query<&GltfExtras>,
    parts: Query<(
        &MeshMaterial3d<StandardMaterial>,
        &GltfMaterialExtras,
        &SourcePrimitiveIndex,
    )>,
    standard: Res<Assets<StandardMaterial>>,
) {
    for (root, visual, row, preview, placement) in &roots {
        if visual.layout != layout_type::ROAD {
            continue;
        }
        let uid = preview
            .map(|value| value.0.clone())
            .or_else(|| placement.map(|value| value.0.uid.clone()));
        let mut stack = vec![(root, None)];
        while let Some((entity, mut cell)) = stack.pop() {
            if let Ok(extras) = nodes.get(entity) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&extras.value) {
                    if let Some(index) = value.get("roadCellType").and_then(|v| v.as_u64()) {
                        cell = Some(index as usize);
                    }
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter().map(|e| (e, cell)));
            }
            let Ok((material, extras, primitive)) = parts.get(entity) else {
                continue;
            };
            let value: serde_json::Value =
                serde_json::from_str(&extras.value).expect("Road material extras JSON");
            if value.get("shader").and_then(|v| v.as_str()) != Some(SHADER) {
                continue;
            }
            let cell = cell
                .filter(|&v| v < 4)
                .expect("Road material requires exported RoadCell reference");
            let get = |key: &str| {
                value
                    .get("floats")
                    .and_then(|v| v.get(key))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| panic!("Road material missing {key}")) as f32
            };
            for (key, expected) in [("_ZTest", 8.0), ("_ZWrite", 0.0), ("_Cull", 2.0)] {
                assert_eq!(get(key), expected, "Road source state {key}");
            }
            let texture = standard
                .get(&material.0)
                .and_then(|m| m.base_color_texture.clone())
                .expect("Road main texture");
            commands.entity(entity).insert(RoadSurface {
                root,
                row: row.map(|row| row.0),
                uid: uid.clone(),
                cell,
                quadrant: primitive.0,
                texture,
                local_mapping: get("_MainTextureLocalMapping"),
                lighting: get("_UsePhenomenaLighting"),
            });
        }
        commands.entity(root).insert(RoadBound);
    }
}

type MaterialCache = HashMap<(AssetId<Image>, Vec<u32>), Handle<RoadMaterial>>;

fn update_materials(
    mut commands: Commands,
    placements: Res<FixturePlacements>,
    surfaces: Query<(Entity, Ref<RoadSurface>)>,
    active_roots: Query<(), With<super::FixtureVisualRoot>>,
    draft: Option<Res<RoadDraft>>,
    mut had_draft: Local<bool>,
    mut materials: ResMut<Assets<RoadMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<MaterialCache>,
    revision: Res<super::FixtureLayoutRevision>,
    mut replaced: MessageWriter<crate::gpu_image_release::ImageTextureReplaced>,
) {
    if revision.is_changed() {
        cache.clear();
    }
    let draft_changed =
        draft.as_ref().is_some_and(|draft| draft.is_changed()) || *had_draft != draft.is_some();
    *had_draft = draft.is_some();
    if surfaces.is_empty()
        || (!placements.is_changed()
            && !draft_changed
            && !surfaces.iter().any(|(_, s)| s.is_added()))
    {
        return;
    }
    let rows: Vec<(String, GridPosition, GridPosition)> = if let Some(draft) = &draft {
        draft
            .rows
            .iter()
            .filter(|row| row.layout == layout_type::ROAD)
            .filter_map(|row| {
                row.footprint()
                    .ok()
                    .map(|(min, max)| (row.package.clone(), min, max))
            })
            .collect()
    } else {
        placements
            .rows
            .iter()
            .filter(|row| row.layout == layout_type::ROAD)
            .map(|row| {
                let bounds = row.placed();
                (row.package.clone(), bounds.min, bounds.max)
            })
            .collect()
    };
    let mut next = MaterialCache::new();
    let mut prepared = HashSet::new();
    let mut clipped = 0;
    for (entity, surface) in &surfaces {
        if !active_roots.contains(surface.root) {
            continue;
        }
        let current = match &draft {
            Some(draft) => surface
                .uid
                .as_ref()
                .and_then(|uid| draft.rows.iter().find(|row| &row.uid == uid))
                .and_then(|row| {
                    row.footprint()
                        .ok()
                        .map(|(min, _)| (row.package.as_str(), min))
                }),
            None => surface
                .row
                .and_then(|index| placements.rows.get(index))
                .map(|row| (row.package.as_str(), row.placed().min)),
        };
        let Some((package, min)) = current else {
            continue;
        };
        let (dx, dz) = CELL_OFFSETS[surface.cell];
        let x = min.x.wrapping_add(dx);
        let z = min.z.wrapping_add(dz);
        // TileNeighbourData.CanConnect accepts any joint occupant with the same
        // fixture ID. Road package identity is one-to-one with that source ID.
        let neighbors = NEIGHBOR_OFFSETS.map(|(dx, dz)| {
            let nx = x.wrapping_add(dx);
            let nz = z.wrapping_add(dz);
            rows.iter().any(|(other, min, max)| {
                other == package && nx >= min.x && nx <= max.x && nz >= min.z && nz <= max.z
            })
        });
        let q = quarter(neighbors, surface.quadrant);
        clipped += usize::from(q.alpha_clip);
        // CN GraphicsConfig.<Road>: immutable source rendering parameters.
        // roadCornerScale is not referenced by this compiled Base program.
        let uniforms = [
            q.connections,
            q.alpha_tiling_offset,
            [
                surface.local_mapping,
                surface.lighting,
                u8::from(q.alpha_clip) as f32,
                2.0,
            ],
            [2.13, 1.22, 6.12, 14.12],
            [0.471, 0.105, 0.0, 0.0],
        ];
        if prepared.insert(surface.texture.id()) {
            // Decide through a shared borrow first: `get_mut` always marks the
            // image Modified, which re-extracts and re-uploads it even when
            // its chain already exists. The pixels stay in the main world: a
            // furniture material could still need this image's emission mask
            // mean, and only the furniture material swap records those.
            use crate::fixture_material::{MipSkip, generate_mip_chain, mip_chain_plan};
            let chained = match images.get(&surface.texture).map(mip_chain_plan) {
                Some(Ok(_)) => images
                    .get_mut(&surface.texture)
                    .map_or(Err(MipSkip::NoData), |image| {
                        generate_mip_chain(image.into_inner())
                    }),
                Some(Err(reason)) => Err(reason),
                None => Ok(0),
            };
            match chained {
                // The chained upload replaces the texture the image already
                // has on the GPU (see crate::gpu_image_release).
                Ok(levels) if levels > 0 => {
                    replaced.write(crate::gpu_image_release::ImageTextureReplaced(
                        surface.texture.id(),
                    ));
                }
                Ok(_) | Err(MipSkip::AlreadyChained) => {}
                Err(reason) => warn!("Road mip generation: {reason:?}"),
            }
        }
        let key = (
            surface.texture.id(),
            uniforms.iter().flatten().map(|v| v.to_bits()).collect(),
        );
        let handle = next
            .entry(key.clone())
            .or_insert_with(|| {
                cache.remove(&key).unwrap_or_else(|| {
                    materials.add(RoadMaterial {
                        uniforms,
                        main_tex: surface.texture.clone(),
                    })
                })
            })
            .clone();
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
    info!(
        "Road material: {} quadrant meshes, {} shared material states, {} alpha-clipped; source queue 2006",
        surfaces.iter().count(),
        next.len(),
        clipped
    );
    *cache = next;
}

pub(super) fn install(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/road_material.wesl");
    crate::gpu_image_release::install(app);
    crate::gpu_image_release::prepare_after_images::<RoadMaterial>(app);
    app.add_plugins(MaterialPlugin::<RoadMaterial>::default())
        .add_systems(
            Update,
            (bind_surfaces, update_materials)
                .chain()
                .after(super::FixtureLayoutSet)
                .after(crate::fixture_colors::prepare)
                .run_if(crate::fixture_colors::ready)
                .before(crate::fixture_material::FixtureMaterialSet),
        );
}
