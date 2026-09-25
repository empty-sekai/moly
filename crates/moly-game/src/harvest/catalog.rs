//! Harvest inputs and the catalog joined from them.
//!
//! Inputs: the extracted package index (`site/harvest.json`: 61 packages,
//! each harvestable prefab's view contract and its master rows), the master
//! slices the drop resolution checks ids against, and the masters the server
//! mock draws from (tools, staminas, materials, unavailable spots). An input
//! that fails to load is named once and harvesting stays off; nothing falls
//! back to a default.

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

use super::law::{Stamina, ToolType};
use super::server_mock::{
    HarvestServerMock, MockFixtureRow, MockInputs, MockMaterialRow, MockSpot, MockToolRow,
    UserHarvestMap, UserTool, MOCK_EXCLUDED,
};
use super::ActionInterface;

/// One input document and what it gates.
struct InputDoc {
    path: &'static str,
    gates: &'static str,
    handle: Handle<JsonAsset>,
}

/// Requested inputs; removed once the catalog is built or an input is absent.
#[derive(Resource)]
pub(crate) struct HarvestInputs(Vec<InputDoc>);

/// Inputs that failed to load (named once; harvesting stays off).
#[derive(Resource)]
pub(crate) struct HarvestInputsAbsent(pub(crate) Vec<String>);

const INPUTS: [(&str, &str); 9] = [
    (
        "site/harvest.json",
        "harvest object packages and their master rows",
    ),
    ("mysekai-fixtures.json", "drop rows of fixture type"),
    ("mysekai-blueprints.json", "drop rows of blueprint type"),
    ("mysekai-items.json", "drop rows of item type"),
    (
        "mysekai-music-records.json",
        "drop rows of music-record type",
    ),
    (
        "mysekai-tools.json",
        "tools: power, cool time, durability, level",
    ),
    (
        "mysekai-staminas.json",
        "stamina maximum for the tools and stamina mock",
    ),
    (
        "mysekai-materials.json",
        "the sites each material drops on (HarvestMapMock)",
    ),
    (
        "mysekai-site-harvest-unavailable-spots.json",
        "the unavailable rectangles HarvestMapMock places around",
    ),
];

/// Startup: request every input.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(HarvestInputs(
        INPUTS
            .iter()
            .map(|(path, gates)| InputDoc {
                path,
                gates,
                handle: server.load(bevy::asset::AssetPath::from(format!("moly://{path}"))),
            })
            .collect(),
    ));
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
}

fn parse(asset: &JsonAsset, path: &str) -> serde_json::Value {
    serde_json::from_str(&asset.0).unwrap_or_else(|error| panic!("{path}: not JSON: {error}"))
}

/// A keyed master document (`{version 1, semantics.table, entries}`).
fn keyed_rows<'a>(value: &'a serde_json::Value, table: &str) -> Vec<&'a serde_json::Value> {
    assert_eq!(
        value["version"].as_u64(),
        Some(1),
        "{table}: unsupported master version"
    );
    assert_eq!(
        value["semantics"]["table"].as_str(),
        Some(table),
        "master table identity mismatch"
    );
    let entries = value["entries"]
        .as_object()
        .unwrap_or_else(|| panic!("{table}: no entries"));
    let mut rows: Vec<&serde_json::Value> = entries.values().collect();
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    for (key, row) in entries {
        let id = row["id"]
            .as_i64()
            .unwrap_or_else(|| panic!("{table}: row without id"));
        assert_eq!(
            key.parse::<i64>().ok(),
            Some(id),
            "{table}: entry key differs from row id"
        );
    }
    rows
}

fn ids(value: &serde_json::Value, table: &str) -> HashSet<i64> {
    keyed_rows(value, table)
        .into_iter()
        .map(|row| row["id"].as_i64().expect("checked"))
        .collect()
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
pub(crate) fn material_type_value(word: &str) -> i32 {
    match word {
        "wood" => 0,
        "mineral" => 1,
        "plant" => 2,
        "junk" => 3,
        "game_character" => 4,
        "other" => 5,
        "tone" => 6,
        "birthday_party" => 7,
        other => panic!("material type word {other} is outside the closed set"),
    }
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
    inputs: Option<Res<HarvestInputs>>,
    sites: Option<Res<crate::site::Sites>>,
) {
    let Some(inputs) = inputs else {
        return;
    };
    let mut absent = Vec::new();
    for doc in &inputs.0 {
        match server.load_state(&doc.handle) {
            LoadState::Failed(error) => {
                absent.push(format!("{} ({}): {error}", doc.path, doc.gates))
            }
            LoadState::Loaded => {}
            _ => return,
        }
    }
    if !absent.is_empty() {
        for line in &absent {
            warn!("[harvest] input absent, harvesting stays off: {line}");
        }
        commands.insert_resource(HarvestInputsAbsent(absent));
        commands.remove_resource::<HarvestInputs>();
        return;
    }
    let Some(sites) = sites else {
        return;
    };
    let mut values = Vec::with_capacity(inputs.0.len());
    for doc in &inputs.0 {
        let Some(asset) = json.get(&doc.handle) else {
            return;
        };
        values.push(parse(asset, doc.path));
    }
    let [index, fixtures, blueprints, items, music, tools, staminas, materials, spots]: [serde_json::Value; 9] =
        values.try_into().expect("nine inputs");

    let fixture_ids: HashSet<i64> = fixtures["fixtures"]
        .as_array()
        .expect("fixture slice without fixtures")
        .iter()
        .map(|row| row["id"].as_i64().expect("fixture row without id"))
        .collect();
    let blueprint_ids = ids(&blueprints, "mysekaiBlueprints");
    let item_ids = ids(&items, "mysekaiItems");
    let music_record_ids = ids(&music, "mysekaiMusicRecords");

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

    let tool_defs: Vec<ToolDef> = keyed_rows(&tools, "mysekaiTools")
        .into_iter()
        .map(|row| ToolDef {
            id: row["id"].as_i64().expect("checked"),
            tool_type: ToolType::from_word(row["mysekaiToolType"].as_str().expect("tool type"))
                .unwrap_or_else(|| {
                    panic!(
                        "tool type {:?} is outside the closed set",
                        row["mysekaiToolType"]
                    )
                }),
            level: row["toolLevel"].as_i64().expect("toolLevel") as i32,
            attack_power: row["attackPower"].as_i64().expect("attackPower") as i32,
            cool_time: row["coolTimeMicroSeconds"]
                .as_f64()
                .expect("coolTimeMicroSeconds") as f32,
            max_durability: row["maxDurability"].as_i64().expect("maxDurability") as i32,
            assetbundle: row["assetbundleName"]
                .as_str()
                .expect("assetbundleName")
                .to_owned(),
        })
        .collect();
    let max_normal_stamina = keyed_rows(&staminas, "mysekaiStaminas")
        .into_iter()
        .find(|row| row["mysekaiStaminaType"].as_str() == Some("normal"))
        .and_then(|row| row["maxStamina"].as_i64())
        .expect("no normal stamina row") as i32;
    let mock_materials: Vec<MockMaterialRow> = keyed_rows(&materials, "mysekaiMaterials")
        .into_iter()
        .map(|row| MockMaterialRow {
            id: row["id"].as_i64().expect("checked"),
            material_type: material_type_value(
                row["mysekaiMaterialType"].as_str().expect("material type"),
            ),
            site_ids: row["mysekaiSiteIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_u64().map(|id| id as u32))
                .collect(),
        })
        // A drop needs a model: rows without one never become drop rows.
        .filter(|row| materials_by_id.contains_key(&row.id))
        .collect();
    let mock_spots: Vec<MockSpot> = keyed_rows(&spots, "mysekaiSiteHarvestUnavailableSpots")
        .into_iter()
        .map(|row| MockSpot {
            site_id: row["mysekaiSiteId"].as_u64().expect("site id") as u32,
            start_x: row["startX"].as_i64().expect("startX") as i32,
            start_z: row["startZ"].as_i64().expect("startZ") as i32,
            width: row["width"].as_i64().expect("width") as i32,
            height: row["height"].as_i64().expect("height") as i32,
        })
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
