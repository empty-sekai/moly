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
//! navigation build layer when it attaches its components), and every placed
//! fixture view also adds a touch BoxCollider child at its origin (see
//! [`touch_box`]); the queries answer against a box with the ported box
//! sweep and world bounds, at the node's pose composed with the box centre.
//! The fixtures' authored MeshColliders are convex: each is cooked into its
//! hull the way the engine cooks a convex mesh with the collider's options,
//! and the queries answer against the hull with the ported sphere-versus-
//! convex sweep and the hull's tight world bounds, at the node's pose
//! composed with the shape's identity local pose (the collider's scale is
//! the mesh scale, so only unscaled nodes are cooked). A convex collider
//! without cooking options or on a scaled node stays in the scene with a
//! bound that encloses the engine's world bounds of it, and a query whose
//! box meets that bound refuses, so a query that answers is one the engine
//! answers without that collider. A touch box's layer is not read (it is created on the
//! default layer and the view's recursive layer write may or may not reach
//! it), so it stands on both: a query whose mask names only one of them and
//! whose box meets it refuses. Until the fixtures' collision scenes are
//! loaded a placeholder refuses every query naming their layers. No
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
//! The order of several touches. The broadphase returns the shapes in the
//! order its static pruner visits them: every static shape of the scene
//! (colliders, triggers and the ones the queries cannot answer against,
//! whatever their layer) sits in one pool, with a tree over it that is
//! rebuilt one physics step at a time and a bucket for the shapes added
//! since that tree was built. The scene keeps that pruner (the law's
//! `StaticPruner`), fed the engine's add and remove sequence, and a query
//! meets its colliders in the pruner's visit order:
//!
//! - a collider adds itself when it wakes and leaves when its node is
//!   deactivated; every physics step is one rebuild step and a commit, and
//!   every query commits first. The fixed step is 0.02 s and a frame runs at
//!   most 16 of them, before its scripts and its particle update;
//! - the game instantiates a site prefab under an active parent (a prefab
//!   clone wakes its colliders in ascending instance id of the originals,
//!   the order the package's preload table first names them: the export's
//!   `preloadFirst`), hides the site in the same run, which removes them in
//!   hierarchy post-order (children first, each node's children before its
//!   own components: the export's `activationOrder`), and shows it when the
//!   player enters, which adds them again in post-order;
//! - the home site loads the player's layout between its instantiation and
//!   its hiding, a frame after the instantiation at the earliest. Each placed
//!   fixture's load adds its own colliders and removes them again at once
//!   (the clone wakes active at the prefab's saved root pose, whose boxes
//!   there are not read: the placed box and the same box about the world
//!   origin stand in); then the site view shows the fixtures in its list
//!   order, each view's own colliders and then its touch box, and in the
//!   same run the site hides. The fixtures sit outside the site view, so
//!   they stay through its hide and show. The list order is the layout
//!   load's: the layouts in the server's order (the placement mock's, one
//!   layout per layout type, placed where its first row stands), each
//!   layout's rows sorted by their position's y with equal keys in order;
//!   the special floor cases, which follow a layout's normal rows, are not
//!   told apart, and each load is taken to finish in its call (a package on
//!   disk);
//! - where the sequence has a frame between two calls (the instantiation and
//!   the layout load at home; the hiding and the showing on entry at every
//!   site), the number of physics steps there is the frame timing's, so the
//!   scene holds one pruner per distinct outcome of every count until the
//!   pruners settle and of a query's commit without a step, and an order is
//!   known only where all of them agree; the showing may be committed by a
//!   query of its frame before the next step, and both stand;
//! - a pruner box is what the engine hands the pruner for a static shape:
//!   the shape's world bounds at inflation one, at the actor's pose composed
//!   with the shape's identity local pose, grown on every side by a
//!   two-hundredth of their size. A touch box's actor stands at its centre's
//!   world point; the scene composes that point its own way, which stands in
//!   for the transform's arithmetic (not read).
//!
//! The engine's pool also holds, for a while, the local player's avatar box:
//! the avatar's only collider, a BoxCollider with no body, which enters when
//! the avatar is cloned, is posed once (a bounds update) and leaves when the
//! game disables it once the avatar model has loaded. On the
//! housing-competition entry both come before the site is instantiated
//! (unless the model load outlasts the scene's five-second wait for the
//! player); on the normal entry the clone follows the server's messages, at
//! a frame not read. The pruner does not hold the box, so its order is
//! claimed only while the pool with that box fits one leaf, on the
//! assumption (read only for the housing-competition entry) that the box
//! left before the site was shown; the scene still feeds and queries the
//! pruners and logs how many queries they agree on.
//!
//! Where the pruner cannot give the order, the query falls back to the add
//! order: while the scene holds at most four static shapes and no shape has
//! left it since it was last empty, every query meets its shapes in the order
//! they were added. The pruner cannot give it when an installed effect adds
//! static shapes (whether they go through the site's hide and show is not
//! read), when the site changes or the placed fixtures change after entry
//! (neither sequence is read), when a shape has only an enclosing bound,
//! when a site's colliders carry no preload or activation order, when a
//! fixture has more than one collider of its own, before the home site's
//! placed fixtures are loaded, and on a query the pruners disagree on or
//! that meets a collider the pruner does not visit. Otherwise the query
//! reports the order unknown whenever the overlap returns more than one
//! collider, and the module law then selects over every order and refuses a
//! lane the order decides. Residual: the scene holds no character collider
//! and no harvest item, so a source scene that has such static shapes beside
//! the site's has more shapes than this one.
use moly_law::particle::collision_mesh::static_pruner::StaticPruner;
use moly_law::particle::collision_mesh::{self as law, CookedMesh, Pose};
use moly_law::particle::collision_query::{Candidate, CollisionScene, OverlapQuery, SweepHit, SweepRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The layer the placed fixtures' colliders are on.
const FIXTURE_LAYER: u32 = 9;

/// The layers the site packages' colliders are on (the package census).
const SITE_LAYERS: u32 = (1 << 0) | (1 << 4) | (1 << 9);

/// Why a query naming the placed fixtures' layers refuses before their
/// collision scenes are loaded.
pub(crate) const FIXTURE_PENDING: &str = "placed fixtures: their collision scenes are not loaded yet";

/// Why a query refuses when it meets the bound of a convex MeshCollider the
/// scene does not cook (no cooking options, or a scaled node).
const CONVEX_REFUSAL: &str = "a convex MeshCollider without cooking options or on a scaled node";

/// Why a query refuses when it meets the bound of a convex MeshCollider whose
/// cook takes a path the convex cooker does not carry.
const CONVEX_COOK_REFUSAL: &str = "a convex MeshCollider whose cook takes a path the convex cooker does not carry";

/// How far a convex collider's bound grows past its input's box, as a
/// fraction of the box's largest side plus a constant: the cooker turns a
/// flat or point-like input into a box thickened by a twentieth of the
/// shortest other side or by a hundredth of the length scale (one), both
/// below this.
const CONVEX_BOUND_MARGIN: (f32, f32) = (0.1, 0.02);

/// Why a query refuses when it meets a box whose pose the scene cannot hold.
const BOX_REFUSAL: &str = "a BoxCollider at a pose the scene cannot hold (non-finite, or a scaled node)";

/// Why a query refuses when its mask names only some of the layers a
/// collider may be on and its box meets the collider.
const LAYER_REFUSAL: &str = "a fixture's touch box whose layer is not read, for a mask naming only one of its layers";

/// Why a query refuses when it meets a fixture's non-convex MeshCollider
/// that the scene does not cook (other cooking options, or a rotated or
/// scaled pose).
const FIXTURE_MESH_REFUSAL: &str = "a fixture MeshCollider with unported cooking options or a rotated or scaled pose";

/// Why a query naming a fixture collider's layer refuses when the scene has
/// no bound for it.
const FIXTURE_UNBOUNDED: &str = "a fixture collider the scene can neither answer against nor bound";

/// The most vertices the convex cooker keeps before it bounds the hull by
/// other means; a hull within it has only input points as vertices.
const CONVEX_VERTEX_LIMIT: usize = 255;

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
    /// Each collider's position in the order this thing adds its static
    /// shapes to the physics scene, when that order is established for every
    /// shape it adds (see the module notes).
    add_positions: Option<Vec<usize>>,
    /// A site's static shapes with their first position in the site
    /// package's preload table and their position in the site's hierarchy
    /// post-order, when both are carried for every shape.
    site_order: Option<Vec<(ShapeAt, u64, u64)>>,
    /// The placed fixtures' static shapes with their fixture's slot in the
    /// site view's fixture list and whether each is its touch box, when
    /// every shape carries its slot.
    fixture_order: Option<Vec<(ShapeAt, usize, bool)>>,
    /// The placeholder the placed fixtures stand in with until their
    /// collision scenes load.
    pending_layout: bool,
}

struct GroundCollider {
    effect: String,
    node: String,
    geometry: String,
    cooking: u32,
    shape: ColliderShape,
    pose: Pose,
    layer: u32,
    /// Every layer it may be on, as a mask (its one layer unless the layer
    /// is not read).
    layers: u32,
    /// World bounds at inflation one: min then max.
    bounds: [f32; 6],
    /// The static pruner's box for it (see [`GroundScene::pool_box`]), or
    /// why the scene cannot give it.
    pool: Result<[f32; 6], &'static str>,
    /// The collider's ordinal in the export. The engine reports runtime
    /// instance ids, which are not package facts; only hit records carry
    /// them and no product output reads them.
    collider_id: i32,
}

/// What the queries answer against: a cooked mesh, a box's half extents, or
/// a cooked convex hull.
#[derive(Clone)]
enum ColliderShape {
    Mesh(Arc<CookedMesh>),
    Box([f32; 3]),
    Convex(Arc<law::HullSupport>),
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
    /// World bounds (min then max, source axes) that enclose the engine's
    /// bounds of the collider. A query refuses only when its box meets them,
    /// and admission leaves such a collider to the queries; without them any
    /// query naming its layers refuses, and so does admission.
    bounds: Option<[f32; 6]>,
}

impl GroundScene {
    fn empty() -> Self {
        Self { colliders: Vec::new(), unported: Vec::new(), inert: Vec::new(), add_positions: Some(Vec::new()),
            site_order: None, fixture_order: None, pending_layout: false }
    }

    /// The static shapes it adds to the physics scene.
    fn shape_count(&self) -> usize {
        self.colliders.len() + self.unported.len() + self.inert.len()
    }

    /// What the scene holds, for diagnostics.
    pub(crate) fn describe(&self) -> Value {
        Value::Array(self.colliders.iter().map(|c| {
            let mut entry = serde_json::json!({
                "effect": c.effect, "node": c.node, "geometry": c.geometry, "cooking": c.cooking, "layer": c.layer,
                "translation": c.pose.translation(),
            });
            match &c.shape {
                ColliderShape::Mesh(mesh) => {
                    entry["triangles"] = mesh.triangle_count().into();
                    entry["vertices"] = mesh.vertex_count().into();
                }
                ColliderShape::Box(half) => {
                    entry["halfExtents"] = serde_json::json!(half);
                    entry["rotation"] = serde_json::json!(c.pose.rotation());
                    entry["layers"] = c.layers.into();
                }
                ColliderShape::Convex(hull) => {
                    entry["hullVertices"] = hull.vertices.len().into();
                    entry["gaussMap"] = hull.gauss_map.is_some().into();
                    entry["rotation"] = serde_json::json!(c.pose.rotation());
                }
            }
            entry
        }).chain(self.unported.iter().map(|u| serde_json::json!({
            "effect": u.effect, "node": u.node, "layers": u.layers, "unported": u.reason,
            "bounds": u.bounds,
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
        self.colliders.iter().filter(|c| mask & c.layers != 0).count()
    }

    /// Why a system with this mask cannot query the scene: the first
    /// collider it cannot answer against on a layer the mask names.
    pub(crate) fn refusal_for(&self, mask: u32) -> Option<String> {
        self.unported.iter().find(|u| u.bounds.is_none() && mask & u.layers != 0).map(|u|
            format!("collision scene: {} collider {} (layers {:#x}): {}", u.effect, u.node, u.layers, u.reason))
    }

    /// The number of colliders the scene carries with a bound it refuses in.
    pub(crate) fn bounded_count(&self) -> usize {
        self.unported.iter().filter(|u| u.bounds.is_some()).count()
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
        // Its one shape is the first it adds.
        scene.add_positions = Some(vec![0]);
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
        let offset = self.shape_count();
        self.add_positions = match (self.add_positions.take(), &other.add_positions) {
            (Some(mut mine), Some(theirs)) => {
                mine.extend(theirs.iter().map(|p| p + offset));
                Some(mine)
            }
            _ => None,
        };
        self.colliders.extend(other.colliders.iter().map(GroundCollider::share));
        self.unported.extend(other.unported.iter().cloned());
        self.inert.extend(other.inert.iter().cloned());
        self.site_order = None;
        self.fixture_order = None;
    }
}

/// The layers a fixture's touch box may be on: its GameObject is created on
/// the default layer, and whether the view's recursive layer write reaches it
/// afterwards is not read, so both layers stand.
pub(crate) const TOUCH_BOX_LAYERS: u32 = (1 << 0) | (1 << FIXTURE_LAYER);

/// The placed fixtures' colliders before their collision scenes are loaded:
/// every query naming their layers refuses.
pub(crate) fn fixture_pending() -> Arc<GroundScene> {
    let mut scene = GroundScene::empty();
    scene.add_positions = None;
    scene.pending_layout = true;
    scene.unported.push(Unported { effect: "placed fixtures".into(), node: "*".into(),
        layers: TOUCH_BOX_LAYERS, reason: FIXTURE_PENDING.into(), query_refusal: FIXTURE_PENDING, bounds: None });
    Arc::new(scene)
}

/// One placed fixture collider in source axes: its shape in its node's frame,
/// the node's world matrix (column major), and the node's pose (rotation
/// x, y, z, w and translation) when the node is unscaled.
pub(crate) struct FixturePart {
    pub(crate) what: String,
    pub(crate) layers: u32,
    pub(crate) world: [f32; 16],
    pub(crate) pose: Option<([f32; 4], [f32; 3])>,
    pub(crate) shape: FixtureShape,
    /// The fixture's slot in the site view's fixture list (the order the
    /// views are shown in), when known.
    pub(crate) slot: Option<usize>,
    /// Whether this is the view's touch box, which is shown after the view's
    /// own colliders (the touch node is the view's last child).
    pub(crate) touch: bool,
}

/// Records a placed fixture shape's slot, or forgets the order when a shape
/// has none.
fn record_slot(order: &mut Option<Vec<(ShapeAt, usize, bool)>>, at: ShapeAt, slot: Option<usize>, touch: bool) {
    match (order.as_mut(), slot) {
        (Some(order), Some(slot)) => order.push((at, slot, touch)),
        _ => *order = None,
    }
}

/// `PxQuat::operator*`.
fn quat_mul(p: [f32; 4], q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = p;
    [
        w * q[0] + q[3] * x + y * q[2] - q[1] * z,
        w * q[1] + q[3] * y + z * q[0] - q[2] * x,
        w * q[2] + q[3] * z + x * q[1] - q[0] * y,
        w * q[3] - x * q[0] - y * q[1] - z * q[2],
    ]
}

/// `PxQuat::rotate`.
fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let (vx, vy, vz) = (2.0 * v[0], 2.0 * v[1], 2.0 * v[2]);
    let w2 = w * w - 0.5;
    let dot2 = x * vx + y * vy + z * vz;
    [
        vx * w2 + (y * vz - z * vy) * w + x * dot2,
        vy * w2 + (z * vx - x * vz) * w + y * dot2,
        vz * w2 + (x * vy - y * vx) * w + z * dot2,
    ]
}

/// A box shape's query pose: the static actor's pose (the node's) composed
/// with the shape's local pose (the box centre, no rotation), as
/// `PxTransform::transform` composes them.
fn box_pose(node: ([f32; 4], [f32; 3]), center: [f32; 3]) -> Option<Pose> {
    let (q, t) = node;
    let r = quat_rotate(q, center);
    let p = [r[0] + t[0], r[1] + t[1], r[2] + t[2]];
    Pose::rotated(quat_mul(q, [0.0, 0.0, 0.0, 1.0]), p).ok()
}

pub(crate) enum FixtureShape {
    Mesh { convex: bool, cooking: Option<u32>, positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]> },
    Box { center: [f32; 3], half: [f32; 3] },
    Unknown(String),
}

/// A fixture view's touch box (the view's touch collider): a BoxCollider
/// child at the view's origin, with no rotation, sized from the placement's
/// grid size and layout type: a layout other than 0 spans the grid's X and Z
/// tiles plus the view's touch adjustment and the touch height (halved for
/// layout 2); layout 0 spans the X tiles, the Y tiles and one Z tile. Its
/// centre is half its height up. The adjustment is what the view's
/// constructor sets, half a tile; the tile is 0.25 on every axis and the
/// touch height 0.125 (the constants' static constructor). Returns the
/// centre and half extents.
pub(crate) fn touch_box(grid: [i32; 3], layout: u8) -> ([f32; 3], [f32; 3]) {
    const TILE: f32 = 0.25;
    const TOUCH_HEIGHT: f32 = 0.125;
    const ADJUST: f32 = TILE * 0.5;
    let size = if layout != 0 {
        let height = if layout == 2 { TOUCH_HEIGHT * 0.5 } else { TOUCH_HEIGHT };
        [TILE * grid[0] as f32 + ADJUST, height, ADJUST + TILE * grid[2] as f32]
    } else {
        [TILE * grid[0] as f32, TILE * grid[1] as f32, TILE]
    };
    ([0.0, size[1] * 0.5, 0.0], size.map(|v| v * 0.5))
}

/// The world bounds of a box in a node's frame through its world matrix
/// (the eight corners), grown outward by a relative binary32 step so that the
/// engine's own rounding of the same bounds stays inside.
fn enclosing_bounds(world: &[f32; 16], lo: [f32; 3], hi: [f32; 3]) -> Option<[f32; 6]> {
    let mut out = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for corner in 0..8 {
        let c = [if corner & 1 == 0 { lo[0] } else { hi[0] }, if corner & 2 == 0 { lo[1] } else { hi[1] },
            if corner & 4 == 0 { lo[2] } else { hi[2] }].map(f64::from);
        for k in 0..3 {
            let v = f64::from(world[k]) * c[0] + f64::from(world[4 + k]) * c[1] + f64::from(world[8 + k]) * c[2]
                + f64::from(world[12 + k]);
            out[k] = out[k].min(v);
            out[3 + k] = out[3 + k].max(v);
        }
    }
    let grow = |v: f64, up: bool| {
        let step = 1e-5 * (1.0 + v.abs());
        let v = if up { v + step } else { v - step };
        v as f32
    };
    let bounds = [grow(out[0], false), grow(out[1], false), grow(out[2], false),
        grow(out[3], true), grow(out[4], true), grow(out[5], true)];
    bounds.iter().all(|v| v.is_finite()).then_some(bounds)
}

/// The placed fixtures' colliders as the physics scene carries them.
pub(crate) fn fixture_scene(parts: Vec<FixturePart>) -> Arc<GroundScene> {
    let mut scene = GroundScene::empty();
    // The fixture views add their colliders at run time; the static pruner
    // takes their order from each fixture's slot.
    scene.add_positions = None;
    let mut order = Some(Vec::new());
    for part in parts {
        let unbounded = |reason: String| Unported { effect: "placed fixtures".into(), node: part.what.clone(),
            layers: part.layers, reason, query_refusal: FIXTURE_UNBOUNDED, bounds: None };
        let bounded = |reason: &'static str, bounds: Option<[f32; 6]>| match bounds {
            Some(bounds) => Unported { effect: "placed fixtures".into(), node: part.what.clone(), layers: part.layers,
                reason: reason.into(), query_refusal: reason, bounds: Some(bounds) },
            None => unbounded(format!("{reason}; its bounds are not finite")),
        };
        let local_box = |points: &[[f32; 3]]| points.iter().fold(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(lo, hi), p| (std::array::from_fn(|k| lo[k].min(p[k])), std::array::from_fn(|k| hi[k].max(p[k]))));
        let entry = match &part.shape {
            FixtureShape::Box { center, half } => {
                match part.pose.and_then(|node| box_pose(node, *center)).filter(|_| half.iter().all(|h| h.is_finite())) {
                    Some(pose) => {
                        let bounds = law::box_world_bounds(*half, &pose);
                        // A static BoxCollider's actor stands at its centre's
                        // world point (the node's transform of the centre) with
                        // the node's rotation and an identity shape pose; the
                        // query pose above composes that point the scene's way,
                        // which stands in for the transform's own arithmetic.
                        let pool = law::shape_world_pose(&pose, [0.0, 0.0, 0.0, 1.0], [0.0; 3])
                            .map(|shape| law::pool_bounds_box(*half, &shape)).map_err(|refused| refused.0);
                        scene.colliders.push(GroundCollider { effect: "placed fixtures".into(), node: part.what.clone(),
                            geometry: "box".into(), cooking: 0, shape: ColliderShape::Box(*half), pose,
                            layer: part.layers.trailing_zeros(), layers: part.layers, bounds, pool,
                            collider_id: i32::MAX });
                        record_slot(&mut order, ShapeAt { list: ShapeAt::COLLIDERS, index: scene.colliders.len() - 1 },
                            part.slot, part.touch);
                        continue;
                    }
                    None => {
                        let lo = std::array::from_fn(|k| center[k] - half[k]);
                        let hi = std::array::from_fn(|k| center[k] + half[k]);
                        bounded(BOX_REFUSAL, enclosing_bounds(&part.world, lo, hi))
                    }
                }
            }
            FixtureShape::Mesh { convex: true, cooking, positions, triangles } => {
                // The shape's query pose: the node's, composed with the
                // shape's identity local pose.
                let pose = part.pose.and_then(|node| box_pose(node, [0.0; 3]));
                let indices: Vec<u32> = triangles.iter().flatten().copied().collect();
                let cooked = cooking.zip(pose).map(|(cooking, pose)| {
                    law::cook_convex(positions, &indices, cooking)
                        .and_then(|hull| law::convex_world_bounds(&hull, &pose).map(|bounds| (hull, bounds)))
                        .map(|(hull, bounds)| (cooking, pose, hull, bounds))
                });
                match cooked {
                    Some(Ok((cooking, pose, hull, bounds))) if part.layers.count_ones() == 1 => {
                        let pool = part.pose.ok_or("a convex collider on a scaled node")
                            .and_then(|(q, t)| Pose::rotated(q, t).map_err(|refused| refused.0))
                            .and_then(|node| law::shape_world_pose(&node, [0.0, 0.0, 0.0, 1.0], [0.0; 3])
                                .map_err(|refused| refused.0))
                            .and_then(|shape| law::pool_bounds_convex(&hull, &shape).map_err(|refused| refused.0));
                        scene.colliders.push(GroundCollider { effect: "placed fixtures".into(), node: part.what.clone(),
                            geometry: "fixture convex".into(), cooking, shape: ColliderShape::Convex(Arc::new(hull)),
                            pose, layer: part.layers.trailing_zeros(), layers: part.layers, bounds, pool,
                            collider_id: i32::MAX });
                        record_slot(&mut order, ShapeAt { list: ShapeAt::COLLIDERS, index: scene.colliders.len() - 1 },
                            part.slot, part.touch);
                        continue;
                    }
                    Some(Ok(_)) => unbounded("a fixture MeshCollider on an unsettled layer".into()),
                    // The engine's own cook fails: the collider has no shape
                    // and is not in the physics scene.
                    Some(Err(refused)) if law::engine_cooks_nothing(&refused) => continue,
                    Some(Err(refused)) => {
                        let (lo, hi) = local_box(positions);
                        let side = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
                        let margin = CONVEX_BOUND_MARGIN.0 * side + CONVEX_BOUND_MARGIN.1;
                        let lo = lo.map(|v| v - margin);
                        let hi = hi.map(|v| v + margin);
                        match enclosing_bounds(&part.world, lo, hi) {
                            Some(bounds) => Unported { effect: "placed fixtures".into(), node: part.what.clone(),
                                layers: part.layers, reason: format!("{CONVEX_COOK_REFUSAL}: {}", refused.0),
                                query_refusal: CONVEX_COOK_REFUSAL, bounds: Some(bounds) },
                            None => unbounded(format!("{CONVEX_COOK_REFUSAL}: {}; its bounds are not finite", refused.0)),
                        }
                    }
                    None => {
                        // The cooked hull's vertices are input points while
                        // the hull stays within the cooker's vertex limit, so
                        // the input's box encloses the hull's own; a hull past
                        // the limit is bounded by other means that are not read.
                        let points: Vec<parry3d::math::Vector> = positions.iter().copied()
                            .map(parry3d::math::Vector::from_array).collect();
                        match parry3d::transformation::try_convex_hull(&points) {
                            Ok((hull, _)) if !hull.is_empty() && hull.len() <= CONVEX_VERTEX_LIMIT => {
                                let (lo, hi) = local_box(positions);
                                bounded(CONVEX_REFUSAL, enclosing_bounds(&part.world, lo, hi))
                            }
                            Ok((hull, _)) => unbounded(format!("a convex MeshCollider whose hull has {} vertices \
                                (the cooker's vertex-limit path is not read)", hull.len())),
                            Err(error) => unbounded(format!("a convex MeshCollider whose hull is degenerate: {error:?}")),
                        }
                    }
                }
            }
            FixtureShape::Mesh { convex: false, cooking, positions, triangles } => {
                let identity = part.world[..12] == [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
                let cooked = cooking.filter(|_| identity)
                    .and_then(|cooking| law::cook(positions, triangles, cooking).ok().map(|mesh| (cooking, mesh)));
                match cooked {
                    Some((cooking, mesh)) => {
                        let translation = [part.world[12], part.world[13], part.world[14]]
                            .map(|v| if v == 0.0 { 0.0 } else { v });
                        match Pose::new([0.0, 0.0, 0.0, 1.0], translation) {
                            Ok(pose) => {
                                let mesh = Arc::new(mesh);
                                let bounds = law::world_bounds(&mesh, &pose);
                                let pool = law::shape_world_pose(&pose, [0.0, 0.0, 0.0, 1.0], [0.0; 3])
                                    .map(|shape| law::pool_bounds_mesh(&mesh, &shape)).map_err(|refused| refused.0);
                                let layer = part.layers.trailing_zeros();
                                if part.layers.count_ones() == 1 {
                                    scene.colliders.push(GroundCollider { effect: "placed fixtures".into(),
                                        node: part.what.clone(), geometry: "fixture".into(), cooking,
                                        shape: ColliderShape::Mesh(mesh), pose, layer, layers: part.layers, bounds,
                                        pool, collider_id: i32::MAX });
                                    record_slot(&mut order,
                                        ShapeAt { list: ShapeAt::COLLIDERS, index: scene.colliders.len() - 1 },
                                        part.slot, part.touch);
                                    continue;
                                }
                                unbounded("a fixture MeshCollider on an unsettled layer".into())
                            }
                            Err(_) => unbounded("a fixture MeshCollider pose the scene cannot hold".into()),
                        }
                    }
                    None => {
                        let (lo, hi) = local_box(positions);
                        bounded(FIXTURE_MESH_REFUSAL, enclosing_bounds(&part.world, lo, hi))
                    }
                }
            }
            FixtureShape::Unknown(kind) => unbounded(format!("a {kind}")),
        };
        scene.unported.push(entry);
        record_slot(&mut order, ShapeAt { list: ShapeAt::UNPORTED, index: scene.unported.len() - 1 }, part.slot,
            part.touch);
    }
    scene.fixture_order = order;
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
        // The placed fixtures' colliders are judged by the queries: the
        // host installs them (or the placeholder) in the live scene.
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
            scene.add_positions = None;
            scene.unported.push(Unported { effect: owner.clone(), node: "*".into(), layers: SITE_LAYERS, reason,
                query_refusal: UNPORTED_QUERY, bounds: None });
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
        // Each shape's add rank (the export's activationOrder): colliders,
        // then the other shapes, in the order pushed.
        let (mut collider_ranks, mut other_ranks) = (Vec::new(), Vec::new());
        // Each shape's first preload-table position and activation rank, for
        // the static pruner's sequence.
        let mut site_ranks: Vec<(ShapeAt, Option<u64>, Option<u64>)> = Vec::new();
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
                reason, query_refusal: UNPORTED_QUERY, bounds: None };
            // A site's colliders are the ones its activation added (the game
            // hides a site once instantiated and shows it on entry); whether
            // an effect's colliders go through that cycle is not read.
            let rank = (c.get("kind").and_then(Value::as_str) == Some("scene"))
                .then(|| c.get("activationOrder").and_then(Value::as_u64)).flatten();
            let preload = (c.get("kind").and_then(Value::as_str) == Some("scene"))
                .then(|| c.get("preloadFirst").and_then(Value::as_u64)).flatten();
            if let Some(reason) = c.get("reason").and_then(Value::as_str) {
                scene.unported.push(unported(format!("not placed by the export: {reason}")));
                other_ranks.push(rank);
                site_ranks.push((ShapeAt { list: ShapeAt::UNPORTED, index: scene.unported.len() - 1 }, preload, rank));
                continue;
            }
            if c.get("isTrigger").and_then(Value::as_bool) == Some(true) {
                scene.inert.push(unported("a trigger: the module drops its hits".into()));
                other_ranks.push(rank);
                site_ranks.push((ShapeAt { list: ShapeAt::INERT, index: scene.inert.len() - 1 }, preload, rank));
                continue;
            }
            match ground_collider(&mut self.meshes, document, ordinal, c, owner, layer) {
                Ok(collider) => {
                    scene.colliders.push(collider);
                    collider_ranks.push(rank);
                    site_ranks.push((ShapeAt { list: ShapeAt::COLLIDERS, index: scene.colliders.len() - 1 }, preload,
                        rank));
                }
                Err(reason) => {
                    scene.unported.push(unported(reason));
                    other_ranks.push(rank);
                    site_ranks.push((ShapeAt { list: ShapeAt::UNPORTED, index: scene.unported.len() - 1 }, preload,
                        rank));
                }
            }
        }
        scene.add_positions = add_positions(&collider_ranks, &other_ranks);
        scene.site_order = site_ranks.into_iter().map(|(at, preload, rank)| Some((at, preload?, rank?))).collect();
        Ok(scene)
    }
}

/// Each collider's position in the add order of one installed thing: its
/// shapes in ascending add rank. `None` when a shape has no rank or two share
/// one (a rank is a first position in one table, so a tie means the ranks
/// came from different tables).
fn add_positions(collider_ranks: &[Option<u64>], other_ranks: &[Option<u64>]) -> Option<Vec<usize>> {
    let ranks: Vec<u64> = collider_ranks.iter().chain(other_ranks).copied().collect::<Option<_>>()?;
    let mut sorted = ranks.clone();
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    Some(collider_ranks.iter().map(|rank| sorted.binary_search(&rank.unwrap_or(0)).unwrap_or(usize::MAX)).collect())
}

impl GroundCollider {
    fn share(&self) -> Self {
        Self {
            effect: self.effect.clone(),
            node: self.node.clone(),
            geometry: self.geometry.clone(),
            cooking: self.cooking,
            shape: self.shape.clone(),
            pose: self.pose,
            layer: self.layer,
            layers: self.layers,
            bounds: self.bounds,
            pool: self.pool,
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
    // The actor's pose composed with the shape's identity local pose.
    let pool = law::shape_world_pose(&pose, [0.0, 0.0, 0.0, 1.0], [0.0; 3])
        .map(|shape| law::pool_bounds_mesh(&mesh, &shape)).map_err(|refused| refused.0);
    Ok(GroundCollider {
        effect: owner.to_owned(),
        node,
        geometry,
        cooking,
        layer,
        layers: 1 << layer,
        bounds,
        pool,
        collider_id: i32::try_from(ordinal).unwrap_or(i32::MAX),
        shape: ColliderShape::Mesh(mesh),
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
/// Every installed system's query shares it, and so does the static pruner
/// that gives the order of several touches (see the module notes).
#[derive(Clone, Default)]
pub(crate) struct SiteScene(Arc<RwLock<SiteEntries>>);

#[derive(Default)]
struct SiteEntries {
    next: u64,
    entries: Vec<(u64, Arc<GroundScene>)>,
    /// A removal since the scene was last empty moved static shapes within
    /// the pool (the removed thing's shapes were not the pool's last) or left
    /// a split tree behind (the scene held more than one leaf of shapes).
    reshuffled: bool,
    pruner: PrunerScene,
}

/// The most static shapes a pruner leaf holds: a scene of at most this many
/// is one leaf in the tree and in the bucket.
const LEAF_SHAPES: usize = 4;

impl SiteEntries {
    fn shape_count(&self) -> usize {
        self.entries.iter().map(|(_, scene)| scene.shape_count()).sum()
    }

    /// Whether every query meets its shapes in add order (see the module
    /// notes): at most one leaf of shapes, no reshuffling removal, and every
    /// installed thing's add order established.
    fn add_order_holds(&self) -> bool {
        !self.reshuffled && self.shape_count() <= LEAF_SHAPES
            && self.entries.iter().all(|(_, scene)| scene.add_positions.is_some())
    }

    fn push(&mut self, scene: Arc<GroundScene>) -> u64 {
        self.next += 1;
        let id = self.next;
        self.entries.push((id, scene));
        id
    }

    fn feed(&mut self) {
        let SiteEntries { entries, pruner, .. } = self;
        pruner.feed(entries);
    }
}

impl SiteScene {
    fn entries(&self) -> std::sync::RwLockReadGuard<'_, SiteEntries> {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn entries_mut(&self) -> std::sync::RwLockWriteGuard<'_, SiteEntries> {
        self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds an installed effect's colliders; the id removes them. Where an
    /// effect's static shapes enter the pruner's pool is not read (see the
    /// module notes), so an effect that adds any leaves the pruner unable to
    /// give the order.
    pub(crate) fn install(&self, scene: Arc<GroundScene>) -> u64 {
        let mut entries = self.entries_mut();
        let shapes = scene.shape_count();
        let id = entries.push(scene);
        if shapes > 0 {
            entries.pruner.refuse("an installed effect's colliders: whether they go through the site's hide and \
                show, and so where they enter the pruner's pool, is not read");
        }
        id
    }

    /// Adds the site's own colliders; `layout` when the site loads the
    /// player's layout (the home site), whose placed fixtures enter the
    /// scene before the site is hidden.
    pub(crate) fn install_site(&self, scene: Arc<GroundScene>, layout: bool) -> u64 {
        let mut entries = self.entries_mut();
        let id = entries.push(scene);
        entries.pruner.site = Some((id, layout));
        entries.feed();
        id
    }

    /// Adds the placed fixtures' colliders (or the placeholder that stands
    /// in for them until their collision scenes load).
    pub(crate) fn install_fixtures(&self, scene: Arc<GroundScene>) -> u64 {
        let mut entries = self.entries_mut();
        let id = entries.push(scene);
        entries.pruner.fixtures = Some(id);
        entries.feed();
        id
    }

    /// Removes a destroyed thing's colliders.
    pub(crate) fn remove(&self, id: u64) {
        let mut entries = self.entries_mut();
        if let Some(at) = entries.entries.iter().position(|(entry, _)| *entry == id) {
            // Shapes after the removed thing's in the pool move into its
            // gaps; and a scene of more than one leaf keeps its split tree
            // until a rebuild, whose timing is not modelled here.
            let later = entries.entries[at + 1..].iter().map(|(_, scene)| scene.shape_count()).sum::<usize>();
            let removed = entries.entries[at].1.shape_count();
            if removed > 0 && (later > 0 || entries.shape_count() > LEAF_SHAPES) {
                entries.reshuffled = true;
            }
            entries.entries.remove(at);
            if entries.pruner.site.is_some_and(|(site, _)| site == id) {
                entries.pruner.site = None;
                entries.pruner.refuse("the site's colliders left the scene (a site change): the order they leave \
                    in and the frames between that and the next site's entry are not modelled");
            } else if entries.pruner.fixtures == Some(id) {
                // The replacement is compared with the fixtures fed.
                entries.pruner.fixtures = None;
            } else if removed > 0 {
                entries.pruner.refuse("an installed effect's colliders left the scene: the order they leave in \
                    is not read");
            }
        }
        if entries.shape_count() == 0 {
            entries.reshuffled = false;
        }
    }

    pub(crate) fn clear(&self) {
        let mut entries = self.entries_mut();
        entries.entries.clear();
        entries.reshuffled = false;
        entries.pruner = PrunerScene::default();
    }

    /// The frame's physics steps, before the frame's particle queries: one
    /// pruner step per fixed step of the frame's delta time.
    pub(crate) fn advance(&self, dt: f32) {
        self.entries_mut().pruner.advance(dt);
    }

    /// The colliders the scene holds now that the queries answer against.
    pub(crate) fn collider_count(&self) -> usize {
        self.entries().entries.iter().map(|(_, scene)| scene.colliders.len()).sum()
    }
}

/// The fixed timestep (the time manager's Fixed Timestep).
const FIXED_STEP: f64 = 0.02;
/// The most fixed steps one frame runs: the time manager's Maximum Allowed
/// Timestep (1/3 s) over the fixed timestep.
const MAX_FIXED_STEPS: u32 = 16;
/// The most physics steps a boundary is followed through before the
/// pruners must have settled.
const BOUNDARY_STEPS: usize = 64;

/// The static shapes the engine's pool holds for a while beside the ones
/// the scene feeds: the local player's avatar box. The avatar prefab's only
/// collider is a BoxCollider on its avatar root with no body anywhere on the
/// avatar (and the game adds none at run time), so a static shape; it enters
/// the pool when the avatar is cloned, moves once when the clone is posed (a
/// bounds update, which forces a rebuild), and leaves when the game disables
/// it for the local player once the avatar model has loaded. On the
/// housing-competition entry, where the scene creates the player itself, the
/// clone and the disable come before the site is instantiated: the scene
/// waits for the player (registered only after the disable), or five
/// seconds, before it sets up the sites, and a model load past that wait
/// moves the disable later. On the normal entry the clone follows the
/// server's messages, at a frame not read. The pruner does not hold the box:
/// its add and removal restart the rebuild, and a removal while shapes added
/// after it are in the pool moves the pool's last shape into its slot. While
/// one leaf holds the whole pool and the box has left before the site is
/// shown, the visits keep the fed pool order; the scene claims the order
/// only there, and on the normal entry that the box has left before the
/// showing is an assumption (not read).
const UNFED_TRANSIENT_SHAPES: usize = 1;

/// Why the pruner's order is not claimed while the pool is more than a leaf.
const TRANSIENT_SHAPE: &str = "the local player's avatar box enters, moves once and leaves the engine's pool (before \
    the site is instantiated on the housing-competition entry, at a frame the server's messages set on the normal \
    entry), the pruner holds neither that box nor its move, and the pool is more than a leaf";

/// The assumption the order stands on where it is claimed.
const TRANSIENT_ASSUMED: &str = "orders claimed on the assumption that the local player's avatar box left the pool \
    before the site was shown: so on the housing-competition entry when the avatar model loads within the \
    five-second wait; on the normal entry the avatar's clone follows the server's messages (not read)";

/// The engine's static pruner for the scene's static shapes, fed the
/// engine's add and remove sequence for the site and the placed fixtures
/// (see the module notes). Where that sequence has an unread number of
/// physics steps between two calls, it holds one pruner per distinct
/// outcome, and an order is known only where they all agree.
#[derive(Default)]
struct PrunerScene {
    /// The installed site's entry, and whether it loads the player's layout.
    site: Option<(u64, bool)>,
    /// The placed fixtures' entry.
    fixtures: Option<u64>,
    /// The placed fixtures the sequence was fed: slot, touch box, pool box.
    fed_fixtures: Option<Vec<(usize, bool, [u32; 6])>>,
    /// One pruner per distinct outcome of the unread step counts.
    variants: Vec<StaticPruner>,
    /// The unread boundaries the pruners span.
    boundaries: Vec<String>,
    fed: bool,
    /// Why the pruner cannot give the order, for good.
    unmodeled: Option<String>,
    /// Time toward the next fixed step.
    clock: f64,
    /// The lines already reported to the log.
    reported: Vec<String>,
    /// Queries answered while fed; of them, the ones every pruner agreed on.
    queries: u64,
    agreed: u64,
}

/// Each distinct pruner once (a pruner's future answers are a function of
/// its state).
fn distinct(pruners: Vec<StaticPruner>) -> Vec<StaticPruner> {
    let mut out: Vec<StaticPruner> = Vec::new();
    for p in pruners {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The pruners after a boundary whose physics step count is not read: every
/// count from none until each pruner settles, and a query's commit without
/// a step (the other queries of a frame without a fixed step).
fn boundary(pruners: Vec<StaticPruner>, name: &str, spans: &mut Vec<String>) -> Result<Vec<StaticPruner>, String> {
    let mut out = Vec::new();
    let mut longest = 0usize;
    for p in pruners {
        let mut committed = p.clone();
        committed.commit();
        out.push(committed);
        let mut stepped = p.clone();
        out.push(p);
        let mut settled = false;
        for k in 1..=BOUNDARY_STEPS {
            let before = stepped.clone();
            stepped.step();
            if stepped == before {
                settled = true;
                longest = longest.max(k - 1);
                break;
            }
            out.push(stepped.clone());
        }
        if !settled {
            return Err(format!("{name}: the pruner does not settle within {BOUNDARY_STEPS} physics steps"));
        }
    }
    let out = distinct(out);
    spans.push(format!("{name}: 0 to {longest} steps, {} distinct", out.len()));
    Ok(out)
}

impl PrunerScene {
    fn refuse(&mut self, why: &str) {
        if self.unmodeled.is_none() {
            self.unmodeled = Some(why.to_owned());
            self.variants.clear();
            self.fed = false;
            self.report(format!("the static pruner cannot give the order: {why}"));
        }
    }

    /// Logs each distinct line once.
    fn report(&mut self, line: String) {
        if !self.reported.contains(&line) {
            bevy::log::info!("[weather-fx] static pruner: {line}");
            self.reported.push(line);
        }
    }

    fn unfed(&mut self) {
        self.fed = false;
        self.variants.clear();
    }

    /// Feeds the engine's sequence once the site (and, at the home site, the
    /// placed fixtures) are in the scene.
    fn feed(&mut self, entries: &[(u64, Arc<GroundScene>)]) {
        if self.unmodeled.is_some() {
            return;
        }
        let find = |id: u64| entries.iter().find(|(entry, _)| *entry == id).map(|(_, scene)| scene.clone());
        let Some((site_entry, layout)) = self.site else {
            return self.unfed();
        };
        let Some(site) = find(site_entry) else {
            return self.unfed();
        };
        let Some(site_order) = site.site_order.clone() else {
            return self.refuse("the site's colliders carry no preload-table or activation order");
        };
        let mut site_shapes = Vec::new();
        for (at, preload, activation) in site_order {
            match site.pool_box(at) {
                Ok(pool) => site_shapes.push((at.id(site_entry), pool, preload, activation)),
                Err(why) => return self.refuse(&format!("the site: {why}")),
            }
        }
        // The placed fixtures, in the view's list order, each view's own
        // colliders before its touch box.
        let mut fixture_shapes = Vec::new();
        if layout {
            let Some(entry) = self.fixtures else {
                return self.unfed();
            };
            let Some(placed) = find(entry) else {
                return self.unfed();
            };
            if placed.pending_layout {
                return self.unfed();
            }
            let Some(order) = placed.fixture_order.clone() else {
                return self.refuse("a placed fixture collider whose fixture's place in the view's list is not known");
            };
            for (at, slot, touch) in order {
                match placed.pool_box(at) {
                    Ok(pool) => fixture_shapes.push((slot, touch, at, at.id(entry), pool)),
                    Err(why) => return self.refuse(&format!("the placed fixtures: {why}")),
                }
            }
            fixture_shapes.sort_by_key(|&(slot, touch, ..)| (slot, touch));
            if fixture_shapes.windows(2).any(|w| w[0].0 == w[1].0 && w[0].1 == w[1].1) {
                return self.refuse("a fixture with more than one collider of its own or more than one touch box: \
                    their order in its hierarchy is not carried");
            }
        }
        let key: Vec<(usize, bool, [u32; 6])> =
            fixture_shapes.iter().map(|&(slot, touch, _, _, pool)| (slot, touch, pool.map(f32::to_bits))).collect();
        if self.fed {
            if self.fed_fixtures.as_ref() == Some(&key) {
                return;
            }
            return self.refuse("the placed fixtures changed after entry: a layout change's add and remove order \
                is not read");
        }
        let mut by_preload = site_shapes.clone();
        by_preload.sort_by_key(|&(_, _, preload, _)| preload);
        let mut post_order = site_shapes;
        post_order.sort_by_key(|&(_, _, _, activation)| activation);
        let mut spans = Vec::new();
        // The site prefab's clone under the active parent: its colliders in
        // preload-table order.
        let mut first = StaticPruner::new();
        for &(id, pool, ..) in &by_preload {
            first.add(id, pool);
        }
        let mut pruners = vec![first];
        if layout {
            pruners = match boundary(pruners, "from the site's instantiation to the layout load", &mut spans) {
                Ok(p) => p,
                Err(why) => return self.refuse(&why),
            };
            // Each fixture's load adds its own colliders at the prefab's
            // saved root pose and removes them again at once; their boxes
            // there are not read, so both the placed box and the same box
            // about the world origin stand in.
            let mut loaded = Vec::new();
            for p in pruners {
                for about_origin in [false, true] {
                    let mut q = p.clone();
                    for &(_, touch, at, _, pool) in &fixture_shapes {
                        if touch {
                            continue;
                        }
                        let id = ShapeAt { list: ShapeAt::TRANSIENT, ..at }.id(self.fixtures.unwrap_or(0));
                        let pool = if about_origin {
                            let half: [f32; 3] = std::array::from_fn(|k| (pool[k + 3] - pool[k]) * 0.5);
                            [-half[0], -half[1], -half[2], half[0], half[1], half[2]]
                        } else {
                            pool
                        };
                        q.add(id, pool);
                        q.remove(id);
                    }
                    loaded.push(q);
                }
            }
            pruners = distinct(loaded);
            // ShowFixtureAll, then in the same run the site hides.
            for p in &mut pruners {
                for &(_, _, _, id, pool) in &fixture_shapes {
                    p.add(id, pool);
                }
                for &(id, ..) in &post_order {
                    p.remove(id);
                }
            }
        } else {
            // The site hides in the same run as its instantiation.
            for p in &mut pruners {
                for &(id, ..) in &post_order {
                    p.remove(id);
                }
            }
        }
        pruners = match boundary(pruners, "from the site's hiding to its showing on entry", &mut spans) {
            Ok(p) => p,
            Err(why) => return self.refuse(&why),
        };
        // ShowSite: the colliders again, in hierarchy post-order; a query of
        // the entry frame may commit them before the next physics step.
        let mut shown = Vec::new();
        for mut p in pruners {
            for &(id, pool, ..) in &post_order {
                p.add(id, pool);
            }
            let mut committed = p.clone();
            committed.commit();
            shown.push(p);
            shown.push(committed);
        }
        self.variants = distinct(shown);
        self.boundaries = spans;
        self.fed_fixtures = Some(key);
        self.fed = true;
        let pool: Vec<String> = self.variants[0].pool_order().iter().map(|id| format!("{id:x}")).collect();
        self.report(format!("fed: {} site shapes, {} placed fixture shapes; {} pruners over {}; pool [{}]",
            by_preload.len(), fixture_shapes.len(), self.variants.len(),
            self.boundaries.join("; "), pool.join(", ")));
    }

    fn advance(&mut self, dt: f32) {
        if !self.fed {
            return;
        }
        self.clock += f64::from(dt);
        let mut steps = 0;
        while self.clock >= FIXED_STEP && steps < MAX_FIXED_STEPS {
            self.clock -= FIXED_STEP;
            steps += 1;
        }
        if steps == MAX_FIXED_STEPS {
            self.clock = self.clock.min(FIXED_STEP);
        }
        if steps == 0 {
            return;
        }
        for p in &mut self.variants {
            for _ in 0..steps {
                p.step();
            }
        }
        if self.variants.len() > 1 {
            self.variants = distinct(std::mem::take(&mut self.variants));
        }
    }

    /// The pruner's visit order of the touched shapes (pruner ids), or why
    /// it cannot be given. Every pruner is queried, and so committed, as
    /// the engine's query flushes the pruner first.
    fn order(&mut self, touched: &[u64], center: [f32; 3], extents: [f32; 3]) -> Result<Vec<u64>, String> {
        if let Some(why) = &self.unmodeled {
            return Err(why.clone());
        }
        if !self.fed {
            return Err(match self.site {
                None => "no site is in the scene".to_owned(),
                Some((_, true)) => "the home site's entry sequence waits for the placed fixtures".to_owned(),
                Some(_) => "the site's entry sequence is not fed".to_owned(),
            });
        }
        let mut agreed: Option<Vec<u64>> = None;
        let mut split = false;
        for p in &mut self.variants {
            let visits = p.overlap(center, extents).map_err(|why| why.0.to_owned())?;
            let seq: Vec<u64> = visits.into_iter().filter(|id| touched.contains(id)).collect();
            match &agreed {
                None => agreed = Some(seq),
                Some(first) if *first == seq => {}
                Some(_) => split = true,
            }
        }
        self.queries += 1;
        self.agreed += u64::from(!split);
        let beside = self.variants[0].pool_order().len() + UNFED_TRANSIENT_SHAPES > LEAF_SHAPES;
        if self.queries.is_power_of_two() {
            let line = format!("{} queries; the pruners agree on {}; {}", self.queries, self.agreed,
                if beside { format!("no order claimed: {TRANSIENT_SHAPE}") } else { TRANSIENT_ASSUMED.to_owned() });
            self.report(line);
        }
        if beside {
            return Err(TRANSIENT_SHAPE.to_owned());
        }
        if split {
            let why = format!("the pruners of the unread step counts disagree on this query ({})",
                self.boundaries.join("; "));
            self.report(why.clone());
            return Err(why);
        }
        let seq = agreed.unwrap_or_default();
        if seq.len() != touched.len() {
            return Err("a collider the query meets that the pruner does not visit".to_owned());
        }
        self.report(format!("orders given ({} pruners agree)", self.variants.len()));
        Ok(seq)
    }
}

/// Where a shape sits in its installed thing: which list, which index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShapeAt {
    list: u8,
    index: usize,
}

impl ShapeAt {
    const COLLIDERS: u8 = 0;
    const INERT: u8 = 1;
    const UNPORTED: u8 = 2;
    /// A fixture collider's add and removal at its load.
    const TRANSIENT: u8 = 3;

    /// The pruner id of this shape of the installed thing `entry`.
    fn id(self, entry: u64) -> u64 {
        (entry << 32) | (u64::from(self.list) << 24) | (self.index as u64 & 0xff_ffff)
    }
}

impl GroundScene {
    /// A static shape's pruner box, or why the scene cannot place it. The
    /// engine hands the pruner the shape's simulation bounds (its world
    /// bounds at inflation one at the actor's pose composed with the shape's
    /// local pose) and the pruner grows them on every side by a two-hundredth
    /// of their size: the law's `pool_bounds_*`.
    fn pool_box(&self, at: ShapeAt) -> Result<[f32; 6], &'static str> {
        match at.list {
            ShapeAt::COLLIDERS => self.colliders[at.index].pool,
            ShapeAt::INERT | ShapeAt::UNPORTED => Err("a static shape the scene holds only an enclosing bound for, or none"),
            _ => Err("an unknown shape list"),
        }
    }
}

/// Each placement row's slot in the site view's fixture list, from each
/// row's layout type and the y of its layout position. The layout load walks
/// the layouts in the server's order, one per layout type, and puts each
/// layout's rows sorted by that y (equal keys keep their order); so the
/// rows' own order is the server's (the placement mock gives it) and a
/// layout type's place is where its first row stands.
pub(crate) fn fixture_slots(rows: &[(u8, i32)]) -> Vec<usize> {
    let mut groups: Vec<u8> = Vec::new();
    for &(layout, _) in rows {
        if !groups.contains(&layout) {
            groups.push(layout);
        }
    }
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&r| (groups.iter().position(|&g| g == rows[r].0), rows[r].1));
    let mut slots = vec![0; rows.len()];
    for (slot, r) in order.into_iter().enumerate() {
        slots[r] = slot;
    }
    slots
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
    /// module notes), at most `max_shapes`, in the static pruner's visit
    /// order when the pruner gives it. A collider the queries cannot answer
    /// against, on a layer the mask names, refuses the call.
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.last.clear();
        self.refusal = None;
        let mut entries = self.scene.entries_mut();
        let add_order = entries.add_order_holds();
        let mut touched = Vec::new();
        for (at, (entry, scene)) in entries.entries.iter().enumerate() {
            if let Some(u) = scene.unported.iter().find(|u| query.collides_with & u.layers != 0
                && u.bounds.as_ref().map_or(true, |bounds| bounds_meet(bounds, query.center, query.extents))) {
                self.refusal.get_or_insert(u.query_refusal);
            }
            for (i, c) in scene.colliders.iter().enumerate() {
                let named = query.collides_with & c.layers;
                if named == 0 || !bounds_meet(&c.bounds, query.center, query.extents) {
                    continue;
                }
                if named != c.layers {
                    self.refusal.get_or_insert(LAYER_REFUSAL);
                    continue;
                }
                // A position the scene does not carry leaves the order unknown.
                let position = scene.add_positions.as_ref().and_then(|positions| positions.get(i).copied());
                let id = ShapeAt { list: ShapeAt::COLLIDERS, index: i }.id(*entry);
                touched.push((at, position, scene.clone(), i, id));
            }
        }
        // Every query flushes the pruner, whatever it meets.
        let ids: Vec<u64> = touched.iter().map(|&(.., id)| id).collect();
        let pruned = entries.pruner.order(&ids, query.center, query.extents);
        drop(entries);
        let order_known = match pruned {
            Ok(order) => {
                touched.sort_by_key(|&(.., id)| order.iter().position(|&o| o == id));
                true
            }
            Err(_) => {
                let add_order = add_order && touched.iter().all(|&(_, position, ..)| position.is_some());
                if add_order {
                    touched.sort_by_key(|&(at, position, ..)| (at, position));
                }
                touched.len() <= 1 || add_order
            }
        };
        let mut touched: Vec<(Arc<GroundScene>, usize)> =
            touched.into_iter().map(|(_, _, scene, i, _)| (scene, i)).collect();
        let limit = usize::try_from(query.max_shapes.max(0)).unwrap_or(0);
        self.order_known = order_known;
        if touched.len() > limit && !self.order_known {
            self.refusal.get_or_insert("more touches than the shape limit keeps, in an unknown order");
        }
        touched.truncate(limit);
        self.last = touched;
        self.last.iter().map(|(scene, i)| scene.candidate(*i)).collect()
    }

    /// Every collider the mask names. A collider the queries cannot answer
    /// against stands in with its enclosing bounds, or unbounded bounds when
    /// it has none, so a lane that could reach it refuses the call instead of
    /// skipping it.
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.scene.entries().entries.iter().flat_map(|(_, scene)| {
            let unported = scene.unported.iter().filter(|u| collides_with & u.layers != 0).map(|u| Candidate {
                bounds_min: u.bounds.map_or([-f32::MAX; 3], |b| [b[0], b[1], b[2]]),
                bounds_max: u.bounds.map_or([f32::MAX; 3], |b| [b[3], b[4], b[5]]),
                is_trigger: false,
                collider_id: -1,
                body_id: None,
            });
            (0..scene.colliders.len())
                .filter(|&i| collides_with & scene.colliders[i].layers != 0)
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
        let swept = match &c.shape {
            ColliderShape::Mesh(mesh) => law::sweep_sphere(mesh, &c.pose, request.origin, request.sphere_radius,
                request.direction, request.distance, None),
            ColliderShape::Box(half) => law::sweep_sphere_box(*half, &c.pose, request.origin, request.sphere_radius,
                request.direction, request.distance, None),
            // The module's hit record starts with the face index -1, and a
            // sweep hit keeps it (the hit flags ask for no face index).
            ColliderShape::Convex(hull) => law::sweep_sphere_convex(hull, &c.pose, request.origin, request.sphere_radius,
                request.direction, request.distance, u32::MAX, None),
        };
        match swept {
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
