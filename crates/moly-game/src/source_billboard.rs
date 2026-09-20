//! Source particle billboard vertices, before the original vertex program.
//! This is distinct from the legacy convenience quad API: the source's pivot,
//! scale/rotation order and non-unit normal are observable shader inputs.
use crate::{
    billboard::{ATTRIBUTE_CUSTOM1, ATTRIBUTE_CUSTOM2},
    particle_geometry::{reflect, Alignment, Frame, Instance},
};
use bevy::{
    math::Mat3,
    mesh::{Indices, Mesh},
    prelude::*,
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode {
    Billboard,
    Horizontal,
}

#[derive(Clone, Debug)]
pub(crate) struct Draw {
    pub mode: Mode,
    pub alignment: Alignment,
    pub pivot: Vec3,
    pub screen_size: Vec2,
    pub allow_roll: bool,
    pub scaling: crate::particle_geometry::Scaling,
}

fn euler(v: Vec3) -> Mat3 {
    Mat3::from_quat(Quat::from_euler(EulerRot::YXZ, v.y, v.x, v.z))
}
fn facing(direction: Vec3, up: Vec3) -> Mat3 {
    let z = direction.normalize_or_zero();
    let x = up.cross(z).normalize_or_zero();
    if x == Vec3::ZERO {
        Mat3::IDENTITY
    } else {
        Mat3::from_cols(x, z.cross(x), z)
    }
}

/// Actual source vertex order is top-left, top-right, bottom-right, bottom-left.
/// Its normal is a cross product of normalized corner vectors, NOT a normalized
/// face normal. The horizontal specialization excludes the pivot from this cross.
pub(crate) fn vertices(
    p: &Instance,
    frame: &Frame,
    draw: &Draw,
    local_simulation: bool,
) -> ([Vec3; 4], Vec3) {
    vertices_sized(p, frame, draw, local_simulation, p.size)
}

fn vertices_sized(
    p: &Instance,
    frame: &Frame,
    draw: &Draw,
    local_simulation: bool,
    size: Vec3,
) -> ([Vec3; 4], Vec3) {
    // The native pivot uses the UNCLAMPED authored size; screen limits affect
    // corner extents, not the pivot offset. Keep the two inputs separate.
    let scale = frame.scale;
    let (offsets, normal) = match draw.mode {
        Mode::Billboard => {
            let s = Mat3::from_diagonal(scale);
            let mut angles = p.rotation;
            if !draw.allow_roll && matches!(draw.alignment, Alignment::View | Alignment::Facing) {
                angles.z += frame
                    .camera_rotation
                    .x_axis
                    .y
                    .atan2(frame.camera_rotation.y_axis.y);
            }
            let rotation = euler(-angles);
            let basis = match draw.alignment {
                Alignment::View => {
                    frame.camera_rotation * Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0))
                }
                Alignment::World => Mat3::IDENTITY,
                Alignment::Local => frame.rotation,
                Alignment::Facing => facing(
                    p.position - frame.camera_position,
                    frame.camera_rotation.y_axis,
                ),
                Alignment::Velocity => unreachable!("velocity billboard geometry is not admitted"),
            };
            let matrix = if draw.alignment == Alignment::View
                || (draw.alignment == Alignment::Local && !local_simulation)
            {
                s * basis * rotation
            } else {
                basis * s * rotation
            };
            let pivot = Vec3::new(
                p.size.x * draw.pivot.x,
                p.size.y * draw.pivot.y,
                p.size.x * draw.pivot.z,
            );
            let corners = [
                Vec3::new(-size.x, size.y, 0.0),
                Vec3::new(size.x, size.y, 0.0),
                Vec3::new(size.x, -size.y, 0.0),
                Vec3::new(-size.x, -size.y, 0.0),
            ];
            let offsets = corners.map(|v| matrix * (v * 0.5 + pivot));
            let normal = offsets[0]
                .normalize_or_zero()
                .cross(offsets[1].normalize_or_zero());
            (offsets, normal)
        }
        Mode::Horizontal => {
            let (sin, cos) = (p.rotation.z + std::f32::consts::FRAC_PI_4).sin_cos();
            let x = size.x * scale.x * 0.5;
            let z = size.y * scale.z * 0.5;
            let corners = [
                Vec3::new(-cos * x, 0.0, sin * z),
                Vec3::new(sin * x, 0.0, cos * z),
                Vec3::new(cos * x, 0.0, -sin * z),
                Vec3::new(-sin * x, 0.0, -cos * z),
            ];
            let pivot = Vec3::new(
                p.size.x * scale.x * draw.pivot.x * cos,
                p.size.x * scale.y * draw.pivot.z,
                p.size.y * scale.z * draw.pivot.y * sin,
            );
            let normal = corners[0]
                .normalize_or_zero()
                .cross(corners[1].normalize_or_zero());
            (corners.map(|v| v + pivot), normal)
        }
    };
    let normal = if normal.length_squared() < 1.0e-30 {
        Vec3::Z
    } else {
        normal
    };
    (offsets.map(|v| p.position + v), normal)
}

/// Source min/max limits operate on the largest XY size with a 1e-6 divisor
/// floor, before renderer scale and the pivot transform. Zero dimensions remain
/// zero; there is no visual minimum-size substitution. Negative limits carry
/// the native disabled/behind-eye behavior, rather than Rust clamp panics.
pub(crate) fn screen_limited(size: Vec3, minimum: f32, maximum: f32) -> Vec3 {
    let largest = size.x.max(size.y).max(0.000001);
    let lower = if minimum >= 0.0 {
        largest.max(minimum)
    } else {
        0.0
    };
    let limited = if maximum >= 0.0 {
        lower.min(maximum)
    } else {
        lower
    };
    let factor = limited / largest;
    Vec3::new(size.x * factor, size.y * factor, size.z)
}

/// Preserve source UVs and custom streams. Geometry is stored in the shared
/// reflected world space; the source-program upload restores source coordinates.
pub(crate) fn write(
    mesh: &mut Mesh,
    draw: &Draw,
    particles: &[Instance],
    frame: &Frame,
    local_simulation: bool,
    fov_y: f32,
    aspect: f32,
) {
    let mut positions = Vec::with_capacity(particles.len() * 4);
    let mut normals = Vec::with_capacity(particles.len() * 4);
    let mut uv = Vec::with_capacity(particles.len() * 4);
    let mut colours = Vec::with_capacity(particles.len() * 4);
    let mut custom1 = Vec::with_capacity(particles.len() * 4);
    let mut custom2 = Vec::with_capacity(particles.len() * 4);
    let mut indices = Vec::with_capacity(particles.len() * 6);
    for p in particles {
        let base = positions.len() as u32;
        let depth = frame
            .camera_rotation
            .z_axis
            .dot(p.position - frame.camera_position);
        // The source camera writer supplies far-plane WIDTH / far distance,
        // divided by renderer scale X. This is the same coefficient at depth.
        let width = (2.0 * (fov_y * 0.5).tan() * aspect * depth) / frame.scale.x.max(0.00001);
        let size = screen_limited(
            p.size,
            draw.screen_size.x * width,
            draw.screen_size.y * width,
        );
        let (corners, normal) = vertices_sized(p, frame, draw, local_simulation, size);
        positions.extend(corners.map(|v| reflect(v).to_array()));
        normals.extend([reflect(normal).to_array(); 4]);
        uv.extend([[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        colours.extend([p.colour.to_array(); 4]);
        custom1.extend([p.custom1.to_array(); 4]);
        custom2.extend([p.custom2.to_array(); 4]);
        // Reflect the source triangle once, just as the mesh producer does.
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_attribute(ATTRIBUTE_CUSTOM1, custom1);
    mesh.insert_attribute(ATTRIBUTE_CUSTOM2, custom2);
    mesh.insert_indices(Indices::U32(indices));
}

#[cfg(test)]
#[path = "source_billboard_tests.rs"]
mod tests;
