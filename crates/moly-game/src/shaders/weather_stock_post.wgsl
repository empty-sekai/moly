// 引擎原生后处理：引擎泛光（预滤波、横向 9 点、纵向 5 点、散射上采样）与
// 引擎 uber（场景色 + 泛光顶层，乘曝光、钳到 0..1、查 LDR 表）。片元式逐条
// 转写自随包发布的引擎泛光与引擎 uber 程序（gamma 色彩空间构建），常数照抄
// 程序里的字面量。
//
// # 色彩域
//
// 真源的相机目标是 UNorm、存编码值：泛光预滤波**直接在编码值上**取阈值
// （程序里没有解码），金字塔存工作值的平方根、各级读出来先平方；引擎 uber
// 先把场景色解码进线性域、加泛光、查表，再编码写回。本管线的场景缓冲是
// sRGB 纹理：采样即解码、写入即编码，所以 uber 里不手写这两步；预滤波要的
// 是编码值，就逐纹素读出再编码回去，并在编码值上自己做双线性（真源硬件
// 是在 UNorm 编码值上插值的）。uber 写回用的是硬件编码：程序的线性段斜率
// 是 12.9232101 而不是 12.92，两者在 8 位存储上的差不到半个量化级。
//
// 纹理原点：本管线全屏三角形的 uv 原点在左上、真源在左下。这里每一步要么
// 只做采样、要么核关于纵轴对称（纵向 5 点），所以不需要翻转；查找表的行
// 顺序与真源同为「第 0 行 = v 0」，上传不翻行、取表坐标照抄。

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct StockPostUniform {
    // 预滤波与上采样：(Lerp(0.05, 0.95, scatter), clamp, 线性阈值, 软膝)。
    prefilter: vec4<f32>,
    // 引擎 uber：(泛光强度, tint′.rgb)；泛光关着时强度 0、顶层是黑图。
    bloom: vec4<f32>,
    // 查表：(1/宽, 1/高, 高 − 1, 2^postExposure)。
    lut: vec4<f32>,
};

@group(0) @binding(0) var t_a: texture_2d<f32>;
@group(0) @binding(1) var t_b: texture_2d<f32>;
@group(0) @binding(2) var s_linear: sampler;
@group(0) @binding(3) var<uniform> u: StockPostUniform;
@group(0) @binding(4) var t_lut: texture_2d<f32>;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let hi = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) * 1.055 - 0.055;
    return mix(hi, c * 12.92, step(c, vec3<f32>(0.0031308)));
}

// 一个场景纹素的编码值（钳边）。
fn load_encoded(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(t_a));
    let q = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    return linear_to_srgb(textureLoad(t_a, q, 0).rgb);
}

// 编码值上的双线性（纹素中心在 (i + 0.5) / 尺寸，钳边）。
fn sample_encoded(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(t_a));
    let p = uv * size - vec2<f32>(0.5);
    let i = floor(p);
    let f = p - i;
    let i0 = vec2<i32>(i);
    let a = load_encoded(i0);
    let b = load_encoded(i0 + vec2<i32>(1, 0));
    let c = load_encoded(i0 + vec2<i32>(0, 1));
    let d = load_encoded(i0 + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// 预滤波：钳亮部 → 最大通道作亮度 → 软膝阈值权重 → 乘回原色、负值抹平 →
// 开方存储。
@fragment
fn stock_bloom_prefilter(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = u.prefilter;
    var c = min(sample_encoded(in.uv), vec3<f32>(p.y));
    let brightness = max(c.b, max(c.g, c.r));
    let over = brightness - p.z;
    var softness = min(p.w + p.w, max(over + p.w, 0.0));
    softness = softness * softness / (p.w * 4.0 + 9.99999975e-05);
    let multiplier = max(softness, over) / max(brightness, 9.99999975e-05);
    c = max(vec3<f32>(multiplier) * c, vec3<f32>(0.0));
    return vec4<f32>(sqrt(c), 1.0);
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    let s = textureSample(t_a, s_linear, uv).rgb;
    return s * s;
}

// 横向 9 点高斯，同时降一半：偏移是**源**（上一级，宽是本级两倍）纹素宽的
// 2、4、6、8 倍，平方域里加权、开方存储。
@fragment
fn stock_bloom_blur_h(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / f32(textureDimensions(t_a).x);
    let o2 = vec2<f32>(t * 2.0, 0.0);
    let o4 = vec2<f32>(t * 4.0, 0.0);
    let o6 = vec2<f32>(t * 6.0, 0.0);
    let o8 = vec2<f32>(t * 8.0, 0.0);
    var acc = tap(in.uv - o8) * 0.0162162203 + tap(in.uv - o6) * 0.0540540516;
    acc = tap(in.uv - o4) * 0.121621624 + acc;
    acc = tap(in.uv - o2) * 0.194594592 + acc;
    acc = tap(in.uv) * 0.227027029 + acc;
    acc = tap(in.uv + o2) * 0.194594592 + acc;
    acc = tap(in.uv + o4) * 0.121621624 + acc;
    acc = tap(in.uv + o6) * 0.0540540516 + acc;
    acc = tap(in.uv + o8) * 0.0162162203 + acc;
    return vec4<f32>(sqrt(acc), 1.0);
}

// 纵向：5 个双线性抽头等效 9 点高斯（同尺寸源，偏移 1.38461542 与
// 3.23076916 个纹素高）。
@fragment
fn stock_bloom_blur_v(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / f32(textureDimensions(t_a).y);
    let near = vec2<f32>(0.0, t * 1.38461542);
    let far = vec2<f32>(0.0, t * 3.23076916);
    var acc = tap(in.uv - far) * 0.0702702701 + tap(in.uv - near) * 0.31621623;
    acc = tap(in.uv) * 0.227027029 + acc;
    acc = tap(in.uv + near) * 0.31621623 + acc;
    acc = tap(in.uv + far) * 0.0702702701 + acc;
    return vec4<f32>(sqrt(acc), 1.0);
}

// 上采样：本级（binding 0）与低一级（binding 1）在平方域按散射权重插值。
@fragment
fn stock_bloom_upsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let low = textureSample(t_b, s_linear, in.uv).rgb;
    let high = textureSample(t_a, s_linear, in.uv).rgb;
    let h2 = high * high;
    let r = u.prefilter.x * (low * low - h2) + h2;
    return vec4<f32>(sqrt(r), 1.0);
}

// 引擎 uber 的查表（LDR 支路）：乘曝光、钳到 0..1、按条带坐标取两次样、
// 沿蓝插值。
fn apply_lut(c_in: vec3<f32>) -> vec3<f32> {
    let lp = u.lut;
    let c = clamp(c_in * lp.w, vec3<f32>(0.0), vec3<f32>(1.0));
    let s = c.zxy * lp.z;
    let shift = floor(s.x);
    let half_texel = lp.xy * 0.5;
    let uy = s.y * lp.x + half_texel.x;
    let vz = s.z * lp.y + half_texel.y;
    let ux = shift * lp.y + uy;
    let a = textureSampleLevel(t_lut, s_linear, vec2<f32>(ux, vz), 0.0).rgb;
    let b = textureSampleLevel(t_lut, s_linear, vec2<f32>(ux + lp.y, vz), 0.0).rgb;
    let t = c.z * lp.z - shift;
    return vec3<f32>(t) * (b - a) + a;
}

// 引擎 uber：场景色（采样即解码）+ 泛光顶层平方 × 强度 × tint′ → 查表 →
// 硬件编码写回。暗角强度 0、无用户表：两段在程序里被各自的参数关掉。
@fragment
fn stock_uber(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c = textureSample(t_a, s_linear, in.uv).rgb;
    let bloom = textureSample(t_b, s_linear, in.uv).rgb;
    c = (bloom * bloom * u.bloom.x) * u.bloom.yzw + c;
    return vec4<f32>(apply_lut(c), 1.0);
}
