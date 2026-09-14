// Canvas Base and TransparentBlock, sharing the live scene environment.
#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::skinning
#import bevy_pbr::view_transformations::position_world_to_clip

struct SurfaceParams {
    color1: vec4<f32>, color2: vec4<f32>, outline: vec4<f32>, controls: vec4<f32>,
}
struct SiteEnv {
    light_vector: vec4<f32>, phenomena_light_color: vec4<f32>, phenomena_shade_color: vec4<f32>,
    drop_shadow_color: vec4<f32>, edge_threshold: vec4<f32>, edge_smoothness: vec4<f32>,
    treasure_position_0: vec4<f32>, treasure_position_1: vec4<f32>, treasure_shadow_intensity: vec4<f32>,
    camera_position: vec4<f32>, ortho_params: vec4<f32>,
    view_matrix_c0: vec4<f32>, view_matrix_c1: vec4<f32>, view_matrix_c2: vec4<f32>, view_matrix_c3: vec4<f32>,
    screen_params: vec4<f32>, fog_params: vec4<f32>, fog_near_color: vec4<f32>, fog_far_color: vec4<f32>,
    projection_params: vec4<f32>, mip_bias: vec4<f32>, time: vec4<f32>,
}
@group(3) @binding(0) var<uniform> surface: SurfaceParams;
@group(3) @binding(1) var<uniform> env: SiteEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;
@group(3) @binding(4) var overlay_tex: texture_2d<f32>;
@group(3) @binding(5) var overlay_sampler: sampler;

fn storage_color(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), x * 12.92, x <= vec3<f32>(0.0031308));
}
fn target_color(stored: vec3<f32>) -> vec3<f32> {
    let x = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}
struct SurfaceOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vertex(mesh: Vertex) -> SurfaceOutput {
#ifdef SKINNED
    let matrix = skinning::skin_model(mesh.joint_indices, mesh.joint_weights, mesh.instance_index);
#else
    let matrix = mesh_functions::get_world_from_local(mesh.instance_index);
#endif
    let world = mesh_functions::mesh_position_local_to_world(matrix, vec4<f32>(mesh.position, 1.0));
    var out: SurfaceOutput;
    out.position = position_world_to_clip(world.xyz);
    out.uv = mesh.uv;
    return out;
}

@fragment
fn fragment(in: SurfaceOutput) -> @location(0) vec4<f32> {
#ifdef SURFACE_BLOCK
    let edge_distance = vec2<f32>(0.5) - abs(in.uv - vec2<f32>(0.5));
    let edge = any(vec2<f32>(surface.controls.y * 0.00100000005) >= edge_distance);
    let cells = floor(in.uv * surface.controls.x);
    let parity = fract((fract(abs(cells.x) * 0.5) * 2.0 + abs(cells.y)) * 0.5) * 2.0;
    var color = (surface.color2 - surface.color1) * parity + surface.color1;
    color.a *= surface.controls.w;
    color = select(color, surface.outline, edge);
    return vec4<f32>(target_color(color.rgb), color.a);
#else
    // The source Base program samples UV0 directly (no ST or alpha clip).
    // Exported fixture PNG rows are inverted relative to Unity's UV frame.
    let uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let main = textureSampleBias(main_tex, main_sampler, uv, env.mip_bias.x);
    let base = storage_color(main.rgb);
    let overlay = storage_color(textureSampleBias(overlay_tex, overlay_sampler, uv, env.mip_bias.x).rgb);
#ifdef SURFACE_DITHER
    let pixel = vec2<f32>(in.position.x, env.screen_params.y - in.position.y);
    let cell = vec2<u32>(fract(pixel * 0.25) * 4.0);
    var bayer = array<f32, 16>(0.0,12.0,3.0,15.0, 8.0,4.0,11.0,7.0, 2.0,14.0,1.0,13.0, 10.0,6.0,9.0,5.0);
    if surface.controls.w - (bayer[cell.y * 4u + cell.x] * 0.0618750006 + 0.00999999978) < 0.0 { discard; }
#endif
    let low = (vec3<f32>(1.0) - 2.0 * overlay) * base * base + 2.0 * base * overlay;
    let high = sqrt(base) * (2.0 * overlay - vec3<f32>(1.0)) + 2.0 * base * (vec3<f32>(1.0) - overlay);
    let softened = select(low, high, overlay >= vec3<f32>(0.5));
    var rgb = (softened - base) * 0.25 + base;
    if surface.controls.z > 0.5 {
        rgb = env.phenomena_light_color.w * (rgb * env.phenomena_light_color.rgb - rgb) + rgb;
    }
    return vec4<f32>(target_color(rgb), main.a);
#endif
}
