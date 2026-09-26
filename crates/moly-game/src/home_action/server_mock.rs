//! `HomeActionApiMock`: the one panel for the server-decided half of craft,
//! canvas and sketch.
//!
//! The client sends a craft request from the craft screen, a canvas request
//! from the canvas screen and a sketch request from the sketch screen, and
//! plays the world sequence once the reply succeeds. The reply's contents
//! (the new fixture or blueprint, the materials spent, the experience and the
//! bonuses) only feed the result, bonus and rank dialogs, which are drawn by
//! the screens' owner. None of the server's rules are in the client, so this
//! panel answers every request with success and invents nothing else; the
//! world sequence after the reply is client code and is not mocked.
//!
//! Entries:
//! - `CraftApiMock`: the reply to a craft at a workbench (the fixture's UID).
//! - `CanvasApiMock`: the reply to a canvas painting (the canvas' UID).
//! - `SketchApiMock`: the reply to a sketch (the sketched fixture is chosen
//!   on the sketch screen; the mock takes no fixture).

use bevy::prelude::*;

/// Which home-action API a request goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeActionApi {
    Craft,
    Canvas,
    Sketch,
}

/// The reply the client reads before it starts the world sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HomeActionReply {
    pub(crate) success: bool,
}

/// The panel resource.
#[derive(Resource, Default, Debug)]
pub(crate) struct HomeActionApiMock {
    /// Replies given per API (craft, canvas, sketch), for the status lines.
    pub(crate) replies: [usize; 3],
}

impl HomeActionApiMock {
    /// Echo success for the request (`CraftApiMock`, `CanvasApiMock`,
    /// `SketchApiMock`). `target` is the fixture UID the request names.
    pub(crate) fn post(&mut self, api: HomeActionApi, target: Option<&str>) -> HomeActionReply {
        let slot = match api {
            HomeActionApi::Craft => 0,
            HomeActionApi::Canvas => 1,
            HomeActionApi::Sketch => 2,
        };
        self.replies[slot] += 1;
        let name = match api {
            HomeActionApi::Craft => "CraftApiMock",
            HomeActionApi::Canvas => "CanvasApiMock",
            HomeActionApi::Sketch => "SketchApiMock",
        };
        info!(
            "[home-action-mock] {name}: request for {} answered with success (reply {}; the reply's contents are the mock's absence of them: no fixture, materials, experience or bonus rows)",
            target.unwrap_or("no fixture"),
            self.replies[slot]
        );
        HomeActionReply { success: true }
    }
}
