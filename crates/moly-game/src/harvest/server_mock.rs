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
//!   and their drop rows), generated once per session by the mock's placement
//!   rule below.
//! - The tools and stamina mock: the user tool rows and the stamina triple.
//! - `HarvestApiMock` and `GatherApiMock`: echo replies to the harvest and
//!   gather requests.
//!
//! The placement rule is **the mock's own, not the source's** (the source
//! places on the server from spawn zones that are not on disk). It picks
//! integer site-local cells inside the site's unavailable-spot frame, away
//! from every unavailable rectangle, 3 m clear of the arrival point and 3 m
//! apart; the kinds and counts per site are the mock's choice, and a kind
//! appears only on a site where a material of its type lists that site.
//! Treasure boxes (paper airplane lane), tone (camera lane) and birthday
//! plants (event calendar) are left out and named.

use std::collections::BTreeMap;

use bevy::prelude::*;

use super::law::{Stamina, ToolType};

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

/// `UserMysekaiHarvestMap`.
#[derive(Clone, Debug, Default)]
pub(crate) struct UserHarvestMap {
    pub(crate) site_id: u32,
    pub(crate) fixtures: Vec<UserFixture>,
    pub(crate) drops: Vec<UserDrop>,
}

/// One `UserMysekaiTool` row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserTool {
    pub(crate) tool_id: i64,
    pub(crate) quantity: i32,
    pub(crate) durability: i32,
}

/// The master rows the mock draws from (joined by the harvest catalog).
#[derive(Clone, Debug)]
pub(crate) struct MockFixtureRow {
    pub(crate) id: i32,
    /// Master fixture type value (wood 0 ... birthday_plant 9).
    pub(crate) kind: i32,
    pub(crate) hp: i32,
    pub(crate) rare: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct MockMaterialRow {
    pub(crate) id: i64,
    /// `MysekaiMaterialType` value (wood 0, mineral 1, plant 2, junk 3, ...).
    pub(crate) material_type: i32,
    pub(crate) site_ids: Vec<u32>,
}

/// One `mysekaiSiteHarvestUnavailableSpots` row. The rectangle covers
/// `x in [startX, startX + width)` and `z in (startZ - height, startZ]`: read
/// from how the rows tile (the site-5 strips at z 30 height 7 and the side
/// strips from z 23 down join without gap or overlap under this reading).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MockSpot {
    pub(crate) site_id: u32,
    pub(crate) start_x: i32,
    pub(crate) start_z: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl MockSpot {
    fn contains(&self, x: i32, z: i32) -> bool {
        self.start_x <= x
            && x < self.start_x + self.width
            && self.start_z - self.height < z
            && z <= self.start_z
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MockToolRow {
    pub(crate) id: i64,
    pub(crate) tool_type: ToolType,
    pub(crate) level: i32,
    pub(crate) max_durability: i32,
}

/// What the panel is built from.
pub(crate) struct MockInputs {
    pub(crate) fixtures: Vec<MockFixtureRow>,
    pub(crate) materials: Vec<MockMaterialRow>,
    pub(crate) spots: Vec<MockSpot>,
    pub(crate) tools: Vec<MockToolRow>,
    /// Master `maxStamina` of the normal pool.
    pub(crate) max_normal_stamina: i32,
    /// The harvest sites (master category harvest).
    pub(crate) harvest_sites: Vec<u32>,
    /// A blueprint id for the toolbox's drop row (mock choice: the lowest).
    pub(crate) toolbox_blueprint: Option<i64>,
}

/// Kinds the map mock places, with the mock's per-site count and the material
/// type its drop rows come from.
const MOCK_KINDS: [(i32, &str, usize, Option<i32>); 6] = [
    (0, "wood", 6, Some(0)),
    (1, "mineral", 5, Some(1)),
    (2, "plant", 5, Some(2)),
    (5, "other", 4, Some(3)),
    (8, "driftage", 1, Some(3)),
    (7, "toolbox", 1, None),
];
/// Kinds left out of the mock map, and who places them.
pub(crate) const MOCK_EXCLUDED: [(&str, &str); 4] = [
    (
        "treasure_box_transport",
        "paper airplane and spawn API (later lane)",
    ),
    (
        "treasure_box_fixed",
        "treasure boxes need AutoMove and the box open (later lane)",
    ),
    (
        "tone",
        "HarvestTone camera 14 and the BGM fade (later lane)",
    ),
    ("birthday_plant", "birthday party calendar (event gated)"),
];

const ARRIVAL_CLEARANCE: i32 = 3;
const SPACING: i32 = 3;
/// Mock drop thresholds of a multi-hit fixture (hp 90): three rows fall at
/// 60, at 30 and on the last attack.
const MULTI_THRESHOLDS: [i32; 3] = [60, 30, 0];
/// Resource types of drop rows.
pub(crate) const RT_MYSEKAI_BLUEPRINT: i32 = 40;
pub(crate) const RT_MYSEKAI_MATERIAL: i32 = 41;

/// The panel resource.
#[derive(Resource)]
pub(crate) struct HarvestServerMock {
    maps: BTreeMap<u32, UserHarvestMap>,
    tools: Vec<UserTool>,
    tool_rows: Vec<MockToolRow>,
    stamina: Stamina,
    materials: BTreeMap<i64, i64>,
    others: BTreeMap<(i32, i64), i64>,
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
    pub(crate) tools: Vec<UserTool>,
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
    /// Build the panel: every harvest site's map by the mock's rule, the tool
    /// rows (one pickaxe and one axe of level 1 at full durability) and the
    /// normal stamina at its master maximum.
    pub(crate) fn new(inputs: &MockInputs) -> Self {
        let mut maps = BTreeMap::new();
        for &site_id in &inputs.harvest_sites {
            maps.insert(site_id, generate_map(site_id, inputs));
        }
        let tools = [ToolType::Pickaxe, ToolType::Axe]
            .into_iter()
            .filter_map(|tool_type| {
                inputs
                    .tools
                    .iter()
                    .filter(|row| row.tool_type == tool_type && row.level == 1)
                    .min_by_key(|row| row.id)
            })
            .map(|row| UserTool {
                tool_id: row.id,
                quantity: 1,
                durability: row.max_durability,
            })
            .collect::<Vec<_>>();
        Self {
            maps,
            tools,
            tool_rows: inputs.tools.clone(),
            stamina: Stamina {
                normal: inputs.max_normal_stamina,
                enhance: 0,
                boost: 0,
            },
            materials: BTreeMap::new(),
            others: BTreeMap::new(),
        }
    }

    /// The login fetch: the user lists as the client first receives them.
    pub(crate) fn login(&self) -> (BTreeMap<u32, UserHarvestMap>, Vec<UserTool>, Stamina) {
        (self.maps.clone(), self.tools.clone(), self.stamina)
    }

    /// `HarvestApiMock`: echo. Stamina := each type's sent rest; the tool's
    /// durability follows its use count (a durability that reaches zero
    /// consumes one of the quantity and resets); the target's hp := the sent
    /// hp, harvested on the last attack; drop rows above the sent hp (all on
    /// the last attack) become dropped.
    pub(crate) fn harvest(&mut self, request: &HarvestRequest) -> SuiteUserReply {
        self.stamina = Stamina {
            normal: request.stamina_rests[0],
            enhance: request.stamina_rests[1],
            boost: request.stamina_rests[2],
        };
        if let Some((tool_id, use_count, sent_durability)) = request.tool {
            let max = self
                .tool_rows
                .iter()
                .find(|row| row.id == tool_id)
                .map(|row| row.max_durability);
            if let (Some(index), Some(max)) = (
                self.tools.iter().position(|tool| tool.tool_id == tool_id),
                max,
            ) {
                let tool = &mut self.tools[index];
                tool.durability -= use_count;
                while tool.durability <= 0 && tool.quantity > 0 {
                    tool.quantity -= 1;
                    tool.durability += max;
                }
                if tool.durability != sent_durability && tool.quantity > 0 {
                    warn!(
                        "[harvest-mock] HarvestApiMock tool {tool_id}: server durability {} differs from the sent {sent_durability}",
                        tool.durability
                    );
                }
                if tool.quantity < 1 {
                    self.tools.remove(index);
                }
            } else {
                warn!("[harvest-mock] HarvestApiMock: tool {tool_id} is not in the user tool list");
            }
        }
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
        self.reply(map)
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
        self.reply(map)
    }

    fn reply(&self, map: Option<UserHarvestMap>) -> SuiteUserReply {
        SuiteUserReply {
            stamina: self.stamina,
            tools: self.tools.clone(),
            map,
            materials: self.materials.clone(),
        }
    }
}

/// `HarvestMapMock` for one site (the mock's placement rule, see the module
/// comment). Deterministic per site id.
pub(crate) fn generate_map(site_id: u32, inputs: &MockInputs) -> UserHarvestMap {
    let spots: Vec<&MockSpot> = inputs
        .spots
        .iter()
        .filter(|spot| spot.site_id == site_id)
        .collect();
    let mut map = UserHarvestMap {
        site_id,
        ..default()
    };
    if spots.is_empty() {
        warn!("[harvest-mock] HarvestMapMock: site {site_id} has no unavailable-spot rows; the mock places nothing there");
        return map;
    }
    let min_x = spots.iter().map(|s| s.start_x).min().unwrap();
    let max_x = spots.iter().map(|s| s.start_x + s.width - 1).max().unwrap();
    let max_z = spots.iter().map(|s| s.start_z).max().unwrap();
    let min_z = spots
        .iter()
        .map(|s| s.start_z - s.height + 1)
        .min()
        .unwrap();
    let mut free: Vec<(i32, i32)> = Vec::new();
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            if x.abs() < ARRIVAL_CLEARANCE && z.abs() < ARRIVAL_CLEARANCE {
                continue;
            }
            if spots.iter().any(|spot| spot.contains(x, z)) {
                continue;
            }
            free.push((x, z));
        }
    }
    let mut rng = MockRng(0x4841_5256_0000_0000 ^ site_id as u64);
    let mut seq = 0;
    for (kind, word, count, material_type) in MOCK_KINDS {
        let materials: Vec<&MockMaterialRow> = match material_type {
            Some(material_type) => inputs
                .materials
                .iter()
                .filter(|row| row.material_type == material_type && row.site_ids.contains(&site_id))
                .collect(),
            None => Vec::new(),
        };
        if material_type.is_some() && materials.is_empty() {
            info!("[harvest-mock] HarvestMapMock site {site_id}: no {word} (no material of its type lists this site)");
            continue;
        }
        if material_type.is_none() && inputs.toolbox_blueprint.is_none() {
            info!("[harvest-mock] HarvestMapMock site {site_id}: no {word} (no blueprint row for its drop)");
            continue;
        }
        let common: Vec<&MockFixtureRow> = inputs
            .fixtures
            .iter()
            .filter(|row| row.kind == kind && !row.rare)
            .collect();
        let rare: Vec<&MockFixtureRow> = inputs
            .fixtures
            .iter()
            .filter(|row| row.kind == kind && row.rare)
            .collect();
        if common.is_empty() && rare.is_empty() {
            warn!("[harvest-mock] HarvestMapMock: no master row of kind {word}");
            continue;
        }
        for _ in 0..count {
            if free.is_empty() {
                warn!("[harvest-mock] HarvestMapMock site {site_id}: no free cell left for {word}");
                break;
            }
            let (x, z) = free[rng.below(free.len())];
            free.retain(|(fx, fz)| (fx - x).abs() >= SPACING || (fz - z).abs() >= SPACING);
            // Mock choice: one in ten objects is the kind's rare row.
            let row = if !rare.is_empty() && (rng.below(10) == 0 || common.is_empty()) {
                rare[rng.below(rare.len())]
            } else {
                common[rng.below(common.len())]
            };
            map.fixtures.push(UserFixture {
                fixture_id: row.id,
                position_x: x,
                position_z: z,
                hp: row.hp,
                status: FIXTURE_SPAWNED,
                group_id: 0,
            });
            let thresholds: &[i32] = if row.hp > 0 {
                &MULTI_THRESHOLDS
            } else if kind == 5 || kind == 8 {
                &[0, 0]
            } else {
                &[0]
            };
            for &threshold in thresholds {
                if threshold >= row.hp && row.hp > 0 {
                    continue;
                }
                seq += 1;
                let (resource_type, resource_id) = match material_type {
                    Some(_) => (
                        RT_MYSEKAI_MATERIAL,
                        materials[rng.below(materials.len())].id,
                    ),
                    None => (
                        RT_MYSEKAI_BLUEPRINT,
                        inputs.toolbox_blueprint.expect("checked above"),
                    ),
                };
                map.drops.push(UserDrop {
                    resource_type,
                    resource_id,
                    position_x: x,
                    position_z: z,
                    hp: threshold,
                    seq,
                    status: DROP_BEFORE,
                    quantity: 1,
                    group_id: 0,
                });
            }
        }
    }
    map
}

#[cfg(test)]
mod value_checks {
    use super::*;

    fn inputs() -> MockInputs {
        MockInputs {
            fixtures: vec![
                MockFixtureRow {
                    id: 1002,
                    kind: 0,
                    hp: 90,
                    rare: false,
                },
                MockFixtureRow {
                    id: 2001,
                    kind: 1,
                    hp: 90,
                    rare: false,
                },
                MockFixtureRow {
                    id: 4009,
                    kind: 2,
                    hp: 0,
                    rare: false,
                },
                MockFixtureRow {
                    id: 5004,
                    kind: 5,
                    hp: 0,
                    rare: false,
                },
            ],
            materials: vec![
                MockMaterialRow {
                    id: 1,
                    material_type: 0,
                    site_ids: vec![5],
                },
                MockMaterialRow {
                    id: 6,
                    material_type: 1,
                    site_ids: vec![5],
                },
                MockMaterialRow {
                    id: 13,
                    material_type: 3,
                    site_ids: vec![5],
                },
            ],
            // The four site-5 frame strips.
            spots: vec![
                MockSpot {
                    site_id: 5,
                    start_x: -30,
                    start_z: 30,
                    width: 60,
                    height: 7,
                },
                MockSpot {
                    site_id: 5,
                    start_x: 13,
                    start_z: 23,
                    width: 17,
                    height: 53,
                },
                MockSpot {
                    site_id: 5,
                    start_x: -30,
                    start_z: -23,
                    width: 43,
                    height: 7,
                },
                MockSpot {
                    site_id: 5,
                    start_x: -30,
                    start_z: 23,
                    width: 12,
                    height: 46,
                },
            ],
            tools: vec![
                MockToolRow {
                    id: 1,
                    tool_type: ToolType::Pickaxe,
                    level: 1,
                    max_durability: 30,
                },
                MockToolRow {
                    id: 6,
                    tool_type: ToolType::Axe,
                    level: 1,
                    max_durability: 30,
                },
                MockToolRow {
                    id: 9,
                    tool_type: ToolType::Axe,
                    level: 4,
                    max_durability: 150,
                },
            ],
            max_normal_stamina: 1000,
            harvest_sites: vec![5],
            toolbox_blueprint: None,
        }
    }

    /// The mock's placement rule on the site-5 frame: every cell inside the
    /// frame's interior, outside every rectangle, clear of the arrival point
    /// and spaced; plant skipped (no plant material lists the site here).
    #[test]
    fn placement_rule_on_the_site_frame() {
        let inputs = inputs();
        let map = generate_map(5, &inputs);
        let kinds: Vec<i32> = map.fixtures.iter().map(|f| f.fixture_id).collect();
        assert_eq!(kinds.iter().filter(|id| **id == 1002).count(), 6);
        assert_eq!(kinds.iter().filter(|id| **id == 2001).count(), 5);
        assert_eq!(kinds.iter().filter(|id| **id == 4009).count(), 0);
        assert_eq!(kinds.iter().filter(|id| **id == 5004).count(), 4);
        for fixture in &map.fixtures {
            let (x, z) = (fixture.position_x, fixture.position_z);
            assert!(
                (-18..13).contains(&x) && (-22..=23).contains(&z),
                "({x}, {z})"
            );
            assert!(!inputs.spots.iter().any(|s| s.contains(x, z)));
            assert!(x.abs() >= ARRIVAL_CLEARANCE || z.abs() >= ARRIVAL_CLEARANCE);
        }
        for (i, a) in map.fixtures.iter().enumerate() {
            for b in &map.fixtures[i + 1..] {
                assert!(
                    (a.position_x - b.position_x).abs() >= SPACING
                        || (a.position_z - b.position_z).abs() >= SPACING
                );
            }
        }
        // Multi-hit rows: 60 / 30 / 0; junk mountains: two rows at 0.
        let tree = map.fixtures.iter().find(|f| f.fixture_id == 1002).unwrap();
        let rows: Vec<i32> = map
            .drops
            .iter()
            .filter(|d| d.position_x == tree.position_x && d.position_z == tree.position_z)
            .map(|d| d.hp)
            .collect();
        assert_eq!(rows, vec![60, 30, 0]);
        // Deterministic per site.
        assert_eq!(generate_map(5, &inputs).fixtures, map.fixtures);
    }

    /// The echo replies keep the drop rows consistent with the client: rows
    /// above the sent hp fall; gathered rows leave the map.
    #[test]
    fn harvest_and_gather_echo() {
        let mut mock = HarvestServerMock::new(&inputs());
        let (maps, tools, stamina) = mock.login();
        assert_eq!(stamina.normal, 1000);
        assert_eq!(
            tools.iter().map(|t| t.tool_id).collect::<Vec<_>>(),
            vec![1, 6]
        );
        let tree = maps[&5]
            .fixtures
            .iter()
            .find(|f| f.fixture_id == 1002)
            .unwrap()
            .clone();
        let reply = mock.harvest(&HarvestRequest {
            site_id: 5,
            position_x: tree.position_x,
            position_z: tree.position_z,
            fixture_id: 1002,
            hp: 50,
            is_last_attack: false,
            tool: Some((6, 2, 28)),
            stamina_rests: [960, 0, 0],
        });
        assert_eq!(reply.stamina.normal, 960);
        assert_eq!(
            reply
                .tools
                .iter()
                .find(|t| t.tool_id == 6)
                .unwrap()
                .durability,
            28
        );
        let map = reply.map.unwrap();
        let at = |d: &&UserDrop| d.position_x == tree.position_x && d.position_z == tree.position_z;
        let status: Vec<(i32, i32)> = map
            .drops
            .iter()
            .filter(at)
            .map(|d| (d.hp, d.status))
            .collect();
        assert_eq!(
            status,
            vec![(60, DROP_DROPPED), (30, DROP_BEFORE), (0, DROP_BEFORE)]
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
            .map(|d| d.hp)
            .collect();
        assert_eq!(left, vec![30, 0]);
        assert_eq!(reply.materials.values().sum::<i64>(), 1);
    }
}
