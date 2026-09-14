//! Cached formal timeline tables and source-qualified fixture clip bindings.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    animation::{AnimatedBy, AnimationClip, AnimationTargetId},
    asset::LoadState,
    gltf::Gltf,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
};
use moly_assets::{json::JsonAsset, scene_state::SourceInactive};
use serde_json::Value;

use super::ProviderPending;
use crate::{
    fixture::FixtureSource,
    fixture_activity_state::{FixtureActivityIdentity, FixtureTarget},
    fixture_activity_timeline::{
        AnimationCoverage, SourceAnimationEvidence, StartTimeline, TimelineAnimationBinding,
        TimelineDefinition, TimelinePackage, TimelinePayload,
    },
};

struct Package {
    text: [String; 3],
    parsed: TimelinePackage,
    definitions: HashMap<String, Arc<TimelineDefinition>>,
}

#[derive(Default)]
pub(super) struct ActivityAssets {
    json: HashMap<String, Handle<JsonAsset>>,
    documents: HashMap<String, (String, Arc<Value>)>,
    gltf: HashMap<String, Handle<Gltf>>,
    packages: HashMap<String, Package>,
}

impl ActivityAssets {
    fn json_text(&mut self, world: &World, path: &str) -> Result<String, ProviderPending> {
        safe_path(path)?;
        let server = world
            .get_resource::<AssetServer>()
            .ok_or_else(|| ProviderPending::new("asset-server", "asset server is not installed"))?;
        let handle = self
            .json
            .entry(path.into())
            .or_insert_with(|| server.load(format!("moly://{path}")));
        if let LoadState::Failed(error) = server.load_state(&*handle) {
            return Err(ProviderPending::new(
                "json-load-failed",
                format!("{path}: {error:?}"),
            ));
        }
        world
            .get_resource::<Assets<JsonAsset>>()
            .and_then(|assets| assets.get(&*handle))
            .map(|asset| asset.0.clone())
            .ok_or_else(|| ProviderPending::new("json-loading", path))
    }

    fn document(&mut self, world: &World, path: &str) -> Result<Arc<Value>, ProviderPending> {
        let text = self.json_text(world, path)?;
        if let Some((old, value)) = self.documents.get(path) {
            if old == &text {
                return Ok(value.clone());
            }
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| ProviderPending::new("json-shape", format!("{path}: {error}")))?;
        let value = Arc::new(value);
        self.documents.insert(path.into(), (text, value.clone()));
        Ok(value)
    }

    pub(super) fn definition(
        &mut self,
        world: &World,
        package: &str,
        prefab: &str,
    ) -> Result<Arc<TimelineDefinition>, ProviderPending> {
        if package.contains('/')
            || package.contains('\\')
            || package.contains(':')
            || !package.starts_with("mysekai__fixture_timeline__")
        {
            return Err(ProviderPending::new(
                "timeline-route",
                "package is not an exact timeline package key",
            ));
        }
        let text = [
            self.json_text(world, &format!("fixture-timeline/tracks/{package}.json"))?,
            self.json_text(world, &format!("fixture-timeline/clips/{package}.json"))?,
            self.json_text(
                world,
                &format!("fixture-timeline/clip-targets/{package}.json"),
            )?,
        ];
        if self
            .packages
            .get(package)
            .is_none_or(|cached| cached.text != text)
        {
            let parsed =
                TimelinePackage::from_jsons(&text[0], &text[1], &text[2]).map_err(|error| {
                    ProviderPending::new("timeline-three-table-join", format!("{package}: {error}"))
                })?;
            self.packages.insert(
                package.into(),
                Package {
                    text,
                    parsed,
                    definitions: HashMap::new(),
                },
            );
        }
        let cached = self.packages.get_mut(package).expect("prepared package");
        if let Some(definition) = cached.definitions.get(prefab) {
            return Ok(definition.clone());
        }
        let definition = cached.parsed.select_prefab(prefab).map_err(|error| {
            ProviderPending::new(
                "exact-timeline-prefab",
                format!("{package}/{prefab}: {error}"),
            )
        })?;
        if definition.package != package {
            return Err(ProviderPending::new(
                "timeline-route",
                "parsed package identity differs from the requested package",
            ));
        }
        cached.definitions.insert(prefab.into(), definition.clone());
        Ok(definition)
    }

    fn source_fixture_clip(
        &mut self,
        world: &World,
        package: &str,
        clip_name: &str,
    ) -> Result<Handle<AnimationClip>, ProviderPending> {
        let index = self.document(world, "fixture-models/index.json")?;
        let row = index
            .get("packages")
            .and_then(Value::as_object)
            .and_then(|packages| packages.get(package))
            .ok_or_else(|| {
                ProviderPending::new(
                    "fixture-source-catalog",
                    format!("source package {package} is absent"),
                )
            })?;
        let file = row.get("glb").and_then(Value::as_str).ok_or_else(|| {
            ProviderPending::new(
                "fixture-source-catalog",
                format!("source package {package} has no GLB"),
            )
        })?;
        if file.contains('/') || file.contains('\\') {
            return Err(ProviderPending::new(
                "fixture-source-catalog",
                "expected a catalog-relative GLB filename",
            ));
        }
        let path = format!("fixture-models/{file}");
        safe_path(&path)?;
        let server = world
            .get_resource::<AssetServer>()
            .ok_or_else(|| ProviderPending::new("asset-server", "asset server is not installed"))?;
        let handle = self
            .gltf
            .entry(path.clone())
            .or_insert_with(|| server.load(format!("moly://{path}")));
        if let LoadState::Failed(error) = server.load_state(&*handle) {
            return Err(ProviderPending::new(
                "fixture-clip-load-failed",
                format!("{path}: {error:?}"),
            ));
        }
        let assets = world
            .get_resource::<Assets<Gltf>>()
            .ok_or_else(|| ProviderPending::new("gltf-assets", "GLTF assets are not installed"))?;
        let gltf = assets
            .get(&*handle)
            .ok_or_else(|| ProviderPending::new("fixture-clip-loading", &path))?;
        gltf.named_animations
            .get(clip_name)
            .cloned()
            .ok_or_else(|| {
                ProviderPending::new(
                    "fixture-source-clip",
                    format!("{package}/{clip_name} has no rendered animation asset"),
                )
            })
    }

    pub(super) fn prepare_fixture_bindings(
        &mut self,
        world: &mut World,
        request: &mut StartTimeline,
    ) -> Result<(), ProviderPending> {
        let mut bindings = Vec::new();
        for track in &request.definition.tracks {
            if track.class != "AnimationTrack" || track.name == "CharacterAnimator" {
                continue;
            }
            for clip in &track.clips {
                let TimelinePayload::Animation { target, .. } = &clip.payload else {
                    return Err(ProviderPending::new(
                        "fixture-animation-payload",
                        "animation track contains a non-animation payload",
                    ));
                };
                let evidence =
                    SourceAnimationEvidence::from_clip_target(target).map_err(|error| {
                        ProviderPending::new("fixture-source-metadata", error.to_string())
                    })?;
                let animation =
                    self.source_fixture_clip(world, &target.target_package, &target.clip_name)?;
                let ids = {
                    let assets =
                        world
                            .get_resource::<Assets<AnimationClip>>()
                            .ok_or_else(|| {
                                ProviderPending::new(
                                    "animation-assets",
                                    "animation assets are not installed",
                                )
                            })?;
                    let clip = assets.get(&animation).ok_or_else(|| {
                        ProviderPending::new("fixture-clip-loading", &target.clip_name)
                    })?;
                    let ids: HashSet<_> = clip
                        .curves()
                        .iter()
                        .filter(|(_, curves)| !curves.is_empty())
                        .map(|(id, _)| *id)
                        .collect();
                    if ids.is_empty() || !clip.duration().is_finite() || clip.duration() <= 0.0 {
                        return Err(ProviderPending::new(
                            "fixture-empty-clip",
                            format!("{} has no actual renderable curves", target.clip_name),
                        ));
                    }
                    ids
                };
                let mut target_sets: HashMap<Entity, HashSet<AnimationTargetId>> = HashMap::new();
                for (entity, id, by) in world
                    .query::<(Entity, &AnimationTargetId, &AnimatedBy)>()
                    .iter(world)
                {
                    if descendant_and_active(world, entity, by.0) {
                        target_sets.entry(by.0).or_default().insert(*id);
                    }
                }
                let animators: Vec<_> = world
                    .query_filtered::<Entity, With<AnimationPlayer>>()
                    .iter(world)
                    .collect();
                let candidates: Vec<_> = animators
                    .into_iter()
                    .filter(|animator| {
                        descendant_and_active(world, *animator, request.fixture)
                            && target_sets
                                .get(animator)
                                .is_some_and(|bound| ids.is_subset(bound))
                    })
                    .collect();
                let [animator] = candidates.as_slice() else {
                    return Err(ProviderPending::new("fixture-animator-binding", format!(
                        "{} has {} actual matching animators in fixture {:?}; no first-match fallback",
                        target.clip_name, candidates.len(), request.fixture,
                    )));
                };
                let animator = *animator;
                let graph = world
                    .get::<AnimationGraphHandle>(animator)
                    .ok_or_else(|| {
                        ProviderPending::new(
                            "fixture-animation-graph",
                            "actual fixture animator has no installed AnimationGraph",
                        )
                    })?
                    .0
                    .clone();
                bindings.push((
                    clip.key.clone(),
                    TimelineAnimationBinding {
                        animator,
                        graph,
                        clip: animation,
                        source: evidence,
                        coverage: AnimationCoverage::sampled_pose(),
                    },
                ));
            }
        }
        request.bindings.animations.extend(bindings);
        Ok(())
    }
}

fn safe_path(path: &str) -> Result<(), ProviderPending> {
    if path.contains(':')
        || path.contains('\\')
        || path.split('/').any(|part| part.is_empty() || part == "..")
    {
        return Err(ProviderPending::new(
            "asset-path",
            "expected an asset-root-relative path",
        ));
    }
    Ok(())
}

fn descendant_and_active(world: &World, mut entity: Entity, root: Entity) -> bool {
    let mut seen = HashSet::new();
    loop {
        if world.get::<SourceInactive>(entity).is_some() || !seen.insert(entity) {
            return false;
        }
        if entity == root {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

pub(super) fn require_live_fixture(
    assets: &mut ActivityAssets,
    world: &World,
    target: &FixtureTarget,
    model_package: &str,
) -> Result<(), ProviderPending> {
    if !world
        .get::<FixtureActivityIdentity>(target.entity)
        .is_some_and(|identity| target.matches(identity) && identity.model_package == model_package)
    {
        return Err(ProviderPending::new(
            "fixture-instance",
            "typed fixture identity changed",
        ));
    }
    if world.get::<SourceInactive>(target.entity).is_some() {
        return Err(ProviderPending::new(
            "fixture-instance",
            "source fixture is inactive",
        ));
    }
    let source = world.get::<FixtureSource>(target.entity).ok_or_else(|| {
        ProviderPending::new("fixture-instance", "placed model GLTF handle is missing")
    })?;
    let index = assets.document(world, "fixture-models/index.json")?;
    let glb = index
        .get("packages")
        .and_then(Value::as_object)
        .and_then(|packages| packages.get(model_package))
        .and_then(|row| row.get("glb"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderPending::new(
                "fixture-instance",
                "instance model package has no formal GLTF route",
            )
        })?;
    let expected = format!("moly://fixture-models/{glb}");
    let actual = world
        .get_resource::<AssetServer>()
        .and_then(|server| server.get_path(&source.0))
        .map(|path| path.to_string());
    if actual.as_deref() != Some(expected.as_str()) {
        return Err(ProviderPending::new(
            "fixture-instance",
            "actual placed GLTF handle does not match its typed model package",
        ));
    }
    if world
        .get_resource::<Assets<Gltf>>()
        .and_then(|assets| assets.get(&source.0))
        .is_none()
    {
        return Err(ProviderPending::new(
            "fixture-instance",
            "placed model GLTF is loading",
        ));
    }
    let instance = world.get::<SceneInstance>(target.entity).ok_or_else(|| {
        ProviderPending::new("fixture-instance", "placed scene has not been instanced")
    })?;
    if !world
        .get_resource::<SceneSpawner>()
        .is_some_and(|spawner| spawner.instance_is_ready(**instance))
    {
        return Err(ProviderPending::new(
            "fixture-instance",
            "placed scene instance is not ready",
        ));
    }
    Ok(())
}
