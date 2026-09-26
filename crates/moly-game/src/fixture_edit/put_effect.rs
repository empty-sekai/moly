//! The field put effect and the put sound of a decided ground placement.
//!
//! Source (`FixtureController.ShowPutEffect`, field branch): the fixture's
//! current grid size (the master size with x and z swapped for the two side
//! directions), the view's world position and the model's put sound id go to
//! `MysekaiFixtureUtility.ShowFieldPutEffect(..., clone: false)`. That
//! utility creates one `FixturePutEffect` on the first put and keeps it; its
//! `SetupPutEffect` loads the effect package once and instantiates its prefab
//! `fx_fixture_put_01` once. Every later put reuses that one instance
//! (`clone` is false on this path): `SetEffect` deactivates it, writes the
//! position, the local scale `(size.x * TILE_SCALE, size.y, size.z *
//! TILE_SCALE)` (the y component is the raw grid height, not scaled) and the
//! identity rotation, and activates it again, which restarts its
//! play-on-awake systems. Then `PlayPutSound` looks the id up in the put
//! sound table and plays the row's `assetbundleName` as a one-shot SE cue
//! (`PlaySEOneShot(cue, 0)`, no cue sheet argument). The 3.0 s `Destroy`
//! of the source body runs only for a cloned copy, which the fence and road
//! planners request; ground puts never destroy the instance.
//!
//! `FixtureController.PlayPutSound` (the same table lookup, without the
//! effect) is what selecting a placed fixture plays after its pick sound.
//!
//! Callers the product does not build: the wall put (`ShowWallPutEffect`,
//! rotation from the wall direction), fence and road placement
//! (`ShowFieldPlacementEffect` with cloned copies), the fence edit view and
//! the site view's own put effect. Ground editing is the only editor here.
//!
//! Named differences:
//! - The instance is held inactive once its scene exists and its particle
//!   systems are prepared while inactive (the effect pools' landing path);
//!   the source's first instance plays on awake where it is created and the
//!   first put restarts it in the same frame, so the visible result is the
//!   put's play. A later put restarts the systems with the particle host's
//!   own `ParticleSystem.Play` (a system that still holds particles keeps its
//!   seeds and restarts its clock).
//! - Puts that arrive while the prefab or its particle document is loading
//!   wait in order, as the source's awaiting callers do; each plays its sound
//!   when the instance is set. The wait also covers the particle host's
//!   preparation of the systems (their meshes and GPU programs), which the
//!   source has no counterpart for; it is bounded at 10 s.
//! - Systems the particle host refuses (it logs each with its reason) are
//!   not drawn; the others play.
//! - A missing prefab, particle document or put sound table is a WARN and
//!   draws or plays nothing, never a default cue.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::fixture::{Direction, Vector3Int};
use serde_json::Value;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture::EditableFixture;

/// The package `AssetBundleNames.GetMysekaiPutFixtureEffect` names.
const PACKAGE: &str = "mysekai__effect__fixture__fx_fixture_put_01";
/// The prefab `SetupPutEffect` loads from it.
const PREFAB: &str = "fx_fixture_put_01";
const PREFAB_GLB: &str = "moly://fixture-put-effects/fx_fixture_put_01/fx_fixture_put_01.glb";
const FIXTURES: &str = "moly://mysekai-fixtures.json";
const PUT_SOUNDS: &str = "moly://mysekai-fixture-put-sounds.json";
/// `MysekaiConstants.TILE_SCALE`.
const TILE_SCALE: f32 = moly_law::objective::TILE_SCALE;
/// Instrument sampling window after a put, and its step.
const SAMPLE_SECONDS: f32 = 3.5;
const SAMPLE_STEP: f32 = 0.25;
/// How long the first put waits for the particle host's preparation.
const PREPARE_WAIT: f32 = 10.0;

fn particle_doc_path() -> String {
    format!("moly://fixture-particles-v2/{PACKAGE}.json")
}

/// `FieldUtility.ConvertGridSize`: the two side directions swap x and z.
pub(crate) fn current_grid_size(size: Vector3Int, direction: Direction) -> Vector3Int {
    match direction {
        Direction::Front | Direction::Back => size,
        Direction::Left | Direction::Right => Vector3Int::new(size.z, size.y, size.x),
    }
}

/// `SetEffect`'s local scale for a grid size.
pub(crate) fn effect_scale(size: Vector3Int) -> Vec3 {
    Vec3::new(
        size.x as f32 * TILE_SCALE,
        size.y as f32,
        size.z as f32 * TILE_SCALE,
    )
}

/// The put sound id and the handle type of each fixture, and the cue of each
/// put sound row.
struct SoundTables {
    put_sound_of: HashMap<i32, i64>,
    handle_of: HashMap<i32, String>,
    cue_of: HashMap<i64, String>,
}

impl SoundTables {
    fn parse(fixtures: &Value, sounds: &Value) -> Result<Self, String> {
        let rows = fixtures["fixtures"]
            .as_array()
            .ok_or("the fixture table has no fixtures array")?;
        let mut put_sound_of = HashMap::with_capacity(rows.len());
        let mut handle_of = HashMap::with_capacity(rows.len());
        for row in rows {
            let id = row["id"].as_i64().ok_or("a fixture row has no id")?;
            let Some(sound) = row["putSoundId"].as_i64() else {
                return Err(format!(
                    "fixture row {id} carries no putSoundId (a release table from before the column was extracted)"
                ));
            };
            let handle = row["handleType"]
                .as_str()
                .ok_or_else(|| format!("fixture row {id} carries no handleType"))?;
            put_sound_of.insert(id as i32, sound);
            handle_of.insert(id as i32, handle.to_owned());
        }
        let entries = sounds["entries"]
            .as_object()
            .ok_or("the put sound table has no entries")?;
        let mut cue_of = HashMap::with_capacity(entries.len());
        for (key, row) in entries {
            let id = row["id"]
                .as_i64()
                .ok_or_else(|| format!("put sound row {key} has no id"))?;
            let cue = row["assetbundleName"]
                .as_str()
                .ok_or_else(|| format!("put sound row {key} has no assetbundleName"))?;
            cue_of.insert(id, cue.to_owned());
        }
        Ok(Self {
            put_sound_of,
            handle_of,
            cue_of,
        })
    }
}

enum Tables {
    Requested {
        fixtures: Handle<JsonAsset>,
        sounds: Handle<JsonAsset>,
    },
    Read(Result<SoundTables, String>),
}

enum Instance {
    NotCreated,
    Loading {
        glb: Handle<Gltf>,
        doc: Handle<JsonAsset>,
        since: f32,
    },
    Spawned {
        root: Entity,
        doc: Arc<Value>,
        /// When its systems were handed to the particle host.
        planned: Option<f32>,
        /// Its source nodes are held inactive (never played yet).
        held: bool,
        since: f32,
    },
    Failed(String),
}

struct Put {
    uid: String,
    fixture_id: i32,
    position: Vec3,
    size: Vector3Int,
    source: &'static str,
    at: f32,
}

struct Sample {
    root: Entity,
    put: u32,
    elapsed: f32,
    next: f32,
}

/// The one `FixturePutEffect` and the master rows `PlayPutSound` reads.
#[derive(Resource)]
pub(crate) struct FixturePutEffect {
    tables: Tables,
    instance: Instance,
    queue: Vec<Put>,
    puts: u32,
    sample: Option<Sample>,
}

pub(super) fn request_tables(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(FixturePutEffect {
        tables: Tables::Requested {
            fixtures: server.load(FIXTURES),
            sounds: server.load(PUT_SOUNDS),
        },
        instance: Instance::NotCreated,
        queue: Vec::new(),
        puts: 0,
        sample: None,
    });
}

fn json_state(server: &AssetServer, handle: &Handle<JsonAsset>) -> Option<bool> {
    match server.load_state(handle) {
        bevy::asset::LoadState::Loaded => Some(true),
        bevy::asset::LoadState::Failed(_) => Some(false),
        _ => None,
    }
}

fn parse_json(world: &World, handle: &Handle<JsonAsset>, path: &str) -> Result<Value, String> {
    let text = &world
        .resource::<Assets<JsonAsset>>()
        .get(handle)
        .ok_or_else(|| format!("{path}: loaded but not in the assets"))?
        .0;
    serde_json::from_str(text).map_err(|error| format!("{path}: not JSON: {error}"))
}

impl FixturePutEffect {
    fn read_tables(&mut self, world: &World) {
        let Tables::Requested { fixtures, sounds } = &self.tables else {
            return;
        };
        let server = world.resource::<AssetServer>();
        let (Some(fixtures_ok), Some(sounds_ok)) =
            (json_state(server, fixtures), json_state(server, sounds))
        else {
            return;
        };
        let read = if !fixtures_ok {
            Err(format!("{FIXTURES} failed to load"))
        } else if !sounds_ok {
            Err(format!("{PUT_SOUNDS} failed to load"))
        } else {
            parse_json(world, fixtures, FIXTURES).and_then(|fixtures| {
                parse_json(world, sounds, PUT_SOUNDS)
                    .and_then(|sounds| SoundTables::parse(&fixtures, &sounds))
            })
        };
        self.tables = Tables::Read(read);
    }

    /// `PlayPutSound(putSoundId)` for a fixture's master row.
    fn play_sound(&self, world: &mut World, fixture_id: i32, source: &'static str) {
        let tables = match &self.tables {
            Tables::Read(Ok(tables)) => tables,
            Tables::Read(Err(error)) => {
                warn!("[fixture-put] PlayPutSound for fixture {fixture_id} ({source}): {error}; no sound");
                return;
            }
            Tables::Requested { .. } => {
                warn!("[fixture-put] PlayPutSound for fixture {fixture_id} ({source}): the fixture and put sound tables are still loading; no sound");
                return;
            }
        };
        let Some(&id) = tables.put_sound_of.get(&fixture_id) else {
            warn!("[fixture-put] PlayPutSound: fixture {fixture_id} ({source}) has no master row; no sound");
            return;
        };
        let Some(cue) = tables.cue_of.get(&id) else {
            // GetMysekaiFixturePutSound returns nothing: the source plays nothing.
            info!("[fixture-put] PlayPutSound: fixture {fixture_id} put sound id {id} has no put sound row; the source plays nothing");
            return;
        };
        info!("[fixture-put] PlayPutSound({id}) for fixture {fixture_id} ({source}): SE one-shot {cue}");
        if let Some(mut se) = world.get_resource_mut::<SeRequests>() {
            se.0.push(SeRequest {
                owner: None,
                cue: cue.clone(),
                class: SeClass::Ingame,
                source,
            });
        }
    }
}

/// `FixtureController.ShowPutEffect` for a ground fixture at its decided
/// pose. The effect and its sound follow once the instance is ready.
pub(super) fn show(world: &mut World, item: &EditableFixture, source: &'static str) {
    let pose = match item.pose() {
        Ok(pose) => pose,
        Err(error) => {
            warn!(
                "[fixture-put] ShowPutEffect for {} ({source}): no pose ({error}); not shown",
                item.uid
            );
            return;
        }
    };
    let now = world.get_resource::<Time>().map_or(0.0, Time::elapsed_secs);
    let size = current_grid_size(item.grid_size, item.direction);
    // `FixtureView.Position`: the site view's position plus the field
    // position of the footprint.
    let position = site_origin(world) + pose.translation;
    let Some(mut effect) = world.get_resource_mut::<FixturePutEffect>() else {
        warn!(
            "[fixture-put] ShowPutEffect for {} ({source}): the put effect owner is not installed",
            item.uid
        );
        return;
    };
    info!(
        "[fixture-put] ShowPutEffect({source}) {} fixture {}: master grid ({}, {}, {}) direction {:?} -> current grid ({}, {}, {}); view position ({:.3}, {:.3}, {:.3})",
        item.uid,
        item.fixture_id,
        item.grid_size.x,
        item.grid_size.y,
        item.grid_size.z,
        item.direction,
        size.x,
        size.y,
        size.z,
        position.x,
        position.y,
        position.z
    );
    effect.queue.push(Put {
        uid: item.uid.clone(),
        fixture_id: item.fixture_id,
        position,
        size,
        source,
        at: now,
    });
}

/// The site view's world position (the scene wrapper that carries the site's
/// coordinate origin). Zero when no site origin exists.
pub(crate) fn site_origin(world: &mut World) -> Vec3 {
    let mut origins = world.query_filtered::<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>();
    origins
        .iter(world)
        .next()
        .map_or(Vec3::ZERO, GlobalTransform::translation)
}

/// `FixtureController.IsBlock` (handle type block or block_transparent) of a
/// fixture's master row; `None` while the table is not read or has no row.
pub(crate) fn is_block(world: &World, fixture_id: i32) -> Option<bool> {
    let effect = world.get_resource::<FixturePutEffect>()?;
    let Tables::Read(Ok(tables)) = &effect.tables else {
        return None;
    };
    let handle = tables.handle_of.get(&fixture_id)?;
    Some(matches!(handle.as_str(), "block" | "block_transparent"))
}

/// `FixtureController.PlayPutSound`: the put sound alone.
pub(super) fn play_put_sound(world: &mut World, fixture_id: i32, source: &'static str) {
    if !world.contains_resource::<FixturePutEffect>() {
        warn!("[fixture-put] PlayPutSound for fixture {fixture_id} ({source}): the put effect owner is not installed; no sound");
        return;
    }
    world.resource_scope(|world, effect: Mut<FixturePutEffect>| {
        effect.play_sound(world, fixture_id, source);
    });
}

fn sample_enabled() -> bool {
    static SAMPLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SAMPLE.get_or_init(|| {
        std::env::var("MOLY_EDIT_AUTOPLAY").is_ok()
            || std::env::var("MOLY_EFFECT_SAMPLE").is_ok_and(|value| value == "1")
    })
}

/// Exclusive: read the tables, create and prepare the instance on the first
/// put, then set it and play the sound for each waiting put.
pub(super) fn advance(world: &mut World) {
    if !world.contains_resource::<FixturePutEffect>() {
        return;
    }
    world.resource_scope(|world, mut effect: Mut<FixturePutEffect>| {
        effect.read_tables(world);
        effect.advance_instance(world);
        effect.apply_puts(world);
        effect.sample(world);
    });
}

impl FixturePutEffect {
    fn advance_instance(&mut self, world: &mut World) {
        let now = world.resource::<Time>().elapsed_secs();
        match &mut self.instance {
            Instance::NotCreated => {
                if self.queue.is_empty() {
                    return;
                }
                // MysekaiFixtureUtility.ShowFieldPutEffect: Create on the
                // first put; SetupPutEffect loads the package once.
                let server = world.resource::<AssetServer>();
                let glb = server.load(bevy::asset::AssetPath::from(PREFAB_GLB));
                let doc = server.load(bevy::asset::AssetPath::from(particle_doc_path()));
                info!("[fixture-put] FixturePutEffect created at the first put; SetupPutEffect loads {PACKAGE} (prefab {PREFAB}) once");
                self.instance = Instance::Loading {
                    glb,
                    doc,
                    since: now,
                };
            }
            Instance::Loading { glb, doc, since } => {
                let server = world.resource::<AssetServer>().clone();
                if server.load_state(&*glb).is_failed()
                    || server.recursive_dependency_load_state(&*glb).is_failed()
                    || server.load_state(&*doc).is_failed()
                {
                    let reason = format!(
                        "prefab {PREFAB_GLB} or particle document {} failed to load",
                        particle_doc_path()
                    );
                    warn!("[fixture-put] {reason}: the put effect is not shown and its sound is not played");
                    self.instance = Instance::Failed(reason);
                    return;
                }
                if !(server.is_loaded_with_dependencies(&*glb)
                    && server.load_state(&*doc).is_loaded())
                {
                    return;
                }
                let scene = world
                    .resource::<Assets<Gltf>>()
                    .get(&*glb)
                    .and_then(|gltf| gltf.default_scene.clone());
                let parsed = parse_json(world, doc, &particle_doc_path());
                let (scene, parsed) = match (scene, parsed) {
                    (Some(scene), Ok(parsed)) => (scene, parsed),
                    (None, _) => {
                        let reason = format!("prefab {PREFAB_GLB} has no default scene");
                        warn!("[fixture-put] {reason}: not shown");
                        self.instance = Instance::Failed(reason);
                        return;
                    }
                    (_, Err(error)) => {
                        warn!("[fixture-put] {error}: not shown");
                        self.instance = Instance::Failed(error);
                        return;
                    }
                };
                let loaded_after = now - *since;
                let root = world
                    .spawn((
                        SceneRoot(scene),
                        Transform::IDENTITY,
                        // Hidden until its source nodes are held inactive.
                        Visibility::Hidden,
                        crate::site_move::PendingInstance,
                        Name::new(format!("fixture put effect {PREFAB}")),
                    ))
                    .id();
                info!("[fixture-put] SetupPutEffect: {PREFAB} loaded {loaded_after:.2}s after the first put; one instance {root:?}");
                self.instance = Instance::Spawned {
                    root,
                    doc: Arc::new(parsed),
                    planned: None,
                    held: false,
                    since: now,
                };
            }
            Instance::Spawned {
                root,
                doc,
                planned,
                held,
                since,
            } => {
                let root = *root;
                if planned.is_some() || world.get::<crate::site_move::InstanceReady>(root).is_none()
                {
                    return;
                }
                let nodes = crate::site_move::effects::set_source_nodes_active(world, root, false);
                if let Some(mut visibility) = world.get_mut::<Visibility>(root) {
                    *visibility = Visibility::Inherited;
                }
                *held = nodes > 0;
                let paths = crate::site_move::node_paths(world, root);
                let anchors: HashMap<String, Vec<Entity>> = paths
                    .iter()
                    .map(|(path, list)| (format!("/{path}"), list.clone()))
                    .collect();
                let server = world.resource::<AssetServer>().clone();
                let doc = doc.clone();
                let mut commands = world.commands();
                crate::weather_fx::fixture::plan(&mut commands, root, &doc, &anchors, &server);
                world.flush();
                *planned = Some(now);
                info!(
                    "[fixture-put] instance {root:?} ready {:.2}s after it was made: {nodes} source nodes held inactive, play-on-awake systems planned",
                    now - *since
                );
            }
            Instance::Failed(_) => {}
        }
    }

    fn apply_puts(&mut self, world: &mut World) {
        if self.queue.is_empty() {
            return;
        }
        let now = world.resource::<Time>().elapsed_secs();
        let (root, first) = match &mut self.instance {
            Instance::Spawned {
                root,
                planned: Some(planned),
                held,
                ..
            } => {
                // SetupPutEffect is awaited before SetEffect: the first put
                // waits until the host has installed (or refused) every
                // system it accepted.
                let preparing = world
                    .get::<crate::weather_fx::fixture::Request>(*root)
                    .is_some();
                if preparing && now - *planned < PREPARE_WAIT {
                    return;
                }
                if preparing {
                    warn!(
                        "[fixture-put] instance {root:?}: particle systems still preparing {PREPARE_WAIT} s after they were planned; the put goes ahead without them"
                    );
                } else if *held {
                    info!(
                        "[fixture-put] instance {root:?}: particle systems prepared {:.2}s after they were planned",
                        now - *planned
                    );
                }
                (*root, std::mem::take(held))
            }
            Instance::Failed(reason) => {
                for put in self.queue.drain(..) {
                    warn!(
                        "[fixture-put] put {} ({}) not shown and its sound not played: {reason}",
                        put.uid, put.source
                    );
                }
                return;
            }
            _ => return,
        };
        let puts = std::mem::take(&mut self.queue);
        let count = puts.len();
        for (index, put) in puts.into_iter().enumerate() {
            // SetEffect: SetActive(false), position, local scale, rotation,
            // SetActive(true).
            let scale = effect_scale(put.size);
            if let Some(mut transform) = world.get_mut::<Transform>(root) {
                *transform = Transform {
                    translation: put.position,
                    rotation: Quat::IDENTITY,
                    scale,
                };
            }
            let restart = if first && index == 0 {
                let nodes = crate::site_move::effects::set_source_nodes_active(world, root, true);
                format!("first activation, {nodes} source nodes active")
            } else {
                match crate::weather_fx::fixture::play(world, root) {
                    Ok(systems) => format!("reactivated, {systems} systems played again"),
                    Err(error) => format!("reactivated, Play refused: {error}"),
                }
            };
            self.puts += 1;
            info!(
                "[fixture-put] SetEffect put {} ({} {}, {:.2}s after ShowPutEffect): instance {root:?} position ({:.3}, {:.3}, {:.3}) scale ({:.3}, {:.3}, {:.3}) rotation identity; grid ({}, {}, {}); {restart}",
                self.puts,
                put.source,
                put.uid,
                now - put.at,
                put.position.x,
                put.position.y,
                put.position.z,
                scale.x,
                scale.y,
                scale.z,
                put.size.x,
                put.size.y,
                put.size.z
            );
            self.play_sound(world, put.fixture_id, put.source);
            if index + 1 == count && sample_enabled() {
                self.sample = Some(Sample {
                    root,
                    put: self.puts,
                    elapsed: 0.0,
                    next: 0.0,
                });
            }
        }
    }

    /// Instrument (`MOLY_EDIT_AUTOPLAY` or `MOLY_EFFECT_SAMPLE=1`): every
    /// 0.25 s after the last put, each system's live particles.
    fn sample(&mut self, world: &World) {
        let Some(sample) = self.sample.as_mut() else {
            return;
        };
        sample.elapsed += world.resource::<Time>().delta_secs();
        if sample.elapsed < sample.next {
            return;
        }
        sample.next += SAMPLE_STEP;
        let now = world.resource::<Time>().elapsed_secs_f64();
        let mut systems = Vec::new();
        let mut total = 0;
        if let Some(children) = world.get::<Children>(sample.root) {
            for child in children.iter() {
                let Some(system) = world.get::<crate::uber_particle::FixtureParticleLive>(child)
                else {
                    continue;
                };
                let playing = world
                    .get::<crate::weather_fx::fixture::Played>(child)
                    .is_some_and(|played| played.playing(&system.0, now));
                total += system.0.pool.len();
                systems.push(format!(
                    "{} live {} born {}{}",
                    system.0.node,
                    system.0.pool.len(),
                    system.0.born_total,
                    if playing { " playing" } else { "" }
                ));
            }
        }
        info!(
            "[fixture-put] sample put {} t {:.2}s: {total} live particles in {} systems [{}]",
            sample.put,
            sample.elapsed,
            systems.len(),
            systems.join("; ")
        );
        if sample.elapsed >= SAMPLE_SECONDS {
            self.sample = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FieldUtility.ConvertGridSize`'s jump table: identity for Front and
    /// Back, x and z swapped for the two side directions.
    #[test]
    fn side_directions_swap_the_footprint_axes() {
        let size = Vector3Int::new(2, 3, 4);
        assert_eq!(current_grid_size(size, Direction::Front), size);
        assert_eq!(current_grid_size(size, Direction::Back), size);
        assert_eq!(
            current_grid_size(size, Direction::Left),
            Vector3Int::new(4, 3, 2)
        );
        assert_eq!(
            current_grid_size(size, Direction::Right),
            Vector3Int::new(4, 3, 2)
        );
    }

    /// `SetEffect` multiplies only x and z by `TILE_SCALE` (0.25); the
    /// height component is the grid height converted to float.
    #[test]
    fn the_height_component_is_not_scaled() {
        assert_eq!(
            effect_scale(Vector3Int::new(2, 3, 4)),
            Vec3::new(0.5, 3.0, 1.0)
        );
    }
}
