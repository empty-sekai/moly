//! Primitive geometry in world units.  Physics scale rules follow Unity's
//! 2022.3 SphereColliderEditor/CapsuleColliderEditor and CapsuleBoundsHandle.
//! Native NavMeshObstacle dimensions and axes are resolved separately in the
//! parent module. Its serialized height CAN be below diameter, but the native
//! carving shape clamps the cylindrical segment to zero (yielding a sphere).

use bevy::prelude::*;

pub(super) type Mesh = (Vec<[f32; 3]>, Vec<[usize; 3]>);
const RADIAL_STEPS: usize = 32;
const LATITUDE_STEPS: usize = 16;

pub(super) fn scale_magnitudes(transform: &GlobalTransform) -> Vec3 {
    let matrix = transform.to_matrix();
    Vec3::new(
        matrix.x_axis.truncate().length(),
        matrix.y_axis.truncate().length(),
        matrix.z_axis.truncate().length(),
    )
}

pub(super) fn physics_scale(transform: &GlobalTransform) -> Result<Vec3, String> {
    let matrix = transform.to_matrix();
    let axes = [
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        matrix.z_axis.truncate(),
    ];
    let scale = Vec3::new(axes[0].length(), axes[1].length(), axes[2].length());
    if !scale.is_finite() || scale.min_element() <= 0.0 {
        return Err("degenerate physics primitive transform".into());
    }
    // lossyScale is exact for an orthogonal TRS frame. Native primitive
    // handling under shear has not been established here. Do not turn an
    // unverified sheared hierarchy into either a skewed primitive or ellipse.
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        if (axes[a].dot(axes[b]) / (scale[a] * scale[b])).abs() > 1e-4 {
            return Err("sheared physics primitive transform unsupported".into());
        }
    }
    Ok(scale)
}

pub(super) fn box_mesh(center: Vec3, size: Vec3) -> Result<Mesh, String> {
    let points = super::box_points(center, size)?
        .into_iter()
        .map(|p| p.to_array())
        .collect();
    // Vertex order is x-major, then y, then z, as in box_points.
    let faces = vec![
        [0, 1, 3],
        [0, 3, 2],
        [4, 6, 7],
        [4, 7, 5],
        [0, 4, 5],
        [0, 5, 1],
        [2, 3, 7],
        [2, 7, 6],
        [0, 2, 6],
        [0, 6, 4],
        [1, 5, 7],
        [1, 7, 3],
    ];
    Ok((points, faces))
}

/// The 34 input points used by Unity 2022.3.62f2 x86_64's capsule branch of
/// CarveNavMeshTile (0xbcdeb4..0xbce339). This is deliberately NOT the finer
/// physics-primitive envelope below: changing its eight-sided boundary would
/// allow locations the source carve still removes. Native downstream plane
/// clipping/agent expansion remains the navigation host's separate obligation.
pub(super) fn nav_capsule_points(
    center: Vec3,
    rotation: Quat,
    radius: f32,
    half_height: f32,
) -> Result<Vec<[f32; 3]>, String> {
    if !center.is_finite()
        || !rotation.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || !half_height.is_finite()
        || half_height < 0.0
    {
        return Err("invalid obstacle capsule geometry".into());
    }
    // Keep the measured f32 constants and multiplication order: these are
    // rodata 0x1547ec and 0x154714, not runtime trigonometric substitutes.
    let ring_radius = radius * 1.082_392_2_f32;
    let middle_radius = (radius * 0.707_106_77_f32) * 1.082_392_2_f32;
    let segment = (half_height - radius).max(0.0);
    let axis_x = rotation * Vec3::X;
    let axis_y = rotation * Vec3::Y;
    let axis_z = rotation * Vec3::Z;
    let mut points = Vec::with_capacity(34);
    for step in 0..8 {
        let angle = ((step as f32 * 0.125_f32) * std::f32::consts::PI) * 2.0;
        let (sin, cos) = angle.sin_cos();
        let direction = axis_x * cos + axis_z * sin;
        let equator = center + direction * ring_radius;
        let middle = center + direction * middle_radius;
        points.push((equator - axis_y * segment).to_array());
        points.push((equator + axis_y * segment).to_array());
        points.push((middle - axis_y * (segment + middle_radius)).to_array());
        points.push((middle + axis_y * (segment + middle_radius)).to_array());
    }
    points.push((center - axis_y * (segment + ring_radius)).to_array());
    points.push((center + axis_y * (segment + ring_radius)).to_array());
    Ok(points)
}

/// Sampling both spherical directions and their antipodes gives a convex
/// circumscribed sphere after a uniform enlargement.  For any unit direction,
/// its closest latitude/longitude sample has dot product at least
/// cos(pi/32)^2. Thus inflation by its reciprocal makes every support plane
/// contain the true sphere; max outward distance is < .98% of radius. Adding
/// both segment endpoints preserves this bound for a capsule (Minkowski sum).
pub(super) fn rounded_solid(
    center: Vec3,
    axis: Vec3,
    radius: f32,
    segment_half: f32,
) -> Result<Mesh, String> {
    if !center.is_finite()
        || !axis.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || !segment_half.is_finite()
        || segment_half < 0.0
        || (axis.length_squared() - 1.0).abs() > 1e-3
    {
        return Err("invalid rounded collider geometry".into());
    }
    let inflation = rounded_inflation();
    let mut points = Vec::with_capacity((LATITUDE_STEPS - 1) * RADIAL_STEPS * 2 + 4);
    for latitude in 0..=LATITUDE_STEPS {
        let theta = latitude as f32 * std::f32::consts::PI / LATITUDE_STEPS as f32;
        let steps = if latitude == 0 || latitude == LATITUDE_STEPS {
            1
        } else {
            RADIAL_STEPS
        };
        for longitude in 0..steps {
            let phi = longitude as f32 * std::f32::consts::TAU / RADIAL_STEPS as f32;
            let normal = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let surface = normal * (radius * inflation);
            for end in [-segment_half, segment_half] {
                points.push((center + surface + axis * end).to_array());
            }
        }
    }
    super::convex_mesh(&points)
}

pub(super) fn rounded_inflation() -> f32 {
    let angular_cos = (std::f32::consts::PI / RADIAL_STEPS as f32).cos();
    // Tiny numerical guard prevents f32 QuickHull plane roundoff from making
    // the analytically circumscribed envelope microscopically inscribed.
    (1.0 + 2e-6) / (angular_cos * angular_cos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_envelope_contains_its_analytic_capsule_with_bounded_error() {
        let radius = 0.4;
        let half = 0.8;
        let axis = Vec3::new(1.0, 2.0, -1.0).normalize();
        let (points, triangles) = rounded_solid(Vec3::ZERO, axis, radius, half).unwrap();
        assert!(rounded_inflation() - 1.0 < 0.0098);
        for triangle in triangles {
            let [a, b, c] = triangle.map(|i| Vec3::from(points[i]));
            let mut normal = (b - a).cross(c - a).normalize();
            if normal.dot(a) < 0.0 {
                normal = -normal;
            }
            let expected_support = radius + half * normal.dot(axis).abs();
            assert!(
                normal.dot(a) + 2e-5 >= expected_support,
                "a hull face cuts the analytic capsule"
            );
        }
        for point in points {
            let point = Vec3::from(point);
            let on_segment = axis * point.dot(axis).clamp(-half, half);
            assert!(point.distance(on_segment) <= radius * rounded_inflation() + 1e-5);
        }
    }

    #[test]
    fn native_carve_uses_34_points_and_normalizes_short_capsules_to_spheres() {
        for (radius, half_height, center_y) in [(0.49, 0.07, 0.0), (0.08, 0.00001, 0.07)] {
            let points =
                nav_capsule_points(Vec3::Y * center_y, Quat::IDENTITY, radius, half_height)
                    .unwrap();
            assert_eq!(points.len(), 34);
            let min = points.iter().fold(f32::INFINITY, |a, p| a.min(p[1]));
            let max = points.iter().fold(f32::NEG_INFINITY, |a, p| a.max(p[1]));
            let native_radius = radius * 1.082_392_2_f32;
            assert!((min - (center_y - native_radius)).abs() < 1e-6);
            assert!((max - (center_y + native_radius)).abs() < 1e-6);
            assert_eq!(points[0], points[1], "zero cylindrical segment");
        }
        let points = nav_capsule_points(Vec3::ZERO, Quat::IDENTITY, 0.4, 1.0).unwrap();
        assert_eq!(points[0], [0.4 * 1.082_392_2, -0.6, 0.0]);
        assert_eq!(points[32], [0.0, -(0.6 + 0.4 * 1.082_392_2), 0.0]);
    }
}
