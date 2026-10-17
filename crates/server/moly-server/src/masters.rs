//! The master tables the model reads, loaded once for every user.
//!
//! Each table is declared as a [`moly_assets::json::master::MasterTable`] and
//! requested from that layer, which reads it from the region's master hosts
//! and names every table it cannot give. A table the join waits for is seated
//! before any user joins; the two large ones (the reward boxes and the cards)
//! load in the background and refuse by name until they arrive.

use std::collections::BTreeMap;

use bevy::asset::io::{AssetReaderError, Reader};
use bevy::asset::{AssetPath, AssetServer};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, IoTaskPool, Task};
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_assets::remote::RemoteRegion;
use moly_masters::clock::RefreshPeriod;
use moly_masters::StaminaMax;
use serde_json::Value;

use crate::model::delivery;
use crate::model::inventory;
use crate::model::music;
use crate::model::tools;
use crate::model::avatar;

const PHENOMENA_INDEX: &str = "moly://phenomena/index.json";

/// The master data the model reads. Each is `None` while loading or when its
/// table is missing ([`Masters::missing`] names those).
#[derive(Resource, Default)]
pub struct Masters {
    pub periods: Option<Vec<RefreshPeriod>>,
    /// Phenomenon id and master name.
    pub phenomena: Option<Vec<(i32, String)>>,
    pub stamina_max: Option<StaminaMax>,
    pub boost_grant: Option<i32>,
    /// (rank, cumulative totalExp), master order.
    pub ranks: Option<Vec<(i32, i32)>>,
    pub(crate) site_categories: Option<Vec<(i32, String)>>,
    /// `mysekaiSiteLevels`: (id, mysekaiSiteId, level), master order.
    pub(crate) site_levels: Option<Vec<(i32, i32, i32)>>,
    /// `mysekaiRankReleases`: (mysekaiRank, mysekaiRankRelaseType, externalId).
    pub(crate) rank_releases: Option<Vec<(i32, String, i32)>>,
    /// Gate ids and gate skin ids.
    pub(crate) gates: Option<(Vec<i32>, Vec<i32>)>,
    /// `mysekaiGates` ids, until the skin ids join them in `gates`.
    pub(crate) gate_ids: Option<Vec<i32>>,
    /// `mysekaiGateSkins` ids, until they join the gate ids in `gates`.
    pub(crate) gate_skin_ids: Option<Vec<i32>>,
    /// The birthday-party delivery tables.
    pub(crate) delivery: Option<delivery::DeliveryTables>,
    /// The delivery tables as they resolve, until all three are in.
    pub(crate) delivery_parts: delivery::DeliveryParts,
    /// The total reward boxes (deferred): what each granted box holds.
    pub(crate) reward_boxes: Deferred<Vec<(i64, delivery::Reward)>>,
    /// The master configs (`configs`): configKey -> value.
    pub(crate) configs: Option<BTreeMap<String, String>>,
    /// The music record ids (`mysekaiMusicRecords`).
    pub(crate) music_records: Option<Vec<i32>>,
    pub(crate) avatar: moly_masters::avatar::AvatarMasters,
    pub(crate) craft: moly_masters::craft::CraftMasters,
    pub(crate) possession: moly_masters::inventory::PossessionMasters,
    /// The cards master (deferred): card id -> its character and rarity.
    pub(crate) cards: Deferred<BTreeMap<i32, moly_masters::craft::CardRow>>,
    /// The tool master (`mysekaiTools`).
    pub(crate) tools: Option<Vec<tools::ToolMaster>>,
    /// The tables the master layer named missing.
    pub missing: Vec<String>,
    /// The tables still loading, by their names.
    pub(crate) pending: Vec<&'static str>,
    /// Every table the join waits for is in.
    pub ready: bool,
}

impl std::fmt::Debug for Masters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Masters")
            .field("ready", &self.ready)
            .field("pending", &self.pending)
            .field("missing", &self.missing)
            .finish()
    }
}

/// A master table the model loads in the background once the join's tables
/// are in, and why it is not there yet.
#[derive(Debug)]
pub(crate) enum Deferred<T> {
    /// Not requested yet: the join's tables are still loading.
    Waiting,
    /// Requested (the upstream table name); not arrived yet.
    Loading(&'static str),
    /// The master layer could not give it (named once there).
    Failed(String),
    Ready(T),
}

impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self::Waiting
    }
}

impl<T> Deferred<T> {
    /// The table, or the refusal: `refused` says what cannot be done, and
    /// the reason says why the table is not there.
    pub(crate) fn get(&self, refused: &str) -> Result<&T, String> {
        match self {
            Self::Waiting => Err(format!(
                "{refused}: its master table is requested once the join's tables are in"
            )),
            Self::Loading(table) => Err(format!(
                "{refused}: the {table} master table is still loading"
            )),
            Self::Failed(reason) => Err(format!("{refused}: {reason}")),
            Self::Ready(value) => Ok(value),
        }
    }
}

impl Masters {
    /// A small synthetic table set a harness seats directly: two refresh
    /// windows, the pool maxima, one recovery grant and three ranks. It
    /// carries no game data; the server's own tests extend it with the gate
    /// and craft tables they need.
    pub fn fixture() -> Self {
        let mut masters = Self::default();
        masters.periods = Some(vec![
            RefreshPeriod {
                id: 1,
                start_hour: 5,
                end_hour: 17,
            },
            RefreshPeriod {
                id: 2,
                start_hour: 17,
                end_hour: 29,
            },
        ]);
        masters.stamina_max = Some(StaminaMax {
            normal: 1000,
            enhance: 1000,
            boost: 10000,
        });
        masters.boost_grant = Some(100);
        masters.ranks = Some(vec![(1, 0), (2, 48_000), (3, 172_300)]);
        masters.phenomena = Some(vec![(1, "a".into()), (2, "b".into())]);
        masters.ready = true;
        masters
    }

    /// Joins the tables that resolve apart: the gate and gate skin ids, and
    /// the three delivery tables.
    fn join_parts(&mut self) {
        if self.gates.is_none() {
            if let (Some(gates), Some(skins)) = (&self.gate_ids, &self.gate_skin_ids) {
                self.gates = Some((gates.clone(), skins.clone()));
            }
        }
        if self.delivery.is_none() {
            self.delivery = self.delivery_parts.complete();
        }
    }

    /// `MysekaiRankModel`'s rank of a total experience.
    pub(crate) fn rank_of(&self, total_exp: i32) -> Option<i32> {
        self.ranks
            .as_ref()?
            .iter()
            .filter(|(_, exp)| *exp <= total_exp)
            .map(|(rank, _)| *rank)
            .max()
    }

    pub(crate) fn exp_of(&self, rank: i32) -> Option<i32> {
        self.ranks
            .as_ref()?
            .iter()
            .find(|(row, _)| *row == rank)
            .map(|(_, exp)| *exp)
    }

    /// The tables a call still waits for, for the refusal's reason.
    pub(crate) fn lacking(&self) -> String {
        let names: Vec<String> = if self.pending.is_empty() {
            self.missing.clone()
        } else {
            self.pending.iter().map(|name| (*name).to_owned()).collect()
        };
        if names.is_empty() {
            "the master tables are still loading".to_owned()
        } else {
            format!("the master tables are still loading: {}", names.join(", "))
        }
    }
}

/// One master table the model reads. The master layer parses it into a fresh
/// [`Masters`] (the parser fills one field) and `seat` moves that field into
/// the model's.
struct ServerTable {
    table: MasterTable<Masters>,
    seat: fn(&mut Masters, Masters),
    /// The region whose master carries the table; `None`: every region.
    region: Option<RemoteRegion>,
    /// A deferred table's field markers; `None`: the join waits for it.
    deferred: Option<DeferredSeat>,
}

/// How a deferred table's field in the model says where it is.
struct DeferredSeat {
    /// Requested: the field reads as loading.
    requested: fn(&mut Masters, &'static str),
    /// The master layer could not give it: the field carries the reason.
    failed: fn(&mut Masters, String),
}

/// A [`ServerTable`] whose parser fills the given field of [`Masters`].
macro_rules! server_table {
    ($table:literal, $name:literal, $parse:path, $($field:ident).+) => {
        ServerTable {
            table: server_table!(@table $table, $name, $parse),
            seat: |masters, partial| masters.$($field).+ = partial.$($field).+,
            region: None,
            deferred: None,
        }
    };
    (@table $table:literal, $name:literal, $parse:path) => {
        MasterTable {
            table: $table,
            name: $name,
            parse: |text| {
                let mut partial = Masters::default();
                $parse(text, &mut partial)?;
                Ok(partial)
            },
        }
    };
}

/// A deferred [`ServerTable`]: its parser fills the given [`Deferred`] field.
macro_rules! deferred_table {
    ($table:literal, $name:literal, $parse:path, $field:ident, $region:expr) => {
        ServerTable {
            table: server_table!(@table $table, $name, $parse),
            seat: |masters, partial| masters.$field = partial.$field,
            region: $region,
            deferred: Some(DeferredSeat {
                requested: |masters, table| masters.$field = Deferred::Loading(table),
                failed: |masters, reason| masters.$field = Deferred::Failed(reason),
            }),
        }
    };
}

/// The model's master tables. The join waits for all but the deferred ones,
/// which are requested once the others are in and seated when they arrive.
static SERVER_TABLES: &[ServerTable] = &[
    server_table!(
        "mysekaiStaminas",
        "mysekaiStaminas (the server's pool maxima)",
        parse_staminas,
        stamina_max
    ),
    server_table!(
        "mysekaiStaminaRecovery",
        "mysekaiStaminaRecovery (the server's boost grant)",
        parse_recovery,
        boost_grant
    ),
    server_table!(
        "mysekaiRanks",
        "mysekaiRanks (the server's rank table)",
        parse_ranks,
        ranks
    ),
    server_table!(
        "mysekaiSites",
        "mysekaiSites (the server's housing site categories)",
        parse_site_categories,
        site_categories
    ),
    server_table!(
        "mysekaiSiteLevels",
        "mysekaiSiteLevels (the server's authored site levels)",
        parse_site_levels,
        site_levels
    ),
    server_table!(
        "mysekaiRankReleases",
        "mysekaiRankReleases (the server's released site levels)",
        parse_rank_releases,
        rank_releases
    ),
    server_table!(
        "mysekaiGates",
        "mysekaiGates (the server's gate ids)",
        parse_gates,
        gate_ids
    ),
    server_table!(
        "mysekaiGateSkins",
        "mysekaiGateSkins (the server's gate skin ids)",
        parse_gate_skins,
        gate_skin_ids
    ),
    server_table!(
        "configs",
        "configs (master configs; without it the delivery reads policies.masterConfigsStandIn)",
        parse_configs,
        configs
    ),
    server_table!(
        "mysekaiMusicRecords",
        "mysekaiMusicRecords (the server's music records)",
        music::parse_records,
        music_records
    ),
    server_table!(
        "avatarCostumes",
        "avatarCostumes (the server's avatar costumes)",
        avatar::parse_costumes,
        avatar.costumes
    ),
    server_table!(
        "avatarAccessories",
        "avatarAccessories (the server's avatar accessories)",
        avatar::parse_accessories,
        avatar.accessories
    ),
    server_table!(
        "avatarSkinColors",
        "avatarSkinColors (the server's avatar skin colours)",
        avatar::parse_skin_colors,
        avatar.skin_colors
    ),
    server_table!(
        "avatarCoordinates",
        "avatarCoordinates (the server's avatar coordinates)",
        avatar::parse_coordinates,
        avatar.coordinates
    ),
    server_table!(
        "mysekaiBlueprints",
        "mysekaiBlueprints (the server's blueprints)",
        crate::model::craft::parse_blueprints,
        craft.blueprints
    ),
    server_table!(
        "mysekaiBlueprintMysekaiMaterialCosts",
        "mysekaiBlueprintMysekaiMaterialCosts (the server's blueprint material costs)",
        crate::model::craft::parse_costs,
        craft.costs
    ),
    server_table!(
        "mysekaiBlueprintTerms",
        "mysekaiBlueprintTerms (the server's blueprint craft terms)",
        crate::model::craft::parse_terms,
        craft.terms
    ),
    server_table!(
        "mysekaiRankObtainedExps",
        "mysekaiRankObtainedExps (the server's first-craft bonus)",
        crate::model::craft::parse_rank_obtained_exps,
        craft.first_craft_bonus
    ),
    server_table!(
        "mysekaiFixturePossessions",
        "mysekaiFixturePossessions (the server's fixture possession limits)",
        inventory::parse_fixture_possessions,
        possession.fixture
    ),
    server_table!(
        "mysekaiMaterialPossessions",
        "mysekaiMaterialPossessions (the server's material possession limits)",
        inventory::parse_material_possessions,
        possession.material
    ),
    server_table!(
        "mysekaiMaterials",
        "mysekaiMaterials (the server's material types)",
        inventory::parse_materials,
        craft.material_types
    ),
    server_table!(
        "mysekaiItems",
        "mysekaiItems (the server's white blueprint item)",
        inventory::parse_items_master,
        craft.white_blueprint_item
    ),
    server_table!(
        "mysekaiTools",
        "mysekaiTools (the server's tools: initial tools, durability resets, tool craft)",
        tools::parse_master,
        tools
    ),
    server_table!(
        "birthdayPartyDeliveryRewards",
        "birthdayPartyDeliveryRewards (the delivery's reward rows)",
        delivery::parse_rewards,
        delivery_parts.rewards
    ),
    server_table!(
        "birthdayPartyDeliveryPointBonuses",
        "birthdayPartyDeliveryPointBonuses (the delivery's point bonus rows)",
        delivery::parse_point_bonuses,
        delivery_parts.bonuses
    ),
    server_table!(
        "birthdayPartyDeliveryTotalRewards",
        "birthdayPartyDeliveryTotalRewards (the delivery's total reward rows)",
        delivery::parse_total_rewards,
        delivery_parts.totals
    ),
    // The total reward boxes: the CN master carries their rows as a table of
    // their own; the JP master nests them in each box's `details`.
    deferred_table!(
        "resourceBoxDetails",
        "resourceBoxDetails (the delivery's total reward boxes)",
        delivery::parse_box_details,
        reward_boxes,
        Some(RemoteRegion::Cn)
    ),
    deferred_table!(
        "resourceBoxes",
        "resourceBoxes (the delivery's total reward boxes)",
        delivery::parse_nested_boxes,
        reward_boxes,
        Some(RemoteRegion::Jp)
    ),
    deferred_table!(
        "cards",
        "cards (the canvas craft's card character and rarity)",
        crate::model::craft::parse_cards,
        cards,
        None
    ),
];

/// The model's master tables not seated yet, and whether the deferred ones
/// have been requested.
#[derive(Resource)]
struct MasterSeats {
    pending: Vec<&'static ServerTable>,
    deferred_requested: bool,
}

type Parser = fn(&str, &mut Masters) -> Result<(), String>;

/// A root document's read, while it runs: `Ok(None)` when the asset source
/// does not hold the file.
type RootRead = Task<Result<Option<String>, String>>;

/// The root documents still reading: name, parser and the read.
#[derive(Resource)]
struct RootReads(Vec<(&'static str, Parser, RootRead)>);

/// A root document's text through the asset source. Reading it here rather
/// than through an asset load lets a document the runtime root lacks be only
/// the model's named missing entry, with no load error for the absent file.
async fn read_root_document(
    server: AssetServer,
    path: AssetPath<'static>,
) -> Result<Option<String>, String> {
    let source = server
        .get_source(path.source().clone_owned())
        .map_err(|error| error.to_string())?;
    let mut reader = match source.reader().read(path.path()).await {
        Ok(reader) => reader,
        Err(AssetReaderError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(Startup, load)
        .add_systems(PreUpdate, resolve_masters.after(master::resolve));
}

fn load(mut commands: Commands, server: Res<AssetServer>, mut masters: ResMut<MasterData>) {
    let reader = server.clone();
    commands.insert_resource(RootReads(vec![(
        "phenomena/index.json (refresh windows, phenomena)",
        parse_phenomena as Parser,
        IoTaskPool::get().spawn(async move {
            read_root_document(reader, AssetPath::from(PHENOMENA_INDEX)).await
        }),
    )]));
    // The deferred tables are requested once these are in.
    for table in SERVER_TABLES
        .iter()
        .filter(|table| table.deferred.is_none())
    {
        masters.request(&table.table);
    }
    masters.request(&PARTIES);
    commands.insert_resource(MasterSeats {
        pending: SERVER_TABLES.iter().collect(),
        deferred_requested: false,
    });
}

/// Each master as it resolves; a missing or malformed one (named once by the
/// master layer, or here for a root document) is listed and the model goes on
/// without it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_masters(
    mut commands: Commands,
    reads: Option<ResMut<RootReads>>,
    seats: Option<ResMut<MasterSeats>>,
    mut master_data: ResMut<MasterData>,
    mut masters: ResMut<Masters>,
    mut parties: ResMut<Parties>,
) {
    let mut outcomes: Vec<(&'static str, Parser, Result<Option<String>, String>)> = Vec::new();
    let mut reads_done = true;
    if let Some(mut reads) = reads {
        reads.0.retain_mut(|(name, parser, read)| {
            match block_on(future::poll_once(read)) {
                Some(outcome) => {
                    outcomes.push((*name, *parser, outcome));
                    false
                }
                None => true,
            }
        });
        if reads.0.is_empty() {
            commands.remove_resource::<RootReads>();
        } else {
            reads_done = false;
        }
    }
    let mut seated: Vec<(&'static ServerTable, _)> = Vec::new();
    let mut requested: Vec<&'static ServerTable> = Vec::new();
    let mut join_waits = false;
    let mut still_loading: Vec<&'static str> = Vec::new();
    if let Some(mut seats) = seats {
        seats.pending.retain(|table| match master_data.take(&table.table) {
            Some(result) => {
                seated.push((*table, result));
                false
            }
            None => true,
        });
        join_waits = seats.pending.iter().any(|table| table.deferred.is_none());
        for table in &seats.pending {
            if table.deferred.is_none() {
                still_loading.push(table.table.name);
            }
        }
        if !seats.deferred_requested && reads_done && !join_waits {
            // The join's tables are in: the deferred ones load in the
            // background now. Without a region (named by the master layer)
            // every candidate is requested and each fails with that reason.
            seats.deferred_requested = true;
            let region = master_data.region();
            seats.pending.retain(|table| match (table.region, region) {
                (Some(only), Some(region)) => only == region,
                _ => true,
            });
            for table in &seats.pending {
                master_data.request(&table.table);
                requested.push(*table);
            }
        }
        if seats.pending.is_empty() {
            commands.remove_resource::<MasterSeats>();
        }
    }
    // The birthday parties are their own root table and resource.
    if let Some(result) = master_data.take(&PARTIES) {
        *parties = Parties::from_result(result);
    }
    if outcomes.is_empty() && seated.is_empty() && requested.is_empty() {
        masters.pending = still_loading;
        return;
    }
    for (name, parser, outcome) in outcomes {
        let error = match outcome {
            Ok(Some(text)) => match parser(&text, &mut masters) {
                Ok(()) => continue,
                Err(error) => format!("malformed: {error}"),
            },
            Ok(None) => {
                let error = "not in the runtime root".to_owned();
                info!("[server] root document {name} is {error}");
                masters.missing.push(format!("{name}: {error}"));
                continue;
            }
            Err(error) => format!("unreadable: {error}"),
        };
        warn!("[server] root document {name} is {error}");
        masters.missing.push(format!("{name}: {error}"));
    }
    for (table, result) in seated {
        match result {
            Ok(partial) => (table.seat)(&mut masters, partial),
            Err(error) => {
                if let Some(deferred) = &table.deferred {
                    (deferred.failed)(&mut masters, error.to_string());
                }
                masters.missing.push(error.to_string());
            }
        }
        if table.deferred.is_some() {
            info!("[server] deferred master {} seated", table.table.name);
        }
    }
    masters.join_parts();
    for table in requested {
        if let Some(deferred) = &table.deferred {
            (deferred.requested)(&mut masters, table.table.table);
        }
    }
    masters.pending = still_loading;
    if !masters.ready && reads_done && !join_waits {
        info!(
            "[server] masters resolved; missing: {:?}",
            masters.missing
        );
        masters.ready = true;
        masters.pending.clear();
    }
}

/// The birthday party master (`birthdayParties`): the rows in session at the
/// server clock, with the two delivery material columns.
const PARTIES: MasterTable<Vec<PartyRow>> = MasterTable {
    table: "birthdayParties",
    name: "birthdayParties (the festival gate and the delivery)",
    parse: parse_parties,
};

/// The birthday parties the server has read.
#[derive(Resource, Default)]
pub struct Parties {
    rows: Vec<PartyRow>,
    /// Why the table is missing (named once by the master layer).
    pub(crate) missing: Option<String>,
}

/// One party in session as the delivery site reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InSessionParty {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) delivery_item_material_id: i64,
    pub(crate) delivery_reward_material_id: i64,
}

/// A `birthdayParties` row: the schedule and the two delivery columns.
#[derive(Clone, Debug)]
struct PartyRow {
    id: i64,
    start_at: i64,
    closed_at: i64,
    assetbundle_name: Option<String>,
    delivery_item_material_id: Option<i64>,
    delivery_reward_material_id: Option<i64>,
}

impl PartyRow {
    fn label(&self) -> String {
        self.assetbundle_name
            .clone()
            .unwrap_or_else(|| format!("#{}", self.id))
    }

    fn delivery_columns(&self) -> Option<(i64, i64)> {
        Some((
            self.delivery_item_material_id?,
            self.delivery_reward_material_id?,
        ))
    }
}

impl Parties {
    fn from_result(result: Result<Vec<PartyRow>, master::MasterError>) -> Self {
        match result {
            Ok(rows) => {
                let incomplete: Vec<String> = rows
                    .iter()
                    .filter(|row| row.delivery_columns().is_none())
                    .map(PartyRow::label)
                    .collect();
                if !incomplete.is_empty() {
                    warn!(
                        "[server] {} party row(s) without deliveryItemMaterialId or deliveryRewardMysekaiMaterialId ({}): the delivery does not take them in session",
                        incomplete.len(),
                        incomplete.join(", ")
                    );
                }
                Self {
                    rows,
                    missing: None,
                }
            }
            Err(error) => {
                info!("[server] no party table: no party is in session");
                Self {
                    rows: Vec::new(),
                    missing: Some(error.to_string()),
                }
            }
        }
    }

    /// Why no party can be in session, for the replies that name it.
    pub(crate) fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

    /// `GetMasterBirthdayPartiesInSession` for the delivery site: the rows in
    /// session, in master order, with the two material columns.
    pub(crate) fn in_session(&self, now_ms: i64) -> Vec<InSessionParty> {
        self.rows
            .iter()
            .filter(|row| is_within_time(now_ms, row.start_at, row.closed_at))
            .filter_map(|row| {
                let (item, reward) = row.delivery_columns()?;
                Some(InSessionParty {
                    id: row.id,
                    label: row.label(),
                    delivery_item_material_id: item,
                    delivery_reward_material_id: reward,
                })
            })
            .collect()
    }
}

/// `TimeUtility.IsWithinTime(checkAt, startAt, endAt)`: `endAt == 0` (no
/// close) means `startAt != 0 && startAt <= checkAt`; otherwise
/// `startAt <= checkAt < endAt`. A row with no start is never in session.
pub(crate) fn is_within_time(check_at: i64, start_at: i64, end_at: i64) -> bool {
    if end_at == 0 {
        start_at != 0 && start_at <= check_at
    } else {
        start_at <= check_at && check_at < end_at
    }
}

/// `birthdayParties`: a row without an integer id or session field is
/// dropped and named by the master layer.
fn parse_parties(text: &str) -> Result<Vec<PartyRow>, String> {
    master::read_rows(text, |row| {
        Ok(Some(PartyRow {
            id: master::int(row, "id")?,
            start_at: master::int(row, "startAt")?,
            closed_at: master::int(row, "closedAt")?,
            assetbundle_name: row
                .get("assetbundleName")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            delivery_item_material_id: row
                .get("deliveryItemMaterialId")
                .and_then(|v| v.as_i64()),
            delivery_reward_material_id: row
                .get("deliveryRewardMysekaiMaterialId")
                .and_then(|v| v.as_i64()),
        }))
    })
}

// ---------------------------------------------------------------------------
// The parsers
// ---------------------------------------------------------------------------

/// The rows of an upstream master table, in id order.
pub(crate) fn master_rows(value: &Value, table: &str) -> Result<Vec<Value>, String> {
    let mut rows = value
        .as_array()
        .ok_or_else(|| format!("{table} is not an array of rows"))?
        .clone();
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    Ok(rows)
}

pub(crate) fn int32(value: &Value, key: &str) -> Result<i32, String> {
    value[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("{key} of {value} is not an int"))
}

/// `configs`: configKey -> value (the value is a string in the master).
fn parse_configs(text: &str, masters: &mut Masters) -> Result<(), String> {
    let configs = master::rows(text)?
        .iter()
        .map(|row| {
            let key = master::text(row, "configKey")?;
            let text = match &row["value"] {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                other => {
                    return Err(format!(
                        "configs {key} has a value {other} that is not text"
                    ))
                }
            };
            Ok((key.to_owned(), text))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    masters.configs = Some(configs);
    Ok(())
}

fn parse_phenomena(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let periods = moly_masters::clock::parse_periods(
        value["refreshTimePeriods"]
            .as_array()
            .ok_or("no refreshTimePeriods")?,
    )?;
    let mut phenomena = value["phenomena"]
        .as_object()
        .ok_or("no phenomena")?
        .values()
        .map(|row| {
            Ok((
                int32(row, "id")?,
                row["master"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    phenomena.sort_by_key(|(id, _)| *id);
    masters.periods = Some(periods);
    masters.phenomena = Some(phenomena);
    Ok(())
}

fn parse_staminas(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let rows = master_rows(&value, "mysekaiStaminas")?;
    let max = |kind: &str| -> Result<i32, String> {
        rows.iter()
            .find(|row| row["mysekaiStaminaType"].as_str() == Some(kind))
            .ok_or_else(|| format!("no {kind} stamina row"))
            .and_then(|row| int32(row, "maxStamina"))
    };
    masters.stamina_max = Some(StaminaMax {
        normal: max("normal")?,
        enhance: max("enhance")?,
        boost: max("boost")?,
    });
    Ok(())
}

/// `mysekaiStaminaRecovery`: one object.
fn parse_recovery(text: &str, masters: &mut Masters) -> Result<(), String> {
    let row = Value::Object(master::object(text)?);
    masters.boost_grant = Some(int32(&row, "recoveryBoostStamina")?);
    Ok(())
}

/// `mysekaiRanks`: (rank, cumulative totalExp), master order.
fn parse_ranks(text: &str, masters: &mut Masters) -> Result<(), String> {
    let ranks = master::rows(text)?
        .iter()
        .map(|row| Ok((int32(row, "mysekaiRank")?, int32(row, "totalExp")?)))
        .collect::<Result<Vec<_>, String>>()?;
    masters.ranks = Some(ranks);
    Ok(())
}

/// `mysekaiSites`: (id, mysekaiSiteCategory), master order. The category
/// tells a housing site (`housing_home`, `housing_room`) from a harvest map
/// and the festival garden.
fn parse_site_categories(text: &str, masters: &mut Masters) -> Result<(), String> {
    let categories = master::rows(text)?
        .iter()
        .map(|row| {
            Ok((
                int32(row, "id")?,
                master::text(row, "mysekaiSiteCategory")?.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.site_categories = Some(categories);
    Ok(())
}

/// `mysekaiSiteLevels`: (id, mysekaiSiteId, level), master order.
fn parse_site_levels(text: &str, masters: &mut Masters) -> Result<(), String> {
    let levels = master::rows(text)?
        .iter()
        .map(|row| {
            Ok((
                int32(row, "id")?,
                int32(row, "mysekaiSiteId")?,
                int32(row, "level")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.site_levels = Some(levels);
    Ok(())
}

/// `mysekaiRankReleases`: (mysekaiRank, mysekaiRankRelaseType, externalId),
/// master order.
fn parse_rank_releases(text: &str, masters: &mut Masters) -> Result<(), String> {
    let releases = master::rows(text)?
        .iter()
        .map(|row| {
            Ok((
                int32(row, "mysekaiRank")?,
                master::text(row, "mysekaiRankRelaseType")?.to_owned(),
                int32(row, "externalId")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.rank_releases = Some(releases);
    Ok(())
}

/// The ids of a master table, master order.
fn master_ids(text: &str) -> Result<Vec<i32>, String> {
    master::rows(text)?
        .iter()
        .map(|row| int32(row, "id"))
        .collect()
}

fn parse_gates(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.gate_ids = Some(master_ids(text)?);
    Ok(())
}

fn parse_gate_skins(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.gate_skin_ids = Some(master_ids(text)?);
    Ok(())
}
