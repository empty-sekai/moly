//! LoopParticleControlBehaviour is a Play/Pause owner. Runtime frames do not
//! continuously Simulate it; systems advance through their ordinary manager.
use super::*;
use crate::fixture_timeline_particles::{self as particles, ParticlePlayBinding, PlayedSystemInfo};
use moly_assets::scene_state::{SetSourceActive, SourceNodeActivity};

#[derive(Clone)]
pub struct LoopParticleBinding {
    target: Entity,
    played: Option<ParticlePlayBinding>,
    systems: Vec<PlayedSystemInfo>,
    included_loop: bool,
}

#[derive(Component)]
struct LoopParticleOwner(TimelineToken);

#[derive(Default)]
struct Behaviour {
    active: bool,
    has_played: bool,
    previous_local: Option<f32>,
}

#[derive(Default)]
pub(super) struct OwnedLoopParticles(HashMap<TimelineClipKey, Behaviour>);

fn resolve(world: &World, request: &StartTimeline, id: &SourceAssetId) -> Option<Entity> {
    let wanted = id.path_id.parse::<i64>().ok()?;
    let mut stack = vec![request.fixture];
    while let Some(node) = stack.pop() {
        if world
            .get::<moly_assets::source_navigation::SourceObjectIdentity>(node)
            .is_some_and(|source| source.file == id.file && source.game_object == wanted)
        {
            return Some(node);
        }
        if let Some(children) = world.get::<Children>(node) {
            stack.extend(children.iter());
        }
    }
    None
}

pub(super) fn prepare(
    world: &mut World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    let clips: Vec<_> = request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter(|clip| matches!(clip.payload, TimelinePayload::LoopParticle { .. }))
        .cloned()
        .collect();
    request
        .bindings
        .loop_particles
        .retain(|key, _| clips.iter().any(|clip| clip.key == *key));
    for clip in clips {
        if request.bindings.loop_particles.contains_key(&clip.key) {
            continue;
        }
        let (owner, target, package) =
            if let Some(bound) = request.bindings.step_effects.get(&clip.key) {
                if !valid_step_attachment(world, request, bound.owner)
                    || !descendant_of(world, bound.object, bound.owner)
                {
                    return Err(invalid("loop particle attachment identity differs"));
                }
                (bound.owner, Some(bound.object), bound.package.clone())
            } else {
                match clip.exposed_source.as_ref() {
                    Some(ExposedSource::Null) => {
                        (request.fixture, None, request.definition.package.clone())
                    }
                    Some(ExposedSource::Object(id)) => (
                        request.fixture,
                        Some(resolve(world, request, id).ok_or_else(|| {
                            invalid("loop particle target is not in its director hierarchy")
                        })?),
                        request.definition.package.clone(),
                    ),
                    Some(ExposedSource::Unreadable(reason)) => {
                        return Err(invalid(format!("loop particle reference: {reason}")))
                    }
                    None => return Err(invalid("loop particle exposed reference missing")),
                }
            };
        let Some(target) = target else {
            continue;
        };
        let systems = particles::played_object_systems(world, owner, target, &package)?;
        let included_loop = systems.iter().any(|system| system.looping);
        let played = systems
            .first()
            .map(|system| system.node)
            .map(|root| particles::prepare_played_object(world, owner, root, &package))
            .transpose()?;
        if let Some(played) = &played {
            // Play/Stop acts on the first particle's subtree. A source with
            // additional listed siblings needs independent system setters;
            // refuse it rather than silently omit their duration updates.
            if systems
                .iter()
                .any(|system| !descendant_of(world, system.node, played.root))
            {
                return Err(invalid(
                    "loop particle systems extend outside the first system subtree",
                ));
            }
        }
        request.bindings.loop_particles.insert(
            clip.key,
            LoopParticleBinding {
                target,
                played,
                systems,
                included_loop,
            },
        );
    }
    Ok(())
}

pub(super) fn validate(
    world: &World,
    request: &StartTimeline,
    own: Option<TimelineToken>,
    require_available: bool,
) -> Result<(), TimelineFailure> {
    let mut count = 0;
    for clip in request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
    {
        if !matches!(clip.payload, TimelinePayload::LoopParticle { .. }) {
            continue;
        }
        if matches!(clip.exposed_source, Some(ExposedSource::Null))
            && !request.bindings.step_effects.contains_key(&clip.key)
        {
            continue;
        }
        let binding = request
            .bindings
            .loop_particles
            .get(&clip.key)
            .ok_or_else(|| invalid("loop particle binding not prepared"))?;
        count += 1;
        let attachment = request
            .bindings
            .step_effects
            .get(&clip.key)
            .is_some_and(|bound| {
                valid_step_attachment(world, request, bound.owner)
                    && descendant_of(world, binding.target, bound.owner)
            });
        if world.get_entity(binding.target).is_err()
            || !(descendant_of(world, binding.target, request.fixture) || attachment)
            || world.get::<SourceNodeActivity>(binding.target).is_none()
        {
            return Err(invalid("loop particle target or activation disappeared"));
        }
        if clip.pre_extrapolation != 0 || clip.post_extrapolation != 0 || clip.clip_in < 0. {
            return Err(invalid("loop particle extrapolation unsupported"));
        }
        if let Some(played) = &binding.played {
            if world.get_entity(played.root).is_err()
                || binding
                    .systems
                    .iter()
                    .any(|s| world.get_entity(s.node).is_err())
            {
                return Err(invalid("loop particle system disappeared"));
            }
            if require_available
                && world
                    .get::<LoopParticleOwner>(played.root)
                    .is_some_and(|owner| Some(owner.0) != own)
            {
                return Err(invalid("loop particle belongs to another director"));
            }
            if request.bindings.controls.values().any(|control| {
                descendant_of(world, control.root, played.root)
                    || descendant_of(world, played.root, control.root)
            }) {
                return Err(invalid(
                    "loop particle overlaps a continuous particle control",
                ));
            }
            for (other_key, other) in &request.bindings.loop_particles {
                if *other_key == clip.key {
                    continue;
                }
                if other.played.as_ref().is_some_and(|other| {
                    descendant_of(world, other.root, played.root)
                        || descendant_of(world, played.root, other.root)
                }) {
                    let Some(other_clip) = request
                        .definition
                        .tracks
                        .iter()
                        .flat_map(|t| &t.clips)
                        .find(|c| c.key == *other_key)
                    else {
                        return Err(invalid("loop particle binding outside selected director"));
                    };
                    if clip.start < other_clip.end() && other_clip.start < clip.end() {
                        return Err(invalid(
                            "overlapping loop particle behaviours share a system",
                        ));
                    }
                }
            }
        }
    }
    if count != request.bindings.loop_particles.len() {
        return Err(invalid("loop particle bindings outside selected director"));
    }
    Ok(())
}

pub(super) fn claim(
    world: &mut World,
    token: TimelineToken,
    request: &StartTimeline,
    owned: &mut OwnedLoopParticles,
) {
    for (key, binding) in &request.bindings.loop_particles {
        if let Some(played) = &binding.played {
            world
                .entity_mut(played.root)
                .insert(LoopParticleOwner(token));
            particles::stop_object(world, played);
        }
        owned.0.insert(key.clone(), Behaviour::default());
    }
}

/// The time used only when a previously played behaviour enters again.
fn resume_time(local: f32, included_loop: bool, root_duration: f32) -> Option<f32> {
    let time = if included_loop && root_duration > 0. {
        local % root_duration
    } else {
        local
    };
    (time > 0.1).then_some(time)
}

fn entered_simulate_time(
    has_played: bool,
    local: f32,
    included_loop: bool,
    root_duration: f32,
) -> Option<f32> {
    has_played
        .then(|| resume_time(local, included_loop, root_duration))
        .flatten()
}

pub(super) fn sample(
    world: &mut World,
    token: TimelineToken,
    request: &StartTimeline,
    owned: &mut OwnedLoopParticles,
    time: f64,
    paused: bool,
) -> Result<(), TimelineFailure> {
    for clip in request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
    {
        let Some(binding) = request.bindings.loop_particles.get(&clip.key) else {
            continue;
        };
        let behaviour = owned
            .0
            .get_mut(&clip.key)
            .ok_or_else(|| invalid("loop particle graph was not started"))?;
        let Some(played) = &binding.played else {
            continue;
        };
        if world
            .get::<LoopParticleOwner>(played.root)
            .is_none_or(|owner| owner.0 != token)
        {
            return Err(invalid("loop particle ownership lost"));
        }
        let active = clip.contains(time) && !paused;
        let local = clip.sample_time(time, false).unwrap_or(0.) as f32;
        let looped = active
            && behaviour.active
            && behaviour
                .previous_local
                .is_some_and(|previous| local < previous);
        if behaviour.active && (!active || looped) {
            particles::stop_object(world, played);
            behaviour.active = false;
        }
        if active && !behaviour.active {
            if !world
                .get::<SourceNodeActivity>(binding.target)
                .is_none_or(SourceNodeActivity::active_self)
            {
                SetSourceActive {
                    entity: binding.target,
                    active: true,
                }
                .apply(world);
            }
            if !binding.included_loop {
                let nodes: Vec<_> = binding.systems.iter().map(|s| s.node).collect();
                particles::set_played_object_duration(
                    world,
                    played,
                    &nodes,
                    clip.duration as f32 - local,
                )
                .map_err(invalid)?;
            }
            if let Some(t) = entered_simulate_time(
                behaviour.has_played,
                local,
                binding.included_loop,
                binding.systems[0].duration,
            ) {
                particles::simulate_object(world, played, t).map_err(invalid)?;
            }
            particles::play_object(world, played).map_err(invalid)?;
            behaviour.has_played = true;
            behaviour.active = true;
        }
        behaviour.previous_local = active.then_some(local);
    }
    Ok(())
}

pub(super) fn release(
    world: &mut World,
    token: TimelineToken,
    request: &StartTimeline,
    owned: &mut OwnedLoopParticles,
) {
    for (key, _) in owned.0.drain() {
        if let Some(played) = request
            .bindings
            .loop_particles
            .get(&key)
            .and_then(|b| b.played.as_ref())
        {
            if world
                .get::<LoopParticleOwner>(played.root)
                .is_some_and(|owner| owner.0 == token)
            {
                particles::stop_object(world, played);
                world.entity_mut(played.root).remove::<LoopParticleOwner>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_simulates_only_above_threshold_and_wraps_only_looping_roots() {
        assert_eq!(resume_time(0.1, false, 1.), None);
        assert_eq!(resume_time(0.11, false, 1.), Some(0.11));
        assert_eq!(resume_time(2.05, true, 1.), None);
        assert_eq!(resume_time(2.25, true, 1.), Some(0.25));
        assert_eq!(resume_time(2.25, false, 1.), Some(2.25));
        assert_eq!(resume_time(2.25, true, 0.), Some(2.25));
        assert_eq!(entered_simulate_time(false, 2.25, false, 1.), None);
        assert_eq!(entered_simulate_time(true, 2.25, false, 1.), Some(2.25));
    }
}
