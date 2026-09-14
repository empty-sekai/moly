// 主光深度 pass：光源向正交一次成图（单图路线，非级联、无图集）。
// 帧块与逐实体块的槽序是 shadowmap.rs 与本文件的契约，两边同改。
// 逐实体矩阵走动态 offset：每次 set_bind_group 换窗，着色器只看窗内一份。

struct ShadowFrame {
    // 深度 pass 的裁剪矩阵（正交，z 近 0 远 1）。
    light_view_proj: mat4x4<f32>,
    // 采样矩阵：xy 图内 uv（含 y 翻转），z 比较深度。pass 侧不采样，
    // 与帧块同体保存一份（shadowmap.rs 一次写入两块）。
    world_to_shadow: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> frame: ShadowFrame;
@group(0) @binding(1) var<uniform> world_from_local: mat4x4<f32>;

@vertex
fn shadow_depth_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return frame.light_view_proj * (world_from_local * vec4<f32>(position, 1.0));
}
