//! The change-site controller (`NPCAvatarChangeSiteController`, see
//! `moly_law::objective::change_site`): the lottery the home site and every
//! floor run when the player enters them, and the loop that sends the drawn
//! NPCs every change interval.
//!
//! Host shape:
//! - The entry into a site is read from the loaded site changing (home or a
//!   floor becoming the loaded site), which in the source is the site
//!   controller's `OnEnterSite`; a harvest map or the festival garden runs no
//!   lottery, as their controllers do not call it.
//! - The player is always the room owner here, and no birthday-party context
//!   object exists (the party runs on its own site, never entered through
//!   home or a floor), so the lottery's gate always passes.
//! - The loop starts once, with the first loaded site (the source starts it
//!   in the mysekai scene's setup), and waits on the scaled NPC clock.
//! - The cap reads the master site levels and rank releases of the
//!   player-data table and the rank the user's total experience reaches in
//!   the master rank table; the talk list, the fixtures placed on the site and
//!   the NPC list are the ones the other NPC lotteries read.
//!
//! `ChangeSiteAsync`: the rows in list order; a row whose NPC stands on its
//! target is skipped; otherwise `MoveNPC` orders the NPC to its target (see
//! `npc::change_site_state`), and the monitor waits while that NPC's current
//! objective is the change-site one and it is not on the target, testing
//! first in the same frame. As the ordered NPC's current objective is still
//! the one the order cancelled, the monitor normally returns at once and every
//! order goes out in one frame. The loop tests whether every NPC of the list
//! stands on its target and waits again only after `ChangeSiteAsync` ends.

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::objective::change_site as law;

use super::residency;

/// `ClientConfig.Mysekai.CharacterChangeSiteIntervalSecond`: FloatConfigs
/// key 123 (the getter's lookup key).
pub(crate) const KEY_INTERVAL: i32 = 123;
/// `ClientConfig.Mysekai.CharacterChangeSiteRate`: FloatConfigs key 124.
pub(crate) const KEY_RATE: i32 = 124;

/// The master rows the cap reads.
#[derive(Debug, Clone)]
struct SiteLevelRow {
    id: i64,
    site_id: i64,
    level: i64,
    entry_max: i32,
}

#[derive(Default)]
struct Masters {
    /// mysekaiSiteLevels in master order.
    levels: Vec<SiteLevelRow>,
    /// (rank, release type, external id) of mysekaiRankReleases, master order.
    releases: Vec<(i64, String, i64)>,
    /// mysekaiRanks in master order.
    ranks: Vec<moly_law::ui::mysekai_rank::MasterMysekaiRank>,
}

impl Masters {
    /// `GetMasterMysekaiSiteLevel(siteId, rank)`: among the site's levels in
    /// master order whose id is released (release type site level, rank at
    /// most the player's), the first with the greatest level.
    fn site_level(&self, site_id: i64, rank: i64) -> Option<&SiteLevelRow> {
        let released: Vec<i64> = self
            .releases
            .iter()
            .filter(|(r, kind, _)| kind == "mysekai_site_level" && *r <= rank)
            .map(|(_, _, id)| *id)
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
}

/// The controller's state.
#[derive(Resource)]
pub(crate) struct ChangeSiteController {
    player_data: Option<Handle<JsonAsset>>,
    rank_table: Option<Handle<JsonAsset>>,
    masters: Option<Result<Masters, String>>,
    /// The static change list; `None` = dropped.
    list: Option<Vec<law::ChangeSiteData>>,
    /// The loop's current wait (scaled seconds), once started.
    wait: Option<moly_law::objective::DelayPromise>,
    wait_since: u32,
    started: bool,
    stopped: Option<String>,
    last_entered: Option<String>,
    random: crate::npc_objective::MemberRng,
    /// `ChangeSiteAsync` in flight.
    change: Option<ChangeAsync>,
}

/// `ChangeSiteAsync` over one list.
struct ChangeAsync {
    rows: Vec<law::ChangeSiteData>,
    next: usize,
    /// The monitor's NPC, its target and the frame of its last test.
    waiting: Option<(Entity, i32, u32)>,
}

impl Default for ChangeSiteController {
    fn default() -> Self {
        Self {
            player_data: None,
            rank_table: None,
            masters: None,
            list: None,
            wait: None,
            wait_since: 0,
            started: false,
            stopped: None,
            last_entered: None,
            random: crate::npc_objective::MemberRng::from_platform(),
            change: None,
        }
    }
}

/// Startup: request the master tables.
pub(crate) fn load(mut controller: ResMut<ChangeSiteController>, server: Res<AssetServer>) {
    controller.player_data =
        Some(server.load::<JsonAsset>("moly://fixture-models/player-data.json"));
    controller.rank_table = Some(server.load::<JsonAsset>(moly_assets::mysekai_ranks()));
}

fn parse_masters(player_data: &str, ranks: &str) -> Result<Masters, String> {
    let data: serde_json::Value =
        serde_json::from_str(player_data).map_err(|e| format!("player-data.json: {e}"))?;
    let tables = &data["tables"];
    let int = |row: &serde_json::Value, field: &str| -> Result<i64, String> {
        row[field]
            .as_i64()
            .ok_or_else(|| format!("{field} is not an int in {row}"))
    };
    let mut masters = Masters::default();
    for row in tables["mysekaiSiteLevels"]
        .as_array()
        .ok_or("no mysekaiSiteLevels")?
    {
        masters.levels.push(SiteLevelRow {
            id: int(row, "id")?,
            site_id: int(row, "mysekaiSiteId")?,
            level: int(row, "level")?,
            // The harvest sites' level rows carry no entry maximum; an absent
            // int field of a master row reads as 0.
            entry_max: i32::try_from(row["characterEntryMaxNum"].as_i64().unwrap_or(0))
                .map_err(|_| "characterEntryMaxNum out of range".to_owned())?,
        });
    }
    for row in tables["mysekaiRankReleases"]
        .as_array()
        .ok_or("no mysekaiRankReleases")?
    {
        masters.releases.push((
            int(row, "mysekaiRank")?,
            row["mysekaiRankRelaseType"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
            int(row, "externalId")?,
        ));
    }
    let ranks: serde_json::Value =
        serde_json::from_str(ranks).map_err(|e| format!("mysekai-ranks.json: {e}"))?;
    let entries = ranks["entries"]
        .as_object()
        .ok_or("mysekai-ranks.json has no entries")?;
    for id in ranks["rowOrder"]
        .as_array()
        .ok_or("mysekai-ranks.json has no rowOrder")?
    {
        let key = id.as_i64().ok_or("rowOrder id is not an int")?.to_string();
        let row = entries
            .get(&key)
            .ok_or_else(|| format!("rank row {key} missing"))?;
        let field = |name: &str| -> Result<i32, String> {
            row[name]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| format!("rank row {key}: {name} is not an int"))
        };
        masters
            .ranks
            .push(moly_law::ui::mysekai_rank::MasterMysekaiRank {
                id: field("id")?,
                mysekai_rank: field("mysekaiRank")?,
                total_exp: field("totalExp")?,
            });
    }
    Ok(masters)
}

/// Update: resolve the master tables once both documents are in.
pub(crate) fn parse(
    mut controller: ResMut<ChangeSiteController>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
) {
    if controller.masters.is_some() {
        return;
    }
    let (Some(data), Some(ranks)) = (
        controller.player_data.clone(),
        controller.rank_table.clone(),
    ) else {
        return;
    };
    for (handle, name) in [(&data, "player-data.json"), (&ranks, "mysekai-ranks.json")] {
        if let LoadState::Failed(error) = server.load_state(handle) {
            let reason = format!("{name} is not in this runtime root ({error})");
            warn!("[npc-change-site] {reason}: the cap cannot be read in this root, every floor lottery draws nobody (a root gap, not a source state)");
            controller.masters = Some(Err(reason));
            return;
        }
    }
    let (Some(data), Some(ranks)) = (jsons.get(&data), jsons.get(&ranks)) else {
        return;
    };
    let masters = parse_masters(&data.0, &ranks.0);
    match &masters {
        Ok(m) => info!(
            "[npc-change-site] masters: {} site levels, {} rank releases, {} ranks",
            m.levels.len(),
            m.releases.len(),
            m.ranks.len()
        ),
        Err(reason) => warn!("[npc-change-site] masters refused: {reason}"),
    }
    controller.masters = Some(masters);
}

/// Update: the lottery on entering home or a floor, and the loop.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn run(
    mut controller: ResMut<ChangeSiteController>,
    site: Option<Res<crate::site::SiteActive>>,
    frame: Res<bevy::diagnostic::FrameCount>,
    clock: Res<crate::npc_clock::NpcClock>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    registry: Option<Res<crate::npc::Registry>>,
    talks: Option<Res<crate::server_panel::TalkDataStore>>,
    tables: Option<Res<crate::fixture_activity_data::FixtureActivityTables>>,
    placements: Res<crate::fixture::FixturePlacements>,
    total_exp: Option<Res<crate::mysekai_rank::UserTotalExp>>,
    npcs: Query<
        (&crate::npc::CharacterUnitId, &crate::npc::NpcActions),
        Without<crate::player::PlayerControlled>,
    >,
) {
    let frame = frame.0;
    let Some(site) = site.as_deref() else {
        return;
    };
    let controller = &mut *controller;
    if !controller.started {
        // StartChangeSiteLoop: Reset (an empty list), then the first wait.
        controller.started = true;
        controller.list = Some(Vec::new());
        if let Some(configs) = configs.as_deref() {
            start_wait(controller, configs, frame);
        }
        info!(
            "[npc-change-site] StartChangeSiteLoop: Reset (empty change list), first wait started"
        );
    }
    // The NPC list, in the avatar store's order (the roster's order).
    let order: Vec<u32> = registry
        .as_deref()
        .map(|registry| registry.character_unit_ids.clone())
        .unwrap_or_default();
    let list_npcs: Vec<law::NpcOnSite> = order
        .iter()
        .filter_map(|unit| {
            npcs.iter()
                .find(|(u, _)| u.0 == *unit)
                .map(|(u, actions)| law::NpcOnSite {
                    unit: u.0 as i32,
                    site: residency::site_type_value(&actions.site_type).unwrap_or(-1),
                })
        })
        .collect();
    // OnEnterSite of home or a floor.
    if controller.last_entered.as_deref() != Some(site.site_type.as_str()) {
        controller.last_entered = Some(site.site_type.clone());
        if let Some(target) =
            residency::site_type_value(&site.site_type).filter(|t| (0..=3).contains(t))
        {
            let masters = controller.masters.as_ref();
            let rank = masters
                .and_then(|m| m.as_ref().ok())
                .zip(total_exp.as_deref())
                .map(|(m, exp)| moly_law::ui::mysekai_rank::get_mysekai_rank(&m.ranks, exp.0));
            let cap = masters
                .and_then(|m| m.as_ref().ok())
                .zip(rank)
                .and_then(|(m, rank)| m.site_level(site.site_id as i64, rank as i64))
                .map(|row| (row.id, row.level, row.entry_max));
            let fixture_ids: Vec<i32> = if placements.site_type() == site.site_type {
                placements.fixture_ids()
            } else {
                Vec::new()
            };
            let talk_rows = talk_rows(talks.as_deref(), tables.as_deref());
            // The engine draw and the order keys share the controller's generator.
            let random = std::cell::RefCell::new(&mut controller.random);
            let mut key_word = String::new();
            let outcome = law::lottery_change_site_characters(target, true, &list_npcs, &mut |t| {
                let units: Vec<i32> = list_npcs.iter().map(|npc| npc.unit).collect();
                let (key, cast) = law::lottery_characters(
                    cap.map(|(_, _, max)| max),
                    &fixture_ids,
                    &talk_rows,
                    &units,
                    &mut |n| crate::npc_objective::engine_int_draw(&mut random.borrow_mut(), n),
                    &mut || {
                        let mut random = random.borrow_mut();
                        ((random.next() as u128) << 64) | random.next() as u128
                    },
                );
                key_word = format!("site {t}: cap {cap:?} (level id, level, entry max) at rank {rank:?}, key {key:?}, cast {cast:?}");
                cast
            });
            // Cancel(): the change in flight is cancelled with its orders.
            match outcome {
                law::LotteryOutcome::Replaced(rows) => {
                    info!(
                        "[npc-change-site] LotteryChangeSiteCharacters({} = {target}) on entering it: Cancel(); {key_word}; change list {:?}",
                        site.site_type,
                        rows.iter().map(|row| (row.unit, row.target_site)).collect::<Vec<_>>()
                    );
                    controller.list = Some(rows);
                    if controller.change.take().is_some() {
                        info!("[npc-change-site] Cancel(): ChangeSiteAsync in flight stops");
                    }
                }
                law::LotteryOutcome::RefusedSiteType(t) => {
                    error!("[npc-change-site] LotteryChangeSiteCharacters: site type {t} is not home or a floor");
                }
                law::LotteryOutcome::NotRun => {}
            }
        }
    }
    // The loop.
    if controller.stopped.is_some() {
        return;
    }
    let Some(configs) = configs.as_deref() else {
        return;
    };
    // The loop awaits ChangeSiteAsync before its next wait.
    if controller.change.is_some() {
        return;
    }
    if controller.wait.is_none() {
        start_wait(controller, configs, frame);
    }
    let Some(mut wait) = controller.wait else {
        return;
    };
    if frame == controller.wait_since || !clock.ready() {
        return;
    }
    if !wait.advance(clock.delta()) {
        controller.wait = Some(wait);
        return;
    }
    controller.wait = None;
    let rows = controller.list.clone().unwrap_or_default();
    if rows.is_empty() {
        start_wait(controller, configs, frame);
        return;
    }
    let draw = crate::npc_objective::engine_int_draw(&mut controller.random, 100) as i32;
    let rate = configs.float(KEY_RATE);
    if !law::change_proceeds(rate, draw) {
        info!("[npc-change-site] loop: Range(0,100) = {draw}, rate {rate} > {draw}: no change this interval");
        start_wait(controller, configs, frame);
        return;
    }
    info!(
        "[npc-change-site] loop: Range(0,100) = {draw}, rate {rate}: ChangeSiteAsync over {} rows (unit, target) {:?}",
        rows.len(),
        rows.iter().map(|row| (row.unit, row.target_site)).collect::<Vec<_>>()
    );
    controller.change = Some(ChangeAsync {
        rows,
        next: 0,
        waiting: None,
    });
}

/// Update, after [`run`]: `ChangeSiteAsync`'s orders and monitor; at its end
/// the loop's arrival test and its next wait.
pub(crate) fn change_site_async(world: &mut World) {
    let Some(mut change) = world.resource_mut::<ChangeSiteController>().change.take() else {
        return;
    };
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let sites = npc_sites(world);
    loop {
        if let Some((actor, target, tested)) = change.waiting {
            if frame == tested {
                world.resource_mut::<ChangeSiteController>().change = Some(change);
                return;
            }
            if world.get_entity(actor).is_ok()
                && super::change_site_state::monitor_waits(world, actor, target)
            {
                change.waiting = Some((actor, target, frame));
                world.resource_mut::<ChangeSiteController>().change = Some(change);
                return;
            }
            change.waiting = None;
        }
        let Some(row) = change.rows.get(change.next).cloned() else {
            break;
        };
        change.next += 1;
        let Some(&(actor, site)) = sites.get(&row.unit) else {
            info!(
                "[npc-change-site] ChangeSiteAsync: unit {} has no NPC, row skipped",
                row.unit
            );
            continue;
        };
        if site == row.target_site {
            continue;
        }
        super::change_site_state::move_npc(world, actor, row.unit as u32, row.target_site, frame);
        // Monitor: its first test runs now.
        if super::change_site_state::monitor_waits(world, actor, row.target_site) {
            change.waiting = Some((actor, row.target_site, frame));
            world.resource_mut::<ChangeSiteController>().change = Some(change);
            return;
        }
    }
    info!(
        "[npc-change-site] ChangeSiteAsync over {} rows ends at frame {frame}",
        change.rows.len()
    );
    let sites = npc_sites(world);
    let site_of = |unit: i32| sites.get(&unit).map_or(-1, |(_, site)| *site);
    let arrived = law::all_arrived(&change.rows, &site_of);
    world.resource_scope(|world, mut controller: Mut<ChangeSiteController>| {
        if arrived {
            controller.list = None;
            info!("[npc-change-site] loop: every NPC of the list stands on its target: change list dropped");
        }
        if let Some(configs) = world.get_resource::<crate::client_config::ClientConfigs>() {
            start_wait(&mut controller, configs, frame);
        }
    });
}

/// Every NPC's entity and site type value, by unit.
fn npc_sites(world: &mut World) -> std::collections::HashMap<i32, (Entity, i32)> {
    let mut query = world.query_filtered::<(
        Entity,
        &crate::npc::CharacterUnitId,
        &crate::npc::NpcActions,
    ), Without<crate::player::PlayerControlled>>();
    query
        .iter(world)
        .map(|(entity, unit, actions)| {
            (
                unit.0 as i32,
                (
                    entity,
                    residency::site_type_value(&actions.site_type).unwrap_or(-1),
                ),
            )
        })
        .collect()
}

fn start_wait(
    controller: &mut ChangeSiteController,
    configs: &crate::client_config::ClientConfigs,
    frame: u32,
) {
    let interval = configs.float(KEY_INTERVAL);
    let ms = law::interval_milliseconds(interval);
    match moly_law::objective::DelayPromise::from_milliseconds(ms) {
        Ok(delay) => {
            controller.wait = Some(delay);
            controller.wait_since = frame;
        }
        Err(ms) => {
            // UniTask.Delay throws on a negative span: the loop ends.
            let reason =
                format!("interval {interval} gives a {ms} ms delay, which the delay refuses");
            error!("[npc-change-site] loop ended: {reason}");
            controller.stopped = Some(reason);
        }
    }
}

/// The talk list as the floor lottery reads it: each row's first fixture
/// condition and its unit group's cast.
fn talk_rows(
    talks: Option<&crate::server_panel::TalkDataStore>,
    tables: Option<&crate::fixture_activity_data::FixtureActivityTables>,
) -> Vec<law::TalkForLottery> {
    let (Some(talks), Some(tables)) = (talks, tables) else {
        return Vec::new();
    };
    talks
        .talk_list()
        .iter()
        .filter_map(|row| {
            let master = tables.talk_master(row.talk_id)?;
            let fixture_condition =
                tables
                    .talk_conditions(row.talk_id)
                    .ok()
                    .and_then(|conditions| {
                        conditions
                            .into_iter()
                            .find(|(kind, _)| {
                                moly_law::talk::row::condition_type_discriminant(kind)
                                    == Some(moly_law::talk::row::CONDITION_MYSEKAI_FIXTURE_ID)
                            })
                            .map(|(_, value)| value)
                    });
            let cast: Vec<i32> = tables
                .unit_ids_of_group(master.unit_group_id)
                .ok()?
                .into_iter()
                .map(|unit| unit as i32)
                .collect();
            Some(law::TalkForLottery {
                talk_id: row.talk_id,
                is_read: row.is_read,
                fixture_condition,
                character_count: cast.len() as i32,
                cast,
            })
        })
        .collect()
}
