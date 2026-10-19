// Mysekai/Avatar-Tool, pass Base (LightMode MysekaiObject): the tool in the
// player's hand. Vertex: object to world to clip, uv0 as is. Fragment: the
// main texture (global mip bias); with _UsePhenomenaLighting above 0.5 the
// colour moves toward colour times the phenomena light colour by its w;
// alpha is the texture's. The second colour target (a constant zero) is not
// drawn, as for every site family. Source shading uses stored/gamma colour;
// the glTF sRGB texture and target are bridged here.
#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

struct ToolParams {
    // x = _UsePhenomenaLighting.
    options: vec4<f32>,
}
struct SiteEnv {
    light_vector: vec4<f32>,
    phenomena_light_color: vec4<f32>,
    phenomena_shade_color: vec4<f32>,
    drop_shadow_color: vec4<f32>,
    edge_threshold: vec4<f32>,
    edge_smoothness: vec4<f32>,
    treasure_position_0: vec4<f32>,
    treasure_position_1: vec4<f32>,
    treasure_shadow_intensity: vec4<f32>,
    camera_position: vec4<f32>,
    ortho_params: vec4<f32>,
    view_matrix_c0: vec4<f32>,
    view_matrix_c1: vec4<f32>,
    view_matrix_c2: vec4<f32>,
    view_matrix_c3: vec4<f32>,
    screen_params: vec4<f32>,
    fog_params: vec4<f32>,
    fog_near_color: vec4<f32>,
    fog_far_color: vec4<f32>,
    projection_params: vec4<f32>,
    mip_bias: vec4<f32>,
    time: vec4<f32>,
    emission_type: vec4<f32>,
}
@group(3) @binding(0) var<uniform> tool: ToolParams;
@group(3) @binding(1) var<uniform> env: SiteEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;

fn srgb_format_encode(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}
fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

struct ToolOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> ToolOutput {
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(mesh.position, 1.0));
    var out: ToolOutput;
    out.position = position_world_to_clip(world.xyz);
    out.uv = mesh.uv;
    return out;
}

@fragment
fn fragment(in: ToolOutput) -> @location(0) vec4<f32> {
    let sampled = textureSampleBias(main_tex, main_sampler, in.uv, env.mip_bias.x);
    let tex = srgb_format_encode(sampled.rgb);
    let lit = env.phenomena_light_color.w * (tex * env.phenomena_light_color.rgb - tex) + tex;
    let rgb = select(tex, lit, 0.5 < tool.options.x);
    return vec4<f32>(srgb_format_decode(rgb), sampled.a);
}
