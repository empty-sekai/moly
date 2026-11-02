//! The harvest steps, registered with the hub's action registry.
//!
//! `player.gather {gatherable, kind?, collect?}` takes one gatherable through
//! the harvest button and, with `collect`, walks over the drops it leaves: the
//! object is chosen once, when the step starts; the button is tapped as a
//! finger taps it, so the harvest screen's own rules (the tool for the kind,
//! the stamina, the swing loop) decide each press.
//!
//! `companion.gather {unit, gatherable, kind?}` has the companion of `unit`
//! swing at a wood or mineral harvest object until it is harvested, with the
//! player's tool for it.

use bevy::prelude::*;
use moly_features::act::walk::{Walk, WalkPoll};
use moly_features::act::{hand, targets, ActionRun, ActionSpec, ActCtx, ActPoll, Fields, GatherRef, Kinds, Step, Target, TargetKind};
use moly_features::companion::CompanionCommand;

use crate::action::{HarvestAction, HarvestPlayerModel, HarvestTargeting};
use crate::companion_gather::{self, GatherTarget};
use crate::{HarvestButton, HarvestDropItem, HarvestObject, STATUS_HARVESTED};

/// Drops are taken from this far around the harvested object (metres).
const DROP_REACH: f32 = 6.0;
/// A drop the player has stood on this long (seconds) is not taken.
const DROP_REFUSED_SECS: f32 = 3.0;
/// A press that has started no swing after this long (seconds) is refused.
const PRESS_ANSWER_SECS: f32 = 1.5;
/// Presses are this far apart (seconds), as a hand taps.
const PRESS_INTERVAL: f32 = 0.3;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("player.gather", true, Some(120.0), |fields| Ok(Box::new(read_gather(fields)?))))
        .step(Step::new("companion.gather", false, Some(120.0), |fields| Ok(Box::new(read_companion_gather(fields)?))))
        .install(app);
}

/// PostUpdate: where the site's gatherables stand, as the hub's targets
/// resolve them (`{gatherable: uid}`).
pub fn report_targets(world: &mut World) {
    let rows: Vec<(u64, Vec3)> = {
        let mut objects = world.query::<(&HarvestObject, &GlobalTransform)>();
        objects.iter(world).map(|(object, at)| (object.uid, at.translation())).collect()
    };
    if let Some(mut reported) = world.get_resource_mut::<moly_features::act::targets::Gatherables>() {
        reported.set(rows);
    }
}

/// The gatherable a step names: a harvest object UID, or the nearest one
/// left (of a kind word when given).
fn gather_ref(fields: &mut Fields<'_>, at: &str) -> Result<GatherRef, String> {
    let what = match fields.req("gatherable")? {
        serde_json::Value::String(word) if word == "nearest" => GatherRef::Nearest { kind: fields.opt_str("kind")?.map(str::to_owned) },
        value => GatherRef::Uid(
            value.as_u64().ok_or_else(|| format!("{at}: gatherable is a harvest object UID or \"nearest\""))?,
        ),
    };
    if matches!(what, GatherRef::Uid(_)) && fields.opt("kind").is_some() {
        return Err(format!("{at}: kind only narrows gatherable \"nearest\""));
    }
    Ok(what)
}

/// `player.gather`: one harvest object through the harvest button, then its
/// drops.
#[derive(Debug)]
struct Gather {
    what: GatherRef,
    collect: bool,
}

fn read_gather(fields: &mut Fields<'_>) -> Result<Gather, String> {
    let at = fields.at().to_owned();
    let what = gather_ref(fields, &at)?;
    let collect = fields.opt_bool("collect")?.unwrap_or(true);
    Ok(Gather { what, collect })
}

impl ActionSpec for Gather {
    fn name(&self) -> &str {
        "player.gather"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(GatherRun {
            what: self.what.clone(),
            collect: self.collect,
            object: None,
            walk: None,
            phase: GatherPhase::Approach,
            lifted_at: None,
            last_press: None,
            presses: 0,
            hp_at_press: 0,
            drop: None,
            drops_taken: 0,
        }))
    }
}

enum GatherPhase {
    /// Walking into the object's contact circle until it is the target.
    Approach,
    /// Tapping the harvest button while the object has hit points left.
    Swing,
    /// Walking over the landed drops.
    Collect,
}

/// A gathering in progress.
struct GatherRun {
    what: GatherRef,
    collect: bool,
    /// The object: its entity, UID, position and status.
    object: Option<(Entity, u64, Vec3, i32)>,
    walk: Option<Walk>,
    phase: GatherPhase,
    /// When the joystick finger was lifted for the presses.
    lifted_at: Option<f32>,
    last_press: Option<f32>,
    presses: u32,
    /// The hit points when the last unanswered press went out.
    hp_at_press: i32,
    /// The drop walked to, and since when the player has stood on it.
    drop: Option<(Vec3, Option<f32>)>,
    drops_taken: u32,
}

impl GatherRun {
    /// The harvest object the step names: entity, UID, position, radius.
    fn find(&self, world: &mut World) -> Result<(Entity, u64, Vec3, f32, i32), String> {
        let from = targets::player(world).unwrap_or(Vec3::ZERO);
        let rows = gatherables(world);
        if rows.is_empty() {
            return Err("this site has no gatherables".into());
        }
        match &self.what {
            GatherRef::Uid(uid) => {
                let row = rows
                    .into_iter()
                    .find(|row| row.0 == *uid)
                    .ok_or_else(|| format!("no gatherable with UID {uid} on this site"))?;
                if row.4 == STATUS_HARVESTED {
                    return Err(format!("gatherable {uid} is already harvested"));
                }
                Ok((row.1, row.0, row.2, row.3, row.4))
            }
            GatherRef::Nearest { kind } => rows
                .into_iter()
                .filter(|row| row.4 != STATUS_HARVESTED)
                .filter(|row| kind.as_deref().is_none_or(|kind| crate::kind_word(row.5) == kind))
                .min_by(|a, b| a.2.distance_squared(from).total_cmp(&b.2.distance_squared(from)))
                .map(|row| (row.1, row.0, row.2, row.3, row.4))
                .ok_or_else(|| match kind {
                    Some(kind) => format!("no {kind} gatherable left to harvest on this site"),
                    None => "no gatherable left to harvest on this site".to_owned(),
                }),
        }
    }
}

/// The site's gatherables: their UID, entity, position, radius, status and
/// view type.
fn gatherables(world: &mut World) -> Vec<(u64, Entity, Vec3, f32, i32, i32)> {
    let mut objects = world.query::<(Entity, &HarvestObject, &GlobalTransform)>();
    objects
        .iter(world)
        .map(|(entity, object, at)| {
            (object.uid, entity, at.translation(), object.radius, object.status, object.fixture_type)
        })
        .collect()
}

impl ActionRun for GatherRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let (entity, uid, origin, _) = match self.object {
            Some(object) => object,
            None => match self.find(world) {
                Ok((entity, uid, at, radius, status)) => {
                    self.object = Some((entity, uid, at, status));
                    // Contact is the player inside the object's circle.
                    let within = (radius * 0.6).max(0.15);
                    self.walk = Some(Walk::contact(Target::at(TargetKind::Gatherable(uid)), within));
                    (entity, uid, at, status)
                }
                Err(error) => {
                    lift(world, ctx);
                    return ActPoll::Failed(error);
                }
            },
        };
        let Some(object) = world.get::<HarvestObject>(entity) else {
            lift(world, ctx);
            return ActPoll::Failed(format!("gatherable {uid} left the site"));
        };
        // The object as it stands now: the game's reads move it on (a hit,
        // the harvest), so it is read again each frame, not carried.
        let (hp, kind, status) = (object.hp, object.fixture_type, object.status);
        let harvested = status == STATUS_HARVESTED;
        let targeted = world.get_resource::<HarvestTargeting>().and_then(|targeting| targeting.target);
        let swinging = world.get_resource::<HarvestAction>().is_some_and(HarvestAction::in_progress);
        let now = ctx.now;
        match self.phase {
            GatherPhase::Approach => {
                if harvested {
                    self.phase = GatherPhase::Collect;
                    return ActPoll::Running(format!("gatherable {uid} is harvested"));
                }
                if targeted == Some(entity) {
                    self.phase = GatherPhase::Swing;
                    return ActPoll::Running(format!("gatherable {uid} is the target"));
                }
                let (ticket, path) = (ctx.ticket, ctx.path.clone());
                let polled = {
                    let walk = self.walk.as_mut().expect("the walk is planned with the object");
                    walk.poll(world, ticket, &path, now)
                };
                match polled {
                    // In reach while another object is the target: walk on
                    // into this one, which the player then faces.
                    WalkPoll::Done(note) => {
                        self.walk.as_mut().expect("the walk is planned with the object").set_within(0.05);
                        ActPoll::Running(format!("{note}; the target is {targeted:?}, not gatherable {uid}"))
                    }
                    other => other.into(),
                }
            }
            GatherPhase::Swing => {
                if harvested {
                    if swinging {
                        return ActPoll::Running(format!("gatherable {uid} harvested; the swing ends"));
                    }
                    if !self.collect {
                        return ActPoll::Done(self.summary(uid, hp));
                    }
                    self.phase = GatherPhase::Collect;
                    return ActPoll::Running(format!("gatherable {uid} harvested after {} presses", self.presses));
                }
                if swinging {
                    return ActPoll::Running(format!("swinging: hp {hp}"));
                }
                if targeted != Some(entity) {
                    self.phase = GatherPhase::Approach;
                    self.lifted_at = None;
                    return ActPoll::Running(format!("gatherable {uid} is no longer the target ({targeted:?})"));
                }
                // Lift the joystick finger first and press on a later frame.
                let pressing = hand::leaning(world).is_some_and(|(owner, _)| owner == ctx.ticket);
                if pressing || self.lifted_at.is_none() {
                    lift(world, ctx);
                    self.lifted_at = Some(now);
                    return ActPoll::Running("standing for the presses".into());
                }
                let Some(button) = world.get_resource::<HarvestButton>() else {
                    return ActPoll::Running("the harvest screen is not set up".into());
                };
                if button.cannot {
                    let tool = (0..=9).contains(&kind).then(|| crate::law::tool_type_for(kind)).flatten();
                    return ActPoll::Failed(format!(
                        "the harvest button shows it cannot act: no tool for gatherable {uid} (it needs {tool:?})"
                    ));
                }
                if !(button.shown && button.interactable) {
                    return ActPoll::Running(format!(
                        "the harvest button is not ready (shown {}, interactable {})",
                        button.shown, button.interactable
                    ));
                }
                if let Some(last) = self.last_press {
                    if hp == self.hp_at_press && now - last > PRESS_ANSWER_SECS {
                        let stamina = world.get_resource::<HarvestPlayerModel>().map(|model| model.stamina.sum());
                        return ActPoll::Failed(format!(
                            "the press on gatherable {uid} started no swing (hp {hp}, stamina {stamina:?}, {} presses)",
                            self.presses
                        ));
                    }
                    if now - last < PRESS_INTERVAL {
                        return ActPoll::Running(format!("hp {hp}"));
                    }
                }
                let (view, node) = crate::action_button_node();
                let position = match hand::ui_point(world, view, node) {
                    Ok(position) => position,
                    Err(error) => return ActPoll::Running(error),
                };
                match hand::tap(world, position) {
                    Ok(()) => {
                        self.hp_at_press = hp;
                        self.last_press = Some(now);
                        self.presses += 1;
                        ActPoll::Running(format!("pressed the harvest button ({}), hp {hp}", self.presses))
                    }
                    Err(error) => ActPoll::Running(error.to_owned()),
                }
            }
            GatherPhase::Collect => self.collect(world, ctx, uid, origin, hp),
        }
    }

    fn stop(&mut self, world: &mut World, ctx: &mut ActCtx) {
        lift(world, ctx);
    }
}

impl GatherRun {
    fn collect(&mut self, world: &mut World, ctx: &mut ActCtx, uid: u64, origin: Vec3, hp: i32) -> ActPoll {
        let (landed, falling) = {
            let mut drops = world.query::<(&HarvestDropItem, &GlobalTransform)>();
            let mut landed = Vec::new();
            let mut falling = 0;
            for (drop, at) in drops.iter(world) {
                let at = at.translation();
                if targets::flat(at, origin) > DROP_REACH {
                    continue;
                }
                if drop.radius > 0.0 {
                    landed.push(at);
                } else {
                    falling += 1;
                }
            }
            (landed, falling)
        };
        let player = match targets::player(world) {
            Ok(player) => player,
            Err(error) => return ActPoll::Running(error),
        };
        // The drop walked to is taken once nothing lies where it was.
        if let Some((at, _)) = self.drop {
            if !landed.iter().any(|drop| drop.distance(at) < 0.05) {
                self.drop = None;
                self.walk = None;
                self.drops_taken += 1;
            }
        }
        if self.drop.is_none() {
            let Some(next) = landed.iter().copied().min_by(|a, b| targets::flat(*a, player).total_cmp(&targets::flat(*b, player))) else {
                if falling > 0 {
                    return ActPoll::Running(format!("{falling} drops still falling"));
                }
                lift(world, ctx);
                return ActPoll::Done(self.summary(uid, hp));
            };
            // Pickups happen only while the player moves, so the walk aims
            // past the drop: a drop already underfoot is still crossed. The
            // point is a heading, not a place to stand (the drop's pickup
            // radius takes it), so a snapped point is not a refusal.
            let away = Vec2::new(next.x - player.x, next.z - player.z).normalize_or_zero();
            let beyond = if away == Vec2::ZERO { Vec2::X } else { away } * 0.5;
            self.drop = Some((next, None));
            self.walk = Some(Walk::contact(Target::at(TargetKind::Point([next.x + beyond.x, next.z + beyond.y])), 0.15));
        }
        let (ticket, path) = (ctx.ticket, ctx.path.clone());
        let polled = {
            let walk = self.walk.as_mut().expect("the walk is planned with the drop");
            walk.poll(world, ticket, &path, ctx.now).into()
        };
        let (at, since) = self.drop.as_mut().expect("a drop is chosen above");
        match polled {
            ActPoll::Done(_) => {
                let since = *since.get_or_insert(ctx.now);
                if ctx.now - since <= DROP_REFUSED_SECS {
                    return ActPoll::Running(format!("on the drop at ({:.2}, {:.2})", at.x, at.z));
                }
                let at = *at;
                let bag = world
                    .get_resource::<moly_features::userdata::client::inventory::ClientMysekaiInventory>()
                    .map(|inventory| {
                        let limit = world
                            .get_resource::<moly_features::userdata::client::inventory::PossessionMasters>()
                            .and_then(|masters| inventory.material_max_count(masters));
                        (inventory.total_material_possession(), limit)
                    });
                lift(world, ctx);
                ActPoll::Failed(format!(
                    "the drop at ({:.2}, {:.2}) is not taken after {DROP_REFUSED_SECS} s on it (bag held and limit {bag:?})",
                    at.x, at.z
                ))
            }
            other => other,
        }
    }

    fn summary(&self, uid: u64, hp: i32) -> String {
        format!("gatherable {uid} harvested: {} presses, hp {hp}, {} drops taken", self.presses, self.drops_taken)
    }
}

/// Lifts the joystick finger when the step has it down.
fn lift(world: &mut World, ctx: &mut ActCtx) {
    let (ticket, path) = (ctx.ticket, ctx.path.clone());
    hand::release_of(world, ticket, &path);
}

/// `companion.gather`: the companion of `unit` swinging at a wood or mineral
/// harvest object until it is harvested, with the player's tool for it.
#[derive(Debug)]
struct CompanionGather {
    unit: u32,
    what: GatherTarget,
}

fn read_companion_gather(fields: &mut Fields<'_>) -> Result<CompanionGather, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let what = match fields.req("gatherable")? {
        serde_json::Value::String(word) if word == "nearest" => GatherTarget::Nearest { kind: fields.opt_str("kind")?.map(str::to_owned) },
        value => GatherTarget::Uid(
            value.as_u64().ok_or_else(|| format!("{at}: gatherable is a harvest object UID or \"nearest\""))?,
        ),
    };
    if matches!(what, GatherTarget::Uid(_)) && fields.opt("kind").is_some() {
        return Err(format!("{at}: kind only narrows gatherable \"nearest\""));
    }
    Ok(CompanionGather { unit, what })
}

impl ActionSpec for CompanionGather {
    fn name(&self) -> &str {
        "companion.gather"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        let seq = targets::companion_seq(world, unit).ok_or_else(|| format!("no companion of unit {unit} is spawned"))?;
        let Some(from) = targets::companion_place(world, unit).map(|place| place.translation) else {
            return Err(format!("companion {unit} has no place yet"));
        };
        // The command takes the next number.
        let (command, note) = companion_gather::prepare(world, unit, from, &self.what)?;
        Ok(Box::new(CompanionGatherRun { unit, command: Some(command), note, seq: seq + 1 }))
    }
}

/// A companion's gather: its command, written on the first frame.
struct CompanionGatherRun {
    unit: u32,
    command: Option<CompanionCommand>,
    note: String,
    seq: u64,
}

impl ActionRun for CompanionGatherRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let unit = self.unit;
        if let Some(command) = self.command.take() {
            world.write_message(command);
            return ActPoll::Running(format!("companion {unit} gathers {}", self.note));
        }
        match targets::companion_state(world, unit) {
            Some(state) => gathered(state, self.seq, unit, &self.note),
            None => ActPoll::Failed(format!("companion {unit} was removed while gathering")),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {
        // A gather plays on: its swings are the companion's.
    }
}

/// The end of a gather: the status at or past the command's number, done or
/// failed.
fn gathered(
    state: (moly_features::companion::CompanionState, moly_features::companion::CompanionStatus),
    seq: u64,
    unit: u32,
    note: &str,
) -> ActPoll {
    use moly_features::companion::CompanionState;
    let (state, status) = state;
    if status.seq < seq {
        return ActPoll::Running(format!("companion {unit} gather {seq}: the command is queued"));
    }
    match state {
        CompanionState::Spawned => ActPoll::Running(format!("companion {unit}: its body is loading")),
        CompanionState::Done if status.seq > seq => ActPoll::Failed(format!(
            "companion {unit} gather {seq} was never accepted: command {} came first",
            status.seq
        )),
        CompanionState::Done => ActPoll::Done(format!("companion {unit} gathered {note}")),
        CompanionState::Failed(reason) => ActPoll::Failed(format!("companion {unit} gather {seq} failed: {reason}")),
        state => ActPoll::Running(format!("companion {unit} gather {seq}: {}", state.label())),
    }
}
