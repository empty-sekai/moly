//! The home actions' requests: craft and canvas (`PostUserMysekaiCraftApi`,
//! `UserMysekaiCraftRequest` {blueprintId, textureId, cardId,
//! isSpecialTraining, quantity}) and sketch (`PostUserMysekaiHousingSketchApi`,
//! `UserMysekaiHousingSketchRequest` {mysekaiOwnerUserId, mysekaiSiteId,
//! mysekaiBlueprintId}). Each replies with the user data it updated
//! (`SuiteUser`); the client starts the world sequence once the reply
//! succeeds, and builds the result dialogs from its own copies (the craft
//! result's first-craft bonus is the master `mysekaiRankObtainedExps` row, the
//! sketch result's blueprint is the one it asked for). The client sends a
//! request through [`super::client::home_action::post`]; the server plugin
//! installs [`handle`] as its endpoint.
//!
//! **Named policy** (the server's rules are not on disk):
//! `policies.homeActionReply`: `success` (the product default) answers every
//! request with success and the sections pending since the last response;
//! `failure` refuses every request, as the server would when the request is
//! not valid (the client shows the API error and starts nothing).
//!
//! **Not answered here** (named): the craft's material spend, fixture grant
//! and experience, and the sketch's blueprint. The requests the home actions
//! send carry no blueprint; the craft and sketch requests that carry one are
//! [`super::craft`]'s. Natively, `MOLY_CRAFT_MOCK_REQUEST` names a blueprint
//! for the home action's craft request, which [`super::craft`] then answers.

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::client::home_action::{HomeActionApi, HomeActionPost, HomeActionReply};
use super::{ResponseKind, ServerModel};

pub(crate) const POLICY_KEY: &str = "homeActionReply";
pub(crate) const REPLY_SUCCESS: &str = "success";
pub(crate) const REPLY_FAILURE: &str = "failure";

fn index(api: HomeActionApi) -> usize {
    match api {
        HomeActionApi::Craft => 0,
        HomeActionApi::Canvas => 1,
        HomeActionApi::Sketch => 2,
    }
}

fn kind(api: HomeActionApi) -> ResponseKind {
    match api {
        HomeActionApi::Craft => ResponseKind::HomeActionCraft,
        HomeActionApi::Canvas => ResponseKind::HomeActionCanvas,
        HomeActionApi::Sketch => ResponseKind::HomeActionSketch,
    }
}

/// `policies.homeActionReply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum HomeActionReplyPolicy {
    #[default]
    Success,
    Failure,
}

pub(crate) fn parse_policy(value: &Value) -> Result<HomeActionReplyPolicy, String> {
    match value.as_str() {
        Some(REPLY_SUCCESS) => Ok(HomeActionReplyPolicy::Success),
        Some(REPLY_FAILURE) => Ok(HomeActionReplyPolicy::Failure),
        _ => Err(format!(
            "policies.{POLICY_KEY} is neither \"{REPLY_SUCCESS}\" nor \"{REPLY_FAILURE}\""
        )),
    }
}

/// The policy from a document's `policies` (the default when absent).
pub(crate) fn parse(policies: &Map<String, Value>) -> Result<HomeActionReplyPolicy, String> {
    policies
        .get(POLICY_KEY)
        .map_or(Ok(HomeActionReplyPolicy::default()), parse_policy)
}

pub(crate) fn write(policy: HomeActionReplyPolicy, policies: &mut Map<String, Value>) {
    policies.insert(
        POLICY_KEY.into(),
        json!(match policy {
            HomeActionReplyPolicy::Success => REPLY_SUCCESS,
            HomeActionReplyPolicy::Failure => REPLY_FAILURE,
        }),
    );
}

/// A `server.edit` of the policy (live); `None` for another path.
pub(crate) fn edit_path(
    policy: &mut HomeActionReplyPolicy,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        ["policies", POLICY_KEY] => Some(parse_policy(value).map(|parsed| *policy = parsed)),
        _ => None,
    }
}

impl ServerModel {
    fn home_action(&mut self, api: HomeActionApi, target: Option<&str>) -> HomeActionReply {
        let count = &mut self.home_action_replies[index(api)];
        *count += 1;
        let count = *count;
        match self.doc.home_action_reply {
            HomeActionReplyPolicy::Failure => {
                info!(
                    "[server] {} for {}: refused (policies.{POLICY_KEY} = {REPLY_FAILURE}; request {count})",
                    kind(api).name(),
                    target.unwrap_or("no fixture")
                );
                HomeActionReply { success: false }
            }
            HomeActionReplyPolicy::Success => {
                if api == HomeActionApi::Craft {
                    if let Some(request) = super::craft::instrument_request() {
                        let success = self.instrument_craft(&request);
                        info!(
                            "[server] {} for {}: MOLY_CRAFT_MOCK_REQUEST {request:?} answered by the craft policies: success {success} (request {count})",
                            kind(api).name(),
                            target.unwrap_or("no fixture")
                        );
                        return HomeActionReply { success };
                    }
                }
                info!(
                    "[server] {} for {}: success (request {count}); the reply carries the pending sections; material spend, fixture inventory, experience and the sketch blueprint are not modelled (the request carries no blueprint)",
                    kind(api).name(),
                    target.unwrap_or("no fixture")
                );
                self.respond(kind(api), false, &[]);
                HomeActionReply { success: true }
            }
        }
    }
}

/// The endpoint the server plugin installs for the client's requests
/// ([`super::client::home_action::post`]). Refused while no server has joined
/// the client.
pub(super) fn handle(In(request): In<HomeActionPost>) -> HomeActionReply {
    let HomeActionPost { api, target } = request;
    let target = target.as_deref();
    super::with_model(|model| {
        if !model.joined {
            warn!(
                "[server] {} refused: the server has not joined the client yet",
                kind(api).name()
            );
            return HomeActionReply { success: false };
        }
        model.home_action(api, target)
    })
    .unwrap_or_else(|| {
        warn!(
            "[server] {} refused: the server model is not installed",
            kind(api).name()
        );
        HomeActionReply { success: false }
    })
}

/// The panel's section.
pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": "homeAction",
        "title": "Craft, canvas and sketch replies",
        "delivery": "live",
        "fields": [{
            "path": format!("policies.{POLICY_KEY}"),
            "type": "enum",
            "values": [
                {"value": REPLY_SUCCESS, "label": "every request succeeds and carries the pending sections"},
                {"value": REPLY_FAILURE, "label": "every request is refused (the client shows the API error)"},
            ],
            "note": "material spend, fixture inventory, craft experience and the sketch blueprint are not modelled: the requests carry no blueprint",
        }],
    }])
}

pub(crate) fn schema_policies() -> Value {
    json!([{
        "name": "home action reply",
        "rule": "success answers craft, canvas and sketch with success and the pending sections; failure refuses them",
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_follows_the_policy() {
        let mut model = super::super::tests::model();
        model.joined = true;
        assert!(model.home_action(HomeActionApi::Craft, Some("fx")).success);
        assert_eq!(model.responses.len(), 1);
        model.doc.home_action_reply = HomeActionReplyPolicy::Failure;
        assert!(!model.home_action(HomeActionApi::Sketch, None).success);
        assert_eq!(model.responses.len(), 1);
        assert_eq!(model.home_action_replies, [1, 0, 1]);
        let mut policies = Map::new();
        write(HomeActionReplyPolicy::Failure, &mut policies);
        assert_eq!(parse(&policies).unwrap(), HomeActionReplyPolicy::Failure);
        assert_eq!(parse(&Map::new()).unwrap(), HomeActionReplyPolicy::Success);
        assert!(parse_policy(&json!("maybe")).is_err());
    }
}
