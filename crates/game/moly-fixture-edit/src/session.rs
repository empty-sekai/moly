//! The editor's draft for callers that run without the editor's screen: the
//! site's grid and rules, a draft opened from the loaded layout, and the
//! editor's save checks.
//!
//! Input: the site's loaded layout, the site's grid, the fixture tables and
//! the fixture areas. Output: a [Draft] that is the editor's own session
//! ([`EditSession`]) and its decided rows, the site's grid and rules
//! ([`rules`]) and the check that a decided layout passes before it is sent
//! ([`check_save`]). Invariants: a draft made here is the draft the editor's
//! screen makes, with the same put rules, the same save checks, the same save
//! call and the same returned inventory. Boundary: the draft belongs to the
//! caller; it runs with no editor game state, no hidden actors and no editor
//! screen, and the fixtures and walk cells come from the normal reload after
//! the save. A draft may be opened only when the site is loaded, the site is
//! not changing, no editor holds a draft and no layout save is out.

use bevy::ecs::world::World;

use super::{layout_rules, EditSession, EditPhase};
use moly_features::fixture::FixturePlacements;
use moly_features::site::{FloorGridLayout, SiteActive};

/// A draft a caller holds: its own, never the editor's.
pub(crate) struct Draft(EditSession);

impl Draft {
    /// The draft as the site layout a save writes, and the inventory it
    /// returns.
    pub(crate) fn decided(
        &mut self,
    ) -> Result<(FixturePlacements, Vec<moly_features::fixture::EditableFixture>), String> {
        self.0.decided_layout()
    }
}

/// Whether a draft may be opened, or why not.
pub(crate) fn ready(world: &World) -> Result<(), String> {
    if world
        .get_resource::<EditSession>()
        .is_some_and(|session| session.phase != EditPhase::Idle)
    {
        return Err("the layout editor is open".into());
    }
    let Some(layout) = world.get_resource::<FixturePlacements>() else {
        return Err("the site layout is not installed".into());
    };
    if layout.site_id() == 0
        || !world.contains_resource::<moly_features::fixture::FixtureScenesReady>()
        || (layout.total() != 0
            && !world.contains_resource::<moly_features::fixture_material::FixtureMaterialsSwapped>())
    {
        return Err("the site's fixtures are still loading".into());
    }
    if !world
        .get_resource::<SiteActive>()
        .is_some_and(|site| site.site_type == layout.site_type())
    {
        return Err("the site is changing".into());
    }
    if super::field_build::saving(world) {
        return Err("another layout save is in flight".into());
    }
    if crate::layout_director::saving(world) {
        return Err("another layout save is in flight".into());
    }
    Ok(())
}

/// The site's floor grid and the rules the put checks read.
pub(crate) fn rules(world: &World) -> Result<(FloorGridLayout, moly_layout::Rules), String> {
    let layout = world
        .get_resource::<FixturePlacements>()
        .ok_or_else(|| "the site layout is not installed".to_owned())?;
    let floor = layout
        .floor_grid()
        .ok_or_else(|| "the site's floor grid is not ready".to_owned())?;
    let rules = layout_rules::rules(world, Some(floor))
        .map_err(|missing| format!("layout rules unavailable: {missing:?}"))?;
    Ok((floor, rules))
}

/// The draft opened from the site's loaded layout, as the editor's screen
/// opens it, without entering the editor's game state.
pub(crate) fn open(world: &mut World) -> Result<Draft, String> {
    ready(world)?;
    let Some(layout) = world.get_resource::<FixturePlacements>().cloned() else {
        return Err("the site layout is not installed".into());
    };
    let homes = world.get_resource::<moly_features::house::HomeFixtures>();
    let entry = world
        .get_resource::<moly_features::fixture::layouts::SiteFixtureLayouts>()
        .ok_or_else(|| "the saved layouts are not ready".to_owned())
        .and_then(|layouts| layouts.begin_edit(&layout, homes));
    let (storage_baseline, inventory) = match entry {
        Ok(entry) => entry,
        Err(error) => return Err(format!("the site's saved layout cannot be opened: {error}")),
    };
    let mut session = EditSession::default();
    session.open_for_layout(layout, storage_baseline, inventory);
    Ok(Draft(session))
}

/// Applies one transaction to the draft; the layout's own refusal is the
/// draft's outcome, not an error.
pub(crate) fn apply(
    world: &mut World,
    draft: &mut Draft,
    request: &moly_layout::Apply,
) -> Result<moly_layout::ApplyOutcome, String> {
    super::apply_in_session(world, &mut draft.0, request)
}

/// The site's rows as the editor's draft holds them.
pub(crate) fn rows(world: &World) -> Vec<moly_features::fixture::EditableFixture> {
    world
        .get_resource::<FixturePlacements>()
        .map_or_else(Vec::new, FixturePlacements::editor_rows)
}

/// The editor's save checks on a decided layout, against the site's fixture
/// areas: what a save is held to before it is sent.
pub(crate) fn check_save(
    world: &World,
    rows: &[moly_features::fixture::EditableFixture],
    floor: FloorGridLayout,
    rules: &moly_layout::Rules,
) -> Result<(), String> {
    let Some(areas) = world.get_resource::<super::assets::FixtureAreas>() else {
        return Err("the fixture areas are not read".into());
    };
    super::validation::check_step_save(rows, Some(floor), areas, rules)
}
