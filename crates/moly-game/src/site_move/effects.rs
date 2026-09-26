//! The `EffectManager` pools, and the move's pooled effects (types 4, 5 and
//! 16). The camera's speed lines are a canvas image, in `speed_lines`.
//!
//! Pools: `EffectManager.Setup`, which the field scene's setup calls once
//! (before any move, harvest or home action), walks the effect table
//! (`EffectResourceData`, exported as the effect-resources document) row by
//! row, one row a frame. For each row it loads the prefab from the row's
//! bundle, instantiates one template under an `EffectRoot` object,
//! deactivates it (`Finish`), clones it `poolSize` times into the row type's
//! pool (the clones start inactive) and destroys the template. A pool is a
//! fixed ring keyed by type: `EffectManager.Emit` takes the next copy in turn
//! (`ResourcePool.GetResource` advances its index modulo the count and
//! checks nothing) and calls its `Emit`, which places it and plays it
//! (`ManagedEffect.Play`: `SetActive(true)`, then `ParticleSystem.Play()` on
//! the root system, children included), even a copy still playing. No copy
//! is ever returned: `ManagedEffect.Stop` only stops emission. An emit of a
//! type whose pool is not built returns nothing; `Setup` runs in the field
//! scene's `SetupAsync`, and `CreateAssetsPool` loads each row's bundle
//! synchronously (`AssetManager.LoadAssetBundle`, then `UniTask.Yield`), so
//! every pool is built a frame per row inside that setup. `SiteMoveEffect`,
//! `FollowEffect` and `HarvestObjectEffect` do not end themselves: after its
//! systems stop a copy stays active and draws nothing.
//!
//! Here the table and the two indexes that resolve its bundles are requested
//! at startup and the table is read once. Every row whose bundle ships a
//! prefab and a particle document has both requested and its ring of
//! `poolSize` copies made at once; once both have loaded, its copies are
//! instantiated (at most one row a frame), held
//! inactive (their source nodes hidden, so their particle clocks do not run)
//! and their particles prepared while inactive, so that an emit plays a
//! prepared copy at once. A copy's systems are prepared as the explicit
//! `Play` runs them (every emitter selected, the source-particle control
//! preparation), except the landing prefabs' (4 and 5), whose systems are
//! all play-on-awake and which the fixture source path plans as authored. At
//! each Play an explicitly prepared copy's renderers take their emitters'
//! authored `enabled` flag (a copy inactive in its pool keeps them off; the
//! root system, whose renderer the prefabs ship off, is not selected). The
//! foot effects (types 14 and 17) are a pool of their own
//! ([`crate::footstep`]): their packages ship no prefab.
//!
//! The move's effects:
//! - 16 `Flying`: `EmitFollowTargetTransForm(Flying, camera transform)` when
//!   the `_s2` clip passes its `PublishFlyingEffectPlay` event, stopped by
//!   `EffectManager.Stop(Flying)` when a landing clip starts (its
//!   `PublishFlyingEffectEnd` event sits at 0). `FollowEffect.LateUpdate`
//!   copies the camera's position and its euler rotation every frame (no
//!   axis frozen). The emit call adds 180 degrees of yaw once, but that write
//!   is replaced by the first `LateUpdate`, before any frame renders it, so
//!   the visible effect follows the camera pose itself. The flying prefab's
//!   systems are not play-on-awake: they run only through the explicit Play.
//! - 4 `SiteMoveEndPlayerEffect` / 5 `SiteMoveFailedPlayerEffect`:
//!   `SiteMoveEffect.Emit` puts the pooled effect at the player's position
//!   and plays it.
//!
//! Named differences:
//! - Rows are built in the order their prefab and document finish loading
//!   (still one row a frame), not in table order.
//! - The loads are asynchronous here, so a row can still be loading when an
//!   emit comes, where the source's pool has long been built. The emit takes
//!   the ring's next copy as the source's does and places it; the copy
//!   starts when it has been instantiated and prepared (the same late start
//!   as a copy played before its systems were prepared), not at the emit.
//! - A copy the ring reaches again is played by the particle host's own
//!   `ParticleSystem.Play` ([`crate::weather_fx::fixture::play`]: a system
//!   that still holds particles keeps its seeds and restarts its clock; one
//!   that holds none plays as at its first Play).
//! - `ManagedEffect.Stop` is `ParticleSystem.Stop()`, which stops emitting
//!   and lets live particles finish: the stopped flying copy stays active
//!   and keeps following the camera (`FollowEffect` never ends itself) until
//!   the next flying emit restarts the pool's copy.
//! - An emitter the particle host refuses (the flying prefab's `pt_01`
//!   emits by distance only, which the host does not run) is skipped with a
//!   WARN; the prefab's other emitters play.
//! - A row without an exported prefab, or whose prefab or document is
//!   missing, is one WARN and no pool: its emits draw nothing.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::{SetSourceActive, SourceNodeActivity};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::{InstanceReady, PendingInstance, SiteMoveOwned};

/// `EffectManager`'s `EffectType` values the product plays by name. The
/// move's three come from the harvest action family, whose packages ship a
/// prefab glb; the player's two foot effects (played by [`crate::footstep`])
/// come from the common action family, whose packages hold no mesh and so
/// ship no glb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectType {
    SiteMoveEndPlayer = 4,
    SiteMoveFailedPlayer = 5,
    Dash = 14,
    Flying = 16,
    WalkWater = 17,
}

const TYPES: [EffectType; 3] = [
    EffectType::Flying,
    EffectType::SiteMoveEndPlayer,
    EffectType::SiteMoveFailedPlayer,
];

/// The particle packages of the move's pooled effects (the release lists
/// them in the fixture-particles-v2 index when it ships them).
pub(crate) fn packages() -> impl Iterator<Item = String> {
    TYPES.into_iter().map(EffectType::package)
}

impl EffectType {
    /// The package's last name segment (`EffectResourceData` bundle names).
    fn leaf(self) -> &'static str {
        match self {
            Self::SiteMoveEndPlayer => "fx_act_user_landing",
            Self::SiteMoveFailedPlayer => "fx_act_user_landing_fail",
            Self::Flying => "fx_act_user_flying",
            Self::Dash => "dash",
            Self::WalkWater => "walk_water",
        }
    }

    /// The prefab `EffectResourceData` names in the package; its root node.
    pub(crate) fn prefab(self) -> &'static str {
        match self {
            Self::Dash => "fx_act_user_walking",
            Self::WalkWater => "fx_act_user_walking_water",
            other => other.leaf(),
        }
    }

    pub(crate) fn package(self) -> String {
        match self {
            Self::Dash | Self::WalkWater => {
                format!("mysekai__effect__site__common__action__{}", self.leaf())
            }
            _ => format!("mysekai__effect__site__harvest__action__{}", self.leaf()),
        }
    }

    pub(crate) fn doc_path(self) -> String {
        format!("moly://fixture-particles-v2/{}.json", self.package())
    }
}

/// The effect table (`EffectResourceData`).
const TABLE: &str = "moly://effect-resources/effect-resources.json";
/// The harvest object effects' prefabs, by package.
const OBJECT_INDEX: &str = "moly://site-harvest-object-effects/index.json";

/// How a row's copies are prepared for `ManagedEffect.Play`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayPath {
    /// The explicit Play of the root system with its children: every
    /// emitter selected, prepared as control systems.
    Explicit,
    /// Play-on-awake systems, planned as authored by the fixture source path.
    OnAwake,
}

/// The log tag of a row: the player that emits it.
fn log_tag(kind: u16) -> &'static str {
    match kind {
        4 | 5 | 16 => "[site-move]",
        15 | 100..=143 => "[harvest-effect]",
        _ => "[effect-pool]",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowState {
    /// Prefab or document still loading.
    Loading,
    /// Its copies are instantiated (the ring exists).
    Built,
    /// Its prefab or document failed to load: no pool.
    Failed,
}

/// One table row with a prefab and a particle document to draw it from.
struct Row {
    kind: u16,
    /// The type's declared name (the table's `effectTypeName`).
    name: String,
    prefab: String,
    pool_size: usize,
    path: PlayPath,
    glb_path: String,
    doc_path: String,
    glb: Handle<Gltf>,
    doc: Handle<JsonAsset>,
    /// The document, parsed once for all the row's copies.
    parsed: Option<Arc<Value>>,
    state: RowState,
    /// The ring: indices into the pools' copies.
    ring: Vec<usize>,
    /// `ResourcePool.pickIndex`.
    pick: usize,
}

/// One copy of a row's prefab, in its ring for the life of the app.
struct PoolCopy {
    kind: u16,
    /// Its place in the ring.
    slot: usize,
    root: Entity,
    age: f32,
    planned: bool,
    /// Its source nodes are held inactive (in the pool, never played).
    held: bool,
    /// Emits that took this copy.
    plays: u32,
    /// Age at its last emit.
    played_at: Option<f32>,
    /// Age at which its systems were prepared.
    prepared_at: Option<f32>,
    /// `ManagedEffect.Stop` ran since its last emit.
    stopped: bool,
    /// Each prepared control draw with its emitter's authored
    /// `renderer.enabled` (the explicit Play's preparation).
    draws: Vec<(Entity, bool)>,
    timing: PrepTiming,
    /// Next sample time after the last emit (instrument).
    next_sample: f32,
}

enum TableState {
    Requested {
        table: Handle<JsonAsset>,
        objects: Handle<JsonAsset>,
        particles: Handle<JsonAsset>,
    },
    Read,
    Absent,
}

/// `EffectManager._assetPool`: the pools of every drawable table row.
#[derive(Resource)]
pub(crate) struct EffectPools {
    table: TableState,
    rows: Vec<Row>,
    copies: Vec<PoolCopy>,
}

/// What an emit took.
pub(crate) struct Emitted {
    pub(crate) root: Entity,
    pub(crate) name: String,
    pub(crate) prefab: String,
    pub(crate) pool_size: usize,
    /// The copy's place in its ring (from 0).
    pub(crate) slot: usize,
    /// Emits that took this copy, this one included.
    pub(crate) plays: u32,
    /// Systems installed under the copy.
    pub(crate) systems: usize,
    /// The copy's age at this emit.
    pub(crate) age: f32,
    /// Seconds since its systems were prepared (`None`: not yet).
    pub(crate) since_prepared: Option<f32>,
}

/// What a `ManagedEffect.Stop` reached.
pub(crate) struct Stopped {
    pub(crate) kind: u16,
    pub(crate) systems: usize,
    pub(crate) prepared: bool,
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(Startup, request_pools)
        .add_systems(Update, advance_pools);
}

/// `EffectManager.Setup`: request the table and the indexes that resolve
/// its bundles.
fn request_pools(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(EffectPools {
        table: TableState::Requested {
            table: server.load(TABLE),
            objects: server.load(OBJECT_INDEX),
            particles: server.load(super::products::INDEX),
        },
        rows: Vec::new(),
        copies: Vec::new(),
    });
}

fn advance_pools(world: &mut World) {
    if !world.contains_resource::<EffectPools>() {
        return;
    }
    let dt = world.resource::<Time>().delta_secs();
    world.resource_scope(|world, mut pools: Mut<EffectPools>| pools.advance(world, dt));
}

/// Loaded (Some(true)), failed (Some(false)) or pending (None).
fn json_state(server: &AssetServer, handle: &Handle<JsonAsset>) -> Option<bool> {
    match server.load_state(handle) {
        bevy::asset::LoadState::Loaded => Some(true),
        bevy::asset::LoadState::Failed(_) => Some(false),
        _ => None,
    }
}

impl EffectPools {
    /// One pass over the table once it and the indexes have loaded: every
    /// row with an exported prefab and a particle document gets both
    /// requested.
    fn read_table(&mut self, world: &mut World) {
        let TableState::Requested {
            table,
            objects,
            particles,
        } = &self.table
        else {
            return;
        };
        let server = world.resource::<AssetServer>().clone();
        let (Some(table_ok), Some(objects_ok), Some(particles_ok)) = (
            json_state(&server, table),
            json_state(&server, objects),
            json_state(&server, particles),
        ) else {
            return;
        };
        if !table_ok || !particles_ok {
            warn!(
                "[effect-pool] {} failed to load: no effect pool is built, and every EffectManager.Emit draws nothing",
                if table_ok { super::products::INDEX } else { TABLE }
            );
            self.table = TableState::Absent;
            return;
        }
        let json = world.resource::<Assets<JsonAsset>>();
        let parse = |handle: &Handle<JsonAsset>, path: &str| -> Value {
            let text = &json
                .get(handle)
                .unwrap_or_else(|| panic!("{path}: loaded but not in the assets"))
                .0;
            serde_json::from_str(text).unwrap_or_else(|error| panic!("{path}: not JSON: {error}"))
        };
        let table_doc = parse(table, TABLE);
        let particle_doc = parse(particles, super::products::INDEX);
        let object_doc = if objects_ok {
            parse(objects, OBJECT_INDEX)
        } else {
            warn!(
                "[effect-pool] {OBJECT_INDEX} failed to load: the harvest object effects have no prefab"
            );
            Value::Null
        };
        let glb_of_package: HashMap<&str, &str> = object_doc["effects"]
            .as_object()
            .map(|objects| {
                objects
                    .values()
                    .filter_map(|row| Some((row["package"].as_str()?, row["glb"].as_str()?)))
                    .collect()
            })
            .unwrap_or_default();
        let particle_docs = particle_doc["packages"]
            .as_object()
            .unwrap_or_else(|| panic!("{}: no packages", super::products::INDEX));
        let rows = table_doc["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("{TABLE}: no resources"));
        let mut foot = Vec::new();
        for row in rows {
            let kind = row["effectType"]
                .as_u64()
                .unwrap_or_else(|| panic!("{TABLE}: row without effectType"))
                as u16;
            if kind == EffectType::Dash as u16 || kind == EffectType::WalkWater as u16 {
                foot.push(kind);
                continue;
            }
            let name = row["effectTypeName"].as_str().unwrap_or("").to_owned();
            let prefab = row["prefabName"]
                .as_str()
                .unwrap_or_else(|| panic!("{TABLE}: type {kind} without prefab"))
                .to_owned();
            let package = row["bundle"]["package"]
                .as_str()
                .unwrap_or_else(|| panic!("{TABLE}: type {kind} without a bundle package"))
                .to_owned();
            let glb_path = if let Some(glb) = glb_of_package.get(package.as_str()) {
                format!("moly://site-harvest-object-effects/{glb}")
            } else if package.contains("__harvest__action__") {
                format!("moly://site-action-effects/{prefab}/{prefab}.glb")
            } else if package.contains("__site__home__") {
                format!("moly://site-home-effects/{prefab}/{prefab}.glb")
            } else {
                warn!(
                    "[effect-pool] type {kind} ({name}): bundle {package} has no exported prefab; no pool, not drawn"
                );
                continue;
            };
            // The particle document is requested only when the release lists it.
            let Some(file) = particle_docs
                .get(&package)
                .filter(|row| row["missing"] != true)
                .and_then(|row| row["file"].as_str())
            else {
                warn!(
                    "[effect-pool] type {kind} ({name}): the particle document of {package} is not in the release's particle index; no pool, not drawn"
                );
                continue;
            };
            let doc_path = format!("moly://fixture-particles-v2/{file}");
            let pool_size = row["poolSize"]
                .as_u64()
                .unwrap_or_else(|| panic!("{TABLE}: type {kind} without poolSize"))
                as usize;
            if pool_size == 0 {
                // `CreateAssetsPool` clones nothing below 1: an empty ring.
                warn!("[effect-pool] type {kind} ({name}): pool size 0; no copy, not drawn");
                continue;
            }
            let path = if kind == EffectType::SiteMoveEndPlayer as u16
                || kind == EffectType::SiteMoveFailedPlayer as u16
            {
                PlayPath::OnAwake
            } else {
                PlayPath::Explicit
            };
            self.rows.push(Row {
                kind,
                glb: server.load(bevy::asset::AssetPath::from(glb_path.clone())),
                doc: server.load(bevy::asset::AssetPath::from(doc_path.clone())),
                name,
                prefab,
                pool_size,
                path,
                glb_path,
                doc_path,
                parsed: None,
                state: RowState::Loading,
                ring: Vec::new(),
                pick: 0,
            });
        }
        let kinds: Vec<(u16, usize)> = self
            .rows
            .iter()
            .map(|row| (row.kind, row.pool_size))
            .collect();
        info!(
            "[effect-pool] EffectManager.Setup: effect table read, {} of {} rows drawable (type, pool size) {kinds:?}, prefabs and particle documents requested; types {foot:?} are the foot effects' own pool",
            self.rows.len(),
            rows.len()
        );
        self.table = TableState::Read;
        // Every row's ring exists from here, as the source's pools do once
        // `Setup`'s pass has run: an emit takes the ring's next copy while
        // the copy's prefab or document is still loading.
        for index in 0..self.rows.len() {
            self.allocate_ring(world, index);
        }
    }

    /// The row's `poolSize` copies, empty until its prefab has loaded (held
    /// hidden, out of any site's rendering hold).
    fn allocate_ring(&mut self, world: &mut World, index: usize) {
        let (kind, pool_size, name) = {
            let row = &self.rows[index];
            (row.kind, row.pool_size, row.name.clone())
        };
        let mut ring = Vec::with_capacity(pool_size);
        for slot in 0..pool_size {
            let root = world
                .spawn((
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    SiteMoveOwned,
                    Name::new(format!("effect copy {name} {slot}")),
                ))
                .id();
            ring.push(self.copies.len());
            self.copies.push(PoolCopy {
                kind,
                slot,
                root,
                age: 0.0,
                planned: false,
                held: false,
                plays: 0,
                played_at: None,
                prepared_at: None,
                stopped: false,
                draws: Vec::new(),
                timing: PrepTiming::start(),
                next_sample: 0.0,
            });
        }
        self.rows[index].ring = ring;
    }

    /// Instantiate the copies of one row whose prefab and document have
    /// loaded (`CreateAssetsPool` builds one row a frame).
    fn build_one_row(&mut self, world: &mut World) {
        let server = world.resource::<AssetServer>().clone();
        for index in 0..self.rows.len() {
            let row = &mut self.rows[index];
            if row.state != RowState::Loading {
                continue;
            }
            let failed = server.load_state(&row.glb).is_failed()
                || server.recursive_dependency_load_state(&row.glb).is_failed()
                || server.load_state(&row.doc).is_failed();
            if failed {
                warn!(
                    "{} effect {} ({}): prefab {} or particle document {} failed to load; no pool, not shown",
                    log_tag(row.kind),
                    row.name,
                    row.kind,
                    row.glb_path,
                    row.doc_path
                );
                row.state = RowState::Failed;
                let roots: Vec<Entity> = row.ring.iter().map(|&i| self.copies[i].root).collect();
                for root in roots {
                    world.despawn(root);
                }
                continue;
            }
            if !(server.is_loaded_with_dependencies(&row.glb)
                && server.load_state(&row.doc).is_loaded())
            {
                continue;
            }
            let scene = world
                .resource::<Assets<Gltf>>()
                .get(&row.glb)
                .and_then(|gltf| gltf.default_scene.clone());
            let Some(scene) = scene else {
                warn!(
                    "{} effect {} ({}) prefab {} has no default scene: no pool, not shown",
                    log_tag(row.kind),
                    row.name,
                    row.kind,
                    row.glb_path
                );
                row.state = RowState::Failed;
                let roots: Vec<Entity> = row.ring.iter().map(|&i| self.copies[i].root).collect();
                for root in roots {
                    world.despawn(root);
                }
                continue;
            };
            let (kind, pool_size, name) = (row.kind, row.pool_size, row.name.clone());
            let ring = row.ring.clone();
            let mut early = 0;
            for &copy in &ring {
                let (root, played) = (self.copies[copy].root, self.copies[copy].plays > 0);
                // An unplayed copy stays hidden until its source nodes are
                // held inactive (the first frame its instance exists); one
                // emitted while loading keeps the visibility its emit set.
                let Ok(mut entity) = world.get_entity_mut(root) else {
                    warn!(
                        "{} effect {name} copy {root:?} is gone before its prefab loaded",
                        log_tag(kind)
                    );
                    continue;
                };
                entity.insert((SceneRoot(scene.clone()), PendingInstance));
                if !played {
                    entity.insert(Visibility::Hidden);
                }
                early += usize::from(played);
                self.copies[copy].timing = PrepTiming::start();
            }
            let row = &mut self.rows[index];
            row.state = RowState::Built;
            info!(
                "[effect-pool] type {kind} {name}: pool built, {pool_size} copies of {} ({early} emitted while it loaded, starting when prepared)",
                row.prefab
            );
            return;
        }
    }

    /// Settled when the table has been read (or is absent) and each of
    /// `kinds` has its pool built, failed, or no row to draw it from.
    pub(crate) fn settled(&self, kinds: &[u16]) -> bool {
        match self.table {
            TableState::Requested { .. } => false,
            TableState::Absent => true,
            TableState::Read => kinds.iter().all(|kind| {
                self.rows
                    .iter()
                    .find(|row| row.kind == *kind)
                    .is_none_or(|row| row.state != RowState::Loading)
            }),
        }
    }

    /// `EffectManager.Emit`: the ring's next copy (`GetResource`) is put at
    /// `pose` and played (`ManagedEffect.Play`: `SetActive(true)` then
    /// `ParticleSystem.Play()`), even one still playing. `Err` names why
    /// nothing is drawn (the type has no pool).
    pub(crate) fn emit(
        &mut self,
        world: &mut World,
        kind: u16,
        pose: Transform,
    ) -> Result<Emitted, String> {
        let Some(row) = self.rows.iter_mut().find(|row| row.kind == kind) else {
            return Err(match self.table {
                TableState::Requested { .. } => {
                    "the effect table is not read yet: no pool".to_owned()
                }
                _ => "the type has no pool (no drawable row in the effect table)".to_owned(),
            });
        };
        match row.state {
            // The ring exists: the copy is placed now and starts when its
            // instance is made and prepared.
            RowState::Loading | RowState::Built => {}
            RowState::Failed => {
                return Err(format!(
                    "{} has no pool (its prefab or particle document failed)",
                    row.name
                ))
            }
        }
        let slot = row.pick;
        row.pick = (row.pick + 1) % row.ring.len();
        let (name, prefab, pool_size, path) = (
            row.name.clone(),
            row.prefab.clone(),
            row.pool_size,
            row.path,
        );
        let copy = &mut self.copies[row.ring[slot]];
        let root = copy.root;
        if let Some(mut transform) = world.get_mut::<Transform>(root) {
            *transform = pose;
        }
        if let Some(mut visibility) = world.get_mut::<Visibility>(root) {
            *visibility = Visibility::Inherited;
        }
        if std::mem::take(&mut copy.held) {
            set_source_nodes_active(world, root, true);
        }
        if copy.plays > 0 && copy.planned {
            // Played again: ParticleSystem.Play() on a system that has run.
            if let Err(error) = crate::weather_fx::fixture::play(world, root) {
                warn!(
                    "{} effect {name} copy {}: Play again refused: {error}",
                    log_tag(kind),
                    slot + 1
                );
            }
        }
        copy.plays += 1;
        copy.played_at = Some(copy.age);
        copy.next_sample = 0.0;
        copy.stopped = false;
        if copy.planned && path == PlayPath::Explicit {
            enable_renderers_named(world, &name, kind, &copy.draws);
        }
        Ok(Emitted {
            root,
            name,
            prefab,
            pool_size,
            slot,
            plays: copy.plays,
            systems: installed_systems(world, root),
            age: copy.age,
            since_prepared: copy.prepared_at.map(|at| copy.age - at),
        })
    }

    /// `ManagedEffect.Stop` on the copy at `root`: `ParticleSystem.Stop()`
    /// (children included, stop emitting); its live particles finish. A copy
    /// not prepared yet takes the stop when it is.
    pub(crate) fn stop(&mut self, world: &mut World, root: Entity) -> Option<Stopped> {
        let copy = self.copies.iter_mut().find(|copy| copy.root == root)?;
        copy.stopped = true;
        let systems = if copy.planned {
            crate::weather_fx::fixture::stop_emitting(world, root)
        } else {
            0
        };
        Some(Stopped {
            kind: copy.kind,
            systems,
            prepared: copy.planned,
        })
    }

    /// Read the table, build a row, hold new copies inactive and prepare
    /// their particles.
    fn advance(&mut self, world: &mut World, dt: f32) {
        self.read_table(world);
        self.build_one_row(world);
        let sample = sample_enabled();
        for index in 0..self.copies.len() {
            self.copies[index].age += dt;
            let root = self.copies[index].root;
            if world.get::<InstanceReady>(root).is_none() {
                continue;
            }
            self.copies[index].timing.instance_ready();
            if self.copies[index].plays == 0 && !self.copies[index].held {
                // Inactive in the pool: its source nodes are hidden and its
                // particle clocks do not run. The scene root itself becomes
                // visible in the same step, because the particle draws
                // (children of the root) only become ready once the renderer
                // has seen them.
                let held = set_source_nodes_active(world, root, false);
                if held == 0 {
                    warn!(
                        "[effect-pool] type {} copy {}: no source node to hold inactive; its systems run in the pool once prepared",
                        self.copies[index].kind,
                        self.copies[index].slot + 1
                    );
                }
                if let Some(mut visibility) = world.get_mut::<Visibility>(root) {
                    *visibility = Visibility::Inherited;
                }
                self.copies[index].held = true;
            }
            if !self.copies[index].planned {
                self.prepare(world, index);
            }
            if sample {
                self.sample(world, index);
            }
        }
    }

    fn prepare(&mut self, world: &mut World, index: usize) {
        let (kind, root, slot) = {
            let copy = &self.copies[index];
            (copy.kind, copy.root, copy.slot)
        };
        let row = self
            .rows
            .iter_mut()
            .find(|row| row.kind == kind)
            .expect("a row per copy");
        if row.parsed.is_none() {
            let text = world
                .resource::<Assets<JsonAsset>>()
                .get(&row.doc)
                .map(|json| json.0.clone());
            let Some(text) = text else {
                // Loaded before the copies were made; gone only if unloaded.
                warn!(
                    "{} effect {} particle document {} unavailable: not shown",
                    log_tag(kind),
                    row.name,
                    row.doc_path
                );
                self.copies[index].planned = true;
                return;
            };
            let started = bevy::platform::time::Instant::now();
            match serde_json::from_str::<Value>(&text) {
                Ok(doc) => row.parsed = Some(Arc::new(doc)),
                Err(error) => {
                    warn!(
                        "{} effect {} particle document unreadable: {error}",
                        log_tag(kind),
                        row.name
                    );
                    self.copies[index].planned = true;
                    return;
                }
            }
            self.copies[index].timing.parsed(started.elapsed());
        }
        let doc = row.parsed.clone().expect("parsed above");
        let (name, path, pool_size) = (row.name.clone(), row.path, row.pool_size);
        let paths = super::node_paths(world, root);
        match path {
            PlayPath::OnAwake => {
                let anchors: HashMap<String, Vec<Entity>> = paths
                    .iter()
                    .map(|(path, list)| (format!("/{path}"), list.clone()))
                    .collect();
                let server = world.resource::<AssetServer>().clone();
                let mut commands = world.commands();
                crate::weather_fx::fixture::plan(&mut commands, root, &doc, &anchors, &server);
                world.flush();
                self.copies[index].planned = true;
            }
            PlayPath::Explicit => {
                // Explicit Play of the root system with its children. An
                // emitter the particle host refuses is skipped with a WARN
                // and the others play, as the fixture path treats each
                // refused emitter (the preparation judges every selected
                // emitter before it builds anything, so a retry without the
                // refused one starts clean). The preparation readies its
                // emitters one after another.
                let mut selected = select_all(&doc, &paths);
                match prepare_skipping_refused(world, root, &doc, &mut selected, &name) {
                    Prepared::Ready(draws) => {
                        let authored = authored_renderers(world, &doc, &draws);
                        let copy = &mut self.copies[index];
                        info!(
                            "{} effect {name} prepared: {} systems, {:.2}s after its copy was made ({})",
                            log_tag(kind),
                            draws.len(),
                            copy.age,
                            if copy.plays == 0 {
                                "inactive in its pool"
                            } else {
                                "playing"
                            }
                        );
                        info!(
                            "[effect-pool] type {kind} {name} copy {} of {pool_size} preparation stages after its copy was made: {}",
                            slot + 1,
                            copy.timing.report()
                        );
                        copy.planned = true;
                        copy.prepared_at = Some(copy.age);
                        copy.draws = authored;
                        if copy.plays > 0 {
                            // Played before its systems were prepared: they
                            // start now, and so do their renderers.
                            let (draws, stopped, since) = (
                                copy.draws.clone(),
                                copy.stopped,
                                copy.age - copy.played_at.unwrap_or(copy.age),
                            );
                            enable_renderers_named(world, &name, kind, &draws);
                            if stopped {
                                // The source emits from a copy it already
                                // holds; here the copy was still preparing
                                // when its Stop ran, so the Stop takes
                                // effect now.
                                let systems =
                                    crate::weather_fx::fixture::stop_emitting(world, root);
                                info!(
                                    "{} type {kind} {root:?}: prepared {since:.2} s after its Emit, after its ManagedEffect.Stop ran; emission stopped on {systems} systems now",
                                    log_tag(kind)
                                );
                            }
                        }
                    }
                    Prepared::Pending => {
                        self.copies[index]
                            .timing
                            .pending(world, root, selected.len());
                    }
                    Prepared::Refused(error) => {
                        warn!("{} effect {name} particles refused: {error}", log_tag(kind));
                        self.copies[index].planned = true;
                    }
                }
            }
        }
    }

    /// `MOLY_EFFECT_SAMPLE=1` (an instrument, off by default): every 0.25 s
    /// after an emit, the copy's live particles, the particles born so far
    /// and its playing systems, until nothing is left (at most 8 s).
    fn sample(&mut self, world: &World, index: usize) {
        let copy = &mut self.copies[index];
        let (Some(played_at), true) = (copy.played_at, copy.planned) else {
            return;
        };
        let since = copy.age - played_at;
        if since < copy.next_sample || copy.next_sample > 8.0 {
            return;
        }
        let now = world.resource::<Time>().elapsed_secs_f64();
        let (mut systems, mut live, mut born, mut playing) = (0, 0, 0u64, 0);
        if let Some(children) = world.get::<Children>(copy.root) {
            for child in children.iter() {
                let Some(system) = world.get::<crate::uber_particle::FixtureParticleLive>(child)
                else {
                    continue;
                };
                systems += 1;
                live += system.0.pool.len();
                born += system.0.born_total;
                if world
                    .get::<crate::weather_fx::fixture::Played>(child)
                    .is_some_and(|played| played.playing(&system.0, now))
                {
                    playing += 1;
                }
            }
        }
        info!(
            "[effect-sample] type {} copy {} {:.2}s after its Emit (play {}){}: {live} live particles, {born} born, {playing} of {systems} systems playing",
            copy.kind,
            copy.slot + 1,
            since,
            copy.plays,
            if copy.stopped { ", stopped" } else { "" }
        );
        copy.next_sample += 0.25;
        if since > 0.5 && live == 0 && playing == 0 {
            // Nothing left to sample until the next emit.
            copy.next_sample = f32::INFINITY;
        }
    }
}

fn sample_enabled() -> bool {
    static SAMPLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SAMPLE.get_or_init(|| {
        let on = std::env::var("MOLY_EFFECT_SAMPLE").is_ok_and(|value| value == "1");
        if on {
            warn!("[effect-sample] MOLY_EFFECT_SAMPLE=1: live particle samples of played effect copies on (instrument)");
        }
        on
    })
}

/// Whether the move's effect pools are settled (built, failed or absent).
pub(crate) fn move_pools_settled(world: &World) -> bool {
    world
        .get_resource::<EffectPools>()
        .is_none_or(|pools| pools.settled(&TYPES.map(|kind| kind as u16)))
}

/// The move's effect state around the shared pools: the flying copy the
/// camera carries.
#[derive(Default)]
pub(crate) struct Effects {
    flying: Option<Entity>,
    /// The flying copy after its Stop, still following.
    stopped: Option<Entity>,
}

impl Effects {
    /// `EffectManager.Emit(type, position)` / `EmitFollowTargetTransForm`:
    /// `ManagedEffect.Play` on the pool's next copy, i.e. `SetActive(true)`
    /// then `Play`.
    pub(crate) fn emit(&mut self, world: &mut World, kind: EffectType, pose: Transform) -> bool {
        if !world.contains_resource::<EffectPools>() {
            warn!("[site-move] effect {kind:?} not shown: no effect pools");
            return false;
        }
        let emitted = world.resource_scope(|world, mut pools: Mut<EffectPools>| {
            pools.emit(world, kind as u16, pose)
        });
        let root = match emitted {
            Ok(emitted) => {
                if emitted.systems > 0 {
                    info!(
                        "[site-move] effect {kind:?} played from its pool: {} systems prepared {:.2}s before",
                        emitted.systems, emitted.age
                    );
                } else {
                    warn!(
                        "[site-move] effect {kind:?} played from its pool {:.2}s after its copy was made, before any of its systems was prepared; they start when prepared",
                        emitted.age
                    );
                }
                if emitted.plays > 1 {
                    info!(
                        "[site-move] effect {kind:?}: copy {} of {} played again (play {}), its systems restarted",
                        emitted.slot + 1,
                        emitted.pool_size,
                        emitted.plays
                    );
                }
                emitted.root
            }
            Err(reason) => {
                warn!("[site-move] effect {kind:?} not shown: {reason}");
                return false;
            }
        };
        if kind == EffectType::Flying {
            self.flying = Some(root);
            // The ring's copy restarted: the camera carries the playing one.
            self.stopped = None;
        }
        true
    }

    /// `EffectManager.Stop(Flying)`.
    pub(crate) fn stop_flying(&mut self, world: &mut World) -> bool {
        let Some(root) = self.flying.take() else {
            return false;
        };
        if installed_systems(world, root) == 0 {
            warn!("[site-move] effect Flying stopped with none of its systems prepared: not shown");
        }
        // ManagedEffect.Stop: ParticleSystem.Stop(withChildren, StopEmitting);
        // the copy stays active and following, its particles finish.
        if world.contains_resource::<EffectPools>() {
            world.resource_scope(|world, mut pools: Mut<EffectPools>| pools.stop(world, root));
        }
        self.stopped = Some(root);
        true
    }

    /// The flying copy the camera carries: the playing one, else the
    /// stopped one whose particles are finishing.
    pub(crate) fn flying_root(&self) -> Option<Entity> {
        self.flying.or(self.stopped)
    }

    /// Per-frame trace of the flying copy's draws (`MOLY_EFFECT_TRACE=1`).
    pub(crate) fn advance(&mut self, world: &mut World, _dt: f32) {
        if let Some(root) = self.flying {
            trace_draws(world, "Flying", root);
        }
    }
}

/// Particle systems installed under an instance.
fn installed_systems(world: &World, root: Entity) -> usize {
    world
        .get::<Children>(root)
        .map(|children| {
            children
                .iter()
                .filter(|child| {
                    world
                        .get::<crate::uber_particle::FixtureParticleLive>(*child)
                        .is_some()
                })
                .count()
        })
        .unwrap_or(0)
}

/// Pair each prepared control draw with its emitter's authored
/// `renderer.enabled`, found by the emitter node the draw's system runs.
pub(crate) fn authored_renderers(
    world: &World,
    doc: &Value,
    draws: &[Entity],
) -> Vec<(Entity, bool)> {
    let emitters = doc["emitters"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    draws
        .iter()
        .map(|&draw| {
            let node = world
                .get::<crate::uber_particle::FixtureParticleLive>(draw)
                .map(|live| live.0.node.clone())
                .unwrap_or_default();
            let emitter = emitters.iter().find(|emitter| emitter["node"] == node.as_str());
            let Some(emitter) = emitter else {
                error!("[effects] prepared draw for {node:?} has no emitter in its document; its renderer stays off");
                return (draw, false);
            };
            (draw, emitter["renderer"]["enabled"] == true)
        })
        .collect()
}

/// The Play step (`SetActive(true)` + `Play`): each prepared draw's renderer
/// takes its emitter's authored `enabled` (a renderer the prefab ships off
/// stays off; nothing turns renderers on or off afterwards).
pub(crate) fn enable_renderers(world: &mut World, kind: EffectType, draws: &[(Entity, bool)]) {
    enable_renderers_named(world, &format!("{kind:?}"), kind as u16, draws);
}

/// [`enable_renderers`] for a table row, by its declared name.
fn enable_renderers_named(world: &mut World, name: &str, kind: u16, draws: &[(Entity, bool)]) {
    if draws.is_empty() {
        return;
    }
    let mut on = 0;
    for &(draw, enabled) in draws {
        if let Some(mut source) = world.get_mut::<crate::source_particle::SourceParticle>(draw) {
            source.enabled = enabled;
            on += usize::from(enabled);
        }
    }
    info!(
        "[effects] effect {name} ({kind}) plays: {on} of {} prepared renderers on, as authored",
        draws.len()
    );
}

/// Per-frame trace of an effect instance's draws (`MOLY_EFFECT_TRACE=1`,
/// an instrument; off by default): the renderer's enabled flag, the draw's
/// inherited visibility and the live particle count of its system after the
/// last particle step.
pub(crate) fn trace_draws(world: &World, label: &str, root: Entity) {
    if !trace_enabled() {
        return;
    }
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let Some(children) = world.get::<Children>(root) else {
        info!("[effect-trace] frame {frame} {label}: no draws");
        return;
    };
    for child in children.iter() {
        let Some(live) = world.get::<crate::uber_particle::FixtureParticleLive>(child) else {
            continue;
        };
        let enabled = world
            .get::<crate::source_particle::SourceParticle>(child)
            .map(|source| source.enabled);
        let visible = world.get::<InheritedVisibility>(child).map(|v| v.get());
        info!(
            "[effect-trace] frame {frame} {label} draw {} enabled={} inherited_visible={} live={} head={:.3}",
            live.0.node,
            enabled.map_or("none".into(), |e| e.to_string()),
            visible.map_or("none".into(), |v| v.to_string()),
            live.0.pool.len(),
            live.0.playback_head
        );
    }
}

pub(crate) fn trace_enabled() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| {
        let on = std::env::var("MOLY_EFFECT_TRACE").is_ok_and(|value| value == "1");
        if on {
            warn!(
                "[effect-trace] MOLY_EFFECT_TRACE=1: per-frame effect draw trace on (instrument)"
            );
        }
        on
    })
}

/// Wall-clock stages of one copy's preparation (an instrument, reported once
/// the copy is prepared): its scene instance ready, its first particle draw
/// spawned (the draw's geometry and shader catalogue resolved), every draw's
/// textures and shader programs loaded, and every draw's GPU pipeline ready
/// (the preparation's end), with the time spent parsing the particle
/// document.
pub(crate) struct PrepTiming {
    made: bevy::platform::time::Instant,
    instance: Option<f32>,
    first_draw: Option<f32>,
    assets: Option<f32>,
    parse: std::time::Duration,
    parses: u32,
    pending_frames: u32,
}

impl PrepTiming {
    pub(crate) fn start() -> Self {
        Self {
            made: bevy::platform::time::Instant::now(),
            instance: None,
            first_draw: None,
            assets: None,
            parse: std::time::Duration::ZERO,
            parses: 0,
            pending_frames: 0,
        }
    }

    fn now(&self) -> f32 {
        self.made.elapsed().as_secs_f32()
    }

    /// The copy's scene instance is ready (first call counts).
    pub(crate) fn instance_ready(&mut self) {
        if self.instance.is_none() {
            self.instance = Some(self.now());
        }
    }

    /// One parse of the particle document.
    pub(crate) fn parsed(&mut self, took: std::time::Duration) {
        self.parse += took;
        self.parses += 1;
    }

    /// A frame whose preparation is still pending, with the number of
    /// systems it prepares.
    pub(crate) fn pending(&mut self, world: &World, root: Entity, systems: usize) {
        self.pending_frames += 1;
        let server = world.resource::<AssetServer>();
        let draws: Vec<&crate::source_particle::SourceParticle> = world
            .get::<Children>(root)
            .map(|children| {
                children
                    .iter()
                    .filter_map(|child| world.get::<crate::source_particle::SourceParticle>(child))
                    .collect()
            })
            .unwrap_or_default();
        if draws.is_empty() {
            return;
        }
        if self.first_draw.is_none() {
            self.first_draw = Some(self.now());
        }
        let loaded = draws.len() >= systems
            && draws.iter().all(|draw| {
                !draw.passes.is_empty()
                    && draw
                        .passes
                        .iter()
                        .all(|pass| server.load_state(&pass.program).is_loaded())
                    && draw
                        .textures
                        .values()
                        .all(|texture| server.load_state(texture).is_loaded())
            });
        if loaded && self.assets.is_none() {
            self.assets = Some(self.now());
        }
    }

    /// The stages, read when the preparation is done.
    pub(crate) fn report(&self) -> String {
        let stage = |at: Option<f32>| at.map_or("not seen".to_owned(), |at| format!("{at:.2}s"));
        format!(
            "scene instance {}, first particle draw {}, textures and shader programs {}, GPU pipelines ready {:.2}s; particle document parsed {}x in {:.1} ms; {} frames pending",
            stage(self.instance),
            stage(self.first_draw),
            stage(self.assets),
            self.now(),
            self.parses,
            self.parse.as_secs_f64() * 1000.0,
            self.pending_frames
        )
    }
}

/// Outcome of a control preparation that skips refused emitters.
pub(crate) enum Prepared {
    /// Not ready yet; call again next frame (with the same root).
    Pending,
    /// Every remaining selected emitter has its draw and system.
    Ready(Vec<Entity>),
    /// The host refused the last remaining emitter (or the document).
    Refused(String),
}

/// Explicit Play of a prefab's root system with its children: prepare the
/// selected emitters as control-driven systems. An emitter the particle host
/// refuses is skipped with a WARN and the others play, as the fixture path
/// treats each refused emitter (the preparation judges every selected
/// emitter before it builds anything, so a retry without the refused one
/// starts clean). The preparation readies its emitters one after another.
pub(crate) fn prepare_skipping_refused(
    world: &mut World,
    root: Entity,
    doc: &Value,
    selected: &mut Vec<(Entity, usize)>,
    label: &str,
) -> Prepared {
    loop {
        match crate::weather_fx::fixture::prepare_control(world, root, doc, selected) {
            Ok(Some(draws)) => return Prepared::Ready(draws),
            Ok(None) => return Prepared::Pending,
            Err(error) => {
                let refused = selected.iter().position(|&(_, ordinal)| {
                    let node = &doc["emitters"][ordinal]["node"];
                    error.starts_with(&format!("{node}: source particle control rejected"))
                });
                match refused {
                    Some(position) if selected.len() > 1 => {
                        warn!("[effects] effect {label} emitter skipped: {error}");
                        selected.remove(position);
                    }
                    _ => return Prepared::Refused(error),
                }
            }
        }
    }
}

/// Every emitter of the document with a source-owned renderer whose node is
/// among `paths`, as `(anchor, emitter ordinal)`.
pub(crate) fn select_all(
    doc: &Value,
    paths: &HashMap<String, Vec<Entity>>,
) -> Vec<(Entity, usize)> {
    let Some(emitters) = doc["emitters"].as_array() else {
        return Vec::new();
    };
    emitters
        .iter()
        .enumerate()
        // A system without a source-owned renderer draws nothing (the root
        // system's renderer is off); Play still runs it, invisibly.
        .filter(|(_, emitter)| crate::weather_fx::fixture::is_source_particle(emitter))
        .filter_map(|(ordinal, emitter)| {
            let node = emitter["node"].as_str()?;
            let anchor = paths.get(node)?.first().copied()?;
            Some((anchor, ordinal))
        })
        .collect()
}

/// `SetActive` on the instance's source nodes: the prefab's top objects, the
/// topmost entities under the scene root that carry a source node's
/// activity. Neither the scene root nor the scene's own root entity between
/// it and the prefab's objects is a source node, and `SetSourceActive` does
/// nothing on an entity without that activity. Returns how many were set.
pub(crate) fn set_source_nodes_active(world: &mut World, root: Entity, active: bool) -> usize {
    let mut nodes = Vec::new();
    let mut stack: Vec<Entity> = world
        .get::<Children>(root)
        .map(|children| children.to_vec())
        .unwrap_or_default();
    while let Some(entity) = stack.pop() {
        if world.get::<SourceNodeActivity>(entity).is_some() {
            nodes.push(entity);
        } else if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    for &entity in &nodes {
        world.commands().queue(SetSourceActive { entity, active });
    }
    world.flush();
    nodes.len()
}

/// `FollowEffect.LateUpdate`: the flying effect takes the camera's pose. Its
/// subtree's global transforms are written here too, because this runs after
/// transform propagation and the particle step reads them this frame.
pub(crate) fn follow_camera(
    root: Entity,
    camera: GlobalTransform,
    nodes: &mut Query<(&mut Transform, &mut GlobalTransform), Without<Camera3d>>,
    children: &Query<&Children>,
) {
    if let Ok((mut local, _)) = nodes.get_mut(root) {
        *local = camera.compute_transform();
    }
    let mut stack = vec![(root, camera)];
    while let Some((entity, global)) = stack.pop() {
        if let Ok((_, mut own)) = nodes.get_mut(entity) {
            *own = global;
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids.iter() {
                if let Ok((local, _)) = nodes.get(kid) {
                    stack.push((kid, global.mul_transform(*local)));
                }
            }
        }
    }
}
