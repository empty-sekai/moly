//! The player's house on the offline HOME layout: a named server-decided mock.
//!
//! In the source the house is a placed system fixture in the user's saved
//! housing layout, which the server supplies; the home site always has one
//! (the house cannot be cleaned up in the layout editor, see
//! [`HomeFixtures::can_clean_up`]). A new user's house choice is server data
//! too, so this mock takes the lowest `home` system fixture
//! (`mysekaiSystemFixtures` row 1 names fixture 1, `mdl_mis0001_house_house1`,
//! 12 x 12 x 13 cells, one colour). Its footprint is chosen free of every
//! starter row in both regions; the fixture layout owner appends it at
//! restore time to any home site layout, starter or saved, that has no house,
//! and checks that choice against the rows it actually loaded. Facing Back
//! puts the door toward +Z, so the camera yaw the entry leaves behind equals
//! the Normal camera's initial yaw.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::fixture::{Direction, GridPosition};
use serde_json::Value;

pub(crate) struct StarterHouse {
    pub(crate) package: &'static str,
    pub(crate) fixture_id: i32,
    pub(crate) texture_id: u32,
    /// Unrotated (Front) footprint corners, as a saved layout row stores them.
    pub(crate) min: GridPosition,
    pub(crate) max: GridPosition,
    pub(crate) direction: Direction,
}

pub(crate) const STARTER_HOUSE: StarterHouse = StarterHouse {
    package: "mysekai__fixture__mdl_mis0001_house_house1",
    fixture_id: 1,
    texture_id: 1,
    min: GridPosition { x: 4, y: 0, z: -17 },
    max: GridPosition {
        x: 15,
        y: 12,
        z: -6,
    },
    direction: Direction::Back,
};

/// The instance UID the mock house gets when the layout owner appends it.
/// Distinct from the starter's positional UIDs and the editor's
/// `offline-edit-*` ones, so a saved layout never collides with it by
/// construction; a collision is still refused.
pub(crate) fn mock_house_uid(site_id: u32) -> String {
    format!("offline-house-{site_id}")
}

/// Which placed packages are the player's house and which are gates, from
/// the fixture master and the system fixture table.
///
/// `MysekaiFixtureUtility.IsHomeFixture(id)`: the master has a
/// `mysekaiSystemFixtures` row whose type is home (2).
/// `IsGateFixture(id)`: the master's fixture type is gate (5).
/// Placed rows are identified by package (`mysekai__fixture__` +
/// `assetbundleName`), so a package shared by masters of different classes
/// is refused.
#[derive(Resource, Debug)]
pub(crate) struct HomeFixtures {
    table: Result<Tables, String>,
}

#[derive(Debug)]
struct Tables {
    homes: HashSet<String>,
    gates: HashSet<String>,
}

impl HomeFixtures {
    pub(crate) fn build(masters: &Value, system: &Value) -> Result<Self, String> {
        let mut home_ids = HashSet::new();
        for entry in system["entries"]
            .as_object()
            .ok_or("system fixture table has no entries map")?
            .values()
        {
            let kind = entry["mysekaiSystemFixtureType"]
                .as_str()
                .ok_or("system fixture row without mysekaiSystemFixtureType")?;
            if kind == "home" {
                let id = entry["mysekaiFixtureId"]
                    .as_i64()
                    .ok_or("home system fixture row without mysekaiFixtureId")?;
                home_ids.insert(id);
            }
        }
        if home_ids.is_empty() {
            return Err("the system fixture table has no home row".into());
        }
        // package -> (is home, is gate) of every master row naming it.
        let mut classes: HashMap<String, Vec<(i64, bool, bool)>> = HashMap::new();
        let mut starter_master = None;
        for row in masters["fixtures"]
            .as_array()
            .ok_or("fixture master has no fixtures array")?
        {
            let id = row["id"].as_i64().ok_or("fixture master row without id")?;
            let bundle = row["assetbundleName"]
                .as_str()
                .ok_or("fixture master row without assetbundleName")?;
            let kind = row["fixtureType"]
                .as_str()
                .ok_or("fixture master row without fixtureType")?;
            let package = format!("mysekai__fixture__{bundle}");
            if i64::from(STARTER_HOUSE.fixture_id) == id {
                starter_master = Some((package.clone(), row.clone()));
            }
            classes
                .entry(package)
                .or_default()
                .push((id, home_ids.contains(&id), kind == "gate"));
        }
        let mut homes = HashSet::new();
        let mut gates = HashSet::new();
        let mut found = HashSet::new();
        for (package, rows) in &classes {
            let home = rows.iter().any(|(_, home, _)| *home);
            let gate = rows.iter().any(|(_, _, gate)| *gate);
            if (home && !rows.iter().all(|(_, home, _)| *home))
                || (gate && !rows.iter().all(|(_, _, gate)| *gate))
            {
                return Err(format!(
                    "package {package} is shared by masters {rows:?} of different classes; placed rows are identified by package"
                ));
            }
            found.extend(rows.iter().map(|(id, _, _)| *id));
            if home {
                homes.insert(package.clone());
            }
            if gate {
                gates.insert(package.clone());
            }
        }
        if let Some(missing) = home_ids.iter().find(|id| !found.contains(id)) {
            return Err(format!("home system fixture {missing} has no master row"));
        }
        // The named mock must stay the master it names.
        let (package, master) = starter_master.ok_or_else(|| {
            format!(
                "the mock house's master {} is not in the fixture master",
                STARTER_HOUSE.fixture_id
            )
        })?;
        let size = |field: &str| master[field].as_i64();
        let expected = (
            i64::from(STARTER_HOUSE.max.x - STARTER_HOUSE.min.x) + 1,
            i64::from(STARTER_HOUSE.max.y - STARTER_HOUSE.min.y) + 1,
            i64::from(STARTER_HOUSE.max.z - STARTER_HOUSE.min.z) + 1,
        );
        if package != STARTER_HOUSE.package
            || !homes.contains(&package)
            || (size("gridWidth"), size("gridHeight"), size("gridDepth"))
                != (Some(expected.0), Some(expected.1), Some(expected.2))
        {
            return Err(format!(
                "the mock house ({}, master {}, {}x{}x{}) disagrees with the masters' home fixture {package}",
                STARTER_HOUSE.package,
                STARTER_HOUSE.fixture_id,
                expected.0,
                expected.1,
                expected.2
            ));
        }
        Ok(Self {
            table: Ok(Tables { homes, gates }),
        })
    }

    fn failed(error: String) -> Self {
        Self { table: Err(error) }
    }

    fn tables(&self) -> Result<&Tables, String> {
        self.table.as_ref().map_err(Clone::clone)
    }

    /// `IsHomeFixture` of a placed package.
    pub(crate) fn is_home(&self, package: &str) -> Result<bool, String> {
        Ok(self.tables()?.homes.contains(package))
    }

    /// `MysekaiFixtureUtility.CanCleanUp(id)` = not a gate and not the
    /// house. `FloorEditState.UpdateActionSelector` shows the store button
    /// only when it holds and `IsRemoveTarget` skips the others; selecting,
    /// moving and rotating the house stay allowed outside the tutorial.
    /// A table that failed to load answers false.
    pub(crate) fn can_clean_up(&self, package: &str) -> bool {
        self.tables()
            .is_ok_and(|t| !t.homes.contains(package) && !t.gates.contains(package))
    }
}

#[derive(Resource)]
pub(crate) struct HomeTablesRequest {
    masters: Handle<JsonAsset>,
    system: Handle<JsonAsset>,
}

/// Startup: request both tables.
pub(crate) fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(HomeTablesRequest {
        masters: server.load::<JsonAsset>(moly_assets::mysekai_fixtures()),
        system: server.load::<JsonAsset>("moly://mysekai-system-fixtures.json"),
    });
}

/// Update: build [`HomeFixtures`] once both tables are in. A failure is
/// kept as the resource's state, so the home site restore refuses loudly
/// instead of waiting forever.
pub(crate) fn build(world: &mut World) {
    let Some(request) = world.get_resource::<HomeTablesRequest>() else {
        return;
    };
    let (masters, system) = (request.masters.clone(), request.system.clone());
    let server = world.resource::<AssetServer>();
    for handle in [&masters, &system] {
        match server.load_state(handle) {
            bevy::asset::LoadState::Loaded => {}
            bevy::asset::LoadState::Failed(error) => {
                let error = format!("a fixture table failed to load: {error}");
                error!("[entry] player's house tables: {error}");
                world.remove_resource::<HomeTablesRequest>();
                world.insert_resource(HomeFixtures::failed(error));
                return;
            }
            _ => return,
        }
    }
    let jsons = world.resource::<Assets<JsonAsset>>();
    let parse = |handle: &Handle<JsonAsset>| -> Result<Value, String> {
        let text = jsons
            .get(handle)
            .ok_or("a loaded fixture table is not in the asset table")?;
        serde_json::from_str(&text.0)
            .map_err(|error| format!("a fixture table is not JSON: {error}"))
    };
    let built = parse(&masters)
        .and_then(|masters| Ok((masters, parse(&system)?)))
        .and_then(|(masters, system)| HomeFixtures::build(&masters, &system));
    world.remove_resource::<HomeTablesRequest>();
    match built {
        Ok(homes) => {
            let tables = homes.tables().expect("built tables");
            info!(
                "[entry] player's house tables: {} home packages, {} gate packages (CanCleanUp false)",
                tables.homes.len(),
                tables.gates.len()
            );
            world.insert_resource(homes);
        }
        Err(error) => {
            error!("[entry] player's house tables: {error}");
            world.insert_resource(HomeFixtures::failed(error));
        }
    }
}

#[cfg(test)]
mod tests {
    //! `IsHomeFixture` / `IsGateFixture` / `CanCleanUp` on rows shaped like
    //! the exports (values copied from the CN tables): the home test reads
    //! the system fixture type, not the master's `system` fixture type.
    use super::*;
    use serde_json::json;

    fn masters(extra: Vec<Value>) -> Value {
        let mut rows = vec![
            json!({"id": 1, "assetbundleName": "mdl_mis0001_house_house1", "fixtureType": "system",
                   "gridWidth": 12, "gridHeight": 13, "gridDepth": 12}),
            json!({"id": 5, "assetbundleName": "mdl_mis0001_system_chest1", "fixtureType": "system",
                   "gridWidth": 2, "gridHeight": 2, "gridDepth": 2}),
            json!({"id": 6, "assetbundleName": "mdl_mis0001_fixture_bed1", "fixtureType": "normal",
                   "gridWidth": 4, "gridHeight": 3, "gridDepth": 6}),
            json!({"id": 900002, "assetbundleName": "mdl_non0006_gate_lon1", "fixtureType": "gate",
                   "gridWidth": 6, "gridHeight": 6, "gridDepth": 2}),
        ];
        rows.extend(extra);
        json!({ "fixtures": rows })
    }

    fn system() -> Value {
        json!({"entries": {
            "1": {"id": 1, "mysekaiFixtureId": 1, "mysekaiSystemFixtureType": "home"},
            "2": {"id": 2, "mysekaiFixtureId": 5, "mysekaiSystemFixtureType": "chest"}
        }})
    }

    /// A chest is a system fixture but not a home: it can be cleaned up.
    #[test]
    fn only_home_system_fixtures_and_gates_refuse_clean_up() {
        let homes = HomeFixtures::build(&masters(vec![]), &system()).expect("valid tables");
        let package = |bundle: &str| format!("mysekai__fixture__{bundle}");
        assert!(homes.is_home(&package("mdl_mis0001_house_house1")).unwrap());
        assert!(!homes.can_clean_up(&package("mdl_mis0001_house_house1")));
        assert!(!homes.can_clean_up(&package("mdl_non0006_gate_lon1")));
        assert!(homes.can_clean_up(&package("mdl_mis0001_system_chest1")));
        assert!(homes.can_clean_up(&package("mdl_mis0001_fixture_bed1")));
        assert!(!homes
            .is_home(&package("mdl_mis0001_system_chest1"))
            .unwrap());
    }
}
