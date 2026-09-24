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
//! The globals are the site ones (`SiteEnvGpuBuffer`: phenomena light and
//! shade, drop-shadow colour 1, sky-bottom colour, fog, edge pair, treasure
//! shadows) and the main-light shadow consumer block of `shadowmap.rs`.
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::mesh::MeshVertexBufferLayoutRef;
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

pub(crate) const OBJECT_SHADER: &str = "Mysekai/Object";
pub(crate) const FLOOR_SHADER: &str = "Mysekai/Room/Floor";

const PARAM_SLOTS: usize = 9;

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
    /// Usage 2 reads a fourth uv set the module glb does not carry.
    pub wall_ao_edge_missing: bool,
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
                    // _BaseTextureMappingMode: 0 uv0, 2 uv1 are the sets the
                    // module glb carries (1 world xz, 3 uv2 not ported here).
                    let mapping = get("_BaseTextureMappingMode")?;
                    domain(&name, "_BaseTextureMappingMode", mapping, &[0.0, 2.0])?;
                    if mapping == 2.0 {
                        1.0
                    } else {
                        0.0
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
                get("_UsePhenomenaLighting")?,
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
                // The edge factor only matters where it is not multiplied
                // away; with _WallAOIntensity 0 it is exactly 1.
                wall_ao_edge_missing: usage == 2.0 && params[6][0] != 0.0,
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
                    domain(&name, "_UVSelection", selection, &[0.0, 1.0])?;
                    selection
                }
            };
            params[1] = [index, 0.0, 0.0, 0.0];
            params[2] = [
                get("_UseFresnel")?,
                get("_FresnelPower")?,
                get("_UsePhenomenaLighting")?,
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
                wall_ao_edge_missing: false,
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
        _layout: &MeshVertexBufferLayoutRef,
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
        for def in defs {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}

impl AsBindGroup for RoomShellMaterial {
    type Data = RoomShellKey;
    type Param = (
        SRes<SiteEnvGpuBuffer>,
        SRes<ShadowMapGpu>,
        SRes<RenderAssets<GpuImage>>,
        SRes<SharedSamplers>,
    );

    fn label() -> &'static str {
        "room_shell_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        (env_buffer, shadow, images, samplers): &mut SystemParamItem<'_, '_, Self::Param>,
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
        ]
    }
}
