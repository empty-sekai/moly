// 天空渐变着色：逐式镜像 moly-law::weather::sky 的律（唯一真源在那边）。
// 网格法线朝内、三角绕序与法线一致，背面剔除下从内部看到的是正面；
// 网格不受光照：天空的颜色只来自渐变条与附加色。
//
// 域（这条链每一步的域，改这个文件之前先读）：
//   1 渐变条按非 sRGB 装载 ⇒ 硬件不解码 ⇒ 采样值就是存储（编码）域；
//   2 交叉淡化与附加项相加都在编码域里做——源天空着色程序全文没有一个
//     幂运算，它是 gamma 色彩空间构建，编码值直写帧缓冲；
//   3 本仓的色彩目标是 sRGB 格式，写出时硬件按 sRGB OETF 再编码一次
//     ⇒ 交给它的必须是解过一次的值，否则整片天空恒亮一档。
// ⇒ 这一路上唯一走 gamma↔linear 往返的是附加色，而它在真源里整段在
//   CPU 侧、在渐变条被采样之前就算完了（见 additive_property）。

#import bevy_pbr::forward_io::VertexOutput

@group(3) @binding(0) var<uniform> gradient_window: vec2<f32>; // (minY, maxY)
@group(3) @binding(1) var<uniform> additive_color: vec4<f32>;
@group(3) @binding(2) var<uniform> additive_params: vec2<f32>; // (intensity, fadeProgress)
@group(3) @binding(4) var ramp1: texture_2d<f32>;
@group(3) @binding(5) var ramp1_sampler: sampler;
@group(3) @binding(6) var ramp2: texture_2d<f32>;
@group(3) @binding(7) var ramp2_sampler: sampler;

// 引擎的 Mathf.GammaToLinearSpace：sRGB 分段解码，三支。
// 幂底数先钳到非负只为让 pow 有定义——被钳掉的那一段（x < 0）恒走第一支，
// 取值不受影响。
fn gamma_to_linear(x: vec3<f32>) -> vec3<f32> {
    let base = max(x, vec3<f32>(0.0));
    let mid = pow((base + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    let high = pow(base, vec3<f32>(2.2));
    let upper = select(mid, high, x >= vec3<f32>(1.0));
    return select(upper, x / 12.92, x <= vec3<f32>(0.04045));
}

// 引擎的 Mathf.LinearToGammaSpace：sRGB 分段编码，四支。指数照抄源里的
// 截断字面量（0.4166667 / 0.45454545），不写 1/2.4 与 1/2.2。
fn linear_to_gamma(x: vec3<f32>) -> vec3<f32> {
    let base = max(x, vec3<f32>(0.0));
    let mid = 1.055 * pow(base, vec3<f32>(0.4166667)) - vec3<f32>(0.055);
    let high = pow(base, vec3<f32>(0.45454545));
    let upper = select(mid, high, x >= vec3<f32>(1.0));
    let positive = select(upper, 12.92 * x, x <= vec3<f32>(0.0031308));
    return select(positive, vec3<f32>(0.0), x <= vec3<f32>(0.0));
}

// 天空材质属性块里 _AdditiveColor 的本帧取值：真源在 CPU 侧每帧算一次，
// 逐通道 LinearToGammaSpace(强度 × GammaToLinearSpace(底值))。⚠ 只有 r/g/b
// 三通道参与——真源那个 uniform 声明成三分量，底色的 alpha 从头到尾没被
// 读，写进属性块的颜色 alpha 是字面量 0。所以这里不碰 additive_color.a。
//
// 这一步在源里是每帧一次的 CPU 计算，这里逐片元重算：全部输入都是 uniform，
// 取值同一个。放在这里而不是喂入侧，是为了让这个着色器与律一一对上。
fn additive_property(color: vec3<f32>, intensity: f32) -> vec3<f32> {
    return linear_to_gamma(gamma_to_linear(color) * intensity);
}

// 交给 sRGB 色彩目标的值：按格式的 EOTF 解一次，硬件写出时那一次编码恰好
// 还原。⚠ 这一步不是真源的式子，是域适配——源是 gamma 色彩空间构建、
// 编码值直写帧缓冲，没有硬件编码那一步。UNORM 目标存不下 0..1 之外的值，
// 先钳与硬件后钳等价，顺带让幂底数为正。
fn srgb_target_value(encoded: vec3<f32>) -> vec3<f32> {
    let e = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // 导出几何按 glTF 的纵向纹理原点存 UV，与作者侧上下相反，所以 v = 1 - uv.y。
    let v = 1.0 - in.uv.y;
    // 窗口退化的分母下限只是坏数据不涂黑画面的保险；正常数据永远除不到它。
    let span = max(gradient_window.y - gradient_window.x, 1e-6);
    let t = clamp((v - gradient_window.x) / span, 0.0, 1.0);
    // 不做半纹素内缩：拿 u 直接采样，32×1 两端 clamp。
    let uv = vec2<f32>(1.0 - t, 0.5);
    // 换现象的交叉淡化：两张 ramp 同时在位按进度 mix；不切换时进度恒 0。
    let a = textureSample(ramp1, ramp1_sampler, uv).rgb;
    let b = textureSample(ramp2, ramp2_sampler, uv).rgb;
    let c = mix(a, b, clamp(additive_params.y, 0.0, 1.0));
    // 编码域里的一次普通相加（源就是 mix(ramp1, ramp2, 进度) + _AdditiveColor）；
    // 没有时间轴驱动时强度是 0，附加项整项给出恰 0。
    let encoded = c + additive_property(additive_color.rgb, additive_params.x);
    return vec4<f32>(srgb_target_value(encoded), 1.0);
}
