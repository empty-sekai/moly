//! EditGameState's actor Hide/Show adaptation. Logical actors, RNG and source
//! content stay alive; only this editor's reversible visibility overlay is held.

use crate::{
    fixture::{FixtureLayoutRevision, FixturePlacements, FixtureScenesReady},
    fixture_material::FixtureMaterialsSwapped,
    npc::CharacterUnitId,
    npc_objective::ObjectiveFace,
    player::{PlayerControlled, PlayerInput},
    site::{GroundEpoch, SiteActive},
    walk_face::WalkFace,
};
use bevy::prelude::*;

#[derive(Component)]
struct ActorPausedByEdit;

#[derive(Component, Clone, Copy)]
struct HiddenActorView {
    previous: Option<Visibility>,
}

#[derive(Resource, Default)]
struct OverlayDiscovery {
    entities: usize,
    actors: usize,
}

/// Source HideAndCancelObjective hides first, then TryCancel checks current
/// action != Talk(4). Ordinary cancellation is supplied by the NPC owner; the
/// existing furniture owner runs separately and retains its own lease checks.
pub(super) fn enter(world: &mut World) {
    world.remove_resource::<OverlayDiscovery>();
    maintain(world);
    maintain_detached_views(world);
}

/// Catch late SD children without repeatedly traversing a stable hierarchy.
/// Children with explicit Visible also receive an overlay: Bevy's Visible can
/// bypass a hidden parent's inherited visibility, unlike Unity SetActive(false).
pub(super) fn maintain(world: &mut World) {
    if !world
        .get_resource::<super::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return;
    }
    let actors: Vec<_> = world
        .query_filtered::<Entity, With<CharacterUnitId>>()
        .iter(world)
        .collect();
    let entity_count = world.entities().len() as usize;
    let discover = world
        .get_resource::<OverlayDiscovery>()
        .is_none_or(|old| old.entities != entity_count || old.actors != actors.len());
    if discover {
        for actor in &actors {
            let first = world.get::<ActorPausedByEdit>(*actor).is_none();
            if first {
                world.entity_mut(*actor).insert(ActorPausedByEdit);
            }
            let mut descendants = vec![*actor];
            let mut index = 0;
            while index < descendants.len() {
                let entity = descendants[index];
                index += 1;
                if let Some(children) = world.get::<Children>(entity) {
                    descendants.extend(children.iter());
                }
            }
            for entity in descendants {
                if world.get::<HiddenActorView>(entity).is_none() {
                    let previous = world.get::<Visibility>(entity).copied();
                    // Transform-only bone nodes need no visibility component;
                    // roots and actual render/scene nodes do.
                    if entity == *actor || previous.is_some() {
                        world
                            .entity_mut(entity)
                            .insert((HiddenActorView { previous }, Visibility::Hidden));
                    }
                }
            }
            if first
                && !world.entity(*actor).contains::<PlayerControlled>()
                && crate::npc_objective::cancel_ordinary_for_layout_edit(world, *actor)
            {
                crate::npc::stop_for_layout_edit(world, *actor);
            }
            if first {
                if let Some(mut input) = world.get_mut::<PlayerInput>(*actor) {
                    input.active = false;
                    input.direction = Vec3::ZERO;
                }
            }
        }
        world.insert_resource(OverlayDiscovery {
            entities: world.entities().len() as usize,
            actors: actors.len(),
        });
    }
    // Material/model writers may finish after entry. Keep only our owned
    // actor view nodes hidden. Detached HUD has its own late-frame pass after
    // every producer, not a second scan/cancellation of the actor hierarchy.
    let revealed: Vec<_> = world
        .query_filtered::<(Entity, &HiddenActorView, &Visibility), (
            Without<crate::balloon::BalloonAnchor>, Without<crate::emoticon::EmoteDraw>,
        )>()
        .iter(world)
        .filter_map(|(entity, _, visibility)| (*visibility != Visibility::Hidden).then_some(entity))
        .collect();
    for entity in revealed {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}

/// The existing reversible visibility owner, applied after detached HUD
/// producers. No reaction/talk request, RNG, lifetime, alpha or animation
/// clock is changed. Entry also calls this synchronously for already-live HUD.
///
/// Keep this separate from maintain: its late pass must not move the ordinary
/// actor cancellation point or traverse every actor view tree a second time.
pub(super) fn maintain_detached_views(world: &mut World) {
    if !world.get_resource::<super::EditSessionActive>()
        .is_some_and(|editor| editor.is_active()) { return; }
    let views: Vec<_> = world.query_filtered::<
        (Entity, Option<&Visibility>, Has<HiddenActorView>),
        Or<(With<crate::balloon::BalloonAnchor>, With<crate::emoticon::EmoteDraw>)>,
    >().iter(world).map(|(entity, visibility, owned)| (entity, visibility.copied(), owned)).collect();
    for (entity, current, owned) in views {
        if !owned {
            world.entity_mut(entity).insert((HiddenActorView { previous: current }, Visibility::Hidden));
        } else if current != Some(Visibility::Hidden) {
            world.entity_mut(entity).insert(Visibility::Hidden);
        }
    }
}

/// Private continuation evidence, sampled from the actual layout reload.
/// No clock/debounce delay can satisfy this condition.
pub(super) struct PendingReload {
    site_id: u32,
    epoch: Option<u64>,
    layout_revision: u64,
    before_walk_generation: Option<u64>,
    needs_new_walk: bool,
    pub exit_after: bool,
}

pub(super) fn wait_for_reload(
    world: &World,
    needs_new_walk: bool,
    exit_after: bool,
) -> PendingReload {
    PendingReload {
        site_id: world
            .get_resource::<FixturePlacements>()
            .map_or(0, FixturePlacements::site_id),
        epoch: world.get_resource::<GroundEpoch>().map(|epoch| epoch.0),
        layout_revision: world
            .get_resource::<FixtureLayoutRevision>()
            .map_or(0, |revision| revision.0),
        before_walk_generation: world.get_resource::<WalkFace>().map(WalkFace::generation),
        needs_new_walk,
        exit_after,
    }
}

pub(super) fn reload_ready(world: &World, pending: &PendingReload) -> Result<bool, String> {
    let Some(layout) = world.get_resource::<FixturePlacements>() else {
        return Ok(false);
    };
    let Some(epoch) = world.get_resource::<GroundEpoch>() else {
        return Ok(false);
    };
    if layout.site_id() != pending.site_id || Some(epoch.0) != pending.epoch {
        return Err("场地在恢复期间已改变，角色仍保持暂停；未在另一地图上恢复旧动作。".into());
    }
    if !world
        .get_resource::<SiteActive>()
        .is_some_and(|site| site.site_type == layout.site_type())
    {
        return Ok(false);
    }
    if world
        .get_resource::<FixtureLayoutRevision>()
        .is_none_or(|revision| revision.0 != pending.layout_revision)
    {
        return Err("布局在恢复期间又被替换，角色仍保持暂停。".into());
    }
    if !world.contains_resource::<FixtureScenesReady>()
        || (layout.total() != 0 && !world.contains_resource::<FixtureMaterialsSwapped>())
    {
        return Ok(false);
    }
    let (Some(walk), Some(objective)) = (
        world.get_resource::<WalkFace>(),
        world.get_resource::<ObjectiveFace>(),
    ) else {
        return Ok(false);
    };
    if walk.layout_revision() != pending.layout_revision
        || !objective.is_fresh(epoch.0)
        || objective.navigation_generation() != walk.generation()
    {
        return Ok(false);
    }
    if pending.needs_new_walk
        && pending
            .before_walk_generation
            .is_some_and(|old| walk.generation() <= old)
    {
        return Ok(false);
    }
    Ok(true)
}

/// Show reattaches against the existing rebuilt WalkField, not another
/// navigation solver. Resolve all required points before revealing any actor.
pub(super) fn restore(world: &mut World) -> Result<(), String> {
    let actors: Vec<_> = world
        .query_filtered::<(Entity, &Transform, Option<&PlayerControlled>), With<ActorPausedByEdit>>(
        )
        .iter(world)
        .map(|(entity, transform, player)| (entity, transform.translation, player.is_some()))
        .collect();
    let generation = world
        .get_resource::<WalkFace>()
        .ok_or("可行走场尚未恢复")?
        .generation();
    let face = world
        .get_resource::<ObjectiveFace>()
        .ok_or("目标面尚未恢复")?;
    let points: Vec<_> = actors
        .iter()
        .map(|(actor, current, player)| {
            let point = face
                .reattach_after_layout(current.to_array())
                .ok_or_else(|| format!("角色 {actor:?} 没有安全的可行走落点，保持暂停"))?;
            Ok((*actor, Vec3::from(point), *player))
        })
        .collect::<Result<_, String>>()?;
    for (actor, point, player) in points {
        if player {
            if let Some(mut transform) = world.get_mut::<Transform>(actor) {
                transform.translation = point;
            }
            if let Some(mut input) = world.get_mut::<PlayerInput>(actor) {
                input.active = false;
                input.direction = Vec3::ZERO;
            }
        } else {
            crate::npc::reattach_after_layout_edit(world, actor, point, generation);
        }
    }
    clear_overlay(world);
    Ok(())
}

/// Site teardown owns the following reseed. Release only our visibility layer;
/// never teleport a retired actor using the previous site's field.
pub(super) fn clear_overlay(world: &mut World) {
    let views: Vec<_> = world
        .query::<(Entity, &HiddenActorView)>()
        .iter(world)
        .map(|(entity, overlay)| (entity, *overlay))
        .collect();
    for (entity, overlay) in views {
        if let Ok(mut view) = world.get_entity_mut(entity) {
            if let Some(previous) = overlay.previous {
                view.insert(previous);
            } else {
                view.remove::<Visibility>();
            }
            view.remove::<HiddenActorView>();
        }
    }
    let actors: Vec<_> = world
        .query_filtered::<Entity, With<ActorPausedByEdit>>()
        .iter(world)
        .collect();
    for actor in actors {
        world.entity_mut(actor).remove::<ActorPausedByEdit>();
    }
    world.remove_resource::<OverlayDiscovery>();
}
