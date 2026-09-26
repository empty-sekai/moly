//! The sphere sweep against a box: `PxGeometryQuery::sweep` for a sphere
//! against a `PxBoxGeometry` with the hit flags normal and MTD and zero
//! inflation, as the engine's PhysX 4.1 build runs it. The sphere is a
//! capsule of zero half height; the capsule-box sweep (not the precise one:
//! the flags do not ask for it) takes the capsule into the box's frame and
//! runs the GJK raycast of the capsule's segment against the box with the
//! radius as the inflation. A raycast whose time of impact is zero is an
//! initial overlap: the GJK penetration runs, and for shapes that overlap
//! deeper than its margins the EPA, and the depth comes back as a negative
//! distance with the position pushed out along the normal.
//!
//! The arithmetic is the build's NEON vector library: every step is one
//! binary32 operation and nothing fuses (a multiply-add is a product then a
//! sum); a reciprocal or a reciprocal square root is the estimate refined
//! four times; a vector's square root is zero for zero and otherwise the
//! value times its reciprocal square root; a dot product adds the x and y
//! products, then the z product plus the zero fourth lane. Every vector's
//! fourth lane is zero along these paths (loads, negations, scales and
//! merges clear it, sums and selects of zeros keep it), so the vectors here
//! carry three lanes.
use super::sweep::MeshSweepHit;
use super::vector::{add, cross, fclamp, neg, rsqrt_n, scale, sub, v3dot, v3neg_scale_sub, v3scale_add, Matrix34, EPS};
use super::{arms, finite3, Pose, Refused};
use crate::particle::armf as a;

type V3 = [f32; 3];

const ZERO3: V3 = [0.0; 3];
const UNIT_X: V3 = [1.0, 0.0, 0.0];
/// The PxQueryHit face index the box sweep leaves as it is.
const NO_FACE: u32 = u32::MAX;
/// The EPA's buffers: facets, support points, silhouette edges.
const MAX_FACETS: usize = 64;
const MAX_SUPPORT_POINTS: usize = 64;
const MAX_EDGES: usize = 32;

/// `FRecip`: the estimate and four refinements (three on the mutant).
fn recip(x: f32) -> f32 {
    let mut e = a::recip_estimate(x);
    for _ in 0..if arms::on("boxRecipThree") { 3 } else { 4 } {
        e = a::mul(e, a::recip_step(x, e));
    }
    e
}

/// `FSqrt`.
fn fsqrt(x: f32) -> f32 {
    if x == 0.0 { x } else { a::mul(x, rsqrt_n(x, 4)) }
}

/// `FDiv`: the product with the reciprocal.
fn fdiv(x: f32, y: f32) -> f32 {
    a::mul(x, recip(y))
}

/// `V3Length`.
fn length(v: V3) -> f32 {
    fsqrt(v3dot(v, v))
}

/// `V3ScaleInv`.
fn scale_inv(v: V3, s: f32) -> V3 {
    scale(v, recip(s))
}

/// `V3Normalize`.
fn normalize(v: V3) -> V3 {
    scale_inv(v, length(v))
}

/// `V3NormalizeSafe`.
fn normalize_safe(v: V3, unsafe_value: V3) -> V3 {
    let l = length(v);
    if l > 0.0 { scale_inv(v, l) } else { unsafe_value }
}

/// `QuatRotate`: `((v*(w*w - 1/2) + (u x v)*w) + u*(u.v))*2`.
fn quat_rotate(q: [f32; 4], v: V3) -> V3 {
    let u = [q[0], q[1], q[2]];
    let w2 = a::add(-0.5, a::mul(q[3], q[3]));
    let temp = v3scale_add(cross(u, v), q[3], scale(v, w2));
    scale(v3scale_add(u, v3dot(u, v), temp), 2.0)
}

/// `QuatRotateInv`: the cross term subtracted.
fn quat_rotate_inv(q: [f32; 4], v: V3) -> V3 {
    let u = [q[0], q[1], q[2]];
    let w2 = a::add(-0.5, a::mul(q[3], q[3]));
    let temp = v3neg_scale_sub(cross(u, v), q[3], scale(v, w2));
    scale(v3scale_add(u, v3dot(u, v), temp), 2.0)
}

/// `QuatTransform`: `p + rotated*2` with the doubling folded into the sum.
fn quat_transform(q: [f32; 4], p: V3, v: V3) -> V3 {
    let u = [q[0], q[1], q[2]];
    let w2 = a::add(-0.5, a::mul(q[3], q[3]));
    let temp = v3scale_add(cross(u, v), q[3], scale(v, w2));
    v3scale_add(v3scale_add(u, v3dot(u, v), temp), 2.0, p)
}

/// `QuatMul`.
fn quat_mul(x: [f32; 4], y: [f32; 4]) -> [f32; 4] {
    let (ix, iy) = ([x[0], x[1], x[2]], [y[0], y[1], y[2]]);
    let real = a::sub(a::mul(x[3], y[3]), v3dot(ix, iy));
    let imag = add(add(scale(ix, y[3]), scale(iy, x[3])), cross(ix, iy));
    [imag[0], imag[1], imag[2], real]
}

/// `QuatGetMat33V`: the three columns.
fn quat_columns(q: [f32; 4]) -> [V3; 3] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (a::add(x, x), a::add(y, y), a::add(z, z));
    let (xx, yy, zz) = (a::mul(x2, x), a::mul(y2, y), a::mul(z2, z));
    let (xy, xz, xw) = (a::mul(x2, y), a::mul(x2, z), a::mul(x2, w));
    let (yz, yw, zw) = (a::mul(y2, z), a::mul(y2, w), a::mul(z2, w));
    let v = a::sub(1.0, xx);
    [
        [a::sub(a::sub(1.0, yy), zz), a::add(xy, zw), a::sub(xz, yw)],
        [a::sub(xy, zw), a::sub(v, zz), a::add(yz, xw)],
        [a::add(xz, yw), a::sub(yz, xw), a::sub(v, yy)],
    ]
}

/// `M33MulV3`.
fn columns_mul(c: &[V3; 3], v: V3) -> V3 {
    add(add(scale(c[0], v[0]), scale(c[1], v[1])), scale(c[2], v[2]))
}

/// The sphere as the capsule-box sweep builds it, in the box's frame: both
/// segment ends at the centre (plus and minus the rotated zero half axis),
/// the radius as its margin and minimum margin, and a sweep margin of zero.
struct Capsule {
    p0: V3,
    p1: V3,
    radius: f32,
}

impl Capsule {
    fn support(&self, dir: V3) -> V3 {
        self.support_index(dir).0
    }

    fn support_index(&self, dir: V3) -> (V3, i32) {
        if v3dot(self.p0, dir) > v3dot(self.p1, dir) { (self.p0, 1) } else { (self.p1, 0) }
    }

    fn support_point(&self, index: i32) -> V3 {
        if index == 1 { self.p0 } else { self.p1 }
    }

    fn sweep_margin(&self) -> f32 {
        if arms::on("boxCapsuleSweepMargin") { self.radius } else { 0.0 }
    }
}

/// The box at the origin of its frame: half extents and the margins the
/// box constructor derives from the smallest one.
struct BoxShape {
    extents: V3,
    margin: f32,
    min_margin: f32,
    sweep_margin: f32,
}

impl BoxShape {
    fn new(extents: V3) -> Self {
        let min = a::min(extents[2], a::min(extents[0], extents[1]));
        Self { extents, margin: a::mul(min, 0.15), min_margin: a::mul(min, 0.05), sweep_margin: a::mul(min, 0.05) }
    }

    fn support(&self, dir: V3) -> V3 {
        self.support_index(dir).0
    }

    fn support_index(&self, dir: V3) -> (V3, i32) {
        let mut index = 0;
        let p = std::array::from_fn(|k| {
            if dir[k] > 0.0 {
                index |= 1 << k;
                self.extents[k]
            } else {
                a::neg(self.extents[k])
            }
        });
        (p, index)
    }

    fn support_point(&self, index: i32) -> V3 {
        std::array::from_fn(|k| if index & (1 << k) != 0 { self.extents[k] } else { a::neg(self.extents[k]) })
    }
}

/// A GJK simplex: the Minkowski points, the points on each shape and their
/// support indices, and the count.
#[derive(Clone, Copy)]
struct Simplex {
    q: [V3; 4],
    a: [V3; 4],
    b: [V3; 4],
    ai: [i32; 4],
    bi: [i32; 4],
    size: usize,
}

impl Simplex {
    fn empty() -> Self {
        Self { q: [ZERO3; 4], a: [ZERO3; 4], b: [ZERO3; 4], ai: [0; 4], bi: [0; 4], size: 0 }
    }

    /// Keeps the listed points, in order, at the front.
    fn keep(&mut self, indices: &[usize]) {
        let old = *self;
        for (to, &from) in indices.iter().enumerate() {
            self.q[to] = old.q[from];
            self.a[to] = old.a[from];
            self.b[to] = old.b[from];
            self.ai[to] = old.ai[from];
            self.bi[to] = old.bi[from];
        }
    }

    fn push(&mut self, pa: V3, pb: V3, q: V3) -> Result<(), Refused> {
        if self.size >= 4 {
            return Err(Refused("a GJK simplex past four points"));
        }
        self.a[self.size] = pa;
        self.b[self.size] = pb;
        self.q[self.size] = q;
        self.size += 1;
        Ok(())
    }
}

/// `closestPtPointSegment`: a degenerate segment keeps its first point.
fn closest_segment(s: &mut Simplex) -> V3 {
    let (p, r) = (s.q[0], s.q[1]);
    let ab = sub(r, p);
    let denom = v3dot(ab, ab);
    let nom = v3dot(neg(p), ab);
    if EPS >= denom {
        s.size = 1;
        return s.q[0];
    }
    v3scale_add(ab, fclamp(fdiv(nom, denom), 0.0, 1.0), p)
}

/// The reciprocal of an edge's parameter denominator, zero when it is no
/// larger than the machine epsilon.
fn edge_recip(x: f32) -> f32 {
    if a::abs(x) > EPS { recip(x) } else { 0.0 }
}

/// `closestPtPointTriangleBaryCentric`: the squared distance of the closest
/// point of triangle abc to the origin (the largest float for a triangle of
/// zero area, the point then not written), with the kept corners' indices.
fn closest_triangle_barycentric(p: V3, q: V3, r: V3, indices: &mut [usize; 3], size: &mut usize, closest: &mut V3)
    -> f32 {
    *size = 3;
    let ab = sub(q, p);
    let ac = sub(r, p);
    let n = cross(ab, ac);
    let nn = v3dot(n, n);
    if nn == 0.0 {
        return f32::MAX;
    }
    let va = v3dot(n, cross(q, r));
    let vb = v3dot(n, cross(r, p));
    let vc = v3dot(n, cross(p, q));
    if va >= 0.0 && vb >= 0.0 && vc >= 0.0 {
        let t = fdiv(v3dot(n, p), nn);
        let pt = scale(n, t);
        *closest = pt;
        return v3dot(pt, pt);
    }
    let (ap, bp, cp) = (neg(p), neg(q), neg(r));
    let d1 = v3dot(ab, ap);
    let d2 = v3dot(ac, ap);
    let d3 = v3dot(ab, bp);
    let d4 = v3dot(ac, bp);
    let d5 = v3dot(ab, cp);
    let d6 = v3dot(ac, cp);
    let unom = a::sub(d4, d3);
    let udenom = a::sub(d5, d6);
    *size = 2;
    if 0.0 >= vc && d1 >= 0.0 && 0.0 >= d3 {
        let t = a::mul(d1, edge_recip(a::sub(d1, d3)));
        let pt = v3scale_add(ab, t, p);
        *closest = pt;
        return v3dot(pt, pt);
    }
    if 0.0 >= va && d4 >= d3 && d5 >= d6 {
        let bc = sub(r, q);
        let t = a::mul(unom, edge_recip(a::add(unom, udenom)));
        indices[0] = indices[1];
        indices[1] = indices[2];
        let pt = v3scale_add(bc, t, q);
        *closest = pt;
        return v3dot(pt, pt);
    }
    if 0.0 >= vb && d2 >= 0.0 && 0.0 >= d6 {
        let t = a::mul(d2, edge_recip(a::sub(d2, d6)));
        indices[1] = indices[2];
        let pt = v3scale_add(ac, t, p);
        *closest = pt;
        return v3dot(pt, pt);
    }
    *size = 1;
    if 0.0 >= d1 && 0.0 >= d2 {
        *closest = p;
        return v3dot(p, p);
    }
    if d3 >= 0.0 && d3 >= d4 {
        indices[0] = indices[1];
        *closest = q;
        return v3dot(q, q);
    }
    indices[0] = indices[2];
    *closest = r;
    v3dot(r, r)
}

/// `closestPtPointTriangle`.
fn closest_triangle(s: &mut Simplex) -> V3 {
    s.size = 3;
    let (p, q, r) = (s.q[0], s.q[1], s.q[2]);
    let area = cross(sub(q, p), sub(r, p));
    if EPS >= v3dot(area, area) {
        s.size = 2;
        return closest_segment(s);
    }
    let mut size = 3;
    let mut indices = [0, 1, 2];
    let mut closest = ZERO3;
    closest_triangle_barycentric(p, q, r, &mut indices, &mut size, &mut closest);
    if size != 3 {
        s.keep(&indices[..2]);
        s.size = size;
    }
    closest
}

/// `PointOutsideOfPlane4`: per face of the tetrahedron, whether the origin
/// is outside it: the products of the face plane's value at a corner of the
/// face and at the opposite corner are compared with zero. This build
/// compares with zero; the published source's allowance of -1e-6 is not in
/// the engine's code.
fn outside_of_planes(p: V3, q: V3, r: V3, d: V3) -> [bool; 4] {
    let ab = sub(q, p);
    let ac = sub(r, p);
    let ad = sub(d, p);
    let bd = sub(d, q);
    let bc = sub(r, q);
    let v0 = cross(ab, ac);
    let v1 = cross(ac, ad);
    let v2 = cross(ad, ab);
    let v3 = cross(bd, bc);
    let signa = [v3dot(v0, p), v3dot(v1, p), v3dot(v2, p), v3dot(v3, q)];
    let signd = [v3dot(v0, d), v3dot(v1, q), v3dot(v2, r), v3dot(v3, p)];
    let floor = if arms::on("boxOutsideAllowance") { -1e-6 } else { 0.0 };
    std::array::from_fn(|k| a::mul(signa[k], signd[k]) >= floor)
}

/// `getClosestPtPointTriangle` over the faces the origin is outside of.
fn closest_face(q: &[V3; 4], outside: [bool; 4], indices: &mut [usize; 3], size: &mut usize) -> V3 {
    let mut best = f32::MAX;
    let mut closest = ZERO3;
    if outside[0] {
        best = closest_triangle_barycentric(q[0], q[1], q[2], indices, size, &mut closest);
    }
    for (lane, corners) in [(1, [0, 2, 3]), (2, [0, 3, 1]), (3, [1, 3, 2])] {
        if !outside[lane] {
            continue;
        }
        let mut face_size = 3;
        let mut face = corners;
        let mut point = ZERO3;
        let sq = closest_triangle_barycentric(q[corners[0]], q[corners[1]], q[corners[2]], &mut face, &mut face_size,
            &mut point);
        if best > sq {
            closest = point;
            best = sq;
            *indices = face;
            *size = face_size;
        }
    }
    closest
}

/// `closestPtPointTetrahedron`: a flat tetrahedron is its first triangle;
/// with the origin inside the simplex stays whole and the point is zero.
fn closest_tetrahedron(s: &mut Simplex) -> V3 {
    let (p, q, r, d) = (s.q[0], s.q[1], s.q[2], s.q[3]);
    let n = normalize(cross(sub(q, p), sub(r, p)));
    let sign_dist = v3dot(n, sub(d, p));
    if 1e-4 > a::abs(sign_dist) {
        s.size = 3;
        return closest_triangle(s);
    }
    let outside = outside_of_planes(p, q, r, d);
    if outside.iter().all(|&o| !o) {
        return ZERO3;
    }
    let mut indices = [0, 1, 2];
    let mut size = s.size;
    let closest = closest_face(&s.q, outside, &mut indices, &mut size);
    s.size = size;
    s.keep(&indices);
    closest
}

/// `GJKCPairDoSimplex`.
fn do_simplex(s: &mut Simplex, support: V3) -> V3 {
    match s.size {
        2 => closest_segment(s),
        3 => closest_triangle(s),
        4 => closest_tetrahedron(s),
        _ => support,
    }
}

/// `barycentricCoordinates` of a point on a segment.
fn barycentric2(p: V3, q: V3, r: V3) -> f32 {
    let v0 = sub(q, p);
    let v1 = sub(r, p);
    let d = sub(v1, v0);
    let denominator = v3dot(d, d);
    let numerator = v3dot(neg(v0), d);
    a::mul(numerator, if denominator > 0.0 { recip(denominator) } else { 0.0 })
}

/// `barycentricCoordinates` of a point in a triangle.
fn barycentric3(p: V3, q: V3, r: V3, t: V3) -> (f32, f32) {
    let n = cross(sub(r, q), sub(t, q));
    let (ca, cb, cc) = (sub(q, p), sub(r, p), sub(t, p));
    let va = v3dot(n, cross(cb, cc));
    let vb = v3dot(n, cross(cc, ca));
    let vc = v3dot(n, cross(ca, cb));
    let total = a::add(va, a::add(vb, vc));
    let denom = if total == 0.0 { 0.0 } else { recip(total) };
    (a::mul(vb, denom), a::mul(vc, denom))
}

/// `getClosestPoint`: the points on each shape for the simplex's closest
/// point; a four-point simplex leaves both at zero.
fn closest_points(s: &Simplex, closest: V3) -> (V3, V3) {
    match s.size {
        1 => (s.a[0], s.b[0]),
        2 => {
            let v = barycentric2(closest, s.q[0], s.q[1]);
            (v3scale_add(sub(s.a[1], s.a[0]), v, s.a[0]), v3scale_add(sub(s.b[1], s.b[0]), v, s.b[0]))
        }
        3 => {
            let (v, w) = barycentric3(closest, s.q[0], s.q[1], s.q[2]);
            let on = |p: &[V3; 4]| add(p[0], add(scale(sub(p[1], p[0]), v), scale(sub(p[2], p[0]), w)));
            (on(&s.a), on(&s.b))
        }
        _ => (ZERO3, ZERO3),
    }
}

/// What a GJK or EPA reports; zero until written.
#[derive(Clone, Copy, Default)]
struct Output {
    closest_a: V3,
    normal: V3,
    pen_dep: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    NonIntersect,
    Contact,
    Degenerate,
    EpaContact,
    EpaDegenerate,
    EpaFail,
}

/// `gjkRaycast` of the capsule against the box along `r` from `s`: the
/// time of impact, the normal and the closest point on the capsule's
/// surface, or no hit.
fn gjk_raycast(cap: &Capsule, bx: &BoxShape, initial_dir: V3, s: V3, r: V3, inflation: f32)
    -> Result<Option<(f32, V3, V3)>, Refused> {
    let mut lambda = 0.0;
    let mut x = v3scale_add(r, lambda, s);
    let search = normalize(if v3dot(initial_dir, initial_dir) > EPS { initial_dir } else { UNIT_X });
    let initial_a = cap.support(neg(search));
    let initial_b = bx.support(search);
    let mut sx = Simplex::empty();
    sx.q[0] = sub(initial_a, initial_b);
    sx.a[0] = initial_a;
    sx.b[0] = initial_b;
    sx.size = 1;
    let mut v = neg(sx.q[0]);
    let min_margin = a::min(cap.sweep_margin(), bx.sweep_margin);
    let eps1 = a::mul(min_margin, 0.1);
    let inflation_plus_eps = a::add(eps1, inflation);
    let eps2 = a::mul(eps1, eps1);
    let inflation2 = a::mul(inflation_plus_eps, inflation_plus_eps);
    let mut clos = sx.q[0];
    let mut pre_clos = clos;
    let mut s_dist = v3dot(v, v);
    let mut not_terminated = s_dist > eps2;
    let mut not_degenerated = true;
    let mut nor = v;
    while not_terminated {
        let mut min_dist = s_dist;
        pre_clos = clos;
        let v_norm = normalize(v);
        let nv_norm = neg(v_norm);
        let support_a = cap.support(v_norm);
        let mut support_b = add(x, bx.support(nv_norm));
        let mut support = sub(support_a, support_b);
        let vw = a::sub(v3dot(v_norm, neg(support)), inflation_plus_eps);
        if vw > 0.0 {
            let vr = v3dot(v_norm, r);
            if vr >= 0.0 {
                return Ok(None);
            }
            let old = lambda;
            lambda = a::sub(lambda, fdiv(vw, vr));
            if lambda > old {
                if lambda > 1.0 {
                    return Ok(None);
                }
                let before = x;
                x = v3scale_add(r, lambda, s);
                let offset = sub(x, before);
                for k in 0..3 {
                    sx.b[k] = add(sx.b[k], offset);
                    sx.q[k] = sub(sx.a[k], sx.b[k]);
                }
                support_b = add(x, bx.support(nv_norm));
                support = sub(support_a, support_b);
                min_dist = f32::MAX;
                nor = v;
            }
        }
        sx.push(support_a, support_b, support)?;
        clos = do_simplex(&mut sx, support);
        v = neg(clos);
        s_dist = v3dot(clos, clos);
        not_degenerated = min_dist > s_dist;
        not_terminated = s_dist > inflation2 && not_degenerated;
    }
    if !(s_dist > eps2 && not_degenerated) {
        v = nor;
    }
    let normal = neg(normalize_safe(v, ZERO3));
    let (closest_a, _) = closest_points(&sx, if not_degenerated { clos } else { pre_clos });
    Ok(Some((lambda, normal, v3neg_scale_sub(normal, cap.radius, closest_a))))
}

/// The indices of the simplex GJK leaves for the EPA to start from.
#[derive(Default)]
struct WarmStart {
    ai: [i32; 4],
    bi: [i32; 4],
    size: usize,
}

impl WarmStart {
    fn assign(&mut self, sx: &Simplex, size: usize) {
        self.ai = sx.ai;
        self.bi = sx.bi;
        self.size = size;
    }
}

/// `gjkPenetration` of the capsule and the box without a warm start, with
/// the surface points (the core shapes on the mutant).
fn gjk_penetration(cap: &Capsule, bx: &BoxShape, initial_dir: V3, contact_dist: f32, warm: &mut WarmStart)
    -> Result<(Status, Output), Refused> {
    let take_core = arms::on("boxPenetrationCoreShape");
    let mut out = Output::default();
    let eps = a::mul(a::min(cap.radius, bx.min_margin), 0.1);
    let rel_dif = a::sub(1.0, 0.000225);
    // The capsule's margin is its radius and it is quadratic; the box is not.
    let (t_margin_a, t_margin_b) = (cap.radius, 0.0);
    let sum_margin = a::add(t_margin_a, t_margin_b);
    let sum_expanded = a::add(sum_margin, contact_dist);
    let mut dist = f32::MAX;
    let mut prev_dist = dist;
    let mut prev_clos = ZERO3;
    let mut not_terminated = true;
    let mut not_degenerated = true;
    let mut sx = Simplex::empty();
    let mut closest = if v3dot(initial_dir, initial_dir) > 0.0 { initial_dir } else { UNIT_X };
    let mut v = normalize(closest);
    while not_terminated {
        prev_dist = dist;
        prev_clos = closest;
        if sx.size >= 4 {
            return Err(Refused("a GJK simplex past four points"));
        }
        let (support_a, ia) = cap.support_index(neg(closest));
        let (support_b, ib) = bx.support_index(closest);
        sx.ai[sx.size] = ia;
        sx.bi[sx.size] = ib;
        let support = sub(support_a, support_b);
        let vw = v3dot(v, support);
        if vw > sum_expanded {
            warm.assign(&sx, sx.size);
            return Ok((Status::NonIntersect, out));
        }
        if vw > a::mul(dist, rel_dif) {
            warm.assign(&sx, sx.size);
            out.normal = v;
            let (ca, _) = closest_points(&sx, closest);
            if take_core {
                out.closest_a = ca;
                out.pen_dep = dist;
            } else {
                out.closest_a = v3neg_scale_sub(v, t_margin_a, ca);
                out.pen_dep = a::sub(dist, sum_margin);
            }
            return Ok((Status::Contact, out));
        }
        sx.push(support_a, support_b, support)?;
        closest = do_simplex(&mut sx, support);
        dist = length(closest);
        v = scale_inv(closest, dist);
        not_degenerated = prev_dist > dist;
        not_terminated = dist > eps && not_degenerated;
    }
    if !not_degenerated {
        warm.assign(&sx, sx.size.saturating_sub(1));
        dist = prev_dist;
        let (ca, _) = closest_points(&sx, prev_clos);
        let n = scale_inv(prev_clos, prev_dist);
        out.normal = n;
        if take_core {
            out.closest_a = ca;
            out.pen_dep = dist;
        } else {
            out.closest_a = v3neg_scale_sub(n, t_margin_a, ca);
            out.pen_dep = a::sub(dist, sum_margin);
            if sum_margin >= dist {
                return Ok((Status::Contact, out));
            }
        }
        return Ok((Status::Degenerate, out));
    }
    warm.assign(&sx, sx.size);
    Ok((Status::EpaContact, out))
}

/// One EPA facet: its corners (indices into the support buffers), plane,
/// neighbours across each edge and the edge index there, and its flags.
#[derive(Clone, Copy)]
struct Facet {
    normal: V3,
    dist: f32,
    adj: [usize; 3],
    adj_edges: [i8; 3],
    indices: [u8; 3],
    obsolete: bool,
    in_heap: bool,
    id: usize,
}

const NO_FACET: usize = usize::MAX;

impl Facet {
    fn blank() -> Self {
        Self { normal: ZERO3, dist: 0.0, adj: [NO_FACET; 3], adj_edges: [-1; 3], indices: [0; 3], obsolete: false,
            in_heap: false, id: 0 }
    }
}

/// The facet id pool with deferred frees (64 ids).
#[derive(Default)]
struct IdPool {
    current: usize,
    free: Vec<usize>,
    deferred: Vec<usize>,
}

impl IdPool {
    fn new_id(&mut self) -> usize {
        match self.free.pop() {
            Some(id) => id,
            None => {
                self.current += 1;
                self.current - 1
            }
        }
    }

    fn free_id(&mut self, id: usize) -> Result<(), Refused> {
        if self.current > 0 && id == self.current - 1 {
            self.current -= 1;
        } else {
            if self.free.len() >= MAX_FACETS {
                return Err(Refused("the EPA facet pool past its capacity"));
            }
            self.free.push(id);
        }
        Ok(())
    }

    fn process_deferred(&mut self) -> Result<(), Refused> {
        for id in std::mem::take(&mut self.deferred) {
            self.free_id(id)?;
        }
        Ok(())
    }

    fn remaining(&self) -> usize {
        MAX_FACETS - (self.current - self.free.len())
    }
}

/// The EPA's state for one penetration query.
struct Epa<'s> {
    cap: &'s Capsule,
    bx: &'s BoxShape,
    heap: Vec<usize>,
    abuf: [V3; MAX_SUPPORT_POINTS],
    bbuf: [V3; MAX_SUPPORT_POINTS],
    facets: [Facet; MAX_FACETS],
    edges: Vec<(usize, usize)>,
    overflow: bool,
    pool: IdPool,
}

fn inc_mod3(i: usize) -> usize {
    [1, 2, 0][i]
}

impl<'s> Epa<'s> {
    fn less(&self, x: usize, y: usize) -> bool {
        self.facets[x].dist < self.facets[y].dist
    }

    fn push(&mut self, id: usize) {
        let mut at = self.heap.len();
        self.heap.push(id);
        while at > 0 {
            let parent = (at - 1) >> 1;
            if !self.less(id, self.heap[parent]) {
                break;
            }
            self.heap[at] = self.heap[parent];
            at = parent;
        }
        self.heap[at] = id;
    }

    fn pop(&mut self) -> usize {
        let last_at = self.heap.len() - 1;
        let min = self.heap[0];
        let last = self.heap[last_at];
        let mut i = 0;
        loop {
            let mut child = 2 * i + 1;
            if child >= last_at {
                break;
            }
            if child + 1 < last_at && self.less(self.heap[child + 1], self.heap[child]) {
                child += 1;
            }
            if self.less(last, self.heap[child]) {
                break;
            }
            self.heap[i] = self.heap[child];
            i = child;
        }
        self.heap[i] = last;
        self.heap.truncate(last_at);
        min
    }

    fn point(&self, i: u8) -> V3 {
        sub(self.abuf[i as usize], self.bbuf[i as usize])
    }

    /// `Facet::isValid2`: sets the plane; a facet of non-degenerate area
    /// whose plane is within the upper bound goes into the heap.
    fn add_facet(&mut self, i0: usize, i1: usize, i2: usize, upper: f32) -> Result<usize, Refused> {
        if self.pool.current - self.pool.free.len() >= MAX_FACETS {
            return Err(Refused("the EPA past its facet capacity"));
        }
        let id = self.pool.new_id();
        let mut f = Facet::blank();
        f.indices = [i0 as u8, i1 as u8, i2 as u8];
        f.id = id;
        let p0 = self.point(f.indices[0]);
        let n = cross(sub(self.point(f.indices[1]), p0), sub(self.point(f.indices[2]), p0));
        let nn = v3dot(n, n);
        let valid_area = nn > EPS;
        let normal = scale(n, rsqrt_n(if valid_area { nn } else { 1.0 }, 4));
        let dist = v3dot(normal, p0);
        f.normal = normal;
        f.dist = dist;
        let valid = valid_area && upper >= dist;
        f.in_heap = valid;
        self.facets[id] = f;
        if valid {
            if self.heap.len() >= MAX_FACETS {
                return Err(Refused("the EPA heap past its capacity"));
            }
            self.push(id);
        }
        Ok(id)
    }

    fn link(&mut self, f: usize, edge0: usize, g: usize, edge1: usize) {
        self.facets[f].adj[edge0] = g;
        self.facets[f].adj_edges[edge0] = edge1 as i8;
        self.facets[g].adj[edge1] = f;
        self.facets[g].adj_edges[edge1] = edge0 as i8;
    }

    fn support(&self, dir: V3) -> (V3, V3, V3) {
        let pa = self.cap.support(neg(dir));
        let pb = self.bx.support(dir);
        (pa, pb, sub(pa, pb))
    }

    fn expand_point(&mut self, upper: f32) -> Result<bool, Refused> {
        let q0 = self.point(0);
        let (pa, pb, q1) = self.support(UNIT_X);
        self.abuf[1] = pa;
        self.bbuf[1] = pb;
        if (0..3).all(|k| q0[k] == q1[k]) {
            return Ok(false);
        }
        self.expand_segment(upper)
    }

    fn expand_segment(&mut self, upper: f32) -> Result<bool, Refused> {
        let v = sub(self.point(1), self.point(0));
        let [x, y, z] = v.map(a::abs);
        let axis = if x > y && z > y {
            [0.0, 1.0, 0.0]
        } else if x > z {
            [0.0, 0.0, 1.0]
        } else {
            UNIT_X
        };
        let n = normalize(cross(axis, v));
        let (pa, pb, _) = self.support(n);
        self.abuf[2] = pa;
        self.bbuf[2] = pb;
        self.expand_triangle(upper)
    }

    fn expand_triangle(&mut self, upper: f32) -> Result<bool, Refused> {
        let f0 = self.add_facet(0, 1, 2, upper)?;
        let f1 = self.add_facet(1, 0, 2, upper)?;
        if self.heap.is_empty() {
            return Ok(false);
        }
        self.link(f0, 0, f1, 0);
        self.link(f0, 1, f1, 2);
        self.link(f0, 2, f1, 1);
        Ok(true)
    }

    /// `Facet::silhouette` from one neighbour across one edge.
    fn silhouette_from(&mut self, start: usize, start_edge: usize, w: V3) -> Result<(), Refused> {
        let mut stack = vec![(start, start_edge)];
        while let Some((f, index)) = stack.pop() {
            if f == NO_FACET {
                return Err(Refused("an EPA facet without a neighbour"));
            }
            if self.facets[f].obsolete {
                continue;
            }
            let p0 = self.point(self.facets[f].indices[0]);
            let d = v3dot(self.facets[f].normal, sub(w, p0));
            if 0.0 > d {
                if self.edges.len() < MAX_EDGES {
                    self.edges.push((f, index));
                } else {
                    self.overflow = true;
                    return Ok(());
                }
            } else {
                self.facets[f].obsolete = true;
                let next = inc_mod3(index);
                let next2 = inc_mod3(next);
                let facet = self.facets[f];
                if stack.len() + 2 > MAX_FACETS {
                    return Err(Refused("the EPA silhouette stack past its capacity"));
                }
                stack.push((facet.adj[next2], facet.adj_edges[next2] as usize));
                stack.push((facet.adj[next], facet.adj_edges[next] as usize));
                if !facet.in_heap {
                    self.pool.deferred.push(facet.id);
                }
            }
        }
        Ok(())
    }

    /// `calculateContactInformation` with the surface points.
    fn contact(&self, f: usize) -> Output {
        let facet = &self.facets[f];
        let [i0, i1, i2] = facet.indices.map(usize::from);
        let (pa0, pa1, pa2) = (self.abuf[i0], self.abuf[i1], self.abuf[i2]);
        let (pb0, pb1, pb2) = (self.bbuf[i0], self.bbuf[i1], self.bbuf[i2]);
        let p0 = sub(pa0, pb0);
        let v0 = sub(sub(pa1, pb1), p0);
        let v1 = sub(sub(pa2, pb2), p0);
        let v2 = sub(scale(facet.normal, facet.dist), p0);
        let g00 = v3dot(v0, v0);
        let g01 = v3dot(v0, v1);
        let g11 = v3dot(v1, v1);
        let g20 = v3dot(v2, v0);
        let g21 = v3dot(v2, v1);
        let det = a::sub(a::mul(g00, g11), a::mul(g01, g01));
        let rcp = if det > EPS { recip(det) } else { 0.0 };
        let lambda1 = a::mul(a::sub(a::mul(g11, g20), a::mul(g01, g21)), rcp);
        let lambda2 = a::mul(a::sub(a::mul(g00, g21), a::mul(g01, g20)), rcp);
        let u = a::sub(1.0, a::add(lambda1, lambda2));
        let pa = v3scale_add(pa0, u, v3scale_add(pa1, lambda1, scale(pa2, lambda2)));
        let dist = a::abs(facet.dist);
        let normal = neg(facet.normal);
        // The capsule is quadratic with its radius as the margin; the box
        // adds nothing.
        let (margin_a, margin_b) = (self.cap.radius, 0.0);
        Output { closest_a: v3neg_scale_sub(normal, margin_a, pa), normal,
            pen_dep: a::neg(a::add(dist, a::add(margin_a, margin_b))) }
    }

    /// `EPA::PenetrationDepth` with the surface points and no degenerate
    /// check (both apply only to the core shapes).
    fn penetration(&mut self, a0: &[V3; 4], b0: &[V3; 4], size: usize) -> Result<(Status, Output), Refused> {
        let mut upper = f32::MAX;
        self.abuf[..4].copy_from_slice(a0);
        self.bbuf[..4].copy_from_slice(b0);
        let mut num = 0usize;
        match size {
            1 => {
                num = 3;
                if !self.expand_point(upper)? {
                    return Ok((Status::EpaFail, Output::default()));
                }
            }
            2 => {
                num = 3;
                if !self.expand_segment(upper)? {
                    return Ok((Status::EpaFail, Output::default()));
                }
            }
            3 => {
                num = 3;
                if !self.expand_triangle(upper)? {
                    return Ok((Status::EpaFail, Output::default()));
                }
            }
            4 => {
                let p0 = self.point(0);
                let n = normalize(cross(sub(self.point(1), p0), sub(self.point(2), p0)));
                if v3dot(n, sub(self.point(3), p0)) > 0.0 {
                    self.abuf.swap(1, 2);
                    self.bbuf.swap(1, 2);
                }
                let f0 = self.add_facet(0, 1, 2, upper)?;
                let f1 = self.add_facet(0, 3, 1, upper)?;
                let f2 = self.add_facet(0, 2, 3, upper)?;
                let f3 = self.add_facet(1, 3, 2, upper)?;
                if self.heap.is_empty() {
                    return Ok((Status::EpaFail, Output::default()));
                }
                self.link(f0, 0, f1, 2);
                self.link(f0, 1, f3, 2);
                self.link(f0, 2, f2, 0);
                self.link(f1, 0, f2, 2);
                self.link(f1, 1, f3, 0);
                self.link(f2, 1, f3, 1);
                num = 4;
            }
            _ => return Err(Refused("an EPA start outside one to four points")),
        }
        // expandPoint and expandSegment set the count through
        // expandTriangle.
        let eps = a::mul(a::min(self.cap.radius, self.bx.min_margin), 0.1);
        let mut facet;
        loop {
            self.pool.process_deferred()?;
            facet = self.pop();
            self.facets[facet].in_heap = false;
            if !self.facets[facet].obsolete {
                let f = self.facets[facet];
                let tempa = self.cap.support(f.normal);
                let tempb = self.bx.support(neg(f.normal));
                let q = sub(tempa, tempb);
                let dist = v3dot(q, f.normal);
                if eps >= a::abs(a::sub(dist, f.dist)) {
                    return Ok((Status::EpaContact, self.contact(facet)));
                }
                upper = a::min(upper, dist);
                if num >= MAX_SUPPORT_POINTS {
                    return Err(Refused("the EPA past its support points"));
                }
                self.abuf[num] = tempa;
                self.bbuf[num] = tempb;
                let index = num;
                num += 1;
                self.edges.clear();
                self.overflow = false;
                self.facets[facet].obsolete = true;
                for e in 0..3 {
                    let (g, ge) = (self.facets[facet].adj[e], self.facets[facet].adj_edges[e]);
                    self.silhouette_from(g, ge as usize, q)?;
                }
                if self.edges.is_empty() || self.overflow {
                    return Ok((Status::EpaDegenerate, self.contact(facet)));
                }
                if self.edges.len() > self.pool.remaining() {
                    return Ok((Status::EpaDegenerate, self.contact(facet)));
                }
                let edges = self.edges.clone();
                let ends = |this: &Self, (f, i): (usize, usize)| {
                    let corners = this.facets[f].indices;
                    (usize::from(corners[inc_mod3(i)]), usize::from(corners[i]))
                };
                let (target, source) = ends(self, edges[0]);
                let first = self.add_facet(target, source, index, upper)?;
                self.link(first, 0, edges[0].0, edges[0].1);
                let mut last = first;
                for &edge in &edges[1..] {
                    let (target, source) = ends(self, edge);
                    let nf = self.add_facet(target, source, index, upper)?;
                    self.link(nf, 0, edge.0, edge.1);
                    self.link(nf, 2, last, 1);
                    last = nf;
                }
                self.link(first, 2, last, 1);
            }
            let id = self.facets[facet].id;
            self.pool.free_id(id)?;
            let more = !self.heap.is_empty() && upper > self.facets[self.heap[0]].dist && num != MAX_SUPPORT_POINTS;
            if !more {
                break;
            }
        }
        Ok((Status::EpaDegenerate, self.contact(facet)))
    }
}

/// `epaPenetration` from the GJK's warm start, with the surface points.
fn epa_penetration(cap: &Capsule, bx: &BoxShape, warm: &WarmStart) -> Result<(Status, Output), Refused> {
    if !(1..=4).contains(&warm.size) {
        return Err(Refused("an EPA start outside one to four points"));
    }
    let mut a0 = [ZERO3; 4];
    let mut b0 = [ZERO3; 4];
    for i in 0..warm.size {
        a0[i] = cap.support_point(warm.ai[i]);
        b0[i] = bx.support_point(warm.bi[i]);
    }
    let mut epa = Epa { cap, bx, heap: Vec::with_capacity(MAX_FACETS), abuf: [ZERO3; MAX_SUPPORT_POINTS],
        bbuf: [ZERO3; MAX_SUPPORT_POINTS], facets: [Facet::blank(); MAX_FACETS], edges: Vec::with_capacity(MAX_EDGES),
        overflow: false, pool: IdPool::default() };
    epa.penetration(&a0, &b0, warm.size)
}

/// The native calls a box sweep makes, in order, for the comparison with
/// the engine's traces: the capsule-box raycast-penetration entry, the GJK
/// raycast, the GJK penetration and the EPA.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxSweepTrace {
    pub calls: Vec<&'static str>,
}

/// `gjkRaycastPenetration` with the initial-overlap resolution on: the
/// time of impact (zero or the negative depth for an overlap), the normal
/// and the closest point on the capsule, all in the box's frame.
fn raycast_penetration(cap: &Capsule, bx: &BoxShape, initial_dir: V3, r: V3, inflation: f32,
    trace: &mut Option<&mut BoxSweepTrace>) -> Result<Option<(f32, V3, V3)>, Refused> {
    let mut note = |call: &'static str| {
        if let Some(t) = trace.as_deref_mut() {
            t.calls.push(call);
        }
    };
    note("setup");
    note("raycast");
    let Some((lambda, mut norm, mut clos_a)) = gjk_raycast(cap, bx, initial_dir, ZERO3, r, inflation)? else {
        return Ok(None);
    };
    let mut toi = lambda;
    if lambda == 0.0 {
        // The sweep contact distance: a hundred times the summed margins.
        let contact_dist = a::mul(a::add(cap.radius, bx.margin), 100.0);
        let mut warm = WarmStart::default();
        note("penetration");
        let (status, out) = gjk_penetration(cap, bx, initial_dir, contact_dist, &mut warm)?;
        let s_dist;
        match status {
            Status::Contact => {
                clos_a = out.closest_a;
                s_dist = out.pen_dep;
                norm = out.normal;
            }
            Status::EpaContact => {
                note("epa");
                let (status, out) = if arms::on("boxSkipEpa") {
                    (Status::EpaFail, Output::default())
                } else {
                    epa_penetration(cap, bx, &warm)?
                };
                if matches!(status, Status::EpaContact | Status::EpaDegenerate) {
                    clos_a = out.closest_a;
                    s_dist = out.pen_dep;
                    norm = out.normal;
                } else {
                    clos_a = ZERO3;
                    s_dist = 0.0;
                    norm = normalize(neg(r));
                }
            }
            _ => {
                clos_a = out.closest_a;
                s_dist = out.pen_dep;
                norm = out.normal;
            }
        }
        toi = a::min(0.0, s_dist);
    }
    Ok(Some((toi, norm, clos_a)))
}

/// The engine's `PxGeometryQuery::sweep` of a sphere of `radius` at
/// `center` along the unit or zero `direction` for `distance` against a box
/// of `half_extents` at `pose`, with the hit flags normal and MTD and zero
/// inflation. `trace` receives the native calls the sweep makes.
pub fn sweep_sphere_box(half_extents: [f32; 3], pose: &Pose, center: [f32; 3], radius: f32, direction: [f32; 3],
    distance: f32, mut trace: Option<&mut BoxSweepTrace>) -> Result<Option<MeshSweepHit>, Refused> {
    if !finite3(center) || !finite3(direction) || !finite3(half_extents) || !distance.is_finite() || !radius.is_finite() {
        return Err(Refused("a non-finite sweep"));
    }
    if !(distance >= 0.0) || !(radius > 0.0) {
        return Err(Refused("a negative sweep distance or a non-positive radius"));
    }
    // The sphere's pose has the identity rotation; the box's pose is the
    // query's.
    let cap_q = [0.0, 0.0, 0.0, 1.0];
    let (box_q, box_p) = (pose.rotation(), pose.translation());
    // The capsule's pose in the box's frame.
    let inv = [a::neg(box_q[0]), a::neg(box_q[1]), a::neg(box_q[2]), box_q[3]];
    let rel_p = quat_rotate(inv, sub(center, box_p));
    let rel_rot = quat_columns(quat_mul(inv, cap_q));
    // The zero half axis, rotated.
    let half_axis = columns_mul(&rel_rot, scale(UNIT_X, 0.0));
    let cap = Capsule { p0: add(rel_p, half_axis), p1: sub(rel_p, half_axis), radius };
    let bx = BoxShape::new(half_extents);
    let r = quat_rotate_inv(box_q, neg(scale(direction, distance)));
    let initial_dir = sub(rel_p, ZERO3);
    let inflation = a::add(radius, 0.0);
    let Some((toi, normal, closest_a)) = raycast_penetration(&cap, &bx, initial_dir, r, inflation, &mut trace)? else {
        return Ok(None);
    };
    let world_a = quat_transform(box_q, box_p, closest_a);
    let dest_normal = quat_rotate(box_q, normal);
    let (position, out_distance) = if 0.0 >= toi {
        (v3neg_scale_sub(dest_normal, toi, world_a), toi)
    } else {
        let len = a::mul(distance, toi);
        (v3scale_add(direction, len, world_a), len)
    };
    Ok(Some(MeshSweepHit { flags: super::sweep::FLAG_NORMAL | super::sweep::FLAG_POSITION, face_index: NO_FACE,
        distance: out_distance, normal: dest_normal, position: Some(position) }))
}

/// The box shape's world bounds at inflation one, min then max: the pose's
/// origin minus and plus, per axis, the sum of the absolute rotated half
/// extents (the rotation's columns scaled by the half extents; no contact
/// offset).
pub fn box_world_bounds(half_extents: [f32; 3], pose: &Pose) -> [f32; 6] {
    let m = Matrix34::from_pose(pose.rotation(), pose.translation());
    let [r0, r1, r2] = if arms::on("boxBoundsRows") {
        let c = m.columns;
        [[c[0][0], c[1][0], c[2][0]], [c[0][1], c[1][1], c[2][1]], [c[0][2], c[1][2], c[2][2]]]
    } else {
        m.columns
    };
    let t = pose.translation();
    let h = half_extents;
    let e: V3 = std::array::from_fn(|k| {
        let e = a::add(a::add(a::abs(a::mul(r0[k], h[0])), a::abs(a::mul(r1[k], h[1]))), a::abs(a::mul(r2[k], h[2])));
        a::mul(a::add(e, 0.0), 1.0)
    });
    [a::sub(t[0], e[0]), a::sub(t[1], e[1]), a::sub(t[2], e[2]), a::add(t[0], e[0]), a::add(t[1], e[1]), a::add(t[2], e[2])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};
    use std::collections::BTreeMap;

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn word(v: &Value) -> u32 {
        let x = v.as_f64().expect("row word");
        assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
        x as u32
    }
    fn words(v: &Value) -> Vec<u32> {
        v.as_array().expect("row array").iter().map(word).collect()
    }
    fn v3(w: &[u32]) -> V3 {
        [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])]
    }
    fn bits(v: V3) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    const ARMS: [&str; 5] =
        ["boxRecipThree", "boxCapsuleSweepMargin", "boxPenetrationCoreShape", "boxSkipEpa", "boxOutsideAllowance"];

    /// One row through the port: the returned fields that differ, and
    /// whether the call order differs.
    fn row_mismatch(row: &Value) -> (Vec<&'static str>, bool, String) {
        let p = words(field(row, "poseBits"));
        let pose = Pose::rotated([p[0], p[1], p[2], p[3]].map(f32::from_bits), v3(&p[4..])).expect("row pose");
        let mut trace = BoxSweepTrace::default();
        let got = sweep_sphere_box(v3(&words(field(row, "halfBits"))), &pose, v3(&words(field(row, "originBits"))),
            f32::from_bits(word(field(row, "radiusBits"))), v3(&words(field(row, "dirBits"))),
            f32::from_bits(word(field(row, "distanceBits"))), Some(&mut trace));
        let calls: Vec<String> = field(row, "calls").as_array().expect("calls").iter()
            .map(|c| c.as_str().expect("call").to_owned()).collect();
        let order = trace.calls.iter().map(|c| c.to_string()).collect::<Vec<_>>() != calls;
        let mut bad = Vec::new();
        let native_hit = field(row, "hit").as_bool().expect("hit");
        match (got, native_hit) {
            (Err(_), _) => bad.push("refused"),
            (Ok(None), false) => {}
            (Ok(Some(hit)), true) => {
                if hit.flags != word(field(row, "flags")) {
                    bad.push("flags");
                }
                if hit.face_index != word(field(row, "faceIndex")) {
                    bad.push("face");
                }
                if hit.distance.to_bits() != word(field(row, "outDistanceBits")) {
                    bad.push("distance");
                }
                if bits(hit.normal) != words(field(row, "normalBits")) {
                    bad.push("normal");
                }
                if hit.position.map(bits) != Some(words(field(row, "positionBits"))) {
                    bad.push("position");
                }
            }
            _ => bad.push("hit"),
        }
        let port = format!("port calls {:?} {:?}", trace.calls, got.map(|h| h.map(|h| (h.distance, h.normal, h.position))));
        (bad, order, port)
    }

    /// The engine's world bounds of a box shape at inflation one (identity,
    /// signed-zero, quarter-turn and random rotations; random and
    /// touch-box-like half extents): the port must equal every bit, and the
    /// mutant must differ on some row.
    #[test]
    #[ignore = "needs MOLY_BOX_BOUNDS_ROWS"]
    fn box_bounds_match_native_bits() {
        let path = std::env::var("MOLY_BOX_BOUNDS_ROWS").expect("MOLY_BOX_BOUNDS_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        assert_eq!(word(field(&doc, "inflationBits")), 0x3f80_0000, "the rows are at inflation one");
        let rows = field(&doc, "rows").as_array().expect("rows");
        let pass = || rows.iter().filter(|row| {
            let p = words(field(row, "poseBits"));
            let pose = Pose::rotated([p[0], p[1], p[2], p[3]].map(f32::from_bits), v3(&p[4..])).expect("row pose");
            let got = box_world_bounds(v3(&words(field(row, "halfBits"))), &pose);
            got.iter().map(|x| x.to_bits()).collect::<Vec<_>>() != words(field(row, "boundsBits"))
        }).count();
        arms::set(None);
        let bad = pass();
        arms::set(Some("boxBoundsRows"));
        let red = pass();
        arms::set(None);
        println!("box bounds replay: {} rows, mismatched {bad}; arm boxBoundsRows red on {red}", rows.len());
        assert_eq!(bad, 0, "box bounds rows differ from native");
        assert!(!rows.is_empty() && red > 0);
    }

    /// The engine's sphere-versus-box sweeps (hit flags normal and MTD,
    /// zero inflation): toward, away, grazing, overlapping, touching,
    /// distance-zero and signed-zero-direction sweeps and the corpus box, at
    /// the identity, signed-zero and random poses. The port must equal
    /// every returned field and the native call order on every row; each
    /// mutant must differ on some row.
    #[test]
    #[ignore = "needs MOLY_BOX_ROWS"]
    fn box_rows_match_native_bits() {
        let path = std::env::var("MOLY_BOX_ROWS").expect("MOLY_BOX_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        assert_eq!(word(field(&doc, "hitFlags")), 0x202, "the rows' hit flags are the product's");
        assert_eq!(word(field(&doc, "inflationBits")), 0, "the rows sweep with zero inflation");
        let rows = field(&doc, "rows").as_array().expect("rows");
        let pass = || {
            let mut reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
            let (mut outputs, mut orders) = (0, 0);
            let mut failures = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                let (bad, order, port) = row_mismatch(row);
                outputs += usize::from(!bad.is_empty());
                orders += usize::from(order);
                for r in &bad {
                    *reasons.entry(r).or_default() += 1;
                }
                if !bad.is_empty() || order {
                    failures.push(format!("row {i} {:?}: {bad:?} order {order}; {port}", field(row, "class").as_str()));
                }
            }
            (outputs, orders, reasons, failures)
        };
        arms::set(None);
        let (outputs, orders, reasons, failures) = pass();
        let count = |f: &dyn Fn(&Value) -> bool| rows.iter().filter(|r| f(r)).count();
        let hits = count(&|r| field(r, "hit").as_bool() == Some(true));
        let negative = count(&|r| field(r, "hit").as_bool() == Some(true)
            && f32::from_bits(word(field(r, "outDistanceBits"))) < 0.0);
        let with = |name: &str| rows.iter().filter(|r| field(r, "calls").as_array().expect("calls").iter()
            .any(|c| c.as_str() == Some(name))).count();
        let mut red = BTreeMap::new();
        for arm in ARMS {
            arms::set(Some(arm));
            let (o, c, _, _) = pass();
            red.insert(arm, (o, c));
        }
        arms::set(None);
        println!("box replay: {} rows ({hits} hits, {negative} negative distances, penetration {}, epa {}), rows \
            differing on returned fields {outputs}, on the call order {orders}, reasons {reasons:?}; arms red (fields, \
            order) {red:?}", rows.len(), with("penetration"), with("epa"));
        for failure in failures.iter().take(20) {
            println!("  {failure}");
        }
        assert!(failures.is_empty(), "{} box rows differ from native", failures.len());
        assert!(hits > 0 && negative > 0 && with("penetration") > 0 && with("epa") > 0);
        for arm in ARMS {
            let (o, c) = red[arm];
            assert!(o + c > 0, "arm {arm} never differs from native");
        }
    }
}
