// 家具 Basic 族的第二颜色目标（自发光 pass 的着色程序）。
//
// 源 Base 程序同时写两个颜色目标：Target0 是主色（由
// `fixture_material.wgsl` 承接），Target1 是自发射遮罩 × 现象自发光
// 开关链。本管线的主 pass 只有一个颜色目标（与站点族同一裁决），这条
// 第二目标由自建 pass 直画进泛光的输入缓冲——深度测试复用主 pass 的
// 深度图（只读不写），混合因子随材质（不透明 One/Zero、混合
// SrcAlpha/OneMinusSrcAlpha，单一 _SrcBlend/_DstBlend 对两个目标同因子）。
//
// # 片元式（源第二目标支路逐句）
//
// 主贴图采样（带全局 mip 偏置）→ 遮罩采样（同 uv 同偏置，只取 xyz）→
// （alpha clip 变体：主贴图 alpha − 0.5 < 0 丢弃）→ Bayer 抖动
// （`ditherAlpha − bayer < 0` 丢弃）→ 现象自发光门 → 输出
// `vec4(mask.rgb × 门, 主贴图 alpha)`。两个采样都在两条 discard 之前
// （源的求值序）。
//
// 现象自发光门（源的全局 int `_MysekaiPhenomenaEmissionType` 与材质两个
// int 的开关链）：类型 1 时门 = bright==1、类型 2 时门 = dark==1、其余 0。
// Basic material defaults follow the phenomenon. Animation events also write
// override mode 1 (force on) and 2 (force off), carried in emission.w.
//
// # 与主 pass 的两点已知同差（同站点族的既有裁决）
//
// - 抖动屏幕坐标走片元内置坐标（源走 clip 位与屏参乘 0.125）；两者在
//   像素中心逐值相等，主 pass 已按同一对账采同一路。
// - 深度测试 GreaterEqual 不写（主 pass 是 reverse-Z、GreaterEqual 写）；
//   本 pass 在主 pass 之后读同一张深度图，不改动它。
//
// # 绑定形状
//
// group 0：逐对象 uniform（动态 offset）——世界矩阵 + 材质参数 12 槽 +
// 自发光两 int，以及主 pass 实际读取的视图 uniform（独立动态 offset）。
// group 1：全局量表（与站点/家具材质共用同一个 buffer，
// 槽序契约见 `SiteEnv`）、遮罩贴图、主贴图、采样器（钳边三线性，与
// 家具主材质同一组参数；glb 里两张纹理共用同一个采样器条目）。

#import bevy_render::view::View

// ---- 逐对象 uniform（group 0 binding 0）----

// 材质参数 12 槽：槽序与 `fixture_material.wgsl` 的 `FixtureParams` 是
// 同一份契约（两处同改）。本 pass 只消费 main_tex_offset / dither_alpha /
// uv_v_flip / 窗门四角；其余槽照带（同一块 uniform 两 pass 共用形状）。
struct FixtureParams {
    main_tex_offset: vec4<f32>,
    dither_alpha: vec4<f32>,
    use_phenomena_lighting: vec4<f32>,
    override_shading_parameter: vec4<f32>,
    local_shading_intensity: vec4<f32>,
    local_edge_threshold: vec4<f32>,
    local_edge_smoothness: vec4<f32>,
    uv_v_flip: vec4<f32>,
    clip_corner_0: vec4<f32>,
    clip_corner_1: vec4<f32>,
    clip_corner_2: vec4<f32>,
    clip_corner_3: vec4<f32>,
}

struct EmissionObject {
    // 与主材质相同的世界矩阵；不要在 CPU 预乘为裁剪矩阵。
    world_from_local: mat4x4<f32>,
    params: FixtureParams,
    // x = bright、y = dark（两个材质 int 的 f32 形，与 1.0 比较相等判门）。
    emission: vec4<f32>,
    // x is the main renderer's joint offset (storage-buffer backend only).
    skin: vec4<u32>,
}

#ifdef SKINNED
#ifdef SKINS_USE_UNIFORM_BUFFERS
#import bevy_pbr::mesh_types::SkinnedMesh
@group(0) @binding(2) var<uniform> joint_matrices: SkinnedMesh;
#else
@group(0) @binding(2) var<storage> joint_matrices: array<mat4x4<f32>>;
#endif

fn emission_skin_model(indexes: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    // Same buffer, four weighted matrices and addition order as Bevy's
    // skinning::skin_model, rebound for this second-colour-target pass.
#ifdef SKINS_USE_UNIFORM_BUFFERS
    return weights.x * joint_matrices.data[indexes.x]
        + weights.y * joint_matrices.data[indexes.y]
        + weights.z * joint_matrices.data[indexes.z]
        + weights.w * joint_matrices.data[indexes.w];
#else
    let base = object.skin.x;
    return weights.x * joint_matrices[base + indexes.x]
        + weights.y * joint_matrices[base + indexes.y]
        + weights.z * joint_matrices[base + indexes.z]
        + weights.w * joint_matrices[base + indexes.w];
#endif
}
#endif

// ---- 全局量（group 1 binding 0）：与站点/家具材质共用同一个 buffer ----
// 槽序与 `env.rs` 的 gpu_bytes 摊平序是契约，两边同改；本 pass 只读
// mip_bias（两个采样）与 emission_type（门链）。

struct SiteEnv {
    light_vector: vec4<f32>,
    phenomena_light_color: vec4<f32>,
    phenomena_shade_color: vec4<f32>,
    drop_shadow_color: vec4<f32>,
    edge_threshold: vec4<f32>,
    edge_smoothness: vec4<f32>,
    treasure_position_0: vec4<f32>,
    treasure_position_1: vec4<f32>,
    treasure_shadow_intensity: vec4<f32>,
    camera_position: vec4<f32>,
    ortho_params: vec4<f32>,
    view_matrix_c0: vec4<f32>,
    view_matrix_c1: vec4<f32>,
    view_matrix_c2: vec4<f32>,
    view_matrix_c3: vec4<f32>,
    screen_params: vec4<f32>,
    fog_params: vec4<f32>,
    fog_near_color: vec4<f32>,
    fog_far_color: vec4<f32>,
    projection_params: vec4<f32>,
    mip_bias: vec4<f32>,
    time: vec4<f32>,
    // 现象自发光类型（全局 int，f32 存整数值；1 = 日系、2 = 夜系、0 = 全关）。
    emission_type: vec4<f32>,
}

@group(0) @binding(0) var<uniform> object: EmissionObject;
@group(0) @binding(1) var<uniform> main_view: View;
@group(1) @binding(0) var<uniform> env: SiteEnv;
@group(1) @binding(1) var mask_tex: texture_2d<f32>;
@group(1) @binding(2) var main_tex: texture_2d<f32>;
@group(1) @binding(3) var main_sampler: sampler;
@group(1) @binding(4) var mask_sampler: sampler;

// ---- 色彩域（改这个 pass 的输出之前先读）----
//
// 真源是 **gamma 色彩空间**构建：遮罩采样不解码，`遮罩 × 门` 按编码值
// 直写 Target1；那张缓冲在 gamma 工程里不做任何 sRGB 变换 ⇒ **源的 Target1
// 携带的是存储（gamma）域的值**，泛光预滤波读到的就是它。
//
// 本 pass 两头与主 pass **不同**，别照 `fixture_material.wgsl` 套：
//
//   1 遮罩贴图按 sRGB 装载 ⇒ 采样时硬件解码 ⇒ 要 [`srgb_format_encode`]
//     编回存储域，才等于源那次不解码的采样；
//   2 **本 pass 的目标是 `Rgba16Float`**（`fixture_emission.rs` 的
//     `EMISSION_FORMAT`）⇒ **硬件不做 sRGB 编码** ⇒ 写出侧**没有**要抵消
//     的东西，**不要**在这里套主 pass 那个 `srgb_format_decode`。
//     写进去的值原样留在缓冲里 = 源的 Target1。
//
// ⇒ 于是缓冲携带存储域值，而泛光预滤波的阈值是 `GammaToLinearSpace(threshold)`
//   ——那个 gamma→linear **是真源自己做的**（游戏自带的泛光 pass 在装配侧
//   `SetVector(scatter', clamp, GammaToLinearSpace(threshold), 同值×0.5)`，
//   律侧 `moly_law::weather` 的打包逐项同形）。**它不是我方为补偿域错加的
//   转换**，所以本次收口**不动**它：存储域的输入配线性化的阈值，正是源那条
//   链自带的怪癖，照抄才对上。谁要把那个 g2l 当成「我方的 bug」删掉，
//   泛光的门限会整体搬家。
//
// ⚠ 主贴图这次采样**只取 `.a`**：硬件的 sRGB 变换不作用于 alpha，采样得到
//   的 `.a` 就是存储值 ⇒ 它不需要、也不能套编码。

// 采样值 → 存储域：抵消硬件的 sRGB 解码（格式 OETF）。与主 pass 同一支；
// **格式**那一族（IEC 61966-2-1），不是引擎的 `Mathf.LinearToGammaSpace`
// （后者多一支 2.2 幂，在 `moly_law::weather::sky` 里叫 `linear_to_gamma`，
// 只用在源自己做 gamma 往返的地方）。
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}

// ---- 顶点 ----

struct EmissionVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
#ifdef SKINNED
    @location(2) joint_indices: vec4<u32>,
    @location(3) joint_weights: vec4<f32>,
#endif
}

struct EmissionVertexOutput {
    @builtin(position) position: vec4<f32>,
    // uv0 + (_MainTexOffsetX, _MainTexOffsetY)，脸族 v 翻转同主 pass。
    @location(0) uv: vec2<f32>,
#ifdef FIXTURE_WINDOW_CLIP
    // 窗外观门四角 NDC 与判别位：语义同主 pass（写零面平面凸四边形，
    // 源以模板 Equal 只画窗洞内；这里同一条屏空间门）。
    @location(1) clip_corner_0: vec2<f32>,
    @location(2) clip_corner_1: vec2<f32>,
    @location(3) clip_corner_2: vec2<f32>,
    @location(4) clip_corner_3: vec2<f32>,
    @location(5) clip_valid: f32,
    @location(6) ndc: vec2<f32>,
#endif
}

@vertex
fn emission_vertex(mesh: EmissionVertexInput) -> EmissionVertexOutput {
    var out: EmissionVertexOutput;
    // Preserve the same two matrix-vector operations as the main material.
    // Reassociating to (VP * M) * p changes rounding and can fail the depth
    // comparison against this very surface after the main pass wrote it.
#ifdef SKINNED
    let world_from_local = emission_skin_model(mesh.joint_indices, mesh.joint_weights);
#else
    let world_from_local = object.world_from_local;
#endif
    let world_position = world_from_local * vec4<f32>(mesh.position, 1.0);
    let clip = main_view.clip_from_world * vec4<f32>(world_position.xyz, 1.0);
    out.position = clip;
    let uv = mesh.uv + object.params.main_tex_offset.xy - env.time.y * object.params.main_tex_offset.zw;
    out.uv = vec2<f32>(
        uv.x,
        select(uv.y, 1.0 - uv.y, object.params.uv_v_flip.x > 0.5),
    );

#ifdef FIXTURE_WINDOW_CLIP
    out.ndc = clip.xy / clip.w;
    var corner_valid = true;
    // var（而非 let）：四角数组要被循环变量动态索引（主 pass 同注）。
    var corners = array<vec4<f32>, 4>(
        object.params.clip_corner_0,
        object.params.clip_corner_1,
        object.params.clip_corner_2,
        object.params.clip_corner_3,
    );
    var corners_ndc = array<vec2<f32>, 4>(
        vec2<f32>(0.0),
        vec2<f32>(0.0),
        vec2<f32>(0.0),
        vec2<f32>(0.0),
    );
    for (var i = 0; i < 4; i++) {
        let corner_world = object.world_from_local * corners[i];
        let corner_clip = main_view.clip_from_world * vec4<f32>(corner_world.xyz, 1.0);
        if corner_clip.w <= 0.0 {
            corner_valid = false;
        }
        corners_ndc[i] = corner_clip.xy / corner_clip.w;
    }
    out.clip_corner_0 = corners_ndc[0];
    out.clip_corner_1 = corners_ndc[1];
    out.clip_corner_2 = corners_ndc[2];
    out.clip_corner_3 = corners_ndc[3];
    out.clip_valid = select(0.0, 1.0, corner_valid);
#endif

    return out;
}

// ---- 片元共用件（与主 pass 同一份 Bayer 表与窗门判定）----

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// 4×4 Bayer 表与主 pass 同一张（屏幕坐标 × 0.125，table[ix*4+iy]，
// × 0.0618750006 + 0.00999999978）。
fn fixture_bayer_value(pixel: vec2<f32>) -> f32 {
    let q = pixel * vec2<f32>(0.125, 0.125);
    let ix = i32(fract(q.x) * 4.0);
    let iy = i32(fract(q.y) * 4.0);
    var table = array<f32, 16>(
        0.0, 12.0, 3.0, 15.0,
        8.0, 4.0, 11.0, 7.0,
        2.0, 14.0, 1.0, 13.0,
        10.0, 6.0, 9.0, 5.0,
    );
    let value = table[ix * 4 + iy];
    return value * 0.0618750006 + 0.00999999978;
}

@fragment
fn emission_fragment(in: EmissionVertexOutput) -> @location(0) vec4<f32> {
    // Texture derivatives are evaluated before conditional clipping.
    let base = textureSampleBias(main_tex, main_sampler, in.uv, env.mip_bias.x);
    let mask = textureSampleBias(mask_tex, mask_sampler, in.uv, env.mip_bias.x);

#ifdef FIXTURE_WINDOW_CLIP
    // 窗外观门（模板 Equal 的等价形，判定式与主 pass 同一条）。
    if in.clip_valid > 0.5 {
        let c0 = in.clip_corner_0;
        let c1 = in.clip_corner_1;
        let c2 = in.clip_corner_2;
        let c3 = in.clip_corner_3;
        let orientation = cross2(c1 - c0, c2 - c0);
        let s0 = cross2(c1 - c0, in.ndc - c0) * orientation;
        let s1 = cross2(c2 - c1, in.ndc - c1) * orientation;
        let s2 = cross2(c3 - c2, in.ndc - c2) * orientation;
        let s3 = cross2(c0 - c3, in.ndc - c3) * orientation;
        if !(s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) {
            discard;
        }
    }
#endif
    // 两个采样都在 discard 链之前（源的求值序：主贴图、遮罩、抖动）。
#ifdef FIXTURE_ALPHA_CLIP
    if base.a - 0.5 < 0.0 {
        discard;
    }
#endif
#ifdef FIXTURE_DITHER
    if object.params.dither_alpha.x - fixture_bayer_value(in.position.xy) < 0.0 {
        discard;
    }
#endif

    // 现象自发光门：类型 1 比 bright、类型 2 比 dark、其余 0
    // Basic animation callbacks can override the result below.
    var gate = 0.0;
    switch (i32(env.emission_type.x + 0.5)) {
        case 1: {
            gate = select(0.0, 1.0, object.emission.x == 1.0);
        }
        case 2: {
            gate = select(0.0, 1.0, object.emission.y == 1.0);
        }
        default: {}
    }

    // 第二颜色目标的输出：遮罩 × 门，alpha = 主贴图 alpha（源的两个
    // 目标 alpha 同值）。遮罩编回存储域后再乘门 = 源的 `Target1`；
    // 目标是 `Rgba16Float`、硬件不编码 ⇒ **写出侧不解码**（见上方「色彩域」）。
#ifdef FIXTURE_RUG
    gate = select(gate, 1.0, object.emission.z == 1.0);
    return vec4<f32>(srgb_format_encode(mask.rgb) * gate, 1.0);
#else
    switch (i32(object.emission.w)) {
        case 1: { gate = 1.0; }
        case 2: { gate = 0.0; }
        default: {}
    }
    return vec4<f32>(srgb_format_encode(mask.rgb) * gate, base.a);
#endif
}
