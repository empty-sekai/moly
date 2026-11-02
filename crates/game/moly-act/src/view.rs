//! `view.set`: the clean view's switches for a recording.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields};

use moly_features::act::{Kinds, Step};

pub fn register(app: &mut App) {
    Kinds::new()
        .step(Step::new("view.set", false, None, |fields| Ok(Box::new(read_view(fields)?))))
        .install(app);
}

/// One `view.set` step: each switch changes only when given.
#[derive(Debug)]
struct ViewSet {
    hud: Option<bool>,
    player: Option<bool>,
}

fn read_view(fields: &mut Fields<'_>) -> Result<ViewSet, String> {
    let at = fields.at().to_owned();
    let hud = fields.opt_bool("hud")?;
    let player = fields.opt_bool("player")?;
    if hud.is_none() && player.is_none() {
        return Err(format!("{at}: give hud, player or both"));
    }
    Ok(ViewSet { hud, player })
}

impl ActionSpec for ViewSet {
    fn name(&self) -> &str {
        "view.set"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let Some(mut clean) = world.get_resource_mut::<moly_features::clean_view::CleanView>() else {
            return Err("the clean view is not installed".into());
        };
        clean.set(self.hud, self.player);
        let note = clean.note();
        Ok(Box::new(Told { note }))
    }
}

/// A step that ended on the frame it started, with its note.
#[derive(Debug)]
pub struct Told {
    pub note: String,
}

impl ActionRun for Told {
    fn poll(&mut self, _world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        ActPoll::Done(self.note.clone())
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}
