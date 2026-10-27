//! A verified RaycastTargetGroup collider input, not an infinite ground plane.
use crate::fixture_edit::{EditCommand, EditContentType, EditSession};
use crate::gesture::{GestureEvent, GestureKind, GestureState, UiPointerEvent, UiPointerPhase};
use bevy::{asset::LoadState, prelude::*, window::PrimaryWindow};
use moly_assets::json::JsonAsset;
use serde_json::Value;

#[derive(Resource)]
struct PickSource {
    handle: Option<Handle<JsonAsset>>,
    triangles: Vec<[Vec3; 3]>,
    error: Option<String>,
}

pub(super) fn install(app: &mut App) {
    app.add_systems(
        Startup,
        |mut commands: Commands, server: Res<AssetServer>| {
            commands.insert_resource(PickSource {
                handle: Some(server.load("moly://site/edit-pick/home.json")),
                triangles: Vec::new(),
                error: None,
            });
        },
    )
    .add_systems(Update, read_source)
    .add_systems(
        Update,
        cancel_pointer
            .in_set(super::super::FixtureEditSystems::Pointer)
            .after(crate::gesture::advance),
    )
    .add_systems(
        Update,
        input
            .in_set(super::super::FixtureEditSystems::Pointer)
            .after(crate::gesture::advance)
            .run_if(crate::game_settings::scene_input_enabled),
    );
}

/// This consumer must run while a modal disables new scene input: a long
/// touch emits no Gesture End when the host loses the pointer.
fn cancel_pointer(
    mut pointers: MessageReader<UiPointerEvent>,
    mut actions: MessageWriter<EditCommand>,
) {
    let mut cancelled = false;
    for event in pointers.read() {
        cancelled |= event.phase == UiPointerPhase::Cancel;
    }
    if cancelled {
        actions.write(EditCommand::RoadPointerCancel);
    }
}

fn vec(value: &Value, count: usize) -> Result<Vec<f32>, String> {
    let v = value
        .as_array()
        .filter(|v| v.len() == count)
        .ok_or("road pick vector length")?;
    v.iter()
        .map(|n| {
            n.as_f64()
                .filter(|n| n.is_finite())
                .map(|n| n as f32)
                .filter(|n| n.is_finite())
                .ok_or("road pick vector number".into())
        })
        .collect()
}

fn parse(text: &str) -> Result<Vec<[Vec3; 3]>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if doc["version"] != 1
        || doc["coordinateContract"] != "unity-lh-y-up-authored"
        || doc["site"] != "home"
    {
        return Err("road pick source contract differs".into());
    }
    let mut result = Vec::new();
    for collider in doc["colliders"]
        .as_array()
        .ok_or("road pick collider list")?
    {
        if collider["tag"] != "RaycastTargetGroup" || collider["convex"] != false {
            return Err("road pick needs exact tagged triangle collider".into());
        }
        if collider["enabled"] != true {
            continue;
        }
        let mut pose = Mat4::IDENTITY;
        let mut active = true;
        for item in collider["chain"]
            .as_array()
            .ok_or("road pick transform chain")?
        {
            active &= item["active"] == true;
            let p = vec(&item["position"], 3)?;
            let r = vec(&item["rotation"], 4)?;
            let s = vec(&item["scale"], 3)?;
            pose *= Mat4::from_scale_rotation_translation(
                Vec3::new(s[0], s[1], s[2]),
                Quat::from_xyzw(r[0], r[1], r[2], r[3]),
                Vec3::new(p[0], p[1], p[2]),
            );
        }
        if !active {
            continue;
        }
        let points: Vec<_> = collider["positions"]
            .as_array()
            .ok_or("road pick positions")?
            .iter()
            .map(|value| {
                let v = vec(value, 3)?;
                Ok(pose.transform_point3(Vec3::new(v[0], v[1], v[2])))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for tri in collider["triangles"]
            .as_array()
            .ok_or("road pick triangles")?
        {
            let tri = tri
                .as_array()
                .filter(|row| row.len() == 3)
                .ok_or("road pick triangle")?;
            let mut row = [Vec3::ZERO; 3];
            for i in 0..3 {
                row[i] = *points
                    .get(
                        tri[i]
                            .as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or("road pick index")?,
                    )
                    .ok_or("road pick index range")?;
            }
            result.push(row);
        }
    }
    if result.is_empty() {
        return Err("road pick source has no active tagged triangles".into());
    }
    Ok(result)
}

fn read_source(
    mut pick: ResMut<PickSource>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
) {
    let Some(handle) = pick.handle.clone() else {
        return;
    };
    let result = match server.load_state(&handle) {
        LoadState::Failed(error) => Err(format!("road pick source missing: {error}")),
        _ => {
            let Some(asset) = json.get(&handle) else {
                return;
            };
            parse(&asset.0)
        }
    };
    match result {
        Ok(triangles) => pick.triangles = triangles,
        Err(error) => {
            warn!("[road-pick] {error}");
            pick.error = Some(error);
        }
    }
    pick.handle = None;
}

fn nearest(ray: Ray3d, triangles: &[[Vec3; 3]]) -> Option<Vec3> {
    let mut nearest = f32::INFINITY;
    for &[a, b, c] in triangles {
        let ab = b - a;
        let ac = c - a;
        let cross = ray.direction.cross(ac);
        let det = ab.dot(cross);
        // Physics.queriesHitBackfaces is false: retain the authored front face.
        if det <= f32::EPSILON {
            continue;
        }
        let inv = det.recip();
        let offset = ray.origin - a;
        let u = offset.dot(cross) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = offset.cross(ab);
        let v = ray.direction.dot(q) * inv;
        if v < 0. || u + v > 1. {
            continue;
        }
        let t = ac.dot(q) * inv;
        if t >= 0. {
            nearest = nearest.min(t);
        }
    }
    nearest
        .is_finite()
        .then(|| ray.origin + *ray.direction * nearest)
}

/// FixtureView.SetupTouchCollider creates this box under the live view:
/// x/z = current grid extent * .25 + HALF_TILE_SIZE.x; height .125.
/// Its center is half the height above the view's local origin.
fn touch_box(
    ray: Ray3d,
    transform: &GlobalTransform,
    size: moly_law::fixture::Vector3Int,
) -> Option<f32> {
    let inverse = transform.affine().inverse();
    let origin = inverse.transform_point3(ray.origin);
    let direction = inverse.transform_vector3(*ray.direction);
    let half = Vec3::new(
        size.x as f32 * 0.25 + 0.125,
        0.125,
        size.z as f32 * 0.25 + 0.125,
    ) * 0.5;
    let center = Vec3::Y * half.y;
    let mut near = 0f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let (min, max) = (center[axis] - half[axis], center[axis] + half[axis]);
        if direction[axis].abs() <= f32::EPSILON {
            if origin[axis] < min || origin[axis] > max {
                return None;
            }
        } else {
            let a = (min - origin[axis]) / direction[axis];
            let b = (max - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    (far >= 0. && near.is_finite()).then_some(near)
}

#[allow(clippy::too_many_arguments)]
fn input(
    mut gestures: MessageReader<GestureEvent>,
    session: Res<EditSession>,
    source: Res<PickSource>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    origins: Query<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>,
    fixtures: Query<
        (
            Option<&crate::fixture_scene_inputs::FixtureScenePlacement>,
            Option<&super::super::presentation::DraftPreview>,
        ),
        With<crate::fixture::FixtureVisualRoot>,
    >,
    source_views: Query<(
        Entity,
        &GlobalTransform,
        &InheritedVisibility,
        &bevy::gltf::GltfExtras,
    )>,
    parents: Query<&ChildOf>,
    mut actions: MessageWriter<EditCommand>,
) {
    let events: Vec<_> = gestures.read().copied().collect();
    if session.content != EditContentType::Road
        || session.phase == super::super::EditPhase::Idle
        || session.exit_dialog
        || session.clean_up_dialog
        || session.saving.is_some()
    {
        return;
    }
    let (Ok(window), Ok((camera, transform)), Ok(origin)) =
        (windows.single(), cameras.single(), origins.single())
    else {
        return;
    };
    if window.width() <= 0. || window.height() <= 0. {
        return;
    }
    for event in events {
        if event.ui_owned {
            continue;
        }
        if event.kind == GestureKind::LongTouch && event.state == GestureState::End {
            actions.write(EditCommand::RoadLongEnd);
            continue;
        }
        if event.kind == GestureKind::Drag && event.state == GestureState::End {
            actions.write(EditCommand::RoadDragEnd);
            continue;
        }
        let Ok(ray) = camera.viewport_to_world(transform, event.position) else {
            continue;
        };
        let p = ray.origin - origin.translation();
        let d = *ray.direction;
        let Ok(dir) = Dir3::new(Vec3::new(-d.x, d.y, d.z)) else {
            continue;
        };
        let source_ray = Ray3d::new(Vec3::new(-p.x, p.y, p.z), dir);
        let Some(hit) = nearest(source_ray, &source.triangles) else {
            continue;
        };
        let Some(center) = moly_law::fixture::road_edit::field_hit_grid(hit.to_array(), [0.25; 3])
        else {
            continue;
        };
        let Some(raw_center) =
            moly_law::fixture::road_edit::raw_field_hit_grid(hit.to_array(), [0.25; 3])
        else {
            continue;
        };
        let uid = || {
            let mut nearest: Option<(f32, String)> = None;
            for (entity, pose, visible, extras) in &source_views {
                if !visible.get() {
                    continue;
                }
                let Ok(meta) = serde_json::from_str::<Value>(&extras.value) else {
                    continue;
                };
                if meta["fixtureViewRoot"] != true {
                    continue;
                }
                let mut root = entity;
                let (placed, preview) = loop {
                    if let Ok(row) = fixtures.get(root) {
                        break row;
                    }
                    let Ok(parent) = parents.get(root) else {
                        break (None, None);
                    };
                    root = parent.parent();
                };
                let id = preview
                    .map(|value| value.uid.as_str())
                    .or_else(|| placed.map(|value| value.0.uid.as_str()));
                let Some(id) = id else {
                    continue;
                };
                let Some(row) = session
                    .road
                    .planned
                    .iter()
                    .chain(&session.rows)
                    .find(|row| {
                        row.uid == id
                            && row.layout == moly_law::fixture::position::layout_type::ROAD
                    })
                else {
                    continue;
                };
                if let Some(distance) = touch_box(ray, pose, row.grid_size) {
                    if nearest.as_ref().is_none_or(|(old, _)| distance < *old) {
                        nearest = Some((distance, id.to_owned()));
                    }
                }
            }
            nearest.map(|(_, id)| id)
        };
        let command = match (event.kind, event.state) {
            (GestureKind::Tap | GestureKind::DoubleTap, GestureState::End) => {
                EditCommand::RoadTap { center }
            }
            (GestureKind::LongTouch, GestureState::Began) => EditCommand::RoadLongStart {
                center: raw_center,
                uid: uid(),
            },
            (GestureKind::Drag, GestureState::Began) => {
                EditCommand::RoadDragStart { center, uid: uid() }
            }
            (GestureKind::Drag, GestureState::Moved) => {
                EditCommand::RoadDragMove { center, raw_center }
            }
            _ => continue,
        };
        actions.write(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_pointer_cancel_is_read_without_enabling_scene_input() {
        let mut app = App::new();
        app.add_message::<UiPointerEvent>();
        app.add_message::<EditCommand>();
        app.add_systems(Update, cancel_pointer);
        app.world_mut().write_message(UiPointerEvent {
            phase: UiPointerPhase::Cancel,
            position: Vec2::ZERO,
            pointer_id: -1,
            touch_count: 0,
        });
        app.update();
        let messages = app.world().resource::<Messages<EditCommand>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(
            cursor
                .read(messages)
                .filter(|command| matches!(command, EditCommand::RoadPointerCancel))
                .count(),
            1
        );
    }

    #[test]
    fn triangle_picker_keeps_authored_front_faces_and_nearest_hit() {
        let triangles = [
            [
                Vec3::new(-1., 0., -1.),
                Vec3::new(0., 0., 1.),
                Vec3::new(1., 0., -1.),
            ],
            [
                Vec3::new(-1., -1., -1.),
                Vec3::new(0., -1., 1.),
                Vec3::new(1., -1., -1.),
            ],
        ];
        assert_eq!(
            nearest(Ray3d::new(Vec3::Y, Dir3::NEG_Y), &triangles),
            Some(Vec3::ZERO)
        );
        assert!(nearest(Ray3d::new(Vec3::NEG_Y * 2., Dir3::Y), &triangles).is_none());
        assert!(nearest(Ray3d::new(Vec3::new(2., 1., 0.), Dir3::NEG_Y), &triangles).is_none());
    }

    #[test]
    fn touch_box_uses_source_padding_and_height() {
        let size = moly_law::fixture::Vector3Int::new(2, 1, 2);
        let pose = GlobalTransform::IDENTITY;
        assert_eq!(
            touch_box(Ray3d::new(Vec3::new(0.3, 1., 0.), Dir3::NEG_Y), &pose, size),
            Some(0.875)
        );
        assert!(
            touch_box(
                Ray3d::new(Vec3::new(0.32, 1., 0.), Dir3::NEG_Y),
                &pose,
                size
            )
            .is_none()
        );
    }
}
