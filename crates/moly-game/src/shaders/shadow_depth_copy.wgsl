// Copies the static shadow depth map into the frame's map, one texel per
// pixel. Both are single-sample Depth32Float textures of the same size; the
// stored depth is written unchanged (no comparison or projection). The map is
// bound as an unfilterable float texture and read through a nearest,
// non-comparison sampler at the pixel's texel centre. On the GL backend a
// depth texture read without a sampler keeps the texture's own linear filter,
// which a 32-bit float depth format cannot use, and reads 0; and a
// `texture_depth_2d` becomes a shadow sampler, which has no plain lookup.
@group(0) @binding(0) var static_depth: texture_2d<f32>;
@group(0) @binding(1) var static_sampler: sampler;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = vec2<f32>(textureDimensions(static_depth));
    return textureSampleLevel(static_depth, static_sampler, pixel.xy / size, 0.0).x;
}
