//! 情报层视图（真源 `ScreenLayerMysekaiInfo`，屏幕号 622）——本仓第一个
//! 带内容物的屏幕层视图：压层即有画面，层栈槽位驱动显隐。
//!
//! ## 层身份与入口
//!
//! 入口是既有动作按钮派发表的 `OpenMysekaiInfo` 槽（写
//! `LayerCommand::Push(MysekaiInfo)`——本模块不动派发表）。真源压层助手
//! 先问 `IsActiveScreen(622)`，已开不重复压——层栈的「同层重复压丢弃」
//! 同形。启动参数（bootData）真源是排名页模型，服务端用户态 ⇒ 面板下发
//! mock（见下）。
//!
//! ## 两页结构（页数 = 2，SetupPage 九步装配序定谳）
//!
//! - **页 0 排名页**（`MysekaiInfoRankPage.Setup`）：rank text, rank gauge,
//!   and on the JP client the home and my-room put-cost limits
//!   (`FixtureHomePutLimitPutCost` as the number, `FixtureMyRoomPutLimitPutCost`
//!   through `MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST`), then the photo button
//!   and `SetupPhotoAsync`. See the rank page section below.
//! - **页 1 设置页**（`MysekaiInfoOption1Page.Setup`）：四组单选档 +
//!   语音下载钮。
//! - 页、页签与各单选组的次序直接读预制体 PPtr 数组：页 0=排名页、
//!   页 1=设置页。页态跨开合保留（真源层实例常驻，序列化字段
//!   在 SetActive(false) 后仍在，重开淡入的是上次页）。
//! - 页签两枚（`CustomIndexToggleGroup` 单选组，回调
//!   `OnNoteTabSelectedIndexChanged` → `ChangePage`）；左右箭头钮（可见
//!   性 = `IsAbleToMovePrev/NextPage`，边界钳 0 / Length-1）。
//!
//! ## 四组档位（选项下标由序列化组定义，不由节点名称推测）
//!
//! | 组 | 枚举 | 值域 | 默认 |
//! |---|---|---|---|
//! | 访问许可 | `MysekaiRoomAcceptUserType` | all=0 · friend_only=1 · reject=2 · review=3 | 无客户端默认（服务端现值，null → 0）|
//! | 画质 | `MysekaiImageQualityType` | high=0 · normal=1 · low=2 | **normal** |
//! | 刷新率 | `MysekaiFpsQualityType` | high=0 · normal=1 | **high** |
//! | 变换通知 | `MysekaiConvertFxitureNotificationType` | on=0 · off=1 | **on** |
//!
//! 访问许可仅有三枚单选钮。review=3 是审核中的服务端状态：显示提示、
//! 选中 reject 的下标 2，并禁用整组三钮、显示源禁用遮罩；不是第四档。
//!
//! 默认值的承重句：`MysekaiOptionSettingData` 构造只写画质一格 = 1 ⇒
//! 默认画质 normal；刷新率与变换通知两格未写 ⇒ CLR 零初始化 = 0（high
//! / on）。访问许可现值真源读 `UserMysekaiVisitSetting`（服务端）。
//!
//! ## 切换律（CustomIndexToggleGroup）
//!
//! 单选；**转亮才回调**（点已选中的档无回调）；初值双写不带通知
//! （set_SelectedIndex + WithoutNotify 后才挂回调 ⇒ 初值不触发）；界外
//! 下标 LogError 拒绝——本仓档位枚举闭集，界外分支由构造消掉。
//!
//! ## 切换即施加律
//!
//! - **画质**（`OnChangeImageQualityToggle`）：写档位**并立即**
//!   `SetImageQuality`——high → `SetTargetDpi(299)` + FXAA 开 · normal →
//!   200 + 开 · low → 180 + 关；`SetTargetDpi` 把
//!   `clamp(目标DPI / 屏DPI, 0, 1)` 写进 RenderScale。FXAA 与目标 DPI 写入现
//!   GameSettings；场景相机按窗口缩放因子推得的屏 DPI 施加渲染比例（见
//!   `game_settings::source_render_scale`）。
//! - **刷新率**（`OnChangeFpsToggle`）：写档位并立即 `SetFpsQuality`——
//!   high → `Application.targetFrameRate = 60` · normal → 30；越界
//!   LogError；同值且非强制早退（幂等门）。**本仓对应物 = `WinitSettings`
//!   的 Reactive 档**：等待 = 1/60 · 1/30，三个反应位全关（只按节拍 tick，
//!   事件缓冲到下一拍）——运行时改档当帧生效。这是本层接线的真行为。浏览器
//!   改为动画帧节拍：到点的动画帧里更新，未到点的帧跳过（`game_settings`）。
//! - **变换通知**（`OnChangeConvertFixtureNotificationTypeToggle`）：只写
//!   档位，无即时副作用。
//! - **访问许可**（`OnChangeVisitSettingToggle`）：非审核状态下只写档位；上报在出场链
//!   （`OnExited` 里 `ChangeVisitType`，服务端域 ⇒ 面板下发 mock）。
//!
//! **启动施加律**：真源场地进场（`SceneMysekai.Start`）即按存量档全量施加
//! （forceUpdate=true 两档合施）。存量档 = 本地设置档的
//! `MysekaiOptionSettingData`（`ApplicationLocalSettings` 同名成员，三键
//! `MysekaiImageQualityType` / `MysekaiFpsQualityType` /
//! `MysekaiConvertFxitureNotificationType`，枚举存整数值）；无档取构造默认
//! （画质 normal · 刷新率 high ⇒ 60）。`init` 在 Startup 施加，首个场景帧
//! 再施加一次刷新率（产品面板的启动装载在 `init` 之后，会写入它自己存的帧率）。
//!
//! ## 页切换律（ChangePage / MoveXxxPage）
//!
//! 点页签或箭头：先写 `_currentPageIndex` → `RefreshTab`（页签组不带通知
//! 写 + 逐页签 `Setup`）→ `RefreshArrowButton` → 重建取消令牌 → 按新旧
//! 差选方向。页过渡 = 旧页方向性淡出 + 两枚页签粒子 + 新页方向性淡入，
//! 唯一等待 = `Delay(interval * 2)`（interval = 粒子时长 / 10）。**粒子
//! 绘制路径未并、两段串行淡变与节拍未提取** ⇒ 本仓当帧落位（与层视图
//! 无动画的既有简化同款），具名挂账。翻页手势（`OnFlick` 左=下一页 ·
//! 右=上一页）挂账：手势层没有 flick 分类器，本模块不自建第二套分类。
//!
//! ## 生命周期（钩子次序照层栈梯，层侧内容在这里）
//!
//! `OnBoot`（bootData=排名模型 mock · 访问许可现值 mock 重读）→
//! `OnInitComponent`（三档读入 + 七 Setup：页签/排名详情钮/箭头钮/页四件
//! 已接，贴图/粒子/翻页手势三件挂账）→ `OnFinishStartAnimation`（全部页
//! Hide 后当前页 FadeIn）→ … → `OnExitStart`（当前页 FadeOut）→
//! `OnExited`（三档写回 SaveToStorage：只并入 `MysekaiOptionSettingData`
//! 一节，写后读回核对；访问许可 `ChangeVisitType` 上报——服务端域 ⇒ mock）。
//!
//! ## 服务端域与 mock 面板（照音频域音量面板的形：具名资源 + 默认 +
//! 环境变量覆写，非法值响亮告警回默认）
//!
//! - **排名页显示件**：`MysekaiRankModel` 读用户态（服务端）⇒ mock：
//!   等级 · 量表比率；the CN client's put-limit count and joint count are
//!   mock values too.
//! - **排名详情对话框**（`Show1ButtonDialog<MysekaiRankListDialog>`,
//!   dialog type 345, shown through the screen manager with the back key
//!   closing it; 允许框外关）：对话框体 = 主表 `MasterMysekaiRanks` 的逐等级
//!   解锁条件清单 + 用户态排名 ⇒ 连同走 mock，框内文案具名「面板下发」。
//! - **访问许可现值**：面板下发 mock（档名用真源枚举名）。
//! - **排名页的等级与等级量表**：`TryMoveScreenLayerMysekaiInfo` 以
//!   `new MysekaiRankModel()`（用户总经验，与菜单共读的同一份 mock，见
//!   [`crate::mysekai_rank::UserTotalExp`]）建 `MysekaiRankGaugeViewDataModel`
//!   作排名页模型；量表走菜单同一条 `UIPartsMysekaiRankGauge.Setup`。量表按
//!   文档里唯一的 `UIPartsMysekaiRankGauge` 组件取（区域布局上它的引用都已解码）。
//!
//! 环境变量：`MOLY_INFO_MOCK_ACCESS_PERMISSION`（all/friend_only/reject/
//! review）· `MOLY_INFO_MOCK_FIXTURE_PUT_LIMIT` ·
//! `MOLY_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT`。
//!
//! ## Rank page costs, photo and voice download
//!
//! - **Put-cost limits** (JP): `GetFixturePutLimitCost` asks
//!   `GetMasterMysekaiFixturePutLimitLevelFromMysekaiRankRelease(rank,
//!   category)` (read natively): the `externalId`s of the
//!   `mysekai_fixture_put_limit` rank releases at or below the rank; among
//!   the put-limit level rows of the site category whose id is one of them,
//!   the first with the highest level; its `putCostLimit`, else 0. Both
//!   tables are master data from the runtime root
//!   (`mysekai-rank-releases.json`, `mysekai-fixture-put-limit-levels.json`);
//!   a root without them leaves the two texts as the prefab has them, named
//!   once.
//! - The rank page binds through `MysekaiInfoRankPage`'s serialized
//!   references when the layout decodes them, else through the same nodes'
//!   prefab paths.
//! - **Photo**: the user's MySekai photo is server state, none here, so
//!   `SetupPhotoAsync` takes its null branch, `_photoFramePreview.Hide()`
//!   (its root CanvasGroup alpha 0). The photo selection it would open is not
//!   in this product, so the photo button stands disabled the way the
//!   tutorial branch disables it (`CustomButton.enabled = false`).
//! - **Voice download**: `SetVoiceDLButtonState(IsMysekaiVoice)`. The bulk
//!   download that sets the flag is a server feature; the runtime root
//!   carries the MySekai talk voice banks themselves, so the flag stands set:
//!   `WORD_DOWNLOADED`, the button disabled (its cover shown), and the
//!   download dialog is never reached.
//!
//! ## 我方选值与具名缺口（改这里之前先读）
//!
//! - 视图和点击共用源预制体布局：Info 主层、Common1 排名详情。动态档位和
//!   用户数据显示写入各自的 UiPrefabView。
//! - The back key: the screen manager gives it to the rank list dialog when
//!   it is open, else to the header back button (the screen pops there).
//! - 层开着时**本层吃掉全部点按**（全屏层阻断世界射线，真源
//!   blockRaycasts 同形）；世界输入让位由共享层态与指针归属处理。

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, LayerId, UiLayerStack,
};
use moly_assets::json::JsonAsset;

/// 页数（真源 SetupPage 装配序定谳：排名页 + 设置页）。
const PAGE_COUNT: usize = 2;

/// 情报层全部渲染文案的字符闭包（静态标签 + 数字位）——并入外壳字符集
/// （气泡层图集的同一并集成员，图集的第五个消费者）。动态值只有闭集
/// 档名 · 整数 · 一位小数，无自由文本 ⇒ 装载期可枚举全量。
pub(crate) const FIXED_TEXTS: &[&str] = &[
    "情报",
    "关闭",
    "排名信息",
    "设定",
    "等级 0",
    "量表",
    "家具摆放上限 0",
    "连接家具上限 0",
    "相片",
    "排名详情",
    "访问许可",
    "画质",
    "刷新率",
    "变换通知",
    "全员",
    "好友限定",
    "拒绝",
    "审核",
    "高",
    "通常",
    "低",
    "开",
    "关",
    "语音下载",
    "上一页",
    "下一页",
    "0123456789.MB?（）",
];

// ---------------------------------------------------------------------------
// 四组档位（真源枚举逐值核过；下标恒等映射）
// ---------------------------------------------------------------------------

/// 访问许可档（真源 `MysekaiRoomAcceptUserType`）。值是服务端持久化
/// （真源读 `UserMysekaiVisitSetting`，null → 0）⇒ 面板下发 mock。review
/// 是锁定前三枚选项的状态，不是可选的第四枚按钮。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessPermission {
    All = 0,
    FriendOnly = 1,
    Reject = 2,
    Review = 3,
}

impl AccessPermission {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::All),
            1 => Some(Self::FriendOnly),
            2 => Some(Self::Reject),
            3 => Some(Self::Review),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::All => "全员",
            Self::FriendOnly => "好友限定",
            Self::Reject => "拒绝",
            Self::Review => "审核",
        }
    }
    /// mock 面板档名（真源枚举名）。
    fn true_name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::FriendOnly => "friend_only",
            Self::Reject => "reject",
            Self::Review => "review",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "all" => Some(Self::All),
            "friend_only" => Some(Self::FriendOnly),
            "reject" => Some(Self::Reject),
            "review" => Some(Self::Review),
            _ => None,
        }
    }
}

/// 画质档（真源 `MysekaiImageQualityType`）。默认 normal（构造点写真源
/// 证据见模块头）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageQuality {
    High = 0,
    Normal = 1,
    Low = 2,
}

impl ImageQuality {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::High),
            1 => Some(Self::Normal),
            2 => Some(Self::Low),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::High => "高",
            Self::Normal => "通常",
            Self::Low => "低",
        }
    }
    /// 真源 `SetImageQuality` 的两件真值：目标 DPI 与 FXAA 开关。
    fn target_dpi_and_fxaa(self) -> (u32, bool) {
        match self {
            Self::High => (299, true),
            Self::Normal => (200, true),
            Self::Low => (180, false),
        }
    }
}

/// 刷新率档（真源 `MysekaiFpsQualityType`）。默认 high（零初始化）。
/// 真值：high → 60 · normal → 30（`Application.targetFrameRate`）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FpsQuality {
    High = 0,
    Normal = 1,
}

impl FpsQuality {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::High),
            1 => Some(Self::Normal),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::High => "高",
            Self::Normal => "通常",
        }
    }
    fn target_frame_rate(self) -> u32 {
        match self {
            Self::High => 60,
            Self::Normal => 30,
        }
    }
}

/// 变换通知档（真源 `MysekaiConvertFxitureNotificationType`，源侧类名即
/// 带这个拼写）。默认 on（零初始化）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvertNotification {
    On = 0,
    Off = 1,
}

impl ConvertNotification {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::On),
            1 => Some(Self::Off),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::On => "开",
            Self::Off => "关",
        }
    }
}

// ---------------------------------------------------------------------------
// 资源
// ---------------------------------------------------------------------------

/// 情报层会话档位（真源 `MysekaiOptionSettingData` 的本仓对应物：会话内
/// 常驻，重开层读到的就是改过的值；出场链 OnExited 写回本地设置档）。访问
/// 许可一格在每次进层时从 mock 面板重读（真源 OnBoot 读服务端现值）。
#[derive(Resource)]
pub(crate) struct InfoSettings {
    access: AccessPermission,
    image_quality: ImageQuality,
    fps: FpsQuality,
    convert: ConvertNotification,
}

impl Default for InfoSettings {
    fn default() -> Self {
        InfoSettings {
            // 占位：进层即从 mock 面板重读（OnBoot 律）。
            access: AccessPermission::All,
            image_quality: ImageQuality::Normal,
            fps: FpsQuality::High,
            convert: ConvertNotification::On,
        }
    }
}

/// `ApplicationLocalSettings.MysekaiOptionSettingData`: its key in the local
/// settings document (the audio half of the same class writes `LiveVolume` and
/// `SystemVolume` beside it) and the class's three MessagePack keys. The enums
/// are stored as their integer values; the convert key keeps the source's
/// `Fxiture` spelling.
const OPTION_SECTION: &str = "MysekaiOptionSettingData";
const KEY_IMAGE_QUALITY: &str = "MysekaiImageQualityType";
const KEY_FPS_QUALITY: &str = "MysekaiFpsQualityType";
const KEY_CONVERT: &str = "MysekaiConvertFxitureNotificationType";

impl InfoSettings {
    /// `ApplicationLocalSettings.LoadFromStorage().MysekaiOptionSettingData`.
    /// A document without the section is a first run: `SetDefaultValueIfNeeded`
    /// puts a new object there (image quality normal, the other two 0). A key
    /// the stored object lacks keeps that constructor default, as MessagePack
    /// leaves an absent member at its initializer. A value outside the enum is
    /// not written by this client; it is named and the default kept.
    fn from_document(document: &Value) -> (Self, &'static str) {
        let mut settings = InfoSettings::default();
        let section = &document[OPTION_SECTION];
        if section.is_null() {
            return (settings, "无存档（首跑：构造默认）");
        }
        let Some(object) = section.as_object() else {
            warn!("[info] 本地设置档 {OPTION_SECTION} 不是对象（{section}），三档取构造默认");
            return (settings, "存档损坏（构造默认）");
        };
        let read = |key: &str| -> Option<usize> {
            let raw = object.get(key)?;
            let value = raw.as_u64().and_then(|v| usize::try_from(v).ok());
            if value.is_none() {
                warn!("[info] 本地设置档 {OPTION_SECTION}.{key}={raw} 不是枚举整数值，该档取构造默认");
            }
            value
        };
        if let Some(index) = read(KEY_IMAGE_QUALITY) {
            match ImageQuality::from_index(index) {
                Some(value) => settings.image_quality = value,
                None => warn!("[info] 本地设置档 {KEY_IMAGE_QUALITY}={index} 不在 high/normal/low，取构造默认 normal"),
            }
        }
        if let Some(index) = read(KEY_FPS_QUALITY) {
            match FpsQuality::from_index(index) {
                Some(value) => settings.fps = value,
                None => warn!("[info] 本地设置档 {KEY_FPS_QUALITY}={index} 不在 high/normal，取构造默认 high"),
            }
        }
        if let Some(index) = read(KEY_CONVERT) {
            match ConvertNotification::from_index(index) {
                Some(value) => settings.convert = value,
                None => warn!("[info] 本地设置档 {KEY_CONVERT}={index} 不在 on/off，取构造默认 on"),
            }
        }
        (settings, "本地设置档")
    }

    /// The stored object, one integer per enum.
    fn section(&self) -> Value {
        serde_json::json!({
            KEY_IMAGE_QUALITY: self.image_quality.index(),
            KEY_FPS_QUALITY: self.fps.index(),
            KEY_CONVERT: self.convert.index(),
        })
    }
}

/// `OnExited`: `LoadFromStorage()`, put this layer's option object in it and
/// `SaveToStorage`. The store merges the one section into the fresh document,
/// so every other domain's fields stay as stored.
fn save_option_settings(settings: &InfoSettings) {
    let section = settings.section();
    match crate::settings_store::save_sections(&[(OPTION_SECTION, section.clone())]) {
        Ok(()) => {
            let stored = crate::settings_store::read_document()
                .map(|document| document[OPTION_SECTION].clone());
            match stored {
                Ok(stored) if stored == section => info!(
                    "[info] OnExited SaveToStorage：{OPTION_SECTION}={section} → {} 读回一致",
                    crate::settings_store::location()
                ),
                Ok(stored) => warn!(
                    "[info] OnExited SaveToStorage：写入 {section}，读回 {stored}——持久化层不一致"
                ),
                Err(error) => warn!("[info] OnExited SaveToStorage：写后读回失败：{error}"),
            }
        }
        Err(error) => warn!(
            "[info] OnExited SaveToStorage 失败：{error}——三档在本会话内仍是新值，重启回存盘值"
        ),
    }
}

/// 情报层 mock 面板（服务端域 ⇒ 面板下发；照音频域音量面板的形）。
/// 默认值全部是我方选值——真值在服务端/主表镜像里，本仓读不到。
#[derive(Resource)]
pub(crate) struct InfoMock {
    access_permission: AccessPermission,
    fixture_put_limit: u32,
    fixture_joint_put_limit: u32,
}

fn env_u32(name: &str, default: u32) -> u32 {
    match std::env::var(name) {
        Ok(raw) => match raw.trim().parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                warn!("[info] mock 面板：{name}={raw:?} 不是非负整数，回默认 {default}");
                default
            }
        },
        Err(_) => default,
    }
}

impl Default for InfoMock {
    fn default() -> Self {
        let access_permission = match std::env::var("MOLY_INFO_MOCK_ACCESS_PERMISSION") {
            Ok(raw) => match AccessPermission::from_name(raw.trim()) {
                Some(value) => value,
                None => {
                    warn!(
                        "[info] mock 面板：MOLY_INFO_MOCK_ACCESS_PERMISSION={raw:?} 不是档名 \
                         （all/friend_only/reject/review），回默认 all"
                    );
                    AccessPermission::All
                }
            },
            Err(_) => AccessPermission::All,
        };
        InfoMock {
            access_permission,
            fixture_put_limit: env_u32("MOLY_INFO_MOCK_FIXTURE_PUT_LIMIT", 20),
            fixture_joint_put_limit: env_u32("MOLY_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT", 10),
        }
    }
}

/// 页态（跨开合保留，见模块头）。
#[derive(Resource)]
pub(crate) struct InfoPageState {
    current: usize,
}

impl Default for InfoPageState {
    fn default() -> Self {
        InfoPageState { current: 0 }
    }
}

/// 层内对话框态（Dialog 槽：真源对话框不入屏幕层栈，开着时模态阻断
/// 层内点按）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfoDialog {
    /// The rank list (`MysekaiRankListDialog`, a one-button dialog, closes on
    /// a tap outside).
    RankList,
}

/// `Show1ButtonDialog<MysekaiRankListDialog>(DialogType.MysekaiRankListDialog)`.
const RANK_LIST_DIALOG: DialogType = DialogType(345);

#[derive(Resource, Default)]
pub(crate) struct InfoDialogState {
    open: Option<InfoDialog>,
    /// The screen manager's handle of the open dialog.
    id: Option<DialogId>,
}

/// 铺装闩（视图一次铺成后置）。
#[derive(Resource, Default)]
pub(crate) struct InfoSpawned;

// ---------------------------------------------------------------------------
// 业务件身份
// ---------------------------------------------------------------------------

/// 单选组身份（设置页四组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ToggleGroup {
    Access,
    ImageQuality,
    Fps,
    Convert,
}

/// 情报层业务件身份；点击矩形由当前 UiPrefabView 的源布局提供。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InfoItem {
    Tab(usize),
    RankLevelText,
    FixturePutText,
    FixtureJointText,
    Photo,
    RankInfo,
    Toggle(ToggleGroup, usize),
    VoiceDownload,
    ArrowPrev,
    ArrowNext,
    RankDialogClose,
}

impl InfoItem {
    /// 两种真实对话框 MessageBody 使用的动态文案。
    fn current_label(self, mock: &InfoMock, rank: i32) -> Option<String> {
        Some(match self {
            InfoItem::RankLevelText => format!("等级 {}", rank),
            InfoItem::FixturePutText => format!("家具摆放上限 {}", mock.fixture_put_limit),
            InfoItem::FixtureJointText => format!("连接家具上限 {}", mock.fixture_joint_put_limit),
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// 实体
// ---------------------------------------------------------------------------

/// 情报层总根（层栈槽位驱动显隐；子件全在 canvas 单位，根缩放 = canvas
/// 缩放）。
#[derive(Component)]
pub(crate) struct InfoRoot;

/// 层内对话框根（Dialog 槽语义：z ≥ 20，模态阻断层内点按）。
#[derive(Component)]
pub(crate) struct InfoDialogRoot {
    which: InfoDialog,
}

// ---------------------------------------------------------------------------
// Startup：资源与启动施加
// ---------------------------------------------------------------------------

/// Startup：资源落位 + 启动施加律（进场即按存量档全量施加：画质与刷新率
/// 两档读自本地设置档，无档取构造默认）。
pub(crate) fn init(mut commands: Commands, mut graphics: ResMut<crate::game_settings::GameSettings>, server: Res<AssetServer>) {
    let (settings, origin) = match crate::settings_store::read_document() {
        Ok(document) => InfoSettings::from_document(&document),
        Err(error) => {
            // PersistentDataUtility.Load failing gives a new object.
            warn!("[info] 本地设置档读取失败：{error}——三档取构造默认");
            (InfoSettings::default(), "读取失败（构造默认）")
        }
    };
    info!(
        "[info] 启动施加（进场全量施加律）：来源={origin} · 画质={} · 刷新率={}（帧率上限 {}fps）· 变换通知={}",
        settings.image_quality.label(),
        settings.fps.label(),
        settings.fps.target_frame_rate(),
        settings.convert.label()
    );
    apply_image_quality(&mut graphics, settings.image_quality);
    apply_fps(&mut graphics, settings.fps);
    commands.insert_resource(settings);
    commands.init_resource::<InfoMock>();
    commands.init_resource::<InfoPageState>();
    commands.init_resource::<InfoDialogState>();
    commands.insert_resource(PutLimitHandles([RANK_RELEASES, PUT_LIMIT_LEVELS].map(|path| server.load::<JsonAsset>(path))));
}

/// `SceneMysekai.Start` applies the stored pair with forceUpdate. The product
/// panel's startup load runs after [`init`] and seats its own saved frame limit
/// (a second copy of the same value the source keeps only here), so the first
/// scene frame applies the stored fps quality again.
fn apply_on_scene_start(graphics: &mut crate::game_settings::GameSettings, settings: &InfoSettings) {
    let rate = settings.fps.target_frame_rate() as u16;
    if graphics.graphics.frame_rate != rate {
        info!(
            "[info] SceneMysekai.Start SetFpsQuality(forceUpdate)：{} → {rate}fps（产品面板的帧率存档是同一值的第二份，以本档为准）",
            graphics.graphics.frame_rate
        );
    }
    apply_fps(graphics, settings.fps);
}

/// 画质施加：FXAA 与目标 DPI 写入现游戏设置，场景相机据此施加渲染比例。
fn apply_image_quality(graphics: &mut crate::game_settings::GameSettings, next: ImageQuality) {
    let (dpi, fxaa) = next.target_dpi_and_fxaa();
    graphics.graphics.fxaa = fxaa;
    graphics.graphics.image_quality = crate::game_settings::ImageQualityPair { target_dpi: dpi as u16, fxaa };
    info!("[info] SetImageQuality: target DPI {dpi}, FXAA {fxaa}");
}

fn apply_fps(graphics: &mut crate::game_settings::GameSettings, next: FpsQuality) {
    graphics.graphics.frame_rate = next.target_frame_rate() as u16;
}

// ---------------------------------------------------------------------------
// Update：源预制体到齐后挂视图
// ---------------------------------------------------------------------------

/// 等 Info 与两种通用对话框源布局就绪后一次挂视图，状态由 place 写入。
#[derive(Resource)]
pub(crate) struct InfoPresentation {
    bindings: InfoBindings,
    elapsed: f32,
    /// The put-cost limit tables, or why the root has none.
    put_limit: Result<PutLimitTables, String>,
}

struct InfoPageBinding {
    node: String,
    groups: [String; 2],
}

struct InfoTabBinding {
    control: String,
    off_image: String,
    on_image: String,
}

struct InfoToggleBinding {
    control: String,
    graphic: String,
    covers: Vec<String>,
}

/// Source references are resolved once. Painting and hit testing consume the
/// same ordered controls, leaving unused prefab template siblings untouched.
struct InfoBindings {
    pages: [InfoPageBinding; PAGE_COUNT],
    tabs: [InfoTabBinding; PAGE_COUNT],
    toggles: Vec<(ToggleGroup, Vec<InfoToggleBinding>)>,
    /// None where the option page class declares no review tip.
    review_tip: Option<String>,
    /// The rank page's put-limit count and joint count texts; None where the
    /// rank page class declares neither.
    put_limit_counts: Option<[String; 2]>,
    rank_page: RankPageBinding,
    voice_download: String,
    /// `_voiceDLText`.
    voice_download_text: String,
    /// The voice download button's covers (its `ShowCover` when disabled).
    voice_download_covers: Vec<String>,
    rank_info: String,
    arrow_prev: String,
    arrow_next: String,
}

/// The Info page class fields that differ between the clients whose layouts
/// this host reads, keyed by the layout's region tag.
///
/// Layouts without a tag are the shared root's CN extractions: there
/// `MysekaiInfoOption1Page` serializes `_reviewTip`, and `MysekaiInfoRankPage`
/// serializes `_fixturePutLimitCountText` and `_fixtureJointPutLimitCountText`.
/// The JP classes of the region root declare none of the three: the option
/// page has six serialized references without the review tip, and the rank
/// page serializes `_fixtureMyRoomPutLimitCostText` and
/// `_fixtureHomePutLimitCostText` in their place. On a
/// region whose class lacks an element, the element is not built; a declared
/// field the layout lacks is an error.
struct InfoFieldSet {
    review_tip: bool,
    put_limit_counts: bool,
    put_limit_costs: bool,
}

fn info_field_set(doc: &UiPrefab) -> InfoFieldSet {
    match doc.source.region.as_deref() {
        None => InfoFieldSet { review_tip: true, put_limit_counts: true, put_limit_costs: false },
        Some("jp") => InfoFieldSet { review_tip: false, put_limit_counts: false, put_limit_costs: true },
        Some(region) => panic!("Info: no declared page field set for region {region}"),
    }
}

pub(crate) fn referenced_component<'a>(
    doc: &'a UiPrefab,
    reference: &Value,
    class: &str,
) -> (String, &'a UiComponent) {
    let reference = reference.as_array().expect("Info serialized component reference");
    assert_eq!(reference[0].as_i64(), Some(0), "Info reference must be local");
    let id = reference[1].as_i64().expect("Info referenced component identity");
    let key = format!("@{id}");
    let node = &doc.nodes[doc.find(&key).expect("Info referenced component node")];
    let component = node.components.iter()
        .find(|c| c.path_id == id && c.class == class)
        .expect("Info reference has the declared component type");
    (key, component)
}

impl InfoBindings {
    fn from_prefab(doc: &UiPrefab) -> Self {
        let root = doc.nodes[0].components.iter()
            .find(|c| c.class == "Sekai.Mysekai.ScreenLayerMysekaiInfo")
            .expect("Info root bindings");
        let (_, option) = referenced_component(doc, &root.fields["_option1Page"],
            "Sekai.Mysekai.MysekaiInfoOption1Page");
        let pages = root.fields["_mysekaiInfoPages"].as_array()
            .expect("Info page references").iter().map(|reference| {
                let (node, page) = referenced_component(doc, reference,
                    "Sekai.Mysekai.ScreenLayerMysekaiInfoPage");
                let groups = ["_leftCanvasGroups", "_rightCanvasGroups"].map(|field| {
                    referenced_component(doc, &page.fields[field], "UnityEngine.CanvasGroup").0
                });
                InfoPageBinding { node, groups }
            }).collect::<Vec<_>>().try_into()
            .unwrap_or_else(|_| panic!("Info source must provide its two page bindings"));
        let (_, tab_group) = referenced_component(doc, &root.fields["_noteTabGroup"],
            "Sekai.UI.CustomIndexToggleGroup");
        let tab_controls = tab_group.fields["indexToggles"].as_array()
            .expect("Info tab toggle references");
        let tabs = root.fields["_infoTabs"].as_array().expect("Info tab references")
            .iter().enumerate().map(|(index, reference)| {
                let (_, tab) = referenced_component(doc, reference,
                    "Sekai.Mysekai.ScreenLayerMysekaiInfoTab");
                assert_eq!(tab_controls.get(index), Some(&tab.fields["_toggle"]),
                    "Info tab setup order must match the source toggle group");
                InfoTabBinding {
                    control: referenced_component(doc, &tab.fields["_toggle"], "Sekai.UI.CustomToggle").0,
                    off_image: referenced_component(doc, &tab.fields["_offImage"], "Sekai.UI.CustomImage").0,
                    on_image: referenced_component(doc, &tab.fields["_onImage"], "Sekai.UI.CustomImage").0,
                }
            }).collect::<Vec<_>>().try_into()
            .unwrap_or_else(|_| panic!("Info source must provide its two tab bindings"));
        let toggles = [
            (ToggleGroup::Access, "_mysekaiVisitSettingToggleGroup"),
            (ToggleGroup::ImageQuality, "_mysekaiImageQualityToggleGroup"),
            (ToggleGroup::Fps, "_mysekaiFPSSettingToggleGroup"),
            (ToggleGroup::Convert, "_mysekaiConvertFixtureNotificationSettingToggleGroup"),
        ].into_iter().map(|(group, field)| {
            let (_, source_group) = referenced_component(doc, &option.fields[field],
                "Sekai.UI.CustomIndexToggleGroup");
            let controls = source_group.fields["indexToggles"].as_array()
                .expect("Info option toggle references").iter().map(|reference| {
                    let (control, toggle) = referenced_component(doc, reference, "Sekai.UI.CustomToggle");
                    let graphic = referenced_component(doc, &toggle.fields["graphic"], "Sekai.UI.CustomImage").0;
                    // Captions retain the source CustomTextMesh wording and do
                    // not get replaced by an inferred label for this index.
                    referenced_component(doc, &toggle.fields["_captionText"], "Sekai.UI.CustomTextMesh");
                    let mut covers = Vec::new();
                    let cover = &toggle.fields["coverImage"];
                    if cover[1].as_i64().is_some_and(|id| id != 0) {
                        covers.push(referenced_component(doc, cover, "Sekai.UI.CustomImage").0);
                    }
                    for cover in toggle.fields["optionalCoverImages"].as_array()
                        .expect("Info optional cover references") {
                        covers.push(referenced_component(doc, cover, "Sekai.UI.CustomImage").0);
                    }
                    InfoToggleBinding { control, graphic, covers }
                }).collect();
            (group, controls)
        }).collect();
        let declared = info_field_set(doc);
        let review_tip = if declared.review_tip {
            Some(referenced_component(doc, &option.fields["_reviewTip"], "Sekai.UI.CustomTextMesh").0)
        } else {
            assert!(option.fields.get("_reviewTip").is_none(),
                "Info: this region's option page declares no _reviewTip, but the layout carries one");
            None
        };
        // The rank page class is not decoded, so its two serialized text
        // references are reached through their nodes' paths.
        let put_limit_counts = declared.put_limit_counts.then(|| {
            ["PutLimitFixtureCount/Value", "PutLimitFixtureJointCount/Value"].map(|path| {
                doc.find(path).unwrap_or_else(|e| panic!("Info rank page put-limit text: {e}"));
                path.to_owned()
            })
        });
        let (voice_download, voice_button) = referenced_component(doc, &option.fields["_voiceDLButton"], "Sekai.UI.CustomButton");
        let rank_page = rank_page_binding(doc, &declared);
        Self {
            pages,
            tabs,
            toggles,
            review_tip,
            put_limit_counts,
            rank_page,
            voice_download,
            voice_download_text: referenced_component(doc, &option.fields["_voiceDLText"], "Sekai.UI.CustomTextMesh").0,
            voice_download_covers: button_covers(doc, voice_button),
            rank_info: referenced_component(doc, &root.fields["_rankInfoButton"], "Sekai.UI.CustomButton").0,
            arrow_prev: referenced_component(doc, &root.fields["_leftArrowButton"], "Sekai.UI.CustomButton").0,
            arrow_next: referenced_component(doc, &root.fields["_rightArrowButton"], "Sekai.UI.CustomButton").0,
        }
    }

    fn toggle(&self, group: ToggleGroup, index: usize) -> Option<&InfoToggleBinding> {
        self.toggles.iter().find(|(candidate, _)| *candidate == group)?.1.get(index)
    }
}

/// `CustomButton.ShowCover`'s graphics (the cover and the optional covers),
/// shown only when the button greys out.
fn button_covers(doc: &UiPrefab, button: &UiComponent) -> Vec<String> {
    if button.fields["disableActionType"].as_i64() != Some(1) {
        return Vec::new();
    }
    let mut covers = Vec::new();
    let cover = &button.fields["coverImage"];
    if cover[1].as_i64().is_some_and(|id| id != 0) {
        covers.push(format!("@{}", cover[1]));
    }
    for cover in button.fields["optionalCoverImages"].as_array().expect("Info optional cover references") {
        covers.push(format!("@{}", cover[1]));
    }
    for cover in &covers {
        doc.find(cover).unwrap_or_else(|e| panic!("Info button cover: {e}"));
    }
    covers
}

/// The rank page's serialized references (`MysekaiInfoRankPage`). A layout
/// that does not decode the class reaches the same nodes by their prefab
/// paths (a decoded layout places each reference on exactly these).
struct RankPageBinding {
    rank_text: String,
    /// `_fixtureHomePutLimitCostText`, `_fixtureMyRoomPutLimitCostText`; None
    /// where the client's class declares neither.
    put_limit_costs: Option<[String; 2]>,
    /// `_photoFramePreview._rootCanvasGroup`: the CanvasGroup on the
    /// preview's own node (the preview class is not decoded).
    photo_preview: Option<String>,
}

fn rank_page_binding(doc: &UiPrefab, declared: &InfoFieldSet) -> RankPageBinding {
    let decoded = doc.nodes.iter().flat_map(|node| node.components.iter())
        .find(|c| c.class == "Sekai.Mysekai.MysekaiInfoRankPage" && c.fields.get("_mysekaiRankText").is_some());
    let text = |field: &str, path: &str| -> String {
        match decoded {
            Some(page) => referenced_component(doc, &page.fields[field], "Sekai.UI.CustomTextMesh").0,
            None => {
                doc.find(path).unwrap_or_else(|e| panic!("Info rank page {field}: {e}"));
                path.to_owned()
            }
        }
    };
    let rank_text = text("_mysekaiRankText", "RankPage/Right/Rank/CustomTextMesh (2)");
    let put_limit_costs = declared.put_limit_costs.then(|| [
        text("_fixtureHomePutLimitCostText", "RankPage/Right/HomePutLimitCost/Value"),
        text("_fixtureMyRoomPutLimitCostText", "RankPage/Right/MyRoomPutLimitCost/Value"),
    ]);
    let preview_node = match decoded {
        Some(page) => {
            let reference = &page.fields["_photoFramePreview"];
            Some(doc.find(&format!("@{}", reference[1])).expect("Info photo frame preview node"))
        }
        None => doc.nodes.iter().position(|node| node.components.iter().any(|c| c.class.ends_with(".UIPartsPhotoFramePreview"))),
    };
    let photo_preview = preview_node.and_then(|node| {
        doc.nodes[node].components.iter().find(|c| c.class == "UnityEngine.CanvasGroup").map(|c| format!("@{}", c.path_id))
    });
    RankPageBinding { rank_text, put_limit_costs, photo_preview }
}

/// The photo button (`_selectedPhotoButton`) stands disabled; see the module
/// header.
const PHOTO_BUTTON_ENABLED: bool = false;

/// `ApplicationLocalSettings.IsMysekaiVoice`; see the module header.
const MYSEKAI_VOICE_DOWNLOADED: bool = true;

/// The wording keys this screen writes (the atlas charset takes their glyphs
/// from here).
pub(crate) const WORDINGS: &[&str] = &["WORD_DOWNLOADED", "WORD_BULK_DOWNLOAD", MYROOM_COST_WORDING];

const MYROOM_COST_WORDING: &str = "MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST";

const RANK_RELEASES: &str = "moly://mysekai-rank-releases.json";
const PUT_LIMIT_LEVELS: &str = "moly://mysekai-fixture-put-limit-levels.json";

/// The two tables' load requests; removed once they resolve.
#[derive(Resource)]
pub(crate) struct PutLimitHandles([Handle<JsonAsset>; 2]);

/// `MysekaiSiteCategory` of a put-limit level row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteCategory {
    HousingHome,
    HousingRoom,
    Other,
}

struct PutLimitLevel {
    id: i64,
    level: i64,
    put_cost_limit: i64,
    category: SiteCategory,
}

/// The master rows `GetMasterMysekaiFixturePutLimitLevelFromMysekaiRankRelease`
/// reads, in master row order.
struct PutLimitTables {
    /// (mysekaiRank, externalId) of the `mysekai_fixture_put_limit` releases.
    releases: Vec<(i64, i64)>,
    levels: Vec<PutLimitLevel>,
}

/// The rows of a keyed master table document, in its `rowOrder`.
fn master_rows<'a>(value: &'a Value, table: &str) -> Result<Vec<&'a Value>, String> {
    if value["version"].as_i64() != Some(1) || value["semantics"]["table"].as_str() != Some(table) {
        return Err(format!("it is not version 1 of the {table} table"));
    }
    let entries = value["entries"].as_object().ok_or("no entries")?;
    value["rowOrder"].as_array().ok_or("no rowOrder")?.iter().map(|id| {
        entries.get(&id.to_string()).ok_or_else(|| format!("rowOrder id {id} has no entry"))
    }).collect()
}

impl PutLimitTables {
    fn parse(releases: &Value, levels: &Value) -> Result<Self, String> {
        let int = |row: &Value, field: &str| row[field].as_i64().ok_or_else(|| format!("{field} of {row} is not an int"));
        let text = |row: &Value, field: &str| row[field].as_str().map(str::to_owned).ok_or_else(|| format!("{field} of {row} is not a string"));
        let mut release_rows = Vec::new();
        for row in master_rows(releases, "mysekaiRankReleases")? {
            if text(row, "mysekaiRankRelaseType")? == "mysekai_fixture_put_limit" {
                release_rows.push((int(row, "mysekaiRank")?, int(row, "externalId")?));
            }
        }
        let mut level_rows = Vec::new();
        for row in master_rows(levels, "mysekaiFixturePutLimitLevels")? {
            let category = match text(row, "mysekaiSiteCategory")?.as_str() {
                "housing_home" => SiteCategory::HousingHome,
                "housing_room" => SiteCategory::HousingRoom,
                _ => SiteCategory::Other,
            };
            level_rows.push(PutLimitLevel { id: int(row, "id")?, level: int(row, "level")?, put_cost_limit: int(row, "putCostLimit")?, category });
        }
        Ok(PutLimitTables { releases: release_rows, levels: level_rows })
    }

    /// `GetFixturePutLimitCost(category)` at `rank`.
    fn cost(&self, rank: i64, category: SiteCategory) -> i64 {
        let ids: Vec<i64> = self.releases.iter().filter(|(release_rank, _)| *release_rank <= rank).map(|(_, id)| *id).collect();
        let mut best: Option<&PutLimitLevel> = None;
        for level in &self.levels {
            if level.category == category && ids.contains(&level.id) && best.is_none_or(|b| b.level < level.level) {
                best = Some(level);
            }
        }
        best.map_or(0, |level| level.put_cost_limit)
    }
}

/// The two tables once both requests settle; None while one still loads.
fn resolve_put_limit(server: &AssetServer, jsons: &Assets<JsonAsset>, handles: &PutLimitHandles) -> Option<Result<PutLimitTables, String>> {
    let mut values = Vec::new();
    for (handle, path) in handles.0.iter().zip([RANK_RELEASES, PUT_LIMIT_LEVELS]) {
        if let bevy::asset::LoadState::Failed(error) = server.load_state(handle) {
            return Some(Err(format!("{path} is not in this runtime root ({error})")));
        }
        let json = jsons.get(handle)?;
        let value: Value = serde_json::from_str(&json.0).unwrap_or_else(|e| panic!("{path} is not JSON: {e}"));
        values.push(value);
    }
    Some(Ok(PutLimitTables::parse(&values[0], &values[1]).unwrap_or_else(|e| panic!("Info put-limit tables: {e}"))))
}

// ScreenLayerMysekaiInfo's constructor sets the page fade duration to 0.1s.
const PAGE_FADE_SECONDS: f32 = 0.1;

pub(crate) fn spawn_when_ready(
    mut commands: Commands, layouts: Res<crate::ui_layout::UiLayouts>, server: Res<AssetServer>, spawned: Option<Res<InfoSpawned>>,
    jsons: Res<Assets<JsonAsset>>, handles: Option<Res<PutLimitHandles>>,
) {
    if spawned.is_some() || !["Info","Common1"].iter().all(|key|layouts.ready(key,&server)) {return;}
    let Some(handles) = handles else { return; };
    let Some(put_limit) = resolve_put_limit(&server, &jsons, &handles) else { return; };
    commands.remove_resource::<PutLimitHandles>();
    match &put_limit {
        Ok(tables) => info!("[info] put-limit tables: {} put-limit releases, {} level rows", tables.releases.len(), tables.levels.len()),
        Err(reason) => warn!("[info] {reason}; the rank page's put-cost texts keep the prefab's text"),
    }
    let missing: Vec<&str> = WORDINGS.iter().copied().filter(|key| !layouts.wordings.contains_key(*key)).collect();
    if !missing.is_empty() {
        warn!("[info] wordings missing from this root: {missing:?}; a text writing one of them refuses");
    }
    let doc = layouts.document("Info").expect("ready Info prefab");
    commands.insert_resource(InfoPresentation { bindings: InfoBindings::from_prefab(doc), elapsed: 0., put_limit });
    commands.spawn((InfoRoot,Visibility::Hidden,Transform::default(),RenderLayers::layer(SITEMAP_LAYER),crate::ui_layout::UiPrefabView::new("Info",SITEMAP_LAYER)));
    for (which,key) in [(InfoDialog::RankList,"Common1")] {
        commands.spawn((InfoDialogRoot{which},Visibility::Hidden,Transform::from_xyz(0.,0.,100.),RenderLayers::layer(SITEMAP_LAYER),crate::ui_layout::UiPrefabView::new(key,SITEMAP_LAYER)));
    }
    commands.insert_resource(InfoSpawned);
}

// ---------------------------------------------------------------------------
// Update：摆位与生命周期
// ---------------------------------------------------------------------------

fn page_label(page: usize) -> &'static str {
    match page {
        0 => "排名信息",
        _ => "设定",
    }
}

/// 进层钩子串（层栈的挂载梯日志之外，层侧内容在这里；无动画 ⇒ 等待点
/// 当帧通过）。
fn on_open(settings: &mut InfoSettings, mock: &InfoMock, page_state: &InfoPageState,
    rank: &moly_law::ui::mysekai_rank::MysekaiRankModel) {
    // OnBoot：bootData = 排名页模型（服务端用户态 ⇒ 面板下发）；访问许可
    // 现值重读（真源 OnBoot 读 UserDataManager）。
    settings.access = mock.access_permission;
    info!(
        "[info] OnBoot：bootData=排名页模型（总经验 {}（mock）查 master 等级表：rank {} / max {} · \
         本级累计 {} · 下级累计 {}；面板下发：家具摆放上限 {} · 连接家具上限 {}）；访问许可现值={}（面板下发）",
        rank.total_exp,
        rank.mysekai_rank,
        rank.max_mysekai_rank,
        rank.total_exp_to_current_rank,
        rank.total_exp_to_next_rank,
        mock.fixture_put_limit,
        mock.fixture_joint_put_limit,
        settings.access.true_name()
    );
    info!(
        "[info] OnInitComponent：三档读入（画质 {} · 刷新率 {} · 变换通知 {}）；七 Setup——\
         SetupTab · SetupRankInfoButton · SetupArrowButton · SetupPage 已接；视图读源预制体；\
         SetupParticle 挂账（粒子绘制路径未并）· SetupPageFlick 挂账\
         （手势层无 flick 分类器）",
        settings.image_quality.label(),
        settings.fps.label(),
        settings.convert.label()
    );
    info!(
        "[info] OnFinishStartAnimation：两页 Hide 后当前页「{}」FadeIn——层视图无动画，\
         当帧收尾（真源两段串行淡变未接）",
        page_label(page_state.current)
    );
    // 当前四组档值与默认位。
    info!(
        "[info] Info 铺装：页 {}/{} · 访问许可={}（面板下发，真源无客户端默认）· 画质={}\
         （默认位 {}）· 刷新率={}（默认位 {}）· 变换通知={}（默认位 {}）",
        page_state.current,
        PAGE_COUNT - 1,
        settings.access.true_name(),
        settings.image_quality.label(),
        ImageQuality::Normal.label(),
        settings.fps.label(),
        FpsQuality::High.label(),
        settings.convert.label(),
        ConvertNotification::On.label()
    );
}

/// 退层钩子串（出场链的层侧内容）。
fn on_close(settings: &InfoSettings, page_state: &InfoPageState) {
    info!(
        "[info] OnExitStart：当前页「{}」FadeOut——层视图无动画，当帧收尾",
        page_label(page_state.current)
    );
    info!(
        "[info] OnExited：三档写回 SaveToStorage（画质 {} · 刷新率 {} · 变换通知 {}）；访问许可上报 \
         ChangeVisitType={}（服务端域 ⇒ 面板下发 mock，上报沿到此）",
        settings.image_quality.label(),
        settings.fps.label(),
        settings.convert.label(),
        settings.access.true_name()
    );
    save_option_settings(settings);
}

/// Review is a locked presentation of the reject option, not another toggle.
fn selected_option(group: ToggleGroup, settings: &InfoSettings) -> usize {
    match group {
        ToggleGroup::Access => if settings.access == AccessPermission::Review { 2 } else { settings.access.index() },
        ToggleGroup::ImageQuality => settings.image_quality.index(),
        ToggleGroup::Fps => settings.fps.index(),
        ToggleGroup::Convert => settings.convert.index(),
    }
}

/// The info screen's rank gauge targets. The rank page class is not decoded,
/// so on a region layout the gauge is the document's single
/// `UIPartsMysekaiRankGauge` component and its decoded references; the
/// shared root's layouts leave the gauge classes undecoded, so there the
/// three nodes are addressed by path, a named gap of that root.
fn info_rank_gauge_targets(doc: &moly_assets::ui_layout::UiPrefab) -> crate::menu_dialog::RankGaugeTargets {
    if doc.source.region.is_none() {
        let gauge = "UIPartsMySekaiRankGauge";
        return crate::menu_dialog::RankGaugeTargets {
            rank_text: format!("{gauge}/CustomTextMesh (2)"),
            rest_text: format!("{gauge}/CustomTextMesh (3)"),
            fill_image: format!("{gauge}/UIPartsGauge/GaugeBase/Mask"),
        };
    }
    let gauges: Vec<i64> = doc.nodes.iter().flat_map(|node| node.components.iter())
        .filter(|c| c.class == "Sekai.Mysekai.UIPartsMysekaiRankGauge")
        .map(|c| c.path_id)
        .collect();
    let [gauge] = gauges[..] else {
        panic!("{}: {} UIPartsMysekaiRankGauge components, the rank page binds one", doc.prefab, gauges.len());
    };
    crate::menu_dialog::rank_gauge_references(doc, gauge)
}

/// The place system's own memory across frames: the open edge, the rank
/// model built per open, and the one-time scene start.
#[derive(Default)]
pub(crate) struct PlaceMemory {
    was_open: bool,
    rank: Option<moly_law::ui::mysekai_rank::MysekaiRankModel>,
    scene_started: bool,
}

/// Update：摆位与逐帧状态。每帧——
/// 1. 层开关沿（进层/退层钩子串各一串，与层栈梯日志同帧互补）；
/// 2. 根可见性 = 层栈当前层是否情报层；根缩放 = canvas 缩放；
/// 3. 源页节点按当前页显隐，对话框根按对话框态显隐；
/// 4. UiPrefabView 覆写单选勾选、箭头、量表与动态文案。
#[allow(clippy::type_complexity)]
pub(crate) fn place(
    windows: Query<&Window,With<PrimaryWindow>>, stack:Res<UiLayerStack>, mock:Res<InfoMock>,
    mut settings:ResMut<InfoSettings>,page_state:Res<InfoPageState>,dialog:Res<InfoDialogState>,
    mut roots:Query<(&mut Visibility,&mut Transform,&mut crate::ui_layout::UiPrefabView),(With<InfoRoot>,Without<InfoDialogRoot>)>,
    mut dialogs:Query<(&InfoDialogRoot,&mut Visibility,&mut Transform,&mut crate::ui_layout::UiPrefabView),Without<InfoRoot>>,
    mut memory: Local<PlaceMemory>,
    time: Res<Time>, mut presentation: Option<ResMut<InfoPresentation>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    ranks: Option<Res<crate::mysekai_rank::MysekaiRanks>>,
    total_exp: Res<crate::mysekai_rank::UserTotalExp>,
    layouts: Res<crate::ui_layout::UiLayouts>,
    mut graphics: ResMut<crate::game_settings::GameSettings>,
) {
    let PlaceMemory { was_open, rank, scene_started } = &mut *memory;
    if !*scene_started {
        *scene_started = true;
        apply_on_scene_start(&mut graphics, &settings);
    }
    let open=stack.current()==LayerId::MysekaiInfo;
    let opening = open && !*was_open;
    if opening {
        // TryMoveScreenLayerMysekaiInfo: a new rank model per open.
        *rank = Some(ranks.as_deref()
            .unwrap_or_else(|| panic!("rank model: the master rank table has not resolved when the info screen opens"))
            .model(total_exp.0));
    }
    match (*was_open,open) {(false,true)=>on_open(&mut settings,&mock,&page_state,rank.as_ref().expect("rank model built on open")),(true,false)=>on_close(&settings,&page_state),_=>{}}
    *was_open=open;
    let Some(presentation) = presentation.as_deref_mut() else { return; };
    if !open || opening { presentation.elapsed = 0.; }
    else { presentation.elapsed = (presentation.elapsed + time.delta_secs()).min(PAGE_FADE_SECONDS); }
    let t = presentation.elapsed / PAGE_FADE_SECONDS;
    // DOFade uses the engine's default OutQuad ease. Both groups fade together
    // for the initial FadeInAsync; directional page-change choreography is a
    // separate source flow, not a generic dialog-scale animation.
    let alpha = 1. - (1. - t) * (1. - t);
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else { return; };
    let scale = root_canvas.scale(window); let page=page_state.current;
    let bindings = &presentation.bindings;
    let put_limit = &presentation.put_limit;
    let reviewing = settings.access == AccessPermission::Review;
    for (mut visible,mut transform,mut view) in &mut roots {
        *visible=if open {Visibility::Inherited}else{Visibility::Hidden};transform.scale=Vec3::splat(scale);
        for (index, source_page) in bindings.pages.iter().enumerate() {
            view.set_visible(&source_page.node, index == page);
            for group in &source_page.groups { view.set_alpha(group, if open && index == page { alpha } else { 0. }); }
        }
        for (index, tab) in bindings.tabs.iter().enumerate() {
            view.set_visible(&tab.on_image, page == index);
            view.set_visible(&tab.off_image, page != index);
        }
        view.set_visible(&bindings.arrow_prev,page>0);view.set_visible(&bindings.arrow_next,page+1<PAGE_COUNT);
        if let Some(rank) = rank.as_ref() {
            view.set_text(&bindings.rank_page.rank_text, rank.mysekai_rank.to_string());
            let doc = layouts.document(view.key).expect("info layout is loaded");
            crate::menu_dialog::apply_rank_gauge(&mut view, &layouts, &info_rank_gauge_targets(doc), rank);
            // FixtureHomePutLimitPutCost.ToString() and
            // GetFormat(MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST, room cost).
            if let (Some([home, room]), Ok(tables)) = (&bindings.rank_page.put_limit_costs, &put_limit) {
                let rank = i64::from(rank.mysekai_rank);
                view.set_text(home, tables.cost(rank, SiteCategory::HousingHome).to_string());
                let format = layouts.wordings.get(MYROOM_COST_WORDING).unwrap_or_else(|| panic!("UI wording missing: {MYROOM_COST_WORDING}"));
                let room_cost = tables.cost(rank, SiteCategory::HousingRoom).to_string();
                let text = moly_law::text::custom_text_mesh::format_wording(format, &[room_cost])
                    .unwrap_or_else(|e| panic!("UI wording {MYROOM_COST_WORDING}: {e}"));
                view.set_text(room, text);
            }
        }
        // SetupPhotoAsync with no user photo: _photoFramePreview.Hide().
        if let Some(preview) = &bindings.rank_page.photo_preview {
            view.set_alpha(preview, 0.);
        }
        // SetVoiceDLButtonState(IsMysekaiVoice).
        let voice_key = if MYSEKAI_VOICE_DOWNLOADED { "WORD_DOWNLOADED" } else { "WORD_BULK_DOWNLOAD" };
        let voice_text = layouts.wordings.get(voice_key).unwrap_or_else(|| panic!("UI wording missing: {voice_key}"));
        view.set_text(&bindings.voice_download_text, voice_text.clone());
        for cover in &bindings.voice_download_covers {
            view.set_visible(cover, MYSEKAI_VOICE_DOWNLOADED);
        }
        if let Some([count, joint]) = &bindings.put_limit_counts {
            view.set_text(count, mock.fixture_put_limit.to_string());
            view.set_text(joint, mock.fixture_joint_put_limit.to_string());
        }
        if let Some(tip) = &bindings.review_tip { view.set_visible(tip, reviewing); }
        for (group, toggles) in &bindings.toggles {
            let selected = selected_option(*group, &settings);
            for (index, toggle) in toggles.iter().enumerate() {
                // Toggle.graphic paints the selected state. Inactive template
                // siblings are not options and keep their source activation.
                view.set_alpha(&toggle.graphic, if index == selected { 1. } else { 0. });
                for cover in &toggle.covers {
                    // The authored cover color equals the source disable_a50
                    // palette entry; showing it preserves the source RGBA.
                    view.set_visible(cover, *group == ToggleGroup::Access && reviewing);
                }
            }
        }
    }
    for (root,mut visible,mut transform,mut view) in &mut dialogs {
        *visible=if open&&dialog.open==Some(root.which){Visibility::Inherited}else{Visibility::Hidden};transform.scale=Vec3::splat(scale);
        view.set_visible("WindowRoot/Tabs",false);
        match root.which {
            InfoDialog::RankList=>{view.set_text("Content/MessageBody",format!("{}
{}
{}",InfoItem::RankLevelText.current_label(&mock, rank.as_ref().map_or(0, |r| r.mysekai_rank)).unwrap(),InfoItem::FixturePutText.current_label(&mock, 0).unwrap(),InfoItem::FixtureJointText.current_label(&mock, 0).unwrap()));}
        }
    }
}

// ---------------------------------------------------------------------------
// Update：点按
// ---------------------------------------------------------------------------

/// 点按屏位（顶为原点）→ 参照画布坐标。
fn to_canvas(position: Vec2, width: f32, height: f32, scale: f32) -> Vec2 {
    Vec2::new(position.x - width / 2.0, height / 2.0 - position.y) / scale
}

fn item_path(item:InfoItem, bindings:&InfoBindings)->Option<&str> {
    Some(match item {
        InfoItem::Tab(index)=>&bindings.tabs.get(index)?.control,
        InfoItem::ArrowPrev=>&bindings.arrow_prev,InfoItem::ArrowNext=>&bindings.arrow_next,
        InfoItem::Photo=>"RankPage/Left/Photo/SelectButton",InfoItem::RankInfo=>&bindings.rank_info,
        InfoItem::VoiceDownload=>&bindings.voice_download,InfoItem::Toggle(group,index)=>&bindings.toggle(group,index)?.control,
        InfoItem::RankDialogClose=>"FooterButtons/UIPartsCommonButton",
        _=>return None,
    })
}
fn hit_test(canvas:Vec2,item:InfoItem,layouts:&crate::ui_layout::UiLayouts,view:&crate::ui_layout::UiPrefabView,size:Vec2,bindings:&InfoBindings)->bool {
    item_path(item,bindings).and_then(|path|view.rect(layouts,path,size)).is_some_and(|r|r.active&&r.contains(canvas))
}
fn hit_item(canvas:Vec2,page:usize,layouts:&crate::ui_layout::UiLayouts,view:&crate::ui_layout::UiPrefabView,size:Vec2,bindings:&InfoBindings)->Option<InfoItem> {
    let mut items=vec![InfoItem::Tab(0),InfoItem::Tab(1),InfoItem::ArrowPrev,InfoItem::ArrowNext];
    if page==0 {items.extend([InfoItem::RankInfo,InfoItem::Photo]);} else {items.push(InfoItem::VoiceDownload);for (group,toggles) in &bindings.toggles {for index in 0..toggles.len(){items.push(InfoItem::Toggle(*group,index));}}}
    items.into_iter().find(|item|hit_test(canvas,*item,layouts,view,size,bindings))
}

/// 页切换（真源 `ChangePage`：写页下标 → RefreshTab（页签组不带通知写 +
/// 逐页签 Setup）→ 箭头刷新 → 方向按新旧差）。页过渡当帧落位（粒子与
/// 淡变挂账，见模块头）。
fn change_page(page_state: &mut InfoPageState, next: usize, via: &str) {
    if next == page_state.current {
        info!(
            "[info] 页点按未变页（{via}）——转亮才回调，同值无回调（真源单选组同形）"
        );
        return;
    }
    let old = page_state.current;
    page_state.current = next;
    let direction = if next > old { "前进" } else { "后退" };
    let prev_able = if next > 0 { "可动" } else { "到界" };
    let next_able = if next + 1 < PAGE_COUNT {
        "可动"
    } else {
        "到界"
    };
    info!(
        "[info] 页签切换 {old}→{next}（{via}）：RefreshTab 不带通知写页签组并逐页签 Setup → \
         箭头刷新（上一页 {prev_able} · 下一页 {next_able}）→ 方向={direction}；页过渡=旧页方向性\
         淡出 + 两枚页签粒子 + 新页方向性淡入（粒子绘制路径未并 · 淡变与节拍未提取 ⇒ 当帧落位，\
         具名挂账）"
    );
}

/// Close the open dialog: `DialogBase.Close` and the destroy after it (no
/// close animation here).
fn close_dialog(dialog: &mut InfoDialogState, screens: &mut UiLayerStack) {
    dialog.open = None;
    if let Some(id) = dialog.id.take() {
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
    }
}

/// 档位切换（切换律见模块头：两组立即施加、两组只写档位）。
fn select_option(
    group: ToggleGroup,
    index: usize,
    graphics: &mut crate::game_settings::GameSettings,
    settings: &mut InfoSettings,
) {
    match group {
        ToggleGroup::Access => {
            let Some(next) = AccessPermission::from_index(index) else {
                return;
            };
            if next == settings.access {
                info!("[info] 访问许可档点按未变（{}）——转亮才回调（真源同形）", next.label());
                return;
            }
            settings.access = next;
            info!(
                "[info] 访问许可 → {}（OnChangeVisitSettingToggle 只写档位，无即时副作用；\
                 上报在出场链——服务端域 ⇒ 面板下发 mock）",
                next.true_name()
            );
        }
        ToggleGroup::ImageQuality => {
            let Some(next) = ImageQuality::from_index(index) else {
                return;
            };
            if next == settings.image_quality && graphics.graphics.fxaa == next.target_dpi_and_fxaa().1 {
                info!("[info] 画质档点按未变（{}）——转亮才回调（真源同形）", next.label());
                return;
            }
            settings.image_quality = next;
            info!(
                "[info] 画质 → {}（OnChangeImageQualityToggle 写档位并立即施加）",
                next.label()
            );
            apply_image_quality(graphics, next);
        }
        ToggleGroup::Fps => {
            let Some(next) = FpsQuality::from_index(index) else {
                return;
            };
            if next == settings.fps && graphics.graphics.frame_rate == next.target_frame_rate() as u16 {
                info!(
                    "[info] 刷新率档点按未变（{}）——转亮才回调（真源同形）",
                    next.label()
                );
                return;
            }
            settings.fps = next;
            info!(
                "[info] 刷新率 → {}（OnChangeFpsToggle 写档位并立即施加）",
                next.label()
            );
            apply_fps(graphics, next);
        }
        ToggleGroup::Convert => {
            let Some(next) = ConvertNotification::from_index(index) else {
                return;
            };
            if next == settings.convert {
                info!(
                    "[info] 变换通知档点按未变（{}）——转亮才回调（真源同形）",
                    next.label()
                );
                return;
            }
            settings.convert = next;
            info!(
                "[info] 变换通知 → {}（OnChangeConvertFixtureNotificationTypeToggle 只写档位，\
                 无即时副作用）",
                next.label()
            );
        }
    }
}

/// 点按分派：对话框开着 ⇒ 模态（先过它，框外点按收框）；然后层件命中。
/// 层开着时**全部点按被本层吃掉**（全屏层阻断世界射线，真源
/// blockRaycasts 同形）。
pub(crate) fn click(
    layouts: Res<crate::ui_layout::UiLayouts>,
    presentation: Option<Res<InfoPresentation>>,
    views: Query<(&crate::ui_layout::UiPrefabView,Option<&InfoDialogRoot>)>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut gestures: MessageReader<GestureEvent>,
    mut graphics: ResMut<crate::game_settings::GameSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut stack: ResMut<UiLayerStack>,
    mut settings: ResMut<InfoSettings>,
    mut page_state: ResMut<InfoPageState>,
    mut dialog: ResMut<InfoDialogState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    // The back key reaches the open rank list first (SubWindowDialog: close);
    // with no dialog the screen manager's header back pops the screen.
    for event in back_keys.read() {
        if dialog.id == Some(event.id) {
            info!("[info] back key: the rank list closes");
            close_dialog(&mut dialog, &mut stack);
        }
    }
    if taps.is_empty() || stack.current() != LayerId::MysekaiInfo {
        return;
    }
    let Some(presentation) = presentation.as_deref() else { return; };
    let bindings = &presentation.bindings;
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    let Some((view,_))=views.iter().find(|(v,_)|v.key=="Info") else {return;};
    for position in &taps {
        // 全屏层吃掉这一下（世界射线拿不到——真源 blockRaycasts 同形）。
        consumed.0 = true;
        let canvas = to_canvas(*position, width, height, scale);
        // ---- 对话框模态先手 ----
        if let Some(which) = dialog.open {
            let Some((dialog_view,_))=views.iter().find(|(_,r)|r.is_some_and(|r|r.which==which)) else {continue;};
            let buttons: &[InfoItem] = match which {
                InfoDialog::RankList => &[InfoItem::RankDialogClose],
            };
            match buttons.iter().copied().find(|item| hit_test(canvas, *item,&layouts,dialog_view,size,bindings)) {
                Some(InfoItem::RankDialogClose) => {
                    info!("[info] 排名清单·关闭按下 → 关框")
                }
                _ => info!(
                    "[info] 对话框开着，框外点按收框（模态；排名清单框真源允许框外关，同形）"
                ),
            }
            close_dialog(&mut dialog, &mut stack);
            continue;
        }
        // ---- 层件 ----
        if let Some(item) = hit_item(canvas, page_state.current,&layouts,view,size,bindings) {
            let disabled_or_selected = match item {
                InfoItem::Tab(index) => index == page_state.current,
                InfoItem::Toggle(group, index) => {
                    (group == ToggleGroup::Access && settings.access == AccessPermission::Review)
                        || selected_option(group, &settings) == index
                }
                // A disabled CustomButton takes no click.
                InfoItem::Photo => !PHOTO_BUTTON_ENABLED,
                InfoItem::VoiceDownload => MYSEKAI_VOICE_DOWNLOADED,
                _ => false,
            };
            if disabled_or_selected {
                // A grouped CustomToggle already on exits before button sound
                // or callbacks. Review additionally locks all access options.
                continue;
            }
            if let Some(path) = item_path(item,bindings) { sounds.source_button(&layouts, view.key, path); }
            match item {
                InfoItem::Tab(index) => change_page(&mut page_state, index, "页签"),
                InfoItem::ArrowPrev if page_state.current > 0 => {
                    let next = page_state.current - 1;
                    change_page(&mut page_state, next, "上一页箭头");
                }
                InfoItem::ArrowNext if page_state.current + 1 < PAGE_COUNT => {
                    let next = page_state.current + 1;
                    change_page(&mut page_state, next, "下一页箭头");
                }
                InfoItem::Toggle(group, index) => {
                    select_option(group, index, &mut graphics, &mut settings)
                }
                InfoItem::Photo => info!(
                    "[info] photo button: OnSelectedPhoto opens the photo selection, which is not in this product"
                ),
                InfoItem::RankInfo => {
                    info!(
                        "[info] 排名详情钮按下 → Show1ButtonDialog(排名清单 342)——Dialog 槽，\
                         不入层栈；清单体 = 主表清单 + 用户态 ⇒ 面板下发 mock"
                    );
                    match stack.show_dialog(RANK_LIST_DIALOG, DisplayLayerType::LayerDialog, DialogBackKey::Close, "ScreenLayerMysekaiInfo (rank info)") {
                        Ok(id) => {
                            // Shown without an open animation here.
                            stack.open_dialog(id);
                            stack.dialog_open_finished(id);
                            dialog.id = Some(id);
                        }
                        Err(error) => warn!("[info] {error}; the rank list shows outside the screen manager"),
                    }
                    dialog.open = Some(InfoDialog::RankList);
                }
                InfoItem::VoiceDownload => info!(
                    "[info] voice download: ShowVoiceDownloadDialog; the bulk download is a server feature not in this product"
                ),
                _ => {}
            }
        } else {
            info!("[info] 层内点按未命中件（面板背景）——吃掉，不下传");
        }
    }
}
