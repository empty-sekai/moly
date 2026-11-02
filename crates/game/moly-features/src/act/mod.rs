//! The action registry: the named actions a script, a plugin, a sequence or
//! any other program of the game may ask for, and the runs of them.
//!
//! An action is registered by name with the crate that owns its behaviour
//! ([`ActionKind`]): the steps a client drives the game with are a script's
//! vocabulary, not the script runner's own. A step is parsed into the specs
//! it stands for (a macro stands for several, in order), started against the
//! world into a run, and then polled once a frame until it says it is done
//! or failed; a run that is stopped, abandoned or whose lease was taken is
//! stopped once ([`ActionRun::stop`]) and never polled again.
//!
//! Programs keep what is theirs either side of an action here: the
//! characters it took ([`Taken`]), the targets its steps name ([`Target`])
//! and the conditions they wait on ([`condition`]). The leases the actions
//! hold are the hub's ([`crate::lease`]), so two programs never take one
//! output from each other by accident.

pub mod condition;
pub mod hand;
mod registration;
pub mod site;
mod taken;
pub mod targets;
pub mod ui_view;
pub mod walk;

use std::sync::Arc;

use bevy::prelude::*;
use serde_json::{Map, Value};

use crate::lease::Holder;
use moly_law::action_button::ButtonType;

pub use registration::{Kinds, Step};
pub use taken::{Activities, Taken};

pub use condition::{Cond, SiteKindName};

/// A kind of action, registered under its name: how a step names it and
/// the fields the step may give.
pub trait ActionKind: Send + Sync + 'static {
    /// The step name (`companion.walk_to`).
    fn name(&self) -> &'static str;
    /// Whether the action writes the hand: it then needs the hand lease.
    fn hand(&self) -> bool {
        false
    }
    /// Checks the step's own fields (its common fields are removed before
    /// this is asked) and gives the specs it stands for; a macro gives
    /// several.
    fn parse(&self, fields: &Map<String, Value>, at: &str) -> Result<Vec<Arc<dyn ActionSpec>>, String>;
}

/// A checked action: the kind it came from, with the step's own fields read.
pub trait ActionSpec: Send + Sync + std::fmt::Debug + 'static {
    /// The step name.
    fn name(&self) -> &str;
    /// Whether the action writes the hand: the runner takes the hand lease
    /// before it starts and lets it go when it ends.
    fn hand(&self) -> bool;
    /// The timeout a step of this action takes when the script gives none.
    fn default_timeout(&self) -> Option<f32> {
        None
    }
    /// The timeout this step works out from its own fields, when neither the
    /// step nor the script gives one (`npc.face`'s turn length).
    fn timeout(&self) -> Option<f32> {
        None
    }
    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String>;
}

/// How an action ended on a poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActPoll {
    /// The action runs on; the note says how far.
    Running(String),
    /// The action ended; the note says what it did.
    Done(String),
    /// The action failed; the note names the refusal.
    Failed(String),
}

impl ActPoll {
    /// The poll as a result, `None` while the action runs on.
    pub fn result(self) -> Option<Result<String, String>> {
        match self {
            Self::Running(_) => None,
            Self::Done(note) => Some(Ok(note)),
            Self::Failed(note) => Some(Err(note)),
        }
    }
}

/// A running action.
pub trait ActionRun: Send + Sync + 'static {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll;
    /// Ends the action without finishing it: a stop, an abort, a failure, a
    /// timeout or a lease taken from it.
    fn stop(&mut self, world: &mut World, ctx: &mut ActCtx);
}

/// What an action's spec and run are given besides the world: the holder
/// they run for, the step's path in the log, the world's clock, and the
/// cards of the track the step runs in.
pub struct ActCtx {
    /// The holder the action runs for: the name it takes its leases under.
    pub holder: Holder,
    /// The action step's ticket: names the run of the characters it takes
    /// ([`Taken`]) and the lease of the finger it leans on ([`hand`]).
    pub ticket: u64,
    /// The step's path in the log.
    pub path: String,
    /// The world's clock.
    pub now: f32,
    /// The cards the step leaves for the steps after it.
    pub card: Card,
}

impl ActCtx {
    pub fn new(holder: Holder, ticket: u64, path: impl Into<String>, now: f32) -> Self {
        Self { holder, ticket, path: path.into(), now, card: Card::new() }
    }
}

/// The cards a step leaves for the steps after it in the same track, in the
/// order they were laid.
#[derive(Clone, Debug, Default)]
pub struct Card {
    laid: Vec<Leaving>,
}

impl Card {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cards `leavings`, in order.
    pub fn of(leavings: Vec<Leaving>) -> Self {
        Self { laid: leavings }
    }

    pub fn lay(&mut self, leaving: Leaving) {
        self.laid.push(leaving);
    }

    /// The first card of the kind `T` reads, taken out of the cards.
    pub fn take<T: FromLeaving>(&mut self) -> Option<T> {
        let index = self.laid.iter().position(|leaving| T::of(leaving).is_some())?;
        let leaving = self.laid.remove(index);
        T::of(&leaving)
    }

    /// The cards, in order.
    pub fn all(&self) -> &[Leaving] {
        &self.laid
    }
}

/// What a finished step leaves for the steps after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Leaving {
    /// The preview record of a furniture activity the step began, ended by
    /// its ticket.
    Activity(u64),
    /// A character the step handed back to its AI with a talk to open.
    Talk(Entity),
}

/// A kind of card a step reads back out of its track's cards.
pub trait FromLeaving: Sized {
    fn of(leaving: &Leaving) -> Option<Self>;
}

impl FromLeaving for u64 {
    fn of(leaving: &Leaving) -> Option<Self> {
        match leaving {
            Leaving::Activity(ticket) => Some(*ticket),
            Leaving::Talk(_) => None,
        }
    }
}

impl FromLeaving for Entity {
    fn of(leaving: &Leaving) -> Option<Self> {
        match leaving {
            Leaving::Talk(entity) => Some(*entity),
            Leaving::Activity(_) => None,
        }
    }
}

/// The registered actions, by name.
#[derive(Resource, Default)]
pub struct Actions {
    kinds: std::collections::BTreeMap<&'static str, Arc<dyn ActionKind>>,
}

impl Actions {
    pub fn register(&mut self, kind: impl ActionKind) {
        self.kinds.insert(kind.name(), Arc::new(kind));
    }

    /// Registers the steps `kinds` gave.
    pub fn register_kinds(&mut self, kinds: Kinds) {
        registration::register_kinds(self, kinds);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn ActionKind>> {
        self.kinds.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.kinds.keys().copied()
    }
}

pub use condition::{condition, eval};

/// Registers the hub's own state: the actions, the leases, the camera's held
/// pose, the characters the actions took and the places the domains report
/// for their targets. A program that runs actions installs this before it
/// registers any.
pub fn install(app: &mut App) {
    app.init_resource::<Actions>()
        .init_resource::<crate::lease::Leases>()
        .init_resource::<hand::Grip>()
        .init_resource::<Taken>()
        .init_resource::<Activities>()
        .init_resource::<targets::Gatherables>();
    crate::camera_hold::install(app);
}

/// Parses one action step, `{ "do": name, ...fields }` with its common fields
/// already removed, into the specs it stands for; `at` names the step in a
/// refusal.
pub fn parse(world: &World, name: &str, fields: &Map<String, Value>, at: &str) -> Result<Vec<Arc<dyn ActionSpec>>, String> {
    let Some(actions) = world.get_resource::<Actions>() else {
        return Err(format!("{at}: no actions are registered"));
    };
    let Some(kind) = actions.get(name) else {
        return Err(format!(
            "{at}: unknown action {name}; the actions are {}",
            actions.names().collect::<Vec<_>>().join(", ")
        ));
    };
    kind.parse(fields, at)
}

/// The fields of one step, each read at most once; [`Fields::finish`]
/// refuses any field no reader took. A missing field is named, a value of
/// the wrong kind is named with the kind it takes.
pub struct Fields<'a> {
    map: &'a Map<String, Value>,
    used: Vec<&'a str>,
    at: String,
}

impl<'a> Fields<'a> {
    pub fn new(map: &'a Map<String, Value>, at: &str) -> Self {
        Self { map, used: Vec::new(), at: at.to_owned() }
    }

    pub fn at(&self) -> &str {
        &self.at
    }

    pub fn map(&self) -> &'a Map<String, Value> {
        self.map
    }

    pub fn opt(&mut self, key: &'a str) -> Option<&'a Value> {
        let value = self.map.get(key)?;
        self.used.push(key);
        Some(value)
    }

    /// Every field not read yet, now counted as read.
    pub fn rest(&mut self) -> Map<String, Value> {
        let rest: Map<String, Value> = self
            .map
            .iter()
            .filter(|(key, _)| !self.used.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.used.extend(self.map.keys().map(String::as_str));
        rest
    }

    pub fn req(&mut self, key: &'a str) -> Result<&'a Value, String> {
        let at = self.at.clone();
        self.opt(key).ok_or_else(|| format!("{at}: missing field {key}"))
    }

    pub fn req_str(&mut self, key: &'a str) -> Result<&'a str, String> {
        let at = self.at.clone();
        self.req(key)?.as_str().ok_or_else(|| format!("{at}: {key} must be a string"))
    }

    pub fn opt_str(&mut self, key: &'a str) -> Result<Option<&'a str>, String> {
        let at = self.at.clone();
        self.opt(key).map(|value| value.as_str().ok_or_else(|| format!("{at}: {key} must be a string"))).transpose()
    }

    pub fn opt_bool(&mut self, key: &'a str) -> Result<Option<bool>, String> {
        let at = self.at.clone();
        self.opt(key).map(|value| value.as_bool().ok_or_else(|| format!("{at}: {key} must be true or false"))).transpose()
    }

    pub fn req_bool(&mut self, key: &'a str) -> Result<bool, String> {
        let at = self.at.clone();
        self.opt_bool(key)?.ok_or_else(|| format!("{at}: missing field {key}"))
    }

    pub fn opt_f32(&mut self, key: &'a str) -> Result<Option<f32>, String> {
        let at = self.at.clone();
        self.opt(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite() && *number >= 0.0)
                    .map(|number| number as f32)
                    .ok_or_else(|| format!("{at}: {key} must be a number of at least 0"))
            })
            .transpose()
    }

    pub fn req_f32(&mut self, key: &'a str) -> Result<f32, String> {
        let at = self.at.clone();
        self.opt_f32(key)?.ok_or_else(|| format!("{at}: missing field {key}"))
    }

    pub fn req_u32(&mut self, key: &'a str) -> Result<u32, String> {
        let at = self.at.clone();
        self.req(key)?
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("{at}: {key} must be a whole number of at least 0"))
    }

    pub fn opt_u32(&mut self, key: &'a str) -> Result<Option<u32>, String> {
        let at = self.at.clone();
        self.opt(key)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| format!("{at}: {key} must be a whole number of at least 0"))
            })
            .transpose()
    }

    pub fn req_i32(&mut self, key: &'a str) -> Result<i32, String> {
        let at = self.at.clone();
        self.opt_i32(key)?.ok_or_else(|| format!("{at}: missing field {key}"))
    }

    pub fn opt_i32(&mut self, key: &'a str) -> Result<Option<i32>, String> {
        let at = self.at.clone();
        self.opt(key)
            .map(|value| {
                value
                    .as_i64()
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or_else(|| format!("{at}: {key} must be a whole number"))
            })
            .transpose()
    }

    pub fn finish(self) -> Result<(), String> {
        match self.map.keys().find(|key| !self.used.contains(&key.as_str())) {
            Some(key) => Err(format!("{}: unknown field {key}", self.at)),
            None => Ok(()),
        }
    }
}

/// A target named by a step's fields: `[x, z]`, one of the words
/// (`house_door`, `room_door`, `player`, `workbench`), or an object with
/// `npc`, `fixture` (placement UID string or master id), `point` or
/// `gatherable` and an optional `offset` `[dx, dz]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub what: TargetKind,
    /// Metres added on x and z after the place is resolved.
    pub offset: [f32; 2],
}

impl Target {
    /// The place itself, with no offset.
    pub fn at(what: TargetKind) -> Self {
        Self { what, offset: [0.0; 2] }
    }
}

/// The place a target names, resolved every frame.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetKind {
    /// World x and z.
    Point([f32; 2]),
    /// The centre of the placed house's entry box.
    HouseDoor,
    /// The room door's sensor.
    RoomDoor,
    Player,
    Npc(u32),
    /// A placed fixture by its placement UID.
    FixtureUid(String),
    /// The placed fixture of this master id nearest the player.
    FixtureId(i32),
    /// A harvest object of the loaded site by its UID.
    Gatherable(u64),
    /// The placed workbench (a craft tool system fixture) nearest the player.
    Workbench,
}

/// A placed fixture: by placement UID, or the instance of a master id
/// nearest the one who goes to it.
#[derive(Clone, Debug, PartialEq)]
pub enum FixtureRef {
    Uid(String),
    Id(i32),
}

/// A gatherable (a harvest object of the loaded site): by its UID, or the
/// nearest one not yet harvested, of one kind when `kind` names it.
#[derive(Clone, Debug, PartialEq)]
pub enum GatherRef {
    Uid(u64),
    Nearest { kind: Option<String> },
}

/// The target a step's `target` field names; `at` names the step.
pub fn target(value: &Value, at: &str) -> Result<Target, String> {
    let at = format!("{at}.target");
    if value.is_array() {
        return Ok(Target::at(TargetKind::Point(pair(value, &at)?)));
    }
    if let Some(name) = value.as_str() {
        let what = match name {
            "house_door" => TargetKind::HouseDoor,
            "room_door" => TargetKind::RoomDoor,
            "player" => TargetKind::Player,
            "workbench" => TargetKind::Workbench,
            other => return Err(format!("{at}: unknown target {other}; use house_door, room_door, player, workbench, [x, z] or an object")),
        };
        return Ok(Target::at(what));
    }
    let object = value.as_object().ok_or_else(|| format!("{at}: expected an object"))?;
    let mut fields = Fields::new(object, &at);
    let offset = match fields.opt("offset") {
        Some(offset) => pair(offset, &format!("{at}.offset"))?,
        None => [0.0; 2],
    };
    let what = match (fields.opt("npc"), fields.opt("fixture"), fields.opt("point"), fields.opt("gatherable")) {
        (Some(_), None, None, None) => TargetKind::Npc(fields.req_u32("npc")?),
        (None, Some(fixture), None, None) => match fixture_ref(fixture, &at)? {
            FixtureRef::Uid(uid) => TargetKind::FixtureUid(uid),
            FixtureRef::Id(id) => TargetKind::FixtureId(id),
        },
        (None, None, Some(point), None) => TargetKind::Point(pair(point, &format!("{at}.point"))?),
        (None, None, None, Some(uid)) => TargetKind::Gatherable(
            uid.as_u64().ok_or_else(|| format!("{at}: gatherable is a harvest object UID"))?,
        ),
        _ => return Err(format!("{at}: give exactly one of npc, fixture, point or gatherable")),
    };
    fields.finish()?;
    Ok(Target { what, offset })
}

pub fn fixture_ref(value: &Value, at: &str) -> Result<FixtureRef, String> {
    match (value.as_str(), value.as_i64()) {
        (Some(uid), _) => Ok(FixtureRef::Uid(uid.to_owned())),
        (_, Some(id)) => Ok(FixtureRef::Id(i32::try_from(id).map_err(|_| format!("{at}: fixture id out of range"))?)),
        _ => Err(format!("{at}: fixture is a placement UID string or a master id")),
    }
}

pub fn pair(value: &Value, at: &str) -> Result<[f32; 2], String> {
    match value.as_array().map(Vec::as_slice) {
        Some([a, b]) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => Ok([a as f32, b as f32]),
            _ => Err(format!("{at}: expected two numbers")),
        },
        _ => Err(format!("{at}: expected [x, z]")),
    }
}

/// A world point: `[x, y, z]`.
pub fn triple(value: &Value, at: &str) -> Result<[f32; 3], String> {
    match value.as_array().map(Vec::as_slice) {
        Some([x, y, z]) => match (x.as_f64(), y.as_f64(), z.as_f64()) {
            (Some(x), Some(y), Some(z)) if x.is_finite() && y.is_finite() && z.is_finite() => Ok([x as f32, y as f32, z as f32]),
            _ => Err(format!("{at}: expected three numbers")),
        },
        _ => Err(format!("{at}: expected [x, y, z]")),
    }
}

/// A cell: two whole numbers in the grid's own range.
pub fn cell_pair(value: &Value, at: &str) -> Result<[i8; 2], String> {
    let Some([x, z]) = value.as_array().map(Vec::as_slice) else {
        return Err(format!("{at}: at is a cell [x, z]"));
    };
    let mut out = [0i8; 2];
    for (slot, value) in out.iter_mut().zip([x, z]) {
        *slot = value
            .as_i64()
            .and_then(|number| i8::try_from(number).ok())
            .ok_or_else(|| format!("{at}: at is two whole numbers from -128 to 127"))?;
    }
    Ok(out)
}

/// A layout tile: three whole numbers in the grid's own range.
pub fn tile(value: &Value, at: &str) -> Result<[i8; 3], String> {
    let Some([x, y, z]) = value.as_array().map(Vec::as_slice) else {
        return Err(format!("{at}: expected [x, y, z]"));
    };
    let mut out = [0i8; 3];
    for (slot, value) in out.iter_mut().zip([x, y, z]) {
        *slot = value
            .as_i64()
            .and_then(|number| i8::try_from(number).ok())
            .ok_or_else(|| format!("{at}: expected three whole numbers from -128 to 127"))?;
    }
    Ok(out)
}

/// The character states a condition may name.
pub const NPC_ACTIONS: &[&str] = &[
    "Idle", "AutoMove", "Rotate", "Talk", "Walk", "Rest", "None", "Tweet", "Greeting",
    "FixtureAction", "FixtureActionIdle", "ChangeSite", "MainSomeFixtureAction",
    "SomeFixtureAction", "Communication",
];

/// The action buttons a step may press, by their type names, with the button
/// of each.
pub const BUTTONS: &[ButtonType] = &[
    ButtonType::Talk,
    ButtonType::GimmickFixture,
    ButtonType::TimelineFixture,
    ButtonType::HouseEntry,
    ButtonType::OpenChest,
    ButtonType::GateEdit,
    ButtonType::OpenCraftTool,
    ButtonType::OpenMysekaiInfo,
    ButtonType::OpenMysekaiBgmSelect,
    ButtonType::OpenMysekaiConvert,
    ButtonType::OpenOtherMysekai,
    ButtonType::OpenAvatarDressUp,
    ButtonType::OpenSecretShop,
    ButtonType::GoHomeSite,
    ButtonType::LeaveMysekai,
    ButtonType::Sketch,
    ButtonType::Delivery,
    ButtonType::BirthdayCutScene,
    ButtonType::GateInvitation,
    ButtonType::OpenMysekaiCompetition,
    ButtonType::DeliveryInformation,
];

/// The button a step names, or a refusal naming the buttons there are.
pub fn button(name: &str, at: &str) -> Result<ButtonType, String> {
    BUTTONS
        .iter()
        .copied()
        .find(|button| format!("{button:?}") == name)
        .ok_or_else(|| {
            let names: Vec<String> = BUTTONS.iter().map(|button| format!("{button:?}")).collect();
            format!("{at}: unknown button {name:?}; the buttons are {}", names.join(", "))
        })
}

