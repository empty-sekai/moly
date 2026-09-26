//! Ground fixture editing with one command boundary and a private map draft.
//!
//! Source: FloorEditState.SelectFixture snapshots StartPosition/StartDirection;
//! Reset restores them (or removes a pre-placement preview); Decide updates
//! tile/layout data before unselecting. SiteLayoutEditor keeps an explicit
//! Cancel / Save / Discard exit dialog. The offline storage envelope and four
//! unlimited catalog rows are product-owned inputs, not real account data.
//!
//! Existing world roots can display draft poses, but FixturePlacements and
//! returned inventory are published only after the shared store succeeds.
//! The put check, the drag's clamp and height, stacking and the rotation of
//! stacked fixtures follow `FloorEditState` and `SiteLayoutUtility`
//! (`tile_rules`). Walls, fences and roads have their own edit states and
//! are not converted into guessed ground behaviour.

mod actors;
mod assets;
mod autoplay;
mod edit_grid;
mod game_state;
mod input;
mod placement;
mod presentation;
mod put_effect;
mod tile_rules;
mod validation;

#[cfg(test)]
mod tests;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture::{EditableFixture, FixturePlacements, OccupancyRow};
use assets::{CANDIDATES, FixtureAreas};
use bevy::prelude::*;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition, Vector3Int};

pub(crate) use input::{read_keyboard, read_pointer};
pub(crate) use put_effect::site_origin;

#[derive(Resource, Default)]
pub struct EditSessionActive {
    active: bool,
}

impl EditSessionActive {
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[derive(Debug, Clone, Copy, Message)]
pub struct LayoutSaved;

/// UI and platform input may request operations, never mutate draft fields.
#[derive(Message, Clone, Debug)]
pub(crate) enum EditCommand {
    Enter,
    SelectCatalog { index: usize },
    SelectPlaced { uid: String },
    SelectInventory { uid: String },
    /// A drag's touched tile (product frame, `GetSelectTile`): the drag puts
    /// the selection's source center there, clamped into the grid and
    /// raised by the column.
    MoveTo { center: GridPosition },
    /// The desktop arrow keys: one tile from the source center.
    Nudge { x: i8, z: i8 },
    Rotate,
    Decide,
    Cancel,
    ReturnToInventory,
    Save,
    RequestExit,
    KeepEditing,
    SaveAndExit,
    DiscardAndExit,
    /// The camera rotate button (`LayoutAction` 13).
    RotateCamera,
    /// The change-look button (`LayoutAction` 14).
    ChangeLookCamera,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EditPhase {
    #[default]
    Idle,
    Browsing,
    Placing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditItemView {
    pub uid: String,
    pub package: String,
    pub texture_id: u32,
    pub fixture_id: i32,
    pub center: GridPosition,
    pub direction: Direction,
    pub grid_size: Vector3Int,
    pub layout: u8,
    pub editable: bool,
}

impl From<&EditableFixture> for EditItemView {
    fn from(item: &EditableFixture) -> Self {
        Self {
            uid: item.uid.clone(),
            package: item.package.clone(),
            texture_id: item.texture_id,
            fixture_id: item.fixture_id,
            center: item.center,
            direction: item.direction,
            grid_size: item.grid_size,
            layout: item.layout,
            editable: validation::is_ground(item),
        }
    }
}

/// `SiteLayoutUtility.CanPutFloor` of the selection at its draft place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PutStatus {
    Ok,
    /// The first check it failed.
    Refused(tile_rules::Refusal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditSelectionView {
    pub item: EditItemView,
    pub from_inventory: bool,
    pub is_new_mock: bool,
    /// The store (return to inventory) action is offered.
    pub can_clean_up: bool,
    pub put_status: PutStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditCatalogView {
    pub index: usize,
    pub package: &'static str,
    pub fixture_id: i32,
    pub grid_size: Vector3Int,
    pub unlimited_mock: bool,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditView {
    pub revision: u64,
    pub active: bool,
    pub phase: EditPhase,
    pub site_id: u32,
    pub site_type: String,
    pub dirty: bool,
    pub exit_dialog: bool,
    pub selected: Option<EditSelectionView>,
    pub placed_rows: Vec<EditItemView>,
    pub inventory: Vec<EditItemView>,
    pub catalog: Vec<EditCatalogView>,
    pub can_save: bool,
    pub feedback: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionOrigin {
    Placed,
    Inventory,
    Mock,
}

#[derive(Clone)]
struct Selection {
    item: EditableFixture,
    /// `LayoutEditData._stackedFixtures`: the fixtures stacked on the
    /// selected one (`GetStackedFixtures` at selection), at their draft
    /// places; they move and rotate with it and are decided with it.
    stacked: Vec<EditableFixture>,
    origin: SelectionOrigin,
    /// `FixtureController.CanCleanUp` of the selected fixture: not a gate and
    /// not the player's house. Only a placed fixture offers the store action.
    can_clean_up: bool,
}

#[derive(Resource, Default)]
pub(crate) struct EditSession {
    next_fixture_uid: u64,
    revision: u64,
    phase: EditPhase,
    catalog_index: usize,
    baseline: Option<FixturePlacements>,
    storage_baseline: Option<crate::fixture::layouts::EditStorageSnapshot>,
    /// Keep editing input/actor ownership while the successful save's actual
    /// fixture scenes and matching navigation generation are being restored.
    pending_reload: Option<actors::PendingReload>,
    rows: Vec<EditableFixture>,
    initial_inventory: Vec<EditableFixture>,
    inventory: Vec<EditableFixture>,
    selected: Option<Selection>,
    exit_dialog: bool,
    feedback: String,
}

impl EditSession {
    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("editor revision exhausted");
    }

    fn say(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.feedback != message {
            self.feedback = message;
            self.changed();
        }
    }

    fn dirty(&self) -> bool {
        self.baseline.as_ref().is_some_and(|base| {
            self.rows != base.editor_rows() || self.inventory != self.initial_inventory
        })
    }

    fn cancel_selection(&mut self) {
        // No original row or inventory record was removed during selection.
        // Dropping this pose overlay therefore restores the exact start state.
        if self.selected.take().is_some() {
            self.phase = EditPhase::Browsing;
            self.changed();
            self.say("已取消本次操作，家具恢复到选中前的位置。");
        }
    }

    fn finish(&mut self) {
        self.phase = EditPhase::Idle;
        self.baseline = None;
        self.storage_baseline = None;
        self.pending_reload = None;
        self.rows.clear();
        self.initial_inventory.clear();
        self.inventory.clear();
        self.selected = None;
        self.exit_dialog = false;
        self.changed();
    }
}

#[derive(Resource, Default)]
struct PendingCommands(Vec<EditCommand>);

/// Input is the existing early keyboard-producer set. The root schedule owns
/// the late UI/pointer ordering; do not make the early layout loader depend on
/// Commands, because the site's loader precedes the field UI input chain.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FixtureEditSystems {
    Input,
    Pointer,
    Commands,
    View,
}

pub(crate) fn has_unsaved_layout(session: &EditSession) -> bool {
    session.dirty() || session.selected.is_some() || session.pending_reload.is_some()
}

/// Draft occupancy never enters the gameplay walk-field or claims playable
/// fixture geometry. Commit emits LayoutSaved and the normal loader rebakes
/// from FixturePlacements. Editing owns input and pauses new activity admission.
pub(crate) fn decided_footprints(_: &EditSession) -> Vec<(GridPosition, GridPosition, i8, u8)> {
    Vec::new()
}

pub(crate) fn decided_scene_rows(_: &EditSession, _: &str) -> Vec<OccupancyRow> {
    Vec::new()
}

pub(crate) fn clear_for_site_change(world: &mut World) {
    if let Some(mut session) = world.get_resource_mut::<EditSession>() {
        session.finish();
    }
    if let Some(mut active) = world.get_resource_mut::<EditSessionActive>() {
        active.active = false;
    }
    presentation::clear(world);
    actors::clear_overlay(world);
    game_state::exit_for_site_change(world);
}

fn receive_commands(
    mut commands: MessageReader<EditCommand>,
    mut pending: ResMut<PendingCommands>,
) {
    pending.0.extend(commands.read().cloned());
}

fn play_se(world: &mut World, cue: &'static str, source: &'static str) {
    if let Some(mut se) = world.get_resource_mut::<SeRequests>() {
        se.0.push(SeRequest { owner: None,
            cue: cue.into(),
            class: SeClass::Ui,
            source,
        });
    }
}

fn begin(world: &mut World, session: &mut EditSession) {
    if session.phase != EditPhase::Idle {
        return;
    }
    if world
        .get_resource::<crate::ui_layers::UiLayerStack>()
        .is_some_and(|layers| !layers.on_field())
        || world
            .get_resource::<crate::menu_shell::ShellDialogState>()
            .is_some_and(|dialogs| dialogs.blocks_field_input())
    {
        session.say("请先关闭当前窗口，再编辑家具。");
        return;
    }
    if world.contains_resource::<crate::player_talk::PlayerTalkSession>()
        || world.contains_resource::<crate::talk::ActiveTalk>()
    {
        session.say("请先结束对话，再编辑家具。");
        return;
    }
    let Some(layout) = world.get_resource::<FixturePlacements>().cloned() else {
        return;
    };
    if layout.site_id() == 0
        || layout.floor_grid().is_none()
        || !world.contains_resource::<crate::fixture::FixtureScenesReady>()
        || (layout.total() != 0
            && !world.contains_resource::<crate::fixture_material::FixtureMaterialsSwapped>())
    {
        session.say("场地或家具仍在加载，请稍后进入编辑。");
        return;
    }
    if !world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == layout.site_type())
    {
        session.say("场地正在切换，暂时不能开始编辑。");
        return;
    }
    let homes = world.get_resource::<crate::entry::house::HomeFixtures>();
    let entry = world.get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
        .ok_or_else(|| "布局存档所有者尚未就绪".to_owned())
        .and_then(|layouts| layouts.begin_edit(&layout, homes));
    let (storage_baseline, inventory) = match entry {
        Ok(entry) => entry,
        Err(error) => {
            session.say(format!("无法开始编辑：{error}；原地图与库存存档未改动。"));
            return;
        }
    };
    session.next_fixture_uid = session.next_fixture_uid.max(layout.next_edit_uid()).max(1);
    session.rows = layout.editor_rows();
    session.baseline = Some(layout);
    session.storage_baseline = Some(storage_baseline);
    session.initial_inventory = inventory.clone();
    session.inventory = inventory;
    session.phase = EditPhase::Browsing;
    session.exit_dialog = false;
    session.selected = None;
    session.changed();
    world.resource_mut::<EditSessionActive>().active = true;
    actors::enter(world);
    // These are the existing owners' normal cancellation paths, not a blanket
    // component deletion or a site-change shortcut that strands NPC objectives.
    crate::player_fixture_action::cancel_for_layout_edit(world);
    crate::npc_fixture_activity::cancel_for_layout_edit(world);
    crate::fixture_gimmick::cancel_for_site_change(world);
    crate::fixture_scene_inputs::invalidate_for_site_change(world);
    play_se(world, "se_change_layout", "edit-enter");
    game_state::enter(world);
    if std::env::var("MOLY_EDIT_AUTOPLAY").is_ok() {
        census(world, session);
    }
    session.say("已进入家具编辑：点击家具或清单选中；决定后仍是草稿，保存才写入本地图。");
}

/// Instrument: the placed floor and rug rows with the master predicates the
/// put rules read, the fixtures stacked on each, and what the rules miss.
fn census(world: &World, session: &EditSession) {
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let rules = match tile_rules::rules(world, floor) {
        Ok(rules) => rules,
        Err(missing) => {
            warn!("[edit-put] census: the put rules miss {missing:?}");
            return;
        }
    };
    let Some(floor) = floor else {
        return;
    };
    let board = match tile_rules::Board::new(&session.rows, floor, &rules) {
        Ok(board) => board,
        Err(missing) => {
            warn!("[edit-put] census: the tile data misses {missing:?}");
            return;
        }
    };
    info!(
        "[edit-put] census: floor layout {} ({}x{}x{}), {} unavailable zones, {} rows",
        floor.layout_id,
        floor.width,
        floor.height,
        floor.depth,
        rules.zones.as_ref().map_or(0, |zones| zones.len()),
        session.rows.len()
    );
    for row in &session.rows {
        let Some(piece) = board.piece(&row.uid) else {
            continue;
        };
        let stacked = board.stacked_on(&row.uid);
        info!(
            "[edit-put] census {} fixture {} layout {} product center ({}, {}, {}) source box ({}, {}, {})..=({}, {}, {}) {:?} stack cells {} add-using {} stacked on it {:?} refusal {:?}",
            row.uid,
            row.fixture_id,
            row.layout,
            row.center.x,
            row.center.y,
            row.center.z,
            piece.min.x,
            piece.min.y,
            piece.min.z,
            piece.max.x,
            piece.max.y,
            piece.max.z,
            piece.traits,
            piece.stack_cell_count(),
            piece.add_using_count(),
            stacked,
            board
                .can_put_floor(
                    piece,
                    &stacked
                        .iter()
                        .filter_map(|uid| board.piece(uid).cloned())
                        .collect::<Vec<_>>()
                )
                .err()
                .map(tile_rules::refusal_label)
        );
    }
}

/// `FocusOnLayoutEdit` (event 36) for a fixture: its view position, its
/// current grid size and zoom support unless it is a block.
fn focus(world: &mut World, item: &EditableFixture, source: &'static str) {
    let Ok(pose) = item.pose() else {
        return;
    };
    let position = site_origin(world) + pose.translation;
    let size = put_effect::current_grid_size(item.grid_size, item.direction);
    let size = Vec3::new(size.x as f32, size.y as f32, size.z as f32);
    let zoom_support = match put_effect::is_block(world, item.fixture_id) {
        Some(block) => !block,
        None => {
            warn!("[edit-camera] focus on {} ({source}): fixture {} has no handle type in the fixture table; focused without zoom support", item.uid, item.fixture_id);
            false
        }
    };
    crate::floor_edit_camera::focus(world, position, size, zoom_support, source);
}

/// `FloorEditState.PutFixture` before the new fixture is shown: its
/// direction from the camera yaw and its tile from the spiral around the
/// camera look-at (`placement`). `false` is `CantPutFixture` (no tile), or a
/// refusal while an input the search reads is missing; `item` is unchanged
/// then and nothing is selected.
fn place_new(
    world: &mut World,
    session: &mut EditSession,
    item: &mut EditableFixture,
    source: &'static str,
) -> bool {
    if !validation::is_ground(item) {
        // `select` names the branch this editor does not build.
        return true;
    }
    let Some(floor) = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid)
    else {
        session.say("地图等级尺寸尚未就绪，暂时不能摆放新家具。");
        return false;
    };
    let put_type = match put_effect::put_type(world, item.fixture_id) {
        Ok(put_type) => put_type,
        Err(error) => {
            warn!("[edit-put] PutFixture {} ({source}): {error}; not put", item.uid);
            session.say("家具主表仍在加载，暂时不能摆放新家具。");
            return false;
        }
    };
    let Some(camera) = world
        .get_resource::<crate::camera::FieldCameraModel>()
        .map(|model| (model.look_at, model.yaw))
    else {
        session.say("相机尚未就绪，暂时不能摆放新家具。");
        return false;
    };
    let (look_at, yaw) = camera;
    let Some(source_direction) = placement::rotation_to_direction(yaw) else {
        warn!("[edit-put] PutFixture {} ({source}): camera yaw {yaw} names no direction; not put", item.uid);
        session.say("相机朝向无效，暂时不能摆放新家具。");
        return false;
    };
    // The look-at relative to the site, in the source frame (X mirrored).
    let relative = look_at - site_origin(world);
    let relative = Vec3::new(-relative.x, relative.y, relative.z);
    let board = match tile_rules::rules(world, Some(floor))
        .and_then(|rules| Ok((tile_rules::Board::new(&session.rows, floor, &rules)?, rules)))
    {
        Ok(found) => found,
        Err(missing) => {
            warn!("[edit-put] PutFixture {} ({source}): the put check's input {missing:?} is missing; not put", item.uid);
            session.say(format!("摆放检查的输入尚未就绪（{missing:?}），暂时不能摆放新家具。"));
            return false;
        }
    };
    let (board, rules) = board;
    let found = placement::place(
        item,
        source_direction,
        relative,
        &board,
        &rules,
        put_type.as_deref(),
    );
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, reason) in &found.rejected {
        let label = placement::reject_label(reason);
        match counts.iter_mut().find(|(known, _)| *known == label) {
            Some((_, n)) => *n += 1,
            None => counts.push((label, 1)),
        }
    }
    info!(
        "[edit-put] PutFixture {} (fixture {}, {source}): camera look-at {:.3} (source frame, site-relative) -> grid ({}, {}, {}) -> start tile ({}, {}, {}); camera yaw {yaw:.2} -> direction {:?}; put type {:?} ({}); radius cap {}; {} candidates rejected {:?}",
        item.uid,
        item.fixture_id,
        found.look_at,
        found.look_at_grid.x,
        found.look_at_grid.y,
        found.look_at_grid.z,
        found.start.x,
        found.start.y,
        found.start.z,
        found.source_direction,
        put_type,
        if found.search_raised { "raised rings searched" } else { "rings at y 0" },
        found.max_radius,
        found.rejected.len(),
        counts
    );
    if std::env::var("MOLY_EDIT_AUTOPLAY").is_ok() {
        for (index, (tile, reason)) in found.rejected.iter().enumerate().take(400) {
            info!(
                "[edit-put]   rejected #{index} tile ({}, {}, {}): {}",
                tile.x,
                tile.y,
                tile.z,
                placement::reject_label(reason)
            );
        }
    }
    let Some((tile, center, direction)) = found.chosen else {
        warn!("[edit-put] PutFixture {} ({source}): no placeable tile within the radius cap; CantPutFixture", item.uid);
        session.say("附近没有可以摆放的空间。");
        return false;
    };
    item.center = center;
    item.direction = direction;
    match item.footprint() {
        Ok((min, max)) => info!(
            "[edit-put] PutFixture {} ({source}): tile ({}, {}, {}) (source frame) -> product center ({}, {}, {}) direction {direction:?}; footprint ({}, {}, {})..=({}, {}, {})",
            item.uid, tile.x, tile.y, tile.z, center.x, center.y, center.z,
            min.x, min.y, min.z, max.x, max.y, max.z
        ),
        Err(error) => warn!("[edit-put] PutFixture {} ({source}): chosen tile has no footprint ({error})", item.uid),
    }
    true
}

/// The committed rows stacked on `uid` (`GetStackedFixtures`), in the
/// source's depth-first order.
fn stacked_rows(
    world: &World,
    session: &EditSession,
    uid: &str,
) -> Result<Vec<EditableFixture>, tile_rules::Missing> {
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let rules = tile_rules::rules(world, floor)?;
    let board = tile_rules::Board::new(
        &session.rows,
        floor.ok_or(tile_rules::Missing::FloorGrid)?,
        &rules,
    )?;
    Ok(board
        .stacked_on(uid)
        .iter()
        .filter_map(|stacked| session.rows.iter().find(|row| row.uid == *stacked).cloned())
        .collect())
}

/// The selection's put check against the committed rows.
fn put_status(world: &World, session: &EditSession, selection: &Selection) -> PutStatus {
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let rules = tile_rules::rules(world, floor);
    validation::put(
        &selection.item,
        &selection.stacked,
        &session.rows,
        floor,
        rules.as_ref().map_err(|missing| *missing),
    )
}

/// A source-frame place of a draft row, back in the product frame.
fn product_row(
    item: &EditableFixture,
    center: GridPosition,
    direction: Direction,
    layout: u8,
) -> Result<EditableFixture, String> {
    let (center, direction, layout) =
        moly_assets::player_data::mirror_fixture_layout(center, item.grid_size, direction, layout)?;
    Ok(EditableFixture {
        center,
        direction,
        layout,
        ..item.clone()
    })
}

/// Where a drag puts the selection.
#[derive(Clone, Copy, Debug)]
enum DragTarget {
    /// The touched tile, in the product frame (`GetSelectTile`).
    Touched(GridPosition),
    /// The desktop arrow keys: one tile from the source center.
    Step { x: i8, z: i8 },
}

/// `FloorEditState.UpdateEditTargetPosition` and
/// `UpdateEditTargetStackFixture`: the center goes to the target clamped so
/// the box stays in the grid (`ClampPosition`), its y to the column height
/// at the new minimum corner (`AdjustSelectTileHeight`), and the stacked
/// fixtures move by the same offset.
fn drag(world: &mut World, session: &mut EditSession, target: DragTarget) {
    let Some(selected) = session.selected.as_ref() else {
        return;
    };
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let prepared = tile_rules::rules(world, floor).and_then(|rules| {
        let floor = floor.ok_or(tile_rules::Missing::FloorGrid)?;
        let board = tile_rules::Board::new(&session.rows, floor, &rules)?;
        let piece = tile_rules::Piece::new(&selected.item, &rules)?;
        Ok((rules, board, piece))
    });
    let (rules, board, piece) = match prepared {
        Ok(prepared) => prepared,
        Err(missing) => {
            session.say(format!("摆放检查的输入尚未就绪（{missing:?}），未移动家具。"));
            return;
        }
    };
    let target = match target {
        DragTarget::Touched(tile) => {
            GridPosition::new((-i16::from(tile.x) - 1) as i8, tile.y, tile.z)
        }
        DragTarget::Step { x, z } => piece.center + GridPosition::new(x.wrapping_neg(), 0, z),
    };
    let clamped = board.clamp(&piece, target);
    let moved = match tile_rules::Piece::at(
        &selected.item,
        &rules,
        clamped,
        piece.direction,
        piece.layout,
    ) {
        Ok(moved) => moved,
        Err(missing) => {
            session.say(format!("目标位置超出网格（{missing:?}），未移动家具。"));
            return;
        }
    };
    let y = board.column_height(piece.layout, moved.min, &piece.uid);
    let center = GridPosition::new(clamped.x, y, clamped.z);
    let offset = center - piece.center;
    let result =
        product_row(&selected.item, center, piece.direction, piece.layout).and_then(|item| {
            let stacked = selected
                .stacked
                .iter()
                .map(|child| {
                    let child_piece = tile_rules::Piece::new(child, &rules)
                        .map_err(|missing| format!("{missing:?}"))?;
                    product_row(
                        child,
                        child_piece.center + offset,
                        child_piece.direction,
                        child_piece.layout,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((item, stacked))
        });
    let (item, stacked) = match result {
        Ok(found) => found,
        Err(error) => {
            session.say(format!("目标位置超出网格（{error}），未移动家具。"));
            return;
        }
    };
    let selected = session.selected.as_mut().expect("checked selection");
    if selected.item == item && selected.stacked == stacked {
        return;
    }
    info!(
        "[edit-put] drag {}: target ({}, {}, {}) -> clamped ({}, {}, {}) -> column height {y} at min ({}, {}); source center ({}, {}, {}) -> ({}, {}, {}); product center ({}, {}, {}); {} stacked moved by ({}, {}, {})",
        item.uid,
        target.x,
        target.y,
        target.z,
        clamped.x,
        clamped.y,
        clamped.z,
        moved.min.x,
        moved.min.z,
        piece.center.x,
        piece.center.y,
        piece.center.z,
        center.x,
        center.y,
        center.z,
        item.center.x,
        item.center.y,
        item.center.z,
        stacked.len(),
        offset.x,
        offset.y,
        offset.z
    );
    selected.item = item;
    selected.stacked = stacked;
    session.changed();
}

/// `FloorEditState.OnPushRotationButton`.
fn rotate(world: &mut World, session: &mut EditSession) {
    use moly_law::fixture::areas::layout_square_min;
    let Some(selected) = session.selected.as_ref() else {
        return;
    };
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let prepared = tile_rules::rules(world, floor).and_then(|rules| {
        let floor = floor.ok_or(tile_rules::Missing::FloorGrid)?;
        let board = tile_rules::Board::new(&session.rows, floor, &rules)?;
        let piece = tile_rules::Piece::new(&selected.item, &rules)?;
        let stacked = selected
            .stacked
            .iter()
            .map(|child| tile_rules::Piece::new(child, &rules))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((rules, board, piece, stacked))
    });
    let (rules, board, piece, stacked) = match prepared {
        Ok(prepared) => prepared,
        Err(missing) => {
            session.say(format!("摆放检查的输入尚未就绪（{missing:?}），未旋转家具。"));
            return;
        }
    };
    if board.crossing_multiple_bases(&piece, &stacked) {
        info!(
            "[edit-put] OnPushRotationButton {}: a fixture on it crosses more than this base; not rotated",
            piece.uid
        );
        return;
    }
    // RotationSquareToRight (source direction + 1), then
    // UpdateEditRotateTargetPosition at the grid position.
    let direction =
        Direction::from_u8((piece.direction as u8 + 1) % 4).expect("four source directions");
    let turned =
        match tile_rules::Piece::at(&selected.item, &rules, piece.center, direction, piece.layout) {
            Ok(turned) => turned,
            Err(missing) => {
                session.say(format!("旋转后超出网格（{missing:?}），未旋转家具。"));
                return;
            }
        };
    let clamped = board.clamp(&turned, piece.center);
    let placed =
        match tile_rules::Piece::at(&selected.item, &rules, clamped, direction, piece.layout) {
            Ok(placed) => placed,
            Err(missing) => {
                session.say(format!("旋转后超出网格（{missing:?}），未旋转家具。"));
                return;
            }
        };
    let y = board.column_height(piece.layout, placed.min, &piece.uid);
    let center = GridPosition::new(clamped.x, y, clamped.z);
    // UpdateStackedFixturesPositions(clamped - grid position), then
    // RotationToRightStackedFixture about the base's square minimum.
    let offset = clamped - piece.center;
    let base_square = layout_square_min(center, selected.item.grid_size);
    let base_size = selected.item.grid_size;
    let result = product_row(&selected.item, center, direction, piece.layout).and_then(|item| {
        let rows = selected
            .stacked
            .iter()
            .zip(&stacked)
            .map(|(row, child)| {
                let at = child.center + offset;
                let own = tile_rules::rotate_right(
                    at,
                    layout_square_min(at, row.grid_size),
                    row.grid_size,
                );
                let d = own - at;
                let about_base = tile_rules::rotate_right(at, base_square, base_size);
                let moved = about_base - GridPosition::new(d.x, 0, d.z);
                let turned = Direction::from_u8((child.direction as u8 + 1) % 4)
                    .expect("four source directions");
                product_row(row, moved, turned, child.layout)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((item, rows))
    });
    let (item, rows) = match result {
        Ok(found) => found,
        Err(error) => {
            session.say(format!("旋转后超出网格（{error}），未旋转家具。"));
            return;
        }
    };
    info!(
        "[edit-put] OnPushRotationButton {}: source direction {:?} -> {direction:?}; center ({}, {}, {}) -> clamped ({}, {}, {}) height {y}; {} stacked rotated with it",
        item.uid,
        piece.direction,
        piece.center.x,
        piece.center.y,
        piece.center.z,
        clamped.x,
        clamped.y,
        clamped.z,
        rows.len()
    );
    let selected = session.selected.as_mut().expect("checked selection");
    selected.item = item;
    selected.stacked = rows;
    session.changed();
}

fn select(
    session: &mut EditSession,
    item: EditableFixture,
    origin: SelectionOrigin,
    world: &mut World,
) {
    if !validation::is_ground(&item) {
        session.say("该家具属于墙面编辑分支，当前地面编辑不会改动它。");
        return;
    }
    // FloorEditState.GetTouchedFixture skips fences and roads: they belong to
    // the fence and road edit states.
    match put_effect::traits_table(world) {
        None => {
            session.say("家具主表仍在加载，暂时不能选中家具。");
            return;
        }
        Some(traits) if traits.get(&item.fixture_id).is_some_and(|t| t.joint) => {
            session.say("围栏与道路属于各自的编辑分支，当前地面编辑不会改动它。");
            return;
        }
        Some(_) => {}
    }
    // `GetStackedFixtures`: a placed fixture carries the fixtures stacked on
    // it (a new or inventory fixture has none).
    let stacked = if origin == SelectionOrigin::Placed {
        match stacked_rows(world, session, &item.uid) {
            Ok(stacked) => stacked,
            Err(missing) => {
                session.say(format!("摆放检查的输入尚未就绪（{missing:?}），暂时不能选中家具。"));
                return;
            }
        }
    } else {
        Vec::new()
    };
    if !stacked.is_empty() {
        info!(
            "[edit-put] SelectFixture {}: stacked fixtures {:?} move with it",
            item.uid,
            stacked.iter().map(|row| row.uid.as_str()).collect::<Vec<_>>()
        );
    }
    // Selecting another item is an explicit Reset of an unfinished operation,
    // never an implicit Decide or an inventory debit.
    session.cancel_selection();
    // A table that did not load answers false (no store action).
    let can_clean_up = world
        .get_resource::<crate::entry::house::HomeFixtures>()
        .is_some_and(|homes| homes.can_clean_up(&item.package));
    let fixture_id = item.fixture_id;
    let put = (origin != SelectionOrigin::Placed).then(|| item.clone());
    session.selected = Some(Selection {
        item,
        stacked,
        origin,
        can_clean_up,
    });
    session.phase = EditPhase::Placing;
    session.changed();
    match put {
        // FloorEditState.PutFixture (a selector cell): the new fixture shows
        // the put effect with its put sound, then the focus; the pick sound
        // is not played.
        Some(item) => {
            put_effect::show(world, &item, "edit-put");
            focus(world, &item, "edit-put");
        }
        // FloorEditState.SelectFixture: the pick sound, the focus, then
        // (after the scale animation) FixtureController.PlayPutSound.
        None => {
            play_se(world, "se_pick_furniture", "edit-pick");
            if let Some(selected) = session.selected.as_ref() {
                let item = selected.item.clone();
                focus(world, &item, "edit-pick");
            }
            put_effect::play_put_sound(world, fixture_id, "edit-pick");
        }
    }
    session.say("已选中家具：拖动或方向键移动，R旋转，决定或取消。");
}

fn decide(session: &mut EditSession, world: &mut World) {
    let Some(selection) = session.selected.as_ref() else {
        return;
    };
    if session.baseline.is_none() {
        return;
    }
    // IsEnableDecideButton: CanPutFloor of the selection.
    let status = put_status(world, session, selection);
    if status != PutStatus::Ok {
        info!(
            "[edit-put] Decide {} refused: {}",
            selection.item.uid,
            validation::put_label(status)
        );
        session.say(format!(
            "这里不能放置：{}。草稿仍然保留。",
            validation::put_label(status)
        ));
        return;
    }
    let selection = session.selected.take().expect("checked selection");
    let decided = selection.item.clone();
    let stacked = selection.stacked.clone();
    match selection.origin {
        SelectionOrigin::Placed => {
            let Some(row) = session
                .rows
                .iter_mut()
                .find(|row| row.uid == selection.item.uid)
            else {
                session.selected = Some(selection);
                session.say("所选UID已不在当前草稿中，未把操作转到其它家具。");
                return;
            };
            *row = selection.item;
        }
        SelectionOrigin::Inventory => {
            let Some(index) = session
                .inventory
                .iter()
                .position(|row| row.uid == selection.item.uid)
            else {
                session.selected = Some(selection);
                session.say("所选UID已不在离线库存中，未复制或替换其它物品。");
                return;
            };
            session.inventory.remove(index);
            session.rows.push(selection.item);
        }
        SelectionOrigin::Mock => session.rows.push(selection.item),
    }
    // UpdateTileData: the stacked fixtures moved with the base keep their
    // new places too.
    for child in &stacked {
        if let Some(row) = session.rows.iter_mut().find(|row| row.uid == child.uid) {
            *row = child.clone();
        }
    }
    session.phase = EditPhase::Browsing;
    session.changed();
    // FloorEditState.OnClickDecideButton: ShowPutEffect (effect and put
    // sound), then the finish sound.
    put_effect::show(world, &decided, "edit-decide");
    play_se(world, "se_housing_finish", "edit-decide");
    // Publish(7, LayoutEditEventData(1, uid)): the edit camera's distance
    // restore.
    let block = put_effect::is_block(world, decided.fixture_id);
    crate::floor_edit_camera::decided(world, block, "edit-decide");
    session.say("已决定摆放，尚未保存。");
}

fn return_item(session: &mut EditSession) {
    let Some(selection) = session.selected.as_ref() else {
        return;
    };
    if selection.origin != SelectionOrigin::Placed {
        session.cancel_selection();
        session.say("已取消取出；库存没有减少，也没有复制新物品。");
        return;
    }
    if !selection.can_clean_up {
        // MysekaiFixtureUtility.CanCleanUp: the player's house and gates
        // stay placed; they can still be moved and rotated.
        session.say("玩家的家与大门不能收回库存，可以移动或旋转。");
        return;
    }
    let uid = selection.item.uid.clone();
    if session.inventory.iter().any(|item| item.uid == uid) {
        session.say("该UID已经在库存中，拒绝重复回收，原数据保持不变。");
        return;
    }
    let Some(index) = session.rows.iter().position(|row| row.uid == uid) else {
        session.say("未找到所选UID，未回收其它家具。");
        return;
    };
    // Return the owned original record, not a new catalog grant. The pending
    // moved pose is irrelevant to ownership and is not accidentally decided.
    let item = session.rows.remove(index);
    session.inventory.push(item);
    session.selected = None;
    session.phase = EditPhase::Browsing;
    session.changed();
    session.say("已回收到离线库存草稿；物品UID保留，保存后可在其它地图重新摆放。");
}

fn save(session: &mut EditSession, world: &mut World, exit_after: bool) {
    if session.phase == EditPhase::Idle {
        return;
    }
    if let Some(pending) = session.pending_reload.as_mut() {
        pending.exit_after |= exit_after;
        session.say("保存已完成，正在等待家具与可行走场恢复；角色继续暂停。");
        return;
    }
    if session.selected.is_some() {
        session.say("请先决定或取消当前家具，再保存。");
        return;
    }
    let Some(base) = session.baseline.as_ref() else {
        return;
    };
    let Some(current) = world.get_resource::<FixturePlacements>() else {
        return;
    };
    if current.site_id() != base.site_id()
        || current.site_type() != base.site_type()
        || current.editor_rows() != base.editor_rows()
    {
        session.say("当前地图布局已变化，未覆盖其它布局；本次草稿仍保留。");
        return;
    }
    let Some(areas) = world.get_resource::<FixtureAreas>() else {
        return;
    };
    let rules = tile_rules::rules(world, base.floor_grid());
    if let Err(error) = validation::save(
        &session.rows,
        base.floor_grid(),
        areas,
        rules.as_ref().map_err(|missing| *missing),
    ) {
        session.say(format!(
            "保存未完成：{error}。未自动回收家具，原布局与草稿均保留。"
        ));
        return;
    }
    let next = match base.with_editor_rows(&session.rows, session.next_fixture_uid) {
        Ok(next) => next,
        Err(error) => {
            session.say(format!("保存未完成：{error}。草稿仍保留。"));
            return;
        }
    };
    let Some(storage_baseline) = session.storage_baseline.as_ref() else {
        session.say("保存失败：缺少本次编辑的存档快照。布局、库存及退出窗口均未提交。");
        return;
    };
    let receipt = match crate::fixture::layouts::persist_edit(&next, &session.inventory, storage_baseline) {
        Ok(receipt) => receipt,
        Err(error) => {
            session.say(format!("保存失败：{error}。布局、库存及退出窗口均未提交。"));
            return;
        }
    };
    let next_storage_baseline = receipt.snapshot();
    // Only the successful durable receipt may publish either half of this
    // transaction. Existing settings and other site buckets are untouched.
    world.insert_resource(next.clone());
    if let Some(mut layouts) =
        world.get_resource_mut::<crate::fixture::layouts::SiteFixtureLayouts>()
    {
        layouts.did_save(&next, receipt);
    }
    session.baseline = Some(next);
    session.storage_baseline = Some(next_storage_baseline);
    session.initial_inventory = session.inventory.clone();
    session.exit_dialog = false;
    presentation::clear(world);
    crate::fixture::reload_after_save(world);
    world.write_message(LayoutSaved);
    // The normal loader/rebake runs on a later site-group turn. Finishing here
    // would release movement into the old WalkFace before LayoutSaved is read.
    session.pending_reload = Some(actors::wait_for_reload(world, true, exit_after));
    session.changed();
    session.say("已保存本地图及库存；正在恢复家具与可行走场，角色继续暂停。");
}

fn apply_command(world: &mut World, session: &mut EditSession, command: EditCommand) {
    if matches!(command, EditCommand::Enter) {
        begin(world, session);
        return;
    }
    if session.phase == EditPhase::Idle {
        return;
    }
    if let Some(pending) = session.pending_reload.as_mut() {
        if matches!(command, EditCommand::RequestExit | EditCommand::SaveAndExit) {
            pending.exit_after = true;
        }
        session.say("正在等待已保存场景恢复；本地图布局已保存，暂不接受新的摆放操作。");
        return;
    }
    if session.exit_dialog
        && !matches!(
            command,
            EditCommand::KeepEditing
                | EditCommand::Save
                | EditCommand::SaveAndExit
                | EditCommand::DiscardAndExit
                | EditCommand::RequestExit
        )
    {
        return;
    }
    match command {
        EditCommand::Enter => {}
        EditCommand::SelectCatalog { index } => {
            let Some(row) = CANDIDATES.get(index) else {
                session.say("未知的离线清单项。");
                return;
            };
            let Some(base) = session.baseline.as_ref() else {
                return;
            };
            let serial = session.next_fixture_uid;
            let Some(next_serial) = serial.checked_add(1) else {
                session.say("离线UID已耗尽。");
                return;
            };
            let uid = format!("offline-edit-{}-{serial}", base.site_id());
            if session
                .rows
                .iter()
                .chain(&session.inventory)
                .any(|item| item.uid == uid)
            {
                session.say("离线UID冲突，未覆盖已有物品。");
                return;
            }
            let mut item = EditableFixture { texture_id: 1,
                uid,
                package: row.package.to_owned(),
                fixture_id: row.fixture_id,
                center: GridPosition::ZERO,
                grid_size: row.grid_size,
                layout: layout_type::FLOOR,
                direction: Direction::Front,
            };
            if !place_new(world, session, &mut item, "catalog") {
                return;
            }
            session.next_fixture_uid = next_serial;
            session.catalog_index = index;
            select(session, item, SelectionOrigin::Mock, world);
        }
        EditCommand::SelectPlaced { uid } => {
            let Some(item) = session.rows.iter().find(|row| row.uid == uid).cloned() else {
                session.say("该UID不在当前地图，没有选择相近位置的其它家具。");
                return;
            };
            if session
                .selected
                .as_ref()
                .is_some_and(|selected| selected.item.uid == uid)
            {
                return;
            }
            select(session, item, SelectionOrigin::Placed, world);
        }
        EditCommand::SelectInventory { uid } => {
            let Some(mut item) = session.inventory.iter().find(|row| row.uid == uid).cloned()
            else {
                session.say("该UID不在离线库存，未生成替代物品。");
                return;
            };
            item.center = GridPosition::ZERO;
            if !place_new(world, session, &mut item, "inventory") {
                return;
            }
            select(session, item, SelectionOrigin::Inventory, world);
        }
        EditCommand::MoveTo { center } => drag(world, session, DragTarget::Touched(center)),
        EditCommand::Nudge { x, z } => drag(world, session, DragTarget::Step { x, z }),
        EditCommand::Rotate => rotate(world, session),
        EditCommand::Decide => decide(session, world),
        EditCommand::RotateCamera => crate::floor_edit_camera::rotate(world),
        EditCommand::ChangeLookCamera => crate::floor_edit_camera::change_look(world),
        EditCommand::Cancel => session.cancel_selection(),
        EditCommand::ReturnToInventory => return_item(session),
        EditCommand::Save => {
            let exit_after = session.exit_dialog;
            save(session, world, exit_after);
        }
        EditCommand::SaveAndExit => save(session, world, true),
        EditCommand::RequestExit => {
            if session.exit_dialog {
                session.exit_dialog = false;
                session.changed();
                return;
            }
            session.cancel_selection();
            if session.dirty() {
                session.exit_dialog = true;
                session.changed();
                session.say("还有未保存的修改：可继续编辑、保存退出，或明确放弃本次草稿。");
            } else {
                session.pending_reload = Some(actors::wait_for_reload(world, false, true));
                session.changed();
                session.say("正在恢复角色与可行走场。");
            }
        }
        EditCommand::KeepEditing => {
            session.exit_dialog = false;
            session.changed();
        }
        EditCommand::DiscardAndExit => {
            // Not a hotkey nor a site-switch fallback. Only the explicit exit
            // prompt offers this source action (ReloadPlayerLayoutDataAsync).
            if session.exit_dialog {
                presentation::clear(world);
                // The published layout/inventory never changed. Drop only
                // the explicit discarded draft while retaining the actor/input
                // lease until Show has a safe current navigation attachment.
                if let Some(base) = session.baseline.as_ref() { session.rows = base.editor_rows(); }
                session.inventory = session.initial_inventory.clone();
                session.selected = None;
                session.phase = EditPhase::Browsing;
                session.exit_dialog = false;
                session.pending_reload = Some(actors::wait_for_reload(world, false, true));
                session.changed();
                session.say("已放弃未保存草稿，正在恢复角色；原地图布局及库存未改动。");
            }
        }
    }
}

fn apply_commands(world: &mut World) {
    let pending = std::mem::take(&mut world.resource_mut::<PendingCommands>().0);
    if pending.is_empty() {
        return;
    }
    if world
        .get_resource::<crate::game_settings::SettingsPanel>()
        .is_some_and(|panel| panel.blocks_world_input())
    {
        return;
    }
    let Some(mut session) = world.remove_resource::<EditSession>() else {
        return;
    };
    for command in pending {
        apply_command(world, &mut session, command);
    }
    let was_active = world.resource::<EditSessionActive>().active;
    world.resource_mut::<EditSessionActive>().active = session.phase != EditPhase::Idle;
    if session.phase == EditPhase::Idle {
        presentation::clear(world);
        if was_active {
            game_state::exit(world);
        }
    }
    world.insert_resource(session);
}

/// Poll real readiness every frame, even when no input command arrives or a
/// settings panel is open. No fixed delay and no mutation of the new map are
/// used as substitutes for a completed rebuild.
fn advance_recovery(world: &mut World) {
    actors::maintain(world);
    let Some(mut session) = world.remove_resource::<EditSession>() else { return; };
    let readiness = session.pending_reload.as_ref().map(|pending| actors::reload_ready(world, pending));
    match readiness {
        Some(Ok(true)) => {
            let exit_after = session.pending_reload.as_ref().is_some_and(|pending| pending.exit_after);
            if exit_after {
                match actors::restore(world) {
                    Ok(()) => {
                        session.finish();
                        world.resource_mut::<EditSessionActive>().active = false;
                        presentation::clear(world);
                        game_state::exit(world);
                        session.say("场景与导航已恢复，已退出家具编辑。");
                    }
                    Err(error) => {
                        // The new scene is installed but has no safe actor
                        // attachment. Keep actors hidden while allowing the
                        // user to repair the layout instead of deadlocking all
                        // editor commands behind a completed reload.
                        session.pending_reload = None;
                        session.changed();
                        session.say(format!("场景已加载，但{error}；仍处于编辑，请调整家具后再退出。"));
                    }
                }
            } else {
                session.pending_reload = None;
                session.changed();
                session.say("本地图与库存已保存，家具和导航已恢复；仍处于编辑模式。");
            }
        }
        Some(Err(error)) => session.say(error),
        Some(Ok(false)) | None => {}
    }
    world.insert_resource(session);
}

fn publish_view(world: &mut World) {
    let (Some(session), Some(view)) = (
        world.get_resource::<EditSession>(),
        world.get_resource::<EditView>(),
    ) else {
        return;
    };
    if view.revision == session.revision {
        return;
    }
    let selected = session
        .selected
        .as_ref()
        .map(|selection| EditSelectionView {
            item: (&selection.item).into(),
            from_inventory: selection.origin == SelectionOrigin::Inventory,
            is_new_mock: selection.origin == SelectionOrigin::Mock,
            can_clean_up: selection.can_clean_up,
            put_status: put_status(world, session, selection),
        });
    let active = session.phase != EditPhase::Idle;
    // A fence or a road is edited by its own state (the floor editor's
    // touch skips it).
    let traits = put_effect::traits_table(world);
    let editable = |item: &EditableFixture| {
        let mut view = EditItemView::from(item);
        view.editable &= !traits
            .as_ref()
            .is_some_and(|traits| traits.get(&item.fixture_id).is_some_and(|t| t.joint));
        view
    };
    let next = EditView {
        revision: session.revision,
        active,
        phase: session.phase,
        site_id: session
            .baseline
            .as_ref()
            .map_or(0, FixturePlacements::site_id),
        site_type: session
            .baseline
            .as_ref()
            .map_or_else(String::new, |rows| rows.site_type().into()),
        dirty: session.dirty(),
        exit_dialog: session.exit_dialog,
        selected,
        placed_rows: session.rows.iter().map(&editable).collect(),
        inventory: session.inventory.iter().map(&editable).collect(),
        catalog: CANDIDATES
            .iter()
            .enumerate()
            .map(|(index, row)| EditCatalogView {
                index,
                package: row.package,
                fixture_id: row.fixture_id,
                grid_size: row.grid_size,
                unlimited_mock: true,
            })
            .collect(),
        can_save: active && session.selected.is_none() && session.pending_reload.is_none(),
        feedback: session.feedback.clone(),
    };
    world.insert_resource(next);
}

pub struct FixtureEditPlugin;

impl Plugin for FixtureEditPlugin {
    fn build(&self, app: &mut App) {
        edit_grid::install(app);
        app.init_resource::<EditSession>()
            .init_resource::<EditSessionActive>()
            .init_resource::<EditView>()
            .init_resource::<PendingCommands>()
            .add_message::<EditCommand>()
            .add_systems(
                Startup,
                (
                    assets::load,
                    put_effect::request_tables,
                    tile_rules::request_zones,
                ),
            )
            .add_systems(
                Update,
                (assets::parse_areas, assets::plan_candidates).chain(),
            )
            .add_systems(Update, tile_rules::read_zones)
            .add_systems(
                Update,
                read_keyboard
                    .in_set(FixtureEditSystems::Input)
                    .run_if(crate::game_settings::scene_input_enabled),
            )
            // Root places Pointer after UI consumption, with no back-edge to
            // FixtureLayoutSet. Its small adapter emits the same EditCommand.
            .add_systems(
                Update,
                read_pointer
                    .in_set(FixtureEditSystems::Pointer)
                    .run_if(crate::game_settings::scene_input_enabled),
            )
            .add_systems(
                Update,
                (receive_commands, apply_commands, advance_recovery)
                    .chain()
                    .in_set(FixtureEditSystems::Commands)
                    .after(FixtureEditSystems::Input)
                    .after(FixtureEditSystems::Pointer)
                    .before(crate::audio::SeDrainSet::Drain),
            )
            .add_systems(
                Update,
                (presentation::sync_visuals, publish_view)
                    .chain()
                    .in_set(FixtureEditSystems::View)
                    .after(FixtureEditSystems::Commands),
            )
            .add_systems(
                Update,
                autoplay::autoplay
                    .before(FixtureEditSystems::Commands)
                    .run_if(crate::game_settings::scene_input_enabled),
            )
            .add_systems(
                Update,
                put_effect::advance
                    .after(FixtureEditSystems::Commands)
                    .before(crate::audio::SeDrainSet::Drain),
            )
            .add_systems(
                Update,
                crate::floor_edit_camera::input
                    .after(FixtureEditSystems::Commands)
                    .run_if(crate::game_settings::camera_input_enabled),
            )
            .add_systems(
                PostUpdate,
                (
                    crate::floor_edit_camera::sample.after(crate::camera::follow_avatar),
                    game_state::sample_tweets
                        .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
                ),
            )
            .add_systems(
                PostUpdate,
                // Update's after-edit/talk producers run after editor Commands.
                // Emoticon's explicit showcase can also create draws in
                // PostUpdate. Flush those exact producers before the same
                // visibility owner captures roots, then let Bevy propagate it.
                (bevy::prelude::ApplyDeferred, actors::maintain_detached_views)
                    .chain()
                    .after(crate::balloon::place)
                    .after(crate::emoticon::advance)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}
