//! `player.deliver`, registered with the hub's action registry: the delivery
//! button's press and release, the two requests the delivery screen sends, on
//! a birthday party in session. The delivery's own rules decide what a press
//! does (the release window, `CanDelivery`, the ends of the hold loop and the
//! end action); the step ends once the end action has run, with the points
//! the party gained.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::act::hand;

use crate::honor::DialogAwait;
use crate::{DeliveryActionState, DeliveryModel, DeliveryRequest};

/// A tap that has started no delivery after this long (seconds) is refused.
const DELIVER_ANSWER_SECS: f32 = 2.0;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("player.deliver", true, Some(180.0), |fields| Ok(Box::new(read_deliver(fields)?))))
        .install(app);
}

/// `player.deliver`: the delivery button's press, its hold and its release.
#[derive(Debug)]
struct Deliver {
    party: Option<i32>,
    hold: Option<f32>,
}

fn read_deliver(fields: &mut Fields<'_>) -> Result<Deliver, String> {
    let at = fields.at().to_owned();
    let party = fields.opt_i32("party")?;
    let hold = fields.opt_f32("hold")?;
    if hold == Some(0.0) {
        return Err(format!("{at}: hold is a number of seconds above 0"));
    }
    Ok(Deliver { party, hold })
}

impl ActionSpec for Deliver {
    fn name(&self) -> &str {
        "player.deliver"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        deliver_press(world, self.party, self.hold, ctx.now)
            .map(|run| Box::new(run) as Box<dyn ActionRun>)
    }
}

/// A delivery in progress.
struct DeliverRun {
    /// The party the press named.
    party: i32,
    /// Seconds to hold the button; none holds it until the party has nothing
    /// left to deliver.
    hold: Option<f32>,
    /// When the press went out, and when the delivery was seen running.
    asked_at: f32,
    started_at: Option<f32>,
    /// The release has been sent.
    released: bool,
    points_at_press: i32,
    /// The refresh dialog's back key was pressed.
    closed_refresh: bool,
}

/// The press: refused before it is sent when no party is in session.
fn deliver_press(world: &mut World, party: Option<i32>, hold: Option<f32>, now: f32) -> Result<DeliverRun, String> {
    let Some(model) = world.get_resource::<DeliveryModel>() else {
        return Err("the delivery screen is not set up".to_owned());
    };
    if !model.parties.iter().any(|site| party.is_none_or(|id| site.id == id)) {
        return Err(match party {
            Some(id) => format!("no birthday party {id} is in session"),
            None => "no birthday party is in session".to_owned(),
        });
    }
    let id = party.unwrap_or_else(|| model.parties[0].id);
    let points = model.party(id).map_or(0, |site| site.tally.current_points());
    world.write_message(DeliveryRequest::Start(Some(id)));
    Ok(DeliverRun {
        party: id,
        hold,
        asked_at: now,
        started_at: None,
        released: false,
        points_at_press: points,
        closed_refresh: false,
    })
}

impl ActionRun for DeliverRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let Some(model) = world.get_resource::<DeliveryModel>() else {
            return ActPoll::Failed("the delivery screen is not set up".into());
        };
        let state = model.state;
        let executing = model.executing;
        let home = world.get_resource::<moly_features::site::SiteActive>().map(|site| site.site_type.clone());
        let (points, can, left) = model.party(self.party).map_or((0, false, 0), |site| {
            (site.tally.current_points(), site.tally.can_delivery(), site.tally.remaining())
        });
        if !executing && self.started_at.is_none() {
            // The press starts the flow in the frame after the request; a
            // press the flow refuses leaves the state where it was.
            if ctx.now - self.asked_at <= DELIVER_ANSWER_SECS {
                return ActPoll::Running(format!("waiting for the delivery of party {} to start (state {state:?})", self.party));
            }
            return ActPoll::Failed(format!(
                "the press on party {} started no delivery (state {state:?}, {left} left to deliver, site {home:?})",
                self.party
            ));
        }
        if self.started_at.is_none() {
            self.started_at = Some(ctx.now);
        }
        if !self.released {
            let held = self.hold.is_some_and(|hold| ctx.now - self.asked_at >= hold);
            if held || !can {
                world.write_message(DeliveryRequest::End);
                self.released = true;
                return ActPoll::Running(format!(
                    "released the delivery button ({}, {left} left to deliver)",
                    if held { "held long enough" } else { "nothing left to deliver" }
                ));
            }
        }
        if !executing && state == DeliveryActionState::Idle {
            return ActPoll::Done(format!("party {} delivered: points {} to {}", self.party, self.points_at_press, points));
        }
        // The end action waits for the refresh dialog's close, which the back
        // key gives it.
        let refreshing = world.get_resource::<DialogAwait>().is_some_and(|awaiting| awaiting.refresh.is_some());
        if !self.closed_refresh && refreshing {
            self.closed_refresh = true;
            return match hand::back_key(world) {
                Ok(()) => ActPoll::Running("pressed the back key on the refresh dialog".into()),
                Err(error) => ActPoll::Running(error.to_owned()),
            };
        }
        ActPoll::Running(format!(
            "delivering party {}: state {state:?}, {points} points, {left} left to deliver",
            self.party
        ))
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // A press left holding the button is released.
        world.write_message(DeliveryRequest::End);
    }
}
