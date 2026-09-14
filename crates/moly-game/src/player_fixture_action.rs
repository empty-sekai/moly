//! Player furniture requests and the lifetime around a shared visual timeline.
//!
//! Player locator selection remains a player-table operation. The selected SD
//! profile changes presentation, not the actor, fixture, slot, or exit rules.
//! The common timeline runner owns all visual sampling and exact companion SE
//! tracks. This module owns approach, attachment, end requests and cleanup.
//!
//! Navigation data is supplied by the scene owner. Missing tiles, agent values,
//! or visual mappings are unfinished preparation, never successful eligibility.

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;

use crate::{
    fixture_activity_data::{FixtureActivityTables, PlayerTimelineRow},
    fixture_activity_state::{
        FixtureActivityIdentity, FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
    },
    fixture_activity_timeline::{
        self, FixtureActivityTimelines, StartTimeline, TimelineBindings, TimelineCompanionTrack,
        TimelineCoverageGap, TimelineDefinition, TimelineOwner, TimelineOwnerKind, TimelineStatus,
        TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::{AttachPoints, AttachPose},
    npc::{CharacterUnitId, MotionPhase},
    player::{DashMode, PlayerControlled, PlayerInput},
    player_avatar::{AvatarDriver, PlayerActionToken},
    player_state::{PlayerActionState, PlayerAvatarStates},
    site::GroundEpoch,
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
    Pending(PlayerFixturePreparationError),
    Unavailable(&'static str),
}

impl PlayerFixtureAvailability {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// The public master-data owner supplies this join and its exact bindings.
/// NoTalk supplies only the visible action; it never supplies player AI rules.
#[derive(Clone)]
pub(crate) struct PlayerFixtureVisualProfile {
    pub target: FixtureTarget,
    pub actor: Entity,
    pub unit_id: u32,
    pub fixture_id: i32,
    pub player_timeline_row_id: i32,
    pub action_point: i32,
    pub visual_action_point: i32,
    pub source_view_local_y: f32,
    /// Parsed from the original locator suffix by the shared data supplier.
    pub slot_id: i32,
    pub no_talk_row_id: i32,
    pub timeline_group_id: i32,
    pub timeline_row_id: i32,
    pub definition: Arc<TimelineDefinition>,
    pub player_definition: Arc<TimelineDefinition>,
    pub bindings: TimelineBindings,
    /// Visible to callers during and after a session. Native IK/offset gaps
    /// must not disappear behind a successful animation-resource preflight.
    pub coverage_notes: Vec<String>,
}

#[derive(Resource, Default)]
pub(crate) struct PlayerFixtureVisualProfiles {
    pub profiles: Vec<PlayerFixtureVisualProfile>,
}

impl PlayerFixtureVisualProfile {
    /// Asset preparation only. Keep this profile alive while sounds load,
    /// then publish it through the shared profile resource. This does not
    /// allocate an activity generation or submit a timeline to the runner.
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

    pub(crate) fn start_request(&self, owner: FixtureActivityOwner, target: &FixtureTarget) -> StartTimeline {
        StartTimeline {
            owner: TimelineOwner {
                activity: owner,
                kind: TimelineOwnerKind::Player,
            },
            fixture: target.entity,
            definition: self.definition.clone(),
            bindings: self.bindings.clone(),
            // Retain every referenced source player SE track, including an
            // empty track. The caller cannot silently omit the sitting sound
            // by supplying an empty companion list. No player body is copied.
            companions: self
                .player_definition
                .tracks
                .iter()
                .filter(|track| track.class == "SETrack")
                .map(|track| TimelineCompanionTrack {
                    source: self.player_definition.clone(),
                    track: track.identity.clone(),
                })
                .collect(),
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
    profile: PlayerFixtureVisualProfile,
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
    NotPrepared(PlayerFixturePreparationError),
    /// Gimmick is a distinct state-10 workflow. It is not a seat timeline.
    GimmickNotPrepared {
        target: FixtureTarget,
        reason: String,
    },
    GimmickStarted { target: FixtureTarget },
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
    pub last_outcome: Option<PlayerFixtureOutcome>,
    last_profile: Option<PlayerFixtureVisualProfile>,
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
    pub(crate) fn profile(&self) -> Option<&PlayerFixtureVisualProfile> {
        self.session
            .as_ref()
            .map(|session| &session.prepared.profile)
            .or(self.last_profile.as_ref())
    }
    pub(crate) fn navigation_coverage(&self) -> Option<&[String]> {
        self.session
            .as_ref()
            .map(|session| session.prepared.navigation_coverage.as_slice())
            .or_else(|| {
                self.last_profile
                    .as_ref()
                    .map(|_| self.last_navigation_coverage.as_slice())
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

/// Preparation and the visible button use the same actual binding validator.
/// This publishes readiness; it does not change the existing button stack.
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
    runtime.availability.clear();
    for target in targets {
        let state = if runtime.active() {
            PlayerFixtureAvailability::Unavailable("player furniture session owns input")
        } else {
            match prepare(world, &target, runtime.generation.saturating_add(1)) {
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
        };
        runtime.availability.insert(target, state);
    }
    world.insert_resource(runtime);
}

fn prepare(
    world: &mut World,
    target: &FixtureTarget,
    generation: u64,
) -> Result<PreparedPlayerFixture, PlayerFixturePreparationError> {
    use PlayerFixturePreparationError::{Invalid, Missing, Rejected, Timeline};
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
    let states = world
        .get_resource::<PlayerAvatarStates>()
        .ok_or(Missing("player state owner"))?;
    if !states.can_intercept {
        return Err(Rejected("player intercept gate is closed"));
    }
    let (actor, unit, position, player_animator) = {
        let mut query = world
            .query_filtered::<(Entity, &CharacterUnitId, &AvatarDriver), With<PlayerControlled>>();
        let mut players = query.iter(world);
        let (actor, unit, driver) = players.next().ok_or(Missing("installed SD player"))?;
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
            unit.0,
            world_pose(world, actor)
                .ok_or(Missing("player world pose"))?
                .translation,
            driver.player,
        )
    };
    let owner = FixtureActivityOwner { actor, generation };
    if navigation.scene_stamp.player != actor {
        return Err(Missing("navigation parameters for the current logical player"));
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
    let rows: Vec<_> = points.player_action_points(&identity.model_package)
        .ok_or(Missing("original player locator array metadata"))?
        .into_iter()
        .filter_map(|point| source_rows.iter().find(|row| row.action_point == point).cloned())
        .collect();

    struct Candidate {
        row: PlayerTimelineRow,
        locate_index: usize,
        slot_id: i32,
        start: AttachPose,
        end: AttachPose,
    }
    let mut nearest: Option<(f32, Candidate)> = None;
    // Stage one has no SD profile, animation readiness, or NoTalk dependency.
    // Those may delay the chosen source seat, but cannot select another seat.
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
        if !can_move_to_locator(&navigation, position, start, &target.uid)?
            || !can_move_to_locator(&navigation, position, Vec3::from(end.position), &target.uid)?
        {
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

    // Stage two resolves only the source-selected seat's presentation.
    let profiles = world
        .get_resource::<PlayerFixtureVisualProfiles>()
        .ok_or(Missing("SD visual profile join"))?;
    let matches: Vec<&PlayerFixtureVisualProfile> = profiles
        .profiles
        .iter()
        .filter(|profile| {
            profile.actor == actor
                && profile.target == *target
                && profile.unit_id == unit
                && profile.fixture_id == identity.master_id
                && profile.player_timeline_row_id == selected.row.id
                && profile.action_point == selected.row.action_point
        })
        .collect();
    let profile = match matches.as_slice() {
        [profile] => *profile,
        [] => {
            return Err(Missing(
                "source SD action mapping for selected player locator",
            ))
        }
        _ => {
            return Err(Invalid(
                "ambiguous SD visual profile for selected player locator".into(),
            ))
        }
    };
    if profile.visual_action_point != selected.row.action_point
        || profile.slot_id != selected.slot_id
    {
        return Err(Invalid(
            "SD profile changes the source player action point or slot".into(),
        ));
    }
    validate_visual_relation(tables, profile, &selected.row)?;
    if !profile
        .bindings
        .animations
        .values()
        .any(|binding| binding.animator == player_animator)
    {
        return Err(Missing("actual SD player animation binding"));
    }
    if profile.bindings.animations.values().any(|binding| {
        binding.animator != player_animator
            && !is_descendant_of(world, binding.animator, target.entity)
    }) || profile
        .bindings
        .actors
        .values()
        .any(|bound| *bound != actor && *bound != target.entity)
    {
        return Err(Invalid(
            "player visual bindings target a nonparticipant actor".into(),
        ));
    }
    let request = profile.start_request(owner, target);
    fixture_activity_timeline::validate_start(world, &request)
        .map_err(|error| Timeline(format!("{error:?}")))?;
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
        profile: profile.clone(),
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

fn validate_visual_relation(
    tables: &FixtureActivityTables,
    profile: &PlayerFixtureVisualProfile,
    player_row: &PlayerTimelineRow,
) -> Result<(), PlayerFixturePreparationError> {
    use PlayerFixturePreparationError::{Invalid, Missing};
    let no_talk = tables
        .sd_visual_rows(profile.unit_id, profile.fixture_id)
        .find(|row| row.id == profile.no_talk_row_id)
        .ok_or(Missing("NoTalk visual relation"))?;
    if no_talk.timeline_group_id != profile.timeline_group_id {
        return Err(Invalid(
            "SD profile's NoTalk timeline group does not match its source row".into(),
        ));
    }
    let timeline = tables
        .timeline_rows(no_talk.timeline_group_id)
        .find(|row| row.id == profile.timeline_row_id)
        .ok_or(Missing("source SD timeline row"))?;
    // A NoTalk visual row describes one character; source action-point column
    // one is its slot in the character group, not its furniture action slot.
    let point = tables
        .action_point_value(timeline.action_point_definition, 0)
        .map_err(|error| Invalid(format!("SD action-point definition: {error:?}")))?;
    if point != Some(player_row.action_point) || point != Some(profile.visual_action_point) {
        return Err(Invalid(
            "source SD and player action-point definitions differ".into(),
        ));
    }
    let master = tables
        .fixture_master(profile.fixture_id)
        .ok_or(Missing("source fixture master for visual variant"))?;
    let (expected_package, expected_prefab) = crate::fixture_activity_data::timeline_asset(
        &timeline.asset_name,
        master.put_type,
        profile.source_view_local_y,
    )
    .map_err(|error| Invalid(format!("visual timeline variant: {error:?}")))?;
    if profile.definition.package != expected_package
        || profile.definition.prefab != expected_prefab
    {
        return Err(Invalid(
            "SD profile definition is not the source timeline prefab".into(),
        ));
    }
    let name = profile
        .player_definition
        .prefab
        .rsplit('/')
        .next()
        .unwrap_or(&profile.player_definition.prefab);
    if name.strip_suffix(".prefab").unwrap_or(name) != player_row.asset_name {
        return Err(Invalid(
            "player companion source is not the selected player timeline prefab".into(),
        ));
    }
    Ok(())
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

/// The normal navmesh branch and the fixture-tile fallback are OR branches at
/// each endpoint. An occupied target tile is not the same as another UID.
fn can_move_to_locator(
    navigation: &PlayerFixtureNavigation,
    player: Vec3,
    locator: Vec3,
    target_uid: &str,
) -> Result<bool, PlayerFixturePreparationError> {
    use FixtureTileKnowledge::*;
    let tile = navigation.world.tile_at(locator);
    if matches!(tile, Unknown) {
        return Err(PlayerFixturePreparationError::Missing(
            "locator tile identity",
        ));
    }
    if !matches!(tile, Missing) {
        if let Some(hit) = navigation
            .world
            .sample_position(locator, navigation.agent_radius)
        {
            if hit.is_finite() && hit.distance(locator) <= navigation.agent_radius {
                let path = navigation.world.path(player, hit)?;
                // CanNavmeshMoveTargetPosition uses CalculatePath's boolean,
                // then the last corner's strictly-less horizontal distance.
                // It does not compare height or require PathComplete.
                if path.query_succeeded {
                    let last = path.corners.last().ok_or_else(|| {
                        PlayerFixturePreparationError::Invalid(
                            "successful navigation query has no last corner".into(),
                        )
                    })?;
                    if !last.is_finite() {
                        return Err(PlayerFixturePreparationError::Invalid(
                            "navigation returned a nonfinite last corner".into(),
                        ));
                    }
                    if Vec2::new(last.x - hit.x, last.z - hit.z).length() < 0.01 {
                        return Ok(true);
                    }
                }
            }
        }
    }
    // Repeated positions may map to the same tile. tile_at is an immutable
    // lookup, so repeating that read preserves the deduplicated source result.
    for x in [-1.0, 0.0, 1.0] {
        for z in [-1.0, 0.0, 1.0] {
            let p = locator
                + Vec3::new(
                    x * navigation.agent_radius,
                    0.0,
                    z * navigation.agent_radius,
                );
            match navigation.world.tile_at(p) {
                Unknown | ExistingUnknownOccupant => {
                    return Err(PlayerFixturePreparationError::Missing(
                        "locator radius tile identities",
                    ));
                }
                Missing => return Ok(false),
                Occupied(uid) if uid != target_uid => return Ok(false),
                Empty | Occupied(_) => {}
            }
        }
    }
    Ok(true)
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
                match prepare(world, &target, next).and_then(|prepared| begin(world, prepared)) {
                    Ok(session) => {
                        runtime.generation = next;
                        runtime.last_outcome = None;
                        runtime.last_timeline_coverage.clear();
                        runtime.last_profile = Some(session.prepared.profile.clone());
                        runtime.last_navigation_coverage =
                            session.prepared.navigation_coverage.clone();
                        runtime.session = Some(session);
                    }
                    Err(error) => {
                        runtime.last_outcome = Some(PlayerFixtureOutcome::NotPrepared(error))
                    }
                }
            }
            PlayerFixtureRequest::Gimmick(target) => {
                let next = runtime.generation.checked_add(1).expect("player activity generation exhausted");
                match crate::fixture_gimmick::start(world, &target, next) {
                    Ok(()) => {
                        runtime.generation = next;
                        runtime.last_outcome = Some(PlayerFixtureOutcome::GimmickStarted { target });
                    }
                    Err(reason) => {
                        warn!("[fixture-gimmick] {} request not prepared: {reason}", target.uid);
                        runtime.last_outcome = Some(PlayerFixtureOutcome::GimmickNotPrepared { target, reason });
                    }
                }
            }
        }
    }
    if let Some(mut session) = runtime.session.take() {
        capture_coverage(world, &session, &mut runtime);
        match step_session(world, &mut session, dt) {
            Ok(false) => runtime.session = Some(session),
            Ok(true) => {
                finish(world, session, false);
                runtime.last_outcome = Some(PlayerFixtureOutcome::Completed);
            }
            Err(reason) => {
                finish(
                    world,
                    session,
                    reason == PlayerFixtureCancelReason::SiteChanged,
                );
                runtime.last_outcome = Some(PlayerFixtureOutcome::Cancelled(reason));
            }
        }
    }
    world.insert_resource(runtime);
}

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
        MotionPhase::Walking,
        DashMode(true),
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
        phase: PlayerFixturePhase::Approaching,
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

fn step_session(
    world: &mut World,
    session: &mut PlayerFixtureSession,
    dt: f32,
) -> Result<bool, PlayerFixtureCancelReason> {
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
    if navigation.scene_stamp.player != actor { return Err(OwnershipLost); }
    let mut pose = world_pose(world, actor).ok_or(PlayerRemoved)?;
    match session.phase {
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
                    .prepared
                    .profile
                    .start_request(session.prepared.owner, &session.prepared.target);
                fixture_activity_timeline::validate_start(world, &request)
                    .map_err(|_| AnimationFailed)?;
                let lease = world
                    .get_mut::<AvatarDriver>(actor)
                    .ok_or(AnimationFailed)?
                    .acquire_fixture_timeline()
                    .map_err(|_| AnimationFailed)?;
                session.animation_lease = Some(lease);
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
                release_animation(world, session);
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
                // restoring the saved EndLoc orientation, not forcing a walk.
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
                world
                    .entity_mut(actor)
                    .insert((MotionPhase::Walking, DashMode(false)));
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

fn release_animation(world: &mut World, session: &mut PlayerFixtureSession) {
    if let Some(token) = session.timeline.take() {
        if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
            timelines.cancel(token);
            timelines.release(token);
        }
    }
    let Some(token) = session.animation_lease.take() else {
        return;
    };
    let actor = session.prepared.owner.actor;
    let animator = world.get::<AvatarDriver>(actor).map(|driver| driver.player);
    let Some(animator) = animator else {
        return;
    };
    let Some(mut player) = world
        .get_entity_mut(animator)
        .ok()
        .and_then(|mut entity| entity.take::<AnimationPlayer>())
    else {
        // The SD body may have been removed while the logical player remains.
        // Clear only our driver lease using an inert player value; there is no
        // live animator left to stop, but global/slot cleanup must continue.
        if let Some(mut driver) = world.get_mut::<AvatarDriver>(actor) {
            driver.release_fixture_timeline(token, &mut AnimationPlayer::default());
        }
        return;
    };
    if let Some(mut driver) = world.get_mut::<AvatarDriver>(actor) {
        driver.release_fixture_timeline(token, &mut player);
    }
    world.entity_mut(animator).insert(player);
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
    if let Some(session) = runtime.session.take() {
        capture_coverage(world, &session, &mut runtime);
        finish(world, session, site_changed);
        runtime.last_outcome = Some(PlayerFixtureOutcome::Cancelled(reason));
    }
    world.insert_resource(runtime);
}
