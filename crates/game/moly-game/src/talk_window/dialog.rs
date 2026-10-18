//! `ScenarioAllSkipConfirmDialog` (dialog type 176), the confirmation the talk
//! window's all-skip button shows: `ScreenLayerMysekaiTalk.OnClickAllSkip`
//! turns auto off, then calls
//! `ScreenManager.Show2ButtonDialog<ScenarioAllSkipConfirmDialog>(message,
//! ok, cancel)`; its OK runs the talk's `SkipEvent` and
//! `HideMenuAndShowMenuButton` (not immediate).
//!
//! The dialog is a `Common2ButtonDialog` that only overrides `EnableBlur`, so
//! it is shown through the common two-button window and answer rules
//! ([`crate::common_dialog`]). The window's tap handler only records the
//! request; this module's system opens the dialog with the world before the
//! common dialog's tap handler reads the tap. `MSG_STORY_SKIP_BODY` is the
//! message the layer passes and `WORD_OK`/`WORD_CANCEL` label the buttons.
//!
//! Named gap: the dialog answers itself through the common handler, which
//! reads the buttons' tap for its own account; the talk sees the dialog gone
//! and treats every answer as OK (`SkipEvent`). The cancel needs the shared
//! handler to hand its answer back to the window.

use crate::ui_layers::{DialogId, DialogState, DialogType, ScreenManager};
use crate::ui_layout::UiLayouts;

use super::*;

/// `Sekai.DialogType.ScenarioAllSkipConfirmDialog` = 176.
pub(super) const DIALOG_TYPE: DialogType = DialogType(176);
/// The layer's message wording.
const MESSAGE_KEY: &str = "MSG_STORY_SKIP_BODY";

/// The confirmation: the tap's request (with the message the layer formats),
/// the dialog it opened and its answer.
#[derive(Resource, Default)]
pub(crate) struct AllSkipConfirm {
    /// `OnClickAllSkip` asked for the dialog, with its message.
    requested: Option<String>,
    shown: Option<DialogId>,
    /// The dialog's answer once it closed itself on one.
    answer: Option<bool>,
}

impl AllSkipConfirm {
    /// Whether the confirmation is up: the window's taps are then the
    /// dialog's.
    pub(super) fn is_open(&self) -> bool {
        self.shown.is_some()
    }

    /// The window's input asks for the confirmation (`OnClickAllSkip`).
    pub(super) fn request(&mut self, message: String) {
        self.answer = None;
        self.requested = Some(message);
    }

    /// The message `OnClickAllSkip` passes, from the root's wordings.
    pub(super) fn message(layouts: &UiLayouts) -> String {
        layouts.wording(MESSAGE_KEY)
    }

    /// The answer, once the dialog has one; `true` for OK.
    pub(super) fn take_answer(&mut self) -> Option<bool> {
        self.answer.take()
    }

    /// The dialog still shown, while the screen manager carries it.
    pub(super) fn shown(&self, screens: &ScreenManager) -> Option<DialogId> {
        self.shown.filter(|id| {
            screens
                .dialogs()
                .any(|(open, dialog, _)| open == *id && dialog == DIALOG_TYPE)
        })
    }

    /// The dialog is gone from the screen: its answer counts as OK (see the
    /// module's named gap). Answers once.
    pub(super) fn settle(&mut self, screens: &ScreenManager) -> Option<bool> {
        self.shown?;
        if self.shown(screens).is_some() {
            return None;
        }
        self.shown = None;
        self.answer = Some(true);
        self.take_answer()
    }
}

/// With the rest of the talk window: the dialog opens before the common
/// dialog's tap handler.
pub(super) fn install(app: &mut App) {
    app.init_resource::<AllSkipConfirm>()
        .add_systems(Update, open.before(crate::common_dialog::click));
}

/// `Show2ButtonDialog<ScenarioAllSkipConfirmDialog>(message, OK, cancel)`;
/// refused while one is shown or while the screen still holds a dialog that is
/// opening or closing.
fn open(world: &mut World) {
    let pending = world
        .get_resource_mut::<AllSkipConfirm>()
        .and_then(|mut confirm| confirm.requested.take());
    let Some(message) = pending else {
        return;
    };
    let shown = world
        .get_resource::<AllSkipConfirm>()
        .is_some_and(|confirm| confirm.shown.is_some());
    let waiting = world.get_resource::<ScreenManager>().is_some_and(|screens| {
        screens
            .dialogs()
            .any(|(_, _, state)| state != DialogState::Show)
    });
    if shown || waiting {
        error!("[talkwin] all skip: the confirmation is shown or a dialog before it is still opening; it was not shown");
        return;
    }
    let buttons = crate::common_dialog::Buttons::Two {
        ok: "WORD_OK",
        cancel: "WORD_CANCEL",
    };
    let Some(id) = crate::common_dialog::open(world, "talk all skip", message, buttons, None)
    else {
        return;
    };
    if let Some(mut confirm) = world.get_resource_mut::<AllSkipConfirm>() {
        confirm.shown = Some(id);
    }
    info!(
        "[talkwin] all skip: the confirmation shown as {id:?} ({} inherits the common window)",
        DIALOG_TYPE.0
    );
}
