//! The convert screen's start and skip chains (`ConfirmConvertStart` ->
//! `ConvertStartProcess` -> `ConvertStartProcessAsync`, `OnShortenSlot` ->
//! `OnClickOkShortenSlot` -> `ShortenSlotProcessAsync`) and the one request
//! each sends.
//!
//! Source structure:
//! - `Presenter.ConfirmConvertStart(slotId, model)` shows the two-button
//!   `MysekaiConvertStartConfirmDialog` with no body key; its OK calls
//!   `ConvertStartProcess(slotId, model)` and its cancel is null.
//! - `ConvertStartProcess`: for a slot the colourful pass extends,
//!   `GetOnceConvertibleCount(slotId, model)` of 0 or less shows the
//!   one-button `Common1ButtonDialog` with `MSG_CONVERT_NOT_EXPIRED_PASS` (its
//!   OK, `WORD_OK`, is `ChangeView(ViewType.Slot)`, so the close button, an
//!   outside tap and the back key do the same) and returns; a count below the
//!   model's `ConvertCount` shows it with `MSG_CONVERT_UPPER_LIMIT_PASS` (its
//!   OK refreshes the item select page) and returns. Any other slot goes on to
//!   the request.
//! - `ConvertStartProcessAsync`: `ExecuteMysekaiConvertStartAPIAsync` sends
//!   `PostUserMysekaiConvertStartApi{ slotId, mysekaiConvertItemId,
//!   convertCount, choiceId }`. Only the service's success answer (the API's
//!   success state, then `UserDataManager.UpdateAll(response)`) runs the
//!   rest: `ChangeView(ViewType.Slot)`, `SetNeedsUpdateSlot(slotId, false)`,
//!   then `PlayConvertStartAsync(slotId)`: `SlotView` finds the cell of that
//!   slot and calls its `PlayConvertStartAsync`, which calls the timer view's
//!   `PlayConvertStart(model)` and the button view's `PlayConvertStart()` once
//!   and holds the gauge's `PlayConvertStartAsync(model)`; that one reads the
//!   model's state again, shows the processing content with the progress line
//!   (`WORD_CONVERT_PROGRESS_FORMAT` of the completed and the remaining
//!   converts) while the wave hides, waits `_settingAnimationSeconds` with
//!   the cancellation swallowed, reads the state again, puts the wave at
//!   `NormalizedProgressTime`, shows it and fades it in; then
//!   `SetNeedsUpdateSlot(slotId, true)`. Any answer that is not the success
//!   (a refusal, or a reply of another kind) runs nothing after the request.
//! - `Presenter.OnShortenSlot(slot)` shows the two-button
//!   `MysekaiConvertFurnitureShortenDialog`, whose OK calls
//!   `OnClickOkShortenSlot(slot)`; that one sends only while the slot's state
//!   is `Processing`, as the dialog's model reads it each frame.
//! - `ShortenSlotProcessAsync`: `ExecuteMysekaiConvertShortenAPIAsync(slot)`
//!   sends `PostUserMysekaiConvertShortenApi{ slotId, consumeJewelQuantity =
//!   slot.ConsumedJewelQuantityForShorten }`, and after the success only
//!   `SetNeedsUpdateSlot(slotId, false)`, `DisableTapScreen`, the cell's
//!   `PlayShortenAsync(NormalizedProgressTime)` (the gauge's
//!   `PlayShortenAsync`, which tweens the wave's progress to 1 over
//!   `_receiveAnimationSeconds`), `EnableTapScreen` and
//!   `SetNeedsUpdateSlot(slotId, true)`.
//!
//! One request is in flight at a time ([`MysekaiConvertService`]): an answer
//! of another call's number leaves the request waiting, and a `link_lost`
//! answer of the convert call that no longer matches it finishes the request
//! on the failure path, as the mission receive's service reads its own.
//!
//! Product mapping, named:
//! - The two dialogs this chain opens are the convert dialogs' unit (324 and
//!   325), not built here: the request a dialog's OK makes waits in
//!   [`ConvertScreen::start_intent`] / [`ConvertScreen::shorten_intent`]
//!   until the dialog's OK reads it and answers [`StartIntent::confirm`] /
//!   [`ShortenIntent::confirm`]. A request written while one waits is refused
//!   by name; a request whose dialog the screen closes is dropped with it.
//! - The dialog's Setup holds the model the presenter passes it, which this
//!   module cannot keep (the item model borrows the user and the master
//!   tables): the slot, the item's id, its choice and the `ConvertCount` are
//!   kept instead, and the model is read again where the request is made.
//! - `DisableTapScreen` / `EnableTapScreen` around the skip: the click cover
//!   this tree has is the layout editor's (`ScreenManager`'s own note), and
//!   the skip's wave lasts the serialized `_receiveAnimationSeconds`, which
//!   the prefab overrides (its own seconds are unverified); the pair is
//!   named once, and the wave is the cell's tween the slot page steps.
//! - The start's cell animation runs on the slot page, which
//!   `ChangeView(Slot)` closes the item select page onto. The gauge's end
//!   state is the slot's own `NormalizedProgressTime` in any case, which the
//!   slots' refresh draws; the gauge's processing content, its wait and its
//!   fade are named.
//! - The wave's curve is the source's linear `_receiveAnimationCurve`
//!   (`AnimationCurve.Linear(0, 0, 1, 1)`) on the source's out-quad scale
//!   curve; a prefab may carry another (unverified).

use std::collections::BTreeSet;

use bevy::prelude::*;
use moly_link::ServerLink;
use moly_masters::load::MasterTables;
use moly_protocol::api::{
    ConvertPost, UserMysekaiConvertReceiveRequest, UserMysekaiConvertShortenRequest,
    UserMysekaiConvertStartRequest,
};
use moly_protocol::{Call, CallKind, Reply};

use super::model::{ItemContext, ItemModel, SlotModel, SlotState};
use super::screen::{note_once, now_ms, read_user, ConvertScreen, ViewType};
use crate::common_dialog::{self, Buttons};
use crate::ui_layout::UiLayouts;
use crate::userdata::client::inventory::ClientMysekaiInventory;
use crate::userdata::convert::ClientConvertSlots;
use crate::userdata::{Answered, ClientUserData, SentCalls, ServerPending};

/// `ShowConvertNotExpiredPassDialog`'s body (`Common1ButtonDialog`).
pub const NOT_EXPIRED_PASS: &str = "MSG_CONVERT_NOT_EXPIRED_PASS";
/// `ShowConvertUpperLimitPassDialog`'s body (`Common1ButtonDialog`).
pub const UPPER_LIMIT_PASS: &str = "MSG_CONVERT_UPPER_LIMIT_PASS";
/// The label both dialogs pass (`WORD_OK`).
pub const OK: &str = "WORD_OK";
/// `ScreenManager.Show1ButtonDialog`'s size for both (`DialogSize.Small`).
pub const DIALOG_SIZE: &str = "DialogSize.Small";
/// The calls of these requests (`Call::Convert`).
pub const CALL: CallKind = CallKind::Convert;
/// The refusal a lost session answers a call with (`moly_link`), read the
/// same way the mission receive's service reads it.
const LINK_LOST: &str = "link_lost";

// ---------------------------------------------------------------------------
// What a dialog's OK asks of this chain
// ---------------------------------------------------------------------------

/// The start confirm dialog's view model
/// (`new MysekaiConvertStartConfirmViewModel(model)`): the model, and its
/// `ConvertCount`, both as the dialog was set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartItem {
    /// `model.Id` (`GetMasterMysekaiConvertItem`'s row).
    pub item_id: i32,
    /// `model.ChoiceId`.
    pub choice_id: i32,
    /// `model.ConvertCount`.
    pub convert_count: i32,
}

/// The request `ConfirmConvertStart(slotId, model)` made, waiting for the
/// start confirm dialog's OK (`ConvertStartProcess(slotId, model)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartIntent {
    pub slot_id: i32,
    /// The model's item; `None` when the panel's model cannot be read on the
    /// frame of the request (its OK then refuses the start by name).
    pub item: Option<StartItem>,
}

impl StartIntent {
    /// `Show2ButtonDialog<MysekaiConvertStartConfirmDialog>(dialog, onOK: _ =>
    /// ConvertStartProcess(slotId, model), onCancel: null)`: the answer of the
    /// dialog's OK is this same request.
    pub fn confirm(self) -> Self {
        self
    }
}

/// The request `OnShortenSlot(slot)` made, waiting for the skip dialog's OK
/// (`OnClickOkShortenSlot(slot)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortenIntent {
    /// `slot.SlotId`.
    pub slot_id: i32,
    /// `IsExtendedSlotByMysekaiColorfulPass(slotId)`.
    pub pass_slot: bool,
}

impl ShortenIntent {
    /// `Show2ButtonDialog<MysekaiConvertFurnitureShortenDialog>(dialog,
    /// onOK: _ => OnClickOkShortenSlot(slot), onCancel: null)`.
    pub fn confirm(self) -> Self {
        self
    }
}

/// The wordings the two pass dialogs draw (the shell charset takes them).
pub const WORDINGS: &[&str] = &[NOT_EXPIRED_PASS, UPPER_LIMIT_PASS, OK];

/// One of `ConvertStartProcess`'s two pass dialogs: waiting to show, then
/// shown as a common dialog until its OK (or its close) answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassDialog {
    pub step: StartStep,
    pub shown: Option<moly_ui::ui_layers::DialogId>,
}

impl PassDialog {
    fn waiting(step: StartStep) -> Self {
        Self { step, shown: None }
    }
}

/// `ConvertStartProcess`'s three ways out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartStep {
    /// `ShowConvertNotExpiredPassDialog()`.
    NotExpired,
    /// `ShowConvertUpperLimitPassDialog()`.
    UpperLimit,
    /// `ConvertStartProcessAsync().Forget()`.
    Send,
}

// ---------------------------------------------------------------------------
// The chains' steps
// ---------------------------------------------------------------------------

/// The models the screen's list holds (`GetMasterMysekaiConvertItem` and the
/// item models it builds), with the row whose choice table it does not load.
fn item_models<'a>(ctx: ItemContext<'a>, seen: &mut BTreeSet<String>) -> Vec<ItemModel<'a>> {
    let models = ItemModel::all(ctx);
    if let Some(skipped) = models.skipped() {
        note_once(seen, skipped.to_owned());
    }
    models.into_models()
}

/// The item the screen's list holds for an id (`GetMasterMysekaiConvertItem`
/// and the item models it builds).
fn item_model<'a>(ctx: ItemContext<'a>, item_id: i32, seen: &mut BTreeSet<String>) -> Option<ItemModel<'a>> {
    item_models(ctx, seen).into_iter().find(|model| model.id() == item_id)
}

/// `ConvertStartProcess(slotId, model)`: a slot the pass extends is held to
/// `GetOnceConvertibleCount` before it may start; any other slot sends.
pub fn start_step(
    ctx: ItemContext<'_>,
    slot_id: i32,
    item: StartItem,
    seen: &mut BTreeSet<String>,
) -> StartStep {
    if !ctx.machine().is_pass_slot(slot_id) {
        return StartStep::Send;
    }
    let once = match item_model(ctx, item.item_id, seen) {
        Some(model) => model.once_convertible_count_in_slot(ctx, slot_id),
        // `GetOnceConvertibleCount` without the model: the pass slot's 0.
        None => 0,
    };
    if once <= 0 {
        StartStep::NotExpired
    } else if once < item.convert_count {
        StartStep::UpperLimit
    } else {
        StartStep::Send
    }
}

/// `ConvertStartProcessAsync`'s request:
/// `UserMysekaiConvertStartRequest{ mysekaiConvertItemId = model.Id,
/// convertCount = model.ConvertCount, choiceId = model.ChoiceId }`.
pub fn start_request(intent: StartIntent) -> Option<ConvertRequest> {
    let item = intent.item?;
    Some(ConvertRequest::Start {
        slot_id: intent.slot_id,
        request: UserMysekaiConvertStartRequest {
            mysekai_convert_item_id: item.item_id,
            convert_count: item.convert_count,
            choice_id: item.choice_id,
        },
    })
}

/// `ShortenSlotProcessAsync`'s request for the slot as the copies have it now:
/// `UserMysekaiConvertShortenRequest{ consumeJewelQuantity =
/// slot.ConsumedJewelQuantityForShorten }`.
pub fn shorten_request(ctx: ItemContext<'_>, intent: ShortenIntent) -> ConvertRequest {
    let model = SlotModel::new(ctx, intent.slot_id, intent.pass_slot);
    ConvertRequest::Shorten {
        slot_id: intent.slot_id,
        request: UserMysekaiConvertShortenRequest {
            consume_jewel_quantity: model.consumed_jewel_quantity_for_shorten(&ctx.machine()),
        },
    }
}

// ---------------------------------------------------------------------------
// The presenter's calls
// ---------------------------------------------------------------------------

/// `Presenter.ConfirmConvertStart(slotId, model)`: the item select page's
/// start button, with the detail panel's item as `model` and the panel's
/// count as its `ConvertCount`.
pub fn confirm_convert_start(screen: &mut ConvertScreen, ctx: ItemContext<'_>) {
    let selection = screen.start_selection();
    let waiting = screen.start_intent.is_some();
    let seen = screen.seen_mut();
    let Some((slot_id, item_id, count)) = selection else {
        // `OnStartConvert` runs only with a selected model.
        note_once(
            seen,
            "OnStartConvert with no selected item: no start confirm dialog".to_owned(),
        );
        return;
    };
    let item = item_model(ctx, item_id, seen).map(|model| StartItem {
        item_id: model.id(),
        // `model.ChoiceId`: the blueprint or record obtain row's choice this
        // model was made for (0 for a normal item and for a random row).
        choice_id: model.choice(),
        convert_count: count,
    });
    if item.is_none() {
        note_once(
            seen,
            format!(
                "ConfirmConvertStart(slot {slot_id}): the item {item_id}'s model is not read on this frame; the confirm dialog's OK refuses the start by name"
            ),
        );
    }
    info!(
        "[convert] ConfirmConvertStart(slot {slot_id}, item {item_id}, count {count}): Show2ButtonDialog(MysekaiConvertStartConfirmDialog, onOK: _ => ConvertStartProcess, onCancel: null).Setup(new MysekaiConvertStartConfirmViewModel(model))"
    );
    if waiting {
        note_once(
            seen,
            format!("ConfirmConvertStart(slot {slot_id}) while the confirm dialog's request waits: the waiting request is kept"),
        );
        return;
    }
    screen.start_intent = Some(StartIntent { slot_id, item });
}

/// `Presenter.OnShortenSlot(slot)`: a processing cell's skip button.
pub fn on_shorten_slot(screen: &mut ConvertScreen, ctx: ItemContext<'_>, slot_id: i32) {
    let pass_slot = ctx.machine().is_pass_slot(slot_id);
    let waiting = screen.shorten_intent.is_some();
    if !waiting {
        screen.shorten_intent = Some(ShortenIntent { slot_id, pass_slot });
    }
    info!(
        "[convert] OnShortenSlot(slot {slot_id}): Show2ButtonDialog(MysekaiConvertFurnitureShortenDialog, onOK: _ => OnClickOkShortenSlot, onCancel: null).Setup(slot)"
    );
    if waiting {
        note_once(
            screen.seen_mut(),
            format!("OnShortenSlot(slot {slot_id}) while the skip dialog's request waits: the waiting request is kept"),
        );
    }
}

/// The start confirm dialog's OK: `ConvertStartProcess(slotId, model)`, whose
/// two pass dialogs return before a request is made.
pub fn convert_start_process(screen: &mut ConvertScreen, ctx: ItemContext<'_>, intent: StartIntent) {
    let Some(item) = intent.item else {
        note_once(
            screen.seen_mut(),
            format!(
                "ConvertStartProcess(slot {}): the dialog's model could not be read; no start is sent",
                intent.slot_id
            ),
        );
        return;
    };
    let step = start_step(ctx, intent.slot_id, item, screen.seen_mut());
    match step {
        StartStep::NotExpired => {
            info!("[convert] ShowConvertNotExpiredPassDialog: Show1ButtonDialog(Common1ButtonDialog, {NOT_EXPIRED_PASS}, {OK}, onOK: _ => ChangeView(ViewType.Slot), Layer_Dialog, {DIALOG_SIZE})");
            screen.pass_dialog = Some(PassDialog::waiting(StartStep::NotExpired));
        }
        StartStep::UpperLimit => {
            info!("[convert] ShowConvertUpperLimitPassDialog: Show1ButtonDialog(Common1ButtonDialog, {UPPER_LIMIT_PASS}, {OK}, onOK: _ => ItemSelectView.Refresh, Layer_Dialog, {DIALOG_SIZE})");
            screen.pass_dialog = Some(PassDialog::waiting(StartStep::UpperLimit));
        }
        StartStep::Send => {
            screen.start_pending = Some(intent);
        }
    }
}

/// The skip dialog's OK: `OnClickOkShortenSlot(slot)`, which sends only from
/// a processing slot.
pub fn click_ok_shorten_slot(
    screen: &mut ConvertScreen,
    ctx: ItemContext<'_>,
    intent: ShortenIntent,
) {
    let model = SlotModel::new(ctx, intent.slot_id, intent.pass_slot);
    if model.state() != SlotState::Processing {
        info!(
            "[convert] OnClickOkShortenSlot(slot {}): the slot is {:?}, not Processing; no skip is sent",
            intent.slot_id,
            model.state()
        );
        return;
    }
    screen.shorten_pending = Some(intent);
}

/// `SlotStatusState.PlayConvertStartAsync(slotId)`: `SlotView` passes it to
/// the cell of that slot. The cell's timer view writes its remaining time and
/// hides the skip, and its button view shows the processing, nothing-to-receive
/// look, both once; the gauge's `PlayConvertStartAsync(model)` shows the
/// processing content and holds the wave hidden through its
/// `_settingAnimationSeconds`, then shows it at the state read again (the
/// slot's own `NormalizedProgressTime`) and fades it in. The slot's next read
/// shows the same wave; the cells' own tween (the gauge's curve) is named.
pub fn play_convert_start(
    screen: &mut ConvertScreen,
    ctx: ItemContext<'_>,
    layouts: &UiLayouts,
    slot_id: i32,
) {
    let model = SlotModel::new(ctx, slot_id, ctx.machine().is_pass_slot(slot_id));
    screen.slot_play_convert_start(&model, ctx, layouts);
    note_once(
        screen.seen_mut(),
        "the gauge's PlayConvertStartAsync holds the wave hidden through its _settingAnimationSeconds and fades it in; the slot's next read shows it at the slot's progress (named)".to_owned(),
    );
}

/// `SlotStatusState.PlayShortenAsync(slotId, currentProgress)`: the cell's
/// gauge tweens the wave's progress to 1 over `_receiveAnimationSeconds`.
pub fn play_shorten(screen: &mut ConvertScreen, slot_id: i32, progress: f32) {
    screen.slot_play_shorten(slot_id, progress);
    note_once(
        screen.seen_mut(),
        "DisableTapScreen / EnableTapScreen around the skip's wave: the click cover this tree has is the layout editor's (named)".to_owned(),
    );
}

// ---------------------------------------------------------------------------
// The two requests
// ---------------------------------------------------------------------------

/// A start or a skip sent on the link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertRequest {
    /// `PostUserMysekaiConvertStartApi`.
    Start {
        slot_id: i32,
        request: UserMysekaiConvertStartRequest,
    },
    /// `PostUserMysekaiConvertShortenApi`.
    Shorten {
        slot_id: i32,
        request: UserMysekaiConvertShortenRequest,
    },
    /// `PostUserMysekaiConvertReceiveApi`.
    Receive {
        slot_id: i32,
        request: UserMysekaiConvertReceiveRequest,
    },
}

impl ConvertRequest {
    /// The call it is sent as (`Call::Convert`, whose slot is the API's path
    /// parameter).
    pub fn call(&self) -> Call {
        match self {
            Self::Start { slot_id, request } => Call::Convert {
                slot_id: *slot_id,
                post: ConvertPost::Start(*request),
            },
            Self::Shorten { slot_id, request } => Call::Convert {
                slot_id: *slot_id,
                post: ConvertPost::Shorten(*request),
            },
            Self::Receive { slot_id, request } => Call::Convert {
                slot_id: *slot_id,
                post: ConvertPost::Receive(*request),
            },
        }
    }

    pub fn slot_id(&self) -> i32 {
        match self {
            Self::Start { slot_id, .. }
            | Self::Shorten { slot_id, .. }
            | Self::Receive { slot_id, .. } => *slot_id,
        }
    }

    pub fn api_name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "PostUserMysekaiConvertStartApi",
            Self::Shorten { .. } => "PostUserMysekaiConvertShortenApi",
            Self::Receive { .. } => "PostUserMysekaiConvertReceiveApi",
        }
    }
}

/// The request in flight and the call's number (`ServerPending`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertSent {
    pub request: ConvertRequest,
    pub sent: ServerPending,
}

/// The chain's one request (each of the source's two API services holds one).
#[derive(Resource, Debug, Default)]
pub struct MysekaiConvertService {
    in_flight: Option<ConvertSent>,
    /// The last request, whether it succeeded or not.
    last: Option<ConvertSent>,
    /// Why the last call was refused before it was sent.
    refused: Option<String>,
}

impl MysekaiConvertService {
    /// The request in flight.
    pub fn in_flight(&self) -> Option<&ConvertSent> {
        self.in_flight.as_ref()
    }

    /// The newest request (the one in flight, or the last answered).
    pub fn last(&self) -> Option<&ConvertSent> {
        self.in_flight.as_ref().or(self.last.as_ref())
    }

    /// The server call this service waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub fn server_pending(&self) -> Option<ServerPending> {
        self.in_flight.as_ref().map(|in_flight| in_flight.sent)
    }

    /// Why the last call was not sent.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// `APIExecutor.Execute` of the request: it is sent on the link; `Err`
    /// names the request that was not sent while another waits.
    pub fn send(&mut self, link: &mut ServerLink, request: ConvertRequest) -> Result<u32, String> {
        if let Some(in_flight) = &self.in_flight {
            let reason = format!(
                "{} of slot {} (seq {:?}) is still unanswered; {} of slot {} is not sent",
                in_flight.request.api_name(),
                in_flight.request.slot_id(),
                in_flight.sent.seq,
                request.api_name(),
                request.slot_id()
            );
            self.refused = Some(reason.clone());
            return Err(reason);
        }
        self.refused = None;
        let call = request.call();
        let api = request.api_name();
        let slot_id = request.slot_id();
        let seq = link.call(call);
        self.in_flight = Some(ConvertSent {
            request: request.clone(),
            sent: ServerPending {
                seq: Some(seq),
                call: CALL,
            },
        });
        info!("[convert] {api} (slot {slot_id}) sent as seq {seq}; one request in flight until its answer");
        Ok(seq)
    }

    /// The answer of `seq`: the request it releases, or `None` for a number
    /// that is not the request in flight (which keeps waiting).
    pub fn answer(&mut self, seq: u32) -> Option<ConvertSent> {
        let in_flight = self.in_flight.take()?;
        if in_flight.sent.seq != Some(seq) {
            self.in_flight = Some(in_flight);
            return None;
        }
        self.last = Some(in_flight.clone());
        Some(in_flight)
    }

    /// Takes the request in flight whatever answer arrives (the lost-session
    /// notice of a number the link no longer waits on).
    pub fn take_unmatched(&mut self) -> Option<ConvertSent> {
        let in_flight = self.in_flight.take()?;
        self.last = Some(in_flight.clone());
        Some(in_flight)
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Update, after the screen's taps and the dialogs that answer them: a start
/// or a skip whose dialog OK'd is sent. The start's `ConvertStartProcess`
/// runs here first, so a pass dialog it shows waits for the start to send.
#[allow(clippy::too_many_arguments)]
pub fn send_pending(
    mut link: ResMut<ServerLink>,
    screen: Option<ResMut<ConvertScreen>>,
    tables: Res<MasterTables>,
    client: Option<Res<ClientUserData>>,
    copies: Option<Res<ClientConvertSlots>>,
    inventory: Option<Res<ClientMysekaiInventory>>,
    real: Res<Time<Real>>,
    mut service: ResMut<MysekaiConvertService>,
) {
    let Some(mut screen) = screen else {
        return;
    };
    let (Some(masters), Some(user)) = (
        tables.convert.as_ref(),
        read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()),
    ) else {
        return;
    };
    let now = now_ms(client.as_deref(), real.elapsed_secs());
    let ctx = ItemContext {
        masters,
        user: &user,
        now,
    };
    if let Some(intent) = screen.start_pending.take() {
        convert_start_process(&mut screen, ctx, intent);
    }
    let mut notes: Vec<String> = Vec::new();
    if let Some(intent) = screen.start_pending.take() {
        match start_request(intent) {
            Some(request) => match service.send(&mut link, request) {
                Ok(seq) => screen.start_sent = Some((intent.slot_id, seq)),
                Err(reason) => notes.push(reason),
            },
            None => notes.push(format!(
                "ConvertStartProcessAsync(slot {}): no model, no request",
                intent.slot_id
            )),
        }
    }
    if let Some(intent) = screen.shorten_pending.take() {
        let request = shorten_request(ctx, intent);
        match service.send(&mut link, request) {
            Ok(seq) => screen.shorten_sent = Some((intent.slot_id, seq)),
            Err(reason) => notes.push(reason),
        }
    }
    for note in notes {
        note_once(screen.seen_mut(), note);
    }
}

/// Update, exclusive, after the start's request step: the pass dialog
/// `ConvertStartProcess` asked for shows as the one-button common dialog,
/// and its answer runs the dialog's onOK (the close button, an outside tap
/// and the back key answer as its OK).
pub fn pass_dialogs(world: &mut World) {
    let Some(dialog) = world
        .get_resource::<ConvertScreen>()
        .and_then(|screen| screen.pass_dialog)
    else {
        return;
    };
    let (caller, key) = match dialog.step {
        StartStep::NotExpired => (
            "MysekaiConvertFurniturePresenter.ShowConvertNotExpiredPassDialog",
            NOT_EXPIRED_PASS,
        ),
        StartStep::UpperLimit => (
            "MysekaiConvertFurniturePresenter.ShowConvertUpperLimitPassDialog",
            UPPER_LIMIT_PASS,
        ),
        StartStep::Send => {
            world.resource_mut::<ConvertScreen>().pass_dialog = None;
            return;
        }
    };
    let Some(shown) = dialog.shown else {
        let body = common_dialog::wording(world, caller, key);
        let shown = common_dialog::open(world, caller, body, Buttons::One { ok: OK }, None);
        let mut screen = world.resource_mut::<ConvertScreen>();
        screen.pass_dialog = shown.map(|id| PassDialog {
            step: dialog.step,
            shown: Some(id),
        });
        if shown.is_none() {
            note_once(screen.seen_mut(), format!("{caller}: the common dialog did not show; its onOK does not run"));
        }
        return;
    };
    if common_dialog::answer(world, shown).is_none() {
        return;
    }
    world.resource_scope(|world, mut screen: Mut<ConvertScreen>| {
        screen.pass_dialog = None;
        match dialog.step {
            StartStep::NotExpired => {
                info!("[convert] {caller}: onOK; ChangeView(ViewType.Slot)");
                screen.change_view(ViewType::Slot);
            }
            _ => {
                info!("[convert] {caller}: onOK; ItemSelectView.Refresh()");
                let (Some(masters), Some(user)) = (
                    world.resource::<MasterTables>().convert.as_ref(),
                    read_user(
                        world.get_resource::<ClientUserData>(),
                        world.get_resource::<ClientConvertSlots>(),
                        world.get_resource::<ClientMysekaiInventory>(),
                    ),
                ) else {
                    note_once(screen.seen_mut(), format!("{caller}: the user's convert data is not in; the item select page is not refreshed"));
                    return;
                };
                let real = world.resource::<Time<Real>>().elapsed_secs();
                let ctx = ItemContext {
                    masters,
                    user: &user,
                    now: now_ms(world.get_resource::<ClientUserData>(), real),
                };
                screen.refresh_item_select(ctx);
            }
        }
    });
}

/// Update, after the reader applied the answer's sections: a start's or a
/// skip's answer runs its callbacks. The reply's tables are in the copies
/// already (`UpdateAll` of the arrival's sections).
#[allow(clippy::too_many_arguments)]
pub fn answers(
    mut answers: MessageReader<Answered>,
    mut service: ResMut<MysekaiConvertService>,
    sent: Res<SentCalls>,
    screen: Option<ResMut<ConvertScreen>>,
    layouts: Res<UiLayouts>,
    tables: Res<MasterTables>,
    client: Option<Res<ClientUserData>>,
    copies: Option<Res<ClientConvertSlots>>,
    inventory: Option<Res<ClientMysekaiInventory>>,
    real: Res<Time<Real>>,
    mut receive: ResMut<super::receive::ConvertReceive>,
) {
    let Some(mut screen) = screen else {
        return;
    };
    for answer in answers.read() {
        // Not this chain's call: its own handler reads it.
        if answer.call != CALL {
            continue;
        }
        // A lost session answers a call of this kind that no longer waits in
        // the link, which is the request in flight here (another send took its
        // number): it finishes on the failure path rather than waiting on.
        let lost_for_this_request = matches!(&answer.outcome, Err(refusal) if refusal.code == LINK_LOST)
            && matches!(sent.0.get(&answer.seq), Some(Call::Convert { .. }));
        let request = if service
            .in_flight()
            .is_some_and(|in_flight| in_flight.sent.seq == Some(answer.seq))
        {
            service.answer(answer.seq)
        } else if lost_for_this_request {
            service.take_unmatched()
        } else {
            None
        };
        let Some(request) = request else {
            continue;
        };
        let api = request.request.api_name();
        let slot_id = request.request.slot_id();
        // The service's `status == Success` is the API's success state only:
        // any other answer, a reply of another kind included, runs nothing.
        let receive_request = matches!(request.request, ConvertRequest::Receive { .. });
        let success = match &answer.outcome {
            Ok(Reply::Convert(_)) if !receive_request => true,
            Ok(Reply::ConvertReceive(_)) if receive_request => true,
            Ok(other) => {
                error!("[convert] {api} (slot {slot_id}, seq {:?}): the answer is {other:?}, not this API's reply; the request's callbacks do not run", answer.seq);
                false
            }
            Err(refusal) => {
                error!("[convert] {api} (slot {slot_id}, seq {:?}): refused ({refusal}); the request's callbacks do not run", answer.seq);
                false
            }
        };
        match request.request {
            ConvertRequest::Start { .. } => {
                screen.start_sent = None;
                if !success {
                    continue;
                }
                let (Some(masters), Some(user)) = (
                    tables.convert.as_ref(),
                    read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()),
                ) else {
                    note_once(screen.seen_mut(), "the start's answer: the user's convert data is not in; the slot's next read shows the slot".to_owned());
                    continue;
                };
                let ctx = ItemContext {
                    masters,
                    user: &user,
                    now: now_ms(client.as_deref(), real.elapsed_secs()),
                };
                info!("[convert] {api} (slot {slot_id}, seq {:?}): success; ChangeView(ViewType.Slot), SetNeedsUpdateSlot({slot_id}, false), PlayConvertStartAsync({slot_id}), SetNeedsUpdateSlot({slot_id}, true)", answer.seq);
                screen.change_view(ViewType::Slot);
                // `SlotStatusState.PlayConvertStartAsync(slotId)`.
                play_convert_start(&mut screen, ctx, &layouts, slot_id);
            }
            ConvertRequest::Shorten { .. } => {
                screen.shorten_sent = None;
                if !success {
                    continue;
                }
                let (Some(masters), Some(user)) = (
                    tables.convert.as_ref(),
                    read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()),
                ) else {
                    note_once(screen.seen_mut(), "the skip's answer: the user's convert data is not in; the wave stays where the next read puts it".to_owned());
                    continue;
                };
                let ctx = ItemContext {
                    masters,
                    user: &user,
                    now: now_ms(client.as_deref(), real.elapsed_secs()),
                };
                // `slot.NormalizedProgressTime` of the dialog's model (the
                // slot as the answer leaves it), read again here.
                let progress = SlotModel::new(ctx, slot_id, ctx.machine().is_pass_slot(slot_id))
                    .normalized_progress_time();
                info!("[convert] {api} (slot {slot_id}, seq {:?}): success; SetNeedsUpdateSlot({slot_id}, false), DisableTapScreen, PlayShortenAsync({slot_id}, {progress}), EnableTapScreen, SetNeedsUpdateSlot({slot_id}, true)", answer.seq);
                play_shorten(&mut screen, slot_id, progress);
            }
            ConvertRequest::Receive { .. } => {
                // `await ExecuteMysekaiConvertReceiveAPIAsync`: the obtained
                // resources, or none on any other answer.
                let obtained = match &answer.outcome {
                    Ok(Reply::ConvertReceive(reply)) if success => reply.obtained_resources.clone(),
                    _ => Vec::new(),
                };
                receive.answered(slot_id, answer.seq, obtained);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn start(slot_id: i32, item: i32, count: i32, choice: i32) -> ConvertRequest {
        ConvertRequest::Start {
            slot_id,
            request: UserMysekaiConvertStartRequest {
                mysekai_convert_item_id: item,
                convert_count: count,
                choice_id: choice,
            },
        }
    }

    /// The start's request is `UserMysekaiConvertStartRequest`'s three fields
    /// and the slot of `PostUserMysekaiConvertStartApi`.
    #[test]
    fn a_start_sends_the_models_item_count_and_choice() {
        let intent = StartIntent {
            slot_id: 1,
            item: Some(StartItem {
                item_id: 7,
                choice_id: 3,
                convert_count: 2,
            }),
        };
        let request = start_request(intent).expect("a model");
        assert_eq!(request, start(1, 7, 2, 3));
        assert_eq!(
            request.call(),
            Call::Convert {
                slot_id: 1,
                post: ConvertPost::Start(UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 7,
                    convert_count: 2,
                    choice_id: 3,
                }),
            }
        );
        assert_eq!(request.call().kind(), CallKind::Convert);
        assert_eq!(request.api_name(), "PostUserMysekaiConvertStartApi");
        // A dialog without a readable model sends nothing.
        assert!(start_request(StartIntent { slot_id: 1, item: None }).is_none());
    }

    /// The skip's request is the slot's price on the call's path `slotId`.
    #[test]
    fn a_skip_sends_the_slots_price() {
        let request = ConvertRequest::Shorten {
            slot_id: 2,
            request: UserMysekaiConvertShortenRequest {
                consume_jewel_quantity: 10,
            },
        };
        assert_eq!(
            request.call(),
            Call::Convert {
                slot_id: 2,
                post: ConvertPost::Shorten(UserMysekaiConvertShortenRequest {
                    consume_jewel_quantity: 10,
                }),
            }
        );
        assert_eq!(request.slot_id(), 2);
        assert_eq!(request.api_name(), "PostUserMysekaiConvertShortenApi");
    }

    /// Only the request in flight's own number releases it.
    #[test]
    fn only_the_requests_own_number_releases_it() {
        let sent = ConvertSent {
            request: start(0, 7, 1, 0),
            sent: ServerPending {
                seq: Some(4),
                call: CALL,
            },
        };
        let mut service = MysekaiConvertService::default();
        service.in_flight = Some(sent.clone());
        assert!(service.answer(5).is_none());
        assert_eq!(service.in_flight(), Some(&sent));
        assert_eq!(service.server_pending(), Some(sent.sent));
        assert_eq!(service.answer(4), Some(sent.clone()));
        assert!(service.in_flight().is_none());
        assert_eq!(service.last(), Some(&sent));
    }

    /// `start_step`: a slot the pass extends reads `GetOnceConvertibleCount`
    /// (0 without the pass); a slot it does not always sends.
    #[test]
    fn only_a_pass_slot_is_held_to_its_pass_count() {
        use moly_masters::convert::ConvertMasters;
        use super::super::model::ConvertUserState;
        let masters = ConvertMasters {
            // `slotNoUpperLimit` 1 and `slotNoUpperLimitForMysekaiColorfulPass`
            // 3: slot 1 is the pass's first.
            slots: Some((1, 3)),
            ..ConvertMasters::empty()
        };
        let mut user = ConvertUserState::default();
        user.level = Some(1);
        let ctx = ItemContext {
            masters: &masters,
            user: &user,
            now: 0,
        };
        let item = StartItem {
            item_id: 1,
            choice_id: 0,
            convert_count: 1,
        };
        assert!(ctx.machine().is_pass_slot(1));
        assert!(!ctx.machine().is_pass_slot(0));
        let mut seen = BTreeSet::new();
        assert_eq!(start_step(ctx, 0, item, &mut seen), StartStep::Send);
        // A pass slot reads 0 without the pass, whatever the item's bounds.
        assert_eq!(start_step(ctx, 1, item, &mut seen), StartStep::NotExpired);
    }
}
