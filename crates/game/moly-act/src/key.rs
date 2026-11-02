//! `key.back`: one press of the back key, and `dialog.answer`: the shown
//! common dialog's OK or cancel button, or the back key a sub-window dialog
//! without a common button takes.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::ui_layers::{DialogId, DialogState, DialogType, ScreenManager};

use moly_features::act::hand;
use crate::Requested;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("key.back", true, None, |_| Ok(Box::new(BackKey))))
        .step(Step::new("dialog.answer", true, Some(10.0), |fields| Ok(Box::new(read_answer(fields)?))))
        .install(app);
}

/// One `key.back` step.
#[derive(Debug)]
struct BackKey;

impl ActionSpec for BackKey {
    fn name(&self) -> &str {
        "key.back"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        match hand::back_key(world) {
            Ok(()) => Ok(Box::new(Requested { note: "the back key pressed".to_owned() })),
            Err(error) => Err(error.to_owned()),
        }
    }
}

/// `dialog.answer`: a tap on the shown common dialog's OK or cancel button,
/// done once the dialog is gone. A sub-window dialog with no common button
/// (the learn-phenomenon one) is answered with the back key its own class
/// registers, which is the answer it takes from the screen.
#[derive(Debug)]
struct Answer {
    ok: bool,
}

fn read_answer(fields: &mut Fields<'_>) -> Result<Answer, String> {
    Ok(Answer { ok: fields.opt_bool("ok")?.unwrap_or(true) })
}

impl ActionSpec for Answer {
    fn name(&self) -> &str {
        "dialog.answer"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(AnswerRun { ok: self.ok, tapped: None, pressed: None }))
    }
}

/// Whether the screen manager still shows the dialog `id`.
fn dialog_open(world: &World, id: DialogId) -> bool {
    world
        .get_resource::<ScreenManager>()
        .is_some_and(|screens| screens.dialogs().any(|(open, _, state)| open == id && state != DialogState::Closed))
}

/// The topmost dialog still open, with its handle: the one the screen
/// manager hands the back key to.
fn topmost_dialog(world: &World) -> Option<(DialogId, DialogType)> {
    world
        .get_resource::<ScreenManager>()?
        .dialogs()
        .filter(|(_, _, state)| *state != DialogState::Closed)
        .map(|(id, dialog, _)| (id, dialog))
        .last()
}

/// An answer in progress.
struct AnswerRun {
    ok: bool,
    tapped: Option<(DialogId, f32)>,
    /// The sub-window dialog answered with the back key, and when.
    pressed: Option<(DialogId, f32)>,
}

impl ActionRun for AnswerRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if let Some((id, at)) = self.pressed {
            if !dialog_open(world, id) {
                return ActPoll::Done(format!("closed the dialog ({})", if self.ok { "answer" } else { "cancel" }));
            }
            if ctx.now - at < 1.0 {
                return ActPoll::Running("waiting for the dialog to close".into());
            }
        }
        if let Some((id, at)) = self.tapped {
            if !moly_features::common_dialog::is_shown(world, id) {
                return ActPoll::Done(format!("answered {}", if self.ok { "OK" } else { "cancel" }));
            }
            if ctx.now - at < 1.0 {
                return ActPoll::Running("waiting for the dialog to close".into());
            }
        }
        let (id, caller, view, node) = match moly_features::common_dialog::shown_button(world, self.ok) {
            Ok(button) => button,
            Err(error) => return self.answer_without_button(world, ctx, error),
        };
        let position = match hand::ui_point(world, view, &node) {
            Ok(position) => position,
            Err(error) => return ActPoll::Running(error),
        };
        match hand::tap(world, position) {
            Ok(()) => {
                self.tapped = Some((id, ctx.now));
                ActPoll::Running(format!("tapped {} on {view} from {caller}", if self.ok { "OK" } else { "cancel" }))
            }
            Err(error) => ActPoll::Running(error.to_owned()),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

impl AnswerRun {
    /// The shown dialog that has no common button. The learn-phenomenon one
    /// takes the back key its own class registers as its whole answer: its
    /// close area, its panel body and the hardware back key all close it, and
    /// the run that opened it waits for its close. Another family's dialog is
    /// named and left alone.
    fn answer_without_button(&mut self, world: &mut World, ctx: &mut ActCtx, error: String) -> ActPoll {
        let Some((id, dialog)) = topmost_dialog(world) else {
            return ActPoll::Running(error);
        };
        if dialog != DialogType::LearnPhenomenaSubWindowDialog {
            return ActPoll::Running(format!("{error}; the topmost dialog is {dialog:?}"));
        }
        match hand::back_key(world) {
            Ok(()) => {
                self.pressed = Some((id, ctx.now));
                ActPoll::Running(format!("pressed the back key on {dialog:?}"))
            }
            Err(error) => ActPoll::Running(error.to_owned()),
        }
    }
}
