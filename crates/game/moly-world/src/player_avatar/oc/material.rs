//! The OC body's material: the base colour of the model's own material
//! (texture times factor, with its alpha mode and sidedness), lit by the same
//! weather light mix as the player avatar's material and cut by the same
//! bayer dither (`_DitherAlpha`, written by the player's dither driver).
//!
//! The avatar program has no lighting term of its own: its colour is the
//! albedo blended toward albedo times the weather light colour by the light's
//! blend factor, in the storage (gamma) domain. The OC material applies that
//! same mix to its albedo, so the OC sits in the weather and the time of day
//! exactly as the avatar does. It reads the avatar's frame globals (light
//! colour, screen parameters) from the same buffer. A model file carries a
//! VRM toon material or a glTF PBR material; both declare a base colour
//! texture and factor, which is what is read here. The factor is a linear
//! colour in glTF and is encoded to the storage domain before it multiplies
//! the texture, which is decoded back to the storage domain after sampling.

use bevy::asset::uuid::Uuid;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::shader::ShaderRef;
use std::marker::PhantomData;

const SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x6f63_5f6d_6174_6572_6961_6c00_0000_0001),
    PhantomData,
);

/// Parameter uniform: two vec4 slots. The slot order is a contract with
/// `OcParams` in `shaders/oc_material.wesl`.
pub const PARAMS_BYTES: usize = 2 * 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcParams {
    /// Base colour factor, rgb in the storage domain, alpha as authored.
    pub base_color: [f32; 4],
    /// x: alpha cutoff (mask mode) · y: `_DitherAlpha` · z: 1 when the
    /// vertex colour multiplies the base colour, else 0 · w: unused.
    pub cutoff_dither: [f32; 4],
}

impl OcParams {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        for slot in [self.base_color, self.cutoff_dither] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes
    }
}

/// Pipeline variant of one OC material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcKey {
    pub mask: bool,
    pub blend: bool,
    pub double_sided: bool,
}

#[derive(Debug, Clone, TypePath, Asset)]
pub struct OcMaterial {
    pub params: OcParams,
    pub key: OcKey,
    pub base_texture: Option<Handle<Image>>,
}

impl OcMaterial {
    /// The OC material of one of the model's own materials. A glTF PBR
    /// material multiplies the vertex colour into its base colour; the VRM
    /// toon materials do not (`vertex_colors` false).
    pub fn from_standard(material: &StandardMaterial, vertex_colors: bool) -> Self {
        let linear = material.base_color.to_linear();
        let encoded = material.base_color.to_srgba();
        let (mask, blend, cutoff) = match material.alpha_mode {
            AlphaMode::Mask(cutoff) => (true, false, cutoff),
            AlphaMode::Opaque => (false, false, 0.0),
            // Blend, premultiplied, additive, multiply and alpha-to-coverage
            // are drawn as plain alpha blending.
            _ => (false, true, 0.0),
        };
        Self {
            params: OcParams {
                base_color: [encoded.red, encoded.green, encoded.blue, linear.alpha],
                cutoff_dither: [cutoff, 1.0, if vertex_colors { 1.0 } else { 0.0 }, 0.0],
            },
            key: OcKey {
                mask,
                blend,
                double_sided: material.cull_mode.is_none(),
            },
            base_texture: material.base_color_texture.clone(),
        }
    }
}

impl AsBindGroup for OcMaterial {
    type Data = OcKey;
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<crate::avatar_material::AvatarEnvGpuBuffer>,
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "oc_material"
    }

    fn build_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        (env_buffer, images, fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
        output: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        let image = match &self.base_texture {
            Some(handle) => images.get(handle).ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        let bindings = UnpreparedBindingResources(vec![
            (0, crate::render::gpu::push_data(output, &self.params.bytes())),
            (1, UnpreparedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    image.texture_view.clone(),
                ),
            ),
            // The texture's own sampler: the model file's wrap and filter modes.
            (
                3,
                UnpreparedBindingResource::Sampler(
                    SamplerBindingType::Filtering,
                    image.sampler.clone(),
                ),
            ),
        ]);
        output.binding_resources.0.extend(bindings.0);
        Ok(())
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
        ]
    }
}

impl Material for OcMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.key.blend {
            AlphaMode::Blend
        } else if self.key.mask {
            AlphaMode::Mask(self.params.cutoff_dither[0])
        } else {
            AlphaMode::Opaque
        }
    }

    /// Like the avatar's material: the colour pass only; the site's shadow
    /// map draws the OC through its own skinned caster route.
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.double_sided {
            descriptor.primitive.cull_mode = None;
        }
        let mut defs: Vec<&str> = Vec::new();
        if key.bind_group_data.mask {
            defs.push("OC_ALPHA_MASK");
        }
        if key.bind_group_data.blend {
            defs.push("OC_ALPHA_BLEND");
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

/// The material pipeline and its shader.
pub fn install(app: &mut App) {
    crate::gpu_image_release::prepare_after_images::<OcMaterial>(app);
    app.add_plugins(MaterialPlugin::<OcMaterial>::default());
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            SHADER.id(),
            Shader::from_wesl(
                include_str!("../../shaders/oc_material.wesl"),
                "moly_world/src/shaders/oc_material.wesl".to_owned(),
            ),
        )
        .expect("OC material shader installation");
}
