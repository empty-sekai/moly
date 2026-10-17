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
//! [`handle`] is the endpoint the server model still registers until it is
//! removed; the client request itself is one `Call::SystemFixtureAction` on
//! the server link (see [`crate::cutscene::birthday`]).

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

use crate::server::with_model;

/// The API's name in the source.
pub(crate) const API: &str = "PostUserMysekaiSystemFixtureActionApi";

pub(crate) use moly_protocol::rows::UserMysekaiSystemFixtureAction;

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

pub(crate) use moly_protocol::api::SystemFixtureActionReply;

/// Refusal constructor for [`SystemFixtureActionReply`].
pub(crate) trait SystemFixtureActionReplyExt {
    /// Logs the refusal and builds the reply that carries it.
    fn refused(reason: String) -> Self;
}

impl SystemFixtureActionReplyExt for SystemFixtureActionReply {
    fn refused(reason: String) -> Self {
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

/// The endpoint the server plugin installs for the request: the reply's
/// `updatedResources` are merged into the client's copy before it returns.
pub(crate) fn handle(
    In(mysekai_system_fixture_id): In<i32>,
    mut copy: ResMut<ClientSystemFixtureActions>,
) -> SystemFixtureActionReply {
    let answer = with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let obtained = model.system_fixture_action(mysekai_system_fixture_id)?;
        let response = model.responses.last().expect("the response just recorded");
        Ok((
            obtained,
            response.sections.system_fixture_actions.clone(),
        ))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()));
    match answer {
        Ok((obtained_resources, rows)) => {
            if let Some(rows) = rows {
                copy.apply(rows);
            }
            SystemFixtureActionReply {
                success: true,
                refusal: None,
                obtained_resources,
            }
        }
        Err(reason) => SystemFixtureActionReply::refused(reason),
    }
}

/// The endpoint and the client's copy.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ClientSystemFixtureActions>();
    let endpoint = app.world_mut().register_system(handle);
    app.insert_resource(SystemFixtureActionEndpoint(endpoint));
}
