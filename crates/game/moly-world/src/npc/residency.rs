//! Residency: which site each NPC is on, and the site gate of the per-frame
//! NPC update.
//!
//! The source keeps every NPC in the avatar store for the whole visit. Each
//! AI model carries its own site type (the NPC's `CurrentSiteType`), written
//! when the NPC is placed and when it changes site; the move range, the talk
//! site filters and the change-site lottery read that field, never the
//! camera's site. The store's per-frame update runs the NPC presenters only
//! while the current site is home or a floor (site types 0 to 3); on the
//! harvest maps and the festival garden no NPC updates at all. A cannon or
//! door move neither disposes nor hides an NPC, so the home NPCs stay on home
//! while the player is elsewhere, and no NPC is ever on a harvest map.
//!
//! Render visibility is separate from simulation ownership. A hidden retained
//! home or floor keeps its navigation, furniture identities, reservations and
//! clocks. Only actual site removal ends those owners.

use bevy::prelude::*;
use std::collections::HashMap;

/// Navigation retained in each site's local frame while its view is hidden.
/// The walk and objective faces are kept together so they share a generation.
#[derive(Resource, Default)]
pub struct SiteNavigation(HashMap<String, Navigation>);

pub struct Navigation {
    pub walk: crate::walk_face::WalkFace,
    pub objective: crate::npc_objective::ObjectiveFace,
    pub door: Option<(Vec3, Vec3)>,
    pub house_exit: Option<crate::fixture_gimmick::house_door::NpcExitProgram>,
}

impl SiteNavigation {
    pub fn get(&self, site: &str) -> Option<&Navigation> {
        self.0.get(site)
    }

    pub(crate) fn insert(&mut self, site: String, navigation: Navigation) { self.0.insert(site, navigation); }

    pub fn forget(&mut self, site: &str) {
        self.0.remove(site);
    }
}

/// Before the current site's fixtures and navigation are detached, retain
/// the same baked faces and the door locator that the active agents use.
pub fn retain_navigation(world: &mut World) {
    let Some(site) = world.get_resource::<crate::site::SiteActive>() else {
        return;
    };
    let name = site.site_type.clone();
    if !npc_update_runs(&name) {
        return;
    }
    let (Some(walk), Some(objective)) = (
        world.get_resource::<crate::walk_face::WalkFace>().cloned(),
        world
            .get_resource::<crate::npc_objective::ObjectiveFace>()
            .cloned(),
    ) else {
        return;
    };
    if walk.generation() != objective.navigation_generation() {
        return;
    }
    let mut house_exit = None;
    let point = match site_type_value(&name) {
        Some(0) => match crate::fixture_gimmick::house_door::find_house(world) {
            crate::fixture_gimmick::house_door::HouseLookup::Found(house) => {
                house_exit = house.npc_exit_program().ok();
                house.inside_door
            }
            _ => None,
        },
        Some(1..=3) => crate::room_door_port::inside_point(world),
        _ => None,
    };
    let door = point
        .and_then(|point| world.get::<GlobalTransform>(point))
        .map(|transform| (transform.translation(), transform.rotation() * Vec3::Z));
    world.resource_mut::<SiteNavigation>().0.insert(
        name.clone(),
        Navigation {
            walk,
            objective,
            door,
            house_exit,
        },
    );
    info!("[npc-residency] retained navigation of {name}, door {door:?}");
}

/// `MysekaiSiteType`, in its declaration order: the value of each site type
/// is its index.
pub const SITE_TYPES: [&str; 9] = [
    "home_site",
    "first_floor",
    "second_floor",
    "third_floor",
    "grassland",
    "shore",
    "flower_garden",
    "memorial_place",
    "festival_garden",
];

/// The value of a site type name, `None` for a name outside the nine.
pub fn site_type_value(name: &str) -> Option<i32> {
    SITE_TYPES
        .iter()
        .position(|site| *site == name)
        .map(|index| index as i32)
}

/// The avatar store's gate on its per-frame NPC update: the current site type
/// is 0, 1, 2 or 3 (home or a floor). Any other site updates no NPC.
pub fn npc_update_runs(site_type: &str) -> bool {
    matches!(site_type_value(site_type), Some(0..=3))
}

/// The same gate read from the loaded site; no loaded site updates no NPC.
pub fn loaded_site_runs(site: Option<&crate::site::SiteActive>) -> bool {
    site.is_some_and(|site| npc_update_runs(&site.site_type))
}

/// The site an NPC is created on: home, whatever site is loaded first. The
/// gate creates the visitors, an AI model's site type starts at 0 (home), and
/// the gate's appearance places them on the home site model on every start,
/// a floor start included (it takes the home site before it places anyone).
/// Only the multiplayer instantiation of a guest's copy of an NPC sets
/// another site at creation (from the room's NPC site table); the gate path
/// never does. So on a floor start no visitor stands in the room: the whole
/// roster is on home, away from the loaded floor.
pub fn spawn_site(_loaded: &str) -> String {
    SITE_TYPES[0].to_owned()
}

/// A member whose own site is not the loaded site: suspended while it is
/// away (see the module notes).
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct Away;

/// Marks members away from the loaded site. It runs every frame after the
/// state machine registers, so a member whose own site differs from the
/// loaded one is suspended from its first frame there. The return to the own
/// site is [`crate::npc::reseed`]'s (it restarts the objective on the rebuilt
/// site); a member is never marked back here. A suspended member's balloon is
/// retired with it: the state machine skips away members, so no hide would
/// reach a balloon left over its hidden model.
#[allow(clippy::type_complexity)]
pub fn mark_away(
    mut commands: Commands,
    site: Option<Res<crate::site::SiteActive>>,
    mut hud: MessageWriter<crate::npc_state::TweetHudEvent>,
    mut npcs: Query<
        (
            Entity,
            &crate::npc::CharacterUnitId,
            &crate::npc::NpcActions,
            &mut Visibility,
            &mut crate::npc::RestLifecycle,
            Has<Away>,
            Has<crate::npc::gate_entries::ModelHidden>,
        ),
        Without<crate::player::PlayerControlled>,
    >,
) {
    let Some(site) = site.as_deref() else {
        return; // between two sites: nothing is loaded to compare with
    };
    for (entity, unit, actions, mut visibility, mut rest, away, model_hidden) in &mut npcs {
        if actions.site_type.is_empty() { continue; }
        if actions.site_type == site.site_type {
            if away {
                commands.entity(entity).remove::<Away>();
                if !model_hidden { visibility.set_if_neq(Visibility::Inherited); }
            }
            continue;
        }
        if away { continue; }
        commands.entity(entity).insert(Away);
        visibility.set_if_neq(Visibility::Hidden);
        hud.write(crate::npc_state::TweetHudEvent::Retire { npc: entity });
        // Hiding a retained site is not a Rest state transition.
        let _ = &mut rest;
        info!(
            "[npc-residency] unit={} stays on {} (site type {:?}) while {} (site type {:?}) is loaded: suspended, not placed here; per-frame NPC update on this site: {}",
            unit.0,
            actions.site_type,
            site_type_value(&actions.site_type),
            site.site_type,
            site_type_value(&site.site_type),
            if npc_update_runs(&site.site_type) {
                "runs"
            } else {
                "off"
            },
        );
    }
}
