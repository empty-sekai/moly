//! Ground fixture editing with one command boundary and a private map draft.
//!
//! Source: FloorEditState.SelectFixture snapshots StartPosition/StartDirection;
//! Reset restores them (or removes a pre-placement preview); Decide updates
//! tile/layout data before unselecting. SiteLayoutEditor keeps an explicit
//! Cancel / Save / Discard exit dialog. The offline storage envelope is a
//! product-owned input, not real account data. The fixture list offers the
//! owned fixtures with their placeable counts (`owned`); a save posts the
//! site's layout to the server model (`PostUserMysekaiHousingLayoutApi`)
//! before the offline envelope is written.
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
mod environment;
pub mod field_build;
mod game_state;
mod input;
mod layout_rules;
mod owned;
mod presentation;
mod put_effect;
mod road;
mod validation;

#[cfg(test)]
mod tests;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture::{EditableFixture, FixturePlacements, OccupancyRow};
use crate::userdata::{Answered, ServerPending};
use assets::FixtureAreas;
use bevy::prelude::*;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition, Vector3Int};
use moly_layout::{resolve as placement, validate as tile_rules};
use moly_link::ServerLink;
use moly_protocol::{Call, Refusal, Reply};

pub use input::{read_keyboard, read_pointer};
pub use put_effect::site_origin;
pub(crate) use put_effect::{play_put_sound, show as show_put_effect};

pub use crate::field_modes::{EditSessionActive, LayoutSaved};

/// UI and platform input may request operations, never mutate draft fields.
pub use crate::ui_inputs::{
    EditCatalogView, EditCommand, EditContentType, EditEnvironmentView, EditItemView, EditPhase,
    EditSelectionView, EditView, PutStatus,
};

/// The view row of a placed or held fixture.
fn item_view(item: &EditableFixture) -> EditItemView {
    EditItemView {
        uid: item.uid.clone(),
        piece: None,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionOrigin {
    Placed,
    Inventory,
    /// A new placement of an owned fixture.
    Owned,
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
pub struct EditSession {
    road: road::RoadSession,
    /// Retain road pointer ownership through the frame containing its release.
    road_camera_drag_claimed: bool,
    next_fixture_uid: u64,
    revision: u64,
    phase: EditPhase,
    catalog_index: usize,
    content: EditContentType,
    environment: Option<i32>,
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
    clean_up_dialog: bool,
    /// The dialog a refused save shows (`SaveLayoutWithValidation`), while
    /// it is shown; the editor takes no command until it is answered.
    save_dialog: Option<SaveDialog>,
    /// The layout-save call whose answer has not arrived.
    saving: Option<ServerPending>,
    /// The layout the waiting save sent.
    saving_layout: Option<FixturePlacements>,
    /// The waiting save was asked from the exit prompt.
    saving_exit_after: bool,
    feedback: String,
}

impl EditSession {
    /// Input ownership is read after commands in the same frame so a road
    /// drag never also pans the field camera before the view is republished.
    pub(crate) fn road_dragging(&self) -> bool {
        self.road.dragging || self.road_camera_drag_claimed
    }
}

/// The dialogs of `SiteLayoutEditor.SaveLayoutWithValidation`.
#[derive(Clone, Debug)]
struct SaveDialog {
    id: crate::ui_layers::DialogId,
    kind: SaveDialogKind,
    /// The save was asked from the exit prompt.
    exit_after: bool,
}

#[derive(Clone, Debug)]
enum SaveDialogKind {
    /// `ShowCutsceneAreaConfirmDialog`: OK saves without the cutscene check.
    CutsceneArea,
    /// `ShowConfirmRemoveInValidFixtureDialog`: OK stores the listed
    /// fixtures and saves.
    RemoveInvalid(Vec<String>),
    /// `ShowErrorGoToHomeDialog`: the house or the gate cannot stay; OK
    /// reloads the saved layout.
    GoToHome,
}

const CUTSCENE_CONFIRM: &str = "MSG_MYSEKAI_BIRTHDAY_PARTY_SAVE_CONFIRMATION";
const INVALID_GO_HOME: &str = "WORD_MYSEKAI_INVALID_LAYOUT";
const INVALID_REMOVE: &str = "MSG_MYSEKAI_CONFIRM_REMOVE_FIXTURE_INVALID_LAYOUT";
const INVALID_REMOVE_OK: &str = "WORD_MYSEKAI_INVALID_FIXTURE_LIST_OK_BUTTON";

/// The wordings of the save dialogs (the shell charset takes them).
pub const WORDINGS: &[&str] = &[
    CUTSCENE_CONFIRM,
    INVALID_GO_HOME,
    INVALID_REMOVE,
    INVALID_REMOVE_OK,
];

impl EditSession {
    /// The server call the editor waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub fn server_pending(&self) -> Option<&ServerPending> {
        self.saving.as_ref()
    }

    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("editor revision exhausted");
    }

    fn say(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.feedback != message {
            info!("[edit-feedback] {message}");
            self.feedback = message;
            self.changed();
        }
    }

    fn dirty(&self) -> bool {
        self.baseline.as_ref().is_some_and(|base| {
            self.rows != base.editor_rows()
                || self.inventory != self.initial_inventory
                || self.environment != base.phenomena_id()
        })
    }

    fn cancel_selection(&mut self) {
        if self.road.cancel() {
            self.changed();
        }
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
        self.road = road::RoadSession::default();
        self.exit_dialog = false;
        self.clean_up_dialog = false;
        self.saving = None;
        self.saving_layout = None;
        self.saving_exit_after = false;
        self.changed();
    }
}

#[derive(Resource, Default)]
struct PendingCommands(Vec<EditCommand>);

/// Input is the existing early keyboard-producer set. The root schedule owns
/// the late UI/pointer ordering; do not make the early layout loader depend on
/// Commands, because the site's loader precedes the field UI input chain.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FixtureEditSystems {
    Input,
    Pointer,
    Commands,
    View,
}

pub fn has_unsaved_layout(session: &EditSession) -> bool {
    session.dirty()
        || session.selected.is_some()
        || road_pending(session)
        || session.pending_reload.is_some()
}

fn road_pending(session: &EditSession) -> bool {
    !session.road.planned.is_empty()
}

/// Draft occupancy never enters the gameplay walk-field or claims playable
/// fixture geometry. Commit emits LayoutSaved and the normal loader rebakes
/// from FixturePlacements. Editing owns input and pauses new activity admission.
pub fn decided_footprints(_: &EditSession) -> Vec<(GridPosition, GridPosition, i8, u8)> {
    Vec::new()
}

pub fn decided_scene_rows(_: &EditSession, _: &str) -> Vec<OccupancyRow> {
    Vec::new()
}

pub fn clear_for_site_change(world: &mut World) {
    if let Some(mut session) = world.get_resource_mut::<EditSession>() {
        session.finish();
    }
    if let Some(mut active) = world.get_resource_mut::<EditSessionActive>() {
        active.active = false;
    }
    world.remove_resource::<crate::field_modes::LayoutEnvironmentPreview>();
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
        se.0.push(SeRequest {
            owner: None,
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
    let homes = world.get_resource::<crate::house::HomeFixtures>();
    let entry = world
        .get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
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
    session.content = if layout.site_type() == "home_site" {
        EditContentType::Outdoor
    } else {
        EditContentType::Floor
    };
    session.environment = layout.phenomena_id();
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
    environment::preview(world, session);
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
    let rules = match layout_rules::rules(world, floor) {
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
            warn!(
                "[edit-camera] focus on {} ({source}): fixture {} has no handle type in the fixture table; focused without zoom support",
                item.uid, item.fixture_id
            );
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
    let Some(camera) = world
        .get_resource::<crate::camera::FieldCameraModel>()
        .map(|model| (model.look_at, model.yaw))
    else {
        session.say("相机尚未就绪，暂时不能摆放新家具。");
        return false;
    };
    let (look_at, yaw) = camera;
    match put_search(world, &session.rows, floor, item, look_at, PutAim { facing: Facing::Yaw(yaw), exact: false }, source) {
        Ok(()) => true,
        Err(message) => {
            session.say(message);
            false
        }
    }
}

/// Where a new fixture's direction comes from.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Facing {
    /// The camera yaw (`RotationToDirection`).
    Yaw(f32),
    Fixed(moly_law::fixture::Direction),
}

/// How a new fixture's direction and tile are taken: `PutFixture` takes the
/// direction from the camera yaw and the tile from the spiral around the
/// look-at; an exact aim tries only the cell the look-at falls in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PutAim {
    pub facing: Facing,
    pub exact: bool,
}

/// The put search of a new fixture (`PutFixture`): its direction from the
/// aim's yaw and its tile from the spiral around `look_at` (a world point) on
/// the tile data of `rows`; an exact aim tries only the cell `look_at` falls
/// in. `Err` is the message the editor shows; `item` is unchanged then.
fn put_search(
    world: &mut World,
    rows: &[EditableFixture],
    floor: crate::site::FloorGridLayout,
    item: &mut EditableFixture,
    look_at: Vec3,
    aim: PutAim,
    source: &'static str,
) -> Result<(), String> {
    let put_type = match put_effect::put_type(world, item.fixture_id) {
        Ok(put_type) => put_type,
        Err(error) => {
            warn!(
                "[edit-put] PutFixture {} ({source}): {error}; not put",
                item.uid
            );
            return Err("家具主表仍在加载，暂时不能摆放新家具。".into());
        }
    };
    let (source_direction, yaw) = match aim.facing {
        Facing::Yaw(yaw) => {
            let Some(direction) = placement::rotation_to_direction(yaw) else {
                warn!(
                    "[edit-put] PutFixture {} ({source}): camera yaw {yaw} names no direction; not put",
                    item.uid
                );
                return Err("相机朝向无效，暂时不能摆放新家具。".into());
            };
            (direction, yaw)
        }
        Facing::Fixed(direction) => (direction, f32::from(direction as u8) * 90.0),
    };
    // The look-at relative to the site, in the source frame (X mirrored).
    let relative = look_at - site_origin(world);
    let relative = Vec3::new(-relative.x, relative.y, relative.z);
    let board = match layout_rules::rules(world, Some(floor))
        .and_then(|rules| Ok((tile_rules::Board::new(rows, floor, &rules)?, rules)))
    {
        Ok(found) => found,
        Err(missing) => {
            warn!(
                "[edit-put] PutFixture {} ({source}): the put check's input {missing:?} is missing; not put",
                item.uid
            );
            return Err(format!(
                "摆放检查的输入尚未就绪（{missing:?}），暂时不能摆放新家具。"
            ));
        }
    };
    let (board, rules) = board;
    let found = if aim.exact {
        placement::place_at(item, source_direction, relative.to_array(), &board, &rules)
    } else {
        placement::place(
            item,
            source_direction,
            relative.to_array(),
            &board,
            &rules,
            put_type.as_deref(),
        )
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, reason) in &found.rejected {
        let label = placement::reject_label(reason);
        match counts.iter_mut().find(|(known, _)| *known == label) {
            Some((_, n)) => *n += 1,
            None => counts.push((label, 1)),
        }
    }
    info!(
        "[edit-put] PutFixture {} (fixture {}, {source}): camera look-at {:?} (source frame, site-relative) -> grid ({}, {}, {}) -> start tile ({}, {}, {}); camera yaw {yaw:.2} -> direction {:?}; put type {:?} ({}); radius cap {}; {} candidates rejected {:?}",
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
        if found.search_raised {
            "raised rings searched"
        } else {
            "rings at y 0"
        },
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
        warn!(
            "[edit-put] PutFixture {} ({source}): no placeable tile within the radius cap; CantPutFixture",
            item.uid
        );
        if let (true, Some((tile, reason))) = (aim.exact, found.rejected.first()) {
            return Err(format!(
                "the cell ({}, {}, {}) cannot take the fixture: {}",
                tile.x,
                tile.y,
                tile.z,
                placement::reject_label(reason)
            ));
        }
        return Err("附近没有可以摆放的空间。".into());
    };
    item.center = center;
    item.direction = direction;
    match item.footprint() {
        Ok((min, max)) => info!(
            "[edit-put] PutFixture {} ({source}): tile ({}, {}, {}) (source frame) -> product center ({}, {}, {}) direction {direction:?}; footprint ({}, {}, {})..=({}, {}, {})",
            item.uid,
            tile.x,
            tile.y,
            tile.z,
            center.x,
            center.y,
            center.z,
            min.x,
            min.y,
            min.z,
            max.x,
            max.y,
            max.z
        ),
        Err(error) => warn!(
            "[edit-put] PutFixture {} ({source}): chosen tile has no footprint ({error})",
            item.uid
        ),
    }
    Ok(())
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
    let rules = layout_rules::rules(world, floor)?;
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
    let rules = layout_rules::rules(world, floor);
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
    let prepared = layout_rules::rules(world, floor).and_then(|rules| {
        let floor = floor.ok_or(tile_rules::Missing::FloorGrid)?;
        let board = tile_rules::Board::new(&session.rows, floor, &rules)?;
        let piece = tile_rules::Piece::new(&selected.item, &rules)?;
        Ok((rules, board, piece))
    });
    let (rules, board, piece) = match prepared {
        Ok(prepared) => prepared,
        Err(missing) => {
            session.say(format!(
                "摆放检查的输入尚未就绪（{missing:?}），未移动家具。"
            ));
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
    let prepared = layout_rules::rules(world, floor).and_then(|rules| {
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
            session.say(format!(
                "摆放检查的输入尚未就绪（{missing:?}），未旋转家具。"
            ));
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
    let turned = match tile_rules::Piece::at(
        &selected.item,
        &rules,
        piece.center,
        direction,
        piece.layout,
    ) {
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
                session.say(format!(
                    "摆放检查的输入尚未就绪（{missing:?}），暂时不能选中家具。"
                ));
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
            stacked
                .iter()
                .map(|row| row.uid.as_str())
                .collect::<Vec<_>>()
        );
    }
    // Selecting another item is an explicit Reset of an unfinished operation,
    // never an implicit Decide or an inventory debit.
    session.cancel_selection();
    // A table that did not load answers false (no store action).
    let can_clean_up = world
        .get_resource::<crate::house::HomeFixtures>()
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
        SelectionOrigin::Owned => session.rows.push(selection.item),
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
    // FloorEditState.RemoveFixture: the fixtures stacked on it go to storage
    // with it (GetStackFixtureUidList, recursively).
    let stacked: Vec<String> = selection
        .stacked
        .iter()
        .map(|row| row.uid.clone())
        .collect();
    if std::iter::once(&uid)
        .chain(&stacked)
        .any(|uid| session.inventory.iter().any(|item| &item.uid == uid))
    {
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
    for child in &stacked {
        if let Some(index) = session.rows.iter().position(|row| &row.uid == child) {
            let row = session.rows.remove(index);
            session.inventory.push(row);
        }
    }
    if !stacked.is_empty() {
        info!(
            "[edit-put] RemoveFixture {uid}: the stacked fixtures {stacked:?} go to storage with it"
        );
    }
    session.selected = None;
    session.phase = EditPhase::Browsing;
    session.changed();
    session.say("已回收到离线库存草稿；物品UID保留，保存后可在其它地图重新摆放。");
}

/// `SiteLayoutEditor.CleanUpLayout` in the floor edit state
/// (`FloorEditState.RemoveFixtureAll`): the selection is dropped, then every
/// fixture of the floor and rug grids that `IsRemoveTarget` passes (not a
/// fence or road, `CanCleanUp`) goes to storage, with what stands on it.
fn clean_up_all(world: &World, session: &mut EditSession) {
    let (Some(homes), Some(traits)) = (
        world.get_resource::<crate::house::HomeFixtures>(),
        put_effect::traits_table(world),
    ) else {
        session.say("家具主表仍在加载，未清理家具。");
        return;
    };
    session.cancel_selection();
    let (mut kept_house, mut kept_joint, mut moved) = (0usize, 0usize, Vec::new());
    let mut index = 0;
    while index < session.rows.len() {
        let row = &session.rows[index];
        let in_grids = row.layout == layout_type::FLOOR || row.layout == layout_type::RUG;
        let joint = traits.get(&row.fixture_id).is_some_and(|t| t.joint);
        let can_clean_up = homes.can_clean_up(&row.package);
        if !in_grids {
            index += 1;
            continue;
        }
        if joint {
            kept_joint += 1;
            index += 1;
            continue;
        }
        if !can_clean_up {
            kept_house += 1;
            index += 1;
            continue;
        }
        if session.inventory.iter().any(|item| item.uid == row.uid) {
            index += 1;
            continue;
        }
        let row = session.rows.remove(index);
        moved.push(row.uid.clone());
        session.inventory.push(row);
    }
    info!(
        "[edit-put] RemoveFixtureAll (floor and rug grids): {} fixtures to storage {:?}; kept {} that CanCleanUp refuses, {} fences or roads; {} rows remain",
        moved.len(),
        moved,
        kept_house,
        kept_joint,
        session.rows.len()
    );
    session.changed();
    session.say(format!(
        "已清理 {} 件家具到离线库存草稿；家与大门保留。保存后生效。",
        moved.len()
    ));
}

fn save(session: &mut EditSession, world: &mut World, exit_after: bool, validate_cutscene: bool) {
    if session.phase == EditPhase::Idle || session.save_dialog.is_some() {
        return;
    }
    if session.saving.is_some() {
        return;
    }
    if let Some(pending) = session.pending_reload.as_mut() {
        pending.exit_after |= exit_after;
        session.say("保存已完成，正在等待家具与可行走场恢复；角色继续暂停。");
        return;
    }
    if session.selected.is_some() || road_pending(session) {
        session.say("请先决定或取消当前家具或道路规划，再保存。");
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
        || current.phenomena_id() != base.phenomena_id()
    {
        session.say("当前地图布局已变化，未覆盖其它布局；本次草稿仍保留。");
        return;
    }
    let Some(areas) = world.get_resource::<FixtureAreas>() else {
        return;
    };
    let rules = layout_rules::rules(world, base.floor_grid());
    if let Err((status, error)) = validation::save(
        &session.rows,
        base.floor_grid(),
        areas,
        rules.as_ref().map_err(|missing| *missing),
        validate_cutscene,
    ) {
        let floor = base.floor_grid();
        refused_save(
            world,
            session,
            status,
            &error,
            floor,
            rules.as_ref().ok(),
            exit_after,
        );
        return;
    }
    let next = match base
        .with_editor_rows(&session.rows, session.next_fixture_uid)
        .and_then(|layout| layout.with_phenomena_id(session.environment))
    {
        Ok(next) => next,
        Err(error) => {
            session.say(format!("保存未完成：{error}。草稿仍保留。"));
            return;
        }
    };
    if session.storage_baseline.is_none() {
        session.say("保存失败：缺少本次编辑的存档快照。布局、库存及退出窗口均未提交。");
        return;
    }
    // PostUserMysekaiHousingLayoutApi first: the saved copy is written only
    // once the server has accepted the layout; a refusal keeps the draft.
    let call = Call::HousingLayout(crate::fixture::layouts::housing_layout_request(&next));
    let kind = call.kind();
    let seq = world.resource_mut::<ServerLink>().call(call);
    info!(
        "[edit-save] PostUserMysekaiHousingLayoutApi sent as seq {seq}; the draft is written on its reply"
    );
    session.saving = Some(ServerPending {
        seq: Some(seq),
        call: kind,
    });
    session.saving_layout = Some(next);
    session.saving_exit_after = exit_after;
}

/// The layout save's answer.
fn read_save_answer(
    mut answers: MessageReader<Answered>,
    session: Option<Res<EditSession>>,
    mut commands: Commands,
) {
    for answer in answers.read() {
        let Some(pending) = session.as_deref().and_then(EditSession::server_pending) else {
            continue;
        };
        if pending.seq != Some(answer.seq) {
            continue;
        }
        let outcome = answer.outcome.clone();
        commands.queue(move |world: &mut World| finish_save(world, outcome));
    }
}

/// Continues the save: an accepted layout is written and published, a
/// refusal keeps the draft.
fn finish_save(world: &mut World, outcome: Result<Reply, Refusal>) {
    let Some(mut session) = world.remove_resource::<EditSession>() else {
        return;
    };
    if session.saving.take().is_none() {
        world.insert_resource(session);
        return;
    }
    let Some(next) = session.saving_layout.take() else {
        world.insert_resource(session);
        return;
    };
    let exit_after = std::mem::take(&mut session.saving_exit_after);
    match outcome {
        Ok(Reply::SuiteUser(reply)) if reply.success => {
            commit_saved_layout(world, &mut session, next, exit_after);
        }
        Ok(Reply::SuiteUser(reply)) => {
            session.say(format!(
                "保存失败：服务器拒绝了布局（{}）。布局、库存及退出窗口均未提交，草稿仍保留。",
                reply.refusal.unwrap_or_default()
            ));
        }
        Err(refusal) => {
            session.say(format!(
                "保存失败：服务器拒绝了布局（{}）。布局、库存及退出窗口均未提交，草稿仍保留。",
                refusal.reason
            ));
        }
        Ok(other) => {
            session.say(format!(
                "保存失败：服务器回复了意外的 {other:?}。布局、库存及退出窗口均未提交，草稿仍保留。"
            ));
        }
    }
    world.insert_resource(session);
}

/// Writes and publishes the accepted layout; a local failure keeps the draft.
fn commit_saved_layout(
    world: &mut World,
    session: &mut EditSession,
    next: FixturePlacements,
    exit_after: bool,
) {
    let Some(storage_baseline) = session.storage_baseline.as_ref() else {
        return;
    };
    let receipt =
        match crate::fixture::layouts::persist_edit(&next, &session.inventory, storage_baseline) {
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
    let previous = session
        .baseline
        .as_ref()
        .map(|layout| layout.editor_rows())
        .unwrap_or_default();
    let updated: Vec<String> = next
        .editor_rows()
        .into_iter()
        .filter(|row| previous.iter().find(|before| before.uid == row.uid) != Some(row))
        .map(|row| row.uid)
        .collect();
    world.init_resource::<crate::npc_objective::events::LayoutReactionFixtures>();
    world
        .resource_mut::<crate::npc_objective::events::LayoutReactionFixtures>()
        .pending
        .push(updated);
    session.baseline = Some(next);
    session.storage_baseline = Some(next_storage_baseline);
    session.initial_inventory = session.inventory.clone();
    session.exit_dialog = false;
    // Newly placed fixtures are loaded only by the preview so far.
    let previews: Vec<_> = world
        .get_resource::<assets::CandidateAssets>()
        .map(|candidates| candidates.glbs.values().cloned().collect())
        .unwrap_or_default();
    crate::fixture::retain_sources_until_swapped(world, previews);
    presentation::clear(world);
    crate::fixture::reload_after_save(world);
    world.write_message(LayoutSaved);
    // The normal loader/rebake runs on a later site-group turn. Finishing here
    // would release movement into the old WalkFace before LayoutSaved is read.
    session.pending_reload = Some(actors::wait_for_reload(world, true, exit_after));
    session.changed();
    session.say("已保存本地图及库存；正在恢复家具与可行走场，角色继续暂停。");
}

/// `SaveLayoutWithValidation` after `CheckSaveLayout` refused: status 3
/// asks to save over the cutscene area; any other verdict lists the invalid
/// fixtures (`CreateInValidFixtureList`) and either asks to store them or,
/// when the house or the gate is on the list, sends the player home.
fn refused_save(
    world: &mut World,
    session: &mut EditSession,
    status: validation::LayoutStatus,
    error: &str,
    floor: Option<crate::site::FloorGridLayout>,
    rules: Option<&moly_layout::Rules>,
    exit_after: bool,
) {
    use crate::common_dialog::Buttons;
    use validation::LayoutStatus;
    let (caller, body, buttons, kind) = match status {
        LayoutStatus::Unavailable => {
            session.say(format!("保存未完成：{error}。原布局与草稿均保留。"));
            return;
        }
        LayoutStatus::CutsceneAreaOverridden => (
            "SiteLayoutEditor.ShowCutsceneAreaConfirmDialog",
            CUTSCENE_CONFIRM,
            Buttons::Two {
                ok: "WORD_OK",
                cancel: "WORD_CANCEL",
            },
            SaveDialogKind::CutsceneArea,
        ),
        LayoutStatus::ErrorLayout | LayoutStatus::ErrorAnimationSpace => {
            let homes = world.get_resource::<crate::house::HomeFixtures>();
            let listed = match (floor, rules, homes) {
                (Some(floor), Some(rules), Some(homes)) => {
                    validation::invalid_fixtures(&session.rows, floor, rules, |package| {
                        !homes.can_clean_up(package)
                    })
                }
                _ => Err("家具主表或摆放规则仍在加载".into()),
            };
            let listed = match listed {
                Ok(listed) if !listed.is_empty() => listed,
                Ok(_) => {
                    session.say(format!(
                        "保存未完成：{error}；无效家具列表为空。原布局与草稿均保留。"
                    ));
                    warn!(
                        "[edit-save] CheckSaveLayout refused ({error}) but CreateInValidFixtureList found nothing"
                    );
                    return;
                }
                Err(reason) => {
                    session.say(format!(
                        "保存未完成：{error}；{reason}。原布局与草稿均保留。"
                    ));
                    return;
                }
            };
            let homes = world.get_resource::<crate::house::HomeFixtures>();
            let house_or_gate = session.rows.iter().any(|row| {
                listed.contains(&row.uid)
                    && homes.is_some_and(|homes| !homes.can_clean_up(&row.package))
            });
            info!(
                "[edit-save] CreateInValidFixtureList: {listed:?} (house or gate listed: {house_or_gate})"
            );
            if house_or_gate {
                (
                    "SiteLayoutEditor.ShowErrorGoToHomeDialog",
                    INVALID_GO_HOME,
                    Buttons::One { ok: "WORD_OK" },
                    SaveDialogKind::GoToHome,
                )
            } else {
                // The source shows the list in its own dialog
                // (InValidLayoutFixtureListDialog); the common dialog shows
                // its message and buttons, and the log names the fixtures.
                (
                    "SiteLayoutEditor.ShowConfirmRemoveInValidFixtureDialog",
                    INVALID_REMOVE,
                    Buttons::Two {
                        ok: INVALID_REMOVE_OK,
                        cancel: "WORD_CANCEL",
                    },
                    SaveDialogKind::RemoveInvalid(listed),
                )
            }
        }
    };
    let text = crate::common_dialog::wording(world, caller, body);
    let Some(id) = crate::common_dialog::open(world, caller, text, buttons, None) else {
        session.say(format!(
            "保存未完成：{error}；确认窗口无法显示。原布局与草稿均保留。"
        ));
        return;
    };
    info!("[edit-save] CheckSaveLayout: {error}; {caller} ({body}) {id:?}");
    session.save_dialog = Some(SaveDialog {
        id,
        kind,
        exit_after,
    });
    session.changed();
}

/// The save dialog's answer, once it has one.
fn answer_save_dialog(world: &mut World) {
    let Some(dialog) = world
        .get_resource::<EditSession>()
        .and_then(|session| session.save_dialog.clone())
    else {
        return;
    };
    let Some(ok) = crate::common_dialog::answer(world, dialog.id) else {
        return;
    };
    let Some(mut session) = world.remove_resource::<EditSession>() else {
        return;
    };
    session.save_dialog = None;
    session.changed();
    info!(
        "[edit-save] {:?} answered {}",
        dialog.kind,
        if ok { "OK" } else { "cancel" }
    );
    match (dialog.kind, ok) {
        (_, false) => session.say("已取消保存；草稿仍保留。"),
        (SaveDialogKind::CutsceneArea, true) => save(&mut session, world, dialog.exit_after, false),
        (SaveDialogKind::RemoveInvalid(listed), true) => {
            // RemoveInValidFixture: the listed fixtures go to storage.
            let mut index = 0;
            while index < session.rows.len() {
                if listed.contains(&session.rows[index].uid) {
                    let row = session.rows.remove(index);
                    if !session.inventory.iter().any(|item| item.uid == row.uid) {
                        session.inventory.push(row);
                    }
                } else {
                    index += 1;
                }
            }
            info!("[edit-save] RemoveInValidFixture: {listed:?} to storage; saving");
            save(&mut session, world, dialog.exit_after, true);
        }
        (SaveDialogKind::GoToHome, true) => {
            // RequestScene(home): the saved layout is shown again and the
            // draft is dropped.
            presentation::clear(world);
            if let Some(base) = session.baseline.as_ref() {
                session.rows = base.editor_rows();
            }
            session.environment = session
                .baseline
                .as_ref()
                .and_then(FixturePlacements::phenomena_id);
            environment::preview(world, &session);
            session.inventory = session.initial_inventory.clone();
            session.selected = None;
            session.phase = EditPhase::Browsing;
            session.exit_dialog = false;
            session.pending_reload = Some(actors::wait_for_reload(world, false, true));
            session.say("布局无效：已恢复保存的布局。");
        }
    }
    world.insert_resource(session);
}

fn apply_command(world: &mut World, session: &mut EditSession, command: EditCommand) {
    if session.saving.is_some() {
        session.say("正在等待服务器保存回复，暂不接受新的操作。");
        return;
    }
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
    if road::handle(world, session, &command) {
        return;
    }
    match command {
        EditCommand::RoadTap { .. }
        | EditCommand::RoadLongStart { .. }
        | EditCommand::RoadLongEnd
        | EditCommand::RoadDragStart { .. }
        | EditCommand::RoadDragMove { .. }
        | EditCommand::RoadDragEnd
        | EditCommand::RoadPointerCancel
        | EditCommand::RoadSelectMode
        | EditCommand::RoadPlanningMode
        | EditCommand::RoadToggleFixtures => {}
        EditCommand::Enter => {}
        EditCommand::SelectContent(content) => environment::select_content(world, session, content),
        EditCommand::SelectEnvironment { phenomena_id } => {
            environment::select(world, session, phenomena_id)
        }
        EditCommand::SelectCatalog { index } => {
            if session.content == EditContentType::Environment {
                return;
            }
            let Some(base) = session.baseline.as_ref() else {
                return;
            };
            let site_id = base.site_id();
            let row = match owned::catalog_for(
                world,
                site_id,
                &session.rows,
                session.content == EditContentType::Road,
            ) {
                Ok((rows, _)) => rows.into_iter().nth(index),
                Err(error) => {
                    session.say(format!("持有家具清单不可用：{error}。"));
                    return;
                }
            };
            let Some(row) = row else {
                session.say("该持有家具已没有可摆放的数量。");
                return;
            };
            // A stored fixture of this row keeps its UID: it is taken first.
            if let Some(stored) = session
                .inventory
                .iter()
                .find(|item| item.fixture_id == row.fixture_id && item.texture_id == row.texture_id)
                .map(|item| item.uid.clone())
            {
                session.catalog_index = index;
                apply_command(world, session, EditCommand::SelectInventory { uid: stored });
                return;
            }
            let serial = session.next_fixture_uid;
            let Some(next_serial) = serial.checked_add(1) else {
                session.say("离线UID已耗尽。");
                return;
            };
            let uid = format!("offline-edit-{site_id}-{serial}");
            if session
                .rows
                .iter()
                .chain(&session.inventory)
                .any(|item| item.uid == uid)
            {
                session.say("离线UID冲突，未覆盖已有物品。");
                return;
            }
            let mut item = row.fixture(uid);
            if !place_new(world, session, &mut item, "owned") {
                return;
            }
            info!(
                "[edit-owned] SelectCatalog {index}: fixture {} texture {} (quantity {}, layouted {}, placeable {}) -> new placement {}",
                row.fixture_id,
                row.texture_id,
                row.quantity,
                row.layouted,
                row.placeable(),
                item.uid
            );
            session.next_fixture_uid = next_serial;
            session.catalog_index = index;
            select(session, item, SelectionOrigin::Owned, world);
        }
        EditCommand::SelectPlaced { uid } => {
            if session.content == EditContentType::Environment {
                return;
            }
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
            if session.content == EditContentType::Environment {
                return;
            }
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
            save(session, world, exit_after, true);
        }
        EditCommand::SaveAndExit => save(session, world, true, true),
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
        EditCommand::RequestCleanUp => {
            if session.phase != EditPhase::Idle
                && !session.exit_dialog
                && session.content != EditContentType::Environment
            {
                session.clean_up_dialog = true;
                session.changed();
            }
        }
        EditCommand::CancelCleanUp => {
            if session.clean_up_dialog {
                session.clean_up_dialog = false;
                session.changed();
            }
        }
        EditCommand::CleanUpAll => {
            if session.content == EditContentType::Environment {
                return;
            }
            // Only the confirmation offers this action.
            if session.clean_up_dialog {
                session.clean_up_dialog = false;
                clean_up_all(world, session);
            }
        }
        EditCommand::DiscardAndExit => {
            // Not a hotkey nor a site-switch fallback. Only the explicit exit
            // prompt offers this source action (ReloadPlayerLayoutDataAsync).
            if session.exit_dialog {
                presentation::clear(world);
                // The published layout/inventory never changed. Drop only
                // the explicit discarded draft while retaining the actor/input
                // lease until Show has a safe current navigation attachment.
                if let Some(base) = session.baseline.as_ref() {
                    session.rows = base.editor_rows();
                }
                session.environment = session
                    .baseline
                    .as_ref()
                    .and_then(FixturePlacements::phenomena_id);
                environment::preview(world, session);
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
    // The camera reads the whole gesture batch after these commands. A drag
    // remains road-owned for that batch even when its End clears dragging.
    if let Some(mut session) = world.get_resource_mut::<EditSession>() {
        session.road_camera_drag_claimed = session.road.dragging;
    }
    let pending = std::mem::take(&mut world.resource_mut::<PendingCommands>().0);
    if pending.is_empty() {
        return;
    }
    // Pointer loss is lifecycle cleanup, not a new edit. Process it before
    // modal/save gates, which intentionally reject new layout operations.
    let pointer_cancelled = pending.iter().any(|command| matches!(command, EditCommand::RoadPointerCancel));
    if pointer_cancelled {
        if let Some(mut session) = world.get_resource_mut::<EditSession>() {
            if session.road.cancel_pointer() { session.changed(); }
        }
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
    if let Some(dialog) = &session.save_dialog {
        info!(
            "[edit-save] {:?} is shown; {} editor commands dropped",
            dialog.kind,
            pending.len()
        );
        world.insert_resource(session);
        return;
    }
    for command in pending {
        // A cancelled drag has an End gesture as well. Do not interpret it
        // as a release that confirms the selection rectangle in this batch.
        if pointer_cancelled && matches!(command,
            EditCommand::RoadPointerCancel | EditCommand::RoadLongStart { .. }
            | EditCommand::RoadLongEnd | EditCommand::RoadDragStart { .. }
            | EditCommand::RoadDragMove { .. } | EditCommand::RoadDragEnd
        ) { continue; }
        apply_command(world, &mut session, command);
        session.road_camera_drag_claimed |= session.road.dragging;
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
/// One draft row in the served layout's record form.
fn served_row(item: &EditableFixture) -> Result<moly_protocol::api::HousingLayoutFixture, String> {
    let (min, max) = item.footprint()?;
    Ok(moly_protocol::api::HousingLayoutFixture {
        package: item.package.clone(),
        mysekai_fixture_id: item.fixture_id,
        minimum: moly_protocol::api::HousingLayoutCorner {
            x: min.x,
            y: min.y,
            z: min.z,
        },
        maximum: moly_protocol::api::HousingLayoutCorner {
            x: max.x,
            y: max.y,
            z: max.z,
        },
        center_y: item.center.y,
        layout_type: item.layout,
        rotation: item.direction as u8,
        texture_id: item.texture_id,
    })
}

/// The open session's draft as a [`moly_layout::Layout`], with one piece
/// number per row in row order.
fn session_layout(session: &EditSession) -> Result<moly_layout::Layout, String> {
    let baseline = session
        .baseline
        .as_ref()
        .ok_or_else(|| "the editor has no layout".to_owned())?;
    let floor = baseline
        .floor_grid()
        .ok_or_else(|| "the site grid is not loaded".to_owned())?;
    let grid = moly_layout::SiteGrid::new(
        floor.level,
        floor.layout_id,
        floor.width,
        floor.height,
        floor.depth,
    );
    let rows = session
        .rows
        .iter()
        .enumerate()
        .map(|(index, item)| {
            Ok(moly_layout::PlacedRow {
                piece: moly_layout::Piece(
                    u32::try_from(index + 1).map_err(|_| "too many rows".to_owned())?,
                ),
                row: served_row(item)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(moly_layout::Layout::new(
        baseline.site_id(),
        baseline.site_type().to_owned(),
        grid,
        rows,
    ))
}

fn item_from_served(
    row: &moly_protocol::api::HousingLayoutFixture,
    uid: String,
    grid_size: moly_law::fixture::Vector3Int,
) -> Result<EditableFixture, String> {
    let min = GridPosition::new(row.minimum.x, row.minimum.y, row.minimum.z);
    let max = GridPosition::new(row.maximum.x, row.maximum.y, row.maximum.z);
    let (mut center, derived) = moly_law::fixture::position::footprint_to_center_size(min, max)?;
    let _ = derived;
    center.y = row.center_y;
    Ok(EditableFixture {
        uid,
        package: row.package.clone(),
        texture_id: row.texture_id,
        fixture_id: row.mysekai_fixture_id,
        center,
        grid_size,
        layout: row.layout_type,
        direction: moly_layout::Rot::from_row(row.rotation)
            .map(moly_layout::Rot::direction)
            .unwrap_or(Direction::Front),
    })
}

/// Runs one `moly.layout.apply` transaction against the open session. A dry
/// run returns the results without touching the draft; a successful real run
/// writes the resulting rows back exactly as the equivalent manual commands
/// would (selection cleared, phase browsing, draft dirty, removed rows to the
/// offline inventory). The whole-layout save remains the only uplink.
///
/// `Err` names an input the transaction could not run on (no open session,
/// no layout or site grid, rules unavailable, a resulting row that does not
/// convert, the uid counter exhausted); `Ok(Err(_))` is the layout's own
/// refusal.
pub fn apply_request(
    world: &mut World,
    request: &moly_layout::Apply,
) -> Result<moly_layout::ApplyOutcome, String> {
    let Some(mut session) = world.remove_resource::<EditSession>() else {
        return Err("the editor is not open".to_owned());
    };
    let result = apply_in_session(world, &mut session, request);
    world.insert_resource(session);
    result
}

fn apply_in_session(
    world: &mut World,
    session: &mut EditSession,
    request: &moly_layout::Apply,
) -> Result<moly_layout::ApplyOutcome, String> {
    let mut layout = session_layout(session)?;
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid)
        .ok_or_else(|| "the site grid is not loaded".to_owned())?;
    let rules = layout_rules::rules(world, Some(floor))
        .map_err(|missing| format!("layout rules unavailable: {missing:?}"))?;
    let outcome = moly_layout::apply(&mut layout, &rules, request);
    if outcome.is_err() || request.dry_run {
        return Ok(outcome);
    }

    let old: Vec<(moly_layout::Piece, String)> = session
        .rows
        .iter()
        .enumerate()
        .map(|(index, item)| (moly_layout::Piece(index as u32 + 1), item.uid.clone()))
        .collect();
    let by_piece: std::collections::HashMap<_, _> = old.iter().cloned().collect();
    let mut kept = std::collections::HashSet::new();
    let site_id = session
        .baseline
        .as_ref()
        .map_or(0, FixturePlacements::site_id);
    let mut rows = Vec::with_capacity(layout.rows.len());
    for placed in &layout.rows {
        let uid = match by_piece.get(&placed.piece) {
            Some(uid) => uid.clone(),
            None => {
                let serial = session.next_fixture_uid;
                let Some(next) = serial.checked_add(1) else {
                    return Err("the fixture uid counter is exhausted".to_owned());
                };
                session.next_fixture_uid = next;
                format!("offline-edit-{site_id}-{serial}")
            }
        };
        kept.insert(placed.piece);
        let grid_size = rules
            .masters
            .get(&placed.row.mysekai_fixture_id)
            .map_or_else(
                || {
                    let min = GridPosition::new(
                        placed.row.minimum.x,
                        placed.row.minimum.y,
                        placed.row.minimum.z,
                    );
                    let max = GridPosition::new(
                        placed.row.maximum.x,
                        placed.row.maximum.y,
                        placed.row.maximum.z,
                    );
                    moly_law::fixture::position::footprint_to_center_size(min, max)
                        .map(|(_, size)| size)
                        .unwrap_or_default()
                },
                |master| master.grid_size,
            );
        let item = item_from_served(&placed.row, uid, grid_size)
            .map_err(|error| format!("a resulting row does not convert: {error}"))?;
        rows.push(item);
    }
    for (piece, uid) in old {
        if kept.contains(&piece) {
            continue;
        }
        if let Some(item) = session.rows.iter().find(|row| row.uid == uid) {
            if !session.inventory.iter().any(|row| row.uid == item.uid) {
                session.inventory.push(item.clone());
            }
        }
    }
    session.rows = rows;
    session.selected = None;
    session.phase = EditPhase::Browsing;
    session.say("已应用摆放操作（尚未保存）。");
    session.changed();
    Ok(outcome)
}
fn advance_recovery(world: &mut World) {
    actors::maintain(world);
    let Some(mut session) = world.remove_resource::<EditSession>() else {
        return;
    };
    let readiness = session
        .pending_reload
        .as_ref()
        .map(|pending| actors::reload_ready(world, pending));
    match readiness {
        Some(Ok(true)) => {
            let exit_after = session
                .pending_reload
                .as_ref()
                .is_some_and(|pending| pending.exit_after);
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
                        session.say(format!(
                            "场景已加载，但{error}；仍处于编辑，请调整家具后再退出。"
                        ));
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

/// Publishes whether the draft holds unsaved changes
/// ([`EditSessionActive::unsaved_layout`]).
fn publish_unsaved(session: Option<Res<EditSession>>, mut active: ResMut<EditSessionActive>) {
    let unsaved = session.as_deref().is_some_and(has_unsaved_layout);
    if active.unsaved_layout != unsaved {
        active.unsaved_layout = unsaved;
    }
}

fn publish_view(world: &mut World) {
    let (Some(session), Some(view)) = (
        world.get_resource::<EditSession>(),
        world.get_resource::<EditView>(),
    ) else {
        return;
    };
    let owned_revision = world
        .get_resource::<crate::userdata::client::inventory::ClientMysekaiInventory>()
        .map_or(0, |owned| owned.revision);
    let environment_choices = if session.phase != EditPhase::Idle {
        environment::choices(world)
    } else {
        Vec::new()
    };
    if view.revision == session.revision
        && view.owned_revision == owned_revision
        && view.environment_choices == environment_choices
    {
        return;
    }
    let selected = session
        .selected
        .as_ref()
        .map(|selection| EditSelectionView {
            item: item_view(&selection.item),
            from_inventory: selection.origin == SelectionOrigin::Inventory,
            is_new: selection.origin == SelectionOrigin::Owned,
            can_clean_up: selection.can_clean_up,
            put_status: put_status(world, session, selection),
        });
    let active = session.phase != EditPhase::Idle;
    let catalog = if active {
        owned_catalog_view(world, session)
    } else {
        Vec::new()
    };
    // A fence or a road is edited by its own state (the floor editor's
    // touch skips it).
    let traits = put_effect::traits_table(world);
    let editable = |item: &EditableFixture| {
        let mut view = item_view(item);
        view.editable &= !traits
            .as_ref()
            .is_some_and(|traits| traits.get(&item.fixture_id).is_some_and(|t| t.joint));
        view
    };
    let next = EditView {
        road: road::view(world, session),
        content: session.content,
        environment: session.environment,
        environment_choices,
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
        clean_up_dialog: session.clean_up_dialog,
        save_dialog: session.save_dialog.is_some(),
        selected,
        placed_rows: session
            .rows
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let mut view = editable(item);
                view.piece = u32::try_from(index + 1).ok();
                view
            })
            .collect(),
        inventory: session.inventory.iter().map(&editable).collect(),
        catalog,
        owned_revision,
        can_save: active
            && session.selected.is_none()
            && !road_pending(session)
            && session.pending_reload.is_none()
            && session.save_dialog.is_none(),
        feedback: session.feedback.clone(),
    };
    world.insert_resource(next);
}

/// The owned fixture list of the edited site; the list's account is logged
/// whenever it changes.
fn owned_catalog_view(world: &World, session: &EditSession) -> Vec<EditCatalogView> {
    let site_id = session
        .baseline
        .as_ref()
        .map_or(0, FixturePlacements::site_id);
    let (rows, note) = match owned::catalog_for(
        world,
        site_id,
        &session.rows,
        session.content == EditContentType::Road,
    ) {
        Ok((rows, omitted)) => {
            let note = format!(
                "{} rows listed (placeable {:?}); left out: {} without a master row, {} not floor or rug (their own edit states), {} without an exported fixture view, {} with no placeable count",
                rows.len(),
                rows.iter()
                    .map(|row| (row.fixture_id, row.texture_id, row.placeable()))
                    .collect::<Vec<_>>(),
                omitted.no_master,
                omitted.other_layout,
                omitted.no_model,
                omitted.none_placeable
            );
            (rows, note)
        }
        Err(error) => (Vec::new(), format!("no owned list: {error}")),
    };
    static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let mut last = LAST.lock().expect("owned list note");
    if *last != note {
        info!("[edit-owned] ConvertUserFixturesToThumbnailData: {note}");
        *last = note;
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| EditCatalogView {
            index,
            placeable: row.placeable(),
            package: row.package,
            fixture_id: row.fixture_id,
            texture_id: row.texture_id,
            grid_size: row.grid_size,
        })
        .collect()
}

pub struct FixtureEditPlugin;

impl Plugin for FixtureEditPlugin {
    fn build(&self, app: &mut App) {
        edit_grid::install(app);
        crate::fixture_put_limit::install(app);
        road::install(app);
        app.init_resource::<EditSession>()
            .init_resource::<field_build::FieldBuilds>()
            .init_resource::<EditSessionActive>()
            .init_resource::<EditView>()
            .init_resource::<PendingCommands>()
            .add_message::<EditCommand>()
            .add_systems(
                Startup,
                (
                    assets::load,
                    owned::load,
                    put_effect::request_tables,
                    layout_rules::request_zones,
                ),
            )
            .add_systems(
                Update,
                (assets::parse_areas, assets::plan_candidates).chain(),
            )
            .add_systems(Update, layout_rules::read_zones)
            .add_systems(Update, owned::parse)
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
                (
                    receive_commands,
                    answer_save_dialog,
                    apply_commands,
                    advance_recovery,
                )
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
                // `site::read_switch` runs before the commands, so it reads
                // the flag the previous frame's commands left.
                publish_unsaved.after(FixtureEditSystems::Commands),
            )
            .add_systems(Update, read_save_answer.after(apply_commands))
            .add_systems(
                Update,
                (field_build::read_answers, field_build::advance)
                    .chain()
                    .after(apply_commands),
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
                (
                    bevy::prelude::ApplyDeferred,
                    actors::maintain_detached_views,
                )
                    .chain()
                    .after(crate::balloon::place)
                    .after(crate::emoticon::advance)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}
