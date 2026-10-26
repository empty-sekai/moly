//! Autonomous single-NPC, no-dialogue furniture activities.
//!
//! The selected action row, actual placed Entity/UID, locator array index and
//! source timeline survive approach, loading, playback and disposal together.
//! Admission runs no path query: the first fixture passing the locator sample
//! (within half a tile), tile, slot, occupancy, UnmovableFixtureList, 1000 m,
//! motion-area and floor-level (a stacked fixture needs a non-block fixture
//! below it) checks is selected even when it cannot be reached; the
//! objective's departure gate tests reachability once, for that target only.
//! This is not a NavMesh implementation: the locator sample and the approach
//! use the existing, explicitly approximate WalkField/ObjectiveFace host.

mod areas;
pub mod companion;
pub mod preview;
pub use areas::NpcFixtureAreas;

use std::collections::{HashMap, HashSet};

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_assets::{player_data::mirror_fixture_layout, scene_state::SourceInactive};
use moly_law::{
    fixture::{
        GridPosition,
        position::{TILE_SIZE, layout_type},
    },
    objective::TalkType,
    path::{
        NpcPathWalkSlot, WaypointDraw, angle_between, heading_yaw, make_positive_yaw, rotate_time,
        turn_angle, turn_motion,
    },
};

use crate::{
    character::MotionDriver,
    fixture::{FixturePlacements, OccupancyRow},
    fixture_activity_data::{
        ActivityKey, ActivityOrigin, ActivitySpec, ActivityTimeline, FixtureActivityTables, PutType,
    },
    fixture_activity_provider::{FixtureActivityProvider, SourceFixtureViewLocalY},
    fixture_activity_state::{
        FixtureActivityIdentity, FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
    },
    fixture_activity_timeline::{
        self, FixtureActivityTimelines, StartTimeline, TimelineBindings, TimelineOwner,
        TimelineOwnerKind, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::AttachPoints,
    fixture_scene_inputs::{FixtureScenePlacement, FixtureSceneSupply},
    fixture_tiles::{FixtureFloorTiles, OccupancyCoverage},
    npc::{
        CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, PauseSeconds, RestLifecycle,
        RouteOutcome, RouteStops, WalkSpeed, WalkState,
    },
    npc_objective::{AiTalkData, MemberRng, ObjectiveFace, ObjectiveMind, TalkSlot},
    player_fixture_action::FixtureTileKnowledge,
    talk::TalkHold,
};

/// Movement ownership starts at arrival, not while an ordinary route is live.
/// It also protects loading/exit from npc::advance's provisional reentry warp.
#[derive(Component, Clone, Copy)]
pub struct NpcFixtureMotionOwner(pub FixtureActivityOwner);

/// `MysekaiConstants.HALF_TILE_SIZE.x`: half the tile size. The action-point
/// check compares the whole 3-D distance of the sample hit against it.
const HALF_TILE_SIZE: f32 = TILE_SIZE * 0.5;

/// IsAvailableFixtureCasePutStatus after its motion-area test. A fixture whose
/// layout Center is on grid level 0 passes. A stacked one passes only when the
/// tile directly below its Center, in its own (floor) layout, holds a fixture
/// that is not a block: `MysekaiFixtureUtility.IsBlock` is true for the handle
/// types block and block_transparent, and false when the master is missing.
/// An empty or off-grid tile below fails.
fn stands_on_usable_tile(
    row: &OccupancyRow,
    instances: &[(
        Entity,
        &FixtureActivityIdentity,
        &GlobalTransform,
        &OccupancyRow,
    )],
    floor: &FixtureFloorTiles,
    tables: &FixtureActivityTables,
) -> Result<bool, FactoryIssue> {
    if row.center_y == 0 {
        return Ok(true);
    }
    if row.layout != layout_type::FLOOR {
        return Err(FactoryIssue::Gap(format!(
            "{} stands above grid level 0 on a non-floor layout; this host keeps tile occupancy for the floor layout alone",
            row.uid
        )));
    }
    let below = cell_below_center(row).map_err(FactoryIssue::Gap)?;
    match floor.tile_at_grid(below) {
        FixtureTileKnowledge::Empty | FixtureTileKnowledge::Missing => Ok(false),
        FixtureTileKnowledge::Occupied(uid) => {
            let (_, occupant, _, _) = instances
                .iter()
                .find(|(_, identity, _, _)| identity.uid == uid)
                .ok_or_else(|| {
                    FactoryIssue::Gap(format!(
                        "tile below {} belongs to {uid}, which has no resolved placement",
                        row.uid
                    ))
                })?;
            let block = tables
                .fixture_master(occupant.master_id)
                .is_some_and(|master| {
                    matches!(master.handle_type.as_str(), "block" | "block_transparent")
                });
            Ok(!block)
        }
        FixtureTileKnowledge::ExistingUnknownOccupant | FixtureTileKnowledge::Unknown => Err(
            FactoryIssue::Gap(format!("tile below {} has no resolved occupant", row.uid)),
        ),
    }
}

/// `GetGridPosition() + GridPosition.Down` in this host's floor grid.
/// GetGridPosition is the layout Center in the source grid frame, whose X axis
/// this host's floor grid mirrors (x -> -x - 1); the source adds Down with
/// signed-byte arithmetic.
fn cell_below_center(row: &OccupancyRow) -> Result<GridPosition, String> {
    let (center, _, _) = mirror_fixture_layout(
        row.layout_center,
        row.layout_grid_size,
        row.direction,
        row.layout,
    )?;
    Ok(GridPosition::new(
        (-i16::from(center.x) - 1) as i8,
        center.y.wrapping_sub(1),
        center.z,
    ))
}

/// `FieldPosition.ToGridPosition` of a position relative to the site: each
/// axis is floored by the tile size, the height after clamping at zero, and
/// the source narrows the result to its signed-byte cell without a range
/// check (an out-of-domain cell then reads as the null tile).
fn source_grid(floor: &FixtureFloorTiles, world: [f32; 3]) -> GridPosition {
    let relative = Vec3::from(world) - floor.site_origin;
    let cell = |value: f32| (value / TILE_SIZE).floor() as i32 as i8;
    GridPosition::new(
        cell(relative.x),
        cell(relative.y.max(0.0)),
        cell(relative.z),
    )
}

/// Separate from Rest's alone_holds: the Rest script may dispose without
/// clearing the actual furniture timeline's animation lease.
#[derive(Component, Clone, Copy)]
pub struct NpcFixtureAnimationOwner(pub FixtureActivityOwner);

/// The draws one no-talk factory call made on the member generator: one
/// sort key per no-talk row (the source orders the rows by a fresh Guid
/// each), then, on a hit, one engine range over the chosen group's
/// timelines (value, length).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoneTalkDraws {
    pub keys: usize,
    pub timeline_pick: Option<(usize, usize)>,
}

/// One placement resolved against its live instance.
type LiveInstance<'a> = (
    Entity,
    &'a FixtureActivityIdentity,
    &'a GlobalTransform,
    &'a OccupancyRow,
);

/// A playable action point of one placed fixture: the locator's source array
/// index and slot, and the StartLoc pose with the height of the site.
struct PlayablePoint {
    locate_index: usize,
    slot_id: i32,
    position: [f32; 3],
    rotation: Quat,
    /// The StartLoc's own world position.
    start_loc: [f32; 3],
}

#[derive(Clone)]
pub struct Selection {
    /// NoneTalk for a no-talk action; SingleCharacterFixture for a fixture
    /// talk whose pre-action carries a timeline.
    kind: TalkType,
    actor: Entity,
    unit: u32,
    pub target: FixtureTarget,
    identity: FixtureActivityIdentity,
    site_epoch: u64,
    source: ActivityKey,
    tweet: Option<moly_law::talk::TweetRef>,
    timeline: ActivityTimeline,
    point: i32,
    locate_index: usize,
    slot_id: i32,
    put_type: PutType,
    pub position: [f32; 3],
    pub rotation: Quat,
    /// The StartLoc's own world position (its own height).
    pub start_loc: [f32; 3],
}

impl Selection {
    /// The NoTalk master row id, for an action from that table.
    pub fn no_talk_row(&self) -> Option<i32> {
        match self.source.origin {
            ActivityOrigin::NoTalk(id) => Some(id),
            _ => None,
        }
    }

    /// The fixture timeline the action plays.
    pub fn timeline_id(&self) -> i32 {
        self.timeline.id
    }

    pub fn ai_data(&self) -> AiTalkData {
        AiTalkData {
            kind: self.kind,
            content: None, // A no-dialogue action has no fabricated talk master.
            target_fixture: Some(self.target.entity),
            target_position: self.position,
            main_character: self.unit,
            characters: vec![self.unit],
            pre_action: None,
            locate: None,
            pending_factory: None,
        }
    }
}

enum Phase {
    Approaching,
    /// A fixture talk after its arrival and before its timeline: the
    /// pre-action tweet, then the wait until the character is not talking.
    TalkWindow(TalkWindow),
    HfsmLooking,
    HfsmNoneLooking,
    HfsmApproach(ActionPointApproach),
    Preparing,
    Playing,
    /// The source Director has completed, but the admitted talk still owns
    /// this actor/fixture. Native `PlayAsyncForNPC` stops its Director before
    /// the talk handoff; keep the activity reservation without re-running the
    /// sparse end pose on every frame.
    TalkHeld,
    Exiting(LocalMove),
}

/// TryPlayTweetAsync's two waits, each polled from the frame after the one
/// that created it: the tweet state Done, then not talking.
#[derive(Clone, Copy)]
enum TalkWindow {
    TweetDone { since: u32 },
    HfsmTweet { elapsed: f32 },
    NotTalking { since: u32 },
}

struct Session {
    owner: FixtureActivityOwner,
    selection: Selection,
    phase: Phase,
    request: Option<StartTimeline>,
    token: Option<TimelineToken>,
    animator: Option<Entity>,
    previous_enable_talk: bool,
    last_pending: Option<String>,
    preview_ticket: Option<u64>,
    waiting_seconds: f32,
    tweet_issued: bool,
    /// The shared timeline's talk flag as last seen: the loop clip with the
    /// talk flag is entered on its rising edge.
    talk_clip: bool,
}

/// One placed fixture as the fixture-talk factories read it (see
/// [`Factory::talk_fixture_geometry`]).
pub struct TalkFixtureGeometry {
    pub entity: Entity,
    pub locators: Vec<(String, crate::fixture_attach::AttachPair)>,
    pub footprint: ((i32, i32), (i32, i32)),
    pub floor_bounds: ((i32, i32), (i32, i32)),
    pub site_origin: [f32; 3],
}

#[derive(Resource, Default)]
pub struct NpcFixtureActivities {
    next_generation: u64,
    sessions: HashMap<Entity, Session>,
    pending_factories: HashMap<Entity, String>,
    /// Host data gaps already reported for the no-talk factory. Each is
    /// reported once until a layout edit or site change can alter it.
    reported_gaps: HashSet<String>,
    /// No-talk data a forced objective set on a character (the home
    /// controller's random fixture action): the decision that finds the
    /// no-talk data in the character's slot runs the no-talk objective on
    /// this selection instead of drawing a new one.
    forced: HashMap<Entity, Selection>,
}

impl NpcFixtureActivities {
    /// Keep `selection` as the data a forced no-talk objective set.
    pub fn force(&mut self, actor: Entity, selection: Selection) {
        self.forced.insert(actor, selection);
    }

    /// Forget the forced data of `actor` (its slot was replaced).
    pub fn clear_forced(&mut self, actor: Entity) {
        self.forced.remove(&actor);
    }
}

/// Why the no-talk factory produced no result on this frame. Neither kind is
/// a source state: the source factory always runs to a fixture or to null.
#[derive(Debug)]
pub enum FactoryIssue {
    /// Inputs the objective loop waits for before drawing (see
    /// [`Factory::loading`]) are not installed. Retrying the same decision on
    /// a later frame resolves it.
    Pending(String),
    /// Host data the factory reads is incomplete or inconsistent (extraction,
    /// placement or spawn work). A hold on it could last forever, so the
    /// no-talk lane yields no action for this decision; the other lanes are
    /// unaffected.
    Gap(String),
}

impl FactoryIssue {
    fn into_reason(self) -> String {
        match self {
            Self::Pending(reason) | Self::Gap(reason) => reason,
        }
    }
}

#[derive(SystemParam)]
pub struct Factory<'w, 's> {
    editor: Res<'w, crate::field_modes::EditSessionActive>,
    tables: Option<Res<'w, FixtureActivityTables>>,
    scripts: Option<Res<'w, crate::talk::TalkStore>>,
    points: Option<Res<'w, AttachPoints>>,
    inputs: Option<Res<'w, FixtureSceneSupply>>,
    sites: Res<'w, crate::fixture::residency::FixtureSites>,
    owners: Query<'w, 's, &'static crate::fixture::residency::FixtureSiteOwner>,
    areas: Res<'w, NpcFixtureAreas>,
    reservations: ResMut<'w, FixtureActivityReservations>,
    runtime: ResMut<'w, NpcFixtureActivities>,
    upper_reservations: ResMut<'w, crate::npc_objective::controller::UpperFixtureReservations>,
    fixtures: Query<
        'w,
        's,
        (
            Entity,
            &'static FixtureActivityIdentity,
            &'static FixtureScenePlacement,
            &'static GlobalTransform,
        ),
        Without<SourceInactive>,
    >,
}

impl Factory<'_, '_> {
    fn site_inputs(&self, site: &str) -> Option<&crate::fixture_scene_inputs::FixtureSceneInputs> {
        self.inputs.as_deref().and_then(FixtureSceneSupply::current)
            .filter(|inputs| inputs.site_type == site).map(|inputs| &**inputs)
            .or_else(|| self.sites.get(site).map(|context| &*context.inputs))
    }

    pub fn reserve_upper_fixture(&mut self, actor: Entity, target: FixtureTarget) -> bool {
        if self.reservations.npc_target_in_use(&target)
            || self.upper_reservations.in_use(&target, actor)
        {
            return false;
        }
        self.upper_reservations.reserve(actor, target);
        true
    }
    /// SelectTargetFixture's reservation/instance admission before its pool
    /// shuffle. Action-point playability is checked independently of a talk.
    pub fn hfsm_target_available(
        &self,
        actor: Entity,
        uid: &str,
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        _face: &ObjectiveFace,
    ) -> Result<bool, String> {
        let rows = placements.occupancy_rows();
        let instances = self
            .live_instances(&rows, placements.site_id())
            .map_err(FactoryIssue::into_reason)?;
        let Some(&(entity, identity, world, _row)) =
            instances.iter().find(|(_, id, _, _)| id.uid == uid)
        else {
            return Ok(false);
        };
        let target = FixtureTarget {
            entity,
            uid: uid.to_owned(),
        };
        if self.reservations.npc_target_in_use(&target)
            || self.upper_reservations.in_use(&target, actor)
            || other_targets
                .iter()
                .any(|(_other, target)| *target == Some(entity))
        {
            return Ok(false);
        }
        let points = self
            .points
            .as_deref()
            .ok_or("source locator arrays loading")?;
        let array = points.source_array(&identity.model_package, world)?;
        if array.is_empty() {
            return Ok(false);
        }
        Ok(array.iter().any(|(name, _)| {
            let stripped = name.replace("loc_start", "");
            let slot = stripped
                .split('_')
                .nth(1)
                .and_then(|p| p.parse::<i32>().ok())
                .unwrap_or(0);
            !self.reservations.action_slot_in_use(&target, slot)
        }))
    }
    /// The placement UID of a live fixture entity.
    pub fn uid_of(&self, fixture: Entity) -> Option<String> {
        self.fixtures
            .get(fixture)
            .ok()
            .map(|(_, identity, _, _)| identity.uid.clone())
    }

    pub fn owns_actor(&self, actor: Entity) -> bool {
        self.runtime.sessions.contains_key(&actor)
    }

    pub fn report_pending(&mut self, actor: Entity, unit: u32, reason: String) {
        if self.runtime.pending_factories.get(&actor) != Some(&reason) {
            info!("[npc-fixture] unit={unit} factory pending: {reason}");
            self.runtime.pending_factories.insert(actor, reason);
        }
    }

    /// A decision was committed: a later identical pending reason is reported again.
    pub fn clear_pending(&mut self, actor: Entity) {
        self.runtime.pending_factories.remove(&actor);
    }

    /// A [`FactoryIssue::Gap`] is unfinished host data, not a per-NPC wait:
    /// it is reported once, whichever NPC's decision meets it first.
    pub fn report_gap(&mut self, unit: u32, reason: String) {
        if !self.runtime.reported_gaps.contains(&reason) {
            warn!(
                "[npc-fixture] no-talk factory not evaluated (first met by unit={unit}): {reason}"
            );
            self.runtime.reported_gaps.insert(reason);
        }
    }

    /// Inputs this host installs while a site loads: the master tables, the
    /// locator arrays and the placement/floor snapshot of the current site
    /// generation. The source starts an NPC's AI only after its site and
    /// master data exist, so the objective loop holds in front of every draw
    /// while this is Err. Gaps inside a built snapshot are not loading; see
    /// [`Self::snapshot_gap`].
    pub fn loading(&self, epoch: u64, site_type: &str) -> Result<(), String> {
        if self.editor.is_active() {
            return Err(
                "the layout editor owns fixture mutation; no new activity is admitted".into(),
            );
        }
        if self.tables.is_none() {
            return Err("no-talk source tables are still loading".into());
        }
        if self.points.is_none() {
            return Err("source fixture locator arrays are still loading".into());
        }
        let inputs = self.site_inputs(site_type)
            .ok_or("current fixture scene inputs are not ready")?;
        if inputs.stamp.site_epoch != epoch || inputs.site_type != site_type {
            return Err("fixture scene inputs belong to another site generation".into());
        }
        Ok(())
    }

    /// Placement or floor rows the built snapshot could not resolve. They are
    /// host data gaps that persist until the extractor or placement owner
    /// closes them, and only the no-talk factory reads the tile occupancy they
    /// leave incomplete.
    fn snapshot_gap(&self, site: &str) -> Option<String> {
        let inputs = self.site_inputs(site)?;
        if let Some(gap) = inputs.gaps.first() {
            return Some(format!(
                "placement/floor snapshot has {} gap(s), first: {gap}",
                inputs.gaps.len()
            ));
        }
        (inputs.floor.coverage() != OccupancyCoverage::Complete)
            .then(|| "placement/floor snapshot occupancy is incomplete".to_owned())
    }

    /// The master tables the talk lotteries read, once installed.
    pub fn tables(&self) -> Option<&FixtureActivityTables> {
        self.tables.as_deref()
    }

    /// The placed fixtures in the fixture manager's insertion order, as the
    /// talk lotteries read them. Every placement stands on the shown site,
    /// whose site type value is `site_type`. The motion-area overlap is the
    /// no-talk factory's own test; a placement without a fixture id is never
    /// named by a talk condition and is not tested.
    pub fn talk_lottery_fixtures(
        &self,
        placements: &FixturePlacements,
        site_type: Option<i32>,
    ) -> Vec<crate::npc_talk_lottery::PlacedFixture> {
        let rows = placements.occupancy_rows();
        let floor = self.site_inputs(placements.site_type())
            .map(|inputs| &inputs.floor);
        let tables = self.tables.as_deref();
        rows.iter()
            .zip(placements.fixture_ids())
            .map(|(row, fixture_id)| crate::npc_talk_lottery::PlacedFixture {
                uid: row.uid.clone(),
                fixture_id,
                located_site_type: site_type,
                is_gate: tables
                    .and_then(|tables| tables.fixture_master(fixture_id))
                    .is_some_and(|master| master.is_gate),
                motion_overlap: if fixture_id == 0 {
                    Err("placement carries no fixture id".into())
                } else {
                    match floor {
                        Some(floor) => self.areas.motion_overlaps(row, &rows, floor),
                        None => Err("current fixture scene inputs are not ready".into()),
                    }
                },
            })
            .collect()
    }

    /// Source action-row permutation is independent of asset readiness.
    /// `draws` receives the factory's draws: the sort keys of the row order
    /// and, on a hit, the timeline pick.
    /// Empty eligible source pool is Ok(None); missing host data is an issue,
    /// not an invitation to select a different ready animation or a General
    /// story. `unmovable` is this NPC's UnmovableFixtureList (placement UIDs).
    pub fn select_none_talk(
        &self,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        unmovable: &[String],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
        draws: &mut NoneTalkDraws,
    ) -> Result<Option<Selection>, FactoryIssue> {
        self.select_internal(
            actor,
            unit,
            site_type,
            epoch,
            from,
            placements,
            other_targets,
            unmovable,
            face,
            rng,
            None,
            None,
            draws,
        )
    }

    /// Content-library staging of one exact action. It is not an AI decision
    /// and does not consult an NPC's UnmovableFixtureList.
    pub fn select_exact(
        &self,
        key: ActivityKey,
        target: &FixtureTarget,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
    ) -> Result<Option<Selection>, String> {
        let tables = self.tables.as_deref().ok_or("角色互动主表尚未载入")?;
        let scripts = self.scripts.as_deref().ok_or("家具前置演出主表尚未载入")?;
        let spec = tables
            .resolve_activity(key, scripts)
            .ok_or("所选互动的原始演员、家具或动作关系不可用")?;
        if spec.unit != unit {
            return Err("所选互动的演员身份已经变化".into());
        }
        self.select_internal(
            actor,
            unit,
            site_type,
            epoch,
            from,
            placements,
            other_targets,
            &[],
            face,
            rng,
            Some((&spec, target)),
            None,
            &mut NoneTalkDraws::default(),
        )
        .map_err(FactoryIssue::into_reason)
    }

    /// The no-talk actions a character's random fixture action chooses
    /// from: the NoTalk rows in master order whose fixture id has a placed
    /// fixture (IsTargetFixture), whose unit is `unit`, and for which
    /// GetTargetNoTalkFixture finds a fixture (the same actionable and
    /// playable tests as the lottery, `unmovable` being this character's
    /// UnmovableFixtureList). Returns the row ids.
    #[allow(clippy::too_many_arguments)]
    pub fn no_talk_action_rows(
        &self,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        unmovable: &[String],
        face: &ObjectiveFace,
    ) -> Result<Vec<i32>, FactoryIssue> {
        let tables = self.tables.as_deref().ok_or_else(|| {
            FactoryIssue::Pending("no-talk source tables are still loading".into())
        })?;
        let mut rows = Vec::new();
        for row in tables.no_talk_rows().iter().filter(|row| row.unit == unit) {
            // The probe draws the timeline pick on a scratch generator: only
            // whether a fixture is found matters here.
            let mut scratch = MemberRng::seeded(unit);
            if self
                .select_internal(
                    actor,
                    unit,
                    site_type,
                    epoch,
                    from,
                    placements,
                    other_targets,
                    unmovable,
                    face,
                    &mut scratch,
                    None,
                    Some(row.id),
                    &mut NoneTalkDraws::default(),
                )?
                .is_some()
            {
                rows.push(row.id);
            }
        }
        Ok(rows)
    }

    /// `CreateNoneTalkData(character, action)`: GetTargetNoTalkFixture for
    /// the one NoTalk row, then, on a hit, one engine pick of the timeline in
    /// its group on `rng`.
    #[allow(clippy::too_many_arguments)]
    pub fn select_no_talk_row(
        &self,
        row: i32,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        unmovable: &[String],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
        draws: &mut NoneTalkDraws,
    ) -> Result<Option<Selection>, FactoryIssue> {
        self.select_internal(
            actor,
            unit,
            site_type,
            epoch,
            from,
            placements,
            other_targets,
            unmovable,
            face,
            rng,
            None,
            Some(row),
            draws,
        )
    }

    /// The forced no-talk data of `actor`, taken (see
    /// [`NpcFixtureActivities::force`]).
    pub fn take_forced(&mut self, actor: Entity) -> Option<Selection> {
        self.runtime.forced.remove(&actor)
    }

    /// Put forced data back (see [`Self::take_forced`]).
    pub fn keep_forced(&mut self, actor: Entity, selection: Selection) {
        self.runtime.forced.insert(actor, selection);
    }

    /// GetAllFixture is insertion ordered. The offline placement owner is
    /// that order in this host; query iteration and package-first lookup are
    /// not. Resolve every row against exactly one live Entity/UID: live
    /// instances are grouped by UID once; a row keeps the instances in query
    /// order whose identity and scene placement both carry its UID.
    fn live_instances<'a>(
        &'a self,
        rows: &'a [OccupancyRow],
        site_id: u32,
    ) -> Result<Vec<LiveInstance<'a>>, FactoryIssue> {
        let mut live: HashMap<&str, Vec<_>> = HashMap::with_capacity(rows.len());
        for fixture in self.fixtures.iter() {
            let (_, identity, placed, _) = fixture;
            if identity.uid == placed.0.uid
                && self.owners.get(fixture.0).is_ok_and(|owner| owner.site_id == site_id) {
                live.entry(identity.uid.as_str()).or_default().push(fixture);
            }
        }
        let mut instances = Vec::with_capacity(rows.len());
        for row in rows {
            let matches = live.get(row.uid.as_str()).map_or(&[][..], Vec::as_slice);
            let [(entity, identity, placed, world)] = matches else {
                return Err(FactoryIssue::Gap(format!(
                    "placement {} has no unique live instance",
                    row.uid
                )));
            };
            if &placed.0 != row || identity.model_package != row.package {
                return Err(FactoryIssue::Gap(format!(
                    "placement {} no longer agrees with its live owner",
                    row.uid
                )));
            }
            instances.push((*entity, *identity, *world, row));
        }
        Ok(instances)
    }

    /// `CanActionableFixture(character, fixture)` for one live placement.
    /// `IsAvailableFixtureCaseCharacter`: no NPC holds the fixture through its
    /// AI talk data (the fixture's NPC-using list is empty), and no other NPC's
    /// talk data targets it. `IsAvailableFixtureCasePutStatus`: the fixture is
    /// not in this NPC's UnmovableFixtureList, lies within 1000 m of the NPC,
    /// has a motion area that overlaps nothing, and stands on grid level 0 or
    /// on a fixture that is not a block. Every placement stands on the shown
    /// site, where every NPC of this host also is, so the site-type and
    /// site-existence tests hold.
    #[allow(clippy::too_many_arguments)]
    fn actionable(
        &self,
        actor: Entity,
        target: &FixtureTarget,
        fixture_world: &GlobalTransform,
        row: &OccupancyRow,
        rows: &[OccupancyRow],
        instances: &[LiveInstance<'_>],
        floor: &FixtureFloorTiles,
        tables: &FixtureActivityTables,
        unmovable: &[String],
        other_targets: &[(Entity, Option<Entity>)],
        from: [f32; 3],
    ) -> Result<bool, FactoryIssue> {
        // A fixture this NPC failed to reach at an earlier departure stays
        // excluded until the list is cleared.
        if unmovable.iter().any(|uid| *uid == target.uid) {
            return Ok(false);
        }
        if self.reservations.npc_target_in_use(target)
            || other_targets
                .iter()
                .any(|(other, held)| *other != actor && *held == Some(target.entity))
        {
            return Ok(false);
        }
        if fixture_world.translation().distance(Vec3::from(from)) > 1000.0 {
            return Ok(false);
        }
        if self
            .areas
            .motion_overlaps(row, rows, floor)
            .map_err(FactoryIssue::Gap)?
        {
            return Ok(false);
        }
        stands_on_usable_tile(row, instances, floor, tables)
    }

    /// `FixtureController.IsPlayableActionPoints(point)` for one live
    /// placement. `None` when the point is not playable: the fixture's
    /// locator array has no entry named for it (the source logs an error and
    /// answers false; a master row can name a point the fixture does not
    /// carry), its action slot is in use, the StartLoc sampled at site height
    /// (SamplePosition 0.3) has no hit within half a tile of the query point,
    /// the StartLoc or EndLoc cell is the null tile, or the StartLoc cell lies
    /// outside the fixture on a tile that is not empty. No path query:
    /// reachability is tested once, for the chosen target only, at departure.
    #[allow(clippy::too_many_arguments)]
    fn playable_point(
        &self,
        target: &FixtureTarget,
        identity: &FixtureActivityIdentity,
        fixture_world: &GlobalTransform,
        row: &OccupancyRow,
        point: i32,
        points: &AttachPoints,
        floor: &FixtureFloorTiles,
        face: &ObjectiveFace,
    ) -> Result<Option<PlayablePoint>, FactoryIssue> {
        use FactoryIssue::Gap;
        if points
            .player_action_points(&identity.model_package)
            .is_some_and(|carried| !carried.contains(&point))
        {
            return Ok(None);
        }
        let locate_index = points
            .instance_index(&identity.model_package, point)
            .ok_or_else(|| {
                Gap(format!(
                    "{} locator {point} lacks a unique source array index",
                    identity.uid
                ))
            })?;
        let slot_id = points
            .instance_slot(&identity.model_package, point)
            .ok_or_else(|| {
                Gap(format!(
                    "{} locator {point} lacks a source slot",
                    identity.uid
                ))
            })?;
        if self.reservations.action_slot_in_use(target, slot_id) {
            return Ok(None);
        }
        let poses = points
            .instance_poses(&identity.model_package, point, fixture_world)
            .ok_or_else(|| Gap("source selected locator has no instance pose".into()))?;
        let end = poses
            .end
            .ok_or_else(|| Gap("source selected locator has no EndLoc".into()))?;
        let position = [
            poses.start.position[0],
            floor.site_origin.y,
            poses.start.position[2],
        ];
        let Some(hit) = face.sample(position, 0.3) else {
            return Ok(None);
        };
        if Vec3::from(hit).distance(Vec3::from(position)) > HALF_TILE_SIZE {
            return Ok(None);
        }
        // StartLoc and EndLoc tiles use their own heights: a stacked
        // fixture's locators sit above grid level 0. A cell outside the
        // authored floor is the source's null tile.
        let start_grid = source_grid(floor, poses.start.position);
        if !floor.contains_grid(start_grid)
            || !floor.contains_grid(source_grid(floor, end.position))
        {
            return Ok(None);
        }
        let inside = row.min.x <= start_grid.x
            && start_grid.x <= row.max.x
            && row.min.z <= start_grid.z
            && start_grid.z <= row.max.z;
        if !inside && !matches!(floor.tile_at_grid(start_grid), FixtureTileKnowledge::Empty) {
            return Ok(None);
        }
        Ok(Some(PlayablePoint {
            locate_index,
            slot_id,
            position,
            rotation: poses.start.rotation,
            start_loc: poses.start.position,
        }))
    }

    /// The admissibility pair of the playable-fixture lottery gate for one
    /// placed fixture of a fixture talk, evaluated for the deciding NPC.
    /// `CanUseFixtureCaseActionPoints(talk, fixture)` first: a talk without a
    /// pre-action is not usable; a pre-action without a timeline group (0) is
    /// usable; otherwise the first timeline of the group whose action-point
    /// row exists decides, and every non-zero action point of that row (in
    /// member-slot order) must be playable on the fixture; a group with no
    /// such timeline is not usable. Then `CanActionableFixture(character,
    /// fixture)`. `Err` names host data this host cannot evaluate.
    #[allow(clippy::too_many_arguments)]
    pub fn talk_fixture_admissible(
        &self,
        actor: Entity,
        from: [f32; 3],
        talk_id: i32,
        uid: &str,
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        unmovable: &[String],
        face: &ObjectiveFace,
    ) -> Result<bool, String> {
        if let Some(gap) = self.snapshot_gap(placements.site_type()) {
            return Err(gap);
        }
        let tables = self
            .tables
            .as_deref()
            .ok_or("talk source tables are still loading")?;
        let points = self
            .points
            .as_deref()
            .ok_or("source fixture locator arrays are still loading")?;
        let inputs = self.site_inputs(placements.site_type())
            .ok_or("current fixture scene inputs are not ready")?;
        let rows = placements.occupancy_rows();
        let instances = self
            .live_instances(&rows, placements.site_id())
            .map_err(FactoryIssue::into_reason)?;
        let &(entity, identity, fixture_world, row) = instances
            .iter()
            .find(|(_, identity, _, _)| identity.uid == uid)
            .ok_or_else(|| format!("placement {uid} has no resolved live instance"))?;
        let target = FixtureTarget {
            entity,
            uid: identity.uid.clone(),
        };
        let usable = match tables.pre_action_of(talk_id) {
            None => false,
            Some(pre_action) => match pre_action.timeline_group_id.unwrap_or(0) {
                0 => true,
                group => {
                    let mut decided = None;
                    for timeline in tables.timeline_rows(group) {
                        let Some(action_points) =
                            tables.action_points_of(timeline.action_point_definition)
                        else {
                            continue;
                        };
                        let mut every = true;
                        for point in action_points {
                            if self
                                .playable_point(
                                    &target,
                                    identity,
                                    fixture_world,
                                    row,
                                    point,
                                    points,
                                    &inputs.floor,
                                    face,
                                )
                                .map_err(FactoryIssue::into_reason)?
                                .is_none()
                            {
                                every = false;
                                break;
                            }
                        }
                        decided = Some(every);
                        break;
                    }
                    decided.unwrap_or(false)
                }
            },
        };
        if !usable {
            return Ok(false);
        }
        self.actionable(
            actor,
            &target,
            fixture_world,
            row,
            &rows,
            &instances,
            &inputs.floor,
            tables,
            unmovable,
            other_targets,
            from,
        )
        .map_err(FactoryIssue::into_reason)
    }

    /// What the fixture-talk factories read of one placed fixture: its live
    /// entity, its action-point array (StartLoc names and poses, array
    /// order), its footprint (BoundingBox, grid x/z, inclusive), the floor
    /// grid's bound (GetGridData(2), inclusive) and the site origin. `Err`
    /// names host data it cannot resolve.
    pub fn talk_fixture_geometry(
        &self,
        uid: &str,
        placements: &FixturePlacements,
    ) -> Result<TalkFixtureGeometry, String> {
        if let Some(gap) = self.snapshot_gap(placements.site_type()) {
            return Err(gap);
        }
        let points = self
            .points
            .as_deref()
            .ok_or("source fixture locator arrays are still loading")?;
        let inputs = self.site_inputs(placements.site_type())
            .ok_or("current fixture scene inputs are not ready")?;
        let rows = placements.occupancy_rows();
        let instances = self
            .live_instances(&rows, placements.site_id())
            .map_err(FactoryIssue::into_reason)?;
        let &(entity, identity, world, row) = instances
            .iter()
            .find(|(_, identity, _, _)| identity.uid == uid)
            .ok_or_else(|| format!("placement {uid} has no resolved live instance"))?;
        let locators = points.source_array(&identity.model_package, world)?;
        let layout = &inputs.floor.layout;
        // GridData.SetupGridData: [-ceil(w/2), ceil(w/2)) per axis.
        let half_width = (layout.width + 1) / 2;
        let half_depth = (layout.depth + 1) / 2;
        Ok(TalkFixtureGeometry {
            entity,
            locators,
            footprint: (
                (row.min.x as i32, row.min.z as i32),
                (row.max.x as i32, row.max.z as i32),
            ),
            floor_bounds: ((-half_width, -half_depth), (half_width - 1, half_depth - 1)),
            site_origin: inputs.floor.site_origin.to_array(),
        })
    }

    fn select_internal(
        &self,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        unmovable: &[String],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
        exact: Option<(&ActivitySpec, &FixtureTarget)>,
        only_row: Option<i32>,
        draws: &mut NoneTalkDraws,
    ) -> Result<Option<Selection>, FactoryIssue> {
        use FactoryIssue::{Gap, Pending};
        self.loading(epoch, site_type).map_err(Pending)?;
        if let Some(gap) = self.snapshot_gap(placements.site_type()) {
            return Err(Gap(gap));
        }
        let tables = self
            .tables
            .as_deref()
            .ok_or_else(|| Pending("no-talk source tables are still loading".into()))?;
        let points = self
            .points
            .as_deref()
            .ok_or_else(|| Pending("source fixture locator arrays are still loading".into()))?;
        let inputs = self.site_inputs(placements.site_type())
            .ok_or_else(|| Pending("current fixture scene inputs are not ready".into()))?;
        let rows = placements.occupancy_rows();
        let instances = self.live_instances(&rows, placements.site_id())?;
        // A projection, not a fabricated row in the NoTalk source table.
        // Exact pre-actions retain their own origin and authored tweet.
        struct Candidate {
            origin: ActivityOrigin,
            unit: u32,
            fixture_id: i32,
            group_id: i32,
            point: Option<i32>,
            timeline_id: Option<i32>,
            tweet: Option<moly_law::talk::TweetRef>,
        }
        let mut order: Vec<(u64, Candidate)> = if let Some((spec, _)) = exact {
            vec![(
                0,
                Candidate {
                    origin: spec.key.origin,
                    unit: spec.unit,
                    fixture_id: spec.fixture_id,
                    group_id: spec.timeline.group_id,
                    point: Some(spec.point),
                    timeline_id: Some(spec.timeline.id),
                    tweet: spec.tweet.clone(),
                },
            )]
        } else if let Some(id) = only_row {
            // One NoTalk row, as GetTargetNoTalkFixture(action) and
            // CreateNoneTalkData(character, action) read it: no order draw.
            tables
                .no_talk_rows()
                .iter()
                .filter(|row| row.id == id)
                .map(|row| {
                    (
                        0,
                        Candidate {
                            origin: ActivityOrigin::NoTalk(row.id),
                            unit: row.unit,
                            fixture_id: row.fixture_id,
                            group_id: row.timeline_group_id,
                            point: None,
                            timeline_id: None,
                            tweet: None,
                        },
                    )
                })
                .collect()
        } else {
            // Preserve the natural source permutation and draw count.
            tables
                .no_talk_rows()
                .iter()
                .map(|row| {
                    (
                        rng.next(),
                        Candidate {
                            origin: ActivityOrigin::NoTalk(row.id),
                            unit: row.unit,
                            fixture_id: row.fixture_id,
                            group_id: row.timeline_group_id,
                            point: None,
                            timeline_id: None,
                            tweet: None,
                        },
                    )
                })
                .collect()
        };
        if exact.is_none() && only_row.is_none() {
            draws.keys = order.len();
        }
        order.sort_by_key(|(key, _)| *key);
        for (_, action) in order {
            if action.unit != unit {
                continue;
            }
            // The source never resolves this action's group if no placed
            // fixture of its master can be considered at all.
            if !instances
                .iter()
                .any(|(_, identity, _, _)| identity.master_id == action.fixture_id)
            {
                continue;
            }
            let group: Vec<_> = tables.timeline_rows(action.group_id).collect();
            // CanUseFixtureCaseActionPointsCaseTimelineGroupId is false for
            // every fixture when the group has no timeline: the row yields no
            // fixture and the walk moves on to the next row.
            let Some(first) = group.first() else {
                continue;
            };
            // Source factory computes locators from FirstOrDefault(group),
            // then independently RandomPick(group) for the actual director.
            let point = match action.point {
                Some(point) => point,
                None => tables.no_talk_point(unit, first).map_err(|error| {
                    Gap(format!("action {:?} locator: {error:?}", action.origin))
                })?,
            };
            for (entity, identity, fixture_world, row) in &instances {
                if identity.master_id != action.fixture_id {
                    continue;
                }
                let target = FixtureTarget {
                    entity: *entity,
                    uid: identity.uid.clone(),
                };
                if exact.is_some_and(|(_, selected)| selected != &target) {
                    continue;
                }
                // Every no-talk fixture master is floor-only; this host keeps
                // tile occupancy for the floor layout alone.
                if row.layout != layout_type::FLOOR {
                    return Err(Gap(format!(
                        "{} is a non-floor placement of a floor-only no-talk fixture",
                        identity.uid
                    )));
                }
                if !self.actionable(
                    actor,
                    &target,
                    fixture_world,
                    row,
                    &rows,
                    &instances,
                    &inputs.floor,
                    tables,
                    unmovable,
                    other_targets,
                    from,
                )? {
                    continue;
                }
                let master = tables
                    .fixture_master(identity.master_id)
                    .ok_or_else(|| Gap(format!("missing fixture master {}", identity.master_id)))?;
                // A gate shows its gate's or its skin's model
                // (`MysekaiGateModel.AssetBundleName`), not its master's.
                if !master.is_gate
                    && identity.model_package != format!("mysekai__fixture__{}", master.model_name)
                {
                    return Err(Gap("source action's instance model/master disagree".into()));
                }
                let Some(playable) = self.playable_point(
                    &target,
                    identity,
                    fixture_world,
                    row,
                    point,
                    points,
                    &inputs.floor,
                    face,
                )?
                else {
                    continue;
                };
                let timeline = if let Some(id) = action.timeline_id {
                    (**group
                        .iter()
                        .find(|timeline| timeline.id == id)
                        .ok_or_else(|| Gap("所选动作已不在原始 Timeline 组中".into()))?)
                    .clone()
                } else {
                    let index = rng.index(group.len());
                    draws.timeline_pick = Some((index, group.len()));
                    (*group[index]).clone()
                };
                return Ok(Some(Selection {
                    kind: TalkType::NoneTalk,
                    actor,
                    unit,
                    target,
                    identity: (*identity).clone(),
                    site_epoch: epoch,
                    source: ActivityKey {
                        origin: action.origin,
                        timeline_id: timeline.id,
                    },
                    tweet: action.tweet.clone(),
                    timeline,
                    point,
                    locate_index: playable.locate_index,
                    slot_id: playable.slot_id,
                    put_type: master.put_type,
                    position: playable.position,
                    rotation: playable.rotation,
                    start_loc: playable.start_loc,
                }));
            }
        }
        Ok(None)
    }

    /// The timeline action of a single-character fixture talk whose
    /// pre-action carries a timeline: the main's locate row names the
    /// locator, the talk's picked timeline plays on it, and the pre-action's
    /// tweet opens the window before it. The claimed slot is the locator
    /// name's slot (TryParseActionPointNameToSlotId), not the locate row's.
    #[allow(clippy::too_many_arguments)]
    pub fn talk_timeline_selection(
        &self,
        actor: Entity,
        unit: u32,
        epoch: u64,
        placements: &FixturePlacements,
        data: &crate::npc_talk_lottery::FixtureTalkData,
        pre_action_id: i32,
        tweet: moly_law::talk::TweetRef,
    ) -> Result<Selection, String> {
        let tables = self
            .tables
            .as_deref()
            .ok_or("talk source tables are still loading")?;
        let points = self
            .points
            .as_deref()
            .ok_or("source fixture locator arrays are still loading")?;
        let rows = placements.occupancy_rows();
        let instances = self
            .live_instances(&rows, placements.site_id())
            .map_err(FactoryIssue::into_reason)?;
        let &(entity, identity, world, _) = instances
            .iter()
            .find(|(_, identity, _, _)| identity.uid == data.fixture)
            .ok_or_else(|| format!("placement {} has no resolved live instance", data.fixture))?;
        let timeline_id = data
            .timeline
            .ok_or("the fixture talk carries no timeline")?;
        let timeline = tables
            .timeline(timeline_id)
            .ok_or_else(|| format!("timeline {timeline_id} has no master row"))?
            .clone();
        let locate = data.locate.as_deref().ok_or("fixture timeline locate list is null")?;
        let row = locate.iter().find(|row| row.unit == unit as i32).copied();
        let locate_index = row.map_or(0, |row| row.index);
        let array = points.source_array(&identity.model_package, world)?;
        let (name, pair) = array.get(locate_index).ok_or_else(|| {
            format!(
                "locator index {} is outside {}'s array",
                locate_index, identity.uid
            )
        })?;
        // The action-point value the locator was found by: the digits of the
        // name's second '_' field.
        let point = name
            .split('_')
            .nth(1)
            .map(|field| {
                field
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
            })
            .and_then(|digits| digits.parse::<i32>().ok())
            .ok_or_else(|| format!("locator name {name} carries no action-point value"))?;
        if points.instance_index(&identity.model_package, point) != Some(locate_index) {
            return Err(format!(
                "{} locator {point} does not resolve to array index {}",
                identity.uid, locate_index
            ));
        }
        let slot_id = points
            .instance_slot(&identity.model_package, point)
            .ok_or_else(|| format!("{} locator {point} lacks a source slot", identity.uid))?;
        let master = tables
            .fixture_master(identity.master_id)
            .ok_or_else(|| format!("missing fixture master {}", identity.master_id))?;
        Ok(Selection {
            kind: TalkType::SingleCharacterFixture,
            actor,
            unit,
            target: FixtureTarget {
                entity,
                uid: identity.uid.clone(),
            },
            identity: (*identity).clone(),
            site_epoch: epoch,
            source: ActivityKey {
                origin: ActivityOrigin::PreAction(pre_action_id),
                timeline_id,
            },
            tweet: Some(tweet),
            timeline,
            point,
            locate_index,
            slot_id,
            put_type: master.put_type,
            position: data.target_position,
            rotation: pair.start.rotation,
            start_loc: pair.start.position,
        })
    }

    pub fn begin(&mut self, selection: Selection, enable_talk: bool) -> bool {
        if self.owns_actor(selection.actor) {
            return false;
        }
        self.runtime.next_generation = self
            .runtime
            .next_generation
            .checked_add(1)
            .expect("NPC fixture activity generation exhausted");
        let owner = FixtureActivityOwner {
            actor: selection.actor,
            generation: self.runtime.next_generation,
        };
        if !self
            .reservations
            .reserve_npc_target(&selection.target, owner)
        {
            return false;
        }
        // A fixture talk claims its locator's slot when its move starts
        // (SetUsingFixtureActionPoint); a no-talk action claims it at playback.
        if selection.kind != TalkType::NoneTalk
            && !self
                .reservations
                .reserve_action_slot(&selection.target, selection.slot_id, owner)
        {
            self.reservations.release_owner(owner);
            return false;
        }
        self.runtime.pending_factories.remove(&selection.actor);
        info!(
            "[npc-fixture] unit={} selected action={} fixture={:?}/{} locator={}/slot={} timeline={} (WalkField approach remains approximate)",
            selection.unit,
            selection.source.source_id(),
            selection.target.entity,
            selection.target.uid,
            selection.locate_index,
            selection.slot_id,
            selection.timeline.id
        );
        self.runtime.sessions.insert(
            selection.actor,
            Session {
                owner,
                selection,
                phase: Phase::Approaching,
                request: None,
                token: None,
                animator: None,
                previous_enable_talk: enable_talk,
                last_pending: None,
                preview_ticket: None,
                waiting_seconds: 0.,
                tweet_issued: false,
                talk_clip: false,
            },
        );
        true
    }
}

/// Runs after the ordinary movement system and before the shared timeline.
/// An Arrived result transfers ownership; it is not objective completion.
pub fn advance(world: &mut World) {
    if world
        .get_resource::<crate::field_modes::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return;
    }
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    runtime
        .pending_factories
        .retain(|actor, _| world.entities().contains(*actor));
    let mut actors: Vec<_> = runtime
        .sessions
        .iter()
        .map(|(actor, session)| (*actor, session.owner.generation))
        .collect();
    actors.sort_by_key(|(_, generation)| *generation);
    let mut provider = world.remove_resource::<FixtureActivityProvider>();
    for (actor, _) in actors {
        let Some(mut session) = runtime.sessions.remove(&actor) else {
            continue;
        };
        let result = tick(world, provider.as_mut(), &mut session);
        match result {
            Ok(true) => dispose(world, session, false, true),
            Ok(false) => {
                preview::record_progress(world, &mut session);
                runtime.sessions.insert(actor, session);
            }
            Err(reason) => {
                warn!(
                    "[npc-fixture] unit={} action={} cancelled: {reason}",
                    session.selection.unit,
                    session.selection.source.source_id()
                );
                preview::record_failure(world, &session, reason);
                dispose(world, session, false, false);
            }
        }
    }
    if let Some(provider) = provider {
        world.insert_resource(provider);
    }
    world.insert_resource(runtime);
}

fn tick(
    world: &mut World,
    provider: Option<&mut FixtureActivityProvider>,
    session: &mut Session,
) -> Result<bool, String> {
    let actor = session.owner.actor;
    // The source's move has no time limit: it ends by arrival, by the stuck
    // rule, by a failed route or by a cancel (the route owner reports each).
    if matches!(session.phase, Phase::Approaching | Phase::Preparing) {
        session.waiting_seconds += world.resource::<Time>().delta_secs();
    }
    let selection = &session.selection;
    // Camera entry generations can change while this exact actor/fixture
    // pair stays alive. The source lifetime is their owning site and identity.
    if !crate::fixture::residency::actor_owns_fixture(world, actor, selection.target.entity) {
        return Err("actor and selected fixture no longer share their site".into());
    }
    if world
        .get::<CharacterUnitId>(actor)
        .is_none_or(|unit| unit.0 != selection.unit)
        || world.get::<FixtureActivityIdentity>(selection.target.entity)
            != Some(&selection.identity)
        || world
            .get::<SourceInactive>(selection.target.entity)
            .is_some()
    {
        return Err("selected actor or fixture Entity/UID no longer exists".into());
    }
    if world
        .get::<TalkSlot>(actor)
        .and_then(|slot| slot.current.as_ref())
        .is_none_or(|data| {
            data.kind != selection.kind || data.target_fixture != Some(selection.target.entity)
        })
    {
        return Err("another objective replaced the selected action".into());
    }
    if world
        .get_resource::<FixtureActivityReservations>()
        .and_then(|reservations| reservations.npc_target_owner(&selection.target))
        != Some(session.owner)
    {
        return Err("future fixture reservation was replaced".into());
    }
    // The playing-fixture talk (the player's talk on this NPC in the fixture
    // action idle state) plays with this data's target fixture and leaves
    // the timeline running, as a talk joined on this fixture does.
    let joined_talk = matches!(session.phase, Phase::Playing | Phase::TalkHeld)
        && (world
            .get_resource::<crate::talk::ActiveTalk>()
            .is_some_and(|talk| {
                talk.participants()
                    .iter()
                    .any(|(_, entity)| *entity == actor)
                    && talk
                        .fixture_instances()
                        .iter()
                        .any(|(_, entity)| *entity == selection.target.entity)
            })
            || world
                .get_resource::<crate::player_talk::PlayerTalkSession>()
                .is_some_and(|talk| talk.keeps_fixture_timeline(actor)));

    // In a fixture talk's window the player's talk on that same data is the
    // window's purpose: its waits hold for it.
    let in_window = matches!(
        session.phase,
        Phase::TalkWindow(_) | Phase::HfsmLooking | Phase::HfsmNoneLooking | Phase::HfsmApproach(_)
    );
    // No path of the player's talk touches the objective: it acts only
    // through the NPC's state (the talk state stops the agent, the move's
    // wait pauses in Talk, a local fit runs to its end, and the objective's
    // next state change ends the talk state). Its hold cancels nothing here.
    let player_talk = world
        .get_resource::<crate::player_talk::PlayerTalkSession>()
        .is_some_and(|talk| talk.npc_entity() == actor);
    if world.get::<TalkHold>(actor).is_some() && !joined_talk && !in_window && !player_talk {
        // Only the admitted cast on this same placed fixture may retain the
        // existing Director. An unrelated conversation still cancels ownership.
        return Err("another conversation owns the actor".into());
    }
    let dt = world.get_resource::<Time>().map_or(0.0, Time::delta_secs);
    if !dt.is_finite() || dt < 0.0 {
        return Err("invalid game delta".into());
    }

    if matches!(session.phase, Phase::Approaching) {
        match world
            .get::<RouteStops>(actor)
            .and_then(|route| route.outcome)
        {
            None => return Ok(false),
            Some(RouteOutcome::Stopped) => {
                return Err("ordinary approach stopped without arrival".into());
            }
            Some(RouteOutcome::Arrived) => {
                world
                    .entity_mut(actor)
                    .insert(NpcFixtureMotionOwner(session.owner));
                world
                    .get_mut::<RouteStops>(actor)
                    .ok_or("route owner was removed")?
                    .hold_for_fixture();
                world
                    .get_mut::<PathSlot>(actor)
                    .ok_or("path slot was removed")?
                    .0 = NpcPathWalkSlot::from_corners(Vec::new());
                world
                    .entity_mut(actor)
                    .insert(MotionPhase::Dwelling { remaining: None });
                if selection.kind == TalkType::NoneTalk {
                    change_action(world, actor, NpcAction::FixtureAction, false)?;
                    session.phase = if crate::npc_objective::events::enrolled(world, actor) {
                        start_hfsm_approach(world, actor, selection)?
                    } else {
                        Phase::Preparing
                    };
                    info!(
                        "[npc-fixture] unit={} arrived at action {}; preparing its exact source timeline",
                        selection.unit,
                        selection.source.source_id()
                    );
                } else {
                    // OnArrive: no locator rotation (the data's rotation is
                    // the zero quaternion), then the look at the fixture.
                    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
                    info!(
                        "[npc unit={}] frame={frame} fixture talk {} arrived; OnArrive: call=DoLookAt(fixture)",
                        selection.unit,
                        selection.source.source_id()
                    );
                    if crate::npc_objective::events::enrolled(world, actor) {
                        let fixture = fixture_look_target(world, actor, selection).ok();
                        if let (Some(target), Some(transform)) =
                            (fixture, world.get::<Transform>(actor))
                        {
                            let forward = (transform.rotation * Vec3::Z).to_array();
                            let duration = rotate_time(angle_between(
                                forward,
                                (target - transform.translation).to_array(),
                            ));
                            world
                                .entity_mut(actor)
                                .insert(crate::npc_look_at::NpcLookAt::new(
                                    target,
                                    forward,
                                    duration,
                                    moly_law::ui::dotween::Ease::OutQuart,
                                    frame,
                                ));
                        }
                        session.phase = Phase::HfsmLooking;
                        return Ok(false);
                    }
                    session.phase = match try_play_tweet(world, actor, &session.selection, frame)? {
                        Some(window) => Phase::TalkWindow(window),
                        None => start_talk_timeline(world, actor, &session.selection, frame)?,
                    };
                }
            }
        }
    }
    if matches!(session.phase, Phase::HfsmLooking) {
        if world.get::<crate::npc_look_at::NpcLookAt>(actor).is_some() {
            return Ok(false);
        }
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        session.phase = match try_play_tweet(world, actor, &session.selection, frame)? {
            Some(_) => Phase::TalkWindow(TalkWindow::HfsmTweet { elapsed: 0.0 }),
            None => start_talk_timeline(world, actor, &session.selection, frame)?,
        };
        return Ok(false);
    }
    if matches!(session.phase, Phase::HfsmNoneLooking) {
        if world.get::<crate::npc_look_at::NpcLookAt>(actor).is_some() {
            return Ok(false);
        }
        change_action(world, actor, NpcAction::FixtureAction, false)?;
        session.phase = Phase::Preparing;
        return Ok(false);
    }
    if let Phase::HfsmApproach(approach) = &mut session.phase {
        if approach.update(world, actor, dt)? {
            world
                .entity_mut(actor)
                .insert(NpcFixtureMotionOwner(session.owner));
            session.phase = finish_hfsm_approach(world, actor, &session.selection)?;
        }
        return Ok(false);
    }
    if let Phase::TalkWindow(window) = session.phase {
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        let (tweet_done, talking) = world
            .get::<NpcActions>(actor)
            .map(|actions| {
                (
                    actions.tweet_state == crate::npc_state::TweetState::Done,
                    actions.current == NpcAction::Talk,
                )
            })
            .ok_or("NPC action owner disappeared")?;
        match window {
            TalkWindow::HfsmTweet { elapsed } => {
                if talking {
                    return Ok(false);
                }
                let elapsed = elapsed + dt;
                if elapsed >= 5.0 {
                    world.write_message(crate::npc_state::TweetHudEvent::Retire { npc: actor });
                    session.phase = Phase::TalkWindow(TalkWindow::NotTalking { since: frame });
                } else {
                    session.phase = Phase::TalkWindow(TalkWindow::HfsmTweet { elapsed });
                }
                return Ok(false);
            }
            TalkWindow::TweetDone { since } => {
                if frame != since && tweet_done {
                    session.phase = Phase::TalkWindow(TalkWindow::NotTalking { since: frame });
                }
                return Ok(false);
            }
            TalkWindow::NotTalking { since } => {
                if frame == since || talking || world.get::<TalkHold>(actor).is_some() {
                    return Ok(false);
                }
                info!(
                    "[npc unit={}] frame={frame} tweet wait ended; call=TryRotateBeforeRotation",
                    selection.unit
                );
                session.phase = start_talk_timeline(world, actor, &session.selection, frame)?;
                return Ok(false);
            }
        }
    }
    if world
        .get::<NpcFixtureMotionOwner>(actor)
        .is_none_or(|held| held.0 != session.owner)
    {
        return Err("fixture movement lease was replaced".into());
    }
    if matches!(session.phase, Phase::TalkHeld) {
        // The completed Director was already stopped. Never replay idle while
        // this admitted dialogue drives its own clips.
        if joined_talk {
            return Ok(false);
        }
        change_action(world, actor, NpcAction::FixtureActionIdle, false)?;
        let (_, end) = live_poses(world, &session.selection)?;
        // LookAt or dialogue motion may have changed the logical root after
        // handoff. Start reattachment from that live pose, never teleport it
        // back to EndLoc a second time.
        let start = *world
            .get::<Transform>(actor)
            .ok_or("NPC pose disappeared")?;
        session.phase = Phase::Exiting(prepare_exit(world, actor, start, end)?);
    }
    if matches!(session.phase, Phase::Preparing) {
        let Some(provider) = provider else {
            return pending(session, "shared activity provider is not initialized");
        };
        if let Err(issue) = prepare(world, provider, session) {
            let reason = format!("{}: {}", issue.stage, issue.reason);
            if issue.retryable {
                return pending(session, &reason);
            }
            return Err(reason);
        }
        let request = session
            .request
            .take()
            .ok_or("prepared timeline request is missing")?;
        let target = session.selection.target.clone();
        let slot = session.selection.slot_id;
        let reserved = world
            .resource_mut::<FixtureActivityReservations>()
            .reserve_action_slot(&target, slot, session.owner);
        if !reserved {
            return Err("selected action slot became occupied before playback".into());
        }
        let poses = live_poses(world, &session.selection)?;
        crate::npc::navigation::set_enabled(world, actor, false);
        set_pose(world, actor, poses.0)?;
        world
            .entity_mut(actor)
            .insert(NpcFixtureAnimationOwner(session.owner));
        if let Some(mut driver) = world.get_mut::<MotionDriver>(actor) {
            driver.playing = None;
        }
        let token = world
            .resource_mut::<FixtureActivityTimelines>()
            .request_start(request);
        session.token = Some(token);
        session.phase = Phase::Playing;
        session.last_pending = None;
        info!(
            "[npc-fixture] unit={} start action={} timeline={} target={:?}/{}",
            session.selection.unit,
            session.selection.source.source_id(),
            session.selection.timeline.id,
            target.entity,
            target.uid
        );
        return Ok(false);
    }
    if matches!(session.phase, Phase::Playing) {
        let token = session
            .token
            .ok_or("playing activity has no timeline token")?;
        if world
            .get::<MotionDriver>(actor)
            .is_none_or(|driver| Some(driver.player) != session.animator)
            || world
                .get::<NpcFixtureAnimationOwner>(actor)
                .is_none_or(|held| held.0 != session.owner)
        {
            return Err("selected NPC animator generation or lease changed".into());
        }
        let in_talk = world.contains_resource::<crate::player_talk::PlayerTalkSession>()
            || world
                .get_resource::<crate::talk::ActiveTalk>()
                .is_some_and(|talk| talk.includes_player());
        let status = {
            let mut timelines = world.resource_mut::<FixtureActivityTimelines>();
            timelines.set_timeout_budget(token, Some(!in_talk));
            timelines
                .status(token)
                .cloned()
                .ok_or("shared timeline removed the selected token")?
        };
        match status {
            TimelineStatus::Preparing => return Ok(false),
            TimelineStatus::Playing { enable_talk, .. } => {
                // Parenting semantics without recursive-despawn ownership:
                // recompose against this same instance's live locator.
                let poses = live_poses(world, &session.selection)?;
                set_pose(world, actor, poses.0)?;
                // The loop clip with the talk flag (LoopFlagBehaviour): its
                // play enables talking and, unless the NPC is talking, changes
                // it to the fixture action idle state, so the player's talk
                // takes the playing-fixture row; its pause disables talking
                // and changes no state back.
                let entered = enable_talk && !session.talk_clip;
                session.talk_clip = enable_talk;
                let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
                if let Ok((mut actions, mut rest)) = query.get_mut(world, actor) {
                    actions.enable_talk = enable_talk;
                    if entered
                        && !matches!(
                            actions.current,
                            NpcAction::Talk | NpcAction::FixtureActionIdle
                        )
                    {
                        actions.change(NpcAction::FixtureActionIdle, &mut rest);
                        info!(
                            "[npc-fixture] unit={} action={} the loop clip enables talking; change to FixtureActionIdle",
                            session.selection.unit,
                            session.selection.source.source_id()
                        );
                    }
                }
                return Ok(false);
            }
            TimelineStatus::Completed => {
                // The talk owner observes the completed generation before
                // releasing its cast. Do not start locomotion under TalkHold.
                if joined_talk {
                    // CN NPCFixtureTimelineView.PlayAsyncForNPC stops its
                    // Director after the terminal
                    // wait; the controller then applies EndLoc. A completed
                    // sparse Root track must not remain below Hips-only talk.
                    let (_, end) = live_poses(world, &session.selection)?;
                    fixture_activity_timeline::cancel_and_release(world, token);
                    session.token = None;
                    release_animation_lease(world, session);
                    crate::npc::navigation::set_enabled(world, actor, true);
                    set_pose(world, actor, end)?;
                    if !world
                        .get::<MotionDriver>(actor)
                        .is_some_and(|driver| driver.alone_holds)
                    {
                        crate::character::resume_idle_after_fixture(world, actor)?;
                    }
                    session.phase = Phase::TalkHeld;
                    info!(
                        "[npc-fixture] unit={} source timeline completed; talk handoff holds EndLoc/idle",
                        session.selection.unit
                    );
                    return Ok(false);
                }
                world
                    .resource_mut::<FixtureActivityTimelines>()
                    .release(token);
                session.token = None;
                release_animation_lease(world, session);
                crate::npc::navigation::set_enabled(world, actor, true);
                change_action(world, actor, NpcAction::FixtureActionIdle, false)?;
                let (_, end) = live_poses(world, &session.selection)?;
                set_pose(world, actor, end)?;
                session.phase = Phase::Exiting(prepare_exit(world, actor, end, end)?);
                info!(
                    "[npc-fixture] unit={} source timeline ended; EndLoc exit begins",
                    session.selection.unit
                );
            }
            TimelineStatus::Cancelled => return Err("shared timeline was cancelled".into()),
            TimelineStatus::Failed(error) => {
                return Err(format!("source timeline failed: {error}"));
            }
        }
    }
    if let Phase::Exiting(movement) = &mut session.phase {
        let (pose, phase, done) = movement.step(dt);
        set_pose(world, actor, pose)?;
        world.entity_mut(actor).insert(phase);
        return Ok(done);
    }
    Ok(false)
}

/// TryPlayTweetAsync for a fixture talk: with a pre-action tweet, its id,
/// the tweet state Pending and the change to the tweet state; the window then
/// waits for Done. `None` without a tweet.
fn try_play_tweet(
    world: &mut World,
    actor: Entity,
    selection: &Selection,
    frame: u32,
) -> Result<Option<TalkWindow>, String> {
    let Some(tweet) = selection.tweet.as_ref().filter(|tweet| tweet.id != 0) else {
        return Ok(None);
    };
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    let (mut actions, mut rest) = query
        .get_mut(world, actor)
        .map_err(|_| "NPC action owner disappeared")?;
    actions.tweet_id = tweet.id;
    actions.tweet_state = crate::npc_state::TweetState::Pending;
    actions.change(NpcAction::Tweet, &mut rest);
    info!(
        "[npc unit={}] frame={frame} tweet {}: Pending, change to Tweet",
        selection.unit, tweet.id
    );
    Ok(Some(TalkWindow::TweetDone { since: frame }))
}

/// TryPlayTimelineAsync of a single-character fixture talk: the fixture
/// action state, then the timeline (the same playback as a no-talk action).
/// The controller's SetupAsync disables talking before it loads the asset,
/// as on the no-talk path.
fn start_talk_timeline(
    world: &mut World,
    actor: Entity,
    selection: &Selection,
    frame: u32,
) -> Result<Phase, String> {
    change_action(world, actor, NpcAction::FixtureAction, false)?;
    info!(
        "[npc unit={}] frame={frame} TryPlayTimelineAsync: change to FixtureAction, timeline {}",
        selection.unit, selection.timeline.id
    );
    if crate::npc_objective::events::enrolled(world, actor) {
        return start_hfsm_approach(world, actor, selection);
    }
    Ok(Phase::Preparing)
}

fn approach_point(world:&World,selection:&Selection)->Result<Option<Transform>,String> {
    let fixture=world.get::<GlobalTransform>(selection.target.entity).ok_or("fixture transform disappeared")?;
    let points=world.get_resource::<AttachPoints>().ok_or("fixture locator source loading")?
        .source_array(&selection.identity.model_package,fixture)?;
    Ok(points.get(selection.locate_index).or_else(||points.first()).map(|(_,pair)|
        Transform::from_translation(Vec3::from(pair.start.position)).with_rotation(pair.start.rotation)))
}
fn fixture_look_target(world:&World,actor:Entity,selection:&Selection)->Result<Vec3,String> {
    if let Some(point)=approach_point(world,selection)? {
        let at=world.get::<Transform>(actor).ok_or("fixture look actor disappeared")?.translation;
        Ok(at+point.rotation*Vec3::Z)
    } else {
        world.get::<GlobalTransform>(selection.target.entity).map(|pose|pose.translation()).ok_or("fixture transform disappeared".into())
    }
}
fn start_hfsm_approach(world:&mut World,actor:Entity,selection:&Selection)->Result<Phase,String> {
    world.entity_mut(actor).remove::<NpcFixtureMotionOwner>();
    Ok(Phase::HfsmApproach(ActionPointApproach::start_fixture(world, actor, selection.target.entity, selection.locate_index)?))
}

/// The helper owns its direct navigation and exact view writes. Animation
/// coroutines belong to the presenter and survive cancellation of this helper.
pub(crate) struct ActionPointApproach {
    point: Option<Vec3>,
    live_point: Option<(Entity, String, String)>,
    step: ApproachStep,
    owner: u64,
    original_stopping: Option<f32>,
    restore_navigation: bool,
}
enum ApproachStep {
    Walking { anchor: Vec3, stalled: f32 },
    Exact { target: Vec3, speed: f32, elapsed: f32, timeout: f32 },
    SwitchingIdle,
    Done,
}
#[derive(Resource,Default)]
struct NextApproachOwner(u64);
#[derive(Component)]
struct ActionPointExactMotion(u64);
#[derive(bevy::ecs::system::SystemParam)]
pub struct ActionPointMotionOwners<'w,'s> {
    actors: Query<'w,'s,Entity,With<ActionPointExactMotion>>,
}
impl ActionPointMotionOwners<'_, '_> {
    pub fn contains(&self,entity:Entity)->bool { self.actors.contains(entity) }
}
impl ActionPointApproach {
    pub(crate) fn start_fixture(world:&mut World,actor:Entity,fixture:Entity,index:usize)->Result<Self,String> {
        let package=world.get::<FixtureActivityIdentity>(fixture).ok_or("approach fixture identity missing")?.model_package.clone();
        let transform=world.get::<GlobalTransform>(fixture).ok_or("approach fixture pose missing")?;
        let points=world.get_resource::<AttachPoints>().ok_or("approach fixture points loading")?.source_array(&package,transform)?;
        let selected=points.get(index).or_else(||points.first());
        let point=selected.map(|(_,pair)|Vec3::from(pair.start.position));
        let live_point=selected.map(|(name,_)|(fixture,package,name.clone()));
        Self::start_optional(world,actor,point,live_point)
    }
    fn start_optional(world:&mut World,actor:Entity,point:Option<Vec3>,live_point:Option<(Entity,String,String)>)->Result<Self,String> {
        use crate::npc::navigation as nav;
        world.init_resource::<NextApproachOwner>();
        let owner={let mut next=world.resource_mut::<NextApproachOwner>();next.0=next.0.checked_add(1).expect("approach owner exhausted");next.0};
        world.entity_mut(actor).insert(ActionPointExactMotion(owner));
        let mut helper=Self { point,live_point,step:ApproachStep::Done,owner,original_stopping:None,restore_navigation:false };
        let initialized=(||->Result<(),String> {
        let Some(point)=point else { helper.switch_idle(world,actor)?;return Ok(()); };
        nav::refresh_binding(world,actor);
        let at=world.get::<Transform>(actor).ok_or("approach view missing")?.translation;
        let target=nav::face(world,actor).and_then(|face|face.sample(point.to_array(),0.5)).map(Vec3::from).unwrap_or(point);
        if at.distance(target)>0.01 && nav::active(world,actor) && nav::on_mesh(world,actor) {
            helper.step=ApproachStep::Walking {anchor:at,stalled:0.0};
            helper.original_stopping=nav::stopping_distance(world,actor);
            nav::set_stopping_distance(world,actor,0.01);
            change_action(world,actor,NpcAction::AutoMove,false)?;
            nav::set_destination(world,actor,owner,target);
            world.entity_mut(actor).insert(MotionPhase::Walking);
            if let Some(name)=world.get::<crate::npc::MotionClips>(actor).map(|clips|clips.walk.clone()) {
                crate::npc_objective::events::begin_animation(world,actor,&name,true,0.5)?;
            }
            helper.step=ApproachStep::Walking {anchor:at,stalled:0.0};
        } else { helper.start_exact(world,actor)?; }
        Ok(())
        })();
        if let Err(reason)=initialized {
            helper.cancel(world,actor);
            helper.release(world,actor);
            return Err(reason);
        }
        Ok(helper)
    }
    fn owns(&self,world:&World,actor:Entity)->bool {
        world.get::<ActionPointExactMotion>(actor).is_some_and(|held|held.0==self.owner)
    }
    fn release(&self,world:&mut World,actor:Entity) {
        if self.owns(world,actor) {
            crate::npc::navigation::release_movement(world, actor, self.owner);
            world.entity_mut(actor).remove::<ActionPointExactMotion>();
        }
    }
    fn restore_stopping(&mut self,world:&mut World,actor:Entity) {
        if let Some(distance)=self.original_stopping.take() {crate::npc::navigation::set_stopping_distance(world,actor,distance);}
    }
    fn switch_idle(&mut self,world:&mut World,actor:Entity)->Result<(),String> {
        crate::npc_objective::events::begin_animation(world,actor,"mov_cw_normal_idle001",false,0.5)?;
        self.step=ApproachStep::SwitchingIdle;
        Ok(())
    }
    fn finish(&mut self,world:&mut World,actor:Entity)->Result<(),String> {
        crate::npc::navigation::stop(world,actor,self.owner);
        if let Some(name)=world.get::<crate::npc::MotionClips>(actor).map(|clips|clips.idle.clone()) {
            crate::npc_objective::events::begin_animation(world,actor,&name,true,0.5)?;
        }
        self.step=ApproachStep::Done;
        self.release(world,actor);
        Ok(())
    }
    fn start_exact(&mut self,world:&mut World,actor:Entity)->Result<(),String> {
        use crate::npc::navigation as nav;
        let Some(at)=world.get::<Transform>(actor).map(|pose|pose.translation) else {return self.finish(world,actor);};
        let live = if let Some((fixture,package,name))=&self.live_point {
            world.get::<GlobalTransform>(*fixture).and_then(|pose|
                world.get_resource::<AttachPoints>()?.source_array(package,pose).ok())
                .and_then(|points|points.into_iter().find(|(current,_)|current==name))
                .map(|(_,point)|Vec3::from(point.start.position))
        } else { self.point };
        let Some(point)=live else {return self.finish(world,actor);};
        let target=Vec3::new(point.x,at.y,point.z);
        let distance=at.distance(target);
        let speed=world.get::<WalkSpeed>(actor).map_or(0.0,|speed|speed.0);
        if distance<0.005 || distance>0.5 || speed<=0.0 {return self.finish(world,actor);}
        let clips=world.get::<crate::npc::MotionClips>(actor).ok_or("approach motion names missing")?;
        let name=if distance>=0.15 {clips.walk.clone()} else {clips.idle.clone()};
        if distance>=0.15 {
            let animation=crate::npc_objective::events::begin_suffix_animation(world,actor,&name,
                crate::npc_objective::events::AnimationSuffix::Loop,true,false,0.1)?;
            crate::npc_objective::events::forget_animation_outcome(world,animation);
        } else {crate::npc_objective::events::begin_animation(world,actor,&name,false,0.15)?;}
        self.restore_navigation=nav::enabled(world,actor);
        nav::set_enabled(world,actor,false);
        world.entity_mut(actor).insert(if distance>=0.15 {MotionPhase::Walking} else {MotionPhase::Dwelling{remaining:None}});
        self.step=ApproachStep::Exact {target,speed,elapsed:0.0,timeout:distance/speed+1.0};
        Ok(())
    }
    fn finish_exact(&mut self,world:&mut World,actor:Entity) {
        use crate::npc::navigation as nav;
        if self.restore_navigation {
            if let Some(at)=world.get::<Transform>(actor).map(|pose|pose.translation) {
                if nav::face(world,actor).and_then(|face|face.sample(at.to_array(),0.05))
                    .is_some_and(|hit|at.distance_squared(Vec3::from(hit))<0.0025) {
                    nav::set_enabled(world,actor,true);
                }
            }
        }
    }
    pub(crate) fn update(&mut self,world:&mut World,actor:Entity,dt:f32)->Result<bool,String> {
        use crate::npc::navigation as nav;
        if matches!(self.step,ApproachStep::Done) {return Ok(true);}
        if !self.owns(world,actor) {return Err("action-point movement owner replaced".into());}
        match &mut self.step {
            ApproachStep::Walking{anchor,stalled} => {
                if !nav::active(world,actor) {
                    self.restore_stopping(world,actor);
                    self.switch_idle(world,actor)?;
                    return Ok(false);
                }
                let status=nav::status(world,actor,self.owner);
                if status.is_some_and(|status|status.pending) {return Ok(false);}
                let stopping=nav::stopping_distance(world,actor).unwrap_or(0.1);
                let exhausted=status.is_none_or(|status|!status.has_path || status.remaining<=stopping);
                let at=world.get::<Transform>(actor).ok_or("approach view disappeared")?.translation;
                if at.distance(*anchor)>0.05 {*anchor=at;*stalled=0.0;} else {*stalled+=dt;}
                if exhausted || *stalled>=3.0 {
                    if nav::on_mesh(world,actor) {nav::reset(world,actor,self.owner);}
                    self.restore_stopping(world,actor);
                    self.start_exact(world,actor)?;
                } else {nav::step(world,actor,self.owner,dt);}
            }
            ApproachStep::Exact{target,speed,elapsed,timeout} => {
                let Some(at)=world.get::<Transform>(actor).map(|pose|pose.translation) else {
                    self.finish_exact(world,actor);self.switch_idle(world,actor)?;return Ok(false);
                };
                *elapsed+=dt;
                let mut done=*elapsed>=*timeout;
                if !done {
                    let delta=*target-at;
                    let distance=delta.length();
                    let movement=dt**speed;
                    let next=if distance==0.0 || distance<=movement {*target} else {at+delta/distance*movement};
                    nav::set_view_position(world,actor,next);
                    if next.distance_squared(*target)<0.000025 {nav::set_view_position(world,actor,*target);done=true;}
                }
                if done {self.finish_exact(world,actor);self.finish(world,actor)?;}
            }
            ApproachStep::SwitchingIdle => {
                if !crate::npc_objective::events::animation_changing(world,actor) {
                    self.step=ApproachStep::Done;self.release(world,actor);
                }
            }
            ApproachStep::Done=>{}
        }
        Ok(matches!(self.step,ApproachStep::Done))
    }
    pub(crate) fn cancel(&self,world:&mut World,actor:Entity) {
        use crate::npc::navigation as nav;
        if matches!(self.step,ApproachStep::Done) || !self.owns(world,actor) {return;}
        if let Some(distance)=self.original_stopping {nav::set_stopping_distance(world,actor,distance);}
        if matches!(self.step,ApproachStep::Walking{..}) && nav::active(world,actor) && nav::on_mesh(world,actor) {
            nav::reset(world,actor,self.owner);
        }
        if matches!(self.step,ApproachStep::Exact{..}) && self.restore_navigation {
            if let Some(at)=world.get::<Transform>(actor).map(|pose|pose.translation) {
                if nav::face(world,actor).and_then(|face|face.sample(at.to_array(),0.05))
                    .is_some_and(|hit|at.distance_squared(Vec3::from(hit))<0.0025) {nav::set_enabled(world,actor,true);}
            }
        }
        self.release(world,actor);
    }
}

fn finish_hfsm_approach(
    world: &mut World,
    actor: Entity,
    selection: &Selection,
) -> Result<Phase, String> {
    change_action(world, actor, NpcAction::FixtureAction, false)?;
    if selection.kind == TalkType::NoneTalk {
        let target = fixture_look_target(world, actor, selection)?;
        let transform = world
            .get::<Transform>(actor)
            .ok_or("fixture look actor disappeared")?;
        let forward = (transform.rotation * Vec3::Z).to_array();
        let duration = rotate_time(angle_between(
            forward,
            (target - transform.translation).to_array(),
        ));
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        world
            .entity_mut(actor)
            .insert(crate::npc_look_at::NpcLookAt::new(
                target,
                forward,
                duration,
                moly_law::ui::dotween::Ease::OutQuart,
                frame,
            ));
        Ok(Phase::HfsmNoneLooking)
    } else {
        Ok(Phase::Preparing)
    }
}

pub fn prepare_exit(
    world: &World,
    actor: Entity,
    start: Transform,
    end: Transform,
) -> Result<LocalMove, String> {
    let speed = world
        .get::<WalkSpeed>(actor)
        .ok_or("source NPC walk speed is missing")?
        .0;
    let sample = crate::npc::navigation::face(world, actor)
        .and_then(|face| face.sample(end.translation.to_array(), 2.0));
    // Source samples the authored EndLoc and uses it as fallback. The delayed
    // talk handoff changes only the movement's initial live pose, not its goal.
    let target = sample.map(Vec3::from).unwrap_or(end.translation);
    let distance = end.translation.distance(target);
    let speed = if distance >= 0.1 { speed } else { speed / 5.0 };
    let direction = (target - end.translation).normalize_or_zero();
    let rotation = if sample.is_none() || direction.dot(end.rotation * Vec3::Z) > 0.9 {
        end.rotation
    } else if direction == Vec3::ZERO {
        Quat::IDENTITY // Unity LookRotation(zero) yields identity.
    } else {
        Quat::from_rotation_y(direction.x.atan2(direction.z))
    };
    LocalMove::new(start, target, rotation, speed)
}

fn pending(session: &mut Session, reason: &str) -> Result<bool, String> {
    if session.last_pending.as_deref() != Some(reason) {
        info!(
            "[npc-fixture] unit={} action={} preparation pending: {reason}",
            session.selection.unit,
            session.selection.source.source_id()
        );
        session.last_pending = Some(reason.into());
    }
    Ok(false)
}

fn prepare(
    world: &mut World,
    provider: &mut FixtureActivityProvider,
    session: &mut Session,
) -> Result<(), crate::fixture_activity_provider::ProviderPending> {
    let selection = &session.selection;
    let view_y = world
        .get::<SourceFixtureViewLocalY>(selection.target.entity)
        .ok_or("source FixtureView local Y is not installed")?
        .0;
    let (package, prefab) = crate::fixture_activity_data::timeline_asset(
        &selection.timeline.asset_name,
        selection.put_type,
        view_y,
    )
    .map_err(|error| format!("source timeline route: {error:?}"))?;
    let (animator, graph) = world
        .get::<MotionDriver>(session.owner.actor)
        .map(|driver| (driver.player, driver.graph.clone()))
        .ok_or("selected NPC body is not installed")?;
    if session.animator.is_some_and(|old| old != animator) {
        return Err("selected NPC body generation changed".into());
    }
    if session.request.is_none() {
        let definition = provider.definition(world, &package, &prefab)?;
        session.request = Some(StartTimeline {
            owner: TimelineOwner {
                activity: session.owner,
                kind: TimelineOwnerKind::Npc,
            },
            fixture: selection.target.entity,
            definition,
            bindings: TimelineBindings::default(),
            companions: Vec::new(),
            timeout_secs: 30.0,
            timeout_budget: TimelineTimeoutBudget::OwnerGated {
                advance: Some(true),
            },
        });
    }
    let request = session.request.as_mut().expect("installed request");
    provider.prepare_bindings(world, request, selection.unit, animator, graph)?;
    if !request
        .bindings
        .animations
        .values()
        .any(|binding| binding.animator == animator)
    {
        return Err("source-selected NPC timeline has no actual body binding".into());
    }
    fixture_activity_timeline::validate_start(world, request).map_err(|error| {
        crate::fixture_activity_provider::ProviderPending {
            stage: "live-binding-preflight",
            reason: error.to_string(),
            retryable: error.retryable,
        }
    })?;
    session.animator = Some(animator);
    Ok(())
}

fn live_poses(world: &World, selection: &Selection) -> Result<(Transform, Transform), String> {
    let fixture = world
        .get::<GlobalTransform>(selection.target.entity)
        .ok_or("fixture transform disappeared")?;
    let points = world
        .get_resource::<AttachPoints>()
        .ok_or("source locators disappeared")?;
    if points.instance_index(&selection.identity.model_package, selection.point)
        != Some(selection.locate_index)
    {
        return Err("source locator array identity changed".into());
    }
    let poses = points
        .instance_poses(&selection.identity.model_package, selection.point, fixture)
        .ok_or("source locator pose disappeared")?;
    let end = poses.end.ok_or("source EndLoc disappeared")?;
    let pose = |pose: crate::fixture_attach::AttachPose| {
        Transform::from_translation(Vec3::from(pose.position)).with_rotation(pose.rotation)
    };
    Ok((pose(poses.start), pose(end)))
}

pub fn set_pose(world: &mut World, actor: Entity, pose: Transform) -> Result<(), String> {
    if !pose.translation.is_finite() || !pose.rotation.is_finite() {
        return Err("nonfinite source actor pose".into());
    }
    // NPC logical roots currently have no transform parent. Do not replace a
    // parent adopted by another owner with a synthetic fixture-relative pose.
    if world.get::<ChildOf>(actor).is_some() {
        return Err("logical NPC has an unowned transform parent".into());
    }
    let mut walk = world
        .get_mut::<WalkState>(actor)
        .ok_or("NPC walk state disappeared")?;
    walk.0.position = pose.translation.to_array();
    walk.0.forward = (pose.rotation * Vec3::Z).to_array();
    walk.0.next_corner = 0;
    drop(walk);
    let scale = world
        .get::<Transform>(actor)
        .ok_or("NPC transform disappeared")?
        .scale;
    let pose = Transform { scale, ..pose };
    world
        .entity_mut(actor)
        .insert((pose, GlobalTransform::from(pose)));
    Ok(())
}

fn change_action(
    world: &mut World,
    actor: Entity,
    action: NpcAction,
    enable_talk: bool,
) -> Result<(), String> {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    let (mut actions, mut rest) = query
        .get_mut(world, actor)
        .map_err(|_| "NPC action owner disappeared")?;
    actions.change(action, &mut rest);
    actions.enable_talk = enable_talk;
    Ok(())
}

fn release_animation_lease(world: &mut World, session: &Session) {
    let actor = session.owner.actor;
    if let Phase::HfsmApproach(approach) = &session.phase {
        approach.cancel(world, actor);
    }
    if world
        .get::<NpcFixtureAnimationOwner>(actor)
        .is_some_and(|held| held.0 == session.owner)
    {
        world.entity_mut(actor).remove::<NpcFixtureAnimationOwner>();
        if let Some(mut driver) = world.get_mut::<MotionDriver>(actor) {
            if Some(driver.player) == session.animator {
                driver.playing = None;
            }
        }
    }
}

fn dispose(world: &mut World, session: Session, site_changed: bool, completed: bool) {
    preview::record_disposed(world, &session, completed);
    crate::balloon::cancel_activity_balloon(world, session.owner);
    let actor = session.owner.actor;
    if let Some(token) = session.token {
        if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
            timelines.cancel(token);
            timelines.release(token);
        }
    }
    let owned_timeline = world.get::<NpcFixtureAnimationOwner>(actor)
        .is_some_and(|held| held.0 == session.owner);
    release_animation_lease(world, &session);
    if owned_timeline { crate::npc::navigation::set_enabled(world, actor, true); }
    let holds_motion = world
        .get::<NpcFixtureMotionOwner>(actor)
        .is_some_and(|held| held.0 == session.owner);
    if holds_motion {
        world.entity_mut(actor).remove::<NpcFixtureMotionOwner>();
    }
    if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
        reservations.release_owner(session.owner);
    }
    if site_changed || !world.entities().contains(actor) {
        return;
    }
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let mut query = world.query::<(
        &CharacterUnitId,
        &PauseSeconds,
        &mut ObjectiveMind,
        &mut TalkSlot,
        &mut NpcActions,
        &mut RestLifecycle,
        &mut RouteStops,
        &mut PathSlot,
        &mut MotionPhase,
    )>();
    if let Ok((
        unit,
        _pause,
        mut mind,
        mut slot,
        mut actions,
        mut rest,
        mut route,
        mut path,
        mut phase,
    )) = query.get_mut(world, actor)
    {
        let still_ours = slot.current.as_ref().is_some_and(|data| {
            data.kind == session.selection.kind
                && data.target_fixture == Some(session.selection.target.entity)
        });
        if !still_ours {
            return;
        }
        // Never overwrite a separately admitted conversation's state/animation.
        if actions.current == NpcAction::Talk {
            if let Some(data) = slot.current.as_mut() {
                data.target_fixture = None;
            }
            mind.executing = false;
            return;
        }
        if completed {
            // The exit already chose its surface position.
            route.finish_fixture();
        } else {
            // A cancel can leave the character on its locator, off the
            // walk face: the next move starts from the point the fit left.
            route.cancel();
        }
        path.0 = NpcPathWalkSlot::from_corners(Vec::new());
        *phase = MotionPhase::Dwelling { remaining: None };
        actions.enable_talk = session.previous_enable_talk;
        if completed {
            // The timeline controller's disposal puts its characters in Rest.
            actions.change(NpcAction::Rest, &mut rest);
        }
        if completed && session.selection.kind == TalkType::SingleCharacterFixture {
            // The timeline's end runs ForceUpdateObjective on its character:
            // the talk objective's cancel releases its claim, then one call;
            // the objective's wait sees the cancel on the next frame.
            crate::npc_objective::owe_force_updates(&mut mind, frame.wrapping_add(1), 1);
        } else {
            // The objective's end: the ForceUpdateObjective calls it owes (two
            // after the timeline end, one after a failed approach), then the Rest.
            crate::npc_objective::finish_objective(unit.0, &mut mind, &slot, frame, !completed);
        }
        info!(
            "[npc-fixture] unit={} action={} disposed completed={completed}; instance/slot leases released",
            unit.0,
            session.selection.source.source_id()
        );
    }
}

impl NpcFixtureActivities {
    /// A fresh activity owner for `actor` (the group talks' claims).
    pub fn next_owner(&mut self, actor: Entity) -> FixtureActivityOwner {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("NPC fixture activity generation exhausted");
        FixtureActivityOwner {
            actor,
            generation: self.next_generation,
        }
    }
}

/// Whether a no-talk objective's fixture session drives `actor`.
pub fn owns_actor(runtime: &NpcFixtureActivities, actor: Entity) -> bool {
    runtime.sessions.contains_key(&actor)
}

/// The greeting cancelled the running no-talk objective of `actor`: the
/// objective's dispose releases its fixture claims, its timeline token and
/// its animation lease. The greeting has already replaced the talk data, so
/// the dispose leaves the objective state to the greeting.
pub fn cancel_for_greeting(world: &mut World, actor: Entity) {
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    runtime.pending_factories.remove(&actor);
    if let Some(session) = runtime.sessions.remove(&actor) {
        dispose(world, session, false, false);
    }
    world.insert_resource(runtime);
}

pub fn cancel_for_site_change(world: &mut World) {
    cancel_runtime(world, true);
}

/// Unlike site destruction, the NPC remains alive after editor entry. Its
/// existing normal disposal branch clears only the matching no-talk target,
/// path and objective while respecting a separately acquired talk owner.
pub fn cancel_for_layout_edit(world: &mut World) {
    cancel_runtime(world, false);
}

fn cancel_runtime(world: &mut World, site_changed: bool) {
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    for (actor, session) in std::mem::take(&mut runtime.sessions) {
        if site_changed && world.get::<crate::fixture::residency::RetainedFixture>(session.selection.target.entity).is_some() {
            runtime.sessions.insert(actor, session);
            continue;
        }
        if !site_changed
            && world
                .get::<NpcActions>(actor)
                .is_some_and(|actions| actions.current == NpcAction::Talk)
        {
            // Presenter.IsEnabledCancelCondition refuses Talk(4). Hiding the
            // actor does not authorize releasing another talk owner's leases.
            runtime.sessions.insert(actor, session);
            continue;
        }
        dispose(world, session, site_changed, false);
    }
    runtime.pending_factories.clear();
    runtime.reported_gaps.clear();
    world.insert_resource(runtime);
}

/// Existing host's local no-NavMesh interpolation, with the source exit speed
/// and phase order: move first, then turn to the exit orientation. It is not a
/// replacement for the native agent's on-mesh reattachment/collision behavior.
pub struct LocalMove {
    pose: Transform,
    leg: crate::npc::FitLeg,
    final_rotation: Quat,
    elapsed: f32,
    turn: Option<(Quat, f32, f32)>,
}

impl LocalMove {
    pub(crate) fn new(
        start: Transform,
        target: Vec3,
        rotation: Quat,
        speed: f32,
    ) -> Result<Self, String> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err("NPC exit speed must retain its positive source value".into());
        }
        let delta = target - start.translation;
        let move_rotation = if delta.x == 0.0 && delta.z == 0.0 {
            start.rotation
        } else {
            Quat::from_rotation_y(delta.x.atan2(delta.z))
        };
        Ok(Self {
            pose: start,
            leg: crate::npc::FitLeg {
                start: start.translation.to_array(),
                target: target.to_array(),
                travel: delta.length() / speed,
                move_quat: move_rotation,
            },
            final_rotation: rotation,
            elapsed: 0.0,
            turn: None,
        })
    }

    pub fn step(&mut self, dt: f32) -> (Transform, MotionPhase, bool) {
        let mut turn_delta = dt;
        if self.turn.is_none() {
            self.elapsed += dt;
            if self.leg.travel > 0.0 && self.elapsed < self.leg.travel {
                let position = self.leg.sample_position(self.elapsed);
                // CalcMoveLerp derives heading from this frame's proposed
                // position minus the live transform. The shared kernel owns
                // the 1.5 compounded quaternion lerp and yaw projection.
                let delta = position - self.pose.translation;
                if delta.length() > 0.00001 && (delta.x != 0.0 || delta.z != 0.0) {
                    self.leg.move_quat = Quat::from_rotation_y(delta.x.atan2(delta.z));
                    self.pose.rotation = self.leg.sample_rotation(self.pose.rotation, self.elapsed);
                }
                self.pose.translation = position;
                return (self.pose, MotionPhase::Walking, false);
            }
            // NoUseNavmeshMoveLerpAsync completes with SetPosition only.
            // Its per-frame CalcMoveLerp is not evaluated again at t=1.
            self.pose.translation = Vec3::from(self.leg.target);
            let forward = (self.pose.rotation * Vec3::Z).to_array();
            let mut duration = rotate_time(angle_between(
                forward,
                (Vec3::from(self.leg.target) - self.pose.translation).to_array(),
            ));
            if duration == 0.0 {
                // Source first passes the target POSITION to GetRotateTime;
                // after exact landing that is zero. DOLocalRotateAsync then
                // passes the Euler END VALUE as a world vector. Do not replace
                // this strange location-dependent fallback with yaw error/60.
                let (y, x, z) = self.final_rotation.to_euler(EulerRot::YXZ);
                let end_value = Vec3::new(
                    make_positive_yaw(x.to_degrees()),
                    make_positive_yaw(y.to_degrees()),
                    make_positive_yaw(z.to_degrees()),
                );
                duration = rotate_time(angle_between(
                    forward,
                    (end_value - self.pose.translation).to_array(),
                ));
            }
            self.turn = Some((self.pose.rotation, duration, 0.0));
            // Do not spend this frame's complete delta on both phases.
            turn_delta = 0.0;
        }
        let (from, duration, elapsed) = self.turn.as_mut().expect("installed exit turn");
        *elapsed += turn_delta;
        let t = if *duration <= 0.0 {
            1.0
        } else {
            (*elapsed / *duration).min(1.0)
        };
        self.pose.rotation = crate::npc::sample_turn_rotation(*from, self.final_rotation, t);
        let angle = turn_angle(
            heading_yaw((*from * Vec3::Z).to_array()),
            heading_yaw((self.final_rotation * Vec3::Z).to_array()),
        );
        let phase = if t >= 1.0 {
            MotionPhase::Dwelling { remaining: None }
        } else {
            MotionPhase::Turning {
                motion: turn_motion(angle),
                from: *from,
                to: self.final_rotation,
                duration: *duration,
                elapsed: *elapsed,
            }
        };
        (self.pose, phase, t >= 1.0)
    }
}

#[cfg(test)]
mod completed_handoff_tests {
    use super::*;

    #[test]
    fn completed_endloc_updates_navigation_without_reflecting_or_rescaling_again() {
        let mut world = World::new();
        let actor = world
            .spawn((
                Transform::from_xyz(-0.5, 0.0, -0.9).with_scale(Vec3::splat(1.2)),
                WalkState(moly_law::path::WalkState::new(
                    [-0.5, 0.0, -0.9],
                    [0., 0., 1.],
                )),
            ))
            .id();
        let end = Transform::from_xyz(0.625, 0., -0.775)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        set_pose(&mut world, actor, end).unwrap();
        let pose = world.get::<Transform>(actor).unwrap();
        let walk = &world.get::<WalkState>(actor).unwrap().0;
        assert_eq!(pose.translation, end.translation);
        assert_eq!(pose.rotation, end.rotation);
        assert_eq!(pose.scale, Vec3::splat(1.2));
        assert_eq!(walk.position, end.translation.to_array());
        assert_eq!(walk.forward, (end.rotation * Vec3::Z).to_array());
    }

    #[test]
    fn delayed_talk_exit_starts_at_live_pose_without_second_endloc_teleport() {
        let mut world = World::new();
        let actor = world.spawn(WalkSpeed(1.0)).id();
        let end = Transform::from_xyz(0.625, 0., -0.775)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        let live =
            Transform::from_xyz(0.725, 0., -0.775).with_rotation(Quat::from_rotation_y(0.14));
        let mut movement = prepare_exit(&world, actor, live, end).unwrap();
        assert_eq!(movement.pose, live);
        assert_eq!(movement.leg.start, live.translation.to_array());
        assert_eq!(movement.leg.target, end.translation.to_array());
        let (first, _, done) = movement.step(0.0);
        assert_eq!(first.translation, live.translation);
        assert_eq!(first.rotation, live.rotation);
        assert!(!done);
    }
}
