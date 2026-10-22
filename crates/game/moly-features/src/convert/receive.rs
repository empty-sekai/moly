//! The convert screen's receive chain (`ConvertReceiveProcessAsync(slot)`)
//! and the one request it sends.
//!
//! Source structure:
//! - `n = MysekaiConvertUtility.GetConvertReceivableCount(slot)`
//!   ([`SlotModel::convert_receivable_count`] over `IsConvertReceivable`,
//!   [`super::model::is_convert_receivable`]). `n <= 0` shows
//!   `ShowPossessionOverLimitDialog` and ends: the two-button
//!   `Common2ButtonDialog` with `MSG_POSSESSION_OVER_LIMIT`, OK
//!   `WORD_TO_CHEST` (`PushUIScreen(MenuScreenType.MysekaiInventory)`),
//!   cancel `WORD_CLOSE` (null), `Layer_Dialog`, `DialogSize.Small`.
//! - `ExecuteMysekaiConvertReceiveAPIAsync(slotId, n)` sends
//!   `PostUserMysekaiConvertReceiveApi{ receiveConvertCount = n }` and
//!   answers the obtained resources; none (a failure, or an empty list) ends
//!   the chain.
//! - `SetNeedsUpdateSlot(slotId, false)`, `DisableTapScreen`,
//!   `PlaySlotReceiveAsync(slotId, slot.NormalizedProgressTime, new
//!   MysekaiConvertFurnitureSlotModel(slotId).NormalizedProgressTime)` (the
//!   cell's wave from the progress before the receive to the progress the
//!   answer leaves), `EnableTapScreen`, `SetNeedsUpdateSlot(slotId, true)`.
//! - The slot's `ConvertItemModel`: a blueprint model whose obtain type is
//!   `random` or `mysekai_fixture_main_genre_choice` awaits
//!   `PlayBlueprintLotteryAnimationAsync`; else a record model awaits
//!   `PlayRecordLotteryAnimationAsync` ([`SlotModel::lottery`]). Each shows
//!   its lottery sub-window (`ShowSubWindowDialog(null, onClose: finished,
//!   allowCloseExternal true)`), calls `Setup(slot's choiceId)`, disables the
//!   taps of `Layer_UI` and `Layer_Header`, and waits for the onClose.
//!   `MysekaiConvertLotterySubWindowDialogBase.Setup` loads its opening
//!   animator prefab (`fx_open_random_box_blueprint` /
//!   `fx_open_random_box_record`) from `mysekai/ui/mysekai_convert_furniture`;
//!   a null prefab logs `Failed to load <name>` and calls `Close()`, whose
//!   onClose ends the wait.
//! - `ShowGetResourceDialogAsync(resources, isReceivedAll:
//!   slot.CurrentCompletedConvertCount - n == 0)`:
//!   `UIUtility.ShowRewardDialog(CalculateSameResourcesQuantity(resources),
//!   isReceivedAll ? MSG_RECEIVED_CONVERT_REWARD :
//!   MSG_RECEIVED_PARTIAL_REWARD_DUE_TO_POSSESSION_LIMIT, unread topics,
//!   onFinish)`, awaited to its onFinish. `CalculateSameResourcesQuantity`
//!   groups the rows by `resourceId` alone (the first row of a group gives
//!   its type and level) and sums the quantities.
//! - After the reward: a slot the colourful pass extends, while the user has
//!   no pass, shows `ShowConvertSlotLockExpiredPassDialog`
//!   (`ShowSubWindowDialog<SubWindowDialog>(Get(MSG_CONVERT_SLOT_LOCK_EXPIRED_PASS))`).
//!
//! Product mapping, named:
//! - The lottery sub-windows' prefab bundle is not on the asset root, so the
//!   source's null path runs: the sub-window opens, the load failure is
//!   logged, and its close is asked for ([`subwindow_dialog::request_close`],
//!   which starts once the open fade ends).
//! - `ShowRewardDialog`'s chain runs here in its order: the blueprint and
//!   record templates, the common reward dialog (or, at six rows or more,
//!   the vertical list dialog, which is not built and is named), then the
//!   surplus item templates. The unread topics this reward passes are not
//!   read (no topic dialog is built).
//! - `isReceivedAll` reads the slot model the presenter held, as it was read
//!   before the request.
//! - `DisableTapScreen` / `EnableTapScreen` and `SetNeedsUpdateSlot` are
//!   named in the log; the wave's seconds are the cell's (see the cell).
//! - The lock-expired pass sub-window shows its message in the body text of
//!   `SubWindowDialog`'s document, which the sub-window module does not
//!   draw: it is refused by name.

use std::collections::BTreeSet;

use bevy::prelude::*;
use moly_link::ServerLink;
use moly_masters::load::MasterTables;
use moly_protocol::api::UserMysekaiConvertReceiveRequest;
use moly_protocol::rows::UserResource;

use super::model::{is_convert_receivable, ItemContext, Lottery, PossessionRoom, SlotModel};
use super::screen::{note_once, now_ms, read_user, ConvertScreen};
use super::start::{ConvertRequest, MysekaiConvertService};
use crate::common_dialog::{self, Buttons};
use crate::get_resource::{self, RewardParts};
use crate::ui_layers::{DialogBackKey, DialogId, DialogType, LayerCommand, LayerId, ScreenManager};
use crate::userdata::client::inventory::{ClientMysekaiInventory, PossessionMasters};
use crate::userdata::convert::ClientConvertSlots;
use crate::userdata::ClientUserData;
use moly_ui::subwindow_dialog;

/// `ShowPossessionOverLimitDialog`'s body.
pub const POSSESSION_OVER_LIMIT: &str = "MSG_POSSESSION_OVER_LIMIT";
/// Its OK label (`PushUIScreen(MysekaiInventory)`).
pub const TO_CHEST: &str = "WORD_TO_CHEST";
/// Its cancel label.
pub const CLOSE: &str = "WORD_CLOSE";
/// The reward's message when every completed convert was received.
pub const RECEIVED_ALL: &str = "MSG_RECEIVED_CONVERT_REWARD";
/// The reward's message when the possession limit held some back.
pub const RECEIVED_PARTIAL: &str = "MSG_RECEIVED_PARTIAL_REWARD_DUE_TO_POSSESSION_LIMIT";
/// `ShowConvertSlotLockExpiredPassDialog`'s message.
pub const LOCK_EXPIRED_PASS: &str = "MSG_CONVERT_SLOT_LOCK_EXPIRED_PASS";
/// The wordings the chain's common dialog draws (the shell charset takes
/// them).
pub const WORDINGS: &[&str] = &[POSSESSION_OVER_LIMIT, TO_CHEST, CLOSE];

/// The lottery sub-windows' documents (layout keys).
pub const LOTTERY_BLUEPRINT: &str = "ConvertLotteryBlueprint";
pub const LOTTERY_RECORD: &str = "ConvertLotteryRecord";
/// The bundle the lottery prefabs load from.
const LOTTERY_BUNDLE: &str = "mysekai/ui/mysekai_convert_furniture";

/// `DialogType.MysekaiConvertBlueprintLotterySubWindowDialog`.
const BLUEPRINT_LOTTERY_DIALOG: DialogType = DialogType(331);
/// `DialogType.MysekaiConvertRecordLotterySubWindowDialog`.
const RECORD_LOTTERY_DIALOG: DialogType = DialogType(332);
/// `DialogType.CommonRewardVerticalListDialog`.
const VERTICAL_LIST_DIALOG: DialogType = DialogType(23);

/// `UIUtility.ShowRewardDialog` shows the vertical list from this many rows.
const VERTICAL_LIST_ROWS: usize = 6;

impl Lottery {
    fn key(self) -> &'static str {
        match self {
            Self::Blueprint => LOTTERY_BLUEPRINT,
            Self::Record => LOTTERY_RECORD,
        }
    }

    fn dialog(self) -> DialogType {
        match self {
            Self::Blueprint => BLUEPRINT_LOTTERY_DIALOG,
            Self::Record => RECORD_LOTTERY_DIALOG,
        }
    }

    /// `FxOpenPrefabName`.
    fn prefab(self) -> &'static str {
        match self {
            Self::Blueprint => "fx_open_random_box_blueprint",
            Self::Record => "fx_open_random_box_record",
        }
    }

    fn caller(self) -> &'static str {
        match self {
            Self::Blueprint => "MysekaiConvertFurniturePresenter.PlayBlueprintLotteryAnimationAsync",
            Self::Record => "MysekaiConvertFurniturePresenter.PlayRecordLotteryAnimationAsync",
        }
    }
}

/// Where a receive's chain stands.
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    /// The request waits for its answer.
    Sent { seq: u32 },
    /// The answer's resources, before the wave.
    Answered { resources: Vec<UserResource> },
    /// The lottery sub-window is shown until its onClose.
    Lottery { dialog: DialogId },
    /// The reward's templates play on the shared chain; `then` is what
    /// follows them.
    Templates { then: AfterTemplates },
    /// The common reward dialog is shown until it is destroyed.
    Common { dialog: DialogId, surplus: Vec<UserResource> },
}

/// What follows a batch of the reward's templates.
#[derive(Clone, Debug, PartialEq)]
enum AfterTemplates {
    /// The common rows, then the surplus templates.
    Common { common: Vec<UserResource>, surplus: Vec<UserResource> },
    /// The reward's onFinish.
    Finish,
}

/// One receive in progress, with the slot model's values as the presenter
/// read them before the request.
#[derive(Clone, Debug, PartialEq)]
struct Flow {
    slot_id: i32,
    /// `GetConvertReceivableCount(slot)`.
    count: i32,
    /// `slot.CurrentCompletedConvertCount`.
    completed: i32,
    /// `slot.NormalizedProgressTime`.
    progress: f32,
    /// `slot.IsExtendedByMysekaiColorfulPass`.
    pass_slot: bool,
    /// The lottery the slot's `ConvertItemModel` plays.
    lottery: Option<Lottery>,
    /// The answer's resources, kept for the reward.
    resources: Vec<UserResource>,
    stage: Stage,
}

/// The receive chain's state: the request a cell's tap made, the
/// over-limit dialog, and the receive in progress (one at a time).
#[derive(Resource, Default)]
pub struct ConvertReceive {
    over_limit: Option<DialogId>,
    flow: Option<Flow>,
    seen: BTreeSet<String>,
}

impl ConvertReceive {
    /// The answer of the receive request `seq` of `slot_id`: its obtained
    /// resources (none on a failure).
    pub fn answered(&mut self, slot_id: i32, seq: u32, resources: Vec<UserResource>) {
        match self.flow.as_mut() {
            Some(flow) if flow.slot_id == slot_id && flow.stage == (Stage::Sent { seq }) => {
                flow.stage = Stage::Answered { resources };
            }
            _ => note_once(
                &mut self.seen,
                format!("the receive answer of slot {slot_id} (seq {seq}) has no receive waiting for it"),
            ),
        }
    }

    /// A receive is in progress (its request, its animations or its reward).
    pub fn busy(&self) -> bool {
        self.flow.is_some() || self.over_limit.is_some()
    }
}

/// `CalculateSameResourcesQuantity`: the rows grouped by `resourceId` in the
/// order their ids first appear; a group takes its first row's type and
/// level and the sum of the quantities.
pub fn same_resources_quantity(resources: &[UserResource]) -> Vec<UserResource> {
    let mut grouped: Vec<UserResource> = Vec::new();
    for row in resources {
        match grouped.iter_mut().find(|group| group.resource_id == row.resource_id) {
            Some(group) => group.quantity += row.quantity,
            None => grouped.push(row.clone()),
        }
    }
    grouped
}

/// Update, exclusive, after the convert answers: starts a receive a cell
/// asked for and steps the one in progress.
pub fn advance(world: &mut World) {
    if world.get_resource::<ConvertScreen>().is_none() {
        return;
    }
    answer_over_limit(world);
    start(world);
    step(world);
}

/// The over-limit dialog's answer: OK pushes the chest screen.
fn answer_over_limit(world: &mut World) {
    let Some(dialog) = world.resource::<ConvertReceive>().over_limit else {
        return;
    };
    let Some(ok) = common_dialog::answer(world, dialog) else {
        return;
    };
    world.resource_mut::<ConvertReceive>().over_limit = None;
    if ok {
        info!("[convert] ShowPossessionOverLimitDialog: OK; PushUIScreen(MenuScreenType.MysekaiInventory)");
        world.write_message(LayerCommand::Push(LayerId::MysekaiInventory));
    } else {
        info!("[convert] ShowPossessionOverLimitDialog: cancel (null)");
    }
}

/// What the receive's start read of the slot.
struct Started {
    count: i32,
    completed: i32,
    progress: f32,
    pass_slot: bool,
    lottery: Option<Lottery>,
}

/// `ConvertReceiveProcessAsync(slot)`'s start: the receivable count, then the
/// over-limit dialog or the request.
fn start(world: &mut World) {
    let busy = world.resource::<ConvertReceive>().busy();
    let Some(slot_id) = world.resource_mut::<ConvertScreen>().receive_intent.take() else {
        return;
    };
    if busy {
        note_once(
            &mut world.resource_mut::<ConvertReceive>().seen,
            format!("ConvertReceiveProcessAsync(slot {slot_id}) while a receive is in progress: not started"),
        );
        return;
    }
    let started = {
        let (Some(masters), Some(user)) = (
            world.resource::<MasterTables>().convert.as_ref(),
            read_user(
                world.get_resource::<ClientUserData>(),
                world.get_resource::<ClientConvertSlots>(),
                world.get_resource::<ClientMysekaiInventory>(),
            ),
        ) else {
            error!("[convert] ConvertReceiveProcessAsync(slot {slot_id}): the convert masters or the user's convert data are not in; no receive");
            return;
        };
        let real = world.resource::<Time<Real>>().elapsed_secs();
        let ctx = ItemContext {
            masters,
            user: &user,
            now: now_ms(world.get_resource::<ClientUserData>(), real),
        };
        let room = match possession_room(world) {
            Ok(room) => room,
            Err(reason) => {
                error!("[convert] ConvertReceiveProcessAsync(slot {slot_id}): {reason}; IsConvertReceivable cannot be read, no receive");
                return;
            }
        };
        let slot = SlotModel::new(ctx, slot_id, ctx.machine().is_pass_slot(slot_id));
        Started {
            count: slot.convert_receivable_count(|rows| is_convert_receivable(rows, room)),
            completed: slot.current_completed_convert_count(),
            progress: slot.normalized_progress_time(),
            pass_slot: slot.is_pass_slot(),
            lottery: slot.lottery(ctx),
        }
    };
    if started.count <= 0 {
        info!(
            "[convert] ConvertReceiveProcessAsync(slot {slot_id}): GetConvertReceivableCount 0; ShowPossessionOverLimitDialog: Show2ButtonDialog(Common2ButtonDialog, {POSSESSION_OVER_LIMIT}, {TO_CHEST}, {CLOSE}, onOK: PushUIScreen(MysekaiInventory), onCancel: null, Layer_Dialog, DialogSize.Small)"
        );
        let caller = "MysekaiConvertUtility.ShowPossessionOverLimitDialog";
        let body = common_dialog::wording(world, caller, POSSESSION_OVER_LIMIT);
        world.resource_mut::<ConvertReceive>().over_limit = common_dialog::open(
            world,
            caller,
            body,
            Buttons::Two {
                ok: TO_CHEST,
                cancel: CLOSE,
            },
            None,
        );
        return;
    }
    let request = ConvertRequest::Receive {
        slot_id,
        request: UserMysekaiConvertReceiveRequest {
            receive_convert_count: started.count,
        },
    };
    let sent = world.resource_scope(|world, mut service: Mut<MysekaiConvertService>| {
        service.send(&mut world.resource_mut::<ServerLink>(), request)
    });
    match sent {
        Ok(seq) => {
            info!(
                "[convert] ConvertReceiveProcessAsync(slot {slot_id}): GetConvertReceivableCount {}; ExecuteMysekaiConvertReceiveAPIAsync({slot_id}, {}) sent as seq {seq}",
                started.count, started.count
            );
            world.resource_mut::<ConvertReceive>().flow = Some(Flow {
                slot_id,
                count: started.count,
                completed: started.completed,
                progress: started.progress,
                pass_slot: started.pass_slot,
                lottery: started.lottery,
                resources: Vec::new(),
                stage: Stage::Sent { seq },
            });
        }
        Err(reason) => error!("[convert] ConvertReceiveProcessAsync(slot {slot_id}): {reason}"),
    }
}

/// `IsConvertReceivable`'s inputs from the user's copies and the possession
/// masters.
fn possession_room(world: &World) -> Result<PossessionRoom, String> {
    let inventory = world
        .get_resource::<ClientMysekaiInventory>()
        .ok_or("the user's inventory is not in")?;
    let masters = world
        .get_resource::<PossessionMasters>()
        .ok_or("the possession masters are not loaded")?;
    Ok(PossessionRoom {
        fixture_total: inventory.fixture_total_count(),
        fixture_max: inventory
            .fixture_max_count(masters)
            .ok_or("GetFixtureMaxCount: the user's fixture possession level has no master row")?,
        material_total: inventory.total_material_possession(),
        material_max: inventory
            .material_max_count(masters)
            .ok_or("GetMaterialMaxCount: the user's material possession level has no master row")?,
    })
}

/// Steps the receive in progress as far as it can go this frame.
fn step(world: &mut World) {
    // Each pass moves the stage on or leaves it waiting.
    for _ in 0..8 {
        let Some(flow) = world.resource::<ConvertReceive>().flow.clone() else {
            return;
        };
        let next = match &flow.stage {
            Stage::Sent { .. } => return,
            Stage::Answered { resources } => answered(world, &flow, resources.clone()),
            Stage::Lottery { dialog } => {
                if !lottery_closed(world, *dialog) {
                    return;
                }
                info!(
                    "[convert] the lottery's onClose: the wait ends; EnableTapScreen(Layer_UI, Layer_Header)"
                );
                reward(world, &flow)
            }
            Stage::Templates { then } => {
                if !get_resource::chain_idle(world) {
                    return;
                }
                match then.clone() {
                    AfterTemplates::Common { common, surplus } => common_reward(world, &flow, common, surplus),
                    AfterTemplates::Finish => finish(world, &flow),
                }
            }
            Stage::Common { dialog, surplus } => {
                if world.resource::<ScreenManager>().dialog_state(*dialog).is_some() {
                    return;
                }
                templates(world, surplus.clone(), AfterTemplates::Finish)
            }
        };
        let mut receive = world.resource_mut::<ConvertReceive>();
        match next {
            Some(stage) => {
                if let Some(flow) = receive.flow.as_mut() {
                    flow.stage = stage;
                }
            }
            None => receive.flow = None,
        }
    }
}

/// The request's answer: an empty one ends the chain; else the wave and the
/// lottery or the reward.
fn answered(world: &mut World, flow: &Flow, resources: Vec<UserResource>) -> Option<Stage> {
    let slot_id = flow.slot_id;
    if resources.is_empty() {
        info!("[convert] ExecuteMysekaiConvertReceiveAPIAsync({slot_id}): no obtained resources; the receive ends");
        return None;
    }
    if let Some(receive) = world.resource_mut::<ConvertReceive>().flow.as_mut() {
        receive.resources = resources;
    }
    // `new MysekaiConvertFurnitureSlotModel(slotId).NormalizedProgressTime`:
    // the slot as the answer leaves it.
    let after = {
        let user = read_user(
            world.get_resource::<ClientUserData>(),
            world.get_resource::<ClientConvertSlots>(),
            world.get_resource::<ClientMysekaiInventory>(),
        );
        match (world.resource::<MasterTables>().convert.as_ref(), user) {
            (Some(masters), Some(user)) => {
                let real = world.resource::<Time<Real>>().elapsed_secs();
                let ctx = ItemContext {
                    masters,
                    user: &user,
                    now: now_ms(world.get_resource::<ClientUserData>(), real),
                };
                Some(SlotModel::new(ctx, slot_id, flow.pass_slot).normalized_progress_time())
            }
            _ => None,
        }
    };
    match after {
        Some(to) => {
            info!(
                "[convert] SetNeedsUpdateSlot({slot_id}, false), DisableTapScreen, PlaySlotReceiveAsync({slot_id}, {}, {to}), EnableTapScreen, SetNeedsUpdateSlot({slot_id}, true)",
                flow.progress
            );
            world
                .resource_mut::<ConvertScreen>()
                .slot_play_receive(slot_id, flow.progress, to);
        }
        None => error!(
            "[convert] PlaySlotReceiveAsync({slot_id}): the slot after the receive is not read; the wave stays where the slots' next read puts it"
        ),
    }
    match flow.lottery {
        Some(lottery) => open_lottery(world, flow, lottery),
        None => reward(world, flow),
    }
}

/// `PlayBlueprintLotteryAnimationAsync` / `PlayRecordLotteryAnimationAsync`:
/// the sub-window shows; its Setup finds no prefab and closes it.
fn open_lottery(world: &mut World, flow: &Flow, lottery: Lottery) -> Option<Stage> {
    let caller = lottery.caller();
    let key = lottery.key();
    subwindow_dialog::register(world, &[(key, key)]);
    let choice_id = flow_choice(world, flow.slot_id);
    let Some(dialog) =
        subwindow_dialog::open(world, lottery.dialog(), key, DialogBackKey::Close, caller, true)
    else {
        error!("[convert] {caller}: the lottery sub-window did not open; the receive goes on to its reward");
        return reward(world, flow);
    };
    info!(
        "[convert] {caller}: ShowSubWindowDialog(null, onClose, true, {}) {dialog:?}; Setup({choice_id}); DisableTapScreen(Layer_UI, Layer_Header); wait for the onClose",
        lottery.dialog().name()
    );
    error!(
        "[convert] {}.Setup: Failed to load {} ({LOTTERY_BUNDLE} is not on the asset root); Close()",
        lottery.dialog().name(),
        lottery.prefab()
    );
    subwindow_dialog::request_close(world);
    Some(Stage::Lottery { dialog })
}

/// The slot row's `choiceId` (0 without a row), as the lottery's Setup reads
/// it.
fn flow_choice(world: &World, slot_id: i32) -> i32 {
    world
        .get_resource::<ClientConvertSlots>()
        .and_then(|copies| copies.slots().iter().find(|row| row.slot_id == slot_id))
        .map_or(0, |row| row.choice_id)
}

/// The lottery sub-window `dialog` has run its onClose (its close started,
/// or it is gone).
fn lottery_closed(world: &World, dialog: DialogId) -> bool {
    match world.resource::<subwindow_dialog::SubWindows>().shown() {
        Some(shown) if shown.id == dialog => shown.closing(),
        _ => true,
    }
}

/// `ShowGetResourceDialogAsync(resources, isReceivedAll)`: the reward's
/// templates first.
fn reward(world: &mut World, flow: &Flow) -> Option<Stage> {
    let resources = world
        .resource::<ConvertReceive>()
        .flow
        .as_ref()
        .map(|flow| flow.resources.clone())
        .unwrap_or_default();
    let grouped = same_resources_quantity(&resources);
    let received_all = flow.completed - flow.count == 0;
    let key = if received_all { RECEIVED_ALL } else { RECEIVED_PARTIAL };
    info!(
        "[convert] ShowGetResourceDialogAsync({} rows, isReceivedAll {received_all}): ShowRewardDialog({} grouped rows, {key}, unread topics, onFinish)",
        resources.len(),
        grouped.len()
    );
    note_once(
        &mut world.resource_mut::<ConvertReceive>().seen,
        "ShowRewardDialog's unread topics are not read: no topic dialog is built (named)".to_owned(),
    );
    let parts = match get_resource::reward_parts(world, &grouped) {
        Ok(parts) => parts,
        Err(reason) => {
            error!("[convert] ShowRewardDialog: {reason}; the reward is not shown");
            return finish(world, flow);
        }
    };
    let RewardParts {
        templates: first,
        common,
        surplus,
    } = parts;
    if first.len() + common.len() >= VERTICAL_LIST_ROWS {
        error!(
            "[convert] ShowRewardDialog: {} rows show in {} (ShowCommonRewardVerticalDialog), which is not built; the surplus templates follow",
            first.len() + common.len(),
            VERTICAL_LIST_DIALOG.name()
        );
        return templates(world, surplus, AfterTemplates::Finish);
    }
    templates(world, first, AfterTemplates::Common { common, surplus })
}

/// Chains a batch of the reward's templates on the shared chain player.
fn templates(world: &mut World, rows: Vec<UserResource>, then: AfterTemplates) -> Option<Stage> {
    for row in &rows {
        if let Err(reason) = get_resource::chain_reward(
            world,
            row,
            "UIUtility.ShowMysekaiGetResourceSubWindowDialog",
        ) {
            error!(
                "[convert] ShowRewardDialog: {} {}: {reason}; its template is not shown",
                row.resource_type, row.resource_id
            );
        }
    }
    Some(Stage::Templates { then })
}

/// `ShowCommonRewardHorizonDialog`: the common reward dialog with the
/// reward's message, when there are common rows.
fn common_reward(
    world: &mut World,
    flow: &Flow,
    common: Vec<UserResource>,
    surplus: Vec<UserResource>,
) -> Option<Stage> {
    if common.is_empty() {
        return templates(world, surplus, AfterTemplates::Finish);
    }
    let key = if flow.completed - flow.count == 0 {
        RECEIVED_ALL
    } else {
        RECEIVED_PARTIAL
    };
    let caller = "UIUtility.ShowCommonRewardHorizonDialog";
    match get_resource::show_common_reward(world, caller, key, common.len()) {
        Ok(dialog) => {
            info!(
                "[convert] {caller}: ChainSubWindowDialog<CommonRewardSubWindowDialog>({key}) {dialog:?}; Setup({}, 0, OpenSE.Reward, {key}, false)",
                common
                    .iter()
                    .map(|row| format!("{} {} x{}", row.resource_type, row.resource_id, row.quantity))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Some(Stage::Common { dialog, surplus })
        }
        Err(reason) => {
            error!("[convert] {caller}: {reason}; the surplus templates follow");
            templates(world, surplus, AfterTemplates::Finish)
        }
    }
}

/// The reward's onFinish, then the lock-expired pass notice.
fn finish(world: &mut World, flow: &Flow) -> Option<Stage> {
    info!("[convert] ShowRewardDialog's onFinish: ShowGetResourceDialogAsync ends");
    if flow.pass_slot {
        let pass_valid = {
            let user = read_user(
                world.get_resource::<ClientUserData>(),
                world.get_resource::<ClientConvertSlots>(),
                world.get_resource::<ClientMysekaiInventory>(),
            );
            let real = world.resource::<Time<Real>>().elapsed_secs();
            let now = now_ms(world.get_resource::<ClientUserData>(), real);
            user.is_some_and(|user| user.pass.is_valid(now))
        };
        if !pass_valid {
            error!(
                "[convert] ShowConvertSlotLockExpiredPassDialog: ShowSubWindowDialog<SubWindowDialog>({LOCK_EXPIRED_PASS}) is refused: the sub-window module does not draw a message body"
            );
        }
    }
    None
}
