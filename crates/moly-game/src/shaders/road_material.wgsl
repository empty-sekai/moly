// CN Mysekai/Fixture/Road Base, runtime instancing inputs supplied by material.
// Source shading uses stored/gamma color; bridge glTF's sRGB texture/target here.
#import bevy_pbr::forward_io::Vertex
#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

struct RoadParams {
    connect: vec4<f32>, // X+, X-, Y+, Y-
    alpha_tiling: vec4<f32>,
    options: vec4<f32>, // local mapping, phenomena lighting, alpha clip, texture scale
    shadow: vec4<f32>, // size, intensity, exponent, corner exponent
    edge: vec4<f32>, // intensity, size
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
    // 现象自发光类型（全局 int `_MysekaiPhenomenaEmissionType`，f32 存整
    // 数值）。Base 主程序不消费它（第二颜色目标的自发光门在自发光 pass
    // 里消费，见 fixture_emission.wgsl）；槽带着，两侧共用同一份表。
    emission_type: vec4<f32>,
}
@group(3) @binding(0) var<uniform> road: RoadParams;
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
struct RoadOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color_uv: vec2<f32>,
    @location(1) alpha_uv: vec2<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) @interpolate(flat) origin: vec3<f32>,
}

@vertex
fn vertex(mesh: Vertex) -> RoadOutput {
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(mesh.position, 1.0));
    var out: RoadOutput;
    out.position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.origin = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let uv = select(world.xz, mesh.position.xz, road.options.x > 0.5) * road.options.w;
    let alpha_uv = mesh.uv * road.alpha_tiling.xy + road.alpha_tiling.zw;
    // Unity source UVs + UnityPy's vertically flipped exported PNGs.
    out.color_uv = vec2<f32>(uv.x, 1.0 - uv.y);
    out.alpha_uv = vec2<f32>(alpha_uv.x, 1.0 - alpha_uv.y);
    return out;
}

@fragment
fn fragment(in: RoadOutput) -> @location(0) vec4<f32> {
    // Both samples precede clipping, preserving uniform implicit derivatives.
    let base = srgb_format_encode(textureSampleBias(main_tex, main_sampler, in.color_uv, env.mip_bias.x).rgb);
    let alpha = textureSampleBias(main_tex, main_sampler, in.alpha_uv, env.mip_bias.x).a;
    if road.options.z > 0.5 && alpha - 0.5 < 0.0 { discard; }
    var rgb = base;
    if road.options.y > 0.5 {
        let delta = in.world_position.xz - in.origin.xz;
        let radial = 1.0 - length(abs(delta + delta) - vec2<f32>(0.5));
        let corner_edge = select(0.0, 1.0, 1.0 - road.edge.y * 0.5 >= radial);
        let corner = exp2(log2(radial) * (road.shadow.x * road.shadow.w));
        var sides = abs(delta * 8.0);
        sides += select(vec2<f32>(0.0), vec2<f32>(-1.0),
            vec2<bool>(delta.x <= 0.0 && road.connect.y > 0.5, delta.y <= 0.0 && road.connect.w > 0.5));
        sides += select(vec2<f32>(0.0), vec2<f32>(-1.0),
            vec2<bool>(delta.x >= 0.0 && road.connect.x > 0.5, delta.y >= 0.0 && road.connect.z > 0.5));
        sides = clamp(sides, vec2<f32>(0.0), vec2<f32>(1.0));
        let edge_flags = select(vec2<f32>(0.0), vec2<f32>(1.0), vec2<f32>(1.0 - road.edge.y) >= sides);
        let powers = exp2(log2(sides) * (road.shadow.x * road.shadow.z));
        let shadow = max(corner, powers.y + powers.x);
        var light = clamp(1.0 - shadow * road.shadow.y, 0.0, 1.0);
        let edge_factor = corner_edge * min(edge_flags.y, edge_flags.x);
        light = road.edge.x * (light * edge_factor - light) + light;
        let tint = env.drop_shadow_color.w * (base * env.drop_shadow_color.rgb - base) + base;
        rgb = light * (base - tint) + tint;
        rgb = env.phenomena_light_color.w * (rgb * env.phenomena_light_color.rgb - rgb) + rgb;
    }
    // Source Road has no fog, treasure shadow or emission. Target0 alpha is 1.
    return vec4<f32>(srgb_format_decode(rgb), 1.0);
}
