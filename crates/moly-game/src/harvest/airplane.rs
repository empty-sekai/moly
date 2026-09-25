//! The paper airplane: the transported treasure box's delivery
//! (`PaperAirplanePresenter` / `PaperAirplaneView`).
//!
//! `HarvestSiteController.Initialize` (outside the tutorial) sets up one
//! airplane per user treasure box still before_spawned, with no site filter:
//! the view is loaded under the site and set inactive, then
//! 1. `WaitUpdatePaperComeTime`: every frame (`Yield(Update)`) it stops once
//!    the elapsed time reaches the box's `transportSeconds` or the player is
//!    not on a harvest site; otherwise it adds the frame time, only while the
//!    current site is the box's site. The elapsed time belongs to the
//!    presenter, which each site load makes anew, so the count restarts on
//!    every arrival;
//! 2. `WaitUntil(IsPlayerOnHarvestSite)` (GameState not SiteMove and a
//!    harvest-category site);
//! 3. the player's site-local position, truncated toward zero, goes to the
//!    spawn API (`TreasureBoxSpawnMock`); a failed reply ends the airplane;
//! 4. the box row (spawned, this site and seq) and the site's map are read
//!    back; a raycast straight down from 50 m above the box's position gives
//!    the landing point (a miss logs and creates no box);
//! 5. `AppearAirplane`: the flying notice (icon, `se_ui_notice_treasurebox`,
//!    text: the UI lane's), the view active at the landing point, one yaw
//!    draw `Random.Range(0, 360)`, then the Animator plays
//!    `paper_airplane_movement` to its end;
//! 6. `CreateTreasureBox`: the map's fixture row at the box's (x, z) through
//!    the ordinary placement, `SetPosition(landing point)`,
//!    `se_spawn_tresure_transform`;
//! 7. `Delay(1.0 s)`, then the view inactive.
//!
//! The clip is evaluated from the package document: the AirPlane node's
//! translation and Euler rotation curves, the renderer switch, and the
//! activation / emission switches of the three effect groups (the glb carries
//! the translation alone). The document stores each curve as segments
//! `(start time, [a, b, c, d])` with the value `((a s + b) s + c) s + d` at `s`
//! seconds into the segment; before the first segment the first value holds,
//! from the last one its value holds.
//!
//! Named gaps: the effect groups' particles are not drawn (their switches are
//! logged); the flying notice's icon and text; the airplane keeps its glb
//! materials; the raycast is the harvest placement's ground height (the
//! highest ground vertex within 2 m), not a physics raycast; a switch counts
//! as on above 0.5 (the document's values are 0 and 1); the frame that starts
//! the clip shows time 0 and each later frame adds its time.

use std::sync::Arc;

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

use super::catalog::{HarvestCatalog, HarvestUserData};
use super::server_mock::{HarvestServerMock, BOX_BEFORE_SPAWNED, BOX_SPAWNED};
use crate::audio::SeRequests;
use crate::player::PlayerControlled;
use crate::site_move::timeline::Delay;

const GLB: &str = "moly://site/props/paper_airplane/paper_airplane.glb";
const DOCUMENT: &str = "moly://site/props/paper_airplane/paper_airplane.json";
/// `ANIMATION_STATE_NAME`.
const CLIP: &str = "paper_airplane_movement";
const PLANE_NODE: &str = "AirPlane";
/// The raycast starts this far above the box position.
const RAY_HEIGHT: f32 = 50.0;
/// `AppearAirplane`: `Delay(1.0 s)` after the transformation, then inactive.
const HIDE_DELAY: f32 = 1.0;

/// One document curve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Curve(pub(crate) Vec<(f32, [f32; 4])>);

impl Curve {
    pub(crate) fn eval(&self, t: f32) -> f32 {
        let keys = &self.0;
        let Some(first) = keys.first() else {
            return 0.0;
        };
        let index = keys.partition_point(|(start, _)| *start <= t);
        if index == 0 {
            return first.1[3];
        }
        let (start, [a, b, c, d]) = keys[index - 1];
        if index == keys.len() {
            return d;
        }
        let s = t - start;
        ((a * s + b) * s + c) * s + d
    }

    /// The largest gap between a segment's value at the next segment's start
    /// and that segment's own first value (a reading check of the format).
    fn continuity(&self) -> f32 {
        self.0
            .windows(2)
            .map(|pair| {
                let (start, [a, b, c, d]) = pair[0];
                let s = pair[1].0 - start;
                (((a * s + b) * s + c) * s + d - pair[1].1[3]).abs()
            })
            .fold(0.0, f32::max)
    }
}

/// `paper_airplane_movement` as the document binds it.
#[derive(Clone, Debug)]
pub(crate) struct AirplaneClip {
    pub(crate) length: f32,
    translation: [Curve; 3],
    euler: [Curve; 3],
    /// `m_Enabled` of the AirPlane MeshRenderer.
    renderer: Curve,
    /// `(node, attribute, curve)` of the effect groups' switches.
    switches: Vec<(String, String, Curve)>,
}

impl AirplaneClip {
    /// The AirPlane node's local pose at `t` in the product frame (source x
    /// reflected: the translation's x negates; `Quaternion.Euler(x, y, z)`
    /// applies z, then x, then y, and the reflection negates the y and z
    /// angles).
    pub(crate) fn plane_pose(&self, t: f32) -> (Vec3, Quat) {
        let [tx, ty, tz] = [0, 1, 2].map(|i| self.translation[i].eval(t));
        let [ex, ey, ez] = [0, 1, 2].map(|i| self.euler[i].eval(t).to_radians());
        (
            Vec3::new(-tx, ty, tz),
            Quat::from_rotation_y(-ey) * Quat::from_rotation_x(ex) * Quat::from_rotation_z(-ez),
        )
    }

    fn renderer_on(&self, t: f32) -> bool {
        self.renderer.eval(t) > 0.5
    }

    fn switch_states(&self, t: f32) -> Vec<bool> {
        self.switches
            .iter()
            .map(|(_, _, curve)| curve.eval(t) > 0.5)
            .collect()
    }

    /// Over the transform curves (the switches step by design).
    fn continuity(&self) -> (usize, f32) {
        let curves = self.translation.iter().chain(self.euler.iter());
        curves.fold((0, 0.0), |(keys, gap), curve| {
            (keys + curve.0.len(), gap.max(curve.continuity()))
        })
    }
}

/// Read the clip from the package document; refuse what this reading does not
/// cover (another clip, another curve kind, a binding outside the AirPlane
/// transform, its renderer switch and the effect switches).
pub(crate) fn parse_clip(document: &serde_json::Value) -> Result<AirplaneClip, String> {
    let clips = document["animations"]["clips"]
        .as_array()
        .ok_or("no animations.clips")?;
    let [clip] = clips.as_slice() else {
        return Err(format!("{} clips, expected one", clips.len()));
    };
    if clip["name"].as_str() != Some(CLIP) {
        return Err(format!("clip {:?} is not {CLIP}", clip["name"]));
    }
    let length = clip["stopTime"].as_f64().ok_or("no stopTime")? as f32;
    let mut translation: [Option<Curve>; 3] = Default::default();
    let mut euler: [Option<Curve>; 3] = Default::default();
    let mut renderer = None;
    let mut switches = Vec::new();
    for curve in clip["curves"].as_array().ok_or("no curves")? {
        if curve["kind"].as_str() != Some("cubic") {
            return Err(format!("curve kind {:?}", curve["kind"]));
        }
        let keys = curve["keys"]
            .as_array()
            .ok_or("curve without keys")?
            .iter()
            .map(|key| {
                let start = key[0].as_f64().ok_or("key time")? as f32;
                let mut c = [0.0f32; 4];
                for (i, slot) in c.iter_mut().enumerate() {
                    *slot = key[1][i].as_f64().ok_or("key coefficient")? as f32;
                }
                Ok((start, c))
            })
            .collect::<Result<Vec<_>, &str>>()?;
        let node = curve["node"].as_str().ok_or("curve without node")?;
        let attribute = curve["attribute"]
            .as_str()
            .ok_or("curve without attribute")?;
        let component = curve["component"]
            .as_u64()
            .ok_or("curve without component")? as usize;
        match (node, attribute) {
            (PLANE_NODE, "translation") if component < 3 => {
                translation[component] = Some(Curve(keys))
            }
            (PLANE_NODE, "eulerAngles") if component < 3 => euler[component] = Some(Curve(keys)),
            (PLANE_NODE, "m_Enabled") => renderer = Some(Curve(keys)),
            (_, "m_IsActive" | "EmissionModule.enabled")
                if node.starts_with("AirPlane/fx_act_box_") =>
            {
                switches.push((node.to_owned(), attribute.to_owned(), Curve(keys)))
            }
            _ => {
                return Err(format!(
                    "binding {node} {attribute} {component} is outside this reading"
                ))
            }
        }
    }
    let take = |slots: [Option<Curve>; 3], what: &str| -> Result<[Curve; 3], String> {
        let [x, y, z] = slots;
        Ok([
            x.ok_or(format!("no {what} x"))?,
            y.ok_or(format!("no {what} y"))?,
            z.ok_or(format!("no {what} z"))?,
        ])
    };
    Ok(AirplaneClip {
        length,
        translation: take(translation, "translation")?,
        euler: take(euler, "eulerAngles")?,
        renderer: renderer.ok_or("no renderer switch")?,
        switches,
    })
}

enum Stage {
    /// `WaitUpdatePaperComeTime`.
    Waiting,
    /// `WaitUntil(IsPlayerOnHarvestSite)`.
    WaitSite,
    /// `AppearAirplane`'s clip.
    Flying {
        time: f32,
        started: f64,
        renderer: bool,
        switches: Vec<bool>,
    },
    /// `Delay(1.0 s)` after the transformation.
    Hiding {
        delay: Delay,
    },
    Done,
}

struct Airplane {
    seq: i32,
    site_id: u32,
    transport: f32,
    elapsed: f32,
    stage: Stage,
    root: Option<Entity>,
    plane: Option<Entity>,
    plane_meshes: Vec<Entity>,
    end: Vec3,
    /// The box row read back after the spawn reply.
    box_position: (i32, i32),
}

/// The airplanes of the current harvest-site stay.
#[derive(Resource, Default)]
pub(crate) struct PaperAirplanes {
    epoch: Option<u64>,
    site_id: Option<u32>,
    runs: Vec<Airplane>,
    glb: Option<Handle<Gltf>>,
    document: Option<Handle<JsonAsset>>,
    clip: Option<Result<Arc<AirplaneClip>, String>>,
    rng: Option<super::Rng>,
}

impl PaperAirplanes {
    /// The site goes: its airplanes go with it (the presenters are destroyed
    /// and their cancellation fires).
    pub(crate) fn clear(&mut self, commands: &mut Commands) {
        for run in self.runs.drain(..) {
            if let Some(root) = run.root {
                commands.entity(root).despawn();
            }
        }
        self.site_id = None;
    }
}

#[derive(SystemParam)]
pub(crate) struct AirplaneInputs<'w> {
    time: Res<'w, Time>,
    frames: Res<'w, FrameCount>,
    server: Res<'w, AssetServer>,
    gltfs: Res<'w, Assets<Gltf>>,
    json: Res<'w, Assets<JsonAsset>>,
    site: Option<Res<'w, crate::site::SiteActive>>,
    epoch: Option<Res<'w, crate::site::GroundEpoch>>,
    site_move: Option<Res<'w, crate::site_move::SiteMoveActive>>,
    catalog: Option<Res<'w, HarvestCatalog>>,
    user: Option<ResMut<'w, HarvestUserData>>,
    mock: Option<ResMut<'w, HarvestServerMock>>,
    arrival: ResMut<'w, super::arrival::HarvestArrival>,
    ground: Res<'w, super::HarvestGroundVerts>,
    se: ResMut<'w, SeRequests>,
}

#[derive(SystemParam)]
pub(crate) struct AirplaneScene<'w, 's> {
    players: Query<'w, 's, &'static Transform, With<PlayerControlled>>,
    names: Query<'w, 's, (Entity, &'static Name)>,
    children: Query<'w, 's, &'static Children>,
    meshes: Query<'w, 's, (), With<Mesh3d>>,
    transforms: Query<'w, 's, &'static mut Transform, Without<PlayerControlled>>,
    visibility: Query<'w, 's, &'static mut Visibility>,
}

/// Update, after the arrival: set up, wait, fly and transform.
pub(crate) fn advance(
    mut commands: Commands,
    mut airplanes: ResMut<PaperAirplanes>,
    inputs: AirplaneInputs,
    scene: AirplaneScene,
) {
    let AirplaneInputs {
        time,
        frames,
        server,
        gltfs,
        json,
        site,
        epoch,
        site_move,
        catalog,
        mut user,
        mut mock,
        mut arrival,
        ground,
        mut se,
    } = inputs;
    let AirplaneScene {
        players,
        names,
        children,
        meshes,
        mut transforms,
        mut visibility,
    } = scene;
    let (Some(site), Some(epoch), Some(catalog), Some(user), Some(mock)) = (
        site.as_deref(),
        epoch.as_deref(),
        catalog,
        user.as_deref_mut(),
        mock.as_deref_mut(),
    ) else {
        return;
    };
    let dt = time.delta_secs();
    let frame = u64::from(frames.0);
    let on_harvest_site = site_move.is_none() && site.category == "harvest";

    // A new site settles: the previous site's airplanes go; a harvest site
    // sets up one per unspawned box.
    if airplanes.epoch != Some(epoch.0) {
        airplanes.epoch = Some(epoch.0);
        airplanes.clear(&mut commands);
        if site.category == "harvest" {
            airplanes.site_id = Some(site.site_id);
            airplanes.rng = Some(super::Rng(
                0x5041_5045_0000_0000 ^ ((site.site_id as u64) << 32) ^ epoch.0,
            ));
            let unspawned: Vec<_> = user
                .boxes
                .iter()
                .filter(|row| row.status == BOX_BEFORE_SPAWNED)
                .cloned()
                .collect();
            if !unspawned.is_empty() {
                airplanes.glb.get_or_insert_with(|| {
                    moly_assets::residency::load_gltf(
                        &server,
                        bevy::asset::AssetPath::from(GLB),
                        moly_assets::residency::GltfResidency::GpuTextures,
                    )
                });
                airplanes
                    .document
                    .get_or_insert_with(|| server.load(DOCUMENT));
            }
            for row in unspawned {
                info!(
                    "[harvest-airplane] GenerateUnclaimedTreasureBox site {}: box seq {} (site {}, {} {}, transportSeconds {}) -> StartTresureBox, the paper airplane view set up inactive",
                    site.site_id, row.seq, row.site_id, row.refresh_type, row.status, row.transport_seconds
                );
                // Setup(uid, box, siteTransform, siteId): the site being
                // loaded, not the box row's.
                airplanes.runs.push(Airplane {
                    seq: row.seq,
                    site_id: site.site_id,
                    transport: row.transport_seconds as f32,
                    elapsed: 0.0,
                    stage: Stage::Waiting,
                    root: None,
                    plane: None,
                    plane_meshes: Vec::new(),
                    end: Vec3::ZERO,
                    box_position: (0, 0),
                });
            }
        }
    }
    if airplanes.runs.is_empty() {
        return;
    }

    // The clip, once.
    if airplanes.clip.is_none() {
        if let Some(document) = airplanes.document.as_ref().and_then(|h| json.get(h)) {
            let parsed = serde_json::from_str::<serde_json::Value>(&document.0)
                .map_err(|error| format!("not JSON: {error}"))
                .and_then(|value| parse_clip(&value))
                .map(Arc::new);
            match &parsed {
                Ok(clip) => {
                    let (keys, gap) = clip.continuity();
                    info!(
                        "[harvest-airplane] {CLIP} read from the document: {:.4} s, {} switches; transform curves {keys} segments, largest gap between a segment's end and the next segment's first value {gap:.6}",
                        clip.length,
                        clip.switches.len()
                    );
                }
                Err(reason) => {
                    warn!("[harvest-airplane] {CLIP} refused: {reason}; the airplane cannot fly")
                }
            }
            airplanes.clip = Some(parsed);
        }
    }
    let clip = airplanes
        .clip
        .as_ref()
        .and_then(|clip| clip.as_ref().ok())
        .cloned();
    let scene = airplanes
        .glb
        .as_ref()
        .and_then(|handle| gltfs.get(handle))
        .and_then(|gltf| gltf.scenes.first().cloned());
    let player = players.single().ok().map(|transform| transform.translation);
    let verts = ground.0.clone();
    let PaperAirplanes { runs, rng, .. } = &mut *airplanes;
    for run in runs.iter_mut() {
        // The view: instantiated under the site, inactive (Setup).
        if run.root.is_none() {
            if let Some(scene) = scene.clone() {
                run.root = Some(
                    commands
                        .spawn((SceneRoot(scene), Transform::default(), Visibility::Hidden))
                        .id(),
                );
            }
        }
        if run.plane.is_none() {
            if let Some(root) = run.root {
                if let Some(plane) = find_named(root, PLANE_NODE, &names, &children) {
                    run.plane = Some(plane);
                    run.plane_meshes = children
                        .get(plane)
                        .map(|kids| kids.iter().filter(|kid| meshes.get(*kid).is_ok()).collect())
                        .unwrap_or_default();
                }
            }
        }
        match &mut run.stage {
            Stage::Waiting => {
                if !(run.elapsed < run.transport) || !on_harvest_site {
                    info!(
                        "[harvest-airplane] box seq {} WaitUpdatePaperComeTime left: elapsed {:.3} s of {} (on a harvest site {on_harvest_site})",
                        run.seq, run.elapsed, run.transport
                    );
                    run.stage = Stage::WaitSite;
                } else if site.site_id == run.site_id {
                    run.elapsed += dt;
                }
            }
            Stage::WaitSite => {
                if !on_harvest_site {
                    continue;
                }
                let Some(player) = player else {
                    continue;
                };
                // The player's site-local position (the product frame
                // reflects x), truncated toward zero.
                let (x, z) = (-player.x as i32, player.z as i32);
                let Some((boxes, map)) = mock.spawn_treasure_box(run.site_id, run.seq, x, z) else {
                    warn!(
                        "[harvest-airplane] box seq {} SyncTresureBoxSpawned(site {}, ({x}, {z})): the spawn mock did not serve it; the airplane ends",
                        run.seq, run.site_id
                    );
                    run.stage = Stage::Done;
                    continue;
                };
                user.boxes = boxes;
                user.maps.insert(run.site_id, map);
                let Some(row) = user
                    .boxes
                    .iter()
                    .find(|row| {
                        row.status == BOX_SPAWNED
                            && row.site_id == run.site_id
                            && row.seq == run.seq
                    })
                    .cloned()
                else {
                    run.stage = Stage::Done;
                    continue;
                };
                run.box_position = (row.position_x, row.position_z);
                // Raycast straight down from 50 m above the box position.
                let (px, pz) = (-(row.position_x as f32), row.position_z as f32);
                let hit = verts
                    .as_deref()
                    .map(|verts| super::surface_y(verts, px, pz, f32::NEG_INFINITY))
                    .filter(|y| y.is_finite() && *y <= RAY_HEIGHT);
                let Some(y) = hit else {
                    error!(
                        "[harvest-airplane] box seq {} raycast from ({px:.2}, {RAY_HEIGHT}, {pz:.2}) hit nothing: no box is created",
                        run.seq
                    );
                    run.stage = Stage::Done;
                    continue;
                };
                run.end = Vec3::new(px, y, pz);
                // AppearAirplane.
                let Some(clip) = clip.as_ref() else {
                    warn!(
                        "[harvest-airplane] box seq {}: {CLIP} not read; the airplane cannot fly",
                        run.seq
                    );
                    run.stage = Stage::Done;
                    continue;
                };
                super::damage::push_se(&mut se, "se_ui_notice_treasurebox", "harvest-airplane");
                let draw = rng.get_or_insert(super::Rng(0)).next_f32() * 360.0;
                if let Some(root) = run.root {
                    if let Ok(mut transform) = transforms.get_mut(root) {
                        transform.translation = run.end;
                        transform.rotation = Quat::from_rotation_y(-draw.to_radians());
                    }
                    if let Ok(mut shown) = visibility.get_mut(root) {
                        *shown = Visibility::Inherited;
                    }
                }
                info!(
                    "[harvest-airplane] box seq {} spawned by the mock at the player's site position ({x}, {z}); landing point ({:.3}, {:.3}, {:.3}); AppearAirplane: NotifyPaperAirplaneFlying (se_ui_notice_treasurebox; the notice icon and text are the UI lane's), view active, yaw draw {draw:.2} deg, {CLIP} ({:.4} s) starts",
                    run.seq, run.end.x, run.end.y, run.end.z, clip.length
                );
                run.stage = Stage::Flying {
                    time: 0.0,
                    started: time.elapsed_secs_f64(),
                    renderer: !clip.renderer_on(0.0),
                    switches: Vec::new(),
                };
            }
            Stage::Flying {
                time: clip_time,
                started,
                renderer,
                switches,
            } => {
                let Some(clip) = clip.as_ref() else {
                    continue;
                };
                // The first flying frame shows time 0; each later frame adds
                // its time.
                if !switches.is_empty() {
                    *clip_time += dt;
                }
                let t = clip_time.min(clip.length);
                let (translation, rotation) = clip.plane_pose(t);
                if let Some(plane) = run.plane {
                    if let Ok(mut transform) = transforms.get_mut(plane) {
                        transform.translation = translation;
                        transform.rotation = rotation;
                    }
                }
                let on = clip.renderer_on(t);
                if on != *renderer {
                    *renderer = on;
                    for mesh in &run.plane_meshes {
                        if let Ok(mut shown) = visibility.get_mut(*mesh) {
                            *shown = if on {
                                Visibility::Inherited
                            } else {
                                Visibility::Hidden
                            };
                        }
                    }
                    info!(
                        "[harvest-airplane] box seq {} t {t:.4}: AirPlane renderer {} ({} mesh entities)",
                        run.seq,
                        if on { "on" } else { "off" },
                        run.plane_meshes.len()
                    );
                }
                let states = clip.switch_states(t);
                let first = switches.is_empty();
                for (index, state) in states.iter().enumerate() {
                    if first || switches[index] != *state {
                        let (node, attribute, _) = &clip.switches[index];
                        if !first || *state {
                            info!(
                                "[harvest-airplane] box seq {} t {t:.4}: {node} {attribute} {} (particles not drawn)",
                                run.seq,
                                if *state { "on" } else { "off" }
                            );
                        }
                    }
                }
                *switches = states;
                // Sampled pose readback for the run log (every 0.5 s of clip).
                let read = run
                    .plane
                    .and_then(|plane| transforms.get(plane).ok())
                    .map(|transform| (transform.translation, transform.rotation));
                if let Some((position, turn)) = read {
                    let step = (t / 0.5).floor();
                    let previous = ((t - dt).max(0.0) / 0.5).floor();
                    if t == 0.0 || step != previous || t >= clip.length {
                        let (ey, ex, ez) = turn.to_euler(EulerRot::YXZ);
                        info!(
                            "[harvest-airplane] box seq {} t {t:.4}: AirPlane local ({:.4}, {:.4}, {:.4}) euler YXZ ({:.2}, {:.2}, {:.2}) deg",
                            run.seq,
                            position.x,
                            position.y,
                            position.z,
                            ey.to_degrees(),
                            ex.to_degrees(),
                            ez.to_degrees()
                        );
                    }
                }
                if *clip_time >= clip.length {
                    // onTransformation = CreateTreasureBox.
                    let (bx, bz) = run.box_position;
                    let map = user.maps.get(&run.site_id);
                    let fixture = map.and_then(|map| {
                        map.fixtures
                            .iter()
                            .find(|row| row.position_x == bx && row.position_z == bz)
                            .cloned()
                    });
                    match fixture {
                        Some(fixture) if catalog.fixtures.contains_key(&fixture.fixture_id) => {
                            let drops: Vec<_> = map
                                .map(|map| {
                                    map.drops
                                        .iter()
                                        .filter(|drop| drop.position_x == bx && drop.position_z == bz)
                                        .cloned()
                                        .collect()
                                })
                                .unwrap_or_default();
                            let fixture_id = fixture.fixture_id;
                            let appended =
                                arrival.append_transported(run.site_id, fixture, drops, run.end);
                            if appended {
                                // The new box joins the scene-ready latch and
                                // the harvest material swap again.
                                commands.remove_resource::<super::HarvestScenesReady>();
                                commands
                                    .remove_resource::<crate::harvest_material::HarvestMaterialsSwapped>();
                            }
                            super::damage::push_se(&mut se, "se_spawn_tresure_transform", "harvest-airplane");
                            info!(
                                "[harvest-airplane] box seq {} {CLIP} done at {:.3} s (clip time {:.4}): CreateTreasureBox fixture {fixture_id} at map ({bx}, {bz}) -> the ordinary placement {}, SetPosition({:.3}, {:.3}, {:.3}), se_spawn_tresure_transform; Delay(1.0 s) then the view inactive",
                                run.seq,
                                time.elapsed_secs_f64() - *started,
                                clip_time,
                                if appended { "queued" } else { "refused (no arrival run on this site)" },
                                run.end.x,
                                run.end.y,
                                run.end.z
                            );
                        }
                        _ => warn!(
                            "[harvest-airplane] box seq {}: the map has no known fixture row at ({bx}, {bz}); no box is created",
                            run.seq
                        ),
                    }
                    run.stage = Stage::Hiding {
                        delay: Delay::new(HIDE_DELAY, frame),
                    };
                }
            }
            Stage::Hiding { delay } => {
                if delay.tick(frame, dt) {
                    if let Some(root) = run.root {
                        if let Ok(mut shown) = visibility.get_mut(root) {
                            *shown = Visibility::Hidden;
                        }
                    }
                    info!(
                        "[harvest-airplane] box seq {}: the paper airplane view inactive",
                        run.seq
                    );
                    run.stage = Stage::Done;
                }
            }
            Stage::Done => {}
        }
    }
}

fn find_named(
    root: Entity,
    name: &str,
    names: &Query<(Entity, &Name)>,
    children: &Query<&Children>,
) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if names
            .get(entity)
            .is_ok_and(|(_, found)| found.as_str() == name)
        {
            return Some(entity);
        }
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
    }
    None
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// The segment format as read (value `((a s + b) s + c) s + d` from the
    /// segment start; the first value before the first segment; the last
    /// segment's value from its start on), on hand-made keys.
    #[test]
    fn airplane_curve_segments() {
        let curve = Curve(vec![
            (0.5, [0.0, 0.0, 0.0, 1.0]),
            (1.0, [1.0, -2.0, 0.5, 2.0]),
            (2.0, [0.0, 0.0, 0.0, 1.5]),
        ]);
        assert_eq!(curve.eval(0.0), 1.0);
        assert_eq!(curve.eval(0.75), 1.0);
        // s = 0.5: 0.125 - 0.5 + 0.25 + 2.0.
        assert_eq!(curve.eval(1.5), 1.875);
        // s = 1.0 would give 1.5: the segment meets the next one.
        assert_eq!(curve.continuity(), 0.0);
        assert_eq!(curve.eval(2.0), 1.5);
        assert_eq!(curve.eval(9.0), 1.5);
    }

    /// The product-frame pose: x reflected, the y and z angles negated.
    #[test]
    fn airplane_pose_frame() {
        let constant = |v: f32| Curve(vec![(0.0, [0.0, 0.0, 0.0, v])]);
        let clip = AirplaneClip {
            length: 1.0,
            translation: [constant(0.25), constant(0.5), constant(-1.5)],
            euler: [constant(0.0), constant(90.0), constant(0.0)],
            renderer: constant(1.0),
            switches: Vec::new(),
        };
        let (position, rotation) = clip.plane_pose(0.5);
        assert_eq!(position, Vec3::new(-0.25, 0.5, -1.5));
        // Source yaw +90 maps +Z to +X; in the product frame to -X.
        let forward = rotation * Vec3::Z;
        assert!((forward - Vec3::NEG_X).length() < 1e-6, "{forward}");
    }
}
