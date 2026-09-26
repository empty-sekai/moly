//! The harvest views' particle systems: `Play()` and `Stop()` on their
//! serialized ParticleSystem fields, through the fixture particle host's
//! played-object path. A field's system and its Transform subtree are
//! prepared from the package's source particle archive (the
//! `fixture-particles-v2` index lists it by package,
//! `mysekai__site__field__object__<leaf>`), then played (the first Play
//! resets the seeds and installs the birth owner) or stopped (`StopEmitting`:
//! the live particles finish).
//!
//! The calls, read off the views:
//! - Setup, for an object placed alive (a harvested one is hidden): a field
//!   whose system has `playOnAwake` plays at instantiation (the engine's
//!   Play, before the view's), then
//!   - tree: rare, `rareParticleSystem.Play()` (not null-checked);
//!     `_objectParticle.Play()` if set;
//!   - stone: `_objectParticle.Play()` if set; `RefreshParticle`: rare,
//!     `rareParticleSystem` active and `Play()`; otherwise inactive and
//!     `Stop()` (not null-checked);
//!   - plant (only with a MeshRenderer on `_plantBeforeObject`): rare,
//!     `rareParticleSystem.Play()` if set; `_objectParticle.Play()` if set;
//!   - birthday plant `_objectParticle.Play()` and driftage
//!     `_driftageStayEffect.Play()`, each if set; junk
//!     `_junkStayEffect.Play()` and toolbox `_junkNormalCutEffect.Play()`
//!     (not null-checked);
//! - `PlayDamageEffect`: treasure box `_treasureBoxNormalCutEffect.Play()`;
//!   tone `_toneFieldEffect.Stop()`;
//! - `OnPlayerActionStart`: birthday plant `_objectParticle.Stop()` if set;
//!   toolbox, after its 1.0 s delay, the box inactive and
//!   `_junkNormalCutEffect.Stop()`;
//! - `ChangeAfterObject`: tree, stone and plant `rareParticleSystem.Stop()`
//!   then `_objectParticle.Stop()`, each if set; birthday plant
//!   `_objectParticle.Stop()`; junk `_junkStayEffect.Stop()`; driftage after
//!   1.0 s `_driftageStayEffect.Stop()`; toolbox after 1.0 s
//!   `_junkNormalCutEffect.Stop()` if set; treasure box after 2.0 s
//!   `_treasureBoxNormalCutEffect.Stop()`; tone after the listen time
//!   `_toneFieldEffect.Stop()`.
//!
//! The calls of one object run in order: a Play still waiting for its
//! archive or GPU preparation holds the calls after it. A Stop of a field
//! never played stops nothing. A null field in a null-checked call is
//! skipped; in an unchecked one it is an error line (the source would throw
//! there). `SetActive` of the stone's rare system is its node's visibility.
//!
//! Named gaps: a package the index does not list draws none of its
//! particles (named once per package); a system with `playOnAwake` that no
//! view field reaches is not played; `ForceChangeAfterObject`'s stops are
//! not reached (the mock places no harvested object that keeps particles).
//! None of this is verified on an exported archive yet.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::source_navigation::{SourceHarvestView, SourceObjectIdentity};
use serde_json::Value;

use super::{HarvestDocs, HarvestObject, HarvestRoot};
use crate::fixture_timeline_particles::{
    play_object, prepare_played_object, stop_object, ParticlePlayBinding,
};
use crate::site_move::timeline::Delay;

const INDEX: &str = "moly://fixture-particles-v2/index.json";

/// `Play()` or `Stop()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticleOp {
    Play,
    Stop,
}

/// Whether the view checks the field for null before the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NullRef {
    Checked,
    Unchecked,
}

#[derive(Clone, Debug)]
struct Call {
    root: Entity,
    field: &'static str,
    op: ParticleOp,
    null: NullRef,
    /// `SetActive` of the field's GameObject before the call.
    active: Option<bool>,
    reason: &'static str,
}

/// A queued entry: one object's Setup (expanded once its prop document is
/// in) or one call.
#[derive(Clone, Debug)]
enum Entry {
    Setup(Entity),
    Call(Call),
}

impl Entry {
    fn root(&self) -> Entity {
        match self {
            Entry::Setup(root) => *root,
            Entry::Call(call) => call.root,
        }
    }
}

/// The views' particle calls, in order, and the delayed ones.
#[derive(Resource, Default)]
pub(crate) struct HarvestParticleCalls {
    pending: VecDeque<Entry>,
    delayed: Vec<(Delay, Call)>,
    index: Option<Handle<JsonAsset>>,
    /// Packages the index lists (read once it loads).
    listed: Option<HashSet<String>>,
    unlisted_named: HashSet<String>,
    /// Frames a call has waited on a retryable preparation.
    waited: u32,
}

impl HarvestParticleCalls {
    /// Whether the archive index lists `package` (`None` until it is read).
    pub(crate) fn listed(&self, package: &str) -> Option<bool> {
        self.listed.as_ref().map(|listed| listed.contains(package))
    }

    /// One call (see the module comment for the source's calls).
    pub(crate) fn queue(
        &mut self,
        root: Entity,
        field: &'static str,
        op: ParticleOp,
        null: NullRef,
        reason: &'static str,
    ) {
        self.pending.push_back(Entry::Call(Call {
            root,
            field,
            op,
            null,
            active: None,
            reason,
        }));
    }

    /// A call after `seconds` (a view's `UniTask.Delay`), counted from
    /// `frame` as the site timeline's delays are.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_after(
        &mut self,
        root: Entity,
        field: &'static str,
        op: ParticleOp,
        null: NullRef,
        reason: &'static str,
        seconds: f32,
        frame: u64,
    ) {
        let call = Call {
            root,
            field,
            op,
            null,
            active: None,
            reason,
        };
        self.delayed.push((Delay::new(seconds, frame), call));
    }

    /// The Setup calls of one object placed alive (expanded in order once
    /// its prop document is in).
    pub(crate) fn setup(&mut self, root: Entity) {
        self.pending.push_back(Entry::Setup(root));
    }
}

/// `ChangeAfterObject`'s particle calls of one object (`frame`: the call's).
pub(crate) fn change_after_object(
    calls: &mut HarvestParticleCalls,
    root: Entity,
    class: &str,
    listen_length: f32,
    frame: u64,
) {
    use NullRef::{Checked, Unchecked};
    use ParticleOp::Stop;
    const WHY: &str = "ChangeAfterObject";
    match class {
        "MysekaiAreaTreeView" | "MysekaiAreaStoneView" | "MysekaiAreaPlantView" => {
            calls.queue(root, "rareParticleSystem", Stop, Checked, WHY);
            calls.queue(root, "_objectParticle", Stop, Checked, WHY);
        }
        "MysekaiBirthdayPlantView" => calls.queue(root, "_objectParticle", Stop, Unchecked, WHY),
        "MysekaiAreaJunkView" => calls.queue(root, "_junkStayEffect", Stop, Unchecked, WHY),
        "MysekaiAreadDriftageView" => calls.queue_after(
            root,
            "_driftageStayEffect",
            Stop,
            Unchecked,
            WHY,
            1.0,
            frame,
        ),
        "MysekaiAreaToolBoxView" => {
            calls.queue_after(root, "_junkNormalCutEffect", Stop, Checked, WHY, 1.0, frame)
        }
        "MysekaiAreaTreasureBoxView" => calls.queue_after(
            root,
            "_treasureBoxNormalCutEffect",
            Stop,
            Unchecked,
            WHY,
            2.0,
            frame,
        ),
        "MysekaiAreaToneView" => calls.queue_after(
            root,
            "_toneFieldEffect",
            Stop,
            Unchecked,
            WHY,
            listen_length,
            frame,
        ),
        _ => {}
    }
}

/// The prepared field subtrees of one object, by field node.
#[derive(Component, Default)]
pub(crate) struct HarvestParticleBindings(HashMap<Entity, ParticlePlayBinding>);

/// The Setup calls of one object; `None` while its prop document loads.
fn expand_setup(world: &World, root: Entity) -> Option<Vec<Call>> {
    let Some(object) = world.get::<HarvestObject>(root) else {
        return Some(Vec::new());
    };
    let (class, rare, package) = (object.class, object.is_rare, object.package.clone());
    let doc = prop_document(world, &package)?;
    let mut out = Vec::new();
    let mut push = |field, op, null, active, reason| {
        out.push(Call {
            root,
            field,
            op,
            null,
            active,
            reason,
        })
    };
    // The engine's Play at instantiation, for every field of the class whose
    // system has playOnAwake.
    for field in fields_of(class) {
        if play_on_awake(world, root, &doc, field) {
            push(
                *field,
                ParticleOp::Play,
                NullRef::Checked,
                None,
                "playOnAwake",
            );
        }
    }
    use NullRef::{Checked, Unchecked};
    use ParticleOp::{Play, Stop};
    let calls: Vec<(&'static str, ParticleOp, NullRef, Option<bool>)> = match class {
        "MysekaiAreaTreeView" => {
            let mut calls = Vec::new();
            if rare {
                calls.push(("rareParticleSystem", Play, Unchecked, None));
            }
            calls.push(("_objectParticle", Play, Checked, None));
            calls
        }
        "MysekaiAreaStoneView" => vec![
            ("_objectParticle", Play, Checked, None),
            if rare {
                ("rareParticleSystem", Play, Unchecked, Some(true))
            } else {
                ("rareParticleSystem", Stop, Unchecked, Some(false))
            },
        ],
        "MysekaiAreaPlantView" => {
            if !plant_has_renderer(world, root) {
                info!("[harvest-particle] {package}: plant Setup returns early (no MeshRenderer on _plantBeforeObject): no particle calls");
                return Some(out);
            }
            let mut calls = Vec::new();
            if rare {
                calls.push(("rareParticleSystem", Play, Checked, None));
            }
            calls.push(("_objectParticle", Play, Checked, None));
            calls
        }
        "MysekaiBirthdayPlantView" => vec![("_objectParticle", Play, Checked, None)],
        "MysekaiAreadDriftageView" => vec![("_driftageStayEffect", Play, Checked, None)],
        "MysekaiAreaJunkView" => vec![("_junkStayEffect", Play, Unchecked, None)],
        "MysekaiAreaToolBoxView" => vec![("_junkNormalCutEffect", Play, Unchecked, None)],
        _ => Vec::new(),
    };
    for (field, op, null, active) in calls {
        push(field, op, null, active, "Setup");
    }
    Some(out)
}

/// The prop document of `package`, parsed (`None` while it loads).
fn prop_document(world: &World, package: &str) -> Option<Value> {
    let handle = world.get_resource::<HarvestDocs>()?.0.get(package)?.clone();
    let doc = world.resource::<Assets<JsonAsset>>().get(&handle)?;
    Some(serde_json::from_str(&doc.0).unwrap_or(Value::Null))
}

/// The ParticleSystem fields of each view class.
fn fields_of(class: &str) -> &'static [&'static str] {
    match class {
        "MysekaiAreaTreeView" | "MysekaiAreaStoneView" | "MysekaiAreaPlantView" => {
            &["_objectParticle", "rareParticleSystem"]
        }
        "MysekaiBirthdayPlantView" => &["_objectParticle"],
        "MysekaiAreadDriftageView" => &["_driftageStayEffect"],
        "MysekaiAreaJunkView" => &["_junkStayEffect"],
        "MysekaiAreaToolBoxView" => &["_junkNormalCutEffect"],
        "MysekaiAreaTreasureBoxView" => &["_treasureBoxNormalCutEffect"],
        "MysekaiAreaToneView" => &["_toneFieldEffect"],
        _ => &[],
    }
}

/// The view contract under `root` and its fields.
fn view_fields(world: &World, root: Entity) -> Option<Value> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Some(view) = world.get::<SourceHarvestView>(entity) {
            return serde_json::from_str(&view.fields_json).ok();
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    None
}

/// A field's component id: `None` for a null reference; `Err` for a
/// reference into another file.
fn reference(fields: &Value, field: &str) -> Result<Option<i64>, String> {
    let Some(reference) = fields.get(field) else {
        return Err(format!("the view has no field {field}"));
    };
    if reference.is_null()
        || reference["id"].as_str() == Some("0")
        || reference["id"].as_i64() == Some(0)
    {
        return Ok(None);
    }
    if reference["file"].as_i64() != Some(0) {
        return Err(format!("{field} is not a component of the prefab itself"));
    }
    reference["id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .or_else(|| reference["id"].as_i64())
        .map(Some)
        .ok_or_else(|| format!("{field} names no component: {reference}"))
}

/// The scene node whose GameObject carries component `id`.
fn node_of(world: &World, root: Entity, id: i64) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world
            .get::<SourceObjectIdentity>(entity)
            .is_some_and(|identity| identity.components.contains(&id))
        {
            return Some(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    None
}

/// Whether the field's system has `playOnAwake` in the prop document.
fn play_on_awake(world: &World, root: Entity, doc: &Value, field: &str) -> bool {
    let Some(fields) = view_fields(world, root) else {
        return false;
    };
    let Ok(Some(id)) = reference(&fields, field) else {
        return false;
    };
    doc["particles"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| {
            row["pathId"].as_i64() == Some(id)
                && row["system"]["playOnAwake"].as_bool() == Some(true)
        })
}

/// Whether `_plantBeforeObject` carries a MeshRenderer: its node has a mesh
/// primitive child.
fn plant_has_renderer(world: &World, root: Entity) -> bool {
    let Some(fields) = view_fields(world, root) else {
        return false;
    };
    let Ok(Some(game_object)) = reference(&fields, "_plantBeforeObject") else {
        return false;
    };
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world
            .get::<SourceObjectIdentity>(entity)
            .is_some_and(|identity| identity.game_object == game_object)
        {
            return world.get::<Mesh3d>(entity).is_some()
                || world.get::<Children>(entity).is_some_and(|children| {
                    children.iter().any(|child| {
                        world.get::<Mesh3d>(child).is_some()
                            && world.get::<SourceObjectIdentity>(child).is_none()
                    })
                });
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    false
}

/// Whether the archive index lists `package` (`None` while it loads).
pub(crate) fn archive_listed(world: &mut World, package: &str) -> Option<bool> {
    let handle = {
        let server = world.resource::<AssetServer>().clone();
        let mut calls = world.resource_mut::<HarvestParticleCalls>();
        calls
            .index
            .get_or_insert_with(|| server.load(INDEX))
            .clone()
    };
    if world.resource::<HarvestParticleCalls>().listed.is_none() {
        let listed = match world.resource::<AssetServer>().load_state(&handle) {
            LoadState::Failed(error) => {
                warn!("[harvest-particle] {INDEX}: {error}; no harvest particles are drawn");
                HashSet::new()
            }
            _ => {
                let json = world.resource::<Assets<JsonAsset>>().get(&handle)?;
                let value: Value = serde_json::from_str(&json.0).unwrap_or(Value::Null);
                value["packages"]
                    .as_object()
                    .map(|packages| packages.keys().cloned().collect())
                    .unwrap_or_default()
            }
        };
        world.resource_mut::<HarvestParticleCalls>().listed = Some(listed);
    }
    let calls = world.resource::<HarvestParticleCalls>();
    Some(calls.listed.as_ref().expect("read above").contains(package))
}

/// Exclusive, Update: the due delayed calls join the queue; then the queue
/// runs in order until a call waits.
pub(crate) fn advance(world: &mut World) {
    // The index is read once (the stay host waits on the answer).
    if world.resource::<HarvestParticleCalls>().listed.is_none() {
        let _ = archive_listed(world, "");
    }
    let frame = u64::from(world.resource::<bevy::diagnostic::FrameCount>().0);
    let dt = world.resource::<Time>().delta_secs();
    {
        let mut calls = world.resource_mut::<HarvestParticleCalls>();
        let mut due = Vec::new();
        calls.delayed.retain_mut(|(delay, entry)| {
            if delay.tick(frame, dt) {
                due.push(entry.clone());
                false
            } else {
                true
            }
        });
        calls.pending.extend(due.into_iter().map(Entry::Call));
    }
    let queue = std::mem::take(&mut world.resource_mut::<HarvestParticleCalls>().pending);
    if queue.is_empty() {
        return;
    }
    let mut held: HashSet<Entity> = HashSet::new();
    let mut kept = VecDeque::new();
    for entry in queue {
        let root = entry.root();
        if held.contains(&root) {
            kept.push_back(entry);
            continue;
        }
        let calls = match entry {
            Entry::Setup(root) => match expand_setup(world, root) {
                Some(calls) => calls,
                None => {
                    held.insert(root);
                    kept.push_back(Entry::Setup(root));
                    continue;
                }
            },
            Entry::Call(call) => vec![call],
        };
        for call in calls {
            if held.contains(&root) {
                kept.push_back(Entry::Call(call));
                continue;
            }
            if let Step::Wait = run(world, &call) {
                held.insert(root);
                kept.push_back(Entry::Call(call));
            }
        }
    }
    let mut calls = world.resource_mut::<HarvestParticleCalls>();
    if kept.is_empty() {
        calls.waited = 0;
    } else {
        calls.waited += 1;
        if calls.waited == 600 {
            warn!(
                "[harvest-particle] {} calls still wait on their archive or GPU preparation after 600 frames",
                kept.len()
            );
        }
    }
    kept.extend(std::mem::take(&mut calls.pending));
    calls.pending = kept;
}

enum Step {
    Done,
    Wait,
}

fn run(world: &mut World, entry: &Call) -> Step {
    let Some(object) = world.get::<HarvestObject>(entry.root) else {
        return Step::Done; // the object went with the site
    };
    if world.get::<HarvestRoot>(entry.root).is_none() {
        return Step::Done;
    }
    let (package, leaf, id) = (
        object.package.clone(),
        object.leaf.clone(),
        object.fixture_id,
    );
    let what = format!(
        "{leaf}#{id} {}.{:?} ({})",
        entry.field, entry.op, entry.reason
    );
    let Some(fields) = view_fields(world, entry.root) else {
        error!("[harvest-particle] {what}: the object has no view contract");
        return Step::Done;
    };
    let node = match reference(&fields, entry.field) {
        Err(reason) => {
            error!("[harvest-particle] {what}: {reason}");
            return Step::Done;
        }
        Ok(None) => {
            if entry.null == NullRef::Unchecked {
                error!("[harvest-particle] {what}: the field is null and the view does not check it (the source throws here)");
            }
            return Step::Done;
        }
        Ok(Some(component)) => match node_of(world, entry.root, component) {
            Some(node) => node,
            None => {
                error!("[harvest-particle] {what}: no scene node carries component {component}");
                return Step::Done;
            }
        },
    };
    if let Some(active) = entry.active {
        if let Some(mut visibility) = world.get_mut::<Visibility>(node) {
            *visibility = if active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    let bound = world
        .get::<HarvestParticleBindings>(entry.root)
        .and_then(|bindings| bindings.0.get(&node).cloned());
    match entry.op {
        ParticleOp::Stop => {
            let Some(binding) = bound else {
                return Step::Done; // never played: nothing to stop
            };
            let stopped = stop_object(world, &binding);
            info!("[harvest-particle] {what}: {stopped} systems stop emitting");
            Step::Done
        }
        ParticleOp::Play => {
            let binding = match bound {
                Some(binding) => binding,
                None => {
                    match archive_listed(world, &package) {
                        None => return Step::Wait,
                        Some(false) => {
                            let mut calls = world.resource_mut::<HarvestParticleCalls>();
                            if calls.unlisted_named.insert(package.clone()) {
                                warn!("[harvest-particle] {package}: the particle archive index does not list it; its particles are not drawn");
                            }
                            return Step::Done;
                        }
                        Some(true) => {}
                    }
                    match prepare_played_object(world, entry.root, node, &package) {
                        Ok(binding) => {
                            let mut root = world.entity_mut(entry.root);
                            if root.get::<HarvestParticleBindings>().is_none() {
                                root.insert(HarvestParticleBindings::default());
                            }
                            root.get_mut::<HarvestParticleBindings>()
                                .expect("inserted above")
                                .0
                                .insert(node, binding.clone());
                            binding
                        }
                        Err(failure) if failure.retryable => return Step::Wait,
                        Err(failure) => {
                            error!("[harvest-particle] {what}: {}", failure.message);
                            return Step::Done;
                        }
                    }
                }
            };
            match play_object(world, &binding) {
                Ok(played) => info!("[harvest-particle] {what}: {played} systems played"),
                Err(error) => error!("[harvest-particle] {what}: {error}"),
            }
            Step::Done
        }
    }
}
