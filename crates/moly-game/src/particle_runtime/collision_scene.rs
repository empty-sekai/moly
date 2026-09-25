//! The physics scene the weather CollisionModules query: the ground
//! MeshColliders of the installed effects, cooked the way the engine cooks
//! them, at their authored poses, behind the module law's scene boundary.
//!
//! What the scene holds. The engine's weather loader installs, per site, the
//! sky and camera effects (the site's own or the phenomenon's global ones)
//! and the site effect; a common prefab is only ever installed nested inside
//! a site effect, whose own collider record lists its collider. A weather
//! change stops the old effect (every system stops emitting) and destroys its
//! GameObject, colliders included, only when the effect's own destroy delay
//! has run out, so during that window the old effect's colliders and the new
//! one's are in the physics scene together. Beyond the weather effects the
//! scene holds the site package's colliders (layers 0, 4 and 9 by the
//! package census), the placed fixtures' colliders (layer 9, set at
//! placement), and no character collider; the ground layer (3) holds only
//! weather-effect colliders. The admitted collision mask is the ground layer,
//! so the scene composes the ground colliders of every installed effect.
//!
//! The collider export lists every collider of the effect prefabs with its
//! authored mesh, cooking options, controls and the local transforms from the
//! prefab root down to the collider. The site anchor of an installed effect
//! is the identity, so a collider's world pose (in source axes, where the
//! module law works) is its chain:
//!
//! - only enabled colliders on active nodes are in the physics scene;
//! - the chain must be identity rotations and unit scales; its translations
//!   are summed root first, and every zero part of the pose is +0. The engine
//!   queries a collider that has no body at its static actor's pose composed
//!   with the shape's local pose; such a collider never sets that local pose,
//!   so it stays the shape's default identity, and composing an
//!   identity-valued actor pose with it gives +0 for every zero part whatever
//!   sign the actor pose carried (the effect packages hold no body). The BV4
//!   queries choose their world matrix by these bits, so the signs are part
//!   of the answer there;
//! - the default cooking (the BVH33 midphase) and the options 30 (mesh
//!   cleaning, welding, faster simulation and the fast midphase: the BV4
//!   midphase) are ported. A ground collider cooked with other options, or a
//!   convex, trigger or non-mesh one, stays in the scene as a collider the
//!   queries cannot answer against: a plan that installs one refuses its
//!   collision systems by name, and a query that can meet one refuses.
//!
//! The order of several touches. The broadphase returns the colliders in the
//! order its pruner stores them, which depends on the scene's history of
//! additions and removals; that order is not transcribed. The query reports
//! the order unknown whenever the overlap returns more than one collider, and
//! the module law then selects over every order and refuses a lane the order
//! decides.
use moly_law::particle::collision_mesh::{self as law, CookedMesh, Pose};
use moly_law::particle::collision_query::{Candidate, CollisionScene, OverlapQuery, SweepHit, SweepRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

const GROUND_LAYER: u32 = 3;

/// The scene a plan's collision systems query, or why they are refused.
pub(crate) type SceneVerdict = Result<Arc<GroundScene>, String>;

/// The colliders a set of installed effects adds to the physics scene.
pub(crate) struct GroundScene {
    colliders: Vec<GroundCollider>,
    /// Colliders in the physics scene the queries cannot answer against,
    /// with their layer and why.
    unported: Vec<Unported>,
}

struct GroundCollider {
    effect: String,
    node: String,
    geometry: String,
    cooking: u32,
    mesh: Arc<CookedMesh>,
    pose: Pose,
    layer: u32,
    /// World bounds at inflation one: min then max.
    bounds: [f32; 6],
    /// The collider's ordinal in the export. The engine reports runtime
    /// instance ids, which are not package facts; only hit records carry
    /// them and no product output reads them.
    collider_id: i32,
}

#[derive(Clone)]
struct Unported {
    effect: String,
    node: String,
    layer: u32,
    reason: String,
}

impl GroundScene {
    /// What the scene holds, for diagnostics.
    pub(crate) fn describe(&self) -> Value {
        Value::Array(self.colliders.iter().map(|c| serde_json::json!({
            "effect": c.effect, "node": c.node, "geometry": c.geometry, "cooking": c.cooking, "layer": c.layer,
            "triangles": c.mesh.triangle_count(), "vertices": c.mesh.vertex_count(),
            "translation": c.pose.translation(),
        })).chain(self.unported.iter().map(|u| serde_json::json!({
            "effect": u.effect, "node": u.node, "layer": u.layer, "unported": u.reason,
        }))).collect())
    }

    /// The number of ground colliders the queries answer against.
    pub(crate) fn collider_count(&self) -> usize {
        self.colliders.len()
    }

    /// Whether two colliders give the same answer to every query: the same
    /// geometry (the export names it by its content digest) cooked with the
    /// same options, at the same pose bits.
    #[cfg(test)]
    fn equivalent(a: &GroundCollider, b: &GroundCollider) -> bool {
        a.geometry == b.geometry && a.cooking == b.cooking && a.layer == b.layer
            && a.pose.rotation().map(f32::to_bits) == b.pose.rotation().map(f32::to_bits)
            && a.pose.translation().map(f32::to_bits) == b.pose.translation().map(f32::to_bits)
    }

    fn candidate(&self, index: usize) -> Candidate {
        let c = &self.colliders[index];
        Candidate {
            bounds_min: [c.bounds[0], c.bounds[1], c.bounds[2]],
            bounds_max: [c.bounds[3], c.bounds[4], c.bounds[5]],
            is_trigger: false,
            collider_id: c.collider_id,
            body_id: None,
        }
    }
}

/// How the effects being judged are installed.
pub(crate) enum Installation<'a> {
    /// Installed together by one plan (the sky, camera and site effects of a
    /// phenomenon at a site).
    Together(&'a [&'a str]),
    /// A template that is installed only nested inside other effects.
    #[cfg_attr(not(test), allow(dead_code))]
    Template,
}

/// Builds and caches the scenes of one collider export.
pub(crate) struct SceneBuilder {
    document: Result<Value, String>,
    meshes: MeshCache,
    effects: HashMap<String, Result<Arc<GroundScene>, String>>,
    verdicts: HashMap<Vec<String>, SceneVerdict>,
    selected: Option<Vec<String>>,
}

impl SceneBuilder {
    /// `document` is the parsed collider export, or why there is none.
    pub(crate) fn new(document: Result<Value, String>) -> Self {
        Self { document, meshes: HashMap::new(), effects: HashMap::new(), verdicts: HashMap::new(),
            selected: Some(Vec::new()) }
    }

    /// Sets how the effects judged next are installed.
    pub(crate) fn select(&mut self, installation: Installation<'_>) {
        self.selected = match installation {
            Installation::Together(effects) => Some(effects.iter().map(|e| (*e).to_owned()).collect()),
            Installation::Template => None,
        };
    }

    /// The scene the collision systems of `effect` query: the colliders of
    /// every effect installed with it (see [`SceneBuilder::select`]; an
    /// effect outside the selection is judged as installed alone).
    pub(crate) fn for_effect(&mut self, effect: &str) -> SceneVerdict {
        let Some(selected) = self.selected.clone() else {
            return Err("collision scene: a common template is installed only nested inside its site effects, \
                whose own records carry its systems".into());
        };
        if selected.iter().any(|e| e == effect) {
            let names: Vec<&str> = selected.iter().map(String::as_str).collect();
            self.for_installed(&names)
        } else {
            self.for_installed(&[effect])
        }
    }

    /// The scene of the effects installed together: every ground collider of
    /// theirs, in the order given; refused by name when one of them cannot
    /// be answered against.
    pub(crate) fn for_installed(&mut self, effects: &[&str]) -> SceneVerdict {
        let key: Vec<String> = effects.iter().map(|e| (*e).to_owned()).collect();
        if let Some(verdict) = self.verdicts.get(&key) {
            return verdict.clone();
        }
        let mut colliders = Vec::new();
        let mut unported = Vec::new();
        let mut verdict = Ok(());
        for effect in effects {
            match self.effect_colliders(effect) {
                Ok(scene) => {
                    colliders.extend(scene.colliders.iter().map(GroundCollider::share));
                    unported.extend(scene.unported.iter().cloned());
                }
                Err(reason) => {
                    verdict = Err(reason);
                    break;
                }
            }
        }
        let verdict = verdict.and_then(|()| match unported.iter().find(|u| u.layer == GROUND_LAYER) {
            Some(u) => Err(format!("collision scene: effect {} collider {}: {}", u.effect, u.node, u.reason)),
            None => Ok(Arc::new(GroundScene { colliders, unported })),
        });
        self.verdicts.insert(key, verdict.clone());
        verdict
    }

    /// The colliders one effect adds to the physics scene (possibly none).
    pub(crate) fn effect_colliders(&mut self, effect: &str) -> Result<Arc<GroundScene>, String> {
        if let Some(scene) = self.effects.get(effect) {
            return scene.clone();
        }
        let scene = self.build(effect).map(Arc::new).map_err(|reason| format!("collision scene: {reason}"));
        self.effects.insert(effect.to_owned(), scene.clone());
        scene
    }

    fn build(&mut self, effect: &str) -> Result<GroundScene, String> {
        let document = self.document.as_ref().map_err(Clone::clone)?;
        let records = document.get("colliders").and_then(Value::as_array)
            .ok_or("the collider export has no collider list")?;
        let mut scene = GroundScene { colliders: Vec::new(), unported: Vec::new() };
        for (ordinal, c) in records.iter().enumerate() {
            // A common template's collider is listed again by every site
            // effect that nests it; the site effect's record is the installed one.
            if c.get("effect").and_then(Value::as_str) != Some(effect)
                || c.get("variant").and_then(Value::as_str) == Some("common")
                || c.get("enabled").and_then(Value::as_bool) != Some(true)
                || c.get("activeInHierarchy").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let node = c.get("node").and_then(Value::as_str).unwrap_or("?").to_owned();
            let layer = c.get("layer").and_then(Value::as_u64).and_then(|l| u32::try_from(l).ok()).filter(|&l| l < 32)
                .ok_or_else(|| format!("collider {node}: no layer"))?;
            if layer != GROUND_LAYER {
                scene.unported.push(Unported { effect: effect.to_owned(), node, layer,
                    reason: format!("a layer-{layer} collider (only the ground layer's colliders are composed)") });
                continue;
            }
            match ground_collider(&mut self.meshes, document, ordinal, c, effect) {
                Ok(collider) => scene.colliders.push(collider),
                Err(reason) => scene.unported.push(Unported { effect: effect.to_owned(), node, layer, reason }),
            }
        }
        Ok(scene)
    }
}

impl GroundCollider {
    fn share(&self) -> Self {
        Self {
            effect: self.effect.clone(),
            node: self.node.clone(),
            geometry: self.geometry.clone(),
            cooking: self.cooking,
            mesh: self.mesh.clone(),
            pose: self.pose,
            layer: self.layer,
            bounds: self.bounds,
            collider_id: self.collider_id,
        }
    }
}

type MeshCache = HashMap<(String, u32), Result<Arc<CookedMesh>, &'static str>>;

fn ground_collider(meshes: &mut MeshCache, document: &Value, ordinal: usize, c: &Value, effect: &str)
    -> Result<GroundCollider, String> {
    let node = c.get("node").and_then(Value::as_str).unwrap_or("?").to_owned();
    if c.get("component").and_then(Value::as_str) != Some("MeshCollider")
        || c.get("queryShape").and_then(Value::as_str) != Some("triangleMesh")
        || c.get("convex").and_then(Value::as_bool) != Some(false) {
        return Err("only a non-convex MeshCollider (triangle mesh query) is ported".into());
    }
    if c.get("isTrigger").and_then(Value::as_bool) != Some(false) {
        return Err("a trigger collider (the query's trigger interaction is not transcribed)".into());
    }
    let cooking = c.pointer("/cooking/value").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())
        .ok_or("no cooking options")?;
    let chain = c.get("chain").and_then(Value::as_array).filter(|chain| !chain.is_empty())
        .ok_or("no transform chain")?;
    let mut translation: Option<[f32; 3]> = None;
    for link in chain {
        let path = link.get("path").and_then(Value::as_str).unwrap_or("?");
        let rotation = floats::<4>(link.get("rotation")).ok_or_else(|| format!("chain node {path:?} has no rotation"))?;
        if rotation[..3].iter().any(|&v| v != 0.0) || rotation[3] != 1.0 {
            return Err(format!("chain node {path:?} is rotated (only identity chains are ported)"));
        }
        if floats::<3>(link.get("scale")) != Some([1.0; 3]) {
            return Err(format!("chain node {path:?} is scaled (only unit-scale chains are ported)"));
        }
        let position = floats::<3>(link.get("position")).ok_or_else(|| format!("chain node {path:?} has no position"))?;
        translation = Some(match translation {
            None => position,
            Some(t) => [t[0] + position[0], t[1] + position[1], t[2] + position[2]],
        });
    }
    // The composed query pose: a +0 rotation and every zero translation part
    // +0 (see the module notes).
    let translation = translation.unwrap_or([0.0; 3]).map(|v| if v == 0.0 { 0.0 } else { v });
    let pose = Pose::new([0.0, 0.0, 0.0, 1.0], translation).map_err(|refused| refused.0.to_owned())?;
    let geometry = c.get("geometry").and_then(Value::as_str).ok_or("no geometry")?.to_owned();
    let mesh = match meshes.get(&(geometry.clone(), cooking)) {
        Some(mesh) => mesh.clone(),
        None => {
            let cooked = cook_geometry(document, &geometry, cooking);
            meshes.insert((geometry.clone(), cooking), cooked.clone());
            cooked
        }
    };
    let mesh = mesh.map_err(|reason| format!("geometry {geometry} (cooking options {cooking}): {reason}"))?;
    let bounds = law::world_bounds(&mesh, &pose);
    Ok(GroundCollider {
        effect: effect.to_owned(),
        node,
        geometry,
        cooking,
        layer: GROUND_LAYER,
        bounds,
        collider_id: i32::try_from(ordinal).unwrap_or(i32::MAX),
        mesh,
        pose,
    })
}

fn floats<const N: usize>(value: Option<&Value>) -> Option<[f32; N]> {
    let list = value?.as_array().filter(|list| list.len() == N)?;
    let mut out = [0.0f32; N];
    for (slot, v) in out.iter_mut().zip(list) {
        // The export writes each binary32 in full; the nearest binary32 of
        // the parsed value is that value.
        let f = v.as_f64()? as f32;
        if !f.is_finite() {
            return None;
        }
        *slot = f;
    }
    Some(out)
}

fn cook_geometry(document: &Value, geometry: &str, cooking: u32) -> Result<Arc<CookedMesh>, &'static str> {
    let record = document.get("geometry").and_then(|g| g.get(geometry)).ok_or("not in the collider export")?;
    let positions: Vec<[f32; 3]> = record.get("positions").and_then(Value::as_array).ok_or("no positions")?
        .iter().map(|p| floats::<3>(Some(p))).collect::<Option<_>>().ok_or("a position that is not three binary32 values")?;
    let triangles: Vec<[u32; 3]> = record.get("triangles").and_then(Value::as_array).ok_or("no triangles")?
        .iter().map(|t| {
            let list = t.as_array().filter(|list| list.len() == 3)?;
            let mut out = [0u32; 3];
            for (slot, v) in out.iter_mut().zip(list) {
                *slot = u32::try_from(v.as_u64()?).ok()?;
            }
            Some(out)
        }).collect::<Option<_>>().ok_or("a triangle that is not three corner indices")?;
    law::cook(&positions, &triangles, cooking).map(Arc::new).map_err(|refused| refused.0)
}

/// The site's physics scene as the weather host keeps it: the colliders of
/// every installed effect, in the order installed, a stopped effect's until
/// the host destroys that effect. Every installed system's query shares it.
#[derive(Clone, Default)]
pub(crate) struct SiteScene(Arc<RwLock<SiteEntries>>);

#[derive(Default)]
struct SiteEntries {
    next: u64,
    entries: Vec<(u64, Arc<GroundScene>)>,
}

impl SiteScene {
    fn entries(&self) -> std::sync::RwLockReadGuard<'_, SiteEntries> {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn entries_mut(&self) -> std::sync::RwLockWriteGuard<'_, SiteEntries> {
        self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds an installed effect's colliders; the id removes them.
    pub(crate) fn install(&self, scene: Arc<GroundScene>) -> u64 {
        let mut entries = self.entries_mut();
        entries.next += 1;
        let id = entries.next;
        entries.entries.push((id, scene));
        id
    }

    /// Removes a destroyed effect's colliders.
    pub(crate) fn remove(&self, id: u64) {
        self.entries_mut().entries.retain(|(entry, _)| *entry != id);
    }

    pub(crate) fn clear(&self) {
        self.entries_mut().entries.clear();
    }

    /// The ground colliders the scene holds now.
    pub(crate) fn collider_count(&self) -> usize {
        self.entries().entries.iter().map(|(_, scene)| scene.colliders.len()).sum()
    }
}

/// One system's view of the physics scene: the last overlap's colliders
/// (the sweep requests index them) and a refusal the module law reads.
pub(crate) struct GroundQuery {
    scene: SiteScene,
    last: Vec<(Arc<GroundScene>, usize)>,
    refusal: Option<&'static str>,
    order_known: bool,
}

impl GroundQuery {
    /// A fixed scene of these colliders.
    #[cfg(test)]
    pub(crate) fn new(scene: Arc<GroundScene>) -> Self {
        let site = SiteScene::default();
        site.install(scene);
        Self::live(site)
    }

    /// The host's live scene.
    pub(crate) fn live(scene: SiteScene) -> Self {
        Self { scene, last: Vec::new(), refusal: None, order_known: true }
    }

    /// Whether colliders `a` and `b` of the last overlap give the same
    /// answer to every query (for diagnostics).
    #[cfg(test)]
    pub(crate) fn same_answers(&self, a: usize, b: usize) -> bool {
        match (self.last.get(a), self.last.get(b)) {
            (Some((sa, ia)), Some((sb, ib))) => GroundScene::equivalent(&sa.colliders[*ia], &sb.colliders[*ib]),
            _ => false,
        }
    }
}

impl CollisionScene for GroundQuery {
    /// The scene overlap of the module's box: the static colliders whose
    /// layer the mask names and whose triangles the box touches (the scene
    /// query's narrowphase; the pruner's inflated bounds contain every
    /// triangle the box can touch), at most `max_shapes`. A collider the
    /// queries cannot answer against, on a layer the mask names, refuses
    /// the call.
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.last.clear();
        self.refusal = None;
        let entries = self.scene.entries();
        let mut touched = Vec::new();
        for (_, scene) in &entries.entries {
            if scene.unported.iter().any(|u| query.collides_with & (1u32 << u.layer) != 0) {
                self.refusal.get_or_insert("an installed effect's collider on a layer the mask names is not ported");
            }
            for (i, c) in scene.colliders.iter().enumerate() {
                if query.collides_with & (1u32 << c.layer) == 0 {
                    continue;
                }
                match law::overlap_box(&c.mesh, &c.pose, query.center, query.extents) {
                    Ok(true) => touched.push((scene.clone(), i)),
                    Ok(false) => {}
                    Err(refused) => {
                        self.refusal.get_or_insert(refused.0);
                    }
                }
            }
        }
        drop(entries);
        let limit = usize::try_from(query.max_shapes.max(0)).unwrap_or(0);
        self.order_known = touched.len() <= 1;
        if touched.len() > limit && !self.order_known {
            self.refusal.get_or_insert("more touches than the shape limit keeps, in an unknown order");
        }
        touched.truncate(limit);
        self.last = touched;
        self.last.iter().map(|(scene, i)| scene.candidate(*i)).collect()
    }

    /// Every collider the mask names. A collider the queries cannot answer
    /// against stands in with unbounded bounds, so a lane that could reach
    /// it refuses the call instead of skipping it.
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.scene.entries().entries.iter().flat_map(|(_, scene)| {
            let unported = scene.unported.iter().filter(|u| collides_with & (1u32 << u.layer) != 0).map(|_| Candidate {
                bounds_min: [-f32::MAX; 3],
                bounds_max: [f32::MAX; 3],
                is_trigger: false,
                collider_id: -1,
                body_id: None,
            });
            (0..scene.colliders.len())
                .filter(|&i| collides_with & (1u32 << scene.colliders[i].layer) != 0)
                .map(|i| scene.candidate(i))
                .chain(unported)
                .collect::<Vec<_>>()
        }).collect()
    }

    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        let Some((scene, index)) = self.last.get(request.shape) else {
            self.refusal.get_or_insert("a sweep against a shape the last overlap did not return");
            return None;
        };
        let c = &scene.colliders[*index];
        match law::sweep_sphere(&c.mesh, &c.pose, request.origin, request.sphere_radius, request.direction,
            request.distance, None) {
            // A hit without the position flag leaves the zeroed position.
            Ok(hit) => hit.map(|hit| SweepHit { position: hit.position.unwrap_or([0.0; 3]), normal: hit.normal,
                distance: hit.distance }),
            Err(refused) => {
                self.refusal.get_or_insert(refused.0);
                None
            }
        }
    }

    fn refusal(&self) -> Option<&'static str> {
        self.refusal
    }

    fn order_known(&self) -> bool {
        self.order_known
    }
}
