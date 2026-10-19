//! The convert screen's own dialogs: the start confirm (324), the skip (325)
//! and the item detail (284).
//!
//! Source structure:
//! - `MysekaiConvertStartConfirmDialog` (`Common2ButtonDialog`): `Setup(new
//!   MysekaiConvertStartConfirmViewModel(model))` writes the converted item's
//!   thumbnail and `ObtainedItemName`, the view model's `ConvertTimeString`,
//!   and the joined-pass balloon (`Enable = HasMysekaiColorfulPass()`,
//!   `MessageText = string.Format(FORMAT_CONVERT_SHORTEN_TIME_HAVE_JOINED_PASS,
//!   GetConvertTimeDecreaseRate() * 100)`); every `Update` sets
//!   `_passExpirationWarningText` active by `IsShowsPassExpirationWarning()`.
//!   The class writes no body: `Content/ConfirmText` is the body for the
//!   ruling's `MSG_CONVERT_START_CONFIRM`. `positiveButton` (`OnClickOK`)
//!   runs `ConvertStartProcess`; the second footer button and the window's
//!   close button cancel, and the hardware back key cancels.
//! - `MysekaiConvertFurnitureShortenDialog` (`Common2ButtonDialog`): every
//!   frame `Refresh` -> `_slotModel.UpdateSlot()` -> `RefreshView` formats
//!   `MSG_CONVERT_SLOT_SHORTEN_CONFIRM` with the skip's price and the item's
//!   name, the remaining time, the price and the crystals held, tints the
//!   price (`FONT_COLOR_PINK2` over the total, `FONT_COLOR_BLACK` otherwise),
//!   turns `NotEnoughJewelText` and `_toJewelShopButton` on only when the
//!   total cannot pay, and `positiveButton` (the skip) only when it can. Its
//!   OK is `OnClickOkShortenSlot`; the second footer button is the cancel.
//!   `_specifiedCommercialTransactionLawArticleButton` opens
//!   `CommonWebviewDialog`; `_toJewelShopButton` closes and pushes
//!   `MenuScreenType.CrystalShop`.
//! - `MysekaiConvertItemDetailDialog` (`Common1ButtonDialog`): `Setup(model)`
//!   sets the thumbnail, `Name = ObtainedItemName` and
//!   `Description/Text = ObtainedItemDescription`. Its OK, the window's close
//!   button and the back key close it (its callback is null).
//!
//! Host mapping: each dialog installs its document once as a runtime copy
//! (`*_VIEW`), is drawn while its slot is up, takes its taps in [`click`]
//! (before the shell's) and its back key in [`back_key`] (after the manager's
//! reader). A dialog opens from the request the screen's own tap wrote
//! ([`super::start::StartIntent`] / [`super::start::ShortenIntent`], the long
//! press's [`ConvertScreen::detail_request`]), only while the layer is
//! playing and no other dialog is up. Its `Setup`
//! runs on every frame it shows (the source's `RefreshView`, and the skip's
//! own `Update`). The start chain's send is ordered after [`open_start`], so
//! the OK's request is sent in the frame it is read.
//!
//! Named refusals:
//! - The account's crystal wallet (`CurrentTotalJewel`, `CurrentPaidJewel`) is
//!   not in the user document: the skip hides `HavetRoot/Cost` and
//!   `HavetRoot/UIPartsPaidLabel (1)/CustomText (1)` instead of drawing an
//!   owned amount, no price is compared against it (so the skip button stays
//!   the document's, `NotEnoughJewelText` hidden), and the price keeps the
//!   document's colour. The price line (`CostRoot/Cost`,
//!   `MSG_CONVERT_SLOT_SHORTEN_CONFIRM`) stays.
//! - The skip's confirm line is hidden when the item's name cannot be read
//!   (the line formats it as `{1}`).
//! - `CommonWebviewDialog` has no prefab: the law button's press is named and
//!   nothing opens. `MenuScreenType.CrystalShop` is not carried: the shop
//!   button closes the skip and is named.
//! - A thumbnail the item icon index lacks leaves the thumbnail off (named).
//!   The resource box thumbnail is one such cell, not in this root.
//! - A `mysekai_music_record_category_choice` record's description names its
//!   category, and the convert master reader keeps the category's id and
//!   track type only: that description stays empty (named once).
//! - The `DialogBase` scale tween is not drawn; the entity's canvas scale
//!   stands.

use std::collections::BTreeSet;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiPrefab;
use moly_masters::convert::ConvertMasters;

use super::detail::Masters;
use super::model::{ItemContext, ItemModel, PassState, RecordKind, SlotModel};
use super::screen::{note_once, now_ms, read_user, ConvertScreen};
use crate::action_button::ActionTapConsumed;
use crate::audio::SeRequests;
use crate::client_config::ClientConfigs;
use crate::canvas::RootCanvas;
use crate::convert::model::ConvertUserState;
use crate::gesture::{GestureEvent, GestureState};
use crate::item_icon::ItemIcons;
use crate::ui_inputs::{BalloonArt, SITEMAP_LAYER};
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::client::inventory::ClientMysekaiInventory;
use crate::userdata::convert::ClientConvertSlots;
use crate::userdata::ClientUserData;

// ---------------------------------------------------------------------------
// Documents, keys and values
// ---------------------------------------------------------------------------

/// The start confirm's document (`dialogs/MysekaiConvertStartConfirmDialog.json`).
pub const START_KEY: &str = "ConvertStartConfirm";
/// Its runtime copy.
pub const START_VIEW: &str = "ConvertStartConfirmView";
/// `DialogType.MysekaiConvertStartConfirmDialog`.
pub const START_DIALOG: DialogType = DialogType(324);
/// The skip's document (`dialogs/MysekaiConvertFurnitureShortenDialog.json`).
pub const SHORTEN_KEY: &str = "ConvertShorten";
/// Its runtime copy.
pub const SHORTEN_VIEW: &str = "ConvertShortenView";
/// `DialogType.MysekaiConvertFurnitureShortenDialog`.
pub const SHORTEN_DIALOG: DialogType = DialogType(325);
/// The item detail's document (`dialogs/MysekaiConvertItemDetailDialog.json`).
pub const DETAIL_KEY: &str = "ConvertItemDetail";
/// Its runtime copy.
pub const DETAIL_VIEW: &str = "ConvertItemDetailView";
/// `DialogType.MysekaiConvertItemDetailDialog`.
pub const DETAIL_DIALOG: DialogType = DialogType(284);
/// `Presenter.ConfirmConvertStart`'s slot caller.
pub const START_CALLER: &str = "MysekaiConvertFurniturePresenter.ConfirmConvertStart";
/// `Presenter.OnShortenSlot`'s slot caller.
pub const SHORTEN_CALLER: &str = "MysekaiConvertFurniturePresenter.OnShortenSlot";
/// `MysekaiConvertFurnitureItemSelectListCell.ShowDetailDialog`'s caller.
pub const DETAIL_CALLER: &str = "MysekaiConvertFurnitureItemSelectListCell.ShowDetailDialog";
/// The start confirm's body key (the ruling's; the document keeps no key).
pub const START_CONFIRM: &str = "MSG_CONVERT_START_CONFIRM";
/// The pass warning's line.
pub const PASS_WARNING: &str = "MSG_CONVERT_START_ATTENTION_EXPIRED_PASS";
/// `ClientConfig.Mysekai.ConvertStartConfirmRemainingDaysUntilPassExpirationWarning`:
/// `ClientConfigs`'s `IntConfigs` key 160
/// (`ClientConfig.Mysekai.get_ConvertStartConfirmRemainingDaysUntilPassExpirationWarning`
/// does `IntConfigs.TryGetValue(160)`, logging and returning 0 when it is
/// missing; that dictionary is built from the master `clientConfigs` rows).
pub const PASS_WARNING_DAYS_KEY: i32 = 160;
/// `FORMAT_CONVERT_SHORTEN_TIME_HAVE_JOINED_PASS` (`{0}` is the rate).
pub const PASS_SHORTEN: &str = "FORMAT_CONVERT_SHORTEN_TIME_HAVE_JOINED_PASS";
/// `MSG_CONVERT_SLOT_SHORTEN_CONFIRM` (`{0}` the price, `{1}` the name).
pub const SHORTEN_CONFIRM: &str = "MSG_CONVERT_SLOT_SHORTEN_CONFIRM";
/// `WORD_CONVERT_SLOT_SHORTEN_DECIDE`: the skip button's label.
pub const SHORTEN_DECIDE: &str = "WORD_CONVERT_SLOT_SHORTEN_DECIDE";
/// `MSG_CONVERT_RECORD_RANDOM_BOX_CHOICE_DESCRIPTION` (`{0}` the category).
pub const RECORD_CHOICE_DESCRIPTION: &str = "MSG_CONVERT_RECORD_RANDOM_BOX_CHOICE_DESCRIPTION";
/// `ColorUtility.FONT_COLOR_PINK2`'s palette entry (`ColorEntry.text_rd`,
/// value 17, read by `PaletteUtility.GetColorCode(ColorEntry.text_rd)`; the
/// `ui-jp` palette numbers its entries from 0 in `ColorEntry` order).
pub const PINK2_ENTRY: usize = 17;
/// `ColorUtility.FONT_COLOR_BLACK`'s palette entry (`ColorEntry.text_dbl`, 14).
pub const BLACK_ENTRY: usize = 14;
const DAY_MS: i64 = 86_400_000;
const DIALOG_Z: f32 = 100.0;

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

/// The nodes the three dialogs write, by the documents' paths. A dialog reads
/// its own with [`read`].
#[derive(Debug, Clone, Default, Component)]
pub struct Nodes {
    pub close_button: String,
    /// The footers' positive, second and third buttons (and labels).
    pub positive: String,
    pub second: String,
    pub third: String,
    pub positive_label: String,
    pub second_label: String,
    pub third_label: String,
    // The start confirm.
    pub confirm_text: String,
    pub item_text: String,
    pub convert_time: String,
    pub pass_balloon: String,
    pub pass_text: String,
    pub warning_text: String,
    // The skip.
    pub shorten_confirm: String,
    pub shorten_current: String,
    pub shorten_cost: String,
    pub shorten_have: String,
    pub shorten_have_paid: String,
    pub shorten_not_enough: String,
    pub shorten_shop: String,
    pub shorten_law: String,
    // The item detail.
    pub detail_thumbnail: String,
    pub detail_icon: String,
    pub detail_name: String,
    pub detail_description: String,
}

/// Which dialog a document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Start,
    Shorten,
    Detail,
}

fn sub(root: &str, below: &str) -> String {
    format!("{root}/{below}")
}

fn footer(window: &str, which: &str, label: bool) -> String {
    let text = if label { "/Text" } else { "" };
    format!("{window}/FooterButtons/UIPartsCommonButton{which}{text}")
}

/// A dialog's nodes, resolved from its document; a missing node refuses the
/// dialog by name.
pub fn read(doc: &UiPrefab, which: Which) -> Result<Nodes, String> {
    let root = doc
        .nodes
        .first()
        .map(|node| node.path.clone())
        .ok_or("the document has no root")?;
    let window = sub(&root, "WindowRoot");
    let content = sub(&window, "ContentRoot/Content");
    let mut nodes = Nodes {
        close_button: sub(&window, "UIPartsCloseButton"),
        positive: footer(&window, "", false),
        second: footer(&window, " (1)", false),
        third: footer(&window, " (2)", false),
        positive_label: footer(&window, "", true),
        second_label: footer(&window, " (1)", true),
        third_label: footer(&window, " (2)", true),
        ..Nodes::default()
    };
    let mut required = vec![nodes.close_button.clone(), nodes.positive.clone()];
    match which {
        Which::Start => {
            let balloon = sub(&content, "ConvertTime/ShortenByJoinedPassBalloon (1)");
            nodes.confirm_text = sub(&content, "ConfirmText");
            nodes.item_text = sub(&content, "ConvertItemText");
            nodes.convert_time = sub(&content, "ConvertTime/Value");
            nodes.pass_text = sub(&balloon, "Content/Text");
            nodes.pass_balloon = balloon;
            nodes.warning_text = sub(&content, "AttentionText");
            required.extend([
                nodes.confirm_text.clone(),
                nodes.item_text.clone(),
                nodes.convert_time.clone(),
                nodes.pass_text.clone(),
                nodes.warning_text.clone(),
                nodes.second.clone(),
            ]);
        }
        Which::Shorten => {
            let cost = sub(&content, "UIPartsPaidCost");
            nodes.shorten_confirm = sub(&content, "ConfirmText");
            nodes.shorten_current = sub(&content, "ShortenTime/Current");
            nodes.shorten_cost = sub(&cost, "CostRoot/Cost");
            nodes.shorten_have = sub(&cost, "HavetRoot/Cost");
            nodes.shorten_have_paid = sub(&cost, "HavetRoot/UIPartsPaidLabel (1)/CustomText (1)");
            nodes.shorten_not_enough = sub(&content, "NotEnoughJewelText");
            nodes.shorten_shop = nodes.third.clone();
            nodes.shorten_law = sub(&content, "SpecifiedCommercialTransactionLawArticleButton");
            required.extend([
                nodes.shorten_confirm.clone(),
                nodes.shorten_current.clone(),
                nodes.shorten_cost.clone(),
                nodes.shorten_have.clone(),
                nodes.shorten_not_enough.clone(),
                nodes.third.clone(),
                nodes.second.clone(),
            ]);
        }
        Which::Detail => {
            let thumbnail = sub(&content, "UIPartsItemThumbnail");
            nodes.detail_icon = sub(&thumbnail, "mask/Icon");
            nodes.detail_thumbnail = thumbnail;
            nodes.detail_name = sub(&content, "Name");
            nodes.detail_description = sub(&content, "Description/Text");
            required.extend([
                nodes.detail_name.clone(),
                nodes.detail_description.clone(),
            ]);
        }
    }
    for path in &required {
        doc.find(path)
            .map_err(|error| format!("{path}: {error}"))?;
    }
    Ok(nodes)
}

/// One dialog's entity.
#[derive(Component)]
pub struct DialogOf(Which);

/// The start confirm's entity.
#[derive(Component)]
pub struct StartRoot;
/// The skip's entity.
#[derive(Component)]
pub struct ShortenRoot;
/// The item detail's entity.
#[derive(Component)]
pub struct DetailRoot;

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// `IsShowsPassExpirationWarning`: the pass runs and not more than `days`
/// whole days are left (`GetRemainingTimeValidMysekaiColorfulPass().TotalDays
/// <= ClientConfig.ConvertStartConfirmRemainingDaysUntilPassExpirationWarning`).
pub fn shows_pass_warning(pass: &PassState, now: i64, days: i32) -> bool {
    if !pass.is_valid(now) {
        return false;
    }
    pass.remaining_ms(now) as f64 / DAY_MS as f64 <= f64::from(days)
}

/// A font colour's palette entry as the layout carries it, or `None` when
/// the root has no palette (`FONT_COLOR_PINK2` is `ColorEntry.text_rd`, 17;
/// `FONT_COLOR_BLACK` is `ColorEntry.text_dbl`, 14). The skip's price picks
/// between them by the owned total, which this product does not hold, so
/// only the entries are read here.
pub fn jewel_color(layouts: &UiLayouts, entry: usize) -> Option<[f32; 4]> {
    layouts.palette_color(entry)
}

/// `MysekaiConvertItemModelBase.GetObtainedItemDescription`, or `None` where
/// the master row it needs is not read here.
pub fn item_description(
    layouts: &UiLayouts,
    ctx: ItemContext<'_>,
    model: &ItemModel<'_>,
    masters: &Masters<'_>,
    seen: &mut BTreeSet<String>,
) -> Option<String> {
    match model {
        ItemModel::Normal { .. } => match super::detail::single_obtain(ctx, model) {
            Some(detail) => masters.name(&detail.resource_type, detail.resource_id),
            // `ObtainResourceBox.description`.
            None => super::item_select::obtain_box(ctx, model).and_then(|box_| box_.description.clone()),
        },
        ItemModel::Blueprint { obtain, .. } => obtain.description_key().map(|key| layouts.wording(key)),
        ItemModel::Record { obtain, choice, .. } => match obtain {
            RecordKind::Random => Some(layouts.wording(obtain.description_key())),
            RecordKind::CategoryChoice => {
                // `MasterMysekaiMusicRecordCategory.name`: the convert master
                // reader keeps the category's id and track type only.
                note_once(
                    seen,
                    format!("record category {choice}'s name is not read by the convert master reader: its description stays empty"),
                );
                None
            }
        },
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The start confirm's view model, as `ConfirmConvertStart` holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartData {
    pub slot_id: i32,
    pub item_id: i32,
    pub convert_count: i32,
}

/// The skip's slot as its model reads it each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortenData {
    pub slot_id: i32,
    pub pass_slot: bool,
}

/// The three dialogs.
#[derive(Resource, Default)]
pub struct ConvertDialogs {
    start: Option<DialogId>,
    shorten: Option<DialogId>,
    detail: Option<DialogId>,
    pub(crate) start_data: Option<StartData>,
    pub(crate) shorten_data: Option<ShortenData>,
    pub(crate) detail_item: Option<i32>,
    seen: BTreeSet<String>,
}

impl ConvertDialogs {
    pub fn start(&self) -> Option<DialogId> {
        self.start
    }

    pub fn shorten(&self) -> Option<DialogId> {
        self.shorten
    }

    pub fn detail(&self) -> Option<DialogId> {
        self.detail
    }

    /// Any of the three is up.
    pub fn any(&self) -> bool {
        self.start.is_some() || self.shorten.is_some() || self.detail.is_some()
    }
}

// ---------------------------------------------------------------------------
// Systems: the documents
// ---------------------------------------------------------------------------

/// Update: each dialog's runtime copy once the root carries its document.
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    starts: Query<(), With<StartRoot>>,
    shortens: Query<(), With<ShortenRoot>>,
    details: Query<(), With<DetailRoot>>,
) {
    let jobs: [(&str, &str, Which, bool); 3] = [
        (START_KEY, START_VIEW, Which::Start, !starts.is_empty()),
        (SHORTEN_KEY, SHORTEN_VIEW, Which::Shorten, !shortens.is_empty()),
        (DETAIL_KEY, DETAIL_VIEW, Which::Detail, !details.is_empty()),
    ];
    for (key, view_key, which, exists) in jobs {
        if exists {
            continue;
        }
        let Some(doc) = layouts.document(key) else {
            return;
        };
        let nodes = match read(doc, which) {
            Ok(nodes) => nodes,
            Err(reason) => {
                error!("[convert] {key} is refused: {reason}");
                continue;
            }
        };
        let document = doc.clone();
        if let Err(reason) = layouts.replace_runtime_document(view_key, document, &server) {
            error!("[convert] {key} is not installed: {reason}");
            continue;
        }
        let entity = match which {
            Which::Start => commands.spawn(StartRoot).id(),
            Which::Shorten => commands.spawn(ShortenRoot).id(),
            Which::Detail => commands.spawn(DetailRoot).id(),
        };
        commands.entity(entity).insert((
            DialogOf(which),
            Visibility::Hidden,
            Transform::from_xyz(0., 0., DIALOG_Z),
            RenderLayers::layer(SITEMAP_LAYER),
            UiPrefabView::new(view_key, SITEMAP_LAYER),
            nodes,
        ));
        info!("[convert] {key}: its document is installed as the runtime copy {view_key}");
    }
}

// ---------------------------------------------------------------------------
// Systems: closing
// ---------------------------------------------------------------------------

fn close(by: &str, dialogs: &mut ConvertDialogs, screens: &mut ScreenManager, which: Which) {
    let id = match which {
        Which::Start => dialogs.start.take(),
        Which::Shorten => dialogs.shorten.take(),
        Which::Detail => dialogs.detail.take(),
    };
    let Some(id) = id else {
        return;
    };
    match which {
        Which::Start => dialogs.start_data = None,
        Which::Shorten => dialogs.shorten_data = None,
        Which::Detail => dialogs.detail_item = None,
    }
    info!("[convert] {which:?} dialog {id:?}: {by}; DialogBase.Close, the dialog is destroyed");
    screens.close_dialog(id);
    screens.dialog_destroyed(id);
}

// ---------------------------------------------------------------------------
// Systems: opening
// ---------------------------------------------------------------------------

/// `Show2ButtonDialog` / `Show1ButtonDialog`: the slot, `Open`, then
/// `OnFinishOpenAnimation`.
fn show(screens: &mut ScreenManager, which: DialogType, caller: &str) -> Result<DialogId, String> {
    let id = screens.show_dialog(which, DisplayLayerType::LayerDialog, DialogBackKey::Close, caller)?;
    screens.open_dialog(id);
    screens.dialog_open_finished(id);
    Ok(id)
}

/// The start confirm, when the screen holds `ConfirmConvertStart`'s request
/// and the layer plays.
pub fn open_start(
    mut screens: ResMut<ScreenManager>,
    mut dialogs: ResMut<ConvertDialogs>,
    mut screen: ResMut<ConvertScreen>,
    roots: Query<(), With<StartRoot>>,
    layouts: Res<UiLayouts>,
) {
    let Some(intent) = screen.start_intent else {
        return;
    };
    if !screen.is_set_up()
        || screens.current_screen() != Some(super::screen::SCREEN)
        || screens.current_state() != crate::ui_layers::ScreenLayerState::Playing
    {
        return;
    }
    if dialogs.any() {
        return;
    }
    if roots.is_empty() || layouts.document(START_VIEW).is_none() {
        error!("[convert] {START_KEY} is not installed: the start confirm does not open");
        return;
    }
    match show(&mut screens, START_DIALOG, START_CALLER) {
        Ok(id) => {
            dialogs.start = Some(id);
            dialogs.start_data = Some(StartData {
                slot_id: intent.slot_id,
                item_id: intent.item.map_or(0, |item| item.item_id),
                convert_count: intent.item.map_or(0, |item| item.convert_count),
            });
            info!("[convert] MysekaiConvertStartConfirmDialog: Show2ButtonDialog(324) is open for slot {}", intent.slot_id);
        }
        Err(reason) => error!("[convert] {reason}"),
    }
}

/// The skip dialog, when the screen holds `OnShortenSlot`'s request.
pub fn open_shorten(
    mut screens: ResMut<ScreenManager>,
    mut dialogs: ResMut<ConvertDialogs>,
    mut screen: ResMut<ConvertScreen>,
    roots: Query<(), With<ShortenRoot>>,
    layouts: Res<UiLayouts>,
) {
    let Some(intent) = screen.shorten_intent else {
        return;
    };
    if !screen.is_set_up()
        || screens.current_screen() != Some(super::screen::SCREEN)
        || screens.current_state() != crate::ui_layers::ScreenLayerState::Playing
    {
        return;
    }
    if dialogs.any() {
        return;
    }
    if roots.is_empty() || layouts.document(SHORTEN_VIEW).is_none() {
        error!("[convert] {SHORTEN_KEY} is not installed: the skip dialog does not open");
        return;
    }
    match show(&mut screens, SHORTEN_DIALOG, SHORTEN_CALLER) {
        Ok(id) => {
            dialogs.shorten = Some(id);
            dialogs.shorten_data = Some(ShortenData {
                slot_id: intent.slot_id,
                pass_slot: intent.pass_slot,
            });
            info!("[convert] MysekaiConvertFurnitureShortenDialog: Show2ButtonDialog(325) is open for slot {}", intent.slot_id);
        }
        Err(reason) => error!("[convert] {reason}"),
    }
}

/// The item detail, when the item select page's long press asked for it.
pub fn open_detail(
    mut screens: ResMut<ScreenManager>,
    mut dialogs: ResMut<ConvertDialogs>,
    mut screen: ResMut<ConvertScreen>,
    roots: Query<(), With<DetailRoot>>,
    layouts: Res<UiLayouts>,
) {
    let Some(item) = screen.detail_request.take() else {
        return;
    };
    if !screen.is_set_up()
        || screens.current_screen() != Some(super::screen::SCREEN)
        || screens.current_state() != crate::ui_layers::ScreenLayerState::Playing
    {
        return;
    }
    if dialogs.any() {
        return;
    }
    if roots.is_empty() || layouts.document(DETAIL_VIEW).is_none() {
        error!("[convert] {DETAIL_KEY} is not installed: the item detail does not open");
        return;
    }
    match show(&mut screens, DETAIL_DIALOG, DETAIL_CALLER) {
        Ok(id) => {
            dialogs.detail = Some(id);
            dialogs.detail_item = Some(item);
            info!("[convert] MysekaiConvertItemDetailDialog: Show1ButtonDialog(284) is open for item {item}");
        }
        Err(reason) => error!("[convert] {reason}"),
    }
}



// ---------------------------------------------------------------------------
// Systems: setup
// ---------------------------------------------------------------------------

/// The tables and copies the dialogs read.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DialogData<'w> {
    pub tables: Res<'w, moly_masters::load::MasterTables>,
    pub client: Option<Res<'w, ClientUserData>>,
    pub copies: Option<Res<'w, ClientConvertSlots>>,
    pub inventory: Option<Res<'w, ClientMysekaiInventory>>,
    pub icons: Option<Res<'w, ItemIcons>>,
    pub real: Res<'w, Time<Real>>,
    pub art: Option<Res<'w, BalloonArt>>,
}

impl DialogData<'_> {
    fn user(&self) -> Option<ConvertUserState> {
        read_user(self.client.as_deref(), self.copies.as_deref(), self.inventory.as_deref())
    }

    fn now(&self) -> i64 {
        now_ms(self.client.as_deref(), self.real.elapsed_secs())
    }

    fn masters(&self) -> Option<&ConvertMasters> {
        self.tables.convert.as_ref()
    }
}

/// The item's thumbnail: the item select page's icon rule, registered as a
/// runtime texture when the root's index has it.
fn thumbnail(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: Option<&ItemIcons>,
    ctx: ItemContext<'_>,
    model: &ItemModel<'_>,
    masters: &Masters<'_>,
    seen: &mut BTreeSet<String>,
) -> Option<String> {
    let (path, _) = super::item_select::icon_load(ctx, model, masters)?;
    if !layouts.has_runtime_texture(&path) {
        let Some(icons) = icons else {
            note_once(seen, "the item icon index is not on this root: the dialog draws no thumbnail".to_owned());
            return None;
        };
        // The icon index keys a package by its load path and the resource
        // file's name after the last `/`; a path without one is refused.
        let Some(resource) = path.rsplit('/').next().filter(|name| !name.is_empty()) else {
            note_once(
                seen,
                format!("the dialog's thumbnail load path {path:?} names no resource file: the thumbnail is off"),
            );
            return None;
        };
        match icons.texture(&path, resource) {
            Ok(image) => {
                layouts.register_runtime_texture(&path, &image, server);
            }
            Err(reason) => {
                note_once(
                    seen,
                    format!("the dialog's thumbnail {path} is not in the item icon index ({reason}): the thumbnail is off"),
                );
                return None;
            }
        }
    }
    Some(path)
}

/// Update: `RefreshView` of the start confirm, each frame it shows (the pass
/// warning is its `Update`).
#[allow(clippy::too_many_arguments)]
pub fn setup_start(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut dialogs: ResMut<ConvertDialogs>,
    data: DialogData,
    configs: Option<Res<ClientConfigs>>,
    mut views: Query<(&Nodes, &mut UiPrefabView), With<StartRoot>>,
) {
    let (Some(slot_id), Some(item_id)) = (
        dialogs.start_data.map(|data| data.slot_id),
        dialogs.start_data.map(|data| data.item_id),
    ) else {
        return;
    };
    let count = dialogs.start_data.map_or(1, |data| data.convert_count);
    if dialogs.start.is_none() {
        return;
    }
    let (Some(user), Some(masters)) = (data.user(), data.masters()) else {
        return;
    };
    let Ok((nodes, mut view)) = views.single_mut() else {
        return;
    };
    let ctx = ItemContext { masters, user: &user, now: data.now() };
    let rows = Masters::new(&data.tables.rows);
    let mut seen = std::mem::take(&mut dialogs.seen);
    let art = data.art.as_deref();
    let model = ItemModel::all(ctx)
        .into_models()
        .into_iter()
        .find(|model| model.id() == item_id);
    if let Some(model) = &model {
        let name = super::item_select::item_name(&layouts, ctx, model, &rows);
        if let Some(name) = name {
            let shows = put_text(&mut view, &nodes.item_text, &name, art, &mut seen);
            view.set_visible(&nodes.item_text, shows);
        }
        let convert_ms = i64::from(model.convert_minutes(ctx, count)) * 60_000;
        let line = super::slot::to_time_string(convert_ms);
        put_text(&mut view, &nodes.convert_time, &line, art, &mut seen);
    } else {
        note_once(&mut seen, format!("the start confirm's item {item_id} is not read: its name and time stay"));
    }
    let _ = slot_id;
    let body = layouts.wording(START_CONFIRM);
    let shows = put_text(&mut view, &nodes.confirm_text, &body, art, &mut seen);
    view.set_visible(&nodes.confirm_text, shows);
    let shows = put_text(&mut view, &nodes.positive_label, &layouts.wording(super::start::OK), art, &mut seen);
    view.set_visible(&nodes.positive, true);
    view.set_visible(&nodes.positive_label, shows);
    let pass = ctx.pass_valid();
    view.set_visible(&nodes.pass_balloon, pass);
    if pass {
        let rate = (ctx.machine().convert_time_decrease_rate() * 100.0).round() as i32;
        let line = format_line(&layouts, &mut seen, PASS_SHORTEN, &[rate.to_string()]);
        let shows = put_text(&mut view, &nodes.pass_text, &line, art, &mut seen);
        view.set_visible(&nodes.pass_text, shows);
    }
    // `RefreshPassExpirationWarning`:
    // `ClientConfig.Mysekai.ConvertStartConfirmRemainingDaysUntilPassExpirationWarning`
    // is `ClientConfigs`'s int key [`PASS_WARNING_DAYS_KEY`]. The client
    // config panel is the one source of the value: without it the warning
    // path refuses by name (it is never defaulted to "no warning", which
    // would hide a pass about to end).
    match configs.as_deref() {
        Some(configs) => {
            let show = shows_pass_warning(&ctx.user.pass, ctx.now, configs.int(PASS_WARNING_DAYS_KEY));
            if show {
                let line = layouts.wording(PASS_WARNING);
                let shows = put_text(&mut view, &nodes.warning_text, &line, art, &mut seen);
                view.set_visible(&nodes.warning_text, shows);
            } else {
                view.set_visible(&nodes.warning_text, false);
            }
        }
        None => {
            error!(
                "[convert] MysekaiConvertStartConfirmDialog: the client config panel (ClientConfigs, IntConfigs key {PASS_WARNING_DAYS_KEY}) is not loaded; RefreshPassExpirationWarning cannot read its days, so the warning is refused and the confirm shows no warning"
            );
            view.set_visible(&nodes.warning_text, false);
        }
    }
    dialogs.seen = seen;
}

/// Update: the skip's `Refresh` each frame: `UpdateSlot()` then `RefreshView`.
pub fn setup_shorten(
    mut layouts: ResMut<UiLayouts>,
    mut dialogs: ResMut<ConvertDialogs>,
    data: DialogData,
    mut views: Query<(&Nodes, &mut UiPrefabView), With<ShortenRoot>>,
) {
    let Some(state) = dialogs.shorten_data else {
        return;
    };
    if dialogs.shorten.is_none() {
        return;
    }
    let (Some(user), Some(masters)) = (data.user(), data.masters()) else {
        return;
    };
    let Ok((nodes, mut view)) = views.single_mut() else {
        return;
    };
    let ctx = ItemContext { masters, user: &user, now: data.now() };
    let rows = Masters::new(&data.tables.rows);
    let mut seen = std::mem::take(&mut dialogs.seen);
    let art = data.art.as_deref();
    let model = SlotModel::new(ctx, state.slot_id, state.pass_slot);
    let price = model.consumed_jewel_quantity_for_shorten(&ctx.machine());
    // The account's crystal wallet (`CurrentTotalJewel`, `CurrentPaidJewel`)
    // is not in the user document (the server names the same absence), so
    // the owned amounts have no value: the nodes that show them are hidden
    // rather than drawn with a made-up number, once, by name.
    view.set_visible(&nodes.shorten_have, false);
    view.set_visible(&nodes.shorten_have_paid, false);
    note_once(
        &mut seen,
        format!(
            "MysekaiConvertFurnitureShortenDialog: the owned-crystal nodes {} (CurrentTotalJewel) and {} (CurrentPaidJewel) are hidden: the account's crystal wallet is not in the user document, so there is no owned amount to show; the price line still shows the skip's cost",
            nodes.shorten_have, nodes.shorten_have_paid
        ),
    );
    let name = model
        .item_id()
        .and_then(|id| ItemModel::all(ctx).into_models().into_iter().find(|model| model.id() == id))
        .and_then(|model| super::item_select::item_name(&layouts, ctx, &model, &rows));
    match name {
        Some(name) => {
            let confirm = format_line(&layouts, &mut seen, SHORTEN_CONFIRM, &[price.to_string(), name]);
            let shows = put_text(&mut view, &nodes.shorten_confirm, &confirm, art, &mut seen);
            view.set_visible(&nodes.shorten_confirm, shows);
        }
        None => {
            // The confirm line formats the item's name (`{1}`); without it the
            // line is hidden, never formatted with an empty name.
            view.set_visible(&nodes.shorten_confirm, false);
            note_once(
                &mut seen,
                format!(
                    "MysekaiConvertFurnitureShortenDialog: the confirm line {} is hidden: the slot's item {:?} has no readable name (no item model for its id, or the master lacks the name row), so {SHORTEN_CONFIRM} is not formatted",
                    nodes.shorten_confirm,
                    model.item_id()
                ),
            );
        }
    }
    let remaining = super::slot::to_time_string(model.remaining_time_to_complete_convert());
    put_text(&mut view, &nodes.shorten_current, &remaining, art, &mut seen);
    put_text(&mut view, &nodes.shorten_cost, &price.to_string(), art, &mut seen);
    // The price's tint (`FONT_COLOR_PINK2` over the owned total,
    // `FONT_COLOR_BLACK` otherwise) needs the owned total, which is not in
    // the user document: without it the price keeps the document's colour
    // (the two entries are still read, so a root without a palette is named).
    if jewel_color(&layouts, PINK2_ENTRY).is_none() || jewel_color(&layouts, BLACK_ENTRY).is_none() {
        note_once(&mut seen, "the layout carries no palette: the skip's price keeps the document's colour".to_owned());
    }
    let label = layouts.wording(SHORTEN_DECIDE);
    put_text(&mut view, &nodes.positive_label, &label, art, &mut seen);
    view.set_visible(&nodes.shorten_not_enough, false);
    dialogs.seen = seen;
}

/// Update: the item detail's `Setup(model)`.
pub fn setup_detail(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut dialogs: ResMut<ConvertDialogs>,
    data: DialogData,
    mut views: Query<(&Nodes, &mut UiPrefabView), With<DetailRoot>>,
) {
    let Some(item) = dialogs.detail_item else {
        return;
    };
    if dialogs.detail.is_none() {
        return;
    }
    let (Some(user), Some(masters)) = (data.user(), data.masters()) else {
        return;
    };
    let Ok((nodes, mut view)) = views.single_mut() else {
        return;
    };
    let ctx = ItemContext { masters, user: &user, now: data.now() };
    let rows = Masters::new(&data.tables.rows);
    let mut seen = std::mem::take(&mut dialogs.seen);
    let art = data.art.as_deref();
    let Some(model) = ItemModel::all(ctx)
        .into_models()
        .into_iter()
        .find(|model| model.id() == item)
    else {
        note_once(&mut seen, format!("the item detail's item {item} is not read: nothing is set up"));
        dialogs.seen = seen;
        return;
    };
    let path = thumbnail(&mut layouts, &server, data.icons.as_deref(), ctx, &model, &rows, &mut seen);
    view.set_visible(&nodes.detail_thumbnail, path.is_some());
    if let Some(path) = &path {
        view.set_texture(&nodes.detail_icon, path);
    }
    if let Some(name) = super::item_select::item_name(&layouts, ctx, &model, &rows) {
        let shows = put_text(&mut view, &nodes.detail_name, &name, art, &mut seen);
        view.set_visible(&nodes.detail_name, shows);
    }
    match item_description(&layouts, ctx, &model, &rows, &mut seen) {
        Some(description) => {
            let shows = put_text(&mut view, &nodes.detail_description, &description, art, &mut seen);
            view.set_visible(&nodes.detail_description, shows);
        }
        None => view.set_visible(&nodes.detail_description, false),
    }
    dialogs.seen = seen;
}

fn format_line(
    layouts: &UiLayouts,
    seen: &mut BTreeSet<String>,
    key: &str,
    args: &[String],
) -> String {
    match moly_law::text::custom_text_mesh::format_wording(&layouts.wording(key), args) {
        Ok(line) => line,
        Err(error) => {
            note_once(seen, format!("the wording {key} does not take {args:?}: {error}"));
            String::new()
        }
    }
}

fn put_text(
    view: &mut UiPrefabView,
    path: &str,
    text: &str,
    art: Option<&BalloonArt>,
    seen: &mut BTreeSet<String>,
) -> bool {
    super::screen::put_text(view, path, Some(&text.to_owned()), art, seen)
}

// ---------------------------------------------------------------------------
// Systems: the taps and the back key
// ---------------------------------------------------------------------------

/// The shown dialog's view, canvas and window.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DialogWorld<'w, 's> {
    pub windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    pub root_canvas: Option<Res<'w, RootCanvas>>,
    pub views: Query<'w, 's, (&'static DialogOf, &'static Nodes, &'static UiPrefabView)>,
}

/// Update: the shown dialog takes the taps. Its positive button (`OnClickOK`)
/// answers its OK, the window's close button and the second footer button
/// cancel, and every other tap is the dialog's (its slot is modal).
#[allow(clippy::too_many_arguments)]
pub fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut dialogs: ResMut<ConvertDialogs>,
    mut screens: ResMut<ScreenManager>,
    mut screen: ResMut<ConvertScreen>,
    mut consumed: ResMut<ActionTapConsumed>,
    layouts: Res<UiLayouts>,
    world: DialogWorld,
    mut sounds: ResMut<SeRequests>,
) {
    let shown = match (dialogs.start, dialogs.shorten, dialogs.detail) {
        (Some(id), _, _) => Some((id, Which::Start)),
        (_, Some(id), _) => Some((id, Which::Shorten)),
        (_, _, Some(id)) => Some((id, Which::Detail)),
        _ => None,
    };
    let Some((_, which)) = shown else {
        gestures.clear();
        return;
    };
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(canvas)) = (world.windows.single(), world.root_canvas.as_deref()) else {
        return;
    };
    let Some((_, nodes, view)) = world
        .views
        .iter()
        .find(|(owner, _, _)| owner.0 == which)
    else {
        return;
    };
    let size = canvas.size(window);
    let scale = canvas.scale(window);
    let key = match which {
        Which::Start => START_VIEW,
        Which::Shorten => SHORTEN_VIEW,
        Which::Detail => DETAIL_VIEW,
    };
    for position in taps {
        consumed.0 = true;
        let point = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        let hit = |path: &str| {
            view.rect(&layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(point))
        };
        if hit(&nodes.close_button) {
            layouts.queue_button_se(&mut sounds, key, &nodes.close_button);
            let by = match which {
                Which::Start => "tap on the window's close button (OnCloseExternal -> OnClickCancel)",
                Which::Shorten => "tap on the window's close button (OnCloseExternal -> OnClickCancel)",
                Which::Detail => "tap on the window's close button (OnCloseExternal -> OnClickOK)",
            };
            close(by, &mut dialogs, &mut screens, which);
            return;
        }
        if which != Which::Detail && hit(&nodes.second) {
            layouts.queue_button_se(&mut sounds, key, &nodes.second);
            close("tap on the footer's cancel button (OnClickCancel -> DialogBase.Close)", &mut dialogs, &mut screens, which);
            return;
        }
        if hit(&nodes.positive) {
            layouts.queue_button_se(&mut sounds, key, &nodes.positive);
            match which {
                // `OnClickOK` -> `ConvertStartProcess`; the request the
                // screen holds waits for the start chain's send.
                Which::Start => close("tap on the footer's OK (OnClickOK -> ConvertStartProcess -> DialogBase.Close)", &mut dialogs, &mut screens, which),
                // `OnClickOK` -> `OnClickOkShortenSlot`.
                Which::Shorten => close("tap on the footer's skip (OnClickOK -> OnClickOkShortenSlot -> DialogBase.Close)", &mut dialogs, &mut screens, which),
                // `Common1ButtonDialog.OnClickOK` with a null callback.
                Which::Detail => close("tap on the footer's OK (OnClickOK -> DialogBase.Close)", &mut dialogs, &mut screens, which),
            }
            return;
        }
        if which == Which::Shorten && hit(&nodes.shorten_shop) {
            layouts.queue_button_se(&mut sounds, key, &nodes.shorten_shop);
            // `ToJewelShop`: `Close()`, then `PushUIScreen(MenuScreenType
            // .CrystalShop)`; the screen is not carried.
            note_once(
                &mut dialogs.seen,
                "the skip dialog's crystal shop button: MenuScreenType.CrystalShop is not carried; the dialog closes and no screen is pushed".to_owned(),
            );
            close("tap on the crystal shop button (ToJewelShop)", &mut dialogs, &mut screens, which);
            return;
        }
        if which == Which::Shorten && hit(&nodes.shorten_law) {
            layouts.queue_button_se(&mut sounds, key, &nodes.shorten_law);
            note_once(
                &mut dialogs.seen,
                "the skip dialog's law button opens CommonWebviewDialog (SpecifiedCommercialTransactionLawArticle12_6), which has no prefab: nothing opens".to_owned(),
            );
            continue;
        }
        let _ = &mut screen;
    }
}

/// Update: the hardware back key of a shown dialog closes it (its
/// `OnHardwareBackKeyProcess`).
pub fn back_key(
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut dialogs: ResMut<ConvertDialogs>,
    mut screens: ResMut<ScreenManager>,
) {
    let events: Vec<DialogBackKeyEvent> = back_keys.read().copied().collect();
    for event in events {
        let which = if dialogs.start == Some(event.id) {
            Which::Start
        } else if dialogs.shorten == Some(event.id) {
            Which::Shorten
        } else if dialogs.detail == Some(event.id) {
            Which::Detail
        } else {
            continue;
        };
        close("hardware back key (OnHardwareBackKeyProcess -> OnClickCancel / OnClickOK)", &mut dialogs, &mut screens, which);
    }
}

/// Update: each dialog's visibility and canvas scale while its slot is up.
pub fn place(
    dialogs: Res<ConvertDialogs>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut views: Query<(&DialogOf, &mut Visibility, &mut Transform)>,
) {
    let up = |which: Which| match which {
        Which::Start => dialogs.start.is_some(),
        Which::Shorten => dialogs.shorten.is_some(),
        Which::Detail => dialogs.detail.is_some(),
    };
    for (owner, mut visibility, mut transform) in &mut views {
        visibility.set_if_neq(if up(owner.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let (Ok(window), Some(canvas)) = (windows.single(), root_canvas.as_deref()) {
            crate::canvas::set_scale(&mut transform, canvas.scale(window));
        }
    }
}


#[cfg(test)]
mod value_checks {
    use super::*;

    /// `IsShowsPassExpirationWarning`: not more than the configured days left,
    /// whole days (`TotalDays <= days`), and never for a pass that ended.
    #[test]
    fn the_pass_warning_counts_whole_days() {
        let now = 1_000_000;
        let pass = PassState { expired_at: now + 3 * DAY_MS };
        assert!(shows_pass_warning(&pass, now, 3));
        assert!(shows_pass_warning(&pass, now, 30));
        assert!(!shows_pass_warning(&pass, now, 2));
        assert!(!shows_pass_warning(&PassState { expired_at: 0 }, now, 30));
        // A day and a half over three days is over the bound.
        let late = PassState { expired_at: now + 3 * DAY_MS + DAY_MS / 2 };
        assert!(!shows_pass_warning(&late, now, 3));
    }

    /// The palette entries the two font colours read (`ColorEntry.text_dbl`
    /// 14 and `text_rd` 17 in the dumped enum).
    #[test]
    fn the_price_colour_entries() {
        assert_eq!(BLACK_ENTRY, 14);
        assert_eq!(PINK2_ENTRY, 17);
    }
}
