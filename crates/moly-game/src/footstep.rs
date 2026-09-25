//! The player's footsteps (`PlayerAvatarPresenter.OnFootSEAnimationEvent`).
//!
//! Source chain, once per `PublishFootSE` animation event of the player
//! avatar (the component that publishes it is added only to the player;
//! NPCs have no footsteps). Of the avatar's motions only the walk clip the
//! Move state plays and the run clip the Dash state plays carry the event:
//!
//! 1. Nothing happens unless the avatar is visible and is the local player,
//!    and the current site's controller holds a `FootSEController` (the
//!    home, room and harvest site controllers build one; the delivery site
//!    controller does not, so the delivery site has no footsteps).
//! 2. The Dash state (23) selects the run cue, any other state the walk cue.
//! 3. On a housing site (category home or room) the fixture under the foot
//!    (`GetGroundFixture`, see [`ground`]) plays its fixture footstep when
//!    there is one; otherwise, and on every other site, the ground under the
//!    foot plays (`PlayFootGroundSe`, see [`surface`]).
//! 4. `IsWater(position)` is read, `EffectManager.Stop("PlayerFootEffect")`
//!    stops the previous foot effects, and then: on water, a ray cast down
//!    from three metres above the player (see [`raycast`]) places the water
//!    splash (type 17) at the hit height; otherwise, in the Dash state, the
//!    dash smoke (type 14) follows the player (see [`effects`]).
//!
//! Leaving the Dash state stops the dash smoke (`PlayerAvatarDashState`'s
//! Dispose), and the cannon move's pre-action stops the key's effects.
//!
//! The site's sound mesh is captured when its controller is built: the
//! world positions of its vertices, through the scene root, once per site
//! load. Every step writes one log line: clip and phase, state, position,
//! nearest colour and master row (or fixture and footstep row), cue and the
//! water test.

pub(crate) mod effects;
mod ground;
mod phase;
mod raycast;
mod surface;

use bevy::animation::graph::AnimationNodeIndex;
use bevy::app::AnimationSystems;
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::residency::{load_gltf, GltfResidency};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site_move::effects::EffectType;
use effects::FootEffects;
use ground::{FixtureRows, GroundFixture, Placed};
use phase::FootPhases;
use surface::{Masters, SoundMeshSource, SoundWave};

/// Height above the player the water ray starts from, and its length.
const WATER_RAY: f32 = 3.0;

#[derive(Resource)]
struct FootInputs {
    manifest: Handle<JsonAsset>,
    site_masters: Handle<JsonAsset>,
    fixture_masters: Handle<JsonAsset>,
    player_data: Handle<JsonAsset>,
    sound_wave: Handle<JsonAsset>,
    site_index: Handle<JsonAsset>,
    phases: Option<Result<FootPhases, String>>,
    masters: Option<Result<Masters, String>>,
    fixtures: Option<Result<FixtureRows, String>>,
    warned: HashSet<String>,
}

impl FootInputs {
    /// WARN once per distinct reason.
    fn warn_once(&mut self, reason: String) {
        if self.warned.insert(reason.clone()) {
            warn!("[footstep] {reason}");
        }
    }
}

fn request_inputs(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(FootInputs {
        manifest: server.load(phase::MANIFEST),
        site_masters: server.load(surface::SITE_FOOTSTEPS),
        fixture_masters: server.load(surface::FIXTURE_FOOTSTEPS),
        player_data: server.load(ground::PLAYER_DATA),
        sound_wave: server.load(surface::SOUND_WAVE),
        site_index: server.load("moly://site/index.json"),
        phases: None,
        masters: None,
        fixtures: None,
        warned: HashSet::new(),
    });
}

/// The document behind a handle: None while loading, Err when it failed.
fn document(
    server: &AssetServer,
    docs: &Assets<JsonAsset>,
    handle: &Handle<JsonAsset>,
    label: &str,
) -> Option<Result<Value, String>> {
    if server.load_state(handle).is_failed() {
        return Some(Err(format!(
            "{label} failed to load (not in the release root?)"
        )));
    }
    let json = docs.get(handle)?;
    Some(serde_json::from_str(&json.0).map_err(|error| format!("{label} is not JSON: {error}")))
}

/// Update: parse the tables once they have loaded.
fn parse_inputs(
    server: Res<AssetServer>,
    docs: Res<Assets<JsonAsset>>,
    inputs: Option<ResMut<FootInputs>>,
) {
    let Some(mut inputs) = inputs else { return };
    if inputs.phases.is_none() {
        if let Some(doc) = document(&server, &docs, &inputs.manifest, phase::MANIFEST) {
            let parsed = doc.and_then(|doc| FootPhases::parse(&doc));
            match &parsed {
                Ok(phases) => info!(
                    "[footstep] event phases from the avatar clips: walk {:?} of {}, run {:?} of {} (fractions of the SD clip playing: product adaptation)",
                    phases.walk,
                    phase::WALK_CLIP,
                    phases.run,
                    phase::RUN_CLIP
                ),
                Err(error) => warn!("[footstep] no footsteps: {error}"),
            }
            inputs.phases = Some(parsed);
        }
    }
    if inputs.masters.is_none() {
        let site = document(
            &server,
            &docs,
            &inputs.site_masters,
            surface::SITE_FOOTSTEPS,
        );
        let fixture = document(
            &server,
            &docs,
            &inputs.fixture_masters,
            surface::FIXTURE_FOOTSTEPS,
        );
        if let (Some(site), Some(fixture)) = (site, fixture) {
            let parsed =
                site.and_then(|site| fixture.and_then(|fixture| Masters::parse(&site, &fixture)));
            match &parsed {
                Ok(masters) => info!(
                    "[footstep] footstep masters: {} site rows, {} fixture rows",
                    masters.site.len(),
                    masters.fixture.len()
                ),
                Err(error) => warn!("[footstep] no footsteps: {error}"),
            }
            inputs.masters = Some(parsed);
        }
    }
    if inputs.fixtures.is_none() {
        if let Some(doc) = document(&server, &docs, &inputs.player_data, ground::PLAYER_DATA) {
            let parsed = doc.and_then(|doc| FixtureRows::parse(&doc));
            match &parsed {
                Ok(rows) if !rows.column => warn!(
                    "[footstep] {} carries no mysekaiFixtureFootstepId column: a footstep on a fixture is silent (the fixture branch is skipped)",
                    ground::PLAYER_DATA
                ),
                Ok(rows) => info!(
                    "[footstep] fixture footstep ids: {} of {} fixture rows carry one",
                    rows.rows.values().filter(|row| row.footstep.is_some()).count(),
                    rows.rows.len()
                ),
                Err(error) => warn!("[footstep] fixture footsteps unavailable: {error}"),
            }
            inputs.fixtures = Some(parsed);
        }
    }
}

/// The current site's `FootSEController` slot.
#[derive(Resource, Default)]
struct FootSite {
    epoch: u64,
    site_type: String,
    controller: Option<SiteController>,
}

enum SiteController {
    /// No controller: the reason.
    Absent(String),
    Pending(PendingController),
    Ready(Controller),
}

struct PendingController {
    source: SoundMeshSource,
    mesh: Handle<Gltf>,
    colliders: Vec<(String, Handle<Gltf>)>,
    fixture_branch: bool,
    census: HashMap<[u32; 4], usize>,
}

struct Controller {
    surface: SoundWave,
    water: [i32; 3],
    fixture_branch: bool,
    colliders: raycast::Colliders,
}

/// Update: build the site's controller when a site has loaded.
fn build_site(world: &mut World) {
    let Some(active) = world.get_resource::<crate::site::SiteActive>().cloned() else {
        return;
    };
    let Some(epoch) = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|e| e.0)
    else {
        return;
    };
    world.init_resource::<FootSite>();
    let current = {
        let site = world.resource::<FootSite>();
        site.epoch == epoch && site.site_type == active.site_type && site.controller.is_some()
    };
    if !current {
        let Some(controller) = start_controller(world, &active) else {
            return;
        };
        let mut site = world.resource_mut::<FootSite>();
        site.epoch = epoch;
        site.site_type = active.site_type.clone();
        if let SiteController::Absent(reason) = &controller {
            info!(
                "[footstep] site {} ({}): no FootSEController: {reason}",
                active.site_type, active.scene
            );
        }
        site.controller = Some(controller);
    }
    let pending = matches!(
        world.resource::<FootSite>().controller,
        Some(SiteController::Pending(_))
    );
    if pending {
        let Some(SiteController::Pending(pending)) =
            world.resource_mut::<FootSite>().controller.take()
        else {
            unreachable!()
        };
        let next = finish_controller(world, &active, pending);
        world.resource_mut::<FootSite>().controller = Some(next);
    }
}

/// The controller the site's controller builds, or None to wait for
/// inputs.
fn start_controller(world: &mut World, active: &crate::site::SiteActive) -> Option<SiteController> {
    let fixture_branch = match active.category.as_str() {
        "housing_home" | "housing_room" => true,
        "harvest" => false,
        "delivery" => {
            return Some(SiteController::Absent(
                "the delivery site controller builds none, so footsteps are silent here".into(),
            ))
        }
        other => {
            warn!(
                "[footstep] site {}: unknown category {other}: no footsteps",
                active.site_type
            );
            return Some(SiteController::Absent(format!("unknown category {other}")));
        }
    };
    let server = world.resource::<AssetServer>().clone();
    let docs = world.resource::<Assets<JsonAsset>>();
    let inputs = world.resource::<FootInputs>();
    let sound_wave = document(&server, docs, &inputs.sound_wave, surface::SOUND_WAVE)?;
    let index = document(&server, docs, &inputs.site_index, "site index")?;
    let (sound_wave, index) = match (sound_wave, index) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(error), _) | (_, Err(error)) => {
            // Once per missing product, not once per site load.
            world
                .resource_mut::<FootInputs>()
                .warn_once(format!("no footsteps on any site: {error}"));
            return Some(SiteController::Absent(error));
        }
    };
    let source = match surface::sound_mesh_source(&sound_wave, &index, &active.scene) {
        Ok(source) => source,
        Err(error) => {
            warn!(
                "[footstep] site {}: no footsteps: {error}",
                active.site_type
            );
            return Some(SiteController::Absent(error));
        }
    };
    let census = sound_wave["scenes"][active.scene.as_str()]["mesh"]["colours"]
        .as_array()
        .map(|colours| {
            colours
                .iter()
                .filter_map(|c| {
                    let n: Vec<f32> = c["normalized"]
                        .as_array()?
                        .iter()
                        .map(|v| v.as_f64().map(|v| v as f32))
                        .collect::<Option<_>>()?;
                    let key: [f32; 4] = n.try_into().ok()?;
                    Some((key.map(f32::to_bits), c["vertices"].as_u64()? as usize))
                })
                .collect()
        })
        .unwrap_or_default();
    let colliders = index["scenes"][active.scene.as_str()]["collision"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["file"].as_str())
                .map(|file| {
                    (
                        file.to_owned(),
                        load_gltf(
                            &server,
                            format!("moly://site/{file}"),
                            GltfResidency::GpuTextures,
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mesh = load_gltf(&server, source.glb.clone(), GltfResidency::GpuTextures);
    Some(SiteController::Pending(PendingController {
        source,
        mesh,
        colliders,
        fixture_branch,
        census,
    }))
}

/// Finish a pending controller once its glbs have loaded.
fn finish_controller(
    world: &mut World,
    active: &crate::site::SiteActive,
    pending: PendingController,
) -> SiteController {
    let server = world.resource::<AssetServer>().clone();
    let state = |handle: &Handle<Gltf>| -> Option<Result<(), String>> {
        if server.load_state(handle).is_failed()
            || server.recursive_dependency_load_state(handle).is_failed()
        {
            return Some(Err(format!("{:?} failed to load", handle.path())));
        }
        server.is_loaded_with_dependencies(handle).then_some(Ok(()))
    };
    let mut all = vec![state(&pending.mesh)];
    all.extend(pending.colliders.iter().map(|(_, handle)| state(handle)));
    if all.iter().any(Option::is_none) {
        return SiteController::Pending(pending);
    }
    if let Some(Some(Err(error))) = all.into_iter().find(|s| matches!(s, Some(Err(_)))) {
        warn!(
            "[footstep] site {}: no FootSEController: {error}",
            active.site_type
        );
        return SiteController::Absent(error);
    }
    let masters = match &world.resource::<FootInputs>().masters {
        Some(Ok(masters)) => masters.clone(),
        Some(Err(error)) => return SiteController::Absent(error.clone()),
        None => return SiteController::Pending(pending),
    };
    let origin = {
        let mut query = world.query_filtered::<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>();
        let roots: Vec<GlobalTransform> = query.iter(world).copied().collect();
        match roots.as_slice() {
            [origin] => *origin,
            _ => return SiteController::Pending(pending),
        }
    };
    let result = (|| -> Result<Controller, String> {
        let gltfs = world.resource::<Assets<Gltf>>();
        let nodes = world.resource::<Assets<GltfNode>>();
        let gltf_meshes = world.resource::<Assets<GltfMesh>>();
        let meshes = world.resource::<Assets<Mesh>>();
        let gltf = gltfs
            .get(&pending.mesh)
            .ok_or("sound mesh glb not in assets")?;
        let handle = gltf
            .named_meshes
            .get(pending.source.mesh.as_str())
            .ok_or_else(|| format!("{} has no mesh {}", pending.source.glb, pending.source.mesh))?;
        let placement = mesh_instances(gltf, nodes)
            .into_iter()
            .find(|(mesh, _)| mesh.id() == handle.id())
            .map(|(_, affine)| affine);
        let gltf_mesh = gltf_meshes.get(handle).ok_or("sound mesh not in assets")?;
        let [primitive] = gltf_mesh.primitives.as_slice() else {
            return Err(format!(
                "sound mesh {} has {} primitives, not 1",
                pending.source.mesh,
                gltf_mesh.primitives.len()
            ));
        };
        let mesh = meshes
            .get(&primitive.mesh)
            .ok_or("sound mesh data not in assets")?;
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|v| v.as_float3())
            .ok_or("sound mesh has no float3 positions")?;
        let colours: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(colours)) => colours.clone(),
            _ => return Err("sound mesh has no float4 vertex colours".into()),
        };
        if positions.len() != pending.source.vertices || colours.len() != positions.len() {
            return Err(format!(
                "sound mesh has {} vertices and {} colours; the source mesh has {}",
                positions.len(),
                colours.len(),
                pending.source.vertices
            ));
        }
        let census = surface::colour_census(&colours);
        if census != pending.census {
            return Err(format!(
                "sound mesh colours {census:?} differ from the source mesh's {:?}",
                pending.census
            ));
        }
        let local = if pending.source.kit {
            // The kit geometry holds the room kit's meshes, not the room
            // scene's placement node; with a single colour every vertex
            // answers the same, so the positions do not matter.
            if census.len() != 1 {
                return Err(
                    "room sound mesh has several colours but its placement in the room is not read"
                        .into(),
                );
            }
            bevy::math::Affine3A::IDENTITY
        } else {
            placement.ok_or("no node of the sound mesh glb places the mesh")?
        };
        let to_world = origin.affine() * local;
        let world_vertices = positions
            .iter()
            .map(|p| Vec3::from(to_world.transform_point3(Vec3::from(*p))))
            .collect();
        let water = masters.water()?;
        let mut triangles = Vec::new();
        for (file, handle) in &pending.colliders {
            let gltf = gltfs
                .get(handle)
                .ok_or_else(|| format!("{file} not in assets"))?;
            for (mesh, affine) in mesh_instances(gltf, nodes) {
                let to_world = origin.affine() * affine;
                let gltf_mesh = gltf_meshes
                    .get(&mesh)
                    .ok_or_else(|| format!("{file}: mesh not in assets"))?;
                for primitive in &gltf_mesh.primitives {
                    let mesh = meshes
                        .get(&primitive.mesh)
                        .ok_or_else(|| format!("{file}: mesh data not in assets"))?;
                    let positions = mesh
                        .attribute(Mesh::ATTRIBUTE_POSITION)
                        .and_then(|v| v.as_float3())
                        .ok_or_else(|| format!("{file}: no float3 positions"))?;
                    let world: Vec<Vec3> = positions
                        .iter()
                        .map(|p| Vec3::from(to_world.transform_point3(Vec3::from(*p))))
                        .collect();
                    let indices: Vec<usize> = match mesh.indices() {
                        Some(indices) => indices.iter().collect(),
                        None => (0..world.len()).collect(),
                    };
                    for tri in indices.chunks_exact(3) {
                        triangles.push([world[tri[0]], world[tri[1]], world[tri[2]]]);
                    }
                }
            }
        }
        Ok(Controller {
            surface: SoundWave {
                world: world_vertices,
                colours,
            },
            water,
            fixture_branch: pending.fixture_branch,
            colliders: raycast::Colliders { triangles },
        })
    })();
    match result {
        Ok(controller) => {
            let mut census: Vec<String> = surface::colour_census(&controller.surface.colours)
                .into_iter()
                .map(|(bits, n)| {
                    let c = bits.map(f32::from_bits);
                    let b = surface::colour_bytes(c);
                    format!("({},{},{})x{n}", b[0], b[1], b[2])
                })
                .collect();
            census.sort();
            info!(
                "[footstep] site {} ({}): FootSEController built: sound mesh {} from {} ({} vertices, colours {}), water colour {:?}, fixture branch {}, {} collider triangles from {:?}",
                active.site_type,
                active.scene,
                pending.source.mesh,
                pending.source.glb,
                controller.surface.world.len(),
                census.join(" "),
                controller.water,
                if controller.fixture_branch { "on" } else { "off" },
                controller.colliders.triangles.len(),
                pending.colliders.iter().map(|(file, _)| file.as_str()).collect::<Vec<_>>()
            );
            SiteController::Ready(controller)
        }
        Err(error) => {
            warn!(
                "[footstep] site {}: no FootSEController: {error}",
                active.site_type
            );
            SiteController::Absent(error)
        }
    }
}

/// Every mesh a glb's nodes place, with the node's transform through its
/// ancestors.
fn mesh_instances(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
) -> Vec<(Handle<GltfMesh>, bevy::math::Affine3A)> {
    let mut parent: HashMap<AssetId<GltfNode>, AssetId<GltfNode>> = HashMap::new();
    for handle in &gltf.nodes {
        if let Some(node) = nodes.get(handle) {
            for child in &node.children {
                parent.insert(child.id(), handle.id());
            }
        }
    }
    let mut out = Vec::new();
    for handle in &gltf.nodes {
        let Some(node) = nodes.get(handle) else {
            continue;
        };
        let Some(mesh) = node.mesh.clone() else {
            continue;
        };
        let mut affine = node.transform.compute_affine();
        let mut at = handle.id();
        while let Some(up) = parent.get(&at) {
            if let Some(up_node) = nodes.get(*up) {
                affine = up_node.transform.compute_affine() * affine;
            }
            at = *up;
        }
        out.push((mesh, affine));
    }
    out
}

/// One `PublishFootSE` crossing.
struct FootEvent {
    clip: String,
    index: usize,
    count: usize,
    fraction: f32,
    cycle: f32,
    player: Entity,
    pose: Transform,
    visible: bool,
    state: PlayerActionState,
}

#[derive(Resource, Default)]
struct FootEvents {
    events: Vec<FootEvent>,
    dash_disposed: bool,
}

#[derive(Default)]
struct FootClock {
    node: Option<AnimationNodeIndex>,
    cycle: f32,
    state: Option<PlayerActionState>,
}

/// PostUpdate, after the animation step: the events the locomotion clip
/// crossed this frame.
#[allow(clippy::too_many_arguments)]
fn detect_events(
    inputs: Option<Res<FootInputs>>,
    states: Option<Res<PlayerAvatarStates>>,
    players: Query<
        (
            Entity,
            &crate::player_avatar::AvatarDriver,
            &Transform,
            Option<&Visibility>,
        ),
        With<crate::player::PlayerControlled>,
    >,
    animators: Query<&AnimationPlayer>,
    clips: Res<Assets<AnimationClip>>,
    mut events: ResMut<FootEvents>,
    mut clock: Local<FootClock>,
) {
    let (Some(inputs), Some(states)) = (inputs, states) else {
        return;
    };
    let state = states.current;
    if clock.state.replace(state) == Some(PlayerActionState::Dash)
        && state != PlayerActionState::Dash
    {
        events.dash_disposed = true;
    }
    let Some(Ok(phases)) = &inputs.phases else {
        return;
    };
    let Ok((player, driver, transform, visibility)) = players.single() else {
        clock.node = None;
        return;
    };
    // Only the Move and Dash states play the avatar clips that carry the
    // events (the walk and the run clip); the auto-move walk and every other
    // avatar motion carry none. A locomotion clip playing in another state
    // (a product walk toward a fixture, for one) therefore has no footsteps.
    let gait = driver
        .gait_clip()
        .filter(|_| matches!(state, PlayerActionState::Move | PlayerActionState::Dash));
    let Some((run, node, clip)) = gait else {
        clock.node = None;
        return;
    };
    let Some(active) = animators
        .get(driver.player)
        .ok()
        .and_then(|a| a.animation(node))
    else {
        clock.node = None;
        return;
    };
    let Some(duration) = driver.sd_clip_duration(clip, &clips).filter(|d| *d > 0.0) else {
        clock.node = None;
        return;
    };
    let cycle = active.completions() as f32 + active.seek_time() / duration;
    // A node that just started (or restarted) plays from time 0.
    let previous = if clock.node == Some(node) && cycle >= clock.cycle {
        clock.cycle
    } else {
        0.0
    };
    clock.node = Some(node);
    clock.cycle = cycle;
    let fractions = if run { &phases.run } else { &phases.walk };
    for (index, fraction) in phase::crossed(fractions, previous, cycle) {
        events.events.push(FootEvent {
            clip: clip.to_owned(),
            index,
            count: fractions.len(),
            fraction,
            cycle,
            player,
            pose: *transform,
            visible: !matches!(visibility, Some(Visibility::Hidden)),
            state,
        });
    }
}

/// PostUpdate: play the crossed events (`OnFootSEAnimationEvent`).
fn play_events(world: &mut World) {
    let (events, disposed) = {
        let mut queue = world.resource_mut::<FootEvents>();
        (
            std::mem::take(&mut queue.events),
            std::mem::take(&mut queue.dash_disposed),
        )
    };
    if disposed {
        stop_type(world, EffectType::Dash, "PlayerAvatarDashState.Dispose");
    }
    for event in events {
        footstep(world, event);
    }
}

fn footstep(world: &mut World, event: FootEvent) {
    let position = event.pose.translation;
    let head = format!(
        "[footstep] {} event {}/{} at phase {:.3} (cycle {:.3}) state {:?} pos ({:.3},{:.3},{:.3})",
        event.clip,
        event.index + 1,
        event.count,
        event.fraction,
        event.cycle,
        event.state,
        position.x,
        position.y,
        position.z
    );
    if !event.visible {
        info!("{head}: avatar not visible, nothing plays");
        return;
    }
    let dash = event.state == PlayerActionState::Dash;
    world.init_resource::<FootSite>();
    let site = world.resource::<FootSite>();
    let fixture_branch = match &site.controller {
        Some(SiteController::Ready(controller)) => controller.fixture_branch,
        Some(SiteController::Absent(reason)) => {
            info!(
                "{head}: site {} has no FootSEController ({reason}), nothing plays",
                site.site_type
            );
            return;
        }
        Some(SiteController::Pending(_)) | None => {
            info!("{head}: the site's FootSEController is not built yet, nothing plays");
            return;
        }
    };
    let (Some(Ok(masters)), fixtures) = (
        world.resource::<FootInputs>().masters.clone(),
        world.resource::<FootInputs>().fixtures.clone(),
    ) else {
        info!("{head}: footstep masters unavailable, nothing plays");
        return;
    };
    // 3. Fixture or ground.
    let (decision, fixture_note) = if fixture_branch {
        match ground_fixture(world, position, fixtures.as_ref()) {
            Ok((cell, decision)) => (
                decision,
                match cell {
                    Some(c) => format!("no fixture at cell ({},{},{}); ", c.x, c.y, c.z),
                    None => "cell outside the grid range; ".to_owned(),
                },
            ),
            Err(reason) => {
                world.resource_mut::<FootInputs>().warn_once(format!(
                    "fixture under the foot undecidable: {reason}; such footsteps are silent"
                ));
                info!("{head}: fixture under the foot undecidable ({reason}), nothing plays");
                return;
            }
        }
    } else {
        (GroundFixture::None, String::new())
    };
    let site = world.resource::<FootSite>();
    let Some(SiteController::Ready(controller)) = &site.controller else {
        unreachable!()
    };
    let (cue, what) = match decision {
        GroundFixture::None => {
            let (colour, vertex) = controller.surface.nearest_colour(position);
            let rgb = surface::colour_bytes(colour);
            let row = masters.site_row(rgb);
            let cue = surface::cue(
                dash,
                row.map(|r| r.walk.as_str()),
                row.map(|r| r.run.as_str()),
            );
            let what = format!(
                "{fixture_note}ground: nearest vertex {} colour ({},{},{}) -> site footstep row {}",
                vertex.map_or("none".into(), |v| format!("#{v}")),
                rgb[0],
                rgb[1],
                rgb[2],
                row.map_or("none".into(), |r| r.id.to_string())
            );
            (Some(cue), what)
        }
        GroundFixture::Fixture { id, pass } => {
            let rows = fixtures.as_ref().and_then(|f| f.as_ref().ok());
            match rows {
                Some(rows) if !rows.column => (
                    None,
                    format!(
                        "fixture {id} ({pass} grid): the catalogue has no footstep column, silent"
                    ),
                ),
                Some(rows) => {
                    let footstep = rows.rows.get(&id).map(|row| row.footstep.unwrap_or(0));
                    let row = footstep.and_then(|f| masters.fixture_row(f));
                    let cue = surface::cue(
                        dash,
                        row.map(|r| r.walk.as_str()),
                        row.map(|r| r.run.as_str()),
                    );
                    (
                        Some(cue),
                        format!(
                            "fixture {id} ({pass} grid): footstep id {} -> fixture footstep row {}",
                            footstep.map_or("none (no master row)".into(), |f| f.to_string()),
                            row.map_or("none".into(), |r| r.id.to_string())
                        ),
                    )
                }
                None => (
                    None,
                    format!("fixture {id} ({pass} grid): fixture catalogue unavailable, silent"),
                ),
            }
        }
    };
    let water = {
        let (colour, _) = controller.surface.nearest_colour(position);
        surface::colour_bytes(colour) == controller.water
    };
    let hit = water.then(|| {
        controller
            .colliders
            .cast_down(position + Vec3::Y * WATER_RAY, WATER_RAY)
    });
    if let Some(cue) = &cue {
        // `ExistsCueName` then `PlaySEOneShot(cue, 0)`: the audio host
        // refuses a cue that is not in its table (a bare prefix never is).
        match world.get_resource_mut::<crate::audio::SeRequests>() {
            Some(mut requests) => requests.0.push(crate::audio::SeRequest {
                owner: None,
                cue: cue.clone(),
                class: crate::audio::SeClass::Ingame,
                source: "footstep",
            }),
            None => error!("[footstep] no SE request queue: cue {cue} not played"),
        }
    }
    info!(
        "{head}: {what}; cue {}; water {water}",
        cue.as_deref().unwrap_or("none")
    );
    // 4. Foot effects.
    stop_key(world, "footstep");
    if water {
        match hit.flatten() {
            Some(point) => {
                let offset = Vec3::new(0.0, point.y - position.y, 0.0);
                emit(
                    world,
                    EffectType::WalkWater,
                    event.player,
                    event.pose,
                    offset,
                );
            }
            None => info!("[footstep] water: no collider within {WATER_RAY} m below, no splash"),
        }
    } else if dash {
        emit(
            world,
            EffectType::Dash,
            event.player,
            event.pose,
            Vec3::ZERO,
        );
    }
}

/// `GetGroundFixture` on the product's fixture inputs.
fn ground_fixture(
    world: &mut World,
    position: Vec3,
    fixtures: Option<&Result<FixtureRows, String>>,
) -> Result<(Option<moly_law::fixture::GridPosition>, GroundFixture), String> {
    let rows = match fixtures {
        Some(Ok(rows)) => rows.clone(),
        Some(Err(error)) => return Err(format!("fixture catalogue: {error}")),
        None => return Err("fixture catalogue still loading".into()),
    };
    let site_type = world.resource::<FootSite>().site_type.clone();
    let inputs = world
        .get_resource::<crate::fixture_scene_inputs::FixtureSceneSupply>()
        .and_then(|supply| supply.current().cloned())
        .ok_or("fixture scene inputs are not ready")?;
    if inputs.site_type != site_type {
        return Err(format!(
            "fixture scene inputs are for {}, not {site_type}",
            inputs.site_type
        ));
    }
    let mut query = world.query::<(
        &crate::fixture_scene_inputs::FixtureScenePlacement,
        Option<&crate::fixture_activity_state::FixtureActivityIdentity>,
    )>();
    let found: Vec<_> = query
        .iter(world)
        .map(|(placement, identity)| (placement.0.clone(), identity.map(|i| i.master_id)))
        .collect();
    let mut placed = Vec::with_capacity(found.len());
    for (row, master) in &found {
        let master_id =
            master.ok_or_else(|| format!("placed fixture {} has no master identity", row.uid))?;
        placed.push(Placed {
            uid: &row.uid,
            master_id,
            layout: row.layout,
            min: row.min,
            max: row.max,
            center_y: row.center_y,
            height: row.layout_grid_size.y,
        });
    }
    let cell = ground::grid_position(position - inputs.floor.site_origin);
    ground::ground_fixture(cell, &inputs.floor, &placed, &rows).map(|decision| (cell, decision))
}

fn with_effects(world: &mut World, f: impl FnOnce(&mut World, &mut FootEffects)) {
    if world.contains_resource::<FootEffects>() {
        world.resource_scope(|world, mut effects: Mut<FootEffects>| f(world, &mut effects));
    }
}

fn emit(world: &mut World, kind: EffectType, player: Entity, pose: Transform, offset: Vec3) {
    with_effects(world, |world, effects| {
        effects.emit(world, kind, player, pose, offset);
    });
}

fn stop_type(world: &mut World, kind: EffectType, reason: &str) {
    with_effects(world, |world, effects| {
        effects.stop_type(world, kind, reason)
    });
}

/// `EffectManager.Stop("PlayerFootEffect")`.
pub(crate) fn stop_key(world: &mut World, reason: &str) {
    with_effects(world, |world, effects| effects.stop_key(world, reason));
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<FootEvents>()
        .add_systems(Startup, (request_inputs, effects::request))
        .add_systems(Update, (parse_inputs, build_site, effects::advance).chain())
        .add_systems(
            PostUpdate,
            (detect_events, play_events, effects::follow)
                .chain()
                .after(AnimationSystems)
                .before(TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            effects::step
                .after(TransformSystems::Propagate)
                .after(crate::camera::follow_avatar),
        );
}
