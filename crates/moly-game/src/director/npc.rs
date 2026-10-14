//! Characters under the director. Taking a character cancels its current
//! objective, stops its walk and ends its AI loop's decisions the way an
//! ended loop is marked, so neither the decision loop nor the presenter's
//! greeting and overlap calls drive it. While it is taken the director
//! walks it with its own navigation and walk law. Releasing it reopens the
//! loop and owes it a forced objective update, so it decides afresh.

use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use crate::npc::{
    CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, RestLifecycle, RouteOutcome, RouteStops, WalkState,
};
use crate::npc_objective::{MemberRng, ObjectiveFace, ObjectiveMind};

/// The reason a taken character's loop shows while the director owns it.
pub(crate) const OWNED: &str = "the director owns this character";

/// Marks a character the director has taken.
#[derive(Component)]
pub(crate) struct DirectorOwned;

/// Takes the character of `unit`. A character in a conversation, or whose
/// loop has ended for another reason, is refused.
pub(crate) fn take(world: &mut World, unit: u32) -> Result<Entity, String> {
    let entity = super::cond::npc(world, unit)?;
    if world.get::<DirectorOwned>(entity).is_some() {
        return Ok(entity);
    }
    let mind = world.get::<ObjectiveMind>(entity).ok_or_else(|| format!("character {unit} has no AI"))?;
    if let Some(reason) = &mind.ai_stopped {
        return Err(format!("character {unit}'s AI loop has ended: {reason}"));
    }
    if world.get::<NpcActions>(entity).is_some_and(|actions| actions.current == NpcAction::Talk) {
        return Err(format!("character {unit} is in a conversation"));
    }
    let cancelled = crate::npc::gate_entries::try_cancel_current_objective(world, entity);
    crate::npc::stop_for_external_activity(world, entity);
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.ai_stopped = Some(OWNED.into());
    }
    world.entity_mut(entity).insert(DirectorOwned);
    info!("[director] character {unit} taken (its objective cancelled: {cancelled}); its AI makes no decision until released");
    Ok(entity)
}

/// Releases a taken character: its walk stops, its loop reopens and a forced
/// objective update is owed to its next decision.
pub(crate) fn release(world: &mut World, entity: Entity) {
    if world.get_entity(entity).is_err() || world.get::<DirectorOwned>(entity).is_none() {
        return;
    }
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    crate::npc::stop_for_external_activity(world, entity);
    world.entity_mut(entity).remove::<DirectorOwned>();
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        if mind.ai_stopped.as_deref() == Some(OWNED) {
            mind.ai_stopped = None;
        }
    }
    let updated = crate::npc::gate_entries::force_update_objective(world, entity);
    info!("[director] character {unit} released (forced objective update owed: {updated})");
}

type WalkParams<'w, 's> = (
    Option<Res<'w, crate::walk_face::WalkFace>>,
    Option<Res<'w, ObjectiveFace>>,
    Query<'w, 's, &'static GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>,
    Query<
        'w,
        's,
        (
            &'static CharacterUnitId,
            &'static mut WalkState,
            &'static mut PathSlot,
            &'static mut RouteStops,
            &'static mut MotionPhase,
            &'static mut NpcActions,
            &'static mut RestLifecycle,
            &'static mut MemberRng,
        ),
    >,
);

/// Starts a walk of a taken character to `goal` (x, z) on the navigation
/// field, with the walk law its objectives use. Without `rests` the route's
/// Rest stops (the law's random side pauses) are dropped and the character
/// walks its navigation corners straight through.
pub(crate) fn walk(world: &mut World, entity: Entity, goal: Vec2, rests: bool) -> Result<(), String> {
    if world.get::<DirectorOwned>(entity).is_none() {
        return Err("take the character before walking it (npc.take)".into());
    }
    let mut params = SystemState::<WalkParams>::new(world);
    let (face, objective, origins, mut actors) = params.get_mut(world);
    let face = face.ok_or("the walk face is not built")?;
    let objective = objective.ok_or("the objective face is not built")?;
    let site_y = origins
        .single()
        .map_err(|_| "one active site origin is required")?
        .translation()
        .y;
    let (unit, mut walk, mut path, mut route, mut phase, mut actions, mut rest, mut rng) =
        actors.get_mut(entity).map_err(|_| "the character has no navigation")?;
    let unit_id = unit.0;
    *phase = crate::npc::depart(
        unit,
        &mut walk.0,
        &mut path.0,
        &mut route,
        &face,
        &objective,
        [goal.x, site_y, goal.y],
        None,
        &mut rng,
    )
    .ok_or_else(|| format!("character {unit_id} has no path to ({:.2}, {:.2})", goal.x, goal.y))?;
    if !rests {
        let dropped = route.drop_rests();
        info!("[director] character {unit_id} walks to ({:.2}, {:.2}) without its {dropped} rest stops", goal.x, goal.y);
    }
    crate::npc::declare_navigation_action(&mut actions, &mut rest, &phase, &route);
    Ok(())
}

/// Turns a taken character toward `towards` over `secs` with the greeting's
/// look-at tween. The tween runs only while the character stands, so a
/// walking character is refused (the `npc.face` step waits for it to stand).
pub(crate) fn face(world: &mut World, entity: Entity, towards: Vec3, secs: f32) -> Result<(), String> {
    if world.get::<DirectorOwned>(entity).is_none() {
        return Err("take the character before turning it (npc.take)".into());
    }
    if !standing(world, entity) {
        return Err("the character is moving; turn it once it stands".into());
    }
    let forward = world
        .get::<WalkState>(entity)
        .map(|walk| walk.0.forward)
        .ok_or("the character has no walk state")?;
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    world.entity_mut(entity).insert(crate::npc_look_at::NpcLookAt::new(
        towards,
        forward,
        secs,
        moly_law::ui::dotween::Ease::Linear,
        frame,
    ));
    Ok(())
}

/// Whether the character stands (its motion is a dwell).
pub(crate) fn standing(world: &World, entity: Entity) -> bool {
    matches!(world.get::<MotionPhase>(entity), Some(MotionPhase::Dwelling { .. }))
}

/// Whether a turn started by [`face`] is still running.
pub(crate) fn facing(world: &World, entity: Entity) -> bool {
    world.get::<crate::npc_look_at::NpcLookAt>(entity).is_some()
}

/// The walk's end: `Some(true)` arrived, `Some(false)` stopped short.
pub(crate) fn walk_outcome(world: &World, entity: Entity) -> Option<bool> {
    world.get::<RouteStops>(entity)?.outcome.map(|outcome| outcome == RouteOutcome::Arrived)
}
