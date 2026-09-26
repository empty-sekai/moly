// 主光深度 pass：光源向正交一次成图（单图路线，非级联、无图集）。
// 帧块与逐实体块的槽序是 shadowmap.rs 与本文件的契约，两边同改。
// 逐实体矩阵走动态 offset：每次 set_bind_group 换窗，着色器只看窗内一份。

struct ShadowFrame {
    // 深度 pass 的裁剪矩阵（正交，z 近 0 远 1）。
    light_view_proj: mat4x4<f32>,
    // 采样矩阵：xy 图内 uv（含 y 翻转），z 比较深度。pass 侧不采样，
    // 与帧块同体保存一份（shadowmap.rs 一次写入两块）。
    world_to_shadow: mat4x4<f32>,
    // The caster bias of the character ShadowCaster program, in world units:
    // x along the light direction, y along the normal (both already negative,
    // as the pipeline computes them). Read only by the skinned entry point.
    shadow_bias: vec4<f32>,
    // xyz: the direction towards the light. Read only by the skinned entry point.
    light_direction: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: ShadowFrame;
@group(0) @binding(1) var<uniform> world_from_local: mat4x4<f32>;

@vertex
fn shadow_depth_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return frame.light_view_proj * (world_from_local * vec4<f32>(position, 1.0));
}

// Skinned casters (the characters). The palette window holds the draw's joint
// matrices of the current frame, each one the joint's world transform times its
// inverse bind pose, so the blended matrix maps a bind-pose vertex straight to
// world space; the mesh entity's own transform takes no part.
@group(1) @binding(0) var<uniform> joint_palette: array<mat4x4<f32>, 256>;

fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let x = cross(m[1], m[2]);
    let y = cross(m[2], m[0]);
    let z = cross(m[0], m[1]);
    let det = dot(m[2], z);
    return mat3x3<f32>(x / det, y / det, z / det);
}

@vertex
fn shadow_depth_skinned_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) joints: vec4<u32>,
    @location(3) weights: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let skin = weights.x * joint_palette[joints.x]
        + weights.y * joint_palette[joints.y]
        + weights.z * joint_palette[joints.z]
        + weights.w * joint_palette[joints.w];
    // The character ShadowCaster vertex program, step by step: the world
    // position, then the offset along the light direction by the depth bias.
    var world = (skin * vec4<f32>(position, 1.0)).xyz;
    let to_light = frame.light_direction.xyz;
    world = to_light * frame.shadow_bias.x + world;
    // The world normal (normalised with the same floor on its squared
    // length), then the offset along it, scaled by one minus the clamped
    // cosine to the light.
    var n = inverse_transpose_3x3(mat3x3<f32>(skin[0].xyz, skin[1].xyz, skin[2].xyz)) * normal;
    n = n * inverseSqrt(max(dot(n, n), 1.17549435e-38));
    let scale = (1.0 - clamp(dot(to_light, n), 0.0, 1.0)) * frame.shadow_bias.y;
    world = n * scale + world;
    var clip = frame.light_view_proj * vec4<f32>(world, 1.0);
    // The program clamps clip z to the near plane (its clip-space near value
    // is -w); this pass's clip-space near value is 0.
    clip.z = max(clip.z, 0.0);
    return clip;
}
