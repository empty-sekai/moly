//! Vector, quaternion and matrix helpers in the engine's evaluation order.
//! Each step is one binary32 operation through the ARM rules; nothing fuses.
use crate::particle::armf as a;

pub(super) type V3 = [f32; 3];

pub(super) const FLT_MAX: f32 = f32::MAX;
/// 1e-3.
pub(super) const EPS_SAME: f32 = f32::from_bits(0x3a83_126f);
/// The single-precision machine epsilon, 2^-23.
pub(super) const EPS: f32 = f32::from_bits(0x3400_0000);

#[inline]
pub(super) fn add(x: V3, y: V3) -> V3 {
    [a::add(x[0], y[0]), a::add(x[1], y[1]), a::add(x[2], y[2])]
}

#[inline]
pub(super) fn sub(x: V3, y: V3) -> V3 {
    [a::sub(x[0], y[0]), a::sub(x[1], y[1]), a::sub(x[2], y[2])]
}

#[inline]
pub(super) fn scale(x: V3, s: f32) -> V3 {
    [a::mul(x[0], s), a::mul(x[1], s), a::mul(x[2], s)]
}

#[inline]
pub(super) fn neg(x: V3) -> V3 {
    x.map(a::neg)
}

/// `(x.x*y.x + x.y*y.y) + x.z*y.z`.
#[inline]
pub(super) fn dot(x: V3, y: V3) -> f32 {
    a::add(a::add(a::mul(x[0], y[0]), a::mul(x[1], y[1])), a::mul(x[2], y[2]))
}

#[inline]
pub(super) fn cross(x: V3, y: V3) -> V3 {
    [
        a::sub(a::mul(x[1], y[2]), a::mul(x[2], y[1])),
        a::sub(a::mul(x[2], y[0]), a::mul(x[0], y[2])),
        a::sub(a::mul(x[0], y[1]), a::mul(x[1], y[0])),
    ]
}

#[inline]
pub(super) fn mag2(x: V3) -> f32 {
    dot(x, x)
}

#[inline]
pub(super) fn magnitude(x: V3) -> f32 {
    a::sqrt(mag2(x))
}

/// `PxVec3::operator/`: the reciprocal first, then three products.
#[inline]
pub(super) fn div_scalar(x: V3, f: f32) -> V3 {
    scale(x, a::div(1.0, f))
}

/// `PxVec3::normalize`: divided by the magnitude when it is positive; the
/// magnitude is returned.
#[inline]
pub(super) fn normalize(x: V3) -> (V3, f32) {
    let m = magnitude(x);
    (if m > 0.0 { div_scalar(x, m) } else { x }, m)
}

/// `PxVec3::getNormalized`: times the reciprocal of the square root of the
/// squared magnitude when that is positive, else zero.
#[inline]
pub(super) fn get_normalized(x: V3) -> V3 {
    let m = mag2(x);
    if m > 0.0 { scale(x, a::div(1.0, a::sqrt(m))) } else { [0.0; 3] }
}

/// C++ `a < b ? a : b`.
#[inline]
pub(super) fn sel_min(x: f32, y: f32) -> f32 {
    if x < y { x } else { y }
}

/// C++ `a > b ? a : b`.
#[inline]
pub(super) fn sel_max(x: f32, y: f32) -> f32 {
    if x > y { x } else { y }
}

/// FRECPE and `n` steps `e = e * FRECPS(x, e)`.
pub(super) fn recip_n(x: f32, n: usize) -> f32 {
    let mut e = a::recip_estimate(x);
    for _ in 0..n {
        e = a::mul(e, a::recip_step(x, e));
    }
    e
}

/// FRSQRTE and `n` steps `r = r * FRSQRTS(r*r, x)`.
pub(super) fn rsqrt_n(x: f32, n: usize) -> f32 {
    let mut r = a::rsqrt_estimate(x);
    for _ in 0..n {
        r = a::mul(r, a::rsqrt_step(a::mul(r, r), x));
    }
    r
}

/// The vector library's square root: `a == 0 ? a : a * rsqrt<4>(a)`.
#[inline]
pub(super) fn fsqrt(x: f32) -> f32 {
    if x == 0.0 { x } else { a::mul(x, rsqrt_n(x, 4)) }
}

/// The vector library's reciprocal, four refinement steps.
#[inline]
pub(super) fn frecip(x: f32) -> f32 {
    recip_n(x, 4)
}

/// The vector `V3Dot`: `(x.x*y.x + x.y*y.y) + (x.z*y.z + w*w')` with the
/// fourth lanes zero.
#[inline]
pub(super) fn v3dot(x: V3, y: V3) -> f32 {
    a::add(a::add(a::mul(x[0], y[0]), a::mul(x[1], y[1])), a::add(a::mul(x[2], y[2]), 0.0))
}

/// One lane of the transposed `V3Dot4`: `(x.x*y.x + x.y*y.y) + x.z*y.z`.
#[inline]
pub(super) fn v3dot4(x: V3, y: V3) -> f32 {
    dot(x, y)
}

#[inline]
pub(super) fn v3normalize(x: V3) -> V3 {
    let inv = frecip(fsqrt(v3dot(x, x)));
    x.map(|c| a::mul(c, inv))
}

#[inline]
pub(super) fn v3length(x: V3) -> f32 {
    fsqrt(v3dot(x, x))
}

/// `c + x*b`.
#[inline]
pub(super) fn v3scale_add(x: V3, b: f32, c: V3) -> V3 {
    std::array::from_fn(|k| a::add(c[k], a::mul(x[k], b)))
}

/// `c - x*b`.
#[inline]
pub(super) fn v3neg_scale_sub(x: V3, b: f32, c: V3) -> V3 {
    std::array::from_fn(|k| a::sub(c[k], a::mul(x[k], b)))
}

/// FMIN then FMAX.
#[inline]
pub(super) fn fclamp(x: f32, lo: f32, hi: f32) -> f32 {
    a::max(a::min(x, hi), lo)
}

/// `PxQuat::rotate`.
pub(super) fn quat_rotate(q: [f32; 4], v: V3) -> V3 {
    let [x, y, z, w] = q;
    let (vx, vy, vz) = (a::add(v[0], v[0]), a::add(v[1], v[1]), a::add(v[2], v[2]));
    let w2 = a::sub(a::mul(w, w), 0.5);
    let dot2 = a::add(a::add(a::mul(x, vx), a::mul(y, vy)), a::mul(z, vz));
    [
        a::add(a::add(a::mul(vx, w2), a::mul(a::sub(a::mul(y, vz), a::mul(z, vy)), w)), a::mul(x, dot2)),
        a::add(a::add(a::mul(vy, w2), a::mul(a::sub(a::mul(z, vx), a::mul(x, vz)), w)), a::mul(y, dot2)),
        a::add(a::add(a::mul(vz, w2), a::mul(a::sub(a::mul(x, vy), a::mul(y, vx)), w)), a::mul(z, dot2)),
    ]
}

/// `PxQuat::rotateInv`.
pub(super) fn quat_rotate_inv(q: [f32; 4], v: V3) -> V3 {
    let [x, y, z, w] = q;
    let (vx, vy, vz) = (a::add(v[0], v[0]), a::add(v[1], v[1]), a::add(v[2], v[2]));
    let w2 = a::sub(a::mul(w, w), 0.5);
    let dot2 = a::add(a::add(a::mul(x, vx), a::mul(y, vy)), a::mul(z, vz));
    [
        a::add(a::sub(a::mul(vx, w2), a::mul(a::sub(a::mul(y, vz), a::mul(z, vy)), w)), a::mul(x, dot2)),
        a::add(a::sub(a::mul(vy, w2), a::mul(a::sub(a::mul(z, vx), a::mul(x, vz)), w)), a::mul(y, dot2)),
        a::add(a::sub(a::mul(vz, w2), a::mul(a::sub(a::mul(x, vy), a::mul(y, vx)), w)), a::mul(z, dot2)),
    ]
}

/// `PxTransform::transformInv` of a point.
#[inline]
pub(super) fn transform_inv(rotation: [f32; 4], translation: V3, p: V3) -> V3 {
    quat_rotate_inv(rotation, sub(p, translation))
}

/// `Cm::Matrix34` of a transform: three rotation columns and the translation.
#[derive(Clone, Copy, Debug)]
pub(super) struct Matrix34 {
    pub(super) columns: [V3; 3],
    pub(super) translation: V3,
}

impl Matrix34 {
    pub(super) fn from_pose(q: [f32; 4], t: V3) -> Self {
        let [x, y, z, w] = q;
        let (x2, y2, z2) = (a::add(x, x), a::add(y, y), a::add(z, z));
        let (xx, yy, zz) = (a::mul(x, x2), a::mul(y, y2), a::mul(z, z2));
        let (xy, xz, xw) = (a::mul(x2, y), a::mul(x2, z), a::mul(w, x2));
        let (yz, yw, zw) = (a::mul(y2, z), a::mul(w, y2), a::mul(w, z2));
        Self {
            columns: [
                [a::sub(a::sub(1.0, yy), zz), a::add(xy, zw), a::sub(xz, yw)],
                [a::sub(xy, zw), a::sub(a::sub(1.0, xx), zz), a::add(yz, xw)],
                [a::add(xz, yw), a::sub(yz, xw), a::sub(a::sub(1.0, xx), yy)],
            ],
            translation: t,
        }
    }

    /// `((c0*p.x + c1*p.y) + c2*p.z) + t` per row.
    pub(super) fn transform(&self, p: V3) -> V3 {
        let [c0, c1, c2] = self.columns;
        std::array::from_fn(|k| {
            a::add(a::add(a::add(a::mul(c0[k], p[0]), a::mul(c1[k], p[1])), a::mul(c2[k], p[2])), self.translation[k])
        })
    }
}

/// The inverse transform as the box queries build it: the conjugate
/// rotation and the conjugate-rotated negated translation.
pub(super) fn inverse_matrix(q: [f32; 4], t: V3) -> Matrix34 {
    let inv_q = [a::neg(q[0]), a::neg(q[1]), a::neg(q[2]), q[3]];
    let inv_t = quat_rotate(inv_q, neg(t));
    Matrix34::from_pose(inv_q, inv_t)
}
