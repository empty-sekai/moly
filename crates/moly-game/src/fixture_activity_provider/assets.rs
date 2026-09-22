//! Cached formal timeline tables and source-qualified fixture clip bindings.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    animation::{AnimatedBy, AnimationClip, AnimationTargetId},
    asset::LoadState,
    ecs::change_detection::Tick,
    gltf::Gltf,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
};
use moly_assets::{json::JsonAsset, scene_state::SourceInactive};
use serde_json::Value;

use super::ProviderPending;
use crate::asset_cache::AssetCache;
use crate::{
    fixture::FixtureSource,
    fixture_activity_state::{FixtureActivityIdentity, FixtureTarget},
    fixture_activity_timeline::{
        AnimationCoverage, SourceAnimationEvidence, StartTimeline, TimelineAnimationBinding,
        TimelineDefinition, TimelinePackage, TimelinePayload,
    },
};

struct Package {
    /// 校验这份套件时 `Assets<JsonAsset>` 的 `last_changed`。见
    /// [`ActivityAssets::json_generation`]。
    validated_at: Option<Tick>,
    text: [String; 3],
    parsed: TimelinePackage,
    definitions: AssetCache<Arc<TimelineDefinition>, 32>,
}

#[derive(Default)]
pub(super) struct ActivityAssets {
    json: AssetCache<Handle<JsonAsset>, 96>,
    documents: AssetCache<(Option<Tick>, String, Arc<Value>), 96>,
    gltf: AssetCache<Handle<Gltf>, 24>,
    packages: AssetCache<Package, 24>,
}

impl ActivityAssets {
    pub(super) fn cache_counts(&self) -> Value {
        serde_json::json!({"json":self.json.len(),"documents":self.documents.len(),
            "gltf":self.gltf.len(),"packages":self.packages.len()})
    }

    /// Release only lookup ownership which has gone cold. Prepared profiles
    /// retain their parsed definitions and animation/audio handles, so this
    /// cannot invalidate a playing request. A complete unused frame retires
    /// the lookup, never an in-flight request touched by preparation that frame.
    pub(super) fn sweep_lookup_caches(&mut self) -> usize {
        let mut removed = self.json.sweep_unused();
        removed += self.documents.sweep_unused();
        removed += self.gltf.sweep_unused();
        for package in self.packages.values_mut() {
            removed += package.definitions.sweep_unused();
        }
        removed += self.packages.sweep_unused();
        removed
    }
    /// `Assets<JsonAsset>` 最后一次被改动的 tick。
    ///
    /// 缓存条目记下它在**哪个代**校验过；只要这个代没变，就说明期间没有
    /// 任何 json 资产被加载、替换或移除，那条缓存必然仍然成立。这是 O(1)
    /// 的，而下面的慢路径要把整份 json 文本克隆一遍再逐字节比对——命中
    /// 缓存的代价因此和重新解析同量级，实测占稳态帧时约 46%
    /// （`definition` 18.3% + `memcmp` 17% + `json_text` 11.2%）。
    ///
    /// 拿不到资源时返回 `None`，而 `None != None` 的判据写在调用点：一律
    /// 走慢路径。**慢路径原样保留原来的逐字节比对**，所以这条快路径最坏
    /// 情况只是没命中，不可能给出与原来不同的结果。
    fn json_generation(world: &World) -> Option<Tick> {
        world
            .get_resource_ref::<Assets<JsonAsset>>()
            .map(|assets| assets.last_changed())
    }

    /// 两个代是否确定相同。`None`（资源不在）一律判不同。
    fn same_generation(a: Option<Tick>, b: Option<Tick>) -> bool {
        matches!((a, b), (Some(x), Some(y)) if x == y)
    }

    /// 资产里那份 json 文本的**借用**。
    ///
    /// 它以前返回 `String`，于是每次调用都把整份文档克隆一遍——而绝大多数
    /// 调用只是想拿它去和缓存比对。借用不改变任何判定结果。
    fn json_text<'w>(&mut self, world: &'w World, path: &str) -> Result<&'w str, ProviderPending> {
        safe_path(path)?;
        let server = world
            .get_resource::<AssetServer>()
            .ok_or_else(|| ProviderPending::new("asset-server", "asset server is not installed"))?;
        let handle = self
            .json
            .get_or_insert_with(path, || server.load(format!("moly://{path}")));
        if let LoadState::Failed(error) = server.load_state(&*handle) {
            return Err(ProviderPending::new(
                "json-load-failed",
                format!("{path}: {error:?}"),
            ));
        }
        world
            .get_resource::<Assets<JsonAsset>>()
            .and_then(|assets| assets.get(&*handle))
            .map(|asset| asset.0.as_str())
            .ok_or_else(|| ProviderPending::new("json-loading", path))
    }

    fn document(&mut self, world: &World, path: &str) -> Result<Arc<Value>, ProviderPending> {
        // A cached parse still depends on this live JSON generation. Touch its
        // handle too, or retirement would cause a reload on the next tick.
        self.json.get(path);
        let generation = Self::json_generation(world);
        if let Some((validated_at, _, value)) = self.documents.get(path) {
            if Self::same_generation(*validated_at, generation) {
                return Ok(value.clone());
            }
        }
        let text = self.json_text(world, path)?;
        if let Some((_, old, value)) = self.documents.get(path) {
            if old.as_str() == text {
                let value = value.clone();
                self.documents.get_mut(path).expect("cached document").0 = generation;
                return Ok(value);
            }
        }
        let value: Value = serde_json::from_str(text)
            .map_err(|error| ProviderPending::new("json-shape", format!("{path}: {error}")))?;
        let value = Arc::new(value);
        let text = text.to_owned();
        self.documents
            .insert(path.into(), (generation, text, value.clone()));
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
        for kind in ["tracks", "clips", "clip-targets"] {
            self.json.get(&format!("fixture-timeline/{kind}/{package}.json"));
        }
        let generation = Self::json_generation(world);
        // 快路：这份套件在当前代校验过 ⇒ 三份 json 都没被动过，不必再取、
        // 再比。慢路径（下面）原样保留原来的「取三份文本 + 逐份比对」。
        let validated = self
            .packages
            .get(package)
            .is_some_and(|cached| Self::same_generation(cached.validated_at, generation));
        if !validated {
            let text = [
                self.json_text(world, &format!("fixture-timeline/tracks/{package}.json"))?,
                self.json_text(world, &format!("fixture-timeline/clips/{package}.json"))?,
                self.json_text(
                    world,
                    &format!("fixture-timeline/clip-targets/{package}.json"),
                )?,
            ];
            let unchanged = self.packages.get(package).is_some_and(|cached| {
                cached
                    .text
                    .iter()
                    .zip(text.iter())
                    .all(|(old, new)| old.as_str() == *new)
            });
            if unchanged {
                if let Some(cached) = self.packages.get_mut(package) {
                    cached.validated_at = generation;
                }
            } else {
                let parsed =
                    TimelinePackage::from_jsons(text[0], text[1], text[2]).map_err(|error| {
                        ProviderPending::new(
                            "timeline-three-table-join",
                            format!("{package}: {error}"),
                        )
                    })?;
                self.packages.insert(
                    package.into(),
                    Package {
                        validated_at: generation,
                        text: text.map(str::to_owned),
                        parsed,
                        definitions: AssetCache::default(),
                    },
                );
            }
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
        expected: &SourceAnimationEvidence,
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
        if package != expected.package {
            validate_relocated_fixture_clip(row, clip_name, expected)?;
        }
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
            .get_or_insert_with(&path, || server.load(format!("moly://{path}")));
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
            if track.class != "AnimationTrack"
                || (track.name == "CharacterAnimator"
                    || request.bindings.actors.contains_key(&track.identity))
            {
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
                // JP stores some fixture Transform clips in a Timeline
                // AssetBundle. They are exported into the bound fixture's GLB;
                // source package identity is evidence, never a guessed model path.
                let model_package = world
                    .get::<FixtureActivityIdentity>(request.fixture)
                    .ok_or_else(|| {
                        ProviderPending::new(
                            "fixture-instance",
                            "placed fixture identity is unavailable",
                        )
                    })?
                    .model_package
                    .clone();
                let animation =
                    self.source_fixture_clip(world, &model_package, &target.clip_name, &evidence)?;
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

fn validate_relocated_fixture_clip(
    row: &Value,
    clip_name: &str,
    expected: &SourceAnimationEvidence,
) -> Result<(), ProviderPending> {
    let clips = row
        .pointer("/animations/clips")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProviderPending::new(
                "fixture-source-evidence",
                "relocated clip has no exported identity ledger",
            )
        })?;
    let matches: Vec<_> = clips
        .iter()
        .filter(|clip| {
            clip["name"].as_str() == Some(clip_name)
                && clip.pointer("/sourceClip/file").and_then(Value::as_str)
                    == Some(expected.asset.file.as_str())
                && clip.pointer("/sourceClip/pathId").and_then(Value::as_str)
                    == Some(expected.asset.path_id.as_str())
        })
        .collect();
    let [clip] = matches.as_slice() else {
        return Err(ProviderPending::new(
            "fixture-source-evidence",
            "relocated clip does not uniquely match the authored source identity",
        ));
    };
    if clip["sourceStartTime"].as_f64() != Some(expected.start_time)
        || clip["sourceStopTime"].as_f64() != Some(expected.stop_time)
        || clip["sourceLoopTime"].as_bool() != Some(expected.looping)
    {
        return Err(ProviderPending::new(
            "fixture-source-evidence",
            "relocated clip changed authored time bounds or loop state",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod relocated_clip_tests {
    use super::*;
    #[test]
    fn external_fixture_clip_requires_exact_source_identity_and_time_domain() {
        let expected = SourceAnimationEvidence {
            package: "mysekai__fixture_timeline__bike".into(),
            clip_name: "bike_L".into(),
            asset: crate::fixture_activity_timeline::SourceAssetId {
                file: "CAB-source".into(),
                path_id: "7".into(),
            },
            start_time: 0.,
            stop_time: 2.,
            looping: true,
        };
        let mut row = serde_json::json!({"animations":{"clips":[{"name":"bike_L","sourceClip":{"file":"CAB-source","pathId":"7"},
            "sourceStartTime":0.,"sourceStopTime":2.,"sourceLoopTime":true}]}});
        assert!(validate_relocated_fixture_clip(&row, "bike_L", &expected).is_ok());
        row["animations"]["clips"][0]["sourceClip"]["file"] = serde_json::json!("CAB-other");
        assert!(validate_relocated_fixture_clip(&row, "bike_L", &expected).is_err());
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
