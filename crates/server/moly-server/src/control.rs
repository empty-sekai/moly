//! The control port's methods: the document and schema views, one edit, a
//! seed install and the open sessions. They serve the JSON-RPC transport the
//! host runs on its loopback control port; the player socket's
//! `Call::Admin` answers the same questions for the game's own director.

use bevy::prelude::*;
use bevy_remote::{BrpError, BrpResult, RemotePlugin};
use moly_protocol::{AdminCall, UserId};
use serde_json::{json, Value};

use crate::admin;
use crate::defaults::Defaults;
use crate::hub::Sessions;
use crate::masters::{Masters, Tables};
use crate::users::{UserEntry, Users};
use moly_masters::load::MasterTables;

/// The server's remote-procedure plugin: the protocol's built-in methods and
/// the `moly.server.*` control methods. The host adds it together with the
/// HTTP transport.
pub fn remote_plugin() -> RemotePlugin {
    RemotePlugin::default()
        .with_method_main("moly.server.document", document)
        .with_method_main("moly.server.schema", schema)
        .with_method_main("moly.server.edit", edit)
        .with_method_main("moly.server.install", install)
        .with_method_main("moly.server.sessions", sessions)
}

/// The user a request names (`params.user`), the local user when it names
/// none.
fn user_of(params: &Option<Value>) -> UserId {
    params
        .as_ref()
        .and_then(|value| value.get("user"))
        .and_then(Value::as_str)
        .map_or_else(UserId::local, |name| UserId(name.to_owned()))
}

fn no_document(user: &UserId) -> BrpError {
    BrpError::internal(format!("user {user} has no document"))
}

fn json_of(text: String) -> BrpResult {
    serde_json::from_str(&text)
        .map_err(|error| BrpError::internal(format!("the view is not JSON: {error}")))
}

fn refusal(error: moly_protocol::Refusal) -> BrpError {
    BrpError::internal(format!("{}: {}", error.code, error.reason))
}

/// `moly.server.document`: the document view, the same one `Call::Admin`
/// answers with `Document`.
fn document(
    In(params): In<Option<Value>>,
    users: Res<Users>,
    masters: Res<Masters>,
    shared: Option<Res<MasterTables>>,
) -> BrpResult {
    let user = user_of(&params);
    let Some(model) = users.0.get(&user).and_then(|entry| entry.model.as_ref()) else {
        return Err(no_document(&user));
    };
    let tables = Tables::new(&masters, shared.as_deref());
    json_of(admin::document_view(model, &tables))
}

/// `moly.server.schema`: the editable fields of the document.
fn schema(
    In(params): In<Option<Value>>,
    users: Res<Users>,
    masters: Res<Masters>,
    shared: Option<Res<MasterTables>>,
) -> BrpResult {
    let user = user_of(&params);
    let Some(model) = users.0.get(&user).and_then(|entry| entry.model.as_ref()) else {
        return Err(no_document(&user));
    };
    let tables = Tables::new(&masters, shared.as_deref());
    json_of(admin::schema_view(model, &tables))
}

/// `moly.server.edit`: one `server.edit` command, under `params.command` or as
/// the params themselves. The
/// answer says whether it was applied and carries the receipt or the refusal
/// text, so a caller reads one shape in both cases.
fn edit(
    In(params): In<Option<Value>>,
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    shared: Option<Res<MasterTables>>,
) -> BrpResult {
    let tables = Tables::new(&masters, shared.as_deref());
    let params = params.unwrap_or(Value::Null);
    let user = user_of(&Some(params.clone()));
    // Either `{"command": {..}}` or the edit's own fields at the top level,
    // as the game's `moly.server.edit` takes them.
    let mut command = match params.get("command").and_then(Value::as_object) {
        Some(command) => command.clone(),
        None => params
            .as_object()
            .cloned()
            .ok_or_else(|| BrpError::internal("moly.server.edit needs a command object"))?,
    };
    command.remove("user");
    command
        .entry("type".to_owned())
        .or_insert_with(|| Value::String("server.edit".to_owned()));
    let Some(model) = users
        .0
        .get_mut(&user)
        .and_then(|entry| entry.model.as_mut())
    else {
        return Err(no_document(&user));
    };
    match admin::answer(
        AdminCall::Edit {
            command_json: Value::Object(command).to_string(),
        },
        model,
        &tables,
    ) {
        Ok(text) => Ok(json!({
            "ok": true,
            "receipt": serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
        })),
        Err(error) => Ok(json!({
            "ok": false,
            "code": error.code,
            "refusal": error.reason,
        })),
    }
}

/// `moly.server.install`: install the seed's documents into a user, creating
/// the user when it has none yet.
fn install(
    In(params): In<Option<Value>>,
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    defaults: Option<Res<Defaults>>,
    shared: Option<Res<MasterTables>>,
) -> BrpResult {
    let tables = Tables::new(&masters, shared.as_deref());
    let params = params.unwrap_or(Value::Null);
    let user = user_of(&Some(params.clone()));
    let server_json = params
        .get("server_json")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let local_json = params.get("local_json").and_then(Value::as_str);
    let entry = users.0.entry(user.clone()).or_insert_with(|| UserEntry {
        model: None,
        load_error: None,
    });
    let text = admin::install(
        entry,
        server_json,
        local_json,
        defaults.as_deref(),
        &tables,
    )
    .map_err(refusal)?;
    json_of(text)
}

/// `moly.server.sessions`: every user the server keeps, its open sessions and
/// its document revision.
fn sessions(
    In(_params): In<Option<Value>>,
    users: Res<Users>,
    open: Res<Sessions>,
) -> BrpResult {
    let rows: Vec<Value> = users
        .0
        .iter()
        .map(|(user, entry)| {
            json!({
                "user": user.to_string(),
                "openSessions": open.sessions_of(user).count(),
                "revision": entry.model.as_ref().map_or(0, |model| model.revision()),
            })
        })
        .collect();
    Ok(Value::Array(rows))
}
