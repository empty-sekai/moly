//! Shared ordinary conversation plans. A plan reserves actors by identity,
//! moves each actor on the site's navigation, then performs the final local
//! approach before exposing its pre-action and player-readable dialogue.

use std::collections::{HashMap, HashSet};
use bevy::prelude::*;
use moly_law::npc_hfsm::{common as law, LowerState, NpcPhase};
use moly_law::objective::TalkType;
use crate::npc::{CharacterUnitId, NpcAction, NpcActions, RestLifecycle};
use super::{AiTalkData, MemberRng, ObjectiveMind, TalkBackend, TalkContent, TalkSlot};
use super::controller::{NpcHfsmActor, NpcHfsmPhase, NpcLowerClock, NpcUpperBody};

const COMMUNICATION_REFRESH: i32 = 121;
const COMMUNICATION_DISTANCE: i32 = 122;
const COMMUNICATION_TWEET_TIME: i32 = 107;

#[derive(Component, Clone, Copy)]
pub struct CommonMember { pub plan: u64, pub main: Entity }

#[derive(Component, Clone)]
struct TalkReservation {
    identity: law::ReservationIdentity,
    main: Entity,
    target: [f32; 3],
}

/// Decision-boundary view of reserved actors. The reservation payload stays
/// private to its owning communication module.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Reservations<'w, 's> {
    actors: Query<'w, 's, (), With<TalkReservation>>,
}
impl Reservations<'_, '_> {
    pub(super) fn contains(&self, entity: Entity) -> bool { self.actors.contains(entity) }
}

#[derive(Clone)]
struct Member { entity: Entity, unit: u32, target: [f32; 3] }

struct Turn { entity: Entity, from: Quat, to: Quat, duration: f32, elapsed: f32 }

struct SocialPlan { id: u64, main: Entity, members: Vec<Entity>, elapsed: f32, turns: Vec<Turn> }

struct CommonPlan {
    id: u64,
    main: Entity,
    unit: u32,
    talk: moly_law::talk::FixtureTalkRow,
    members: Vec<Member>,
    phase: law::CommonPhase,
    clock: law::GatheringClock,
    phase_time: f32,
    arrived: HashSet<Entity>,
    approaching: HashSet<Entity>,
    tweet_anchor: Option<Entity>,
    requested_frame: u32,
    entered_lower: bool,
    expression_actor: Option<Entity>,
    turns: Vec<Turn>,
    fixture_data: Option<HashMap<Entity, AiTalkData>>,
    main_off_plan: f32,
}

impl CommonPlan {
    fn kind(&self) -> TalkType { if self.fixture_data.is_some() { TalkType::CommunicationWhileDoingWait } else { TalkType::CommonSomeCharacter } }
    fn lower(&self, main: bool) -> LowerState {
        match (self.fixture_data.is_some(), main) {
            (true,true) => LowerState::CommunicationWhileDoingWait,
            (true,false) => LowerState::SubCommunicationWhileDoingWait,
            (false,true) => LowerState::CommonSomeCharacterCommunication,
            (false,false) => LowerState::SubCommonSomeCharacterWait,
        }
    }
}

#[derive(Resource, Default)]
pub struct CommonPlans {
    next: u64,
    plans: Vec<CommonPlan>,
    last_communication: HashMap<Entity, f32>,
    socials: Vec<SocialPlan>,
}

fn position(world: &World, entity: Entity) -> Option<[f32; 3]> {
    Some(world.get::<Transform>(entity)?.translation.to_array())
}

fn action(world: &mut World, entity: Entity, next: NpcAction) {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut state, mut rest)) = query.get_mut(world, entity) { state.change(next, &mut rest); }
}

fn set_lower(world: &mut World, entity: Entity, state: LowerState) {
    if let Ok(mut actor) = world.get_entity_mut(entity) {
        actor.insert((NpcHfsmPhase(NpcPhase::Lower), NpcHfsmActor {
            upper: None, lower: Some(state), lower_override: None,
        }, NpcUpperBody(None), NpcLowerClock(None)));
    }
}

pub(super) fn reserved(world: &World, entity: Entity) -> bool { world.get::<TalkReservation>(entity).is_some() }

fn recruitable(world: &World, entity: Entity) -> bool {
    let Some(actions) = world.get::<NpcActions>(entity) else { return false; };
    if !actions.ready() || actions.current == NpcAction::Talk || world.get::<crate::talk::TalkHold>(entity).is_some() { return false; }
    if matches!(actions.current, NpcAction::FixtureAction | NpcAction::FixtureActionIdle | NpcAction::MainSomeFixtureAction | NpcAction::SomeFixtureAction | NpcAction::ChangeSite) { return false; }
    if let Some(actor) = world.get::<NpcHfsmActor>(entity) {
        if let Some(lower) = actor.lower { return lower.flags().can_be_recruited; }
        if let Some(upper) = actor.upper { return upper.can_be_recruited(); }
    }
    true
}

fn reservable(world: &World, entity: Entity, site: &str) -> bool {
    world.get::<NpcActions>(entity).is_some_and(|state| state.ready()
        && state.site_type == site && state.current != NpcAction::ChangeSite)
        && world.get::<CommonMember>(entity).is_none()
        && world.get::<TalkReservation>(entity).is_none()
}

fn communication_eligible(world: &World, entity: Entity, main: bool, now: f32, refresh: f32) -> bool {
    let Some(state) = world.get::<NpcActions>(entity) else { return false; };
    if !state.ready() || state.since_initialized < 1.0
        || world.get::<CommonMember>(entity).is_some() || world.get::<TalkReservation>(entity).is_some() {
        return false;
    }
    let accepted = if main { 0x4a3u32 } else { 0x1006a3u32 };
    if (accepted & (1 << state.current as u32)) == 0 { return false; }
    if let Some(kind) = world.get::<TalkSlot>(entity).and_then(TalkSlot::kind) {
        if !matches!(kind, TalkType::Common | TalkType::SingleCharacterFixture | TalkType::CommonFixture
            | TalkType::NoneTalk | TalkType::TutorialFreeWalking | TalkType::RandomWalk) { return false; }
        if !main && kind == TalkType::SingleCharacterFixture && matches!(state.current, NpcAction::Tweet | NpcAction::FixtureAction | NpcAction::FixtureActionIdle) { return false; }
    }
    if let Some(lower) = world.get::<NpcHfsmActor>(entity).and_then(|a| a.lower) {
        if matches!(lower, LowerState::SingleFixtureAction | LowerState::MultiFixtureAction | LowerState::SubFixtureAction | LowerState::SubCommunication) { return false; }
    }
    let last = world.resource::<CommonPlans>().last_communication.get(&entity).copied().unwrap_or(0.0);
    !main || (last <= now && last + refresh < now)
}

fn start_social(world: &mut World, main: Entity, nearby: &[u32], actors: &[(Entity, u32)]) {
    if !recruitable(world, main) { return; }
    let mut members: Vec<Entity> = actors.iter().filter(|(entity, unit)| nearby.contains(unit) && recruitable(world, *entity)).map(|(e,_)|*e).collect();
    if members.is_empty() { return; }
    let id = { let mut plans = world.resource_mut::<CommonPlans>(); plans.next = plans.next.wrapping_add(1); plans.next };
    members.push(main);
    let center = members.iter().filter_map(|entity| position(world, *entity)).map(Vec3::from).sum::<Vec3>() / members.len() as f32;
    let participants: Vec<u32> = members.iter().filter_map(|entity| world.get::<CharacterUnitId>(*entity).map(|u|u.0)).collect();
    let mut turns = Vec::new();
    for &entity in &members {
        crate::npc::stop_for_external_activity(world, entity);
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
            mind.executing = false; mind.body = None; mind.rest = None; mind.yield_since = None;
            mind.force_updates = 0;
        }
        if !super::controller::transition_event_lower(world, entity, if entity == main { LowerState::MainCommunication } else { LowerState::SubCommunication }) { continue; }
        world.entity_mut(entity).insert((CommonMember { plan: id, main }, crate::communication_runtime::CommunicationPlayback { epoch: id, participants: participants.clone() }));
        action(world, entity, NpcAction::Communication);
        if let Some(transform) = world.get::<Transform>(entity) {
            let delta = center - transform.translation;
            let angle = moly_law::path::angle_between((transform.rotation * Vec3::Z).to_array(), delta.to_array());
            let duration = moly_law::path::rotate_time(angle);
            if duration > 0.0 { turns.push(Turn { entity, from: transform.rotation, to: Quat::from_rotation_y(delta.x.atan2(delta.z)), duration, elapsed: 0.0 }); }
        }
    }
    world.resource_mut::<CommonPlans>().socials.push(SocialPlan { id, main, members, elapsed: 0.0, turns });
}

fn advance_social(world: &mut World, mut plan: SocialPlan, dt: f32, now: f32, cancel: bool) {
    let main_talking = world.get::<NpcActions>(plan.main).is_some_and(|s|s.current == NpcAction::Talk);
    if !main_talking { plan.elapsed += dt; }
    let ended = cancel || plan.elapsed >= 15.0 || plan.members.iter().any(|entity| !same_plan(world, *entity, plan.id) || !same_site(world, plan.main, *entity));
    if ended {
        for entity in plan.members {
            if !same_plan(world, entity, plan.id) { continue; }
            world.entity_mut(entity).remove::<CommonMember>();
            world.entity_mut(entity).remove::<crate::communication_runtime::CommunicationPlayback>();
            world.resource_mut::<CommonPlans>().last_communication.insert(entity, now);
            if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) { mind.skip_next_rest = true; mind.current = None; }
            if world.get::<NpcActions>(entity).is_some_and(|s|s.current != NpcAction::Talk) { action(world, entity, NpcAction::Idle); }
            super::controller::request_decision(world, entity);
        }
        return;
    }
    for turn in &mut plan.turns {
        if world.get::<NpcActions>(turn.entity).is_some_and(|s|s.current == NpcAction::Talk) { continue; }
        turn.elapsed += dt;
        if let Some(mut transform) = world.get_mut::<Transform>(turn.entity) { transform.rotation = crate::npc::sample_turn_rotation(turn.from, turn.to, (turn.elapsed / turn.duration).min(1.0)); }
    }
    plan.turns.retain(|turn|turn.elapsed < turn.duration);
    world.resource_mut::<CommonPlans>().socials.push(plan);
}

fn same_site(world: &World, a: Entity, b: Entity) -> bool {
    world.get::<NpcActions>(a).zip(world.get::<NpcActions>(b)).is_some_and(|(a, b)| a.site_type == b.site_type)
}

fn reachable(world: &World, entity: Entity, target: [f32; 3]) -> bool {
    let Some(from) = position(world, entity) else { return false; };
    crate::npc::navigation::face(world, entity).is_some_and(|face|
        face.agent_destination_path(Vec3::from(from), Vec3::from(target)).is_some_and(|(_, complete)| complete))
}

fn destinations(world: &World, positions: &[[f32; 3]], entities: &[Entity], rng: &mut MemberRng, all_free: bool) -> Option<Vec<[f32; 3]>> {
    if all_free && law::already_together(positions) { return Some(positions.to_vec()); }
    let face = crate::npc::navigation::face(world, *entities.first()?)?;
    let mut center = [0.0; 3];
    for p in positions { for i in 0..3 { center[i] += p[i]; } }
    for value in &mut center { *value /= positions.len() as f32; }
    for attempt in 0..5 {
        let candidate = if attempt == 0 { face.sample(center, 2.0) } else { None };
        let candidate = candidate.or_else(|| {
            let points = face.walkable();
            if points.is_empty() { return None; }
            let (x, z) = points[super::engine_int_draw(rng, points.len())];
            face.sample([x as f32 * 0.25, face.ref_y(), z as f32 * 0.25], 2.0)
        });
        let Some(center) = candidate else { continue; };
        let Some(circle) = law::circle_positions(positions, center) else { continue; };
        let mut sampled = Vec::new();
        for (index, target) in circle.into_iter().enumerate() {
            let Some(target) = face.sample(target, 1.0) else { break; };
            if !face.field.segment_walkable([center[0], center[2]], [target[0], target[2]]) { break; }
            if recruitable(world, entities[index]) && !reachable(world, entities[index], target) { break; }
            sampled.push(target);
        }
        if sampled.len() == positions.len() { return Some(sampled); }
    }
    None
}

fn move_member(world: &mut World, member: &Member, plan: &CommonPlan) -> bool {
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    if let Some(mut budget) = world.get_mut::<super::controller::NpcTransitionBudget>(member.entity) {
        if budget.frame != frame { budget.frame = frame; budget.count = 0; }
        budget.count += 1;
        if budget.count > moly_law::npc_hfsm::MAX_TRANSITIONS_PER_FRAME { return false; }
    }
    let Some(face) = crate::npc::navigation::face(world, member.entity) else { return false; };
    let Some(from) = position(world, member.entity) else { return false; };
    let polyline = face.field.generate_path([from[0], from[2]], [member.target[0], member.target[2]]);
    let mut navigator = super::controller::mover_parts(world, member.entity);
    let Some(nav) = navigator.as_mut() else { return false; };
    let Some(mut rng) = world.get::<MemberRng>(member.entity).cloned() else { return false; };
    let phase = crate::npc::depart_along_to(&CharacterUnitId(member.unit), &mut nav.walk.0,
        &mut nav.path.0, &mut nav.route, &face.walk_face(), &face, member.target, None,
        &mut rng, polyline, 0.1);
    world.entity_mut(member.entity).insert(rng);
    let Some(phase) = phase else { return false; };
    nav.motion = phase;
    super::controller::put_mover(world, member.entity, navigator.unwrap());
    super::lower::exit(world, member.entity);
    let data = plan.fixture_data.as_ref().and_then(|rows|rows.get(&member.entity)).cloned().unwrap_or_else(|| AiTalkData {
        kind: TalkType::CommonSomeCharacter, content: Some(TalkContent { master_id: plan.talk.talk_id, backend: TalkBackend::CommonSomeCharacter, is_general: Some(true) }),
        target_fixture: None, target_position: member.target, main_character: plan.unit,
        characters: plan.members.iter().map(|m|m.unit).collect(), pre_action: Some(plan.talk.tweet.clone()), locate: None, pending_factory: None,
    });
    if let Some(mut slot) = world.get_mut::<TalkSlot>(member.entity) { slot.set_current(data); }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(member.entity) {
        mind.executing = false; mind.body = None; mind.rest = None; mind.yield_since = None;
        mind.force_updates = 0; mind.skip_next_rest = true;
    }
    if let Some(mut flags) = world.get_mut::<super::controller::NpcHfsmFlags>(member.entity) { flags.needs_plan = false; }
    world.entity_mut(member.entity).insert(CommonMember { plan: plan.id, main: plan.main });
    world.write_message(crate::npc_state::TweetHudEvent::Hide { npc: member.entity, unit: member.unit, site_type: None });
    facial(world, member.entity, None);
    world.entity_mut(member.entity).insert((NpcHfsmPhase(NpcPhase::Upper), NpcHfsmActor {
        upper: Some(moly_law::npc_hfsm::UpperState::MoveToPosition), lower: None,
        lower_override: Some(plan.lower(member.entity == plan.main)),
    }, NpcUpperBody(None), NpcLowerClock(None)));
    action(world, member.entity, NpcAction::AutoMove);
    true
}

/// The initiating presenter is always the plan leader; group slot order only
/// supplies the other actors. This entry is shared by AI and directed requests.
pub fn start(world: &mut World, main: Entity, talk_id: i32) -> Result<(), String> {
    let talk = world.get_resource::<crate::player_talk::PlayerTalkStore>()
        .and_then(|store| store.common.iter().find(|row| row.talk_id == talk_id)).cloned()
        .ok_or("common conversation is not loaded")?;
    let unit = world.get::<CharacterUnitId>(main).ok_or("initiating character is absent")?.0;
    if !talk.unit_ids.contains(&(unit as i32)) { return Err("initiating character is outside the cast".into()); }
    let site = world.get::<NpcActions>(main).ok_or("initiating character has no action model")?.site_type.clone();
    let actors: HashMap<u32, Entity> = world.query::<(Entity, &CharacterUnitId)>().iter(world).map(|(e, u)| (u.0, e)).collect();
    let mut entities = vec![main];
    for id in talk.unit_ids.iter().copied().filter(|id| *id != unit as i32) {
        let entity = *actors.get(&(id as u32)).ok_or("a conversation member is absent")?;
        if !reservable(world, entity, &site) { return Err("a conversation member cannot be reserved".into()); }
        entities.push(entity);
    }
    if world.get::<CommonMember>(main).is_some() || !recruitable(world, main) { return Err("initiating character is occupied".into()); }
    let positions: Vec<_> = entities.iter().map(|e| position(world, *e)).collect::<Option<_>>().ok_or("member position unavailable")?;
    let mut rng = world.get::<MemberRng>(main).cloned().ok_or("initiating character has no generator")?;
    let targets = destinations(world, &positions, &entities, &mut rng, entities.iter().all(|e| recruitable(world, *e)));
    world.entity_mut(main).insert(rng);
    let targets = targets.ok_or("no connected gathering positions are reachable")?;
    let now = world.resource::<Time>().elapsed_secs();
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let id = { let mut plans = world.resource_mut::<CommonPlans>(); plans.next = plans.next.wrapping_add(1); plans.next };
    if let Some(mut budget) = world.get_mut::<super::controller::NpcTransitionBudget>(main) { if budget.frame != frame { budget.frame = frame; budget.count = 0; } }
    let members = entities.iter().zip(targets).map(|(e, target)| Member { entity: *e, unit: world.get::<CharacterUnitId>(*e).unwrap().0, target }).collect();
    let plan = CommonPlan { id, main, unit, talk, members, phase: law::CommonPhase::Gathering,
        clock: law::GatheringClock::new(now), phase_time: 0.0, arrived: HashSet::new(), approaching: HashSet::new(),
        tweet_anchor: None, requested_frame: frame, entered_lower: false, expression_actor: None, turns: Vec::new(), fixture_data: None, main_off_plan: 0.0 };
    // Supporting presenters receive their gather upper before the main's
    // move begins; this is the plan factory's registration order.
    let ordered_members: Vec<Member> = plan.members.iter().skip(1).chain(plan.members.iter().take(1)).cloned().collect();
    for member in &ordered_members {
        if recruitable(world, member.entity) {
            if !move_member(world, member, &plan) {
                let failed = member.entity;
                // End the partially installed plan without clearing unrelated
                // member state; teardown checks each actor's plan identity.
                cleanup(world, plan);
                return Err(format!("member {failed:?} could not begin its gathering move"));
            }
        } else {
            world.entity_mut(member.entity).insert(TalkReservation { identity: law::ReservationIdentity {
                kind: law::ReservationKind::Common, plan: id, main: unit }, main, target: member.target });
        }
    }
    world.resource_mut::<CommonPlans>().plans.push(plan);
    Ok(())
}

/// A fixture's while-doing wait uses its authored locator targets with the
/// same reservation, navigation and final-approach owner as ordinary groups.
pub(super) fn start_while_doing(world: &mut World, main: Entity, talk: moly_law::talk::FixtureTalkRow,
    members: Vec<(Entity, AiTalkData)>) -> Result<(), String> {
    if members.len() < 2 { return Err("while-doing communication requires its cast".into()); }
    let unit = world.get::<CharacterUnitId>(main).ok_or("main disappeared")?.0;
    let site = world.get::<NpcActions>(main).ok_or("main action model missing")?.site_type.clone();
    for (entity, _) in &members {
        if *entity != main && !reservable(world,*entity,&site) { return Err("while-doing member cannot be reserved".into()); }
    }
    let now = world.resource::<Time>().elapsed_secs();
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let id = { let mut store=world.resource_mut::<CommonPlans>(); store.next=store.next.wrapping_add(1); store.next };
    let mut ordered=members;
    ordered.sort_by_key(|(entity,_)|*entity != main);
    let data: HashMap<_,_> = ordered.iter().cloned().collect();
    let cast: Vec<_> = ordered.iter().map(|(entity,data)|Member { entity:*entity, unit:world.get::<CharacterUnitId>(*entity).map_or(0,|u|u.0), target:data.target_position }).collect();
    let plan = CommonPlan {id,main,unit,talk,members:cast,phase:law::CommonPhase::Gathering,clock:law::GatheringClock::new(now),phase_time:0.0,
        arrived:HashSet::new(),approaching:HashSet::new(),tweet_anchor:None,requested_frame:frame,entered_lower:false,expression_actor:None,
        turns:Vec::new(),fixture_data:Some(data),main_off_plan:0.0};
    let ordered: Vec<_> = plan.members.iter().skip(1).chain(plan.members.iter().take(1)).cloned().collect();
    for member in &ordered {
        if recruitable(world,member.entity) {
            if !move_member(world,member,&plan) { cleanup(world,plan); return Err("while-doing gather route failed".into()); }
        } else {
            world.entity_mut(member.entity).insert(TalkReservation {identity:law::ReservationIdentity {kind:law::ReservationKind::WhileDoingWait,plan:id,main:unit}, main,target:member.target});
        }
    }
    world.resource_mut::<CommonPlans>().plans.push(plan);
    Ok(())
}

fn same_plan(world: &World, entity: Entity, plan: u64) -> bool { world.get::<CommonMember>(entity).is_some_and(|m| m.plan == plan) }

fn hide_tweet(world: &mut World, plan: &mut CommonPlan) {
    if let Some(anchor) = plan.tweet_anchor.take() {
        world.write_message(crate::npc_state::TweetHudEvent::Hide { npc: anchor, unit: plan.unit, site_type: None });
        world.despawn(anchor);
    }
    if let Some(mut state) = world.get_mut::<NpcActions>(plan.main) { state.is_tweeting = false; }
}

fn facial(world: &mut World, entity: Entity, tweet: Option<&moly_law::talk::TweetRef>) {
    let Some(unit) = world.get::<CharacterUnitId>(entity).map(|u| u.0) else { return; };
    let Some(toon) = world.get::<crate::character_material::ToonMaterials>(entity) else { return; };
    let eye = toon.slot_handle("eye").cloned();
    let mouth = toon.slot_handle("mouth").cloned();
    let Some(tables) = world.get_resource::<crate::alone_action_runtime::FacialTables>() else { return; };
    let Some((default_eye, default_mouth)) = tables.default_patterns(unit) else { return; };
    let eye_pattern = tables.eye_pattern(tweet.map(|t| t.eye.as_str()).filter(|s| !s.is_empty()).unwrap_or(default_eye));
    let mouth_pattern = tables.lip_pattern(tweet.map(|t| t.mouth.as_str()).filter(|s| !s.is_empty()).unwrap_or(default_mouth));
    let mut materials = world.resource_mut::<Assets<crate::character_material::CharacterMaterial>>();
    if let Some(eye) = eye { crate::alone_action_runtime::apply_eye_pattern(&mut materials, &eye, eye_pattern); }
    if let (Some(mouth), Some(pattern)) = (mouth, mouth_pattern) { crate::alone_action_runtime::apply_mouth_pattern(&mut materials, &mouth, pattern); }
}

fn cleanup(world: &mut World, mut plan: CommonPlan) {
    hide_tweet(world, &mut plan);
    if let Some(entity) = plan.expression_actor.take() {
        if world.get::<NpcActions>(entity).is_some_and(|state| state.current != NpcAction::Talk) { facial(world, entity, None); }
    }
    let now = world.resource::<Time>().elapsed_secs();
    for member in plan.members {
        if world.get::<TalkReservation>(member.entity).is_some_and(|r| r.identity.plan == plan.id) {
            world.entity_mut(member.entity).remove::<TalkReservation>();
        }
        if !same_plan(world, member.entity, plan.id) { continue; }
        world.entity_mut(member.entity).remove::<CommonMember>();
        world.entity_mut(member.entity).remove::<crate::communication_runtime::CommunicationPlayback>();
        crate::npc::stop_movement(world, member.entity);
        if let Some(mut slot) = world.get_mut::<TalkSlot>(member.entity) { slot.reset_ai_talk_data(); }
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(member.entity) {
            mind.skip_next_rest = true; mind.executing = false; mind.body = None; mind.rest = None;
            mind.current = None; mind.yield_since = None;
        }
        if world.get::<NpcActions>(member.entity).is_some_and(|s| s.current != NpcAction::Talk) { action(world, member.entity, NpcAction::Idle); }
        super::controller::request_decision(world, member.entity);
        world.resource_mut::<CommonPlans>().last_communication.insert(member.entity, now);
    }
}

fn advance_plan(world: &mut World, plan: &mut CommonPlan, dt: f32, now: f32) {
    if world.get::<NpcActions>(plan.main).is_none() { plan.phase = law::CommonPhase::Complete; return; }
    let main_matches = same_plan(world, plan.main, plan.id) && world.get::<NpcHfsmActor>(plan.main).is_some_and(|a|a.upper.is_some() || a.lower == Some(plan.lower(true)));
    if !main_matches {
        plan.main_off_plan += dt;
        if plan.fixture_data.is_some() || plan.main_off_plan >= law::SHARED_PLAN_GRACE_SECONDS { plan.phase = law::CommonPhase::Complete; }
        return;
    }
    plan.main_off_plan = 0.0;
    if plan.members.iter().any(|m| !same_site(world, plan.main, m.entity)) { plan.phase = law::CommonPhase::Complete; return; }
    for turn in &mut plan.turns {
        if world.get::<NpcActions>(turn.entity).is_some_and(|s| s.current == NpcAction::Talk) { continue; }
        turn.elapsed += dt;
        if let Some(mut transform) = world.get_mut::<Transform>(turn.entity) {
            transform.rotation = crate::npc::sample_turn_rotation(turn.from, turn.to, (turn.elapsed / turn.duration).min(1.0));
        }
    }
    plan.turns.retain(|t| t.elapsed < t.duration);
    match plan.phase {
        law::CommonPhase::Gathering => {
            for member in &plan.members {
                if same_plan(world, member.entity, plan.id)
                    && world.get::<NpcHfsmPhase>(member.entity).is_some_and(|p| p.0 == NpcPhase::Upper)
                    && world.get::<crate::npc::MotionPhase>(member.entity).is_some_and(|p| matches!(p, crate::npc::MotionPhase::Dwelling { .. })) {
                    let distance = position(world, member.entity).map(|at| law::distance(at, member.target)).unwrap_or(f32::INFINITY);
                    if distance >= law::ARRIVAL_DISTANCE {
                        // The main's failed upper never enters its lower;
                        // supporting actors leave their failed move for retry.
                        if member.entity == plan.main { plan.phase = law::CommonPhase::Complete; return; }
                        world.entity_mut(member.entity).remove::<CommonMember>();
                        continue;
                    }
                    set_lower(world, member.entity, plan.lower(member.entity == plan.main));
                    action(world, member.entity, NpcAction::Idle);
                    if member.entity == plan.main { plan.entered_lower = true; plan.clock = law::GatheringClock::new(now); }
                }
            }
            if !plan.entered_lower { return; }
            let pending = plan.members.iter().any(|m| world.get::<TalkReservation>(m.entity).is_some_and(|r| r.identity.plan == plan.id));
            let moving = plan.members.iter().any(|m| world.get::<crate::npc::MotionPhase>(m.entity).is_some_and(|p| matches!(p, crate::npc::MotionPhase::Walking)));
            if !plan.clock.advance(dt, now, pending, moving) { plan.phase = law::CommonPhase::Complete; return; }
            for member in plan.members.clone() {
                if !same_plan(world, member.entity, plan.id) {
                    if let Some(reservation) = world.get::<TalkReservation>(member.entity).cloned() {
                        if reservation.identity.plan == plan.id && recruitable(world, member.entity)
                            && world.get::<super::controller::NpcHfsmFlags>(member.entity).is_some_and(|flags| flags.needs_plan) {
                            world.entity_mut(member.entity).remove::<TalkReservation>();
                            let valid = law::valid_reservation(reservation.identity, Some((plan.id, plan.unit, now < plan.clock.deadline)), None);
                            if valid && reservation.main == plan.main && reachable(world, member.entity, reservation.target) { move_member(world, &member, plan); }
                        }
                    } else if plan.clock.can_retry() && recruitable(world, member.entity) {
                        move_member(world, &member, plan);
                    } else if !plan.clock.can_retry() { plan.phase = law::CommonPhase::Complete; return; }
                    continue;
                }
                if plan.arrived.contains(&member.entity) {
                    if world.get::<TalkSlot>(member.entity).and_then(TalkSlot::kind) != Some(plan.kind()) { plan.phase = law::CommonPhase::Complete; return; }
                    continue;
                }
                let Some(at) = position(world, member.entity) else { plan.phase = law::CommonPhase::Complete; return; };
                let stopped = world.get::<crate::npc::MotionPhase>(member.entity).is_some_and(|p| matches!(p, crate::npc::MotionPhase::Dwelling { .. }));
                if !plan.approaching.contains(&member.entity) && law::distance(at, member.target) < law::ARRIVAL_DISTANCE && stopped {
                    crate::npc::stop_movement(world, member.entity);
                    plan.approaching.insert(member.entity); plan.clock.progress();
                }
                if plan.approaching.contains(&member.entity) {
                    let next = law::approach_step(at, member.target, dt);
                    crate::npc::force_set_position(world, member.entity, next);
                    if law::distance(next, member.target) <= law::FINAL_APPROACH_EPSILON {
                        plan.approaching.remove(&member.entity); plan.arrived.insert(member.entity); plan.clock.progress();
                        action(world, member.entity, NpcAction::Communication);
                        world.entity_mut(member.entity).insert(crate::communication_runtime::CommunicationPlayback {
                            epoch: plan.id, participants: plan.members.iter().map(|m| m.unit).collect(),
                        });
                    }
                } else if stopped && law::distance(at, member.target) >= law::ARRIVAL_DISTANCE {
                    move_member(world, &member, plan);
                }
            }
            if plan.arrived.len() == plan.members.len() {
                let center = plan.members.iter().filter_map(|m| position(world, m.entity)).map(Vec3::from).sum::<Vec3>() / plan.members.len() as f32;
                for member in &plan.members {
                    if let Some(transform) = world.get::<Transform>(member.entity) {
                        let delta = center - transform.translation;
                        if delta.length() >= 0.05 {
                            let forward = (transform.rotation * Vec3::Z).to_array();
                            let angle = moly_law::path::angle_between(forward, delta.to_array());
                            let duration = moly_law::path::rotate_time(angle);
                            if duration > 0.0 { plan.turns.push(Turn { entity: member.entity, from: transform.rotation,
                                to: Quat::from_rotation_y(delta.x.atan2(delta.z)), duration, elapsed: 0.0 }); }
                        }
                    }
                }
                if plan.talk.tweet.id != 0 && !plan.talk.tweet.text.is_empty() {
                    let anchor = world.spawn((Transform::from_translation(center), GlobalTransform::default())).id();
                    world.write_message(crate::npc_state::TweetHudEvent::Show { npc: anchor, unit: plan.unit,
                        tweet_id: plan.talk.tweet.id, text: plan.talk.tweet.text.clone(), site_type: None });
                    plan.tweet_anchor = Some(anchor);
                    // Only the authored special expression recipient is
                    // changed. It is association metadata, never the leader.
                    let expression_unit = world.get_resource::<crate::npc_talk_lottery::TalkExtraTables>()
                        .and_then(|tables| tables.some_character_talks.as_ref().ok())
                        .and_then(|rows| rows.iter().find(|row| row.talk_id == plan.talk.talk_id)).map(|row| row.main_unit);
                    if expression_unit == Some(18) && !plan.talk.tweet.eye.is_empty() {
                        if let Some(member) = plan.members.iter().find(|member| member.unit == 18) {
                            facial(world, member.entity, Some(&plan.talk.tweet));
                            plan.expression_actor = Some(member.entity);
                        }
                    }
                    if let Some(mut state) = world.get_mut::<NpcActions>(plan.main) {
                        state.is_tweeting = true; state.tweet_id = plan.talk.tweet.id;
                        if state.current != NpcAction::Tweet {
                            state.before = state.current; state.current = NpcAction::Tweet; state.elapsed = 0.0;
                        }
                    }
                }
                plan.phase = law::CommonPhase::Tweeting; plan.phase_time = 0.0;
            }
        }
        law::CommonPhase::Tweeting => {
            plan.phase_time += dt;
            let talking = world.get::<NpcActions>(plan.main).is_some_and(|s| s.current == NpcAction::Talk);
            let timeout = world.resource::<crate::client_config::ClientConfigs>().int(COMMUNICATION_TWEET_TIME) as f32 / 1000.0;
            if talking || plan.phase_time >= timeout {
                hide_tweet(world, plan);
                if !talking { if let Some(entity) = plan.expression_actor.take() { facial(world, entity, None); } }
                plan.phase = law::CommonPhase::WaitingTalkEnd;
            }
        }
        law::CommonPhase::WaitingTalkEnd => {
            if world.get::<NpcActions>(plan.main).is_some_and(|s| s.current != NpcAction::Talk) { plan.phase = law::CommonPhase::Complete; }
        }
        law::CommonPhase::Complete => {}
    }
}

/// Installed at the presenter's communication step. Explicit requests are
/// consumed here before automatic selection; no fixture is synthesized.
pub fn update(world: &mut World) {
    let Some(dt) = world.get_resource::<crate::npc_clock::NpcClock>().filter(|c| c.ready()).map(|c| c.delta()) else { return; };
    if world.get_resource::<crate::client_config::ClientConfigs>().is_none() { return; }
    let now = world.resource::<Time>().elapsed_secs();
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let stale: Vec<Entity> = world.query::<(Entity, &TalkReservation)>().iter(world)
        .filter(|(_, r)| !world.resource::<CommonPlans>().plans.iter().any(|plan| plan.id == r.identity.plan && plan.main == r.main))
        .map(|(entity, _)| entity).collect();
    for entity in stale { world.entity_mut(entity).remove::<TalkReservation>(); }
    let editing = world.get_resource::<crate::field_modes::EditSessionActive>().is_some_and(|e| e.is_active());
    let socials = std::mem::take(&mut world.resource_mut::<CommonPlans>().socials);
    for social in socials { advance_social(world, social, dt, now, editing); }
    if editing {
        let plans = std::mem::take(&mut world.resource_mut::<CommonPlans>().plans);
        for plan in plans { cleanup(world, plan); }
        return;
    }
    let mut plans = std::mem::take(&mut world.resource_mut::<CommonPlans>().plans);
    for mut plan in plans.drain(..) {
        if frame != plan.requested_frame { advance_plan(world, &mut plan, dt, now); }
        if plan.phase == law::CommonPhase::Complete { cleanup(world, plan); }
        else { world.resource_mut::<CommonPlans>().plans.push(plan); }
    }
    let common_ids: HashSet<i32> = world.get_resource::<crate::player_talk::PlayerTalkStore>().map(|s| s.common.iter().map(|r| r.talk_id).collect()).unwrap_or_default();
    let requests: Vec<_> = world.get_resource::<super::DirectedTalks>().map(|r| r.requests.iter().filter(|(_, id)| common_ids.contains(id)).map(|(e,id)|(*e,*id)).collect()).unwrap_or_default();
    for (entity, id) in requests {
        world.resource_mut::<super::DirectedTalks>().requests.remove(&entity);
        let outcome = start(world, entity, id).map(|()| format!("common conversation {id} gathering"));
        world.resource_mut::<super::DirectedTalks>().outcomes.insert(entity, outcome);
    }
    if !crate::communication_runtime::ready(world) || world.get_resource::<crate::server_panel::ServerPanel>().is_none_or(|p| p.is_tutorial())
        || world.get_resource::<crate::alone_action_runtime::AloneExecutionGate>().is_some_and(|g| g.cutscene_active) { return; }
    let config = world.resource::<crate::client_config::ClientConfigs>();
    let refresh = config.float(COMMUNICATION_REFRESH);
    let reach = config.float(COMMUNICATION_DISTANCE);
    let overlap = config.float(crate::client_config::KEY_CHARACTER_OVERLAP_DISTANCE);
    let Some(list) = world.get_resource::<crate::server_panel::TalkDataStore>() else { return; };
    let offered: Vec<i32> = list.talk_list().iter().map(|row| row.talk_id).collect();
    let cast: HashMap<i32, Vec<u32>> = world.get_resource::<crate::player_talk::PlayerTalkStore>()
        .map(|store| store.common.iter().map(|row| (row.talk_id, row.unit_ids.iter().map(|id| *id as u32).collect())).collect()).unwrap_or_default();
    let actors: Vec<(Entity, u32)> = world.query::<(Entity, &CharacterUnitId)>().iter(world).map(|(e,u)|(e,u.0)).collect();
    for &(entity, unit) in &actors {
        if !communication_eligible(world, entity, true, now, refresh) { continue; }
        let Some(at) = position(world, entity) else { continue; };
        let site = world.get::<NpcActions>(entity).unwrap().site_type.clone();
        let near: Vec<u32> = actors.iter().filter_map(|(other, id)| {
            if *other == entity || !recruitable(world, *other) || !communication_eligible(world, *other, false, now, refresh) { return None; }
            if world.get::<NpcActions>(*other)?.site_type != site { return None; }
            let d = law::distance(at, position(world, *other)?);
            (d > overlap && d <= reach).then_some(*id)
        }).collect();
        // A main is chosen by presenter order: every eligible nearby
        // character must have a greater unit id at this entry point.
        if near.is_empty() || near.iter().any(|other| *other <= unit)
            || !actors.iter().filter(|(_, id)| near.contains(id)).any(|(other, _)| {
                let last = world.resource::<CommonPlans>().last_communication.get(other).copied().unwrap_or(0.0);
                last <= now && last + refresh < now
            }) { continue; }
        // The communication hook draws its 50% gate before looking for rows,
        // and one integer pick after collecting rows in the offered order.
        let mut rng = world.get::<MemberRng>(entity).cloned().unwrap();
        let gate = super::engine_int_draw(&mut rng, 100);
        let selected = if gate <= 49 {
            let pool: Vec<i32> = offered.iter().copied().filter(|id| cast.get(id).is_some_and(|members|
                members.contains(&unit) && members.iter().any(|id| *id != unit && near.contains(id)))).collect();
            if pool.is_empty() { None } else { Some(pool[super::engine_int_draw(&mut rng, pool.len())]) }
        } else { None };
        world.entity_mut(entity).insert(rng);
        let started = selected.is_some_and(|id| start(world, entity, id).is_ok());
        if !started { start_social(world, entity, &near, &actors); }
    }
}
