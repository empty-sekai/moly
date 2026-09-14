//! ClientConfig 可下发面板：服务端下发的四张类型字典（键为整型 id）。
//!
//! 真源是一族静态 getter：每个 getter 的方法体都是同一形状的壳——
//! 「按整数字面量 id 查一张类型字典」，查表失败打日志并回 0。四张
//! 字典按类型分表（FloatConfigs / IntConfigs / StringConfigs /
//! BoolConfigs），同 id 跨表是两个键。面板产物（`client-config.json`，
//! 从主表 clientConfigs 提取）就是这四张字典的定型面：值的唯一来源。
//!
//! 消费侧的取值一律走本模块的键常量（常量名 = 真源 getter 名）。
//! 缺席即响亮拒绝：文件装载失败在解析点 panic；面板在场但缺某键，
//! 在取值点 panic 并具名表与键。散写常量的回落路径不存在——迁移过的
//! 键，旧常量已删。

use bevy::asset::{Assets, LoadState};
use bevy::prelude::*;
use std::collections::HashMap;

use moly_assets::json::JsonAsset;

// ---------------------------------------------------------------------------
// 键常量（真源 getter 名 → 整型 id；面板字典的键）
// ---------------------------------------------------------------------------

/// 捏合缩放的距离换算比例（FieldCameraConfig.AddDistanceRatio，
/// FloatConfigs 键 65）：距离 = 距离 − 比例 × 捏合增量。
pub(crate) const KEY_FIELD_CAMERA_ADD_DISTANCE_RATIO: i32 = 65;

/// 缩放联动俯仰下限的减项（FieldCameraConfig.MoveLookAtRatio，
/// FloatConfigs 键 66）：俯仰下限 = 插值 − 比例。
pub(crate) const KEY_FIELD_CAMERA_MOVE_LOOK_AT_RATIO: i32 = 66;

/// 站点取景界的不可见边距格数（InvisibleGridCount，IntConfigs 键 69）：
/// 近界半径 = max(全径 − 格值 × 格数, 0) × 0.5。
pub(crate) const KEY_INVISIBLE_GRID_COUNT: i32 = 69;

/// 家具旁配对对话的半径，米（NPCTalkRadius，FloatConfigs 键 104）。
pub(crate) const KEY_NPC_TALK_RADIUS: i32 = 104;

/// CharacterOverlapTime / CharacterOverlapDistance（FloatConfigs，秒/米）。
pub(crate) const KEY_CHARACTER_OVERLAP_TIME: i32 = 134;
pub(crate) const KEY_CHARACTER_OVERLAP_DISTANCE: i32 = 135;

/// 配对成员数抽签权重四档（NPCLotteryTalk1..4Wight，FloatConfigs 键
/// 144–147）：按候选段成员数取档的权重抽签。
pub(crate) const KEY_NPC_LOTTERY_TALK1_WIGHT: i32 = 144;
pub(crate) const KEY_NPC_LOTTERY_TALK2_WIGHT: i32 = 145;
pub(crate) const KEY_NPC_LOTTERY_TALK3_WIGHT: i32 = 146;
pub(crate) const KEY_NPC_LOTTERY_TALK4_WIGHT: i32 = 147;

/// 目标抽签的家具对话道占比（NPCLotteryFixtureTalkPercent，FloatConfigs
/// 键 142）：`[0,100)` 抽签落在该窗内进家具道。
pub(crate) const KEY_NPC_LOTTERY_FIXTURE_TALK_PERCENT: i32 = 142;

/// 已读家具对话窗占比（NPCLotteryAlreadyReadFixtureTalkPercent，
/// FloatConfigs 键 149）：与家具对话道占比拼成已读窗的上界。
pub(crate) const KEY_NPC_LOTTERY_ALREADY_READ_FIXTURE_TALK_PERCENT: i32 = 149;

/// 无对话（NoTalk）家具行为占比（NPCLotteryNoneTalkFixtureActionPercent，
/// FloatConfigs 键 143）：家具道内的第二张抽签落在此窗内立无对话目标。
pub(crate) const KEY_NPC_LOTTERY_NONE_TALK_FIXTURE_ACTION_PERCENT: i32 = 143;

/// 未读尽时的已读家具对话占比
/// （NPCLotteryAlreadyReadFixtureTalkPercentWhenHasNotRead，FloatConfigs
/// 键 148）：家具道第三张抽签的分流界。
pub(crate) const KEY_NPC_LOTTERY_ALREADY_READ_WHEN_HAS_NOT_READ: i32 = 148;

/// 靠近家具的移动容差（CharacterFixtureMoveOffset，FloatConfigs 键
/// 153）：靠近家具目的地链的表面采样容差，米。
pub(crate) const KEY_CHARACTER_FIXTURE_MOVE_OFFSET: i32 = 153;

/// 户外游走距离档下界（格）（NPCRandomMoveRange 的 MinDistance，
/// IntConfigs 键 152）：游走环滤的下界格数。
pub(crate) const KEY_NPC_RANDOM_MOVE_MIN_DISTANCE: i32 = 152;

/// 户外游走距离档上界（格）（NPCRandomMoveRange 的 MaxDistance，
/// IntConfigs 键 154）：游走环滤的上界格数。
pub(crate) const KEY_NPC_RANDOM_MOVE_MAX_DISTANCE: i32 = 154;

/// 室内游走距离档下界（格）（InRoom 的 MinDistance，IntConfigs 键
/// 155）。
pub(crate) const KEY_NPC_RANDOM_MOVE_IN_ROOM_MIN_DISTANCE: i32 = 155;

/// 室内游走距离档上界（格）（InRoom 的 MaxDistance，IntConfigs 键
/// 156）。
pub(crate) const KEY_NPC_RANDOM_MOVE_IN_ROOM_MAX_DISTANCE: i32 = 156;

/// 掉落批逐帧 pacing 的批数阈值（HarvestDropDelayItemCount，
/// IntConfigs 键 176）：批内项数达到该值才逐帧放行，小批同帧。
pub(crate) const KEY_HARVEST_DROP_DELAY_ITEM_COUNT: i32 = 176;

/// 玩家常态步速（MysekaiNormalMoveScale，FloatConfigs 键 77）：移动态
/// 每帧 `Move(输入 × scale × dt)` 的 scale。
pub(crate) const KEY_MYSEKAI_NORMAL_MOVE_SCALE: i32 = 77;

/// 采集场步速（MysekaiHarvestMoveScale，FloatConfigs 键 78）：站点类型
/// grassland 时取代键 77 的那档。
pub(crate) const KEY_MYSEKAI_HARVEST_MOVE_SCALE: i32 = 78;

/// 冲刺速率乘数（MysekaiDashSpeedRate，FloatConfigs 键 95）：dash 态的
/// Move 乘数是 rate × scale（采集/相机档的 scale 折算在前），动画速率
/// 不乘它。
pub(crate) const KEY_MYSEKAI_DASH_SPEED_RATE: i32 = 95;

// ---------------------------------------------------------------------------
// 装载：请求 → 解析
// ---------------------------------------------------------------------------

/// 面板 JSON 的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct ClientConfigHandle(Handle<JsonAsset>);

/// Startup：请求装载面板。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ClientConfigHandle(
        server.load::<JsonAsset>(moly_assets::client_config()),
    ));
}

/// Update：面板到齐即解析。装载失败响亮 panic（资产边界的唯一拒绝点）。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<ClientConfigHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("ClientConfig 面板装载失败：{err:?}");
    }
    let Some(json) = jsons.get(&handle.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&json.0)
        .unwrap_or_else(|err| panic!("ClientConfig 面板不是合法 JSON：{err}"));
    let float = parse_table(&value, "FloatConfigs", |v, key| {
        v.as_f64()
            .map(|n| n as f32)
            .unwrap_or_else(|| panic!("ClientConfig 面板 FloatConfigs 键 {key} 的值不是数"))
    });
    let int = parse_table(&value, "IntConfigs", |v, key| {
        v.as_i64()
            .map(|n| n as i32)
            .unwrap_or_else(|| panic!("ClientConfig 面板 IntConfigs 键 {key} 的值不是整数"))
    });
    // StringConfigs / BoolConfigs 面板里在（提取侧定型），消费面还没有
    // 读者——先验形状不建访问器，键随读者一起加。
    let string = parse_table(&value, "StringConfigs", |v, key| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("ClientConfig 面板 StringConfigs 键 {key} 的值不是字符串"))
    });
    let bool = parse_table(&value, "BoolConfigs", |v, key| {
        v.as_bool()
            .unwrap_or_else(|| panic!("ClientConfig 面板 BoolConfigs 键 {key} 的值不是布尔"))
    });
    info!(
        "[client-config] 面板就绪：FloatConfigs {} 键 · IntConfigs {} 键 · StringConfigs {} 键 · BoolConfigs {} 键（服务端 ClientConfig 四张类型字典，值全部来自主表）",
        float.len(),
        int.len(),
        string.len(),
        bool.len()
    );
    commands.insert_resource(ClientConfigs { float, int });
    commands.remove_resource::<ClientConfigHandle>();
}

/// 一张字典的定型：顶层键名下的对象，键解析成整型 id，值按表型解析。
/// 字典缺席或键名不是整数都是响亮拒绝——面板是键的契约面。
fn parse_table<T>(
    value: &serde_json::Value,
    table: &str,
    parse_value: impl Fn(&serde_json::Value, i32) -> T,
) -> HashMap<i32, T> {
    let entries = value
        .get(table)
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("ClientConfig 面板缺 {table} 字典"));
    let mut map = HashMap::with_capacity(entries.len());
    for (key, cell) in entries {
        let key: i32 = key.parse().unwrap_or_else(|_| {
            panic!("ClientConfig 面板 {table} 的键 {key} 不是整型 id")
        });
        map.insert(key, parse_value(cell, key));
    }
    map
}

// ---------------------------------------------------------------------------
// 资源与取值
// ---------------------------------------------------------------------------

/// 四张类型字典（真源按表名分四张 Dictionary，同 id 跨表是两个键）。
/// 取值面只在本 crate；类型本身对外可见是因为相机域的取景/输入系统是
/// 公开函数、签名里带着它（同站点取景模型的可见性故事）。
#[derive(Resource)]
pub struct ClientConfigs {
    float: HashMap<i32, f32>,
    int: HashMap<i32, i32>,
}

impl ClientConfigs {
    /// FloatConfigs 键。缺键响亮拒绝并具名键——真源查表失败只回 0 打
    /// 日志，那是「服务端漏配」的运维事故面；本地面板缺键是面板与消费
    /// 者对不上号的构造错误，静默回 0 会把事故伪装成 0 值行为。
    pub(crate) fn float(&self, key: i32) -> f32 {
        *self
            .float
            .get(&key)
            .unwrap_or_else(|| panic!("ClientConfig 面板 FloatConfigs 缺键 {key}"))
    }

    /// IntConfigs 键，缺键响亮拒绝（同 [`Self::float`]）。
    pub(crate) fn int(&self, key: i32) -> i32 {
        *self
            .int
            .get(&key)
            .unwrap_or_else(|| panic!("ClientConfig 面板 IntConfigs 缺键 {key}"))
    }
}
