//! Site readiness after importing the source state of the actual instances.
//!
//! The sidecar inactiveNodes list spans every prefab in its package. Its paths
//! are relative to each prefab, not exclusively the primary scene; path counts
//! cannot identify which instantiated object is inactive. The glTF adapter now
//! applies each node's own active/enabled metadata and retains dormant objects.
//! This module keeps the package list for legacy sound diagnostics, establishes
//! the ready epoch, and provides the shared path collector. It does not delete
//! entities by a package-wide list or interpret uninstantiated prefabs as errors.
//!
//! 时机在 [`bevy::scene::SceneSpawnerSystems::Spawn`] 之后（SpawnScene
//! 集）：`SceneInstanceReady` 在 Update 之后才触发，`SiteReady` 同帧就被
//! 取景行吃掉——在 Update 里门在 `SiteReady` 上的清扫永远轮空。这里在
//! 展开完成的当帧立 [`SiteSettled`]；源休眠标记和显隐已经随 Scene 克隆。
//! Camera/navigation explicitly ignore source-inactive geometry, without
//! confusing it with an active navigation surface intentionally hidden in colour.

use crate::site::{GroundEpoch, SiteReady, SiteRoot};
use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

/// 站点 json 侧车的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct SiteSceneJsonHandle(Handle<JsonAsset>);

/// 解析出的名单，按真源顺序。`inactiveNodes` 键是可选的：站点没有它
/// 就是空表，缺席不是缺陷；键在而形状不对才是响亮拒绝。
#[derive(Resource)]
pub(crate) struct InactiveList(Vec<String>);

impl InactiveList {
    /// 名单里是否有这条路径（只问在不在，不问次数：声源的 miss 分类
    /// 用——同路径全数清扫后，剩下的实例归清扫一侧）。
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.0.iter().any(|p| p == path)
    }
}

/// 站点内容定案：实际 Scene 的源节点状态已导入。
/// 取景与站点锚计数吃它，不吃 [`SiteReady`]。
#[derive(Resource)]
pub struct SiteSettled;

/// 请求装载站点 json 侧车（首次装载与每次换站都走这里；场景目录名由
/// 装载计划给出）。拆站时由 [`teardown`] 撤下在途请求。
pub(crate) fn request(commands: &mut Commands, server: &AssetServer, scene: &str) {
    let handle = server.load::<JsonAsset>(moly_assets::site_scene_json(scene));
    commands.insert_resource(SiteSceneJsonHandle(handle));
}

/// 拆站面：撤下侧车请求与已解析名单，让新站的解析重新起跳。
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<SiteSceneJsonHandle>();
    commands.remove_resource::<InactiveList>();
}

/// Update：侧车装载完成后解析一次。失败响亮 panic（资产边界的唯一拒绝
/// 点），未到齐静默等下一帧。内部形参带私有资源，故 pub(crate)。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<SiteSceneJsonHandle>>,
    active: Option<Res<crate::site::SiteActive>>,
) {
    let (Some(handle), Some(active)) = (handle, active) else {
        return; // 未请求，或已解析并撤下；请求只出自装载计划，账目必已在
    };
    let site = active.scene.as_str();
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("站点 json 侧车装载失败（{site}）：{err:?}");
    }
    let Some(json) = jsons.get(&handle.0) else {
        return; // 还在装
    };
    let value: serde_json::Value = serde_json::from_str(&json.0)
        .unwrap_or_else(|err| panic!("站点 json 侧车不是合法 JSON（{site}）：{err}"));
    let list = match value.get("inactiveNodes") {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .unwrap_or_else(|| panic!("站点 {site} 的 inactiveNodes 不是数组"))
            .iter()
            .map(|v| {
                v.as_str()
                    .unwrap_or_else(|| panic!("站点 {site} 的 inactiveNodes 有非字符串条目"))
                    .to_owned()
            })
            .collect(),
    };
    info!(
        "[site-ready] parsed inactive sidecar for {site}: {} rows",
        list.len()
    );
    commands.insert_resource(InactiveList(list));
    commands.remove_resource::<SiteSceneJsonHandle>();
}

/// SpawnScene（引擎场景展开之后）：保留实例的源状态，然后立
/// [`SiteSettled`] 并推 [`GroundEpoch`] 代数。内部形参带私有资源，故
/// pub(crate)。
pub(crate) fn apply(
    mut commands: Commands,
    ready: Option<Res<SiteReady>>,
    list: Option<Res<InactiveList>>,
    active: Option<Res<crate::site::SiteActive>>,
    roots: Query<Entity, With<SiteRoot>>,
    epoch: Option<Res<GroundEpoch>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    inactive: Query<(), With<moly_assets::scene_state::SourceInactive>>,
) {
    let (Some(_), Some(list), Some(active)) = (ready, list, active) else {
        return; // 场景未展开，或侧车未解析完，下一帧再试
    };
    if roots.is_empty() {
        return;
    }
    let (mut retained, mut dormant, mut dormant_meshes) = (0usize, 0usize, 0usize);
    for root in &roots {
        let (_, subtree) = subtree_of(root, &children, &meshes);
        retained += subtree.len();
        for entity in subtree {
            if inactive.get(entity).is_ok() {
                dormant += 1;
                dormant_meshes += meshes.get(entity).is_ok() as usize;
            }
        }
    }
    info!(
        "站点 {} 源实例状态：保留 {retained} 实体，休眠 {dormant}（网格 {dormant_meshes}）；包级 inactiveNodes {} 条不作实例删除名单",
        active.scene, list.0.len()
    );
    // 代数跨站单调：重播种面与锚行按它起跳（换站拆资源时不撤它）。
    let next = epoch.as_deref().map_or(1, |e| e.0 + 1);
    commands.insert_resource(GroundEpoch(next));
    commands.insert_resource(SiteSettled);
    commands.remove_resource::<SiteReady>();
}

/// 深度前序收集全部有名字节点的全链路径；无名的包装实体（world_root）
/// 不进路径。声源挂接（`site_sound`）与清扫共用这一套路径坐标系。
pub(crate) fn collect(
    entity: Entity,
    prefix: &mut Vec<String>,
    names: &Query<&Name>,
    children: &Query<&Children>,
    out: &mut HashMap<String, Vec<Entity>>,
) {
    let named = names.get(entity).ok().map(|n| n.as_str().to_owned());
    if let Some(name) = &named {
        prefix.push(name.clone());
        out.entry(prefix.join("/")).or_default().push(entity);
    }
    if let Ok(kids) = children.get(entity) {
        for kid in kids {
            collect(*kid, prefix, names, children, out);
        }
    }
    if named.is_some() {
        prefix.pop();
    }
}

/// 实例子树：(网格实体数, 全部实体)，不改变实体生命周期。
fn subtree_of(
    entity: Entity,
    children: &Query<&Children>,
    meshes: &Query<(), With<Mesh3d>>,
) -> (usize, Vec<Entity>) {
    let mut all = vec![entity];
    let mut count = 0usize;
    let mut queue = vec![entity];
    while let Some(e) = queue.pop() {
        if meshes.get(e).is_ok() {
            count += 1;
        }
        if let Ok(kids) = children.get(e) {
            for kid in kids {
                all.push(*kid);
                queue.push(*kid);
            }
        }
    }
    (count, all)
}
