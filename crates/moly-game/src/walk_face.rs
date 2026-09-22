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
        reason: "conservative single-surface collider projection; not native Unity bake".into(),
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
#[allow(clippy::type_complexity)]
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
) {
    let saved_now = !saved.is_empty();
    saved.clear();
    // A save stays pending until the committed scene's collision graph is ready.
    *pending |= saved_now
        || face
            .as_deref()
            .is_some_and(|face| face.layout_revision != revision.0);
    if !*pending {
        return;
    }
    let (Some(site), Some(source), Some(placements), Some(handles), Some(face)) =
        (&site, &source, &placements, &handles, &face)
    else {
        return; // 面不在（未烘好或换站清扫中）：台账已落，build 会带上
    };
    let Some(tris) = collect_tris(&meshes, handles, &parts) else {
        return;
    };
    let voxel = site_voxel(site, source);
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
        reason: "conservative single-surface collider projection; not native Unity bake".into(),
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
