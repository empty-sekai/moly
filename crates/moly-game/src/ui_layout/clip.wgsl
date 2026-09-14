// Clipping terms for UI/Default, FillColor and the two TMP material families.
// Source color/font shading remains the existing prefab host's bitmap path.
#import bevy_sprite::mesh2d_functions as mesh_functions
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#import bevy_sprite::mesh2d_view_bindings::view
#endif

struct ClipUniform {
    color: vec4<f32>,
    rect: vec4<f32>,
    softness_pixel: vec4<f32>,
    flags: vec4<u32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: ClipUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var image: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var image_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
    @location(2) uv: vec2<f32>,
#ifdef VERTEX_COLORS
    @location(4) color: vec4<f32>,
#endif
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) canvas_position: vec2<f32>,
    @location(2) mask: vec4<f32>,
    @location(3) color: vec4<f32>,
}

@vertex
fn vertex(input: Vertex) -> Varyings {
    var output: Varyings;
    let world = mesh_functions::mesh2d_position_local_to_world(
        mesh_functions::get_world_from_local(input.instance_index), vec4(input.position, 1.0));
    output.position = mesh_functions::mesh2d_position_world_to_clip(world);
    output.uv = input.uv;
    output.canvas_position = input.position.xy;
    let rect = clamp(material.rect, vec4(-2e10), vec4(2e10));
    output.mask = vec4(input.position.xy * 2.0 - rect.xy - rect.zw,
        vec2(0.25) / (vec2(0.25) * material.softness_pixel.xy + abs(material.softness_pixel.zw)));
    output.color = material.color;
#ifdef VERTEX_COLORS
    output.color *= input.color;
#endif
    return output;
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    var color = input.color;
    if material.flags.x != 0u {
        let sampled = textureSample(image, image_sampler, input.uv);
        if material.flags.z != 0u {
            // Source FillColor takes the texture's alpha only.
            color.a *= sampled.a;
        } else {
            color *= sampled;
        }
    }
    var factor = 1.0;
    if material.flags.y != 0u {
        let inside = all(input.canvas_position >= material.rect.xy)
            && all(input.canvas_position <= material.rect.zw);
        factor = select(0.0, 1.0, inside);
    } else {
        let m = clamp((material.rect.zw - material.rect.xy - abs(input.mask.xy))
            * input.mask.zw, vec2(0.0), vec2(1.0));
        factor = m.x * m.y;
    }
    // The host's straight-alpha blend applies the same factor to the source
    // contribution as the source UI/TMP premultiplied output + One blend.
    color.a *= factor;
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
