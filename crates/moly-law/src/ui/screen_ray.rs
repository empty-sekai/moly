//! The engine's screen-point ray of an orthographic camera
//! (`Camera::ScreenPointToRay` for the mono eye), in the engine's operation
//! order, with the camera pose and matrices it reads.
//!
//! The camera's pixel rect is its normalized viewport rect placed in the
//! target rect (the screen when it renders to no texture) and clamped to it;
//! the implicit aspect is that rect's width over its height (1 for a zero
//! height). The implicit orthographic projection spans aspect * -size ..
//! size * aspect by -size .. size between the near and far planes. The
//! world-to-camera matrix is a (1, 1, -1) scale times the inverse of the
//! camera Transform's world position and rotation (its scale ignored). The
//! ray origin unprojects the screen point at the near-plane distance through
//! the inverse world-to-clip matrix; its direction is the camera's
//! normalized forward axis. A point the unprojection rejects gets the
//! engine's fallback ray (camera position, +z) and an error.
//!
//! Matrices are column-major, element (row r, column c) at `c * 4 + r`, as
//! the engine stores them. Every product and sum below is in the engine's
//! order; the engine's 4x4 matrix product fuses its multiply-adds, its
//! inverse and unprojection do not.

pub type Matrix = [f32; 16];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
}

/// A Transform's local position, rotation (x, y, z, w) and scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// An orthographic camera with an implicit projection and aspect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrthographicCamera {
    pub orthographic_size: f32,
    pub near: f32,
    pub far: f32,
    /// `m_NormalizedViewPortRect`.
    pub viewport: Rect,
    /// The camera Transform's world position and rotation.
    pub world_position: [f32; 3],
    pub world_rotation: [f32; 4],
}

/// ARM `fmax` for the non-NaN operands the rect clamps see.
fn fmax(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// The camera's pixel rect inside `target`.
pub fn camera_pixel_rect(target: Rect, viewport: Rect) -> Rect {
    let x0 = target.x + viewport.x * target.width;
    let y0 = target.y + target.height * viewport.y;
    let x_end = x0 + target.width * viewport.width;
    let y_end = y0 + target.height * viewport.height;
    let x_min = if x0 < target.x { target.x } else { x0 };
    let y_min = if y0 < target.y { target.y } else { y0 };
    let target_x_max = target.x + target.width;
    let target_y_max = target.y + target.height;
    let x_max = if x_end > target_x_max { target_x_max } else { x_end };
    let y_max = if y_end > target_y_max { target_y_max } else { y_end };
    Rect { x: x_min, y: y_min, width: fmax(x_max - x_min, 0.0), height: fmax(y_max - y_min, 0.0) }
}

/// `Camera::ResetAspect`'s implicit aspect of a pixel rect.
pub fn implicit_aspect(rect: Rect) -> f32 {
    if rect.height == 0.0 { 1.0 } else { rect.width / rect.height }
}

/// `RectfToRectInt`: edges rounded half up (a negative edge floored), the
/// size the difference of the rounded edges.
pub fn rect_to_rect_int(rect: Rect) -> [i32; 4] {
    // Minus the largest float below 1 (bits 0xbf7fffff): the floor step
    // for a negative edge.
    const BELOW_ONE: f32 = -0.99999994;
    let x_end = (rect.x + rect.width) + 0.5;
    let y_end = (rect.y + rect.height) + 0.5;
    let x = rect.x + 0.5;
    let y = rect.y + 0.5;
    let x = if x >= 0.0 { x } else { x + BELOW_ONE };
    let y = if y >= 0.0 { y } else { y + BELOW_ONE };
    let (x0, y0) = (x as i32, y as i32);
    let (x1, y1) = (x_end as u32 as i32, y_end as u32 as i32);
    [x0, y0, x1.wrapping_sub(x0), y1.wrapping_sub(y0)]
}

/// `Matrix4x4f::SetOrtho`.
pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Matrix {
    let width = right - left;
    let height = top - bottom;
    let depth = far - near;
    let mut m = [0.0; 16];
    m[0] = 2.0 / width;
    m[5] = 2.0 / height;
    m[10] = -2.0 / depth;
    m[12] = -(left + right) / width;
    m[13] = -(bottom + top) / height;
    m[14] = -(near + far) / depth;
    m[15] = 1.0;
    m
}

/// `Camera::GetProjectionMatrix` for an implicit orthographic projection.
pub fn implicit_orthographic_projection(size: f32, aspect: f32, near: f32, far: f32) -> Matrix {
    ortho(aspect * -size, size * aspect, -size, size, near, far)
}

/// `_MultiplyMatrices4x4_NEON`: each output column is the left matrix's
/// column 0 times the right column's first element, then columns 1..3
/// multiply-added (fused) in turn.
pub fn multiply(lhs: &Matrix, rhs: &Matrix) -> Matrix {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut v = lhs[r] * rhs[c * 4];
            for k in 1..4 {
                v = lhs[k * 4 + r].mul_add(rhs[c * 4 + k], v);
            }
            out[c * 4 + r] = v;
        }
    }
    out
}

/// `InvertMatrix4x4_Full`: Gauss-Jordan elimination with partial pivoting
/// over row swaps. A singular matrix gives false and all zeros.
pub fn invert_full(m: &Matrix) -> (bool, Matrix) {
    let at = |r: usize, c: usize| m[c * 4 + r];
    let mut rows = [[0.0f32; 8]; 4];
    for (r, row) in rows.iter_mut().enumerate() {
        for c in 0..4 {
            row[c] = at(r, c);
            row[4 + c] = if c == r { 1.0 } else { 0.0 };
        }
    }
    // r[k] indexes the physical row that plays row k.
    let mut r = [0usize, 1, 2, 3];
    let singular = (false, [0.0; 16]);
    macro_rules! v { ($k:expr, $c:expr) => { rows[r[$k]][$c] }; }
    if v!(3, 0).abs() > v!(2, 0).abs() { r.swap(3, 2); }
    if v!(2, 0).abs() > v!(1, 0).abs() { r.swap(2, 1); }
    if v!(1, 0).abs() > v!(0, 0).abs() { r.swap(1, 0); }
    if v!(0, 0) == 0.0 { return singular; }
    let m1 = v!(1, 0) / v!(0, 0);
    let m2 = v!(2, 0) / v!(0, 0);
    let m3 = v!(3, 0) / v!(0, 0);
    for c in 1..4 {
        let s = v!(0, c);
        v!(1, c) -= m1 * s;
        v!(2, c) -= m2 * s;
        v!(3, c) -= m3 * s;
    }
    for c in 4..8 {
        let s = v!(0, c);
        if s != 0.0 {
            v!(1, c) -= m1 * s;
            v!(2, c) -= m2 * s;
            v!(3, c) -= m3 * s;
        }
    }
    if v!(3, 1).abs() > v!(2, 1).abs() { r.swap(3, 2); }
    if v!(2, 1).abs() > v!(1, 1).abs() { r.swap(2, 1); }
    if v!(1, 1) == 0.0 { return singular; }
    let m2 = v!(2, 1) / v!(1, 1);
    let m3 = v!(3, 1) / v!(1, 1);
    for c in 2..4 {
        let s = v!(1, c);
        v!(2, c) -= m2 * s;
        v!(3, c) -= m3 * s;
    }
    for c in 4..8 {
        let s = v!(1, c);
        if s != 0.0 {
            v!(2, c) -= m2 * s;
            v!(3, c) -= m3 * s;
        }
    }
    if v!(3, 2).abs() > v!(2, 2).abs() { r.swap(3, 2); }
    if v!(2, 2) == 0.0 { return singular; }
    let m3 = v!(3, 2) / v!(2, 2);
    for c in 3..8 {
        let s = v!(2, c);
        v!(3, c) -= m3 * s;
    }
    if v!(3, 3) == 0.0 { return singular; }
    // Back substitution.
    let s = 1.0 / v!(3, 3);
    for c in 4..8 { v!(3, c) *= s; }
    let m2 = v!(2, 3);
    let s = 1.0 / v!(2, 2);
    for c in 4..8 { v!(2, c) = s * (v!(2, c) - v!(3, c) * m2); }
    let m1 = v!(1, 3);
    for c in 4..8 { v!(1, c) -= v!(3, c) * m1; }
    let m0 = v!(0, 3);
    for c in 4..8 { v!(0, c) -= v!(3, c) * m0; }
    let m1 = v!(1, 2);
    let s = 1.0 / v!(1, 1);
    for c in 4..8 { v!(1, c) = s * (v!(1, c) - v!(2, c) * m1); }
    let m0 = v!(0, 2);
    for c in 4..8 { v!(0, c) -= v!(2, c) * m0; }
    let m0 = v!(0, 1);
    let s = 1.0 / v!(0, 0);
    for c in 4..8 { v!(0, c) = s * (v!(0, c) - v!(1, c) * m0); }
    let mut out = [0.0; 16];
    for k in 0..4 {
        for c in 0..4 {
            out[c * 4 + k] = v!(k, 4 + c);
        }
    }
    (true, out)
}

/// `RotateVectorByQuat`.
pub fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let (x, y, z) = (qx * 2.0, qy * 2.0, qz * 2.0);
    let (xx, yy, zz) = (qx * x, qy * y, qz * z);
    let (xy, xz, yz) = (qx * y, qx * z, qy * z);
    let (wx, wy, wz) = (qw * x, qw * y, qw * z);
    [
        (1.0 - (yy + zz)) * v[0] + (xy - wz) * v[1] + (xz + wy) * v[2],
        (xy + wz) * v[0] + (1.0 - (xx + zz)) * v[1] + (yz - wx) * v[2],
        (xz - wy) * v[0] + (yz + wx) * v[1] + (1.0 - (xx + yy)) * v[2],
    ]
}

/// The Hamilton product `lhs * rhs` of two rotations.
pub fn multiply_rotations(lhs: [f32; 4], rhs: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = lhs;
    let [bx, by, bz, bw] = rhs;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by + ay * bw + az * bx - ax * bz,
        aw * bz + az * bw + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

impl Pose {
    /// A point of this Transform's local space in its parent's space: scaled,
    /// rotated, then offset.
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let scaled = [p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]];
        let rotated = rotate(self.rotation, scaled);
        [rotated[0] + self.position[0], rotated[1] + self.position[1], rotated[2] + self.position[2]]
    }
}

/// The world position and rotation of a Transform at `local_position` and
/// `local_rotation` under a parent with the world pose `parent`: the local
/// position taken through the parent (scaled, rotated, offset), the parent's
/// rotation times the local one.
pub fn child_world_pose(parent: &Pose, local_position: [f32; 3], local_rotation: [f32; 4]) -> ([f32; 3], [f32; 4]) {
    (parent.transform_point(local_position), multiply_rotations(parent.rotation, local_rotation))
}

/// The world-to-local matrix of a Transform at world `position` and
/// `rotation`, without its scale: the rotation's inverse, then minus the
/// rotated position.
pub fn world_to_local_no_scale(position: [f32; 3], rotation: [f32; 4]) -> Matrix {
    let q = [-rotation[0], -rotation[1], -rotation[2], rotation[3]];
    let columns = [rotate(q, [1.0, 0.0, 0.0]), rotate(q, [0.0, 1.0, 0.0]), rotate(q, [0.0, 0.0, 1.0])];
    let mut m = [0.0; 16];
    for (c, column) in columns.iter().enumerate() {
        m[c * 4..c * 4 + 3].copy_from_slice(column);
    }
    for r in 0..3 {
        m[12 + r] = columns[0][r] * -position[0]
            + (columns[1][r] * -position[1] + columns[2][r] * -position[2]);
    }
    m[15] = 1.0;
    m
}

/// `Matrix4x4f::SetScale`.
pub fn scale(v: [f32; 3]) -> Matrix {
    let mut m = [0.0; 16];
    m[0] = v[0];
    m[5] = v[1];
    m[10] = v[2];
    m[15] = 1.0;
    m
}

/// `CameraUnProject`: the world point on the ray through screen `p` (x, y
/// in pixels) at camera distance `p[2]`, or None where the engine gives up.
pub fn camera_unproject(p: [f32; 3], camera_to_world: &Matrix, clip_to_world: &Matrix, viewport: [i32; 4]) -> Option<[f32; 3]> {
    let [vx, vy, vw, vh] = viewport.map(|v| v as f32);
    let dx = p[0] - vx;
    let dy = p[1] - vy;
    let x = (dx + dx) / vw + -1.0;
    let y = (dy + dy) / vh + -1.0;
    // A depth between the near and far planes: the ray is the same at any.
    const DEPTH: f32 = 0.95;
    let m = clip_to_world;
    let w = m[15] + (m[11] * DEPTH + (m[3] * x + m[7] * y));
    // Rejected unless |w| > 1e-7 (a NaN is rejected too).
    if !((if w < 0.0 { -w } else { w }) > 1e-7) {
        return None;
    }
    let inverse = 1.0 / w;
    let row = |r: usize| m[12 + r] + ((m[r] * x + m[4 + r] * y) + m[8 + r] * DEPTH);
    let point = [inverse * row(0), row(1) * inverse, row(2) * inverse];
    let camera = [camera_to_world[12], camera_to_world[13], camera_to_world[14]];
    let axis_z = [camera_to_world[8], camera_to_world[9], camera_to_world[10]];
    let dir = [point[0] - camera[0], point[1] - camera[1], point[2] - camera[2]];
    // Distance along the camera's forward axis (minus its z axis).
    let distance = (dir[1] * -axis_z[1] - axis_z[0] * dir[0]) - dir[2] * axis_z[2];
    // Rejected unless |distance| >= 1e-6 (a NaN is rejected too).
    if !((if distance < 0.0 { -distance } else { distance }) >= 1e-6) {
        return None;
    }
    let orthographic = m[3] == 0.0 && m[7] == 0.0 && m[11] == 0.0 && m[15] == 1.0;
    Some(if orthographic {
        let along = distance - p[2];
        [point[0] + axis_z[0] * along, point[1] + axis_z[1] * along, point[2] + axis_z[2] * along]
    } else {
        let k = p[2] / distance;
        [camera[0] + k * dir[0], camera[1] + dir[1] * k, camera[2] + dir[2] * k]
    })
}

/// The camera's matrices for a target (screen) of `target` pixels:
/// (world to camera, projection, world to clip).
pub fn matrices(camera: &OrthographicCamera, target: [f32; 2]) -> (Matrix, Matrix, Matrix) {
    let rect = camera_pixel_rect(Rect { x: 0.0, y: 0.0, width: target[0], height: target[1] }, camera.viewport);
    let aspect = implicit_aspect(rect);
    let projection = implicit_orthographic_projection(camera.orthographic_size, aspect, camera.near, camera.far);
    let world_to_camera = multiply(&scale([1.0, 1.0, -1.0]), &world_to_local_no_scale(camera.world_position, camera.world_rotation));
    (world_to_camera, projection, multiply(&projection, &world_to_camera))
}

/// `Camera::ScreenPointToRay` (mono eye) of an orthographic camera for the
/// screen point `point` (pixels, origin bottom-left) on a target of
/// `target` pixels. The error is the engine's out-of-frustum case.
pub fn screen_point_to_ray(camera: &OrthographicCamera, target: [f32; 2], point: [f32; 2]) -> (Ray, Option<String>) {
    let rect = camera_pixel_rect(Rect { x: 0.0, y: 0.0, width: target[0], height: target[1] }, camera.viewport);
    let viewport = rect_to_rect_int(rect);
    let (world_to_camera, _, world_to_clip) = matrices(camera, target);
    let (_, clip_to_world) = invert_full(&world_to_clip);
    let (_, camera_to_world) = invert_full(&world_to_camera);
    match camera_unproject([point[0], point[1], camera.near], &camera_to_world, &clip_to_world, viewport) {
        Some(origin) => {
            let z = [camera_to_world[8], camera_to_world[9], camera_to_world[10]];
            let length = ((z[0] * z[0] + z[1] * z[1]) + z[2] * z[2]).sqrt();
            (Ray { origin, direction: [-z[0] / length, -z[1] / length, -z[2] / length] }, None)
        }
        None => (
            Ray { origin: camera.world_position, direction: [0.0, 0.0, 1.0] },
            Some(format!(
                "screen position out of view frustum (screen pos {}, {}) (camera rect {} {} {} {})",
                point[0], point[1], viewport[0], viewport[1], viewport[2], viewport[3]
            )),
        ),
    }
}
