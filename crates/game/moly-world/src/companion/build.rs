//! A companion's build: a fixture already in the layout stands as a ghost
//! and turns solid one step per hammer hit.
//!
//! The pieces are the original's own, put to a new use:
//!
//! - the ghost is the furniture material's own Bayer dither
//!   (`_DitherAlpha`), on copies of the instance's materials; it starts at
//!   [`GHOST_ALPHA`] and each hit raises it, the last one to solid;
//! - the swing is the player's pickaxe harvest loop ([`BUILD_CLIP`]); its
//!   hit event (`PublishHarvestActionHit`) times each step;
//! - the tool is the player tool model the craft motion holds, the hammer
//!   ([`AvatarItem::Hammer`]), on the companion's right arm.
//!
//! A build is one command ([`super::CompanionCommand::Build`]): the
//! companion waits for the navigation baked with the fixture in it, walks
//! to the nearest walkable point beside the fixture, turns to it and swings.

use super::{faces, turn_to, Companion, CompanionState, CompanionStatus, BuildCue, Gait, Walker};
use crate::fixture::FixtureInstanceSeed;
use crate::fixture_material::{FixtureMaterial, FixtureMaterialsSwapped};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::avatar_item::{self, AvatarItem, AvatarItemView, Created, Hand};
use crate::player_avatar::{AvatarDriver, STATE_FADE};
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use std::collections::HashMap;

/// The swing of a build: the pickaxe harvest loop.
pub const BUILD_CLIP: &str = "mov_u000_site_pickax01_L";
/// The clip event that marks a hit.
const HIT_EVENT: &str = "PublishHarvestActionHit";
/// How solid a fixture stands before the first hit.
pub const GHOST_ALPHA: f32 = 0.3;
/// The hits of a build when the command names none, and the most it takes.
pub const DEFAULT_SWINGS: u32 = 4;
pub const MAX_SWINGS: u32 = 12;
/// How long a build waits for the navigation baked with its fixture.
const NAVIGATION_WAIT_SECS: f32 = 60.0;
/// The rings searched for a place to stand beside the fixture, metres.
const STAND_RINGS: (f32, f32, f32) = (0.5, 4.0, 0.25);
const STAND_ANGLES: usize = 16;
/// How close to its stand point a stopped walk still swings from.
const STAND_REACH: f32 = 0.8;

/// Fixture instances drawn as ghosts, by layout uid, with how solid each
/// stands (0 not drawn, 1 solid). An instance not listed is drawn as it is.
#[derive(Resource, Default)]
pub struct FixtureGhosts(HashMap<String, f32>);

impl FixtureGhosts {
    pub fn set(&mut self, uid: &str, alpha: f32) {
        self.0.insert(uid.to_owned(), alpha.clamp(0.0, 1.0));
    }

    /// The instance is drawn solid again.
    pub fn clear(&mut self, uid: &str) {
        self.0.remove(uid);
    }

    /// Every instance is drawn solid again.
    pub fn clear_all(&mut self) {
        self.0.clear();
    }

    pub fn alpha(&self, uid: &str) -> Option<f32> {
        self.0.get(uid).copied()
    }
}

/// A fixture instance drawn as a ghost: its primitives' own materials, and
/// the dithered copies drawn in their place.
#[derive(Component)]
pub struct Ghosted {
    originals: Vec<(Entity, Handle<FixtureMaterial>)>,
    copies: Vec<Handle<FixtureMaterial>>,
    alpha: f32,
}

/// Update: every listed instance draws through its dithered copies; an
/// instance no longer listed gets its own materials back.
#[allow(clippy::type_complexity)]
pub fn apply_ghosts(
    mut commands: Commands,
    ghosts: Res<FixtureGhosts>,
    swapped: Option<Res<FixtureMaterialsSwapped>>,
    mut materials: ResMut<Assets<FixtureMaterial>>,
    roots: Query<(Entity, &FixtureInstanceSeed, Option<&Ghosted>)>,
    children: Query<&Children>,
    handles: Query<&MeshMaterial3d<FixtureMaterial>>,
) {
    if swapped.is_none() {
        return;
    }
    for (root, seed, ghosted) in &roots {
        let wanted = ghosts.alpha(seed.uid()).filter(|alpha| *alpha < 1.0);
        match (wanted, ghosted) {
            (None, None) => {}
            (None, Some(ghosted)) => {
                for (entity, handle) in &ghosted.originals {
                    if let Ok(mut primitive) = commands.get_entity(*entity) {
                        primitive.insert(MeshMaterial3d(handle.clone()));
                    }
                }
                commands.entity(root).remove::<Ghosted>();
                info!("[build] fixture {} drawn solid", seed.uid());
            }
            (Some(alpha), Some(ghosted)) => {
                if ghosted.alpha == alpha {
                    continue;
                }
                for handle in &ghosted.copies {
                    if let Some(mut material) = materials.get_mut(handle) {
                        material.params.dither_alpha = alpha;
                    }
                }
                commands.entity(root).insert(Ghosted {
                    originals: ghosted.originals.clone(),
                    copies: ghosted.copies.clone(),
                    alpha,
                });
            }
            (Some(alpha), None) => {
                let mut originals = Vec::new();
                let mut stack = vec![root];
                while let Some(entity) = stack.pop() {
                    if let Ok(handle) = handles.get(entity) {
                        originals.push((entity, handle.0.clone()));
                    }
                    if let Ok(kids) = children.get(entity) {
                        stack.extend(kids.iter());
                    }
                }
                // The scene has not expanded yet.
                if originals.is_empty() {
                    continue;
                }
                let mut copies = Vec::with_capacity(originals.len());
                for (entity, handle) in &originals {
                    let Some(mut material) = materials.get(handle).cloned() else {
                        continue;
                    };
                    material.key.dither = true;
                    material.params.dither_alpha = alpha;
                    let copy = materials.add(material);
                    commands.entity(*entity).insert(MeshMaterial3d(copy.clone()));
                    copies.push(copy);
                }
                info!(
                    "[build] fixture {} drawn as a ghost at {alpha:.2} ({} primitives)",
                    seed.uid(),
                    copies.len()
                );
                commands.entity(root).insert(Ghosted {
                    originals,
                    copies,
                    alpha,
                });
            }
        }
    }
}

/// A build in flight on a companion.
#[derive(Component, Clone, Debug)]
pub struct BuildPlan {
    pub seq: u64,
    pub uid: String,
    /// The fixture's position, which the companion turns to.
    pub at: Vec3,
    pub swings: u32,
    pub(super) phase: BuildPhase,
    /// Seconds spent waiting for the navigation.
    pub(super) waited: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BuildPhase {
    /// Waiting for the navigation baked with the fixture in the layout.
    Navigation,
    /// Walking to its stand point.
    Approach { stand: Vec3 },
    /// Swinging.
    Swing,
}

/// The swings of a build: the clip's time and the hits counted so far.
#[derive(Component)]
pub(super) struct Hammering {
    elapsed: f32,
    length: f32,
    hit: f32,
    hits: u32,
    started: bool,
}

/// The tool model in a companion's right hand.
#[derive(Component)]
pub(super) struct HeldTool(pub(super) Entity);

impl BuildPlan {
    /// How solid the fixture stands after `hits` of the build's swings.
    fn alpha(&self, hits: u32) -> f32 {
        GHOST_ALPHA + (1.0 - GHOST_ALPHA) * hits as f32 / self.swings.max(1) as f32
    }
}

/// The nearest point beside `at` the companion can stand on and reach from
/// where it is, on the first ring that has one; on that ring, the point
/// nearest the companion.
fn stand_point(
    face: &crate::walk_face::WalkFace,
    objective: &crate::npc_objective::ObjectiveFace,
    from: Vec3,
    at: Vec3,
) -> Option<Vec3> {
    let (first, last, step) = STAND_RINGS;
    let mut radius = first;
    while radius <= last {
        let mut best: Option<(f32, Vec3)> = None;
        for index in 0..STAND_ANGLES {
            let angle = index as f32 * std::f32::consts::TAU / STAND_ANGLES as f32;
            let xz = [at.x + radius * angle.cos(), at.z + radius * angle.sin()];
            if !face.walkable_at(xz) {
                continue;
            }
            let Some(point) = objective.navigation_point_at(xz) else {
                continue;
            };
            if face.path([from.x, from.z], xz).is_none() {
                continue;
            }
            let point = Vec3::from(point);
            let distance = point.distance(from);
            if best.is_none_or(|(known, _)| distance < known) {
                best = Some((distance, point));
            }
        }
        if let Some((_, point)) = best {
            return Some(point);
        }
        radius += step;
    }
    None
}

/// Update, after the commands: a build waiting for its navigation sets out
/// once the walk cells are baked with its fixture in the layout.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn approach(
    mut commands: Commands,
    time: Res<Time>,
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    revision: Option<Res<crate::fixture::FixtureLayoutRevision>>,
    mut ghosts: ResMut<FixtureGhosts>,
    mut companions: Query<(
        Entity,
        &Companion,
        &mut CompanionStatus,
        &mut BuildPlan,
        &mut Transform,
        &mut WalkState,
        &mut PathSlot,
        &mut RouteStops,
        &mut MotionPhase,
    )>,
) {
    for (entity, companion, mut status, mut plan, mut transform, mut walk, mut slot, mut route, mut phase) in
        &mut companions
    {
        if plan.phase != BuildPhase::Navigation {
            continue;
        }
        let unit = companion.unit;
        let ready = faces(site.as_deref(), epoch.as_deref(), face.as_deref(), objective.as_deref())
            .filter(|(face, _)| revision.as_deref().is_some_and(|revision| face.layout_revision() == revision.0));
        let Some((face, objective)) = ready else {
            plan.waited += time.delta_secs();
            if plan.waited > NAVIGATION_WAIT_SECS {
                ghosts.clear(&plan.uid);
                status.fail("the navigation with the fixture in it was not baked");
                error!(
                    "[companion] unit {unit} build {}: no navigation with fixture {} in it after {NAVIGATION_WAIT_SECS:.0} s",
                    plan.seq, plan.uid
                );
                commands.entity(entity).remove::<BuildPlan>();
            }
            continue;
        };
        let from = transform.translation;
        let Some(stand) = stand_point(face, objective, from, plan.at) else {
            ghosts.clear(&plan.uid);
            status.fail("no walkable point beside the fixture");
            error!(
                "[companion] unit {unit} build {}: no walkable point beside fixture {} at {:?}",
                plan.seq, plan.uid, plan.at
            );
            commands.entity(entity).remove::<BuildPlan>();
            continue;
        };
        info!(
            "[companion] unit {unit} build {}: fixture {} at {:?}; stands at {stand:?}",
            plan.seq, plan.uid, plan.at
        );
        if stand.distance(from) <= STAND_REACH {
            plan.phase = BuildPhase::Swing;
            continue;
        }
        match crate::npc::companion::depart(
            unit,
            &mut walk,
            &mut slot,
            &mut route,
            face,
            objective,
            stand.to_array(),
            0.3,
        ) {
            Ok(next) => {
                *phase = next;
                if let Some(turned) = turn_to(from, stand) {
                    transform.rotation = turned;
                }
                status.state = CompanionState::Walking;
                plan.phase = BuildPhase::Approach { stand };
                commands.entity(entity).insert(Walker::started());
            }
            // Out of reach and no walk there: the hammer would hit the air.
            Err(reason) => {
                ghosts.clear(&plan.uid);
                status.fail(reason);
                error!(
                    "[companion] unit {unit} build {}: the walk to fixture {} was refused: {reason}",
                    plan.seq, plan.uid
                );
                commands.entity(entity).remove::<BuildPlan>();
            }
        }
    }
}

/// What the walk of a build's approach ended with: the swing, or the build's
/// failure when it stopped out of reach.
pub(super) fn walk_ended(
    plan: &mut BuildPlan,
    status: &mut CompanionStatus,
    ghosts: &mut FixtureGhosts,
    at: Vec3,
    arrived: bool,
) -> bool {
    let BuildPhase::Approach { stand } = plan.phase else {
        return false;
    };
    if arrived || at.distance(stand) <= STAND_REACH {
        plan.phase = BuildPhase::Swing;
        return true;
    }
    ghosts.clear(&plan.uid);
    status.fail(crate::npc::companion::WALK_STOPPED);
    error!(
        "[companion] build {}: the walk to fixture {} stopped {:.2} m from its stand point",
        plan.seq,
        plan.uid,
        at.distance(stand)
    );
    false
}

/// Update, after the walks: the swings of each build that reached its
/// fixture, the hammer in its hand and the fixture's steps to solid; each
/// hit and the built fixture are written as [`BuildCue`]s.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn swing(
    mut commands: Commands,
    time: Res<Time>,
    mut cues: MessageWriter<BuildCue>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    view: Res<AvatarItemView>,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut ghosts: ResMut<FixtureGhosts>,
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut companions: Query<(
        Entity,
        &Companion,
        &mut CompanionStatus,
        &mut BuildPlan,
        &mut AvatarDriver,
        &mut Gait,
        &mut Transform,
        Option<&mut Hammering>,
        Option<&HeldTool>,
    )>,
) {
    for (entity, companion, mut status, plan, mut driver, mut gait, mut transform, hammering, held) in
        &mut companions
    {
        if plan.phase != BuildPhase::Swing {
            continue;
        }
        let unit = companion.unit;
        if held.is_none() {
            match avatar_item::create(
                &view,
                Hand::Right,
                AvatarItem::Hammer,
                &mut commands,
                &server,
                &gltfs,
                Some(&*driver),
                &names,
                &parents,
            ) {
                Created::Done(tool) => {
                    commands.entity(entity).insert(HeldTool(tool));
                }
                Created::Wait(_) => {}
                Created::Refused(reason) => {
                    warn!("[companion] unit {unit} build {}: no hammer in hand: {reason}", plan.seq);
                }
            }
        }
        let Some(mut hammering) = hammering else {
            let Some(clip) = driver.clips().get(BUILD_CLIP) else {
                ghosts.clear(&plan.uid);
                status.fail("the motion group has no build swing");
                error!("[companion] unit {unit} build {}: no clip {BUILD_CLIP}", plan.seq);
                commands.entity(entity).remove::<BuildPlan>();
                continue;
            };
            let length = clip.length;
            let hit = clip
                .events
                .iter()
                .find(|(_, function, _)| function == HIT_EVENT)
                .map_or(length * 0.5, |(time, ..)| *time);
            if let Some(turned) = turn_to(transform.translation, plan.at) {
                transform.rotation = turned;
            }
            status.state = CompanionState::Building;
            commands.entity(entity).insert(Hammering {
                elapsed: 0.0,
                length,
                hit,
                hits: 0,
                started: false,
            });
            info!(
                "[companion] unit {unit} build {}: {} swings of {BUILD_CLIP} ({length:.3} s, hit at {hit:.3} s)",
                plan.seq, plan.swings
            );
            continue;
        };
        if !hammering.started {
            let node = match driver.node(BUILD_CLIP, &mut graphs) {
                Ok((node, _)) => node,
                Err(error) => {
                    ghosts.clear(&plan.uid);
                    status.fail("the build swing cannot play");
                    error!("[companion] unit {unit} build {}: {BUILD_CLIP}: {error}", plan.seq);
                    commands.entity(entity).remove::<(BuildPlan, Hammering)>();
                    continue;
                }
            };
            let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
                continue;
            };
            transitions
                .play(&mut animator, node, STATE_FADE)
                .set_repeat(RepeatAnimation::Forever);
            // The swing owns the avatar's animator (see [`super::walk::drive_clips`])
            // until the hammer is put down.
            gait.hammer_played = true;
            hammering.started = true;
            continue;
        }
        hammering.elapsed += time.delta_secs();
        let due = if hammering.elapsed < hammering.hit {
            0
        } else {
            (((hammering.elapsed - hammering.hit) / hammering.length).floor() as u32 + 1).min(plan.swings)
        };
        while hammering.hits < due {
            hammering.hits += 1;
            let alpha = plan.alpha(hammering.hits);
            ghosts.set(&plan.uid, alpha);
            // Where the body stands as it swings.
            let from = transform.translation;
            cues.write(BuildCue::Hit {
                unit,
                uid: plan.uid.clone(),
                hit: hammering.hits,
                of: plan.swings,
                at: plan.at,
                from,
            });
            info!(
                "[companion] unit {unit} build {}: hit {}/{} on fixture {}, {alpha:.2} solid",
                plan.seq, hammering.hits, plan.swings, plan.uid
            );
        }
        if hammering.elapsed >= plan.swings as f32 * hammering.length {
            ghosts.clear(&plan.uid);
            if let Some(HeldTool(tool)) = held {
                commands.entity(*tool).despawn();
            }
            commands.entity(entity).remove::<(BuildPlan, Hammering, HeldTool)>();
            cues.write(BuildCue::Built { unit, uid: plan.uid.clone() });
            // The clip driver takes the companion back to its idle.
            gait.hammer_played = false;
            status.state = CompanionState::Done;
            info!("[companion] unit {unit} build {}: fixture {} built", plan.seq, plan.uid);
        }
    }
}
