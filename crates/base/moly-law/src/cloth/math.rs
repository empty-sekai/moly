//! 平面向量/四元数最小算术。只实现布料律用到的运算；约定
//! `[f32; 4]` 四元数是 `(x, y, z, w)`，引擎翻译归调用方。

/// 欧几里得模长。
pub fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// 归一化；零向量原样返回（调用方负责先用 [`length`] 把关）。
pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len <= f32::EPSILON {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// 两单位向量间的四元数（THREE.setFromUnitVectors / Unity
/// FromToRotation 的最小形）：轴=叉积、角=反余弦；反向退化沿 +Y
/// 转 π（与 demo cloth.js 的退化臂一致）。
pub fn from_unit_vectors(from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    let d = dot(from, to);
    if d > 1.0 - 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if d < -(1.0 - 1e-6) {
        // 反向退化：任取与 from 不平行的轴。demo 取 +Y。
        let axis = if from[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let a = normalize(cross(axis, from));
        return [a[0], a[1], a[2], 0.0];
    }
    let axis = cross(from, to);
    [axis[0], axis[1], axis[2], 1.0 + d]
}

pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if len <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

pub fn quat_identity() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

/// 单位四元数共轭即逆。
pub fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// 单位四元数旋转向量。
pub fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let s = q[3];
    let uv = cross(u, v);
    let uuv = cross(u, uv);
    [
        v[0] + 2.0 * (s * uv[0] + uuv[0]),
        v[1] + 2.0 * (s * uv[1] + uuv[1]),
        v[2] + 2.0 * (s * uv[2] + uuv[2]),
    ]
}

/// 球面插值 `a → b`（`t ∈ [0,1]`），demo `THREE.Quaternion.slerp`
/// 同形：点积负先取反走短弧；近平行（dot > 0.9995）退化线性 +
/// 归一化；否则正弦权重。返回未强制归一（slerp 本身保长度，
/// 线性臂由 normalize 兜底）。
pub fn quat_slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let mut bb = b;
    if d < 0.0 {
        d = -d;
        bb = [-b[0], -b[1], -b[2], -b[3]];
    }
    if d > 0.9995 {
        return quat_normalize([
            a[0] + t * (bb[0] - a[0]),
            a[1] + t * (bb[1] - a[1]),
            a[2] + t * (bb[2] - a[2]),
            a[3] + t * (bb[3] - a[3]),
        ]);
    }
    let theta = d.clamp(-1.0, 1.0).acos();
    let sin_t = theta.sin();
    let wa = ((1.0 - t) * theta).sin() / sin_t;
    let wb = (t * theta).sin() / sin_t;
    [
        a[0] * wa + bb[0] * wb,
        a[1] * wa + bb[1] * wb,
        a[2] * wa + bb[2] * wb,
        a[3] * wa + bb[3] * wb,
    ]
}

/// 两单位向量夹角（弧度）。
pub fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}
