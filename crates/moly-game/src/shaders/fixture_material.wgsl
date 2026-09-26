// 家具 Basic 族（Mysekai/Fixture/Basic）的 Base 程序。式子照源程序
// platform9 Base 主程序的运算顺序逐句翻译；族内变体用宏分派。
//
// ⚠ **色彩域是这份移植的承重前提，动任何一条算术之前先读下方「色彩域」
//    那一节**（在 `UNBOUND_CUBE_SAMPLE` 之后、顶点输出之前）。整条片元
//    链在**存储（gamma）域**上算，不在线性域——真源是 gamma 色彩空间构建。
//
// 变体键（specialize 注入）：
//   FIXTURE_ALPHA_CLIP  _USE_ALPHA_CLIP：被采 alpha − 0.5 < 0 丢弃。
//                       阈值是编译期常量 0.5（源变体里 `_AlphaClip`
//                       属性整个被剔除，不进 uniform）。
//   FIXTURE_DITHER      未定义 = `_DISABLE_DITHER`：Bayer 抖动块整块
//                       编译期移除（live 材质全数据关抖动，块保留给
//                       未关的变体）。关着时 `_DitherAlpha` 槽不被消费。
//   FIXTURE_WINDOW_CLIP 窗外观支路（fixtureShaderUsage=1 → 二维表落点
//                       26/27）：源以模板缓冲把窗外网裁进窗洞（窗包的
//                       写零面先在洞上写 ref4，外观网按 Equal ref4 只在
//                       洞里画）。本管线没有模板缓冲，等价门在片元里
//                       做：写零面是平面凸四边形，顶点里把四角与本顶点
//                       投到 NDC，片元做「点在凸四边形内」判定，窗外
//                       discard。四角烘在材质 uniform 尾部四槽（写零面
//                       网格顶点，本网格局部系——该 glb 里写零面节点与
//                       外观网节点的局部变换均为恒等，共用本网格的
//                       world_from_local 即正确）。
//   FIXTURE_MODULE_FRESNEL _ENABLE_MODULE_FRESNEL：菲涅尔加色（宝藏阴影
//                       后、雾前）往 rgb 加 exp2(log2(clamp(1−N·V,0,1))
//                       × power) × color.rgb × color.a。live 带它的 3 个
//                       材质全是载具、均未摆放：当前画面上看不见，为载具
//                       摆入准备。
//   FIXTURE_MODULE_REFLECTION _ENABLE_MODULE_REFLECTION：反射加色，紧跟
//                       在菲涅尔支之后、雾之前，往 rgb 加
//                       min(pow(1−N·V, power), 1) × 立方图采样 × intensity。
//                       立方图采样在本管线是**常数**（材质没给它绑对象，
//                       引擎按维度顶上内建默认立方图，那张是六面同色 ⇒
//                       采样与方向/mip/偏置无关）⇒ 源里的反射向量不用算。
//                       ⚠ 这个折叠**只对槽位为空的材质成立**：载具那三个
//                       材质绑了真立方图，需要真采样，未实现、具名挂账。
//   FIXTURE_VIEW_DIR    上面两条**任一**开着时注入：顶点里构造视线方向并
//                       输出（两条分支读同一个顶点输出，源里两个变体的
//                       这段构造逐行相同，所以只有一份）。由 Rust 侧按
//                       「菲涅尔或反射」派生，不是独立的源 keyword。
//   FIXTURE_NORMAL_MAP  JP Basic programs carry an int-gated normal-map block
//                       (`_UseNormalMap` > 0.5) in every variant. The int is a
//                       per-material constant, so the block is specialised on
//                       it. The frame is the screen-derivative cotangent frame
//                       built from uv1 (TEXCOORD9 = in_TEXCOORD1), sampled at
//                       uv1 optionally shifted by `_NormalParallaxShift`.
//                       Only admitted together with FIXTURE_CRYSTAL: that is
//                       the only variant the data uses it with, and the
//                       variants differ in which normal later stages read.
//   FIXTURE_CRYSTAL     `_ENABLE_CRYSTAL_FIXTURE` (JP 6.8.1): after the fogged
//                       and clamped colour, the light-influence mix towards the
//                       texel, the normal-driven colour remap and the bright
//                       fresnel add. Its programs always carry the fresnel and
//                       reflection keywords too.
//   FIXTURE_MULTI_UV    `_ENABLE_MULTI_UV_SCROLL` (JP 6.8.1): the base colour is
//                       not `_MainTex` but up to three `_LayerTex*` layers,
//                       sampled at world-planar coordinates (the position
//                       projected on a horizontal tangent frame built in the
//                       vertex stage) with per-layer tiling, scroll and a
//                       view-dependent parallax offset, and composited over
//                       each other by layer alpha. The rest of the chain is
//                       the plain Basic one; its programs carry no clip,
//                       dither or texture-branch keywords.
// 两条质感分支的参数（菲涅尔两参 + 反射两参）合用 binding 4 的那一块，
// 不并进 FixtureParams（那份 12 槽序与自发光 pass 的对象池布局共用）。
// 顶点属性键由引擎按网格布局注入：VERTEX_NORMALS / VERTEX_UVS_A。
//
// 与源的两点已知差异（同站点族的既有裁决）：
// - 源程序写两个颜色目标（SV_Target1 = 自发射遮罩 × 现象自发光开关
//   链：类型 1 时门 = bright==1、类型 2 时门 = dark==1、其余 0，随后
//   override 模式 1 强置 1 / 2 强置 0；两目标 alpha 都 = 主贴图 alpha）。
//   本管线的主 pass 只有一个颜色目标（与站点族同一裁决）；第二目标由
//   自建的自发光 pass 承接（`fixture_emission.wgsl`，直画进泛光的输入
//   缓冲），门链与 discard 链在那里逐句同源。主程序的这三条路径不
//   消费那三个材质 int 与 `_EmissionMaskTex`。
// - 源的抖动屏幕坐标走 clip 位（TEXCOORD3.xy/ww × 屏参），本管线用
//   片元内置坐标；两者在像素中心逐值相等（同左下原点、同半像素
//   偏移），站点族已按同一对账采同一路。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::skinning
#import bevy_pbr::view_transformations::position_world_to_clip

// ---- 材质 uniform（group 3 binding 0）：每条 Unity 属性一个 vec4 槽 ----
// 槽序与 `fixture_material.rs` 里 `FixtureParams` 的字段序是契约，两边同改。
// 源 Base 程序不消费的属性（_BaseOpacity、顶点色）没有槽；fresnel 两参
// 走独立的 binding 4（那份 12 槽序与自发光 pass 的对象池布局共用，不能
// 扩）；雾走全局 env 槽（group 3 binding 1），与站点族共用同一份。

struct FixtureParams {
    // xy = (_MainTexOffsetX, _MainTexOffsetY)：顶点里加进 uv0。
    main_tex_offset: vec4<f32>,
    dither_alpha: vec4<f32>,
    use_phenomena_lighting: vec4<f32>,
    override_shading_parameter: vec4<f32>,
    local_shading_intensity: vec4<f32>,
    local_edge_threshold: vec4<f32>,
    local_edge_smoothness: vec4<f32>,
    // Source UV origin is bottom-left; exported image rows begin at the top.
    uv_v_flip: vec4<f32>,
    // 窗外观门四角（xyz 用，w 填 0）：写零面网格顶点，本网格局部系，
    // 已排成平面凸环序。非窗材质全零且消费代码整块编译期移除。
    clip_corner_0: vec4<f32>,
    clip_corner_1: vec4<f32>,
    clip_corner_2: vec4<f32>,
    clip_corner_3: vec4<f32>,
}

// ---- 全局量（group 3 binding 1）：一帧一份，与站点材质共用同一个 buffer ----
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
    // 现象自发光类型（全局 int `_MysekaiPhenomenaEmissionType`，f32 存整
    // 数值）。Base 主程序不消费它（第二颜色目标的自发光门在自发光 pass
    // 里消费，见 fixture_emission.wgsl）；槽带着，两侧共用同一份表。
    emission_type: vec4<f32>,
}

@group(3) @binding(0) var<uniform> params: FixtureParams;
@group(3) @binding(1) var<uniform> env: SiteEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;

// ---- 质感分支两参块（group 3 binding 4）----
// 单独一块的缘故见文件头变体键说明：FixtureParams 的槽序是跨 pass 的
// 字节契约。分支关着的变体不声明消费它（特化整块移除），绑定照常
// 存在于共享布局里。这一块只在主 pass 的片元里读，两条质感分支共用。

struct FixtureShadingBranch {
    // x = _FresnelPower。
    fresnel_power: vec4<f32>,
    // _FresnelColor：xyz 是项色，w 是因子（源序里项要乘 color.a）。
    fresnel_color: vec4<f32>,
    // 反射支两参：x = _ReflectionFresnelPower、y = _ReflectionIntensity。
    // zw 填 0（源里这条分支没有别的数值输入——立方图那一项在本管线是
    // 折叠掉的常数，见片元里的说明）。
    reflection: vec4<f32>,
    // Crystal: (_LightInfluenceScale, _UseNormalDrivenEmission,
    // _NormalDarkToColor, _NormalBrightToEmission).
    crystal_colour: vec4<f32>,
    // Crystal: (_UseBrightFresnelAdd, _BrightFresnelAddIntensity, 0, 0).
    crystal_fresnel: vec4<f32>,
    // Normal map: (_BumpScale, _NormalExaggeration, _UseNormalParallaxShift,
    // _NormalParallaxShift).
    normal_map: vec4<f32>,
    // Multi-UV layers: `_LayerTex1_ST`, `_LayerTex2_ST`, `_LayerTex3_ST`.
    layer1_st: vec4<f32>,
    layer2_st: vec4<f32>,
    layer3_st: vec4<f32>,
    // (_LayerTex1_UVScrollX, _LayerTex1_UVScrollY, _LayerTex2_UVScrollX, _LayerTex2_UVScrollY).
    layer_scroll_12: vec4<f32>,
    // (_LayerTex3_UVScrollX, _LayerTex3_UVScrollY, _UseLayerTex2, _UseLayerTex3).
    layer_scroll_3_use: vec4<f32>,
    // (_LayerTex1/2/3_ParallaxStrength, 0).
    layer_parallax: vec4<f32>,
}

@group(3) @binding(4) var<uniform> branch_params: FixtureShadingBranch;
// `_NormalMap`. Materials without the normal-map block bind a fallback that
// no compiled variant reads (the layout is shared by every variant).
@group(3) @binding(5) var normal_tex: texture_2d<f32>;
@group(3) @binding(6) var normal_sampler: sampler;
// `_LayerTex1/2/3` of the multi-UV variant (fallbacks elsewhere, unread).
@group(3) @binding(7) var layer1_tex: texture_2d<f32>;
@group(3) @binding(8) var layer1_sampler: sampler;
@group(3) @binding(9) var layer2_tex: texture_2d<f32>;
@group(3) @binding(10) var layer2_sampler: sampler;
@group(3) @binding(11) var layer3_tex: texture_2d<f32>;
@group(3) @binding(12) var layer3_sampler: sampler;

// 未绑定的立方图采样器采出来的常数（每通道 128/255）。引擎按纹理**维度**
// 索引它的内建默认贴图表顶上去，而立方图那一格是六面各自整面清成同一个
// 颜色的 ⇒ **采样值与方向、mip 级、mip 偏置、过滤模式全部无关**，于是源里
// 那次 `texture(立方图, R, bias)` 折叠成这一个常数，反射向量 R 连算都不用算。
// 工程跑在 gamma 空间 ⇒ 引擎给它选的是无 sRGB 标记的格式 ⇒ 采样不做 sRGB
// 解码，128/255 不打折。出处与适用边界（**只对立方图槽位为空的材质成立**；
// 载具那三个材质绑了真立方图，本常数对它们是错的）在
// `moly_law::fixture::fresnel` 的 `UNBOUND_CUBE_SAMPLE` 说明里。
// ⚠ 别把它「化简」成 128.0 / 255.0：那个除法在常量求值里要经两次舍入，
// 与律那侧的位不保证一致；这个十进制是该 f32 的最短往返形。
const UNBOUND_CUBE_SAMPLE: f32 = 0.5019608;

// ---- 色彩域（改这个文件里任何一条算术之前先读）----
//
// 真源是 **gamma 色彩空间**构建：贴图采样不解码、片元里整条链在存储
// （编码）域上算、结果按编码值直写帧缓冲。本管线两头都不同：
//
//   1 家具主贴图是 glTF 的 `base_color_texture`（`fixture_material.rs` 里
//     从 `StandardMaterial` 取），**bevy_gltf 恒按 sRGB 装载、loader
//     settings 没有旋钮** ⇒ 采样时硬件按 sRGB EOTF 解码，
//     `textureSampleBias` 给出的是线性值；
//   2 本管线的色彩目标是 sRGB 格式 ⇒ 写出时硬件按 sRGB OETF 再编码一次。
//
// ⇒ 采样后先 [`srgb_format_encode`] 编回存储域，中间整条链与源逐式同域，
//   写出前 [`srgb_format_decode`] 解一次、硬件那一次编码恰好还原。**两步
//   都不是源的式子，是域适配**——源没有硬件的那两步，所以它也没有对应的逆。
//
// ⚠ **紧挨着的 `UNBOUND_CUBE_SAMPLE` 就是这条前提的独立证人，而在本节落地
//   之前它和代码是互相打脸的**：那段注释自己写着「工程跑在 gamma 空间 ⇒
//   引擎给它选的是无 sRGB 标记的格式 ⇒ 采样不做 sRGB 解码，128/255 不打折」
//   ——也就是说它是一个**存储域**的常数；可当时反射支把它加到的那个 `rgb`
//   是**解码后的线性值**。两句话都在这个文件里，没人把它们连起来。
//   ⇒ 留这段是因为本仓的规矩：一条设计的理由必须留在代码里。谁要是把下面
//   两次域变换当成多余的开销删掉，这个常数会立刻重新变成一个域错，而
//   **没有任何判据会红**（当前摆位下带反射分支的 3 个材质全是未摆放的载具）。
//
// ⚠ **uniform 颜色与顶点色不参与这两步**：颜色 uniform 两侧同为原始浮点
//   直推（不做域转换）⇒ 本来就在存储域；wgpu 的顶点格式**没有 sRGB 变体**，
//   硬件不可能解码一个顶点属性。
//
// ⚠ **仓里并存两族 gamma 曲线，名字刻意不同族，别拿错**：
//   · **格式**那一族（IEC 61966-2-1）= 本文件的 `srgb_format_encode` /
//     `srgb_format_decode`，抵消的是**硬件**在采样与写出上做的那两步，
//     指数写 `1/2.4`。`srgb_format_decode` 逐式同
//     `moly_law::weather::sky::srgb_target_value`（律侧是唯一权威形）。
//   · **引擎**那一族 = `moly_law::weather::sky::gamma_to_linear` /
//     `linear_to_gamma`，多一支 2.2 幂、指数是源里的截断字面量，只用在
//     **源自己**做 gamma 往返的地方。两族混用会在低端偏若干码。
//
// ⚠ **alpha 不参与**：硬件的 sRGB 变换只作用于 rgb 三通道，`base.a` 就是
//   存储值 ⇒ clip 阈值与输出 alpha 都不改。

// 采样值 → 存储域：抵消硬件的 sRGB 解码（格式 OETF）。
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}

// 存储域 → 交给 sRGB 色彩目标的值（格式 EOTF）。先钳与硬件后钳等价
// （曲线单调）——源末行那次 `clamp(rgb, 0, 1)` 由这里的钳承接，语义未变。
fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

// ---- 顶点输出 ----

struct FixtureVertexOutput {
    @builtin(position) position: vec4<f32>,
    // 源的 TEXCOORD2：世界坐标（宝藏阴影读 xz）。
    @location(0) world_position: vec3<f32>,
    // 源的 TEXCOORD1：逆转置法线，片元里再归一化一次（源的形状）。
    @location(1) world_normal: vec3<f32>,
    // 源的 TEXCOORD0：uv0 + (_MainTexOffsetX, _MainTexOffsetY)。
    @location(2) uv: vec2<f32>,
    // 雾斜坡：顶点里算好（源的 vFogRamp）。
    @location(3) fog_ramp: f32,
#ifdef FIXTURE_WINDOW_CLIP
    // 窗外观门：四角 NDC。四角对全网格是常量，插值不引入误差。
    @location(4) clip_corner_0: vec2<f32>,
    @location(5) clip_corner_1: vec2<f32>,
    @location(6) clip_corner_2: vec2<f32>,
    @location(7) clip_corner_3: vec2<f32>,
    // 四角全部在视平面前方（clip.w > 0）时为 1；有角在身后时 NDC 除法
    // 不可信，门放行（此时窗外网自身也大半被近面裁掉，放行不是漏）。
    @location(8) clip_valid: f32,
    // 本片元 NDC：clip.xy/clip.w 在顶点算好。NDC 是屏幕仿射量，透视
    // 校正插值逐值精确（与影贴图坐标同一条对账路）。
    @location(9) ndc: vec2<f32>,
#endif
#ifdef FIXTURE_VIEW_DIR
    // 源的 TEXCOORD5（质感分支变体独有）：视线方向，片元里再归一化
    // 一次（源的形状）。与窗外观门同开时各用各的 location。
    // 门是「菲涅尔或反射任一条开着」——两条分支读的是**同一个**顶点
    // 输出，源里两个变体的这段构造逐行相同（同一个正交/透视选择、
    // 同一个相机位减片元位），所以只有一份，不按分支复制。
    @location(10) view_dir: vec3<f32>,
#endif
#ifdef FIXTURE_NORMAL_MAP
    // The source's TEXCOORD9: raw uv1 in the source's bottom-left origin
    // (no offset). The cotangent frame differentiates it as is; only the
    // texture fetch converts the row origin.
    @location(11) uv1: vec2<f32>,
#endif
#ifdef FIXTURE_MULTI_UV
    // The source's TEXCOORD7 / TEXCOORD8 / TEXCOORD6: the horizontal tangent,
    // the bitangent and the view direction in that frame.
    @location(12) layer_tangent: vec3<f32>,
    @location(13) layer_bitangent: vec3<f32>,
    @location(14) layer_view: vec3<f32>,
#endif
}

@vertex
fn vertex(mesh: Vertex) -> FixtureVertexOutput {
#ifdef SKINNED
    // Furniture can be a SkinnedMeshRenderer too (e.g. the fridge door).
    // The authored joint palette already includes world transforms and bind
    // poses; applying only the mesh instance transform leaves it in rest pose.
    let world_from_local = skinning::skin_model(
        mesh.joint_indices, mesh.joint_weights, mesh.instance_index,
    );
#else
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
#endif
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(mesh.position, 1.0),
    );

    var out: FixtureVertexOutput;
    out.position = position_world_to_clip(world_position.xyz);
    out.world_position = world_position.xyz;

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
    let uv = mesh.uv + params.main_tex_offset.xy - env.time.y * params.main_tex_offset.zw;
    // Preserve source offsets before converting the texture coordinate origin.
    out.uv = vec2<f32>(
        uv.x,
        select(uv.y, 1.0 - uv.y, params.uv_v_flip.x > 0.5),
    );
#else
    let uv = params.main_tex_offset.xy;
    out.uv = vec2<f32>(
        uv.x,
        select(uv.y, 1.0 - uv.y, params.uv_v_flip.x > 0.5),
    );
#endif

    // 雾深：与站点族同一条对账路——源读 GL 约定下的 clip.z，式子
    // ((z+n)/(n+f))·f 在该约定下恰等于 f·(clip.w − n)/(f − n)，而
    // clip.w（眼空间前向深度）在两种 clip 约定下相同，用 clip.w 重建。
    let n = env.projection_params.y;
    let f = env.projection_params.z;
    var fog_depth = f * (out.position.w - n) / (f - n);
    fog_depth = max(fog_depth, 0.0);
    fog_depth = fog_depth * env.fog_params.x + env.fog_params.y;
    out.fog_ramp = clamp(fog_depth, 0.0, 1.0);

#ifdef FIXTURE_WINDOW_CLIP
    // 窗外观门：写零面四角与本顶点同局部系，共用本网格的
    // world_from_local 与投影（该 glb 的写零面节点局部变换为恒等）。
    out.ndc = out.position.xy / out.position.w;
    var corner_valid = true;
    // var（而非 let）：四角数组要被循环变量动态索引，值得有函数地址
    // 空间背书，naga 对 let 数组的动态索引不保证放行。
    var corners = array<vec4<f32>, 4>(
        params.clip_corner_0,
        params.clip_corner_1,
        params.clip_corner_2,
        params.clip_corner_3,
    );
    var corners_ndc = array<vec2<f32>, 4>(
        vec2<f32>(0.0),
        vec2<f32>(0.0),
        vec2<f32>(0.0),
        vec2<f32>(0.0),
    );
    for (var i = 0; i < 4; i++) {
        let corner_world = mesh_functions::mesh_position_local_to_world(
            mesh_functions::get_world_from_local(mesh.instance_index),
            corners[i],
        );
        let corner_clip = position_world_to_clip(corner_world.xyz);
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

#ifdef FIXTURE_VIEW_DIR
    // 视线方向：透视相机 = 相机位减片元位按 inversesqrt(dot) 归一化
    //（源在这里没有 epsilon 钳制，与法线支不同）；正交相机直接取视图
    // 矩阵第三行——列主序的 MatrixV[0].z/[1].z/[2].z 即本表 c0.z/c1.z/c2.z。
    let to_camera = env.camera_position.xyz - world_position.xyz;
    out.view_dir = select(
        to_camera * inverseSqrt(dot(to_camera, to_camera)),
        vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z),
        env.ortho_params.w != 0.0,
    );
#endif

#ifdef FIXTURE_NORMAL_MAP
#ifdef VERTEX_UVS_B
    out.uv1 = mesh.uv_b;
#else
    // A mesh without the channel reads the engine's zero vertex input.
    out.uv1 = vec2<f32>(0.0);
#endif
#endif

#ifdef FIXTURE_MULTI_UV
    // Source: T = normalize_xz(N.z, 0, -N.x), B = cross(N, T), with N the
    // renormalised world normal and V the normalised direction to the camera;
    // TEXCOORD6 = (dot(V.xz, T.xz), dot(V, B), dot(V, N)). The constant axes in
    // T do not reflect with the world, so in this pipeline's x-reflected world
    // the reflected source frame is T = (-N.z, 0, N.x), B = cross(T, N); every
    // dot product (and the planar coordinates in the fragment) then equals the
    // source's.
    let layer_normal = normalize(out.world_normal);
    let layer_to_camera = env.camera_position.xyz - world_position.xyz;
    let layer_view_dir = inverseSqrt(dot(layer_to_camera, layer_to_camera)) * layer_to_camera;
    var layer_tangent = vec3<f32>(-layer_normal.z, 0.0, layer_normal.x);
    layer_tangent = inverseSqrt(dot(layer_tangent.xz, layer_tangent.xz)) * layer_tangent;
    let layer_bitangent = cross(layer_tangent, layer_normal);
    out.layer_view = vec3<f32>(
        dot(layer_view_dir.xz, layer_tangent.xz),
        dot(layer_view_dir, layer_bitangent),
        dot(layer_view_dir, layer_normal),
    );
    out.layer_tangent = layer_tangent;
    out.layer_bitangent = layer_bitangent;
#endif

    return out;
}

// ---- 片元共用件 ----

// 2D 叉积（z 分量）：窗外观门的边侧判定用。
fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

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

// 雾：与站点族同一份（源在雾 pass 关时把两支 alpha 强写 0，合成端
// 系数精确归零直通——无需 enabled 门）。式子是 demo 片元律的等价形：
// delta = (1−ramp)·(fogCol − rgb)，终值 = mix(rgb, fogCol.rgb, h·a·(1−ramp))。
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

// 4×4 Bayer 表与站点族同一张（源经 one-hot 点积还原出的行 × 列）；
// 差异只在屏幕缩放：家具 Base 的源式是屏幕坐标 × 0.125（站点
// FieldObject 是 × 0.25），一格覆盖 2 像素、周期 8 像素。
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
    // 表按 [x][y] 读（x 为行）：demo 的取法是 table[ix*4+iy]。
    // 此前这里写成 iy*4+ix（转置读法）——live 材质 _DisableDither 全 1，
    // 块编译期移除，零观测差；按判据修齐到与 demo 同向。
    let value = table[ix * 4 + iy];
    return value * 0.0618750006 + 0.00999999978;
}

#ifdef FIXTURE_MULTI_UV
// One layer's sample point: planar coordinates tiled by the layer's ST, the
// scroll subtracted with `_Time.y`, then the parallax offset along `dir`.
fn multi_uv_layer_uv(planar: vec2<f32>, st: vec4<f32>, scroll: vec2<f32>, strength: f32,
                     weight: f32, dir: vec2<f32>) -> vec2<f32> {
    var uv = planar * st.xy + st.zw;
    uv = vec2<f32>(-env.time.y) * scroll + uv;
    let offset = weight * strength * 0.5;
    uv = vec2<f32>(offset) * dir + uv;
    // Source uv space -> exported image rows (top first).
    return vec2<f32>(uv.x, select(uv.y, 1.0 - uv.y, params.uv_v_flip.x > 0.5));
}

// The multi-UV base colour (rgb in the stored domain) and alpha, statement by
// statement: layer 1 always, layers 2 and 3 behind their int gates, each
// composited over the result by its own alpha.
fn multi_uv_base(in: FixtureVertexOutput) -> vec4<f32> {
    let planar = vec2<f32>(
        dot(in.world_position, in.layer_tangent),
        dot(in.world_position, in.layer_bitangent),
    );
    // Parallax weight: z = sat(V.N), t = sat((z - 0.5) * 2.50000024),
    // w = smoothstep(t) * (z^3 - 1) + 1.
    let z = clamp(in.layer_view.z, 0.0, 1.0);
    let t = clamp((z + -0.5) * 2.50000024, 0.0, 1.0);
    let smooth_t = t * t * (t * -2.0 + 3.0);
    let weight = smooth_t * (z * z * z + -1.0) + 1.0;
    // Offset direction: normalize(TEXCOORD6).xy / (normalize(TEXCOORD6).z + 0.42).
    let view_scale = inverseSqrt(dot(in.layer_view, in.layer_view));
    let dir = (view_scale * in.layer_view.xy) / (in.layer_view.z * view_scale + 0.419999987);
    let uv1 = multi_uv_layer_uv(planar, branch_params.layer1_st,
        branch_params.layer_scroll_12.xy, branch_params.layer_parallax.x, weight, dir);
    let uv2 = multi_uv_layer_uv(planar, branch_params.layer2_st,
        branch_params.layer_scroll_12.zw, branch_params.layer_parallax.y, weight, dir);
    let uv3 = multi_uv_layer_uv(planar, branch_params.layer3_st,
        branch_params.layer_scroll_3_use.xy, branch_params.layer_parallax.z, weight, dir);
    // All three fetches run in uniform control flow; the int gates then select
    // (the source fetches only behind the gates, the selected values are equal).
    let l1 = textureSampleBias(layer1_tex, layer1_sampler, uv1, env.mip_bias.x);
    let l2 = textureSampleBias(layer2_tex, layer2_sampler, uv2, env.mip_bias.x);
    let l3 = textureSampleBias(layer3_tex, layer3_sampler, uv3, env.mip_bias.x);
    let c1 = srgb_format_encode(l1.rgb);
    let c2 = srgb_format_encode(l2.rgb);
    let c3 = srgb_format_encode(l3.rgb);
    var rgb = c1;
    var alpha = l1.a;
    if branch_params.layer_scroll_3_use.z > 0.5 {
        rgb = vec3<f32>(l2.a) * (c2 - c1) + c1;
        alpha = (1.0 - l1.a) * l2.a + l1.a;
    }
    if branch_params.layer_scroll_3_use.w > 0.5 {
        rgb = vec3<f32>(l3.a) * (c3 - rgb) + rgb;
        alpha = (1.0 - alpha) * l3.a + alpha;
    }
    return vec4<f32>(rgb, alpha);
}
#endif

#ifdef FIXTURE_NORMAL_MAP
// The JP Basic normal-map block, statement by statement. `n` is the
// normalised interpolated normal, the result is the perturbed normal.
//
// Frame: the cotangent frame from screen derivatives of the world position
// and of uv1. Two sign conventions meet here and cancel: WGSL's dpdy runs
// down the framebuffer (the source's dFdy runs up, so dpdy = -dFdy for both
// the position and the uv), and this pipeline's world is the source's world
// reflected in x (a cross product of reflected vectors is minus the reflected
// cross product). Worked through, T and B come out as the reflection of the
// source's T and B, which is exactly how every other vector here relates to
// the source, so the source statements are transcribed unchanged.
//
// The fetch is the only place the row origin changes (exported images start
// at the top); the parallax shift is added in the source's uv space first.
fn crystal_normal(n: vec3<f32>, world_position: vec3<f32>, uv1: vec2<f32>) -> vec3<f32> {
    let duv_x = dpdx(uv1);
    let duv_y = dpdy(uv1);
    let dpos_x = dpdx(world_position);
    let dpos_y = dpdy(world_position);
    // dp1perp = N.yzx * dx.zxy - N.zxy * dx.yzx; dp2perp = dy.yzx * N.zxy - N.yzx * dy.zxy.
    let dp1perp = n.yzx * dpos_x.zxy - n.zxy * dpos_x.yzx;
    let dp2perp = dpos_y.yzx * n.zxy - n.yzx * dpos_y.zxy;
    let tangent_raw = dp2perp * duv_x.x + duv_y.x * dp1perp;
    let bitangent_raw = dp2perp * duv_x.y + duv_y.y * dp1perp;
    let frame_scale = inverseSqrt(max(dot(tangent_raw, tangent_raw), dot(bitangent_raw, bitangent_raw)));
    let tangent = frame_scale * tangent_raw;
    let bitangent = frame_scale * bitangent_raw;
    let to_camera = env.camera_position.xyz - world_position;
    let view = inverseSqrt(dot(to_camera, to_camera)) * to_camera;
    let shifted = vec2<f32>(dot(tangent, view), dot(bitangent, view))
        * vec2<f32>(branch_params.normal_map.w) + uv1;
    let uv = select(uv1, shifted, branch_params.normal_map.z > 0.5);
    let fetch_uv = vec2<f32>(uv.x, select(uv.y, 1.0 - uv.y, params.uv_v_flip.x > 0.5));
    // Stored-domain texel (see the colour-domain section): the source runs in
    // gamma space and reads the stored bytes.
    let texel = srgb_format_encode(
        textureSampleBias(normal_tex, normal_sampler, fetch_uv, env.mip_bias.x).rgb,
    );
    var decoded = texel * vec3<f32>(2.0) + vec3<f32>(-1.0);
    decoded = vec3<f32>(decoded.xy * vec2<f32>(branch_params.normal_map.x), decoded.z);
    var bumped = bitangent * decoded.y;
    bumped = decoded.x * tangent + bumped;
    bumped = decoded.z * n + bumped;
    bumped = bumped * inverseSqrt(dot(bumped, bumped)) - n;
    bumped = vec3<f32>(branch_params.normal_map.y) * bumped + n;
    return inverseSqrt(dot(bumped, bumped)) * bumped;
}
#endif

@fragment
fn fragment(in: FixtureVertexOutput) -> @location(0) vec4<f32> {
    // Sample before per-fragment clipping so implicit derivatives remain uniform.
    let base = textureSampleBias(main_tex, main_sampler, in.uv, env.mip_bias.x);
#ifdef FIXTURE_MULTI_UV
    // The layers replace the main texture as the base colour and alpha (the
    // main texture is still sampled above; nothing reads it in this variant).
    let multi_base = multi_uv_base(in);
    let base_stored = multi_base.rgb;
    let base_alpha = multi_base.a;
#else
    let base_stored = srgb_format_encode(base.rgb);
    let base_alpha = base.a;
#endif
#ifdef FIXTURE_NORMAL_MAP
    // The source runs the normal-map block before its discard as well; the
    // screen derivatives and the fetch stay in uniform control flow here.
    let lit_normal = crystal_normal(normalize(in.world_normal), in.world_position, in.uv1);
#endif
#ifdef FIXTURE_WINDOW_CLIP
    // 窗外观门（等价模板 Equal ref4）：片元 NDC 不在写零面投影凸四边形
    // 内 → discard。环向两种都可能（从背面看四边形翻转），按四边形自身
    // 有向面积把四个边侧判定归一到同一侧；投影退化成线（orientation
    // 为 0）时全等号成立、门放行——那个视角下窗外网自身也近乎侧对
    // 相机，放行可见面积极小，且比误杀整个窗洞安全。
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
    // 主贴图带全局 mip 偏置采样。
    // alpha clip 变体：tex.a − 0.5 < 0 丢弃（阈值编译期常量）。
#ifdef FIXTURE_ALPHA_CLIP
    if base.a - 0.5 < 0.0 {
        discard;
    }
#endif
    // Bayer 抖动：dither − 阈值 < 0 丢弃（`_DISABLE_DITHER` 变体整块移除）。
#ifdef FIXTURE_DITHER
    if params.dither_alpha.x - fixture_bayer_value(in.position.xy) < 0.0 {
        discard;
    }
#endif

#ifdef FIXTURE_FENCE
    // Fence normalizes at the vertex only; retain the interpolated normal length.
    let normal = in.world_normal;
#else
    let normal = normalize(in.world_normal);
#endif
    // The normal the toon ramp and the reflection read. With the normal-map
    // block it is the perturbed normal; the fresnel module and the crystal
    // fresnel add keep reading the interpolated one (the crystal programs
    // keep both in separate registers).
#ifndef FIXTURE_NORMAL_MAP
    let lit_normal = normal;
#endif
    // Half-lambert of the lit normal. The crystal tail reads it again, so it
    // is computed outside the phenomena gate (the source computes it before
    // the select, too; it has no side effects).
    let half_lambert = dot(env.light_vector.xyz, lit_normal) * 0.5 + 0.5;

    // 现象光照门：关时 rgb 直通（源的 select 形状，两侧都算后选门）。
    // `base_rgb` 是编回存储域的主贴图色（见上方「色彩域」）——现象色、
    // 阴影色、菲涅尔色、雾色、`UNBOUND_CUBE_SAMPLE` 全在存储域，同域才
    // 是源的那个函数。
    let base_rgb = base_stored;
    var rgb = base_rgb;
    if params.use_phenomena_lighting.x > 0.5 {
        let lit = phenomena_light_blend(base_rgb);
#ifdef FIXTURE_RUG
        rgb = lit;
#else
        // toon 阴影：阈值/平滑度/强度按 _OverrideShadingParameter 选局部或全局。
        let use_local = params.override_shading_parameter.x > 0.5;
        let threshold = select(env.edge_threshold.x, params.local_edge_threshold.x, use_local);
        let smoothness = select(env.edge_smoothness.x, params.local_edge_smoothness.x, use_local);
        let intensity = select(1.0, params.local_shading_intensity.x, use_local);
        let upper = threshold + smoothness;
        let lower = threshold - smoothness;
        var ramp = (half_lambert - upper) / (lower - upper);
        ramp = clamp(ramp, 0.0, 1.0);
        let factor = intensity * ramp;
        rgb = lit + factor * (env.phenomena_shade_color.rgb * lit - lit);
#endif
    }

    // Fence Base has no treasure shadow inputs.
#ifndef FIXTURE_FENCE
    // 宝藏阴影两项。
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);
#endif

    // 质感分支的两条加色支路（`_ENABLE_MODULE_FRESNEL` /
    // `_ENABLE_MODULE_REFLECTION`，各自独立开关，可单开也可同开）。
    // N 与 V 都在片元里再归一化一次（源的形状）。
    //
    // ⚠ **`1 − N·V` 只算一次，两支各取一份**：源里两条分支同开的那个变体
    // 里这个减法只出现一次，菲涅尔支吃 `clamp(·,0,1)` 之后的副本、反射支
    // 吃**未钳的原件**。写成各算一次、或让两支共用同一个钳后值，都是另一
    // 个函数（反射支的上钳存在的理由正是未钳值会大于 1）。
    // 逐句律在 moly-law 的 fixture::fresnel。
#ifdef FIXTURE_VIEW_DIR
    let view_dir = normalize(in.view_dir);
    let n_dot_v = dot(normal, view_dir);
    let one_minus_raw = 1.0 - n_dot_v;
#endif

    // 菲涅尔加色（宝藏阴影后、雾前）：源序 clamp(1−N·V, 0, 1) → log2 →
    // ×power → exp2 → ×color.rgb → ×color.a → +rgb。
#ifdef FIXTURE_MODULE_FRESNEL
    let one_minus = clamp(one_minus_raw, 0.0, 1.0);
    let fresnel = exp2(log2(one_minus) * branch_params.fresnel_power.x);
    rgb = (vec3<f32>(fresnel) * branch_params.fresnel_color.rgb)
        * branch_params.fresnel_color.a
        + rgb;
#endif

    // 反射加色：源序 (1−N·V) → log2（**入参无钳**）→ ×power → exp2 →
    // min(·, 1)（**只上钳**）→ ×立方图采样 → ×intensity → +rgb。
    // 立方图那一项在本管线是常数（未绑定采样器 ⇒ 引擎的内建默认立方图
    // 六面同色 ⇒ 采样与方向无关），所以源里那个反射向量 R = V − 2(N·V)N
    // 在这里连算都不用算——它唯一的用处是当采样方向，而采样值与方向无关。
    // 乘法结合序照源：**采样常数先乘，intensity 后乘**，两次乘法不合并成
    // 一个预乘常数（f32 乘法不满足结合律，折叠只替换取值那一步）。
#ifdef FIXTURE_MODULE_REFLECTION
    // With the normal-map block the reflection reads the perturbed normal
    // (its own dot product in the source); otherwise the shared subtraction.
#ifdef FIXTURE_NORMAL_MAP
    let reflection_one_minus = 1.0 - dot(view_dir, lit_normal);
#else
    let reflection_one_minus = one_minus_raw;
#endif
    let reflection_fresnel = exp2(log2(reflection_one_minus) * branch_params.reflection.x);
    let reflection_clamped = min(reflection_fresnel, 1.0);
    rgb = vec3<f32>((UNBOUND_CUBE_SAMPLE * reflection_clamped) * branch_params.reflection.y)
        + rgb;
#endif

    // 雾：无条件应用（源在宝藏阴影之后、现象光门之外——门关时直通的
    // rgb 也吃雾，与 Tree 族「雾在门内」不同，照 demo 的次序）。
#ifndef FIXTURE_FENCE
    rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);
#endif

#ifdef FIXTURE_CRYSTAL
    // Crystal tail, after the fogged colour has been clamped (apply_fog ends
    // with the source's clamp). Three steps, each an int or float gate read
    // at run time as in the source:
    // 1. `_LightInfluenceScale` < 1: mix from the texel towards the lit colour
    //    by the clamped scale.
    let influence = branch_params.crystal_colour.x;
    let influenced = vec3<f32>(clamp(influence, 0.0, 1.0)) * (rgb - base_rgb) + base_rgb;
    rgb = select(rgb, influenced, influence < 1.0);
    // 2. `_UseNormalDrivenEmission`: rgb * dark + hl * (rgb + bright - rgb * dark),
    //    hl being the half-lambert of the lit normal (unclamped).
    let dark_to_colour = branch_params.crystal_colour.z;
    let bright_to_emission = branch_params.crystal_colour.w;
    let driven_dark = rgb * vec3<f32>(dark_to_colour);
    var driven = rgb + vec3<f32>(bright_to_emission);
    driven = -rgb * vec3<f32>(dark_to_colour) + driven;
    driven = vec3<f32>(half_lambert) * driven + driven_dark;
    rgb = select(rgb, driven, branch_params.crystal_colour.y > 0.5);
    // 3. `_UseBrightFresnelAdd`: hl * (1 - sat(N.V))^max(power, 0.001) * fresnel
    //    colour * intensity, the intensity capped at 0.1 when phenomena
    //    lighting is off. V is rebuilt per fragment from the camera position
    //    (not the vertex view direction), N is the interpolated normal.
    let crystal_to_camera = env.camera_position.xyz - in.world_position;
    let crystal_view = inverseSqrt(dot(crystal_to_camera, crystal_to_camera)) * crystal_to_camera;
    let crystal_n_dot_v = clamp(dot(normal, crystal_view), 0.0, 1.0);
    let crystal_power = max(branch_params.fresnel_power.x, 0.00100000005);
    var crystal_rim = exp2(log2(1.0 - crystal_n_dot_v) * crystal_power);
    let rim_intensity = select(
        branch_params.crystal_fresnel.y,
        min(branch_params.crystal_fresnel.y, 0.100000001),
        params.use_phenomena_lighting.x < 0.5,
    );
    crystal_rim = half_lambert * crystal_rim;
    let rim_added = (vec3<f32>(crystal_rim) * branch_params.fresnel_color.rgb) * vec3<f32>(rim_intensity) + rgb;
    rgb = select(rgb, rim_added, branch_params.crystal_fresnel.x > 0.5);
    // The source writes this without a further clamp; its colour target is
    // 8-bit unorm, so the store clamps. `srgb_format_decode` clamps the same.
#endif

    // 输出 alpha = tex.a（源的第二颜色目标连同自发光开关链挂账，见文件头
    // 注释；`_BaseOpacity` 在 Base 程序里不被消费）。
    // 源末行那次 `clamp(rgb, 0, 1)` 由 `srgb_format_decode` 里的钳承接
    // （它先钳后解，与「钳在存储域」逐值等价）——不是把 clamp 删了。
    return vec4<f32>(srgb_format_decode(rgb), base_alpha);
}
