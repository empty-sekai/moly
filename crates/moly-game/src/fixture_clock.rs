//! `Sekai.Mysekai.ClockGimmick`: fixture clock hands follow the device clock.
//!
//! Source behaviour, per MonoBehaviour instance:
//!
//! - The constructor sets `triggerMinutes` to an empty list and writes -1 to
//!   both `lastTriggeredHour` and `lastTriggeredMinute` (one eight-byte store
//!   covers the two adjacent ints).
//! - `Awake` caches `GetComponent<Animator>()` and calls `SetupClockTime`;
//!   `Update` calls `SetupClockTime` and then `CheckAndTriggerAnimation`. Each
//!   of the two reads `DateTime.Now` on its own, so a frame reads the device
//!   clock twice.
//! - `SetupClockTime` (both hand objects non-null): `hourMesh.localEulerAngles =
//!   (0, 0, -((hour % 12) * 30 + (minute / 60) * 30))` and
//!   `minuteMesh.localEulerAngles = (0, 0, minute * -6)`, in single precision
//!   with separate multiply and add (no fused step). The write replaces the
//!   whole local rotation, so an authored tilt on a hand is discarded.
//! - `CheckAndTriggerAnimation`: with an Animator, a non-empty
//!   `actionTriggerName` and a non-empty `triggerMinutes`, when the current
//!   minute is listed and (hour, minute) differs from the last trigger, it calls
//!   `Animator.SetTrigger(actionTriggerName)` and records (hour, minute).
//!
//! `DateTime.Now` is the device's local clock, not the server refresh period.
//! The serialized fields come from the fixture controller export (the same
//! per-package documents the gimmick catalogue streams); a package absent from
//! that export has unknown clock state, which is reported, never assumed.
//! The one clock with a trigger drives a general Animator controller (an idle
//! state and a triggered state with fixed-duration transitions); that controller
//! family is not ported, so its trigger is reported loudly when it fires.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use bevy::{asset::LoadState, prelude::*};
use moly_assets::{json::JsonAsset, scene_state::SourceInactive, source_navigation::SourceObjectIdentity};
use serde_json::Value;

use crate::fixture::{FixtureVisualRoot, FixtureVisualSceneReady};
use crate::fixture_colors::FixtureColorChoice;

const INDEX: &str = "moly://fixture-gimmick/browser-index.json";
/// `Mathf.Deg2Rad` as libunity's Euler conversion uses it.
const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SourceObject {
    file: String,
    game_object: i64,
}

#[derive(Clone, Debug)]
struct Trigger {
    name: String,
    minutes: Vec<i32>,
}

/// One serialized ClockGimmick component.
#[derive(Clone, Debug)]
struct ClockDefinition {
    owner: SourceObject,
    /// `m_Enabled`: a disabled behaviour still gets `Awake`, never `Update`.
    enabled: bool,
    hour: SourceObject,
    minute: SourceObject,
    /// Present only when the Animator, the trigger name and the minute list
    /// all satisfy `CheckAndTriggerAnimation`'s entry test.
    trigger: Option<Trigger>,
}

#[derive(Resource)]
pub(crate) struct ClockSupply {
    index: Handle<JsonAsset>,
    paths: Option<Result<HashMap<String, String>, String>>,
    pending: HashMap<String, Handle<JsonAsset>>,
    known: HashMap<String, Arc<Result<Vec<ClockDefinition>, String>>>,
    unsupplied: BTreeSet<String>,
    reported_unsupplied: usize,
}

/// A bound ClockGimmick instance on an actual fixture scene.
struct BoundClock {
    package: String,
    owner: Entity,
    hour: Entity,
    minute: Entity,
    definition: ClockDefinition,
    last_triggered: (i32, i32),
    awake_done: bool,
}

#[derive(Component)]
pub(crate) struct FixtureClocks(Vec<BoundClock>);

/// The visual root's clock binding is decided (bound, none, or refused).
#[derive(Component)]
pub(crate) struct FixtureClocksResolved;

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ClockSupply {
        index: server.load(INDEX),
        paths: None,
        pending: HashMap::new(),
        known: HashMap::new(),
        unsupplied: BTreeSet::new(),
        reported_unsupplied: 0,
    });
}

fn parse_index(doc: &Value) -> Result<HashMap<String, String>, String> {
    if doc["schemaVersion"].as_u64() != Some(1) {
        return Err("fixture controller index schema is not 1".into());
    }
    let mut paths = HashMap::new();
    for row in doc["packages"].as_array().ok_or("fixture controller index packages missing")? {
        let name = row["name"].as_str().ok_or("fixture controller package name missing")?;
        let path = row["path"].as_str().ok_or("fixture controller package path missing")?;
        let file = path
            .strip_prefix("fixture-gimmick/by-package/")
            .filter(|file| file.ends_with(".json"))
            .ok_or("fixture controller package path is outside its namespace")?;
        if file.is_empty()
            || file.starts_with('.')
            || !file.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err("fixture controller package path is outside its namespace".into());
        }
        if paths.insert(name.to_owned(), path.to_owned()).is_some() {
            return Err(format!("duplicate fixture controller package {name}"));
        }
    }
    Ok(paths)
}

fn pointer_id(value: &Value, label: &str) -> Result<i64, String> {
    if value["m_FileID"].as_i64() != Some(0) {
        return Err(format!("{label} points into another serialized file"));
    }
    match &value["m_PathID"] {
        Value::String(text) => text.parse().map_err(|_| format!("{label} path id is not i64")),
        Value::Number(number) => number.as_i64().ok_or_else(|| format!("{label} path id is not i64")),
        _ => Err(format!("{label} has no path id")),
    }
}

fn source_id(value: &Value, label: &str) -> Result<SourceObject, String> {
    let file = value["file"].as_str().ok_or_else(|| format!("{label} has no file"))?;
    let id = value["pathId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| format!("{label} has no i64 path id"))?;
    Ok(SourceObject { file: file.to_owned(), game_object: id })
}

/// Every ClockGimmick component of one controller-export package. The
/// components live on the Animator hierarchies the export lists.
fn definitions(package: &Value) -> Result<Vec<ClockDefinition>, String> {
    let animators = package["animators"].as_array().ok_or("package has no animator list")?;
    let mut out = Vec::new();
    for animator in animators {
        let nodes = animator["nodes"].as_array().ok_or("animator has no node list")?;
        let mut objects = HashMap::new();
        for node in nodes {
            let object = source_id(&node["gameObject"], "node GameObject")?;
            objects.insert(object.game_object, object);
        }
        for node in nodes {
            let components = node["components"].as_array().ok_or("node has no component list")?;
            for component in components {
                if component["source"]["script"].as_str() != Some("ClockGimmick") {
                    continue;
                }
                let owner = source_id(&node["gameObject"], "ClockGimmick GameObject")?;
                let fields = &component["fields"];
                let enabled = match fields["m_Enabled"].as_u64() {
                    Some(0) => false,
                    Some(1) => true,
                    _ => return Err("ClockGimmick m_Enabled is not 0 or 1".into()),
                };
                let hand = |key: &str| -> Result<SourceObject, String> {
                    let id = pointer_id(&fields[key], key)?;
                    // IsNull on either hand skips SetupClockTime entirely; a
                    // null pointer here is a clock that never moves, which the
                    // export does not contain. Refuse rather than guess.
                    if id == 0 {
                        return Err(format!("ClockGimmick {key} is null"));
                    }
                    let object = objects
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| format!("ClockGimmick {key} is outside the exported hierarchy"))?;
                    if object.file != owner.file {
                        return Err(format!("ClockGimmick {key} crosses serialized files"));
                    }
                    Ok(object)
                };
                let hour = hand("hourMesh")?;
                let minute = hand("minuteMesh")?;
                let name = fields["actionTriggerName"].as_str().ok_or("ClockGimmick actionTriggerName missing")?;
                let minutes = fields["triggerMinutes"]
                    .as_array()
                    .ok_or("ClockGimmick triggerMinutes missing")?
                    .iter()
                    .map(|m| m.as_i64().and_then(|m| i32::try_from(m).ok()).ok_or("ClockGimmick trigger minute is not an int"))
                    .collect::<Result<Vec<_>, _>>()?;
                // GetComponent<Animator>() on the behaviour's own GameObject.
                let has_animator = components.iter().any(|c| c["source"]["class"].as_str() == Some("Animator"));
                let trigger = (has_animator && !name.is_empty() && !minutes.is_empty())
                    .then(|| Trigger { name: name.to_owned(), minutes });
                out.push(ClockDefinition { owner, enabled, hour, minute, trigger });
            }
        }
    }
    Ok(out)
}

/// Load and parse the controller export for every fixture package that has a
/// visual instance. At most one package document is parsed per update.
pub(crate) fn supply(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    supply: Option<ResMut<ClockSupply>>,
    roots: Query<&FixtureColorChoice, (With<FixtureVisualRoot>, Without<FixtureClocksResolved>)>,
) {
    let Some(mut supply) = supply else { return; };
    if roots.is_empty() {
        return;
    }
    if supply.paths.is_none() {
        if let LoadState::Failed(error) = server.load_state(&supply.index) {
            warn!("[fixture-clock] controller export index unavailable, clock state unknown for every fixture: {error}");
            supply.paths = Some(Err(error.to_string()));
        } else if let Some(asset) = jsons.get(&supply.index) {
            let parsed = serde_json::from_str::<Value>(&asset.0)
                .map_err(|e| e.to_string())
                .and_then(|doc| parse_index(&doc));
            if let Err(error) = &parsed {
                warn!("[fixture-clock] controller export index rejected: {error}");
            }
            supply.paths = Some(parsed);
        } else {
            return;
        }
    }
    let wanted: BTreeSet<String> = roots.iter().map(|choice| choice.package.clone()).collect();
    for package in &wanted {
        if supply.known.contains_key(package) || supply.pending.contains_key(package) {
            continue;
        }
        let path = match supply.paths.as_ref().expect("index decided") {
            Ok(paths) => paths.get(package).cloned(),
            Err(error) => {
                let error = error.clone();
                supply.known.insert(package.clone(), Arc::new(Err(error)));
                continue;
            }
        };
        match path {
            Some(path) => {
                let handle = server.load(format!("moly://{path}"));
                supply.pending.insert(package.clone(), handle);
            }
            None => {
                supply.unsupplied.insert(package.clone());
                supply.known.insert(package.clone(), Arc::new(Ok(Vec::new())));
            }
        }
    }
    if supply.unsupplied.len() != supply.reported_unsupplied {
        supply.reported_unsupplied = supply.unsupplied.len();
        warn!(
            "[fixture-clock] {} placed fixture packages are absent from the controller export; their ClockGimmick state is unknown and they keep their authored pose",
            supply.unsupplied.len()
        );
    }
    let mut names: Vec<_> = supply.pending.keys().cloned().collect();
    names.sort();
    for name in names {
        let handle = supply.pending[&name].clone();
        let result = if let LoadState::Failed(error) = server.load_state(&handle) {
            Err(format!("controller export package load failed: {error}"))
        } else {
            let Some(asset) = jsons.get(&handle) else { continue; };
            serde_json::from_str::<Value>(&asset.0)
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    if doc["package"]["name"].as_str() != Some(name.as_str()) {
                        return Err("controller export package identity mismatch".into());
                    }
                    definitions(&doc["package"])
                })
        };
        match &result {
            Err(error) => warn!("[fixture-clock] {name}: {error}"),
            Ok(clocks) if !clocks.is_empty() => info!("[fixture-clock] {name}: {} ClockGimmick component(s)", clocks.len()),
            Ok(_) => {}
        }
        supply.known.insert(name.clone(), Arc::new(result));
        supply.pending.remove(&name);
        break;
    }
}

fn find(
    root: Entity,
    target: &SourceObject,
    children: &Query<&Children>,
    identities: &Query<&SourceObjectIdentity>,
) -> Result<Entity, String> {
    let mut stack = vec![root];
    let mut found = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
        if identities
            .get(entity)
            .is_ok_and(|id| id.file == target.file && id.game_object == target.game_object)
        {
            found.push(entity);
        }
    }
    match found.as_slice() {
        [entity] => Ok(*entity),
        other => Err(format!("{} scene nodes carry source GameObject {}", other.len(), target.game_object)),
    }
}

/// Bind each decided package's clocks to the actual scene nodes by source
/// object identity (no node-name fallback).
pub(crate) fn bind(
    mut commands: Commands,
    supply: Option<Res<ClockSupply>>,
    roots: Query<(Entity, &FixtureColorChoice), (With<FixtureVisualRoot>, With<FixtureVisualSceneReady>, Without<FixtureClocksResolved>)>,
    children: Query<&Children>,
    identities: Query<&SourceObjectIdentity>,
) {
    let Some(supply) = supply else { return; };
    for (root, choice) in &roots {
        let Some(known) = supply.known.get(&choice.package) else { continue; };
        let definitions = match known.as_ref() {
            Ok(definitions) => definitions,
            Err(_) => {
                commands.entity(root).insert(FixtureClocksResolved);
                continue;
            }
        };
        let mut bound = Vec::new();
        for definition in definitions {
            let result = (|| {
                Ok::<_, String>(BoundClock {
                    package: choice.package.clone(),
                    owner: find(root, &definition.owner, &children, &identities)?,
                    hour: find(root, &definition.hour, &children, &identities)?,
                    minute: find(root, &definition.minute, &children, &identities)?,
                    definition: definition.clone(),
                    last_triggered: (-1, -1),
                    awake_done: false,
                })
            })();
            match result {
                Ok(clock) => bound.push(clock),
                Err(error) => warn!("[fixture-clock] {}: ClockGimmick not bound: {error}", choice.package),
            }
        }
        let mut entity = commands.entity(root);
        entity.insert(FixtureClocksResolved);
        if !bound.is_empty() {
            info!("[fixture-clock] {}: {} clock(s) bound to the device clock", choice.package, bound.len());
            entity.insert(FixtureClocks(bound));
        }
    }
}

/// `SetupClockTime`'s two hand angles in degrees for a local (hour, minute),
/// in the source's single-precision operation order.
pub(crate) fn hand_angles(hour: i32, minute: i32) -> (f32, f32) {
    let minute_f = minute as f32;
    let hour_z = -(((hour % 12) as f32) * 30.0 + (minute_f / 60.0) * 30.0);
    let minute_z = minute_f * -6.0;
    (hour_z, minute_z)
}

/// `localEulerAngles = (0, 0, z)` is `Quaternion.Euler(0, 0, z)`, a pure
/// rotation about the local Z axis, mapped once into the exported basis.
fn hand_rotation(z_degrees: f32) -> Quat {
    moly_assets::coordinates::source_rotation(Quat::from_rotation_z(z_degrees * DEG_TO_RAD))
}

fn set_hand(transforms: &mut Query<&mut Transform>, entity: Entity, z: f32) {
    if let Ok(mut transform) = transforms.get_mut(entity) {
        let rotation = hand_rotation(z);
        if transform.rotation != rotation {
            transform.rotation = rotation;
        }
    }
}

fn setup_clock_time(clock: &BoundClock, transforms: &mut Query<&mut Transform>) {
    let (hour, minute) = local_hour_minute();
    let (hour_z, minute_z) = hand_angles(hour, minute);
    set_hand(transforms, clock.hour, hour_z);
    set_hand(transforms, clock.minute, minute_z);
}

pub(crate) fn advance(
    mut clocks: Query<&mut FixtureClocks>,
    inactive: Query<(), With<SourceInactive>>,
    mut transforms: Query<&mut Transform>,
) {
    for mut clocks in &mut clocks {
        for clock in &mut clocks.0 {
            // Neither Awake nor Update runs while the GameObject is inactive
            // in the hierarchy.
            if inactive.contains(clock.owner) {
                continue;
            }
            if !clock.awake_done {
                clock.awake_done = true;
                setup_clock_time(clock, &mut transforms);
                if !clock.definition.enabled {
                    continue;
                }
            }
            if !clock.definition.enabled {
                continue;
            }
            setup_clock_time(clock, &mut transforms);
            if let Some(trigger) = &clock.definition.trigger {
                let (hour, minute) = local_hour_minute();
                if trigger.minutes.contains(&minute) && clock.last_triggered != (hour, minute) {
                    warn!(
                        "[fixture-clock] {}: ClockGimmick fires Animator trigger {} at {hour:02}:{minute:02}; this fixture's general Animator controller is not ported, so the triggered motion does not play",
                        clock.package, trigger.name
                    );
                    clock.last_triggered = (hour, minute);
                }
            }
        }
    }
}

/// The device's local (hour, minute), `DateTime.Now.Hour` / `.Minute`.
/// Native builds accept `MOLY_FIXTURE_CLOCK_NOW=HH:MM` to pin it for a
/// verification run; a malformed value is refused loudly.
fn local_hour_minute() -> (i32, i32) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(pinned) = std::env::var("MOLY_FIXTURE_CLOCK_NOW") {
        let parsed = pinned.split_once(':').and_then(|(h, m)| {
            let (h, m) = (h.trim().parse::<i32>().ok()?, m.trim().parse::<i32>().ok()?);
            ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
        });
        return parsed.unwrap_or_else(|| panic!("MOLY_FIXTURE_CLOCK_NOW is not HH:MM: {pinned:?}"));
    }
    device_local_hour_minute()
}

#[cfg(target_arch = "wasm32")]
fn device_local_hour_minute() -> (i32, i32) {
    let now = js_sys::Date::new_0();
    (now.get_hours() as i32, now.get_minutes() as i32)
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn device_local_hour_minute() -> (i32, i32) {
    // SAFETY: time(NULL) and localtime_r with a zeroed out-parameter owned here.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        assert!(!libc::localtime_r(&now, &mut local).is_null(), "localtime_r failed");
        (local.tm_hour, local.tm_min)
    }
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn device_local_hour_minute() -> (i32, i32) {
    // SAFETY: time(NULL) and localtime_s with a zeroed out-parameter owned here.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        assert_eq!(libc::localtime_s(&mut local, &now), 0, "localtime_s failed");
        (local.tm_hour, local.tm_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Research instrument: `MOLY_CLOCK_SOURCE_TABLE` names a table of the two
    /// hand angles produced by executing the source's own SetupClockTime
    /// arithmetic (JP 6.8.1 native code, run instruction by instruction) for
    /// every local (hour, minute). The port must reproduce every bit pattern.
    #[test]
    #[ignore = "requires a caller-supplied table executed from the source binary"]
    fn hand_angles_equal_the_source_binary() {
        let path = std::env::var_os("MOLY_CLOCK_SOURCE_TABLE").expect("MOLY_CLOCK_SOURCE_TABLE");
        let table: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let rows = table["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 24 * 60, "table must cover every local hour and minute");
        let mut mismatches = Vec::new();
        for row in rows {
            let hour = row["hour"].as_i64().unwrap() as i32;
            let minute = row["minute"].as_i64().unwrap() as i32;
            let want = (row["hourZ"].as_u64().unwrap() as u32, row["minuteZ"].as_u64().unwrap() as u32);
            let (h, m) = hand_angles(hour, minute);
            if (h.to_bits(), m.to_bits()) != want {
                mismatches.push((hour, minute, h, m, want));
            }
        }
        println!("clock hand angles compared: {} rows, {} mismatches", rows.len(), mismatches.len());
        assert!(mismatches.is_empty(), "{mismatches:?}");
    }
}
