//! A companion's walk: the walk to its spawn point, the route's step and
//! its outcome, the clip that follows from it, and the status line.

use super::{
    turn_to, Companion, CompanionState, CompanionStatus, Gait, Pending, Walker, MAX_STEP,
    WALK_SPEED,
};
use crate::npc::companion::{self, Motion, Watch};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::{AvatarDriver, IDLE_CLIP, STATE_FADE, WALK_CLIP};
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;

/// Update: a companion whose body is wired this frame is shown, or walks
/// from its spawn seat to the spawn point it was seated away from.
#[allow(clippy::too_many_arguments)]
pub fn plan_walk(
    mut commands: Commands,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    mut companions: Query<(
        Entity,
        &Companion,
        &Pending,
        &mut WalkState,
        &mut PathSlot,
        &mut RouteStops,
        &mut MotionPhase,
        &mut Transform,
        &mut CompanionStatus,
    )>,
) {
    for (
        entity,
        companion,
        pending,
        mut walk,
        mut slot,
        mut route,
        mut phase,
        mut transform,
        mut status,
    ) in &mut companions
    {
        let seat = |commands: &mut Commands, entity: Entity| {
            commands.entity(entity).insert(Visibility::Inherited);
            commands.entity(entity).insert(Pending::Show);
        };
        match pending {
            Pending::Show => {
                commands.entity(entity).insert(Visibility::Inherited);
                commands.entity(entity).remove::<Pending>();
                status.state = CompanionState::Idle;
                let at = transform.translation;
                info!(
                    "[companion] unit {}: shown at ({:.2},{:.2},{:.2})",
                    companion.unit, at.x, at.y, at.z
                );
            }
            Pending::ArriveAt(at) => {
                let (Some(face), Some(objective)) = (face.as_deref(), objective.as_deref()) else {
                    seat(&mut commands, entity);
                    continue;
                };
                match companion::depart(
                    companion.unit,
                    &mut walk,
                    &mut slot,
                    &mut route,
                    face,
                    objective,
                    at.to_array(),
                    0.3,
                ) {
                    Ok(next) => {
                        *phase = next;
                        if let Some(turned) = turn_to(transform.translation, *at) {
                            transform.rotation = turned;
                        }
                        // Shown from its seat on; the spawn is a walk in
                        // flight until it arrives.
                        commands.entity(entity).insert((Walker::started(), Visibility::Inherited));
                        commands.entity(entity).remove::<Pending>();
                        status.state = CompanionState::Walking;
                        info!(
                            "[companion] unit {}: walks from its seat to the spawn point {at:?}",
                            companion.unit
                        );
                    }
                    Err(reason) => {
                        info!(
                            "[companion] unit {}: stays at its seat; the spawn point is out of reach: {reason}",
                            companion.unit
                        );
                        seat(&mut commands, entity);
                    }
                }
            }
        }
    }
}

/// Update: a walking companion's route, one agent step per frame on the
/// roster's frame clock, and the speed it moved at.
#[allow(clippy::too_many_arguments)]
pub fn step_motion(
    clock: Res<crate::npc_clock::NpcClock>,
    time: Res<Time>,
    frame: Res<bevy::diagnostic::FrameCount>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    mut companions: Query<(
        &Companion,
        &mut WalkState,
        &mut PathSlot,
        &mut RouteStops,
        &mut MotionPhase,
        &mut Transform,
        &mut Walker,
        &mut Gait,
    )>,
) {
    let (Some(face), Some(objective)) = (face.as_deref(), objective.as_deref()) else {
        return;
    };
    if !clock.ready() {
        return;
    }
    // The stuck watchdog keeps this host's elapsed seconds, as the roster's does.
    let now = time.elapsed_secs();
    let dt = clock.delta().min(MAX_STEP);
    for (
        companion,
        mut walk,
        mut slot,
        mut route,
        mut phase,
        mut transform,
        mut runtime,
        mut gait,
    ) in &mut companions
    {
        if !runtime.walks() || dt <= 0.0 {
            continue;
        }
        // A route of another navigation build is not walked on this one: the
        // walk ends where the companion stands.
        if !route.follows_generation(face.generation()) {
            route.stop();
            *phase = MotionPhase::Dwelling { remaining: None };
            let watch = runtime.watch.map_or_else(|| Watch::at(now), |watch| watch.at_time(now));
            runtime.end(watch, Motion::Stopped);
            continue;
        }
        let from = transform.translation;
        let mut watch = runtime.watch.map_or_else(|| Watch::at(now), |watch| watch.at_time(now));
        let motion = companion::advance(
            companion.unit,
            WALK_SPEED,
            &mut walk,
            &mut slot,
            &mut route,
            &mut watch,
            &mut transform,
            &mut phase,
            face,
            objective,
            dt,
        );
        runtime.end(watch, motion);
        let moved = transform.translation - from;
        gait.speed = Vec3::new(moved.x, 0.0, moved.z).length() / dt;
        gait.stepped = Some(frame.0);
    }
}

/// Update: the outcome of every walk that ended, before this frame's
/// commands; a companion that took no step this frame stands.
pub fn route_outcome(
    frame: Res<bevy::diagnostic::FrameCount>,
    mut companions: Query<(&mut CompanionStatus, &mut Walker, &mut Gait)>,
) {
    for (mut status, mut runtime, mut gait) in &mut companions {
        match runtime.take() {
            Some(Motion::Arrived) if status.state == CompanionState::Walking => {
                status.state = CompanionState::Done;
                info!("[companion] seq {}: the walk arrived", status.seq);
            }
            Some(Motion::Stopped) if status.state == CompanionState::Walking => {
                status.fail(companion::WALK_STOPPED);
                error!(
                    "[companion] seq {}: the walk failed: {}",
                    status.seq,
                    companion::WALK_STOPPED
                );
            }
            _ => {}
        }
        if gait.stepped != Some(frame.0) {
            gait.speed = 0.0;
        }
    }
}

/// Update: the clip of each companion, once its body is up: the idle clip
/// while it stands, the walk clip while its walk is in flight (its step
/// may not move it every frame), each fading in as the player's
/// locomotion states do.
pub fn drive_clips(
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut companions: Query<(&AvatarDriver, &mut Gait, &CompanionStatus, Has<Pending>)>,
) {
    for (driver, mut gait, status, pending) in &mut companions {
        if pending || status.state == CompanionState::Spawned {
            continue;
        }
        let walking = status.state == CompanionState::Walking;
        let clip = if walking { WALK_CLIP } else { IDLE_CLIP };
        if gait.playing == Some(clip) {
            continue;
        }
        let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
            continue;
        };
        let Some(node) = driver.node_of(clip) else {
            continue;
        };
        let animation = transitions.play(&mut animator, node, STATE_FADE);
        animation.set_repeat(RepeatAnimation::Forever);
        gait.playing = Some(clip);
        info!(
            "[companion] seq {}: PlayAnimation({clip}, fade {:.2}s)",
            status.seq,
            STATE_FADE.as_secs_f32()
        );
    }
}

/// Update: the periodic status line of each companion.
pub fn report(companions: Query<(&Companion, &CompanionStatus, &Transform)>) {
    for (companion, status, transform) in &companions {
        let state = match &status.state {
            CompanionState::Failed(reason) => format!("failed ({reason})"),
            state => state.label().to_owned(),
        };
        let at = transform.translation;
        info!(
            "[companion] unit {} seq {} {state} at ({:.2},{:.2},{:.2})",
            companion.unit, status.seq, at.x, at.y, at.z
        );
    }
}
