// Format adaptation only. Source programs and their blend state are unchanged.
// Point loads preserve texel locations; alpha is never gamma transformed.
@group(0) @binding(0) var image: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055,
        c * 12.92, c <= vec3<f32>(0.0031308));
}

fn decode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4)),
        c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn to_encoded(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(position.xy), 0);
    return vec4<f32>(encode_srgb(c.rgb), c.a);
}

@fragment
fn to_linear(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(position.xy), 0);
    return vec4<f32>(decode_srgb(c.rgb), c.a);
}
