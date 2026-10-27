//! Shared source master join for JP 7.0.0 BulkHarvest.
use moly_law::bulk_harvest::{Candidate, FixtureKey};
use moly_protocol::harvest::{UserHarvestMap, FIXTURE_SPAWNED};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub id: i32,
    pub name: String,
    pub group_id: i32,
    pub group_name: String,
    pub ordinal: usize,
    pub required_tool_id: Option<i32>,
}

#[derive(Clone, Debug, Default)]
pub struct BulkHarvestMasters {
    pub sites: BTreeMap<i32, Vec<Target>>,
    /// fixture ID -> (bulk target ID, original HP, lastAttackStamina).
    pub fixtures: BTreeMap<i32, (i32, i32, i32)>,
}

impl BulkHarvestMasters {
    pub fn parse(rows: &BTreeMap<&'static str, Vec<Value>>) -> Result<Self, String> {
        let table = |name| rows.get(name).ok_or_else(|| format!("{name} is absent"));
        let mut groups = BTreeMap::new();
        for row in table("mysekaiSiteBulkHarvestTargetGroups")? {
            groups.insert(
                crate::int32(row, "id")?,
                (
                    crate::int32(row, "seq")?,
                    row["name"].as_str().unwrap_or("").to_owned(),
                    row.get("requiredToolId")
                        .and_then(Value::as_i64)
                        .and_then(|n| i32::try_from(n).ok())
                        .filter(|n| *n > 0),
                ),
            );
        }
        let mut targets = BTreeMap::new();
        for row in table("mysekaiSiteBulkHarvestTargets")? {
            let id = crate::int32(row, "id")?;
            let group_id = crate::int32(row, "mysekaiSiteBulkHarvestTargetGroupId")?;
            let (group_seq, group_name, tool) = groups
                .get(&group_id)
                .ok_or_else(|| format!("bulk target {id}: group {group_id} absent"))?;
            targets.insert(
                id,
                (
                    *group_seq,
                    crate::int32(row, "seq")?,
                    Target {
                        id,
                        name: row["name"].as_str().unwrap_or("").to_owned(),
                        group_id,
                        group_name: group_name.clone(),
                        ordinal: 0,
                        required_tool_id: *tool,
                    },
                ),
            );
        }
        let mut out = Self::default();
        for row in table("mysekaiSiteBulkHarvests")? {
            let site = crate::int32(row, "mysekaiSiteId")?;
            let id = crate::int32(row, "mysekaiSiteBulkHarvestTargetId")?;
            let target = targets
                .get(&id)
                .ok_or_else(|| format!("bulk site {site}: target {id} absent"))?;
            let site = out.sites.entry(site).or_default();
            if !site.iter().any(|row| row.id == id) {
                site.push(target.2.clone());
            }
        }
        for site in out.sites.values_mut() {
            site.sort_by_key(|row| {
                let value = &targets[&row.id];
                (value.0, value.1)
            });
            for (ordinal, row) in site.iter_mut().enumerate() {
                row.ordinal = ordinal;
            }
        }
        for row in table("mysekaiSiteHarvestFixtures")? {
            let id = crate::int32(row, "id")?;
            out.fixtures.insert(
                id,
                (
                    crate::optional_int32(row, "mysekaiSiteBulkHarvestTargetId")?,
                    crate::int32(row, "hp")?,
                    crate::int32(row, "lastAttackStamina")?,
                ),
            );
        }
        Ok(out)
    }

    pub fn targets(&self, site: i32) -> &[Target] {
        self.sites.get(&site).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Source X/Z coordinates. Rendering reflections belong at the client adapter.
    pub fn candidates(&self, map: &UserHarvestMap, player_x: f32, player_z: f32) -> Vec<Candidate> {
        map.fixtures
            .iter()
            .filter_map(|row| {
                let (target_id, _, last) = self.fixtures.get(&row.fixture_id)?;
                let target = self
                    .targets(map.site_id as i32)
                    .iter()
                    .find(|target| target.id == *target_id)?;
                let dx = row.position_x as f32 - player_x;
                let dz = row.position_z as f32 - player_z;
                Some(Candidate {
                    key: FixtureKey {
                        fixture_id: row.fixture_id,
                        position_x: row.position_x,
                        position_z: row.position_z,
                    },
                    target_id: *target_id,
                    ordinal: target.ordinal,
                    distance_squared: dx * dx + dz * dz,
                    hp: row.hp,
                    last_attack_stamina: *last,
                    required_tool_id: target.required_tool_id,
                    spawned: row.status == FIXTURE_SPAWNED,
                    view_exists: true,
                })
            })
            .collect()
    }
}
