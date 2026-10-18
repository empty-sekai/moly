//! The server values the client reads without a world: the home gate row and
//! the rank-release rows a response carried, for the callers that hold no
//! `World` (the master parsers, the gate presenter's bundle lookup).
//!
//! Both are set by [`crate::userdata::apply_arrivals`] from the response's
//! `Sections`; before the first answer they are absent, and a reader that
//! needs one says so.

use std::sync::Mutex;

use moly_protocol::rows::{Gate, RankReleaseRow};
use serde_json::{json, Value};

#[derive(Default)]
struct Values {
    gate: Option<Gate>,
    rank_releases: Vec<RankReleaseRow>,
}

static VALUES: Mutex<Values> = Mutex::new(Values {
    gate: None,
    rank_releases: Vec::new(),
});

/// Records the home gate row a response carried.
pub(crate) fn seat_gate(gate: Gate) {
    if let Ok(mut values) = VALUES.lock() {
        values.gate = Some(gate);
    }
}

/// The home gate's (gate id, skin id), once a response carried it.
pub(crate) fn home_gate() -> Option<(i32, i32)> {
    let values = VALUES.lock().ok()?;
    values.gate.map(|gate| (gate.gate_id, gate.skin_id))
}

/// Records the `mysekaiRankReleases` rows the server appends.
pub(crate) fn seat_rank_releases(rows: Vec<RankReleaseRow>) {
    if let Ok(mut values) = VALUES.lock() {
        values.rank_releases = rows;
    }
}

/// The appended `mysekaiRankReleases` rows in the master table's own field
/// names (the master spells the release type `mysekaiRankRelaseType`).
pub(crate) fn rank_release_rows() -> Vec<Value> {
    let Ok(values) = VALUES.lock() else {
        return Vec::new();
    };
    values
        .rank_releases
        .iter()
        .map(|row| {
            json!({
                "mysekaiRank": row.mysekai_rank,
                "mysekaiRankRelaseType": row.mysekai_rank_release_type,
                "externalId": row.external_id,
            })
        })
        .collect()
}

/// The `mysekaiRankReleases` rows a reader walks: the rows it read itself,
/// plus the rows the server appends under its site-level release policy
/// ([`rank_release_rows`]). Every reader of the table takes its rows through
/// here, so they agree on what the releases are; without a response, and under
/// the source's own policy, the table's rows are all of them.
pub(crate) fn rank_release_rows_with_appended(mut rows: Vec<Value>) -> Vec<Value> {
    rows.extend(rank_release_rows());
    rows
}
