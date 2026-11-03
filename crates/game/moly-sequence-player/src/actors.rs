//! The actors: each frame's position and facing on the root, the body's pose,
//! and the face.
//!
//! The player's frame decides a pose per cast member and leaves it in
//! [`PosePlan`]; [`apply`] samples it on the bodies in a system that has the
//! animation stores, because the player's own frame function would have to
//! clone an asset store to reach them. The pose never goes through the
//! transitions: `MotionDriver::pose` starts each layer on the body's graph at
//! its time and weight and clears the body's `AnimationTransitions`, whose
//! `advance_transitions` would otherwise write the departing idle over the
//! weights the player set every frame. The animations a transition was
//! fading are paused and stay owned by the body (nothing stops them, so a
//! layer the player is about to write is never stopped); the next segment
//! change of locomotion crossfades in from them.
//!
//! An actor whose motion track shows nothing walks while its move carries it
//! and stands otherwise. A walk's loop carries a metre of ground at the speed
//! it is played, so the metres walked place the loop's phase and each step
//! continues the one before ([`walked`]); a frame that asks for the same
//! place as the frame before leaves the clips playing and the feet stand.
//! Standing plays the idle loop at the sequence time.
//!
//! The document's facing is the root's rotation about +Y. The place and the
//! facing are also written on the character's walk state, which the walk
//! system lays on the root every frame, so a paused or stopped actor stays
//! where the player left it and its AI resumes from there. A body another
//! state took is refused by that state's name, and the player stops with it.


use bevy::animation::AnimationPlayer;
use bevy::gltf::Gltf;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use moly_features::character::{BodyToken, MotionDriver, MotionKind, MotionLibrary, PoseLayer, SEGMENT_BLEND};
use moly_features::npc::WalkState;
use moly_sequence::doc::{MotionMode, Part};
use moly_sequence::{ActorFrame, MotionFrame};

use crate::run::ClipNames;

/// Metres the walk's loop carries the actor.
pub(crate) const WALK_METRE: f32 = 1.0;

/// The longest step between two laid frames (seconds) that moves the walk on:
/// a frame drawn after a pause or a seek leaves the feet where they stand.
const WALK_STEP: f32 = 0.25;

/// The bones whose height over the feet each part of a body reads, in order:
/// the first one the body has is taken.
pub(crate) const BONE_CHAINS: &[(Part, &[&str])] = &[(Part::Waist, &["Hips"]), (Part::Chest, &["Chest", "Spine"]), (Part::Head, &["Head"])];

/// One cast member's pose for a frame: its root's place and facing, the
/// layers its body plays there (empty when the clips it plays need no new
/// write), and its face.
pub(crate) struct Posed {
    pub unit: u32,
    pub entity: Entity,
    pub token: BodyToken,
    pub position: Vec3,
    pub yaw: f32,
    pub layers: Vec<PoseLayer>,
    pub eye: Option<String>,
    pub mouth: Option<String>,
}

/// The poses the player's frame left for the bodies this frame.
#[derive(Resource, Default)]
pub(crate) struct PosePlan {
    posed: Vec<Posed>,
}

impl PosePlan {
    /// Drops the poses of a playback that is over.
    pub fn clear(&mut self) {
        self.posed.clear();
    }
}

/// A library clip's length, as the world holds it now.
pub(crate) struct Lengths<'a> {
    gltf: &'a Gltf,
    clips: &'a Assets<AnimationClip>,
}

impl Lengths<'_> {
    fn of(&self, name: &str) -> Option<f32> {
        moly_features::character::motion_length(name, self.gltf, self.clips)
    }
}

/// Decides each cast member's pose for this frame and leaves it in
/// [`PosePlan`]. A body another state took is refused by that state's name.
pub(crate) fn plan(world: &mut World, player: &mut crate::Player, frames: &[ActorFrame<'_>], from: f32, to: f32) -> Result<(), String> {
    let mut posed = Vec::with_capacity(frames.len());
    if player.sampled.len() != frames.len() {
        player.sampled = vec![Vec::new(); frames.len()];
    }
    let library = world
        .get_resource::<MotionLibrary>()
        .map(|library| library.gltf.clone())
        .ok_or("the shared motion library is not installed")?;
    let libraries = world.get_resource::<Assets<Gltf>>().ok_or("the shared motion library is not loaded")?;
    let clips = world.get_resource::<Assets<AnimationClip>>().ok_or("the animation clips are not in the world")?;
    let lengths = Lengths { gltf: libraries.get(&library).ok_or("the shared motion library is not loaded")?, clips };
    for (index, frame) in frames.iter().enumerate() {
        let (unit, entity, token) = (player.actors[index].unit, player.actors[index].entity, player.actors[index].token);
        let (Some(entity), Some(token)) = (entity, token) else {
            continue;
        };
        if let Some(driver) = world.get::<MotionDriver>(entity) {
            if !driver.holds(token) {
                let by = driver.held_by().map_or("locomotion".to_owned(), |owner| owner.name().to_owned());
                return Err(format!("{by} took character {unit}"));
            }
        }
        let metres = walked(player.actors[index].walked, frame, from, to);
        player.actors[index].walked = metres;
        let names = player.clips.as_ref().and_then(|clips| clips.get(index));
        let layers = layers(names, frame, metres, to, &lengths);
        let key: Vec<(String, bool, f32, f32)> = layers.iter().map(|layer| (layer.clip.clone(), layer.looping, layer.local, layer.weight)).collect();
        let same = key == player.sampled[index];
        player.sampled[index] = key;
        // A face is a command: it is written when its name is not the one
        // written last, and the actor only records what `apply` writes.
        let written = &player.actors[index];
        let eye = frame.eye.map(str::to_owned).filter(|eye| Some(eye) != written.eye.as_ref());
        let mouth = frame.mouth.map(str::to_owned).filter(|mouth| Some(mouth) != written.mouth.as_ref());
        posed.push(Posed {
            unit,
            entity,
            token,
            position: frame.position,
            yaw: frame.yaw,
            layers: if same { Vec::new() } else { layers },
            eye,
            mouth,
        });
    }
    world.resource_mut::<PosePlan>().posed = posed;
    Ok(())
}

/// The metres the actor has walked: a moving frame adds its speed over the
/// seconds since the last laid frame, and a still frame starts again from
/// zero, so every move starts its walk at the loop's first frame.
fn walked(before: f32, frame: &ActorFrame<'_>, from: f32, to: f32) -> f32 {
    match frame.moving {
        Some(moving) if to - from > 0.0 && to - from <= WALK_STEP => before + moving.speed * (to - from) / WALK_METRE,
        Some(_) => before,
        None => 0.0,
    }
}

/// The layers an actor frame plays: its motion track's, or the walk while its
/// move carries it, or the idle.
fn layers(names: Option<&ClipNames>, frame: &ActorFrame<'_>, metres: f32, t: f32, length: &Lengths<'_>) -> Vec<PoseLayer> {
    let mut layers: Vec<PoseLayer> = frame.motions.iter().filter_map(|motion| layer(motion, length)).collect();
    if layers.is_empty() {
        if frame.moving.is_some() {
            if let Some(walk) = names.and_then(|names| names.walk[0].clone()) {
                layers.push(PoseLayer { clip: walk, local: metres.rem_euclid(WALK_METRE), weight: 1.0, looping: true });
            }
        }
        if layers.is_empty() {
            if let Some(idle) = names.and_then(|names| names.idle[0].clone()) {
                layers.push(PoseLayer { clip: idle, local: t, weight: 1.0, looping: true });
            }
        }
    }
    layers
}

/// The layer of one motion frame: a looping motion plays its loop from the
/// clip's time; a motion that plays once plays its family's start, then the
/// loop, then the end where the library has one, else the loop's last frame.
fn layer(motion: &MotionFrame<'_>, length: &Lengths<'_>) -> Option<PoseLayer> {
    let looping = format!("{}_L", motion.name);
    match motion.mode {
        MotionMode::Loop => {
            let clip = if length.of(&looping).is_some() { looping } else { motion.name.to_owned() };
            Some(PoseLayer { clip, local: motion.local, weight: motion.weight, looping: true })
        }
        MotionMode::Once => {
            let start = format!("{}_S", motion.name);
            let end = format!("{}_E", motion.name);
            let start_length = length.of(&start);
            if let Some(start_length) = start_length {
                if motion.local < start_length {
                    return Some(PoseLayer { clip: start, local: motion.local, weight: motion.weight, looping: false });
                }
            }
            let into = motion.local - start_length.unwrap_or(0.0);
            match length.of(&looping) {
                Some(loop_length) if into < loop_length => Some(PoseLayer { clip: looping, local: into, weight: motion.weight, looping: true }),
                Some(loop_length) => match length.of(&end) {
                    Some(end_length) if into - loop_length < end_length => {
                        Some(PoseLayer { clip: end, local: into - loop_length, weight: motion.weight, looping: false })
                    }
                    // The loop holds its last frame once the end has played.
                    _ => Some(PoseLayer { clip: looping, local: into, weight: motion.weight, looping: false }),
                },
                None => Some(PoseLayer { clip: motion.name.to_owned(), local: motion.local, weight: motion.weight, looping: false }),
            }
        }
    }
}

/// Applies the frame's plan: each root's place and facing, its body's pose
/// and its face. A normal system after the player's frame and the characters'
/// animation driver, so it has the animation stores the frame function
/// cannot borrow.
/// The stores a pose reads its clips from and adds its graph nodes to.
#[derive(SystemParam)]
pub(crate) struct MotionStores<'w> {
    library: Res<'w, MotionLibrary>,
    libraries: Res<'w, Assets<Gltf>>,
    clips: Res<'w, Assets<AnimationClip>>,
    graphs: ResMut<'w, Assets<AnimationGraph>>,
}

/// The face materials and the facial tables a planned face is written with.
#[derive(SystemParam)]
pub(crate) struct FaceStores<'w, 's> {
    materials: ResMut<'w, Assets<moly_features::character_material::CharacterMaterial>>,
    toons: Query<'w, 's, &'static moly_features::character_material::ToonMaterials>,
    tables: Option<Res<'w, moly_features::alone_action_runtime::FacialTables>>,
}

/// The events' actions, polled after the player's frame and the poses it
/// planned have been laid: each run's step is polled, and the next step of a
/// run starts once the one before has ended.
pub(crate) fn events(world: &mut World) {
    let mut player = std::mem::take(&mut *world.resource_mut::<crate::Player>());
    crate::controls::poll(world, &mut player);
    world.insert_resource(player);
}

/// Lays the frame's plan on the bodies: each root's place and facing (and
/// the walk state's), the body's pose, then the face.
pub(crate) fn apply(
    mut plan: ResMut<PosePlan>,
    mut stores: MotionStores,
    mut faces: FaceStores,
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut drivers: Query<&mut MotionDriver>,
    mut roots: Query<(&mut Transform, &mut GlobalTransform, Option<&mut WalkState>)>,
    mut player: ResMut<crate::Player>,
) {
    let MotionStores { library, libraries, clips, graphs } = &mut stores;
    let Some(gltf) = libraries.get(&library.gltf) else {
        return;
    };
    for posed in plan.posed.drain(..) {
        if let Ok((mut transform, mut global, walk)) = roots.get_mut(posed.entity) {
            transform.translation = posed.position;
            transform.rotation = Quat::from_rotation_y(posed.yaw.to_radians());
            *global = GlobalTransform::from(*transform);
            if let Some(mut walk) = walk {
                walk.0.position = posed.position.to_array();
                walk.0.forward = (transform.rotation * Vec3::Z).to_array();
            }
        }
        if !posed.layers.is_empty() {
            let Ok(mut driver) = drivers.get_mut(posed.entity) else {
                continue;
            };
            let Ok((mut animation, mut transitions)) = animators.get_mut(driver.player) else {
                continue;
            };
            if let Err(reason) = driver.pose(
                posed.token,
                &posed.layers,
                graphs,
                gltf,
                &mut animation,
                &mut transitions,
                clips,
                posed.unit,
            ) {
                warn!("[sequence] character {}: {reason}", posed.unit);
            }
        }
        if posed.eye.is_none() && posed.mouth.is_none() {
            continue;
        }
        let Some(tables) = faces.tables.as_deref() else {
            warn!("[sequence] character {}: the facial tables are not loaded, so its face is not written", posed.unit);
            continue;
        };
        let Ok(toon) = faces.toons.get(posed.entity) else {
            continue;
        };
        write_face(&mut faces.materials, tables, toon, posed.eye.as_deref(), posed.mouth.as_deref());
        // The names just written are the ones a frame only has to match.
        if let Some(actor) = player.actors.iter_mut().find(|actor| actor.unit == posed.unit) {
            actor.eye = posed.eye.clone();
            actor.mouth = posed.mouth.clone();
        }
    }
}
/// Writes the eye and mouth patterns on the body's face materials, a name the
/// tables do not hold reading as their zero-valued row.
fn write_face(
    materials: &mut Assets<moly_features::character_material::CharacterMaterial>,
    tables: &moly_features::alone_action_runtime::FacialTables,
    toon: &moly_features::character_material::ToonMaterials,
    eye: Option<&str>,
    mouth: Option<&str>,
) {
    if let (Some(pattern), Some(handle)) = (eye, toon.slot_handle("eye")) {
        moly_features::alone_action_runtime::apply_eye_pattern(materials, handle, Some(tables.eye_pattern(pattern).unwrap_or_default()));
    }
    if let (Some(pattern), Some(handle)) = (mouth, toon.slot_handle("mouth")) {
        moly_features::alone_action_runtime::apply_mouth_pattern(materials, handle, tables.lip_pattern(pattern).unwrap_or_default());
    }
}

/// The parts of a body the camera can aim at, in metres over its feet: a
/// part whose bones the body does not have is left out and reads as the feet.
pub(crate) fn heights(world: &mut World, body: Entity, unit: u32) -> Result<Vec<(Part, f32)>, String> {
    let root = world.get::<GlobalTransform>(body).map(|global| global.translation()).ok_or(format!("unit {unit}'s body has no place"))?;
    let mut named = world.query::<(&Name, &GlobalTransform)>();
    let mut parts = Vec::new();
    for (part, chain) in BONE_CHAINS {
        if let Some((_, global)) = named.iter(world).find(|(name, _)| chain.contains(&name.as_str())) {
            parts.push((*part, global.translation().y - root.y));
        }
    }
    Ok(parts)
}

/// Ends each actor's hold: when the body is still the player's, the idle
/// plays through the transitions and locomotion carries on from it.
pub(crate) fn settle<'a>(world: &mut World, actors: impl Iterator<Item = &'a mut crate::Actor>) {
    for actor in actors {
        let Some(entity) = actor.entity else {
            continue;
        };
        let Some(token) = actor.token.take() else {
            continue;
        };
        let (animator, idle_node) = {
            let Some(mut driver) = world.get_mut::<MotionDriver>(entity) else {
                continue;
            };
            if !driver.hand_back(token) {
                continue;
            }
            (driver.player, driver.idle)
        };
        let mut players = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
        if let Ok((mut player, mut transitions)) = players.get_mut(world, animator) {
            transitions.play(&mut player, idle_node, SEGMENT_BLEND).repeat();
        }
        drop(players);
        if let Some(mut driver) = world.get_mut::<MotionDriver>(entity) {
            driver.playing = Some(MotionKind::Idle);
        }
        actor.held = false;
    }
}

