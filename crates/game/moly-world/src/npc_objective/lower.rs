//! Lower objects own presentation lifetimes; geometry and authored timeline
//! players are shared with the rest of the product.
use super::controller::{NpcHfsmActor, NpcHfsmPhase, NpcLowerClock, NpcUpperBody};
use super::{AiTalkData, ObjectiveMind, TalkSlot};
use crate::npc::{NpcAction, NpcActions, RestLifecycle};
use bevy::prelude::*;
use moly_law::{
    npc_hfsm::{LowerState, NpcPhase},
    objective::TalkType,
};

#[derive(Component, Clone, Copy)]
struct Running {
    state: LowerState,
    phase: Phase,
    elapsed: f32,
}
#[derive(Component)]
pub(super) struct ArrivalNpc(pub Entity);
#[derive(Component)]
pub(super) struct PreselectedGeneral;
#[derive(Clone, Copy)]
enum Phase {
    Looking,
    NoTalk,
    Tweet,
    TalkEnd,
    PostTweet,
    Greeting,
    Presentation,
    Done,
}

pub(super) fn for_talk(data: Option<&AiTalkData>, unit: u32) -> LowerState {
    let Some(data) = data else {
        return LowerState::GeneralTalk;
    };
    match data.kind {
        TalkType::SingleCharacterFixture => LowerState::SingleFixtureAction,
        TalkType::MultipleCharacterFixture if data.main_character != unit => {
            LowerState::SubFixtureAction
        }
        TalkType::MultipleCharacterFixture => LowerState::MultiFixtureAction,
        TalkType::CommunicationWhileDoingWait if data.main_character != unit => {
            LowerState::SubCommunicationWhileDoingWait
        }
        TalkType::CommunicationWhileDoingWait => LowerState::CommunicationWhileDoingWait,
        TalkType::CommonFixture => LowerState::CommonFixtureTweet,
        TalkType::NoneTalk => LowerState::NoneTalkFixtureAction,
        TalkType::Greeting => LowerState::Greeting,
        TalkType::TutorialWaiting => LowerState::TutorialWait,
        TalkType::CutScene => LowerState::CutScene,
        TalkType::EntrySite => LowerState::EntrySite,
        TalkType::PhotoShot => LowerState::PhotoShot,
        TalkType::AfterEditLayout => LowerState::AfterEditReaction,
        TalkType::HighPriorityTalk => LowerState::HighPriorityFixtureReaction,
        TalkType::ImmediatelyFixture => LowerState::ImmediatelyTimeline,
        TalkType::BirthdayParty => LowerState::BirthdayParty,
        TalkType::CommonSomeCharacter => LowerState::CommonSomeCharacterCommunication,
        _ => LowerState::GeneralTalk,
    }
}

fn change(world: &mut World, entity: Entity, action: NpcAction) {
    let mut q = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = q.get_mut(world, entity) {
        actions.change(action, &mut rest);
    }
}

fn presentation_owned(world: &World, entity: Entity) -> bool {
    world
        .get_resource::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .is_some_and(|r| crate::npc_fixture_activity::owns_actor(r, entity))
        || world
            .get_resource::<crate::npc_fixture_talk::FixtureTalkGroups>()
            .is_some_and(|r| r.owns(entity))
}

fn show_tweet(world: &mut World, entity: Entity) -> Phase {
    if world
        .get::<NpcActions>(entity)
        .is_none_or(|a| a.tweet_id == 0)
    {
        return Phase::Done;
    }
    change(world, entity, NpcAction::Tweet);
    Phase::Tweet
}

fn pick_general(world: &mut World, entity: Entity) -> bool {
    let Some(unit) = world
        .get::<crate::npc::CharacterUnitId>(entity)
        .map(|u| u.0)
    else {
        return false;
    };
    let Some(site) = world.get::<NpcActions>(entity).map(|a| a.site_type.clone()) else {
        return false;
    };
    let previous = world
        .get::<TalkSlot>(entity)
        .and_then(TalkSlot::previous_id);
    let Some(mut rng) = world.get::<super::MemberRng>(entity).cloned() else {
        return false;
    };
    let mut state = bevy::ecs::system::SystemState::<crate::player_talk::TalkCatalog>::new(world);
    let Ok(catalog) = state.get(world) else { return false; };
    let picked = catalog.lottery(unit, &site, previous, |len| {
        super::engine_int_draw(&mut rng, len)
    });
    let resolved = picked
        .as_ref()
        .ok()
        .and_then(|p| catalog.resolve(p.master_id));
    world.entity_mut(entity).insert(rng);
    let Some(resolved) = resolved else {
        if let Err(reason) = picked {
            warn!("[npc-hfsm] general lower lottery: {reason}");
        }
        return false;
    };
    let target = match super::general_data_destination(world, entity) {
        Ok(target) => target,
        Err(reason) => {
            warn!("[npc-hfsm] general data target: {reason}");
            return false;
        }
    };
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) {
        actions.tweet_id = resolved.pre_action().id;
    }
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        slot.set_current(AiTalkData {
            kind: TalkType::Common,
            content: Some(resolved.content()),
            target_fixture: None,
            target_position: target,
            main_character: unit,
            characters: vec![unit],
            pre_action: Some(resolved.pre_action()),
            locate: None,
            pending_factory: None,
        });
    }
    true
}

fn enter(world: &mut World, entity: Entity, state: LowerState) -> Running {
    let preselected = world
        .entity_mut(entity)
        .take::<PreselectedGeneral>()
        .is_some();
    if state == LowerState::GeneralTalk && !preselected {
        pick_general(world, entity);
    }
    let phase = match state {
        LowerState::GeneralTalk
        | LowerState::CommonFixtureTweet
        | LowerState::SingleFixtureAction
            if !presentation_owned(world, entity) =>
        {
            crate::npc::stop_movement(world, entity);
            let data = world
                .get::<TalkSlot>(entity)
                .and_then(|s| s.current.as_ref());
            if data.and_then(|d| d.content.as_ref()).is_none() {
                Phase::NoTalk
            } else {
                let target = if state == LowerState::GeneralTalk {
                    world
                        .get::<ArrivalNpc>(entity)
                        .and_then(|a| world.get::<Transform>(a.0))
                        .map(|t| t.translation)
                } else {
                    data.and_then(|d| d.target_fixture)
                        .and_then(|fixture| world.get::<GlobalTransform>(fixture))
                        .map(|t| t.translation())
                };
                if let Some(target) = target {
                    let Some(t) = world.get::<Transform>(entity) else {
                        return Running {
                            state,
                            phase: Phase::Done,
                            elapsed: 0.0,
                        };
                    };
                    let forward = (t.rotation * Vec3::Z).to_array();
                    let duration = moly_law::path::rotate_time(moly_law::path::angle_between(
                        forward,
                        (target - t.translation).to_array(),
                    ));
                    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
                    world
                        .entity_mut(entity)
                        .insert(crate::npc_look_at::NpcLookAt::new(
                            target,
                            forward,
                            duration,
                            moly_law::ui::dotween::Ease::OutQuart,
                            frame,
                        ));
                    Phase::Looking
                } else {
                    show_tweet(world, entity)
                }
            }
        }
        LowerState::Greeting => {
            crate::npc::stop_movement(world, entity);
            change(world, entity, NpcAction::Greeting);
            Phase::Greeting
        }
        _ => Phase::Presentation,
    };
    Running {
        state,
        phase,
        elapsed: 0.0,
    }
}

pub(super) fn exit(world: &mut World, entity: Entity) {
    let old = world.get::<Running>(entity).copied();
    if let Ok(mut actor) = world.get_entity_mut(entity) {
        actor.remove::<Running>();
        actor.remove::<ArrivalNpc>();
        actor.remove::<PreselectedGeneral>();
    }
    if old.is_some() {
        if let Some(mut actions) = world.get_mut::<NpcActions>(entity) {
            actions.target_fixture = None;
            actions.pending_talk_data = None;
        }
        if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
            slot.reset_ai_talk_data();
        }
        if world
            .get::<NpcActions>(entity)
            .is_some_and(|a| a.current != NpcAction::Talk)
        {
            change(world, entity, NpcAction::Idle);
        }
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.executing = false;
        mind.body = None;
        mind.rest = None;
        mind.yield_since = None;
        mind.force_updates = 0;
        mind.current = None;
    }
}

/// Returns true when this lower or its timeline player owns this frame.
pub(super) fn update(world: &mut World, entity: Entity, dt: f32) -> bool {
    let owned = presentation_owned(world, entity);
    let Some(actor) = world.get::<NpcHfsmActor>(entity).copied() else {
        return false;
    };
    if owned && actor.lower.is_none() && actor.upper.is_none() {
        if world
            .get::<crate::npc::RouteStops>(entity)
            .is_some_and(|r| r.outcome.is_none())
            && world
                .get::<crate::npc::MotionPhase>(entity)
                .is_some_and(|p| {
                    !matches!(p, crate::npc::MotionPhase::Dwelling { remaining: None })
                })
        {
            return true;
        }
        let unit = world
            .get::<crate::npc::CharacterUnitId>(entity)
            .map_or(0, |u| u.0);
        let state = actor.lower_override.unwrap_or_else(|| {
            for_talk(
                world
                    .get::<TalkSlot>(entity)
                    .and_then(|s| s.current.as_ref()),
                unit,
            )
        });
        world.entity_mut(entity).insert((
            NpcHfsmPhase(NpcPhase::Lower),
            NpcHfsmActor {
                upper: None,
                lower: Some(state),
                lower_override: None,
            },
            NpcUpperBody(None),
            NpcLowerClock(None),
            Running {
                state,
                phase: Phase::Presentation,
                elapsed: 0.0,
            },
        ));
        return true;
    }
    let Some(state) = actor.lower else {
        return false;
    };
    if matches!(state, LowerState::Idle { .. } | LowerState::Rest { .. }) {
        return false;
    }
    if world.get::<crate::talk::TalkHold>(entity).is_some()
        || world
            .get::<NpcActions>(entity)
            .is_some_and(|a| a.current == NpcAction::Talk)
    {
        return true;
    }
    let mut running = match world.get::<Running>(entity).copied() {
        Some(r) if r.state == state => r,
        _ => {
            let r = enter(world, entity, state);
            world.entity_mut(entity).insert(r);
            return true;
        }
    };
    running.elapsed += dt;
    match running.phase {
        Phase::Looking => {
            if world.get::<crate::npc_look_at::NpcLookAt>(entity).is_none() {
                running.phase = show_tweet(world, entity);
                running.elapsed = 0.0;
            }
        }
        Phase::NoTalk => {
            if running.elapsed >= 2.0 {
                running.phase = Phase::Done;
            }
        }
        Phase::Tweet => {
            if running.elapsed >= 5.0 {
                change(world, entity, NpcAction::Idle);
                running.phase = if state == LowerState::CommonFixtureTweet {
                    Phase::TalkEnd
                } else {
                    Phase::Done
                };
            }
        }
        Phase::TalkEnd => {
            running.phase = Phase::PostTweet;
            running.elapsed = 0.0;
        }
        Phase::PostTweet => {
            if running.elapsed >= 5.0 {
                running.phase = Phase::Done;
            }
        }
        Phase::Greeting => {
            if world
                .get::<NpcActions>(entity)
                .is_some_and(|a| a.first_talk_complete)
            {
                running.phase = Phase::Done;
            }
        }
        Phase::Presentation => {
            if !owned {
                running.phase = Phase::Done;
            }
        }
        Phase::Done => {
            super::controller::finish_lower(world, entity);
            return true;
        }
    }
    world.entity_mut(entity).insert(running);
    true
}
