//! A selected fixture conversation owns its source pre-action until it closes.
//!
//! One source Director binds the complete admitted cast plus the placed
//! furniture. No timeline row is redrawn on retry; no actor gets a copied clock.
//! Automatic NPC conversations already have an activity owner and do not enter
//! this path. Body/voice admission waits for the source talk-enabled loop gate.

use super::{ActiveTalk, TalkHold};
use crate::player_talk::TalkCancelRequest;
use crate::{
    character::MotionDriver,
    fixture_activity_data::{FixtureActivityTables, FixtureTalkActionInput, FixtureTalkActionPlan},
    fixture_activity_provider::{FixtureActivityProvider, SourceFixtureViewLocalY},
    fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner},
    fixture_activity_timeline::{
        self as timeline, FixtureActivityTimelines, StartTimeline, TimelineOwner,
        TimelineOwnerKind, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::AttachPoints,
    npc::{CharacterUnitId, WalkState},
    npc_fixture_activity::NpcFixtureAnimationOwner,
    npc_objective::MemberRng,
};
use bevy::{animation::graph::AnimationGraph, prelude::*};
use moly_law::talk::UniformDraw;

#[derive(Component, Clone, Copy)]
pub(crate) struct TalkFixtureActorLease {
    owner: Entity,
}

#[derive(Clone)]
struct PriorActor {
    entity: Entity,
    transform: Transform,
    walk: moly_law::path::WalkState,
}

struct Session {
    owner: Entity,
    talk: i32,
    main: Entity,
    anchor: Entity,
    cast: Vec<(u32, Entity)>,
    plan: Option<FixtureTalkActionPlan>,
    plan_resolved: bool,
    draft: Option<StartTimeline>,
    token: Option<TimelineToken>,
    prior: Vec<PriorActor>,
    elapsed: f32,
    pending: String,
    failed: Option<String>,
    ready: bool,
}

#[derive(Resource, Default)]
pub(crate) struct FixtureTalkAction {
    session: Option<Session>,
}

impl FixtureTalkAction {
    pub(crate) fn diagnostics(
        &self,
        timelines: Option<&FixtureActivityTimelines>,
    ) -> serde_json::Value {
        self.session.as_ref().map_or(serde_json::Value::Null, |session| {
            let status = session.token.and_then(|token| timelines.and_then(|all| all.status(token)));
            serde_json::json!({
                "talk":session.talk,"ready":session.ready,"pending":session.pending,"error":session.failed,
                "owner":format!("{:?}",session.owner),"elapsed":session.elapsed,
                "source":session.plan.as_ref().map(|plan|serde_json::json!({"package":plan.package,
                    "prefab":plan.prefab,"timelineRow":plan.timeline.id,"fixtureUid":plan.fixture_uid,
                    "cast":plan.cast.iter().map(|member|serde_json::json!({"unit":member.unit,
                        "point":member.locate.action_point_value,"slot":member.locate.unit_slot,
                        "start":member.locate.start.position})).collect::<Vec<_>>() })),
                "status":status.map(|value|format!("{value:?}")),
                "time":session.token.and_then(|token|timelines.and_then(|all|all.sampled_time(token))),
                "coverage":session.token.and_then(|token|timelines.and_then(|all|all.coverage(token))).map(|value|format!("{value:?}")),
                "actorLeases":session.prior.len(),
            })
        })
    }
}

struct CastDraw(MemberRng);
impl UniformDraw for CastDraw {
    fn draw(&mut self, count: usize) -> usize {
        ((self.0.next() >> 33) as usize) % count
    }
}

fn teardown(world: &mut World, session: &mut Session) {
    if let Some(token) = session.token.take() {
        timeline::cancel_and_release(world, token);
    }
    for prior in session.prior.drain(..) {
        if world
            .get::<TalkFixtureActorLease>(prior.entity)
            .map(|lease| lease.owner)
            != Some(session.owner)
        {
            continue;
        }
        if let Some(mut transform) = world.get_mut::<Transform>(prior.entity) {
            *transform = prior.transform;
        }
        if let Some(mut walk) = world.get_mut::<WalkState>(prior.entity) {
            walk.0 = prior.walk;
        }
        if let Some(mut driver) = world.get_mut::<MotionDriver>(prior.entity) {
            driver.playing = None;
        }
        world
            .entity_mut(prior.entity)
            .remove::<TalkFixtureActorLease>();
    }
}

pub(crate) fn cancel_owner(world: &mut World, owner: Option<Entity>) {
    let Some(owner) = owner else {
        return;
    };
    let Some(mut state) = world.remove_resource::<FixtureTalkAction>() else {
        return;
    };
    if state
        .session
        .as_ref()
        .is_some_and(|session| session.owner == owner)
    {
        if let Some(mut session) = state.session.take() {
            teardown(world, &mut session);
        }
    }
    world.insert_resource(state);
}

fn body_ready(world: &mut World, session: &mut Session) {
    if session.ready
        || !world
            .get_resource::<ActiveTalk>()
            .is_some_and(|talk| talk.effect_owner == session.owner)
    {
        return;
    }
    world.resource_scope(|world, mut talk: Mut<ActiveTalk>| {
        talk.preparing = false;
        // The source window enters once, at body admission, never while the
        // pre-action is loading and never again on subsequent loop frames.
        if let Some(mut window) = world.get_resource_mut::<crate::talk_window::TalkWindowState>() {
            window.open(crate::talk_window::TalkSession::Pair(&mut talk));
        }
    });
    session.ready = true;
    session.pending.clear();
}

fn fail(world: &mut World, session: &mut Session, reason: String) {
    warn!(
        "[fixture-talk-action] talk={} failed: {}",
        session.talk, reason
    );
    session.failed = Some(reason);
    teardown(world, session);
    world.write_message(TalkCancelRequest);
}

fn prepare(world: &mut World, session: &mut Session) -> Result<(), (bool, String)> {
    if !session.plan_resolved {
        let tables = world
            .get_resource::<FixtureActivityTables>()
            .ok_or((true, "source tables loading".into()))?;
        let attach = world
            .get_resource::<AttachPoints>()
            .ok_or((true, "source attachment points loading".into()))?;
        let identity = world
            .get::<FixtureActivityIdentity>(session.anchor)
            .ok_or((true, "placed fixture identity loading".into()))?;
        let world_pose = world
            .get::<GlobalTransform>(session.anchor)
            .ok_or((true, "placed fixture transform loading".into()))?;
        let local_y = world
            .get::<SourceFixtureViewLocalY>(session.anchor)
            .ok_or((true, "source fixture view loading".into()))?;
        let seed = session.cast.first().map(|(unit, _)| *unit).unwrap_or(1);
        let mut rng = CastDraw(
            world
                .get::<MemberRng>(session.main)
                .cloned()
                .unwrap_or_else(|| MemberRng::seeded(seed)),
        );
        session.plan = tables
            .prepare_fixture_talk_action(
                FixtureTalkActionInput {
                    talk_id: session.talk,
                    participants: &session.cast,
                    fixture: session.anchor,
                    fixture_id: identity.master_id,
                    fixture_uid: &identity.uid,
                    fixture_world: *world_pose,
                    fixture_view_local_y: Some(local_y.0),
                },
                attach,
                &mut rng,
            )
            .map_err(|error| (false, format!("source pre-action: {error:?}")))?;
        // Store the draw once, including source groups containing only one row.
        world.entity_mut(session.main).insert(rng.0);
        session.plan_resolved = true;
    }
    let Some(plan) = session.plan.as_ref() else {
        body_ready(world, session);
        return Ok(());
    };
    if plan.fixture != session.anchor
        || plan.talk_id != session.talk
        || world
            .get::<FixtureActivityIdentity>(session.anchor)
            .is_none_or(|id| id.uid != plan.fixture_uid)
    {
        return Err((
            false,
            "source instance changed during talk preparation".into(),
        ));
    }
    world.init_resource::<FixtureActivityProvider>();
    world.resource_scope(
        |world, mut provider: Mut<FixtureActivityProvider>| -> Result<(), (bool, String)> {
            if session.draft.is_none() {
                let definition = provider
                    .definition(world, &plan.package, &plan.prefab)
                    .map_err(|error| (error.retryable, error.reason))?;
                session.draft = Some(StartTimeline {
                    owner: TimelineOwner {
                        activity: FixtureActivityOwner {
                            actor: session.main,
                            generation: session.owner.to_bits(),
                        },
                        kind: TimelineOwnerKind::Talk,
                    },
                    fixture: session.anchor,
                    definition,
                    bindings: Default::default(),
                    companions: Vec::new(),
                    timeout_secs: 30.,
                    timeout_budget: TimelineTimeoutBudget::OwnerGated {
                        advance: Some(false),
                    },
                });
            }
            let mut cast = Vec::<(u32, Entity, Entity, Handle<AnimationGraph>)>::new();
            for member in &plan.cast {
                if world.get::<CharacterUnitId>(member.actor).map(|id| id.0) != Some(member.unit) {
                    return Err((false, "admitted actor identity changed".into()));
                }
                if world
                    .get::<NpcFixtureAnimationOwner>(member.actor)
                    .is_some()
                {
                    return Err((
                        false,
                        "actor has another source fixture activity owner".into(),
                    ));
                }
                let driver = world
                    .get::<MotionDriver>(member.actor)
                    .ok_or((true, format!("unit {} animator loading", member.unit)))?;
                cast.push((
                    member.unit,
                    member.actor,
                    driver.player,
                    driver.graph.clone(),
                ));
            }
            provider
                .prepare_talk_cast_bindings(world, session.draft.as_mut().unwrap(), &cast)
                .map_err(|error| (error.retryable, error.reason))?;
            timeline::validate_start(world, session.draft.as_ref().unwrap())
                .map_err(|error| (error.retryable, error.to_string()))?;
            Ok(())
        },
    )?;
    // Mutate transforms only once all source bindings pass preflight. Keep full
    // authored rotation and Y, rather than the stage's approximate idle anchors.
    for member in &plan.cast {
        let transform = *world
            .get::<Transform>(member.actor)
            .ok_or((false, "actor transform lost".into()))?;
        let walk = world
            .get::<WalkState>(member.actor)
            .ok_or((false, "actor navigation lost".into()))?
            .0
            .clone();
        session.prior.push(PriorActor {
            entity: member.actor,
            transform,
            walk,
        });
        let locate = &member.locate.start;
        {
            let mut walk = world.get_mut::<WalkState>(member.actor).unwrap();
            walk.0.position = locate.position;
            walk.0.forward = (locate.rotation * Vec3::Z).to_array();
        }
        {
            let mut pose = world.get_mut::<Transform>(member.actor).unwrap();
            pose.translation = Vec3::from_array(locate.position);
            pose.rotation = locate.rotation;
        }
        world.entity_mut(member.actor).insert((
            TalkFixtureActorLease {
                owner: session.owner,
            },
            TalkHold,
        ));
        if let Some(mut driver) = world.get_mut::<MotionDriver>(member.actor) {
            driver.playing = None;
        }
    }
    world.init_resource::<FixtureActivityTimelines>();
    let token = world
        .resource_mut::<FixtureActivityTimelines>()
        .request_start(session.draft.take().unwrap());
    session.token = Some(token);
    session.pending = "source pre-action entering".into();
    info!(
        "[fixture-talk-action] talk={} prefab={} actors={} one_director=true",
        session.talk,
        plan.prefab,
        plan.cast.len()
    );
    Ok(())
}

pub(crate) fn drive(world: &mut World) {
    let wanted = world
        .get_resource::<ActiveTalk>()
        .filter(|talk| talk.preparing)
        .and_then(|talk| {
            talk.fixture_entity(talk.fixture_id).and_then(|anchor| {
                talk.participants.first().map(|(_, main)| {
                    (
                        talk.effect_owner,
                        talk.talk_id,
                        *main,
                        anchor,
                        talk.participants.clone(),
                    )
                })
            })
        });
    let active_owner = world
        .get_resource::<ActiveTalk>()
        .map(|talk| talk.effect_owner);
    let mut state = world
        .remove_resource::<FixtureTalkAction>()
        .unwrap_or_default();
    if state
        .session
        .as_ref()
        .is_some_and(|session| Some(session.owner) != active_owner)
    {
        if let Some(mut old) = state.session.take() {
            teardown(world, &mut old);
        }
    }
    if state.session.is_none() {
        if let Some((owner, talk, main, anchor, cast)) = wanted {
            state.session = Some(Session {
                owner,
                talk,
                main,
                anchor,
                cast,
                plan: None,
                plan_resolved: false,
                draft: None,
                token: None,
                prior: Vec::new(),
                elapsed: 0.,
                pending: "source pre-action preparation".into(),
                failed: None,
                ready: false,
            });
        }
    }
    if let Some(session) = state.session.as_mut() {
        session.elapsed += world.get_resource::<Time>().map_or(0., Time::delta_secs);
        if session.failed.is_none() {
            if let Some(token) = session.token {
                let status = world
                    .get_resource::<FixtureActivityTimelines>()
                    .and_then(|all| all.status(token))
                    .cloned();
                match status {
                    Some(TimelineStatus::Playing {
                        loop_started,
                        enable_talk,
                        ..
                    }) if loop_started && enable_talk => body_ready(world, session),
                    Some(TimelineStatus::Playing { .. }) => {
                        let no_loop = session.plan.as_ref().is_some_and(|_| {
                            // A source timeline without a loop may admit at its first
                            // successfully sampled frame, not before asset preflight.
                            world
                                .get_resource::<FixtureActivityTimelines>()
                                .is_some_and(|all| all.has_no_loop(token))
                        });
                        if no_loop {
                            body_ready(world, session);
                        }
                    }
                    Some(TimelineStatus::Completed) => body_ready(world, session),
                    Some(TimelineStatus::Failed(error)) => fail(world, session, error.to_string()),
                    Some(TimelineStatus::Cancelled) | None => {
                        fail(world, session, "source pre-action cancelled".into())
                    }
                    _ => {}
                }
            } else if !session.plan_resolved || session.plan.is_some() && !session.ready {
                if let Err((retryable, reason)) = prepare(world, session) {
                    if retryable {
                        session.pending = reason;
                    } else {
                        fail(world, session, reason);
                    }
                }
            }
            if !session.ready && session.failed.is_none() && session.elapsed > 18. {
                fail(
                    world,
                    session,
                    format!(
                        "source pre-action preparation timed out: {}",
                        session.pending
                    ),
                );
            }
        }
    }
    world.insert_resource(state);
}
