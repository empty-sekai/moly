// 雨粒子的着色：billboard 装配 + 片元链。体例与站点族一致——式子按
// 源程序的运算顺序逐句翻译，闸掉的环写明是谁闸的。
//
// 与站点族不同处：这个族的网格是**逐帧重建的属性池**，四个标准属性
// 各自换了语义（律侧状态直接喂顶点属性）：
//   POSITION = 粒子世界中心（雨实体的变换恒等，属性即世界坐标）
//   UV_0     = 四边形角点（0..1）
//   UV_1     = 粒子 X/Y 全尺寸（米）
//   COLOR    = 逐粒子颜色（colorOverLifetime 求值）
//
// 顶点装配是演示件同名模式的直译：视空间原点 + 角点×尺寸。片元链
// 是逐段判读后的子集：本材质（雨滴）的染色环被 HDR 闸在 Rust 侧拒掉、
// 假光/alpha 过渡/边缘/亮度透明全关、发光归特效缓冲那条 pass、软粒子
// 按范围裁决不做——每一环的去向写在链条注释里。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::view_transformations::{position_world_to_view, position_view_to_clip}

// ---- 材质 uniform（group 3）：装载时从材质记录读入，静态 ----

// uSheet：片表动画的格数与偏移。本材质 _BaseMap 是单张 2D 图
// （_BASE_MAP_MODE_2D），无片表 → (1, 1, 0, 0)。
@group(3) @binding(0) var<uniform> sheet: vec4<f32>;
// uBaseST：_BaseMap 的缩放平移（textureScaleOffset._BaseMap）。
@group(3) @binding(1) var<uniform> base_st: vec4<f32>;
@group(3) @binding(2) var base_map: texture_2d<f32>;
@group(3) @binding(3) var base_sampler: sampler;

struct RainVertexOutput {
    @builtin(position) position: vec4<f32>,
    // 角点坐标（0..1），片元侧从这里出发换算采样 uv。
    @location(0) uv: vec2<f32>,
    // 逐粒子颜色（COLOR0 顶点流），片元链最后一乘。
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> RainVertexOutput {
    var out: RainVertexOutput;
    // 视空间粒子原点：源式 modelViewMatrix * vec4(0,0,0,1)。
    let view_pos = position_world_to_view(mesh.position);
    // 角点 = uv - 0.5（pivot 在中心）；对齐项 = 角点 × 尺寸
    // （源式 position.xy * scale，X 取列 0 长、Y 取列 1 长——属性池把
    // 每粒子的出生尺寸直接放进了 UV_1，语义相同）。
    let corner = mesh.uv - vec2<f32>(0.5);
    let aligned = corner * mesh.uv_b;
    // 绕视轴旋转项（uRotation）：start.rotation 恒 0 且
    // rotationOverLifetime 未映射 → 恒等旋转，源式的那组 cos/sin
    // 乘 I·1 化简为直加。
    let view_final = vec3<f32>(view_pos.xy + aligned, view_pos.z);
    out.position = position_view_to_clip(view_final);
    out.uv = mesh.uv;
    out.color = mesh.color;
    return out;
}

@fragment
fn fragment(in: RainVertexOutput) -> @location(0) vec4<f32> {
    // uv0 = vParticleUv * uSheet.xy + uSheet.zw。
    let uv0 = in.uv * sheet.xy + sheet.zw;
    // 基础槽：绕轴心旋转关（_BaseMapRotationEnabled 0）、ST 变换、
    // 逐粒子偏移选择器全零流（两个 *Coord 都是 0）→ 偏移项为 0。
    let uv_base = uv0 * base_st.xy + base_st.zw;
    // 纵向翻转一次：演示件贴图 flipY=true（GL 纹理原点在左下），
    // 本管线的贴图原点在左上——同一个「画面底 = 图像底」语义两边差
    // 一次 y 翻转，在这里补齐。
    let uv_sample = vec2<f32>(uv_base.x, 1.0 - uv_base.y);
    let b = textureSample(base_map, base_sampler, uv_sample);

    // 染色环：HDR 闸（乘数任一通道 > 1.0001）在 Rust 装载侧判——本材质
    // _TintBlendRate 1.0 × _TintColor 高动态通道，整环被拒并计数。此处
    // tintMult 恒 1，链条无此步。
    var c = b;
    // 假光（_FakeLightEnabled 0）、alpha 过渡（无 _DISSOLVE/_FADE 关键
    // 字）都关。发光只在 MysekaiEffect 那条 pass 上，归特效缓冲，
    // 前向链不落。
    // 顶点色：源式 C.rgb *= uColor; C.a *= uAlpha——uColor/uAlpha 在源侧
    // 是逐粒子量（演示件每粒子一份 uniform），本管线由 COLOR0 顶点流
    // 携带（colorOverLifetime 的求值）。
    c = vec4<f32>(c.rgb * in.color.rgb, c.a * in.color.a);
    // 边缘透明 / 亮度透明（关键字缺席）都关。软粒子是 alpha 乘法链的最
    // 后一步，范围裁决不做（无深度 prepass 可读），挂账具名在收工报告。
    return c;
}
