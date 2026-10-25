//! A companion uses a placed fixture: its character's own action on that
//! fixture (a no-talk action, or the pre-action a talk of the unit holds
//! there), the one a roster member of the same unit plays there.
//!
//! A use is one command ([`super::CompanionCommand::Use`]). The action point
//! is the roster's choice for that action
//! ([`crate::npc_fixture_activity::companion`]); the companion walks to it
//! on the roster's navigation, then the action's source timeline plays on
//! the body's own animator while the companion stands on the action's
//! StartLoc, and the companion ends on its EndLoc.

use super::{faces, turn_to, Companion, CompanionState, CompanionStatus, Walker};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::npc_fixture_activity::companion::{self as activity, CompanionAction};
use crate::player_avatar::oc::own::OwnAnimator;
use crate::player_avatar::oc::OcModel;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;

/// How close to the action point a walk ends; the action's own fit puts the
/// companion on the StartLoc from there.
const REACH: f32 = 0.3;

/// A use in flight on a companion.
#[derive(Component)]
pub struct UsePlan {
    pub seq: u64,
    pub uid: String,
    activity: Option<i32>,
    variant: Option<u32>,
    phase: UsePhase,
    action: Option<CompanionAction>,
    body: Option<Entity>,
}

#[derive(Clone, Debug, PartialEq)]
enum UsePhase {
    /// The action point is not chosen yet.
    Select,
    /// Walking to the action point.
    Approach { landing: Vec3 },
    /// At the action point: the timeline loads.
    Prepare,
    /// The timeline plays.
    Play,
    /// The use ended in a refusal, named.
    Failed(String),
}

impl UsePlan {
    pub fn new(seq: u64, uid: String, activity: Option<i32>, variant: Option<u32>) -> Self {
        Self { seq, uid, activity, variant, phase: UsePhase::Select, action: None, body: None }
    }
}

/// What the walk of a use's approach ended with: the action, or the use's
/// failure when it stopped short of the action point.
pub(super) fn walk_ended(plan: &mut UsePlan, at: Vec3, arrived: bool) -> bool {
    let UsePhase::Approach { landing } = plan.phase else {
        return false;
    };
    plan.phase = if arrived || at.distance(landing) <= REACH {
        UsePhase::Prepare
    } else {
        UsePhase::Failed("the walk stopped before the action point".into())
    };
    true
}

/// Update, after the companion steps: each use goes as far as it can this
/// frame. A use ends with the companion's status: done on the EndLoc, or
/// failed with its reason; either way every lease of the action is back.
pub(super) fn advance(world: &mut World) {
    let users: Vec<Entity> = world
        .query_filtered::<Entity, With<UsePlan>>()
        .iter(world)
        .collect();
    for entity in users {
        let Some(mut plan) = world.entity_mut(entity).take::<UsePlan>() else {
            continue;
        };
        let unit = world.get::<Companion>(entity).map_or(0, |companion| companion.unit);
        let outcome = step(world, entity, unit, &mut plan);
        let ended = match outcome {
            Ok(false) => {
                world.entity_mut(entity).insert(plan);
                continue;
            }
            Ok(true) => Ok(()),
            Err(reason) => Err(reason),
        };
        if let Some(action) = plan.action.take() {
            action.release(world);
        }
        let Some(mut status) = world.get_mut::<CompanionStatus>(entity) else {
            continue;
        };
        match ended {
            Ok(()) => {
                status.state = CompanionState::Done;
                info!("[companion] unit {unit} use {}: fixture {} used", plan.seq, plan.uid);
            }
            Err(reason) => {
                status.fail(&reason);
                error!("[companion] unit {unit} use {} of fixture {} failed: {reason}", plan.seq, plan.uid);
            }
        }
    }
}

/// One frame of a use: `Ok(true)` once the action has ended.
fn step(world: &mut World, entity: Entity, unit: u32, plan: &mut UsePlan) -> Result<bool, String> {
    match plan.phase.clone() {
        UsePhase::Select => {
            let body = world
                .query_filtered::<(Entity, &OcModel), With<OwnAnimator>>()
                .iter(world)
                .find(|(_, model)| model.player == entity)
                .map(|(body, _)| body)
                .ok_or("the body has no own animator")?;
            let action = activity::select(world, entity, body, unit, &plan.uid, plan.activity, plan.variant)?;
            let landing = action.landing();
            plan.body = Some(body);
            plan.action = Some(action);
            plan.phase = depart(world, entity, unit, landing)?;
            Ok(false)
        }
        UsePhase::Approach { .. } => Ok(false),
        UsePhase::Prepare => {
            let body = plan.body.ok_or("the use has no body")?;
            let animator = world
                .get::<OwnAnimator>(body)
                .map(|own| own.animator)
                .ok_or("the body lost its own animator")?;
            let graph = world
                .get::<AnimationGraphHandle>(animator)
                .map(|graph| graph.0.clone())
                .ok_or("the body's animator has no graph")?;
            let action = plan.action.as_mut().ok_or("the use has no action")?;
            if !action.prepare(world, animator, graph)? {
                return Ok(false);
            }
            action.start(world)?;
            // The body's own animator holds it while the action plays.
            if let Some(mut status) = world.get_mut::<CompanionStatus>(entity) {
                status.state = CompanionState::Using;
            }
            plan.phase = UsePhase::Play;
            Ok(false)
        }
        UsePhase::Play => plan.action.as_mut().ok_or("the use has no action")?.poll(world),
        UsePhase::Failed(reason) => Err(reason),
    }
}

/// The walk to the action point, or the action at once when the companion
/// already stands there.
fn depart(world: &mut World, entity: Entity, unit: u32, landing: Vec3) -> Result<UsePhase, String> {
    let mut state = SystemState::<(
        Option<Res<crate::site::SiteActive>>,
        Option<Res<crate::site::GroundEpoch>>,
        Option<Res<crate::walk_face::WalkFace>>,
        Option<Res<crate::npc_objective::ObjectiveFace>>,
        Query<(
            &mut Transform,
            &mut WalkState,
            &mut PathSlot,
            &mut RouteStops,
            &mut MotionPhase,
            &mut CompanionStatus,
        )>,
    )>::new(world);
    {
        let (site, epoch, face, objective, mut companions) =
            state.get_mut(world).map_err(|error| format!("the companion steps cannot run: {error}"))?;
        let (face, objective) = faces(site.as_deref(), epoch.as_deref(), face.as_deref(), objective.as_deref())
            .ok_or("no navigation on this site")?;
        let (mut transform, mut walk, mut slot, mut route, mut phase, mut status) =
            companions.get_mut(entity).map_err(|_| "the companion's navigation state is in use")?;
        let from = transform.translation;
        if from.distance(landing) <= REACH {
            return Ok(UsePhase::Prepare);
        }
        let next = crate::npc::companion::depart(
            unit,
            &mut walk,
            &mut slot,
            &mut route,
            face,
            objective,
            landing.to_array(),
            0.1,
        )?;
        *phase = next;
        if let Some(turned) = turn_to(from, landing) {
            transform.rotation = turned;
        }
        status.state = CompanionState::Walking;
    }
    world.entity_mut(entity).insert(Walker::started());
    Ok(UsePhase::Approach { landing })
}
