//! `HarvestServerMock`: the one panel for the server-decided harvest data.
//!
//! The client receives the harvest map, the tools, the stamina and the
//! materials as user data and posts its harvest and gather logs to two APIs.
//! None of the server's rules are in the client, so this panel stands in for
//! them. Every value it invents is labelled as the mock's own choice; the
//! client path after it (position snap, draws, damage, local debit, drop fall,
//! pickup) is client code and is not mocked.
//!
//! Entries, as the server replies are shaped:
//! - `HarvestMapMock`: the user harvest map of each harvest site (fixture rows
//!   and their drop rows), generated from the built-in map rules below for
//!   each refresh window, and generated anew when a new window begins.
//! - The stamina mock: the stamina triple (`StaminaMock`: the normal pool at
//!   its master maximum, enhance 0, and the boost pool of one boost
//!   recovery, the recovery master's `recoveryBoostStamina`; boost is spent
//!   first, so the first swings of a session run boosted, at the boost speed
//!   and with the boost effects).
//! - `HarvestApiMock` and `GatherApiMock`: echo replies to the harvest and
//!   gather requests. The user tool rows are not this panel's: they are the
//!   server document's `userMysekaiTools`, which the craft replies also
//!   change; the harvest reply's tools come from the server model's tool use
//!   policy (`crate::server::tools`).
//! - `TreasureBoxListMock`: the user treasure box rows. The mock's choice:
//!   one `once_a_day` box (seq 1, the first harvest site), status
//!   before_spawned, `transportSeconds` = [`TRANSPORT_SECONDS`] (the server
//!   value is not on disk). The client sets up an airplane for every
//!   unspawned box on whichever harvest site loads, and the spawn request
//!   carries that site.
//! - `TreasureBoxSpawnMock`: the reply to the spawn request. The mock's
//!   placement rule: the box (by seq) becomes spawned on the sent site at the
//!   sent player position (the server's rule is not on disk); the site's map
//!   gains a `treasure_box_transport`
//!   fixture row there (hp 0, spawned) with one drop row of a type-3
//!   material listing the site.
//! - `ReleaseApiMock`: the user phenomena rows. The mock's choices: the list
//!   is empty at login (a user who has learned no phenomenon, so the first
//!   harvest arrival learns today's); the reply to the release request
//!   appends today's phenomenon once (`obtainedAt` 0: no client reader in
//!   this flow). The client's today stands in for the server's (the weather
//!   chain's current phenomenon id on the harvest site).
//!
//! The map rules (`map_rules.json`) are statistics of the server's harvest
//! map replies, aggregated per site; they hold no user data. The server
//! decides the map, so the rules reproduce its frequencies, not its code:
//! - Spawn points. Every fixture id has its own fixed set of site-local
//!   points (a stone stands only on stone points, each flower only on its
//!   own points); ids never share a point across types. A group is one such
//!   set (two ids that stand on the same points form one group).
//! - Counts. A group places one weighted mix of (fixture id, count): most
//!   counts are fixed (fourteen trees, six toolboxes), some are "none or n"
//!   (two rare leaf piles, one rare tree). The mix's fixtures take distinct
//!   points: the group's fixed points (filled in every map) first, then
//!   points drawn at random from the rest. Which points stand is random;
//!   where they can stand is not.
//! - Drops. A struck fixture (hp above 0) has a row at each step hp with
//!   the step resource (wood or stone); every fixture then has a weighted
//!   number of last-attack rows: the first from its first-row table (the
//!   site's own wood, stone or flower material), the rest from its
//!   other-row table. The fixed treasure box's row is a music record of the
//!   rule's track type, every master record of that type at equal weight.
//! - Refresh. A map belongs to one master refresh window (05:00 and 17:00
//!   device local time); a new window replaces every harvest map, harvested
//!   rows and unpicked drops included. The maps are not replaced while a
//!   harvest site is loaded; the next arrival after the window change sees
//!   the new ones.
//!
//! Two other flower-garden plant mixes exist (every flower doubled; a
//! hydrangea mix with the rare white flower). What selects them is not in
//! the rules, so the generator places the common mix. The transported box
//! (paper airplane) and birthday plants (event calendar) are left out of
//! the map and named.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::Deserialize;

use super::law::Stamina;
use super::{RT_MATERIAL, RT_MYSEKAI_FIXTURE, RT_MYSEKAI_ITEM, RT_MYSEKAI_MATERIAL, RT_MYSEKAI_MUSIC_RECORD};

/// `UserMysekaiSiteHarvestFixtureStatus`.
pub(crate) const FIXTURE_SPAWNED: i32 = 1;
pub(crate) const FIXTURE_HARVESTED: i32 = 2;
/// `MysekaiSiteHarvestResourceDropStatus`.
pub(crate) const DROP_BEFORE: i32 = 0;
pub(crate) const DROP_DROPPED: i32 = 1;

/// One `UserMysekaiSiteHarvestFixture` row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserFixture {
    pub(crate) fixture_id: i32,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) hp: i32,
    pub(crate) status: i32,
    pub(crate) group_id: i32,
}

/// One `UserMysekaiSiteHarvestResourceDrop` row. It belongs to the fixture
/// at the same (x, z); `hp` is the fixture hp at which it falls.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserDrop {
    pub(crate) resource_type: i32,
    pub(crate) resource_id: i64,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) hp: i32,
    pub(crate) seq: i32,
    pub(crate) status: i32,
    pub(crate) quantity: i32,
    pub(crate) group_id: i32,
}

/// `UserMysekaiTreasureBoxStatus`.
pub(crate) const BOX_BEFORE_SPAWNED: i32 = 0;
pub(crate) const BOX_SPAWNED: i32 = 1;
/// `TreasureBoxListMock`'s `transportSeconds` (the mock's constant).
pub(crate) const TRANSPORT_SECONDS: i32 = 10;

/// One `UserMysekaiTreasureBox` row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserTreasureBox {
    pub(crate) refresh_type: &'static str,
    pub(crate) seq: i32,
    pub(crate) transport_seconds: i32,
    pub(crate) status: i32,
    pub(crate) site_id: u32,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
}

/// `UserMysekaiHarvestMap`.
#[derive(Clone, Debug, Default)]
pub(crate) struct UserHarvestMap {
    pub(crate) site_id: u32,
    pub(crate) fixtures: Vec<UserFixture>,
    pub(crate) drops: Vec<UserDrop>,
}

/// One `UserMysekaiTool` row as the harvest reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserTool {
    pub(crate) tool_id: i64,
    pub(crate) quantity: i32,
    pub(crate) durability: i32,
}

/// The server's `userMysekaiTools` rows as the harvest reads them.
pub(crate) fn user_tools(rows: &[crate::server::tools::UserMysekaiTool]) -> Vec<UserTool> {
    rows.iter()
        .map(|row| UserTool {
            tool_id: i64::from(row.mysekai_tool_id),
            quantity: row.quantity,
            durability: row.durability,
        })
        .collect()
}

/// The master rows the mock draws from (joined by the harvest catalog).
#[derive(Clone, Debug)]
pub(crate) struct MockFixtureRow {
    pub(crate) id: i32,
    /// Master fixture type value (wood 0 ... birthday_plant 9).
    pub(crate) kind: i32,
    pub(crate) hp: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct MockMaterialRow {
    pub(crate) id: i64,
    /// `MysekaiMaterialType` value (wood 0, mineral 1, plant 2, junk 3, ...).
    pub(crate) material_type: i32,
    pub(crate) site_ids: Vec<u32>,
}

/// What the panel is built from.
#[derive(Clone)]
pub(crate) struct MockInputs {
    pub(crate) fixtures: Vec<MockFixtureRow>,
    pub(crate) materials: Vec<MockMaterialRow>,
    /// Master `maxStamina` of the normal pool.
    pub(crate) max_normal_stamina: i32,
    /// `recoveryBoostStamina` of one boost recovery.
    pub(crate) boost_grant: i32,
    /// The harvest sites (master category harvest).
    pub(crate) harvest_sites: Vec<u32>,
    /// Master music record ids by `mysekaiMusicTrackType`.
    pub(crate) music_records: BTreeMap<String, Vec<i64>>,
}

/// Kinds left out of the mock map, and who places them.
pub(crate) const MOCK_EXCLUDED: [(&str, &str); 2] = [
    (
        "treasure_box_transport",
        "delivered by the paper airplane (TreasureBoxSpawnMock)",
    ),
    ("birthday_plant", "birthday party calendar (event gated)"),
];

/// The built-in map rules (see the module comment).
const MAP_RULES: &str = include_str!("map_rules.json");

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MapRules {
    schema_version: u32,
    sites: Vec<SiteRules>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SiteRules {
    site_id: u32,
    groups: Vec<GroupRules>,
    drops: Vec<DropRules>,
}

/// One set of spawn points and what a map places on it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GroupRules {
    /// The master fixture type word of the group's fixtures.
    kind: String,
    /// Site-local (x, z).
    points: Vec<[i32; 2]>,
    /// The points every map fills.
    fixed_points: Vec<[i32; 2]>,
    mixes: Vec<Mix>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mix {
    weight: u32,
    /// (fixture id, count).
    fixtures: Vec<(i32, usize)>,
}

/// (resource type word, resource id, quantity).
type DropResource = (String, i64, i32);
/// (resource type word, resource id, quantity, weight).
type WeightedResource = (String, i64, i32, u32);

/// The drop rows of one fixture id on one site.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DropRules {
    fixture_id: i32,
    /// The step rows' hps, in seq order.
    step_hps: Vec<i32>,
    step_resource: Option<DropResource>,
    /// (last-attack rows, weight).
    row_counts: Vec<(usize, u32)>,
    first_row: Vec<WeightedResource>,
    /// The first last-attack row is a music record of this track type.
    #[serde(default)]
    record_track_type: Option<String>,
    other_rows: Vec<WeightedResource>,
}

impl MapRules {
    /// The built-in rules. They are checked once here; a malformed table is
    /// a build defect, not an input.
    pub(crate) fn builtin() -> Self {
        let rules: Self = serde_json::from_str(MAP_RULES)
            .unwrap_or_else(|error| panic!("map_rules.json: {error}"));
        assert_eq!(rules.schema_version, 1, "map_rules.json: schemaVersion");
        for site in &rules.sites {
            for group in &site.groups {
                assert!(
                    group.fixed_points.iter().all(|p| group.points.contains(p)),
                    "map_rules.json site {}: a fixed point outside its group",
                    site.site_id
                );
                assert!(
                    group.mixes.iter().any(|mix| mix.weight > 0),
                    "map_rules.json site {}: a group without weight",
                    site.site_id
                );
            }
            for drops in &site.drops {
                let words = drops
                    .step_resource
                    .iter()
                    .map(|row| &row.0)
                    .chain(drops.first_row.iter().map(|row| &row.0))
                    .chain(drops.other_rows.iter().map(|row| &row.0));
                for word in words {
                    assert!(
                        resource_type_value(word).is_some(),
                        "map_rules.json: resource type {word} is outside the drop families"
                    );
                }
                assert!(
                    drops.first_row.is_empty() != drops.record_track_type.is_none(),
                    "map_rules.json fixture {}: the first row needs exactly one of a table and a record track type",
                    drops.fixture_id
                );
            }
        }
        rules
    }
}

/// The drop families' resource type words (`Sekai.Constants.ResourceType`).
fn resource_type_value(word: &str) -> Option<i32> {
    match word {
        "material" => Some(RT_MATERIAL),
        "mysekai_fixture" => Some(RT_MYSEKAI_FIXTURE),
        "mysekai_material" => Some(RT_MYSEKAI_MATERIAL),
        "mysekai_item" => Some(RT_MYSEKAI_ITEM),
        "mysekai_music_record" => Some(RT_MYSEKAI_MUSIC_RECORD),
        _ => None,
    }
}

/// The panel resource.
#[derive(Resource)]
pub(crate) struct HarvestServerMock {
    maps: BTreeMap<u32, UserHarvestMap>,
    /// What a refresh generates the maps from.
    inputs: MockInputs,
    rules: MapRules,
    /// The refresh window (its start, epoch ms) the maps belong to; `None`
    /// when no window was known at generation.
    window: Option<i64>,
    boxes: Vec<UserTreasureBox>,
    /// The `treasure_box_transport` master row (lowest id).
    transport_fixture: Option<i32>,
    /// Type-3 material rows (the transported box's drop row).
    box_materials: Vec<MockMaterialRow>,
    stamina: Stamina,
    materials: BTreeMap<i64, i64>,
    others: BTreeMap<(i32, i64), i64>,
    /// `userMysekaiPhenomena` (ReleaseApiMock).
    phenomena: Vec<super::learn::UserPhenomenon>,
}

/// A request of `PostUserMysekaiHarvestApi` (one merged run of stacks).
#[derive(Clone, Debug)]
pub(crate) struct HarvestRequest {
    pub(crate) site_id: u32,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) fixture_id: i32,
    pub(crate) hp: i32,
    pub(crate) is_last_attack: bool,
    /// (tool id, use count, the durability the client holds after them).
    pub(crate) tool: Option<(i64, i32, i32)>,
    /// `[{type, rest}]` in the order normal, enhance, boost.
    pub(crate) stamina_rests: [i32; 3],
}

/// A request of `PostUserMysekaiGatherApi`.
#[derive(Clone, Debug)]
pub(crate) struct GatherRequest {
    pub(crate) site_id: u32,
    pub(crate) drops: Vec<UserDrop>,
}

/// The parts of a `SuiteUser` reply the client merges.
#[derive(Clone, Debug)]
pub(crate) struct SuiteUserReply {
    pub(crate) stamina: Stamina,
    /// `userMysekaiTools` when the reply carries it (the harvest reply of a
    /// request that used a tool).
    pub(crate) tools: Option<Vec<UserTool>>,
    pub(crate) map: Option<UserHarvestMap>,
    pub(crate) materials: BTreeMap<i64, i64>,
}

struct MockRng(u64);

impl MockRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

impl HarvestServerMock {
    /// Build the panel: every harvest site's map for `window` by the map
    /// rules and the normal stamina at its master maximum.
    pub(crate) fn new(inputs: MockInputs, window: Option<i64>) -> Self {
        let rules = MapRules::builtin();
        if window.is_none() {
            info!("[harvest-mock] HarvestMapMock: no refresh window is known yet; the maps hold until one is");
        }
        let maps = generate_maps(&inputs, &rules, window.unwrap_or(0));
        // TreasureBoxListMock (the mock's choice).
        let boxes = inputs
            .harvest_sites
            .first()
            .map(|&site_id| UserTreasureBox {
                refresh_type: "once_a_day",
                seq: 1,
                transport_seconds: TRANSPORT_SECONDS,
                status: BOX_BEFORE_SPAWNED,
                site_id,
                position_x: 0,
                position_z: 0,
            })
            .into_iter()
            .collect();
        Self {
            maps,
            boxes,
            transport_fixture: inputs
                .fixtures
                .iter()
                .filter(|row| row.kind == 3)
                .map(|row| row.id)
                .min(),
            box_materials: inputs
                .materials
                .iter()
                .filter(|row| row.material_type == 3)
                .cloned()
                .collect(),
            // StaminaMock (the mock's choice: the user has used one boost
            // recovery; the user's pools are server data).
            stamina: Stamina {
                normal: inputs.max_normal_stamina,
                enhance: 0,
                boost: inputs.boost_grant,
            },
            materials: BTreeMap::new(),
            others: BTreeMap::new(),
            phenomena: Vec::new(),
            inputs,
            rules,
            window,
        }
    }

    /// The login fetch: the user lists as the client first receives them
    /// (the tools come from the server document).
    pub(crate) fn login(&self) -> (BTreeMap<u32, UserHarvestMap>, Stamina) {
        (self.maps.clone(), self.stamina)
    }

    /// The refresh: the maps of a new refresh window replace every harvest
    /// site's map. `None` when the maps already belong to `window`.
    pub(crate) fn refresh(&mut self, window: i64) -> Option<BTreeMap<u32, UserHarvestMap>> {
        if self.window == Some(window) {
            return None;
        }
        let before = self.window.replace(window);
        self.maps = generate_maps(&self.inputs, &self.rules, window);
        info!(
            "[harvest-mock] HarvestMapMock refresh: window {before:?} -> {window}; {}",
            self.maps
                .iter()
                .map(|(site, map)| format!("site {site} {} fixtures {} drop rows", map.fixtures.len(), map.drops.len()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        Some(self.maps.clone())
    }

    /// The user phenomena rows at login.
    pub(crate) fn phenomena(&self) -> Vec<super::learn::UserPhenomenon> {
        self.phenomena.clone()
    }

    /// `ReleaseApiMock` (`PostUserMysekaiReleaseApi(userId, mysekaiSiteId)`):
    /// today's phenomenon is recorded once; the reply carries the whole list.
    /// The site does not enter the row (the rows have no site field).
    pub(crate) fn release(
        &mut self,
        _site_id: u32,
        today: i32,
    ) -> Vec<super::learn::UserPhenomenon> {
        if !self.phenomena.iter().any(|row| row.phenomena_id == today) {
            self.phenomena.push(super::learn::UserPhenomenon {
                phenomena_id: today,
                obtained_at: 0,
            });
        }
        self.phenomena.clone()
    }

    /// `TreasureBoxListMock`: the user treasure box rows at login.
    pub(crate) fn treasure_boxes(&self) -> Vec<UserTreasureBox> {
        self.boxes.clone()
    }

    /// `TreasureBoxSpawnMock` (`PatchUserMysekaiSpawnApi`): the box row of
    /// (site, seq) becomes spawned at the sent position; the site's map gains
    /// the transported box's fixture row and one drop row there. `None` when
    /// the mock cannot serve it (the client then ends the airplane, as on a
    /// failed reply).
    pub(crate) fn spawn_treasure_box(
        &mut self,
        site_id: u32,
        seq: i32,
        position_x: i32,
        position_z: i32,
    ) -> Option<(Vec<UserTreasureBox>, UserHarvestMap)> {
        let fixture_id = self.transport_fixture?;
        let material = self
            .box_materials
            .iter()
            .filter(|row| row.site_ids.contains(&site_id))
            .map(|row| row.id)
            .min()?;
        let row = self
            .boxes
            .iter_mut()
            .find(|row| row.seq == seq && row.status == BOX_BEFORE_SPAWNED)?;
        row.status = BOX_SPAWNED;
        row.site_id = site_id;
        row.position_x = position_x;
        row.position_z = position_z;
        let map = self.maps.get_mut(&site_id)?;
        map.fixtures.push(UserFixture {
            fixture_id,
            position_x,
            position_z,
            hp: 0,
            status: FIXTURE_SPAWNED,
            group_id: 0,
        });
        let seq = map.drops.iter().map(|drop| drop.seq).max().unwrap_or(0) + 1;
        map.drops.push(UserDrop {
            resource_type: RT_MYSEKAI_MATERIAL,
            resource_id: material,
            position_x,
            position_z,
            hp: 0,
            seq,
            status: DROP_BEFORE,
            quantity: 1,
            group_id: 0,
        });
        Some((self.boxes.clone(), map.clone()))
    }

    /// `HarvestApiMock`: echo. Stamina := each type's sent rest; the tool's
    /// rows are the server document's after its tool use policy (a
    /// durability that reaches zero consumes one of the quantity and resets);
    /// the target's hp := the sent hp, harvested on the last attack; drop
    /// rows above the sent hp (all on the last attack) become dropped.
    pub(crate) fn harvest(&mut self, request: &HarvestRequest) -> SuiteUserReply {
        self.stamina = Stamina {
            normal: request.stamina_rests[0],
            enhance: request.stamina_rests[1],
            boost: request.stamina_rests[2],
        };
        let tools = request
            .tool
            .and_then(|(tool_id, use_count, sent_durability)| {
                let Ok(tool_id) = i32::try_from(tool_id) else {
                    warn!("[harvest-mock] HarvestApiMock: tool {tool_id} is not a 32-bit tool id");
                    return None;
                };
                let rows = crate::server::tools::harvest_tool_use(tool_id, use_count, sent_durability);
                if rows.is_none() {
                    warn!("[harvest-mock] HarvestApiMock: no server model holds userMysekaiTools; the reply carries no tools");
                }
                rows
            })
            .map(|rows| user_tools(&rows));
        let map = self.maps.get_mut(&request.site_id);
        let map = match map {
            Some(map) => {
                if let Some(fixture) = map.fixtures.iter_mut().find(|fixture| {
                    fixture.position_x == request.position_x
                        && fixture.position_z == request.position_z
                        && fixture.fixture_id == request.fixture_id
                }) {
                    fixture.hp = request.hp;
                    if request.is_last_attack {
                        fixture.status = FIXTURE_HARVESTED;
                    }
                } else {
                    warn!(
                        "[harvest-mock] HarvestApiMock: no fixture {} at ({}, {}) on site {}",
                        request.fixture_id, request.position_x, request.position_z, request.site_id
                    );
                }
                for drop in &mut map.drops {
                    if drop.position_x == request.position_x
                        && drop.position_z == request.position_z
                        && drop.status == DROP_BEFORE
                        && (request.is_last_attack || drop.hp > request.hp)
                    {
                        drop.status = DROP_DROPPED;
                    }
                }
                Some(map.clone())
            }
            None => None,
        };
        self.reply(map, tools)
    }

    /// `GatherApiMock`: echo. Materials grow by the gathered quantities (the
    /// other resource lists likewise, by type); the gathered rows leave the
    /// map (the client drop-status enum has no "gathered" value, and a row
    /// left as dropped would be re-created on the next entry).
    pub(crate) fn gather(&mut self, request: &GatherRequest) -> SuiteUserReply {
        for drop in &request.drops {
            if drop.resource_type == RT_MYSEKAI_MATERIAL {
                *self.materials.entry(drop.resource_id).or_default() += drop.quantity as i64;
            } else {
                *self
                    .others
                    .entry((drop.resource_type, drop.resource_id))
                    .or_default() += drop.quantity as i64;
            }
        }
        let map = self.maps.get_mut(&request.site_id).map(|map| {
            map.drops.retain(|row| {
                !request.drops.iter().any(|drop| {
                    drop.position_x == row.position_x
                        && drop.position_z == row.position_z
                        && drop.seq == row.seq
                        && drop.hp == row.hp
                })
            });
            map.clone()
        });
        self.reply(map, None)
    }

    fn reply(&self, map: Option<UserHarvestMap>, tools: Option<Vec<UserTool>>) -> SuiteUserReply {
        SuiteUserReply {
            stamina: self.stamina,
            tools,
            map,
            materials: self.materials.clone(),
        }
    }
}

/// Update: a new refresh window replaces the harvest maps, between harvest
/// site visits only (a loaded harvest site keeps the rows its objects stand
/// for; the next arrival reads the new maps).
pub(crate) fn refresh_maps(
    arrival: Res<super::arrival::HarvestArrival>,
    mock: Option<ResMut<HarvestServerMock>>,
    user: Option<ResMut<super::catalog::HarvestUserData>>,
) {
    let (Some(mut mock), Some(mut user)) = (mock, user) else {
        return;
    };
    if arrival.site_id().is_some() {
        return;
    }
    let Some(window) = crate::server::refresh_window_start() else {
        return;
    };
    if let Some(maps) = mock.refresh(window) {
        user.maps = maps;
    }
}

/// Every harvest site's map for one refresh window.
fn generate_maps(inputs: &MockInputs, rules: &MapRules, window: i64) -> BTreeMap<u32, UserHarvestMap> {
    inputs
        .harvest_sites
        .iter()
        .map(|&site_id| (site_id, generate_map(site_id, window, rules, inputs)))
        .collect()
}

/// One weighted draw.
fn pick<'a, T>(rng: &mut MockRng, items: &'a [T], weight: impl Fn(&T) -> u32) -> Option<&'a T> {
    let total: u64 = items.iter().map(|item| u64::from(weight(item))).sum();
    if total == 0 {
        return None;
    }
    let mut draw = rng.next_u64() % total;
    for item in items {
        let w = u64::from(weight(item));
        if draw < w {
            return Some(item);
        }
        draw -= w;
    }
    None
}

/// `HarvestMapMock` for one site in one refresh window (the map rules, see
/// the module comment). Deterministic per (site, window).
pub(crate) fn generate_map(
    site_id: u32,
    window: i64,
    rules: &MapRules,
    inputs: &MockInputs,
) -> UserHarvestMap {
    let mut map = UserHarvestMap {
        site_id,
        ..default()
    };
    let Some(site) = rules.sites.iter().find(|site| site.site_id == site_id) else {
        warn!("[harvest-mock] HarvestMapMock: the map rules have no site {site_id}; the mock places nothing there");
        return map;
    };
    let mut rng = MockRng(0x4841_5256_0000_0000 ^ u64::from(site_id) ^ (window as u64).rotate_left(20));
    let mut seq = 0;
    for group in &site.groups {
        let Some(mix) = pick(&mut rng, &group.mixes, |mix| mix.weight) else {
            continue;
        };
        let wanted: Vec<i32> = mix
            .fixtures
            .iter()
            .flat_map(|&(id, count)| std::iter::repeat_n(id, count))
            .collect();
        let mut free: Vec<[i32; 2]> = group
            .points
            .iter()
            .filter(|point| !group.fixed_points.contains(point))
            .copied()
            .collect();
        let mut slots = group.fixed_points.clone();
        while slots.len() < wanted.len() && !free.is_empty() {
            let at = rng.below(free.len());
            slots.push(free.swap_remove(at));
        }
        if slots.len() < wanted.len() {
            warn!(
                "[harvest-mock] HarvestMapMock site {site_id}: {} {} fixtures on {} points; the rest is not placed",
                wanted.len(),
                group.kind,
                slots.len()
            );
        }
        for (&fixture_id, &[x, z]) in wanted.iter().zip(&slots) {
            let Some(row) = inputs.fixtures.iter().find(|row| row.id == fixture_id) else {
                warn!(
                    "[harvest-mock] HarvestMapMock site {site_id}: rule fixture {fixture_id} ({}) has no master row; not placed",
                    group.kind
                );
                continue;
            };
            map.fixtures.push(UserFixture {
                fixture_id,
                position_x: x,
                position_z: z,
                hp: row.hp,
                status: FIXTURE_SPAWNED,
                group_id: 0,
            });
            let Some(drops) = site.drops.iter().find(|drops| drops.fixture_id == fixture_id) else {
                warn!("[harvest-mock] HarvestMapMock site {site_id}: fixture {fixture_id} has no drop rule; it drops nothing");
                continue;
            };
            let mut push = |(word, resource_id, quantity): (&str, i64, i32), hp: i32| {
                seq += 1;
                map.drops.push(UserDrop {
                    resource_type: resource_type_value(word).expect("checked by MapRules::builtin"),
                    resource_id,
                    position_x: x,
                    position_z: z,
                    hp,
                    seq,
                    status: DROP_BEFORE,
                    quantity,
                    group_id: 0,
                });
            };
            if let Some((word, id, quantity)) = &drops.step_resource {
                for &hp in &drops.step_hps {
                    push((word, *id, *quantity), hp);
                }
            }
            let rows = pick(&mut rng, &drops.row_counts, |row| row.1).map_or(0, |row| row.0);
            for k in 0..rows {
                if k == 0 {
                    if let Some(track_type) = &drops.record_track_type {
                        match inputs.music_records.get(track_type).filter(|ids| !ids.is_empty()) {
                            Some(ids) => push(("mysekai_music_record", ids[rng.below(ids.len())], 1), 0),
                            None => warn!(
                                "[harvest-mock] HarvestMapMock site {site_id}: no master music record of track type {track_type}; fixture {fixture_id} drops no record"
                            ),
                        }
                        continue;
                    }
                }
                let table = if k == 0 { &drops.first_row } else { &drops.other_rows };
                if let Some((word, id, quantity, _)) = pick(&mut rng, table, |row| row.3) {
                    push((word, *id, *quantity), 0);
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// Master rows for every fixture the built-in rules name (struck kinds
    /// at hp 90), the four harvest sites and a record pool.
    fn inputs() -> MockInputs {
        let rules = MapRules::builtin();
        let mut fixtures: Vec<MockFixtureRow> = Vec::new();
        for group in rules.sites.iter().flat_map(|site| &site.groups) {
            for &(id, _) in group.mixes.iter().flat_map(|mix| &mix.fixtures) {
                if !fixtures.iter().any(|row| row.id == id) {
                    let struck = group.kind == "wood" || group.kind == "mineral";
                    fixtures.push(MockFixtureRow {
                        id,
                        kind: 0,
                        hp: if struck { 90 } else { 0 },
                    });
                }
            }
        }
        MockInputs {
            fixtures,
            materials: vec![MockMaterialRow {
                id: 13,
                material_type: 3,
                site_ids: vec![5],
            }],
            max_normal_stamina: 1000,
            boost_grant: 100,
            harvest_sites: rules.sites.iter().map(|site| site.site_id).collect(),
            music_records: [("music".to_owned(), vec![1, 2, 3])].into_iter().collect(),
        }
    }

    /// Every placed fixture stands on a point of a group that names it, no
    /// point holds two, the fixed points are filled, and each fixture's rows
    /// are its step rows then at least one last-attack row, seq contiguous.
    #[test]
    fn maps_follow_the_rules() {
        let rules = MapRules::builtin();
        let inputs = inputs();
        for window in [0, 1_790_200_000_000, 1_790_243_200_000] {
            for site in &rules.sites {
                let map = generate_map(site.site_id, window, &rules, &inputs);
                assert!(!map.fixtures.is_empty(), "site {}", site.site_id);
                let mut used = std::collections::HashSet::new();
                for fixture in &map.fixtures {
                    let point = [fixture.position_x, fixture.position_z];
                    assert!(used.insert(point), "site {} point {point:?} twice", site.site_id);
                    assert!(site.groups.iter().any(|group| group.points.contains(&point)
                        && group.mixes.iter().any(|mix| mix.fixtures.iter().any(|f| f.0 == fixture.fixture_id))));
                    let drops = site.drops.iter().find(|d| d.fixture_id == fixture.fixture_id).unwrap();
                    let hps: Vec<i32> = map
                        .drops
                        .iter()
                        .filter(|d| d.position_x == point[0] && d.position_z == point[1])
                        .map(|d| d.hp)
                        .collect();
                    assert_eq!(&hps[..drops.step_hps.len()], &drops.step_hps[..]);
                    assert!(hps.len() > drops.step_hps.len());
                    assert!(hps[drops.step_hps.len()..].iter().all(|hp| *hp == 0));
                }
                for group in &site.groups {
                    for point in &group.fixed_points {
                        assert!(used.contains(point), "site {} fixed point {point:?}", site.site_id);
                    }
                }
                let seqs: Vec<i32> = map.drops.iter().map(|d| d.seq).collect();
                assert_eq!(seqs, (1..=seqs.len() as i32).collect::<Vec<_>>());
                assert_eq!(generate_map(site.site_id, window, &rules, &inputs).fixtures, map.fixtures);
            }
        }
        // A new window draws other points.
        assert_ne!(
            generate_map(5, 0, &rules, &inputs).fixtures,
            generate_map(5, 43_200_000, &rules, &inputs).fixtures
        );
    }

    #[test]
    fn refresh_replaces_the_maps_once_per_window() {
        let mut mock = HarvestServerMock::new(inputs(), Some(0));
        assert!(mock.refresh(0).is_none());
        let maps = mock.refresh(43_200_000).expect("a new window");
        assert_eq!(mock.login().0[&5].fixtures, maps[&5].fixtures);
        assert!(mock.refresh(43_200_000).is_none());
    }

    /// The echo replies keep the drop rows consistent with the client: rows
    /// above the sent hp fall; gathered rows leave the map.
    #[test]
    fn harvest_and_gather_echo() {
        let mut mock = HarvestServerMock::new(inputs(), Some(0));
        let (maps, stamina) = mock.login();
        assert_eq!(stamina.normal, 1000);
        assert_eq!(stamina.boost, 100);
        let tree = maps[&5]
            .fixtures
            .iter()
            .find(|f| f.fixture_id == 1001)
            .unwrap()
            .clone();
        let reply = mock.harvest(&HarvestRequest {
            site_id: 5,
            position_x: tree.position_x,
            position_z: tree.position_z,
            fixture_id: 1001,
            hp: 50,
            is_last_attack: false,
            tool: Some((6, 2, 28)),
            stamina_rests: [960, 0, 0],
        });
        assert_eq!(reply.stamina.normal, 960);
        let map = reply.map.unwrap();
        let at = |d: &&UserDrop| d.position_x == tree.position_x && d.position_z == tree.position_z;
        let steps: Vec<(i32, i32)> = map
            .drops
            .iter()
            .filter(at)
            .filter(|d| d.hp > 0)
            .map(|d| (d.hp, d.status))
            .collect();
        assert_eq!(
            steps,
            vec![
                (1, DROP_BEFORE),
                (16, DROP_BEFORE),
                (31, DROP_BEFORE),
                (46, DROP_BEFORE),
                (61, DROP_DROPPED),
                (76, DROP_DROPPED)
            ]
        );
        let dropped: Vec<UserDrop> = map
            .drops
            .iter()
            .filter(at)
            .filter(|d| d.status == DROP_DROPPED)
            .cloned()
            .collect();
        let reply = mock.gather(&GatherRequest {
            site_id: 5,
            drops: dropped,
        });
        let left: Vec<i32> = reply
            .map
            .unwrap()
            .drops
            .iter()
            .filter(at)
            .filter(|d| d.hp > 0)
            .map(|d| d.hp)
            .collect();
        assert_eq!(left, vec![1, 16, 31, 46]);
        assert_eq!(reply.materials.values().sum::<i64>(), 2);
    }
}
