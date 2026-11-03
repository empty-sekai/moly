//! A stage set: a temporary room away from the user's own scene.
//!
//! A set moves to the requested site, publishes its furniture through the
//! ordinary fixture pipeline and adds the exact cast through the ordinary NPC
//! pipeline; the site's other characters are parked and hidden. Ending the set
//! returns through the ordinary site transition and restores every pose and
//! camera it found. No preview data is written to the editor or the settings
//! store.
//!
//! A caller arranges a set in steps of its own choosing. It requests one,
//! which is refused by name while another set is held. It runs
//! [`StageSet::drive`] every frame and reads [`StageSet::phase`] after it: the
//! set's own steps are the site switch, the layout and the cast, and the
//! timeouts. On `LoadingContent` the furniture is published and the caller
//! waits for the resources only it knows about, then supplies them with
//! [`StageSet::finish`]. The set then stands in `Ready` until
//! [`StageSet::restore`], or [`StageSet::fail`] by name, runs the way back.
//!
//! A set also loses itself when the furniture fails to load, when the room's
//! own surfaces fail to load, and when the assets do not arrive in time; a
//! caller that sees the resources it stages fail calls [`StageSet::fail`]
//! itself. A set that leaves always ends in `Returned` or `Failed`, in
//! bounded time: the way back gives up after the site timeout.
//!
//! [`StageSet::recast`] swaps a standing set's cast for the units of a new
//! choice in place, keeping its furniture, layout and site.

use std::fmt;

use bevy::prelude::*;

use crate::camera::{
    CameraSetting, CameraStateType, CameraTween, FieldCameraModel, FieldCameraState, FpsViewMemory,
    NormalCameraMemory,
};
use crate::character::{AvatarRoot, MotionDriver};
use crate::fixture::{
    self, EditableFixture, FixtureLayoutRevision, FixtureLoadFailure, FixturePlacements,
    FixtureScenesReady,
};
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::fixture_gimmick;
use crate::fixture_material::FixtureMaterialsSwapped;
use crate::lease::Holder;
use crate::npc::{self, CharacterUnitId, MotionPhase, NpcActions, WalkState};
use crate::npc_fixture_activity;
use crate::npc_objective::ObjectiveFace;
use crate::player::{PlayerControlled, PlayerInput};
use crate::player_fixture_action::PlayerFixtureRuntime;
use crate::player_talk;
use crate::room_appearance::{RoomAppearance, RoomAppearanceState};
use crate::site::{
    GroundEpoch, SiteChangeRequest, SiteScenesReady, SiteSelection, TemporarySiteActive,
    TemporarySiteChangeRequest,
};
use crate::site_material::SiteMaterialsSwapped;
use crate::talk::{self, TalkHold};
use crate::walk_face::WalkFace;

const SITE_TIMEOUT: f32 = 90.;
const ASSET_TIMEOUT: f32 = 120.;

/// The site a set is built on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetSite {
    /// The home site's courtyard: furniture and character actions belong
    /// there rather than in an unrelated indoor room.
    Courtyard,
    /// The ground floor room, for content that needs a real room surface.
    Room,
}

impl SetSite {
    /// The site type the site pipeline knows this site by.
    pub fn site_type(self) -> &'static str {
        match self {
            Self::Courtyard => "home_site",
            Self::Room => "first_floor",
        }
    }
}

/// A request for a set.
#[derive(Clone, Debug)]
pub struct SetRequest {
    /// Who arranges the set.
    pub requester: Holder,
    pub site: SetSite,
    /// The characters the set shows, kept even when they are already there.
    pub units: Vec<u32>,
    /// The furniture rows published through the ordinary layout pipeline.
    pub fixtures: Vec<EditableFixture>,
}

/// What the caller supplies when it finishes arranging the set.
#[derive(Default)]
pub struct Content {
    /// The room surfaces the caller stages; the set keeps and restores the
    /// room's own.
    pub appearance: Option<RoomAppearance>,
    /// Whether the caller itself holds characters on the set (its staged
    /// poses, say). The way back waits for those holds to end.
    pub held: bool,
}

/// The first placed instance of each of `rows`, by UID: the instance of that
/// row on the current site.
pub fn first_instance_of_each(world: &mut World, rows: &[EditableFixture]) -> Vec<Entity> {
    let found: Vec<_> = world
        .query::<(Entity, &FixtureActivityIdentity)>()
        .iter(world)
        .map(|(entity, identity)| (identity.uid.clone(), identity.master_id, entity))
        .collect();
    rows.iter()
        .filter_map(|row| {
            found
                .iter()
                .filter(|(uid, id, _)| uid == &row.uid && *id == row.fixture_id)
                .map(|(_, _, entity)| *entity)
                .min()
        })
        .collect()
}

/// Where a set is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetPhase {
    /// Moving to the requested site, or waiting for the current one to settle.
    SwitchingSite,
    /// The furniture is published; the caller arranges what it stages.
    LoadingContent,
    /// The set stands and the caller's content may play.
    Ready,
    /// Running the caller's steps before the way back.
    Cleaning,
    /// Walking back to the original site.
    Returning,
    /// Back at the original site, the poses restored.
    Returned,
    /// The end of a set that lost itself, the original scene restored.
    Failed(String),
}

impl SetPhase {
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
    /// Whether this phase keeps the set held.
    pub fn is_running(&self) -> bool {
        !matches!(self, Self::Returned | Self::Failed(_))
    }
}

impl fmt::Display for SetPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SwitchingSite => f.write_str("SwitchingSite"),
            Self::LoadingContent => f.write_str("LoadingContent"),
            Self::Ready => f.write_str("Ready"),
            Self::Cleaning => f.write_str("Cleaning"),
            Self::Returning => f.write_str("Returning"),
            Self::Returned => f.write_str("Returned"),
            Self::Failed(_) => f.write_str("Failed"),
        }
    }
}

/// A set's bookkeeping, readable by any system.
#[derive(Resource, Debug)]
pub struct SetSession {
    ticket: u64,
    requester: Holder,
    original_site: String,
    destination: &'static str,
    units: Vec<u32>,
    rows: Vec<EditableFixture>,
    phase: SetPhase,
    temporary_actors: Vec<Entity>,
    original_player_position: Option<[f32; 3]>,
    original_actor_count: usize,
}

impl SetSession {
    pub fn ticket(&self) -> u64 {
        self.ticket
    }
    pub fn requester(&self) -> &Holder {
        &self.requester
    }
    pub fn original_site(&self) -> &str {
        &self.original_site
    }
    pub fn destination(&self) -> &str {
        self.destination
    }
    pub fn units(&self) -> &[u32] {
        &self.units
    }
    pub fn rows(&self) -> &[EditableFixture] {
        &self.rows
    }
    pub fn phase(&self) -> &SetPhase {
        &self.phase
    }
    pub fn temporary_actors(&self) -> &[Entity] {
        &self.temporary_actors
    }
    pub fn original_player_position(&self) -> Option<[f32; 3]> {
        self.original_player_position
    }
    /// The characters of the original scene the set restores.
    pub fn original_actor_count(&self) -> usize {
        self.original_actor_count
    }
    /// The site the set's furniture was placed on: its destination, or the
    /// original site when that is already the destination.
    pub fn site_type(&self) -> &str {
        if self.original_site == self.destination {
            &self.original_site
        } else {
            self.destination
        }
    }
}

struct ParkedActor {
    entity: Entity,
    visibility: Visibility,
}

#[derive(Clone, Copy)]
struct Pose {
    entity: Entity,
    transform: Transform,
}

/// The set held now, readable by observers through [`SetState::session`].
#[derive(Resource)]
pub struct SetState {
    session: SetSession,
    departure_epoch: u64,
    return_epoch: u64,
    elapsed: f32,
    parked_actors: Vec<ParkedActor>,
    original_player: Option<Pose>,
    original_actors: Vec<Pose>,
    original_cameras: Vec<Pose>,
    original_camera_state: Option<CameraSnapshot>,
    original_appearance: Option<RoomAppearance>,
    failure: Option<String>,
    /// The set ended where it stood: the way back needs no new site.
    never_left: bool,
}

impl SetState {
    /// The set while it is held; once it has ended, no set is held.
    pub fn session(&self) -> Option<&SetSession> {
        self.session.phase.is_running().then_some(&self.session)
    }
}

/// The camera state a set found: transforms, lens, the camera's own state and
/// its memories, restored when the set ends.
#[derive(Clone)]
struct CameraSnapshot {
    model: Option<FieldCameraModel>,
    state: Option<FieldCameraState>,
    first_person: Option<FpsViewMemory>,
    normal: Option<NormalCameraMemory>,
    tween: Option<CameraTween>,
    projections: Vec<(Entity, Projection)>,
    visibility: Vec<(Entity, Visibility)>,
}

impl CameraSnapshot {
    fn capture(world: &mut World) -> Self {
        Self {
            model: world.get_resource::<FieldCameraModel>().cloned(),
            state: world.get_resource::<FieldCameraState>().copied(),
            first_person: world.get_resource::<FpsViewMemory>().copied(),
            normal: world.get_resource::<NormalCameraMemory>().cloned(),
            tween: world.get_resource::<CameraTween>().copied(),
            projections: world
                .query_filtered::<(Entity, &Projection), With<Camera3d>>()
                .iter(world)
                .map(|(entity, projection)| (entity, projection.clone()))
                .collect(),
            visibility: world
                .query_filtered::<(Entity, &Visibility), With<AvatarRoot>>()
                .iter(world)
                .map(|(entity, visibility)| (entity, *visibility))
                .collect(),
        }
    }
    /// The temporary room starts in the normal third-person view even when
    /// the original scene was first person. The snapshot keeps the original
    /// lens, FPS memory and avatar visibility for the way back.
    fn prepare_temporary(world: &mut World) {
        world.insert_resource(FieldCameraState(CameraStateType::Normal));
        world.remove_resource::<CameraTween>();
        world.remove_resource::<NormalCameraMemory>();
        let fov = world
            .get_resource::<CameraSetting>()
            .map(|setting| setting.fov.to_radians());
        if let Some(fov) = fov {
            for mut projection in world
                .query_filtered::<&mut Projection, With<Camera3d>>()
                .iter_mut(world)
            {
                if let Projection::Perspective(projection) = &mut *projection {
                    projection.fov = fov;
                }
            }
        }
        for mut visibility in world
            .query_filtered::<&mut Visibility, With<AvatarRoot>>()
            .iter_mut(world)
        {
            visibility.set_if_neq(Visibility::Visible);
        }
    }
    fn restore(&self, world: &mut World) {
        replace(world, self.model.clone());
        replace(world, self.state);
        replace(world, self.first_person);
        replace(world, self.normal.clone());
        // A reset begun inside the set is a camera tween too; restoring the
        // saved tween (or its absence) replaces it.
        replace(world, self.tween);
        for (entity, projection) in &self.projections {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                entity.insert(projection.clone());
            }
        }
        for (entity, visibility) in &self.visibility {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                entity.insert(*visibility);
            }
        }
    }
}

fn replace<T: Resource>(world: &mut World, value: Option<T>) {
    if let Some(value) = value {
        world.insert_resource(value);
    } else {
        world.remove_resource::<T>();
    }
}

/// Installs the stage set service: its bookkeeping and a frame clock, so a
/// set keeps its timeouts when the schedule has no `Time` of its own.
pub fn install(app: &mut App) {
    app.init_resource::<StageSet>();
    if !app.world().contains_resource::<Time>() {
        app.init_resource::<Time>();
    }
}

/// Whether the ordinary owners a set displaces are all idle. A caller whose
/// staged poses put actors under a hold adds that hold as `held`, as the
/// content library's preview does.
pub fn owners_idle(world: &World, held: bool) -> bool {
    !held
        && !npc_fixture_activity::preview::active(world)
        && !world.contains_resource::<player_talk::PlayerTalkSession>()
        && !world.contains_resource::<talk::ActiveTalk>()
        && !world.resource::<PlayerFixtureRuntime>().active()
        && !fixture_gimmick::session::blocks_replacement(
            world.resource::<fixture_gimmick::Gimmicks>(),
        )
}

fn origin(world: &World) -> (u64, bool) {
    let current_epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
    let settled = current_epoch.is_some() && world.contains_resource::<SiteScenesReady>();
    (current_epoch.unwrap_or(0), settled)
}

fn generation_ready(
    original_site: &str,
    destination: &str,
    departure_epoch: u64,
    current_epoch: Option<u64>,
) -> bool {
    current_epoch.is_some_and(|current| {
        if original_site == destination {
            current >= departure_epoch.max(1)
        } else {
            current > departure_epoch
        }
    })
}

fn actors_ready(world: &mut World, required: &[u32]) -> bool {
    let rows: Vec<_> = world
        .query::<(Entity, &CharacterUnitId, &NpcActions)>()
        .iter(world)
        .map(|(entity, unit, actions)| (entity, unit.0, actions.ready()))
        .collect();
    required.iter().all(|unit| {
        rows.iter().any(|(entity, id, ready)| {
            id == unit && *ready && world.get::<MotionDriver>(*entity).is_some()
        })
    })
}

fn reserve_required_actors(world: &mut World, required: &[u32]) {
    let actors: Vec<_> = world
        .query::<(Entity, &CharacterUnitId)>()
        .iter(world)
        .filter(|(_, unit)| required.contains(&unit.0))
        .map(|(entity, _)| entity)
        .collect();
    for entity in actors {
        npc::stop_for_external_activity(world, entity);
    }
}

fn park_background_actors(world: &mut World, required: &[u32]) -> Vec<ParkedActor> {
    let actors: Vec<_> = world
        .query_filtered::<(Entity, &CharacterUnitId, &Visibility), Without<PlayerControlled>>()
        .iter(world)
        .filter(|(_, unit, _)| !required.contains(&unit.0))
        .map(|(entity, _, visibility)| (entity, *visibility))
        .collect();
    for (entity, _) in &actors {
        npc::stop_for_external_activity(world, *entity);
        world
            .entity_mut(*entity)
            .insert((TalkHold, Visibility::Hidden));
    }
    actors
        .into_iter()
        .map(|(entity, visibility)| ParkedActor { entity, visibility })
        .collect()
}

fn restore_background_actors(world: &mut World, parked: &[ParkedActor]) {
    for actor in parked {
        if let Ok(mut entity) = world.get_entity_mut(actor.entity) {
            entity.remove::<TalkHold>();
            entity.insert(actor.visibility);
        }
    }
}

fn restore_original_poses(world: &mut World, state: &SetState) {
    if let Some(camera) = &state.original_camera_state {
        camera.restore(world);
    }
    if let Some(Pose { entity, transform }) = state.original_player {
        if let Ok(mut player) = world.get_entity_mut(entity) {
            player.insert((
                transform,
                GlobalTransform::from(transform),
                PlayerInput::default(),
                MotionPhase::Dwelling { remaining: None },
            ));
        }
    }
    for Pose { entity, transform } in &state.original_actors {
        if world.get_entity(*entity).is_err() {
            continue;
        }
        world.entity_mut(*entity).insert(*transform);
        if let Some(mut walk) = world.get_mut::<WalkState>(*entity) {
            walk.0.position = transform.translation.to_array();
            walk.0.forward = (transform.rotation * Vec3::Z).to_array();
        }
    }
    for Pose { entity, transform } in &state.original_cameras {
        if let Ok(mut camera) = world.get_entity_mut(*entity) {
            camera.insert(*transform);
        }
    }
}

/// Swaps the set's cast for `units` in place, keeping its furniture.
fn recast(state: &mut SetState, world: &mut World, units: Vec<u32>) -> Result<(), String> {
    if !state.session.phase.is_running() {
        return Err("独立场景已经结束，请重新选择".into());
    }
    restore_background_actors(world, &state.parked_actors);
    state.parked_actors = park_background_actors(world, &units);
    let added = npc::spawn_temporary_units(world, &units)?;
    state.session.temporary_actors.extend(added);
    state.session.units = units;
    state.session.phase = SetPhase::LoadingContent;
    state.elapsed = 0.;
    info!(
        "[stage-set] set {} re-cast for {} units",
        state.session.ticket,
        state.session.units.len()
    );
    Ok(())
}

/// Leaves the live site for the way back: the set ends `Failed` with
/// `reason`, or `Returned` without one.
fn leave(state: &mut SetState, reason: Option<String>) {
    if !matches!(
        state.session.phase,
        SetPhase::Cleaning | SetPhase::Returning
    ) {
        let phase = std::mem::replace(&mut state.session.phase, SetPhase::Cleaning);
        info!(
            "[stage-set] set {} leaves {phase:?}: {}",
            state.session.ticket,
            reason.as_deref().unwrap_or("场景已结束")
        );
    }
    if reason.is_some() {
        state.failure = reason;
    }
    state.elapsed = 0.;
}

/// Ends the caller's staging: the parked characters, the cast it spawned,
/// the room surfaces, and the temporary layout if the set never left.
fn end_of_cleaning(world: &mut World, state: &mut SetState) {
    restore_background_actors(world, &state.parked_actors);
    state.parked_actors.clear();
    npc::remove_temporary_units(world, &state.session.temporary_actors);
    state.session.temporary_actors.clear();
    // Withdraw an unconsumed preview before either path: a set called off
    // before the switch is read must not switch later.
    world.remove_resource::<TemporarySiteChangeRequest>();
    if let Some(appearance) = state.original_appearance.take() {
        world.insert_resource(appearance);
    }
    let never_left = world
        .get_resource::<SiteSelection>()
        .is_some_and(|selection| selection.site_type() == state.session.original_site)
        && world
            .get_resource::<GroundEpoch>()
            .is_some_and(|epoch| epoch.0 == state.departure_epoch)
        && !world.contains_resource::<TemporarySiteActive>()
        && !world.contains_resource::<fixture::TemporaryFixtureLayout>();
    state.never_left = never_left;
    if never_left {
        restore_original_poses(world, state);
        state.session.phase = match state.failure.take() {
            Some(reason) => SetPhase::Failed(reason),
            None => SetPhase::Returned,
        };
        info!(
            "[stage-set] set {} ended where it stood; the original scene is untouched",
            state.session.ticket
        );
        return;
    }
    state.return_epoch = world
        .get_resource::<GroundEpoch>()
        .map(|epoch| epoch.0)
        .unwrap_or(0);
    world.remove_resource::<TemporarySiteChangeRequest>();
    world.insert_resource(SiteChangeRequest(state.session.original_site.clone()));
    state.session.phase = SetPhase::Returning;
    state.elapsed = 0.;
    info!(
        "[stage-set] set {} returns to {}",
        state.session.ticket, state.session.original_site
    );
}

fn begin(world: &mut World, request: SetRequest) -> Result<(), String> {
    // A set may be requested while the current site is still streaming. Do
    // not spend a minute finishing a scene that is immediately discarded:
    // epoch 0 means "no settled origin yet" and the destination's first
    // settled generation advances to 1.
    let (epoch, origin_settled) = origin(world);
    let original_site = world.resource::<SiteSelection>().site_type().to_owned();
    let destination = request.site.site_type();
    // A half-loaded origin has no authoritative poses yet. In that case the
    // normal site pipeline reseeds the player, actors and camera on return.
    let original_player = origin_settled
        .then(|| {
            world
                .query_filtered::<(Entity, &Transform), With<PlayerControlled>>()
                .iter(world)
                .next()
                .map(|(entity, transform)| Pose { entity, transform: *transform })
        })
        .flatten();
    let original_actors = if origin_settled {
        world
            .query_filtered::<(Entity, &Transform), (With<CharacterUnitId>, Without<PlayerControlled>)>()
            .iter(world)
            .map(|(entity, transform)| Pose { entity, transform: *transform })
            .collect()
    } else {
        Vec::new()
    };
    let original_camera_state = origin_settled.then(|| CameraSnapshot::capture(world));
    let original_cameras = if origin_settled {
        world
            .query_filtered::<(Entity, &Transform), With<Camera3d>>()
            .iter(world)
            .map(|(entity, transform)| Pose { entity, transform: *transform })
            .collect()
    } else {
        Vec::new()
    };
    CameraSnapshot::prepare_temporary(world);
    // A set on the site it stands on lays its furniture over the current
    // generation: a change to the same site would reload the site's own
    // layout over the temporary one.
    if original_site != destination {
        world.insert_resource(TemporarySiteChangeRequest(destination.to_owned()));
    } else {
        info!("[stage-set] the set reuses the current site {destination} while it settles");
    }
    let original_appearance = world.get_resource::<RoomAppearance>().cloned();
    info!(
        "[stage-set] {} requests a set on {} for {destination} ({} units, {} fixtures)",
        request.requester.id,
        original_site,
        request.units.len(),
        request.fixtures.len()
    );
    let session = SetSession {
        ticket: 0,
        requester: request.requester,
        original_site,
        destination,
        units: request.units,
        rows: request.fixtures,
        phase: SetPhase::SwitchingSite,
        temporary_actors: Vec::new(),
        original_player_position: original_player.map(|pose| pose.transform.translation.to_array()),
        original_actor_count: original_actors.len(),
    };
    world.insert_resource(SetState {
        session,
        departure_epoch: epoch,
        return_epoch: 0,
        elapsed: 0.,
        parked_actors: Vec::new(),
        original_player,
        original_actors,
        original_cameras,
        original_camera_state,
        original_appearance,
        failure: None,
        never_left: false,
    });
    Ok(())
}

/// The set's own steps for one frame. A set driven without its caller's
/// content stands still: a caller that stops driving stops the set.
fn advance(world: &mut World, state: &mut SetState, content: Option<Content>) {
    if content.is_none() {
        return;
    }
    state.elapsed += world.resource::<Time>().delta_secs();
    let phase = state.session.phase.clone();
    match phase {
        SetPhase::SwitchingSite => {
            let selection_ready = world
                .get_resource::<SiteSelection>()
                .is_some_and(|selection| selection.site_type() == state.session.destination);
            let epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
            let site_ready = generation_ready(
                &state.session.original_site,
                state.session.destination,
                state.departure_epoch,
                epoch,
            ) && world.contains_resource::<SiteScenesReady>()
                && world.contains_resource::<SiteMaterialsSwapped>()
                && world
                    .get_resource::<FixturePlacements>()
                    .is_some_and(|layout| layout.site_type() == state.session.destination);
            if selection_ready && site_ready {
                match fixture::install_temporary_layout(world, &state.session.rows) {
                    Ok(()) => {
                        state.parked_actors = park_background_actors(world, &state.session.units);
                        state.session.phase = SetPhase::LoadingContent;
                        state.elapsed = 0.;
                    }
                    Err(reason) => leave(state, Some(reason)),
                }
            } else if state.elapsed >= SITE_TIMEOUT {
                leave(state, Some("无法在限定时间内准备独立场景".into()));
            }
        }
        SetPhase::LoadingContent => {
            if let Some(failure) = world.get_resource::<FixtureLoadFailure>() {
                leave(state, Some(failure.0.clone()));
            } else if let Some(error) = room_error(world) {
                leave(state, Some(error));
            }
            if matches!(state.session.phase, SetPhase::LoadingContent) {
                settle(world, state, content);
            }
        }
        SetPhase::Ready => {}
        SetPhase::Cleaning => {
            if owners_idle(world, content.is_some_and(|content| content.held)) {
                end_of_cleaning(world, state);
            }
        }
        SetPhase::Returning => {
            let epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
            // A set that never left its site waits for no new generation.
            let back = world
                .get_resource::<SiteSelection>()
                .is_some_and(|selection| selection.site_type() == state.session.original_site)
                && world.contains_resource::<SiteScenesReady>();
            let restored = back
                && epoch.is_some_and(|epoch| {
                    epoch > state.return_epoch || (state.never_left && epoch == state.return_epoch)
                })
                && world.get_resource::<ObjectiveFace>().is_some_and(|face| {
                    epoch.is_some_and(|epoch| face.is_fresh(epoch))
                });
            if let (Some(epoch), true) = (epoch, restored) {
                restore_original_poses(world, state);
                state.session.phase = match state.failure.take() {
                    Some(reason) => SetPhase::Failed(reason),
                    None => SetPhase::Returned,
                };
                info!(
                    "[stage-set] set {} back on {} at epoch {epoch}",
                    state.session.ticket, state.session.original_site
                );
            } else if state.elapsed >= SITE_TIMEOUT {
                // The way back gave up: the set still ends, answered by name.
                let reason = state
                    .failure
                    .take()
                    .unwrap_or_else(|| "返回原场景仍在等待资源；不会保存临时布局".to_owned());
                info!(
                    "[stage-set] set {} could not return to {}: {reason}",
                    state.session.ticket, state.session.original_site
                );
                state.session.phase = SetPhase::Failed(reason);
            }
        }
        SetPhase::Returned | SetPhase::Failed(_) => {}
    }
}

/// Whether the room already wears `appearance`: the site's own surfaces, or
/// the ones the caller staged on an earlier frame.
fn already_worn(world: &World, appearance: &RoomAppearance) -> bool {
    world
        .get_resource::<RoomAppearance>()
        .is_some_and(|current| current.wall == appearance.wall && current.floor == appearance.floor)
}

/// A room site's own surface error.
fn room_error(world: &World) -> Option<String> {
    world
        .get_resource::<SiteSelection>()
        .filter(|selection| selection.is_room())
        .and_then(|_| world.resource::<RoomAppearanceState>().error.clone())
}

/// Moves `LoadingContent` on to `Ready` once the room, the published
/// furniture and the exact cast are all ready, the caller's content
/// supplied.
fn settle(world: &mut World, state: &mut SetState, content: Option<Content>) {
    let Some(content) = content else {
        return;
    };
    if let Some(appearance) = content.appearance {
        if !already_worn(world, &appearance) {
            // The first surface a set wears is the one it restores.
            if state.original_appearance.is_none() {
                state.original_appearance = world.get_resource::<RoomAppearance>().cloned();
            }
            world.insert_resource(appearance);
        }
    }
    if world.resource::<SiteSelection>().is_room()
        && !world.resource::<RoomAppearanceState>().ready
    {
        return;
    }
    let ids = fixture_ids(&state.session.rows);
    let controller_ready = match fixture_gimmick::catalog_stream::ready_for(world, &ids) {
        Ok(ready) => ready,
        Err(reason) => {
            leave(state, Some(reason));
            return;
        }
    };
    if scene_ready(world) {
        if state.session.temporary_actors.is_empty() {
            match npc::spawn_temporary_units(world, &state.session.units) {
                Ok(actors) => state.session.temporary_actors = actors,
                Err(reason) => {
                    leave(state, Some(reason));
                    return;
                }
            }
        }
        reserve_required_actors(world, &state.session.units);
        if actors_ready(world, &state.session.units) && controller_ready {
            state.session.phase = SetPhase::Ready;
            state.elapsed = 0.;
            info!(
                "[stage-set] set {} stands on {} ({} units, {} fixtures)",
                state.session.ticket,
                state.session.destination,
                state.session.units.len(),
                state.session.rows.len()
            );
            return;
        }
    }
    if state.elapsed >= ASSET_TIMEOUT {
        leave(state, Some("所需角色或家具没有在限定时间内准备完成".into()));
    }
}

fn scene_ready(world: &World) -> bool {
    world.contains_resource::<SiteMaterialsSwapped>()
        && world.get_resource::<GroundEpoch>().is_some_and(|epoch| {
            world.contains_resource::<FixtureScenesReady>()
                && world.contains_resource::<FixtureMaterialsSwapped>()
                && world.get_resource::<ObjectiveFace>().is_some_and(|face| {
                    face.is_fresh(epoch.0)
                        && world
                            .get_resource::<WalkFace>()
                            .is_some_and(|navigation| {
                                navigation.layout_revision()
                                    == world.resource::<FixtureLayoutRevision>().0
                                    && face.navigation_generation() == navigation.generation()
                            })
                })
        })
}

fn fixture_ids(rows: &[EditableFixture]) -> Vec<i32> {
    let mut ids: Vec<i32> = rows.iter().map(|row| row.fixture_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_origin_allows_pre_settle_switch_without_snapshotting_partial_state() {
        let mut world = World::new();
        assert_eq!(origin(&world), (0, false));

        world.insert_resource(SiteScenesReady);
        assert_eq!(origin(&world), (0, false));

        world.insert_resource(GroundEpoch(4));
        assert_eq!(origin(&world), (4, true));

        world.remove_resource::<SiteScenesReady>();
        assert_eq!(origin(&world), (4, false));
    }

    #[test]
    fn same_site_independent_preview_reuses_the_current_generation() {
        assert!(generation_ready("home_site", "home_site", 4, Some(4)));
        assert!(generation_ready("home_site", "home_site", 0, Some(1)));
        assert!(!generation_ready("home_site", "home_site", 0, None));
    }

    #[test]
    fn cross_site_independent_preview_requires_a_new_generation() {
        assert!(!generation_ready("grassland", "home_site", 4, Some(4)));
        assert!(generation_ready("grassland", "home_site", 4, Some(5)));
    }
}

/// The stage set service: one set at a time.
#[derive(Resource, Default)]
pub struct StageSet {
    next_ticket: u64,
}

impl StageSet {
    /// Arranges a set for `request` and answers its ticket. Refused by name
    /// while another set runs; to arrange a different set on the same site, a
    /// caller restores this one and waits for it to end.
    pub fn request(world: &mut World, request: SetRequest) -> Result<u64, String> {
        if let Some(session) = world.get_resource::<SetState>().and_then(SetState::session) {
            return Err(format!(
                "另一个场景（{} 的 {}）正在准备，请稍候",
                session.requester.id, session.ticket
            ));
        }
        begin(world, request)?;
        let ticket = {
            let mut set = world.resource_mut::<StageSet>();
            set.next_ticket = set.next_ticket.wrapping_add(1).max(1);
            set.next_ticket
        };
        world.resource_mut::<SetState>().session.ticket = ticket;
        Ok(ticket)
    }

    /// Re-casts the set with `ticket` for a new choice: its fixtures, layout
    /// and site stay; the characters it added before are removed, its
    /// background characters are restored and parked again for `units`, and
    /// the units not yet present are spawned. The set goes back to
    /// `LoadingContent` under the same ticket. `false` when `ticket` no
    /// longer names a running set.
    pub fn recast(world: &mut World, ticket: u64, units: Vec<u32>) -> Result<(), String> {
        let Some(mut state) = Self::held(world, ticket) else {
            return Err("独立场景已经结束，请重新选择".into());
        };
        let outcome = recast(&mut state, world, units);
        world.insert_resource(state);
        outcome
    }

    /// The set held now, if any.
    pub fn session(world: &World) -> Option<&SetSession> {
        world.get_resource::<SetState>()?.session()
    }

    /// Where the set of `ticket` is; `None` when no set holds it.
    pub fn phase(world: &World, ticket: u64) -> Option<&SetPhase> {
        world
            .get_resource::<SetState>()
            .filter(|state| state.session.ticket == ticket)
            .map(|state| &state.session.phase)
    }

    /// Supplies what only the caller stages, as the set's steps for this
    /// frame: `true` once the room, the published furniture and the exact
    /// cast are all ready.
    pub fn finish(world: &mut World, ticket: u64, content: Content) -> bool {
        Self::drive(world, ticket, Some(content));
        Self::phase(world, ticket) == Some(&SetPhase::Ready)
    }

    /// Runs the set's steps for this frame, `content` supplied for the
    /// caller's arrangement. Without it the set stands still and the way
    /// back never starts: a caller that stops driving stops the set.
    pub fn drive(world: &mut World, ticket: u64, content: Option<Content>) {
        if let Some(mut state) = Self::held(world, ticket) {
            advance(world, &mut state, content);
            world.insert_resource(state);
        }
    }

    /// Starts the way back: the set removes what it added once the owners
    /// of the caller's content are idle, returns through the ordinary site
    /// transition, and restores the poses. `false` when the ticket no longer
    /// names a running set.
    pub fn restore(world: &mut World, ticket: u64) -> bool {
        Self::call_off(world, ticket, None)
    }

    /// Ends the set by name: the same way back, answered by
    /// [`SetPhase::Failed`] with `reason` once the original scene is back.
    pub fn fail(world: &mut World, ticket: u64, reason: impl Into<String>) -> bool {
        Self::call_off(world, ticket, Some(reason.into()))
    }

    /// Forgets the set of `ticket` once it has ended.
    pub fn discard(world: &mut World, ticket: u64) {
        if world
            .get_resource::<SetState>()
            .is_some_and(|state| state.session.ticket == ticket && !state.session.phase.is_running())
        {
            world.remove_resource::<SetState>();
        }
    }

    fn held(world: &mut World, ticket: u64) -> Option<SetState> {
        if !Self::phase(world, ticket)
            .is_some_and(|phase| phase.is_running())
        {
            return None;
        }
        world.remove_resource::<SetState>()
    }

    fn call_off(world: &mut World, ticket: u64, reason: Option<String>) -> bool {
        let ended = Self::phase(world, ticket).is_some_and(|phase| !phase.is_running());
        let Some(mut state) = Self::held(world, ticket) else {
            return ended;
        };
        // A set already on its way back keeps the reason it left with,
        // failing only if it had none.
        if matches!(
            state.session.phase,
            SetPhase::SwitchingSite | SetPhase::LoadingContent | SetPhase::Ready
        ) || state.failure.is_none()
        {
            leave(&mut state, reason);
        }
        world.insert_resource(state);
        true
    }
}
