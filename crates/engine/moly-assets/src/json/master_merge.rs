//! The rows of a master table across the regions: the JP rows, then every
//! row an enabled region adds whose key JP does not have.
//!
//! The merge runs on a table's text before its consumer's parser sees it, so
//! a consumer reads one table whatever the regions are. A merged table is a
//! JSON array:
//!
//! - every JP row first, unchanged and in JP order, without a region field;
//! - then the exclusive rows of each enabled non-JP region, in `ENABLED`
//!   order, in that region's order, each with the field [`REGION_FIELD`]
//!   holding the region's directory (`"cn"`);
//! - two non-JP regions that add the same key refuse the whole table,
//!   naming the table, the key and the regions;
//! - a non-JP table that cannot be read gives the JP rows alone, and the
//!   layer that reads the table reports the reason by name.
//!
//! A row's key is one field (`id` by default) or several ([`TABLE_KEYS`]).
//! Key fields are compared as their JSON text, so a string key and a numeric
//! one never meet, and an absent field and a null one are absent. A table
//! whose document is an object, or whose rows lack their key, is not merged:
//! its rows are the JP text's.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::remote::{Region, ENABLED};

/// The field an exclusive row carries to name the region it comes from: the
/// region's directory.
pub const REGION_FIELD: &str = "moly:region";

/// The tables whose key is not `id`: the natural keys upstream declares.
pub const TABLE_KEYS: &[(&str, &[&str])] = &[
    // A text entry's own key, a string.
    ("wordings", &["wordingKey"]),
    // `id` repeats across purposes; a box is its purpose and its id.
    ("resourceBoxes", &["resourceBoxPurpose", "id"]),
    // The id and sequence columns are not in every region's table.
    ("musicTags", &["musicId", "musicTag"]),
];

/// The table's key fields: those of [`TABLE_KEYS`], or `id`.
pub fn table_key(table: &str) -> &'static [&'static str] {
    TABLE_KEYS
        .iter()
        .find(|(name, _)| *name == table)
        .map_or(&["id"][..], |(_, key)| *key)
}

/// A table's text as one region reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableText<'a> {
    /// The JP table's text.
    Jp(&'a str),
    /// The text of an enabled non-JP region.
    Region(Region, &'a str),
    /// An enabled non-JP region's table that could not be read, and why: the
    /// table is the JP rows alone.
    RegionMissing(Region, &'a str),
}

/// What a merge did with a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merged {
    /// The table's rows are the JP text's: no region is enabled besides
    /// [JP](Region::Jp), or the table is not one that merges.
    JpOnly,
    /// The regions' rows, with one entry per enabled non-JP region (the
    /// regions whose table could not be read add none).
    Merged { added: Vec<(Region, usize)> },
    /// Two regions add the same key: the table is refused whole.
    Collision(Collision),
}

/// The regions whose exclusive rows share a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collision {
    /// The table, by its upstream name.
    pub table: String,
    /// The key fields' JSON text, in key order (`42`, `"WORD_EX"`,
    /// `3,42`).
    pub key: String,
    /// The regions that add it, in `ENABLED` order.
    pub regions: Vec<Region>,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let regions: Vec<&str> = self.regions.iter().map(|region| region.directory()).collect();
        write!(f, "{} add the row keyed {}", regions.join(" and "), self.key)
    }
}

/// The regions a table is merged from: every [enabled](ENABLED) region
/// besides [JP](Region::Jp), in `ENABLED` order.
pub fn merged_regions() -> Vec<Region> {
    ENABLED
        .iter()
        .copied()
        .filter(|region| *region != Region::Jp)
        .collect()
}

/// A row's key: its key fields' JSON text, joined. `None` when the row is
/// not an object or a key field is absent or null.
fn row_key(row: &Value, fields: &[&str]) -> Option<String> {
    let row = row.as_object()?;
    let mut parts = Vec::with_capacity(fields.len());
    for field in fields {
        let value = row.get(*field)?;
        if value.is_null() {
            return None;
        }
        parts.push(value.to_string());
    }
    Some(parts.join(","))
}

/// `text` as an array of rows: `Ok(None)` for a document that is not an
/// array, `Err` for an array whose elements are not all objects.
fn rows_of(text: &str) -> Result<Option<Vec<Value>>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
    let Value::Array(rows) = value else {
        return Ok(None);
    };
    if let Some(index) = rows.iter().position(|row| !row.is_object()) {
        return Err(format!("row {index} is not an object"));
    }
    Ok(Some(rows))
}

/// An exclusive row's fields: the region field, the row's key fields in key
/// order, then the row's other fields in its own order.
fn ordered_row(mut row: Map<String, Value>, fields: &[&str], region: &str) -> Map<String, Value> {
    let mut ordered = Map::new();
    ordered.insert(REGION_FIELD.to_owned(), Value::String(region.to_owned()));
    for field in fields {
        if let Some(value) = row.shift_remove(*field) {
            ordered.insert((*field).to_owned(), value);
        }
    }
    ordered.extend(row);
    ordered
}

/// The merged text of `table`: its JP text and the texts of
/// [`merged_regions`] in order.
///
/// `Ok((None, Merged::JpOnly))` is the JP text unchanged: the regions given
/// add no row JP lacks, or the JP document is not an array of rows. `Err`
/// names a non-JP text that is not an array of rows, or rows (JP or not) that
/// lack their key: the table is then the JP text as well.
pub fn merge_table(
    table: &str,
    jp: &str,
    regions: &[TableText<'_>],
) -> Result<(Option<String>, Merged), String> {
    let fields = table_key(table);
    let Some(jp_rows) = rows_of(jp)? else {
        return Ok((None, Merged::JpOnly));
    };
    let mut keys: HashSet<String> = HashSet::with_capacity(jp_rows.len());
    for row in &jp_rows {
        let key = row_key(row, fields).ok_or_else(|| {
            format!(
                "the jp table does not merge: a row lacks its key ({})",
                fields.join(", ")
            )
        })?;
        keys.insert(key);
    }
    let mut exclusive: Vec<Value> = Vec::new();
    // Whichever region added each exclusive key first.
    let mut owners: HashMap<String, Region> = HashMap::new();
    let mut added: Vec<(Region, usize)> = Vec::new();
    let mut collision: Option<Collision> = None;
    for entry in regions {
        let (region, text) = match entry {
            TableText::Region(region, text) => (*region, *text),
            TableText::RegionMissing(region, _) => {
                added.push((*region, 0));
                continue;
            }
            TableText::Jp(_) => continue,
        };
        let rows = rows_of(text)
            .map_err(|reason| format!("the {} table {reason}", region.directory()))?
            .ok_or_else(|| {
                format!(
                    "the {} table is not an array of rows",
                    region.directory()
                )
            })?;
        let mut count = 0;
        for row in rows {
            let key = row_key(&row, fields).ok_or_else(|| {
                format!(
                    "the {} table does not merge: a row lacks its key ({})",
                    region.directory(),
                    fields.join(", ")
                )
            })?;
            if keys.contains(&key) {
                continue;
            }
            if let Some(first) = owners.get(&key) {
                // Recorded once; every added row is dropped by a collision.
                if collision.is_none() {
                    let mut clash = vec![*first];
                    if *first != region {
                        clash.push(region);
                    }
                    collision = Some(Collision {
                        table: table.to_owned(),
                        key,
                        regions: clash,
                    });
                }
                continue;
            }
            owners.insert(key, region);
            // A row was checked to be an object by `rows_of`.
            if let Value::Object(row) = row {
                exclusive.push(Value::Object(ordered_row(row, fields, region.directory())));
                count += 1;
            }
        }
        added.push((region, count));
    }
    if let Some(collision) = collision {
        return Ok((None, Merged::Collision(collision)));
    }
    if exclusive.is_empty() {
        return Ok((None, Merged::JpOnly));
    }
    let mut merged: Vec<Value> = jp_rows;
    merged.extend(exclusive);
    let text = serde_json::to_string(&Value::Array(merged))
        .map_err(|error| format!("the merged table is not JSON ({error})"))?;
    Ok((Some(text), Merged::Merged { added }))
}
