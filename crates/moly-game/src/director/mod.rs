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
//! y up) like a thumb; `player.release` lifts the joystick finger; `tap {at |
//! at_fraction}` taps the screen. Every input gate of the product applies to
//! these.
//!
//! Characters: `npc.take {unit}` cancels the character's objective and holds
//! its AI; `npc.walk_to {unit, to, within, rests}` walks a taken character
//! with its own navigation (`rests: true` keeps the walk law's rest stops);
//! `npc.release {unit}` gives it back to its AI. A script's
//! taken characters are released when it ends.
//!
//! World: `weather.set {phenomenon}`, `site.switch {site}` (a site type such
//! as `first_floor`), and the door sequences `site.enter_room` and
//! `site.go_home` (walk to the door, press its button, wait for the move to
//! land).
//!
//! Targets: `[x, z]`, `"house_door"`, `"room_door"`, `"player"`, or an object
//! `{npc: unit}`, `{fixture: "uid" | master id}` or `{point: [x, z]}`, each
//! with an optional `offset: [dx, dz]`.
//!
//! Conditions: `"site_moving"`, `"entry_done"`, `"dialog_shown"`,
//! `"talk_active"`, or `{site_is}`, `{site_kind: home|room|harvest|delivery}`,
//! `{button_head}`, `{button_stacked}`, `{phenomenon_is}`, `{player_near: {to,
//! within}}`, `{npc_near: {unit, to, within}}`, `{all: [..]}`, `{any: [..]}`,
//! `{not: ..}`.
//!
//! Commands: `{"run": script}` (or a bare script), `{"stop": true}`,
//! `{"pause": true}`, `{"resume": true}`. A native run reads a script file
//! named by `MOLY_DIRECTOR_SCRIPT` at start; the browser engine takes
//! commands through [`director_command`] and reports through
//! [`director_snapshot`].

mod cond;
mod hand;
mod npc;
mod run;
mod script;

use std::sync::Mutex;

use bevy::prelude::*;
use serde_json::{json, Value};

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
/// error that ended it.
pub fn director_snapshot() -> String {
    let published = PUBLISHED.lock().map(|text| text.clone()).unwrap_or_default();
    if published.is_empty() {
        json!({ "status": Status::Idle.word() }).to_string()
    } else {
        published
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Director>()
        .add_systems(Startup, load_native_script)
        .add_systems(
            Update,
            tick.before(crate::joystick::advance)
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

fn tick(world: &mut World) {
    let commands: Vec<Command> = COMMANDS.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default();
    world.resource_scope(|world, mut director: Mut<Director>| {
        let director = &mut *director;
        director.shared.now = world.resource::<Time>().elapsed_secs();
        for command in commands {
            apply(world, director, command);
        }
        if director.status == Status::Running {
            let track = director.track.as_mut().expect("a running director has a track");
            match track.tick(world, &mut director.shared) {
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
                director.shared.hand.release(world);
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
        track.abort(world, &mut director.shared);
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
        world.write_message(match status {
            Status::Done => AppExit::Success,
            _ => AppExit::from_code(1),
        });
    }
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
        "owned": director.shared.owned.iter().map(|(unit, _)| unit).collect::<Vec<_>>(),
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
}
