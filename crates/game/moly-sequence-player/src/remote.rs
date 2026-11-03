//! The sequence's methods of the control surface: `moly.sequence.load`,
//! `play`, `pause`, `seek`, `speed`, `stop`, `status` and `schema`.
//!
//! The methods join the protocol's table at `Startup`, once the control
//! surface has built it. A method that changes the player queues its command
//! for the player's next frame and answers that frame's reply and status;
//! `status` and `schema` answer at once.

use bevy::prelude::*;
use bevy_remote::{error_codes, BrpError, BrpResult, RemoteMethodSystemId, RemoteMethods};
use serde_json::{json, Map, Value};

use crate::controls::Command;

/// The methods, by name.
const METHODS: &[&str] = &["moly.sequence.load", "moly.sequence.play", "moly.sequence.pause", "moly.sequence.seek", "moly.sequence.speed", "moly.sequence.stop", "moly.sequence.status", "moly.sequence.schema"];

/// The params each method takes, as JSON Schema.
fn params_schema() -> Value {
    json!({
        "moly.sequence.load": { "type": "object", "properties": { "path": { "type": "string", "description": "The file of the sequence document." } }, "required": ["path"], "additionalProperties": false },
        "moly.sequence.play": { "type": "object", "properties": { "from": { "type": "number", "minimum": 0, "description": "Seconds to start at; absent goes on from the played time, or from the start." } }, "additionalProperties": false },
        "moly.sequence.pause": { "type": "object", "additionalProperties": false },
        "moly.sequence.seek": { "type": "object", "properties": { "t": { "type": "number", "minimum": 0 }, "marker": { "type": "string" } }, "additionalProperties": false },
        "moly.sequence.speed": { "type": "object", "properties": { "scale": { "type": "number", "minimum": 0 } }, "required": ["scale"], "additionalProperties": false },
        "moly.sequence.stop": { "type": "object", "additionalProperties": false },
        "moly.sequence.status": { "type": "object", "additionalProperties": false },
        "moly.sequence.schema": { "type": "object", "additionalProperties": false }
    })
}

pub fn install(app: &mut App) {
    app.add_systems(Startup, register);
}

fn register(world: &mut World) {
    if !world.contains_resource::<RemoteMethods>() {
        return;
    }
    let load = world.register_system(load);
    let play = world.register_system(play);
    let pause = world.register_system(pause);
    let seek = world.register_system(seek);
    let speed = world.register_system(speed);
    let stop = world.register_system(stop);
    let status = world.register_system(status);
    let schema = world.register_system(schema);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("moly.sequence.load", RemoteMethodSystemId::Instant(load));
    methods.insert("moly.sequence.play", RemoteMethodSystemId::Instant(play));
    methods.insert("moly.sequence.pause", RemoteMethodSystemId::Instant(pause));
    methods.insert("moly.sequence.seek", RemoteMethodSystemId::Instant(seek));
    methods.insert("moly.sequence.speed", RemoteMethodSystemId::Instant(speed));
    methods.insert("moly.sequence.stop", RemoteMethodSystemId::Instant(stop));
    methods.insert("moly.sequence.status", RemoteMethodSystemId::Instant(status));
    methods.insert("moly.sequence.schema", RemoteMethodSystemId::Instant(schema));
}

fn refuse(message: impl Into<String>) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message: message.into(), data: None }
}

fn refusal(message: impl Into<String>) -> BrpError {
    BrpError { code: error_codes::RESOURCE_NOT_PRESENT, message: message.into(), data: None }
}

/// The params as an object; none reads as an empty one. A key the method
/// does not take is refused rather than ignored.
fn object(params: Option<Value>, method: &str, takes: &[&str]) -> Result<Map<String, Value>, BrpError> {
    let params = match params {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(refuse(format!("{method}: params must be an object"))),
    };
    if let Some(key) = params.keys().find(|key| !takes.contains(&key.as_str())) {
        let takes = if takes.is_empty() { "no parameters".to_owned() } else { takes.join(", ") };
        return Err(refuse(format!("{method}: unknown parameter \"{key}\"; it takes {takes}")));
    }
    Ok(params)
}

/// A number of seconds, 0 or more.
fn seconds(params: &Map<String, Value>, key: &str, method: &str) -> Result<Option<f32>, BrpError> {
    match params.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| Some(value as f32))
            .ok_or_else(|| refuse(format!("{method}: {key} must be a number of seconds, 0 or more"))),
    }
}

/// Runs a command on the player now, answering its note and the status
/// after it.
fn run(world: &mut World, command: Command, method: &str) -> BrpResult {
    crate::controls::run(world, &command)
        .map(|reply| json!({ "reply": reply.note(), "status": crate::status(world) }))
        .map_err(|error| refusal(format!("{method}: {}", error.0)))
}

fn load(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.sequence.load", &["path"])?;
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| refuse("moly.sequence.load: path must be a file name or path"))?
        .to_owned();
    run(world, Command::Load { path }, "moly.sequence.load")
}

fn play(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.sequence.play", &["from"])?;
    let from = seconds(&params, "from", "moly.sequence.play")?;
    run(world, Command::Play { from }, "moly.sequence.play")
}

fn pause(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    object(params, "moly.sequence.pause", &[])?;
    run(world, Command::Pause, "moly.sequence.pause")
}

fn seek(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.sequence.seek", &["t", "marker"])?;
    let t = seconds(&params, "t", "moly.sequence.seek")?;
    let marker = match params.get("marker") {
        None => None,
        Some(marker) => Some(marker.as_str().map(str::to_owned).ok_or_else(|| refuse("moly.sequence.seek: marker must be the name of a marker"))?),
    };
    if t.is_none() == marker.is_none() {
        return Err(refuse("moly.sequence.seek: give exactly one of t and marker"));
    }
    run(world, Command::Seek { t, marker }, "moly.sequence.seek")
}

fn speed(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.sequence.speed", &["scale"])?;
    let scale = params
        .get("scale")
        .and_then(Value::as_f64)
        .filter(|scale| scale.is_finite() && *scale >= 0.0)
        .map(|scale| scale as f32)
        .ok_or_else(|| refuse("moly.sequence.speed: scale must be a number of 0 or more"))?;
    run(world, Command::Speed { scale }, "moly.sequence.speed")
}

fn stop(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    object(params, "moly.sequence.stop", &[])?;
    run(world, Command::Stop, "moly.sequence.stop")
}

fn status(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    object(params, "moly.sequence.status", &[])?;
    Ok(crate::status(world))
}

fn schema(In(params): In<Option<Value>>, _world: &mut World) -> BrpResult {
    object(params, "moly.sequence.schema", &[])?;
    Ok(json!({ "methods": METHODS, "params": params_schema() }))
}
