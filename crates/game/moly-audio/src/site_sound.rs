//! Site sound sources: MysekaiSoundObject instances in the site json sidecar's
//! `components` table -> a [`crate::audio::SoundObject`] component on the
//! expanded scene entity.
//!
//! A sound source is not a furniture prefab concern: in the game content it is
//! a first-class entry on the site scene side, with exactly one in total over
//! all sites (the waterfall at the grassland site). The sidecars of the other
//! sites have no such component key — an absent key is an empty table, not a
//! defect (the same optional-key rule as inactiveNodes). `node` is the
//! slash-separated **full-chain path**, relative to the site root and without
//! the root itself, in the same coordinate system as inactiveNodes; `fields`
//! carries the sound name and the max distance (the serialized fields). The
//! package name is not in the sidecar: one-shot and resident SE all live in
//! the shared SE package (a constant), which this module fills in when it
//! attaches the component.
//!
//! Timing: attached once after the scene is expanded and the sidecar is
//! parsed (a resident latch + a done marker; a site change tears it down and
//! starts over). The attach point is after the inactiveNodes sweep — path
//! lookup happens on the swept tree, and a miss is classified against the
//! inactiveNodes list before it is reported: a path on the list is a
//! legitimate sweep (disabled/hidden objects do not wake, do not register),
//! and only a miss outside the list is a loud data error.
//!
//! A site change does not tear down the Set A state (previous name / volume /
//! sounding entity): the sound source object manager is not part of any scene
//! content (created in code when the world is entered, resident across
//! sites), so after a site change the old sound continues, and updating again
//! on return to the site is the original behaviour, reproduced here.

use crate::markers::{SiteActive, SiteScenesReady, SiteRoot};
use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

/// 站点 json 侧车的装载请求（声源表自己的那份；AssetServer 按路径去重，
/// 与 inactiveNodes 名单、材质换装共用同一份文件）。解析成功后即撤。
#[derive(Resource)]
pub struct SiteSoundJsonHandle(Handle<JsonAsset>);

/// 一条待挂的声源：全链节点路径 + 声音名 + 最大距离（侧车顺序）。
struct SoundSourceRow {
    node: String,
    sound_name: String,
    max_distance: f32,
}

/// 解析出的声源表。`components.MysekaiSoundObject` 键可选：站点没有它
/// 就是空表；键在而形状不对才是响亮拒绝。
#[derive(Resource)]
pub struct SoundSourceList(Vec<SoundSourceRow>);

/// 本站声源已挂上（空表也立——挂 0 是账目，不是缺席）。换站拆除。
#[derive(Resource)]
pub struct SiteSoundFed;

/// 请求装载站点 json 侧车（首次装载与每次换站都走这里；场景目录名由
/// 装载计划给出）。拆站时由 [`teardown`] 撤下在途请求。
pub fn request(commands: &mut Commands, server: &AssetServer, scene: &str) {
    let handle = server.load::<JsonAsset>(moly_assets::site_scene_json(scene));
    commands.insert_resource(SiteSoundJsonHandle(handle));
}

/// 拆站面：撤下侧车请求、已解析声源表与完成标记，让新站的声源重新
/// 起跳。A 套运行态不在这里撤——见模块注释。
pub fn teardown(commands: &mut Commands) {
    commands.remove_resource::<SiteSoundJsonHandle>();
    commands.remove_resource::<SoundSourceList>();
    commands.remove_resource::<SiteSoundFed>();
}

/// Update: parse once after the sidecar has loaded. A failure panics loudly
/// (the single refusal point at the asset boundary); an incomplete load waits
/// silently for the next frame. Disabled instances (m_Enabled=0) are filtered
/// here: disabled components do not wake and do not register, so attaching
/// them
/// would add state the game does not have.
pub fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<SiteSoundJsonHandle>>,
    active: Option<Res<SiteActive>>,
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
    let rows = match value.pointer("/components/MysekaiSoundObject") {
        None => Vec::new(),
        Some(entry) => {
            let instances = entry
                .get("instances")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("站点 {site} 的 MysekaiSoundObject.instances 不是数组"));
            let mut rows = Vec::new();
            let mut disabled = 0usize;
            for inst in instances {
                let node = inst
                    .get("node")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("站点 {site} 的 MysekaiSoundObject 实例缺 node 字符串")
                    })
                    .to_owned();
                let fields = inst
                    .get("fields")
                    .unwrap_or_else(|| panic!("站点 {site} 的声源 {node} 缺 fields"));
                let enabled = match fields.get("m_Enabled") {
                    Some(serde_json::Value::Bool(b)) => *b,
                    Some(v) => v.as_i64().map(|n| n != 0).unwrap_or_else(|| {
                        panic!("站点 {site} 的声源 {node} 的 m_Enabled 不是布尔或整数")
                    }),
                    None => panic!("站点 {site} 的声源 {node} 缺 m_Enabled"),
                };
                if !enabled {
                    disabled += 1;
                    continue;
                }
                let sound_name = fields
                    .get("soundName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("站点 {site} 的声源 {node} 缺 soundName 字符串")
                    })
                    .to_owned();
                let max_distance = fields
                    .get("maxDistance")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!("站点 {site} 的声源 {node} 的 maxDistance 不是数值")
                    }) as f32;
                if max_distance < 0.0 {
                    panic!("站点 {site} 的声源 {node} 的 maxDistance 为负：{max_distance}");
                }
                rows.push(SoundSourceRow {
                    node,
                    sound_name,
                    max_distance,
                });
            }
            if disabled > 0 {
                info!("站点 {site} 的声源：{disabled} 条禁用（不挂）");
            }
            rows
        }
    };
    commands.insert_resource(SoundSourceList(rows));
    commands.remove_resource::<SiteSoundJsonHandle>();
}

/// Update：scene 展开且侧车解析完成后，按全链路径把声源挂上命中的实例
/// （多重集逐条消费，与 inactiveNodes 同一套对号纪律），挂上即立完成
/// 标记。此前每帧空转；挂点在清扫之后，见模块注释。
pub fn apply(
    mut commands: Commands,
    list: Option<Res<SoundSourceList>>,
    active: Option<Res<SiteActive>>,
    scenes_ready: Option<Res<SiteScenesReady>>,
    fed: Option<Res<SiteSoundFed>>,
    inactive: Option<Res<crate::inactive_nodes::InactiveList>>,
    roots: Query<Entity, With<SiteRoot>>,
    names: Query<&Name>,
    children: Query<&Children>,
) {
    let (Some(list), Some(active), Some(_)) = (list, active, scenes_ready) else {
        return; // 侧车未解析完，或 scene 未展开，下一帧再试
    };
    if fed.is_some() || roots.is_empty() {
        return; // 本站已挂上，或站点实体不在（换站拆除了）
    }
    // 全链路径表：与 inactiveNodes 同一套收集（多重集消费序与 glTF
    // 子序一致；房间站的多个场景实体按树根名区分，路径不相撞）。
    let mut by_path: HashMap<String, Vec<Entity>> = HashMap::new();
    for root in &roots {
        crate::inactive_nodes::collect(root, &mut Vec::new(), &names, &children, &mut by_path);
    }
    let mut attached = 0usize;
    let mut swept = 0usize;
    let mut missed = 0usize;
    for row in &list.0 {
        let key = format!("{}/{path}", active.scene, path = row.node);
        let Some(queue) = by_path.get_mut(&key) else {
            if inactive
                .as_deref()
                .is_some_and(|list| list.contains(&row.node))
            {
                swept += 1; // path on the list: legitimate sweep, never registered
            } else {
                error!(
                    "声源 {} 的节点路径在场景树中对不上：{}",
                    row.sound_name, row.node
                );
                missed += 1;
            }
            continue;
        };
        let Some(entity) = queue.first().copied() else {
            error!(
                "声源 {} 的节点路径实例已用尽：{}",
                row.sound_name, row.node
            );
            missed += 1;
            continue;
        };
        queue.remove(0);
        commands.entity(entity).insert(crate::audio::SoundObject {
            cue: row.sound_name.clone(),
            package: crate::audio::SE_PACKAGE.to_owned(),
            max_distance: row.max_distance,
        });
        attached += 1;
        info!(
            "站点声源挂上 {}：cue {} · 最大距离 {:.1}",
            row.node, row.sound_name, row.max_distance
        );
    }
    info!(
        "站点声源：挂上 {attached}（清扫让位 {swept} · 未命中 {missed}）——侧车 {} 条",
        list.0.len()
    );
    commands.insert_resource(SiteSoundFed);
}
