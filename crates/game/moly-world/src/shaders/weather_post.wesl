// The weather post-processing fullscreen pass family: copy (diffusion
// prefilter), 4-tap downsample, scatter upsample, composite (diffusion blend +
// bloom + screen flare), and the bloom's own trio (threshold prefilter, squared-
// domain downsample, squared-domain upsample). The fragment formulas follow the
// product chain's shaders: the diffusion prefilter is a plain copy (no
// threshold), downsampling is a 4-tap box, upsampling is a 4-tap box of the
// next lower level blended by the scatter weight; the bloom prefilter runs a
// soft-knee threshold curve and square-compresses its output, and both pyramid
// levels filter in the squared domain; in the composite the diffusion layer
// first raises contrast around a pivot and is then merged by blend mode, the
// bloom block is merged in the intensity–tint–overlay three-step form, and the
// screen flare and the sun halo each stack one hard-light layer of two opposite
// linear gradients along their own axis (screen flare first, sun halo second).
//
// The chain's input is the camera colour **after** engine post-processing: the
// engine's colour LUT and engine bloom are already done in an upstream node (see
// weather_stock_post.wesl), so no further grading happens here.
//
// # Colour domain conventions (the same as the product chain)
//
// The product's chain is "gamma passthrough": the scene buffer holds encoded
// values, the composite explicitly decodes to linear at the start and explicitly
// encodes back at the end. This pipeline's scene buffer is an sRGB texture — the
// hardware decodes to linear on sampling (equivalent to the chain's sRGBToLinear)
// and encodes back on write (equivalent to the chain tail's linearToSRGB), so
// the composite formulas do not hand-write those two round trips; the copy pass
// must instead **manually encode back to the storage domain** before writing the
// pyramid, so that the pyramid carries encoded values like the product chain
// (the composite takes the diffusion layer as encoded values, with no second
// decode).
// The bloom pyramid differs: its input is the emission buffer (half-float,
// **storage domain** — the mask is encoded back to the storage domain after
// sampling, the buffer is `Rgba16Float` and the hardware does not encode again),
// the prefilter square-root-compresses storage-domain values, the levels carry
// compressed values, and the composite squares them back to the storage domain
// before consuming them.
//
// The vertex stage uses the engine's fullscreen triangle (3 vertices cover the
// screen); uv is the screen coordinate with the origin at the top-left and v
// pointing down. The product's fullscreen blit has its origin at the bottom-left
// and v pointing up (the vertex shader maps uv directly to clip coordinates,
// uv·2−1), and both halos define their gradient axes in that coordinate system
// ⇒ halos always use `source_uv = (u, 1 − v)`; places that only sample textures
// do not depend on the origin.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct WeatherPostUniform {
    // 扩散：(门, 强度, 对比, 混合模式)
    diff_a: vec4<f32>,
    // 扩散：(散射权重, 0, 0, 0) —— 上采样 pass 只读 x，与合成共用一个
    // uniform 块时各自按偏移绑。
    diff_b: vec4<f32>,
    // 屏幕耀斑：(投影轴.x, 投影轴.y, 强度, 衰减指数)
    flare_axis: vec4<f32>,
    // 耀斑两层色：rgb 是混色，alpha 是该层权重。
    flare_c1: vec4<f32>,
    flare_c2: vec4<f32>,
    // 耀斑：(衰减偏移1, 衰减偏移2, 门, 0)
    flare_off: vec4<f32>,
    // 泛光：(强度, overlay 强度, 0, 0)——强度已折轴门与亮门（乘法，源
    // 式子本身就是一次乘法）。
    bloom_a: vec4<f32>,
    // 泛光：rgb 是着色 tint。
    bloom_b: vec4<f32>,
    // 太阳光晕：(屏幕轴.x, 屏幕轴.y, 因子 × 强度, 衰减指数)；轴与因子是
    // 每帧由方向光向量与相机算的绘制时量。
    sun_axis: vec4<f32>,
    // 太阳光晕两层色：rgb 是混色，alpha 是该层权重。
    sun_c1: vec4<f32>,
    sun_c2: vec4<f32>,
    // 太阳光晕：(衰减偏移1, 衰减偏移2, 门, 0)。
    sun_off: vec4<f32>,
};

// 直拷与降采样读 binding 0；上采样的高级别也读 binding 0、低级别读
// binding 1；合成的场景图读 binding 0、扩散层读 binding 1、泛光金字塔
// 顶层读 binding 6。各 pass 的 bind group 布局只声明自己用到的 binding。
@group(0) @binding(0) var t_a: texture_2d<f32>;
@group(0) @binding(1) var t_b: texture_2d<f32>;
@group(0) @binding(2) var s_linear: sampler;
@group(0) @binding(3) var<uniform> u_post: WeatherPostUniform;
@group(0) @binding(4) var<uniform> u_scatter: vec4<f32>;
// 泛光金字塔参数（预滤波与上采样共用一份：x=散射权重、y=亮部钳制、
// z=阈值（gamma 域转线性域后的值）、w=膝宽）。
@group(0) @binding(5) var<uniform> u_bloom_params: vec4<f32>;
@group(0) @binding(6) var t_c: texture_2d<f32>;

// 扩散对比度的枢轴：0.5 在编码域上的线性值（pow(0.5, 2.2)）。
const DIFF_PIVOT: f32 = 0.217637643;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let hi = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) * 1.055 - 0.055;
    return mix(hi, c * 12.92, step(c, vec3<f32>(0.0031308)));
}

// hard light：判别位在混合色 s（>0.5 走变亮支）。
fn hard_light(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    return mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(vec3<f32>(0.5), s));
}

// overlay：与 hard light 同式，判别位换成底色 b。
fn overlay(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    return mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(vec3<f32>(0.5), b));
}

fn soft_light(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    return mix(
        2.0 * b * s + b * b * (1.0 - 2.0 * s),
        2.0 * b * (1.0 - s) + sqrt(max(b, vec3<f32>(0.0))) * (2.0 * s - 1.0),
        step(vec3<f32>(0.5), s),
    );
}

fn pow_safe(x: f32, e: f32) -> f32 {
    return select(0.0, pow(x, e), x > 0.0);
}

// 预过滤（扩散）：直拷进编码域——扩散的预过滤没有阈值，整幅画面都进模糊。
// 源是上游引擎后处理写出的相机色（已调色）。
@fragment
fn weather_copy_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(t_a, s_linear, in.uv).rgb;
    return vec4<f32>(linear_to_srgb(c), 1.0);
}

// 降采样：4-tap box，采样步长取**源纹理**尺寸的倒数。
@fragment
fn weather_down4_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_a));
    let s = textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(-1.0, -1.0)).rgb
        + textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(1.0, -1.0)).rgb
        + textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(-1.0, 1.0)).rgb
        + textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(1.0, 1.0)).rgb;
    return vec4<f32>(s * 0.25, 1.0);
}

// 上采样：低一级 4-tap box，与本级按散射权重线性混合；采样步长取**低级别**
// 纹理尺寸的倒数。
@fragment
fn weather_up_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let low_texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_b));
    let lo = textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(-1.0, -1.0)).rgb
        + textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(1.0, -1.0)).rgb
        + textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(-1.0, 1.0)).rgb
        + textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(1.0, 1.0)).rgb;
    let hi = textureSample(t_a, s_linear, in.uv).rgb;
    return vec4<f32>(mix(hi, lo * 0.25, u_scatter.x), 1.0);
}

// ---- 泛光三连（金字塔携带平方压缩值：预滤波开方压缩、两级在平方域
// 滤波、合成开方还原）----

// 泛光预滤波：阈值软膝曲线。luma 取 rgb 最大通道；低于阈值时软膝把
// 权重滚到 0（不是硬切）；权重除 luma 后乘回原色、开方压缩输出。
// 零发射（黑）像素：luma=0 ⇒ 权重 0 ⇒ 输出 0，不进金字塔。
@fragment
fn bloom_prefilter_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(t_a, s_linear, in.uv);
    let p = u_bloom_params;
    let denom = p.w * 4.0 + 1e-4;
    let c = min(sample.rgb, vec3<f32>(p.y));
    var luma = max(c.g, c.r);
    luma = max(c.b, luma);
    let x = luma - p.z;
    let soft = min(2.0 * p.w, max(x + p.w, 0.0));
    let w0 = max(soft * soft / denom, x);
    let w = w0 / max(luma, 1e-4);
    return vec4<f32>(sqrt(max(w * c, vec3<f32>(0.0))), 1.0);
}

// 泛光降采样：4 角 ±1 **源** texel 的平方平均再开方（等效在平方域做
// box 滤波后保持压缩存储）。
@fragment
fn bloom_down_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_a));
    let a = textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(-1.0, -1.0)).rgb;
    let b = textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(1.0, -1.0)).rgb;
    let c = textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(-1.0, 1.0)).rgb;
    let d = textureSample(t_a, s_linear, in.uv + texel * vec2<f32>(1.0, 1.0)).rgb;
    let sq = (a * a + b * b + c * c + d * d) * 0.25;
    return vec4<f32>(sqrt(sq), 1.0);
}

// 泛光上采样：低级别 4 角（±1 低级别 texel）的平方平均 = low_sq；本级
// 直采 s、s_sq = s²；输出 sqrt(scatter·(low_sq − s_sq) + s_sq)。散射
// 权重与预滤波共用 u_bloom_params.x（一条链一个值）。暗轴变体（第二套
// 散射权重）是死支路：创作数据里暗轴强度恒 0，不编译。
@fragment
fn bloom_up_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let low_texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_b));
    let a = textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(-1.0, -1.0)).rgb;
    let b = textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(1.0, -1.0)).rgb;
    let c = textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(-1.0, 1.0)).rgb;
    let d = textureSample(t_b, s_linear, in.uv + low_texel * vec2<f32>(1.0, 1.0)).rgb;
    let low_sq = (a * a + b * b + c * c + d * d) * 0.25;
    let s = textureSample(t_a, s_linear, in.uv).rgb;
    let s_sq = s * s;
    return vec4<f32>(sqrt(max(u_bloom_params.x * (low_sq - s_sq) + s_sq, vec3<f32>(0.0))), 1.0);
}

// One halo layer: two opposite linear gradients along `axis`, each weighted by
// (offset, exponent, colour alpha, strength), stacked in order as hard-light
// layers (colour 1 first, colour 2 second). The screen flare and the sun halo
// are the same function run once on each of two uniform sets (the program's two
// sections have the same form).
fn flare_layer(
    b_in: vec3<f32>,
    uv: vec2<f32>,
    axis: vec2<f32>,
    strength: f32,
    exponent: f32,
    offsets: vec2<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
) -> vec3<f32> {
    var b = b_in;
    let t = dot(uv - vec2<f32>(0.5), axis);
    let w1 = pow_safe(clamp(t - offsets.x, 0.0, 1.0), exponent) * c1.a * strength;
    let w2 = pow_safe(clamp((1.0 - t) - offsets.y, 0.0, 1.0), exponent) * c2.a * strength;
    b = mix(b, hard_light(b, c1.rgb), w1);
    b = mix(b, hard_light(b, c2.rgb), w2);
    return b;
}

// Composite: the base colour (the camera colour after upstream engine post-
// processing) is clamped to 100 and enters the linear domain (the hardware
// already decoded it to linear), then diffusion → bloom → screen flare → sun
// halo (the program's section order), and the hardware encodes back to the
// storage domain at the end.
@fragment
fn weather_composite_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(t_a, s_linear, in.uv).rgb;
    var b = min(c, vec3<f32>(100.0));
    let source_uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);

    if u_post.diff_a.x > 0.5 {
        var d = textureSample(t_b, s_linear, in.uv).rgb;
        d = clamp((d - vec3<f32>(DIFF_PIVOT)) * u_post.diff_a.z + vec3<f32>(DIFF_PIVOT), vec3<f32>(0.0), vec3<f32>(1.0));
        var s = b;
        let mode = u_post.diff_a.w;
        if mode < 8.0 {
            s = hard_light(b, d);
        } else if mode < 12.0 {
            s = b + d;
        } else if mode < 16.0 {
            s = overlay(b, d);
        } else if mode < 20.0 {
            s = soft_light(b, d);
        }
        b = mix(b, s, u_post.diff_a.y);
    }

    // 泛光块（无 shader 门——源里这一段恒跑，关闭靠强度参数本身为 0；
    // 关闭时金字塔不更新、顶层按 1×1 黑图采样，tinted 精确为 0）。
    // 三段式：着色叠加（tinted）、overlay 混合（判别位在底色 b）、再叠
    // 一次 tinted。金字塔携带平方压缩值，先平方回线性。
    let bloom = textureSample(t_c, s_linear, in.uv).rgb;
    let bloom_sq = bloom * bloom;
    let tinted = u_post.bloom_a.x * bloom_sq;
    let blend = tinted * u_post.bloom_b.rgb + b;
    let overlaid = overlay(b, blend);
    b = u_post.bloom_a.y * (overlaid - b) + b;
    b = max(b, vec3<f32>(0.0));
    b = tinted * u_post.bloom_b.rgb + b;

    if u_post.flare_off.z > 0.5 {
        b = flare_layer(b, source_uv, u_post.flare_axis.xy, u_post.flare_axis.z, u_post.flare_axis.w,
            u_post.flare_off.xy, u_post.flare_c1, u_post.flare_c2);
    }
    if u_post.sun_off.z > 0.5 {
        b = flare_layer(b, source_uv, u_post.sun_axis.xy, u_post.sun_axis.z, u_post.sun_axis.w,
            u_post.sun_off.xy, u_post.sun_c1, u_post.sun_c2);
    }

    return vec4<f32>(max(b, vec3<f32>(0.0)), 1.0);
}
