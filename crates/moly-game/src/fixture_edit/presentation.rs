//! A reversible view of the private editor draft. This module owns temporary
//! poses/previews only; it never commits layouts, inventory or gameplay leases.

use super::{EditPhase, EditSession, PutStatus, assets::CandidateAssets, validation};
use crate::fixture::{EditableFixture, FixtureRoot};
use crate::fixture_scene_inputs::FixtureScenePlacement;
use bevy::{gltf::Gltf, prelude::*};
use moly_law::fixture::position::TILE_SIZE;
use std::collections::{HashMap, HashSet};

#[derive(Component, Clone)]
struct OriginalPose {
    transform: Transform,
    visibility: Visibility,
}

#[derive(Component)]
pub(super) struct DraftPreview {
    pub uid: String,
}

#[derive(Component)]
struct TileMarker;

#[derive(Resource)]
struct TileAssets {
    mesh: Handle<Mesh>,
    valid: Handle<StandardMaterial>,
    invalid: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct ProjectionStamp {
    revision: u64,
    roots: usize,
    waiting_assets: bool,
}

/// Source Reset restores the selected start pose, while explicit discard
/// reloads the saved layout. The committed record was never changed, so both
/// paths can remove only the editor's own overlays and restore captured poses.
pub(super) fn clear(world: &mut World) {
    world.remove_resource::<ProjectionStamp>();
    let originals: Vec<_> = world
        .query::<(Entity, &OriginalPose)>()
        .iter(world)
        .map(|(entity, pose)| (entity, pose.clone()))
        .collect();
    for (entity, pose) in originals {
        if let Ok(mut entity) = world.get_entity_mut(entity) {
            entity
                .insert((pose.transform, pose.visibility))
                .remove::<OriginalPose>();
        }
    }
    let previews: Vec<_> = world
        .query_filtered::<Entity, Or<(With<DraftPreview>, With<TileMarker>)>>()
        .iter(world)
        .collect();
    for entity in previews {
        world.despawn(entity);
    }
}

pub(super) fn sync_visuals(world: &mut World) {
    let Some(session) = world.get_resource::<EditSession>() else {
        return;
    };
    if session.phase == EditPhase::Idle {
        return;
    }
    let revision = session.revision;
    // A successful save hands scene creation back to the normal loader. Do
    // not capture its intentionally-hidden not-yet-material-swapped roots as
    // the editor's original visibility, or later moves would hide them again.
    if !world.contains_resource::<crate::fixture::FixtureScenesReady>()
        || (world
            .get_resource::<crate::fixture::FixturePlacements>()
            .is_some_and(|layout| layout.total() != 0)
            && !world.contains_resource::<crate::fixture_material::FixtureMaterialsSwapped>())
    {
        return;
    }
    let root_count = world
        .query_filtered::<Entity, With<FixtureRoot>>()
        .iter(world)
        .count();
    if world.get_resource::<ProjectionStamp>().is_some_and(|old| {
        old.revision == revision && old.roots == root_count && !old.waiting_assets
    }) {
        return;
    }
    let session = world.resource::<EditSession>();
    let rows = session.rows.clone();
    let selected = session
        .selected
        .as_ref()
        .map(|selected| selected.item.clone());
    let floor = session
        .baseline
        .as_ref()
        .and_then(crate::fixture::FixturePlacements::floor_grid);
    let by_uid: HashMap<_, _> = rows.iter().map(|row| (row.uid.as_str(), row)).collect();
    let roots: Vec<_> = world
        .query_filtered::<(
            Entity,
            &FixtureScenePlacement,
            &Transform,
            &Visibility,
            Option<&OriginalPose>,
        ), With<FixtureRoot>>()
        .iter(world)
        .map(|(entity, row, pose, visibility, original)| {
            (
                entity,
                row.0.uid.clone(),
                *pose,
                *visibility,
                original.cloned(),
            )
        })
        .collect();
    let mut original_uids = HashSet::new();
    for (entity, uid, pose, visibility, original) in roots {
        original_uids.insert(uid.clone());
        let origin = original.clone().unwrap_or(OriginalPose {
            transform: pose,
            visibility,
        });
        let item = selected
            .as_ref()
            .filter(|item| item.uid == uid)
            .or_else(|| by_uid.get(uid.as_str()).copied());
        let target_visibility = if item.is_some() {
            origin.visibility
        } else {
            Visibility::Hidden
        };
        if original.is_none() {
            world.entity_mut(entity).insert(origin);
        }
        if visibility != target_visibility {
            world.entity_mut(entity).insert(target_visibility);
        }
        if let Some(target) = item.and_then(|item| item.pose().ok()) {
            if pose.translation != target.translation || pose.rotation != target.rotation {
                // Keep source-authored scale. Only the layout-owned position
                // and yaw are overlaid; child animation data remains untouched.
                let mut transform = world.get_mut::<Transform>(entity).expect("root transform");
                transform.translation = target.translation;
                transform.rotation = target.rotation;
            }
        }
    }
    let mut transient: Vec<_> = rows
        .iter()
        .filter(|row| !original_uids.contains(&row.uid))
        .cloned()
        .collect();
    if let Some(selected) = &selected {
        if !original_uids.contains(&selected.uid) {
            if let Some(row) = transient.iter_mut().find(|row| row.uid == selected.uid) {
                *row = selected.clone();
            } else {
                transient.push(selected.clone());
            }
        }
    }
    let waiting_assets = !sync_previews(world, &transient);
    sync_marker(world, selected.as_ref(), &rows, floor);
    world.insert_resource(ProjectionStamp {
        revision,
        roots: root_count,
        waiting_assets,
    });
}

fn sync_previews(world: &mut World, rows: &[EditableFixture]) -> bool {
    let mut ready = true;
    let previews: Vec<_> = world
        .query::<(Entity, &DraftPreview)>()
        .iter(world)
        .map(|(entity, preview)| (entity, preview.uid.clone()))
        .collect();
    for (entity, uid) in &previews {
        if !rows.iter().any(|row| &row.uid == uid) {
            world.despawn(*entity);
        }
    }
    for row in rows {
        let Ok(pose) = row.pose() else {
            continue;
        };
        if let Some((entity, _)) = previews.iter().find(|(_, uid)| *uid == row.uid) {
            if let Some(mut transform) = world.get_mut::<Transform>(*entity) {
                if transform.translation != pose.translation || transform.rotation != pose.rotation
                {
                    *transform = pose;
                }
            }
            continue;
        }
        let handle = world.resource_scope(|world, mut candidates: Mut<CandidateAssets>| {
            if let Some(handle) = candidates.glbs.get(&row.package) {
                return Some(handle.clone());
            }
            let path = candidates.paths.get(&row.package)?.clone();
            let handle = world.resource::<AssetServer>().load::<Gltf>(path);
            candidates.glbs.insert(row.package.clone(), handle.clone());
            Some(handle)
        });
        let Some(handle) = handle else {
            ready = false;
            continue;
        };
        if !world
            .resource::<AssetServer>()
            .is_loaded_with_dependencies(&handle)
        {
            ready = false;
            continue;
        }
        let Some(scene) = world
            .resource::<Assets<Gltf>>()
            .get(&handle)
            .and_then(|gltf| gltf.default_scene.clone())
        else {
            ready = false;
            continue;
        };
        // Share source material preparation, but not committed gameplay roles.
        // The material owner reveals this root after the scene is expanded and
        // all source replacements are ready, exactly as for saved furniture.
        world.spawn((
            SceneRoot(scene),
            crate::fixture::FixtureVisualRoot { layout: row.layout },
            crate::fixture_colors::FixtureColorChoice { package: row.package.clone(), texture_id: row.texture_id },
            DraftPreview {
                uid: row.uid.clone(),
            },
            pose,
            Visibility::Hidden,
        ));
    }
    ready
}

fn sync_marker(
    world: &mut World,
    selected: Option<&EditableFixture>,
    rows: &[EditableFixture],
    floor: Option<crate::site::FloorGridLayout>,
) {
    let markers: Vec<_> = world
        .query_filtered::<Entity, With<TileMarker>>()
        .iter(world)
        .collect();
    let Some(selected) = selected else {
        for entity in markers {
            world.despawn(entity);
        }
        return;
    };
    if !world.contains_resource::<TileAssets>() {
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(Plane3d::default().mesh().size(1.0, 1.0));
        let material = |color| StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..Default::default()
        };
        let valid = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material(Color::srgba(0.2, 0.8, 0.3, 0.35)));
        let invalid = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material(Color::srgba(0.9, 0.2, 0.15, 0.35)));
        world.insert_resource(TileAssets {
            mesh,
            valid,
            invalid,
        });
    }
    let (Ok(pose), Ok((min, max))) = (selected.pose(), selected.footprint()) else {
        for entity in markers {
            world.despawn(entity);
        }
        return;
    };
    let transform =
        Transform::from_translation(pose.translation + Vec3::Y * 0.01).with_scale(Vec3::new(
            (max.x as i32 - min.x as i32 + 1) as f32 * TILE_SIZE,
            1.0,
            (max.z as i32 - min.z as i32 + 1) as f32 * TILE_SIZE,
        ));
    let status = validation::put(selected, rows, floor);
    let assets = world.resource::<TileAssets>();
    let mesh = assets.mesh.clone();
    let material = if status == PutStatus::Ok {
        assets.valid.clone()
    } else {
        assets.invalid.clone()
    };
    if let Some(entity) = markers.first() {
        if world
            .get::<Transform>(*entity)
            .is_some_and(|old| *old != transform)
        {
            world.entity_mut(*entity).insert(transform);
        }
        if world
            .get::<MeshMaterial3d<StandardMaterial>>(*entity)
            .is_none_or(|old| old.0 != material)
        {
            world.entity_mut(*entity).insert(MeshMaterial3d(material));
        }
    } else {
        world.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            TileMarker,
            transform,
            Visibility::Visible,
        ));
    }
}
