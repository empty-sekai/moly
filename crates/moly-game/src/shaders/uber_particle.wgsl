// Both passes evaluate in the source storage colour space. The forward target
// needs format decoding; the ARGB32 effect target clamps/quantizes that value.
#ifndef UBER_EFFECT_PASS
#import bevy_pbr::view_transformations::{position_world_to_view, position_view_to_clip}
#import bevy_pbr::mesh_view_bindings::view
// 顶点槽位由 `UberParticleMaterial::specialize` 显式装配；两处同改。
struct UberVertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
    @location(8) custom1: vec4<f32>,
    @location(9) custom2: vec4<f32>,
}
#endif
struct UberT1Params {
    base_st: vec4<f32>,
    tint_colour: vec4<f32>,
    scalars: vec4<f32>,
    luminance: vec4<f32>,
    // x=tint selector; y=soft intensity; z=soft keyword; w=emission selector.
    coords: vec4<f32>,
}
#ifdef UBER_EFFECT_PASS
#import bevy_render::view::View
@group(0) @binding(1) var<uniform> view: View;
@group(2) @binding(0) var opaque_depth: texture_2d<f32>;
@group(2) @binding(1) var<uniform> depth_available: vec4<u32>;
struct EmissionObject {
    clip_from_local: mat4x4<f32>,
    params: UberT1Params,
    emission_colour: vec4<f32>,
    emission: vec4<f32>,
    phenomena_light: vec4<f32>,
    padding: array<vec4<f32>, 6>,
}
@group(0) @binding(0) var<uniform> object: EmissionObject;
@group(1) @binding(2) var base_map: texture_2d<f32>;
@group(1) @binding(3) var base_sampler: sampler;
struct EffectVertex {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) custom1: vec4<f32>,
    @location(4) custom2: vec4<f32>,
}
#else
// Only the first two slots are read here; the bound buffer is the whole
// site-wide block shared with the fixture and site materials.
struct ParticleEnv {
    light_vector: vec4<f32>,
    phenomena_light_color: vec4<f32>,
}
@group(3) @binding(0) var<uniform> params: UberT1Params;
@group(3) @binding(1) var base_map: texture_2d<f32>;
@group(3) @binding(2) var base_sampler: sampler;
@group(3) @binding(3) var<uniform> env: ParticleEnv;
@group(1) @binding(0) var opaque_depth: texture_2d<f32>;
@group(1) @binding(1) var<uniform> depth_available: vec4<u32>;
#endif
struct UberVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) custom1: vec4<f32>,
    @location(3) custom2: vec4<f32>,
}
#ifdef UBER_EFFECT_PASS
@vertex
fn emission_vertex(mesh: EffectVertex) -> UberVertexOutput {
    var out: UberVertexOutput;
    out.position = object.clip_from_local * vec4<f32>(mesh.position, 1.0);
    out.uv = mesh.uv;
    out.colour = mesh.colour;
    out.custom1 = mesh.custom1;
    out.custom2 = mesh.custom2;
    return out;
}
#else
@vertex
fn vertex(mesh: UberVertex) -> UberVertexOutput {
    var out: UberVertexOutput;
    out.position = position_view_to_clip(position_world_to_view(mesh.position));
    out.uv = mesh.uv;
    out.colour = mesh.color;
    out.custom1 = mesh.custom1;
    out.custom2 = mesh.custom2;
    return out;
}
#endif
fn srgb_format_encode(v: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
}
fn srgb_format_decode(v: vec3<f32>) -> vec3<f32> {
    return select(pow(max((v + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
}
/// 逐粒子流选择器：`coord = 分量号 × 10 + 来源号`。
/// 来源 0 = 常量零向量 · 1 = custom1 · 2 = custom2；分量号 0..3 选 x/y/z/w。
/// 源程序写成 `dot(TempArray0[来源], ImmCB_1[分量])`，两个都是单位向量/常量，
/// 等价于直接取该分量——`TempArray0[0]` 在源里就是写死的 `vec4(0,0,0,0)`。
fn stream_value(coord: f32, custom1: vec4<f32>, custom2: vec4<f32>) -> f32 {
    let packed = u32(max(coord, 0.0));
    let source = packed % 10u;
    let component = packed / 10u;
    var v = vec4<f32>(0.0);
    if source == 1u { v = custom1; }
    else if source == 2u { v = custom2; }
    if component == 1u { return v.y; }
    if component == 2u { return v.z; }
    if component == 3u { return v.w; }
    return v.x;
}

fn source_colour(in: UberVertexOutput, p: UberT1Params) -> vec4<f32> {
    let uv = in.uv * p.base_st.xy + p.base_st.zw;
    let sample = textureSampleBias(base_map, base_sampler, vec2<f32>(uv.x, 1.0 - uv.y), p.scalars.y);
    let base = vec4<f32>(srgb_format_encode(sample.rgb), sample.a);
#ifdef UBER_TINT_AREA_ALL
    // 源口径：混合率 = 材质标量 + 逐粒子流那一项，再钳制。
    let tint_rate = p.scalars.x + stream_value(p.coords.x, in.custom1, in.custom2);
    let tinted = base * ((p.tint_colour - vec4<f32>(1.0)) * clamp(tint_rate, 0.0, 1.0) + vec4<f32>(1.0));
#else
#ifdef UBER_PLAIN_COLOUR
    // 原版粒子族（`Particles/Standard Unlit`）：材质色是**无条件平乘**，
    // 不是上面那条朝染色插值的臂。源程序片元段化简后恰三步——
    //   c = texture(_MainTex, uv) ; c *= _Color ; c *= 顶点色
    // 末一步在本函数返回处统一乘。两族互斥：该族没有 `_TINT_AREA_*`
    // 关键字，染色族没有 `_Color` 属性，所以共用 `tint_colour` 这一槽，
    // 由这个 def 决定它按哪种语义读。
    let tinted = base * p.tint_colour;
#else
    let tinted = base;
#endif
#endif
#ifdef UBER_EFFECT_PASS
#ifdef UBER_EMISSION_AREA_ALL
    var emission_colour = vec3<f32>(0.0);
    if object.emission.y == 0.0 { emission_colour = object.emission_colour.rgb; }
    if object.emission.y == 1.0 { emission_colour = tinted.rgb; }
    let intensity = object.emission.x + stream_value(p.coords.w, in.custom1, in.custom2);
    return vec4<f32>(tinted.rgb * emission_colour * intensity, tinted.a) * in.colour;
#else
    // Actual JP MysekaiEffect with no emission-area keyword writes zero RGB.
    // Alpha still participates in blend factors and the subsequent alpha chain.
    return vec4<f32>(vec3<f32>(0.0), tinted.a * in.colour.a);
#endif
#else
    return tinted * in.colour;
#endif
}
// 源程序的片元链在末尾有两条 uniform 分支。两条都读同一个「已乘过自发光
// 与顶点色」的 rgb，次序固定：亮度键控透明（只改 alpha）→ 现象光（只改
// rgb）。基色程序与带自发光的程序在这一点上一致，所以这段尾巴对两个入口
// 同形。两者都在源存储色彩空间里算，前向入口的格式解码仍在最后一步。

/// `_TranceparencyByLuminanceEnabled` 臂：按源亮度对 alpha 做一次 smoothstep
/// 溶解。逐句对着源程序的标量段转写——
///   v      = _InverseLuminanceTransparency >= 0.5 ? 1 - lum : lum
///   p      = min(progress, 1) ; s = min(sharpness, 1)
///   lo     = p * (2 - s) - (1 - s)
///   宽度    = p * (2 - s) - lo
///   alpha *= smoothstep(lo, lo + 宽度, v)
/// 宽度代数上等于 `1 - s`，但这里照源写成那个减式，并照源先取倒数再相乘
/// （而不是直接相除）：s = 1 时源本身就是除以零，这里不加保护——加了就与
/// 源不一样。
fn luminance_transparency(rgb: vec3<f32>, alpha: f32, l: vec4<f32>) -> f32 {
    let lum = dot(rgb, vec3<f32>(0.298911989, 0.586610973, 0.114478));
    let v = select(lum, 1.0 - lum, l.z >= 0.5);
    let p = min(l.x, 1.0);
    let s = min(l.y, 1.0);
    let lo = p * (2.0 - s) - (1.0 - s);
    let inv_width = 1.0 / (p * (2.0 - s) - lo);
    var t = clamp((v - lo) * inv_width, 0.0, 1.0);
    t = t * t * (t * -2.0 + 3.0);
    return alpha * t;
}

/// `_PhenomenaLightEnabled` 臂：先把 rgb 钳到 [0,1]，再按全局现象光色的
/// alpha 在「钳后色」与「钳后色 × 光色」之间插值。家具链的
/// `phenomena_light_blend` 是同一条式子——差别只在粒子这条先钳制，那一步
/// 对 HDR 粒子色不是恒等，不能省。
fn phenomena_light(rgb: vec3<f32>, light: vec4<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return light.w * (c * light.rgb - c) + c;
}

/// 两条臂的共同尾段。`scalars.z` / `scalars.w` 就是源程序里那两个 uniform
/// 判别量，判据照源写成 `0.5 < x`。两条臂都读**进入尾段时**的 rgb：现象光
/// 不读被亮度臂改过的 alpha，亮度臂也不读被现象光改过的 rgb。
fn shade_tail(colour: vec4<f32>, p: UberT1Params, light: vec4<f32>) -> vec4<f32> {
    var out = colour;
    if p.scalars.z > 0.5 {
        out.a = luminance_transparency(colour.rgb, colour.a, p.luminance);
    }
    if p.scalars.w > 0.5 {
        out = vec4<f32>(phenomena_light(colour.rgb, light), out.a);
    }
    return out;
}

// Source tail: saturate((sceneEyeZ - particleEyeZ) * (1 / intensity)).
// No epsilon is added to the authored intensity. The eye-depth transform below
// uses the actual projection, so finite-far and infinite reverse-Z do not share
// guessed near/far constants. Current Moly weather cameras are perspective.
// BEGIN RAW DEPTH LOAD CONTRACT
// The host aliases the existing depth-format view as unfilterable float. This
// is not a colour conversion or a second depth render: raw texels are unchanged.
// GLSL ES can texelFetch this sampler2D without shadow-comparison semantics.
fn load_depth_at_pixel(depth: texture_2d<f32>, pixel: vec2<f32>) -> f32 {
    return textureLoad(depth, vec2<i32>(pixel), 0).x;
}
// END RAW DEPTH LOAD CONTRACT

fn positive_eye_depth(device_z: f32) -> f32 {
    return view.clip_from_view[3][2] / (device_z + view.clip_from_view[2][2]);
}
fn soften_alpha(colour: vec4<f32>, scene_z: f32, particle_z: f32, intensity: f32) -> vec4<f32> {
    let distance = positive_eye_depth(scene_z) - positive_eye_depth(particle_z);
    let fade = clamp((1.0 / intensity) * distance, 0.0, 1.0);
    return vec4<f32>(colour.rgb, colour.a * fade);
}

#ifdef UBER_EFFECT_PASS
@fragment
fn emission_fragment(in: UberVertexOutput) -> @location(0) vec4<f32> {
    var colour = shade_tail(source_colour(in, object.params), object.params, object.phenomena_light);
    if object.params.coords.z > 0.5 {
        if depth_available.x == 0u { discard; }
        // Same single-sample snapshot as the forward path. Incompatible MSAA
        // attachments are rejected by the host; no sample-zero resolve is used.
        let scene_z = load_depth_at_pixel(opaque_depth, in.position.xy);
        colour = soften_alpha(colour, scene_z, in.position.z, object.params.coords.y);
    }
    return colour;
}
#else
@fragment
fn fragment(in: UberVertexOutput) -> @location(0) vec4<f32> {
    var colour = shade_tail(source_colour(in, params), params, env.phenomena_light_color);
    if params.coords.z > 0.5 {
        // Discard, rather than merely setting alpha=0: authored blend factors
        // may be One/One, so alpha alone does not guarantee zero contribution.
        if depth_available.x == 0u { discard; }
        let scene_z = load_depth_at_pixel(opaque_depth, in.position.xy);
        colour = soften_alpha(colour, scene_z, in.position.z, params.coords.y);
    }
    return vec4<f32>(srgb_format_decode(colour.rgb), colour.a);
}
#endif
