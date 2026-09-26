// Character silhouette: the depth-stencil replay of the stencil-relevant
// draws and the avatar's stencil-shadow pass. The slot layout is a contract
// with character_silhouette.rs; change both together.
//
// Group 0: the view. Group 1: the draw's space, one world matrix (rigid) or
// the joint palette window of its skin (SKINNED). Group 2, read only by the
// silhouette fragment: the scene colour under the pass, the scene depth and
// the silhouette colour.

struct SilhouetteView {
    clip_from_world: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> view: SilhouetteView;

#ifdef SKINNED
// Each matrix is the joint's world transform times its inverse bind pose.
@group(1) @binding(0) var<uniform> joint_palette: array<mat4x4<f32>, 256>;
#else
@group(1) @binding(0) var<uniform> world_from_local: mat4x4<f32>;
#endif

struct Vertex {
    @location(0) position: vec3<f32>,
#ifdef SKINNED
    @location(1) joints: vec4<u32>,
    @location(2) weights: vec4<f32>,
#endif
};

// The same world and clip positions the colour pass computes for the mesh:
// the blended joint matrix (or the world matrix) applied to the local
// position, then the view's clip-from-world matrix.
@vertex
fn vertex(in: Vertex) -> @builtin(position) vec4<f32> {
#ifdef SKINNED
    let model = in.weights.x * joint_palette[in.joints.x]
        + in.weights.y * joint_palette[in.joints.y]
        + in.weights.z * joint_palette[in.joints.z]
        + in.weights.w * joint_palette[in.joints.w];
#else
    let model = world_from_local;
#endif
    let world = model * vec4<f32>(in.position, 1.0);
    return view.clip_from_world * vec4<f32>(world.xyz, 1.0);
}

@group(2) @binding(0) var scene_colour: texture_2d<f32>;
// The camera depth read raw, one texel per pixel (a depth texture bound as
// unfilterable float, readable on every backend).
@group(2) @binding(1) var scene_depth: texture_2d<f32>;
@group(2) @binding(2) var<uniform> silhouette_colour: vec4<f32>;

// The sRGB transfer both ways, on [0, 1]: the scene target stores linear
// values and encodes them on write, the source's frame buffer holds the
// encoded values themselves.
fn encode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = x * 12.92;
    let high = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, x <= vec3<f32>(0.0031308));
}

fn decode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = x / 12.92;
    let high = pow((x + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, x <= vec3<f32>(0.04045));
}

// The stencil-shadow pass's fragment: its program outputs the silhouette
// colour, which the pass blends with SrcAlpha / OneMinusSrcAlpha for colour
// and alpha alike, in the encoded values of a gamma colour-space frame. Its
// depth test is Greater: the fragment passes where it lies behind what the
// scene holds at the pixel. The product's depth is reversed (1 near, 0 far),
// so behind is a smaller value. The stencil test and its operations run in
// the pipeline state.
@fragment
fn silhouette_fragment(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    if !(pixel.z < textureLoad(scene_depth, p, 0).x) {
        discard;
    }
    let under = textureLoad(scene_colour, p, 0);
    let a = silhouette_colour.a;
    let rgb = silhouette_colour.rgb * a + encode(under.rgb) * (1.0 - a);
    return vec4<f32>(decode(rgb), a * a + under.a * (1.0 - a));
}
