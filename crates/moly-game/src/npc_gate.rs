//! The gate controller's random appearance of the visiting characters, once
//! per scene setup (see [`moly_law::objective::appearance`]).
//!
//! Every visiting NPC is created on the entry-site objective, hidden. When
//! every one of them runs it, the controller draws the gathering and places
//! the members one after another on the home site: each placement is the
//! agent's warp to the position, the show, and the cancel of the entry-site
//! objective with its ForceUpdateObjective (a reset and a new talk data);
//! a member at a random cell also raises the flag that skips its next Rest.
//!
//! Frames follow the controller's awaits: a member placed at a random cell
//! waits one frame for its character (the pick is made then) and one more
//! for its initialization (the placement); a member placed near the first
//! one computes its position on the frame it starts, waits the same two
//! frames, then yields one frame before the next member starts. The first
//! member starts on the frame every NPC runs its entry-site objective (the
//! frames of the NPC creation before it are not modelled).
//!
//! Engine draws come from one engine generator (the range of the gathering
//! size and each random-cell pick); the gathering's order and the near
//! search's order take fresh keys from a separate generator (the source
//! orders by new `Guid`s, not by the engine generator).
//!
//! Every step writes one `[npc-gate] {json}` line with the generator state
//! before its draw, so a replay can recompute each value.
//!
//! Not modelled: the room notification and the first-visit callback after
//! the placements, the player entry's wait for every NPC to be reported
//! appeared, the return-from-another-room flag (the panel does not carry
//! it, so the greeting stays owed), the show's teleport check (its view flag
//! is not read), and a roster created away from the home site (it keeps
//! empty talk data and is not placed here).

use std::collections::HashSet;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_law::objective::{self, appearance, Cell, ObjectiveType};
use serde_json::json;

use crate::client_config::{
    ClientConfigs, KEY_CHARACTER_COMMUNICATION_DISTANCE, KEY_CHARACTER_OVERLAP_DISTANCE,
};
use crate::npc::{CharacterUnitId, WalkState};
use crate::npc_objective::{
    cell_of, owe_force_updates, platform_seed, BodyWait, MemberRng, ObjectiveFace, ObjectiveMind,
};

/// Which part of the plan a member belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    First,
    Near,
    Rest,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::First => "first",
            Kind::Near => "near",
            Kind::Rest => "rest",
        }
    }
}

/// The current member's step.
#[derive(Debug, Clone, Copy)]
enum Phase {
    /// The member's step starts on `at`.
    Start { at: u32 },
    /// The random-cell pick on `at`; `fallback` when the near search found
    /// none.
    Pick { at: u32, fallback: bool },
    /// The placement on `at`.
    Place {
        at: u32,
        position: [f32; 3],
        immediate: bool,
    },
}

struct Run {
    rand: appearance::EngineRand,
    keys: MemberRng,
    members: Vec<(u32, Kind)>,
    index: usize,
    phase: Phase,
    first_position: Option<[f32; 3]>,
}

#[derive(Default)]
enum State {
    #[default]
    Waiting,
    Running(Box<Run>),
    Done,
}

/// The appearance's progress (once per app run: a scene is set up once).
#[derive(Resource, Default)]
pub(crate) struct GateAppearance {
    state: State,
}

fn emit(value: serde_json::Value) {
    info!("[npc-gate] {value}");
}

fn engine_state() -> [u32; 4] {
    let a = platform_seed();
    let b = platform_seed();
    let state = [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32];
    if state == [0; 4] {
        [1, 0, 0, 0]
    } else {
        state
    }
}

/// A uniform order of `len` items: one fresh key each, sorted.
fn guid_order(keys: &mut MemberRng, len: usize) -> Vec<usize> {
    let mut keyed: Vec<(u64, usize)> = (0..len).map(|index| (keys.next(), index)).collect();
    keyed.sort_by_key(|&(key, _)| key);
    keyed.into_iter().map(|(_, index)| index).collect()
}

struct GuidPermute<'a> {
    keys: &'a mut MemberRng,
    count: usize,
}

impl objective::Permute for GuidPermute<'_> {
    fn permutation(&mut self, len: usize) -> Vec<usize> {
        self.count += len;
        guid_order(self.keys, len)
    }
}

/// The NPC entity of a unit.
fn entity_of(world: &mut World, unit: u32) -> Option<Entity> {
    let mut query = world.query_filtered::<(Entity, &CharacterUnitId), With<ObjectiveMind>>();
    query
        .iter(world)
        .find(|(_, id)| id.0 == unit)
        .map(|(entity, _)| entity)
}

/// Update (after the AI loop): hide the characters that started the
/// entry-site objective on this frame, then run the appearance.
pub(crate) fn appear(world: &mut World) {
    let frame = world.resource::<FrameCount>().0;
    hide_entry_site_starts(world, frame);
    let state = std::mem::take(&mut world.resource_mut::<GateAppearance>().state);
    let state = match state {
        State::Waiting => match start(world, frame) {
            Some(run) => {
                let mut run = Box::new(run);
                if advance(world, &mut run, frame) {
                    State::Done
                } else {
                    State::Running(run)
                }
            }
            None => State::Waiting,
        },
        State::Running(mut run) => {
            if advance(world, &mut run, frame) {
                State::Done
            } else {
                State::Running(run)
            }
        }
        State::Done => State::Done,
    };
    world.resource_mut::<GateAppearance>().state = state;
}

/// The entry-site objective's start: hidden (the host has no cut-scene
/// game state).
fn hide_entry_site_starts(world: &mut World, frame: u32) {
    let mut query = world.query::<(&CharacterUnitId, &ObjectiveMind, &mut Visibility)>();
    for (unit, mind, mut visibility) in query.iter_mut(world) {
        if mind.body == Some(BodyWait::EntrySite { since: frame }) {
            *visibility = Visibility::Hidden;
            info!(
                "[npc unit={}] frame={frame} entry-site objective: Hide",
                unit.0
            );
        }
    }
}

/// The start: every visiting unit exists and runs its entry-site objective.
fn start(world: &mut World, frame: u32) -> Option<Run> {
    world.get_resource::<crate::npc::Spawned>()?;
    // The roster: the visiting units in the panel's expansion order (the
    // host's full-catalogue showcase is every catalogue unit instead).
    let units: Vec<u32> = world
        .get_resource::<crate::npc::Registry>()?
        .character_unit_ids
        .clone();
    if units.is_empty() {
        return None;
    }
    let mut query = world.query::<(&CharacterUnitId, &ObjectiveMind)>();
    let running: HashSet<u32> = query
        .iter(world)
        .filter(|(_, mind)| {
            mind.current == Some(ObjectiveType::EntrySite)
                && matches!(mind.body, Some(BodyWait::EntrySite { .. }))
        })
        .map(|(unit, _)| unit.0)
        .collect();
    if !units.iter().all(|unit| running.contains(unit)) {
        return None;
    }
    let mut rand = appearance::EngineRand::from_state(engine_state());
    let mut keys = MemberRng::from_platform();
    let before = rand.state;
    let max = (units.len() as i32).wrapping_add(1);
    let take = rand.range_int(appearance::GATHER_MIN, max);
    let order = guid_order(&mut keys, units.len());
    let Some(plan) = appearance::gather_plan(&units, take, &order) else {
        error!("[npc-gate] frame={frame} the gathering is empty: its minimum raises and no character appears");
        return None;
    };
    let walkable: Vec<Cell> = world
        .get_resource::<ObjectiveFace>()
        .map(|face| face.walkable().to_vec())
        .unwrap_or_default();
    emit(json!({
        "step": "start",
        "frame": frame,
        "units": units,
        "walkable": walkable,
        "range": [appearance::GATHER_MIN, max],
        "state": before,
        "take": take,
        "state_after": rand.state,
        "guid_keys": units.len(),
        "order": order,
        "gather": plan.gather,
        "first": plan.first,
        "near": plan.near,
        "rest": plan.rest,
    }));
    let mut members = vec![(plan.first, Kind::First)];
    members.extend(plan.near.iter().map(|&unit| (unit, Kind::Near)));
    members.extend(plan.rest.iter().map(|&unit| (unit, Kind::Rest)));
    Some(Run {
        rand,
        keys,
        members,
        index: 0,
        phase: Phase::Start { at: frame },
        first_position: None,
    })
}

/// Runs every step due on `frame`; `true` when the last member is placed.
fn advance(world: &mut World, run: &mut Run, frame: u32) -> bool {
    loop {
        let Some(&(unit, kind)) = run.members.get(run.index) else {
            return true;
        };
        match run.phase {
            Phase::Start { at } => {
                if at > frame {
                    return false;
                }
                if kind == Kind::Near {
                    match near_position(world, run, unit, frame) {
                        Some(position) => {
                            run.phase = Phase::Place {
                                at: frame + 2,
                                position,
                                immediate: false,
                            };
                        }
                        None => {
                            run.phase = Phase::Pick {
                                at: frame + 1,
                                fallback: true,
                            };
                        }
                    }
                } else {
                    run.phase = Phase::Pick {
                        at: frame + 1,
                        fallback: false,
                    };
                }
            }
            Phase::Pick { at, fallback } => {
                if at > frame {
                    return false;
                }
                let position = random_position(world, run, unit, kind, fallback, frame);
                run.phase = Phase::Place {
                    at: frame + 1,
                    position,
                    immediate: true,
                };
            }
            Phase::Place {
                at,
                position,
                immediate,
            } => {
                if at > frame {
                    return false;
                }
                let placed = place(world, unit, kind, position, immediate, frame);
                if kind == Kind::First {
                    let Some(placed) = placed else {
                        error!(
                            "[npc-gate] frame={frame} unit={unit}: the first character is missing; reading its position raises and the rest do not appear"
                        );
                        return true;
                    };
                    run.first_position = Some(placed);
                }
                run.index += 1;
                // A member placed near the first one yields one frame.
                let next = if kind == Kind::Near { frame + 1 } else { frame };
                run.phase = Phase::Start { at: next };
            }
        }
    }
}

/// GetRandomPosition over the rebuilt walkable list: one engine pick, the
/// picked cell's corner; an empty list gives zero without a draw.
fn random_position(
    world: &mut World,
    run: &mut Run,
    unit: u32,
    kind: Kind,
    fallback: bool,
    frame: u32,
) -> [f32; 3] {
    let Some(face) = world.get_resource::<ObjectiveFace>() else {
        emit(
            json!({"step": "pick", "frame": frame, "unit": unit, "kind": kind.word(), "refused": "no walk face"}),
        );
        return [0.0; 3];
    };
    let walkable = face.walkable();
    let before = run.rand.state;
    let (pick, cell) = if walkable.is_empty() {
        (None, None)
    } else {
        let index = run.rand.range_int(0, walkable.len() as i32);
        (Some(index), walkable.get(index as usize).copied())
    };
    let position = cell.map(appearance::field_position).unwrap_or([0.0; 3]);
    emit(json!({
        "step": "pick",
        "frame": frame,
        "unit": unit,
        "kind": kind.word(),
        "fallback": fallback,
        "walkable": walkable.len(),
        "state": before,
        "range": [0, walkable.len()],
        "pick": pick,
        "state_after": run.rand.state,
        "cell": cell,
        "position": position,
    }));
    position
}

/// GetNotPlacedFloorInTargetRange from the first one's position: the ring
/// radii from the panel distances, then the wander search over the walkable
/// cells minus the player's and the NPCs' cells. `None` when the answer is
/// (almost) zero.
fn near_position(world: &mut World, run: &mut Run, unit: u32, frame: u32) -> Option<[f32; 3]> {
    let first = run.first_position.unwrap_or([0.0; 3]);
    let (overlap, communication) = {
        let configs = world.resource::<ClientConfigs>();
        (
            configs.float(KEY_CHARACTER_OVERLAP_DISTANCE),
            configs.float(KEY_CHARACTER_COMMUNICATION_DISTANCE),
        )
    };
    let min_distance = overlap + objective::TILE_SCALE;
    // The site renders at the origin, so a position here is already
    // relative to the home site's origin.
    let (grid, min_cells, max_cells) =
        appearance::target_range_cells(first, [0, 0, 0], min_distance, communication);
    let origin: Cell = (grid[0], grid[2]);
    let mut occupied: Vec<Cell> = Vec::new();
    {
        let mut npcs = world.query_filtered::<&WalkState, With<ObjectiveMind>>();
        for walk in npcs.iter(world) {
            occupied.push(cell_of(walk.0.position[0], walk.0.position[2]));
        }
        let mut players =
            world.query_filtered::<&Transform, With<crate::player::PlayerControlled>>();
        for transform in players.iter(world) {
            occupied.push(cell_of(transform.translation.x, transform.translation.z));
        }
    }
    let face = world.get_resource::<ObjectiveFace>()?;
    let eligible: Vec<Cell> = face
        .walkable()
        .iter()
        .copied()
        .filter(|cell| !occupied.contains(cell))
        .collect();
    let ring: Vec<Cell> = eligible
        .iter()
        .copied()
        .filter(|&cell| objective::ring_filter(origin, cell, min_cells, max_cells))
        .collect();
    let mut permute = GuidPermute {
        keys: &mut run.keys,
        count: 0,
    };
    let mut probe = face.probe();
    let found = objective::wander_target(
        origin,
        min_cells,
        max_cells,
        &eligible,
        |cell| face.world_of(cell),
        &mut permute,
        &mut probe,
    );
    let answer = appearance::not_placed_floor_answer(found);
    let none = appearance::answer_is_none(answer);
    emit(json!({
        "step": "near",
        "frame": frame,
        "unit": unit,
        "first_position": first,
        "min_distance": min_distance,
        "max_distance": communication,
        "grid": grid,
        "min_cells": min_cells,
        "max_cells": max_cells,
        "occupied": occupied,
        "eligible": eligible,
        "ring": ring,
        "keys": permute.count,
        "answer": answer,
        "none": none,
    }));
    (!none).then_some(answer)
}

/// ForceSetPosition (the agent's warp), Show, then the cancel of the
/// current objective: when it reports true, ForceUpdateObjective (owed to
/// the next AI pass; the objective ends on the next frame), and for a
/// random-cell member SetImmediatelyExecuteNextObjective. Returns the
/// placed position.
fn place(
    world: &mut World,
    unit: u32,
    kind: Kind,
    position: [f32; 3],
    immediate: bool,
    frame: u32,
) -> Option<[f32; 3]> {
    let Some(entity) = entity_of(world, unit) else {
        emit(
            json!({"step": "place", "frame": frame, "unit": unit, "kind": kind.word(), "missing": true}),
        );
        return None;
    };
    // Warp: the position mapped onto the navigation cells within the
    // agent's query box, at the navigation surface's height. A position
    // that maps nowhere leaves the agent where it is.
    let warped = world
        .get_resource::<crate::walk_face::WalkFace>()
        .and_then(|walk_face| {
            walk_face.sample(
                [position[0], position[2]],
                moly_law::carve::AGENT_QUERY_HALF_EXTENT,
            )
        })
        .and_then(|xz| {
            world
                .get_resource::<ObjectiveFace>()
                .and_then(|face| face.navigation_point_at(xz))
        });
    if let Some(at) = warped {
        crate::npc::force_set_position(world, entity, at);
    } else {
        warn!("[npc-gate] frame={frame} unit={unit}: the warp to {position:?} maps onto no navigation cell; the agent stays");
    }
    if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
        *visibility = Visibility::Inherited;
    }
    let cancelled = world
        .get::<ObjectiveMind>(entity)
        .is_some_and(ObjectiveMind::cancel_reports);
    if cancelled {
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
            owe_force_updates(&mut mind, frame.wrapping_add(1), 1);
            if immediate {
                mind.skip_next_rest = true;
            }
        }
    }
    let placed = world.get::<WalkState>(entity).map(|walk| walk.0.position);
    emit(json!({
        "step": "place",
        "frame": frame,
        "unit": unit,
        "kind": kind.word(),
        "position": position,
        "warped": warped,
        "placed": placed,
        "cancelled": cancelled,
        "immediate": cancelled && immediate,
    }));
    placed
}
