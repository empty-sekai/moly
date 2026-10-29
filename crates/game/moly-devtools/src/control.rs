//! The control surface: the game's verbs as Bevy Remote Protocol methods,
//! beside the protocol's own `world.*`, `registry.schema` and `rpc.discover`.
//!
//! One method set serves every client. A native run opens a localhost HTTP
//! endpoint when `MOLY_CONTROL_PORT` names a port; the browser page sends
//! requests through [`control_request`]. Both transports take and answer
//! JSON-RPC 2.0 objects, and a request is handled in the frame after it
//! arrives. `moly.methods` lists the moly methods with what each takes.

pub mod layout;
mod observe;
/// The observation the `moly.observe` method answers, for the plugin host.
pub(crate) use observe::observe;
mod state;
mod ui;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy_remote::{error_codes, BrpError, BrpMessage, BrpResult, BrpSender, RemoteMethods, RemotePlugin};
use serde_json::{json, Map, Value};

/// The moly methods: name, what it does, and its params as JSON Schema.
const METHODS: &[(&str, &str, &str)] = &[
    ("moly.methods", "Lists the moly methods with what each takes.", NONE),
    (
        "moly.state",
        "One read of the running game: frame, time, server clock, entry, site, weather, the player, the characters (entity, position, away, action, objective), the screen stack and dialogs, and the camera. Entities are the protocol's entity numbers.",
        NONE,
    ),
    (
        "moly.ui.views",
        "The UI views shown and the controls on each (buttons, toggles, sliders): the path a director ui.tap step takes, whether the control is active and interactable, and the window point in logical pixels that the step taps.",
        NONE,
    ),
    (
        "moly.tap",
        "Taps the screen: down and up in one frame, as a touch does, so the game's own gestures decide what it means. Give at (logical pixels, top-left origin) or at_fraction (0..1 of the window).",
        r#"{"type":"object","properties":{"at":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"at_fraction":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"additionalProperties":false}"#,
    ),
    (
        "moly.ui.tap",
        "Taps one control of a shown UI view, the node a director ui.tap step takes: the path moly.ui.views answers. The point is one inside the control's rect that the view's own press raycast gives to that control; when a control covering the rect takes every point, it is refused with the covering paths.",
        r#"{"type":"object","properties":{"view":{"type":"string"},"path":{"type":"string"}},"required":["view","path"],"additionalProperties":false}"#,
    ),
    (
        "moly.panel",
        "Opens or closes the control panel and picks its tab (director, server, state, methods or more); answers {open, tab, narrow}.",
        r#"{"type":"object","properties":{"open":{"type":"boolean"},"tab":{"enum":["director","server","state","methods","more"]}},"additionalProperties":false}"#,
    ),
    (
        "moly.time",
        "Reads or sets the game clock: scale (0 or more, 1 is normal speed), paused, step (run that many frames, then pause), and fixed_dt (seconds each frame advances, whatever the frame took; null for real time). Answers the clock after the change: elapsed, scale, paused, stepping (frames of a step still to run), fixed_dt, frame, and the world seed with each random stream's draws.",
        r#"{"type":"object","properties":{"scale":{"type":"number","minimum":0},"paused":{"type":"boolean"},"step":{"type":"integer","minimum":1},"fixed_dt":{"type":["number","null"],"exclusiveMinimum":0}},"additionalProperties":false}"#,
    ),
    (
        "moly.observe",
        "The world as structured data, by section (what; every section when it is absent): site (with the layout grid's floor size and the tiles each placed fixture covers), gatherables (harvest objects with position, hit points, the tool type they need and whether the player targets one; and the drops on the ground), bag (materials, items, held total and limit), tools (gathering tools and stamina), recipes (the craft lists with costs and whether the apply button is on, else its balloon key), fixtures (owned and placed), characters, companions (each companion's position, the yaw it faces in radians, its command status and number, and while a build is its last command, the position of the fixture it builds as fixture_at), weather and time.",
        r#"{"type":"object","properties":{"what":{"type":"array","items":{"enum":["site","gatherables","bag","tools","recipes","fixtures","characters","companions","weather","time"]}}},"additionalProperties":false}"#,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "moly.plugins",
        "The plugins loaded at start: each with its directory, version, kind and settings; the plugins the runtime stopped, with their reasons; the configured but not loaded ones with what stopped them; and the seam table, whose exclusive entries name the provider of the seam (or of one target) and whose multi entries list their providers.",
        NONE,
    ),
    (
        "moly.events",
        "The event stream: the events after the one numbered since (default 0), oldest first, at most limit (default 200, at most 1000), only of kinds when given. Each event has seq, frame, time, kind and its fields; kinds are item_obtained, craft_done, craft_refused, delivery_started, delivery_refused, delivery_done, character_arrived, character_left, weather_changed, site_entered, talk_started, talk_ended, tweet_shown, server.call, server.reply, server.push and server.link. Answers {events, next, missed, latest}: pass next as since on the following read; missed counts the events dropped before they were read.",
        r#"{"type":"object","properties":{"since":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":1000},"kinds":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}"#,
    ),
    (
        "moly.director.schema",
        "The director's steps, conditions and target forms, as JSON Schema.",
        NONE,
    ),
    (
        "moly.director.command",
        "Queues a director command for the next frame: a script {name, exit, steps}, {run: script}, {stop: true}, {pause: true} or {resume: true}. A script is checked whole before anything runs; a new script replaces a running one.",
        r#"{"type":"object"}"#,
    ),
    (
        "moly.director.examples",
        "The bundled example scripts: [{id, title, script}].",
        NONE,
    ),
    (
        "moly.director.snapshot",
        "The director's status (idle, queued, running, paused, done, failed or stopped), the script's name, the steps running now, the characters it holds, the error that ended it and the run's log.",
        NONE,
    ),
    (
        "moly.director.check",
        "Whether a director condition holds now, and what was read.",
        r#"{"type":"object","properties":{"cond":{"description":"A condition of moly.director.schema"}},"required":["cond"]}"#,
    ),
    ("moly.server.schema", "The editable fields and the actions of the server document; the server is asked, and this answers its last answer (a pending placeholder until then).", NONE),
    (
        "moly.server.document",
        "The server document with its clock, the client's copies, the pending sections and the named refusals; the server is asked, and this answers its last answer with the game's own `client` part (a pending placeholder until then).",
        NONE,
    ),
    (
        "moly.server.edit",
        "Sends one edit of the server document and answers {pending: seq}; the outcome is read with moly.server.result and by moly.server.document.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"value":{},"action":{"type":"string"}}}"#,
    ),
    (
        "moly.server.result",
        "The answer to the edit numbered seq: {seq, ok: true, receipt} or {seq, ok: false, code, reason}, or {seq, pending: true}.",
        r#"{"type":"object","properties":{"seq":{"type":"integer","minimum":1}},"required":["seq"]}"#,
    ),
    (
        "moly.game.snapshot",
        "The browser game's projection: entry, storage, persistence, memory and named refusals.",
        NONE,
    ),
    (
        "moly.library.command",
        "Queues a content library command for the next frame (the library's page command object, such as {type: open}).",
        r#"{"type":"object","properties":{"type":{"type":"string"}},"required":["type"]}"#,
    ),
    (
        "moly.library.snapshot",
        "The content library's page snapshot, published where the page draws the library.",
        NONE,
    ),
    ("moly.library.catalog", "The content library's catalogue export.", NONE),
    (
        "moly.layout.view",
        "The layout editor's state: whether it is open, the selection with its put status, the placed fixtures, the inventory, the owned catalog (index, fixture, placeable count), whether Save is offered, and feedback naming the last refusal. Grid centers are [x, y, z] tiles; y is the stack height.",
        NONE,
    ),
    (
        "moly.layout.command",
        "Queues one layout editor command for the next frame, as the editor's own buttons, list cells and drags send it: enter, select_catalog {index}, select_placed {uid}, select_inventory {uid}, move_to {center: [x, y, z]} (the tile a drag touches), nudge {x, z}, rotate, decide, cancel, store, save, save_and_exit, request_exit, keep_editing, discard_and_exit, rotate_camera, change_look, request_clean_up, clean_up_all, cancel_clean_up. Read moly.layout.view afterwards for the result.",
        r#"{"type":"object","properties":{"command":{"type":"string"},"index":{"type":"integer","minimum":0},"uid":{"type":"string"},"center":{"type":"array","items":{"type":"integer"},"minItems":3,"maxItems":3},"x":{"type":"integer"},"z":{"type":"integer"}},"required":["command"]}"#,
    ),
    (
        "moly.layout.apply",
        "Runs one atomic layout operation sequence against the open editor session: all operations or none. Operations name the session piece numbers (moly.layout.view placed rows). A dry run returns the results and leaves the draft unchanged; a successful run updates the draft and the original whole-layout save stays the only uplink. Returns status applied with results, or status rejected with index, closed refusal code and results before it.",
        r#"{"type":"object","properties":{"ops":{"type":"array","minItems":1},"dry_run":{"type":"boolean"},"base":{"type":"integer"},"check_paths":{"type":"boolean"}},"required":["ops"]}"#,
    ),
    (
        "moly.perf.snapshot",
        "Frame time percentiles over the last 600 frames and over the newest `frames` frames (120 by default).",
        r#"{"type":"object","properties":{"frames":{"type":"integer","minimum":1}}}"#,
    ),
    (
        "moly.camera.free",
        "Puts the field camera at a world pose for screenshots and holds it until reset: position, target, optional up (default +Y) and fov in degrees (default 45). Answers the applied pose.",
        r#"{"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"target":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"up":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"fov":{"type":"number","minimum":5,"maximum":120}},"required":["position","target"]}"#,
    ),
    (
        "moly.camera.reset",
        "Clears the free camera and lets the player-following field camera take over on the next frame.",
        NONE,
    ),
    (
        "moly.capture",
        "Captures the next rendered frame of the window; answers {id} for moly.capture.take.",
        NONE,
    ),
    (
        "moly.capture.take",
        "A captured frame as {png (base64), width, height} once it is ready, else null. A frame is handed out once.",
        r#"{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}"#,
    ),
];

const NONE: &str = r#"{"type":"object","additionalProperties":false}"#;

/// Captured frames kept for `moly.capture.take`; older ones are dropped.
const KEPT_CAPTURES: usize = 4;

/// The request mailbox, shared with the page transport.
static SENDER: OnceLock<async_channel::Sender<BrpMessage>> = OnceLock::new();

pub fn install(app: &mut App) {
    let remote = RemotePlugin::default()
        .with_method_main("moly.methods", methods)
        .with_method_main("moly.state", world_state)
        .with_method_main("moly.ui.views", ui_views)
        .with_method_main("moly.tap", tap)
        .with_method_main("moly.ui.tap", ui_tap)
        .with_method_main("moly.panel", panel)
        .with_method_main("moly.time", time)
        .with_method_main("moly.events", read_events)
        .with_method_main("moly.observe", observe_world)
        .with_method_main("moly.director.schema", director_schema)
        .with_method_main("moly.director.command", director_command)
        .with_method_main("moly.director.examples", director_examples)
        .with_method_main("moly.director.snapshot", director_snapshot)
        .with_method_main("moly.director.check", director_check)
        .with_method_main("moly.server.schema", server_schema)
        .with_method_main("moly.server.document", server_document)
        .with_method_main("moly.server.edit", server_edit)
        .with_method_main("moly.server.result", server_result)
        .with_method_main("moly.game.snapshot", game_snapshot)
        .with_method_main("moly.library.command", library_command)
        .with_method_main("moly.library.snapshot", library_snapshot)
        .with_method_main("moly.library.catalog", library_catalog)
        .with_method_main("moly.layout.view", layout_view)
        .with_method_main("moly.layout.command", layout_command)
        .with_method_main("moly.layout.apply", layout_apply)
        .with_method_main("moly.perf.snapshot", perf_snapshot)
        .with_method_main("moly.camera.free", camera_free)
        .with_method_main("moly.camera.reset", camera_reset)
        .with_method_main("moly.capture", capture)
        .with_method_main("moly.capture.take", capture_take);
    // The plugins' report is read where the plugin host lives, in native runs.
    #[cfg(not(target_arch = "wasm32"))]
    let remote = remote.with_method_main("moly.plugins", plugins);
    app.add_plugins(remote)
        .init_resource::<Captures>()
        .init_resource::<PluginsAnswer>()
        .init_resource::<crate::camera::FreeCameraPose>()
        .init_resource::<TimeStep>()
        .add_systems(Startup, share_sender)
        .add_systems(First, crate::bus::stamp)
        .add_systems(Last, (crate::bus::watch, end_step, publish_report))
        .add_systems(Last, (crate::camera::apply_free_camera, free_camera_global).chain())
        // The talk camera rewrites the camera transform in PostUpdate every
        // frame, after transform propagation; the pose is stated again
        // after it, on the transform and the global transform both, so the
        // rendered view is the free one while a conversation plays.
        .add_systems(
            PostUpdate,
            (
                crate::director::talk_lens::follow_speaker,
                crate::camera::apply_free_camera,
                free_camera_global,
            )
                .chain()
                .after(crate::talk_camera::advance),
        );
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = native_port() {
        info!("[control] listening on 127.0.0.1:{port}");
        app.add_plugins(bevy_remote::http::RemoteHttpPlugin::default().with_port(port));
    }
}

/// `MOLY_CONTROL_PORT`, when set; anything but a port number is refused.
#[cfg(not(target_arch = "wasm32"))]
fn native_port() -> Option<u16> {
    let value = std::env::var("MOLY_CONTROL_PORT").ok()?;
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Some(port),
        _ => panic!("[control] MOLY_CONTROL_PORT must be a port number, not {value:?}"),
    }
}

fn share_sender(sender: Res<BrpSender>) {
    let _ = SENDER.set(async_channel::Sender::clone(&sender));
}

/// Last: the plugins' report, as the control surface read this frame. The
/// report is kept apart from the world it reports on, and is read by the
/// page from any thread.
#[derive(Resource, Default)]
pub(crate) struct PluginsAnswer(pub(crate) Option<Value>);

/// Last: takes the plugins' report for the control surface to answer from.
/// The plugin host's frame runs before this one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn publish_report(mut answer: ResMut<PluginsAnswer>) {
    answer.0 = Some(crate::plugin_host::report());
}

/// Last: the browser engine runs no Luau plugins, so there is no report.
#[cfg(target_arch = "wasm32")]
pub(crate) fn publish_report(mut answer: ResMut<PluginsAnswer>) {
    answer.0 = None;
}

/// One JSON-RPC 2.0 request, answered with its JSON-RPC 2.0 response. A
/// watching method answers its first update.
pub async fn control_request(request: &str) -> String {
    let answer = |id: Value, result: BrpResult| -> String {
        match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        }
        .to_string()
    };
    let request: Value = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => return answer(Value::Null, Err(failure(error_codes::PARSE_ERROR, format!("request is not JSON: {error}")))),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return answer(id, Err(failure(error_codes::INVALID_REQUEST, "request has no string method")));
    };
    let Some(sender) = SENDER.get() else {
        return answer(id, Err(BrpError::internal("the control surface has not started")));
    };
    let (reply, replies) = async_channel::bounded(1);
    let message = BrpMessage { method: method.to_owned(), params: request.get("params").cloned(), sender: reply };
    if sender.send(message).await.is_err() {
        return answer(id, Err(BrpError::internal("the control surface has stopped")));
    }
    match replies.recv().await {
        Ok(result) => answer(id, result),
        Err(_) => answer(id, Err(BrpError::internal("the request was dropped without an answer"))),
    }
}

/// Sends one request from inside the game (the control panel). The answer
/// comes on the returned channel once the frame's requests are handled.
pub fn call(method: &str, params: Option<Value>) -> Result<async_channel::Receiver<BrpResult>, String> {
    let sender = SENDER.get().ok_or("the control surface has not started")?;
    let (reply, replies) = async_channel::bounded(1);
    sender
        .try_send(BrpMessage { method: method.to_owned(), params, sender: reply })
        .map_err(|_| "the control surface is busy; try again".to_owned())?;
    Ok(replies)
}

/// A method that cannot answer for the state it was asked in.
fn refusal(message: impl Into<String>) -> BrpError {
    failure(error_codes::RESOURCE_NOT_PRESENT, message)
}

fn failure(code: i16, message: impl Into<String>) -> BrpError {
    BrpError { code, message: message.into(), data: None }
}

fn refuse(message: impl Into<String>) -> BrpError {
    failure(error_codes::INVALID_PARAMS, message)
}

/// The params as an object; none reads as an empty one. A method whose table
/// schema closes its properties takes the parameters it names and no others,
/// so a misspelled one is refused rather than quietly ignored.
fn object(params: Option<Value>, method: &str) -> Result<Map<String, Value>, BrpError> {
    let params = match params {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(refuse(format!("{method}: params must be an object"))),
    };
    let Some(schema) = closed_schema(method) else {
        return Ok(params);
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    let unknown = params.keys().find(|key| match properties {
        Some(properties) => !properties.contains_key(*key),
        None => true,
    });
    if let Some(key) = unknown {
        let takes = match properties {
            Some(properties) if !properties.is_empty() => properties.keys().cloned().collect::<Vec<String>>().join(", "),
            _ => "no parameters".to_owned(),
        };
        return Err(refuse(format!("{method}: unknown parameter \"{key}\"; it takes {takes}")));
    }
    Ok(params)
}

/// The method's params schema, when the table gives it one that closes its
/// properties; a method the table does not name is not checked.
fn closed_schema(method: &str) -> Option<Value> {
    let (_, _, schema) = METHODS.iter().find(|(name, ..)| *name == method)?;
    let schema: Value = serde_json::from_str(schema).expect("method params are JSON Schema");
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    closed.then_some(schema)
}

/// A projection the game publishes as JSON text.
fn projection(text: String) -> BrpResult {
    serde_json::from_str(&text).map_err(|error| BrpError::internal(format!("projection is not JSON: {error}")))
}

fn methods(In(_): In<Option<Value>>, registered: Res<RemoteMethods>) -> BrpResult {
    let registered = registered.methods();
    let listed = METHODS
        .iter()
        .filter(|(name, ..)| registered.iter().any(|method| method == name))
        .map(|(name, about, params)| {
            let params: Value = serde_json::from_str(params).expect("method params are JSON Schema");
            json!({ "name": name, "description": about, "params": params })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "methods": listed }))
}

fn ui_views(In(_): In<Option<Value>>, world: &mut World) -> BrpResult {
    ui::views(world).map_err(|message| failure(error_codes::RESOURCE_NOT_PRESENT, message))
}

fn panel(
    In(params): In<Option<Value>>,
    panel: Option<ResMut<crate::control_panel::ControlPanel>>,
    mut claims: ResMut<crate::input_claims::InputClaims>,
    time: Res<Time<Real>>,
) -> BrpResult {
    let params = object(params, "moly.panel")?;
    let mut panel = panel.ok_or_else(|| failure(error_codes::RESOURCE_NOT_PRESENT, "moly.panel: the control panel is not installed"))?;
    let answer = crate::control_panel::command(&mut panel, &params, time.elapsed_secs_f64()).map_err(refuse);
    crate::control_panel::publish(&panel, &mut claims);
    answer
}

fn tap(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.tap")?;
    let point = |name: &str| -> Result<Option<[f64; 2]>, BrpError> {
        match params.get(name) {
            None => Ok(None),
            Some(Value::Array(at)) if at.len() == 2 => {
                let mut pair = [0.; 2];
                for (slot, value) in pair.iter_mut().zip(at) {
                    *slot = value.as_f64().ok_or_else(|| refuse(format!("moly.tap: {name} must be two numbers")))?;
                }
                Ok(Some(pair))
            }
            Some(_) => Err(refuse(format!("moly.tap: {name} must be two numbers"))),
        }
    };
    let (at, fraction) = match (point("at")?, point("at_fraction")?) {
        (Some(at), None) => (at, false),
        (None, Some(at)) => (at, true),
        _ => return Err(refuse("moly.tap: give exactly one of at (logical pixels) or at_fraction (0..1 of the window)")),
    };
    let position = if fraction {
        let size = crate::director::window_size(world).ok_or_else(|| refusal("moly.tap: no primary window"))?;
        Vec2::new(at[0] as f32 * size.x, at[1] as f32 * size.y)
    } else {
        Vec2::new(at[0] as f32, at[1] as f32)
    };
    crate::director::tap(world, position).map_err(refusal)?;
    Ok(json!({ "tapped": [position.x, position.y] }))
}

fn ui_tap(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.ui.tap")?;
    let view = params.get("view").and_then(Value::as_str).ok_or_else(|| refusal("moly.ui.tap: params need view"))?;
    let path = params.get("path").and_then(Value::as_str).ok_or_else(|| refusal("moly.ui.tap: params need path"))?;
    let position = crate::director::ui_point(world, view, path).map_err(refuse)?;
    crate::director::tap(world, position).map_err(refusal)?;
    Ok(json!({ "tapped": [position.x, position.y], "view": view, "path": path }))
}

fn world_state(In(_): In<Option<Value>>, world: &mut World) -> BrpResult {
    Ok(state::snapshot(world))
}

/// A step in progress: frames still to run before the clock pauses again.
#[derive(Resource, Default)]
struct TimeStep {
    remaining: u32,
}

/// Last: counts a frame of a step; the clock pauses after its last frame, so
/// the next frame's virtual delta is zero.
fn end_step(mut step: ResMut<TimeStep>, mut time: ResMut<Time<Virtual>>) {
    if step.remaining == 0 {
        return;
    }
    step.remaining -= 1;
    if step.remaining == 0 {
        time.pause();
    }
}

fn time(
    In(params): In<Option<Value>>,
    mut time: ResMut<Time<Virtual>>,
    mut step: ResMut<TimeStep>,
    mut strategy: ResMut<bevy::time::TimeUpdateStrategy>,
    frame: Res<bevy::diagnostic::FrameCount>,
) -> BrpResult {
    let params = object(params, "moly.time")?;
    if let Some(scale) = params.get("scale") {
        let scale = scale
            .as_f64()
            .filter(|scale| scale.is_finite() && *scale >= 0.)
            .ok_or_else(|| refuse("moly.time: scale must be a number of 0 or more"))?;
        time.set_relative_speed_f64(scale);
    }
    match params.get("fixed_dt") {
        None => {}
        Some(Value::Null) => *strategy = bevy::time::TimeUpdateStrategy::Automatic,
        Some(dt) => {
            let dt = dt
                .as_f64()
                .filter(|dt| dt.is_finite() && *dt > 0. && *dt <= 1.)
                .ok_or_else(|| refuse("moly.time: fixed_dt must be seconds above 0 and at most 1, or null"))?;
            *strategy = bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(dt));
        }
    }
    let frames = match params.get("step") {
        None => None,
        Some(frames) => Some(
            frames
                .as_u64()
                .filter(|frames| (1..=u64::from(u32::MAX)).contains(frames))
                .ok_or_else(|| refuse("moly.time: step must be a whole number of frames, 1 or more"))? as u32,
        ),
    };
    match (params.get("paused"), frames) {
        (Some(_), Some(_)) => return Err(refuse("moly.time: give paused or step, not both")),
        (None, None) => {}
        (Some(Value::Bool(true)), None) => {
            step.remaining = 0;
            time.pause();
        }
        (Some(Value::Bool(false)), None) => {
            step.remaining = 0;
            time.unpause();
        }
        (Some(_), None) => return Err(refuse("moly.time: paused must be true or false")),
        (None, Some(frames)) => {
            // The request is answered after this frame's Last, so the step's
            // frames are the next `frames` frames.
            step.remaining = frames;
            time.unpause();
        }
    }
    let fixed_dt = match *strategy {
        bevy::time::TimeUpdateStrategy::ManualDuration(dt) => json!(dt.as_secs_f64()),
        _ => Value::Null,
    };
    Ok(json!({
        "elapsed": time.elapsed_secs_f64(),
        "scale": time.relative_speed_f64(),
        "paused": time.is_paused(),
        "stepping": step.remaining,
        "fixed_dt": fixed_dt,
        "frame": frame.0,
        "seed": crate::world_seed::seed(),
        "draws": crate::world_seed::draws().iter().map(|(name, count)| (name.to_string(), json!(count))).collect::<Map<String, Value>>(),
    }))
}

fn observe_world(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.observe")?;
    let what = match params.get("what") {
        None => Vec::new(),
        Some(Value::Array(names)) => names
            .iter()
            .map(|name| name.as_str().map(str::to_owned))
            .collect::<Option<Vec<String>>>()
            .ok_or_else(|| refuse("moly.observe: what must be section names"))?,
        Some(_) => return Err(refuse("moly.observe: what must be an array of section names")),
    };
    observe(world, &what).map_err(refuse)
}

#[cfg(not(target_arch = "wasm32"))]
fn plugins(In(_): In<Option<Value>>, answer: Res<PluginsAnswer>) -> BrpResult {
    answer.0.clone().ok_or_else(|| refusal("moly.plugins: the plugins' report is not ready yet"))
}

fn read_events(In(params): In<Option<Value>>) -> BrpResult {
    let params = object(params, "moly.events")?;
    let since = match params.get("since") {
        None => 0,
        Some(since) => since.as_u64().ok_or_else(|| refuse("moly.events: since must be a whole number, 0 or more"))?,
    };
    let limit = match params.get("limit") {
        None => 200,
        Some(limit) => limit
            .as_u64()
            .filter(|limit| (1..=crate::bus::MOST_PER_READ as u64).contains(limit))
            .ok_or_else(|| refuse(format!("moly.events: limit must be a whole number from 1 to {}", crate::bus::MOST_PER_READ)))?
            as usize,
    };
    let kinds = match params.get("kinds") {
        None => None,
        Some(Value::Array(kinds)) => Some(
            kinds
                .iter()
                .map(|kind| kind.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
                .ok_or_else(|| refuse("moly.events: kinds must be strings"))?,
        ),
        Some(_) => return Err(refuse("moly.events: kinds must be an array of strings")),
    };
    Ok(crate::bus::read(since, limit, kinds.as_deref()))
}

fn director_examples(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::director::EXAMPLES.to_owned())
}

fn library_command(In(params): In<Option<Value>>) -> BrpResult {
    let command = params.ok_or_else(|| refuse("moly.library.command: params are the command"))?;
    crate::content_library::bridge::library_command(&command.to_string()).map_err(refuse)?;
    Ok(Value::Null)
}

fn library_snapshot(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::content_library::bridge::library_snapshot())
}

fn library_catalog(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::content_library::bridge::library_catalog())
}

fn layout_view(In(_): In<Option<Value>>, world: &mut World) -> BrpResult {
    Ok(layout::view(world))
}

fn layout_command(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.layout.command")?;
    let command = layout::command(&params).map_err(refuse)?;
    let queued = format!("{command:?}");
    world.write_message(command);
    Ok(json!({ "queued": queued }))
}

fn layout_apply(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.layout.apply")?;
    let request = layout::apply_request(&params).map_err(refuse)?;
    let outcome = crate::fixture_edit::apply_request(world, &request);
    Ok(layout::apply_outcome(&outcome))
}

fn director_schema(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::director::SCHEMA.to_owned())
}

fn director_command(In(params): In<Option<Value>>) -> BrpResult {
    let command = params.ok_or_else(|| refuse("moly.director.command: params are the command"))?;
    crate::director::director_command(&command.to_string()).map_err(refuse)?;
    Ok(Value::Null)
}

fn director_snapshot(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::director::director_snapshot())
}

fn director_check(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.director.check")?;
    let condition = params.get("cond").ok_or_else(|| refuse("moly.director.check: params need cond"))?;
    let (holds, read) = crate::director::check(world, condition).map_err(refuse)?;
    Ok(json!({ "holds": holds, "read": read }))
}

fn server_schema(In(_): In<Option<Value>>, world: &mut World) -> BrpResult {
    if crate::userdata::admin_call(world, moly_protocol::AdminCall::Schema).is_none() {
        return Err(refuse("the server link is not installed"));
    }
    let Some(cache) = world.get_resource::<crate::userdata::ServerViewCache>() else {
        return Err(refuse("the server view cache is not installed"));
    };
    projection(cache.schema_text())
}

fn server_document(In(_): In<Option<Value>>, world: &mut World) -> BrpResult {
    if crate::userdata::admin_call(world, moly_protocol::AdminCall::Document).is_none() {
        return Err(refuse("the server link is not installed"));
    }
    let Some(cache) = world.get_resource::<crate::userdata::ServerViewCache>() else {
        return Err(refuse("the server view cache is not installed"));
    };
    projection(cache.document_text())
}

fn server_edit(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let mut edit = object(params, "moly.server.edit")?;
    edit.insert("type".into(), "server.edit".into());
    let Some(seq) = crate::userdata::admin_call(
        world,
        moly_protocol::AdminCall::Edit {
            command_json: Value::Object(edit).to_string(),
        },
    ) else {
        return Err(refuse("the server link is not installed"));
    };
    Ok(json!({ "pending": seq }))
}

fn server_result(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let params = object(params, "moly.server.result")?;
    let seq = params
        .get("seq")
        .and_then(Value::as_u64)
        .and_then(|seq| u32::try_from(seq).ok())
        .ok_or_else(|| refuse("moly.server.result: seq must be a whole number"))?;
    let Some(cache) = world.get_resource::<crate::userdata::ServerViewCache>() else {
        return Err(refuse("the server view cache is not installed"));
    };
    match &cache.last_edit {
        Some((at, Ok(receipt))) if *at == seq => {
            Ok(json!({ "seq": seq, "ok": true, "receipt": receipt }))
        }
        Some((at, Err(refusal))) if *at == seq => Ok(json!({
            "seq": seq,
            "ok": false,
            "code": refusal.code,
            "reason": refusal.reason,
        })),
        _ => Ok(json!({ "seq": seq, "pending": true })),
    }
}

fn game_snapshot(In(_): In<Option<Value>>) -> BrpResult {
    projection(crate::browser_game::game_snapshot())
}

fn perf_snapshot(In(params): In<Option<Value>>) -> BrpResult {
    let params = object(params, "moly.perf.snapshot")?;
    let frames = match params.get("frames") {
        None => moly_perf::DEFAULT_RECENT,
        Some(frames) => frames
            .as_u64()
            .filter(|frames| *frames > 0)
            .ok_or_else(|| refuse("moly.perf.snapshot: frames must be a positive integer"))? as usize,
    };
    projection(moly_perf::snapshot_json(frames))
}

/// `[x, y, z]` from a JSON array, for the free camera's two points.
fn vec3_param(value: Option<&Value>, method: &str, name: &str) -> Result<Vec3, BrpError> {
    let array = value
        .and_then(Value::as_array)
        .filter(|array| array.len() == 3)
        .ok_or_else(|| refuse(format!("{method}: {name} must be [x, y, z]")))?;
    let mut out = [0.0f32; 3];
    for (slot, value) in out.iter_mut().zip(array) {
        *slot = value
            .as_f64()
            .ok_or_else(|| refuse(format!("{method}: {name} must be [x, y, z] of numbers")))? as f32;
    }
    Ok(Vec3::from(out))
}

fn free_camera_global(
    pose: Res<crate::camera::FreeCameraPose>,
    mut camera: Query<(&Transform, &mut GlobalTransform), With<Camera3d>>,
) {
    if pose.0.is_none() {
        return;
    }
    if let Ok((transform, mut global)) = camera.single_mut() {
        *global = GlobalTransform::from(*transform);
    }
}

fn camera_free(
    In(params): In<Option<Value>>,
    mut pose: ResMut<crate::camera::FreeCameraPose>,
) -> BrpResult {
    let params = object(params, "moly.camera.free")?;
    let eye = vec3_param(params.get("position"), "moly.camera.free", "position")?;
    let target = vec3_param(params.get("target"), "moly.camera.free", "target")?;
    let up = match params.get("up") {
        Some(value) => vec3_param(Some(value), "moly.camera.free", "up")?,
        None => Vec3::Y,
    };
    let fov = match params.get("fov") {
        None => 45.0,
        Some(value) => value
            .as_f64()
            .filter(|fov| (5.0..=120.0).contains(fov))
            .ok_or_else(|| refuse("moly.camera.free: fov must be a number in 5..=120"))? as f32,
    };
    *pose = crate::camera::FreeCameraPose(Some(crate::camera::FreeCameraPoseValue {
        eye,
        target,
        up,
        fov_degrees: fov,
    }));
    Ok(json!({
        "free": true,
        "position": [eye.x, eye.y, eye.z],
        "target": [target.x, target.y, target.z],
        "up": [up.x, up.y, up.z],
        "fov": fov,
    }))
}

fn camera_reset(
    In(_): In<Option<Value>>,
    mut pose: ResMut<crate::camera::FreeCameraPose>,
) -> BrpResult {
    *pose = crate::camera::FreeCameraPose(None);
    Ok(json!({"free": false}))
}

/// Frames captured for `moly.capture.take`, by id; `None` while the frame
/// is still being read back.
#[derive(Resource, Default)]
struct Captures {
    next: u64,
    frames: BTreeMap<u64, Option<Value>>,
}

fn capture(In(_): In<Option<Value>>, mut commands: Commands, mut captures: ResMut<Captures>) -> BrpResult {
    captures.next += 1;
    let id = captures.next;
    captures.frames.insert(id, None);
    while captures.frames.len() > KEPT_CAPTURES {
        captures.frames.pop_first();
    }
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut captures: ResMut<Captures>| {
            let Some(slot) = captures.frames.get_mut(&id) else {
                return; // dropped as older than the kept frames
            };
            *slot = Some(match encode_png(&captured.image) {
                Ok(frame) => frame,
                Err(error) => json!({ "error": error }),
            });
        },
    );
    Ok(json!({ "id": id }))
}

/// The frame as base64 PNG without its alpha channel, which holds
/// brightness rather than coverage when HDR is on.
fn encode_png(image: &Image) -> Result<Value, String> {
    use base64::Engine as _;
    let rgb = image.clone().try_into_dynamic().map_err(|error| format!("frame: {error}"))?.to_rgb8();
    let mut png = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut png, image::ImageFormat::Png).map_err(|error| format!("png: {error}"))?;
    Ok(json!({
        "png": base64::engine::general_purpose::STANDARD.encode(png.into_inner()),
        "width": rgb.width(),
        "height": rgb.height(),
    }))
}

fn capture_take(In(params): In<Option<Value>>, mut captures: ResMut<Captures>) -> BrpResult {
    let params = object(params, "moly.capture.take")?;
    let id = params
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| refuse("moly.capture.take: params need the capture id"))?;
    match captures.frames.get(&id) {
        None => Err(refuse(format!("moly.capture.take: no capture {id} is kept"))),
        Some(None) => Ok(Value::Null),
        Some(Some(_)) => Ok(captures.frames.remove(&id).flatten().unwrap_or(Value::Null)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_moly_method_is_registered_and_listed() {
        let mut app = App::new();
        install(&mut app);
        let registered = app.world().resource::<RemoteMethods>().methods();
        let mut moly: Vec<&str> = registered.iter().map(String::as_str).filter(|name| name.starts_with("moly.")).collect();
        moly.sort_unstable();
        let mut listed: Vec<&str> = METHODS.iter().map(|(name, ..)| *name).collect();
        listed.sort_unstable();
        assert_eq!(moly, listed);
        for (name, _, params) in METHODS {
            serde_json::from_str::<Value>(params).unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }
}
