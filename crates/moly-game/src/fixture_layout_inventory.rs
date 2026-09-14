//! Account-wide returned items for the product-owned offline layout envelope.
//! It retains exact instance identity; it is not a claim about a server account.

use super::{
    Direction, FixturePlacements, SECTION, VERSION, byte, canonical_package, grid, grid_json,
};
use crate::fixture::EditableFixture;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

pub(super) fn read(document: &Value) -> Result<Vec<EditableFixture>, String> {
    let Some(section) = document.get(SECTION) else {
        return Ok(Vec::new());
    };
    if section["version"].as_u64() != Some(VERSION) {
        return Err("Unsupported OfflineSiteLayouts version; saved data was not modified".into());
    }
    let Some(records) = section.get("returnedFixtures") else {
        return Ok(Vec::new());
    };
    let records = records
        .as_array()
        .ok_or("returnedFixtures must be an array")?;
    let mut result = Vec::with_capacity(records.len());
    let mut seen = HashSet::new();
    for record in records {
        let uid = record["mysekaiUniqueId"]
            .as_str()
            .filter(|uid| !uid.is_empty())
            .ok_or("returned fixture UID is empty")?;
        if !seen.insert(uid) {
            return Err(format!("returned fixture UID is duplicated: {uid}"));
        }
        let name = record["package"]
            .as_str()
            .ok_or("returned fixture package is missing")?;
        let package = canonical_package(name).ok_or_else(|| {
            format!("returned fixture package is not supported by the offline producer: {name}")
        })?;
        let min = grid(&record["minimum"], "returned minimum")?;
        let max = grid(&record["maximum"], "returned maximum")?;
        let (mut center, grid_size) =
            moly_law::fixture::position::footprint_to_center_size(min, max)?;
        center.y = byte(&record["centerY"], "returned centerY")?;
        let layout = record["layoutType"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or("returned layoutType must be u8")?;
        let direction = record["rotation"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .and_then(Direction::from_u8)
            .ok_or("returned rotation must be 0..3")?;
        let fixture_id = record["mysekaiFixtureId"]
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .ok_or("returned mysekaiFixtureId must be i32")?;
        let item = EditableFixture {
            uid: uid.into(),
            package,
            fixture_id,
            center,
            grid_size,
            layout,
            direction,
        };
        item.pose()?;
        result.push(item);
    }
    Ok(result)
}

/// All previous records are available when an item moves map→inventory or
/// inventory→another map, so domain-unknown fields travel with that same UID.
pub(super) fn previous_records(document: &Value) -> Result<HashMap<&str, &Value>, String> {
    let mut result = HashMap::new();
    if let Some(section) = document.get(SECTION) {
        let sites = section
            .get("sites")
            .and_then(Value::as_object)
            .ok_or("OfflineSiteLayouts.sites must be an object")?;
        for site in sites.values() {
            let records = site["fixtures"]
                .as_array()
                .ok_or("saved fixtures must be an array")?;
            for record in records {
                let uid = record["mysekaiUniqueId"]
                    .as_str()
                    .filter(|uid| !uid.is_empty())
                    .ok_or("saved fixture UID is empty")?;
                if result.insert(uid, record).is_some() {
                    return Err(format!("UID belongs to multiple saved records: {uid}"));
                }
            }
        }
        if let Some(records) = section.get("returnedFixtures") {
            for record in records
                .as_array()
                .ok_or("returnedFixtures must be an array")?
            {
                let uid = record["mysekaiUniqueId"]
                    .as_str()
                    .filter(|uid| !uid.is_empty())
                    .ok_or("returned fixture UID is empty")?;
                if result.insert(uid, record).is_some() {
                    return Err(format!("UID is both placed and returned: {uid}"));
                }
            }
        }
    }
    Ok(result)
}

pub(super) fn validate_ownership(
    document: &Value,
    layout: &FixturePlacements,
    inventory: &[EditableFixture],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for item in layout.editor_rows().iter().chain(inventory) {
        if item.uid.is_empty() || !seen.insert(item.uid.clone()) {
            return Err(format!(
                "edited UID is empty or appears in both layout and inventory: {}",
                item.uid
            ));
        }
    }
    if let Some(sites) = document
        .get(SECTION)
        .and_then(|section| section.get("sites"))
    {
        for (id, site) in sites
            .as_object()
            .ok_or("OfflineSiteLayouts.sites must be an object")?
        {
            if id == &layout.site_id.to_string() {
                continue;
            }
            for record in site["fixtures"]
                .as_array()
                .ok_or("saved fixtures must be an array")?
            {
                let uid = record["mysekaiUniqueId"]
                    .as_str()
                    .filter(|uid| !uid.is_empty())
                    .ok_or("saved fixture UID is empty")?;
                if !seen.insert(uid.to_owned()) {
                    return Err(format!(
                        "fixture UID {uid} is already owned by another map; no record was copied or removed"
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn encode(
    items: &[EditableFixture],
    previous: &HashMap<&str, &Value>,
) -> Result<Value, String> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let (min, max) = moly_law::fixture::position::footprint_front(item.center, item.grid_size);
        moly_law::fixture::position::footprint_to_center_size(min, max)?;
        let mut record = previous
            .get(item.uid.as_str())
            .map(|record| (*record).clone())
            .unwrap_or_else(|| Value::Object(Map::new()));
        let object = record
            .as_object_mut()
            .ok_or("returned fixture record must be an object")?;
        for (key, value) in [
            ("mysekaiUniqueId", json!(item.uid)),
            ("package", json!(item.package)),
            ("mysekaiFixtureId", json!(item.fixture_id)),
            ("minimum", grid_json(min)),
            ("maximum", grid_json(max)),
            ("centerY", json!(item.center.y)),
            ("layoutType", json!(item.layout)),
            ("rotation", json!(item.direction as u8)),
        ] {
            object.insert(key.to_owned(), value);
        }
        records.push(record);
    }
    Ok(Value::Array(records))
}
