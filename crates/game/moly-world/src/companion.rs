//! The AI companion: a second visible character that is not the player.
//!
//! # Commands
//!
//! [`CompanionCommand`] is a message; write one per command, in the order
//! the companion should see them.
//!
//! ```text
//! Spawn   { unit: u32, at: Vec3 }              put a companion of `unit`'s character at `at`
//! WalkTo  { unit: u32, to: Vec3, within: f32 } walk it there on the navigation, within `within` m
//! Despawn { unit: u32 }                        remove it
//! ```
//!
//! There is one companion per unit.
//!
//! # Status
//!
//! Each companion entity carries a [`CompanionStatus`]:
//!
//! ```text
//! CompanionStatus { seq: u64, state: CompanionState }
//! CompanionState: Spawned | Idle | Walking | Done | Failed(String)
//! ```
//!
//! The spawn is `seq` 1 and `Spawned` while its body loads; then the
//! companion is shown as `Idle`. Every further accepted command takes the
//! next `seq`, so a caller that writes a command reads the status until it
//! sees a `seq` at or past its own: `Walking` and `Idle` are a walk in
//! flight, `Done` is the walk's arrival or the despawn, `Failed` is the
//! command's named refusal.
//!
//! A command about no companion has no status to carry its refusal; it is
//! logged as `[companion] … refused: <reason>`:
//!
//! ```text
//! spawn:   already spawned | no navigation on this site | unknown unit |
//!          character body <file> failed to load: <error> | rig profile <file> failed to load
//! walk:    not spawned | the body is still loading | no navigation on this site |
//!          the companion stands off the walk cells | the target is off the navigation cells |
//!          the navigation has no path to the target | the route has no waypoint beyond the
//!          companion | the walk stopped on the navigation before the target
//! despawn: not spawned
//! ```
//!
//! A spawn point off the walk cells seats on the nearest walkable point,
//! and the companion walks to the point asked for once its body is in.
//!
//! # The body
//!
//! The companion carries the body layer's avatar body
//! ([`crate::player_avatar::body::PlayerBody`], wired with `ProxyRest` and
//! [`AvatarDriver`] as the player's is), its own navigation state (the walk
//! state, path slot and route the NPCs use) and a [`Companion`] marker. It
//! carries neither `PlayerControlled` nor `AvatarRoot`: the player's input,
//! state machine, camera, audio and sky never see it.
//!
//! The character it wears is chosen per entity: the companion carries
//! [`crate::player_avatar::oc::OcChoice`], and once its avatar skeleton is in,
//! the character body of that unit covers it in place of the player's own
//! choice (see [`crate::player_avatar::oc::choice`]).
//!
//! Movement is the NPCs' navigation ([`crate::npc::companion`]): the walk
//! cells' path, the waypoint table and the crowd agent the roster walks
//! with. The clips are the avatar motion group's own: the idle clip while
//! standing and the walk clip while walking, crossfading as the player's
//! locomotion states do.

mod walk;

use walk::{drive_clips, plan_walk, report, route_outcome, step_motion};

use crate::npc::companion::{Motion, Watch};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::body::PlayerBody;
use crate::player_avatar::oc::OcChoice;
use bevy::asset::{AssetPath, LoadState};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use moly_assets::json::JsonAsset;
use std::time::Duration;

/// The walking speed of a companion, metres per second.
pub const WALK_SPEED: f32 = 1.5;
/// The largest `within` a walk accepts, metres.
pub const WITHIN_LIMIT: f32 = 5.0;
/// The period of the per-companion status line.
const REPORT_PERIOD: Duration = Duration::from_secs(5);
/// The longest step of a stalled frame, seconds: a companion never steps
/// further than the engine clock allows its NPCs.
const MAX_STEP: f32 = 0.1;

/// A command about the companion of `unit`.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CompanionCommand {
    /// Put a companion of character `unit` at `at`, facing +Z.
    Spawn { unit: u32, at: Vec3 },
    /// Walk the companion of `unit` to `to`, finishing within `within`
    /// metres of it.
    WalkTo { unit: u32, to: Vec3, within: f32 },
    /// Remove the companion of `unit`.
    Despawn { unit: u32 },
}

/// What a companion is doing, and how its last command ended.
///
/// `seq` counts the commands accepted for that companion from one; a spawn
/// is the first. Once a status carries the `seq` of the caller's command,
/// `state` is that command's outcome: `Idle` and `Walking` are a walk in
/// flight, `Done` its arrival or the despawn, `Failed` its named refusal.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct CompanionStatus {
    pub seq: u64,
    pub state: CompanionState,
}

impl CompanionStatus {
    /// The status a spawn starts: its first number, its body loading.
    pub fn spawned() -> Self {
        Self {
            seq: 1,
            state: CompanionState::Spawned,
        }
    }

    /// The next accepted command's number.
    fn accept(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    fn fail(&mut self, reason: &str) {
        self.state = CompanionState::Failed(reason.to_owned());
    }
}

/// The states of [`CompanionStatus`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompanionState {
    /// Accepted; the body is loading.
    Spawned,
    Idle,
    Walking,
    Done,
    Failed(String),
}

impl CompanionState {
    pub fn label(&self) -> &str {
        match self {
            Self::Spawned => "spawned",
            Self::Idle => "idle",
            Self::Walking => "walking",
            Self::Done => "done",
            Self::Failed(_) => "failed",
        }
    }
}

/// A companion body, by the unit whose character it wears.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Companion {
    pub unit: u32,
}

/// The character body of a spawn, while its files load. A spawn waiting for
/// its files carries this and not yet a [`Companion`].
#[derive(Component, Clone)]
pub struct CompanionSpawn {
    /// The unit whose companion is being spawned.
    pub unit: u32,
    at: Vec3,
    glb: String,
    rig_file: String,
    gltf: Handle<Gltf>,
    rig: Handle<JsonAsset>,
}

/// What a companion does once its body is wired into the site.
#[derive(Component, Clone, Copy)]
enum Pending {
    /// Show the body.
    Show,
    /// Walk from the spawn seat to the spawn point.
    ArriveAt(Vec3),
}

/// The walk runtime of one companion: its stuck watchdog, the motion the
/// last step ended with, and that walk's end once a step finds it.
#[derive(Component, Default)]
struct Walker {
    watch: Option<Watch>,
    motion: Option<Motion>,
    finished: Option<Motion>,
}

impl Walker {
    fn started() -> Self {
        Self {
            watch: None,
            motion: Some(Motion::Walking),
            finished: None,
        }
    }

    /// Whether the next step continues the walk.
    fn walks(&self) -> bool {
        self.motion == Some(Motion::Walking)
    }

    /// The end of this frame's step.
    fn end(&mut self, watch: Watch, motion: Motion) {
        self.watch = Some(watch);
        self.motion = Some(motion);
        if motion != Motion::Walking {
            self.finished = Some(motion);
        }
    }

    /// The end of this walk, once.
    fn take(&mut self) -> Option<Motion> {
        self.finished.take()
    }
}

/// The clips of one companion's animator, as they were last chosen, and the
/// speed of its last step.
#[derive(Component, Default)]
struct Gait {
    playing: Option<&'static str>,
    /// Metres per second in the last step that ran this frame.
    speed: f32,
    /// The frame it last stepped on.
    stepped: Option<u32>,
}

/// The companion domain: the body, its walk and its clips.
pub struct CompanionPlugin;

impl Plugin for CompanionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CompanionCommand>()
            // Commands and spawns run before the body layer's request; the
            // companions' bodies then go through the body layer's own steps
            // (registered with the player's), and the player's locomotion
            // driver brings every avatar animator up later in the same frame.
            .add_systems(
                Update,
                (receive, spawn)
                    .chain()
                    .before(crate::player_avatar::body::request),
            )
            .add_systems(
                Update,
                plan_walk
                    .after(crate::player_avatar::body::wire)
                    .after(spawn),
            )
            .add_systems(
                Update,
                (step_motion, route_outcome, drive_clips)
                    .chain()
                    .after(plan_walk),
            )
            .add_systems(Update, report.run_if(on_timer(REPORT_PERIOD)));
    }
}

/// The navigation faces a companion's routes are built on: the loaded
/// site's, when both faces are in and of the same navigation build. A site
/// change or a site move in flight has none.
fn faces<'a>(
    site: Option<&crate::site::SiteActive>,
    epoch: Option<&crate::site::GroundEpoch>,
    face: Option<&'a crate::walk_face::WalkFace>,
    objective: Option<&'a crate::npc_objective::ObjectiveFace>,
) -> Option<(&'a crate::walk_face::WalkFace, &'a crate::npc_objective::ObjectiveFace)> {
    site?;
    let epoch = epoch?;
    let (face, objective) = (face?, objective?);
    if !objective.is_fresh(epoch.0) || face.generation() != objective.navigation_generation() {
        return None;
    }
    Some((face, objective))
}

/// The character manifest's (model file, rig profile) of a unit.
fn manifest_row(
    server: &AssetServer,
    jsons: &Assets<JsonAsset>,
    unit: u32,
) -> Option<(String, String)> {
    let manifest = server.load::<JsonAsset>(moly_assets::character_manifest());
    let json = jsons.get(&manifest)?;
    let manifest = moly_assets::character::CharacterManifest::parse(&json.0).ok()?;
    let code = unit.checked_add(100)?.to_string();
    manifest
        .units
        .iter()
        .find(|row| row.unit == code)
        .map(|row| (row.glb.clone(), row.rig.clone()))
}

/// A yaw-only rotation from `from` towards `to`; `None` when they coincide.
fn turn_to(from: Vec3, to: Vec3) -> Option<Quat> {
    let delta = Vec3::new(to.x - from.x, 0.0, to.z - from.z);
    (delta.length_squared() >= 1e-8).then(|| Quat::from_rotation_y(delta.x.atan2(delta.z)))
}

/// An empty path slot: a companion that stands.
fn empty_path() -> PathSlot {
    PathSlot(moly_law::path::NpcPathWalkSlot::from_corners(Vec::new()))
}

/// Update: every command, in arrival order.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn receive(
    mut commands: Commands,
    mut requests: MessageReader<CompanionCommand>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    sweeps: Query<(), With<crate::field_modes::SiteMoveActive>>,
    spawns: Query<(Entity, &CompanionSpawn)>,
    mut companions: Query<(
        Entity,
        &Companion,
        &mut CompanionStatus,
        &mut Transform,
        &mut WalkState,
        &mut PathSlot,
        &mut RouteStops,
        &mut MotionPhase,
    )>,
) {
    // A spawn whose character file failed is refused by name: no body was
    // built to carry the refusal.
    for (entity, spawn) in &spawns {
        if let LoadState::Failed(error) = server.load_state(&spawn.gltf) {
            error!(
                "[companion] spawn of unit {} refused: character body {} failed to load: {error:#}",
                spawn.unit, spawn.glb
            );
            commands.entity(entity).despawn();
        }
    }
    for command in requests.read() {
        let unit = match command {
            CompanionCommand::Spawn { unit, .. }
            | CompanionCommand::WalkTo { unit, .. }
            | CompanionCommand::Despawn { unit } => *unit,
        };
        let spawned = companions.iter().any(|(_, body, ..)| body.unit == unit);
        if !spawned && !matches!(command, CompanionCommand::Spawn { .. }) {
            let what = match command {
                CompanionCommand::WalkTo { .. } => "walk",
                _ => "despawn",
            };
            error!("[companion] {what} of unit {unit} refused: not spawned");
            continue;
        }
        match command {
            CompanionCommand::Spawn { unit, at } => {
                // A spawn in flight counts as spawned.
                if spawned || spawns.iter().any(|(_, spawn)| spawn.unit == *unit) {
                    error!("[companion] spawn of unit {unit} refused: already spawned");
                    continue;
                }
                // A site move in flight keeps no walkable surface.
                if sweeps.iter().next().is_some()
                    || faces(
                        site.as_deref(),
                        epoch.as_deref(),
                        face.as_deref(),
                        objective.as_deref(),
                    )
                    .is_none()
                {
                    error!("[companion] spawn of unit {unit} refused: no navigation on this site");
                    continue;
                }
                let Some((glb, rig_file)) = manifest_row(&server, &jsons, *unit) else {
                    error!(
                        "[companion] spawn of unit {unit} refused: unknown unit (the character manifest has no body for it)"
                    );
                    continue;
                };
                // The scene is bound to the avatar skeleton, as a model
                // file's: no animations, lights or cameras; the rig profile
                // gives it the cast's materials.
                let gltf = server
                    .load_builder()
                    .with_settings(|settings: &mut bevy::gltf::GltfLoaderSettings| {
                        settings.load_materials = moly_assets::residency::GPU_ONLY;
                        settings.load_cameras = false;
                        settings.load_lights = false;
                        settings.load_animations = false;
                    })
                    .load(moly_assets::character_glb(&glb));
                let rig = server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig_file}")));
                info!("[companion] unit {unit}: character body {glb} requested at {at:?}");
                commands.spawn((
                    Name::new(format!("companion spawn unit {unit}")),
                    CompanionSpawn {
                        unit: *unit,
                        at: *at,
                        glb,
                        rig_file,
                        gltf,
                        rig,
                    },
                ));
            }
            CompanionCommand::WalkTo { unit, to, within } => {
                let Some((
                    entity,
                    _,
                    mut status,
                    mut transform,
                    mut walk,
                    mut slot,
                    mut route,
                    mut phase,
                )) = companions.iter_mut().find(|(_, body, ..)| body.unit == *unit)
                else {
                    continue;
                };
                if status.state == CompanionState::Spawned {
                    status.fail("the body is still loading");
                    error!(
                        "[companion] unit {unit} walk {} refused: the body is still loading",
                        status.seq
                    );
                    continue;
                }
                let seq = status.accept();
                let Some((face, objective)) = faces(
                    site.as_deref(),
                    epoch.as_deref(),
                    face.as_deref(),
                    objective.as_deref(),
                ) else {
                    status.fail("no navigation on this site");
                    error!(
                        "[companion] unit {unit} walk {seq} refused: no navigation on this site"
                    );
                    continue;
                };
                let within = within.clamp(0.0, WITHIN_LIMIT);
                info!(
                    "[companion] unit {unit} walk {seq}: to {to:?} within {within:.2}m from {:?}",
                    transform.translation
                );
                match crate::npc::companion::depart(
                    *unit,
                    &mut walk,
                    &mut slot,
                    &mut route,
                    face,
                    objective,
                    to.to_array(),
                    within,
                ) {
                    Ok(next) => {
                        *phase = next;
                        if let Some(turned) = turn_to(transform.translation, *to) {
                            transform.rotation = turned;
                        }
                        status.state = CompanionState::Walking;
                        commands.entity(entity).insert((
                            Walker::started(),
                        ));
                    }
                    Err(reason) => {
                        status.fail(reason);
                        error!("[companion] unit {unit} walk {seq} refused: {reason}");
                    }
                }
            }
            CompanionCommand::Despawn { unit } => {
                let Some((entity, _, mut status, ..)) =
                    companions.iter_mut().find(|(_, body, ..)| body.unit == *unit)
                else {
                    continue;
                };
                let seq = status.accept();
                status.state = CompanionState::Done;
                info!("[companion] unit {unit} despawn {seq}");
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Update: a spawn whose files are in becomes a companion: seated on the
/// walk cells, at the site's own height for that ground, hidden until its
/// body is wired.
fn spawn(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    face: Option<Res<crate::walk_face::WalkFace>>,
    objective: Option<Res<crate::npc_objective::ObjectiveFace>>,
    spawns: Query<(Entity, &CompanionSpawn)>,
) {
    let Some(face) = face.as_deref() else {
        return;
    };
    for (entity, spawn) in &spawns {
        if let LoadState::Failed(error) = server.load_state(&spawn.rig) {
            error!(
                "[companion] spawn of unit {} refused: rig profile {} failed to load: {error:#}",
                spawn.unit, spawn.rig_file
            );
            commands.entity(entity).despawn();
            continue;
        }
        if !server.is_loaded_with_dependencies(&spawn.gltf) || jsons.get(&spawn.rig).is_none() {
            continue;
        }
        // A spawn point off the walk cells seats on the nearest one, and the
        // companion walks there once its body is in.
        let (x, z) = face.seat(spawn.at.x, spawn.at.z);
        let seated = [spawn.at.x, spawn.at.z] != [x, z];
        let height = objective
            .as_deref()
            .and_then(|objective| objective.navigation_point_at([x, z]).map(|point| point[1]))
            .or_else(|| face.surface_height([x, z]))
            .unwrap_or(spawn.at.y);
        let seed = Vec3::new(x, height, z);
        let pending = if seated {
            Pending::ArriveAt(Vec3::new(spawn.at.x, height, spawn.at.z))
        } else {
            Pending::Show
        };
        commands.entity(entity).remove::<CompanionSpawn>().insert((
            Name::new(format!("companion unit {}", spawn.unit)),
            Companion { unit: spawn.unit },
            CompanionStatus::spawned(),
            MotionPhase::Dwelling { remaining: None },
            WalkState(moly_law::path::WalkState::new(seed.to_array(), [0.0, 0.0, 1.0])),
            empty_path(),
            RouteStops::default(),
            Gait::default(),
            Transform::from_translation(seed),
            Visibility::Hidden,
            PlayerBody::avatar(),
            OcChoice(spawn.unit),
            pending,
        ));
        info!(
            "[companion] unit {}: body at ({x:.2},{height:.2},{z:.2}){}",
            spawn.unit,
            if seated {
                "; the spawn point is off the walk cells and the companion walks to it"
            } else {
                ""
            }
        );
    }
}
