//! The inventory screen's recycle and disassemble requests: the fixture
//! disassemble, the material recycle and the tool recycle.
//!
//! Each request names one owned row and the quantity the player gave up
//! (`mysekaiFixtureId` and `textureId`, `mysekaiMaterialId`, or
//! `mysekaiToolId`, and `quantity`). The reply spends the quantity from the
//! row, drops a row that reaches 0 and returns the resources the row gives
//! back; it carries the owned tables the request changed (the fixtures, the
//! tools, or the materials with their possession). The owned blueprint count
//! that the source's fixture disassemble also spends is not changed here.
//!
//! What a row gives back is named in the document while the amounts the
//! source reads from its own masters are not carried:
//!
//! - `policies.fixturesDisassembleMaterials`: the resources a disassembled
//!   fixture gives back, per fixture (none by default).
//! - `policies.materialsRecycleRewards`: the resources a recycled material
//!   gives back, per material (one coin by default).
//! - `policies.toolsRecycleRewards`: the resources a recycled tool gives
//!   back, per tool (one coin by default).
//!
//! The reply is a `SuiteUser` reply, as the source's three responses are:
//! its `updatedResources` are the tables above, already in the game's copies
//! when the answer is read.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy::prelude::*;
use moly_protocol::api::MysekaiInventoryDisassemble;
use moly_protocol::rows::{SuiteUserSections, UserResource};
use serde_json::{Map, Value};

use super::delivery::SECTION_MYSEKAI_MATERIALS;
use super::{ResponseKind, ServerModel, Tables};

/// `policies.fixturesDisassembleMaterials`.
pub(crate) const POLICY_FIXTURES: &str = "fixturesDisassembleMaterials";
/// `policies.materialsRecycleRewards`.
pub(crate) const POLICY_MATERIALS: &str = "materialsRecycleRewards";
/// `policies.toolsRecycleRewards`.
pub(crate) const POLICY_TOOLS: &str = "toolsRecycleRewards";
/// The policy keys this part reads and writes.
pub(crate) const POLICY_KEYS: [&str; 3] = [POLICY_FIXTURES, POLICY_MATERIALS, POLICY_TOOLS];

/// The default material and tool reward: one coin per recycled row.
fn coin_reward() -> Vec<UserResource> {
    vec![UserResource {
        resource_type: "coin".to_owned(),
        resource_id: 0,
        resource_level: 0,
        quantity: 1,
    }]
}

/// The rewards the document states under `policies`, by key; an absent key
/// is left out.
pub(crate) fn parse_policies(
    policies: &Map<String, Value>,
) -> Result<BTreeMap<String, Vec<UserResource>>, String> {
    let mut out = BTreeMap::new();
    for key in POLICY_KEYS {
        let Some(value) = policies.get(key) else {
            continue;
        };
        out.insert(key.to_owned(), parse_rewards(value, key)?);
    }
    Ok(out)
}

/// One policy's resource rows; each quantity is the reward of one unit.
fn parse_rewards(value: &Value, key: &str) -> Result<Vec<UserResource>, String> {
    let at = format!("policies.{key}");
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{at} is not an array of resources"))?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let row_at = format!("{at}[{i}]");
            let row = moly_protocol::json::object(row, &row_at)?;
            moly_protocol::json::only(
                row,
                &["resourceType", "resourceId", "resourceLevel", "quantity"],
                &row_at,
            )?;
            let kind = row
                .get("resourceType")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{row_at}.resourceType is not a string"))?
                .to_owned();
            let reward = UserResource {
                resource_type: kind,
                resource_id: moly_protocol::json::int32(row, "resourceId", &row_at)?,
                resource_level: moly_protocol::json::int32(row, "resourceLevel", &row_at)?,
                quantity: moly_protocol::json::int32(row, "quantity", &row_at)?,
            };
            if reward.quantity < 1 {
                return Err(format!(
                    "{row_at}.quantity = {} is below 1",
                    reward.quantity
                ));
            }
            Ok(reward)
        })
        .collect()
}

/// Writes the stated policies back into the document (the keys it omits
/// keep their defaults).
pub(crate) fn write_policies(
    recycle: &BTreeMap<String, Vec<UserResource>>,
    policies: &mut Map<String, Value>,
) {
    for (key, rows) in recycle {
        policies.insert(key.clone(), moly_protocol::json::resources_value(rows));
    }
}

/// A panel edit of one policy key (`None`: not this part's path); the
/// request the server answers next carries the change.
pub(crate) fn edit_policy(
    recycle: &mut BTreeMap<String, Vec<UserResource>>,
    parts: &[&str],
    value: &Value,
) -> Option<Result<bool, String>> {
    match parts {
        ["policies", key] if POLICY_KEYS.contains(key) => {
            Some(parse_rewards(value, key).map(|rows| {
                recycle.insert((*key).to_owned(), rows);
                true
            }))
        }
        _ => None,
    }
}

/// The panel's section for the stated policies.
pub(crate) fn schema_sections() -> Value {
    let rows = |key: &str, note: &str| {
        serde_json::json!({
            "path": format!("policies.{key}"),
            "type": "rows",
            "row": {"resourceType": "text", "resourceId": "int", "resourceLevel": "int", "quantity": "int"},
            "note": note,
        })
    };
    serde_json::json!([{
        "key": "mysekaiInventoryRecycle",
        "title": "Inventory recycle and disassemble rewards",
        "delivery": "next-response",
        "fields": [
            rows(POLICY_FIXTURES, "the resources a disassembled fixture gives back, each quantity counted per fixture; none by default"),
            rows(POLICY_MATERIALS, "the resources a recycled material gives back, each quantity counted per material; one coin by default"),
            rows(POLICY_TOOLS, "the resources a recycled tool gives back, each quantity counted per tool; one coin by default"),
        ],
    }])
}

/// The panel's named policies.
pub(crate) fn schema_policies() -> Value {
    serde_json::json!([
        {"name": "inventory recycle", "rule": "PostUserMysekaiDisassembleFixtureApi, PostUserMysekaiRecycleMaterialApi and PostUserMysekaiRecycleToolApi spend the stated quantity of the owned row (a row at 0 leaves its list) and the reply carries the changed tables; the rewards are the policies.fixturesDisassembleMaterials, policies.materialsRecycleRewards and policies.toolsRecycleRewards rows, one coin per recycled material or tool by default; the source also spends an owned blueprint for a disassembled fixture, which this server does not"},
    ])
}

/// The rewards of `rows`, each quantity scaled by `count`.
fn scaled(rows: &[UserResource], count: i32) -> Vec<UserResource> {
    rows.iter()
        .map(|row| UserResource {
            quantity: row.quantity.wrapping_mul(count),
            ..row.clone()
        })
        .collect()
}

/// `quantity` of `have` given up, at least `least`; refused by name when
/// the request asks for more than the row holds.
fn spend(quantity: i32, least: i32, have: i32, what: &str) -> Result<(), String> {
    if quantity < least {
        return Err(format!("{what}: quantity {quantity} is below {least}"));
    }
    if quantity > have {
        return Err(format!(
            "{what}: quantity {quantity} is above the owned {have}"
        ));
    }
    Ok(())
}

/// The response kind a reply of `request` is recorded as.
fn response_kind(request: &MysekaiInventoryDisassemble) -> ResponseKind {
    match request {
        MysekaiInventoryDisassemble::Fixture(_) => ResponseKind::FixtureDisassemble,
        MysekaiInventoryDisassemble::Material(_) => ResponseKind::MaterialRecycle,
        MysekaiInventoryDisassemble::Tool(_) => ResponseKind::ToolRecycle,
    }
}

impl ServerModel {
    /// One recycle or disassemble request: spends the row and records the
    /// reply with the tables it changed. `Err` refuses without a change.
    pub(super) fn recycle_reply(
        &mut self,
        request: MysekaiInventoryDisassemble,
        masters: &Tables<'_>,
    ) -> Result<(Vec<UserResource>, SuiteUserSections), String> {
        let kind = response_kind(&request);
        let (rewards, carries): (Vec<UserResource>, &[&str]) = match request {
            MysekaiInventoryDisassemble::Fixture(row) => (
                self.disassemble_fixture(row.mysekai_fixture_id, row.texture_id, row.quantity)?,
                &[super::inventory::SECTION_FIXTURES],
            ),
            MysekaiInventoryDisassemble::Material(row) => (
                self.recycle_material(row.mysekai_material_id, row.quantity, masters)?,
                &[SECTION_MYSEKAI_MATERIALS],
            ),
            MysekaiInventoryDisassemble::Tool(row) => (
                self.recycle_tool(row.mysekai_tool_id, row.quantity, masters)?,
                &[super::tools::SECTION],
            ),
        };
        self.respond(kind, false, carries, masters);
        let updated = self
            .responses
            .last()
            .expect("the response just recorded")
            .sections
            .inventory
            .clone();
        info!(
            "[server] {}: {:?} spent; {} reward row(s)",
            request.api_name(),
            request,
            rewards.len()
        );
        Ok((rewards, updated))
    }

    /// The fixture disassemble: the row of the fixture and texture loses the
    /// quantity and leaves the list when it reaches 0.
    fn disassemble_fixture(
        &mut self,
        mysekai_fixture_id: i32,
        texture_id: i32,
        quantity: i32,
    ) -> Result<Vec<UserResource>, String> {
        let what = format!("fixture {mysekai_fixture_id} texture {texture_id}");
        let have = self
            .doc
            .inventory
            .fixtures
            .iter()
            .find(|row| {
                row.mysekai_fixture_id == mysekai_fixture_id && row.texture_id == texture_id
            })
            .map(|row| row.quantity)
            .ok_or_else(|| format!("userMysekaiFixtures has no row of {what}"))?;
        spend(quantity, 1, have, &what)?;
        let rows = &mut self.doc.inventory.fixtures;
        if have == quantity {
            rows.retain(|row| {
                !(row.mysekai_fixture_id == mysekai_fixture_id && row.texture_id == texture_id)
            });
        } else {
            for row in rows.iter_mut() {
                if row.mysekai_fixture_id == mysekai_fixture_id && row.texture_id == texture_id {
                    row.quantity = have - quantity;
                }
            }
        }
        let rewards = scaled(self.rewards(POLICY_FIXTURES), quantity);
        self.commit();
        Ok(rewards)
    }

    /// The material recycle: the material row loses the quantity and leaves
    /// the materials when it reaches 0.
    fn recycle_material(
        &mut self,
        mysekai_material_id: i32,
        quantity: i32,
        masters: &Tables<'_>,
    ) -> Result<Vec<UserResource>, String> {
        if let Some(types) = masters.craft().material_types.as_ref() {
            if !types.contains_key(&mysekai_material_id) {
                return Err(format!(
                    "material {mysekai_material_id} is not a master material"
                ));
            }
        }
        let what = format!("material {mysekai_material_id}");
        let have = self
            .doc
            .delivery
            .mysekai_materials
            .get(&mysekai_material_id)
            .copied()
            .unwrap_or(0);
        spend(quantity, 1, have, &what)?;
        let left = have - quantity;
        let materials = &mut self.doc.delivery.mysekai_materials;
        if left == 0 {
            materials.remove(&mysekai_material_id);
        } else {
            materials.insert(mysekai_material_id, left);
        }
        let rewards = scaled(self.rewards(POLICY_MATERIALS), quantity);
        self.commit();
        Ok(rewards)
    }

    /// The tool recycle: the tool row loses the quantity and leaves the
    /// tools when it reaches 0.
    fn recycle_tool(
        &mut self,
        mysekai_tool_id: i32,
        quantity: i32,
        masters: &Tables<'_>,
    ) -> Result<Vec<UserResource>, String> {
        if let Some(tools) = &masters.tools {
            if !tools.iter().any(|row| row.id == mysekai_tool_id) {
                return Err(format!("tool {mysekai_tool_id} is not a master tool"));
            }
        }
        let what = format!("tool {mysekai_tool_id}");
        let rows = self
            .doc
            .tools
            .as_ref()
            .ok_or("userMysekaiTools is not seated")?;
        let have = rows
            .iter()
            .find(|row| row.mysekai_tool_id == mysekai_tool_id)
            .map(|row| row.quantity)
            .ok_or_else(|| format!("userMysekaiTools has no row of {what}"))?;
        spend(quantity, 1, have, &what)?;
        let rows = self.doc.tools.as_mut().expect("checked above");
        if have == quantity {
            rows.retain(|row| row.mysekai_tool_id != mysekai_tool_id);
        } else {
            for row in rows.iter_mut() {
                if row.mysekai_tool_id == mysekai_tool_id {
                    row.quantity = have - quantity;
                }
            }
        }
        let rewards = scaled(self.rewards(POLICY_TOOLS), quantity);
        self.commit();
        Ok(rewards)
    }

    /// The document's rewards for a key, or the key's named default.
    fn rewards(&self, key: &str) -> &[UserResource] {
        match self.doc.recycle.get(key) {
            Some(rows) => rows,
            None => match key {
                POLICY_MATERIALS | POLICY_TOOLS => default_coin(),
                _ => &[],
            },
        }
    }
}

/// The default material and tool reward.
static DEFAULT_COIN: OnceLock<Vec<UserResource>> = OnceLock::new();

fn default_coin() -> &'static [UserResource] {
    DEFAULT_COIN.get_or_init(coin_reward)
}
