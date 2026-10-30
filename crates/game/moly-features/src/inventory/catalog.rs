//! The master tables the inventory screen reads besides the rows' names: a
//! material's type, site ids and description; a tool's description; and the tab rows with
//! their genre names.
//!
//! The tables are requested at start-up (`catalog::request`) and taken when
//! they resolve (`MasterData::take`). A table the runtime root does not carry
//! leaves the columns it holds at their reading without it: a material's
//! description empty, and the screen names the
//! absent table in its log once.
//!
//! The fixture filter document (`mysekaiFixtureFilteredDatas`) holds one
//! `MysekaiFixtureFilteredData` per search target (craft, inventory, site
//! edit).

use std::collections::HashMap;

use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterError, MasterTable};
use serde_json::Value;

use super::order::{self, FixtureFilter, Tab};

/// The layout cost of a fixture (`PutCost`), by fixture id.

/// A material's type (`mysekaiMaterialType`), by material id.
pub type MaterialTypes = HashMap<i32, String>;

/// The site ids a material appears on (`mysekaiSiteIds`).
pub type MaterialSites = HashMap<i32, Vec<i32>>;

/// A material's `description`, by material id. The materials master of the
/// runtime root serves no `description` column, so the map stays empty there
/// and a material's page shows its name in place of the flavour text.
pub type MaterialTexts = HashMap<i32, String>;

/// A tool's `description` and `assetbundleName`, by tool id.
pub type ToolTexts = HashMap<i32, (String, String)>;

fn int(row: &Value, key: &str) -> Option<i32> {
    row[key].as_i64().and_then(|value| i32::try_from(value).ok())
}

fn field_text(row: &Value, key: &str) -> String {
    row[key].as_str().unwrap_or_default().to_owned()
}

/// `mysekaiMaterials`: the types, the site ids and the descriptions.
fn parse_materials(text: &str) -> Result<(MaterialTypes, MaterialSites, MaterialTexts), String> {
    let mut types = HashMap::new();
    let mut sites = HashMap::new();
    let mut texts = HashMap::new();
    for row in &rows(text)? {
        let Some(id) = int(row, "id") else { continue };
        if let Some(kind) = row["mysekaiMaterialType"].as_str() {
            types.insert(id, kind.to_owned());
        }
        let ids: Vec<i32> = row["mysekaiSiteIds"]
            .as_array()
            .map(|sites| sites.iter().filter_map(Value::as_i64).map(|id| id as i32).collect())
            .unwrap_or_default();
        if !ids.is_empty() {
            sites.insert(id, ids);
        }
        texts.insert(id, field_text(row, "description"));
    }
    Ok((types, sites, texts))
}

/// `mysekaiTools`: each tool's description and model bundle.
fn parse_tools(text: &str) -> Result<ToolTexts, String> {
    let mut out = HashMap::new();
    for row in &rows(text)? {
        let Some(id) = int(row, "id") else { continue };
        out.insert(
            id,
            (field_text(row, "description"), field_text(row, "assetbundleName")),
        );
    }
    Ok(out)
}

const MATERIALS: MasterTable<(MaterialTypes, MaterialSites, MaterialTexts)> = MasterTable {
    table: "mysekaiMaterials",
    name: "mysekaiMaterials (the inventory material types and texts)",
    parse: parse_materials,
};
const TOOLS: MasterTable<ToolTexts> = MasterTable {
    table: "mysekaiTools",
    name: "mysekaiTools (the inventory tool description)",
    parse: parse_tools,
};
const FIXTURE_TAB_VIEWS: MasterTable<Vec<Value>> = MasterTable {
    table: "mysekaiFixtureTabViews",
    name: "mysekaiFixtureTabViews (the inventory genre tabs)",
    parse: rows,
};
const MAIN_GENRES: MasterTable<Vec<Value>> = MasterTable {
    table: "mysekaiFixtureMainGenres",
    name: "mysekaiFixtureMainGenres (the inventory genre tab names)",
    parse: rows,
};
const SUB_GENRES: MasterTable<Vec<Value>> = MasterTable {
    table: "mysekaiFixtureSubGenres",
    name: "mysekaiFixtureSubGenres (the inventory genre sub tab names)",
    parse: rows,
};
const MATERIAL_TABS: MasterTable<Vec<Value>> = MasterTable {
    table: "mysekaiMaterialTabs",
    name: "mysekaiMaterialTabs (the inventory material type tabs)",
    parse: rows,
};

/// Start-up: requests the tables the screen reads besides the rows' masters.
pub fn request(data: &mut MasterData) {
    data.request(&MATERIALS);
    data.request(&TOOLS);
    data.request(&FIXTURE_TAB_VIEWS);
    data.request(&MAIN_GENRES);
    data.request(&SUB_GENRES);
    data.request(&MATERIAL_TABS);
}

/// A table's rows: the upstream array, or the keyed document's `entries`
/// (its rows by id, in the document's order).
fn rows(text: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if let Some(rows) = value.as_array() {
        return Ok(rows.clone());
    }
    let entries = value["entries"]
        .as_object()
        .ok_or("the document is neither an array of rows nor a keyed table")?;
    Ok(entries.values().cloned().collect())
}

fn take<T: 'static>(
    slot: &mut Option<Result<T, MasterError>>,
    data: &mut MasterData,
    table: &MasterTable<T>,
) {
    if slot.is_none() {
        *slot = data.take(table);
    }
}

/// The tables, by what they are read for.
#[derive(Resource, Default)]
pub struct Catalog {
    materials: Option<Result<(MaterialTypes, MaterialSites, MaterialTexts), MasterError>>,
    tools: Option<Result<ToolTexts, MasterError>>,
    pub fixture_tab_views: Option<Result<Vec<Value>, MasterError>>,
    pub main_genres: Option<Result<Vec<Value>, MasterError>>,
    pub sub_genres: Option<Result<Vec<Value>, MasterError>>,
    pub material_tab_rows: Option<Result<Vec<Value>, MasterError>>,
    /// The absent tables whose names were logged.
    named: Vec<&'static str>,
}

impl Catalog {
    /// Takes each table as it resolves.
    pub fn poll(&mut self, data: &mut MasterData) {
        take(&mut self.materials, data, &MATERIALS);
        take(&mut self.tools, data, &TOOLS);
        take(&mut self.fixture_tab_views, data, &FIXTURE_TAB_VIEWS);
        take(&mut self.main_genres, data, &MAIN_GENRES);
        take(&mut self.sub_genres, data, &SUB_GENRES);
        take(&mut self.material_tab_rows, data, &MATERIAL_TABS);
    }

    /// Whether the layer answered every table.
    pub fn settled(&self) -> bool {
        self.materials.is_some()
            && self.tools.is_some()
            && self.fixture_tab_views.is_some()
            && self.main_genres.is_some()
            && self.sub_genres.is_some()
            && self.material_tab_rows.is_some()
    }

    /// The catalogue that answers nothing: a stage, which draws no screen.
    pub fn answered() -> Self {
        Self {
            materials: Some(Ok((
                MaterialTypes::new(),
                MaterialSites::new(),
                MaterialTexts::new(),
            ))),
            tools: Some(Ok(ToolTexts::new())),
            fixture_tab_views: Some(Ok(Vec::new())),
            main_genres: Some(Ok(Vec::new())),
            sub_genres: Some(Ok(Vec::new())),
            material_tab_rows: Some(Ok(Vec::new())),
            named: Vec::new(),
        }
    }

    /// Fills the rows' columns the catalogue tables hold: a material's
    /// description.
    pub fn decorate(&self, kind: super::ContentListType, rows: &mut [super::Row]) {
        use super::ContentListType;
        for row in rows {
            let id = row.id;
            let Some(master) = row.master.as_mut() else {
                continue;
            };
            match kind {
                ContentListType::Fixture => {}
                ContentListType::Material => {
                    if let Some(texts) = Self::ok(&self.materials).map(|materials| &materials.2) {
                        if let Some(description) = texts.get(&id) {
                            master.description = description.clone();
                        }
                    }
                }
                ContentListType::Item | ContentListType::Tool => {}
            }
        }
    }

    /// Names each absent table once, with what its columns read without it.
    pub fn name_absent(&mut self) {
        let tables: [(&str, Option<String>); 6] = [
            ("mysekaiMaterials", reason(&self.materials)),
            ("mysekaiTools", reason(&self.tools)),
            ("mysekaiFixtureTabViews", reason(&self.fixture_tab_views)),
            ("mysekaiFixtureMainGenres", reason(&self.main_genres)),
            ("mysekaiFixtureSubGenres", reason(&self.sub_genres)),
            ("mysekaiMaterialTabs", reason(&self.material_tab_rows)),
        ];
        for (name, reason) in tables {
            let Some(reason) = reason else { continue };
            if self.named.contains(&name) {
                continue;
            }
            self.named.push(name);
            warn!(
                "[inventory] the master {name} is absent ({reason}); the columns it holds read empty and its tabs are not drawn"
            );
        }
    }

    fn ok<T>(slot: &Option<Result<T, MasterError>>) -> Option<&T> {
        slot.as_ref().and_then(|result| result.as_ref().ok())
    }

    pub fn material_types(&self) -> Option<&MaterialTypes> {
        Self::ok(&self.materials).map(|materials| &materials.0)
    }

    pub fn material_sites(&self) -> Option<&MaterialSites> {
        Self::ok(&self.materials).map(|materials| &materials.1)
    }

    pub fn material_texts(&self) -> Option<&MaterialTexts> {
        Self::ok(&self.materials).map(|materials| &materials.2)
    }

    pub fn tools(&self) -> Option<&ToolTexts> {
        Self::ok(&self.tools)
    }

    /// The fixture list's tabs, `seq` order (`order::fixture_tabs`).
    pub fn fixture_tabs(&self) -> Vec<Tab> {
        let rows = |slot: &Option<Result<Vec<Value>, MasterError>>| {
            Self::ok(slot).cloned().unwrap_or_default()
        };
        order::fixture_tabs(
            &rows(&self.fixture_tab_views),
            &rows(&self.main_genres),
            &rows(&self.sub_genres),
        )
    }

    /// The material type tabs, master order (`order::material_tabs`).
    pub fn material_tabs(&self) -> Vec<Tab> {
        let rows = |slot: &Option<Result<Vec<Value>, MasterError>>| {
            Self::ok(slot).cloned().unwrap_or_default()
        };
        order::material_tabs(&rows(&self.material_tab_rows))
    }
}

/// The reason a table is absent (`None`: it is there, or still loading).
fn reason<T>(slot: &Option<Result<T, MasterError>>) -> Option<String> {
    match slot {
        Some(Err(error)) => Some(error.to_string()),
        _ => None,
    }
}

/// The stored fixture filters (`MysekaiFixtureFilteredData`), one per search
/// target: the fixture list reads the inventory row (target 1).
#[derive(Resource, Default)]
pub struct Filters {
    pub rows: [FixtureFilter; 3],
}

impl Filters {
    pub fn set(&mut self, target: usize, filter: FixtureFilter) {
        if let Some(slot) = self.rows.get_mut(target) {
            *slot = filter;
        }
    }
}

/// The `mysekaiFixtureFilteredDatas` rows as the document holds them
/// (`entries`, or the bare array); a row's `searchTargetType` places it, and
/// a row without one takes its index.
pub fn filters_from(value: &Value) -> [FixtureFilter; 3] {
    let rows = match value["entries"].as_array().or_else(|| value.as_array()) {
        Some(rows) => rows,
        None => return Default::default(),
    };
    let mut out: [FixtureFilter; 3] = Default::default();
    for (index, row) in rows.iter().enumerate() {
        let target = row["searchTargetType"]
            .as_u64()
            .map_or(index, |target| target as usize);
        if let Some(slot) = out.get_mut(target) {
            *slot = FixtureFilter::read(row);
        }
    }
    out
}
