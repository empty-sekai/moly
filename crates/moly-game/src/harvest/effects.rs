//! The harvest and home effects (`EffectManager` types 101 to 143, and 15
//! `Craft`), played from the `EffectManager` pools
//! ([`crate::site_move::effects::EffectPools`]), which the effect table's
//! rows build before any harvest or home action.
//!
//! `Emit(type, position, key)` takes the next copy of the type's pool ring
//! (its size is the table's pool size), puts it at the position and plays
//! its root particle system with its children (`ManagedEffect.Play`), even a
//! copy still playing. `HarvestObjectEffect` and `SiteMoveEffect` (the craft
//! prefab's) do not end themselves: a copy stays active after its systems
//! stop and is only replayed by a later emit that reaches it in the ring.
//! Emit poses, read off the views: the tree and stone hit effects (normal
//! and boost) are moved 0.2 m back toward the player and 0.35 m up and
//! turned to face the player (`LookRotation(-0.2 * dir)`); every other
//! harvest emit is at the view's position; 142 at the player's position; the
//! craft effect at the workbench, 0.22 m up.
//!
//! A kept copy (the workbench's craft effect) is held by its emitter's
//! ticket until `ManagedEffect.Stop`, which stops its emission; its live
//! particles finish. A later emit that reaches the same copy in the ring
//! plays it again while the first emitter still holds it, as the source's
//! shared reference does.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{EffectHook, HarvestEffectHooks};
use crate::site_move::effects::EffectPools;

#[derive(Resource, Default)]
pub(crate) struct HarvestEffects {
    /// Kept copies by the emitter's ticket.
    kept: HashMap<u64, Entity>,
    pub(crate) played: usize,
    pub(crate) skipped: usize,
}

/// Exclusive: emit the flow's requests into the pools and stop the kept
/// copies it asks to stop.
pub(crate) fn advance(world: &mut World) {
    let pending = std::mem::take(&mut world.resource_mut::<HarvestEffectHooks>().pending);
    let kept = std::mem::take(&mut world.resource_mut::<HarvestEffectHooks>().kept);
    let stops = std::mem::take(&mut world.resource_mut::<HarvestEffectHooks>().stops);
    world.resource_mut::<HarvestEffectHooks>().total += pending.len() + kept.len();
    if pending.is_empty() && kept.is_empty() && stops.is_empty() {
        return;
    }
    if !world.contains_resource::<EffectPools>() {
        warn!(
            "[harvest-effect] {} emits and {} stops with no effect pools: not drawn",
            pending.len() + kept.len(),
            stops.len()
        );
        world.resource_mut::<HarvestEffects>().skipped += pending.len() + kept.len();
        return;
    }
    world.resource_scope(|world, mut effects: Mut<HarvestEffects>| {
        world.resource_scope(|world, mut pools: Mut<EffectPools>| {
            for hook in pending {
                emit(&mut effects, &mut pools, world, hook);
            }
            for (ticket, hook) in kept {
                if let Some(root) = emit(&mut effects, &mut pools, world, hook) {
                    effects.kept.insert(ticket, root);
                }
            }
            for ticket in stops {
                stop(&mut effects, &mut pools, world, ticket);
            }
        });
    });
}

fn emit(
    effects: &mut HarvestEffects,
    pools: &mut EffectPools,
    world: &mut World,
    hook: EffectHook,
) -> Option<Entity> {
    let pose = Transform::from_translation(hook.position).with_rotation(hook.rotation);
    match pools.emit(world, hook.kind, pose) {
        Ok(emitted) => {
            effects.played += 1;
            let state = match emitted.since_prepared {
                Some(since) => format!(
                    "{} systems prepared {since:.2}s before (copy made {:.2}s before)",
                    emitted.systems, emitted.age
                ),
                None => format!(
                    "copy made {:.2}s before, not prepared yet: its systems start when prepared",
                    emitted.age
                ),
            };
            info!(
                "[harvest-effect] EffectManager.Emit({}) {} ({}, pool size {}) at ({:.2}, {:.2}, {:.2}) yaw {:.1} deg: copy {} of {} from its pool, play {}; {state}",
                hook.kind,
                emitted.name,
                emitted.prefab,
                emitted.pool_size,
                hook.position.x,
                hook.position.y,
                hook.position.z,
                hook.rotation.to_euler(EulerRot::YXZ).0.to_degrees(),
                emitted.slot + 1,
                emitted.pool_size,
                emitted.plays
            );
            Some(emitted.root)
        }
        Err(reason) => {
            effects.skipped += 1;
            warn!(
                "[harvest-effect] EffectManager.Emit({}) at {:.2}: {reason}; not drawn",
                hook.kind, hook.position
            );
            None
        }
    }
}

/// `ManagedEffect.Stop` on a kept copy: `ParticleSystem.Stop()` (children
/// included, stop emitting); its particles finish their lifetimes and the
/// copy stays in its pool.
fn stop(effects: &mut HarvestEffects, pools: &mut EffectPools, world: &mut World, ticket: u64) {
    let Some(root) = effects.kept.remove(&ticket) else {
        return;
    };
    let Some(stopped) = pools.stop(world, root) else {
        return;
    };
    info!(
        "[harvest-effect] ManagedEffect.Stop type {} {root:?}: emission stopped on {} systems{}; the copy stays in its pool",
        stopped.kind,
        stopped.systems,
        if stopped.prepared {
            ""
        } else {
            " (not prepared yet: the stop applies when it is)"
        }
    );
}
