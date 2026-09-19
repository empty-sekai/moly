//! Source-authored phenomenon selection, environment transition and post stack.
//! All entries are loaded from the current index, ordered by their source IDs.
//! Native C-key and WeatherRequest share one transition owner.
//!
//! EnvironmentShaderView blends light/config over the source transition. At its
//! completion SetEnvironmentData commits sky-bottom color and emission type;
//! RefreshPostProcess replaces the VolumeProfile. Fog/post parameters do not
//! interpolate independently. Sky uses its separate two-ramp crossfade.
//!
//! ParticleBloom reads the MysekaiEffect/fixture-MRT target, not scene color.
//! Source pass state, scene-depth sharing and soft particles are handled by
//! fixture_emission/weather_depth. Fog remains a per-material global consumer.
//! Stock Bloom, SunFlare, runtime LUT grading, cloud/wind consumers and thunder
//! timeline remain explicit implementation work until their source paths close.
//! A render node or a nonempty target alone is not a pixel-equivalence claim.

use bevy::asset::uuid::Uuid;
use crate::weather_transition::{EnvironmentSelection, GlobalEffectIdentity, WeatherTransition, WeatherFxPrepared, WeatherEnvironmentUpdate};
use bevy::asset::{AssetPath, LoadState};
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::query::QueryItem;
use bevy::image::ImageLoaderSettings;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, RenderSubGraph, ViewNode,
    ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    BindingResource, Buffer, BufferInitDescriptor, BufferUsages, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, Operations, Origin3d, PipelineCache,
    PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use moly_assets::json::JsonAsset;
use moly_law::weather::{
    ColorAdjustmentsParams, DiffusionParams, FogVolumeState, PostProcessProfile, ScreenFlareParams,
};
use std::marker::PhantomData;

use crate::env::SiteEnv;
use crate::fixture_emission::ViewEmissionTarget;
use crate::fixture_material::EmissionAccount;
use crate::light;
use crate::site::SiteActive;
use crate::sky::{SkyDome, SkyGradient};

/// 天气链着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`（与天空壳同一
/// 形状——仓内没有默认资产源目录，字符串内嵌进资产表）。
const WEATHER_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x7c41_9e02_4b6f_8a55_d0e3_1f92_6ab4_c708),
    PhantomData,
);

/// 默认现象：与天空壳同一档（白天基准），画面基线一致。
const DEFAULT_PHENOMENON: &str = "001_sunny";

/// 交叉淡化时长（秒）。
const CROSS_FADE_SECONDS: f32 = crate::weather_transition::HOME_ENVIRONMENT_FADE_SECONDS;

/// 耀斑衰减指数的下限（与真源链同值；0 会让 powSafe 分支整支为 0）。
const FLARE_EXPONENT_FLOOR: f32 = 1e-3;

/// 金字塔纹理的格式：半浮点，与产品链的 HalfFloat 缓冲同一档。
const PYRAMID_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// uniform 块的 GPU 大小：10 个 vec4。
const WEATHER_UNIFORM_BYTES: usize = 192;

// ---- 主世界 ---------------------------------------------------------------

/// 一个现象档解出的全局量：交叉淡化在两份之间逐项混合。
struct ResolvedPhenomenon {
    timeline: Option<moly_law::weather::timeline::Timeline>,
    character_light_color: [f32; 4],
    character_skin_shade: [f32; 4],
    character_body_shade: [f32; 4],
    renderer_type: i32,
    home_light_dir: Vec3,
    sky_bottom_color: [f32; 4],
    /// 档位的清单 id（现象主表的 id 列）。真源问候链的 tweet 门按它相等
    /// 比较，不是按名字——名字只是资产目录名。
    id: i32,
    name: String,
    /// L1 光向公式按本档 config 的角度解出。
    light_dir: Vec3,
    light_color: [f32; 4],
    shade_color: [f32; 4],
    drop_shadow: [f32; 4],
    /// 雾九参（未折叠；折叠在混合后做，淡化期密度还乘门权重）。
    fog: FogVolumeState,
    fog_on: bool,
    diff: DiffusionParams,
    diff_on: bool,
    flare: ScreenFlareParams,
    flare_on: bool,
    /// 太阳光晕轴的采纳门（停用轴：不参与像素，计数进如实记账）。
    sun_on: bool,
    /// 粒子泛光轴的采纳门（本单已接：自发光半边进了泛光输入缓冲；
    /// 粒子特效半边等粒子域落地，其前恒空）。
    bloom_on: bool,
    /// 现象自发光类型（config 顶层 `emissionType`：1 = 日系、2 = 夜系）。
    /// 家具自发光 pass 的全局门读它——交叉淡化期按目标档取值（枚举档
    /// 不插值，与混合模式同一条裁决）。
    emission_type: i32,
    /// 泛光轴（已接）：预滤波 `_Params` 的律打包值
    /// `[scatter', clamp, g2l(threshold), knee]`。交叉淡化对已打包值线性
    /// 插值——g2l/scatter′ 是非线性打包，插值是近似；语料 15 档泛光参数
    /// 全同（threshold 0 / scatter 0.5 / tint 白 / clamp 65472），插值恒等。
    /// 未来档间出现差异时须把插值移到打包前的原始域。
    bloom_prefilter: [f32; 4],
    /// 律 uber 装箱的 `_Bloom_Params`：`[intensity, 归一线性 tint.rgb]`。
    /// 同上，tint 的亮度归一是非线性打包，插值是近似（语料恒等）。
    bloom_uber: [f32; 4],
    /// `_Bloom_Enable_States.x` 的档值（`brightEnable`，0/1）。淡化时与
    /// 轴门一起折成权重乘进强度（真源里 `w = enable.x · intensity` 是
    /// 一次乘法，稳态取值两边精确一致）。
    bloom_bright_gate: f32,
    /// `_BloomOverlayStrength` 的档值。
    bloom_overlay: f32,
    /// `fixedBufferHeight`：泛光金字塔的基高（基宽 = 基高 × 视口宽高比）。
    bloom_buffer_height: f32,
    /// 引擎原生调色的采纳值，**原始单位**（EV / 百分数 / 度 / HDR 颜色）。
    /// 淡化在原始单位上插值、之后才装箱——`2^EV` 与两个百分数都是非线性
    /// 打包，先打包再插值会插出档间不存在的曝光与对比。
    grade: ColorAdjustmentsParams,
    split_shadows: [f32; 4],
    split_highlights: [f32; 4],
    /// 调色轴的采纳门（源组件 `IsActive()` 的逐项转写）。
    grade_on: bool,
    /// 本档渐变条（天空壳的第二槽）。
    ramp: Handle<Image>,
    /// 站点覆写变体：真源对 config 与 postprocess 两件是**逐资产**的两级
    /// 查找——站点自己的环境包里 `ExistsAsset` 命中就整件用站点的，否则
    /// 用现象全局包的（两侧独立回退，不是成对门）。提取产物把带覆写的
    /// 站点列在清单 `overrides` 对象里（键 = 站点类型名；语料里只有
    /// first_floor 带，成对的 config+postprocess，14/15 档；无覆写的档
    /// ——如 999——此项为空，站内站外都用全局解）。渐变条覆写包不携带，
    /// 变体与全局共享同一份。运行时按 `SiteActive.site_type` 取用；切站
    /// 是整档 profile 即时换（真源 `RefreshPostProcess` 直写
    /// `Volume.sharedProfile`，CrossFade 机制只淡天空与特效，不淡后处理
    /// 参数）。
    sites: Vec<(String, Box<ResolvedPhenomenon>)>,
}

impl ResolvedPhenomenon {
    fn selection(&self, site: &SiteActive, site_generation: u64) -> EnvironmentSelection {
        EnvironmentSelection { name:self.name.clone(), site_id:site.site_id, environment_site:site.env_site.clone(), site_generation, global_effect:GlobalEffectIdentity {
            phenomenon_id:self.id, renderer_type:self.renderer_type,
        }}
    }
}

/// 现象清单装载请求；解析成功后即撤。
#[derive(Resource)]
struct IndexRequest(Handle<JsonAsset>);

/// 装载中的现象档：三路句柄都在手，等全部到达。id 用来在 `parse_index`
/// 排序，随后随档解出结果一起携带（问候链的门要读它）。
struct PendingPhenomenon {
    timeline: Option<Handle<JsonAsset>>,
    timeline_summary: Option<moly_assets::weather_index::TimelineDocument>,
    display: PhenomenonOption,
    sky_bottom_color: [f32; 4],
    id: i32,
    name: String,
    config: Handle<JsonAsset>,
    postprocess: Handle<JsonAsset>,
    ramp: Handle<Image>,
    /// 站点覆写的装载请求（键 = 站点类型名）。两侧各自可选：清单里某个
    /// 站点只列了一件时，另一件按真源的逐资产查找回退到全局——不是
    /// 数据损伤（语料里两件总是成对出现）。
    overrides: Vec<(String, OverridePair)>,
}

/// 一站覆写对：config 与 postprocess 各自可选。
struct OverridePair {
    config: Option<Handle<JsonAsset>>,
    postprocess: Option<Handle<JsonAsset>>,
}

/// 天气运行态：动态清单、当前档、等待资源的目的档与进行中的淡化。档案
/// 到达后由 `resolve_all` 填充；此前的帧全部早退。
#[derive(Resource, Default)]
struct WeatherRun {
    queued: Option<usize>,
    pending: Vec<PendingPhenomenon>,
    phenomena: Vec<ResolvedPhenomenon>,
    current: usize,
}

/// 当前现象档名——天气域对外的一面：音频（BGM/环境音按档换曲）与小地图
/// 天气钮（图标与文本）都读它，不进 `WeatherRun` 的私有下标。两侧同挂后
/// 随真实档走；`audio::install` 另有同型兜底 `init_resource`（幂等）。
#[derive(Resource)]
pub struct CurrentPhenomenon(pub String);

impl Default for CurrentPhenomenon {
    fn default() -> Self {
        Self(DEFAULT_PHENOMENON.to_string())
    }
}

/// 默认现象 id：真源现象主表的公开常量（值 1），与 [`DEFAULT_PHENOMENON]
/// （001_sunny）是同一档——清单里该档的 id 就是 1。
const DEFAULT_PHENOMENON_ID: i32 = 1;

/// 当前现象档的 id——问候链 tweet 门的比较对象：真源在选取时读「当前
/// 现象 id」，与条件行的 value1 相等才放行（名字不进这条门）。与档名
/// 同点写：装载落定取默认档 id，切档取目标档 id——真源在交叉淡化的
/// 视觉等待**之前**就写当前现象 id，这里的写点同为淡化起点。wasm 分支
/// 不装 `WeatherPlugin`，此资源常驻默认值（与档名的 wasm 故事一致）。
#[derive(Resource)]
pub struct CurrentPhenomenonId(pub i32);

impl Default for CurrentPhenomenonId {
    fn default() -> Self {
        Self(DEFAULT_PHENOMENON_ID)
    }
}

/// 一次性的切档请求：浏览器桥与宿主天气钮写它，天气链在下一帧核对档位
/// 是否真实存在，再走与 C 键**同一条**核对与淡化。未知 id 只打一行拒绝行
/// ——请求面绝不 panic、也绝不把淡化砍在半途。
#[derive(Message, Debug, Clone, Copy)]
pub struct WeatherRequest(pub i32);

/// Source identity, display metadata and resolved icon path for product UI.
#[derive(Clone, Debug)]
pub struct PhenomenonOption {
    pub id: i32,
    pub name: String,
    pub icon: Option<String>,
    pub icon_source: Option<moly_assets::weather_icons::WeatherIconArtifact>,
    pub metadata: Option<moly_assets::weather_index::PhenomenonMetadata>,
}

/// 现象档清单（id 升序，与现象运行态同序），由 `resolve_all` 在同一次落定
/// 里写满。浏览器与宿主 UI 只读它来画天气钮——档位表由运行时给出，界面因此
/// 点不到不存在的档；真送错了也只是被拒绝一次。
#[derive(Resource, Default)]
pub struct PhenomenonCatalogue(pub Vec<PhenomenonOption>);

/// 一帧解出的后处理轴值（淡化权重已折进强度），主世界每帧写、抽取到渲染
/// 世界。两条轴全关时节点整段旁路。
#[derive(Debug, Clone, Copy, Resource, ExtractResource)]
pub struct WeatherPostParams {
    pub diff_on: bool,
    pub diff_intensity: f32,
    pub diff_contrast: f32,
    pub diff_blend_mode: f32,
    pub diff_scatter: f32,
    pub diff_max_iterations: f32,
    pub diff_buffer_height: f32,
    pub flare_on: bool,
    pub flare_intensity: f32,
    /// 投影轴 (cos d, sin d)，d 是方向角的弧度值。
    pub flare_axis: [f32; 2],
    pub flare_color1: [f32; 4],
    pub flare_color2: [f32; 4],
    pub flare_offset1: f32,
    pub flare_offset2: f32,
    pub flare_exponent: f32,
    /// 现象自发光类型（枚举档，按目标档取值；全局槽里 f32 存整数值）。
    pub emission_type: i32,
    pub bloom_on: bool,
    /// 泛光预滤波参数（律打包 `[scatter', clamp, g2l(threshold), knee]`）。
    pub bloom_prefilter: [f32; 4],
    /// 泛光 uber 参数 `[强度, 归一 tint.rgb]`（强度已折门权重）。
    pub bloom_uber: [f32; 4],
    /// `_BloomOverlayStrength`。
    pub bloom_overlay: f32,
    /// 泛光金字塔基高（`fixedBufferHeight`）。
    pub bloom_buffer_height: f32,
    pub grade_on: bool,
    /// 调色采纳值，原始单位（装箱在写 uniform 时做）。
    pub grade: ColorAdjustmentsParams,
    pub split_shadows: [f32; 4],
    pub split_highlights: [f32; 4],
}

impl WeatherPostParams {
    /// 两轴全关的中性态。
    fn neutral() -> Self {
        WeatherPostParams {
            diff_on: false,
            diff_intensity: 0.0,
            diff_contrast: 1.0,
            diff_blend_mode: 6.0,
            diff_scatter: 0.0,
            diff_max_iterations: 5.0,
            diff_buffer_height: 540.0,
            flare_on: false,
            flare_intensity: 0.0,
            flare_axis: [1.0, 0.0],
            flare_color1: [1.0, 1.0, 1.0, 1.0],
            flare_color2: [0.0; 4],
            flare_offset1: 0.0,
            flare_offset2: 0.0,
            flare_exponent: 1.0,
            emission_type: 0,
            bloom_on: false,
            bloom_prefilter: [0.0; 4],
            bloom_uber: [0.0; 4],
            bloom_overlay: 0.0,
            bloom_buffer_height: 540.0,
            split_shadows: [0.5, 0.5, 0.5, 0.0],
            split_highlights: [0.5, 0.5, 0.5, 0.0],
            grade_on: false,
            grade: ColorAdjustmentsParams {
                post_exposure: 0.0,
                contrast: 0.0,
                color_filter: [1.0, 1.0, 1.0, 1.0],
                hue_shift: 0.0,
                saturation: 0.0,
            },
        }
    }
}

/// 交叉淡化（或稳态）解出的一帧输出：写全局量桥 + 轴值 + 天空槽。
struct Blended {
    sky_bottom_color: [f32; 4],
    light_dir: Vec3,
    light_color: [f32; 4],
    shade_color: [f32; 4],
    drop_shadow: [f32; 4],
    fog: moly_law::weather::FogGlobals,
    post: WeatherPostParams,
}

fn lerpf(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 开关门折成 0/1 权重（产品链的混合对 active 标志就是这么做的）。
fn gate(on: bool) -> f32 {
    if on { 1.0 } else { 0.0 }
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        lerpf(a[0], b[0], t),
        lerpf(a[1], b[1], t),
        lerpf(a[2], b[2], t),
        lerpf(a[3], b[3], t),
    ]
}

/// 两档按进度混合：光九项线性（光向球面插值）、雾九参线性后折叠（门折成
/// 权重缩总密度）、扩散/耀斑数值线性（枚举档按目标取值，门折成权重缩强度）。
/// t=1 时精确等于 b 档单独生效。
/// EnvironmentShaderView blends the light/config. Volume.sharedProfile,
/// sky-bottom color and emission type are committed only after that transition.
/// They are not interpolated, even when their stored values are numeric.
fn committed<'a, T>(previous: &'a T, next: &'a T, progress: f32) -> &'a T {
    if progress < 1.0 { previous } else { next }
}
fn blend_at_site(a: &ResolvedPhenomenon, b: &ResolvedPhenomenon, p: &ResolvedPhenomenon, t: f32, source_home: bool, destination_home: bool) -> Blended {
    let direction_a = if source_home { a.home_light_dir } else { a.light_dir };
    let direction_b = if destination_home { b.home_light_dir } else { b.light_dir };
    let mut fog = p.fog;
    fog.enabled = p.fog_on;
    let (sin_d, cos_d) = p.flare.direction.to_radians().sin_cos();
    Blended {
        sky_bottom_color: p.sky_bottom_color,
        light_dir: light::slerp_direction(direction_a, direction_b, t),
        light_color: lerp4(a.light_color, b.light_color, t),
        shade_color: lerp4(a.shade_color, b.shade_color, t),
        drop_shadow: lerp4(a.drop_shadow, b.drop_shadow, t),
        fog: fog.globals(false),
        post: WeatherPostParams {
            diff_on: p.diff_on,
            diff_intensity: p.diff.intensity,
            diff_contrast: p.diff.contrast,
            diff_blend_mode: p.diff.blend_mode,
            diff_scatter: p.diff.scatter,
            diff_max_iterations: p.diff.max_iterations,
            diff_buffer_height: p.diff.buffer_height,
            flare_on: p.flare_on,
            flare_intensity: p.flare.intensity,
            flare_axis: [cos_d, sin_d],
            flare_color1: p.flare.color1, flare_color2: p.flare.color2,
            flare_offset1: p.flare.offset1, flare_offset2: p.flare.offset2,
            flare_exponent: p.flare.exponent.max(FLARE_EXPONENT_FLOOR),
            emission_type: p.emission_type,
            bloom_on: p.bloom_on,
            bloom_prefilter: p.bloom_prefilter,
            bloom_uber: [p.bloom_uber[0] * gate(p.bloom_on) * p.bloom_bright_gate,
                p.bloom_uber[1], p.bloom_uber[2], p.bloom_uber[3]],
            bloom_overlay: p.bloom_overlay,
            bloom_buffer_height: p.bloom_buffer_height,
            grade_on: p.grade_on,
            grade: p.grade,
            split_shadows: p.split_shadows,
            split_highlights: p.split_highlights,
        },
    }
}


/// Startup：着色程序入表、空运行态落位、请求现象清单。
fn load(mut commands: Commands, mut shaders: ResMut<Assets<Shader>>, server: Res<AssetServer>) {
    // 返回的句柄就是钉死的 WEATHER_SHADER，丢弃以免 must_use 告警。
    let _ = shaders.insert(
        WEATHER_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/weather_post.wgsl"),
            "moly_game/src/shaders/weather_post.wgsl".to_owned(),
        ),
    );
    commands.insert_resource(WeatherRun::default());
    commands.insert_resource(WeatherPostParams::neutral());
    commands.insert_resource(IndexRequest(server.load::<JsonAsset>(AssetPath::from(
        "moly://phenomena/index.json".to_owned(),
    ))));
}

/// Update：现象清单到达 → 按 id 排出 15 档，逐档请求三路产物。
fn parse_index(
    mut commands: Commands,
    server: Res<AssetServer>,
    index: Res<Assets<JsonAsset>>,
    request: Option<Res<IndexRequest>>,
    mut run: ResMut<WeatherRun>,
) {
    let Some(request) = request else { return; };
    if let LoadState::Failed(err) = server.load_state(&request.0) {
        panic!("phenomenon index load failed: {err:?}");
    }
    let Some(doc) = index.get(&request.0) else { return; };
    let source = moly_assets::weather_index::PhenomenonIndex::from_bytes(doc.0.as_bytes())
        .unwrap_or_else(|err| panic!("invalid phenomenon index: {err}"));
    if source.phenomena.values().any(|entry| entry.timeline.is_some()) {
        source.environment_controller.as_ref()
            .expect("source timeline requires the controller-owned director descriptor")
            .validate().unwrap_or_else(|e| panic!("source environment director: {e}"));
    }
    let mut entries: Vec<_> = source.phenomena.into_iter().collect();
    entries.sort_by_key(|(_, entry)| entry.id);
    let load = |file: &str| server.load::<JsonAsset>(
        AssetPath::from(format!("moly://phenomena/{file}")));
    let mut pending = Vec::with_capacity(entries.len());
    for (name, entry) in entries {
        let ramp = server.load_with_settings::<Image, _>(
            AssetPath::from(format!("moly://phenomena/{}", entry.ramp.file)),
            |settings: &mut ImageLoaderSettings| settings.is_srgb = false,
        );
        let overrides = entry.overrides.into_iter().map(|(site, files)| {
            (site, OverridePair {
                config: files.config.as_deref().map(&load),
                postprocess: files.postprocess.as_deref().map(&load),
            })
        }).collect();
        pending.push(PendingPhenomenon {
            timeline: entry.timeline.as_ref().map(|timeline| load(&timeline.file)),
            timeline_summary: entry.timeline,
            display: PhenomenonOption {
                id: entry.id, name: name.clone(), icon: entry.icon,
                icon_source: entry.master.as_ref().and_then(|master| master.icon_assetbundle_name.as_ref())
                    .and_then(|key| source.icon_sources.get(key)).map(|receipt| receipt.artifact.clone()),
                metadata: entry.master,
            },
            id: entry.id, name, config: load(&entry.config),
            postprocess: load(&entry.postprocess), ramp,
            sky_bottom_color: entry.ramp.sky_bottom_color, overrides,
        });
    }
    info!("phenomenon index: {} source entries, sorted by id", pending.len());
    run.pending = pending;
    commands.remove_resource::<IndexRequest>();
}

/// Update：15 档三路产物全部到达 → 解出全部档位、当前档落地、如实记账。
fn resolve_all(
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut run: ResMut<WeatherRun>,
    mut phenomenon: ResMut<CurrentPhenomenon>,
    mut phenomenon_id: ResMut<CurrentPhenomenonId>,
    mut catalogue: ResMut<PhenomenonCatalogue>,
) {
    if run.pending.is_empty() || !run.phenomena.is_empty() {
        return;
    }
    for p in &run.pending {
        if let Some(handle) = &p.timeline {
            if let LoadState::Failed(err) = server.load_state(handle) {
                panic!("source weather timeline {} failed to load: {err:?}", p.name);
            }
        }
        for handle in [&p.config, &p.postprocess] {
            if let LoadState::Failed(err) = server.load_state(handle) {
                panic!("现象 {} 的档案装载失败：{err:?}", p.name);
            }
        }
        for (site, pair) in &p.overrides {
            for (kind, handle) in [
                ("config", pair.config.as_ref()),
                ("postprocess", pair.postprocess.as_ref()),
            ] {
                if let Some(handle) = handle {
                    if let LoadState::Failed(err) = server.load_state(handle) {
                        panic!("现象 {} 站点 {site} 的 {kind} 装载失败：{err:?}", p.name);
                    }
                }
            }
        }
        if let LoadState::Failed(err) = server.load_state(&p.ramp) {
            panic!("现象 {} 的渐变条装载失败：{err:?}", p.name);
        }
    }
    let all_loaded = run.pending.iter().all(|p| {
        server.load_state(&p.config).is_loaded()
            && p.timeline.as_ref().is_none_or(|h| server.load_state(h).is_loaded())
            && server.load_state(&p.postprocess).is_loaded()
            && server.load_state(&p.ramp).is_loaded()
            && p.overrides.iter().all(|(_, pair)| {
                pair.config
                    .as_ref()
                    .map(|h| server.load_state(h).is_loaded())
                    .unwrap_or(true)
                    && pair
                        .postprocess
                        .as_ref()
                        .map(|h| server.load_state(h).is_loaded())
                        .unwrap_or(true)
            })
    });
    if !all_loaded {
        return;
    }
    let mut resolved: Vec<ResolvedPhenomenon> = Vec::with_capacity(run.pending.len());
    let mut display_options = Vec::with_capacity(run.pending.len());
    for p in run.pending.drain(..) {
        let Some(config_doc) = json.get(&p.config) else {
            panic!("现象 {} 的 config.json 不在资产表里", p.name);
        };
        let Some(post_doc) = json.get(&p.postprocess) else {
            panic!("现象 {} 的 postprocess.json 不在资产表里", p.name);
        };
        // 站点覆写变体：逐资产回退（该件缺席时用全局档的文档），解出与
        // 全局同形的整档。渐变条共享全局档的句柄（覆写包不携带渐变条）。
        let mut sites = Vec::new();
        for (site, pair) in &p.overrides {
            // 清单列了该件（且装载门已过）才取覆写文档；没列 ⇒ None ⇒
            // 解析时回退全局档。装载门过后的 get 缺席是数据损伤，响亮。
            let doc_of =
                |handle: &Option<Handle<JsonAsset>>, key: &str| -> Option<&str> {
                    handle.as_ref().map(|h| {
                        json.get(h)
                            .unwrap_or_else(|| {
                                panic!("现象 {} 站点 {site} 的 {key} 不在资产表里", p.name)
                            })
                            .0
                            .as_str()
                    })
                };
            let config_override_doc = doc_of(&pair.config, "config");
            let post_override_doc = doc_of(&pair.postprocess, "postprocess");
            let variant = resolve_phenomenon(
                p.id,
                &p.name,
                config_override_doc.unwrap_or(config_doc.0.as_str()),
                post_override_doc.unwrap_or(post_doc.0.as_str()),
                p.ramp.clone(),
                p.sky_bottom_color,
            );
            // 覆写档的逐轴门逐站报账：全局态与站点态的差异（扩散/雾这类
            // 在室内整族关掉的轴）不落一行日志就看不见。
            info!(
                "现象 {} 站点 {site} 覆写档：调色门 {}、屏幕耀斑门 {}、泛光门 {}（强度 {:.3}）、扩散门 {}（强度 {:.3}）、雾门 {}（密度 {:.3}）、自发光类型 {}",
                p.name,
                u8::from(variant.grade_on),
                u8::from(variant.flare_on),
                u8::from(variant.bloom_on),
                variant.bloom_uber[0],
                u8::from(variant.diff_on),
                variant.diff.intensity,
                u8::from(variant.fog_on),
                variant.fog.density,
                variant.emission_type,
            );
            sites.push((site.clone(), Box::new(variant)));
        }
        let mut phenomenon =
            resolve_phenomenon(p.id, &p.name, &config_doc.0, &post_doc.0, p.ramp, p.sky_bottom_color);
        phenomenon.sites = sites;
        phenomenon.timeline = p.timeline.as_ref().map(|handle| {
            let document = json.get(handle).expect("loaded source timeline missing from asset table");
            let parsed = moly_assets::weather_timeline::WeatherTimeline::from_bytes(document.0.as_bytes())
                .unwrap_or_else(|e| panic!("weather {} timeline: {e}", p.name));
            let summary = p.timeline_summary.as_ref().expect("timeline summary disappeared");
            assert_eq!(parsed.tracks.len(), summary.tracks as usize, "source timeline track summary drift");
            assert_eq!(parsed.tracks.iter().map(|t|t.clips.len()).sum::<usize>(), summary.clips as usize, "source timeline clip summary drift");
            assert_eq!(parsed.duration, summary.duration, "source timeline duration summary drift");
            parsed.compile().unwrap_or_else(|e| panic!("weather {} timeline compile: {e}", p.name))
        });
        resolved.push(phenomenon);
        display_options.push(p.display);
    }
    run.current = resolved
        .iter()
        .position(|r| r.name == DEFAULT_PHENOMENON)
        .unwrap_or_else(|| panic!("现象清单里没有默认现象 {DEFAULT_PHENOMENON}"));
    // 对外面落一份档名（音频等消费者不等私有下标）；id 与名同点写——
    // 问候链的门按 id 比较，档名对不上门。
    phenomenon.0 = resolved[run.current].name.clone();
    phenomenon_id.0 = resolved[run.current].id;
    // 对外档位表与当前档同点落定：浏览器与宿主 UI 读到的清单从这一刻起就
    // 是完整的，不存在「先看到空表、再看到档位」的中间态。顺序按 id 排，
    // 与真源现象主表一致。
    *catalogue = PhenomenonCatalogue(display_options);
    let sun_on = resolved.iter().filter(|r| r.sun_on).count();
    let bloom_on = resolved.iter().filter(|r| r.bloom_on).count();
    let grade_on = resolved.iter().filter(|r| r.grade_on).count();
    let names = resolved
        .iter()
        .map(|r| r.name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let current = &resolved[run.current];
    let total = resolved.len();
    info!(
        "天气系统就绪：{total} 档 [{names}]，当前 {}（id {}；光向 {:?}、雾门 {} 密度 {:.3}、扩散门 {} 强度 {:.3} 模式 {}、屏幕耀斑门 {} 强度 {:.3}、泛光门 {} 强度 {:.3}、自发光类型 {}）",
        current.name,
        current.id,
        current.light_dir,
        current.fog_on,
        current.fog.density,
        current.diff_on,
        current.diff.intensity,
        current.diff.blend_mode,
        current.flare_on,
        current.flare.intensity,
        current.bloom_on,
        current.bloom_uber[0],
        current.emission_type,
    );
    // 调色轴逐档报（这一族此前整族不进像素，是「天气只有一个颜色」的
    // 字面来源）：门 + 装箱后的四个量，档间不同才说明它真在出力。
    let grades = resolved
        .iter()
        .map(|r| {
            let g = r.grade.pack();
            format!(
                "{}={}/{:.4},{:.4},{:.4},{:.4},{:.4}",
                r.name,
                u8::from(r.grade_on),
                g.post_exposure_linear,
                g.hue_sat_con[0],
                g.hue_sat_con[1],
                g.hue_sat_con[2],
                g.color_filter_linear[0],
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    info!(
        "调色轴（引擎原生 ColorAdjustments，挂在 Mysekai 后处理的上游）：{grade_on}/{total} 档非恒等；档=门/曝光倍数,色相偏移,饱和系数,对比系数,滤色.r ⇒ {grades}"
    );
    // 一次性 oracle 转储：把部署链的 color_grade 在 8 个固定采样输入上
    // 的 15 档输出打成一行 ASCII（跨实现逐值比对用的数；稳定键名）。
    {
        let samples: [[f32; 3]; 8] = [
            [0.05, 0.30, 0.90],
            [0.50, 0.50, 0.50],
            [0.95, 0.05, 0.20],
            [0.30, 0.60, 0.10],
            [0.80, 0.70, 0.40],
            [0.12, 0.45, 0.67],
            [0.99, 0.99, 0.99],
            [0.02, 0.02, 0.02],
        ];
        let mut line = String::from("COLOR_GRADE_ORACLE");
        for r in &resolved {
            line.push_str(&format!(" |{}", r.name));
            for s in samples {
                let o = rust_color_grade(s, r.grade_on, &r.grade);
                line.push_str(&format!(";{:.6},{:.6},{:.6}", o[0], o[1], o[2]));
            }
        }
        info!("{line}");
    }
    warn!(
        "天气系统如实记账（不接的支路与原因）：① 太阳光晕——方向向量是平行光方向经相机逆变换后的 xy，z 的符号约定在真源读不出来，停用不近似（{sun_on}/{total} 档开这一支，参数照读不参与像素）；② 粒子泛光的粒子特效半边——那条输入缓冲在源侧是 `_EffectSourceTex`（只装特效与第二颜色目标，不是整幅画面），粒子域落地前该半边恒空、贡献恒零；自发光半边（家具第二颜色目标）已接进泛光金字塔（{bloom_on}/{total} 档开这一支）；③ 引擎原生 SplitToning——只有一档带且组件级 active 为真，那一档的调色比本模块多两次 gamma 域 soft light，未接；同族的 WhiteBalance 与原生 Bloom 各只有一档带且 active 为假 ⇒ 落构造默认 ⇒ 恒等，不是缺口；④ 打雷档的时间轴（天空附加色闪光）——独立时间轴域，本模块不驱动，附加强度底值恒 0；⑤ 调色走的是引擎「烘 LUT + 查表」两段链，本模块按同一串式子逐像素直算（不烘 32³ 表），少了表的三线性插值误差；LDR 档还是 HDR 档由渲染管线资产的序列化字段决定，反编译里读不到，这里按 LDR 实现（本链帧缓冲是 8bit，输入已在 [0,1]）。"
    );
    run.phenomena = resolved;
}

/// 一档的 config + postprocess + 渐变条 → 解出结果。字段缺失或解析失败都是
/// 数据损伤，响亮 panic（不造默认值顶替）。
fn resolve_phenomenon(
    id: i32,
    name: &str,
    config_doc: &str,
    post_doc: &str,
    ramp: Handle<Image>,
    sky_bottom_color: [f32; 4],
) -> ResolvedPhenomenon {
    let config: moly_assets::weather_index::EnvironmentSelection =
        serde_json::from_str(config_doc)
            .unwrap_or_else(|err| panic!("phenomenon {name}: invalid environment selection: {err}"));
    let [xz, y] = config.light.angles(false);
    let light_dir = light::dir_toward_light(xz, y);
    let [xz, y] = config.light.angles(true);
    let home_light_dir = light::dir_toward_light(xz, y);
    let emission_type = config.emission_type;
    let profile = PostProcessProfile::from_bytes(post_doc.as_bytes())
        .unwrap_or_else(|err| panic!("现象 {name} 的 postprocess.json 解析失败：{err}"));
    let (split_shadows, split_highlights) = profile.resolve_split_toning().unwrap_or_else(|err| panic!("SplitToning: {err}"));
    let post = profile
        .resolve()
        .unwrap_or_else(|err| panic!("现象 {name} 的后处理档案字段损伤：{err}"));
    // 泛光轴装箱：预滤波 `_Params` 与 uber `_Bloom_Params` 都从律取——律里
    // 是照源 pass 代码自己的 SetVector 式子装箱的，这里不二次推导。
    let bloom_prefilter = post.bloom_prefilter_params();
    let uber = post.uber_post_params();
    let bloom_uber = uber.bloom_params;
    let bloom_bright_gate = uber.bloom_enable_states[0];
    let bloom_overlay = uber.scalars[3];
    let bloom_buffer_height = post.bloom_lq.params.fixed_buffer_height;
    ResolvedPhenomenon {
        timeline: None,
        character_light_color: config.light.character_directional_light_color,
        character_skin_shade: config.light.character_shade_skin_color,
        character_body_shade: config.light.character_body_shade_color,
        id,
        name: name.to_owned(),
        light_dir,
        home_light_dir, sky_bottom_color,
        renderer_type: config.renderer_type,
        light_color: config.light.phenomena_directional_light_color,
        shade_color: config.light.phenomena_shade_color,
        drop_shadow: config.light.drop_shadow_color1,
        fog: post.fog.params,
        fog_on: post.fog.enabled,
        diff: post.sky_diffusion.params,
        diff_on: post.sky_diffusion.enabled,
        flare: post.screen_flarepara.params,
        flare_on: post.screen_flarepara.enabled,
        sun_on: post.sun_flarepara.enabled,
        bloom_on: post.bloom_lq.enabled,
        emission_type,
        bloom_prefilter,
        bloom_uber,
        bloom_bright_gate,
        bloom_overlay,
        bloom_buffer_height,
        split_shadows,
        split_highlights,
        grade: post.color_grading.params,
        grade_on: post.color_grading.enabled,
        ramp,
        sites: Vec::new(),
    }
}

/// 一档在指定站点的生效解：站点覆写表里列了该站就取覆写变体，否则取
/// 全局解（真源逐资产两级查找的取用侧）。切站即时换——覆写变体是整档
/// 解好的值，取用本身没有过渡（真源 `RefreshPostProcess` 直写
/// `sharedProfile`）。
fn effective<'a>(r: &'a ResolvedPhenomenon, site: &str) -> &'a ResolvedPhenomenon {
    r.sites
        .iter()
        .find(|(name, _)| name == site)
        .map(|(_, variant)| &**variant)
        .unwrap_or(r)
}

/// Update：淡化推进 + 每帧写出。稳态（无淡化）也每帧写——与律「每帧从当前
/// 解析状态重取」同节奏。
fn advance(
    time: Res<Time>,
    server: Res<AssetServer>,
    mut run: ResMut<WeatherRun>,
    site: Option<Res<SiteActive>>,
    prepared: Option<Res<WeatherFxPrepared>>,
    mut phase: ResMut<WeatherTransition>,
    mut current: ResMut<CurrentPhenomenon>,
    mut current_id: ResMut<CurrentPhenomenonId>,
) {
    if run.phenomena.is_empty() { return; }
    let Some(site) = site.as_deref() else { return; };
    let to = run.queued.unwrap_or(run.current);
    let destination = effective(&run.phenomena[to], &site.env_site).selection(site, phase.site_generation);
    phase.request(destination.clone());
    // SiteEnvironmentManager writes its public ID before awaiting the view loader.
    // Audio and post-profile consumers use the separate completed selection.
    if current.0 != destination.name { current.0 = destination.name.clone(); }
    current_id.0 = destination.global_effect.phenomenon_id;
    let ramp_ready = server.load_state(&run.phenomena[to].ramp).is_loaded();
    let started = ramp_ready && prepared.as_deref().is_some_and(|ready| phase.start_prepared(ready));
    if started { run.current = to; run.queued = None; }
    if !started { phase.advance(time.delta_secs()); }
}

/// Environment audio changes only after the real global FX commit, not at request.
#[derive(Resource)]
pub(crate) struct CommittedPhenomenon(pub String);
impl Default for CommittedPhenomenon {
    fn default() -> Self { Self(DEFAULT_PHENOMENON.to_string()) }
}

pub(crate) fn commit_environment(
    mut phase: ResMut<WeatherTransition>,
    effects: Option<Res<crate::weather_transition::WeatherGlobalFxCommitted>>,
    mut committed_name: ResMut<CommittedPhenomenon>,
) {
    if let Some(effects) = effects.as_deref() {
        if phase.commit(effects) {
            committed_name.0 = phase.committed.as_ref().unwrap().name.clone();
        }
    }
}

/// The source controller's GameTime/Loop director is validated at load time.
/// Cancellation leaves the last committed playable running; a detached site
/// clears it, and only a completed environment commit starts a fresh clock.
#[derive(Resource, Default)]
pub(crate) struct WeatherTimelineState {
    serial: Option<u64>,
    generation: u64,
    pub elapsed: f64,
    pub local_time: f64,
    pub duration: Option<f64>,
    pub values: moly_law::weather::timeline::Values,
}

fn evaluate_timeline(
    time: Res<Time>, run: Res<WeatherRun>, phase: Res<WeatherTransition>,
    mut state: ResMut<WeatherTimelineState>,
) {
    if state.generation != phase.site_generation {
        *state = WeatherTimelineState { generation: phase.site_generation, ..default() };
    }
    let Some(selection) = phase.committed.as_ref()
        .filter(|selection| selection.site_generation == phase.site_generation) else {
        state.values = default(); state.duration = None; return;
    };
    let Some(row) = run.phenomena.iter().find(|p|p.id==selection.global_effect.phenomenon_id) else { return; };
    if state.serial != phase.committed_serial() {
        state.elapsed = 0.0;
        state.serial = phase.committed_serial();
    } else {
        state.elapsed += time.delta_secs_f64();
    }
    if let Some(timeline) = &row.timeline {
        state.duration = Some(timeline.duration);
        state.local_time = state.elapsed.rem_euclid(timeline.duration);
        state.values = timeline.evaluate(state.local_time).expect("validated source environment timeline failed");
    } else {
        state.duration = None; state.local_time = 0.0; state.values = default();
    }
}

/// Consume the frozen source and destination site variants. A new SiteActive must
/// never rewrite both halves of an in-flight crossfade or a pending destination.
fn write_environment(
    run: Res<WeatherRun>,
    phase: Res<WeatherTransition>,
    mut env: ResMut<SiteEnv>,
    mut post: ResMut<WeatherPostParams>,
    timeline: Res<WeatherTimelineState>,
    mut character: Option<ResMut<crate::character_material::CharacterEnv>>,
    mut avatar: Option<ResMut<crate::avatar_material::AvatarEnv>>,
    sky_materials: Query<&MeshMaterial3d<SkyGradient>, With<SkyDome>>,
    mut materials: ResMut<Assets<SkyGradient>>,
) {
    let Some(next) = phase.loaded.as_ref() else { return; };
    let previous = phase.source.as_ref().unwrap_or(next);
    let profile = phase.committed.as_ref().unwrap_or(previous);
    let resolve = |selection: &EnvironmentSelection| {
        let row = run.phenomena.iter().find(|row| row.id == selection.global_effect.phenomenon_id)
            .expect("loaded weather selection missing from its source catalogue");
        effective(row, &selection.environment_site)
    };
    let (a,b,p) = (resolve(previous),resolve(next),resolve(profile));
    let blended = blend_at_site(a,b,p,phase.progress,previous.environment_site=="home",next.environment_site=="home");
    env.globals.light_vector = blended.light_dir.to_array();
    let values = timeline.values;
    let add = moly_law::weather::timeline::additive_light;
    env.globals.phenomena_directional_light_color = add(blended.light_color, values.light_color, values.light_intensity);
    let character_light = add(lerp4(a.character_light_color, b.character_light_color, phase.progress),
        values.light_color, values.light_intensity);
    if let Some(character) = character.as_deref_mut() {
        character.globals.light_vector = blended.light_dir.to_array();
        character.globals.light_color = character_light;
        character.globals.skin_shade_color = lerp4(a.character_skin_shade, b.character_skin_shade, phase.progress);
        character.globals.body_shade_color = lerp4(a.character_body_shade, b.character_body_shade, phase.progress);
        character.globals.fog_params = blended.fog.fog_params;
        character.globals.fog_near_color = blended.fog.fog_near_color;
        character.globals.fog_far_color = blended.fog.fog_far_color;
        character.fog_ready = true;
    }
    if let Some(avatar) = avatar.as_deref_mut() { avatar.light_color = character_light; }
    env.globals.phenomena_shade_color = blended.shade_color;
    env.drop_shadow_color = blended.drop_shadow;
    env.sky_bottom_color = blended.sky_bottom_color;
    env.globals.fog_params = blended.fog.fog_params;
    env.globals.fog_near_color = blended.fog.fog_near_color;
    env.globals.fog_far_color = blended.fog.fog_far_color;
    env.emission_type = blended.post.emission_type as f32;
    *post = blended.post;
    if let Ok(handle) = sky_materials.single() {
        if let Some(material) = materials.get_mut(&handle.0) {
            material.set_ramps(a.ramp.clone(),b.ramp.clone(),phase.progress);
            material.set_timeline_additive(values.sky_color, values.sky_intensity);
        }
    }
}

/// Update：两个入口共用这一条切换链——场景输入可用时 C 键循环推进到下一
/// 档，以及浏览器桥/宿主 UI 请求的**确定性档位**。两者都在这里核对档位、
/// 开一段交叉淡化（淡化在下一帧起推进）。
///
/// 另有**验证用**的自动切换：环境变量 `MOLY_WEATHER_AUTOSWITCH_SECS` 给了
/// 秒数就按该周期自动切——验收要在无人按键的跑法里从日志推导「轴 resource
/// 值随切换变」，真人按键路径（C 键）不受影响；变量不给时这条路径完全不
/// 生效。淡化进行中不叠新切换（0.25s 的窗口，叠了会砍在半途）。
///
/// 按键仍按场景输入门（设置面板/内容库挡世界输入时不该被键盘改天气），而
/// 请求来自宿主界面上的显式操作，不受那条门限制——否则正在播一段对话时
/// 天气钮会变成死键。未知档位只打一行拒绝行：请求面不能 panic。
fn switch_phenomenon(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut run: ResMut<WeatherRun>,
    site: Option<Res<SiteActive>>,
    mut requests: MessageReader<WeatherRequest>,
    panel: Res<crate::game_settings::SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
    // 自发光账目（换装完成的收账件；Option——换装未完成时缺件，此切换
    // 无行可打）。
    emission: Option<Res<EmissionAccount>>,
    // (周期, 已计秒数)，首次调用时按环境变量定型。
    mut auto: Local<Option<(f32, f32)>>,
) {
    if run.phenomena.is_empty() {
        return;
    }
    if auto.is_none() {
        *auto = std::env::var("MOLY_WEATHER_AUTOSWITCH_SECS")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
            .filter(|period| *period > 0.0)
            .map(|period| (period, 0.0));
    }
    let autoswitch = match auto.as_mut() {
            Some((period, elapsed)) => {
                *elapsed += time.delta_secs();
                if *elapsed >= *period {
                    *elapsed = 0.0;
                    true
                } else {
                    false
                }
            }
            None => false,
        };
    // 显式请求比同帧的按键更具体：请求给出的是**目标档**，按键只是「下一
    // 档」。两者都过同一段核对，因此两条入口的淡化与记账逐式同形。
    let requested = requests.read().last().map(|request| request.0);
    let pressed = crate::game_settings::scene_input_enabled(panel, library)
        && (keys.just_pressed(KeyCode::KeyC) || autoswitch);
    let to = match requested {
        Some(id) => match run.phenomena.iter().position(|entry| entry.id == id) {
            Some(index) => index,
            None => {
                warn!("天气请求：清单里没有 id {id} 的档位，忽略这一次请求");
                return;
            }
        },
        None if pressed && run.phenomena.len() > 1 => {
            (run.current + 1) % run.phenomena.len()
        }
        None => return,
    };
    if run.queued == Some(to) { return; }
    let from = run.current;
    // 切换行的两侧取当前站点的生效变体：覆写站上切换的数值面就是覆写档
    // 的（淡化混合与收口读同一对变体，见 `advance`）。
    let environment_site = site
        .as_deref()
        .map(|s| s.env_site.as_str())
        .unwrap_or("");
    let a = effective(&run.phenomena[from], environment_site);
    let b = effective(&run.phenomena[to], environment_site);
    info!(
        "天气切换：{}(id {}) → {}(id {})（交叉淡化 {}s）：扩散强度 {:.3}→{:.3}、散射 {:.3}→{:.3}、混合模式 {}→{}、雾密度 {:.3}→{:.3}、屏幕耀斑强度 {:.3}→{:.3}、泛光强度 {:.3}→{:.3}、泛光预滤波（scatter,clamp,thr_g2l,knee）{:?}→{:?}、自发光类型 {}→{}",
        a.name,
        a.id,
        b.name,
        b.id,
        CROSS_FADE_SECONDS,
        a.diff.intensity,
        b.diff.intensity,
        a.diff.scatter,
        b.diff.scatter,
        a.diff.blend_mode,
        b.diff.blend_mode,
        a.fog.density,
        b.fog.density,
        a.flare.intensity,
        b.flare.intensity,
        a.bloom_uber[0],
        b.bloom_uber[0],
        a.bloom_prefilter,
        b.bloom_prefilter,
        a.emission_type,
        b.emission_type,
    );
    // 逐材质自发光账目行：按**目标档**的现象类型推门值与贡献（门 × 遮罩
    // 均值）——「切到夜/傍晚档后自发光贡献可从日志推导」的数据面；着色
    // 器侧另有一份门（同一条开关链），两处同一式。
    if let Some(account) = emission.as_deref() {
        for line in account.contribution_lines(b.emission_type) {
            info!("自发光账目（目标 {}）{line}", b.name);
        }
    }
    // The request may supersede an in-flight source task; the new load has its own identity.
    run.queued = Some(to);
}

// ---- 渲染侧 ---------------------------------------------------------------

/// uniform 块的 GPU 形：与 `weather_post.wgsl` 的 `WeatherPostUniform` 逐 lane
/// 对齐（8 个 vec4，128 字节）。
#[derive(Debug, Clone, Copy, ShaderType)]
struct WeatherPostUniform {
    /// (门, 强度, 对比, 混合模式)
    diff_a: [f32; 4],
    /// (散射权重, 0, 0, 0)。上采样 pass 不从这块按偏移取——uniform 偏移
    /// 须对齐 32 字节（min_uniform_buffer_offset_alignment），16 会拒；
    /// 散射权重单独一块 buffer（见 `WeatherPostGpu::scatter`）。
    diff_b: [f32; 4],
    /// (轴.x, 轴.y, 强度, 指数)
    flare_axis: [f32; 4],
    flare_c1: [f32; 4],
    flare_c2: [f32; 4],
    /// (偏移1, 偏移2, 门, 0)
    flare_off: [f32; 4],
    /// 泛光：(强度, overlay 强度, 0, 0)。强度是 blend 侧已折轴门与亮门
    /// 的值（`WeatherPostParams::bloom_uber` 的 x 槽）。
    bloom_a: [f32; 4],
    /// 泛光：(tint.r, tint.g, tint.b, 0)。
    bloom_b: [f32; 4],
    /// 调色：(曝光线性倍数, 色相偏移, 饱和系数, 对比系数)。
    grade_a: [f32; 4],
    /// 调色：(滤色.r, 滤色.g, 滤色.b, 门)。
    grade_b: [f32; 4],
    split_shadows: [f32; 4],
    split_highlights: [f32; 4],
}

/// 散射权重的折算归律（`moly_law::weather::scatter_prime`）——同一个
/// `scatter * 0.9 + 0.05` 此前在律与本模块各有一份，律那份还漏了折算。
/// 折算要在淡化**之后**做：淡化插的是档案原值。
fn scatter_weight(scatter: f32) -> f32 {
    moly_law::weather::scatter_prime(scatter)
}

/// 部署链 `color_grade` 的 Rust 转写，只给一次性 oracle 转储用（运行时
/// 逐像素的那份在 WGSL 里，与这里的式子逐项同形）。
fn rust_color_grade(c_in: [f32; 3], gate_on: bool, g: &ColorAdjustmentsParams) -> [f32; 3] {
    if !gate_on {
        return c_in;
    }
    let pack = g.pack();
    let post = pack.post_exposure_linear;
    let (hs, ss, cs) = (pack.hue_sat_con[0], pack.hue_sat_con[1], pack.hue_sat_con[2]);
    let fl = pack.color_filter_linear;
    let sat3 = |v: [f32; 3]| [v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0), v[2].clamp(0.0, 1.0)];
    let l2l = |x: f32| 0.244161 * (5.555556_f32 * x + 0.047996).max(0.0).log10() + 0.386036;
    let l2lin = |x: f32| (10.0f32.powf((x - 0.386036) / 0.244161) - 0.047996) / 5.555556;
    let rgb_to_hsv = |c: [f32; 3]| {
        let (r, gg, b) = (c[0], c[1], c[2]);
        let p = if b < gg { (gg, b, 0.0, -1.0 / 3.0) } else { (b, gg, -1.0, 2.0 / 3.0) };
        let q = if p.0 < r { (r, p.1, p.2, p.0) } else { (p.0, p.1, p.3, r) };
        let d = q.0 - q.3.min(q.1);
        let e = 1.0e-4f32;
        [
            (q.2 + (q.3 - q.1) / (6.0 * d + e)).abs(),
            d / (q.0 + e),
            q.0,
        ]
    };
    let hsv_to_rgb = |c: [f32; 3]| {
        let (h, s, v) = (c[0], c[1], c[2]);
        let ch = |i: f32| {
            let p = (((h + i) % 1.0) * 6.0 - 3.0).abs();
            v * (1.0 + s * ((p - 1.0).clamp(0.0, 1.0) - 1.0))
        };
        [ch(1.0), ch(2.0 / 3.0), ch(1.0 / 3.0)]
    };
    let rotate_hue = |v: f32| {
        if v < 0.0 {
            v + 1.0
        } else if v > 1.0 {
            v - 1.0
        } else {
            v
        }
    };
    // uber：曝光 → LDR 支路 tonemap（None ⇒ saturate）。
    let mut c = sat3([c_in[0] * post, c_in[1] * post, c_in[2] * post]);
    // LUT：LogC 对比 → 滤色 → 抹平负值。
    c = [
        l2lin((l2l(c[0]) - 0.4135884) * cs + 0.4135884),
        l2lin((l2l(c[1]) - 0.4135884) * cs + 0.4135884),
        l2lin((l2l(c[2]) - 0.4135884) * cs + 0.4135884),
    ];
    c = [c[0] * fl[0], c[1] * fl[1], c[2] * fl[2]];
    c = [c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)];
    // HSV 色相。
    let mut hsv = rgb_to_hsv(c);
    hsv[0] = rotate_hue(hsv[0] + hs);
    c = hsv_to_rgb(hsv);
    // 绕亮度整体饱和。
    let luma = c[0] * 0.2126729 + c[1] * 0.7151522 + c[2] * 0.072175;
    c = [
        luma + ss * (c[0] - luma),
        luma + ss * (c[1] - luma),
        luma + ss * (c[2] - luma),
    ];
    sat3(c)
}

impl WeatherPostUniform {
    fn from_params(params: &WeatherPostParams) -> Self {
        WeatherPostUniform {
            diff_a: [
                gate(params.diff_on),
                params.diff_intensity,
                params.diff_contrast,
                params.diff_blend_mode,
            ],
            diff_b: [scatter_weight(params.diff_scatter), 0.0, 0.0, 0.0],
            flare_axis: [
                params.flare_axis[0],
                params.flare_axis[1],
                params.flare_intensity,
                params.flare_exponent,
            ],
            flare_c1: params.flare_color1,
            flare_c2: params.flare_color2,
            flare_off: [
                params.flare_offset1,
                params.flare_offset2,
                gate(params.flare_on),
                0.0,
            ],
            bloom_a: [params.bloom_uber[0], params.bloom_overlay, 0.0, 0.0],
            bloom_b: [
                params.bloom_uber[1],
                params.bloom_uber[2],
                params.bloom_uber[3],
                0.0,
            ],
            split_shadows: params.split_shadows,
            split_highlights: params.split_highlights,
            grade_a: {
                let g = params.grade.pack();
                [
                    g.post_exposure_linear,
                    g.hue_sat_con[0],
                    g.hue_sat_con[1],
                    g.hue_sat_con[2],
                ]
            },
            grade_b: {
                let g = params.grade.pack();
                [
                    g.color_filter_linear[0],
                    g.color_filter_linear[1],
                    g.color_filter_linear[2],
                    gate(params.grade_on),
                ]
            },
        }
    }

    /// 按槽序摊平成字节（与全局量桥同一写法）。
    fn gpu_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(WEATHER_UNIFORM_BYTES);
        for slot in [
            self.diff_a,
            self.diff_b,
            self.flare_axis,
            self.flare_c1,
            self.flare_c2,
            self.flare_off,
            self.bloom_a,
            self.bloom_b,
            self.grade_a,
            self.grade_b,
            self.split_shadows,
            self.split_highlights,
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), WEATHER_UNIFORM_BYTES);
        bytes
    }
}

/// 渲染世界的 uniform buffer；Prepare 侧每帧写。`scatter` 是上采样 pass 专用
/// 的独立小块（对齐约束见 `WeatherPostUniform::diff_b` 的注释）；`bloom_params`
/// 是泛光金字塔参数（预滤波与上采样共用一份：散射权重同一值，单独一块
/// 同一理由）。
#[derive(Resource)]
struct WeatherPostGpu {
    buffer: Buffer,
    scatter: Buffer,
    /// 泛光金字塔参数 vec4（律打包 `[scatter', clamp, g2l(threshold), knee]`）。
    bloom_params: Buffer,
}

/// 7 条管线的缓存 id + 5 个 bind group 布局 + 采样器 + 全黑占位图。
#[derive(Resource)]
struct WeatherPipelines {
    copy: CachedRenderPipelineId,
    down: CachedRenderPipelineId,
    up: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    bloom_prefilter: CachedRenderPipelineId,
    bloom_down: CachedRenderPipelineId,
    bloom_up: CachedRenderPipelineId,
    /// 直拷与降采样共用（binding 0 纹理 + binding 2 采样器）。
    copy_layout: BindGroupLayoutDescriptor,
    /// 上采样（0 高级别、1 低级别、2 采样器、4 散射权重）。
    up_layout: BindGroupLayoutDescriptor,
    /// 合成（0 场景、1 扩散层、2 采样器、3 整个 uniform 块、6 泛光金字塔）。
    composite_layout: BindGroupLayoutDescriptor,
    /// 泛光预滤波（0 输入、2 采样器、5 金字塔参数）。
    bloom_prefilter_layout: BindGroupLayoutDescriptor,
    /// 泛光上采样（0 本级、1 低级别、2 采样器、5 金字塔参数）。
    bloom_up_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    /// 1×1 全黑：扩散/泛光关着时合成 pass 的占位绑定（全黑 ⇒ 零贡献）。
    black: CachedTexture,
}

/// 本帧一个视图的两座金字塔（Prepare 侧现算；对应轴关着时为空，节点跳过
/// 那一段）。
#[derive(Component, Default)]
struct WeatherPyramid {
    downs: Vec<CachedTexture>,
    ups: Vec<CachedTexture>,
    bloom_downs: Vec<CachedTexture>,
    bloom_uploads: Vec<CachedTexture>,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct WeatherPostLabel;

/// 扩散金字塔的层级布局，逐项照源 pass：**高取档案的 `bufferHeight`
/// 原值**（不钳视口——源侧的描述符里根本没有视口，只有宽高比进得来），
/// 宽 = `trunc(宽高比 × 基高)`，级数 = `min(maxIterations,
/// trunc(log2(基高) − 1))`，且 `trunc(log2(基高) − 1) < 1` 时强取 1。
///
/// 每边至少 1 是源侧自己的地板（逐级折半那一步取 `max(1, ...)`），
/// 不是我方防御。此前这里额外钳了视口高、并把两边地板抬到 16、宽用
/// 四舍五入——三处都让小窗口下的模糊半径与真游戏不同，而没有任何
/// 判据会红：基高恒 540、常见视口更高，三处偏差同时休眠。
fn pyramid_levels(
    viewport_width: u32,
    viewport_height: u32,
    buffer_height: f32,
    max_iterations: f32,
) -> (u32, u32, usize) {
    let aspect = viewport_width.max(1) as f32 / viewport_height.max(1) as f32;
    let base = buffer_height.trunc() as i64;
    // 基高 ≤ 0 是数据损伤（源参数的下界是 32）；兜构造默认而不是造一张
    // 零尺寸纹理。
    let base = if base <= 0 { 540 } else { base };
    let th = (base as u32).max(1);
    let tw = ((th as f32 * aspect).trunc() as u32).max(1);
    let cap = ((th as f64).log2() as f32 - 1.0) as i64;
    let iters = max_iterations.trunc() as i64;
    let n = if cap < 1 { 1 } else { iters.min(cap).max(1) };
    (tw, th, n as usize)
}

fn pyramid_texture_descriptor(width: u32, height: u32) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: Some("weather_diffusion_texture"),
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

/// 泛光金字塔的层级布局：宽按画面宽高比折算（截断），高取泛光缓冲基高
/// **原值**——不钳视口，与扩散侧不同（产品链的泛光以固定缓冲高为基准，
/// 视口只进宽高比）。级数 = trunc(log2(h) − 1) 与上限 6 取小，逐级折半、
/// 每边至少 1；基高数据损伤（≤0）兜 540，与扩散侧同款。
fn bloom_pyramid_levels(
    viewport_width: u32,
    viewport_height: u32,
    buffer_height: f32,
) -> (u32, u32, usize) {
    let viewport_height = viewport_height.max(1);
    let viewport_width = viewport_width.max(1);
    let aspect = viewport_width as f32 / viewport_height as f32;
    let base = buffer_height.trunc() as i64;
    let base = if base <= 0 { 540 } else { base };
    let th = base as u32;
    let tw = ((th as f32 * aspect).trunc() as u32).max(1);
    let cap = (f64::from(th).log2() - 1.0).trunc() as i64;
    let n = if cap < 1 { 1 } else { cap.min(6) };
    (tw, th, n as usize)
}

/// 泛光金字塔的纹理描述符：label 与扩散侧不同——纹理缓存以整个 descriptor
/// （含 label）为键，两座金字塔尺寸序列相同也不许共享。
fn bloom_texture_descriptor(width: u32, height: u32) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: Some("weather_bloom_texture"),
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

/// RenderStartup：建 4 条管线（直拷 / 降采样 / 上采样 / 合成）与渲染侧资源。
fn init_weather_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let texture_entry = texture_2d(TextureSampleType::Float { filterable: true });
    let sampler_entry = sampler(SamplerBindingType::Filtering);

    let copy_layout = BindGroupLayoutDescriptor::new(
        "weather_copy_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_entry),
                (2, sampler_entry),
                // 扩散预过滤也读调色 uniform（金字塔的源是已调色的场景色）。
                (3, uniform_buffer::<WeatherPostUniform>(false)),
            ),
        ),
    );
    let up_layout = BindGroupLayoutDescriptor::new(
        "weather_up_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_entry),
                (1, texture_2d(TextureSampleType::Float { filterable: true })),
                (2, sampler_entry),
                (4, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );
    let composite_layout = BindGroupLayoutDescriptor::new(
        "weather_composite_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_entry),
                (1, texture_2d(TextureSampleType::Float { filterable: true })),
                (2, sampler_entry),
                (3, uniform_buffer::<WeatherPostUniform>(false)),
                (6, texture_2d(TextureSampleType::Float { filterable: true })),
            ),
        ),
    );
    let bloom_prefilter_layout = BindGroupLayoutDescriptor::new(
        "weather_bloom_prefilter_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_entry),
                (2, sampler_entry),
                (5, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );
    let bloom_up_layout = BindGroupLayoutDescriptor::new(
        "weather_bloom_up_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_entry),
                (1, texture_2d(TextureSampleType::Float { filterable: true })),
                (2, sampler_entry),
                (5, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );

    let vertex = fullscreen_shader.to_vertex_state();
    // 合成 pass 的目标格式：相机是 LDR（`camera.rs` 无 HDR/色调映射），
    // 主纹理即引擎默认格式。
    let pipeline = |entry_point: &str, format: TextureFormat, layout: &BindGroupLayoutDescriptor| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("weather_{entry_point}").into()),
            layout: vec![layout.clone()],
            vertex: vertex.clone(),
            fragment: Some(FragmentState {
                shader: WEATHER_SHADER.clone(),
                entry_point: Some(entry_point.to_owned().into()),
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
    let copy = pipeline("weather_copy_fragment", PYRAMID_FORMAT, &copy_layout);
    let down = pipeline("weather_down4_fragment", PYRAMID_FORMAT, &copy_layout);
    let up = pipeline("weather_up_fragment", PYRAMID_FORMAT, &up_layout);
    let composite = pipeline(
        "weather_composite_fragment",
        TextureFormat::bevy_default(),
        &composite_layout,
    );
    let bloom_prefilter = pipeline(
        "bloom_prefilter_fragment",
        PYRAMID_FORMAT,
        &bloom_prefilter_layout,
    );
    let bloom_down = pipeline("bloom_down_fragment", PYRAMID_FORMAT, &copy_layout);
    let bloom_up = pipeline("bloom_up_fragment", PYRAMID_FORMAT, &bloom_up_layout);

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    // 全黑占位图（1×1）：显式写入数据，不依赖未初始化纹理的内容。
    let black_texture = render_device.create_texture(&TextureDescriptor {
        label: Some("weather_black"),
        size: bevy::render::render_resource::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
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
    let black = CachedTexture {
        texture: black_texture,
        default_view: black_view,
    };
    commands.insert_resource(WeatherPipelines {
        copy,
        down,
        up,
        composite,
        bloom_prefilter,
        bloom_down,
        bloom_up,
        copy_layout,
        up_layout,
        composite_layout,
        bloom_prefilter_layout,
        bloom_up_layout,
        sampler,
        black,
    });

    // uniform buffer：一次建，Prepare 侧每帧整块写。散射权重与泛光金字塔
    // 参数各自单独一块（对齐约束见 `WeatherPostUniform::diff_b` 的注释）。
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("weather_post_uniform"),
        contents: &WeatherPostUniform::from_params(&WeatherPostParams::neutral()).gpu_bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    let scatter = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("weather_scatter_uniform"),
        // WGSL 里声明成 vec4<f32>（最小绑定尺寸 16），x 是权重、yzw 填零。
        contents: &[
            scatter_weight(0.0f32).to_le_bytes(),
            0.0f32.to_le_bytes(),
            0.0f32.to_le_bytes(),
            0.0f32.to_le_bytes(),
        ]
        .concat(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    let bloom_params = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("weather_bloom_params_uniform"),
        // 律打包形 `[scatter', clamp, g2l(threshold), knee]`（vec4）。
        contents: &[0.0f32.to_le_bytes(); 4].concat(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(WeatherPostGpu {
        buffer,
        scatter,
        bloom_params,
    });
}

/// Prepare：抽取后的轴值写进 uniform buffer。抽取经 ExtractCommands 落地，
/// Prepare 在其后，无序挂载会读到上一帧的值——所以挂在 Prepare 集。
fn write_weather_uniforms(
    params: Res<WeatherPostParams>,
    gpu: Res<WeatherPostGpu>,
    queue: Res<RenderQueue>,
) {
    queue.write_buffer(&gpu.buffer, 0, &WeatherPostUniform::from_params(&params).gpu_bytes());
    queue.write_buffer(&gpu.scatter, 0, &scatter_weight(params.diff_scatter).to_le_bytes());
    // 泛光金字塔参数：律打包值整块透传（消费侧不二次推导——律里是照
    // 源 pass 自己的 SetVector 式子装箱的；扩散侧的 scatter 折算不发生
    // 在这条轴上）。
    let p = params.bloom_prefilter;
    queue.write_buffer(
        &gpu.bloom_params,
        0,
        &[p[0].to_le_bytes(), p[1].to_le_bytes(), p[2].to_le_bytes(), p[3].to_le_bytes()].concat(),
    );
}

/// PrepareResources：按当帧视口与轴值现算两座金字塔，走引擎纹理缓存（免
/// 每帧重建）。挂 PrepareResources 而非 Prepare：本系统用 Commands 给视图
/// 实体插组件、图节点要读它——与引擎泛光同一挂点（组件插入与图执行之间
/// 有同步点保证）。
fn prepare_weather_pyramid(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    params: Res<WeatherPostParams>,
    views: Query<(Entity, &ExtractedCamera, Option<&WeatherPyramid>)>,
) {
    for (entity, camera, previous) in &views {
        // WeatherPostNode is installed only in Core3d. Overlay/composite 2D
        // cameras never consume these textures; allocating for them duplicates
        // the scene's pyramids. Drop an old component if a view changes graph.
        if camera.render_graph != Core3d.intern() {
            if previous.is_some() {
                commands.entity(entity).remove::<WeatherPyramid>();
            }
            continue;
        }
        let Some(viewport) = camera.physical_viewport_size else {
            if previous.is_some() {
                commands.entity(entity).remove::<WeatherPyramid>();
            }
            continue;
        };
        // 扩散金字塔（关着时为空——节点只剩合成）。
        let (downs, ups) = if params.diff_on {
            let (tw, th, n) = pyramid_levels(
                viewport.x,
                viewport.y,
                params.diff_buffer_height,
                params.diff_max_iterations,
            );
            let mut downs = Vec::with_capacity(n);
            let mut ups = Vec::with_capacity(n.max(1) - 1);
            let mut w = tw;
            let mut h = th;
            for i in 0..n {
                downs.push(texture_cache.get(&render_device, pyramid_texture_descriptor(w, h)));
                if i < n - 1 {
                    ups.push(texture_cache.get(&render_device, pyramid_texture_descriptor(w, h)));
                }
                w = (w >> 1).max(1);
                h = (h >> 1).max(1);
            }
            (downs, ups)
        } else {
            (Vec::new(), Vec::new())
        };
        // 泛光金字塔（关着时为空）。上采样链与扩散同构：第 i 级读自己的
        // 降采样 + 第 i+1 级的上采样结果（最深处取降采样链尾），写第 i
        // 级的上采样纹理。
        let (bloom_downs, bloom_uploads) = if params.bloom_on {
            let (tw, th, n) =
                bloom_pyramid_levels(viewport.x, viewport.y, params.bloom_buffer_height);
            let mut downs = Vec::with_capacity(n);
            let mut uploads = Vec::with_capacity(n.max(1) - 1);
            let mut w = tw;
            let mut h = th;
            for i in 0..n {
                downs.push(texture_cache.get(&render_device, bloom_texture_descriptor(w, h)));
                if i < n - 1 {
                    uploads.push(texture_cache.get(&render_device, bloom_texture_descriptor(w, h)));
                }
                w = (w >> 1).max(1);
                h = (h >> 1).max(1);
            }
            (downs, uploads)
        } else {
            (Vec::new(), Vec::new())
        };
        commands.entity(entity).insert(WeatherPyramid {
            downs,
            ups,
            bloom_downs,
            bloom_uploads,
        });
    }
}

/// 后处理节点：直拷预过滤 → 4-tap 降采样链 → 散射上采样链 → 泛光三连
///（阈值预滤波 → 平方域降采样 → 平方域上采样）→ 合成（扩散混合 + 泛光 +
/// 屏幕耀斑）。三条轴全关时整段跳过——不取 post_process_write，主纹理
/// 原样过（与不挂这条链逐位一致）。
#[derive(Default)]
struct WeatherPostNode;

impl ViewNode for WeatherPostNode {
    // 只在带金字塔组件的视图上跑（Prepare 只给 Core3d 视图插）。自发光
    // 目标是 Option：换装未完成或自发光 pass 未建目标时缺件——扩散/
    // 耀斑照跑，泛光段跳过、合成按黑图（零贡献，fail-closed 不吃上一
    // 帧余像）。
    type ViewQuery = (
        &'static ViewTarget,
        &'static WeatherPyramid,
        Option<&'static ViewEmissionTarget>,
        Option<&'static TransparentCapture>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, pyramid, emission_target, transparent_capture): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        if transparent_capture.is_some() { return Ok(()); }
        let params = world.resource::<WeatherPostParams>();
        if !params.diff_on && !params.flare_on && !params.bloom_on && !params.grade_on && params.split_highlights[3] == 0.0 {
            return Ok(());
        }
        let pipelines = world.resource::<WeatherPipelines>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let gpu = world.resource::<WeatherPostGpu>();
        let (
            Some(copy_pipeline),
            Some(down_pipeline),
            Some(up_pipeline),
            Some(composite_pipeline),
            Some(bloom_prefilter_pipeline),
            Some(bloom_down_pipeline),
            Some(bloom_up_pipeline),
        ) = (
            pipeline_cache.get_render_pipeline(pipelines.copy),
            pipeline_cache.get_render_pipeline(pipelines.down),
            pipeline_cache.get_render_pipeline(pipelines.up),
            pipeline_cache.get_render_pipeline(pipelines.composite),
            pipeline_cache.get_render_pipeline(pipelines.bloom_prefilter),
            pipeline_cache.get_render_pipeline(pipelines.bloom_down),
            pipeline_cache.get_render_pipeline(pipelines.bloom_up),
        ) else {
            // 着色器还在编（首几帧），这一帧跳过。
            return Ok(());
        };

        // 源/目的每帧轮换，bind group 只能在这里建。全部**先建后跑**：设备
        // 句柄借 render_context 的不可变引用，跑 pass 要可变借用，交叠即撞。
        let post_process = view_target.post_process_write();
        // 扩散金字塔的级数（扩散关着时为 0）。
        let n = if params.diff_on { pyramid.downs.len() } else { 0 };
        // 合成里的扩散层：单级金字塔直取直拷结果，多级取上采样链顶；扩散
        // 关着时用占位黑图（全黑 ⇒ 零贡献），着色器里门也不开。
        let diff_view: &TextureView = if n == 0 {
            &pipelines.black.default_view
        } else if n == 1 {
            &pyramid.downs[0].default_view
        } else {
            &pyramid.ups[0].default_view
        };
        // 泛光金字塔的级数：泛光开着且自发光目标在手才跑（目标缺件时整段
        // 跳过，合成按黑图——自发光缓冲没有「上一帧可用」的余像可吃）。
        let bloom_n = if params.bloom_on && emission_target.is_some() {
            pyramid.bloom_downs.len()
        } else {
            0
        };
        // 合成里的泛光层：与扩散同构——单级直取预滤波结果，多级取上采样
        // 链顶；关闭/缺件时占位黑图。
        let bloom_view: &TextureView = if bloom_n == 0 {
            &pipelines.black.default_view
        } else if bloom_n == 1 {
            &pyramid.bloom_downs[0].default_view
        } else {
            &pyramid.bloom_uploads[0].default_view
        };

        let copy_bind_group: Option<BindGroup>;
        let down_bind_groups: Vec<BindGroup>;
        let up_bind_groups: Vec<BindGroup>;
        let bloom_prefilter_bind_group: Option<BindGroup>;
        let bloom_down_bind_groups: Vec<BindGroup>;
        let bloom_up_bind_groups: Vec<BindGroup>;
        let composite_bind_group: BindGroup;
        {
            let device = render_context.render_device();
            let sampler = &pipelines.sampler;
            let copy_layout = pipeline_cache.get_bind_group_layout(&pipelines.copy_layout);
            let up_layout = pipeline_cache.get_bind_group_layout(&pipelines.up_layout);
            let composite_layout =
                pipeline_cache.get_bind_group_layout(&pipelines.composite_layout);
            let bloom_prefilter_layout =
                pipeline_cache.get_bind_group_layout(&pipelines.bloom_prefilter_layout);
            let bloom_up_layout = pipeline_cache.get_bind_group_layout(&pipelines.bloom_up_layout);

            if n > 0 {
                // 直拷预过滤：源（编码域画面）→ 金字塔第一级。调色 uniform
                // 也绑上（金字塔的源是已调色的场景色）。
                copy_bind_group = Some(device.create_bind_group(
                    "weather_copy_bind_group",
                    &copy_layout,
                    &BindGroupEntries::with_indices((
                        (0, post_process.source),
                        (2, BindingResource::Sampler(sampler)),
                        (3, gpu.buffer.as_entire_binding()),
                    )),
                ));
                // 逐级降采样：上一级 → 下一级。建组顺序无所谓，跑序才要紧。
                down_bind_groups = (1..n)
                    .map(|i| {
                        device.create_bind_group(
                            "weather_down_bind_group",
                            &copy_layout,
                            &BindGroupEntries::with_indices((
                                (0, &pyramid.downs[i - 1].default_view),
                                (2, BindingResource::Sampler(sampler)),
                                (3, gpu.buffer.as_entire_binding()),
                            )),
                        )
                    })
                    .collect();
                // 逐级上采样：高级别 + 低一级（最深处取降采样链尾，其余取上
                // 一轮上采样）按散射权重混。建组按索引正序，跑序倒序。
                // 散射权重绑独立小块 buffer 的整段（见 WeatherPostGpu::scatter
                // 的注释——按偏移绑会撞 uniform 对齐限制）。
                up_bind_groups = (0..n - 1)
                    .map(|i| {
                        let low = if i == n - 2 {
                            &pyramid.downs[i + 1].default_view
                        } else {
                            &pyramid.ups[i + 1].default_view
                        };
                        device.create_bind_group(
                            "weather_up_bind_group",
                            &up_layout,
                            &BindGroupEntries::with_indices((
                                (0, &pyramid.downs[i].default_view),
                                (1, low),
                                (2, BindingResource::Sampler(sampler)),
                                (4, gpu.scatter.as_entire_binding()),
                            )),
                        )
                    })
                    .collect();
            } else {
                copy_bind_group = None;
                down_bind_groups = Vec::new();
                up_bind_groups = Vec::new();
            }

            if bloom_n > 0 {
                // 泛光预滤波：自发光缓冲（已 resolve 的单采样视图）→ 金字塔
                // 第一级。阈值软膝在着色器里；参数整块绑（预滤波与上采样
                // 共用同一份打包）。
                bloom_prefilter_bind_group = Some(device.create_bind_group(
                    "bloom_prefilter_bind_group",
                    &bloom_prefilter_layout,
                    &BindGroupEntries::with_indices((
                        (0, &emission_target.unwrap().resolved),
                        (2, BindingResource::Sampler(sampler)),
                        (5, gpu.bloom_params.as_entire_binding()),
                    )),
                ));
                bloom_down_bind_groups = (1..bloom_n)
                    .map(|i| {
                        device.create_bind_group(
                            "bloom_down_bind_group",
                            &copy_layout,
                            &BindGroupEntries::with_indices((
                                (0, &pyramid.bloom_downs[i - 1].default_view),
                                (2, BindingResource::Sampler(sampler)),
                                (3, gpu.buffer.as_entire_binding()),
                            )),
                        )
                    })
                    .collect();
                // 泛光上采样：与扩散同构（本级 + 低一级），参数块换成泛光
                // 自己的（散射权重同值，无扩散侧的 lerp 折算）。
                bloom_up_bind_groups = (0..bloom_n - 1)
                    .map(|i| {
                        let low = if i == bloom_n - 2 {
                            &pyramid.bloom_downs[i + 1].default_view
                        } else {
                            &pyramid.bloom_uploads[i + 1].default_view
                        };
                        device.create_bind_group(
                            "bloom_up_bind_group",
                            &bloom_up_layout,
                            &BindGroupEntries::with_indices((
                                (0, &pyramid.bloom_downs[i].default_view),
                                (1, low),
                                (2, BindingResource::Sampler(sampler)),
                                (5, gpu.bloom_params.as_entire_binding()),
                            )),
                        )
                    })
                    .collect();
            } else {
                bloom_prefilter_bind_group = None;
                bloom_down_bind_groups = Vec::new();
                bloom_up_bind_groups = Vec::new();
            }

            // 合成：源 + 扩散层 + 泛光层 → 目的。写目的是 post_process_write
            // 的约定——主纹理已被翻到目的侧，不写就丢帧。
            composite_bind_group = device.create_bind_group(
                "weather_composite_bind_group",
                &composite_layout,
                &BindGroupEntries::with_indices((
                    (0, post_process.source),
                    (1, diff_view),
                    (2, BindingResource::Sampler(sampler)),
                    (3, gpu.buffer.as_entire_binding()),
                    (6, bloom_view),
                )),
            );
        }

        // 跑 pass：直拷 → 降采样链 → 上采样链（倒序）→ 泛光三连 → 合成。
        if let Some(bind_group) = copy_bind_group.as_ref() {
            fullscreen_pass(
                render_context,
                copy_pipeline,
                bind_group,
                &pyramid.downs[0].default_view,
                "weather_copy_pass",
            );
            for i in 1..n {
                fullscreen_pass(
                    render_context,
                    down_pipeline,
                    &down_bind_groups[i - 1],
                    &pyramid.downs[i].default_view,
                    "weather_down_pass",
                );
            }
            for i in (0..n - 1).rev() {
                fullscreen_pass(
                    render_context,
                    up_pipeline,
                    &up_bind_groups[i],
                    &pyramid.ups[i].default_view,
                    "weather_up_pass",
                );
            }
        }
        if let Some(bind_group) = bloom_prefilter_bind_group.as_ref() {
            fullscreen_pass(
                render_context,
                bloom_prefilter_pipeline,
                bind_group,
                &pyramid.bloom_downs[0].default_view,
                "bloom_prefilter_pass",
            );
            for i in 1..bloom_n {
                fullscreen_pass(
                    render_context,
                    bloom_down_pipeline,
                    &bloom_down_bind_groups[i - 1],
                    &pyramid.bloom_downs[i].default_view,
                    "bloom_down_pass",
                );
            }
            for i in (0..bloom_n - 1).rev() {
                fullscreen_pass(
                    render_context,
                    bloom_up_pipeline,
                    &bloom_up_bind_groups[i],
                    &pyramid.bloom_uploads[i].default_view,
                    "bloom_up_pass",
                );
            }
        }
        fullscreen_pass(
            render_context,
            composite_pipeline,
            &composite_bind_group,
            post_process.destination,
            "weather_composite_pass",
        );
        Ok(())
    }
}

/// 一个全屏三角形 pass：设管线、设 bind group、画 3 顶点。
#[allow(clippy::too_many_arguments)]
fn fullscreen_pass(
    render_context: &mut RenderContext,
    pipeline: &bevy::render::render_resource::RenderPipeline,
    bind_group: &bevy::render::render_resource::BindGroup,
    target: &TextureView,
    label: &'static str,
) {
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
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// 天气系统插件：主世界装载/切换/逐帧写出，渲染世界 uniform + 金字塔 +
/// render graph 节点（挂在色调映射之后、主后处理收尾之前）。
#[derive(Component, Clone, bevy::render::extract_component::ExtractComponent)]
pub(crate) struct TransparentCapture;

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<TransparentCapture>::default());
        app.add_plugins(ExtractResourcePlugin::<WeatherPostParams>::default())
            .init_resource::<WeatherTransition>()
            .init_resource::<WeatherTimelineState>()
            .init_resource::<CommittedPhenomenon>()
            .init_resource::<CurrentPhenomenon>()
            .init_resource::<CurrentPhenomenonId>()
            .init_resource::<PhenomenonCatalogue>()
            .add_message::<WeatherRequest>()
            .add_systems(Startup, load)
            .add_systems(Update, (parse_index, resolve_all).chain().before(WeatherEnvironmentUpdate))
            .add_systems(Update, (switch_phenomenon, advance).chain().in_set(WeatherEnvironmentUpdate))
            .add_systems(Update, (commit_environment, evaluate_timeline, write_environment).chain().after(crate::weather_fx::spawn_when_ready).after(crate::character_material::apply_fog));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_weather_pipelines)
            // Prepare 链在 ExtractCommands 之后：抽取系统用 Commands 落资源，
            // 无序挂载会读到不存在/上一帧的值；金字塔挂 PrepareResources（理由
            // 见该函数注释）。
            .add_systems(
                Render,
                (
                    write_weather_uniforms.in_set(RenderSystems::Prepare),
                    prepare_weather_pyramid.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_render_graph_node::<ViewNodeRunner<WeatherPostNode>>(Core3d, WeatherPostLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    WeatherPostLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }
}

#[cfg(test)]
mod transition_commit_tests {
    use super::committed;
    #[test]
    fn profile_and_globals_hold_until_crossfade_completion() {
        // CrossFadeCore awaits DoCrossFadeAsync before RefreshShaderView and
        // RefreshPostProcess. The previous profile remains authoritative before then.
        let previous = (1, [0.5518868, 0.8096058, 1.0, 1.0]);
        let next = (2, [0.12, 0.16, 0.3, 1.0]);
        for progress in [0.0, 0.01, 0.5, 0.999999] {
            assert_eq!(committed(&previous, &next, progress), &previous);
        }
        assert_eq!(committed(&previous, &next, 1.0), &next);
    }
}
