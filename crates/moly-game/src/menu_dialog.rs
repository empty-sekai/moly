//! 菜单对话框（真源 `MysekaiMenuDialog`，DialogType 309，Dialog 槽）——
//! 场地屏外壳菜单钮的目标：体力/等级两格 + 12 个可按件的导航中枢。
//!
//! ## 入口与身份（Dialog 槽，不是层栈）
//!
//! 外壳菜单钮 `ShowMenu` → `DialogUtility.ShowMysekaiMenu(onChangeSite)`：
//! 先 `TryGetActiveDialog(MysekaiMenuDialog, Layer_Dialog)`（已开 ⇒ 只
//! 重新 Setup，不重复实例化），否则 `InstantiateDialog(309, Layer_Dialog)`
//! → `Initialize(messageBody=null, onClose=null, allowCloseExternal=true)`
//! → SetActive(false) → `Setup(ViewData)` → `Open`。**框外点按收框成立**
//! （allowCloseExternal = true，Initialize 第三参原生核过）。ViewData 的
//! `OnChangeSite` 在本对话框体内从不被调用（仅存字段，构造点穷举：仅
//! 两处读都是 ShouldEnable 两格）——本仓不设对应物，具名。
//!
//! ViewData 派生（原生 `ShowMysekaiMenu`）：`IsOwner()` ·
//! `ShouldEnableInventoryButton = !IsVisiting()` ·
//! `ShouldEnableInfoButton = !IsVisiting()`。单机（无多人域）⇒ 恒真；
//! 访客态走 mock 面板一格。
//!
//! ## 12 个可按件（字段表穷举定谳：CustomButton 恰 11 + DialogBase 关闭钮）
//!
//! | 钮（字段名） | 路由（方法体直读） | 使能律 |
//! |---|---|---|
//! | 体力恢复 `_recoverStaminaButton` | `Show2ButtonDialog(MysekaiRecoverBoostStaminaDialog)` + 自身 `Close` | Setup 恒 true |
//! | 宝箱 `_chestButton` | `PushUIScreen(MysekaiInventory 607)` + `Close`（先查多人房） | `!IsVisiting()` |
//! | 情报 `_mysekaiInfoButton` | `MysekaiInfoUtility.TryMoveScreenLayerMysekaiInfo`（先查已开 622）+ `Close` | `!IsVisiting()` |
//! | 换装 `_mysekaiAvtarChangeButton`（源侧拼写照录） | `PushUIScreen(MysekaiAvatarCostumeSetting 629)` + `Close` | Setup 恒 true |
//! | 回标题 `_transitionToTitleButton` | `MysekaiUtility.TransitionToTitle`：`Disconnect` 后 `SceneManager.RequestScene(0)`（标题场景） | 无 Setup，恒可按 |
//! | 退出 mysekai `_exitMysekaiButton` | `TransitionToOutGame`：一次性闩 `_isTransitionOutGame` → `DisableTapScreen` + `StopBGM(0.25)` + `FadeOutAsync(0.25)` → `Yield` → `MysekaiUtility.TransitionToOutGame` = `Disconnect` 后 `RequestScene(1)`（游戏外场景） | Setup 显式置 true |
//! | 拍照 `_photoShotButton` | `ChangeState(GameStateType.PhotoShot)` + `Close`（先查已在 PhotoShot 态） | `MysekaiPhotoShotUtility.IsAllowedToPhotoShotInCurrentSite()` = `CurrentSiteType < 4` |
//! | 相册 `_photoAlbumButton` | `MysekaiPhotoAlbumUtility.TryShowPhotoAlbumScreen()` + `Close` | 无 Setup，恒可按 |
//! | 水晶商店 `_crystalShopButton` | `BootArg` + `PushUIScreen(CrystalShop)` + `Close` | `MysekaiUtility.IsAllowedToOpenCrystalShop()` = `!IsVisiting(竞赛) && (IsOwner || 离线模式)` |
//! | 白图写生 `_whiteBlueprintSketchButton` | `Player.ChangeState(SelectSketchItem)` + `ChangeState(Sketch)` + `Close`；他人编辑中 → `MSG_SKETCH_WARNING_OWNER_EDIT_MODE` 子窗（Overlay 层） | `SketchUtility.IsSketchAvailable()`（master+user 道具查） |
//! | 素材交换 `_exchangeButton` | `BootData(CallerSceneType.Mysekai)` + `PushUIScreen(MasterLessonMaterialExchange)` + `Close` | 无 Setup，恒可按 |
//! | 关闭（`DialogBase.closeButton`） | `Close` → `CloseProcess` = `HideAsync` + `PlaySEOneShot("SE_SUBWINDOW_CLOSE")` + `Dispose` 资产包 | 恒可按 |
//!
//! 计数修正：普查记「13 钮 / 12 导航钮」，序列化字段表穷举为 **11 个
//! CustomButton + 关闭钮**（逐字段点名，无第 12 个导航钮）。每条路由句
//! 尾的 `Close()` 是对话框自己的关框（路由后收框），不进层栈。
//!
//! ## 两格取值链（+ 一处普查修正）
//!
//! - **体力格**：`HarvestUtility.GetStaminaData()` 读
//!   `UserDataManager.UserMysekaiStamina{normal,enhance,boost}`（**服务端
//!   用户态**）→ `MysekaiStaminaView.UpdateStaminaView` + 量表率
//!   `GetStaminaGageRate`（`MasterMysekaiStaminas`；已提取，完整量表消费
//!   尚未接入）。体力空 ⇒ `_recoverStaminaLabel` 亮（`IsEmptyStamina`
//!   = normal ≤ 0 且 boost ≤ 0 且 enhance < 1）。
//! - **等级格**：`MysekaiRankModel` 读 `UserMysekaiGamedata.totalExp`
//!   （**服务端用户态**）→ 查 `MasterMysekaiRank` 表（**master 镜像不在
//!   提取管线**）得等级与距下一级经验，进 `UIPartsMysekaiRankGauge`。
//! - ⚠ **普查的「宝石余额」一格在真源里不存在**：字段表穷举无 jewel
//!   字段、原生树本文件 `grep -ic jewel` = 0 ⇒ 三格修正为两格。宝石是
//!   恢复对话框的消耗货币，不在本对话框的显示面。
//! - ⚠ 「体力视图直接持 `ScreenLayerMysekaiHUD` 引用回写」也查无字段
//!   （原生 HUD grep = 0）：恢复完成后的体力回写走**回调**——
//! `Show2ButtonDialog` 时传 `onRecoverFinish = UpdateStaminaGateView`。
//!   恢复对话框未建 ⇒ 回调对应物具名挂账（建它时接同一条回调律）。
//!
//! ## 钮使能的两层门（本仓形态）
//!
//! 真源使能律由 user 态派生（上表右列）；本仓再叠一层「路由目标是否
//! 已建」：目标未建的钮**置灰**（点按不响应、响亮记账），已建的走真
//! 路由。当前已建目标唯一：情报层。两层门都报在开框行里
//! （源使能 + 目标已建与否），不合成一个数。
//!
//! ## 生命周期（Setup 装配序，真源逐句）
//!
//! `Setup(ViewData)`：先 await `WaitHarvestSync`（采集同步，无对应物 ⇒
//! 当帧通过）→ 八个 `SetupXxxButton`（挂监听 + 写使能）→
//! `_exitMysekaiButton.enabled = true` → `SetupRawImage`（9 个图标位从
//! 资产包 `mysekai/ui/mysekai_menu` 逐名装贴图：btn_cheki_menu_avatar
//! · btn_cheki_menu_album · btn_mysekai_menu_home · btn_cheki_menu_item
//! · btn_cheki_menu_crystalShop · btn_cheki_menu_mySekaiInfo ·
//! btn_cheki_menu_sketch · btn_cheki_menu_photo · btn_mysekai_menu_change）
//! → `SetupMysekaiRankGauge` → `UpdateStaminaGateView` → 回标题/退出/
//! 素材交换三个监听 → 教程态 `SetupTutorial` / 生日态 `SetupForBirthday`
//! 全量置灰覆盖（本仓无教程/生日上下文 ⇒ 不接，具名）。
//!
//! 关框：`Close` = `CloseProcess` = `HideAsync`（收场动画）+
//! `PlaySEOneShot("SE_SUBWINDOW_CLOSE")` + `Dispose("mysekai/ui/mysekai_menu")`。
//! 窗口沿序列化的 SubWindowSlideAnimation 绑定滑动，开关均为 0.2 秒
//! OutQuart；收场期间视图保留并占用输入。SE_SUBWINDOW_CLOSE 尚缺可播放
//! 的源 cue，资产包 Dispose 尚无独立对应物。
//!
//! ## 服务端域与 mock 面板（照音频/情报面板的形：具名资源 + 默认 +
//! 环境变量覆写，非法值响亮告警回默认）
//!
//! 体力三值 · 体力量表上限（master）· 等级 · 距下一级经验（master 派生）
//! · 访客态（使能输入）· 拍照许可 · 水晶商店许可 · 写生可用 ⇒ 全部
//! 具名 mock。环境变量：`MOLY_MENU_MOCK_STAMINA_NORMAL` ·
//! `MOLY_MENU_MOCK_STAMINA_ENHANCE` · `MOLY_MENU_MOCK_STAMINA_BOOST` ·
//! `MOLY_MENU_MOCK_STAMINA_MAX` · `MOLY_MENU_MOCK_RANK_LEVEL` ·
//! `MOLY_MENU_MOCK_RANK_EXP_NEXT` · `MOLY_MENU_MOCK_VISITING` ·
//! `MOLY_MENU_MOCK_PHOTO_SHOT_ALLOWED` · `MOLY_MENU_MOCK_CRYSTAL_SHOP_ALLOWED`
//! · `MOLY_MENU_MOCK_SKETCH_AVAILABLE`。
//!
//! ## 我方选值与具名缺口（改这里之前先读）
//!
//! - 贴图与 RectTransform 使用提取数据。滑动目标、背板和内容矩形跟随
//!   组件引用；动画距离由窗口实际装配尺寸计算，不另设固定屏宽。
//! - 固定文案并入外壳字符集（图集的**第六个**消费者）：动态值只有整数，
//!   无自由文本 ⇒ 装载期可枚举全量。缺字形在铺字点 fail-closed。
//! - **模态**：开着时全部点按先过它（真源 Dialog 槽 blockRaycasts 同
//!   形）；外壳点按与小地图点按的门都读同一个 menu_open 位（与离开
//!   确认框同族门）。已知偏差与外壳确认框同款：动作按钮的点按消费
//!   不过这道门（它不读对话框态，既有具名挂账），本模块不动它的判定面。
//! - 退出/回标题两钮的目标是**场景迁移**（RequestScene 0/1），整个场景
//!   域在产品外（本仓只有 mysekai 场景）⇒ 置灰 + 具名记账。
//! - 冒烟口：`MOLY_MENU_DIALOG_AUTOSMOKE_SECS` 给秒数后按 2 秒一拍五步
//!   （开框 → 置灰钮 → 关闭钮收框 → 再开框 → 情报真跳转），点按走事件面
//!   注入，与真实点按同一条分派路。
//!
//! 具名挂账（本模块不实现，收工报里重列）：
//! - 恢复体力对话框 `MysekaiRecoverBoostStaminaDialog`（自发结算 API 族，
//!   判 mock）——体力回写回调 `onRecoverFinish` 的消费者。
//! - 层栈目标未建的 5 钮（宝箱/换装/水晶商店/写生/素材交换）+ 相册
//!   （PhotoAlbum 域非层栈）+ 场景迁移 2 钮（回标题/退出）。
//! - SE_SUBWINDOW_CLOSE · 教程/生日置灰覆盖。


use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};

use crate::action_button::ActionTapConsumed;
use crate::balloon::canvas_scale;
use crate::gesture::{GestureEvent, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId};
use crate::ui_layout::{UiLayouts, UiPrefabView};

const MENU_SLIDE_DURATION: f32 = 0.2;

// ---------------------------------------------------------------------------
// 可按件：身份与屏位（我方选值）
// ---------------------------------------------------------------------------

/// 对话框件的身份（命中判定、摆位与文案重建的键）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MenuButton {
    /// 体力恢复（真源 `_recoverStaminaButton`）。
    RecoverStamina,
    /// 宝箱（真源 `_chestButton`）。
    Chest,
    /// 情报（真源 `_mysekaiInfoButton`）。
    Info,
    /// 换装（真源 `_mysekaiAvtarChangeButton`，源侧拼写照录）。
    AvatarChange,
    /// 回标题（真源 `_transitionToTitleButton`）。
    TransitionToTitle,
    /// 退出 mysekai（真源 `_exitMysekaiButton`）。
    ExitMysekai,
    /// 拍照（真源 `_photoShotButton`）。
    PhotoShot,
    /// 相册（真源 `_photoAlbumButton`）。
    PhotoAlbum,
    /// 水晶商店（真源 `_crystalShopButton`）。
    CrystalShop,
    /// 白图写生（真源 `_whiteBlueprintSketchButton`）。
    WhiteBlueprintSketch,
    /// 素材交换（真源 `_exchangeButton`）。
    Exchange,
    /// 关闭（真源 `DialogBase.closeButton`）。
    Close,
}

/// 全部可按件（序即真源装配序：八钮 Setup → exit → 三监听；关闭钮随
/// DialogBase）。**12 件**——计数定谳承重句：CustomButton 字段穷举 11 +
/// DialogBase 关闭钮 1，普查的「13/12」对不上字段表。
pub(crate) const ALL_BUTTONS: [MenuButton; 12] = [
    MenuButton::RecoverStamina,
    MenuButton::Chest,
    MenuButton::Info,
    MenuButton::AvatarChange,
    MenuButton::TransitionToTitle,
    MenuButton::ExitMysekai,
    MenuButton::PhotoShot,
    MenuButton::PhotoAlbum,
    MenuButton::CrystalShop,
    MenuButton::WhiteBlueprintSketch,
    MenuButton::Exchange,
    MenuButton::Close,
];

impl MenuButton {
    pub(crate) fn source_path(self) -> &'static str {
        match self {
            Self::RecoverStamina => "MenuHeader/bg/UIPartsPlusButton",
            Self::Chest => "MenuRoot/Chest", Self::Info => "MenuRoot/MysekaiSetting",
            Self::AvatarChange => "MenuRoot/AvatarChange", Self::Exchange => "MenuRoot/Exchange",
            Self::PhotoShot => "MenuRoot/Photo", Self::PhotoAlbum => "MenuRoot/Album",
            Self::CrystalShop => "MenuRoot/Crystal", Self::WhiteBlueprintSketch => "MenuRoot/Sketch",
            Self::TransitionToTitle => "MenuRoot/TitleCell", Self::ExitMysekai => "MenuRoot/Home",
            Self::Close => "UIPartsCloseButton",
        }
    }

    /// 主文案（全部我方自写——真源是资产字符串未提取；字表已对仓内
    /// 字体子集核过全量可烘）。
    fn label(self) -> &'static str {
        match self {
            MenuButton::RecoverStamina => "恢复体力",
            MenuButton::Chest => "宝箱",
            MenuButton::Info => "情报",
            MenuButton::AvatarChange => "换装",
            MenuButton::TransitionToTitle => "回标题",
            MenuButton::ExitMysekai => "退出mysekai",
            MenuButton::PhotoShot => "拍照",
            MenuButton::PhotoAlbum => "相册",
            MenuButton::CrystalShop => "水晶商店",
            MenuButton::WhiteBlueprintSketch => "白图写生",
            MenuButton::Exchange => "素材交换",
            MenuButton::Close => "关闭",
        }
    }

    /// 真源使能律（上表右列；输入全部来自 mock 面板，见 [`MenuMock`]）。
    fn source_enabled(self, mock: &MenuMock) -> bool {
        match self {
            // !IsVisiting()（ViewData 派生，原生核过）。
            MenuButton::Chest | MenuButton::Info => !mock.visiting,
            // IsAllowedToPhotoShotInCurrentSite = CurrentSiteType < 4。
            MenuButton::PhotoShot => mock.photo_shot_allowed,
            // IsAllowedToOpenCrystalShop = !竞赛访客 && (站主 || 离线)。
            MenuButton::CrystalShop => mock.crystal_shop_allowed,
            // IsSketchAvailable（master+user 道具查）。
            MenuButton::WhiteBlueprintSketch => mock.sketch_available,
            // Setup 显式置 true（体力恢复/换装/退出）；其余无 Setup 调用，
            // 预制体初值取可按。
            _ => true,
        }
    }

    /// 路由目标是否已建（本仓叠加门：未建 ⇒ 置灰；真源没有这层门——
    /// 它的全部目标都在）。已建目标当前唯一：情报层。
    fn target_built(self) -> bool {
        matches!(self, MenuButton::Info)
    }

    /// 路由目标名（开框行与点按行用）。
    fn route_target(self) -> &'static str {
        match self {
            MenuButton::RecoverStamina => "恢复体力对话框（二钮框）",
            MenuButton::Chest => "库存层 PushUIScreen(MysekaiInventory 607)",
            MenuButton::Info => "情报层 TryMoveScreenLayerMysekaiInfo(622)",
            MenuButton::AvatarChange => "换装层 PushUIScreen(MysekaiAvatarCostumeSetting 629)",
            MenuButton::TransitionToTitle => "标题场景 SceneManager.RequestScene(0)",
            MenuButton::ExitMysekai => "游戏外场景 SceneManager.RequestScene(1)",
            MenuButton::PhotoShot => "拍照态 ChangeState(GameStateType.PhotoShot)",
            MenuButton::PhotoAlbum => "相册域 TryShowPhotoAlbumScreen",
            MenuButton::CrystalShop => "水晶商店层 PushUIScreen(CrystalShop)",
            MenuButton::WhiteBlueprintSketch => "写生态 ChangeState(GameStateType.Sketch)",
            MenuButton::Exchange => "素材交换层 PushUIScreen(MasterLessonMaterialExchange)",
            MenuButton::Close => "关框（HideAsync+SE+Dispose 资产包）",
        }
    }
}

/// 菜单对话框全部渲染文案的字符闭包（静态标签 + 数字位）——并入外壳
/// 字符集（图集的第六个消费者）。动态值只有整数，无自由文本 ⇒ 装载期
/// 可枚举全量。
pub(crate) const FIXED_TEXTS: &[&str] = &[
    "菜单",
    "体力 0/0",
    "等级 0 距下一级 0exp",
    "体力已空 可恢复",
    "恢复体力",
    "宝箱",
    "情报",
    "换装",
    "回标题",
    "退出mysekai",
    "拍照",
    "相册",
    "水晶商店",
    "白图写生",
    "素材交换",
    "关闭",
    "0123456789",
];

// ---------------------------------------------------------------------------
// mock 面板（服务端态，具名「mock 值」；照情报层面板的形）
// ---------------------------------------------------------------------------

/// 菜单对话框的具名 mock 服务端状态（体力三值 + 量表上限 + 等级两值 +
/// 四个使能输入）。真值分别住在 `UserMysekaiStamina` /
/// `UserMysekaiGamedata.totalExp`（服务端用户态）与 master 镜像（不在
/// 提取管线）里，本仓读不到 ⇒ 面板下发；默认值全部是我方选值。
#[derive(Resource)]
pub(crate) struct MenuMock {
    /// `UserMysekaiStamina.normalStamina`（服务端）。
    stamina_normal: i32,
    /// `UserMysekaiStamina.enhanceStamina`（服务端）。
    stamina_enhance: i32,
    /// `UserMysekaiStamina.boostStamina`（服务端）。
    stamina_boost: i32,
    /// 体力量表上限（`MasterMysekaiStaminas`，master 镜像不在管线）。
    stamina_max: u32,
    /// 等级（`totalExp` 查 `MasterMysekaiRank` 的派生值，master 不在管线
    /// ⇒ 直接 mock 派生结果）。
    rank_level: u32,
    /// 距下一级经验（同上，派生结果）。
    rank_exp_next: u32,
    /// `MysekaiMultiplayController.IsVisiting()`——多人访客态（使能输入；
    /// 本仓无多人域，默认 false）。
    visiting: bool,
    /// `MysekaiPhotoShotUtility.IsAllowedToPhotoShotInCurrentSite()` =
    /// `CurrentSiteType < 4`（当前站语义 mock；默认 true）。
    photo_shot_allowed: bool,
    /// `MysekaiUtility.IsAllowedToOpenCrystalShop()`（默认 true：单机恒站主）。
    crystal_shop_allowed: bool,
    /// `SketchUtility.IsSketchAvailable()`（master+user 道具查；默认 true）。
    sketch_available: bool,
}

impl MenuMock {
    /// 体力合计（真源 `StaminaData.SumStamina` = normal + enhance + boost）。
    fn stamina_sum(&self) -> i32 {
        self.stamina_normal.wrapping_add(self.stamina_enhance).wrapping_add(self.stamina_boost)
    }

    /// 三池均空才亮恢复提示；增强池也参与，并保留原有符号整数语义。
    fn stamina_empty(&self) -> bool {
        self.stamina_normal <= 0 && self.stamina_boost <= 0 && self.stamina_enhance < 1
    }
}

fn env_i32(name: &str, default: i32) -> i32 {
    match std::env::var(name) {
        Ok(raw) => raw.trim().parse().unwrap_or_else(|_| {
            warn!("[menu_dialog] mock 面板：{name}={raw:?} 不是有符号整数，回默认 {default}");
            default
        }),
        Err(_) => default,
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    match std::env::var(name) {
        Ok(raw) => match raw.trim().parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                warn!("[menu_dialog] mock 面板：{name}={raw:?} 不是非负整数，回默认 {default}");
                default
            }
        },
        Err(_) => default,
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => match raw.trim() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => {
                warn!("[menu_dialog] mock 面板：{name}={raw:?} 不是 true/false，回默认 {default}");
                default
            }
        },
        Err(_) => default,
    }
}

impl Default for MenuMock {
    fn default() -> Self {
        MenuMock {
            stamina_normal: env_i32("MOLY_MENU_MOCK_STAMINA_NORMAL", 120),
            stamina_enhance: env_i32("MOLY_MENU_MOCK_STAMINA_ENHANCE", 0),
            stamina_boost: env_i32("MOLY_MENU_MOCK_STAMINA_BOOST", 0),
            stamina_max: env_u32("MOLY_MENU_MOCK_STAMINA_MAX", 240),
            rank_level: env_u32("MOLY_MENU_MOCK_RANK_LEVEL", 3),
            rank_exp_next: env_u32("MOLY_MENU_MOCK_RANK_EXP_NEXT", 450),
            visiting: env_bool("MOLY_MENU_MOCK_VISITING", false),
            photo_shot_allowed: env_bool("MOLY_MENU_MOCK_PHOTO_SHOT_ALLOWED", true),
            crystal_shop_allowed: env_bool("MOLY_MENU_MOCK_CRYSTAL_SHOP_ALLOWED", true),
            sketch_available: env_bool("MOLY_MENU_MOCK_SKETCH_AVAILABLE", true),
        }
    }
}

// ---------------------------------------------------------------------------
// 实体与资源
// ---------------------------------------------------------------------------

/// Dialog view lifetime includes the closing slide after the open request clears.
#[derive(Component)]
pub(crate) struct MenuDialogRoot {
    binding: MenuSlideBinding,
    phase: MenuSlidePhase,
    requested_open: bool,
    elapsed: f32,
    slide_width: f32,
    canvas: Option<Vec2>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuSlidePhase {
    Hidden,
    Opening,
    Open,
    Closing,
}

/// Keep identities, not a parallel menu hierarchy or a hard-coded travel width.
struct MenuSlideBinding {
    animated: String,
    content: String,
    back_panel: String,
    content_size_delta: Vec2,
    authored_panel_size_delta: Vec2,
    slide_offset: Vec2,
}

impl MenuSlideBinding {
    fn from_document(doc: &UiPrefab) -> Self {
        fn reference(component: &UiComponent, field: &str) -> i64 {
            let value = &component.fields[field];
            assert_eq!(value[0].as_i64(), Some(0), "menu reference must be local: {field}");
            value[1].as_i64().filter(|id| *id != 0)
                .unwrap_or_else(|| panic!("menu reference missing: {field}"))
        }
        fn component(doc: &UiPrefab, id: i64, class: &str) -> usize {
            let node = doc.find(&format!("@{id}")).unwrap_or_else(|error| panic!("{error}"));
            assert!(doc.nodes[node].components.iter().any(|c| c.path_id == id && c.class == class),
                "menu component {id} is not {class}");
            node
        }
        let menu = doc.nodes.iter().flat_map(|node| &node.components)
            .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuDialog")
            .expect("menu must contain its presenter binding");
        let animation_id = reference(menu, "animation");
        let animation_node = component(doc, animation_id, "Sekai.SubWindowSlideAnimation");
        let animation = doc.nodes[animation_node].components.iter()
            .find(|c| c.path_id == animation_id).unwrap();
        assert_eq!(animation.fields["slideType"].as_i64(), Some(0),
            "menu slide must move the complete window");
        let window_id = reference(animation, "subWindowComponent");
        assert_eq!(window_id, reference(menu, "subWindowComponent"),
            "menu presenter and animation must bind the same subwindow");
        let window_node = component(doc, window_id, "Sekai.SubWindowComponent");
        let window = doc.nodes[window_node].components.iter().find(|c| c.path_id == window_id).unwrap();
        assert_eq!(window.fields["windowType"].as_i64(), Some(2),
            "menu subwindow must enter from the right");
        let content = format!("@{}", reference(window, "contentTransform"));
        let back_panel = format!("@{}", reference(window, "backPanelTransform"));
        let content_node = doc.find(&content).unwrap_or_else(|error| panic!("{error}"));
        let panel_node = doc.find(&back_panel).unwrap_or_else(|error| panic!("{error}"));
        let offset = &animation.fields["slideOffset"];
        let slide_offset = Vec2::new(
            offset[0].as_f64().expect("menu slideOffset.x must be present") as f32,
            offset[1].as_f64().expect("menu slideOffset.y must be present") as f32,
        );
        Self {
            animated: format!("@{}", doc.nodes[window_node].transform_id),
            content,
            back_panel,
            content_size_delta: Vec2::from_array(doc.nodes[content_node].rect.size_delta),
            authored_panel_size_delta: Vec2::from_array(doc.nodes[panel_node].rect.size_delta),
            slide_offset,
        }
    }

    fn setup_width(&self, view: &mut UiPrefabView, layouts: &UiLayouts, canvas: Vec2) -> f32 {
        // Setup starts from the authored hierarchy. Resetting the cached view
        // avoids feeding the previous window instance's stretched size back in.
        view.set_anchored_position(&self.animated, Vec2::ZERO);
        view.set_size_delta(&self.back_panel, self.authored_panel_size_delta);
        let content = view.rect(layouts, &self.content, canvas).expect("menu content geometry missing");
        let panel = view.rect(layouts, &self.back_panel, canvas).expect("menu back panel geometry missing");
        let content_position = content.world.transform_point3(Vec3::ZERO);
        let panel_position = panel.world.transform_point3(Vec3::ZERO);
        let width = panel_position.x - content_position.x + self.content_size_delta.x;
        view.set_size_delta(&self.back_panel, Vec2::new(width, panel.size.y));
        width + self.slide_offset.x
    }
}

/// 铺装闩（视图一次铺成后置）。
#[derive(Resource, Default)]
pub(crate) struct MenuDialogSpawned;

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

pub(crate) fn init(mut commands: Commands) {
    commands.init_resource::<MenuMock>();
}

// ---------------------------------------------------------------------------
// Update：铺件（图集到齐一次）
// ---------------------------------------------------------------------------

/// Bind the presenter and animation to their serialized component references.
pub(crate) fn spawn_when_ready(
    mut commands: Commands, layouts: Res<crate::ui_layout::UiLayouts>,
    server: Res<AssetServer>, spawned: Option<Res<MenuDialogSpawned>>,
) {
    if spawned.is_some() || !layouts.ready("Menu", &server) { return; }
    let binding = MenuSlideBinding::from_document(layouts.document("Menu").expect("menu layout missing"));
    commands.spawn((MenuDialogRoot {
            binding, phase: MenuSlidePhase::Hidden, requested_open: false,
            elapsed: 0., slide_width: 0., canvas: None,
        }, Visibility::Hidden, Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER), crate::ui_layout::UiPrefabView::new("Menu", SITEMAP_LAYER)));
    commands.insert_resource(MenuDialogSpawned);
}

// ---------------------------------------------------------------------------
// Update：摆位与开关沿
// ---------------------------------------------------------------------------

/// 开框沿（Setup 装配序的日志同形：八钮 Setup → exit 置亮 → 图标/等级/
/// 体力三读数 → 三监听）。逐钮报源使能与目标已建两层门——不合成一个数。
fn on_open(mock: &MenuMock) {
    info!(
        "[menu_dialog] 开框：DialogUtility.ShowMysekaiMenu → TryGetActiveDialog(菜单对话框) 未开 \
         ⇒ InstantiateDialog(309, Dialog 槽) → Initialize(allowCloseExternal=true) → Setup(ViewData)"
    );
    info!(
        "[menu_dialog] Setup·ViewData 派生：IsOwner()=true（单机恒站主）· \
         ShouldEnableInventoryButton=!IsVisiting()={} · ShouldEnableInfoButton=!IsVisiting()={} \
         （访客态为面板下发 mock 值）",
        !mock.visiting, !mock.visiting
    );
    for which in ALL_BUTTONS {
        let source = which.source_enabled(mock);
        let built = which.target_built();
        info!(
            "[menu_dialog]   钮「{}」：源使能={} · 目标已建={} · 可点击={} —— 路由：{}",
            which.label(),
            source,
            built,
            source && built,
            which.route_target()
        );
    }
    info!(
        "[menu_dialog] Setup·两格读值（服务端态，mock 面板下发）：体力格 \
         UserMysekaiStamina{{normal={}, enhance={}, boost={}}} 合计 {}/{}（量表上限为 master 派生 \
         mock）· 恢复提示={}（IsEmptyStamina 律）· 等级格 UserMysekaiGamedata.totalExp 派生 \
         等级 {} 距下一级 {}exp（master 表 mock 派生）——普查记的「宝石余额」一格在真源字段表 \
         里不存在（具名修正，见模块头）",
        mock.stamina_normal,
        mock.stamina_enhance,
        mock.stamina_boost,
        mock.stamina_sum(),
        mock.stamina_max,
        mock.stamina_empty(),
        mock.rank_level,
        mock.rank_exp_next
    );
}

/// 关框沿（CloseProcess 的同形日志）。
fn on_close() {
    info!(
        "[menu_dialog] 关框：HideAsync，窗口向右滑出（0.2s，OutQuart）；\
         SE_SUBWINDOW_CLOSE 的源音频尚未解析，不代播其他 cue"
    );
}

/// Update：摆位与逐帧状态。每帧——
/// 1. 开关沿（开框沿逐钮报路由表 + 两格读值；关框沿一行）；
/// 2. 根可见性包含滑出阶段；根缩放 = canvas 缩放；
/// 3. 逐件：可按件底框色按「可点击 = 源使能 ∧ 目标已建」摆置灰态，
///    文案变了整组重建；显示件同（恢复提示随体力空律）。
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>, mut dialog: ResMut<ShellDialogState>, mock: Res<MenuMock>,
    layouts: Res<UiLayouts>, time: Res<Time>,
    mut roots: Query<(&mut MenuDialogRoot, &mut Visibility, &mut Transform, &mut UiPrefabView)>,
) {
    let Ok(window)=windows.single() else {return;};
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. { return; }
    let scale = canvas_scale(size.x, size.y);
    let canvas = size / scale;
    for (mut root,mut visibility,mut transform,mut view) in &mut roots {
        let open = dialog.menu_open;
        let changed = root.requested_open != open;
        if changed {
            root.requested_open = open;
            root.elapsed = 0.;
            root.phase = if open {
                on_open(&mock);
                dialog.menu_closing = false;
                MenuSlidePhase::Opening
            } else {
                on_close();
                dialog.menu_closing = true;
                MenuSlidePhase::Closing
            };
        }
        if root.phase == MenuSlidePhase::Hidden {
            *visibility = Visibility::Hidden;
            dialog.menu_closing = false;
            continue;
        }
        if (changed && open) || root.canvas != Some(canvas) {
            let slide_width = root.binding.setup_width(&mut view, &layouts, canvas);
            root.slide_width = slide_width;
            root.canvas = Some(canvas);
        }
        if !changed && matches!(root.phase, MenuSlidePhase::Opening | MenuSlidePhase::Closing) {
            root.elapsed = (root.elapsed + time.delta_secs()).min(MENU_SLIDE_DURATION);
            if root.elapsed >= MENU_SLIDE_DURATION {
                root.phase = if open { MenuSlidePhase::Open } else { MenuSlidePhase::Hidden };
                dialog.menu_closing = false;
            }
        }
        let t = (root.elapsed / MENU_SLIDE_DURATION).clamp(0., 1.);
        let out_quart = 1. - (1. - t).powi(4);
        let offset = match root.phase {
            MenuSlidePhase::Opening => root.slide_width * (1. - out_quart),
            MenuSlidePhase::Open => 0.,
            MenuSlidePhase::Closing => root.slide_width * out_quart,
            MenuSlidePhase::Hidden => root.slide_width,
        };
        view.set_anchored_position(&root.binding.animated, Vec2::new(offset, 0.));
        *visibility=if root.phase != MenuSlidePhase::Hidden {Visibility::Inherited}else{Visibility::Hidden};
        transform.scale=Vec3::splat(scale);
        for (button,texture) in [
            (MenuButton::AvatarChange,"btn_cheki_menu_avatar"),(MenuButton::PhotoAlbum,"btn_cheki_menu_album"),
            (MenuButton::ExitMysekai,"btn_mysekai_menu_home"),(MenuButton::Chest,"btn_cheki_menu_item"),
            (MenuButton::CrystalShop,"btn_cheki_menu_crystalShop"),(MenuButton::Info,"btn_cheki_menu_mySekaiInfo"),
            (MenuButton::WhiteBlueprintSketch,"btn_cheki_menu_sketch"),(MenuButton::PhotoShot,"btn_cheki_menu_photo"),
            (MenuButton::Exchange,"btn_mysekai_menu_change"),
        ] {
            view.set_texture(button.source_path(),texture);
            view.set_visible(&format!("{}/Cover",button.source_path()),!button.source_enabled(&mock)||!button.target_built());
        }
        view.set_visible("MenuRoot/TitleCell",false);
        view.set_visible("MenuHeader/bg/info",mock.stamina_empty());
        let rank="MenuHeader/bg/UIPartsMySekaiRankGauge";
        view.set_text(&format!("{rank}/CustomTextMesh (2)"),mock.rank_level.to_string());
        let remaining=layouts.wordings.get("W_C_713").cloned().unwrap_or_default().replace("{0}",&mock.rank_exp_next.to_string());
        view.set_text(&format!("{rank}/CustomTextMesh (3)"),remaining);
    }
}

// ---------------------------------------------------------------------------
// Update：点按（模态先手）
// ---------------------------------------------------------------------------

/// 点按屏位（顶为原点）→ 参照画布坐标。
fn to_canvas(position: Vec2, width: f32, height: f32, scale: f32) -> Vec2 {
    Vec2::new(position.x - width / 2.0, height / 2.0 - position.y) / scale
}

fn hit_test(canvas: Vec2, which: MenuButton, layouts: &crate::ui_layout::UiLayouts, view: &crate::ui_layout::UiPrefabView, size:Vec2) -> bool {
    view.rect(layouts,which.source_path(),size).is_some_and(|rect| rect.active && rect.contains(canvas))
}

/// 点按分派：对话框开着 ⇒ 模态（全部点按先过它，真源 Dialog 槽
/// blockRaycasts 同形；外壳与小地图的点按门都读同一个 menu_open 位）。
/// 框外点按收框（allowCloseExternal=true）。已知偏差与外壳确认框同款：
/// 动作按钮的点按消费先于本模块（它不读对话框态，既有具名挂账），本
/// 模块不动它的判定面。
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut dialog: ResMut<ShellDialogState>,
    mock: Res<MenuMock>,
    layouts: Res<crate::ui_layout::UiLayouts>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut layer_commands: MessageWriter<LayerCommand>,
    views: Query<&crate::ui_layout::UiPrefabView, With<MenuDialogRoot>>,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() || !(dialog.menu_open || dialog.menu_closing) {
        return;
    }
    if dialog.menu_closing {
        consumed.0 = true;
        return;
    }
    let Ok(view) = views.single() else { return; };
    let Ok(window) = windows.single() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = canvas_scale(width, height);
    let mut close_requested = false;
    for position in &taps {
        // 模态：这一下被对话框吃掉（世界射线与外壳件都拿不到）。
        consumed.0 = true;
        let canvas = to_canvas(*position, width, height, scale);
        let hit = ALL_BUTTONS
            .into_iter()
            .find(|which| hit_test(canvas, *which, &layouts, view, Vec2::new(width,height)/scale));
        match hit {
            Some(MenuButton::Close) => {
                sounds.source_button(&layouts, view.key, MenuButton::Close.source_path());
                info!(
                    "[menu_dialog] 关闭钮按下 → {}——开始窗口滑出",
                    MenuButton::Close.route_target()
                );
                close_requested = true;
            }
            Some(which) => {
                let source = which.source_enabled(&mock);
                let built = which.target_built();
                if source && built {
                    sounds.source_button(&layouts, view.key, which.source_path());
                    // 已建目标：走真路由（当前唯一：情报层）。
                    match which {
                        MenuButton::Info => {
                            // 真源 OnClickMysekaiInfoButton：先查已开 622
                            // （层栈同层重复压丢弃同形）→ 压层 → Close。
                            info!(
                                "[menu_dialog] 情报钮按下 → TryMoveScreenLayerMysekaiInfo → \
                                 层栈压层（已开即丢弃，真源 IsActiveScreen(622) 同形）→ 关框"
                            );
                            layer_commands.write(LayerCommand::Push(LayerId::MysekaiInfo));
                            close_requested = true;
                        }
                        // 已建目标路由臂当前唯一：情报（见 target_built）。
                        _ => unreachable!("已建目标路由臂唯一：情报"),
                    }
                } else {
                    // 置灰钮点按不响应（真源里 enabled=false 的钮同样不
                    // 派发；本仓叠加门：源使能真而目标未建的也置灰）。
                    let reason = if !source {
                        "源使能=false（user 态派生，mock 面板）"
                    } else {
                        "路由目标未建（具名挂账）"
                    };
                    info!(
                        "[menu_dialog] 钮「{}」置灰，点按不响应（{reason}）——路由：{}",
                        which.label(),
                        which.route_target()
                    );
                }
            }
            None if view.rect(&layouts, "Window/ContentRoot", Vec2::new(width,height)/scale).is_some_and(|r|r.active && r.contains(canvas)) => {}
            None => {
                // 框外点按收框（allowCloseExternal=true，Initialize 第三参
                // 原生核过）。
                info!("[menu_dialog] 框外点按收框（模态；allowCloseExternal=true）");
                close_requested = true;
            }
        }
    }
    if close_requested {
        dialog.menu_open = false;
        dialog.menu_closing = true;
    }
}

