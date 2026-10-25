//! The phenomena the user has learned, as the client holds them.
//!
//! - `UserDataManager.UserMysekaiPhenomenas` (`UserMysekaiPhenomena`:
//!   `mysekaiPhenomenaId` and `obtainedAt`): [`ClientMysekaiPhenomena`], set by
//!   responses only. `UpdateMysekaiPhenomena(rows)` replaces the whole list
//!   (`UpdateAll` included), and a response that carries no section leaves the
//!   copy as it is. The join carries it, and so does the release reply.
//! - `SiteEnvironmentUtility.HasTodayEnvironment()`: `Any` over the rows whose
//!   `mysekaiPhenomenaId` equals `UserDataManager.TodayMysekaiPhenomenaId`;
//!   the harvest site's arrival reads it
//!   ([`crate::harvest::learn`]), as the site-map phenomena catalog does.
//! - `SiteEnvironmentUtility.ExecuteReleaseAPI(siteId)` is one
//!   `Call::Release` on the server link ([`crate::harvest::learn`]); its
//!   reply is a `SuiteUser` answer whose sections this copy receives.

use bevy::prelude::*;
use serde_json::Value;

pub use moly_protocol::api::ReleaseRequest as UserMysekaiReleaseRequest;
pub use moly_protocol::json::phenomena_value as rows_value;
pub use moly_protocol::rows::UserMysekaiPhenomena;

/// The client's copy of `userMysekaiPhenomenas`, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientMysekaiPhenomena {
    rows: Vec<UserMysekaiPhenomena>,
    /// Responses that carried the section (0: none yet).
    pub revision: u64,
}

impl ClientMysekaiPhenomena {
    /// The rows `UpdateMysekaiPhenomena` was last given.
    pub fn rows(&self) -> &[UserMysekaiPhenomena] {
        &self.rows
    }

    /// `UpdateMysekaiPhenomena`: the carried rows replace the list.
    pub fn apply(&mut self, rows: Vec<UserMysekaiPhenomena>) {
        self.rows = rows;
        self.revision += 1;
    }

    /// `SiteEnvironmentUtility.HasTodayEnvironment()`: some row is the
    /// environment `today_phenomena_id` (`false` for an empty list).
    pub fn has_today_environment(&self, today_phenomena_id: i32) -> bool {
        self.rows
            .iter()
            .any(|row| row.mysekai_phenomena_id == today_phenomena_id)
    }

    pub fn view(&self) -> Value {
        rows_value(&self.rows)
    }
}
