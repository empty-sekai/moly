//! Scoped use of the existing source controller. A preview suspends exactly one
//! instance and restores its channels, never resets every fixture in the scene.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope { Library, Talk }

pub(super) struct Lease {
    pub(super) owner: Entity,
    scope: Scope,
    previous: Option<Instance>,
    emission: Vec<(Entity, Option<FixtureEmissionOverride>)>,
    rotation: Option<(Entity, Quat)>,
    stop_after: Option<f32>,
    one_shot: bool,
}

pub(crate) fn availability(world: &World, target: &FixtureTarget) -> Result<(), String> {
    prepare_binding(world, target).map(|_| ())
}

fn acquire(world: &mut World, target: &FixtureTarget, owner: Entity, scope: Scope) -> Result<(), String> {
    if let Some(lease) = world.resource::<Gimmicks>().leases.get(target) {
        return if lease.owner == owner { Ok(()) } else { Err("fixture controller is owned by another performance".into()) };
    }
    let Prepared { definition, rotation, outputs, one_shot, .. } = prepare_binding(world, target)?;
    let emission = outputs.iter().map(|entity| (*entity, world.get::<FixtureEmissionOverride>(*entity).copied())).collect();
    let rotation_snapshot = rotation.as_ref().and_then(|binding| super::rotation::snapshot(world, binding));
    let mut runtime = world.resource_mut::<Gimmicks>();
    let previous = runtime.instances.remove(target);
    runtime.instances.insert(target.clone(), Instance { on: false, definition, playback: None, rotation,
        pending: PendingTriggers::default(), outputs });
    runtime.leases.insert(target.clone(), Lease { owner, scope, previous, emission, rotation: rotation_snapshot,
        stop_after: None, one_shot });
    Ok(())
}

fn trigger(instance: &mut Instance, on: bool) {
    instance.on = on;
    if instance.rotation.is_some() {
        instance.pending.set(if on { Trigger::On } else { Trigger::Off });
    } else {
        let program = if on { instance.definition.on.clone() } else { instance.definition.off.clone() };
        instance.playback = Some(Playback::new(program, instance.outputs.clone(), None));
    }
}

/// Retains the normal player switch animation/input lease and the same authored
/// On/Off controller. The library observes the effect, not merely the 1s gesture.
pub(crate) fn start_preview(world: &mut World, target: &FixtureTarget, owner: Entity, generation: u64) -> Result<(), String> {
    acquire(world, target, owner, Scope::Library)?;
    if let Err(reason) = super::start(world, target, generation) {
        finish_owner(world, owner);
        return Err(reason);
    }
    info!("[fixture-gimmick] preview {:?} owns {}", owner, target.uid);
    Ok(())
}

/// Talk execution does not impersonate a player switch: the talk already owns
/// player controls, and only the admitted real fixture controller is triggered.
pub(crate) fn talk_trigger(world: &mut World, entity: Entity, owner: Entity, play: bool, delay: f32) -> Result<(), String> {
    if world.get_entity(owner).is_err() { return Ok(()); }
    let identity = world.get::<FixtureActivityIdentity>(entity).ok_or("admitted fixture disappeared")?;
    let target = FixtureTarget { entity, uid: identity.uid.clone() };
    if play { acquire(world, &target, owner, Scope::Talk)?; }
    let mut runtime = world.resource_mut::<Gimmicks>();
    let Some(lease) = runtime.leases.get_mut(&target).filter(|lease| lease.owner == owner) else {
        return Ok(()); // A stale delayed Stop cannot affect the replacement owner.
    };
    if !play && delay > 0. { lease.stop_after = Some(delay); return Ok(()); }
    lease.stop_after = None;
    if let Some(instance) = runtime.instances.get_mut(&target) { trigger(instance, play); }
    info!("[fixture-gimmick] talk {:?} {} trigger={}", owner, target.uid, if play { "On" } else { "Off" });
    Ok(())
}

pub(crate) fn blocks_replacement(runtime: &Gimmicks) -> bool {
    runtime.player.is_some() || !runtime.leases.is_empty()
}

pub(crate) fn lease_count(runtime: &Gimmicks) -> usize { runtime.leases.len() }

pub(crate) fn active(runtime: &Gimmicks, owner: Entity) -> bool {
    runtime.leases.values().any(|lease| lease.owner == owner)
}

pub(crate) fn finish_owner(world: &mut World, owner: Entity) {
    crate::audio::dispose_scoped_se(world, owner);
    world.resource_scope(|world, mut runtime: Mut<Gimmicks>| {
        let targets: Vec<_> = runtime.leases.iter().filter(|(_, lease)| lease.owner == owner).map(|(target, _)| target.clone()).collect();
        if runtime.player.as_ref().is_some_and(|player| targets.contains(&player.target)) {
            if let Some(player) = runtime.player.take() { super::finish(world, player); }
        }
        for target in targets {
            let Some(lease) = runtime.leases.remove(&target) else { continue; };
            runtime.instances.remove(&target);
            if !world.get::<FixtureActivityIdentity>(target.entity).is_some_and(|identity| target.matches(identity)) { continue; }
            for (entity, prior) in lease.emission {
                if world.get::<FixtureEmission>(entity).is_none() { continue; }
                if let Some(mode) = prior { world.entity_mut(entity).insert(mode); }
                else { world.entity_mut(entity).remove::<FixtureEmissionOverride>(); }
            }
            if let Some((entity, rotation)) = lease.rotation {
                if let Some(mut transform) = world.get_mut::<Transform>(entity) { transform.rotation = rotation; }
            }
            if let Some(previous) = lease.previous { runtime.instances.insert(target.clone(), previous); }
            info!("[fixture-gimmick] owner {:?} restored {}", owner, target.uid);
        }
    });
}

pub(crate) fn cancel_previews(world: &mut World) {
    let owners: Vec<_> = world.resource::<Gimmicks>().leases.values().filter(|lease| lease.scope == Scope::Library).map(|lease| lease.owner).collect();
    for owner in owners { finish_owner(world, owner); }
}

/// Scheduled around the normal controller evaluation. Delayed stops remain
/// owned, and one-shot completion is its source callback plus Off playback.
pub(super) fn advance(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    let mut completed = Vec::new();
    world.resource_scope(|world, mut runtime: Mut<Gimmicks>| {
        let runtime = &mut *runtime;
        for (target, lease) in &mut runtime.leases {
            if world.get_entity(lease.owner).is_err()
                || !world.get::<FixtureActivityIdentity>(target.entity).is_some_and(|identity| target.matches(identity)) {
                completed.push(lease.owner); continue;
            }
            let Some(instance) = runtime.instances.get_mut(target) else { completed.push(lease.owner); continue; };
            if let Some(remaining) = &mut lease.stop_after {
                *remaining -= delta;
                if *remaining <= 0. { trigger(instance, false); lease.stop_after = None; }
            }
            if lease.scope == Scope::Library && lease.one_shot && !instance.on && !instance.pending.off
                && instance.playback.as_ref().is_none_or(|playback| playback.elapsed >= playback.program.duration) {
                completed.push(lease.owner);
            }
        }
    });
    for owner in completed { finish_owner(world, owner); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_owner_cannot_release_a_replacement() {
        let mut world = World::new(); world.init_resource::<Gimmicks>();
        let old = world.spawn_empty().id(); let current = world.spawn_empty().id();
        let target = FixtureTarget { entity: world.spawn_empty().id(), uid: "fixture-a".into() };
        world.resource_mut::<Gimmicks>().leases.insert(target, Lease { owner: current, scope: Scope::Talk,
            previous: None, emission: vec![], rotation: None, stop_after: Some(3.), one_shot: false });
        finish_owner(&mut world, old);
        assert!(active(world.resource::<Gimmicks>(), current));
        assert!(!active(world.resource::<Gimmicks>(), old));
    }
    #[test]
    fn cancelling_library_does_not_cancel_talk_leases() {
        let mut world = World::new(); world.init_resource::<Gimmicks>();
        let owner = world.spawn_empty().id(); let target = FixtureTarget { entity: world.spawn_empty().id(), uid: "talk-fixture".into() };
        world.resource_mut::<Gimmicks>().leases.insert(target, Lease { owner, scope: Scope::Talk,
            previous: None, emission: vec![], rotation: None, stop_after: None, one_shot: false });
        cancel_previews(&mut world); assert!(active(world.resource::<Gimmicks>(), owner));
    }
}
