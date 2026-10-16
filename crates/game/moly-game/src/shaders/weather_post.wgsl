// 天气后处理的全屏 pass 族：直拷（扩散预过滤）、4-tap 降采样、散射上
// 采样、合成（扩散混合 + 泛光 + 屏幕耀斑），以及泛光自己的三连（阈值
// 预滤波、平方域降采样、平方域上采样）。片元式逐条转写自产品链的着色
// 式：扩散的预滤波是直拷（没有阈值）、降采样 4-tap box、上采样低一级
// 4-tap box 后按散射权重混合；泛光的预滤波走阈值软膝曲线并把输出平方
// 压缩，金字塔两级都在平方域滤波；合成里扩散层先绕枢轴提对比再按混合
// 模式并入，泛光块按强度-着色-overlay 三段式并入，屏幕耀斑与太阳光晕
// 各沿自己的轴两条反向线性渐变各叠一层 hard light（先屏幕、后太阳）。
//
// 本链的输入是**引擎后处理之后**的相机色：引擎的调色查表与引擎泛光在
// 上游节点里已经做完（见 weather_stock_post.wgsl），这里不再调色。
//
// # 色彩域约定（与产品链同形）
//
// 产品的这条链是「gamma 直通」：场景缓冲里存的是编码值，合成开始时显式
// 解码进线性域、收尾显式编码回去。本管线的场景缓冲是 sRGB 纹理——采样时
// 硬件自动解码成线性（等价于链上那一步 sRGBToLinear）、写入时硬件自动编码
// 回去（等价于链尾 linearToSRGB），所以合成式里不再手写这两个往返；
// 直拷 pass 反而要**手动编码回存储域**再写金字塔，让金字塔与产品链一样
// 携带编码值（合成里扩散层拿的就是编码值，不二次解码）。
// 泛光金字塔不同：它的输入是自发光缓冲（半浮点、**存储域**——遮罩采样后
// 编回存储域，缓冲是 `Rgba16Float`、硬件不再编码），预滤波把存储域值开方
// 压缩后各级携带压缩值，合成里先平方回存储域再消费。
//
// 顶点用引擎的全屏三角形（3 顶点覆盖全屏），uv 即屏幕坐标，原点在左上、
// v 向下。真源的全屏 blit 原点在左下、v 向上（顶点程序把 uv 直接映成裁剪
// 坐标 uv·2−1），两个光晕的渐变轴按那个坐标定义 ⇒ 光晕一律用
// `source_uv = (u, 1 − v)`；只做纹理采样的地方与原点无关。

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

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

// 一层光晕：沿轴 `axis` 的两条反向线性渐变，各按 (偏移, 指数, 色的 alpha,
// 强度) 取权重，依次叠一层 hard light（先色1、后色2）。屏幕耀斑与太阳光晕
// 是同一个函数在两套 uniform 上各跑一次（真源程序里两段逐字同形）。
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

// 合成：基色（上游引擎后处理之后的相机色）clamp 到 100 进线性域（硬件解码
// 已经给出线性值），扩散 → 泛光 → 屏幕耀斑 → 太阳光晕（真源程序的段序），
// 收尾硬件编码回存储域。
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
