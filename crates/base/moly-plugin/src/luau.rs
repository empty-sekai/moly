//! The Luau runtime for plugin scripts.
//!
//! Each plugin runs in its own Luau state, sandboxed: the standard library
//! without `io`, `os`, `debug` or `package`, read-only globals, a memory
//! limit, and an instruction budget per frame. The script sees one global,
//! `moly`, the plugin interface:
//!
//! | | |
//! |---|---|
//! | `moly.id`, `moly.config` | the plugin's id and its resolved settings |
//! | `moly.on(name, fn)` | `"tick"` (called with the frame's delta in seconds), an event kind, or `"*"` for every event |
//! | `moly.spawn(fn, ...)` | start a task: a coroutine that may wait |
//! | `moly.observe(what?)` | the world as structured data, by section |
//! | `moly.act(step)` | run one action step and wait for it (tasks only); returns `{ ok = true, ... }` or `{ ok = false, reason = "..." }` |
//! | `moly.wait(seconds)` | wait inside a task |
//! | `moly.now()` | seconds of game time since the plugin started |
//! | `moly.random()` | a random number in [0, 1), drawn from the machine at each call (the sandboxed `math.random` repeats a run's seed) |
//! | `moly.log(...)` | write a line to the game log |
//! | `moly.store.get(key)`, `moly.store.set(key, value)` | the plugin's own saved data |
//!
//! The game implements [`Host`]; [`Runtime::tick`] lends it to the scripts
//! for one frame. Host functions exist only during that frame, so a script
//! that keeps a reference to one cannot call it later.
//!
//! A plugin that raises errors or exceeds its budget on
//! [`Limits::strikes`] consecutive frames is stopped, and the reason is
//! returned to the game.

use std::{
    cell::{Cell, RefCell},
    fmt,
    path::{Component, Path, PathBuf},
    rc::Rc,
};

use mlua::{Function, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Thread, Value, VmState};
use mlua::prelude::LuaThreadStatus as ThreadStatus;

use crate::manifest::PluginId;

/// What a plugin may do to the game. Implemented by the game.
pub trait Host {
    /// Read the named sections (all when empty).
    fn observe(&mut self, plugin: &PluginId, what: &[String]) -> Result<serde_json::Value, String>;
    /// Start one action step. `Ok` is a ticket for [`Host::poll`]; `Err` a
    /// refusal reason given at once.
    fn act(&mut self, plugin: &PluginId, step: serde_json::Value) -> Result<u64, String>;
    /// The outcome of a started step; `None` while it runs. `Ok` carries
    /// the step's result fields, `Err` its refusal or failure reason.
    fn poll(&mut self, ticket: u64) -> Option<Result<serde_json::Value, String>>;
    fn log(&mut self, plugin: &PluginId, line: &str);
    fn store_get(&mut self, plugin: &PluginId, key: &str) -> Option<serde_json::Value>;
    /// `None` removes the key.
    fn store_set(&mut self, plugin: &PluginId, key: &str, value: Option<serde_json::Value>);
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes of Luau memory per plugin.
    pub memory: usize,
    /// Interrupt checks per plugin per frame (Luau checks at calls and loop
    /// back edges). A task that runs out yields until the next frame; a
    /// handler that runs out raises an error.
    pub instructions: u64,
    /// Consecutive failing frames before the plugin is stopped.
    pub strikes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory: 64 << 20,
            instructions: 200_000,
            strikes: 3,
        }
    }
}

#[derive(Debug)]
pub enum PluginError {
    Io { path: PathBuf, error: String },
    /// The entry script or a required module failed to load or run.
    Script(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "cannot read {}: {error}", path.display()),
            Self::Script(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for PluginError {}

/// A plugin the runtime stopped, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub plugin: PluginId,
    pub reason: String,
}

/// The Luau side of `moly.*`: the parts that need yielding are written in
/// Luau, over host functions that the runtime installs in `hostref.h` for
/// the duration of each frame.
const PRELUDE: &str = r#"
local moly, hostref, handlers, tasks = ...

local function host()
    local h = rawget(hostref, "h")
    if h == nil then
        error("the plugin interface is available from the first frame on, not while the script loads", 3)
    end
    return h
end

function moly.on(name, fn)
    assert(type(name) == "string", "moly.on: the name must be a string")
    assert(type(fn) == "function", "moly.on: the handler must be a function")
    local list = handlers[name]
    if list == nil then
        list = {}
        handlers[name] = list
    end
    table.insert(list, fn)
end

function moly.spawn(fn, ...)
    assert(type(fn) == "function", "moly.spawn: expects a function")
    local co = coroutine.create(fn)
    table.insert(tasks, { co = co, args = table.pack(...) })
end

function moly.observe(what)
    if what ~= nil and type(what) ~= "table" then
        error("moly.observe: expects a list of section names", 2)
    end
    return host().observe(what or {})
end

function moly.act(step)
    if type(step) ~= "table" then
        error("moly.act: expects a step table", 2)
    end
    if not coroutine.isyieldable() then
        error("moly.act waits for the step to finish, so it runs only inside a task started with moly.spawn", 2)
    end
    local ticket, refusal = host().act(step)
    if ticket == nil then
        return { ok = false, reason = refusal }
    end
    while true do
        local done, result = host().poll(ticket)
        if done then
            return result
        end
        coroutine.yield()
    end
end

function moly.wait(seconds)
    if not coroutine.isyieldable() then
        error("moly.wait runs only inside a task started with moly.spawn", 2)
    end
    local until_ = rawget(hostref, "now") + (seconds or 0)
    repeat
        coroutine.yield()
    until rawget(hostref, "now") >= until_
end

function moly.now()
    return rawget(hostref, "now")
end

function moly.random()
    return host().random()
end

function moly.log(...)
    local parts = table.pack(...)
    for i = 1, parts.n do
        parts[i] = tostring(parts[i])
    end
    local line = table.concat(parts, " ", 1, parts.n)
    local h = rawget(hostref, "h")
    if h == nil then
        -- While the script loads, the line waits for the first frame.
        local pending = rawget(hostref, "pending")
        if pending == nil then
            pending = {}
            rawset(hostref, "pending", pending)
        end
        table.insert(pending, line)
        return
    end
    h.log(line)
end

moly.store = {
    get = function(key)
        assert(type(key) == "string", "moly.store.get: the key must be a string")
        return host().store_get(key)
    end,
    set = function(key, value)
        assert(type(key) == "string", "moly.store.set: the key must be a string")
        host().store_set(key, value)
    end,
}
table.freeze(moly.store)
"#;

struct Task {
    thread: Thread,
    /// Arguments for the first resume.
    args: Option<mlua::MultiValue>,
}

struct Plugin {
    id: PluginId,
    lua: Lua,
    /// `hostref.h` holds the frame's host table; `hostref.now` the clock.
    hostref: Table,
    handlers: Table,
    /// Tasks queued by `moly.spawn` and not yet adopted.
    spawned: Table,
    tasks: Vec<Task>,
    budget: Rc<Budget>,
    now: f64,
    strikes: u32,
}

/// A random number in [0, 1) from the machine's own source (`moly.random`):
/// the top 53 bits of a scrambled clock reading, divided by 2^53.
struct UnitRandom;

impl UnitRandom {
    fn draw() -> f64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64);
        let mut mix = nanos ^ (nanos >> 17) ^ 0x9E37_79B9_7F4A_7C15;
        let mut value = 0u64;
        for _ in 0..53 {
            mix = mix.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            value = (value << 1) | (mix >> 63);
        }
        value as f64 / 9_007_199_254_740_992.0
    }
}

#[derive(Default)]
struct Budget {
    remaining: Cell<u64>,
    /// Whether the code running now is a task (which may yield).
    in_task: Cell<bool>,
}

pub struct Runtime {
    limits: Limits,
    plugins: Vec<Plugin>,
}

impl Runtime {
    pub fn new(limits: Limits) -> Self {
        Self { limits, plugins: Vec::new() }
    }

    /// The plugins still running, in load order.
    pub fn running(&self) -> impl Iterator<Item = &PluginId> {
        self.plugins.iter().map(|plugin| &plugin.id)
    }

    /// Load a plugin: run its entry script once. The script registers
    /// handlers and tasks; the plugin interface proper starts on the first
    /// [`Runtime::tick`].
    pub fn load(
        &mut self,
        id: PluginId,
        dir: &Path,
        entry: &str,
        config: &toml::Table,
    ) -> Result<(), PluginError> {
        let script = |error: mlua::Error| PluginError::Script(format!("{id}: {error}"));
        let libs = StdLib::COROUTINE
            | StdLib::TABLE
            | StdLib::STRING
            | StdLib::UTF8
            | StdLib::BIT
            | StdLib::MATH
            | StdLib::BUFFER
            | StdLib::VECTOR;
        let lua = Lua::new_with(libs, LuaOptions::default()).map_err(script)?;
        lua.set_memory_limit(self.limits.memory).map_err(script)?;

        // Loading (the interface prelude and the entry script) runs on one
        // frame's budget.
        let budget = Rc::new(Budget::default());
        budget.remaining.set(self.limits.instructions);
        let interrupt_budget = budget.clone();
        lua.set_interrupt(move |_| {
            let left = interrupt_budget.remaining.get();
            if left > 0 {
                interrupt_budget.remaining.set(left - 1);
                return Ok(VmState::Continue);
            }
            if interrupt_budget.in_task.get() {
                Ok(VmState::Yield)
            } else {
                Err(mlua::Error::runtime("instruction budget for this frame exceeded"))
            }
        });

        let moly = lua.create_table().map_err(script)?;
        moly.set("id", id.as_str()).map_err(script)?;
        moly.set("config", lua.to_value(config).map_err(script)?).map_err(script)?;
        let hostref = lua.create_table().map_err(script)?;
        hostref.set("now", 0.0).map_err(script)?;
        let handlers = lua.create_table().map_err(script)?;
        let spawned = lua.create_table().map_err(script)?;
        lua.load(PRELUDE)
            .set_name("=moly")
            .call::<()>((moly.clone(), hostref.clone(), handlers.clone(), spawned.clone()))
            .map_err(script)?;
        moly.set_readonly(true);

        lua.globals().set("moly", moly).map_err(script)?;
        lua.globals()
            .set("require", require_function(&lua, dir.to_owned()).map_err(script)?)
            .map_err(script)?;
        // Scripts come from files only: no compiling strings at run time.
        lua.globals().set("loadstring", Value::Nil).map_err(script)?;
        lua.sandbox(true).map_err(script)?;

        let source = read_module(dir, entry)?;
        lua.load(source)
            .set_name(format!("@{id}/{entry}"))
            .exec()
            .map_err(script)?;

        self.plugins.push(Plugin {
            id,
            lua,
            hostref,
            handlers,
            spawned,
            tasks: Vec::new(),
            budget,
            now: 0.0,
            strikes: 0,
        });
        Ok(())
    }

    /// Run one frame: the `tick` handlers, the handlers of each event, then
    /// every task until it waits. Returns the plugins stopped this frame.
    pub fn tick(&mut self, host: &mut dyn Host, dt: f64, events: &[serde_json::Value]) -> Vec<Stopped> {
        let host = RefCell::new(host);
        let limits = self.limits;
        let mut stopped = Vec::new();
        self.plugins.retain_mut(|plugin| match plugin.frame(&host, dt, events, limits) {
            Ok(()) => {
                plugin.strikes = 0;
                true
            }
            Err(error) => {
                plugin.strikes += 1;
                let line = format!("error: {error}");
                host.borrow_mut().log(&plugin.id, &line);
                if plugin.strikes < limits.strikes {
                    return true;
                }
                stopped.push(Stopped {
                    plugin: plugin.id.clone(),
                    reason: format!("stopped after {} failing frames in a row; last error: {error}", plugin.strikes),
                });
                false
            }
        });
        stopped
    }
}

impl Plugin {
    fn frame(
        &mut self,
        host: &RefCell<&mut dyn Host>,
        dt: f64,
        events: &[serde_json::Value],
        limits: Limits,
    ) -> mlua::Result<()> {
        self.now += dt;
        self.hostref.set("now", self.now)?;
        let Plugin { id, lua, hostref, handlers, spawned, tasks, budget, .. } = self;
        budget.remaining.set(limits.instructions);
        let (id, lua) = (&*id, &*lua);
        let result = lua.scope(|scope| {
            let h = lua.create_table()?;
            h.set(
                "observe",
                scope.create_function(|lua, what: Vec<String>| {
                    let read = host.borrow_mut().observe(id, &what).map_err(mlua::Error::runtime)?;
                    lua.to_value(&read)
                })?,
            )?;
            h.set(
                "act",
                scope.create_function(|lua, step: Value| {
                    let step: serde_json::Value = lua.from_value(step)?;
                    Ok(match host.borrow_mut().act(id, step) {
                        Ok(ticket) => (Some(ticket), None),
                        Err(reason) => (None, Some(reason)),
                    })
                })?,
            )?;
            h.set(
                "poll",
                scope.create_function(|lua, ticket: u64| {
                    let Some(outcome) = host.borrow_mut().poll(ticket) else {
                        return Ok((false, Value::Nil));
                    };
                    let result = lua.create_table()?;
                    match outcome {
                        Ok(serde_json::Value::Object(fields)) => {
                            for (key, value) in fields {
                                result.set(key, lua.to_value(&value)?)?;
                            }
                            result.set("ok", true)?;
                        }
                        Ok(serde_json::Value::Null) => result.set("ok", true)?,
                        Ok(other) => {
                            result.set("ok", true)?;
                            result.set("value", lua.to_value(&other)?)?;
                        }
                        Err(reason) => {
                            result.set("ok", false)?;
                            result.set("reason", reason)?;
                        }
                    }
                    Ok((true, Value::Table(result)))
                })?,
            )?;
            h.set(
                "log",
                scope.create_function(|_, line: String| {
                    host.borrow_mut().log(id, &line);
                    Ok(())
                })?,
            )?;
            h.set(
                "store_get",
                scope.create_function(|lua, key: String| match host.borrow_mut().store_get(id, &key) {
                    Some(value) => lua.to_value(&value),
                    None => Ok(Value::Nil),
                })?,
            )?;
            h.set(
                "store_set",
                scope.create_function(|lua, (key, value): (String, Value)| {
                    let value = match value {
                        Value::Nil => None,
                        value => Some(lua.from_value::<serde_json::Value>(value)?),
                    };
                    host.borrow_mut().store_set(id, &key, value);
                    Ok(())
                })?,
            )?;
            // Drawn whenever it is called: the machine's own random source,
            // so a run's seed does not repeat the script's choices.
            h.set(
                "random",
                scope.create_function(|_, ()| Ok(UnitRandom::draw()))?,
            )?;
            hostref.set("h", h)?;
            // Lines logged while the script loaded come out on its first frame.
            if let Some(pending) = hostref.get::<Option<Table>>("pending")? {
                hostref.set("pending", Value::Nil)?;
                for line in pending.sequence_values::<String>() {
                    host.borrow_mut().log(id, &line?);
                }
            }
            let run = run_frame(lua, handlers, spawned, tasks, budget, dt, events);
            hostref.set("h", Value::Nil)?;
            run
        });
        budget.in_task.set(false);
        result
    }

}

fn run_frame(
    lua: &Lua,
    handlers: &Table,
    spawned: &Table,
    tasks: &mut Vec<Task>,
    budget: &Budget,
    dt: f64,
    events: &[serde_json::Value],
) -> mlua::Result<()> {
        budget.in_task.set(false);
        if let Ok(list) = handlers.get::<Table>("tick") {
            for handler in list.sequence_values::<Function>() {
                handler?.call::<()>(dt)?;
            }
        }
        for event in events {
            let kind = event.get("kind").and_then(serde_json::Value::as_str).unwrap_or("");
            let mut lists = Vec::new();
            if let Ok(list) = handlers.get::<Table>(kind) {
                lists.push(list);
            }
            if let Ok(list) = handlers.get::<Table>("*") {
                lists.push(list);
            }
            if lists.is_empty() {
                continue;
            }
            let value = lua.to_value(event)?;
            for list in lists {
                for handler in list.sequence_values::<Function>() {
                    handler?.call::<()>(value.clone())?;
                }
            }
        }
        adopt_spawned(spawned, tasks)?;
        budget.in_task.set(true);
        let mut failure = None;
        let mut index = 0;
        while index < tasks.len() {
            let task = &mut tasks[index];
            let resumed = match task.args.take() {
                Some(args) => task.thread.resume::<()>(args),
                None => task.thread.resume::<()>(()),
            };
            let finished = task.thread.status() != ThreadStatus::Resumable;
            if let Err(error) = resumed {
                failure.get_or_insert(error);
            }
            if finished {
                tasks.remove(index);
            } else {
                index += 1;
            }
            // Tasks started by a task run from the next frame on.
        }
        budget.in_task.set(false);
        adopt_spawned(spawned, tasks)?;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

fn adopt_spawned(spawned: &Table, tasks: &mut Vec<Task>) -> mlua::Result<()> {
        let count = spawned.raw_len();
        for index in 1..=count {
            let entry: Table = spawned.raw_get(index)?;
            let thread: Thread = entry.get("co")?;
            let packed: Table = entry.get("args")?;
            let n: usize = packed.get("n")?;
            let mut args = mlua::MultiValue::new();
            for arg in 1..=n {
                args.push_back(packed.raw_get(arg)?);
            }
            tasks.push(Task { thread, args: Some(args) });
        }
        spawned.clear()?;
        Ok(())
    }

/// `require("name")` loads `<plugin dir>/name.luau` once and returns what
/// it returns. Names are relative paths inside the plugin directory.
fn require_function(lua: &Lua, dir: PathBuf) -> mlua::Result<Function> {
    let loaded = lua.create_table()?;
    lua.create_function(move |lua, name: String| {
        if let Some(module) = loaded.raw_get::<Option<Value>>(name.as_str())? {
            return Ok(module);
        }
        let file = format!("{name}.luau");
        let source = read_module(&dir, &file).map_err(|error| mlua::Error::runtime(error.to_string()))?;
        let module: Value = lua.load(source).set_name(format!("@{file}")).call(())?;
        let module = if module.is_nil() { Value::Boolean(true) } else { module };
        loaded.raw_set(name.as_str(), module.clone())?;
        Ok(module)
    })
}

/// Read a script inside the plugin directory; paths that leave it are refused.
fn read_module(dir: &Path, relative: &str) -> Result<String, PluginError> {
    let path = Path::new(relative);
    let inside = path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if !inside {
        return Err(PluginError::Script(format!(
            "{relative}: a script path must stay inside the plugin directory"
        )));
    }
    let full = dir.join(path);
    std::fs::read_to_string(&full).map_err(|error| PluginError::Io { path: full, error: error.to_string() })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    /// A host that records what the scripts did. `act` hands out tickets
    /// whose outcome is ready after `act_frames` polls.
    #[derive(Default)]
    struct Recorder {
        lines: Vec<String>,
        acts: Vec<serde_json::Value>,
        pending: HashMap<u64, u32>,
        act_frames: u32,
        store: HashMap<String, serde_json::Value>,
    }

    impl Host for Recorder {
        fn observe(&mut self, _: &PluginId, what: &[String]) -> Result<serde_json::Value, String> {
            Ok(json!({ "asked": what }))
        }
        fn act(&mut self, _: &PluginId, step: serde_json::Value) -> Result<u64, String> {
            if step.get("do").and_then(serde_json::Value::as_str) == Some("refused") {
                return Err("no such step".into());
            }
            self.acts.push(step);
            let ticket = self.acts.len() as u64;
            self.pending.insert(ticket, self.act_frames);
            Ok(ticket)
        }
        fn poll(&mut self, ticket: u64) -> Option<Result<serde_json::Value, String>> {
            let left = self.pending.get_mut(&ticket)?;
            if *left > 0 {
                *left -= 1;
                return None;
            }
            self.pending.remove(&ticket);
            Some(Ok(json!({ "ticket": ticket })))
        }
        fn log(&mut self, _: &PluginId, line: &str) {
            self.lines.push(line.to_owned());
        }
        fn store_get(&mut self, _: &PluginId, key: &str) -> Option<serde_json::Value> {
            self.store.get(key).cloned()
        }
        fn store_set(&mut self, _: &PluginId, key: &str, value: Option<serde_json::Value>) {
            match value {
                Some(value) => self.store.insert(key.to_owned(), value),
                None => self.store.remove(key),
            };
        }
    }

    /// Load `files` (with `main.luau` as the entry) from a directory unique
    /// to the test.
    fn load(runtime: &mut Runtime, name: &str, files: &[(&str, &str)]) -> Result<(), PluginError> {
        let dir = std::env::temp_dir().join(format!("moly-plugin-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        runtime.load(PluginId::new("test.plugin").unwrap(), &dir, "main.luau", &toml::Table::new())
    }

    #[test]
    fn a_task_acts_waits_and_resumes_across_frames() {
        let script = r#"
            moly.spawn(function()
                local done = moly.act({ ["do"] = "walk" })
                moly.log("acted", done.ticket)
                local refused = moly.act({ ["do"] = "refused" })
                moly.log("refused", refused.ok, refused.reason)
                moly.wait(0.5)
                moly.log("waited", moly.now() >= 0.5)
            end)
        "#;
        let mut runtime = Runtime::new(Limits::default());
        load(&mut runtime, "task", &[("main.luau", script)]).unwrap();
        let mut host = Recorder { act_frames: 1, ..Default::default() };
        // Frame 1 starts the step; frame 2 sees it finished, is refused the
        // second at once and starts waiting.
        for _ in 0..2 {
            assert!(runtime.tick(&mut host, 0.25, &[]).is_empty());
        }
        assert_eq!(host.lines, ["acted 1", "refused false no such step"]);
        assert_eq!(host.acts, [json!({ "do": "walk" })]);
        // 0.5 s at 0.25 s a frame: still waiting after one more frame,
        // resumed after the next.
        runtime.tick(&mut host, 0.25, &[]);
        assert_eq!(host.lines.len(), 2);
        runtime.tick(&mut host, 0.25, &[]);
        assert_eq!(host.lines.last().unwrap(), "waited true");
    }

    #[test]
    fn handlers_get_the_tick_and_their_event_kind() {
        let script = r#"
            moly.on("tick", function(dt) moly.log("tick", dt) end)
            moly.on("talk_started", function(e) moly.log("talk", e.unit) end)
            moly.on("*", function(e) moly.log("any", e.kind) end)
            moly.on("tick", function() moly.store.set("seen", (moly.store.get("seen") or 0) + 1) end)
        "#;
        let mut runtime = Runtime::new(Limits::default());
        load(&mut runtime, "events", &[("main.luau", script)]).unwrap();
        let mut host = Recorder::default();
        let events = [json!({ "kind": "talk_started", "unit": 5 }), json!({ "kind": "weather_changed" })];
        runtime.tick(&mut host, 0.5, &events);
        assert_eq!(host.lines, ["tick 0.5", "talk 5", "any talk_started", "any weather_changed"]);
        runtime.tick(&mut host, 0.5, &[]);
        assert_eq!(host.store.get("seen").and_then(serde_json::Value::as_f64), Some(2.0));
    }

    #[test]
    fn the_sandbox_hides_the_system_libraries_and_freezes_globals() {
        let script = r#"
            moly.on("tick", function()
                moly.log(type(os), type(io), type(debug), type(package), type(loadstring))
                moly.log(pcall(function() string.upper = nil end))
                moly.log(pcall(function() moly.id = "other" end))
            end)
        "#;
        let mut runtime = Runtime::new(Limits::default());
        load(&mut runtime, "sandbox", &[("main.luau", script)]).unwrap();
        let mut host = Recorder::default();
        runtime.tick(&mut host, 0.0, &[]);
        assert_eq!(host.lines[0], "nil nil nil nil nil");
        assert!(host.lines[1].starts_with("false"), "{}", host.lines[1]);
        assert!(host.lines[2].starts_with("false"), "{}", host.lines[2]);
    }

    #[test]
    fn a_looping_handler_is_stopped_after_its_strikes() {
        let script = r#"moly.on("tick", function() while true do end end)"#;
        let mut runtime = Runtime::new(Limits { instructions: 10_000, ..Limits::default() });
        load(&mut runtime, "loop", &[("main.luau", script)]).unwrap();
        let mut host = Recorder::default();
        assert!(runtime.tick(&mut host, 0.0, &[]).is_empty());
        assert!(runtime.tick(&mut host, 0.0, &[]).is_empty());
        let stopped = runtime.tick(&mut host, 0.0, &[]);
        assert_eq!(stopped.len(), 1);
        assert!(stopped[0].reason.contains("instruction budget"), "{}", stopped[0].reason);
        assert_eq!(runtime.running().count(), 0);
        assert_eq!(host.lines.len(), 3);
    }

    #[test]
    fn a_looping_task_yields_each_frame_and_keeps_running() {
        let script = r#"
            moly.spawn(function()
                local n = 0
                while true do n = n + 1 end
            end)
            moly.on("tick", function() moly.log("alive") end)
        "#;
        let mut runtime = Runtime::new(Limits { instructions: 10_000, ..Limits::default() });
        load(&mut runtime, "task-loop", &[("main.luau", script)]).unwrap();
        let mut host = Recorder::default();
        for _ in 0..5 {
            assert!(runtime.tick(&mut host, 0.0, &[]).is_empty());
        }
        assert_eq!(runtime.running().count(), 1);
        assert_eq!(host.lines, ["alive"; 5]);
    }

    #[test]
    fn require_stays_inside_the_plugin_directory() {
        let inside = r#"local util = require("lib/util") moly.on("tick", function() moly.log(util.name) end)"#;
        let mut runtime = Runtime::new(Limits::default());
        let files = [("main.luau", inside), ("lib/util.luau", r#"return { name = "util" }"#)];
        load(&mut runtime, "require", &files).unwrap();
        let mut host = Recorder::default();
        runtime.tick(&mut host, 0.0, &[]);
        assert_eq!(host.lines, ["util"]);

        let outside = r#"require("../escape")"#;
        let error = load(&mut runtime, "require-out", &[("main.luau", outside)]).unwrap_err();
        assert!(error.to_string().contains("inside the plugin directory"), "{error}");
    }

    #[test]
    fn the_interface_waits_for_the_first_frame() {
        let mut runtime = Runtime::new(Limits::default());
        let error = load(&mut runtime, "early", &[("main.luau", r#"moly.log("too early")"#)]).unwrap_err();
        assert!(error.to_string().contains("first frame"), "{error}");
    }
}
