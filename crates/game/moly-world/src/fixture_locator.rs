//! `CanMoveToLocatorWithFallback` over the placed fixtures' navigation: the
//! locator reach test both furniture buttons and the player's fixture action
//! share. Its inputs are the world's own navigation types, so it lives with
//! them; the buttons' remaining admission rules stay in
//! `crate::action_button::admission`.

use bevy::prelude::*;
use moly_law::{carve::NavMeshRegion, objective::TILE_SCALE};

use crate::player_fixture_action::{
    FixtureTileKnowledge, PlayerFixtureNavigation, PlayerFixturePreparationError,
};

/// The threshold every CanNavmeshMoveTargetPosition call on these paths passes.
pub const REACH_THRESHOLD: f32 = 0.01;

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
pub fn can_move_to_locator_with_fallback(
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

pub fn path_length(path: &[Vec3]) -> Option<f32> {
    if path.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let length: f32 = path.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
    length.is_finite().then_some(length)
}

/// CanNavmeshMoveTargetPosition: the query's boolean, then the last corner's
/// strictly-less horizontal distance. Height and path status are not read.
pub fn can_navmesh_move(
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
