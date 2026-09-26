//! `RoomController`'s door: the wall skin's door prefab, played by the door
//! moves.
//!
//! - `SetUpDoor(wallId, textureId)` loads the wall's skin bundle,
//!   instantiates its `mdl_<skin>_door_door1` prefab under the wall's
//!   `Loc_door` transform, keeps the prefab's Animator and runs
//!   `CullingDoor`. `SetupEntrance` keeps the wall's `loc_inside` locator as
//!   the door action point. `SiteManager.SetupRoom` then shows the door.
//! - `OpenDoorAsync`: `ShowDoor` (the prefab active), `Animator.Play("Open")`,
//!   one `Yield`, then it waits the clip. `CloseDoorAsync`:
//!   `Animator.Play("Close")`, one `Yield`, then `t = 0; while (t < length)
//!   { t += deltaTime; Yield }`, then `se_door_close`. The moves start both
//!   without awaiting them, so the close outlives the move.
//! - `Animator.Play` switches state at once; the controller's `Open` and
//!   `Close` states have no transitions and hold their last pose.
//! - `SetupDoorSensor`, after `SetUpDoor`: the first active Transform under
//!   the door wall named `gimmick_door` (ignoring case) becomes a
//!   `PlayerActionSensor` of radius 1 with action type `Door`, without an
//!   id. The room site's enter registers it with the collision manager and
//!   its exit removes it; the action button reads it (the go-home button).
//!
//! The release root carries the prefab in `site/skins/<skin>/`: glb scene
//! for the prefab asset, the controller's states and clips in the sidecar.
//! The clip plays like the cannon's: an animation graph with the player
//! paused and sought by the door's own clock (the clock advances from the
//! frame of `Play`, the product's convention for animator clocks).
//!
//! Named differences:
//! - `CullingDoor` (the prefab active only while the camera is behind the
//!   door's forward) runs once in `SetUpDoor` and every room entry path the
//!   product has then reaches `SetupRoom`'s `ShowDoor`; the door is spawned
//!   shown.
//! - The door texture: `SetUpDoor` sets the renderer's texture from the
//!   surface-appearance bundle named by the server config's
//!   `MyRoomDoorAppearanceAssetName` and the texture id; the product keeps
//!   the texture the skin prefab carries.
//! - The base material is `Mysekai/Fixture/Basic`, whose parameters the
//!   fixture material path reads from glTF extras the skin glb does not
//!   carry: the door is drawn with the imported base-colour material. The
//!   stencil mesh writes no colour (`_ColorMask` 0) and the shadow mesh is
//!   off (`_Show` 0): both are hidden. The door parts carry their source
//!   shader attribute ([`SourceStencilAttribute`]), and the character
//!   silhouette's stencil replay draws the stencil mesh from it.
//! - The clip's binding path is `Loc_door/root/joint_door1` (the animation
//!   file's hierarchy); under the prefab's Animator the joint is
//!   `root/joint_door1`. The release glb binds the channel to the prefab's
//!   `joint_door1`, and this port plays it there; whether the engine binds
//!   the source clip under the prefab is not settled here.

use bevy::animation::graph::AnimationNodeIndex;
use bevy::diagnostic::FrameCount;
use bevy::gltf::{Gltf, GltfMaterialName};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use super::{InstanceReady, PendingInstance};
use crate::character_silhouette::{door_part_attribute, SourceStencilAttribute};

/// `SetUpDoor`'s search: a Transform named exactly this under the wall.
const DOOR_ANCHOR: &str = "Loc_door";
/// `SetupEntrance`'s door action point.
const INSIDE: &str = "loc_inside";
/// `SetupDoorSensor`'s search, compared ignoring case.
const SENSOR: &str = "gimmick_door";
/// `new PlayerActionSensor(1f, transform)`.
pub(crate) const DOOR_SENSOR_RADIUS: f32 = 1.0;
const OPEN_STATE: &str = "Base Layer.Open";
const CLOSE_STATE: &str = "Base Layer.Close";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clip {
    Open,
    Close,
}

/// `CloseDoorAsync`'s wait: one `Yield`, then the accumulating loop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloseWait {
    started: u64,
    t: Option<f32>,
}

impl CloseWait {
    pub(crate) fn new(frame: u64) -> Self {
        Self {
            started: frame,
            t: None,
        }
    }

    /// One frame; true on the frame the loop exits (the SE plays).
    pub(crate) fn tick(&mut self, frame: u64, dt: f32, length: f32) -> bool {
        if frame == self.started {
            return false;
        }
        let t = self.t.get_or_insert(0.0);
        if *t < length {
            *t += dt;
            false
        } else {
            true
        }
    }
}

struct Playback {
    clip: Clip,
    clock: f32,
    close: Option<CloseWait>,
    ended: bool,
}

struct Instance {
    root: Entity,
    animator: Option<(Entity, AnimationNodeIndex, AnimationNodeIndex)>,
    lengths: (f32, f32),
}

/// The current room's door.
#[derive(Resource)]
pub(crate) struct RoomDoor {
    epoch: u64,
    skin: String,
    glb: Handle<Gltf>,
    sidecar: Handle<JsonAsset>,
    anchor: Entity,
    inside: Entity,
    /// The door sensor's attach point; `SetupDoorSensor` leaves the sensor
    /// null when the wall has no such node.
    sensor: Option<Entity>,
    pending: Option<Entity>,
    clips: Option<DoorPrefab>,
    instance: Option<Instance>,
    playback: Option<Playback>,
    refused: Option<String>,
}

/// What the skin sidecar says about the door prefab.
#[derive(Clone, Debug)]
struct DoorPrefab {
    /// The glb scene of the prefab asset.
    scene: usize,
    open: String,
    close: String,
    open_length: f32,
    close_length: f32,
    /// Material names the product does not draw (stencil-only, shadow off).
    hidden: Vec<String>,
    /// Material names with the source shader attribute of their program.
    attributes: Vec<(String, u8)>,
}

/// The rooms this port refused a door for, so the refusal is named once.
#[derive(Resource, Default)]
struct RoomDoorRefusals(Option<u64>);

pub(crate) fn install(app: &mut App) {
    app.init_resource::<RoomDoorRefusals>();
}

fn current(world: &World) -> Option<&RoomDoor> {
    let epoch = world.get_resource::<crate::site::GroundEpoch>()?.0;
    world
        .get_resource::<RoomDoor>()
        .filter(|door| door.epoch == epoch && world.get_entity(door.anchor).is_ok())
}

/// The door action point (`loc_inside`) of the current room, once found.
pub(crate) fn inside_point(world: &World) -> Option<Entity> {
    current(world)
        .filter(|door| world.get_entity(door.inside).is_ok())
        .map(|door| door.inside)
}

/// The door sensor's attach point and the door action point of the current
/// room, while the room site is the active one.
pub(crate) fn sensor(door: Option<&RoomDoor>, epoch: Option<u64>) -> Option<(Entity, Entity)> {
    let door = door.filter(|door| Some(door.epoch) == epoch)?;
    Some((door.sensor?, door.inside))
}

/// The current room's door prefab stands and its animator is bound, or the
/// door was refused (named once).
pub(crate) fn settled(world: &World) -> bool {
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|epoch| epoch.0);
    if epoch.is_some()
        && world
            .get_resource::<RoomDoorRefusals>()
            .is_some_and(|refused| refused.0 == epoch)
    {
        return true;
    }
    current(world).is_some_and(|door| door.refused.is_some() || door.instance.is_some())
}

/// Update: `SetUpDoor` for each room load. Runs once the room's scenes
/// are expanded and its skin is known.
pub(crate) fn ensure(world: &mut World) {
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| (site.is_indoor(), site.site_type.clone()));
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|epoch| epoch.0);
    let (Some((indoor, site_type)), Some(epoch)) = (site, epoch) else {
        world.remove_resource::<RoomDoor>();
        return;
    };
    if !indoor {
        world.remove_resource::<RoomDoor>();
        return;
    }
    if !world.contains_resource::<crate::site::SiteScenesReady>() {
        return;
    }
    let current = world
        .get_resource::<RoomDoor>()
        .is_some_and(|door| door.epoch == epoch && world.get_entity(door.anchor).is_ok());
    if !current {
        world.remove_resource::<RoomDoor>();
        let found = find_points(world);
        let (anchor, inside, sensor) = match found {
            Ok(points) => points,
            Err(reason) => {
                if world.resource::<RoomDoorRefusals>().0 != Some(epoch) {
                    world.resource_mut::<RoomDoorRefusals>().0 = Some(epoch);
                    warn!("[room-door] {site_type}: SetUpDoor refused: {reason}");
                }
                return;
            }
        };
        let skin = world
            .get_resource::<crate::room_appearance::RoomAppearance>()
            .map(|appearance| appearance.wall.clone())
            .unwrap_or_default();
        let server = world.resource::<AssetServer>().clone();
        world.insert_resource(RoomDoor {
            epoch,
            glb: server.load(format!("moly://site/skins/{skin}/{skin}.glb")),
            sidecar: server.load(format!("moly://site/skins/{skin}/{skin}.json")),
            skin,
            anchor,
            inside,
            sensor: sensor.as_ref().ok().copied(),
            pending: None,
            clips: None,
            instance: None,
            playback: None,
            refused: None,
        });
        info!("[room-door] {site_type}: SetUpDoor under {DOOR_ANCHOR} {anchor:?}, door action point {INSIDE} {inside:?}");
        match &sensor {
            Ok(sensor) => info!(
                "[room-door] {site_type}: SetupDoorSensor at {SENSOR} {sensor:?}, radius {DOOR_SENSOR_RADIUS}"
            ),
            Err(reason) => warn!(
                "[room-door] {site_type}: SetupDoorSensor refused: {reason}; the room has no go-home button"
            ),
        }
    }
    world.resource_scope(|world, mut door: Mut<RoomDoor>| door.build(world));
}

/// `Loc_door`, `loc_inside` and `gimmick_door` under the room's scene
/// roots. The room carries one door wall, so each is looked for once over
/// all roots. The sensor is found or named missing on its own: a second
/// `gimmick_door` is not chosen between, and it costs the sensor only.
fn find_points(world: &mut World) -> Result<(Entity, Entity, Result<Entity, String>), String> {
    let mut roots = world.query_filtered::<Entity, With<crate::site::SiteRoot>>();
    let roots: Vec<Entity> = roots.iter(world).collect();
    let (mut anchors, mut insides, mut sensors) = (Vec::new(), Vec::new(), Vec::new());
    for root in roots {
        for entity in super::descendants(world, root) {
            let Some(name) = world.get::<Name>(entity) else {
                continue;
            };
            if name.as_str() == DOOR_ANCHOR {
                anchors.push(entity);
            } else if name.as_str().eq_ignore_ascii_case(INSIDE) {
                insides.push(entity);
            } else if name.as_str().eq_ignore_ascii_case(SENSOR) {
                sensors.push(entity);
            }
        }
    }
    let sensor = match sensors.as_slice() {
        [sensor] => Ok(*sensor),
        _ => Err(format!(
            "the room has {} {SENSOR} nodes, not one",
            sensors.len()
        )),
    };
    match (anchors.as_slice(), insides.as_slice()) {
        ([anchor], [inside]) => Ok((*anchor, *inside, sensor)),
        _ => Err(format!(
            "the room has {} {DOOR_ANCHOR} and {} {INSIDE} nodes, not one of each",
            anchors.len(),
            insides.len()
        )),
    }
}

impl RoomDoor {
    fn refuse(&mut self, reason: String) {
        if self.refused.is_none() {
            warn!(
                "[room-door] {}: door prefab refused: {reason}; the room has no door prop",
                self.skin
            );
            self.refused = Some(reason);
        }
    }

    fn build(&mut self, world: &mut World) {
        if self.refused.is_some() || self.instance.is_some() {
            return;
        }
        let server = world.resource::<AssetServer>().clone();
        if server.load_state(&self.glb).is_failed() || server.load_state(&self.sidecar).is_failed()
        {
            return self.refuse(format!("site/skins/{0}/{0} did not load", self.skin));
        }
        if let Some(pending) = self.pending {
            if world.get::<InstanceReady>(pending).is_none() {
                return;
            }
            self.pending = None;
            self.bind(world, pending);
            return;
        }
        if !server.is_loaded_with_dependencies(&self.glb)
            || !server.load_state(&self.sidecar).is_loaded()
        {
            return;
        }
        let prefab = match self.read_sidecar(world) {
            Ok(prefab) => prefab,
            Err(reason) => return self.refuse(reason),
        };
        let scene = world
            .resource::<Assets<Gltf>>()
            .get(&self.glb)
            .and_then(|gltf| gltf.scenes.get(prefab.scene).cloned());
        let Some(scene) = scene else {
            return self.refuse(format!("the glb has no scene {}", prefab.scene));
        };
        // Instantiate(prefab, Loc_door): local pose as authored (identity).
        let root = world
            .spawn((
                SceneRoot(scene),
                Transform::IDENTITY,
                Visibility::Inherited,
                PendingInstance,
                Name::new("room door"),
                ChildOf(self.anchor),
            ))
            .id();
        self.pending = Some(root);
        self.clips = Some(prefab);
    }

    fn read_sidecar(&self, world: &World) -> Result<DoorPrefab, String> {
        let text = world
            .resource::<Assets<JsonAsset>>()
            .get(&self.sidecar)
            .map(|json| json.0.clone())
            .ok_or("skin sidecar not loaded")?;
        let doc: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        // AssetBundleNames.GetMysekaiHouseDoorAssetName: "mdl_" + skin + "_door_door1.prefab".
        let asset = format!("/mdl_{}_door_door1.prefab", self.skin);
        let roots: Vec<&Value> = doc["roots"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| {
                        row["assets"].as_array().is_some_and(|assets| {
                            assets
                                .iter()
                                .any(|a| a.as_str().is_some_and(|a| a.ends_with(&asset)))
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let [root] = roots.as_slice() else {
            return Err(format!("{} roots are the prefab {asset}", roots.len()));
        };
        let scene = root["scene"]
            .as_u64()
            .ok_or("the prefab root has no scene")? as usize;
        let animators = root["animators"]
            .as_array()
            .ok_or("the prefab has no animator list")?;
        let [animator] = animators.as_slice() else {
            return Err(format!("the prefab has {} animators", animators.len()));
        };
        let machine = &animator["controllerData"]["stateMachines"][0];
        let state_clip = |path: &str| -> Result<String, String> {
            let state = machine["states"]
                .as_array()
                .and_then(|states| states.iter().find(|s| s["fullPath"].as_str() == Some(path)))
                .ok_or_else(|| format!("the door controller has no state {path}"))?;
            if state["transitions"]
                .as_array()
                .is_some_and(|t| !t.is_empty())
            {
                return Err(format!("{path} has transitions"));
            }
            if state["speed"].as_f64() != Some(1.0) {
                return Err(format!("{path} speed is {}", state["speed"]));
            }
            let motions = state["motions"].as_array().ok_or("state without motions")?;
            let [motion] = motions.as_slice() else {
                return Err(format!("{path} has {} motions", motions.len()));
            };
            motion["clip"]["source"]["name"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{path} has no clip"))
        };
        let open = state_clip(OPEN_STATE)?;
        let close = state_clip(CLOSE_STATE)?;
        let length = |name: &str| -> Result<f32, String> {
            animator["clips"]
                .as_array()
                .and_then(|clips| {
                    clips
                        .iter()
                        .find(|clip| clip["source"]["source"]["name"].as_str() == Some(name))
                })
                .and_then(|clip| clip["stopTime"].as_f64())
                .map(|stop| stop as f32)
                .ok_or_else(|| format!("clip {name} has no length"))
        };
        let (open_length, close_length) = (length(&open)?, length(&close)?);
        let hidden = doc["materials"]
            .as_array()
            .map(|materials| {
                materials
                    .iter()
                    .filter(|m| {
                        let floats = &m["floats"];
                        let shader = m["shader"]["name"].as_str().unwrap_or_default();
                        floats["_ColorMask"].as_f64() == Some(0.0)
                            || (shader == "Mysekai/Fixture/ShadowMesh"
                                && floats["_Show"].as_f64() == Some(0.0))
                    })
                    .filter_map(|m| m["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let attributes = doc["materials"]
            .as_array()
            .map(|materials| {
                materials
                    .iter()
                    .filter_map(|m| Some((m["name"].as_str()?.to_owned(), door_part_attribute(m)?)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(DoorPrefab {
            scene,
            open,
            close,
            open_length,
            close_length,
            hidden,
            attributes,
        })
    }

    fn bind(&mut self, world: &mut World, root: Entity) {
        let prefab = self.clips.clone().expect("read before the spawn");
        let clips = world.resource::<Assets<Gltf>>().get(&self.glb).map(|gltf| {
            (
                gltf.named_animations.get(prefab.open.as_str()).cloned(),
                gltf.named_animations.get(prefab.close.as_str()).cloned(),
            )
        });
        let entities = super::descendants(world, root);
        // Stencil-only and switched-off shadow meshes draw nothing here.
        let mut hidden = 0;
        for entity in &entities {
            let hide = world
                .get::<GltfMaterialName>(*entity)
                .is_some_and(|name| prefab.hidden.contains(&name.0));
            if hide {
                world.entity_mut(*entity).insert(Visibility::Hidden);
                hidden += 1;
            }
        }
        // The parts' source shader attributes, read by the silhouette's
        // stencil replay (a hidden stencil mesh is replayed all the same).
        let mut marked = 0;
        for entity in &entities {
            let attribute = world.get::<GltfMaterialName>(*entity).and_then(|name| {
                prefab
                    .attributes
                    .iter()
                    .find(|(material, _)| *material == name.0)
                    .map(|(_, attribute)| *attribute)
            });
            if let Some(attribute) = attribute {
                world
                    .entity_mut(*entity)
                    .insert(SourceStencilAttribute(attribute));
                marked += 1;
            }
        }
        let animator = entities
            .iter()
            .copied()
            .find(|entity| world.get::<AnimationPlayer>(*entity).is_some());
        let bound = match (clips, animator) {
            (Some((Some(open), Some(close))), Some(animator)) => {
                let mut graph = AnimationGraph::new();
                let open_node = graph.add_clip(open, 1.0, graph.root);
                let close_node = graph.add_clip(close, 1.0, graph.root);
                let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
                world
                    .entity_mut(animator)
                    .insert(AnimationGraphHandle(graph));
                Some((animator, open_node, close_node))
            }
            _ => {
                warn!(
                    "[room-door] {}: the clips {} / {} or the animation player are missing; the door stands closed",
                    self.skin, prefab.open, prefab.close
                );
                None
            }
        };
        info!(
            "[room-door] {}: door prefab bound {} (scene {}, clips {} {:.4}s / {} {:.4}s, {hidden} stencil/shadow meshes hidden, {marked} parts with a source shader attribute)",
            self.skin,
            if bound.is_some() { "with its animator" } else { "without an animator" },
            prefab.scene,
            prefab.open,
            prefab.open_length,
            prefab.close,
            prefab.close_length
        );
        self.instance = Some(Instance {
            root,
            animator: bound,
            lengths: (prefab.open_length, prefab.close_length),
        });
    }

    fn play(&mut self, world: &mut World, clip: Clip, frame: u64) {
        let Some(instance) = &self.instance else {
            info!("[room-door] {clip:?}: the room has no door prop (named above)");
            return;
        };
        // OpenDoorAsync's ShowDoor; both states start from their first frame.
        if clip == Clip::Open {
            if let Some(mut visibility) = world.get_mut::<Visibility>(instance.root) {
                *visibility = Visibility::Inherited;
            }
        }
        if let Some((animator, open, close)) = instance.animator {
            if let Some(mut player) = world.get_mut::<AnimationPlayer>(animator) {
                player.stop_all();
                let node = if clip == Clip::Open { open } else { close };
                player.play(node).pause().seek_to(0.0);
            }
        }
        self.playback = Some(Playback {
            clip,
            clock: 0.0,
            close: (clip == Clip::Close).then(|| CloseWait::new(frame)),
            ended: false,
        });
        info!(
            "[room-door] Animator.Play({}) at frame {frame}",
            if clip == Clip::Open { "Open" } else { "Close" }
        );
    }
}

/// `RoomController.OpenDoorAsync().Forget()`.
pub(crate) fn open(world: &mut World) {
    let frame = u64::from(world.resource::<FrameCount>().0);
    if world.contains_resource::<RoomDoor>() {
        world.resource_scope(|world, mut door: Mut<RoomDoor>| door.play(world, Clip::Open, frame));
    } else {
        info!("[room-door] OpenDoorAsync: no door for this room");
    }
}

/// Whether an awaited `OpenDoorAsync` would have returned: the open clip of
/// the current room's door has reached its end. True without a door (the
/// await has nothing to wait on); false while the open clip plays or a close
/// clip is playing.
pub(crate) fn open_finished(world: &World) -> bool {
    match current(world).and_then(|door| door.playback.as_ref()) {
        Some(playback) => playback.clip == Clip::Open && playback.ended,
        None => current(world).is_none_or(|door| door.instance.is_none()),
    }
}

/// `RoomController.CloseDoorAsync().Forget()`.
pub(crate) fn close(world: &mut World) {
    let frame = u64::from(world.resource::<FrameCount>().0);
    if world.contains_resource::<RoomDoor>() {
        world.resource_scope(|world, mut door: Mut<RoomDoor>| door.play(world, Clip::Close, frame));
    } else {
        info!("[room-door] CloseDoorAsync: no door for this room");
    }
}

/// Update, after the executor: the door's clip clock and the close SE.
pub(crate) fn advance(world: &mut World) {
    let Some(mut door) = world.remove_resource::<RoomDoor>() else {
        return;
    };
    let dt = world.resource::<Time>().delta_secs();
    let frame = u64::from(world.resource::<FrameCount>().0);
    if let (Some(playback), Some(instance)) = (door.playback.as_mut(), door.instance.as_ref()) {
        let length = match playback.clip {
            Clip::Open => instance.lengths.0,
            Clip::Close => instance.lengths.1,
        };
        playback.clock = (playback.clock + dt).min(length);
        if let Some((animator, open, close)) = instance.animator {
            let node = if playback.clip == Clip::Open {
                open
            } else {
                close
            };
            if let Some(mut player) = world.get_mut::<AnimationPlayer>(animator) {
                if let Some(active) = player.animation_mut(node) {
                    active.seek_to(playback.clock);
                }
            }
        }
        match playback.clip {
            Clip::Open => {
                if !playback.ended && playback.clock >= length {
                    playback.ended = true;
                    info!("[room-door] Open clip at its end ({length:.4}s), held open");
                }
            }
            Clip::Close => {
                if let Some(wait) = playback.close.as_mut() {
                    if !playback.ended && wait.tick(frame, dt, length) {
                        playback.ended = true;
                        super::push_se(world, "se_door_close");
                        info!(
                            "[room-door] CloseDoorAsync loop done ({:.4}s >= {length:.4}s): se_door_close",
                            wait.t.unwrap_or_default()
                        );
                    }
                }
            }
        }
    }
    world.insert_resource(door);
}

#[cfg(test)]
mod tests {
    //! Research instrument: `CloseDoorAsync`'s wait as the source loop runs
    //! it, frame by frame (the Play frame, one Yield, then add-then-yield
    //! while below the length), at 0.25 s frames for the 1.0 s close clip.
    use super::*;

    #[test]
    fn close_wait_runs_the_source_loop() {
        let mut wait = CloseWait::new(10);
        // Frame 10: Play and Yield. Frames 11..14 add 0.25 each (t reaches
        // 1.0 on frame 14's add); frame 15 finds t >= 1.0 and plays the SE.
        let done: Vec<bool> = (10..=15).map(|f| wait.tick(f, 0.25, 1.0)).collect();
        assert_eq!(done, [false, false, false, false, false, true]);
    }
}
