//! Player furniture requests and the lifetime around the player's own timeline.
//!
//! Player locator selection remains a player-table operation. The selected
//! row's own timeline plays on the player avatar, as `PlayerFixtureTimelineView`
//! binds it: its CharacterAnimator stream on the avatar's animator, its
//! Fixture1 stream on the fixture's. The common timeline runner owns all
//! sampling and SE. This module owns approach, attachment, the avatar's own
//! state motions around the timeline, end requests and cleanup.
//!
//! Resources follow the source state entry: a request selects one seat on the
//! requested fixture and reserves it, then the session loads that seat's
//! timelines, bindings and sounds before the player starts to move. Nothing is
//! prepared for fixtures nobody asked for, a request never waits on another
//! fixture's readiness, and the session's resources are released with it.
//!
//! Navigation data is supplied by the scene owner. Missing tiles, agent values,
//! or avatar clips are unfinished preparation, never successful eligibility.

pub(crate) mod walk;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use bevy::{ecs::system::SystemState, prelude::*};

use crate::{
    action_button::admission::can_move_to_locator_with_fallback,
    fixture_activity_data::{FixtureActivityTables, PlayerTimelineRow},
    fixture_activity_provider::{
        self, FixtureActivityProvider, PlayerTimelinePlan, ProviderPending,
    },
    fixture_activity_state::{
        FixtureActivityIdentity, FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
    },
    fixture_activity_timeline::{
        self, FixtureActivityTimelines, StartTimeline, TimelineBindings, TimelineCoverageGap,
        TimelineDefinition, TimelineOwner, TimelineOwnerKind, TimelinePayload, TimelineStatus,
        TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::{AttachPoints, AttachPose},
    npc::MotionPhase,
    player::{DashMode, PlayerControlled, PlayerInput},
    player_avatar::{
        AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, AUTO_MOVE_CLIP,
        IDLE_MOTION, STATE_FADE, WALK_MOTION,
    },
    player_state::{PlayerActionState, PlayerAvatarStates},
    site::{GroundEpoch, NavMeshSourceRegion},
};

// This value belongs specifically to PlayForPlayer's local-player entry. It
// is not a clip duration, a default for NPCs, or evidence that playback began.
const PLAYER_TIMELINE_TIMEOUT_SECS: f64 = 100_000_000.0;
const NAV_SAMPLE_DISTANCE: f32 = 3.0;
const EXIT_SPEED: f32 = 0.4;

#[derive(Clone, Debug, Message)]
pub(crate) enum PlayerFixtureRequest {
    Timeline(FixtureTarget),
    Gimmick(FixtureTarget),
    PreviewGimmick {
        target: FixtureTarget,
        owner: Entity,
    },
    RequestEnd,
    Cancel(PlayerFixtureCancelReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerFixtureCancelReason {
    User,
    SiteChanged,
    TargetRemoved,
    PlayerRemoved,
    NavigationFailed,
    AnimationFailed,
    OwnershipLost,
}

#[derive(Clone, Debug)]
pub(crate) enum PlayerFixturePreparationError {
    Missing(&'static str),
    Invalid(String),
    Rejected(&'static str),
    Timeline(String),
}

#[derive(Clone, Debug)]
pub(crate) enum PlayerFixtureAvailability {
    Ready {
        player_row_id: i32,
        locate_index: usize,
        slot_id: i32,
    },
    GimmickReady,
    Pending(PlayerFixturePreparationError),
    Unavailable(&'static str),
}

impl PlayerFixtureAvailability {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. } | Self::GimmickReady)
    }
}

/// One seat's loaded timeline: the selected player row's own timeline and the
/// exact bindings it needs. A session owns it from state entry and releases
/// it with the session.
#[derive(Clone)]
pub(crate) struct PlayerTimelineProfile {
    pub target: FixtureTarget,
    pub actor: Entity,
    pub fixture_id: i32,
    pub player_timeline_row_id: i32,
    pub action_point: i32,
    /// Parsed from the original locator suffix by the shared data supplier.
    pub slot_id: i32,
    /// The row's own prefab from its `mdl_` timeline package.
    pub definition: Arc<TimelineDefinition>,
    pub bindings: TimelineBindings,
    /// Visible to callers during and after a session. Native IK/offset gaps
    /// must not disappear behind a successful animation-resource preflight.
    pub coverage_notes: Vec<String>,
}

impl PlayerTimelineProfile {
    /// Asset preparation only. Keep this profile alive while sounds load.
    /// This does not allocate an activity generation or submit a timeline to
    /// the runner.
    pub(crate) fn preload_source_sounds(
        &mut self,
        world: &World,
        target: &FixtureTarget,
    ) -> Result<(), PlayerFixturePreparationError> {
        let mut request = self.start_request(
            FixtureActivityOwner {
                actor: self.actor,
                generation: 0,
            },
            target,
        );
        fixture_activity_timeline::prepare_source_sounds(world, &mut request)
            .map_err(|error| PlayerFixturePreparationError::Timeline(format!("{error:?}")))?;
        self.bindings.sounds = request.bindings.sounds;
        Ok(())
    }

    pub(crate) fn start_request(
        &self,
        owner: FixtureActivityOwner,
        target: &FixtureTarget,
    ) -> StartTimeline {
        StartTimeline {
            owner: TimelineOwner {
                activity: owner,
                kind: TimelineOwnerKind::Player,
            },
            fixture: target.entity,
            definition: self.definition.clone(),
            bindings: self.bindings.clone(),
            // The row's own timeline carries its SE tracks itself.
            companions: Vec::new(),
            timeout_secs: PLAYER_TIMELINE_TIMEOUT_SECS,
            timeout_budget: TimelineTimeoutBudget::PlayerWall,
        }
    }
}

/// The distinction between an unknown tile and an absent tile is important:
/// unknown input is not proof that a movement or fallback is impossible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FixtureTileKnowledge {
    /// Whether a tile exists at this coordinate is unknown.
    Unknown,
    Missing,
    /// The authored tile exists but the occupant identity is not ready.
    ExistingUnknownOccupant,
    Empty,
    Occupied(String),
}

/// The query's boolean and its corner buffer are separate outputs. A failed
/// calculation with no corners is still a known result, not missing input.
/// In particular, locator ranking sums the buffer even when the boolean is
/// false; the reachability utility reads the boolean before its last corner.
#[derive(Clone, Debug)]
pub(crate) struct PlayerFixturePath {
    pub query_succeeded: bool,
    pub corners: Vec<Vec3>,
    /// Diagnostic only. Neither locator ranking nor CanNavmeshMoveTargetPosition
    /// reads this value, so it must not become an additional admission gate.
    pub status: Option<PlayerFixturePathStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerFixturePathStatus {
    Complete,
    Partial,
    Invalid,
}

/// An immutable, generation-stamped scene navigation snapshot. Implementations
/// must return actual samples/paths/UID tiles, not precomputed permissive flags.
/// A WalkFace adapter must document its single-height grid approximation.
pub(crate) trait PlayerFixtureNavWorld: Send + Sync {
    fn sample_position(&self, position: Vec3, max_distance: f32) -> Option<Vec3>;
    /// CalculatePath with the caller's all-areas mask. Its endpoint projection
    /// belongs to the navigation backend, not the player's radius or a separate
    /// movement helper's sample-and-pullback policy. Missing backend inputs are
    /// an error; a source query returning false belongs in PlayerFixturePath.
    fn path(
        &self,
        from: Vec3,
        to: Vec3,
    ) -> Result<PlayerFixturePath, PlayerFixturePreparationError>;
    fn tile_at(&self, position: Vec3) -> FixtureTileKnowledge;
    fn constrain_move(&self, from: Vec3, to: Vec3) -> Option<Vec3>;
}

#[derive(Resource, Clone)]
pub(crate) struct PlayerFixtureNavigation {
    pub world: Arc<dyn PlayerFixtureNavWorld>,
    pub scene_stamp: crate::fixture_scene_inputs::FixtureSceneStamp,
    pub site_generation: u64,
    pub navigation_generation: u64,
    pub agent_radius: f32,
    pub agent_speed: f32,
    pub default_move_speed: f32,
    pub coverage_notes: Vec<String>,
}

impl PlayerFixtureNavigation {
    fn validate(&self, site_generation: u64) -> Result<(), PlayerFixturePreparationError> {
        if self.site_generation != site_generation {
            return Err(PlayerFixturePreparationError::Missing(
                "current-site navigation snapshot",
            ));
        }
        if [self.agent_radius, self.agent_speed, self.default_move_speed]
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(PlayerFixturePreparationError::Invalid(
                "agent radius/speed and player default speed need positive source values".into(),
            ));
        }
        Ok(())
    }
}

struct PreparedPlayerFixture {
    owner: FixtureActivityOwner,
    target: FixtureTarget,
    identity: FixtureActivityIdentity,
    player_row: PlayerTimelineRow,
    locate_index: usize,
    slot_id: i32,
    /// The selected row's data-level join; its assets load after admission.
    plan: PlayerTimelinePlan,
    end: AttachPose,
    start_anchor_local: Transform,
    approach_path: Vec<Vec3>,
    approach_hit: Vec3,
    navigation_generation: u64,
    site_generation: u64,
    navigation_coverage: Vec<String>,
}

/// Independently held from animation ownership: approach is still real player
/// locomotion, while manual movement and other business actions must yield.
#[derive(Component, Clone, Copy)]
pub(crate) struct PlayerFixtureHeld(pub FixtureActivityOwner);

/// Outlives a removed player entity so finally can release the global player
/// state lock. A later activity must replace this lease when taking control.
#[derive(Resource)]
pub(crate) struct PlayerFixtureControlOwner(pub FixtureActivityOwner);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerFixturePhase {
    /// State entry: the seat is reserved and its resources are loading. The
    /// player holds still; the run motion starts only once they are ready.
    Loading,
    Approaching,
    Fitting,
    StartingTimeline,
    Playing,
    Exiting,
}

#[derive(Clone, Debug)]
pub(crate) enum PlayerFixtureOutcome {
    Completed,
    Cancelled(PlayerFixtureCancelReason),
    /// Nothing played and no resource failed: admission refused the request,
    /// or the runner refused to start a loaded session because an animator
    /// it binds is playing another timeline. A refused session has already
    /// returned to idle and released its seat.
    NotPrepared(PlayerFixturePreparationError),
    /// The seat was reserved and its resources were requested, but a load or
    /// binding failed. The session returned to idle and released everything;
    /// nothing retries it.
    LoadFailed {
        target: FixtureTarget,
        error: PlayerFixturePreparationError,
    },
    /// Gimmick is a distinct state-10 workflow. It is not a seat timeline.
    GimmickNotPrepared {
        target: FixtureTarget,
        reason: String,
    },
    GimmickStarted {
        target: FixtureTarget,
    },
}

/// Why a session ended before completing.
enum SessionEnd {
    Cancelled(PlayerFixtureCancelReason),
    LoadFailed(PlayerFixturePreparationError),
    /// Loaded, but the runner cannot start it now (see `verify_loaded`).
    Refused(PlayerFixturePreparationError),
}

impl From<PlayerFixtureCancelReason> for SessionEnd {
    fn from(reason: PlayerFixtureCancelReason) -> Self {
        Self::Cancelled(reason)
    }
}

struct LinearMove {
    start: Vec3,
    target: Vec3,
    elapsed: f32,
    duration: f32,
}

impl LinearMove {
    fn new(start: Vec3, target: Vec3, speed: f32) -> Self {
        Self {
            start,
            target,
            elapsed: 0.0,
            duration: start.distance(target) / speed,
        }
    }

    fn step(&mut self, dt: f32) -> (Vec3, f32, bool) {
        self.elapsed += dt;
        let fraction = if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        };
        (
            self.start.lerp(self.target, fraction),
            fraction,
            fraction >= 1.0,
        )
    }
}

struct PlayerFixtureSession {
    prepared: PreparedPlayerFixture,
    phase: PlayerFixturePhase,
    /// The seat's resources: the unfinished attempt while loading (it keeps
    /// in-flight handles alive), then the complete profile. Dropped with the
    /// session, which releases every handle the session loaded.
    profile: Option<PlayerTimelineProfile>,
    before_parent: Option<Entity>,
    before_dash: bool,
    anchor: Option<Entity>,
    anchor_local: Option<Transform>,
    animation_lease: Option<PlayerActionToken>,
    timeline: Option<TimelineToken>,
    end_requested: bool,
    end_dispatched: bool,
    path_cursor: usize,
    last_position: Vec3,
    unchanged_secs: f32,
    movement: Option<LinearMove>,
}

#[derive(Resource, Default)]
pub(crate) struct PlayerFixtureRuntime {
    pending: Vec<PlayerFixtureRequest>,
    session: Option<PlayerFixtureSession>,
    generation: u64,
    availability: HashMap<FixtureTarget, PlayerFixtureAvailability>,
    availability_sweep: AvailabilitySweep,
    pub last_outcome: Option<PlayerFixtureOutcome>,
    last_navigation_coverage: Vec<String>,
    pub last_timeline_coverage: Vec<TimelineCoverageGap>,
}

impl PlayerFixtureRuntime {
    pub(crate) fn active(&self) -> bool {
        self.session.is_some()
    }
    pub(crate) fn blocks_manual_movement(&self) -> bool {
        self.active()
    }
    pub(crate) fn phase(&self) -> Option<PlayerFixturePhase> {
        self.session.as_ref().map(|session| session.phase)
    }
    pub(crate) fn target(&self) -> Option<&FixtureTarget> {
        self.session
            .as_ref()
            .map(|session| &session.prepared.target)
    }
    /// The live session's loaded profile. An ended session keeps none: its
    /// resources are released when it ends.
    pub(crate) fn profile(&self) -> Option<&PlayerTimelineProfile> {
        self.session
            .as_ref()
            .filter(|session| session.phase != PlayerFixturePhase::Loading)
            .and_then(|session| session.profile.as_ref())
    }
    pub(crate) fn navigation_coverage(&self) -> Option<&[String]> {
        self.session
            .as_ref()
            .map(|session| session.prepared.navigation_coverage.as_slice())
            .or_else(|| {
                (!self.last_navigation_coverage.is_empty())
                    .then_some(self.last_navigation_coverage.as_slice())
            })
    }
    pub(crate) fn availability(
        &self,
        target: &FixtureTarget,
    ) -> Option<&PlayerFixtureAvailability> {
        self.availability.get(target)
    }
}

/// Schedule before request admission. Separate MessageReader state means the
/// existing button/gesture consumers need not surrender their event readers.
pub(crate) fn receive_requests(
    mut requests: MessageReader<PlayerFixtureRequest>,
    mut runtime: ResMut<PlayerFixtureRuntime>,
    editor: Res<crate::fixture_edit::EditSessionActive>,
) {
    // Entry has already cancelled the old activity owner. Drain requests made
    // earlier in this frame too, so a queued tap cannot reacquire that owner.
    if editor.is_active() {
        requests.clear();
        runtime.pending.clear();
        return;
    }
    runtime.pending.extend(requests.read().cloned());
}

/// Run before receive_requests/advance. Only a session already present at the
/// start of this input pass observes gestures, so its initiating tap cannot
/// also request an immediate end. Only TAP/END matches the gesture protocol.
pub(crate) fn request_end_from_input(
    mut gestures: MessageReader<crate::gesture::GestureEvent>,
    joystick: Res<crate::joystick::JoystickState>,
    runtime: Res<PlayerFixtureRuntime>,
    mut requests: MessageWriter<PlayerFixtureRequest>,
) {
    if runtime
        .session
        .as_ref()
        .is_none_or(|session| session.end_dispatched)
    {
        gestures.clear();
        return;
    }
    let tapped = gestures.read().any(|event| {
        event.kind == crate::gesture::GestureKind::Tap
            && event.state == crate::gesture::GestureState::End
    });
    if tapped || joystick.active {
        requests.write(PlayerFixtureRequest::RequestEnd);
    }
}

/// Work allowed per frame for re-deriving availability. A full layout is
/// revisited over several frames; placements without an entry come first.
const AVAILABILITY_BUDGET: Duration = Duration::from_micros(1500);

/// Round-robin position of the incremental availability refresh.
#[derive(Default)]
struct AvailabilitySweep {
    cursor: usize,
    session_active: bool,
}

/// Product-layer cache for the content library, which labels and gates its
/// furniture entries with it. The source has no counterpart: it never sweeps
/// every placed fixture, and evaluates a seat only for the fixture the player
/// is touching or has tapped. So nothing on the in-game path reads this: the
/// action button pushes and dispatches from its own collision state, and
/// `advance` selects the seat again from the live world when a request
/// arrives, so an entry that is a few frames old cannot start the wrong seat.
///
/// An entry answers what `select` would answer for that fixture now: seat
/// selection, the data-level plan of its player row and the approach query. It needs
/// no loaded or published timeline; resources load only for a request. The
/// refresh runs within a per-frame budget instead of re-planning every seat
/// of every fixture each frame; at least one fixture is re-derived per frame.
pub(crate) fn refresh_availability(world: &mut World) {
    let Some(mut runtime) = world.remove_resource::<PlayerFixtureRuntime>() else {
        return;
    };
    let targets: Vec<FixtureTarget> = world
        .query::<(Entity, &FixtureActivityIdentity)>()
        .iter(world)
        .map(|(entity, identity)| FixtureTarget {
            entity,
            uid: identity.uid.clone(),
        })
        .collect();
    let live: HashSet<&FixtureTarget> = targets.iter().collect();
    runtime.availability.retain(|target, _| live.contains(target));
    if runtime.active() {
        for target in targets {
            runtime.availability.insert(
                target,
                PlayerFixtureAvailability::Unavailable("player furniture session owns input"),
            );
        }
        runtime.availability_sweep.session_active = true;
        world.insert_resource(runtime);
        return;
    }
    if std::mem::take(&mut runtime.availability_sweep.session_active) {
        // Entries written while a session owned input say nothing about the
        // seats now; re-derive them before anything else.
        runtime.availability.clear();
    }
    if targets.is_empty() {
        runtime.availability_sweep.cursor = 0;
        world.insert_resource(runtime);
        return;
    }
    let started = bevy::platform::time::Instant::now();
    let within_budget = |derived: usize| derived == 0 || started.elapsed() < AVAILABILITY_BUDGET;
    let mut derived = 0usize;
    let missing: Vec<usize> = (0..targets.len())
        .filter(|index| !runtime.availability.contains_key(&targets[*index]))
        .collect();
    for index in missing {
        if !within_budget(derived) {
            break;
        }
        let state = derive_availability(world, &runtime, &targets[index]);
        runtime.availability.insert(targets[index].clone(), state);
        derived += 1;
    }
    let count = targets.len();
    let mut cursor = runtime.availability_sweep.cursor % count;
    for _ in 0..count {
        if !within_budget(derived) {
            break;
        }
        let state = derive_availability(world, &runtime, &targets[cursor]);
        runtime.availability.insert(targets[cursor].clone(), state);
        derived += 1;
        cursor = (cursor + 1) % count;
    }
    runtime.availability_sweep.cursor = cursor;
    world.insert_resource(runtime);
}

fn derive_availability(
    world: &mut World,
    runtime: &PlayerFixtureRuntime,
    target: &FixtureTarget,
) -> PlayerFixtureAvailability {
    if world
        .get::<FixtureActivityIdentity>(target.entity)
        .and_then(|identity| {
            world
                .get_resource::<FixtureActivityTables>()?
                .fixture_master(identity.master_id)
        })
        .is_some_and(|master| matches!(master.player_action_type.as_str(), "loop" | "one_shot"))
    {
        return match crate::fixture_gimmick::session::availability(world, target) {
            Ok(()) => PlayerFixtureAvailability::GimmickReady,
            Err(reason) => {
                PlayerFixtureAvailability::Pending(PlayerFixturePreparationError::Invalid(reason))
            }
        };
    }
    match select(world, target, runtime.generation.saturating_add(1)) {
        Ok(prepared) => PlayerFixtureAvailability::Ready {
            player_row_id: prepared.player_row.id,
            locate_index: prepared.locate_index,
            slot_id: prepared.slot_id,
        },
        Err(PlayerFixturePreparationError::Rejected(reason)) => {
            PlayerFixtureAvailability::Unavailable(reason)
        }
        Err(error) => PlayerFixtureAvailability::Pending(error),
    }
}

/// Admission for one requested fixture, with no asset access: the nearest
/// unoccupied reachable seat on that fixture (GetNearestPlayerLocatorIndex at
/// the tap), the data-level plan of its player row, and the approach query to
/// its StartLoc.
fn select(
    world: &mut World,
    target: &FixtureTarget,
    generation: u64,
) -> Result<PreparedPlayerFixture, PlayerFixturePreparationError> {
    use PlayerFixturePreparationError::{Invalid, Missing, Rejected};
    let identity = world
        .get::<FixtureActivityIdentity>(target.entity)
        .filter(|identity| target.matches(identity))
        .ok_or(Rejected("fixture Entity/UID is stale"))?
        .clone();
    let fixture_world = *world
        .get::<GlobalTransform>(target.entity)
        .ok_or(Missing("fixture transform"))?;
    let site_generation = world
        .get_resource::<GroundEpoch>()
        .ok_or(Missing("site generation"))?
        .0;
    if !world.contains_resource::<FixtureActivityTimelines>() {
        return Err(Missing("shared timeline runtime"));
    }
    if world.contains_resource::<PlayerFixtureControlOwner>() {
        return Err(Rejected("player fixture control lease is occupied"));
    }
    let navigation = world
        .get_resource::<PlayerFixtureNavigation>()
        .ok_or(Missing("player navigation supplier"))?
        .clone();
    navigation.validate(site_generation)?;
    crate::fixture_scene_inputs::validate_admission(world, &navigation)?;
    let region = world
        .get_resource::<NavMeshSourceRegion>()
        .ok_or(Missing("navigation source region"))?
        .0;
    let states = world
        .get_resource::<PlayerAvatarStates>()
        .ok_or(Missing("player state owner"))?;
    if !states.can_intercept {
        return Err(Rejected("player intercept gate is closed"));
    }
    let (actor, position) = {
        let mut query = world.query_filtered::<(Entity, &AvatarDriver), With<PlayerControlled>>();
        let mut players = query.iter(world);
        let (actor, driver) = players.next().ok_or(Missing("player avatar body"))?;
        if players.next().is_some() {
            return Err(Invalid("multiple logical players".into()));
        }
        if !driver.locomotion_owns_animator() {
            return Err(Rejected("another player action owns animation"));
        }
        if world.get::<PlayerFixtureHeld>(actor).is_some()
            || world.get::<crate::talk::TalkHold>(actor).is_some()
        {
            return Err(Rejected("another player activity holds movement"));
        }
        (
            actor,
            world_pose(world, actor)
                .ok_or(Missing("player world pose"))?
                .translation,
        )
    };
    let owner = FixtureActivityOwner { actor, generation };
    if navigation.scene_stamp.player != actor {
        return Err(Missing(
            "navigation parameters for the current logical player",
        ));
    }
    let tables = world
        .get_resource::<FixtureActivityTables>()
        .ok_or(Missing("fixture activity tables"))?;
    let master = tables
        .fixture_master(identity.master_id)
        .ok_or(Missing("fixture master identity"))?;
    if master.player_action_type != "timeline" {
        return Err(Rejected("fixture is not a player timeline action"));
    }
    if format!("mysekai__fixture__{}", master.model_name) != identity.model_package {
        return Err(Invalid(
            "fixture instance model does not match its master".into(),
        ));
    }
    let source_rows: Vec<PlayerTimelineRow> = tables
        .player_timelines(identity.master_id)
        .cloned()
        .collect();
    if source_rows.is_empty() {
        return Err(Rejected("no player timeline locator for this fixture"));
    }
    let points = world
        .get_resource::<AttachPoints>()
        .ok_or(Missing("fixture attach points"))?;
    let reservations = world
        .get_resource::<FixtureActivityReservations>()
        .ok_or(Missing("shared fixture reservations"))?;
    // Generate from the actual attachment array, with the source's first
    // matching master row. Known invalid StartLoc names generate no locator;
    // they do not turn the remaining valid seats into a missing-data error.
    let rows: Vec<_> = points
        .player_action_points(&identity.model_package)
        .ok_or(Missing("original player locator array metadata"))?
        .into_iter()
        .filter_map(|point| {
            source_rows
                .iter()
                .find(|row| row.action_point == point)
                .cloned()
        })
        .collect();

    struct Candidate {
        row: PlayerTimelineRow,
        locate_index: usize,
        slot_id: i32,
        start: AttachPose,
        end: AttachPose,
    }
    let mut nearest: Option<(f32, Candidate)> = None;
    // Stage one has no animation readiness dependency. It may delay the
    // chosen source seat, but cannot select another seat.
    for row in rows {
        let locate_index = points
            .instance_index(&identity.model_package, row.action_point)
            .ok_or(Missing("unique original locator array index"))?;
        let slot_id = points
            .instance_slot(&identity.model_package, row.action_point)
            .ok_or(Missing("original locator slot suffix"))?;
        let poses = points
            .instance_poses(&identity.model_package, row.action_point, &fixture_world)
            .ok_or(Missing("instance StartLoc/EndLoc"))?;
        let end = poses.end.ok_or(Missing("instance EndLoc"))?;
        if reservations.action_slot_in_use(target, slot_id)
            || reservations.player_point_in_use(target, locate_index)
        {
            continue;
        }
        let start = Vec3::from(poses.start.position);
        // GetNearestPlayerLocatorIndex here and at the button's admission
        // calls the same CanMoveToLocator, with the region's fallback.
        let reach = |locator| {
            can_move_to_locator_with_fallback(&navigation, region, position, locator, &target.uid)
        };
        if !reach(start)? || !reach(Vec3::from(end.position))? {
            continue;
        }
        let path = navigation.world.path(position, start)?;
        // GetNearestPlayerLocatorIndex ignores the query boolean and sums
        // adjacent corners. Zero and one corner therefore both rank as zero;
        // tile-fallback eligibility must not silently choose a different seat.
        let length = path_length(&path.corners).ok_or_else(|| {
            Invalid("navigation returned a nonfinite locator corner buffer".into())
        })?;
        if nearest
            .as_ref()
            .is_none_or(|(shortest, _)| length < *shortest)
        {
            nearest = Some((
                length,
                Candidate {
                    row,
                    locate_index,
                    slot_id,
                    start: poses.start,
                    end,
                },
            ));
        }
    }
    let (_, selected) = nearest.ok_or(Rejected("no unoccupied reachable player locator"))?;

    // Stage two plans only the source-selected seat's row, from master
    // tables; that row's resources load after admission.
    let plan =
        fixture_activity_provider::plan_player_row(world, actor, target, &identity, &selected.row)
            .map_err(plan_error)?;
    if plan.slot_id != selected.slot_id {
        return Err(Invalid("player row plan changes the source slot".into()));
    }
    let start_position = Vec3::from(selected.start.position);
    let hit = navigation
        .world
        .sample_position(start_position, NAV_SAMPLE_DISTANCE)
        .ok_or(Rejected(
            "selected player locator cannot be sampled for approach",
        ))?;
    if !hit.is_finite() || hit.distance(start_position) > NAV_SAMPLE_DISTANCE {
        return Err(Invalid(
            "navigation sample violates its query radius".into(),
        ));
    }
    let path = navigation.world.path(position, hit)?;
    let approach_path = provisional_approach_corners(path)?;
    let mut navigation_coverage = navigation.coverage_notes.clone();
    navigation_coverage.push(
        "Player approach uses provisional corner stepping; live agent on-mesh state, steering target, and actual velocity remain unsupplied."
            .into(),
    );
    Ok(PreparedPlayerFixture {
        owner,
        target: target.clone(),
        identity,
        player_row: selected.row,
        locate_index: selected.locate_index,
        slot_id: selected.slot_id,
        plan,
        end: selected.end,
        start_anchor_local: GlobalTransform::from(Transform {
            translation: start_position,
            rotation: selected.start.rotation,
            scale: Vec3::ONE,
        })
        .reparented_to(&fixture_world),
        approach_path,
        approach_hit: hit,
        navigation_generation: navigation.navigation_generation,
        site_generation,
        navigation_coverage,
    })
}

/// A data-level join failure in the preparation vocabulary the callers read.
fn plan_error(pending: ProviderPending) -> PlayerFixturePreparationError {
    use PlayerFixturePreparationError::{Invalid, Missing};
    match pending.stage {
        "player-rig" => Missing("player avatar animator"),
        "source-tables" => Missing("fixture activity tables"),
        "source-locators" => Missing("fixture attach points"),
        "fixture-instance" => Missing("fixture transform"),
        _ => Invalid(format!("{}: {}", pending.stage, pending.reason)),
    }
}

/// A live-instance failure at the tap, or a resource failure while a session
/// loads: still-arriving input is unfinished, anything else is final.
fn load_error(pending: ProviderPending) -> PlayerFixturePreparationError {
    if pending.retryable {
        PlayerFixturePreparationError::Missing("placed fixture instance")
    } else {
        PlayerFixturePreparationError::Timeline(format!("{}: {}", pending.stage, pending.reason))
    }
}

/// The loaded profile still describes the admitted seat, the live avatar body
/// and only this fixture's nodes, and the runner would accept it now.
/// `Rejected` means only the last part failed: an animator the profile binds
/// is playing another timeline. That is contention, not a failed resource;
/// every other error is a failed load or binding.
fn verify_loaded(
    world: &World,
    session: &PlayerFixtureSession,
) -> Result<(), PlayerFixturePreparationError> {
    use PlayerFixturePreparationError::{Invalid, Missing, Rejected, Timeline};
    let prepared = &session.prepared;
    let profile = session
        .profile
        .as_ref()
        .ok_or(Missing("player timeline profile"))?;
    if profile.action_point != prepared.player_row.action_point
        || profile.slot_id != prepared.slot_id
    {
        return Err(Invalid(
            "player timeline profile changes the source player action point or slot".into(),
        ));
    }
    if profile.definition.package != prepared.plan.player_package
        || prefab_leaf(&profile.definition.prefab) != prepared.player_row.asset_name
    {
        return Err(Invalid(
            "player timeline profile is not the selected row's own prefab".into(),
        ));
    }
    let actor = prepared.owner.actor;
    let player_animator = world
        .get::<AvatarDriver>(actor)
        .map(|driver| driver.player)
        .ok_or(Missing("player avatar body"))?;
    if !profile
        .bindings
        .animations
        .values()
        .any(|binding| binding.animator == player_animator)
    {
        return Err(Missing("player avatar animation binding"));
    }
    if profile.bindings.animations.values().any(|binding| {
        binding.animator != player_animator
            && !is_descendant_of(world, binding.animator, prepared.target.entity)
    }) || profile
        .bindings
        .actors
        .values()
        .any(|bound| *bound != actor && *bound != prepared.target.entity)
    {
        return Err(Invalid(
            "player timeline bindings target a nonparticipant actor".into(),
        ));
    }
    let request = profile.start_request(prepared.owner, &prepared.target);
    fixture_activity_timeline::validate_start(world, &request).map_err(|_| {
        // validate_start is validate_prepared plus the runner's rule that an
        // animator plays one timeline at a time, over the same inputs. When
        // only that rule fails, a bound animator (for example the fixture's,
        // driven by an NPC on another seat) is busy: the runner refuses the
        // start, and no resource failed. Otherwise the failure is reported
        // as validate_prepared words it, without the ownership rule.
        match fixture_activity_timeline::validate_prepared(world, &request) {
            Ok(()) => Rejected("a bound animator is occupied by another timeline"),
            Err(failure) => Timeline(format!("{failure:?}")),
        }
    })
}

fn prefab_leaf(path: &str) -> &str {
    let leaf = path.rsplit(['/', '\\']).next().unwrap_or(path);
    leaf.strip_suffix(".prefab").unwrap_or(leaf)
}

fn path_length(path: &[Vec3]) -> Option<f32> {
    if path.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let length: f32 = path.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
    length.is_finite().then_some(length)
}

/// This is preparation for the existing corner-stepping adapter, not another
/// source reachability gate. AutoMoveTargetAsync issues SetDestination without
/// checking its return and finishes from the live agent's velocity and position.
/// A query's false/Partial result or a distant last corner cannot reject that
/// source action. An empty corner buffer is valid query data; this provisional
/// adapter lacks the live steering input needed to drive it, so preparation is
/// unfinished rather than evidence that the selected locator is unreachable.
fn provisional_approach_corners(
    path: PlayerFixturePath,
) -> Result<Vec<Vec3>, PlayerFixturePreparationError> {
    if path_length(&path.corners).is_none() {
        return Err(PlayerFixturePreparationError::Invalid(
            "navigation returned a nonfinite approach corner buffer".into(),
        ));
    }
    if path.corners.is_empty() {
        return Err(PlayerFixturePreparationError::Missing(
            "live player agent steering for an empty approach corner buffer",
        ));
    }
    Ok(path.corners)
}

fn is_descendant_of(world: &World, entity: Entity, root: Entity) -> bool {
    let mut current = entity;
    let mut seen = Vec::new();
    loop {
        if current == root {
            return true;
        }
        if seen.contains(&current) {
            return false;
        }
        seen.push(current);
        let Some(parent) = world.get::<ChildOf>(current) else {
            return false;
        };
        current = parent.parent();
    }
}

/// Run once before the timeline runner and player locomotion each frame. This
/// exclusive adapter writes the actual player Transform/parent and starts the
/// shared runner, not a log-only effect queue.
pub(crate) fn advance(world: &mut World) {
    let Some(mut runtime) = world.remove_resource::<PlayerFixtureRuntime>() else {
        return;
    };
    let dt = world.get_resource::<Time>().map_or(0.0, Time::delta_secs);
    for request in std::mem::take(&mut runtime.pending) {
        match request {
            PlayerFixtureRequest::Cancel(reason) => {
                crate::fixture_gimmick::session::cancel_previews(world);
                if let Some(session) = runtime.session.take() {
                    capture_coverage(world, &session, &mut runtime);
                    finish(
                        world,
                        session,
                        reason == PlayerFixtureCancelReason::SiteChanged,
                    );
                    runtime.last_outcome = Some(PlayerFixtureOutcome::Cancelled(reason));
                }
            }
            PlayerFixtureRequest::RequestEnd => {
                if let Some(session) = runtime.session.as_mut() {
                    session.end_requested = true;
                }
            }
            PlayerFixtureRequest::Timeline(target) => {
                if let Some(session) = runtime.session.as_mut() {
                    // The presenter toggles its current timeline, not another
                    // concurrent session, even if a new target is now nearby.
                    session.end_requested = true;
                    continue;
                }
                let next = runtime
                    .generation
                    .checked_add(1)
                    .expect("player activity generation exhausted");
                // Admission never waits on resources: an unloaded timeline is
                // loaded by the session this request opens.
                match select(world, &target, next)
                    .and_then(|prepared| require_live_instance(world, prepared))
                    .and_then(|prepared| begin(world, prepared))
                {
                    Ok(session) => {
                        runtime.generation = next;
                        runtime.last_outcome = None;
                        runtime.last_timeline_coverage.clear();
                        runtime.last_navigation_coverage =
                            session.prepared.navigation_coverage.clone();
                        runtime.session = Some(session);
                    }
                    Err(error) => {
                        runtime.last_outcome = Some(PlayerFixtureOutcome::NotPrepared(error))
                    }
                }
            }
            PlayerFixtureRequest::PreviewGimmick { target, owner } => {
                let next = runtime.generation.saturating_add(1);
                match crate::fixture_gimmick::session::start_preview(world, &target, owner, next) {
                    Ok(()) => {
                        runtime.generation = next;
                        runtime.last_outcome =
                            Some(PlayerFixtureOutcome::GimmickStarted { target });
                    }
                    Err(reason) => {
                        warn!(
                            "[fixture-gimmick] {} preview rejected: {reason}",
                            target.uid
                        );
                        runtime.last_outcome =
                            Some(PlayerFixtureOutcome::GimmickNotPrepared { target, reason });
                    }
                }
            }
            PlayerFixtureRequest::Gimmick(target) => {
                let next = runtime
                    .generation
                    .checked_add(1)
                    .expect("player activity generation exhausted");
                match crate::fixture_gimmick::start(world, &target, next) {
                    Ok(()) => {
                        runtime.generation = next;
                        runtime.last_outcome =
                            Some(PlayerFixtureOutcome::GimmickStarted { target });
                    }
                    Err(reason) => {
                        warn!(
                            "[fixture-gimmick] {} request not prepared: {reason}",
                            target.uid
                        );
                        runtime.last_outcome =
                            Some(PlayerFixtureOutcome::GimmickNotPrepared { target, reason });
                    }
                }
            }
        }
    }
    if let Some(mut session) = runtime.session.take() {
        capture_coverage(world, &session, &mut runtime);
        // A load that completes this frame moves on in the same frame, so a
        // resident timeline starts its approach as soon as it is admitted.
        let stepped = match load_session(world, &mut session) {
            Ok(true) => step_session(world, &mut session, dt).map_err(SessionEnd::Cancelled),
            other => other.map(|_| false),
        };
        match stepped {
            Ok(false) => runtime.session = Some(session),
            Ok(true) => {
                finish(world, session, false);
                runtime.last_outcome = Some(PlayerFixtureOutcome::Completed);
            }
            Err(SessionEnd::Cancelled(reason)) => {
                info!(
                    "[player-fixture] {} player row {} cancelled: {reason:?}",
                    session.prepared.target.uid, session.prepared.player_row.id
                );
                finish(
                    world,
                    session,
                    reason == PlayerFixtureCancelReason::SiteChanged,
                );
                runtime.last_outcome = Some(PlayerFixtureOutcome::Cancelled(reason));
            }
            Err(SessionEnd::LoadFailed(error)) => {
                // Reported once: the session ends here and nothing retries it.
                error!(
                    "[player-fixture] {} player row {} resources failed: {error:?}",
                    session.prepared.target.uid, session.prepared.player_row.id
                );
                let target = session.prepared.target.clone();
                finish(world, session, false);
                runtime.last_outcome = Some(PlayerFixtureOutcome::LoadFailed { target, error });
            }
            Err(SessionEnd::Refused(error)) => {
                // Contention, not a failed resource: one line, then the same
                // release as any other end.
                warn!(
                    "[player-fixture] {} player row {} not started: {error:?}",
                    session.prepared.target.uid, session.prepared.player_row.id
                );
                finish(world, session, false);
                runtime.last_outcome = Some(PlayerFixtureOutcome::NotPrepared(error));
            }
        }
    }
    world.insert_resource(runtime);
}

/// The live placed instance behind an admitted seat. The source only offers a
/// fixture's button once its FixtureView is set up, so a request for an
/// instance that is not live yet is refused rather than waited on.
fn require_live_instance(
    world: &mut World,
    prepared: PreparedPlayerFixture,
) -> Result<PreparedPlayerFixture, PlayerFixturePreparationError> {
    world.init_resource::<FixtureActivityProvider>();
    world
        .resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.require_live_fixture(world, &prepared.target, &prepared.identity.model_package)
        })
        .map_err(load_error)?;
    Ok(prepared)
}

/// State entry of UseTimelineFixture: reserve the seat, take the state and
/// input, and start loading. The run motion is not started here; the source
/// changes to it only after the awaited timeline load.
fn begin(
    world: &mut World,
    prepared: PreparedPlayerFixture,
) -> Result<PlayerFixtureSession, PlayerFixturePreparationError> {
    let actor = prepared.owner.actor;
    let before_parent = world.get::<ChildOf>(actor).map(ChildOf::parent);
    let before_dash = world
        .get::<DashMode>(actor)
        .ok_or(PlayerFixturePreparationError::Missing("player dash state"))?
        .0;
    let position = world_pose(world, actor)
        .ok_or(PlayerFixturePreparationError::Missing("player pose"))?
        .translation;
    let mut reservations = world.resource_mut::<FixtureActivityReservations>();
    if !reservations.reserve_player_slot(
        &prepared.target,
        prepared.slot_id,
        prepared.locate_index,
        prepared.owner,
    ) {
        return Err(PlayerFixturePreparationError::Rejected(
            "player slot was reserved before admission",
        ));
    }
    drop(reservations);
    let mut states = world.resource_mut::<PlayerAvatarStates>();
    states.change_status(PlayerActionState::UseTimelineFixture);
    states.can_intercept = false;
    drop(states);
    world.entity_mut(actor).insert((
        PlayerFixtureHeld(prepared.owner),
        MotionPhase::Dwelling { remaining: None },
    ));
    world.insert_resource(PlayerFixtureControlOwner(prepared.owner));
    if let Some(animator) = world.get::<AvatarDriver>(actor).map(|driver| driver.player) {
        if let Some(mut animation_player) = world.get_mut::<AnimationPlayer>(animator) {
            for (_, animation) in animation_player.playing_animations_mut() {
                animation.set_speed(1.0);
            }
        }
    }
    if let Some(mut input) = world.get_mut::<PlayerInput>(actor) {
        input.active = false;
        input.direction = Vec3::ZERO;
    }
    Ok(PlayerFixtureSession {
        prepared,
        phase: PlayerFixturePhase::Loading,
        profile: None,
        before_parent,
        before_dash,
        anchor: None,
        anchor_local: None,
        animation_lease: None,
        timeline: None,
        end_requested: false,
        end_dispatched: false,
        path_cursor: 0,
        last_position: position,
        unchanged_secs: 0.0,
        movement: None,
    })
}

/// The loading phase, polled once per frame: load the selected row's own
/// timeline, bind it to this fixture and the player's avatar, and load its
/// SE, like SetupPlayFixtureTimelineAsync before any movement.
/// Returns true once the session is past loading. Loads still in flight keep
/// the session waiting; a failed load or binding ends it (the source's
/// finally returns the player to idle and releases everything), and nothing
/// retries it. A loaded session the runner cannot start now (a bound
/// animator is busy) ends the same way but is reported as a refusal.
fn load_session(world: &mut World, session: &mut PlayerFixtureSession) -> Result<bool, SessionEnd> {
    if session.phase != PlayerFixturePhase::Loading {
        return Ok(true);
    }
    check_live(world, session)?;
    world.init_resource::<FixtureActivityProvider>();
    let polled = world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
        provider.prepare_player_timeline(world, &session.prepared.plan, &mut session.profile)
    });
    match polled {
        Ok(()) => {}
        Err(pending) if pending.retryable => return Ok(false),
        Err(pending) => return Err(SessionEnd::LoadFailed(load_error(pending))),
    }
    verify_loaded(world, session).map_err(|error| match error {
        PlayerFixturePreparationError::Rejected(_) => SessionEnd::Refused(error),
        error => SessionEnd::LoadFailed(error),
    })?;
    // ChangeAnimation(AvatarConfig.RunMotion), then the move to StartLoc.
    // The session holds the animator from here to its end.
    let actor = session.prepared.owner.actor;
    session.animation_lease = Some(
        play_avatar_motion(world, actor, AUTO_MOVE_CLIP, None)
            .map_err(|_| SessionEnd::Cancelled(PlayerFixtureCancelReason::AnimationFailed))?,
    );
    world.entity_mut(actor).insert(MotionPhase::Walking);
    session.phase = PlayerFixturePhase::Approaching;
    Ok(true)
}

/// `ChangeAnimation(motion)`: `PlayAnimation(name, 0.25)` on the player's
/// animator. With a lease the call plays on the session's own lease; without
/// one it takes the animator from locomotion.
fn play_avatar_motion(
    world: &mut World,
    actor: Entity,
    clip: &str,
    lease: Option<PlayerActionToken>,
) -> Result<PlayerActionToken, ()> {
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    let Ok(mut driver) = drivers.get_mut(actor) else {
        error!("[player-fixture] ChangeAnimation({clip}): the player has no avatar driver");
        return Err(());
    };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        error!("[player-fixture] ChangeAnimation({clip}): the avatar animator is missing");
        return Err(());
    };
    let motion = PlayerActionMotion {
        clip,
        speed: 1.0,
        blend: STATE_FADE,
        blocks_manual_movement: true,
    };
    let result = match lease {
        Some(token) => driver
            .play_owned_action(token, motion, &mut graphs, &mut animator, &mut transitions)
            .map(|()| token),
        None => driver.start_action(
            PlayerActionOwner::FixtureTimeline,
            motion,
            &mut graphs,
            &mut animator,
            &mut transitions,
        ),
    };
    match result {
        Ok(token) => {
            info!("[player-fixture] ChangeAnimation({clip}, fade 0.25s)");
            Ok(token)
        }
        Err(error) => {
            error!("[player-fixture] ChangeAnimation({clip}) refused: {error}");
            Err(())
        }
    }
}

/// The state's finally: `ChangeAnimation(AvatarConfig.IdleMotion)`; then
/// `ChangeStatus(Idle)` initializes the Idle state, whose
/// `PlayAnimation(c_000_mov_idle_00, 0.25)` crossfades from it in the same
/// frame and hands the animator back to locomotion as the Idle state.
fn play_finally_idle(world: &mut World, actor: Entity, lease: Option<PlayerActionToken>) {
    let free = world
        .get::<AvatarDriver>(actor)
        .is_some_and(AvatarDriver::locomotion_owns_animator);
    let owned = lease.is_some_and(|token| {
        world
            .get::<AvatarDriver>(actor)
            .is_some_and(|driver| driver.owns_fixture_timeline(token))
    });
    if !owned && !free {
        return;
    }
    let Ok(token) = play_avatar_motion(world, actor, IDLE_MOTION, lease.filter(|_| owned)) else {
        return;
    };
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    if let Ok(mut driver) = drivers.get_mut(actor) {
        if let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) {
            driver.play_idle(
                Some(token),
                STATE_FADE,
                &mut graphs,
                &mut animator,
                &mut transitions,
            );
        }
    }
}

/// Everything a live session must still own this frame: the site, the target
/// instance, the logical player, its control and seat leases and animator.
fn check_live(
    world: &World,
    session: &PlayerFixtureSession,
) -> Result<(), PlayerFixtureCancelReason> {
    use PlayerFixtureCancelReason::*;
    let actor = session.prepared.owner.actor;
    if world.get_resource::<GroundEpoch>().map(|epoch| epoch.0)
        != Some(session.prepared.site_generation)
    {
        return Err(SiteChanged);
    }
    if !world.contains_resource::<FixtureActivityTimelines>() {
        return Err(AnimationFailed);
    }
    let identity = world
        .get::<FixtureActivityIdentity>(session.prepared.target.entity)
        .ok_or(TargetRemoved)?;
    if !session.prepared.target.matches(identity) || identity != &session.prepared.identity {
        return Err(TargetRemoved);
    }
    if world.get::<PlayerControlled>(actor).is_none() {
        return Err(PlayerRemoved);
    }
    if world.get::<PlayerFixtureHeld>(actor).map(|held| held.0) != Some(session.prepared.owner)
        || world
            .get_resource::<PlayerFixtureControlOwner>()
            .is_none_or(|lease| lease.0 != session.prepared.owner)
        || world
            .get_resource::<FixtureActivityReservations>()
            .is_none_or(|reservations| {
                !reservations.owns_player_slot(
                    &session.prepared.target,
                    session.prepared.slot_id,
                    session.prepared.locate_index,
                    session.prepared.owner,
                )
            })
    {
        return Err(OwnershipLost);
    }
    let driver = world.get::<AvatarDriver>(actor).ok_or(AnimationFailed)?;
    if session.animation_lease.is_none() && !driver.locomotion_owns_animator() {
        return Err(OwnershipLost);
    }
    if session
        .animation_lease
        .is_some_and(|lease| !driver.owns_fixture_timeline(lease))
    {
        return Err(OwnershipLost);
    }
    Ok(())
}

fn step_session(
    world: &mut World,
    session: &mut PlayerFixtureSession,
    dt: f32,
) -> Result<bool, PlayerFixtureCancelReason> {
    use PlayerFixtureCancelReason::*;
    let actor = session.prepared.owner.actor;
    check_live(world, session)?;
    if let (Some(anchor), Some(local)) = (session.anchor, session.anchor_local) {
        let instance = world
            .get::<GlobalTransform>(session.prepared.target.entity)
            .ok_or(TargetRemoved)?;
        let global = instance.mul_transform(local);
        let mut anchor_entity = world.get_entity_mut(anchor).map_err(|_| OwnershipLost)?;
        anchor_entity.insert((global.compute_transform(), global));
    }
    let navigation = world
        .get_resource::<PlayerFixtureNavigation>()
        .ok_or(NavigationFailed)?
        .clone();
    navigation
        .validate(session.prepared.site_generation)
        .map_err(|_| NavigationFailed)?;
    if navigation.scene_stamp.player != actor {
        return Err(OwnershipLost);
    }
    let mut pose = world_pose(world, actor).ok_or(PlayerRemoved)?;
    match session.phase {
        // Stepped by load_session; the session reaches this match only after it.
        PlayerFixturePhase::Loading => return Ok(false),
        PlayerFixturePhase::Approaching => {
            if navigation.navigation_generation != session.prepared.navigation_generation {
                // Keep the destination chosen by AutoMoveTarget. A new nav
                // snapshot needs a fresh path, not a newly invented cancel
                // event solely because a generation number changed.
                let path = navigation
                    .world
                    .path(pose.translation, session.prepared.approach_hit)
                    .map_err(|_| NavigationFailed)?;
                session.prepared.approach_path =
                    provisional_approach_corners(path).map_err(|_| NavigationFailed)?;
                session.path_cursor = 0;
                session.prepared.navigation_generation = navigation.navigation_generation;
            }
            if dt <= 0.0 {
                return Ok(false);
            }
            while session.path_cursor < session.prepared.approach_path.len()
                && pose
                    .translation
                    .distance(session.prepared.approach_path[session.path_cursor])
                    <= 0.00001
            {
                session.path_cursor += 1;
            }
            let steering = session
                .prepared
                .approach_path
                .get(session.path_cursor)
                .copied()
                // Exhausting a Partial path never supplies a final straight
                // segment to the requested destination. Keep its last corner;
                // the provisional adapter still lacks a live agent's steering.
                .or_else(|| session.prepared.approach_path.last().copied())
                .ok_or(NavigationFailed)?;
            let delta = steering - pose.translation;
            let normal = if delta.length() <= 0.00001 {
                Vec3::ZERO
            } else {
                delta.normalize()
            };
            let desired = normal * navigation.agent_speed;
            // CalcVelocity's comparison is intentionally retained as written:
            // the longer displacement branch returns delta, not desired.
            let velocity = if delta.length_squared() > desired.length_squared() {
                delta
            } else {
                desired
            };
            let requested_step = velocity * dt;
            let requested = if requested_step.length_squared() >= delta.length_squared() {
                steering
            } else {
                pose.translation + requested_step
            };
            let accepted = navigation
                .world
                .constrain_move(pose.translation, requested)
                .ok_or(NavigationFailed)?;
            if !accepted.is_finite() {
                return Err(NavigationFailed);
            }
            // This quotient is the adapter's velocity estimate, not the source
            // NavMeshAgent.velocity readback. Exact agent stepping remains a
            // separate navigation-backend obligation, exposed in coverage.
            let adapter_velocity = (accepted - pose.translation) / dt;
            if adapter_velocity.length_squared() <= 0.01
                && pose.translation.distance(session.prepared.approach_hit) < 0.1
            {
                session.phase = PlayerFixturePhase::Fitting;
                // The local fitting coroutine reads StartLoc when it begins.
                // Legitimate fixture movement during approach is not a reason
                // to fit to the preparation-time world coordinate.
                let live_start = world
                    .get::<GlobalTransform>(session.prepared.target.entity)
                    .ok_or(TargetRemoved)?
                    .mul_transform(session.prepared.start_anchor_local)
                    .translation();
                session.movement = (pose.translation.distance(live_start) >= 0.01).then(|| {
                    LinearMove::new(pose.translation, live_start, navigation.default_move_speed)
                });
                return Ok(false);
            }
            if adapter_velocity.x != 0.0 || adapter_velocity.z != 0.0 {
                let target_rotation =
                    Quat::from_rotation_y(adapter_velocity.x.atan2(adapter_velocity.z));
                let angle = 2.0
                    * pose
                        .rotation
                        .dot(target_rotation)
                        .abs()
                        .min(1.0)
                        .acos()
                        .to_degrees();
                pose.rotation = if angle <= 0.027 {
                    target_rotation
                } else {
                    pose.rotation
                        .slerp(target_rotation, (dt / 0.04).clamp(0.0, 1.0))
                };
            }
            pose.translation = accepted;
            if accepted.distance(session.last_position) < 0.001 {
                session.unchanged_secs += dt;
            } else {
                session.unchanged_secs = 0.0;
            }
            session.last_position = accepted;
            if session.unchanged_secs >= 0.5 {
                return Err(NavigationFailed);
            }
            set_world_pose(world, actor, pose).ok_or(PlayerRemoved)?;
        }
        PlayerFixturePhase::Fitting => {
            let mut done = session.movement.is_none();
            if let Some(movement) = session.movement.as_mut() {
                let (position, t, finished) = movement.step(dt);
                let delta = position - pose.translation;
                if delta.x != 0.0 || delta.z != 0.0 {
                    let target_rotation = Quat::from_rotation_y(delta.x.atan2(delta.z));
                    pose.rotation = pose
                        .rotation
                        .lerp(target_rotation, (t * 1.5).clamp(0.0, 1.0))
                        .normalize();
                }
                pose.translation = position;
                set_world_pose(world, actor, pose).ok_or(PlayerRemoved)?;
                done = finished;
            }
            if done {
                attach_player(world, session)?;
                let request = session
                    .profile
                    .as_ref()
                    .ok_or(AnimationFailed)?
                    .start_request(session.prepared.owner, &session.prepared.target);
                fixture_activity_timeline::validate_start(world, &request)
                    .map_err(|_| AnimationFailed)?;
                // The lease taken for the run motion carries over: the
                // timeline samples this same animator from its start.
                if session.animation_lease.is_none() {
                    return Err(AnimationFailed);
                }
                log_character_rows(&request);
                session.timeline = Some(
                    world
                        .resource_mut::<FixtureActivityTimelines>()
                        .request_start(request),
                );
                session.phase = PlayerFixturePhase::StartingTimeline;
                session.movement = None;
            }
        }
        PlayerFixturePhase::StartingTimeline | PlayerFixturePhase::Playing => {
            let token = session.timeline.ok_or(AnimationFailed)?;
            let state = {
                let timelines = world.resource::<FixtureActivityTimelines>();
                match timelines.status(token) {
                    Some(TimelineStatus::Preparing) => 0,
                    Some(TimelineStatus::Playing { loop_started, .. }) => {
                        if *loop_started {
                            2
                        } else {
                            1
                        }
                    }
                    Some(TimelineStatus::Completed) => 3,
                    Some(TimelineStatus::Cancelled | TimelineStatus::Failed(_)) | None => {
                        return Err(AnimationFailed);
                    }
                }
            };
            if state >= 1 {
                session.phase = PlayerFixturePhase::Playing;
            }
            if session.end_requested && !session.end_dispatched && state == 2 {
                session.end_dispatched = world
                    .resource_mut::<FixtureActivityTimelines>()
                    .request_end(token);
            }
            if state == 3 {
                detach_player(world, session, false);
                release_timeline(world, session);
                pose = world_pose(world, actor).ok_or(PlayerRemoved)?;
                let current_hit = navigation
                    .world
                    .sample_position(pose.translation, NAV_SAMPLE_DISTANCE)
                    .ok_or(NavigationFailed)?;
                if !current_hit.is_finite()
                    || current_hit.distance(pose.translation) > NAV_SAMPLE_DISTANCE
                {
                    return Err(NavigationFailed);
                }
                // TryMoveEndPoint: already on the sampled surface means only
                // restoring the saved EndLoc orientation (a zero-length
                // DORotate), not forcing a walk.
                if current_hit.distance(pose.translation) < 0.01 {
                    pose.rotation = session.prepared.end.rotation;
                    set_world_pose(world, actor, pose).ok_or(PlayerRemoved)?;
                    return Ok(true);
                }
                let end = Vec3::from(session.prepared.end.position);
                let hit = navigation
                    .world
                    .sample_position(end, NAV_SAMPLE_DISTANCE)
                    .ok_or(NavigationFailed)?;
                if !hit.is_finite() || hit.distance(end) > NAV_SAMPLE_DISTANCE {
                    return Err(NavigationFailed);
                }
                session.movement = Some(LinearMove::new(pose.translation, hit, EXIT_SPEED));
                session.phase = PlayerFixturePhase::Exiting;
                // ChangeAnimation(AvatarConfig.WalkMotion), then MoveEndPoint.
                play_avatar_motion(world, actor, WALK_MOTION, session.animation_lease)
                    .map_err(|_| AnimationFailed)?;
                world.entity_mut(actor).insert(MotionPhase::Walking);
            }
        }
        PlayerFixturePhase::Exiting => {
            let movement = session.movement.as_mut().ok_or(NavigationFailed)?;
            let (position, _, done) = movement.step(dt);
            let direction = movement.target - movement.start;
            if direction.x != 0.0 || direction.z != 0.0 {
                pose.rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));
            }
            pose.translation = position;
            set_world_pose(world, actor, pose).ok_or(PlayerRemoved)?;
            if done {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn attach_player(
    world: &mut World,
    session: &mut PlayerFixtureSession,
) -> Result<(), PlayerFixtureCancelReason> {
    let actor = session.prepared.owner.actor;
    let fixture = session.prepared.target.entity;
    let fixture_world = *world
        .get::<GlobalTransform>(fixture)
        .ok_or(PlayerFixtureCancelReason::TargetRemoved)?;
    let mut pose = world_pose(world, actor).ok_or(PlayerFixtureCancelReason::PlayerRemoved)?;
    let local = session.prepared.start_anchor_local;
    let anchor_world = fixture_world.mul_transform(local);
    pose.translation = anchor_world.translation();
    pose.rotation = anchor_world.to_scale_rotation_translation().1;
    // Mirror the selected instance's StartLoc transform in an owned root.
    // Parenting the logical player beneath a despawnable scene root would
    // recursively delete it before finally can run. The explicit composition
    // preserves parent-motion behavior without inheriting despawn ownership.
    let anchor = world
        .spawn((anchor_world.compute_transform(), anchor_world))
        .id();
    session.anchor = Some(anchor);
    session.anchor_local = Some(local);
    world
        .entity_mut(actor)
        .insert((ChildOf(anchor), MotionPhase::Dwelling { remaining: None }));
    set_world_pose(world, actor, pose).ok_or(PlayerFixtureCancelReason::PlayerRemoved)?;
    Ok(())
}

fn world_pose(world: &World, actor: Entity) -> Option<Transform> {
    let local = *world.get::<Transform>(actor)?;
    match world.get::<ChildOf>(actor) {
        Some(parent) => Some(
            world
                .get::<GlobalTransform>(parent.parent())?
                .mul_transform(local)
                .compute_transform(),
        ),
        None => Some(local),
    }
}

fn set_world_pose(world: &mut World, actor: Entity, pose: Transform) -> Option<()> {
    let global = GlobalTransform::from(pose);
    let local = match world.get::<ChildOf>(actor) {
        Some(parent) => global.reparented_to(world.get::<GlobalTransform>(parent.parent())?),
        None => pose,
    };
    world.entity_mut(actor).insert((local, global));
    Some(())
}

fn detach_player(world: &mut World, session: &mut PlayerFixtureSession, site_changed: bool) {
    let actor = session.prepared.owner.actor;
    let owns_parent = session.anchor.is_some_and(|anchor| {
        world
            .get::<ChildOf>(actor)
            .is_some_and(|parent| parent.parent() == anchor)
    });
    if owns_parent && world.get::<Transform>(actor).is_some() {
        let pose = if site_changed {
            None
        } else {
            world_pose(world, actor)
        };
        world.entity_mut(actor).remove::<ChildOf>();
        if !site_changed {
            if let Some(parent) = session
                .before_parent
                .filter(|parent| world.get::<GlobalTransform>(*parent).is_some())
            {
                world.entity_mut(actor).insert(ChildOf(parent));
            }
            if let Some(pose) = pose {
                set_world_pose(world, actor, pose);
            }
        }
        // On site change the reseeder owns position. Do not reapply an old
        // attachment, previous approach position, or saved EndLoc to a new site.
    }
    if let Some(anchor) = session.anchor.take() {
        session.anchor_local = None;
        // Never recursively remove a child adopted by a different owner.
        let has_children = world
            .get::<Children>(anchor)
            .is_some_and(|children| !children.is_empty());
        if !has_children {
            if let Ok(entity) = world.get_entity_mut(anchor) {
                entity.despawn();
            }
        }
    }
}

/// The runner's session ends; the timeline's own nodes stop and its poses are
/// restored when the runner releases it. The avatar lease stays with the
/// session, which plays the state's own motions on it until its finally.
fn release_timeline(world: &mut World, session: &mut PlayerFixtureSession) {
    if let Some(token) = session.timeline.take() {
        if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
            timelines.cancel(token);
            timelines.release(token);
        }
    }
}

/// The finally's animation half: the timeline is released, then the avatar
/// plays IdleMotion and enters the Idle state (`play_finally_idle`). Only an
/// avatar whose animator has gone releases the lease without playing.
fn release_animation(world: &mut World, session: &mut PlayerFixtureSession) {
    release_timeline(world, session);
    let lease = session.animation_lease.take();
    let actor = session.prepared.owner.actor;
    let animator = world.get::<AvatarDriver>(actor).map(|driver| driver.player);
    let Some(animator) = animator else {
        return;
    };
    if world.get::<AnimationPlayer>(animator).is_some() {
        play_finally_idle(world, actor, lease);
        return;
    }
    let Some(token) = lease else {
        return;
    };
    // The avatar body's animator has gone while the logical player remains.
    // Clear only our driver lease using an inert player value; there is no
    // live animator left to stop, but global/slot cleanup must continue.
    if let Some(mut driver) = world.get_mut::<AvatarDriver>(actor) {
        driver.release_fixture_timeline(token, &mut AnimationPlayer::default());
    }
}

fn finish(world: &mut World, mut session: PlayerFixtureSession, site_changed: bool) {
    release_animation(world, &mut session);
    detach_player(world, &mut session, site_changed);
    let actor = session.prepared.owner.actor;
    let held = world.get::<PlayerFixtureHeld>(actor).map(|held| held.0);
    let owns_controls = world
        .get_resource::<PlayerFixtureControlOwner>()
        .is_some_and(|lease| lease.0 == session.prepared.owner);
    if held == Some(session.prepared.owner) {
        world.entity_mut(actor).remove::<PlayerFixtureHeld>();
        world.entity_mut(actor).insert((
            DashMode(session.before_dash),
            MotionPhase::Dwelling { remaining: None },
        ));
        if let Some(mut input) = world.get_mut::<PlayerInput>(actor) {
            input.active = false;
            input.direction = Vec3::ZERO;
        }
    }
    if owns_controls {
        world.remove_resource::<PlayerFixtureControlOwner>();
        // A different live holder has already taken over. Do not reopen its
        // intercept gate or change its state in a delayed finally callback.
        if held.is_none_or(|owner| owner == session.prepared.owner) {
            if let Some(mut states) = world.get_resource_mut::<PlayerAvatarStates>() {
                if states.current == PlayerActionState::UseTimelineFixture {
                    states.can_intercept = true;
                    states.change_status(PlayerActionState::Idle);
                }
            }
        }
    }
    if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
        reservations.release_owner(session.prepared.owner);
    }
}

/// The timeline's CharacterAnimator rows, as the runner is asked to play
/// them on the avatar: the clip name, its start and duration on the axis, and
/// the clip's own source length and loop flag.
fn log_character_rows(request: &StartTimeline) {
    let rows: Vec<String> = request
        .definition
        .tracks
        .iter()
        .filter(|track| track.name == "CharacterAnimator")
        .flat_map(|track| &track.clips)
        .filter_map(|clip| match &clip.payload {
            TimelinePayload::Animation { target, .. } => {
                let source = request.bindings.animations.get(&clip.key)?;
                Some(format!(
                    "{} [{:.3}, {:.3}) in {:.3} x{:.2} source {:.3}s loop {}",
                    target.clip_name,
                    clip.start,
                    clip.end(),
                    clip.clip_in,
                    clip.time_scale,
                    source.source.stop_time - source.source.start_time,
                    source.source.looping
                ))
            }
            _ => None,
        })
        .collect();
    info!(
        "[player-fixture] timeline {} ({:.3}s) on the avatar: {}",
        request
            .definition
            .prefab
            .rsplit('/')
            .next()
            .unwrap_or(&request.definition.prefab),
        request.definition.duration,
        rows.join(" | ")
    );
}

fn capture_coverage(
    world: &World,
    session: &PlayerFixtureSession,
    runtime: &mut PlayerFixtureRuntime,
) {
    if let Some(token) = session.timeline {
        if let Some(coverage) = world
            .get_resource::<FixtureActivityTimelines>()
            .and_then(|timelines| timelines.coverage(token))
        {
            runtime.last_timeline_coverage = coverage.to_vec();
        }
    }
}

/// Explicit site teardown hook; invoke before player reseeding. The owned
/// attachment root is independent of fixture-scene recursive despawn.
pub(crate) fn cancel_for_site_change(world: &mut World) {
    cancel_runtime(world, true, PlayerFixtureCancelReason::SiteChanged);
}

/// Editing retains the actor/site. Use the existing normal finally path so
/// only this generation's animation/control leases are released and the player
/// detaches in the current world; do not pretend that the site was destroyed.
pub(crate) fn cancel_for_layout_edit(world: &mut World) {
    cancel_runtime(world, false, PlayerFixtureCancelReason::User);
}

fn cancel_runtime(world: &mut World, site_changed: bool, reason: PlayerFixtureCancelReason) {
    let Some(mut runtime) = world.remove_resource::<PlayerFixtureRuntime>() else {
        return;
    };
    runtime.pending.clear();
    runtime.availability.clear();
    runtime.availability_sweep = AvailabilitySweep::default();
    if let Some(session) = runtime.session.take() {
        capture_coverage(world, &session, &mut runtime);
        finish(world, session, site_changed);
        runtime.last_outcome = Some(PlayerFixtureOutcome::Cancelled(reason));
    }
    world.insert_resource(runtime);
}
