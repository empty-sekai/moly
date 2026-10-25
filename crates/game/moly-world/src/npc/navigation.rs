//! Persistent agent switches and direct destinations, independent of route resets.
use super::{CrowdMotion, RouteStops, WalkState};
use crate::npc_objective::ObjectiveFace;
use bevy::prelude::*;

#[derive(Component, Clone, Copy)]
pub(crate) struct NavigationAgent {
    pub enabled: bool,
    pub stopping_distance: f32,
    binding_generation: Option<u64>,
}
impl Default for NavigationAgent {
    fn default() -> Self {
        Self {
            enabled: true,
            stopping_distance: moly_law::path::crowd::NPC_STOPPING_DISTANCE,
            binding_generation: None,
        }
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct NavigationAgents<'w, 's> {
    agents: Query<'w, 's, &'static mut NavigationAgent>,
}
impl NavigationAgents<'_, '_> {
    pub fn enabled(&self, actor: Entity) -> bool {
        self.agents.get(actor).is_ok_and(|agent| agent.enabled)
    }
    pub fn reconnect(
        &mut self,
        actor: Entity,
        face: &ObjectiveFace,
        at: [f32; 3],
    ) -> Option<[f32; 3]> {
        let mut agent = self.agents.get_mut(actor).ok()?;
        if !agent.enabled {
            return None;
        }
        if agent.binding_generation == Some(face.navigation_generation())
            && face.walk_face().field.on_cell([at[0], at[2]])
        {
            return Some(at);
        }
        let point = reconnect_point(face, at)?;
        agent.binding_generation = Some(face.navigation_generation());
        Some(point)
    }
    pub fn stopping_distance(&self, actor: Entity) -> f32 {
        self.agents
            .get(actor)
            .map_or(moly_law::path::crowd::NPC_STOPPING_DISTANCE, |agent| {
                agent.stopping_distance
            })
    }
}

pub(crate) fn face(world: &World, actor: Entity) -> Option<ObjectiveFace> {
    if super::is_away(world, actor) {
        let own = &world.get::<super::NpcActions>(actor)?.site_type;
        world
            .get_resource::<super::residency::SiteNavigation>()?
            .get(own)
            .map(|site| site.objective.clone())
    } else {
        world.get_resource::<ObjectiveFace>().cloned()
    }
}

pub(crate) fn exists(world: &World, actor: Entity) -> bool {
    world.get::<NavigationAgent>(actor).is_some()
        && world.get::<crate::character::MotionDriver>(actor).is_some()
}
pub(crate) fn enabled(world: &World, actor: Entity) -> bool {
    exists(world, actor)
        && world
            .get::<NavigationAgent>(actor)
            .is_some_and(|agent| agent.enabled)
}
pub(crate) fn active(world: &World, actor: Entity) -> bool {
    enabled(world, actor)
        && world
            .get::<moly_assets::scene_state::SourceInactive>(actor)
            .is_none()
}
pub(crate) fn on_mesh(world: &World, actor: Entity) -> bool {
    if !active(world, actor) {
        return false;
    }
    let Some(at) = world.get::<Transform>(actor).map(|pose| pose.translation) else {
        return false;
    };
    face(world, actor).is_some_and(|face| face.walk_face().field.on_cell([at.x, at.z]))
}

/// Behaviour.enabled is distinct from isStopped, path presence, and model visibility.
fn reconnect_point(face: &ObjectiveFace, at: [f32; 3]) -> Option<[f32; 3]> {
    let [x, z] = face
        .walk_face()
        .relocate([at[0], at[2]], moly_law::path::crowd::NPC_AGENT_RADIUS)?;
    if [x, z] == [at[0], at[2]] {
        Some(at)
    } else {
        face.navigation_point_at([x, z])
    }
}

/// Enabling registers the view on the crowd's navigation cells. The engine's
/// reconnect query is separate from the caller's SamplePosition acceptance test.
pub(crate) fn set_enabled(world: &mut World, actor: Entity, enabled: bool) {
    let changed = if let Some(mut agent) = world.get_mut::<NavigationAgent>(actor) {
        let changed = agent.enabled != enabled;
        agent.enabled = enabled;
        if changed {
            agent.binding_generation = None;
        }
        changed
    } else {
        false
    };
    if enabled && changed {
        refresh_binding(world, actor);
    }
}
pub(crate) fn refresh_binding(world: &mut World, actor: Entity) {
    if !active(world, actor) {
        return;
    }
    let Some(face) = face(world, actor) else {
        return;
    };
    let Some(at) = world
        .get::<Transform>(actor)
        .map(|pose| pose.translation.to_array())
    else {
        return;
    };
    let generation = face.navigation_generation();
    if world
        .get::<NavigationAgent>(actor)
        .is_some_and(|agent| agent.binding_generation == Some(generation))
        && face.walk_face().field.on_cell([at[0], at[2]])
    {
        return;
    }
    let bound = reconnect_point(&face, at);
    if let Some(point) = bound {
        set_view_position(world, actor, Vec3::from(point));
    }
    if let Some(mut agent) = world.get_mut::<NavigationAgent>(actor) {
        agent.binding_generation = bound.map(|_| generation);
    }
}
pub(crate) fn stop(world: &mut World, actor: Entity, owner: u64) {
    super::stop_movement(world, actor);
    if let Some(mut route) = world.get_mut::<RouteStops>(actor) {
        route.agent_stopped = true;
    }
    reset(world, actor, owner);
}
pub(crate) fn stopping_distance(world: &World, actor: Entity) -> Option<f32> {
    world
        .get::<NavigationAgent>(actor)
        .map(|agent| agent.stopping_distance)
}
pub(crate) fn set_stopping_distance(world: &mut World, actor: Entity, distance: f32) {
    if let Some(mut agent) = world.get_mut::<NavigationAgent>(actor) {
        agent.stopping_distance = distance;
    }
}

#[derive(Component)]
struct DirectDestination {
    owner: u64,
    submitted_frame: u32,
    crowd: CrowdMotion,
}
#[derive(Clone, Copy)]
pub(crate) struct DirectStatus {
    pub pending: bool,
    pub has_path: bool,
    pub remaining: f32,
}

/// SetupMoveNavMesh + SetDestination. The route wrapper and its random waypoints
/// are not involved; a failed path is observed by the caller on its next poll.
pub(crate) fn set_destination(world: &mut World, actor: Entity, owner: u64, target: Vec3) {
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let at = world
        .get::<Transform>(actor)
        .map(|pose| pose.translation.to_array());
    let corridor = at.and_then(|at| {
        face(world, actor)?
            .walk_face()
            .field
            .corridor(at, None, target.to_array())
    });
    super::stop_movement(world, actor);
    if let Some(mut route) = world.get_mut::<RouteStops>(actor) {
        route.agent_stopped = false;
    }
    world.entity_mut(actor).insert(DirectDestination {
        owner,
        submitted_frame: frame,
        crowd: CrowdMotion {
            corridor,
            ..Default::default()
        },
    });
    // The synchronous corridor query has resolved the path; initialize the
    // remaining-distance readback without spending movement time.
    step(world, actor, owner, 0.0);
}

pub(crate) fn status(world: &World, actor: Entity, owner: u64) -> Option<DirectStatus> {
    let direct = world
        .get::<DirectDestination>(actor)
        .filter(|direct| direct.owner == owner)?;
    Some(DirectStatus {
        pending: direct.submitted_frame == world.resource::<bevy::diagnostic::FrameCount>().0,
        has_path: direct.crowd.corridor.is_some(),
        remaining: direct.crowd.remaining.unwrap_or(f32::INFINITY),
    })
}

/// Advances only the engine crowd for this direct destination. The approach
/// reads the previous frame's remainingDistance before it asks for this step.
pub(crate) fn step(world: &mut World, actor: Entity, owner: u64, dt: f32) {
    if !on_mesh(world, actor) {
        return;
    }
    let Some(face) = face(world, actor) else {
        return;
    };
    let stopping =
        stopping_distance(world, actor).unwrap_or(moly_law::path::crowd::NPC_STOPPING_DISTANCE);
    let Some(mut direct) = world.entity_mut(actor).take::<DirectDestination>() else {
        return;
    };
    if direct.owner != owner {
        world.entity_mut(actor).insert(direct);
        return;
    }
    let stopped = world
        .get::<RouteStops>(actor)
        .is_some_and(|route| route.agent_stopped);
    if stopped {
        direct.crowd.velocity = [0.0; 3];
    } else if let Some(speed) = world.get::<super::WalkSpeed>(actor).map(|speed| speed.0) {
        let mut query = world.query::<(&mut WalkState, &mut Transform)>();
        if let Ok((mut walk, mut pose)) = query.get_mut(world, actor) {
            if super::crowd_step(
                &mut walk.0,
                &mut direct.crowd,
                speed,
                stopping,
                &face.walk_face(),
                &face,
                &mut pose.rotation,
                dt,
            )
            .is_none()
            {
                direct.crowd.corridor = None;
            }
            pose.translation = Vec3::from(walk.0.position);
        }
    }
    world.entity_mut(actor).insert(direct);
}

/// ResetPath/velocity for this lease; an old lower cannot remove a newer move.
pub(crate) fn reset(world: &mut World, actor: Entity, owner: u64) {
    if world
        .get::<DirectDestination>(actor)
        .is_some_and(|direct| direct.owner == owner)
    {
        world.entity_mut(actor).remove::<DirectDestination>();
    }
}

/// The view Transform setter used while an agent is disabled. Unlike Warp it
/// never resets the route, navigation switch, motion phase, or MoveTarget.
pub(crate) fn set_view_position(world: &mut World, actor: Entity, at: Vec3) {
    if let Some(mut pose) = world.get_mut::<Transform>(actor) {
        pose.translation = at;
    }
    if let Some(mut walk) = world.get_mut::<WalkState>(actor) {
        walk.0.position = at.to_array();
    }
}
