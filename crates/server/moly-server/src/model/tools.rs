//! `userMysekaiTools`: the user's harvest tools in the server document, the
//! one list the harvest replies and the craft replies both change.
//!
//! **Document key** (the source's `SuiteUser` key, optional on read, always
//! written): `userMysekaiTools`, `UserMysekaiTool` rows (`mysekaiToolId`,
//! `quantity`, `durability`, `lastObtainedAt`), one per tool. `null` (or an
//! absent key) is a user the server has not seated yet.
//!
//! **Named policies** (the server's rules are not in the client):
//! - *Initial tools* (the mock's choice): a user without tool rows gets, at
//!   the join, the lowest-id level 1 `pickaxe` and `axe` rows of the tool
//!   master, quantity 1 at the master `maxDurability`, `lastObtainedAt` at
//!   the server clock.
//! - *Tool use* (the harvest reply): the used tool loses the request's use
//!   count; while its durability is at or below 0 and a quantity is left, one
//!   of the quantity is consumed and the durability goes up by the master
//!   `maxDurability`; a row whose quantity reaches 0 leaves the list. The
//!   client counts the same way (`DecreaseCountAndResetDurability`); a
//!   durability that differs from the one the client sent is logged.
//! - *Tool craft* (the craft reply for a `mysekai_tool` blueprint): the row
//!   of the craft target gains the crafted quantity and its `lastObtainedAt`
//!   becomes the server clock; a new row starts at the master
//!   `maxDurability`. The craft screen shows that row's quantity
//!   (`CraftPreview.SetToolQuantityText`).
//!
//! The client's copy is [`ClientMysekaiTools`]: the responses that carry the
//! section set it, and the craft reply sets it at once (the API executor
//! merges a reply before its caller reads the user data). The harvest's
//! player model follows it.
//!
//! Native instrument: `MOLY_HARVEST_MOCK_TOOLS` (`toolId:quantity:durability`,
//! comma separated) states the native document's rows; game mode reads no
//! environment variable.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::document::ServerDocument;
use super::{Masters, ResponseKind, ServerModel};

pub(crate) use moly_protocol::json::SECTION_TOOLS as SECTION;
const INSTRUMENT: &str = "MOLY_HARVEST_MOCK_TOOLS";
/// The tool types the initial tools policy seats, in seating order.
const INITIAL_TYPES: [&str; 2] = ["pickaxe", "axe"];

pub(crate) use moly_masters::tools::ToolMaster;
pub(crate) use moly_protocol::json::tools_section_value as value;
pub(crate) use moly_protocol::rows::UserMysekaiTool;

/// `mysekaiTools`, id order.
pub(crate) fn parse_master(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.tools = Some(moly_masters::tools::parse_tool_master(
        &moly_assets::json::master::rows(text)?,
    )?);
    Ok(())
}

fn max_durability(masters: &Masters, tool_id: i32) -> Option<i32> {
    masters
        .tools
        .as_ref()?
        .iter()
        .find(|row| row.id == tool_id)
        .map(|row| row.max_durability)
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

/// The rows of a section value (`null`: not seated).
pub(crate) fn parse_rows(value: &Value) -> Result<Option<Vec<UserMysekaiTool>>, String> {
    moly_protocol::json::parse_tools(value)
}

/// This part from a schemaVersion 2 document (absent: not seated).
pub(crate) fn parse(doc: &Map<String, Value>) -> Result<Option<Vec<UserMysekaiTool>>, String> {
    match doc.get(SECTION) {
        None => Ok(None),
        Some(value) => parse_rows(value),
    }
}

/// The checks that need no master data: a tool id of at least 1 once per
/// list, a quantity of at least 1 (a used-up row leaves the list) and a
/// durability of at least 1.
pub(crate) fn check_structure(rows: &Option<Vec<UserMysekaiTool>>) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for (i, row) in rows.iter().flatten().enumerate() {
        let at = format!("{SECTION}[{i}]");
        if row.mysekai_tool_id < 1 {
            return Err(format!(
                "{at}.mysekaiToolId = {} is below 1",
                row.mysekai_tool_id
            ));
        }
        if row.quantity < 1 {
            return Err(format!(
                "{at}.quantity = {} is below 1 (a used-up tool leaves the list)",
                row.quantity
            ));
        }
        if row.durability < 1 {
            return Err(format!("{at}.durability = {} is below 1", row.durability));
        }
        if !ids.insert(row.mysekai_tool_id) {
            return Err(format!("{at} repeats tool {}", row.mysekai_tool_id));
        }
    }
    Ok(())
}

/// The checks against the tool master (skipped while it is absent): a master
/// tool, a durability at most its `maxDurability`.
pub(crate) fn check_masters(
    rows: &Option<Vec<UserMysekaiTool>>,
    masters: &Masters,
) -> Result<(), String> {
    if masters.tools.is_none() {
        return Ok(());
    }
    for (i, row) in rows.iter().flatten().enumerate() {
        let max = max_durability(masters, row.mysekai_tool_id).ok_or_else(|| {
            format!(
                "{SECTION}[{i}].mysekaiToolId = {} is not a master tool",
                row.mysekai_tool_id
            )
        })?;
        if row.durability > max {
            return Err(format!(
                "{SECTION}[{i}].durability = {} is above the master maxDurability {max}",
                row.durability
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

/// The initial tools policy's rows (`Err`: the master cannot give them).
pub(crate) fn initial_rows(masters: &Masters, now: i64) -> Result<Vec<UserMysekaiTool>, String> {
    let tools = masters
        .tools
        .as_ref()
        .ok_or("the tool master (mysekaiTools) is absent")?;
    INITIAL_TYPES
        .iter()
        .map(|kind| {
            tools
                .iter()
                .filter(|row| row.tool_type == *kind && row.level == 1)
                .min_by_key(|row| row.id)
                .map(|row| UserMysekaiTool {
                    mysekai_tool_id: row.id,
                    quantity: 1,
                    durability: row.max_durability,
                    last_obtained_at: now,
                })
                .ok_or_else(|| format!("the tool master has no level 1 {kind} row"))
        })
        .collect()
}

/// What a tool use did to the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolUse {
    /// The row after the use (`quantity` 0: it left the list).
    pub(crate) after: UserMysekaiTool,
    /// Quantity consumed by durability resets.
    pub(crate) consumed: i32,
}

/// The tool use policy on a list.
pub(crate) fn use_tool(
    rows: &mut Vec<UserMysekaiTool>,
    tool_id: i32,
    use_count: i32,
    max: i32,
) -> Result<ToolUse, String> {
    let index = rows
        .iter()
        .position(|row| row.mysekai_tool_id == tool_id)
        .ok_or_else(|| format!("tool {tool_id} is not in {SECTION}"))?;
    if max < 1 {
        return Err(format!(
            "tool {tool_id} has a master maxDurability of {max}: no durability to reset to"
        ));
    }
    let row = &mut rows[index];
    row.durability -= use_count;
    let mut consumed = 0;
    while row.durability <= 0 && row.quantity > 0 {
        row.quantity -= 1;
        row.durability += max;
        consumed += 1;
    }
    let after = *row;
    if row.quantity < 1 {
        rows.remove(index);
    }
    Ok(ToolUse { after, consumed })
}

/// The tool craft policy on a list.
pub(crate) fn grant_tool(
    rows: &mut Vec<UserMysekaiTool>,
    tool_id: i32,
    quantity: i32,
    max: i32,
    now: i64,
) -> UserMysekaiTool {
    match rows.iter_mut().find(|row| row.mysekai_tool_id == tool_id) {
        Some(row) => {
            row.quantity += quantity;
            row.last_obtained_at = now;
            *row
        }
        None => {
            let row = UserMysekaiTool {
                mysekai_tool_id: tool_id,
                quantity,
                durability: max,
                last_obtained_at: now,
            };
            rows.push(row);
            row
        }
    }
}

impl ServerModel {
    /// The initial tools policy for a user without tool rows; whether the
    /// document changed.
    pub(super) fn seat_tools(&mut self, masters: &Masters) -> bool {
        if self.doc.tools.is_some() {
            return false;
        }
        match initial_rows(masters, self.now_ms()) {
            Ok(rows) => {
                info!(
                    "[server] initial tools: {:?} (the mock's choice: the lowest-id level 1 pickaxe and axe at their master maxDurability)",
                    rows.iter()
                        .map(|row| (row.mysekai_tool_id, row.quantity, row.durability))
                        .collect::<Vec<_>>()
                );
                self.doc.tools = Some(rows);
                true
            }
            Err(reason) => {
                self.push_error(format!("{SECTION} is not seated: {reason}"));
                false
            }
        }
    }

    /// The master `maxDurability` a crafted tool starts at, checked before
    /// the craft spends anything.
    pub(super) fn crafted_tool_max(&self, tool_id: i32, masters: &Masters) -> Result<i32, String> {
        max_durability(masters, tool_id).ok_or_else(|| match &masters.tools {
            None => "the tool master (mysekaiTools) is absent: the crafted tool cannot be granted"
                .to_owned(),
            Some(_) => format!("craft target {tool_id} is not a master tool"),
        })
    }

    /// The craft reply's grant for a tool blueprint (the tool craft policy).
    pub(super) fn grant_crafted_tool(
        &mut self,
        tool_id: i32,
        quantity: i32,
        max: i32,
        now: i64,
    ) -> UserMysekaiTool {
        let rows = self.doc.tools.get_or_insert_with(Vec::new);
        grant_tool(rows, tool_id, quantity, max, now)
    }

    /// The harvest reply's tools: the tool use policy, answered as a response
    /// that carries the section.
    fn harvest_tool_use(
        &mut self,
        tool_id: i32,
        use_count: i32,
        sent_durability: i32,
        masters: &Masters,
    ) -> Vec<UserMysekaiTool> {
        let outcome = match (
            max_durability(masters, tool_id),
            self.doc.tools.as_mut(),
        ) {
            (_, None) => Err(format!("{SECTION} is not seated")),
            (None, _) => Err(format!(
                "tool {tool_id} has no master row (mysekaiTools) to reset its durability from"
            )),
            (Some(max), Some(rows)) => use_tool(rows, tool_id, use_count, max),
        };
        match outcome {
            Ok(used) => {
                info!(
                    "[server] {}: tool {tool_id} used {use_count}: durability {} quantity {} (consumed {}); {SECTION} rows {}",
                    ResponseKind::Harvest.name(),
                    used.after.durability,
                    used.after.quantity,
                    used.consumed,
                    self.doc.tools.as_ref().map_or(0, Vec::len)
                );
                if used.after.quantity > 0 && used.after.durability != sent_durability {
                    warn!(
                        "[server] {}: tool {tool_id}: server durability {} differs from the sent {sent_durability}",
                        ResponseKind::Harvest.name(),
                        used.after.durability
                    );
                }
                self.commit();
                self.respond(ResponseKind::Harvest, false, &[SECTION], masters);
            }
            Err(reason) => warn!(
                "[server] {}: the tool use is not applied: {reason}",
                ResponseKind::Harvest.name()
            ),
        }
        self.doc.tools.clone().unwrap_or_default()
    }
}


// ---------------------------------------------------------------------------
// Native instrument and the panel
// ---------------------------------------------------------------------------

/// `MOLY_HARVEST_MOCK_TOOLS` over the native document; a malformed entry
/// stops loudly.
pub(crate) fn native_overlay(doc: &mut ServerDocument) {
    let Some(raw) = super::instrument_env(INSTRUMENT) else {
        return;
    };
    let mut rows = Vec::new();
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let parts: Vec<Option<i32>> = entry
            .split(':')
            .map(|part| part.trim().parse::<i32>().ok())
            .collect();
        let [Some(id), Some(quantity), Some(durability)] = parts.as_slice() else {
            panic!("{INSTRUMENT} entry {entry:?} is not toolId:quantity:durability");
        };
        rows.push(UserMysekaiTool {
            mysekai_tool_id: *id,
            quantity: *quantity,
            durability: *durability,
            last_obtained_at: 0,
        });
    }
    let rows = Some(rows);
    if let Err(reason) = check_structure(&rows) {
        panic!("{INSTRUMENT}={raw:?}: {reason}");
    }
    info!(
        "[server] native overlay: {INSTRUMENT} -> {SECTION} {:?}",
        rows
    );
    doc.tools = rows;
}

/// A panel edit of the section (`None`: not this part's path).
pub(crate) fn edit_path(
    tools: &mut Option<Vec<UserMysekaiTool>>,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [SECTION] => Some(parse_rows(value).map(|rows| *tools = rows)),
        _ => None,
    }
}

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": "mysekaiTools",
        "title": "Harvest tools",
        "delivery": "next-response",
        "fields": [
            {"path": SECTION, "type": "rows", "row": {
                "mysekaiToolId": "int", "quantity": "int", "durability": "int", "lastObtainedAt": "epoch-ms"},
                "note": "null: a new user, seated at the join by the initial tools policy; the harvest reply spends durability and the tool craft adds quantity"},
        ],
    }])
}

pub(crate) fn schema_policies() -> Value {
    json!([
        {"name": "initial tools", "rule": "a document without userMysekaiTools gets, at the join, the lowest-id level 1 pickaxe and axe of the tool master, quantity 1 at maxDurability (the mock's choice)"},
        {"name": "tool use", "rule": "the harvest reply takes the use count from the tool's durability; while durability <= 0 and quantity > 0: quantity - 1, durability + master maxDurability; a row at quantity 0 leaves the list (the client's DecreaseCountAndResetDurability counts the same)"},
        {"name": "tool craft", "rule": "a mysekai_tool blueprint adds the crafted quantity to the craft target's row (a new row starts at maxDurability); lastObtainedAt = server clock; no experience (IsFirstCraft is false for tools)"},
    ])
}
