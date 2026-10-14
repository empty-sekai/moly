//! Where targets are and whether conditions hold, read from the world each
//! frame. A target that is not in the world yet (the house before its site
//! has loaded, an NPC on another site) is an `Err` naming what is missing;
//! the step that asked waits on it until its timeout.

use bevy::prelude::*;

use super::script::{Cond, SiteKindName, Target, TargetKind};
use crate::fixture::FixtureRoot;
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::npc::CharacterUnitId;
use crate::npc_objective::ObjectiveMind;
use crate::player::PlayerControlled;

/// The player's world position.
pub(crate) fn player(world: &mut World) -> Result<Vec3, String> {
    world
        .query_filtered::<&GlobalTransform, With<PlayerControlled>>()
        .single(world)
        .map(GlobalTransform::translation)
        .map_err(|_| "the player is not placed".to_owned())
}

/// The roster NPC of `unit` (not the player, not a cut-scene stand-in).
pub(crate) fn npc(world: &mut World, unit: u32) -> Result<Entity, String> {
    world
        .query_filtered::<(Entity, &CharacterUnitId), (
            With<ObjectiveMind>,
            Without<PlayerControlled>,
            Without<crate::cutscene::CutSceneAvatar>,
        )>()
        .iter(world)
        .find(|(_, id)| id.0 == unit)
        .map(|(entity, _)| entity)
        .ok_or_else(|| format!("character {unit} is not on this site"))
}

/// The target's world position; x and z carry its offset.
pub(crate) fn resolve(world: &mut World, target: &Target) -> Result<Vec3, String> {
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
        TargetKind::FixtureUid(uid) => world
            .query_filtered::<(&FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>()
            .iter(world)
            .find(|(identity, _)| identity.uid == *uid)
            .map(|(_, at)| at.translation())
            .ok_or_else(|| format!("no placed fixture with UID {uid}"))?,
        TargetKind::FixtureId(id) => {
            let from = player(world).unwrap_or(Vec3::ZERO);
            world
                .query_filtered::<(&FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>()
                .iter(world)
                .filter(|(identity, _)| identity.master_id == *id)
                .map(|(_, at)| at.translation())
                .min_by(|a, b| a.distance_squared(from).total_cmp(&b.distance_squared(from)))
                .ok_or_else(|| format!("no placed fixture of master id {id}"))?
        }
    };
    Ok(at + Vec3::new(target.offset[0], 0.0, target.offset[1]))
}

/// Horizontal distance.
pub(crate) fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

fn site_type(world: &World) -> Option<String> {
    world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone())
}

fn kind_of(site_type: &str) -> Option<SiteKindName> {
    Some(match site_type {
        "home_site" => SiteKindName::Home,
        "first_floor" | "second_floor" | "third_floor" => SiteKindName::Room,
        "grassland" | "shore" | "flower_garden" | "memorial_place" => SiteKindName::Harvest,
        "festival_garden" => SiteKindName::Delivery,
        _ => return None,
    })
}

/// Whether `cond` holds, and what was read (for a timeout's report).
pub(crate) fn eval(world: &mut World, cond: &Cond) -> (bool, String) {
    match cond {
        Cond::All(list) => {
            let mut notes = Vec::new();
            for cond in list {
                let (holds, note) = eval(world, cond);
                notes.push(note);
                if !holds {
                    return (false, notes.join("; "));
                }
            }
            (true, notes.join("; "))
        }
        Cond::Any(list) => {
            let mut notes = Vec::new();
            for cond in list {
                let (holds, note) = eval(world, cond);
                notes.push(note);
                if holds {
                    return (true, notes.join("; "));
                }
            }
            (false, notes.join("; "))
        }
        Cond::Not(inner) => {
            let (holds, note) = eval(world, inner);
            (!holds, format!("not ({note})"))
        }
        Cond::SiteIs(want) => {
            let site = site_type(world);
            (site.as_deref() == Some(want.as_str()), format!("site {site:?}"))
        }
        Cond::SiteKind(want) => {
            let site = site_type(world);
            (site.as_deref().and_then(kind_of) == Some(*want), format!("site {site:?}"))
        }
        Cond::SiteMoving => {
            let moving = world.contains_resource::<crate::site_move::SiteMoveActive>();
            (moving, format!("site moving {moving}"))
        }
        Cond::EntryDone => {
            let open = crate::entry::control_open(world.get_resource::<crate::entry::EntrySequence>());
            (open, format!("player control {}", if open { "open" } else { "closed" }))
        }
        Cond::DialogShown => {
            let shown = world
                .get_resource::<crate::ui_layers::ScreenManager>()
                .is_some_and(crate::ui_layers::ScreenManager::exists_dialog);
            (shown, format!("dialog shown {shown}"))
        }
        Cond::CameraSettled => {
            let state = world.get_resource::<crate::camera::FieldCameraState>().map(|state| state.0);
            let tween = world.contains_resource::<crate::camera::CameraTween>();
            let settled = state.is_some_and(|state| state != crate::camera::CameraStateType::HouseEntry) && !tween;
            (settled, format!("camera state {state:?}, tween in flight {tween}"))
        }
        Cond::TalkActive => {
            let talk = world.contains_resource::<crate::talk::ActiveTalk>()
                || world.contains_resource::<crate::player_talk::PlayerTalkSession>();
            (talk, format!("talk active {talk}"))
        }
        Cond::ButtonHead(button) => {
            let head = world
                .get_resource::<crate::action_button::ActionButtonState>()
                .and_then(|state| state.current());
            (head.is_some_and(|(shown, _)| shown == *button), format!("button head {head:?}"))
        }
        Cond::ButtonStacked(button) => {
            let stacked = world
                .get_resource::<crate::action_button::ActionButtonState>()
                .is_some_and(|state| state.stacked(*button));
            (stacked, format!("{button:?} stacked {stacked}"))
        }
        Cond::PlayerNear { to, within } => near(world, None, to, *within),
        Cond::NpcNear { unit, to, within } => near(world, Some(*unit), to, *within),
        Cond::NpcAction { unit, is } => match npc(world, *unit) {
            Ok(entity) => {
                let current = world
                    .get::<crate::npc::NpcActions>(entity)
                    .map(|actions| format!("{:?}", actions.current));
                (current.as_deref() == Some(is.as_str()), format!("character {unit} action {current:?}"))
            }
            Err(error) => (false, error),
        },
        Cond::PhenomenonIs(want) => {
            let current = world
                .get_resource::<crate::weather::CurrentPhenomenonId>()
                .map(|id| id.0);
            (current == Some(*want), format!("phenomenon {current:?}"))
        }
    }
}

fn near(world: &mut World, unit: Option<u32>, to: &Target, within: f32) -> (bool, String) {
    let from = match unit {
        None => player(world),
        Some(unit) => resolve(world, &Target { what: TargetKind::Npc(unit), offset: [0.0; 2] }),
    };
    match (from, resolve(world, to)) {
        (Ok(from), Ok(to)) => {
            let distance = flat(from, to);
            (distance <= within, format!("distance {distance:.2} m (within {within} m)"))
        }
        (Err(error), _) | (_, Err(error)) => (false, error),
    }
}
