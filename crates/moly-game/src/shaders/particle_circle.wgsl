// Hidden/particle_circle, Universal Forward (the non-instanced program): the
// stay glow the rare stones play. Particle vertices arrive in world space
// (the draw has an identity transform); the colour stream carries the source
// storage value, so the output is format-decoded for the sRGB target.
// Vertex slots are assembled by `ParticleCircleMaterial::specialize`.
#import bevy_pbr::view_transformations::position_world_to_clip

struct CircleParams {
    // x = _UsePhenomenaLighting (material float).
    use_phenomena_lighting: vec4<f32>,
}
// Only the first two slots are read; the bound buffer is the whole site-wide
// block shared with the site, fixture and particle materials.
struct CircleEnv {
    light_vector: vec4<f32>,
    phenomena_light_color: vec4<f32>,
}
@group(3) @binding(0) var<uniform> params: CircleParams;
@group(3) @binding(1) var<uniform> env: CircleEnv;

struct CircleVertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) colour: vec4<f32>,
}
struct CircleOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
}

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@vertex
fn vertex(v: CircleVertex) -> CircleOutput {
    var out: CircleOutput;
    out.position = position_world_to_clip(v.position);
    out.uv = v.uv;
    out.colour = v.colour;
    return out;
}

@fragment
fn fragment(in: CircleOutput) -> @location(0) vec4<f32> {
    let d = in.uv - vec2<f32>(0.5, 0.5);
    let a = max(-sqrt(dot(d, d)) * 2.0 + 1.0, 0.0);
    // Source: exp2(log2(a) * 1.25). At a = 0 its log2 is -inf and exp2 gives
    // 0; WGSL leaves log2(0) undefined, so that zero is written out.
    let falloff = select(0.0, exp2(log2(a) * 1.25), a > 0.0);
    let alpha = falloff * in.colour.w;
    let c = in.colour.rgb;
    let light = env.phenomena_light_color;
    let lit = light.www * (c * light.xyz - c) + c;
    let rgb = select(c, lit, 0.5 < params.use_phenomena_lighting.x);
    return vec4<f32>(srgb_format_decode(rgb), alpha);
}
