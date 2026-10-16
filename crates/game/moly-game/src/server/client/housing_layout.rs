//! The layout editor's save as the client sends it:
//! `PostUserMysekaiHousingLayoutApi` with a `UserMysekaiHousingEditLayoutRequest`
//! (the site and its whole layout). The editor writes its own saved copy
//! only once the reply succeeds; a refusal keeps the draft.
//!
//! The rows are in the served layout's record form (package, master fixture
//! id, footprint corners, centre height, layout type bits, rotation, texture),
//! the form the server document's `userMysekaiSiteHousingLayouts` rows use,
//! not the source's `MysekaiSiteHousingLayout` fields. Named gaps: the
//! request's `mysekaiFixtureSurfaceAppearances` (no surface editing here) and
//! `mysekaiPhenomenaId` are not sent.
//!
//! The request runs through [`HousingLayoutEndpoint`], a one-shot system the
//! server model installs; [`post`] runs it and returns the `SuiteUser` reply.
//! Without an endpoint the request is refused by name.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use serde_json::Value;

use super::craft::SuiteUserReply;

/// The API's name in the source.
pub(crate) const API: &str = "PostUserMysekaiHousingLayoutApi";

/// `UserMysekaiHousingEditLayoutRequest`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserMysekaiHousingEditLayoutRequest {
    pub(crate) mysekai_site_id: u32,
    /// The site's type (`home_site`, ...), which names its served layout row.
    pub(crate) site_type: String,
    /// `userMysekaiSiteHousingLayout.mysekaiSiteHousingLayouts`: every placed
    /// fixture of the site, in the served record form.
    pub(crate) fixtures: Vec<Value>,
}

/// The server model's endpoint for the request.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HousingLayoutEndpoint(
    pub(crate) SystemId<In<UserMysekaiHousingEditLayoutRequest>, SuiteUserReply>,
);

/// Send the request and read its reply.
pub(crate) fn post(
    world: &mut World,
    request: UserMysekaiHousingEditLayoutRequest,
) -> SuiteUserReply {
    let Some(endpoint) = world.get_resource::<HousingLayoutEndpoint>().copied() else {
        warn!("[server] {API} refused: no server endpoint is installed");
        return SuiteUserReply::refused(format!("{API}: no server endpoint is installed"));
    };
    world
        .run_system_with(endpoint.0, request)
        .unwrap_or_else(|error| {
            warn!("[server] {API} refused: {error}");
            SuiteUserReply::refused(format!("{API}: {error}"))
        })
}
