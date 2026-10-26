//! Event-owned NPC lower states. The event that starts a performance keeps
//! ownership until its real completion signal; the controller supplies the
//! transition budget and the pause between objectives.

use bevy::prelude::*;
use bevy::ecs::system::SystemState;
use bevy::animation::graph::AnimationNodeIndex;
use moly_assets::json::master::{MasterData, MasterTable};
use moly_law::npc_hfsm::LowerState;
use moly_law::objective::{ObjectiveType, TalkType};

use crate::field_modes::{GameStateType, HeldGameState};
use crate::fixture_activity_state::{FixtureActivityOwner, FixtureActivityReservations, FixtureTarget};
use crate::fixture_activity_timeline::{self as timeline, FixtureActivityTimelines, StartTimeline, TimelineStatus, TimelineToken};
use crate::npc::{CharacterUnitId, NpcAction, NpcActions, RestLifecycle};
use super::{AiTalkData, BodyWait, ObjectiveMind, TalkSlot};
use super::controller::{self, NpcHfsmActor, NpcHfsmFlags, NpcHfsmScopeResource};

pub mod tutorial;

#[derive(Component)]
struct EventLower {
    state: LowerState,
    entered: u32,
    elapsed: f32,
    phase: u8,
    token: Option<TimelineToken>,
    owner: Option<FixtureActivityOwner>,
    fixture_reaction: Option<(i32, Entity)>,
    photo: Option<PhotoCall>,
    birthday_pending: bool,
}

impl EventLower {
    fn new(state: LowerState, entered: u32) -> Self {
        Self { state, entered, elapsed: 0.0, phase: 0, token: None, owner: None, fixture_reaction: None, photo: None, birthday_pending: false }
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PhotoRow {
    game_character_unit_id: u32,
    seq: i32,
    motion_name: String,
    emoticon_name: Option<String>,
    expression_eye_name: String,
    expression_mouth_name: String,
}

#[derive(Resource, Default)]
struct PhotoMode {
    active: bool,
    previous_game_state: Option<HeldGameState>,
    rows: Option<Result<Vec<PhotoRow>, String>>,
}

#[derive(Component)]
struct TutorialEntered;

#[derive(Component)]
struct DeferredBirthday { yielded_at: u32 }

/// The tutorial runner's completion callback is independent of global
/// tutorial progression: the next tutorial step may still hold the world.
pub fn notify_tutorial_complete(world: &mut World, entity: Entity) {
    if let Some(mut lower)=world.get_mut::<EventLower>(entity) {
        if lower.state==LowerState::TutorialWait { lower.phase=1; }
    }
}

/// The controller calls this at DecideAndTransitionToNext, before its plan
/// factory. The empty waiting lower preserves the current tutorial talk data.
pub fn return_to_tutorial_waiting(world: &mut World, entity: Entity) -> bool {
    if !world.get_resource::<crate::server_panel::ServerPanel>().is_some_and(|panel|panel.is_tutorial())
        || world.get::<TalkSlot>(entity).and_then(TalkSlot::kind) != Some(TalkType::TutorialWaiting) { return false; }
    tutorial::reenter_waiting(world, entity).is_ok()
}

struct PhotoCall {
    sequence: i32,
    requested: bool,
    move_to: Option<crate::npc_fixture_activity::LocalMove>,
    last_camera: Option<Vec3>,
    camera_target: Option<Entity>,
    row: Option<PhotoRow>,
    emote: Option<crate::emoticon::EmoteTicket>,
    motion_error: Option<String>,
}

const PHOTO_ROWS: MasterTable<Vec<PhotoRow>> = MasterTable {
    table: "mysekaiPhotoGameCharacterUnitCalls",
    name: "photo character call sequences",
    parse: |text| serde_json::from_str(text).map_err(|error| format!("photo call rows: {error}")),
};

fn load_photo_rows(world: &mut World) {
    world.init_resource::<PhotoMode>();
    if world.resource::<PhotoMode>().rows.is_some() { return; }
    let Some(mut masters) = world.get_resource_mut::<MasterData>() else { return; };
    masters.request(&PHOTO_ROWS);
    if let Some(result) = masters.take(&PHOTO_ROWS) {
        world.resource_mut::<PhotoMode>().rows = Some(result.map_err(|error|error.to_string()));
    }
}

pub fn photo_mode_active(world: &World) -> bool {
    world.get_resource::<PhotoMode>().is_some_and(|mode|mode.active)
}

/// The photo screen's enter/exit. The lower itself watches the game state;
/// leaving disposes its current call, reserved position and facial override.
pub fn set_photo_mode(world: &mut World, active: bool) -> Result<(), String> {
    load_photo_rows(world);
    if active {
        match world.resource::<PhotoMode>().rows.as_ref() {
            Some(Ok(_)) => {}
            Some(Err(reason)) => return Err(reason.clone()),
            None => return Err("photo character calls are loading".into()),
        }
        if !photo_mode_active(world) {
            let previous = world.get_resource::<HeldGameState>().copied();
            let mut mode = world.resource_mut::<PhotoMode>();
            mode.previous_game_state = previous; mode.active = true;
            world.insert_resource(HeldGameState(GameStateType::PhotoShot));
        }
    } else if photo_mode_active(world) {
        let previous = { let mut mode = world.resource_mut::<PhotoMode>(); mode.active = false; mode.previous_game_state.take() };
        if world.get_resource::<HeldGameState>().is_some_and(|state|state.0 == GameStateType::PhotoShot) {
            if let Some(previous) = previous { world.insert_resource(previous); }
            else { world.remove_resource::<HeldGameState>(); }
        }
    }
    Ok(())
}

/// CallCharacter event: each subscribed lower cancels the previous call and
/// advances its own authored sequence. One press fans out in roster order.
pub fn call_photo_characters(world: &mut World) -> Result<(), String> {
    if !photo_mode_active(world) { return Err("photo mode is not active".into()); }
    let mut actors: Vec<(u32, Entity)> = world.query::<(Entity, &CharacterUnitId, &EventLower)>().iter(world)
        .filter(|(_, _, event)|event.state == LowerState::PhotoShot)
        .map(|(entity, unit, _)|(unit.0,entity)).collect();
    actors.sort_unstable();
    for (_, entity) in actors { request_photo_call(world, entity); }
    Ok(())
}

/// The selected portrait sends the event only to its own character.
pub fn call_photo_character(world: &mut World, unit: u32) -> Result<(), String> {
    if !photo_mode_active(world) { return Err("photo mode is not active".into()); }
    let actor = world.query::<(Entity,&CharacterUnitId,&NpcHfsmActor)>().iter(world)
        .find(|(_,id,_)|id.0==unit).map(|(entity,_,_)|entity).ok_or("selected character is unavailable")?;
    if world.get::<EventLower>(actor).is_none_or(|event|event.state!=LowerState::PhotoShot) {
        let actions=world.get::<NpcActions>(actor).ok_or("selected character has no action model")?;
        if matches!(actions.current,NpcAction::FixtureAction|NpcAction::FixtureActionIdle|NpcAction::PhotoShot|NpcAction::Tweet|NpcAction::ChangeSite)
            || world.get::<crate::npc_fixture_activity::NpcFixtureAnimationOwner>(actor).is_some()
            || world.get::<NpcHfsmActor>(actor).and_then(|actor|actor.lower).is_some_and(|lower|lower.flags().blocks_photo_shot_capture) {
            return Ok(());
        }
        if !enter_photo(world,actor) { return Ok(()); }
    }
    request_photo_call(world,actor);
    if let Some(mut blink)=world.get_mut::<crate::npc_view::NpcBlink>(actor) { blink.set_enabled(true); }
    Ok(())
}

fn request_photo_call(world: &mut World, entity: Entity) {
    if let Some(ticket) = world.get::<EventLower>(entity).and_then(|event|event.photo.as_ref()).and_then(|call|call.emote) {
        if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.forget(ticket); }
    }
    if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.hide(entity); }
    world.entity_mut(entity).remove::<crate::npc_look_at::NpcLookAt>();
    world.entity_mut(entity).remove::<crate::npc_fixture_activity::NpcFixtureMotionOwner>();
    if let Some(mut motion)=world.get_mut::<crate::npc::MotionPhase>(entity) { *motion=crate::npc::MotionPhase::Dwelling { remaining: None }; }
    if let Some(mut event) = world.get_mut::<EventLower>(entity) {
        event.phase = 0;
        if let Some(call) = event.photo.as_mut() { call.requested = true; call.move_to = None; call.emote = None; }
    }
    if let Some(mut actions)=world.get_mut::<NpcActions>(entity) { actions.reserved_move_position=None; }
}

pub fn enrolled(world: &World, entity: Entity) -> bool {
    world.get::<CharacterUnitId>(entity).is_some_and(|unit| {
        world.get_resource::<NpcHfsmScopeResource>().is_some_and(|scope| scope.0.enrols(unit.0))
    })
}

pub(super) fn owns(world: &World, entity: Entity) -> bool {
    world.get::<EventLower>(entity).is_some()
        || world.get_resource::<crate::npc::change_site_state::ChangeSiteRuns>().is_some_and(|runs| runs.owns(entity))
}

fn frame(world: &World) -> u32 {
    world.get_resource::<bevy::diagnostic::FrameCount>().map_or(0, |frame| frame.0)
}

fn action(world: &mut World, entity: Entity, next: NpcAction) {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = query.get_mut(world, entity) {
        actions.change(next, &mut rest);
    }
}

fn mark_objective(world: &mut World, entity: Entity, state: LowerState) {
    let at = frame(world);
    let kind = match state {
        LowerState::EntrySite | LowerState::EntryStandby => ObjectiveType::EntrySite,
        LowerState::TutorialWait => ObjectiveType::TutorialWait,
        LowerState::CutScene => ObjectiveType::CutScene,
        LowerState::BirthdayParty => ObjectiveType::BirthdayPartyWait,
        LowerState::PhotoShot => ObjectiveType::PhotoShot,
        LowerState::AfterEditReaction => ObjectiveType::AfterEditLayoutReaction,
        LowerState::HighPriorityFixtureReaction => ObjectiveType::AfterEditLayoutHighPriorityTalkReaction,
        LowerState::ImmediatelyTimeline => ObjectiveType::ImmediatelyFixtureTimeline,
        LowerState::ChangeSite => ObjectiveType::ChangeSite,
        _ => return,
    };
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.begin_objective(kind);
        mind.executing = true;
        mind.cancelled = false;
        mind.force_updates = 0;
        mind.body = match state {
            LowerState::EntrySite | LowerState::EntryStandby => Some(BodyWait::EntrySite { since: at }),
            LowerState::BirthdayParty => Some(BodyWait::BirthdayPartyWait { since: at }),
            _ => None,
        };
    }
}

fn talk_data(world: &World, entity: Entity, kind: TalkType) -> AiTalkData {
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |id| id.0);
    AiTalkData {
        kind, content: None, target_fixture: None,
        target_position: world.get::<Transform>(entity).map_or([0.0; 3], |t| t.translation.to_array()),
        main_character: unit, characters: vec![unit], pre_action: None, locate: None, pending_factory: None,
    }
}

fn enter(world: &mut World, entity: Entity, state: LowerState) -> bool {
    if !enrolled(world, entity) { return false; }
    let supplied = world.get::<TalkSlot>(entity).and_then(|slot|slot.current.clone());
    let already_entered = world.get::<NpcHfsmActor>(entity).and_then(|actor| actor.lower) == Some(state);
    let forced = matches!(state, LowerState::EntrySite | LowerState::EntryStandby | LowerState::CutScene);
    if !already_entered && !(if forced { controller::force_event_lower(world, entity, state) }
        else { controller::transition_event_lower(world, entity, state) }) { return false; }
    exit(world, entity);
    let at = frame(world);
    world.entity_mut(entity).insert(EventLower::new(state, at));
    mark_objective(world, entity, state);
    if let Some(data) = supplied {
        let preserve = matches!((state, data.kind),
            (LowerState::EntrySite | LowerState::EntryStandby, TalkType::EntrySite)
            | (LowerState::TutorialWait, TalkType::TutorialWaiting)
            | (LowerState::BirthdayParty, TalkType::BirthdayParty)
            | (LowerState::CutScene, TalkType::CutScene)
            | (LowerState::ChangeSite, TalkType::ChangeSite));
        if preserve {
            if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) { slot.set_current(data); slot.interrupt = None; }
        }
    }
    true
}

/// The initial appearance owner hides an actor until the gate publishes its
/// placement. Entry itself deliberately has no completion timer.
pub fn enter_site(world: &mut World, entity: Entity) -> bool {
    if !enter(world, entity, LowerState::EntrySite) { return false; }
    if !world.get_resource::<HeldGameState>().is_some_and(|state| state.0 == GameStateType::CutScene) {
        crate::npc::gate_entries::hide(world, entity);
    }
    true
}

/// The appearance scheduler may park an already created NPC while its gate
/// performance is pending. Its own fallback shows the character at 30 s.
pub fn enter_standby(world: &mut World, entity: Entity) -> bool {
    enter(world, entity, LowerState::EntryStandby)
}

/// Birthday's AI entry forces its lower directly, including after photo mode.
pub fn begin_birthday(world: &mut World, entity: Entity) -> bool {
    if !enrolled(world, entity) || !controller::force_event_lower(world, entity, LowerState::BirthdayParty) { return false; }
    let at = frame(world);
    world.entity_mut(entity).insert(EventLower::new(LowerState::BirthdayParty, at));
    mark_objective(world, entity, LowerState::BirthdayParty);
    let mut data = talk_data(world, entity, TalkType::BirthdayParty);
    data.target_position = [0.0; 3];
    data.characters.clear();
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) { slot.set_current(data); slot.interrupt = None; }
    action(world, entity, NpcAction::Idle);
    true
}

fn turn(world: &mut World, entity: Entity, towards: Vec3, seconds: Option<f32>) {
    if towards.length_squared() < 9.9999994e-11 { return; }
    let Some(transform) = world.get::<Transform>(entity) else { return; };
    let forward = (transform.rotation * Vec3::Z).to_array();
    let duration = seconds.unwrap_or_else(|| moly_law::path::rotate_time(
        moly_law::path::angle_between(forward, (towards - transform.translation).to_array())));
    let at = frame(world);
    world.entity_mut(entity).insert(crate::npc_look_at::NpcLookAt::new(
        towards, forward, duration, moly_law::ui::dotween::Ease::OutQuart, at));
}

/// The layout-save lottery supplies its selected tweet. This lower owns the
/// turn, the tweet presentation, the five-second wait and the data cleanup.
pub fn begin_after_edit(world: &mut World, entity: Entity, tweet: i32) -> bool {
    if !enter(world, entity, LowerState::AfterEditReaction) { return false; }
    let data = talk_data(world, entity, TalkType::AfterEditLayout);
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) { slot.set_current(data); slot.interrupt = None; }
    action(world, entity, NpcAction::Idle);
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) { actions.tweet_id = tweet; }
    let player = world.query_filtered::<&Transform, With<crate::player::PlayerControlled>>()
        .iter(world).next().map(|t|t.translation);
    if let Some(player) = player { turn(world, entity, player, None); }
    true
}

/// The successful save publishes stable instance identities before reloading
/// their scenes. Selection waits for the replacement instances to be ready.
#[derive(Resource, Default)]
pub struct LayoutReactionFixtures {
    pub pending: Vec<Vec<String>>,
}

/// A selected furniture talk first gets the surprise and look-at phase;
/// only after that phase does its ordinary fixture plan start moving.
pub fn begin_high_priority(world: &mut World, entity: Entity, talk: i32, fixture: Entity) -> bool {
    let Some(target) = world.get::<GlobalTransform>(fixture).map(|t|t.translation()) else { return false; };
    if !enter(world, entity, LowerState::HighPriorityFixtureReaction) { return false; }
    action(world, entity, NpcAction::Idle);
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) { actions.pending_talk_data = Some(talk); actions.target_fixture = Some(fixture); }
    if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() {
        emotes.show(entity, "fx_emote_010".to_owned(), false);
    }
    turn(world, entity, target, Some(1.0));
    world.get_mut::<EventLower>(entity).unwrap().fixture_reaction = Some((talk, fixture));
    true
}

/// Admit a fully prepared immediate timeline. The same Director token is
/// returned to the caller's wait and owned here until lower-state exit.
pub fn begin_immediate_timeline(world: &mut World, entity: Entity, request: StartTimeline) -> Result<TimelineToken, String> {
    if !enrolled(world, entity) { return Err("actor is not controlled by the event state machine".into()); }
    timeline::validate_start(world, &request).map_err(|error| error.message)?;
    let owner = request.owner.activity;
    let fixture = request.fixture;
    let identity = world.get::<crate::fixture_activity_state::FixtureActivityIdentity>(fixture)
        .ok_or("immediate timeline fixture has no instance identity")?.clone();
    world.init_resource::<FixtureActivityReservations>();
    let target = FixtureTarget { entity: fixture, uid: identity.uid };
    if !world.resource_mut::<FixtureActivityReservations>().reserve_npc_target(&target, owner) {
        return Err("immediate timeline fixture is already reserved".into());
    }
    if !enter(world, entity, LowerState::ImmediatelyTimeline) {
        world.resource_mut::<FixtureActivityReservations>().release_owner(owner);
        return Err("current lower state cannot be interrupted".into());
    }
    let mut data = talk_data(world, entity, TalkType::ImmediatelyFixture);
    data.target_fixture = Some(fixture);
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) { slot.set_current(data); slot.interrupt = None; }
    action(world, entity, NpcAction::FixtureAction);
    world.entity_mut(entity).insert(crate::npc_fixture_activity::NpcFixtureAnimationOwner(owner));
    let token = world.resource_mut::<FixtureActivityTimelines>().request_start(request);
    if let Some(mut lower) = world.get_mut::<EventLower>(entity) {
        lower.token = Some(token); lower.owner = Some(owner);
    }
    Ok(token)
}

/// The change-site host calls this only after its door walk has arrived;
/// that navigation belongs to the upper, the doorway sequence to this lower.
pub fn begin_change_site(world: &mut World, entity: Entity) -> bool {
    if owns(world, entity) && world.get::<EventLower>(entity).is_some_and(|l| l.state == LowerState::ChangeSite) { return true; }
    enter(world, entity, LowerState::ChangeSite)
}

/// Release owned effects before another event or plan is installed. This
/// function never changes the controller's phase, allowing forced takeover.
pub fn exit(world: &mut World, entity: Entity) {
    let Some(lower) = world.get_entity_mut(entity).ok().and_then(|mut actor| actor.take::<EventLower>()) else { return; };
    if let Some(token) = lower.token { timeline::cancel_and_release(world, token); }
    if let Some(owner) = lower.owner {
        if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() { reservations.release_owner(owner); }
        world.entity_mut(entity).remove::<crate::npc_fixture_activity::NpcFixtureAnimationOwner>();
    }
    if matches!(lower.state, LowerState::AfterEditReaction | LowerState::HighPriorityFixtureReaction | LowerState::PhotoShot | LowerState::ImmediatelyTimeline) {
        world.write_message(crate::npc_state::TweetHudEvent::Retire { npc: entity });
        world.entity_mut(entity).remove::<crate::npc_look_at::NpcLookAt>();
        if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.hide(entity); }
    }
    if lower.state == LowerState::HighPriorityFixtureReaction && lower.phase != 2 {
        if let Some(mut actions) = world.get_mut::<NpcActions>(entity) { actions.pending_talk_data = None; actions.target_fixture = None; }
    }
    if lower.state == LowerState::PhotoShot {
        if let Some(ticket) = lower.photo.as_ref().and_then(|call|call.emote) {
            if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.forget(ticket); }
        }
        if let Some(mut actions) = world.get_mut::<NpcActions>(entity) { actions.reserved_move_position = None; }
        crate::npc::stop_movement(world, entity);
        world.entity_mut(entity).remove::<crate::npc_fixture_activity::NpcFixtureMotionOwner>();
        stop_photo_clip(world, entity);
        reset_face(world, entity);
        crate::npc_ik::clear(world,entity);
        if lower.birthday_pending {
            let yielded_at = frame(world);
            world.entity_mut(entity).insert(DeferredBirthday { yielded_at });
        }
    }
    let expected = match lower.state {
        LowerState::AfterEditReaction => Some(TalkType::AfterEditLayout),
        LowerState::BirthdayParty => Some(TalkType::BirthdayParty),
        LowerState::ImmediatelyTimeline => Some(TalkType::ImmediatelyFixture),
        LowerState::EntrySite | LowerState::EntryStandby => Some(TalkType::EntrySite),
        LowerState::CutScene => Some(TalkType::CutScene),
        _ => None,
    };
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        if expected.is_some() && slot.kind() == expected { slot.current = None; slot.interrupt = None; }
    }
}

pub fn reset_face(world: &mut World, entity: Entity) {
    let Some(unit) = world.get::<CharacterUnitId>(entity).map(|id|id.0) else { return; };
    let Some(materials) = world.get::<crate::character_material::ToonMaterials>(entity) else { return; };
    let (Some(eye), Some(mouth)) = (materials.slot_handle("eye").cloned(), materials.slot_handle("mouth").cloned()) else { return; };
    if !world.contains_resource::<crate::alone_action_runtime::FacialTables>() { return; }
    world.resource_scope(|world, tables: Mut<crate::alone_action_runtime::FacialTables>| {
        let Some(mut materials) = world.get_resource_mut::<Assets<crate::character_material::CharacterMaterial>>() else { return; };
        crate::player_talk::reset_default_face(unit, &tables, &mut materials, &eye, &mouth);
    });
}

fn photo_face(world: &mut World, entity: Entity, row: &PhotoRow) {
    let Some(materials) = world.get::<crate::character_material::ToonMaterials>(entity) else { return; };
    let (Some(eye), Some(mouth)) = (materials.slot_handle("eye").cloned(), materials.slot_handle("mouth").cloned()) else { return; };
    let Some(tables) = world.get_resource::<crate::alone_action_runtime::FacialTables>() else { return; };
    let eye_pattern = tables.eye_pattern(&row.expression_eye_name);
    let mouth_pattern = tables.lip_pattern(&row.expression_mouth_name).unwrap_or_default();
    if let Some(mut materials) = world.get_resource_mut::<Assets<crate::character_material::CharacterMaterial>>() {
        crate::alone_action_runtime::apply_eye_pattern(&mut materials, &eye, eye_pattern);
        crate::alone_action_runtime::apply_mouth_pattern(&mut materials, &mouth, mouth_pattern);
    }
}

fn stop_photo_clip(world: &mut World, entity: Entity) {
    cancel_animation(world, entity);
    let Some((player, idle)) = world.get::<crate::character::MotionDriver>(entity).map(|d|(d.player,d.idle)) else { return; };
    if let Some(mut driver) = world.get_mut::<crate::character::MotionDriver>(entity) { driver.alone_holds = false; driver.playing = None; }
    let mut query = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
    if let Ok((mut player, mut transitions)) = query.get_mut(world, player) {
        transitions.play(&mut player, idle, std::time::Duration::from_secs_f32(0.5)).repeat();
    }
}

fn play_animation_clip(world: &mut World, entity: Entity, name: &str, blend: f32, repeat: bool) -> Option<(AnimationNodeIndex, f32)> {
    let (player, graph) = world.get::<crate::character::MotionDriver>(entity).map(|d|(d.player,d.graph.clone()))?;
    let handle = world.get_resource::<crate::character::MotionLibrary>()?.gltf.clone();
    let library = world.get_resource::<Assets<bevy::gltf::Gltf>>()?.get(&handle)?;
    let clip = library.named_animations.get(name)?.clone();
    let duration = world.get_resource::<Assets<AnimationClip>>()?.get(&clip)?.duration();
    let node = {
        let mut graphs = world.get_resource_mut::<Assets<AnimationGraph>>()?;
        let graph = graphs.get_mut(&graph)?;
        let existing = graph.graph.node_indices().find(|node| matches!(&graph.graph[*node].node_type,
            bevy::animation::graph::AnimationNodeType::Clip(handle) if handle == &clip));
        existing.unwrap_or_else(||graph.add_clip(clip, 1.0, graph.root))
    };
    let mut query = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
    let Ok((mut animation, mut transitions)) = query.get_mut(world, player) else { return None; };
    let playing = transitions.play(&mut animation, node, std::time::Duration::from_secs_f32(blend.max(0.0)));
    playing.set_speed(1.0);
    if repeat { playing.repeat(); }
    if let Some(mut driver) = world.get_mut::<crate::character::MotionDriver>(entity) { driver.alone_holds = true; driver.playing = None; }
    Some((node, duration))
}

fn current_animation_family(world: &World, entity: Entity) -> Option<String> {
    if let Some(owner) = world.get::<EventAnimation>(entity) {
        if owner.source_view.as_ref().is_none_or(|view| view.matches(world, entity)) {
            return Some(owner.family.strip_suffix("_O").unwrap_or(&owner.family).to_owned());
        }
    }
    let driver = world.get::<crate::character::MotionDriver>(entity)?;
    let player = world.get::<AnimationPlayer>(driver.player)?;
    let graph = world.get_resource::<Assets<AnimationGraph>>()?.get(&driver.graph)?;
    let library = world.get_resource::<crate::character::MotionLibrary>()?;
    let gltf = world.get_resource::<Assets<bevy::gltf::Gltf>>()?.get(&library.gltf)?;
    let (node, _) = player.playing_animations().max_by(|(_, a), (_, b)|a.weight().total_cmp(&b.weight()))?;
    let bevy::animation::graph::AnimationNodeType::Clip(handle) = &graph.graph[*node].node_type else { return None; };
    let name = gltf.named_animations.iter().find(|(_, candidate)|*candidate == handle)?.0;
    Some(["_S", "_L", "_E", "_O"].into_iter().find_map(|suffix|name.strip_suffix(suffix)).unwrap_or(name).to_owned())
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) struct AnimationLease(u64);
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum AnimationOutcome { Pending, Complete, Cancelled }
#[derive(Resource,Default)]
struct AnimationOutcomes(std::collections::HashMap<u64, AnimationOutcome>);
#[derive(Resource,Default)]
struct NextAnimationLease(u64);
#[derive(Clone,Copy)]
pub(crate) enum AnimationSuffix { Start, Loop, End, Once }
impl AnimationSuffix {
    fn suffix(self)->&'static str {match self {Self::Start=>"_S",Self::Loop=>"_L",Self::End=>"_E",Self::Once=>"_O"}}
}
/// Exact clips belonging to one source view, joined by serialized identity.
/// The renderer name and authored length remain paired; the shared GLB union
/// cannot establish whether an individual controller contains a clip.
#[derive(Clone, Debug)]
pub(crate) struct SourceAnimationClip {
    pub exported_name: String,
    pub length: f32,
}
pub(crate) type SourceAnimationMap = std::collections::BTreeMap<String, SourceAnimationClip>;

/// Release only this completed sequence. Keeping Idle in the locomotion
/// driver prevents an immediate replay of the just-entered idle family.
pub(crate) fn release_source_idle(world: &mut World, entity: Entity, lease: AnimationLease, idle: &str) {
    let owned = world.get::<EventAnimation>(entity).is_some_and(|a| a.lease == lease && a.phase == 3);
    if !owned { return; }
    let has_loop = world.get::<EventAnimation>(entity).and_then(|a| a.source.as_ref())
        .is_some_and(|source| source.contains_key(&format!("{idle}_L")));
    let same_idle = world.get::<crate::npc::MotionClips>(entity).is_some_and(|clips| clips.idle == idle);
    cancel_animation_if_owned(world, entity, lease);
    if has_loop && same_idle {
        if let Some(mut driver) = world.get_mut::<crate::character::MotionDriver>(entity) {
            driver.playing = Some(crate::character::MotionKind::Idle);
        }
    }
}

fn play_animation_binding(
    world: &mut World,
    entity: Entity,
    name: &str,
    blend: f32,
    repeat: bool,
    source: Option<&SourceAnimationMap>,
) -> Result<Option<(AnimationNodeIndex, f32)>, String> {
    let Some(source) = source else {
        return Ok(play_animation_clip(world, entity, name, blend, repeat));
    };
    let Some(binding) = source.get(name) else {
        return Ok(None); // Known absence in this view: native skips this segment.
    };
    let (node, _) = play_animation_clip(world, entity, &binding.exported_name, blend, repeat)
        .ok_or_else(|| format!("source clip {name} is present, but its renderer is unavailable"))?;
    Ok(Some((node, binding.length)))
}

#[derive(Component)]
struct EventAnimation {
    lease:AnimationLease,
    family:String,
    blend:f32,
    phase:u8,
    suffix:Option<AnimationSuffix>,
    source:Option<std::sync::Arc<SourceAnimationMap>>,
    source_view:Option<tutorial::rotation::ViewIdentity>,
    wait:Option<crate::tween_clock::Delay>,
}

pub(crate) fn begin_animation(world:&mut World,entity:Entity,name:&str,play_end:bool,blend:f32)->Result<(),String> {
    let lease=begin_animation_owned(world,entity,name,true,play_end,blend)?;
    forget_animation_outcome(world,lease);
    Ok(())
}
pub(crate) fn begin_animation_owned(world:&mut World,entity:Entity,name:&str,has_exit_time:bool,play_end:bool,blend:f32)->Result<AnimationLease,String> {
    begin_animation_inner(world,entity,name,None,has_exit_time,play_end,blend,None,None)
}
pub(crate) fn begin_suffix_animation(world:&mut World,entity:Entity,name:&str,suffix:AnimationSuffix,has_exit_time:bool,play_end:bool,blend:f32)->Result<AnimationLease,String> {
    begin_animation_inner(world,entity,name,Some(suffix),has_exit_time,play_end,blend,None,None)
}
/// The same owned ChangeAnimation sequence, bound to the actor's source map.
pub(crate) fn begin_source_animation_owned(
    world: &mut World,
    entity: Entity,
    name: &str,
    has_exit_time: bool,
    play_end: bool,
    blend: f32,
    source: std::sync::Arc<SourceAnimationMap>,
    view: tutorial::rotation::ViewIdentity,
) -> Result<AnimationLease, String> {
    if !view.matches(world, entity) { return Err("source animation view was replaced".into()); }
    begin_animation_inner(world, entity, name, None, has_exit_time, play_end, blend, Some(source), Some(view))
}
fn begin_animation_inner(world:&mut World,entity:Entity,name:&str,suffix:Option<AnimationSuffix>,has_exit_time:bool,play_end:bool,blend:f32,source:Option<std::sync::Arc<SourceAnimationMap>>,source_view:Option<tutorial::rotation::ViewIdentity>)->Result<AnimationLease,String> {
    if world.get_entity(entity).is_err() { return Err("animation actor was removed".into()); }
    world.init_resource::<NextAnimationLease>();
    world.init_resource::<AnimationOutcomes>();
    let lease={let mut next=world.resource_mut::<NextAnimationLease>();next.0=next.0.checked_add(1).expect("animation lease exhausted");AnimationLease(next.0)};
    if name.is_empty() {world.resource_mut::<AnimationOutcomes>().0.insert(lease.0,AnimationOutcome::Complete);return Ok(lease);}
    let current=current_animation_family(world,entity);
    let family=name.strip_suffix("_O").unwrap_or(name);
    // ChangeAnimation checks the current original family before replacing its
    // cancellation token. A same-family call is already complete.
    if suffix.is_none() && current.as_deref()==Some(family) {world.resource_mut::<AnimationOutcomes>().0.insert(lease.0,AnimationOutcome::Complete);return Ok(lease);}
    let library=world.get_resource::<crate::character::MotionLibrary>().ok_or("motion library is not requested")?;
    if world.get_resource::<Assets<bevy::gltf::Gltf>>().and_then(|gltfs|gltfs.get(&library.gltf)).is_none() {return Err("motion library is loading".into());}
    // Retire a source task from a replaced view without releasing the new
    // view's animation driver. It cannot satisfy the same-family fast path.
    if world.get::<EventAnimation>(entity).and_then(|a| a.source_view.as_ref())
        .is_some_and(|view| !view.matches(world, entity)) {
        advance_animation(world, entity, 0.0)?;
    }
    cancel_animation(world,entity);
    let mut animation=EventAnimation {lease,family:name.to_owned(),blend,phase:0,suffix,source,source_view,wait:None};
    // TryPlayPrevEndAnimationAsync immediately returns when hasExitTime is true.
    if play_end && !has_exit_time {
        if let Some(current)=current {
            if let Some((_,duration))=play_animation_binding(world,entity,&format!("{current}_E"),blend,false,animation.source.as_deref())? {
                animation.wait=Some(crate::tween_clock::Delay::new(crate::tween_clock::delay_seconds(f64::from(duration)),u64::from(frame(world))));
            }
        }
    }
    world.resource_mut::<AnimationOutcomes>().0.insert(lease.0,AnimationOutcome::Pending);
    world.entity_mut(entity).insert(animation);
    if let Err(reason) = advance_animation(world,entity,0.0) {
        forget_animation_outcome(world, lease);
        return Err(reason);
    }
    Ok(lease)
}
pub(crate) fn animation_outcome(world:&World,_entity:Entity,lease:AnimationLease)->AnimationOutcome {
    world.get_resource::<AnimationOutcomes>().and_then(|outcomes|outcomes.0.get(&lease.0)).copied().unwrap_or(AnimationOutcome::Cancelled)
}
pub(crate) fn animation_pending(world:&World,entity:Entity,lease:AnimationLease)->bool {
    animation_outcome(world,entity,lease)==AnimationOutcome::Pending
}
/// Owned callers discard the outcome after observing its terminal state, or
/// when they abandon the wait. Fire-and-forget callers discard it immediately.
pub(crate) fn forget_animation_outcome(world:&mut World,lease:AnimationLease) {
    if let Some(mut outcomes)=world.get_resource_mut::<AnimationOutcomes>() {outcomes.0.remove(&lease.0);}
}
pub(crate) fn cancel_animation_if_owned(world:&mut World,entity:Entity,lease:AnimationLease) {
    if world.get::<EventAnimation>(entity).is_some_and(|animation|animation.lease==lease) {cancel_animation(world,entity);}
}
pub(crate) fn animation_changing(world:&World,entity:Entity)->bool {
    world.get::<EventAnimation>(entity).is_some_and(|animation|animation.phase!=3)
}
pub(crate) fn cancel_animation(world:&mut World,entity:Entity) {
    let Ok(mut actor)=world.get_entity_mut(entity) else {return;};
    if let Some(animation)=actor.take::<EventAnimation>() {
        if let Some(mut outcomes)=world.get_resource_mut::<AnimationOutcomes>() {
            if let Some(outcome)=outcomes.0.get_mut(&animation.lease.0) {
                if *outcome==AnimationOutcome::Pending {*outcome=AnimationOutcome::Cancelled;}
            }
        }
        if let Some(mut driver)=world.get_mut::<crate::character::MotionDriver>(entity) {driver.alone_holds=false;}
    }
}
fn advance_animation(world: &mut World, entity: Entity, dt: f32) -> Result<(), String> {
    let Some(mut animation) = world.get_entity_mut(entity).ok().and_then(|mut e| e.take::<EventAnimation>()) else {
        return Ok(());
    };
    if animation.source_view.as_ref().is_some_and(|view| !view.matches(world, entity)) {
        if let Some(mut outcomes) = world.get_resource_mut::<AnimationOutcomes>() {
            if let Some(outcome) = outcomes.0.get_mut(&animation.lease.0) {
                if *outcome == AnimationOutcome::Pending { *outcome = AnimationOutcome::Cancelled; }
            }
        }
        return Ok(()); // The replacement view and its driver are untouched.
    }
    let result = (|| -> Result<(), String> {
        if let Some(wait) = animation.wait.as_mut() {
            if !wait.tick(u64::from(frame(world)), dt) { return Ok(()); }
            animation.wait = None;
        }
        if animation.phase == 0 {
            if let Some(suffix) = animation.suffix {
                let repeat = matches!(suffix, AnimationSuffix::Loop);
                let name = format!("{}{}", animation.family, suffix.suffix());
                animation.phase = if repeat { 3 } else { 2 };
                if let Some((_, duration)) = play_animation_binding(world, entity, &name,
                    animation.blend, repeat, animation.source.as_deref())? {
                    if !repeat {
                        animation.wait = Some(crate::tween_clock::Delay::new(
                            crate::tween_clock::delay_seconds(f64::from(duration)), u64::from(frame(world))));
                    }
                }
            } else {
                let one_shot = animation.family.ends_with("_O");
                let clip = if one_shot { animation.family.clone() } else { format!("{}_S", animation.family) };
                animation.phase = if one_shot { 2 } else { 1 };
                if let Some((_, duration)) = play_animation_binding(world, entity, &clip,
                    animation.blend, false, animation.source.as_deref())? {
                    animation.wait = Some(crate::tween_clock::Delay::new(
                        crate::tween_clock::delay_seconds(f64::from(duration)), u64::from(frame(world))));
                }
            }
        }
        if animation.wait.is_none() && animation.phase == 1 {
            let clip = format!("{}_L", animation.family);
            play_animation_binding(world, entity, &clip, animation.blend, true, animation.source.as_deref())?;
            animation.phase = 3;
        } else if animation.wait.is_none() && animation.phase == 2 {
            animation.phase = 3;
        }
        Ok(())
    })();
    if result.is_err() || animation.phase == 3 {
        if let Some(mut outcomes) = world.get_resource_mut::<AnimationOutcomes>() {
            if let Some(outcome) = outcomes.0.get_mut(&animation.lease.0) {
                *outcome = if result.is_err() { AnimationOutcome::Cancelled } else { AnimationOutcome::Complete };
            }
        }
    }
    if result.is_err() {
        if let Some(mut driver) = world.get_mut::<crate::character::MotionDriver>(entity) {
            driver.alone_holds = false;
        }
    } else {
        world.entity_mut(entity).insert(animation);
    }
    result
}

fn overlap_position(world: &mut World, entity: Entity) -> Option<Vec3> {
    let site = world.get::<NpcActions>(entity)?.site_type.clone();
    let position = world.get::<Transform>(entity)?.translation;
    let others: Vec<(Vec3, Option<Vec3>)> = world.query::<(Entity, &Transform, &NpcActions)>().iter(world)
        .filter(|(other, _, actions)| *other != entity && actions.site_type == site)
        .map(|(_, t, actions)|(t.translation, actions.reserved_move_position.map(Vec3::from))).collect();
    let overlaps = |at: Vec3| others.iter().any(|(position, reserved)|position.distance(at) < 0.46
        || reserved.is_some_and(|position| position.distance(at) < 0.46));
    if !overlaps(position) { return None; }
    let cells = {
        let mut params = SystemState::<crate::npc::walkable::WalkableSource>::new(world);
        params.get_mut(world).cells_for(&site).ok()?
    };
    let face = crate::npc::navigation::face(world, entity)?;
    let center = super::cell_of(position.x, position.z);
    let mut result = None;
    let mut nearest = f32::MAX;
    // The source enumerates relative x first and z second. Canonical x is
    // reflected, so reversing its offset keeps source tie order.
    for dx in -4..=4 {
        for dz in -4..=4 {
            let cell = (center.0 - dx, center.1 + dz);
            if !cells.contains(&cell) { continue; }
            let Some(at) = face.sample(face.world_of(cell), 0.25).map(Vec3::from) else { continue; };
            let distance = position.distance(at);
            if distance <= 1.0 && distance < nearest && !overlaps(at)
                && face.field.segment_walkable([position.x,position.z],[at.x,at.z]) {
                result = Some(at); nearest = distance;
            }
        }
    }
    result
}

fn enter_photo(world: &mut World, entity: Entity) -> bool {
    if !enter(world, entity, LowerState::PhotoShot) { return false; }
    action(world, entity, NpcAction::PhotoShot);
    world.get_mut::<EventLower>(entity).unwrap().photo = Some(PhotoCall {
        sequence: 0, requested: false, move_to: None, last_camera: None, camera_target: None, row: None, emote: None, motion_error: None,
    });
    true
}

fn step_photo(world: &mut World, entity: Entity, dt: f32) -> bool {
    if !world.get_resource::<HeldGameState>().is_some_and(|state|state.0 == GameStateType::PhotoShot) {
        let pending = world.contains_resource::<crate::markers::BirthdayContext>();
        world.get_mut::<EventLower>(entity).unwrap().birthday_pending = pending;
        return true;
    }
    let Some(unit) = world.get::<CharacterUnitId>(entity).map(|id|id.0) else { return true; };
    let (mut call, mut phase) = {
        let mut event = world.get_mut::<EventLower>(entity).unwrap();
        let Some(call) = event.photo.take() else { return false; };
        (call,event.phase)
    };
    if call.requested {
        let rows = world.get_resource::<PhotoMode>().and_then(|mode|mode.rows.as_ref()).and_then(|rows|rows.as_ref().ok());
        let max = rows.into_iter().flatten().filter(|row|row.game_character_unit_id == unit).map(|row|row.seq).max().unwrap_or(0);
        call.sequence = if call.sequence + 1 <= max { call.sequence + 1 } else { 1 };
        call.row = rows.into_iter().flatten().find(|row|row.game_character_unit_id == unit && row.seq == call.sequence).cloned();
        call.requested = false;
        if let Some(target) = overlap_position(world, entity) {
            if let (Some(pose), Some(speed)) = (world.get::<Transform>(entity).copied(), world.get::<crate::npc::WalkSpeed>(entity).map(|s|s.0)) {
                call.move_to = crate::npc_fixture_activity::LocalMove::new(pose,target,pose.rotation,speed).ok();
                call.last_camera = None;
                world.entity_mut(entity).insert(crate::npc_fixture_activity::NpcFixtureMotionOwner(FixtureActivityOwner { actor: entity, generation: u64::MAX - 1 }));
                if let Some(mut actions) = world.get_mut::<NpcActions>(entity) { actions.reserved_move_position = Some(target.to_array()); }
            }
        }
        phase = 1;
    }
    if phase == 1 {
        let moving = if let Some(movement) = call.move_to.as_mut() {
            let (pose, motion, done) = movement.step(dt);
            if let Some(mut transform) = world.get_mut::<Transform>(entity) { transform.translation = pose.translation; transform.rotation = pose.rotation; }
            if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(entity) { walk.0.position = pose.translation.to_array(); walk.0.forward = (pose.rotation*Vec3::Z).to_array(); }
            if let Some(mut phase)=world.get_mut::<crate::npc::MotionPhase>(entity) { *phase=motion; }
            if done {
                call.move_to = None;
                world.entity_mut(entity).remove::<crate::npc_fixture_activity::NpcFixtureMotionOwner>();
                if let Some(mut actions)=world.get_mut::<NpcActions>(entity) { actions.reserved_move_position=None; }
            }
            !done
        } else { false };
        if !moving {
            let camera = world.query_filtered::<(Entity,&Camera, &GlobalTransform), With<Camera3d>>().iter(world)
                .find(|(_,camera, _)|camera.is_active).map(|(entity,_,transform)|(entity,transform.translation()));
            if let Some((camera_entity,camera)) = camera {
                if call.last_camera.is_none_or(|last|last.distance_squared(camera)>=1.0e-10) {
                    if crate::npc_ik::set_target(world,entity,camera_entity) {
                        call.camera_target=Some(camera_entity);phase=4;
                    }
                } else {phase=2;}
            } else {phase=2;}
            if phase==1 {phase=2;}
        }
    } else if phase==4 {
        if crate::npc_ik::settled(world,entity).is_some() {
            if let Some(camera)=call.camera_target.and_then(|camera|world.get::<GlobalTransform>(camera)).map(|t|t.translation()) {
                let y=world.get::<Transform>(entity).map_or(camera.y,|t|t.translation.y);
                turn(world,entity,Vec3::new(camera.x,y,camera.z),None);
                call.last_camera=Some(camera);
                }
            phase=2;
        }
    } else if phase == 2 && world.get::<crate::npc_look_at::NpcLookAt>(entity).is_none() {
        if let Some(row) = call.row.as_ref() {
            match begin_animation(world, entity, &row.motion_name, true, 0.5) {
                Ok(()) => {
                    photo_face(world,entity,row);
                    if let Some(emote)=&row.emoticon_name {
                        if let Some(mut emotes)=world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { call.emote = Some(emotes.show_owned(entity,emote.clone(),false)); }
                    }
                    call.motion_error=None;
                    phase=3;
                }
                Err(reason) => {
                    if call.motion_error.as_ref()!=Some(&reason) {warn!("[npc-photo] unit {unit} sequence {}: {reason}",call.sequence);}
                    call.motion_error=Some(reason);
                }
            }
        } else { phase=3; }
    } else if phase == 3 {
        let done = !animation_changing(world, entity) && call.emote.is_none_or(|ticket| crate::emoticon::show_completed(world, ticket));
        if done {
            if let Some(ticket) = call.emote {
                if let Some(mut emotes)=world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.hide_owned(entity, ticket); }
            }
            phase=5;
        }
    } else if phase == 5 && call.emote.is_none_or(|ticket|crate::emoticon::hide_completed(world, ticket)) {
        if let Some(ticket) = call.emote.take() {
            if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.forget(ticket); }
        }
        // Completed call poses are kept until another call or mode exit.
        phase=0;
    }
    if let Some(mut event)=world.get_mut::<EventLower>(entity) { event.phase=phase; event.photo=Some(call); }
    false
}

pub fn complete(world: &mut World, entity: Entity) {
    if world.get::<EventLower>(entity).is_some_and(|lower| matches!(lower.state, LowerState::EntrySite | LowerState::EntryStandby)) {
        crate::npc::gate_entries::show(world, entity);
    }
    exit(world, entity);
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.current = None; mind.body = None; mind.cancelled = false; mind.executing = false; mind.force_updates = 0;
    }
    controller::finish_lower(world, entity);
}

pub(super) fn update(world: &mut World) {
    load_photo_rows(world);
    let Some(dt) = world.get_resource::<crate::npc_clock::NpcClock>().filter(|clock| clock.ready()).map(|clock|clock.delta()) else { return; };
    let at = frame(world);
    let animations: Vec<Entity> = world.query_filtered::<Entity, With<EventAnimation>>().iter(world).collect();
    for entity in animations {
        if let Err(reason) = advance_animation(world, entity, dt) {
            warn!("[event-animation] {reason}");
        }
    }
    let birthday: Vec<Entity> = world.query::<(Entity, &DeferredBirthday)>().iter(world)
        .filter(|(_, pending)|pending.yielded_at != at).map(|(entity, _)|entity).collect();
    for entity in birthday {
        world.entity_mut(entity).remove::<DeferredBirthday>();
        crate::npc::gate_entries::force_update_birthday_wait_talk(world, entity);
    }
    if world.get_resource::<crate::server_panel::ServerPanel>().is_some_and(|panel|panel.is_tutorial()) {
        let entrants: Vec<Entity> = world.query::<(Entity,&NpcHfsmActor)>().iter(world)
            .filter(|(entity,_)| enrolled(world,*entity) && world.get::<TutorialEntered>(*entity).is_none())
            .map(|(entity,_)|entity).collect();
        for entity in entrants {
            if controller::force_event_lower(world,entity,LowerState::TutorialWait) {
                world.entity_mut(entity).insert((TutorialEntered,EventLower::new(LowerState::TutorialWait,at)));
                mark_objective(world,entity,LowerState::TutorialWait);
                if let Some(mut flags)=world.get_mut::<NpcHfsmFlags>(entity) { flags.has_started=false; }
            }
        }
    }
    tutorial::update(world);
    let actors: Vec<Entity> = world.query::<(Entity, &NpcHfsmFlags)>().iter(world)
        .filter(|(entity, flags)| (flags.has_started || owns(world,*entity)) && !flags.suppress_recovery && enrolled(world, *entity))
        .map(|(entity, _)|entity).collect();
    for entity in actors {
        let kind = world.get::<TalkSlot>(entity).and_then(TalkSlot::kind);
        if !owns(world, entity) {
            let state = match kind {
                Some(TalkType::EntrySite) => { enter_site(world, entity); continue; }
                Some(TalkType::BirthdayParty) => LowerState::BirthdayParty,
                Some(TalkType::TutorialWaiting) => LowerState::TutorialWait,
                Some(TalkType::CutScene) => LowerState::CutScene,
                _ => continue,
            };
            if enter(world, entity, state) {
                match state {
                    LowerState::BirthdayParty => action(world, entity, NpcAction::Idle),
                    LowerState::CutScene => action(world, entity, NpcAction::CutScene),
                    _ => {}
                }
            }
            continue;
        }
        let Some(lower) = world.get::<EventLower>(entity) else { continue; };
        let (state, entered, phase, token) = (lower.state, lower.entered, lower.phase, lower.token);
        if world.get::<NpcHfsmActor>(entity).and_then(|actor| actor.lower) != Some(state) {
            exit(world, entity); continue;
        }
        if entered == at || world.get::<NpcActions>(entity).is_some_and(|a|a.current == NpcAction::Talk) { continue; }
        if world.get::<ObjectiveMind>(entity).is_some_and(|mind| mind.cancelled) {
            complete(world, entity); continue;
        }
        let done = match state {
            LowerState::EntrySite => {
                // The gate places and shows first, then requests the next
                // objective. Visibility alone is not completion.
                kind != Some(TalkType::EntrySite)
            }
            LowerState::EntryStandby => {
                let mut lower = world.get_mut::<EventLower>(entity).unwrap(); lower.elapsed += dt;
                if lower.elapsed >= 30.0 { crate::npc::gate_entries::show(world, entity); true } else { false }
            }
            LowerState::BirthdayParty => kind != Some(TalkType::BirthdayParty),
            LowerState::TutorialWait => phase == 1,
            LowerState::CutScene => !world.get_resource::<HeldGameState>().is_some_and(|state|state.0 == GameStateType::CutScene)
                && world.get::<crate::fixture_activity_state::CutSceneCastLease>(entity).is_none(),
            LowerState::AfterEditReaction => match phase {
                0 if world.get::<crate::npc_look_at::NpcLookAt>(entity).is_none() => {
                    action(world, entity, NpcAction::Tweet);
                    let mut lower = world.get_mut::<EventLower>(entity).unwrap(); lower.phase = 1; lower.elapsed = 0.0;
                    false
                }
                1 => {
                    let mut lower = world.get_mut::<EventLower>(entity).unwrap(); lower.elapsed += dt;
                    if lower.elapsed >= 5.0 {
                        lower.phase = 2;
                        world.write_message(crate::npc_state::TweetHudEvent::Retire { npc: entity });
                    }
                    false
                }
                2 => true,
                _ => false,
            },
            LowerState::HighPriorityFixtureReaction => match phase {
                0 if world.get::<crate::npc_look_at::NpcLookAt>(entity).is_none() => {
                    let mut lower = world.get_mut::<EventLower>(entity).unwrap(); lower.phase = 1; lower.elapsed = 0.0;
                    false
                }
                1 => {
                    let pending = {
                        let mut lower = world.get_mut::<EventLower>(entity).unwrap(); lower.elapsed += dt;
                        if lower.elapsed >= 1.0 { lower.phase = 2; lower.fixture_reaction } else { None }
                    };
                    if let Some((talk, fixture)) = pending {
                        if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() { emotes.hide(entity); }
                        if !controller::request_fixture_plan(world, entity, talk, fixture) { complete(world, entity); }
                    }
                    false
                }
                _ => false,
            },
            LowerState::ImmediatelyTimeline => {
                if phase == 1 { true } else {
                    let finished = token.is_some_and(|token| world.get_resource::<FixtureActivityTimelines>().and_then(|timelines|timelines.status(token))
                        .is_none_or(|status|matches!(status, TimelineStatus::Completed | TimelineStatus::Cancelled | TimelineStatus::Failed(_))));
                    if finished { world.get_mut::<EventLower>(entity).unwrap().phase = 1; }
                    false
                }
            }
            LowerState::ChangeSite => false,
            LowerState::PhotoShot => step_photo(world,entity,dt),
            _ => false,
        };
        if done { complete(world, entity); }
    }
}
