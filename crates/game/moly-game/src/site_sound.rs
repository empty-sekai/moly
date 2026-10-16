//! 站点声源：site json 侧车 `components` 表里的 MysekaiSoundObject 实例 →
//! 展开后场景实体上的 [`crate::audio::SoundObject`] 组件。
//!
//! 声源不是家具 prefab 的事：游戏内容里它是站点场景侧的一等条目，全部
//! 站点合计恰一处（草原站瀑布），其余站点的侧车没有这个组件键——键缺席
//! 是空表，不是缺陷（与 inactiveNodes 同一条可选键纪律）。`node` 是斜杠
//! 分隔的**全链路径**，相对站点主根、不含根自身，与 inactiveNodes 同一
//! 坐标系；`fields` 带声音名与最大距离（真源序列化字段）。cue 到包名不
//! 在侧车里：一次性与常驻 SE 全部活在共享 SE 包（真源常量），挂组件时
//! 由本模块补上。
//!
//! 时机：scene 展开且侧车解析完成后一次挂上（常驻闩 + 完成标记，换站
//! 拆除重来）。挂点在 inactiveNodes 清扫之后——路径查找发生在清扫后的
//! 树上，miss 先对 inactiveNodes 名单分类再报：名单内的路径是合法清扫
//! （真源里禁用/隐藏的对象不跑唤醒、不注册），名单外的 miss 才是
//! 响亮的数据错。
//!
//! 换站不撤 A 套运行态（前后名/音量/在播实体）：真源的声源对象管理器
//! 组件不在任何场景内容里（进世界时代码态创建、跨站点常驻），换站后
//! 旧声持续、回到站点续更新是源行为，照搬。

use crate::site::{SiteActive, SiteScenesReady, SiteRoot};
use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

/// 站点 json 侧车的装载请求（声源表自己的那份；AssetServer 按路径去重，
/// 与 inactiveNodes 名单、材质换装共用同一份文件）。解析成功后即撤。
#[derive(Resource)]
pub(crate) struct SiteSoundJsonHandle(Handle<JsonAsset>);

/// 一条待挂的声源：全链节点路径 + 声音名 + 最大距离（侧车顺序）。
struct SoundSourceRow {
    node: String,
    sound_name: String,
    max_distance: f32,
}

/// 解析出的声源表。`components.MysekaiSoundObject` 键可选：站点没有它
/// 就是空表；键在而形状不对才是响亮拒绝。
#[derive(Resource)]
pub(crate) struct SoundSourceList(Vec<SoundSourceRow>);

/// 本站声源已挂上（空表也立——挂 0 是账目，不是缺席）。换站拆除。
#[derive(Resource)]
pub(crate) struct SiteSoundFed;

/// 请求装载站点 json 侧车（首次装载与每次换站都走这里；场景目录名由
/// 装载计划给出）。拆站时由 [`teardown`] 撤下在途请求。
pub(crate) fn request(commands: &mut Commands, server: &AssetServer, scene: &str) {
    let handle = server.load::<JsonAsset>(moly_assets::site_scene_json(scene));
    commands.insert_resource(SiteSoundJsonHandle(handle));
}

/// 拆站面：撤下侧车请求、已解析声源表与完成标记，让新站的声源重新
/// 起跳。A 套运行态不在这里撤——见模块注释。
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<SiteSoundJsonHandle>();
    commands.remove_resource::<SoundSourceList>();
    commands.remove_resource::<SiteSoundFed>();
}

/// Update：侧车装载完成后解析一次。失败响亮 panic（资产边界的唯一拒绝
/// 点），未到齐静默等下一帧。禁用实例（m_Enabled=0）在这里就筛掉：
/// 真源里禁用的组件不跑唤醒、不注册，挂上反而是发明。
pub(crate) fn parse(
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
pub(crate) fn apply(
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
                swept += 1; // 名单内的路径：合法清扫，真源里本就不注册
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
