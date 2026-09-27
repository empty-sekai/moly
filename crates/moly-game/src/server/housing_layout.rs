//! `PostUserMysekaiHousingLayoutApi` (`UserMysekaiHousingEditLayoutRequest`
//! {mysekaiSiteId, userMysekaiSiteHousingLayout, mysekaiPhenomenaId}): the
//! layout editor's save. The server stores the site's layout and replies with
//! the user data it changed (`SuiteUser`). The client sends it through
//! [`super::client::housing_layout::post`]; the server plugin installs
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
use serde_json::Value;

use super::client::craft::SuiteUserReply;
use super::client::housing_layout::{API, UserMysekaiHousingEditLayoutRequest};
use super::client::inventory::SuiteUserSections;
use super::{ResponseKind, ServerModel};

/// The fields a served row needs, with a check of each.
fn check_row(row: &Value, index: usize) -> Result<(), String> {
    let at = format!("row {index}");
    let object = row
        .as_object()
        .ok_or_else(|| format!("{at} is not an object"))?;
    let has = |key: &str, ok: fn(&Value) -> bool| {
        object
            .get(key)
            .filter(|value| ok(value))
            .map(|_| ())
            .ok_or_else(|| format!("{at}: {key} is missing or malformed"))
    };
    has("package", Value::is_string)?;
    has("mysekaiFixtureId", Value::is_i64)?;
    has("minimum", Value::is_object)?;
    has("maximum", Value::is_object)?;
    has("centerY", Value::is_i64)?;
    has("layoutType", Value::is_u64)?;
    has("rotation", Value::is_u64)?;
    has("textureId", Value::is_u64)?;
    Ok(())
}

impl ServerModel {
    fn housing_layout(&mut self, request: UserMysekaiHousingEditLayoutRequest) -> SuiteUserReply {
        let UserMysekaiHousingEditLayoutRequest {
            mysekai_site_id,
            site_type,
            fixtures,
        } = request;
        if let Err(error) = fixtures
            .iter()
            .enumerate()
            .try_for_each(|(index, row)| check_row(row, index))
        {
            warn!("[server] {API} for site {mysekai_site_id} ({site_type}) refused: {error}");
            return SuiteUserReply::refused(format!("{API}: {error}"));
        }
        let count = fixtures.len();
        let served = self.doc.layouts.as_array_mut().and_then(|rows| {
            rows.iter_mut()
                .find(|row| row["siteType"].as_str() == Some(site_type.as_str()))
        });
        match served {
            Some(row) => {
                row["mysekaiFixtures"] = Value::Array(fixtures);
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
        self.respond(ResponseKind::HousingLayout, false, &[]);
        SuiteUserReply {
            success: true,
            refusal: None,
            updated: SuiteUserSections::default(),
        }
    }
}

/// The endpoint the server plugin installs for the client's requests.
/// Refused while no server has joined the client.
pub(super) fn handle(In(request): In<UserMysekaiHousingEditLayoutRequest>) -> SuiteUserReply {
    super::with_model(|model| {
        if !model.joined {
            warn!("[server] {API} refused: the server has not joined the client yet");
            return SuiteUserReply::refused(format!(
                "{API}: the server has not joined the client yet"
            ));
        }
        model.housing_layout(request)
    })
    .unwrap_or_else(|| {
        warn!("[server] {API} refused: the server model is not installed");
        SuiteUserReply::refused(format!("{API}: the server model is not installed"))
    })
}
