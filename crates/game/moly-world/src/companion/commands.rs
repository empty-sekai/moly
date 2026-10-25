//! The companion commands, one system each: a spawn asks for its character
//! body, a walk plans its route, a turn, a build, a motion and a despawn
//! each write the status of the companion and the components the next steps
//! read.
//!
//! A command about a companion that is not spawned has no status to carry
//! its refusal: it is logged as `[companion] <command> of unit <unit>
//! refused: not spawned`. Every system reads the whole message stream, taking
//! the commands of its own kind in the order of arrival; the systems are
//! chained, so the messages are read once each.

use crate::companion::{
    act, build, face, turn_to, use_fixture, Companion, CompanionSpawn, CompanionState,
    CompanionStatus, FixtureGhosts, Walker, WITHIN_LIMIT,
};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use bevy::asset::{AssetPath, LoadState};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

/// Logs the command's refusal for a companion that is not spawned.
fn not_spawned(command: &crate::companion::CompanionCommand, unit: u32) {
    use crate::companion::CompanionCommand;
    let what = match command {
        CompanionCommand::WalkTo { .. } => "walk",
        CompanionCommand::Face { .. } => "face",
        CompanionCommand::Build { .. } => "build",
        CompanionCommand::Motion { .. } => "motion",
        CompanionCommand::Use { .. } => "use",
        CompanionCommand::Spawn { .. } => "spawn",
        CompanionCommand::Despawn { .. } => "despawn",
    };
    error!("[companion] {what} of unit {unit} refused: not spawned");
}

/// A companion's body: the status a command writes, and the walk state,
/// path, route and motion phase a walk works with.
type Bodies<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Companion,
        &'static mut CompanionStatus,
        &'static mut Transform,
        &'static mut WalkState,
        &'static mut PathSlot,
        &'static mut RouteStops,
        &'static mut MotionPhase,
    ),
>;

/// Update: every spawn, in arrival order. A spawn whose character file failed
/// is refused by name: no body was built to carry the refusal.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    sweeps: Query<(), With<crate::field_modes::SiteMoveActive>>,
    spawns: Query<(Entity, &CompanionSpawn)>,
    companions: Query<&Companion>,
) {
    for (entity, spawn) in &spawns {
        if let LoadState::Failed(error) = server.load_state(&spawn.gltf) {
            error!(
                "[companion] spawn of unit {} refused: character body {} failed to load: {error:#}",
                spawn.unit, spawn.glb
            );
            commands.entity(entity).despawn();
        }
    }
    for command in requests.read() {
        let crate::companion::CompanionCommand::Spawn { unit, at } = command else {
            continue;
        };
        let spawned = companions.iter().any(|body| body.unit == *unit);
        // A spawn in flight counts as spawned.
        if spawned || spawns.iter().any(|(_, spawn)| spawn.unit == *unit) {
            error!("[companion] spawn of unit {unit} refused: already spawned");
            continue;
        }
        // A site move in flight keeps no walkable surface.
        if sweeps.iter().next().is_some()
            || crate::companion::faces(
                site.as_deref(),
                epoch.as_deref(),
                face.as_deref(),
                objective.as_deref(),
            )
            .is_none()
        {
            error!("[companion] spawn of unit {unit} refused: no navigation on this site");
            continue;
        }
        let Some((glb, rig_file)) = crate::companion::manifest_row(&server, &jsons, *unit) else {
            error!(
                "[companion] spawn of unit {unit} refused: unknown unit (the character manifest has no body for it)"
            );
            continue;
        };
        // The scene is bound to the avatar skeleton, as a model file's: no
        // animations, lights or cameras; the rig profile gives it the cast's
        // materials.
        let gltf = server
            .load_builder()
            .with_settings(|settings: &mut bevy::gltf::GltfLoaderSettings| {
                settings.load_materials = moly_assets::residency::GPU_ONLY;
                settings.load_cameras = false;
                settings.load_lights = false;
                settings.load_animations = false;
            })
            .load(moly_assets::character_glb(&glb));
        let rig = server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig_file}")));
        info!("[companion] unit {unit}: character body {glb} requested at {at:?}");
        commands.spawn((
            Name::new(format!("companion spawn unit {unit}")),
            CompanionSpawn {
                unit: *unit,
                at: *at,
                glb,
                rig_file,
                gltf,
                rig,
            },
        ));
    }
}

/// Update: every turn, in arrival order. A turn comes after the build, walk
/// or motion in flight, not over it.
pub(super) fn face_command(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    builds: Query<&build::BuildPlan>,
    mut companions: Query<(Entity, &Companion, &mut CompanionStatus, &Transform, &WalkState)>,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::Face { unit, to, secs } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status, transform, walk)) =
            companions.iter_mut().find(|(_, body, ..)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        if status.state == CompanionState::Spawned {
            status.refuse("the body is still loading");
            error!("[companion] unit {unit} face {} refused: the body is still loading", status.seq);
            continue;
        }
        if builds.get(entity).is_ok() {
            status.refuse("a build is already in flight");
            error!("[companion] unit {unit} face refused: a build is already in flight");
            continue;
        }
        if status.state == CompanionState::Walking {
            status.refuse("a walk is already in flight");
            error!("[companion] unit {unit} face refused: a walk is already in flight");
            continue;
        }
        if status.state == CompanionState::Acting {
            status.refuse("a motion is already in flight");
            error!("[companion] unit {unit} face refused: a motion is already in flight");
            continue;
        }
        let seq = status.accept();
        // The turn starts where the body faces: the forward its walk state
        // holds (its rotation is not read here).
        let at = transform.translation;
        let forward = walk.0.forward;
        let yaw = forward[0].atan2(forward[2]);
        info!(
            "[companion] unit {unit} face {seq}: to {to:?} over {secs:.2}s from ({:.2},{:.2},{:.2})",
            at.x, at.y, at.z
        );
        status.state = CompanionState::Turning;
        commands
            .entity(entity)
            .insert(face::Facing::new(yaw, *to, at, *secs));
    }
}

/// Update: every build, in arrival order.
pub(super) fn build_command(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    mut ghosts: ResMut<FixtureGhosts>,
    builds: Query<&build::BuildPlan>,
    mut companions: Query<(Entity, &Companion, &mut CompanionStatus)>,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::Build { unit, uid, at, swings } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status)) = companions.iter_mut().find(|(_, body, _)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        if status.state == CompanionState::Spawned {
            status.refuse("the body is still loading");
            error!(
                "[companion] unit {unit} build {} refused: the body is still loading",
                status.seq
            );
            continue;
        }
        if builds.get(entity).is_ok() {
            status.refuse("a build is already in flight");
            error!("[companion] unit {unit} build refused: a build is already in flight");
            continue;
        }
        let seq = status.accept();
        let swings = match *swings {
            0 => build::DEFAULT_SWINGS,
            swings => swings.min(build::MAX_SWINGS),
        };
        if ghosts.alpha(uid).is_none() {
            ghosts.set(uid, build::GHOST_ALPHA);
        }
        status.state = CompanionState::Walking;
        info!("[companion] unit {unit} build {seq}: fixture {uid} at {at:?}, {swings} swings");
        commands.entity(entity).insert(build::BuildPlan {
            seq,
            uid: uid.clone(),
            at: *at,
            swings,
            phase: build::BuildPhase::Navigation,
            waited: 0.0,
        });
    }
}

/// Update: every motion, in arrival order. A motion comes after the build or
/// turn in flight; a walk in flight is stopped where the companion stands.
#[allow(clippy::too_many_arguments)]
pub(super) fn motion(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    server: Res<AssetServer>,
    libraries: Res<Assets<bevy::gltf::Gltf>>,
    bodies: Query<
        (Entity, &crate::player_avatar::oc::OcModel),
        With<crate::player_avatar::oc::own::OwnAnimator>,
    >,
    builds: Query<&build::BuildPlan>,
    mut companions: Query<(Entity, &Companion, &mut CompanionStatus)>,
    mut walks: Query<(
        &mut WalkState,
        &mut PathSlot,
        &mut RouteStops,
        &mut MotionPhase,
    )>,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::Motion { unit, motion } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status)) = companions.iter_mut().find(|(_, body, _)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        if status.state == CompanionState::Spawned {
            status.refuse("the body is still loading");
            error!(
                "[companion] unit {unit} motion {} refused: the body is still loading",
                status.seq
            );
            continue;
        }
        if builds.get(entity).is_ok() {
            status.refuse("a build is already in flight");
            error!("[companion] unit {unit} motion refused: a build is already in flight");
            continue;
        }
        if status.state == CompanionState::Turning {
            status.refuse("a turn is already in flight");
            error!("[companion] unit {unit} motion refused: a turn is already in flight");
            continue;
        }
        if status.state == CompanionState::Acting {
            status.refuse("a motion is already in flight");
            error!("[companion] unit {unit} motion refused: a motion is already in flight");
            continue;
        }
        let walked = if status.state == CompanionState::Walking {
            act::stop_walk(entity, &mut walks)
        } else {
            Ok(())
        };
        let checked = act::refuse(entity, motion, &bodies, &server, &libraries);
        match walked.and(checked) {
            Ok(body) => {
                // The body's one-shot row is queued; the body's own-motion
                // offer starts its segments once the body can take them.
                commands.run_system_cached_with(crate::player_avatar::oc::own::queue_once, (body, motion.clone()));
                let seq = status.accept();
                status.state = CompanionState::Acting;
                commands.entity(entity).insert(act::Acting::new(seq, motion));
                info!("[companion] unit {unit} motion {seq}: {motion}");
            }
            Err(reason) => {
                status.refuse(&reason);
                error!("[companion] unit {unit} motion {} refused: {reason}", status.seq);
            }
        }
    }
}

/// Update: every despawn, in arrival order. A build in flight is given up,
/// and its fixture stands solid.
pub(super) fn despawn(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    mut ghosts: ResMut<FixtureGhosts>,
    builds: Query<&build::BuildPlan>,
    mut companions: Query<(Entity, &Companion, &mut CompanionStatus)>,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::Despawn { unit } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status)) = companions.iter_mut().find(|(_, body, _)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        if let Ok(plan) = builds.get(entity) {
            ghosts.clear(&plan.uid);
        }
        let seq = status.accept();
        status.state = CompanionState::Done;
        info!("[companion] unit {unit} despawn {seq}");
        commands.entity(entity).despawn();
    }
}

/// Update: every use, in arrival order. A companion that is building or
/// already using a fixture refuses it.
#[allow(clippy::type_complexity)]
pub(super) fn use_command(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    mut companions: Query<(
        Entity,
        &Companion,
        &mut CompanionStatus,
        Has<use_fixture::UsePlan>,
        Has<build::BuildPlan>,
    )>,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::Use { unit, uid, activity, variant } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status, using, building)) =
            companions.iter_mut().find(|(_, body, ..)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        let busy = if status.state == CompanionState::Spawned {
            Some("the body is still loading")
        } else if building {
            Some("a build is in flight")
        } else if using {
            Some("another use is in flight")
        } else {
            None
        };
        if let Some(reason) = busy {
            status.refuse(reason);
            error!("[companion] unit {unit} use {} refused: {reason}", status.seq);
            continue;
        }
        let seq = status.accept();
        status.state = CompanionState::Walking;
        info!("[companion] unit {unit} use {seq}: fixture {uid}");
        commands
            .entity(entity)
            .insert(use_fixture::UsePlan::new(seq, uid.clone(), *activity, *variant));
    }
}

/// Update: every walk, in arrival order. A walk ends a build in flight; its
/// fixture stands solid.
#[allow(clippy::too_many_arguments)]
pub(super) fn walk(
    mut commands: Commands,
    mut requests: MessageReader<crate::companion::CompanionCommand>,
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    mut ghosts: ResMut<FixtureGhosts>,
    builds: Query<(&build::BuildPlan, Option<&build::HeldTool>)>,
    mut companions: Bodies,
) {
    for command in requests.read() {
        let crate::companion::CompanionCommand::WalkTo { unit, to, within } = command else {
            continue;
        };
        let unit = *unit;
        let Some((entity, _, mut status, mut transform, mut walk, mut slot, mut route, mut phase)) =
            companions.iter_mut().find(|(_, body, ..)| body.unit == unit)
        else {
            not_spawned(command, unit);
            continue;
        };
        if status.state == CompanionState::Spawned {
            status.refuse("the body is still loading");
            error!(
                "[companion] unit {unit} walk {} refused: the body is still loading",
                status.seq
            );
            continue;
        }
        if let Ok((plan, held)) = builds.get(entity) {
            ghosts.clear(&plan.uid);
            if let Some(tool) = held {
                commands.entity(tool.0).despawn();
            }
            commands
                .entity(entity)
                .remove::<(build::BuildPlan, build::Hammering, build::HeldTool)>();
            info!("[companion] unit {unit}: build {} given up for a walk", plan.seq);
        }
        let Some((face, objective)) = crate::companion::faces(
            site.as_deref(),
            epoch.as_deref(),
            face.as_deref(),
            objective.as_deref(),
        ) else {
            status.refuse("no navigation on this site");
            error!("[companion] unit {unit} walk refused: no navigation on this site");
            continue;
        };
        let seq = status.accept();
        let within = within.clamp(0.0, WITHIN_LIMIT);
        info!(
            "[companion] unit {unit} walk {seq}: to {to:?} within {within:.2}m from {:?}",
            transform.translation
        );
        match crate::npc::companion::depart(
            unit,
            &mut walk,
            &mut slot,
            &mut route,
            face,
            objective,
            to.to_array(),
            within,
        ) {
            Ok(next) => {
                *phase = next;
                if let Some(turned) = turn_to(transform.translation, *to) {
                    transform.rotation = turned;
                }
                status.state = CompanionState::Walking;
                commands.entity(entity).insert(Walker::started());
            }
            Err(reason) => {
                status.fail(reason);
                error!("[companion] unit {unit} walk {seq} refused: {reason}");
            }
        }
    }
}
