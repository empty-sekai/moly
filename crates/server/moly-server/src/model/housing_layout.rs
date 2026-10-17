//! `PostUserMysekaiHousingLayoutApi` (`UserMysekaiHousingEditLayoutRequest`
//! {mysekaiSiteId, userMysekaiSiteHousingLayout, mysekaiPhenomenaId}): the
//! layout editor's save. The server stores the site's layout and replies with
//! the user data it changed (`SuiteUser`). The client sends it through
//! the client post helper; the server plugin installs
//! [`handle`] as its endpoint.
//!
//! The stored layout is the served one: the document's
//! `userMysekaiSiteHousingLayouts` row of the site's type gets the posted
//! rows, so the next join serves them. The rows are checked for the fields
//! the served form needs; the site layout rules were checked by the editor.
//!
//! **Named policy**: the served layouts hold only the home site's row (the
//! server panel reads that row alone), so a save of another site is accepted
//! and not stored. A layout changes no owned table (placing and storing move
//! the layouted count, not the quantity), so the reply's `SuiteUser` sections
//! carry nothing new; the pending sections go out as a response.
//!
//! **Not modelled** (named): the reply's `deletedMysekaiCharacterTalkIds` and
//! the talk lotteries a new layout would draw.

use bevy::prelude::*;

pub(crate) use moly_protocol::api::{
    SuiteUserReply, UserMysekaiHousingEditLayoutRequest,
};
pub(crate) use moly_protocol::rows::SuiteUserSections;
use super::{ResponseKind, ServerModel, Tables};

/// The API's name in the source.
pub(crate) const API: &str = "PostUserMysekaiHousingLayoutApi";

impl ServerModel {
    pub(crate) fn housing_layout(
        &mut self,
        request: UserMysekaiHousingEditLayoutRequest,
        masters: &Tables<'_>,
    ) -> SuiteUserReply {
        let UserMysekaiHousingEditLayoutRequest {
            mysekai_site_id,
            site_type,
            fixtures,
        } = request;
        let count = fixtures.len();
        let served = self.doc.layouts.as_array_mut().and_then(|rows| {
            rows.iter_mut()
                .find(|row| row["siteType"].as_str() == Some(site_type.as_str()))
        });
        match served {
            Some(row) => {
                row["mysekaiFixtures"] =
                    moly_protocol::json::housing_layout_fixtures_value(&fixtures);
                info!(
                    "[server] {} for site {mysekai_site_id} ({site_type}): {count} rows stored in the served layout; the reply's SuiteUser carries no owned table (a layout moves the layouted count, not the quantity)",
                    ResponseKind::HousingLayout.name()
                );
                self.commit();
            }
            None => info!(
                "[server] {} for site {mysekai_site_id} ({site_type}): {count} rows accepted and not stored (the served layouts hold only the home site's row)",
                ResponseKind::HousingLayout.name()
            ),
        }
        self.respond(ResponseKind::HousingLayout, false, &[], masters);
        SuiteUserReply {
            success: true,
            refusal: None,
            updated: SuiteUserSections::default(),
        }
    }
}

