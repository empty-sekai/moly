//! The reward drops (`OnDropItem`), the unclaimed drops of an arrival
//! (`GenerateUnclaimedDropItemsIfNeededAsync`), gathering and the gather
//! API loop (`ScheduleExecuteSendGatherData`).
//!
//! `OnDropItem(party, withAnimation, isSynchronized)`: a model of the
//! party's reward material (resource type 41, its rarity, quantity 1); when
//! the party is within its drop limit the model joins the synchronized or
//! the unsynchronized list. The landing point (`GetDropPosition`): one draw
//! of the angle in the annulus' bounds, then one of the distance in
//! [min, max], from the place on x/z, snapped to the walk field from the
//! arrival point (`TryGetCanNavmeshTargetPosition`). With the animation the
//! view starts at `ItemDropStartPosition`, plays `PlayDropItemSE` (rarity_2
//! and rarity_3 the rare cue, rarity_1 and rarity_4 nothing, other values
//! throw) and hops to the landing point (`PlayDeliveryDropAnimationAsync`:
//! the harvest hop with the view scaling from 0 to 1 in 0.6 s, linear);
//! without it the view is put at the landing point. The view's radius is 0
//! until then, its serialized radius after; a drop beyond the limit keeps
//! radius 0 and flies to the player (`MoveItemTowardsPlayerAsync`), where it
//! joins the auto-gather list and goes.
//!
//! Gathering: each drop is a collision object of the collision manager; its
//! enter edge (`OnCollisionEnterDropItem`) adds its uid to the collision list
//! and sets state Gather (published); its exit edge removes the uid. Every
//! frame on the delivery site (`OnUpdateGatherDropItem`) each listed drop
//! the user can collect flies to the player (`OnGatherDropItemAsync`:
//! `se_pick_item`, the collection notice, the model leaves the party, the
//! gather stack gains it), and the list clears. The flight: per frame, when
//! the squared distance to the player is below `DropItemApproachDistance`^2
//! it arrives; otherwise its acceleration grows by the frame time and it
//! moves toward the player by `DropItemApproachSpeed` plus that
//! acceleration (`Vector3.MoveTowards`, a per-frame step).
//!
//! The gather loop, from the arrival while the site is on: every
//! `DeliveryGatherAPIInterval` (FloatConfigs 169), in state Gather with a
//! non-empty stack: state InGathering (published), the gather API with a
//! copy of the stack (`contents`: per party of the stack, its gathered
//! count; `crate::server::delivery::put_birthday_party_gather`), those items
//! leave the stack, every party synchronizes, the total reward animation of
//! the reply, then state Gather while the stack still holds items, else Idle
//! (published).
//!
//! `CanCollectResource(dropItem)`: a drop is the party's reward material,
//! `ResourceType.mysekai_material` (41), whose arm of the method's switch is
//! `CanCollectMaterial(resourceId, quantity)`. That answers yes at once when
//! `IsMaterialReceivableOverPossessionLimit`: the material's master row has
//! `MysekaiMaterialType` game_character (4) or birthday_party (7); a missing
//! row is a LogError and a no. Otherwise it sums the quantity of every
//! Gather stack of the harvest user data, the quantity and the user's
//! material possession and compares them with the possession limit of the
//! user's possession level. A no leaves the drop where it is and runs
//! `HarvestUtility.NoticeCollectItem` instead of the gather.
//!
//! `ShowRareDropEffect(rarity)` right after the view's `Init`: rarity_1,
//! rarity_2 and rarity_3 play `_normalDropEffect`, `_rareDropEffect` and
//! `_ultraRareDropEffect` (`ParticleSystem.Play()`, children included) and
//! keep it as the current effect; rarity_4 plays nothing; a larger value
//! throws. The hop's sequence starts with `StopAllDropEffect` (the three
//! stop and clear) and ends with `ResumeCurrentDropEffect`.
//!
//! Named stand-ins and gaps: the landing height is the walk field's height,
//! else the highest ground vertex within 2 m, the harvest drops' stand-in
//! for `PlayDeliveryDropAnimationAsync`'s `Physics.Raycast` down onto the
//! colliders: the product has no ray query on the site's colliders (they
//! are cooked only in the weather collision scene, which answers sphere
//! sweeps). The possession branch of `CanCollectMaterial` is not ported
//! (every party's reward material in the master is birthday_party, so no
//! delivery drop reaches it; one that does is answered yes and logged as
//! WARN). The collection notices (`NoticeCollectItem`, the gather's) are
//! not drawn. The drop effects are not drawn: in the drop documents the
//! three view fields reference one system, `fx_stay_dropitem_birthday_rare_01
//! /root` with five child rows, all drawn with `Mysekai/Effect/UberUnlit`;
//! the prop documents carry no program catalogue for that shader and their
//! rows carry no `useUnscaledTime`, so the site prop particle host (the
//! harvest stay particles) refuses them, and the play is logged here only.
//! The view appears when its package has loaded (it is requested on the
//! arrival).

use bevy::diagnostic::FrameCount;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use moly_law::action_button::inside_circle;
use moly_law::delivery as law;

use super::honor::{RewardOwner, RewardRuns};
use super::site::{DeliveryObjects, DeliverySite};
use super::{publish, DeliveryActionState, DeliveryModel, DeliveryProgress, DropModel};
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::harvest::{HarvestDocs, HarvestGltfs, Rng};
use crate::player::PlayerControlled;
use crate::server::delivery::{ClientBirthdayPartyData, GatherContent};
use crate::site_move::timeline::{delay_seconds, Delay};

/// `DeliveryGatherAPIInterval` (FloatConfigs 169).
pub(crate) const KEY_GATHER_API_INTERVAL: i32 = 169;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    /// `OnGatherDropItemAsync`.
    Gather,
    /// Beyond the drop limit (`MoveItemTowardsPlayerAsync` of `OnDropItem`).
    AutoGather,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DropPhase {
    Hop,
    Rest,
    Fly { flight: Flight, acceleration: f32 },
}

/// A drop view on the delivery site (`MysekaiAreaDropItemView`).
#[derive(Component)]
pub(crate) struct DeliveryDropItem {
    pub(crate) model: DropModel,
    /// The collision radius now.
    pub(crate) radius: f32,
    /// The view's serialized radius.
    view_radius: f32,
    within: bool,
    /// The collision manager's state of this object.
    inside: bool,
    phase: DropPhase,
}

impl DeliveryDropItem {
    pub(crate) fn gathering(&self) -> bool {
        matches!(self.phase, DropPhase::Fly { .. })
    }
}

/// A drop whose model exists and whose view waits for its package.
struct PendingSpawn {
    model: DropModel,
    within: bool,
    with_animation: bool,
    /// The drawn landing point on x/z (product frame) before the snap.
    raw: Vec2,
    yaw_deg: f32,
    r: f32,
}

#[derive(Resource, Default)]
pub(crate) struct DeliveryDropSpawns {
    pending: Vec<PendingSpawn>,
    ground: Option<Vec<Vec3>>,
    /// `RemoveDropItemView`: the drop views to destroy on the next step.
    removed: Vec<u64>,
}

impl DeliveryDropSpawns {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Request a drop package's model and document (the harvest caches).
pub(crate) fn request(
    server: &AssetServer,
    catalog: &crate::harvest::catalog::HarvestCatalog,
    package: &str,
    glbs: &mut HarvestGltfs,
    docs: &mut HarvestDocs,
) {
    let Some(files) = catalog.packages.get(package) else {
        error!("[delivery-drop] the drop package {package} is not in the harvest index");
        return;
    };
    glbs.by_key.entry(package.to_owned()).or_insert_with(|| {
        moly_assets::residency::load_gltf(
            server,
            bevy::asset::AssetPath::from(format!("moly://site/{}", files.glb)),
            moly_assets::residency::GltfResidency::GpuTextures,
        )
    });
    docs.0.entry(package.to_owned()).or_insert_with(|| {
        server.load::<moly_assets::json::JsonAsset>(bevy::asset::AssetPath::from(format!(
            "moly://site/{}",
            files.document
        )))
    });
}

/// `OnDropItem`'s model half (see the module comment); the view follows in
/// [`spawn`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_drop_item(
    model: &mut DeliveryModel,
    spawns: &mut DeliveryDropSpawns,
    objects: &DeliveryObjects,
    party_id: i32,
    rarity: i32,
    with_animation: bool,
    synchronized: bool,
    reason: &str,
) {
    let uid = model.next_uid();
    let Some(party) = model.party_mut(party_id) else {
        return;
    };
    let drop = DropModel {
        uid,
        party_id,
        material_id: party.reward_material_id,
        rarity,
        site_id: objects.site_id,
    };
    let within = party.within_limit();
    if within {
        if synchronized {
            party.drops.push(drop.clone());
        } else {
            party.unsynced_drops.push(drop.clone());
        }
    }
    let mut rng = Rng(0xDE11_0000_0000_0000u64 ^ (uid << 8) ^ objects.site_id as u64);
    let (a, b) = party.range.angle_bounds();
    let angle = law::random_range(a.min(b), a.max(b), rng.next_f32());
    let distance = law::random_range(
        party.range.min_distance,
        party.range.max_distance,
        rng.next_f32(),
    );
    let point = law::drop_point([-objects.place.x, objects.place.z], angle, distance);
    let raw = Vec2::new(-point[0], point[1]);
    let (yaw_deg, r) = if with_animation {
        (
            (rng.next_f32() * 360.0).floor(),
            law::random_range(0.6, 0.8, rng.next_f32()),
        )
    } else {
        (0.0, 0.0)
    };
    info!(
        "[delivery-drop] OnDropItem ({reason}) party {party_id} uid {uid}: material {} rarity {rarity}; within limit {within} (drops {} / {}, {}); range {:?}; angle {angle:.4} rad, distance {distance:.3} -> ({:.3}, {:.3}) before the snap",
        drop.material_id,
        party.drop_count(),
        party.tally.max_drop_item_count,
        if synchronized { "synchronized" } else { "unsynchronized" },
        party.range,
        raw.x,
        raw.y
    );
    spawns.pending.push(PendingSpawn {
        model: drop,
        within,
        with_animation,
        raw,
        yaw_deg,
        r,
    });
}

/// `GenerateUnclaimedDropItemsIfNeededAsync`: per party, drops the server
/// counts as dropped but the client does not hold are re-created on the
/// ground (no animation, synchronized), up to the limit; a client holding
/// more removes the surplus synchronized drops.
pub(crate) fn generate_unclaimed(
    model: &mut DeliveryModel,
    client: &ClientBirthdayPartyData,
    spawns: &mut DeliveryDropSpawns,
    objects: &DeliveryObjects,
    reason: &str,
) {
    let ids: Vec<(i32, i64)> = model
        .parties
        .iter()
        .map(|p| (p.id, p.reward_material_id))
        .collect();
    for (id, _) in ids {
        let dropped = client
            .user_birthday_party(id)
            .map_or(0, |row| row.dropped_mysekai_material_count);
        let Some(party) = model.party(id) else {
            continue;
        };
        let count = party.drop_count();
        let max = party.tally.max_drop_item_count;
        if count < dropped {
            let n = (dropped - count).min(max - count);
            info!(
                "[delivery-drop] GenerateUnclaimedDropItemsIfNeeded ({reason}) party {id}: the server counts {dropped} dropped, the client holds {count}: {n} re-created"
            );
            for _ in 0..n {
                on_drop_item(model, spawns, objects, id, -1, false, true, "unclaimed");
            }
        } else if count > dropped {
            remove_synchronized_drop_items(model, spawns, id, count - dropped, reason);
        }
    }
}

fn height_at(
    face: Option<&crate::walk_face::WalkFace>,
    objective: Option<&crate::npc_objective::ObjectiveFace>,
    epoch: Option<u64>,
    verts: &[Vec3],
    x: f32,
    z: f32,
) -> f32 {
    let point = face
        .zip(objective)
        .zip(epoch)
        .and_then(|((face, surface), epoch)| {
            (surface.navigation_generation() == face.generation() && surface.is_fresh(epoch))
                .then(|| surface.navigation_point_at([x, z]))
                .flatten()
        });
    match point {
        Some(point) => point[1],
        None => {
            crate::harvest::surface_y(verts, x, z, 0.0)
                + face.map_or(0.0, |f| f.height_offset([x, z]))
        }
    }
}

/// `PlayDropItemSE(rarity)`.
fn play_drop_item_se(rarity: i32, se: &mut SeRequests) {
    match rarity {
        1 | 2 => se.0.push(SeRequest {
            owner: None,
            cue: "se_drop_rare_material".to_owned(),
            class: SeClass::Ingame,
            source: "delivery-drop",
        }),
        0 | 3 => {}
        other => panic!("delivery drop rarity {other} is outside the SE arms (the source throws)"),
    }
}

/// Update: spawn the views whose package is in, in model order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    glbs: Res<HarvestGltfs>,
    docs: Res<HarvestDocs>,
    catalog: Option<Res<crate::harvest::catalog::HarvestCatalog>>,
    site: Res<DeliverySite>,
    mut spawns: ResMut<DeliveryDropSpawns>,
    mut se: ResMut<SeRequests>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    ground: Option<Res<crate::site::GroundMeshes>>,
    meshes: Res<Assets<Mesh>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
) {
    let (Some(objects), Some(catalog), Some(face)) = (site.objects.as_ref(), catalog, face) else {
        return;
    };
    if spawns.pending.is_empty() {
        return;
    }
    if spawns.ground.is_none() {
        let Some(ground) = ground.as_deref() else {
            return;
        };
        spawns.ground = Some(crate::harvest::ground_world_verts(ground, &meshes, &parts));
    }
    while let Some(item) = spawns.pending.first() {
        let Some(material) = catalog.materials.get(&item.model.material_id) else {
            error!(
                "[delivery-drop] uid {}: material {} has no drop model row (GetDropItemPrefab returns null): the view is not created",
                item.model.uid, item.model.material_id
            );
            spawns.pending.remove(0);
            continue;
        };
        let package = material.package.clone();
        let rarity = material.rarity;
        if !glbs.ready(&server, &package) || !docs.ready(&server, &package) {
            return;
        }
        let (Some(gltf), Some(doc)) = (
            glbs.get(&package).and_then(|h| gltfs.get(h)),
            docs.0.get(&package).and_then(|h| json.get(h)),
        ) else {
            return;
        };
        let item = spawns.pending.remove(0);
        let leaf = package.rsplit("__").next().unwrap_or(&package).to_owned();
        let value: serde_json::Value = serde_json::from_str(&doc.0)
            .unwrap_or_else(|err| panic!("drop document {leaf} is not JSON: {err}"));
        let view = &value["components"]["MysekaiAreaDropItemView"]["instances"][0]["fields"];
        let view_radius = view["radius"].as_f64().unwrap_or_else(|| {
            panic!("drop document {leaf}: MysekaiAreaDropItemView has no radius")
        }) as f32;
        let scene_index = crate::harvest::prefab_scene_index(&doc.0, &leaf);
        let scene = gltf.scenes.get(scene_index).cloned().unwrap_or_else(|| {
            panic!("drop model {leaf}: prefab scene {scene_index} out of range")
        });
        let verts = spawns.ground.as_deref().unwrap_or(&[]);
        let (snapped, tries) = crate::harvest::snap_from(&face, objects.arrive.xz(), item.raw);
        let y = height_at(
            Some(&*face),
            objective.as_deref(),
            epoch.as_deref().map(|e| e.0),
            verts,
            snapped.x,
            snapped.y,
        );
        let landing = Vec3::new(snapped.x, y, snapped.y);
        let start = if item.with_animation {
            objects.drop_start
        } else {
            landing
        };
        let mut model = item.model.clone();
        model.rarity = rarity;
        let mut transform = Transform::from_translation(start)
            .with_rotation(Quat::from_rotation_y(item.yaw_deg * 0.017_453_292));
        let (phase, radius) = if item.with_animation {
            transform.scale = Vec3::ZERO;
            (DropPhase::Hop, 0.0)
        } else if item.within {
            (DropPhase::Rest, view_radius)
        } else {
            (
                DropPhase::Fly {
                    flight: Flight::AutoGather,
                    acceleration: 0.0,
                },
                0.0,
            )
        };
        let entity = commands
            .spawn((
                SceneRoot(scene),
                DeliveryDropItem {
                    model: model.clone(),
                    radius,
                    view_radius,
                    within: item.within,
                    inside: false,
                    phase,
                },
                transform,
                Visibility::default(),
            ))
            .id();
        let effect = match rarity {
            0 => "_normalDropEffect",
            1 => "_rareDropEffect",
            2 => "_ultraRareDropEffect",
            3 => "nothing",
            other => panic!(
                "[delivery-drop] ShowRareDropEffect: rarity {other} is out of range (the source throws ArgumentOutOfRangeException)"
            ),
        };
        info!(
            "[delivery-drop] view uid {}: ShowRareDropEffect(rarity {rarity}) plays {effect}{} (not drawn: the prop particle host refuses the UberUnlit rows)",
            model.uid,
            if item.with_animation {
                "; the hop starts with StopAllDropEffect and ends with ResumeCurrentDropEffect"
            } else {
                ""
            }
        );
        if item.with_animation {
            play_drop_item_se(rarity, &mut se);
            let distance = start.distance(landing);
            let d1 = distance.clamp(0.6, 1.0);
            let d2 = (1.0 - distance).clamp(0.3, 1.0);
            commands
                .entity(entity)
                .insert(crate::harvest::HarvestDropAnimation::hop(
                    start, landing, d1, d2, item.r, true,
                ));
            info!(
                "[delivery-drop] view uid {} ({leaf}, rarity {rarity}{}): from ItemDropStartPosition ({:.3}, {:.3}, {:.3}) hop to ({:.3}, {:.3}, {:.3}) ({:.3} m from the place, snap tries {tries}); d1 {d1:.3} d2 {d2:.3} r {:.3} yaw {} deg; radius 0 then {view_radius}",
                model.uid,
                if matches!(rarity, 1 | 2) { ", se_drop_rare_material" } else { "" },
                start.x,
                start.y,
                start.z,
                landing.x,
                landing.y,
                landing.z,
                landing.xz().distance(objects.place.xz()),
                item.r,
                item.yaw_deg
            );
        } else {
            info!(
                "[delivery-drop] view uid {} ({leaf}, rarity {rarity}) put at ({:.3}, {:.3}, {:.3}) ({:.3} m from the place, snap tries {tries}); radius {radius}",
                model.uid,
                landing.x,
                landing.y,
                landing.z,
                landing.xz().distance(objects.place.xz())
            );
        }
    }
}

/// `RemoveSynchronizedDropItems(siteData, removeCount)`: nothing below 1;
/// otherwise the first `min(removeCount, count)` unsynchronized drops, then
/// the first `min(rest, count)` synchronized ones, each removed from the
/// model (`DeliverySiteModel.RemoveDropItem`) and its view destroyed
/// (`RemoveDropItemView`).
fn remove_synchronized_drop_items(
    model: &mut DeliveryModel,
    spawns: &mut DeliveryDropSpawns,
    party_id: i32,
    remove_count: i32,
    reason: &str,
) {
    if remove_count < 1 {
        return;
    }
    let Some(party) = model.party(party_id) else {
        return;
    };
    let take = |list: &[DropModel], n: i32| -> Vec<u64> {
        list.iter()
            .take(n.max(0) as usize)
            .map(|drop| drop.uid)
            .collect()
    };
    let mut uids = take(&party.unsynced_drops, remove_count);
    let rest = remove_count - uids.len() as i32;
    if rest >= 1 {
        uids.extend(take(&party.drops, rest));
    }
    for uid in &uids {
        model.remove_drop(*uid);
        let before = spawns.pending.len();
        spawns.pending.retain(|pending| pending.model.uid != *uid);
        if spawns.pending.len() == before {
            spawns.removed.push(*uid);
        }
    }
    info!(
        "[delivery-drop] GenerateUnclaimedDropItemsIfNeeded ({reason}) party {party_id}: the client holds {remove_count} more than the server counts: RemoveSynchronizedDropItems removes {uids:?} (unsynchronized first)"
    );
}

/// `OnCollisionEnterDropItem`: the uid joins the collision list (a list
/// add, a second enter adds it again), state Gather (published).
fn enter_drop(
    model: &mut DeliveryModel,
    progress: &mut MessageWriter<DeliveryProgress>,
    uid: u64,
    why: &str,
) {
    model.collision_drops.push(uid);
    model.state = DeliveryActionState::Gather;
    let rate = model.rate;
    publish(progress, DeliveryActionState::Gather, None, 0, 0.0, rate);
    info!("[delivery-drop] OnCollisionEnterDropItem uid {uid} ({why}); state Gather");
}

/// Update: the drops' collision edges (the collision manager, while it
/// updates), and the delivery screen's `TriggerOnEnterCollisions`, which
/// runs the enter callback of every drop the player collides with whatever
/// the game state.
pub(crate) fn scan(
    eligibility: crate::interaction::InteractionEligibility,
    mut model: ResMut<DeliveryModel>,
    mut progress: MessageWriter<DeliveryProgress>,
    mut retrigger: ResMut<super::DeliveryEnterRetrigger>,
    players: Query<&Transform, (With<PlayerControlled>, Without<DeliveryDropItem>)>,
    mut drops: Query<(&Transform, &mut DeliveryDropItem), Without<PlayerControlled>>,
) {
    let again = std::mem::take(&mut retrigger.drops);
    if model.site_id.is_none() {
        return;
    }
    if again {
        let mut colliding: Vec<u64> = drops
            .iter()
            .filter(|(_, item)| item.inside)
            .map(|(_, item)| item.model.uid)
            .collect();
        colliding.sort_unstable();
        for uid in colliding {
            enter_drop(&mut model, &mut progress, uid, "TriggerOnEnterCollisions");
        }
    }
    if !eligibility.collision_updates() {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let mut items: Vec<_> = drops.iter_mut().collect();
    items.sort_by_key(|(_, item)| item.model.uid);
    for (transform, mut item) in items {
        let inside = inside_circle(
            player.translation.to_array(),
            transform.translation.to_array(),
            item.radius,
        );
        if inside == item.inside {
            continue;
        }
        item.inside = inside;
        let uid = item.model.uid;
        if inside {
            let why = format!(
                "player {:.3} m away, radius {}",
                player.translation.distance(transform.translation),
                item.radius
            );
            enter_drop(&mut model, &mut progress, uid, &why);
        } else if let Some(index) = model.collision_drops.iter().position(|u| *u == uid) {
            // `List.Remove`: the first occurrence.
            model.collision_drops.remove(index);
        }
    }
}

/// Update: the hops' ends and the flights.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    time: Res<Time>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    mut spawns: ResMut<DeliveryDropSpawns>,
    mut model: ResMut<DeliveryModel>,
    mut se: ResMut<SeRequests>,
    players: Query<&Transform, (With<PlayerControlled>, Without<DeliveryDropItem>)>,
    mut drops: Query<
        (
            Entity,
            &mut Transform,
            &mut DeliveryDropItem,
            Has<crate::harvest::HarvestDropAnimation>,
        ),
        Without<PlayerControlled>,
    >,
) {
    if !spawns.removed.is_empty() {
        let removed = std::mem::take(&mut spawns.removed);
        for (entity, _, item, _) in &drops {
            if removed.contains(&item.model.uid) {
                commands.entity(entity).despawn();
            }
        }
        info!("[delivery-drop] RemoveDropItemView {removed:?}: views destroyed");
    }
    let Some(configs) = configs else {
        return;
    };
    let Ok(player) = players.single() else {
        return;
    };
    let player = player.translation;
    let step = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_SPEED);
    let reach = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_DISTANCE);
    let dt = time.delta_secs();
    for (entity, mut transform, mut item, hopping) in &mut drops {
        match item.phase {
            DropPhase::Hop => {
                if hopping {
                    continue;
                }
                transform.scale = Vec3::ONE;
                if item.within {
                    item.radius = item.view_radius;
                    item.phase = DropPhase::Rest;
                    info!(
                        "[delivery-drop] uid {} landed at ({:.3}, {:.3}, {:.3}): radius {}",
                        item.model.uid,
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                        item.radius
                    );
                } else {
                    item.radius = 0.0;
                    item.phase = DropPhase::Fly {
                        flight: Flight::AutoGather,
                        acceleration: 0.0,
                    };
                    info!(
                        "[delivery-drop] uid {} landed beyond the drop limit: radius 0, MoveItemTowardsPlayerAsync (auto-gather)",
                        item.model.uid
                    );
                }
            }
            DropPhase::Rest => {}
            DropPhase::Fly {
                flight,
                acceleration,
            } => {
                let d = transform.translation - player;
                if d.length_squared() < reach * reach {
                    let drop = item.model.clone();
                    match flight {
                        Flight::AutoGather => {
                            if let Some(party) = model.party_mut(drop.party_id) {
                                party.auto_gathered.push(drop.clone());
                            }
                            info!(
                                "[delivery-drop] uid {} reached the player: the collection notice (UI lane), auto-gather list +1, view destroyed",
                                drop.uid
                            );
                        }
                        Flight::Gather => {
                            se.0.push(SeRequest {
                                owner: None,
                                cue: "se_pick_item".to_owned(),
                                class: SeClass::Ingame,
                                source: "delivery-gather",
                            });
                            model.remove_drop(drop.uid);
                            model.add_gather_stack(drop.clone());
                            info!(
                                "[delivery-drop] uid {} gathered: se_pick_item, the collection notice (UI lane), RemoveDropItem, gather stack {}",
                                drop.uid,
                                model.gather_stack.len()
                            );
                        }
                    }
                    commands.entity(entity).despawn();
                    continue;
                }
                let acceleration = acceleration + dt;
                transform.translation = crate::harvest::move_towards(
                    transform.translation,
                    player,
                    step + acceleration,
                );
                item.phase = DropPhase::Fly {
                    flight,
                    acceleration,
                };
            }
        }
    }
}

/// Update: `OnUpdateGatherDropItem` (the controller's Update on the delivery
/// site).
pub(crate) fn gather(
    mut model: ResMut<DeliveryModel>,
    catalog: Option<Res<crate::harvest::catalog::HarvestCatalog>>,
    mut warned: Local<bool>,
    mut drops: Query<&mut DeliveryDropItem>,
) {
    if model.site_id.is_none() || model.collision_drops.is_empty() {
        return;
    }
    let uids = std::mem::take(&mut model.collision_drops);
    for uid in uids {
        let Some(drop) = model.drop_model(uid) else {
            continue;
        };
        if !can_collect_resource(catalog.as_deref(), &drop, &mut warned) {
            info!(
                "[delivery-drop] uid {uid}: CanCollectResource no: HarvestUtility.NoticeCollectItem (not drawn); the drop stays"
            );
            continue;
        }
        if let Some(mut item) = drops.iter_mut().find(|item| item.model.uid == uid) {
            if matches!(item.phase, DropPhase::Rest) {
                item.phase = DropPhase::Fly {
                    flight: Flight::Gather,
                    acceleration: 0.0,
                };
                info!("[delivery-drop] OnGatherDropItemAsync uid {uid}: MoveItemTowardsPlayer");
            }
        }
    }
}

/// `HarvestUserDataManager.CanCollectResource` for a drop (resource type
/// `mysekai_material`, quantity 1): `CanCollectMaterial`'s
/// `IsMaterialReceivableOverPossessionLimit` arm.
fn can_collect_resource(
    catalog: Option<&crate::harvest::catalog::HarvestCatalog>,
    drop: &DropModel,
    warned: &mut bool,
) -> bool {
    const GAME_CHARACTER: i32 = 4;
    const BIRTHDAY_PARTY: i32 = 7;
    let Some(material) = catalog.and_then(|catalog| catalog.materials.get(&drop.material_id))
    else {
        error!(
            "[delivery-drop] IsMaterialReceivableOverPossessionLimit: no MasterMysekaiMaterial row for id {} (the source's LogError){}; CanCollectResource no",
            drop.material_id,
            if catalog.is_none() { ", the mysekai material master is not loaded" } else { "" }
        );
        return false;
    };
    if matches!(material.material_type, GAME_CHARACTER | BIRTHDAY_PARTY) {
        return true;
    }
    if !*warned {
        *warned = true;
        warn!(
            "[delivery-drop] CanCollectMaterial({}, 1): material type {} is not receivable over the possession limit; the possession check (Gather stacks, UserMysekaiMaterialPossession, the possession level's limit) is not ported: answered yes",
            drop.material_id, material.material_type
        );
    }
    true
}

/// `ScheduleExecuteSendGatherData`.
#[derive(Resource, Default)]
pub(crate) struct DeliveryGatherLoop {
    delay: Option<Delay>,
    interval: f32,
    awaiting_reward: bool,
}

impl DeliveryGatherLoop {
    pub(crate) fn start(&mut self, frame: u64, interval: f32) {
        self.interval = interval;
        self.delay = Some(Delay::new(delay_seconds(interval as f64), frame));
        self.awaiting_reward = false;
    }

    pub(crate) fn cancel(&mut self) {
        *self = Self::default();
    }
}

/// Update: the gather loop.
pub(crate) fn gather_loop(
    frames: Res<FrameCount>,
    time: Res<Time>,
    mut gather: ResMut<DeliveryGatherLoop>,
    mut model: ResMut<DeliveryModel>,
    mut client: ResMut<ClientBirthdayPartyData>,
    mut runs: ResMut<RewardRuns>,
    mut progress: MessageWriter<DeliveryProgress>,
) {
    let frame = u64::from(frames.0);
    if gather.awaiting_reward {
        if !runs.take_finished(RewardOwner::Gather) {
            return;
        }
        after_gather(&mut gather, &mut model, &mut progress, frame);
        return;
    }
    let Some(delay) = gather.delay.as_mut() else {
        return;
    };
    if !delay.tick(frame, time.delta_secs()) {
        return;
    }
    let interval = gather.interval;
    gather.delay = Some(Delay::new(delay_seconds(interval as f64), frame));
    if model.state != DeliveryActionState::Gather || model.gather_stack.is_empty() {
        return;
    }
    let items = model.gather_stack.clone();
    model.state = DeliveryActionState::InGathering;
    publish(
        &mut progress,
        DeliveryActionState::InGathering,
        None,
        0,
        0.0,
        model.rate,
    );
    // UserBirthdayPartyGatherRequest.contents: per party of the stack, in
    // stack order, its gathered count.
    let mut contents: Vec<GatherContent> = Vec::new();
    for item in &items {
        match contents
            .iter_mut()
            .find(|content| content.birthday_party_id == item.party_id)
        {
            Some(content) => content.gathered_count += 1,
            None => contents.push(GatherContent {
                birthday_party_id: item.party_id,
                gathered_count: 1,
            }),
        }
    }
    let rewards = match crate::server::delivery::put_birthday_party_gather(&mut client, &contents) {
        Ok(reply) => reply.obtained_delivery_total_rewards,
        Err(reason) => {
            error!("[delivery-drop] {reason}: the error dialog (UI lane)");
            Vec::new()
        }
    };
    for item in &items {
        model.gather_stack.retain(|d| d != item);
    }
    model.update_synchronized(&client);
    if rewards.is_empty() {
        after_gather(&mut gather, &mut model, &mut progress, frame);
    } else {
        runs.start(RewardOwner::Gather, rewards);
        gather.awaiting_reward = true;
        gather.delay = None;
    }
}

fn after_gather(
    gather: &mut DeliveryGatherLoop,
    model: &mut DeliveryModel,
    progress: &mut MessageWriter<DeliveryProgress>,
    frame: u64,
) {
    let state = if model.gather_stack.is_empty() {
        DeliveryActionState::Idle
    } else {
        DeliveryActionState::Gather
    };
    model.state = state;
    publish(progress, state, None, 0, 0.0, model.rate);
    info!(
        "[delivery-drop] gather loop: state {state:?}; tallies {:?}",
        model
            .parties
            .iter()
            .map(|p| (p.id, p.drops.len(), p.unsynced_drops.len()))
            .collect::<Vec<_>>()
    );
    gather.awaiting_reward = false;
    gather.delay = Some(Delay::new(delay_seconds(gather.interval as f64), frame));
}
