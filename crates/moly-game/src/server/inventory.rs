//! The user's owned MySekai tables in the server document, and the derived
//! material possession the responses carry with the materials.
//!
//! **Document keys** (the source's `SuiteUser` keys; each optional on read,
//! always written):
//! - `userMysekaiFixtures`: `UserMysekaiFixture` rows (`mysekaiFixtureId`,
//!   `textureId`, `quantity`, `lastObtainedAt`), one per fixture and texture.
//! - `userMysekaiCanvases`: `UserMysekaiCanvas` rows (`mysekaiFixtureId`,
//!   `cardId`, `isSpecialTraining`, `quantity`, `lastObtainedAt`).
//! - `userMysekaiBlueprints`: `UserMysekaiBlueprint` rows
//!   (`mysekaiBlueprintId`, `obtainedAt`); read when `policies.ownedBlueprints`
//!   is `stated`.
//! - `userMysekaiItems`: `UserMysekaiItem` rows (`mysekaiItemId`, `quantity`,
//!   `lastObtainedAt`).
//! - `userMysekaiGamedata.mysekaiMaterialPossessionLevel` and
//!   `.mysekaiFixturePossessionLevel`: the two possession levels.
//!
//! `userMysekaiMaterials` is the birthday-party delivery's section; this part
//! reads and changes the same rows.
//!
//! **Named defaults**: no fixture, canvas or item row; both possession
//! levels [`DEFAULT_POSSESSION_LEVEL`] (the first master level); the owned
//! blueprints policy [`OWNED_EVERY_BLUEPRINT`].
//!
//! **Named policies** (the server's rules are not on disk):
//! - *Owned blueprints*: `everyBlueprint` (the user owns every master
//!   blueprint; the `userMysekaiBlueprints` rows keep their `obtainedAt`, the
//!   rest carry 0) or `stated` (the rows). The craft list shows a blueprint
//!   only when it is available without possession or a row names it
//!   (`UserResourceFactory`), so the craft reply checks the same.
//! - *Material possession* (an inference, not a read):
//!   `userMysekaiMaterialPossession.quantity` is the sum of the
//!   `userMysekaiMaterials` quantities except the `game_character` and
//!   `birthday_party` types, the two types the client lets a player receive
//!   over the possession limit (`MysekaiUserDataUtility.IsMaterialReceivableOverPossessionLimit`).
//!   The server derives it whenever a response carries the materials; the
//!   document does not store it.
//!
//! Native instrument: `MOLY_CRAFT_MOCK_MATERIALS` (`id:quantity`, comma
//! separated) sets `userMysekaiMaterials` rows in the native document; game
//! mode reads no environment variable.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::client::craft::MasterBlueprint;
use super::client::inventory::{
    blueprints_value, canvases_value, fixtures_value, items_value, ClientMysekaiInventory,
    PossessionMasters, PossessionRow, SuiteUserSections, UserMysekaiBlueprint, UserMysekaiCanvas,
    UserMysekaiFixture, UserMysekaiGamedata, UserMysekaiItem, UserMysekaiMaterial,
    UserMysekaiMaterialPossession,
};
use super::delivery::SECTION_MYSEKAI_MATERIALS;
use super::document::{boolean, int32, int64, object, only};
use super::{Masters, ServerModel, SECTION_GAMEDATA};

pub(crate) const SECTION_FIXTURES: &str = "userMysekaiFixtures";
pub(crate) const SECTION_CANVASES: &str = "userMysekaiCanvases";
pub(crate) const SECTION_BLUEPRINTS: &str = "userMysekaiBlueprints";
pub(crate) const SECTION_ITEMS: &str = "userMysekaiItems";
/// The sections of this part a response carries (the document keys).
pub(crate) const SECTIONS: [&str; 4] = [
    SECTION_FIXTURES,
    SECTION_CANVASES,
    SECTION_BLUEPRINTS,
    SECTION_ITEMS,
];
/// The document keys this part reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 4] = SECTIONS;
/// The keys this part reads under `userMysekaiGamedata`.
pub(crate) const GAMEDATA_KEYS: [&str; 2] = [
    "mysekaiMaterialPossessionLevel",
    "mysekaiFixturePossessionLevel",
];
/// The keys this part reads under `policies`.
pub(crate) const POLICY_KEYS: [&str; 1] = ["ownedBlueprints"];

pub(crate) const DEFAULT_POSSESSION_LEVEL: i32 = 1;
pub(crate) const OWNED_EVERY_BLUEPRINT: &str = "everyBlueprint";
pub(crate) const OWNED_STATED: &str = "stated";

/// `MysekaiMaterialType` names the client lets a player receive over the
/// possession limit (`IsMaterialReceivableOverPossessionLimit`).
pub(crate) const OVER_LIMIT_MATERIAL_TYPES: [&str; 2] = ["game_character", "birthday_party"];

/// `policies.ownedBlueprints`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedBlueprints {
    EveryBlueprint,
    Stated,
}

/// This part of the server document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InventoryDoc {
    pub(crate) fixtures: Vec<UserMysekaiFixture>,
    pub(crate) canvases: Vec<UserMysekaiCanvas>,
    pub(crate) blueprints: Vec<UserMysekaiBlueprint>,
    pub(crate) items: Vec<UserMysekaiItem>,
    pub(crate) material_possession_level: i32,
    pub(crate) fixture_possession_level: i32,
    pub(crate) owned_blueprints: OwnedBlueprints,
}

impl Default for InventoryDoc {
    fn default() -> Self {
        Self {
            fixtures: Vec::new(),
            canvases: Vec::new(),
            blueprints: Vec::new(),
            items: Vec::new(),
            material_possession_level: DEFAULT_POSSESSION_LEVEL,
            fixture_possession_level: DEFAULT_POSSESSION_LEVEL,
            owned_blueprints: OwnedBlueprints::EveryBlueprint,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

fn rows<'a>(value: &'a Value, at: &str) -> Result<&'a Vec<Value>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{at} is not an array"))
}

pub(crate) fn parse_fixtures(value: &Value) -> Result<Vec<UserMysekaiFixture>, String> {
    rows(value, SECTION_FIXTURES)?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_FIXTURES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiFixtureId",
                    "textureId",
                    "quantity",
                    "lastObtainedAt",
                ],
                &at,
            )?;
            Ok(UserMysekaiFixture {
                mysekai_fixture_id: int32(row, "mysekaiFixtureId", &at)?,
                texture_id: int32(row, "textureId", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

pub(crate) fn parse_canvases(value: &Value) -> Result<Vec<UserMysekaiCanvas>, String> {
    rows(value, SECTION_CANVASES)?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_CANVASES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiFixtureId",
                    "cardId",
                    "isSpecialTraining",
                    "quantity",
                    "lastObtainedAt",
                ],
                &at,
            )?;
            Ok(UserMysekaiCanvas {
                mysekai_fixture_id: int32(row, "mysekaiFixtureId", &at)?,
                card_id: int32(row, "cardId", &at)?,
                is_special_training: boolean(row, "isSpecialTraining", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

pub(crate) fn parse_blueprints(value: &Value) -> Result<Vec<UserMysekaiBlueprint>, String> {
    rows(value, SECTION_BLUEPRINTS)?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_BLUEPRINTS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiBlueprintId", "obtainedAt"], &at)?;
            Ok(UserMysekaiBlueprint {
                mysekai_blueprint_id: int32(row, "mysekaiBlueprintId", &at)?,
                obtained_at: int64(row, "obtainedAt", &at)?,
            })
        })
        .collect()
}

pub(crate) fn parse_items(value: &Value) -> Result<Vec<UserMysekaiItem>, String> {
    rows(value, SECTION_ITEMS)?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_ITEMS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiItemId", "quantity", "lastObtainedAt"], &at)?;
            Ok(UserMysekaiItem {
                mysekai_item_id: int32(row, "mysekaiItemId", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

pub(crate) fn parse_owned_blueprints(value: &Value) -> Result<OwnedBlueprints, String> {
    match value.as_str() {
        Some(OWNED_EVERY_BLUEPRINT) => Ok(OwnedBlueprints::EveryBlueprint),
        Some(OWNED_STATED) => Ok(OwnedBlueprints::Stated),
        _ => Err(format!(
            "policies.ownedBlueprints is neither \"{OWNED_EVERY_BLUEPRINT}\" nor \"{OWNED_STATED}\""
        )),
    }
}

fn level(value: &Value, path: &str) -> Result<i32, String> {
    value
        .as_i64()
        .and_then(|raw| i32::try_from(raw).ok())
        .ok_or_else(|| format!("{path} is not an integer"))
}

impl InventoryDoc {
    /// This part from a schemaVersion 2 document: each key optional (the
    /// default when absent), each present one strict.
    pub(crate) fn parse(
        doc: &Map<String, Value>,
        policies: &Map<String, Value>,
    ) -> Result<Self, String> {
        let mut out = Self::default();
        if let Some(value) = doc.get(SECTION_FIXTURES) {
            out.fixtures = parse_fixtures(value)?;
        }
        if let Some(value) = doc.get(SECTION_CANVASES) {
            out.canvases = parse_canvases(value)?;
        }
        if let Some(value) = doc.get(SECTION_BLUEPRINTS) {
            out.blueprints = parse_blueprints(value)?;
        }
        if let Some(value) = doc.get(SECTION_ITEMS) {
            out.items = parse_items(value)?;
        }
        if let Some(gamedata) = doc.get(SECTION_GAMEDATA).and_then(Value::as_object) {
            if let Some(value) = gamedata.get(GAMEDATA_KEYS[0]) {
                out.material_possession_level =
                    level(value, "userMysekaiGamedata.mysekaiMaterialPossessionLevel")?;
            }
            if let Some(value) = gamedata.get(GAMEDATA_KEYS[1]) {
                out.fixture_possession_level =
                    level(value, "userMysekaiGamedata.mysekaiFixturePossessionLevel")?;
            }
        }
        if let Some(value) = policies.get(POLICY_KEYS[0]) {
            out.owned_blueprints = parse_owned_blueprints(value)?;
        }
        out.check_structure()?;
        Ok(out)
    }

    pub(crate) fn check_structure(&self) -> Result<(), String> {
        let mut keys = BTreeSet::new();
        for (i, row) in self.fixtures.iter().enumerate() {
            let at = format!("{SECTION_FIXTURES}[{i}]");
            if row.mysekai_fixture_id < 1 || row.texture_id < 1 {
                return Err(format!(
                    "{at} needs a fixture id and a texture id of at least 1"
                ));
            }
            if row.quantity < 0 {
                return Err(format!("{at}.quantity = {} is negative", row.quantity));
            }
            if !keys.insert((row.mysekai_fixture_id, row.texture_id)) {
                return Err(format!(
                    "{at} repeats fixture {} texture {}",
                    row.mysekai_fixture_id, row.texture_id
                ));
            }
        }
        let mut keys = BTreeSet::new();
        for (i, row) in self.canvases.iter().enumerate() {
            let at = format!("{SECTION_CANVASES}[{i}]");
            if row.quantity < 0 {
                return Err(format!("{at}.quantity = {} is negative", row.quantity));
            }
            if !keys.insert((row.mysekai_fixture_id, row.card_id, row.is_special_training)) {
                return Err(format!(
                    "{at} repeats fixture {} card {} isSpecialTraining {}",
                    row.mysekai_fixture_id, row.card_id, row.is_special_training
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (i, row) in self.blueprints.iter().enumerate() {
            if !ids.insert(row.mysekai_blueprint_id) {
                return Err(format!(
                    "{SECTION_BLUEPRINTS}[{i}] repeats blueprint {}",
                    row.mysekai_blueprint_id
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (i, row) in self.items.iter().enumerate() {
            let at = format!("{SECTION_ITEMS}[{i}]");
            if row.quantity < 0 {
                return Err(format!("{at}.quantity = {} is negative", row.quantity));
            }
            if !ids.insert(row.mysekai_item_id) {
                return Err(format!("{at} repeats item {}", row.mysekai_item_id));
            }
        }
        for (key, value) in [
            (GAMEDATA_KEYS[0], self.material_possession_level),
            (GAMEDATA_KEYS[1], self.fixture_possession_level),
        ] {
            if value < 1 {
                return Err(format!("userMysekaiGamedata.{key} = {value} is below 1"));
            }
        }
        Ok(())
    }

    /// Writes this part's keys into the document object and its policies;
    /// the levels go into the `userMysekaiGamedata` object already written.
    pub(crate) fn write(&self, doc: &mut Map<String, Value>, policies: &mut Map<String, Value>) {
        doc.insert(SECTION_FIXTURES.into(), fixtures_value(&self.fixtures));
        doc.insert(SECTION_CANVASES.into(), canvases_value(&self.canvases));
        doc.insert(
            SECTION_BLUEPRINTS.into(),
            blueprints_value(&self.blueprints),
        );
        doc.insert(SECTION_ITEMS.into(), items_value(&self.items));
        if let Some(gamedata) = doc.get_mut(SECTION_GAMEDATA).and_then(Value::as_object_mut) {
            gamedata.insert(
                GAMEDATA_KEYS[0].into(),
                json!(self.material_possession_level),
            );
            gamedata.insert(
                GAMEDATA_KEYS[1].into(),
                json!(self.fixture_possession_level),
            );
        }
        policies.insert(
            POLICY_KEYS[0].into(),
            json!(match self.owned_blueprints {
                OwnedBlueprints::EveryBlueprint => OWNED_EVERY_BLUEPRINT,
                OwnedBlueprints::Stated => OWNED_STATED,
            }),
        );
    }
}

/// The checks against the masters; an absent master skips its check.
pub(crate) fn check_masters(doc: &InventoryDoc, masters: &Masters) -> Result<(), String> {
    if let Some(blueprints) = &masters.craft.blueprints {
        for (i, row) in doc.blueprints.iter().enumerate() {
            if !blueprints.contains_key(&row.mysekai_blueprint_id) {
                return Err(format!(
                    "{SECTION_BLUEPRINTS}[{i}].mysekaiBlueprintId = {} is not a master blueprint",
                    row.mysekai_blueprint_id
                ));
            }
        }
    }
    for (key, value, rows) in [
        (
            GAMEDATA_KEYS[0],
            doc.material_possession_level,
            masters.possession.material.as_ref(),
        ),
        (
            GAMEDATA_KEYS[1],
            doc.fixture_possession_level,
            masters.possession.fixture.as_ref(),
        ),
    ] {
        if let Some(rows) = rows {
            if !rows.iter().any(|row| row.level == value) {
                return Err(format!(
                    "userMysekaiGamedata.{key} = {value} is not a master possession level"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

impl ServerModel {
    /// The owned blueprint rows the policy gives.
    pub(super) fn owned_blueprint_rows(&self) -> Vec<UserMysekaiBlueprint> {
        let stated = &self.doc.inventory.blueprints;
        match self.doc.inventory.owned_blueprints {
            OwnedBlueprints::Stated => stated.clone(),
            OwnedBlueprints::EveryBlueprint => match &self.masters.craft.blueprints {
                Some(master) => master
                    .keys()
                    .map(|id| {
                        stated
                            .iter()
                            .find(|row| row.mysekai_blueprint_id == *id)
                            .copied()
                            .unwrap_or(UserMysekaiBlueprint {
                                mysekai_blueprint_id: *id,
                                obtained_at: 0,
                            })
                    })
                    .collect(),
                None => stated.clone(),
            },
        }
    }

    /// Whether the user owns a blueprint by the owned blueprints policy.
    pub(super) fn owns_blueprint(&self, blueprint: &MasterBlueprint) -> bool {
        match self.doc.inventory.owned_blueprints {
            OwnedBlueprints::EveryBlueprint => true,
            OwnedBlueprints::Stated => self
                .doc
                .inventory
                .blueprints
                .iter()
                .any(|row| row.mysekai_blueprint_id == blueprint.id),
        }
    }

    /// `userMysekaiMaterials` as client rows.
    pub(super) fn material_rows(&self) -> Vec<UserMysekaiMaterial> {
        self.doc
            .delivery
            .mysekai_materials
            .iter()
            .map(|(id, quantity)| UserMysekaiMaterial {
                mysekai_material_id: *id,
                quantity: *quantity,
            })
            .collect()
    }

    /// The material possession policy (`None` without the materials master).
    pub(super) fn material_possession(&self) -> Option<UserMysekaiMaterialPossession> {
        let types = self.masters.craft.material_types.as_ref()?;
        let quantity = self
            .doc
            .delivery
            .mysekai_materials
            .iter()
            .filter(|(id, _)| {
                types
                    .get(id)
                    .is_none_or(|kind| !OVER_LIMIT_MATERIAL_TYPES.contains(&kind.as_str()))
            })
            .map(|(_, quantity)| *quantity)
            .sum();
        Some(UserMysekaiMaterialPossession { quantity })
    }

    pub(super) fn gamedata_row(&self) -> UserMysekaiGamedata {
        UserMysekaiGamedata {
            mysekai_rank: self.doc.gamedata.mysekai_rank,
            total_exp: self.doc.gamedata.total_exp,
            mysekai_material_possession_level: self.doc.inventory.material_possession_level,
            mysekai_fixture_possession_level: self.doc.inventory.fixture_possession_level,
        }
    }

    /// The owned tables named in `sections`, full, as the client merges them.
    pub(super) fn inventory_update(&mut self, sections: &[String]) -> SuiteUserSections {
        let has = |name: &str| sections.iter().any(|section| section == name);
        let materials = has(SECTION_MYSEKAI_MATERIALS);
        let possession = if materials {
            let possession = self.material_possession();
            if possession.is_none() {
                self.push_error(
                    "userMysekaiMaterialPossession is not carried: the materials master (mysekai-materials.json) is absent".into(),
                );
            }
            possession
        } else {
            None
        };
        let inventory = &self.doc.inventory;
        SuiteUserSections {
            user_mysekai_materials: materials.then(|| self.material_rows()),
            user_mysekai_material_possession: possession,
            user_mysekai_fixtures: has(SECTION_FIXTURES).then(|| inventory.fixtures.clone()),
            user_mysekai_canvases: has(SECTION_CANVASES).then(|| inventory.canvases.clone()),
            user_mysekai_blueprints: has(SECTION_BLUEPRINTS).then(|| self.owned_blueprint_rows()),
            user_mysekai_items: has(SECTION_ITEMS).then(|| inventory.items.clone()),
            user_mysekai_gamedata: has(SECTION_GAMEDATA).then(|| self.gamedata_row()),
        }
    }
}

/// The native instrument `MOLY_CRAFT_MOCK_MATERIALS` (`id:quantity`, comma
/// separated) as `userMysekaiMaterials` rows over the document's; a
/// malformed entry stops loudly.
pub(crate) fn native_overlay(doc: &mut super::document::ServerDocument) {
    const NAME: &str = "MOLY_CRAFT_MOCK_MATERIALS";
    let Some(raw) = super::instrument_env(NAME) else {
        return;
    };
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let parsed = entry.split_once(':').and_then(|(id, quantity)| {
            Some((
                id.trim().parse::<i32>().ok()?,
                quantity.trim().parse::<i32>().ok()?,
            ))
        });
        let Some((id, quantity)) = parsed.filter(|(id, quantity)| *id >= 1 && *quantity >= 0)
        else {
            panic!("{NAME} entry {entry:?} is not id:quantity with an id of at least 1 and a quantity of at least 0");
        };
        doc.delivery.mysekai_materials.insert(id, quantity);
    }
    info!(
        "[server] native overlay: {NAME} -> userMysekaiMaterials {:?}",
        doc.delivery.mysekai_materials
    );
}

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

fn possession_rows(text: &str, table: &str) -> Result<Vec<PossessionRow>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    super::keyed_rows(&value, table)?
        .iter()
        .map(|row| {
            Ok(PossessionRow {
                level: super::int32(row, "level")?,
                possession_limit: super::int32(row, "possessionLimit")?,
            })
        })
        .collect()
}

/// `mysekai-fixture-possessions.json`.
pub(crate) fn parse_fixture_possessions(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.possession.fixture = Some(possession_rows(text, "mysekaiFixturePossessions")?);
    Ok(())
}

/// `mysekai-material-possessions.json`.
pub(crate) fn parse_material_possessions(text: &str, masters: &mut Masters) -> Result<(), String> {
    masters.possession.material = Some(possession_rows(text, "mysekaiMaterialPossessions")?);
    Ok(())
}

/// `mysekai-materials.json`: material id -> `mysekaiMaterialType`.
pub(crate) fn parse_materials(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let types = super::keyed_rows(&value, "mysekaiMaterials")?
        .iter()
        .map(|row| {
            let kind = row["mysekaiMaterialType"]
                .as_str()
                .ok_or_else(|| format!("mysekaiMaterialType of {row} is not a string"))?;
            Ok((super::int32(row, "id")?, kind.to_owned()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    masters.craft.material_types = Some(types);
    Ok(())
}

/// `mysekai-items.json`: the first `white_blueprint` item.
pub(crate) fn parse_items_master(text: &str, masters: &mut Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let rows = super::keyed_rows(&value, "mysekaiItems")?;
    let white = rows
        .iter()
        .find(|row| row["mysekaiItemType"].as_str() == Some("white_blueprint"))
        .map(|row| super::int32(row, "id"))
        .transpose()?;
    masters.craft.white_blueprint_item = Some(white);
    Ok(())
}

// ---------------------------------------------------------------------------
// The client copy's other writer
// ---------------------------------------------------------------------------

/// The birthday-party delivery merges its replies' `userMysekaiMaterials`
/// straight into its copy; this keeps the inventory copy's read view equal.
pub(crate) fn mirror_materials(
    birthday: Res<super::delivery::ClientBirthdayPartyData>,
    mut inventory: ResMut<ClientMysekaiInventory>,
) {
    if !birthday.is_changed() {
        return;
    }
    let rows: Vec<UserMysekaiMaterial> = birthday
        .mysekai_materials()
        .iter()
        .map(|(id, quantity)| UserMysekaiMaterial {
            mysekai_material_id: *id,
            quantity: *quantity,
        })
        .collect();
    if rows != inventory.materials() {
        inventory.apply(SuiteUserSections {
            user_mysekai_materials: Some(rows),
            ..Default::default()
        });
    }
}

// ---------------------------------------------------------------------------
// Panel edits and schema
// ---------------------------------------------------------------------------

fn edit_int(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} = {raw} does not fit a 32-bit integer"))
}

/// A `server.edit` of this part: `Some(section)` names the section the next
/// response carries (`None` for a live policy); `None` when the path is not
/// this part's.
pub(crate) fn edit_path(
    doc: &mut InventoryDoc,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<Option<&'static str>, String>> {
    let result = match parts {
        [SECTION_FIXTURES] => parse_fixtures(value).map(|rows| {
            doc.fixtures = rows;
            Some(SECTION_FIXTURES)
        }),
        [SECTION_CANVASES] => parse_canvases(value).map(|rows| {
            doc.canvases = rows;
            Some(SECTION_CANVASES)
        }),
        [SECTION_BLUEPRINTS] => parse_blueprints(value).map(|rows| {
            doc.blueprints = rows;
            Some(SECTION_BLUEPRINTS)
        }),
        [SECTION_ITEMS] => parse_items(value).map(|rows| {
            doc.items = rows;
            Some(SECTION_ITEMS)
        }),
        [SECTION_ITEMS, id] => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{id} is not an item id"))?;
            let quantity = edit_int(value, path)?;
            match doc.items.iter_mut().find(|row| row.mysekai_item_id == id) {
                Some(row) => row.quantity = quantity,
                None => doc.items.push(UserMysekaiItem {
                    mysekai_item_id: id,
                    quantity,
                    last_obtained_at: 0,
                }),
            }
            Ok(Some(SECTION_ITEMS))
        })(),
        [SECTION_GAMEDATA, key] if GAMEDATA_KEYS.contains(key) => {
            edit_int(value, path).map(|level| {
                if *key == GAMEDATA_KEYS[0] {
                    doc.material_possession_level = level;
                } else {
                    doc.fixture_possession_level = level;
                }
                Some(SECTION_GAMEDATA)
            })
        }
        ["policies", "ownedBlueprints"] => parse_owned_blueprints(value).map(|policy| {
            doc.owned_blueprints = policy;
            // The client's blueprint copy follows the policy.
            Some(SECTION_BLUEPRINTS)
        }),
        _ => return None,
    };
    Some(result)
}

/// The panel's sections.
pub(crate) fn schema_sections() -> Value {
    json!([
        {
            "key": "mysekaiInventory",
            "title": "Owned fixtures, canvases, blueprints and items",
            "delivery": "next-response",
            "fields": [
                {"path": SECTION_FIXTURES, "type": "rows", "row": {
                    "mysekaiFixtureId": "int", "textureId": "int", "quantity": "int", "lastObtainedAt": "epoch-ms"},
                    "note": "one row per fixture and texture; the craft reply adds to it"},
                {"path": SECTION_CANVASES, "type": "rows", "row": {
                    "mysekaiFixtureId": "int", "cardId": "int", "isSpecialTraining": "bool", "quantity": "int", "lastObtainedAt": "epoch-ms"}},
                {"path": SECTION_BLUEPRINTS, "type": "rows", "row": {"mysekaiBlueprintId": "int", "obtainedAt": "epoch-ms"},
                    "when": {"policies.ownedBlueprints": OWNED_STATED}},
                {"path": SECTION_ITEMS, "type": "rows", "row": {"mysekaiItemId": "int", "quantity": "int", "lastObtainedAt": "epoch-ms"}},
                {"path": "userMysekaiItems.<mysekaiItemId>", "type": "int", "min": 0,
                    "note": "the white blueprint (the first white_blueprint item of the items master) pays for a sketch"},
                {"path": "userMysekaiMaterials.<mysekaiMaterialId>", "type": "int", "min": 0,
                    "note": "the birthday-party section's rows; the craft reply spends them"},
            ],
        },
        {
            "key": "mysekaiPossession",
            "title": "Possession levels",
            "delivery": "next-response",
            "fields": [
                {"path": "userMysekaiGamedata.mysekaiFixturePossessionLevel", "type": "int",
                    "min": 1, "levels": "masters.possession.fixture",
                    "note": "the fixture possession limit is the master row of this level; the craft reply refuses what the client's possession checks refuse"},
                {"path": "userMysekaiGamedata.mysekaiMaterialPossessionLevel", "type": "int",
                    "min": 1, "levels": "masters.possession.material"},
                {"path": "userMysekaiMaterialPossession", "type": "derived", "readOnly": true,
                    "by": "the material possession policy (an inference): the userMysekaiMaterials quantities except the game_character and birthday_party types"},
            ],
        },
        {
            "key": "mysekaiInventoryPolicies",
            "title": "Owned data server policies",
            "delivery": "live",
            "fields": [
                {"path": "policies.ownedBlueprints", "type": "enum", "values": [
                    {"value": OWNED_EVERY_BLUEPRINT, "label": "the user owns every master blueprint"},
                    {"value": OWNED_STATED, "label": "the userMysekaiBlueprints rows"},
                ]},
            ],
        },
    ])
}

/// The panel's named policies of this part.
pub(crate) fn schema_policies() -> Value {
    json!([
        {"name": "owned blueprints", "key": "policies.ownedBlueprints", "rule": "everyBlueprint: every master blueprint is owned; stated: the userMysekaiBlueprints rows. The craft list shows a blueprint only when it is available without possession or owned (UserResourceFactory)"},
        {"name": "material possession (inference)", "rule": "userMysekaiMaterialPossession.quantity = the sum of userMysekaiMaterials quantities except the game_character and birthday_party types (the types IsMaterialReceivableOverPossessionLimit lets over the limit), derived whenever a response carries the materials"},
        {"name": "possession levels", "rule": format!("both default to {DEFAULT_POSSESSION_LEVEL}, the first master level; the panel sets them")},
    ])
}
