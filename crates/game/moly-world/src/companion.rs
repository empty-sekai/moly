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
//! Face    { unit: u32, to: Vec3, secs: f32 }   turn it to face `to` over `secs` (0 snaps)
//! Build   { unit: u32, uid: String, at: Vec3, swings: u32 }
//!                                              hammer the placed fixture `uid` (standing at `at`)
//!                                              from a ghost to solid in `swings` hits
//! Motion  { unit: u32, motion: String }        play the motion-library motion `motion` once
//!                                              on the body's own skeleton, then its idle
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
//! CompanionState: Spawned | Idle | Walking | Turning | Building | Acting | Done | Failed(String)
//! ```
//!
//! The spawn is `seq` 1 and `Spawned` while its body loads; then the
//! companion is shown as `Idle`. Every further accepted command takes the
//! next `seq`, so a caller that writes a command reads the status until it
//! sees a `seq` at or past its own: `Walking` and `Idle` are a walk in
//! flight, `Turning` a face's turn ([`face`]), `Building` a build's swings,
//! `Acting` a motion ([`act`]) on the body's own skeleton, `Done` is the
//! walk's arrival, the turn's end, the build's last hit, the motion's end or
//! the despawn, `Failed` is the command's named refusal.
//!
//! A face command turns the body on the spot, the yaw from the one it faces
//! to the point tweened linearly over the command's seconds; the walk state
//! follows the turn, as it does when a walk or a build turns the body.
//!
//! A build ([`build`]) walks the companion to a stand point beside the
//! fixture, then swings the harvest pickaxe clip with the hammer in its hand;
//! each hit makes the fixture's dither less transparent until it is drawn
//! solid. The fixture itself is put and saved by the caller first (the layout
//! editor's own put search and save); the companion only draws it as a ghost
//! and builds it. A walk command gives a build in flight up, and its fixture
//! is drawn solid.
//!
//! A motion ([`act`]) plays one motion of the shared motion library on the
//! companion's own body: the motion's start segment, its loop segment once
//! when the library has one, its end segment when it has one, then the
//! body's idle again. The companion stands while it plays.
//!
//! A command about no companion has no status to carry its refusal; it is
//! logged as `[companion] 闂?refused: <reason>`:
//!
//! ```text
//! spawn:   already spawned | no navigation on this site | unknown unit |
//!          character body <file> failed to load: <error> | rig profile <file> failed to load
//! walk:    not spawned | the body is still loading | a build is already in flight |
//!          no navigation on this site | the companion stands off the walk cells |
//!          the target is off the navigation cells | the navigation has no path to the target |
//!          the route has no waypoint beyond the companion | the walk stopped on the
//!          navigation before the target
//! face:    not spawned | the body is still loading | a build or a walk is already in flight |
//!          a motion is already in flight
//! motion:  not spawned | the body is still loading | a build or a turn is already in flight
//!          (a walk is stopped where it is) | a motion is already in flight |
//!          the body has no own motion | the motion library is not loaded |
//!          the motion library has no start segment <motion>_S | the navigation
//!          state is in use
//! build:   not spawned | the body is still loading | a build is already in flight |
//!          the navigation with the fixture in it was not baked | no walkable point beside the
//!          fixture | a walk's refusal on the way there | the motion group has no build swing |
//!          the build swing cannot play
//! despawn: not spawned
//! ```
//!
//! # Build cues
//!
//! A build writes a [`BuildCue`] on each of its hits and one when the
//! fixture is built: the fixture's position and the companion's at the hit.
//! They are for whoever plays the original's hit and put cues
//! (`moly_features::companion_cues` does in the game).
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

mod act;
pub mod build;
mod commands;
pub mod expression;
pub mod face;
mod own_motion;
pub mod use_fixture;
mod walk;

pub use build::FixtureGhosts;
pub use expression::CompanionFace;
pub use face::yaw_toward;
use walk::{drive_clips, plan_walk, report, route_outcome, step_motion, Source};

use crate::npc::companion::{Motion, Watch};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::body::PlayerBody;
use crate::player_avatar::oc::OcChoice;
use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use moly_assets::json::JsonAsset;
use std::time::Duration;

/// The largest `within` a walk accepts, metres.
pub const WITHIN_LIMIT: f32 = 5.0;
/// The period of the per-companion status line.
const REPORT_PERIOD: Duration = Duration::from_secs(5);
/// The longest step of a stalled frame, seconds: a companion never steps
/// further than the engine clock allows its NPCs.
const MAX_STEP: f32 = 0.1;

/// A command about the companion of `unit`.
#[derive(Message, Clone, PartialEq)]
pub enum CompanionCommand {
    /// Put a companion of character `unit` at `at`, facing +Z.
    Spawn { unit: u32, at: Vec3 },
    /// Walk the companion of `unit` to `to`, finishing within `within`
    /// metres of it.
    WalkTo { unit: u32, to: Vec3, within: f32 },
    /// Turn the companion of `unit` to face `to` over `secs` seconds; zero
    /// snaps it round (see [`face`]).
    Face { unit: u32, to: Vec3, secs: f32 },
    /// Remove the companion of `unit`.
    Despawn { unit: u32 },
    /// Build the fixture `uid` of the layout, standing at `at`: it stands
    /// as a ghost until the companion has walked beside it and hammered it
    /// `swings` times (see [`build`]).
    Build { unit: u32, uid: String, at: Vec3, swings: u32 },
    /// Play the motion-library motion `motion` once on the body's own
    /// skeleton, then its idle (see [`act`]).
    Motion { unit: u32, motion: String },
    /// Use the placed fixture `uid`: the character's own action
    /// there, the one named by `activity` and `variant` or the first (see
    /// [`use_fixture`]).
    Use { unit: u32, uid: String, activity: Option<i32>, variant: Option<u32> },
}

/// What a companion is doing, and how its last command ended.
///
/// `seq` counts the commands accepted for that companion from one; a spawn
/// is the first. Once a status carries the `seq` of the caller's command,
/// `state` is that command's outcome: `Idle` and `Walking` are a walk in
/// flight, `Turning` a turn, `Done` its arrival, the turn's end or the
/// despawn, `Failed` its named refusal.
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

    /// A command refused on arrival: it takes its number like an accepted
    /// one, so a caller polling that number reads the refusal.
    fn refuse(&mut self, reason: &str) {
        self.accept();
        self.fail(reason);
    }
}

/// The states of [`CompanionStatus`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompanionState {
    /// Accepted; the body is loading.
    Spawned,
    Idle,
    Walking,
    /// Turning to face a point.
    Turning,
    /// Hammering a build.
    Building,
    /// Playing a motion on the body's own skeleton.
    Acting,
    /// Playing a fixture's action on the body's own skeleton.
    Using,
    Done,
    Failed(String),
}

impl CompanionState {
    pub fn label(&self) -> &str {
        match self {
            Self::Spawned => "spawned",
            Self::Idle => "idle",
            Self::Walking => "walking",
            Self::Turning => "turning",
            Self::Building => "building",
            Self::Acting => "acting",
            Self::Using => "using",
            Self::Done => "done",
            Self::Failed(_) => "failed",
        }
    }
}

/// A step of a companion's build, for the cues the original plays with it.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum BuildCue {
    /// A hammer hit: the `hit`th of `of` on the fixture `uid`, which stands
    /// at `at`; `from` is the companion at the hit.
    Hit { unit: u32, uid: String, hit: u32, of: u32, at: Vec3, from: Vec3 },
    /// The build's last hit: the fixture `uid` is solid.
    Built { unit: u32, uid: String },
}

/// A companion body, by the unit whose character it wears.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Companion {
    pub unit: u32,
}

/// The character body of a spawn, while its files load. A spawn waiting for
/// its files carries this and not yet a [`Companion`].
#[derive(Component, Clone)]
pub(crate) struct CompanionSpawn {
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
pub(crate) struct Walker {
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
pub(crate) struct Gait {
    /// The source playing the body, if one has been chosen (see
    /// [`drive_clips`]).
    source: Option<Source>,
    /// The build swing owns the avatar's animator: its clip plays until
    /// the swing's end.
    hammer_played: bool,
    /// Metres per second in the last step that ran this frame.
    speed: f32,
    /// The frame it last stepped on.
    stepped: Option<u32>,
}

/// The frame's companion commands, in arrival order, taken effect before the
/// steps that follow them read the companions.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompanionCommands;

/// The frame's companion steps after the commands: the approach, the walk,
/// the build swing and the clip switch. A build's cues are written here.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompanionSteps;

/// The companion domain: the body, its walk and its clips.
pub struct CompanionPlugin;

impl Plugin for CompanionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CompanionCommand>()
            .add_message::<BuildCue>()
            .init_resource::<FixtureGhosts>()
            .init_resource::<crate::player_avatar::oc::own::OnceMotions>()
            // Companions stay on the site they were spawned on: they leave
            // with it and come back when it is shown again.
            .init_resource::<ParkedCompanions>()
            .add_systems(crate::site::SiteChangeTeardown, leave_site)
            .add_systems(
                Update,
                return_to_site
                    .run_if(resource_added::<crate::markers::SiteReady>)
                    .before(CompanionCommands),
            )
            // The commands, one system each, in arrival order; then the
            // spawns; all before the body layer's request. The companions'
            // bodies then go through the body layer's own steps (registered
            // with the player's), and the player's locomotion driver brings
            // every avatar animator up later in the same frame.
            .add_systems(
                Update,
                (
                    commands::spawn,
                    commands::walk,
                    commands::face_command,
                    commands::build_command,
                    commands::motion,
                    commands::use_command,
                    commands::despawn,
                    spawn,
                )
                    .chain()
                    .in_set(CompanionCommands)
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
                (build::approach, step_motion, route_outcome, build::swing, drive_clips)
                    .chain()
                    .after(plan_walk)
                    .after(CompanionCommands)
                    .in_set(CompanionSteps)
                    // The body layer's offer reads the asks and the one-shot
                    // rows these leave (the commands' rows land at the sync
                    // point the ordering puts between them).
                    .before(crate::player_avatar::oc::own::offer),
            )
            .add_systems(
                Update,
                face::step.after(step_motion).after(CompanionCommands),
            )
            // A use's selection, preparation and playback need the world:
            // the fixture activity provider and the shared timelines.
            .add_systems(Update, use_fixture::advance.after(CompanionSteps))
            // A motion's end follows the animator that played it, and comes
            // before this frame's commands.
            .add_systems(
                Update,
                act::finish
                    .after(bevy::app::AnimationSystems)
                    .before(CompanionCommands),
            )
            .add_systems(Update, build::apply_ghosts)
            .add_systems(Update, expression::apply)
            .add_systems(Update, report.run_if(on_timer(REPORT_PERIOD)));
    }
}

/// The navigation faces a companion's routes are built on: the loaded
/// site's, when both faces are in and of the same navigation build. A site
/// change or a site move in flight has none.
pub(crate) fn faces<'a>(
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
pub(crate) fn manifest_row(
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
            CompanionFace::default(),
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

/// Whether a companion of `unit` is being spawned: its command is read and
/// its character body is still loading.
pub fn is_spawning(world: &mut World, unit: u32) -> bool {
    world.query::<&CompanionSpawn>().iter(world).any(|spawn| spawn.unit == unit)
}

/// Companions kept for the site they stood on while another site is shown:
/// each one's unit, site and place.
#[derive(Resource, Default)]
pub struct ParkedCompanions(Vec<(u32, String, Vec3)>);

/// Site change teardown: a companion stays on the site it was spawned on.
/// It leaves with that site and is kept (its unit and place) for its return,
/// together with a spawn still loading; its balloon goes, and the ghosts of a
/// build it did not finish are cleared (the fixture is already in the saved
/// layout, and is drawn solid when the site is next shown). A caller polling
/// a command of a companion that left learns that it was removed.
fn leave_site(
    mut commands: Commands,
    site: Option<Res<crate::site::SiteSelection>>,
    mut parked: ResMut<ParkedCompanions>,
    companions: Query<(Entity, &Companion, &Transform)>,
    spawns: Query<(Entity, &CompanionSpawn)>,
    ghosts: Option<ResMut<FixtureGhosts>>,
    mut huds: MessageWriter<crate::npc_state::TweetHudEvent>,
) {
    let site = site.map(|site| site.site_type().to_owned());
    for (entity, companion, transform) in &companions {
        huds.write(crate::npc_state::TweetHudEvent::Retire { npc: entity });
        commands.entity(entity).despawn();
        match &site {
            Some(site) => {
                info!("[companion] unit {}: stays on {site}", companion.unit);
                parked.0.push((companion.unit, site.clone(), transform.translation));
            }
            None => info!("[companion] unit {}: removed with a site of no name", companion.unit),
        }
    }
    for (entity, spawn) in &spawns {
        commands.entity(entity).despawn();
        if let Some(site) = &site {
            info!("[companion] unit {}: its spawn stays on {site}", spawn.unit);
            parked.0.push((spawn.unit, site.clone(), spawn.at));
        }
    }
    if let Some(mut ghosts) = ghosts {
        ghosts.clear_all();
    }
}

/// Update, once a site is shown: the companions kept for it come back where
/// they stood.
fn return_to_site(
    site: Option<Res<crate::site::SiteSelection>>,
    mut parked: ResMut<ParkedCompanions>,
    mut requests: MessageWriter<CompanionCommand>,
) {
    let Some(site) = site else {
        return;
    };
    parked.0.retain(|(unit, stood_on, at)| {
        if stood_on != site.site_type() {
            return true;
        }
        info!("[companion] unit {unit}: back on {stood_on}");
        requests.write(CompanionCommand::Spawn { unit: *unit, at: *at });
        false
    });
}
