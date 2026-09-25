// Room shell programs: the module walls, floor and entrance of a room floor.
//
// Two source shaders, one pipeline arm each (exactly one family define):
//   ROOM_OBJECT  Mysekai/Object Base, the runtime `_ObjectShaderUsage` switch
//                kept as a uniform. Room shell usages: 2 (main walls) and 10
//                (entrance wall and floor, the default arm); 14 is the skip
//                arm. Variant keywords of the room materials: _DISABLE_DITHER
//                on all, _RECEIVE_SHADOWS_OFF on the main walls only.
//   ROOM_FLOOR   Mysekai/Room/Floor Base (keywords: _MAIN_LIGHT_SHADOWS only;
//                it has no fog variant, so the floor is never fogged).
// ROOM_RECEIVE_SHADOWS selects the _MAIN_LIGHT_SHADOWS program of a material
// that does not carry _RECEIVE_SHADOWS_OFF (the main light casts soft shadows
// in every environment, so the keyword is on). Without it the Object program
// is the shadowless one (for Object, the _MAIN_LIGHT_SHADOWS +
// _RECEIVE_SHADOWS_OFF program is the same text as the shadowless one).
//
// Fog: the fog pass turns _USE_MYSEKAI_FOG on globally while the site's fog
// volume is present and enabled and neither IsDisable nor IsDisableFog is set,
// and off otherwise, writing both fog colour alphas as zero when off (of the
// room floors only 002 enables its volume). The Object fog program differs
// from the fogless one only by the fog step at the end of the
// phenomena-lighting gate; with zero alphas that step returns its input
// clamped to [0, 1]. Every room shell material has the height fade off and a
// zero additive colour (room_shell.rs refuses any other), so nothing after the
// step can move a clamped value and the output decode clamps anyway: one
// program covers both keyword states.
//
// Emission: SV_Target1 is the emission-mask sample times a gate that is 0 for
// every room shell material (_Enable_Emission, manual, debug, bright and dark
// phenomena emission all 0), so the second target is not drawn.
//
// Colour domain: as in site_material.wgsl. The source is a Gamma player;
// sampled texels are encoded back to the stored domain, uniforms and vertex
// colours are stored values, and the output is decoded once before the sRGB
// attachment re-encodes it.

#import bevy_pbr::mesh_functions
#import bevy_pbr::view_transformations::position_world_to_clip

struct RoomParams {
    // (usage, useVertexColorBlend, useVertexAlphaOpacity, cull)
    object0: vec4<f32>,
    // (uv index of the main texture, mainTextureLocalMapping, uvScrollX, uvScrollY)
    object1: vec4<f32>,
    // (useFresnel, fresnelPower, usePhenomenaLighting, baseOpacity)
    lighting: vec4<f32>,
    fresnel_color: vec4<f32>,
    back_face_color: vec4<f32>,
    // (overrideShadingParameter, localShadingIntensity, localEdgeThreshold, localEdgeSmoothness)
    shading: vec4<f32>,
    // (wallAOIntensity, wallAOScaleX, wallAOScaleY, wallAOExponent)
    wall_ao: vec4<f32>,
    additive_color: vec4<f32>,
    // (useHeightFade, heightFadePosition, heightFadeLength, heightFadeExponent)
    height_fade: vec4<f32>,
}

// The shared site globals; slot order is the contract with env.rs gpu_bytes
// (the same struct as in site_material.wgsl).
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
    shadow_mask_edges: vec4<f32>,
    dropitem_globals: vec4<f32>,
    sky_bottom_color: vec4<f32>,
}

// Main-light shadow consumer block; slot order is the contract with
// shadowmap.rs (the same struct as in site_material.wgsl).
struct SiteShadow {
    world_to_shadow: mat4x4<f32>,
    // (width, height, 1/width, 1/height)
    size: vec4<f32>,
    // _MainLightShadowParams: .x strength, .z/.w distance fade.
    params: vec4<f32>,
};

@group(3) @binding(0) var<uniform> params: RoomParams;
@group(3) @binding(1) var<uniform> env: SiteEnv;
@group(3) @binding(2) var main_tex: texture_2d<f32>;
@group(3) @binding(3) var main_sampler: sampler;
@group(3) @binding(4) var<uniform> shadow: SiteShadow;
@group(3) @binding(5) var shadow_tex: texture_depth_2d;
@group(3) @binding(6) var shadow_cmp: sampler_comparison;

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

// The vertex input room_shell.rs binds: the mesh pipeline's standard
// locations (it sets the VERTEX_* defs from the mesh) and the third and
// fourth uv sets (TEXCOORD_2 / TEXCOORD_3 of the module glb) where the mesh
// has them.
struct RoomVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef ROOM_UV_2
    @location(8) uv_c: vec2<f32>,
#endif
#ifdef ROOM_UV_3
    @location(9) uv_d: vec2<f32>,
#endif
}

struct RoomVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) fog_ramp: f32,
    @location(6) uv_c: vec2<f32>,
    @location(7) uv_d: vec2<f32>,
}

@vertex
fn vertex(mesh: RoomVertex) -> RoomVertexOutput {
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(mesh.position, 1.0),
    );
    var out: RoomVertexOutput;
    out.position = position_world_to_clip(world_position.xyz);
    out.world_position = world_position;
#ifdef VERTEX_NORMALS
    // Both programs normalize the inverse-transposed normal in the vertex stage.
    out.world_normal = normalize(mesh_functions::mesh_normal_local_to_world(
        mesh.normal,
        mesh.instance_index,
    ));
#else
    out.world_normal = vec3<f32>(0.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = mesh.uv;
#else
    out.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = mesh.uv_b;
#else
    out.uv_b = vec2<f32>(0.0);
#endif
#ifdef VERTEX_COLORS
    out.color = mesh.color;
#else
    out.color = vec4<f32>(1.0);
#endif
#ifdef ROOM_UV_2
    out.uv_c = mesh.uv_c;
#else
    out.uv_c = vec2<f32>(0.0);
#endif
#ifdef ROOM_UV_3
    out.uv_d = mesh.uv_d;
#else
    out.uv_d = vec2<f32>(0.0);
#endif
    // Object fog ramp: the source reads the GL clip z; ((z+n)/(n+f))*f equals
    // f*(clip.w-n)/(f-n) there, and clip.w is the eye depth in either clip
    // convention (the same reconstruction as site_material.wgsl).
    let n = env.projection_params.y;
    let f = env.projection_params.z;
    var fog_depth = f * (out.position.w - n) / (f - n);
    fog_depth = max(fog_depth, 0.0);
    fog_depth = fog_depth * env.fog_params.x + env.fog_params.y;
    out.fog_ramp = clamp(fog_depth, 0.0, 1.0);
    return out;
}

// The mesh uv set a room texture is drawn with: index 0, 1, 2 are uv0, uv1,
// uv2 (room_appearance.rs refuses a mesh that lacks the set its material
// samples).
fn select_uv(in: RoomVertexOutput, index: f32) -> vec2<f32> {
    if index == 2.0 {
        return in.uv_c;
    }
    return select(in.uv, in.uv_b, index == 1.0);
}

fn view_direction(world_position: vec3<f32>) -> vec3<f32> {
    if env.ortho_params.w == 0.0 {
        return normalize(env.camera_position.xyz - world_position);
    }
    return vec3<f32>(env.view_matrix_c0.z, env.view_matrix_c1.z, env.view_matrix_c2.z);
}

fn phenomena_light_blend(rgb: vec3<f32>) -> vec3<f32> {
    return env.phenomena_light_color.w
        * (rgb * env.phenomena_light_color.rgb - rgb)
        + rgb;
}

fn drop_shadow_tint(colour: vec3<f32>) -> vec3<f32> {
    return env.drop_shadow_color.w * (colour * env.drop_shadow_color.rgb - colour) + colour;
}

// ---- main-light shadow (URP soft 9-tap, as transcribed in site_material.wgsl) ----

fn tent_weights(f: f32) -> array<f32, 6> {
    let below = min(f, 0.0);
    let above = max(f, 0.0);
    let inward = 1.0 - f;
    let centre_sq = (f + 0.5) * (f + 0.5);
    let w = 0.159999996;
    let s = 0.0799999982;
    return array<f32, 6>(
        inward * w,
        (inward - below * below + 1.0) * w,
        ((f + 1.0) - above * above + 1.0) * w,
        (f + 1.0) * w,
        (centre_sq * 0.5 - f) * w,
        centre_sq * s,
    );
}

fn pcf9(shadow_uv: vec2<f32>, compare_depth: f32) -> f32 {
    let texel = shadow.size.zw;
    let scaled = shadow_uv * shadow.size.xy;
    let cell = floor(scaled + vec2<f32>(0.5));
    let frac = scaled - cell;
    let wx = tent_weights(frac.x);
    let wy = tent_weights(frac.y);
    let col = vec3<f32>(wx[0] + wx[4], wx[2] + wx[1], wx[5] + wx[3]);
    let row = vec3<f32>(wy[0] + wy[4], wy[5] + wy[3], wy[2] + wy[1]);
    let dx = vec3<f32>(
        (wx[0] / col[0] - 2.5) * texel.x,
        (wx[2] / col[1] - 0.5) * texel.x,
        (wx[5] / col[2] + 1.5) * texel.x,
    );
    let dy = vec3<f32>(
        (wy[0] / row[0] - 2.5) * texel.y,
        (wy[5] / row[1] - 0.5) * texel.y,
        (wy[2] / row[2] + 1.5) * texel.y,
    );
    let base = cell * texel;
    var uvs: array<vec2<f32>, 9>;
    uvs[0] = vec2<f32>(base.x + dx.z, base.y + dy.x);
    uvs[1] = vec2<f32>(base.x + dx.y, base.y + dy.x);
    uvs[2] = vec2<f32>(base.x + dx.x, base.y + dy.x);
    uvs[3] = vec2<f32>(base.x + dx.x, base.y + dy.y);
    uvs[4] = vec2<f32>(base.x + dx.x, base.y + dy.z);
    uvs[5] = vec2<f32>(base.x + dx.y, base.y + dy.y);
    uvs[6] = vec2<f32>(base.x + dx.z, base.y + dy.y);
    uvs[7] = vec2<f32>(base.x + dx.y, base.y + dy.z);
    uvs[8] = vec2<f32>(base.x + dx.z, base.y + dy.z);
    var samples: array<f32, 9>;
    for (var i = 0; i < 9; i += 1) {
        samples[i] = textureSampleCompareLevel(shadow_tex, shadow_cmp, uvs[i], compare_depth);
    }
    var sum = samples[1] * (row[0] * col[1]);
    sum += samples[2] * (row[0] * col[0]);
    sum += samples[0] * (row[0] * col[2]);
    sum += samples[3] * (row[1] * col[0]);
    sum += samples[5] * (row[1] * col[1]);
    sum += samples[6] * (row[1] * col[2]);
    sum += samples[4] * (row[2] * col[0]);
    sum += samples[7] * (row[2] * col[1]);
    sum += samples[8] * (row[2] * col[2]);
    return sum;
}

// Both room programs: strength lerp, then `0 >= z || z >= 1` is unshadowed
// (the test includes z == 0), then the squared distance fade toward 1.
fn main_light_atten(world_position: vec4<f32>) -> f32 {
    let projected = shadow.world_to_shadow * world_position;
    let sum = pcf9(projected.xy, projected.z);
    var atten = sum * shadow.params.x + (1.0 - shadow.params.x);
    if projected.z <= 0.0 || projected.z >= 1.0 {
        atten = 1.0;
    }
    let delta = world_position.xyz - env.camera_position.xyz;
    var fade = clamp(dot(delta, delta) * shadow.params.z + shadow.params.w, 0.0, 1.0);
    fade = fade * fade;
    return fade * (1.0 - atten) + atten;
}

fn treasure_shadow(rgb: vec3<f32>, world_xz: vec2<f32>, treasure_xz: vec2<f32>, intensity: f32) -> vec3<f32> {
    let distance = length(world_xz - treasure_xz);
    var t = (distance + -0.959999979) * 24.9999866;
    t = clamp(t, 0.0, 1.0);
    return intensity * 0.5 * (rgb * t - rgb) + rgb;
}

fn apply_fog(rgb: vec3<f32>, world_y: f32, fog_ramp: f32) -> vec3<f32> {
    let fog_color = clamp(
        fog_ramp * (env.fog_near_color - env.fog_far_color) + env.fog_far_color,
        vec4<f32>(0.0),
        vec4<f32>(1.0),
    );
    let height = min(exp2(-world_y * env.fog_params.z), 1.0) * fog_color.w;
    let distance_blended = fog_ramp * (rgb - fog_color.rgb) + fog_color.rgb;
    return clamp(height * (distance_blended - rgb) + rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(in: RoomVertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    let view = view_direction(in.world_position.xyz);
#ifdef ROOM_OBJECT
    // ---- Mysekai/Object Base ----
    let usage = params.object0.x;
    // Main texture coordinate: the room shell's _BaseTextureMappingMode is one
    // of the uv sets (the skin texture's uvset picks it); local mapping 1
    // overrides with uv0; usage 11 (road scale) never occurs here.
    var coord = select_uv(in, params.object1.x);
    if params.object1.y == 1.0 {
        coord = in.uv;
    }
    let t = env.time.y;
    // uv - time*(scrollX, scrollY); the glb v axis is flipped, so the v scroll
    // changes sign.
    let uv = vec2<f32>(coord.x - t * params.object1.z, coord.y + t * params.object1.w);
    let main = textureSampleBias(main_tex, main_sampler, uv, env.mip_bias.x);
    let alpha = select(main.a, main.a * in.color.w, 0.0 < params.object0.z);
    let ndv = dot(normal, view);
    let vcb = params.object0.y;
    let main_rgb = srgb_format_encode(main.rgb);
    var rgb = vcb * (main_rgb * in.color.rgb - main_rgb) + main_rgb;
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_0.xz, env.treasure_shadow_intensity.x);
    rgb = treasure_shadow(rgb, in.world_position.xz, env.treasure_position_1.xz, env.treasure_shadow_intensity.y);
    let fresnel = exp2(log2(clamp(1.0 - ndv, 0.0, 1.0)) * params.lighting.y);
    let with_fresnel = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
    rgb = select(rgb, with_fresnel, params.lighting.x == 1.0);
    let back = params.back_face_color.a * (params.back_face_color.rgb - rgb) + rgb;
    let faced = select(back, rgb, front);
    rgb = select(faced, rgb, params.object0.w != 0.0);
    if 0.5 < params.lighting.z {
        // _UseObject3DPreviewLight is 0 on every room shell material (checked
        // when the material is built), so the phenomena colours are read.
        let lit = phenomena_light_blend(rgb);
        let ndl = clamp(dot(env.light_vector.xyz, normal), 0.0, 1.0);
        var mask = clamp(
            (ndl - env.shadow_mask_edges.x) / (env.shadow_mask_edges.y - env.shadow_mask_edges.x),
            0.0,
            1.0,
        );
        mask = select(mask, 1.0, usage != 0.0);
        // _DebugShadowMask has no writer (the environment view writes the edge
        // pair and both mask edges, not the debug flag), so the debug arm is off.
        rgb = lit;
#ifdef ROOM_RECEIVE_SHADOWS
        let atten = main_light_atten(in.world_position);
#endif
        let shade = env.phenomena_shade_color;
        let use_local = 0.5 < params.shading.x;
        let intensity = select(1.0, params.shading.y, use_local);
        if usage == 2.0 {
            // Wall AO: ao = intensity*(-vc.r) + 1, times the edge factor
            // _WallAOIntensity*(d - 1) + 1 with p = exp2((sx, sy)*e *
            // log2(|2*uv3 - 1|)) and d = 1 - dot(p, p), in the source's
            // operation order. uv3 is the mesh's fourth uv set (0..1 across
            // each wall) in the source's bottom-left origin; the glb stores V
            // flipped. A mesh without the set (a module file exported before
            // the uv-set export) has no ROOM_UV_3 input and is drawn without
            // the factor; room_appearance.rs names it.
            var ao = intensity * (-in.color.r) + 1.0;
#ifdef ROOM_UV_3
            let uv3 = vec2<f32>(in.uv_d.x, 1.0 - in.uv_d.y);
            let edge_uv = uv3 * vec2<f32>(2.0, 2.0) + vec2<f32>(-1.0, -1.0);
            let edge_exponent = vec2<f32>(params.wall_ao.y, params.wall_ao.z)
                * vec2<f32>(params.wall_ao.w, params.wall_ao.w);
            let edge = exp2(edge_exponent * log2(abs(edge_uv)));
            let d = -dot(edge, edge) + 1.0;
            ao = ao * (params.wall_ao.x * (d + -1.0) + 1.0);
#endif
            let shaded = shade.rgb * rgb;
            let toward = (-rgb) * shade.rgb + rgb;
            rgb = ao * toward + shaded;
        } else if usage == 14.0 {
            // The skip arm: the light-blended colour goes on unshaded.
        } else {
            let threshold = select(env.edge_threshold.x, params.shading.z, use_local);
            let smoothness = select(env.edge_smoothness.x, params.shading.w, use_local);
            let rgb2 = vcb * (rgb * in.color.rgb - rgb) + rgb;
            let half_lambert = ndl * 0.5 + 0.5;
            let upper = smoothness + threshold;
            let lower = -smoothness + threshold;
            let ramp = clamp((half_lambert - upper) / (lower - upper), 0.0, 1.0);
            let factor = intensity * (shade.w * ramp);
            rgb = factor * (shade.rgb * rgb2 - rgb2) + rgb2;
        }
        // Drop shadow: the road term is taken only for usage 11. Shadowless
        // program: s = mask*0 + 1. Shadow program: s = mask*(atten*atten - 1) + 1.
#ifdef ROOM_RECEIVE_SHADOWS
        let s = mask * (atten * atten + -1.0) + 1.0;
#else
        let s = mask * 0.0 + 1.0;
#endif
        let tinted = drop_shadow_tint(rgb);
        rgb = s * (rgb - tinted) + tinted;
        rgb = apply_fog(rgb, in.world_position.y, in.fog_ramp);
    }
    rgb = params.additive_color.rgb * params.additive_color.a + rgb;
    var out_rgb = rgb;
    if 0.5 < params.height_fade.x {
        let pml = -params.height_fade.z + params.height_fade.y;
        let num = -pml + in.world_position.y;
        let den = -pml + params.height_fade.y;
        var thf = clamp(num / den, 0.0, 1.0);
        thf = min(exp2(log2(thf) * params.height_fade.w), 1.0);
        out_rgb = thf * (rgb - env.sky_bottom_color.rgb) + env.sky_bottom_color.rgb;
    }
    return vec4<f32>(srgb_format_decode(out_rgb), alpha * params.lighting.w);
#endif
#ifdef ROOM_FLOOR
    // ---- Mysekai/Room/Floor Base (_MAIN_LIGHT_SHADOWS program) ----
    let main = textureSampleBias(main_tex, main_sampler, select_uv(in, params.object1.x), env.mip_bias.x);
    var rgb = srgb_format_encode(main.rgb) * in.color.rgb;
    let fresnel = exp2(log2(clamp(1.0 - dot(normal, view), 0.0, 1.0)) * params.lighting.y);
    let with_fresnel = fresnel * params.fresnel_color.rgb * params.fresnel_color.a + rgb;
    rgb = select(rgb, with_fresnel, 0.5 < params.lighting.x);
    if 0.5 < params.lighting.z {
        let atten = main_light_atten(in.world_position);
        let lit = phenomena_light_blend(rgb);
        let tinted = drop_shadow_tint(lit);
        rgb = atten * (lit - tinted) + tinted;
    }
    return vec4<f32>(srgb_format_decode(rgb), main.a);
#endif
}
