//! The script format and its validation. A script is checked whole before
//! any step runs: an unknown action, an unknown field or a value of the wrong
//! kind is refused with the step's path and the field's name.

use std::sync::Arc;

use moly_law::action_button::ButtonType;
use serde_json::{Map, Value};

/// A validated script.
#[derive(Clone, Debug)]
pub(crate) struct Script {
    pub(crate) name: String,
    pub(crate) steps: Arc<[Step]>,
    /// Native runs end the application when the script ends: success when it
    /// completed, failure when a step failed.
    pub(crate) exit: bool,
}

/// One step with its common fields.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) node: Node,
    /// The step's name in the log: its `label`, or its action.
    pub(crate) label: String,
    /// Seconds the step may run before it fails.
    pub(crate) timeout: Option<f32>,
    /// A failure is logged and the script goes on.
    pub(crate) optional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Join {
    /// The block ends when every track has ended.
    All,
    /// The block ends when one track has ended; the others are stopped.
    Any,
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Wait { secs: f32 },
    WaitUntil { until: Cond },
    Assert { that: Cond },
    Log { text: String },
    Capture,
    Parallel { tracks: Vec<Arc<[Step]>>, join: Join },
    Repeat { times: u32, steps: Arc<[Step]> },
    WalkTo { to: Target, within: f32 },
    Interact { target: Target, button: ButtonType, stop: f32 },
    Tap { at: [f32; 2], fraction: bool },
    /// The joystick held at a stick ratio (x right, y up, within the unit
    /// circle) for `secs`.
    Stick { stick: [f32; 2], secs: f32 },
    Release,
    /// Logs the player's position and each target's.
    Where { of: Vec<Target> },
    NpcTake { unit: u32 },
    /// `rests` keeps the walk law's rest stops on the route.
    NpcWalkTo { unit: u32, to: Target, within: f32, rests: bool },
    NpcRelease { unit: u32 },
    /// Turns a taken, standing character toward a target over `secs`.
    NpcFace { unit: u32, to: Target, secs: f32 },
    /// Shows an emoticon over a character for `secs`.
    NpcEmote { unit: u32, name: String, secs: f32, use_root: bool },
    /// Plays one of the character's authored no-talk activities at a placed
    /// fixture: `activity` picks the action by id and `variant` (1-based)
    /// its timeline, the first of each by default; `rests` keeps the walk
    /// law's rest stops on the way there.
    NpcUse { unit: u32, fixture: FixtureRef, activity: Option<i32>, variant: Option<u32>, rests: bool },
    /// Shows a tweet of the master over a taken character for `secs`.
    NpcTweet { unit: u32, tweet: i32, secs: f32 },
    Weather { phenomenon: i32 },
    SiteSwitch { site: String },
}

/// A placed fixture: by placement UID, or the instance of a master id
/// nearest the one who goes to it.
#[derive(Clone, Debug)]
pub(crate) enum FixtureRef {
    Uid(String),
    Id(i32),
}

/// A place in the world, resolved every frame.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub(crate) what: TargetKind,
    /// Metres added on x and z after the place is resolved.
    pub(crate) offset: [f32; 2],
}

#[derive(Clone, Debug)]
pub(crate) enum TargetKind {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SiteKindName {
    Home,
    Room,
    Harvest,
    Delivery,
}

#[derive(Clone, Debug)]
pub(crate) enum Cond {
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    /// The active site's type (`home_site`, `first_floor`, ...).
    SiteIs(String),
    SiteKind(SiteKindName),
    /// A site move (cannon or door) is in progress.
    SiteMoving,
    /// The entry sequence has given the player control.
    EntryDone,
    DialogShown,
    /// A player or NPC conversation is playing.
    TalkActive,
    ButtonHead(ButtonType),
    ButtonStacked(ButtonType),
    PlayerNear { to: Target, within: f32 },
    NpcNear { unit: u32, to: Target, within: f32 },
    PhenomenonIs(i32),
    /// The character's action state (`Idle`, `Walk`, `Rest`, `Talk`, ...).
    NpcAction { unit: u32, is: String },
}

const ACTIONS: &[&str] = &[
    "wait", "wait_until", "assert", "log", "where", "capture", "parallel", "repeat",
    "player.walk_to", "player.interact", "player.stick", "player.release", "player.talk_to", "tap",
    "npc.take", "npc.walk_to", "npc.face", "npc.emote", "npc.use", "npc.tweet", "npc.release",
    "weather.set", "site.switch", "site.enter_room", "site.go_home",
];

/// Parses a script document.
pub(crate) fn parse(document: &Value) -> Result<Script, String> {
    let mut top = Fields::new(document, "script")?;
    let name = top.opt_str("name")?.unwrap_or("script").to_owned();
    let exit = top.opt_bool("exit")?.unwrap_or(false);
    let steps = steps(top.req("steps")?, "steps")?;
    top.finish()?;
    if steps.is_empty() {
        return Err("script: steps is empty".into());
    }
    Ok(Script { name, steps, exit })
}

fn steps(value: &Value, at: &str) -> Result<Arc<[Step]>, String> {
    let list = value.as_array().ok_or_else(|| format!("{at}: expected a list of steps"))?;
    let mut out = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        out.extend(step(item, &format!("{at}[{index}]"))?);
    }
    Ok(out.into())
}

/// One step; a macro expands into several.
fn step(value: &Value, at: &str) -> Result<Vec<Step>, String> {
    let mut fields = Fields::new(value, at)?;
    let action = fields.req_str("do")?.to_owned();
    let at = format!("{at} ({action})");
    let label = fields.opt_str("label")?.map(str::to_owned);
    let timeout = fields.opt_f32("timeout")?;
    let optional = fields.opt_bool("optional")?.unwrap_or(false);
    let one = |node: Node, default_timeout: Option<f32>| Step {
        node,
        label: label.clone().unwrap_or_else(|| action.clone()),
        timeout: timeout.or(default_timeout),
        optional,
    };
    let steps = match action.as_str() {
        "wait" => vec![one(Node::Wait { secs: fields.req_f32("secs")? }, None)],
        "wait_until" => vec![one(Node::WaitUntil { until: cond(fields.req("until")?, &at)? }, Some(30.0))],
        "assert" => vec![one(Node::Assert { that: cond(fields.req("that")?, &at)? }, None)],
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
                .map(|(index, track)| steps(track, &format!("{at}.tracks[{index}]")))
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
            vec![one(Node::Repeat { times, steps: steps(fields.req("steps")?, &format!("{at}.steps"))? }, None)]
        }
        "player.walk_to" => vec![one(
            Node::WalkTo { to: target(fields.req("to")?, &at)?, within: fields.opt_f32("within")?.unwrap_or(0.3) },
            Some(60.0),
        )],
        "player.interact" => vec![one(
            Node::Interact {
                target: target(fields.req("target")?, &at)?,
                button: button(fields.req_str("button")?, &at)?,
                stop: fields.opt_f32("stop")?.unwrap_or(0.2),
            },
            Some(60.0),
        )],
        "player.talk_to" => {
            let unit = fields.req_u32("unit")?;
            vec![
                one(
                    Node::Interact {
                        target: Target { what: TargetKind::Npc(unit), offset: [0.0; 2] },
                        button: ButtonType::Talk,
                        stop: fields.opt_f32("stop")?.unwrap_or(0.8),
                    },
                    Some(60.0),
                ),
                one(Node::WaitUntil { until: Cond::TalkActive }, Some(10.0)),
            ]
        }
        "player.stick" => {
            let stick = pair(fields.req("stick")?, &format!("{at}.stick"))?;
            if stick[0].hypot(stick[1]) > 1.0 + 1e-4 {
                return Err(format!("{at}: stick is a ratio within the unit circle, got {stick:?}"));
            }
            let secs = fields.req_f32("secs")?;
            vec![one(Node::Stick { stick, secs }, None)]
        }
        "player.release" => vec![one(Node::Release, None)],
        "tap" => {
            let (at_value, fraction) = match (fields.opt("at"), fields.opt("at_fraction")) {
                (Some(value), None) => (value, false),
                (None, Some(value)) => (value, true),
                _ => return Err(format!("{at}: give exactly one of at (logical pixels) or at_fraction (0..1 of the window)")),
            };
            vec![one(Node::Tap { at: pair(at_value, &format!("{at}.at"))?, fraction }, None)]
        }
        "npc.take" => vec![one(Node::NpcTake { unit: fields.req_u32("unit")? }, None)],
        "npc.walk_to" => vec![one(
            Node::NpcWalkTo {
                unit: fields.req_u32("unit")?,
                to: target(fields.req("to")?, &at)?,
                within: fields.opt_f32("within")?.unwrap_or(0.3),
                rests: fields.opt_bool("rests")?.unwrap_or(false),
            },
            Some(60.0),
        )],
        "npc.release" => vec![one(Node::NpcRelease { unit: fields.req_u32("unit")? }, None)],
        "npc.face" => {
            let unit = fields.req_u32("unit")?;
            let to = target(fields.req("to")?, &at)?;
            let secs = fields.opt_f32("secs")?.unwrap_or(1.0);
            if !(secs >= 0.0) {
                return Err(format!("{at}: secs must not be negative"));
            }
            vec![one(Node::NpcFace { unit, to, secs }, Some(secs + 5.0))]
        }
        "npc.emote" => {
            let unit = fields.req_u32("unit")?;
            let name = fields.req_str("name")?.to_owned();
            let secs = fields.opt_f32("secs")?.unwrap_or(2.0);
            let use_root = fields.opt_bool("use_root")?.unwrap_or(false);
            vec![one(Node::NpcEmote { unit, name, secs, use_root }, None)]
        }
        "npc.use" => {
            let unit = fields.req_u32("unit")?;
            let fixture = fixture_ref(fields.req("fixture")?, &at)?;
            let activity = fields.opt_i32("activity")?;
            let variant = fields.opt_u32("variant")?;
            if variant == Some(0) {
                return Err(format!("{at}: variant counts from 1"));
            }
            let rests = fields.opt_bool("rests")?.unwrap_or(false);
            vec![one(Node::NpcUse { unit, fixture, activity, variant, rests }, Some(300.0))]
        }
        "npc.tweet" => {
            let unit = fields.req_u32("unit")?;
            let tweet = fields.req_i32("tweet")?;
            let secs = fields.opt_f32("secs")?.unwrap_or(3.0);
            vec![one(Node::NpcTweet { unit, tweet, secs }, None)]
        }
        "weather.set" => vec![one(Node::Weather { phenomenon: fields.req_i32("phenomenon")? }, None)],
        "site.switch" => vec![one(Node::SiteSwitch { site: fields.req_str("site")?.to_owned() }, Some(60.0))],
        // Through the house's door: walk to its entry box, press the entry
        // button, then wait for the move to start and to land in a room.
        "site.enter_room" => door_steps(&one, TargetKind::HouseDoor, ButtonType::HouseEntry, SiteKindName::Room),
        "site.go_home" => door_steps(&one, TargetKind::RoomDoor, ButtonType::GoHomeSite, SiteKindName::Home),
        other => {
            return Err(format!("{at}: unknown action {other}; the actions are {}", ACTIONS.join(", ")));
        }
    };
    fields.finish()?;
    Ok(steps)
}

fn door_steps(
    one: &dyn Fn(Node, Option<f32>) -> Step,
    door: TargetKind,
    button: ButtonType,
    lands: SiteKindName,
) -> Vec<Step> {
    vec![
        one(Node::Interact { target: Target { what: door, offset: [0.0; 2] }, button, stop: 0.2 }, Some(60.0)),
        one(Node::WaitUntil { until: Cond::SiteMoving }, Some(5.0)),
        one(
            Node::WaitUntil {
                until: Cond::All(vec![Cond::Not(Box::new(Cond::SiteMoving)), Cond::SiteKind(lands)]),
            },
            Some(60.0),
        ),
    ]
}

/// A condition: a bare name, or an object with one key.
fn cond(value: &Value, at: &str) -> Result<Cond, String> {
    if let Some(name) = value.as_str() {
        return Ok(match name {
            "site_moving" => Cond::SiteMoving,
            "entry_done" => Cond::EntryDone,
            "dialog_shown" => Cond::DialogShown,
            "talk_active" => Cond::TalkActive,
            other => return Err(format!("{at}: unknown condition {other}")),
        });
    }
    let object = value.as_object().ok_or_else(|| format!("{at}: a condition is a name or an object with one key"))?;
    let mut entries = object.iter();
    let (Some((key, inner)), None) = (entries.next(), entries.next()) else {
        return Err(format!("{at}: a condition object has exactly one key"));
    };
    let at = format!("{at}.{key}");
    Ok(match key.as_str() {
        "all" | "any" => {
            let list = inner.as_array().ok_or_else(|| format!("{at}: expected a list of conditions"))?;
            let conds = list.iter().map(|item| cond(item, &at)).collect::<Result<Vec<_>, _>>()?;
            if key == "all" { Cond::All(conds) } else { Cond::Any(conds) }
        }
        "not" => Cond::Not(Box::new(cond(inner, &at)?)),
        "site_is" => Cond::SiteIs(inner.as_str().ok_or_else(|| format!("{at}: expected a site type"))?.to_owned()),
        "site_kind" => Cond::SiteKind(match inner.as_str() {
            Some("home") => SiteKindName::Home,
            Some("room") => SiteKindName::Room,
            Some("harvest") => SiteKindName::Harvest,
            Some("delivery") => SiteKindName::Delivery,
            _ => return Err(format!("{at}: expected home, room, harvest or delivery")),
        }),
        "button_head" => Cond::ButtonHead(button(inner.as_str().unwrap_or_default(), &at)?),
        "button_stacked" => Cond::ButtonStacked(button(inner.as_str().unwrap_or_default(), &at)?),
        "phenomenon_is" => Cond::PhenomenonIs(
            inner.as_i64().and_then(|v| i32::try_from(v).ok()).ok_or_else(|| format!("{at}: expected a phenomenon id"))?,
        ),
        "player_near" => {
            let mut fields = Fields::new(inner, &at)?;
            let near = Cond::PlayerNear { to: target(fields.req("to")?, &at)?, within: fields.opt_f32("within")?.unwrap_or(0.5) };
            fields.finish()?;
            near
        }
        "npc_action" => {
            let mut fields = Fields::new(inner, &at)?;
            let unit = fields.req_u32("unit")?;
            let is = fields.req_str("is")?.to_owned();
            if !NPC_ACTIONS.contains(&is.as_str()) {
                return Err(format!("{at}: unknown action state {is}; the states are {}", NPC_ACTIONS.join(", ")));
            }
            fields.finish()?;
            Cond::NpcAction { unit, is }
        }
        "npc_near" => {
            let mut fields = Fields::new(inner, &at)?;
            let near = Cond::NpcNear {
                unit: fields.req_u32("unit")?,
                to: target(fields.req("to")?, &at)?,
                within: fields.opt_f32("within")?.unwrap_or(0.5),
            };
            fields.finish()?;
            near
        }
        other => return Err(format!("{at}: unknown condition {other}")),
    })
}

/// A target: `[x, z]`, a name (`house_door`, `room_door`, `player`), or an
/// object with `npc`, `fixture` (placement UID string or master id) or
/// `point`, and an optional `offset` `[dx, dz]`.
fn target(value: &Value, at: &str) -> Result<Target, String> {
    let at = format!("{at}.target");
    if value.is_array() {
        return Ok(Target { what: TargetKind::Point(pair(value, &at)?), offset: [0.0; 2] });
    }
    if let Some(name) = value.as_str() {
        let what = match name {
            "house_door" => TargetKind::HouseDoor,
            "room_door" => TargetKind::RoomDoor,
            "player" => TargetKind::Player,
            other => return Err(format!("{at}: unknown target {other}; use house_door, room_door, player, [x, z] or an object")),
        };
        return Ok(Target { what, offset: [0.0; 2] });
    }
    let mut fields = Fields::new(value, &at)?;
    let offset = match fields.opt("offset") {
        Some(offset) => pair(offset, &format!("{at}.offset"))?,
        None => [0.0; 2],
    };
    let what = match (fields.opt("npc"), fields.opt("fixture"), fields.opt("point")) {
        (Some(_), None, None) => TargetKind::Npc(fields.req_u32("npc")?),
        (None, Some(fixture), None) => match fixture_ref(fixture, &at)? {
            FixtureRef::Uid(uid) => TargetKind::FixtureUid(uid),
            FixtureRef::Id(id) => TargetKind::FixtureId(id),
        },
        (None, None, Some(point)) => TargetKind::Point(pair(point, &format!("{at}.point"))?),
        _ => return Err(format!("{at}: give exactly one of npc, fixture or point")),
    };
    fields.finish()?;
    Ok(Target { what, offset })
}

fn fixture_ref(value: &Value, at: &str) -> Result<FixtureRef, String> {
    match (value.as_str(), value.as_i64()) {
        (Some(uid), _) => Ok(FixtureRef::Uid(uid.to_owned())),
        (_, Some(id)) => Ok(FixtureRef::Id(i32::try_from(id).map_err(|_| format!("{at}: fixture id out of range"))?)),
        _ => Err(format!("{at}: fixture is a placement UID string or a master id")),
    }
}

fn pair(value: &Value, at: &str) -> Result<[f32; 2], String> {
    match value.as_array().map(Vec::as_slice) {
        Some([a, b]) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => Ok([a as f32, b as f32]),
            _ => Err(format!("{at}: expected two numbers")),
        },
        _ => Err(format!("{at}: expected [x, z]")),
    }
}

/// The character action states a condition may name.
const NPC_ACTIONS: &[&str] = &[
    "Idle", "AutoMove", "Rotate", "Talk", "Walk", "Rest", "None", "Tweet", "Greeting",
    "FixtureAction", "FixtureActionIdle", "ChangeSite", "MainSomeFixtureAction",
    "SomeFixtureAction", "Communication",
];

/// The action buttons a script may press, by their type names.
const BUTTONS: &[ButtonType] = &[
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

fn button(name: &str, at: &str) -> Result<ButtonType, String> {
    BUTTONS
        .iter()
        .copied()
        .find(|button| format!("{button:?}") == name)
        .ok_or_else(|| {
            let names: Vec<String> = BUTTONS.iter().map(|button| format!("{button:?}")).collect();
            format!("{at}: unknown button {name:?}; the buttons are {}", names.join(", "))
        })
}

/// The fields of one object, each read at most once; [`Fields::finish`]
/// refuses any field no reader took.
struct Fields<'a> {
    map: &'a Map<String, Value>,
    used: Vec<&'a str>,
    at: String,
}

impl<'a> Fields<'a> {
    fn new(value: &'a Value, at: &str) -> Result<Self, String> {
        let map = value.as_object().ok_or_else(|| format!("{at}: expected an object"))?;
        Ok(Self { map, used: Vec::new(), at: at.to_owned() })
    }

    fn opt(&mut self, key: &'a str) -> Option<&'a Value> {
        let value = self.map.get(key)?;
        self.used.push(key);
        Some(value)
    }

    fn req(&mut self, key: &'a str) -> Result<&'a Value, String> {
        let at = self.at.clone();
        self.opt(key).ok_or_else(|| format!("{at}: missing field {key}"))
    }

    fn req_str(&mut self, key: &'a str) -> Result<&'a str, String> {
        let at = self.at.clone();
        self.req(key)?.as_str().ok_or_else(|| format!("{at}: {key} must be a string"))
    }

    fn opt_str(&mut self, key: &'a str) -> Result<Option<&'a str>, String> {
        let at = self.at.clone();
        self.opt(key).map(|value| value.as_str().ok_or_else(|| format!("{at}: {key} must be a string"))).transpose()
    }

    fn opt_bool(&mut self, key: &'a str) -> Result<Option<bool>, String> {
        let at = self.at.clone();
        self.opt(key).map(|value| value.as_bool().ok_or_else(|| format!("{at}: {key} must be true or false"))).transpose()
    }

    fn opt_f32(&mut self, key: &'a str) -> Result<Option<f32>, String> {
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

    fn req_f32(&mut self, key: &'a str) -> Result<f32, String> {
        let at = self.at.clone();
        self.opt_f32(key)?.ok_or_else(|| format!("{at}: missing field {key}"))
    }

    fn req_u32(&mut self, key: &'a str) -> Result<u32, String> {
        let at = self.at.clone();
        self.req(key)?
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("{at}: {key} must be a whole number of at least 0"))
    }

    fn req_i32(&mut self, key: &'a str) -> Result<i32, String> {
        let at = self.at.clone();
        self.opt_i32(key)?.ok_or_else(|| format!("{at}: missing field {key}"))
    }

    fn opt_i32(&mut self, key: &'a str) -> Result<Option<i32>, String> {
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

    fn opt_u32(&mut self, key: &'a str) -> Result<Option<u32>, String> {
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

    fn finish(self) -> Result<(), String> {
        match self.map.keys().find(|key| !self.used.contains(&key.as_str())) {
            Some(key) => Err(format!("{}: unknown field {key}", self.at)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_door_macro_expands_into_its_three_steps() {
        let script = parse(&json!({ "steps": [{ "do": "site.enter_room" }] })).unwrap();
        assert_eq!(script.steps.len(), 3);
        assert!(matches!(script.steps[0].node, Node::Interact { button: ButtonType::HouseEntry, .. }));
        assert!(matches!(script.steps[1].node, Node::WaitUntil { until: Cond::SiteMoving }));
    }

    #[test]
    fn unknown_actions_fields_and_buttons_are_refused_by_name() {
        let error = parse(&json!({ "steps": [{ "do": "player.fly" }] })).unwrap_err();
        assert!(error.contains("unknown action player.fly"), "{error}");
        let error = parse(&json!({ "steps": [{ "do": "wait", "secs": 1, "sec": 2 }] })).unwrap_err();
        assert!(error.contains("unknown field sec"), "{error}");
        let error = parse(&json!({ "steps": [{ "do": "player.interact", "target": "house_door", "button": "Door" }] }))
            .unwrap_err();
        assert!(error.contains("unknown button \"Door\""), "{error}");
    }

    #[test]
    fn nested_blocks_and_targets_parse() {
        let script = parse(&json!({
            "name": "two walkers",
            "steps": [
                { "do": "parallel", "join": "any", "tracks": [
                    [{ "do": "player.walk_to", "to": [1.0, 2.0] }],
                    [{ "do": "npc.walk_to", "unit": 3, "to": { "npc": 4, "offset": [0.5, 0.0] } }]
                ] },
                { "do": "repeat", "times": 2, "steps": [{ "do": "wait", "secs": 0.5 }] },
                { "do": "wait_until", "until": { "all": ["entry_done", { "not": "site_moving" }] } }
            ]
        }))
        .unwrap();
        assert_eq!(script.name, "two walkers");
        let Node::Parallel { tracks, join } = &script.steps[0].node else { panic!() };
        assert_eq!((tracks.len(), *join), (2, Join::Any));
        let Node::NpcWalkTo { to, .. } = &tracks[1][0].node else { panic!() };
        assert!(matches!(to.what, TargetKind::Npc(4)));
        assert_eq!(to.offset, [0.5, 0.0]);
    }

    #[test]
    fn fixture_activities_take_a_uid_or_a_master_id() {
        let script = parse(&json!({ "steps": [
            { "do": "npc.use", "unit": 2, "fixture": 1053, "activity": 7, "variant": 2 },
            { "do": "npc.use", "unit": 2, "fixture": "p-12" },
            { "do": "npc.tweet", "unit": 2, "tweet": 301 }
        ] }))
        .unwrap();
        let Node::NpcUse { fixture: FixtureRef::Id(1053), activity: Some(7), variant: Some(2), .. } = &script.steps[0].node
        else {
            panic!("{:?}", script.steps[0].node)
        };
        assert!(matches!(&script.steps[1].node, Node::NpcUse { fixture: FixtureRef::Uid(uid), activity: None, .. } if uid == "p-12"));
        assert_eq!(script.steps[1].timeout, Some(300.0));
        assert!(matches!(&script.steps[2].node, Node::NpcTweet { tweet: 301, secs, .. } if *secs == 3.0));
        let error = parse(&json!({ "steps": [{ "do": "npc.use", "unit": 2, "fixture": 1, "variant": 0 }] })).unwrap_err();
        assert!(error.contains("variant counts from 1"), "{error}");
    }
}
