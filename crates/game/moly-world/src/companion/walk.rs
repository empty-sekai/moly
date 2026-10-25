//! A companion's walk: the walk to its spawn point, the route's step and
//! its outcome, the clip that follows from it, and the status line.

use super::own_motion;
use super::{
    turn_to, Companion, CompanionState, CompanionStatus, Gait, Pending, Walker, MAX_STEP,
};
use crate::npc::companion::{self, Motion, Watch};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::oc::own::{EndOwn, Family, OwnAnimator, PlayOwn};
use crate::player_avatar::oc::OcModel;
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

/// The stick a companion walks with: pushed all the way, as the player walks
/// with the stick at its rim.
const WALK_STICK: f32 = 1.0;

/// The pace of a companion walking with the avatar's walk clip: the player's
/// Move state with the stick at [`WALK_STICK`] (speed, animation speed).
/// `None` until the site and the client configs are in.
fn avatar_pace(
    configs: Option<&crate::client_config::ClientConfigs>,
    site: Option<&crate::site::SiteSelection>,
) -> Option<(f32, f32)> {
    Some(crate::player::move_pace(configs?, site?, WALK_STICK))
}

/// Update: a walking companion's route, one agent step per frame on the
/// roster's frame clock, at the avatar's pace ([`avatar_pace`]), and the
/// speed it moved at.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn step_motion(
    clock: Res<crate::npc_clock::NpcClock>,
    time: Res<Time>,
    frame: Res<bevy::diagnostic::FrameCount>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
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
        let Some((speed, _)) = avatar_pace(configs.as_deref(), site.as_deref()) else {
            continue;
        };
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
            speed,
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
#[allow(clippy::type_complexity)]
pub fn route_outcome(
    mut commands: Commands,
    frame: Res<bevy::diagnostic::FrameCount>,
    mut ghosts: ResMut<super::FixtureGhosts>,
    mut companions: Query<(
        Entity,
        &mut CompanionStatus,
        &mut Walker,
        &mut Gait,
        &Transform,
        Option<&mut super::build::BuildPlan>,
        Option<&mut super::use_fixture::UsePlan>,
    )>,
) {
    for (entity, mut status, mut runtime, mut gait, transform, plan, using) in &mut companions {
        let ended = runtime.take();
        // The walk of a use's approach ends in its action, not in Done.
        if let (Some(motion), Some(mut using)) = (ended, using) {
            if motion != Motion::Walking {
                super::use_fixture::walk_ended(&mut using, transform.translation, motion == Motion::Arrived);
            }
            if gait.stepped != Some(frame.0) {
                gait.speed = 0.0;
            }
            continue;
        }
        // The walk of a build's approach ends in its swing, not in Done.
        if let (Some(motion), Some(mut plan)) = (ended, plan) {
            if status.state == CompanionState::Walking && motion != Motion::Walking {
                let at = transform.translation;
                if !super::build::walk_ended(&mut plan, &mut status, &mut ghosts, at, motion == Motion::Arrived) {
                    commands.entity(entity).remove::<super::build::BuildPlan>();
                }
            }
            if gait.stepped != Some(frame.0) {
                gait.speed = 0.0;
            }
            continue;
        }
        match ended {
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

/// Update, after the walks and the builds: the motion source of each
/// companion's body.
///
/// The body plays the avatar's idle and walk clips, retargeted onto it, and
/// the build's swing on the avatar's animator. A motion ([`super::act`])
/// hands the body to its own animator ([`own_motion`]): the motion's
/// segments play on the body's own skeleton, and once the motion is over the
/// body fades back to the avatar's clip. The avatar skeleton plays its clips
/// throughout, so the body comes back to its live pose; the hand-over each
/// way fades (the avatar's clip with [STATE_FADE], the body's own clips from
/// the first frame its own motion is calibrated).
#[allow(clippy::too_many_arguments)]
pub fn drive_clips(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
    bodies: Query<(Entity, &OcModel), With<OwnAnimator>>,
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut companions: Query<(
        Entity,
        &mut AvatarDriver,
        &mut Gait,
        &CompanionStatus,
        Option<&super::build::Hammering>,
    )>,
) {
    for (entity, mut driver, mut gait, status, hammering) in &mut companions {
        // A spawning body has no clips yet.
        if status.state == CompanionState::Spawned {
            continue;
        }
        // The build's swing holds the avatar's animator while the hammer is in
        // the hand, and its clip plays out until the hammer is put down.
        let swinging = hammering.is_some() || gait.hammer_played;
        let body = own_motion::own_body(&bodies, entity);
        let source = if swinging {
            Source::Swing
        } else if matches!(status.state, CompanionState::Acting | CompanionState::Using) && body.is_some() {
            Source::Own
        } else {
            Source::Avatar(own_motion::family(&status.state))
        };
        if gait.source == Some(source) {
            continue;
        }
        // The body fades from its own pose, if it had one, back to the
        // avatar's.
        if gait.source == Some(Source::Own) && source != Source::Own {
            if let Some(body) = body {
                commands.write_message(EndOwn(body));
            }
        }
        match source {
            Source::Own => {
                // The motion's row is queued for the body: its segments start
                // once the body's own animator owns it.
                if let Some(body) = body {
                    commands.write_message(PlayOwn(body));
                }
                gait.source = Some(source);
                info!("[companion] seq {}: the body's own motion", status.seq);
            }
            Source::Avatar(family) => {
                // The walk clip plays at the pace the walk moves at.
                let rate = match family {
                    Family::Walk => match avatar_pace(configs.as_deref(), site.as_deref()) {
                        Some((_, rate)) => rate,
                        None => continue,
                    },
                    _ => 1.0,
                };
                if play_avatar(&mut driver, &mut animators, &mut graphs, status, family, rate) {
                    gait.source = Some(source);
                }
            }
            // The swing plays its own clip on the avatar's animator.
            Source::Swing => gait.source = Some(source),
        }
    }
}

/// The motion source of a companion's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// A motion's segments, on the body's own skeleton.
    Own,
    /// The avatar's idle or walk clip, retargeted onto the body.
    Avatar(Family),
    /// A build's swing, which plays its clip on the avatar's animator itself.
    Swing,
}

/// The avatar's idle or walk clip, faded in as the player's locomotion
/// states do; `false` when it cannot play.
fn play_avatar(
    driver: &mut AvatarDriver,
    animators: &mut Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    graphs: &mut Assets<AnimationGraph>,
    status: &CompanionStatus,
    family: Family,
    rate: f32,
) -> bool {
    let clip = if family == Family::Walk { WALK_CLIP } else { IDLE_CLIP };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        return false;
    };
    // The clip joins the body's graph on its first play.
    let node = match driver.node(clip, graphs) {
        Ok((node, _)) => node,
        Err(error) => {
            error!("[companion] seq {}: {clip} cannot play: {error}", status.seq);
            return false;
        }
    };
    transitions
        .play(&mut animator, node, STATE_FADE)
        .set_repeat(RepeatAnimation::Forever)
        .set_speed(rate);
    info!(
        "[companion] seq {}: PlayAnimation({clip}, fade {:.2}s, speed {rate:.2})",
        status.seq,
        STATE_FADE.as_secs_f32()
    );
    true
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
