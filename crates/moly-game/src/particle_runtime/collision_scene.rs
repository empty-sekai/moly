//! The physics scene the weather CollisionModules query: the MeshColliders of
//! the installed effects and of the site, cooked the way the engine cooks
//! them, at their authored poses, behind the module law's scene boundary.
//!
//! What the scene holds. The engine's weather loader installs, per site, the
//! sky and camera effects (the site's own or the phenomenon's global ones)
//! and the site effect; a common prefab is only ever installed nested inside
//! a site effect, whose own collider record lists its collider. A weather
//! change stops the old effect (every system stops emitting) and destroys its
//! GameObject, colliders included, only when the effect's own destroy delay
//! has run out, so during that window the old effect's colliders and the new
//! one's are in the physics scene together. The site's own colliders (the
//! site package's site prefab: layers 0, 4 and 9 by the package census) are
//! static and in the scene for the whole visit. The placed fixtures'
//! colliders sit on layer 9 (the fixture view moves every fixture node to the
//! navigation build layer when it attaches its components); their placement
//! is a mock and their shapes are not cooked here, so a site with placed
//! fixtures carries them as colliders the queries cannot answer against. No
//! character collider is in the scene.
//!
//! What a query meets. The module asks the scene for the static shapes (and
//! the dynamic ones when it collides with dynamic colliders) whose layer its
//! mask names, trigger shapes included (the query's trigger interaction is
//! the global setting). The engine's filter keeps every such shape and stops
//! the query before any narrowphase, so the candidates are the shapes whose
//! broadphase bounds meet the query box, in the broadphase's order. The
//! module then tests each lane against every candidate's world bounds at
//! inflation one, and the query box holds every lane's box, so a collider
//! whose bounds miss the query box is swept by no lane: the scene returns the
//! colliders whose bounds meet the box, which sweeps exactly the pairs the
//! engine sweeps. A trigger's hits are dropped by the module and never end a
//! lane, so a trigger changes no output; triggers are kept out of the
//! candidates.
//!
//! The collider export lists every collider of the effect prefabs and of the
//! site packages' site prefabs with its authored mesh, cooking options,
//! controls and the local transforms down to the collider. A site effect is
//! instantiated under the site prefab root, and a scene row's chain starts at
//! that root, so the two share one frame; the site anchor of an installed
//! effect is the identity, so a collider's world pose (in source axes, where
//! the module law works) is its chain:
//!
//! - only enabled colliders on active nodes are in the physics scene;
//! - the chain must be identity rotations and unit scales; its translations
//!   are summed root first, and every zero part of the pose is +0. The engine
//!   queries a collider that has no body at its static actor's pose composed
//!   with the shape's local pose; such a collider never sets that local pose,
//!   so it stays the shape's default identity, and composing an
//!   identity-valued actor pose with it gives +0 for every zero part whatever
//!   sign the actor pose carried (neither the effect nor the site packages
//!   hold a body). The BV4 queries choose their world matrix by these bits,
//!   so the signs are part of the answer there;
//! - the default cooking (the BVH33 midphase) and the options 30 (mesh
//!   cleaning, welding, faster simulation and the fast midphase: the BV4
//!   midphase) are ported, on any layer. A collider cooked with other
//!   options, or a convex or non-mesh one, stays in the scene as a collider
//!   the queries cannot answer against: a system whose mask names its layer
//!   is refused by name at admission, and a query whose mask names it
//!   refuses.
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

/// The layer the placed fixtures' colliders are on.
const FIXTURE_LAYER: u32 = 9;

/// The layers the site packages' colliders are on (the package census).
const SITE_LAYERS: u32 = (1 << 0) | (1 << 4) | (1 << 9);

/// Why a system whose mask names the placed fixtures' layer is refused.
pub(crate) const FIXTURE_REFUSAL: &str = "fixture colliders: placement mock and convex/box cooking not ported";

/// Why a query meets a collider the scene cannot answer against.
const UNPORTED_QUERY: &str = "a collider on a layer the mask names is not ported";

/// The scene a plan's collision systems query, or why they are refused.
pub(crate) type SceneVerdict = Result<Arc<GroundScene>, String>;

/// The colliders one installed thing (an effect, a site, the placed
/// fixtures, or several of them together) adds to the physics scene.
pub(crate) struct GroundScene {
    colliders: Vec<GroundCollider>,
    /// Colliders in the physics scene the queries cannot answer against,
    /// with their layer and why.
    unported: Vec<Unported>,
    /// Trigger colliders: in the scene, but no query output depends on them.
    inert: Vec<Unported>,
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
    /// The layers it is on, as a mask (one layer, except for a site package
    /// the export does not place, whose colliders stand on every layer the
    /// census finds site colliders on).
    layers: u32,
    reason: String,
    /// What a query that meets it refuses with.
    query_refusal: &'static str,
}

impl GroundScene {
    fn empty() -> Self {
        Self { colliders: Vec::new(), unported: Vec::new(), inert: Vec::new() }
    }

    /// What the scene holds, for diagnostics.
    pub(crate) fn describe(&self) -> Value {
        Value::Array(self.colliders.iter().map(|c| serde_json::json!({
            "effect": c.effect, "node": c.node, "geometry": c.geometry, "cooking": c.cooking, "layer": c.layer,
            "triangles": c.mesh.triangle_count(), "vertices": c.mesh.vertex_count(),
            "translation": c.pose.translation(),
        })).chain(self.unported.iter().map(|u| serde_json::json!({
            "effect": u.effect, "node": u.node, "layers": u.layers, "unported": u.reason,
        }))).chain(self.inert.iter().map(|u| serde_json::json!({
            "effect": u.effect, "node": u.node, "layers": u.layers, "inert": u.reason,
        }))).collect())
    }

    /// The number of colliders the queries answer against.
    pub(crate) fn collider_count(&self) -> usize {
        self.colliders.len()
    }

    /// The number of colliders the queries answer against on a layer the
    /// mask names.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn colliders_named(&self, mask: u32) -> usize {
        self.colliders.iter().filter(|c| mask & (1u32 << c.layer) != 0).count()
    }

    /// Why a system with this mask cannot query the scene: the first
    /// collider it cannot answer against on a layer the mask names.
    pub(crate) fn refusal_for(&self, mask: u32) -> Option<String> {
        self.unported.iter().find(|u| mask & u.layers != 0).map(|u| {
            if u.query_refusal == FIXTURE_REFUSAL {
                FIXTURE_REFUSAL.to_owned()
            } else {
                format!("collision scene: {} collider {} (layers {:#x}): {}", u.effect, u.node, u.layers, u.reason)
            }
        })
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

    /// The scene of this scene's one collider on `node` (for the replays
    /// that install colliders one by one in a native order).
    #[cfg(test)]
    pub(crate) fn single(&self, node: &str) -> Option<Arc<GroundScene>> {
        let mut found = self.colliders.iter().filter(|c| c.node == node);
        let collider = found.next()?;
        if found.next().is_some() {
            return None;
        }
        let mut scene = GroundScene::empty();
        scene.colliders.push(collider.share());
        Some(Arc::new(scene))
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

    fn extend(&mut self, other: &GroundScene) {
        self.colliders.extend(other.colliders.iter().map(GroundCollider::share));
        self.unported.extend(other.unported.iter().cloned());
        self.inert.extend(other.inert.iter().cloned());
    }
}

/// The placed fixtures' colliders, as the scene carries them: on their
/// layer, not answered against.
pub(crate) fn fixture_colliders() -> Arc<GroundScene> {
    let mut scene = GroundScene::empty();
    scene.unported.push(Unported { effect: "placed fixtures".into(), node: "*".into(), layers: 1 << FIXTURE_LAYER,
        reason: FIXTURE_REFUSAL.into(), query_refusal: FIXTURE_REFUSAL });
    Arc::new(scene)
}

/// How the effects being judged are installed.
pub(crate) enum Installation<'a> {
    /// Installed together by one plan (the sky, camera and site effects of a
    /// phenomenon at a site), with the site's own colliders when the site is
    /// named, and the placed fixtures' when there are any.
    Together { effects: &'a [&'a str], site: Option<&'a str>, fixtures: bool },
    /// A template that is installed only nested inside other effects.
    #[cfg_attr(not(test), allow(dead_code))]
    Template,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Selection {
    effects: Vec<String>,
    site: Option<String>,
    fixtures: bool,
}

/// Builds and caches the scenes of one collider export.
pub(crate) struct SceneBuilder {
    document: Result<Value, String>,
    meshes: MeshCache,
    effects: HashMap<String, Result<Arc<GroundScene>, String>>,
    sites: HashMap<String, Arc<GroundScene>>,
    verdicts: HashMap<Selection, SceneVerdict>,
    selected: Option<Selection>,
}

impl SceneBuilder {
    /// `document` is the parsed collider export, or why there is none.
    pub(crate) fn new(document: Result<Value, String>) -> Self {
        Self { document, meshes: HashMap::new(), effects: HashMap::new(), sites: HashMap::new(),
            verdicts: HashMap::new(), selected: Some(Selection { effects: Vec::new(), site: None, fixtures: false }) }
    }

    /// Sets how the effects judged next are installed.
    pub(crate) fn select(&mut self, installation: Installation<'_>) {
        self.selected = match installation {
            Installation::Together { effects, site, fixtures } => Some(Selection {
                effects: effects.iter().map(|e| (*e).to_owned()).collect(),
                site: site.map(str::to_owned),
                fixtures,
            }),
            Installation::Template => None,
        };
    }

    /// The scene the collision systems of `effect` query: the colliders of
    /// every effect installed with it, the site's and the placed fixtures'
    /// (see [`SceneBuilder::select`]; an effect outside the selection is
    /// judged as installed alone at the selection's site).
    pub(crate) fn for_effect(&mut self, effect: &str) -> SceneVerdict {
        let Some(mut selected) = self.selected.clone() else {
            return Err("collision scene: a common template is installed only nested inside its site effects, \
                whose own records carry its systems".into());
        };
        if !selected.effects.iter().any(|e| e == effect) {
            selected.effects = vec![effect.to_owned()];
        }
        self.scene_of(&selected)
    }

    /// The scene of the effects installed together at `site` (none: no
    /// site colliders), with the placed fixtures' when `fixtures`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn for_installed(&mut self, effects: &[&str], site: Option<&str>, fixtures: bool) -> SceneVerdict {
        self.scene_of(&Selection { effects: effects.iter().map(|e| (*e).to_owned()).collect(),
            site: site.map(str::to_owned), fixtures })
    }

    fn scene_of(&mut self, selection: &Selection) -> SceneVerdict {
        if let Some(verdict) = self.verdicts.get(selection) {
            return verdict.clone();
        }
        let mut scene = GroundScene::empty();
        let mut verdict = Ok(());
        for effect in &selection.effects {
            match self.effect_colliders(effect) {
                Ok(effect_scene) => scene.extend(&effect_scene),
                Err(reason) => {
                    verdict = Err(reason);
                    break;
                }
            }
        }
        if verdict.is_ok() {
            if let Some(site) = &selection.site {
                scene.extend(&self.site_colliders(site));
            }
        }
        if selection.fixtures {
            scene.extend(&fixture_colliders());
        }
        let verdict = verdict.map(|()| Arc::new(scene));
        self.verdicts.insert(selection.clone(), verdict.clone());
        verdict
    }

    /// The colliders one effect adds to the physics scene (possibly none).
    pub(crate) fn effect_colliders(&mut self, effect: &str) -> Result<Arc<GroundScene>, String> {
        if let Some(scene) = self.effects.get(effect) {
            return scene.clone();
        }
        let scene = self.build(|c| c.get("effect").and_then(Value::as_str) == Some(effect)
                // A common template's collider is listed again by every site
                // effect that nests it; the site effect's record is the installed one.
                && c.get("variant").and_then(Value::as_str) != Some("common"), effect)
            .map(Arc::new).map_err(|reason| format!("collision scene: {reason}"));
        self.effects.insert(effect.to_owned(), scene.clone());
        scene
    }

    /// The colliders the site's own package adds to the physics scene: the
    /// scene rows of its site prefab. When the export does not carry the
    /// site's package, or does not place its site prefab where the site
    /// effects are instantiated, the site's colliders stand in the scene as
    /// colliders the queries cannot answer against, on every layer the
    /// census finds site colliders on.
    pub(crate) fn site_colliders(&mut self, site: &str) -> Arc<GroundScene> {
        if let Some(scene) = self.sites.get(site) {
            return scene.clone();
        }
        let owner = format!("site {site}");
        let placed = self.document.as_ref().map_err(Clone::clone).and_then(|document| {
            let sites = document.get("sites").and_then(Value::as_object)
                .ok_or("the collider export carries no site packages")?;
            match sites.get(site) {
                None => Err(format!("the collider export does not carry the site package of {site}")),
                Some(entry) if entry.get("siteView").and_then(Value::as_str) == Some("") => Ok(()),
                Some(_) => Err("the site view is not on the site prefab root, so the frame the site effects share \
                    with its colliders is not established".to_owned()),
            }
        });
        let scene = placed.and_then(|()| self.build(|c| c.get("kind").and_then(Value::as_str) == Some("scene")
            && c.get("site").and_then(Value::as_str) == Some(site), &owner));
        let scene = Arc::new(scene.unwrap_or_else(|reason| {
            let mut scene = GroundScene::empty();
            scene.unported.push(Unported { effect: owner.clone(), node: "*".into(), layers: SITE_LAYERS, reason,
                query_refusal: UNPORTED_QUERY });
            scene
        }));
        self.sites.insert(site.to_owned(), scene.clone());
        scene
    }

    fn build(&mut self, selects: impl Fn(&Value) -> bool, owner: &str) -> Result<GroundScene, String> {
        let document = self.document.as_ref().map_err(Clone::clone)?;
        let records = document.get("colliders").and_then(Value::as_array)
            .ok_or("the collider export has no collider list")?;
        let mut scene = GroundScene::empty();
        for (ordinal, c) in records.iter().enumerate() {
            if !selects(c)
                || c.get("enabled").and_then(Value::as_bool) != Some(true)
                || c.get("activeInHierarchy").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let node = c.get("node").and_then(Value::as_str).unwrap_or("?").to_owned();
            let layer = c.get("layer").and_then(Value::as_u64).and_then(|l| u32::try_from(l).ok()).filter(|&l| l < 32)
                .ok_or_else(|| format!("collider {node}: no layer"))?;
            let unported = |reason: String| Unported { effect: owner.to_owned(), node: node.clone(), layers: 1 << layer,
                reason, query_refusal: UNPORTED_QUERY };
            if let Some(reason) = c.get("reason").and_then(Value::as_str) {
                scene.unported.push(unported(format!("not placed by the export: {reason}")));
                continue;
            }
            if c.get("isTrigger").and_then(Value::as_bool) == Some(true) {
                scene.inert.push(unported("a trigger: the module drops its hits".into()));
                continue;
            }
            match ground_collider(&mut self.meshes, document, ordinal, c, owner, layer) {
                Ok(collider) => scene.colliders.push(collider),
                Err(reason) => scene.unported.push(unported(reason)),
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

fn ground_collider(meshes: &mut MeshCache, document: &Value, ordinal: usize, c: &Value, owner: &str, layer: u32)
    -> Result<GroundCollider, String> {
    let node = c.get("node").and_then(Value::as_str).unwrap_or("?").to_owned();
    match (c.get("component").and_then(Value::as_str), c.get("queryShape").and_then(Value::as_str)) {
        (Some("MeshCollider"), Some("triangleMesh")) if c.get("convex").and_then(Value::as_bool) == Some(false) => {}
        (Some("BoxCollider"), _) => return Err("a BoxCollider (the sphere-versus-box sweep is not ported)".into()),
        _ => return Err("only a non-convex MeshCollider (triangle mesh query) is ported".into()),
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
        effect: owner.to_owned(),
        node,
        geometry,
        cooking,
        layer,
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

/// Whether a collider's world bounds meet the query box. A small slack keeps
/// this a superset of the module's own per-lane tests (each lane's box lies
/// inside the query box, whose margin is below a binary32 step at site
/// coordinates): a collider it lets through that no lane meets is swept by
/// no lane.
fn bounds_meet(bounds: &[f32; 6], center: [f32; 3], extents: [f32; 3]) -> bool {
    (0..3).all(|k| {
        let (c, e) = (f64::from(center[k]), f64::from(extents[k]));
        let (lo, hi) = (f64::from(bounds[k]), f64::from(bounds[3 + k]));
        let slack = 1e-4 * (1.0 + c.abs() + e.abs() + lo.abs().max(hi.abs()));
        lo - slack <= c + e && hi + slack >= c - e
    }) && bounds.iter().chain(&center).chain(&extents).all(|v| !v.is_nan())
}

/// The site's physics scene as the weather host keeps it: the colliders of
/// every installed thing (effects, the site, the placed fixtures), in the
/// order installed, a stopped effect's until the host destroys that effect.
/// Every installed system's query shares it.
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

    /// Adds an installed thing's colliders; the id removes them.
    pub(crate) fn install(&self, scene: Arc<GroundScene>) -> u64 {
        let mut entries = self.entries_mut();
        entries.next += 1;
        let id = entries.next;
        entries.entries.push((id, scene));
        id
    }

    /// Removes a destroyed thing's colliders.
    pub(crate) fn remove(&self, id: u64) {
        self.entries_mut().entries.retain(|(entry, _)| *entry != id);
    }

    pub(crate) fn clear(&self) {
        self.entries_mut().entries.clear();
    }

    /// The colliders the scene holds now that the queries answer against.
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
    /// layer the mask names and whose world bounds meet the box (see the
    /// module notes), at most `max_shapes`. A collider the queries cannot
    /// answer against, on a layer the mask names, refuses the call.
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.last.clear();
        self.refusal = None;
        let entries = self.scene.entries();
        let mut touched = Vec::new();
        for (_, scene) in &entries.entries {
            if let Some(u) = scene.unported.iter().find(|u| query.collides_with & u.layers != 0) {
                self.refusal.get_or_insert(u.query_refusal);
            }
            for (i, c) in scene.colliders.iter().enumerate() {
                if query.collides_with & (1u32 << c.layer) != 0 && bounds_meet(&c.bounds, query.center, query.extents) {
                    touched.push((scene.clone(), i));
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
            let unported = scene.unported.iter().filter(|u| collides_with & u.layers != 0).map(|_| Candidate {
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
