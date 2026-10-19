// 观众 avatar 着色：`Mysekai/Avatar` Base pass 的逐句翻译（式子全在
// moly-law 的 shading::avatar，本文件逐式镜像，槽序与 avatar_material.rs
// 的 Rust 结构体是契约，两边同改）。
//
// 与 `Mysekai/Character` 是两份不同的程序：没有 face/body 遮罩、球面距离场、
// 眉毛穿透那一族。Base 片元是一条 part-index 槽分派链：合并网格把多个 part
// 合成一个网格一份材质，靠顶点 UV0 的 z 分量区分哪片属于哪个 part。
//
// ⛔ 顶点 UV0.z（part-index）是数据，不是绘制结构。本仓的合成体 glb 没有
// z 分量（提取缺口：`CombineMesh` 没在提取侧把 part-index 烘进 UV0.z）。
// 这里照真源按「UV0.z 会在」写；喂数据那一侧在缺 z 分量时响亮拒绝，不拆绘制。
//
// 域（这条链每一步的域）：颜色贴图按 sRGB 装载 ⇒ 硬件采样时解码成线性 ⇒
// 采样后先 `srgb_format_encode` 编回存储（gamma）域，整条片元链在存储域上算
// （真源是 gamma 色彩空间构建，无幂运算）；写出前 `srgb_format_decode` 解一次
// 交给 sRGB 目标，硬件那一次编码恰好还原。uniform 颜色与顶点色不参与这两步
// （两侧同为原始浮点、本来就在存储域）；alpha 不参与（硬件 sRGB 只作用于 rgb）。
// 相机是 Tonemapping::None + DebandDither::Disabled ⇒ 材质程序写的就是显示
// 终值，域适配必须落在本着色器里。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::skinning
#import bevy_pbr::view_transformations::position_world_to_clip

// ---- 材质 uniform（group 3 binding 0）：每条 Unity 属性一个槽 ----

struct AvatarParams {
    // x: _SkinColor.r · y: g · z: b · w: a（调色，默认白）。
    skin_color: vec4<f32>,
    // x: _Alpha（alpha 增益底，默认 0）· y: _DitherAlpha（bayer discard 减数）·
    // z: _EnablePenlightLighting（penlight 总门，< 1 关）· w: 未用。
    alpha_dither_gate: vec4<f32>,
    // 左 penlight：x: _LeftPenlightActive · yzw: 未用。
    left_active: vec4<f32>,
    // 右 penlight：x: _RightPenlightActive。
    right_active: vec4<f32>,
    left_color: vec4<f32>,
    right_color: vec4<f32>,
    // (xyz 位置, w 强度)。
    left_param: vec4<f32>,
    right_param: vec4<f32>,
}

// ---- 全局量（group 3 binding 1）：一帧一份，全部 avatar 材质共用 ----

struct AvatarEnv {
    light_color: vec4<f32>,
    screen_params: vec4<f32>,
    mip_bias: vec4<f32>,
    projection_params: vec4<f32>,
}

@group(3) @binding(0) var<uniform> params: AvatarParams;
@group(3) @binding(1) var<uniform> env: AvatarEnv;
@group(3) @binding(2) var skin_tex: texture_2d<f32>;
@group(3) @binding(3) var skin_sampler: sampler;
@group(3) @binding(4) var accessory_tex: texture_2d<f32>;
@group(3) @binding(5) var accessory_sampler: sampler;
@group(3) @binding(6) var penlight_body_tex: texture_2d<f32>;
@group(3) @binding(7) var penlight_body_sampler: sampler;
@group(3) @binding(8) var penlight_light_tex: texture_2d<f32>;
@group(3) @binding(9) var penlight_light_sampler: sampler;

// ---- 顶点输出 ----

struct AvatarVertexOutput {
    @builtin(position) position: vec4<f32>,
    // xyz：世界坐标（penlight 距离场读它）。
    @location(0) world_position: vec3<f32>,
    // 主贴图坐标（源在顶点程序里直传 TEXCOORD0.xy，无 ST 变换）。
    @location(1) uv: vec2<f32>,
    // part-index：源顶点程序 `int(in_TEXCOORD0.z)`，flat 插值（逐顶点常量，
    // 不做插值）——合并网格按它分派贴图。本仓 glb 缺这个通道时喂入侧拒绝。
    @location(2) @interpolate(flat) part_index: i32,
    // 源的屏幕位置：xy = 0.5·(w + 分量)、z = clip.z、w = clip.w；bayer 表
    // 在片元里用它除 w 取像素坐标。
    @location(3) screen_pos: vec4<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> AvatarVertexOutput {
#ifdef SKINNED
    let world_from_local = skinning::skin_model(
        mesh.joint_indices,
        mesh.joint_weights,
        mesh.instance_index,
    );
#else
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
#endif
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(mesh.position, 1.0),
    );
    let clip = position_world_to_clip(world_position.xyz);

    var out: AvatarVertexOutput;
    out.position = clip;
    out.world_position = world_position.xyz;

#ifdef VERTEX_UVS_A
    out.uv = mesh.uv;
#else
    out.uv = vec2<f32>(0.0);
#endif

    // part-index：真源把它烘在顶点 UV0 的 z 分量（`int(in_TEXCOORD0.z)`），
    // 合并网格的单网格单材质靠它分派贴图槽。提取侧的产物把它烘成第二套 UV
    // （TEXCOORD_1）的 x 分量（Bevy 两套 UV 都是 Float32x2、3 分量会被整段
    // 丢弃，TEXCOORD_1.x 是唯一既装载得进又载得了值的载体）；消费端读作
    // vertex.uv_b（第二套 UV 顶点属性）。基座只含身体这一个 part ⇒ 全顶点
    // 恒身体槽（0），这是真源单 part 的语义，不是占位。
#ifdef VERTEX_UVS_B
    out.part_index = i32(round(mesh.uv_b.x));
#else
    // 无第二套 UV ⇒ 通道缺失（喂入侧已 fail-closed 拒绝，不会走到这）；
    // 保底钉身体槽，让着色器结构在任何变体下都有定义。
    out.part_index = 0;
#endif

    // 屏幕位置：y 先乘投影翻转位（本管线 GL 约定下恒 1），再取
    // 0.5·(w + 分量)，zw 原样直传——源程序的次序。
    // 源片元用 NDC = screen_pos.xy / w（除以随时间变化的 w），屏幕参数补回
    // 像素。wgsl @builtin(position) 就是 fragment 的像素坐标，同一量、无除法；
    // 直接喂像素坐标更稳（w=1 恒定）。两路在 bayer 表上逐值一致。
    let y_flipped = clip.y * env.projection_params.x;
    out.screen_pos = vec4<f32>(
        0.5 * clip.w + 0.5 * clip.x,
        0.5 * clip.w + 0.5 * y_flipped,
        clip.z,
        clip.w,
    );
    return out;
}

// ---- 片元共用件 ----

// 交给 sRGB 色彩目标的值：按格式的 EOTF 解一次，硬件写出时那一次编码恰好
// 还原。⚠ 这一步不是真源的式子，是域适配（同 sky_gradient.wgsl 那份，
// 逐式同 moly_law::weather::sky::srgb_target_value）。先钳到 0..1 再解。
fn srgb_target_value(encoded: vec3<f32>) -> vec3<f32> {
    let e = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

// sRGB 装载的贴图采样后编回存储域：硬件把存储（编码）字节解成线性，这里
// 再按 OETF 编回去，让整条片元链在存储域上算（真源的全链域）。
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let e = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = e * 12.92;
    let hi = 1.055 * pow(e, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, e <= vec3<f32>(0.0031308));
}

// 源经 one-hot 点积还原出的 4×4 抖动表，行是屏幕像素 y、列是 x。查表式：
// 屏幕位置 xy/w × _ScreenParams.xy × 0.25 取小数 × 4 即 mod 4；该域非负，
// 恒走源 fract 的正支路。与站点族/Character 同表同向。
fn bayer_value(screen_pos: vec4<f32>) -> f32 {
    let qx = screen_pos.x / screen_pos.w * env.screen_params.x * 0.25;
    let qy = screen_pos.y / screen_pos.w * env.screen_params.y * 0.25;
    let ix = i32(fract(abs(qx)) * 4.0);
    let iy = i32(fract(abs(qy)) * 4.0);
    var table = array<f32, 16>(
        0.0, 12.0, 3.0, 15.0,
        8.0, 4.0, 11.0, 7.0,
        2.0, 14.0, 1.0, 13.0,
        10.0, 6.0, 9.0, 5.0,
    );
    let value = table[iy * 4 + ix];
    return value * 0.0618750006 + 0.00999999978;
}

// alpha 增益：源对 skin/accessory/penlight_body 三支同式——
// albedo = (1-albedo) * ((-A)*A + 1) + albedo。A=_Alpha。式对任意 albedo
// 在增益 1 时恒白（(1-a)+a=1），是软提亮不是恒等。
fn alpha_gain(albedo: vec3<f32>, alpha: f32) -> vec3<f32> {
    let gain = (-alpha) * alpha + 1.0;
    return (vec3<f32>(1.0) - albedo) * gain + albedo;
}

// 一支 penlight 的距离衰减权重：(1 - min(d * 2.85714293, 1))² * w * active。
fn penlight_weight(world: vec3<f32>, param: vec4<f32>, is_active: f32) -> f32 {
    let d = distance(world, param.xyz);
    let t = 1.0 - min(d * 2.85714293, 1.0);
    return t * t * param.w * is_active;
}

@fragment
fn fragment(in: AvatarVertexOutput) -> @location(0) vec4<f32> {
    let bias = env.mip_bias.x;
    // The part varies across fragments. Compute derivatives before that branch
    // and retain the mip bias by scaling the gradients.
    let uv_dx = dpdx(in.uv) * exp2(bias);
    let uv_dy = dpdy(in.uv) * exp2(bias);
    let alpha = params.alpha_dither_gate.x;

    // 槽分派：按 part-index 选贴图与调色（源是一条 if/else 链）。贴图按
    // sRGB 装载 ⇒ 采样值是线性 ⇒ 编回存储域再进链。每支只在命中时采样。
    var albedo = vec3<f32>(1.0);
    if in.part_index == 0 {
        let s = textureSampleGrad(skin_tex, skin_sampler, in.uv, uv_dx, uv_dy);
        let s_rgb = srgb_format_encode(s.rgb);
        // base = tex.a * (skin_color.rgb - tex.rgb) + tex.rgb，再过 alpha 增益。
        let base = s.a * (params.skin_color.rgb - s_rgb) + s_rgb;
        albedo = alpha_gain(base, alpha);
    } else if in.part_index == 1 {
        let t = textureSampleGrad(accessory_tex, accessory_sampler, in.uv, uv_dx, uv_dy);
        albedo = alpha_gain(srgb_format_encode(t.rgb), alpha);
    } else if in.part_index == 2 || in.part_index == 4 {
        let t = textureSampleGrad(penlight_body_tex, penlight_body_sampler, in.uv, uv_dx, uv_dy);
        albedo = alpha_gain(srgb_format_encode(t.rgb), alpha);
    } else if in.part_index == 3 || in.part_index == 5 {
        let t = textureSampleGrad(penlight_light_tex, penlight_light_sampler, in.uv, uv_dx, uv_dy);
        let t_rgb = srgb_format_encode(t.rgb);
        let pen = select(params.right_color, params.left_color, in.part_index == 3);
        // tex.rgb * (tex.a * (pen.rgb - 1) + 1)。
        let tint = t.a * (pen.rgb - vec3<f32>(1.0)) + vec3<f32>(1.0);
        albedo = t_rgb * tint;
    }
    // 其余 part_index 落默认白（albedo 初值）。

    // penlight 叠加：part 属于 penlight（index ∉ {0,1}）且总门开时才叠加；
    // 否则基色直通。源对全部 part 都算了距离场，但非 penlight part 的输出
    // 在门后被丢——这里按门的语义先判门（门的两条件取或）。
    let is_penlight_part = in.part_index != 0 && in.part_index != 1;
    var shaded = albedo;
    if is_penlight_part && params.alpha_dither_gate.z >= 1.0 {
        let lw = penlight_weight(in.world_position, params.left_param, params.left_active.x);
        let rw = penlight_weight(in.world_position, params.right_param, params.right_active.x) * 0.5;
        shaded = albedo + lw * params.left_color.rgb + rw * params.right_color.rgb;
    }

    // bayer 抖动 discard：`_DitherAlpha` 减阈值低于零丢弃。本程序无
    // `_UseDither` 门（与 Character 不同），整支无条件执行。
    if params.alpha_dither_gate.y - bayer_value(in.screen_pos) < 0.0 {
        discard;
    }

    // 光混：light.w 是混合因子，lit = light.w*(albedo*light.rgb - albedo)+albedo。
    let lc = env.light_color;
    let rgb = lc.w * (shaded * lc.rgb - shaded) + shaded;

    // 写出：存储域的值解一次交给 sRGB 目标，硬件那一次编码恰好还原。
    return vec4<f32>(srgb_target_value(rgb), 1.0);
}
