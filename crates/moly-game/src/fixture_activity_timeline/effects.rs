//! Source-bound ControlPlayableAsset ownership. The particle adapter resolves
//! the actual fixture instance and its capabilities; this owner supplies the
//! Director clock, activation, exclusion and post-playback restoration.
use super::*;
use crate::fixture_timeline_particles::{self as particles, ParticleControlBinding};
use moly_assets::scene_state::{SetSourceActive, SourceNodeActivity};

#[derive(Component)]
struct EffectOwner(TimelineToken);

struct OwnedEffect {
    binding: ParticleControlBinding,
    prior_active: bool,
    controls_active: bool,
    post_playback: u64,
    active_clip: Option<TimelineClipKey>,
}

#[derive(Default)]
pub(super) struct OwnedEffects(HashMap<Entity, OwnedEffect>);

fn source_binding<'a>(
    clip: &'a TimelineClip,
    settings: &ControlSettings,
) -> Result<&'a source::TimelineEffectBinding, TimelineFailure> {
    let binding = clip
        .effect_binding
        .as_ref()
        .ok_or_else(|| invalid("ControlPlayableAsset has no exact NPC view effect binding"))?;
    if clip.playable.is_none()
        || binding.playable != clip.playable
        || binding.exposed_name != settings.exposed_name
        || binding.bind_name.is_empty()
    {
        return Err(invalid(
            "ControlPlayableAsset playable/exposed binding identity differs",
        ));
    }
    Ok(binding)
}

pub(super) fn prepare(
    world: &mut World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    let selected: Vec<_> = request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter_map(|clip| match &clip.payload {
            TimelinePayload::Control(settings) => Some((clip.clone(), settings.clone())),
            _ => None,
        })
        .collect();
    let selected_keys: HashSet<_> = selected.iter().map(|(clip, _)| clip.key.clone()).collect();
    request
        .bindings
        .controls
        .retain(|key, _| selected_keys.contains(key));
    for (clip, settings) in selected {
        let binding = source_binding(&clip, &settings)?;
        let view = request
            .definition
            .fixture_view
            .as_ref()
            .ok_or_else(|| invalid("ControlPlayableAsset NPC view metadata is missing"))?;
        if view
            .effects
            .iter()
            .filter(|row| row.playable == clip.playable)
            .count()
            != 1
        {
            return Err(invalid("ControlPlayableAsset effect binding is duplicated"));
        }
        request.bindings.controls.insert(
            clip.key.clone(),
            particles::prepare(world, request.fixture, &binding.bind_name, &settings)?,
        );
    }
    Ok(())
}

fn related_owner(world: &World, root: Entity, own: Option<TimelineToken>) -> bool {
    let mut parent = Some(root);
    while let Some(entity) = parent {
        if world
            .get::<EffectOwner>(entity)
            .is_some_and(|owner| Some(owner.0) != own)
        {
            return true;
        }
        parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
    }
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world
            .get::<EffectOwner>(entity)
            .is_some_and(|owner| Some(owner.0) != own)
        {
            return true;
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    false
}

pub(super) fn validate(
    world: &World,
    request: &StartTimeline,
    own: Option<TimelineToken>,
    require_available: bool,
) -> Result<(), TimelineFailure> {
    let mut controls: Vec<(&TimelineClip, &ControlSettings, Entity)> = Vec::new();
    for clip in request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
    {
        let TimelinePayload::Control(settings) = &clip.payload else {
            continue;
        };
        source_binding(clip, settings)?;
        let binding = request
            .bindings
            .controls
            .get(&clip.key)
            .ok_or_else(|| invalid("source particle control is not prepared"))?;
        particles::validate(world, binding)?;
        if !descendant_of(world, binding.root, request.fixture)
            || world.get::<SourceNodeActivity>(binding.root).is_none()
        {
            return Err(invalid(
                "source effect is outside its fixture or lacks authored activation",
            ));
        }
        if require_available && related_owner(world, binding.root, own) {
            return Err(invalid("source effect already belongs to another Director"));
        }
        if clip.clip_in < 0.0 || clip.pre_extrapolation != 0 || clip.post_extrapolation != 0 {
            return Err(invalid(
                "particle control requires the authored non-extrapolated time interval",
            ));
        }
        for (other, other_settings, root) in &controls {
            if *root == binding.root {
                if control_conflict(clip, settings, other, other_settings) {
                    return Err(invalid(
                        "same effect has overlapping or conflicting source controls",
                    ));
                }
            } else if descendant_of(world, *root, binding.root)
                || descendant_of(world, binding.root, *root)
            {
                return Err(invalid(
                    "nested source controls need a shared particle ownership policy",
                ));
            }
        }
        controls.push((clip, settings, binding.root));
    }
    if controls.len() != request.bindings.controls.len() {
        return Err(invalid(
            "control bindings contain clips outside the selected Director",
        ));
    }
    Ok(())
}

fn control_conflict(
    a: &TimelineClip,
    a_settings: &ControlSettings,
    b: &TimelineClip,
    b_settings: &ControlSettings,
) -> bool {
    a.start < b.end() && b.start < a.end()
        || a_settings.active != b_settings.active
        || a_settings.post_playback != b_settings.post_playback
        || a_settings.update_particle != b_settings.update_particle
        || a_settings.update_director != b_settings.update_director
        || a_settings.update_itime_control != b_settings.update_itime_control
        || a_settings.search_hierarchy != b_settings.search_hierarchy
}

pub(super) fn claim(
    world: &mut World,
    token: TimelineToken,
    request: &StartTimeline,
    owned: &mut OwnedEffects,
) -> Result<(), TimelineFailure> {
    for clip in request
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
    {
        let TimelinePayload::Control(settings) = &clip.payload else {
            continue;
        };
        let binding = &request.bindings.controls[&clip.key];
        if owned.0.contains_key(&binding.root) {
            continue;
        }
        let prior_active = world
            .get::<SourceNodeActivity>(binding.root)
            .ok_or_else(|| invalid("source effect activation disappeared"))?
            .active_self();
        world.entity_mut(binding.root).insert(EffectOwner(token));
        owned.0.insert(
            binding.root,
            OwnedEffect {
                binding: binding.clone(),
                prior_active,
                controls_active: settings.active,
                post_playback: settings.post_playback,
                active_clip: None,
            },
        );
    }
    Ok(())
}

pub(super) fn sample(
    world: &mut World,
    token: TimelineToken,
    request: &StartTimeline,
    owned: &mut OwnedEffects,
    time: f64,
) -> Result<(), TimelineFailure> {
    for (root, effect) in &mut owned.0 {
        if world
            .get::<EffectOwner>(*root)
            .is_none_or(|owner| owner.0 != token)
        {
            return Err(invalid("source particle control ownership was lost"));
        }
        let active = request
            .definition
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|clip| {
                request
                    .bindings
                    .controls
                    .get(&clip.key)
                    .is_some_and(|b| b.root == *root)
                    && clip.contains(time)
            });
        if effect.controls_active {
            SetSourceActive {
                entity: *root,
                active: active.is_some(),
            }
            .apply(world);
        }
        let (local, seed, binding) = match active {
            Some(clip) => {
                let TimelinePayload::Control(settings) = &clip.payload else {
                    unreachable!();
                };
                (
                    clip.sample_time(time, false),
                    settings.random_seed,
                    &request.bindings.controls[&clip.key],
                )
            }
            None => (None, 1, &effect.binding),
        };
        // OnBehaviourPlay belongs to the playable identity, not merely to its
        // local clock. A large frame can skip the gap between two controls on
        // one root while their local times remain monotonic (coffee's pot).
        let active_key = active.map(|clip| clip.key.clone());
        if active_key != effect.active_clip {
            particles::sample(world, &effect.binding, None, 1)?;
            effect.active_clip = active_key;
        }
        particles::sample(world, binding, local, seed)?;
    }
    Ok(())
}

pub(super) fn release(world: &mut World, token: TimelineToken, owned: &mut OwnedEffects) {
    for (root, effect) in owned.0.drain() {
        if world
            .get::<EffectOwner>(root)
            .is_none_or(|owner| owner.0 != token)
        {
            continue;
        }
        particles::release(world, &effect.binding);
        if effect.controls_active {
            SetSourceActive {
                entity: root,
                active: restored_active(effect.post_playback, effect.prior_active),
            }
            .apply(world);
        }
        world.entity_mut(root).remove::<EffectOwner>();
    }
}

fn restored_active(post_playback: u64, prior: bool) -> bool {
    match post_playback {
        0 => true,
        1 => false,
        _ => prior,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn post_playback_preserves_authored_active_inactive_and_revert() {
        for prior in [true, false] {
            assert!(restored_active(0, prior));
            assert!(!restored_active(1, prior));
            assert_eq!(restored_active(2, prior), prior);
        }
    }

    #[test]
    fn nested_directors_cannot_steal_each_others_effect_subtree() {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let child = world.spawn(ChildOf(root)).id();
        let token = TimelineToken {
            serial: 1,
            owner: FixtureActivityOwner {
                actor: root,
                generation: 1,
            },
        };
        world.entity_mut(child).insert(EffectOwner(token));
        assert!(related_owner(&world, root, None));
        assert!(!related_owner(&world, root, Some(token)));
        world.entity_mut(child).remove::<EffectOwner>();
        world.entity_mut(root).insert(EffectOwner(token));
        assert!(related_owner(&world, child, None));
    }
}
