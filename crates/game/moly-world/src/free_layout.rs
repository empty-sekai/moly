//! The free layout mode: a switch the player keeps for one site, which lifts
//! four rules of the site's layout together.
//!
//! - A home site layout that has been saved may hold no house: the starter
//!   house is appended only to a first visit, whose layout was never saved.
//! - The house and the gates are stored like any other fixture.
//! - Any number of homes and gates may stand; the house a join enters through
//!   stays the first one of the layout, as it always is.
//! - A placement does not need an owned copy.
//!
//! The switch is a property of the server document, because the document is
//! what a session starts from; it is kept for the site it was switched on at.
//! The game keeps its part of the document in the local list (the section
//! `SECTION`), which every writer of the document merges rather than
//! replaces: a layout save writes the layout section only and never drops the
//! switch. The extension the source server has no row for is
//! `molyExtensions.freeLayout` in the server's own document (the game writes
//! its row beside a layout save, so a client that reads the server's copy
//! holds it), and a site's mode is on when either names it.

use bevy::prelude::*;
use serde_json::{Map, Value};

use crate::settings_store;

/// The local list's section of the switches: one row per site id, `true`
/// when the free layout mode is on there.
const SECTION: &str = "MolyFreeLayouts";

/// The free layout mode of one site.
#[derive(Resource, Default)]
pub struct FreeLayout {
    site_id: u32,
    on: bool,
}

impl FreeLayout {
    /// The mode of the document's section for `site_id`: on only for a row
    /// that names the site and is `true`.
    pub fn read(document: &Value, site_id: u32) -> Self {
        Self {
            site_id,
            on: in_document(document, site_id) || server_document_row(document, site_id),
        }
    }

    pub fn site_id(&self) -> u32 {
        self.site_id
    }

    /// Whether the site's rules are lifted.
    pub fn is_on(&self) -> bool {
        self.on
    }
}

/// The document change one switch writes: the section with the site's row.
fn sites_row(site_id: u32, on: bool) -> Value {
    let mut rows = Map::new();
    rows.insert(site_id.to_string(), Value::Bool(on));
    Value::Object(rows)
}

/// The server document's own switch as one change writes it.
fn server_patch(site_id: u32, on: bool) -> (&'static str, Value) {
    let mut switch = Map::new();
    switch.insert(site_id.to_string(), Value::Bool(on));
    let mut extensions = Map::new();
    extensions.insert("freeLayout".to_owned(), Value::Object(switch));
    ("molyExtensions", Value::Object(extensions))
}

/// Writes the site's switch into the server's document. A refused write leaves
/// the document as it was and names why; the site's layout save that follows
/// then goes without the row, and the game's own switch still holds.
pub fn set_server_document(site_id: u32, on: bool) -> Result<(), String> {
    settings_store::save_sections(&[server_patch(site_id, on)])
}

/// The document utilities' read of the switch: the local list's row, the
/// server document's own row, `off` for a site neither names, and a refused
/// read for a section that is not an object (never an off).
pub(crate) fn mode_in(document: &Value, site_id: u32) -> Result<bool, String> {
    let local = read_in(document, site_id, "the document")?;
    let server = server_read(document, site_id, "the document")?;
    Ok(local || server)
}

/// Whether the local list holds the mode on for `site_id` (a malformed
/// section reads as off; the writers read it with `read_in`).
pub fn in_document(document: &Value, site_id: u32) -> bool {
    document
        .get(SECTION)
        .and_then(|section| section.get(site_id.to_string()))
        .is_some_and(|switch| switch.as_bool() == Some(true))
}

/// Whether the server document names `site_id` with the free layout mode on.
pub fn server_document_row(document: &Value, site_id: u32) -> bool {
    document
        .get("molyExtensions")
        .and_then(|extensions| extensions.get("freeLayout"))
        .and_then(|row| row.get(site_id.to_string()))
        .is_some_and(|switch| switch.as_bool() == Some(true))
}

/// Reads the switch for one site from the server document's extension. A
/// section naming no site as a switch is malformed and refused, never read
/// as off.
pub(crate) fn server_read(document: &Value, site_id: u32, at: &str) -> Result<bool, String> {
    let Some(extensions) = document.get("molyExtensions") else {
        return Ok(false);
    };
    let switch = match extensions.get("freeLayout") {
        None => return Ok(false),
        Some(switch) => switch,
    };
    match switch {
        Value::Object(rows) => rows
            .get(&site_id.to_string())
            .map(|row| {
                row.as_bool()
                    .ok_or_else(|| format!("{at}.molyExtensions.freeLayout.{site_id} is not true or false"))
            })
            .unwrap_or(Ok(false)),
        _ => Err(format!("{at}.molyExtensions.freeLayout is not an object")),
    }
}

/// Reads the local list's switch for one site. A section naming no site as a
/// switch is malformed and refused, never read as off.
pub(crate) fn read_in(document: &Value, site_id: u32, at: &str) -> Result<bool, String> {
    match document.get(SECTION) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Object(rows)) => rows
            .get(&site_id.to_string())
            .map(|row| {
                row.as_bool()
                    .ok_or_else(|| format!("{at}.{SECTION}.{site_id} is not true or false"))
            })
            .unwrap_or(Ok(false)),
        Some(_) => Err(format!("{at}.{SECTION} is not an object")),
    }
}

/// Saves the switch into the local list and installs it for its site. A
/// refused write leaves the document and the mode as they were and names why.
///
/// The write is one document change: a layout save that runs its own change
/// under the same store lock applies both or neither, so neither write drops
/// the other's switch.
pub fn set(world: &mut World, site_id: u32, on: bool) -> Result<(), String> {
    settings_store::save_sections(&[(SECTION, sites_row(site_id, on))])?;
    world.insert_resource(FreeLayout { site_id, on });
    info!(
        "[layout] free layout {} for site {site_id}",
        if on { "on" } else { "off" }
    );
    Ok(())
}

/// The switch as the resource holds it (its site and whether it is on), read
/// from the document when no switch has been made since the start.
pub fn mode(world: &World) -> Result<(u32, bool), String> {
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .ok_or_else(|| "no site is active".to_owned())?;
    if let Some(held) = world
        .get_resource::<FreeLayout>()
        .filter(|held| held.site_id == site.site_id)
    {
        return Ok((held.site_id, held.on));
    }
    let document = settings_store::read_document()?;
    let local = read_in(&document, site.site_id, "the settings document")?;
    let server = server_read(&document, site.site_id, "the settings document")?;
    Ok((site.site_id, local || server))
}
