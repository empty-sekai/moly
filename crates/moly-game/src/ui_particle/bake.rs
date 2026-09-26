//! The source's canvas bake, in the source basis: the Unity world (y up, the
//! camera looking down +z), with column vectors composed as `Matrix4x4`
//! composes them. The runtime simulates in the same world with x reflected;
//! [`to_runtime`] and [`reflect`] cross that boundary.
//!
//! The frame of one UIParticle: the root canvas, the UIParticle node (whose
//! own scale `ModifyScale` drives while it ignores the canvas scaler), each
//! particle system node under it, and the baking camera
//! (`BakingCamera.GetCamera`).
//!
//! `UIParticleUpdater.BakeMesh`, per particle system in `m_Particles` order:
//! - `rootMatrix = Rotate(root.rotation).inverse * Scale(root.lossyScale).inverse`;
//! - a system on its own node below the root, simulating in Local space:
//!   `Translate(root.InverseTransformPoint(system.position)) * rootMatrix`;
//!   in any other space `rootMatrix * Translate(-root.position)`;
//! - a system on the root node itself: `GetScaledMatrix` (Local:
//!   `Rotate(rotation).inverse * Scale(lossyScale).inverse`, World:
//!   `worldToLocalMatrix`);
//! - then `Scale(scale) * matrix`, where `scale` is the root canvas' local
//!   scale times `scale3D` while the UIParticle ignores the canvas scaler and
//!   `scale3D` otherwise.
//! The baked mesh (`ParticleSystemRenderer.BakeMesh(mesh, camera, true)`:
//! the system's rotation and scale, not its position) goes through that
//! matrix into the UIParticle's local space, and the CanvasRenderer draws it
//! there.

use bevy::math::{Affine3A, Mat3, Mat4, Quat, Vec3};

/// `Mathf.Epsilon`, the smallest positive subnormal float.
const EPSILON: f32 = f32::from_bits(1);

/// `Mathf.Approximately`: `|b - a| < max(1e-6 * max(|a|, |b|), Epsilon * 8)`.
pub(crate) fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (1.0e-6 * a.abs().max(b.abs())).max(EPSILON * 8.0)
}

/// `BakingCamera.s_OrthoPosition`: its static constructor stores zero in x
/// and y and the word 0xC47A0000 in z.
pub(crate) const ORTHO_POSITION: Vec3 = Vec3::new(0.0, 0.0, f32::from_bits(0xc47a_0000));

/// `BakingCamera.GetCamera` sets `farClipPlane` to the word 0x44FA0000.
pub(crate) const FAR_CLIP_PLANE: f32 = f32::from_bits(0x44fa_0000);

/// A Transform in the source world: its world matrix (the local TRS matrices
/// composed from the root) and its world rotation (`Transform.rotation`, the
/// local rotations alone composed from the root).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Node {
    pub(crate) matrix: Mat4,
    pub(crate) rotation: Quat,
}

impl Node {
    /// A root Transform at `position`, `rotation` and `scale`.
    pub(crate) fn root(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            matrix: Mat4::from_scale_rotation_translation(scale, rotation, position),
            rotation,
        }
    }

    /// A child Transform at its local `position`, `rotation` and `scale`.
    pub(crate) fn child(&self, position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            matrix: self.matrix * Mat4::from_scale_rotation_translation(scale, rotation, position),
            rotation: self.rotation * rotation,
        }
    }

    /// `Transform.position`.
    pub(crate) fn position(&self) -> Vec3 {
        self.matrix.w_axis.truncate()
    }

    /// `Transform.lossyScale`: the diagonal of the inverse world rotation
    /// times the world rotation-and-scale matrix.
    pub(crate) fn lossy_scale(&self) -> Vec3 {
        let m = Mat3::from_quat(self.rotation.inverse()) * Mat3::from_mat4(self.matrix);
        Vec3::new(m.x_axis.x, m.y_axis.y, m.z_axis.z)
    }

    /// `Transform.InverseTransformPoint`.
    pub(crate) fn inverse_transform_point(&self, point: Vec3) -> Vec3 {
        self.matrix.inverse().transform_point3(point)
    }
}

/// `UIParticleUpdater.ModifyScale`: the UIParticle node's local scale this
/// frame while it ignores the canvas scaler. `current` is the scale it holds
/// (the serialized one before the first frame). Each component of the target
/// is the inverse of the root canvas' local scale, or 1 where that is
/// approximately 0; a scale already within `Approximately` of it (on the
/// squared distance) is kept as it is.
pub(crate) fn driven_scale(current: Vec3, root_canvas_scale: Vec3) -> Vec3 {
    let inverse = |s: f32| if approximately(s, 0.0) { 1.0 } else { 1.0 / s };
    let target = Vec3::new(
        inverse(root_canvas_scale.x),
        inverse(root_canvas_scale.y),
        inverse(root_canvas_scale.z),
    );
    if approximately((current - target).length_squared(), 0.0) {
        current
    } else {
        target
    }
}

/// The scale `BakeMesh` multiplies in last.
pub(crate) fn bake_scale(
    ignore_canvas_scaler: bool,
    root_canvas_scale: Vec3,
    scale3d: Vec3,
) -> Vec3 {
    if ignore_canvas_scaler {
        root_canvas_scale * scale3d
    } else {
        scale3d
    }
}

/// The simulation spaces the bake distinguishes. Custom is refused before a
/// system reaches the bake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    Local,
    World,
}

/// The matrix `BakeMesh` pushes one system's baked mesh with: baked vertices
/// into the UIParticle's local space. `system` is the system's own node, or
/// `None` when the system sits on the UIParticle node itself.
pub(crate) fn bake_matrix(root: &Node, system: Option<&Node>, space: Space, scale: Vec3) -> Mat4 {
    let root_matrix =
        Mat4::from_quat(root.rotation).inverse() * Mat4::from_scale(root.lossy_scale()).inverse();
    let matrix = match (system, space) {
        (Some(system), Space::Local) => {
            Mat4::from_translation(root.inverse_transform_point(system.position())) * root_matrix
        }
        (Some(_), Space::World) => root_matrix * Mat4::from_translation(-root.position()),
        (None, Space::Local) => root_matrix,
        (None, Space::World) => root.matrix.inverse(),
    };
    Mat4::from_scale(scale) * matrix
}

/// A baked vertex into the root canvas' local space: the bake into the
/// UIParticle's local space ([`bake_matrix`]), then the UIParticle node's
/// frame relative to the root canvas (the CanvasRenderer draws the mesh in the
/// node's space).
pub(crate) fn to_canvas(
    canvas_root: &Node,
    root: &Node,
    system: Option<&Node>,
    space: Space,
    scale: Vec3,
) -> Mat4 {
    canvas_root.matrix.inverse() * root.matrix * bake_matrix(root, system, space, scale)
}

/// The render matrix `BakeMesh(..., useTransform: true)` applies to a Local
/// system's particles: the system's world rotation and its scale (its local
/// scale under the Local scaling mode, its lossy scale under Hierarchy), no
/// position.
pub(crate) fn local_render_matrix(system: &Node, scale: Vec3) -> Mat4 {
    Mat4::from_quat(system.rotation) * Mat4::from_scale(scale)
}

/// The extra world simulation of `BakeMesh`: while the UIParticle has drawn
/// before (`activeMeshIndices.CountFast() != 0`), a World system's particles
/// move by the root's displacement since the last bake, each component times
/// `1 - 1 / max(0.001, scale)`.
pub(crate) fn world_displacement(
    position: Vec3,
    cached: Vec3,
    scale: Vec3,
    drawn_before: bool,
) -> Vec3 {
    if !drawn_before {
        return Vec3::ZERO;
    }
    let factor = |s: f32| 1.0 - 1.0 / s.max(0.001);
    (position - cached) * Vec3::new(factor(scale.x), factor(scale.y), factor(scale.z))
}

/// `BakingCamera.GetCamera`: the camera the bake orients billboards to. It
/// sits at [`ORTHO_POSITION`], rotated as the root canvas' world camera is
/// for a canvas that is not an overlay and has one (identity otherwise), and
/// its orthographic size is the root canvas rect's larger side times the
/// canvas scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BakingCamera {
    pub(crate) position: Vec3,
    pub(crate) rotation: Quat,
    pub(crate) orthographic_size: f32,
    pub(crate) far: f32,
}

pub(crate) fn baking_camera(
    root_rect: [f32; 2],
    scale_factor: f32,
    world_camera_rotation: Option<Quat>,
) -> BakingCamera {
    BakingCamera {
        position: ORTHO_POSITION,
        rotation: world_camera_rotation.unwrap_or(Quat::IDENTITY),
        orthographic_size: root_rect[0].max(root_rect[1]) * scale_factor,
        far: FAR_CLIP_PLANE,
    }
}

/// The runtime basis: the source world with x reflected.
pub(crate) fn reflect(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}

/// A source-world matrix in the runtime basis (the x reflection on both sides).
pub(crate) fn to_runtime(m: Mat4) -> Affine3A {
    let f = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    Affine3A::from_mat4(f * m * f)
}

/// The baking camera as a runtime-basis camera transform. The runtime's
/// geometry reads a camera the way the host's cameras are built: looking down
/// its -z, with the source camera's axes (right, up, forward) recovered as
/// (reflect(x), reflect(y), -reflect(z)).
pub(crate) fn runtime_camera(camera: &BakingCamera) -> Affine3A {
    let source = Mat3::from_quat(camera.rotation);
    let rotation = Mat3::from_cols(
        reflect(source.x_axis),
        reflect(source.y_axis),
        -reflect(source.z_axis),
    );
    Affine3A::from_mat3_translation(rotation, reflect(camera.position))
}
