//! While the talk lens frames a conversation, a placed fixture standing
//! between the eye and a cast member fades out with the fixture programs' own
//! Bayer dither: its materials take the dither variant (`_DisableDither` off)
//! with a lowered `_DitherAlpha`, and go back to their own materials once the
//! fixture no longer stands in the way. Site-family materials on a fixture
//! take the site programs' dither state the same way. The fixture a single
//! character is talking about is the subject of the shot and never fades.
//!
//! A fixture blocks when the segment from the eye to a cast member's body
//! passes through the fixture's bounds short of the body. Alphas are held per
//! dither level, so a fading fixture changes material only when its alpha
//! crosses one of the sixteen Bayer thresholds; one material copy serves
//! every mesh that shares a base and a level.
//!
//! The emission pass draws from its own object pool and keeps a fixture's
//! glowing parts as they are.

use std::collections::HashMap;

use bevy::asset::AssetId;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use moly_features::site_object_dither::{dither_level, level_alpha, with_state};
use moly_render::fixture_material::FixtureMaterial;
use moly_render::render_inputs::FixtureRoot;
use moly_render::site_material::SiteMaterial;

use super::talk_lens::TalkLens;
use crate::camera::FreeCameraPose;

/// The alpha a blocking fixture fades to.
const CLEAR_ALPHA: f32 = 0.3;
/// Alpha change per second while a fixture fades out or back in.
const FADE_PER_SECOND: f32 = 3.0;
/// Heights above a cast member's feet the eye must see.
const BODY_HEIGHTS: [f32; 2] = [0.35, 0.8];
/// A fixture this close behind the body along the line of sight does not
/// count as blocking it (the body stands at or in front of it).
const BODY_MARGIN: f32 = 0.25;
/// The dither level of alpha 1: every fragment drawn.
const OPAQUE: u32 = 16;

/// Each placed fixture's world bounds (the union of its meshes' bounds) and
/// its meshes, measured while the talk lens is on.
#[derive(Resource, Default)]
pub struct FixtureBounds {
    bounds: HashMap<Entity, (Vec3, Vec3)>,
    meshes: HashMap<Entity, Vec<Entity>>,
    owner: HashMap<Entity, Option<Entity>>,
}

impl FixtureBounds {
    /// The world bounds (min, max) of a placed fixture's root.
    pub fn of(&self, root: Entity) -> Option<(Vec3, Vec3)> {
        self.bounds.get(&root).copied()
    }
}

/// The fixtures faded so far, the materials their meshes had, and the shared
/// material copies per base and dither level.
#[derive(Default)]
pub struct LensClearState {
    alpha: HashMap<Entity, f32>,
    fixture_base: HashMap<Entity, Handle<FixtureMaterial>>,
    site_base: HashMap<Entity, Handle<SiteMaterial>>,
    fixture_copies: HashMap<(AssetId<FixtureMaterial>, u32), Handle<FixtureMaterial>>,
    site_copies: HashMap<(AssetId<SiteMaterial>, u32), Handle<SiteMaterial>>,
}

/// Measures the placed fixtures while the talk lens is on (and while a faded
/// fixture still has to come back).
pub fn measure_fixtures(
    lens: Option<Res<TalkLens>>,
    meshes: Query<(Entity, &GlobalTransform, &Aabb), Or<(With<MeshMaterial3d<FixtureMaterial>>, With<MeshMaterial3d<SiteMaterial>>)>>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<FixtureRoot>>,
    mut fixtures: ResMut<FixtureBounds>,
) {
    if lens.is_none() && fixtures.bounds.is_empty() {
        return;
    }
    let fixtures = &mut *fixtures;
    fixtures.bounds.clear();
    fixtures.meshes.clear();
    if lens.is_none() {
        fixtures.owner.clear();
        return;
    }
    fixtures.owner.retain(|mesh, _| meshes.contains(*mesh));
    for (mesh, transform, aabb) in &meshes {
        let owner = *fixtures
            .owner
            .entry(mesh)
            .or_insert_with(|| fixture_root(mesh, &parents, &roots));
        let Some(root) = owner else { continue };
        let (min, max) = world_bounds(transform, aabb);
        let entry = fixtures.bounds.entry(root).or_insert((min, max));
        entry.0 = entry.0.min(min);
        entry.1 = entry.1.max(max);
        fixtures.meshes.entry(root).or_default().push(mesh);
    }
}

pub fn clear_view(
    lens: Option<Res<TalkLens>>,
    talk: Option<Res<crate::talk::ActiveTalk>>,
    pose: Res<FreeCameraPose>,
    time: Res<Time>,
    bodies: Query<&GlobalTransform>,
    fixtures: Res<FixtureBounds>,
    mut fixture_handles: Query<&mut MeshMaterial3d<FixtureMaterial>>,
    mut site_handles: Query<&mut MeshMaterial3d<SiteMaterial>>,
    mut fixture_materials: ResMut<Assets<FixtureMaterial>>,
    mut site_materials: ResMut<Assets<SiteMaterial>>,
    mut state: Local<LensClearState>,
) {
    let looking = lens.as_deref().is_some_and(|lens| lens.clear);
    let eye = pose.0.map(|pose| pose.eye);
    let targets: Vec<Vec3> = match (looking, eye, talk.as_deref()) {
        (true, Some(_), Some(talk)) => talk
            .participants()
            .iter()
            .filter_map(|(_, body)| bodies.get(*body).ok().map(GlobalTransform::translation))
            .flat_map(|feet| BODY_HEIGHTS.map(|height| feet + Vec3::Y * height))
            .collect(),
        _ => Vec::new(),
    };
    if targets.is_empty() && state.alpha.is_empty() {
        return;
    }
    let bounds = &fixtures.bounds;
    let members = &fixtures.meshes;
    if bounds.is_empty() {
        // The lens is off: every faded mesh takes its own material back now.
        for (mesh, base) in state.fixture_base.drain() {
            if let Ok(mut handle) = fixture_handles.get_mut(mesh) {
                handle.0 = base;
            }
        }
        for (mesh, base) in state.site_base.drain() {
            if let Ok(mut handle) = site_handles.get_mut(mesh) {
                handle.0 = base;
            }
        }
        state.alpha.clear();
        state.fixture_copies.clear();
        state.site_copies.clear();
        return;
    }

    // A single character talking about a fixture: that fixture is what the
    // shot shows, so it never fades.
    let subject: Vec<Entity> = match talk.as_deref() {
        Some(talk) if talk.participants().len() == 1 => {
            talk.fixture_instances().iter().map(|(_, fixture)| *fixture).collect()
        }
        _ => Vec::new(),
    };
    let step = FADE_PER_SECOND * time.delta_secs();
    let mut levels: Vec<(Entity, u32)> = Vec::new();
    for (&root, &(min, max)) in bounds {
        let blocking = !subject.contains(&root)
            && eye.is_some_and(|eye| targets.iter().any(|&body| blocks(eye, body, min, max)));
        let current = state.alpha.get(&root).copied().unwrap_or(1.0);
        let wanted = if blocking { CLEAR_ALPHA } else { 1.0 };
        let alpha = if current < wanted { (current + step).min(wanted) } else { (current - step).max(wanted) };
        if alpha >= 1.0 {
            if state.alpha.remove(&root).is_some() {
                levels.push((root, OPAQUE));
            }
        } else {
            state.alpha.insert(root, alpha);
            levels.push((root, dither_level(alpha)));
        }
    }
    // A fixture that left the scene is no longer tracked; its meshes went
    // with it.
    state.alpha.retain(|root, _| bounds.contains_key(root));

    for (root, level) in levels {
        for &mesh in members.get(&root).into_iter().flatten() {
            if let Ok(mut handle) = fixture_handles.get_mut(mesh) {
                set_fixture(&mut state, mesh, &mut handle, level, &mut fixture_materials);
            }
            if let Ok(mut handle) = site_handles.get_mut(mesh) {
                set_site(&mut state, mesh, &mut handle, level, &mut site_materials);
            }
        }
    }
    if state.alpha.is_empty() {
        state.fixture_base.clear();
        state.site_base.clear();
        state.fixture_copies.clear();
        state.site_copies.clear();
    }
}

/// The placed fixture a mesh belongs to, if any.
fn fixture_root(mesh: Entity, parents: &Query<&ChildOf>, roots: &Query<(), With<FixtureRoot>>) -> Option<Entity> {
    let mut at = mesh;
    loop {
        if roots.contains(at) {
            return Some(at);
        }
        at = parents.get(at).ok()?.parent();
    }
}

fn world_bounds(transform: &GlobalTransform, aabb: &Aabb) -> (Vec3, Vec3) {
    let centre = Vec3::from(aabb.center);
    let half = Vec3::from(aabb.half_extents);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for corner in 0..8 {
        let sign = Vec3::new(
            if corner & 1 == 0 { -1.0 } else { 1.0 },
            if corner & 2 == 0 { -1.0 } else { 1.0 },
            if corner & 4 == 0 { -1.0 } else { 1.0 },
        );
        let point = transform.transform_point(centre + half * sign);
        min = min.min(point);
        max = max.max(point);
    }
    (min, max)
}

/// Whether the segment from `eye` to `body` passes through the box short of
/// the body.
fn blocks(eye: Vec3, body: Vec3, min: Vec3, max: Vec3) -> bool {
    let along = body - eye;
    let length = along.length();
    if length <= BODY_MARGIN {
        return false;
    }
    let mut enter = 0.0_f32;
    let mut leave = length;
    let direction = along / length;
    for axis in 0..3 {
        let (origin, step, low, high) = (eye[axis], direction[axis], min[axis], max[axis]);
        if step.abs() < 1e-6 {
            if origin < low || origin > high {
                return false;
            }
            continue;
        }
        let (a, b) = ((low - origin) / step, (high - origin) / step);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
        if enter > leave {
            return false;
        }
    }
    enter < length - BODY_MARGIN
}

fn set_fixture(
    state: &mut LensClearState,
    mesh: Entity,
    handle: &mut MeshMaterial3d<FixtureMaterial>,
    level: u32,
    materials: &mut Assets<FixtureMaterial>,
) {
    if level >= OPAQUE {
        if let Some(base) = state.fixture_base.remove(&mesh) {
            handle.0 = base;
        }
        return;
    }
    let base = state.fixture_base.entry(mesh).or_insert_with(|| handle.0.clone()).clone();
    let key = (base.id(), level);
    let copy = match state.fixture_copies.get(&key) {
        Some(copy) => copy.clone(),
        None => {
            let Some(mut material) = materials.get(&base).cloned() else { return };
            material.key.dither = true;
            material.params.dither_alpha = level_alpha(level);
            let copy = materials.add(material);
            state.fixture_copies.insert(key, copy.clone());
            copy
        }
    };
    if handle.0 != copy {
        handle.0 = copy;
    }
}

fn set_site(
    state: &mut LensClearState,
    mesh: Entity,
    handle: &mut MeshMaterial3d<SiteMaterial>,
    level: u32,
    materials: &mut Assets<SiteMaterial>,
) {
    if level >= OPAQUE {
        if let Some(base) = state.site_base.remove(&mesh) {
            handle.0 = base;
        }
        return;
    }
    let base = state.site_base.entry(mesh).or_insert_with(|| handle.0.clone()).clone();
    let key = (base.id(), level);
    let copy = match state.site_copies.get(&key) {
        Some(copy) => copy.clone(),
        None => {
            let Some(material) = materials.get(&base) else { return };
            // A family without the dither axes keeps its material.
            let Ok(Some(material)) = with_state(material, true, level_alpha(level)) else { return };
            let copy = materials.add(material);
            state.site_copies.insert(key, copy.clone());
            copy
        }
    };
    if handle.0 != copy {
        handle.0 = copy;
    }
}
