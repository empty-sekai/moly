//! Instance-owned navigation metadata, kept separate from render visibility.

use bevy::prelude::*;
use serde_json::Value;

#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct SourceObjectIdentity {
    pub file: String,
    pub game_object: i64,
    pub transform: i64,
    pub components: Vec<i64>,
    pub child_order: Vec<i64>,
}

#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct SourceNavMeshObstacle {
    pub component_id: i64,
    pub enabled: bool,
    pub shape: u8,
    /// Authored Unity-local values; conversion happens at the world boundary.
    pub center: Vec3,
    pub extents: Vec3,
    pub carve: bool,
    pub only_stationary: bool,
    pub move_threshold: f32,
    pub stationary_time: f32,
}

#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct SourceNavMeshObstacles(pub Vec<SourceNavMeshObstacle>);

#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct SourceHarvestView {
    pub class: String,
    pub component_id: i64,
    // Reflectable storage is deliberate: ignored fields would be lost when a
    // glTF Scene is cloned. Consumers parse this once when binding the view.
    pub fields_json: String,
}

pub(crate) fn import(extras: &Value, entity: &mut EntityWorldMut) {
    if let Some(source) = extras.get("sourceObject") {
        entity.insert(SourceObjectIdentity {
            file: source["file"]
                .as_str()
                .expect("source object file identity")
                .into(),
            game_object: identity(&source["gameObjectId"]),
            transform: identity(&source["transformId"]),
            components: extras["sourceComponentIds"]
                .as_array()
                .expect("source component identities")
                .iter()
                .map(identity)
                .collect(),
            child_order: extras["sourceChildOrder"]
                .as_array()
                .expect("source child order")
                .iter()
                .map(identity)
                .collect(),
        });
    }
    if let Some(obstacles) = extras.get("navMeshObstacles") {
        let obstacles = obstacles
            .as_array()
            .expect("source navigation obstacles")
            .iter()
            .map(|row| {
                let shape = row["shape"]
                    .as_u64()
                    .filter(|shape| *shape <= 1)
                    .expect("source navigation shape must be Capsule or Box")
                    as u8;
                SourceNavMeshObstacle {
                    component_id: identity(&row["componentId"]),
                    enabled: boolean(&row["enabled"]),
                    shape,
                    center: vector(&row["center"]),
                    extents: vector(&row["extents"]),
                    carve: boolean(&row["carve"]),
                    only_stationary: boolean(&row["onlyStationary"]),
                    move_threshold: number(&row["moveThreshold"]),
                    stationary_time: number(&row["stationaryTime"]),
                }
            })
            .collect();
        entity.insert(SourceNavMeshObstacles(obstacles));
    }
    if let Some(view) = extras.get("harvestView") {
        entity.insert(SourceHarvestView {
            class: view["class"].as_str().expect("harvest view class").into(),
            component_id: identity(&view["componentId"]),
            fields_json: view.get("fields").expect("harvest view fields").to_string(),
        });
    }
}

fn identity(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|value| value.parse().ok())
        .expect("source signed identity")
}

fn boolean(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) if value.as_u64() == Some(0) => false,
        Value::Number(value) if value.as_u64() == Some(1) => true,
        _ => panic!("source navigation boolean"),
    }
}

fn number(value: &Value) -> f32 {
    value
        .as_f64()
        .map(|number| number as f32)
        .filter(|number| number.is_finite())
        .expect("finite source navigation number")
}

fn vector(value: &Value) -> Vec3 {
    Vec3::new(
        number(&value["x"]),
        number(&value["y"]),
        number(&value["z"]),
    )
}

pub(crate) fn register(app: &mut App) {
    app.register_type::<SourceObjectIdentity>()
        .register_type::<SourceNavMeshObstacle>()
        .register_type::<SourceNavMeshObstacles>()
        .register_type::<SourceHarvestView>();
}
