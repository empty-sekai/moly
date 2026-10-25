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
pub const OWNED: &str = "the director owns this character";

/// Marks a character the director has taken.
#[derive(Component)]
pub struct DirectorOwned;

/// Takes the character of `unit`. A character in a conversation, or whose
/// loop has ended for another reason, is refused.
pub fn take(world: &mut World, unit: u32) -> Result<Entity, String> {
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

/// Hands the character of `unit` the talk `talk` to open on its next
/// decision, which builds that talk instead of drawing one (see
/// `npc_objective::DirectedTalks`). A taken character goes back to its AI
/// with no forced update owed; any other has its current objective
/// cancelled. Either way the next decision skips the Rest pause. The talk's
/// other speakers must be free, so one the director holds is refused.
pub fn open_talk(world: &mut World, unit: u32, talk: i32) -> Result<Entity, String> {
    let entity = super::cond::npc(world, unit)?;
    let speakers = {
        let tables = world
            .get_resource::<crate::fixture_activity_data::FixtureActivityTables>()
            .ok_or("the furniture activity tables are not loaded")?;
        let master = tables.talk_master(talk).ok_or_else(|| format!("talk {talk} has no master row"))?;
        tables.unit_ids_of_group(master.unit_group_id)?
    };
    if speakers.first() != Some(&unit) {
        return Err(format!("talk {talk} is opened by its first speaker, not by character {unit} (speakers {speakers:?})"));
    }
    for &speaker in &speakers[1..] {
        if super::cond::npc(world, speaker).is_ok_and(|other| world.get::<DirectorOwned>(other).is_some()) {
            return Err(format!("character {speaker} speaks in talk {talk} and the director holds it (npc.release first)"));
        }
    }
    let mind = world.get::<ObjectiveMind>(entity).ok_or_else(|| format!("character {unit} has no AI"))?;
    if let Some(reason) = mind.ai_stopped.as_deref().filter(|reason| *reason != OWNED) {
        return Err(format!("character {unit}'s AI loop has ended: {reason}"));
    }
    if world.get::<NpcActions>(entity).is_some_and(|actions| actions.current == NpcAction::Talk) {
        return Err(format!("character {unit} is in a conversation"));
    }
    if world.get::<DirectorOwned>(entity).is_some() {
        crate::npc::stop_for_external_activity(world, entity);
        world.entity_mut(entity).remove::<DirectorOwned>();
    } else {
        crate::npc::gate_entries::try_cancel_current_objective(world, entity);
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        if mind.ai_stopped.as_deref() == Some(OWNED) {
            mind.ai_stopped = None;
        }
        mind.skip_next_rest = true;
    }
    let mut talks = world.get_resource_or_insert_with(crate::npc_objective::DirectedTalks::default);
    talks.outcomes.remove(&entity);
    talks.requests.insert(entity, talk);
    info!("[director] character {unit} asked to open talk {talk} on its next decision");
    Ok(entity)
}

/// What the character's decision built for its requested talk, once it has
/// decided.
pub fn talk_outcome(world: &mut World, entity: Entity) -> Option<Result<String, String>> {
    world.get_resource_mut::<crate::npc_objective::DirectedTalks>()?.outcomes.remove(&entity)
}

/// Withdraws a talk request its character has not decided on yet.
pub fn drop_talk(world: &mut World, entity: Entity) {
    if let Some(mut talks) = world.get_resource_mut::<crate::npc_objective::DirectedTalks>() {
        talks.requests.remove(&entity);
        talks.outcomes.remove(&entity);
    }
}

/// Releases a taken character: its walk stops, its loop reopens and a forced
/// objective update is owed to its next decision.
pub fn release(world: &mut World, entity: Entity) {
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
pub fn walk(world: &mut World, entity: Entity, goal: Vec2, rests: bool) -> Result<(), String> {
    if world.get::<DirectorOwned>(entity).is_none() {
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
pub fn face(world: &mut World, entity: Entity, towards: Vec3, secs: f32) -> Result<(), String> {
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

/// Starts one of a taken character's authored no-talk activities at a placed
/// fixture through the furniture owner its AI uses (the walk there, the
/// timeline, the exit). Without `rests` the approach drops the walk law's
/// rest stops, as [`walk`] does. The phases are read with [`activity_phase`]
/// under `ticket`.
pub fn begin_activity(
    world: &mut World,
    entity: Entity,
    fixture: &super::script::FixtureRef,
    activity: Option<i32>,
    variant: Option<u32>,
    rests: bool,
    ticket: u64,
) -> Result<String, String> {
    use crate::fixture_activity_data::{ActivityOrigin, FixtureActivityTables};
    use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureTarget};
    use super::script::FixtureRef;

    if world.get::<DirectorOwned>(entity).is_none() {
        return Err("take the character before it uses a fixture (npc.take)".into());
    }
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    let from = world.get::<GlobalTransform>(entity).map_or(Vec3::ZERO, GlobalTransform::translation);
    let mut placed: Vec<(Entity, String, i32, f32)> = world
        .query::<(Entity, &FixtureActivityIdentity, Option<&GlobalTransform>)>()
        .iter(world)
        .filter(|(_, identity, _)| match fixture {
            FixtureRef::Uid(uid) => identity.uid == *uid,
            FixtureRef::Id(id) => identity.master_id == *id,
        })
        .map(|(instance, identity, at)| {
            let distance = at.map_or(f32::INFINITY, |at| super::cond::flat(at.translation(), from));
            (instance, identity.uid.clone(), identity.master_id, distance)
        })
        .collect();
    placed.sort_by(|a, b| a.3.total_cmp(&b.3).then_with(|| a.1.cmp(&b.1)));
    let Some((fixture_entity, uid, master, _)) = placed.into_iter().next() else {
        return Err(format!("no placed fixture {fixture:?} on this site"));
    };
    let tables = world.get_resource::<FixtureActivityTables>().ok_or("the furniture activity tables are not loaded")?;
    let store = world.get_resource::<crate::talk::TalkStore>().ok_or("the talk store is not loaded")?;
    let mine: Vec<_> = tables
        .catalogue_activities(store)
        .into_iter()
        .filter(|spec| spec.unit == unit && matches!(spec.key.origin, ActivityOrigin::NoTalk(_)))
        .collect();
    let spec = mine
        .iter()
        .filter(|spec| spec.fixture_id == master)
        .filter(|spec| activity.is_none_or(|id| spec.key.source_id() == id))
        .find(|spec| variant.is_none_or(|variant| spec.variant == variant as usize))
        .cloned();
    let Some(spec) = spec else {
        let mut offered: Vec<String> = mine
            .iter()
            .map(|spec| format!("{} at {} ({} variants)", spec.key.source_id(), spec.fixture_id, spec.variants))
            .collect();
        offered.dedup();
        offered.truncate(24);
        return Err(format!(
            "character {unit} has no activity {} variant {} at fixture {master}; its activities: [{}]",
            activity.map_or("(any)".into(), |id| id.to_string()),
            variant.map_or("(first)".into(), |variant| variant.to_string()),
            offered.join(", ")
        ));
    };
    let target = FixtureTarget { entity: fixture_entity, uid: uid.clone() };
    crate::npc_fixture_activity::preview::start(world, spec.key, target, ticket);
    if let Some((crate::npc_fixture_activity::preview::PreviewPhase::Failed, error)) = activity_phase(world, ticket) {
        end_activity(world, ticket, false);
        return Err(error.unwrap_or_else(|| "the activity was refused".into()));
    }
    let dropped = match world.get_mut::<RouteStops>(entity) {
        Some(mut route) if !rests => route.drop_rests(),
        _ => 0,
    };
    Ok(format!(
        "activity {} variant {}/{} at fixture {master} ({uid}); {dropped} rest stops dropped",
        spec.key.source_id(),
        spec.variant,
        spec.variants
    ))
}

/// The phase of the activity started under `ticket`, with its last note.
pub fn activity_phase(
    world: &World,
    ticket: u64,
) -> Option<(crate::npc_fixture_activity::preview::PreviewPhase, Option<String>)> {
    world
        .get_resource::<crate::npc_fixture_activity::preview::DirectorPreviews>()?
        .0
        .get(&ticket)
        .cloned()
}

/// Forgets the activity of `ticket`; `cancel` also ends its session if it
/// still runs.
pub fn end_activity(world: &mut World, ticket: u64, cancel: bool) {
    if cancel {
        crate::npc_fixture_activity::preview::cancel(world, ticket);
    }
    if let Some(mut ledger) = world.get_resource_mut::<crate::npc_fixture_activity::preview::DirectorPreviews>() {
        ledger.0.remove(&ticket);
    }
}

/// Shows the master's tweet `tweet` over a taken character, as its tweet
/// state shows one.
pub fn show_tweet(world: &mut World, entity: Entity, tweet: i32) -> Result<String, String> {
    if world.get::<DirectorOwned>(entity).is_none() {
        return Err("take the character before it tweets (npc.take)".into());
    }
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    let text = world
        .get_resource::<crate::npc_tweet::TweetTables>()
        .ok_or("the tweet master is not loaded")?
        .tweet(tweet)
        .map(|row| row.text.clone())
        .ok_or_else(|| format!("the tweet master has no row {tweet}"))?;
    let site_type = site_type(world, entity);
    world.write_message(crate::npc_state::TweetHudEvent::Show { npc: entity, unit, tweet_id: tweet, text, site_type });
    Ok(format!("tweet {tweet} shown over character {unit}"))
}

/// Hides the tweet balloon of a character.
pub fn hide_tweet(world: &mut World, entity: Entity) {
    if world.get_entity(entity).is_err() {
        world.write_message(crate::npc_state::TweetHudEvent::Retire { npc: entity });
        return;
    }
    let unit = match world.get::<CharacterUnitId>(entity) {
        Some(unit) => unit.0,
        None => world
            .get::<crate::companion::Companion>(entity)
            .map_or(0, |companion| companion.unit),
    };
    let site_type = site_type(world, entity);
    world.write_message(crate::npc_state::TweetHudEvent::Hide { npc: entity, unit, site_type });
}

/// Shows the tweet balloon of the text `text` over the entity `entity`,
/// with the tweet id `id` for the balloon's log. The balloon hangs over the
/// head of the body at `entity` ([`crate::balloon::head_root`]) in place of
/// an NPC avatar view's root, so it hangs where the roster's balloons do.
pub fn show_balloon(world: &mut World, entity: Entity, text: String, id: i32) {
    let unit = match world.get::<CharacterUnitId>(entity) {
        Some(unit) => unit.0,
        None => world
            .get::<crate::companion::Companion>(entity)
            .map_or(0, |companion| companion.unit),
    };
    let site_type = site_type(world, entity);
    world.write_message(crate::npc_state::TweetHudEvent::Show { npc: entity, unit, tweet_id: id, text, site_type });
}

fn site_type(world: &World, entity: Entity) -> Option<i32> {
    world
        .get::<NpcActions>(entity)
        .and_then(|actions| crate::npc::residency::site_type_value(&actions.site_type))
}
