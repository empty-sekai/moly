//! Shared master put-cost rules for layout editors and the rank page.
//! Missing master columns stay unavailable instead of granting free capacity.
use bevy::prelude::*;
use moly_assets::json::{JsonAsset, master::{self, MasterData, MasterTable}};
use std::collections::{HashMap, HashSet};
use crate::fixture::EditableFixture;

/// (mysekaiRank, externalId) of the `mysekai_fixture_put_limit` rank
/// releases, in master row order.
const PUT_LIMIT_RELEASES: MasterTable<Vec<(i64, i64)>> = MasterTable {
    table: "mysekaiRankReleases",
    name: "mysekaiRankReleases (fixture put-cost limits)",
    parse: parse_put_limit_releases,
};

/// The put-limit level rows, in master row order.
const PUT_LIMIT_LEVELS: MasterTable<Vec<PutLimitLevel>> = MasterTable {
    table: "mysekaiFixturePutLimitLevels",
    name: "mysekaiFixturePutLimitLevels (fixture put-cost limits)",
    parse: parse_put_limit_levels,
};

/// `MysekaiSiteCategory` of a put-limit level row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SiteCategory {
    HousingHome,
    HousingRoom,
    Other,
}

#[derive(Clone)]
struct PutLimitLevel {
    id: i64,
    level: i64,
    put_cost_limit: i64,
    category: SiteCategory,
}

/// The master rows `GetMasterMysekaiFixturePutLimitLevelFromMysekaiRankRelease`
/// reads, in master row order.
#[derive(Clone)]
pub(crate) struct PutLimitTables {
    /// (mysekaiRank, externalId) of the `mysekai_fixture_put_limit` releases.
    pub(crate) releases: Vec<(i64, i64)>,
    levels: Vec<PutLimitLevel>,
}

// Both put-limit tables refuse as a whole on a row they cannot read (the
// put-cost texts are then hidden) rather than dropping the row:
// a dropped release or level row would change the limit computed from the
// rows that remain.

fn parse_put_limit_releases(text: &str) -> Result<Vec<(i64, i64)>, String> {
    let mut releases = Vec::new();
    for row in crate::site_expansion::law::rank_release_rows(master::rows(text)?) {
        if master::text(&row, "mysekaiRankRelaseType")? == "mysekai_fixture_put_limit" {
            releases.push((master::int(&row, "mysekaiRank")?, master::int(&row, "externalId")?));
        }
    }
    Ok(releases)
}

fn parse_put_limit_levels(text: &str) -> Result<Vec<PutLimitLevel>, String> {
    master::rows(text)?.iter().map(|row| {
        let category = match master::text(row, "mysekaiSiteCategory")? {
            "housing_home" => SiteCategory::HousingHome,
            "housing_room" => SiteCategory::HousingRoom,
            _ => SiteCategory::Other,
        };
        Ok(PutLimitLevel { id: master::int(row, "id")?, level: master::int(row, "level")?, put_cost_limit: master::int(row, "putCostLimit")?, category })
    }).collect()
}

impl PutLimitTables {
    pub(crate) fn level_count(&self) -> usize { self.levels.len() }

    /// `GetFixturePutLimitCost(category)` at `rank`.
    pub(crate) fn cost(&self, rank: i64, category: SiteCategory) -> i64 {
        let ids: Vec<i64> = self.releases.iter().filter(|(release_rank, _)| *release_rank <= rank).map(|(_, id)| *id).collect();
        let mut best: Option<&PutLimitLevel> = None;
        for level in &self.levels {
            if level.category == category && ids.contains(&level.id) && best.is_none_or(|b| b.level < level.level) {
                best = Some(level);
            }
        }
        best.map_or(0, |level| level.put_cost_limit)
    }
}

/// The two tables once both have resolved; None while one still loads. A
/// table that is absent or not in the shape the JP rule reads is named once
/// by the master layer, and its error is kept.
fn resolve_put_limit(masters: &mut MasterData) -> Option<Result<PutLimitTables, String>> {
    masters.request(&PUT_LIMIT_RELEASES);
    masters.request(&PUT_LIMIT_LEVELS);
    if !masters.is_resolved(PUT_LIMIT_RELEASES.key()) || !masters.is_resolved(PUT_LIMIT_LEVELS.key()) {
        return None;
    }
    let (releases, levels) = (masters.take(&PUT_LIMIT_RELEASES)?, masters.take(&PUT_LIMIT_LEVELS)?);
    Some(match (releases, levels) {
        (Ok(releases), Ok(levels)) => Ok(PutLimitTables { releases, levels }),
        (Err(error), _) | (_, Err(error)) => Err(error.to_string()),
    })
}


#[derive(Clone, Copy, Debug)]
struct Costs { first: i32, second: i32 }

#[derive(Resource, Default)]
pub struct PutCostMaster {
    limits: Option<Result<PutLimitTables, String>>,
    fixtures: Option<Result<HashMap<i32, Costs>, String>>,
}

impl PutCostMaster {
    pub(crate) fn limits(&self) -> Option<&Result<PutLimitTables, String>> { self.limits.as_ref() }
}

#[derive(Resource)]
struct FixtureCostsRequest(Handle<JsonAsset>);

pub fn install(app: &mut App) {
    app.init_resource::<PutCostMaster>().add_systems(Startup, load).add_systems(Update, resolve);
}

fn load(mut commands: Commands, server: Res<AssetServer>, mut masters: ResMut<MasterData>) {
    masters.request(&PUT_LIMIT_RELEASES);
    masters.request(&PUT_LIMIT_LEVELS);
    commands.insert_resource(FixtureCostsRequest(server.load("moly://mysekai-fixtures.json")));
}

fn resolve(mut masters: ResMut<MasterData>, mut costs: ResMut<PutCostMaster>,
    request: Option<Res<FixtureCostsRequest>>, server: Res<AssetServer>, json: Res<Assets<JsonAsset>>) {
    if costs.limits.is_none() {
        costs.limits = resolve_put_limit(&mut masters);
    }
    if costs.fixtures.is_none() {
        if let Some(request) = request {
            if let bevy::asset::LoadState::Failed(error) = server.load_state(&request.0) {
                costs.fixtures = Some(Err(format!("fixture cost metadata failed: {error}")));
            } else if let Some(data) = json.get(&request.0) {
                costs.fixtures = Some(parse_costs(&data.0));
            }
        }
    }
}

fn parse_costs(text: &str) -> Result<HashMap<i32, Costs>, String> {
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|e|e.to_string())?;
    let rows = doc["fixtures"].as_array().ok_or("fixture slice lacks rows")?;
    let mut output = HashMap::new();
    for row in rows {
        let int = |name: &str| row[name].as_i64().and_then(|value|i32::try_from(value).ok())
            .ok_or_else(||format!("fixture {:?} is missing {name}",row["id"]));
        let id = int("id")?;
        let (first, second) = (int("firstPutCost")?, int("secondPutCost")?);
        if first < 0 || second < 0 { return Err(format!("fixture {id} has negative placement cost")); }
        if output.insert(id, Costs {first, second}).is_some() {return Err(format!("duplicate fixture cost {id}"));}
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget { pub limit: i32, pub used: i32, pub remaining: i32 }

fn masters(world: &World) -> Result<(&PutLimitTables, &HashMap<i32, Costs>), String> {
    let owner = world.get_resource::<PutCostMaster>().ok_or("put-cost master owner is not installed")?;
    Ok((owner.limits.as_ref().ok_or("put-limit tables still loading")?.as_ref().map_err(Clone::clone)?,
        owner.fixtures.as_ref().ok_or("fixture costs still loading")?.as_ref().map_err(Clone::clone)?))
}

fn total(draft: &[EditableFixture], costs: &HashMap<i32, Costs>) -> Result<i32, String> {
    let mut groups: HashMap<(i32,u32),HashSet<&str>> = HashMap::new();
    for row in draft { groups.entry((row.fixture_id,row.texture_id)).or_default().insert(&row.uid); }
    let mut used = 0i32;
    for ((id,_),uids) in groups {
        let cost = costs.get(&id).ok_or_else(||format!("fixture {id} has no cost master row"))?;
        let count = i32::try_from(uids.len()).map_err(|_|"fixture count overflow")?;
        let group = cost.second.checked_mul(count-1).and_then(|more|cost.first.checked_add(more))
            .ok_or("placement cost overflow")?;
        used = used.checked_add(group).ok_or("site placement cost overflow")?;
    }
    Ok(used)
}

pub fn budget(world: &World, site_type: &str, draft: &[EditableFixture]) -> Result<Budget,String> {
    let (limits,costs) = masters(world)?;
    let category = match site_type {
        "home_site" => SiteCategory::HousingHome,
        "first_floor" | "second_floor" | "third_floor" => SiteCategory::HousingRoom,
        _ => SiteCategory::Other,
    };
    let rank = world.get_resource::<crate::userdata::ClientUserData>().and_then(|user|user.gamedata.mysekai_rank)
        .unwrap_or_else(|| { warn!("[put-cost] user rank unavailable; MysekaiUtility.GetMysekaiRank uses rank 1"); 1 });
    let limit = i32::try_from(limits.cost(i64::from(rank),category)).map_err(|_|"put limit overflows i32")?;
    let mut used = total(draft,costs)?;
    let site_id = world.get_resource::<crate::site::Sites>().and_then(|sites|sites.site_id(site_type))
        .ok_or("edited site is absent from the site master")?;
    let appearances = world.get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
        .ok_or("layout owner is not installed")?.surface_fixture_ids(site_id)?;
    for id in appearances {
        let cost = costs.get(&id).ok_or_else(||format!("surface fixture {id} has no cost master row"))?;
        used = used.checked_add(cost.first).ok_or("surface put cost overflow")?;
    }
    let remaining = limit.checked_sub(used).ok_or("remaining put cost overflow")?;
    Ok(Budget{limit,used,remaining})
}

fn count_with_cost(remaining:i32, first:i32, second:i32, already:bool) -> i32 {
    if remaining < 1 || first < 1 { return 0; }
    if already { if second < 1 || second > remaining {0} else {remaining/second} }
    else if first > remaining {0}
    else if second < 1 || remaining-first < second {1}
    else {(remaining-first)/second+1}
}

pub fn max_placeable(world: &World, site_type: &str, draft: &[EditableFixture],
    fixture_id:i32, texture_id:u32, owned_available:i32) -> Result<i32,String> {
    let budget = budget(world,site_type,draft)?;
    let (_,costs)=masters(world)?;
    let cost=costs.get(&fixture_id).ok_or_else(||format!("fixture {fixture_id} has no cost master row"))?;
    let already=draft.iter().any(|row|row.fixture_id==fixture_id && row.texture_id==texture_id);
    Ok(count_with_cost(budget.remaining,cost.first,cost.second,already).min(owned_available))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_cost_limit_counts_first_and_repeat_placements_separately() {
        assert_eq!(count_with_cost(21,10,3,false),4);
        assert_eq!(count_with_cost(21,10,3,true),7);
        assert_eq!(count_with_cost(9,10,3,false),0);
        assert_eq!(count_with_cost(10,10,0,false),1);
        assert_eq!(count_with_cost(10,10,0,true),0);
        assert_eq!(count_with_cost(10,0,1,true),0);
        assert_eq!(count_with_cost(0,10,0,false),0);
    }
    #[test]
    fn missing_cost_columns_do_not_become_free() {
        assert!(parse_costs(r#"{"fixtures":[{"id":1}]}"#).is_err());
        assert!(parse_costs(r#"{"fixtures":[{"id":1,"firstPutCost":0,"secondPutCost":0}]}"#).is_ok());
    }
}
