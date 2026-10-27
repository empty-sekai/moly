//! Existing offline statistical map policy shared by the server and client.
//! It is not a reconstruction of the upstream server algorithm.
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

//! Existing offline statistical map policy, shared by server and client.
//! This is not a reconstruction of the upstream server algorithm.
use bevy::prelude::*;
use moly_protocol::harvest::{UserDrop, UserFixture, UserHarvestMap, DROP_BEFORE, FIXTURE_SPAWNED};
use serde::Deserialize;
use std::collections::BTreeMap;
const RT_MATERIAL: i32 = 2;
const RT_MYSEKAI_FIXTURE: i32 = 39;
const RT_MYSEKAI_MATERIAL: i32 = 41;
const RT_MYSEKAI_ITEM: i32 = 42;
const RT_MYSEKAI_MUSIC_RECORD: i32 = 44;
/// The master rows the mock draws from (joined by the harvest catalog).
#[derive(Clone, Debug)]
pub struct MockFixtureRow {
    pub id: i32,
    /// Master fixture type value (wood 0 ... birthday_plant 9).
    pub kind: i32,
    pub hp: i32,
}

#[derive(Clone, Debug)]
pub struct MockMaterialRow {
    pub id: i64,
    /// `MysekaiMaterialType` value (wood 0, mineral 1, plant 2, junk 3, ...).
    pub material_type: i32,
    pub site_ids: Vec<u32>,
}

/// What the panel is built from.
#[derive(Clone)]
pub struct MockInputs {
    pub fixtures: Vec<MockFixtureRow>,
    pub materials: Vec<MockMaterialRow>,
    /// Master `maxStamina` of the normal pool.
    pub max_normal_stamina: i32,
    /// `recoveryBoostStamina` of one boost recovery.
    pub boost_grant: i32,
    /// The harvest sites (master category harvest).
    pub harvest_sites: Vec<u32>,
    /// Master music record ids by `mysekaiMusicTrackType`.
    pub music_records: BTreeMap<String, Vec<i64>>,
}

/// Kinds left out of the mock map, and who places them.
pub const MOCK_EXCLUDED: [(&str, &str); 2] = [
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
pub struct MapRules {
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
    pub fn builtin() -> Self {
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

/// Every harvest site's map for one refresh window.
pub fn generate_maps(
    inputs: &MockInputs,
    rules: &MapRules,
    window: i64,
) -> BTreeMap<u32, UserHarvestMap> {
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
pub fn generate_map(
    site_id: u32,
    window: i64,
    rules: &MapRules,
    inputs: &MockInputs,
) -> UserHarvestMap {
    let mut map = UserHarvestMap {
        site_id,
        ..Default::default()
    };
    let Some(site) = rules.sites.iter().find(|site| site.site_id == site_id) else {
        warn!("[harvest-mock] HarvestMapMock: the map rules have no site {site_id}; the mock places nothing there");
        return map;
    };
    let mut rng =
        MockRng(0x4841_5256_0000_0000 ^ u64::from(site_id) ^ (window as u64).rotate_left(20));
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
            let Some(drops) = site
                .drops
                .iter()
                .find(|drops| drops.fixture_id == fixture_id)
            else {
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
                let table = if k == 0 {
                    &drops.first_row
                } else {
                    &drops.other_rows
                };
                if let Some((word, id, quantity, _)) = pick(&mut rng, table, |row| row.3) {
                    push((word, *id, *quantity), 0);
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod rule_tests {
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
                    assert!(
                        used.insert(point),
                        "site {} point {point:?} twice",
                        site.site_id
                    );
                    assert!(site.groups.iter().any(|group| group.points.contains(&point)
                        && group
                            .mixes
                            .iter()
                            .any(|mix| mix.fixtures.iter().any(|f| f.0 == fixture.fixture_id))));
                    let drops = site
                        .drops
                        .iter()
                        .find(|d| d.fixture_id == fixture.fixture_id)
                        .unwrap();
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
                        assert!(
                            used.contains(point),
                            "site {} fixed point {point:?}",
                            site.site_id
                        );
                    }
                }
                let seqs: Vec<i32> = map.drops.iter().map(|d| d.seq).collect();
                assert_eq!(seqs, (1..=seqs.len() as i32).collect::<Vec<_>>());
                assert_eq!(
                    generate_map(site.site_id, window, &rules, &inputs).fixtures,
                    map.fixtures
                );
            }
        }
        // A new window draws other points.
        assert_ne!(
            generate_map(5, 0, &rules, &inputs).fixtures,
            generate_map(5, 43_200_000, &rules, &inputs).fixtures
        );
    }
}
