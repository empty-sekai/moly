//! Harvest inputs and the catalog joined from them.
//!
//! Inputs: the extracted package index (`site/harvest.json`: 61 packages,
//! each harvestable prefab's view contract and its master rows), read from
//! the asset root, and the master tables read from the region's master
//! mirror: the slices the drop resolution checks ids against, and the
//! masters the server mock draws from (tools, staminas, materials,
//! unavailable spots). An input that fails to load, or a master table that is
//! absent or malformed, is named once and harvesting stays off; nothing falls
//! back to a default.

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterError, MasterKey, MasterTable};
use moly_assets::json::JsonAsset;

use super::law::{Stamina, ToolType};
use super::server_mock::{
    HarvestServerMock, MockFixtureRow, MockInputs, MockMaterialRow, MockSpot, MockToolRow,
    UserHarvestMap, UserTool, MOCK_EXCLUDED,
};
use super::ActionInterface;

/// The package index and what it gates.
const INDEX: &str = "site/harvest.json";
const INDEX_GATES: &str = "harvest object packages and their master rows";

/// The requested inputs; removed once the catalog is built or an input is
/// absent.
#[derive(Resource)]
pub(crate) struct HarvestInputs {
    index: Handle<JsonAsset>,
    /// The master tables, once every one has resolved.
    masters: Option<HarvestMasters>,
}

/// Inputs that failed to load (named once; harvesting stays off).
#[derive(Resource)]
pub(crate) struct HarvestInputsAbsent(pub(crate) Vec<String>);

const FIXTURE_IDS: MasterTable<HashSet<i64>> = MasterTable {
    table: "mysekaiFixtures",
    name: "mysekaiFixtures (harvest drop rows of fixture type)",
    parse: parse_ids,
};
const BLUEPRINT_IDS: MasterTable<HashSet<i64>> = MasterTable {
    table: "mysekaiBlueprints",
    name: "mysekaiBlueprints (harvest drop rows of blueprint type)",
    parse: parse_ids,
};
const ITEM_IDS: MasterTable<HashSet<i64>> = MasterTable {
    table: "mysekaiItems",
    name: "mysekaiItems (harvest drop rows of item type)",
    parse: parse_ids,
};
const MUSIC_RECORD_IDS: MasterTable<HashSet<i64>> = MasterTable {
    table: "mysekaiMusicRecords",
    name: "mysekaiMusicRecords (harvest drop rows of music-record type)",
    parse: parse_ids,
};
const TOOLS: MasterTable<Vec<ToolDef>> = MasterTable {
    table: "mysekaiTools",
    name: "mysekaiTools (harvest tools: power, cool time, durability, level)",
    parse: parse_tools,
};
const STAMINAS: MasterTable<i32> = MasterTable {
    table: "mysekaiStaminas",
    name: "mysekaiStaminas (stamina maximum for the tools and stamina mock)",
    parse: parse_max_normal_stamina,
};
const MATERIALS: MasterTable<Vec<MockMaterialRow>> = MasterTable {
    table: "mysekaiMaterials",
    name: "mysekaiMaterials (the sites each material drops on, HarvestMapMock)",
    parse: parse_materials,
};
const SPOTS: MasterTable<Vec<MockSpot>> = MasterTable {
    table: "mysekaiSiteHarvestUnavailableSpots",
    name: "mysekaiSiteHarvestUnavailableSpots (the rectangles HarvestMapMock places around)",
    parse: parse_spots,
};
const STAMINA_RECOVERY: MasterTable<i32> = MasterTable {
    table: "mysekaiStaminaRecovery",
    name: "mysekaiStaminaRecovery (the boost grant of the stamina mock)",
    parse: parse_boost_grant,
};

fn master_keys() -> [MasterKey; 9] {
    [
        FIXTURE_IDS.key(),
        BLUEPRINT_IDS.key(),
        ITEM_IDS.key(),
        MUSIC_RECORD_IDS.key(),
        TOOLS.key(),
        STAMINAS.key(),
        MATERIALS.key(),
        SPOTS.key(),
        STAMINA_RECOVERY.key(),
    ]
}

/// The master tables the catalog and the server mock read.
struct HarvestMasters {
    fixture_ids: Result<HashSet<i64>, MasterError>,
    blueprint_ids: Result<HashSet<i64>, MasterError>,
    item_ids: Result<HashSet<i64>, MasterError>,
    music_record_ids: Result<HashSet<i64>, MasterError>,
    tools: Result<Vec<ToolDef>, MasterError>,
    max_normal_stamina: Result<i32, MasterError>,
    materials: Result<Vec<MockMaterialRow>, MasterError>,
    spots: Result<Vec<MockSpot>, MasterError>,
    boost_grant: Result<i32, MasterError>,
}

impl HarvestMasters {
    /// Every table, once every one has resolved.
    fn take(masters: &mut MasterData) -> Option<Self> {
        if master_keys()
            .into_iter()
            .any(|key| !masters.is_resolved(key))
        {
            return None;
        }
        Some(HarvestMasters {
            fixture_ids: masters.take(&FIXTURE_IDS)?,
            blueprint_ids: masters.take(&BLUEPRINT_IDS)?,
            item_ids: masters.take(&ITEM_IDS)?,
            music_record_ids: masters.take(&MUSIC_RECORD_IDS)?,
            tools: masters.take(&TOOLS)?,
            max_normal_stamina: masters.take(&STAMINAS)?,
            materials: masters.take(&MATERIALS)?,
            spots: masters.take(&SPOTS)?,
            boost_grant: masters.take(&STAMINA_RECOVERY)?,
        })
    }

    /// The tables this consumer does not get (already named by the master
    /// layer).
    fn absent(&self) -> Vec<String> {
        [
            self.fixture_ids.as_ref().err(),
            self.blueprint_ids.as_ref().err(),
            self.item_ids.as_ref().err(),
            self.music_record_ids.as_ref().err(),
            self.tools.as_ref().err(),
            self.max_normal_stamina.as_ref().err(),
            self.materials.as_ref().err(),
            self.spots.as_ref().err(),
            self.boost_grant.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .map(MasterError::to_string)
        .collect()
    }
}

/// Startup: request every input.
pub(crate) fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut masters: ResMut<MasterData>,
) {
    commands.insert_resource(HarvestInputs {
        index: server.load(bevy::asset::AssetPath::from(format!("moly://{INDEX}"))),
        masters: None,
    });
    masters.request(&FIXTURE_IDS);
    masters.request(&BLUEPRINT_IDS);
    masters.request(&ITEM_IDS);
    masters.request(&MUSIC_RECORD_IDS);
    masters.request(&TOOLS);
    masters.request(&STAMINAS);
    masters.request(&MATERIALS);
    masters.request(&SPOTS);
    masters.request(&STAMINA_RECOVERY);
}

/// The ids of a master table's rows.
fn parse_ids(text: &str) -> Result<HashSet<i64>, String> {
    master::rows(text)?
        .iter()
        .map(|row| master::int(row, "id"))
        .collect()
}

/// A master table's rows in id order.
fn rows_by_id(text: &str) -> Result<Vec<serde_json::Value>, String> {
    let mut rows = master::rows(text)?;
    for row in &rows {
        master::int(row, "id")?;
    }
    rows.sort_by_key(|row| row["id"].as_i64());
    Ok(rows)
}

fn parse_tools(text: &str) -> Result<Vec<ToolDef>, String> {
    rows_by_id(text)?
        .iter()
        .map(|row| {
            let word = master::text(row, "mysekaiToolType")?;
            Ok(ToolDef {
                id: master::int(row, "id")?,
                tool_type: ToolType::from_word(word)
                    .ok_or_else(|| format!("tool type {word:?} is outside the closed set"))?,
                level: master::int32(row, "toolLevel")?,
                attack_power: master::int32(row, "attackPower")?,
                cool_time: row["coolTimeMicroSeconds"]
                    .as_f64()
                    .ok_or("coolTimeMicroSeconds is not a number")?
                    as f32,
                max_durability: master::int32(row, "maxDurability")?,
                assetbundle: master::text(row, "assetbundleName")?.to_owned(),
                sprite_name: master::text(row, "spriteName")?.to_owned(),
            })
        })
        .collect()
}

/// The lowest-id normal stamina row's maximum.
fn parse_max_normal_stamina(text: &str) -> Result<i32, String> {
    let rows = rows_by_id(text)?;
    let row = rows
        .iter()
        .find(|row| row["mysekaiStaminaType"].as_str() == Some("normal"))
        .ok_or("no normal stamina row")?;
    master::int32(row, "maxStamina")
}

/// One boost recovery: the table's single row grants recoveryBoostStamina.
fn parse_boost_grant(text: &str) -> Result<i32, String> {
    let row = serde_json::Value::Object(master::object(text)?);
    master::int32(&row, "recoveryBoostStamina")
}

fn parse_materials(text: &str) -> Result<Vec<MockMaterialRow>, String> {
    rows_by_id(text)?
        .iter()
        .map(|row| {
            let word = master::text(row, "mysekaiMaterialType")?;
            Ok(MockMaterialRow {
                id: master::int(row, "id")?,
                material_type: material_type(word).ok_or_else(|| {
                    format!("material type word {word} is outside the closed set")
                })?,
                site_ids: row["mysekaiSiteIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_u64().map(|id| id as u32))
                    .collect(),
            })
        })
        .collect()
}

fn parse_spots(text: &str) -> Result<Vec<MockSpot>, String> {
    rows_by_id(text)?
        .iter()
        .map(|row| {
            Ok(MockSpot {
                site_id: row["mysekaiSiteId"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or("mysekaiSiteId is not a site id")?,
                start_x: master::int32(row, "startX")?,
                start_z: master::int32(row, "startZ")?,
                width: master::int32(row, "width")?,
                height: master::int32(row, "height")?,
            })
        })
        .collect()
}

/// Files of one exported package.
#[derive(Clone, Debug)]
pub(crate) struct PackageFiles {
    pub(crate) glb: String,
    pub(crate) document: String,
    pub(crate) leaf: String,
}

/// One master fixture row joined with its package's view contract.
#[derive(Clone, Debug)]
pub(crate) struct FixtureDef {
    pub(crate) id: i32,
    pub(crate) package: String,
    pub(crate) leaf: String,
    pub(crate) class: &'static str,
    pub(crate) interface: ActionInterface,
    /// The view's serialized `mysekaiSiteHarvestFixtureType` (the model is
    /// built from the view's value, not the master's).
    pub(crate) view_type: i32,
    /// The master row's fixture type value (wood 0 ... birthday_plant 9).
    pub(crate) master_kind: i32,
    pub(crate) radius: f32,
    pub(crate) collision_type: i32,
    pub(crate) hp: i32,
    pub(crate) last_attack_stamina: i32,
    pub(crate) is_rare: bool,
    pub(crate) assetbundle: String,
}

/// One harvest material row that has a drop model.
#[derive(Clone, Debug)]
pub(crate) struct MaterialDef {
    pub(crate) package: String,
    pub(crate) material_type: i32,
    pub(crate) rarity: i32,
}

/// One tool master row.
#[derive(Clone, Debug)]
pub(crate) struct ToolDef {
    pub(crate) id: i64,
    pub(crate) tool_type: ToolType,
    pub(crate) level: i32,
    pub(crate) attack_power: i32,
    pub(crate) cool_time: f32,
    pub(crate) max_durability: i32,
    pub(crate) assetbundle: String,
    /// `spriteName`: the tool thumbnail's file stem (`<spriteName>_t`).
    pub(crate) sprite_name: String,
}

#[derive(Resource)]
pub(crate) struct HarvestCatalog {
    pub(crate) fixtures: HashMap<i32, FixtureDef>,
    pub(crate) materials: HashMap<i64, MaterialDef>,
    pub(crate) packages: HashMap<String, PackageFiles>,
    pub(crate) fixture_ids: HashSet<i64>,
    pub(crate) blueprint_ids: HashSet<i64>,
    pub(crate) item_ids: HashSet<i64>,
    pub(crate) music_record_ids: HashSet<i64>,
    pub(crate) tools: Vec<ToolDef>,
}

impl HarvestCatalog {
    pub(crate) fn tool(&self, id: i64) -> Option<&ToolDef> {
        self.tools.iter().find(|tool| tool.id == id)
    }
}

/// The client's user data as the server mock last replied it (the login
/// fetch, then each API reply merged in).
#[derive(Resource)]
pub(crate) struct HarvestUserData {
    pub(crate) maps: std::collections::BTreeMap<u32, UserHarvestMap>,
    pub(crate) tools: Vec<UserTool>,
    pub(crate) stamina: Stamina,
    pub(crate) materials: std::collections::BTreeMap<i64, i64>,
    /// `userMysekaiTreasureBoxes`.
    pub(crate) boxes: Vec<super::server_mock::UserTreasureBox>,
    /// `userMysekaiPhenomena`.
    pub(crate) phenomena: Vec<super::learn::UserPhenomenon>,
}

/// Master fixture type words (`MysekaiSiteHarvestFixtureType`).
pub(crate) fn fixture_kind_value(word: &str) -> i32 {
    match word {
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
        other => panic!("fixture type word {other} is outside the closed set"),
    }
}

/// `MysekaiMaterialType`.
fn material_type(word: &str) -> Option<i32> {
    Some(match word {
        "wood" => 0,
        "mineral" => 1,
        "plant" => 2,
        "junk" => 3,
        "game_character" => 4,
        "other" => 5,
        "tone" => 6,
        "birthday_party" => 7,
        _ => return None,
    })
}

/// `MysekaiMaterialType` of a package index row.
pub(crate) fn material_type_value(word: &str) -> i32 {
    material_type(word)
        .unwrap_or_else(|| panic!("material type word {word} is outside the closed set"))
}

/// `MysekaiMaterialRarityType`.
pub(crate) fn material_rarity_value(word: &str) -> i32 {
    match word {
        "rarity_1" => 0,
        "rarity_2" => 1,
        "rarity_3" => 2,
        "rarity_4" => 3,
        other => panic!("material rarity word {other} is outside the closed set"),
    }
}

/// Update: once every input is loaded, join the catalog, build the server
/// mock panel and take the login fetch. An absent input is named once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    inputs: Option<ResMut<HarvestInputs>>,
    mut masters: ResMut<MasterData>,
    sites: Option<Res<crate::site::Sites>>,
) {
    let Some(mut inputs) = inputs else {
        return;
    };
    if inputs.masters.is_none() {
        inputs.masters = HarvestMasters::take(&mut masters);
    }
    let Some(tables) = &inputs.masters else {
        return;
    };
    let mut absent = tables.absent();
    match server.load_state(&inputs.index) {
        LoadState::Failed(error) => {
            let line = format!("{INDEX} ({INDEX_GATES}): {error}");
            warn!("[harvest] input absent, harvesting stays off: {line}");
            absent.push(line);
        }
        LoadState::Loaded => {}
        _ => return,
    }
    if !absent.is_empty() {
        info!(
            "[harvest] harvesting stays off: {} input(s) absent",
            absent.len()
        );
        commands.insert_resource(HarvestInputsAbsent(absent));
        commands.remove_resource::<HarvestInputs>();
        return;
    }
    let Some(sites) = sites else {
        return;
    };
    let Some(asset) = json.get(&inputs.index) else {
        return;
    };
    let index: serde_json::Value =
        serde_json::from_str(&asset.0).unwrap_or_else(|error| panic!("{INDEX}: not JSON: {error}"));
    let Some(HarvestMasters {
        fixture_ids: Ok(fixture_ids),
        blueprint_ids: Ok(blueprint_ids),
        item_ids: Ok(item_ids),
        music_record_ids: Ok(music_record_ids),
        tools: Ok(tool_defs),
        max_normal_stamina: Ok(max_normal_stamina),
        materials: Ok(mock_materials),
        spots: Ok(mock_spots),
        boost_grant: Ok(boost_grant),
    }) = inputs.masters.take()
    else {
        return;
    };

    let packages_value = index["packages"]
        .as_object()
        .expect("harvest index without packages");
    let mut packages = HashMap::new();
    let mut defs = HashMap::new();
    let mut materials_by_id = HashMap::new();
    for (key, entry) in packages_value {
        let status = entry["status"].as_str().unwrap_or("");
        if status == "no-mesh" {
            continue;
        }
        assert_eq!(
            status, "exported",
            "harvest package not exported: {key} ({status})"
        );
        let glb = entry["glb"]
            .as_str()
            .unwrap_or_else(|| panic!("{key}: no glb"))
            .to_owned();
        let document = entry["document"]
            .as_str()
            .unwrap_or_else(|| panic!("{key}: no document"))
            .to_owned();
        let leaf = glb
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".glb"))
            .unwrap_or_else(|| panic!("{key}: glb is not a .glb path"))
            .to_owned();
        packages.insert(
            key.clone(),
            PackageFiles {
                glb,
                document,
                leaf: leaf.clone(),
            },
        );
        for row in entry["materialRows"].as_array().into_iter().flatten() {
            let id = row["id"]
                .as_i64()
                .unwrap_or_else(|| panic!("{key}: material row without id"));
            let previous = materials_by_id.insert(
                id,
                MaterialDef {
                    package: key.clone(),
                    material_type: material_type_value(
                        row["mysekaiMaterialType"].as_str().expect("material type"),
                    ),
                    rarity: material_rarity_value(
                        row["mysekaiMaterialRarityType"]
                            .as_str()
                            .expect("material rarity"),
                    ),
                },
            );
            assert!(
                previous.is_none(),
                "material row {id} appears in two packages"
            );
        }
        let Some(view) = entry["view"].as_array() else {
            continue;
        };
        assert_eq!(view.len(), 1, "{key}: view is not one contract");
        let contract = &view[0];
        let class_word = contract["class"]
            .as_str()
            .unwrap_or_else(|| panic!("{key}: view without class"));
        let class = super::VIEW_CLASSES
            .iter()
            .copied()
            .find(|known| *known == class_word)
            .unwrap_or_else(|| panic!("{key}: view class {class_word} is outside the closed set"));
        let interface = super::interface_of(class).expect("closed set");
        for master in entry["masterRows"].as_array().into_iter().flatten() {
            let id = master["id"].as_i64().expect("master row id") as i32;
            let def = FixtureDef {
                id,
                package: key.clone(),
                leaf: leaf.clone(),
                class,
                interface,
                view_type: contract["mysekaiSiteHarvestFixtureType"]
                    .as_i64()
                    .expect("view fixture type") as i32,
                master_kind: fixture_kind_value(
                    master["mysekaiSiteHarvestFixtureType"]
                        .as_str()
                        .expect("master fixture type"),
                ),
                radius: contract["radius"].as_f64().expect("view radius") as f32,
                collision_type: contract["collisionType"]
                    .as_i64()
                    .expect("view collision type") as i32,
                hp: master["hp"].as_i64().expect("master hp") as i32,
                last_attack_stamina: master["lastAttackStamina"]
                    .as_i64()
                    .expect("lastAttackStamina") as i32,
                // IsRareHarvestFixture: master rarity above rarity_1.
                is_rare: master["mysekaiSiteHarvestFixtureRarityType"]
                    .as_str()
                    .expect("rarity")
                    != "rarity_1",
                assetbundle: master["assetbundleName"]
                    .as_str()
                    .unwrap_or(&leaf)
                    .to_owned(),
            };
            assert!(
                defs.insert(id, def).is_none(),
                "master fixture {id} appears twice"
            );
        }
    }

    // A drop needs a model: rows without one never become drop rows.
    let mock_materials: Vec<MockMaterialRow> = mock_materials
        .into_iter()
        .filter(|row| materials_by_id.contains_key(&row.id))
        .collect();
    let harvest_sites: Vec<u32> = sites
        .switchable()
        .filter_map(|(site_type, _)| sites.placement(site_type))
        .filter(|placement| placement.category == "harvest")
        .map(|placement| placement.site_id)
        .collect();
    let mut mock_fixtures: Vec<MockFixtureRow> = defs
        .values()
        .map(|def| MockFixtureRow {
            id: def.id,
            kind: def.master_kind,
            hp: def.hp,
            rare: def.is_rare,
        })
        .collect();
    mock_fixtures.sort_by_key(|row| row.id);
    let mock_inputs = MockInputs {
        fixtures: mock_fixtures,
        materials: mock_materials,
        spots: mock_spots,
        tools: tool_defs
            .iter()
            .map(|tool| MockToolRow {
                id: tool.id,
                tool_type: tool.tool_type,
                level: tool.level,
                max_durability: tool.max_durability,
            })
            .collect(),
        max_normal_stamina,
        boost_grant,
        harvest_sites: harvest_sites.clone(),
        toolbox_blueprint: blueprint_ids.iter().copied().min(),
    };
    let mock = HarvestServerMock::new(&mock_inputs);
    let (maps, user_tools, stamina) = mock.login();
    info!(
        "[harvest] catalog: {} master fixtures on {} packages, {} drop materials, {} tools; HarvestServerMock (the mock's own placement rule, not the source's) for harvest sites {:?}; left out of the mock map: {}",
        defs.len(),
        packages.len(),
        materials_by_id.len(),
        tool_defs.len(),
        harvest_sites,
        MOCK_EXCLUDED
            .iter()
            .map(|(kind, why)| format!("{kind} ({why})"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    for (site_id, map) in &maps {
        let mut kinds: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
        for fixture in &map.fixtures {
            *kinds
                .entry(defs[&fixture.fixture_id].master_kind)
                .or_default() += 1;
        }
        info!(
            "[harvest-mock] HarvestMapMock site {site_id}: {} fixtures (by kind {:?}), {} drop rows",
            map.fixtures.len(),
            kinds,
            map.drops.len()
        );
    }
    info!(
        "[harvest-mock] tools mock: {:?}; stamina mock: normal {} enhance {} boost {}",
        user_tools
            .iter()
            .map(|tool| format!(
                "id {} qty {} durability {}",
                tool.tool_id, tool.quantity, tool.durability
            ))
            .collect::<Vec<_>>(),
        stamina.normal,
        stamina.enhance,
        stamina.boost
    );
    commands.insert_resource(super::action::HarvestPlayerModel::from_login(
        stamina,
        &user_tools,
    ));
    commands.insert_resource(HarvestUserData {
        maps,
        tools: user_tools,
        stamina,
        materials: Default::default(),
        boxes: mock.treasure_boxes(),
        phenomena: mock.phenomena(),
    });
    commands.insert_resource(mock);
    commands.insert_resource(HarvestCatalog {
        fixtures: defs,
        materials: materials_by_id,
        packages,
        fixture_ids,
        blueprint_ids,
        item_ids,
        music_record_ids,
        tools: tool_defs,
    });
    commands.remove_resource::<HarvestInputs>();
}
