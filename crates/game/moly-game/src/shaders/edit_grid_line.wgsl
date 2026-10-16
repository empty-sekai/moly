// Mysekai/Grid/Line, the layout grid's solid and dotted lines (one program,
// no keyword).
//
// Vertex: uv = uv0 * _MainTex_ST.xy + _MainTex_ST.zw; the position goes
// through the object and view-projection matrices.
// Fragment: alpha = (_LineFill >= fract(uv.x * _LineRepeat) ? 1 : 0) * _Color1.a,
// rgb = _Color1.rgb. uv.x is the position along the line, so the solid lines
// (_LineFill 1) are whole and the dotted ones repeat _LineRepeat times a
// metre with the _LineFill share lit.
//
// Colour domain: the source is a gamma-space player and writes the colour as
// stored; the target here is sRGB, so the output is decoded once and the
// attachment's encode restores the stored value.

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

struct GridLine {
    main_tex_st: vec4<f32>,
    color1: vec4<f32>,
    // (_LineRepeat, _LineFill, 0, 0)
    line: vec4<f32>,
}
@group(3) @binding(0) var<uniform> grid: GridLine;

struct LineOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> LineOutput {
    var out: LineOutput;
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(mesh.position, 1.0));
    out.position = position_world_to_clip(world.xyz);
#ifdef VERTEX_UVS_A
    out.uv = mesh.uv * grid.main_tex_st.xy + grid.main_tex_st.zw;
#else
    out.uv = grid.main_tex_st.zw;
#endif
    return out;
}

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: LineOutput) -> @location(0) vec4<f32> {
    let lit = select(0.0, 1.0, grid.line.y >= fract(in.uv.x * grid.line.x));
    return vec4<f32>(srgb_format_decode(grid.color1.xyz), lit * grid.color1.w);
}
