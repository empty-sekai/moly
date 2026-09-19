//! GPU consumption of source-addressed programs and full source texture data.
//!
//! Material/global writers supply values explicitly. This path does not select a
//! shader by its display name, infer a sampler from a keyword, flatten arrays or
//! substitute a generic material. Rendering ownership is established by the
//! caller before submitting a draw; an asset being loaded is not that proof.
use bevy::{
    ecs::system::{SystemParamItem, lifetimeless::*},
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        render_asset::{AssetExtractionError, PrepareAssetError, RenderAsset, RenderAssetPlugin},
        render_resource::*,
        renderer::{RenderAdapter, RenderDevice, RenderQueue},
    },
};
use moly_assets::source_shader::{
    ProgramAbi, ProgramReceipt, SourceShaderError, UniformValue,
    abi::{ShaderStage, TextureDimension as SourceTextureDimension},
    loader::{SourceProgramAsset, SourceTextureAsset},
    sampler::CompiledTextureSlot,
    state::SourcePassState,
    texture::{SourceSampler, SourceTexture, SourceTextureKind},
};
use std::{collections::BTreeMap, sync::Arc};
type Result<T> = std::result::Result<T, SourceShaderError>;
fn require(value: bool, reason: impl Into<String>) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(SourceShaderError(reason.into()))
    }
}

pub struct SourceShaderPlugin;
impl Plugin for SourceShaderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RenderAssetPlugin::<PreparedSourceTexture>::default(),
            RenderAssetPlugin::<GpuSourceProgram>::default(),
        ));
    }
}

/// Both views share exactly the same source texels. The material/pass owner must
/// choose the view explicitly according to the source project's colour path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceTextureEncoding {
    Raw,
    SrgbDecoded,
}

pub struct GpuSourceTexture {
    pub source: Arc<SourceTexture>,
    pub texture: Texture,
    // GLES cannot reinterpret a texture through an sRGB view. Keep a second
    // allocation of the identical source mip/layer bytes on those devices.
    _srgb_texture: Option<Texture>,
    pub raw: TextureView,
    pub srgb: TextureView,
    pub dimension: TextureViewDimension,
}
impl GpuSourceTexture {
    pub fn upload(
        device: &RenderDevice,
        queue: &RenderQueue,
        asset: &SourceTextureAsset,
    ) -> Result<Self> {
        Self::upload_with_view_formats(device, queue, asset, true)
    }
    fn upload_with_view_formats(
        device: &RenderDevice,
        queue: &RenderQueue,
        asset: &SourceTextureAsset,
        view_formats: bool,
    ) -> Result<Self> {
        asset.source.verify_pixels(&asset.pixels)?;
        let source = &asset.source;
        let limits = device.limits();
        require(
            source.width <= limits.max_texture_dimension_2d
                && source.height <= limits.max_texture_dimension_2d
                && source.layers <= limits.max_texture_array_layers,
            "source texture exceeds this device's texture limits",
        )?;
        let dimension = match source.kind {
            SourceTextureKind::Texture2D => TextureViewDimension::D2,
            SourceTextureKind::Texture2DArray => TextureViewDimension::D2Array,
        };
        let create = |format, formats| {
            device.create_texture(&TextureDescriptor {
                label: Some("source texture: complete layer-major mip chain"),
                size: Extent3d {
                    width: source.width,
                    height: source.height,
                    depth_or_array_layers: source.layers,
                },
                mip_level_count: source.mip_count,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_DST
                    | TextureUsages::COPY_SRC,
                view_formats: formats,
            })
        };
        let texture = create(
            TextureFormat::Rgba8Unorm,
            if view_formats {
                &[TextureFormat::Rgba8UnormSrgb]
            } else {
                &[]
            },
        );
        let srgb_texture = (!view_formats).then(|| create(TextureFormat::Rgba8UnormSrgb, &[]));
        for target in std::iter::once(&texture).chain(srgb_texture.iter()) {
            for plane in &source.planes {
                // Queue writes permit tightly packed rows; no 256-byte copy-buffer
                // alignment or preview-image Y flip is introduced here.
                queue.write_texture(
                    TexelCopyTextureInfo {
                        texture: target,
                        mip_level: plane.mip,
                        origin: Origin3d {
                            x: 0,
                            y: 0,
                            z: plane.layer,
                        },
                        aspect: TextureAspect::All,
                    },
                    &asset.pixels[plane.offset as usize..(plane.offset + plane.bytes) as usize],
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(plane.width * 4),
                        rows_per_image: Some(plane.height),
                    },
                    Extent3d {
                        width: plane.width,
                        height: plane.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let view = |texture: &Texture, format| {
            texture.create_view(&TextureViewDescriptor {
                label: Some("qualified source texture view"),
                format: Some(format),
                dimension: Some(dimension),
                ..Default::default()
            })
        };
        Ok(Self {
            source: source.clone(),
            raw: view(&texture, TextureFormat::Rgba8Unorm),
            srgb: view(
                srgb_texture.as_ref().unwrap_or(&texture),
                TextureFormat::Rgba8UnormSrgb,
            ),
            _srgb_texture: srgb_texture,
            texture,
            dimension,
        })
    }
    pub fn view(&self, encoding: SourceTextureEncoding) -> &TextureView {
        match encoding {
            SourceTextureEncoding::Raw => &self.raw,
            SourceTextureEncoding::SrgbDecoded => &self.srgb,
        }
    }
}
/// A rejected GPU upload retains its exact source reason. It is not a retry
/// loop, a loaded substitute texture, or a fabricated bind-group error.
pub struct PreparedSourceTexture {
    pub result: Result<GpuSourceTexture>,
}
impl RenderAsset for PreparedSourceTexture {
    type SourceAsset = SourceTextureAsset;
    type Param = (SRes<RenderDevice>, SRes<RenderQueue>, SRes<RenderAdapter>);
    fn byte_len(source: &Self::SourceAsset) -> Option<usize> {
        Some(source.pixels.len())
    }
    fn prepare_asset(
        source: Self::SourceAsset,
        _id: AssetId<Self::SourceAsset>,
        param: &mut SystemParamItem<Self::Param>,
        _previous: Option<&Self>,
    ) -> std::result::Result<Self, PrepareAssetError<Self::SourceAsset>> {
        let supports_views = param
            .2
            .get_downlevel_capabilities()
            .flags
            .contains(DownlevelFlags::VIEW_FORMATS);
        let result =
            GpuSourceTexture::upload_with_view_formats(&param.0, &param.1, &source, supports_views);
        if let Err(error) = &result {
            error!(source=?source.source.source, %error, "source texture GPU upload rejected");
        }
        Ok(Self { result })
    }
    fn take_gpu_data(
        source: &mut Self::SourceAsset,
        _previous: Option<&Self>,
    ) -> std::result::Result<Self::SourceAsset, AssetExtractionError> {
        Ok(source.clone())
    }
}

/// Shader handles retain the verified generated program. PipelineCache compiles
/// those exact handles and reports backend compilation errors normally.
pub struct GpuSourceProgram {
    pub receipt: Arc<ProgramReceipt>,
    pub vertex: Handle<Shader>,
    pub fragment: Handle<Shader>,
}
impl RenderAsset for GpuSourceProgram {
    type SourceAsset = SourceProgramAsset;
    type Param = ();
    fn prepare_asset(
        source: Self::SourceAsset,
        _id: AssetId<Self::SourceAsset>,
        _: &mut SystemParamItem<Self::Param>,
        _previous: Option<&Self>,
    ) -> std::result::Result<Self, PrepareAssetError<Self::SourceAsset>> {
        Ok(Self {
            receipt: source.receipt,
            vertex: source.vertex,
            fragment: source.fragment,
        })
    }
    fn take_gpu_data(
        source: &mut Self::SourceAsset,
        _previous: Option<&Self>,
    ) -> std::result::Result<Self::SourceAsset, AssetExtractionError> {
        Ok(source.clone())
    }
}

fn visibility(stages: &[ShaderStage]) -> ShaderStages {
    stages.iter().fold(ShaderStages::empty(), |v, s| {
        v | match s {
            ShaderStage::Vertex => ShaderStages::VERTEX,
            ShaderStage::Fragment => ShaderStages::FRAGMENT,
        }
    })
}
/// Declared ABI, not shader name, determines every resource binding and stage.
pub fn binding_layout(abi: &ProgramAbi) -> Result<BindGroupLayoutDescriptor> {
    abi.validate()?;
    let stages = abi
        .uniforms
        .iter()
        .flat_map(|u| u.stages.iter())
        .copied()
        .collect::<Vec<_>>();
    require(
        abi.uniform_bytes > 0,
        "source program has no qualified uniform block",
    )?;
    let mut entries = vec![BindGroupLayoutEntry {
        binding: abi.uniform_binding,
        visibility: visibility(&stages),
        count: None,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: BufferSize::new(u64::from(abi.uniform_bytes)),
        },
    }];
    for texture in &abi.textures {
        entries.push(BindGroupLayoutEntry {
            binding: texture.texture_binding,
            visibility: visibility(&texture.stages),
            count: None,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: match texture.dimension {
                    SourceTextureDimension::D2 => TextureViewDimension::D2,
                    SourceTextureDimension::D2Array => TextureViewDimension::D2Array,
                },
                multisampled: false,
            },
        });
        entries.push(BindGroupLayoutEntry {
            binding: texture.sampler_binding,
            visibility: visibility(&texture.stages),
            count: None,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
        });
    }
    Ok(BindGroupLayoutDescriptor::new(
        "source program ABI",
        &entries,
    ))
}

/// A sampler plan is supplied by a proven texture or inline-sampler owner. This
/// translation deliberately cannot choose that owner or resolve a missing one.
pub fn sampler_descriptor(
    source: &SourceSampler,
    mip_count: u32,
) -> Result<SamplerDescriptor<'static>> {
    require(mip_count > 0, "source sampler has no mip range")?;
    require(
        source.mip_bias == 0.0,
        "nonzero source sampler bias needs a qualified shader/LOD adapter",
    )?;
    require(
        (0..=1).contains(&source.anisotropy),
        "source anisotropy requires device-capability and quality-setting ownership",
    )?;
    let (mag, min, mip) = match source.filter {
        0 => (
            FilterMode::Nearest,
            FilterMode::Nearest,
            FilterMode::Nearest,
        ),
        1 => (FilterMode::Linear, FilterMode::Linear, FilterMode::Nearest),
        2 => (FilterMode::Linear, FilterMode::Linear, FilterMode::Linear),
        _ => return Err(SourceShaderError("unknown source filter mode".into())),
    };
    let wrap = |value| match value {
        0 => Ok(AddressMode::Repeat),
        1 => Ok(AddressMode::ClampToEdge),
        2 => Ok(AddressMode::MirrorRepeat),
        _ => Err(SourceShaderError(
            "source wrap mode requires an explicit sampler adapter".into(),
        )),
    };
    Ok(SamplerDescriptor {
        label: Some("source-owned sampler"),
        address_mode_u: wrap(source.wrap_u)?,
        address_mode_v: wrap(source.wrap_v)?,
        address_mode_w: wrap(source.wrap_w)?,
        mag_filter: mag,
        min_filter: min,
        mipmap_filter: mip,
        lod_min_clamp: 0.0,
        lod_max_clamp: (mip_count - 1) as f32,
        compare: None,
        anisotropy_clamp: 1,
        border_color: None,
    })
}

pub struct SourceSampledTexture<'a> {
    pub image: &'a GpuSourceTexture,
    pub encoding: SourceTextureEncoding,
}
/// Renderer-owned resources have no asset identity. The view owner supplies
/// both the actual attachment view and its sampler, including nonfilterable
/// depth snapshots. They must never be wrapped in a fabricated source asset.
pub enum SourceSampledResource<'a> {
    Asset(SourceSampledTexture<'a>),
    View {
        view: &'a TextureView,
        sampler: &'a Sampler,
        dimension: TextureViewDimension,
        filterable: bool,
    },
}

pub fn resource_binding_layout(
    abi: &ProgramAbi,
    unfilterable: &std::collections::BTreeSet<String>,
) -> Result<BindGroupLayoutDescriptor> {
    let mut layout = binding_layout(abi)?;
    for name in unfilterable {
        let declaration = abi
            .textures
            .iter()
            .find(|t| &t.name == name)
            .ok_or_else(|| SourceShaderError(format!("unconsumed view resource {name}")))?;
        for entry in &mut layout.entries {
            if entry.binding == declaration.texture_binding {
                if let BindingType::Texture { sample_type, .. } = &mut entry.ty {
                    *sample_type = TextureSampleType::Float { filterable: false };
                }
            }
            if entry.binding == declaration.sampler_binding {
                entry.ty = BindingType::Sampler(SamplerBindingType::NonFiltering);
            }
        }
    }
    Ok(layout)
}
pub struct SourceGpuBinding {
    pub group: BindGroup,
    pub uniform: Buffer,
}
impl SourceGpuBinding {
    pub fn create(
        device: &RenderDevice,
        cache: &PipelineCache,
        program: &ProgramReceipt,
        values: &BTreeMap<String, UniformValue>,
        textures: &BTreeMap<String, SourceSampledTexture<'_>>,
    ) -> Result<Self> {
        let resources = textures
            .iter()
            .map(|(name, texture)| {
                (
                    name.clone(),
                    SourceSampledResource::Asset(SourceSampledTexture {
                        image: texture.image,
                        encoding: texture.encoding,
                    }),
                )
            })
            .collect();
        Self::create_resources(device, cache, program, values, &resources)
    }

    pub fn create_resources(
        device: &RenderDevice,
        cache: &PipelineCache,
        program: &ProgramReceipt,
        values: &BTreeMap<String, UniformValue>,
        textures: &BTreeMap<String, SourceSampledResource<'_>>,
    ) -> Result<Self> {
        let abi = &program.abi;
        let unfilterable = textures
            .iter()
            .filter_map(|(name, resource)| {
                matches!(
                    resource,
                    SourceSampledResource::View {
                        filterable: false,
                        ..
                    }
                )
                .then(|| name.clone())
            })
            .collect();
        let layout = resource_binding_layout(abi, &unfilterable)?;
        require(
            values.len() == abi.uniforms.len()
                && values
                    .keys()
                    .all(|key| abi.uniforms.iter().any(|u| &u.name == key)),
            "source uniform writers are missing or include an unconsumed input",
        )?;
        let bytes = abi.pack_uniforms(|field| {
            values
                .get(&field.name)
                .cloned()
                .ok_or_else(|| SourceShaderError(format!("unowned source uniform {}", field.name)))
        })?;
        require(
            textures.len() == abi.textures.len()
                && textures
                    .keys()
                    .all(|key| abi.textures.iter().any(|t| &t.name == key)),
            "source textures are missing or include an unconsumed resource",
        )?;
        for declaration in &abi.textures {
            let texture = &textures[&declaration.name];
            let dimension = match declaration.dimension {
                SourceTextureDimension::D2 => TextureViewDimension::D2,
                SourceTextureDimension::D2Array => TextureViewDimension::D2Array,
            };
            require(
                match texture {
                    SourceSampledResource::Asset(texture) => texture.image.dimension == dimension,
                    SourceSampledResource::View {
                        dimension: actual, ..
                    } => *actual == dimension,
                },
                format!(
                    "source texture {} dimensionality disagrees with its compiled program",
                    declaration.name
                ),
            )?;
        }
        let samplers = abi
            .textures
            .iter()
            .map(|declaration| {
                let owner = CompiledTextureSlot::resolve(
                    program.source.reference.gpu_program_type as i32,
                    program.source.parameters.as_ref(),
                    declaration,
                )?;
                let SourceSampledResource::Asset(texture) = &textures[&declaration.name] else {
                    return Ok((declaration.name.clone(), None));
                };
                let texture = texture.image;
                let descriptor = sampler_descriptor(
                    owner.source_sampler(&texture.source)?,
                    texture.source.mip_count,
                )?;
                Ok((
                    declaration.name.clone(),
                    Some(device.create_sampler(&descriptor)),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("source program uniform bytes"),
            contents: &bytes,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let mut entries = vec![BindGroupEntry {
            binding: abi.uniform_binding,
            resource: uniform.as_entire_binding(),
        }];
        for declaration in &abi.textures {
            let texture = &textures[&declaration.name];
            let (view, sampler) = match texture {
                SourceSampledResource::Asset(texture) => (
                    texture.image.view(texture.encoding),
                    samplers[&declaration.name].as_ref().expect("asset sampler"),
                ),
                SourceSampledResource::View { view, sampler, .. } => (*view, *sampler),
            };
            entries.push(BindGroupEntry {
                binding: declaration.texture_binding,
                resource: BindingResource::TextureView(view),
            });
            entries.push(BindGroupEntry {
                binding: declaration.sampler_binding,
                resource: BindingResource::Sampler(sampler),
            });
        }
        let group = device.create_bind_group(
            "source program exact resources",
            &cache.get_bind_group_layout(&layout),
            &entries,
        );
        Ok(Self { group, uniform })
    }

    /// Keep layouts, samplers and bind groups stable while camera/global writers
    /// update their values. Resource replacement is a separate preparation step.
    pub fn write_uniforms(
        &self,
        queue: &RenderQueue,
        abi: &ProgramAbi,
        values: &BTreeMap<String, UniformValue>,
    ) -> Result<()> {
        require(
            values.len() == abi.uniforms.len(),
            "source uniform writer count changed",
        )?;
        let bytes = abi.pack_uniforms(|field| {
            values
                .get(&field.name)
                .cloned()
                .ok_or_else(|| SourceShaderError(format!("unowned source uniform {}", field.name)))
        })?;
        queue.write_buffer(&self.uniform, 0, &bytes);
        Ok(())
    }
}

pub struct SourcePipelineTarget {
    pub color: TextureFormat,
    pub depth: TextureFormat,
    pub samples: u32,
    pub front_face: FrontFace,
}
/// The caller supplies the source-qualified pass and geometry layout. No pass
/// is selected by its first position, name, or apparent shader-family similarity.
pub fn pipeline_descriptor(
    program: &GpuSourceProgram,
    state: SourcePassState,
    vertices: VertexBufferLayout,
    target: SourcePipelineTarget,
) -> Result<RenderPipelineDescriptor> {
    let abi = &program.receipt.abi;
    let expected = abi
        .interfaces
        .vertex
        .iter()
        .filter(|i| i.direction == "in")
        .collect::<Vec<_>>();
    require(
        vertices.step_mode == VertexStepMode::Vertex && vertices.attributes.len() == expected.len(),
        "source vertex stream count or step mode disagrees with the compiled ABI",
    )?;
    for interface in expected {
        let attribute = vertices
            .attributes
            .iter()
            .find(|a| a.shader_location == interface.location)
            .ok_or_else(|| {
                SourceShaderError(format!(
                    "source vertex attribute {} is absent",
                    interface.name
                ))
            })?;
        let format = match (interface.ty.as_str(), interface.adapter.as_deref()) {
            ("vec4", Some("vec3-position-w-one")) => VertexFormat::Float32x3,
            ("float", None) => VertexFormat::Float32,
            ("vec2", None) => VertexFormat::Float32x2,
            ("vec3", None) => VertexFormat::Float32x3,
            ("vec4", None) => VertexFormat::Float32x4,
            _ => {
                return Err(SourceShaderError(format!(
                    "unqualified vertex ABI {}",
                    interface.name
                )));
            }
        };
        require(
            attribute.format == format,
            format!("source vertex format mismatch for {}", interface.name),
        )?;
        require(
            attribute.offset + format.size() <= vertices.array_stride,
            "source vertex attribute exceeds its stride",
        )?;
    }
    require(
        matches!(target.samples, 1 | 2 | 4 | 8 | 16),
        "unsupported source attachment sample count",
    )?;
    require(
        state.depth_clip,
        "disabled source depth clipping requires a device-qualified capability",
    )?;
    require(
        !state.alpha_to_coverage || target.samples > 1,
        "source alpha-to-coverage has no multisample target",
    )?;
    let mut descriptor = RenderPipelineDescriptor {
        label: Some(format!("source program {}", program.receipt.source.program.identity).into()),
        layout: vec![binding_layout(abi)?],
        vertex: VertexState {
            shader: program.vertex.clone(),
            shader_defs: vec![],
            entry_point: Some("main".into()),
            buffers: vec![vertices],
        },
        fragment: Some(FragmentState {
            shader: program.fragment.clone(),
            shader_defs: vec![],
            entry_point: Some("main".into()),
            targets: vec![Some(ColorTargetState {
                format: target.color,
                blend: None,
                write_mask: ColorWrites::empty(),
            })],
        }),
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            front_face: target.front_face,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(DepthStencilState {
            format: target.depth,
            depth_write_enabled: false,
            depth_compare: CompareFunction::Never,
            stencil: StencilState::default(),
            bias: DepthBiasState::default(),
        }),
        multisample: MultisampleState {
            count: target.samples,
            mask: !0,
            alpha_to_coverage_enabled: state.alpha_to_coverage,
        },
        ..Default::default()
    };
    crate::source_render_state::apply(&mut descriptor, state.raster);
    Ok(descriptor)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gpu_tests;
