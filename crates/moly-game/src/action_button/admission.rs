//! Rising-edge admission for the furniture action buttons.
//!
//! AddShowButtonStack runs once, when the player's box starts to overlap one
//! fixture. For furniture it first asks IsActionButtonTypeAvailable
//! (CheckTargetSite, then CanShowFixtureActionButton): an unavailable fixture
//! is neither pushed nor removed. It then asks IsCanActionFixture: a fixture
//! that cannot act is removed, one that can is pushed when absent.
//! RemoveNotCollisionObject asks availability again and nothing else.
//!
//! CheckTargetSite holds by construction: fixture roots exist only for the
//! active site's layout, and a site change replaces every one of them.
//!
//! The locator rules below are the player's furniture-action rules. The
//! fallback after a failed direct locator test differs by the loaded
//! snapshot's region; see [`can_move_to_locator_with_fallback`].
//!
//! The source always has its inputs. Moly can lack one (navigation not yet
//! installed for this site, identity not yet bound). Such a fixture is
//! deferred: it has not joined the colliding set, so its rising edge is
//! evaluated again once the input exists.

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_law::{carve::NavMeshRegion, objective::TILE_SCALE};

use crate::{
    fixture_activity_data::FixtureActivityTables,
    fixture_activity_state::{FixtureActivityIdentity, FixtureActivityReservations, FixtureTarget},
    fixture_attach::AttachPoints,
    fixture_scene_inputs::FixtureSceneSupply,
    player_fixture_action::{
        FixtureTileKnowledge, PlayerFixtureNavigation, PlayerFixturePreparationError,
    },
    site::{GroundEpoch, NavMeshSourceRegion},
};

/// CanShowFixtureActionButton samples the selected StartLoc within this
/// distance, in both regions.
const START_SAMPLE_DISTANCE: f32 = 3.0;

/// The threshold every CanNavmeshMoveTargetPosition call on these paths passes.
const REACH_THRESHOLD: f32 = 0.01;

/// The two furniture buttons with an admission rule of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FixtureKind {
    Gimmick,
    Timeline,
}

/// IsActionButtonTypeAvailable for one furniture button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Availability {
    Available,
    Unavailable,
    /// JP GetNearestPlayerLocatorIndex reads the player locator list, which
    /// only timeline furniture with player rows has. With a non-empty attach
    /// array and no such list the source raises inside AddShowButtonStack,
    /// so the fixture is neither pushed nor removed.
    Raises,
}

/// The stack effect of one rising edge, after the lock check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Enter {
    /// Available and IsCanActionFixture: push when absent.
    Push,
    /// Available but IsCanActionFixture is false: RemoveShowButtonStack.
    Remove,
    /// Neither push nor removal.
    Skip(&'static str),
}

/// The navigation snapshot a deferred evaluation was made against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InputRevision {
    site: u64,
    navigation: u64,
    scene: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retry {
    /// A resource or identity is not installed yet; cheap to ask again.
    NextFrame,
    /// This navigation snapshot cannot answer; ask a different snapshot.
    NewInputs(InputRevision),
    /// Static attachment data cannot answer; the fixture joins the colliding
    /// set without a button.
    Never,
}

#[derive(Debug)]
pub(crate) struct Deferred {
    pub(crate) reason: String,
    pub(crate) retry: Retry,
}

impl Deferred {
    fn next_frame(reason: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            retry: Retry::NextFrame,
        }
    }

    fn never(reason: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            retry: Retry::Never,
        }
    }
}

/// One player pose against one placed fixture.
pub(crate) struct Probe<'a> {
    pub(crate) player: Entity,
    /// The player's world position.
    pub(crate) position: Vec3,
    pub(crate) target: &'a FixtureTarget,
    pub(crate) identity: &'a FixtureActivityIdentity,
    /// The fixture root's world transform.
    pub(crate) world: &'a GlobalTransform,
    pub(crate) kind: FixtureKind,
}

/// FixtureModel.PlayerTimelineLocates: one seat per attach entry whose parsed
/// StartLoc point has a player timeline row.
struct Seat {
    /// The attach array index.
    index: usize,
    slot: i32,
    start: Vec3,
    end: Vec3,
}

#[derive(SystemParam)]
pub(crate) struct AdmissionInputs<'w> {
    navigation: Option<Res<'w, PlayerFixtureNavigation>>,
    region: Option<Res<'w, NavMeshSourceRegion>>,
    tables: Option<Res<'w, FixtureActivityTables>>,
    points: Option<Res<'w, AttachPoints>>,
    reservations: Option<Res<'w, FixtureActivityReservations>>,
    epoch: Option<Res<'w, GroundEpoch>>,
    supply: Option<Res<'w, FixtureSceneSupply>>,
    walk: Option<Res<'w, crate::walk_face::WalkFace>>,
}

impl AdmissionInputs<'_> {
    pub(crate) fn site_epoch(&self) -> Option<u64> {
        self.epoch.as_deref().map(|epoch| epoch.0)
    }

    /// The installed navigation snapshot's revision.
    pub(crate) fn revision(&self) -> Option<InputRevision> {
        self.navigation.as_deref().map(|navigation| InputRevision {
            site: navigation.site_generation,
            navigation: navigation.navigation_generation,
            scene: navigation.scene_stamp.input_revision,
        })
    }

    /// AddShowButtonStack after its lock check.
    pub(crate) fn enter(&self, probe: &Probe) -> Result<Enter, Deferred> {
        match self.availability(probe)? {
            Availability::Unavailable => {
                return Ok(Enter::Skip("IsActionButtonTypeAvailable is false"))
            }
            Availability::Raises => {
                return Ok(Enter::Skip(
                    "the source raises reading a missing player locator list",
                ))
            }
            Availability::Available => {}
        }
        Ok(if self.can_action(probe)? {
            Enter::Push
        } else {
            Enter::Remove
        })
    }

    /// IsActionButtonTypeAvailable: CheckTargetSite (held by construction),
    /// then CanShowFixtureActionButton.
    pub(crate) fn availability(&self, probe: &Probe) -> Result<Availability, Deferred> {
        let ids = self.action_points(probe)?;
        // ActionPoints IsNullOrEmpty. The attach view lists entries whose
        // StartLoc name parses; no furniture with a gimmick or timeline action
        // has an entry whose name does not parse.
        if ids.is_empty() {
            return Ok(Availability::Available);
        }
        let seats = self.seats(probe, &ids)?;
        let region = self.region()?;
        if seats.is_none() && region == NavMeshRegion::Jp {
            return Ok(Availability::Raises);
        }
        let navigation = self.navigation(probe.player)?;
        // GetNearestPlayerLocatorIndex: without a qualifying seat (or, in CN,
        // without a seat list) it returns its default, index 0.
        let mut nearest: Option<(f32, usize)> = None;
        for seat in seats.iter().flatten() {
            if self.slot_in_use(probe, seat.slot)?
                || !self.seat_reachable(probe, navigation, region, seat)?
            {
                continue;
            }
            // The query's boolean is not read; adjacent corners are summed.
            let path = navigation
                .world
                .path(probe.position, seat.start)
                .map_err(|error| self.undecided(error))?;
            let length = path_length(&path.corners).ok_or_else(|| {
                self.undecided(PlayerFixturePreparationError::Invalid(
                    "navigation returned a nonfinite locator corner buffer".into(),
                ))
            })?;
            if nearest.is_none_or(|(shortest, _)| length < shortest) {
                nearest = Some((length, seat.index));
            }
        }
        let start = self.start_of(probe, &ids, nearest.map_or(0, |(_, index)| index))?;
        let Some(hit) = navigation
            .world
            .sample_position(start, START_SAMPLE_DISTANCE)
            .filter(|hit| hit.is_finite())
        else {
            return Ok(Availability::Unavailable);
        };
        Ok(
            if can_navmesh_move(navigation, probe.position, hit)
                .map_err(|error| self.undecided(error))?
            {
                Availability::Available
            } else {
                Availability::Unavailable
            },
        )
    }

    /// CanShowHouseEntryButton: GameState Normal (the scan runs only then)
    /// and CanMoveDoorActionPoint, `MoveUtility.CanNavmeshMoveTargetPosition`
    /// to the house's inside-door point: the static path query (both ends
    /// mapped within its query box) succeeds and its last corner is within
    /// `door_law::HOUSE_ENTRY_REACH` of the point horizontally.
    pub(crate) fn house_entry(
        &self,
        player: Entity,
        position: Vec3,
        door: Vec3,
    ) -> Result<bool, Deferred> {
        self.navigation(player)?;
        let walk = self
            .walk
            .as_deref()
            .ok_or_else(|| Deferred::next_frame("walk field"))?;
        Ok(walk.field.can_navmesh_move_target_position(
            [position.x, position.z],
            [door.x, door.z],
            crate::site_move::door_law::HOUSE_ENTRY_REACH,
        ))
    }

    /// IsCanActionFixture: a gimmick always can; a timeline fixture needs
    /// IsPlayableActionPoints and CanPlayerTimelineAction.
    pub(crate) fn can_action(&self, probe: &Probe) -> Result<bool, Deferred> {
        if probe.kind == FixtureKind::Gimmick {
            return Ok(true);
        }
        let ids = self.action_points(probe)?;
        let points = self.points()?;
        let package = &probe.identity.model_package;
        // IsPlayableActionPoints: some attach entry whose slot is free.
        let mut playable = false;
        for &id in &ids {
            let slot = points
                .instance_slot(package, id)
                .ok_or_else(|| Deferred::never("original locator slot suffix"))?;
            if !self.slot_in_use(probe, slot)? {
                playable = true;
                break;
            }
        }
        if !playable {
            return Ok(false);
        }
        // CanPlayerTimelineAction: a null or empty seat list cannot act;
        // otherwise any seat CanMoveToLocator reaches.
        let Some(seats) = self.seats(probe, &ids)? else {
            return Ok(false);
        };
        if seats.is_empty() {
            return Ok(false);
        }
        let navigation = self.navigation(probe.player)?;
        let region = self.region()?;
        let mut undecided = None;
        for seat in &seats {
            match self.seat_reachable(probe, navigation, region, seat) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(deferred) => {
                    undecided.get_or_insert(deferred);
                }
            }
        }
        undecided.map_or(Ok(false), Err)
    }

    fn points(&self) -> Result<&AttachPoints, Deferred> {
        self.points
            .as_deref()
            .ok_or_else(|| Deferred::next_frame("fixture attach points"))
    }

    fn region(&self) -> Result<NavMeshRegion, Deferred> {
        self.region
            .as_deref()
            .map(|region| region.0)
            .ok_or_else(|| Deferred::next_frame("navigation source region"))
    }

    fn slot_in_use(&self, probe: &Probe, slot: i32) -> Result<bool, Deferred> {
        Ok(self
            .reservations
            .as_deref()
            .ok_or_else(|| Deferred::next_frame("shared fixture reservations"))?
            .action_slot_in_use(probe.target, slot))
    }

    /// The same admission inputs the player's furniture action validates.
    fn navigation(&self, player: Entity) -> Result<&PlayerFixtureNavigation, Deferred> {
        let navigation = self
            .navigation
            .as_deref()
            .ok_or_else(|| Deferred::next_frame("player navigation supplier"))?;
        let epoch = self
            .site_epoch()
            .ok_or_else(|| Deferred::next_frame("site generation"))?;
        if navigation.site_generation != epoch {
            return Err(Deferred::next_frame("current-site navigation snapshot"));
        }
        if !navigation.agent_radius.is_finite() || navigation.agent_radius <= 0.0 {
            return Err(Deferred::next_frame("positive player agent radius"));
        }
        let current = self
            .supply
            .as_deref()
            .and_then(FixtureSceneSupply::current)
            .ok_or_else(|| Deferred::next_frame("current fixture scene inputs"))?;
        if current.stamp != navigation.scene_stamp {
            return Err(Deferred::next_frame(
                "NavMesh installed for the current fixture scene input revision",
            ));
        }
        if navigation.scene_stamp.player != player {
            return Err(Deferred::next_frame(
                "navigation parameters for the current logical player",
            ));
        }
        Ok(navigation)
    }

    fn undecided(&self, error: PlayerFixturePreparationError) -> Deferred {
        let reason = format!("{error:?}");
        match self.revision() {
            Some(revision) => Deferred {
                reason,
                retry: Retry::NewInputs(revision),
            },
            None => Deferred {
                reason,
                retry: Retry::NextFrame,
            },
        }
    }

    /// Parseable attach entries, in the source array's order.
    fn action_points(&self, probe: &Probe) -> Result<Vec<i32>, Deferred> {
        self.points()?
            .player_action_points(&probe.identity.model_package)
            .ok_or_else(|| Deferred::never("original attach array metadata"))
    }

    /// SetupPlayerTimelineLocateIndexes creates the list only for timeline
    /// furniture that has player timeline rows. `None` is that null list.
    fn seats(&self, probe: &Probe, ids: &[i32]) -> Result<Option<Vec<Seat>>, Deferred> {
        if probe.kind != FixtureKind::Timeline {
            return Ok(None);
        }
        let rows: Vec<i32> = self
            .tables
            .as_deref()
            .ok_or_else(|| Deferred::next_frame("fixture activity tables"))?
            .player_timelines(probe.identity.master_id)
            .map(|row| row.action_point)
            .collect();
        if rows.is_empty() {
            return Ok(None);
        }
        let points = self.points()?;
        let package = &probe.identity.model_package;
        let mut seats = Vec::new();
        for &id in ids {
            if !rows.contains(&id) {
                continue;
            }
            let index = points
                .instance_index(package, id)
                .ok_or_else(|| Deferred::never("unique original locator array index"))?;
            let slot = points
                .instance_slot(package, id)
                .ok_or_else(|| Deferred::never("original locator slot suffix"))?;
            let poses = points
                .instance_poses(package, id, probe.world)
                .ok_or_else(|| Deferred::never("instance StartLoc/EndLoc"))?;
            let end = poses.end.ok_or_else(|| Deferred::never("instance EndLoc"))?;
            seats.push(Seat {
                index,
                slot,
                start: Vec3::from(poses.start.position),
                end: Vec3::from(end.position),
            });
        }
        Ok(Some(seats))
    }

    /// The world StartLoc of the attach array entry at `index`.
    fn start_of(&self, probe: &Probe, ids: &[i32], index: usize) -> Result<Vec3, Deferred> {
        let points = self.points()?;
        let package = &probe.identity.model_package;
        let id = ids
            .iter()
            .copied()
            .find(|&id| points.instance_index(package, id) == Some(index))
            .ok_or_else(|| Deferred::never("StartLoc of the selected attach array entry"))?;
        let poses = points
            .instance_poses(package, id, probe.world)
            .ok_or_else(|| Deferred::never("instance StartLoc"))?;
        Ok(Vec3::from(poses.start.position))
    }

    /// CanMoveToLocator(index, radius, uid): a free slot, then StartLoc, then
    /// EndLoc, each through the region's fallback.
    fn seat_reachable(
        &self,
        probe: &Probe,
        navigation: &PlayerFixtureNavigation,
        region: NavMeshRegion,
        seat: &Seat,
    ) -> Result<bool, Deferred> {
        if self.slot_in_use(probe, seat.slot)? {
            return Ok(false);
        }
        let uid = probe.target.uid.as_str();
        let reach = |locator| {
            can_move_to_locator_with_fallback(navigation, region, probe.position, locator, uid)
                .map_err(|error| self.undecided(error))
        };
        Ok(reach(seat.start)? && reach(seat.end)?)
    }
}

fn path_length(path: &[Vec3]) -> Option<f32> {
    if path.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let length: f32 = path.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
    length.is_finite().then_some(length)
}

/// CanNavmeshMoveTargetPosition: the query's boolean, then the last corner's
/// strictly-less horizontal distance. Height and path status are not read.
pub(crate) fn can_navmesh_move(
    navigation: &PlayerFixtureNavigation,
    from: Vec3,
    target: Vec3,
) -> Result<bool, PlayerFixturePreparationError> {
    let path = navigation.world.path(from, target)?;
    if !path.query_succeeded {
        return Ok(false);
    }
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
    Ok(Vec2::new(last.x - target.x, last.z - target.z).length() < REACH_THRESHOLD)
}

/// CanMoveToLocatorWithFallback.
///
/// Both regions first try CanMoveToLocator: the locator's tile must exist and
/// a sample within the agent radius must be reachable. They differ after that:
///
/// * CN: CanMoveToLocatorByFixture scans a 3x3 pattern at plus or minus the
///   agent radius; any missing tile, or a tile held by another fixture, fails.
/// * JP: CanMoveToLocatorByFixture reads the locator's own tile only, and its
///   result is combined with CanReachLocatorByNavMeshPath, which samples
///   within twice MysekaiConstants.TILE_SCALE and tests reachability. The
///   source evaluates both operands; neither has a side effect, so a known
///   false operand decides the result.
///
/// An unknown tile is not proof either way and is returned as an error.
pub(crate) fn can_move_to_locator_with_fallback(
    navigation: &PlayerFixtureNavigation,
    region: NavMeshRegion,
    player: Vec3,
    locator: Vec3,
    target_uid: &str,
) -> Result<bool, PlayerFixturePreparationError> {
    if can_move_to_locator(navigation, player, locator)? {
        return Ok(true);
    }
    match region {
        NavMeshRegion::Cn => cn_tile_fallback(navigation, locator, target_uid),
        NavMeshRegion::Jp => jp_fallback(navigation, player, locator, target_uid),
    }
}

/// CanMoveToLocator(Transform, radius).
fn can_move_to_locator(
    navigation: &PlayerFixtureNavigation,
    player: Vec3,
    locator: Vec3,
) -> Result<bool, PlayerFixturePreparationError> {
    match navigation.world.tile_at(locator) {
        FixtureTileKnowledge::Unknown => {
            return Err(PlayerFixturePreparationError::Missing(
                "locator tile identity",
            ))
        }
        FixtureTileKnowledge::Missing => return Ok(false),
        _ => {}
    }
    let radius = navigation.agent_radius;
    let Some(hit) = navigation
        .world
        .sample_position(locator, radius)
        .filter(|hit| hit.is_finite() && hit.distance(locator) <= radius)
    else {
        return Ok(false);
    };
    can_navmesh_move(navigation, player, hit)
}

/// CN CanMoveToLocatorByFixture. Repeated positions may map to one tile;
/// the lookup is immutable, so reading it again gives the same answer.
fn cn_tile_fallback(
    navigation: &PlayerFixtureNavigation,
    locator: Vec3,
    target_uid: &str,
) -> Result<bool, PlayerFixturePreparationError> {
    use FixtureTileKnowledge::*;
    let radius = navigation.agent_radius;
    for x in [-1.0, 0.0, 1.0] {
        for z in [-1.0, 0.0, 1.0] {
            match navigation
                .world
                .tile_at(locator + Vec3::new(x * radius, 0.0, z * radius))
            {
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

/// JP CanMoveToLocatorByFixture and CanReachLocatorByNavMeshPath.
fn jp_fallback(
    navigation: &PlayerFixtureNavigation,
    player: Vec3,
    locator: Vec3,
    target_uid: &str,
) -> Result<bool, PlayerFixturePreparationError> {
    use FixtureTileKnowledge::*;
    let by_fixture = match navigation.world.tile_at(locator) {
        Missing => Some(false),
        Occupied(uid) if uid != target_uid => Some(false),
        Empty | Occupied(_) => Some(true),
        Unknown | ExistingUnknownOccupant => None,
    };
    if by_fixture == Some(false) {
        return Ok(false);
    }
    let reach = match navigation
        .world
        .sample_position(locator, 2.0 * TILE_SCALE)
        .filter(|hit| hit.is_finite())
    {
        Some(hit) => can_navmesh_move(navigation, player, hit)?,
        None => false,
    };
    match by_fixture {
        Some(_) => Ok(reach),
        None if !reach => Ok(false),
        None => Err(PlayerFixturePreparationError::Missing(
            "locator tile identity",
        )),
    }
}
