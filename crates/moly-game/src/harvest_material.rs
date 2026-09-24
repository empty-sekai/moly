//! 采集物材质：props 文档的材质表按族谱换装。
//!
//! 数据面：每个摆放点名包的文档（`site/props/<leaf>/<leaf>.json`，与
//! glb 同目录）带 `materials[]`（shader 名 · keyword 集 · 浮点表 · 槽表）
//! 与 `textureColourSpace`——与站点 sidecar 同一提取产物形状，走同一个
//! [`parse_site_sidecar`]。glb 侧的材质句柄按它在 glb `materials` 里的下标
//! 与文档材质条目 join（提取产物两侧同序），并核对 `GltfMaterialName` 与
//! 条目名一致；一个包里可以有同名的两个材质（`treasure_box1` 的两个
//! `mat_base`，贴图不同），按名 join 会把第二个解析成第一个。下标对不上
//! 时退回按名 join，且只在名字在文档里唯一时成立。句柄在 scene 实例间
//! 共享：同包两条摆放只解析一次、同时换上。
//!
//! 族分派（采集物九个子类的视图族谱）：已移植三族。`Mysekai/Site/FieldObject`
//! 与 `Mysekai/Site/Tree` 复用站点材质管线
//! （`SiteMaterial` 与其 MaterialPlugin 都由站点材质插件装着），解析走
//! 同一族门（keyword 全集 + 律的标量校验），贴图从 `site/props/<leaf>`
//! 目录装载（著色空间按文档声明）。`Mysekai/TreasureBox` (the base shape
//! of the two treasure packages) resolves to the site pipeline's TreasureBox
//! family (`resolve_treasurebox`: phenomena light and shade, drop shadow,
//! rare overlay, treasure shadows, fog). Harvest objects are placed only by
//! the development preview gallery (`MOLY_HARVEST_PREVIEW=1`), so this
//! family, like the other two here, is drawn only there. 其余
//! （`Mysekai/Effect/UberUnlit` 演出族、粒子 listen point 一族）是
//! 范围外：按 shader 名计数保留。Particle system materials are not glb
//! materials; the stay particles are drawn by `harvest_particles.rs`.
//!
//! 换装时机：全部摆放的 scene 展开完毕（`HarvestScenesReady`）后一次
//! 性建 plan、等贴图到齐、逐实体换。挂账：采集物贴图不补 mip 链
//! （与站点族同一裁决——demo 侧的补链在家具域，是另一条独立链路）。

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::gltf::{Gltf, GltfMaterialName};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use moly_assets::sidecar::{parse_site_sidecar, SiteSidecar};
use moly_law::shading::fieldobject;

use crate::harvest::{HarvestDocs, HarvestGltfs, HarvestObject, HarvestRoot, HarvestScenesReady};
use crate::site_material::{
    load_dir_texture, resolve_fieldobject, resolve_treasurebox, resolve_tree, SiteFamily,
    SiteMaterial, TREASUREBOX_SHADER_NAME,
};

/// 换装完成标记。
#[derive(Resource)]
pub struct HarvestMaterialsSwapped;

/// 一条换装计划：解析好的采集物材质与其来源包（日志对账用）。
struct Planned {
    name: String,
    leaf: String,
    material: SiteMaterial,
}

/// glb 材质句柄的归类：换装的指向 plan 下标，其余保留原材质。
#[derive(Clone, Copy)]
enum GltfClass {
    Swap(usize),
    Retain,
}

/// 一次换装的全部状态。
struct SwapPlan {
    planned: Vec<Planned>,
    classes: HashMap<Handle<StandardMaterial>, GltfClass>,
    tally: SwapTally,
}

/// 计数们：换装完成时一次性 `info!`/`warn!`，是「真的换上了吗」的
/// 现算证据。
#[derive(Default)]
struct SwapTally {
    fieldobject_materials: usize,
    fieldobject_entities: usize,
    tree_materials: usize,
    tree_entities: usize,
    treasurebox_materials: usize,
    treasurebox_entities: usize,
    /// Handles joined by name because their glb index did not name the same
    /// document entry (or the glb had no index for them).
    name_joined: usize,
    /// 其他 shader（演出/粒子族）：按 shader 名计数保留。
    retained_by_shader: HashMap<String, usize>,
    retained_entities: usize,
    /// glb 有名而文档材质表无名。
    glb_only: Vec<String>,
    /// 族门/律校验的具名拒绝。
    refused: Vec<String>,
    /// 文档侧槽 URI 不在顶层 textures[] 的条数（装载层指标，全表累计）。
    unmatched_texture_slots: usize,
}

/// Update：全部采集物 scene 展开后（`HarvestScenesReady`，harvest 模块的
/// 闩）建 plan、等贴图到齐、一次性换装。此前每帧空转。
fn switch_materials(
    mut commands: Commands,
    ready: Option<Res<HarvestScenesReady>>,
    swapped: Option<Res<HarvestMaterialsSwapped>>,
    server: Res<AssetServer>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    docs: Option<Res<HarvestDocs>>,
    gltfs: Res<Assets<Gltf>>,
    glbs: Option<Res<HarvestGltfs>>,
    roots: Query<(Entity, &HarvestObject), With<HarvestRoot>>,
    children: Query<&Children>,
    parts: Query<(&MeshMaterial3d<StandardMaterial>, &GltfMaterialName)>,
    mut materials: ResMut<Assets<SiteMaterial>>,
    mut plan: Local<Option<SwapPlan>>,
) {
    if swapped.is_some() {
        return;
    }
    if ready.is_none() {
        return;
    }
    let (Some(docs), Some(glbs)) = (docs, glbs) else {
        return;
    };
    // 文档全部到位（计划阶段已请求摆放点名包）。装载失败具名 panic——
    // 点名包的文档不在盘上是数据断点。
    for (package, handle) in &docs.0 {
        match server.load_state(handle) {
            LoadState::Failed(err) => panic!("采集物文档装载失败（{package}）：{err:?}"),
            LoadState::Loaded => {}
            _ => return,
        }
    }
    let Some(mut state) = plan
        .take()
        .or_else(|| build_swap_plan(&server, &json, &docs, &gltfs, &glbs, &roots, &children, &parts))
    else {
        return;
    };

    // 等贴图到齐：换装早于贴图到位会让实体闪回默认材质。装载失败具名 panic。
    let mut all_loaded = true;
    for item in &state.planned {
        let textures = std::iter::once(&item.material.main_tex)
            .chain(item.material.overlay_tex.iter())
            .chain(item.material.overlay2nd_tex.iter())
            .chain(item.material.leaf_mask_tex.iter());
        for texture in textures {
            match server.load_state(texture) {
                LoadState::Failed(err) => {
                    panic!("采集物材质 {} 的贴图装载失败：{err:?}", item.name)
                }
                LoadState::Loaded => {}
                _ => all_loaded = false,
            }
        }
    }
    if !all_loaded {
        *plan = Some(state);
        return;
    }

    // 换装执行：走一遍摆放层级按句柄分组实体，逐条换。scene 展开后的
    // 实体集合在 ready 之后不再变。
    let mut entities_by_handle: HashMap<Handle<StandardMaterial>, Vec<Entity>> = HashMap::new();
    for (root, _) in roots.iter() {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            let Ok((material, _)) = parts.get(entity) else {
                continue;
            };
            entities_by_handle
                .entry(material.0.clone())
                .or_default()
                .push(entity);
        }
    }
    for (handle, entities) in &entities_by_handle {
        match state.classes.get(handle).copied() {
            Some(GltfClass::Swap(index)) => {
                let item = &state.planned[index];
                let site_handle = materials.add(item.material.clone());
                for entity in entities {
                    commands
                        .entity(*entity)
                        .remove::<MeshMaterial3d<StandardMaterial>>()
                        .insert(MeshMaterial3d(site_handle.clone()));
                }
                match item.material.key.family {
                    SiteFamily::FieldObject => {
                        state.tally.fieldobject_materials += 1;
                        state.tally.fieldobject_entities += entities.len();
                    }
                    SiteFamily::Tree => {
                        state.tally.tree_materials += 1;
                        state.tally.tree_entities += entities.len();
                    }
                    SiteFamily::TreasureBox { .. } => {
                        state.tally.treasurebox_materials += 1;
                        state.tally.treasurebox_entities += entities.len();
                    }
                    // 换装计划只由 resolve_fieldobject/resolve_tree/
                    // resolve_treasurebox 产出，其余族在采集物族谱里不存在——
                    // 出现即数据面错位。
                    family => panic!(
                        "采集物换装出现了不可能的族 {family:?}（材质 {}）",
                        item.name
                    ),
                }
            }
            _ => {
                state.tally.retained_entities += entities.len();
            }
        }
    }

    info!(
        "采集物材质换装：FieldObject {} 材质 {} 实体，Tree {} 材质 {} 实体，TreasureBox {} 材质 {} 实体；其他 shader 保留 {} 材质 {} 实体（按 shader：{:?}）；按名 join {} 个句柄（其余按 glb 材质下标）",
        state.tally.fieldobject_materials,
        state.tally.fieldobject_entities,
        state.tally.tree_materials,
        state.tally.tree_entities,
        state.tally.treasurebox_materials,
        state.tally.treasurebox_entities,
        state.tally.retained_by_shader.values().sum::<usize>(),
        state.tally.retained_entities,
        state.tally.retained_by_shader,
        state.tally.name_joined,
    );
    // 逐材质采样行（验收对账：族与变体从日志可推导——树动画变体是
    // 针叶树与绣球的分界）。
    for item in &state.planned {
        info!(
            "采集物材质采样 {}（{}）：族 {:?} 树动画 {}",
            item.name,
            item.leaf,
            item.material.key.family,
            item.material.key.tree_animation,
        );
    }
    if !state.tally.refused.is_empty() {
        warn!(
            "采集物具名拒绝 {} 条：{:?}",
            state.tally.refused.len(),
            state.tally.refused
        );
    }
    if !state.tally.glb_only.is_empty() {
        warn!(
            "glb 有名而文档材质表无 {} 条：{:?}",
            state.tally.glb_only.len(),
            state.tally.glb_only
        );
    }
    info!(
        "采集物文档对账：槽 URI 不在 textures[] 的 {} 条",
        state.tally.unmatched_texture_slots,
    );
    commands.insert_resource(HarvestMaterialsSwapped);
}

/// 建 plan：走全部摆放的层级按句柄收首见（句柄在同包摆放间共享，一次
/// 解析全体换上），逐包解析文档材质表，按 glb 材质下标 join（名字核对；
/// 下标对不上时按唯一名字 join），按 shader 名分派族。
#[allow(clippy::too_many_arguments)]
fn build_swap_plan(
    server: &AssetServer,
    json: &Assets<moly_assets::json::JsonAsset>,
    docs: &HarvestDocs,
    gltfs: &Assets<Gltf>,
    glbs: &HarvestGltfs,
    roots: &Query<(Entity, &HarvestObject), With<HarvestRoot>>,
    children: &Query<&Children>,
    parts: &Query<(&MeshMaterial3d<StandardMaterial>, &GltfMaterialName)>,
) -> Option<SwapPlan> {
    // 首见句柄 → (材质名, 包键, 包短名)。同句柄的实体共享同一次解析。
    let mut seen: HashMap<Handle<StandardMaterial>, (String, String, String)> = HashMap::new();
    for (root, object) in roots.iter() {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            let Ok((material, name)) = parts.get(entity) else {
                continue;
            };
            seen.entry(material.0.clone()).or_insert_with(|| {
                (
                    name.0.clone(),
                    object.package.to_string(),
                    object.leaf.clone(),
                )
            });
        }
    }
    // 逐包解析文档（同包多摆放只解析一次）。解析失败具名 panic——文档
    // 是提取产物，形状不对是数据断点。
    let mut sidecars: HashMap<String, SiteSidecar> = HashMap::new();
    for package in seen.values().map(|(_, package, _)| package) {
        if sidecars.contains_key(package) {
            continue;
        }
        let Some(handle) = docs.0.get(package) else {
            panic!("换装计划缺文档句柄：{package}");
        };
        let Some(doc) = json.get(handle) else {
            return None; // 上层已确认 Loaded；保险回 None 下帧再试
        };
        let sidecar = parse_site_sidecar(doc.0.as_bytes()).unwrap_or_else(|err| {
            panic!("采集物文档不是合法 sidecar（{package}）：{err:?}")
        });
        sidecars.insert(package.clone(), sidecar);
    }

    let mut planned = Vec::new();
    let mut classes = HashMap::new();
    let mut tally = SwapTally::default();
    for (handle, (name, package, leaf)) in &seen {
        let sidecar = &sidecars[package];
        tally.unmatched_texture_slots += sidecar.unmatched.len();
        // The glb material index of this handle names the document entry at
        // the same index (the export writes both lists in one order).
        let glb_index = glbs
            .by_key
            .get(package)
            .and_then(|glb| gltfs.get(glb))
            .and_then(|gltf| gltf.materials.iter().position(|candidate| candidate == handle));
        let by_index = glb_index
            .and_then(|index| sidecar.materials.get(index))
            .filter(|source| source.name == *name);
        let slot = match by_index {
            Some(source) => source,
            None => {
                let mut same_name = sidecar.materials.iter().filter(|source| source.name == *name);
                match (same_name.next(), same_name.next()) {
                    (Some(source), None) => {
                        tally.name_joined += 1;
                        source
                    }
                    (None, _) => {
                        tally.glb_only.push(format!("{name}（{leaf}）"));
                        classes.insert(handle.clone(), GltfClass::Retain);
                        continue;
                    }
                    (Some(_), Some(_)) => {
                        tally.refused.push(format!(
                            "{leaf}: material name {name} is not unique in the document and glb index {glb_index:?} does not name it"
                        ));
                        classes.insert(handle.clone(), GltfClass::Retain);
                        continue;
                    }
                }
            }
        };
        let dir = format!("site/props/{leaf}");
        let load = |uri: &str| load_dir_texture(server, sidecar, &dir, uri);
        let resolved = match slot.shader.as_str() {
            fieldobject::SHADER_NAME => resolve_fieldobject(sidecar, slot, load),
            moly_law::shading::tree::SHADER_NAME => resolve_tree(sidecar, slot, load),
            TREASUREBOX_SHADER_NAME => resolve_treasurebox(sidecar, slot, load),
            // 其他（UberUnlit 演出族等）：范围外，按 shader 名计数保留。
            other => {
                *tally.retained_by_shader.entry(other.to_string()).or_default() += 1;
                classes.insert(handle.clone(), GltfClass::Retain);
                continue;
            }
        };
        match resolved {
            Ok(material) => {
                planned.push(Planned {
                    name: name.clone(),
                    leaf: leaf.clone(),
                    material,
                });
                classes.insert(handle.clone(), GltfClass::Swap(planned.len() - 1));
            }
            Err(reason) => {
                tally.refused.push(reason);
                classes.insert(handle.clone(), GltfClass::Retain);
            }
        }
    }
    Some(SwapPlan {
        planned,
        classes,
        tally,
    })
}

/// 采集物材质插件。材质管线（`MaterialPlugin<SiteMaterial>`）与全局量桥
/// 由站点材质插件装着，这里只挂换装系统。
pub struct HarvestMaterialPlugin;

impl Plugin for HarvestMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, switch_materials);
    }
}
