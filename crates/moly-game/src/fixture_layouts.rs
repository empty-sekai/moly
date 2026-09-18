//! Offline layout ownership. Source housing layouts are bucketed by actual
//! mysekaiSiteId, not scene name (all three floors share one scene) or level.
//! The storage envelope is product-owned; it is not a server API response.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::{
    compact, gallery, Direction, FixturePlacements, GridPosition, PlacementMock, PLACEMENTS,
};
use crate::{settings_store, site::OfflineSceneContent};

#[path = "fixture_layout_inventory.rs"]
mod inventory;

pub(crate) const SECTION: &str = "OfflineSiteLayouts";
const VERSION: u64 = 1;

/// Private optimistic editor baseline. Full records, including unknown fields,
/// are compared so a same-site edit with unchanged inventory cannot be lost.
/// Missing returnedFixtures is the legacy representation of an empty list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditStorageSnapshot {
    site_id: u32,
    site_record: Option<Value>,
    returned_record: Value,
}

impl EditStorageSnapshot {
    fn capture(document: &Value, site_id: u32) -> Result<Self, String> {
        Ok(Self {
            site_id,
            site_record: saved_site(document, site_id)?.cloned(),
            returned_record: returned_record(document),
        })
    }

    fn check(&self, document: &Value, site_id: u32) -> Result<(), String> {
        if self.site_id != site_id {
            return Err(
                "Editor storage snapshot belongs to another site; no layout was written".into(),
            );
        }
        let current = Self::capture(document, site_id)?;
        if current.site_record != self.site_record {
            return Err("This site's saved layout changed while editing; the existing document and draft were retained".into());
        }
        if current.returned_record != self.returned_record {
            return Err("Offline inventory changed while editing; the existing document and draft were retained".into());
        }
        Ok(())
    }
}

fn returned_record(document: &Value) -> Value {
    document
        .get(SECTION)
        .and_then(|section| section.get("returnedFixtures"))
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

/// Created only from a successfully written document. Advancing the in-memory
/// editor/owner baseline requires no further I/O that could fail after commit.
pub(crate) struct LayoutSaveReceipt {
    document: Value,
    snapshot: EditStorageSnapshot,
}

impl LayoutSaveReceipt {
    fn from_written(document: Value, site_id: u32) -> Self {
        let key = site_id.to_string();
        // The checked patch always inserts this bucket with a valid site ID.
        // Take its fully merged record (including preserved unknown fields),
        // without introducing a new fallible post-write validation step.
        let snapshot = EditStorageSnapshot {
            site_id,
            site_record: document
                .get(SECTION)
                .and_then(|section| section.get("sites"))
                .and_then(|sites| sites.get(key.as_str()))
                .cloned(),
            returned_record: returned_record(&document),
        };
        Self { document, snapshot }
    }

    pub(crate) fn snapshot(&self) -> EditStorageSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Resource, Clone)]
pub(crate) struct SiteFixtureLayouts {
    document: Result<Value, String>,
    /// Switching away keeps the current map's rows/UIDs even before the user
    /// explicitly saves. Unsaved editor decisions are guarded by the editor.
    visited: HashMap<u32, FixturePlacements>,
}

impl Default for SiteFixtureLayouts {
    fn default() -> Self {
        Self {
            document: settings_store::read_document(),
            visited: HashMap::new(),
        }
    }
}

impl SiteFixtureLayouts {
    pub(crate) fn from_document(document: Value) -> Self {
        Self {
            document: Ok(document),
            visited: HashMap::new(),
        }
    }

    pub(crate) fn saved_level(&self, site_id: u32) -> Result<Option<u32>, String> {
        if let Some(layout) = self.visited.get(&site_id) {
            return Ok(Some(layout.level));
        }
        let Some(row) = saved_site(self.document.as_ref().map_err(Clone::clone)?, site_id)? else {
            return Ok(None);
        };
        positive_u32(&row["level"], "site level").map(Some)
    }

    pub(crate) fn remember(&mut self, layout: &FixturePlacements) {
        if layout.site_id != 0 {
            self.visited.insert(layout.site_id, layout.clone());
        }
    }

    /// Fresh entry reads the selected map and returned inventory together.
    /// Reject a stale loaded/cached map as well: otherwise a newer on-disk
    /// record sampled at Enter could legitimize writing an older world draft.
    pub(crate) fn begin_edit(
        &self,
        layout: &FixturePlacements,
    ) -> Result<(EditStorageSnapshot, Vec<super::EditableFixture>), String> {
        if layout.site_id == 0 {
            return Err("current site layout is not installed".into());
        }
        let document = settings_store::read_document()?;
        let snapshot = EditStorageSnapshot::capture(&document, layout.site_id)?;
        let cached = saved_site(
            self.document.as_ref().map_err(Clone::clone)?,
            layout.site_id,
        )?;
        if cached != snapshot.site_record.as_ref() {
            return Err("This site's saved record changed after it was loaded; reload it before editing; no record was changed".into());
        }
        if let Some(record) = snapshot.site_record.as_ref() {
            let saved = decode(record, layout.site_id, &layout.site_type, layout.level)?;
            if saved.editor_rows() != layout.editor_rows()
                || saved.next_edit_uid() != layout.next_edit_uid()
            {
                return Err("The loaded site no longer matches its saved furniture/UID state; reload before editing".into());
            }
        }
        let returned = inventory::read(&document)?;
        Ok((snapshot, returned))
    }

    /// Called only after a durable receipt. Its exact written document is
    /// installed directly; a separate read can neither invalidate this success
    /// nor replace the baseline with some later writer's unrelated document.
    pub(crate) fn did_save(&mut self, layout: &FixturePlacements, receipt: LayoutSaveReceipt) {
        self.remember(layout);
        self.document = Ok(receipt.document);
    }

    pub(crate) fn validate_target(
        &self,
        site_id: u32,
        site_type: &str,
        floor: crate::site::FloorGridLayout,
        content: OfflineSceneContent,
    ) -> Result<(), String> {
        let layout = self.restore(site_id, site_type, floor.level, content)?;
        validate_floor_layout(&layout, floor)
    }

    pub(crate) fn restore(
        &self,
        site_id: u32,
        site_type: &str,
        level: u32,
        content: OfflineSceneContent,
    ) -> Result<FixturePlacements, String> {
        if let Some(cached) = self.visited.get(&site_id) {
            if cached.site_type != site_type {
                return Err(format!(
                    "site {site_id} changed identity; existing layout is retained"
                ));
            }
            let mut layout = cached.clone();
            layout.level = level;
            return Ok(layout);
        }
        if let Some(saved) = saved_site(self.document.as_ref().map_err(Clone::clone)?, site_id)? {
            return decode(saved, site_id, site_type, level);
        }
        // This showcase is a HOME starter only. A room or harvest map without
        // a saved layout starts empty; it must never inherit outdoor rows.
        let mut rows = if site_type == "home_site" {
            match content {
                OfflineSceneContent::Compact => compact::PLACEMENTS.to_vec(),
                OfflineSceneContent::Full => PLACEMENTS.to_vec(),
            }
        } else {
            Vec::new()
        };
        if site_type == "home_site" {
            gallery::append_preview(&mut rows);
            if let Some(direction) = super::direction_override() {
                for row in &mut rows {
                    row.direction = direction;
                }
            }
        }
        let instance_uids = (1..=rows.len())
            .map(|serial| format!("offline-fixture-{site_id}-{serial}"))
            .collect();
        Ok(FixturePlacements {
            rows: rows.into_iter().map(PlacementMock::into_owned).collect(),
            instance_uids,
            site_id,
            site_type: site_type.to_owned(),
            level,
            next_edit_uid: 1,
            floor: None,
        })
    }
}

fn saved_site(document: &Value, site_id: u32) -> Result<Option<&Value>, String> {
    let Some(section) = document.get(SECTION) else {
        return Ok(None);
    };
    if section["version"].as_u64() != Some(VERSION) {
        return Err("Unsupported OfflineSiteLayouts version; saved data was not modified".into());
    }
    let sites = section["sites"]
        .as_object()
        .ok_or("OfflineSiteLayouts.sites must be an object; saved data was not modified")?;
    let Some(site) = sites.get(&site_id.to_string()) else {
        return Ok(None);
    };
    if site["mysekaiSiteId"].as_u64() != Some(site_id as u64) {
        return Err(format!(
            "layout bucket {site_id} contains a different mysekaiSiteId"
        ));
    }
    Ok(Some(site))
}

fn positive_u32(value: &Value, label: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v != 0)
        .ok_or_else(|| format!("{label} must be a positive u32"))
}

fn byte(value: &Value, label: &str) -> Result<i8, String> {
    value
        .as_i64()
        .and_then(|v| i8::try_from(v).ok())
        .ok_or_else(|| format!("{label} must be a signed grid byte"))
}

fn grid(value: &Value, label: &str) -> Result<GridPosition, String> {
    Ok(GridPosition::new(
        byte(&value["x"], label)?,
        byte(&value["y"], label)?,
        byte(&value["z"], label)?,
    ))
}

fn grid_json(value: GridPosition) -> Value {
    json!({ "x": value.x, "y": value.y, "z": value.z })
}

fn decode(
    saved: &Value,
    site_id: u32,
    site_type: &str,
    level: u32,
) -> Result<FixturePlacements, String> {
    if saved["siteType"].as_str() != Some(site_type) {
        return Err(format!("saved site {site_id} does not name {site_type}"));
    }
    // Validate the saved level even when an explicit input selects a different
    // one. Never reinterpret a malformed document as an empty/default layout.
    positive_u32(&saved["level"], "saved site level")?;
    let next_edit_uid = saved["nextEditUid"]
        .as_u64()
        .filter(|v| *v != 0)
        .ok_or("saved nextEditUid must be positive")?;
    let records = saved["fixtures"]
        .as_array()
        .ok_or("saved fixtures must be an array")?;
    let mut rows = Vec::with_capacity(records.len());
    let mut instance_uids = Vec::with_capacity(records.len());
    let mut unique = HashSet::new();
    for record in records {
        let uid = record["mysekaiUniqueId"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("saved fixture UID is empty")?;
        if !unique.insert(uid) {
            return Err(format!("saved fixture UID is duplicated: {uid}"));
        }
        let name = record["package"]
            .as_str()
            .ok_or("saved fixture package is missing")?;
        let package = canonical_package(name)
            .ok_or_else(|| format!("saved fixture package is invalid: {name}"))?;
        let min = grid(&record["minimum"], "saved minimum")?;
        let max = grid(&record["maximum"], "saved maximum")?;
        let center_y = byte(&record["centerY"], "saved centerY")?;
        let layout = record["layoutType"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or("saved layoutType must be u8")?;
        let direction = record["rotation"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .and_then(Direction::from_u8)
            .ok_or("saved rotation must be 0..3")?;
        let fixture_id = record["mysekaiFixtureId"]
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .ok_or("saved mysekaiFixtureId must be i32")?;
        let (mut center, size) = moly_law::fixture::position::footprint_to_center_size(min, max)?;
        center.y = center_y;
        let (placed_min, placed_max) =
            moly_law::fixture::position::layout_footprint(center, size, direction, layout)?;
        moly_law::fixture::position::field_position(placed_min, placed_max, center_y, layout)?;
        rows.push(PlacementMock {
            texture_id: record
                .get("textureId")
                .map(|v| positive_u32(v, "textureId"))
                .transpose()?
                .unwrap_or(1),
            package,
            min,
            max,
            center_y,
            layout,
            direction,
            fixture_id,
        });
        instance_uids.push(uid.to_owned());
    }
    Ok(FixturePlacements {
        rows,
        instance_uids,
        site_id,
        site_type: site_type.to_owned(),
        level,
        next_edit_uid,
        floor: None,
    })
}

fn canonical_package(name: &str) -> Option<String> {
    let suffix = name.strip_prefix("mysekai__fixture__")?;
    (!suffix.is_empty()
        && suffix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_'))
    .then(|| name.to_owned())
}

/// Build every imported site before the caller replaces the local document.
pub(crate) fn import_section(
    data: &moly_assets::player_data::ImportedPlayerData,
    sites: &crate::site::Sites,
) -> Result<Value, String> {
    let mut records = Map::new();
    for site in &data.sites {
        if sites.site_id(&site.site_type) != Some(site.id) {
            return Err(format!(
                "Site {} does not match the loaded scene catalog",
                site.id
            ));
        }
        let floor = sites.floor_grid(&site.site_type, site.level)?;
        let fixtures: Vec<_> = site
            .fixtures
            .iter()
            .map(|row| {
                let (min, max) =
                    moly_law::fixture::position::footprint_front(row.center, row.grid_size);
                json!({ "mysekaiUniqueId": row.uid, "package": row.package,
                "mysekaiFixtureId": row.fixture_id, "textureId": row.texture_id,
                "minimum": grid_json(min), "maximum": grid_json(max), "centerY": row.center.y,
                "layoutType": row.layout, "rotation": row.direction as u8,
                "sourceArray": row.source_array, "sourceData": row.source })
            })
            .collect();
        let record = json!({ "mysekaiSiteId": site.id, "siteType": site.site_type,
            "level": site.level, "nextEditUid": 1, "fixtures": fixtures,
            "mysekaiFixtureSurfaceAppearances": site.source.get("mysekaiFixtureSurfaceAppearances")
                .cloned().unwrap_or_else(|| json!([])),
            "mysekaiPhenomenaId": site.source.get("mysekaiPhenomenaId") });
        let layout = decode(&record, site.id, &site.site_type, site.level)?;
        validate_floor_layout(&layout, floor)?;
        records.insert(site.id.to_string(), record);
    }
    Ok(json!({ "version": VERSION, "sites": records, "returnedFixtures": [] }))
}

pub(crate) fn validate_document(
    document: &Value,
    sites: &crate::site::Sites,
) -> Result<(), String> {
    let Some(section) = document.get(SECTION) else {
        return Ok(());
    };
    if section["version"].as_u64() != Some(VERSION) {
        return Err("Unsupported layout backup version".into());
    }
    for (key, record) in section["sites"]
        .as_object()
        .ok_or("Backup sites must be an object")?
    {
        let id = key
            .parse::<u32>()
            .map_err(|_| "Backup site ID is invalid")?;
        saved_site(document, id)?;
        let site_type = record["siteType"]
            .as_str()
            .ok_or("Backup site type is missing")?;
        if sites.site_id(site_type) != Some(id) {
            return Err("Backup site identity does not match the catalog".into());
        }
        let level = positive_u32(&record["level"], "backup site level")?;
        validate_floor_layout(
            &decode(record, id, site_type, level)?,
            sites.floor_grid(site_type, level)?,
        )?;
    }
    inventory::read(document)?;
    inventory::previous_records(document)?;
    Ok(())
}

/// Existing rows are checked too: a level override may not turn a large saved
/// room into a small room while retaining furniture beyond its source grid.
/// No row is cropped, moved or deleted. Walls belong to their own layout grids
/// and are not incorrectly tested as floor volume by this boundary.
pub(crate) fn validate_floor_layout(
    layout: &FixturePlacements,
    floor: crate::site::FloorGridLayout,
) -> Result<(), String> {
    use moly_law::fixture::position::{layout_type, WALL_LAYOUT_MASK};
    if floor.width <= 0 || floor.height <= 0 || floor.depth <= 0 {
        return Err("selected source floor has invalid dimensions".into());
    }
    let half_x = (floor.width + 1) / 2;
    let half_z = (floor.depth + 1) / 2;
    for (row, uid) in layout.rows.iter().zip(&layout.instance_uids) {
        if row.layout & WALL_LAYOUT_MASK != 0 {
            continue;
        }
        let height = if row.layout == layout_type::FLOOR {
            floor.height
        } else if matches!(row.layout, layout_type::RUG | layout_type::ROAD) {
            1
        } else {
            return Err(format!(
                "layout {uid} has an unsupported ground layer {}",
                row.layout
            ));
        };
        let placed = row.placed();
        if (placed.min.x as i32) < -half_x
            || (placed.max.x as i32) >= half_x
            || (placed.min.z as i32) < -half_z
            || (placed.max.z as i32) >= half_z
            || placed.min.y < 0
            || placed.max.y as i32 >= height
        {
            return Err(format!("site {} ({}) level {} is incompatible with existing fixture {} ({}); choose a compatible stage or edit its layout first; saved rows were retained",
                layout.site_id, layout.site_type, floor.level, uid, row.package));
        }
    }
    Ok(())
}

/// Layout and returned items share the existing settings write. Validate the
/// exact Enter/last-success snapshot and build the patch on the SAME fresh
/// document used by merge/write under the exclusive settings transaction.
pub(crate) fn persist_edit(
    layout: &FixturePlacements,
    returned: &[super::EditableFixture],
    expected: &EditStorageSnapshot,
) -> Result<LayoutSaveReceipt, String> {
    if layout.site_id == 0 {
        return Err("current site layout is not installed".into());
    }
    if layout.rows.len() != layout.instance_uids.len() {
        return Err("fixture rows and instance UID columns differ".into());
    }
    validate_floor_layout(
        layout,
        layout.floor.ok_or("selected source floor is unavailable")?,
    )?;
    let document = settings_store::save_sections_checked(|document| {
        expected.check(document, layout.site_id)?;
        // Keep the existing strict inventory/document schema and ownership
        // checks; the snapshot check does not reinterpret malformed data.
        inventory::read(document)?;
        inventory::validate_ownership(document, layout, returned)?;
        let previous_rows = inventory::previous_records(document)?;
        let mut fixtures = Vec::with_capacity(layout.rows.len());
        for (row, uid) in layout.rows.iter().zip(&layout.instance_uids) {
            let mut record = previous_rows
                .get(uid.as_str())
                .map(|row| (*row).clone())
                .unwrap_or_else(|| Value::Object(Map::new()));
            let object = record
                .as_object_mut()
                .ok_or("saved fixture record must be an object")?;
            for (key, value) in [
                ("mysekaiUniqueId", json!(uid)),
                ("package", json!(row.package)),
                ("mysekaiFixtureId", json!(row.fixture_id)),
                ("minimum", grid_json(row.min)),
                ("maximum", grid_json(row.max)),
                ("centerY", json!(row.center_y)),
                ("layoutType", json!(row.layout)),
                ("rotation", json!(row.direction as u8)),
                ("textureId", json!(row.texture_id)),
            ] {
                object.insert(key.to_owned(), value);
            }
            fixtures.push(record);
        }
        let mut sites = Map::new();
        sites.insert(
            layout.site_id.to_string(),
            json!({
                "mysekaiSiteId": layout.site_id,
                "siteType": layout.site_type,
                "level": layout.level,
                "nextEditUid": layout.next_edit_uid,
                "fixtures": fixtures,
            }),
        );
        let mut patch = json!({ "version": VERSION, "sites": sites });
        patch["returnedFixtures"] = inventory::encode(returned, &previous_rows)?;
        Ok(vec![(SECTION.to_owned(), patch)])
    })?;
    Ok(LayoutSaveReceipt::from_written(document, layout.site_id))
}
