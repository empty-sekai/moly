//! One slot cell of the convert screen (`MysekaiConvertFurnitureSlot`): its
//! three views (`Root/GaugeView`, `Root/ButtonView`, `Root/CountDown`), the
//! four states their content follows, and the water-level tween it runs.
//!
//! Source structure:
//! - `MysekaiConvertFurnitureSlotCellView.Setup(model, onSelected, onShorten)`
//!   records the model and its state, then sets up the gauge, the button and
//!   the timer views. `UpdateSlotView()` refreshes the numbers when the state
//!   did not change (`RefreshView`) and rebuilds all three views when it did
//!   (`SetupView`).
//! - `MysekaiConvertFixtureSlotCellGaugeView`: `SetupView(state)` turns on one
//!   content per state (locked: `Lock`; ready: `ReadyContent`; processing:
//!   `ProcessingContent` and its `ReceiveCountText`; complete:
//!   `CompleteMessage`), hides the `Base/WaveImage` for locked and ready and
//!   shows it at `NormalizedProgressTime` for processing and at 1 for
//!   complete, and turns on the processing or the complete highlight root;
//!   the cell's `Button` takes a click for locked and ready slots only.
//!   `RefreshInProcessing` / `RefreshInComplete` write
//!   `WORD_CONVERT_PROGRESS_FORMAT` with the completed and the remaining
//!   converts and set the wave's progress. `SetupEffect` instantiates effect
//!   prefabs from a bundle that is not part of the public asset root (named,
//!   not loaded). `PlayConvertStartAsync` (the wave from 0 over the start's
//!   seconds, halving its time), `PlayReceiveAsync(from, to)` and
//!   `PlayShortenAsync(progress)` tween the wave and keep its tween while it
//!   runs.
//! - `MysekaiConvertFixtureSlotCellButtonView`: `SetupView` shows
//!   `Off`/`On`/`UnReceive`/`Receiveable` by state and `IsReceivable`, and
//!   enables its `Button` for a processing slot that has something to receive
//!   and for a complete one; `PlayConvertStart` shows the processing,
//!   nothing-to-receive look at once; `Refresh` follows a processing slot.
//! - `MysekaiConvertFixtureSlotCellTimerView`: `SetupView` sets the count
//!   down background's alpha to 0.1 with `WORD_THREE_HYPHEN` and hides the
//!   skip button for every state but processing, which shows the remaining
//!   time (`TimeSpan.ToTimeString`) and the skip button.
//!   `SetupShortenSlotButton` enables the skip button by `IsReleasedShorten`
//!   and shows `WORD_CONVERT_MACHINE_LEVEL` with the release level on the
//!   button's cover while the skip is locked.
//!
//! The numbers are read through the slot model at the frame's clock (its
//! methods, not the module's free helpers), and each cell keeps the values it
//! last set so a refresh of its numbers (`SlotChange::Numbers`) is told from
//! a rebuild of its views (`SlotChange::State`).

use std::collections::BTreeSet;

use bevy::prelude::*;

use super::super::model::{ItemContext, SlotChange, SlotModel, SlotState};
use super::put_text;
use crate::ui_inputs::BalloonArt;
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// `WORD_CONVERT_PROGRESS_FORMAT`: the completed and the remaining converts.
pub const PROGRESS_FORMAT: &str = "WORD_CONVERT_PROGRESS_FORMAT";
/// `FORMAT_MYSEKAI_CONVERT_SLOT_NO`: the slot number (slot id from 0).
pub const SLOT_NO_FORMAT: &str = "FORMAT_MYSEKAI_CONVERT_SLOT_NO";
/// `WORD_THREE_HYPHEN`: the timer of a slot that is not processing.
pub const THREE_HYPHEN: &str = "WORD_THREE_HYPHEN";
/// `WORD_CONVERT_MACHINE_LEVEL`: the line of the skip's lock balloon.
pub const MACHINE_LEVEL: &str = "WORD_CONVERT_MACHINE_LEVEL";
/// The count down background's alpha for a slot that is not processing.
pub const TIMER_IDLE_ALPHA: f32 = 0.1;
/// `_receiveAnimationSeconds`: the receive and the shorten wave tween's
/// duration. Its value lives in the gauge's serialized prefab data, which is
/// not read here (unverified): the constant is a stand-in and no test pins it.
pub const RECEIVE_SECONDS: f32 = 0.3;
/// The three effects `SetupEffect` instantiates in the gauge's roots. The
/// bundle they load from is not part of the public asset root: named, not
/// loaded.
pub const EFFECT_PREFABS: [&str; 3] = [
    "fx_mysekai_convert_furniture_wait",
    "fx_mysekai_convert_furniture_highlight_light",
    "fx_mysekai_convert_furniture_highlight",
];

/// The node paths of one cell, below
/// `.../MysekaiConvertFurnitureSlotView/Slots/MysekaiConvertFurnitureSlot`.
#[derive(Debug, Clone, Default)]
pub struct CellNodes {
    /// The cell itself (`MysekaiConvertFurnitureSlotCellView`).
    pub root: String,
    /// The cell's CanvasGroup path, when it has one (`Show` / `Hide`).
    pub canvas_group: Option<String>,
    // Gauge view.
    pub gauge: String,
    pub shadow: String,
    pub base: String,
    pub wave: String,
    pub processing_highlight: String,
    pub complete_highlight: String,
    pub blah: String,
    pub highlights: String,
    pub ready_content: String,
    pub empty_icon: String,
    pub select_item: String,
    pub setting_message: String,
    pub processing_content: String,
    pub processing_message: String,
    pub complete_message: String,
    pub slot_number: String,
    pub cover_interaction: String,
    pub frame: String,
    /// The gauge's own `Button` (locked and ready cells listen on it).
    pub button: String,
    pub lock: String,
    pub lock_icon: String,
    pub lock_text: String,
    // Button view.
    pub button_view: String,
    pub off: String,
    pub on: String,
    pub un_receive: String,
    pub receivable: String,
    pub interact: String,
    pub effect: String,
    /// `Root/ButtonView/Button`.
    pub view_button: String,
    // Count down.
    pub count_down: String,
    pub time_icon: String,
    pub timer_text: String,
    pub shorten: String,
    pub shorten_icon: String,
    pub shorten_cover: String,
    pub lock_balloon: String,
    pub lock_balloon_text: String,
}

fn sub(root: &str, below: &str) -> String {
    if below.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{below}")
    }
}

impl CellNodes {
    /// The nodes of one cell, given the cell's own path.
    pub fn of(root: &str) -> Self {
        let gauge = sub(root, "Root/GaugeView");
        let button_view = sub(root, "Root/ButtonView");
        let count_down = sub(root, "Root/CountDown");
        // `_shortenSlotLockedButton` (the comment in the source's fields
        // names it) is the skip button's `Cover` object.
        let shorten = sub(&count_down, "ShortenSlotButton");
        let shorten_cover = sub(&shorten, "Cover");
        let lock_balloon = sub(&shorten_cover, "UIPartsCommonBalloon");
        let lock_balloon_text = sub(&lock_balloon, "Content/CustomText");
        Self {
            canvas_group: None,
            root: root.to_owned(),
            shadow: sub(&gauge, "Shadow"),
            base: sub(&gauge, "Base"),
            wave: sub(&gauge, "Base/WaveImage"),
            processing_highlight: sub(&gauge, "Base/ProcessingHighlightEffectRoot"),
            complete_highlight: sub(&gauge, "Base/CompleteHighlightEffectRoot"),
            blah: sub(&gauge, "Blah"),
            highlights: sub(&gauge, "HighLights"),
            ready_content: sub(&gauge, "ReadyContent"),
            empty_icon: sub(&gauge, "ReadyContent/EmptyIcon"),
            select_item: sub(&gauge, "ReadyContent/SelectItem"),
            setting_message: sub(&gauge, "SettingMessage"),
            processing_content: sub(&gauge, "ProcessingContent"),
            processing_message: sub(&gauge, "ProcessingContent/ProcessingMessage"),
            complete_message: sub(&gauge, "CompleteMessage"),
            slot_number: sub(&gauge, "SlotNumberText"),
            cover_interaction: sub(&gauge, "CoverInteraction"),
            frame: sub(&gauge, "Frame"),
            button: sub(&gauge, "Button"),
            lock: sub(&gauge, "Lock"),
            lock_icon: sub(&gauge, "Lock/Icon"),
            lock_text: sub(&gauge, "Lock/Message/Text"),
            button_view: button_view.clone(),
            off: sub(&button_view, "Content/Off"),
            on: sub(&button_view, "Content/On"),
            un_receive: sub(&button_view, "Content/UnReceive"),
            receivable: sub(&button_view, "Content/Receiveable"),
            interact: sub(&button_view, "Content/Interact"),
            effect: sub(&button_view, "Content/Effect"),
            view_button: sub(&button_view, "Button"),
            time_icon: sub(&count_down, "TimeIcon"),
            timer_text: sub(&count_down, "TimerText"),
            count_down,
            shorten_icon: sub(&shorten, "Icon"),
            shorten_cover,
            lock_balloon,
            lock_balloon_text,
            shorten,
            gauge,
        }
    }
}

/// `WordingManager.Get(key)` through the layout's dictionary.
pub fn text(layouts: &UiLayouts, key: &str) -> String {
    layouts.wording(key)
}

/// `string.Format(WordingManager.Get(key), args)`; a format this reader
/// cannot apply is named once and the line is left empty.
pub fn format(layouts: &UiLayouts, seen: &mut BTreeSet<String>, key: &str, args: &[String]) -> String {
    let source = layouts.wording(key);
    match moly_law::text::custom_text_mesh::format_wording(&source, args) {
        Ok(line) => line,
        Err(error) => {
            super::super::screen::note_once(
                seen,
                format!("the wording {key} ({source:?}) does not take {args:?}: {error}"),
            );
            String::new()
        }
    }
}

/// `TimeSpan.ToTimeString()`: hours, minutes and seconds as `HH:mm:ss`, the
/// hours not folded into days. The source's extension is not in this tree:
/// the format is unverified against the client.
pub fn to_time_string(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// `IsReleasedShorten`: the `convert_time_shorten` release row names a level
/// the machine (`UserMysekaiGamedata.mysekaiConvertFixtureLevel`) reaches.
pub fn is_released_shorten(ctx: ItemContext<'_>) -> bool {
    let machine = ctx.machine();
    machine.is_released_shorten(machine.level(ctx.user))
}

/// `IsReleasedShorten`'s released level: the `convert_time_shorten` row's
/// `mysekaiConvertFixtureLevel` (`WORD_CONVERT_MACHINE_LEVEL`'s argument).
pub fn released_shorten_level(ctx: ItemContext<'_>) -> i32 {
    super::super::model::released_shorten_level(ctx.masters)
}

/// The water-level tween the cell runs: `DOVirtual.Float(from, to,
/// _receiveAnimationSeconds, ...)` with the ease `SetEase` leaves unset, the
/// DOTween default `Ease.OutQuad` (`DOTween.Init` `defaultEaseType =
/// Ease.OutQuad`; `EaseManager.Evaluate` case 8: `num27 = t / d + -1`,
/// `1 - num27 * num27`). It reaches `to` only at the end of its duration.
#[derive(Debug, Clone, Copy)]
pub struct WaveTween {
    pub from: f32,
    pub to: f32,
    pub seconds: f32,
    pub elapsed: f32,
}

impl WaveTween {
    /// One step: the value to show and whether the tween has finished.
    pub fn step(&mut self, dt: f32) -> (f32, bool) {
        self.elapsed += dt;
        let t = if self.seconds > 0.0 {
            (self.elapsed / self.seconds).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let value = self.from + (self.to - self.from) * crate::ui_inputs::out_quad(t);
        (value.clamp(0.0, 1.0), self.elapsed >= self.seconds)
    }
}

/// One cell's view state, as the source's three sub-views hold it. The
/// default is a cell with no state set up yet (every content off, the timer
/// at its not-processing look).
#[derive(Debug, Clone)]
pub struct CellView {
    /// `_slotState`.
    pub state: Option<SlotState>,
    /// The wave is shown (processing and complete), and its progress; `None`
    /// leaves the wave hidden.
    pub wave_progress: Option<f32>,
    /// `_progressText` (`ProcessingContent/ReceiveCountText`).
    pub progress_visible: bool,
    /// The gauge's `Button` listens for the locked and ready states.
    pub gauge_button: bool,
    /// `On`, `UnReceive`, `Receiveable`, `Interact` and `Effect` of the
    /// button view.
    pub on: bool,
    pub un_receive: bool,
    pub receivable: bool,
    pub interact: bool,
    pub effect: bool,
    pub effect_seconds: f32,
    /// `Root/ButtonView/Button` enabled (a processing cell with something to
    /// receive, or a complete one).
    pub view_button_enabled: bool,
    /// `TimerText` (`None` leaves the layout's serialized text).
    pub timer_text: Option<String>,
    /// The count down background's alpha.
    pub timer_alpha: f32,
    /// `ShortenSlotButton` shown (`Active`) and `enabled`.
    pub shorten_visible: bool,
    pub shorten_enabled: bool,
    /// The skip's cover listens (`ShowShortenLockedBalloon`).
    pub cover_listens: bool,
    /// The lock balloon (`Cover/UIPartsCommonBalloon`): its text
    /// (`WORD_CONVERT_MACHINE_LEVEL`, set while the skip is locked) and
    /// whether its cover opened it (`ShowShortenLockedBalloon`).
    pub lock_balloon_text: Option<String>,
    pub lock_balloon_open: bool,
    /// `ReceiveCountText`.
    pub progress_text: Option<String>,
    /// `UIPartsConvertItem.SetupEmpty`: the thumbnail is off.
    pub item_empty: bool,
    /// `SlotNumberText`.
    pub slot_number: Option<String>,
    /// The model's values the next frame compares (`UpdateSlotView`).
    pub before: Option<super::super::model::OwnedSlotModel>,
}

impl CellView {
    /// The skip button shows and its cover listens (the skip is locked).
    pub fn shortening_locked(&self) -> bool {
        self.shorten_visible && self.cover_listens
    }
}

impl Default for CellView {
    fn default() -> Self {
        Self {
            state: None,
            wave_progress: None,
            progress_visible: false,
            gauge_button: false,
            on: false,
            un_receive: false,
            receivable: false,
            interact: false,
            effect: false,
            effect_seconds: 0.0,
            view_button_enabled: false,
            timer_text: None,
            timer_alpha: TIMER_IDLE_ALPHA,
            shorten_visible: false,
            shorten_enabled: false,
            cover_listens: false,
            lock_balloon_text: None,
            lock_balloon_open: false,
            progress_text: None,
            item_empty: false,
            slot_number: None,
            before: None,
        }
    }
}

/// One cell.
#[derive(Debug)]
pub struct Cell {
    pub nodes: CellNodes,
    /// `_slotId`.
    pub slot_id: i32,
    pub view: CellView,
    pub tween: Option<WaveTween>,
}

impl Cell {
    pub fn new(nodes: CellNodes, slot_id: i32) -> Self {
        Self {
            nodes,
            slot_id,
            view: CellView::default(),
            tween: None,
        }
    }

    /// `MysekaiConvertFixtureSlotView.Setup(slotId, model)`: the slot number
    /// and the converted item's thumbnail (`SetupEmpty` without one).
    pub fn setup_slot_view(
        &mut self,
        model: &SlotModel<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        self.slot_id = model.slot_id();
        self.view.slot_number = Some(format(
            layouts,
            seen,
            SLOT_NO_FORMAT,
            &[model.slot_id().to_string()],
        ));
        self.view.item_empty = model.item_id().is_none();
    }

    /// `MysekaiConvertFurnitureSlotCellView.Setup`: the slot view, then
    /// `SetupView` of the three sub-views.
    pub fn setup(
        &mut self,
        model: &SlotModel<'_>,
        ctx: ItemContext<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        self.setup_slot_view(model, layouts, seen);
        self.setup_view(model, ctx, layouts, seen);
    }

    /// `MysekaiConvertFixtureSlotView.Setup(slotId, model)` only: the detail
    /// page's own gauge view shows an item that is not a slot.
    pub fn setup_slot_only(
        &mut self,
        model: &SlotModel<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        self.setup_slot_view(model, layouts, seen);
    }

    fn progress_line(
        &self,
        model: &SlotModel<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        model.is_processing().then(|| {
            format(
                layouts,
                seen,
                PROGRESS_FORMAT,
                &[
                    model.current_completed_convert_count().to_string(),
                    model.remaining_convert_count().to_string(),
                ],
            )
        })
    }

    /// `SetupView`: all three sub-views rebuilt for the slot's state. The
    /// wave tween runs while the state holds (`_playWaveAnimation && state
    /// == state`), so a rebuild keeps it.
    pub fn setup_view(
        &mut self,
        model: &SlotModel<'_>,
        ctx: ItemContext<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        let state = model.state();
        // `IsReleasedShorten` at the machine's level; the machine's level is
        // the user's `mysekaiConvertFixtureLevel`.
        let released = is_released_shorten(ctx);
        let view = &mut self.view;
        view.state = Some(state);
        // The gauge: one content per state (`RefreshContent`); the wave only
        // while processing or complete; the highlight root of processing or
        // complete. The gauge's `Button` takes the locked and ready cells'
        // press (`BindSlotButton`); the button view's `Button` takes a
        // processing cell's receive and a complete cell's press.
        view.gauge_button = matches!(state, SlotState::Locked | SlotState::Ready);
        if self.tween.is_none() {
            view.wave_progress = match state {
                SlotState::Processing => Some(model.normalized_progress_time()),
                SlotState::Complete => Some(1.0),
                SlotState::Locked | SlotState::Ready => None,
            };
        }
        view.progress_visible = state.is_processing();
        // The button view.
        let receivable = model.is_receivable();
        view.on = matches!(state, SlotState::Processing | SlotState::Complete);
        view.un_receive = state == SlotState::Processing && !receivable;
        view.receivable = view.on && receivable;
        view.interact = state == SlotState::Ready;
        view.effect = state == SlotState::Complete;
        view.effect_seconds = 0.0;
        view.view_button_enabled =
            (state == SlotState::Processing && receivable) || state == SlotState::Complete;
        // The count down: the remaining time for a processing cell only.
        view.shorten_visible = state == SlotState::Processing;
        view.shorten_enabled = view.shorten_visible && released;
        view.cover_listens = view.shorten_visible && !released;
        view.lock_balloon_open = false;
        view.lock_balloon_text = None;
        view.timer_alpha = if view.shorten_visible {
            1.0
        } else {
            TIMER_IDLE_ALPHA
        };
        view.timer_text = Some(if view.shorten_visible {
            to_time_string(model.remaining_time_to_complete_convert())
        } else {
            text(layouts, THREE_HYPHEN)
        });
        if view.shorten_visible && !released {
            // `SetupShortenSlotButton`: the cover reads the released level.
            let line = format(
                layouts,
                seen,
                MACHINE_LEVEL,
                &[released_shorten_level(ctx).to_string()],
            );
            view.lock_balloon_text = Some(line);
        }
        // RefreshView of the numbers.
        self.refresh_view(model, layouts, seen);
    }

    /// `RefreshView` / `RefreshInProcessing` / `RefreshInComplete`.
    pub fn refresh_view(
        &mut self,
        model: &SlotModel<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        let state = model.state();
        let progress_text = self.progress_line(model, layouts, seen);
        let tweening = self.tween.is_some();
        let view = &mut self.view;
        // A running tween owns the wave until it lands.
        if !tweening {
            view.wave_progress = match state {
                SlotState::Processing => Some(model.normalized_progress_time()),
                SlotState::Complete => Some(1.0),
                SlotState::Locked | SlotState::Ready => None,
            };
        }
        view.progress_text = progress_text;
        if state == SlotState::Processing {
            view.timer_text = Some(to_time_string(model.remaining_time_to_complete_convert()));
            let receivable = model.is_receivable();
            view.un_receive = !receivable;
            view.receivable = receivable;
            view.view_button_enabled = receivable;
        }
        view.before = Some(model.owned());
    }

    /// `UpdateSlotView()`: a rebuild when the state changed, a refresh of the
    /// numbers otherwise.
    pub fn update_slot_view(
        &mut self,
        model: &SlotModel<'_>,
        ctx: ItemContext<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) -> SlotChange {
        let change = match self.view.before.as_ref() {
            Some(before) => model.change_since(before),
            None => SlotChange::State,
        };
        match change {
            SlotChange::Same => {}
            SlotChange::Numbers => self.refresh_view(model, layouts, seen),
            SlotChange::State => self.setup_view(model, ctx, layouts, seen),
        }
        change
    }

    /// `PlayReceiveAsync(from, to)`: the wave from the slot's progress at the
    /// receive to the fresh model's progress.
    pub fn play_receive(&mut self, from: f32, to: f32) {
        self.tween = Some(WaveTween {
            from,
            to,
            seconds: RECEIVE_SECONDS,
            elapsed: 0.0,
        });
    }

    /// `Timer.PlayConvertStart(model)`: the count down's opacity 1, the skip
    /// hidden (`SetActive(false)`), and the remaining time of the model the
    /// cell holds; the slot's next read shows the skip again for its state.
    #[allow(dead_code)] // The start's process (U9) plays it after its request's answer.
    pub fn play_convert_start_timer(&mut self, model: &SlotModel<'_>) {
        self.view.timer_alpha = 1.0;
        self.view.shorten_visible = false;
        self.view.shorten_enabled = false;
        self.view.cover_listens = false;
        self.view.timer_text = Some(to_time_string(model.remaining_time_to_complete_convert()));
    }

    /// `Button.PlayConvertStart()`: `On` and `UnReceive` on, `Receiveable` off,
    /// the button disabled. The source enables its off image rather than its
    /// on image (the `Off`/`On` contents here are named by state, not by the
    /// image fields); the cell keeps the processing look the state gives it.
    #[allow(dead_code)] // The start's process (U9) plays it after its request's answer.
    pub fn play_convert_start_button(&mut self) {
        self.view.on = true;
        self.view.un_receive = true;
        self.view.receivable = false;
        self.view.view_button_enabled = false;
    }

    /// `Gauge.PlayConvertStartAsync(model)`: `slotModel.SetupSlotState()`,
    /// the processing content with its progress line and the wave hidden
    /// through `_settingAnimationSeconds` (with the cancellation swallowed);
    /// then the state read again, the wave at `NormalizedProgressTime` and
    /// shown, fading in over `_receiveAnimationSeconds`. The wave is drawn by
    /// the slot's next read (`RefreshInProcessing`), so the wait and the fade
    /// are named and the wave starts where that read puts it.
    #[allow(dead_code)] // The start's process (U9) plays it after its request's answer.
    pub fn play_convert_start(
        &mut self,
        model: &SlotModel<'_>,
        ctx: ItemContext<'_>,
        layouts: &UiLayouts,
        seen: &mut BTreeSet<String>,
    ) {
        // The state read again and the content of it (the processing content
        // with the progress line, the highlight root, the gauge's button off,
        // the button view's and the count down's processing look).
        self.tween = None;
        self.setup_view(model, ctx, layouts, seen);
        // The wave hides for the wait; the next read shows it at the slot's
        // own `NormalizedProgressTime`.
        self.view.wave_progress = None;
    }

    /// `ShowShortenLockedBalloon`: the skip's cover opens its balloon.
    pub fn show_shorten_locked_balloon(&mut self) {
        if self.view.shortening_locked() {
            self.view.lock_balloon_open = true;
        }
    }

    /// `PlayShortenAsync(progress)`: the wave from the slot's progress to 1.
    pub fn play_shorten(&mut self, progress: f32) {
        self.tween = Some(WaveTween {
            from: progress,
            to: 1.0,
            seconds: RECEIVE_SECONDS,
            elapsed: 0.0,
        });
    }

    /// `Dispose`: the wave's `Dispose` and the three effects' prefabs
    /// destroyed (the layout draws no effect prefab, named).
    pub fn dispose(&mut self) {
        self.tween = None;
        self.view.effect = false;
    }

    /// One frame of the cell's content: the gauge's four states, the button
    /// view's look and enable, the count down's alpha, text and skip button.
    /// The gauge's `Button` and the button view's `Button` are leaf buttons:
    /// `enabled = false` would also hide their raycast target, so their
    /// enable is kept by [`Self::view`] alone (the press goes to the
    /// covering `CoverInteraction` and the target reads `live_button`).
    pub fn draw(
        &self,
        view: &mut UiPrefabView,
        layouts: &UiLayouts,
        art: Option<&BalloonArt>,
        warned: &mut BTreeSet<String>,
    ) {
        let nodes = &self.nodes;
        let state = self.view.state;
        let locked = state == Some(SlotState::Locked);
        let ready = state == Some(SlotState::Ready);
        let processing = state == Some(SlotState::Processing);
        let complete = state == Some(SlotState::Complete);
        // The gauge: one content per state, the wave and the highlights. The
        // cell's CanvasGroup alpha is not what the layout draws (named):
        // `Show` / `Hide` leave the cell drawn or not with its page. The
        // base, the shadows, the highlights, the frame and the cover effect
        // keep their serialized objects.
        view.set_visible(&nodes.lock, locked);
        view.set_visible(&nodes.ready_content, ready);
        view.set_visible(&nodes.processing_content, processing);
        view.set_visible(&nodes.complete_message, complete);
        // The wave is the serialized `Base/WaveImage` (a `UIWaveGaugeAnimation`
        // whose fill a code-driven value cuts); its animation prefab is not in
        // the public asset root, so the progress is drawn as the image's fill.
        let wave = self
            .view
            .wave_progress
            .filter(|_| processing || complete);
        view.set_visible(&nodes.wave, wave.is_some());
        if let Some(progress) = wave {
            view.set_fill(&nodes.wave, progress);
        }
        view.set_visible(&nodes.processing_highlight, processing);
        view.set_visible(&nodes.complete_highlight, complete);
        let progress = self
            .view
            .progress_text
            .as_ref()
            .filter(|_| self.view.progress_visible);
        let receive = format!("{}/ReceiveCountText", nodes.processing_content);
        let shows = put_text(view, &receive, progress, art, warned);
        view.set_visible(&receive, progress.is_some() && shows);
        // The slot number (set by `SetText`; a missing glyph hides it).
        let number = self.view.slot_number.clone();
        let shows = put_text(view, &nodes.slot_number, number.as_ref(), art, warned);
        view.set_visible(&nodes.slot_number, shows);
        // The button view.
        view.set_visible(&nodes.on, self.view.on);
        view.set_visible(&nodes.off, !self.view.on);
        view.set_visible(&nodes.un_receive, self.view.un_receive);
        view.set_visible(&nodes.receivable, self.view.receivable);
        view.set_visible(&nodes.interact, self.view.interact);
        view.set_visible(&nodes.effect, self.view.effect);
        // The count down: the source's base image alpha (0.1 when not
        // processing) is not what the layout draws (named); the text and the
        // skip button are. The skip button's enabled is its face: the
        // source's `disabledImage` cover (`ShortenSlotButton/Cover`) shows
        // while the skip is locked, and its balloon line is that cell's
        // `WORD_CONVERT_MACHINE_LEVEL` line, shown when the cover opens it.
        view.set_visible(&nodes.shorten, self.view.shorten_visible);
        view.set_visible(&nodes.shorten_icon, self.view.shorten_visible);
        view.set_visible(
            &nodes.shorten_cover,
            self.view.shorten_visible && !self.view.shorten_enabled,
        );
        // `UIPartsCommonBalloon.Show()` on the skip's cover.
        let balloon = self
            .view
            .lock_balloon_open
            .then_some(self.view.lock_balloon_text.as_ref())
            .flatten();
        let shows = put_text(view, &nodes.lock_balloon_text, balloon, art, warned);
        view.set_visible(&nodes.lock_balloon, balloon.is_some() && shows);
        let timer = self.view.timer_text.clone();
        let shows = put_text(view, &nodes.timer_text, timer.as_ref(), art, warned);
        view.set_visible(&nodes.timer_text, shows);
        let _ = (locked, ready, layouts);
    }

    /// One frame: the wave and the effect tweens.
    pub fn step(&mut self, dt: f32) {
        if let Some(tween) = self.tween.as_mut() {
            let (value, done) = tween.step(dt);
            self.view.wave_progress = Some(value);
            if done {
                self.tween = None;
            }
        }
        if self.view.effect {
            self.view.effect_seconds = (self.view.effect_seconds + dt).min(1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_string_counts_hours_minutes_and_seconds() {
        assert_eq!(to_time_string(0), "00:00:00");
        assert_eq!(to_time_string(999), "00:00:00");
        assert_eq!(to_time_string(1_000), "00:00:01");
        assert_eq!(to_time_string(59_999), "00:00:59");
        assert_eq!(to_time_string(60_000), "00:01:00");
        assert_eq!(to_time_string(3_723_000), "01:02:03");
        // Past a day the hours keep counting rather than folding into days.
        assert_eq!(to_time_string(26 * 3_600_000 + 5_000), "26:00:05");
        assert_eq!(to_time_string(-1), "00:00:00");
    }

    #[test]
    fn the_wave_tween_lands_on_its_end() {
        let mut tween = WaveTween {
            from: 0.25,
            to: 0.75,
            seconds: 1.0,
            elapsed: 0.0,
        };
        let (half, done) = tween.step(0.5);
        assert!(!done);
        // OutQuad at t = 0.5 is (0.5 - 2) * -0.5 = 0.75.
        assert!((half - (0.25 + 0.5 * 0.75)).abs() < 1e-6, "{half}");
        let (end, done) = tween.step(0.5);
        assert!(done);
        assert!((end - 0.75).abs() < 1e-6, "{end}");
    }

}
