//! The completion prefab is instantiated independently (not an EffectManager
//! pool), at the player's position with identity rotation, and destroyed when
//! all systems finish or after the source's four second limit.

use bevy::{gltf::Gltf, prelude::*, world_serialization::WorldAssetRoot};
use moly_assets::json::JsonAsset;
use serde_json::Value;

const PACKAGE: &str = "mysekai__effect__site__harvest__action__fx_aggregation_01";

#[derive(Resource)]
pub struct CompletionAssets {
    particles: Handle<JsonAsset>,
    gltf: Option<Handle<Gltf>>,
    document: Option<Handle<JsonAsset>>,
    failure: Option<String>,
}

#[derive(Component)]
struct Completion {
    started: f64,
    spawned: bool,
    prepared: bool,
}

pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CompletionAssets {
        particles: server.load("moly://fixture-particles-v2/index.json"),
        gltf: None,
        document: None,
        failure: None,
    });
}

pub fn ready(world: &World) -> bool {
    let Some(assets) = world.get_resource::<CompletionAssets>() else {
        return false;
    };
    let server = world.resource::<AssetServer>();
    assets.failure.is_none()
        && assets
            .gltf
            .as_ref()
            .is_some_and(|h| server.is_loaded_with_dependencies(h))
        && assets
            .document
            .as_ref()
            .is_some_and(|h| server.is_loaded_with_dependencies(h))
}

pub fn emit(world: &mut World) -> Option<Entity> {
    if !ready(world) {
        error!("[bulk-harvest] fx_aggregation_01 is not ready");
        return None;
    }
    let position = world
        .query_filtered::<&Transform, With<crate::player::PlayerControlled>>()
        .single(world)
        .ok()?
        .translation;
    let root = world
        .spawn((
            Completion {
                started: world.resource::<Time>().elapsed_secs_f64(),
                spawned: false,
                prepared: false,
            },
            Transform::from_translation(position),
            Visibility::Visible,
        ))
        .id();
    Some(root)
}

pub fn finished(world: &World, root: Entity) -> bool {
    let Some(effect) = world.get::<Completion>(root) else {
        return true;
    };
    if !effect.prepared {
        return false;
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    crate::site_move::descendants(world, root)
        .into_iter()
        .all(|child| {
            let Some(system) = world.get::<crate::uber_particle::FixtureParticleLive>(child) else {
                return true;
            };
            system.0.pool.is_empty()
                && world
                    .get::<crate::weather_fx::fixture::Played>(child)
                    .is_none_or(|p| !p.playing(&system.0, now))
        })
}

pub fn stop(world: &mut World, root: Entity) {
    if world.entities().contains(root) {
        world.despawn(root);
    }
}

pub fn play_sound(world: &mut World, bundle: &str, cue: &str) {
    // The shared ingame sound catalog resolves cue names from their decoded
    // ACBs. Keep the source bundle in the diagnostic receipt.
    info!("[bulk-harvest] completion sound {bundle}/{cue}");
    super::damage::push_se(
        &mut world.resource_mut::<crate::audio::SeRequests>(),
        cue,
        "bulk-harvest-complete",
    );
}

pub fn tick(world: &mut World) {
    let Some(mut assets) = world.remove_resource::<CompletionAssets>() else {
        return;
    };
    let server = world.resource::<AssetServer>().clone();
    if assets.gltf.is_none() && assets.failure.is_none() {
        let json = world.resource::<Assets<JsonAsset>>();
        if let Some(particles) = json.get(&assets.particles) {
            let parsed = (|| -> Result<(String, String), String> {
                let particles: Value =
                    serde_json::from_str(&particles.0).map_err(|e| e.to_string())?;
                let document = particles["packages"][PACKAGE]["file"]
                    .as_str()
                    .ok_or("completion particle document is absent")?;
                Ok((
                    "moly://site-action-effects/fx_aggregation_01/fx_aggregation_01.glb".to_owned(),
                    format!("moly://fixture-particles-v2/{document}"),
                ))
            })();
            match parsed {
                Ok((glb, document)) => {
                    assets.gltf = Some(server.load(glb));
                    assets.document = Some(server.load(document));
                }
                Err(reason) => {
                    warn!("[bulk-harvest] {reason}");
                    assets.failure = Some(reason);
                }
            }
        }
    }
    let roots: Vec<_> = world
        .query_filtered::<Entity, With<Completion>>()
        .iter(world)
        .collect();
    for root in roots {
        let (started, spawned, prepared) = {
            let c = world.get::<Completion>(root).expect("completion");
            (c.started, c.spawned, c.prepared)
        };
        if world.resource::<Time>().elapsed_secs_f64() - started >= 4.0
            || (prepared && finished(world, root))
        {
            stop(world, root);
            continue;
        }
        if !spawned {
            let scene = assets
                .gltf
                .as_ref()
                .and_then(|h| world.resource::<Assets<Gltf>>().get(h))
                .and_then(|g| g.default_scene.clone());
            if let Some(scene) = scene {
                world.entity_mut(root).insert(WorldAssetRoot(scene));
                world
                    .get_mut::<Completion>(root)
                    .expect("completion")
                    .spawned = true;
            }
        }
        if prepared {
            continue;
        }
        let paths = crate::site_move::node_paths(world, root);
        if paths.is_empty() {
            continue;
        }
        let document = assets
            .document
            .as_ref()
            .and_then(|h| world.resource::<Assets<JsonAsset>>().get(h))
            .and_then(|asset| serde_json::from_str::<Value>(&asset.0).ok());
        let Some(document) = document else {
            continue;
        };
        let mut selected = crate::site_move::effects::select_all(&document, &paths);
        match crate::site_move::effects::prepare_skipping_refused(
            world,
            root,
            &document,
            &mut selected,
            "fx_aggregation_01",
        ) {
            crate::site_move::effects::Prepared::Pending => {}
            crate::site_move::effects::Prepared::Refused(reason) => {
                error!("[bulk-harvest] completion particles refused: {reason}");
                stop(world, root);
            }
            crate::site_move::effects::Prepared::Ready(draws) => {
                let authored =
                    crate::site_move::effects::authored_renderers(world, &document, &draws);
                for (draw, enabled) in authored {
                    if let Some(mut visibility) = world.get_mut::<Visibility>(draw) {
                        *visibility = if enabled {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        };
                    }
                }
                for draw in &draws {
                    if let Err(reason) = crate::weather_fx::fixture::play_pending(world, *draw) {
                        error!("[bulk-harvest] completion Play failed: {reason}");
                    }
                }
                world
                    .get_mut::<Completion>(root)
                    .expect("completion")
                    .prepared = true;
            }
        }
    }
    world.insert_resource(assets);
}
