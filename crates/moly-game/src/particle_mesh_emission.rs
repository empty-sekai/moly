//! Source mesh-surface particle births, separate from mesh-renderer geometry.
//! The surface is the module's mesh cache of the Triangle placement (see
//! `moly_law::particle::shape_mesh`): triangle areas, per-submesh sums and
//! the area lookup table, in the source's own vertex and triangle order. The
//! native birth path samples it through the Shape law; the legacy step picks
//! and blends with the same cache from its own draws.
use crate::particle_geometry::SourceMesh;
use moly_law::particle::shape_mesh::MeshShapeCache;

#[derive(Clone, Debug)]
pub(crate) struct EmissionSurface {
    native: MeshShapeCache,
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
        if source.submesh_ends.last() != Some(&source.indices.len())
            || source.submesh_ends.windows(2).any(|pair| pair[0] > pair[1])
            || source.submesh_ends.iter().any(|end| end % 3 != 0)
        {
            return Err("mesh emission requires the source submesh boundaries".into());
        }
        let mut submeshes = Vec::with_capacity(source.submesh_ends.len());
        let mut start = 0;
        for &end in &source.submesh_ends {
            // Positions/normals are already restored to source coordinates by
            // SourceMesh. Its indices retain GLB winding for drawing; undo that
            // reflection here to restore the native triangle corner order.
            submeshes.push(
                source.indices[start..end]
                    .chunks_exact(3)
                    .flat_map(|t| [t[0], t[2], t[1]])
                    .collect::<Vec<u32>>(),
            );
            start = end;
        }
        let native = MeshShapeCache::new(
            source.positions.iter().map(|p| p.to_array()).collect(),
            Some(source.normals.iter().map(|n| n.to_array()).collect()),
            &submeshes,
            None,
        )
        .map_err(|refused| format!("source emission mesh cache refused: {refused:?}"))?;
        let total_area = native.total_area();
        if !total_area.is_finite() || native.triangles().iter().any(|t| !t.area.is_finite()) {
            return Err("source emission surface area overflow".into());
        }
        if total_area <= 0.0 {
            return Err("source emission mesh has no positive surface area".into());
        }
        Ok(Self { native })
    }
    /// The module's mesh cache, as the native Shape law reads it.
    pub(crate) fn native(&self) -> &MeshShapeCache {
        &self.native
    }
    /// The legacy step's sample: the native pick and blend (normal offset
    /// zero, the only one the surface contract admits) from its own draws.
    pub(crate) fn sample(&self, selector: f32, u: f32, v: f32) -> ([f32; 3], [f32; 3]) {
        assert!(
            [selector, u, v]
                .iter()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
            "source sampler requires three U01 draws"
        );
        self.native.point(self.native.pick(selector), u, v, 0.0)
    }
    pub(crate) fn triangles(&self) -> usize {
        self.native.triangles().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
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
            submesh_ends: vec![6],
            uv: vec![Vec2::ZERO; 6],
            colours: vec![Vec4::ONE; 6],
            bounds_size: Vec3::new(14., 0., 2.),
        }
    }
    #[test]
    fn triangle_selection_is_area_weighted_and_preserves_source_order() {
        let surface = EmissionSurface::from_source(&source()).unwrap();
        assert_eq!(surface.native().triangles().iter().map(|t| t.area).collect::<Vec<_>>(), vec![1., 4.]);
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
        mesh.submesh_ends = vec![3];
        assert!(EmissionSurface::from_source(&mesh).is_err());
        let mut mesh = source();
        mesh.indices.pop();
        assert!(EmissionSurface::from_source(&mesh).is_err());
        let mut mesh = source();
        mesh.submesh_ends.clear();
        assert!(EmissionSurface::from_source(&mesh).is_err());
    }
}
