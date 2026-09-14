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
//! Walls, multi-selection, stacks and full account inventory remain separate
//! source branches; they are not converted into guessed ground behavior.

mod actors;
mod assets;
mod input;
mod presentation;
mod validation;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture::{EditableFixture, FixturePlacements, OccupancyRow};
use assets::{CANDIDATES, FixtureAreas};
use bevy::prelude::*;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition, Vector3Int};

pub(crate) use input::{read_keyboard, read_pointer};

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
    MoveTo { center: GridPosition },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PutStatus {
    Ok,
    OutOfBounds { cells: usize },
    Overlap { cells: usize },
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditSelectionView {
    pub item: EditItemView,
    pub from_inventory: bool,
    pub is_new_mock: bool,
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
    origin: SelectionOrigin,
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
    let entry = world.get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
        .ok_or_else(|| "布局存档所有者尚未就绪".to_owned())
        .and_then(|layouts| layouts.begin_edit(&layout));
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
    session.say("已进入家具编辑：点击家具或清单选中；决定后仍是草稿，保存才写入本地图。");
}

fn select(
    session: &mut EditSession,
    item: EditableFixture,
    origin: SelectionOrigin,
    world: &mut World,
) {
    if !validation::is_ground(&item) {
        session.say("该家具需要墙面、道路或堆叠编辑分支，当前地面编辑不会改动它。");
        return;
    }
    // Selecting another item is an explicit Reset of an unfinished operation,
    // never an implicit Decide or an inventory debit.
    session.cancel_selection();
    session.selected = Some(Selection { item, origin });
    session.phase = EditPhase::Placing;
    session.changed();
    play_se(world, "se_pick_furniture", "edit-pick");
    session.say("已选中家具：拖动或方向键移动，R旋转，决定或取消。");
}

fn decide(session: &mut EditSession, world: &mut World) {
    let Some(selection) = session.selected.as_ref() else {
        return;
    };
    let Some(base) = session.baseline.as_ref() else {
        return;
    };
    let status = validation::put(&selection.item, &session.rows, base.floor_grid());
    if status != PutStatus::Ok {
        session.say(format!(
            "这里不能放置：{}。草稿仍然保留。",
            validation::put_label(status)
        ));
        return;
    }
    let selection = session.selected.take().expect("checked selection");
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
    session.phase = EditPhase::Browsing;
    session.changed();
    play_se(world, "se_housing_finish", "edit-decide");
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
    if let Err(error) = validation::save(&session.rows, base.floor_grid(), areas) {
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
            session.next_fixture_uid = next_serial;
            session.catalog_index = index;
            let item = EditableFixture { texture_id: 1,
                uid,
                package: row.package.to_owned(),
                fixture_id: row.fixture_id,
                center: GridPosition::ZERO,
                grid_size: row.grid_size,
                layout: layout_type::FLOOR,
                direction: Direction::Front,
            };
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
            select(session, item, SelectionOrigin::Inventory, world);
        }
        EditCommand::MoveTo { center } => {
            if let Some(selected) = session.selected.as_mut() {
                if center.y != 0 {
                    session.say("抬高与堆叠编辑尚未接入，未改变家具高度。");
                    return;
                }
                if selected.item.center != center {
                    selected.item.center = center;
                    session.changed();
                }
            }
        }
        EditCommand::Nudge { x, z } => {
            if let Some(selected) = session.selected.as_mut() {
                let Some(x) = selected.item.center.x.checked_add(x) else {
                    return;
                };
                let Some(z) = selected.item.center.z.checked_add(z) else {
                    return;
                };
                let center = GridPosition::new(x, 0, z);
                if selected.item.center != center {
                    selected.item.center = center;
                    session.changed();
                }
            }
        }
        EditCommand::Rotate => {
            if let Some(selected) = session.selected.as_mut() {
                selected.item.direction =
                    Direction::from_u8((selected.item.direction as u8 + 1) % 4)
                        .expect("four source direction values");
                session.changed();
            }
        }
        EditCommand::Decide => decide(session, world),
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
    world.resource_mut::<EditSessionActive>().active = session.phase != EditPhase::Idle;
    if session.phase == EditPhase::Idle {
        presentation::clear(world);
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

fn publish_view(session: Res<EditSession>, mut view: ResMut<EditView>) {
    if view.revision == session.revision {
        return;
    }
    let floor = session
        .baseline
        .as_ref()
        .and_then(FixturePlacements::floor_grid);
    let selected = session
        .selected
        .as_ref()
        .map(|selection| EditSelectionView {
            item: (&selection.item).into(),
            from_inventory: selection.origin == SelectionOrigin::Inventory,
            is_new_mock: selection.origin == SelectionOrigin::Mock,
            put_status: validation::put(&selection.item, &session.rows, floor),
        });
    let active = session.phase != EditPhase::Idle;
    *view = EditView {
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
        placed_rows: session.rows.iter().map(EditItemView::from).collect(),
        inventory: session.inventory.iter().map(EditItemView::from).collect(),
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
}

pub struct FixtureEditPlugin;

impl Plugin for FixtureEditPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditSession>()
            .init_resource::<EditSessionActive>()
            .init_resource::<EditView>()
            .init_resource::<PendingCommands>()
            .add_message::<EditCommand>()
            .add_systems(Startup, assets::load)
            .add_systems(
                Update,
                (assets::parse_areas, assets::plan_candidates).chain(),
            )
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
