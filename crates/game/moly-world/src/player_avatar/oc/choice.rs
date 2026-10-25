//! A character body chosen by one holder of an avatar skeleton.
//!
//! [`super::OcSelection`] is the device's one choice and dresses the player.
//! [`OcChoice`] is the same choice carried on an entity: once an avatar
//! body's skeleton is in place ([`super::bind::ProxyRest`]), the character
//! body of its unit is loaded as the cast's bodies are, and
//! [`super::bind::attach`] puts it on that skeleton in place of the global
//! choice. Removing the component removes the body and the skeleton shows
//! the global choice again.

use super::source::CharacterBody;
use bevy::asset::{AssetPath, LoadState};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

/// The character body an avatar skeleton wears, by unit id. It wins over
/// [`super::OcSelection`] on the entity it is on.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcChoice(pub u32);

/// The scene loaded for one [`OcChoice`]: its character body and model file.
/// The scene is spawned by [`super::bind::attach`] once the file is in.
#[derive(Clone, Debug)]
pub struct ChosenBody {
    pub body: CharacterBody,
    pub gltf: Handle<Gltf>,
}

/// The unit whose body was requested for one holder.
#[derive(Component, Clone)]
pub struct OcChoiceLoad {
    choice: u32,
}

/// The chosen bodies of the whole world, by unit: one load per unit however
/// many holders wear it.
#[derive(Resource, Default)]
pub struct OcChoiceBodies {
    units: Vec<(u32, ChosenBody)>,
}

impl OcChoiceBodies {
    /// The scene of `unit`'s body, once its file is in with its dependencies.
    pub fn ready(&self, server: &AssetServer, unit: u32) -> Option<&ChosenBody> {
        self.units
            .iter()
            .find(|(chosen, body)| {
                *chosen == unit && server.is_loaded_with_dependencies(&body.gltf)
            })
            .map(|(_, body)| body)
    }
}

/// Update: each `OcChoice` whose avatar skeleton is in place loads its unit's
/// body; a choice of another unit loads again, and a removed choice drops
/// its load.
pub fn request(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut bodies: ResMut<OcChoiceBodies>,
    choices: Query<(Entity, &OcChoice)>,
    loads: Query<(Entity, &OcChoiceLoad)>,
    proxies: Query<(), With<super::bind::ProxyRest>>,
) {
    for (entity, load) in &loads {
        if choices.get(entity).ok().map(|(_, choice)| choice.0) != Some(load.choice) {
            commands.entity(entity).remove::<OcChoiceLoad>();
        }
    }
    // Nothing to load until a choice's skeleton is in and it has no load yet.
    if choices
        .iter()
        .all(|(entity, _)| loads.get(entity).is_ok() || proxies.get(entity).is_err())
    {
        return;
    }
    let Some(rows) = manifest_rows(&server, &jsons) else {
        return;
    };
    for (entity, choice) in &choices {
        if loads.get(entity).is_ok() || proxies.get(entity).is_err() {
            continue;
        }
        let Some((unit, glb, rig)) = rows.iter().find(|(unit, ..)| *unit == choice.0) else {
            continue;
        };
        if !bodies.units.iter().any(|(chosen, _)| *chosen == *unit) {
            let gltf = server
                .load_builder()
                .with_settings(|settings: &mut bevy::gltf::GltfLoaderSettings| {
                    settings.load_materials = moly_assets::residency::GPU_ONLY;
                    settings.load_cameras = false;
                    settings.load_lights = false;
                    settings.load_animations = false;
                })
                .load(moly_assets::character_glb(glb));
            info!("[oc] unit {unit}: character body {glb} for a holder's own skeleton");
            bodies.units.push((
                *unit,
                ChosenBody {
                    body: CharacterBody {
                        unit: *unit,
                        rig_file: rig.clone(),
                        rig: server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig}"))),
                    },
                    gltf,
                },
            ));
        }
        commands.entity(entity).insert(OcChoiceLoad { choice: *unit });
    }
}

/// The character manifest's units in unit order: (unit, model file, rig
/// profile). `None` until the manifest is in; a failed manifest panics at
/// this boundary.
fn manifest_rows(
    server: &AssetServer,
    jsons: &Assets<JsonAsset>,
) -> Option<Vec<(u32, String, String)>> {
    let manifest = server.load::<JsonAsset>(moly_assets::character_manifest());
    if let LoadState::Failed(error) = server.load_state(&manifest) {
        panic!("the character manifest failed to load: {error:?}");
    }
    let json = jsons.get(&manifest)?;
    let manifest = moly_assets::character::CharacterManifest::parse(&json.0)
        .expect("the character manifest did not parse");
    let mut rows: Vec<(u32, String, String)> = manifest
        .units
        .into_iter()
        .filter_map(|row| {
            let glb = row.glb.clone();
            row.unit
                .parse::<u32>()
                .ok()
                .and_then(|code| code.checked_sub(100))
                .map(|unit| (unit, glb, row.rig))
        })
        .collect();
    rows.sort_by_key(|row| row.0);
    Some(rows)
}
