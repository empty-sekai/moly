//! Pickup: drops approach the moving player and are collected
//! (`MysekaiDropItemManager.OnCollectDropItem`, `MoveItemTowardsPlayer`).
//!
//! A landed drop has radius 1.0; the player is in contact when the 3D
//! distance to it is within that radius. Every frame, for each drop in
//! contact that the inventory can take, and only while the player is in the
//! Move or Dash state: when the squared distance is below
//! `DropItemApproachDistance`^2 (FloatConfigs 90) the drop is queued for
//! collection and its acceleration reset; then, while it is attracted, its
//! acceleration grows by the frame time and it moves toward the player by
//! `DropItemApproachSpeed` (FloatConfigs 89) plus that acceleration, a
//! per-frame step (`Vector3.MoveTowards`, not scaled by the frame time).
//!
//! Collection (`HarvestUtility.OnCollectDropItem`): the material cue
//! (`se_get_rare_material` for a tone material or a rarity of rarity_2 and
//! above, else `se_get_material`), a gather stack for the log loop, then
//! `GetDropItem`: `se_pick_item` and the drop is destroyed. Leaving a harvest
//! site by cannon collects every drop in contact
//! (`AllCollectCollisionDropItemAsync`).
//!
//! Named gaps: `CanCollectResource` (inventory capacity, server data) is the
//! mock's own answer, always yes; the collection notice (HUD, dialogs) is
//! the UI lane's.

use bevy::prelude::*;

use super::queue::{HarvestLogQueue, Stack};
use super::{HarvestDropItem, HarvestStats, RT_MYSEKAI_MATERIAL};
use crate::audio::SeRequests;
use crate::player::PlayerControlled;
use crate::player_state::{PlayerActionState, PlayerAvatarStates};

fn collect(
    commands: &mut Commands,
    entity: Entity,
    item: &HarvestDropItem,
    se: &mut SeRequests,
    queue: &mut HarvestLogQueue,
    stats: &mut HarvestStats,
    reason: &str,
) {
    if item.resource_type == RT_MYSEKAI_MATERIAL {
        let rare = item.material_type == 6 || item.rarity >= 1;
        super::damage::push_se(
            se,
            if rare {
                "se_get_rare_material"
            } else {
                "se_get_material"
            },
            "harvest-pickup",
        );
    }
    super::damage::push_se(se, "se_pick_item", "harvest-pickup");
    queue.stacks.push(Stack::Gather {
        site_id: item.site_id,
        drop: item.row.clone(),
    });
    stats.collected += 1;
    info!(
        "[harvest-pickup] collected uid {} ({reason}): resourceType {} id {} qty {} rarity {}",
        item.uid, item.resource_type, item.resource_id, item.row.quantity, item.rarity
    );
    commands.entity(entity).despawn();
}

/// Update: approach and collection while the player moves.
#[allow(clippy::too_many_arguments)]
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
) {
    let Some(configs) = configs else {
        return;
    };
    if !eligibility.collision_updates() {
        return;
    }
    if !matches!(
        states.current,
        PlayerActionState::Move | PlayerActionState::Dash
    ) {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let player = player.translation;
    let step = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_SPEED);
    let reach = configs.float(crate::client_config::KEY_DROP_ITEM_APPROACH_DISTANCE);
    let dt = time.delta_secs();
    for (entity, mut item, mut transform) in &mut drops {
        if item.radius <= 0.0 || transform.translation.distance(player) > item.radius {
            continue;
        }
        let d = transform.translation - player;
        let mut queued = false;
        if d.length_squared() < reach * reach {
            item.acceleration = 0.0;
            queued = true;
        }
        if item.attracting {
            item.acceleration += dt;
            transform.translation =
                move_towards(transform.translation, player, step + item.acceleration);
        }
        if queued {
            collect(
                &mut commands,
                entity,
                &item,
                &mut se,
                &mut queue,
                &mut stats,
                "walked into it",
            );
        }
    }
}

/// `Vector3.MoveTowards`.
fn move_towards(current: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let d = target - current;
    let length = d.length();
    if length <= max_delta || length == 0.0 {
        target
    } else {
        current + d / length * max_delta
    }
}

/// Update: the cannon leaves a harvest site; every drop in contact is
/// collected first.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_on_leave(
    mut commands: Commands,
    site_move: Option<Res<crate::site_move::SiteMoveActive>>,
    site: Option<Res<crate::site::SiteActive>>,
    players: Query<&Transform, (With<PlayerControlled>, Without<HarvestDropItem>)>,
    drops: Query<(Entity, &HarvestDropItem, &Transform), Without<PlayerControlled>>,
    mut se: ResMut<SeRequests>,
    mut queue: ResMut<HarvestLogQueue>,
    mut stats: ResMut<HarvestStats>,
) {
    let Some(site_move) = site_move else {
        return;
    };
    if !site_move.is_added() || !site.is_some_and(|site| site.category == "harvest") {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let mut count = 0usize;
    for (entity, item, transform) in &drops {
        if item.radius > 0.0 && transform.translation.distance(player.translation) <= item.radius {
            collect(
                &mut commands,
                entity,
                item,
                &mut se,
                &mut queue,
                &mut stats,
                "in contact at the cannon's leave",
            );
            count += 1;
        }
    }
    info!(
        "[harvest-pickup] AllCollectCollisionDropItemAsync at the cannon's leave: {count} drops in contact collected"
    );
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
