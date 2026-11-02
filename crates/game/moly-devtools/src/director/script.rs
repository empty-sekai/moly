//! The script format and its validation. A script is checked whole before
//! any step runs: an unknown action, an unknown field or a value of the wrong
//! kind is refused with the step's path and the field's name.
//!
//! The flow steps (`wait`, `wait_until`, `assert`, `log`, `where`, `capture`,
//! `parallel`, `repeat`) and the macros that stand for flow around an
//! interaction (`player.talk_to`, `player.craft`, `site.switch`,
//! `site.enter_room`, `site.go_home`) are the script runner's own. Every
//! other step is an action of the hub's registry, checked by the crate that
//! owns it ([`moly_features::act`]) and kept as the specs it stands for
//! ([`Node::Act`]).

use std::sync::Arc;

use bevy::prelude::*;
use moly_features::act::{self as acts, ActionSpec, Cond, Fields, SiteKindName, Target, TargetKind};
use moly_law::action_button::ButtonType;
use serde_json::{Map, Value};

/// A validated script.
#[derive(Clone)]
pub struct Script {
    pub name: String,
    pub steps: Arc<[Step]>,
    /// Native runs end the application when the script ends: success when it
    /// completed, failure when a step failed.
    pub exit: bool,
}

/// One step with its common fields.
#[derive(Clone)]
pub struct Step {
    pub node: Node,
    /// The step's name in the log: its `label`, or its action.
    pub label: String,
    /// The step's action (`player.walk_to`, `log`, ...), whatever its label.
    pub action: String,
    /// Seconds the step may run before it fails.
    pub timeout: Option<f32>,
    /// A failure is logged and the script goes on.
    pub optional: bool,
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Step")
            .field("node", &self.node)
            .field("label", &self.label)
            .field("action", &self.action)
            .field("timeout", &self.timeout)
            .field("optional", &self.optional)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// The block ends when every track has ended.
    All,
    /// The block ends when one track has ended; the others are stopped.
    Any,
}

/// One step's node.
#[derive(Clone)]
pub enum Node {
    Wait { secs: f32 },
    WaitUntil { until: Cond },
    Assert { that: Cond },
    Log { text: String },
    Where { of: Vec<Target> },
    Capture,
    Parallel { tracks: Vec<Arc<[Step]>>, join: Join },
    Repeat { times: u32, steps: Arc<[Step]> },
    /// An action of the registry: the specs it stands for, in order.
    Act(Arc<[Arc<dyn ActionSpec>]>),
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wait { secs } => f.debug_struct("Wait").field("secs", secs).finish(),
            Self::WaitUntil { until } => f.debug_struct("WaitUntil").field("until", until).finish(),
            Self::Assert { that } => f.debug_struct("Assert").field("that", that).finish(),
            Self::Log { text } => f.debug_struct("Log").field("text", text).finish(),
            Self::Where { of } => f.debug_struct("Where").field("of", &of.len()).finish(),
            Self::Capture => f.write_str("Capture"),
            Self::Parallel { tracks, join } => {
                f.debug_struct("Parallel").field("tracks", &tracks.len()).field("join", join).finish()
            }
            Self::Repeat { times, steps } => {
                f.debug_struct("Repeat").field("times", times).field("steps", &steps.len()).finish()
            }
            Self::Act(specs) => f.debug_struct("Act").field("specs", specs).finish(),
        }
    }
}

/// Parses a script document, with its actions checked against the world's
/// registry.
pub fn parse(document: &Value, world: &mut World) -> Result<Script, String> {
    let mut top = fields_of(document, "script")?;
    let name = top.opt_str("name")?.unwrap_or("script").to_owned();
    let exit = top.opt_bool("exit")?.unwrap_or(false);
    let steps = steps(world, top.req("steps")?, "steps")?;
    top.finish()?;
    if steps.is_empty() {
        return Err("script: steps is empty".into());
    }
    Ok(Script { name, steps, exit })
}

/// Parses one step object, checked as a script's step is; a macro expands
/// into the several steps it stands for.
pub fn parse_step(world: &mut World, value: &Value) -> Result<Arc<[Step]>, String> {
    step(world, value, "step").map(Arc::from)
}

fn steps(world: &mut World, value: &Value, at: &str) -> Result<Arc<[Step]>, String> {
    let list = value.as_array().ok_or_else(|| format!("{at}: expected a list of steps"))?;
    let mut out = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        out.extend(step(world, item, &format!("{at}[{index}]"))?);
    }
    Ok(out.into())
}

/// One step; a macro expands into several.
fn step(world: &mut World, value: &Value, at: &str) -> Result<Vec<Step>, String> {
    let mut fields = fields_of(value, at)?;
    let action = fields.req_str("do")?.to_owned();
    let at = format!("{at} ({action})");
    let label = fields.opt_str("label")?.map(str::to_owned);
    let timeout = fields.opt_f32("timeout")?;
    let optional = fields.opt_bool("optional")?.unwrap_or(false);
    let one = |node: Node, default_timeout: Option<f32>| Step {
        node,
        label: label.clone().unwrap_or_else(|| action.clone()),
        action: action.clone(),
        timeout: timeout.or(default_timeout),
        optional,
    };
    let steps = match action.as_str() {
        "wait" => vec![one(Node::Wait { secs: fields.req_f32("secs")? }, None)],
        "wait_until" => vec![one(Node::WaitUntil { until: cond_fields(&mut fields, "until")? }, Some(30.0))],
        "assert" => vec![one(Node::Assert { that: cond_fields(&mut fields, "that")? }, None)],
        "log" => vec![one(Node::Log { text: fields.req_str("text")?.to_owned() }, None)],
        "where" => {
            let of = match fields.opt("of") {
                Some(list) => list
                    .as_array()
                    .ok_or_else(|| format!("{at}: of must be a list of targets"))?
                    .iter()
                    .map(|item| target(item, &at))
                    .collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
            };
            vec![one(Node::Where { of }, None)]
        }
        "capture" => vec![one(Node::Capture, None)],
        "parallel" => {
            let list = fields.req("tracks")?.as_array().ok_or_else(|| format!("{at}: tracks must be a list of step lists"))?;
            let tracks = list
                .iter()
                .enumerate()
                .map(|(index, track)| steps(world, track, &format!("{at}.tracks[{index}]")))
                .collect::<Result<Vec<_>, _>>()?;
            let join = match fields.opt_str("join")?.unwrap_or("all") {
                "all" => Join::All,
                "any" => Join::Any,
                other => return Err(format!("{at}: join is all or any, not {other}")),
            };
            vec![one(Node::Parallel { tracks, join }, None)]
        }
        "repeat" => {
            let times = fields.req_u32("times")?;
            vec![one(Node::Repeat { times, steps: steps(world, fields.req("steps")?, &format!("{at}.steps"))? }, None)]
        }
        "player.talk_to" => {
            let unit = fields.req_u32("unit")?;
            let stop = fields.opt_f32("stop")?.unwrap_or(0.8);
            vec![
                one(action_step(world, "player.interact", interact_fields(TargetKind::Npc(unit), ButtonType::Talk, stop), &at)?, Some(60.0)),
                one(Node::WaitUntil { until: Cond::TalkActive }, Some(10.0)),
            ]
        }
        "player.craft" => craft(world, &mut fields, &at)?,
        // The entry's Finish is what opens movement and gestures; a site
        // move before it breaks the join's room/player check. Wait for it,
        // then request the move.
        "site.switch" => {
            let site = fields.req_str("site")?.to_owned();
            let mut own = Map::new();
            own.insert("site".to_owned(), Value::String(site));
            vec![
                one(Node::WaitUntil { until: Cond::ControlOpen }, Some(30.0)),
                one(action_step(world, "site.switch", own, &at)?, Some(60.0)),
            ]
        }
        // Through the house's door: walk to its entry box, press the entry
        // button, then wait for the move to start and to land in a room.
        "site.enter_room" => door(world, &one, TargetKind::HouseDoor, ButtonType::HouseEntry, SiteKindName::Room, &at)?,
        "site.go_home" => door(world, &one, TargetKind::RoomDoor, ButtonType::GoHomeSite, SiteKindName::Home, &at)?,
        other => {
            // The step's own fields are the action's: the registry reads them
            // and gives the specs they stand for.
            let own = fields.rest();
            let specs = acts::parse(world, other, &own, &at)?;
            vec![one(Node::Act(specs.into()), None)]
        }
    };
    fields.finish()?;
    Ok(steps)
}

/// The steps the actions of other domains start: the walk of an interaction
/// and a site move's request.
fn craft(world: &mut World, fields: &mut Fields<'_>, at: &str) -> Result<Vec<Step>, String> {
    let workbench = fields.opt("workbench").cloned();
    let mut workbench_own = Map::new();
    workbench_own.insert("target".to_owned(), workbench.clone().unwrap_or(Value::String("workbench".to_owned())));
    workbench_own.insert("button".to_owned(), Value::String(format!("{:?}", ButtonType::OpenCraftTool)));
    workbench_own.insert("stop".to_owned(), Value::from(0.2));
    let mut craft_own = Map::new();
    craft_own.insert("blueprint".to_owned(), Value::from(fields.req_i32("blueprint")?));
    let mut out = Vec::new();
    if !matches!(workbench, Some(Value::Bool(false))) {
        out.push(action_step(world, "player.interact", workbench_own, at)?);
    }
    out.push(action_step(world, "player.craft", craft_own, at)?);
    Ok(out
        .into_iter()
        .map(|node| Step { node, label: String::new(), action: String::new(), timeout: Some(60.0), optional: false })
        .collect())
}

/// The step of an interaction with a target's button.
fn interact_step(world: &mut World, target: TargetKind, button: ButtonType, stop: f32, at: &str) -> Result<Node, String> {
    action_step(world, "player.interact", interact_fields(target, button, stop), at)
}

/// The fields a `player.interact` step is read from.
fn interact_fields(target: TargetKind, button: ButtonType, stop: f32) -> Map<String, Value> {
    let mut own = Map::new();
    own.insert("target".to_owned(), target_value(&target));
    own.insert("button".to_owned(), Value::String(format!("{button:?}")));
    own.insert("stop".to_owned(), Value::from(stop));
    own
}

/// A target kind as a step's `target` field.
fn target_value(target: &TargetKind) -> Value {
    match target {
        TargetKind::HouseDoor => Value::String("house_door".to_owned()),
        TargetKind::RoomDoor => Value::String("room_door".to_owned()),
        TargetKind::Player => Value::String("player".to_owned()),
        TargetKind::Workbench => Value::String("workbench".to_owned()),
        TargetKind::Point([x, z]) => Value::Array(vec![Value::from(*x), Value::from(*z)]),
        other => {
            let mut object = Map::new();
            match other {
                TargetKind::Npc(unit) => {
                    object.insert("npc".to_owned(), Value::from(*unit));
                }
                TargetKind::FixtureUid(uid) => {
                    object.insert("fixture".to_owned(), Value::String(uid.clone()));
                }
                TargetKind::FixtureId(id) => {
                    object.insert("fixture".to_owned(), Value::from(*id));
                }
                TargetKind::Gatherable(uid) => {
                    object.insert("gatherable".to_owned(), Value::from(*uid));
                }
                TargetKind::Point(_)
                | TargetKind::HouseDoor
                | TargetKind::RoomDoor
                | TargetKind::Player
                | TargetKind::Workbench => {}
            }
            Value::Object(object)
        }
    }
}

/// The node an action of the registry stands for.
fn action_step(world: &mut World, name: &str, own: Map<String, Value>, at: &str) -> Result<Node, String> {
    acts::parse(world, name, &own, at).map(|specs| Node::Act(specs.into()))
}

fn door(
    world: &mut World,
    one: &dyn Fn(Node, Option<f32>) -> Step,
    door: TargetKind,
    button: ButtonType,
    lands: SiteKindName,
    at: &str,
) -> Result<Vec<Step>, String> {
    Ok(vec![
        // The entry's control gate comes first: the door press belongs to the
        // player who can move, not to the cover.
        one(Node::WaitUntil { until: Cond::ControlOpen }, Some(30.0)),
        one(interact_step(world, door, button, 0.2, at)?, Some(60.0)),
        one(Node::WaitUntil { until: Cond::SiteMoving }, Some(5.0)),
        one(
            Node::WaitUntil {
                until: Cond::All(vec![Cond::Not(Box::new(Cond::SiteMoving)), Cond::SiteKind(lands)]),
            },
            Some(60.0),
        ),
    ])
}

/// A condition read from a step's field.
fn cond_fields<'a>(fields: &mut Fields<'a>, key: &'a str) -> Result<Cond, String> {
    let at = fields.at().to_owned();
    let value = fields.req(key)?.clone();
    acts::condition::parse(&value, &at)
}

/// A condition outside a script, as the control surface sends one.
pub fn condition(value: &Value) -> Result<Cond, String> {
    acts::condition::parse(value, "condition")
}

/// A target a step names: `[x, z]`, a name (`house_door`, `room_door`,
/// `player`), or an object with `npc`, `fixture`, `point` or `gatherable`.
fn target(value: &Value, at: &str) -> Result<Target, String> {
    acts::target(value, at)
}

/// The fields of one object, read by the hub's reader: each read at most
/// once, and [`Fields::finish`] refuses any field no reader took.
fn fields_of<'a>(value: &'a Value, at: &str) -> Result<Fields<'a>, String> {
    let map = value.as_object().ok_or_else(|| format!("{at}: expected an object"))?;
    Ok(Fields::new(map, at))
}
