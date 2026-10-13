//! Site objects' camera dither on the home site: `SiteObjectManager`,
//! `SiteObjectView` and `SiteObjectDitherCalculation`.
//!
//! # The source rule
//!
//! - `HomeSiteObstacleController.UpdateView` attaches a site object view
//!   (`AttachSiteObjectView`) to every mesh renderer of each obstacle ring
//!   whose own object name contains `hardwood`, `shrub` or `flower`
//!   (ordinal, case-insensitive). A view takes the renderers under its object
//!   (`GetComponentsInChildren<Renderer>`) and copies their materials; once
//!   its object is active it registers with the manager, which turns its
//!   dither off and writes `_DitherAlpha` 1.
//! - Every frame (`SiteObjectManager.Update`, no hide mode; the parallel
//!   count is 1, so every view active in the hierarchy is a target): the
//!   avatar depth is the absolute view-space depth of the player's position,
//!   a view's camera depth that of its object's position, and the dither
//!   start the camera's near clip plane plus `_siteObjectDitherMinDistance`.
//!   On the home site a view whose camera depth is below the avatar depth
//!   dithers (`SetDitherActive(true)`: the `_DISABLE_DITHER` keyword off) with
//!   `_DitherAlpha` = `_siteObjectFarDitherAlpha` × clamp((depth − start) /
//!   `_siteObjectDitherDistanceLength`, 0, 1); every other target stops
//!   dithering with `_DitherAlpha` 1. The three values are the graphics
//!   configuration's (`graphics-config/site-object-dither.json`).
//!
//! # How the product reproduces it
//!
//! Views that hold the same state share their material copies: a view's
//! primitives take their base material with the view's keyword and alpha,
//! one asset for each distinct (base, keyword, alpha), released once no view
//! holds that state. A base that already holds the state is kept. The Tree
//! program reads the keyword (`SITE_TREE_DITHER`) and the alpha; the
//! FieldObject program dithers on the alpha alone. A view over a primitive
//! of another family is refused by name once; that primitive keeps its
//! material.
//!
//! # Not in this host (named)
//!
//! - Other sites: the views their packages serialize dither only when the
//!   ray from the avatar hits their bounds (`RayOBBJob`), not transcribed;
//!   no view is attached there.
//! - The hide modes (photo shot, phenomena edit, first-person camera) and
//!   the cut-scene game state, in which the source turns a dithering view's
//!   renderers off, and the low-height filter a cut-scene sets: in the
//!   cut-scene state the views keep their state.
//! - The tree fixtures' dither (`TreeFixtureDitherManager`).

use std::collections::{HashMap, HashSet};

use bevy::asset::AssetId;
use bevy::prelude::*;

use moly_assets::json::JsonAsset;

use crate::game_state::{GameStateManager, GameStateType};
use crate::site::{HomeObstacleRing, SiteRoot, SiteVisualPending};
use crate::site_material::{SiteFamily, SiteMaterial, SiteMaterialsSwapped};

/// The graphics configuration's site-object dither document.
const CONFIG_PATH: &str = "moly://graphics-config/site-object-dither.json";

/// `AttachSiteObjectView`'s name tests, in its order.
const NAME_PARTS: [&str; 3] = ["hardwood", "shrub", "flower"];

/// The three site-object values of the graphics configuration.
#[derive(Debug, Clone, Copy)]
struct DitherConfig {
    min_distance: f32,
    distance_length: f32,
    far_alpha: f32,
}

/// The document's three floats, from their serialized bit words.
fn parse_config(text: &str) -> Result<DitherConfig, String> {
    let document: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("does not parse: {error}"))?;
    let word = |field: &str| -> Result<f32, String> {
        let hex = document["bits"][field]
            .as_str()
            .ok_or_else(|| format!("no bits for {field}"))?;
        u32::from_str_radix(hex, 16)
            .map(f32::from_bits)
            .map_err(|error| format!("{field}: {error}"))
    };
    Ok(DitherConfig {
        min_distance: word("_siteObjectDitherMinDistance")?,
        distance_length: word("_siteObjectDitherDistanceLength")?,
        far_alpha: word("_siteObjectFarDitherAlpha")?,
    })
}

/// The document's load, kept by the dither system.
#[derive(Default)]
struct ConfigLoad {
    handle: Option<Handle<JsonAsset>>,
    settled: Option<Result<DitherConfig, String>>,
}

impl ConfigLoad {
    fn poll(&mut self, server: &AssetServer, documents: &Assets<JsonAsset>) -> Option<DitherConfig> {
        if let Some(settled) = &self.settled {
            return settled.as_ref().ok().copied();
        }
        let handle = self
            .handle
            .get_or_insert_with(|| server.load(CONFIG_PATH))
            .clone();
        let settled = if server.load_state(&handle).is_failed() {
            Err(format!("{CONFIG_PATH} did not load"))
        } else if let Some(document) = documents.get(&handle) {
            parse_config(&document.0)
        } else {
            return None;
        };
        match &settled {
            Ok(config) => info!(
                "[site-object-dither] min distance {} (bits {:08x}), distance length {} (bits {:08x}), far alpha {} (bits {:08x}) from the graphics configuration",
                config.min_distance,
                config.min_distance.to_bits(),
                config.distance_length,
                config.distance_length.to_bits(),
                config.far_alpha,
                config.far_alpha.to_bits()
            ),
            Err(reason) => warn!("[site-object-dither] refused: {reason}; the site objects keep their materials"),
        }
        let config = settled.as_ref().ok().copied();
        self.settled = Some(settled);
        config
    }
}

/// `SiteObjectView`: the primitives under the view's object with their base
/// materials, and the state last written (`None`: the registration state,
/// dither off and alpha 1, not written yet).
#[derive(Component)]
pub(crate) struct SiteObjectView {
    primitives: Vec<(Entity, Handle<SiteMaterial>)>,
    written: Option<(bool, u32)>,
}

/// A ring whose views are attached.
#[derive(Component)]
pub(crate) struct SiteObjectViewsAttached;

/// The material of `base` holding the view state, or `None` when `base`
/// already holds it. `Err` names a family without the dither axes.
fn with_state(base: &SiteMaterial, active: bool, alpha: f32) -> Result<Option<SiteMaterial>, SiteFamily> {
    match base.key.family {
        SiteFamily::Tree => {
            if !active && !base.key.tree_dither {
                return Ok(None);
            }
            let mut material = base.clone();
            material.key.tree_dither = active;
            material.params.dither_alpha = alpha;
            Ok(Some(material))
        }
        SiteFamily::FieldObject => {
            if base.params.dither_alpha.to_bits() == alpha.to_bits() {
                return Ok(None);
            }
            let mut material = base.clone();
            material.params.dither_alpha = alpha;
            Ok(Some(material))
        }
        family => Err(family),
    }
}

/// Update: `AttachSiteObjectView` on each obstacle ring once the site
/// materials are on its primitives.
#[allow(clippy::type_complexity)]
fn attach_views(
    mut commands: Commands,
    swapped: Option<Res<SiteMaterialsSwapped>>,
    rings: Query<
        Entity,
        (
            With<HomeObstacleRing>,
            With<SiteRoot>,
            Without<SiteVisualPending>,
            Without<SiteObjectViewsAttached>,
        ),
    >,
    children: Query<&Children>,
    names: Query<&Name>,
    primitives: Query<&MeshMaterial3d<SiteMaterial>>,
) {
    if swapped.is_none() {
        return;
    }
    for ring in &rings {
        let mut views = 0;
        let mut covered = 0;
        for node in children.iter_descendants(ring) {
            let Ok(name) = names.get(node) else {
                continue;
            };
            let lower = name.as_str().to_ascii_lowercase();
            if !NAME_PARTS.iter().any(|part| lower.contains(part)) {
                continue;
            }
            // A mesh renderer on the object itself: a primitive child.
            let renders = children
                .get(node)
                .is_ok_and(|kids| kids.iter().any(|kid| primitives.contains(kid)));
            if !renders {
                continue;
            }
            let under: Vec<_> = children
                .iter_descendants(node)
                .filter_map(|entity| primitives.get(entity).ok().map(|handle| (entity, handle.0.clone())))
                .collect();
            covered += under.len();
            views += 1;
            commands.entity(node).insert(SiteObjectView {
                primitives: under,
                written: None,
            });
        }
        commands.entity(ring).insert(SiteObjectViewsAttached);
        info!("[site-object-dither] ring {ring:?}: {views} site object views over {covered} primitives");
    }
}

/// Update: `SiteObjectManager.Update` with no hide mode, over the views
/// active in the hierarchy.
#[allow(clippy::too_many_arguments)]
fn update_dither(
    mut views: Query<(&mut SiteObjectView, &GlobalTransform, &InheritedVisibility)>,
    mut handles: Query<&mut MeshMaterial3d<SiteMaterial>>,
    mut materials: ResMut<Assets<SiteMaterial>>,
    players: Query<&GlobalTransform, With<crate::player::PlayerControlled>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    game_state: Option<Res<GameStateManager>>,
    server: Res<AssetServer>,
    documents: Res<Assets<JsonAsset>>,
    mut config: Local<ConfigLoad>,
    mut copies: Local<HashMap<(AssetId<SiteMaterial>, bool, u32), Handle<SiteMaterial>>>,
    mut refused: Local<HashSet<AssetId<SiteMaterial>>>,
    time: Res<Time>,
    mut reported: Local<(f32, Option<(usize, usize, usize)>)>,
) {
    if views.is_empty() {
        copies.clear();
        return;
    }
    let Some(config) = config.poll(&server, &documents) else {
        return;
    };
    if game_state.is_some_and(|state| state.current_type() == GameStateType::CutScene) {
        return;
    }
    let Some((view_from_world, near)) = cameras
        .iter()
        .find(|(camera, _, _)| camera.is_active)
        .and_then(|(_, transform, projection)| match projection {
            Projection::Perspective(p) => Some((transform.affine().inverse(), p.near)),
            _ => None,
        })
    else {
        return;
    };
    let Some(player) = players.iter().next() else {
        return;
    };
    let depth = |point: Vec3| view_from_world.transform_point3(point).z.abs();
    let avatar_depth = depth(player.translation());
    let start = near + config.min_distance;
    let mut used = HashSet::new();
    let (mut targets, mut far, mut ramp) = (0, 0, 0);
    for (mut view, transform, visible) in &mut views {
        if !visible.get() {
            continue;
        }
        targets += 1;
        let camera_depth = depth(transform.translation());
        let (active, alpha) = if camera_depth - avatar_depth < 0.0 {
            let ratio = (camera_depth - start) / config.distance_length;
            let clamped = if ratio < 0.0 { 0.0 } else { ratio.min(1.0) };
            (true, config.far_alpha * clamped)
        } else {
            (false, 1.0)
        };
        if active {
            if alpha == config.far_alpha { far += 1 } else { ramp += 1 }
        }
        let state = (active, alpha.to_bits());
        if view.written == Some(state) {
            for (_, base) in &view.primitives {
                used.insert((base.id(), active, alpha.to_bits()));
            }
            continue;
        }
        for (entity, base) in &view.primitives {
            let key = (base.id(), active, alpha.to_bits());
            used.insert(key);
            let target = if let Some(copy) = copies.get(&key) {
                copy.clone()
            } else {
                let Some(base_material) = materials.get(base) else {
                    continue;
                };
                match with_state(base_material, active, alpha) {
                    Ok(None) => base.clone(),
                    Ok(Some(material)) => {
                        let copy = materials.add(material);
                        copies.insert(key, copy.clone());
                        copy
                    }
                    Err(family) => {
                        if refused.insert(base.id()) {
                            warn!("[site-object-dither] a site object view over a {family:?} material: the family has no dither axes here; the primitive keeps its material");
                        }
                        continue;
                    }
                }
            };
            if let Ok(mut handle) = handles.get_mut(*entity) {
                if handle.0 != target {
                    handle.0 = target;
                }
            }
        }
        view.written = Some(state);
    }
    copies.retain(|key, _| used.contains(key));
    let now = time.elapsed_secs();
    let counts = (targets, far, ramp);
    if reported.1 != Some(counts) && now - reported.0 >= 2.0 {
        *reported = (now, Some(counts));
        info!(
            "[site-object-dither] targets {targets}: dithering at the far alpha {far}, on the near ramp {ramp}; avatar depth {avatar_depth:.2}, start {start:.2}; material copies {}",
            copies.len()
        );
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(
        Update,
        (attach_views, update_dither)
            .chain()
            .after(crate::site_material::reveal_material_ready_roots),
    );
}
