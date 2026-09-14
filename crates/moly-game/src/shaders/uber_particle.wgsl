// Both passes evaluate in the source storage colour space. The forward target
// needs format decoding; the floating-point effect target stores that value directly.
#ifndef UBER_EFFECT_PASS
#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::view_transformations::{position_world_to_view, position_view_to_clip}
#endif
struct UberT1Params {
    base_st: vec4<f32>,
    tint_colour: vec4<f32>,
    scalars: vec4<f32>,
}
#ifdef UBER_EFFECT_PASS
struct EmissionObject {
    clip_from_local: mat4x4<f32>,
    params: UberT1Params,
    emission_colour: vec4<f32>,
    emission: vec4<f32>,
    padding: array<vec4<f32>, 8>,
}
@group(0) @binding(0) var<uniform> object: EmissionObject;
@group(1) @binding(2) var base_map: texture_2d<f32>;
@group(1) @binding(3) var base_sampler: sampler;
struct EffectVertex {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
}
#else
@group(3) @binding(0) var<uniform> params: UberT1Params;
@group(3) @binding(1) var base_map: texture_2d<f32>;
@group(3) @binding(2) var base_sampler: sampler;
#endif
struct UberVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
}
#ifdef UBER_EFFECT_PASS
@vertex
fn emission_vertex(mesh: EffectVertex) -> UberVertexOutput {
    var out: UberVertexOutput;
    out.position = object.clip_from_local * vec4<f32>(mesh.position, 1.0);
    out.uv = mesh.uv;
    out.colour = mesh.colour;
    return out;
}
#else
@vertex
fn vertex(mesh: Vertex) -> UberVertexOutput {
    var out: UberVertexOutput;
    out.position = position_view_to_clip(position_world_to_view(mesh.position));
    out.uv = mesh.uv;
    out.colour = mesh.color;
    return out;
}
#endif
fn srgb_format_encode(v: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
}
fn srgb_format_decode(v: vec3<f32>) -> vec3<f32> {
    return select(pow(max((v + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
}
fn source_colour(in: UberVertexOutput, p: UberT1Params) -> vec4<f32> {
    let uv = in.uv * p.base_st.xy + p.base_st.zw;
    let sample = textureSampleBias(base_map, base_sampler, vec2<f32>(uv.x, 1.0 - uv.y), p.scalars.y);
    let base = vec4<f32>(srgb_format_encode(sample.rgb), sample.a);
#ifdef UBER_TINT_AREA_ALL
    let tinted = base * ((p.tint_colour - vec4<f32>(1.0)) * clamp(p.scalars.x, 0.0, 1.0) + vec4<f32>(1.0));
#else
    let tinted = base;
#endif
#ifdef UBER_EFFECT_PASS
#ifdef UBER_EMISSION_AREA_ALL
    var emission_colour = vec3<f32>(0.0);
    if object.emission.y == 0.0 { emission_colour = object.emission_colour.rgb; }
    if object.emission.y == 1.0 { emission_colour = tinted.rgb; }
    return vec4<f32>(tinted.rgb * emission_colour * object.emission.x, tinted.a) * in.colour;
#else
    return tinted * in.colour;
#endif
#else
    return tinted * in.colour;
#endif
}
#ifdef UBER_EFFECT_PASS
@fragment
fn emission_fragment(in: UberVertexOutput) -> @location(0) vec4<f32> {
    return source_colour(in, object.params);
}
#else
@fragment
fn fragment(in: UberVertexOutput) -> @location(0) vec4<f32> {
    let colour = source_colour(in, params);
    return vec4<f32>(srgb_format_decode(colour.rgb), colour.a);
}
#endif
