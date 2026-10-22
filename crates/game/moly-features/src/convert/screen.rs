//! `ScreenLayerMysekaiConvertFurniture` (screen 627): the convert screen's
//! shell, its presenter and the two pages it switches between.
//!
//! Source structure:
//! - `OnBoot` -> `BootProcess`: `Presenter.Initialize` (`SetupViewStateMachine`
//!   registers the slot status state as `ViewType.Slot` and the item select
//!   state as `ViewType.ItemSelect`), `Presenter.LoadAssetBundle`
//!   (`"mysekai/ui/mysekai_convert_furniture"`), then `ScreenBootDone`.
//! - `OnInitComponent`: `Presenter.Setup` (`HideAll()`, then
//!   `ChangeView(ViewType.Slot)`), `SoundManager.PlaySEOneShot("se_enter_converter")`,
//!   and the tutorial slide (`TutorialUtility.ShowTutorialSlideByType`).
//! - `ChangeView(ViewType.Slot)`: `ChangeTo(Slot)` and
//!   `ClearOnBackUIScreenOverride()` (the back key pops the screen).
//!   `ChangeView(ViewType.ItemSelect)`: `OnBackUIScreenOverride = _ =>
//!   ChangeView(ViewType.Slot)` (the back key returns to the slot page and
//!   does not pop). `HideAll` hides both pages; each state's `OnExit` hides
//!   its own page.
//! - `OnWillExit`: `ScreenWillExitDone` (the exit never waits). `OnExited` ->
//!   `Presenter.Dispose`: the back override cleared, both states disposed,
//!   `UnloadAssetBundle`. `Update`: the view state machine's `Update` (the
//!   slot status state's per-frame refresh in [`super::slot`], the item select
//!   state's in [`super::item_select`]).
//! - `MysekaiConvertUtility.MoveScreenLayerMysekaiConvert` pushes the screen;
//!   the action button path already sends `LayerCommand::Push(MysekaiConvert)`.
//!
//! The two pages (`MysekaiConvertFurnitureSlotView` and
//! `MysekaiConvertFurnitureItemSelectView`) are exclusive. `Show` / `Hide`
//! set a page CanvasGroup's alpha, `interactable` and `blocksRaycasts`; this
//! view draws a page while it shows and hides it, with its subtree, while it
//! does not (the CanvasGroup's alpha is not what the layout draws).
//!
//! The item select page composes its own document: the screen's document with
//! the page's tab cells, its cell pool and the detail panel's consumption rows
//! instantiated into it. It is drawn from [`ITEM_SELECT_RUNTIME`], a second
//! view, because its cells only exist in that composition.
//!
//! Product mapping, named:
//! - The boot waits for the convert master tables, the screen document, the
//!   item select page's composition (which needs the tab and list cell
//!   documents) and the user's convert data (slots, histories, items, pass,
//!   machine level and the holdings a consumption box is paid from); the
//!   source's data manager holds all of them when it boots.
//! - The back key: `OnBackUIScreenOverride` is the screen manager's, and its
//!   override list (`BackOverride`) is another module's. This view answers the
//!   back key itself (Escape, the desktop form of the header's back button):
//!   on the item select page it returns to the slot page (the source's
//!   delegate, no pop); on the slot page it asks for `BackUIScreen`
//!   (`LayerCommand::Pop`). Until the manager carries a convert override of
//!   its own, its own back-key path (the header's button) pops on either page
//!   (named).
//! - `LoadAssetBundle` / `UnloadAssetBundle` load and free prefabs from a
//!   bundle that is not part of the public asset root (the effects, the
//!   lottery animations, the slot page's animation): nothing is loaded or
//!   freed here (named once).
//! - The environment SE `se_trading` (`StartTradingSE`, `StopEnvironmentSE(0)`)
//!   has no environment channel in this tree's SE queue: it is queued as an
//!   in-game one-shot on a stand-in period while a slot processes (named,
//!   unverified against the cue's length).
//! - The tutorial slide (`MYSEKAI_CONVERT`) is not carried (named once).
//! - The item select page's long press opens `MysekaiConvertItemDetailDialog`
//!   (the convert dialogs' unit: one named line); its start button makes
//!   `ConfirmConvertStart`'s request and the slot page's skip button
//!   `OnShortenSlot`'s, both for the dialog of the convert dialogs' unit to
//!   answer, and [`super::start`] sends and completes the request after it.
//! - The item select page's cells draw the client's item icons as runtime
//!   textures (the raw images need no Sprite metrics).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use moly_protocol::rows::ColorfulPass;

use super::detail::Masters;
use super::item_select::{self, ItemAction, ItemSelectPage, PageBindings};
use super::model::{ConvertUserState, Holdings, ItemContext, PassState};
use super::slot::{self, SlotAction, SlotCells};
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::canvas::RootCanvas;
use crate::gesture::{UiPointerEvent, UiPointerPhase};
use crate::item_icon::ItemIcons;
use crate::menu_shell::ShellDialogState;
use crate::ui_inputs::{event_pointer, to_canvas, BalloonArt, SCREEN_UI_LAYER};
use crate::ui_layers::{
    LayerCommand, MenuScreenType, ScreenHook, ScreenLayerEvent, ScreenLayerState, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::client::inventory::ClientMysekaiInventory;
use crate::userdata::client::ClientUserData;
use crate::userdata::convert::ClientConvertSlots;

/// The screen layer's document key
/// (`screens/ScreenLayerMysekaiConvertFurniture.json`).
pub const KEY: &str = "ConvertFurniture";
/// The item select page's composed document.
pub const ITEM_SELECT_RUNTIME: &str = "ConvertItemSelectRuntime";
/// The screen (`MenuScreenType.MysekaiConvertFurniture`).
pub const SCREEN: MenuScreenType = MenuScreenType::MysekaiConvertFurniture;
const SLOT_PAGE_CLASS: &str = "MysekaiConvertFurnitureSlotView";
const ITEM_SELECT_PAGE_CLASS: &str = "MysekaiConvertFurnitureItemSelectView";
/// `SoundManager.PlaySEOneShot("se_enter_converter")`.
const ENTER_SE: &str = "se_enter_converter";
/// `StartTradingSE`: `PlayEnvironmentSE("se_trading", 0, 1)`.
pub const TRADING_SE: &str = "se_trading";
/// The stand-in period of the `se_trading` loop request (its cue's length is
/// in the bundle's loop table, which this product does not read for it).
const TRADING_PERIOD: Duration = Duration::from_millis(2_000);
/// `TutorialUtility.ShowTutorialSlideByType("MYSEKAI_CONVERT", null)`.
const TUTORIAL: &str = "MYSEKAI_CONVERT";

/// `ViewType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewType {
    /// `ViewType.Slot` (`MysekaiConvertFurnitureSlotStatusState`).
    #[default]
    Slot,
    /// `ViewType.ItemSelect` (`MysekaiConvertFurnitureItemSelectState`).
    ItemSelect,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Logs one line once per distinct message.
pub fn note_once(seen: &mut BTreeSet<String>, message: String) {
    if seen.insert(message.clone()) {
        info!("convert: {message}");
    }
}

/// The one line a unit leaves where a later unit's call stands:
/// `"convert: <what> is not built yet"`, and nothing else.
pub fn not_built(seen: &mut BTreeSet<String>, what: &str) {
    note_once(seen, format!("{what} is not built yet"));
}

/// The serialized `CustomButton` of a node (the pressable the source's click
/// comes from), when the node has one.
#[allow(dead_code)] // The dialogs' units (U7) read the buttons by field.
pub fn button_id(doc: &UiPrefab, path: &str) -> Option<i64> {
    let index = doc.find(path).ok()?;
    doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class == "Sekai.UI.CustomButton")
        .map(|component| component.path_id)
}

/// The node path of the first node with a component whose class ends with
/// `suffix`.
pub fn class_path(doc: &UiPrefab, suffix: &str) -> Option<String> {
    doc.nodes
        .iter()
        .find(|node| {
            node.components
                .iter()
                .any(|component| component.class.ends_with(&format!(".{suffix}")))
        })
        .map(|node| node.path.clone())
}

/// A node's serialized component whose class ends with `suffix`.
#[allow(dead_code)] // The dialogs' units (U7) read fields.
pub fn class_component<'a>(
    doc: &'a UiPrefab,
    path: &str,
    suffix: &str,
) -> Result<&'a UiComponent, String> {
    let index = doc.find(path).map_err(|error| error.to_string())?;
    doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class.ends_with(&format!(".{suffix}")))
        .ok_or_else(|| format!("{path} has no {suffix} component"))
}

/// A serialized reference of a component: `None` for null; a reference out
/// of the prefab is refused (the layout's own rule).
#[allow(dead_code)] // The dialogs' units (U7) read fields.
pub fn reference(fields: &serde_json::Value, name: &str) -> Option<i64> {
    let pair = fields.get(name)?.as_array()?;
    assert_eq!(
        pair.first().and_then(serde_json::Value::as_i64),
        Some(0),
        "a serialized reference points outside the prefab"
    );
    pair.get(1)
        .and_then(serde_json::Value::as_i64)
        .filter(|id| *id != 0)
}

/// The user side the model reads, from the client copies; `None` until the
/// client holds the convert slots (the boot then waits). `holdings` is what a
/// consumption box is paid from (`UserDataUtility.GetHaveQuantity`).
pub fn read_user(
    client: Option<&ClientUserData>,
    copies: Option<&ClientConvertSlots>,
    inventory: Option<&ClientMysekaiInventory>,
) -> Option<ConvertUserState> {
    let copies = copies?;
    let level = client.and_then(|client| client.gamedata.mysekai_convert_fixture_level);
    let pass = client.and_then(|client| client.colorful_pass).unwrap_or(ColorfulPass {
        id: 0,
        expired_at: 0,
    });
    let mut holdings = Holdings::default();
    if let Some(inventory) = inventory {
        for row in inventory.materials() {
            holdings
                .materials
                .insert(row.mysekai_material_id, row.quantity);
        }
        for row in inventory.items() {
            holdings.items.insert(row.mysekai_item_id, row.quantity);
        }
        for row in inventory.fixtures() {
            *holdings.fixtures.entry(row.mysekai_fixture_id).or_insert(0) += row.quantity;
        }
    }
    Some(ConvertUserState {
        slots: copies.slots().to_vec(),
        histories: copies.histories().to_vec(),
        items: copies.items().to_vec(),
        pass: PassState {
            expired_at: pass.expired_at,
        },
        level,
        holdings,
    })
}

/// `TimeUtility.GetCurrentTimestamp`: the client's last server date plus the
/// real time since it; the device clock without a copy.
pub fn now_ms(client: Option<&ClientUserData>, realtime: f32) -> i64 {
    match client {
        Some(client) => client.current_timestamp(realtime),
        None => crate::userdata::clock::device_now_ms(),
    }
}

/// The root canvas' size for a view's layout, or the reference resolution
/// while the window is not readable.
fn item_canvas(
    windows: &Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<&RootCanvas>,
) -> Vec2 {
    match (windows.single(), root_canvas) {
        (Ok(window), Some(root)) => root.size(window),
        _ => Vec2::new(1920.0, 1080.0),
    }
}

/// A text a node can draw: every non-blank character has a glyph cell in the
/// shared atlas. Answers whether the node may show; with no text to set the
/// node keeps its own.
pub fn put_text(
    view: &mut UiPrefabView,
    path: &str,
    text: Option<&String>,
    art: Option<&BalloonArt>,
    warned: &mut BTreeSet<String>,
) -> bool {
    let Some(text) = text else {
        return true;
    };
    if text.is_empty() {
        view.set_text(path, String::new());
        return true;
    }
    let Some(art) = art else {
        return false;
    };
    if let Some(missing) = text
        .chars()
        .find(|ch| !ch.is_whitespace() && art.glyph_cell(*ch).is_none())
    {
        if warned.insert(format!("{path} {missing}")) {
            warn!(
                "convert: the text {text:?} at {path} has the character {missing:?}, which the shared glyph atlas lacks: the node is hidden"
            );
        }
        return false;
    }
    view.set_text(path, text.clone());
    true
}

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

/// The screen layer's serialized pages.
#[derive(Debug, Clone)]
pub struct ScreenBindings {
    pub slot_page: String,
    pub item_select_page: String,
}

/// The screen document's pages and the slot page's cells; refused by name.
pub fn bind(doc: &UiPrefab) -> Result<(ScreenBindings, slot::SlotBinding), String> {
    let slot_page =
        class_path(doc, SLOT_PAGE_CLASS).ok_or_else(|| format!("no {SLOT_PAGE_CLASS}"))?;
    let item_select_page = class_path(doc, ITEM_SELECT_PAGE_CLASS)
        .ok_or_else(|| format!("no {ITEM_SELECT_PAGE_CLASS}"))?;
    let slot_binding = slot::bind_page(doc)?;
    Ok((
        ScreenBindings {
            slot_page,
            item_select_page,
        },
        slot_binding,
    ))
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// `MysekaiConvertFurniturePresenter`, the layer's view and the two pages.
#[derive(Resource, Default)]
pub struct ConvertScreen {
    bindings: Option<Arc<ScreenBindings>>,
    /// `MysekaiConvertFurnitureSlotView` and its cells.
    slot: Option<SlotCells>,
    /// `MysekaiConvertFurnitureItemSelectView`'s state.
    item_select: Option<ItemSelectPage>,
    /// The item select page's composed nodes.
    item_select_bindings: Option<Arc<PageBindings>>,
    /// Why the screen cannot be drawn on this root.
    absent: Option<String>,
    /// The view is spawned.
    spawned: bool,
    /// `OnBoot` set the page up (`Initialize` and `ScreenBootDone`).
    booted: bool,
    /// `OnInitComponent` ran (`Setup`).
    set_up: bool,
    /// The item select page shows (`ViewType.ItemSelect`); the back key then
    /// returns to the slot page instead of popping.
    on_item_select: bool,
    /// The trading SE is queued.
    trading: bool,
    /// The next trading SE request.
    trading_next: Duration,
    /// The screen answered its refused push with a pop.
    refusal_popped: bool,
    /// `ConfirmConvertStart`'s request, waiting for the start confirm
    /// dialog's OK ([`super::start`]).
    pub start_intent: Option<super::start::StartIntent>,
    /// `ConvertStartProcess`'s request, sent when the dialog's OK passed the
    /// pass checks.
    pub start_pending: Option<super::start::StartIntent>,
    /// The start sent: its slot and the call's number.
    pub start_sent: Option<(i32, u32)>,
    /// `OnShortenSlot`'s request, waiting for the skip dialog's OK.
    pub shorten_intent: Option<super::start::ShortenIntent>,
    /// `OnClickOkShortenSlot`'s request, sent for a processing slot.
    pub shorten_pending: Option<super::start::ShortenIntent>,
    /// The skip sent: its slot and the call's number.
    pub shorten_sent: Option<(i32, u32)>,
    /// `ConvertStartProcess`'s pass dialog, waiting to show or shown
    /// ([`super::start::pass_dialogs`]).
    pub pass_dialog: Option<super::start::PassDialog>,
    /// `ConvertReceiveProcessAsync(slot)`'s slot, waiting for the receive
    /// chain ([`super::receive`]).
    pub receive_intent: Option<i32>,
    /// `ShowDetailDialog(model)`'s item, waiting for the item detail dialog
    /// ([`super::dialogs`]).
    pub detail_request: Option<i32>,
    pub seen: BTreeSet<String>,
}

impl ConvertScreen {
    /// The active page.
    pub fn view(&self) -> ViewType {
        if self.on_item_select {
            ViewType::ItemSelect
        } else {
            ViewType::Slot
        }
    }

    /// Why the screen cannot be drawn on this root, once it is known.
    #[allow(dead_code)] // The refusal is logged and popped; this reads it for a host's panel.
    pub fn absent(&self) -> Option<&str> {
        self.absent.as_deref()
    }

    /// The lines the presenter has noted, for a feature that holds a
    /// `&mut ConvertScreen` and names its own refused lines.
    pub fn seen_mut(&mut self) -> &mut BTreeSet<String> {
        &mut self.seen
    }

    /// `OnInitComponent` ran (`Presenter.Setup`): the screen's page is up and
    /// its dialogs may open.
    pub(crate) fn is_set_up(&self) -> bool {
        self.set_up
    }

    /// `SlotStatusState.PlayConvertStartAsync(slotId)`'s call into the slot
    /// page ([`SlotCells::play_convert_start`]); no page has nothing to play.
    pub(crate) fn slot_play_convert_start(
        &mut self,
        model: &super::model::SlotModel<'_>,
        ctx: super::model::ItemContext<'_>,
        layouts: &UiLayouts,
    ) {
        if let Some(slot) = self.slot.as_mut() {
            slot.play_convert_start(model, ctx, layouts);
        }
    }

    /// `SlotStatusState.PlayShortenAsync(slotId, currentProgress)`'s call into
    /// the slot page ([`SlotCells::play_shorten`]).
    pub(crate) fn slot_play_shorten(&mut self, slot_id: i32, progress: f32) {
        if let Some(slot) = self.slot.as_mut() {
            slot.play_shorten(slot_id, progress);
        }
    }

    /// `SlotStatusState.PlaySlotReceiveAsync(slotId, from, to)`'s call into
    /// the slot page ([`SlotCells::play_receive`]).
    pub(crate) fn slot_play_receive(&mut self, slot_id: i32, from: f32, to: f32) {
        if let Some(slot) = self.slot.as_mut() {
            slot.play_receive(slot_id, from, to);
        }
    }

    /// `ItemSelectView.Refresh()` of the item select page, when it shows.
    pub(crate) fn refresh_item_select(&mut self, ctx: ItemContext<'_>) {
        if let Some(page) = self.item_select.as_mut() {
            page.refresh(ctx);
        }
    }

    /// `ItemSelectView.OnStartConvert`: the slot the page converts in and the
    /// detail panel's item with its `ConvertCount` (`_onConvertStart(
    /// _convertSlotId, model)`), while the item select page shows.
    pub fn start_selection(&self) -> Option<(i32, i32, i32)> {
        if self.view() != ViewType::ItemSelect {
            return None;
        }
        let page = self.item_select.as_ref()?;
        Some((page.slot_id(), page.item()?, page.detail.count()))
    }

    fn slot(&self) -> Option<&SlotCells> {
        self.slot.as_ref()
    }

    fn slot_mut(&mut self) -> Option<&mut SlotCells> {
        self.slot.as_mut()
    }

    /// `ItemSelectState.OnEnter`: `ItemSelectView.Setup(slotId)`, `Refresh()`
    /// and `Show()`.
    fn open_item_select(&mut self, slot_id: i32, ctx: ItemContext<'_>) {
        let tabs = super::model::tabs(ctx.masters);
        let page = self.item_select.get_or_insert_with(ItemSelectPage::default);
        page.setup(slot_id, &tabs);
        page.refresh(ctx);
    }

    /// `ItemSelectState.OnExit`: `ItemSelectView.Hide()`. The state is
    /// dropped, so the next `ChangeView(ViewType.ItemSelect)` runs `Setup`
    /// again over the slot it enters on.
    fn close_item_select(&mut self) {
        if let Some(page) = self.item_select.as_mut() {
            page.hide();
        }
        self.item_select = None;
    }

    /// Prepares the slot page's binding once the document is loaded.
    fn prepare(&mut self, layouts: &UiLayouts) {
        if self.slot.is_some() || self.absent.is_some() {
            return;
        }
        let Some(doc) = layouts.document(KEY) else {
            return;
        };
        match slot::bind_page(doc) {
            Ok(binding) => self.slot = Some(SlotCells::new(binding.page, binding.cells)),
            Err(reason) => {
                error!("[convert] the slot page is refused: {reason}");
                self.absent.get_or_insert(reason);
            }
        }
    }

    /// `ChangeView(view)`; the item select page's state opens on the next
    /// frame, when the user data and the master tables are read.
    pub(crate) fn change_view(&mut self, view: ViewType) {
        if self.view() == view {
            return;
        }
        match view {
            ViewType::Slot => {
                self.on_item_select = false;
                // `ItemSelectState.OnExit`.
                self.close_item_select();
                if let Some(slot) = self.slot_mut() {
                    slot.show_slot_page();
                }
                info!("[convert] ChangeView(ViewType.Slot): ItemSelectState.OnExit, ChangeTo(Slot), ClearOnBackUIScreenOverride()");
            }
            ViewType::ItemSelect => {
                if self.item_select_bindings.is_none() {
                    // The page's nodes were not composed, so the page has
                    // nothing to show: the slot page stays (named once).
                    note_once(
                        &mut self.seen,
                        "ChangeView(ViewType.ItemSelect) on a root without the page's composed nodes: the slot page stays".to_owned(),
                    );
                    if let Some(slot) = self.slot_mut() {
                        slot.show_slot_page();
                    }
                    return;
                }
                self.on_item_select = true;
                // `SlotStatusState.OnExit`.
                if let Some(slot) = self.slot_mut() {
                    slot.hide_slot_page();
                }
                info!("[convert] ChangeView(ViewType.ItemSelect): ChangeTo(ItemSelect), OnBackUIScreenOverride = _ => ChangeView(ViewType.Slot)");
            }
        }
    }
}

#[derive(Component)]
pub struct ConvertScreenRoot;

/// The item select page's composed view.
#[derive(Component)]
pub struct ConvertItemSelectRoot;

/// The slot page's pointer press and the item select page's.
#[derive(Resource, Default)]
pub struct ConvertPress {
    held: Option<(i32, Vec2)>,
    /// The source button (node, component) the press went to on the slot
    /// page.
    pressed: Option<(usize, i64)>,
    /// The node the press went to on the item select page, across frames.
    item_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Update: the views spawn once the document, the convert master tables, the
/// item select page's cell documents and the user's convert data are all in.
#[allow(clippy::too_many_arguments)]
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    tables: Res<moly_masters::load::MasterTables>,
    client: Option<Res<ClientUserData>>,
    copies: Option<Res<ClientConvertSlots>>,
    inventory: Option<Res<ClientMysekaiInventory>>,
    item_icons: Option<Res<ItemIcons>>,
    real: Res<Time<Real>>,
    mut screen: ResMut<ConvertScreen>,
) {
    if screen.spawned || screen.absent.is_some() {
        return;
    }
    screen.prepare(&layouts);
    if screen.absent.is_some() {
        return;
    }
    if layouts.document(KEY).is_none() {
        return;
    }
    let Some(masters) = tables.convert.as_ref() else {
        return;
    };
    let Some(user) = read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()) else {
        return;
    };
    if !layouts.ready(KEY, &server) {
        return;
    }
    // The item select page's composition needs three shared cell documents:
    // the tab cell, the list cell and the consumption row. Each is an optional
    // document of the root; a missing one is named and the page is composed
    // without its cells.
    let cells_ready = [
        item_select::TAB_CELL,
        item_select::ITEM_CELL,
        super::detail::COST_DOCUMENT,
    ]
    .into_iter()
    .all(|key| layouts.document(key).is_none() || layouts.ready(key, &server));
    if !cells_ready {
        return;
    }
    let bound = layouts.document(KEY).map(bind);
    match bound {
        None => {}
        Some(Err(reason)) => {
            error!("[convert] {KEY} is refused: {reason}; the screen is popped when it is pushed");
            screen.absent.get_or_insert(reason);
        }
        Some(Ok((bindings, slot_binding))) => {
            let mut view = UiPrefabView::new(KEY, SCREEN_UI_LAYER);
            // `HideAll`.
            view.set_visible(&bindings.item_select_page, false);
            let tabs = super::model::tabs(masters);
            match item_select::compose(&layouts, tabs.len()) {
                Ok((document, page)) => {
                    match layouts.replace_runtime_document(ITEM_SELECT_RUNTIME, document, &server) {
                        Ok(()) => {
                            // The cells' item icons: those the root's index
                            // holds, by the load paths the page uses.
                            let now = now_ms(client.as_deref(), real.elapsed_secs());
                            let ctx = ItemContext {
                                masters,
                                user: &user,
                                now,
                            };
                            let rows = Masters::new(&tables.rows);
                            let paths = item_select::icon_loads(ctx, &rows);
                            register_icons(
                                &mut layouts,
                                &server,
                                item_icons.as_deref(),
                                &paths,
                                &mut screen.seen,
                            );
                            screen.item_select_bindings = Some(Arc::new(page));
                        }
                        Err(reason) => {
                            error!("[convert] the item select page could not be installed ({reason}); its nodes stay hidden");
                        }
                    }
                }
                Err(reason) => {
                    error!("[convert] the item select page could not be composed ({reason}); its nodes stay hidden");
                }
            }
            info!(
                "[convert] {KEY}: slot page {}, item select page {}, {} slot cells",
                bindings.slot_page,
                bindings.item_select_page,
                slot_binding.cells.len()
            );
            screen.slot = Some(SlotCells::new(slot_binding.page, slot_binding.cells));
            screen.bindings = Some(Arc::new(bindings));
            screen.spawned = true;
            commands.spawn((
                ConvertScreenRoot,
                Visibility::Hidden,
                Transform::default(),
                RenderLayers::layer(SCREEN_UI_LAYER),
                view,
            ));
            if screen.item_select_bindings.is_some() {
                commands.spawn((
                    ConvertItemSelectRoot,
                    Visibility::Hidden,
                    Transform::default(),
                    RenderLayers::layer(SCREEN_UI_LAYER),
                    UiPrefabView::new(ITEM_SELECT_RUNTIME, SCREEN_UI_LAYER),
                ));
            }
        }
    }
}

/// The page's item icons as runtime textures: the client's icon atlas indexes
/// them by load path (`ItemIcons::texture`), and the layout binds them by that
/// same path. A load the index lacks is counted and named once.
fn register_icons(
    layouts: &mut UiLayouts,
    server: &AssetServer,
    icons: Option<&ItemIcons>,
    paths: &[String],
    seen: &mut BTreeSet<String>,
) {
    let Some(icons) = icons else {
        note_once(
            seen,
            "the item icon index is not on this root: the item select page draws no thumbnails".to_owned(),
        );
        return;
    };
    let mut missing = 0;
    for path in paths {
        if layouts.has_runtime_texture(path) {
            continue;
        }
        let Some(resource) = path.rsplit('/').next() else {
            missing += 1;
            continue;
        };
        match icons.texture(path, resource) {
            Ok(image) => {
                layouts.register_runtime_texture(path, &image, server);
            }
            Err(_) => missing += 1,
        }
    }
    // The item thumbnail nodes are raw images (no Image type), so they draw by
    // the plain sprite path and need no authored Sprite metrics.
    if missing > 0 {
        note_once(
            seen,
            format!(
                "the item icon index lacks {missing} item select thumbnail(s); those cells draw no image"
            ),
        );
    }
}

/// Update, after the screen manager: the layer's hooks.
#[allow(clippy::too_many_arguments)]
pub fn follow_screen(
    mut events: MessageReader<ScreenLayerEvent>,
    layouts: Res<UiLayouts>,
    tables: Res<moly_masters::load::MasterTables>,
    client: Option<Res<ClientUserData>>,
    copies: Option<Res<ClientConvertSlots>>,
    inventory: Option<Res<ClientMysekaiInventory>>,
    real: Res<Time<Real>>,
    mut sounds: ResMut<SeRequests>,
    mut screen: ResMut<ConvertScreen>,
) {
    let hooks: Vec<ScreenLayerEvent> = events
        .read()
        .filter(|event| event.screen == SCREEN)
        .copied()
        .collect();
    if hooks.is_empty() {
        return;
    }
    screen.prepare(&layouts);
    for event in hooks {
        match event.hook {
            ScreenHook::OnBoot => {
                if let Some(reason) = screen.absent.clone() {
                    error!(
                        "[convert] OnBoot on a root that refuses the screen ({reason}): ScreenBootDone at once and the push is popped"
                    );
                    screen.booted = true;
                    continue;
                }
                let Some(masters) = tables.convert.as_ref() else {
                    note_once(
                        &mut screen.seen,
                        "OnBoot: the convert master tables are not in; the boot waits".to_owned(),
                    );
                    continue;
                };
                let Some(user) =
                    read_user(client.as_deref(), copies.as_deref(), inventory.as_deref())
                else {
                    note_once(
                        &mut screen.seen,
                        "OnBoot: the user's convert slots are not in; the boot waits".to_owned(),
                    );
                    continue;
                };
                if screen.slot.is_none() {
                    note_once(
                        &mut screen.seen,
                        "OnBoot: the screen document is not loaded; the boot waits".to_owned(),
                    );
                    continue;
                }
                let now = now_ms(client.as_deref(), real.elapsed_secs());
                if let Some(slot) = screen.slot_mut() {
                    slot.setup(&user, masters, now, &layouts);
                }
                screen.booted = true;
                info!(
                    "[convert] OnBoot -> BootProcess: Presenter.Initialize -> SetupViewStateMachine(SlotStatusState, ItemSelectState); LoadAssetBundle(mysekai/ui/mysekai_convert_furniture) is named, not loaded (the bundle is not in the public asset root); ScreenBootDone"
                );
            }
            ScreenHook::OnInitComponent => {
                if !screen.booted {
                    continue;
                }
                screen.set_up = true;
                if let Some(slot) = screen.slot_mut() {
                    slot.show_slot_page();
                }
                info!("[convert] OnInitComponent: Presenter.Setup: HideAll(), ChangeView(Slot): ChangeTo(Slot), ClearOnBackUIScreenOverride()");
                sounds.0.push(SeRequest {
                    owner: None,
                    cue: ENTER_SE.to_owned(),
                    class: SeClass::Ui,
                    source: "convert",
                });
                note_once(
                    &mut screen.seen,
                    format!(
                        "SoundManager.PlaySEOneShot({ENTER_SE}); TutorialUtility.ShowTutorialSlideByType({TUTORIAL}) is not carried"
                    ),
                );
                screen_start_trading(&mut screen, &mut sounds, real.elapsed());
            }
            ScreenHook::OnWillExit => {
                info!("[convert] OnWillExit -> ScreenWillExitDone");
            }
            ScreenHook::OnExited => {
                screen_stop_trading(&mut screen);
                // `ItemSelectState.Dispose`.
                screen.item_select = None;
                screen.booted = false;
                screen.set_up = false;
                screen.on_item_select = false;
                screen.trading_next = Duration::ZERO;
                if let Some(slot) = screen.slot_mut() {
                    slot.dispose();
                }
                info!("[convert] OnExited -> Presenter.Dispose: the back override cleared, SlotStatusState.Dispose, ItemSelectState.Dispose, SlotView.Dispose (the effects' prefabs destroyed, the wave disposed), UnloadAssetBundle(mysekai/ui/mysekai_convert_furniture, true) (named, not loaded)");
            }
            _ => {}
        }
    }
}

/// Update, after the hooks and the screen manager: the refusal's pop, the
/// trading SE's loop, the root's visibility and the current page's frame.
#[allow(clippy::too_many_arguments)]
pub fn advance(
    real: Res<Time<Real>>,
    screens: Res<ScreenManager>,
    layouts: Res<UiLayouts>,
    tables: Res<moly_masters::load::MasterTables>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    client: Option<Res<ClientUserData>>,
    copies: Option<Res<ClientConvertSlots>>,
    inventory: Option<Res<ClientMysekaiInventory>>,
    art: Option<Res<BalloonArt>>,
    mut sounds: ResMut<SeRequests>,
    mut screen: ResMut<ConvertScreen>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut roots: Query<
        (&mut Visibility, &mut UiPrefabView),
        (With<ConvertScreenRoot>, Without<ConvertItemSelectRoot>),
    >,
    mut item_roots: Query<
        (&mut Visibility, &mut UiPrefabView),
        (With<ConvertItemSelectRoot>, Without<ConvertScreenRoot>),
    >,
) {
    let current = screens.current_screen() == Some(SCREEN);
    let playing = screens.current_state() == ScreenLayerState::Playing;
    // A refused screen answers its push with a pop once the slot is free.
    if current {
        if let Some(reason) = screen.absent.clone() {
            if !screen.refusal_popped && !screens.is_ui_layer_working() {
                error!(
                    "[convert] PushUIScreen(MenuScreenType::MysekaiConvertFurniture) refused: {reason}; the screen is popped"
                );
                layer_commands.write(LayerCommand::Pop);
                screen.refusal_popped = true;
            }
        }
    } else {
        screen.refusal_popped = false;
    }
    // The trading SE repeats while a slot processes.
    if screen.trading && real.elapsed() >= screen.trading_next {
        screen.trading_next = real.elapsed() + TRADING_PERIOD;
        sounds.0.push(SeRequest {
            owner: None,
            cue: TRADING_SE.to_owned(),
            class: SeClass::Ingame,
            source: "convert",
        });
    }
    // The roots are drawn while the layer is active and its start animation
    // done.
    let shown = current && screen.set_up && playing;
    let item_select_shown = shown && screen.view() == ViewType::ItemSelect;
    if let Ok((mut visibility, _)) = roots.single_mut() {
        visibility.set_if_neq(if shown && !item_select_shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    if let Ok((mut visibility, _)) = item_roots.single_mut() {
        visibility.set_if_neq(if item_select_shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    if !shown || screen.absent.is_some() {
        return;
    }
    let Some(bindings) = screen.bindings.clone() else {
        return;
    };
    let Ok((_, mut view)) = roots.single_mut() else {
        return;
    };
    // `HideAll`, then the current page's `Show`. The item select page is drawn
    // from its composed document (the second view), so the layer's own copy of
    // it stays hidden.
    view.set_visible(&bindings.slot_page, !item_select_shown);
    view.set_visible(&bindings.item_select_page, false);
    let Some(masters) = tables.convert.as_ref() else {
        return;
    };
    let Some(user) = read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()) else {
        return;
    };
    let now = now_ms(client.as_deref(), real.elapsed_secs());
    let ctx = ItemContext {
        masters,
        user: &user,
        now,
    };
    if item_select_shown {
        let Some(page) = screen.item_select_bindings.clone() else {
            return;
        };
        // `ItemSelectState.OnEnter` on the first frame the page shows: the
        // slot whose gauge asked for it, once.
        if screen.item_select.is_none() {
            let slot_id = screen
                .slot
                .as_mut()
                .and_then(SlotCells::take_item_select_slot)
                .unwrap_or(0);
            screen.open_item_select(slot_id, ctx);
        }
        let Ok((_, mut item_view)) = item_roots.single_mut() else {
            return;
        };
        let rows = Masters::new(&tables.rows);
        let canvas = item_canvas(&windows, root_canvas.as_deref());
        // The long press (`OnLongPressEvent`) and the frame, over the panel's
        // item (the list's selected one).
        let long = screen
            .item_select
            .as_mut()
            .and_then(|state| state.step(real.elapsed_secs() as f64));
        if let Some(ItemAction::Detail(item)) = long {
            // `ShowDetailDialog` -> `Show1ButtonDialog<
            // MysekaiConvertItemDetailDialog>.Setup(model)`.
            screen.detail_request = Some(item);
            info!("[convert] ItemSelectListCell.ShowDetailDialog({item})");
        }
        let ConvertScreen {
            item_select, ..
        } = &mut *screen;
        let Some(state) = item_select.as_mut() else {
            return;
        };
        state.draw(
            &mut item_view,
            &layouts,
            art.as_deref(),
            ctx,
            &rows,
            &page,
            canvas,
        );
        return;
    }
    let dt = real.delta_secs();
    let trading = if let Some(slot) = screen.slot_mut() {
        if !slot.is_shown() {
            return;
        }
        // `SlotStatusState.OnUpdate`: the slots' views, then the processing
        // flag's change (the animation and the trading SE).
        slot.update_views(&user, masters, now, &layouts);
        let processing = slot.update_processing();
        slot.step(dt);
        slot.draw(&mut view, &layouts, art.as_deref());
        processing
    } else {
        return;
    };
    if trading {
        screen_start_trading(&mut screen, &mut sounds, real.elapsed());
    } else {
        screen_stop_trading(&mut screen);
    }
}

fn screen_start_trading(screen: &mut ConvertScreen, sounds: &mut SeRequests, now: Duration) {
    if screen.trading {
        return;
    }
    screen.trading = true;
    screen.trading_next = now + TRADING_PERIOD;
    sounds.0.push(SeRequest {
        owner: None,
        cue: TRADING_SE.to_owned(),
        class: SeClass::Ingame,
        source: "convert",
    });
    info!("[convert] StartTradingSE: PlayEnvironmentSE({TRADING_SE}, 0, 1); the SE queue has no environment channel, so the cue is queued as an in-game one-shot (named)");
}

fn screen_stop_trading(screen: &mut ConvertScreen) {
    if !screen.trading {
        return;
    }
    screen.trading = false;
    info!("[convert] StopTradingSE: StopEnvironmentSE(0)");
}

/// Update: the back key (Escape, the desktop form of the header's back
/// button): the item select page returns to the slot page; the slot page
/// asks for `BackUIScreen` (`LayerCommand::Pop`).
pub fn back_key(
    keys: Res<ButtonInput<KeyCode>>,
    screens: Res<ScreenManager>,
    mut screen: ResMut<ConvertScreen>,
    mut layer_commands: MessageWriter<LayerCommand>,
) {
    if !keys.just_pressed(KeyCode::Escape)
        || screens.current_screen() != Some(SCREEN)
        || !screen.set_up
        || screens.is_ui_layer_working()
        || screens.exists_dialog()
    {
        return;
    }
    if screen.on_item_select {
        info!("[convert] back key: BackUIScreen -> OnBackUIScreenOverride -> ChangeView(ViewType.Slot)");
        screen.change_view(ViewType::Slot);
    } else {
        info!("[convert] back key: BackUIScreen -> PopUIScreen (no override on the slot page)");
        layer_commands.write(LayerCommand::Pop);
    }
}

/// Update: a press inside the convert screen takes the pointer; the slot
/// page's buttons and the item select page's tabs, cells, count and start
/// button answer it through the source's raycast over each page's view.
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut pointers: MessageReader<UiPointerEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    tables: Res<moly_masters::load::MasterTables>,
    // The user side a press reads: the client copy, the convert slots and the
    // holdings a consumption box is paid from.
    (client, copies, inventory): (
        Option<Res<ClientUserData>>,
        Option<Res<ClientConvertSlots>>,
        Option<Res<ClientMysekaiInventory>>,
    ),
    real: Res<Time<Real>>,
    (screens, dialog): (Res<ScreenManager>, Res<ShellDialogState>),
    root_canvas: Option<Res<RootCanvas>>,
    roots: Query<&UiPrefabView, (With<ConvertScreenRoot>, Without<ConvertItemSelectRoot>)>,
    item_roots: Query<&UiPrefabView, (With<ConvertItemSelectRoot>, Without<ConvertScreenRoot>)>,
    mut screen: ResMut<ConvertScreen>,
    mut press: ResMut<ConvertPress>,
    mut sounds: ResMut<SeRequests>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>,
) {
    let events: Vec<UiPointerEvent> = pointers.read().copied().collect();
    let scrolls: Vec<MouseWheel> = wheel.read().copied().collect();
    if screen.absent.is_some() {
        return;
    }
    if events.is_empty() && scrolls.is_empty() {
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let canvas = root_canvas.size(window);
    let blocked = screens.current_screen() != Some(SCREEN)
        || dialog.blocks_field_input()
        || screens.current_state() != ScreenLayerState::Playing;
    if screen.view() == ViewType::ItemSelect {
        input_item_select(
            &events,
            &scrolls,
            window,
            root_canvas,
            canvas,
            blocked,
            &layouts,
            &tables,
            client.as_deref(),
            copies.as_deref(),
            inventory.as_deref(),
            real.elapsed_secs(),
            &item_roots,
            &mut screen,
            &mut press,
            &mut sounds,
            &mut consumed,
        );
        return;
    }
    let Ok(view) = roots.single() else {
        return;
    };
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let screen = &mut *screen;
    let mut clicks: Vec<String> = Vec::new();
    for event in events {
        // The screen takes every pointer event while it is current (the full
        // screen layer is the pointer's owner), so the field's own tap
        // handlers do not also see a tap on it.
        consumed.0 = true;
        let point = event.position;
        let pointer = event_pointer(window, root_canvas, point);
        let target = view.press_target(&layouts, pointer, canvas);
        match event.phase {
            UiPointerPhase::Down => {
                press.held = Some((event.pointer_id, point));
                press.pressed = None;
                if blocked {
                    continue;
                }
                let Some((node, id)) = target else {
                    continue;
                };
                let path = doc.nodes[node].path.clone();
                if let Some((index, role)) = screen.slot().and_then(|slot| slot.live_button(&path)) {
                    press.pressed = Some((node, id));
                    info!("[convert] press: cell {index} ({role:?}) at {path}");
                }
            }
            UiPointerPhase::Move => {
                if let Some(held) = press
                    .held
                    .as_mut()
                    .filter(|(id, _)| *id == event.pointer_id)
                {
                    held.1 = point;
                }
            }
            UiPointerPhase::Up | UiPointerPhase::Cancel => {
                let pressed = press.pressed.take();
                press.held = None;
                let Some((node, id)) = pressed else {
                    continue;
                };
                if event.phase == UiPointerPhase::Cancel || blocked || target != Some((node, id)) {
                    continue;
                }
                clicks.push(doc.nodes[node].path.clone());
            }
        }
    }
    let now = now_ms(client.as_deref(), real.elapsed_secs());
    for path in clicks {
        // `CustomSelectableDefine.PlaySE` of the source button.
        layouts.queue_button_se(&mut sounds, KEY, &path);
        let action = screen.slot_mut().and_then(|slot| slot.click(&path));
        if let Some(action) = action {
            match action {
                SlotAction::ChangeView(view_type) => screen.change_view(view_type),
                SlotAction::UnlockBalloon => {
                    if let Some(slot) = screen.slot.as_mut() {
                        slot.show_unlock_slot_balloon(&layouts);
                    }
                }
                SlotAction::ShowShortenLockedBalloon => {
                    if let Some(slot) = screen.slot.as_mut() {
                        slot.show_shorten_locked_balloon(&layouts);
                    }
                }
                SlotAction::Shorten(slot_id) => {
                    // `OnShortenSlot(slot)`.
                    if let (Some(masters), Some(user)) = (
                        tables.convert.as_ref(),
                        read_user(client.as_deref(), copies.as_deref(), inventory.as_deref()),
                    ) {
                        let ctx = ItemContext {
                            masters,
                            user: &user,
                            now,
                        };
                        super::start::on_shorten_slot(screen, ctx, slot_id);
                    }
                }
                SlotAction::Receive(slot_id) => {
                    // `ConvertReceiveProcessAsync(slot)`: the receive chain
                    // takes it on its next step.
                    if screen.receive_intent.replace(slot_id).is_some() {
                        note_once(
                            &mut screen.seen,
                            format!("ConvertReceiveProcessAsync(slot {slot_id}) while another receive waits to start: the later one is kept"),
                        );
                    }
                }
            }
        }
    }
}

/// The item select page's pointer and wheel.
#[allow(clippy::too_many_arguments)]
fn input_item_select(
    events: &[UiPointerEvent],
    scrolls: &[MouseWheel],
    window: &Window,
    root_canvas: &RootCanvas,
    canvas: Vec2,
    blocked: bool,
    layouts: &UiLayouts,
    tables: &moly_masters::load::MasterTables,
    client: Option<&ClientUserData>,
    copies: Option<&ClientConvertSlots>,
    inventory: Option<&ClientMysekaiInventory>,
    realtime: f32,
    item_roots: &Query<&UiPrefabView, (With<ConvertItemSelectRoot>, Without<ConvertScreenRoot>)>,
    screen: &mut ConvertScreen,
    press: &mut ConvertPress,
    sounds: &mut SeRequests,
    consumed: &mut crate::action_button::ActionTapConsumed,
) {
    let Ok(view) = item_roots.single() else {
        return;
    };
    let Some(page) = screen.item_select_bindings.clone() else {
        return;
    };
    let Some(doc) = layouts.document(ITEM_SELECT_RUNTIME) else {
        return;
    };
    let Some(masters) = tables.convert.as_ref() else {
        return;
    };
    let Some(user) = read_user(client, copies, inventory) else {
        return;
    };
    let now = now_ms(client, realtime);
    let ctx = ItemContext {
        masters,
        user: &user,
        now,
    };
    if screen.item_select.is_none() {
        return;
    }
    if blocked {
        return;
    }
    let scale = root_canvas.scale(window);
    let inside = |point: Vec2| {
        view.rect(layouts, &page.viewport, canvas)
            .is_some_and(|rect| rect.contains(point))
    };
    // The controls: the detail panel first (it sits above the list), then the
    // page's tabs and cells. A press keeps the node it started on, so a
    // release only acts when it meets the same control again. The page's
    // state is held out of the screen for one event and put back before the
    // event's own call into the screen (the start confirm, the detail line).
    for event in events {
        consumed.0 = true;
        let Some(mut page_state) = screen.item_select.take() else {
            return;
        };
        let state = &mut page_state;
        // The calls into the screen this event makes once its state is back.
        let mut start_confirm = false;
        let mut detail_dialog = None;
        let point = to_canvas(event.position, window, root_canvas);
        let pointer = event_pointer(window, root_canvas, event.position);
        let path = view
            .press_target(layouts, pointer, canvas)
            .and_then(|(node, _)| doc.nodes.get(node))
            .map(|node| node.path.clone());
        let detail = path
            .as_deref()
            .and_then(|path| state.detail.hit(&page.detail, path));
        let hit = match detail {
            Some(_) => None,
            None => path.as_deref().and_then(|path| state.hit(&page, path)),
        };
        match event.phase {
            UiPointerPhase::Down => {
                press.held = Some((event.pointer_id, point));
                press.item_path = path.clone();
                if detail.is_none() {
                    state.pressed(event.pointer_id, point, hit, inside(point), realtime as f64);
                }
            }
            UiPointerPhase::Move => {
                if let Some(current) = press
                    .held
                    .as_mut()
                    .filter(|(id, _)| *id == event.pointer_id)
                {
                    current.1 = point;
                }
                if detail.is_none() {
                    state.moved(event.pointer_id, point, hit);
                }
            }
            UiPointerPhase::Up | UiPointerPhase::Cancel => {
                press.held = None;
                let started = press.item_path.take();
                let same = started.is_some() && started == path;
                if !same || event.phase == UiPointerPhase::Cancel {
                    // The pointer left the control (or was lost): the page's
                    // press and drag end without a click.
                    state.released(event.pointer_id, None);
                } else if let Some(action) = detail {
                    let Some(path) = path.as_deref() else {
                        screen.item_select = Some(page_state);
                        continue;
                    };
                    layouts.queue_button_se(sounds, ITEM_SELECT_RUNTIME, path);
                    match action {
                        super::detail::Action::Decrement => {
                            let count = state.detail.count() - 1;
                            state.detail.set_count(ctx, count);
                        }
                        super::detail::Action::Increment => {
                            let count = state.detail.count() + 1;
                            state.detail.set_count(ctx, count);
                        }
                        // `UIPartsCommonButton`'s `InvokeConvertStart` ->
                        // `ItemSelectView.OnStartConvert` ->
                        // `ItemSelectState.OnConvertStart` ->
                        // `Presenter.ConfirmConvertStart(_convertSlotId,
                        // model)`.
                        super::detail::Action::Start => start_confirm = true,
                        super::detail::Action::LockedBalloon => {
                            info!(
                                "[convert] ItemSelectDetailView.ShowLockedBalloon: {}",
                                layouts.wording(super::detail::LOCKED_KEY)
                            );
                        }
                    }
                } else {
                    match state.released(event.pointer_id, hit) {
                        Some(ItemAction::SelectTab(_)) => {
                            if let Some(path) = path.as_deref() {
                                layouts.queue_button_se(sounds, ITEM_SELECT_RUNTIME, path);
                            }
                            state.refresh(ctx);
                        }
                        Some(ItemAction::SelectItem(_)) => {
                            if let Some(path) = path.as_deref() {
                                layouts.queue_button_se(sounds, ITEM_SELECT_RUNTIME, path);
                            }
                        }
                        // `ShowDetailDialog` -> `Show1ButtonDialog<
                        // MysekaiConvertItemDetailDialog>.Setup(model)`.
                        Some(ItemAction::Detail(item)) => detail_dialog = Some(item),
                        None => {}
                    }
                }
            }
        }
        // The page's state goes back before the screen's own calls.
        screen.item_select = Some(page_state);
        if start_confirm {
            super::start::confirm_convert_start(screen, ctx);
        }
        if let Some(item) = detail_dialog {
            // `ShowDetailDialog` -> the item detail dialog's request.
            screen.detail_request = Some(item);
            info!("[convert] ItemSelectListCell.ShowDetailDialog({item})");
        }
    }
    // The wheel over the list, on the page's state as the taps left it.
    if let Some(cursor) = window.cursor_position() {
        let point = to_canvas(cursor, window, root_canvas);
        let inside = inside(point);
        let Some(mut page_state) = screen.item_select.take() else {
            return;
        };
        for event in scrolls.iter() {
            let lines = match event.unit {
                MouseScrollUnit::Line => event.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            page_state.wheel(inside, lines);
        }
        screen.item_select = Some(page_state);
    }
}
