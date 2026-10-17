//! The home actions' requests as the client sends them: craft and canvas
//! (`PostUserMysekaiCraftApi`) and sketch (`PostUserMysekaiHousingSketchApi`).
//! The client starts the world sequence once the reply succeeds.
//!
//! The request runs through [`HomeActionEndpoint`], a one-shot system the
//! server model installs; [`post`] runs it and returns the reply. Without an
//! endpoint the request is refused by name, as a request with no server to
//! answer it.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

pub(crate) use moly_protocol::api::{HomeActionApi, HomeActionPost, HomeActionReply};

/// The server model's endpoint for the home actions' requests.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HomeActionEndpoint(pub(crate) SystemId<In<HomeActionPost>, HomeActionReply>);

/// Send a request and read its reply.
#[allow(dead_code)] // Called by the home actions' seam.
pub(crate) fn post(world: &mut World, api: HomeActionApi, target: Option<&str>) -> HomeActionReply {
    let Some(endpoint) = world.get_resource::<HomeActionEndpoint>().copied() else {
        warn!(
            "[server] {} refused: no server endpoint is installed",
            api.api_name()
        );
        return HomeActionReply { success: false };
    };
    let request = HomeActionPost {
        api,
        target: target.map(str::to_owned),
    };
    world
        .run_system_with(endpoint.0, request)
        .unwrap_or_else(|error| {
            warn!("[server] {} refused: {error}", api.api_name());
            HomeActionReply { success: false }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(In(request): In<HomeActionPost>) -> HomeActionReply {
        HomeActionReply {
            success: request.target.as_deref() == Some("fx"),
        }
    }

    #[test]
    fn a_request_runs_through_the_endpoint() {
        let mut world = World::new();
        assert!(!post(&mut world, HomeActionApi::Craft, Some("fx")).success);
        let endpoint = world.register_system(echo);
        world.insert_resource(HomeActionEndpoint(endpoint));
        assert!(post(&mut world, HomeActionApi::Craft, Some("fx")).success);
        assert!(!post(&mut world, HomeActionApi::Sketch, None).success);
    }
}
