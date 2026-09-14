// 表情族的着色：sprite 平面与粒子 billboard 共用一条链。
//
// 与雨族同款的分工：四边形在 Rust 侧逐帧**预装成世界系顶点**
// （粒子 billboard 的视面对齐、自旋与 sprite 节点树的旋转缩放都在
// 顶点装配那一侧算完），着色器只做世界 → 视 → 裁剪与采样。
// 实体的变换恒等，POSITION 属性即世界坐标。
//
// 逐顶点属性语义：
//   POSITION = 四边形顶点的世界坐标（已含尺寸、旋转、pivot、子矩形无关）
//   UV_0     = 采样 uv 的折算起点（sprite 的子矩形、片表动画的格位、
//              基础图旋转都已折进这一流——链上仅剩材质级的 ST 变换）
//   COLOR    = 逐顶点色（粒子的 colorOverLifetime 求值 / sprite 节点色）
//
// 片元链是源材质（表情族两套：表情 sprite 自带材质与特效 UberUnlit）
// 的闸后子集：基础图采样 → 纵向翻转 → 顶点色一乘。染色/假光/alpha
// 过渡/边缘/亮度透明在源侧全关（装载侧逐值断言），软粒子按范围裁决
// 不做（与雨族同一裁决，装载时计数），发光归特效缓冲那条 pass。
//
// `base_st` 是材质 textureScaleOffset._BaseMap（语料恒等，读自记录）。

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::view_transformations::{position_world_to_view, position_view_to_clip}

// ---- 材质 uniform（group 3）：装载时从材质记录读入，静态 ----

@group(3) @binding(0) var<uniform> base_st: vec4<f32>;
@group(3) @binding(1) var base_map: texture_2d<f32>;
@group(3) @binding(2) var base_sampler: sampler;

struct EmoteVertexOutput {
    @builtin(position) position: vec4<f32>,
    // 采样 uv 的折算起点（四角坐标或已折算的格内 uv）。
    @location(0) uv: vec2<f32>,
    // 逐顶点色（COLOR0 顶点流），片元链最后一乘。
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> EmoteVertexOutput {
    var out: EmoteVertexOutput;
    // 顶点已是世界坐标（实体变换恒等，属性即世界量）。
    let view_pos = position_world_to_view(mesh.position);
    out.position = position_view_to_clip(view_pos);
    out.uv = mesh.uv;
    out.color = mesh.color;
    return out;
}

@fragment
fn fragment(in: EmoteVertexOutput) -> @location(0) vec4<f32> {
    // 基础槽：片表格位与子矩形已折进 UV_0，这里只剩材质级的 ST 变换
    // （uv0 · ST.xy + ST.zw；源链的 uSheet 一步对两族都恒等）。
    let uv_base = in.uv * base_st.xy + base_st.zw;
    // 纵向翻转一次：贴图原点差异（同雨族的口径——本管线贴图原点在
    // 左上，源侧的 GL 约定原点在左下，差一次 y 翻转，在这里补齐）。
    let uv_sample = vec2<f32>(uv_base.x, 1.0 - uv_base.y);
    let b = textureSample(base_map, base_sampler, uv_sample);
    // 顶点色：源式 C.rgb *= uColor; C.a *= uAlpha——逐粒子量由 COLOR0
    // 顶点流携带（colorOverLifetime / sprite 节点色的求值）。
    var c = b;
    c = vec4<f32>(c.rgb * in.color.rgb, c.a * in.color.a);
    // 染色环（源侧 HDR 闸全关）、假光、alpha 过渡、边缘透明、亮度透明
    // 都关；软粒子是 alpha 乘法链的最后一步，范围裁决不做（无深度
    // prepass 可读），装载时计数挂账。
    return c;
}
