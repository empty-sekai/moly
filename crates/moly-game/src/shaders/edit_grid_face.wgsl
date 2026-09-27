// Mysekai/Grid/Face-Uber, the layout grid's tile face (one program, no
// keyword).
//
// Vertex: the position goes through the object and view-projection
// matrices; the varying carries uv0 (tile units) in xy and uv0 * _TileSize
// (metres) in zw.
// Fragment: the state texel at the tile (texel fetch at the truncated uv) is
// decoded from its red, green and blue halves: state = 4r + 2g + b, with 7
// read as 6. State 0 is transparent, 1 _HighlightColor, 2
// _FixtureExistColor; 3 to 6 (hided fixture, motion area, motion disable,
// motion conflict) are stripes: f = (_FillRate >= fract(dot(uv * _TileSize,
// (0.7071, -0.7071)) * _PatternRepeat)) ? 1 : 0, colour = C1 + f * (C2 - C1)
// with that state's two colours. The result is clamped to 0..1.
//
// Named difference: a fetch at the quad's far edge (uv equal to the tile
// count) is clamped to the last texel; the source's fetch there is out of
// range.
//
// Colour domain: as edit_grid_line.wgsl (stored values, decoded once for the
// sRGB target).

#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

struct GridFace {
    // (_TileSize, _PatternRepeat, _FillRate, 0)
    params: vec4<f32>,
    highlight: vec4<f32>,
    fixture_exist: vec4<f32>,
    hided: vec4<f32>,
    hided2: vec4<f32>,
    motion_area: vec4<f32>,
    motion_area2: vec4<f32>,
    motion_disable: vec4<f32>,
    motion_disable2: vec4<f32>,
    conflict: vec4<f32>,
    conflict2: vec4<f32>,
}
@group(3) @binding(0) var<uniform> face: GridFace;
@group(3) @binding(1) var state_tex: texture_2d<f32>;

struct FaceOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec4<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> FaceOutput {
    var out: FaceOutput;
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(mesh.position, 1.0));
    out.position = position_world_to_clip(world.xyz);
#ifdef VERTEX_UVS_A
    out.uv = vec4<f32>(mesh.uv, mesh.uv * face.params.x);
#else
    out.uv = vec4<f32>(0.0);
#endif
    return out;
}

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

fn stripe(pattern: vec2<f32>, c1: vec4<f32>, c2: vec4<f32>) -> vec4<f32> {
    let along = dot(pattern, vec2<f32>(0.707106769, -0.707106769));
    let f = select(0.0, 1.0, face.params.z >= fract(along * face.params.y));
    return vec4<f32>(f) * (c2 - c1) + c1;
}

@fragment
fn fragment(in: FaceOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(state_tex));
    let texel = clamp(vec2<i32>(in.uv.xy), vec2<i32>(0), size - vec2<i32>(1));
    let t = textureLoad(state_tex, texel, 0).xyz;
    let low = t < vec3<f32>(0.5);
    let pair = select(vec3<i32>(1, 3, 5), vec3<i32>(0, 2, 4), low.z);
    let low_g = select(pair.y, pair.x, low.y);
    let high_g = select(6, pair.z, low.y);
    let state = select(high_g, low_g, low.x);
    var color = vec4<f32>(0.0);
    switch state {
        case 1: {
            color = face.highlight;
        }
        case 2: {
            color = face.fixture_exist;
        }
        case 3: {
            color = stripe(in.uv.zw, face.hided, face.hided2);
        }
        case 4: {
            color = stripe(in.uv.zw, face.motion_area, face.motion_area2);
        }
        case 5: {
            color = stripe(in.uv.zw, face.motion_disable, face.motion_disable2);
        }
        case 6: {
            color = stripe(in.uv.zw, face.conflict, face.conflict2);
        }
        default: {
            color = vec4<f32>(0.0);
        }
    }
    color = clamp(color, vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(srgb_format_decode(color.xyz), color.w);
}
