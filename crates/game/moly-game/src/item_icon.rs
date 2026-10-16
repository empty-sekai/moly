//! MySekai item icons by the client's own load path.
//!
//! The client loads an item's icon through `UITextureLoader.LoadAsync(bundle,
//! resource)` with bundle paths the `AssetBundleNames` getters build:
//! `mysekai/item_preview/{material,item}/` + `iconAssetbundleName` and
//! `mysekai/item_preview/fixture/{assetbundleName}_{id}` (the collect-item
//! notice), `mysekai/thumbnail/{material,item}/` + `iconAssetbundleName`
//! (the inventory cells) and `mysekai/thumbnail/tool/` + the tool's
//! `assetbundleName`, with or without `_t` (the harvest HUD). Each package
//! holds one texture named as its last path segment.
//!
//! The root's `mysekai-item-icons/index.json` maps each such load path to
//! the exported file; [`ItemIcons::texture`] answers a load path and resource
//! with the file's asset path, or why there is none. A root without the index
//! answers every request with that reason, so each consumer refuses by name.

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

const DIRECTORY: &str = "mysekai-item-icons";

#[derive(Resource)]
struct IndexHandle(Handle<JsonAsset>);

/// The icon index of the root.
#[derive(Resource)]
pub(crate) enum ItemIcons {
    /// Load path to (resource, file under the directory).
    Ready(HashMap<String, (String, String)>),
    Absent(String),
}

impl ItemIcons {
    /// The asset path of the texture `LoadAsync(load_path, resource)` loads.
    pub(crate) fn texture(&self, load_path: &str, resource: &str) -> Result<String, String> {
        match self {
            Self::Ready(map) => match map.get(load_path) {
                Some((name, image)) if name == resource => {
                    Ok(format!("moly://{DIRECTORY}/{image}"))
                }
                Some((name, _)) => Err(format!(
                    "the package holds {}, not the requested resource",
                    crate::balloon::ascii_or(name)
                )),
                None => Err(format!("{DIRECTORY}/index.json does not list this package")),
            },
            Self::Absent(reason) => Err(reason.clone()),
        }
    }
}

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(IndexHandle(
        server.load::<JsonAsset>(format!("moly://{DIRECTORY}/index.json")),
    ));
}

fn settle(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    handle: Option<Res<IndexHandle>>,
) {
    let Some(handle) = handle else { return };
    let icons = match server.load_state(&handle.0) {
        LoadState::Loaded => {
            let Some(asset) = json.get(&handle.0) else {
                return;
            };
            match serde_json::from_str::<serde_json::Value>(&asset.0) {
                Ok(index) => match index["icons"].as_object() {
                    Some(entries) => ItemIcons::Ready(
                        entries
                            .iter()
                            .filter_map(|(key, entry)| {
                                Some((
                                    key.clone(),
                                    (
                                        entry["resource"].as_str()?.to_owned(),
                                        entry["image"].as_str()?.to_owned(),
                                    ),
                                ))
                            })
                            .collect(),
                    ),
                    None => ItemIcons::Absent(format!("{DIRECTORY}/index.json has no icons")),
                },
                Err(error) => ItemIcons::Absent(format!("{DIRECTORY}/index.json: {error}")),
            }
        }
        LoadState::Failed(error) => ItemIcons::Absent(format!(
            "{DIRECTORY}/index.json is not on this root ({error})"
        )),
        _ => return,
    };
    match &icons {
        ItemIcons::Ready(map) => info!("[item-icon] {} load paths in {DIRECTORY}", map.len()),
        ItemIcons::Absent(reason) => {
            warn!("[item-icon] no icon index: {reason}; icons are refused by name")
        }
    }
    commands.insert_resource(icons);
    commands.remove_resource::<IndexHandle>();
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(Startup, load).add_systems(Update, settle);
}
