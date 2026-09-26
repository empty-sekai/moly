// Mysekai/Effect/UI-Uber: the program a UIParticle's baked mesh is drawn
// with on a canvas (its pass without a LightMode, the one the UI camera
// draws).
//
// Vertex: the canvas-space position through the canvas entity's transform;
// the texture coordinate is uv * _MainTex_ST.xy + _MainTex_ST.zw; the vertex
// colour passes through.
//
// Fragment: c = texture(_MainTex, uv) * colour; _BlendMode 2 multiplies the
// rgb by the alpha, 0, 1 and any other value keep it; the alpha is c.a. The
// program has no clip rect and no alpha clip.
//
// Colour domain: the source is a Gamma-space build, so it samples without a
// decode and works on stored values. The texture here is loaded as sRGB (the
// sampler decodes it) and the colour target encodes on write, so the sample
// is encoded back to its stored value first and the result decoded once
// before the write. The vertex colour is a stored value (vertex formats have
// no sRGB variant). Alpha takes neither step. Blending then happens in linear
// light, as for the product's other canvas draws; the source blends stored
// values.
//
// Images are stored top row first and the source reads v upward, so the
// texture is read at (u, 1 - v) after the scale and offset.
#import bevy_sprite::mesh2d_functions as mesh_functions
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#import bevy_sprite::mesh2d_view_bindings::view
#endif

struct UiUber {
    // _MainTex_ST: (scale x, scale y, offset x, offset y).
    main_tex_st: vec4<f32>,
    // x: _BlendMode.
    blend_mode: vec4<u32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: UiUber;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var main_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var main_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(input: Vertex) -> Varyings {
    var output: Varyings;
    let world = mesh_functions::mesh2d_position_local_to_world(
        mesh_functions::get_world_from_local(input.instance_index), vec4(input.position, 1.0));
    output.position = mesh_functions::mesh2d_position_world_to_clip(world);
    output.uv = input.uv * material.main_tex_st.xy + material.main_tex_st.zw;
    output.color = input.color;
    return output;
}

// Sample to stored value: undoes the sampler's sRGB decode.
fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}

// Stored value to the value handed to the sRGB target (the target clamps to
// [0, 1], so clamping first is the same).
fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    let sampled = textureSample(main_tex, main_sampler, vec2<f32>(input.uv.x, 1.0 - input.uv.y));
    let c = vec4<f32>(srgb_format_encode(sampled.rgb), sampled.a) * input.color;
    var rgb = c.rgb;
    if material.blend_mode.x == 2u {
        rgb = c.a * c.rgb;
    }
    var color = vec4<f32>(srgb_format_decode(rgb), c.a);
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
