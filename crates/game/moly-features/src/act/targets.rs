//! Where targets are, read from the world each frame, and the companions'
//! places and command numbers: the primitives the actions of every domain
//! share. A target that is not in the world yet (the house before its site
//! has loaded, a character on another site) is an `Err` naming what is
//! missing; the step that asked waits on it until its timeout.

use bevy::prelude::*;

use crate::companion::{Companion, CompanionState, CompanionStatus};
use crate::fixture::FixtureRoot;
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::npc_objective::ObjectiveMind;

use super::{Target, TargetKind, Taken};

/// The player's world position.
pub fn player(world: &mut World) -> Result<Vec3, String> {
    world
        .query_filtered::<&GlobalTransform, With<crate::player::PlayerControlled>>()
        .single(world)
        .map(GlobalTransform::translation)
        .map_err(|_| "the player is not placed".to_owned())
}

/// The body of the roster character `unit` (not the player, not a cut-scene
/// stand-in).
pub fn npc(world: &mut World, unit: u32) -> Result<Entity, String> {
    Taken::body(world, unit)
}

/// The target's world position; x and z carry its offset.
pub fn resolve(world: &mut World, target: &Target) -> Result<Vec3, String> {
    let at = match &target.what {
        TargetKind::Point([x, z]) => Vec3::new(*x, player(world).map_or(0.0, |p| p.y), *z),
        TargetKind::Player => player(world)?,
        TargetKind::Npc(unit) => {
            let entity = npc(world, *unit)?;
            world
                .get::<GlobalTransform>(entity)
                .map(GlobalTransform::translation)
                .ok_or_else(|| format!("character {unit} has no transform"))?
        }
        TargetKind::HouseDoor => {
            let house = world
                .query::<(&crate::site_move::door::HouseEntryPoint, &GlobalTransform)>()
                .iter(world)
                .next()
                .map(|(point, house)| (point.1, house.compute_transform()));
            let (door, root) = house.ok_or("no placed house with an entry point on this site")?;
            let door = door
                .and_then(|door| world.get::<GlobalTransform>(door))
                .map(GlobalTransform::translation);
            crate::interaction::house_box_centre(&root, door)
        }
        TargetKind::RoomDoor => {
            let epoch = world.get_resource::<crate::site::GroundEpoch>().map(|epoch| epoch.0);
            let (sensor, _) = crate::site_move::room_door::sensor(
                world.get_resource::<crate::site_move::room_door::RoomDoor>(),
                epoch,
            )
            .ok_or("this site has no room door sensor")?;
            world
                .get::<GlobalTransform>(sensor)
                .map(GlobalTransform::translation)
                .ok_or("the room door sensor has no transform")?
        }
        TargetKind::FixtureUid(uid) => placed_fixtures(world)
            .into_iter()
            .find(|(_, identity, _)| identity.uid == *uid)
            .map(|(_, _, at)| at)
            .ok_or_else(|| format!("no placed fixture with UID {uid}"))?,
        TargetKind::FixtureId(id) => {
            let from = player(world).unwrap_or(Vec3::ZERO);
            placed_fixtures(world)
                .into_iter()
                .filter(|(_, identity, _)| identity.master_id == *id)
                .map(|(_, _, at)| at)
                .min_by(|a, b| a.distance_squared(from).total_cmp(&b.distance_squared(from)))
                .ok_or_else(|| format!("no placed fixture of master id {id}"))?
        }
        TargetKind::Gatherable(uid) => match world.get_resource::<Gatherables>() {
            Some(gatherables) => gatherables
                .0
                .iter()
                .find(|(at_uid, _)| at_uid == uid)
                .map(|(_, at)| *at)
                .ok_or_else(|| format!("no gatherable with UID {uid} on this site"))?,
            None => return Err("this site has no gatherables".into()),
        },
        TargetKind::Workbench => {
            let from = player(world).unwrap_or(Vec3::ZERO);
            let placed = placed_fixtures(world);
            let systems = world
                .get_resource::<crate::system_fixture::SystemFixtures>()
                .ok_or("the system fixture table is not set up")?;
            placed
                .into_iter()
                .filter(|(_, identity, _)| {
                    systems.of_master(identity.master_id) == Some(crate::system_fixture::SystemFixtureType::CraftTool)
                })
                .map(|(_, _, at)| at)
                .min_by(|a, b| a.distance_squared(from).total_cmp(&b.distance_squared(from)))
                .ok_or("no placed workbench (a craft tool fixture) on this site")?
        }
    };
    Ok(at + Vec3::new(target.offset[0], 0.0, target.offset[1]))
}

/// The target's world position, resolved once when the step starts: an
/// action that walks to a place it has already found takes no offset again.
pub fn place(world: &mut World, target: &Target) -> Result<Vec3, String> {
    resolve(world, target)
}

/// The placed fixtures of the site, with their identities and positions.
fn placed_fixtures(world: &mut World) -> Vec<(Entity, FixtureActivityIdentity, Vec3)> {
    let mut placed = world.query_filtered::<(Entity, &FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>();
    placed.iter(world).map(|(entity, identity, at)| (entity, identity.clone(), at.translation())).collect()
}

/// Horizontal distance.
pub fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// Where the site's gatherables stand, by UID: the domain that owns the
/// harvest objects reports them here at its install, so a target resolves
/// through the hub without the hub knowing them.
#[derive(Resource, Default)]
pub struct Gatherables(Vec<(u64, Vec3)>);

impl Gatherables {
    /// Reports the gatherables of the site, oldest first.
    pub fn set(&mut self, rows: Vec<(u64, Vec3)>) {
        self.0 = rows;
    }

    /// The gatherables of the site.
    pub fn rows(&self) -> &[(u64, Vec3)] {
        &self.0
    }
}

/// The placed fixture of `uid` on the site: its entity and where it stands.
pub fn placed_fixture(world: &mut World, uid: &str) -> Option<(Entity, GlobalTransform)> {
    let mut placed = world.query::<(Entity, &FixtureActivityIdentity, &GlobalTransform)>();
    placed
        .iter(world)
        .find(|(_, identity, _)| identity.uid == uid)
        .map(|(entity, _, at)| (entity, *at))
}

/// The body of `unit`'s companion.
pub fn companion_entity(world: &mut World, unit: u32) -> Option<Entity> {
    world
        .query::<(Entity, &Companion)>()
        .iter(world)
        .find(|(_, companion)| companion.unit == unit)
        .map(|(entity, _)| entity)
}

/// The transform of `unit`'s companion: its place and the yaw it faces.
pub fn companion_place(world: &mut World, unit: u32) -> Option<Transform> {
    world
        .query::<(&Companion, &Transform)>()
        .iter(world)
        .find(|(companion, _)| companion.unit == unit)
        .map(|(_, transform)| *transform)
}

/// The state of `unit`'s companion, with its command number.
pub fn companion_state(world: &mut World, unit: u32) -> Option<(CompanionState, CompanionStatus)> {
    world
        .query::<(&Companion, &CompanionStatus)>()
        .iter(world)
        .find(|(companion, _)| companion.unit == unit)
        .map(|(_, status)| (status.state.clone(), status.clone()))
}

/// The command number of `unit`'s companion: its spawn is the first.
pub fn companion_seq(world: &mut World, unit: u32) -> Option<u64> {
    companion_state(world, unit).map(|(_, status)| status.seq)
}

/// The body `companion` wears, when it is dressed.
pub fn worn_body(world: &mut World, companion: Entity) -> Option<Entity> {
    world
        .query::<(Entity, &crate::player_avatar::oc::OcModel)>()
        .iter(world)
        .find(|(_, model)| model.player == companion)
        .map(|(entity, _)| entity)
}

/// The units of the roster's characters on this site.
pub fn roster(world: &mut World) -> Vec<(Entity, u32)> {
    world
        .query_filtered::<(Entity, &crate::npc::CharacterUnitId), (
            With<ObjectiveMind>,
            Without<crate::player::PlayerControlled>,
            Without<crate::cut_scene::CutSceneAvatar>,
        )>()
        .iter(world)
        .map(|(entity, unit)| (entity, unit.0))
        .collect()
}
