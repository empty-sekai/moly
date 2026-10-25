//! The director: scripted orchestration of the player, the characters and
//! the world, for debugging runs and authored sequences.
//!
//! A script is a JSON document `{ "name", "exit"?, "steps": [...] }`. Each
//! step is an object with `do` (the action) and its fields, plus the common
//! fields `label` (its name in the log), `timeout` (seconds before it fails)
//! and `optional` (a failure is logged and the script goes on).
//!
//! Control: `wait {secs}`, `wait_until {until: cond}`, `assert {that: cond}`,
//! `log {text}`, `where {of: [targets]}` (log the player's position and the
//! targets'), `capture` (save the next frame), `parallel {tracks: [[..],
//! [..]], join: all|any}`, `repeat {times, steps}`.
//!
//! The player is driven through a virtual hand that writes the same touch
//! streams a finger makes: `player.walk_to {to, within}` holds the joystick
//! along a path on the navigation field; `player.interact {target, button,
//! stop}` walks until the button heads the action-button stack (tapping the
//! change-target button when it is stacked behind another) and presses it;
//! `player.talk_to {unit}` is an interaction on the character's Talk button;
//! `player.stick {stick: [x, y], secs}` holds the stick at a ratio (x right,
//! y up) like a thumb; `player.release` lifts the joystick finger;
//! `player.deliver {party, hold}` presses the delivery button of a birthday
//! party in session, holds it (`hold` seconds, or until the party has
//! nothing left to deliver) and releases it, the two requests the delivery
//! screen sends; a press with no party in session is refused by name;
//! `player.wear {unit}` and `player.wear_avatar` choose the player's body (a
//! character body of the cast, or the avatar body; the step ends once the
//! choice is made, and the body is worn when it has loaded); `tap {at |
//! at_fraction}` taps the screen; `ui.tap {view, path}` taps the centre of a
//! node of a shown UI view (`view` its document key, `path` a unique node
//! path suffix or `@<id>`; a view that is not shown is refused with the
//! views that are); `dialog.answer {ok}` taps the shown common dialog's OK or
//! cancel button, and answers a sub-window dialog that has no common button
//! (the learn-phenomenon one) with the back key its own class takes. Every
//! input gate of the product applies to these.
//!
//! Characters: `npc.take {unit}` cancels the character's objective and holds
//! its AI; `npc.walk_to {unit, to, within, rests}` walks a taken character
//! with its own navigation (`rests: true` keeps the walk law's rest stops);
//! `npc.face {unit, to, secs}` turns a taken, standing character toward a
//! target; `npc.emote {unit, name, secs, use_root}` shows an emoticon (such
//! as `fx_emote_001`) over a character; `npc.use {unit, fixture, activity,
//! variant, rests}` plays one of a taken character's authored no-talk
//! activities at a placed fixture (`fixture` a placement UID or a master id,
//! the nearest instance; the first activity and timeline by default) through
//! the furniture owner its AI uses, and ends when the activity has; `npc.tweet
//! {unit, tweet, secs}` shows a tweet of the master over a taken character;
//! `npc.talk {unit, talk}` hands the character (taken or not) to its AI with
//! a talk to open: its next decision builds that talk as the talk-by-id
//! factories build a drawn one (the talk's fixture, the other speakers
//! drafted) instead of drawing, and the step ends when the talk is built or
//! refused, the talk then playing under the AI; `npc.release {unit}` gives
//! it back to its AI. A script's taken characters
//! are released when it ends.
//!
//! Camera: `camera.drag {by: [dx, dy], secs, from: [fx, fy]}` drags one
//! finger by logical pixels from a point given as fractions of the window
//! (the field above the player by default); `camera.zoom {by, secs}` turns
//! the wheel by pixels (positive zooms in). Both go through the gesture
//! layer and the camera's own input law.
//!
//! Server: `server.edit {path, value}` or `server.edit {action, ...}` makes
//! one edit of the server document (the clock, the stamina, the phenomenon
//! schedule, the gate actions and the rest the server panel offers); the
//! server model checks it as it does the panel's, and the step fails with
//! the model's refusal.
//!
//! The layout editor: `layout.place {fixture, texture, center, rotate,
//! decide, save}` puts one owned fixture through the editor's own commands,
//! the objects `moly.layout.command` takes, read by the same reader: enter,
//! select the catalog row of the fixture (and texture) with a placeable
//! copy, turn it a quarter turn `rotate` times, move it onto the tile,
//! decide it and save. A refusal of the editor's is the step's failure, with
//! its put status and feedback; the editor stays open afterwards.
//!
//! World: `weather.set {phenomenon}`, `site.switch {site}` (a site type such
//! as `first_floor`; it waits until the entry has opened movement), and the
//! door sequences `site.enter_room` and `site.go_home` (wait for the entry,
//! walk to the door, press its button, wait for the move to land).
//!
//! Targets: `[x, z]`, `"house_door"`, `"room_door"`, `"player"`, or an object
//! `{npc: unit}`, `{fixture: "uid" | master id}` or `{point: [x, z]}`, each
//! with an optional `offset: [dx, dz]`.
//!
//! Conditions: `"site_moving"`, `"entry_done"`, `"control_open"` (the entry
//! has opened movement and gestures), `"dialog_shown"`,
//! `"talk_active"`, `"camera_settled"` (the entry shot has handed over and
//! no camera transition is in flight), or `{site_is}`, `{site_kind: home|room|harvest|delivery}`,
//! `{button_head}`, `{button_stacked}`, `{phenomenon_is}`, `{player_near: {to,
//! within}}`, `{npc_near: {unit, to, within}}`, `{npc_here: unit}` (the
//! character is on this site), `{ui_shown: {view, path}}` (the node is
//! shown and active), `{npc_action: {unit, is}}`
//! (an action state such as `Walk` or `Talk`), `{all: [..]}`, `{any: [..]}`,
//! `{not: ..}`.
//!
//! Commands: `{"run": script}` (or a bare script), `{"stop": true}`,
//! `{"pause": true}`, `{"resume": true}`. A native run reads a script file
//! named by `MOLY_DIRECTOR_SCRIPT` at start, and a script with `exit` ends
//! the app once its captures are on disk; the browser engine takes
//! commands through [`director_command`] and reports through
//! [`director_snapshot`] (status `idle`, `queued`, `running`, `paused`,
//! `done`, `failed` or `stopped`).

mod cond;
mod hand;
mod npc;
pub mod run;
mod script;

/// The touch the director's steps write, for the control surface's own steps.
pub use hand::{tap, ui_point, window_size};
/// The steps a program outside the director's scripts runs, by the ticket
/// they advance with ([`run::ACTED_TICKET`] plus the program's number).
pub use run::ACTED_TICKET;
pub use script::{parse_step, Step};

use std::sync::Mutex;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::view::screenshot::Screenshot;
use serde_json::{json, Value};

/// The ticket of the director's own script: the run that takes the hand
/// before any other.
const DIRECTOR_TICKET: u64 = 0;

/// How long a finished script with `exit` waits for its captures to save.
#[cfg(not(target_arch = "wasm32"))]
const EXIT_CAPTURE_WAIT: f32 = 5.0;

/// Commands waiting for the next frame.
static COMMANDS: Mutex<Vec<Command>> = Mutex::new(Vec::new());
/// The last published snapshot.
static PUBLISHED: Mutex<String> = Mutex::new(String::new());
const MAX_PENDING: usize = 16;

enum Command {
    Run(script::Script),
    Stop,
    Pause,
    Resume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Idle,
    Running,
    Paused,
    Done,
    Failed,
    Stopped,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Resource)]
struct Director {
    status: Status,
    script: Option<script::Script>,
    track: Option<run::Track>,
    shared: run::Shared,
    started: f32,
    error: Option<String>,
    /// The revision last published, and when.
    published: Option<(u64, f32)>,
    /// A finished script's exit, held while its captures save: the exit,
    /// the frame it finished on and the time it gives up waiting.
    exit: Option<(AppExit, u32, f32)>,
}

impl Default for Director {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            script: None,
            track: None,
            shared: run::Shared::default(),
            started: 0.0,
            error: None,
            published: None,
            exit: None,
        }
    }
}

/// Parses one command document.
fn parse_command(input: &str) -> Result<Command, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| format!("director command: {error}"))?;
    let object = value.as_object().ok_or("director command: expected an object")?;
    if object.contains_key("steps") {
        return script::parse(&value).map(Command::Run);
    }
    match object.iter().next().filter(|_| object.len() == 1) {
        Some((key, script)) if key == "run" => script::parse(script).map(Command::Run),
        Some((key, Value::Bool(true))) if key == "stop" => Ok(Command::Stop),
        Some((key, Value::Bool(true))) if key == "pause" => Ok(Command::Pause),
        Some((key, Value::Bool(true))) if key == "resume" => Ok(Command::Resume),
        _ => Err("director command: expected {\"run\": script}, a script, {\"stop\": true}, {\"pause\": true} or {\"resume\": true}".into()),
    }
}

/// Validates a command and queues it for the next frame. A script is checked
/// whole here, so a refusal is returned before anything runs.
pub fn director_command(input: &str) -> Result<(), String> {
    let command = parse_command(input)?;
    let mut queue = COMMANDS.lock().map_err(|_| "director command queue is unavailable".to_owned())?;
    if queue.len() >= MAX_PENDING {
        return Err("director command queue is full; retry after the next frame".into());
    }
    queue.push(command);
    Ok(())
}

/// The run's state: status, script, the steps running now, the log and the
/// error that ended it. While commands wait for the next frame the status
/// is `queued`, so a caller that polls right after sending a script does
/// not read the state from before it.
pub fn director_snapshot() -> String {
    let pending = COMMANDS.lock().map(|queue| queue.len()).unwrap_or(0);
    let published = PUBLISHED.lock().map(|text| text.clone()).unwrap_or_default();
    let mut snapshot: Value = serde_json::from_str(&published).unwrap_or_else(|_| json!({ "status": Status::Idle.word() }));
    if pending > 0 {
        snapshot["status"] = json!("queued");
        snapshot["pending"] = json!(pending);
    }
    snapshot.to_string()
}

/// The status of the director's run: `idle`, `running`, `paused`, `done`,
/// `failed` or `stopped`.
pub fn status(world: &World) -> &'static str {
    world.get_resource::<Director>().map_or("idle", |director| director.status.word())
}

/// Whether a director script runs or waits for its next frame: it holds the
/// hand through both.
pub fn script_running(world: &World) -> bool {
    matches!(status(world), "running" | "paused" | "queued")
}

/// Whether a condition holds now, and what was read.
pub fn check(world: &mut World, condition: &Value) -> Result<(bool, String), String> {
    let condition = script::condition(condition)?;
    Ok(cond::eval(world, &condition))
}

/// The steps, conditions and target forms of a script, as JSON Schema.
pub const SCHEMA: &str = include_str!("schema.json");

/// The bundled example scripts: `[{id, title, script}]`.
pub const EXAMPLES: &str = include_str!("examples.json");

/// The bundled examples' titles and scripts, in order.
pub fn examples() -> Vec<(String, Value)> {
    let examples: Vec<Value> = serde_json::from_str(EXAMPLES).expect("bundled director examples are JSON");
    examples
        .into_iter()
        .map(|example| (example["title"].as_str().unwrap_or_default().to_owned(), example["script"].clone()))
        .collect()
}

pub fn install(app: &mut App) {
    app.init_resource::<Director>()
        .add_systems(Startup, load_native_script)
        .add_systems(
            Update,
            director_tick
                .before(crate::joystick::advance)
                .before(crate::gesture::advance)
                .before(crate::action_button::click),
        );
}

/// A native run's script file, from `MOLY_DIRECTOR_SCRIPT`.
fn load_native_script() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = std::env::var_os("MOLY_DIRECTOR_SCRIPT") else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        let loaded = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
            .and_then(|text| director_command(&text));
        match loaded {
            Ok(()) => info!("[director] script {} queued", path.display()),
            Err(error) => panic!("[director] MOLY_DIRECTOR_SCRIPT {}: {error}", path.display()),
        }
    }
}

/// The director's frame: the queued commands, the running script's steps and
/// the published snapshot. Systems that run after the director's frame
/// order themselves after this one.
pub fn director_tick(world: &mut World) {
    let commands: Vec<Command> = COMMANDS.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default();
    world.resource_scope(|world, mut director: Mut<Director>| {
        let director = &mut *director;
        director.shared.now = world.resource::<Time>().elapsed_secs();
        for command in commands {
            apply(world, director, command);
        }
        release_exit(world, director);
        if director.status == Status::Running {
            let track = director.track.as_mut().expect("a running director has a track");
            match track.tick(world, &mut director.shared, DIRECTOR_TICKET) {
                run::TrackState::Running => {}
                run::TrackState::Done => finish(world, director, Status::Done, None),
                run::TrackState::Failed(error) => finish(world, director, Status::Failed, Some(error)),
            }
        }
        publish(director);
    });
}

fn apply(world: &mut World, director: &mut Director, command: Command) {
    match command {
        Command::Run(script) => {
            if matches!(director.status, Status::Running | Status::Paused) {
                stop(world, director, "replaced by a new script");
            }
            info!("[director] running script {} ({} steps)", script.name, script.steps.len());
            director.track = Some(run::Track::new(script.steps.clone(), script.name.clone()));
            director.script = Some(script);
            director.shared.log.clear();
            director.shared.take_hand(DIRECTOR_TICKET);
            director.shared.revision += 1;
            director.started = director.shared.now;
            director.error = None;
            director.status = Status::Running;
        }
        Command::Stop => {
            if matches!(director.status, Status::Running | Status::Paused) {
                stop(world, director, "stopped by command");
            }
        }
        Command::Pause => {
            if director.status == Status::Running {
                director.shared.release_hand(world, DIRECTOR_TICKET);
                director.status = Status::Paused;
                director.shared.revision += 1;
                info!("[director] paused");
            }
        }
        Command::Resume => {
            if director.status == Status::Paused {
                director.status = Status::Running;
                director.shared.revision += 1;
                info!("[director] resumed");
            }
        }
    }
}

fn stop(world: &mut World, director: &mut Director, why: &str) {
    if let Some(track) = director.track.as_mut() {
        track.abort(world, &mut director.shared, DIRECTOR_TICKET);
    }
    finish(world, director, Status::Stopped, Some(why.to_owned()));
}

fn finish(world: &mut World, director: &mut Director, status: Status, error: Option<String>) {
    director.shared.release_all(world);
    director.track = None;
    director.status = status;
    director.shared.revision += 1;
    let name = director.script.as_ref().map_or("script", |script| script.name.as_str());
    let elapsed = director.shared.now - director.started;
    match &error {
        None => info!("[director] script {name} done in {elapsed:.1} s"),
        Some(error) => warn!("[director] script {name} {} after {elapsed:.1} s: {error}", status.word()),
    }
    director.error = error;
    #[cfg(not(target_arch = "wasm32"))]
    if director.script.as_ref().is_some_and(|script| script.exit) {
        let exit = match status {
            Status::Done => AppExit::Success,
            _ => AppExit::from_code(1),
        };
        director.exit = Some((exit, world.resource::<FrameCount>().0, director.shared.now + EXIT_CAPTURE_WAIT));
    }
}

/// Sends a held exit once no screenshot is pending. A capture requested on
/// the script's last frame spawns its screenshot a frame later, so the
/// check starts two frames after the finish.
fn release_exit(world: &mut World, director: &mut Director) {
    let Some((exit, frame, give_up)) = director.exit.clone() else {
        return;
    };
    if world.resource::<FrameCount>().0.wrapping_sub(frame) < 2 {
        return;
    }
    let pending = world.query_filtered::<(), With<Screenshot>>().iter(world).count();
    if pending > 0 && director.shared.now < give_up {
        return;
    }
    if pending > 0 {
        warn!("[director] exiting with {pending} capture(s) still unsaved");
    }
    director.exit = None;
    world.write_message(exit);
}

fn publish(director: &mut Director) {
    // A new record publishes at once; a running script's current steps are
    // republished twice a second.
    let now = director.shared.now;
    if let Some((revision, at)) = director.published {
        if revision == director.shared.revision && (director.status != Status::Running || now - at < 0.5) {
            return;
        }
    }
    let mut current = Vec::new();
    if let Some(track) = &director.track {
        track.current(&mut current);
    }
    let snapshot = json!({
        "status": director.status.word(),
        "script": director.script.as_ref().map(|script| script.name.clone()),
        "elapsed": director.shared.now - director.started,
        "current": current,
        "owned": director.shared.owned().map(|(unit, _)| unit).collect::<Vec<_>>(),
        "error": director.error,
        "log": director.shared.log,
    });
    if let Ok(mut published) = PUBLISHED.lock() {
        *published = snapshot.to_string();
    }
    director.published = Some((director.shared.revision, now));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_take_a_script_bare_or_under_run_and_refuse_anything_else() {
        let script = r#"{ "steps": [{ "do": "wait", "secs": 1 }] }"#;
        assert!(matches!(parse_command(script), Ok(Command::Run(_))));
        assert!(matches!(parse_command(&format!(r#"{{ "run": {script} }}"#)), Ok(Command::Run(_))));
        assert!(matches!(parse_command(r#"{ "stop": true }"#), Ok(Command::Stop)));
        assert!(parse_command(r#"{ "stop": false }"#).is_err());
        assert!(parse_command(r#"{ "jump": true }"#).is_err());
    }

    #[test]
    fn the_schema_names_every_action_the_parser_takes() {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        let steps: Vec<&str> = schema["steps"].as_array().unwrap().iter().map(|step| step["name"].as_str().unwrap()).collect();
        let mut named = steps.clone();
        named.sort_unstable();
        let mut parsed = script::ACTIONS.to_vec();
        parsed.sort_unstable();
        assert_eq!(named, parsed);
        for condition in schema["conditions"].as_array().unwrap() {
            let name = condition["name"].as_str().unwrap();
            let probe = if condition["form"] == "bare" { json!(name) } else { json!({ name: null }) };
            if let Err(error) = script::condition(&probe) {
                assert!(!error.contains("unknown condition"), "{name}: {error}");
            }
        }
    }

    #[test]
    fn every_bundled_example_parses() {
        let examples = examples();
        assert!(!examples.is_empty());
        for (title, script) in examples {
            assert!(!title.is_empty());
            script::parse(&script).unwrap_or_else(|error| panic!("{title}: {error}"));
        }
    }

    #[test]
    fn a_sent_script_reads_as_queued_until_a_frame_takes_it() {
        let before: Value = serde_json::from_str(&director_snapshot()).unwrap();
        director_command(r#"{ "steps": [{ "do": "wait", "secs": 1 }] }"#).unwrap();
        let queued: Value = serde_json::from_str(&director_snapshot()).unwrap();
        assert_eq!(queued["status"], "queued");
        assert_eq!(queued["pending"], 1);
        COMMANDS.lock().unwrap().clear();
        let after: Value = serde_json::from_str(&director_snapshot()).unwrap();
        assert_eq!(after["status"], before["status"]);
    }
}
