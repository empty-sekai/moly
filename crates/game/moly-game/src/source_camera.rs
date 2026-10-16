//! Camera conventions at the original-program boundary.
//!
//! The game's build lists Vulkan before OpenGL ES 3 as its graphics APIs, so
//! the programs run under the Vulkan conventions: a reversed-Z projection
//! whose clip z/w is 1 at the near plane and 0 at the far plane, the
//! reversed `_ZBufferParams` and a reversed depth texture. Their arithmetic
//! is the same in both translations; under the OpenGL ES conventions it reads
//! other values (a program's own clip z/w lies in [-1,1] there, so
//! `LinearEyeDepth` of it is not the eye distance). The programs' Y stays as
//! in the OpenGL ES reading (no projection flip, `_ProjectionParams.x` 1, a
//! bottom-left-origin depth texture), which samples the same pixel.
//!
//! The renderer's view projection has the same Z row, so the program's clip
//! position needs no adapter. A finite source far plane must apply to ALL
//! draws on that camera, not only particle vertices. The main-world camera
//! remains a Perspective projection for controls/picking; the render view
//! changes only its Z row.
use bevy::prelude::*;
use moly_assets::source_shader::{Result, SourceShaderError};

fn clip_planes(camera: &Projection) -> Result<(f32, f32)> {
    let (near, far) = match camera {
        Projection::Perspective(p) => (p.near, p.far),
        Projection::Orthographic(p) => (p.near, p.far),
        Projection::Custom(_) => {
            return Err(SourceShaderError(
                "custom camera projection needs explicit source parameter ownership".into(),
            ))
        }
    };
    if !(near.is_finite() && far.is_finite() && near > 0.0 && far > near) {
        return Err(SourceShaderError(format!(
            "source camera requires positive finite ordered clip planes: {near}, {far}"
        )));
    }
    Ok((near, far))
}

/// Preserve the extracted XY framing (including a sub-view) while installing
/// the actual finite far plane. This precedes every view uniform and draw.
pub(crate) fn render_projection(mut extracted: Mat4, camera: &Projection) -> Result<Mat4> {
    let (near, far) = clip_planes(camera)?;
    if matches!(camera, Projection::Perspective(_)) {
        if extracted.z_axis.w != -1.0
            || extracted.w_axis.w != 0.0
            || extracted.x_axis.z != 0.0
            || extracted.y_axis.z != 0.0
        {
            return Err(SourceShaderError(
                "oblique source perspective projection is not owned".into(),
            ));
        }
        let reciprocal_range = (far - near).recip();
        extracted.z_axis.z = near * reciprocal_range;
        extracted.w_axis.z = far * (near * reciprocal_range);
    }
    Ok(extracted)
}

/// ScriptableRenderer's `_ZBufferParams` writer on a reversed-Z device,
/// preserving its f32 operation order: y = far * reciprocal(near), x = 1-y,
/// z = x * reciprocal(far), w = y * reciprocal(far), then y += x, x = -x,
/// w += z, z = -z. This is paired with a reversed-depth, bottom-left-origin
/// source snapshot.
pub(crate) fn z_buffer_params(camera: &Projection) -> Result<[f32; 4]> {
    let (near, far) = clip_planes(camera)?;
    let inv_far = far.recip();
    let y = far * near.recip();
    let x = 1.0 - y;
    let (z, w) = (x * inv_far, y * inv_far);
    Ok([-x, y + x, -z, w + z])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::CameraProjection;

    #[test]
    fn finite_source_planes_keep_the_framing() {
        for (near, far) in [(0.01, 5.0), (0.1, 100.0), (0.3, 1000.0), (1.0, 10000.0)] {
            let camera = Projection::Perspective(PerspectiveProjection {
                near,
                far,
                fov: 1.1,
                aspect_ratio: 1.7,
                ..default()
            });
            let old = camera.get_clip_from_view();
            let render = render_projection(old, &camera).unwrap();
            // Reversed near/far clip boundaries, and unchanged XY framing.
            for (eye, render_z) in [(near, 1.0), (far, 0.0)] {
                let p = Vec4::new(0.3, -0.2, -eye, 1.0);
                let r = render * p;
                assert!((r.z / r.w - render_z).abs() < 3e-6);
                assert_eq!((old * p).truncate().truncate(), r.truncate().truncate());
            }
        }
    }

    #[test]
    fn source_depth_reconstructs_eye_distance_in_its_own_domain() {
        let camera = Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: 100.0,
            ..default()
        });
        let render = render_projection(camera.get_clip_from_view(), &camera).unwrap();
        let p = z_buffer_params(&camera).unwrap();
        // Current source writer operations, independently recorded values.
        assert_eq!(
            p.map(f32::to_bits),
            [999.0f32, 1.0, 9.99, 0.010000229].map(f32::to_bits)
        );
        // `LinearEyeDepth` of the depth texture and of a program's own clip
        // z/w both give the eye distance.
        for eye in [0.1, 0.2, 1.0, 3.0, 10.0, 50.0, 99.0] {
            let clip = render * Vec4::new(0.0, 0.0, -eye, 1.0);
            let reconstructed = (p[2] * (clip.z / clip.w) + p[3]).recip();
            assert!(
                (reconstructed - eye).abs() < 0.001 * eye,
                "{eye}: {reconstructed}"
            );
        }
    }

    #[test]
    fn unowned_clip_planes_do_not_substitute_an_infinite_far_plane() {
        for (near, far) in [
            (0.0, 10.0),
            (1.0, 1.0),
            (1.0, f32::INFINITY),
            (f32::NAN, 100.0),
        ] {
            let camera = Projection::Perspective(PerspectiveProjection {
                near,
                far,
                ..default()
            });
            assert!(render_projection(Mat4::IDENTITY, &camera).is_err());
            assert!(z_buffer_params(&camera).is_err());
        }
    }
}

#[cfg(test)]
#[path = "source_camera_native_tests.rs"]
mod native_tests;
