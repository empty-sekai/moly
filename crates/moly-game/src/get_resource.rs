//! 获得 · 子窗口对话框（真源 `MysekaiGetResourceSubWindowDialog`，
//! DialogType 329，Dialog 槽）——Dialog 族里复用面最宽的一件：四个开门者
//! 共用（`UIUtility.ShowRewardDialog` / `MysekaiResourceUtility.NoticeCollectItem`
//! / `MysekaiSecretShopPresenter` / `SketchUtility.ShowSketchResultDialog`）。
//!
//! ## 入口与身份（Dialog 槽，不是层栈）
//!
//! 挂 `DisplayLayerType.Layer_Dialog(4)` 恒压 UI 层；与菜单对话框同槽
//! （单活跃对话框，本仓由模态保证互斥：一窗开着时另一窗的开门者不可达）。
//! `Initialize(messageBody, onClose, allowCloseExternal=true)`（两弹法第三参
//! 同为 allowCloseExternal，原生签名核过）⇒ **框外点按收框**。
//!
//! ## 窗口构件（序列化字段表穷举）
//!
//! 基类 `SubWindowDialog` = `messageBodyText` + `SubWindowComponent`
//! （windowType · contentTransform · backPanelTransform · closeButtons——
//! **无标题栏**）+ 开关动画；本体自有序列化件三件：`_thumbnailRoot` ·
//! `_thumbnailWithBalloonPrefab` · `_blueprintView`。可按件 = 缩略图位
//! （点按出名称气球）+ 关闭钮（closeButtons，`Close` → `CloseProcess` =
//! `HideAsync` + `PlaySEOneShot(SE_SUBWINDOW_CLOSE)` + Dispose 资产包）。
//!
//! ## Setup 四臂分派（`Setup(resource, messageBodyKey)` 按 ResourceType switch）
//!
//! | 臂 | 资源类型 | messageBody 组装 |
//! |---|---|---|
//! | 蓝图 | `mysekai_blueprint`(40) | **不走缩略图**（换 `_blueprintView`）→ `Format(MSG_GET_BLUEPRINT, GetResourceName)` =「获得了“{0}”。」→ await 蓝图视图（名称气球钮 + 图标 + 3D 预览） |
//! | 道具 | `mysekai_item`(42) | 缩略图 → `MysekaiItemModel(id).MysekaiItemType`：==2（多余唱片）→ 固定「因为已获得过该唱片，作为代替，获得了“多余的唱片”。」；==1（多余设计图）→ `Format(MSG_GET_SURPLUS_BLUEPRINT, GetResourceName)`；其余 → `Wording.Get(messageBodyKey)`（调用方 key） |
//! | 唱片 | `mysekai_music_record`(44) | 缩略图（尺寸三角 200×200）→ 固定「获得了唱片。」(MSG_GET_RECORD) |
//! | 其余 | default | 缩略图 → `Wording.Get(messageBodyKey)` |
//!
//! ⚠ **`messageBodyKey` 在两个开门者手里是死参**：秘密商店与写生都显式传
//! `MSG_RECEIVED_REWARD`（获得奖励。），但它们的资源都是 type 40 → 走蓝图
//! 臂 → 蓝图臂不读 messageBodyKey。该 key 只在「道具臂 ItemType ∉ {1,2}」
//! 与「default 臂」两处被读。
//!
//! ## 数量与名称气球
//!
//! 数量不进 messageBody——走缩略图 `innerText` 徽记，格式串字面量
//! `×{0}`（原生核过）。缩略图位点按 → `UIPartsCommonBalloon.Setup(GetResourceName,
//! 1, 1)`——名称气球显示条目名（自动调宽、框外收，本仓取点按切换 + 换格
//! 重置的简化形，自动调宽未接具名）。蓝图臂的 `_blueprintView` 里还有一个
//! `_showsBalloonButton` 走同一条名称气球律。
//!
//! ## 名称链（GetResourceName 按 ResourceType 的分派，方法体直读）
//!
//! - 40 → `MysekaiResourceUtility.GetMysekaiBlueprintName(id)`：查蓝图表
//!   `MysekaiCraftType`——1 → 道具名、0 与 2 → 家具名（craftTargetId 查表），
//!   统一 `Format(FORMAT_MYSEKAI_BLUEPRINT_NAME)` =「{0}的设计图」。
//! - 42 → `mysekaiItems` 表的 name 列（整表恰 5 行，公开镜像在盘：空白
//!   设计图/多余设计图/多余唱片/胶卷/设计图碎片——**真值**，随模块携带）。
//! - 44 → `MysekaiMusicRecordModel(id).RecordTitle` =
//!   `Format(WORD_FORMAT_RECORD_NAME, MusicTitle)` =「《{0}》的唱片」。
//! - 其余 → 大 master 分派（卡片/服装/唱片曲名/素材/荣誉…），对应表不在
//!   本仓提取产物 ⇒ `other` 条目的名称由 mock 面板直供。
//!
//! 文案五键值全部来自公开镜像 wording 表（真值，随模块携带）；道具臂
//! ItemType==1 的文案带 `<color=#ff5588>` 富文本跨段——本仓铺字器单色，
//! 跨段色未接（具名挂账），标签剥除后正文保留。
//!
//! ## 四开门者（方法体逐个直读）
//!
//! | 开门者 | 弹法 | 资源 | messageBodyKey |
//! |---|---|---|---|
//! | `UIUtility.ShowRewardDialog` | 分组逐资源 `ChainSubWindowDialog(null, null, true, 329, 4)` + `Setup().Forget()`；一组 ≥6 条走竖版 CommonReward 不走此窗 | 分组 `List<UserResource>`（服务端响应） | 调用方传入 |
//! | `MysekaiResourceUtility.NoticeCollectItem`（蓝图/唱片臂） | 同上 Chain 形 + `Play(回调)`；**重复获得**转 `GetMysekaiItem(1)/(2)`（ItemType 1/2 的行）为 mysekai_item 条目 | 蓝图 {id, 40, 1, qty}；唱片 {id, 44, 1, qty} | null |
//! | `MysekaiSecretShopPresenter` | `ShowSubWindowDialog(null, OnClosedGetResourceDialog, true, 329, 4)` | 购买响应（课金域出范围） | MSG_RECEIVED_REWARD（死参，见上） |
//! | `SketchUtility.ShowSketchResultDialog(blueprintId, onClose)` | 同上 ShowSubWindowDialog + **await** Setup | `new UserResource(blueprintId, 40, level=0, 1)` | 同上 |
//!
//! 链播放器（`ChainDialogPlayer`）：入队 N 格，每格 OnClose → 出队开下一格，
//! 空队列 → `Play` 的 onFinish。本仓取同形：关一格出队开下一格（同窗重
//! Setup），播完走完成沿。
//!
//! ## 开窗沿（OpenAsync 逐句）
//!
//! await `WaitUntil(isSetup)` → `SceneManager.CurrentScene == Mysekai(8)` ?
//! `PlaySEOneShot(se_get_blueprint)` : `PlaySEOneShot(SE_REWARD_DIALOG_OPEN)`
//! → `DialogBase.Open`。本仓只有 mysekai 场景 ⇒ **恒走 se_get_blueprint 臂**
//! （SE 域未接，具名挂账，只记账不发声）。开关动画未接（对话框族既有
//! 简化同款）。
//!
//! ## 服务端域与 mock 面板
//!
//! `UserResource` 载荷（resourceId/resourceType/resourceLevel/quantity）与
//! 四开门者的触发上下文都是服务端响应 ⇒ 具名 mock（环境变量
//! `MOLY_GET_RESOURCE_MOCK_OPENER` = reward|collect|secret_shop|sketch ·
//! `MOLY_GET_RESOURCE_MOCK_ENTRIES` 条目语法
//! `blueprint:id:level:qty:素材名` · `item:id:level:qty[:文案]` ·
//! `record:id:level:qty:曲名` · `other:id:level:qty:名:文案`，逗号分隔多
//! 条目，非法条目响亮告警跳过、全废回默认三件）。条目名里蓝图/唱片两臂
//! 用**真格式律**当场组装（FORMAT_MYSEKAI_BLUEPRINT_NAME /
//! WORD_FORMAT_RECORD_NAME），道具臂查真值 5 行表；开门行报开门者名与
//! 弹法参数。
//!
//! ## 我方选值与具名缺口（改这里之前先读）
//!
//! - 视图来自 GetResource 源预制体，正文和名称写入 UiPrefabView；
//!   缩略图/蓝图预览节点按条目类型显隐，点击读取同一视图的当前源矩形。
//! - 固定文案并入外壳字符集（图集的**第七个**消费者）：动态值只有条目名
//!   与数字，默认集装载期可枚举全量；环境变量覆写的自定义名缺字形在铺
//!   字点 fail-closed（静默缺字是看不见的错值）。
//! - **模态**：开着时全部点按先过它（与菜单对话框同一条 Dialog 槽模态律）；
//!   外壳点按的门读同一个 get_resource_open 位。动作按钮的点按消费不过
//!   这道门（既有具名挂账，本模块不动它的判定面）。
//!
//! 具名挂账（本模块不实现，收工报里重列）：
//! - 四开门者的门一个未建：通用奖励 API（UIUtility 调用方）· 采集结算域
//!   （NoticeCollectItem）· 写生域（SketchUtility）——三件待建；秘密商店
//!   课金域出范围（范围通则）。菜单对话框的水晶商店钮走 PushUIScreen
//!   (CrystalShop) 不走此窗（已核）。
//! - 蓝图 3D 预览 · 缩略图贴图（卡片/道具/蓝图/唱片四类专用缩略图）·
//!   名称气球自动调宽 · 富文本跨段色 · 开关动画 · SE（开窗 se_get_blueprint
//!   与关窗 SE_SUBWINDOW_CLOSE）· 蓝图/唱片/素材名 master 表落盘（落盘后
//!   名字链换真值，切换点只有 mock 条目组装一处）。

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::action_button::ActionTapConsumed;
use crate::balloon::canvas_scale;
use crate::gesture::{GestureEvent, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;

// ---------------------------------------------------------------------------
// 资源条目：数据形（UserResource 载荷镜像 + 名字链产物）
// ---------------------------------------------------------------------------

/// 资源类型（`UserResource.resourceType` 的字符串名 → `ResourceType` 枚举；
/// Setup 四臂分派的键）。前三臂闭集，其余全部走 default 臂。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    /// `mysekai_blueprint`(40)——蓝图臂（不走缩略图，换蓝图视图）。
    MysekaiBlueprint,
    /// `mysekai_item`(42)——道具臂（ItemType 三分派）。
    MysekaiItem,
    /// `mysekai_music_record`(44)——唱片臂（固定文案）。
    MysekaiMusicRecord,
    /// 其余全部资源类型（default 臂：Get(messageBodyKey)）。
    Other,
}

impl ResourceKind {
    /// 线名（日志与条目行用；真源 resourceType 字符串）。
    fn wire_name(self) -> &'static str {
        match self {
            ResourceKind::MysekaiBlueprint => "mysekai_blueprint(40)",
            ResourceKind::MysekaiItem => "mysekai_item(42)",
            ResourceKind::MysekaiMusicRecord => "mysekai_music_record(44)",
            ResourceKind::Other => "其余(default 臂)",
        }
    }
}

/// `mysekaiItems` master 表整表（公开镜像，**真值**）：id → (ItemType 枚举
/// 值, 显示名)。ItemType 枚举：white_blueprint=0 · surplus_blueprint=1 ·
/// surplus_music_record=2 · mysekai_photo_film=3 · blueprint_fragment=4。
/// 道具臂的 ItemType 三分派与名称都查这里；表落盘后换提取产物，切换点
/// 唯一。
const MYSEKAI_ITEMS: [(u32, u8, &str); 5] = [
    (1, 0, "空白设计图"),
    (2, 1, "多余设计图"),
    (5, 2, "多余唱片"),
    (6, 3, "胶卷"),
    (7, 4, "设计图碎片"),
];

/// 一条资源条目（`UserResource` 载荷的镜像形 + 名字链产物）。
#[derive(Debug, Clone)]
pub(crate) struct ResourceEntry {
    kind: ResourceKind,
    resource_id: u32,
    resource_level: u32,
    quantity: u32,
    /// 条目显示名（名称气球与条目行的显示面）：蓝图/唱片两臂由真格式律
    /// 组装，道具臂查 5 行表，other 臂 mock 直供（名字链的 master 表不在
    /// 提取产物）。
    name: String,
    /// messageBody 文案（真源 = `Wording.Get(调用方 messageBodyKey)` 的
    /// 查表值）：只有道具臂 ItemType ∉ {1,2} 与 default 臂读它，缺值在
    /// 铺装点 fail-closed。
    message: Option<String>,
}

impl ResourceEntry {
    /// 当前条目的 messageBody 组装（Setup 四臂逐臂）。
    fn message_body(&self) -> String {
        match self.kind {
            // 蓝图臂：Format(MSG_GET_BLUEPRINT, GetResourceName)。
            ResourceKind::MysekaiBlueprint => format!("获得了“{}”。", self.name),
            // 道具臂：ItemType 三分派（ItemType==2 固定文案不格式化——
            // 文案字面量里名字是写死的；==1 格式化；其余调用方 key）。
            ResourceKind::MysekaiItem => {
                let Some((_, item_type, _)) = MYSEKAI_ITEMS.iter().find(|(id, _, _)| *id == self.resource_id) else {
                    // 查不到 id 的条目在 mock 解析点已拒，到不了这里。
                    panic!(
                        "[get_resource] mysekaiItems 表没有 id={}（构造点已穷举，fail-closed）",
                        self.resource_id
                    );
                };
                match item_type {
                    2 => "因为已获得过该唱片，作为代替，获得了“多余的唱片”。".to_owned(),
                    1 => format!("因为已获得过该设计图，作为代替，获得了“{}”。", self.name),
                    _ => self.message.clone().unwrap_or_else(|| {
                        panic!(
                            "[get_resource] 道具臂 ItemType={} 需要 messageBodyKey（调用方文案），\
                             条目未给（fail-closed）",
                            item_type
                        )
                    }),
                }
            }
            // 唱片臂：固定 MSG_GET_RECORD。
            ResourceKind::MysekaiMusicRecord => "获得了唱片。".to_owned(),
            // default 臂：Get(messageBodyKey)。
            ResourceKind::Other => self.message.clone().unwrap_or_else(|| {
                panic!("[get_resource] default 臂需要 messageBodyKey（调用方文案），条目未给（fail-closed）")
            }),
        }
    }

    /// Setup 臂名（条目行用）。
    fn arm_name(&self) -> &'static str {
        match self.kind {
            ResourceKind::MysekaiBlueprint => "蓝图",
            ResourceKind::MysekaiItem => "道具",
            ResourceKind::MysekaiMusicRecord => "唱片",
            ResourceKind::Other => "default",
        }
    }
}

// ---------------------------------------------------------------------------
// 开门者（四件，参数定谳见模块头表）
// ---------------------------------------------------------------------------

/// 四个开门者（mock 面板具名；真源里各是自己的域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GetResourceOpener {
    /// `UIUtility.ShowRewardDialog`（通用奖励 API，分组逐资源 Chain）。
    RewardDialog,
    /// `MysekaiResourceUtility.NoticeCollectItem`（蓝图/唱片臂）。
    NoticeCollect,
    /// `MysekaiSecretShopPresenter`（课金域，出范围）。
    SecretShop,
    /// `SketchUtility.ShowSketchResultDialog`（写生域）。
    Sketch,
}

impl GetResourceOpener {
    /// 开门者名（开门行用）。
    fn name(self) -> &'static str {
        match self {
            GetResourceOpener::RewardDialog => "UIUtility.ShowRewardDialog",
            GetResourceOpener::NoticeCollect => "MysekaiResourceUtility.NoticeCollectItem",
            GetResourceOpener::SecretShop => "MysekaiSecretShopPresenter",
            GetResourceOpener::Sketch => "SketchUtility.ShowSketchResultDialog",
        }
    }

    /// 弹法参数（开门行用；方法体直读的调用形）。
    fn call_shape(self) -> &'static str {
        match self {
            GetResourceOpener::RewardDialog => {
                "分组逐资源 ChainSubWindowDialog(null, null, true, 329, 4) + Setup().Forget()；\
                 一组 ≥6 条走竖版 CommonReward 不走此窗"
            }
            GetResourceOpener::NoticeCollect => {
                "ChainSubWindowDialog(null, null, true, 329, 4) + Play(回调)；messageBodyKey=null；\
                 重复获得转 GetMysekaiItem(1)/(2) 为 mysekai_item 条目"
            }
            GetResourceOpener::SecretShop => {
                "ShowSubWindowDialog(null, OnClosedGetResourceDialog, true, 329, 4)；\
                 messageBodyKey=MSG_RECEIVED_REWARD（type 40 臂不读它，死参）"
            }
            GetResourceOpener::Sketch => {
                "ShowSketchResultDialog(blueprintId, onClose)：await Setup；\
                 UserResource(blueprintId, 40, level=0, 1)；messageBodyKey 同上死参"
            }
        }
    }

    /// 门账（未建/出范围定谳；开门行末尾报）。
    fn gate_note(self) -> &'static str {
        match self {
            GetResourceOpener::RewardDialog => "调用方（通用奖励 API 域）未建，具名挂账",
            GetResourceOpener::NoticeCollect => "采集结算域未建，具名挂账",
            GetResourceOpener::SecretShop => "课金域出范围（范围通则），具名挂账",
            GetResourceOpener::Sketch => "写生域未建，具名挂账",
        }
    }
}

// ---------------------------------------------------------------------------
// mock 面板（服务端态，具名「mock 值」；照菜单对话框面板的形）
// ---------------------------------------------------------------------------

/// 获得子窗的具名 mock 服务端状态（开门者 + 资源队列）。真值住服务端响应
/// （四开门者的载荷）；本仓读不到 ⇒ 面板下发。默认三件走链形（一窗一条，
/// 关一格出队开下一格）演示 ChainDialogPlayer；单开门者（商店/写生）给
/// 单条目即单窗形。
#[derive(Resource)]
pub(crate) struct GetResourceMock {
    opener: GetResourceOpener,
    entries: Vec<ResourceEntry>,
}

impl Default for GetResourceMock {
    fn default() -> Self {
        let opener = match std::env::var("MOLY_GET_RESOURCE_MOCK_OPENER")
            .unwrap_or_default()
            .trim()
        {
            "" => GetResourceOpener::NoticeCollect,
            "reward" => GetResourceOpener::RewardDialog,
            "collect" => GetResourceOpener::NoticeCollect,
            "secret_shop" => GetResourceOpener::SecretShop,
            "sketch" => GetResourceOpener::Sketch,
            raw => {
                warn!(
                    "[get_resource] mock 面板：MOLY_GET_RESOURCE_MOCK_OPENER={raw:?} 不在 \
                     reward|collect|secret_shop|sketch 里，回默认 collect"
                );
                GetResourceOpener::NoticeCollect
            }
        };
        let entries = match std::env::var("MOLY_GET_RESOURCE_MOCK_ENTRIES") {
            Ok(spec) => {
                let parsed = parse_entries(&spec);
                if parsed.is_empty() {
                    warn!("[get_resource] mock 面板：条目全废，回默认三件");
                    default_entries()
                } else {
                    parsed
                }
            }
            Err(_) => default_entries(),
        };
        GetResourceMock { opener, entries }
    }
}

/// 默认三件：三条 mysekai 臂各一条（蓝图/道具/唱片），走链形。
fn default_entries() -> Vec<ResourceEntry> {
    parse_entries("blueprint:101:1:1:圆木长凳,item:2:1:3,record:5:1:1:无名曲")
}

/// 条目语法解析（见模块头）。非法条目响亮告警跳过（与菜单面板 env 同形）。
fn parse_entries(spec: &str) -> Vec<ResourceEntry> {
    let mut entries = Vec::new();
    for raw in spec.split(',') {
        match parse_entry(raw.trim()) {
            Some(entry) => entries.push(entry),
            None => warn!("[get_resource] mock 面板：条目 {raw:?} 语法不符，跳过"),
        }
    }
    entries
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn parse_entry(raw: &str) -> Option<ResourceEntry> {
    let fields: Vec<&str> = raw.split(':').collect();
    let bad = || None;
    match fields.first().copied() {
        Some("blueprint") => {
            // blueprint:id:level:qty:素材名 → 名称 = Format(FORMAT_MYSEKAI_BLUEPRINT_NAME)。
            if fields.len() != 5 {
                return bad();
            }
            let name = format!("{}的设计图", fields[4].trim());
            finish_entry(ResourceKind::MysekaiBlueprint, &fields[1..4], name, None)
        }
        Some("record") => {
            // record:id:level:qty:曲名 → 名称 = Format(WORD_FORMAT_RECORD_NAME)。
            if fields.len() != 5 {
                return bad();
            }
            let name = format!("《{}》的唱片", fields[4].trim());
            finish_entry(ResourceKind::MysekaiMusicRecord, &fields[1..4], name, None)
        }
        Some("item") => {
            // item:id:level:qty[:文案]——名称查 5 行表；ItemType ∉ {1,2} 需要
            // 调用方文案（messageBodyKey 的查表值）。
            if fields.len() != 4 && fields.len() != 5 {
                return bad();
            }
            let id = parse_u32(fields[1])?;
            let (_, item_type, table_name) = *MYSEKAI_ITEMS.iter().find(|(row, _, _)| *row == id)?;
            let message = (fields.len() == 5).then(|| fields[4].trim().to_owned());
            if !matches!(item_type, 1 | 2) && message.is_none() {
                warn!(
                    "[get_resource] mock 面板：道具 id={} 的 ItemType={} 需要 messageBodyKey \
                     文案（第 5 段），条目不完整，跳过",
                    id, item_type
                );
                return None;
            }
            finish_entry(ResourceKind::MysekaiItem, &fields[1..4], table_name.to_owned(), message)
        }
        Some("other") => {
            // other:id:level:qty:名:文案——名字链 master 表不在提取产物，
            // 两样都 mock 直供。
            if fields.len() != 6 {
                return bad();
            }
            let name = fields[4].trim().to_owned();
            let message = fields[5].trim().to_owned();
            finish_entry(ResourceKind::Other, &fields[1..4], name, Some(message))
        }
        _ => None,
    }
}

/// 三段数值（id/level/qty）装进条目；数值非法整条拒。
fn finish_entry(
    kind: ResourceKind,
    nums: &[&str],
    name: String,
    message: Option<String>,
) -> Option<ResourceEntry> {
    let (id, level, qty) = (parse_u32(nums[0]), parse_u32(nums[1]), parse_u32(nums[2]));
    Some(ResourceEntry {
        kind,
        resource_id: id?,
        resource_level: level?,
        quantity: qty?,
        name,
        message,
    })
}

// ---------------------------------------------------------------------------
// 铺装文案闭包（外壳字符集的第七个消费者）
// ---------------------------------------------------------------------------

/// 全部渲染文案的字符闭包：固定文案 + 真值表名 + 默认条目名 + 数字位。
/// 动态值只有条目名（mock 供给）与数字；默认集装载期可枚举全量，环境
/// 变量覆写的自定义名缺字形在铺字点 fail-closed。
pub(crate) const FIXED_TEXTS: &[&str] = &[
    "关闭",
    "×0123456789",
    // 蓝图臂文案（Format MSG_GET_BLUEPRINT 的骨架 + 默认名）。
    "获得了“”。",
    "圆木长凳的设计图",
    // 道具臂 ItemType==1（Format MSG_GET_SURPLUS_BLUEPRINT，富文本跨段
    // 色标签已剥——铺字器单色，跨段色具名挂账）。
    "因为已获得过该设计图，作为代替，获得了“”。",
    // 道具臂 ItemType==2（固定文案，名字写死在文案里）。
    "因为已获得过该唱片，作为代替，获得了“多余的唱片”。",
    // 唱片臂（固定 MSG_GET_RECORD）。
    "获得了唱片。",
    // default 臂与道具臂调用方文案的默认（MSG_RECEIVED_REWARD 真值）。
    "获得奖励。",
    // 道具 5 行表的真值名。
    "空白设计图",
    "多余设计图",
    "多余唱片",
    "胶卷",
    "设计图碎片",
    // 唱片臂默认名（Format WORD_FORMAT_RECORD_NAME 的骨架 + 默认曲名）。
    "《无名曲》的唱片",
];

// ---------------------------------------------------------------------------
// 实体与资源
// ---------------------------------------------------------------------------

/// 获得子窗总根（Dialog 槽：order-2 覆盖相机，件 z ≥ 20；开 = 外壳对话框
/// 态的 get_resource_open 位）。
#[derive(Component)]
pub(crate) struct GetResourceRoot;

/// 获得窗口的点击目标，矩形取当前 UiPrefabView 的源布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GetResourcePane {
    Panel,
    Thumb,
    Close,
}

/// 铺装闩（视图一次铺成后置）。
#[derive(Resource, Default)]
pub(crate) struct GetResourceSpawned;

/// 链播放态（真源 `ChainDialogPlayer` 的同形弱化：关一格出队开下一格）。
#[derive(Resource, Default)]
pub(crate) struct GetResourcePlayer {
    /// 当前播到的条目下标。
    pub(crate) cursor: usize,
    /// 名称气球开着（点按缩略图位切换；换格重置）。
    pub(crate) balloon_shown: bool,
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

pub(crate) fn init(mut commands: Commands) {
    commands.init_resource::<GetResourceMock>();
    commands.init_resource::<GetResourcePlayer>();
}

// ---------------------------------------------------------------------------
// Update：源预制体到齐后挂视图
// ---------------------------------------------------------------------------

/// 等源布局就绪后一次挂 GetResource 视图，条目状态由 place 写入。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<crate::ui_layout::UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<GetResourceSpawned>>,
) {
    if spawned.is_some() || !layouts.ready("GetResource", &server) { return; }
    commands.spawn((GetResourceRoot, Visibility::Hidden, Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER), crate::ui_layout::UiPrefabView::new("GetResource", SITEMAP_LAYER)));
    commands.insert_resource(GetResourceSpawned);
}

// ---------------------------------------------------------------------------
// Update：摆位与开关沿
// ---------------------------------------------------------------------------

/// 开门沿（真源弹法逐句同形日志）：开门者名 + 弹法参数 + 四门账 + 队列
/// 与逐条目铺装行。
fn on_open(mock: &GetResourceMock) {
    info!(
        "[get_resource] 开门：{} → {} —— 本门：{}",
        mock.opener.name(),
        mock.opener.call_shape(),
        mock.opener.gate_note()
    );
    for opener in [
        GetResourceOpener::RewardDialog,
        GetResourceOpener::NoticeCollect,
        GetResourceOpener::SecretShop,
        GetResourceOpener::Sketch,
    ] {
        info!("[get_resource]   开门者门账：{} —— {}", opener.name(), opener.gate_note());
    }
    info!(
        "[get_resource] 队列 {} 条（ChainDialogPlayer 同形：关一格出队开下一格，播完走完成沿）",
        mock.entries.len()
    );
    for (index, entry) in mock.entries.iter().enumerate() {
        info!(
            "[get_resource]   条目 {}：{} id={} level={} qty={} →「{}」（{}臂）",
            index + 1,
            entry.kind.wire_name(),
            entry.resource_id,
            entry.resource_level,
            entry.quantity,
            entry.name,
            entry.arm_name()
        );
    }
}

/// 单格 Setup 沿（四臂分派的逐臂日志 + 当前条目铺装行）。
fn on_setup_entry(mock: &GetResourceMock, cursor: usize) {
    let Some(entry) = mock.entries.get(cursor) else {
        return;
    };
    let arm_detail = match entry.kind {
        ResourceKind::MysekaiBlueprint => {
            "不走缩略图（换蓝图视图）→ Format(MSG_GET_BLUEPRINT, GetResourceName) → await 蓝图视图 \
             （名称气球钮 + 图标 + 3D 预览；3D 预览未接，具名挂账）"
        }
        ResourceKind::MysekaiItem => {
            "缩略图 → MysekaiItemModel(id).MysekaiItemType 三分派（==2 固定文案 · ==1 Format · \
             其余 Wording.Get(messageBodyKey)）"
        }
        ResourceKind::MysekaiMusicRecord => {
            "缩略图（尺寸三角 200×200）→ 固定 MSG_GET_RECORD"
        }
        ResourceKind::Other => "缩略图 → Wording.Get(messageBodyKey)（调用方 key）",
    };
    info!(
        "[get_resource] Setup·{}臂（{}）：{}",
        entry.arm_name(),
        entry.kind.wire_name(),
        arm_detail
    );
    info!(
        "[get_resource] 条目铺装 {}/{}：名称「{}」· 徽记 ×{} · messageBody=「{}」· resourceLevel={}",
        cursor + 1,
        mock.entries.len(),
        entry.name,
        entry.quantity,
        entry.message_body(),
        entry.resource_level
    );
}

/// 换格沿（ChainDialogPlayer 出队同形）。
fn on_chain_advance(mock: &GetResourceMock, cursor: usize) {
    info!(
        "[get_resource] 关一格 → 出队开下一格（EnqueueDialogQueue 的 OnClose→Dequeue 同形）"
    );
    on_setup_entry(mock, cursor);
}

/// 关窗沿（CloseProcess 的同形日志；队列播完附完成沿）。
fn on_close_edge(mock: &GetResourceMock, cursor: usize) {
    info!(
        "[get_resource] 关窗：Close → CloseProcess = HideAsync（收场动画未接，当帧收尾）· \
         PlaySEOneShot(SE_SUBWINDOW_CLOSE)（SE 域未接，具名挂账）· Dispose 资产包（本仓不装，无对应物）"
    );
    if cursor + 1 >= mock.entries.len() {
        info!(
            "[get_resource] 队列播完 → ChainDialogPlayer.Play 的 onFinish（链完成沿）——开门者回调：{}",
            mock.opener.name()
        );
    }
}

/// Update：摆位与逐帧状态。每帧——
/// 1. 开关沿（开门沿 + 单格 Setup 沿；换格沿；关窗沿 + 完成沿）；
/// 2. 根可见性 = 外壳对话框态的 get_resource_open 位；根缩放 = canvas 缩放；
/// 3. 正文逐臂组装，源缩略图/预览节点按臂显隐；名称气球读当前条目名。
#[allow(clippy::type_complexity)]
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    dialog: Res<ShellDialogState>, mock: Res<GetResourceMock>,
    mut player: ResMut<GetResourcePlayer>,
    mut roots: Query<(&mut Visibility, &mut Transform, &mut crate::ui_layout::UiPrefabView), With<GetResourceRoot>>,
    mut was_open: Local<bool>, mut last_cursor: Local<usize>,
) {
    let open = dialog.get_resource_open;
    match (*was_open, open) {
        (false, true) => { on_open(&mock); on_setup_entry(&mock, player.cursor); *last_cursor = player.cursor; }
        (true, true) if player.cursor != *last_cursor => {
            on_chain_advance(&mock, player.cursor); *last_cursor = player.cursor; player.balloon_shown = false;
        }
        (true, false) => on_close_edge(&mock, player.cursor),
        _ => {}
    }
    *was_open = open;
    let Ok(window) = windows.single() else { return; };
    for (mut visible, mut transform, mut view) in &mut roots {
        *visible = if open { Visibility::Inherited } else { Visibility::Hidden };
        transform.scale = Vec3::splat(canvas_scale(window.width(), window.height()));
        let entry = mock.entries.get(player.cursor);
        view.set_text("Content/BodyText", entry.map(|e| e.message_body()).unwrap_or_default());
        view.set_visible("Content/Object3DPreview", entry.is_some_and(|e| e.kind == ResourceKind::MysekaiBlueprint));
        view.set_visible("Content/ThunbmainRoot", entry.is_some_and(|e| e.kind != ResourceKind::MysekaiBlueprint));
        view.set_visible("Object3DPreview/UIPartsLoadingCircleTexture", false);
        view.set_visible("Object3DPreview/UIPartsCommonBalloon (1)", player.balloon_shown);
        view.set_text("UIPartsCommonBalloon (1)/Content/CustomText", entry.map(|e|e.name.clone()).unwrap_or_default());
    }
}

// ---------------------------------------------------------------------------
// Update：点按（模态先手）
// ---------------------------------------------------------------------------

/// 点按屏位（顶为原点）→ 参照画布坐标。
fn to_canvas(position: Vec2, width: f32, height: f32, scale: f32) -> Vec2 {
    Vec2::new(position.x - width / 2.0, height / 2.0 - position.y) / scale
}

fn hit_test(
    canvas: Vec2,
    which: GetResourcePane,
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
) -> bool {
    let path = match which {
        GetResourcePane::Thumb => "Content/ThunbmainRoot",
        GetResourcePane::Panel => "CenterSubWindowComponent/ContentRoot",
        GetResourcePane::Close => "CenterSubWindowComponent/CloseArea",
    };
    view.rect(layouts, path, size).is_some_and(|r| r.active && r.contains(canvas))
}

/// 关一格出队开下一格；队列播完收窗走完成沿（链完成沿在 [`place`] 的
/// 关窗沿里补）。
fn advance_or_finish(dialog: &mut ShellDialogState, player: &mut GetResourcePlayer, entries: &[ResourceEntry]) {
    if player.cursor + 1 < entries.len() {
        player.cursor += 1;
    } else {
        dialog.get_resource_open = false;
    }
}

/// 点按分派：子窗开着 ⇒ 模态（全部点按先过它，与菜单对话框同一条
/// Dialog 槽模态律；外壳的门读同一个 get_resource_open 位）。缩略图位
/// 点按切换名称气球；关闭钮与框外点按收窗（allowCloseExternal=true）；
/// 队列有余则换格不收窗。已知偏差与外壳确认框同款：动作按钮的点按消费
/// 先于本模块（既有具名挂账），本模块不动它的判定面。
pub(crate) fn click(
    layouts: Res<crate::ui_layout::UiLayouts>,
    views: Query<&crate::ui_layout::UiPrefabView, With<GetResourceRoot>>,
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut dialog: ResMut<ShellDialogState>,
    mock: Res<GetResourceMock>,
    mut player: ResMut<GetResourcePlayer>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() || !dialog.get_resource_open {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = canvas_scale(width, height);
    let size = Vec2::new(width, height) / scale;
    let Ok(view) = views.single() else { return; };
    let current_name = mock
        .entries
        .get(player.cursor)
        .map(|entry| entry.name.clone())
        .unwrap_or_default();
    for position in &taps {
        // 模态：这一下被子窗吃掉（世界射线与外壳件都拿不到）。
        consumed.0 = true;
        let canvas = to_canvas(*position, width, height, scale);
        if hit_test(canvas, GetResourcePane::Thumb, &layouts, view, size) {
            // 缩略图位点按 → 名称气球（真源 OnClick → CommonBalloon.Setup(
            // GetResourceName, 1, 1)；蓝图臂 _showsBalloonButton 同律）。
            player.balloon_shown = !player.balloon_shown;
            info!(
                "[get_resource] 缩略图位点按 → 名称气球{}（GetResourceName =「{}」；自动调宽未接，具名挂账）",
                if player.balloon_shown { "开" } else { "收" },
                current_name
            );
        } else if hit_test(canvas, GetResourcePane::Close, &layouts, view, size) {
            info!(
                "[get_resource] 关闭钮按下 → Close → CloseProcess 沿：HideAsync·SE_SUBWINDOW_CLOSE·\
                 Dispose（收场动画与 SE 域未接，具名挂账）"
            );
            advance_or_finish(&mut dialog, &mut player, &mock.entries);
        } else if !hit_test(canvas, GetResourcePane::Panel, &layouts, view, size) {
            info!("[get_resource] 框外点按收窗（模态；allowCloseExternal=true，Initialize 第三参原生核过）");
            advance_or_finish(&mut dialog, &mut player, &mock.entries);
        } else {
            info!("[get_resource] 面板空白处点按：不收框（框内非钮区无动作；收框沿只在钮与框外）");
        }
    }
}
