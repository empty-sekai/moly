//! Source-authored particle geometry. The simulation and source data retain
//! Unity's particle conventions; the rendering boundary performs the one X
//! reflection used by moly-root's GLB producer. A Mesh particle never becomes a
//! billboard. Numeric engine observations live in tests/data, independently of
//! this implementation; current-native call-site receipts live in the lane.
use bevy::math::Affine3A;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use std::sync::Arc;
use crate::billboard::{ATTRIBUTE_CUSTOM1, ATTRIBUTE_CUSTOM2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Alignment { View, World, Local, Facing, Velocity }
impl Alignment {
    pub(crate) fn from_source(value: i64) -> Option<Self> {
        Some(match value { 0 => Self::View, 1 => Self::World, 2 => Self::Local,
            3 => Self::Facing, 4 => Self::Velocity, _ => return None })
    }
}

/// Authored MainModule scaling affects particle geometry, independently of
/// simulation-space ownership. Local uses this emitter node, not its ancestors.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Scaling { Hierarchy, Local(Vec3) }
impl Scaling {
    pub(crate) fn apply(self, mut frame: Frame) -> Frame {
        if let Self::Local(scale) = self { frame.scale = scale; }
        frame
    }
}

/// All vectors/matrices below are in the source (Unity) world coordinate basis.
/// The owner's translation is already included in Instance::position.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub rotation: Mat3,
    pub scale: Vec3,
    pub camera_rotation: Mat3,
    pub camera_position: Vec3,
}
#[derive(Clone, Copy)]
pub(crate) struct Instance {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Source Euler angles in radians, applied Z then X then Y.
    pub rotation: Vec3,
    pub size: Vec3,
    pub colour: Vec4,
    pub custom1: Vec4,
    pub custom2: Vec4,
}

pub(crate) fn reflect(v: Vec3) -> Vec3 { Vec3::new(-v.x, v.y, v.z) }
fn reflect_rotation(r: Mat3) -> Mat3 {
    Mat3::from_cols(-reflect(r.x_axis), reflect(r.y_axis), reflect(r.z_axis))
}
pub(crate) fn source_frame(owner: &GlobalTransform, camera: &GlobalTransform) -> Frame {
    let (scale, rotation, _) = owner.to_scale_rotation_translation();
    let camera_rotation: Mat3 = camera.affine().matrix3.into();
    Frame {
        rotation: reflect_rotation(Mat3::from_quat(rotation)), scale,
        camera_rotation: Mat3::from_cols(reflect(camera_rotation.x_axis),
            reflect(camera_rotation.y_axis), -reflect(camera_rotation.z_axis)),
        camera_position: reflect(camera.translation()),
    }
}
fn euler(rotation: Vec3) -> Mat3 {
    Mat3::from_quat(Quat::from_euler(EulerRot::YXZ, rotation.y, rotation.x, rotation.z))
}
fn facing(forward: Vec3, up: Vec3) -> Mat3 {
    // The native LookRotation fallback for a zero velocity is the identity.
    let Some(z) = forward.try_normalize() else { return Mat3::IDENTITY; };
    let Some(x) = up.cross(z).try_normalize() else { return Mat3::IDENTITY; };
    Mat3::from_cols(x, z.cross(x), z)
}
fn aligned_basis(alignment: Alignment, particle: &Instance, frame: &Frame) -> Mat3 {
    match alignment {
        Alignment::View => frame.camera_rotation,
        Alignment::World => Mat3::IDENTITY,
        Alignment::Local => frame.rotation,
        Alignment::Facing => facing(particle.position - frame.camera_position, frame.camera_rotation.y_axis),
        Alignment::Velocity => facing(particle.velocity, Vec3::Y),
    }
}

/// Current CalculateMeshParticleTransform, verified against the independently
/// measured combinations of all five alignments, both simulation spaces,
/// nonuniform/negative scale, source XYZ rotation, mesh bounds, and pivot.
/// Scale precedes the particle rotation in the matrix product: B * S * R * P.
/// Moving S past R is observably wrong for nonuniform source transforms.
pub(crate) fn mesh_transform(
    particle: &Instance, frame: &Frame, alignment: Alignment,
    bounds_size: Vec3, pivot: Vec3,
) -> Affine3A {
    let linear = aligned_basis(alignment, particle, frame)
        * Mat3::from_diagonal(frame.scale)
        * euler(particle.rotation)
        * Mat3::from_diagonal(particle.size);
    // The renderer's Z pivot has the opposite sign to its X/Y mesh offset.
    let offset = bounds_size * Vec3::new(pivot.x, pivot.y, -pivot.z);
    Affine3A::from_mat3_translation(linear, particle.position + linear * offset)
}

#[derive(Clone, Debug)]
pub(crate) struct SourceMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub colours: Vec<Vec4>,
    /// GLB triangle winding is already reflected by the producer. Keep it.
    pub indices: Vec<u32>,
    /// Authored source Mesh.m_LocalAABB size, not a guessed sphere/quad size.
    pub bounds_size: Vec3,
}
impl SourceMesh {
    pub(crate) fn from_primitives(meshes: &[&Mesh], bounds_size: Vec3) -> Result<Self, String> {
        if !bounds_size.is_finite() || bounds_size.cmplt(Vec3::ZERO).any() {
            return Err("invalid authored source mesh bounds".into());
        }
        let mut out = Self { positions: Vec::new(), normals: Vec::new(), uv: Vec::new(),
            colours: Vec::new(), indices: Vec::new(), bounds_size };
        for mesh in meshes {
            if mesh.primitive_topology() != bevy::mesh::PrimitiveTopology::TriangleList {
                return Err("particle source primitive is not a triangle list".into());
            }
            let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
                return Err("source mesh position stream is missing or malformed".into());
            };
            let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
                return Err("source mesh normal stream is missing or malformed".into());
            };
            let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
                return Err("source mesh UV0 stream is missing or malformed".into());
            };
            if normals.len() != positions.len() || uv.len() != positions.len() {
                return Err("source mesh attribute lengths disagree".into());
            }
            let offset = u32::try_from(out.positions.len()).map_err(|_| "particle mesh exceeds u32 index range")?;
            out.positions.extend(positions.iter().map(|p| reflect(Vec3::from_array(*p))));
            out.normals.extend(normals.iter().map(|p| reflect(Vec3::from_array(*p))));
            // GLB uses top-origin UVs; particle shader's input contract is the
            // original source UV, so undo the producer's V reflection exactly once.
            out.uv.extend(uv.iter().map(|p| Vec2::new(p[0], 1.0 - p[1])));
            match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
                Some(VertexAttributeValues::Float32x4(colours)) if colours.len() == positions.len() =>
                    out.colours.extend(colours.iter().map(|c| Vec4::from_array(*c))),
                None => out.colours.extend(std::iter::repeat_n(Vec4::ONE, positions.len())),
                _ => return Err("source mesh colour stream is malformed".into()),
            }
            let indices: Vec<u32> = match mesh.indices() {
                Some(Indices::U16(values)) => values.iter().map(|v| u32::from(*v)).collect(),
                Some(Indices::U32(values)) => values.clone(),
                None => (0..positions.len() as u32).collect(),
            };
            if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= positions.len()) {
                return Err("source mesh index stream is malformed".into());
            }
            out.indices.extend(indices.into_iter().map(|i| i + offset));
        }
        if out.positions.is_empty() || out.indices.is_empty() { return Err("source mesh has no geometry".into()); }
        Ok(out)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MeshDraw {
    pub source: Arc<SourceMesh>,
    pub scaling: Scaling,
    pub alignment: Alignment,
    pub pivot: Vec3,
}

fn take_v3(mesh: &mut Mesh, attribute: bevy::mesh::MeshVertexAttribute) -> Vec<[f32; 3]> {
    match mesh.remove_attribute(attribute) { Some(VertexAttributeValues::Float32x3(mut v)) => { v.clear(); v }, _ => Vec::new() }
}
fn take_v4(mesh: &mut Mesh, attribute: bevy::mesh::MeshVertexAttribute) -> Vec<[f32; 4]> {
    match mesh.remove_attribute(attribute) { Some(VertexAttributeValues::Float32x4(mut v)) => { v.clear(); v }, _ => Vec::new() }
}

/// Reuse the existing mesh's allocations. Every particle draws the complete
/// authored primitive (positions, normals, UVs, vertex colours and indices).
pub(crate) fn write_mesh(mesh: &mut Mesh, draw: &MeshDraw, particles: &[Instance], frame: &Frame) {
    let mut positions = take_v3(mesh, Mesh::ATTRIBUTE_POSITION);
    let mut normals = take_v3(mesh, Mesh::ATTRIBUTE_NORMAL);
    let mut colours = take_v4(mesh, Mesh::ATTRIBUTE_COLOR);
    let mut custom1 = take_v4(mesh, ATTRIBUTE_CUSTOM1);
    let mut custom2 = take_v4(mesh, ATTRIBUTE_CUSTOM2);
    let mut uv = match mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(mut v)) => { v.clear(); v }, _ => Vec::new(),
    };
    let mut indices = match mesh.remove_indices() {
        Some(Indices::U32(mut v)) => { v.clear(); v }, _ => Vec::new(),
    };
    let source = &draw.source;
    let count = source.positions.len().saturating_mul(particles.len());
    positions.reserve(count); normals.reserve(count); colours.reserve(count);
    custom1.reserve(count); custom2.reserve(count); uv.reserve(count);
    indices.reserve(source.indices.len().saturating_mul(particles.len()));
    for particle in particles {
        let transform = mesh_transform(particle, frame, draw.alignment, source.bounds_size, draw.pivot);
        let linear: Mat3 = transform.matrix3.into();
        let normal_transform = if linear.determinant().abs() > 1e-30 { linear.inverse().transpose() } else { Mat3::ZERO };
        let base = positions.len() as u32;
        for i in 0..source.positions.len() {
            positions.push(reflect(transform.transform_point3(source.positions[i])).to_array());
            normals.push(reflect((normal_transform * source.normals[i]).normalize_or_zero()).to_array());
            uv.push(source.uv[i].to_array());
            colours.push((particle.colour * source.colours[i]).to_array());
            custom1.push(particle.custom1.to_array()); custom2.push(particle.custom2.to_array());
        }
        indices.extend(source.indices.iter().map(|i| *i + base));
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
mod tests {
    use super::*;
    use serde_json::Value;
    fn vec(v: &Value) -> Vec3 {
        Vec3::new(v["x"].as_f64().unwrap() as f32, v["y"].as_f64().unwrap() as f32, v["z"].as_f64().unwrap() as f32)
    }
    #[test]
    fn all_mesh_engine_observations_match_source_transform() {
        let data: Value = serde_json::from_str(include_str!("../tests/data/particle-mesh-geometry.json")).unwrap();
        let source: Vec<_> = data["sourceVertices"].as_array().unwrap().iter().map(vec).collect();
        let mut cases = 0;
        let mut failures = Vec::new();
        let mut maximum = 0.0f32;
        for row in data["cases"].as_array().unwrap() {
            let rotation = euler(vec(&row["ownerRotation"]) * (std::f32::consts::PI / 180.0));
            let scale = vec(&row["ownerScale"]);
            let frame = Frame { rotation, scale,
                camera_rotation: euler(vec(&row["cameraRotation"]) * (std::f32::consts::PI / 180.0)),
                camera_position: vec(&row["cameraPosition"]) };
            let mut position = vec(&row["particlePosition"]);
            let mut velocity = vec(&row["velocity"]);
            // BakeMesh's useTransform flag applies rotation/scale but excludes
            // owner translation. Reproduce that API boundary, not a fake world
            // translation fabricated by the measurement wrapper.
            if row["simulation"] == "Local" { position = rotation * (scale * position); velocity = rotation * (scale * velocity); }
            let instance = Instance { position, velocity, rotation: (vec(&row["particleRotation"]) * (std::f32::consts::PI / 180.0)),
                size: vec(&row["particleSize"]), colour: Vec4::ONE, custom1: Vec4::ZERO, custom2: Vec4::ZERO };
            let alignment = match row["alignment"].as_str().unwrap() {
                "View" => Alignment::View, "World" => Alignment::World, "Local" => Alignment::Local,
                "Facing" => Alignment::Facing, "Velocity" => Alignment::Velocity, _ => panic!("unknown source alignment"),
            };
            let transform = mesh_transform(&instance, &frame, alignment, Vec3::new(1.0,2.0,3.0), vec(&row["pivot"]));
            for (i, expected) in row["vertices"].as_array().unwrap().iter().enumerate() {
                let error = (transform.transform_point3(source[i]) - vec(expected)).abs().max_element();
                maximum = maximum.max(error);
                if !error.is_finite() || error > 0.000025 {
                    failures.push(format!("{alignment:?}/{}/{}, vertex={i}, error={error}", row["simulation"], row["variant"]));
                }
            }
            cases += 1;
        }
        assert_eq!(cases,130,"the independent source corpus must remain complete");
        assert!(failures.is_empty(),"max error={maximum}, {}",failures.join("\n"));
    }
}
