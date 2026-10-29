// The OC body's material. The slot order of `OcParams` is a contract with
// `player_avatar/oc/material.rs`, and `AvatarEnv` is the avatar material's
// frame globals (`avatar_material.wesl`), bound from the same buffer.
//
// Colour: the model's base colour texture times its base colour factor (and
// the vertex colour for a glTF PBR material, whose definition multiplies it;
// the VRM toon materials do not read it), in the storage (gamma) domain the
// avatar program computes in: the sRGB texture is decoded to linear by the
// sampler and encoded back, the factor arrives encoded. Then the avatar's
// light mix `lit = light.w * (albedo * light.rgb - albedo) + albedo` and its
// bayer dither discard on `_DitherAlpha`. The output is decoded once for the
// sRGB target, whose hardware encode restores the storage value.

import bevy_pbr::render::forward_io::Vertex;
import bevy_pbr::render::mesh_functions;
import bevy_pbr::render::skinning;
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct OcParams {
    // rgb: base colour factor (storage domain) · a: base alpha factor.
    base_color: vec4<f32>,
    // x: alpha cutoff (mask mode) · y: `_DitherAlpha` · z: 1 when the vertex
    // colour multiplies the base colour, else 0 · w: unused.
    cutoff_dither: vec4<f32>,
}

struct AvatarEnv {
    light_color: vec4<f32>,
    screen_params: vec4<f32>,
    mip_bias: vec4<f32>,
    projection_params: vec4<f32>,
}

@group(3) @binding(0) var<uniform> params: OcParams;
@group(3) @binding(1) var<uniform> env: AvatarEnv;
@group(3) @binding(2) var base_tex: texture_2d<f32>;
@group(3) @binding(3) var base_sampler: sampler;

struct OcVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // The avatar program's screen position: xy = 0.5 * (w + component),
    // z and w as clip.
    @location(1) screen_pos: vec4<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> OcVertexOutput {
    @if(SKINNED) let world_from_local = skinning::skin_model(
        mesh.joint_indices,
        mesh.joint_weights,
        mesh.instance_index,
    );
    @if(!SKINNED) let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(mesh.position, 1.0),
    );
    let clip = position_world_to_clip(world_position.xyz);

    var out: OcVertexOutput;
    out.position = clip;
    @if(VERTEX_UVS_A) out.uv = mesh.uv;
    @if(!VERTEX_UVS_A) out.uv = vec2<f32>(0.0);
    @if(VERTEX_COLORS) out.color = mesh.color;
    @if(!VERTEX_COLORS) out.color = vec4<f32>(1.0);
    let y_flipped = clip.y * env.projection_params.x;
    out.screen_pos = vec4<f32>(
        0.5 * clip.w + 0.5 * clip.x,
        0.5 * clip.w + 0.5 * y_flipped,
        clip.z,
        clip.w,
    );
    return out;
}

// The value handed to an sRGB colour target: decoded once by the format's
// EOTF so that the hardware encode on write restores it.
fn srgb_target_value(encoded: vec3<f32>) -> vec3<f32> {
    let e = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

// A linear value encoded to the storage domain (the sRGB OETF).
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let e = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = e * 12.92;
    let hi = 1.055 * pow(e, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, e <= vec3<f32>(0.0031308));
}

// The avatar program's 4x4 dither threshold for this pixel.
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

@fragment
fn fragment(in: OcVertexOutput) -> @location(0) vec4<f32> {
    let bias = env.mip_bias.x;
    let sampled = textureSampleBias(base_tex, base_sampler, in.uv, bias);
    let vertex = mix(vec4<f32>(1.0), in.color, params.cutoff_dither.z);
    let albedo = srgb_format_encode(sampled.rgb) * srgb_format_encode(vertex.rgb)
        * params.base_color.rgb;
    let alpha = sampled.a * vertex.a * params.base_color.a;

    @if(OC_ALPHA_MASK) if alpha < params.cutoff_dither.x {
        discard;
    }

    if params.cutoff_dither.y - bayer_value(in.screen_pos) < 0.0 {
        discard;
    }

    let lc = env.light_color;
    let rgb = lc.w * (albedo * lc.rgb - albedo) + albedo;

    @if(OC_ALPHA_BLEND) return vec4<f32>(srgb_target_value(rgb), alpha);
    @if(!OC_ALPHA_BLEND) return vec4<f32>(srgb_target_value(rgb), 1.0);
}
