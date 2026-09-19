// Copy the actual camera attachment after opaques, one raw texel per pixel.
// Source and destination are separate, single-sample Depth32Float textures of
// equal extent. No geometry replay, filtering, projection conversion, resolve,
// comparison sampler or colour target participates in this operation.
@group(0) @binding(0) var camera_depth: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(camera_depth, vec2<i32>(pixel.xy), 0).x;
}
