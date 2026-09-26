//! Room shell material: the module walls, floor and entrance of a room floor
//! drawn with the source programs (`shaders/room_shell.wgsl`).
//!
//! Source: the room module prefabs (house lv_01..lv_05) carry four materials:
//! `mat_floor_main` (`Mysekai/Room/Floor`), `mat_wall_main` (`Mysekai/Object`,
//! usage 2, `_DISABLE_DITHER` + `_RECEIVE_SHADOWS_OFF`), `mat_wall_ent` and
//! `mat_floor_ent` (`Mysekai/Object`, usage 10, `_DISABLE_DITHER`). The skin
//! only replaces textures: `WallView.ChangeWallAppearance` and
//! `FloorView.ChangeFloorAppearance` put the skin texture into `_MainTex`
//! through a MaterialPropertyBlock together with the uv set its name selects
//! (`_BaseTextureMappingMode` 0/2/3 on the wall, `_UVSelection` 0/1/2 on the
//! floor, both "uv set n - 1"). Every other value is the module material's.
//! Neither program reads `_Color` or `_MainTex_ST`.
//!
//! Usage 2 multiplies the wall occlusion by the edge factor
//! `_WallAOIntensity * (d - 1) + 1`, whose `d` comes from the mesh's fourth uv
//! set, and `mat_wall_main` carries intensity 0.5, so the factor is live. A
//! skin texture named "uvset3" is drawn with the third set. The module glb
//! carries both as `TEXCOORD_2` / `TEXCOORD_3`; the engine's glTF loader maps
//! only the first two, so [`attach_source_uv_sets`] reads them from the glTF
//! source and puts them on the module meshes ([`ATTRIBUTE_UV_2`] /
//! [`ATTRIBUTE_UV_3`]), and the pipeline binds whichever of them a mesh has.
//! A wall mesh without the fourth set (a module file exported before the
//! uv-set export) gets a pipeline without it, which leaves the edge factor
//! out; `room_appearance.rs` names that once per room load.
//!
//! The globals are the site ones (`SiteEnvGpuBuffer`: phenomena light and
//! shade, drop-shadow colour 1, sky-bottom colour, fog, edge pair, treasure
//! shadows), the main-light shadow consumer block of `shadowmap.rs` and the
//! site-extension globals (`SiteExtensionGpuBuffer`, site_extension.rs).
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::gltf::{Gltf, GltfMesh};
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;

use crate::env::SiteEnvGpuBuffer;
use crate::render::gpu::SharedSamplers;
use crate::shadowmap::ShadowMapGpu;
use crate::site_extension::SiteExtensionGpuBuffer;

pub(crate) const OBJECT_SHADER: &str = "Mysekai/Object";
pub(crate) const FLOOR_SHADER: &str = "Mysekai/Room/Floor";

const PARAM_SLOTS: usize = 9;

/// The third and fourth uv sets of a module mesh (`TEXCOORD_2` /
/// `TEXCOORD_3` of the module glb, V flipped like the first two).
pub(crate) const ATTRIBUTE_UV_2: MeshVertexAttribute =
    MeshVertexAttribute::new("Room_Uv_2", 0x4d_4f_4c_59_52_02, VertexFormat::Float32x2);
pub(crate) const ATTRIBUTE_UV_3: MeshVertexAttribute =
    MeshVertexAttribute::new("Room_Uv_3", 0x4d_4f_4c_59_52_03, VertexFormat::Float32x2);

/// Shader locations of the two sets. The standard attributes keep the mesh
/// pipeline's locations 0..5; 6 and 7 are the (unused) joint slots.
const UV_2_LOCATION: u32 = 8;
const UV_3_LOCATION: u32 = 9;

/// `_UsePhenomenaLighting` as the room programs see it: `RoomController` runs
/// `MysekaiMaterialExtension.SetPhenomenaLighting(on: true)` over the shared
/// materials of all renderers under the room, and for any shader other than
/// `Mysekai/Effect/UberUnlit` that writes `on` (1) into this property.
const PHENOMENA_LIGHTING_ON: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShellProgram {
    Object,
    Floor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RoomShellKey {
    pub program: ShellProgram,
    /// The `_MAIN_LIGHT_SHADOWS` program without `_RECEIVE_SHADOWS_OFF`.
    pub receive_shadows: bool,
    pub queue: u32,
}

#[derive(Asset, TypePath, Debug, Clone)]
pub(crate) struct RoomShellMaterial {
    pub key: RoomShellKey,
    params: [[f32; 4]; PARAM_SLOTS],
    pub main_tex: Handle<Image>,
}

/// One module material resolved against the source program, before its
/// texture is loaded.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedShell {
    pub key: RoomShellKey,
    params: [[f32; 4]; PARAM_SLOTS],
    /// Usage 2 with a non-zero `_WallAOIntensity`: the edge factor reads the
    /// mesh's fourth uv set (a mesh without it is drawn without the factor).
    pub reads_uv3: bool,
}

impl ResolvedShell {
    pub(crate) fn with_texture(&self, main_tex: Handle<Image>) -> RoomShellMaterial {
        RoomShellMaterial {
            key: self.key,
            params: self.params,
            main_tex,
        }
    }

    /// The mesh uv set the main texture is sampled with.
    pub(crate) fn uv_index(&self) -> u32 {
        self.params[1][0] as u32
    }
}

fn float(record: &serde_json::Value, name: &str, key: &str) -> Result<f32, String> {
    record
        .pointer(&format!("/floats/{key}"))
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("room material {name} lacks float {key}"))
}

fn color(record: &serde_json::Value, name: &str, key: &str) -> Result<[f32; 4], String> {
    let items = record
        .pointer(&format!("/colors/{key}"))
        .and_then(|v| v.as_array())
        .filter(|items| items.len() == 4)
        .ok_or_else(|| format!("room material {name} lacks colour {key}"))?;
    let mut out = [0.0f32; 4];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("room material {name} colour {key} is not finite"))?
            as f32;
    }
    Ok(out)
}

fn domain(name: &str, key: &str, value: f32, allowed: &[f32]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "room material {name}: {key} = {value} is outside the ported values {allowed:?}"
        ))
    }
}

/// Resolve one module material record (the module sidecar's `materials[]`).
/// `uv_index` is the skin binding's uv set for the two main slots (the
/// property block), or `None` to keep the module material's own mapping.
pub(crate) fn resolve(
    record: &serde_json::Value,
    uv_index: Option<u32>,
) -> Result<ResolvedShell, String> {
    let name = record
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_owned();
    let shader = record
        .pointer("/shader/name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("room material {name} names no shader"))?;
    let keywords: Vec<&str> = record
        .get("keywords")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    let queue = record
        .get("renderQueue")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format!("room material {name} lacks its render queue"))?;
    let get = |key: &str| float(record, &name, key);
    let mut params = [[0.0f32; 4]; PARAM_SLOTS];
    match shader {
        OBJECT_SHADER => {
            // The room shell variants: dithering off on all, shadow reception
            // off on the main walls. Any other keyword is an unported variant.
            if !keywords.contains(&"_DISABLE_DITHER")
                || keywords
                    .iter()
                    .any(|k| !matches!(*k, "_DISABLE_DITHER" | "_RECEIVE_SHADOWS_OFF"))
            {
                return Err(format!(
                    "room material {name}: keywords {keywords:?} are not a ported Object variant"
                ));
            }
            let usage = get("_ObjectShaderUsage")?;
            domain(&name, "_ObjectShaderUsage", usage, &[2.0, 10.0, 14.0])?;
            let preview = get("_UseObject3DPreviewLight")?;
            domain(&name, "_UseObject3DPreviewLight", preview, &[0.0])?;
            // The second colour target is the emission mask times a gate
            // built from these four and the phenomenon's emission type; with
            // all four 0 the gate is 0 in every phenomenon, which is why the
            // pipeline draws no second target.
            for key in [
                "_EnableManualEmission",
                "_DebugEmission",
                "_BrightPhenomenaEmission",
                "_DarkPhenomenaEmission",
            ] {
                domain(&name, key, get(key)?, &[0.0])?;
            }
            // The pass state reads _Cull; the pipeline keeps back-face culling.
            let cull = get("_Cull")?;
            domain(&name, "_Cull", cull, &[2.0])?;
            let local_mapping = get("_MainTextureLocalMapping")?;
            domain(
                &name,
                "_MainTextureLocalMapping",
                local_mapping,
                &[0.0, 1.0],
            )?;
            let index = match uv_index {
                Some(index) => index as f32,
                None => {
                    // _BaseTextureMappingMode: 0 uv0, 2 uv1, 3 uv2 (1, world
                    // xz, is not ported here).
                    let mapping = get("_BaseTextureMappingMode")?;
                    domain(&name, "_BaseTextureMappingMode", mapping, &[0.0, 2.0, 3.0])?;
                    match mapping as u32 {
                        2 => 1.0,
                        3 => 2.0,
                        _ => 0.0,
                    }
                }
            };
            params[0] = [
                usage,
                get("_UseVertexColorBlend")?,
                get("_UseVertexAlphaOpacity")?,
                cull,
            ];
            params[1] = [index, local_mapping, get("_UVScrollX")?, get("_UVScrollY")?];
            params[2] = [
                get("_UseFresnel")?,
                get("_FresnelPower")?,
                // `RoomController` calls `MysekaiMaterialExtension.SetPhenomenaLighting(on: true)`
                // on every material of every renderer of the room, which writes 1 into
                // `_UsePhenomenaLighting`, overwriting the authored value.
                PHENOMENA_LIGHTING_ON,
                get("_BaseOpacity")?,
            ];
            params[3] = color(record, &name, "_FresnelColor")?;
            params[4] = color(record, &name, "_BackFaceColor")?;
            params[5] = [
                get("_OverrideShadingParameter")?,
                get("_LocalShadingIntensity")?,
                get("_LocalEdgeThreshold")?,
                get("_LocalEdgeSmoothness")?,
            ];
            params[6] = [
                get("_WallAOIntensity")?,
                get("_WallAOScaleX")?,
                get("_WallAOScaleY")?,
                get("_WallAOExponent")?,
            ];
            params[7] = color(record, &name, "_AdditiveColor")?;
            params[8] = [
                get("_UseHeightFade")?,
                get("_HeightFadePosition")?,
                get("_HeightFadeLength")?,
                get("_HeightFadeExponent")?,
            ];
            // The fog program differs from the fogless one only by clamping
            // the lit colour to [0, 1] before the additive colour and the
            // height fade; the output decode clamps anyway. So one program
            // stands for both states of the global fog keyword only while
            // neither of those two steps can move a clamped value.
            let additive = params[7];
            let additive_zero = additive[3] == 0.0 || additive[..3].iter().all(|c| *c == 0.0);
            if params[8][0] > 0.5 || !additive_zero {
                return Err(format!(
                    "room material {name}: _UseHeightFade {} / _AdditiveColor {:?} make the fog keyword state visible; only the keyword-independent shape is ported",
                    params[8][0], additive
                ));
            }
            Ok(ResolvedShell {
                key: RoomShellKey {
                    program: ShellProgram::Object,
                    receive_shadows: !keywords.contains(&"_RECEIVE_SHADOWS_OFF"),
                    queue,
                },
                params,
                // With _WallAOIntensity 0 the edge factor is exactly 1 and the
                // fourth set is multiplied away.
                reads_uv3: usage == 2.0 && params[6][0] != 0.0,
            })
        }
        FLOOR_SHADER => {
            if !keywords.is_empty() {
                return Err(format!("room material {name}: keywords {keywords:?} are not a ported Room/Floor variant"));
            }
            let index = match uv_index {
                Some(index) => index as f32,
                None => {
                    let selection = get("_UVSelection")?;
                    domain(&name, "_UVSelection", selection, &[0.0, 1.0, 2.0])?;
                    selection
                }
            };
            params[1] = [index, 0.0, 0.0, 0.0];
            params[2] = [
                get("_UseFresnel")?,
                get("_FresnelPower")?,
                // Forced on by `SetPhenomenaLighting(on: true)`, as on the Object materials.
                PHENOMENA_LIGHTING_ON,
                1.0,
            ];
            params[3] = color(record, &name, "_FresnelColor")?;
            Ok(ResolvedShell {
                key: RoomShellKey {
                    program: ShellProgram::Floor,
                    receive_shadows: true,
                    queue,
                },
                params,
                reads_uv3: false,
            })
        }
        other => Err(format!(
            "room material {name} uses {other}, which is not a room shell program"
        )),
    }
}

impl Material for RoomShellMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/room_shell.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/room_shell.wgsl".into()
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
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let key = key.bind_group_data;
        // Room/Floor (2005) must draw before the rugs (2007/2008), whose pass
        // compares depth with Always and writes none.
        crate::material_order::set_queue(descriptor, key.queue);
        let mut defs = vec![match key.program {
            ShellProgram::Object => "ROOM_OBJECT",
            ShellProgram::Floor => "ROOM_FLOOR",
        }];
        if key.receive_shadows {
            defs.push("ROOM_RECEIVE_SHADOWS");
        }
        // The vertex input the room programs read: the standard attributes at
        // the mesh pipeline's locations (it sets the matching VERTEX_* defs)
        // and the third and fourth uv sets where the mesh has them.
        let mut attributes = vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)];
        for (attribute, location) in [
            (Mesh::ATTRIBUTE_NORMAL, 1),
            (Mesh::ATTRIBUTE_UV_0, 2),
            (Mesh::ATTRIBUTE_UV_1, 3),
            (Mesh::ATTRIBUTE_COLOR, 5),
        ] {
            if layout.0.contains(attribute.id) {
                attributes.push(attribute.at_shader_location(location));
            }
        }
        for (attribute, location, def) in [
            (ATTRIBUTE_UV_2, UV_2_LOCATION, "ROOM_UV_2"),
            (ATTRIBUTE_UV_3, UV_3_LOCATION, "ROOM_UV_3"),
        ] {
            if layout.0.contains(attribute.id) {
                attributes.push(attribute.at_shader_location(location));
                defs.push(def);
            }
        }
        descriptor.vertex.buffers = vec![layout.0.get_layout(&attributes)?];
        for def in defs {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}

/// Put the third and fourth uv sets of every module primitive on its mesh,
/// read from the module glb's own `TEXCOORD_2` / `TEXCOORD_3` accessors (the
/// engine's glTF loader maps only `TEXCOORD_0` / `TEXCOORD_1`). A mesh that
/// already has a set keeps it. Returns how many primitives carry each set. A
/// primitive whose mesh does not have one vertex per accessor element is
/// refused: the set would be misaligned with the vertices.
pub(crate) fn attach_source_uv_sets(
    module: &Gltf,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &mut Assets<Mesh>,
) -> Result<[usize; 2], String> {
    let source = module
        .source
        .as_ref()
        .ok_or("room module was loaded without its glTF source")?;
    let blob = source
        .blob
        .as_deref()
        .ok_or("room module glb has no binary chunk")?;
    let mut counts = [0usize; 2];
    for (mesh_index, gltf_mesh) in source.document.meshes().enumerate() {
        let loaded = module
            .meshes
            .get(mesh_index)
            .and_then(|handle| gltf_meshes.get(handle))
            .ok_or_else(|| format!("room module mesh {mesh_index} is not loaded"))?;
        for primitive in gltf_mesh.primitives() {
            let target = &loaded
                .primitives
                .get(primitive.index())
                .ok_or_else(|| {
                    format!(
                        "room module mesh {mesh_index} primitive {} is not loaded",
                        primitive.index()
                    )
                })?
                .mesh;
            // A glb has one buffer, its binary chunk.
            let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob));
            for (slot, (set, attribute)) in [(2, ATTRIBUTE_UV_2), (3, ATTRIBUTE_UV_3)]
                .into_iter()
                .enumerate()
            {
                let Some(coords) = reader.read_tex_coords(set) else {
                    continue;
                };
                let values: Vec<[f32; 2]> = coords.into_f32().collect();
                let missing = || {
                    format!(
                        "room module mesh {mesh_index} primitive {} has no mesh",
                        primitive.index()
                    )
                };
                let mesh = meshes.get(target).ok_or_else(missing)?;
                if values.len() != mesh.count_vertices() {
                    return Err(format!(
                        "room module mesh {mesh_index} primitive {}: TEXCOORD_{set} has {} elements for {} vertices",
                        primitive.index(),
                        values.len(),
                        mesh.count_vertices()
                    ));
                }
                // Only a mesh without the set is touched, so a rebuild of the
                // materials does not re-upload the module meshes.
                if mesh.attribute(attribute).is_none() {
                    meshes
                        .get_mut(target)
                        .ok_or_else(missing)?
                        .try_insert_attribute(attribute, values)
                        .map_err(|error| format!("room module mesh {mesh_index}: {error}"))?;
                }
                counts[slot] += 1;
            }
        }
    }
    Ok(counts)
}

impl AsBindGroup for RoomShellMaterial {
    type Data = RoomShellKey;
    type Param = (
        SRes<SiteEnvGpuBuffer>,
        SRes<ShadowMapGpu>,
        SRes<RenderAssets<GpuImage>>,
        SRes<SharedSamplers>,
        SRes<SiteExtensionGpuBuffer>,
    );

    fn label() -> &'static str {
        "room_shell_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        (env_buffer, shadow, images, samplers, site_extension): &mut SystemParamItem<
            '_,
            '_,
            Self::Param,
        >,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let main = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let mut bytes = Vec::with_capacity(PARAM_SLOTS * 16);
        for slot in &self.params {
            for value in slot {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let bindings = BindingResources(vec![
            (0, OwnedBindingResource::Data(OwnedData(bytes))),
            (1, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (
                3,
                OwnedBindingResource::Sampler(
                    SamplerBindingType::Filtering,
                    samplers.repeat_linear.clone(),
                ),
            ),
            (
                4,
                OwnedBindingResource::Buffer(shadow.consumer_buffer.clone()),
            ),
            (
                5,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    shadow.depth_view.clone(),
                ),
            ),
            (
                6,
                OwnedBindingResource::Sampler(
                    SamplerBindingType::Comparison,
                    shadow.cmp_sampler.clone(),
                ),
            ),
            (
                7,
                OwnedBindingResource::Buffer(site_extension.buffer.clone()),
            ),
        ]);
        Ok(UnpreparedBindGroup { bindings })
    }

    fn bind_group_data(&self) -> Self::Data {
        self.key
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
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
            uniform(4),
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Depth,
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 6,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Comparison),
                count: None,
            },
            // binding 7: the site-extension globals (site_extension.rs).
            uniform(7),
        ]
    }
}
