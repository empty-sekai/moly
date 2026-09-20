//! Source mesh-surface particle births, separate from mesh-renderer geometry.
//! Triangle probability is proportional to source area, not triangle count.
use crate::particle_geometry::SourceMesh;
use bevy::prelude::*;

#[derive(Clone, Debug)]
struct Triangle {
    positions: [Vec3; 3],
    normals: [Vec3; 3],
}
#[derive(Clone, Debug)]
pub(crate) struct EmissionSurface {
    triangles: Vec<Triangle>,
    cumulative_area: Vec<f32>,
    total_area: f32,
}
impl EmissionSurface {
    pub(crate) fn from_source(source: &SourceMesh) -> Result<Self, String> {
        if source.indices.is_empty()
            || source.indices.len() % 3 != 0
            || source.positions.len() != source.normals.len()
            || source
                .positions
                .iter()
                .chain(source.normals.iter())
                .any(|v| !v.is_finite())
        {
            return Err(
                "mesh emission requires finite source positions/normals and complete triangles"
                    .into(),
            );
        }
        let mut triangles = Vec::with_capacity(source.indices.len() / 3);
        let mut cumulative_area = Vec::with_capacity(triangles.capacity());
        let mut total_area = 0.0f32;
        for indices in source.indices.chunks_exact(3) {
            // Positions/normals are already restored to source coordinates by
            // SourceMesh. Its indices retain GLB winding for drawing; undo that
            // reflection here to preserve the native barycentric vertex order.
            let indices = [
                indices[0] as usize,
                indices[2] as usize,
                indices[1] as usize,
            ];
            if indices.iter().any(|&i| i >= source.positions.len()) {
                return Err("emission triangle index is out of bounds".into());
            }
            let positions = indices.map(|i| source.positions[i]);
            let normals = indices.map(|i| source.normals[i]);
            let area = (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .length()
                * 0.5;
            total_area += area;
            if !area.is_finite() || !total_area.is_finite() {
                return Err("source emission surface area overflow".into());
            }
            // Retain zero-area source triangles and their index ordering. They
            // occupy no probability interval; do not rewrite the source mesh.
            cumulative_area.push(total_area);
            triangles.push(Triangle { positions, normals });
        }
        if total_area <= 0.0 {
            return Err("source emission mesh has no positive surface area".into());
        }
        Ok(Self {
            triangles,
            cumulative_area,
            total_area,
        })
    }
    pub(crate) fn sample(&self, selector: f32, mut u: f32, mut v: f32) -> ([f32; 3], [f32; 3]) {
        assert!(
            [selector, u, v]
                .iter()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
            "source sampler requires three U01 draws"
        );
        let target = selector * self.total_area;
        let index = self
            .cumulative_area
            .partition_point(|&area| area < target)
            .min(self.triangles.len() - 1);
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        let w = (1.0 - u) - v;
        let triangle = &self.triangles[index];
        let weighted =
            |values: [Vec3; 3]| ((values[0] * u + values[1] * v) + values[2] * w).to_array();
        (weighted(triangle.positions), weighted(triangle.normals))
    }
    pub(crate) fn triangles(&self) -> usize {
        self.triangles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> SourceMesh {
        SourceMesh {
            positions: vec![
                Vec3::ZERO,
                Vec3::new(2., 0., 0.),
                Vec3::new(0., 0., 1.),
                Vec3::new(10., 0., 0.),
                Vec3::new(14., 0., 0.),
                Vec3::new(10., 0., 2.),
            ],
            normals: vec![Vec3::Y; 6],
            indices: vec![0, 2, 1, 3, 5, 4],
            uv: vec![Vec2::ZERO; 6],
            colours: vec![Vec4::ONE; 6],
            bounds_size: Vec3::new(14., 0., 2.),
        }
    }
    #[test]
    fn triangle_selection_is_area_weighted_and_preserves_source_order() {
        let surface = EmissionSurface::from_source(&source()).unwrap();
        assert_eq!(surface.cumulative_area, vec![1., 5.]);
        assert_eq!(surface.sample(0., 1., 0.).0, [0., 0., 0.]);
        assert_eq!(surface.sample(0.2, 0., 1.).0, [2., 0., 0.]);
        assert_eq!(surface.sample(0.2001, 1., 0.).0, [10., 0., 0.]);
        assert_eq!(surface.sample(1., 0., 0.).0, [10., 0., 2.]);
        let selected = (0..10000)
            .filter(|i| surface.sample((*i as f32 + 0.5) / 10000., 0.3, 0.2).0[0] < 5.)
            .count();
        assert_eq!(selected, 2000);
    }
    #[test]
    fn barycentric_reflection_has_source_weights_and_unmodified_normals() {
        let mut mesh = source();
        mesh.normals[0] = Vec3::X;
        mesh.normals[1] = Vec3::Y;
        mesh.normals[2] = Vec3::Z;
        let surface = EmissionSurface::from_source(&mesh).unwrap();
        let (position, normal) = surface.sample(0., 0.8, 0.7);
        assert!((Vec3::from_array(position) - Vec3::new(0.6, 0., 0.5)).length() < 1e-6);
        assert!((Vec3::from_array(normal) - Vec3::new(0.2, 0.3, 0.5)).length() < 1e-6);
        assert!(Vec3::from_array(normal).length() < 0.7);
    }
    #[test]
    fn malformed_surface_is_an_error_not_an_origin_particle() {
        let mut mesh = source();
        mesh.indices[1] = 100;
        assert!(EmissionSurface::from_source(&mesh).is_err());
        let mut mesh = source();
        mesh.positions[0].x = f32::NAN;
        assert!(EmissionSurface::from_source(&mesh).is_err());
        let mut mesh = source();
        mesh.indices = vec![0, 0, 0];
        assert!(EmissionSurface::from_source(&mesh).is_err());
        let mut mesh = source();
        mesh.indices.pop();
        assert!(EmissionSurface::from_source(&mesh).is_err());
    }
}
