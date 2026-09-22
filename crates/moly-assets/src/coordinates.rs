//! The only source-to-runtime spatial basis boundary. See coordinate-contract.md.
use bevy::prelude::*;
use serde_json::Value;

pub const CONTRACT: &str = "moly-rh-y-up-reflect-x-v1";

#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct CanonicalCoordinates;

/// Raw source evidence is converted explicitly, never by inspecting determinant.
pub fn source_position(value: Vec3) -> Vec3 {
    Vec3::new(-value.x, value.y, value.z)
}

pub fn source_rotation(value: Quat) -> Quat {
    Quat::from_xyzw(value.x, -value.y, -value.z, value.w)
}

pub fn validate_document(value: &Value) -> Result<(), String> {
    match value.get("coordinateContract").and_then(Value::as_str) {
        Some(CONTRACT) => Ok(()),
        Some(other) => Err(format!("unsupported coordinate contract {other}")),
        None => Err("coordinate contract missing; re-export this resource snapshot".into()),
    }
}

pub(crate) fn import(extras: &Value, entity: &mut EntityWorldMut) {
    if extras.get("coordinateContract").is_some() {
        validate_document(extras).expect("glTF coordinate contract");
        entity.insert(CanonicalCoordinates);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converted_hierarchy_matches_reflected_source_in_every_direction() {
        let local_position = Vec3::new(1.97, 0.35, -0.99);
        let local_rotation = Quat::from_euler(EulerRot::YXZ, 0.72, -0.31, 0.18);
        for direction in 0..4 {
            let source = Transform::from_xyz(2.25, 0.5, -3.75)
                .with_rotation(Quat::from_rotation_y(direction as f32 * std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(0.7, 1.3, 1.1));
            let converted = Transform::from_translation(source_position(source.translation))
                .with_rotation(source_rotation(source.rotation))
                .with_scale(source.scale);
            let expected = source_position(source.transform_point(local_position));
            assert!((converted.transform_point(source_position(local_position)) - expected).length() < 1e-5);
            let composed = source_rotation(source.rotation * local_rotation);
            let individual = source_rotation(source.rotation) * source_rotation(local_rotation);
            assert!(composed.dot(individual).abs() > 0.99999);
        }
    }

    #[test]
    fn reflection_is_an_involution_and_unknown_contracts_are_rejected() {
        let p = Vec3::new(1.0, 2.0, -3.0);
        let q = Quat::from_rotation_y(0.83);
        assert_eq!(source_position(source_position(p)), p);
        assert_eq!(source_rotation(source_rotation(q)), q);
        assert!(validate_document(&serde_json::json!({})).is_err());
        assert!(validate_document(&serde_json::json!({"coordinateContract":"guess"})).is_err());
    }
}
