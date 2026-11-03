//! A character taken from its AI: the action steps walk, turn, use a fixture
//! and show a tweet with it, and its own AI makes no decision until it is
//! given back.
//!
//! Taking a character cancels its current objective, stops its walk and ends
//! its AI loop's decisions the way an ended loop is marked, so neither the
//! decision loop nor the presenter's greeting and overlap calls drive it.
//! Giving it back reopens the loop and owes it a forced objective update, so
//! it decides afresh.
//!
//! The action steps that take characters keep who took which in
//! [`crate::act::Taken`]; this module is the body's own part of taking and
//! giving back.

use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use crate::npc::{
    self, CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, RestLifecycle, RouteOutcome, RouteStops, WalkState,
};
use crate::npc_objective::{MemberRng, ObjectiveFace, ObjectiveMind};

/// The reason a taken character's loop shows while another program owns it.
pub const OWNED: &str = "the director owns this character";

pub use crate::npc::DirectedOwned;

/// Takes the character of `unit`. A character in a conversation, or whose
/// loop has ended for another reason, is refused.
pub fn take(world: &mut World, unit: u32) -> Result<Entity, String> {
    let entity = crate::act::Taken::body(world, unit)?;
    if world.get::<DirectedOwned>(entity).is_some() {
        return Ok(entity);
    }
    let mind = world.get::<ObjectiveMind>(entity).ok_or_else(|| format!("character {unit} has no AI"))?;
    if let Some(reason) = &mind.ai_stopped {
        return Err(format!("character {unit}'s AI loop has ended: {reason}"));
    }
    if world.get::<NpcActions>(entity).is_some_and(|actions| actions.current == NpcAction::Talk) {
        return Err(format!("character {unit} is in a conversation"));
    }
    let cancelled = npc::gate_entries::try_cancel_current_objective(world, entity);
    npc::stop_for_external_activity(world, entity);
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.ai_stopped = Some(OWNED.into());
    }
    world.entity_mut(entity).insert(DirectedOwned);
    info!("[director] character {unit} taken (its objective cancelled: {cancelled}); its AI makes no decision until released");
    Ok(entity)
}

/// Releases a taken character: its walk stops, its loop reopens and a forced
/// objective update is owed to its next decision.
pub fn release(world: &mut World, entity: Entity) {
    if world.get_entity(entity).is_err() || world.get::<DirectedOwned>(entity).is_none() {
        return;
    }
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    npc::stop_for_external_activity(world, entity);
    world.entity_mut(entity).remove::<DirectedOwned>();
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        if mind.ai_stopped.as_deref() == Some(OWNED) {
            mind.ai_stopped = None;
        }
    }
    let updated = npc::gate_entries::force_update_objective(world, entity);
    info!("[director] character {unit} released (forced objective update owed: {updated})");
}

/// Whether the character of `unit` is taken.
pub fn owned(world: &mut World, unit: u32) -> bool {
    crate::act::Taken::body(world, unit).is_ok_and(|entity| world.get::<DirectedOwned>(entity).is_some())
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
pub fn walk(world: &mut World, entity: Entity, goal: Vec2, rests: bool) -> Result<(), String> {
    if world.get::<DirectedOwned>(entity).is_none() {
        return Err("take the character before walking it (npc.take)".into());
    }
    let mut params = SystemState::<WalkParams>::new(world);
    let (face, objective, origins, mut actors) = params.get_mut(world).expect("system params");
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
    *phase = npc::depart(
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
    npc::declare_navigation_action(&mut actions, &mut rest, &phase, &route);
    Ok(())
}

/// Turns a taken character toward `towards` over `secs` with the greeting's
/// look-at tween. The tween runs only while the character stands, so a
/// walking character is refused (the `npc.face` step waits for it to stand).
pub fn face(world: &mut World, entity: Entity, towards: Vec3, secs: f32) -> Result<(), String> {
    if world.get::<DirectedOwned>(entity).is_none() {
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
pub fn standing(world: &World, entity: Entity) -> bool {
    matches!(world.get::<MotionPhase>(entity), Some(MotionPhase::Dwelling { .. }))
}

/// Whether a turn started by [`face`] is still running.
pub fn facing(world: &World, entity: Entity) -> bool {
    world.get::<crate::npc_look_at::NpcLookAt>(entity).is_some()
}

/// The walk's end: `Some(true)` arrived, `Some(false)` stopped short.
pub fn walk_outcome(world: &World, entity: Entity) -> Option<bool> {
    world.get::<RouteStops>(entity)?.outcome.map(|outcome| outcome == RouteOutcome::Arrived)
}
