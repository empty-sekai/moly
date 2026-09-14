// 站点族的 Base 程序：八个 shader 族共用一份，族与 keyword 变体用宏分派。
// 式子照源程序的运算顺序逐句翻译。
//
// 变体键（specialize 注入；每条管线恰有一个族键）：
//   SITE_FIELDOBJECT      FieldObject 族：uniform 阈 clip（keyword）+
//                         Bayer 抖动（0.25）+ 平方顶点色浮点 lerp +
//                         运行时 fresnel 开关 + overlay（门用原始顶点 alpha）。
//   SITE_GROUND           Ground 族（滚动族）。
//   SITE_TREE             Tree 族：常量阈 clip（keyword）+ 原始顶点色布尔
//                         选择（两次）+ 现象光照门（EMF fresnel 门内第一、
//                         高度渐变门内末尾）+ alpha 恒 1.0。
//   SITE_WATER            Water 族（滚动族）。
//   SITE_GROUND_BIRTHDAY  Ground-Birthday 族（滚动族）：抖动 0.125 第一、
//                         overlay 门多乘 opacity 标量、落影走 maskramp。
//   SITE_OBJECT           Object 族：坐标源 switch + 无 fract 滚动 +
//                         严格 >0 的被选 alpha + 精确等 1 的 fresnel +
//                         toon 因子乘 shade.w + 门后无条件加色 +
//                         恒编译的位置-长度形高度淡出。
//   SITE_DROPITEM         DropItem 族：顶点 uv 选择 + fract 滚动 +
//                         常量阈 clip + 全局边缘对 toon + 光混合按
//                         light.w·强度缩放 + alpha 直出 tex.a。
//   SITE_UI_UBER          UI-Uber 族：顶点 ST 变换 + 无 bias 采样 +
//                         纯乘顶点色（blend 状态在 specialize 落）。
//   SITE_SCROLL           合成宏：Ground/Water/Ground-Birthday 三族的
//                         顶点滚动段与片元共用臂（预处理器没有「或」）。
//   SITE_OVERLAY_1ST      _USE_OVERLAY_TEXTURE：第一层 overlay（四族共用
//   SITE_OVERLAY_2ND      _USE_OVERLAY_TEXTURE_2ND：第二层（Ground/Water）。
//   SITE_MODULE_FRESNEL   _ENABLE_MODULE_FRESNEL：keyword 编译期 fresnel
//                         （Ground/Water 链尾、Tree 门内第一）。
//   SITE_TREE_ANIMATION   _USE_TREE_ANIMATION：树动画（风摆 + 叶旋转）。
//   SITE_UNIFORM_ALPHA_CLIP   _USE_ALPHA_CLIP 的 uniform 阈形式（FO/Object）。
//   SITE_CONST_ALPHA_CLIP     _USE_ALPHA_CLIP 的常量 0.5 阈（Tree/DropItem）。
//   SITE_SELECTED_ALPHA_CLIP  _USE_ALPHA_CLIP 的被选 alpha 阈
//                         （Ground/Water/Birthday：selected − 0.5 < 0）。
//   SITE_GROUND_HEIGHT_FADE   _USE_HEIGHT_FADE：Ground 高度淡出（雾后）。
//   SITE_TREE_HEIGHT_FADE     _USE_HEIGHT_FADE：Tree 三色两段高度渐变。
//   SITE_BIRTHDAY_DITHER  !_DISABLE_DITHER：Birthday 抖动（0.125）。
// 顶点属性键由引擎按网格布局注入：VERTEX_COLORS / VERTEX_UVS_A /
// VERTEX_UVS_B；网格没有顶点色时按源语义回白色 (1,1,1,1)。
//
// 源程序写两个颜色目标（SV_Target1 = 0 或 emission）；本管线的主 pass
// 只有一个颜色目标，第二目标不在此列（Object 的 emission 块同此放弃）。
//
// ⚠ **色彩域是这份移植的承重前提，动任何一条算术之前先读下方「色彩域」
//    那一节**（在绑定声明之后、顶点输出之前）。整条片元链在**存储
//    （gamma）域**上算，不在线性域——真源是 gamma 色彩空间构建。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

// ---- 材质 uniform（group 3 binding 0）：每条 Unity 属性一个 vec4 槽 ----
// 槽序与 `site_material.rs` 里 `SiteParams::bytes` 的摊平序是契约，两边
// 同改。族不消费的槽写零。

struct SiteParams {
    use_vertex_color_blend: vec4<f32>,
    alpha_clip: vec4<f32>,
    base_opacity: vec4<f32>,
    dither_alpha: vec4<f32>,
    override_shading_parameter: vec4<f32>,
    local_shading_intensity: vec4<f32>,
    local_edge_threshold: vec4<f32>,
    local_edge_smoothness: vec4<f32>,
    fresnel_power: vec4<f32>,
    use_fresnel: vec4<f32>,
    fresnel_color: vec4<f32>,
    texture_coord_overlay1st: vec4<f32>,
    overlay_st: vec4<f32>,
    uv_scroll: vec4<f32>,
    uv_scroll_overlay1st: vec4<f32>,
    use_overlay_texture_vertex_alpha: vec4<f32>,
    use_vertex_alpha_opacity: vec4<f32>,
    use_phenomena_lighting: vec4<f32>,
    texture_coord_overlay2nd: vec4<f32>,
    overlay_st_2nd: vec4<f32>,
    uv_scroll_overlay2nd: vec4<f32>,
    use_overlay_texture_vertex_alpha_2nd: vec4<f32>,
    turbulence: vec4<f32>,
    strength: vec4<f32>,
    leaf_rotation_speed: vec4<f32>,
    leaf_rotation_range: vec4<f32>,
    receive_shadow: vec4<f32>,
    object_uv_scroll: vec4<f32>,
    birthday_overlay_opacity: vec4<f32>,
    height_fade_rcp_length: vec4<f32>,
    height_fade_start_time_rcp_length: vec4<f32>,
    height_fade_exponent: vec4<f32>,
    use_height_fade: vec4<f32>,
    height_fade_position: vec4<f32>,
    height_fade_length: vec4<f32>,
    height_gradient_pos01: vec4<f32>,
    height_gradient_pos12: vec4<f32>,
    height_gradient_color0: vec4<f32>,
    height_gradient_color1: vec4<f32>,
    height_gradient_color2: vec4<f32>,
    dropitem_uv_selection: vec4<f32>,
    dropitem_uv_scroll: vec4<f32>,
    ui_uber_main_st: vec4<f32>,
    additive_color: vec4<f32>,
    object_texture_mapping: vec4<f32>,
    object_main_texture_local_mapping: vec4<f32>,
}

// ---- 全局量（group 3 binding 1）：一帧一份，全部站点材质共用 ----
// 槽序与 `env.rs` 的 gpu_bytes 摊平序是契约，两边同改。

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
    // 现象自发光类型（全局 int）：站点族不消费（那门的读者在家具族的
    // 自发光 pass），槽带着保持表两侧同形。
    emission_type: vec4<f32>,
    // Ground/Water/Birthday 落影 maskramp 的边缘对 (.x=E1, .y=E2)。
    shadow_mask_edges: vec4<f32>,
    // DropItem 全局标量 (.x=现象光强度, .y=边缘平滑, .z=边缘阈, .w=明暗强度)。
    dropitem_globals: vec4<f32>,
    // 高度淡出混合目标（_MysekaiSkyBottomColor）。
    sky_bottom_color: vec4<f32>,
}

@group(3) @binding(0) var<uniform> params: SiteParams;
@group(3) @binding(1) var<uniform> env: SiteEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;
@group(3) @binding(4) var overlay_tex: texture_2d<f32>;
@group(3) @binding(5) var overlay_sampler: sampler;
@group(3) @binding(6) var overlay2nd_tex: texture_2d<f32>;
@group(3) @binding(7) var overlay2nd_sampler: sampler;
// 叶遮罩在顶点阶段采样（叶旋转的门），两段都声明可见。
@group(3) @binding(8) var leaf_mask_tex: texture_2d<f32>;
@group(3) @binding(9) var leaf_mask_sampler: sampler;

// ---- 主光阴影消费口（group 3 binding 10/11/12）：单图单矩阵（
// 槽 [0..3] 之外不存在；深度图由 shadowmap.rs 的深度 pass 一次成图）。
// 律五件套（moly_law::shading::fieldobject 的 tint/apply/atten/pcf9）是
// 移植 oracle，这里按同式同序转录；改式必须律与本文件同步。
// 槽序与 shadowmap.rs 的 ShadowConsumer 摊平序是契约，两边同改。

struct SiteShadow {
    // 世界→阴影图：xy 图内 uv（含 y 翻转），z 比较深度（[0,1]，近 0 远 1）。
    world_to_shadow: mat4x4<f32>,
    // (宽, 高, 1/宽, 1/高)。
    size: vec4<f32>,
    // _MainLightShadowParams 形状：.x 强度，.z 距离平方系数，.w fade 起点。
    params: vec4<f32>,
};

@group(3) @binding(10) var<uniform> shadow: SiteShadow;
@group(3) @binding(11) var shadow_tex: texture_depth_2d;
@group(3) @binding(12) var shadow_cmp: sampler_comparison;

// ---- 色彩域（改这个文件里任何一条算术之前先读）----
//
// 真源是 **gamma 色彩空间**构建：贴图采样不解码、片元里整条链在存储
// （编码）域上算、结果按编码值直写帧缓冲。本管线两头都不同：
//
//   1 站点贴图按 sidecar 的 `textureColourSpace` 装载（`site_material.rs`
//     的 `load_dir_texture`），本族消费的四个槽在提取产物里**全部**声明
//     sRGB ⇒ 采样时硬件按 sRGB EOTF 解码，`textureSample*` 给出的是线性值；
//   2 本管线的色彩目标是 sRGB 格式 ⇒ 写出时硬件按 sRGB OETF 再编码一次。
//
// ⇒ 采样后先 [`srgb_format_encode`] 编回存储域，中间整条链与源逐式同域，
//   写出前 [`srgb_format_decode`] 解一次、硬件那一次编码恰好还原。**两步都
//   不是源的式子，是域适配**——源没有硬件的那两步，所以它也没有对应的逆。
//
// ⚠ **uniform 颜色不参与这两步**：它们两侧同为原始浮点直推（`site_material.rs`
//   从提取产物的 `colors` 原样拷入、`env.rs` 的现象色是源字面量，都不做域
//   转换）⇒ 本来就在存储域。顶点色同理：wgpu 的顶点格式**没有 sRGB 变体**，
//   硬件不可能解码一个顶点属性，所以 `in.color` 也是存储值。
//   ⇒ 收口之前，这些存储域的量正在和解码后的线性贴图值一起算——那是与
//   「输出多编一次」并列的第二个域错，同一步一起收。
//
// ⚠ **仓里并存两族 gamma 曲线，名字刻意不同族，别拿错**：
//   · **格式**那一族（IEC 61966-2-1）= 本文件的 `srgb_format_encode` /
//     `srgb_format_decode`，抵消的是**硬件**在采样与写出上做的那两步，
//     指数写 `1/2.4`。`srgb_format_decode` 逐式同
//     `moly_law::weather::sky::srgb_target_value`（律侧是唯一权威形）。
//   · **引擎**那一族 = `moly_law::weather::sky::gamma_to_linear` /
//     `linear_to_gamma`，多一支 2.2 幂、指数是源里的截断字面量，只用在
//     **源自己**做 gamma 往返的地方（例如天空的附加色属性）。
//   两族混用会在低端偏若干码；写成纯 `pow(2.2)` 同理，它在 c=0.05 附近偏约 7 码。
//
// ⚠ **alpha 不参与**：硬件的 sRGB 变换只作用于 rgb 三通道，采样得到的 `.a`
//   就是存储值（源的 alpha 也是存储值）⇒ clip 阈值、overlay 门、输出 alpha
//   全部不改。

// 采样值 → 存储域：抵消硬件的 sRGB 解码（格式 OETF）。UNORM 的 sRGB 纹理
// 解码后恒在 [0,1]，`max` 只为让幂底数有定义，取值不受影响。
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}

// 单通道形（叶遮罩只取 `.r`）。
fn srgb_format_encode_scalar(linear: f32) -> f32 {
    let x = max(linear, 0.0);
    let hi = 1.055 * pow(x, 1.0 / 2.4) - 0.055;
    return select(hi, x * 12.92, x <= 0.0031308);
}

// 存储域 → 交给 sRGB 色彩目标的值（格式 EOTF）。UNORM 目标存不下 0..1
// 之外的值，先钳与硬件后钳等价（曲线单调），顺带让幂底数为正。
fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

// ---- 顶点输出 ----

struct SiteVertexOutput {
    @builtin(position) position: vec4<f32>,
    // xyz：世界坐标。
    @location(0) world_position: vec4<f32>,
    // 顶点里归一化过的逆转置法线；片元里按各族源形状决定是否再归一化。
    @location(1) world_normal: vec3<f32>,
    // 原样 uv0：FO/Tree 的主贴图坐标、滚动族的滚动底、UI-Uber 的 ST 底。
    @location(2) uv: vec2<f32>,
    // 原样 uv1：overlay 在 _TextureCoord_Overlay1st == 1、Object mapping==2、
    // DropItem _UVSelection==1 时的坐标源。
    @location(3) uv_b: vec2<f32>,
    // 原样 COLOR0：FO 的顶点色混合用它的平方、Tree/Object 用原始值。
    @location(4) color: vec4<f32>,
    // 雾斜坡：顶点里算好。
    @location(5) fog_ramp: f32,
    // 滚动族：顶点里归一化过的视线；fresnel 片元里再归一化。
    @location(6) view_dir: vec3<f32>,
    // 滚动族：滚动后的 uv0，主贴图坐标。
    @location(7) scrolled_uv: vec2<f32>,
    // 滚动族：rgb 平方、w 原样（源的 TEXCOORD4）。
    @location(8) color_sq: vec4<f32>,
    // overlay：ST 变换 + overlay 滚动后的坐标。
    @location(9) overlay_uv: vec2<f32>,
    // 第二层 overlay：同上（量名带 2nd）。
    @location(10) overlay2nd_uv: vec2<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> SiteVertexOutput {
    // 树动画先在局部空间改位置，改完才做世界变换。
    var local_position = vec4<f32>(mesh.position, 1.0);
#ifdef SITE_TREE_ANIMATION
    // 风摆：正弦脉动乘 (0.66, 1) 的方向向量，幅度随高度的四次增长，
    // 位移后球面规整回原半径。源用平滑时间；这里取同一时间（无平滑器）。
    let ts = env.time.y * params.turbulence.x;
    let s = sin(ts);
    let shaping = s * 0.100000001 * (s * 0.100000001 - 0.100000001) + 0.100000001;
    let sway = shaping * vec2<f32>(0.660000026, 1.0);
    let growth = 0.0110000018 * (mesh.position.y * params.strength.x) + 1.0;
    let growth_sq = growth * growth;
    let amp = growth_sq * growth_sq - growth_sq;
    var swayed = vec3<f32>(
        mesh.position.x + sway.x * amp,
        mesh.position.y,
        mesh.position.z + sway.y * amp,
    );
    swayed = swayed * inverseSqrt(dot(swayed, swayed)) * length(mesh.position);
    local_position = vec4<f32>(swayed, 1.0);
#endif
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        local_position,
    );

    var out: SiteVertexOutput;
    out.position = position_world_to_clip(world_position.xyz);
    out.world_position = world_position;

#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        mesh.normal,
        mesh.instance_index,
    );
#else
    out.world_normal = vec3<f32>(0.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = mesh.uv;
#ifdef FIXTURE_SOURCE_UV
    out.uv.y = 1.0 - out.uv.y;
#endif
#else
    out.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = mesh.uv_b;
#ifdef FIXTURE_SOURCE_UV
    out.uv_b.y = 1.0 - out.uv_b.y;
#endif
#else
    out.uv_b = vec2<f32>(0.0, 1.0); // 缺失源 TEXCOORD1 为 (0,0)，导出 V 翻转后为 (0,1)。
#endif
#ifdef VERTEX_COLORS
    out.color = mesh.color;
#else
    // 源语义：COLOR0 缺席是白色——乘白对混合与 alpha 都无贡献。
    out.color = vec4<f32>(1.0);
#endif
#ifdef SITE_TREE_ANIMATION
    // 叶旋转：绕 (0.75, 0.25) 的 2D 旋转，遮罩顶点采样 > 0.5 才启用。
    let ang = sin(env.time.y * params.leaf_rotation_speed.x * 3.14159274)
        * params.leaf_rotation_range.x * 0.00872664712;
    let d = out.uv - vec2<f32>(0.75, 0.25);
    let rotated = vec2<f32>(
        cos(ang) * d.x - sin(ang) * d.y,
        cos(ang) * d.y + sin(ang) * d.x,
    ) + vec2<f32>(0.75, 0.25);
    // 叶遮罩按 sRGB 装载（提取产物里该槽的色彩空间声明是 sRGB）⇒ 采样值
    // 已被硬件解码，而源比的是**存储值** > 0.5 ⇒ 编回存储域再比。不编的
    // 话阈值实际落在存储域的 0.7354 上，存储值在 (0.5, 0.7354) 的纹素整片
    // 反向。
    let mask = srgb_format_encode_scalar(
        textureSampleLevel(leaf_mask_tex, leaf_mask_sampler, out.uv, 0.0).r,
    );
    if mask > 0.5 {
        out.uv = rotated;
    }
#endif
#ifdef SITE_DROPITEM
    // DropItem 的法线在顶点归一、片元不再归一（源形状；其余族片元 renorm）。
#ifdef VERTEX_NORMALS
    out.world_normal = normalize(out.world_normal);
#endif
    // uv 选择：switch(_UVSelection)——1 取 uv1，0 与其它取 uv0。
    if params.dropitem_uv_selection.x == 1.0 {
        out.uv = out.uv_b;
    }
#endif
#ifdef SITE_UI_UBER
    // UI-Uber：uv0 先过 _MainTex_ST 变换再进片元。
    out.uv = out.uv * params.ui_uber_main_st.xy + params.ui_uber_main_st.zw;
#endif

    // 雾深：源读 GL 约定下的 clip.z，式子 ((z+n)/(n+f))·f 在该约定下
    // 恰等于 f·(clip.w − n)/(f − n)，而 clip.w（眼空间深度）在两种
    // clip 约定下相同——用 clip.w 重建，不依赖管线的 z 约定。
    let n = env.projection_params.y;
    let f = env.projection_params.z;
    var fog_depth = f * (out.position.w - n) / (f - n);
    fog_depth = max(fog_depth, 0.0);
    fog_depth = fog_depth * env.fog_params.x + env.fog_params.y;
    out.fog_ramp = clamp(fog_depth, 0.0, 1.0);

    // 滚动族（Ground/Water/Birthday）：滚动 uv、视线、平方顶点色、overlay
    // 坐标都在顶点程序里。
#ifdef SITE_SCROLL
    let t = env.time.y;
    var scrolled = out.uv;
#ifdef VERTEX_UVS_A
    // Y 项是加不是减：glb 里 v'=1−v（glTF 约定），源式 −t·S_y+v
    // 换进 v' 空间得 v'+t·S_y——静止时两次翻转相抵，动起来必须补这个
    // 符号，否则 V 向滚动整体反向。X 项不补：世界 x 镜像自洽。
    scrolled = vec2<f32>(
        out.uv.x - t * params.uv_scroll.x,
        out.uv.y + t * params.uv_scroll.y,
    );
#endif
    out.scrolled_uv = scrolled;

    var to_camera = env.camera_position.xyz - world_position.xyz;
    let inverse_length = inverseSqrt(dot(to_camera, to_camera));
    var view = to_camera * inverse_length;
    if env.ortho_params.w != 0.0 {
        view = vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z);
    }
    out.view_dir = view;

    out.color_sq = vec4<f32>(out.color.rgb * out.color.rgb, out.color.a);

#ifdef SITE_OVERLAY_1ST
    // overlay 坐标：switch(_TextureCoord_Overlay1st)——1 取原样 uv1，
    // 0 与其它取滚动 uv0；然后 ST 变换，再减 overlay 自己的滚动。
    var coord_base = scrolled;
    if params.texture_coord_overlay1st.x == 1.0 {
        coord_base = out.uv_b;
    }
    let st = coord_base * params.overlay_st.xy + params.overlay_st.zw;
    // overlay 的 Y 滚动同样在 v' 空间翻符号（见上方 scrolled 处注释）。
    // ST 若非恒等还差 offset 换算 (1−t_y−off_y)；当前全部场景数据里
    // overlay ST 恒等，不换算。uv1 直取支路同此符号。
    out.overlay_uv = vec2<f32>(
        st.x - t * params.uv_scroll_overlay1st.x,
        st.y + t * params.uv_scroll_overlay1st.y,
    );
#else
    out.overlay_uv = vec2<f32>(0.0);
#endif
#ifdef SITE_OVERLAY_2ND
    // 第二层 overlay：与第一层完全同形（量名带 2nd）。
    var coord_base_2nd = scrolled;
    if params.texture_coord_overlay2nd.x == 1.0 {
        coord_base_2nd = out.uv_b;
    }
    let st_2nd = coord_base_2nd * params.overlay_st_2nd.xy + params.overlay_st_2nd.zw;
    out.overlay2nd_uv = vec2<f32>(
        st_2nd.x - t * params.uv_scroll_overlay2nd.x,
        st_2nd.y + t * params.uv_scroll_overlay2nd.y,
    );
#else
    out.overlay2nd_uv = vec2<f32>(0.0);
#endif
#else
    // 非滚动族（FO/Tree/Object/DropItem/UI-Uber）：滚动槽全部落中性值。
    out.scrolled_uv = out.uv;
    out.view_dir = vec3<f32>(0.0);
    out.color_sq = vec4<f32>(1.0);
#ifdef SITE_OVERLAY_1ST
    // FO 的 overlay 坐标：switch(_TextureCoord_Overlay1st)——0 与缺省取
    // **原始** uv0（未滚动，与滚动族的「滚动 uv」不同形），1 取 uv1；
    // 然后 ST 变换，再减 overlay 自己的滚动（v' 空间 y 翻符号同上）。
    let t = env.time.y;
    var coord_base = out.uv;
    if params.texture_coord_overlay1st.x == 1.0 {
        coord_base = out.uv_b;
    }
    let st = coord_base * params.overlay_st.xy + params.overlay_st.zw;
    out.overlay_uv = vec2<f32>(
        st.x - t * params.uv_scroll_overlay1st.x,
        st.y + t * params.uv_scroll_overlay1st.y,
    );
#else
    out.overlay_uv = vec2<f32>(0.0);
#endif
    out.overlay2nd_uv = vec2<f32>(0.0);
#endif

    return out;
}

// ---- 片元共用件 ----

fn phenomena_light_blend(rgb: vec3<f32>) -> vec3<f32> {
    return env.phenomena_light_color.w
        * (rgb * env.phenomena_light_color.rgb - rgb)
        + rgb;
}

fn treasure_shadow(rgb: vec3<f32>, world_xz: vec2<f32>, treasure_xz: vec2<f32>, intensity: f32) -> vec3<f32> {
    let distance = length(world_xz - treasure_xz);
    var t = (distance + -0.959999979) * 24.9999866;
    t = clamp(t, 0.0, 1.0);
    return intensity * 0.5 * (rgb * t - rgb) + rgb;
}

fn apply_fog(rgb: vec3<f32>, world_y: f32, fog_ramp: f32) -> vec3<f32> {
    // 雾色整支插值后 clamp，alpha 参与高度项。
    var fog_color = clamp(
        fog_ramp * (env.fog_near_color - env.fog_far_color) + env.fog_far_color,
        vec4<f32>(0.0),
        vec4<f32>(1.0),
    );
    let distance_blended = fog_ramp * (rgb - fog_color.rgb) + fog_color.rgb;
    let delta = distance_blended - rgb;
    let height = min(exp2(-world_y * env.fog_params.z), 1.0) * fog_color.w;
    return clamp(height * delta + rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

// 源像素坐标是 GL 约定：原点左下、y 向上、半像素偏移。本管线的帧缓冲
// 原点在左上（与 shadowmap.rs 的采样矩阵同一结论），x 同向直取、y 折算
// H − position.y；折算后的坐标与源逐像素一致。
fn bayer_pixel(position: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(position.x, env.screen_params.y - position.y);
}

// 源经 one-hot 点积还原出的 4×4 表：行是屏幕像素 y（自下而上）、列是 x。
// 量化系数是族的量：FieldObject 0.25、Birthday 0.125（源两处逐行核过，
// 表与缩放/偏移两族完全同值，差只在量化；像素域经顶点的
// 0.5·(clip.xy + clip.w) 除回 w 恒正，负支路不可达）。
fn bayer_value(pixel: vec2<f32>, quantization: f32) -> f32 {
    // 源式：像素坐标 · 量化系数 取小数 · 4 即 mod 4。
    let q = pixel * vec2<f32>(quantization, quantization);
    let ix = i32(fract(q.x) * 4.0);
    let iy = i32(fract(q.y) * 4.0);
    var table = array<f32, 16>(
        0.0, 12.0, 3.0, 15.0,
        8.0, 4.0, 11.0, 7.0,
        2.0, 14.0, 1.0, 13.0,
        10.0, 6.0, 9.0, 5.0,
    );
    let value = table[iy * 4 + ix];
    return value * 0.0618750006 + 0.00999999978;
}

// ---- DropShadow 律消费口（按 moly-law shading::fieldobject 同式同序转录）----

// 律 drop_shadow_tint：tint = lerp(colour, colour × C1.rgb, C1.a)，
// C1 即 env.drop_shadow_color（真源全局量 _MysekaiDropShadowColor1）。
fn drop_shadow_tint(colour: vec3<f32>) -> vec3<f32> {
    return env.drop_shadow_color.w * (colour * env.drop_shadow_color.rgb - colour) + colour;
}

// 律 apply_drop_shadow：lerp(tint(colour), colour, atten)。
fn apply_drop_shadow(colour: vec3<f32>, atten: f32) -> vec3<f32> {
    let tinted = drop_shadow_tint(colour);
    return atten * (colour - tinted) + tinted;
}

// 律 shadow_attenuate 的直线序三步：强度门 → 比较深度出 [0,1) 整段免影
// （覆写前一步）→ 距离 fade 平方往无影插回。
fn shadow_attenuate(shadow_sum: f32, dist_sq: f32, compare_depth: f32) -> f32 {
    var atten = shadow_sum * shadow.params.x + (1.0 - shadow.params.x);
    if compare_depth < 0.0 || compare_depth >= 1.0 {
        atten = 1.0;
    }
    let fade = clamp(dist_sq * shadow.params.z + shadow.params.w, 0.0, 1.0);
    let fade_sq = fade * fade;
    return fade_sq * (1.0 - atten) + atten;
}

// 律 tent_weights：单轴六份 tent 权重，f 是该轴落在基准 texel 内的带符号
// 小数，六份之和恒为 1。两个权重常数按源字面（律 TENT_WEIGHT /
// TENT_SECONDARY_WEIGHT）。
fn tent_weights(f: f32) -> array<f32, 6> {
    let below = min(f, 0.0);
    let above = max(f, 0.0);
    let inward = 1.0 - f;
    let centre_sq = (f + 0.5) * (f + 0.5);
    let w = 0.159999996;
    let s = 0.0799999982;
    return array<f32, 6>(
        inward * w,
        (inward - below * below + 1.0) * w,
        ((f + 1.0) - above * above + 1.0) * w,
        (f + 1.0) * w,
        (centre_sq * 0.5 - f) * w,
        centre_sq * s,
    );
}

// 九个 tap 的 UV 与 tent 权重按源片元程序序推得（两轴的中/尾偏移配对
// 不同，照抄不改写），每份 tap 是一次 lod0 单 texel 硬件深度比较——用
// Level 变体，片元里有 discard/门支路也不需要均匀控制流。加权合成按源
// 乘加次序累加（中上、左上、右上、左中、中心、右中、左下、中下、右下）。
// 格子定位按源式取「uv × 图边长」：源的 _MainLightShadowmapSize 四元组
// 是 (1/宽, 1/高, 宽, 高)（URP 装配侧 SetGlobalVector 逐分量可核），
// 格子行乘 .zw（边长）得 texel 坐标、tap 偏移行乘 .xy（texel 尺寸）回
// 归一化 uv。本表 size 槽序是 (宽, 高, 1/宽, 1/高)——两组分量恰好互换，
// 故「× size.xy」即源的「× .zw」、「× size.zw」即源的「× .xy」。
// moly-law 的 pcf9_tap_uvs 把格子行也乘了 texel 尺寸（两行本应取不同
// 分量）——乘 texel 尺寸会把全部格子塌到图角、九个 tap 恒采左上一角，
// 四族收影目视不可见的病灶即此。律侧修正是另一张单；本文件以源片元
// 文本为准，勿按律「改回」。
fn pcf9(shadow_uv: vec2<f32>, compare_depth: f32) -> f32 {
    let texel = shadow.size.zw;
    let scaled = shadow_uv * shadow.size.xy;
    let cell = floor(scaled + vec2<f32>(0.5));
    let frac = scaled - cell;
    let wx = tent_weights(frac.x);
    let wy = tent_weights(frac.y);
    // 列和（x 轴：左 wa+we、中 wc+wb、右 wf+wd）与行和
    // （y 轴：上 wa+we、中 wf+wd、下 wc+wb）。
    let col = vec3<f32>(wx[0] + wx[4], wx[2] + wx[1], wx[5] + wx[3]);
    let row = vec3<f32>(wy[0] + wy[4], wy[5] + wy[3], wy[2] + wy[1]);
    let dx = vec3<f32>(
        (wx[0] / col[0] - 2.5) * texel.x,
        (wx[2] / col[1] - 0.5) * texel.x,
        (wx[5] / col[2] + 1.5) * texel.x,
    );
    let dy = vec3<f32>(
        (wy[0] / row[0] - 2.5) * texel.y,
        (wy[5] / row[1] - 0.5) * texel.y,
        (wy[2] / row[2] + 1.5) * texel.y,
    );
    let base = cell * texel;
    var uvs: array<vec2<f32>, 9>;
    uvs[0] = vec2<f32>(base.x + dx.z, base.y + dy.x);
    uvs[1] = vec2<f32>(base.x + dx.y, base.y + dy.x);
    uvs[2] = vec2<f32>(base.x + dx.x, base.y + dy.x);
    uvs[3] = vec2<f32>(base.x + dx.x, base.y + dy.y);
    uvs[4] = vec2<f32>(base.x + dx.x, base.y + dy.z);
    uvs[5] = vec2<f32>(base.x + dx.y, base.y + dy.y);
    uvs[6] = vec2<f32>(base.x + dx.z, base.y + dy.y);
    uvs[7] = vec2<f32>(base.x + dx.y, base.y + dy.z);
    uvs[8] = vec2<f32>(base.x + dx.z, base.y + dy.z);
    var samples: array<f32, 9>;
    for (var i = 0; i < 9; i += 1) {
        samples[i] = textureSampleCompareLevel(shadow_tex, shadow_cmp, uvs[i], compare_depth);
    }
    var sum = samples[1] * (row[0] * col[1]);
    sum += samples[2] * (row[0] * col[0]);
    sum += samples[0] * (row[0] * col[2]);
    sum += samples[3] * (row[1] * col[0]);
    sum += samples[5] * (row[1] * col[1]);
    sum += samples[6] * (row[1] * col[2]);
    sum += samples[4] * (row[2] * col[0]);
    sum += samples[7] * (row[2] * col[1]);
    sum += samples[8] * (row[2] * col[2]);
    return sum;
}

// 主光阴影衰减：采样坐标 = 单矩阵 × 世界坐标；距离平方用世界位到相机位。
// 收影轴是材质上的显式槽（真源 _RECEIVE_SHADOWS_OFF 轴：带该 keyword 的
// 变体源里整族无影针，解析层按 keyword 定 0）。
fn main_light_shadow_atten(world_position: vec4<f32>) -> f32 {
    if params.receive_shadow.x < 0.5 {
        return 1.0;
    }
    let projected = shadow.world_to_shadow * world_position;
    let shadow_sum = pcf9(projected.xy, projected.z);
    let delta = world_position.xyz - env.camera_position.xyz;
    return shadow_attenuate(shadow_sum, dot(delta, delta), projected.z);
}

// toon 斜坡：half-Lambert 在 [lower, upper] 上的归一化斜坡（各族的
// 边缘对选取不同，斜坡式全族同形）。
fn toon_ramp(half_lambert: f32, threshold: f32, smoothness: f32) -> f32 {
    let upper = threshold + smoothness;
    let lower = threshold - smoothness;
    var ramp = (half_lambert - upper) / (lower - upper);
    return clamp(ramp, 0.0, 1.0);
}

// 片元按族分臂：每条管线恰有一个族键，预处理后只剩一臂（各自带 return）。
@fragment
fn fragment(in: SiteVertexOutput) -> @location(0) vec4<f32> {
#ifdef SITE_FIELDOBJECT
    // ---- FieldObject 族 ----
    // 主贴图带全局 mip 偏置采样。
    let base = textureSampleBias(main_tex, main_sampler, in.uv, env.mip_bias.x);
#ifdef SITE_UNIFORM_ALPHA_CLIP
    // clip 仅在 _USE_ALPHA_CLIP 编译进来（无该 keyword 的变体零丢弃）。
    if base.a - params.alpha_clip.x < 0.0 {
        discard;
    }
#endif
    // Bayer 抖动（量化 0.25，全族恒抖动）：dither − 阈值 < 0 丢弃。
    if params.dither_alpha.x - bayer_value(bayer_pixel(in.position), 0.25) < 0.0 {
        discard;
    }

    // `base_rgb` 是编回存储域的主贴图色（见文件头「色彩域」）——顶点色与
    // uniform 本来就在存储域，同域相乘/混合才是源的那个函数。
    let base_rgb = srgb_format_encode(base.rgb);
    var rgb = base_rgb;
#ifdef SITE_OVERLAY_1ST
    // overlay 门用**原始**顶点 alpha、不乘 opacity 标量（FO 形，区别于
    // Birthday 的 _Overlay1st_Opacity）：(uotva·(vc.w − 1) + 1)·ov.a。
    // overlay 同按 sRGB 装载 ⇒ 编回存储域再混合（门与 alpha 不动）。
    let overlay = textureSampleBias(overlay_tex, overlay_sampler, in.overlay_uv, env.mip_bias.x);
    let overlay_rgb = srgb_format_encode(overlay.rgb);
    let gate = (params.use_overlay_texture_vertex_alpha.x * (in.color.w - 1.0) + 1.0) * overlay.a;
    rgb = gate * (overlay_rgb - base_rgb) + base_rgb;
#endif
    // 顶点色混合是浮点 lerp、平方色，作用在混合后的 rgb 上：
    // rgb + f·(rgb·vc² − rgb)。
    rgb = rgb + params.use_vertex_color_blend.x
        * (rgb * in.color.rgb * in.color.rgb - rgb);

    // 输出 alpha 在光照前写好：tex.a · _BaseOpacity（FO 全臂无 vao select）。
    let alpha = base.a * params.base_opacity.x;

    rgb = phenomena_light_blend(rgb);

    // toon 阴影：阈值/平滑度/强度按 _OverrideShadingParameter 选局部或全局。
    let use_local = params.override_shading_parameter.x > 0.5;
    let threshold = select(env.edge_threshold.x, params.local_edge_threshold.x, use_local);
    let smoothness = select(env.edge_smoothness.x, params.local_edge_smoothness.x, use_local);
    let intensity = select(1.0, params.local_shading_intensity.x, use_local);
    let normal = normalize(in.world_normal);
    var half_lambert = dot(env.light_vector.xyz, normal);
    half_lambert = half_lambert * 0.5 + 0.5;
    let factor = intensity * toon_ramp(half_lambert, threshold, smoothness);
    rgb = rgb + factor * (env.phenomena_shade_color.rgb * rgb - rgb);

    // 落影针一（律链序：toon 阴影之后、宝藏影之前）。全族带 _RSO：
    // receive_shadow=0 早退 1、apply 是恒等插值——源里该族零影针，数值一致。
    let shadow_atten_value = main_light_shadow_atten(in.world_position);
    rgb = apply_drop_shadow(rgb, shadow_atten_value);

    // 宝藏阴影两项。
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);

    // fresnel：附加项先算，运行时开关选择（0.5 比较）。视线按 ortho 位选。
    var view = vec3<f32>(0.0);
    if env.ortho_params.w == 0.0 {
        let to_camera = env.camera_position.xyz - in.world_position.xyz;
        view = normalize(to_camera);
    } else {
        view = vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z);
    }
    var one_minus = clamp(1.0 - dot(normal, view), 0.0, 1.0);
    let fresnel = exp2(log2(one_minus) * params.fresnel_power.x);
    let with_fresnel = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
    rgb = select(rgb, with_fresnel, params.use_fresnel.x > 0.5);

    // 落影针二（律链序：菲涅耳之后、雾之前；同一衰减未改写）。
    rgb = apply_drop_shadow(rgb, shadow_atten_value);

    rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);
    return vec4<f32>(srgb_format_decode(rgb), alpha);
#endif
#ifdef SITE_TREE
    // ---- Tree 族 ----
    let base = textureSampleBias(main_tex, main_sampler, in.uv, env.mip_bias.x);
#ifdef SITE_CONST_ALPHA_CLIP
    // clip 仅在 _USE_ALPHA_CLIP 编译进来；阈值是编译期常量 0.5。
    if base.a - 0.5 < 0.0 {
        discard;
    }
#endif

    // 顶点色是原始色的布尔选择（区别于 FO 的平方/浮点 lerp）：源门取
    // 「use < 0.5 为真选原色」，select 语义相反故取反写。
    let base_rgb = srgb_format_encode(base.rgb);
    var rgb = select(base_rgb, base_rgb * in.color.rgb, params.use_vertex_color_blend.x >= 0.5);

    // 现象光照门：关时 rgb 直通（雾与高度渐变也在门内），开时走完整链。
    if params.use_phenomena_lighting.x > 0.5 {
#ifdef SITE_MODULE_FRESNEL
        // EMF fresnel 在门内第一：法线 renorm + 视线按 ortho 位选。
        let normal_f = normalize(in.world_normal);
        var view = vec3<f32>(0.0);
        if env.ortho_params.w == 0.0 {
            let to_camera = env.camera_position.xyz - in.world_position.xyz;
            view = normalize(to_camera);
        } else {
            view = vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z);
        }
        var one_minus = clamp(1.0 - dot(normal_f, view), 0.0, 1.0);
        let fresnel = exp2(log2(one_minus) * params.fresnel_power.x);
        rgb = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
#endif
        // 边缘对与强度先选好（纯标量，与光混合的先后无数值差）。
        let use_local = params.override_shading_parameter.x > 0.5;
        let threshold = select(env.edge_threshold.x, params.local_edge_threshold.x, use_local);
        let smoothness = select(env.edge_smoothness.x, params.local_edge_smoothness.x, use_local);
        let intensity = select(1.0, params.local_shading_intensity.x, use_local);

        rgb = phenomena_light_blend(rgb);

        // toon 用原始 half-Lambert（未 clamp 的 n·L 直接 ×0.5+0.5）。
        let normal = normalize(in.world_normal);
        var half_lambert = dot(env.light_vector.xyz, normal);
        half_lambert = half_lambert * 0.5 + 0.5;
        let factor = intensity * toon_ramp(half_lambert, threshold, smoothness);
        rgb = rgb + factor * (env.phenomena_shade_color.rgb * rgb - rgb);

        // 落影针一（恒等：全族带 _RSO，见 FO 臂注）。源里该族零影针。
        let shadow_atten_value = main_light_shadow_atten(in.world_position);
        rgb = apply_drop_shadow(rgb, shadow_atten_value);

        rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
        rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);

        // 门内第二道顶点色乘：同一个布尔选择再乘一次原始 vc.rgb。
        rgb = select(rgb, rgb * in.color.rgb, params.use_vertex_color_blend.x >= 0.5);

        // 落影针二（恒等，同上）。
        rgb = apply_drop_shadow(rgb, shadow_atten_value);

        rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);

#ifdef SITE_TREE_HEIGHT_FADE
        // 三色两段高度渐变（门内末尾）。h 用淡出参数 t（倒数长度形），
        // 非原始高度。Pos01=0 的行除法按源字面保留（h=0 处 0/0 与源同形）。
        if params.use_height_fade.x > 0.5 {
            var h = clamp(
                in.world_position.y * params.height_fade_rcp_length.x
                    + -params.height_fade_start_time_rcp_length.x,
                0.0,
                1.0,
            );
            var g0 = clamp(h / params.height_gradient_pos01.x, 0.0, 1.0);
            g0 = min(exp2(log2(g0) * params.height_fade_exponent.x), 1.0);
            let seg0 = params.height_gradient_color0.rgb
                + g0 * (params.height_gradient_color1.rgb - params.height_gradient_color0.rgb);
            var g1 = clamp(
                (h + -params.height_gradient_pos01.x)
                    / (-params.height_gradient_pos01.x + params.height_gradient_pos12.x),
                0.0,
                1.0,
            );
            g1 = min(exp2(log2(g1) * params.height_fade_exponent.x), 1.0);
            let seg1 = params.height_gradient_color1.rgb
                + g1 * (params.height_gradient_color2.rgb - params.height_gradient_color1.rgb);
            // 源 greaterThanEqual：Pos12 >= h 选 seg1，再 Pos01 >= h 选 seg0
            // （边界点归低位段一侧）。
            var grad = select(
                params.height_gradient_color2.rgb,
                seg1,
                params.height_gradient_pos12.x >= h,
            );
            grad = select(grad, seg0, params.height_gradient_pos01.x >= h);
            rgb = h * (grad - rgb) + rgb;
        }
#endif
    }
    // 输出 alpha 恒 1.0：源的 tex.a 只喂 clip，不进输出。
    // ⚠ 现象光照门关时 rgb 是**未经任何算术的存储域底色**，同样要解一次
    //   才能交给 sRGB 目标——门关不等于不需要域适配。
    return vec4<f32>(srgb_format_decode(rgb), 1.0);
#endif
#ifdef SITE_OBJECT
    // ---- Object 族（usage∈{8,12} 走 default 光照分支；2/11/14 在解析层
    // 具名拒绝，此处不编译它们的支路）----
    // 法线 renorm + 视线选择（正交用 MatrixV 列 z）。
    let normal = normalize(in.world_normal);
    var view = vec3<f32>(0.0);
    if env.ortho_params.w == 0.0 {
        let to_camera = env.camera_position.xyz - in.world_position.xyz;
        view = normalize(to_camera);
    } else {
        view = vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z);
    }

    // 主贴图坐标源 switch（_BaseTextureMappingMode）：0 与缺省 uv0、
    // 1 世界 xz、2 uv1（3 = uv2 拒绝）。世界 xz 的 v 分量过 glb 翻转律：
    // 源 v = worldPos.z，v' 空间取 1 − z。
    var coord = in.uv;
    if params.object_texture_mapping.x == 1.0 {
        coord = vec2<f32>(in.world_position.x, 1.0 - in.world_position.z);
    }
    else if params.object_texture_mapping.x == 2.0 {
        coord = in.uv_b;
    }
    // _MainTextureLocalMapping == 1 覆写回 uv0。
    if params.object_main_texture_local_mapping.x == 1.0 {
        coord = in.uv;
    }

    // 滚动：uv − t·(Sx,Sy)，**无 fract**（v' 空间 y 取反律）。
    let t = env.time.y;
    let uv = vec2<f32>(
        coord.x - t * params.object_uv_scroll.x,
        coord.y + t * params.object_uv_scroll.y,
    );
    let main = textureSampleBias(main_tex, main_sampler, uv, env.mip_bias.x);
#ifdef SITE_UNIFORM_ALPHA_CLIP
    if main.a - params.alpha_clip.x < 0.0 {
        discard;
    }
#endif
    // 被选 alpha：源门是**严格 > 0**（区别于滚动族的 0.5 比较）。
    let selected = select(main.a, main.a * in.color.w, params.use_vertex_alpha_opacity.x > 0.0);

    // ndotl 先 clamp 后用（toon 的 half 用 clamp 副本；maskramp 的 r 在
    // usage!=0 时被强制 1.0——本族 usage∈{8,12} 恒死，不编译）。
    let ndotl_clamped = clamp(dot(env.light_vector.xyz, normal), 0.0, 1.0);

    // vc1：浮点 lerp、**原始未平方** vc.rgb（区别于 FO 的平方形）。
    // `main_rgb` 是编回存储域的主贴图色（见文件头「色彩域」）。
    let main_rgb = srgb_format_encode(main.rgb);
    var rgb = main_rgb;
    rgb = rgb + params.use_vertex_color_blend.x * (rgb * in.color.rgb - rgb);

    // 宝箱影两项（链序：vc1 之后、fresnel 之前）。
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);

    // fresnel：源门是**精确等 1**（_UseFresnel == 1.0），非 0.5 比较。
    var one_minus = clamp(1.0 - dot(normal, view), 0.0, 1.0);
    let fresnel = exp2(log2(one_minus) * params.fresnel_power.x);
    let with_fresnel = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
    rgb = select(rgb, with_fresnel, params.use_fresnel.x == 1.0);
    // _BackFaceColor 支路死在 _Cull==2（数据 16/16），不转录。

    // 现象光照门。
    if params.use_phenomena_lighting.x > 0.5 {
        rgb = phenomena_light_blend(rgb);

        let use_local = params.override_shading_parameter.x > 0.5;
        let threshold = select(env.edge_threshold.x, params.local_edge_threshold.x, use_local);
        let smoothness = select(env.edge_smoothness.x, params.local_edge_smoothness.x, use_local);
        let intensity = select(1.0, params.local_shading_intensity.x, use_local);

        // vc2：与 vc1 同形（浮点 lerp、原始 vc.rgb），作用在光混合后的 rgb 上。
        var rgb2 = rgb;
        rgb2 = rgb2 + params.use_vertex_color_blend.x * (rgb2 * in.color.rgb - rgb2);

        let half_lambert = ndotl_clamped * 0.5 + 0.5;
        // Object 独有：因子乘 shade.w（滚动族与 FO 都没有这一项）。
        let factor = intensity * env.phenomena_shade_color.w
            * toon_ramp(half_lambert, threshold, smoothness);
        rgb = factor * (env.phenomena_shade_color.rgb * rgb2 - rgb2) + rgb2;

        // usage != 11：道路纹理缩放与落影整块不编译（恒等跳过）。
        // 雾在门内。
        rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);
    }

    // 门后无条件加色：rgb += _AdditiveColor.rgb · .w。
    rgb = rgb + params.additive_color.rgb * params.additive_color.w;
    // emission（SV_Target1）块放弃：本管线单颜色目标。

    // 高度淡出：块恒编译（无 keyword），运行时 0.5<_UseHeightFade 选择；
    // 位置-长度形（区别于 Ground/Tree 的倒数长度形）。分母按源字面取
    // (−PmL)+Position（代数上等于 Length，浮点次序照抄）。
    var rgb_out = rgb;
    if params.use_height_fade.x > 0.5 {
        let pml = -params.height_fade_length.x + params.height_fade_position.x;
        let num = -pml + in.world_position.y;
        let den = -pml + params.height_fade_position.x;
        var thf = clamp(num / den, 0.0, 1.0);
        thf = min(exp2(log2(thf) * params.height_fade_exponent.x), 1.0);
        rgb_out = thf * (rgb - env.sky_bottom_color.rgb) + env.sky_bottom_color.rgb;
    }

    return vec4<f32>(srgb_format_decode(rgb_out), selected * params.base_opacity.x);
#endif
#ifdef SITE_DROPITEM
    // ---- DropItem 族 ----
    // 滚动带 fract（v' 空间 y 翻转律）。
    let t = env.time.y;
    let uv = vec2<f32>(
        fract(in.uv.x - t * params.dropitem_uv_scroll.x),
        fract(in.uv.y + t * params.dropitem_uv_scroll.y),
    );
    let tex = textureSampleBias(main_tex, main_sampler, uv, env.mip_bias.x);
#ifdef SITE_CONST_ALPHA_CLIP
    if tex.a - 0.5 < 0.0 {
        discard;
    }
#endif
    // 法线不 renorm（顶点已归一——源形状）；half 用原始 ndotl。
    let ndotl = dot(env.light_vector.xyz, in.world_normal);
    let half_lambert = ndotl * 0.5 + 0.5;
    // toon 用全局边缘对（MysekaiGraphicsConfig 常量，env.dropitem_globals）。
    let factor = toon_ramp(
        half_lambert,
        env.dropitem_globals.z,
        env.dropitem_globals.y,
    ) * env.dropitem_globals.w;
    // 光混合按 light.w · 现象光强度 缩放（本族独有）。
    // `tex_rgb` 是编回存储域的贴图色（见文件头「色彩域」）。
    let tex_rgb = srgb_format_encode(tex.rgb);
    var rgb = tex_rgb;
    let blend_scale = env.phenomena_light_color.w * env.dropitem_globals.x;
    rgb = blend_scale * (rgb * env.phenomena_light_color.rgb - rgb) + rgb;
    // toon（因子无 shade.w）。
    rgb = factor * (env.phenomena_shade_color.rgb * rgb - rgb) + rgb;
    // alpha 直出 tex.a（无 _BaseOpacity）；无雾/宝箱/影/顶点色。
    return vec4<f32>(srgb_format_decode(rgb), tex.a);
#endif
#ifdef SITE_UI_UBER
    // ---- UI-Uber 族（_BlendMode 0/1 同 rgb；2 在解析层拒绝）----
    // 无 mip bias（隐式导数）；顶点已过 ST 变换。
    let tex = textureSample(main_tex, main_sampler, in.uv);
    // 源式是 tex·vc 整乘（tex 存储域直乘）；rgb 先编回存储域再乘，alpha
    // 不参与域适配（见文件头「色彩域」）。
    let tex_rgb = srgb_format_encode(tex.rgb);
    let col = vec4<f32>(tex_rgb * in.color.rgb, tex.a * in.color.a);
    return vec4<f32>(srgb_format_decode(col.rgb), col.a);
#endif
#ifdef SITE_SCROLL
    // ---- Ground / Water / Ground-Birthday 共用臂 ----
    // 抖动第一（Birthday 在 !_DISABLE_DITHER 时；量化 0.125——FO 是 0.25）。
#ifdef SITE_BIRTHDAY_DITHER
    if params.dither_alpha.x - bayer_value(bayer_pixel(in.position), 0.125) < 0.0 {
        discard;
    }
#endif
    // half-Lambert 与 toon 因子先于纹理采样（源的次序）；法线 renorm。
    let normal = normalize(in.world_normal);
    var half_lambert = dot(env.light_vector.xyz, normal);
    half_lambert = half_lambert * 0.5 + 0.5;
    let use_local = params.override_shading_parameter.x > 0.5;
    let threshold = select(env.edge_threshold.x, params.local_edge_threshold.x, use_local);
    let smoothness = select(env.edge_smoothness.x, params.local_edge_smoothness.x, use_local);
    let intensity = select(1.0, params.local_shading_intensity.x, use_local);
    let factor = intensity * toon_ramp(half_lambert, threshold, smoothness);

    // 落影的 maskramp：clamp 过的 ndotl 在全局边缘对 (E1,E2) 上的斜坡
    // （源 _ShadowMaskEdge1/2）；落影衰减先过它再进恒等插值：
    // atten' = r·(atten − 1) + 1（Ground record45/49、Water record40、
    // Birthday record28 逐行同形）。
    let ndotl_clamped = clamp(dot(env.light_vector.xyz, normal), 0.0, 1.0);
    let mask_r = clamp(
        (ndotl_clamped + -env.shadow_mask_edges.x)
            / (-env.shadow_mask_edges.x + env.shadow_mask_edges.y),
        0.0,
        1.0,
    );

    var rgb = vec3<f32>(0.0);
    var main_alpha = 0.0;
#ifdef SITE_OVERLAY_1ST
    // overlay 先采样；门 = (use_va·(vc.w − 1) + 1)·ov.a（Birthday 再乘
    // _Overlay1st_Opacity 标量）。
    // 两张贴图都按 sRGB 装载 ⇒ 两边各自编回存储域再混合（门与 alpha 不动）。
    let overlay = textureSampleBias(overlay_tex, overlay_sampler, in.overlay_uv, env.mip_bias.x);
    let main = textureSampleBias(main_tex, main_sampler, in.scrolled_uv, env.mip_bias.x);
    let overlay_rgb = srgb_format_encode(overlay.rgb);
    let main_rgb = srgb_format_encode(main.rgb);
    main_alpha = main.a;
    var gate = (params.use_overlay_texture_vertex_alpha.x * (in.color_sq.w - 1.0) + 1.0)
        * overlay.a;
#ifdef SITE_GROUND_BIRTHDAY
    gate = gate * params.birthday_overlay_opacity.x;
#endif
    let blended = gate * (overlay_rgb - main_rgb) + main_rgb;
    // 顶点色布尔选择作用在混合后的 rgb 上（平方色）。
    rgb = select(blended, blended * in.color_sq.rgb, params.use_vertex_color_blend.x > 0.5);
#else ifdef SITE_OVERLAY_2ND
    // 第二层 overlay：与第一层同形（r24 与 r27 是互斥的两条变体，live
    // 材质里两 keyword 不同时开）。门与纹理都换 2nd 的。
    // 两张贴图都按 sRGB 装载 ⇒ 两边各自编回存储域再混合（门与 alpha 不动）。
    let overlay = textureSampleBias(overlay2nd_tex, overlay2nd_sampler, in.overlay2nd_uv, env.mip_bias.x);
    let main = textureSampleBias(main_tex, main_sampler, in.scrolled_uv, env.mip_bias.x);
    let overlay_rgb = srgb_format_encode(overlay.rgb);
    let main_rgb = srgb_format_encode(main.rgb);
    main_alpha = main.a;
    let gate = (params.use_overlay_texture_vertex_alpha_2nd.x * (in.color_sq.w - 1.0) + 1.0)
        * overlay.a;
    let blended = gate * (overlay_rgb - main_rgb) + main_rgb;
    rgb = select(blended, blended * in.color_sq.rgb, params.use_vertex_color_blend.x > 0.5);
#else
    // 无 overlay 变体：底色 = _MainTex.rgb。
    let main = textureSampleBias(main_tex, main_sampler, in.scrolled_uv, env.mip_bias.x);
    let main_rgb = srgb_format_encode(main.rgb);
    main_alpha = main.a;
    rgb = select(main_rgb, main_rgb * in.color_sq.rgb, params.use_vertex_color_blend.x > 0.5);
#endif // overlay 链（1st/2nd/无 overlay 三支只算 rgb 与 main_alpha）
    // 以下公共尾部对三种 overlay 形态同型。
    {
    rgb = phenomena_light_blend(rgb);
    rgb = rgb + factor * (env.phenomena_shade_color.rgb * rgb - rgb);

    // 落影：衰减先过 maskramp（atten' = r·(atten−1)+1）再进恒等插值。
    let shadow_atten_value = main_light_shadow_atten(in.world_position);
    let atten_prime = mask_r * (shadow_atten_value + -1.0) + 1.0;
    rgb = apply_drop_shadow(rgb, atten_prime);

    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);

#ifdef SITE_MODULE_FRESNEL
    // keyword fresnel：这个变体编译进来就意味着开，附加是无条件的。
    let view = normalize(in.view_dir);
    var one_minus = clamp(1.0 - dot(normal, view), 0.0, 1.0);
    let fresnel = exp2(log2(one_minus) * params.fresnel_power.x);
    rgb = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
#endif

    rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);

#ifdef SITE_GROUND_HEIGHT_FADE
    // Ground 高度淡出：倒数长度形，雾后、臂末（runtime 0.5<_UseHeightFade）。
    if params.use_height_fade.x > 0.5 {
        var t_hf = clamp(
            in.world_position.y * params.height_fade_rcp_length.x
                + -params.height_fade_start_time_rcp_length.x,
            0.0,
            1.0,
        );
        t_hf = min(exp2(log2(t_hf) * params.height_fade_exponent.x), 1.0);
        rgb = t_hf * (rgb - env.sky_bottom_color.rgb) + env.sky_bottom_color.rgb;
    }
#endif

    // 被选 alpha：use_va 开时乘顶点 alpha（0.5 比较）；再乘 _BaseOpacity。
    let selected = select(
        main_alpha,
        main_alpha * in.color_sq.w,
        params.use_vertex_alpha_opacity.x > 0.5,
    );
#ifdef SITE_SELECTED_ALPHA_CLIP
    // clip 变体：用被选 alpha（乘 BaseOpacity 之前）判常量 0.5，严格小于
    // 才丢弃——恰好等于 0.5 的像素存活。
    if selected - 0.5 < 0.0 {
        discard;
    }
#endif
    return vec4<f32>(srgb_format_decode(rgb), selected * params.base_opacity.x);
    }
#endif // 族链（每管线恰一臂）
}
