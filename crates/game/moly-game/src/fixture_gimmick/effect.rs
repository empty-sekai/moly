//! A FixtureView's `ShowFixtureEffect` and `HideFixtureEffect` animation
//! events.
//!
//! `FindOrCacheEffect` takes the first Transform whose name is the event's
//! string among `GetComponentsInChildren<Transform>(includeInactive: true)`
//! of the FixtureView (the view's own object first, then its hierarchy depth
//! first) and caches that object with its own ParticleSystem. A name it does
//! not find is logged and the call does nothing.
//!
//! Show: when the object's own system exists and is stopped,
//! `SetActive(true)` and `Play()` (children included); otherwise an inactive
//! object is activated. Hide: when the object carries no system of its own
//! or it is stopped, `SetActive(false)`; otherwise `Stop(withChildren: true,
//! StopEmittingAndClear)`.
//!
//! The objects these events name carry, as their own system, one with no
//! Emission module, and the emitting systems below it. That system draws
//! nothing and the particle host prepares no draw for it, but its play state
//! chooses the branch, so it keeps a clock here (see [`IdleSystem`]).
//!
//! An object's systems are prepared on the first call that names it; until
//! its particle document and GPU preparation are ready, its calls wait in
//! order.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use moly_assets::{particle_source::ParticleSourceModules, scene_state::{SourceInactive, SourceNodeActivity}};
use serde_json::Value;

use crate::fixture_timeline_particles::{self as particles, ParticlePlayBinding};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Call {
    Show,
    Hide,
}

impl Call {
    pub(super) fn function(self) -> &'static str {
        match self {
            Self::Show => "ShowFixtureEffect",
            Self::Hide => "HideFixtureEffect",
        }
    }
}

/// One FixtureView's effect cache and its waiting calls.
#[derive(Default)]
pub(super) struct Effects {
    found: HashMap<String, Object>,
    calls: VecDeque<(Call, String)>,
}

struct Object {
    binding: ParticlePlayBinding,
    own: Own,
}

/// The object's own ParticleSystem.
enum Own {
    /// `GetComponent<ParticleSystem>()` found none.
    Absent,
    /// The particle host simulates it; its play state is the host's.
    Simulated,
    Idle(IdleSystem),
}

/// The play state of an object's own system that has no Emission module. Play
/// starts its system time; a non-looping system stops emitting on the first
/// update whose time has reached its duration and, holding no particle, its
/// play ends with that update. `Stop` with clear and deactivation end it at
/// once; a Play on a playing system restarts nothing.
struct IdleSystem {
    duration: f32,
    looping: bool,
    speed: f32,
    play_on_awake: bool,
    time: Option<f32>,
}

impl IdleSystem {
    fn from_record(record: &Value) -> Result<Self, String> {
        let number = |key: &str| record[key].as_f64().map(|value| value as f32)
            .ok_or_else(|| format!("own system record lacks {key}"));
        let flag = |key: &str| record[key].as_bool().ok_or_else(|| format!("own system record lacks {key}"));
        // Its clock reads Time.deltaTime and starts at Play; a start delay or
        // the unscaled clock would move where the stop falls.
        if flag("useUnscaledTime")? {
            return Err("own system without emission runs on the unscaled clock".into());
        }
        if record["startDelay"]["mode"].as_str() != Some("constant") || record["startDelay"]["value"].as_f64() != Some(0.0) {
            return Err("own system without emission has a start delay".into());
        }
        Ok(Self {
            duration: number("duration")?,
            looping: flag("looping")?,
            speed: number("simulationSpeed")?,
            play_on_awake: flag("playOnAwake")?,
            time: None,
        })
    }

    fn update(&mut self, dt: f32) {
        let Some(time) = self.time.as_mut() else { return };
        if !self.looping && *time >= self.duration {
            self.time = None;
        } else {
            *time += dt * self.speed;
        }
    }
}

impl Object {
    /// `Item2 != null && Item2.isStopped`, or `None` without a system.
    fn own_stopped(&self, world: &World) -> Option<bool> {
        match &self.own {
            Own::Absent => None,
            Own::Simulated => Some(!particles::object_system_playing(world, &self.binding).unwrap_or(false)),
            Own::Idle(idle) => Some(idle.time.is_none()),
        }
    }

    fn active_in_hierarchy(&self, world: &World) -> bool {
        world.get::<SourceInactive>(self.binding.root).is_none()
    }

    /// `SetActive(true)`: activation plays the play-on-awake systems that
    /// become active, the object's own included.
    fn activate(&mut self, world: &mut World) -> Result<(), String> {
        if particles::activate_played_object(world, &self.binding)?.is_some() && self.active_in_hierarchy(world) {
            if let Own::Idle(idle) = &mut self.own {
                if idle.play_on_awake && idle.time.is_none() {
                    idle.time = Some(0.0);
                }
            }
        }
        Ok(())
    }

    /// `Play()` with children: the own system first, then the systems below.
    fn play(&mut self, world: &mut World) -> Result<(), String> {
        if self.active_in_hierarchy(world) {
            if let Own::Idle(idle) = &mut self.own {
                if idle.time.is_none() {
                    idle.time = Some(0.0);
                }
            }
        }
        particles::play_systems(world, &self.binding).map(drop)
    }

    fn deactivate(&mut self, world: &mut World) -> Result<(), String> {
        let was_active = self.active_in_hierarchy(world);
        if particles::deactivate_played_object(world, &self.binding)?.is_some() && was_active {
            if let Own::Idle(idle) = &mut self.own {
                idle.time = None;
            }
        }
        Ok(())
    }

    fn stop_and_clear(&mut self, world: &mut World) {
        particles::stop_and_clear_object(world, &self.binding);
        if let Own::Idle(idle) = &mut self.own {
            idle.time = None;
        }
    }

    fn show(&mut self, world: &mut World) -> Result<(), String> {
        if self.own_stopped(world) == Some(true) {
            self.activate(world)?;
            self.play(world)
        } else if !world.get::<SourceNodeActivity>(self.binding.root).is_none_or(SourceNodeActivity::active_self) {
            self.activate(world)
        } else {
            Ok(())
        }
    }

    fn hide(&mut self, world: &mut World) -> Result<(), String> {
        match self.own_stopped(world) {
            None | Some(true) => self.deactivate(world),
            Some(false) => {
                self.stop_and_clear(world);
                Ok(())
            }
        }
    }
}

/// The first object named `name` in the view's hierarchy, depth first from
/// the view itself.
fn find(world: &World, view: Entity, name: &str) -> Option<Entity> {
    let mut stack = vec![view];
    while let Some(entity) = stack.pop() {
        if world.get::<Name>(entity).is_some_and(|found| found.as_str() == name) {
            return Some(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().rev());
        }
    }
    None
}

enum Lookup {
    Ready,
    Preparing,
    Absent,
    Refused(String),
}

impl Effects {
    pub(super) fn request(&mut self, call: Call, name: String) {
        self.calls.push_back((call, name));
    }

    fn lookup(&mut self, world: &mut World, fixture: Entity, view: Entity, name: &str) -> Lookup {
        if self.found.contains_key(name) {
            return Lookup::Ready;
        }
        let Some(object) = find(world, view, name) else { return Lookup::Absent };
        let (binding, record) = match particles::prepare_effect_object(world, fixture, object) {
            Ok(prepared) => prepared,
            Err(failure) if failure.retryable => return Lookup::Preparing,
            Err(failure) => return Lookup::Refused(failure.message),
        };
        let own = match record {
            None => Own::Absent,
            Some(record) => match ParticleSourceModules::from_system(&record) {
                Err(reason) => return Lookup::Refused(reason),
                Ok(modules) if modules.enabled.iter().any(|module| module == "EmissionModule") => {
                    if particles::object_system_playing(world, &binding).is_none() {
                        return Lookup::Refused("the object's own system emits but has no prepared draw".into());
                    }
                    Own::Simulated
                }
                Ok(_) => match IdleSystem::from_record(&record) {
                    Ok(idle) => Own::Idle(idle),
                    Err(reason) => return Lookup::Refused(reason),
                },
            },
        };
        self.found.insert(name.to_owned(), Object { binding, own });
        Lookup::Ready
    }

    /// Run the waiting calls in order, then this frame's update of the idle
    /// own systems (a call's Play is followed by the frame's particle update).
    pub(super) fn advance(&mut self, world: &mut World, fixture: Entity, view: Entity, uid: &str, dt: f32) {
        while let Some((call, name)) = self.calls.front().cloned() {
            match self.lookup(world, fixture, view, &name) {
                Lookup::Preparing => break,
                Lookup::Absent => error!("[fixture-gimmick] {uid} {}: no object named {name} under the FixtureView; the call does nothing", call.function()),
                Lookup::Refused(reason) => error!("[fixture-gimmick] {uid} {}({name}) refused: {reason}", call.function()),
                Lookup::Ready => {
                    let object = self.found.get_mut(&name).expect("looked up above");
                    let result = match call {
                        Call::Show => object.show(world),
                        Call::Hide => object.hide(world),
                    };
                    match result {
                        Ok(()) => info!("[fixture-gimmick] {uid} {}({name}) active={} own-stopped={:?}", call.function(),
                            object.active_in_hierarchy(world), object.own_stopped(world)),
                        Err(reason) => error!("[fixture-gimmick] {uid} {}({name}) refused: {reason}", call.function()),
                    }
                }
            }
            self.calls.pop_front();
        }
        if dt > 0.0 {
            for object in self.found.values_mut() {
                if let Own::Idle(idle) = &mut object.own {
                    idle.update(dt);
                }
            }
        }
    }
}
