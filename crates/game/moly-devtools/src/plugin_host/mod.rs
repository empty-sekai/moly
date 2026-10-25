//! The plugin host: the Luau plugins the profile enables, run against the
//! game.
//!
//! Discovery: at startup the host reads the plugins directory of the game
//! (`plugins` beside the executable) and the user's (`plugins` in the data
//! directory: `MOLY_DATA_DIR` when it is set, else the launcher's data
//! folder). Each subdirectory of either with a `plugin.toml` is an installed
//! plugin. Both directories are offered to [`moly_plugin::resolve`], which
//! refuses an id installed in both, so a plugin has one directory and no
//! copy silently wins. The profile is `plugins/profile.toml` in the data
//! directory; without the file it is the empty profile, which enables
//! nothing. Each loaded plugin's settings are its manifest's `[config]` with
//! the profile's table over it.
//!
//! A plugin set that cannot start stops the game: every problem is logged
//! with `error!` and the app exits with an error. The Luau plugins load in
//! the resolved order, and a script that fails to load ends the start the
//! same way. With nothing installed or enabled the host does nothing and
//! logs nothing above `info`.
//!
//! Each frame, after the director's: the plugins' steps advance, then every
//! plugin runs a frame of its scripts (`moly.on("tick")` handlers, the
//! handlers of the events the bus recorded since the frame before, and the
//! tasks), then the store writes what changed. A plugin that keeps raising
//! errors is stopped, with the runtime's reason in the log and in
//! `moly.plugins`.
//!
//! `moly.observe` answers what the `moly.observe` method answers. A step
//! asked for with `moly.act` is checked when it is asked for, as a
//! [`crate::director::parse_step`] step, and runs as a track with its own
//! shared state; the task resumes with its outcome once the step ends:
//! `{ ok = true, value }` with the step's result (a `log`'s text, a
//! `where`'s positions) or `{ ok = false, reason }` with the refusal or the
//! failure. A step that ran to its end without a note takes `value` as
//! `null`. A refused step is never started.
//!
//! Control lease: the steps that drive the player through the virtual hand
//! ([`is_hand_action`]: `player.*`, `camera.*`, `tap`, `ui.tap` and
//! `dialog.answer`) belong to one owner at a time. A human is above the
//! director and a plugin is below it: while a director script runs, a plugin
//! hand step is refused by name, and a plugin hand step is refused by name
//! while another plugin's hand step holds the hand. The hand step that
//! starts first holds the hand until its track ends (done, failed or
//! aborted). A plugin asks for its steps once per frame, so the task that
//! asks for the hand first holds it. When human input arrives (a key press,
//! a mouse button, a touch, a gamepad button or stick) the hand step that
//! holds the hand is aborted, and it fails with [`HUMAN_INPUT`]: `human
//! input took control`.
//!
//! Store: each plugin has one JSON object at `plugins/store/<id>.json` in
//! the data directory, read when its scripts load and written once, at the
//! end of the frame its keys changed in, through a temporary file that is
//! renamed into place. A plugin that has saved nothing has no file.
//!
//! The Luau runtime holds values that cannot leave the main thread, so the
//! loaded set is a non-send resource; the plugins' report, which the control
//! surface reads from any thread, is kept as text beside it and rewritten at
//! the end of every frame.
//!
//! `moly.plugins` reports what the host loaded, what it stopped and why,
//! what the profile enabled that did not load, and the seam table.

mod lease;
mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use bevy::prelude::*;
use moly_plugin::luau::{Host, Limits, Runtime, Stopped};
use moly_plugin::manifest::{Manifest, PluginId, PluginKind};
use moly_plugin::resolve::{Installed, Resolved};
use moly_plugin::{Profile, SeamKind};
use serde_json::{json, Value};

use crate::director::run::{Shared, Track, TrackState};
use crate::director::{Step, ACTED_TICKET};

/// The reason a plugin's hand step fails with when a person takes the hand.
pub const HUMAN_INPUT: &str = "human input took control";

/// The reason a hand step is refused while another plugin's holds the hand.
pub const HAND_HELD: &str = "the hand is held by another plugin's step";

/// The reason a hand step is refused while a director script runs.
pub const DIRECTOR_HOLDS: &str = "a director script holds the hand";

/// The plugins directory of the game, beside the executable.
const GAME_DIR: &str = "plugins";
/// The profile's name in the data directory's plugins directory.
const PROFILE: &str = "profile.toml";

pub fn install(app: &mut App) {
    app.init_non_send::<PluginHost>()
        // The host's frame follows the director's, then swaps its report in
        // before the page the control surface answers from is written.
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (advance.after(crate::director::director_tick), publish_report).chain().before(crate::control::publish_report),
        );
}

/// The loaded plugin set, the store and the steps across frames. It keeps
/// the Luau runtime, so it is a non-send resource.
#[derive(Default)]
struct PluginHost {
    /// The data directory. `None` while it is unknown.
    data: Option<PathBuf>,
    /// The plugins of the resolved order, as `moly.plugins` lists them.
    installed: Vec<PluginRow>,
    /// What the profile enabled that did not load, and why.
    failed: Vec<(PluginId, String)>,
    /// The plugins the runtime stopped, and why.
    stopped: BTreeMap<PluginId, String>,
    /// The provider of each exclusive seam under its name, each target's
    /// under `name@target`; of each multi seam, its providers joined by
    /// `", "` under its name.
    seams: BTreeMap<&'static str, BTreeMap<String, String>>,
    /// The Luau runtime; `None` before start and while a frame runs it.
    runtime: Option<Runtime>,
    /// The shared state of every plugin's steps.
    shared: Shared,
    /// The bus sequence number the last frame read up to.
    events: u64,
    next_ticket: u64,
    /// The steps running now, by ticket.
    acts: BTreeMap<u64, Act>,
    /// The ticket of the hand step that holds the hand.
    hand: Option<u64>,
    /// The plugins still loaded, in the resolved order.
    running: Vec<PluginId>,
    /// Each plugin's saved data.
    store: BTreeMap<PluginId, store::File>,
    /// The plugins whose store changed this frame.
    dirty: BTreeSet<PluginId>,
}

/// One plugin of the resolved order, as `moly.plugins` lists it.
struct PluginRow {
    id: PluginId,
    dir: String,
    version: String,
    kind: &'static str,
    config: Value,
}

/// One step a plugin asked for.
struct Act {
    plugin: PluginId,
    track: Track,
    /// Whether the step's action drives the hand.
    hand: bool,
    /// The ticket the track runs under in the shared state.
    ticket: u64,
    /// The ended steps counted when the step was asked for: the step's own
    /// result is the record that follows.
    ended: usize,
    /// The outcome, once the step has ended, for the next poll.
    outcome: Option<Result<Value, String>>,
}

/// Whether a plugin set was loaded at start.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Startup: discovery, the profile, resolution and the scripts.
fn load(world: &mut World) {
    let data = match data_dir() {
        Ok(data) => data,
        Err(reason) => return fail(world, &reason),
    };
    let home = data.join(GAME_DIR);
    let profile_path = home.join(PROFILE);
    let profile = match Profile::read(&profile_path) {
        Ok(profile) => profile,
        Err(error) => {
            return fail(world, &format!("The plugin profile {} is not usable: {error}", profile_path.display()))
        }
    };
    if profile.plugins.is_empty() {
        // No plugin is enabled: nothing is read further, and nothing is
        // reported.
        world.non_send_mut::<PluginHost>().data = Some(data);
        return;
    }
    let enabled: Vec<PluginId> = profile.enabled().cloned().collect();
    let found = discover(&home, &data, &enabled, game_plugins());
    let Some(version) = crate::VERSION.parse().ok() else {
        return fail(world, &format!("The version {} is not semver, so plugin requirements cannot be checked.", crate::VERSION));
    };
    let resolved = match moly_plugin::resolve(&found.installed, &profile, &version) {
        Ok(resolved) => resolved,
        Err(errors) => {
            let reasons: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return refuse(world, reasons);
        }
    };
    let (runtime, mut failed) = start(&resolved);
    for (plugin, reason) in &found.unreadable {
        failed.push((plugin.clone(), reason.clone()));
    }
    if !failed.is_empty() {
        let reasons: Vec<String> = failed.iter().map(|(plugin, reason)| format!("{plugin} did not load: {reason}")).collect();
        return refuse(world, reasons);
    }
    for (dir, reason) in &found.refused {
        warn!("[plugins] {dir}: {reason}");
    }
    let seams = seam_table(&resolved);
    let loaded: Vec<PluginId> = runtime.running().cloned().collect();
    if !loaded.is_empty() {
        info!("[plugins] running {}", join_ids(&loaded));
    }
    let installed: Vec<PluginRow> = resolved
        .order
        .iter()
        .map(|unit| PluginRow {
            id: unit.manifest.id.clone(),
            dir: display(&unit.dir, &data),
            version: unit.manifest.version.to_string(),
            kind: match unit.manifest.kind {
                PluginKind::Luau => "luau",
                PluginKind::Native => "native",
            },
            config: toml_json(&resolved.config[&unit.manifest.id]),
        })
        .collect();
    let running = loaded;
    let store: BTreeMap<PluginId, store::File> = resolved
        .order
        .iter()
        .map(|unit| (unit.manifest.id.clone(), store::File::read(&home, &unit.manifest.id)))
        .collect();
    let events = lease::bus_latest();
    let mut host = world.non_send_mut::<PluginHost>();
    host.data = Some(data);
    host.installed = installed;
    host.failed = failed;
    host.seams = seams;
    host.runtime = Some(runtime);
    host.running = running;
    host.store = store;
    host.events = events;
    write_report(&host);
    STARTED.store(true, Ordering::Relaxed);
}

/// The installed plugins of both directories, and the directories that
/// could not be used as one.
#[derive(Default)]
struct Found {
    installed: Vec<Installed>,
    /// An enabled plugin whose directory cannot be used: its id, and why.
    unreadable: Vec<(PluginId, String)>,
    /// A directory no enabled plugin owns, and why it cannot be used.
    refused: Vec<(String, String)>,
}

/// The subdirectories of the data directory's `plugins` and of the game's
/// with a `plugin.toml`.
fn discover(home: &Path, data: &Path, enabled: &[PluginId], game: Option<PathBuf>) -> Found {
    let mut roots = vec![home.to_owned()];
    roots.extend(game);
    let mut found = Found::default();
    for root in roots {
        for dir in list(&root) {
            read_dir(&mut found, dir, data, enabled);
        }
    }
    found
}

/// Reads one plugin directory into `found`: an installed plugin, or the
/// reason it cannot be used.
fn read_dir(found: &mut Found, dir: PathBuf, data: &Path, enabled: &[PluginId]) {
    // A directory the profile enables stops the set; any other is logged.
    let refuse = |found: &mut Found, named: &str, id: PluginId, reason: String| {
        let reason = format!("{named}: {reason}");
        if enabled.contains(&id) {
            found.unreadable.push((id, reason));
        } else {
            found.refused.push((named.to_owned(), reason));
        }
    };
    let manifest = match Manifest::read(&dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            // A manifest that does not parse has no id; the profile names
            // plugins by id, so only the directory's name can tell.
            let Some(id) = dir.file_name().and_then(|name| PluginId::new(&name.to_string_lossy()).ok()) else {
                found.refused.push((display(&dir, data), format!("its plugin.toml is not usable: {error}")));
                return;
            };
            refuse(found, &display(&dir, data), id, format!("its plugin.toml is not usable: {error}"));
            return;
        }
    };
    let id = manifest.id.clone();
    match manifest.entry.as_deref() {
        Some(entry) if !dir.join(entry).is_file() => {
            refuse(found, &display(&dir, data), id, format!("its entry script {entry} does not exist"));
        }
        _ => found.installed.push(Installed { dir, manifest }),
    }
}

/// The subdirectories of `dir` with a `plugin.toml`; none when it cannot be
/// read.
fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("plugin.toml").is_file())
        .collect()
}

/// A directory as the report names it: relative to the data directory when
/// it is inside it, absolute otherwise.
fn display(dir: &Path, data: &Path) -> String {
    match dir.strip_prefix(data) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => dir.display().to_string(),
    }
}

/// The game's plugins directory (`plugins` beside the executable), when the
/// executable's folder is known.
fn game_plugins() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(GAME_DIR))
}

/// Runs each resolved plugin's entry script, in the resolved order; the ones
/// that fail are returned with the runtime's reason.
fn start(resolved: &Resolved) -> (Runtime, Vec<(PluginId, String)>) {
    let mut runtime = Runtime::new(Limits::default());
    let mut failed = Vec::new();
    for unit in &resolved.order {
        if unit.manifest.kind == PluginKind::Native {
            continue;
        }
        let Some(entry) = &unit.manifest.entry else { continue };
        let id = unit.manifest.id.clone();
        let config = &resolved.config[&id];
        if let Err(error) = runtime.load(id.clone(), &unit.dir, entry, config) {
            failed.push((id, error.to_string()));
        }
    }
    (runtime, failed)
}

/// The provider of each seam after resolution.
fn seam_table(resolved: &Resolved) -> BTreeMap<&'static str, BTreeMap<String, String>> {
    let mut table = BTreeMap::new();
    for decl in moly_plugin::seam::SEAMS {
        let mut providers = BTreeMap::new();
        match decl.kind {
            SeamKind::Exclusive => {
                if let Some(seam) = resolved.seams.exclusive.get(decl.name) {
                    providers.insert(decl.name.to_owned(), seam.whole.name());
                    for target in seam.targets.keys() {
                        let key = format!("{}@{target}", decl.name);
                        providers.insert(key, resolved.seams.provider(decl.name, Some(target)).name());
                    }
                }
            }
            SeamKind::Multi => {
                if let Some(list) = resolved.seams.multi.get(decl.name) {
                    providers.insert(decl.name.to_owned(), join_ids(list));
                }
            }
        }
        if !providers.is_empty() {
            table.insert(decl.name, providers);
        }
    }
    table
}

/// Startup refuses the set: every problem is logged with its own line, then
/// the reason the game exits with.
fn refuse(world: &mut World, reasons: Vec<String>) {
    for reason in &reasons {
        error!("[plugins] {reason}");
    }
    let count = reasons.len();
    fail(world, &format!("The plugin set cannot start for {count} problem(s); see above."));
}

/// Startup refuses: the reason is logged and the game exits with an error.
fn fail(world: &mut World, reason: &str) {
    error!("[plugins] {reason}");
    world.write_message(AppExit::error());
}

/// Each frame, after the director's: the steps, the scripts, the store and
/// the report.
fn advance(world: &mut World) {
    if !STARTED.load(Ordering::Relaxed) {
        return;
    }
    let dt = world.resource::<Time>().delta_secs_f64();
    let now = world.resource::<Time>().elapsed_secs();
    let human = lease::human_input(world);
    let refusing = crate::director::script_running(world);
    // The frame's scripts see the world through the host, so the host is
    // taken out of it for the frame.
    let mut host = world.remove_non_send::<PluginHost>().expect("the plugin host is installed at start");
    host.shared.now = now;
    let events = lease::bus_since(host.events);
    host.events = lease::bus_latest();
    if human {
        abort_hand(world, &mut host);
    }
    advance_steps(world, &mut host, refusing);
    // The runtime is taken out of the host for the frame, so the scripts can
    // reach the host through the frame.
    let Some(mut runtime) = host.runtime.take() else {
        world.insert_non_send(host);
        return;
    };
    let stopped = runtime.tick(&mut Frame { host: &mut host, world }, dt, &events);
    host.runtime = Some(runtime);
    host.running.retain(|running| !stopped.iter().any(|stop| stop.plugin == *running));
    for Stopped { plugin, reason } in stopped {
        error!("[plugins] {plugin} {reason}");
        host.stopped.insert(plugin, reason);
    }
    write_store(&mut host);
    world.insert_non_send(host);
}

/// Writes the store of each plugin that changed it this frame.
fn write_store(host: &mut PluginHost) {
    let dirty = std::mem::take(&mut host.dirty);
    let Some(data) = host.data.clone() else {
        return;
    };
    let home = data.join(GAME_DIR);
    for id in dirty {
        let Some(file) = host.store.get_mut(&id) else { continue };
        let written = file.write(&home, &id);
        if let Err(error) = written {
            warn!("[plugins] {id}: {error}");
        }
    }
}

/// Offers the host's report to the control surface's page for this frame.
fn publish_report(world: &mut World) {
    if let Some(host) = world.get_non_send::<PluginHost>() {
        write_report(host);
    }
}

/// Aborts the hand step that holds the hand, if any: its task resumes with
/// [`HUMAN_INPUT`] as the step's failure.
fn abort_hand(world: &mut World, host: &mut PluginHost) {
    let Some(ticket) = host.hand.take() else {
        return;
    };
    let Some(act) = host.acts.get_mut(&ticket) else { return };
    act.track.abort(world, &mut host.shared, act.ticket);
    act.outcome = Some(Err(HUMAN_INPUT.to_owned()));
    let shared_ticket = act.ticket;
    host.shared.release(world, shared_ticket);
}

/// The host as one frame's scripts see it: a step's control is refused while
/// a director script runs, which is read from the world at hand.
struct Frame<'a> {
    host: &'a mut PluginHost,
    world: &'a mut World,
}

impl Host for Frame<'_> {
    fn observe(&mut self, _: &PluginId, what: &[String]) -> Result<Value, String> {
        crate::control::observe(self.world, what)
    }

    fn act(&mut self, plugin: &PluginId, step: Value) -> Result<u64, String> {
        let steps = crate::director::parse_step(&step).map_err(|reason| format!("step refused: {reason}"))?;
        let hand = is_hand(&steps);
        if hand {
            if crate::director::script_running(self.world) {
                return Err(DIRECTOR_HOLDS.into());
            }
            if let Some(holder) = self.host.hand.and_then(|ticket| self.host.acts.get(&ticket)) {
                return Err(format!("{HAND_HELD} ({})", holder.plugin));
            }
        }
        self.host.next_ticket += 1;
        let ticket = self.host.next_ticket;
        let ended = self.host.shared.ended();
        let track = Track::new(steps, format!("{plugin}/act"));
        if hand {
            // The hand is lent before the step's own track runs, so the step
            // that asks first holds it.
            self.host.hand = Some(ticket);
        }
        let act = Act { plugin: plugin.clone(), track, hand, ticket: ACTED_TICKET | ticket, ended, outcome: None };
        self.host.acts.insert(ticket, act);
        Ok(ticket)
    }

    fn poll(&mut self, ticket: u64) -> Option<Result<Value, String>> {
        let act = self.host.acts.get_mut(&ticket)?;
        act.outcome.take()
    }

    fn log(&mut self, plugin: &PluginId, line: &str) {
        info!("[plugins] {plugin}: {line}");
    }

    fn store_get(&mut self, plugin: &PluginId, key: &str) -> Option<Value> {
        self.host.store.get(plugin).and_then(|file| file.get(key))
    }

    fn store_set(&mut self, plugin: &PluginId, key: &str, value: Option<Value>) {
        let file = self.host.store.entry(plugin.clone()).or_default();
        if file.set(key, value) {
            self.host.dirty.insert(plugin.clone());
        }
    }
}

/// Advances the plugins' steps, the one that holds the hand first, and lets
/// the hand go when its step ends. While a director script runs no hand step
/// starts.
fn advance_steps(world: &mut World, host: &mut PluginHost, refusing: bool) {
    let holding = host.hand;
    let mut tickets: Vec<u64> = holding.into_iter().collect();
    tickets.extend(host.acts.keys().copied().filter(|ticket| Some(*ticket) != holding));
    let mut ended = Vec::new();
    for ticket in tickets {
        let Some(act) = host.acts.get_mut(&ticket) else { continue };
        if act.outcome.is_some() {
            // The task has not read the outcome yet.
            continue;
        }
        if act.hand && Some(ticket) != holding {
            // Another step holds the hand: this one is refused by name.
            act.outcome = Some(Err(HAND_HELD.to_owned()));
            ended.push(ticket);
            continue;
        }
        if act.hand && refusing {
            // The director's script holds the hand: the step waits for its
            // end.
            continue;
        }
        let state = act.track.tick(world, &mut host.shared, act.ticket);
        // A step that ended reports its own result, the last note its track
        // recorded.
        let value = if matches!(state, TrackState::Running) {
            Value::Null
        } else {
            let notes = host.shared.ended_after(act.ticket, act.ended);
            notes.last().map_or(Value::Null, |note| json!(note))
        };
        match state {
            TrackState::Running => {}
            TrackState::Done => {
                act.outcome = Some(Ok(json!({ "value": value })));
                ended.push(ticket);
            }
            TrackState::Failed(reason) => {
                act.outcome = Some(Err(reason));
                ended.push(ticket);
            }
        }
    }
    for ticket in ended {
        let Some(act) = host.acts.get(&ticket) else { continue };
        // The hand is free again, and whatever the step held is let go; its
        // outcome waits for the task's next poll.
        if host.hand == Some(ticket) {
            host.hand = None;
        }
        let shared_ticket = act.ticket;
        host.shared.release(world, shared_ticket);
    }
}

/// Whether the steps drive the player through the virtual hand.
pub fn is_hand(steps: &[Step]) -> bool {
    steps.iter().any(|step| is_hand_action(&step.action))
}

/// Whether an action drives the player through the virtual hand:
/// `player.*`, `camera.*`, `tap`, `ui.tap` and `dialog.answer`.
pub fn is_hand_action(action: &str) -> bool {
    matches!(action, "tap" | "ui.tap" | "dialog.answer") || action.starts_with("player.") || action.starts_with("camera.")
}

fn join_ids(ids: &[PluginId]) -> String {
    ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

/// The data directory: `MOLY_DATA_DIR` when it is set and not empty, else the
/// launcher's data folder (its locations file, or the default folder of the
/// home directory).
fn data_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(moly_store::location::DATA_DIR_ENV) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    moly_store::location::Locations::load()?.data_dir()
}

/// A settings table as JSON. Its text is not kept, only what the table holds:
/// numbers, strings, booleans, lists and nested tables.
fn toml_json(table: &moly_plugin::toml::Table) -> Value {
    serde_json::from_str(&moly_plugin::toml::to_string(table).unwrap_or_default()).unwrap_or(Value::Null)
}

/// Writes the report `moly.plugins` answers from the host's state.
fn write_report(host: &PluginHost) {
    let failed: BTreeMap<PluginId, String> = host.failed.iter().cloned().collect();
    let report = json!({
        "plugins": host
            .installed
            .iter()
            .map(|plugin| json!({
                "id": plugin.id.as_str(),
                "dir": plugin.dir,
                "version": plugin.version,
                "kind": plugin.kind,
                "running": host.running.contains(&plugin.id),
                "config": plugin.config,
            }))
            .collect::<Vec<_>>(),
        "stopped": rows(&host.stopped),
        "failed": rows(&failed),
        "seams": host.seams,
    });
    if let Ok(mut published) = REPORT.lock() {
        *published = report.to_string();
    }
}

/// The report, read by the control surface from any thread.
static REPORT: Mutex<String> = Mutex::new(String::new());

/// The report as the control surface copies it into `moly.plugins`: the four
/// lists, empty when no plugin set was loaded.
pub fn report() -> Value {
    let text = snapshot();
    match serde_json::from_str::<Value>(&text) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({ "plugins": [], "stopped": [], "failed": [], "seams": {} }),
    }
}

/// Plugins and their reasons, as the report lists them.
fn rows(rows: &BTreeMap<PluginId, String>) -> Vec<Value> {
    rows.iter().map(|(plugin, reason)| json!({ "id": plugin.as_str(), "reason": reason })).collect()
}

/// The plugins report, as JSON text: `{plugins, stopped, failed, seams}`.
pub fn snapshot() -> String {
    REPORT.lock().map(|report| report.clone()).unwrap_or_default()
}
