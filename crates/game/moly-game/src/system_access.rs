//! System and schedule errors that otherwise surface one at a time on the
//! first frames of the running game, reported all at once.
//!
//! Bevy rejects a system whose parameters conflict (two queries that may
//! borrow the same component mutably, or `Res<T>` beside `ResMut<T>`) when a
//! schedule first initializes it, rejects a schedule whose graph cannot be
//! built (a cycle, or an ordering against a system added more than once), and
//! stops a system whose resource is missing when it first runs. Each of these
//! halts the game at the first one. [`conflicts`] runs the startup schedules,
//! then initializes every system on its own, checks the parameters of every
//! system that runs unconditionally, and builds every schedule, collecting
//! each failure.

use std::panic::{catch_unwind, AssertUnwindSafe};

use bevy::app::PluginsState;
use bevy::ecs::schedule::graph::Direction;
use bevy::ecs::schedule::{NodeId, ScheduleGraph, ScheduleLabel, Schedules, SystemKey};
use bevy::prelude::*;

/// The schedules whose systems run every frame from the first one; a missing
/// resource in one of their unconditional systems stops the game at once.
fn runs_every_frame(label: &str) -> bool {
    matches!(
        label,
        "First" | "PreUpdate" | "Update" | "PostUpdate" | "Last" | "FixedUpdate"
            | "FixedPreUpdate" | "FixedPostUpdate"
    )
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_default()
}

/// Whether the system or one of the sets that contain it carries a run
/// condition.
fn conditioned(graph: &ScheduleGraph, key: SystemKey) -> bool {
    if graph.systems.has_conditions(key) {
        return true;
    }
    let hierarchy = graph.hierarchy().graph();
    let mut stack = vec![NodeId::System(key)];
    let mut seen = Vec::new();
    while let Some(node) = stack.pop() {
        for parent in hierarchy.neighbors_directed(node, Direction::Incoming) {
            if seen.contains(&parent) {
                continue;
            }
            seen.push(parent);
            if let NodeId::Set(set) = parent {
                if graph.system_sets.has_conditions(set) {
                    return true;
                }
            }
            stack.push(parent);
        }
    }
    false
}

/// Finishes building `app`, runs its startup schedules, then initializes
/// each system of each main-world schedule, checks the parameters of the
/// unconditional systems of the per-frame schedules, and builds each
/// schedule. Returns how many systems were initialized and one line per
/// failure.
pub fn conflicts(app: &mut App) -> (usize, Vec<String>) {
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut rejected = Vec::new();
    let mut checked = 0usize;
    let world = app.world_mut();
    for label in [
        PreStartup.intern(),
        Startup.intern(),
        PostStartup.intern(),
    ] {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| {
            let _ = world.try_run_schedule(label);
        })) {
            rejected.push(format!("{label:?} run panicked: {}", panic_message(&*panic)));
        }
    }
    world.resource_scope(|world, mut schedules: Mut<Schedules>| {
        for (label, schedule) in schedules.iter_mut() {
            let every_frame = runs_every_frame(&format!("{label:?}"));
            let graph = schedule.graph_mut();
            let keys: Vec<_> = graph.systems.iter().map(|(key, _, _)| key).collect();
            for key in keys {
                let unconditional = every_frame && !conditioned(graph, key);
                let Some(node) = graph.systems.get_mut(key) else {
                    continue;
                };
                let name = node.system.name().to_string();
                checked += 1;
                if let Err(panic) =
                    catch_unwind(AssertUnwindSafe(|| node.system.initialize(world)))
                {
                    rejected.push(format!("{label:?} {name}: {}", panic_message(&*panic)));
                    continue;
                }
                if unconditional {
                    if let Err(error) = node.system.validate_param(world) {
                        if !error.skipped {
                            rejected.push(format!("{label:?} {name}: {error}"));
                        }
                    }
                }
            }
        }
    });
    let labels: Vec<_> = world
        .resource::<Schedules>()
        .iter()
        .map(|(_, schedule)| schedule.label())
        .collect();
    for label in labels {
        let Some(mut schedule) = world.resource_mut::<Schedules>().remove(label) else {
            continue;
        };
        match catch_unwind(AssertUnwindSafe(|| schedule.initialize(world))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => rejected.push(format!("{label:?} build: {error}")),
            Err(panic) => {
                rejected.push(format!("{label:?} build panicked: {}", panic_message(&*panic)))
            }
        }
        world.resource_mut::<Schedules>().insert(schedule);
    }
    std::panic::set_hook(previous_hook);
    (checked, rejected)
}
