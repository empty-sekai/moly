//! Master tables: the game's own tables, read at runtime from the regions'
//! master hosts ([`crate::remote`]), never from the asset root.
//!
//! A consumer declares each table it reads as a [`MasterTable`]: the upstream
//! table name, the name its log lines and the missing list use, and a parser
//! from the table's text to the consumer's typed value. It requests the table
//! from [`MasterData`] and takes the typed result once it has resolved.
//!
//! [`resolve`] is the one system that loads them. Once the root's region is
//! known (`source.region` of the root's `source.json`), each requested table
//! is read as the merged table ([`merge_table`]): the JP table, then the same
//! table of every enabled non-JP region ([`ENABLED`]). Each is read from the master
//! bases in order ([`MasterBases`]), base by base and JP's table first. A
//! producer `master` pin in `source.json` selects its verified repository
//! commit for that region. Its version document must agree before tables
//! load; moving mirrors are not fallbacks for a pin. Roots without a pin
//! keep the latest sources. The upstream data version is logged once. A base whose
//! read timed out (no response headers within the reader's bound) is demoted
//! for the session: every later table tries it after the others.
//!
//! The merged text is what the consumer's parser reads, so a consumer sees
//! all JP rows and, after them, each row an enabled region adds with the
//! field `"moly:region"` naming the region; a key two non-JP regions both add
//! refuses the table by name. A non-JP table that no base serves leaves the
//! table as the JP rows, named once. A table that is not merged (a document
//! that is not an array of rows, or rows without their key) is read as the JP
//! table, named once.
//!
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

mod source;

use source::{parse_source, MasterSource};

use super::JsonAsset;
use crate::remote::{self, Region, ENABLED};

pub use super::master_merge::{
    merge_table, merged_regions, table_key, Collision, Merged, TableText, REGION_FIELD, TABLE_KEYS,
};

/// The root's identity document: its region and optional producer master pin.
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
    /// The admitted host it reads through ([`crate::remote::Remote`]).
    pub host: remote::Remote,
}

/// The bases every table is read from, tried in order: a table is absent
/// only when each admitted base has failed. The first is each region's
/// repository; moving master hosts follow as mirrors for roots without a pin,
/// and carry the `jp` and `cn` directories only. The app may replace the list.
#[derive(Resource, Clone)]
pub struct MasterBases(pub Vec<MasterBase>);

impl Default for MasterBases {
    fn default() -> Self {
        MasterBases(vec![
            MasterBase {
                name: "the region's master repository",
                host: remote::Remote::MasterRepository,
            },
            MasterBase {
                name: "the master host",
                host: remote::Remote::Master,
            },
            MasterBase {
                name: "the master mirror",
                host: remote::Remote::MasterMirror,
            },
        ])
    }
}

impl MasterBases {
    /// The first admitted untried base, preserving timeout demotion. A pin
    /// admits only the repository; neither failover nor demotion drops it.
    fn next(
        &self,
        source: &MasterSource,
        region: Region,
        tried: &[usize],
        demoted: &[usize],
    ) -> Option<usize> {
        let available = |index: &usize| {
            !tried.contains(index)
                && self
                    .0
                    .get(*index)
                    .is_some_and(|base| source.admits(base.host, region))
        };
        (0..self.0.len())
            .filter(available)
            .find(|index| !demoted.contains(index))
            .or_else(|| demoted.iter().copied().find(available))
    }
}

/// Tables a region's master hosts do not publish. A request for one could
/// only fail on every base, so it resolves as absent, by name, without one;
/// its consumer names what it reads instead.
const UNPUBLISHED: &[(Region, &str)] = &[
    // Both CN master hosts answer 404 for it.
    (Region::Cn, "configs"),
];

fn unpublished(region: Region, table: &str) -> Option<String> {
    UNPUBLISHED
        .iter()
        .any(|(only, name)| *only == region && *name == table)
        .then(|| format!("the {} master hosts do not publish it", region.directory()))
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
    /// The regions of the merged table, JP first: the regions
    /// [`ENABLED`] names, read in that order.
    regions: Vec<Region>,
    /// The region whose table is being read (an index into `regions`).
    region: usize,
    /// The base being read (an index into [`MasterBases`]).
    base: Option<usize>,
    /// The bases already tried for the current region.
    tried: Vec<usize>,
    handle: Option<Handle<JsonAsset>>,
    /// The failed reads of the current region, with their errors.
    failures: Vec<String>,
    /// What each region's read did so far, in `regions` order.
    outcomes: Vec<TableOutcome>,
}

/// How one region's table was read.
#[derive(Clone, Debug)]
enum TableOutcome {
    /// The region's table, and whether it was read from the repository host
    /// (rather than from a master host's region directory).
    Read(Region, String, bool),
    /// The region's table could not be read, and why (the bases that failed).
    Missing(Region, String),
    /// The region's master hosts do not publish the table.
    Unpublished(Region),
}

impl TableOutcome {
    fn region(&self) -> Region {
        match self {
            TableOutcome::Read(region, ..)
            | TableOutcome::Missing(region, _)
            | TableOutcome::Unpublished(region) => *region,
        }
    }
}

/// The master tables of the root: requests, results until their consumers
/// take them, and the tables named missing.
#[derive(Resource, Default)]
pub struct MasterData {
    /// The root's region and optional immutable producer master pin. Each
    /// table still merges the regions in [`ENABLED`].
    source: Document<MasterSource>,
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
    /// What each region gave each table (named once per table).
    region_rows: Vec<RegionRows>,
    /// The region tables counted in `region_rows`.
    counted: Vec<&'static str>,
    /// Tables read as the JP table, and why (named once per table).
    unmerged: Vec<Unmerged>,
}

/// What one region gave one table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionRows {
    pub table: &'static str,
    pub name: &'static str,
    pub region: Region,
    /// Rows only this region has (JP: every row; a region whose table could
    /// not be read, or that adds no row: none).
    pub added: usize,
}

impl fmt::Display for RegionRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} rows of the {} table are its own",
            self.name,
            self.added,
            self.region.directory()
        )
    }
}

/// A table that is the JP table rather than the merged one, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unmerged {
    pub table: &'static str,
    pub name: &'static str,
    pub reason: String,
}

impl fmt::Display for Unmerged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is read as the JP table only: {}", self.name, self.reason)
    }
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
            regions: ENABLED.to_vec(),
            region: 0,
            base: None,
            tried: Vec::new(),
            handle: None,
            failures: Vec::new(),
            outcomes: Vec::new(),
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

    /// The root's region, once its identity document has been read. Every
    /// table is the merged table of [`ENABLED`] whatever it is; it names the
    /// master storage the root's own assets are read from, and the region
    /// whose upstream data version is logged.
    pub fn region(&self) -> Option<Region> {
        match &self.source {
            Document::Read(Ok(source)) => Some(source.region),
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

    /// What each enabled region gave the tables read so far.
    pub fn region_rows(&self) -> &[RegionRows] {
        &self.region_rows
    }

    /// The tables read as the JP table rather than the merged one.
    pub fn unmerged(&self) -> &[Unmerged] {
        &self.unmerged
    }

    /// Names a table read as the JP table, and why: once per table.
    fn name_unmerged(&mut self, table: &'static str, name: &'static str, reason: String) {
        if self.unmerged.iter().any(|known| known.table == table) {
            return;
        }
        let unmerged = Unmerged {
            table,
            name,
            reason,
        };
        warn!("[masters] {unmerged}");
        self.unmerged.push(unmerged);
    }

    /// Counts what each region gave a table, once per table.
    fn count_region_rows(
        &mut self,
        table: &'static str,
        name: &'static str,
        rows: &[(Region, usize)],
    ) {
        if self.counted.contains(&table) {
            return;
        }
        self.counted.push(table);
        for (region, added) in rows {
            self.region_rows.push(RegionRows {
                table,
                name,
                region: *region,
                added: *added,
            });
        }
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
    let source = match data.source.poll(
        &server,
        &jsons,
        || AssetPath::from(SOURCE_DOCUMENT),
        parse_source,
    ) {
        None => return,
        Some(Ok(source)) => source.clone(),
        Some(Err(reason)) => {
            let reason = format!("the root's master source is invalid: source.json {reason}");
            data.refuse_pending(reason);
            return;
        }
    };
    let version = data.read_version(&server, &jsons, &bases, &source);
    if source.pin.is_some() {
        match version {
            None => return,
            Some(Err(reason)) => {
                data.refuse_pending(format!("the pinned master version was refused: {reason}"));
                return;
            }
            Some(Ok(())) => {}
        }
    }
    let mut waiting = Vec::new();
    for mut entry in std::mem::take(&mut data.pending) {
        match entry.advance(data, &server, &jsons, &bases, &source) {
            Some(result) => {
                data.resolved.insert(entry.key, result);
            }
            None => waiting.push(entry),
        }
    }
    data.pending = waiting;
}

impl MasterData {
    fn refuse_pending(&mut self, reason: String) {
        for entry in std::mem::take(&mut self.pending) {
            let error = entry.error(reason.clone());
            self.name_missing(&error, true);
            self.resolved.insert(entry.key, Err(error));
        }
    }

    /// Reads and logs the version once. Legacy roots only log it; a pinned
    /// root waits for a matching version before requesting any master table.
    fn read_version(
        &mut self,
        server: &AssetServer,
        jsons: &Assets<JsonAsset>,
        bases: &MasterBases,
        source: &MasterSource,
    ) -> Option<Result<(), String>> {
        if let Document::Read(result) = &self.upstream_version {
            return Some(result.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        let Some((base, path)) = bases
            .0
            .iter()
            .find_map(|base| source.version(base.host).map(|path| (base, path)))
        else {
            let reason = "no admitted master base serves the source version".to_owned();
            warn!("[masters] {reason}");
            self.upstream_version = Document::Read(Err(reason.clone()));
            return Some(Err(reason));
        };
        let result = self.upstream_version.poll(
            server,
            jsons,
            || path,
            |text| source.parse_version(text),
        )?;
        let identity = source.pin.as_ref().map_or_else(
            || "latest".to_owned(),
            |pin| format!("commit {}", pin.commit),
        );
        match result {
            Ok(version) => info!(
                "[masters] {}: upstream dataVersion {version} ({}; {identity})",
                source.region.directory(),
                base.name
            ),
            Err(reason) => warn!(
                "[masters] {}: the upstream version was refused ({}; {identity}): {reason}",
                source.region.directory(),
                base.name
            ),
        }
        Some(result.as_ref().map(|_| ()).map_err(Clone::clone))
    }
}

impl Pending {
    fn error(&self, reason: String) -> MasterError {
        MasterError {
            table: self.table,
            name: self.name,
            reason,
        }
    }

    /// One frame of this table: its regions' tables read one after another,
    /// JP first. `Some` once the merged table has resolved, or refused.
    fn advance(
        &mut self,
        data: &mut MasterData,
        server: &AssetServer,
        jsons: &Assets<JsonAsset>,
        bases: &MasterBases,
        source: &MasterSource,
    ) -> Option<Result<Parsed, MasterError>> {
        // JP's table is the table; every later region adds to it.
        if self.region == 0 {
            if let Some(reason) = data.absent.get(self.table) {
                return Some(Err(self.error(reason.clone())));
            }
        }
        let Some(region) = self.regions.get(self.region).copied() else {
            return Some(self.finish(data));
        };
        if !source.pinned(region) && unpublished(region, self.table).is_some() {
            self.outcomes.push(TableOutcome::Unpublished(region));
            self.next_region();
            return None;
        }
        // The next base: the first untried one that is not demoted, else the
        // first untried demoted one.
        if self.base.is_none() {
            self.base = bases.next(source, region, &self.tried, &data.demoted);
        }
        let Some(index) = self.base else {
            let reason = if self.failures.is_empty() {
                "no admitted master base serves this source".to_owned()
            } else {
                format!("no master base served it ({})", self.failures.join("; "))
            };
            if self.region == 0 {
                let error = self.error(reason.clone());
                data.absent.insert(self.table, reason);
                data.name_missing(&error, true);
                return Some(Err(error));
            }
            self.outcomes.push(TableOutcome::Missing(region, reason));
            self.next_region();
            return None;
        };
        let base = bases.0[index];
        let Some(path) = source.table(base.host, region, self.table) else {
            self.failures
                .push(format!("{} is not admitted for this source", base.name));
            self.tried.push(index);
            self.base = None;
            self.handle = None;
            return None;
        };
        let handle = self.handle.get_or_insert_with(|| server.load(path)).clone();
        if let LoadState::Failed(error) = server.load_state(&handle) {
            self.failures.push(format!("{}: {error}", base.name));
            self.tried.push(index);
            self.base = None;
            self.handle = None;
            if timed_out(&error) {
                let next = bases.next(source, region, &self.tried, &data.demoted);
                data.demote(index, &base, self.table, next.map(|other| &bases.0[other]));
            }
            return None;
        }
        let text = jsons.get(&handle)?.0.clone();
        self.outcomes.push(TableOutcome::Read(
            region,
            text,
            base.host == remote::Remote::MasterRepository,
        ));
        self.next_region();
        None
    }

    /// Moves on to the next region's table.
    fn next_region(&mut self) {
        self.region += 1;
        self.tried.clear();
        self.base = None;
        self.handle = None;
        self.failures.clear();
    }

    /// The table once every region has been read: its JP text, or its
    /// refused, merged, or JP-only resolution.
    fn finish(&mut self, data: &mut MasterData) -> Result<Parsed, MasterError> {
        let (table, name) = (self.table, self.name);
        let mut jp = None;
        let mut others: Vec<TableOutcome> = Vec::new();
        for outcome in std::mem::take(&mut self.outcomes) {
            if outcome.region() == Region::Jp {
                jp = Some(outcome);
            } else {
                others.push(outcome);
            }
        }
        let Some(TableOutcome::Read(_, text, _)) = jp else {
            // JP's read is the table's failure, named once, above.
            let reason = data
                .absent
                .get(table)
                .cloned()
                .unwrap_or_else(|| "the JP master base did not serve it".to_owned());
            return Err(self.error(reason));
        };
        let texts: Vec<TableText> = others
            .iter()
            .filter_map(|outcome| match outcome {
                TableOutcome::Read(region, text, _) => Some(TableText::Region(*region, text.as_str())),
                TableOutcome::Missing(region, reason) => {
                    Some(TableText::RegionMissing(*region, reason.as_str()))
                }
                TableOutcome::Unpublished(region) => {
                    Some(TableText::RegionMissing(*region, "is not published"))
                }
            })
            .collect();
        for outcome in &others {
            if let TableOutcome::Unpublished(region) = outcome {
                warn!(
                    "[masters] {name}: the {} master hosts do not publish it; the table is read without it",
                    region.directory()
                );
            }
        }
        let (merged, outcome) = match merge_table(table, &text, &texts) {
            Ok(merged) => merged,
            Err(reason) => {
                data.name_unmerged(table, name, reason);
                (None, Merged::JpOnly)
            }
        };
        for outcome in &others {
            if let TableOutcome::Missing(region, reason) = outcome {
                warn!(
                    "[masters] {name}: the {} table could not be read ({reason}); the table is the JP rows alone",
                    region.directory()
                );
            }
        }
        match outcome {
            Merged::Collision(collision) => {
                let error = self.error(format!("refused: {collision}"));
                data.name_missing(&error, true);
                return Err(error);
            }
            Merged::JpOnly => {
                if merged.is_none() {
                    count_rows(data, table, name, &text, &others);
                }
            }
            Merged::Merged { .. } => count_rows(data, table, name, &text, &others),
        }
        let text = merged.as_deref().unwrap_or(&text);
        // Refusals filed outside a parse of this layer are not this table's.
        take_refused_rows();
        let parsed = (self.parse)(text);
        data.name_refused_rows(table, name, take_refused_rows());
        match parsed {
            Ok(value) => {
                let regions = 1 + others
                    .iter()
                    .filter(|outcome| matches!(outcome, TableOutcome::Read(..)))
                    .count();
                info!(
                    "[masters] {name}: {} from the master bases",
                    match regions {
                        1 => "the JP table read".to_owned(),
                        regions => format!("the merged table of {regions} regions read"),
                    }
                );
                Ok(value)
            }
            Err(reason) => {
                let error = self.error(format!("malformed: {reason}"));
                data.name_missing(&error, false);
                Err(error)
            }
        }
    }
}

/// Counts what each region added to a table: JP's rows, and each other
/// region's exclusive rows. JP is counted even when its text was not merged.
fn count_rows(
    data: &mut MasterData,
    table: &'static str,
    name: &'static str,
    jp: &str,
    others: &[TableOutcome],
) {
    let jp_rows = serde_json::from_str::<Value>(jp)
        .ok()
        .and_then(|value| match value {
            Value::Array(rows) => Some(rows.len()),
            _ => None,
        })
        .unwrap_or(0);
    let mut rows = vec![(Region::Jp, jp_rows)];
    for outcome in others {
        match outcome {
            TableOutcome::Read(region, text, _) => {
                let (region, added) = merge_table(table, jp, &[TableText::Region(*region, text)])
                    .ok()
                    .and_then(|(_, merged)| match merged {
                        Merged::Merged { mut added } => added.pop(),
                        _ => None,
                    })
                    .unwrap_or((*region, 0));
                rows.push((region, added));
            }
            TableOutcome::Missing(region, _) | TableOutcome::Unpublished(region) => {
                rows.push((*region, 0))
            }
        }
    }
    data.count_region_rows(table, name, &rows);
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

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn rows(text: &str) -> Vec<Value> {
        match serde_json::from_str(text).expect("JSON") {
            Value::Array(rows) => rows,
            _ => panic!("not an array"),
        }
    }

    fn id(row: &Value) -> i64 {
        row["id"].as_i64().expect("an id")
    }

    fn region_field<'a>(row: &'a Value) -> Option<&'a str> {
        row.get(REGION_FIELD).and_then(Value::as_str)
    }

    /// The merged text and what the merge reported; `JpOnly` gives the JP
    /// text back, as its caller does.
    fn merged(table: &str, jp: &str, others: &[TableText<'_>]) -> (Vec<Value>, Merged) {
        let (text, outcome) = merge_table(table, jp, others).expect("a table of rows");
        match outcome {
            Merged::JpOnly => (rows(jp), outcome),
            other => match text {
                Some(text) => (rows(&text), other),
                None => panic!("a merged table without its text"),
            },
        }
    }

    #[test]
    fn the_jp_rows_come_first_unchanged_and_then_each_regions_exclusive_rows() {
        let jp = r#"[{"id": 1, "name": "a"}, {"id": 3, "name": "c"}]"#;
        let cn = r#"[{"id": 4, "name": "d"}, {"id": 3, "name": "other"}, {"id": 2, "name": "b"}]"#;
        let tw = r#"[{"id": 1, "name": "same"}, {"id": 5, "name": "e"}]"#;
        let (rows, outcome) = merged(
            "t",
            jp,
            &[
                TableText::Region(Region::Cn, cn),
                TableText::Region(Region::Tw, tw),
            ],
        );
        assert_eq!(
            outcome,
            Merged::Merged {
                added: vec![(Region::Cn, 2), (Region::Tw, 1)]
            }
        );
        // Every JP row first, as it was and in order, with no region field.
        assert_eq!(&rows[..2], &serde_json::from_str::<Vec<Value>>(jp).unwrap()[..]);
        // Then each region's exclusive rows, regions in the given order and
        // the rows in theirs.
        let tagged: Vec<(i64, Option<&str>)> = rows
            .iter()
            .map(|row| (id(row), region_field(row)))
            .collect();
        assert_eq!(
            tagged,
            [(1, None), (3, None), (4, Some("cn")), (2, Some("cn")), (5, Some("tw"))]
        );
        // An exclusive row keeps its own fields; a shared key is JP's row.
        assert_eq!(rows[2]["name"], "d");
        assert_eq!(rows[3]["name"], "b");
    }

    #[test]
    fn an_exclusive_row_is_tagged_with_its_region_and_keeps_key_order_first() {
        let jp = r#"[{"id": 1}]"#;
        let cn = r#"[{"name": "n", "id": 7, "seq": 3}]"#;
        let (rows, outcome) = merged("t", jp, &[TableText::Region(Region::Cn, cn)]);
        assert_eq!(outcome, Merged::Merged { added: vec![(Region::Cn, 1)] });
        assert_eq!(rows[1], serde_json::json!({"moly:region": "cn", "id": 7, "name": "n", "seq": 3}));
    }

    #[test]
    fn a_key_compares_as_json_text_and_a_null_key_is_absent() {
        // A string "1" is not the number 1.
        let jp = r#"[{"id": 1}]"#;
        let cn = r#"[{"id": "1"}]"#;
        let (rows, outcome) = merged("t", jp, &[TableText::Region(Region::Cn, cn)]);
        assert_eq!(outcome, Merged::Merged { added: vec![(Region::Cn, 1)] });
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], serde_json::json!({"moly:region": "cn", "id": "1"}));
        // A null key is an absent one: the table does not merge.
        let cn = r#"[{"id": null}]"#;
        assert!(merge_table("t", jp, &[TableText::Region(Region::Cn, cn)]).is_err());
    }

    #[test]
    fn the_declared_keys_are_the_census_exceptions_and_id_otherwise() {
        let key = |table: &str| table_key(table).to_vec();
        assert_eq!(key("musicVocals"), ["id"]);
        assert_eq!(key("wordings"), ["wordingKey"]);
        assert_eq!(key("resourceBoxes"), ["resourceBoxPurpose", "id"]);
        assert_eq!(key("musicTags"), ["musicId", "musicTag"]);

        let wordings_jp = r#"[{"wordingKey": "W", "value": "a"}]"#;
        let wordings_cn = r#"[{"wordingKey": "W", "value": "b"}, {"wordingKey": "X", "value": "c"}]"#;
        let (rows, _) = merged(
            "wordings",
            wordings_jp,
            &[TableText::Region(Region::Cn, wordings_cn)],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["value"], "a");
        assert_eq!(rows[1]["wordingKey"], "X");

        // A box is its purpose and its id: the same id under another purpose
        // is another box.
        let boxes_jp = r#"[{"resourceBoxPurpose": "a", "id": 1, "name": "x"}]"#;
        let boxes_cn = r#"[
            {"resourceBoxPurpose": "a", "id": 1, "name": "y"},
            {"resourceBoxPurpose": "b", "id": 1, "name": "z"}
        ]"#;
        let (rows, _) = merged(
            "resourceBoxes",
            boxes_jp,
            &[TableText::Region(Region::Cn, boxes_cn)],
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row["resourceBoxPurpose"].as_str(), id(row)))
                .collect::<Vec<_>>(),
            [(Some("a"), 1), (Some("b"), 1)]
        );
        assert_eq!(region_field(&rows[1]), Some("cn"));

        // An id the CN table no longer carries does not make two CN rows
        // collide under the tags key.
        let tags_jp = r#"[{"id": 9, "musicId": 1, "musicTag": "a"}]"#;
        let tags_cn = r#"[{"musicId": 1, "musicTag": "b"}, {"musicId": 1, "musicTag": "c"}]"#;
        let (rows, outcome) = merged("musicTags", tags_jp, &[TableText::Region(Region::Cn, tags_cn)]);
        assert_eq!(outcome, Merged::Merged { added: vec![(Region::Cn, 2)] });
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn two_regions_adding_one_key_refuse_the_table_naming_it() {
        let jp = r#"[{"id": 1}]"#;
        let cn = r#"[{"id": 2}, {"id": 3}]"#;
        let tw = r#"[{"id": 4}, {"id": 2}]"#;
        let (text, outcome) = merge_table(
            "mysekaiFixtures",
            jp,
            &[
                TableText::Region(Region::Cn, cn),
                TableText::Region(Region::Tw, tw),
            ],
        )
        .expect("a table of rows");
        assert_eq!(text, None);
        assert_eq!(
            outcome,
            Merged::Collision(Collision {
                table: "mysekaiFixtures".to_owned(),
                key: "2".to_owned(),
                regions: vec![Region::Cn, Region::Tw],
            })
        );
        let Merged::Collision(collision) = &outcome else {
            unreachable!()
        };
        let named = collision.to_string();
        assert!(named.contains("cn"));
        assert!(named.contains("tw"));
    }

    #[test]
    fn a_table_that_is_not_an_array_is_not_merged_and_one_without_keys_is_refused() {
        // A document that is one object is the region's whole table.
        let jp = r#"{"boostStamina": 1}"#;
        let cn = r#"{"boostStamina": 2}"#;
        let (text, outcome) = merge_table("mysekaiStaminaRecovery", jp, &[TableText::Region(Region::Cn, cn)])
            .expect("not a table of rows to merge");
        assert_eq!((text, outcome), (None, Merged::JpOnly));

        // A table whose rows lack their key is not merged.
        let jp = r#"[{"id": 1}]"#;
        let cn = r#"[{"name": "no id"}]"#;
        assert!(merge_table("t", jp, &[TableText::Region(Region::Cn, cn)]).is_err());
        let jp = r#"[{"name": "no id"}]"#;
        let cn = r#"[{"id": 1}]"#;
        assert!(merge_table("t", jp, &[TableText::Region(Region::Cn, cn)]).is_err());
        // A key only JP has is JP's row.
        let jp = r#"[{"id": 1}, {"id": 1}]"#;
        let cn = r#"[{"id": 1}]"#;
        let (_, outcome) = merge_table("t", jp, &[TableText::Region(Region::Cn, cn)]).expect("rows");
        assert_eq!(outcome, Merged::JpOnly);
    }

    #[test]
    fn an_unread_region_gives_the_jp_rows_alone_and_a_shared_table_merges() {
        let jp = r#"[{"id": 1}]"#;
        let cn = r#"[{"id": 2}]"#;
        // One region could not be read: JP plus whatever else could.
        let (rows, outcome) = merged(
            "t",
            jp,
            &[
                TableText::RegionMissing(Region::Cn, "no master base served it"),
                TableText::Region(Region::Tw, cn),
            ],
        );
        assert_eq!(
            outcome,
            Merged::Merged {
                added: vec![(Region::Cn, 0), (Region::Tw, 1)]
            }
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(region_field(&rows[1]), Some("tw"));

        // Every region unread: the JP rows alone.
        let (rows, outcome) = merged("t", jp, &[TableText::RegionMissing(Region::Cn, "no base")]);
        assert_eq!(outcome, Merged::JpOnly);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn the_layer_counts_each_regions_rows_and_names_a_table_it_cannot_merge() {
        let mut data = MasterData::default();
        let jp = r#"[{"id": 1}, {"id": 2}]"#;
        let others = vec![
            TableOutcome::Read(Region::Cn, r#"[{"id": 2}, {"id": 3}]"#.to_owned(), true),
            TableOutcome::Missing(Region::Tw, "no master base served it".to_owned()),
        ];
        count_rows(&mut data, "t", "t (test)", jp, &others);
        // Counted once per table.
        count_rows(&mut data, "t", "t (test)", jp, &others);
        let counted: Vec<(Region, usize)> = data
            .region_rows()
            .iter()
            .map(|entry| (entry.region, entry.added))
            .collect();
        assert_eq!(counted, [(Region::Jp, 2), (Region::Cn, 1), (Region::Tw, 0)]);

        data.name_unmerged("t", "t (test)", "is not merged: a JP row lacks its key (id)".into());
        data.name_unmerged("t", "t (test)", "another reason".into());
        assert_eq!(data.unmerged().len(), 1);
    }
}

#[cfg(test)]
mod master_pin_tests {
    use super::*;

    fn source(pin: bool) -> MasterSource {
        let mut value = serde_json::json!({
            "version": 1,
            "source": {"region": "jp", "assetVersion": "7", "assetHash": "assets"}
        });
        if pin {
            value["master"] = serde_json::json!({
                "region": "jp", "repository": "Team-Haruki/haruki-sekai-master",
                "commit": "0123456789abcdef0123456789abcdef01234567",
                "dataVersion": "7", "assetVersion": "7", "assetHash": "assets"
            });
        }
        parse_source(&value.to_string()).unwrap()
    }

    #[test]
    fn pinned_reads_do_not_fail_over_to_a_latest_mirror() {
        let bases = MasterBases::default();
        let source = source(true);
        assert_eq!(bases.next(&source, Region::Jp, &[], &[]), Some(0));
        // A previous timeout does not make a moving mirror an alternative.
        assert_eq!(bases.next(&source, Region::Jp, &[], &[0]), Some(0));
        // Once the repository failed, the pinned table has no further base.
        assert_eq!(bases.next(&source, Region::Jp, &[0], &[0]), None);
        assert_eq!(bases.next(&source, Region::Jp, &[0], &[]), None);
    }

    #[test]
    fn latest_reads_keep_the_existing_failover_and_demotion_order() {
        let bases = MasterBases::default();
        let source = source(false);
        assert_eq!(bases.next(&source, Region::Jp, &[], &[]), Some(0));
        assert_eq!(bases.next(&source, Region::Jp, &[0], &[]), Some(1));
        assert_eq!(bases.next(&source, Region::Jp, &[], &[0]), Some(1));
        assert_eq!(bases.next(&source, Region::Jp, &[1, 2], &[0]), Some(0));
        assert_eq!(bases.next(&source, Region::Jp, &[0, 1, 2], &[0]), None);
    }

    #[test]
    fn a_version_refusal_reaches_every_waiting_consumer() {
        const ONE: MasterTable<Vec<Value>> = MasterTable {
            table: "first",
            name: "first",
            parse: rows,
        };
        const TWO: MasterTable<Vec<Value>> = MasterTable {
            table: "second",
            name: "second",
            parse: rows,
        };
        let mut data = MasterData::default();
        data.request(&ONE);
        data.request(&TWO);
        data.refuse_pending("the pinned master version was refused".into());
        assert!(data.ready());
        assert_eq!(data.missing().len(), 2);
        for table in [&ONE, &TWO] {
            assert_eq!(
                data.take(table).unwrap().unwrap_err().reason,
                "the pinned master version was refused"
            );
        }
    }
}
