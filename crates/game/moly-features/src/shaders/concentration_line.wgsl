// Mysekai/ConcentrationLine: the field camera's speed lines, drawn by the
// RawImage of the FieldCamera prefab's screen-space overlay canvas.
//
// Vertex: the unit quad (positions in [-0.5, 0.5]) is written straight into
// clip xy, so it covers the viewport whatever the overlay camera's
// projection; clip z is the near plane, the overlay canvas's
// "always pass" depth test. The texture coordinate is the RawImage's: the
// rect's corners map to its uvRect, (0, 0) at the bottom left, y up.
//
// Fragment: a line-by-line transcription of the shipped program (the one
// without instancing keywords). It is a shader graph: the polar angle of the
// uv around the centre, three octaves of value noise over that angle, a sine
// of the noise plus time, mixed with twice the distance from the centre and
// smooth-stepped between two material floats. The output colour is the
// material colour and the alpha is that coverage times its alpha; the
// vertex colour is not read.
//
// Colour domain: the material colour is the stored value of a Gamma-space
// player; this pass writes to an sRGB attachment, so the colour is decoded
// once and the attachment's encode restores the stored value. Blending then
// happens in linear light, as for the product's other overlay draws; the
// source blends encoded values (named in speed_lines.rs).

struct ConcentrationLine {
    // _TimeParameters: (t, sin t, cos t, 0).
    time_parameters: vec4<f32>,
    // (Vector1_2, Vector1_603d0dab08414d719cd581f940fbdbc8,
    //  Vector1_1, Vector1_faf012929d054e78b256dae299677873)
    a: vec4<f32>,
    // (Vector1_266bcaed2966410aa10f41605fd8de44,
    //  Vector1_21a8a39812064880bc8182238f00af1b, enabled, 0)
    b: vec4<f32>,
    // Color_28639f00d0534a889342fdf1a9d89b8a, as stored.
    color: vec4<f32>,
    // RawImage.uvRect: (x, y, width, height).
    uv_rect: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: ConcentrationLine;

struct Vertex {
    @location(0) position: vec3<f32>,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(input: Vertex) -> Varyings {
    var out: Varyings;
    out.position = vec4<f32>(input.position.xy * 2.0, 1.0, 1.0);
    let corner = input.position.xy + vec2<f32>(0.5, 0.5);
    out.uv = material.uv_rect.xy + corner * material.uv_rect.zw;
    return out;
}

const K: vec2<f32> = vec2<f32>(12.9898005, 78.2330017);

fn srgb_format_decode(stored: vec3<f32>) -> vec3<f32> {
    let e = clamp(stored, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((e + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, e / 12.92, e <= vec3<f32>(0.04045));
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    let Vector1_2 = material.a.x;
    let Vector1_603d = material.a.y;
    let Vector1_1 = material.a.z;
    let Vector1_faf0 = material.a.w;
    let Vector1_266b = material.b.x;
    let Vector1_21a8 = material.b.y;
    let enabled = material.b.z;

    // u_xlat0.xy: the uv around the centre.
    let p = input.uv + vec2<f32>(-0.5, -0.5);
    // atan2(p.x, p.y) in the compiled polynomial form.
    var t8 = 1.0 / max(abs(p.y), abs(p.x));
    var t12 = min(abs(p.y), abs(p.x));
    t8 = t8 * t12;
    t12 = t8 * t8;
    var t1 = t12 * 0.0208350997 + -0.0851330012;
    t1 = t12 * t1 + 0.180141002;
    t1 = t12 * t1 + -0.330299497;
    t12 = t12 * t1 + 0.999866009;
    t1 = t12 * t8;
    t1 = t1 * -2.0 + 1.57079637;
    t1 = select(0.0, t1, abs(p.y) < abs(p.x));
    t8 = t8 * t12 + t1;
    t12 = select(0.0, -3.14159274, p.y < -p.y);
    t8 = t12 + t8;
    t12 = min(p.y, p.x);
    let below = t12 < -t12;
    t1 = max(p.y, p.x);
    // u_xlat0.x: the distance from the centre.
    let r = sqrt(dot(p, p));
    let flip = (t1 >= -t1) && below;
    var angle = select(t8, -t8, flip);
    angle = angle * Vector1_266b;

    // Three octaves of value noise at (v, v) for v = angle times each scale.
    var u4 = vec3<f32>(angle) * vec3<f32>(38.2165565, 19.1082783, 9.55413914);
    var u1 = fract(u4);
    u4 = floor(u4);
    let u2s = u1 * u1;
    u1 = -u1 * vec3<f32>(2.0, 2.0, 2.0) + vec3<f32>(3.0, 3.0, 3.0);
    let u3 = -u2s * u1 + vec3<f32>(1.0, 1.0, 1.0);
    u1 = u1 * u2s;
    var u2 = u4.xxyy + vec4<f32>(0.0, 1.0, 0.0, 1.0);
    var u13 = dot(u2.zw, K);
    u13 = sin(u13);
    u13 = u13 * 43758.5469;
    u13 = fract(u13);
    var u15 = dot(u2.ww, K);
    u15 = sin(u15);
    u15 = u15 * 43758.5469;
    u15 = fract(u15);
    u15 = u1.y * u15;
    u13 = u3.y * u13 + u15;
    u13 = u13 * u1.y;
    var u8 = vec2<f32>(dot(u4.yy, K), 0.0);
    u8.x = sin(u8.x);
    u8.x = u8.x * 43758.5469;
    u8.x = fract(u8.x);
    var u10 = dot(u2.wz, K);
    u10 = sin(u10);
    u10 = u10 * 43758.5469;
    u10 = fract(u10);
    var u5 = vec3<f32>(u1.y * u10, 0.0, 0.0);
    u8.x = u3.y * u8.x + u5.x;
    u4.y = u3.y * u8.x + u13;
    u5.x = dot(u2.xy, K);
    u5.x = sin(u5.x);
    u5.x = u5.x * 43758.5469;
    u13 = dot(u2.yy, K);
    u2.x = dot(u2.yx, K);
    u2.x = sin(u2.x);
    u2.x = u2.x * 43758.5469;
    u2.x = fract(u2.x);
    u2.x = u1.x * u2.x;
    u13 = sin(u13);
    u5.z = u13 * 43758.5469;
    u5.x = fract(u5.x);
    u5.z = fract(u5.z);
    u13 = u5.z * u1.x;
    u5.x = u3.x * u5.x + u13;
    u1.x = u5.x * u1.x;
    u4.x = dot(u4.xx, K);
    u4.x = sin(u4.x);
    u4.x = u4.x * 43758.5469;
    u4.y = u4.y * 0.25;
    u4.x = fract(u4.x);
    u4.x = u3.x * u4.x + u2.x;
    u4.x = u3.x * u4.x + u1.x;
    u4.x = u4.x * 0.125 + u4.y;
    let u1xy = u4.zz + vec2<f32>(0.0, 1.0);
    u1.x = u1xy.x;
    u1.y = u1xy.y;
    u8.x = dot(u4.zz, K);
    u8.x = sin(u8.x);
    u8.x = u8.x * 43758.5469;
    var u12 = dot(u1.xy, K);
    u12 = sin(u12);
    u8.y = u12 * 43758.5469;
    u8 = fract(u8);
    u13 = dot(u1.yy, K);
    u1.x = dot(u1.yx, K);
    u1.x = sin(u1.x);
    u1.x = u1.x * 43758.5469;
    u1.x = fract(u1.x);
    u1.x = u1.x * u1.z;
    u8.x = u3.z * u8.x + u1.x;
    u1.x = sin(u13);
    u1.x = u1.x * 43758.5469;
    u1.x = fract(u1.x);
    u1.x = u1.x * u1.z;
    u12 = u3.z * u8.y + u1.x;
    u12 = u12 * u1.z;
    u8.x = u3.z * u8.x + u12;
    u4.x = u8.x * 0.5 + u4.x;

    // The sine of the noise plus time, mixed with twice the distance.
    u8.x = material.time_parameters.x * Vector1_2;
    u4.x = u4.x * Vector1_21a8 + u8.x;
    u4.x = sin(u4.x);
    u4.x = -r * 2.0 + u4.x;
    var c = r + r;
    c = Vector1_faf0 * u4.x + c;
    c = c + -Vector1_603d;
    u4.x = -Vector1_603d + Vector1_1;
    u4.x = 1.0 / u4.x;
    c = u4.x * c;
    c = clamp(c, 0.0, 1.0);
    u4.x = c * -2.0 + 3.0;
    c = c * c;
    c = c * u4.x;

    // RawImage.enabled: a disabled image contributes nothing.
    let alpha = c * material.color.w * enabled;
    return vec4<f32>(srgb_format_decode(material.color.xyz), alpha);
}
