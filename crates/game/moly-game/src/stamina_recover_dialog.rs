//! The boost stamina recovery dialog (`MysekaiRecoverBoostStaminaDialog`,
//! DialogType 321) and its confirmation (`BoostStaminaRecoverConfirmDialog`,
//! 322): the menu dialog's "recover stamina" button target.
//!
//! ## Entry and exit (the dialog slot, not the layer stack)
//!
//! `MysekaiMenuDialog.OnClickRecoverBoostStaminaButton` calls
//! `ScreenManager.Show2ButtonDialog<MysekaiRecoverBoostStaminaDialog>(321,
//! Layer_Dialog, allowCloseExternal true)` and `Setup(onRecoverFinish =
//! UpdateStaminaGateView)`, which this module reads as
//! [`crate::menu_dialog::RecoverStaminaRequest`], then the menu dialog closes.
//! A tap outside the recover dialog, its close button, its cancel button, and
//! the back key close it; its OK opens
//! `Show2ButtonDialog<BoostStaminaRecoverConfirmDialog>(322, …)`. The
//! confirmation's OK sends `PostUserMysekaiBoostStaminaRecoveryApi` (the
//! recovery type and `userMysekaiStaminaRecoveryConsumeResources`), and its
//! cancel, close button, outside tap or back key return to the recover dialog.
//! The answer shows the completion notice (`MSG_BOOST_STAMINA_RECOVER_COMPLETE`)
//! and closes the recover dialog, whose close runs `onRecoverFinish` (the menu
//! cell, [`crate::menu_dialog::RecoverFinished`]) and the field HUD's
//! `UpdateStaminaView` (`HarvestStaminaRefresh`).
//!
//! ## The two top tabs (`_liveBoostConsumptionOrRecoverTabGroup`)
//!
//! - **Consumption** (`ConvertLiveBonusToBoostStaminaView`): live bonus
//!   bought with boost stamina. The slider runs 1..`GetSliderViewData`'s bound
//!   `max(1, min(live bonus, ⌊(boost maximum − boost) · boostQuantity /
//!   recoveryBoostStamina⌋))`; each count spends the benefit for
//!   `boostQuantity` live bonus and restores `recoveryBoostStamina` boost
//!   stamina. A short live bonus shows `WORD_LIVE_BONUS_NOT_ENOUGH`, a full
//!   pool shows the limit balloon.
//! - **Recover** (`UIPartsTabGroup`, `_recoverContentSelectTabGroup`): the item
//!   tab lists every held boost item (each row's amount and the boost the
//!   rows' values add up to, as `OnChangeSelectedItem` writes them); the
//!   crystal tab sets the crystal count from the wallet and the missing room.
//!
//! ## OK's three routes (`OnClickOk`)
//!
//! 1. the consumption tab → `boost` with `[live bonus (0), slider value]` and
//!    a confirmation for `ConvertLiveBonusToBoostStamina(…).Item2`;
//! 2. the crystal tab → `jewel` with `[crystals (1), slider value]` and a
//!    confirmation for the multiplied value;
//! 3. the item tab → `boost_item` with one `[boost item id, quantity]` per row
//!    that holds a quantity, and a confirmation for
//!    `Math.Min(staminaMax − boost, Σ row values)`.
//!
//! The positive button's `enabled` (a disabled `CustomButton` dispatches
//! nothing, and its cover shows): the crystal tab from
//! `CurrentTotalJewel >= price`, the item tab from the rows' quantities, the
//! consumption tab from `UserLiveBonus` alone.
//!
//! ## The confirmation
//!
//! `BoostStaminaRecoverConfirmDialog.Initialize(StaminaData)` shows the number
//! changer from the live bonus before to after, the before views at the
//! client's pools and the after views at those pools with the boost stamina
//! the request restores, and the `MAX` warnings of a view at the boost
//! maximum. Its body is `MSG_BOOST_STAMINA_RECOVER_CONFIRM` with that boost
//! stamina.
//!
//! ## Named gaps
//!
//! - `UserLiveBonus` and `UserDataManager.CurrentTotalJewel` are not carried by
//!   any response of this product: both enter as zero and are named. The
//!   crystal unit's price is `CRYSTAL_UNIT_PRICE` (the source reads
//!   `MasterJewelBehavior`, which the server's master set does not carry; the
//!   server decides what a crystal recovery restores). The held boost items
//!   have no per-user table: each master boost item enters held once (named).
//! - The item tab's list is a `FixedSizeReuseScroller` of
//!   `LiveBoostRecoverListCell` cells whose prefab is not part of the
//!   documents: each row is drawn as text (the master id, the held amount and
//!   the row's amount) under the list's content, not as the source cell. The
//!   rows do not reflow beyond the content's height; the list does not
//!   scroll.
//! - The `UIPartsSelectCost` numeric input field has no decode chain for this
//!   client's player data and stays disabled; the slider's track sets the
//!   count.
//! - `UIPartsNumberChanger`'s after-number colour is a TMP colour code; the
//!   text keeps its serialized colour.
//! - The commercial-transaction-law button opens a web view (outside this
//!   product); a tap on it is logged and draws nothing.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use moly_link::ServerLink;
use moly_masters::recovery::{BoostItemRow, ItemRoom, RecoveryRow};
use moly_protocol::api::BoostStaminaRecoveryType;
use moly_protocol::rows::Stamina;
use moly_protocol::{Call, CallKind, Reply};
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureState};
use crate::harvest::action::HarvestPlayerModel;
use crate::harvest::HarvestStaminaRefresh;
use crate::menu_dialog::{stamina_view_state, RecoverStaminaRequest};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::client::ClientUserData;
use crate::userdata::stamina_recover::{self, request};
use crate::userdata::{Answered, ServerPending};

/// `DialogType.MysekaiRecoverBoostStaminaDialog`.
const DIALOG: DialogType = DialogType(321);
/// `DialogType.BoostStaminaRecoverConfirmDialog`.
const CONFIRM: DialogType = DialogType(322);
/// The document keys of the two dialogs.
const DIALOG_KEY: &str = "RecoverBoostStamina";
const CONFIRM_KEY: &str = "BoostStaminaConfirm";
const DIALOG_Z: f32 = 300.0;
const CONFIRM_Z: f32 = 310.0;
/// `UserMysekaiStaminaRecoveryConsumeResource.resourceId` the live bonus and
/// the crystals are sent with.
const LIVE_BONUS_RESOURCE: i32 = 0;
const CRYSTAL_RESOURCE: i32 = 1;
/// What one crystal recovery count spends, in `CurrentTotalJewel`.
const CRYSTAL_UNIT_PRICE: i32 = 1;
/// The height and gap of the item rows this view draws.
const ROW_HEIGHT: f32 = 88.0;
const ROW_GAP: f32 = 24.0;
/// `MSG_BOOST_STAMINA_RECOVER_COMPLETE`: the completion notice's text.
pub(crate) const RECOVER_COMPLETE: &str = "MSG_BOOST_STAMINA_RECOVER_COMPLETE";
/// `MSG_BOOST_STAMINA_RECOVER_CONFIRM`: the confirmation's body.
pub(crate) const CONFIRM_BODY_WORDING: &str = "MSG_BOOST_STAMINA_RECOVER_CONFIRM";

/// The wording keys the two dialogs write; the shell's glyph set takes them.
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_LIVE_BONUS",
    "WORD_ITEM_CRYSTAL",
    "WORD_BOOST_RECOVER_METHOD_ITEM",
    "WORD_BOOST_RECOVER_METHOD_JEWEL",
    "WORD_SETTING_UP",
    "WORD_LIVE_BONUS_NOT_ENOUGH",
    "WORD_SHOP_SPECIFIED_COMMERCIAL_TRANSACTION_LAW",
    "WORD_DECIDE",
    "WORD_CANCEL",
    "MSG_NOTHING_RECOVER",
    "MSG_OVER_BOOST_STAMINA_RECOVER",
    "MSG_NOT_ENOUGH_JEWEL",
    CONFIRM_BODY_WORDING,
    RECOVER_COMPLETE,
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct Recover {
    live: Option<Live>,
    confirm: Option<ConfirmOpen>,
    pending: Option<ServerPending>,
    request: Option<moly_protocol::api::UserMysekaiStaminaRequest>,
    requested: i32,
    rows: Vec<RowNodes>,
    /// The item content's drawn size: the rows' height, and its width.
    content_height: Option<f32>,
    content_width: Option<f32>,
    shown: Option<String>,
}

struct Live {
    id: DialogId,
    top: Top,
    method: Method,
    live_bonus: i32,
    crystals: i32,
    convert_max: i32,
    convert_value: i32,
    jewel_max: i32,
    jewel_value: i32,
    items: Vec<ItemRow>,
    stamina: Stamina,
    max: moly_masters::StaminaMax,
    row: RecoveryRow,
    boosts: Vec<BoostItemRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Top {
    Consumption,
    Recover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Item,
    Crystal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemRow {
    item: BoostItemRow,
    held: i32,
    selected: i32,
}

impl ItemRow {
    /// `GetSelectedItemSumValue`'s value of the row.
    fn value(&self, row: RecoveryRow) -> i32 {
        moly_masters::boost_item_value(self.item.recovery_value, self.selected, row)
    }
}

struct ConfirmOpen {
    id: DialogId,
    recover: i32,
    changer: (i32, i32),
}

#[derive(Clone, Debug)]
struct RowNodes {
    entity: Entity,
}

#[derive(Component)]
struct RecoverView;

#[derive(Component)]
struct ConfirmView;

#[derive(Resource)]
struct Spawned;

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Recover>()
        .add_systems(Update, spawn_when_ready)
        .add_systems(
            Update,
            (open, click, read_answers)
                .chain()
                .after(crate::menu_dialog::click)
                .before(crate::ui_layers::advance),
        )
        .add_systems(
            Update,
            process
                .after(crate::ui_layers::advance)
                .before(crate::ui_layout::render),
        )
        .add_systems(Update, crate::menu_dialog::on_recover_finish);
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

fn component<'a>(doc: &'a UiPrefab, id: i64) -> &'a UiComponent {
    doc.nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|component| component.path_id == id)
        .unwrap_or_else(|| panic!("{}: component {id} is on no node", doc.prefab))
}

fn class_of<'a>(doc: &'a UiPrefab, id: i64, class: &str) -> &'a UiComponent {
    let component = component(doc, id);
    assert_eq!(component.class, class, "{}: component {id}", doc.prefab);
    component
}

fn reference(doc: &UiPrefab, fields: &Value, name: &str) -> i64 {
    let pointer = fields[name]
        .as_array()
        .filter(|pointer| pointer.len() == 2)
        .unwrap_or_else(|| panic!("{}: {name} is not a reference", doc.prefab));
    assert_eq!(pointer[0].as_i64(), Some(0), "{}: {name} is external", doc.prefab);
    pointer[1]
        .as_i64()
        .unwrap_or_else(|| panic!("{}: {name} has no path id", doc.prefab))
}

fn object_of(doc: &UiPrefab, id: i64) -> String {
    let index = doc
        .find(&format!("@{id}"))
        .unwrap_or_else(|error| panic!("{error}"));
    format!("@{}", doc.nodes[index].game_object_id)
}

fn path_of(doc: &UiPrefab, id: i64) -> String {
    let index = doc
        .find(&format!("@{id}"))
        .unwrap_or_else(|error| panic!("{error}"));
    doc.nodes[index].path.clone()
}

fn common_button(doc: &UiPrefab, id: i64) -> String {
    let wrapper = class_of(doc, id, "Sekai.UI.UIPartsCommonButton");
    object_of(doc, reference(doc, &wrapper.fields, "customButton"))
}

fn select_cost(doc: &UiPrefab, id: i64) -> (i64, i64, i64, i64) {
    let select = class_of(doc, id, "Sekai.UI.UIPartsSelectCost");
    (
        reference(doc, &select.fields, "slider"),
        reference(doc, &select.fields, "numText"),
        reference(doc, &select.fields, "decrementButton"),
        reference(doc, &select.fields, "incrementButton"),
    )
}

/// The dialog's nodes.
struct Page {
    positive: String,
    positive_cover: String,
    positive_label: String,
    negative: String,
    negative_label: String,
    close: String,
    window: String,
    tabs: (String, String),
    item_cell: (String, String),
    crystal_cell: (String, String),
    convert_body: String,
    convert_decrement: String,
    convert_increment: String,
    convert_slider: String,
    convert_text: String,
    convert_shortage: String,
    convert_balloon: String,
    recovery_body: String,
    item_content: String,
    item_nothing: String,
    item_balloon: String,
    item_warning: String,
    item_current: String,
    item_after: String,
    crystal_decrement: String,
    crystal_increment: String,
    crystal_slider: String,
    crystal_text: String,
    crystal_max: String,
    crystal_balloon: String,
    crystal_attention: String,
    crystal_confirm: String,
}

impl Page {
    fn from_document(doc: &UiPrefab) -> Self {
        let dialog = doc
            .nodes
            .iter()
            .flat_map(|node| node.components.iter())
            .find(|component| component.class == "Sekai.Mysekai.MysekaiRecoverBoostStaminaDialog")
            .unwrap_or_else(|| panic!("{}: no MysekaiRecoverBoostStaminaDialog", doc.prefab));
        let field = |name: &str| reference(doc, &dialog.fields, name);
        let positive = field("positiveButton");
        let positive_cover = reference(
            doc,
            &class_of(doc, positive, "Sekai.UI.CustomButton").fields,
            "coverImage",
        );
        let toggles = class_of(
            doc,
            field("_liveBoostConsumptionOrRecoverTabGroup"),
            "Sekai.UI.CustomIndexToggleGroup",
        )
        .fields["indexToggles"]
            .as_array()
            .expect("the top tab toggles")
            .clone();
        assert_eq!(toggles.len(), 2, "{}: the top tabs are not two", doc.prefab);
        let toggle = |index: usize| {
            let id = toggles[index][1].as_i64().expect("a top tab toggle");
            object_of(doc, id)
        };
        let cells = class_of(
            doc,
            field("_recoverContentSelectTabGroup"),
            "Sekai.UI.UIPartsTabGroup",
        )
        .fields["selectorCells"]
            .as_array()
            .expect("the method cells")
            .clone();
        assert_eq!(cells.len(), 2, "{}: the recovery methods are not two", doc.prefab);
        let cell = |index: usize| {
            let id = cells[index][1].as_i64().expect("a method cell");
            let selector_cell = class_of(doc, id, "Sekai.UIPartsSelectorCell");
            (
                object_of(doc, reference(doc, &selector_cell.fields, "selectBannerCover")),
                object_of(doc, reference(doc, &selector_cell.fields, "button")),
            )
        };
        let convert = class_of(
            doc,
            field("convertLiveBonusToBoostStaminaView"),
            "Sekai.Mysekai.ConvertLiveBonusToBoostStaminaView",
        );
        let item = class_of(
            doc,
            field("_boostRecoveryMethodItemContent"),
            "Sekai.Mysekai.BoostStaminaRecoveryMethodItemContent",
        );
        let crystal = class_of(
            doc,
            field("_boostRecoveryMethodJewelContent"),
            "Sekai.Mysekai.BoostStaminaRecoveryMethodJewelContent",
        );
        let convert_select = reference(doc, &convert.fields, "_slider");
        let (convert_slider, convert_text, convert_decrement, convert_increment) = select_cost(doc, convert_select);
        let crystal_select = reference(doc, &crystal.fields, "slider");
        let (crystal_slider, crystal_text, crystal_decrement, crystal_increment) = select_cost(doc, crystal_select);
        let list = class_of(doc, reference(doc, &item.fields, "_listView"), "Sekai.FixedSizeReuseScroller");
        let scroll = class_of(doc, reference(doc, &list.fields, "scrollRect"), "Sekai.UI.CustomScrollRect");
        let text = |fields: &Value, name: &str| object_of(doc, reference(doc, fields, name));
        let content = object_of(doc, reference(doc, &scroll.fields, "m_Content"));
        Page {
            positive: object_of(doc, positive),
            positive_cover: object_of(doc, positive_cover),
            positive_label: object_of(doc, field("positiveButtonLabelMesh")),
            negative: object_of(doc, field("negativeButton")),
            negative_label: object_of(doc, field("negativeButtonLabelMesh")),
            close: common_button(doc, field("closeButton")),
            window: object_of(doc, doc.nodes[0].game_object_id),
            tabs: (toggle(0), toggle(1)),
            item_cell: cell(0),
            crystal_cell: cell(1),
            convert_body: path_of(doc, field("convertLiveBonusToBoostStaminaView")),
            convert_decrement: object_of(doc, convert_decrement),
            convert_increment: object_of(doc, convert_increment),
            convert_slider: object_of(doc, convert_slider),
            convert_text: object_of(doc, convert_text),
            convert_shortage: text(&convert.fields, "_liveBnousNotEnoughLabel"),
            convert_balloon: text(&convert.fields, "limitMessageBalloon"),
            recovery_body: path_of(doc, reference(doc, &dialog.fields, "_recoverContentSelectTabGroup"))
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_owned())
                .expect("the recovery tab group's parent"),
            item_content: content,
            item_nothing: path_of(doc, reference(doc, &item.fields, "_nothing")),
            item_balloon: path_of(doc, reference(doc, &item.fields, "_balloon")),
            item_warning: path_of(doc, reference(doc, &item.fields, "_warningMessage")),
            item_current: path_of(doc, reference(doc, &item.fields, "_currentBoostText")),
            item_after: path_of(doc, reference(doc, &item.fields, "_afterBoostText")),
            crystal_decrement: object_of(doc, crystal_decrement),
            crystal_increment: object_of(doc, crystal_increment),
            crystal_slider: object_of(doc, crystal_slider),
            crystal_text: object_of(doc, crystal_text),
            crystal_max: path_of(doc, reference(doc, &crystal.fields, "maxText")),
            crystal_balloon: path_of(doc, reference(doc, &crystal.fields, "limitMessageBalloon")),
            crystal_attention: path_of(doc, reference(doc, &crystal.fields, "jewelAttentionText")),
            crystal_confirm: path_of(doc, reference(doc, &crystal.fields, "confirmText")),
        }
    }
}

/// The confirmation's nodes.
struct ConfirmPage {
    message: String,
    changer: (i64, i64),
    before_normal: String,
    before_boost: String,
    after_normal: String,
    after_boost: String,
    before_warning: String,
    after_warning: String,
    positive: String,
    negative: String,
    close: String,
    window: String,
}

impl ConfirmPage {
    fn from_document(doc: &UiPrefab) -> Self {
        let dialog = doc
            .nodes
            .iter()
            .flat_map(|node| node.components.iter())
            .find(|component| component.class == "Sekai.Mysekai.BoostStaminaRecoverConfirmDialog")
            .unwrap_or_else(|| panic!("{}: no BoostStaminaRecoverConfirmDialog", doc.prefab));
        let field = |name: &str| reference(doc, &dialog.fields, name);
        let changer = class_of(doc, field("liveBoostNumberChanger"), "Sekai.UIPartsNumberChanger");
        let text = |name: &str| object_of(doc, field(name));
        ConfirmPage {
            message: path_of(doc, field("messageBodyTextMesh")),
            changer: (
                reference(doc, &changer.fields, "numberBeforeTextMesh"),
                reference(doc, &changer.fields, "numberAfterTextMesh"),
            ),
            before_normal: text("_beforeNormalStaminaView"),
            before_boost: text("_beforeBoostStaminaView"),
            after_normal: text("_afterNormalStaminaView"),
            after_boost: text("_afterBoostStaminaView"),
            before_warning: text("_beforeWarningMessage"),
            after_warning: text("_afterWarningMessage"),
            positive: object_of(doc, field("positiveButton")),
            negative: object_of(doc, field("negativeButton")),
            close: common_button(doc, field("closeButton")),
            window: object_of(doc, doc.nodes[0].game_object_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<Spawned>>,
) {
    if spawned.is_some() || !layouts.ready(DIALOG_KEY, &server) || !layouts.ready(CONFIRM_KEY, &server) {
        return;
    }
    commands.spawn((
        RecoverView,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., DIALOG_Z),
        RenderLayers::layer(SITEMAP_LAYER),
        UiPrefabView::new(DIALOG_KEY, SITEMAP_LAYER),
    ));
    commands.spawn((
        ConfirmView,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., CONFIRM_Z),
        RenderLayers::layer(SITEMAP_LAYER),
        UiPrefabView::new(CONFIRM_KEY, SITEMAP_LAYER),
    ));
    commands.insert_resource(Spawned);
    info!("[stamina-recover] the recover dialog and its confirmation views spawned");
}

// ---------------------------------------------------------------------------
// The dialog's inputs at `Setup`
// ---------------------------------------------------------------------------

/// `GetSliderViewData`'s bound, then `SetSliderPositionOneValueMax` and
/// `UpdateComsumptionNum(1)`.
fn conversion_bound(live_bonus: i32, stamina: Stamina, max: moly_masters::StaminaMax, row: RecoveryRow) -> i32 {
    moly_masters::recovery::conversion_slider_max(live_bonus, stamina.boost, max.boost, row)
}

/// `BoostStaminaRecoveryMethodJewelContent.Setup`'s bound, at least one.
fn jewel_bound(crystals: i32, stamina: Stamina, max: moly_masters::StaminaMax, row: RecoveryRow) -> i32 {
    let affordable = crystals / CRYSTAL_UNIT_PRICE.max(1);
    let fit = if row.recovery_boost_stamina == 0 {
        0
    } else {
        (max.boost.saturating_sub(stamina.boost) as f32 / row.recovery_boost_stamina as f32) as i32
    };
    affordable.min(fit).max(1)
}

/// `BoostStaminaRecoveryMethodItemContent.CreateData`.
fn item_rows(held: &[(i32, i32)], masters: &[BoostItemRow]) -> Vec<ItemRow> {
    held.iter()
        .filter_map(|(id, quantity)| {
            let item = masters.iter().find(|item| item.id == *id)?;
            (*quantity != 0).then_some(ItemRow {
                item: *item,
                held: *quantity,
                selected: 0,
            })
        })
        .collect()
}

/// `OnGetItemSelectNumMax` for `items[index]`.
fn row_max(items: &[ItemRow], index: usize, row: RecoveryRow, max: moly_masters::StaminaMax, boost: i32) -> i32 {
    let item = &items[index];
    let others = items
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .map(|(_, other)| other.value(row))
        .fold(0i32, i32::wrapping_add);
    moly_masters::recovery::item_select_max(
        item.item,
        ItemRoom {
            boost_max: max.boost,
            boost,
            floor: 1,
        },
        others,
        item.held,
    )
}

// ---------------------------------------------------------------------------
// Open: the menu dialog's recover button
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn open(
    mut lines: MessageReader<RecoverStaminaRequest>,
    mut screens: ResMut<ScreenManager>,
    mut recover: ResMut<Recover>,
    shared: Option<Res<moly_masters::load::MasterTables>>,
    user: Option<Res<ClientUserData>>,
    model: Option<Res<HarvestPlayerModel>>,
) {
    for RecoverStaminaRequest(menu) in lines.read() {
        if recover.live.is_some() || recover.confirm.is_some() {
            info!("[stamina-recover] the recover dialog is already shown; no second instance");
            continue;
        }
        let caller = "MysekaiMenuDialog.OnClickRecoverBoostStaminaButton";
        let Some(tables) = shared.as_deref().filter(|tables| tables.ready()) else {
            error!("[stamina-recover] {caller}: the master tables are not loaded; the dialog cannot show");
            continue;
        };
        let (Some(row), Some(max)) = (tables.stamina_recovery, tables.stamina_max) else {
            error!("[stamina-recover] {caller}: mysekaiStaminaRecovery or mysekaiStaminas did not load; the dialog cannot show");
            continue;
        };
        let boosts = tables.boost_items.clone().unwrap_or_default();
        let model_stamina = model.as_deref().map(|model| Stamina {
            normal: model.stamina.normal,
            enhance: model.stamina.enhance,
            boost: model.stamina.boost,
        });
        let Some(stamina) = stamina_recover::stamina(user.as_deref(), model_stamina) else {
            error!("[stamina-recover] {caller}: the client holds no UserMysekaiStamina yet; the dialog cannot show");
            continue;
        };
        let id = match screens.show_dialog(DIALOG, DisplayLayerType::LayerDialog, DialogBackKey::Close, caller) {
            Ok(id) => id,
            Err(reason) => {
                error!("[stamina-recover] {caller}: {reason}");
                continue;
            }
        };
        screens.open_dialog(id);
        screens.dialog_open_finished(id);
        let live_bonus = 0;
        let crystals = 0;
        let convert_max = conversion_bound(live_bonus, stamina, max, row);
        let jewel_max = jewel_bound(crystals, stamina, max, row);
        let held: Vec<(i32, i32)> = boosts.iter().map(|item| (item.id, 1)).collect();
        let mut items = item_rows(&held, &boosts);
        if !items.is_empty() {
            items[0].selected = row_max(&items, 0, row, max, stamina.boost).max(1);
        }
        recover.rows.clear();
        recover.content_height = None;
        recover.shown = None;
        info!(
            "[stamina-recover] {caller} (menu {menu:?}): Show2ButtonDialog(MysekaiRecoverBoostStaminaDialog, 321) + Setup(onRecoverFinish): stamina {stamina:?} over {max:?}, live bonus {live_bonus} (not carried), crystals {crystals} (not carried); consumption 1..{convert_max}, crystal 1..{jewel_max}, {} item rows",
            items.len()
        );
        recover.live = Some(Live {
            id,
            top: Top::Consumption,
            method: Method::Item,
            live_bonus,
            crystals,
            convert_max,
            convert_value: 1,
            jewel_max,
            jewel_value: 1,
            items,
            stamina,
            max,
            row,
            boosts,
        });
    }
}

// ---------------------------------------------------------------------------
// Taps and the back key
// ---------------------------------------------------------------------------

fn to_canvas(position: Vec2, window: &Window, scale: f32) -> Vec2 {
    Vec2::new(position.x - window.width() / 2.0, window.height() / 2.0 - position.y) / scale
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

#[allow(clippy::too_many_arguments)]
fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut screens: ResMut<ScreenManager>,
    mut recover: ResMut<Recover>,
    mut link: ResMut<ServerLink>,
    mut finished: MessageWriter<crate::menu_dialog::RecoverFinished>,
    views: Query<&UiPrefabView, (With<RecoverView>, Without<ConfirmView>)>,
    confirms: Query<&UiPrefabView, (With<ConfirmView>, Without<RecoverView>)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    if recover.live.is_none() {
        return;
    }
    let Some(root) = root_canvas.as_deref() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = root.scale(window);
    let canvas = root.size(window);
    if let Some(confirm_id) = recover.confirm.as_ref().map(|confirm| confirm.id) {
        match confirm_answer(&mut back_keys, &mut gestures, &mut consumed, window, root, &layouts, &confirms, confirm_id) {
            Some(true) => confirm_request(&mut screens, &mut recover, &mut link, confirm_id),
            Some(false) => {
                info!("[stamina-recover] the confirmation cancelled: back to the recover dialog");
                recover.confirm = None;
                recover.request = None;
                close_instance(&mut screens, confirm_id);
                recover.shown = None;
            }
            None => {}
        }
        return;
    }
    let Some(live_id) = recover.live.as_ref().map(|live| live.id) else {
        return;
    };
    if back_keys.read().any(|event| event.id == live_id) {
        info!("[stamina-recover] back key -> Close");
        close_dialog(&mut screens, &mut recover, live_id, &mut finished);
        return;
    }
    let Some(taps) = taps(&mut gestures, &mut consumed) else {
        return;
    };
    let Ok(view) = views.single() else {
        return;
    };
    let Some(doc) = layouts.document(DIALOG_KEY) else {
        return;
    };
    let page = Page::from_document(doc);
    for position in taps {
        let point = to_canvas(position, &window, scale);
        if hit(&view, &layouts, &page.close, point, canvas)
            || hit(&view, &layouts, &page.negative, point, canvas)
            || !hit(&view, &layouts, &page.window, point, canvas)
        {
            info!("[stamina-recover] the close button, cancel or a tap outside (OnCloseExternal) -> Close");
            close_dialog(&mut screens, &mut recover, live_id, &mut finished);
            return;
        }
        let Some(live) = recover.live.as_mut() else {
            return;
        };
        if hit(&view, &layouts, &page.tabs.0, point, canvas) {
            if live.top != Top::Consumption {
                info!("[stamina-recover] OnLiveBoostUseOrRecoverTabChangeTab(Consumption)");
                live.top = Top::Consumption;
            }
        } else if hit(&view, &layouts, &page.tabs.1, point, canvas) {
            if live.top != Top::Recover {
                info!("[stamina-recover] OnLiveBoostUseOrRecoverTabChangeTab(Recover)");
                live.top = Top::Recover;
            }
        }
        if live.top == Top::Recover {
            if hit(&view, &layouts, &page.item_cell.1, point, canvas) {
                if live.method != Method::Item {
                    info!("[stamina-recover] OnLiveBoostUseContentTabChangeTab(Item)");
                }
                live.method = Method::Item;
            } else if hit(&view, &layouts, &page.crystal_cell.1, point, canvas) {
                if live.method != Method::Crystal {
                    info!("[stamina-recover] OnLiveBoostUseContentTabChangeTab(Crystal)");
                }
                live.method = Method::Crystal;
            }
        }
        match (live.top, live.method) {
            (Top::Consumption, _) => step_select(&view, &layouts, live, &page, Select::Convert, point, canvas),
            (Top::Recover, Method::Crystal) => step_select(&view, &layouts, live, &page, Select::Crystal, point, canvas),
            (Top::Recover, Method::Item) => {
                let count = recover.rows.len();
                for index in 0..count {
                    if !row_hit(&view, &layouts, &page, index, point, canvas) {
                        continue;
                    }
                    let Some(live) = recover.live.as_mut() else {
                        return;
                    };
                    if index >= live.items.len() {
                        continue;
                    }
                    let bound = row_max(&live.items, index, live.row, live.max, live.stamina.boost).max(1);
                    let selected = live.items[index].selected;
                    live.items[index].selected = if selected < bound { selected + 1 } else { 1 };
                    info!(
                        "[stamina-recover] OnChangeSelectedItem: boost item {} -> {} of {bound}",
                        live.items[index].item.id, live.items[index].selected
                    );
                }
            }
        }
        let Some(live) = recover.live.as_ref() else {
            return;
        };
        if !hit(&view, &layouts, &page.positive, point, canvas) {
            continue;
        }
        let enabled = match (live.top, live.method) {
            (Top::Consumption, _) => live.live_bonus >= 1,
            (Top::Recover, Method::Crystal) => live.crystals >= CRYSTAL_UNIT_PRICE,
            (Top::Recover, Method::Item) => live.items.iter().any(|row| row.selected > 0),
        };
        if !enabled {
            info!("[stamina-recover] SetEnabledPositiveButton: the positive button is disabled; nothing");
            continue;
        }
        let (top, method) = (live.top, live.method);
        open_confirmation(&mut screens, &mut recover, top, method);
        return;
    }
}

#[derive(Clone, Copy)]
enum Select {
    Convert,
    Crystal,
}

fn step_select(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    live: &mut Live,
    page: &Page,
    which: Select,
    point: Vec2,
    canvas: Vec2,
) {
    let (decrement, increment, slider, max, current) = match which {
        Select::Convert => (
            &page.convert_decrement,
            &page.convert_increment,
            &page.convert_slider,
            live.convert_max,
            live.convert_value,
        ),
        Select::Crystal => (
            &page.crystal_decrement,
            &page.crystal_increment,
            &page.crystal_slider,
            live.jewel_max,
            live.jewel_value,
        ),
    };
    let value = if hit(view, layouts, decrement, point, canvas) {
        current - 1
    } else if hit(view, layouts, increment, point, canvas) {
        current + 1
    } else if let Some(rect) = view.rect(layouts, slider, canvas).filter(|rect| rect.contains(point)) {
        let fraction = ((point.x - (rect.center().x - rect.size.x / 2.0)) / rect.size.x.max(1.0)).clamp(0.0, 1.0);
        (fraction * max as f32).round() as i32
    } else {
        return;
    };
    let value = value.clamp(1, max.max(1));
    match which {
        Select::Convert => live.convert_value = value,
        Select::Crystal => live.jewel_value = value,
    }
}

/// The item list's row `index`, as the view lays it out: the content's top
/// edge, one row every `ROW_HEIGHT + ROW_GAP`, and the row's `ROW_HEIGHT`.
fn row_hit(view: &UiPrefabView, layouts: &UiLayouts, page: &Page, index: usize, point: Vec2, canvas: Vec2) -> bool {
    let Some(rect) = view.rect(layouts, &page.item_content, canvas).filter(|rect| rect.active) else {
        return false;
    };
    let top = -rect.pivot.y * rect.size.y;
    let y = point.y - (top - index as f32 * (ROW_HEIGHT + ROW_GAP));
    y <= 0.0 && y >= -ROW_HEIGHT && (point.x - rect.center().x).abs() <= rect.size.x * 0.5
}

fn taps(gestures: &mut MessageReader<GestureEvent>, consumed: &mut ActionTapConsumed) -> Option<Vec<Vec2>> {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return None;
    }
    consumed.0 = true;
    Some(taps)
}

fn close_instance(screens: &mut ScreenManager, id: DialogId) {
    if screens.dialogs().any(|(open, ..)| open == id) {
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
    }
}

/// The recover dialog's `Close`: its close animation and destroy, then
/// `onRecoverFinish` and the field HUD's `UpdateStaminaView`.
fn close_dialog(
    screens: &mut ScreenManager,
    recover: &mut Recover,
    id: DialogId,
    finished: &mut MessageWriter<crate::menu_dialog::RecoverFinished>,
) {
    let Some(live) = recover.live.take() else {
        return;
    };
    debug_assert_eq!(live.id, id);
    close_instance(screens, id);
    finished.write(crate::menu_dialog::RecoverFinished);
    recover.shown = None;
    info!("[stamina-recover] Close: onRecoverFinish (UpdateStaminaGateView) queued");
}

#[allow(clippy::too_many_arguments)]
fn confirm_answer(
    back_keys: &mut MessageReader<DialogBackKeyEvent>,
    gestures: &mut MessageReader<GestureEvent>,
    consumed: &mut ActionTapConsumed,
    window: &Window,
    root: &crate::canvas::RootCanvas,
    layouts: &UiLayouts,
    views: &Query<&UiPrefabView, (With<ConfirmView>, Without<RecoverView>)>,
    id: DialogId,
) -> Option<bool> {
    if back_keys.read().any(|event| event.id == id) {
        info!("[stamina-recover] the confirmation: back key -> cancel");
        return Some(false);
    }
    let taps = taps(gestures, consumed)?;
    let Ok(view) = views.single() else {
        return None;
    };
    let doc = layouts.document(CONFIRM_KEY)?;
    let page = ConfirmPage::from_document(doc);
    let scale = root.scale(window);
    let canvas = root.size(window);
    for position in taps {
        let point = to_canvas(position, window, scale);
        if hit(view, layouts, &page.positive, point, canvas) {
            info!("[stamina-recover] the confirmation: OK (OnClickOK)");
            return Some(true);
        }
        if hit(view, layouts, &page.negative, point, canvas)
            || hit(view, layouts, &page.close, point, canvas)
            || !hit(view, layouts, &page.window, point, canvas)
        {
            return Some(false);
        }
    }
    None
}

/// `OnClickOk`: `Show2ButtonDialog(BoostStaminaRecoverConfirmDialog, 322)`
/// and `Initialize(StaminaData)` with the boost stamina the request restores.
fn open_confirmation(screens: &mut ScreenManager, recover: &mut Recover, top: Top, method: Method) {
    let Some(live) = recover.live.as_ref() else {
        return;
    };
    let mut consumes: Vec<(i32, i32)> = Vec::new();
    let (recovery_type, restored, changer) = match (top, method) {
        (Top::Consumption, _) => {
            consumes.push((LIVE_BONUS_RESOURCE, live.convert_value));
            (
                BoostStaminaRecoveryType::Boost,
                moly_masters::converted_boost(live.row, live.convert_value),
                (
                    live.live_bonus,
                    live.live_bonus
                        .saturating_sub(live.convert_value.saturating_mul(live.row.boost_quantity)),
                ),
            )
        }
        (Top::Recover, Method::Crystal) => {
            consumes.push((CRYSTAL_RESOURCE, live.jewel_value));
            (
                BoostStaminaRecoveryType::Jewel,
                moly_masters::converted_boost(live.row, live.jewel_value),
                (live.live_bonus, live.live_bonus),
            )
        }
        (Top::Recover, Method::Item) => {
            consumes.extend(
                live.items
                    .iter()
                    .filter(|row| row.selected > 0)
                    .map(|row| (row.item.id, row.selected)),
            );
            let sum = live
                .items
                .iter()
                .map(|row| row.value(live.row))
                .fold(0i32, i32::wrapping_add);
            (
                BoostStaminaRecoveryType::BoostItem,
                sum.min(live.max.boost.saturating_sub(live.stamina.boost)),
                (live.live_bonus, live.live_bonus),
            )
        }
    };
    let caller = "MysekaiRecoverBoostStaminaDialog.OnClickOk";
    let id = match screens.show_dialog(CONFIRM, DisplayLayerType::LayerDialog, DialogBackKey::Close, caller) {
        Ok(id) => id,
        Err(reason) => {
            error!("[stamina-recover] {caller}: {reason}");
            return;
        }
    };
    screens.open_dialog(id);
    screens.dialog_open_finished(id);
    info!(
        "[stamina-recover] {caller}: Show2ButtonDialog(BoostStaminaRecoverConfirmDialog, 322) Initialize(boost stamina +{restored}) for {} {consumes:?}",
        recovery_type.word()
    );
    recover.request = Some(request(recovery_type, consumes));
    recover.requested = restored;
    recover.confirm = Some(ConfirmOpen {
        id,
        recover: restored,
        changer,
    });
    recover.shown = None;
}

/// The confirmation's OK: the request on the link, the confirmation closed.
fn confirm_request(screens: &mut ScreenManager, recover: &mut Recover, link: &mut ServerLink, id: DialogId) {
    let Some(confirm) = recover.confirm.take() else {
        return;
    };
    if confirm.id != id {
        return;
    }
    close_instance(screens, id);
    let Some(request) = recover.request.take() else {
        return;
    };
    let call = Call::BoostStaminaRecovery(request);
    let kind = call.kind();
    let seq = link.call(call);
    recover.pending = Some(ServerPending {
        seq: Some(seq),
        call: kind,
    });
    recover.shown = None;
    info!("[stamina-recover] PostUserMysekaiBoostStaminaRecoveryApi sent as seq {seq}");
}

// ---------------------------------------------------------------------------
// The answer: the notice, the close, the cell and the HUD
// ---------------------------------------------------------------------------

fn read_answers(
    mut answers: MessageReader<Answered>,
    mut recover: ResMut<Recover>,
    mut commands: Commands,
    mut finished: MessageWriter<crate::menu_dialog::RecoverFinished>,
    mut screens: ResMut<ScreenManager>,
    mut refresh: Option<ResMut<HarvestStaminaRefresh>>,
) {
    for answer in answers.read() {
        let Some(pending) = recover.pending else {
            continue;
        };
        if pending.call != CallKind::BoostStaminaRecovery || pending.seq != Some(answer.seq) {
            continue;
        }
        recover.pending = None;
        let complete = match &answer.outcome {
            Ok(Reply::BoostStaminaRecovery(reply)) if reply.success => true,
            Ok(Reply::BoostStaminaRecovery(reply)) => {
                error!(
                    "[stamina-recover] PostUserMysekaiBoostStaminaRecoveryApi answered success false ({}); the recover dialog stays",
                    reply.refusal.clone().unwrap_or_default()
                );
                false
            }
            Ok(other) => {
                error!("[stamina-recover] PostUserMysekaiBoostStaminaRecoveryApi answered with {other:?}");
                false
            }
            Err(refusal) => {
                error!(
                    "[stamina-recover] PostUserMysekaiBoostStaminaRecoveryApi refused: {} ({}); the recover dialog stays",
                    refusal.reason, refusal.code
                );
                false
            }
        };
        if !complete {
            continue;
        }
        info!(
            "[stamina-recover] RecoverStamina.OnFinishPostMysekaiRecoverBoostStaminaAPI (seq {}): +{} boost stamina; ShowCompleteRecoverBoostDialog",
            answer.seq, recover.requested
        );
        if let Some(queue) = refresh.as_deref_mut() {
            queue.0.push(());
        }
        commands.queue(|world: &mut World| {
            let body = world
                .get_resource::<crate::ui_layout::UiLayouts>()
                .and_then(|layouts| layouts.wordings.get(RECOVER_COMPLETE).cloned())
                .unwrap_or_else(|| RECOVER_COMPLETE.to_owned());
            crate::common_dialog::open_detached(
                world,
                "RecoverStamina.ShowCompleteRecoverBoostDialog",
                body,
                crate::common_dialog::Buttons::One { ok: "WORD_OK" },
            );
        });
        if let Some(live) = recover.live.take() {
            close_instance(&mut screens, live.id);
            finished.write(crate::menu_dialog::RecoverFinished);
            recover.shown = None;
        }
        recover.request = None;
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn state_key(live: &Live, confirm: Option<&ConfirmOpen>) -> String {
    let items: Vec<String> = live
        .items
        .iter()
        .map(|row| format!("{}:{}", row.item.id, row.selected))
        .collect();
    format!(
        "{:?}/{:?}/{}/{}/{}/{}/{}|{:?}/{:?}",
        live.top,
        live.method,
        live.convert_value,
        live.convert_max,
        live.jewel_value,
        live.jewel_max,
        items.join(","),
        confirm.map(|confirm| confirm.recover),
        confirm.map(|confirm| confirm.changer),
    )
}

fn fraction(value: i32, max: i32) -> f32 {
    if max <= 1 {
        1.0
    } else {
        value as f32 / max as f32
    }
}

#[allow(clippy::too_many_arguments)]
fn process(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<UiLayouts>,
    mut recover: ResMut<Recover>,
    mut views: Query<(&mut UiPrefabView, &mut Visibility, &mut Transform), (With<RecoverView>, Without<ConfirmView>)>,
    mut confirms: Query<(&mut UiPrefabView, &mut Visibility, &mut Transform), (With<ConfirmView>, Without<RecoverView>)>,
) {
    let scale = match (windows.single().ok(), root_canvas.as_deref()) {
        (Some(window), Some(root)) => root.scale(window),
        _ => 1.0,
    };
    let Some(live) = recover.live.as_ref() else {
        if let Ok((_, mut visibility, mut transform)) = views.single_mut() {
            visibility.set_if_neq(Visibility::Hidden);
            crate::canvas::set_scale(&mut transform, scale);
        }
        if let Ok((_, mut visibility, mut transform)) = confirms.single_mut() {
            visibility.set_if_neq(Visibility::Hidden);
            crate::canvas::set_scale(&mut transform, scale);
        }
        for nodes in &recover.rows {
            commands.entity(nodes.entity).insert(Visibility::Hidden);
        }
        return;
    };
    let Some(doc) = layouts.document(DIALOG_KEY) else {
        return;
    };
    let key = state_key(live, recover.confirm.as_ref());
    let changed = recover.shown.as_deref() != Some(key.as_str());
    if let Ok((mut view, mut visibility, mut transform)) = views.single_mut() {
        visibility.set_if_neq(Visibility::Inherited);
        crate::canvas::set_scale(&mut transform, scale);
        if changed {
            let page = Page::from_document(doc);
            let canvas = windows.single().ok().zip(root_canvas.as_deref()).map(|(window, root)| root.size(window));
            let width = canvas
                .and_then(|canvas| view.rect(&layouts, &page.item_content, canvas))
                .map_or(0.0, |rect| rect.size.x);
            recover.content_width = Some(width);
            draw_page(&mut commands, &mut recover, &mut view, doc);
        }
    }
    if let Ok((mut view, mut visibility, mut transform)) = confirms.single_mut() {
        visibility.set_if_neq(match recover.confirm {
            Some(_) => Visibility::Inherited,
            None => Visibility::Hidden,
        });
        crate::canvas::set_scale(&mut transform, scale);
        if changed {
            if let (Some(doc), Some(confirm)) = (layouts.document(CONFIRM_KEY), recover.confirm.as_ref()) {
                draw_confirm(&mut view, doc, confirm, recover.live.as_ref());
            }
        }
    }
    recover.shown = Some(key);
}

fn draw_page(commands: &mut Commands, recover: &mut Recover, view: &mut UiPrefabView, doc: &UiPrefab) {
    let Some(live) = recover.live.as_ref() else {
        return;
    };
    let page = Page::from_document(doc);
    let consumption = live.top == Top::Consumption;
    let item_tab = live.top == Top::Recover && live.method == Method::Item;
    let crystal_tab = live.top == Top::Recover && live.method == Method::Crystal;
    // The top tabs switch the two bodies.
    view.set_visible(&page.convert_body, consumption);
    view.set_visible(&page.recovery_body, !consumption);
    // The consumption tab.
    let convert_after = live
        .live_bonus
        .saturating_sub(live.convert_value.saturating_mul(live.row.boost_quantity));
    view.set_slider(&page.convert_slider, fraction(live.convert_value, live.convert_max));
    view.set_text(&page.convert_text, live.convert_value.to_string());
    view.set_visible(&page.convert_shortage, live.live_bonus < live.convert_value);
    view.set_visible(&page.convert_balloon, live.stamina.boost >= live.max.boost || convert_after > live.live_bonus);
    // The recovery method tabs: the selected content shows, its banner covers
    // the tab that is not selected.
    view.set_visible(&page.item_cell.0, crystal_tab);
    view.set_visible(&page.crystal_cell.0, item_tab);
    // The item tab.
    let item_sum = live
        .items
        .iter()
        .map(|row| row.value(live.row))
        .fold(0i32, i32::wrapping_add)
        .min(live.max.boost.saturating_sub(live.stamina.boost));
    view.set_visible(&page.item_nothing, item_tab && live.items.is_empty());
    view.set_visible(&page.item_content, item_tab && !live.items.is_empty());
    view.set_text(&page.item_current, live.stamina.boost.to_string());
    view.set_text(&page.item_after, (live.stamina.boost + item_sum).to_string());
    view.set_visible(&page.item_warning, item_tab && live.stamina.boost + item_sum >= live.max.boost);
    view.set_visible(&page.item_balloon, item_tab && live.stamina.boost + item_sum >= live.max.boost);
    // The crystal tab.
    view.set_slider(&page.crystal_slider, fraction(live.jewel_value, live.jewel_max));
    view.set_text(&page.crystal_text, live.jewel_value.to_string());
    view.set_visible(&page.crystal_max, crystal_tab && live.stamina.boost >= live.max.boost);
    view.set_visible(&page.crystal_balloon, crystal_tab && live.stamina.boost >= live.max.boost);
    view.set_visible(&page.crystal_attention, crystal_tab && live.crystals < live.jewel_value);
    view.set_text(&page.crystal_confirm, crystal_sentence(doc, live));
    // The item rows.
    let rows_needed = if item_tab { live.items.len() } else { 0 };
    while recover.rows.len() > rows_needed {
        if let Some(row) = recover.rows.pop() {
            commands.entity(row.entity).despawn();
        }
    }
    while recover.rows.len() < rows_needed {
        let entity = commands
            .spawn((
                Text::new(String::new()),
                TextFont::from_font_size(26.0),
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Left),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(24.0),
                    top: Val::Px(recover.rows.len() as f32 * (ROW_HEIGHT + ROW_GAP)),
                    ..default()
                },
                Transform::from_xyz(0., 0., 0.5),
                Visibility::Inherited,
            ))
            .id();
        recover.rows.push(RowNodes { entity });
    }
    if item_tab {
        let height = (live.items.len() as f32 * (ROW_HEIGHT + ROW_GAP) - ROW_GAP).max(1.0);
        if recover.content_height != Some(height) {
            let width = recover.content_width.unwrap_or(0.0);
            view.set_size_delta(&page.item_content, Vec2::new(width, height));
            recover.content_height = Some(height);
        }
        for (index, nodes) in recover.rows.iter().enumerate() {
            let row = &live.items[index];
            commands.entity(nodes.entity).insert((
                Visibility::Inherited,
                Text::new(format!(
                    "{}  x{}  →  +{}{}",
                    row.item.id,
                    row.selected,
                    row.value(live.row),
                    if row.selected >= row_max(&live.items, index, live.row, live.max, live.stamina.boost) {
                        "  (next tap: 1)"
                    } else {
                        ""
                    }
                )),
            ));
        }
    }
}

/// The crystal tab's confirmation sentence: the serialized text with the
/// count for the crystals it spends and the boost stamina it restores.
fn crystal_sentence(doc: &UiPrefab, live: &Live) -> String {
    let restored = moly_masters::converted_boost(live.row, live.jewel_value);
    let text = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|component| component.fields.get("wordingKey").and_then(Value::as_str) == Some("W_C_87"))
        .and_then(|component| component.fields.get("m_text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "W_C_87".to_owned());
    text.replace("990", &live.jewel_value.to_string())
        .replace("99", &restored.to_string())
}

fn draw_confirm(view: &mut UiPrefabView, doc: &UiPrefab, confirm: &ConfirmOpen, live: Option<&Live>) {
    let Some(live) = live else {
        return;
    };
    let page = ConfirmPage::from_document(doc);
    let body = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|component| {
            component.fields.get("wordingKey").and_then(Value::as_str) == Some(CONFIRM_BODY_WORDING)
        })
        .and_then(|component| component.fields.get("m_text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| CONFIRM_BODY_WORDING.to_owned());
    view.set_text(&page.message, body);
    view.set_text(&format!("@{}", page.changer.0), confirm.changer.0.to_string());
    view.set_text(&format!("@{}", page.changer.1), confirm.changer.1.to_string());
    // `UpdateStaminaView(staminaData, stock, false)` on the before views and on
    // the after views (whose boost is the restored one): the gauge colour, the
    // gradient, the icon's sprite and colour, the stock backs and the count.
    for (root, boost) in [(&page.before_normal, live.stamina.boost), (&page.after_normal, live.stamina.boost + confirm.recover)] {
        let state = stamina_view_state(
            Stamina {
                normal: live.stamina.normal,
                enhance: live.stamina.enhance,
                boost,
            },
            live.max,
        );
        write_stamina_view(view, doc, root, &state);
    }
    view.set_visible(&page.before_warning, live.stamina.boost >= live.max.boost);
    view.set_visible(&page.after_warning, live.stamina.boost + confirm.recover >= live.max.boost);
}

/// The `MysekaiStaminaView` under `root` (its view component's references):
/// `UpdateStaminaView`'s writes without the palette entries (the serialized
/// colours stay; the module notes name that).
fn write_stamina_view(view: &mut UiPrefabView, doc: &UiPrefab, root: &str, state: &crate::menu_dialog::StaminaViewState) {
    let index = doc.find(root).expect("a stamina view node");
    let node = &doc.nodes[index];
    let Some(component) = node.components.iter().find(|component| component.class == "Sekai.Mysekai.MysekaiStaminaView") else {
        return;
    };
    let path = |name: &str| {
        let id = reference(doc, &component.fields, name);
        path_of(doc, id)
    };
    view.set_visible(&path("_staminaGradiantImage"), state.gradient);
    view.set_visible(&path("_staminaGageImageOnStock"), state.on_stock);
    view.set_visible(&path("_staminaGageImageOnNoStock"), !state.on_stock);
    view.set_visible(&path("_boostStaminaCount"), state.boost_count.is_some());
    if let Some(count) = state.boost_count {
        let wanted = format!("txt_stamina_{count}");
        let id = reference(doc, &component.fields, "_boostStaminaCount");
        let serialized = doc
            .nodes
            .iter()
            .flat_map(|node| node.components.iter())
            .find(|component| component.path_id == id)
            .and_then(|component| component.sprite.as_ref())
            .and_then(|sprite| sprite["name"].as_str());
        if serialized != Some(wanted.as_str()) {
            error_once!(
                "[stamina-recover] {}: the stock count sprite {wanted} is not this root's; the serialized {serialized:?} stays",
                doc.prefab
            );
        }
    }
    let gauge = path("_staminaGageImage");
    view.set_fill(&gauge, state.fill);
}
