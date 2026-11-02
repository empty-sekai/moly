//! `player.craft`: one blueprint on the open craft screen. The screen's own
//! checks decide (the apply button's balloon, the term, the server's
//! answer), and each refusal ends the step with its reason.
//!
//! The step's work runs on the hub action registry like any other
//! ([`Craft`]); it is registered by the player's kind, because the step that
//! comes before it (walking to the workbench and opening the craft screen) is
//! the player's.

use bevy::prelude::*;
use moly_features::act::hand;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll};
use moly_features::home_action::craft::{self, CraftPhase};

/// A tap on the craft screen that has not shown its answer after this long
/// (seconds) is refused.
const CRAFT_ANSWER_SECS: f32 = 2.0;

/// One `player.craft` step's work on the craft screen.
#[derive(Debug)]
pub struct Craft {
    pub blueprint: i32,
}

impl ActionSpec for Craft {
    fn name(&self) -> &str {
        "player.craft"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, _world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(CraftRun {
            blueprint: self.blueprint,
            phase: CraftStepPhase::Choose,
            input_at: ctx.now,
            executed: false,
            chosen: String::new(),
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CraftStepPhase {
    /// Waiting for the open screen, then choosing the blueprint.
    Choose,
    /// The choice is handed over; reading the apply button.
    Check,
    /// The apply button is tapped; the confirm is answered.
    Confirm,
    /// The confirm's OK is tapped; the craft runs to its result.
    Craft,
}

/// A craft in progress.
struct CraftRun {
    blueprint: i32,
    phase: CraftStepPhase,
    /// When the last tap or key went out.
    input_at: f32,
    executed: bool,
    chosen: String,
}

impl ActionRun for CraftRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let now = ctx.now;
        let screen = craft::phase(world);
        match self.phase {
            CraftStepPhase::Choose => {
                if screen != CraftPhase::Idle {
                    return ActPoll::Running(format!("waiting for the open craft screen ({screen:?})"));
                }
                match craft::choose(world, self.blueprint) {
                    Ok(note) => {
                        self.chosen = note;
                        self.phase = CraftStepPhase::Check;
                        self.input_at = now;
                        ActPoll::Running(format!("blueprint {} chosen: {}", self.blueprint, self.chosen))
                    }
                    Err(error) => ActPoll::Failed(error),
                }
            }
            CraftStepPhase::Check => match craft::selected(world) {
                Some((id, state)) if id == self.blueprint => match state {
                    Err(reason) => ActPoll::Failed(format!("blueprint {} cannot be crafted: {reason}", self.blueprint)),
                    Ok(()) => {
                        let Some((view, node)) = moly_features::home_action::craft::view::apply_button_node(world) else {
                            return ActPoll::Running("the craft screen's view is not built".into());
                        };
                        let position = match hand::ui_point(world, view, &node) {
                            Ok(position) => position,
                            Err(error) => return ActPoll::Running(error),
                        };
                        match hand::tap(world, position) {
                            Ok(()) => {
                                self.phase = CraftStepPhase::Confirm;
                                self.input_at = now;
                                ActPoll::Running("the apply button tapped".into())
                            }
                            Err(error) => ActPoll::Running(error.to_owned()),
                        }
                    }
                },
                _ if now - self.input_at > CRAFT_ANSWER_SECS => {
                    ActPoll::Failed(format!("blueprint {} was not selected ({})", self.blueprint, self.chosen))
                }
                _ => ActPoll::Running(format!("waiting for blueprint {} to be selected", self.blueprint)),
            },
            CraftStepPhase::Confirm => match screen {
                CraftPhase::Confirm => {
                    let (_, caller, view, node) = match moly_features::common_dialog::shown_button(world, true) {
                        Ok(button) => button,
                        Err(error) => return ActPoll::Running(error),
                    };
                    let position = match hand::ui_point(world, view, &node) {
                        Ok(position) => position,
                        Err(error) => return ActPoll::Running(error),
                    };
                    match hand::tap(world, position) {
                        Ok(()) => {
                            self.phase = CraftStepPhase::Craft;
                            self.input_at = now;
                            ActPoll::Running(format!("the confirm's OK tapped ({caller})"))
                        }
                        Err(error) => ActPoll::Running(error.to_owned()),
                    }
                }
                CraftPhase::TermLimit => {
                    ActPoll::Failed(format!("blueprint {}'s term is over: the term limit dialog shows", self.blueprint))
                }
                _ if now - self.input_at > CRAFT_ANSWER_SECS => ActPoll::Failed(format!(
                    "the apply tap opened no confirm ({screen:?}){}",
                    craft::refusal(world).map(|reason| format!(": {reason}")).unwrap_or_default()
                )),
                _ => ActPoll::Running("waiting for the confirm".into()),
            },
            CraftStepPhase::Craft => match screen {
                CraftPhase::Dialogs
                    if world
                        .get_resource::<moly_features::ui_layers::ScreenManager>()
                        .is_some_and(moly_features::ui_layers::ScreenManager::exists_dialog) =>
                {
                    self.executed = true;
                    // The follow-up dialogs (bonus, rank-up, new site, status)
                    // close on the back key, as on their close buttons.
                    if now - self.input_at > 1.0 {
                        self.input_at = now;
                        if let Err(error) = hand::back_key(world) {
                            return ActPoll::Running(error.to_owned());
                        }
                        return ActPoll::Running("a follow-up dialog shows; the back key closes it".into());
                    }
                    ActPoll::Running("a follow-up dialog shows".into())
                }
                CraftPhase::Executing | CraftPhase::Dialogs | CraftPhase::Done => {
                    self.executed = true;
                    ActPoll::Running(format!("crafting ({screen:?})"))
                }
                CraftPhase::Result => {
                    self.executed = true;
                    // The result dialog closes on the back key, as on its
                    // close area.
                    if now - self.input_at > 1.0 {
                        self.input_at = now;
                        if let Err(error) = hand::back_key(world) {
                            return ActPoll::Running(error.to_owned());
                        }
                        return ActPoll::Running("the result shows; the back key closes it".into());
                    }
                    ActPoll::Running("the result shows".into())
                }
                CraftPhase::Idle | CraftPhase::Closed if self.executed => {
                    ActPoll::Done(format!("blueprint {} crafted ({})", self.blueprint, self.chosen))
                }
                CraftPhase::Confirm if now - self.input_at > CRAFT_ANSWER_SECS => {
                    ActPoll::Failed("the confirm's OK tap did not answer it".into())
                }
                _ if now - self.input_at > CRAFT_ANSWER_SECS => ActPoll::Failed(format!(
                    "the craft was not done: {}",
                    craft::refusal(world).unwrap_or_else(|| format!("the screen is {screen:?}"))
                )),
                _ => ActPoll::Running("waiting for the craft".into()),
            },
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}
