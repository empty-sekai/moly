// 主光深度 pass：光源向正交一次成图（单图路线，非级联、无图集）。
// 帧块与逐实体块的槽序是 shadowmap.rs 与本文件的契约，两边同改。
// 逐实体矩阵走动态 offset：每次 set_bind_group 换窗，着色器只看窗内一份。

struct ShadowFrame {
    // 深度 pass 的裁剪矩阵（正交，z 近 0 远 1）。
    light_view_proj: mat4x4<f32>,
    // 采样矩阵：xy 图内 uv（含 y 翻转），z 比较深度。pass 侧不采样，
    // 与帧块同体保存一份（shadowmap.rs 一次写入两块）。
    world_to_shadow: mat4x4<f32>,
    // The ShadowCaster programs' caster bias, in world units: x along the
    // light direction, y along the normal (both already negative, as the
    // pipeline computes them).
    shadow_bias: vec4<f32>,
    // xyz: the direction towards the light.
    light_direction: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: ShadowFrame;
@group(0) @binding(1) var<uniform> world_from_local: mat4x4<f32>;

fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let x = cross(m[1], m[2]);
    let y = cross(m[2], m[0]);
    let z = cross(m[0], m[1]);
    let det = dot(m[2], z);
    return mat3x3<f32>(x / det, y / det, z / det);
}

// The ShadowCaster vertex program, step by step, from the world position:
// the offset along the light direction by the depth bias; the world normal
// (normalised with the same floor on its squared length) and the offset
// along it, scaled by one minus the clamped cosine to the light; the clip
// position with z clamped to the near plane (the program's clip-space near
// value is -w, this pass's is 0).
fn biased_clip(world_position: vec3<f32>, world_normal: vec3<f32>) -> vec4<f32> {
    let to_light = frame.light_direction.xyz;
    var world = to_light * frame.shadow_bias.x + world_position;
    let n = world_normal * inverseSqrt(max(dot(world_normal, world_normal), 1.17549435e-38));
    let scale = (1.0 - clamp(dot(to_light, n), 0.0, 1.0)) * frame.shadow_bias.y;
    world = n * scale + world;
    var clip = frame.light_view_proj * vec4<f32>(world, 1.0);
    clip.z = max(clip.z, 0.0);
    return clip;
}

fn model_3x3(m: mat4x4<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
}

// Rigid casters: the world normal is the normal times the world-to-object
// matrix, the inverse transpose of the world matrix's 3x3.
@vertex
fn shadow_depth_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
) -> @builtin(position) vec4<f32> {
    let world = (world_from_local * vec4<f32>(position, 1.0)).xyz;
    return biased_clip(world, inverse_transpose_3x3(model_3x3(world_from_local)) * normal);
}

// A rigid mesh without a normal stream: the bias along the light only.
@vertex
fn shadow_depth_vertex_without_normal(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    let world = (world_from_local * vec4<f32>(position, 1.0)).xyz;
    var clip = frame.light_view_proj
        * vec4<f32>(frame.light_direction.xyz * frame.shadow_bias.x + world, 1.0);
    clip.z = max(clip.z, 0.0);
    return clip;
}

// Skinned casters (the characters). The palette window holds the draw's joint
// matrices of the current frame, each one the joint's world transform times its
// inverse bind pose, so the blended matrix maps a bind-pose vertex straight to
// world space; the mesh entity's own transform takes no part.
@group(1) @binding(0) var<uniform> joint_palette: array<mat4x4<f32>, 256>;

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
    let world = (skin * vec4<f32>(position, 1.0)).xyz;
    return biased_clip(world, inverse_transpose_3x3(model_3x3(skin)) * normal);
}
