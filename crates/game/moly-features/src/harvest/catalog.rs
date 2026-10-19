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
use serde_json::Value;

use super::law::{Stamina, ToolType};
use super::server_mock::{
    HarvestServerMock, MockFixtureRow, MockInputs, MockMaterialRow, UserHarvestMap, UserTool,
    MOCK_EXCLUDED,
};
use super::ActionInterface;

/// The package index and what it gates.
const INDEX: &str = "site/harvest.json";
const INDEX_GATES: &str = "harvest object packages and their master rows";

/// The requested inputs; removed once the catalog is built or an input is
/// absent.
#[derive(Resource)]
pub struct HarvestInputs {
    index: Handle<JsonAsset>,
    /// The master tables, once every one has resolved.
    masters: Option<HarvestMasters>,
}

/// Inputs that failed to load (named once; harvesting stays off).
#[derive(Resource)]
pub struct HarvestInputsAbsent(pub Vec<String>);

// The shared loader requests this table too (its own request type); this
// reader keys its request by its own type, as every consumer does.
const STAMINA_RECOVERY: MasterTable<moly_masters::RecoveryRow> = MasterTable {
    table: "mysekaiStaminaRecovery",
    name: "mysekaiStaminaRecovery (the boost grant of the stamina mock)",
    parse: parse_boost_grant,
};

fn master_keys() -> [MasterKey; 1] {
    [STAMINA_RECOVERY.key()]
}

/// The master tables the catalog and the server mock read.
struct HarvestMasters {
    boost_grant: Result<moly_masters::RecoveryRow, MasterError>,
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
            boost_grant: masters.take(&STAMINA_RECOVERY)?,
        })
    }

    /// The tables this consumer does not get (already named by the master
    /// layer).
    fn absent(&self) -> Vec<String> {
        [self.boost_grant.as_ref().err()]
        .into_iter()
        .flatten()
        .map(MasterError::to_string)
        .collect()
    }
}

/// Startup: request every input.
pub fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut masters: ResMut<MasterData>,
) {
    commands.insert_resource(HarvestInputs {
        index: server.load(bevy::asset::AssetPath::from(format!("moly://{INDEX}"))),
        masters: None,
    });
    masters.request(&STAMINA_RECOVERY);
}

/// The absent line for a table a reader takes from the shared loader.
fn row_absent(shared: &moly_masters::load::MasterTables, table: &str) -> String {
    shared
        .missing
        .iter()
        .find(|line| line.starts_with(table))
        .cloned()
        .unwrap_or_else(|| format!("{table}: not in the runtime root"))
}

/// The ids of a table's rows.
fn parse_ids(rows: &[Value]) -> Result<HashSet<i64>, String> {
    rows.iter().map(|row| master::int(row, "id")).collect()
}

/// A table's rows in id order.
fn rows_by_id(rows: &[Value]) -> Result<Vec<Value>, String> {
    let mut rows = rows.to_vec();
    for row in &rows {
        master::int(row, "id")?;
    }
    rows.sort_by_key(|row| row["id"].as_i64());
    Ok(rows)
}

fn parse_tools(rows: &[Value]) -> Result<Vec<ToolDef>, String> {
    rows_by_id(rows)?
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


/// One boost recovery: the table's single row (`boostQuantity`,
/// `recoveryBoostStamina`), which the stamina mock grants once.
fn parse_boost_grant(text: &str) -> Result<moly_masters::RecoveryRow, String> {
    let row = serde_json::Value::Object(master::object(text)?);
    Ok(moly_masters::RecoveryRow {
        boost_quantity: master::int32(&row, "boostQuantity")?,
        recovery_boost_stamina: master::int32(&row, "recoveryBoostStamina")?,
    })
}


/// `mysekaiMusicRecords`: every id, and the ids by track type.
#[derive(Default)]
pub struct MusicRecords {
    ids: HashSet<i64>,
    by_track_type: std::collections::BTreeMap<String, Vec<i64>>,
}

fn parse_music_records(rows: &[Value]) -> Result<MusicRecords, String> {
    let mut records = MusicRecords {
        ids: HashSet::new(),
        by_track_type: Default::default(),
    };
    for row in rows_by_id(rows)? {
        let id = master::int(&row, "id")?;
        records.ids.insert(id);
        records
            .by_track_type
            .entry(master::text(&row, "mysekaiMusicTrackType")?.to_owned())
            .or_default()
            .push(id);
    }
    Ok(records)
}

/// Files of one exported package.
#[derive(Clone, Debug)]
pub struct PackageFiles {
    pub glb: String,
    pub document: String,
    pub leaf: String,
}

/// One master fixture row joined with its package's view contract.
#[derive(Clone, Debug)]
pub struct FixtureDef {
    pub id: i32,
    pub package: String,
    pub leaf: String,
    pub class: &'static str,
    pub interface: ActionInterface,
    /// The view's serialized `mysekaiSiteHarvestFixtureType` (the model is
    /// built from the view's value, not the master's).
    pub view_type: i32,
    /// The master row's fixture type value (wood 0 ... birthday_plant 9).
    pub master_kind: i32,
    pub radius: f32,
    pub collision_type: i32,
    pub hp: i32,
    pub last_attack_stamina: i32,
    pub is_rare: bool,
    pub assetbundle: String,
}

/// One harvest material row that has a drop model.
#[derive(Clone, Debug)]
pub struct MaterialDef {
    pub package: String,
    pub material_type: i32,
    pub rarity: i32,
}

/// One tool master row.
#[derive(Clone, Debug)]
pub struct ToolDef {
    pub id: i64,
    pub tool_type: ToolType,
    pub level: i32,
    pub attack_power: i32,
    pub cool_time: f32,
    pub max_durability: i32,
    pub assetbundle: String,
    /// `spriteName`: the tool thumbnail's file stem (`<spriteName>_t`).
    pub sprite_name: String,
}

#[derive(Resource)]
pub struct HarvestCatalog {
    pub fixtures: HashMap<i32, FixtureDef>,
    pub materials: HashMap<i64, MaterialDef>,
    pub packages: HashMap<String, PackageFiles>,
    pub fixture_ids: HashSet<i64>,
    pub blueprint_ids: HashSet<i64>,
    pub item_ids: HashSet<i64>,
    pub music_record_ids: HashSet<i64>,
    pub tools: Vec<ToolDef>,
}

impl HarvestCatalog {
    pub fn tool(&self, id: i64) -> Option<&ToolDef> {
        self.tools.iter().find(|tool| tool.id == id)
    }
}

/// The client's user data as the server mock last replied it (the login
/// fetch, then each API reply merged in).
#[derive(Resource)]
pub struct HarvestUserData {
    pub maps: std::collections::BTreeMap<u32, UserHarvestMap>,
    pub tools: Vec<UserTool>,
    pub stamina: Stamina,
    pub materials: std::collections::BTreeMap<i64, i64>,
    /// `userMysekaiTreasureBoxes`.
    pub boxes: Vec<super::server_mock::UserTreasureBox>,
    /// `userMysekaiPhenomena`.
    pub phenomena: Vec<super::learn::UserPhenomenon>,
}

/// Master fixture type words (`MysekaiSiteHarvestFixtureType`).
pub fn fixture_kind_value(word: &str) -> i32 {
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
pub fn material_type_value(word: &str) -> i32 {
    material_type(word)
        .unwrap_or_else(|| panic!("material type word {word} is outside the closed set"))
}

/// `MysekaiMaterialRarityType`.
pub fn material_rarity_value(word: &str) -> i32 {
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
pub fn build(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    inputs: Option<ResMut<HarvestInputs>>,
    mut masters: ResMut<MasterData>,
    stamina: Option<Res<moly_masters::StaminaMax>>,
    master_tables: Option<Res<moly_masters::load::MasterTables>>,
    sites: Option<Res<crate::site::Sites>>,
    server_tools: Option<Res<crate::userdata::client::tools::ClientMysekaiTools>>,
) {
    let Some(mut inputs) = inputs else {
        return;
    };
    let stamina = stamina.as_deref().copied();
    if let Some(max) = stamina {
        crate::userdata::stamina::seat(max);
    }
    if inputs.masters.is_none() {
        inputs.masters = HarvestMasters::take(&mut masters);
    }
    let Some(tables) = &inputs.masters else {
        return;
    };
    // The drop classifier's blueprint and item ids and its material rows come
    // from the shared loader's tables, which the craft readers already
    // requested: the catalog asks for no second copy of them.
    let Some(shared) = master_tables
        .as_deref()
        .filter(|shared| shared.ready() && shared.rows_ready())
    else {
        return;
    };
    let mut absent = tables.absent();
    if stamina.is_none() {
        absent.push(
            "mysekaiStaminas (stamina maximum for the tools and stamina mock): not in the runtime root"
                .to_owned(),
        );
    }
    let blueprint_ids = shared.blueprint_ids.clone().unwrap_or_else(|| {
        absent.push(
            "mysekaiBlueprints (the server's blueprints): not in the runtime root".to_owned(),
        );
        HashSet::new()
    });
    let item_ids = shared.item_ids.clone().unwrap_or_else(|| {
        absent.push(
            "mysekaiItems (the server's white blueprint item): not in the runtime root".to_owned(),
        );
        HashSet::new()
    });
    let mut mock_materials: Vec<MockMaterialRow> = Vec::new();
    match &shared.material_rows {
        Some(rows) => {
            for (id, word, site_ids) in rows {
                let Some(kind) = material_type(word) else {
                    absent.push(format!(
                        "mysekaiMaterials (the server's material types): material type word {word} is outside the closed set"
                    ));
                    break;
                };
                mock_materials.push(MockMaterialRow {
                    id: *id,
                    material_type: kind,
                    site_ids: site_ids.clone(),
                });
            }
        }
        None => absent.push(
            "mysekaiMaterials (the server's material types): not in the runtime root".to_owned(),
        ),
    }
    // The fixture, tool and record rows come from the shared loader too.
    let mut fixture_ids = HashSet::new();
    match shared.rows.get("mysekaiFixtures") {
        Some(rows) => match parse_ids(rows) {
            Ok(ids) => fixture_ids = ids,
            Err(error) => absent.push(format!("mysekaiFixtures: {error}")),
        },
        None => absent.push(row_absent(shared, "mysekaiFixtures")),
    }
    let mut tool_defs = Vec::new();
    match shared.rows.get("mysekaiTools") {
        Some(rows) => match parse_tools(rows) {
            Ok(tools) => tool_defs = tools,
            Err(error) => absent.push(format!("mysekaiTools: {error}")),
        },
        None => absent.push(row_absent(shared, "mysekaiTools")),
    }
    let mut music_records = MusicRecords::default();
    match shared.rows.get("mysekaiMusicRecords") {
        Some(rows) => match parse_music_records(rows) {
            Ok(records) => music_records = records,
            Err(error) => absent.push(format!("mysekaiMusicRecords: {error}")),
        },
        None => absent.push(row_absent(shared, "mysekaiMusicRecords")),
    }
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
    let Some(max_stamina) = stamina else {
        return;
    };
    let Some(sites) = sites else {
        return;
    };
    // The login's tools are the server document's: wait for the join that
    // carries them.
    let Some(server_tools) = server_tools.filter(|tools| tools.revision > 0) else {
        return;
    };
    let Some(asset) = json.get(&inputs.index) else {
        return;
    };
    let index: serde_json::Value =
        serde_json::from_str(&asset.0).unwrap_or_else(|error| panic!("{INDEX}: not JSON: {error}"));
    let Some(HarvestMasters {
        boost_grant: Ok(recovery),
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
        })
        .collect();
    mock_fixtures.sort_by_key(|row| row.id);
    let mock_inputs = MockInputs {
        fixtures: mock_fixtures,
        materials: mock_materials,
        max_normal_stamina: max_stamina.normal,
        boost_grant: recovery.recovery_boost_stamina,
        harvest_sites: harvest_sites.clone(),
        music_records: music_records.by_track_type,
    };
    let mock = HarvestServerMock::new(mock_inputs, crate::userdata::clock::refresh_window_start());
    let (maps, stamina) = mock.login();
    let user_tools = super::server_mock::user_tools(server_tools.rows());
    info!(
        "[harvest] catalog: {} master fixtures on {} packages, {} drop materials, {} tools; HarvestServerMock (the built-in map rules, one map per refresh window) for harvest sites {:?}; left out of the mock map: {}",
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
        "[harvest-mock] tools (the server document's userMysekaiTools): {:?}; stamina mock: normal {} enhance {} boost {}",
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
        music_record_ids: music_records.ids,
        tools: tool_defs,
    });
    commands.remove_resource::<HarvestInputs>();
}

/// Update: the harvest's tool lists follow the client's `userMysekaiTools`
/// copy when a response other than the harvest's own changes it (a craft
/// reply, a panel edit). A copy equal to the list the last harvest reply set
/// leaves the player model alone, so hits the client has counted but not yet
/// sent are not undone.
pub fn follow_server_tools(
    server_tools: Option<Res<crate::userdata::client::tools::ClientMysekaiTools>>,
    user: Option<ResMut<HarvestUserData>>,
    model: Option<ResMut<super::action::HarvestPlayerModel>>,
    mut seen: Local<u64>,
) {
    let (Some(server_tools), Some(mut user), Some(mut model)) = (server_tools, user, model) else {
        return;
    };
    if server_tools.revision == *seen {
        return;
    }
    *seen = server_tools.revision;
    let rows = super::server_mock::user_tools(server_tools.rows());
    if rows == user.tools {
        return;
    }
    info!(
        "[harvest] userMysekaiTools changed by the server: {:?} -> {:?}",
        user.tools
            .iter()
            .map(|tool| (tool.tool_id, tool.quantity, tool.durability))
            .collect::<Vec<_>>(),
        rows.iter()
            .map(|tool| (tool.tool_id, tool.quantity, tool.durability))
            .collect::<Vec<_>>()
    );
    model.refresh_tools(&rows);
    user.tools = rows;
}
