//! The administration calls: the document and schema views, an edit, and the
//! browser game's seed install. Their replies are JSON text.

use bevy::prelude::*;
use moly_protocol::{AdminCall, Refusal};

use crate::defaults::Defaults;
use crate::masters::Masters;
use crate::model::document;
use crate::model::{edit, Origin, ServerModel};
use crate::users::UserEntry;

/// The document with its clock, the pending sections and the missing
/// masters: what the control plane reads.
pub(crate) fn document_view(model: &ServerModel, masters: &Masters) -> String {
    serde_json::json!({
        "schemaVersion": document::SCHEMA_VERSION,
        "origin": model.origin.name(),
        "revision": model.revision,
        "serverNowMs": model.now_ms(),
        "document": model.doc.to_value(&model.pending),
        "pending": model.pending,
        "joined": model.joined,
        "missingMasters": masters.missing,
        "homeActionReplies": {
            "craft": model.home_action_replies[0],
            "canvas": model.home_action_replies[1],
            "sketch": model.home_action_replies[2],
        },
        "musicPlayReplies": {
            "set": model.music_play_replies[0],
            "eject": model.music_play_replies[1],
        },
        "errors": model.errors,
    })
    .to_string()
}

/// The editable fields of the document.
pub(crate) fn schema_view(model: &ServerModel, masters: &Masters) -> String {
    edit::schema(model, masters).to_string()
}

/// The schema of a server that has no document yet.
pub(crate) fn schema_without_model() -> String {
    edit::schema_without_model().to_string()
}

/// One administration call answered, as JSON text.
pub(crate) fn answer(
    call: AdminCall,
    model: &mut ServerModel,
    masters: &Masters,
) -> Result<String, Refusal> {
    match call {
        AdminCall::Schema => Ok(schema_view(model, masters)),
        AdminCall::Document => Ok(document_view(model, masters)),
        AdminCall::Edit { command_json } => {
            let command: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&command_json).map_err(|error| {
                    Refusal::new(
                        "edit_not_json",
                        format!("server.edit: {command_json:?} is not a JSON object: {error}"),
                    )
                })?;
            model
                .edit(&command, masters)
                .map(|receipt| receipt.to_string())
                .map_err(|reason| Refusal::new("edit_refused", reason))
        }
        // The install replaces the model; the dispatch handles it.
        AdminCall::Install { .. } => Err(Refusal::new(
            "install_not_here",
            "the seed install replaces the user's document",
        )),
    }
}

/// The browser game's seed: the server document replaces the user's, the
/// local document is the page's own (the server keeps no client-local lists).
pub(crate) fn install(
    entry: &mut UserEntry,
    server_json: Option<String>,
    local_json: Option<&str>,
    defaults: Option<&Defaults>,
    masters: &Masters,
) -> Result<String, Refusal> {
    let (doc, origin, pending) = match server_json {
        Some(text) => {
            let (doc, pending) = document::parse_v2(&text).map_err(|reason| {
                Refusal::new(
                    "install_refused",
                    format!("the seed's server document is refused: {reason}"),
                )
            })?;
            (doc, Origin::Seed, pending)
        }
        None => match defaults {
            Some(defaults) => (defaults.doc.clone(), Origin::SeedDefault, Vec::new()),
            None => {
                return Err(Refusal::new(
                    "install_not_ready",
                    "the product default document is still loading",
                ))
            }
        },
    };
    let mut model = ServerModel::new(doc, origin, pending);
    // The installed seed is the user's document from here on.
    model.changed = true;
    if let Some(defaults) = defaults {
        model.avatar_instrument = defaults.avatar_instrument.clone();
    }
    if masters.ready {
        // The install answers with its own JSON; the sections of a join are
        // not carried here, so the next join answers with them.
        model.join(masters);
        model.responses.clear();
    }
    entry.model = Some(model);
    entry.load_error = None;
    if let Some(local) = local_json {
        info!(
            "[server] seed install: the local document ({} bytes) belongs to the page; the server keeps no client-local lists",
            local.len()
        );
    }
    Ok(serde_json::json!({
        "installed": true,
        "origin": origin.name(),
        "localDocument": local_json.is_some(),
    })
    .to_string())
}
