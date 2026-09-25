//! The physics scene a weather effect's CollisionModule queries: the
//! effect's ground MeshCollider, cooked the way the engine cooks it, at its
//! authored pose, behind the module law's scene boundary.
//!
//! The collider export lists every collider of the effect prefabs with its
//! authored mesh, cooking options, controls and the local transforms from
//! the prefab root down to the collider. The site anchor of an installed
//! effect is the identity, so a collider's world pose (in source axes, where
//! the module law works) is its chain. One scene is built per effect:
//!
//! - colliders of the common variant are left out (a common prefab is
//!   installed inside its site effect, whose own record lists the collider);
//! - only enabled colliders on active nodes are in the physics scene, and the
//!   admitted collision mask is the ground layer, so only layer-3 colliders
//!   can be touched;
//! - one collider per effect: the order of several broadphase touches is not
//!   transcribed, and every shipped effect has one;
//! - the chain must be identity rotations and unit scales; its translations
//!   are summed root first, and every zero part of the pose is +0. The
//!   engine queries a collider that has no body at its static actor's pose
//!   composed with the shape's local pose; such a collider never sets that
//!   local pose, so it stays the shape's default identity, and composing an
//!   identity-valued actor pose with it gives +0 for every zero part
//!   whatever sign the actor pose carried (the effect packages hold no
//!   body). The BV4 queries choose their world matrix by these bits, so
//!   the signs are part of the answer there;
//! - the default cooking (the BVH33 midphase) and the options 30 (mesh
//!   cleaning, welding, faster simulation and the fast midphase: the BV4
//!   midphase) are ported; a collider cooked with other options refuses its
//!   effect by name, as does a convex, trigger or non-mesh collider.
//!
//! Colliders of other installed effects (another weather's effect still
//! retiring, the site's own colliders on other layers) are not composed into
//! the scene.
use moly_law::particle::collision_mesh::{self as law, CookedMesh, Pose};
use moly_law::particle::collision_query::{Candidate, CollisionScene, OverlapQuery, SweepHit, SweepRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

const GROUND_LAYER: u64 = 3;

/// A scene for an effect, or why its collision systems are refused.
pub(crate) type SceneVerdict = Result<Arc<GroundScene>, String>;

pub(crate) struct GroundScene {
    colliders: Vec<GroundCollider>,
}

struct GroundCollider {
    node: String,
    geometry: String,
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

impl GroundScene {
    /// What the scene holds, for diagnostics.
    pub(crate) fn describe(&self) -> Value {
        Value::Array(self.colliders.iter().map(|c| serde_json::json!({
            "node": c.node, "geometry": c.geometry, "layer": c.layer,
            "triangles": c.mesh.triangle_count(), "vertices": c.mesh.vertex_count(),
            "translation": c.pose.translation(),
        })).collect())
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

/// Builds and caches the scenes of one collider export.
pub(crate) struct SceneBuilder {
    document: Result<Value, String>,
    meshes: MeshCache,
    scenes: HashMap<String, SceneVerdict>,
}

impl SceneBuilder {
    /// `document` is the parsed collider export, or why there is none.
    pub(crate) fn new(document: Result<Value, String>) -> Self {
        Self { document, meshes: HashMap::new(), scenes: HashMap::new() }
    }

    pub(crate) fn for_effect(&mut self, effect: &str) -> SceneVerdict {
        if let Some(verdict) = self.scenes.get(effect) {
            return verdict.clone();
        }
        let verdict = self.build(effect).map(Arc::new).map_err(|reason| format!("collision scene: {reason}"));
        self.scenes.insert(effect.to_owned(), verdict.clone());
        verdict
    }

    fn build(&mut self, effect: &str) -> Result<GroundScene, String> {
        let document = self.document.as_ref().map_err(Clone::clone)?;
        let colliders = document.get("colliders").and_then(Value::as_array)
            .ok_or("the collider export has no collider list")?;
        let chosen: Vec<(usize, &Value)> = colliders.iter().enumerate().filter(|(_, c)| {
            c.get("effect").and_then(Value::as_str) == Some(effect)
                && c.get("variant").and_then(Value::as_str) != Some("common")
                && c.get("enabled").and_then(Value::as_bool) == Some(true)
                && c.get("activeInHierarchy").and_then(Value::as_bool) == Some(true)
                && c.get("layer").and_then(Value::as_u64) == Some(GROUND_LAYER)
        }).collect();
        match chosen.as_slice() {
            [] => Err("no enabled layer-3 collider on an active node of this effect (common-variant colliders are \
                installed with their site effect)".into()),
            [(ordinal, collider)] => {
                let collider = ground_collider(&mut self.meshes, document, *ordinal, collider)?;
                Ok(GroundScene { colliders: vec![collider] })
            }
            _ => Err(format!("{} ground colliders in one effect: the order of several broadphase touches is not \
                transcribed", chosen.len())),
        }
    }
}

type MeshCache = HashMap<(String, u32), Result<Arc<CookedMesh>, &'static str>>;

fn ground_collider(meshes: &mut MeshCache, document: &Value, ordinal: usize, c: &Value) -> Result<GroundCollider, String> {
    let node = c.get("node").and_then(Value::as_str).unwrap_or("?").to_owned();
    let named = |reason: &str| format!("collider {node}: {reason}");
    if c.get("component").and_then(Value::as_str) != Some("MeshCollider")
        || c.get("queryShape").and_then(Value::as_str) != Some("triangleMesh")
        || c.get("convex").and_then(Value::as_bool) != Some(false) {
        return Err(named("only a non-convex MeshCollider (triangle mesh query) is ported"));
    }
    if c.get("isTrigger").and_then(Value::as_bool) != Some(false) {
        return Err(named("a trigger collider (the query's trigger interaction is not transcribed)"));
    }
    let cooking = c.pointer("/cooking/value").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| named("no cooking options"))?;
    let chain = c.get("chain").and_then(Value::as_array).filter(|chain| !chain.is_empty())
        .ok_or_else(|| named("no transform chain"))?;
    let mut translation: Option<[f32; 3]> = None;
    for link in chain {
        let path = link.get("path").and_then(Value::as_str).unwrap_or("?");
        let rotation = floats::<4>(link.get("rotation")).ok_or_else(|| named(&format!("chain node {path:?} has no rotation")))?;
        if rotation[..3].iter().any(|&v| v != 0.0) || rotation[3] != 1.0 {
            return Err(named(&format!("chain node {path:?} is rotated (only identity chains are ported)")));
        }
        if floats::<3>(link.get("scale")) != Some([1.0; 3]) {
            return Err(named(&format!("chain node {path:?} is scaled (only unit-scale chains are ported)")));
        }
        let position = floats::<3>(link.get("position")).ok_or_else(|| named(&format!("chain node {path:?} has no position")))?;
        translation = Some(match translation {
            None => position,
            Some(t) => [t[0] + position[0], t[1] + position[1], t[2] + position[2]],
        });
    }
    // The composed query pose: a +0 rotation and every zero translation part
    // +0 (see the module notes).
    let translation = translation.unwrap_or([0.0; 3]).map(|v| if v == 0.0 { 0.0 } else { v });
    let pose = Pose::new([0.0, 0.0, 0.0, 1.0], translation).map_err(|refused| named(refused.0))?;
    let geometry = c.get("geometry").and_then(Value::as_str).ok_or_else(|| named("no geometry"))?.to_owned();
    let mesh = match meshes.get(&(geometry.clone(), cooking)) {
        Some(mesh) => mesh.clone(),
        None => {
            let cooked = cook_geometry(document, &geometry, cooking);
            meshes.insert((geometry.clone(), cooking), cooked.clone());
            cooked
        }
    };
    let mesh = mesh.map_err(|reason| named(&format!("geometry {geometry} (cooking options {cooking}): {reason}")))?;
    let bounds = law::world_bounds(&mesh, &pose);
    Ok(GroundCollider {
        node,
        geometry,
        layer: GROUND_LAYER as u32,
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

/// One system's view of its effect's scene: the last overlap's colliders
/// (the sweep requests index them) and a refusal the module law reads.
pub(crate) struct GroundQuery {
    scene: Arc<GroundScene>,
    last: Vec<usize>,
    refusal: Option<&'static str>,
}

impl GroundQuery {
    pub(crate) fn new(scene: Arc<GroundScene>) -> Self {
        Self { scene, last: Vec::new(), refusal: None }
    }
}

impl CollisionScene for GroundQuery {
    /// The scene overlap of the module's box: the static colliders whose
    /// layer the mask names and whose triangles the box touches (the scene
    /// query's narrowphase; the pruner's inflated bounds contain every
    /// triangle the box can touch), at most `max_shapes`.
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.last.clear();
        self.refusal = None;
        let limit = usize::try_from(query.max_shapes.max(0)).unwrap_or(0);
        for (i, c) in self.scene.colliders.iter().enumerate() {
            if self.last.len() >= limit {
                break;
            }
            if query.collides_with & (1u32 << c.layer) == 0 {
                continue;
            }
            match law::overlap_box(&c.mesh, &c.pose, query.center, query.extents) {
                Ok(true) => self.last.push(i),
                Ok(false) => {}
                Err(refused) => {
                    self.refusal.get_or_insert(refused.0);
                }
            }
        }
        self.last.iter().map(|&i| self.scene.candidate(i)).collect()
    }

    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        (0..self.scene.colliders.len())
            .filter(|&i| collides_with & (1u32 << self.scene.colliders[i].layer) != 0)
            .map(|i| self.scene.candidate(i))
            .collect()
    }

    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        let Some(&index) = self.last.get(request.shape) else {
            self.refusal.get_or_insert("a sweep against a shape the last overlap did not return");
            return None;
        };
        let c = &self.scene.colliders[index];
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
}
