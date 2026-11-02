//! `weather.set`: the phenomenon the site is shown with, requested through
//! the server panel's own schedule edit.

use bevy::prelude::*;
use moly_features::act::{ActCtx, ActionRun, ActionSpec, Fields};

use crate::Requested;
use moly_features::server_panel::PhenomenaScheduleEdit;

use moly_features::act::{Kinds, Step};

pub fn register(app: &mut App) {
    Kinds::new()
        .step(Step::new("weather.set", false, None, |fields| Ok(Box::new(read_weather(fields)?))))
        .install(app);
}

/// One `weather.set` step: the phenomenon id.
#[derive(Debug)]
struct Weather {
    phenomenon: i32,
}

fn read_weather(fields: &mut Fields<'_>) -> Result<Weather, String> {
    Ok(Weather { phenomenon: fields.req_i32("phenomenon")? })
}

impl ActionSpec for Weather {
    fn name(&self) -> &str {
        "weather.set"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        world.write_message(PhenomenaScheduleEdit::Set(self.phenomenon));
        Ok(Box::new(Requested { note: format!("phenomenon {} requested", self.phenomenon) }))
    }
}

