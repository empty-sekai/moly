//! Transport-only, source-scoped loading for the existing fixture controller.
//! Native keeps its original full catalogue. The browser fetches an index,
//! then only the definitions for actual placed gimmick fixtures; the game
//! profile also fetches the placed house's package for its `HouseView` record.
//! Definition compilation, admission, leases, source events and restoration are
//! unchanged.
use super::house_door::HouseCatalog;
use super::*;
use std::collections::HashSet;

const INDEX: &str = "moly://fixture-gimmick/browser-index.json";

#[derive(Resource)]
pub(crate) struct Stream {
    index: Handle<JsonAsset>,
    master: Handle<JsonAsset>,
    source: Option<(String, String)>,
    paths: HashMap<String, String>,
    pending: HashMap<String, Handle<JsonAsset>>,
    initialized: bool,
    legacy: bool,
    fallback_requested: bool,
    error: Option<String>,
    /// Whether this profile drives a house (the game, not a stage).
    houses: bool,
}

pub(super) fn load(commands: &mut Commands, server: &AssetServer, houses: bool) {
    commands.insert_resource(Catalog::default());
    if houses {
        commands.insert_resource(HouseCatalog::default());
    }
    commands.insert_resource(Stream {
        index: server.load(INDEX),
        master: server.load("moly://mysekai-fixtures.json"),
        source: None,
        paths: HashMap::new(),
        pending: HashMap::new(),
        initialized: false,
        legacy: false,
        fallback_requested: false,
        error: None,
        houses,
    });
}

fn parse_index(
    doc: &Value,
    region: &str,
    version: &str,
) -> Result<HashMap<String, String>, String> {
    if doc["schemaVersion"].as_u64() != Some(1)
        || doc["region"].as_str() != Some(region)
        || doc["gameVersion"].as_str() != Some(version)
    {
        return Err(
            "fixture controller index provenance differs from the selected snapshot".into(),
        );
    }
    let mut paths = HashMap::new();
    for row in doc["packages"]
        .as_array()
        .ok_or("fixture controller index packages missing")?
    {
        let name = row["name"]
            .as_str()
            .ok_or("fixture controller package name missing")?;
        let path = row["path"]
            .as_str()
            .ok_or("fixture controller package path missing")?;
        if !path.starts_with("fixture-gimmick/by-package/")
            || !path.ends_with(".json")
            || path.split('/').any(|part| {
                part.is_empty()
                    || part.starts_with('.')
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            })
        {
            return Err("fixture controller package path is outside its source namespace".into());
        }
        if paths.insert(name.to_owned(), path.to_owned()).is_some() {
            return Err("duplicate fixture controller package identity".into());
        }
    }
    Ok(paths)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    stream: Option<ResMut<Stream>>,
    catalog: Option<ResMut<Catalog>>,
    tables: Option<Res<FixtureActivityTables>>,
    homes: Option<Res<crate::entry::house::HomeFixtures>>,
    mut houses: Option<ResMut<HouseCatalog>>,
    fixtures: Query<&FixtureActivityIdentity>,
) {
    let (Some(mut stream), Some(mut catalog), Some(tables)) = (stream, catalog, tables) else {
        return;
    };
    if !stream.initialized {
        if let LoadState::Failed(error) = server.load_state(&stream.index) {
            // Compatibility with old extracted directories. Even this fallback
            // is deferred until a real fixture needs it, never plain dialogue.
            warn!(
                "[fixture-gimmick] compact index unavailable; defer legacy supplier until needed: {error}"
            );
            stream.legacy = true;
            stream.initialized = true;
        } else {
            let (Some(index), Some(master)) = (jsons.get(&stream.index), jsons.get(&stream.master))
            else {
                return;
            };
            let result = (|| {
                let master: Value = serde_json::from_str(&master.0).map_err(|e| e.to_string())?;
                let region = master["region"]
                    .as_str()
                    .ok_or("fixture master region missing")?
                    .to_owned();
                let version = master["gameVersion"]
                    .as_str()
                    .ok_or("fixture master version missing")?
                    .to_owned();
                let index: Value = serde_json::from_str(&index.0).map_err(|e| e.to_string())?;
                Ok::<_, String>((parse_index(&index, &region, &version)?, (region, version)))
            })();
            match result {
                Ok((paths, source)) => {
                    stream.paths = paths;
                    stream.source = Some(source);
                }
                Err(error) => {
                    warn!("[fixture-gimmick] compact index rejected: {error}");
                    stream.error = Some(error);
                }
            }
            stream.initialized = true;
        }
    }
    let needed: HashSet<String> = fixtures
        .iter()
        .filter(|identity| {
            tables
                .fixture_master(identity.master_id)
                .is_some_and(|master| {
                    matches!(master.player_action_type.as_str(), "loop" | "one_shot")
                })
        })
        .map(|identity| identity.model_package.clone())
        .collect();
    // The house door reads the first placed home fixture's record
    // (house_door::find_house), so every placed home package is fetched.
    let house_needed: HashSet<String> = match (stream.houses, homes.as_deref()) {
        (true, Some(homes)) => fixtures
            .iter()
            .filter(|identity| homes.is_home(&identity.model_package) == Ok(true))
            .map(|identity| identity.model_package.clone())
            .collect(),
        _ => HashSet::new(),
    };
    if stream.legacy {
        if !(needed.is_empty() && house_needed.is_empty()) && !stream.fallback_requested {
            commands.insert_resource(CatalogLoad(
                server.load("moly://fixture-gimmick/gimmicks.json"),
            ));
            stream.fallback_requested = true;
        }
        return;
    }
    for name in needed.iter().chain(house_needed.difference(&needed)) {
        if catalog.0.contains_key(name) || stream.pending.contains_key(name) {
            continue;
        }
        let refusal = if let Some(error) = &stream.error {
            error.clone()
        } else if let Some(path) = stream.paths.get(name) {
            let handle = server.load(format!("moly://{path}"));
            stream.pending.insert(name.clone(), handle);
            continue;
        } else {
            "fixture controller metadata is not supplied for this package".to_owned()
        };
        if let Some(houses) = houses
            .as_deref_mut()
            .filter(|_| house_needed.contains(name))
        {
            houses.refuse(name, refusal.clone());
        }
        catalog.0.insert(name.clone(), Err(refusal));
    }
    // A content replacement can drop requests that nobody needs. Active leases
    // retain their Arc<Definition>; cancelling a transfer never resets an owner.
    stream
        .pending
        .retain(|name, _| needed.contains(name) || house_needed.contains(name));
    let mut names: Vec<_> = stream.pending.keys().cloned().collect();
    names.sort();
    for name in names {
        let handle = stream.pending[&name].clone();
        let document = if let LoadState::Failed(error) = server.load_state(&handle) {
            Err(format!("fixture controller package load failed: {error}"))
        } else {
            let Some(asset) = jsons.get(&handle) else {
                continue;
            };
            (|| -> Result<Value, String> {
                let doc: Value = serde_json::from_str(&asset.0).map_err(|e| e.to_string())?;
                let (region, version) = stream
                    .source
                    .as_ref()
                    .ok_or("fixture controller index has no source")?;
                if doc["schemaVersion"].as_u64() != Some(1)
                    || doc["region"].as_str() != Some(region.as_str())
                    || doc["gameVersion"].as_str() != Some(version.as_str())
                    || doc["package"]["name"].as_str() != Some(name.as_str())
                {
                    return Err("fixture controller package provenance/identity mismatch".into());
                }
                Ok(doc)
            })()
        };
        // The one existing compiler remains authoritative.
        let result = document
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|doc| definition(&doc["package"]).map(Arc::new));
        if let Some(houses) = houses
            .as_deref_mut()
            .filter(|_| house_needed.contains(&name))
        {
            match &document {
                Ok(doc) => houses.insert(&name, &doc["package"]),
                Err(error) => houses.refuse(&name, error.clone()),
            }
        }
        if let Err(error) = &result {
            if needed.contains(&name) {
                warn!("[fixture-gimmick] {name} rejected: {error}");
            }
        }
        catalog.0.insert(name.clone(), result);
        stream.pending.remove(&name);
        // Bound main-thread parsing to one selected package per update.
        break;
    }
    if catalog.0.len() > 64 {
        catalog
            .0
            .retain(|name, _| needed.contains(name) || house_needed.contains(name));
    }
}

/// Why the game still waits for `package`'s streamed house record: the index
/// or the package document is in flight. A settled failure is not waited on;
/// the house catalogue names it. `None` wherever no stream supplies houses.
pub(crate) fn house_pending(world: &World, package: &str) -> Option<String> {
    let stream = world
        .get_resource::<Stream>()
        .filter(|stream| stream.houses)?;
    if stream.legacy {
        return (!stream.fallback_requested || world.contains_resource::<CatalogLoad>())
            .then(|| "house controller catalogue is loading".to_owned());
    }
    let settled = stream.initialized
        && world
            .get_resource::<Catalog>()
            .is_some_and(|catalog| catalog.0.contains_key(package));
    (!settled).then(|| format!("{package}: house controller record is loading"))
}

/// [`house_pending`] over the placed home fixtures, for the entry's wait.
pub(crate) fn home_pending(world: &mut World) -> Option<String> {
    if !world
        .get_resource::<Stream>()
        .is_some_and(|stream| stream.houses)
    {
        return None;
    }
    let packages: Vec<String> = world
        .query::<&FixtureActivityIdentity>()
        .iter(world)
        .map(|identity| identity.model_package.clone())
        .collect();
    let homes = world.get_resource::<crate::entry::house::HomeFixtures>()?;
    packages
        .iter()
        .filter(|package| homes.is_home(package) == Ok(true))
        .find_map(|package| house_pending(world, package))
}

/// Resource readiness only, not another eligibility implementation. Ordinary
/// admission still decides whether a loaded controller can play. This keeps a
/// furniture story in Preparing until its selected source payload arrives.
pub(crate) fn ready_for(world: &mut World, ids: &[i32]) -> Result<bool, String> {
    if !world.contains_resource::<Stream>() || ids.is_empty() {
        return Ok(true);
    }
    let identities: Vec<_> = world
        .query::<&FixtureActivityIdentity>()
        .iter(world)
        .filter(|identity| ids.contains(&identity.master_id))
        .cloned()
        .collect();
    let stream = world.resource::<Stream>();
    let Some(tables) = world.get_resource::<FixtureActivityTables>() else {
        return Ok(false);
    };
    let needed: Vec<_> = identities
        .iter()
        .filter(|identity| {
            tables
                .fixture_master(identity.master_id)
                .is_some_and(|master| {
                    matches!(master.player_action_type.as_str(), "loop" | "one_shot")
                })
        })
        .collect();
    if needed.is_empty() {
        return Ok(true);
    }
    if !stream.initialized {
        return Ok(false);
    }
    if stream.legacy {
        return Ok(stream.fallback_requested && !world.contains_resource::<CatalogLoad>());
    }
    let Some(catalog) = world.get_resource::<Catalog>() else {
        return Ok(false);
    };
    package_readiness(
        catalog,
        needed
            .iter()
            .map(|identity| identity.model_package.as_str()),
    )
}

// Transport failure is not a pending animation or frame synchronization. A
// known failed fetch/parse must return through the existing restoration owner
// immediately, not keep the temporary scene alive until a generic timeout.
fn package_readiness<'a>(
    catalog: &Catalog,
    packages: impl Iterator<Item = &'a str>,
) -> Result<bool, String> {
    let mut all_ready = true;
    for name in packages {
        match catalog.0.get(name) {
            Some(Ok(_)) => {}
            Some(Err(reason)) => return Err(reason.clone()),
            None => all_ready = false,
        }
    }
    Ok(all_ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn index() -> Value {
        json!({"schemaVersion":1,"region":"jp","gameVersion":"6.8.1","packages":[{"name":"fixture-source","path":"fixture-gimmick/by-package/abc.json"}]})
    }
    #[test]
    fn index_identity_is_not_numeric_cross_region_guessing() {
        assert_eq!(parse_index(&index(), "jp", "6.8.1").unwrap().len(), 1);
        assert!(parse_index(&index(), "cn", "6.8.1").is_err());
        assert!(parse_index(&index(), "jp", "6.0.0").is_err());
    }
    #[test]
    fn package_paths_and_duplicates_fail_closed() {
        for path in [
            "../cn/gimmick.json",
            "https://other/fixture.json",
            "fixture-gimmick/by-package/../x.json",
            "fixture-gimmick/by-package/a%2fb.json",
        ] {
            let mut doc = index();
            doc["packages"][0]["path"] = json!(path);
            assert!(parse_index(&doc, "jp", "6.8.1").is_err());
        }
        let mut doc = index();
        let row = doc["packages"][0].clone();
        doc["packages"].as_array_mut().unwrap().push(row);
        assert!(parse_index(&doc, "jp", "6.8.1").is_err());
    }
    #[test]
    fn source_failures_are_not_reported_as_indefinite_pending() {
        let mut catalog = Catalog::default();
        assert_eq!(
            package_readiness(&catalog, ["unrequested"].into_iter()),
            Ok(false)
        );
        catalog.0.insert(
            "failed".into(),
            Err("Asset HTTP 503: source package".into()),
        );
        assert!(
            package_readiness(&catalog, ["unrequested", "failed"].into_iter())
                .unwrap_err()
                .contains("503")
        );
        assert!(package_readiness(&catalog, ["failed"].into_iter()).is_err());
        assert_eq!(package_readiness(&catalog, [].into_iter()), Ok(true));
    }
    #[test]
    fn ordinary_empty_world_has_no_gimmick_dependency() {
        let mut world = World::new();
        assert_eq!(ready_for(&mut world, &[]), Ok(true));
    }
}
