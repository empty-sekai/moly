//! Keyboard and mouse adapters. They emit commands; there is no second editor.

use super::{EditCommand, EditPhase, EditSession, assets::CANDIDATES, presentation::DraftPreview};
use bevy::{
    input::mouse::AccumulatedMouseMotion,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
    window::PrimaryWindow,
};
use moly_law::fixture::{GridPosition, position::TILE_SIZE};

pub(crate) fn read_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<EditSession>,
    mut actions: MessageWriter<EditCommand>,
) {
    if keys.just_pressed(KeyCode::KeyE) {
        actions.write(if session.phase == EditPhase::Idle {
            EditCommand::Enter
        } else {
            EditCommand::RequestExit
        });
    }
    if session.phase == EditPhase::Idle {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        actions.write(if session.exit_dialog {
            EditCommand::KeepEditing
        } else if session.selected.is_some() {
            EditCommand::Cancel
        } else {
            EditCommand::RequestExit
        });
    }
    if session.exit_dialog {
        if keys.just_pressed(KeyCode::KeyS) {
            actions.write(EditCommand::SaveAndExit);
        }
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        actions.write(EditCommand::SelectCatalog {
            index: session.catalog_index,
        });
    }
    let delta = i32::from(keys.just_pressed(KeyCode::BracketRight))
        - i32::from(keys.just_pressed(KeyCode::BracketLeft));
    if delta != 0 {
        let index =
            (session.catalog_index as i32 + delta).rem_euclid(CANDIDATES.len() as i32) as usize;
        actions.write(EditCommand::SelectCatalog { index });
    }
    if keys.just_pressed(KeyCode::KeyR) {
        actions.write(EditCommand::Rotate);
    }
    if keys.just_pressed(KeyCode::KeyD) || keys.just_pressed(KeyCode::Enter) {
        actions.write(EditCommand::Decide);
    }
    if keys.just_pressed(KeyCode::KeyS) {
        // The desktop shortcut invokes the same action as the source Save button.
        actions.write(EditCommand::SaveAndExit);
    }
    if keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace) {
        actions.write(EditCommand::ReturnToInventory);
    }
    let x = i8::from(keys.just_pressed(KeyCode::ArrowRight))
        - i8::from(keys.just_pressed(KeyCode::ArrowLeft));
    let z = i8::from(keys.just_pressed(KeyCode::ArrowDown))
        - i8::from(keys.just_pressed(KeyCode::ArrowUp));
    if x != 0 || z != 0 {
        actions.write(EditCommand::Nudge { x, z });
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_pointer(
    session: Res<EditSession>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    site_origins: Query<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>,
    meshes: Res<Assets<Mesh>>,
    parts: Query<(
        Entity,
        &Mesh3d,
        &GlobalTransform,
        Option<&InheritedVisibility>,
    )>,
    hierarchy: Query<(
        Option<&ChildOf>,
        Option<&moly_assets::scene_state::SourceInactive>,
        Option<&Visibility>,
    )>,
    roots: Query<
        &crate::fixture_scene_inputs::FixtureScenePlacement,
        With<crate::fixture::FixtureRoot>,
    >,
    previews: Query<&DraftPreview>,
    ui: crate::ui_layout::PointerUi,
    node_interactions: Query<&Interaction>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>,
    mut actions: MessageWriter<EditCommand>,
) {
    if session.phase == EditPhase::Idle || session.exit_dialog || consumed.0 {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    // Covers source-prefab UI and the product-owned Bevy settings/editor UI.
    if ui.captures(cursor, Vec2::new(window.width(), window.height()))
        || node_interactions
            .iter()
            .any(|state| *state != Interaction::None)
    {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor) else {
        return;
    };
    if session.selected.is_some()
        && buttons.pressed(MouseButton::Left)
        && motion.delta != Vec2::ZERO
    {
        let Ok(origin) = site_origins.single() else {
            return;
        };
        if let Some(center) = ray_to_grid(ray, origin.translation()) {
            actions.write(EditCommand::MoveTo { center });
            consumed.0 = true;
        }
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let mut nearest: Option<(f32, String)> = None;
    for (entity, handle, transform, visible) in &parts {
        if visible.is_some_and(|visible| !visible.get()) {
            continue;
        }
        let mut current = entity;
        let mut uid = None;
        loop {
            let Ok((parent, inactive, visibility)) = hierarchy.get(current) else {
                break;
            };
            if inactive.is_some() || visibility.is_some_and(|v| *v == Visibility::Hidden) {
                break;
            }
            if let Ok(row) = roots.get(current) {
                uid = Some(row.0.uid.clone());
                break;
            }
            if let Ok(row) = previews.get(current) {
                uid = Some(row.uid.clone());
                break;
            }
            let Some(parent) = parent else {
                break;
            };
            current = parent.parent();
        }
        let Some(uid) = uid else {
            continue;
        };
        if !session.rows.iter().any(|row| row.uid == uid) {
            continue;
        }
        let Some(mesh) = meshes.get(&handle.0) else {
            continue;
        };
        if let Some(distance) = hit_mesh(ray, transform, mesh) {
            if nearest
                .as_ref()
                .is_none_or(|(previous, _)| distance < *previous)
            {
                nearest = Some((distance, uid));
            }
        }
    }
    if let Some((_, uid)) = nearest {
        actions.write(EditCommand::SelectPlaced { uid });
        consumed.0 = true;
    }
}

fn ray_to_grid(ray: Ray3d, origin: Vec3) -> Option<GridPosition> {
    if ray.direction.y.abs() < f32::EPSILON {
        return None;
    }
    let t = (origin.y - ray.origin.y) / ray.direction.y;
    if t < 0.0 {
        return None;
    }
    let point = ray.origin + *ray.direction * t - origin;
    let x = (point.x / TILE_SIZE).floor();
    let z = (point.z / TILE_SIZE).floor();
    if !x.is_finite()
        || !z.is_finite()
        || x < i8::MIN as f32
        || x > i8::MAX as f32
        || z < i8::MIN as f32
        || z > i8::MAX as f32
    {
        return None;
    }
    Some(GridPosition::new(x as i8, 0, z as i8))
}

/// PC host pick adapter: exact source mesh triangles and their actual root UID.
/// No package-nearest, grid rounding, cylinder or infinite-height hit proxy.
/// This reads mesh bind-pose positions; fully skinned collider tracking remains
/// a named input-adapter gap. The UID list provides an exact selection fallback.
fn hit_mesh(ray: Ray3d, transform: &GlobalTransform, mesh: &Mesh) -> Option<f32> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let Some(VertexAttributeValues::Float32x3(vertices)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let inverse = transform.affine().inverse();
    let origin = inverse.transform_point3(ray.origin);
    let direction = inverse.transform_vector3(*ray.direction);
    if !origin.is_finite() || !direction.is_finite() {
        return None;
    }
    let index_count = mesh.indices().map_or(vertices.len(), Indices::len);
    let index = |i: usize| -> Option<usize> {
        match mesh.indices() {
            Some(Indices::U16(values)) => values.get(i).map(|value| *value as usize),
            Some(Indices::U32(values)) => values.get(i).map(|value| *value as usize),
            None => Some(i),
        }
    };
    let mut nearest: Option<f32> = None;
    for start in (0..index_count.saturating_sub(2)).step_by(3) {
        let (Some(a), Some(b), Some(c)) = (index(start), index(start + 1), index(start + 2)) else {
            continue;
        };
        let (Some(a), Some(b), Some(c)) = (vertices.get(a), vertices.get(b), vertices.get(c))
        else {
            continue;
        };
        let edge1 = Vec3::from(*b) - Vec3::from(*a);
        let edge2 = Vec3::from(*c) - Vec3::from(*a);
        let p = direction.cross(edge2);
        let determinant = edge1.dot(p);
        if determinant.abs() < 1e-8 {
            continue;
        }
        let inverse_determinant = determinant.recip();
        let t = origin - Vec3::from(*a);
        let u = t.dot(p) * inverse_determinant;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = t.cross(edge1);
        let v = direction.dot(q) * inverse_determinant;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let distance = edge2.dot(q) * inverse_determinant;
        if distance >= 0.0 && nearest.is_none_or(|old| distance < old) {
            nearest = Some(distance);
        }
    }
    nearest
}
