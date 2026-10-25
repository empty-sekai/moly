//! The script format and its validation. A script is checked whole before
//! any step runs: an unknown action, an unknown field or a value of the wrong
//! kind is refused with the step's path and the field's name.

use std::sync::Arc;

use moly_law::action_button::ButtonType;
use serde_json::{Map, Value};

/// A validated script.
#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    pub steps: Arc<[Step]>,
    /// Native runs end the application when the script ends: success when it
    /// completed, failure when a step failed.
    pub exit: bool,
}

/// One step with its common fields.
#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// The block ends when every track has ended.
    All,
    /// The block ends when one track has ended; the others are stopped.
    Any,
}

#[derive(Clone, Debug)]
pub enum Node {
    Wait { secs: f32 },
    WaitUntil { until: Cond },
    Assert { that: Cond },
    Log { text: String },
    Capture,
    Parallel { tracks: Vec<Arc<[Step]>>, join: Join },
    Repeat { times: u32, steps: Arc<[Step]> },
    WalkTo { to: Target, within: f32 },
    Interact { target: Target, button: ButtonType, stop: f32 },
    /// Harvests one gatherable through the harvest button and, with
    /// `collect`, walks over the drops it leaves.
    Gather { what: GatherRef, collect: bool },
    /// Crafts one blueprint on the open craft screen: its choice, the apply
    /// button, the confirm's OK and the result's close.
    Craft { blueprint: i32 },
    /// Puts one owned fixture through the layout editor's own command
    /// sequence: enter, select its catalog row, turn it, move it to the
    /// tile, decide it and save.
    LayoutPlace { fixture: i32, texture: Option<u32>, center: [i8; 3], rotate: u32, decide: bool, save: bool },
    /// Presses the delivery button on a birthday party in session and holds
    /// it (`hold` seconds, or until the party has nothing left to deliver),
    /// then releases it: the delivery screen's own two requests.
    Deliver { party: Option<i32>, hold: Option<f32> },
    /// Taps the shown common dialog's OK (`ok`) or cancel button; a dialog
    /// of the learn-phenomenon family, which has no common button, is
    /// answered with the back key it takes.
    DialogAnswer { ok: bool },
    /// One press of the back key.
    BackKey,
    Tap { at: [f32; 2], fraction: bool },
    /// A tap at the centre of a node of a shown UI view.
    UiTap { view: String, path: String },
    /// The joystick held at a stick ratio (x right, y up, within the unit
    /// circle) for `secs`.
    Stick { stick: [f32; 2], secs: f32 },
    Release,
    /// The player's body: a character body by unit, or the avatar body
    /// (`None`).
    Wear { unit: Option<u32> },
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
    /// Hands a character to its AI with a talk to open on its next decision.
    NpcTalk { unit: u32, talk: i32 },
    /// Puts the companion of `unit` at a world point; the step ends once its
    /// body is in and it stands, or has walked to the point.
    CompanionSpawn { unit: u32, at: [f32; 3] },
    /// Walks the companion of `unit` to a world point, ending within `within`
    /// metres of it.
    CompanionWalkTo { unit: u32, to: [f32; 3], within: f32 },
    /// Turns the companion of `unit` to face a world point over `secs`
    /// seconds; `player` is the player's position when the step starts.
    CompanionFace { unit: u32, to: FaceTo, secs: f32 },
    /// Shows an emoticon over the companion of `unit` for `secs`.
    CompanionEmote { unit: u32, name: String, secs: f32, use_root: bool },
    /// Plays the motion-library motion `motion` on the companion of `unit`'s
    /// own body once: its start, loop and end segments, then its idle.
    CompanionMotion { unit: u32, motion: String },
    /// Sets the eye and the mouth pattern of the companion of `unit`, or
    /// returns its face to the unit's default (`reset`).
    CompanionExpression { unit: u32, eye: Option<String>, mouth: Option<String>, reset: bool },
    /// Shows a tweet balloon over the companion of `unit` for `secs`: the
    /// tweet master's text of `tweet`, or `text` as given.
    CompanionBalloon { unit: u32, text: Option<String>, tweet: Option<i32>, secs: f32 },
    /// Removes the companion of `unit`.
    CompanionDespawn { unit: u32 },
    /// Puts an owned fixture near a world point (by default in front of the
    /// companion) and has the companion of `unit` hammer it from a ghost to
    /// solid in `swings` hits.
    CompanionBuild { unit: u32, fixture: i32, texture: Option<u32>, near: Option<[f32; 3]>, swings: u32 },
    /// Has the companion of `unit` use a placed fixture: walk to the action
    /// point of its character's action there and play it.
    CompanionUse { unit: u32, target: UseTarget, activity: Option<i32>, variant: Option<u32> },
    /// Shows the emoticon `name` at a world point for `secs`.
    EffectPlay { name: String, at: [f32; 3], secs: f32 },
    /// A one-finger drag of `by` logical pixels (x right, y down) over
    /// `secs`, starting at `from` (fractions of the window).
    CameraDrag { by: [f32; 2], secs: f32, from: [f32; 2] },
    /// A wheel turn of `by` pixels (positive zooms in) spread over `secs`.
    CameraZoom { by: f32, secs: f32 },
    Weather { phenomenon: i32 },
    /// One `server.edit` on the server document (a field path and value, or
    /// an action with its fields), checked by the server; the step ends on
    /// the answer.
    ServerEdit { command: Map<String, Value> },
    SiteSwitch { site: String },
}

/// The point a companion turns to.
/// The fixture a `companion.use` step uses.
#[derive(Clone, Debug, PartialEq)]
pub enum UseTarget {
    /// The placed copy of this master id nearest the companion.
    Nearest(i32),
    /// The placed fixture with this uid.
    Uid(String),
}

#[derive(Clone, Debug)]
pub enum FaceTo {
    Point([f32; 3]),
    /// The player's position when the step starts.
    Player,
}

/// A placed fixture: by placement UID, or the instance of a master id
/// nearest the one who goes to it.
#[derive(Clone, Debug)]
pub enum FixtureRef {
    Uid(String),
    Id(i32),
}

/// A gatherable (a harvest object of the loaded site): by its UID, or the
/// nearest one not yet harvested, of one kind when `kind` names it.
#[derive(Clone, Debug)]
pub enum GatherRef {
    Uid(u64),
    Nearest { kind: Option<String> },
}

/// A place in the world, resolved every frame.
#[derive(Clone, Debug)]
pub struct Target {
    pub what: TargetKind,
    /// Metres added on x and z after the place is resolved.
    pub offset: [f32; 2],
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKindName {
    Home,
    Room,
    Harvest,
    Delivery,
}

#[derive(Clone, Debug)]
pub enum Cond {
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    /// The active site's type (`home_site`, `first_floor`, ...).
    SiteIs(String),
    SiteKind(SiteKindName),
    /// A site move (cannon or door) is in progress.
    SiteMoving,
    /// The entry sequence has ended, including the stamina refresh clip when the join reports one.
    EntryDone,
    /// The entry has opened movement and gestures (`entry::control_open`),
    /// which a site move must wait for.
    ControlOpen,
    DialogShown,
    /// A player or NPC conversation is playing.
    TalkActive,
    /// The camera has left its entry shot and no transition tween is in
    /// flight, so a drag or a wheel turn moves it.
    CameraSettled,
    ButtonHead(ButtonType),
    ButtonStacked(ButtonType),
    PlayerNear { to: Target, within: f32 },
    NpcNear { unit: u32, to: Target, within: f32 },
    PhenomenonIs(i32),
    /// The character's action state (`Idle`, `Walk`, `Rest`, `Talk`, ...).
    NpcAction { unit: u32, is: String },
    /// The character is on this site with its AI (a roster character, not a
    /// cut-scene stand-in).
    NpcHere(u32),
    /// The node at `path` of the UI view `view` is shown and active.
    UiShown { view: String, path: String },
}

pub(super) const ACTIONS: &[&str] = &[
    "wait", "wait_until", "assert", "log", "where", "capture", "parallel", "repeat",
    "player.walk_to", "player.interact", "player.gather", "player.craft", "player.stick", "player.release", "player.talk_to", "player.wear",
    "player.wear_avatar", "player.deliver", "tap", "ui.tap", "dialog.answer", "key.back", "layout.place",
    "npc.take", "npc.walk_to", "npc.face", "npc.emote", "npc.use", "npc.tweet", "npc.talk", "npc.release",
    "companion.spawn", "companion.walk_to", "companion.face", "companion.emote", "companion.motion",
    "companion.expression", "companion.balloon", "companion.despawn", "companion.build",
    "companion.use",
    "effect.play",
    "camera.drag", "camera.zoom", "server.edit", "weather.set", "site.switch", "site.enter_room", "site.go_home",
];

/// Parses a script document.
pub fn parse(document: &Value) -> Result<Script, String> {
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

/// Parses one step object, checked as a script's step is; a macro expands
/// into the several steps it stands for.
pub fn parse_step(value: &Value) -> Result<Arc<[Step]>, String> {
    step(value, "step").map(Arc::from)
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
        action: action.clone(),
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
        "player.gather" => {
            let what = match fields.req("gatherable")? {
                Value::String(word) if word == "nearest" => {
                    GatherRef::Nearest { kind: fields.opt_str("kind")?.map(str::to_owned) }
                }
                value => GatherRef::Uid(
                    value
                        .as_u64()
                        .ok_or_else(|| format!("{at}: gatherable is a harvest object UID or \"nearest\""))?,
                ),
            };
            if matches!(what, GatherRef::Uid(_)) && fields.opt("kind").is_some() {
                return Err(format!("{at}: kind only narrows gatherable \"nearest\""));
            }
            let collect = fields.opt_bool("collect")?.unwrap_or(true);
            vec![one(Node::Gather { what, collect }, Some(120.0))]
        }
        "player.craft" => {
            let blueprint = fields.req_i32("blueprint")?;
            let workbench = match fields.opt("workbench") {
                Some(Value::Bool(false)) => None,
                Some(fixture) => Some(match fixture_ref(fixture, &at)? {
                    FixtureRef::Uid(uid) => TargetKind::FixtureUid(uid),
                    FixtureRef::Id(id) => TargetKind::FixtureId(id),
                }),
                None => Some(TargetKind::Workbench),
            };
            let mut out = Vec::new();
            if let Some(what) = workbench {
                out.push(one(
                    Node::Interact { target: Target { what, offset: [0.0; 2] }, button: ButtonType::OpenCraftTool, stop: 0.2 },
                    Some(60.0),
                ));
            }
            out.push(one(Node::Craft { blueprint }, Some(60.0)));
            out
        }
        "dialog.answer" => {
            let ok = fields.opt_bool("ok")?.unwrap_or(true);
            vec![one(Node::DialogAnswer { ok }, Some(10.0))]
        }
        "layout.place" => {
            let fixture = fields.req_i32("fixture")?;
            let texture = fields.opt_u32("texture")?;
            let center = tile(fields.req("center")?, &format!("{at}.center"))?;
            let rotate = fields.opt_u32("rotate")?.unwrap_or(0);
            if rotate > 3 {
                return Err(format!("{at}: rotate is 0 to 3 quarter turns, got {rotate}"));
            }
            let decide = fields.opt_bool("decide")?.unwrap_or(true);
            let save = fields.opt_bool("save")?.unwrap_or(true);
            vec![one(Node::LayoutPlace { fixture, texture, center, rotate, decide, save }, Some(120.0))]
        }
        "player.deliver" => {
            let party = fields.opt_i32("party")?;
            let hold = fields.opt_f32("hold")?;
            if hold == Some(0.0) {
                return Err(format!("{at}: hold is a number of seconds above 0"));
            }
            vec![one(Node::Deliver { party, hold }, Some(180.0))]
        }
        "key.back" => vec![one(Node::BackKey, None)],
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
        "player.wear" => vec![one(Node::Wear { unit: Some(fields.req_u32("unit")?) }, None)],
        "player.wear_avatar" => vec![one(Node::Wear { unit: None }, None)],
        "tap" => {
            let (at_value, fraction) = match (fields.opt("at"), fields.opt("at_fraction")) {
                (Some(value), None) => (value, false),
                (None, Some(value)) => (value, true),
                _ => return Err(format!("{at}: give exactly one of at (logical pixels) or at_fraction (0..1 of the window)")),
            };
            vec![one(Node::Tap { at: pair(at_value, &format!("{at}.at"))?, fraction }, None)]
        }
        "ui.tap" => {
            let view = fields.req_str("view")?.to_owned();
            let path = fields.req_str("path")?.to_owned();
            vec![one(Node::UiTap { view, path }, None)]
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
        "npc.talk" => {
            let unit = fields.req_u32("unit")?;
            let talk = fields.req_i32("talk")?;
            vec![one(Node::NpcTalk { unit, talk }, Some(60.0))]
        }
        "companion.spawn" => {
            let unit = fields.req_u32("unit")?;
            let at = triple(fields.req("at")?, &format!("{at}.at"))?;
            vec![one(Node::CompanionSpawn { unit, at }, Some(60.0))]
        }
        "companion.walk_to" => {
            let unit = fields.req_u32("unit")?;
            let to = triple(fields.req("to")?, &format!("{at}.to"))?;
            // The walk law caps the distance a walk may finish within.
            let within = fields
                .opt_f32("within")?
                .unwrap_or(0.3)
                .min(crate::companion::WITHIN_LIMIT);
            vec![one(Node::CompanionWalkTo { unit, to, within }, Some(60.0))]
        }
        "companion.face" => {
            let unit = fields.req_u32("unit")?;
            let to = face_to(fields.req("to")?, &format!("{at}.to"))?;
            let secs = fields.opt_f32("secs")?.unwrap_or(0.4);
            vec![one(Node::CompanionFace { unit, to, secs }, Some(secs + 5.0))]
        }
        "companion.emote" => {
            let unit = fields.req_u32("unit")?;
            let name = fields.req_str("name")?.to_owned();
            let secs = fields.opt_f32("secs")?.unwrap_or(2.0);
            let use_root = fields.opt_bool("use_root")?.unwrap_or(false);
            vec![one(Node::CompanionEmote { unit, name, secs, use_root }, None)]
        }
        "companion.motion" => {
            let unit = fields.req_u32("unit")?;
            let motion = fields.req_str("motion")?.to_owned();
            vec![one(Node::CompanionMotion { unit, motion }, Some(60.0))]
        }
        "companion.expression" => {
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
            vec![one(Node::CompanionExpression { unit, eye, mouth, reset }, None)]
        }
        "companion.balloon" => {
            let unit = fields.req_u32("unit")?;
            let text = fields.opt_str("text")?.map(str::to_owned);
            let tweet = fields.opt_i32("tweet")?;
            if text.is_some() == tweet.is_some() {
                return Err(format!("{at}: give exactly one of text or tweet"));
            }
            let secs = fields.opt_f32("secs")?.unwrap_or(3.0);
            vec![one(Node::CompanionBalloon { unit, text, tweet, secs }, Some(secs + 5.0))]
        }
        "companion.despawn" => vec![one(Node::CompanionDespawn { unit: fields.req_u32("unit")? }, Some(10.0))],
        "companion.build" => {
            let unit = fields.req_u32("unit")?;
            let fixture = fields.req_i32("fixture")?;
            let texture = fields.opt_u32("texture")?;
            let near = fields.opt("near").map(|near| triple(near, &format!("{at}.near"))).transpose()?;
            let swings = fields
                .opt_u32("swings")?
                .unwrap_or(crate::companion::build::DEFAULT_SWINGS)
                .clamp(1, crate::companion::build::MAX_SWINGS);
            vec![one(Node::CompanionBuild { unit, fixture, texture, near, swings }, Some(120.0))]
        }
        "companion.use" => {
            let unit = fields.req_u32("unit")?;
            let target = match (fields.opt_i32("fixture")?, fields.opt_str("uid")?) {
                (Some(fixture), None) => UseTarget::Nearest(fixture),
                (None, Some(uid)) => UseTarget::Uid(uid.to_owned()),
                _ => return Err(format!("{at}: takes one of fixture and uid")),
            };
            let activity = fields.opt_i32("activity")?;
            let variant = fields.opt_u32("variant")?;
            vec![one(Node::CompanionUse { unit, target, activity, variant }, Some(120.0))]
        }
        "effect.play" => {
            let name = fields.req_str("name")?.to_owned();
            let at = triple(fields.req("at")?, &format!("{at}.at"))?;
            let secs = fields.opt_f32("secs")?.unwrap_or(2.0);
            vec![one(Node::EffectPlay { name, at, secs }, None)]
        }
        "camera.drag" => {
            let by = pair(fields.req("by")?, &format!("{at}.by"))?;
            let secs = fields.opt_f32("secs")?.unwrap_or(0.5);
            let from = match fields.opt("from") {
                Some(from) => pair(from, &format!("{at}.from"))?,
                None => [0.5, 0.35],
            };
            if from.iter().any(|f| !(0.0..=1.0).contains(f)) {
                return Err(format!("{at}: from is a fraction of the window, got {from:?}"));
            }
            vec![one(Node::CameraDrag { by, secs, from }, None)]
        }
        "camera.zoom" => {
            let by = fields
                .req("by")?
                .as_f64()
                .filter(|by| by.is_finite())
                .ok_or_else(|| format!("{at}: by must be a number of pixels"))? as f32;
            let secs = fields.opt_f32("secs")?.unwrap_or(0.5);
            vec![one(Node::CameraZoom { by, secs }, None)]
        }
        "server.edit" => {
            // The step's other fields are the edit; the server model checks
            // them when the step runs and names what it refuses.
            let mut command = fields.rest();
            if !command.contains_key("path") && !command.contains_key("action") {
                return Err(format!("{at}: a server edit has a path and a value, or an action"));
            }
            command.insert("type".into(), "server.edit".into());
            vec![one(Node::ServerEdit { command }, None)]
        }
        "weather.set" => vec![one(Node::Weather { phenomenon: fields.req_i32("phenomenon")? }, None)],
        // The entry's Finish is what opens movement and gestures
        // (`MysekaiUtility.EnableGestureLayer` / `EnableJoyStick`); a site
        // move before it breaks the join's room/player check. Wait for it,
        // then request the move.
        "site.switch" => vec![
            one(Node::WaitUntil { until: Cond::ControlOpen }, Some(30.0)),
            one(Node::SiteSwitch { site: fields.req_str("site")?.to_owned() }, Some(60.0)),
        ],
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
        // The entry's control gate comes first: the door press belongs to the
        // player who can move, not to the cover.
        one(Node::WaitUntil { until: Cond::ControlOpen }, Some(30.0)),
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

/// A condition outside a script, as the control surface sends one.
pub fn condition(value: &Value) -> Result<Cond, String> {
    cond(value, "condition")
}

/// A condition: a bare name, or an object with one key.
fn cond(value: &Value, at: &str) -> Result<Cond, String> {
    if let Some(name) = value.as_str() {
        return Ok(match name {
            "site_moving" => Cond::SiteMoving,
            "entry_done" => Cond::EntryDone,
            "control_open" => Cond::ControlOpen,
            "dialog_shown" => Cond::DialogShown,
            "talk_active" => Cond::TalkActive,
            "camera_settled" => Cond::CameraSettled,
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
        "ui_shown" => {
            let mut fields = Fields::new(inner, &at)?;
            let shown = Cond::UiShown { view: fields.req_str("view")?.to_owned(), path: fields.req_str("path")?.to_owned() };
            fields.finish()?;
            shown
        }
        "npc_here" => Cond::NpcHere(
            inner
                .as_u64()
                .and_then(|unit| u32::try_from(unit).ok())
                .ok_or_else(|| format!("{at}: expected a character unit"))?,
        ),
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
            "workbench" => TargetKind::Workbench,
            other => return Err(format!("{at}: unknown target {other}; use house_door, room_door, player, workbench, [x, z] or an object")),
        };
        return Ok(Target { what, offset: [0.0; 2] });
    }
    let mut fields = Fields::new(value, &at)?;
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

/// The point a companion turns to: a world point `[x, y, z]`, or `player`.
fn face_to(value: &Value, at: &str) -> Result<FaceTo, String> {
    match value {
        Value::String(word) if word == "player" => Ok(FaceTo::Player),
        value => triple(value, at).map(FaceTo::Point),
    }
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

/// A world point: `[x, y, z]`.
fn triple(value: &Value, at: &str) -> Result<[f32; 3], String> {
    match value.as_array().map(Vec::as_slice) {
        Some([x, y, z]) => match (x.as_f64(), y.as_f64(), z.as_f64()) {
            (Some(x), Some(y), Some(z)) if x.is_finite() && y.is_finite() && z.is_finite() => Ok([x as f32, y as f32, z as f32]),
            _ => Err(format!("{at}: expected three numbers")),
        },
        _ => Err(format!("{at}: expected [x, y, z]")),
    }
}

/// A layout tile: three whole numbers in the grid's own range.
fn tile(value: &Value, at: &str) -> Result<[i8; 3], String> {
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

    /// Every field not read yet, now counted as read.
    fn rest(&mut self) -> Map<String, Value> {
        let rest: Map<String, Value> = self
            .map
            .iter()
            .filter(|(key, _)| !self.used.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.used.extend(self.map.keys().map(String::as_str));
        rest
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
    fn a_door_macro_expands_into_its_four_steps() {
        let script = parse(&json!({ "steps": [{ "do": "site.enter_room" }] })).unwrap();
        assert_eq!(script.steps.len(), 4);
        assert!(matches!(script.steps[0].node, Node::WaitUntil { until: Cond::ControlOpen }));
        assert!(matches!(script.steps[1].node, Node::Interact { button: ButtonType::HouseEntry, .. }));
        assert!(matches!(script.steps[2].node, Node::WaitUntil { until: Cond::SiteMoving }));
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
                { "do": "wait_until", "until": { "all": ["entry_done", { "not": "site_moving" }] } },
                { "do": "wait_until", "until": { "all": [{ "npc_here": 13 }, { "npc_here": 14 }] } }
            ]
        }))
        .unwrap();
        assert_eq!(script.name, "two walkers");
        let Node::WaitUntil { until: Cond::All(here) } = &script.steps[3].node else { panic!() };
        assert!(matches!(here.as_slice(), [Cond::NpcHere(13), Cond::NpcHere(14)]));
        let error = parse(&json!({ "steps": [{ "do": "wait_until", "until": { "npc_here": "x" } }] })).unwrap_err();
        assert!(error.contains("expected a character unit"), "{error}");
        let ui = parse(&json!({ "steps": [
            { "do": "wait_until", "until": { "ui_shown": { "view": "ShellHome", "path": "MenuButton" } } },
            { "do": "ui.tap", "view": "ShellHome", "path": "MenuButton" }
        ] }))
        .unwrap();
        assert!(matches!(&ui.steps[0].node, Node::WaitUntil { until: Cond::UiShown { view, path } }
            if view == "ShellHome" && path == "MenuButton"));
        assert!(matches!(&ui.steps[1].node, Node::UiTap { view, .. } if view == "ShellHome"));
        let error = parse(&json!({ "steps": [{ "do": "ui.tap", "view": "ShellHome" }] })).unwrap_err();
        assert!(error.contains("path"), "{error}");
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
            { "do": "npc.tweet", "unit": 2, "tweet": 301 },
            { "do": "npc.talk", "unit": 2, "talk": 4105 }
        ] }))
        .unwrap();
        let Node::NpcUse { fixture: FixtureRef::Id(1053), activity: Some(7), variant: Some(2), .. } = &script.steps[0].node
        else {
            panic!("{:?}", script.steps[0].node)
        };
        assert!(matches!(&script.steps[1].node, Node::NpcUse { fixture: FixtureRef::Uid(uid), activity: None, .. } if uid == "p-12"));
        assert_eq!(script.steps[1].timeout, Some(300.0));
        assert!(matches!(&script.steps[2].node, Node::NpcTweet { tweet: 301, secs, .. } if *secs == 3.0));
        assert!(matches!(&script.steps[3].node, Node::NpcTalk { unit: 2, talk: 4105 }));
        assert_eq!(script.steps[3].timeout, Some(60.0));
        let error = parse(&json!({ "steps": [{ "do": "npc.use", "unit": 2, "fixture": 1, "variant": 0 }] })).unwrap_err();
        assert!(error.contains("variant counts from 1"), "{error}");
    }

    #[test]
    fn server_edits_pass_their_fields_through() {
        let script = parse(&json!({ "steps": [
            { "do": "server.edit", "path": "clock.offsetMs", "value": 3600000, "label": "an hour on" },
            { "do": "server.edit", "action": "sync" }
        ] }))
        .unwrap();
        let Node::ServerEdit { command } = &script.steps[0].node else { panic!("not a server edit") };
        assert_eq!(Value::Object(command.clone()), json!({ "type": "server.edit", "path": "clock.offsetMs", "value": 3600000 }));
        assert_eq!(script.steps[0].label, "an hour on");
        assert!(matches!(&script.steps[1].node, Node::ServerEdit { command } if command["action"] == "sync"));
        let error = parse(&json!({ "steps": [{ "do": "server.edit", "value": 1 }] })).unwrap_err();
        assert!(error.contains("a path and a value, or an action"), "{error}");
    }

    #[test]
    fn camera_steps_take_pixels_and_window_fractions() {
        let script = parse(&json!({ "steps": [
            { "do": "camera.drag", "by": [-200, 0] },
            { "do": "camera.zoom", "by": -120, "secs": 1 },
            { "do": "wait_until", "until": "camera_settled" }
        ] }))
        .unwrap();
        assert!(matches!(script.steps[2].node, Node::WaitUntil { until: Cond::CameraSettled }));
        assert!(matches!(&script.steps[0].node,
            Node::CameraDrag { by: [x, y], secs, from: [fx, fy] }
                if *x == -200.0 && *y == 0.0 && *secs == 0.5 && *fx == 0.5 && *fy == 0.35));
        assert!(matches!(&script.steps[1].node, Node::CameraZoom { by, secs } if *by == -120.0 && *secs == 1.0));
        let error = parse(&json!({ "steps": [{ "do": "camera.drag", "by": [1, 1], "from": [1.5, 0] }] })).unwrap_err();
        assert!(error.contains("fraction of the window"), "{error}");
    }
}
