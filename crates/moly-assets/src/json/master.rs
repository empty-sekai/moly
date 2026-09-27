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
//! then its mirror), and the region's upstream data version is logged once. A table that no base
//! serves, or whose text its consumer's parser refuses, is named once in the
//! log and in the root's missing list ([`MasterData::missing`]); its consumer
//! takes the named error and applies its own fallback. Nothing here panics on
//! data.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use bevy::app::{App, PreUpdate};
use bevy::asset::{AssetPath, AssetServer, AssetTrackingSystems, Assets, Handle, LoadState};
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
    /// The base being read.
    base: usize,
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
            base: 0,
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
        let Some(base) = bases.0.get(self.base) else {
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
        let handle = self
            .handle
            .get_or_insert_with(|| server.load(base.table(region, self.table)))
            .clone();
        if let LoadState::Failed(error) = server.load_state(&handle) {
            self.failures.push(format!("{}: {error}", base.name));
            self.base += 1;
            self.handle = None;
            return None;
        }
        let text = &jsons.get(&handle)?.0;
        match (self.parse)(text) {
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

/// A string field of a row.
pub fn text<'a>(row: &'a Value, field: &str) -> Result<&'a str, String> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} of {} is not a string", row_label(row)))
}
