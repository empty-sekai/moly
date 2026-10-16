// Sekai/Area/WipeCircle: the circle wipe of the door moves, drawn by the
// `wipe` RawImage (6000 x 6000 canvas units, centred) of the
// SiteTransitionerNormal prefab, under the screen manager's layer canvas.
//
// Vertex: the unit quad (positions in [-0.5, 0.5]) is written straight into
// clip xy, so it covers the viewport; clip z is the near plane, the overlay
// canvas's "always pass" depth test. The varying carries the canvas point of
// the fragment (canvas units from the screen centre, y up), from which the
// RawImage's texture coordinate is derived: (0, 0) at the rect's bottom left.
//
// Fragment: the shipped program line by line. Vertex stage:
// `uv - (_OffsetX, _OffsetY)`, `* 2 - 1`, `* _Scale^3`; fragment stage:
// alpha = `sqrt(dot(p, p)) >= 1.0 ? 1 : 0`, colour = `_Color.xyz` (the
// RawImage colour is white, so the vertex colour is the material colour).
// The expression is affine in uv, so evaluating it per fragment equals the
// source's interpolated varying. Fragments outside the 6000-unit rect are
// not drawn (the rect has no fragment there).
//
// Colour domain: `_Color` is the stored value of a Gamma-space player; this
// pass writes to an sRGB attachment, so the colour is decoded once and the
// attachment's encode restores the stored value. The alpha is 0 or 1, so the
// blend (SrcAlpha / OneMinusSrcAlpha) is the same in either domain.

struct WipeCircle {
    // (_Scale, _OffsetX, _OffsetY, 0)
    params: vec4<f32>,
    // Half the screen in canvas units (x, y), 0, 0.
    half_extent: vec4<f32>,
    // _Color, as stored.
    color: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: WipeCircle;

struct Vertex {
    @location(0) position: vec3<f32>,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) canvas: vec2<f32>,
}

@vertex
fn vertex(input: Vertex) -> Varyings {
    var out: Varyings;
    let clip = input.position.xy * 2.0;
    out.position = vec4<f32>(clip, 1.0, 1.0);
    out.canvas = clip * material.half_extent.xy;
    return out;
}

const WIPE_RECT: f32 = 6000.0;

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    let uv = input.canvas / WIPE_RECT + vec2<f32>(0.5, 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        discard;
    }
    let scale = material.params.x;
    let offset = material.params.yz;
    // u_xlat0.xy = uv - offset; then * 2 - 1.
    var a = uv - offset;
    a = a * vec2<f32>(2.0, 2.0) + vec2<f32>(-1.0, -1.0);
    // u_xlat4 = _Scale * _Scale * _Scale.
    var s = scale * scale;
    s = s * scale;
    let p = vec2<f32>(s) * a;
    let alpha = select(0.0, 1.0, sqrt(dot(p, p)) >= 1.0);
    return vec4<f32>(srgb_format_decode(material.color.xyz), alpha);
}
