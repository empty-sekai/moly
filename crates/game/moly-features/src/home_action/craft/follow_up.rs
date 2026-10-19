//! The craft's follow-up dialogs: the fixture bonus (407), the rank-up
//! (336) and the rank's release dialogs, the player status update (344) and
//! the new harvest site (410).
//!
//! The source runs them after the result dialog's close, one at a time:
//! `ShowFixtureBonusDialog` when the crafted fixture's bonus exists, then
//! `OpenMysekaiRankDialogIfNeededAsync` (the rank-up when the rank changed),
//! then, for every rank between, `OpenMysekaiRankReleaseDialogAsync`: the
//! site map's new-harvest-site direction first, then each release row (the
//! status update). Every dialog is awaited to its close before the next one.
//!
//! What each draws:
//! - 407: the character icon (not built: the 2D character model and its
//!   texture are not loaded), and two `FORMAT_ADD_PARAM` texts, the bonus
//!   rate before and after the craft.
//! - 336: the rank numbers before and after (the title texture and the
//!   particle/animation are not loaded).
//! - 344: the release's icon (not loaded), the release message, and the
//!   before/after values (hidden when both are empty).
//! - 410: the harvest site's banner (not loaded) and
//!   `MSG_MYSEKAI_OPEN_SITE_MESSAGE(siteName)`.
//!
//! The close button (and the window base, where the prefab lists it) and the
//! back key close each; the close is the awaited callback.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::Value;

use super::CraftScreen;
use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{DialogBackKeyEvent, DialogId, DialogType};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// `MysekaiRankUpDialog`'s document.
const KEY_RANK_UP: &str = "CraftRankUp";
/// `FixtureBonusSubWindowDialog`'s document.
const KEY_BONUS: &str = "CraftFixtureBonus";
/// `MysekaiPlayerStatusUpdateDialog`'s document.
const KEY_STATUS: &str = "CraftStatus";
/// `MysekaiNewHarvestSiteOpenDialog`'s document.
const KEY_NEW_SITE: &str = "CraftNewSite";

const RANK_UP_DIALOG: DialogType = DialogType::MysekaiRankUpDialog;
const FIXTURE_BONUS_DIALOG: DialogType = DialogType::FixtureBonusSubWindowDialog;
const STATUS_DIALOG: DialogType = DialogType::MysekaiPlayerStatusUpdateDialog;
const NEW_SITE_DIALOG: DialogType = DialogType::MysekaiNewHarvestSiteOpenDialog;

/// `MysekaiTopicEnum.HarvestSiteDialogReleased`.
const TOPIC_HARVEST_SITE_DIALOG: i32 = 3;

/// The bonus the crafted fixture gives, captured before the request so the
/// dialog can show the rate before and after.
#[derive(Clone, Debug)]
pub(super) struct BonusStart {
    /// The summed bonus rate before the craft.
    before: f32,
    /// `MasterMysekaiFixtureGameCharacterGroupPerformanceBonus`'s target
    /// character, the one the sum is over.
    character: i64,
}

/// What one follow-up dialog shows. The view draws the current one.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FollowUp {
    RankUp {
        before: i32,
        after: i32,
    },
    Bonus {
        /// `FORMAT_ADD_PARAM` texts, already formatted.
        before: String,
        after: String,
    },
    Status {
        /// The release message's wording key.
        message: &'static str,
        before: String,
        after: String,
        /// Both values are non-empty: the change row is on.
        diff: bool,
        /// The `mysekai/thumbnail/player_status/` resource name.
        icon: String,
    },
    NewSite {
        site_id: i32,
        site_name: String,
    },
}

impl FollowUp {
    fn kind(&self) -> FollowUpKind {
        match self {
            Self::RankUp { .. } => FollowUpKind::RankUp,
            Self::Bonus { .. } => FollowUpKind::Bonus,
            Self::Status { .. } => FollowUpKind::Status,
            Self::NewSite { .. } => FollowUpKind::NewSite,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FollowUpKind {
    RankUp,
    Bonus,
    Status,
    NewSite,
}

// ---------------------------------------------------------------------------
// The master rows the release dialogs and the bonus dialog read
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct FixtureRow {
    id: i64,
    /// `mysekaiFixtureGameCharacterGroupPerformanceBonusId`, 0 without one.
    bonus_id: i64,
}

#[derive(Clone, Debug)]
struct BonusRow {
    id: i64,
    group: i64,
    rate: i64,
}

#[derive(Clone, Debug)]
struct GroupRow {
    group: i64,
    character: i64,
}

#[derive(Clone, Debug)]
struct ReleaseRow {
    rank: i64,
    kind: String,
    external: i64,
}

#[derive(Clone, Debug)]
struct LevelRow {
    id: i64,
    site: i64,
}

#[derive(Clone, Debug)]
struct SiteRow {
    id: i64,
    category: String,
    name: String,
}

#[derive(Clone, Debug)]
struct PutLimitRow {
    id: i64,
    put_limit: i64,
    category: String,
}

#[derive(Default, Debug)]
struct Masters {
    fixtures: Vec<FixtureRow>,
    bonuses: Vec<BonusRow>,
    groups: Vec<GroupRow>,
    limit: i64,
    releases: Vec<ReleaseRow>,
    levels: Vec<LevelRow>,
    sites: Vec<SiteRow>,
    put_limits: Vec<PutLimitRow>,
}

#[derive(Resource, Default)]
pub struct FollowUpMasters {
    masters: Option<Masters>,
    /// A table failed to read: named once, the dialogs that need it stay off.
    failed: bool,
}

const BONUSES: MasterTable<Vec<BonusRow>> = MasterTable {
    table: "mysekaiFixtureGameCharacterGroupPerformanceBonuses",
    name: "mysekaiFixtureGameCharacterGroupPerformanceBonuses (the craft's fixture bonus)",
    parse: parse_bonuses,
};
const GROUPS: MasterTable<Vec<GroupRow>> = MasterTable {
    table: "mysekaiFixtureGameCharacterGroups",
    name: "mysekaiFixtureGameCharacterGroups (the craft's fixture bonus)",
    parse: parse_groups,
};
const LIMIT: MasterTable<Vec<i64>> = MasterTable {
    table: "mysekaiFixtureGameCharacterPerformanceBonusLimit",
    name: "mysekaiFixtureGameCharacterPerformanceBonusLimit (the craft's fixture bonus)",
    parse: parse_limit,
};
const RELEASES: MasterTable<Vec<ReleaseRow>> = MasterTable {
    table: "mysekaiRankReleases",
    name: "mysekaiRankReleases (the craft's rank release dialogs)",
    parse: parse_releases,
};
const LEVELS: MasterTable<Vec<LevelRow>> = MasterTable {
    table: "mysekaiSiteLevels",
    name: "mysekaiSiteLevels (the craft's rank release dialogs)",
    parse: parse_levels,
};
const SITES: MasterTable<Vec<SiteRow>> = MasterTable {
    table: "mysekaiSites",
    name: "mysekaiSites (the craft's rank release dialogs)",
    parse: parse_sites,
};
const PUT_LIMITS: MasterTable<Vec<PutLimitRow>> = MasterTable {
    table: "mysekaiFixturePutLimitLevels",
    name: "mysekaiFixturePutLimitLevels (the craft's rank release dialogs)",
    parse: parse_put_limits,
};

pub fn request_masters(masters: &mut MasterData) {
    masters.request(&BONUSES);
    masters.request(&GROUPS);
    masters.request(&LIMIT);
    masters.request(&RELEASES);
    masters.request(&LEVELS);
    masters.request(&SITES);
    masters.request(&PUT_LIMITS);
}

pub fn resolve(
    mut masters: ResMut<FollowUpMasters>,
    mut data: ResMut<MasterData>,
    shared: Option<Res<moly_masters::load::MasterTables>>,
) {
    if masters.masters.is_some() || masters.failed {
        return;
    }
    let calls = [BONUSES.key(), GROUPS.key(), LIMIT.key()];
    let releases = [RELEASES.key(), LEVELS.key(), SITES.key(), PUT_LIMITS.key()];
    if calls
        .iter()
        .chain(releases.iter())
        .copied()
        .any(|key| !data.is_resolved(key))
    {
        return;
    }
    let Some(shared) = shared else {
        return;
    };
    if !shared.rows_ready() {
        return;
    }
    let Some(rows) = shared.rows.get("mysekaiFixtures") else {
        masters.failed = true;
        error!(
            "[craft] the shared fixture rows are absent ({}); the bonus dialog stays off",
            shared
                .missing
                .iter()
                .find(|line| line.starts_with("mysekaiFixtures"))
                .cloned()
                .unwrap_or_else(|| "not in the runtime root".to_owned())
        );
        return;
    };
    let fixtures = match parse_fixture_rows(rows) {
        Ok(fixtures) => fixtures,
        Err(reason) => {
            masters.failed = true;
            error!("[craft] the shared fixture rows are not readable ({reason}); the bonus dialog stays off");
            return;
        }
    };
    let bonuses = match data.take(&BONUSES) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] the bonus rows are not readable ({error}); the bonus dialog stays off");
            return;
        }
        None => return,
    };
    let groups = match data.take(&GROUPS) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] the character groups are not readable ({error}); the bonus dialog stays off");
            return;
        }
        None => return,
    };
    let limit = match data.take(&LIMIT) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] the bonus limit is not readable ({error}); the bonus dialog stays off");
            return;
        }
        None => return,
    };
    let releases = match data.take(&RELEASES) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] mysekaiRankReleases is not readable ({error}); the release dialogs stay off");
            return;
        }
        None => return,
    };
    let levels = match data.take(&LEVELS) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] mysekaiSiteLevels is not readable ({error}); the release dialogs stay off");
            return;
        }
        None => return,
    };
    let sites = match data.take(&SITES) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] mysekaiSites is not readable ({error}); the release dialogs stay off");
            return;
        }
        None => return,
    };
    let put_limits = match data.take(&PUT_LIMITS) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            masters.failed = true;
            error!("[craft] mysekaiFixturePutLimitLevels is not readable ({error}); the release dialogs stay off");
            return;
        }
        None => return,
    };
    let read = Masters {
        fixtures,
        bonuses,
        groups,
        limit: limit.first().copied().unwrap_or(0),
        releases,
        levels,
        sites,
        put_limits,
    };
    info!(
        "[craft] follow-up masters: {} shared fixtures, {} bonus rows, {} groups, limit {}, {} releases, {} levels, {} sites, {} put limits",
        read.fixtures.len(),
        read.bonuses.len(),
        read.groups.len(),
        read.limit,
        read.releases.len(),
        read.levels.len(),
        read.sites.len(),
        read.put_limits.len()
    );
    masters.masters = Some(read);
}

fn parse_fixture_rows(rows: &[Value]) -> Result<Vec<FixtureRow>, String> {
    rows.iter()
        .map(|row| {
            Ok(FixtureRow {
                id: master::int(row, "id")?,
                bonus_id: row["mysekaiFixtureGameCharacterGroupPerformanceBonusId"]
                    .as_i64()
                    .unwrap_or(0),
            })
        })
        .collect()
}

fn parse_bonuses(text: &str) -> Result<Vec<BonusRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(BonusRow {
                id: master::int(row, "id")?,
                group: master::int(row, "mysekaiFixtureGameCharacterGroupId")?,
                rate: master::int(row, "bonusRate")?,
            })
        })
        .collect()
}

fn parse_groups(text: &str) -> Result<Vec<GroupRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(GroupRow {
                group: master::int(row, "groupId")?,
                character: master::int(row, "gameCharacterId")?,
            })
        })
        .collect()
}

fn parse_limit(text: &str) -> Result<Vec<i64>, String> {
    let row = Value::Object(master::object(text)?);
    Ok(vec![master::int(&row, "bonusRateLimit")?])
}

fn parse_releases(text: &str) -> Result<Vec<ReleaseRow>, String> {
    let mut rows = crate::site_expansion::law::rank_release_rows(master::rows(text)?);
    rows.sort_by_key(|row| row["mysekaiRank"].as_i64().unwrap_or(0));
    rows.iter()
        .map(|row| {
            Ok(ReleaseRow {
                rank: master::int(row, "mysekaiRank")?,
                kind: master::text(row, "mysekaiRankRelaseType")?.to_owned(),
                external: master::int(row, "externalId")?,
            })
        })
        .collect()
}

fn parse_levels(text: &str) -> Result<Vec<LevelRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(LevelRow {
                id: master::int(row, "id")?,
                site: master::int(row, "mysekaiSiteId")?,
            })
        })
        .collect()
}

fn parse_sites(text: &str) -> Result<Vec<SiteRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(SiteRow {
                id: master::int(row, "id")?,
                category: master::text(row, "mysekaiSiteCategory")?.to_owned(),
                name: master::text(row, "name")?.to_owned(),
            })
        })
        .collect()
}

fn parse_put_limits(text: &str) -> Result<Vec<PutLimitRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(PutLimitRow {
                id: master::int(row, "id")?,
                put_limit: master::int(row, "putCostLimit")?,
                category: master::text(row, "mysekaiSiteCategory")?.to_owned(),
            })
        })
        .collect()
}

fn masters(world: &World) -> Option<&Masters> {
    world.get_resource::<FollowUpMasters>()?.masters.as_ref()
}

// ---------------------------------------------------------------------------
// The bonus dialog's data
// ---------------------------------------------------------------------------

/// `HasFixtureBonus` and the before rate: the fixture's bonus row and the sum
/// over the owned fixtures of its target character, before the request.
pub(super) fn bonus_start(world: &World, fixture_id: i32) -> Option<BonusStart> {
    let masters = masters(world)?;
    let row = masters.fixtures.iter().find(|row| row.id == i64::from(fixture_id))?;
    let bonus = masters.bonuses.iter().find(|bonus| bonus.id == row.bonus_id)?;
    let group = masters.groups.iter().find(|group| group.group == bonus.group)?;
    let before = bonus_sum(world, masters, group.character)?;
    Some(BonusStart {
        before,
        character: group.character,
    })
}

fn bonus_sum(world: &World, masters: &Masters, character: i64) -> Option<f32> {
    let inventory = world.get_resource::<crate::userdata::inventory::ClientMysekaiInventory>()?;
    let mut sum = 0.0f32;
    for fixture in inventory.fixtures() {
        let Some(row) = masters
            .fixtures
            .iter()
            .find(|row| row.id == i64::from(fixture.mysekai_fixture_id))
        else {
            continue;
        };
        let Some(bonus) = masters.bonuses.iter().find(|bonus| bonus.id == row.bonus_id) else {
            continue;
        };
        let Some(group) = masters.groups.iter().find(|group| group.group == bonus.group) else {
            continue;
        };
        if group.character == character {
            sum += bonus.rate as f32;
        }
    }
    Some(sum.min(masters.limit as f32))
}

/// The bonus dialog's texts: `Setup` formats `FORMAT_ADD_PARAM` with the
/// before and after rates (one decimal, as the source truncates/rounds to).
pub(super) fn bonus_dialog(world: &World, start: &BonusStart) -> Option<FollowUp> {
    let masters = masters(world)?;
    let after = bonus_sum(world, masters, start.character)?;
    Some(FollowUp::Bonus {
        before: format!("{:.1}", start.before),
        after: format!("{:.1}", after),
    })
}

// ---------------------------------------------------------------------------
// The rank release dialogs
// ---------------------------------------------------------------------------

/// The release dialogs for the ranks `before + 1 ..= after`, in the order the
/// source awaits them: the site map's new harvest site (when its topic is
/// set), then each rank's release rows in master order.
pub(super) fn release_dialogs(world: &mut World, before: i32, after: i32) -> Vec<FollowUp> {
    let (site, mut out) = {
        let Some(masters) = masters(world) else {
            return Vec::new();
        };
        let site = latest_harvest_site(masters, after);
        let mut out = Vec::new();
        for rank in (before + 1)..=after {
            for row in masters.releases.iter().filter(|row| row.rank == i64::from(rank)) {
                match row.kind.as_str() {
                    // The rank loop opens only the put-limit rows; a site
                    // level release is opened when the site is entered.
                    "mysekai_site_level" => {}
                    "mysekai_fixture_put_limit" => {
                        if let Some(status) = put_limit_release(masters, row) {
                            out.push(status);
                        }
                    }
                    other => warn!("[craft] rank release {} is {other}, not read", row.external),
                }
            }
        }
        (site, out)
    };
    let topic = world
        .get_resource::<crate::site_expansion::MysekaiLocalSettings>()
        .is_some_and(|local| local.0.topics.contains(&TOPIC_HARVEST_SITE_DIALOG));
    if topic {
        match site {
            Some((site_id, site_name)) => out.insert(0, FollowUp::NewSite { site_id, site_name }),
            None => warn!(
                "[craft] HarvestSiteDialogReleased is set and rank {after} releases no harvest site; the new-site dialog does not show"
            ),
        }
        if let Some(mut local) = world.get_resource_mut::<crate::site_expansion::MysekaiLocalSettings>() {
            local.0.topics.retain(|topic| *topic != TOPIC_HARVEST_SITE_DIALOG);
        }
    }
    out
}

/// `HarvestUtility.GetLatestReleaseSiteId`: among the rank's site-level
/// releases whose site is a harvest site, the last one's site id and name.
fn latest_harvest_site(masters: &Masters, rank: i32) -> Option<(i32, String)> {
    let mut latest = None;
    for row in masters.releases.iter().filter(|row| row.rank == i64::from(rank) && row.kind == "mysekai_site_level") {
        let Some(level) = masters.levels.iter().find(|level| level.id == row.external) else {
            continue;
        };
        let Some(site) = masters.sites.iter().find(|site| site.id == level.site) else {
            continue;
        };
        if site.category == "harvest" {
            latest = Some((i32::try_from(site.id).ok()?, site.name.clone()));
        }
    }
    latest
}

fn put_limit_release(masters: &Masters, row: &ReleaseRow) -> Option<FollowUp> {
    let level = masters
        .put_limits
        .iter()
        .find(|level| level.id == row.external)?;
    let previous = masters
        .releases
        .iter()
        .filter(|other| other.rank < row.rank && other.kind == "mysekai_fixture_put_limit")
        .filter_map(|other| {
            masters
                .put_limits
                .iter()
                .find(|level| level.id == other.external)
                .map(|level| (other.rank, level))
        })
        .filter(|(_, other)| other.category == level.category)
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, level)| level);
    let Some(previous) = previous else {
        warn!(
            "[craft] put-limit release {} has no previous {} level; the release dialog is skipped",
            level.id, level.category
        );
        return None;
    };
    let (message, icon) = if level.category == "housing_room" {
        (
            "MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST_UP",
            "item_fixture_joint_release_put_limit",
        )
    } else {
        (
            "MSG_MYSEKAI_FIXTURE_HOME_LAYOUT_COST_UP",
            "item_fixture_release_put_limit",
        )
    };
    Some(FollowUp::Status {
        message,
        before: previous.put_limit.to_string(),
        after: level.put_limit.to_string(),
        diff: true,
        icon: icon.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Opening and closing (through the craft screen's own dialog helpers)
// ---------------------------------------------------------------------------

/// `ShowSubWindowDialog` for one follow-up. The new harvest site removes its
/// topic before it opens, as `PlaySiteMapOpenDirectionIfNeeded` does.
pub(super) fn open(world: &mut World, follow_up: &FollowUp) -> Option<DialogId> {
    let (dialog, caller) = match follow_up {
        FollowUp::RankUp { .. } => (RANK_UP_DIALOG, "MysekaiRankUtility.OpenMysekaiRankUpDialogAsync"),
        FollowUp::Bonus { .. } => (FIXTURE_BONUS_DIALOG, "CraftPreview.ShowFixtureBonusDialog"),
        FollowUp::Status { .. } => (STATUS_DIALOG, "MysekaiRankUtility.OpenMysekaiRankReleaseDialogAsync"),
        FollowUp::NewSite { .. } => {
            if let Some(mut local) = world.get_resource_mut::<crate::site_expansion::MysekaiLocalSettings>() {
                local.0.topics.retain(|topic| *topic != TOPIC_HARVEST_SITE_DIALOG);
            }
            (NEW_SITE_DIALOG, "MysekaiUtility.PlaySiteMapOpenDirectionIfNeeded")
        }
    };
    if let FollowUp::Status { icon, .. } = follow_up {
        info!("[craft] status update {icon}: the thumbnail asset is not loaded");
    }
    if let FollowUp::NewSite { site_id, .. } = follow_up {
        info!("[craft] new harvest site {site_id}: the banner texture is not loaded");
    }
    info!("[craft] follow-up {follow_up:?}: ShowSubWindowDialog({dialog:?})");
    let id = super::show(world, dialog, caller);
    if id.is_some() {
        let cue = match follow_up {
            FollowUp::RankUp { .. } => Some("se_sekairank_up"),
            FollowUp::Status { .. } | FollowUp::NewSite { .. } => Some("se_get_blueprint"),
            FollowUp::Bonus { .. } => None,
        };
        if let Some(cue) = cue {
            world.resource_mut::<SeRequests>().0.push(SeRequest {
                owner: None,
                cue: cue.to_owned(),
                class: SeClass::Ui,
                source: "craft-follow-up",
            });
        }
    }
    id
}

// ---------------------------------------------------------------------------
// The views
// ---------------------------------------------------------------------------

fn component<'a>(doc: &'a UiPrefab, selector: &str, suffix: &str) -> Result<&'a UiComponent, String> {
    let node = &doc.nodes[doc.find(selector)?];
    node.components
        .iter()
        .find(|component| component.class.ends_with(suffix))
        .ok_or_else(|| format!("{} lacks source component {suffix}", node.path))
}

fn pointer(value: &Value) -> Result<i64, String> {
    let p = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or("expected a decoded PPtr")?;
    if p[0].as_i64() != Some(0) {
        return Err("the PPtr is external".into());
    }
    p[1].as_i64().ok_or_else(|| "invalid PPtr path id".into())
}

fn field(fields: &Value, name: &str) -> Result<String, String> {
    match pointer(&fields[name])? {
        0 => Err(format!("required source reference {name} is null")),
        id => Ok(format!("@{id}")),
    }
}

fn close_buttons(doc: &UiPrefab, dialog: &Value) -> Result<Vec<String>, String> {
    let window = component(doc, &field(dialog, "subWindowComponent")?, ".SubWindowComponent")?
        .fields
        .clone();
    window["closeButtons"]
        .as_array()
        .ok_or("SubWindowComponent closeButtons")?
        .iter()
        .map(|button| match pointer(button)? {
            0 => Err("a null close button".to_owned()),
            id => Ok(format!("@{id}")),
        })
        .collect()
}

struct Bindings {
    /// The texts per kind; unused paths are empty.
    before: String,
    after: String,
    description: String,
    /// The sub window's message body (405's title).
    title: String,
    changes: String,
    close: Vec<String>,
}

fn bind(doc: &UiPrefab, kind: FollowUpKind) -> Result<Bindings, String> {
    let root = doc
        .nodes
        .iter()
        .find(|node| node.path == doc.prefab)
        .ok_or("the document has no root node")?;
    let dialog = root
        .components
        .iter()
        .find(|component| match kind {
            FollowUpKind::RankUp => component.class.ends_with(".MysekaiRankUpDialog"),
            FollowUpKind::Bonus => component.class.ends_with(".FixtureBonusSubWindowDialog"),
            FollowUpKind::Status => component.class.ends_with(".MysekaiPlayerStatusUpdateDialog"),
            FollowUpKind::NewSite => component.class.ends_with(".MysekaiNewHarvestSiteOpenDialog"),
        })
        .ok_or("the root has no source dialog component")?
        .fields
        .clone();
    let (before, after) = match kind {
        FollowUpKind::RankUp => (
            field(&dialog, "_beforeRankText")?,
            field(&dialog, "_afterRankText")?,
        ),
        FollowUpKind::Bonus => (
            field(&dialog, "_beforeBonusText")?,
            field(&dialog, "_afterBonusText")?,
        ),
        FollowUpKind::Status => (
            field(&dialog, "_beforeValueText")?,
            field(&dialog, "_afterValueText")?,
        ),
        FollowUpKind::NewSite => (String::new(), String::new()),
    };
    let description = match kind {
        FollowUpKind::Status | FollowUpKind::NewSite => field(&dialog, "_description")?,
        FollowUpKind::RankUp | FollowUpKind::Bonus => String::new(),
    };
    let title = if kind == FollowUpKind::Bonus {
        field(&dialog, "messageBodyText")?
    } else {
        String::new()
    };
    let changes = if kind == FollowUpKind::Status {
        field(&dialog, "_changesLayout")?
    } else {
        String::new()
    };
    Ok(Bindings {
        before,
        after,
        description,
        title,
        changes,
        close: close_buttons(doc, &dialog)?,
    })
}

#[derive(Component)]
pub struct FollowUpView {
    kind: FollowUpKind,
    bindings: Bindings,
}

/// Spawn each view once its document is ready; a root without one names it.
pub fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    existing: Query<&FollowUpView>,
) {
    for (kind, key) in [
        (FollowUpKind::RankUp, KEY_RANK_UP),
        (FollowUpKind::Bonus, KEY_BONUS),
        (FollowUpKind::Status, KEY_STATUS),
        (FollowUpKind::NewSite, KEY_NEW_SITE),
    ] {
        if existing.iter().any(|view| view.kind == kind) || !layouts.ready(key, &server) {
            continue;
        }
        let Some(doc) = layouts.document(key) else {
            continue;
        };
        match bind(doc, kind) {
            Ok(bindings) => {
                info!("[craft] {key} view spawned");
                commands.spawn((
                    FollowUpView { kind, bindings },
                    Visibility::Hidden,
                    Transform::from_xyz(0., 0., 30.),
                    RenderLayers::layer(SITEMAP_LAYER),
                    UiPrefabView::new(key, SITEMAP_LAYER),
                ));
            }
            Err(reason) => error!("[craft] {key} could not be bound ({reason}); it is shown without a view"),
        }
    }
}

fn wording(layouts: &UiLayouts, key: &str, args: &[String]) -> Option<String> {
    let format = layouts.wordings.get(key)?;
    moly_law::text::custom_text_mesh::format_wording(format, args).ok()
}

fn draw(layouts: &UiLayouts, view: &mut UiPrefabView, bindings: &Bindings, follow_up: &FollowUp) {
    match follow_up {
        FollowUp::RankUp { before, after } => {
            view.set_text(&bindings.before, before.to_string());
            view.set_text(&bindings.after, after.to_string());
        }
        FollowUp::Bonus { before, after } => {
            if let Some(title) = wording(layouts, "MSG_FIXTURE_BONUS_UP_DIALOG", &[]) {
                view.set_text(&bindings.title, title);
            }
            let before = wording(layouts, "FORMAT_ADD_PARAM", &[before.clone()])
                .unwrap_or_else(|| before.clone());
            let after = wording(layouts, "FORMAT_ADD_PARAM", &[after.clone()])
                .unwrap_or_else(|| after.clone());
            view.set_text(&bindings.before, before);
            view.set_text(&bindings.after, after);
        }
        FollowUp::Status { message, before, after, diff, .. } => {
            if let Some(text) = wording(layouts, message, &[]) {
                view.set_text(&bindings.description, text);
            }
            view.set_text(&bindings.before, before.clone());
            view.set_text(&bindings.after, after.clone());
            view.set_visible(&bindings.changes, *diff);
        }
        FollowUp::NewSite { site_name, .. } => {
            if let Some(text) = wording(layouts, "MSG_MYSEKAI_OPEN_SITE_MESSAGE", &[site_name.clone()]) {
                view.set_text(&bindings.description, text);
            }
        }
    }
}

/// Show the current follow-up's view and draw it; hide the others.
pub fn place(
    screen: Res<CraftScreen>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut views: Query<(&FollowUpView, &mut Visibility, &mut Transform, &mut UiPrefabView)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let scale = root_canvas.scale(window);
    let shown = screen.follow_up.as_ref();
    for (view, mut visible, mut transform, mut prefab) in &mut views {
        let active = shown.is_some_and(|follow_up| follow_up.kind() == view.kind);
        visible.set_if_neq(if active { Visibility::Inherited } else { Visibility::Hidden });
        if active {
            crate::canvas::set_scale(&mut transform, scale);
            draw(&layouts, &mut prefab, &view.bindings, shown.expect("active follow-up"));
        }
    }
}

/// Taps and the back key while a follow-up is shown: a close button or the
/// back key closes it (the awaited close).
#[allow(clippy::too_many_arguments)]
pub fn click(
    mut screen: ResMut<CraftScreen>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<(&FollowUpView, &UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let (Some(dialog), Some(follow_up)) = (screen.dialog, screen.follow_up.clone()) else {
        return;
    };
    if screen.answer.is_some() {
        return;
    }
    if back_keys.read().any(|event| event.id == dialog) {
        info!("[craft] {follow_up:?}: back key: Close");
        screen.answer = Some(true);
        return;
    }
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    let Ok((view, prefab)) = views
        .iter()
        .find(|(view, _)| view.kind == follow_up.kind())
        .ok_or(())
    else {
        return;
    };
    for position in taps {
        consumed.0 = true;
        let canvas = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        let hit = view.bindings.close.iter().any(|path| {
            prefab
                .rect(&layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        });
        if hit {
            info!("[craft] {follow_up:?}: tap on a close button: Close");
            screen.answer = Some(true);
            break;
        }
    }
}
