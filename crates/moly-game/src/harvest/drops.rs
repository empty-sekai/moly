//! `HarvestObjectPresenter.CreateDropItem` and the drop hop.
//!
//! A batch plays `PlayDropItemSE` once, then each row becomes a drop (one a
//! frame when the batch reached `HarvestDropDelayItemCount`). Per item
//! (`HarvestUtility.CreateDropItem`): two draws, an angle in [0, 2 pi) and a
//! radius in [min, max] of the row's scatter arm; the search position is the
//! fixture's (x, 0, z) plus (cos a, height, sin a) times the radius; a
//! scattered item hops there (`PlayHarvestDropAnimationAsync`: x/z DOMove
//! 0.6 s OutCubic, y 0.18 OutQuad up / 0.18 InQuad down / 0.12 OutQuad up to
//! `y + d2 * d1 * r * 0.43` / 0.12 InQuad down, two more draws: yaw and r in
//! [0.6, 0.8]) and takes radius 1.0 once landed; an unscattered item has
//! radius 1.0 at once. The landing height stands in for the source's
//! raycast with the ground-vertex sample.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;

use super::{
    drop_scatter_params, DropPrefab, HarvestDocs, HarvestDropBatches, HarvestDropItem,
    HarvestDropSeq, HarvestGltfs, HarvestGroundVerts, HarvestObject, HarvestStats, PendingDrop,
    Rng,
};
use crate::audio::SeRequests;

const DROP_DURATION: f32 = 0.6;
const DROP_SECOND_HOP: f32 = 0.43;

#[derive(Component)]
pub(crate) struct HarvestDropAnimation {
    elapsed: f32,
    spawn: Vec3,
    landing: Vec3,
    d1: f32,
    d2: f32,
    r: f32,
}

/// `PlayDropItemSE`: a birthday-plant fixture has its own cue; otherwise by
/// the batch's highest rarity, rarity_2 or rarity_3 play the rare cue,
/// rarity_1 and rarity_4 nothing; other values throw.
pub(crate) fn play_drop_item_se(fixture_type: i32, batch: &[PendingDrop], se: &mut SeRequests) {
    if fixture_type == 9 {
        super::damage::push_se(se, "se_drop_birthday_material", "harvest-drop");
        return;
    }
    match batch.iter().map(|drop| drop.rarity).max() {
        Some(1 | 2) => super::damage::push_se(se, "se_drop_rare_material", "harvest-drop"),
        Some(0 | 3) | None => {}
        Some(other) => {
            panic!("drop batch rarity {other} is outside the SE arms (the source throws)")
        }
    }
}

fn ease_in_quad(u: f32) -> f32 {
    u * u
}

fn ease_out_quad(u: f32) -> f32 {
    1.0 - (1.0 - u) * (1.0 - u)
}

fn ease_out_cubic(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

/// Update: consume the drop batches.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    glbs: Res<HarvestGltfs>,
    docs: Res<HarvestDocs>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    ground: Res<HarvestGroundVerts>,
    arrival: Res<super::arrival::HarvestArrival>,
    mut batches: ResMut<HarvestDropBatches>,
    mut objects: Query<&mut HarvestObject>,
    mut seq: ResMut<HarvestDropSeq>,
    mut stats: ResMut<HarvestStats>,
) {
    let Some(verts) = ground.0.as_ref() else {
        return;
    };
    let site_id = arrival.site_id().unwrap_or(0);
    for batch in &mut batches.0 {
        let quota = if batch.delay { 1 } else { usize::MAX };
        let mut taken = 0usize;
        while taken < quota {
            let Some(item) = batch.remaining.first().cloned() else {
                break;
            };
            let package = match &item.prefab {
                DropPrefab::Unresolved(reason) => {
                    stats.drop_refused += 1;
                    info!(
                        "[harvest-drop] refused: {reason} (resourceType {} id {} row hp {} seq {})",
                        item.row.resource_type, item.row.resource_id, item.row.hp, item.row.seq
                    );
                    remove_pending(batch.origin, &item, &mut objects);
                    batch.remaining.remove(0);
                    taken += 1;
                    continue;
                }
                DropPrefab::Model { package } => package.clone(),
            };
            if !glbs.ready(&server, &package) || !docs.ready(&server, &package) {
                break;
            }
            let (Some(gltf), Some(doc)) = (
                glbs.get(&package).and_then(|handle| gltfs.get(handle)),
                docs.0.get(&package).and_then(|handle| json.get(handle)),
            ) else {
                break;
            };
            let leaf = package.rsplit("__").next().unwrap_or(&package).to_owned();
            let scene_index = super::prefab_scene_index(&doc.0, &leaf);
            let scene = gltf.scenes.get(scene_index).cloned().unwrap_or_else(|| {
                panic!("drop model {leaf}: prefab scene {scene_index} out of range")
            });
            seq.0 += 1;
            let uid = seq.0;
            let mut rng = Rng(0xD30D_0000_0000_0000u64.wrapping_add(uid));
            let (min_range, max_range, height) =
                drop_scatter_params(item.row.resource_type, item.material_type);
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let range = min_range + rng.next_f32() * (max_range - min_range);
            let is_scatter = 0.0 < max_range;
            // Source frame: (x + cos a r, height, z + sin a r); x reflected.
            let source_x = batch.position_x as f32 + angle.cos() * range;
            let source_z = batch.position_z as f32 + angle.sin() * range;
            let land_x = -source_x;
            let land_z = source_z;
            let land_y = super::surface_y(verts, land_x, land_z, height);
            let spawn_pos = Vec3::new(-(batch.position_x as f32), 0.0, batch.position_z as f32);
            let landing = Vec3::new(land_x, land_y, land_z);
            let distance = spawn_pos.distance(landing);
            let d1 = distance.clamp(0.6, 1.0);
            let d2 = (1.0 - distance).clamp(0.3, 1.0);
            let yaw_deg = if is_scatter {
                (rng.next_f32() * 360.0).floor()
            } else {
                0.0
            };
            let r = if is_scatter {
                0.6 + rng.next_f32() * 0.2
            } else {
                0.0
            };
            let entity = commands
                .spawn((
                    SceneRoot(scene),
                    HarvestDropItem {
                        uid,
                        radius: if is_scatter { 0.0 } else { 1.0 },
                        rarity: item.rarity,
                        resource_type: item.row.resource_type,
                        resource_id: item.row.resource_id,
                        site_id,
                        row: item.row.clone(),
                        material_type: item.material_type,
                        acceleration: 0.0,
                        attracting: true,
                    },
                    Transform::from_translation(spawn_pos)
                        .with_rotation(Quat::from_rotation_y(yaw_deg * 0.017_453_292)),
                    Visibility::default(),
                ))
                .id();
            if is_scatter {
                commands.entity(entity).insert(HarvestDropAnimation {
                    elapsed: 0.0,
                    spawn: spawn_pos,
                    landing,
                    d1,
                    d2,
                    r,
                });
            }
            stats.drop_items += 1;
            info!(
                "[harvest-drop] spawned {leaf} uid {uid}: resourceType {} id {} row hp {} seq {} qty {} rarity {} at ({:.2}, 0, {:.2}) -> landing ({:.2}, {:.2}, {:.2}) angle {:.1} deg range {:.2}{}",
                item.row.resource_type,
                item.row.resource_id,
                item.row.hp,
                item.row.seq,
                item.row.quantity,
                item.rarity,
                spawn_pos.x,
                spawn_pos.z,
                landing.x,
                landing.y,
                landing.z,
                angle.to_degrees(),
                range,
                if is_scatter {
                    format!(" hop d1 {d1:.3} d2 {d2:.3} r {r:.3}")
                } else {
                    " (no scatter: radius 1.0 at once)".into()
                },
            );
            remove_pending(batch.origin, &item, &mut objects);
            batch.remaining.remove(0);
            taken += 1;
        }
    }
    batches.0.retain(|batch| !batch.remaining.is_empty());
}

/// `RemoveTargetBeforeDropItem` (a miss is a harmless no-op).
fn remove_pending(
    origin: Option<Entity>,
    item: &PendingDrop,
    objects: &mut Query<&mut HarvestObject>,
) {
    let Some(mut object) = origin.and_then(|origin| objects.get_mut(origin).ok()) else {
        return;
    };
    if let Some(index) = object
        .pending_drops
        .iter()
        .position(|drop| drop.row.seq == item.row.seq && drop.row.hp == item.row.hp)
    {
        object.pending_drops.remove(index);
    }
}

/// Update: the drop hop; a landed item takes radius 1.0.
pub(crate) fn advance_animations(
    time: Res<Time>,
    mut commands: Commands,
    mut drops: Query<(
        Entity,
        &mut HarvestDropAnimation,
        &mut Transform,
        &mut HarvestDropItem,
    )>,
) {
    for (entity, mut anim, mut transform, mut item) in &mut drops {
        anim.elapsed += time.delta_secs();
        let t = anim.elapsed;
        if t >= DROP_DURATION {
            transform.translation = anim.landing;
            item.radius = 1.0;
            commands.entity(entity).remove::<HarvestDropAnimation>();
            continue;
        }
        let y_land = anim.landing.y;
        let first_peak = y_land + anim.d1 * anim.r;
        let second_peak = y_land + anim.d2 * anim.d1 * anim.r * DROP_SECOND_HOP;
        let y = if t < 0.18 {
            anim.spawn.y + (first_peak - anim.spawn.y) * ease_out_quad(t / 0.18)
        } else if t < 0.36 {
            first_peak + (y_land - first_peak) * ease_in_quad((t - 0.18) / 0.18)
        } else if t < 0.48 {
            y_land + (second_peak - y_land) * ease_out_quad((t - 0.36) / 0.12)
        } else {
            second_peak + (y_land - second_peak) * ease_in_quad((t - 0.48) / 0.12)
        };
        let u = ease_out_cubic(t / DROP_DURATION);
        transform.translation = Vec3::new(
            anim.spawn.x + (anim.landing.x - anim.spawn.x) * u,
            y,
            anim.spawn.z + (anim.landing.z - anim.spawn.z) * u,
        );
    }
}
