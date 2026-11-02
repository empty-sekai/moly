//! `companion.build`: an owned fixture put near the companion (or at a
//! given point) through the field build's own put and save, then hammered by
//! the companion from a ghost to solid.

use bevy::prelude::*;
use moly_features::act::{targets, ActCtx, ActPoll, ActionRun};
use moly_features::companion::{build, CompanionCommand, CompanionState, CompanionStatus, FixtureGhosts};
use moly_features::field_build::{FieldBuildState, FieldBuilder, FieldPut};
use moly_law::fixture::Direction;

use crate::companion::BuildPlace;

/// How far in front of the companion a build's put search starts, metres.
const BUILD_AHEAD: f32 = 1.5;

/// A build in progress.
pub struct BuildRun {
    unit: u32,
    fixture: i32,
    texture: Option<u32>,
    place: BuildPlace,
    swings: u32,
    phase: BuildStep,
}

impl BuildRun {
    /// A build of `fixture` (of `texture`, or any) near the companion of
    /// `unit`, hammered `swings` times.
    pub fn new(unit: u32, fixture: i32, texture: Option<u32>, place: BuildPlace, swings: u32) -> Self {
        Self { unit, fixture, texture, place, swings, phase: BuildStep::Put }
    }
}

/// Where a `companion.build` step stands.
enum BuildStep {
    /// The fixture is not put yet.
    Put,
    /// The layout with the fixture `uid` is saving and reloading, as the
    /// field build `ticket`.
    Saving { ticket: u64, uid: String },
    /// The companion's build command, by its number, of the fixture `uid`.
    Building { seq: u64, uid: String },
}

impl ActionRun for BuildRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let (unit, fixture, texture, swings, place) = (self.unit, self.fixture, self.texture, self.swings, self.place);
        let Some(builder) = world.get_resource::<FieldBuilder>().copied() else {
            return ActPoll::Failed("the layout editor's field build is not installed".to_owned());
        };
        match &mut self.phase {
            BuildStep::Put => {
                let Some(stand) = targets::companion_place(world, unit) else {
                    return ActPoll::Failed(format!("no companion of unit {unit} is spawned"));
                };
                // One layout save is out at a time: a build whose turn has
                // not come waits for the save in flight (until the step's
                // timeout).
                if (builder.saving)(world) {
                    return ActPoll::Running(format!("companion {unit}: waiting for another build's save"));
                }
                // In front of the companion: its body faces its local +Z.
                let near = place.at.map(Vec3::from).unwrap_or(stand.translation + stand.rotation * Vec3::Z * BUILD_AHEAD);
                let put = FieldPut { at: near, exact: place.exact, direction: place.direction.and_then(Direction::from_u8) };
                match (builder.start)(world, fixture, texture, put) {
                    Err(reason) => ActPoll::Failed(format!("fixture {fixture} was not put: {reason}")),
                    Ok((ticket, uid)) => {
                        // Drawn as a ghost from its first frame.
                        world.resource_mut::<FixtureGhosts>().set(&uid, build::GHOST_ALPHA);
                        let note = format!("fixture {fixture} put as {uid} near {near:?}; saving");
                        self.phase = BuildStep::Saving { ticket, uid };
                        ActPoll::Running(note)
                    }
                }
            }
            BuildStep::Saving { ticket, uid } => match (builder.state)(world, *ticket) {
                None | Some(FieldBuildState::Saving) => ActPoll::Running(format!("fixture {uid}: saving the layout")),
                Some(FieldBuildState::Reloading) => ActPoll::Running(format!("fixture {uid}: the layout is reloading")),
                Some(FieldBuildState::Refused(reason)) => {
                    world.resource_mut::<FixtureGhosts>().clear(uid);
                    ActPoll::Failed(format!("fixture {uid}: {reason}"))
                }
                Some(FieldBuildState::Placed { .. }) => {
                    let Some(at) = instance_at(world, uid) else {
                        return ActPoll::Running(format!("fixture {uid}: waiting for its instance"));
                    };
                    let Some(seq) = targets::companion_seq(world, unit) else {
                        world.resource_mut::<FixtureGhosts>().clear(uid);
                        return ActPoll::Failed(format!("companion {unit} was removed before its build"));
                    };
                    world.write_message(CompanionCommand::Build { unit, uid: uid.clone(), at, swings });
                    let note = format!("fixture {uid} stands at {at:?}; companion {unit} builds it");
                    // The command takes the next number.
                    self.phase = BuildStep::Building { seq: seq + 1, uid: uid.clone() };
                    ActPoll::Running(note)
                }
            },
            BuildStep::Building { seq, uid } => {
                let (seq, uid) = (*seq, uid.clone());
                match targets::companion_state(world, unit) {
                    Some(state) => built(world, builder, state, seq, unit, &uid),
                    None => ActPoll::Failed(format!("companion {unit} was removed while building")),
                }
            }
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // A fixture saved for a build no companion was asked to make is drawn
        // solid.
        if let BuildStep::Saving { uid, .. } = &self.phase {
            world.resource_mut::<FixtureGhosts>().clear(uid);
        }
    }
}

/// Where the spawned instance of the placed fixture `uid` stands.
fn instance_at(world: &mut World, uid: &str) -> Option<Vec3> {
    world
        .query::<(&moly_features::fixture::FixtureInstanceSeed, &GlobalTransform)>()
        .iter(world)
        .find(|(seed, _)| seed.uid() == uid)
        .map(|(_, transform)| transform.translation())
}

/// Where the placed fixture `uid` of the layout stands: at the root of its
/// instance when that exists, else at the pose its placement row puts it at,
/// on the site's own origin.
pub fn placed_fixture(world: &mut World, uid: &str) -> Option<Vec3> {
    let instance = world
        .query::<(&moly_features::fixture::FixtureInstanceSeed, &GlobalTransform)>()
        .iter(world)
        .find(|(seed, _)| seed.uid() == uid)
        .map(|(_, transform)| transform.translation());
    if instance.is_some() {
        return instance;
    }
    let placements = world.get_resource::<moly_features::fixture::FixturePlacements>()?;
    let row = placements.editor_rows().into_iter().find(|row| row.uid == uid)?;
    let pose = row.pose().ok()?;
    // The put effect's view position: the site's own origin plus the pose.
    let origin = moly_features::act::site::origin(world);
    Some(origin + pose.translation)
}

/// The end of a build: the status at or past the command's number, done or
/// failed; a build that is done names where its fixture stands.
fn built(
    world: &mut World,
    builder: FieldBuilder,
    (state, status): (CompanionState, CompanionStatus),
    seq: u64,
    unit: u32,
    uid: &str,
) -> ActPoll {
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} build {seq}: the command is queued"));
    }
    match state {
        CompanionState::Idle
        | CompanionState::Walking
        | CompanionState::Turning
        | CompanionState::Building
        | CompanionState::Gathering
        | CompanionState::Acting
        | CompanionState::Using => ActPoll::Running(format!("companion {unit} build {seq}: {}", state.label())),
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} build {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => {
            let note = format!("companion {unit} build {seq} done");
            let cell = (builder.placed_cell)(world, uid)
                .map(|(center, direction)| format!(", cell ({}, {}, {}) direction {}", center.x, center.y, center.z, direction as u8))
                .unwrap_or_default();
            match placed_fixture(world, uid) {
                Some(at) => ActPoll::Done(format!("{note}; fixture {uid} at ({:.2}, {:.2}, {:.2}){cell}", at.x, at.y, at.z)),
                None => ActPoll::Done(format!("{note}; fixture {uid}{cell}")),
            }
        }
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} build {seq} failed: {reason}")),
    }
}
