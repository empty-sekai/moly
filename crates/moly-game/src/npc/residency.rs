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
//! This host holds one site at a time. An NPC whose own site is not the
//! loaded site has no ground here, so it is suspended ([`Away`]): hidden, left
//! out of the objective machine, the agent step and the presenter calls, and
//! not placed on the loaded site. When its own site is loaded again it
//! resumes where it stood (the source never re-places an NPC on entering a
//! site); only its current objective is restarted, because the fixtures it
//! referred to were rebuilt with the site.
//!
//! Named gap: the source's AI loop has no site test. A home NPC's loop keeps
//! running on the unseen home while the player is away: its rest delay
//! elapses on the scaled clock, then it decides and walks, until a step waits
//! on the per-frame state machine (the tweet state's Done, for one), which
//! does not run off sites 0 to 3. That advance is driven by the scaled clock
//! and by the agent's own motion over the time away, not a bounded set of
//! steps at the moment the player leaves, so it is not replayed here; the
//! suspended NPC keeps the state it had.

use bevy::prelude::*;

/// `MysekaiSiteType`, in its declaration order: the value of each site type
/// is its index.
pub(crate) const SITE_TYPES: [&str; 9] = [
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
pub(crate) fn site_type_value(name: &str) -> Option<i32> {
    SITE_TYPES
        .iter()
        .position(|site| *site == name)
        .map(|index| index as i32)
}

/// The avatar store's gate on its per-frame NPC update: the current site type
/// is 0, 1, 2 or 3 (home or a floor). Any other site updates no NPC.
pub(crate) fn npc_update_runs(site_type: &str) -> bool {
    matches!(site_type_value(site_type), Some(0..=3))
}

/// The same gate read from the loaded site; no loaded site updates no NPC.
pub(crate) fn loaded_site_runs(site: Option<&crate::site::SiteActive>) -> bool {
    site.is_some_and(|site| npc_update_runs(&site.site_type))
}

/// The site an NPC is created on. The source creates the visitors through
/// the home gate, and an AI model's site type starts at 0 (home); a floor as
/// the first loaded site is a host entry the source does not have, and there
/// the roster is created on that floor.
pub(crate) fn spawn_site(loaded: &str) -> String {
    if npc_update_runs(loaded) {
        loaded.to_owned()
    } else {
        SITE_TYPES[0].to_owned()
    }
}

/// A member whose own site is not the loaded site: suspended while it is
/// away (see the module notes).
#[derive(Component, Debug)]
pub(crate) struct Away;

/// Marks members away from the loaded site. It runs every frame after the
/// state machine registers, so a member whose own site differs from the
/// loaded one is suspended from its first frame there. The return to the own
/// site is [`crate::npc::reseed`]'s (it restarts the objective on the rebuilt
/// site); a member is never marked back here.
#[allow(clippy::type_complexity)]
pub(crate) fn mark_away(
    mut commands: Commands,
    site: Option<Res<crate::site::SiteActive>>,
    mut npcs: Query<
        (
            Entity,
            &crate::npc::CharacterUnitId,
            &crate::npc::NpcActions,
            &mut Visibility,
            &mut crate::npc::RestLifecycle,
        ),
        (Without<Away>, Without<crate::player::PlayerControlled>),
    >,
) {
    let Some(site) = site.as_deref() else {
        return; // between two sites: nothing is loaded to compare with
    };
    for (entity, unit, actions, mut visibility, mut rest) in &mut npcs {
        if actions.site_type.is_empty() || actions.site_type == site.site_type {
            continue;
        }
        commands.entity(entity).insert(Away);
        *visibility = Visibility::Hidden;
        rest.leave_for_residency();
        info!(
            "[npc-residency] unit={} stays on {} (site type {:?}) while {} (site type {:?}) is loaded: suspended, not placed here; per-frame NPC update on this site: {}",
            unit.0,
            actions.site_type,
            site_type_value(&actions.site_type),
            site.site_type,
            site_type_value(&site.site_type),
            if npc_update_runs(&site.site_type) { "runs" } else { "off" },
        );
    }
}
