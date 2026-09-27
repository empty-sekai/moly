// Mysekai/Indoor/BG, the room plain background (one program, no keyword).
//
// Vertex: the quad's positions are written straight into clip xy with w = 1;
// clip z is the depth of the eye distance `_PositionZ` (the source writes the
// value its depth buffer holds at that distance, from _ZBufferParams). Here
// the same eye distance goes through this view's projection, so the quad
// sits at that distance in this renderer's depth convention. That is the
// reversed-Z (Vulkan) reading of the source value; under OpenGL ES the same
// value lands at another depth. The source's graphics API is not decided, so
// this choice is open (see plain_background.rs).
// Fragment: rgb = lerp(_Color2, _Color1, uv.y), alpha 1.
//
// Colour domain: the source is a Gamma-space player and writes the colour as
// stored. The two colours are the material's raw values (stored domain); the
// target of this pass is sRGB, so the output is decoded once and the
// attachment's encode restores the stored value.

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_view_bindings::view

@group(3) @binding(0) var<uniform> color1: vec4<f32>;
@group(3) @binding(1) var<uniform> color2: vec4<f32>;
// (_PositionZ, padding, padding, padding)
@group(3) @binding(2) var<uniform> params: vec4<f32>;

struct PlainOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> PlainOutput {
    var out: PlainOutput;
    let eye = view.clip_from_view * vec4<f32>(0.0, 0.0, -params.x, 1.0);
    out.position = vec4<f32>(mesh.position.xy, eye.z / eye.w, 1.0);
#ifdef VERTEX_UVS_A
    out.uv = mesh.uv;
#else
    out.uv = vec2<f32>(0.0);
#endif
    return out;
}

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: PlainOutput) -> @location(0) vec4<f32> {
    let rgb = in.uv.y * (color1.xyz - color2.xyz) + color2.xyz;
    return vec4<f32>(srgb_format_decode(rgb), 1.0);
}
