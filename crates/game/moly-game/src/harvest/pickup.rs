//! Pickup: drops approach the moving player and are collected
//! (`MysekaiDropItemManager.OnCollectDropItem`, `MoveItemTowardsPlayer`).
//!
//! The drop list (`_dropItemList`) holds the drops the player's collision
//! owner added, in enter order: a landed drop (radius 1.0) enters when the
//! 3D distance to the player is within its radius and exits when it is not.
//! At the enter, a drop the inventory cannot take (`CanCollectResource`,
//! [`super::possession`]) raises the collection notice first
//! (`NoticeCollectItem`, [`super::notice`]). Drops that enter in one frame
//! are listed by uid (the physics order of one frame is not read here).
//! The collision scan runs while `ObjectCollisionManager.IsCanUpdate` holds
//! (the interaction gate) and not during a site move (its game state is
//! none of Normal, Harvest and Sketch).
//!
//! Every frame while the update is enabled (`SceneMysekai.Update` →
//! `OnUpdate`): destroyed drops leave the list; then for each listed drop,
//! in list order, a drop the inventory cannot take ends the frame's pass
//! (nothing is collected that frame, and the drops after it do not move);
//! otherwise, only while the player is in the Move or Dash state, the drop
//! approaches: when the squared distance is below
//! `DropItemApproachDistance`^2 (FloatConfigs 90) it is queued for
//! collection and its acceleration reset; then, while it is attracted, its
//! acceleration grows by the frame time and it moves toward the player by
//! `DropItemApproachSpeed` (FloatConfigs 89) plus that acceleration, a
//! per-frame step (`Vector3.MoveTowards`, not scaled by the frame time).
//! Then each queued drop the inventory can take (checked again, after the
//! stacks queued before it) is collected.
//!
//! Collection (`HarvestUtility.OnCollectDropItem`): the collection notice
//! (with its cue), then a gather stack for the log loop; then `GetDropItem`:
//! `se_pick_item`, the drop leaves the list and is destroyed.
//!
//! Leaving a harvest site by cannon (`AllCollectCollisionDropItemAsync`,
//! not awaited by the cannon): the update is disabled; the listed drops are
//! walked in order, and each live one the inventory can take is collected
//! (`OnCollectDropItem`, then destroyed without `se_pick_item`), one per
//! frame (a `UniTask.Yield` after each); then the list is cleared and the
//! update enabled again. The walk is the list's own enumerator: a list
//! changed under it throws in the source, which ends the task with the
//! update left disabled; that case is reported here and reproduced.
//!
//! Named gaps: the notice is not drawn (see [`super::notice`]); the
//! possession values are the `PossessionMock` panel's.

use bevy::prelude::*;

use super::catalog::{HarvestCatalog, HarvestUserData};
use super::notice::{notice_collect_item, CollectNotice};
use super::possession::{Capacity, PossessionLimits, PossessionMock};
use super::queue::{HarvestLogQueue, Stack};
use super::{HarvestDropItem, HarvestStats};
use crate::audio::SeRequests;
use crate::player::PlayerControlled;
use crate::player_state::{PlayerActionState, PlayerAvatarStates};

/// `MysekaiDropItemManager`: the drop list, `_isEnableUpdate`, and the
/// cannon's walk while it runs.
#[derive(Resource)]
pub(crate) struct HarvestDropManager {
    list: Vec<Entity>,
    /// The list's version (`List<T>._version`): every add and remove bumps it.
    version: u64,
    enable_update: bool,
    all_collect: Option<AllCollect>,
}

impl Default for HarvestDropManager {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            version: 0,
            enable_update: true,
            all_collect: None,
        }
    }
}

/// `AllCollectCollisionDropItemAsync`'s enumerator.
struct AllCollect {
    index: usize,
    version: u64,
    collected: usize,
}

impl HarvestDropManager {
    fn remove(&mut self, entity: Entity) {
        if let Some(at) = self.list.iter().position(|listed| *listed == entity) {
            self.list.remove(at);
            self.version += 1;
        }
    }
}

/// The user data and panels the capacity check and the notice read.
struct Inputs<'a> {
    catalog: &'a HarvestCatalog,
    user: &'a HarvestUserData,
    limits: Option<&'a PossessionLimits>,
    mock: &'a PossessionMock,
}

impl<'a> Inputs<'a> {
    fn capacity<'q>(&self, queue: &'q HarvestLogQueue) -> Capacity<'q>
    where
        'a: 'q,
    {
        Capacity {
            stacks: &queue.stacks,
            catalog: self.catalog,
            limits: self.limits,
            mock: self.mock,
        }
    }

    fn can_collect(&self, queue: &HarvestLogQueue, item: &HarvestDropItem) -> bool {
        self.capacity(queue)
            .can_collect(item.resource_type, item.resource_id, item.row.quantity)
    }

    /// `HarvestUtility.NoticeCollectItem(model)`.
    fn notice(
        &self,
        queue: &HarvestLogQueue,
        item: &HarvestDropItem,
        se: &mut SeRequests,
        notices: &mut Vec<CollectNotice>,
        reason: &str,
    ) {
        notice_collect_item(
            item.resource_type,
            item.resource_id,
            item.row.quantity,
            &self.capacity(queue),
            self.catalog,
            self.user,
            se,
            notices,
            reason,
        );
    }
}

/// `HarvestUtility.OnCollectDropItem`: the notice, then the gather stack.
fn on_collect_drop_item(
    inputs: &Inputs,
    item: &HarvestDropItem,
    se: &mut SeRequests,
    queue: &mut HarvestLogQueue,
    stats: &mut HarvestStats,
    notices: &mut Vec<CollectNotice>,
    reason: &str,
) {
    inputs.notice(queue, item, se, notices, reason);
    queue.stacks.push(Stack::Gather {
        site_id: item.site_id,
        drop: item.row.clone(),
    });
    stats.collected += 1;
    info!(
        "[harvest-pickup] collected uid {} ({reason}): resourceType {} id {} qty {} rarity {}",
        item.uid, item.resource_type, item.resource_id, item.row.quantity, item.rarity
    );
}

/// Update: the collision scan, then the manager's frame pass.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn advance(
    mut commands: Commands,
    time: Res<Time>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    states: Res<PlayerAvatarStates>,
    eligibility: crate::interaction::InteractionEligibility,
    players: Query<&Transform, (With<PlayerControlled>, Without<HarvestDropItem>)>,
    // Disjoint from the eligibility's NPC query (drops carry no unit id).
    mut drops: Query<
        (Entity, &mut HarvestDropItem, &mut Transform),
        (
            Without<PlayerControlled>,
            Without<crate::npc::CharacterUnitId>,
        ),
    >,
    mut se: ResMut<SeRequests>,
    mut queue: ResMut<HarvestLogQueue>,
    mut stats: ResMut<HarvestStats>,
    mut manager: ResMut<HarvestDropManager>,
    data: (
        Option<Res<HarvestCatalog>>,
        Option<Res<HarvestUserData>>,
        Option<Res<PossessionLimits>>,
        Res<PossessionMock>,
        Option<Res<crate::site_move::SiteMoveActive>>,
    ),
    mut notices: MessageWriter<CollectNotice>,
) {
    let (catalog, user, limits, mock, site_move) = data;
    let (Some(configs), Some(catalog), Some(user)) = (configs, catalog, user) else {
        return;
    };
    let inputs = Inputs {
        catalog: &catalog,
        user: &user,
        limits: limits.as_deref(),
        mock: &mock,
    };
    let Ok(player) = players.single() else {
        return;
    };
    let player = player.translation;
    let mut raised = Vec::new();

    // The collision owner: enters in uid order, then exits.
    if eligibility.collision_updates() && site_move.is_none() {
        let mut entered: Vec<(u64, Entity)> = Vec::new();
        let mut exited: Vec<Entity> = Vec::new();
        for (entity, item, transform) in &drops {
            let inside = item.radius > 0.0 && transform.translation.distance(player) <= item.radius;
            let listed = manager.list.contains(&entity);
            if inside && !listed {
                entered.push((item.uid, entity));
            } else if !inside && listed {
                exited.push(entity);
            }
        }
        entered.sort_by_key(|(uid, _)| *uid);
        for (_, entity) in entered {
            let (_, item, _) = drops.get(entity).expect("listed above");
            if !inputs.can_collect(&queue, item) {
                inputs.notice(
                    &queue,
                    item,
                    &mut se,
                    &mut raised,
                    "entered, the inventory cannot take it",
                );
            }
            manager.list.push(entity);
            manager.version += 1;
        }
        for entity in exited {
            manager.remove(entity);
        }
    }

    if manager.enable_update {
        // RemoveAll(item == null).
        let before = manager.list.len();
        manager.list.retain(|entity| drops.contains(*entity));
        if manager.list.len() != before {
            manager.version += 1;
        }
        let moving = matches!(
            states.current,
            PlayerActionState::Move | PlayerActionState::Dash
        );
        let step = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_SPEED);
        let reach = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_DISTANCE);
        let dt = time.delta_secs();
        let mut to_collect: Vec<Entity> = Vec::new();
        let mut stopped = false;
        let listed = manager.list.clone();
        for entity in listed {
            let Ok((_, mut item, mut transform)) = drops.get_mut(entity) else {
                continue;
            };
            if !inputs.can_collect(&queue, &item) {
                stopped = true;
                break;
            }
            if !moving {
                continue;
            }
            // MoveItemTowardsPlayer.
            if (transform.translation - player).length_squared() < reach * reach {
                item.acceleration = 0.0;
                to_collect.push(entity);
            }
            if item.attracting {
                item.acceleration += dt;
                transform.translation =
                    move_towards(transform.translation, player, step + item.acceleration);
            }
        }
        if !stopped {
            for entity in to_collect {
                let Ok((_, item, _)) = drops.get(entity) else {
                    continue;
                };
                if !inputs.can_collect(&queue, item) {
                    continue;
                }
                on_collect_drop_item(
                    &inputs,
                    item,
                    &mut se,
                    &mut queue,
                    &mut stats,
                    &mut raised,
                    "walked into it",
                );
                // GetDropItem.
                super::damage::push_se(&mut se, "se_pick_item", "harvest-pickup");
                manager.remove(entity);
                commands.entity(entity).despawn();
            }
        }
    }
    notices.write_batch(raised);
}

/// `Vector3.MoveTowards`.
pub(crate) fn move_towards(current: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let d = target - current;
    let length = d.length();
    if length <= max_delta || length == 0.0 {
        target
    } else {
        current + d / length * max_delta
    }
}

/// Update: the cannon leaves a harvest site
/// (`AllCollectCollisionDropItemAsync`, started the frame the site move
/// begins, then one collected drop a frame).
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_on_leave(
    mut commands: Commands,
    site_move: Option<Res<crate::site_move::SiteMoveActive>>,
    site: Option<Res<crate::site::SiteActive>>,
    drops: Query<&HarvestDropItem>,
    mut se: ResMut<SeRequests>,
    mut queue: ResMut<HarvestLogQueue>,
    mut stats: ResMut<HarvestStats>,
    mut manager: ResMut<HarvestDropManager>,
    data: (
        Option<Res<HarvestCatalog>>,
        Option<Res<HarvestUserData>>,
        Option<Res<PossessionLimits>>,
        Res<PossessionMock>,
    ),
    mut notices: MessageWriter<CollectNotice>,
) {
    let (catalog, user, limits, mock) = data;
    let started = site_move
        .as_ref()
        .is_some_and(|site_move| site_move.is_added())
        && site.is_some_and(|site| site.category == "harvest");
    if started {
        if manager.all_collect.is_some() {
            warn!("[harvest-pickup] AllCollectCollisionDropItemAsync started while one runs");
        }
        manager.enable_update = false;
        manager.all_collect = Some(AllCollect {
            index: 0,
            version: manager.version,
            collected: 0,
        });
        info!(
            "[harvest-pickup] AllCollectCollisionDropItemAsync at the cannon's leave: {} drops listed",
            manager.list.len()
        );
    }
    let Some(mut walk) = manager.all_collect.take() else {
        return;
    };
    let (Some(catalog), Some(user)) = (catalog, user) else {
        manager.all_collect = Some(walk);
        return;
    };
    let inputs = Inputs {
        catalog: &catalog,
        user: &user,
        limits: limits.as_deref(),
        mock: &mock,
    };
    let mut raised = Vec::new();
    loop {
        if walk.version != manager.version {
            error!(
                "[harvest-pickup] AllCollectCollisionDropItemAsync: the drop list changed under its enumerator after {} collected; the source throws (InvalidOperationException) and the update stays disabled",
                walk.collected
            );
            break;
        }
        let next = manager.list.get(walk.index).copied();
        let Some(entity) = next else {
            manager.list.clear();
            manager.version += 1;
            manager.enable_update = true;
            info!(
                "[harvest-pickup] AllCollectCollisionDropItemAsync done: {} drops collected; list cleared, update enabled",
                walk.collected
            );
            break;
        };
        walk.index += 1;
        let Ok(item) = drops.get(entity) else {
            continue; // destroyed
        };
        if !inputs.can_collect(&queue, item) {
            continue;
        }
        on_collect_drop_item(
            &inputs,
            item,
            &mut se,
            &mut queue,
            &mut stats,
            &mut raised,
            "listed at the cannon's leave",
        );
        commands.entity(entity).despawn();
        walk.collected += 1;
        // UniTask.Yield: the next drop on the next frame.
        manager.all_collect = Some(walk);
        break;
    }
    notices.write_batch(raised);
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// `Vector3.MoveTowards`: a step shorter than the distance moves along
    /// the line; a longer one lands on the target.
    #[test]
    fn move_towards_steps_and_lands() {
        let a = Vec3::ZERO;
        let b = Vec3::new(3.0, 0.0, 4.0);
        let p = move_towards(a, b, 0.13 + 0.016);
        assert!((p.length() - 0.146).abs() < 1e-6);
        assert_eq!(move_towards(a, b, 6.0), b);
    }
}
