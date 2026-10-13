// Copy the actual camera attachment after opaques, one raw texel per pixel.
// Source and destination are separate, single-sample Depth32Float textures of
// equal extent. No geometry replay, filtering, projection conversion, resolve,
// comparison sampler or colour target participates in this operation. The
// attachment is read through a nearest, non-comparison sampler at the texel
// centre (see `raw_depth_layout`).
@group(0) @binding(0) var camera_depth: texture_2d<f32>;
@group(0) @binding(1) var depth_sampler: sampler;

fn depth_at(pixel: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(camera_depth));
    return textureSampleLevel(camera_depth, depth_sampler, pixel / size, 0.0).x;
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
    return depth_at(pixel.xy);
}

// The original GLES program's ComputeScreenPos is bottom-left-origin and
// its ZBufferParams describe forward [0,1] depth. Adapt the sampled attachment,
// not the original program or its clip-space varyings. Actual depth testing
// continues against the unchanged reversed camera attachment.
@fragment
fn source_fragment(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
    let height = f32(textureDimensions(camera_depth).y);
    return 1.0 - depth_at(vec2<f32>(pixel.x, height - pixel.y));
}
