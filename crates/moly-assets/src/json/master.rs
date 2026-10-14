//! Master tables: the game's own tables, read at runtime from the region's
//! master hosts ([`crate::remote`]), never from the asset root.
//!
//! A consumer declares each table it reads as a [`MasterTable`]: the upstream
//! table name, the name its log lines and the missing list use, and a parser
//! from the table's text to the consumer's typed value. It requests the table
//! from [`MasterData`] and takes the typed result once it has resolved.
//!
//! [`resolve`] is the one system that loads them. Once the root's region is
//! known (`source.region` of the root's `source.json`), each requested table
//! is read from the master bases in order ([`MasterBases`]: the master host,
//! then its mirror), and the region's upstream data version is logged once. A
//! base whose read timed out (no response headers within the reader's bound)
//! is demoted for the session: every later table tries it after the others.
//! A table that no base serves, or whose text its consumer's parser refuses,
//! is named once in the log and in the root's missing list
//! ([`MasterData::missing`]); its consumer takes the named error and applies
//! its own fallback. A parser that reads its rows with [`read_rows`] drops a
//! row it cannot read instead of refusing the table: each distinct reason is
//! named once per consumer, with the number of rows it dropped
//! ([`MasterData::refused_rows`]). Nothing here panics on data.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use bevy::app::{App, PreUpdate};
use bevy::asset::io::AssetReaderError;
use bevy::asset::{
    AssetLoadError, AssetPath, AssetServer, AssetTrackingSystems, Assets, Handle, LoadState,
};
use bevy::prelude::*;
use serde_json::{Map, Value};

use super::JsonAsset;
use crate::remote::{self, Remote, RemoteRegion};

/// The root's identity document; its `source.region` selects the region
/// whose master tables are read.
const SOURCE_DOCUMENT: &str = "moly://source.json";

/// One master table a consumer reads.
pub struct MasterTable<T> {
    /// The upstream table name (`mysekaiRanks`).
    pub table: &'static str,
    /// The table's name in log lines and in the missing list, unique per
    /// consumer (`mysekaiRanks (the rank gauges)`).
    pub name: &'static str,
    /// The table's text to the consumer's value; `Err` names the data-shape
    /// failure.
    pub parse: fn(&str) -> Result<T, String>,
}

impl<T: 'static> MasterTable<T> {
    /// The key a request and its result are filed under.
    pub fn key(&self) -> MasterKey {
        MasterKey(TypeId::of::<T>(), self.name)
    }
}

/// A requested table's identity: its consumer's value type and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MasterKey(TypeId, &'static str);

/// A table its consumer does not get: absent from every base, or malformed
/// for the consumer's parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterError {
    pub table: &'static str,
    pub name: &'static str,
    pub reason: String,
}

impl fmt::Display for MasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.reason)
    }
}

/// One place master tables are read from.
#[derive(Clone, Copy, Debug)]
pub struct MasterBase {
    /// The base's name in log lines.
    pub name: &'static str,
    /// The admitted host it reads through (`Remote::Master` or
    /// `Remote::MasterMirror`).
    pub host: Remote,
}

impl MasterBase {
    /// A region's table, by its upstream name.
    fn table(self, region: RemoteRegion, table: &str) -> AssetPath<'static> {
        remote::master_table(self.host, region, table)
    }

    /// A region's `versions/current_version.json`.
    fn version(self, region: RemoteRegion) -> AssetPath<'static> {
        AssetPath::from(format!(
            "{}://{}/versions/current_version.json",
            self.host.source(),
            region_directory(region)
        ))
    }
}

/// The bases every table is read from, tried in order: a table is absent
/// only when each of them has failed. The app may replace the list.
#[derive(Resource, Clone)]
pub struct MasterBases(pub Vec<MasterBase>);

impl Default for MasterBases {
    fn default() -> Self {
        MasterBases(vec![
            MasterBase {
                name: "the master host",
                host: Remote::Master,
            },
            MasterBase {
                name: "the master mirror",
                host: Remote::MasterMirror,
            },
        ])
    }
}

/// Tables a region's master hosts do not publish. A request for one could
/// only fail on every base, so it resolves as absent, by name, without one;
/// its consumer names what it reads instead.
const UNPUBLISHED: &[(RemoteRegion, &str)] = &[
    // Both CN master hosts answer 404 for it.
    (RemoteRegion::Cn, "configs"),
];

fn unpublished(region: RemoteRegion, table: &str) -> Option<String> {
    UNPUBLISHED
        .iter()
        .any(|(only, name)| *only == region && *name == table)
        .then(|| format!("the {} master hosts do not publish it", region_directory(region)))
}

/// The region's directory on a master host, as upstream names it.
fn region_directory(region: RemoteRegion) -> &'static str {
    match region {
        RemoteRegion::Jp => "jp",
        RemoteRegion::Cn => "cn",
    }
}

/// A document the layer reads itself: requested, loading, or read.
#[derive(Default)]
enum Document<T> {
    #[default]
    Unrequested,
    Loading(Handle<JsonAsset>),
    Read(Result<T, String>),
}

impl<T> Document<T> {
    /// Requests the document on first use and reads it once it has loaded;
    /// `None` while it loads.
    fn poll(
        &mut self,
        server: &AssetServer,
        jsons: &Assets<JsonAsset>,
        path: impl FnOnce() -> AssetPath<'static>,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Option<&Result<T, String>> {
        if let Document::Unrequested = self {
            *self = Document::Loading(server.load(path()));
        }
        if let Document::Loading(handle) = self {
            let read = match server.load_state(&*handle) {
                LoadState::Failed(error) => Err(format!("did not load ({error})")),
                _ => match jsons.get(&*handle) {
                    Some(json) => parse(&json.0),
                    None => return None,
                },
            };
            *self = Document::Read(read);
        }
        match self {
            Document::Read(read) => Some(read),
            _ => None,
        }
    }
}

type Parsed = Box<dyn Any + Send + Sync>;

struct Pending {
    key: MasterKey,
    table: &'static str,
    name: &'static str,
    parse: Box<dyn Fn(&str) -> Result<Parsed, String> + Send + Sync>,
    /// The base being read (an index into [`MasterBases`]).
    base: Option<usize>,
    /// The bases already tried.
    tried: Vec<usize>,
    handle: Option<Handle<JsonAsset>>,
    /// The bases that failed, with their errors.
    failures: Vec<String>,
}

/// The master tables of the root: requests, results until their consumers
/// take them, and the tables named missing.
#[derive(Resource, Default)]
pub struct MasterData {
    region: Document<RemoteRegion>,
    upstream_version: Document<String>,
    pending: Vec<Pending>,
    resolved: HashMap<MasterKey, Result<Parsed, MasterError>>,
    missing: Vec<MasterError>,
    /// Tables no base served, and why (named once per table).
    absent: HashMap<&'static str, String>,
    /// Bases whose read timed out, in the order they did: tried last for
    /// the rest of the session.
    demoted: Vec<usize>,
    /// Rows consumers dropped, one entry per consumer and reason.
    refused_rows: Vec<RefusedRows>,
}

/// Rows of one table one consumer dropped for one reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefusedRows {
    pub table: &'static str,
    pub name: &'static str,
    pub reason: String,
    /// How many rows the reason dropped.
    pub count: usize,
    /// The first of them (`row 12`, by its id).
    pub first: String,
}

impl fmt::Display for RefusedRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} row(s) dropped, {} (first: {})",
            self.name, self.count, self.reason, self.first
        )
    }
}

impl MasterData {
    /// Requests a table; a table already requested, or resolved and not yet
    /// taken, is left as it is.
    pub fn request<T: Send + Sync + 'static>(&mut self, table: &MasterTable<T>) {
        let key = table.key();
        if self.resolved.contains_key(&key) || self.pending.iter().any(|entry| entry.key == key) {
            return;
        }
        let parse = table.parse;
        self.pending.push(Pending {
            key,
            table: table.table,
            name: table.name,
            parse: Box::new(move |text| parse(text).map(|value| Box::new(value) as Parsed)),
            base: None,
            tried: Vec::new(),
            handle: None,
            failures: Vec::new(),
        });
    }

    /// The table's result, once it has resolved; the result is handed out
    /// once.
    pub fn take<T: 'static>(&mut self, table: &MasterTable<T>) -> Option<Result<T, MasterError>> {
        let result = self.resolved.remove(&table.key())?;
        Some(match result {
            Ok(value) => value
                .downcast::<T>()
                .map(|value| *value)
                .map_err(|_| MasterError {
                    table: table.table,
                    name: table.name,
                    reason: "it resolved to another type".into(),
                }),
            Err(error) => Err(error),
        })
    }

    /// Whether a requested table has resolved and waits to be taken.
    pub fn is_resolved(&self, key: MasterKey) -> bool {
        self.resolved.contains_key(&key)
    }

    /// Whether a requested table is still loading.
    pub fn is_pending(&self, key: MasterKey) -> bool {
        self.pending.iter().any(|entry| entry.key == key)
    }

    /// No requested table is still loading.
    pub fn ready(&self) -> bool {
        self.pending.is_empty()
    }

    /// The root's tables named missing: absent from every base, or
    /// malformed for a consumer.
    pub fn missing(&self) -> &[MasterError] {
        &self.missing
    }

    /// The root's region, once its identity document has been read.
    pub fn region(&self) -> Option<RemoteRegion> {
        match &self.region {
            Document::Read(Ok(region)) => Some(*region),
            _ => None,
        }
    }

    /// The bases demoted for the session because a read of theirs timed out.
    pub fn demoted(&self) -> &[usize] {
        &self.demoted
    }

    /// The rows consumers dropped, one entry per consumer and reason.
    pub fn refused_rows(&self) -> &[RefusedRows] {
        &self.refused_rows
    }

    /// Names the rows a consumer's parser dropped: once per consumer and
    /// reason, with the count and the first row.
    fn name_refused_rows(&mut self, table: &'static str, name: &'static str, rows: Vec<RowRefusal>) {
        for group in group_refusals(table, name, rows) {
            let named = self
                .refused_rows
                .iter()
                .any(|known| known.name == group.name && known.reason == group.reason);
            if !named {
                warn!("[masters] {group}");
                self.refused_rows.push(group);
            }
        }
    }

    /// Demotes a base whose read timed out; named once per base.
    fn demote(&mut self, index: usize, base: &MasterBase, table: &str, next: Option<&MasterBase>) {
        if self.demoted.contains(&index) {
            return;
        }
        self.demoted.push(index);
        warn!(
            "[masters] {} sent no response headers in time for {table}; for the rest of the session every table tries {} first",
            base.name,
            next.map_or("no other base", |next| next.name)
        );
    }

    /// Names a table its consumer does not get: once per table when it is
    /// absent, once per consumer when it is malformed for that consumer.
    fn name_missing(&mut self, error: &MasterError, per_table: bool) {
        let named = self
            .missing
            .iter()
            .any(|known| known.table == error.table && (per_table || known.name == error.name));
        if !named {
            warn!("[masters] {error}");
            self.missing.push(error.clone());
        }
    }
}

/// Registers the layer; runs with the JSON asset registration.
pub(super) fn register(app: &mut App) {
    app.init_resource::<MasterBases>()
        .init_resource::<MasterData>()
        .add_systems(PreUpdate, resolve.after(AssetTrackingSystems));
}

/// PreUpdate, after the asset server has delivered this frame's loads:
/// reads the root's region, logs the region's upstream data version once,
/// advances every requested table, and files each resolved one for its
/// consumer.
pub fn resolve(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    bases: Res<MasterBases>,
    mut data: ResMut<MasterData>,
) {
    if data.pending.is_empty() {
        return;
    }
    let data = &mut *data;
    let region = match data.region.poll(
        &server,
        &jsons,
        || AssetPath::from(SOURCE_DOCUMENT),
        parse_region,
    ) {
        None => return,
        Some(Ok(region)) => *region,
        Some(Err(reason)) => {
            let reason = format!("the root's region is unknown: source.json {reason}");
            for entry in std::mem::take(&mut data.pending) {
                let error = entry.error(reason.clone());
                data.name_missing(&error, true);
                data.resolved.insert(entry.key, Err(error));
            }
            return;
        }
    };
    let version_unread = !matches!(data.upstream_version, Document::Read(_));
    if let (true, Some(base)) = (version_unread, bases.0.first()) {
        let directory = region_directory(region);
        match data
            .upstream_version
            .poll(&server, &jsons, || base.version(region), parse_version)
        {
            Some(Ok(version)) => {
                info!("[masters] {directory}: upstream dataVersion {version} ({})", base.name)
            }
            Some(Err(reason)) => warn!(
                "[masters] {directory}: the upstream dataVersion is missing: {}'s versions/current_version.json {reason}",
                base.name
            ),
            None => {}
        }
    }
    let mut waiting = Vec::new();
    for mut entry in std::mem::take(&mut data.pending) {
        match entry.advance(data, &server, &jsons, &bases, region) {
            Some(result) => {
                data.resolved.insert(entry.key, result);
            }
            None => waiting.push(entry),
        }
    }
    data.pending = waiting;
}

impl Pending {
    fn error(&self, reason: String) -> MasterError {
        MasterError {
            table: self.table,
            name: self.name,
            reason,
        }
    }

    /// One frame of this table; `Some` once it has resolved.
    fn advance(
        &mut self,
        data: &mut MasterData,
        server: &AssetServer,
        jsons: &Assets<JsonAsset>,
        bases: &MasterBases,
        region: RemoteRegion,
    ) -> Option<Result<Parsed, MasterError>> {
        if let Some(reason) = data.absent.get(self.table) {
            return Some(Err(self.error(reason.clone())));
        }
        if let Some(reason) = unpublished(region, self.table) {
            let error = self.error(reason.clone());
            data.absent.insert(self.table, reason);
            data.name_missing(&error, true);
            return Some(Err(error));
        }
        // The next base: the first untried one that is not demoted, else the
        // first untried demoted one.
        if self.base.is_none() {
            let untried = |index: &usize| !self.tried.contains(index);
            self.base = (0..bases.0.len())
                .filter(untried)
                .find(|index| !data.demoted.contains(index))
                .or_else(|| data.demoted.iter().copied().find(untried));
        }
        let Some(index) = self.base else {
            let reason = if self.failures.is_empty() {
                "no master base is configured".to_owned()
            } else {
                format!("no master base served it ({})", self.failures.join("; "))
            };
            let error = self.error(reason.clone());
            data.absent.insert(self.table, reason);
            data.name_missing(&error, true);
            return Some(Err(error));
        };
        let base = bases.0[index];
        let handle = self
            .handle
            .get_or_insert_with(|| server.load(base.table(region, self.table)))
            .clone();
        if let LoadState::Failed(error) = server.load_state(&handle) {
            self.failures.push(format!("{}: {error}", base.name));
            self.tried.push(index);
            self.base = None;
            self.handle = None;
            if timed_out(&error) {
                let next = (0..bases.0.len())
                    .find(|other| !self.tried.contains(other) && !data.demoted.contains(other));
                data.demote(index, &base, self.table, next.map(|other| &bases.0[other]));
            }
            return None;
        }
        let text = &jsons.get(&handle)?.0;
        // Refusals filed outside a parse of this layer are not this table's.
        take_refused_rows();
        let parsed = (self.parse)(text);
        data.name_refused_rows(self.table, self.name, take_refused_rows());
        match parsed {
            Ok(value) => {
                info!("[masters] {}: read from {}", self.name, base.name);
                Some(Ok(value))
            }
            Err(reason) => {
                let error = self.error(format!("malformed: {reason}"));
                data.name_missing(&error, false);
                Some(Err(error))
            }
        }
    }
}

/// Whether a load failed because its read timed out (the remote reader's
/// bound on the response headers).
fn timed_out(error: &AssetLoadError) -> bool {
    matches!(
        error,
        AssetLoadError::AssetReaderError(AssetReaderError::Io(io))
            if io.kind() == std::io::ErrorKind::TimedOut
    )
}

/// `source.region` of the root's identity document.
fn parse_region(text: &str) -> Result<RemoteRegion, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
    if value["version"].as_u64() != Some(1) {
        return Err("has an unsupported or missing identity version".into());
    }
    match value["source"]["region"].as_str() {
        Some("jp") => Ok(RemoteRegion::Jp),
        Some("cn") => Ok(RemoteRegion::Cn),
        other => Err(format!("names the source region {other:?}, not jp or cn")),
    }
}

/// `dataVersion` of an upstream `current_version.json`.
fn parse_version(text: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
    value["dataVersion"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "has no dataVersion".to_owned())
}

// ---------------------------------------------------------------------------
// Row readers for consumer parsers
// ---------------------------------------------------------------------------

/// A plain upstream table: a JSON array of row objects, in master order.
pub fn rows(text: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("not JSON ({error})"))?;
    let Value::Array(rows) = value else {
        return Err("not an array of rows".into());
    };
    if let Some(index) = rows.iter().position(|row| !row.is_object()) {
        return Err(format!("row {index} is not an object"));
    }
    Ok(rows)
}

/// A single-row upstream table: one JSON object.
pub fn object(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(text).map_err(|error| format!("not JSON ({error})"))? {
        Value::Object(row) => Ok(row),
        _ => Err("not a row object".into()),
    }
}

/// How a row is named in a field error: its id, or its text.
fn row_label(row: &Value) -> String {
    match row.get("id") {
        Some(id) => format!("row {id}"),
        None => format!("row {row}"),
    }
}

/// An integer field of a row.
pub fn int(row: &Value, field: &str) -> Result<i64, String> {
    row.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{field} of {} is not an integer", row_label(row)))
}

/// A 32-bit integer field of a row.
pub fn int32(row: &Value, field: &str) -> Result<i32, String> {
    i32::try_from(int(row, field)?)
        .map_err(|_| format!("{field} of {} is out of the 32-bit range", row_label(row)))
}

/// A number field of a row (integer or float).
pub fn number(row: &Value, field: &str) -> Result<f64, String> {
    row.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{field} of {} is not a number", row_label(row)))
}

/// A string field of a row.
pub fn text<'a>(row: &'a Value, field: &str) -> Result<&'a str, String> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} of {} is not a string", row_label(row)))
}

/// A string field of a row that names one of a closed set of values (an
/// enum the consumer reads); `Err` names a field that is not a string or a
/// value outside the set.
pub fn one_of<T: Copy>(row: &Value, field: &str, values: &[(&str, T)]) -> Result<T, String> {
    let value = text(row, field)?;
    values
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, known)| *known)
        .ok_or_else(|| format!("{field} of {} is {value:?}, not a value this reader knows", row_label(row)))
}

// ---------------------------------------------------------------------------
// Row-level refusal
// ---------------------------------------------------------------------------

/// One row a consumer's parser does not take, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowRefusal {
    /// The row's label (`row 12`, by its id).
    pub row: String,
    /// What of the row the consumer cannot read, without the row's label.
    pub reason: String,
}

thread_local! {
    /// Rows refused by [`read_rows`] since the layer last took them. A
    /// parser runs synchronously inside [`resolve`], so the layer takes the
    /// rows of each parse right after it.
    static REFUSED_ROWS: RefCell<Vec<RowRefusal>> = const { RefCell::new(Vec::new()) };
}

/// The rows refused by [`read_rows`] on this thread since the last call. The
/// layer calls it around every parse; a parser's own tests call it to see
/// what the parse refused.
pub fn take_refused_rows() -> Vec<RowRefusal> {
    REFUSED_ROWS.with(|rows| std::mem::take(&mut *rows.borrow_mut()))
}

/// A plain upstream table read row by row. `read` gives a row's value,
/// `Ok(None)` for a row the consumer does not take by its own rule (not a
/// refusal), or `Err` naming what of the row it cannot read: a field absent
/// or of another type, a value outside the set it knows. A refused row is
/// dropped, and the layer names each distinct reason once per consumer with
/// the number of rows it dropped. `Err` only when the text is not a table
/// of rows.
///
/// For tables whose rows are independent entries. A table whose rows only
/// mean something together (thresholds, a sequence) refuses as a whole
/// with [`rows`] and `?`.
pub fn read_rows<T>(
    text: &str,
    read: impl FnMut(&Value) -> Result<Option<T>, String>,
) -> Result<Vec<T>, String> {
    let (kept, refused) = read_parsed_rows(&rows(text)?, read);
    REFUSED_ROWS.with(|rows| rows.borrow_mut().extend(refused));
    Ok(kept)
}

/// [`read_rows`] over rows already parsed, for a reader that reads its
/// tables outside this layer: the refused rows are returned to it, to be
/// named once per reason ([`group_refusals`]).
pub fn read_parsed_rows<T>(
    rows: &[Value],
    mut read: impl FnMut(&Value) -> Result<Option<T>, String>,
) -> (Vec<T>, Vec<RowRefusal>) {
    let mut kept = Vec::new();
    let mut refused = Vec::new();
    for row in rows {
        match read(row) {
            Ok(Some(value)) => kept.push(value),
            Ok(None) => {}
            Err(reason) => {
                let label = row_label(row);
                // The row readers name the row inside their reason; it is
                // taken out so rows refused for one reason are named together.
                let reason = reason.replace(&format!(" of {label}"), "");
                refused.push(RowRefusal { row: label, reason });
            }
        }
    }
    (kept, refused)
}

/// Refusals grouped by reason, in the order each reason first appears.
pub fn group_refusals(table: &'static str, name: &'static str, rows: Vec<RowRefusal>) -> Vec<RefusedRows> {
    let mut groups: Vec<RefusedRows> = Vec::new();
    for row in rows {
        match groups.iter_mut().find(|group| group.reason == row.reason) {
            Some(group) => group.count += 1,
            None => groups.push(RefusedRows {
                table,
                name,
                reason: row.reason,
                count: 1,
                first: row.row,
            }),
        }
    }
    groups
}

#[cfg(test)]
mod row_refusal_tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        A,
        B,
    }

    fn read(row: &Value) -> Result<Option<(i64, Kind)>, String> {
        if text(row, "skip").is_ok() {
            return Ok(None);
        }
        Ok(Some((int(row, "id")?, one_of(row, "kind", &[("a", Kind::A), ("b", Kind::B)])?)))
    }

    #[test]
    fn rows_a_reader_cannot_read_are_dropped_and_named_by_reason() {
        take_refused_rows();
        let text = r#"[
            {"id": 1, "kind": "a"},
            {"id": 2, "kind": "c"},
            {"id": "3", "kind": "b"},
            {"id": 4},
            {"id": 5, "kind": "b"},
            {"id": 6, "kind": "c"},
            {"id": 7, "skip": "yes"}
        ]"#;
        let kept = read_rows(text, read).expect("a table of rows");
        assert_eq!(kept, [(1, Kind::A), (5, Kind::B)]);
        let groups = group_refusals("t", "t (test)", take_refused_rows());
        let named: Vec<(String, usize, String)> = groups
            .iter()
            .map(|group| (group.reason.clone(), group.count, group.first.clone()))
            .collect();
        assert_eq!(
            named,
            [
                ("kind is \"c\", not a value this reader knows".to_owned(), 2, "row 2".to_owned()),
                ("id is not an integer".to_owned(), 1, "row \"3\"".to_owned()),
                ("kind is not a string".to_owned(), 1, "row 4".to_owned()),
            ]
        );
        assert!(take_refused_rows().is_empty());
    }

    #[test]
    fn a_text_that_is_not_a_table_still_refuses_the_table() {
        take_refused_rows();
        assert!(read_rows("{\"id\": 1}", read).is_err());
        assert!(read_rows("[1, 2]", read).is_err());
        assert!(take_refused_rows().is_empty());
    }

    #[test]
    fn the_layer_names_each_reason_once_per_consumer() {
        let mut data = MasterData::default();
        let refusal = |row: &str, reason: &str| RowRefusal {
            row: row.into(),
            reason: reason.into(),
        };
        data.name_refused_rows("t", "t (one)", vec![refusal("row 1", "x"), refusal("row 2", "x")]);
        data.name_refused_rows("t", "t (one)", vec![refusal("row 3", "x")]);
        data.name_refused_rows("t", "t (two)", vec![refusal("row 1", "x")]);
        let named: Vec<(&str, usize)> = data
            .refused_rows()
            .iter()
            .map(|group| (group.name, group.count))
            .collect();
        assert_eq!(named, [("t (one)", 2), ("t (two)", 1)]);
    }
}
