//! 家具挂点档案：装载、解析，与「实例摆放变换 × 挂点 local」的世界位组合。
//!
//! 挂点侧条目（`loc_start`/`loc_end` 对）带着各自 prefab 下的 local
//! 位置/朝向。真源在实例化后读它们的 Transform——读数即
//! 「实例摆放变换 × 挂点 local」。本模块照这条链组合：每个摆放实例
//! （位置 + 绕 Y 朝向）× 该包每条挂点条目 → 世界位与世界朝向。
//! 实例 API 使用完整 TRS；挂点本身的缩放不改变挂点原点。
//!
//! `AttachWorlds` 是旧消费者的一次性摆放快照，不能供移动/编辑后的实例使用。
//! 活动会话读取实际实体的 `GlobalTransform`，按源数组身份解析 Start/End。
//!
//! 两张读出面（`AttachWorlds`）：决策侧的动作点支按（包, id）取件；
//! 执行层的身份判定过滤器按位置扫描全表——扫描面是**全部已摆放
//! 实例**的挂点（不止对话锚定件），与真源「决策写的点是不是动作点
//! 表某条的坐标」同一问法。
//!
//! 解析对档案的形状 fail-closed：缺键、缺分量、分量不是数都具名
//! panic——静默丢一条挂点会把「走环带」伪装成数据事实。

use bevy::asset::{Assets, LoadState};
use bevy::prelude::*;
use std::collections::HashMap;

use moly_assets::json::JsonAsset;

use crate::fixture::FixturePlacements;

// ---------------------------------------------------------------------------
// 装载：请求 → 解析
// ---------------------------------------------------------------------------

/// 挂点档案的装载请求；解析成功后即撤（句柄资源持有期间装载不取消）。
#[derive(Resource)]
pub(crate) struct AttachPointsHandle(Handle<JsonAsset>);

/// Startup：请求装载挂点档案。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(AttachPointsHandle(
        server.load::<JsonAsset>(moly_assets::fixture_attach_points()),
    ));
}

/// 一条挂点侧条目（`loc_start` 侧）的 local 三元组。`id_value` 是
/// StartLoc 名字里的三位数字去零垫（运行时的选择键——决策侧按它取件）。
struct AttachLocal {
    id_value: i32,
    position: [f32; 3],
    rotation: Quat,
    end: Option<AttachPose>,
    source_index: Option<usize>,
    source_action_point: Option<(i32, i32)>,
    source_name_known: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct AttachPose {
    pub position: [f32; 3],
    pub rotation: Quat,
}

pub(crate) struct AttachPair {
    pub start: AttachPose,
    pub end: Option<AttachPose>,
}

/// 解析完的挂点档案：包名 → 该包的挂点条目。999 包全在键上，其中不带
/// 挂点对的包是空表（正常态，不是缺件）。
#[derive(Resource)]
pub struct AttachPoints {
    packages: HashMap<String, Vec<AttachLocal>>,
    views: HashMap<String, Vec<AttachViewIdentity>>,
}

pub(crate) struct AttachViewIdentity {
    pub file: String,
    pub game_object: i64,
    pub transform: i64,
}

impl AttachPoints {
    /// 档案文本 → 条目表。逐条具名校验（见模块注释的 fail-closed 面）。
    fn parse(text: &str) -> AttachPoints {
        let value: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|err| panic!("家具挂点档案不是合法 JSON：{err}"));
        let packages = value
            .get("packages")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("家具挂点档案缺 packages 对象"));
        let mut map = HashMap::with_capacity(packages.len());
        let mut views = HashMap::new();
        for (name, cell) in packages {
            if let Some(source_views) = cell.get("views").and_then(serde_json::Value::as_array) {
                let mut parsed = Vec::new();
                for view in source_views {
                    let game_object = view
                        .get("gameObject")
                        .expect("source FixtureView GameObject");
                    let transform = view.get("transform").expect("source FixtureView Transform");
                    let file = game_object
                        .get("file")
                        .and_then(serde_json::Value::as_str)
                        .expect("source FixtureView file");
                    assert_eq!(
                        transform.get("file").and_then(serde_json::Value::as_str),
                        Some(file),
                        "source FixtureView object/transform files differ"
                    );
                    let id = |value: &serde_json::Value| {
                        value
                            .get("pathId")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|id| id.parse::<i64>().ok())
                            .expect("source FixtureView id")
                    };
                    parsed.push(AttachViewIdentity {
                        file: file.into(),
                        game_object: id(game_object),
                        transform: id(transform),
                    });
                }
                views.insert(name.clone(), parsed);
            }
            let entries = cell
                .get("entries")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("家具挂点档案的包 {name} 缺 entries 数组"));
            let mut locals = Vec::with_capacity(entries.len());
            for entry in entries {
                let id = entry
                    .get("idValue")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| panic!("家具挂点档案的包 {name} 有条目缺 idValue"))
                    as i32;
                let transform = entry
                    .get("start")
                    .and_then(|v| v.get("transform"))
                    .unwrap_or_else(|| panic!("家具挂点档案的包 {name} 条目 {id} 缺 start 变换"));
                let position = read_vec3(transform.get("position"))
                    .unwrap_or_else(|| panic!("家具挂点档案的包 {name} 条目 {id} 的位置不是三数"));
                let rotation = read_quat(transform.get("rotation"))
                    .unwrap_or_else(|| panic!("家具挂点档案的包 {name} 条目 {id} 的朝向不是四数"));
                locals.push(AttachLocal {
                    id_value: id,
                    position,
                    rotation,
                    end: entry.get("end").filter(|end| !end.is_null()).map(|end| {
                        let transform = end.get("transform").expect("EndLoc transform");
                        AttachPose {
                            position: read_vec3(transform.get("position"))
                                .expect("EndLoc position"),
                            rotation: read_quat(transform.get("rotation"))
                                .expect("EndLoc rotation"),
                        }
                    }),
                    source_index: entry
                        .pointer("/source/entryIndex")
                        .and_then(serde_json::Value::as_u64)
                        .map(|index| index as usize),
                    source_action_point: entry
                        .pointer("/start/name")
                        .and_then(serde_json::Value::as_str)
                        .and_then(parse_action_point),
                    source_name_known: entry
                        .pointer("/start/name")
                        .and_then(serde_json::Value::as_str)
                        .is_some(),
                });
            }
            map.insert(name.clone(), locals);
        }
        AttachPoints {
            packages: map,
            views,
        }
    }

    pub(crate) fn instance_view(&self, package: &str) -> Option<&AttachViewIdentity> {
        match self.views.get(package)?.as_slice() {
            [view] => Some(view),
            _ => None,
        }
    }

    /// Resolve both ends against the actual placed instance. Two instances of
    /// one model do not share a world-space anchor or an activity reservation.
    pub(crate) fn instance_poses(
        &self,
        package: &str,
        id: i32,
        world: &GlobalTransform,
    ) -> Option<AttachPair> {
        let entry = self.instance_entry(package, id)?;
        let project = |pose: AttachPose| {
            // Compose before decomposing: reflection and local rotation do
            // not commute. The resulting actor uses an X-reflected GLB.
            let composed = world.mul_transform(
                Transform::from_translation(Vec3::from(pose.position)).with_rotation(pose.rotation),
            );
            let (_, rotation, translation) = composed.to_scale_rotation_translation();
            AttachPose {
                position: translation.to_array(),
                rotation,
            }
        };
        Some(AttachPair {
            start: project(AttachPose {
                position: entry.position,
                rotation: entry.rotation,
            }),
            end: entry.end.map(project),
        })
    }

    /// The serialized FixtureView array index, not the numeric locator code.
    /// Legacy data or ambiguous multi-view entries cannot supply this identity.
    pub(crate) fn instance_index(&self, package: &str, id: i32) -> Option<usize> {
        self.instance_entry(package, id)?.source_index
    }

    /// The source walks FixtureView's serialized array and generates a player
    /// locator only when its StartLoc name parses. A known invalid name is not
    /// missing metadata and must not disable another valid seat in that array.
    pub(crate) fn player_action_points(&self, package: &str) -> Option<Vec<i32>> {
        let entries = self.packages.get(package)?;
        if entries
            .iter()
            .any(|entry| entry.source_index.is_none() || !entry.source_name_known)
        {
            return None;
        }
        let mut ordered: Vec<_> = entries.iter().collect();
        ordered.sort_by_key(|entry| entry.source_index);
        Some(
            ordered
                .into_iter()
                .filter_map(|entry| entry.source_action_point.map(|(point, _)| point))
                .collect(),
        )
    }

    /// The parsed source suffix slot, not the point code or the array index.
    /// The source utility's TryParse failure for a present suffix yields zero,
    /// while an invalid action-point prefix rejects the locator.
    pub(crate) fn instance_slot(&self, package: &str, id: i32) -> Option<i32> {
        let entry = self.instance_entry(package, id)?;
        entry.source_index?;
        entry.source_action_point.map(|(_, slot)| slot)
    }

    fn instance_entry(&self, package: &str, id: i32) -> Option<&AttachLocal> {
        let mut matches = self.packages.get(package)?.iter().filter(|entry| {
            entry
                .source_action_point
                .is_some_and(|(point, _)| point == id)
        });
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }
}

/// Translate the source utility's Replace / Split / Int32.TryParse sequence.
/// Do not repair an authored StartLoc pointing at loc_end (ice3): the source
/// rejects that name even though the archival pair still carries an idValue.
fn parse_action_point(name: &str) -> Option<(i32, i32)> {
    let stripped = name.replace("loc_start", "");
    let mut parts = stripped.split('_');
    let point = parts.next()?.trim().parse::<i32>().ok()?;
    if point == -1 {
        return None;
    }
    let slot = parts
        .next()
        .map(|part| part.trim().parse::<i32>().unwrap_or(0))
        .unwrap_or(0);
    Some((point, slot))
}

/// 三元组读数（位置）。None = 缺键或不是数，调用方具名拒绝。
fn read_vec3(cell: Option<&serde_json::Value>) -> Option<[f32; 3]> {
    let items = cell?.as_array()?;
    if items.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (index, item) in items.iter().enumerate() {
        out[index] = item.as_f64()? as f32;
    }
    Some(out)
}

/// 四元数读数（朝向，xyzw 序）。None = 缺键或不是数。
fn read_quat(cell: Option<&serde_json::Value>) -> Option<Quat> {
    let items = cell?.as_array()?;
    if items.len() != 4 {
        return None;
    }
    let mut out = [0.0; 4];
    for (index, item) in items.iter().enumerate() {
        out[index] = item.as_f64()? as f32;
    }
    Some(Quat::from_xyzw(out[0], out[1], out[2], out[3]))
}

// ---------------------------------------------------------------------------
// 组合：摆放变换 × 挂点 local
// ---------------------------------------------------------------------------

/// 一个摆放实例的一条挂点条目的世界位。位置 = 摆放位 + 绕 Y 朝向 ×
/// local 位置；朝向 = 绕 Y 朝向 × local 朝向（两侧的因子都是纯绕 Y
/// 旋转——摆放表朝向绕 Y、档案侧朝向实测全为恒等或纯绕 Y——所以
/// 组合朝向恒纯绕 Y，贴合支的朝向律可以按欧拉 y 直读）。
pub struct AttachWorld {
    pub uid: String,
    /// 摆放包名（组合表的键的一半）。
    pub package: String,
    /// 挂点 id（StartLoc 名字数字）。
    pub id_value: i32,
    pub position: [f32; 3],
    pub rotation: Quat,
}

/// 组合完的动作点世界位全表。两张读出面见 [`AttachWorlds::entry`] 与
/// [`AttachWorlds::matching`]。
#[derive(Resource)]
pub struct AttachWorlds {
    worlds: Vec<AttachWorld>,
}

impl AttachWorlds {
    /// 某摆放实例某 id 的挂点世界位（决策侧动作点支的取件面）。
    pub(crate) fn entry(&self, uid: &str, id_value: i32) -> Option<&AttachWorld> {
        self.worlds
            .iter()
            .find(|world| world.uid == uid && world.id_value == id_value)
    }

    /// 身份判定过滤器：目标位与全表挂点比 x/z 两维（引擎 Approximately
    /// 的逐分量式，见 [`approximately`]）。命中即「决策写的点就是某条
    /// 挂点的坐标」——纯位置比对，不看目标从哪条链来：动作点目标按
    /// 构造命中自己的挂点，环带落点撞上挂点坐标的同样命中。
    pub(crate) fn matching(&self, position: [f32; 3]) -> Option<&AttachWorld> {
        self.worlds.iter().find(|world| {
            approximately(world.position[0], position[0])
                && approximately(world.position[2], position[2])
        })
    }
}

/// 组合：全部摆放行 × 各自包的挂点条目。摆放包不在档案键上即具名
/// panic（两份清单对不上一条摆放是数据断点，不是「该包没有挂点」——
/// 后者的形状是键在、表空）。
fn compose(placements: &FixturePlacements, points: &AttachPoints) -> AttachWorlds {
    let mut worlds = Vec::new();
    for placed in placements.placed_instances() {
        let (package, position, yaw) = (placed.package, placed.position, placed.yaw);
        let entries = points
            .packages
            .get(package)
            .unwrap_or_else(|| panic!("摆放包 {package} 不在挂点档案的键上（两份清单不一致）"));
        let instance = GlobalTransform::from(crate::fixture::source_transform(position, yaw));
        for entry in entries {
            let pair = points
                .instance_poses(package, entry.id_value, &instance)
                .expect("entry from this source package");
            worlds.push(AttachWorld {
                uid: placed.uid.to_owned(),
                package: package.to_owned(),
                id_value: entry.id_value,
                position: pair.start.position,
                rotation: pair.start.rotation,
            });
        }
    }
    AttachWorlds { worlds }
}

/// 引擎 Approximately 的逐分量式：`|b−a| < max(1e-6·max(|a|,|b|), 8·EPS)`。
/// EPS 在引擎侧是 float 可表示的最小非零值（flush-to-zero 关闭时
/// 1.401298e-45，开启时 1.17549435e-38）——对米级坐标，两个读法都比
/// 一次加法的舍入差小几个数量级，比较的胜负全由相对项 `1e-6·maxAbs`
/// 决定，常数选择在这里不承重（同链算出的两次结果位级相等，环带落点
/// 与挂点坐标的差是厘米级以上）。
fn approximately(a: f32, b: f32) -> bool {
    const EPSILON: f32 = 1.401_298_5e-45;
    (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(EPSILON * 8.0)
}

/// Update：档案到齐即解析；档案与摆放都在场即组合世界位（一次性）。
/// 装载失败响亮 panic（资产边界的唯一拒绝点）。组合晚于解析一帧
/// （命令在同步点落地）——决策侧的资源门挡住这一帧的空窗。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<AttachPointsHandle>>,
    points: Option<Res<AttachPoints>>,
    placements: Res<FixturePlacements>,
    worlds: Option<Res<AttachWorlds>>,
) {
    if let Some(handle) = handle {
        if let LoadState::Failed(err) = server.load_state(&handle.0) {
            panic!("家具挂点档案装载失败：{err:?}");
        }
        if let Some(json) = jsons.get(&handle.0) {
            let parsed = AttachPoints::parse(&json.0);
            let with_entries = parsed
                .packages
                .values()
                .filter(|entries| !entries.is_empty())
                .count();
            let entries: usize = parsed.packages.values().map(Vec::len).sum();
            info!(
                "[fixture-attach] 挂点档案就绪：{} 包带条目，共 {} 条（挂点 local 三元组；世界位由摆放变换组合现算）",
                with_entries, entries
            );
            commands.insert_resource(parsed);
            commands.remove_resource::<AttachPointsHandle>();
        }
    }
    if placements.site_id() != 0 && (worlds.is_none() || placements.is_changed()) {
        if let Some(points) = points {
            let composed = compose(&placements, &points);
            info!(
                "[fixture-attach] 动作点世界位组合：{} 条摆放，命中挂点 {} 点",
                placements.total(),
                composed.worlds.len()
            );
            commands.insert_resource(composed);
        }
    }
}
