//! Engine post-processing: the colour grading LUT and engine bloom, mounted upstream
//! of Mysekai post-processing.
//!
//! The phenomenon camera has `renderPostProcessing` enabled, so the renderer queues
//! two engine passes every frame: the colour grading LUT pass (bakes the volume
//! stack's colour-adjustment components into a 32³ LDR lookup table, flattened into a
//! 1024×32 `R8G8B8A8_UNorm` strip) and the engine post-processing pass (on the
//! `BeforeRenderingPostProcessing` event: scene colour + engine bloom, multiplied by
//! the exposure factor, clamped to 0..1, then looked up in that table). Mysekai's own
//! uber pass follows on `AfterRenderingPostProcessing` and reads the camera colour
//! written by engine post-processing. This node therefore runs after tonemapping and
//! before the weather chain, and **runs for every phenomenon**.
//!
//! The table is baked on the main-world CPU (the rule is in `moly_law::weather::lut`
//! and agrees value by value with the program shipped with the build: differences
//! below 1e-5, and after 8-bit quantisation 40 bytes differ by 1, all on rounding
//! boundaries caused by last-bit differences between math libraries): the product
//! bakes it every frame, but its input changes only when a whole post-processing
//! profile is switched, so this side bakes once on the frame its input changes and
//! produces the same table.
//!
//! Engine bloom (when the `Bloom` component is active and its intensity > 0):
//! half-resolution threshold prefilter → per level "horizontal 9-tap Gaussian with
//! half downsample + vertical 5-tap" → per level upsample weighted by scatter → the
//! uber pass squares the top level, multiplies by intensity and tint′ and adds it to
//! the scene colour. The pyramid's storage format is set by the pipeline asset: the
//! engine post-processing pass uses the format the renderer requests (when that
//! format can serve as a linear render target), and the renderer requests
//! `MakeRenderTextureGraphicsFormat(supportsHDR, …)` while this pipeline asset has
//! `supportsHDR` off ⇒ the platform default LDR format is requested, which in gamma
//! colour space is `R8G8B8A8_UNorm`, a valid render target on any device ⇒ neither
//! the B10G11R11 nor the RGBM branch is taken.
//! So each level stores the 8-bit UNorm of the square root of its working values,
//! and this side stores the same format.

use std::marker::PhantomData;
use std::sync::Arc;

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::core_pipeline::FullscreenShader;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    AddressMode, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferInitDescriptor,
    BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState,
    Operations, Origin3d, PipelineCache, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderStages, ShaderType, TexelCopyBufferLayout, TexelCopyTextureInfo,
    Texture, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
    TextureUsages, TextureView,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use moly_law::weather::lut::{self, LdrLutInputs, LutStack};
use moly_law::weather::StockBloomParams;

use crate::render::gpu::{Bound, SharedBindGroupCache};
use crate::weather::{weather_post, TransparentCapture, WeatherPostParams};

const STOCK_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x3e0b_71c4_92d5_4f18_a6c7_58e2_0d91_b4f3),
    PhantomData,
);

/// 泛光金字塔的存储格式：8 位 UNorm、不做 sRGB 编解码（见模块 doc）。
const PYRAMID_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

// ---- 主世界 ---------------------------------------------------------------

/// 主世界 → 渲染世界：烘好的表（按代号去重上传）、uber 查表参数、引擎
/// 泛光的门与采纳值。
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct StockPostState {
    lut: Option<Arc<Vec<u8>>>,
    /// 每烘一次加一；渲染世界只在代号变时上传。0 = 还没有表。
    generation: u64,
    /// uber 的 `_Lut_Params`：`(1/宽, 1/高, 高 − 1, 2^postExposure)`。
    lut_params: [f32; 4],
    bloom_on: bool,
    bloom: Option<StockBloomParams>,
}

/// PostUpdate：天气侧写出本帧的后处理输入之后，输入变了就重烘表、换泛光
/// 参数。档案解出之前没有输入：不烘、节点整段旁路（画面未调色，与「引擎
/// 后处理还没有档案」同义）。
fn bake_lut(
    post: Res<WeatherPostParams>,
    mut state: ResMut<StockPostState>,
    mut baked: Local<Option<LutStack>>,
) {
    let Some(stack) = post.lut else {
        return;
    };
    let mut changed = false;
    let target = state.bypass_change_detection();
    if baked.as_ref() != Some(&stack) {
        let bytes = lut::bake(&LdrLutInputs::from_stack(&stack));
        target.lut = Some(Arc::new(bytes));
        target.generation += 1;
        target.lut_params = lut::uber_lut_params(stack.post_exposure);
        *baked = Some(stack);
        changed = true;
        info!(
            "引擎颜色分级：LDR 查找表 {}×{} 重烘（第 {} 张）；曝光 {:.4} EV → 倍数 {:.6}，对比 {:.3}%，饱和 {:.3}%，色相 {:.3}°，分离色调平衡 {:.3}",
            lut::LUT_WIDTH,
            lut::LUT_SIZE,
            target.generation,
            stack.post_exposure,
            target.lut_params[3],
            stack.contrast,
            stack.saturation,
            stack.hue_shift,
            stack.split_balance,
        );
    }
    let bloom = post.stock_bloom_on.then_some(post.stock_bloom);
    if target.bloom_on != post.stock_bloom_on || target.bloom != bloom {
        target.bloom_on = post.stock_bloom_on;
        target.bloom = bloom;
        changed = true;
        if let Some(b) = bloom {
            info!(
                "引擎泛光：开（threshold {:.4} gamma 域，intensity {:.4}，scatter {:.4}，clamp {}，tint {:?}，downscale {}，maxIterations {}）",
                b.threshold, b.intensity, b.scatter, b.clamp, b.tint, b.downscale, b.max_iterations
            );
        }
    }
    if changed {
        state.set_changed();
    }
}

// ---- 渲染世界 -------------------------------------------------------------

#[derive(Debug, Clone, Copy, ShaderType)]
struct StockPostUniform {
    /// 预滤波与上采样：`(Lerp(0.05, 0.95, scatter), clamp, 线性阈值, 软膝)`。
    prefilter: [f32; 4],
    /// 引擎 uber：`(泛光强度, tint′.rgb)`；泛光关着时强度 0。
    bloom: [f32; 4],
    /// 查表：`(1/宽, 1/高, 高 − 1, 2^postExposure)`。
    lut: [f32; 4],
}

impl StockPostUniform {
    fn bytes(&self) -> Vec<u8> {
        [self.prefilter, self.bloom, self.lut]
            .iter()
            .flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes()))
            .collect()
    }
}

#[derive(Resource)]
struct StockPostGpu {
    lut_texture: Texture,
    lut_view: TextureView,
    /// 已上传的表代号（0 = 还没有表，节点旁路）。
    uploaded: u64,
    uniform: Buffer,
}

#[derive(Resource)]
struct StockPostPipelines {
    prefilter: CachedRenderPipelineId,
    blur_h: CachedRenderPipelineId,
    blur_v: CachedRenderPipelineId,
    upsample: CachedRenderPipelineId,
    uber: CachedRenderPipelineId,
    /// 预滤波（0 场景、3 参数）。
    prefilter_layout: BindGroupLayoutDescriptor,
    /// 横纵模糊（0 源、2 采样器）。
    blur_layout: BindGroupLayoutDescriptor,
    /// 上采样（0 本级、1 低一级、2 采样器、3 参数）。
    upsample_layout: BindGroupLayoutDescriptor,
    /// uber（0 场景、1 泛光顶层、2 采样器、3 参数、4 查找表）。
    uber_layout: BindGroupLayoutDescriptor,
    /// Bilinear, clamp-to-edge: all of these textures use a linear clamp-to-edge sampler.
    sampler: Sampler,
    black: CachedTexture,
}

/// Engine bloom pyramid of one view: `downs[i]` and `ups[i]` have the same size (the
/// program has two textures per level: the downsample result and the upsample result,
/// the latter also serving as the temporary target of the horizontal blur during
/// downsampling).
#[derive(Component)]
pub(crate) struct StockBloomPyramid {
    downs: Vec<CachedTexture>,
    ups: Vec<CachedTexture>,
}

fn init_stock_post(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    let prefilter_layout = BindGroupLayoutDescriptor::new(
        "stock_post_prefilter_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            ((0, tex()), (3, uniform_buffer::<StockPostUniform>(false))),
        ),
    );
    let blur_layout = BindGroupLayoutDescriptor::new(
        "stock_post_blur_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            ((0, tex()), (2, sampler(SamplerBindingType::Filtering))),
        ),
    );
    let upsample_layout = BindGroupLayoutDescriptor::new(
        "stock_post_upsample_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, tex()),
                (1, tex()),
                (2, sampler(SamplerBindingType::Filtering)),
                (3, uniform_buffer::<StockPostUniform>(false)),
            ),
        ),
    );
    let uber_layout = BindGroupLayoutDescriptor::new(
        "stock_post_uber_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, tex()),
                (1, tex()),
                (2, sampler(SamplerBindingType::Filtering)),
                (3, uniform_buffer::<StockPostUniform>(false)),
                (4, tex()),
            ),
        ),
    );
    let vertex = fullscreen_shader.to_vertex_state();
    let pipeline = |entry: &str, format: TextureFormat, layout: &BindGroupLayoutDescriptor| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("stock_post_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: vertex.clone(),
            fragment: Some(FragmentState {
                shader: STOCK_SHADER.clone(),
                entry_point: Some(entry.to_owned().into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: default(),
            ..default()
        })
    };
    let prefilter = pipeline("stock_bloom_prefilter", PYRAMID_FORMAT, &prefilter_layout);
    let blur_h = pipeline("stock_bloom_blur_h", PYRAMID_FORMAT, &blur_layout);
    let blur_v = pipeline("stock_bloom_blur_v", PYRAMID_FORMAT, &blur_layout);
    let upsample = pipeline("stock_bloom_upsample", PYRAMID_FORMAT, &upsample_layout);
    // 相机是 LDR：主纹理即引擎默认格式（与天气链合成 pass 同一目标格式）。
    let uber = pipeline("stock_uber", TextureFormat::Rgba8UnormSrgb, &uber_layout);

    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("stock_post_linear_clamp"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let small = |label: &'static str, width: u32, height: u32, format: TextureFormat| {
        render_device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: bevy::render::render_resource::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let black_texture = small("stock_post_black", 1, 1, TextureFormat::Rgba8Unorm);
    render_queue.write_texture(
        TexelCopyTextureInfo {
            texture: &black_texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &[0u8, 0, 0, 255],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        bevy::render::render_resource::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let black_view = black_texture.create_view(&default());
    let lut_texture = small(
        "stock_post_lut",
        lut::LUT_WIDTH as u32,
        lut::LUT_SIZE as u32,
        // The LDR table is `R8G8B8A8_UNorm`: it already stores graded values, so no
        // sRGB encode/decode is applied.
        TextureFormat::Rgba8Unorm,
    );
    let lut_view = lut_texture.create_view(&default());
    let uniform = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("stock_post_uniform"),
        contents: &StockPostUniform {
            prefilter: [0.0; 4],
            bloom: [0.0; 4],
            lut: [0.0; 4],
        }
        .bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(StockPostPipelines {
        prefilter,
        blur_h,
        blur_v,
        upsample,
        uber,
        prefilter_layout,
        blur_layout,
        upsample_layout,
        uber_layout,
        sampler,
        black: CachedTexture {
            texture: black_texture,
            default_view: black_view,
        },
    });
    commands.insert_resource(StockPostGpu {
        lut_texture,
        lut_view,
        uploaded: 0,
        uniform,
    });
}

/// Prepare：表按代号上传，参数块在状态变的那一帧整块写。
fn write_stock_post(
    state: Res<StockPostState>,
    mut gpu: ResMut<StockPostGpu>,
    queue: Res<RenderQueue>,
) {
    if let Some(bytes) = state.lut.as_ref() {
        if gpu.uploaded != state.generation {
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &gpu.lut_texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                bytes,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(lut::LUT_WIDTH as u32 * 4),
                    rows_per_image: Some(lut::LUT_SIZE as u32),
                },
                bevy::render::render_resource::Extent3d {
                    width: lut::LUT_WIDTH as u32,
                    height: lut::LUT_SIZE as u32,
                    depth_or_array_layers: 1,
                },
            );
            gpu.uploaded = state.generation;
        }
    }
    if !state.is_changed() {
        return;
    }
    let (prefilter, bloom) = match state.bloom.as_ref() {
        Some(b) if state.bloom_on => (b.prefilter_params(), b.uber_params()),
        _ => ([0.0; 4], [0.0, 1.0, 1.0, 1.0]),
    };
    let uniform = StockPostUniform {
        prefilter,
        bloom,
        lut: state.lut_params,
    };
    queue.write_buffer(&gpu.uniform, 0, &uniform.bytes());
}

fn pyramid_descriptor(label: &'static str, width: u32, height: u32) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: Some(label),
        size: bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: PYRAMID_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }
}

/// PrepareResources：引擎泛光开着时，按当帧视口现算金字塔（尺寸律归律：
/// 首级是相机目标尺寸右移 1 或 2 位，级数由长边定）。
fn prepare_stock_bloom_pyramid(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    state: Res<StockPostState>,
    views: Query<
        (Entity, &ExtractedCamera, Option<&StockBloomPyramid>),
        Without<TransparentCapture>,
    >,
    mut reported: Local<Option<(u32, u32, usize)>>,
) {
    for (entity, camera, previous) in &views {
        let bloom = state.bloom.filter(|_| state.bloom_on);
        let viewport = camera.physical_viewport_size;
        let (Some(bloom), Some(viewport), true) =
            (bloom, viewport, camera.schedule == Core3d.intern())
        else {
            if previous.is_some() {
                commands.entity(entity).remove::<StockBloomPyramid>();
            }
            continue;
        };
        let plan = match bloom.plan(viewport.x, viewport.y) {
            Ok(plan) => plan,
            Err(err) => {
                error!("引擎泛光：{err}；本帧不画泛光");
                // Drop last frame's pyramid too, or the node would keep
                // compositing a stale one.
                if previous.is_some() {
                    commands.entity(entity).remove::<StockBloomPyramid>();
                }
                continue;
            }
        };
        let key = (plan.width, plan.height, plan.mip_count);
        if *reported != Some(key) {
            *reported = Some(key);
            info!(
                "引擎泛光金字塔：视口 {}×{} → 首级 {}×{}，{} 级",
                viewport.x, viewport.y, plan.width, plan.height, plan.mip_count
            );
            if plan.mip_count < 2 {
                warn!("engine bloom: with only 1 level the uber pass reads an upsample top level that was never written, so its content is undefined; this side uses a black image (zero contribution)");
            }
        }
        let (mut w, mut h) = (plan.width.max(1), plan.height.max(1));
        let mut downs = Vec::with_capacity(plan.mip_count);
        let mut ups = Vec::with_capacity(plan.mip_count);
        for _ in 0..plan.mip_count {
            downs.push(
                texture_cache.get(&render_device, pyramid_descriptor("stock_bloom_down", w, h)),
            );
            ups.push(texture_cache.get(&render_device, pyramid_descriptor("stock_bloom_up", w, h)));
            w = (w >> 1).max(1);
            h = (h >> 1).max(1);
        }
        commands
            .entity(entity)
            .insert(StockBloomPyramid { downs, ups });
    }
}

/// Core3d：引擎原生后处理（bloom 金字塔 + uber），排在色调映射之后、天气后处理之前。
pub(crate) fn stock_post(
    world: &World,
    view: ViewQuery<(&ViewTarget, Option<&StockBloomPyramid>, Option<&TransparentCapture>)>,
    bind_groups: Local<SharedBindGroupCache>,
    mut render_context: RenderContext,
) {
    let (view_target, pyramid, transparent_capture) = view.into_inner();
    if transparent_capture.is_some() {
        return;
    }
    let gpu = world.resource::<StockPostGpu>();
    if gpu.uploaded == 0 {
        // 档案还没解出：没有表可查。
        return;
    }
    let state = world.resource::<StockPostState>();
    let pipelines = world.resource::<StockPostPipelines>();
    let cache_p = world.resource::<PipelineCache>();
    let (Some(prefilter), Some(blur_h), Some(blur_v), Some(upsample), Some(uber)) = (
        cache_p.get_render_pipeline(pipelines.prefilter),
        cache_p.get_render_pipeline(pipelines.blur_h),
        cache_p.get_render_pipeline(pipelines.blur_v),
        cache_p.get_render_pipeline(pipelines.upsample),
        cache_p.get_render_pipeline(pipelines.uber),
    ) else {
        return;
    };
    let post_process = view_target.post_process_write();
    let frame = world.resource::<FrameCount>().0;
    let n = match pyramid {
        Some(p) if state.bloom_on && p.downs.len() >= 2 => p.downs.len(),
        _ => 0,
    };
    let bloom_view: &TextureView = match pyramid {
        Some(p) if n > 0 => &p.ups[0].default_view,
        _ => &pipelines.black.default_view,
    };

    let mut cache = bind_groups.lock();
    let device = render_context.render_device();
    let sampler_ = &pipelines.sampler;
    let prefilter_layout = cache_p.get_bind_group_layout(&pipelines.prefilter_layout);
    let blur_layout = cache_p.get_bind_group_layout(&pipelines.blur_layout);
    let upsample_layout = cache_p.get_bind_group_layout(&pipelines.upsample_layout);
    let uber_layout = cache_p.get_bind_group_layout(&pipelines.uber_layout);
    // (管线, bind group, 目标, 标签)，先全部取好再跑（跑 pass 要可变借用）。
    let mut passes: Vec<(
        &RenderPipeline,
        bevy::render::render_resource::BindGroup,
        &TextureView,
        &'static str,
    )> = Vec::new();
    if let (Some(p), true) = (pyramid, n > 0) {
        let group = cache.get(
            device,
            "stock_bloom_prefilter",
            &prefilter_layout,
            &[
                (0, Bound::View(post_process.source)),
                (3, Bound::whole(&gpu.uniform)),
            ],
            frame,
        );
        passes.push((
            prefilter,
            group,
            &p.downs[0].default_view,
            "stock_bloom_prefilter",
        ));
        for i in 1..n {
            let h_group = cache.get(
                device,
                "stock_bloom_blur",
                &blur_layout,
                &[
                    (0, Bound::View(&p.downs[i - 1].default_view)),
                    (2, Bound::Sampler(sampler_)),
                ],
                frame,
            );
            passes.push((
                blur_h,
                h_group,
                &p.ups[i].default_view,
                "stock_bloom_blur_h",
            ));
            let v_group = cache.get(
                device,
                "stock_bloom_blur",
                &blur_layout,
                &[
                    (0, Bound::View(&p.ups[i].default_view)),
                    (2, Bound::Sampler(sampler_)),
                ],
                frame,
            );
            passes.push((
                blur_v,
                v_group,
                &p.downs[i].default_view,
                "stock_bloom_blur_v",
            ));
        }
        for i in (0..n - 1).rev() {
            let low = if i == n - 2 {
                &p.downs[i + 1].default_view
            } else {
                &p.ups[i + 1].default_view
            };
            let group = cache.get(
                device,
                "stock_bloom_upsample",
                &upsample_layout,
                &[
                    (0, Bound::View(&p.downs[i].default_view)),
                    (1, Bound::View(low)),
                    (2, Bound::Sampler(sampler_)),
                    (3, Bound::whole(&gpu.uniform)),
                ],
                frame,
            );
            passes.push((
                upsample,
                group,
                &p.ups[i].default_view,
                "stock_bloom_upsample",
            ));
        }
    }
    let uber_group = cache.get(
        device,
        "stock_uber",
        &uber_layout,
        &[
            (0, Bound::View(post_process.source)),
            (1, Bound::View(bloom_view)),
            (2, Bound::Sampler(sampler_)),
            (3, Bound::whole(&gpu.uniform)),
            (4, Bound::View(&gpu.lut_view)),
        ],
        frame,
    );
    passes.push((uber, uber_group, post_process.destination, "stock_uber"));
    drop(cache);

    for (pipeline, group, target, label) in &passes {
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn load_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        STOCK_SHADER.id(),
        Shader::from_wesl(
            include_str!("shaders/weather_stock_post.wesl"),
            "moly_world/src/shaders/weather_stock_post.wesl".to_owned(),
        ),
    );
}

/// 引擎原生后处理插件（由天气插件装入）：主世界烘表、渲染世界上传与节点。
pub struct StockPostPlugin;

impl Plugin for StockPostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StockPostState>()
            .add_plugins(ExtractResourcePlugin::<StockPostState>::default())
            .add_systems(Startup, load_shader)
            .add_systems(PostUpdate, bake_lut);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_stock_post)
            .add_systems(
                Render,
                (
                    write_stock_post.in_set(RenderSystems::Prepare),
                    prepare_stock_bloom_pyramid.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                stock_post
                    .after(tonemapping)
                    .before(weather_post)
                    .in_set(Core3dSystems::PostProcess),
            );
    }
}
