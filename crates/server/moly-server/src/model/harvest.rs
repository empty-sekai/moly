//! One durable offline map owner for ordinary and bulk harvest settlement.
//! Seed policy is the existing statistical map_rules policy, not upstream code.
use super::{ResponseKind, ServerModel, Tables, SECTION_GAMEDATA, SECTION_STAMINA};
use moly_law::bulk_harvest::{self as law, Budget, FixtureKey, Tool};
use moly_masters::{bulk_harvest::BulkHarvestMasters, offline_harvest as seed};
use moly_protocol::{api::*, harvest::*, rows::*, Refusal};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const SECTION: &str = "userMysekaiHarvestMaps";
pub(crate) const BOX_SECTION: &str = "userMysekaiTreasureBoxes";
pub(crate) const DOCUMENT_KEYS: [&str; 3] = [SECTION, BOX_SECTION, "harvestMapWindow"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct HarvestDoc {
    pub maps: BTreeMap<u32, UserHarvestMap>,
    pub boxes: Vec<UserTreasureBox>,
    pub(crate) window: Option<i64>,
}

impl HarvestDoc {
    pub fn parse(doc: &Map<String, Value>) -> Result<Self, String> {
        let maps: Vec<UserHarvestMap> =
            serde_json::from_value(doc.get(SECTION).cloned().unwrap_or(Value::Array(vec![])))
                .map_err(|e| format!("{SECTION}: {e}"))?;
        let mut out = Self {
            maps: BTreeMap::new(),
            boxes: serde_json::from_value(
                doc.get(BOX_SECTION)
                    .cloned()
                    .unwrap_or(Value::Array(vec![])),
            )
            .map_err(|e| format!("{BOX_SECTION}: {e}"))?,
            window: match doc.get("harvestMapWindow") {
                None | Some(Value::Null) => None,
                Some(value) => Some(value.as_i64().ok_or("harvestMapWindow is not an integer")?),
            },
        };
        for map in maps {
            if out.maps.insert(map.site_id, map).is_some() {
                return Err("duplicate harvest site".into());
            }
        }
        out.check_structure()?;
        Ok(out)
    }
    pub fn write(&self, doc: &mut Map<String, Value>) {
        doc.insert(
            SECTION.into(),
            serde_json::to_value(self.maps.values().collect::<Vec<_>>())
                .expect("harvest map serializes"),
        );
        doc.insert(
            BOX_SECTION.into(),
            serde_json::to_value(&self.boxes).expect("boxes serialize"),
        );
        doc.insert("harvestMapWindow".into(), serde_json::json!(self.window));
    }
    pub fn check_structure(&self) -> Result<(), String> {
        for (&site, map) in &self.maps {
            if site == 0 || map.site_id != site {
                return Err("invalid harvest site".into());
            }
            let mut keys = BTreeSet::new();
            let mut points = BTreeSet::new();
            for row in &map.fixtures {
                if row.fixture_id < 1
                    || row.hp < 0
                    || ![FIXTURE_SPAWNED, FIXTURE_HARVESTED].contains(&row.status)
                    || !keys.insert(key(row))
                    || !points.insert((row.position_x, row.position_z))
                {
                    return Err("invalid or duplicate harvest fixture".into());
                }
            }
            let mut drops = BTreeSet::new();
            for row in &map.drops {
                if row.quantity < 1
                    || row.hp < 0
                    || row.seq < 1
                    || resource_word(row.resource_type).is_none()
                    || ![DROP_BEFORE, DROP_DROPPED].contains(&row.status)
                    || !points.contains(&(row.position_x, row.position_z))
                    || !drops.insert((row.position_x, row.position_z, row.hp, row.seq))
                {
                    return Err("invalid or duplicate harvest drop".into());
                }
            }
        }
        let mut boxes = BTreeSet::new();
        for row in &self.boxes {
            if row.seq < 1
                || !boxes.insert(row.seq)
                || ![BOX_BEFORE_SPAWNED, BOX_SPAWNED].contains(&row.status)
            {
                return Err("invalid treasure box".into());
            }
        }
        Ok(())
    }
}

fn refused(reason: impl Into<String>) -> Refusal {
    Refusal::new("harvest_invalid", reason)
}
fn key(row: &UserFixture) -> FixtureKey {
    FixtureKey {
        fixture_id: row.fixture_id,
        position_x: row.position_x,
        position_z: row.position_z,
    }
}
fn request_key(row: &BulkHarvestFixture) -> FixtureKey {
    FixtureKey {
        fixture_id: row.mysekai_site_harvest_fixture_id,
        position_x: row.position_x,
        position_z: row.position_z,
    }
}
fn budget(s: Stamina) -> Budget {
    Budget {
        normal: s.normal,
        enhance: s.enhance,
        boost: s.boost,
    }
}
fn stamina(b: Budget) -> Stamina {
    Stamina {
        normal: b.normal,
        enhance: b.enhance,
        boost: b.boost,
    }
}
fn read_rests(rows: &[MysekaiStaminaUsed]) -> Result<Budget, Refusal> {
    let mut rests = BTreeMap::new();
    for row in rows {
        if row.rest < 0
            || !["normal", "enhance", "boost"].contains(&row.mysekai_stamina_type.as_str())
            || rests
                .insert(row.mysekai_stamina_type.as_str(), row.rest)
                .is_some()
        {
            return Err(refused("invalid stamina rests"));
        }
    }
    if rests.len() != 3 {
        return Err(refused("all three stamina rests required"));
    }
    Ok(Budget {
        normal: rests["normal"],
        enhance: rests["enhance"],
        boost: rests["boost"],
    })
}

fn master_rows<'a>(masters: &'a Tables<'_>, name: &str) -> Result<&'a [Value], Refusal> {
    masters
        .shared_rows()
        .and_then(|tables| tables.rows.get(name))
        .map(Vec::as_slice)
        .ok_or_else(|| Refusal::new("masters_loading", format!("{name} is absent")))
}
fn bulk_masters(masters: &Tables<'_>) -> Result<BulkHarvestMasters, Refusal> {
    BulkHarvestMasters::parse(
        &masters
            .shared_rows()
            .ok_or_else(|| refused("shared masters absent"))?
            .rows,
    )
    .map_err(|e| Refusal::new("masters_loading", e))
}
fn tool_rows(
    doc: &super::document::ServerDocument,
    masters: &Tables<'_>,
) -> Result<Vec<Tool>, Refusal> {
    let rows = master_rows(masters, "mysekaiTools")?;
    doc.tools
        .iter()
        .flatten()
        .map(|owned| {
            let row = rows
                .iter()
                .find(|row| row["id"].as_i64() == Some(i64::from(owned.mysekai_tool_id)))
                .ok_or_else(|| refused("tool master missing"))?;
            Ok(Tool {
                id: owned.mysekai_tool_id,
                quantity: owned.quantity,
                durability: owned.durability,
                max_durability: super::int32(row, "maxDurability").map_err(refused)?,
                attack_power: super::int32(row, "attackPower").map_err(refused)?,
            })
        })
        .collect()
}

fn seed_inputs(masters: &Tables<'_>) -> Result<seed::MockInputs, Refusal> {
    let fixtures = master_rows(masters, "mysekaiSiteHarvestFixtures")?
        .iter()
        .map(|row| {
            let kind = match row["mysekaiSiteHarvestFixtureType"].as_str().unwrap_or("") {
                "wood" => 0,
                "mineral" => 1,
                "plant" => 2,
                "treasure_box_transport" => 3,
                "treasure_box_fixed" => 4,
                "other" => 5,
                "tone" => 6,
                "toolbox" => 7,
                "driftage" => 8,
                "birthday_plant" => 9,
                _ => -1,
            };
            Ok(seed::MockFixtureRow {
                id: super::int32(row, "id").map_err(refused)?,
                kind,
                hp: super::int32(row, "hp").map_err(refused)?,
            })
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    let materials = masters
        .shared_rows()
        .and_then(|s| s.material_rows.as_ref())
        .ok_or_else(|| Refusal::new("masters_loading", "mysekaiMaterials absent"))?
        .iter()
        .map(|(id, kind, sites)| seed::MockMaterialRow {
            id: *id,
            material_type: if kind == "junk" { 3 } else { -1 },
            site_ids: sites.clone(),
        })
        .collect();
    let mut music_records = BTreeMap::<String, Vec<i64>>::new();
    for row in master_rows(masters, "mysekaiMusicRecords")? {
        if let (Some(kind), Some(id)) = (row["mysekaiMusicTrackType"].as_str(), row["id"].as_i64())
        {
            music_records.entry(kind.into()).or_default().push(id);
        }
    }
    let harvest_sites = masters
        .site_categories
        .as_ref()
        .ok_or_else(|| Refusal::new("masters_loading", "mysekaiSites absent"))?
        .iter()
        .filter(|(_, kind)| kind == "harvest")
        .map(|(id, _)| *id as u32)
        .collect();
    Ok(seed::MockInputs {
        fixtures,
        materials,
        harvest_sites,
        music_records,
        max_normal_stamina: 0,
        boost_grant: 0,
    })
}

/// Request rows must exactly equal authoritative rows, including quantities.
/// Reject duplicates before any mutation; this also prevents replay credit.
fn validate_drops(
    map: &UserHarvestMap,
    sent: &[MysekaiSiteHarvestResourceDrop],
    expected_status: i32,
    positions: Option<&BTreeSet<(i32, i32)>>,
) -> Result<Vec<UserDrop>, Refusal> {
    let mut keys = BTreeSet::new();
    let mut result = Vec::new();
    for request in sent {
        let k = (
            request.position_x,
            request.position_z,
            request.hp,
            request.seq,
        );
        if !keys.insert(k) {
            return Err(refused("duplicate drop"));
        }
        let row = map
            .drops
            .iter()
            .find(|row| (row.position_x, row.position_z, row.hp, row.seq) == k)
            .ok_or_else(|| refused("stale drop"))?;
        if row.status != expected_status
            || MysekaiSiteHarvestResourceDrop::from_row(row).as_ref() != Some(request)
            || positions.is_some_and(|points| !points.contains(&(row.position_x, row.position_z)))
        {
            return Err(refused("forged or unrelated drop"));
        }
        result.push(row.clone());
    }
    if let Some(points) = positions {
        let expected = map
            .drops
            .iter()
            .filter(|row| {
                row.status == DROP_BEFORE && points.contains(&(row.position_x, row.position_z))
            })
            .count();
        if expected != result.len() {
            return Err(refused("bulk request omitted pending drops"));
        }
    }
    Ok(result)
}

impl ServerModel {
    /// Refresh maps only on arrival. A loaded site keeps its map until exit.
    pub(crate) fn harvest_map(&mut self, site: i32, masters: &Tables<'_>) -> Result<(), Refusal> {
        if !masters.site_categories.as_ref().is_some_and(|rows| {
            rows.iter()
                .any(|(id, kind)| *id == site && kind == "harvest")
        }) {
            return Err(refused("not a harvest site"));
        }
        let now = self.now_ms();
        let periods = masters
            .periods
            .as_ref()
            .ok_or_else(|| Refusal::new("masters_loading", "refresh periods absent"))?;
        let window = super::clock::window_start(now, periods, super::clock::device_utc_offset_ms)
            .ok_or_else(|| refused("no current refresh window"))?;
        if self.doc.harvest.window != Some(window) || self.doc.harvest.maps.is_empty() {
            let inputs = seed_inputs(masters)?;
            self.doc.harvest.maps =
                seed::generate_maps(&inputs, &seed::MapRules::builtin(), window);
            self.doc.harvest.window = Some(window);
            // Existing offline transport policy: one once-a-day box on first seating.
            // Map refresh does not silently grant another box.
            if self.doc.harvest.boxes.is_empty() {
                self.doc.harvest.boxes = inputs
                    .harvest_sites
                    .first()
                    .map(|&site_id| UserTreasureBox {
                        refresh_type: "once_a_day".into(),
                        seq: 1,
                        transport_seconds: 10,
                        status: BOX_BEFORE_SPAWNED,
                        site_id,
                        position_x: 0,
                        position_z: 0,
                    })
                    .into_iter()
                    .collect();
            }
            self.commit();
        }
        self.respond(
            ResponseKind::Harvest,
            false,
            &[SECTION, BOX_SECTION, SECTION_STAMINA],
            masters,
        );
        Ok(())
    }

    pub(crate) fn harvest_spawn_box(
        &mut self,
        request: &UserMysekaiTreasureBoxSpawnRequest,
        masters: &Tables<'_>,
    ) -> Result<(), Refusal> {
        let site = u32::try_from(request.mysekai_site_id).map_err(|_| refused("invalid site"))?;
        let index = self
            .doc
            .harvest
            .boxes
            .iter()
            .position(|b| b.seq == request.seq && b.status == BOX_BEFORE_SPAWNED)
            .ok_or_else(|| refused("treasure box already spawned"))?;
        let inputs = seed_inputs(masters)?;
        let fixture_id = inputs
            .fixtures
            .iter()
            .filter(|r| r.kind == 3)
            .map(|r| r.id)
            .min()
            .ok_or_else(|| refused("transport fixture missing"))?;
        let material = inputs
            .materials
            .iter()
            .filter(|r| r.material_type == 3 && r.site_ids.contains(&site))
            .map(|r| r.id)
            .min()
            .ok_or_else(|| refused("transport material missing"))?;
        let map = self
            .doc
            .harvest
            .maps
            .get_mut(&site)
            .ok_or_else(|| refused("harvest map not seated"))?;
        if map
            .fixtures
            .iter()
            .any(|f| f.position_x == request.position_x && f.position_z == request.position_z)
        {
            return Err(refused("treasure position occupied"));
        }
        let seq = map
            .drops
            .iter()
            .map(|r| r.seq)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| refused("drop sequence overflow"))?;
        map.fixtures.push(UserFixture {
            fixture_id,
            position_x: request.position_x,
            position_z: request.position_z,
            hp: 0,
            status: FIXTURE_SPAWNED,
            group_id: 0,
        });
        map.drops.push(UserDrop {
            resource_type: 41,
            resource_id: material,
            position_x: request.position_x,
            position_z: request.position_z,
            hp: 0,
            seq,
            status: DROP_BEFORE,
            quantity: 1,
            group_id: 0,
        });
        let b = &mut self.doc.harvest.boxes[index];
        b.site_id = site;
        b.status = BOX_SPAWNED;
        b.position_x = request.position_x;
        b.position_z = request.position_z;
        self.commit();
        self.respond(
            ResponseKind::Harvest,
            false,
            &[SECTION, BOX_SECTION],
            masters,
        );
        Ok(())
    }

    pub(crate) fn harvest(
        &mut self,
        request: &HarvestRequest,
        masters: &Tables<'_>,
    ) -> Result<Stamina, Refusal> {
        let site = request.mysekai_site_id as u32;
        let target = &request.mysekai_site_harvest_fixture_target;
        let map = self
            .doc
            .harvest
            .maps
            .get(&site)
            .ok_or_else(|| refused("harvest map not seated"))?;
        let at = map
            .fixtures
            .iter()
            .position(|r| {
                r.fixture_id == target.mysekai_site_harvest_fixture_id
                    && r.position_x == target.position_x
                    && r.position_z == target.position_z
                    && r.status == FIXTURE_SPAWNED
            })
            .ok_or_else(|| refused("stale harvest fixture"))?;
        let fixture = &map.fixtures[at];
        if target.hp < 0
            || target.hp > fixture.hp
            || (target.is_last_attack && target.hp != 0)
            || (target.hp == fixture.hp && !target.is_last_attack)
        {
            return Err(refused("invalid harvest hp"));
        }
        let row = master_rows(masters, "mysekaiSiteHarvestFixtures")?
            .iter()
            .find(|row| row["id"].as_i64() == Some(i64::from(fixture.fixture_id)))
            .ok_or_else(|| refused("fixture master missing"))?;
        let last = super::int32(row, "lastAttackStamina").map_err(refused)?;
        let cost = i64::from(fixture.hp - target.hp)
            + if target.is_last_attack {
                i64::from(last)
            } else {
                0
            };
        let before = self
            .doc
            .stamina
            .ok_or_else(|| refused("stamina not seated"))?;
        let after = budget(before)
            .consume(cost)
            .ok_or_else(|| refused("insufficient stamina"))?;
        if read_rests(&request.mysekai_stamina_useds)? != after
            && !self
                .pending
                .iter()
                .any(|section| section == SECTION_STAMINA)
        {
            return Err(refused("stale stamina rests"));
        }
        let required_kind = match row["mysekaiSiteHarvestFixtureType"].as_str() {
            Some("wood") => Some("axe"),
            Some("mineral") => Some("pickaxe"),
            _ => None,
        };
        if required_kind.is_some() != request.mysekai_tool_used.is_some() {
            return Err(refused("harvest tool requirement disagrees"));
        }
        let mut next_tools = self.doc.tools.clone().unwrap_or_default();
        if let Some(used) = &request.mysekai_tool_used {
            let tools = tool_rows(&self.doc, masters)?;
            let tool = tools
                .iter()
                .find(|row| row.id == used.mysekai_tool_id)
                .ok_or_else(|| refused("tool not owned"))?;
            let master_tool = master_rows(masters, "mysekaiTools")?
                .iter()
                .find(|row| row["id"].as_i64() == Some(i64::from(used.mysekai_tool_id)))
                .expect("tool_rows checked");
            if master_tool["mysekaiToolType"].as_str() != required_kind || tool.attack_power <= 0 {
                return Err(refused("wrong harvest tool"));
            }
            let damage = i64::from(fixture.hp - target.hp);
            let expected_hits = if target.hp == 0 {
                (i64::from(fixture.hp) + i64::from(tool.attack_power) - 1)
                    / i64::from(tool.attack_power)
            } else {
                if damage % i64::from(tool.attack_power) != 0 {
                    return Err(refused("harvest damage disagrees with tool"));
                }
                damage / i64::from(tool.attack_power)
            } + i64::from(target.is_last_attack);
            if i64::from(used.use_count) != expected_hits
                || used.use_count < 1
                || i64::from(used.use_count) > tool.remaining_hits()
            {
                return Err(refused("invalid tool hit count or durability"));
            }
            super::tools::use_tool(
                &mut next_tools,
                used.mysekai_tool_id,
                used.use_count,
                tool.max_durability,
            )
            .map_err(refused)?;
        }
        let map = self.doc.harvest.maps.get_mut(&site).expect("validated");
        map.fixtures[at].hp = target.hp;
        if target.is_last_attack {
            map.fixtures[at].status = FIXTURE_HARVESTED;
        }
        for drop in &mut map.drops {
            if drop.position_x == target.position_x
                && drop.position_z == target.position_z
                && drop.status == DROP_BEFORE
                && (target.is_last_attack || drop.hp > target.hp)
            {
                drop.status = DROP_DROPPED;
            }
        }
        self.doc.tools = Some(next_tools);
        if !self
            .pending
            .iter()
            .any(|section| section == SECTION_STAMINA)
        {
            self.doc.stamina = Some(stamina(after));
        }
        let replied = self.doc.stamina.expect("seated");
        self.commit();
        self.respond(
            ResponseKind::Harvest,
            false,
            &[SECTION, SECTION_STAMINA, super::tools::SECTION],
            masters,
        );
        Ok(replied)
    }

    pub(crate) fn gather(
        &mut self,
        request: &UserMysekaiGatherRequest,
        masters: &Tables<'_>,
    ) -> Result<Option<Stamina>, Refusal> {
        let site = request.mysekai_site_id as u32;
        let map = self
            .doc
            .harvest
            .maps
            .get(&site)
            .ok_or_else(|| refused("harvest map not seated"))?;
        let drops = validate_drops(
            map,
            &request.gather_mysekai_site_harvest_resource_drops,
            DROP_DROPPED,
            None,
        )?;
        let before = self.doc.clone();
        if let Err(error) = self.grant_harvest_resources(&drops, masters) {
            self.doc = before;
            return Err(error);
        }
        remove_drops(
            self.doc.harvest.maps.get_mut(&site).expect("validated"),
            &drops,
        );
        self.commit();
        self.respond(ResponseKind::Gather, false, &harvest_sections(), masters);
        Ok(self.doc.stamina)
    }

    pub(crate) fn bulk_harvest(
        &mut self,
        request: &UserMysekaiSiteBulkHarvestRequest,
        masters: &Tables<'_>,
    ) -> Result<UserMysekaiSiteBulkHarvestResponse, Refusal> {
        if !self
            .doc
            .colorful_pass
            .is_some_and(|pass| self.now_ms() < pass.expired_at)
        {
            return Err(Refusal::new(
                "mysekai_colorful_pass_invalid",
                "Colorful Pass is expired",
            ));
        }
        let material_limit = masters
            .possession()
            .material_max_count(self.doc.inventory.material_possession_level)
            .ok_or_else(|| Refusal::new("masters_loading", "material possession limit absent"))?;
        if self
            .material_possession(masters)
            .ok_or_else(|| refused("material types absent"))?
            .quantity
            >= material_limit
        {
            return Err(Refusal::new(
                "harvest_possession_limit",
                "material possession limit reached",
            ));
        }
        let site = request.mysekai_site_id as u32;
        if self
            .doc
            .harvest
            .boxes
            .iter()
            .any(|b| b.status == BOX_BEFORE_SPAWNED && b.site_id == site)
        {
            return Err(Refusal::new(
                "harvest_treasure_not_synced",
                "treasure box not spawned",
            ));
        }
        let map = self
            .doc
            .harvest
            .maps
            .get(&site)
            .ok_or_else(|| refused("harvest map not seated"))?;
        let masters_bulk = bulk_masters(masters)?;
        let candidates = masters_bulk.candidates(map, 0., 0.);
        let mut requested = BTreeSet::new();
        let mut chosen = Vec::new();
        for (order, row) in request
            .target_mysekai_site_harvest_fixtures
            .iter()
            .enumerate()
        {
            let key = request_key(row);
            if !requested.insert(key) {
                return Err(refused("duplicate bulk fixture"));
            }
            let mut candidate = candidates
                .iter()
                .find(|c| c.key == key && c.spawned)
                .cloned()
                .ok_or_else(|| refused("stale or unavailable bulk fixture"))?;
            // Server does not infer the player's unsent position. Validate the
            // client-selected sequence and its exact costs, never reorder it.
            candidate.ordinal = order;
            chosen.push(candidate);
        }
        if chosen.is_empty() {
            return Err(refused("no bulk targets"));
        }
        let ids: Vec<i32> = chosen
            .iter()
            .map(|c| c.target_id)
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });
        if ids != request.mysekai_site_bulk_harvest_target_ids {
            return Err(refused("bulk target IDs disagree with fixtures"));
        }
        let tools = tool_rows(&self.doc, masters)?;
        let before_stamina = self
            .doc
            .stamina
            .ok_or_else(|| refused("stamina not seated"))?;
        if budget(before_stamina).total() < 1 {
            return Err(refused("insufficient stamina"));
        }
        let plan = law::calculate(&chosen, None, &tools, budget(before_stamina));
        if plan.targets.len() != chosen.len() {
            return Err(refused("bulk budget changed before settlement"));
        }
        if read_rests(&request.mysekai_stamina_useds)? != plan.stamina_after {
            return Err(refused("bulk stamina rests disagree"));
        }
        let points = requested
            .iter()
            .map(|key| (key.position_x, key.position_z))
            .collect();
        let drops = validate_drops(
            map,
            &request.gather_mysekai_site_harvest_resource_drops,
            DROP_BEFORE,
            Some(&points),
        )?;
        let mut next_tools = self.doc.tools.clone().unwrap_or_default();
        for (&id, &hits) in &plan.tool_hits {
            let tool = tools.iter().find(|r| r.id == id).expect("validated");
            super::tools::use_tool(
                &mut next_tools,
                id,
                i32::try_from(hits).map_err(|_| refused("tool hit overflow"))?,
                tool.max_durability,
            )
            .map_err(refused)?;
        }
        let before = self.doc.clone();
        let obtained = match self.grant_harvest_resources(&drops, masters) {
            Ok(rows) => rows,
            Err(error) => {
                self.doc = before;
                return Err(error);
            }
        };
        let map = self.doc.harvest.maps.get_mut(&site).expect("validated");
        for fixture in &mut map.fixtures {
            if requested.contains(&key(fixture)) {
                fixture.hp = 0;
                fixture.status = FIXTURE_HARVESTED;
            }
        }
        remove_drops(map, &drops);
        self.doc.tools = Some(next_tools);
        self.doc.stamina = Some(stamina(plan.stamina_after));
        self.commit();
        let sections = harvest_sections();
        self.respond(ResponseKind::Harvest, false, &sections, masters);
        let updated_resources = self.inventory_update(
            &sections.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            masters,
        );
        Ok(UserMysekaiSiteBulkHarvestResponse {
            updated_resources,
            obtained_drop_resources: obtained,
            player_rank_rewards: vec![],
            // Existing offline server policy has no timed break schedule.
            is_in_break_time: false,
        })
    }

    /// Same owned rows and limits as ordinary collect; reject the whole batch
    /// when a row does not fit. Caller rolls back this document on any error.
    fn grant_harvest_resources(
        &mut self,
        drops: &[UserDrop],
        masters: &Tables<'_>,
    ) -> Result<Vec<UserResource>, Refusal> {
        let now = self.now_ms();
        let mut obtained: Vec<UserResource> = Vec::new();
        let material_limit = masters
            .possession()
            .material_max_count(self.doc.inventory.material_possession_level)
            .ok_or_else(|| Refusal::new("masters_loading", "material possession limit absent"))?;
        let fixture_limit = masters
            .possession()
            .fixture_max_count(self.doc.inventory.fixture_possession_level)
            .ok_or_else(|| Refusal::new("masters_loading", "fixture possession limit absent"))?;
        for drop in drops {
            let row = MysekaiSiteHarvestResourceDrop::from_row(drop)
                .ok_or_else(|| refused("unsupported drop"))?;
            let id = row.resource_id;
            let qty = row.quantity;
            match row.resource_type.as_str() {
                "material" => {
                    let entry = self.doc.delivery.materials.entry(id).or_default();
                    *entry = entry
                        .checked_add(qty)
                        .ok_or_else(|| refused("material quantity overflow"))?;
                }
                "mysekai_material" => {
                    let kind = masters
                        .shared_rows()
                        .and_then(|s| s.material_rows.as_ref())
                        .and_then(|rows| rows.iter().find(|r| r.0 == i64::from(id)))
                        .map(|r| r.1.as_str())
                        .ok_or_else(|| refused("material master missing"))?;
                    if !["game_character", "birthday_party"].contains(&kind)
                        && self
                            .material_possession(masters)
                            .ok_or_else(|| refused("material possession missing"))?
                            .quantity
                            .checked_add(qty)
                            .is_none_or(|n| n > material_limit)
                    {
                        return Err(refused("material possession limit"));
                    }
                    let entry = self.doc.delivery.mysekai_materials.entry(id).or_default();
                    *entry = entry
                        .checked_add(qty)
                        .ok_or_else(|| refused("material quantity overflow"))?;
                }
                "mysekai_fixture" => {
                    let total: i64 = self
                        .doc
                        .inventory
                        .fixtures
                        .iter()
                        .map(|r| i64::from(r.quantity))
                        .sum();
                    if total + i64::from(qty) > i64::from(fixture_limit) {
                        return Err(refused("fixture possession limit"));
                    }
                    if let Some(owned) = self
                        .doc
                        .inventory
                        .fixtures
                        .iter_mut()
                        .find(|r| r.mysekai_fixture_id == id && r.texture_id == 1)
                    {
                        owned.quantity = owned
                            .quantity
                            .checked_add(qty)
                            .ok_or_else(|| refused("fixture quantity overflow"))?;
                        owned.last_obtained_at = now;
                    } else {
                        self.doc.inventory.fixtures.push(UserMysekaiFixture {
                            mysekai_fixture_id: id,
                            texture_id: 1,
                            quantity: qty,
                            last_obtained_at: now,
                        });
                    }
                }
                "mysekai_item" => {
                    if let Some(owned) = self
                        .doc
                        .inventory
                        .items
                        .iter_mut()
                        .find(|r| r.mysekai_item_id == id)
                    {
                        owned.quantity = owned
                            .quantity
                            .checked_add(qty)
                            .ok_or_else(|| refused("item quantity overflow"))?;
                        owned.last_obtained_at = now;
                    } else {
                        self.doc.inventory.items.push(UserMysekaiItem {
                            mysekai_item_id: id,
                            quantity: qty,
                            last_obtained_at: now,
                        });
                    }
                }
                "mysekai_blueprint" => {
                    if !self
                        .doc
                        .inventory
                        .blueprints
                        .iter()
                        .any(|r| r.mysekai_blueprint_id == id)
                    {
                        self.doc.inventory.blueprints.push(UserMysekaiBlueprint {
                            mysekai_blueprint_id: id,
                            obtained_at: now,
                        });
                    }
                }
                "mysekai_music_record" => {
                    if !self
                        .doc
                        .music_records
                        .iter()
                        .any(|r| r.mysekai_music_record_id == id)
                    {
                        self.doc.music_records.push(OwnedMusicRecord {
                            mysekai_music_record_id: id,
                            obtained_at: now,
                        });
                    }
                }
                _ => return Err(refused("resource cannot be gathered")),
            }
            if let Some(existing) = obtained
                .iter_mut()
                .find(|r| r.resource_type == row.resource_type && r.resource_id == id)
            {
                existing.quantity = existing
                    .quantity
                    .checked_add(qty)
                    .ok_or_else(|| refused("reward quantity overflow"))?;
            } else {
                obtained.push(UserResource {
                    resource_type: row.resource_type,
                    resource_id: id,
                    resource_level: 1,
                    quantity: qty,
                });
            }
        }
        Ok(obtained)
    }
}

fn remove_drops(map: &mut UserHarvestMap, drops: &[UserDrop]) {
    map.drops
        .retain(|row| !drops.iter().any(|drop| drop == row));
}
fn harvest_sections() -> Vec<&'static str> {
    vec![
        SECTION,
        BOX_SECTION,
        SECTION_STAMINA,
        SECTION_GAMEDATA,
        super::tools::SECTION,
        super::delivery::SECTION_MYSEKAI_MATERIALS,
        super::delivery::SECTION_MATERIALS,
        super::inventory::SECTION_FIXTURES,
        super::inventory::SECTION_ITEMS,
        super::inventory::SECTION_BLUEPRINTS,
        super::music_play::RECORDS_SECTION,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_masters::inventory::{PossessionMasters, PossessionRow};
    use moly_masters::load::MasterTables;
    use serde_json::json;

    fn tables() -> (crate::masters::Masters, MasterTables) {
        let mut m = crate::masters::Masters::fixture();
        m.site_categories = Some(vec![(5, "harvest".into())]);
        let mut s = MasterTables::fixture();
        s.rows.insert(
            "mysekaiSiteBulkHarvestTargetGroups",
            vec![
                json!({"id":1,"seq":1,"name":"tree","requiredToolId":10}),
                json!({"id":2,"seq":2,"name":"plant"}),
            ],
        );
        s.rows.insert(
            "mysekaiSiteBulkHarvestTargets",
            vec![
                json!({"id":1,"seq":1,"name":"tree","mysekaiSiteBulkHarvestTargetGroupId":1}),
                json!({"id":2,"seq":2,"name":"plant","mysekaiSiteBulkHarvestTargetGroupId":2}),
            ],
        );
        s.rows.insert(
            "mysekaiSiteBulkHarvests",
            vec![
                json!({"id":1,"mysekaiSiteId":5,"mysekaiSiteBulkHarvestTargetId":1}),
                json!({"id":2,"mysekaiSiteId":5,"mysekaiSiteBulkHarvestTargetId":2}),
            ],
        );
        s.rows.insert("mysekaiSiteHarvestFixtures",vec![json!({"id":1,"hp":80,"lastAttackStamina":10,"mysekaiSiteHarvestFixtureType":"wood","mysekaiSiteBulkHarvestTargetId":1}),json!({"id":2,"hp":0,"lastAttackStamina":10,"mysekaiSiteHarvestFixtureType":"plant","mysekaiSiteBulkHarvestTargetId":2})]);
        s.rows.insert(
            "mysekaiTools",
            vec![json!({"id":10,"attackPower":40,"maxDurability":400,"mysekaiToolType":"axe"})],
        );
        s.material_rows = Some(vec![(1, "wood".into(), vec![5])]);
        s.craft.as_mut().unwrap().material_types = Some([(1, "wood".into())].into_iter().collect());
        s.possession = Some(PossessionMasters {
            material: Some(vec![PossessionRow {
                level: 1,
                possession_limit: 100,
            }]),
            fixture: Some(vec![PossessionRow {
                level: 1,
                possession_limit: 100,
            }]),
        });
        (m, s)
    }
    fn model() -> ServerModel {
        let doc = super::super::document::migrate_v1(
            crate::defaults::CHECKED_IN_SLICE,
            super::super::document::Migration::CheckedInDefault,
        )
        .unwrap();
        let mut model = ServerModel::new(doc, super::super::Origin::CheckedInDefault, vec![]);
        model.doc.clock = super::super::document::Clock::Fixed { at_ms: 1000 };
        model.doc.colorful_pass = Some(ColorfulPass {
            id: 1,
            expired_at: 2000,
        });
        model.doc.stamina = Some(Stamina {
            normal: 100,
            enhance: 30,
            boost: 20,
        });
        model.doc.tools = Some(vec![UserMysekaiTool {
            mysekai_tool_id: 10,
            quantity: 1,
            durability: 10,
            last_obtained_at: 0,
        }]);
        model.doc.harvest.maps.insert(
            5,
            UserHarvestMap {
                site_id: 5,
                fixtures: vec![
                    UserFixture {
                        fixture_id: 1,
                        position_x: 1,
                        position_z: 0,
                        hp: 80,
                        status: FIXTURE_SPAWNED,
                        group_id: 0,
                    },
                    UserFixture {
                        fixture_id: 2,
                        position_x: 2,
                        position_z: 0,
                        hp: 0,
                        status: FIXTURE_SPAWNED,
                        group_id: 0,
                    },
                ],
                drops: vec![
                    drop(1, 61, 1, DROP_BEFORE),
                    drop(1, 0, 2, DROP_BEFORE),
                    drop(2, 0, 3, DROP_BEFORE),
                ],
            },
        );
        model
    }
    fn drop(x: i32, hp: i32, seq: i32, status: i32) -> UserDrop {
        UserDrop {
            resource_type: 41,
            resource_id: 1,
            position_x: x,
            position_z: 0,
            hp,
            seq,
            status,
            quantity: 1,
            group_id: 0,
        }
    }
    fn rests(b: Budget) -> Vec<MysekaiStaminaUsed> {
        [
            ("normal", b.normal),
            ("enhance", b.enhance),
            ("boost", b.boost),
        ]
        .into_iter()
        .map(|(kind, rest)| MysekaiStaminaUsed {
            mysekai_stamina_type: kind.into(),
            rest,
        })
        .collect()
    }
    fn request(
        model: &ServerModel,
        masters: &Tables<'_>,
        checked: Option<&[i32]>,
    ) -> UserMysekaiSiteBulkHarvestRequest {
        let map = &model.doc.harvest.maps[&5];
        let bm = bulk_masters(masters).unwrap();
        let plan = law::calculate(
            &bm.candidates(map, 0., 0.),
            checked,
            &tool_rows(&model.doc, masters).unwrap(),
            budget(model.doc.stamina.unwrap()),
        );
        let targets = plan
            .targets
            .iter()
            .map(|key| BulkHarvestFixture {
                mysekai_site_harvest_fixture_id: key.fixture_id,
                position_x: key.position_x,
                position_z: key.position_z,
            })
            .collect();
        let drops =
            map.drops
                .iter()
                .filter(|row| {
                    row.status == DROP_BEFORE
                        && plan.targets.iter().any(|k| {
                            k.position_x == row.position_x && k.position_z == row.position_z
                        })
                })
                .map(|row| MysekaiSiteHarvestResourceDrop::from_row(row).unwrap())
                .collect();
        UserMysekaiSiteBulkHarvestRequest {
            mysekai_site_id: 5,
            mysekai_site_bulk_harvest_target_ids: plan.target_ids,
            target_mysekai_site_harvest_fixtures: targets,
            gather_mysekai_site_harvest_resource_drops: drops,
            mysekai_stamina_useds: rests(plan.stamina_after),
        }
    }
    #[test]
    fn normal_hit_gather_then_bulk_share_map_and_no_duplicate_rewards() {
        let (m, s) = tables();
        let tables = Tables::new(&m, Some(&s));
        let mut model = model();
        let after = budget(model.doc.stamina.unwrap()).consume(40).unwrap();
        model
            .harvest(
                &HarvestRequest {
                    mysekai_site_id: 5,
                    mysekai_site_harvest_fixture_target: MysekaiSiteHarvestFixtureTarget {
                        position_x: 1,
                        position_z: 0,
                        mysekai_site_harvest_fixture_id: 1,
                        hp: 40,
                        is_last_attack: false,
                    },
                    mysekai_tool_used: Some(MysekaiToolUsed {
                        mysekai_tool_id: 10,
                        use_count: 1,
                        durability: 9,
                    }),
                    mysekai_stamina_useds: rests(after),
                },
                &tables,
            )
            .unwrap();
        let gathered =
            MysekaiSiteHarvestResourceDrop::from_row(&model.doc.harvest.maps[&5].drops[0]).unwrap();
        model
            .gather(
                &UserMysekaiGatherRequest {
                    mysekai_site_id: 5,
                    gather_mysekai_site_harvest_resource_drops: vec![gathered],
                },
                &tables,
            )
            .unwrap();
        assert_eq!(model.doc.delivery.mysekai_materials[&1], 1);
        let request = request(&model, &tables, Some(&[1]));
        let reply = model.bulk_harvest(&request, &tables).unwrap();
        assert_eq!(reply.obtained_drop_resources[0].quantity, 1);
        assert_eq!(model.doc.delivery.mysekai_materials[&1], 2);
        assert_eq!(model.doc.tools.as_ref().unwrap()[0].durability, 7);
        assert_eq!(
            model.doc.stamina,
            Some(Stamina {
                normal: 60,
                enhance: 0,
                boost: 0
            })
        );
        assert_eq!(
            model.doc.harvest.maps[&5].fixtures[1].status,
            FIXTURE_SPAWNED
        );
        assert_eq!(model.doc.harvest.maps[&5].drops.len(), 1);
        let before = model.doc.clone();
        assert!(model.bulk_harvest(&request, &tables).is_err());
        assert_eq!(model.doc, before);
        let text = model.doc.to_value(&[]).to_string();
        let (reloaded, _) = super::super::document::parse_v2(&text).unwrap();
        assert_eq!(reloaded.harvest, model.doc.harvest);
    }
    #[test]
    fn forged_duplicate_stale_or_missing_drop_is_atomic() {
        let (m, s) = tables();
        let tables = Tables::new(&m, Some(&s));
        for variant in 0..4 {
            let mut model = model();
            let mut req = request(&model, &tables, None);
            match variant {
                0 => req.gather_mysekai_site_harvest_resource_drops[0].quantity = 100,
                1 => req
                    .gather_mysekai_site_harvest_resource_drops
                    .push(req.gather_mysekai_site_harvest_resource_drops[0].clone()),
                2 => req
                    .target_mysekai_site_harvest_fixtures
                    .push(req.target_mysekai_site_harvest_fixtures[0]),
                _ => {
                    req.gather_mysekai_site_harvest_resource_drops.pop();
                }
            }
            let before = model.doc.clone();
            assert!(model.bulk_harvest(&req, &tables).is_err());
            assert_eq!(model.doc, before);
        }
    }
    #[test]
    fn unrelated_fallen_drops_stay_and_full_inventory_rolls_back() {
        let (m, mut s) = tables();
        let tables = Tables::new(&m, Some(&s));
        let mut model = model();
        model.doc.harvest.maps.get_mut(&5).unwrap().drops[0].status = DROP_DROPPED;
        let req = request(&model, &tables, Some(&[1]));
        model.bulk_harvest(&req, &tables).unwrap();
        assert!(model.doc.harvest.maps[&5]
            .drops
            .iter()
            .any(|r| r.seq == 1 && r.status == DROP_DROPPED));
        s.possession.as_mut().unwrap().material.as_mut().unwrap()[0].possession_limit = 1;
        let tables = Tables::new(&m, Some(&s));
        let mut model = self::model();
        let req = request(&model, &tables, None);
        let before = model.doc.clone();
        assert!(model.bulk_harvest(&req, &tables).is_err());
        assert_eq!(model.doc, before);
    }
    #[test]
    fn pass_expiry_and_treasure_block_before_debit() {
        let (m, s) = tables();
        let tables = Tables::new(&m, Some(&s));
        let mut model = model();
        let req = request(&model, &tables, None);
        model.doc.colorful_pass.as_mut().unwrap().expired_at = 1000;
        let before = model.doc.clone();
        assert_eq!(
            model.bulk_harvest(&req, &tables).unwrap_err().code,
            "mysekai_colorful_pass_invalid"
        );
        assert_eq!(model.doc, before);
        model.doc.colorful_pass.as_mut().unwrap().expired_at = 2000;
        model.doc.harvest.boxes.push(UserTreasureBox {
            refresh_type: "once_a_day".into(),
            seq: 1,
            transport_seconds: 10,
            status: BOX_BEFORE_SPAWNED,
            site_id: 5,
            position_x: 0,
            position_z: 0,
        });
        let before = model.doc.clone();
        assert_eq!(
            model.bulk_harvest(&req, &tables).unwrap_err().code,
            "harvest_treasure_not_synced"
        );
        assert_eq!(model.doc, before);
    }
    #[test]
    fn full_fixture_inventory_does_not_gate_unrelated_material_bulk() {
        let (m,mut s)=tables();
        s.possession.as_mut().unwrap().fixture.as_mut().unwrap()[0].possession_limit=1;
        let tables=Tables::new(&m,Some(&s));
        let mut model=model();
        model.doc.inventory.fixtures.push(UserMysekaiFixture{mysekai_fixture_id:7,texture_id:1,quantity:1,last_obtained_at:0});
        let req=request(&model,&tables,None);
        assert_eq!(model.bulk_harvest(&req,&tables).unwrap().obtained_drop_resources[0].quantity,3);
        assert_eq!(model.doc.inventory.fixtures[0].quantity,1);
    }

    #[test]
    fn missing_possession_material_types_refuse_without_any_debit() {
        let (m,mut s)=tables();
        s.craft.as_mut().unwrap().material_types=None;
        let tables=Tables::new(&m,Some(&s));
        let mut model=model();let req=request(&model,&tables,None);let before=model.doc.clone();
        assert!(model.bulk_harvest(&req,&tables).is_err());
        assert_eq!(model.doc,before);
    }

    #[test]
    fn invalid_refresh_window_cannot_silently_reset_the_harvest_map() {
        for value in [serde_json::json!("broken"),serde_json::json!(1.5),serde_json::json!(true)] {
            let document=serde_json::json!({"harvestMapWindow":value});
            assert!(HarvestDoc::parse(document.as_object().unwrap()).is_err());
        }
        assert!(HarvestDoc::parse(serde_json::json!({}).as_object().unwrap()).is_ok());
    }

}
