//! Road planning owns a temporary projection until Decide. Saving remains the
//! editor's existing server/local transaction, never a parallel persistence path.
use super::{EditCommand, EditContentType, EditSession, EditableFixture, item_view, owned};
use bevy::prelude::*;
use moly_law::fixture::{Direction, GridPosition, position::layout_type};
use moly_layout::validate::{Board, Piece, Rules};

mod pick;

#[derive(Default)]
pub(super) struct RoadSession {
    target: Option<owned::OwnedRow>,
    /// A returned-item cell names this exact instance, even when its type is
    /// also present in the server-owned catalogue.
    target_uid: Option<String>,
    pub planned: Vec<EditableFixture>,
    original: Vec<EditableFixture>,
    history: Vec<GridPosition>,
    drag_origin: Option<GridPosition>,
    drag_rows: Vec<EditableFixture>,
    multi: Option<(GridPosition, GridPosition)>,
    pub dragging: bool,
    can_decide: bool,
    pub(super) select_mode: bool,
    pub(super) hide_fixtures: bool,
}

impl RoadSession {
    pub(super) fn cancel_pointer(&mut self) -> bool {
        let changed = self.dragging || self.multi.is_some() || self.drag_origin.is_some();
        self.dragging = false;
        self.multi = None;
        self.drag_origin = None;
        self.drag_rows.clear();
        changed
    }

    pub(super) fn cancel(&mut self) -> bool {
        let changed = !self.planned.is_empty() || !self.history.is_empty() || self.dragging;
        self.planned.clear();
        self.original.clear();
        self.history.clear();
        self.dragging = false;
        self.can_decide = false;
        self.multi = None;
        self.drag_origin = None;
        self.drag_rows.clear();
        changed
    }
}

pub(super) fn install(app: &mut App) {
    pick::install(app);
}

fn rules(world: &World, session: &EditSession) -> Result<Rules, String> {
    super::layout_rules::rules(
        world,
        session
            .baseline
            .as_ref()
            .and_then(super::FixturePlacements::floor_grid),
    )
    .map_err(|error| format!("road placement inputs unavailable: {error:?}"))
}

fn board(world: &World, session: &EditSession) -> Result<(Board, Rules), String> {
    let rules = rules(world, session)?;
    let floor = session
        .baseline
        .as_ref()
        .and_then(super::FixturePlacements::floor_grid)
        .ok_or("road grid unavailable")?;
    let rows: Vec<_> = session
        .rows
        .iter()
        .filter(|row| !session.road.original.iter().any(|old| old.uid == row.uid))
        .cloned()
        .collect();
    let board =
        Board::new(&rows, floor, &rules).map_err(|error| format!("road grid: {error:?}"))?;
    Ok((board, rules))
}

fn available(
    world: &World,
    session: &EditSession,
    target: &owned::OwnedRow,
) -> Result<i32, String> {
    let base = session
        .baseline
        .as_ref()
        .ok_or("road session has no baseline")?;
    if let Some(uid) = &session.road.target_uid {
        if !session.inventory.iter().any(|row| {
            row.uid == *uid
                && row.fixture_id == target.fixture_id
                && row.texture_id == target.texture_id
        }) {
            return Err("selected returned road UID is no longer available".into());
        }
    }
    let returned = session
        .inventory
        .iter()
        .filter(|row| {
            row.layout == layout_type::ROAD
                && row.fixture_id == target.fixture_id
                && row.texture_id == target.texture_id
        })
        .count();
    let returned = i32::try_from(returned).map_err(|_| "returned road count exceeds i32")?;
    let owned = match owned::catalog_for(world, base.site_id(), &session.rows, true) {
        Ok((catalog, _)) => catalog
            .iter()
            .find(|row| row.fixture_id == target.fixture_id && row.texture_id == target.texture_id)
            .map_or(0, owned::OwnedRow::placeable),
        // Returned placements are already durable local ownership; a missing
        // server catalogue must not erase them or fabricate extra instances.
        Err(_) if returned > 0 => 0,
        Err(error) => return Err(error),
    };
    let count = available_count(owned, returned);
    crate::fixture_put_limit::max_placeable(
        world,
        base.site_type(),
        &session.rows,
        target.fixture_id,
        target.texture_id,
        count,
    )
}

fn available_count(owned: i32, returned: i32) -> i32 {
    // Returned rows are included in owned placeable quantity. They are not
    // additional copies. Locally imported layouts may also have returned
    // instances not represented by this server's current catalogue.
    owned.max(returned).max(0)
}

fn select_inventory(session: &mut EditSession, uid: &str) -> Result<(), String> {
    let row = session
        .inventory
        .iter()
        .find(|row| row.uid == uid && row.layout == layout_type::ROAD)
        .ok_or("selected returned road UID is no longer available")?;
    let target = owned::OwnedRow {
        fixture_id: row.fixture_id,
        texture_id: row.texture_id,
        package: row.package.clone(),
        grid_size: row.grid_size,
        layout: row.layout,
        quantity: 0,
        layouted: 0,
    };
    session.road.cancel();
    session.road.target = Some(target);
    session.road.target_uid = Some(uid.to_owned());
    session.road.select_mode = false;
    session.changed();
    Ok(())
}

fn source_item(
    target: &owned::OwnedRow,
    center: GridPosition,
    uid: String,
) -> Result<EditableFixture, String> {
    let mut row = target.fixture(uid);
    (row.center, row.direction, row.layout) = moly_layout::validate::mirror_fixture_layout(
        center,
        row.grid_size,
        Direction::Front,
        layout_type::ROAD,
    )?;
    Ok(row)
}

fn plan(
    world: &World,
    session: &mut EditSession,
    candidates: &[GridPosition],
) -> Result<(), String> {
    let target = session
        .road
        .target
        .as_ref()
        .ok_or("choose a road first")?
        .clone();
    let limit = available(world, session, &target)?;
    let (board, rules) = board(world, session)?;
    let mut pieces = Vec::new();
    let mut planned = Vec::new();
    for &center in candidates {
        let row = source_item(&target, center, format!("road-preview-{}", planned.len()))?;
        let piece = Piece::new(&row, &rules).map_err(|e| format!("road candidate: {e:?}"))?;
        if !board.can_put_road(&piece, &pieces, None) || board.road_fully_covered(&piece, None) {
            continue;
        }
        if planned.len() >= limit.max(0) as usize {
            break;
        }
        pieces.push(piece);
        planned.push(row);
    }
    session.road.planned = planned;
    session.road.can_decide = !session.road.planned.is_empty();
    session.changed();
    Ok(())
}

fn tap(world: &World, session: &mut EditSession, center: GridPosition) -> Result<(), String> {
    if session.road.select_mode {
        if session.road.original.is_empty() {
            return Ok(());
        }
        return move_to(world, session, center);
    }
    if session.road.target.is_none() {
        return Ok(());
    }
    if session.road.history.is_empty() {
        plan(world, session, &[center])?;
        if !session.road.planned.is_empty() {
            session.road.history.push(center);
        }
    } else {
        let first = session.road.history[0];
        let candidates = moly_law::fixture::road_edit::tap_candidates(first, center)?;
        plan(world, session, &candidates)?;
        session.road.history.push(center);
    }
    Ok(())
}

fn select_uid(session: &mut EditSession, uid: &str) -> Result<(), String> {
    let Some(row) = session
        .rows
        .iter()
        .find(|row| row.uid == uid && row.layout == layout_type::ROAD)
        .cloned()
    else {
        return Ok(());
    };
    session.road.cancel();
    session.road.target = None;
    session.road.target_uid = None;
    session.road.original = vec![row.clone()];
    session.road.planned = vec![row];
    session.road.can_decide = true;
    session.changed();
    Ok(())
}

fn finish_multi(world: &World, session: &mut EditSession) -> Result<(), String> {
    let Some((start, end)) = session.road.multi.take() else {
        return Ok(());
    };
    session.road.dragging = false;
    let (board, _) = board(world, session)?;
    let mut ids = Vec::new();
    for x in start.x.min(end.x)..=start.x.max(end.x) {
        for z in start.z.min(end.z)..=start.z.max(end.z) {
            for owner in board.joint_occupants(layout_type::ROAD, GridPosition::new(x, 0, z)) {
                if !ids.contains(&owner.uid) {
                    ids.push(owner.uid.clone());
                }
            }
        }
    }
    session.road.original = ids
        .iter()
        .filter_map(|uid| session.rows.iter().find(|row| row.uid == *uid))
        .cloned()
        .collect();
    session.road.planned = session.road.original.clone();
    session.road.can_decide = !session.road.planned.is_empty();
    session.changed();
    Ok(())
}

fn move_to(world: &World, session: &mut EditSession, center: GridPosition) -> Result<(), String> {
    let (board, rules) = board(world, session)?;
    let source_rows = if session.road.drag_origin.is_some() {
        &session.road.drag_rows
    } else {
        &session.road.planned
    };
    if source_rows.is_empty() {
        return Ok(());
    }
    let pivot = if let Some(start) = session.road.drag_origin {
        start
    } else {
        let positions: Vec<_> = source_rows
            .iter()
            .filter_map(|row| row.pose().ok())
            .map(|pose| pose.translation)
            .collect();
        let average = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
        moly_law::fixture::road_edit::field_hit_grid([-average.x, average.y, average.z], [0.25; 3])
            .ok_or("road selection pivot outside grid")?
    };
    let delta = center - pivot;
    let mut rows = Vec::new();
    let mut valid = true;
    for row in source_rows {
        let source = Piece::new(row, &rules).map_err(|e| format!("road selection: {e:?}"))?;
        let mut moved = row.clone();
        (moved.center, moved.direction, moved.layout) =
            moly_layout::validate::mirror_fixture_layout(
                source.center + delta,
                row.grid_size,
                source.direction,
                row.layout,
            )?;
        let piece = Piece::new(&moved, &rules).map_err(|e| format!("road selection: {e:?}"))?;
        valid &= board.can_move_road(&piece);
        rows.push(moved);
    }
    session.road.planned = rows;
    session.road.can_decide = valid;
    session.changed();
    Ok(())
}

/// Build all three draft components together. A UID, geometry or budget
/// error leaves rows, stored placements and the UID cursor unchanged.
struct DecidedRoads {
    rows: Vec<EditableFixture>,
    inventory: Vec<EditableFixture>,
    next_uid: u64,
    created: Vec<EditableFixture>,
}

fn decide_rows(
    session: &EditSession,
    rules: &Rules,
    floor: crate::site::FloorGridLayout,
    site_id: u32,
) -> Result<DecidedRoads, String> {
    let moving = !session.road.original.is_empty();
    let mut rows: Vec<_> = session
        .rows
        .iter()
        .filter(|row| !session.road.original.iter().any(|old| old.uid == row.uid))
        .cloned()
        .collect();
    let mut inventory = session.inventory.clone();
    let mut next_uid = session.next_fixture_uid;
    let mut identities = std::collections::HashSet::new();
    for row in session.rows.iter().chain(&inventory) {
        if row.uid.is_empty() || !identities.insert(row.uid.clone()) {
            return Err("road draft contains duplicate or empty fixture UID".into());
        }
    }
    let mut board = Board::new(&rows, floor, rules).map_err(|e| format!("road grid: {e:?}"))?;
    let mut planned_pieces = Vec::new();
    let mut created = Vec::new();
    let mut selected_uid = session.road.target_uid.as_deref();
    for mut row in session.road.planned.clone() {
        let piece = Piece::new(&row, rules).map_err(|e| format!("road decide: {e:?}"))?;
        if moving {
            if !session.road.original.iter().any(|old| old.uid == row.uid)
                || !board.can_move_road(&piece)
            {
                return Err("road move destination is no longer valid".into());
            }
        } else if !board.can_put_road(&piece, &planned_pieces, None)
            || board.road_fully_covered(&piece, None)
        {
            return Err("road plan destination is no longer valid".into());
        }
        if !moving {
            let stored_index = if let Some(uid) = selected_uid.take() {
                Some(
                    inventory
                        .iter()
                        .position(|old| {
                            old.uid == uid
                                && old.fixture_id == row.fixture_id
                                && old.texture_id == row.texture_id
                        })
                        .ok_or("selected returned road UID is no longer available")?,
                )
            } else {
                inventory.iter().position(|old| {
                    old.fixture_id == row.fixture_id && old.texture_id == row.texture_id
                })
            };
            if let Some(index) = stored_index {
                row.uid = inventory.remove(index).uid;
            } else {
                let serial = next_uid;
                next_uid = serial.checked_add(1).ok_or("offline road UID exhausted")?;
                row.uid = format!("offline-edit-{site_id}-{serial}");
                if !identities.insert(row.uid.clone()) {
                    return Err("offline road UID conflicts with an existing placement".into());
                }
            }
        }
        // CreateRemoveTargetList reads joint owners, preserving partial
        // overlaps of the same fixture id and removing exact-origin copies.
        let mut remove = Vec::new();
        for x in piece.min.x..=piece.max.x {
            for z in piece.min.z..=piece.max.z {
                for other in board.joint_occupants(layout_type::ROAD, GridPosition::new(x, 0, z)) {
                    if other.uid != row.uid
                        && (other.fixture_id != row.fixture_id || other.center == piece.min)
                        && !remove.contains(&other.uid)
                    {
                        remove.push(other.uid.clone());
                    }
                }
            }
        }
        rows.retain(|old| {
            if remove.contains(&old.uid) {
                inventory.push(old.clone());
                false
            } else {
                true
            }
        });
        planned_pieces.push(Piece::new(&row, rules).map_err(|e| format!("road decide: {e:?}"))?);
        created.push(row.clone());
        rows.push(row);
        board = Board::new(&rows, floor, rules).map_err(|e| format!("road grid: {e:?}"))?;
    }
    // Source RemoveOverlapRoad applies its UID-aware overload in placed-road
    // encounter order. Matching candidate UID refuses the overlap verdict.
    let placed: Vec<_> = session
        .road
        .planned
        .iter()
        .map(|row| row.uid.clone())
        .collect();
    if moving {
        for uid in placed {
            let Some(piece) = board.piece(&uid) else {
                continue;
            };
            if board.road_fully_covered(piece, Some(&uid)) {
                rows.retain(|row| {
                    if row.uid == uid {
                        inventory.push(row.clone());
                        false
                    } else {
                        true
                    }
                });
                board = Board::new(&rows, floor, rules).map_err(|e| format!("road grid: {e:?}"))?;
            }
        }
    }
    for row in &rows {
        if row.layout == layout_type::ROAD {
            let piece = board
                .piece(&row.uid)
                .ok_or("road placement missing from final grid")?;
            if !board.holds_own_center(piece) {
                return Err("road UID does not own its center tile".into());
            }
        }
    }
    Ok(DecidedRoads {
        rows,
        inventory,
        next_uid,
        created,
    })
}

fn decide(world: &mut World, session: &mut EditSession) -> Result<(), String> {
    if session.road.planned.is_empty() || !session.road.can_decide {
        return Ok(());
    }
    let moving = !session.road.original.is_empty();
    if !moving {
        let target = session
            .road
            .target
            .as_ref()
            .ok_or("road plan has no selected fixture")?;
        if session.road.planned.len() > available(world, session, target)?.max(0) as usize {
            return Err(
                "road inventory or put cost changed; revise the plan before deciding".into(),
            );
        }
    }
    let rules = rules(world, session)?;
    let base = session.baseline.as_ref().ok_or("road baseline missing")?;
    let result = decide_rows(
        session,
        &rules,
        base.floor_grid().ok_or("road grid unavailable")?,
        base.site_id(),
    )?;
    let budget = crate::fixture_put_limit::budget(world, base.site_type(), &result.rows)?;
    if budget.remaining < 0 {
        return Err("road plan exceeds the current placement limit".into());
    }
    session.rows = result.rows;
    session.inventory = result.inventory;
    session.next_fixture_uid = result.next_uid;
    session.road.cancel();
    session.road.target_uid = None;
    if moving {
        session.road.select_mode = true;
    }
    session.changed();
    if !moving {
        super::play_se(
            world,
            "se_housing_finish",
            "RoadEditPlanningState.CreateRoad",
        );
        let origin = super::site_origin(world);
        let data: Vec<_> = result
            .created
            .iter()
            .map(|row| {
                let piece = Piece::new(row, &rules).expect("decided road was validated");
                (piece.center, piece.current_size())
            })
            .collect();
        for (index, (range, size)) in moly_law::fixture::road_edit::effect_segments(&data)
            .into_iter()
            .enumerate()
        {
            let rows = &result.created[range];
            let position = rows
                .iter()
                .map(|row| row.pose().expect("decided road pose").translation)
                .sum::<Vec3>()
                / rows.len() as f32;
            super::put_effect::show_field_placement(
                world,
                rows[0].fixture_id,
                origin + position,
                size,
                index != 0,
                "RoadEditPlanningState.CreateRoad",
            );
        }
    }
    Ok(())
}

pub(super) fn handle(world: &mut World, session: &mut EditSession, command: &EditCommand) -> bool {
    if session.content != EditContentType::Road {
        return false;
    }
    let result = match command {
        EditCommand::SelectCatalog { index } => {
            let site = session
                .baseline
                .as_ref()
                .map_or(0, super::FixturePlacements::site_id);
            match owned::catalog_for(world, site, &session.rows, true) {
                Ok((rows, _)) => {
                    session.road.cancel();
                    session.catalog_index = *index;
                    session.road.target = rows.into_iter().nth(*index);
                    session.road.target_uid = None;
                    session.road.select_mode = false;
                    session.changed();
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        EditCommand::SelectInventory { uid } => select_inventory(session, uid),
        EditCommand::RoadTap { center } => tap(world, session, *center),
        EditCommand::RoadLongStart { center, uid }
            if session.road.select_mode && session.road.original.is_empty() =>
        {
            let selected = if let Some(uid) = uid {
                select_uid(session, uid)
            } else {
                Ok(())
            };
            if session.road.original.is_empty() {
                session.road.multi = Some((
                    *center + GridPosition::new(-1, 0, -2),
                    *center + GridPosition::new(2, 0, 1),
                ));
                session.road.dragging = true;
                session.changed();
            }
            selected
        }
        EditCommand::RoadLongStart { .. } => Ok(()),
        EditCommand::RoadLongEnd => finish_multi(world, session),
        EditCommand::RoadDragStart { center, uid } => {
            if session.road.multi.is_some() {
                session.road.dragging = true;
            } else {
                let touches_planned = uid
                    .as_ref()
                    .is_some_and(|uid| session.road.planned.iter().any(|row| row.uid == *uid));
                session.road.dragging = touches_planned;
                if touches_planned && session.road.select_mode {
                    session.road.drag_origin = Some(*center);
                    session.road.drag_rows = session.road.planned.clone();
                }
            }
            session.changed();
            Ok(())
        }
        EditCommand::RoadDragMove { center, raw_center } if session.road.dragging => {
            if let Some((_, end)) = session.road.multi.as_mut() {
                *end = *raw_center + GridPosition::new(2, 0, 1);
                session.changed();
                Ok(())
            } else if session.road.select_mode {
                move_to(world, session, *center)
            } else {
                moly_law::fixture::road_edit::update_drag_history(
                    &mut session.road.history,
                    *center,
                );
                match moly_law::fixture::road_edit::drag_candidates(&session.road.history) {
                    Ok(candidates) => plan(world, session, &candidates),
                    Err(e) => Err(e.into()),
                }
            }
        }
        EditCommand::RoadDragMove { .. } => Ok(()),
        EditCommand::RoadDragEnd => {
            session.road.dragging = false;
            session.road.drag_origin = None;
            session.road.drag_rows.clear();
            session.changed();
            finish_multi(world, session)
        }
        EditCommand::RoadPointerCancel => {
            if session.road.cancel_pointer() {
                session.changed();
            }
            Ok(())
        }
        EditCommand::RoadToggleFixtures => {
            session.road.hide_fixtures = !session.road.hide_fixtures;
            session.changed();
            Ok(())
        }
        EditCommand::RoadSelectMode => {
            session.road.cancel();
            session.road.target = None;
            session.road.target_uid = None;
            session.road.select_mode = true;
            session.changed();
            Ok(())
        }
        EditCommand::RoadPlanningMode => {
            session.road.cancel();
            session.road.target = None;
            session.road.target_uid = None;
            session.road.select_mode = false;
            session.changed();
            Ok(())
        }
        EditCommand::SelectPlaced { uid } => {
            let selected = session
                .rows
                .iter()
                .find(|row| row.uid == *uid && row.layout == layout_type::ROAD)
                .cloned();
            session.road.cancel();
            session.road.select_mode = true;
            session.road.target = None;
            session.road.target_uid = None;
            if let Some(row) = selected {
                session.road.original = vec![row.clone()];
                session.road.planned = vec![row];
                session.road.can_decide = true;
            }
            session.changed();
            Ok(())
        }
        EditCommand::Decide => decide(world, session),
        EditCommand::Cancel => {
            session.road.cancel();
            session.changed();
            Ok(())
        }
        EditCommand::CleanUpAll if session.clean_up_dialog => {
            session.clean_up_dialog = false;
            let mut removed = Vec::new();
            session.rows.retain(|row| {
                if row.layout == layout_type::ROAD {
                    removed.push(row.clone());
                    false
                } else {
                    true
                }
            });
            session.inventory.extend(removed);
            session.road.cancel();
            session.changed();
            Ok(())
        }
        EditCommand::ReturnToInventory => {
            let ids: Vec<_> = session
                .road
                .original
                .iter()
                .map(|row| row.uid.clone())
                .collect();
            let mut removed = Vec::new();
            session.rows.retain(|row| {
                if ids.contains(&row.uid) {
                    removed.push(row.clone());
                    false
                } else {
                    true
                }
            });
            session.inventory.extend(removed);
            session.road.cancel();
            session.changed();
            Ok(())
        }
        EditCommand::Rotate | EditCommand::Nudge { .. } | EditCommand::MoveTo { .. } => Ok(()),
        _ => return false,
    };
    if let Err(error) = result {
        session.say(error);
    }
    true
}

pub(super) fn view(world: &World, session: &EditSession) -> crate::ui_inputs::EditRoadView {
    let target = session
        .road
        .target
        .as_ref()
        .map(|row| item_view(&row.fixture(String::new())));
    let remaining = session
        .road
        .target
        .as_ref()
        .and_then(|target| available(world, session, target).ok())
        .map_or(0, |n| (n - session.road.planned.len() as i32).max(0));
    crate::ui_inputs::EditRoadView {
        select_mode: session.road.select_mode,
        selection_bounds: session.road.multi,
        target,
        planned: session.road.planned.iter().map(item_view).collect(),
        moving: !session.road.original.is_empty(),
        dragging: session.road.dragging,
        can_decide: session.road.can_decide,
        remaining,
        can_cancel: session.road.original.is_empty()
            || session.road.planned != session.road.original,
        show_fixtures: !session.road.hide_fixtures,
        show_count: !session.road.select_mode
            && (session.road.dragging || session.road.history.len() <= 1),
        markers: if !session.road.select_mode
            && session.road.planned.len() > 1
            && !session.road.history.is_empty()
        {
            session
                .road
                .history
                .first()
                .copied()
                .zip(session.road.history.last().copied())
        } else {
            None
        },
    }
}

fn replaced_by(old: &EditableFixture, target: &EditableFixture) -> bool {
    if old.layout != layout_type::ROAD || old.uid == target.uid {
        return false;
    }
    let rules = Rules::default();
    let (Ok(old), Ok(target)) = (Piece::new(old, &rules), Piece::new(target, &rules)) else {
        return false;
    };
    let overlaps = old.min.x <= target.max.x
        && old.max.x >= target.min.x
        && old.min.z <= target.max.z
        && old.max.z >= target.min.z;
    overlaps && (old.fixture_id != target.fixture_id || old.center == target.min)
}

pub(super) fn projection(session: &EditSession) -> Vec<EditableFixture> {
    let mut rows: Vec<_> = session
        .rows
        .iter()
        .filter(|row| {
            !session
                .road
                .planned
                .iter()
                .any(|target| replaced_by(row, target))
        })
        .cloned()
        .collect();
    for row in &session.road.planned {
        if let Some(old) = rows.iter_mut().find(|old| old.uid == row.uid) {
            *old = row.clone();
        } else {
            rows.push(row.clone());
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uid: &str, id: i32, x: i8, z: i8) -> EditableFixture {
        let target = owned::OwnedRow {
            fixture_id: id,
            texture_id: 0,
            package: format!("road-{id}"),
            grid_size: moly_law::fixture::Vector3Int::new(2, 1, 2),
            layout: layout_type::ROAD,
            quantity: 8,
            layouted: 0,
        };
        source_item(&target, GridPosition::new(x, 0, z), uid.into()).unwrap()
    }

    fn rules_and_grid() -> (Rules, crate::site::FloorGridLayout) {
        let mut rules = Rules::default();
        for id in [1, 2] {
            rules.traits.insert(
                id,
                moly_layout::validate::Traits::from_row("normal", "road", "none", "road"),
            );
        }
        (
            rules,
            crate::site::FloorGridLayout {
                level: 1,
                layout_id: 1,
                width: 20,
                height: 8,
                depth: 20,
            },
        )
    }

    #[test]
    fn cancel_move_restores_the_exact_projection_and_keeps_inventory() {
        let original = row("original", 1, 0, 0);
        let other = row("other", 2, 4, 0);
        let mut session = EditSession {
            rows: vec![original.clone(), other.clone()],
            ..Default::default()
        };
        session.road.original = vec![original.clone()];
        session.road.planned = vec![row("original", 1, 4, 0)];
        assert!(!projection(&session).iter().any(|row| row.uid == "other"));
        session.road.cancel();
        assert_eq!(projection(&session), vec![original, other]);
        assert!(session.inventory.is_empty());
    }

    #[test]
    fn decision_reuses_stored_uid_and_returns_replaced_roads() {
        let (rules, grid) = rules_and_grid();
        let mut session = EditSession {
            inventory: vec![row("stored", 1, 0, 0)],
            next_fixture_uid: 7,
            ..Default::default()
        };
        session.road.planned = vec![row("preview", 1, 2, 0)];
        let result = decide_rows(&session, &rules, grid, 1).unwrap();
        assert_eq!(result.rows[0].uid, "stored");
        assert!(result.inventory.is_empty());
        assert_eq!(result.next_uid, 7);
        assert_eq!(session.inventory[0].uid, "stored");
        let original = row("moving", 1, 0, 0);
        session.rows = vec![original.clone(), row("replaced", 2, 4, 0)];
        session.inventory.clear();
        session.road.original = vec![original];
        session.road.planned = vec![row("moving", 1, 4, 0)];
        let result = decide_rows(&session, &rules, grid, 1).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].uid, "moving");
        assert_eq!(result.inventory[0].uid, "replaced");
    }

    #[test]
    fn late_invalid_candidate_cannot_partially_debit_inventory_or_uid_cursor() {
        let (rules, grid) = rules_and_grid();
        let mut session = EditSession {
            inventory: vec![row("stored", 1, 0, 0)],
            next_fixture_uid: 7,
            ..Default::default()
        };
        session.road.planned = vec![row("first", 1, 0, 0), row("outside", 1, 9, 0)];
        assert!(decide_rows(&session, &rules, grid, 1).is_err());
        assert!(session.rows.is_empty());
        assert_eq!(session.inventory[0].uid, "stored");
        assert_eq!(session.next_fixture_uid, 7);
    }

    #[test]
    fn returned_count_does_not_double_count_server_owned_quantity() {
        assert_eq!(available_count(4, 2), 4);
        assert_eq!(available_count(0, 2), 2);
        assert_eq!(available_count(1, 2), 2);
        assert_eq!(available_count(-2, 0), 0);
    }

    #[test]
    fn returned_cell_keeps_its_exact_uid_through_planning_and_decision() {
        let (rules, grid) = rules_and_grid();
        let first = row("first-returned", 1, 0, 0);
        let selected = row("selected-returned", 1, 0, 0);
        let mut session = EditSession {
            inventory: vec![first.clone(), selected.clone()],
            next_fixture_uid: 7,
            ..Default::default()
        };
        select_inventory(&mut session, "selected-returned").unwrap();
        assert!(!session.road.select_mode);
        assert_eq!(
            session.road.target_uid.as_deref(),
            Some("selected-returned")
        );
        assert!(session.road.planned.is_empty());
        assert_eq!(session.inventory, vec![first.clone(), selected.clone()]);
        session.road.planned = vec![row("preview", 1, 2, 0)];
        let result = decide_rows(&session, &rules, grid, 1).unwrap();
        assert_eq!(result.rows[0].uid, "selected-returned");
        assert_eq!(result.inventory, vec![first]);
        assert_eq!(result.next_uid, 7);
        session
            .inventory
            .retain(|row| row.uid != "selected-returned");
        assert!(decide_rows(&session, &rules, grid, 1).is_err());
        assert_eq!(session.inventory[0].uid, "first-returned");
        assert_eq!(session.next_fixture_uid, 7);
    }

    #[test]
    fn empty_ground_long_press_publishes_rectangle_without_waiting_for_drag() {
        let mut world = World::new();
        let mut session = EditSession {
            content: EditContentType::Road,
            ..Default::default()
        };
        session.road.select_mode = true;
        let revision = session.revision;
        assert!(handle(
            &mut world,
            &mut session,
            &EditCommand::RoadLongStart {
                center: GridPosition::ZERO,
                uid: None,
            }
        ));
        assert!(session.revision > revision);
        assert!(session.road.dragging);
        assert_eq!(
            session.road.multi,
            Some((GridPosition::new(-1, 0, -2), GridPosition::new(2, 0, 1)))
        );
    }

    #[test]
    fn road_release_frame_does_not_pan_camera_and_next_ground_drag_does() {
        use crate::gesture::{GestureEvent, GestureKind, GestureState};
        let (_, grid) = rules_and_grid();
        let original = row("moving", 1, 0, 0);
        let mut world = World::new();
        world.insert_resource(super::super::FixtureAreas {
            loaded: true,
            ..Default::default()
        });
        super::super::put_effect::insert_test_tables(
            &mut world,
            &[(1, moly_layout::validate::Traits::from_row("normal", "road", "none", "road"))],
        );
        let mut session = EditSession {
            content: EditContentType::Road,
            phase: super::super::EditPhase::Browsing,
            baseline: Some(super::super::FixturePlacements::test_layout(&[original.clone()], grid)),
            rows: vec![original.clone()],
            ..Default::default()
        };
        session.road.select_mode = true;
        session.road.original = vec![original.clone()];
        session.road.planned = vec![original.clone()];
        session.road.drag_rows = vec![original.clone()];
        session.road.drag_origin = Some(GridPosition::ZERO);
        session.road.dragging = true;
        world.insert_resource(session);
        world.init_resource::<super::super::EditSessionActive>();
        world.insert_resource(super::super::PendingCommands(vec![
            EditCommand::RoadDragMove {
                center: GridPosition::new(2, 0, 0),
                raw_center: GridPosition::new(2, 0, 0),
            },
            EditCommand::RoadDragEnd,
        ]));
        let moved = GestureEvent {
            kind: GestureKind::Drag,
            state: GestureState::Moved,
            position: Vec2::new(80., 80.),
            delta: Vec2::new(24., 12.),
            ui_owned: false,
        };
        let ended = GestureEvent { state: GestureState::End, ..moved };
        super::super::apply_commands(&mut world);
        let session = world.resource::<EditSession>();
        assert!(!session.road.dragging);
        assert_ne!(session.road.planned[0].center, original.center);
        assert_eq!(session.rows, vec![original.clone()]);
        assert_eq!(
            crate::floor_edit_camera::camera_drag_delta([moved, ended].iter(), false, session.road_dragging()),
            Vec2::ZERO,
        );
        // The no-command frame must release the frame claim as well.
        super::super::apply_commands(&mut world);
        assert!(!world.resource::<EditSession>().road_dragging());
        // A later ground drag has no road hit and therefore still pans.
        world.resource_mut::<super::super::PendingCommands>().0 = vec![
            EditCommand::RoadDragStart { center: GridPosition::new(6, 0, 0), uid: None },
            EditCommand::RoadDragMove {
                center: GridPosition::new(8, 0, 0),
                raw_center: GridPosition::new(8, 0, 0),
            },
            EditCommand::RoadDragEnd,
        ];
        super::super::apply_commands(&mut world);
        assert_eq!(
            crate::floor_edit_camera::camera_drag_delta(
                [moved, ended].iter(), false, world.resource::<EditSession>().road_dragging(),
            ),
            moved.delta,
        );
    }

    #[test]
    fn pointer_loss_releases_rectangle_even_while_settings_blocks_edit_commands() {
        for modal_open in [false, true] {
            let mut world = World::new();
            let original = row("original", 1, 0, 0);
            let stored = row("stored", 2, 0, 0);
            let mut session = EditSession {
                content: EditContentType::Road,
                phase: super::super::EditPhase::Browsing,
                rows: vec![original.clone()],
                inventory: vec![stored.clone()],
                ..Default::default()
            };
            session.road.original = vec![original.clone()];
            session.road.planned = vec![original.clone()];
            session.road.multi = Some((GridPosition::ZERO, GridPosition::new(3, 0, 3)));
            session.road.drag_origin = Some(GridPosition::ZERO);
            session.road.drag_rows = vec![original.clone()];
            session.road.dragging = true;
            let revision = session.revision;
            world.insert_resource(session);
            world.init_resource::<super::super::EditSessionActive>();
            world.insert_resource(crate::game_settings::SettingsPanel {
                open: modal_open,
                ..Default::default()
            });
            // Host drag cancellation also emits DragEnd. No selection/board
            // operation may be attempted here (this world has no grid).
            world.insert_resource(super::super::PendingCommands(vec![
                EditCommand::RoadDragEnd,
                EditCommand::RoadPointerCancel,
            ]));
            super::super::apply_commands(&mut world);
            let session = world.resource::<EditSession>();
            assert!(session.revision > revision);
            assert!(!session.road.dragging);
            assert!(session.road.multi.is_none());
            assert!(session.road.drag_origin.is_none());
            assert!(session.road.drag_rows.is_empty());
            assert_eq!(session.rows, vec![original.clone()]);
            assert_eq!(session.road.planned, vec![original]);
            assert_eq!(session.inventory, vec![stored]);
        }
    }
}
