//! A fixture put on the field by a character rather than through the
//! editor's screen.
//!
//! It goes through the editor's own steps with no editor session open: the
//! owned catalog row ([`owned::catalog`]), the put search around a world
//! point ([`super::put_search`], the spiral `PutFixture` runs around the
//! camera look-at), the save checks ([`validation::save`]), the housing
//! layout call, the saved copy written on its accepted reply and the layout
//! reload. A stored copy of the fixture is taken first, as the catalog row's
//! selection takes it.
//!
//! [`start`] answers the new row's uid at once, before the save, so a caller
//! can prepare how the instance is drawn before it is spawned; the build's
//! state then goes `Saving` → `Reloading` → `Placed` (the fixture is spawned
//! and the walk cells are baked with it), or `Refused` with the reason.

use super::{actors, layout_rules, owned, put_search, validation, EditPhase, EditSession, Facing, PutAim};
use crate::fixture::{EditableFixture, FixturePlacements};
use moly_law::fixture::Direction;
use crate::userdata::Answered;
use bevy::prelude::*;
use moly_link::ServerLink;
use moly_protocol::{Call, Refusal, Reply};
use std::collections::HashMap;

/// Where a field build stands.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldBuildState {
    /// The housing layout call is out.
    Saving,
    /// Saved; the layout is reloading and the walk cells rebaking.
    Reloading,
    /// The fixture `uid` is in the layout, spawned, with the walk cells
    /// baked around it.
    Placed { uid: String },
    Refused(String),
}

/// The field build in flight, from its call to its reload.
struct InFlight {
    ticket: u64,
    seq: u32,
    uid: String,
    next: FixturePlacements,
    returned: Vec<EditableFixture>,
    storage: crate::fixture::layouts::EditStorageSnapshot,
    reload: Option<actors::PendingReload>,
}

/// The field builds by ticket, and the one in flight.
#[derive(Resource, Default)]
pub struct FieldBuilds {
    next_ticket: u64,
    states: HashMap<u64, FieldBuildState>,
    in_flight: Option<InFlight>,
}

impl FieldBuilds {
    pub fn state(&self, ticket: u64) -> Option<&FieldBuildState> {
        self.states.get(&ticket)
    }

    fn settle(&mut self, ticket: u64, state: FieldBuildState) {
        match &state {
            FieldBuildState::Refused(reason) => warn!("[field-build] {ticket}: refused: {reason}"),
            state => info!("[field-build] {ticket}: {state:?}"),
        }
        self.states.insert(ticket, state);
    }
}

/// Where and how a field build puts its fixture.
#[derive(Clone, Copy, Debug)]
pub struct FieldPut {
    /// A world point: the put search starts at the tile nearest it, or, when
    /// `exact`, the fixture goes on the cell it falls in or the put is
    /// refused.
    pub at: Vec3,
    pub exact: bool,
    /// The direction number the put gets (0 front, 1 left, 2 back, 3 right,
    /// the number a camera yaw of 90 × n degrees gives); the camera's yaw
    /// decides it when absent.
    pub direction: Option<Direction>,
}

/// Is a field build's layout save in flight? Only one save is out at a time:
/// [`start`] refuses while it is.
pub fn saving(world: &World) -> bool {
    world.get_resource::<FieldBuilds>().is_some_and(|builds| builds.in_flight.is_some())
}

/// The cell and direction a placed fixture was saved with.
pub fn placed_cell(world: &World, uid: &str) -> Option<(moly_law::fixture::GridPosition, Direction)> {
    world
        .get_resource::<FixturePlacements>()?
        .editor_rows()
        .into_iter()
        .find(|row| row.uid == uid)
        .map(|row| (row.center, row.direction))
}

/// Puts an owned copy of `fixture_id` (of `texture`, or any) where `put`
/// says, and sends the layout with it. `Ok` is the build's ticket and the
/// new row's uid.
pub fn start(
    world: &mut World,
    fixture_id: i32,
    texture: Option<u32>,
    put: FieldPut,
) -> Result<(u64, String), String> {
    let FieldPut { at: near, exact, direction } = put;
    if saving(world) {
        return Err("another field build is still saving".into());
    }
    if world.get_resource::<EditSession>().is_some_and(|session| session.phase != EditPhase::Idle) {
        return Err("the layout editor is open".into());
    }
    let Some(layout) = world.get_resource::<FixturePlacements>().cloned() else {
        return Err("the site layout is not installed".into());
    };
    let Some(floor) = layout.floor_grid() else {
        return Err("the site's floor grid is not ready".into());
    };
    if layout.site_id() == 0
        || !world.contains_resource::<crate::fixture::FixtureScenesReady>()
        || (layout.total() != 0 && !world.contains_resource::<crate::fixture_material::FixtureMaterialsSwapped>())
    {
        return Err("the site's fixtures are still loading".into());
    }
    if !world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == layout.site_type())
    {
        return Err("the site is changing".into());
    }
    let homes = world.get_resource::<crate::house::HomeFixtures>();
    let (storage, mut returned) = world
        .get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
        .ok_or_else(|| "the saved layouts are not ready".to_owned())
        .and_then(|layouts| layouts.begin_edit(&layout, homes))?;
    let site_id = layout.site_id();
    let mut rows = layout.editor_rows();
    let (catalog, _) = owned::catalog(world, site_id, &rows)?;
    let Some(row) = catalog.into_iter().find(|row| {
        row.fixture_id == fixture_id && texture.is_none_or(|texture| row.texture_id == texture) && row.placeable() > 0
    }) else {
        return Err(format!("no placeable copy of fixture {fixture_id} is owned"));
    };
    let mut next_uid = layout.next_edit_uid();
    let stored = returned
        .iter()
        .position(|item| item.fixture_id == row.fixture_id && item.texture_id == row.texture_id);
    let mut item = match stored {
        Some(index) => {
            let mut item = returned.remove(index);
            item.center = moly_law::fixture::GridPosition::ZERO;
            item
        }
        None => {
            let uid = format!("offline-edit-{site_id}-{next_uid}");
            if rows.iter().chain(&returned).any(|item| item.uid == uid) {
                return Err(format!("the offline uid {uid} is taken"));
            }
            next_uid = next_uid.checked_add(1).ok_or("the offline uids are used up")?;
            row.fixture(uid)
        }
    };
    let facing = match direction {
        Some(direction) => Facing::Fixed(direction),
        None => {
            let Some(yaw) = world
                .get_resource::<crate::camera::FieldCameraModel>()
                .map(|model| model.yaw)
            else {
                return Err("the camera is not ready".into());
            };
            Facing::Yaw(yaw)
        }
    };
    let aim = PutAim { facing, exact };
    put_search(world, &rows, floor, &mut item, near, aim, "field")?;
    rows.push(item.clone());
    let rules = layout_rules::rules(world, Some(floor));
    let Some(areas) = world.get_resource::<super::FixtureAreas>() else {
        return Err("the fixture areas are not read".into());
    };
    validation::save(&rows, Some(floor), areas, rules.as_ref().map_err(|missing| *missing), true)
        .map_err(|(_, error)| error)?;
    let next = layout.with_editor_rows(&rows, next_uid)?;
    let call = Call::HousingLayout(crate::fixture::layouts::housing_layout_request(&next));
    let seq = world.resource_mut::<ServerLink>().call(call);
    let mut builds = world.get_resource_or_insert_with(FieldBuilds::default);
    builds.next_ticket += 1;
    let ticket = builds.next_ticket;
    info!(
        "[field-build] {ticket}: fixture {fixture_id} as {} at center ({}, {}, {}) {:?} near {near:?}; PostUserMysekaiHousingLayoutApi sent as seq {seq}",
        item.uid, item.center.x, item.center.y, item.center.z, item.direction
    );
    builds.settle(ticket, FieldBuildState::Saving);
    builds.in_flight = Some(InFlight {
        ticket,
        seq,
        uid: item.uid.clone(),
        next,
        returned,
        storage,
        reload: None,
    });
    Ok((ticket, item.uid))
}

/// Update: the housing layout reply of the field build in flight.
pub(super) fn read_answers(
    mut answers: MessageReader<Answered>,
    builds: Option<Res<FieldBuilds>>,
    mut commands: Commands,
) {
    let Some(seq) = builds.as_deref().and_then(|builds| builds.in_flight.as_ref()).map(|flight| flight.seq) else {
        return;
    };
    for answer in answers.read() {
        if answer.seq != seq {
            continue;
        }
        let outcome = answer.outcome.clone();
        commands.queue(move |world: &mut World| finish_save(world, outcome));
    }
}

/// An accepted layout is written and published, and the layout reloads;
/// a refusal ends the build.
fn finish_save(world: &mut World, outcome: Result<Reply, Refusal>) {
    let Some(mut builds) = world.remove_resource::<FieldBuilds>() else {
        return;
    };
    let Some(mut flight) = builds.in_flight.take() else {
        world.insert_resource(builds);
        return;
    };
    let refusal = match outcome {
        Ok(Reply::SuiteUser(reply)) if reply.success => None,
        Ok(Reply::SuiteUser(reply)) => Some(format!("the server refused the layout ({})", reply.refusal.unwrap_or_default())),
        Err(refusal) => Some(format!("the server refused the layout ({})", refusal.reason)),
        Ok(other) => Some(format!("the server answered {other:?}")),
    };
    if let Some(reason) = refusal {
        builds.settle(flight.ticket, FieldBuildState::Refused(reason));
        world.insert_resource(builds);
        return;
    }
    let receipt = match crate::fixture::layouts::persist_edit(&flight.next, &flight.returned, &flight.storage) {
        Ok(receipt) => receipt,
        Err(error) => {
            builds.settle(flight.ticket, FieldBuildState::Refused(format!("the saved copy was not written: {error}")));
            world.insert_resource(builds);
            return;
        }
    };
    world.insert_resource(flight.next.clone());
    if let Some(mut layouts) = world.get_resource_mut::<crate::fixture::layouts::SiteFixtureLayouts>() {
        layouts.did_save(&flight.next, receipt);
    }
    crate::fixture::reload_after_save(world);
    world.write_message(super::LayoutSaved);
    flight.reload = Some(actors::wait_for_reload(world, true, false));
    builds.settle(flight.ticket, FieldBuildState::Reloading);
    builds.in_flight = Some(flight);
    world.insert_resource(builds);
}

/// Update: the reload of a saved field build, until its fixture is spawned
/// and the walk cells are baked with it.
pub(super) fn advance(world: &mut World) {
    let Some(reload) = world
        .get_resource::<FieldBuilds>()
        .and_then(|builds| builds.in_flight.as_ref())
        .and_then(|flight| flight.reload.clone())
    else {
        return;
    };
    let state = match actors::reload_ready(world, &reload) {
        Ok(false) => return,
        Ok(true) => None,
        Err(error) => Some(error),
    };
    let mut builds = world.resource_mut::<FieldBuilds>();
    let Some(flight) = builds.in_flight.take() else {
        return;
    };
    let state = match state {
        None => FieldBuildState::Placed { uid: flight.uid },
        Some(error) => FieldBuildState::Refused(error),
    };
    builds.settle(flight.ticket, state);
}
