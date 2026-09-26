//! 玩家、NPC 出生、目标查询和路线共用侵蚀后的可行走场。
//! 家具阻挡读取源 PhysicsCollider 几何；本地单层投影仍不是原生烘焙瓦片。
//! 导航代数让目标面和执行路线同步失效。高度仍由地表采样承担。

use crate::fixture::FixturePlacements;
use crate::fixture_collision::{CollisionBakeStatus, CollisionInputs};
use crate::fixture_edit::LayoutSaved;
use crate::site::{NavMeshSourceRegion, SiteActive, WalkFaceMeshes};
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use moly_law::carve::{self, BakeCounts, WalkField};
use std::sync::Arc;

/// 站点可行走面：世界系三角网的 xz 投影 + 挖洞烘焙出的可行走场。换站
/// 拆资源后由 [`build`] 按新站重建。
#[derive(Resource)]
pub struct WalkFace {
    pub(crate) field: Arc<WalkField>,
    generation: u64,
    /// The actual published layout whose footprints were baked into field.
    /// Generation alone is insufficient evidence for a particular Save.
    layout_revision: u64,
}

impl WalkFace {
    pub(crate) fn on_face(&self, x: f32, z: f32) -> bool {
        self.walkable_at([x, z])
    }

    /// 种子落座：无条件吸到最近面点（出生点与换站落位必须站在面内）。
    pub(crate) fn seat(&self, x: f32, z: f32) -> (f32, f32) {
        let q = self
            .field
            .nearest_walkable([x, z], None)
            .expect("可行走场没有出生点");
        (q[0], q[1])
    }

    /// 沿可行走场的折线（挖洞 + 半径内缩后的面）：查询链（端点吸附 +
    /// 拉回梯度）见 `moly_law::carve`。返回路点折线，全败 `None`。
    pub fn path(&self, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
        self.field.path(start, goal)
    }

    /// 点在烘焙后的可行走场上吗（足迹洞与侵蚀带都算不可走；出界同）。
    pub fn walkable_at(&self, p: [f32; 2]) -> bool {
        self.field.walkable_at(p)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn layout_revision(&self) -> u64 {
        self.layout_revision
    }

    pub fn sample(&self, p: [f32; 2], tolerance: f32) -> Option<[f32; 2]> {
        self.field.nearest_walkable(p, Some(tolerance))
    }

    pub fn path_exact(&self, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
        self.field.path_exact(start, goal)
    }

    pub fn segment_walkable(&self, start: [f32; 2], goal: [f32; 2]) -> bool {
        self.field.segment_walkable(start, goal)
    }

    pub fn constrain_move(&self, start: [f32; 2], goal: [f32; 2]) -> [f32; 2] {
        self.field.constrain_move(start, goal)
    }

    /// One agent move on the navigation cells (`WalkField::move_position`,
    /// the engine's per-frame corridor move); returns the new x/z.
    pub fn move_position(&self, start: [f32; 3], goal: [f32; 3]) -> [f32; 2] {
        self.field.move_position(start, goal)
    }

    /// Where the engine's crowd keeps an agent standing at `p`
    /// (`WalkField::relocate`): `p` on a navigation cell, else the closest
    /// point of the nearest cell in its query box, `None` when there is none.
    pub fn relocate(&self, p: [f32; 2], agent_radius: f32) -> Option<[f32; 2]> {
        self.field.relocate(p, agent_radius)
    }

    /// Local low-step elevation above the original site surface. The raw
    /// surface remains responsible for continuous terrain height.
    pub(crate) fn height_offset(&self, point: [f32; 2]) -> f32 {
        self.field.height_offset(point)
    }

    /// 烘焙账目（重烘对账行的读面）。
    pub fn carve_counts(&self) -> &BakeCounts {
        self.field.counts()
    }
}

/// Update：面网格柄齐备且站点场景展开后，把可行走面提为世界系三角网
/// （几何面）并烘可行走场（阻挡 = 已提交实例的源物理几何）。柄对
/// 不上实体或网格资产未到齐都整帧等（面宁可晚到不可残缺——残缺面
/// 会把玩家假拘在洞边）。
#[allow(clippy::type_complexity)]
pub(crate) fn build(
    mut commands: Commands,
    revision: Res<crate::fixture::FixtureLayoutRevision>,
    meshes: Res<Assets<Mesh>>,
    handles: Option<Res<WalkFaceMeshes>>,
    face: Option<Res<WalkFace>>,
    site: Option<Res<SiteActive>>,
    source: Option<Res<NavMeshSourceRegion>>,
    placements: Option<Res<FixturePlacements>>,
    collision: CollisionInputs,
    mut failure: Local<String>,
    parts: Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
) {
    if face.is_some() {
        return;
    }
    let Some(handles) = handles else {
        return;
    };
    let (Some(site), Some(source)) = (site, source) else {
        return;
    };
    let Some(placements) = placements else {
        return;
    };
    if placements.site_id() == 0 {
        return;
    }
    let Some(tris) = collect_tris(&meshes, &handles, &parts) else {
        return; // 场景未展开完，下一帧再试
    };
    let voxel = site_voxel(&site, &source);
    let sources = match collision.collect(placements.total()) {
        Ok(sources) => sources,
        Err(reason) => {
            record_failure(&mut commands, &mut failure, reason);
            return;
        }
    };
    failure.clear();
    commands.insert_resource(CollisionBakeStatus {
        ready: true,
        reason: "cell-clipped source collider spans; not native Unity bake".into(),
        colliders: sources.colliders,
        polygons: sources.polygons.len(),
    });
    let n_obstacles = sources.colliders;
    let field = WalkField::bake_colliders(&tris, &sources.polygons, voxel);
    let (min, max) =
        tris.iter()
            .flatten()
            .fold(([f32::MAX; 2], [-f32::MAX; 2]), |(mut min, mut max), p| {
                min[0] = min[0].min(p[0]);
                min[1] = min[1].min(p[2]);
                max[0] = max[0].max(p[0]);
                max[1] = max[1].max(p[2]);
                (min, max)
            });
    info!(
        "[walk-face] 约束面就绪：三角 {}，x [{:.1},{:.1}] z [{:.1},{:.1}]",
        tris.len(),
        min[0],
        max[0],
        min[1],
        max[1]
    );
    log_bake("烘好", &field, n_obstacles);
    commands.insert_resource(WalkFace {
        field: Arc::new(field),
        generation: 1,
        layout_revision: revision.0,
    });
}

/// 放稳行变化即重烘；会话撤销时同时撤销增量洞，保存事件仍驱动同一入口。
/// The same system then runs the frame's NavMeshObstacle update
/// ([`RuntimeCarving`]) on the face it did not replace this frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn rebake_on_save(
    mut commands: Commands,
    mut saved: MessageReader<LayoutSaved>,
    revision: Res<crate::fixture::FixtureLayoutRevision>,
    mut pending: Local<bool>,
    meshes: Res<Assets<Mesh>>,
    handles: Option<Res<WalkFaceMeshes>>,
    face: Option<Res<WalkFace>>,
    site: Option<Res<SiteActive>>,
    source: Option<Res<NavMeshSourceRegion>>,
    placements: Option<Res<FixturePlacements>>,
    collision: CollisionInputs,
    mut failure: Local<String>,
    parts: Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
    mut carving: RuntimeCarving,
) {
    carving.adopt(&mut commands);
    let saved_now = !saved.is_empty();
    saved.clear();
    let rebaked = rebake(
        &mut commands,
        saved_now,
        &revision,
        &mut pending,
        &meshes,
        handles.as_deref(),
        face.as_deref(),
        site.as_deref(),
        source.as_deref(),
        placements.as_deref(),
        &collision,
        &mut failure,
        &parts,
    );
    if !rebaked {
        carving.apply(&mut commands, face.as_deref());
    }
}

/// The layout rebake; true when it replaced the face this frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn rebake(
    commands: &mut Commands,
    saved_now: bool,
    revision: &crate::fixture::FixtureLayoutRevision,
    pending: &mut bool,
    meshes: &Assets<Mesh>,
    handles: Option<&WalkFaceMeshes>,
    face: Option<&WalkFace>,
    site: Option<&SiteActive>,
    source: Option<&NavMeshSourceRegion>,
    placements: Option<&FixturePlacements>,
    collision: &CollisionInputs,
    failure: &mut String,
    parts: &Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
) -> bool {
    // A save stays pending until the committed scene's collision graph is ready.
    *pending |= saved_now || face.is_some_and(|face| face.layout_revision != revision.0);
    if !*pending {
        return false;
    }
    let (Some(site), Some(source), Some(placements), Some(handles), Some(face)) =
        (site, source, placements, handles, face)
    else {
        return false; // 面不在（未烘好或换站清扫中）：台账已落，build 会带上
    };
    let Some(tris) = collect_tris(meshes, handles, parts) else {
        return false;
    };
    let voxel = site_voxel(site, source);
    let sources = match collision.collect(placements.total()) {
        Ok(sources) => sources,
        Err(reason) => {
            record_failure(commands, failure, reason);
            return false;
        }
    };
    failure.clear();
    commands.insert_resource(CollisionBakeStatus {
        ready: true,
        reason: "cell-clipped source collider spans; not native Unity bake".into(),
        colliders: sources.colliders,
        polygons: sources.polygons.len(),
    });
    let n_obstacles = sources.colliders;
    let prior = *face.carve_counts();
    let field = WalkField::bake_colliders(&tris, &sources.polygons, voxel);
    log_bake("重烘（摆放变化）", &field, n_obstacles);
    let fresh = *field.counts();
    info!(
        "[walk-face] 重烘对账：可走 {} → {} 格 · 足迹杀 {} → {} · 侵蚀杀 {} → {} · 小区杀 {} → {}",
        prior.walkable,
        fresh.walkable,
        prior.obstacle_nulled,
        fresh.obstacle_nulled,
        prior.erosion_nulled,
        fresh.erosion_nulled,
        prior.region_nulled,
        fresh.region_nulled,
    );
    commands.insert_resource(WalkFace {
        field: Arc::new(field),
        generation: face.generation.checked_add(1).expect("导航代数溢出"),
        layout_revision: revision.0,
    });
    *pending = false;
    true
}

/// QA probe state: what the last report covered, so that a report is written
/// once per walk-field generation, site and set of door locators.
#[derive(Default)]
pub(crate) struct ProbeState {
    read: bool,
    pairs: Option<Vec<([f32; 2], [f32; 2])>>,
    reported: Option<(String, u64, Vec<[i32; 2]>)>,
}

/// Parses `x,z>x,z;x,z>x,z` (empty entries skipped).
fn probe_pairs(raw: &str) -> Result<Vec<([f32; 2], [f32; 2])>, String> {
    let point = |text: &str| -> Result<[f32; 2], String> {
        let mut parts = text.split(',').map(|v| v.trim().parse::<f32>());
        match (parts.next(), parts.next(), parts.next()) {
            (Some(Ok(x)), Some(Ok(z)), None) if x.is_finite() && z.is_finite() => Ok([x, z]),
            _ => Err(format!("walk probe point {text:?} is not x,z")),
        }
    };
    raw.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (from, to) = entry
                .split_once('>')
                .ok_or_else(|| format!("walk probe entry {entry:?} is not from>to"))?;
            Ok((point(from)?, point(to)?))
        })
        .collect()
}

/// QA probe, active only while `MOLY_WALK_PROBE` is set. For every walk-field
/// generation it reports, per door locator (`loc_inside`), the static path
/// query `CanNavmeshMoveTargetPosition` uses (from each `loc_outside` and from
/// the player), and, per `from>to` pair in the variable, the agent path query
/// `GeneratePath` uses. It changes nothing.
pub(crate) fn probe(
    face: Option<Res<WalkFace>>,
    site: Option<Res<SiteActive>>,
    named: Query<(&Name, &GlobalTransform)>,
    players: Query<&GlobalTransform, With<crate::player::PlayerControlled>>,
    mut state: Local<ProbeState>,
) {
    if !state.read {
        state.read = true;
        state.pairs = std::env::var("MOLY_WALK_PROBE")
            .ok()
            .map(|raw| probe_pairs(&raw).unwrap_or_else(|error| panic!("[walk-probe] {error}")));
    }
    let Some(pairs) = state.pairs.as_ref() else {
        return;
    };
    let (Some(face), Some(site)) = (face, site) else {
        return;
    };
    let xz = |transform: &GlobalTransform| {
        let t = transform.translation();
        [t.x, t.z]
    };
    let inside: Vec<[f32; 2]> = named
        .iter()
        .filter(|(name, _)| name.as_str() == "loc_inside")
        .map(|(_, transform)| xz(transform))
        .collect();
    let key = (
        site.site_type.clone(),
        face.generation,
        inside
            .iter()
            .map(|p| {
                [
                    (p[0] * 1000.0).round() as i32,
                    (p[1] * 1000.0).round() as i32,
                ]
            })
            .collect::<Vec<_>>(),
    );
    if state.reported.as_ref() == Some(&key) {
        return;
    }
    let field = &face.field;
    let end_distance = |from: [f32; 2], to: [f32; 2]| {
        field
            .calculate_path(from, to, carve::STATIC_QUERY_HALF_EXTENT)
            .and_then(|path| path.corners.last().copied())
            .map(|last| ((last[0] - to[0]).powi(2) + (last[1] - to[1]).powi(2)).sqrt())
    };
    let mut origins: Vec<(&str, [f32; 2])> = named
        .iter()
        .filter(|(name, _)| name.as_str() == "loc_outside")
        .map(|(_, transform)| ("loc_outside", xz(transform)))
        .collect();
    origins.extend(players.iter().map(|transform| ("player", xz(transform))));
    for target in &inside {
        let off_field = field
            .nearest_walkable(*target, None)
            .map(|p| ((p[0] - target[0]).powi(2) + (p[1] - target[1]).powi(2)).sqrt());
        for (label, origin) in &origins {
            let reach = crate::site_move::door_law::HOUSE_ENTRY_REACH;
            info!(
                "[walk-probe] {} generation {}: loc_inside ({:.4}, {:.4}) walkable {} off-field {:?} from {label} ({:.4}, {:.4}): static path end distance {:?}, CanNavmeshMoveTargetPosition({reach}) {}",
                site.site_type,
                face.generation,
                target[0],
                target[1],
                field.walkable_at(*target),
                off_field,
                origin[0],
                origin[1],
                end_distance(*origin, *target),
                field.can_navmesh_move_target_position(*origin, *target, reach),
            );
        }
    }
    for (from, to) in pairs {
        let corners = field.generate_path(*from, *to);
        info!(
            "[walk-probe] {} generation {}: ({:.3}, {:.3}) -> ({:.3}, {:.3}): walkable {}, agent query maps {}, GeneratePath {} corners, static path end distance {:?}",
            site.site_type,
            face.generation,
            from[0],
            from[1],
            to[0],
            to[1],
            field.walkable_at(*from),
            field.can_calculate_path(*from, *to, carve::AGENT_QUERY_HALF_EXTENT),
            corners.len(),
            end_distance(*from, *to),
        );
        info!(
            "[walk-probe] {} generation {}: ({:.3}, {:.3}) agent-box endpoint {:?}",
            site.site_type,
            face.generation,
            from[0],
            from[1],
            field.endpoint_report(*from, carve::AGENT_QUERY_HALF_EXTENT),
        );
    }
    state.reported = Some(key);
}

/// 换站清扫：面、面源柄与已存台账一起撤（新站由 [`build`] 重建）。
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<WalkFaceMeshes>();
    commands.remove_resource::<WalkFace>();
    commands.remove_resource::<CollisionBakeStatus>();
}

/// 面三角收集：柄对应的全部网格实体齐备且资产到齐时返回世界系三角网，
/// 否则 `None`（调用方整帧等）。
#[allow(clippy::type_complexity)]
fn collect_tris(
    meshes: &Assets<Mesh>,
    handles: &WalkFaceMeshes,
    parts: &Query<(&Mesh3d, &GlobalTransform), Without<moly_assets::scene_state::SourceInactive>>,
) -> Option<Vec<[[f32; 3]; 3]>> {
    let mut by_handle = handles
        .0
        .iter()
        .map(|handle| (handle.id(), false))
        .collect::<Vec<_>>();
    let mut tris = Vec::new();
    for (mesh3d, global) in parts {
        let Some(position) = by_handle
            .iter_mut()
            .find(|(id, _)| *id == mesh3d.0.id())
            .map(|(_, seen)| seen)
        else {
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            return None; // 柄在场而资产未到，等齐再说
        };
        *position = true;
        let Some(verts) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            panic!("可行走面网格没有 float3 的 POSITION 属性");
        };
        let world = |i: usize| global.transform_point(Vec3::from(verts[i]));
        let count = verts.len();
        let index = mesh
            .indices()
            .map(|indices| indices.iter().collect::<Vec<_>>());
        let triples = match &index {
            Some(list) => (0..list.len() / 3)
                .map(|i| [list[i * 3], list[i * 3 + 1], list[i * 3 + 2]])
                .collect::<Vec<_>>(),
            None => (0..count / 3)
                .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
                .collect::<Vec<_>>(),
        };
        for [a, b, c] in triples {
            let (va, vb, vc) = (world(a), world(b), world(c));
            tris.push([va.to_array(), vb.to_array(), vc.to_array()]);
        }
    }
    if by_handle.iter().any(|(_, seen)| !*seen) || tris.is_empty() {
        return None; // 场景未展开完
    }
    Some(tris)
}

/// 站点体素（`InitializeNavMesh` 的站点分派，见 carve 律）。站点名是
/// 九值闭集，出现未知名是接线错误——响亮拒绝。
fn site_voxel(site: &SiteActive, source: &NavMeshSourceRegion) -> f32 {
    let value = carve::site_type_value(&site.site_type).unwrap_or_else(|| {
        panic!(
            "[walk-face] 未知站点类型名 {:?}：体素无法分派",
            site.site_type
        )
    });
    carve::voxel_size(source.0, value)
}

fn record_failure(commands: &mut Commands, prior: &mut String, reason: String) {
    if *prior != reason {
        warn!("[walk-face] collider navigation pending: {reason}");
        *prior = reason.clone();
    }
    commands.insert_resource(CollisionBakeStatus {
        ready: false,
        reason,
        colliders: 0,
        polygons: 0,
    });
}

/// 烘焙账目行（建面与重烘同格式）。
fn log_bake(tag: &str, field: &WalkField, obstacles: usize) {
    let counts = field.counts();
    info!(
        "[walk-face] 可行走场{tag}：voxel {:.2} · 格 {} · 入面 {} · 阻挡足迹 {obstacles} 行（杀 {} 格）· 侵蚀杀 {} · 小区杀 {} · 可走 {} 格 · 单调分区 {} 区 · 轮廓 {} 条/{} 顶点 · 导航多边形 {} 个",
        field.voxel(),
        counts.cells,
        counts.face,
        counts.obstacle_nulled,
        counts.erosion_nulled,
        counts.region_nulled,
        counts.walkable,
        counts.regions,
        counts.contours,
        counts.contour_verts,
        counts.polygons,
    );
}

// —— Runtime NavMeshObstacle carving ——

/// A node's NavMeshObstacle records (the `navMeshObstacles` extras the
/// extractor writes on runtime-spawned objects, e.g. harvest objects), each
/// with its `NavMeshObstacle::UpdateState` state.
#[derive(Component, Clone, Debug)]
pub(crate) struct RuntimeObstacles(Vec<(ObstacleRecord, carve::obstacle::ObstacleState)>);

impl RuntimeObstacles {
    /// `NavMeshObstacle.enabled` for every obstacle on this node (e.g. a view
    /// that turns a serialized-off obstacle on). Takes effect on the next
    /// obstacle update; a re-enabled obstacle starts from a fresh snapshot.
    #[allow(dead_code)] // for the object views that toggle their obstacles
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        for (record, state) in &mut self.0 {
            if record.enabled != enabled {
                record.enabled = enabled;
                *state = carve::obstacle::ObstacleState::default();
            }
        }
    }
}

/// One serialized NavMeshObstacle, centre in the moly frame (x reflected),
/// extents as authored (a capsule's x is its radius, y its half height).
#[derive(Clone, Debug)]
struct ObstacleRecord {
    kind: carve::obstacle::CarveKind,
    center: Vec3,
    extents: [f32; 3],
    enabled: bool,
    carve: bool,
    only_stationary: bool,
    move_threshold: f32,
    time_to_stationary: f32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObstacleExtras {
    nav_mesh_obstacles: Vec<ObstacleJson>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObstacleJson {
    schema_version: u32,
    coordinate_contract: String,
    enabled: serde_json::Value,
    shape: u32,
    center: Xyz,
    extents: Xyz,
    carve: bool,
    only_stationary: bool,
    move_threshold: f32,
    stationary_time: f32,
}

#[derive(serde::Deserialize)]
struct Xyz {
    x: f32,
    y: f32,
    z: f32,
}

fn obstacle_records(value: &str) -> Result<Vec<ObstacleRecord>, String> {
    let extras: ObstacleExtras =
        serde_json::from_str(value).map_err(|error| format!("navMeshObstacles extras: {error}"))?;
    extras
        .nav_mesh_obstacles
        .into_iter()
        .map(|o| {
            if o.schema_version != 2 || o.coordinate_contract != moly_assets::coordinates::CONTRACT {
                return Err(format!(
                    "navMeshObstacles schema {} / contract {} unsupported",
                    o.schema_version, o.coordinate_contract
                ));
            }
            let enabled = match &o.enabled {
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::Number(n) => n.as_u64() == Some(1),
                other => return Err(format!("NavMeshObstacle m_Enabled {other} unreadable")),
            };
            let kind = match o.shape {
                0 => carve::obstacle::CarveKind::Capsule,
                1 => carve::obstacle::CarveKind::Box,
                other => return Err(format!("NavMeshObstacle shape {other} unknown")),
            };
            Ok(ObstacleRecord {
                kind,
                center: Vec3::new(o.center.x, o.center.y, o.center.z),
                extents: [o.extents.x, o.extents.y, o.extents.z],
                enabled,
                carve: o.carve,
                only_stationary: o.only_stationary,
                move_threshold: o.move_threshold,
                time_to_stationary: o.stationary_time,
            })
        })
        .collect()
}

/// The carve set last applied, and the face generation it produced.
#[derive(Default)]
pub(crate) struct CarveApplied {
    generation: u64,
    carves: Vec<carve::RuntimeCarve>,
}

/// `NavMeshManager::UpdateNavMeshObstacles` for the runtime obstacles: each
/// frame every registered obstacle runs `UpdateState`; a carving obstacle
/// that is enabled, active and stationary contributes its carve shape, and
/// when the set changes the walk field is rebuilt from its baked state with
/// the whole set (`WalkField::carve_obstacles`), under a new navigation
/// generation. An obstacle counts as active while its node is visible in
/// the hierarchy and not source-inactive: the objects that carry these
/// records toggle their parts with visibility, standing in for
/// `SetActive`, which enables and disables the component.
///
/// Named differences: the engine applies the carve results after its carve
/// jobs finish; here they apply in the frame the set changes. The navigation
/// polygons are tested at the obstacle node's height (this field has no
/// detail height). The static fixtures' obstacles are not in this set: they
/// go through the bake (voxel null and agent-radius erosion), not this clip.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct RuntimeCarving<'w, 's> {
    fresh: Query<
        'w,
        's,
        (Entity, &'static bevy::gltf::GltfExtras, Option<&'static Name>),
        (Added<bevy::gltf::GltfExtras>, Without<RuntimeObstacles>),
    >,
    obstacles: Query<
        'w,
        's,
        (
            Entity,
            &'static mut RuntimeObstacles,
            &'static GlobalTransform,
            Option<&'static InheritedVisibility>,
            Has<moly_assets::scene_state::SourceInactive>,
        ),
    >,
    time: Res<'w, Time>,
    applied: Local<'s, CarveApplied>,
}

impl RuntimeCarving<'_, '_> {
    /// Attaches the obstacle records of newly spawned nodes.
    fn adopt(&mut self, commands: &mut Commands) {
        for (entity, extras, name) in &self.fresh {
            if !extras.value.contains("navMeshObstacles") {
                continue;
            }
            match obstacle_records(&extras.value) {
                Ok(records) if !records.is_empty() => {
                    let states = records
                        .into_iter()
                        .map(|record| (record, carve::obstacle::ObstacleState::default()))
                        .collect();
                    commands.entity(entity).insert(RuntimeObstacles(states));
                }
                Ok(_) => {}
                Err(reason) => error!(
                    "[walk-face] obstacle records of {} refused: {reason}",
                    name.map_or("<unnamed>", |name| name.as_str())
                ),
            }
        }
    }

    /// One obstacle update and, when the carve set changed or the face was
    /// replaced, the carved face.
    fn apply(&mut self, commands: &mut Commands, face: Option<&WalkFace>) {
        let Some(face) = face else {
            *self.applied = CarveApplied::default();
            return;
        };
        let dt = self.time.delta_secs();
        let mut rows: Vec<_> = self.obstacles.iter_mut().collect();
        rows.sort_by_key(|row| row.0);
        let mut carves = Vec::new();
        for (_, mut obstacles, global, visibility, inactive) in rows {
            let active = !inactive && visibility.is_none_or(|v| v.get());
            let (scale, rotation, translation) = global.to_scale_rotation_translation();
            let axes = [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z].map(|a| a.to_array());
            for (record, state) in obstacles.0.iter_mut() {
                if !active || !record.enabled || !record.carve {
                    *state = carve::obstacle::ObstacleState::default();
                    continue;
                }
                let center = global.transform_point(record.center);
                let shape = carve::obstacle::CarveShape::from_moly(
                    record.kind,
                    center.to_array(),
                    axes,
                    scale.to_array(),
                    record.extents,
                );
                let pose = carve::obstacle::ObstaclePose {
                    position: [-translation.x, translation.y, translation.z],
                    rotation: [rotation.x, -rotation.y, -rotation.z, rotation.w],
                    lossy_scale: scale.to_array(),
                    world_extents: shape.extents,
                };
                let carving = state.update(
                    &pose,
                    dt,
                    record.only_stationary,
                    record.move_threshold,
                    record.time_to_stationary,
                );
                if carving {
                    if let Some(carve) = carve::runtime_carve(&shape, translation.y) {
                        carves.push(carve);
                    }
                }
            }
        }
        let replaced = face.generation != self.applied.generation;
        if !replaced && carves == self.applied.carves {
            return;
        }
        if replaced && carves.is_empty() {
            *self.applied = CarveApplied {
                generation: face.generation,
                carves,
            };
            return;
        }
        let field = face.field.carve_obstacles(&carves);
        let counts = *field.counts();
        let generation = face.generation.checked_add(1).expect("导航代数溢出");
        info!(
            "[walk-face] runtime carve: {} obstacle shapes, walk cells carved {}, walkable {}, cells {} (generation {generation})",
            carves.len(),
            counts.carved,
            counts.walkable,
            counts.polygons,
        );
        commands.insert_resource(WalkFace {
            field: Arc::new(field),
            generation,
            layout_revision: face.layout_revision,
        });
        *self.applied = CarveApplied { generation, carves };
    }
}
