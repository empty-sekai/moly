//! The home actions' requests as the client sends them: craft and canvas
//! (`PostUserMysekaiCraftApi`) and sketch (`PostUserMysekaiHousingSketchApi`).
//! The client starts the world sequence once the reply succeeds.
//!
//! The request is one `Call::HomeAction` on the server link (see
//! [`crate::home_action`]); this module keeps the endpoint type the in-process
//! model registers.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

pub(crate) use moly_protocol::api::{HomeActionApi, HomeActionPost, HomeActionReply};

/// The server model's endpoint for the home actions' requests.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HomeActionEndpoint(pub(crate) SystemId<In<HomeActionPost>, HomeActionReply>);
