//! The companions: each step writes one command of the companion's own
//! queue and ends when the companion's status reaches the command's number,
//! done or failed. A companion is its unit's body, dressed or not; its
//! commands are the product's own, so the step only decides when to write
//! them and what its note says.
//!
//! A companion's build puts an owned fixture in front of it (or at a given
//! point) through the field build's own put and save, then has the companion
//! hammer it from a ghost to solid ([`crate::companion_build`]).

use bevy::prelude::*;
use moly_features::act::targets;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::companion::{build, CompanionCommand, CompanionState, CompanionStatus};

use crate::companion_build::BuildRun;
use crate::Requested;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("companion.spawn", false, Some(60.0), |fields| Ok(Box::new(read_spawn(fields)?))))
        .step(Step::new("companion.walk_to", false, Some(60.0), |fields| Ok(Box::new(read_walk(fields)?))))
        .step(Step::new("companion.face", false, None, |fields| Ok(Box::new(read_face(fields)?))))
        .step(Step::new("companion.emote", false, None, |fields| Ok(Box::new(read_emote(fields)?))))
        .step(Step::new("companion.motion", false, Some(60.0), |fields| Ok(Box::new(read_motion(fields)?))))
        .step(Step::new("companion.expression", false, None, |fields| Ok(Box::new(read_expression(fields)?))))
        .step(Step::new("companion.balloon", false, None, |fields| Ok(Box::new(read_balloon(fields)?))))
        .step(Step::new("companion.despawn", false, Some(10.0), |fields| Ok(Box::new(Despawn { unit: fields.req_u32("unit")? }))))
        .step(Step::new("companion.build", false, Some(120.0), |fields| Ok(Box::new(read_build(fields)?))))
        .step(Step::new("companion.use", false, Some(120.0), |fields| Ok(Box::new(read_use(fields)?))))
        .install(app);
}

/// The refusal of a step that needs a companion that is spawned.
fn spawned(world: &mut World, unit: u32) -> Result<u64, String> {
    targets::companion_seq(world, unit).ok_or_else(|| format!("no companion of unit {unit} is spawned"))
}

/// Whether no companion of `unit` is spawned, or is being spawned: the
/// reason to refuse a spawn, as the world's companion commands do.
fn companion_absent(world: &mut World, unit: u32) -> Result<(), String> {
    if world.query::<&moly_features::companion::Companion>().iter(world).any(|companion| companion.unit == unit) {
        return Err(format!("a companion of unit {unit} is already spawned"));
    }
    if moly_features::companion::is_spawning(world, unit) {
        return Err(format!("a companion of unit {unit} is already being spawned"));
    }
    Ok(())
}

/// `companion.spawn`: the companion of `unit` put where it stands, or walked
/// to `at`.
#[derive(Debug)]
struct Spawn {
    unit: u32,
    at: Vec3,
}

fn read_spawn(fields: &mut Fields<'_>) -> Result<Spawn, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let at = moly_features::act::triple(fields.req("at")?, &format!("{at}.at"))?;
    Ok(Spawn { unit, at: Vec3::from(at) })
}

impl ActionSpec for Spawn {
    fn name(&self) -> &str {
        "companion.spawn"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        companion_absent(world, self.unit)?;
        Ok(Box::new(SpawnRun { unit: self.unit, at: self.at, written: false, polls: 0 }))
    }
}

/// A spawn, from its command until the body is shown.
struct SpawnRun {
    unit: u32,
    at: Vec3,
    written: bool,
    polls: u8,
}

impl ActionRun for SpawnRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::Spawn { unit, at: self.at });
            return ActPoll::Running(format!("spawning companion {unit}"));
        }
        if let Some(state) = targets::companion_state(world, unit) {
            return spawned_note(state, unit);
        }
        // The spawn waits for its character body before it is a companion.
        if moly_features::companion::is_spawning(world, unit) {
            return ActPoll::Running(format!("companion {unit}: its character body is loading"));
        }
        // The command is read and its spawn entity inserted a frame or two
        // after it is written; nothing there after that is a refusal.
        self.polls += 1;
        if self.polls < 3 {
            return ActPoll::Running(format!("spawning companion {unit}"));
        }
        ActPoll::Failed(format!("companion {unit} was not spawned; see the [companion] log line"))
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The end of a spawn: shown and standing, or walking to a spawn point off
/// its seat.
fn spawned_note(state: (CompanionState, CompanionStatus), unit: u32) -> ActPoll {
    let (state, _) = state;
    match state {
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Idle => ActPoll::Done(format!("companion {unit} spawned")),
        CompanionState::Walking | CompanionState::Turning | CompanionState::Building | CompanionState::Gathering
        | CompanionState::Acting | CompanionState::Using => {
            ActPoll::Running(format!("companion {unit} walks to its spawn point"))
        }
        CompanionState::Done => ActPoll::Done(format!("companion {unit} spawned at its spawn point")),
        // The body is shown from its seat on: a walk-in that stops short
        // leaves the companion standing where it stopped.
        CompanionState::Failed(reason) => {
            ActPoll::Done(format!("companion {unit} spawned short of its spawn point: {reason}"))
        }
    }
}

/// `companion.walk_to`: the companion of `unit` walked to a world point,
/// ending within `within` metres of it.
#[derive(Debug)]
struct WalkTo {
    unit: u32,
    to: Vec3,
    within: f32,
}

fn read_walk(fields: &mut Fields<'_>) -> Result<WalkTo, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let to = moly_features::act::triple(fields.req("to")?, &format!("{at}.to"))?;
    // The walk law caps the distance a walk may finish within.
    let within = fields.opt_f32("within")?.unwrap_or(0.3).min(moly_features::companion::WITHIN_LIMIT);
    Ok(WalkTo { unit, to: Vec3::from(to), within })
}

impl ActionSpec for WalkTo {
    fn name(&self) -> &str {
        "companion.walk_to"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        // The command takes the next number.
        let seq = spawned(world, self.unit)? + 1;
        Ok(Box::new(WalkRun { unit: self.unit, to: self.to, within: self.within, seq, written: false }))
    }
}

/// A walk, from its command until its status reaches its number.
struct WalkRun {
    unit: u32,
    to: Vec3,
    within: f32,
    seq: u64,
    written: bool,
}

impl ActionRun for WalkRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::WalkTo { unit, to: self.to, within: self.within });
            return ActPoll::Running(format!("companion {unit} is walking"));
        }
        match targets::companion_state(world, unit) {
            Some(state) => walked(state, self.seq, unit),
            // Gone after the command: the walk cannot end.
            None => ActPoll::Failed(format!("companion {unit} was removed while walking")),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The end of a walk: the status at or past the command's number, done or
/// failed.
fn walked(state: (CompanionState, CompanionStatus), seq: u64, unit: u32) -> ActPoll {
    let (state, status) = state;
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} walk {seq}: the command is queued"));
    }
    match state {
        CompanionState::Idle | CompanionState::Walking | CompanionState::Turning | CompanionState::Building
        | CompanionState::Gathering | CompanionState::Acting | CompanionState::Using => {
            ActPoll::Running(format!("companion {unit} walk {seq}: {}", state.label()))
        }
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} walk {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => ActPoll::Done(format!("companion {unit} walk {seq} arrived")),
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} walk {seq} failed: {reason}")),
    }
}

/// The point a companion turns to.
#[derive(Clone, Debug)]
enum FaceTo {
    Point([f32; 3]),
    /// The player's position when the step starts.
    Player,
}

/// The point a companion turns to: a world point `[x, y, z]`, or `player`.
fn face_to(value: &serde_json::Value, at: &str) -> Result<FaceTo, String> {
    match value {
        serde_json::Value::String(word) if word == "player" => Ok(FaceTo::Player),
        value => moly_features::act::triple(value, at).map(FaceTo::Point),
    }
}

/// `companion.face`: the companion of `unit` turned to face a world point, or
/// where the player stands as the step starts, over `secs`.
#[derive(Debug)]
struct Face {
    unit: u32,
    to: FaceTo,
    secs: f32,
}

fn read_face(fields: &mut Fields<'_>) -> Result<Face, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let to = face_to(fields.req("to")?, &format!("{at}.to"))?;
    let secs = fields.opt_f32("secs")?.unwrap_or(0.4);
    Ok(Face { unit, to, secs })
}

impl ActionSpec for Face {
    fn name(&self) -> &str {
        "companion.face"
    }

    fn hand(&self) -> bool {
        false
    }

    fn timeout(&self) -> Option<f32> {
        Some(self.secs + 5.0)
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let seq = spawned(world, self.unit)? + 1;
        // `player` is where the player stands as the step starts.
        let to = match &self.to {
            FaceTo::Point(point) => Vec3::from(*point),
            FaceTo::Player => targets::player(world)?,
        };
        Ok(Box::new(FaceRun { unit: self.unit, to, secs: self.secs, seq, written: false }))
    }
}

/// A turn, from its command until its status reaches its number.
struct FaceRun {
    unit: u32,
    to: Vec3,
    secs: f32,
    seq: u64,
    written: bool,
}

impl ActionRun for FaceRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::Face { unit, to: self.to, secs: self.secs });
            let to = self.to;
            return ActPoll::Running(format!("companion {unit} turns to ({:.2}, {:.2}, {:.2})", to.x, to.y, to.z));
        }
        match targets::companion_state(world, unit) {
            Some(state) => turned(state, self.seq, unit),
            // Gone after the command: the turn cannot end.
            None => ActPoll::Failed(format!("companion {unit} was removed while turning")),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The end of a turn: the status at or past the command's number, done or
/// failed.
fn turned(state: (CompanionState, CompanionStatus), seq: u64, unit: u32) -> ActPoll {
    let (state, status) = state;
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} face {seq}: the command is queued"));
    }
    match state {
        CompanionState::Idle | CompanionState::Walking | CompanionState::Building | CompanionState::Gathering
        | CompanionState::Acting | CompanionState::Using => {
            ActPoll::Running(format!("companion {unit} face {seq}: {}", state.label()))
        }
        CompanionState::Turning => ActPoll::Running(format!("companion {unit} face {seq}: turning")),
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} face {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => ActPoll::Done(format!("companion {unit} face {seq} done")),
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} face {seq} failed: {reason}")),
    }
}

/// `companion.emote`: an emoticon shown over the companion of `unit` for
/// `secs`.
#[derive(Debug)]
struct Emote {
    unit: u32,
    name: String,
    secs: f32,
    use_root: bool,
}

fn read_emote(fields: &mut Fields<'_>) -> Result<Emote, String> {
    let unit = fields.req_u32("unit")?;
    let name = fields.req_str("name")?.to_owned();
    let secs = fields.opt_f32("secs")?.unwrap_or(2.0);
    let use_root = fields.opt_bool("use_root")?.unwrap_or(false);
    Ok(Emote { unit, name, secs, use_root })
}

impl ActionSpec for Emote {
    fn name(&self) -> &str {
        "companion.emote"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = targets::companion_entity(world, self.unit)
            .ok_or_else(|| format!("no companion of unit {} is spawned", self.unit))?;
        // The emote rides the body the companion wears, whose bones its own
        // motion moves; an undressed companion carries it itself.
        let actor = targets::worn_body(world, entity).unwrap_or(entity);
        let lease = moly_features::emoticon::timeline::show(world, actor, &self.name, self.use_root)?;
        Ok(Box::new(Showing { lease, until: ctx.now + self.secs, companion: Some(self.unit) }))
    }
}

/// An emoticon over a body until its time; `companion` is the unit whose
/// emote it is, for the note, or none for a character's.
pub struct Showing {
    pub lease: moly_features::emoticon::timeline::EmoteLease,
    pub until: f32,
    pub companion: Option<u32>,
}

impl ActionRun for Showing {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if ctx.now < self.until {
            return match self.companion {
                Some(unit) => ActPoll::Running(format!("companion {unit} balloon: {:.1} s left", self.until - ctx.now)),
                None => ActPoll::Running(format!("{:.1} s left", self.until - ctx.now)),
            };
        }
        moly_features::emoticon::timeline::hide(world, self.lease, false);
        ActPoll::Done(String::new())
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        moly_features::emoticon::timeline::hide(world, self.lease, true);
    }
}

/// `companion.motion`: one motion of the motion library played on the
/// companion's own body once.
#[derive(Debug)]
struct Motion {
    unit: u32,
    motion: String,
}

fn read_motion(fields: &mut Fields<'_>) -> Result<Motion, String> {
    let unit = fields.req_u32("unit")?;
    let motion = fields.req_str("motion")?.to_owned();
    Ok(Motion { unit, motion })
}

impl ActionSpec for Motion {
    fn name(&self) -> &str {
        "companion.motion"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let seq = spawned(world, self.unit)? + 1;
        Ok(Box::new(MotionRun { unit: self.unit, motion: self.motion.clone(), seq, written: false }))
    }
}

/// A motion, from its command until its status reaches its number.
struct MotionRun {
    unit: u32,
    motion: String,
    seq: u64,
    written: bool,
}

impl ActionRun for MotionRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::Motion { unit, motion: self.motion.clone() });
            return ActPoll::Running(format!("companion {unit} plays {}", self.motion));
        }
        match targets::companion_state(world, unit) {
            Some(state) => moved_to(state, self.seq, unit, &self.motion),
            None => ActPoll::Failed(format!("companion {unit} was removed while playing {}", self.motion)),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The end of a motion: the status at or past the command's number, done or
/// failed.
fn moved_to(state: (CompanionState, CompanionStatus), seq: u64, unit: u32, motion: &str) -> ActPoll {
    let (state, status) = state;
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} motion {seq}: the command is queued"));
    }
    match state {
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} motion {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => ActPoll::Done(format!("companion {unit} played {motion}")),
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} motion {seq} failed: {reason}")),
        state => ActPoll::Running(format!("companion {unit} motion {seq}: {}", state.label())),
    }
}

/// `companion.expression`: the eye and mouth patterns of the companion of
/// `unit`, or its unit's default face (`reset`).
#[derive(Debug)]
struct Expression {
    unit: u32,
    eye: Option<String>,
    mouth: Option<String>,
    reset: bool,
}

fn read_expression(fields: &mut Fields<'_>) -> Result<Expression, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let eye = fields.opt_str("eye")?.map(str::to_owned);
    let mouth = fields.opt_str("mouth")?.map(str::to_owned);
    let reset = fields.opt_bool("reset")?.unwrap_or(false);
    if reset && (eye.is_some() || mouth.is_some()) {
        return Err(format!("{at}: reset takes neither eye nor mouth"));
    }
    if !reset && eye.is_none() && mouth.is_none() {
        return Err(format!("{at}: give eye, mouth or reset"));
    }
    Ok(Expression { unit, eye, mouth, reset })
}

impl ActionSpec for Expression {
    fn name(&self) -> &str {
        "companion.expression"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        let entity = targets::companion_entity(world, unit).ok_or_else(|| format!("no companion of unit {unit} is spawned"))?;
        let applied = if self.reset {
            moly_features::companion::expression::reset(world, entity)
        } else {
            moly_features::companion::expression::set(world, entity, self.eye.as_deref(), self.mouth.as_deref())
        };
        let note = match applied {
            Ok(()) if self.reset => format!("companion {unit}: the default face"),
            Ok(()) => "companion face set".to_owned(),
            Err(error) => return Err(error),
        };
        Ok(Box::new(Requested { note }))
    }
}

/// `companion.balloon`: a tweet balloon shown over the companion of `unit`
/// for `secs`, with the tweet master's text of `tweet`, or `text` as given.
#[derive(Debug)]
struct Balloon {
    unit: u32,
    text: Option<String>,
    tweet: Option<i32>,
    secs: f32,
}

fn read_balloon(fields: &mut Fields<'_>) -> Result<Balloon, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let text = fields.opt_str("text")?.map(str::to_owned);
    let tweet = fields.opt_i32("tweet")?;
    if text.is_some() == tweet.is_some() {
        return Err(format!("{at}: give exactly one of text or tweet"));
    }
    let secs = fields.opt_f32("secs")?.unwrap_or(3.0);
    Ok(Balloon { unit, text, tweet, secs })
}

impl ActionSpec for Balloon {
    fn name(&self) -> &str {
        "companion.balloon"
    }

    fn hand(&self) -> bool {
        false
    }

    fn timeout(&self) -> Option<f32> {
        Some(self.secs + 5.0)
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        let entity = targets::companion_entity(world, unit).ok_or_else(|| format!("no companion of unit {unit} is spawned"))?;
        let text = match (&self.text, self.tweet) {
            (Some(text), _) => text.clone(),
            (None, Some(tweet)) => world
                .get_resource::<moly_features::npc_tweet::TweetTables>()
                .ok_or_else(|| "the tweet master is not loaded".to_owned())
                .and_then(|tables| {
                    tables
                        .tweet(tweet)
                        .map(|row| row.text.clone())
                        .ok_or_else(|| format!("the tweet master has no row {tweet}"))
                })?,
            (None, None) => return Err("a balloon has no text".into()),
        };
        crate::npcs::show_balloon(world, entity, text, self.tweet.unwrap_or(0));
        Ok(Box::new(BalloonRun { unit, entity, until: ctx.now + self.secs }))
    }
}

/// A balloon over its body until its time.
struct BalloonRun {
    unit: u32,
    entity: Entity,
    until: f32,
}

impl ActionRun for BalloonRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if ctx.now >= self.until {
            crate::npcs::hide_tweet(world, self.entity);
            ActPoll::Done(String::new())
        } else {
            ActPoll::Running(format!("companion {} balloon: {:.1} s left", self.unit, self.until - ctx.now))
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        crate::npcs::hide_tweet(world, self.entity);
    }
}

/// `companion.despawn`: the companion of `unit` removed.
#[derive(Debug)]
struct Despawn {
    unit: u32,
}

impl ActionSpec for Despawn {
    fn name(&self) -> &str {
        "companion.despawn"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        spawned(world, self.unit)?;
        Ok(Box::new(DespawnRun { unit: self.unit, written: false }))
    }
}

/// A removal, from its command until the companion is gone.
struct DespawnRun {
    unit: u32,
    written: bool,
}

impl ActionRun for DespawnRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::Despawn { unit });
        }
        if targets::companion_seq(world, unit).is_some() {
            ActPoll::Running(format!("removing companion {unit}"))
        } else {
            ActPoll::Done(format!("companion {unit} removed"))
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// Where a `companion.build` puts its fixture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildPlace {
    /// A world point; in front of the companion when absent.
    pub at: Option<[f32; 3]>,
    /// The fixture goes on the cell `at` falls in or the step fails, rather
    /// than on the nearest legal tile.
    pub exact: bool,
    /// The direction number (0 front, 1 left, 2 back, 3 right); the
    /// camera's when absent.
    pub direction: Option<u8>,
}

/// `companion.build`: an owned fixture put near the companion (or at a
/// point), then hammered from a ghost to solid in `swings` hits.
#[derive(Debug)]
struct Build {
    unit: u32,
    fixture: i32,
    texture: Option<u32>,
    place: BuildPlace,
    swings: u32,
}

fn read_build(fields: &mut Fields<'_>) -> Result<Build, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let fixture = fields.req_i32("fixture")?;
    let texture = fields.opt_u32("texture")?;
    let near = fields.opt("near").map(|near| moly_features::act::triple(near, &format!("{at}.near"))).transpose()?;
    let exact = fields.opt("at").map(|exact| moly_features::act::triple(exact, &format!("{at}.at"))).transpose()?;
    if near.is_some() && exact.is_some() {
        return Err(format!("{at}: takes one of near and at"));
    }
    let direction = fields.opt_u32("direction")?;
    if direction.is_some_and(|direction| direction > 3) {
        return Err(format!("{at}: direction is 0 to 3"));
    }
    let place = BuildPlace {
        at: exact.or(near),
        exact: exact.is_some(),
        direction: direction.map(|direction| direction as u8),
    };
    let swings = fields.opt_u32("swings")?.unwrap_or(build::DEFAULT_SWINGS).clamp(1, build::MAX_SWINGS);
    Ok(Build { unit, fixture, texture, place, swings })
}

impl ActionSpec for Build {
    fn name(&self) -> &str {
        "companion.build"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        spawned(world, self.unit)?;
        Ok(Box::new(BuildRun::new(self.unit, self.fixture, self.texture, self.place, self.swings)))
    }
}

/// `companion.use`: the companion of `unit` walks to a placed fixture and
/// plays its character's own action there.
#[derive(Clone, Debug)]
enum UseTarget {
    /// The placed copy of this master id nearest the companion.
    Nearest(i32),
    /// The placed fixture with this uid.
    Uid(String),
}

#[derive(Debug)]
struct Use {
    unit: u32,
    target: UseTarget,
    activity: Option<i32>,
    variant: Option<u32>,
}

fn read_use(fields: &mut Fields<'_>) -> Result<Use, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let target = match (fields.opt_i32("fixture")?, fields.opt_str("uid")?) {
        (Some(fixture), None) => UseTarget::Nearest(fixture),
        (None, Some(uid)) => UseTarget::Uid(uid.to_owned()),
        _ => return Err(format!("{at}: takes one of fixture and uid")),
    };
    let activity = fields.opt_i32("activity")?;
    let variant = fields.opt_u32("variant")?;
    Ok(Use { unit, target, activity, variant })
}

impl ActionSpec for Use {
    fn name(&self) -> &str {
        "companion.use"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let seq = spawned(world, self.unit)? + 1;
        let uid = use_target(world, self.unit, &self.target)?;
        Ok(Box::new(UseRun { unit: self.unit, uid, activity: self.activity, variant: self.variant, seq, written: false }))
    }
}

/// The uid of the fixture a `companion.use` step names: the placed copy of
/// a master id nearest the companion, or a placed uid as given.
fn use_target(world: &mut World, unit: u32, target: &UseTarget) -> Result<String, String> {
    let mut placed = world.query::<(&moly_features::fixture_activity_state::FixtureActivityIdentity, &GlobalTransform)>();
    match target {
        UseTarget::Uid(uid) => placed
            .iter(world)
            .any(|(identity, _)| identity.uid == *uid)
            .then(|| uid.clone())
            .ok_or_else(|| format!("no placed fixture {uid} on this site")),
        UseTarget::Nearest(fixture) => {
            let entity = targets::companion_entity(world, unit).ok_or_else(|| format!("no companion of unit {unit} is spawned"))?;
            let from = world
                .get::<GlobalTransform>(entity)
                .map(GlobalTransform::translation)
                .ok_or_else(|| format!("companion {unit} has no place yet"))?;
            placed
                .iter(world)
                .filter(|(identity, _)| identity.master_id == *fixture)
                .min_by(|(_, a), (_, b)| {
                    a.translation().distance_squared(from).total_cmp(&b.translation().distance_squared(from))
                })
                .map(|(identity, _)| identity.uid.clone())
                .ok_or_else(|| format!("no placed fixture {fixture} on this site"))
        }
    }
}

/// A use, from its command until its status reaches its number.
struct UseRun {
    unit: u32,
    uid: String,
    activity: Option<i32>,
    variant: Option<u32>,
    seq: u64,
    written: bool,
}

impl ActionRun for UseRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if !self.written {
            self.written = true;
            world.write_message(CompanionCommand::Use { unit, uid: self.uid.clone(), activity: self.activity, variant: self.variant });
            return ActPoll::Running(format!("companion {unit} uses fixture {}", self.uid));
        }
        match targets::companion_state(world, unit) {
            Some(state) => used(state, self.seq, unit, &self.uid),
            None => ActPoll::Failed(format!("companion {unit} was removed while using fixture {}", self.uid)),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The end of a use: the status at or past the command's number, done or
/// failed.
fn used(state: (CompanionState, CompanionStatus), seq: u64, unit: u32, uid: &str) -> ActPoll {
    let (state, status) = state;
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} use {seq}: the command is queued"));
    }
    match state {
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} use {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => ActPoll::Done(format!("companion {unit} used fixture {uid}")),
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} use {seq} failed: {reason}")),
        state => ActPoll::Running(format!("companion {unit} use {seq}: {}", state.label())),
    }
}
