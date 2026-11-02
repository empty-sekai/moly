//! A selected fixture conversation owns its source pre-action until it closes.
//!
//! One source Director binds the complete admitted cast plus the placed
//! furniture. No timeline row is redrawn on retry; no actor gets a copied clock.
//! Automatic NPC conversations already have an activity owner and do not enter
//! this path. Body/voice admission waits for the source talk-enabled loop gate.
//!
//! A conversation with one character follows the single-character fixture
//! order instead (`SingleFixtureActionLowerState`): the character notices the
//! fixture and talks first, and only after the talk ends walks to its action
//! point and plays the pre-action to its end. That run outlives the talk; it
//! ends at its own completion or at the next talk cancel request.

use super::{ActiveTalk, TalkHold};
use crate::player_talk::TalkCancelRequest;
use crate::{
    character::{BodyOwner, BodyToken, MotionDriver},
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
use bevy::{
    animation::graph::AnimationGraph,
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use moly_law::talk::UniformDraw;

#[derive(Component, Clone, Copy)]
pub struct TalkFixtureActorLease {
    owner: Entity,
    /// False once the pre-action owns the member's animation.
    pub approaching: bool,
}

#[derive(Clone)]
struct PriorActor {
    entity: Entity,
    transform: Transform,
    walk: moly_law::path::WalkState,
    /// The member's hold on its body for the pre-action; dropped once a
    /// talk motion takes the body.
    body: Option<BodyToken>,
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
    timeout: f32,
    pending: String,
    failed: Option<String>,
    ready: bool,
    /// The talk ended and the pre-action runs on its own.
    talked: bool,
    /// The pre-action completed and its cast stands at EndLoc.
    settled: bool,
    /// A talk cancel request arrived: the run ends with its talk.
    cancelled: bool,
}

#[derive(Resource, Default)]
pub struct FixtureTalkAction {
    session: Option<Session>,
    /// A single-character pre-action still running after its talk.
    after_talk: Option<Session>,
}

impl FixtureTalkAction {
    /// Whether a pre-action is still running after its talk ended.
    pub fn after_talk(&self) -> bool {
        self.after_talk.is_some()
    }

    pub fn diagnostics(
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

/// One character: the source talks before the pre-action (the notice tweet
/// and its talk come before the approach to the action point).
fn talks_first(session: &Session) -> bool {
    session.plan.as_ref().is_some_and(|plan| plan.cast.len() == 1)
}

/// The talk owning `session` ended. A talk-first session that was admitted
/// goes on to its pre-action; any other is torn down.
fn end_talk(world: &mut World, state: &mut FixtureTalkAction, mut session: Session) {
    if talks_first(&session)
        && session.ready
        && !session.talked
        && !session.cancelled
        && session.failed.is_none()
    {
        if let Some(mut old) = state.after_talk.take() {
            teardown(world, &mut old);
        }
        session.talked = true;
        session.elapsed = 0.;
        session.pending = "source pre-action after the talk".into();
        state.after_talk = Some(session);
    } else {
        teardown(world, &mut session);
    }
}

fn teardown(world: &mut World, session: &mut Session) {
    // Preparation may have acquired particle/clip leases before a later
    // source or navigation failure. Drop this request immediately; diagnostics
    // and a queued cancellation message must not keep those leases alive.
    // Shared effect instances remain owned by any other live request.
    session.draft.take();
    // The talk can end on the frame its Director completes, before `drive`
    // has seen the completion. A completed Director still ends the source
    // way: the actors stand at EndLoc, not where they were before the talk.
    let completed = session.settled || session.token.is_some_and(|token| {
        world
            .get_resource::<FixtureActivityTimelines>()
            .and_then(|all| all.status(token))
            .is_some_and(|status| matches!(status, TimelineStatus::Completed))
    });
    if completed && !session.settled {
        if let Err(reason) = complete_pre_action(world, session) {
            warn!(
                "[fixture-talk-action] talk={} completion at teardown failed: {}",
                session.talk, reason
            );
        }
    }
    if let Some(token) = session.token.take() {
        timeline::cancel_and_release(world, token);
    }
    for mut prior in session.prior.drain(..) {
        if world
            .get::<TalkFixtureActorLease>(prior.entity)
            .map(|lease| lease.owner)
            != Some(session.owner)
        {
            // The member left this talk: its hold leaves with it.
            if let Some(mut driver) = world.get_mut::<MotionDriver>(prior.entity) {
                driver.hand_back_slot(&mut prior.body);
            }
            continue;
        }
        if !completed {
            if let Some(mut transform) = world.get_mut::<Transform>(prior.entity) {
                *transform = prior.transform;
            }
            if let Some(mut walk) = world.get_mut::<WalkState>(prior.entity) {
                walk.0 = prior.walk;
            }
        }
        if let Some(mut driver) = world.get_mut::<MotionDriver>(prior.entity) {
            driver.hand_back_slot(&mut prior.body);
            driver.playing = None;
        }
        crate::npc::cancel_external_approach(world, prior.entity);
        world
            .entity_mut(prior.entity)
            .remove::<TalkFixtureActorLease>();
        // A run after its talk ends its own hold; a live talk's hold ends
        // with the talk.
        if session.talked {
            world.entity_mut(prior.entity).remove::<TalkHold>();
        }
    }
}

pub fn cancel_owner(world: &mut World, owner: Option<Entity>) {
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
        if let Some(session) = state.session.take() {
            end_talk(world, &mut state, session);
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

fn complete_pre_action(world: &mut World, session: &mut Session) -> Result<(), String> {
    // Native MoveAndPlayTimelineAsync disposes the Director, restores the
    // parent and applies EndLoc before entering FixtureActionIdle. Holding its
    // final Root pose through the dialogue adds a second yaw to LookAt for
    // Root-host actors (e.g. unit 30); shared talk clips only animate Hips.
    // This is deliberately separate from the talk-enabled live-loop gate.
    if let Some(token) = session.token.take() {
        timeline::cancel_and_release(world, token);
    }
    let plan = session.plan.as_ref().ok_or("completed pre-action has no source plan")?;
    // Each member's hold ends before its pose settles. An idle is played only
    // where the hold is still the newest one; a talk motion that took a
    // member's body also holds it, so that clip keeps playing until the
    // holder lets go.
    for (member, prior) in plan.cast.iter().zip(session.prior.iter_mut()) {
        let Some(lease) = world.get::<TalkFixtureActorLease>(member.actor) else {
            return Err("completed pre-action actor lease changed".into());
        };
        if lease.owner != session.owner {
            return Err("completed pre-action actor lease changed".into());
        }
        let held = prior.body.filter(|body| {
            world
                .get::<MotionDriver>(member.actor)
                .is_some_and(|driver| driver.holds(*body))
        });
        prior.body = None;
        settle_completed_actor(world, member.actor, member.locate.end.unwrap_or(member.locate.start))?;
        if let Some(body) = held {
            crate::character::resume_idle_after_fixture(world, member.actor, body)?;
        }
    }
    session.settled = true;
    body_ready(world, session);
    Ok(())
}

fn settle_completed_actor(
    world: &mut World,
    actor: Entity,
    pose: crate::fixture_activity_data::ActivityPose,
) -> Result<(), String> {
    let mut query = world.query::<(&mut Transform, &mut WalkState)>();
    let (mut transform, mut walk) = query.get_mut(world, actor)
        .map_err(|_| "completed pre-action actor navigation is missing")?;
    transform.translation = Vec3::from(pose.position);
    transform.rotation = pose.rotation;
    walk.0.position = pose.position;
    walk.0.forward = (pose.rotation * Vec3::Z).to_array();
    walk.0.next_corner = 0;
    world.entity_mut(actor).insert(crate::npc::MotionPhase::Dwelling { remaining: None });
    Ok(())
}

fn fail(world: &mut World, session: &mut Session, reason: String) {
    warn!(
        "[fixture-talk-action] talk={} failed: {}",
        session.talk, reason
    );
    if session.talked {
        session.failed = Some(reason);
        teardown(world, session);
        return;
    }
    if let Some(mut ledger) = world.get_resource_mut::<crate::player_talk::PlayerTalkLedger>() {
        ledger.last_outcome = Some(crate::player_talk::PlayerTalkOutcome::Rejected {
            requested: Some(crate::npc_objective::TalkContent {
                master_id: session.talk,
                backend: crate::npc_objective::TalkBackend::Fixture,
                is_general: None,
            }),
            reason: reason.clone(),
        });
    }
    session.failed = Some(reason);
    teardown(world, session);
    if !session.talked {
        world.write_message(TalkCancelRequest);
    }
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
    let Some(plan) = session
        .plan
        .as_ref()
        .filter(|_| session.talked || !talks_first(session))
    else {
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
    // Reserve the cast only after every binding passes preflight. Navigation
    // owns the approach and local fitting, just as for an autonomous activity.
    if session.prior.is_empty() {
        // The selected Director already fixes the future talk station. Stage
        // an independent observer before the visible approach, not at the
        // completion frame (which would introduce another apparent teleport).
        // Live talk-enabled loops keep fixture framing; never redraw a plan.
        if waits_for_completion(&session.draft.as_ref().unwrap().definition) {
            if let Some(player) = world.get_resource::<ActiveTalk>().and_then(|talk| talk.player) {
                let main = plan.cast.iter().find(|member| member.actor == session.main)
                    .ok_or((false, "main actor is absent from source plan".into()))?;
                let anchor = Vec3::from(main.locate.end.unwrap_or(main.locate.start).position);
                let occupied: Vec<_> = plan.cast.iter().flat_map(|member| [
                    Vec3::from(member.locate.start.position),
                    Vec3::from(member.locate.end.unwrap_or(member.locate.start).position),
                ]).collect();
                crate::field_modes::stage_fixture_talk_observer(
                    world, session.talk, player, anchor, &occupied,
                ).map_err(|reason| (false, reason))?;
            }
        }
        session.elapsed = 0.;
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
                body: None,
            });
            // A run after its talk holds the actor the same way; the talk's
            // own hold already ended with the talk.
            world.entity_mut(member.actor).insert((
                TalkFixtureActorLease {
                    owner: session.owner,
                    approaching: true,
                },
                TalkHold,
            ));
            if let Some(mut driver) = world.get_mut::<MotionDriver>(member.actor) {
                driver.playing = None;
            }
            crate::npc::begin_external_approach(
                world,
                member.actor,
                crate::npc::FitCandidate {
                    position: member.locate.start.position,
                    rotation: member.locate.start.rotation,
                },
            )
            .map_err(|reason| (false, reason))?;
        }
    }
    for member in &plan.cast {
        match world
            .get::<crate::npc::RouteStops>(member.actor)
            .and_then(|route| route.outcome)
        {
            Some(crate::npc::RouteOutcome::Arrived) => {}
            Some(crate::npc::RouteOutcome::Stopped) => {
                return Err((false, format!("unit {} approach stopped", member.unit)))
            }
            None => return Err((true, "cast walking to source action points".into())),
        }
    }
    for (member, prior) in plan.cast.iter().zip(session.prior.iter_mut()) {
        // The movement owner reached the exact source position. Only transfer
        // the authored final orientation and animation ownership here.
        world.get_mut::<WalkState>(member.actor).unwrap().0.forward =
            (member.locate.start.rotation * Vec3::Z).to_array();
        world.get_mut::<Transform>(member.actor).unwrap().rotation = member.locate.start.rotation;
        world
            .get_mut::<TalkFixtureActorLease>(member.actor)
            .unwrap()
            .approaching = false;
        // The pre-action enters the member's body at arrival; its later
        // completion hands the token back through `PriorActor::body`.
        if let Some(mut driver) = world.get_mut::<MotionDriver>(member.actor) {
            driver.enter(&mut prior.body, BodyOwner::FixtureTalk);
        }
    }
    world.init_resource::<FixtureActivityTimelines>();
    // Loading and navigation do not consume the authored Director's time.
    // Some pre-actions (including the horse statue) last almost 18 seconds
    // and intentionally never enable talk until their complete ending.
    session.elapsed = 0.;
    session.timeout = (session.draft.as_ref().unwrap().definition.duration as f32 + 5.).max(18.);
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

fn waits_for_completion(definition: &timeline::TimelineDefinition) -> bool {
    let mut has_loop_gate = false;
    for clip in definition.tracks.iter().flat_map(|track| &track.clips) {
        if let timeline::TimelinePayload::LoopFlag { enable_talk, .. } = clip.payload {
            has_loop_gate = true;
            if enable_talk { return false; }
        }
    }
    has_loop_gate
}

pub fn drive(world: &mut World, mut cancels: Local<MessageCursor<TalkCancelRequest>>) {
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
    let cancelled = world
        .get_resource::<Messages<TalkCancelRequest>>()
        .is_some_and(|messages| cancels.read(messages).count() > 0);
    let mut state = world
        .remove_resource::<FixtureTalkAction>()
        .unwrap_or_default();
    // A cancel request or the next talk ends a pre-action left running after
    // its talk.
    if cancelled || wanted.is_some() {
        if let Some(mut after) = state.after_talk.take() {
            teardown(world, &mut after);
        }
    }
    // The talk may end on a later frame than the request was read.
    if cancelled {
        if let Some(session) = state.session.as_mut() {
            session.cancelled = true;
        }
    }
    if state
        .session
        .as_ref()
        .is_some_and(|session| Some(session.owner) != active_owner)
    {
        if let Some(old) = state.session.take() {
            end_talk(world, &mut state, old);
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
                timeout: 120.,
                pending: "source pre-action preparation".into(),
                failed: None,
                ready: false,
                talked: false,
                settled: false,
                cancelled: false,
            });
        }
    }
    if let Some(session) = state.session.as_mut() {
        advance(world, session);
    }
    if let Some(after) = state.after_talk.as_mut() {
        advance(world, after);
        // A failure has already torn the run down; a completed run releases
        // its cast where the pre-action left it.
        if after.failed.is_some() || after.settled {
            let mut after = state.after_talk.take().unwrap();
            if after.failed.is_none() {
                teardown(world, &mut after);
            }
        }
    }
    world.insert_resource(state);
}

/// A run after its talk that has not started its pre-action yet.
fn waits_to_start(session: &Session) -> bool {
    session.talked && session.token.is_none() && !session.settled
}

/// Advances one session by a frame.
fn advance(world: &mut World, session: &mut Session) {
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
                Some(TimelineStatus::Completed) => {
                    if let Err(reason) = complete_pre_action(world, session) {
                        fail(world, session, reason);
                    }
                }
                Some(TimelineStatus::Failed(error)) => fail(world, session, error.to_string()),
                Some(TimelineStatus::Cancelled) | None => {
                    fail(world, session, "source pre-action cancelled".into())
                }
                _ => {}
            }
        } else if !session.plan_resolved || session.plan.is_some() && (!session.ready || waits_to_start(session)) {
            if let Err((retryable, reason)) = prepare(world, session) {
                if retryable {
                    session.pending = reason;
                } else {
                    fail(world, session, reason);
                }
            }
        }
        if (!session.ready || waits_to_start(session))
            && session.failed.is_none()
            && session.elapsed > session.timeout
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_preparation_releases_draft_before_cancel_message_is_consumed() {
        use std::sync::Arc;
        let mut app = App::new();
        app.add_message::<TalkCancelRequest>();
        let world = app.world_mut();
        let owner = world.spawn_empty().id();
        let identity = timeline::SourceAssetId { file: "source".into(), path_id: "1".into() };
        let definition = Arc::new(timeline::TimelineDefinition {
            director_transforms: None,
            package: "fixture".into(), prefab: "fixture".into(), fixture_view: None,
            director: identity.clone(), timeline: identity, duration: 1.0, tracks: Vec::new(),
        });
        let weak = Arc::downgrade(&definition);
        let mut session = Session {
            owner, talk: 970, main: owner, anchor: owner, cast: Vec::new(),
            plan: None, plan_resolved: true,
            draft: Some(StartTimeline {
                owner: TimelineOwner {
                    activity: FixtureActivityOwner { actor: owner, generation: 1 },
                    kind: TimelineOwnerKind::Talk,
                },
                fixture: owner, definition, bindings: Default::default(), companions: Vec::new(),
                timeout_secs: 30.0,
                timeout_budget: TimelineTimeoutBudget::OwnerGated { advance: Some(true) },
            }),
            token: None, prior: Vec::new(), elapsed: 0.0, timeout: 120.0,
            pending: "prepared first effect; second failed".into(), failed: None, ready: false,
            talked: false, settled: false, cancelled: false,
        };
        assert!(weak.upgrade().is_some());
        fail(world, &mut session, "source preflight failed".into());
        assert!(session.draft.is_none());
        assert!(weak.upgrade().is_none());
        assert_eq!(session.failed.as_deref(), Some("source preflight failed"));
        assert!(world.get_entity(owner).is_ok());
    }

    #[test]
    fn completed_actor_uses_source_endloc_and_synchronizes_navigation() {
        let mut world = World::new();
        let owner = world.spawn_empty().id();
        let actor = world.spawn((
            Transform::from_xyz(0.125, 0., 1.075).with_scale(Vec3::splat(1.2)),
            WalkState(moly_law::path::WalkState::new([0.125, 0., 1.075], [0., 0., 1.])),
            TalkFixtureActorLease { owner, approaching: false },
        )).id();
        let end = crate::fixture_activity_data::ActivityPose {
            position: [0.625, 0., -0.775], rotation: Quat::from_rotation_y(std::f32::consts::PI),
        };
        settle_completed_actor(&mut world, actor, end).unwrap();
        let pose = world.get::<Transform>(actor).unwrap();
        let walk = &world.get::<WalkState>(actor).unwrap().0;
        assert_eq!(pose.translation.to_array(), end.position);
        assert_eq!(pose.rotation, end.rotation);
        assert_eq!(pose.scale, Vec3::splat(1.2));
        assert_eq!(walk.position, end.position);
        assert_eq!(walk.forward, (end.rotation * Vec3::Z).to_array());
        assert!(!world.get::<TalkFixtureActorLease>(actor).unwrap().approaching);
    }
}
