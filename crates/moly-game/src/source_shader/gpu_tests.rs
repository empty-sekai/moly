//! Opt-in GPU binding tests use current supplied assets and explicit fixture
//! globals/vertices. They do not claim that a weather camera or pass owner is
//! already connected merely because these isolated draw packets are correct.
use super::*;
use bevy::{
    asset::LoadState,
    render::{render_asset::RenderAssets, RenderApp},
};
use moly_assets::source_shader::{material::MaterialSnapshot, SourceShaderCatalogue};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn fixture_global(name: &str) -> UniformValue {
    let identity = vec![
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    UniformValue::Float(match name {
        "hlslcc_mtx4x4unity_ObjectToWorld"
        | "hlslcc_mtx4x4unity_WorldToObject"
        | "hlslcc_mtx4x4unity_MatrixVP"
        | "hlslcc_mtx4x4unity_MatrixV" => identity,
        "_GlobalMipBias" => vec![0.0, 0.0],
        "_GlobalPhenomenaDirectionalLightColor" => vec![0.8, 0.6, 0.4, 1.0],
        "_WorldSpaceCameraPos" => vec![0.0, 0.0, 3.0],
        "_ProjectionParams" => vec![1.0, 0.1, 100.0, 0.01],
        "_ZBufferParams" => vec![-999.0, 1000.0, -9.99, 10.0],
        "unity_OrthoParams" => vec![2.0, 2.0, 0.0, 1.0],
        _ => panic!("GPU fixture has no writer for {name}"),
    })
}
fn vertices(abi: &ProgramAbi) -> (VertexBufferLayout, Vec<u8>, Vec<Value>) {
    let inputs = abi
        .interfaces
        .vertex
        .iter()
        .filter(|v| v.direction == "in")
        .collect::<Vec<_>>();
    let mut attributes = Vec::new();
    let mut stride = 0;
    let mut specification = Vec::new();
    for field in &inputs {
        let format = match (field.ty.as_str(), field.adapter.as_deref()) {
            ("vec4", Some("vec3-position-w-one")) => VertexFormat::Float32x3,
            ("vec2", None) => VertexFormat::Float32x2,
            ("vec3", None) => VertexFormat::Float32x3,
            ("vec4", None) => VertexFormat::Float32x4,
            _ => panic!("fixture vertex type {}", field.ty),
        };
        attributes.push(VertexAttribute {
            format,
            offset: stride,
            shader_location: field.location,
        });
        specification.push(json!({"name":field.name,"location":field.location,"offset":stride,"components":format.size()/4}));
        stride += format.size();
    }
    let mut bytes = Vec::new();
    for (x, y, u, v) in [
        (-0.9f32, -0.9f32, 0.0, 0.0),
        (0.9, -0.9, 1.0, 0.0),
        (0.9, 0.9, 1.0, 1.0),
        (-0.9, 0.9, 0.0, 1.0),
    ] {
        for (field, attribute) in inputs.iter().zip(&attributes) {
            let value: Vec<f32> = match field.name.as_str() {
                "in_POSITION0" => vec![x, y, 0.0, 1.0],
                "in_NORMAL0" => vec![0.0, 0.0, 1.0],
                "in_TANGENT0" => vec![1.0, 0.0, 0.0, 1.0],
                "in_COLOR0" => vec![0.75, 0.5, 0.25, 0.7],
                "in_TEXCOORD0" => vec![u, v, 0.0, 0.0],
                "in_TEXCOORD1" => vec![0.3, 0.4, 0.5, 0.6],
                "in_TEXCOORD2" => vec![0.7, 0.8, 0.9, 1.0],
                _ => panic!("fixture has no source vertex writer {}", field.name),
            };
            for value in &value[..(attribute.format.size() / 4) as usize] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    (
        VertexBufferLayout {
            array_stride: stride,
            step_mode: VertexStepMode::Vertex,
            attributes,
        },
        bytes,
        specification,
    )
}
fn value_json(value: &UniformValue) -> Value {
    match value {
        UniformValue::Float(v) => json!({"type":"float","value":v}),
        UniformValue::Signed(v) => json!({"type":"int","value":v}),
        UniformValue::Unsigned(v) => json!({"type":"uint","value":v}),
        UniformValue::Bool(v) => json!({"type":"bool","value":v}),
    }
}
fn load(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
#[ignore = "requires a real GPU and a caller-supplied current-program binding manifest"]
fn actual_source_program_binding_pixels() {
    let manifest =
        load(std::env::var_os("MOLY_SOURCE_GPU_MANIFEST").expect("MOLY_SOURCE_GPU_MANIFEST"));
    let root = PathBuf::from(manifest["sourceRoot"].as_str().unwrap());
    let output =
        PathBuf::from(std::env::var_os("MOLY_SOURCE_GPU_OUT").expect("MOLY_SOURCE_GPU_OUT"));
    std::fs::create_dir_all(&output).unwrap();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::render::RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    moly_assets::source_shader::register(&mut app);
    app.add_plugins(SourceShaderPlugin);
    app.finish();
    app.cleanup();
    let mut reports = Vec::new();
    for (index, case) in manifest["sourceCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let material = MaterialSnapshot::parse(&case["material"]).unwrap();
        let catalogue_bytes = std::fs::read(root.join(&material.shader.variants.file)).unwrap();
        material.shader.variants.verify(&catalogue_bytes).unwrap();
        let catalogue = SourceShaderCatalogue::parse(&catalogue_bytes).unwrap();
        material.matches(&catalogue).unwrap();
        let pass = catalogue
            .passes
            .iter()
            .find(|p| {
                p.sub_shader_index as u64 == case["subShaderIndex"].as_u64().unwrap()
                    && p.pass_index as u64 == case["passIndex"].as_u64().unwrap()
            })
            .unwrap();
        let state =
            SourcePassState::resolve(pass, |name| material.scalar_property(&catalogue, name))
                .unwrap();
        let program_path = case["program"]["file"].as_str().unwrap();
        let receipt =
            ProgramReceipt::parse(&std::fs::read(root.join(program_path)).unwrap()).unwrap();
        let program_handle: Handle<SourceProgramAsset> = app
            .world()
            .resource::<AssetServer>()
            .load(program_path.to_owned());
        let mut texture_handles = BTreeMap::new();
        for field in &receipt.abi.textures {
            let source = material.textures[&field.name]
                .texture
                .as_ref()
                .expect("fixture has no material-owned texture");
            let handle: Handle<SourceTextureAsset> = app
                .world()
                .resource::<AssetServer>()
                .load(source.content.file.clone());
            texture_handles.insert(field.name.clone(), handle);
        }
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            app.update();
            let server = app.world().resource::<AssetServer>();
            if let Some(LoadState::Failed(error)) = server.get_load_state(program_handle.id()) {
                panic!("source program load failed: {error}");
            }
            for handle in texture_handles.values() {
                if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
                    panic!("source texture load failed: {error}");
                }
            }
            let world = app.sub_app(RenderApp).world();
            if world
                .resource::<RenderAssets<GpuSourceProgram>>()
                .get(program_handle.id())
                .is_some()
                && texture_handles.values().all(|h| {
                    world
                        .resource::<RenderAssets<PreparedSourceTexture>>()
                        .get(h.id())
                        .is_some()
                })
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "source assets did not become GPU-ready"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let values = receipt
            .abi
            .uniforms
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    material
                        .uniform(&catalogue, f)
                        .unwrap()
                        .unwrap_or_else(|| fixture_global(&f.name)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let (vertex_layout, vertex_bytes, vertex_spec) = vertices(&receipt.abi);
        let pipeline_id = {
            let world = app.sub_app(RenderApp).world();
            let program = world
                .resource::<RenderAssets<GpuSourceProgram>>()
                .get(program_handle.id())
                .unwrap();
            let descriptor = pipeline_descriptor(
                program,
                state,
                vertex_layout.clone(),
                SourcePipelineTarget {
                    color: TextureFormat::Rgba8Unorm,
                    depth: TextureFormat::Depth32Float,
                    samples: 1,
                    front_face: FrontFace::Ccw,
                },
            )
            .unwrap();
            world
                .resource::<PipelineCache>()
                .queue_render_pipeline(descriptor)
        };
        for _ in 0..10 {
            app.update();
        }
        let world = app.sub_app(RenderApp).world();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let cache = world.resource::<PipelineCache>();
        let pipeline = cache.get_render_pipeline(pipeline_id).unwrap_or_else(|| {
            panic!(
                "source pipeline not ready: {:?}",
                cache.get_render_pipeline_state(pipeline_id)
            )
        });
        let textures = world.resource::<RenderAssets<PreparedSourceTexture>>();
        let bindings = texture_handles
            .iter()
            .map(|(name, h)| {
                let texture = textures.get(h.id()).unwrap().result.as_ref().unwrap();
                assert_eq!(
                    texture.source.source,
                    material.textures[name].texture.as_ref().unwrap().source
                );
                (
                    name.clone(),
                    SourceSampledTexture {
                        image: texture,
                        encoding: SourceTextureEncoding::Raw,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let binding =
            SourceGpuBinding::create(device, cache, &receipt, &values, &bindings).unwrap();
        let vertex = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("source ABI fixture vertices"),
            contents: &vertex_bytes,
            usage: BufferUsages::VERTEX,
        });
        let index_bytes = [0u32, 1, 2, 0, 2, 3]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let indices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("source ABI fixture triangles"),
            contents: &index_bytes,
            usage: BufferUsages::INDEX,
        });
        let size = 64u32;
        let extent = Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&TextureDescriptor {
            label: Some("source raw colour probe"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&TextureDescriptor {
            label: Some("source reverse-depth probe"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let target_view = target.create_view(&default());
        let depth_view = depth.create_view(&default());
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("source binding pixels"),
            size: u64::from(size * size * 4),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("current source material binding draw"),
        });
        {
            let attachments = [Some(RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::new(0.12, 0.24, 0.36, 0.5).into()),
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("current source material selected pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*binding.group, &[]);
            pass.set_vertex_buffer(0, *vertex.slice(..));
            pass.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        device.poll(PollType::wait_indefinitely()).unwrap();
        receiver
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap();
        let pixels = readback.slice(..).get_mapped_range().to_vec();
        readback.unmap();
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        let clear = &pixels[..4];
        let non_clear_pixels = pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != clear)
            .count();
        let stem = format!("case-{index:02}");
        std::fs::write(output.join(format!("{stem}.rgba")), &pixels).unwrap();
        std::fs::write(output.join(format!("{stem}.vertices")), &vertex_bytes).unwrap();
        let raw = &pass.serialized_state;
        let report = json!({"case":index,"name":case["name"],"programIdentity":receipt.source.program.identity,"program":program_path,"sourceMaterial":case["material"]["sourceMaterial"]["source"],
            "width":size,"height":size,"rgba":format!("{stem}.rgba"),"vertices":format!("{stem}.vertices"),"vertexStride":vertex_layout.array_stride,"vertexAttributes":vertex_spec,
            "uniforms":values.iter().map(|(name,value)|(name.clone(),value_json(value))).collect::<BTreeMap<_,_>>(),
            "textures":receipt.abi.textures.iter().map(|field|json!({"name":field.name,"dimension":match field.dimension {SourceTextureDimension::D2=>"2d",SourceTextureDimension::D2Array=>"2d-array"},"sourceTexture":material.textures[&field.name].texture.as_ref().unwrap().content.file,"samplerOwner":"compiled-gles-coupled-texture","colorView":"raw"})).collect::<Vec<_>>(),
            "renderState":{"cull":state.raster.cull,"depthTest":state.raster.depth_test,"depthWrite":state.raster.depth_write,"colorMask":state.raster.color_mask,"srcColor":state.raster.src_color,"dstColor":state.raster.dst_color,"srcAlpha":state.raster.src_alpha,"dstAlpha":state.raster.dst_alpha,"colorOp":state.raster.color_op,"alphaOp":state.raster.alpha_op},
            "sourcePassState":raw,"nonClearPixels":non_clear_pixels,"visibleCoverageQualified":non_clear_pixels>0,"fixtureClear":[0.12,0.24,0.36,0.5],"pixelRange":[pixels.iter().min(),pixels.iter().max()],"scope":"product source-program GPU binder with explicit fixture inputs, not complete weather owner proof"});
        println!(
            "source GPU binding {} {} complete: {} bytes",
            index,
            case["name"],
            pixels.len()
        );
        reports.push(report);
        std::fs::write(
            output.join("binding-pixel-results.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
    assert!(!reports.is_empty());
    assert!(
        reports
            .iter()
            .any(|r| r["visibleCoverageQualified"] == true),
        "a background-only render cannot qualify GPU binding coverage"
    );
}
