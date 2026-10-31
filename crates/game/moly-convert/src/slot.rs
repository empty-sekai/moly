//! The convert screen's slot page (`MysekaiConvertFurnitureSlotView`) and
//! its state (`MysekaiConvertFurnitureSlotStatusState`): one cell per machine
//! slot, its gauge, receive look, countdown and skip button.
//!
//! Input: the page's nodes and cells read from the screen document
//! ([`bind_page`]); the user's convert state, the convert masters and the
//! frame's clock on every read ([`SlotCells::setup`] once, then
//! [`SlotCells::update_views`]); the request a press makes, answered by
//! [`SlotCells::click`].
//!
//! Output: each cell's content, texts and tweens drawn into the page's view
//! ([`SlotCells::draw`]); the lock balloon of a slot the pass would extend
//! ([`SlotCells::show_unlock_slot_balloon`]) and the skip's lock balloon of a
//! cell ([`SlotCells::show_shorten_locked_balloon`]); the action a press asks
//! the presenter for ([`SlotAction`]): a page change, a balloon, or a skip
//! request for the screen to hold.
//!
//! Invariants: the layout carries three cells and the machine three slots, so
//! one cell is one slot and nothing is reordered. A cell is drawn while its
//! CanvasGroup shows; the page's `Show` / `Hide` alpha is not what the layout
//! draws, so the cell's group keeps its serialized state. Only a press on a
//! live button is taken ([`SlotCells::live_button`]): a processing cell whose
//! converts are all received has nothing to receive, so its button view's
//! press is not taken at all (the source's `_selectedButton.enabled`).
//!
//! Boundaries (each a named line, nothing played or pushed in its place):
//! - The animator `fx_mysekai_convert_furniture` and the three effects'
//!   prefabs are not in the public asset root: neither is loaded, so the
//!   processing animation is not played and the effects are not drawn. The
//!   screen's trading sound plays while a slot processes.
//! - The title icon (the system fixture type's `convert` icon,
//!   `SystemFixtureIcon.Setup(convert)`) and the base images are the
//!   package's textures, which the screen document does not name.
//! - The crystal shop screen (`MenuScreenType.CrystalShop`) that the lock
//!   balloon's transition button pushes is not carried: the press is named
//!   and nothing is pushed.
//! - The receive of a finished slot (`ConvertReceiveProcessAsync`) is not
//!   built: its press is named and nothing is sent.
//!
//! Every call into code that is not built stands as one named line
//! (`"convert: <what> is not built yet"`) and does nothing else.

mod cell;

use std::collections::BTreeSet;

use bevy::prelude::*;
use moly_assets::ui_layout::UiPrefab;
use moly_masters::convert::ConvertMasters;
use moly_ui::ui_inputs::BalloonArt;
use moly_ui::ui_layout::{UiLayouts, UiPrefabView};

use super::model::{ConvertUserState, ItemContext, Machine, SlotModel, SlotState};
use super::screen::{not_built, note_once, ViewType};

// The cells' texts draw through the screen's glyph check.
pub use super::screen::put_text;

// The detail panel and the dialogs read the cell's texts; the item select
// page's cells reuse the node layout of `CellNodes::of` under their own paths.
pub use cell::{format as format_wording, to_time_string, Cell, CellNodes, MACHINE_LEVEL, SLOT_NO_FORMAT};

/// The slot page's class and the cell's.
const PAGE_CLASS: &str = "MysekaiConvertFurnitureSlotView";
const CELL_CLASS: &str = "MysekaiConvertFurnitureSlotCellView";
/// The release-condition balloon's class.
const BALLOON_CLASS: &str = "UIPartsReleaseConditionsBalloon";
/// `SlotView.ShowUnlockSlotBalloon`'s line.
pub const RELEASE_BALLOON: &str = "MSG_RELEASING_AT_MYSEKAI_COLORFUL_PASS_PURCHASE";
/// The slot page's animation prefab (not in the public asset root).
const ANIMATION_PREFAB: &str = "fx_mysekai_convert_furniture";
/// `SystemFixtureIcon.Setup`: the fixture type whose icon the title shows.
const TITLE_FIXTURE: &str = "convert";

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

/// The slot page's serialized nodes.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct PageBindings {
    /// `ComponentRoot/MysekaiConvertFurnitureSlotView`.
    pub view: String,
    /// The `UIPartsReleaseConditionsBalloon` under the page's slots.
    pub balloon: String,
    pub balloon_text: String,
    pub balloon_base_right_image: String,
    pub balloon_transition: String,
}


/// The slot page's nodes and cells.
#[derive(Debug, Clone)]
pub struct SlotBinding {
    pub page: PageBindings,
    pub cells: Vec<CellNodes>,
}

/// The page's nodes and cells; refuses a document without the page.
pub fn bind_page(doc: &UiPrefab) -> Result<SlotBinding, String> {
    let anchor = doc
        .nodes
        .iter()
        .position(|node| {
            node.components
                .iter()
                .any(|component| component.class.ends_with(&format!(".{PAGE_CLASS}")))
        })
        .ok_or_else(|| format!("no {PAGE_CLASS} in the screen"))?;
    let view = doc.nodes[anchor].path.clone();
    let slots = format!("{view}/Slots");
    let mut cells = Vec::new();
    for node in &doc.nodes {
        if !node
            .components
            .iter()
            .any(|component| component.class.ends_with(&format!(".{CELL_CLASS}")))
        {
            continue;
        }
        let mut nodes = CellNodes::of(&node.path);
        nodes.canvas_group = node
            .components
            .iter()
            .any(|component| component.class == "UnityEngine.CanvasGroup")
            .then(|| node.path.clone());
        cells.push(nodes);
    }
    if cells.is_empty() {
        return Err(format!("the page has no {CELL_CLASS} cells"));
    }
    let balloon = doc
        .nodes
        .iter()
        .find(|node| {
            node.path.starts_with(&format!("{slots}/"))
                && node
                    .components
                    .iter()
                    .any(|component| component.class.ends_with(&format!(".{BALLOON_CLASS}")))
        })
        .map_or_else(
            || format!("{slots}/UIPartsReleaseConditionsBalloon"),
            |node| node.path.clone(),
        );
    let page = PageBindings {
        balloon_text: format!("{balloon}/Content/CustomText"),
        balloon_base_right_image: format!("{balloon}/Content/Base/Right/Image"),
        balloon_transition: format!("{balloon}/Content/TransitionButton"),
        balloon,
        view,
    };
    Ok(SlotBinding { page, cells })
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// What a click on the page asks of the presenter (`Presenter.OnSelectedSlot`
/// and `OnShortenSlot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotAction {
    /// `Presenter.ChangeView`.
    ChangeView(ViewType),
    /// `SlotView.ShowUnlockSlotBalloon` (and, under it, `ShowCrystalShop`).
    UnlockBalloon,
    /// `ShowShortenLockedBalloon`.
    ShowShortenLockedBalloon,
    /// `OnShortenSlot(slot)` of the cell's slot (`Presenter.OnShortenSlot`).
    Shorten(i32),
}

/// The page's buttons, as the source's fields name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    /// `Root/GaugeView/Button` (locked and ready cells; `OnSelectedSlot`).
    Gauge,
    /// `Root/ButtonView/Button` (a processing cell with something to
    /// receive, a complete cell; `ConvertReceiveProcessAsync`).
    View,
    /// `Root/CountDown/ShortenSlotButton` (processing, skip released;
    /// `OnShortenSlot`).
    Shorten,
    /// `Root/CountDown/ShortenSlotButton/Cover` (processing, skip locked;
    /// `ShowShortenLockedBalloon`).
    ShortenCover,
    /// The lock balloon's transition button (`ShowCrystalShop`).
    BalloonTransition,
}

/// `MysekaiConvertFurnitureSlotView` with its cells.
#[derive(Debug, Default)]
pub struct SlotCells {
    page: PageBindings,
    cells: Vec<Cell>,
    /// `_needsUpdateSlotDictionary`: the slots re-read this frame.
    needs_update: BTreeSet<usize>,
    /// `_isProcessingBefore`.
    processing_before: bool,
    /// The page is shown (the state's `OnEnter` ran, not its `OnExit`).
    page_shown: bool,
    /// `ShowUnlockSlotBalloon` ran.
    balloon_shown: bool,
    /// The lock balloon's line while it shows.
    balloon_line: Option<String>,
    /// `ShowShortenLockedBalloon` ran.
    shorten_balloon_shown: bool,
    /// The slot whose press asked for `ChangeView(ViewType.ItemSelect)`
    /// (`ItemSelectState.Setup(slotId)`'s id).
    item_select_slot: Option<i32>,
    seen: BTreeSet<String>,
}

impl SlotCells {
    pub fn new(page: PageBindings, nodes: Vec<CellNodes>) -> Self {
        let cells = nodes
            .into_iter()
            .enumerate()
            .map(|(index, nodes)| Cell::new(nodes, index as i32))
            .collect();
        Self {
            page,
            cells,
            ..Self::default()
        }
    }

    /// The page is shown.
    pub fn is_shown(&self) -> bool {
        self.page_shown
    }

    /// `SlotStatusState.OnEnter`: each cell built for its slot at the clock
    /// (`SlotView.Setup`), the page shown, every slot marked for the next
    /// read, the processing flag read.
    pub fn setup(
        &mut self,
        user: &ConvertUserState,
        masters: &ConvertMasters,
        now: i64,
        layouts: &UiLayouts,
    ) {
        let ctx = ItemContext {
            masters,
            user,
            now,
        };
        for cell in &mut self.cells {
            let slot_id = cell.slot_id;
            let model = SlotModel::new(ctx, slot_id, ctx.machine().is_pass_slot(slot_id));
            cell.setup(&model, ctx, layouts, &mut self.seen);
        }
        self.needs_update = (0..self.cells.len()).collect();
        self.page_shown = true;
        self.balloon_shown = false;
        self.balloon_line = None;
        self.shorten_balloon_shown = false;
        self.processing_before = self.any_processing();
        info!(
            "[convert] SlotStatusState.OnEnter: SlotView.Setup({} cells), Show(), every slot marked for update, IsProcessing {}",
            self.cells.len(),
            self.processing_before
        );
        if self.processing_before {
            note_once(
                &mut self.seen,
                format!(
                    "{ANIMATION_PREFAB} is not in the public asset root: the slot animation not played; StartTradingSE sounds while a slot processes"
                ),
            );
        }
        note_once(
            &mut self.seen,
            format!(
                "SlotView.Setup binds the base images and the title icon (SystemFixtureIcon.Setup({TITLE_FIXTURE})) from the package's textures, which the screen document does not name"
            ),
        );
        // The cell CanvasGroups: `Show` / `Hide` set their alpha; the layout
        // draws the serialized state, so the cells are drawn with the page.
        let opaque: Vec<&str> = self
            .cells
            .iter()
            .filter_map(|cell| cell.nodes.canvas_group.as_deref())
            .filter(|group| group.contains("Slots/"))
            .collect();
        if !opaque.is_empty() {
            note_once(
                &mut self.seen,
                format!(
                    "{} cell CanvasGroup(s) set alpha 1 by Show/Hide; the layout draws their serialized value",
                    opaque.len()
                ),
            );
        }
    }

    /// `SlotView.Show()`: the page's CanvasGroup alpha 1, interactable and
    /// raycast-blocking, and each cell's `Show()` does the same for its own
    /// group (drawn as the page's visibility here, named).
    pub fn show_slot_page(&mut self) {
        self.page_shown = true;
    }

    /// `SlotView.Hide()`: `HideAll` and each cell's `Hide()`, leaving the
    /// page's `OnExit`.
    pub fn hide_slot_page(&mut self) {
        self.page_shown = false;
        self.balloon_shown = false;
        self.balloon_line = None;
        self.shorten_balloon_shown = false;
    }

    /// `SlotView.Dispose`: each cell's wave tween and effects dropped; the
    /// page is hidden for the next boot.
    pub fn dispose(&mut self) {
        self.hide_slot_page();
        self.needs_update.clear();
        self.processing_before = false;
        for cell in &mut self.cells {
            cell.dispose();
        }
    }

    /// `ItemSelectState.Setup(slotId)` and `ChangeView(ViewType.ItemSelect)`:
    /// the page takes the slot whose gauge was pressed (the presenter reads
    /// it with [`Self::take_item_select_slot`]).
    pub fn item_select(&mut self, slot_id: i32) {
        self.item_select_slot = Some(slot_id);
        self.page_shown = false;
    }

    /// The slot the item select page opens on, once.
    pub fn take_item_select_slot(&mut self) -> Option<i32> {
        self.item_select_slot.take()
    }

    /// `UpdateSlotView` of every marked slot.
    pub fn update_views(
        &mut self,
        user: &ConvertUserState,
        masters: &ConvertMasters,
        now: i64,
        layouts: &UiLayouts,
    ) {
        let machine = Machine::new(masters);
        let ctx = ItemContext {
            masters,
            user,
            now,
        };
        let cells = std::mem::take(&mut self.cells);
        let needs = std::mem::take(&mut self.needs_update);
        let mut out = Vec::with_capacity(cells.len());
        for (index, mut cell) in cells.into_iter().enumerate() {
            let slot_id = cell.slot_id;
            if needs.contains(&index) {
                let model = SlotModel::new(ctx, slot_id, machine.is_pass_slot(slot_id));
                cell.update_slot_view(&model, ctx, layouts, &mut self.seen);
            }
            out.push(cell);
        }
        self.cells = out;
    }

    /// `SlotView.PlayConvertStartAsync(slotId)` ->
    /// `SlotCellView.PlayConvertStartAsync()`: the cell of that slot starts
    /// (the timer and button views once, the gauge's wave at the state read
    /// again).
    pub fn play_convert_start(
        &mut self,
        model: &SlotModel<'_>,
        ctx: ItemContext<'_>,
        layouts: &UiLayouts,
    ) {
        let slot_id = model.slot_id();
        let seen = &mut self.seen;
        if let Some(cell) = self.cells.iter_mut().find(|cell| cell.slot_id == slot_id) {
            cell.play_convert_start_timer(model);
            cell.play_convert_start_button();
            // `Gauge.PlayConvertStartAsync(model)`: `slotModel.SetupSlotState()`
            // and the content of its state again, the wave hidden for its wait
            // (no cell tween runs here: the next read shows the slot's own
            // progress).
            cell.play_convert_start(model, ctx, layouts, seen);
        }
    }

    /// `SlotView.PlayShortenAsync(slotId, currentProgress)` ->
    /// `SlotCellView.PlayShortenAsync(currentProgress)` ->
    /// `Gauge.PlayShortenAsync(currentProgress)`: the cell of that slot tweens
    /// its wave to 1.
    pub fn play_shorten(&mut self, slot_id: i32, progress: f32) {
        if let Some(cell) = self.cells.iter_mut().find(|cell| cell.slot_id == slot_id) {
            cell.play_shorten(progress);
        }
    }

    /// `SlotStatusState.OnUpdate`'s processing flag; answers the new flag.
    pub fn update_processing(&mut self) -> bool {
        let now = self.any_processing();
        if now != self.processing_before {
            if now {
                info!("[convert] IsProcessing: PlayAnimation (named: the animation prefab is not in the public asset root)");
            } else {
                info!("[convert] IsProcessing ends: PauseAnimation (named)");
            }
        }
        self.processing_before = now;
        now
    }

    /// `IsProcessing`: any slot processing or complete.
    pub fn any_processing(&self) -> bool {
        self.cells
            .iter()
            .any(|cell| cell.view.state.is_some_and(SlotState::is_processing))
    }

    /// One frame of the cells' own tweens.
    pub fn step(&mut self, dt: f32) {
        for cell in &mut self.cells {
            cell.step(dt);
        }
    }

    /// The cell a node belongs to.
    pub fn cell_of(&self, path: &str) -> Option<usize> {
        (0..self.cells.len()).find(|index| {
            let root = &self.cells[*index].nodes.root;
            path == root || path.starts_with(&format!("{root}/"))
        })
    }

    /// The button role of a node path, when that button can take a press
    /// (`Button.enabled` and the owning object active).
    pub fn live_button(&self, path: &str) -> Option<(usize, ButtonRole)> {
        if !self.page_shown {
            return None;
        }
        if self.balloon_shown
            && (path == self.page.balloon_transition
                || path.starts_with(&format!("{}/", self.page.balloon_transition)))
        {
            return Some((usize::MAX, ButtonRole::BalloonTransition));
        }
        let index = self.cell_of(path)?;
        let cell = &self.cells[index];
        let nodes = &cell.nodes;
        let inside = |button: &str| path == button || path.starts_with(&format!("{button}/"));
        if inside(&nodes.button) {
            return cell.view.gauge_button.then_some((index, ButtonRole::Gauge));
        }
        if inside(&nodes.view_button) {
            return cell.view.view_button_enabled.then_some((index, ButtonRole::View));
        }
        if inside(&nodes.shorten_cover) {
            return (cell.view.shorten_visible && cell.view.cover_listens)
                .then_some((index, ButtonRole::ShortenCover));
        }
        if inside(&nodes.shorten) {
            return (cell.view.shorten_visible && cell.view.shorten_enabled)
                .then_some((index, ButtonRole::Shorten));
        }
        None
    }

    /// The click on a live button: the presenter's answer.
    pub fn click(&mut self, path: &str) -> Option<SlotAction> {
        let (index, role) = self.live_button(path)?;
        self.shorten_balloon_shown = false;
        match role {
            ButtonRole::BalloonTransition => {
                // `ShowCrystalShop`: the crystal shop opens on the colourful
                // pass page; the screen is not carried.
                not_built(
                    &mut self.seen,
                    "the crystal shop screen (it opens on the colourful pass page)",
                );
                self.balloon_shown = false;
                None
            }
            ButtonRole::Gauge => self.on_selected_slot(index),
            // `ConvertReceiveProcessAsync(slot)` beside the gauge's press.
            ButtonRole::View => {
                not_built(&mut self.seen, "the receive process");
                None
            }
            ButtonRole::ShortenCover => {
                if let Some(cell) = self.cells.get_mut(index) {
                    cell.show_shorten_locked_balloon();
                }
                Some(SlotAction::ShowShortenLockedBalloon)
            }
            ButtonRole::Shorten => {
                // `OnShortenSlot(slot)`.
                Some(SlotAction::Shorten(self.cells[index].slot_id))
            }
        }
    }

    /// `Presenter.OnSelectedSlot(slot)`.
    fn on_selected_slot(&mut self, index: usize) -> Option<SlotAction> {
        let state = self.cells.get(index)?.view.state?;
        match state {
            SlotState::Locked => {
                self.shorten_balloon_shown = false;
                Some(SlotAction::UnlockBalloon)
            }
            SlotState::Ready => {
                // `ItemSelectState.Setup(slotId)` and
                // `ChangeView(ViewType.ItemSelect)`.
                let slot_id = self.cells[index].slot_id;
                self.item_select(slot_id);
                Some(SlotAction::ChangeView(ViewType::ItemSelect))
            }
            // `ConvertReceiveProcessAsync(slot)`: the receivable count, the
            // receive request, the wave and the reward window are not built.
            SlotState::Processing | SlotState::Complete => {
                not_built(&mut self.seen, "the receive process");
                None
            }
        }
    }

    /// `SlotView.ShowUnlockSlotBalloon`: the page's release-conditions
    /// balloon takes `MSG_RELEASING_AT_MYSEKAI_COLORFUL_PASS_PURCHASE` and
    /// shows over the locked cells.
    pub fn show_unlock_slot_balloon(&mut self, layouts: &UiLayouts) {
        let line = layouts.wording(RELEASE_BALLOON);
        self.balloon_shown = true;
        self.balloon_line = Some(line);
        self.shorten_balloon_shown = false;
        info!(
            "[convert] SlotView.ShowUnlockSlotBalloon: {} shows {RELEASE_BALLOON}, centred over the locked cells",
            self.page.balloon
        );
    }

    /// `MysekaiConvertFixtureSlotCellTimerView.ShowShortenLockedBalloon`: the
    /// cell's `UIPartsCommonBalloon` shows `WORD_CONVERT_MACHINE_LEVEL` with
    /// the skip's released level.
    pub fn show_shorten_locked_balloon(&mut self, layouts: &UiLayouts) {
        self.shorten_balloon_shown = true;
        info!(
            "[convert] ShowShortenLockedBalloon: {}",
            layouts.wording(MACHINE_LEVEL)
        );
    }

    /// One frame of the page: the cells' content, texts and tweens.
    pub fn draw(&mut self, view: &mut UiPrefabView, layouts: &UiLayouts, art: Option<&BalloonArt>) {
        if !self.page_shown {
            view.set_visible(&self.page.view, false);
            return;
        }
        view.set_visible(&self.page.view, true);
        for cell in &self.cells {
            cell.draw(view, layouts, art, &mut self.seen);
        }
        // The lock balloon: shown after a locked cell's press.
        let line = self.balloon_line.clone();
        view.set_visible(&self.page.balloon, line.is_some());
        if let Some(line) = line {
            let shows = put_text(view, &self.page.balloon_text, Some(&line), art, &mut self.seen);
            // `UIPartsReleaseConditionsBalloon`'s base shows its right image
            // when the text is wider than the base's left part, or while the
            // transition button shows; the layout's horizontal group decides
            // that here, so the image follows the text being drawn.
            view.set_visible(&self.page.balloon_base_right_image, shows);
            view.set_visible(&self.page.balloon_transition, shows);
        }
        // The skip's lock balloon (`UIPartsCommonBalloon`) is each cell's own,
        // drawn with the cell: its cover's press opens it.
        let _ = self.shorten_balloon_shown;
    }
}
