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

// The original GLES program's ComputeScreenPos is bottom-left-origin and
// its ZBufferParams describe forward [0,1] depth. Adapt the sampled attachment,
// not the original program or its clip-space varyings. Actual depth testing
// continues against the unchanged reversed camera attachment.
@fragment
fn source_fragment(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = vec2<i32>(textureDimensions(camera_depth));
    let p = vec2<i32>(i32(pixel.x), size.y - 1 - i32(pixel.y));
    return 1.0 - textureLoad(camera_depth, p, 0).x;
}
