//! 采集物装载、摆放与交互：第五实体的收口。
//!
//! 装载链：`site/harvest.json`（提取侧的包清单，`JsonAsset` 通道）→
//! 61 包全量请求 glb（每包 3.3MB 量级，提取侧已全部导出）→ 摆放 mock
//! 逐条点名包与存档列 → 逐包按文档 `roots[]` 里唯一的 `.prefab` 根选
//! scene（FBX 与 prefab 两个 scene 同名、默认 scene 不保证是 prefab，
//! 文档根表是唯一可靠分派面）→ 摆放律（随机偏航/随机缩放、世界位 =
//! 站点原点 + 整数列、脚下贴地表采样）→ scene 展开后换装
//! （`harvest_material.rs`）。
//!
//! 摆放坐标与存档行是**服务端域**（真源里来自玩家存档下发），按范围
//! 通则做成**可下发的具名 mock**（下表）：行形状与
//! `UserMysekaiSiteHarvestFixture` 的存档列一一对应（fixtureId ·
//! positionX/Z · status · groupId），值是我方选择、逐条具名。master 表
//! 在盘上（清单行内嵌），HP 种子 / lastAttackStamina / 稀有度全部从
//! master 行 join，不 mock。`isRare` 用 master 的 rarity > 0（视图契约
//! 里的 `isRareObject` 是另一条序列化位，值全 0，不参与判定）。
//!
//! 交互律直迁自真源方法体：`UpdateHp`（多击算术：常规击中返回 damage、
//! 打空最后一点返回 prev、终结一击返回 lastAttackStamina、已收场返回 0
//! 且不改状态）与 `OnDamage` 的接口分派（多击族看 IsLastAttack 分两臂、
//! 单击族无条件整臂、终结/单击都走消失律=视觉隐藏；RemoveCollisionObject
//! 挂账——本栈没有碰撞注册表）。击打接口是 [`HarvestHits`] 队列
//! （玩家交互从这里入队；冒烟仪表是另一个写者）。
//!
//! 掉落链（HandleResourceDrop → CreateDropItem，真源方法体直迁）：击中
//! 后按 IsLastAttack 双臂滤波待掉列表（非终结臂 `击后 HP < 行.hp <= 击前
//! HP`、终结臂 `行.hp >= 击后 HP`——列表在构造时按 `行.hp <= 初始 HP`
//! 同步），批内先播一声 SE（生日 fixture 独占一声、其余按批内最大稀有
//! 度 ∈ {rarity_2, rarity_3} 取 rare 声、{rarity_1, rarity_4} 静默、越界
//! 即真源 throw），再逐项生成掉落实体（fire-and-forget；批大小 ≥ 下发
//! 阈值时逐帧 pacing 一项）。单项的落点是极坐标散布（角度 [0,2π) 弧度、
//! 半径按 ResourceType/材质类型分档），高度贴地表采样（真源 raycast
//! 向下打地面碰撞体）；散布动画 = 水平 OutCubic 0.6s + Y 轴双跳轮廓
//! （0.18 OutQuad / 0.18 InQuad / 0.12 OutQuad / 0.12 InQuad，后跳峰值
//! = d2·d1·r·0.43）；散布半径 0 的族（tone 材质）不散布不动画、radius
//! 置 1.0。掉落表本身是**服务端域**（User 前缀的存档行），按范围通则做
//! 成具名 mock（[`DROP_TABLE_MOCK`]），行形状与存档列一一对应；行的
//! master join 分别消费采集清单内嵌材料行、家具切片及完整设计图/道具/
//! 唱片主表。普通素材的生日/玻璃球分流仍缺对应材料表消费者，具名保留。
//!
//! 掉落物实体不带 [`HarvestRoot`]：材质换装走不到它们，保留 glb 的默认
//! PBR 材质（与宝箱基形族的具名拒同一裁决——扩族是材质域另一单）。
//!
//! 挂账（真源可读、本单范围外，接手时从这里的具名出发）：
//! - 受击演出（PlayMultiActionDamageEffect / PlaySingleActionDamageEffect）
//!   与稀有序列（IMultiActionObject 的稀有接口调用）——动画/粒子域；
//! - navmesh 合成与落点吸附（grasslands 没有采集 navmesh，脚下高度与
//!   掉落落点都走地表顶点采样近似——真源落点是 raycast 打地面碰撞体、
//!   水平走 navmesh 采样，本栈两维都用顶点采样替）；
//! - punch 只锚三点（世界 X 轴振幅 0.01、时长 0.7s、端点回零），DOTween
//!   的 punch 缓动曲线未逐点移植；
//! - 掉落特效的粒子播放本体（ShowRareDropEffect 的四臂选择律已移植并
//!   计数，粒子实例与 Stop/Resume 回调是动画/粒子域）；tone 类掉落视图
//!   自己的悬浮演出（玩家头像耦合定位 + 无限浮动序列）——玩家/相机域；
//! - 捡拾（掉落视图的碰撞注册与拾取交互——本栈没有碰撞注册表，掉落实
//!   体带 uid/radius 字段留给拾取域；进图已掉落行的重建臂
//!   （CreateUnclaimedDropItems，存档列 status=dropped）同属捡拾域）。

use std::collections::{HashMap, HashSet};

use bevy::asset::{LoadState, RecursiveDependencyLoadState};
use bevy::ecs::observer::On;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneRoot};

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::site::{GroundMeshes, SITE};

/// 摆放 mock：服务端域的下发清单（具名替身，见模块注释）。
///
/// 12 行：11 行在场（status=spawned）+ 1 行已收场（进图重建臂：装载时
/// 直接隐藏，不参与交互）。九个 fixtureType 全覆盖——wood 0（针叶树）·
/// mineral 1（石 ×2，其一 rarity_2 走稀有臂）· plant 2（绣球）·
/// treasure_box_transport 3（宝箱）· treasure_box_fixed 4（宝箱1——
/// 只在 master 行出现，视图的序列化 type 仍是 3，模型按视图值构造，
/// 真源同形）· other 5（垃圾山）· tone 6（音阵）· toolbox 7（工具箱）·
/// driftage 8（木桶）· birthday_plant 9（生日花）。多击族 = wood 与
/// mineral（master hp 90）；其余单击族（master hp 0）。稀有行：
/// 石 2006 · 音阵 7001 · 木桶 6001 · 生日花 8002（master rarity > 0）。
///
/// 坐标取整数世界单位（真源式：worldPos = 站点原点 + (positionX, 0,
/// positionZ)，无格值系数），落在相机取景的广场（世界 (0, ·, −8) 一带）
/// 周围、家具摆放区之外。
// The historical rocks/chests were a development fixture gallery, not a
// player's saved world. They are no longer injected into ordinary gameplay.
fn preview_placements_enabled() -> bool {
    std::env::var("MOLY_HARVEST_PREVIEW").ok().as_deref() == Some("1")
}

const PLACEMENTS: [PlacementMock; 12] = [
    // 针叶树：多击族（wood，master 1002：hp 90 · 终结体力 10），树动画
    // 材质（_USE_TREE_ANIMATION）。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_wood_common_conifer01",
        fixture_id: 1002,
        position_x: 6,
        position_z: -8,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 石：多击族基形（mineral，master 2001：hp 90 · 体力 10 · 非稀有）。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_rock_common_stone01",
        fixture_id: 2001,
        position_x: -6,
        position_z: -8,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 石：稀有行（master 2006，rarity_2 ⇒ isRare=true）——多击终结 +
    // 稀有臂的唯一覆盖。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_rock_common_stone01",
        fixture_id: 2006,
        position_x: -6,
        position_z: -6,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 绣球：单击族（plant，master 4009：hp 0 · 体力 20），树族材质基形。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_flower_common_hydrangea01",
        fixture_id: 4009,
        position_x: -4,
        position_z: -11,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 宝箱：单击族（treasure_box_transport，master 111：hp 0 · 体力
    // 20）。材质族 Mysekai/TreasureBox 不在已移植五族内，换装侧具名拒。
    PlacementMock {
        package: "mysekai__site__field__object__treasure_box",
        fixture_id: 111,
        position_x: 2,
        position_z: -12,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 宝箱1：单击族（master 112 的 type 是 treasure_box_fixed=4——九类里
    // 唯一不在视图序列化 type 里出现的一类，视图值仍是 3）。
    PlacementMock {
        package: "mysekai__site__field__object__treasure_box1",
        fixture_id: 112,
        position_x: 4,
        position_z: -12,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 垃圾山：单击族（other，master 5004：hp 0 · 体力 50）。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_lostitem_common_junkmountain01",
        fixture_id: 5004,
        position_x: 7,
        position_z: -11,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 音阵：单击族（tone，master 7001：hp 0 · 体力 100 · 稀有）。
    // 纯粒子包（prefab scene 0 个 renderer）。
    PlacementMock {
        package: "mysekai__site__field__object__tone_gust",
        fixture_id: 7001,
        position_x: -7,
        position_z: -10,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 工具箱：单击族（toolbox，master 3001：hp 0 · 体力 20）。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_tool_toolbox01",
        fixture_id: 3001,
        position_x: 6,
        position_z: -5,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 木桶：单击族（driftage，master 6001：hp 0 · 体力 50 · 稀有）。
    // prefab 根在 scene 0（FBX 在 1），文档根表分派的覆盖行之一。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_tool_barrel01",
        fixture_id: 6001,
        position_x: -2,
        position_z: -12,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 生日花：单击族（birthday_plant，master 8002：hp 0 · 体力 250 ·
    // 稀有，全表最大的终结体力）。单 scene 包。
    PlacementMock {
        package: "mysekai__site__field__object__mdl_site_dewdrop_birthday_plant103",
        fixture_id: 8002,
        position_x: 0,
        position_z: -12,
        status: STATUS_SPAWNED,
        group_id: 0,
    },
    // 已收场行：进图重建臂（status=harvested ⇒ 装载时直接隐藏 + 碰撞
    // 移除，不参与交互）——同一 master 行的第二处摆放。
    PlacementMock {
        package: "mysekai__site__field__object__treasure_box",
        fixture_id: 111,
        position_x: 5,
        position_z: -13,
        status: STATUS_HARVESTED,
        group_id: 0,
    },
];

/// 掉落表 mock：服务端域的下发清单（真源里 `User` 前缀的存档行，由
/// GetTargetBeforeDrops 按 siteId 下发；真值读不到，行值是我方选择，
/// 见模块注释）。
///
/// 14 行对 12 条摆放：掉落族的覆盖臂——双臂滤波三档（首击非终结臂 ·
/// 中段非终结臂 · 终结臂）、批 ≥ 下发阈值的逐帧 pacing、批大小 0 的
/// 立即返回臂（无行的目标 + 多击族阈值之间的每一击）、SE 三臂（生日 ·
/// rare · 静默）、散布参数四档（mask 族 · wood/mineral · tone 不散布 ·
/// 素材 else 档）、特效三臂（常 · 稀 · 极稀）、fixture 存在门、
/// GetDropItemPrefab 的确定性拒绝（tool）与 master 表不在盘的具名缺口
/// （道具族）。行位 = 摆放位（GetTargetBeforeDrops 按位筛）。
const DROP_TABLE_MOCK: [DropRowMock; 14] = [
    // 针叶树（多击 hp 90）：三档双臂。首击（90→89）非终结臂收 hp 90
    // 行（89 < 90 ≤ 90）。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 1,
        position_x: 6,
        position_z: -8,
        hp: 90,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 2,
        group_id: 0,
    },
    // 中段：第 46 击（45→44）非终结臂收 hp 45 行（44 < 45 ≤ 45）——
    // rarity_2 素材在单项批里走 rare 声与稀有特效臂。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 5,
        position_x: 6,
        position_z: -8,
        hp: 45,
        seq: 2,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 终结臂独占：hp 0 行只在第 91 击（终结，击后 hp 0）以 0 ≥ 0 收进。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 3,
        position_x: 6,
        position_z: -8,
        hp: 0,
        seq: 3,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 3,
        group_id: 0,
    },
    // 石（多击 hp 90）：首击一次收两行——批大小 2 ≥ 下发阈值 2，走逐帧
    // pacing 臂；批内最大稀有度 rarity_2 走 rare 声。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 6,
        position_x: -6,
        position_z: -8,
        hp: 90,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 11,
        position_x: -6,
        position_z: -8,
        hp: 90,
        seq: 2,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 稀有石（多击 hp 90）：rarity_3 素材——极稀特效臂；终结击时待掉已
    // 空，走批大小 0 的立即返回臂。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 12,
        position_x: -6,
        position_z: -6,
        hp: 90,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 绣球（单击）：终结臂收 hp 0 行；植物落素材 else 档散布，rarity_1
    // 批走 SE 静默臂。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 21,
        position_x: -4,
        position_z: -11,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 宝箱（单击）：fixture 族——存在门查家具主表，模型是常量
    // glassballdrop01；mask 档散布（不读素材类型），稀有度 0。
    DropRowMock {
        resource_type: RT_MYSEKAI_FIXTURE,
        resource_id: 1002,
        position_x: 2,
        position_z: -12,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 宝箱1（单击）：tool 族——GetDropItemPrefab 的确定性拒绝（LogError
    // → null，无表依赖）；行照常被批消耗，不生成实体。
    DropRowMock {
        resource_type: RT_MYSEKAI_TOOL,
        resource_id: 0,
        position_x: 4,
        position_z: -12,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 垃圾山（单击）：junk 素材 else 档散布。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 13,
        position_x: 7,
        position_z: -11,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 音阵（单击）：tone 素材——min = max = 0 的不散布臂（radius 1.0、
    // 无散布动画、无 yaw 抽值），高度档 1.0。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 24,
        position_x: -7,
        position_z: -10,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 工具箱（单击）：道具族——道具主表存在门通过后使用设计图掉落模型。
    DropRowMock {
        resource_type: RT_MYSEKAI_ITEM,
        resource_id: 1,
        position_x: 6,
        position_z: -5,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 木桶（单击）：junk rarity_2——rare 声与稀有特效臂。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 32,
        position_x: -2,
        position_z: -12,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 1,
        group_id: 0,
    },
    // 生日花（单击，fixtureType 9）：SE 生日独占臂；生日素材
    // （birthday_party）落 else 档散布。
    DropRowMock {
        resource_type: RT_MYSEKAI_MATERIAL,
        resource_id: 69,
        position_x: 0,
        position_z: -12,
        hp: 0,
        seq: 1,
        status: DROP_STATUS_BEFORE_DROP,
        quantity: 2,
        group_id: 0,
    },
    // 工具箱之外：无行的目标（批大小 0 臂的另一半）由稀有石的终结击与
    // 多击族阈值之间的每一击覆盖；已收场宝箱位（5, −13）不放行。
];

/// ResourceType 的掉落族闭集（真源枚举值；掉落链 switch 的全部分支键）。
const RT_MATERIAL: i32 = 2;
const RT_MYSEKAI_FIXTURE: i32 = 39;
const RT_MYSEKAI_BLUEPRINT: i32 = 40;
const RT_MYSEKAI_MATERIAL: i32 = 41;
const RT_MYSEKAI_ITEM: i32 = 42;
const RT_MYSEKAI_TOOL: i32 = 43;
const RT_MYSEKAI_MUSIC_RECORD: i32 = 44;

/// fixture 族掉落的常量模型（真源 GetDropItemPrefab 的 39 臂：GetMasterFixture
/// 只做存在门，模型恒取玻璃球基形，与 id 无关）。
const GLASSBALL_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_glassball_common_glassballdrop01";
const BLUEPRINT_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_blueprint_common_blueprintdrop01";
const RECORD_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_record_common_recorddrop01";

/// UserMysekaiSiteHarvestResourceDropStatus 的掉落前值（真源枚举；掉落
/// 发生后转 dropped=1，本表只放掉落前行——转场是捡拾域的服务端回执）。
const DROP_STATUS_BEFORE_DROP: i32 = 0;

/// 一条掉落行（服务端域 mock 的行形状，与存档列一一对应：resourceType
/// 枚举列（+0x18 的字符串序列化列是同一值的第二份，不重复建模）·
/// resourceId · positionX/Z · hp · seq · status · quantity · groupId）。
#[derive(Clone, Copy)]
struct DropRowMock {
    resource_type: i32,
    resource_id: i64,
    position_x: i32,
    position_z: i32,
    /// 行 HP 阈值：双臂滤波比较的就是它（真源 +0x2c）。
    hp: i32,
    /// 存档列 seq：捡拾域的排序键，本链只在日志里带着。
    seq: i32,
    /// 存档列 status（UserMysekaiSiteHarvestResourceDropStatus）。
    status: i32,
    /// 存档列 quantity：个数在捡拾域结算，本链不乘它——批大小数的是行
    /// 数，不是个数之和。
    quantity: i32,
    /// 存档列 groupId：本单不消费，列位保留。
    #[allow(dead_code)]
    group_id: i32,
}

/// UserMysekaiSiteHarvestFixtureStatus 的值域（真源枚举；进图前 0 不进
/// 本面——摆放 mock 只用在场/已收场两值）。
const STATUS_SPAWNED: i32 = 1;
/// 摆放行状态：已收场（交互面排除它；拾取域按此排除已收场行）。
pub(crate) const STATUS_HARVESTED: i32 = 2;

/// 服务端配置 HarvestObjectScaleMin / HarvestObjectScaneMax 的 mock
/// （真源键名原文拼写 ScaneMax；键值由服务端下发，本地读不到）。值是
/// 我方选择：1 附近的窄带，让缩放律在日志与画面上可辨而不至于把小
/// 物件放大到穿模。接下发面板时换成回包值。
const SCALE_MIN: f32 = 0.9;
const SCALE_MAX: f32 = 1.1;

/// 视图类的闭集（清单 `view[].class` 的值域）与接口分派键。真源
/// OnDamage 按视图实现的接口分派：IMultiActionObject 两个实现类、
/// ISingleActionObject 七个实现类（声明侧逐一核对）。
const VIEW_CLASSES: [&str; 9] = [
    "MysekaiAreaStoneView",
    "MysekaiAreaTreeView",
    "MysekaiAreaPlantView",
    "MysekaiAreaJunkView",
    "MysekaiAreaToneView",
    "MysekaiAreaToolBoxView",
    "MysekaiAreaTreasureBoxView",
    "MysekaiAreadDriftageView",
    "MysekaiBirthdayPlantView",
];

/// OnDamage 的两臂（视图类实现的接口）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionInterface {
    /// IMultiActionObject：石头与树——HP 多击，非终结击不消失。
    Multi,
    /// ISingleActionObject：其余七类——无终结条件，整臂一次走完。
    Single,
}

fn interface_of(view_class: &str) -> Option<ActionInterface> {
    match view_class {
        "MysekaiAreaStoneView" | "MysekaiAreaTreeView" => Some(ActionInterface::Multi),
        "MysekaiAreadDriftageView"
        | "MysekaiAreaJunkView"
        | "MysekaiAreaPlantView"
        | "MysekaiAreaToneView"
        | "MysekaiAreaToolBoxView"
        | "MysekaiAreaTreasureBoxView"
        | "MysekaiBirthdayPlantView" => Some(ActionInterface::Single),
        _ => None,
    }
}

/// 一条摆放（服务端域 mock 的行形状，与存档列一一对应）。
#[derive(Clone, Copy)]
struct PlacementMock {
    /// 提取清单 `packages` 的键。
    package: &'static str,
    /// 存档列 mysekaiSiteHarvestFixtureId：master 行的 join 键。
    fixture_id: i32,
    /// 存档列 positionX / positionZ：整数世界单位，直接加站点原点。
    position_x: i32,
    position_z: i32,
    /// 存档列 status（UserMysekaiSiteHarvestFixtureStatus）。
    status: i32,
    /// 存档列 groupId：本单不消费（掉落物域的分组键），列位保留。
    #[allow(dead_code)]
    group_id: i32,
}

/// plan 阶段对好的每条摆放：mock 行 × 清单视图契约 × master 行的合取。
pub(crate) struct PlacementPlan {
    pub(crate) package: &'static str,
    /// 日志用的包短名（清单 `glb` 字段的文件名段）。
    pub(crate) leaf: String,
    pub(crate) fixture_id: i32,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) status: i32,
    /// 存档列 HP 的种子 = master 行的 hp（服务端首刷时从 master 起）。
    pub(crate) hp: i32,
    /// master 行 lastAttackStamina（终结一击的 UpdateHp 返回值）。
    pub(crate) last_attack_stamina: i32,
    /// master 行 rarity > 0 ⇒ isRare（IsRareHarvestFixture 的 join 式）。
    pub(crate) is_rare: bool,
    /// 视图契约的序列化 mysekaiSiteHarvestFixtureType（模型构造点读它，
    /// 不读 master 行——真源同形）。
    pub(crate) fixture_type: i32,
    /// 视图契约的 radius（交互半径；摆放时 × 随机缩放，真源同式）。
    pub(crate) radius: f32,
    /// 视图契约的 collisionType（对账用；碰撞注册表挂账）。
    pub(crate) collision_type: i32,
    pub(crate) interface: ActionInterface,
    /// 受击/终结 SE（视图类 SE 方法体逐类对齐，见 [`se_cues`]）。
    pub(crate) se_hit: Option<&'static str>,
    pub(crate) se_final: Option<&'static str>,
}

/// 视图类的受击/终结 SE（真源视图类方法体逐类读全，cue 字面量逐一核过）：
/// 多击族三方法——PlayHitSE（每击）/ PlayLastAttackSE（终结）/
/// PlayRareObjectBreakSE（稀有终结，树与石共用 break_rare——
/// SE_TREE_BREAK_ROCK 与 SE_RARE_BREAK_ROCK 是同一 cue 字面量）；单击族
/// 一击走全臂——PlaySE（植物的稀有支取 `_rare` 变体）或
/// OnPlayerActionStart / ChangeAfterObject 段内的一声。音阵（tone）纯
/// 粒子视图，真源 0 SE ⇒ None。
fn se_cues(view_class: &str, is_rare: bool) -> (Option<&'static str>, Option<&'static str>) {
    match view_class {
        "MysekaiAreaTreeView" => (
            Some("se_axe1"),
            Some(if is_rare {
                "se_break_rare"
            } else {
                "se_fallen_tree"
            }),
        ),
        "MysekaiAreaStoneView" => (
            Some("se_pickaxe1"),
            Some(if is_rare {
                "se_break_rare"
            } else {
                "se_break_rock"
            }),
        ),
        "MysekaiAreaPlantView" => (
            Some(if is_rare {
                "se_pick_plant_rare"
            } else {
                "se_pick_plant"
            }),
            None,
        ),
        "MysekaiAreaJunkView" => (Some("se_rustle"), None),
        "MysekaiAreadDriftageView" => (Some("se_broken_barrel"), None),
        "MysekaiAreaToolBoxView" => (Some("se_tresure_open"), None),
        "MysekaiAreaTreasureBoxView" => (Some("se_spawn_tresure_open"), None),
        "MysekaiBirthdayPlantView" => (Some("se_pick_birthday_plant"), None),
        _ => (None, None),
    }
}

/// 采集物根实体标记（一条摆放一个）。
#[derive(Component)]
pub struct HarvestRoot;

/// 一条摆放的模型面与存档列（交互律读写它）。字段表对齐真源
/// HarvestObjectModel 的消费子集。字段全开放：玩家交互按
/// position/radius 选目标，本模块是被击接口的另一端。
#[derive(Component)]
pub struct HarvestObject {
    pub package: &'static str,
    pub leaf: String,
    /// 视图契约的序列化 type（构造点真源形状：读 VIEW，不读 master）。
    pub fixture_type: i32,
    pub fixture_id: i32,
    pub position_x: i32,
    pub position_z: i32,
    /// HP（真源 +0x28）。
    pub hp: i32,
    /// 上一次 UpdateHp 看到的 HP（真源 +0x40；构造时 = hp）。
    pub prev_hp: i32,
    /// UserMysekaiSiteHarvestFixtureStatus（真源 +0x20）。
    pub status: i32,
    /// 终结标记（真源 +0x24；置位后粘住——已收场复击不改它）。
    pub is_last_attack: bool,
    /// master 行的 lastAttackStamina（真源 +0x2c）。
    pub last_attack_stamina: i32,
    /// master rarity > 0（真源 +0x44 的来源）。
    pub is_rare: bool,
    /// 交互半径 = 视图 radius × 随机缩放（SetupScaleRandom 的乘式）。
    pub radius: f32,
    /// 视图契约的 collisionType（对账用；碰撞注册表挂账）。
    pub collision_type: i32,
    /// OnDamage 的分派键（视图类实现的接口，plan 阶段折进闭集）。
    pub interface: ActionInterface,
    /// 受击 SE（多击非终结击与单击族；plan 阶段按视图类折好）。
    pub se_hit: Option<&'static str>,
    /// 终结 SE（多击族终结击；稀有终结已折成 break_rare）。
    pub se_final: Option<&'static str>,
    /// 待掉列表（真源 +0x58 的 List：构造期按位同步，双臂滤波的迭代
    /// 源，生成时逐项移除——真源 RemoveTargetBeforeDropItem）。
    pub(crate) pending_drops: Vec<PendingDrop>,
    /// 本仓的击打序号（纯日志用；律的状态在 hp/prev_hp/status 里）。
    pub(crate) hits_taken: usize,
}

impl HarvestObject {
    /// UpdateHp（真源方法体逐句直迁，RVA 0x56DE7F8）：
    /// - prev = hp；prevHp = prev；
    /// - prev < 1 且 prev == 0 且 status ≠ harvested ⇒ 终结：
    ///   IsLastAttack 置位、status 转 harvested、返回 lastAttackStamina；
    /// - prev ≥ 1 且 status ≠ harvested：prev − damage > 0 ⇒ hp 扣减、
    ///   返回 damage；否则 hp 归 0、返回 prev；
    /// - 其余（已收场，或 prev < 1 的其余形态）返回 0 且不改状态。
    pub(crate) fn update_hp(&mut self, damage: i32) -> i32 {
        let prev = self.hp;
        self.prev_hp = prev;
        if prev < 1 {
            if prev == 0 && self.status != STATUS_HARVESTED {
                self.is_last_attack = true;
                self.status = STATUS_HARVESTED;
                return self.last_attack_stamina;
            }
        } else if self.status != STATUS_HARVESTED {
            if 0 < prev - damage {
                self.hp = prev - damage;
                return damage;
            }
            self.hp = 0;
            return prev;
        }
        0
    }
}

/// 单击族的受击冲量（真源 DOPunchPosition：世界 X 轴 (0.01, 0, 0)、
/// 时长 0.7s——常量取自方法体字面量）。DOTween 的 punch 缓动曲线未逐点
/// 移植：这里锚三点（峰值 ≤ 振幅、单次出-回、端点回零），包络用
/// sin(πt)·(1−t)。
#[derive(Component)]
pub struct HarvestPunch {
    elapsed: f32,
    /// 触发时的根位置（世界系）；结束时精确回它。
    base: Vec3,
}

const PUNCH_AMPLITUDE: f32 = 0.01;
const PUNCH_DURATION: f32 = 0.7;

/// 掉落行的解析面（plan 阶段把 mock 行 join 出模型与稀有度后折成它；
/// 同步进 [`HarvestObject::pending_drops`]，被击时双臂滤波、生成时逐项
/// 移除）。
#[derive(Clone)]
pub(crate) struct PendingDrop {
    resource_type: i32,
    resource_id: i64,
    position_x: i32,
    position_z: i32,
    /// 行 HP 阈值（双臂滤波比较的就是它）。
    hp: i32,
    /// 存档列 seq（日志对账用）。
    seq: i32,
    /// 存档列 status（同步门的筛选键；掉落发生后服务端转 dropped，本表
    /// 只有 before_drop 行）。
    status: i32,
    /// 存档列 quantity（日志对账用——批大小数行数，个数在捡拾域结算）。
    quantity: i32,
    /// GetDropItemPrefab 的解析结果：模型包键，或具名拒绝。
    prefab: DropPrefab,
    /// GetDropRarityType 的解析结果（SE 臂与特效臂的分支键）。
    rarity: i32,
    /// MysekaiMaterial 行的素材类型（散布三参的分支键；非素材族无意义，
    /// 填 −1）。
    material_type: i32,
    /// 行身份（mock 表下标；生成时从待掉列表按它移除——真源
    /// RemoveTargetBeforeDropItem 按行对象移除的盘上形状）。
    row: usize,
}

/// GetDropItemPrefab 的解析结果。
#[derive(Clone)]
enum DropPrefab {
    /// 模型在盘：包键（清单 `packages` 的键）。
    Model { package: String },
    /// 确定性拒绝、主表没有对应行，或尚未供给的普通材料解析：具名记账，
    /// 不用一个无关模型代替。已知物品表缺行对应 GetDropItemPrefab 的空值臂。
    Unresolved(&'static str),
}

/// GetDropRarityType（真源方法体直迁）：mask {39, 40, 42, 44} → 0；
/// 普通素材（2）→ 2；mysekai_material（41）→ 素材行稀有度；其余（43
/// 落这，真源 LogError 后记 0）→ 0。
fn drop_rarity_type(resource_type: i32, material_rarity: Option<i32>) -> i32 {
    match resource_type {
        RT_MYSEKAI_FIXTURE | RT_MYSEKAI_BLUEPRINT | RT_MYSEKAI_ITEM | RT_MYSEKAI_MUSIC_RECORD => 0,
        RT_MATERIAL => 2,
        RT_MYSEKAI_MATERIAL => material_rarity.expect("素材行必带稀有度"),
        RT_MYSEKAI_TOOL => 0,
        other => panic!("resourceType {other} 不在掉落族闭集里"),
    }
}

/// GetDropMinRange / GetDropMaxRange / GetDropHeight 三方法的合取（真源
/// 方法体直迁，返回 (min, max, height)）：mask {2, 39, 40, 42, 44} 一族
/// 公共档；41 按素材类型分档——wood/mineral（< 2）、tone（== 6）与其余
/// else 档；mask 外的其余类型（43 等）同 else 档。
fn drop_scatter_params(resource_type: i32, material_type: i32) -> (f32, f32, f32) {
    match resource_type {
        RT_MATERIAL
        | RT_MYSEKAI_FIXTURE
        | RT_MYSEKAI_BLUEPRINT
        | RT_MYSEKAI_ITEM
        | RT_MYSEKAI_MUSIC_RECORD => (0.6, 1.0, 0.0),
        RT_MYSEKAI_MATERIAL => match material_type {
            0 | 1 => (0.7, 1.2, 0.2),
            6 => (0.0, 0.0, 1.0),
            _ => (0.6, 1.0, 0.0),
        },
        _ => (0.6, 1.0, 0.0),
    }
}

/// MysekaiMaterialType 的序列化闭集（真源枚举值；清单 materialRows 全表
/// 穷举过值域，未知词是数据断点）。
fn material_type_value(word: &str) -> i32 {
    match word {
        "wood" => 0,
        "mineral" => 1,
        "plant" => 2,
        "junk" => 3,
        "game_character" => 4,
        "other" => 5,
        "tone" => 6,
        "birthday_party" => 7,
        other => panic!("素材类型不在闭集里：{other}"),
    }
}

/// MysekaiMaterialRarityType 的序列化闭集（真源枚举值）。
fn material_rarity_value(word: &str) -> i32 {
    match word {
        "rarity_1" => 0,
        "rarity_2" => 1,
        "rarity_3" => 2,
        "rarity_4" => 3,
        other => panic!("素材稀有度不在闭集里：{other}"),
    }
}

/// 一次掉落批（被击时入队；spawn_drops 逐项消耗）。真源是
/// fire-and-forget：每项一个独立异步任务，批内 delay 时逐帧出一项
/// （DelayFrame(1) 的 pacing 形状——尾项之后的 await 不影响可观察的
/// 生成时刻，不入码）。
struct DropBatch {
    /// 来源摆放实体（出生位与待掉列表的持有者）。
    origin: Entity,
    remaining: Vec<PendingDrop>,
    /// 批大小 ≥ 下发阈值 ⇒ 逐帧 pacing；否则同帧出全。
    delay: bool,
}

/// 掉落批队列。
#[derive(Resource, Default)]
pub(crate) struct HarvestDropBatches(Vec<DropBatch>);

/// 解析好的掉落行全集（plan 阶段落；同步按位筛它）。
#[derive(Resource, Default)]
struct HarvestDropRows(Vec<PendingDrop>);

/// 掉落生成序号发号器（真源 MysekaiDropItemModel 构造点的 uid 来源；
/// 拒绝行不取号——真源在模型构造前就返回）。
#[derive(Resource, Default)]
struct HarvestDropSeq(u64);

/// 地表顶点快照（摆放与掉落共用；spawn_when_ready 首次摆放时采一次，
/// 之后常驻——站点切换只移除网格资源，世界系快照对采集物仍然成立）。
#[derive(Resource, Default)]
struct HarvestGroundVerts(Option<Vec<Vec3>>);

/// 掉落实体（marker + 捡拾域要读的面；不带 [`HarvestRoot`]——材质换装
/// 族谱走不到它，保留 glb 默认 PBR 材质，见模块注释）。
#[derive(Component)]
pub struct HarvestDropItem {
    /// 真源视图 Init(uid) 的那份唯一 id（捡拾域的碰撞注册键）。
    pub uid: u64,
    /// 真源视图 radius：散布族生成时 0、不散布族（tone）置 1.0。
    pub radius: f32,
    /// GetDropRarityType 的值（ShowRareDropEffect 的分支键）。
    pub rarity: i32,
    pub resource_type: i32,
    pub resource_id: i64,
}

/// 掉落散布动画（真源 PlayHarvestDropAnimationAsync 的 DOTween Sequence
/// 结构：主位移 XZ 出 OutCubic 0.6s，Y 轴被后注册的四段补间覆盖成双跳
/// 轮廓——分段与常量见 [`advance_drop_animations`]）。
#[derive(Component)]
struct HarvestDropAnimation {
    elapsed: f32,
    /// 出生位（世界系；摆位 x、y = 0、摆位 z）。
    spawn: Vec3,
    /// 落位（散布点 XZ + 地表采样 Y）。
    landing: Vec3,
    d1: f32,
    d2: f32,
    r: f32,
}

/// 散布动画总时长（主位移 DOMove 的 0.6s）。
const DROP_DURATION: f32 = 0.6;
/// 第二跳峰值系数（d2·d1·r·0.43 的 0.43）。
const DROP_SECOND_HOP: f32 = 0.43;

fn ease_in_quad(u: f32) -> f32 {
    u * u
}

fn ease_out_quad(u: f32) -> f32 {
    1.0 - (1.0 - u) * (1.0 - u)
}

fn ease_out_cubic(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

/// 一次击打请求（被击接口的入队形状，对齐真源 OnDamage 的形参）。
#[derive(Debug, Clone, Copy)]
pub struct HarvestHit {
    /// 目标摆放实体（持 [`HarvestObject`]）。
    pub target: Entity,
    /// 伤害值：无工具时 1（真源 GatheringAction 侧的空手臂值）。
    pub damage: i32,
    /// 工具等级：无工具 0。演出侧消费（挂账），列位保留。
    pub tool_level: i32,
    /// 加成体力标记（isUseBoostStaminaOrEnhanceStamina）：演出侧消费，
    /// 列位保留。
    pub is_boost: bool,
}

/// 被击队列：本模块的 `on_damage` 每帧排空。写者必须排在它之前入队
/// （冒烟仪表 `autohit` 与玩家交互）。玩家交互从
/// `Query<(Entity, &Transform), With<HarvestRoot>>` 里按交互半径选目标。
#[derive(Resource, Default)]
pub struct HarvestHits(pub Vec<HarvestHit>);

/// 交互账本（周期状态行与收工报数的现算来源；不做验收用）。
#[derive(Resource, Default)]
pub struct HarvestStats {
    /// 入队并处理的击打总数。
    pub hits: usize,
    /// 多击族非终结击。
    pub multi_hits: usize,
    /// 多击族终结击。
    pub multi_final: usize,
    /// 单击族击打（含已收场复击）。
    pub single_hits: usize,
    /// 终结击里稀有目标的次数。
    pub rare_final: usize,
    /// 消失律触发（视觉从可见转隐藏）的次数。
    pub hidden: usize,
    /// POST 回显条数（每目标序列一条，对齐真源请求批的合键形状）。
    pub post_echoes: usize,
    /// 已收场复击的空转次数（UpdateHp 返回 0 臂）。
    pub idle_returns: usize,
    /// 掉落批入队数（count ≠ 0 臂；空批即真源的立即返回，不入账）。
    pub drop_batches: usize,
    /// 掉落实体生成数。
    pub drop_items: usize,
    /// 具名拒绝的掉落行（确定性拒绝、无主表行或尚未供给的解析）。
    pub drop_refused: usize,
    /// 不散布族生成数（tone：radius 1.0、无动画）。
    pub drop_noscatter: usize,
    /// 掉落 SE 三臂计数（生日独占 · rare · 静默）。
    pub drop_se_birthday: usize,
    pub drop_se_rare: usize,
    pub drop_se_silent: usize,
    /// ShowRareDropEffect 三臂计数（粒子本体挂账，臂律在；越界即真源
    /// LogError，本仓 panic）。
    pub drop_effect_normal: usize,
    pub drop_effect_rare: usize,
    pub drop_effect_ultra: usize,
}

/// 已请求装载的清单与掉落身份表。共用既有 JsonAsset/资产源，路径相同
/// 的请求由 AssetServer 共享，不从采集目标表推导家具或物品身份。
#[derive(Resource)]
struct HarvestIndexAsset {
    index: Handle<moly_assets::json::JsonAsset>,
    fixtures: Handle<moly_assets::json::JsonAsset>,
    blueprints: Handle<moly_assets::json::JsonAsset>,
    items: Handle<moly_assets::json::JsonAsset>,
    music_records: Handle<moly_assets::json::JsonAsset>,
}

/// 全量 glb 句柄（装载面：61 包；`order` 保序供逐包对账）。
#[derive(Resource)]
pub(crate) struct HarvestGltfs {
    pub(crate) by_key: HashMap<String, Handle<Gltf>>,
    order: Vec<String>,
}

/// 摆放点名包的文档句柄（`JsonAsset` 原文：scene 分派在 harvest、材质
/// 表在 harvest_material，同一份文件两个消费方）。
#[derive(Resource)]
pub(crate) struct HarvestDocs(pub(crate) HashMap<String, Handle<moly_assets::json::JsonAsset>>);

/// plan 阶段的合取结果（与 [`PLACEMENTS`] 同序）。
#[derive(Resource)]
pub(crate) struct HarvestPlans(pub(crate) Vec<PlacementPlan>);

/// 61 包 glb 全部到齐的闩（装载面的完成信号）。
#[derive(Resource)]
pub struct HarvestLoaded;

/// 已展开的摆放数（spawn 闩）。
#[derive(Resource, Default)]
struct HarvestSpawnedCount(usize);

/// 摆放实体表（表序；冒烟仪表按它选目标）。
#[derive(Resource, Default)]
pub(crate) struct HarvestEntities(pub(crate) Vec<Entity>);

/// 全部摆放的 scene 展开完毕（换装的门）；由 [`on_scene_ready`] 计数
/// 置位。
#[derive(Resource)]
pub struct HarvestScenesReady;

/// 已展开的 scene 计数（常驻）。
#[derive(Resource, Default)]
struct HarvestScenesReadyCount(usize);

/// Startup：请求装载清单，交接面就位。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    // Ordinary scenes deliberately have no injected harvest demonstration.
    // Do not load or validate that unrelated gallery (including model-less
    // source entries) merely to establish an empty harvest scene.
    if !preview_placements_enabled() {
        return;
    }
    commands.insert_resource(HarvestIndexAsset {
        index: server.load(bevy::asset::AssetPath::from(
            "moly://site/harvest.json".to_owned(),
        )),
        fixtures: server.load(moly_assets::mysekai_fixtures()),
        blueprints: server.load(moly_assets::mysekai_blueprints()),
        items: server.load(moly_assets::mysekai_items()),
        music_records: server.load(moly_assets::mysekai_music_records()),
    });
}

/// The keyed master deliverable keeps the complete row under its id. This
/// consumer needs existence only; it does not infer ownership or rewards.
fn keyed_master_ids(asset: &moly_assets::json::JsonAsset, table: &str) -> HashSet<i64> {
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|error| panic!("{table} master JSON: {error}"));
    assert_eq!(
        value["version"].as_u64(),
        Some(1),
        "{table}: unsupported master version"
    );
    assert_eq!(
        value["semantics"]["table"].as_str(),
        Some(table),
        "master table identity mismatch"
    );
    value["entries"]
        .as_object()
        .expect("keyed master requires entries")
        .iter()
        .map(|(key, row)| {
            let id = row["id"].as_i64().expect("master row requires integer id");
            assert_eq!(
                key.parse::<i64>().ok(),
                Some(id),
                "{table}: entry key differs from row id"
            );
            id
        })
        .collect()
}

fn constant_drop_prefab(ids: &HashSet<i64>, resource_id: i64, package: &str) -> DropPrefab {
    if ids.contains(&resource_id) {
        DropPrefab::Model {
            package: package.to_owned(),
        }
    } else {
        DropPrefab::Unresolved("掉落资源在对应主表中不存在（GetDropItemPrefab 返回空）")
    }
}

/// Update：清单到位后全量对账（每包 exported、视图契约闭集、摆放 mock
/// 逐行 join master 行），然后请求 61 个 glb 与摆放点名包的文档。
/// 对账失败具名 panic——mock 点名的包/行不在盘上是数据断点，不静默跳过。
fn plan_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    index: Option<Res<HarvestIndexAsset>>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    planned: Option<Res<HarvestGltfs>>,
) {
    if planned.is_some() {
        return;
    }
    let Some(index) = index else {
        return;
    };
    for handle in [
        &index.index,
        &index.fixtures,
        &index.blueprints,
        &index.items,
        &index.music_records,
    ] {
        match server.load_state(handle) {
            LoadState::Failed(err) => panic!("采集物清单/主表装载失败：{err:?}"),
            LoadState::Loaded => {}
            _ => return,
        }
    }
    let (Some(asset), Some(fixtures), Some(blueprints), Some(items), Some(music_records)) = (
        json.get(&index.index),
        json.get(&index.fixtures),
        json.get(&index.blueprints),
        json.get(&index.items),
        json.get(&index.music_records),
    ) else {
        return;
    };
    let fixture_value: serde_json::Value = serde_json::from_str(&fixtures.0)
        .unwrap_or_else(|error| panic!("家具主表切片 JSON: {error}"));
    let fixture_ids: HashSet<i64> = fixture_value["fixtures"]
        .as_array()
        .expect("家具主表切片缺 fixtures 数组")
        .iter()
        .map(|row| row["id"].as_i64().expect("家具主表行缺 id"))
        .collect();
    let blueprint_ids = keyed_master_ids(blueprints, "mysekaiBlueprints");
    let item_ids = keyed_master_ids(items, "mysekaiItems");
    let music_record_ids = keyed_master_ids(music_records, "mysekaiMusicRecords");
    info!(
        "掉落身份表就绪：家具 {} · 设计图 {} · 道具 {} · 唱片 {}",
        fixture_ids.len(),
        blueprint_ids.len(),
        item_ids.len(),
        music_record_ids.len()
    );
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|err| panic!("采集物清单不是合法 JSON：{err}"));
    let packages = value
        .get("packages")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("采集物清单缺 packages 对象"));

    // 全量装载的对账门：每包现算（不抄清单自带的 summary）。
    let mut by_key = HashMap::with_capacity(packages.len());
    let mut order = Vec::with_capacity(packages.len());
    let mut class_counts = [0usize; VIEW_CLASSES.len()];
    let mut unknown_classes: Vec<&str> = Vec::new();
    let mut with_view = 0usize;
    let mut companions = 0usize;
    for (key, entry) in packages {
        let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            status, "exported",
            "采集物包未导出：{key}（status = {status:?}）"
        );
        let glb = entry
            .get("glb")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("采集物清单条目缺 glb 文件名：{key}"));
        if preview_placements_enabled() {
            let handle =
                server.load::<Gltf>(bevy::asset::AssetPath::from(format!("moly://site/{glb}")));
            by_key.insert(key.clone(), handle);
            order.push(key.clone());
        }
        // 视图契约：null ⇒ 伴生包；数组恰一条 ⇒ 契约。
        let view = entry
            .get("view")
            .unwrap_or_else(|| panic!("采集物清单条目缺 view 字段：{key}"));
        if view.is_null() {
            companions += 1;
            continue;
        }
        let view = view
            .as_array()
            .unwrap_or_else(|| panic!("采集物包的 view 不是数组：{key}"));
        assert_eq!(
            view.len(),
            1,
            "采集物包的 view 不是单条契约：{key}（{} 条）",
            view.len()
        );
        with_view += 1;
        let class = view[0]
            .get("class")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("采集物视图契约缺 class：{key}"));
        match VIEW_CLASSES.iter().position(|known| *known == class) {
            Some(slot) => class_counts[slot] += 1,
            None => unknown_classes.push(class),
        }
    }
    let histogram = VIEW_CLASSES
        .iter()
        .zip(class_counts)
        .filter(|(_, count)| *count > 0)
        .map(|(class, count)| format!("{class} {count}"))
        .collect::<Vec<_>>()
        .join(" · ");
    info!(
        "采集物装载计划：{} 包全量请求（视图契约 {} · 伴生 {}），视图类分布：{}{}；摆放 mock {} 条（{} 条已收场行）",
        packages.len(),
        with_view,
        companions,
        histogram,
        if unknown_classes.is_empty() {
            String::new()
        } else {
            format!(" · 未知类 {:?}", unknown_classes)
        },
        PLACEMENTS.len(),
        PLACEMENTS
            .iter()
            .filter(|row| row.status == STATUS_HARVESTED)
            .count(),
    );

    // 摆放行合取：视图契约 + master 行（按 fixture_id join）。
    let mut plans = Vec::with_capacity(PLACEMENTS.len());
    let mut docs = HashMap::new();
    for row in PLACEMENTS
        .into_iter()
        .filter(|_| preview_placements_enabled())
    {
        let entry = packages
            .get(row.package)
            .unwrap_or_else(|| panic!("摆放 mock 点名的包不在清单里：{}", row.package));
        let view = entry
            .get("view")
            .and_then(|v| v.as_array())
            .filter(|v| v.len() == 1)
            .unwrap_or_else(|| panic!("摆放 mock 点名的包没有单条视图契约：{}", row.package));
        let contract = &view[0];
        let class = contract
            .get("class")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("视图契约缺 class：{}", row.package));
        let interface = interface_of(class)
            .unwrap_or_else(|| panic!("视图类不在接口闭集里：{class}（{}）", row.package));
        let fixture_type = contract
            .get("mysekaiSiteHarvestFixtureType")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("视图契约缺 mysekaiSiteHarvestFixtureType：{}", row.package))
            as i32;
        let radius = contract
            .get("radius")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("视图契约缺 radius：{}", row.package))
            as f32;
        let collision_type = contract
            .get("collisionType")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("视图契约缺 collisionType：{}", row.package))
            as i32;
        // 提取侧把 Unity 的 bool 序列化成 int 0/1（37 条视图契约实测全
        // int、全 0）；bool/int 两形都收，其他形状具名 panic。
        let view_is_rare = match contract.get("isRareObject") {
            None => panic!("视图契约缺 isRareObject：{}", row.package),
            Some(v) => match (v.as_bool(), v.as_i64()) {
                (Some(flag), _) => flag,
                (None, Some(0)) => false,
                (None, Some(1)) => true,
                (None, _) => {
                    panic!("视图契约 isRareObject 形状不对（{v}）：{}", row.package)
                }
            },
        };
        // master 行内嵌在清单条目里；join 键 = id（IsRareHarvestFixture 同式）。
        let master = entry
            .get("masterRows")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("清单条目缺 masterRows：{}", row.package))
            .iter()
            .find(|m| m.get("id").and_then(|v| v.as_i64()) == Some(row.fixture_id as i64))
            .unwrap_or_else(|| {
                panic!(
                    "摆放 mock 的 fixtureId {} 在 {} 的 masterRows 里找不到",
                    row.fixture_id, row.package
                )
            });
        let hp = master
            .get("hp")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("master 行缺 hp：{}#{}", row.package, row.fixture_id))
            as i32;
        let stamina = master
            .get("lastAttackStamina")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| {
                panic!(
                    "master 行缺 lastAttackStamina：{}#{}",
                    row.package, row.fixture_id
                )
            }) as i32;
        let rarity = master
            .get("mysekaiSiteHarvestFixtureRarityType")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "master 行缺 mysekaiSiteHarvestFixtureRarityType：{}#{}",
                    row.package, row.fixture_id
                )
            });
        // rarity_1 = 0（非稀有）；rarity_2/3/4 ⇒ 稀有（rarity > 0）。
        let is_rare = rarity != "rarity_1";
        let glb = entry
            .get("glb")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("清单条目缺 glb 文件名：{}", row.package));
        let leaf = glb
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".glb"))
            .unwrap_or_else(|| panic!("glb 字段不是 .glb 路径：{glb}"))
            .to_string();
        let document = entry
            .get("document")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("清单条目缺 document 文件名：{}", row.package));
        docs.entry(row.package.to_string()).or_insert_with(|| {
            server.load::<moly_assets::json::JsonAsset>(bevy::asset::AssetPath::from(format!(
                "moly://site/{document}"
            )))
        });
        if view_is_rare {
            // 视图的 isRareObject 与 master rarity 是两个来源；真源判稀有
            // 用 master。全表 37 条视图契约的该位都是 false，此处只记不判。
            info!(
                "视图契约的 isRareObject 与 master rarity 不一致：{}（视图 true · master {rarity}）",
                row.package
            );
        }
        let (se_hit, se_final) = se_cues(class, is_rare);
        plans.push(PlacementPlan {
            package: row.package,
            leaf,
            fixture_id: row.fixture_id,
            position_x: row.position_x,
            position_z: row.position_z,
            status: row.status,
            hp,
            last_attack_stamina: stamina,
            is_rare,
            fixture_type,
            radius,
            collision_type,
            interface,
            se_hit,
            se_final,
        });
    }
    // 采集材料的模型/稀有度 join 已在各包 materialRows；家具、设计图、
    // 道具和唱片的存在资格来自上方各自的主表，不混用采集目标 masterRows。
    let mut material_join: HashMap<i64, (String, i32, i32)> = HashMap::new();
    for (key, entry) in packages {
        if let Some(rows) = entry.get("materialRows").and_then(|v| v.as_array()) {
            for row in rows {
                let id = row
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| panic!("素材行缺 id：{key}"));
                let type_word = row
                    .get("mysekaiMaterialType")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("素材行缺 mysekaiMaterialType：{key}#{id}"));
                let rarity_word = row
                    .get("mysekaiMaterialRarityType")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("素材行缺 mysekaiMaterialRarityType：{key}#{id}"));
                if let Some((first, _, _)) = material_join.get(&id) {
                    panic!("素材行 id {id} 重复（{first} 与 {key}）——master 行应唯一");
                }
                material_join.insert(
                    id,
                    (
                        key.clone(),
                        material_type_value(type_word),
                        material_rarity_value(rarity_word),
                    ),
                );
            }
        }
    }
    for package in [
        GLASSBALL_DROP_PACKAGE,
        BLUEPRINT_DROP_PACKAGE,
        RECORD_DROP_PACKAGE,
    ] {
        assert!(
            packages.contains_key(package),
            "掉落常量模型包不在清单里：{package}"
        );
    }

    // 掉落行解析：GetDropItemPrefab 的分支律逐族直迁（fixture 存在门 +
    // 常量模型 / 素材 join / 普通材料待供给 / tool 确定性拒绝），
    // 稀有度同源解析（GetDropRarityType）。行位必须落在摆放位上——
    // GetTargetBeforeDrops 的 fixture 存在门（LogError + 空列表臂）在
    // mock 上以具名 warn 落形。
    let mut drop_rows: Vec<PendingDrop> = Vec::with_capacity(DROP_TABLE_MOCK.len());
    for (row_index, row) in DROP_TABLE_MOCK.iter().enumerate() {
        if !PLACEMENTS.iter().any(|placement| {
            placement.position_x == row.position_x && placement.position_z == row.position_z
        }) {
            warn!(
                "掉落行位 ({}, {}) 不在任何摆放上——GetTargetBeforeDrops 的存在门：行不进待掉列表（resourceType {} · id {}）",
                row.position_x, row.position_z, row.resource_type, row.resource_id
            );
            continue;
        }
        let (prefab, rarity) = match row.resource_type {
            RT_MYSEKAI_FIXTURE => (
                constant_drop_prefab(&fixture_ids, row.resource_id, GLASSBALL_DROP_PACKAGE),
                drop_rarity_type(row.resource_type, None),
            ),
            RT_MYSEKAI_MATERIAL => {
                let Some((package, _material_type, material_rarity)) =
                    material_join.get(&row.resource_id)
                else {
                    panic!("掉落行点名的素材 {} 不在 materialRows 里", row.resource_id);
                };
                (
                    DropPrefab::Model {
                        package: package.clone(),
                    },
                    drop_rarity_type(row.resource_type, Some(*material_rarity)),
                )
            }
            RT_MYSEKAI_TOOL => (
                DropPrefab::Unresolved("tool 族：真源确定性拒绝（LogError → null，无表依赖）"),
                drop_rarity_type(row.resource_type, None),
            ),
            RT_MYSEKAI_BLUEPRINT => (
                constant_drop_prefab(&blueprint_ids, row.resource_id, BLUEPRINT_DROP_PACKAGE),
                drop_rarity_type(row.resource_type, None),
            ),
            RT_MYSEKAI_ITEM => (
                constant_drop_prefab(&item_ids, row.resource_id, BLUEPRINT_DROP_PACKAGE),
                drop_rarity_type(row.resource_type, None),
            ),
            RT_MYSEKAI_MUSIC_RECORD => (
                constant_drop_prefab(&music_record_ids, row.resource_id, RECORD_DROP_PACKAGE),
                drop_rarity_type(row.resource_type, None),
            ),
            RT_MATERIAL => (
                DropPrefab::Unresolved("普通素材族：材料表/生日对应模型分流尚未供给本消费者"),
                drop_rarity_type(row.resource_type, None),
            ),
            other => panic!("掉落行的 resourceType {other} 不在掉落族闭集里"),
        };
        let material_type = match row.resource_type {
            RT_MYSEKAI_MATERIAL => {
                material_join
                    .get(&row.resource_id)
                    .expect("素材 join 上面已核过")
                    .1
            }
            _ => -1,
        };
        drop_rows.push(PendingDrop {
            resource_type: row.resource_type,
            resource_id: row.resource_id,
            position_x: row.position_x,
            position_z: row.position_z,
            hp: row.hp,
            seq: row.seq,
            status: row.status,
            quantity: row.quantity,
            prefab,
            rarity,
            material_type,
            row: row_index,
        });
    }
    // 解出模型的行，其包的文档一并进等待集（材质换装等全表文档，多等
    // 三个包是良性的——drop 模型包本来就在全量 glb 面里）。
    for drop in &drop_rows {
        if let DropPrefab::Model { package } = &drop.prefab {
            let entry = packages
                .get(package)
                .unwrap_or_else(|| panic!("掉落模型包不在清单里：{package}"));
            let document = entry
                .get("document")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("清单条目缺 document 文件名：{package}"));
            docs.entry(package.clone()).or_insert_with(|| {
                server.load::<moly_assets::json::JsonAsset>(bevy::asset::AssetPath::from(format!(
                    "moly://site/{document}"
                )))
            });
        }
    }
    info!(
        "掉落表 mock：{} 行（解析出模型 {} · 具名拒绝 {}），素材 join {} 行 · fixture 门 {} id",
        drop_rows.len(),
        drop_rows
            .iter()
            .filter(|drop| matches!(drop.prefab, DropPrefab::Model { .. }))
            .count(),
        drop_rows
            .iter()
            .filter(|drop| matches!(drop.prefab, DropPrefab::Unresolved(_)))
            .count(),
        material_join.len(),
        fixture_ids.len(),
    );
    commands.insert_resource(HarvestDropRows(drop_rows));
    commands.insert_resource(HarvestGltfs { by_key, order });
    commands.insert_resource(HarvestDocs(docs));
    commands.insert_resource(HarvestPlans(plans));
}

/// Update：61 包 glb 逐包对账（失败具名 panic），全部到齐落装载闩。
/// 进度行按计数变化打（装载期每帧轮询，不逐帧刷日志）。
fn check_loaded(
    mut commands: Commands,
    server: Res<AssetServer>,
    assets: Option<Res<HarvestGltfs>>,
    loaded: Option<Res<HarvestLoaded>>,
    mut last_logged: Local<usize>,
) {
    if loaded.is_some() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    let mut done = 0usize;
    for key in &assets.order {
        let Some(handle) = assets.by_key.get(key) else {
            panic!("装载计划缺 glb 句柄：{key}");
        };
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("采集物 glb 装载失败（{key}）：{err:?}");
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(handle)
        {
            panic!("采集物 glb 的依赖装载失败（{key}）：{err:?}");
        }
        if server.is_loaded_with_dependencies(handle) {
            done += 1;
        } else {
            break;
        }
    }
    if done > *last_logged && done < assets.order.len() {
        *last_logged = done;
        info!("采集物 glb 装载进度：{}/{}", done, assets.order.len());
    }
    if done == assets.order.len() {
        info!("采集物 glb 装载完成：{}/{}", done, assets.order.len());
        commands.insert_resource(HarvestLoaded);
    }
}

/// 地表顶点表（脚下采样用；与名册同式：GroundMeshes 里的网格在世界系
/// 的顶点全集）。
fn ground_world_verts(
    ground: &GroundMeshes,
    meshes: &Assets<Mesh>,
    parts: &Query<(&Mesh3d, &GlobalTransform)>,
) -> Vec<Vec3> {
    let mut verts = Vec::new();
    for (mesh3d, global) in parts {
        if !ground.0.contains(&mesh3d.0) {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            continue;
        };
        verts.extend(
            positions
                .iter()
                .map(|p| global.transform_point(Vec3::from(*p))),
        );
    }
    verts
}

/// 采样点附近的地表高度：半径内地形顶点的最高者；没有顶点时取
/// `fallback`（采集物脚下允许贴地近似——navmesh 吸附挂账）。
fn surface_y(verts: &[Vec3], x: f32, z: f32, fallback: f32) -> f32 {
    const RADIUS: f32 = 2.0;
    verts
        .iter()
        .filter(|v| {
            let dx = v.x - x;
            let dz = v.z - z;
            dx * dx + dz * dz <= RADIUS * RADIUS
        })
        .map(|v| v.y)
        .fold(fallback, f32::max)
}

/// 文档 `roots[]` 里带 `.prefab` 资产的根恰一个 ⇒ 它的 `scene` 下标。
/// 提取侧把 FBX 与 prefab 各导成一个 scene 且同名，glb 的默认 scene
/// 不保证是 prefab（实测多数包默认是 FBX）——文档根表是唯一可靠分派面。
fn prefab_scene_index(document: &str, leaf: &str) -> usize {
    let value: serde_json::Value = serde_json::from_str(document)
        .unwrap_or_else(|err| panic!("采集物文档不是合法 JSON（{leaf}）：{err}"));
    let roots = value
        .get("roots")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("采集物文档缺 roots 数组（{leaf}）"));
    let mut prefab_roots = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let has_prefab = root
            .get("assets")
            .and_then(|v| v.as_array())
            .is_some_and(|assets| {
                assets
                    .iter()
                    .any(|a| a.as_str().is_some_and(|s| s.ends_with(".prefab")))
            });
        if !has_prefab {
            continue;
        }
        let scene = root
            .get("scene")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| panic!("采集物文档的 prefab 根缺 scene（{leaf}#{index}）"));
        prefab_roots.push(scene as usize);
    }
    match prefab_roots.len() {
        1 => prefab_roots[0],
        0 => panic!("采集物文档没有 prefab 根（{leaf}）"),
        n => panic!("采集物文档有 {n} 个 prefab 根（{leaf}），scene 分派有歧义"),
    }
}

/// 出生抽签的确定性随机：splitmix64 高位取 [0,1)（与雨粒子同式）。
struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }
}

/// Update：逐摆放等 glb 与文档到齐，按文档选 prefab scene、按律落位。
/// 每条独立过门；失败具名 panic。
fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    assets: Option<Res<HarvestGltfs>>,
    docs: Option<Res<HarvestDocs>>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    plans: Option<Res<HarvestPlans>>,
    drops: Option<Res<HarvestDropRows>>,
    ground: Option<Res<GroundMeshes>>,
    meshes: Res<Assets<Mesh>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
    mut spawned: ResMut<HarvestSpawnedCount>,
    mut entities: ResMut<HarvestEntities>,
    mut ground_verts: ResMut<HarvestGroundVerts>,
) {
    let (Some(assets), Some(docs), Some(plans), Some(drops), Some(ground)) =
        (assets, docs, plans, drops, ground)
    else {
        return;
    };
    for (index, plan) in plans.0.iter().enumerate() {
        if index < spawned.0 {
            continue;
        }
        let Some(handle) = assets.by_key.get(plan.package) else {
            panic!("装载计划缺 glb 句柄：{}", plan.package);
        };
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("采集物 glb 装载失败（{}）：{err:?}", plan.leaf);
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(handle)
        {
            panic!("采集物 glb 的依赖装载失败（{}）：{err:?}", plan.leaf);
        }
        if !server.is_loaded_with_dependencies(handle) {
            return;
        }
        let Some(gltf) = gltfs.get(handle) else {
            return;
        };
        let Some(doc_handle) = docs.0.get(plan.package) else {
            panic!("摆放计划缺文档句柄：{}", plan.package);
        };
        match server.load_state(doc_handle) {
            LoadState::Failed(err) => {
                panic!("采集物文档装载失败（{}）：{err:?}", plan.leaf)
            }
            LoadState::Loaded => {}
            _ => return,
        }
        let Some(doc) = json.get(doc_handle) else {
            return;
        };
        let scene_index = prefab_scene_index(&doc.0, &plan.leaf);
        let Some(scene) = gltf.scenes.get(scene_index) else {
            panic!(
                "采集物 glb 的 prefab scene 下标越界（{}）：{} ≥ {}",
                plan.leaf,
                scene_index,
                gltf.scenes.len()
            );
        };

        // 摆放律：随机缩放（SetupScaleRandom：区间是我方 mock，见
        // SCALE_MIN 注释）与随机偏航（Random[0,360) × 0.017453292）。
        // 两次抽值的先后是我方选择（真源两处抽值各自独立，先后不影响
        // 各自的分布）；种子按摆放序确定，跨次运行可复算。
        let mut rng = Rng(0x5EED_0000_0000_0000u64.wrapping_add(index as u64));
        let scale = SCALE_MIN + rng.next_f32() * (SCALE_MAX - SCALE_MIN);
        let yaw_deg = rng.next_f32() * 360.0;
        let yaw = yaw_deg * 0.017453292;
        // 世界位 = 站点原点 + 整数列（站点根是恒等变换 ⇒ 原点即世界原
        // 点）；脚下高度贴地表采样（navmesh 吸附挂账）。
        let x = plan.position_x as f32;
        let z = plan.position_z as f32;
        let verts = ground_verts
            .0
            .get_or_insert_with(|| ground_world_verts(&ground, &meshes, &parts));
        let y = surface_y(verts, x, z, 0.0);
        let hidden = plan.status == STATUS_HARVESTED;
        // 待掉列表同步（真源构造期 SynchronizedTargetBeforeDropItems，
        // 方法体直迁）：按位筛 + status=before_drop + 行 hp ≤ 初始 HP
        // （构造期 HP 就是 master hp；同步后列表只被双臂滤波与逐项移除
        // 触碰）。
        let pending_drops: Vec<PendingDrop> = drops
            .0
            .iter()
            .filter(|drop| {
                drop.position_x == plan.position_x
                    && drop.position_z == plan.position_z
                    && drop.status == DROP_STATUS_BEFORE_DROP
                    && drop.hp <= plan.hp
            })
            .cloned()
            .collect();
        let pending_count = pending_drops.len();
        let entity = commands
            .spawn((
                SceneRoot(scene.clone()),
                HarvestRoot,
                HarvestObject {
                    package: plan.package,
                    leaf: plan.leaf.clone(),
                    fixture_type: plan.fixture_type,
                    fixture_id: plan.fixture_id,
                    position_x: plan.position_x,
                    position_z: plan.position_z,
                    hp: plan.hp,
                    prev_hp: plan.hp,
                    status: plan.status,
                    is_last_attack: false,
                    last_attack_stamina: plan.last_attack_stamina,
                    is_rare: plan.is_rare,
                    radius: plan.radius * scale,
                    collision_type: plan.collision_type,
                    interface: plan.interface,
                    se_hit: plan.se_hit,
                    se_final: plan.se_final,
                    pending_drops,
                    hits_taken: 0,
                },
                Transform::from_translation(Vec3::new(x, y, z))
                    .with_rotation(Quat::from_rotation_y(yaw))
                    .with_scale(Vec3::splat(scale)),
                if hidden {
                    Visibility::Hidden
                } else {
                    Visibility::default()
                },
            ))
            .id();
        entities.0.push(entity);
        info!(
            "采集物摆放 {}#{}：type {}（{} · 接口 {}）hp {} 稀有 {} radius {:.2}×{:.3} 世界 ({:.2}, {:.2}, {:.2}) yaw {:.1}° scene {} 待掉 {}{}",
            plan.leaf,
            plan.fixture_id,
            plan.fixture_type,
            master_type_word(plan.fixture_type),
            interface_word(plan.interface),
            plan.hp,
            plan.is_rare,
            plan.radius,
            scale,
            x,
            y,
            z,
            yaw_deg,
            scene_index,
            pending_count,
            if hidden { " [已收场：进图隐藏]" } else { "" },
        );
        spawned.0 += 1;
    }
}

fn interface_word(interface: ActionInterface) -> &'static str {
    match interface {
        ActionInterface::Multi => "多击",
        ActionInterface::Single => "单击",
    }
}

/// 视图契约 mysekaiSiteHarvestFixtureType 的值域（真源枚举的序列化值）。
/// master 行的 4（treasure_box_fixed）不在视图序列化里出现，落 `_` 臂。
fn master_type_word(view_type: i32) -> &'static str {
    match view_type {
        0 => "wood",
        1 => "mineral",
        2 => "plant",
        3 => "treasure_box",
        5 => "other",
        6 => "tone",
        7 => "toolbox",
        8 => "driftage",
        9 => "birthday_plant",
        _ => "未知",
    }
}

/// 全局观察者：scene 实例展开完毕时计数；全部摆放展开后立换装的闩。
fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    roots: Query<&HarvestRoot>,
    plans: Option<Res<HarvestPlans>>,
    mut count: ResMut<HarvestScenesReadyCount>,
    mut commands: Commands,
) {
    if roots.get(trigger.event().entity).is_err() {
        return;
    }
    let Some(plans) = plans else {
        return;
    };
    count.0 += 1;
    if count.0 == plans.0.len() {
        info!("采集物 scene 全部展开：{}/{}", count.0, plans.0.len());
        commands.insert_resource(HarvestScenesReady);
    }
}

/// Update：排空被击队列，逐击跑真源 OnDamage 的模拟侧律。
///
/// 写者先于读者：`autohit`（冒烟仪表）与玩家交互的拾取（`pick.rs`，
/// 跨链用 `.before` 显式排序）都在本系统之前入队。真源分派键是视图类
/// 实现的接口；本仓在 plan 阶段把视图类折进闭集 [`ActionInterface`]。
pub(crate) fn on_damage(
    mut commands: Commands,
    mut hits: ResMut<HarvestHits>,
    mut objects: Query<(&mut HarvestObject, &mut Visibility, &Transform)>,
    mut punches: Query<&mut HarvestPunch>,
    mut batches: ResMut<HarvestDropBatches>,
    mut stats: ResMut<HarvestStats>,
    mut se: ResMut<SeRequests>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
) {
    // 掉落 pacing 阈值是面板键（HarvestDropDelayItemCount，IntConfigs
    // 176）。面板未立（装载窗内）本帧不消费被击队——被击留在队里下一
    // 帧整批处理，装载闩语义，回落散写常量的路径不存在（面板缺席在
    // 解析点已经响亮 panic）。
    let Some(config) = configs.as_deref() else {
        return;
    };
    let drop_delay_item_count =
        config.int(crate::client_config::KEY_HARVEST_DROP_DELAY_ITEM_COUNT) as usize;
    for hit in hits.0.drain(..) {
        let Ok((mut object, mut visibility, transform)) = objects.get_mut(hit.target) else {
            warn!("采集击打的目标不是采集物：{:?}", hit.target);
            continue;
        };
        let already_harvested = object.status == STATUS_HARVESTED;
        object.hits_taken += 1;
        // UpdateHp 逐句直迁（见 HarvestObject::update_hp 的真源对照）。
        let returned = object.update_hp(hit.damage);
        let ordinal = object.hits_taken;
        let mut tail = String::new();
        // HandleResourceDrop（真源方法体直迁，多击每一击与单击族都走）：
        // 类型门（fixtureType < 10，否则真源 throw）→ 待掉列表按
        // IsLastAttack 双臂滤波——非终结臂「击后 HP < 行 hp ≤ 击前 HP」、
        // 终结臂「行 hp ≥ 击后 HP」（UpdateHp 之后读：HP 是击后值、PrevHp
        // 是击前值；终结击的 IsLastAttack 已由 UpdateHp 置位）→ 批非空
        // 才有声与队列（count = 0 即真源 CreateDropItem 的立即返回臂）。
        assert!(
            object.fixture_type < 10,
            "采集物类型 {} 不在掉落门内（真源 ArgumentOutOfRangeException）",
            object.fixture_type
        );
        let admitted: Vec<PendingDrop> = object
            .pending_drops
            .iter()
            .filter(|drop| {
                if object.is_last_attack {
                    drop.hp >= object.hp
                } else {
                    object.hp < drop.hp && drop.hp <= object.prev_hp
                }
            })
            .cloned()
            .collect();
        if !admitted.is_empty() {
            let count = admitted.len();
            // pacing 阈值来自面板：批内项数达到阈值才逐帧放行，小批同帧。
            let delay = drop_delay_item_count <= count;
            // PlayDropItemSE（批入队前一声）：生日 fixture（fixtureType 9）
            // 独占一声；其余按批内最大稀有度——rarity_2/3 取 rare 声、
            // rarity_1/4 静默、越界即真源 throw。站点门
            // （SiteManager.GetSite 转型 HarvestSiteController，空则整批
            // 跳过且无声）在采集站点上恒过，形状记注释不入码。
            if object.fixture_type == 9 {
                stats.drop_se_birthday += 1;
                se.0.push(SeRequest {
                    owner: None,
                    cue: "se_drop_birthday_material".into(),
                    class: SeClass::Ingame,
                    source: "harvest-drop",
                });
            } else {
                let max_rarity = admitted.iter().map(|drop| drop.rarity).max();
                match max_rarity {
                    Some(1 | 2) => {
                        stats.drop_se_rare += 1;
                        se.0.push(SeRequest {
                            owner: None,
                            cue: "se_drop_rare_material".into(),
                            class: SeClass::Ingame,
                            source: "harvest-drop",
                        });
                    }
                    Some(0 | 3) => {
                        stats.drop_se_silent += 1;
                    }
                    Some(other) => {
                        panic!("掉落批最大稀有度 {other} 越界（真源 throw maxRarity）")
                    }
                    None => unreachable!("批非空必有大稀有度"),
                }
            }
            batches.0.push(DropBatch {
                origin: hit.target,
                remaining: admitted,
                delay,
            });
            stats.drop_batches += 1;
            tail.push_str(&format!(
                "→ 掉落批 {count} 项（{}，pacing 阈值 {drop_delay_item_count}，面板 IntConfigs {}）",
                if delay { "逐帧 pacing" } else { "同帧" },
                crate::client_config::KEY_HARVEST_DROP_DELAY_ITEM_COUNT
            ));
        }
        match object.interface {
            ActionInterface::Multi => {
                if !object.is_last_attack {
                    stats.multi_hits += 1;
                    // 受击演出（PlayMultiActionDamageEffect）挂账，见模块
                    // 注释；掉落已接（本函数头部的 HandleResourceDrop 段）。
                } else {
                    stats.multi_final += 1;
                    if object.is_rare {
                        stats.rare_final += 1;
                    }
                    // 消失律：ChangeAfterObject 的公共终态 = 视觉隐藏
                    // （树的终态是整棵隐藏，无树桩；RemoveCollisionObject
                    // 挂账——本栈无碰撞注册表）。
                    if *visibility != Visibility::Hidden {
                        *visibility = Visibility::Hidden;
                        stats.hidden += 1;
                    }
                    if !already_harvested {
                        stats.post_echoes += 1;
                        info!(
                            "采集 POST 回显（mock）：site_id={SITE} target{{position=({}, {}), fixture_id={}, hp={}, is_last_attack={}}} tool_used=∅ stamina_useds=[{}]",
                            object.position_x,
                            object.position_z,
                            object.fixture_id,
                            object.hp,
                            object.is_last_attack,
                            object.last_attack_stamina
                        );
                        tail.push_str("→ 消失 + POST 回显");
                    } else {
                        stats.idle_returns += 1;
                        tail.push_str(&format!("→ 复击空转（UpdateHp 返回 {returned}，状态不变）"));
                    }
                }
            }
            ActionInterface::Single => {
                stats.single_hits += 1;
                // 单击臂无条件整段执行（真源不看 IsLastAttack）：
                // punch + 掉落（已接，见本函数头部）+ ChangeAfterObject +
                // RemoveCollision。
                let base = match punches.get_mut(hit.target) {
                    Ok(mut punch) => {
                        // DOTween 的替换语义：重开计时，基准位不变。
                        punch.elapsed = 0.0;
                        None
                    }
                    Err(_) => Some(transform.translation),
                };
                if let Some(base) = base {
                    commands
                        .entity(hit.target)
                        .insert(HarvestPunch { elapsed: 0.0, base });
                }
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                    stats.hidden += 1;
                }
                if !already_harvested {
                    stats.post_echoes += 1;
                    info!(
                        "采集 POST 回显（mock）：site_id={SITE} target{{position=({}, {}), fixture_id={}, hp={}, is_last_attack={}}} tool_used=∅ stamina_useds=[{}]",
                        object.position_x,
                        object.position_z,
                        object.fixture_id,
                        object.hp,
                        object.is_last_attack,
                        object.last_attack_stamina
                    );
                    tail.push_str("→ 消失 + punch + POST 回显");
                } else {
                    stats.idle_returns += 1;
                    tail.push_str(&format!("→ 复击空转（UpdateHp 返回 {returned}，状态不变）"));
                }
            }
        }
        // 受击 SE（一次性族入队，通道侧做 2.0s 同 cue 抑制）：已收场复击
        // = UpdateHp 返回 0 臂，无演出即无声；终结击取 final（稀有已折成
        // break_rare），其余取 hit（单击族只有 hit）。
        let se_cue = if already_harvested {
            None
        } else if object.interface == ActionInterface::Multi && object.is_last_attack {
            object.se_final
        } else {
            object.se_hit
        };
        if let Some(cue) = se_cue {
            se.0.push(SeRequest {
                owner: None,
                cue: cue.into(),
                class: SeClass::Ingame,
                source: "harvest-hit",
            });
        }
        stats.hits += 1;
        info!(
            "采集击中 {}#{}（type {}）第 {} 击：damage {} prev_hp {} → hp {} ret {} 接口 {} last={} SE {} {}",
            object.leaf,
            object.fixture_id,
            object.fixture_type,
            ordinal,
            hit.damage,
            object.prev_hp,
            object.hp,
            returned,
            interface_word(object.interface),
            object.is_last_attack,
            se_cue.unwrap_or("∅"),
            tail
        );
    }
}

/// Update：单击族的 punch 推进（世界 X 轴偏移，端点回基准位）。
/// （`pub(crate)` 仅供 schedule 里玩家态写者锚 `after`——段信号是在飞
/// punch 的事后读数；逻辑不变。）
pub(crate) fn advance_punch(
    time: Res<Time>,
    mut commands: Commands,
    mut punches: Query<(Entity, &mut HarvestPunch, &mut Transform)>,
) {
    for (entity, mut punch, mut transform) in &mut punches {
        punch.elapsed += time.delta_secs();
        let t = punch.elapsed / PUNCH_DURATION;
        if t >= 1.0 {
            transform.translation = punch.base;
            commands.entity(entity).remove::<HarvestPunch>();
        } else {
            // 单次出-回包络：峰值 ≤ 振幅、端点回零（DOTween punch 曲线
            // 的近似，见模块注释）。
            let offset = PUNCH_AMPLITUDE * (std::f32::consts::PI * t).sin() * (1.0 - t);
            transform.translation = punch.base + Vec3::X * offset;
        }
    }
}

/// Update：掉落批逐项消耗（真源 CreateDropItem 的批循环 + fire-and-
/// forget 单项任务）：拒绝行具名记日志并照常消耗；模型行等资产到齐后
/// 按律生成——极坐标散布（角度 [0,2π) 弧度 → 半径 [min,max]，抽值顺序
/// 照真源：角度 → 半径 → yaw → r）、落位贴地表采样（raycast 向下打地面
/// 的近似，见模块注释 navmesh 挂账）、散布动画随实体挂上（Y 双跳 + XZ
/// OutCubic）。delay 批逐帧出一项（DelayFrame(1) 的 pacing），其余同帧
/// 出全；单项资产未到齐就原地持着（真源每个单项任务是独立异步，互不
/// 阻塞）。
fn spawn_drops(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    assets: Option<Res<HarvestGltfs>>,
    docs: Option<Res<HarvestDocs>>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    ground: Res<HarvestGroundVerts>,
    mut batches: ResMut<HarvestDropBatches>,
    mut objects: Query<&mut HarvestObject>,
    mut seq: ResMut<HarvestDropSeq>,
    mut stats: ResMut<HarvestStats>,
) {
    let (Some(assets), Some(docs)) = (assets, docs) else {
        return;
    };
    let Some(verts) = ground.0.as_ref() else {
        return; // 地表快照还没采（首次摆放前不会有击，防守性早退）
    };
    for batch in &mut batches.0 {
        let Ok(mut object) = objects.get_mut(batch.origin) else {
            panic!(
                "掉落批的来源实体不存在：{:?}（采集物不 despawn，实体丢了是装配断点）",
                batch.origin
            );
        };
        // 出生位 = 来源摆放位（真源 CreateDropItem 的 target position：
        // PositionX、0、PositionZ——y 分量显式 0）。
        let target_x = object.position_x as f32;
        let target_z = object.position_z as f32;
        let quota = if batch.delay { 1 } else { usize::MAX };
        let mut taken = 0usize;
        while taken < quota {
            let Some(item) = batch.remaining.first().cloned() else {
                break;
            };
            let package = match &item.prefab {
                // GetDropItemPrefab 的 null 臂：真源已具名记日志、单项任务
                // 静默完成；行在批循环里照常消耗（RemoveTargetBeforeDropItem
                // 在 Forget 之后无条件执行）。
                DropPrefab::Unresolved(reason) => {
                    stats.drop_refused += 1;
                    info!(
                        "[harvest-drop] 具名拒绝：{reason}（resourceType {} · id {} · 行 hp {} · seq {}）",
                        item.resource_type, item.resource_id, item.hp, item.seq
                    );
                    remove_pending(&mut object, item.row);
                    batch.remaining.remove(0);
                    taken += 1;
                    continue;
                }
                DropPrefab::Model { package } => package.clone(),
            };
            // 资产门：模型 glb（连同依赖）与文档到齐才生成。
            let Some(handle) = assets.by_key.get(&package) else {
                panic!("掉落模型包不在装载计划里：{package}");
            };
            if let LoadState::Failed(err) = server.load_state(handle) {
                panic!("掉落模型 glb 装载失败（{package}）：{err:?}");
            }
            if !server.is_loaded_with_dependencies(handle) {
                break;
            }
            let Some(gltf) = gltfs.get(handle) else {
                break;
            };
            let Some(doc_handle) = docs.0.get(&package) else {
                panic!("掉落模型包缺文档句柄：{package}");
            };
            match server.load_state(doc_handle) {
                LoadState::Failed(err) => {
                    panic!("掉落模型文档装载失败（{package}）：{err:?}")
                }
                LoadState::Loaded => {}
                _ => break,
            }
            let Some(doc) = json.get(doc_handle) else {
                break;
            };
            let leaf = package.rsplit("__").next().unwrap_or(&package);
            let scene_index = prefab_scene_index(&doc.0, leaf);
            let Some(scene) = gltf.scenes.get(scene_index) else {
                panic!(
                    "掉落模型 glb 的 prefab scene 下标越界（{leaf}）：{scene_index} ≥ {}",
                    gltf.scenes.len()
                );
            };

            // 抽值（顺序照真源）：角度 [0,2π) 弧度 → 半径 [min,max]；
            // yaw 与 r 只在散布臂抽（d__38 的两处 Random）。
            seq.0 += 1;
            let uid = seq.0;
            let mut rng = Rng(0xD30D_0000_0000_0000u64.wrapping_add(uid));
            let (min_range, max_range, height) =
                drop_scatter_params(item.resource_type, item.material_type);
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let range = min_range + rng.next_f32() * (max_range - min_range);
            let is_scatter = 0.0 < max_range;
            // searchPosition = target + (cos·range, height, sin·range)；
            // 站点原点在世界上即原点，站点项为 0。
            let land_x = target_x + angle.cos() * range;
            let land_z = target_z + angle.sin() * range;
            // 落位 Y：raycast 向下打地面的近似——地表顶点采样，miss 臂的
            // fallback 就是散布高度档。
            let land_y = surface_y(verts, land_x, land_z, height);
            let spawn_pos = Vec3::new(target_x, 0.0, target_z);
            let landing = Vec3::new(land_x, land_y, land_z);
            // d1 先封顶后托底（clamp(distance, 0.6, 1.0)）；d2 = clamp(1−
            // distance, 0.3, 1.0)。
            let distance = spawn_pos.distance(landing);
            let d1 = distance.clamp(0.6, 1.0);
            let d2 = (1.0 - distance).clamp(0.3, 1.0);
            let yaw_deg = if is_scatter {
                rng.next_f32() * 360.0
            } else {
                0.0
            };
            let yaw = yaw_deg * 0.017453292;
            let r = if is_scatter {
                0.6 + rng.next_f32() * (0.8 - 0.6)
            } else {
                0.0
            };

            // ShowRareDropEffect 的分支键（粒子本体挂账；越界即真源
            // LogError，本仓 panic）。
            match item.rarity {
                0 => stats.drop_effect_normal += 1,
                1 => stats.drop_effect_rare += 1,
                2 => stats.drop_effect_ultra += 1,
                other => panic!("掉落特效稀有度 {other} 越界（真源 LogError）"),
            }

            let entity = commands
                .spawn((
                    SceneRoot(scene.clone()),
                    HarvestDropItem {
                        uid,
                        radius: if is_scatter { 0.0 } else { 1.0 },
                        rarity: item.rarity,
                        resource_type: item.resource_type,
                        resource_id: item.resource_id,
                    },
                    // 出生即摆位（localPosition = target 位置，y = 0）；
                    // 旋转只在散布臂设置（d__38 的 set_rotation）。
                    Transform::from_translation(spawn_pos)
                        .with_rotation(Quat::from_rotation_y(yaw)),
                    Visibility::default(),
                ))
                .id();
            if is_scatter {
                commands.entity(entity).insert(HarvestDropAnimation {
                    elapsed: 0.0,
                    spawn: spawn_pos,
                    landing,
                    d1,
                    d2,
                    r,
                });
            } else {
                stats.drop_noscatter += 1;
            }
            stats.drop_items += 1;
            info!(
                "[harvest-drop] 生成 {leaf}：uid {uid} · resourceType {} id {} · 行 hp {} seq {} qty {} · 稀有度 {}（特效 {}）· {}· 出生 ({:.2}, 0, {:.2}) · 角度 {:.1}° 半径 {:.2} · 落位 ({:.2}, {:.2}, {:.2}){}",
                item.resource_type,
                item.resource_id,
                item.hp,
                item.seq,
                item.quantity,
                item.rarity,
                effect_word(item.rarity),
                if is_scatter {
                    format!("散布（min {min_range} max {max_range} 高 {height}）")
                } else {
                    "不散布（tone 档：radius 1.0、无动画）".to_string()
                },
                target_x,
                target_z,
                angle.to_degrees(),
                range,
                landing.x,
                landing.y,
                landing.z,
                if is_scatter {
                    format!(
                        " · yaw {yaw_deg:.1}° · d1 {d1:.3} d2 {d2:.3} r {r:.3}（一跳峰 {:.3} 二跳峰 {:.3}）",
                        landing.y + d1 * r,
                        landing.y + d2 * d1 * r * DROP_SECOND_HOP
                    )
                } else {
                    String::new()
                },
            );
            remove_pending(&mut object, item.row);
            batch.remaining.remove(0);
            taken += 1;
        }
    }
    batches.0.retain(|batch| !batch.remaining.is_empty());
}

/// 从来源摆放的待掉列表移除一行（真源 RemoveTargetBeforeDropItem；
/// List.Remove 的 miss 是无害空操作——终局臂把在途行再次收进批时，
/// 第二次移除按真源形状静默放过）。
fn remove_pending(object: &mut HarvestObject, row: usize) {
    if let Some(index) = object.pending_drops.iter().position(|drop| drop.row == row) {
        object.pending_drops.remove(index);
    }
}

/// ShowRareDropEffect 三臂的日志用词。
fn effect_word(rarity: i32) -> &'static str {
    match rarity {
        0 => "常",
        1 => "稀",
        2 => "极稀",
        _ => "越界",
    }
}

/// Update：掉落散布动画推进。真源 Sequence 结构（DOTween）：
/// - 主位移 DOMove(落位, 0.6, OutCubic)——XZ 出 0.6s；
/// - Y 轴被 Join/Insert 的四段 DOMoveY 覆盖成双跳：[0, 0.18] 出（峰
///   y_land+d1·r，OutQuad）· [0.18, 0.36] 落 y_land（InQuad）·
///   [0.36, 0.48] 出（峰 y_land+d2·d1·r·0.43，OutQuad）· [0.48, 0.60]
///   落 y_land（InQuad）；
/// - 结束后位置精确等于落位（tween 端点），组件移除。
fn advance_drop_animations(
    time: Res<Time>,
    mut commands: Commands,
    mut drops: Query<(Entity, &mut HarvestDropAnimation, &mut Transform)>,
) {
    for (entity, mut anim, mut transform) in &mut drops {
        anim.elapsed += time.delta_secs();
        let t = anim.elapsed;
        if t >= DROP_DURATION {
            transform.translation = anim.landing;
            commands.entity(entity).remove::<HarvestDropAnimation>();
            continue;
        }
        let y_land = anim.landing.y;
        let first_peak = y_land + anim.d1 * anim.r;
        let second_peak = y_land + anim.d2 * anim.d1 * anim.r * DROP_SECOND_HOP;
        let y = if t < 0.18 {
            // 出生 y = 0（批侧显式置 0），首段从 0 出峰。
            let u = t / 0.18;
            anim.spawn.y + (first_peak - anim.spawn.y) * ease_out_quad(u)
        } else if t < 0.36 {
            let u = (t - 0.18) / 0.18;
            first_peak + (y_land - first_peak) * ease_in_quad(u)
        } else if t < 0.48 {
            let u = (t - 0.36) / 0.12;
            y_land + (second_peak - y_land) * ease_out_quad(u)
        } else {
            let u = (t - 0.48) / 0.12;
            second_peak + (y_land - second_peak) * ease_in_quad(u)
        };
        let u = ease_out_cubic(t / DROP_DURATION);
        let x = anim.spawn.x + (anim.landing.x - anim.spawn.x) * u;
        let z = anim.spawn.z + (anim.landing.z - anim.spawn.z) * u;
        transform.translation = Vec3::new(x, y, z);
    }
}

/// 冒烟口的状态机（`Local`；脚本的阶段与计数）。
#[derive(Default)]
struct AutoHitState {
    /// 已收场的目标数（表序里 status=spawned 的行依次推进）。
    phase: usize,
    /// 当前目标已发击数。
    sent: usize,
    /// 复击段剩余次数。
    probe_left: Option<usize>,
    finished: bool,
    warned_window: bool,
}

/// 每帧注入的击打数（脚本速率；91 击的多击链约 16 帧）。
const HITS_PER_FRAME: usize = 6;
/// 单目标击打上限：hp 90 damage 1 的律值是 91 击收场，超限即律坏。
const AUTO_HIT_CAP: usize = 200;

/// 无工具（damage 1）下打到收场的律值击数：hp ≥ 1 时 hp+1（hp 击打空
/// HP + 终结 1 击），hp = 0 时 1 击（单击族直接终结）。
fn law_hit_count(hp: i32) -> usize {
    hp.max(0) as usize + 1
}

/// 冒烟口（`MOLY_HARVEST_AUTOHIT_SECS`，宿主侧仪表，同仓 player 的
/// `MOLY_PLAYER_AUTOWALK_SECS` 同款）：设为正数时启动后该秒数内按固定
/// 脚本自动击打——多击族逐击打到收场（hp 90 damage 1 的律值 91 击：
/// 超过 [`AUTO_HIT_CAP`] 未收场即 panic，把多击律坏掉变成响亮失败）、
/// 单击族各一击、全部收场后对第一个单击目标复击三下（UpdateHp 的
/// 返回 0 臂）。脚本自带终点；窗口先关则 warn 半途而废。
fn autohit(
    mut hits: ResMut<HarvestHits>,
    entities: Option<Res<HarvestEntities>>,
    plans: Option<Res<HarvestPlans>>,
    objects: Query<&HarvestObject>,
    time: Res<Time>,
    mut state: Local<AutoHitState>,
) {
    let armed = match std::env::var("MOLY_HARVEST_AUTOHIT_SECS") {
        Ok(raw) => raw.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as f32,
        Err(_) => 0.0,
    };
    if armed <= 0.0 || state.finished {
        return;
    }
    if time.elapsed_secs() >= armed {
        if !state.warned_window {
            state.warned_window = true;
            warn!(
                "自动击打窗口结束，脚本未完成（phase {} · 已发 {} 击）",
                state.phase, state.sent
            );
        }
        return;
    }
    let (Some(entities), Some(plans)) = (entities, plans) else {
        return;
    };
    // 目标序列：表序里 status=spawned 的行。
    let active: Vec<usize> = plans
        .0
        .iter()
        .enumerate()
        .filter(|(_, plan)| plan.status != STATUS_HARVESTED)
        .map(|(index, _)| index)
        .collect();
    if state.phase < active.len() {
        let target_index = active[state.phase];
        let Some(&entity) = entities.0.get(target_index) else {
            return; // 该行还没 spawn
        };
        let Ok(object) = objects.get(entity) else {
            return;
        };
        if object.status == STATUS_HARVESTED {
            let law_hits = law_hit_count(plans.0[target_index].hp);
            info!(
                "[harvest-autohit] {} 收场：{} 击（起始 hp {} · 律值 {law_hits} 击；发数与律值不符即律坏）",
                object.leaf, state.sent, plans.0[target_index].hp
            );
            state.phase += 1;
            state.sent = 0;
            return;
        }
        if state.sent >= AUTO_HIT_CAP {
            panic!(
                "自动击打 {} 超过 {AUTO_HIT_CAP} 击未收场——多击律坏了（律值 {} 击 · 当前 hp {}）",
                object.leaf,
                law_hit_count(plans.0[target_index].hp),
                object.hp
            );
        }
        // 按律值 pacing 收尾：91 击的多击链在收场击后不拖空转尾巴，日志
        // 计数逐类可推导。律值发完仍未收场（律坏）则逐帧补击到上限 panic，
        // 不干等。
        let law_hits = law_hit_count(plans.0[target_index].hp);
        let batch = if state.sent < law_hits {
            HITS_PER_FRAME.min(law_hits - state.sent)
        } else {
            1
        };
        for _ in 0..batch {
            hits.0.push(HarvestHit {
                target: entity,
                damage: 1,
                tool_level: 0,
                is_boost: false,
            });
            state.sent += 1;
        }
        return;
    }
    // 复击段：第一个单击族目标（表序）。
    let probe_left = state.probe_left.get_or_insert(3);
    if *probe_left > 0 {
        let probe_index = active
            .iter()
            .copied()
            .find(|&index| plans.0[index].interface == ActionInterface::Single)
            .expect("摆放 mock 里没有单击族目标，复击段选不到目标");
        let Some(&entity) = entities.0.get(probe_index) else {
            return;
        };
        hits.0.push(HarvestHit {
            target: entity,
            damage: 1,
            tool_level: 0,
            is_boost: false,
        });
        *probe_left -= 1;
        return;
    }
    state.finished = true;
    info!(
        "[harvest-autohit] 脚本完成：{} 个目标全部收场 + 3 次复击空转",
        active.len()
    );
}

/// 周期状态行（2s）：账本现算，供冒烟对账。
fn report(
    stats: Res<HarvestStats>,
    objects: Query<&HarvestObject>,
    anims: Query<&HarvestDropAnimation>,
    hits: Res<HarvestHits>,
) {
    let total = objects.iter().count();
    let harvested = objects
        .iter()
        .filter(|object| object.status == STATUS_HARVESTED)
        .count();
    let pending: usize = objects
        .iter()
        .map(|object| object.pending_drops.len())
        .sum();
    info!(
        "[harvest] 摆放 {} · 已收场 {} · 击中 {}（多击非终结 {} · 多击终结 {} · 单击 {}）· 稀有终结 {} · 消失 {} · POST 回显 {} · 复击空转 {} · 掉落批 {} · 生成 {}（拒 {} · 无散布 {} · 动画在跑 {}）· SE（生日 {} · rare {} · 静默 {}）· 特效（常 {} · 稀 {} · 极稀 {}）· 待掉余 {} · 队列余 {}",
        total,
        harvested,
        stats.hits,
        stats.multi_hits,
        stats.multi_final,
        stats.single_hits,
        stats.rare_final,
        stats.hidden,
        stats.post_echoes,
        stats.idle_returns,
        stats.drop_batches,
        stats.drop_items,
        stats.drop_refused,
        stats.drop_noscatter,
        anims.iter().count(),
        stats.drop_se_birthday,
        stats.drop_se_rare,
        stats.drop_se_silent,
        stats.drop_effect_normal,
        stats.drop_effect_rare,
        stats.drop_effect_ultra,
        pending,
        hits.0.len(),
    );
}

/// 采集物装载与交互插件。材质换装在 `harvest_material.rs` 的插件里，
/// 两边以 [`HarvestScenesReady`] 闩交接；链序即写者先于读者
/// （autohit → on_damage → spawn_drops → advance_punch →
/// advance_drop_animations）。
pub struct HarvestPlugin;

impl Plugin for HarvestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HarvestHits>()
            .init_resource::<HarvestStats>()
            .init_resource::<HarvestSpawnedCount>()
            .init_resource::<HarvestScenesReadyCount>()
            .init_resource::<HarvestEntities>()
            .init_resource::<HarvestDropBatches>()
            .init_resource::<HarvestDropRows>()
            .init_resource::<HarvestDropSeq>()
            .init_resource::<HarvestGroundVerts>()
            .add_systems(Startup, load)
            .add_observer(on_scene_ready)
            .add_systems(
                Update,
                (
                    plan_when_ready,
                    check_loaded,
                    spawn_when_ready,
                    autohit,
                    on_damage,
                    spawn_drops,
                    advance_punch,
                    advance_drop_animations,
                    report.run_if(bevy::time::common_conditions::on_timer(
                        std::time::Duration::from_secs(2),
                    )),
                )
                    .chain(),
            );
    }
}
