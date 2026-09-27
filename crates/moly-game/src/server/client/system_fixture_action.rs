//! The system fixture action as the client holds it.
//!
//! - `UserDataManager.UserMysekaiSystemFixtureActions`
//!   (`UserMysekaiSystemFixtureAction` {userId, mysekaiSystemFixtureId}):
//!   [`ClientSystemFixtureActions`], set by the server model's responses
//!   only. `UpdateUserMysekaiSystemFixtureActions(data)` replaces the whole
//!   array when a response carries one (a null keeps the copy), and
//!   `IsActionedSystemFixture(id)` is false for a null or empty array, else
//!   whether any row names the id. The user id is the session's and is not
//!   kept.
//! - `MysekaiSystemFixtureActionService.ExecuteAPI(systemFixtureId)`:
//!   `PostUserMysekaiSystemFixtureActionApi(userId, mysekaiSystemFixtureId)`,
//!   an empty body. The reply (`UserMysekaiSystemFixtureActionResponse`)
//!   carries `updatedResources` (`SuiteUser`) and `obtainedResources`
//!   (`UserResource` rows); on success `OnFinishAPI` merges
//!   `updatedResources` (`UserDataManager.UpdateAll`) before the caller
//!   reads the copies. The birthday party's `ExecuteAsync` stops when the
//!   reply is null and keeps `obtainedResources` for its reward dialog.
//!
//! [`post`] runs [`SystemFixtureActionEndpoint`], a one-shot system the
//! server model installs; when it returns, the reply's rows are already in
//! [`ClientSystemFixtureActions`]. Without an endpoint the request is
//! refused by name.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use serde_json::{json, Value};

use super::talk_read::UserResource;

/// The API's name in the source.
pub(crate) const API: &str = "PostUserMysekaiSystemFixtureActionApi";

/// `UserMysekaiSystemFixtureAction` (the user id is the session's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiSystemFixtureAction {
    pub(crate) mysekai_system_fixture_id: i32,
}

/// The rows as the response key's JSON array.
pub(crate) fn rows_value(rows: &[UserMysekaiSystemFixtureAction]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| json!({"mysekaiSystemFixtureId": row.mysekai_system_fixture_id}))
            .collect(),
    )
}

/// The client's copy of `userMysekaiSystemFixtureActions`, set by responses
/// only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientSystemFixtureActions {
    rows: Vec<UserMysekaiSystemFixtureAction>,
    /// Responses that carried the section.
    pub(crate) revision: u64,
}

impl ClientSystemFixtureActions {
    /// `UserDataManager.IsActionedSystemFixture(systemFixtureId)`.
    pub(crate) fn is_actioned(&self, mysekai_system_fixture_id: i32) -> bool {
        self.rows
            .iter()
            .any(|row| row.mysekai_system_fixture_id == mysekai_system_fixture_id)
    }

    /// `UpdateUserMysekaiSystemFixtureActions`: a response's rows replace
    /// the copy.
    pub(crate) fn apply(&mut self, rows: Vec<UserMysekaiSystemFixtureAction>) {
        self.rows = rows;
        self.revision += 1;
    }
}

/// `UserMysekaiSystemFixtureActionResponse`, or a refusal (the source's
/// null reply: the API error, and the caller stops).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SystemFixtureActionReply {
    pub(crate) success: bool,
    /// Why the server refused (`None` on success).
    pub(crate) refusal: Option<String>,
    /// `obtainedResources`.
    pub(crate) obtained_resources: Vec<UserResource>,
}

impl SystemFixtureActionReply {
    pub(crate) fn refused(reason: String) -> Self {
        warn!("[server] {API} refused: {reason}");
        Self {
            success: false,
            refusal: Some(format!("{API}: {reason}")),
            obtained_resources: Vec::new(),
        }
    }
}

/// The server model's endpoint for the request (the system fixture id).
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct SystemFixtureActionEndpoint(
    pub(crate) SystemId<In<i32>, SystemFixtureActionReply>,
);

/// `MysekaiSystemFixtureActionService.ExecuteAPI(systemFixtureId)`: send
/// the request and read its reply.
pub(crate) fn post(world: &mut World, mysekai_system_fixture_id: i32) -> SystemFixtureActionReply {
    let Some(endpoint) = world.get_resource::<SystemFixtureActionEndpoint>().copied() else {
        return SystemFixtureActionReply::refused("no server endpoint is installed".into());
    };
    world
        .run_system_with(endpoint.0, mysekai_system_fixture_id)
        .unwrap_or_else(|error| SystemFixtureActionReply::refused(error.to_string()))
}
