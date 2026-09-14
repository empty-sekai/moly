// 角色 toon 程序的逐句翻译：mask switch → 双支因子 → shade 混 → 光混 →
// 抖动 → 眉 → 雾 → brightness。式子照 moly-law 的角色律；槽序与
// `character_material.rs` 的 Rust 结构体是契约，两边同改。
//
// 变体键（specialize 注入）：
//   CHARACTER_EYEBROW   材质带 `_EyebrowTex` 槽：眉采样、discard 与输出
//                       alpha 覆写。眉标量三槽材质都带、只有纹理槽在场
//                       才开变体（已裁）。
//   CHARACTER_FOG       全局量桥的雾档案在场：顶点算雾斜坡、片元走
//                       高度雾。
// mask 的槽 switch 不做编译期分派：源是同一片元程序按材质 uniform
// 分支，这里照原形运行时分派。
//
// 源程序写两个颜色目标（SV_Target1 = 0）；本管线的主 pass 只有一个
// 颜色目标，第二目标不在此列。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::skinning
#import bevy_pbr::view_transformations::position_world_to_clip

// ---- 材质 uniform（group 3 binding 0）：每条 Unity 属性一个槽 ----

struct CharacterParams {
    // x: _CharacterShaderUsage · y: _EyebrowClip · z: _EyebrowAlpha ·
    // w: _UseDither。
    usage_and_eyebrow: vec4<f32>,
    brightness: vec4<f32>,
    override_shading_parameter: vec4<f32>,
    local_body_shading_intensity: vec4<f32>,
    local_body_shading_edge_threshold: vec4<f32>,
    local_body_shading_edge_smoothness: vec4<f32>,
    dither_alpha: vec4<f32>,
    // _MainTex 的 (scaleX, scaleY, offsetX, offsetY)。
    main_tex_st: vec4<f32>,
    // 头参考点：头骨世界位（xyz）；源只吃 .xz（距离场）。
    head_position: vec4<f32>,
}

// ---- 全局量（group 3 binding 1）：一帧一份，全部角色材质共用 ----

struct CharacterEnv {
    light_vector: vec4<f32>,
    light_color: vec4<f32>,
    skin_shade_color: vec4<f32>,
    body_shade_color: vec4<f32>,
    // x: 球面支 edge · y: 球面支 smoothness。
    sphere_edge_and_smoothness: vec4<f32>,
    screen_params: vec4<f32>,
    mip_bias: vec4<f32>,
    fog_params: vec4<f32>,
    fog_near_color: vec4<f32>,
    fog_far_color: vec4<f32>,
    projection_params: vec4<f32>,
}

@group(3) @binding(0) var<uniform> params: CharacterParams;
@group(3) @binding(1) var<uniform> env: CharacterEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;
@group(3) @binding(4) var body_mask_tex: texture_2d<f32>;
@group(3) @binding(5) var body_mask_sampler: sampler;
@group(3) @binding(6) var eyebrow_tex: texture_2d<f32>;
@group(3) @binding(7) var eyebrow_sampler: sampler;

// ---- 顶点输出 ----

struct CharacterVertexOutput {
    @builtin(position) position: vec4<f32>,
    // xyz：世界坐标（球面距离场与高度雾都读它）。
    @location(0) world_position: vec4<f32>,
    // 顶点里归一化过的世界法线；片元里再归一化一次（源的形状）。
    @location(1) world_normal: vec3<f32>,
    // _MainTex_ST 变换后的主贴图坐标（源在顶点程序里做变换）。
    @location(2) uv: vec2<f32>,
    // 源的屏幕位置：xy = 0.5·(w + 分量)、z = clip.z、w = clip.w；
    // 抖动表在片元里用它除 w 取像素坐标。
    @location(3) screen_pos: vec4<f32>,
    // 雾斜坡：顶点里算好。只有加雾变体有这一段。
    @location(4) fog_ramp: f32,
}

@vertex
fn vertex(mesh: Vertex) -> CharacterVertexOutput {
    // 蒙皮：角色网格带关节权重——关节矩阵混合替代实例矩阵（引擎标准
    // 顶点路径同款 ifdef）。缺这支则三网格回绑定姿势（T-pose + 脸部
    // 网格塌位），静态网格的站点材质范式没有此支不等于角色也没有。
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

    var out: CharacterVertexOutput;
    out.position = clip;
    out.world_position = world_position;

#ifdef VERTEX_NORMALS
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, mesh.normal);
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        mesh.normal,
        mesh.instance_index,
    );
#endif
#else
    out.world_normal = vec3<f32>(0.0);
#endif

#ifdef VERTEX_UVS_A
    out.uv = mesh.uv * params.main_tex_st.xy + params.main_tex_st.zw;
#else
    out.uv = vec2<f32>(0.0);
#endif

    // 屏幕位置：y 先乘投影翻转位（本管线 GL 约定下恒 1），再取
    // 0.5·(w + 分量)，zw 原样直传——源程序的次序。
    let y_flipped = clip.y * env.projection_params.x;
    out.screen_pos = vec4<f32>(
        0.5 * clip.w + 0.5 * clip.x,
        0.5 * clip.w + 0.5 * y_flipped,
        clip.z,
        clip.w,
    );

#ifdef CHARACTER_FOG
    // 距离雾因子：源式 ((clip.z + n)/(n + f))·f 读 GL z 约定，在该约定下
    // 恰等于 f·(clip.w − n)/(f − n)——用 clip.w（眼空间深度，两种约定同
    // 值）重建，不依赖本管线的 z 约定。max 0 在缩放偏置之前。
    let n = env.projection_params.y;
    let f = env.projection_params.z;
    var fog_depth = f * (clip.w - n) / (f - n);
    fog_depth = max(fog_depth, 0.0);
    fog_depth = fog_depth * env.fog_params.x + env.fog_params.y;
    out.fog_ramp = clamp(fog_depth, 0.0, 1.0);
#else
    out.fog_ramp = 0.0;
#endif

    return out;
}

// ---- 片元共用件 ----

// 平滑支共用的 x²(3 − 2x)，按源的两步乘加次序。
fn smoothstep_poly(x: f32) -> f32 {
    let t = x * -2.0 + 3.0;
    return x * x * t;
}

// 源经 one-hot 点积还原出的 4×4 抖动表，行是屏幕像素 y（自下而上）、
// 列是 x。查表式：屏幕位置 xy/w × _ScreenParams.xy × 0.25 取小数 × 4
// 即 mod 4；该域非负，恒走源 fract 的正支路。
fn bayer_value(screen_pos: vec4<f32>) -> f32 {
    let qx = screen_pos.x / screen_pos.w * env.screen_params.x * 0.25;
    let qy = screen_pos.y / screen_pos.w * env.screen_params.y * 0.25;
    let ix = i32(fract(qx) * 4.0);
    let iy = i32(fract(qy) * 4.0);
    var table = array<f32, 16>(
        0.0, 12.0, 3.0, 15.0,
        8.0, 4.0, 11.0, 7.0,
        2.0, 14.0, 1.0, 13.0,
        10.0, 6.0, 9.0, 5.0,
    );
    let value = table[iy * 4 + ix];
    return value * 0.0618750006 + 0.00999999978;
}

// 距离雾插值之后的高度雾，按源程序的次序：雾色四通道 clamp 后，高度
// 项是 fog_colour.w · min(exp2(−y·z), 1)，min 夹在 exp2 上、乘的是插值
// 后的雾色 alpha，两者都不可交换改写。
fn apply_fog(rgb: vec3<f32>, world_y: f32, fog_ramp: f32) -> vec3<f32> {
    let fog_color = clamp(
        fog_ramp * (env.fog_near_color - env.fog_far_color) + env.fog_far_color,
        vec4<f32>(0.0),
        vec4<f32>(1.0),
    );
    let distance_blended = fog_ramp * (rgb - fog_color.rgb) + fog_color.rgb;
    let delta = distance_blended - rgb;
    let height = min(exp2(-world_y * env.fog_params.z), 1.0) * fog_color.w;
    return clamp(height * delta + rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(in: CharacterVertexOutput) -> @location(0) vec4<f32> {
    // 主贴图带全局 mip 偏置采样（源的全局量，整支管线的采样都偏置）。
    let bias = env.mip_bias.x;
    let albedo = textureSampleBias(main_tex, main_sampler, in.uv, bias);

    // 抖动 discard：先 _UseDither 门，再 _DitherAlpha 减阈值低于零丢弃。
    if params.usage_and_eyebrow.w > 0.0 {
        if params.dither_alpha.x - bayer_value(in.screen_pos) < 0.0 {
            discard;
        }
    }

    // 眉纹理 r；discard 门槛低于零丢弃，恰等于门槛不丢。
#ifdef CHARACTER_EYEBROW
    let eyebrow_r = textureSampleBias(eyebrow_tex, eyebrow_sampler, in.uv, bias).r;
    if eyebrow_r - params.usage_and_eyebrow.y < 0.0 {
        discard;
    }
#endif

    // body 遮罩采样无条件执行（源同形）；face 槽的采样值被 switch 钉死、
    // 不进有效支路。
    let body_mask = textureSampleBias(body_mask_tex, body_mask_sampler, in.uv, bias).xy;

    // mask 的槽 switch：1 = body 槽透传遮罩、0 = face 槽钉成 (1, 0)、
    // 其余值落默认 (0, 0)。
    var mask = vec2<f32>(0.0, 0.0);
    if params.usage_and_eyebrow.x == 1.0 {
        mask = body_mask;
    } else if params.usage_and_eyebrow.x == 0.0 {
        mask = vec2<f32>(1.0, 0.0);
    }

    // shade 色：lerp(skin, body, 1 − mask.x)，四通道一起插值——对
    // mask.x 连续；不连续的只有下面的因子式（分界线的落点）。
    let shade = env.skin_shade_color
        + (1.0 - mask.x) * (env.body_shade_color - env.skin_shade_color);

    // body 支因子：直取 dot（无 half-Lambert），先减 mask.y 再夹 [0,1]。
    let normal = normalize(in.world_normal);
    let ndl = dot(normal, env.light_vector.xyz);
    let lambert = clamp(ndl - mask.y, 0.0, 1.0);
    // override 门选材质局部三元组或全局缺省 (1, 0, 0)。
    let use_local = params.override_shading_parameter.x > 0.5;
    let intensity = select(1.0, params.local_body_shading_intensity.x, use_local);
    let threshold = select(0.0, params.local_body_shading_edge_threshold.x, use_local);
    let smoothness = select(0.0, params.local_body_shading_edge_smoothness.x, use_local);
    let upper = threshold + smoothness;
    let lower = threshold - smoothness;
    // 硬阈值支：smoothness 低于门限走它（threshold ≥ lambert ? 1 : 0）。
    var body_factor = intensity * select(0.0, 1.0, threshold >= lambert);
    if smoothness >= 0.00400000019 {
        // 平滑支：源先取分母倒数再相乘，不写除法。
        let inv_span = 1.0 / (lower - upper);
        let x = clamp((lambert - upper) * inv_span, 0.0, 1.0);
        body_factor = intensity * smoothstep_poly(x);
    }

    // 球面支因子：头周 XZ 距离场，normalize 无地板（头正上方是源自带的
    // NaN 缺口）；光源 .xz 不归一化。edge/smoothness 是引擎全局量
    // （读不出值的具名替身，见 Rust 侧注释）。
    let dx = in.world_position.x - params.head_position.x;
    let dz = in.world_position.z - params.head_position.z;
    let inv = inverseSqrt(dx * dx + dz * dz);
    let sdot = dx * inv * env.light_vector.x + dz * inv * env.light_vector.z;
    let sphere_edge = env.sphere_edge_and_smoothness.x;
    let sphere_smoothness = env.sphere_edge_and_smoothness.y;
    let sphere_inv_span = 1.0 / (sphere_smoothness * -2.0);
    let sx = clamp(
        (sdot - sphere_edge - sphere_smoothness) * sphere_inv_span,
        0.0,
        1.0,
    );
    let sphere_factor = smoothstep_poly(sx);

    // regime 选择：mask.x < 0.5 走 body、否则走球面——因子在 0.5 处无
    // 过渡带，这是分界线的机制。
    let factor = select(body_factor, sphere_factor, mask.x >= 0.5);

    // 光混：color.w 是混合因子。
    let lit = albedo.rgb
        + env.light_color.w * (albedo.rgb * env.light_color.rgb - albedo.rgb);

    // 着色混：源先乘 shade 色的 alpha 再混合。
    let k = shade.w * factor;
    var rgb = lit + k * (lit * shade.rgb - lit);

#ifdef CHARACTER_FOG
    rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);
#endif

    // 输出 alpha：眉变体覆写成 albedo.a × 眉纹理 r × _EyebrowAlpha，
    // 其余变体直通 albedo.a。
    var alpha = albedo.a;
#ifdef CHARACTER_EYEBROW
    alpha = albedo.a * eyebrow_r * params.usage_and_eyebrow.z;
#endif

    // brightness 统一乘 rgb 与 alpha 两边。
    let brightness = params.brightness.x;
    return vec4<f32>(rgb * brightness, alpha * brightness);
}
