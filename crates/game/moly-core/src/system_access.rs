//! System and schedule errors that otherwise surface one at a time on the
//! first frames of the running game, reported all at once.
//!
//! Bevy rejects a system whose parameters conflict (two queries that may
//! borrow the same component mutably, or `Res<T>` beside `ResMut<T>`) when a
//! schedule first initializes it, and rejects a schedule whose graph cannot be
//! built (a cycle, or an ordering against a system added more than once).
//! Each of these halts the game at the first one. [`conflicts`] runs the
//! startup schedules, then initializes every system on its own and builds
//! every schedule, collecting each failure.
//!
//! A system whose resource is missing is not reported here: the engine checks
//! a system's parameters only as part of running it, so that error surfaces
//! when the system first runs.

use std::panic::{catch_unwind, AssertUnwindSafe};

use bevy::app::PluginsState;
use bevy::ecs::schedule::{ScheduleLabel, Schedules};
use bevy::prelude::*;

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_default()
}

/// Finishes building `app`, runs its startup schedules, then initializes
/// each system of each main-world schedule and builds each schedule. Returns
/// how many systems were initialized and one line per failure.
pub fn conflicts(app: &mut App) -> (usize, Vec<String>) {
    while app.plugins_state() == PluginsState::Adding {
        #[cfg(not(target_arch = "wasm32"))]
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
            let graph = schedule.graph_mut();
            let keys: Vec<_> = graph.systems.iter().map(|(key, _, _)| key).collect();
            for key in keys {
                let Some(node) = graph.systems.get_mut(key) else {
                    continue;
                };
                let name = node.name().to_string();
                checked += 1;
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| node.initialize(world))) {
                    rejected.push(format!("{label:?} {name}: {}", panic_message(&*panic)));
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
            Ok(Ok(_)) => {}
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
