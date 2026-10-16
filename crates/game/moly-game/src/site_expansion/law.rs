//! The rank-release reads and the client's local lists behind a site
//! expansion, as pure functions over the master rows.
//!
//! - `MasterDataManager.GetMasterMysekaiRankReleases(from, to)` keeps the
//!   rows with `from < rank <= to`, in master order.
//! - `MysekaiRankUtility.GetUnlockedMasterMysekaiRankRelease(rank, site)`
//!   reads the releases of `(1, rank]`, keeps the site-level rows of that
//!   site, and returns the first row whose rank is the greatest: the latest
//!   release at or below the rank, not a release of that exact rank.
//! - `MysekaiTopicsManager` keeps two client-local lists in the application's
//!   local settings: the topics and `UnlockedMasterSiteLevelIds`. The second
//!   grows only through the expansion performances (the cutscene path saves
//!   the unlocked level whether or not a cutscene exists for it).

use moly_assets::json::master::{MasterData, MasterTable};
use serde_json::Value;

/// `MysekaiTopicEnum.SiteExpansion`.
pub(crate) const TOPIC_SITE_EXPANSION: i32 = 1;
/// `MysekaiTopicsManager.MIN_MYSEKAI_RANK_RELEASE`: a rank below it adds no
/// expansion topic.
const MIN_MYSEKAI_RANK_RELEASE: i32 = 2;
/// `MysekaiRankReleaseType.mysekai_site_level` as the master names it.
const SITE_LEVEL: &str = "mysekai_site_level";
/// The site-type values `RemoveSiteExpansionTopic` walks (`home_site`,
/// `first_floor`, `second_floor`, `third_floor` = 0..3). It passes each
/// value where a site id is expected, so it checks site ids 0 to 3: no site,
/// home, the first floor and the second floor. The third floor (id 4) never
/// keeps the topic.
const REMOVE_TOPIC_SITE_ARGUMENTS: [i32; 4] = [0, 1, 2, 3];

#[derive(Clone, Debug)]
pub(crate) struct SiteLevelRow {
    pub(crate) id: i32,
    pub(crate) site_id: i32,
    pub(crate) level: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct RankReleaseRow {
    pub(crate) rank: i32,
    pub(crate) kind: String,
    pub(crate) external_id: i32,
}

/// A `mysekaiCutScenes` row.
#[derive(Clone, Debug)]
pub(crate) struct CutSceneRow {
    pub(crate) id: i32,
    pub(crate) external_id: i32,
    pub(crate) condition: String,
    pub(crate) bundle: String,
}

/// The master rows the expansion reads, in master order.
#[derive(Clone, Debug, Default)]
pub(crate) struct Masters {
    pub(crate) site_types: Vec<(i32, String)>,
    pub(crate) levels: Vec<SiteLevelRow>,
    pub(crate) releases: Vec<RankReleaseRow>,
    /// `mysekaiCutScenes`; `None` when the table is missing (named once by
    /// the master layer).
    pub(crate) cutscenes: Option<Vec<CutSceneRow>>,
}

fn int(row: &Value, field: &str) -> Result<i32, String> {
    row[field]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("{field} is not an int"))
}

/// The rows of a plain master table.
fn rows(text: &str) -> Result<Vec<Value>, String> {
    moly_assets::json::master::rows(text)
}

/// The four master tables of [`Masters`] as one consumer reads them, each
/// under that consumer's name.
pub(crate) struct MasterTables {
    pub(crate) site_types: MasterTable<Vec<(i32, String)>>,
    pub(crate) levels: MasterTable<Vec<SiteLevelRow>>,
    pub(crate) releases: MasterTable<Vec<RankReleaseRow>>,
    pub(crate) cutscenes: MasterTable<Vec<CutSceneRow>>,
}

impl MasterTables {
    /// Requests the four tables.
    pub(crate) fn request(&self, masters: &mut MasterData) {
        masters.request(&self.site_types);
        masters.request(&self.levels);
        masters.request(&self.releases);
        masters.request(&self.cutscenes);
    }

    /// The tables once all four have resolved. A missing site, level or
    /// release table (named once by the master layer) is the error; a
    /// missing cut-scene table leaves the cut scenes out.
    pub(crate) fn take(&self, masters: &mut MasterData) -> Option<Result<Masters, String>> {
        let keys = [
            self.site_types.key(),
            self.levels.key(),
            self.releases.key(),
            self.cutscenes.key(),
        ];
        if keys.into_iter().any(|key| !masters.is_resolved(key)) {
            return None;
        }
        let site_types = masters.take(&self.site_types)?;
        let levels = masters.take(&self.levels)?;
        let releases = masters.take(&self.releases)?;
        let cutscenes = masters.take(&self.cutscenes)?;
        Some((|| {
            Ok(Masters {
                site_types: site_types.map_err(|error| error.to_string())?,
                levels: levels.map_err(|error| error.to_string())?,
                releases: releases.map_err(|error| error.to_string())?,
                cutscenes: cutscenes.ok(),
            })
        })())
    }
}

/// The table set of the site expansion.
pub(crate) const EXPANSION_TABLES: MasterTables = MasterTables {
    site_types: MasterTable {
        table: "mysekaiSites",
        name: "mysekaiSites (the site expansion)",
        parse: parse_site_types,
    },
    levels: MasterTable {
        table: "mysekaiSiteLevels",
        name: "mysekaiSiteLevels (the site expansion)",
        parse: parse_levels,
    },
    releases: MasterTable {
        table: "mysekaiRankReleases",
        name: "mysekaiRankReleases (the site expansion)",
        parse: parse_releases,
    },
    cutscenes: MasterTable {
        table: "mysekaiCutScenes",
        name: "mysekaiCutScenes (the site expansion)",
        parse: parse_cutscenes,
    },
};

/// The table set of the room's floor buttons.
pub(crate) const FLOOR_BUTTON_TABLES: MasterTables = MasterTables {
    site_types: MasterTable {
        table: "mysekaiSites",
        name: "mysekaiSites (the floor buttons)",
        parse: parse_site_types,
    },
    levels: MasterTable {
        table: "mysekaiSiteLevels",
        name: "mysekaiSiteLevels (the floor buttons)",
        parse: parse_levels,
    },
    releases: MasterTable {
        table: "mysekaiRankReleases",
        name: "mysekaiRankReleases (the floor buttons)",
        parse: parse_releases,
    },
    cutscenes: MasterTable {
        table: "mysekaiCutScenes",
        name: "mysekaiCutScenes (the floor buttons)",
        parse: parse_cutscenes,
    },
};

/// `mysekaiSites`: (id, mysekaiSiteType), master order.
pub(crate) fn parse_site_types(text: &str) -> Result<Vec<(i32, String)>, String> {
    rows(text)?
        .iter()
        .map(|row| {
            let kind = row["mysekaiSiteType"]
                .as_str()
                .ok_or("mysekaiSiteType is not a string")?;
            Ok((int(row, "id")?, kind.to_owned()))
        })
        .collect()
}

/// `mysekaiSiteLevels`, master order.
pub(crate) fn parse_levels(text: &str) -> Result<Vec<SiteLevelRow>, String> {
    rows(text)?
        .iter()
        .map(|row| {
            Ok(SiteLevelRow {
                id: int(row, "id")?,
                site_id: int(row, "mysekaiSiteId")?,
                level: int(row, "level")?,
            })
        })
        .collect()
}

/// `mysekaiRankReleases`, master order.
pub(crate) fn parse_releases(text: &str) -> Result<Vec<RankReleaseRow>, String> {
    rows(text)?
        .iter()
        .map(|row| {
            Ok(RankReleaseRow {
                rank: int(row, "mysekaiRank")?,
                // The master column carries the source's own misspelling.
                kind: row["mysekaiRankRelaseType"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
                external_id: int(row, "externalId")?,
            })
        })
        .collect()
}

/// `mysekaiCutScenes`, master order.
pub(crate) fn parse_cutscenes(text: &str) -> Result<Vec<CutSceneRow>, String> {
    rows(text)?
        .iter()
        .map(|row| {
            Ok(CutSceneRow {
                id: int(row, "id")?,
                external_id: int(row, "externalId")?,
                condition: row["mysekaiCutSceneConditionType"]
                    .as_str()
                    .ok_or("mysekaiCutSceneConditionType is not a string")?
                    .to_owned(),
                bundle: row["timelineAssetbundleName"]
                    .as_str()
                    .ok_or("timelineAssetbundleName is not a string")?
                    .to_owned(),
            })
        })
        .collect()
}

impl Masters {
    /// `MasterDataManager.GetMasterMysekaiCutScene(externalId, conditionType)`
    /// for the site-level condition: the first row with that external id and
    /// condition. `Err` when the catalog has no cut-scene table.
    pub(crate) fn site_level_cutscene(
        &self,
        external_id: i32,
    ) -> Result<Option<&CutSceneRow>, String> {
        let rows = self
            .cutscenes
            .as_ref()
            .ok_or("the mysekaiCutScenes table is missing")?;
        Ok(rows
            .iter()
            .find(|row| row.external_id == external_id && row.condition == SITE_LEVEL))
    }

    pub(crate) fn site_id(&self, site_type: &str) -> Option<i32> {
        self.site_types
            .iter()
            .find(|(_, kind)| kind == site_type)
            .map(|(id, _)| *id)
    }

    /// `GetMasterMysekaiRankReleases(from, to)`.
    fn rank_releases(&self, from: i32, to: i32) -> impl Iterator<Item = &RankReleaseRow> {
        self.releases
            .iter()
            .filter(move |row| from < row.rank && row.rank <= to)
    }

    /// `GetMasterMysekaiSiteLevel(id)`.
    pub(crate) fn site_level(&self, id: i32) -> Option<&SiteLevelRow> {
        self.levels.iter().find(|row| row.id == id)
    }

    /// `GetUnlockedMasterMysekaiRankRelease(rank, siteId)`.
    pub(crate) fn unlocked_rank_release(&self, rank: i32, site_id: i32) -> Option<&RankReleaseRow> {
        let rows: Vec<&RankReleaseRow> = self
            .rank_releases(1, rank)
            .filter(|row| row.kind == SITE_LEVEL)
            .filter(|row| {
                self.site_level(row.external_id)
                    .is_some_and(|level| level.site_id == site_id)
            })
            .collect();
        let top = rows.iter().map(|row| row.rank).max()?;
        rows.into_iter().find(|row| row.rank == top)
    }

    /// `GetMasterMysekaiSiteLevel(siteId, rank)`: among the site's levels in
    /// master order whose id a site-level release of rank at most `rank`
    /// names, the first with the greatest level.
    pub(crate) fn master_site_level(&self, site_id: i32, rank: i32) -> Option<&SiteLevelRow> {
        let released: Vec<i32> = self
            .releases
            .iter()
            .filter(|row| row.kind == SITE_LEVEL && row.rank <= rank)
            .map(|row| row.external_id)
            .collect();
        let mut best: Option<&SiteLevelRow> = None;
        let mut best_level = -1;
        for row in self.levels.iter().filter(|row| row.site_id == site_id) {
            if released.contains(&row.id) && best_level < row.level {
                best = Some(row);
                best_level = row.level;
            }
        }
        best
    }

    /// `MysekaiUtility.GetMysekaiSiteLevel(siteId)`: 0 when no level is
    /// released.
    pub(crate) fn mysekai_site_level(&self, site_id: i32, rank: i32) -> i32 {
        self.master_site_level(site_id, rank)
            .map_or(0, |row| row.level)
    }
}

/// The client-local part of the application's local settings the
/// expansion reads and writes: `MysekaiTopics` (topic values) and
/// `UnlockedMasterSiteLevelIds`. State of the web save layer.
#[derive(Clone, Debug, Default)]
pub(crate) struct LocalLists {
    pub(crate) topics: Vec<i32>,
    pub(crate) unlocked_master_site_level_ids: Vec<i32>,
}

impl LocalLists {
    /// `HasTopic`.
    pub(crate) fn has_topic(&self, topic: i32) -> bool {
        self.topics.contains(&topic)
    }

    /// `AddTopicIfNeeded(prev, current)` (its site-expansion half): true when
    /// the topic was added by this call.
    pub(crate) fn add_topic_if_needed(
        &mut self,
        masters: &Masters,
        prev: i32,
        current: i32,
    ) -> bool {
        if prev >= current || current < MIN_MYSEKAI_RANK_RELEASE {
            return false;
        }
        let any = masters
            .rank_releases(prev, current)
            .any(|row| row.kind == SITE_LEVEL);
        if !any || self.has_topic(TOPIC_SITE_EXPANSION) {
            return false;
        }
        self.topics.push(TOPIC_SITE_EXPANSION);
        true
    }

    /// `GetUnlockMasterMysekaiSiteLevel(rank, siteId)`.
    pub(crate) fn unlock_master_site_level<'a>(
        &self,
        masters: &'a Masters,
        rank: i32,
        site_id: i32,
    ) -> Option<&'a SiteLevelRow> {
        let release = masters.unlocked_rank_release(rank, site_id)?;
        if self
            .unlocked_master_site_level_ids
            .contains(&release.external_id)
        {
            return None;
        }
        masters.site_level(release.external_id)
    }

    /// `SiteActionExecutor.IsNeedSiteExpansionAction(siteId)`; the player is
    /// always the room owner here.
    pub(crate) fn is_need_site_expansion_action(
        &self,
        masters: &Masters,
        rank: i32,
        site_id: i32,
    ) -> bool {
        self.has_topic(TOPIC_SITE_EXPANSION)
            && self
                .unlock_master_site_level(masters, rank, site_id)
                .is_some()
    }

    /// `SaveUnlockSiteLevel`.
    pub(crate) fn save_unlock_site_level(&mut self, id: i32) {
        if !self.unlocked_master_site_level_ids.contains(&id) {
            self.unlocked_master_site_level_ids.push(id);
        }
    }

    /// `RemoveSiteExpansionTopic`: true when the topic was removed.
    pub(crate) fn remove_site_expansion_topic(&mut self, masters: &Masters, rank: i32) -> bool {
        if REMOVE_TOPIC_SITE_ARGUMENTS.iter().any(|site| {
            self.unlock_master_site_level(masters, rank, *site)
                .is_some()
        }) {
            return false;
        }
        let before = self.topics.len();
        self.topics.retain(|topic| *topic != TOPIC_SITE_EXPANSION);
        self.topics.len() != before
    }

    /// `GetSiteUnlockedLevel(siteId)`: the greatest level of the list's rows
    /// of that site, else 1.
    pub(crate) fn site_unlocked_level(&self, masters: &Masters, site_id: i32) -> i32 {
        self.unlocked_master_site_level_ids
            .iter()
            .filter_map(|id| masters.site_level(*id))
            .filter(|row| row.site_id == site_id)
            .map(|row| row.level)
            .max()
            .unwrap_or(1)
    }

    /// `MysekaiUtility.GetMysekaiSiteUnlockedLevel(siteId, siteType)`: with
    /// the topic, home (type 0) and the first floor (type 1) show the local
    /// list's level; everything else shows the rank's level.
    pub(crate) fn mysekai_site_unlocked_level(
        &self,
        masters: &Masters,
        rank: i32,
        site_id: i32,
        site_type: &str,
    ) -> i32 {
        let listed = matches!(site_type, "home_site" | "first_floor");
        if listed && self.has_topic(TOPIC_SITE_EXPANSION) {
            self.site_unlocked_level(masters, site_id)
        } else {
            masters.mysekai_site_level(site_id, rank)
        }
    }
}
