//! `site.switch`: the site requested through the site move.
//!
//! The door sequences (`site.enter_room`, `site.go_home`) are a macro of flow
//! steps and of the player's `player.interact` with a door's button: the
//! script runner expands them, not this crate.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields};
use moly_features::site::SiteChangeRequest;

use moly_features::act::{Kinds, Step};

pub fn register(app: &mut App) {
    Kinds::new()
        .step(Step::new("site.switch", false, None, |fields| Ok(Box::new(read_site(fields)?))))
        .install(app);
}

/// One `site.switch` step: the site type to move to.
#[derive(Debug)]
struct SiteSwitch {
    site: String,
}

fn read_site(fields: &mut Fields<'_>) -> Result<SiteSwitch, String> {
    Ok(SiteSwitch { site: fields.req_str("site")?.to_owned() })
}

impl ActionSpec for SiteSwitch {
    fn name(&self) -> &str {
        "site.switch"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        world.insert_resource(SiteChangeRequest(self.site.clone()));
        Ok(Box::new(Switching { site: self.site.clone(), moved: false }))
    }
}

/// The move, until the site is the one asked for.
struct Switching {
    site: String,
    /// Once the move has been seen in progress.
    moved: bool,
}

impl ActionRun for Switching {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let site = &self.site;
        let current = world.get_resource::<moly_features::site::SiteActive>().map(|active| active.site_type.clone());
        let moving = world.contains_resource::<moly_features::site_move::SiteMoveActive>();
        if current.as_deref() == Some(site.as_str()) && !moving {
            ActPoll::Done(format!("on {site}"))
        } else if self.moved && !moving {
            // A move that ends elsewhere was refused.
            ActPoll::Failed(format!("the move ended on {current:?}"))
        } else {
            self.moved |= moving;
            ActPoll::Running(format!("site {current:?}, moving {moving}"))
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}
