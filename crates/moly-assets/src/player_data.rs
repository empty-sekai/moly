//! Original Mysekai housing responses joined to caller-supplied master data.
//! Source grid layouts are reflected into the renderer frame. Player IDs stay opaque.

use moly_law::fixture::{Direction, GridPosition, Vector3Int, position::layout_type};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FIXTURES: usize = 20_000;
const ARRAYS: [&str; 9] = [
    "mysekaiFixtures",
    "mysekaiCanvases",
    "mysekaiGrowingPlants",
    "mysekaiCustomFixtureCollections",
    "mysekaiCustomFixturePenlights",
    "mysekaiCustomFixtureHonors",
    "mysekaiCustomFixtureBondsHonors",
    "mysekaiCustomFixtureRecordJackets",
    "mysekaiCustomFixturePhotos",
];

#[derive(Clone, Debug)]
pub struct ImportedFixture {
    pub uid: String,
    pub fixture_id: i32,
    pub package: String,
    pub center: GridPosition,
    pub grid_size: Vector3Int,
    pub layout: u8,
    pub direction: Direction,
    pub texture_id: u32,
    pub source: Value,
    pub source_array: String,
}

#[derive(Clone, Debug)]
pub struct ImportedSite {
    pub id: u32,
    pub site_type: String,
    pub level: u32,
    pub fixtures: Vec<ImportedFixture>,
    pub source: Value,
}

#[derive(Clone, Debug)]
pub struct ImportedPlayerData {
    pub rank: u32,
    pub region: String,
    pub player_name: Option<String>,
    pub player_id: Option<String>,
    pub sites: Vec<ImportedSite>,
    pub source: Value,
    pub notices: Vec<ImportNotice>,
}

impl ImportedPlayerData {
    pub fn fixture_count(&self) -> usize {
        self.sites.iter().map(|site| site.fixtures.len()).sum()
    }
}

/// Machine-readable record of something an otherwise successful import left
/// out. The wording belongs to the consumer: the native status line renders
/// [`ImportNotice::describe`], and the browser host localizes `code` with the
/// carried counts so one import reads correctly in every language.
///
/// A player whose saved layout holds furniture the exported catalog never
/// produced is normal, not corruption. Those instances are dropped from the
/// scene and reported here; structural problems still fail the whole import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum ImportNotice {
    /// Custom/special furniture arrays kept as records but not rendered.
    SpecialFurnitureRetained { count: usize },
    /// Floor/wall skin records kept while the scene keeps its own textures.
    SurfaceAppearanceRetained { count: usize },
    /// No exported model package for the master fixture ID.
    FixtureModelMissing { count: usize, fixtures: Vec<u32> },
    /// The saved texture ID has no entry in this master version.
    FixtureTextureMissing { count: usize, fixtures: Vec<u32> },
    /// The texture exists but no exported color texture came with the catalog.
    FixtureColorMissing { count: usize, fixtures: Vec<u32> },
}

/// How many distinct fixture IDs a notice spells out. The count stays exact;
/// the list only needs to be long enough to point at the catalog gap.
const NOTICE_FIXTURE_LIMIT: usize = 32;

impl ImportNotice {
    /// English one-line text for the native status line and logs. Browser
    /// consumers read the structured fields instead of parsing this string.
    pub fn describe(&self) -> String {
        fn listed(fixtures: &[u32]) -> String {
            fixtures
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
        match self {
            Self::SpecialFurnitureRetained { count } => format!(
                "{count} custom furniture records retained. Custom images and ornaments are not rendered by this importer yet."
            ),
            Self::SurfaceAppearanceRetained { count } => format!(
                "{count} floor/wall skin records retained. The scene still uses its supplied floor/wall textures."
            ),
            Self::FixtureModelMissing { count, fixtures } => format!(
                "{count} furniture instances skipped: no exported model for fixture IDs {}.",
                listed(fixtures)
            ),
            Self::FixtureTextureMissing { count, fixtures } => format!(
                "{count} furniture instances skipped: texture not present in this master version for fixture IDs {}.",
                listed(fixtures)
            ),
            Self::FixtureColorMissing { count, fixtures } => format!(
                "{count} furniture instances skipped: no exported color texture for fixture IDs {}.",
                listed(fixtures)
            ),
        }
    }
}

/// Catalog gaps collected while reading one player document.
#[derive(Default)]
struct SkippedFixtures {
    model_missing: MissingFixtures,
    texture_missing: MissingFixtures,
    color_missing: MissingFixtures,
}

impl SkippedFixtures {
    fn notices(&self) -> Vec<ImportNotice> {
        [
            self.model_missing
                .notice(|count, fixtures| ImportNotice::FixtureModelMissing { count, fixtures }),
            self.texture_missing
                .notice(|count, fixtures| ImportNotice::FixtureTextureMissing { count, fixtures }),
            self.color_missing
                .notice(|count, fixtures| ImportNotice::FixtureColorMissing { count, fixtures }),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[derive(Default)]
struct MissingFixtures {
    /// Instances skipped, counting repeats of the same master fixture ID.
    count: usize,
    /// Distinct master fixture IDs, first-seen order, capped for the payload.
    fixtures: Vec<u32>,
}

impl MissingFixtures {
    fn record(&mut self, fixture_id: u32) {
        self.count += 1;
        if self.fixtures.len() < NOTICE_FIXTURE_LIMIT && !self.fixtures.contains(&fixture_id) {
            self.fixtures.push(fixture_id);
        }
    }

    fn notice(&self, build: impl FnOnce(usize, Vec<u32>) -> ImportNotice) -> Option<ImportNotice> {
        (self.count > 0).then(|| build(self.count, self.fixtures.clone()))
    }
}

pub struct PlayerDataCatalog {
    region: String,
    tables: HashMap<String, HashMap<u32, Value>>,
    packages: HashSet<String>,
    colors: Value,
}

impl PlayerDataCatalog {
    pub fn from_json(master: &str, models: &str) -> Result<Self, String> {
        let document: Value =
            serde_json::from_str(master).map_err(|e| format!("Import catalog: {e}"))?;
        if document["version"].as_u64() != Some(1) {
            return Err("Unsupported player import catalog version".into());
        }
        let region = text(&document["region"], "catalog region")?.to_owned();
        if !matches!(region.as_str(), "cn" | "jp") {
            return Err("Import catalog region must be cn or jp".into());
        }
        let mut tables = HashMap::new();
        for name in [
            "mysekaiFixtures",
            "mysekaiCustomFixtures",
            "mysekaiSites",
            "mysekaiSiteLevels",
            "mysekaiSiteLayouts",
            "mysekaiRankReleases",
        ] {
            let rows = array(&document["tables"][name], name)?;
            let mut entries = HashMap::new();
            for row in rows {
                let id = positive(&row["id"], "master ID")?;
                if entries.insert(id, row.clone()).is_some() {
                    return Err(format!("Duplicate {name} master ID {id}"));
                }
            }
            tables.insert(name.to_owned(), entries);
        }
        let models: Value =
            serde_json::from_str(models).map_err(|e| format!("Model catalog: {e}"))?;
        let entries = models["packages"]
            .as_object()
            .ok_or("Model catalog has no packages")?;
        let packages = entries
            .iter()
            .filter(|(_, entry)| {
                entry["status"].as_str() == Some("exported")
                    && entry["hasFixtureView"].as_bool() == Some(true)
                    && entry["glb"].as_str().is_some()
            })
            .map(|(name, _)| name.clone())
            .collect();
        Ok(Self {
            region,
            tables,
            packages,
            colors: document["colorTextures"].clone(),
        })
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn import(
        &self,
        json: &str,
        requested_region: Option<&str>,
    ) -> Result<ImportedPlayerData, String> {
        if json.len() > MAX_IMPORT_BYTES {
            return Err("Player data exceeds 32 MiB".into());
        }
        let source: Value = serde_json::from_str(json.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("Invalid player JSON: {e}"))?;
        if !source.is_object() {
            return Err("Player data must be a JSON object".into());
        }
        let root = if source.get("data").is_some_and(Value::is_object)
            && source.get("userMysekaiSiteHousingLayouts").is_none()
            && source.get("updatedResources").is_none()
        {
            &source["data"]
        } else {
            &source
        };
        let updated = match root.get("updatedResources") {
            Some(value) if value.is_object() => value,
            Some(_) => return Err("updatedResources must be an object".into()),
            None => root,
        };
        if [
            requested_region,
            root["region"].as_str(),
            source["region"].as_str(),
        ]
        .into_iter()
        .flatten()
        .any(|region| region != self.region)
        {
            return Err(format!(
                "Player region does not match the {} asset catalog; use matching assets",
                self.region
            ));
        }
        let rank_value = [
            root.get("mysekaiRank"),
            updated
                .get("userMysekaiGamedata")
                .and_then(|v| v.get("mysekaiRank")),
            root.get("userMysekaiGamedata")
                .and_then(|v| v.get("mysekaiRank")),
        ]
        .into_iter()
        .flatten()
        .next()
        .ok_or("Player data has no mysekaiRank")?;
        let rank = positive(rank_value, "mysekaiRank")?;
        let sites_value = updated
            .get("userMysekaiSiteHousingLayouts")
            .or_else(|| root.get("userMysekaiSiteHousingLayouts"))
            .ok_or("No userMysekaiSiteHousingLayouts in this file")?;
        let rows = array(sites_value, "userMysekaiSiteHousingLayouts")?;
        if rows.is_empty() {
            return Err("Player data contains no housing sites".into());
        }
        let mut sites = Vec::new();
        let mut site_ids = HashSet::new();
        let mut uids = HashSet::new();
        let mut special_count = 0;
        let mut appearance_count = 0;
        let mut skipped = SkippedFixtures::default();
        for site in rows {
            let id = positive(&site["mysekaiSiteId"], "mysekaiSiteId")?;
            if !site_ids.insert(id) {
                return Err(format!("Duplicate housing site {id}"));
            }
            let master_site = self.row("mysekaiSites", id)?;
            let site_type = text(&master_site["mysekaiSiteType"], "site type")?;
            if !matches!(
                site_type,
                "home_site" | "first_floor" | "second_floor" | "third_floor"
            ) {
                return Err(format!("Site {id} is not a housing site"));
            }
            let level = self.site_level(id, rank)?;
            let level_id = positive(&level["id"], "site level ID")?;
            let floor = self.tables["mysekaiSiteLayouts"]
                .values()
                .find(|row| {
                    row["mysekaiSiteLevelId"].as_u64() == Some(level_id as u64)
                        && row["mysekaiLayoutType"].as_str() == Some("floor")
                })
                .ok_or_else(|| format!("Site {id} has no floor for its released level"))?;
            let width = positive(&floor["width"], "floor width")?;
            let depth = positive(&floor["depth"], "floor depth")?;
            if width > 256 || depth > 256 {
                return Err("Floor exceeds the signed grid domain".into());
            }
            let mut fixtures = Vec::new();
            let mut layers = HashSet::new();
            for group in array(
                &site["mysekaiSiteHousingLayouts"],
                "mysekaiSiteHousingLayouts",
            )? {
                let layer = parse_layout(text(&group["mysekaiLayoutType"], "mysekaiLayoutType")?)?;
                if !layers.insert(layer) {
                    return Err(format!("Site {id} has duplicate layout layers"));
                }
                for array_name in ARRAYS {
                    for fixture in optional_array(group, array_name)? {
                        if uids.len() >= MAX_FIXTURES {
                            return Err("Too many furniture instances in player data".into());
                        }
                        let uid = text(&fixture["mysekaiUniqueId"], "mysekaiUniqueId")?.to_owned();
                        if uid.len() > 256 || !uids.insert(uid.clone()) {
                            return Err("Furniture IDs must be unique, non-empty strings of at most 256 bytes".into());
                        }
                        let custom_id = fixture
                            .get("mysekaiCustomFixtureId")
                            .filter(|v| !v.is_null())
                            .map(|v| positive(v, "mysekaiCustomFixtureId"))
                            .transpose()?;
                        let custom = custom_id
                            .map(|key| self.row("mysekaiCustomFixtures", key))
                            .transpose()?;
                        let fixture_id = fixture
                            .get("mysekaiFixtureId")
                            .filter(|v| v.as_u64() != Some(0))
                            .or_else(|| custom.and_then(|v| v.get("mysekaiFixtureId")))
                            .ok_or("Furniture record has no master fixture ID")?;
                        let fixture_id = positive(fixture_id, "mysekaiFixtureId")?;
                        let master = self.row("mysekaiFixtures", fixture_id)?;
                        let asset = text(&master["assetbundleName"], "fixture assetbundleName")?;
                        if !asset
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                        {
                            return Err("Fixture asset name is not a catalog identifier".into());
                        }
                        let package = format!("mysekai__fixture__{asset}");
                        if !self.packages.contains(&package) {
                            skipped.model_missing.record(fixture_id);
                            continue;
                        }
                        let grid = custom.unwrap_or(&master["gridSize"]);
                        let size = Vector3Int {
                            x: dimension(&grid["width"])? as i32,
                            y: dimension(&grid["height"])? as i32,
                            z: dimension(&grid["depth"])? as i32,
                        };
                        let position = &fixture["position"];
                        let center = GridPosition::new(
                            grid_byte(&position["x"], "position.x")?,
                            grid_byte(&position["y"], "position.y")?,
                            grid_byte(&position["z"], "position.z")?,
                        );
                        // Resolve the authored layout first, then reflect its
                        // occupied cells into the runtime/glTF frame. `position`
                        // is a layout anchor, not a Euclidean point: even spans
                        // and quarter turns need a pivot correction.
                        let source_center = wall_to_site(center, layer, width, depth)?;
                        let source_direction = if let Some(direction) =
                            moly_law::fixture::position::wall_direction(layer)
                        {
                            direction
                        } else {
                            wire_rotation_direction(
                                fixture["rotation"]
                                    .as_i64()
                                    .ok_or("rotation must be an integer number of degrees")?,
                            )?
                        };
                        let (center, direction, layer) =
                            mirror_fixture_layout(source_center, size, source_direction, layer)?;
                        let texture_id = fixture
                            .get("textureId")
                            .filter(|v| !v.is_null())
                            .map(|v| positive(v, "textureId"))
                            .transpose()?
                            .unwrap_or(1);
                        if texture_id != 1
                            && !optional_array(master, "mysekaiFixtureAnotherColors")?
                                .iter()
                                .any(|v| v["textureId"].as_u64() == Some(texture_id as u64))
                        {
                            skipped.texture_missing.record(fixture_id);
                            continue;
                        }
                        if texture_id != 1
                            && self
                                .colors
                                .get(&package)
                                .and_then(|colors| colors.get(texture_id.to_string()))
                                .is_none()
                        {
                            skipped.color_missing.record(fixture_id);
                            continue;
                        }
                        if array_name != "mysekaiFixtures" {
                            special_count += 1;
                        }
                        fixtures.push(ImportedFixture {
                            uid,
                            fixture_id: i32::try_from(fixture_id)
                                .map_err(|_| "Fixture ID exceeds i32")?,
                            package,
                            center,
                            grid_size: size,
                            layout: layer,
                            direction,
                            texture_id,
                            source: fixture.clone(),
                            source_array: array_name.to_owned(),
                        });
                    }
                }
            }
            appearance_count += optional_array(site, "mysekaiFixtureSurfaceAppearances")?.len();
            sites.push(ImportedSite {
                id,
                site_type: site_type.to_owned(),
                level: positive(&level["level"], "site level")?,
                fixtures,
                source: site.clone(),
            });
        }
        sites.sort_by_key(|site| site.id);
        let mut notices = Vec::new();
        if special_count > 0 {
            notices.push(ImportNotice::SpecialFurnitureRetained {
                count: special_count,
            });
        }
        if appearance_count > 0 {
            notices.push(ImportNotice::SurfaceAppearanceRetained {
                count: appearance_count,
            });
        }
        notices.extend(skipped.notices());
        let game_data = updated
            .get("userGamedata")
            .or_else(|| root.get("userGamedata"));
        let player_name = game_data
            .and_then(|v| v["name"].as_str())
            .or_else(|| root["name"].as_str())
            .map(str::to_owned);
        let player_id = game_data
            .and_then(|v| v.get("userId"))
            .or_else(|| root.get("userId"))
            .and_then(opaque_id);
        Ok(ImportedPlayerData {
            rank,
            region: self.region.clone(),
            player_name,
            player_id,
            sites,
            source,
            notices,
        })
    }

    fn row(&self, table: &str, id: u32) -> Result<&Value, String> {
        self.tables[table]
            .get(&id)
            .ok_or_else(|| format!("Missing {table} master ID {id}; update matching region assets"))
    }

    fn site_level(&self, id: u32, rank: u32) -> Result<&Value, String> {
        let released: HashSet<u64> = self.tables["mysekaiRankReleases"]
            .values()
            .filter(|row| {
                row["mysekaiRankRelaseType"].as_str() == Some("mysekai_site_level")
                    && row["mysekaiRank"]
                        .as_u64()
                        .is_some_and(|r| r <= rank as u64)
            })
            .filter_map(|row| row["externalId"].as_u64())
            .collect();
        self.tables["mysekaiSiteLevels"]
            .values()
            .filter(|row| {
                row["mysekaiSiteId"].as_u64() == Some(id as u64)
                    && row["id"].as_u64().is_some_and(|v| released.contains(&v))
            })
            .max_by_key(|row| row["level"].as_u64().unwrap_or(0))
            .ok_or_else(|| format!("Site {id} is not unlocked at Mysekai rank {rank}"))
    }
}

fn positive(value: &Value, label: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v != 0)
        .ok_or_else(|| format!("{label} must be a positive integer"))
}

fn dimension(value: &Value) -> Result<u32, String> {
    positive(value, "fixture grid dimension").and_then(|v| {
        if v <= 127 {
            Ok(v)
        } else {
            Err("Fixture grid dimension exceeds signed grid domain".into())
        }
    })
}

fn grid_byte(value: &Value, label: &str) -> Result<i8, String> {
    value
        .as_i64()
        .and_then(|v| i8::try_from(v).ok())
        .ok_or_else(|| format!("{label} must be an integer in -128..127"))
}

/// Reflect one authored layout through the shared Unity -> glTF X boundary.
///
/// The wire `position` expands through the native direction-dependent placement
/// law before it becomes a world pose. Reflect the closed occupied-cell footprint
/// (`x_cell -> -x_cell - 1`), then solve the mirrored anchor under the target
/// direction/layout. This preserves exact world-space Y/Z and negates world X,
/// including even footprints and rotations at the edge of an even-sized site.
pub fn mirror_fixture_layout(
    source_center: GridPosition,
    size: Vector3Int,
    source_direction: Direction,
    source_layout: u8,
) -> Result<(GridPosition, Direction, u8), String> {
    use moly_law::fixture::position::layout_footprint;

    let (source_min, source_max) =
        layout_footprint(source_center, size, source_direction, source_layout)?;
    let target_layout = mirror_layout(source_layout);
    let target_direction = mirror_direction(source_direction);
    let mirror_cell = |value: i8| -> Result<i8, String> {
        i8::try_from(-(value as i16) - 1)
            .map_err(|_| "Mirrored fixture footprint exceeds signed grid domain".to_owned())
    };
    let expected_min = GridPosition::new(mirror_cell(source_max.x)?, source_min.y, source_min.z);
    let expected_max = GridPosition::new(mirror_cell(source_min.x)?, source_max.y, source_max.z);

    // Placement is translation-equivariant inside the signed-grid domain. A
    // zero-anchor footprint exposes the layout/direction-specific offset, so the
    // target anchor can be solved without guessing a model pivot.
    let zero = GridPosition::new(0, source_center.y, 0);
    let (base_min, _) = layout_footprint(zero, size, target_direction, target_layout)?;
    let x = i16::from(expected_min.x) - i16::from(base_min.x);
    let z = i16::from(expected_min.z) - i16::from(base_min.z);
    let target_center = GridPosition::new(
        i8::try_from(x).map_err(|_| "Mirrored fixture center.x exceeds signed grid domain")?,
        source_center.y,
        i8::try_from(z).map_err(|_| "Mirrored fixture center.z exceeds signed grid domain")?,
    );
    let actual = layout_footprint(target_center, size, target_direction, target_layout)?;
    if actual != (expected_min, expected_max) {
        return Err("Mirrored fixture footprint does not round-trip through placement law".into());
    }
    Ok((target_center, target_direction, target_layout))
}

fn mirror_layout(layout: u8) -> u8 {
    match layout {
        layout_type::WALL_LEFT => layout_type::WALL_RIGHT,
        layout_type::WALL_RIGHT => layout_type::WALL_LEFT,
        _ => layout,
    }
}

fn wire_rotation_direction(rotation: i64) -> Result<Direction, String> {
    if !(-360..=360).contains(&rotation) || rotation % 90 != 0 {
        return Err("Furniture rotation must be a cardinal angle in degrees".into());
    }
    Ok(Direction::from_u8((rotation.rem_euclid(360) / 90) as u8).unwrap())
}

/// Conjugating a yaw by the X reflection reverses its sign.
fn mirror_direction(direction: Direction) -> Direction {
    match direction {
        Direction::Front => Direction::Front,
        Direction::Left => Direction::Right,
        Direction::Back => Direction::Back,
        Direction::Right => Direction::Left,
    }
}

fn text<'a>(value: &'a Value, label: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{label} must be a non-empty string"))
}

fn array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{label} must be an array"))
}

fn optional_array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(v) => array(v, key),
    }
}

fn opaque_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|v| v.to_string()))
}

fn parse_layout(value: &str) -> Result<u8, String> {
    match value {
        "floor" => Ok(layout_type::FLOOR),
        "rug" => Ok(layout_type::RUG),
        "road" => Ok(layout_type::ROAD),
        "wall_front" => Ok(layout_type::WALL_FRONT),
        "wall_back" => Ok(layout_type::WALL_BACK),
        "wall_left" => Ok(layout_type::WALL_LEFT),
        "wall_right" => Ok(layout_type::WALL_RIGHT),
        _ => Err(format!("Unknown housing layout type {value}")),
    }
}

fn wall_to_site(
    position: GridPosition,
    layer: u8,
    width: u32,
    depth: u32,
) -> Result<GridPosition, String> {
    let x = position.x as i32;
    let w = width.div_ceil(2) as i32;
    let d = depth.div_ceil(2) as i32;
    let (x, z) = match layer {
        layout_type::WALL_FRONT => (!x, -d),
        layout_type::WALL_BACK => (x, d),
        layout_type::WALL_LEFT => (-w, x),
        layout_type::WALL_RIGHT => (w, !x),
        _ => return Ok(position),
    };
    Ok(GridPosition::new(
        i8::try_from(x).map_err(|_| "Wall X exceeds signed grid domain")?,
        position.y,
        i8::try_from(z).map_err(|_| "Wall Z exceeds signed grid domain")?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use moly_law::fixture::position::{field_position, layout_footprint};

    fn assert_reflection(
        center: GridPosition,
        size: Vector3Int,
        direction: Direction,
        layout: u8,
    ) -> (GridPosition, Direction, u8) {
        let source_fp = layout_footprint(center, size, direction, layout).unwrap();
        let source_world = field_position(source_fp.0, source_fp.1, center.y, layout).unwrap();
        let target = mirror_fixture_layout(center, size, direction, layout).unwrap();
        let target_fp = layout_footprint(target.0, size, target.1, target.2).unwrap();
        let target_world = field_position(target_fp.0, target_fp.1, target.0.y, target.2).unwrap();

        assert_eq!(
            (target_fp.0.z, target_fp.1.z),
            (source_fp.0.z, source_fp.1.z)
        );
        assert_eq!(target_fp.0.x as i16, -(source_fp.1.x as i16) - 1);
        assert_eq!(target_fp.1.x as i16, -(source_fp.0.x as i16) - 1);
        assert!((target_world[0] + source_world[0]).abs() < 1.0e-6);
        assert!((target_world[1] - source_world[1]).abs() < 1.0e-6);
        assert!((target_world[2] - source_world[2]).abs() < 1.0e-6);
        target
    }

    #[test]
    fn real_home_boundary_fixture_reflects_inside_same_even_floor() {
        // real-home-outside fixture-0425: crane game, wire center (41,0,-45),
        // master 7x5x4, yaw 180. Reflect occupied cells into the scene frame;
        // the Z boundary remains unchanged and the X cells stay in the floor.
        let target = assert_reflection(
            GridPosition::new(41, 0, -45),
            Vector3Int::new(7, 5, 4),
            Direction::Back,
            layout_type::FLOOR,
        );
        let fp = layout_footprint(target.0, Vector3Int::new(7, 5, 4), target.1, target.2).unwrap();
        assert_eq!((fp.0.z, fp.1.z), (-45, -42));
        assert_eq!(target.1, Direction::Back);
    }

    #[test]
    fn housing_wire_quarter_turn_reflects_pose_and_yaw() {
        let target = assert_reflection(
            GridPosition::new(34, 0, -42),
            Vector3Int::new(4, 3, 2),
            Direction::Left,
            layout_type::FLOOR,
        );
        assert_eq!(target.1, Direction::Right);
        assert_eq!(
            mirror_direction(wire_rotation_direction(90).unwrap()),
            Direction::Right
        );
        assert_eq!(
            mirror_direction(wire_rotation_direction(270).unwrap()),
            Direction::Left
        );
        assert!(wire_rotation_direction(45).is_err());
    }

    #[test]
    fn wall_reflection_preserves_front_back_and_swaps_sides() {
        let front_source =
            wall_to_site(GridPosition::new(3, 4, 0), layout_type::WALL_FRONT, 10, 10).unwrap();
        let front = assert_reflection(
            front_source,
            Vector3Int::new(3, 2, 1),
            moly_law::fixture::position::wall_direction(layout_type::WALL_FRONT).unwrap(),
            layout_type::WALL_FRONT,
        );
        assert_eq!(front.2, layout_type::WALL_FRONT);

        let side_source =
            wall_to_site(GridPosition::new(2, 3, 0), layout_type::WALL_LEFT, 10, 10).unwrap();
        let side = assert_reflection(
            side_source,
            Vector3Int::new(2, 2, 1),
            moly_law::fixture::position::wall_direction(layout_type::WALL_LEFT).unwrap(),
            layout_type::WALL_LEFT,
        );
        assert_eq!(side.2, layout_type::WALL_RIGHT);
        assert_eq!(side.1, Direction::Left);
    }

    #[test]
    fn all_cardinal_grid_layouts_round_trip_without_half_cell_drift() {
        for x in [-12, -1, 0, 9] {
            for width in 1..=6 {
                for depth in 1..=5 {
                    let size = Vector3Int::new(width, 3, depth);
                    for value in 0..4 {
                        let direction = Direction::from_u8(value).unwrap();
                        for layout in [layout_type::FLOOR, layout_type::RUG, layout_type::ROAD,
                            layout_type::WALL_FRONT, layout_type::WALL_BACK,
                            layout_type::WALL_LEFT, layout_type::WALL_RIGHT]
                        {
                            let center = GridPosition::new(x, 2, -4);
                            let target = assert_reflection(center, size, direction, layout);
                            assert_eq!(mirror_fixture_layout(target.0, size, target.1, target.2).unwrap(),
                                (center, direction, layout));
                        }
                    }
                }
            }
        }
    }
}
