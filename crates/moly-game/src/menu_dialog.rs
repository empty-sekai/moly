//! 菜单对话框（真源 `MysekaiMenuDialog`，DialogType 312（JP 6.8.1 的枚举值），Dialog 槽）——
//! 场地屏外壳菜单钮的目标：体力/等级两格 + 12 个可按件的导航中枢。
//!
//! ## 入口与身份（Dialog 槽，不是层栈）
//!
//! 外壳菜单钮 `ShowMenu` → `DialogUtility.ShowMysekaiMenu(onChangeSite)`：
//! 先 `TryGetActiveDialog(MysekaiMenuDialog, Layer_Dialog)`（已开 ⇒ 只
//! 重新 Setup，不重复实例化），否则 `InstantiateDialog(312, Layer_Dialog)`
//! → `Initialize(messageBody=null, onClose=null, allowCloseExternal=true)`
//! → SetActive(false) → `Setup(ViewData)` → `Open`。**框外点按收框成立**
//! （allowCloseExternal = true，Initialize 第三参原生核过）。ViewData 的
//! `OnChangeSite` 在本对话框体内从不被调用（仅存字段，构造点穷举：仅
//! 两处读都是 ShouldEnable 两格）——本仓不设对应物，具名。
//!
//! 本仓经屏幕管理器走这条生命周期：开框 `show_dialog(MysekaiMenuDialog,
//! LayerDialog, DialogBackKey::Close)` + `open_dialog`，滑入结束
//! `dialog_open_finished`；收框 `close_dialog`，滑出结束
//! `dialog_destroyed`；管理器交来的返回键（`SubWindowDialog` 的
//! `OnHardwareBackKeyProcess` = `CloseProcess`）收框。
//!
//! ViewData 派生（原生 `ShowMysekaiMenu`）：`IsOwner()` ·
//! `ShouldEnableInventoryButton = !IsVisiting()` ·
//! `ShouldEnableInfoButton = !IsVisiting()`。单机（无多人域）⇒ 恒真；
//! 访客态走 mock 面板一格。
//!
//! ## The eleven buttons: source handler, source rule, product
//!
//! The dialog serializes exactly 11 `CustomButton` fields; the twelfth
//! tappable part is `DialogBase.closeButton`. Every handler but the two scene
//! transitions ends in the dialog's own `Close()`. `Setup` writes each
//! button's `enabled` once (a disabled `CustomButton` dispatches nothing and
//! `ShowCover` shows its grey cover: every one of these buttons is
//! `DisableActionType.Grayout` with its `Cover` child as the cover image).
//!
//! A button whose target has no counterpart in this product is greyed with
//! the target named. That is this product's adaptation, not the source: in
//! [`MenuButton::setup`] each button carries its source rule and, next to it,
//! the adaptation line "greyed until X exists". When X lands the adaptation
//! line is deleted and the button follows its source rule again. Buttons
//! whose target exists follow the source rule exactly. A tap on a greyed
//! button logs the reason.
//!
//! | button (field) | source handler and target | source enabled rule | product |
//! |---|---|---|---|
//! | recover stamina (`_recoverStaminaButton`) | `ScreenManager.Show2ButtonDialog<MysekaiRecoverBoostStaminaDialog>` (DialogType 321), its `Setup(onRecoverFinish = UpdateStaminaGateView)`, `Close` | `true` | greyed until the dialog has a view. The game carries its `Dialog/` prefab, so `ShowDialog` would return a dialog, not an error; no view of this product draws it and the UI root carries no layout of it, so `ShowDialog` is not called (an instance nobody draws would take the back key) |
//! | chest (`_chestButton`) | unless `IsActiveScreen(MysekaiInventory)`: `PushUIScreen(MysekaiInventory)` (607); `Close` | `ViewData.ShouldEnableInventoryButton` = `!IsVisiting()` | opens: the screen manager's `PushUIScreen` with this caller, no boot argument (the inventory screen, `crate::inventory`); the report to the multiplayer room has no domain here |
//! | info (`_mysekaiInfoButton`) | `MysekaiInfoUtility.TryMoveScreenLayerMysekaiInfo`: unless `IsActiveScreen(MysekaiInfo)`, `PushUIScreen(MysekaiInfo, ScreenLayerMysekaiInfoBootData)` (622); `Close` | `ViewData.ShouldEnableInfoButton` = `!IsVisiting()` | opens: the screen manager's `PushUIScreen` with this caller |
//! | avatar change (`_mysekaiAvtarChangeButton`, the source's spelling) | unless active: `PushUIScreen(MysekaiAvatarCostumeSetting)` (629); `Close` | `true` | greyed until a MysekaiAvatarCostumeSetting view exists (registered, no view) |
//! | back to title (`_transitionToTitleButton`, `TitleCell`) | `MysekaiUtility.TransitionToTitle`: `Disconnect`, then `SceneManager.RequestScene(Title)` | no write (the serialized `true`) | hidden, as in the source: `TitleCell` is inactive in the prefab and nothing activates it |
//! | leave MySekai (`_exitMysekaiButton`, `Home`) | `TransitionToOutGame`: once (`_isTransitionOutGame`), tap off, BGM stop and fade out, then `MysekaiUtility.TransitionToOutGame`: `Disconnect`, then `RequestScene(OutGame)` | `true` | greyed until an out-game scene exists (this product has only the MySekai scene) |
//! | photo (`_photoShotButton`) | unless in it: `GameStateManager.ChangeState(PhotoShot)`, whose `OnEnter` pushes MysekaiPhotoShot (632); `Close` | `MysekaiPhotoShotUtility.IsAllowedToPhotoShotInCurrentSite()` = `CurrentSiteType < grassland`, read from the active site | greyed until a MysekaiPhotoShot view exists (registered, no view) |
//! | album (`_photoAlbumButton`) | `MysekaiPhotoAlbumUtility.TryShowPhotoAlbumScreen`: unless active, `PushUIScreen(MysekaiPhotoAlbum)` (642); `Close` | no write (the serialized `true`) | greyed until a MysekaiPhotoAlbum view exists (registered, no view) |
//! | crystal shop (`_crystalShopButton`) | active: `Close`; else `BootArgObject`, `PushUIScreen(CrystalShop)` (89), `Close` | `MysekaiUtility.IsAllowedToOpenCrystalShop()` = not visiting a housing competition entry, and the room owner or an offline boot | greyed until a CrystalShop screen exists (outside this product's screen table) |
//! | sketch (`_whiteBlueprintSketchButton`) | unless in it: `Player.ChangeState(SelectSketchItem)`, `ChangeState(Sketch)`; `Close` (the owner-editing warning needs another player) | `SketchUtility.IsSketchAvailable()` | enters the sketch state (unchanged) |
//! | material exchange (`_exchangeButton`) | unless active: `BootArgObject = BootData(CallerSceneType.Mysekai)`, `PushUIScreen(MasterLessonMaterialExchange)` (50); `Close` | no write (the serialized `true`) | greyed until a MasterLessonMaterialExchange screen exists (outside this product's screen table) |
//! | close (`closeButton`) | `Close` = `CloseProcess`: `HideAsync`, `PlaySEOneShot("SE_SUBWINDOW_CLOSE")`, `Dispose` of the menu bundle | no write (the serialized `true`) | closes |
//!
//! The site map and the option dialog are not targets of this dialog: no
//! field reaches them, and `ViewData.OnChangeSite` is stored but never
//! invoked here.
//!
//! ## 两格取值链（+ 一处普查修正）
//!
//! - **体力格**（`UpdateStaminaGateView`，见 [`update_stamina_gate_view`]）：
//!   `HarvestUtility.GetStaminaData(0)` 读
//!   `UserDataManager.UserMysekaiStamina{normal,enhance,boost}`（**服务端
//!   用户态**，本仓为服务端模型的客户端副本）并算出 boost 存量
//!   `GetBoostStaminaStockCount`；`GetStaminaGageRate` 读 master 三池上限；
//!   `MysekaiStaminaView.UpdateStaminaView(体力, 存量, false)` 定量表色、
//!   渐变开关、图标精灵与色、存量底板与存量数字精灵；
//!   `UpdateStaminaGageRate(体力, 三率, false)` 定量表填充。体力空 ⇒
//!   `_recoverStaminaLabel` 亮（`IsEmptyStamina` = normal ≤ 0 且 boost ≤ 0
//!   且 enhance < 1）。缺口：`icon_stamina_boost_h40` 与
//!   `txt_stamina_2..9` 不在 UI 根的运行时精灵里（提取缺口，逐名报一次，
//!   保留序列化精灵）；`SetNativeSize` 随之未接；渐变图是量表图
//!   `Coffee.UISoftMask.SoftMask` 的子图形，软遮罩未移植，开着时画整张。
//! - **等级格**：`MysekaiRankModel` 读 `UserMysekaiGamedata.totalExp`
//!   （**服务端用户态**，mock 面板下发）→ 查 master 等级表（运行时根的
//!   `mysekai-ranks.json`，移植见 `moly_law::ui::mysekai_rank`）得等级、
//!   本级与下级累计经验，进 `UIPartsMysekaiRankGauge`。
//!   `UIPartsMysekaiRankGauge.Setup` 把等级经 `_rankText.SetText` 写成数字，
//!   再调 `_gaugeExp.Setup(等级, 最高等级, 总经验, 本级累计, 下级累计,
//!   "MSG_REST_VALUE")`：未到最高等级时剩余文字走
//!   `SetWordingText("MSG_REST_VALUE", [下级累计 - 总经验])`，到最高等级时走
//!   `SetWordingText("WORD_MAX")`；写进的是 `restTextMesh`（`restText` 为空
//!   时）。两区的客户端都是这两个键。量表 `UIPartsGauge.Setup(总经验 -
//!   本级累计, 下级累计 - 本级累计)`（到最高等级时 `(0, 1)`）把比值写进
//!   `fillImage` 的 fillAmount。那张图是一个 stencil `Mask` 的图形，量表
//!   条是它的子节点：由 UI 渲染器的 stencil 消费者按遮罩图形的覆盖裁切
//!   （`ui_layout/stencil_mask.rs`）。量表条上的 `UiEffect.GradientColor`
//!   未解码（提取缺口），条仍是白色。
//! - ⚠ **普查的「宝石余额」一格在真源里不存在**：字段表穷举无 jewel
//!   字段、原生树本文件 `grep -ic jewel` = 0 ⇒ 三格修正为两格。宝石是
//!   恢复对话框的消耗货币，不在本对话框的显示面。
//! - ⚠ 「体力视图直接持 `ScreenLayerMysekaiHUD` 引用回写」也查无字段
//!   （原生 HUD grep = 0）：恢复完成后的体力回写走**回调**——
//! `Show2ButtonDialog` 时传 `onRecoverFinish = UpdateStaminaGateView`。
//!   恢复对话框未建 ⇒ 回调对应物具名挂账（建它时接同一条回调律）。
//!
//! The open line reports both layers for each button (the source rule and
//! the adaptation) and does not merge them into one number.
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
//! ## Server state
//!
//! The stamina triple is the client's copy of `UserMysekaiStamina`
//! (`crate::server::ClientUserData`, set by the server model's responses:
//! the join, the harvest and gather replies), the same copy the harvest
//! login reads; the gauge maximum is the master normal `maxStamina`. Total
//! experience (`UserMysekaiGamedata.totalExp`) comes from the same server
//! document through `crate::mysekai_rank::UserTotalExp`. The visiting state
//! and the crystal-shop and sketch permissions stay named mock values of this
//! module (native instruments `MOLY_MENU_MOCK_VISITING`,
//! `MOLY_MENU_MOCK_CRYSTAL_SHOP_ALLOWED`, `MOLY_MENU_MOCK_SKETCH_AVAILABLE`;
//! game mode reads none). The photo permission is not server state: it is the
//! current site's type, read from the active site. The stamina and
//! total-experience instruments land in the native server document.
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
//! - When the menu finishes opening, one line lists each button's centre in
//!   window pixels (the tap map for driving the menu with real taps).
//!
//! Named gaps (not implemented here):
//! - The seven greyed targets of the table above, and the title scene behind
//!   the hidden title button. When the recover dialog lands, its
//!   `onRecoverFinish` calls [`update_stamina_gate_view`].
//! - `SE_SUBWINDOW_CLOSE`. `SetupTutorial` (the tutorial is out of scope) and
//!   `SetupForBirthday` (no birthday context runs in this product).


use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, MenuScreenType, ScreenManager,
};
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

    /// The source field of the button.
    fn field(self) -> &'static str {
        match self {
            MenuButton::RecoverStamina => "_recoverStaminaButton",
            MenuButton::Chest => "_chestButton",
            MenuButton::Info => "_mysekaiInfoButton",
            MenuButton::AvatarChange => "_mysekaiAvtarChangeButton",
            MenuButton::TransitionToTitle => "_transitionToTitleButton",
            MenuButton::ExitMysekai => "_exitMysekaiButton",
            MenuButton::PhotoShot => "_photoShotButton",
            MenuButton::PhotoAlbum => "_photoAlbumButton",
            MenuButton::CrystalShop => "_crystalShopButton",
            MenuButton::WhiteBlueprintSketch => "_whiteBlueprintSketchButton",
            MenuButton::Exchange => "_exchangeButton",
            MenuButton::Close => "closeButton",
        }
    }

    /// `Setup`'s `enabled` write for this button: the source rule, then this
    /// product's adaptation next to it. Deleting an adaptation line (setting
    /// `greyed_until: None`) returns the button to its source rule; its
    /// route then goes into [`click`]. The tutorial and birthday overrides
    /// (`SetupTutorial`, `SetupForBirthday`) are not ported.
    fn setup(self, inputs: &SetupInputs) -> ButtonSetup {
        match self {
            MenuButton::RecoverStamina => ButtonSetup {
                rule: "SetupRecoverStaminaButton: enabled = true",
                source: true,
                // Adaptation: greyed until the MysekaiRecoverBoostStaminaDialog view exists.
                greyed_until: Some(Missing::Dialog(RECOVER_BOOST_STAMINA_DIALOG)),
            },
            MenuButton::Chest => ButtonSetup {
                rule: "SetupChestButton: enabled = ViewData.ShouldEnableInventoryButton = !IsVisiting()",
                source: !inputs.visiting,
                greyed_until: None,
            },
            MenuButton::Info => ButtonSetup {
                rule: "SetupMysekaiInfoButton: enabled = ViewData.ShouldEnableInfoButton = !IsVisiting()",
                source: !inputs.visiting,
                greyed_until: None,
            },
            MenuButton::AvatarChange => ButtonSetup {
                rule: "SetupAvatarChangeButton: enabled = true",
                source: true,
                // Adaptation: greyed until the MysekaiAvatarCostumeSetting view exists.
                greyed_until: Some(Missing::Screen(MenuScreenType::MysekaiAvatarCostumeSetting)),
            },
            MenuButton::TransitionToTitle => ButtonSetup {
                rule: "no Setup write: the serialized enabled (true); TitleCell stays inactive",
                source: true,
                // Adaptation: greyed until the title scene exists (the cell is
                // hidden in the source, so this only speaks if it is shown).
                greyed_until: Some(Missing::Scene("Title")),
            },
            MenuButton::ExitMysekai => ButtonSetup {
                rule: "Setup: _exitMysekaiButton.enabled = true",
                source: true,
                // Adaptation: greyed until the out-game scene exists.
                greyed_until: Some(Missing::Scene("OutGame")),
            },
            MenuButton::PhotoShot => ButtonSetup {
                rule: "SetupMysekaiPhotoShotButton: enabled = IsAllowedToPhotoShotInCurrentSite() = CurrentSiteType < grassland",
                source: inputs.photo_shot_allowed,
                // Adaptation: greyed until the MysekaiPhotoShot view exists.
                greyed_until: Some(Missing::Screen(MenuScreenType::MysekaiPhotoShot)),
            },
            MenuButton::PhotoAlbum => ButtonSetup {
                rule: "SetupMysekaiPhotoAlbumButton: no enabled write (the serialized true)",
                source: true,
                // Adaptation: greyed until the MysekaiPhotoAlbum view exists.
                greyed_until: Some(Missing::Screen(MenuScreenType::MysekaiPhotoAlbum)),
            },
            MenuButton::CrystalShop => ButtonSetup {
                rule: "SetupCrystalShopButton: enabled = IsAllowedToOpenCrystalShop()",
                source: inputs.crystal_shop_allowed,
                // Adaptation: greyed until the CrystalShop screen exists.
                greyed_until: Some(Missing::OutGameScreen("CrystalShop", 89)),
            },
            MenuButton::WhiteBlueprintSketch => ButtonSetup {
                rule: "SetupMysekaiWhiteBluePrintSketchButton: enabled = SketchUtility.IsSketchAvailable()",
                source: inputs.sketch_available,
                greyed_until: None,
            },
            MenuButton::Exchange => ButtonSetup {
                rule: "no Setup write: the serialized enabled (true)",
                source: true,
                // Adaptation: greyed until the MasterLessonMaterialExchange screen exists.
                greyed_until: Some(Missing::OutGameScreen("MasterLessonMaterialExchange", 50)),
            },
            MenuButton::Close => ButtonSetup {
                rule: "DialogBase.closeButton: no Setup write (the serialized true)",
                source: true,
                greyed_until: None,
            },
        }
    }

    /// The source handler and its target (the open and tap lines).
    fn handler(self) -> &'static str {
        match self {
            MenuButton::RecoverStamina => "OnClickRecoverBoostStaminaButton: Show2ButtonDialog<MysekaiRecoverBoostStaminaDialog>, Setup(onRecoverFinish), Close",
            MenuButton::Chest => "OnClickChestButton: unless IsActiveScreen(MysekaiInventory), PushUIScreen(MysekaiInventory); Close",
            MenuButton::Info => "OnClickMysekaiInfoButton: MysekaiInfoUtility.TryMoveScreenLayerMysekaiInfo; Close",
            MenuButton::AvatarChange => "OnClickAvtarChangeButton: unless IsActiveScreen(MysekaiAvatarCostumeSetting), PushUIScreen(MysekaiAvatarCostumeSetting); Close",
            MenuButton::TransitionToTitle => "TransitionToTitle: MysekaiUtility.TransitionToTitle (Disconnect, RequestScene(Title))",
            MenuButton::ExitMysekai => "TransitionToOutGame: fade out, MysekaiUtility.TransitionToOutGame (Disconnect, RequestScene(OutGame))",
            MenuButton::PhotoShot => "OnClickMysekaiPhotoShotButton: unless in it, ChangeState(GameStateType.PhotoShot) (OnEnter pushes MysekaiPhotoShot); Close",
            MenuButton::PhotoAlbum => "OnClickMysekaiPhotoAlbumButton: MysekaiPhotoAlbumUtility.TryShowPhotoAlbumScreen (PushUIScreen(MysekaiPhotoAlbum)); Close",
            MenuButton::CrystalShop => "OnClickMysekaiCrystalShopButton: unless IsActiveScreen(CrystalShop), BootArgObject, PushUIScreen(CrystalShop); Close",
            MenuButton::WhiteBlueprintSketch => "OnClickMysekaiWhiteBluePrintSketchButton: unless in it, Player.ChangeState(SelectSketchItem), ChangeState(GameStateType.Sketch); Close",
            MenuButton::Exchange => "OnClickExchangeButton: unless IsActiveScreen(MasterLessonMaterialExchange), BootData(Mysekai), PushUIScreen(MasterLessonMaterialExchange); Close",
            MenuButton::Close => "Close: CloseProcess (HideAsync, SE_SUBWINDOW_CLOSE, Dispose of the menu bundle)",
        }
    }
}

/// `DialogType.MysekaiRecoverBoostStaminaDialog`.
const RECOVER_BOOST_STAMINA_DIALOG: DialogType = DialogType(321);

/// A source target with no counterpart in this product: the subject of a
/// greyed button's adaptation.
#[derive(Debug, Clone, Copy)]
enum Missing {
    /// A MySekai screen id that no view of this product draws.
    Screen(MenuScreenType),
    /// A `MenuScreenType` outside the MySekai block (its source name and
    /// value); the product's screen table holds the MySekai block only.
    OutGameScreen(&'static str, u16),
    /// A dialog type that no view of this product draws.
    Dialog(DialogType),
    /// A `SceneManager.Scene` other than MySekai.
    Scene(&'static str),
}

impl Missing {
    /// The named reason, read against the screen and dialog registries.
    fn reason(self) -> String {
        let table = |screen: MenuScreenType| {
            if screen.registered() {
                "registered in the screen table, but no view of this product draws it"
            } else {
                "not in this product's screen table, and no view of this product draws it"
            }
        };
        match self {
            Missing::Screen(screen) => format!("{screen:?} is {}", table(screen)),
            Missing::OutGameScreen(name, id) => format!("{name}({id}) is {}", table(MenuScreenType(id))),
            Missing::Dialog(dialog) => match dialog.prefab() {
                Some(prefab) => format!(
                    "{dialog:?}: the game carries {prefab}, so ShowDialog would return a dialog, but no view of this product draws it; ShowDialog is not called"
                ),
                None => format!("{dialog:?}: ShowDialog returns an error (no Dialog/ prefab in the game's Resources)"),
            },
            Missing::Scene(scene) => format!("SceneManager.Scene.{scene} is not in this product (it has only the MySekai scene)"),
        }
    }
}

/// One button's `Setup` result.
#[derive(Debug, Clone, Copy)]
struct ButtonSetup {
    /// The source's enabled rule, as written.
    rule: &'static str,
    /// The value the source rule gives.
    source: bool,
    /// This product's adaptation: greyed until this target exists.
    greyed_until: Option<Missing>,
}

impl ButtonSetup {
    /// Enabled in this product: the source rule, unless the adaptation greys it.
    fn enabled(&self) -> bool {
        self.source && self.greyed_until.is_none()
    }
}

/// `Setup`'s inputs: the named mock values (server state) and the current
/// site's photo permission.
struct SetupInputs {
    visiting: bool,
    photo_shot_allowed: bool,
    crystal_shop_allowed: bool,
    sketch_available: bool,
}

impl SetupInputs {
    fn read(mock: &MenuMock, site: Option<&crate::site::SiteActive>) -> Self {
        SetupInputs {
            visiting: mock.visiting,
            photo_shot_allowed: photo_shot_allowed_in(site),
            crystal_shop_allowed: mock.crystal_shop_allowed,
            sketch_available: mock.sketch_available,
        }
    }
}

/// `MysekaiPhotoShotUtility.IsAllowedToPhotoShotInCurrentSite`:
/// `SiteManager.CurrentSiteType < MysekaiSiteType.grassland`: the home site
/// and the three floors allow it, the five site types from grassland on do
/// not.
fn photo_shot_allowed_in(site: Option<&crate::site::SiteActive>) -> bool {
    const GRASSLAND: u32 = 4;
    let Some(site) = site else {
        error!("[menu_dialog] IsAllowedToPhotoShotInCurrentSite: no site is active; the photo button stays disabled");
        return false;
    };
    let site_type = moly_law::carve::site_type_value(&site.site_type)
        .unwrap_or_else(|| panic!("[menu_dialog] site type {} is not a MysekaiSiteType", site.site_type));
    site_type < GRASSLAND
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

/// The menu dialog's named mock enable inputs. The stamina is the client's
/// copy of the server model (`crate::server::ClientUserData`); total
/// experience is [`crate::mysekai_rank::UserTotalExp`]; the rank table is
/// master data from the runtime root.
#[derive(Resource)]
pub(crate) struct MenuMock {
    /// `MysekaiMultiplayController.IsVisiting()`——多人访客态（使能输入；
    /// 本仓无多人域，默认 false）。
    visiting: bool,
    /// `MysekaiUtility.IsAllowedToOpenCrystalShop()`（默认 true：单机恒站主）。
    crystal_shop_allowed: bool,
    /// `SketchUtility.IsSketchAvailable()`（master+user 道具查；默认 true）。
    sketch_available: bool,
}

/// `StaminaData.SumStamina` of the client's copy (normal + enhance + boost).
fn stamina_sum(stamina: crate::server::Stamina) -> i32 {
    stamina
        .normal
        .wrapping_add(stamina.enhance)
        .wrapping_add(stamina.boost)
}

// ---------------------------------------------------------------------------
// The stamina cell: `MysekaiMenuDialog.UpdateStaminaGateView`
// ---------------------------------------------------------------------------

/// The menu's one read of the stamina: the client's copy of
/// `UserMysekaiStamina` (`HarvestUtility.GetStaminaData` reads
/// `UserDataManager.UserMysekaiStamina`) and the master pool maxima
/// (`GetStaminaGageRate` and `GetBoostStaminaStockCount` read
/// `mysekaiStaminas`). Both come from the server model; this is the seam the
/// model switches.
fn menu_stamina(
    user: Option<&crate::server::ClientUserData>,
) -> Result<(crate::server::Stamina, crate::server::StaminaMax), &'static str> {
    let stamina = user
        .ok_or("the server model has made no response yet")?
        .stamina
        .ok_or("the server model has not seated the stamina pools")?;
    let max = crate::server::stamina_max().ok_or("the stamina masters are not loaded")?;
    Ok((stamina, max))
}

/// The processor's signed division: a zero divisor gives zero (`sdiv`), the
/// one overflow wraps.
fn sdiv(a: i32, b: i32) -> i32 {
    if b == 0 { 0 } else { a.wrapping_div(b) }
}

/// `HarvestUtility.GetBoostStaminaStockCount(boost)`: the boost pool over a
/// tenth of the boost master's `maxStamina` (C# integer division, both).
fn boost_stamina_stock_count(boost: i32, max: crate::server::StaminaMax) -> i32 {
    sdiv(boost, max.boost / 10)
}

/// `HarvestUtility.GetStaminaGageRate(staminaData)`: (normal / normal
/// maximum, enhance / enhance maximum, (boost mod the boost stock unit) /
/// unit), each quotient in single precision. The unit is a tenth of the boost
/// maximum (integer), the remainder is `boost - sdiv(boost, unit) * unit`.
fn stamina_gage_rate(stamina: crate::server::Stamina, max: crate::server::StaminaMax) -> (f32, f32, f32) {
    let unit = max.boost / 10;
    let rest = stamina.boost.wrapping_sub(sdiv(stamina.boost, unit).wrapping_mul(unit));
    (
        stamina.normal as f32 / max.normal as f32,
        stamina.enhance as f32 / max.enhance as f32,
        rest as f32 / unit as f32,
    )
}

/// What `MysekaiStaminaView.UpdateStaminaView(staminaData, stockCount,
/// isForceBoostView)` and `UpdateStaminaGageRate(staminaData, rates,
/// isBoostMode)` leave on the view.
#[derive(Debug, Clone, PartialEq)]
struct StaminaViewState {
    /// `_staminaGageImage.color`: white, or the palette entry
    /// [`STAMINA_GAUGE_COLOR_ENTRY`] when neither enhance nor boost is held.
    gauge_palette: bool,
    /// `_staminaGradiantImage.enabled`.
    gradient: bool,
    /// `_staminaIcon.SpriteName`.
    icon: &'static str,
    /// `_staminaIcon.color`: the palette entry
    /// [`STAMINA_EMPTY_ICON_COLOR_ENTRY`] when the stamina is empty, else white.
    icon_palette: bool,
    /// `_staminaGageImageOnStock.enabled`; `_staminaGageImageOnNoStock` is
    /// its opposite.
    on_stock: bool,
    /// `_boostStaminaCount`: enabled with sprite `txt_stamina_{n}`, or disabled.
    boost_count: Option<i32>,
    /// `_staminaGageImage.fillAmount`.
    fill: f32,
}

/// `SetStaminaGaugeColor`'s palette entry of a gauge without enhance or boost.
const STAMINA_GAUGE_COLOR_ENTRY: usize = 55;
/// `SetStaminaGaugeColor`'s palette entry of the icon of an empty stamina.
const STAMINA_EMPTY_ICON_COLOR_ENTRY: usize = 5;

/// `UpdateStaminaGateView`'s two view calls: `UpdateStaminaView(staminaData,
/// GetStaminaData's stock count, false)` and `UpdateStaminaGageRate(
/// staminaData, GetStaminaGageRate(staminaData), false)`.
fn stamina_view_state(stamina: crate::server::Stamina, max: crate::server::StaminaMax) -> StaminaViewState {
    let stock = boost_stamina_stock_count(stamina.boost, max);
    stamina_view_state_with(stamina, stock, stamina_gage_rate(stamina, max))
}

/// [`stamina_view_state`] with the stock count and the three rates given:
/// `UpdateStaminaView(staminaData, stock, false)` then
/// `UpdateStaminaGageRate(staminaData, normal, enhance, boost, false)`
/// (whose own stock count, from the same boost pool, equals `stock`).
fn stamina_view_state_with(
    stamina: crate::server::Stamina,
    stock: i32,
    (normal, enhance, boost): (f32, f32, f32),
) -> StaminaViewState {
    let force_boost_view = false;
    let is_boost_mode = false;
    // SetStaminaGaugeColor.
    let held = stamina.enhance > 0 || stamina.boost > 0 || force_boost_view;
    let empty = stamina.normal <= 0 && stamina.boost <= 0 && stamina.enhance < 1;
    let icon = if stamina.boost > 0 || force_boost_view {
        "icon_stamina_boost_h40"
    } else {
        "icon_stamina_normal_h40"
    };
    // UpdateStaminaView: the stock backs, then the count image, whose sprite
    // number stops at 9.
    let on_stock = stock >= 1;
    let boost_count = on_stock.then(|| stock.min(9));
    // UpdateStaminaGageRate: the stock count again from the boost pool; above
    // 9 the gauge is full, else the boost rate while boost is held, else the
    // enhance rate with enhance of at least 1, else the normal rate.
    let boosted = stamina.boost > 0 || is_boost_mode;
    let fill = if stock > 9 {
        1.0
    } else if boosted {
        boost
    } else if stamina.enhance >= 1 {
        enhance
    } else {
        normal
    };
    StaminaViewState {
        gauge_palette: !held,
        gradient: held,
        icon,
        icon_palette: empty && !force_boost_view,
        on_stock,
        boost_count,
        fill,
    }
}

/// The `MysekaiStaminaView` the dialog's `_staminaView` references, and the
/// dialog's `_recoverStaminaLabel`, as view selectors.
struct StaminaTargets {
    gauge: i64,
    gradient: String,
    icon: i64,
    on_stock: String,
    on_no_stock: String,
    boost_count: i64,
    boost_count_node: String,
    recover_label: String,
}

impl StaminaTargets {
    fn from_document(doc: &UiPrefab) -> Self {
        let dialog = doc.nodes.iter().flat_map(|node| node.components.iter())
            .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuDialog")
            .unwrap_or_else(|| panic!("{}: no MysekaiMenuDialog component", doc.prefab));
        let view_id = reference(doc, &dialog.fields, "_staminaView");
        let view = doc.nodes.iter().flat_map(|node| node.components.iter())
            .find(|c| c.path_id == view_id && c.class == "Sekai.Mysekai.MysekaiStaminaView")
            .unwrap_or_else(|| panic!("{}: _staminaView {view_id} is not a MysekaiStaminaView", doc.prefab));
        // A Graphic's `enabled` as its GameObject's active flag: each of
        // these nodes carries only that Graphic and has no children.
        let graphic_node = |field: &str| -> String {
            let id = reference(doc, &view.fields, field);
            let index = doc.find(&format!("@{id}")).unwrap_or_else(|error| panic!("{error}"));
            let node = &doc.nodes[index];
            assert!(
                doc.nodes.iter().all(|n| n.parent_transform_id != node.transform_id),
                "{}: MysekaiStaminaView {field} is not a leaf node", doc.prefab
            );
            format!("@{}", node.game_object_id)
        };
        StaminaTargets {
            gauge: reference(doc, &view.fields, "_staminaGageImage"),
            gradient: graphic_node("_staminaGradiantImage"),
            icon: reference(doc, &view.fields, "_staminaIcon"),
            on_stock: graphic_node("_staminaGageImageOnStock"),
            on_no_stock: graphic_node("_staminaGageImageOnNoStock"),
            boost_count: reference(doc, &view.fields, "_boostStaminaCount"),
            boost_count_node: graphic_node("_boostStaminaCount"),
            recover_label: format!("@{}", reference(doc, &dialog.fields, "_recoverStaminaLabel")),
        }
    }
}

/// `CustomImage.SpriteName = name` on an image: the runtime sprite of that
/// name when the UI root carries it; nothing to do when the serialized sprite
/// already is that sprite. Any other name is a sprite the root does not
/// carry, reported once.
fn set_sprite_name(view: &mut UiPrefabView, layouts: &UiLayouts, doc: &UiPrefab, image: i64, name: &str) {
    let path = format!("@{image}");
    if layouts.has_runtime_texture(name) {
        view.set_texture(&path, name);
        return;
    }
    let index = doc.find(&path).unwrap_or_else(|error| panic!("{error}"));
    let serialized = doc.nodes[index].components.iter().find(|c| c.path_id == image)
        .and_then(|c| c.sprite.as_ref()).and_then(|s| s["name"].as_str());
    if serialized != Some(name) {
        error_once!(
            "[menu_dialog] {}: SpriteName {name} on image @{image} is not among the UI root's runtime sprites; the serialized {serialized:?} stays",
            doc.prefab
        );
    }
}

/// `UpdateStaminaGateView` on the view (Setup; the recover dialog's finish
/// callback once that dialog exists).
fn update_stamina_gate_view(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    doc: &UiPrefab,
    user: Option<&crate::server::ClientUserData>,
) {
    let targets = StaminaTargets::from_document(doc);
    let (stamina, max) = match menu_stamina(user) {
        Ok(read) => read,
        Err(reason) => {
            error!("[menu_dialog] UpdateStaminaGateView: {reason}; the stamina cell keeps its serialized state");
            return;
        }
    };
    let state = stamina_view_state(stamina, max);
    info!(
        "[menu_dialog] UpdateStaminaGateView: {stamina:?} over maxima {max:?}, stock {} -> {state:?}",
        boost_stamina_stock_count(stamina.boost, max)
    );
    // PaletteUtility.GetColor: the palette is on a region root only; the
    // shared root carries none, so there the colour stays as serialized.
    let set_color = |view: &mut UiPrefabView, graphic: i64, entry: Option<usize>| {
        let color = match entry {
            None => [1.0; 4],
            Some(entry) => match layouts.palette_color(entry) {
                Some(color) => color,
                None => {
                    error_once!(
                        "[menu_dialog] {}: palette entry {entry} is not on this UI root; the stamina colours stay as serialized",
                        doc.prefab
                    );
                    return;
                }
            },
        };
        view.set_graphic_color(graphic, color);
    };
    set_color(view, targets.gauge, state.gauge_palette.then_some(STAMINA_GAUGE_COLOR_ENTRY));
    view.set_visible(&targets.gradient, state.gradient);
    set_sprite_name(view, layouts, doc, targets.icon, state.icon);
    set_color(view, targets.icon, state.icon_palette.then_some(STAMINA_EMPTY_ICON_COLOR_ENTRY));
    view.set_visible(&targets.on_stock, state.on_stock);
    view.set_visible(&targets.on_no_stock, !state.on_stock);
    view.set_visible(&targets.boost_count_node, state.boost_count.is_some());
    if let Some(count) = state.boost_count {
        set_sprite_name(view, layouts, doc, targets.boost_count, &format!("txt_stamina_{count}"));
    }
    view.set_fill(&format!("@{}", targets.gauge), state.fill);
    // _recoverStaminaLabel.SetActive(staminaData.IsEmptyStamina).
    view.set_visible(&targets.recover_label, stamina.normal <= 0 && stamina.boost <= 0 && stamina.enhance < 1);
}

/// A native instrument (game mode reads none).
fn env_bool(name: &str, default: bool) -> bool {
    match crate::server::instrument_env(name) {
        Some(raw) => match raw.trim() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => {
                warn!("[menu_dialog] mock 面板：{name}={raw:?} 不是 true/false，回默认 {default}");
                default
            }
        },
        None => default,
    }
}

impl Default for MenuMock {
    fn default() -> Self {
        MenuMock {
            visiting: env_bool("MOLY_MENU_MOCK_VISITING", false),
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
    /// The screen manager's instance of this dialog, from `ShowDialog` to
    /// its destruction after the close animation.
    dialog: Option<DialogId>,
    /// The last `Setup`'s button writes, in [`ALL_BUTTONS`] order.
    setup: Option<[ButtonSetup; 12]>,
}

impl MenuDialogRoot {
    /// The last `Setup`'s write for one button.
    fn button(&self, which: MenuButton) -> Option<ButtonSetup> {
        let index = ALL_BUTTONS.iter().position(|button| *button == which)?;
        self.setup.map(|setup| setup[index])
    }
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
            elapsed: 0., slide_width: 0., canvas: None, dialog: None, setup: None,
        }, Visibility::Hidden, Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER), crate::ui_layout::UiPrefabView::new("Menu", SITEMAP_LAYER)));
    commands.insert_resource(MenuDialogSpawned);
}

// ---------------------------------------------------------------------------
// Update：摆位与开关沿
// ---------------------------------------------------------------------------

/// 开框沿（Setup 装配序的日志同形：八钮 Setup → exit 置亮 → 图标/等级/
/// 体力三读数 → 三监听）。Each button reports its source rule and the
/// adaptation separately; they are not merged into one number.
fn on_open(
    mock: &MenuMock,
    setup: &[ButtonSetup; 12],
    user: Option<&crate::server::ClientUserData>,
    rank: &moly_law::ui::mysekai_rank::MysekaiRankModel,
) {
    info!(
        "[menu_dialog] 开框：DialogUtility.ShowMysekaiMenu → TryGetActiveDialog(菜单对话框) 未开 \
         ⇒ InstantiateDialog(312, Dialog 槽) → Initialize(allowCloseExternal=true) → Setup(ViewData)"
    );
    info!(
        "[menu_dialog] Setup·ViewData 派生：IsOwner()=true（单机恒站主）· \
         ShouldEnableInventoryButton=!IsVisiting()={} · ShouldEnableInfoButton=!IsVisiting()={} \
         （访客态为面板下发 mock 值）",
        !mock.visiting, !mock.visiting
    );
    for (which, button) in ALL_BUTTONS.iter().zip(setup) {
        let adaptation = match button.greyed_until {
            Some(missing) => format!("greyed until its target exists: {}", missing.reason()),
            None => "no adaptation".to_owned(),
        };
        info!(
            "[menu_dialog]   {which:?} ({}): source {} -> {}; {adaptation}; enabled {}{} -- {}",
            which.field(),
            button.rule,
            button.source,
            button.enabled(),
            if *which == MenuButton::TransitionToTitle { "; hidden (TitleCell is inactive in the prefab and nothing activates it)" } else { "" },
            which.handler()
        );
    }
    let stamina = user.and_then(|user| user.stamina);
    info!(
        "[menu_dialog] Setup: the stamina cell reads the client copy of UserMysekaiStamina {:?} (sum {:?}, gauge maximum = master normal maxStamina {:?}); \
         recover label (IsEmptyStamina) = {}; the rank cell reads UserMysekaiGamedata.totalExp={} \
         through the master rank table: rank {} / max {} · this rank {} · next rank {} · {} exp to go",
        stamina,
        stamina.map(stamina_sum),
        crate::server::stamina_max().map(|max| max.normal),
        user.is_some_and(crate::server::ClientUserData::stamina_empty),
        rank.total_exp,
        rank.mysekai_rank,
        rank.max_mysekai_rank,
        rank.total_exp_to_current_rank,
        rank.total_exp_to_next_rank,
        rank.exp_to_next_rank
    );
}

/// The wording keys `UIPartsGaugeExp.Setup` writes into the rank gauge's
/// rest text; the shell's glyph set takes their text.
pub(crate) const RANK_GAUGE_WORDINGS: [&str; 2] =
    [moly_law::ui::mysekai_rank::RANK_GAUGE_REST_WORDING_KEY, "WORD_MAX"];

/// `UIPartsGaugeExp.Setup`'s rest-text call as a wording key and its
/// arguments: `SetWordingText("WORD_MAX")` (no arguments) at the highest
/// rank, otherwise `SetWordingText(restWordingKey, [value])`, the argument
/// boxed from the int and so formatted as its decimal digits.
fn rest_wording(rest: &moly_law::ui::mysekai_rank::RestText) -> (&str, Option<Vec<String>>) {
    match rest {
        moly_law::ui::mysekai_rank::RestText::Max => (RANK_GAUGE_WORDINGS[1], None),
        moly_law::ui::mysekai_rank::RestText::Rest { key, value } => (key.as_str(), Some(vec![value.to_string()])),
    }
}

/// `UIPartsMysekaiRankGauge.Setup(new MysekaiRankGaugeViewDataModel(model))`
/// on a view: the rank text, the rest text and the gauge's fill image.
pub(crate) fn apply_rank_gauge(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    gauge: &RankGaugeTargets,
    model: &moly_law::ui::mysekai_rank::MysekaiRankModel,
) {
    let (rank_text, setup) = moly_law::ui::mysekai_rank::rank_gauge_setup(model);
    view.set_text(&gauge.rank_text, moly_law::text::custom_text_mesh::set_text(&rank_text, false));
    let (key, args) = rest_wording(&setup.rest);
    if let Some(text) = layouts.set_wording_text(view.key, &gauge.rest_text, key, args.as_deref()) {
        view.set_text(&gauge.rest_text, text);
    }
    if let Some(fill) = moly_law::ui::mysekai_rank::gauge_fill_amount(setup.gauge_now, setup.gauge_max) {
        view.set_fill(&gauge.fill_image, fill);
    }
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
/// 3. 逐件：each button's cover shows while the last `Setup` left it
///    disabled (the source rule, or the adaptation that greys it).
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>, mut dialog: ResMut<ShellDialogState>, mock: Res<MenuMock>,
    layouts: Res<UiLayouts>, time: Res<Time>,
    mut roots: Query<(&mut MenuDialogRoot, &mut Visibility, &mut Transform, &mut UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    ranks: Option<Res<crate::mysekai_rank::MysekaiRanks>>,
    total_exp: Res<crate::mysekai_rank::UserTotalExp>,
    user: Option<Res<crate::server::ClientUserData>>,
    (mut screens, mut back_keys): (ResMut<ScreenManager>, MessageReader<DialogBackKeyEvent>),
    site: Option<Res<crate::site::SiteActive>>,
) {
    // SubWindowDialog.OnHardwareBackKeyProcess is CloseProcess: the back key
    // the screen manager hands this dialog closes it.
    for key in back_keys.read() {
        if roots.iter().any(|(root, ..)| root.dialog == Some(key.id)) && dialog.menu_open {
            info!("[menu_dialog] back key (SubWindowDialog.OnHardwareBackKeyProcess -> CloseProcess)");
            dialog.menu_open = false;
        }
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else { return; };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. { return; }
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    for (mut root,mut visibility,mut transform,mut view) in &mut roots {
        let open = dialog.menu_open;
        let changed = root.requested_open != open;
        if changed {
            root.requested_open = open;
            root.elapsed = 0.;
            root.phase = if open {
                // A request while the previous instance is still closing:
                // that instance is destroyed first.
                if let Some(old) = root.dialog.take() {
                    screens.dialog_destroyed(old);
                }
                // DialogUtility.ShowMysekaiMenu: InstantiateDialog(type 312,
                // Layer_Dialog), Initialize, Setup, Open.
                match screens.show_dialog(
                    DialogType::MysekaiMenuDialog,
                    DisplayLayerType::LayerDialog,
                    DialogBackKey::Close,
                    "DialogUtility.ShowMysekaiMenu",
                ) {
                    Ok(id) => {
                        screens.open_dialog(id);
                        root.dialog = Some(id);
                        // Setup's enabled writes, once per open.
                        let inputs = SetupInputs::read(&mock, site.as_deref());
                        root.setup = Some(ALL_BUTTONS.map(|which| which.setup(&inputs)));
                    }
                    Err(error) => {
                        error!("[menu_dialog] {error}: the menu does not open");
                        dialog.menu_open = false;
                        root.requested_open = false;
                        continue;
                    }
                }
                dialog.menu_closing = false;
                MenuSlidePhase::Opening
            } else {
                on_close();
                if let Some(id) = root.dialog {
                    screens.close_dialog(id);
                }
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
        let mut opened_now = false;
        if !changed && matches!(root.phase, MenuSlidePhase::Opening | MenuSlidePhase::Closing) {
            root.elapsed = (root.elapsed + time.delta_secs()).min(MENU_SLIDE_DURATION);
            if root.elapsed >= MENU_SLIDE_DURATION {
                // OnFinishOpenAnimation, or Destroy after the close animation.
                root.phase = if open {
                    if let Some(id) = root.dialog {
                        screens.dialog_open_finished(id);
                    }
                    opened_now = true;
                    MenuSlidePhase::Open
                } else {
                    if let Some(id) = root.dialog.take() {
                        screens.dialog_destroyed(id);
                    }
                    MenuSlidePhase::Hidden
                };
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
        if opened_now {
            log_tap_map(&view, &layouts, canvas, scale, size);
        }
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
        }
        // CustomButton.OnDisable -> ShowCover: every button here greys out
        // (DisableActionType.Grayout) with its Cover child, so the cover
        // shows while Setup left the button disabled.
        if let Some(setup) = root.setup {
            for (which, button) in ALL_BUTTONS.iter().zip(setup) {
                if *which != MenuButton::TransitionToTitle {
                    view.set_visible(&format!("{}/Cover", which.source_path()), !button.enabled());
                }
            }
        }
        // TitleCell is inactive in the prefab and nothing activates it.
        view.set_visible("MenuRoot/TitleCell",false);
        // SetupMysekaiRankGauge, then UpdateStaminaGateView: the rank model
        // from the user's total experience through the gauge the dialog
        // references, then the stamina cell.
        if changed && open {
            let doc = layouts.document(view.key).expect("menu dialog layout is loaded");
            let model = ranks.as_deref()
                .unwrap_or_else(|| panic!("rank model: the master rank table has not resolved when the menu opens"))
                .model(total_exp.0);
            on_open(&mock, root.setup.as_ref().expect("Setup ran at the open edge"), user.as_deref(), &model);
            apply_rank_gauge(&mut view, &layouts, &rank_gauge_targets(doc), &model);
            update_stamina_gate_view(&mut view, &layouts, doc, user.as_deref());
        }
    }
}

/// A rank gauge's three targets as view selectors.
pub(crate) struct RankGaugeTargets {
    /// `UIPartsMysekaiRankGauge._rankText`.
    pub(crate) rank_text: String,
    /// `UIPartsGaugeExp`'s rest text (`restText` when set, else
    /// `restTextMesh`, as `UIPartsGaugeExp.Setup` chooses).
    pub(crate) rest_text: String,
    /// `UIPartsGaugeExp.gauge` -> `UIPartsGauge.fillImage`.
    pub(crate) fill_image: String,
}

/// The menu dialog's rank gauge targets: `MysekaiMenuDialog.
/// _mysekaiRankGauge` and the references under it. Both roots decode the
/// dialog's reference. A shared-root layout extracted before the extractor
/// decoded the three gauge classes (`UIPartsMysekaiRankGauge`,
/// `UIPartsGaugeExp`, `UIPartsGauge`; the CN class declares the same fields)
/// keeps them as raw bytes; only there are the three nodes the references
/// name addressed by their path under the referenced gauge, reported once.
fn rank_gauge_targets(doc: &moly_assets::ui_layout::UiPrefab) -> RankGaugeTargets {
    let dialog = doc.nodes.iter().flat_map(|node| node.components.iter())
        .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuDialog")
        .unwrap_or_else(|| panic!("{}: no MysekaiMenuDialog component", doc.prefab));
    let gauge = reference(doc, &dialog.fields, "_mysekaiRankGauge");
    let decoded = doc.nodes.iter().flat_map(|node| node.components.iter())
        .find(|c| c.path_id == gauge)
        .is_some_and(|c| c.fields.get("_rankText").is_some());
    if !decoded {
        assert!(doc.source.region.is_none(), "{}: a region layout leaves the rank gauge undecoded", doc.prefab);
        error_once!(
            "[menu_dialog] {}: the shared root's rank gauge classes are raw bytes (re-extract the shared root with the gauge decoders); the gauge texts and fill are addressed by path",
            doc.prefab
        );
        let index = doc.find(&format!("@{gauge}")).unwrap_or_else(|error| panic!("{error}"));
        let rank = doc.nodes[index].path.as_str();
        return RankGaugeTargets {
            rank_text: format!("{rank}/CustomTextMesh (2)"),
            rest_text: format!("{rank}/CustomTextMesh (3)"),
            fill_image: format!("{rank}/UIPartsGauge/GaugeBase/Mask"),
        };
    }
    rank_gauge_references(doc, gauge)
}

/// A serialized reference inside the layout file: its path id.
fn reference(doc: &moly_assets::ui_layout::UiPrefab, fields: &serde_json::Value, name: &str) -> i64 {
    let pointer = fields[name].as_array().filter(|p| p.len() == 2)
        .unwrap_or_else(|| panic!("{}: {name} is not a reference", doc.prefab));
    assert_eq!(pointer[0].as_i64(), Some(0), "{}: {name} points outside the layout file", doc.prefab);
    pointer[1].as_i64().unwrap_or_else(|| panic!("{}: {name} has no path id", doc.prefab))
}

/// The targets under one `UIPartsMysekaiRankGauge` component of a region
/// layout.
pub(crate) fn rank_gauge_references(doc: &moly_assets::ui_layout::UiPrefab, gauge_id: i64) -> RankGaugeTargets {
    let component = |id: i64, class: &str| {
        doc.nodes.iter().flat_map(|node| node.components.iter())
            .find(|c| c.path_id == id)
            .filter(|c| c.class == class)
            .unwrap_or_else(|| panic!("{}: component {id} is not a {class}", doc.prefab))
    };
    let gauge = component(gauge_id, "Sekai.Mysekai.UIPartsMysekaiRankGauge");
    let rank_text = reference(doc, &gauge.fields, "_rankText");
    let exp = component(reference(doc, &gauge.fields, "_gaugeExp"), "Sekai.UIPartsGaugeExp");
    let rest = match reference(doc, &exp.fields, "restText") {
        0 => reference(doc, &exp.fields, "restTextMesh"),
        custom_text => panic!("{}: rank gauge rest text {custom_text} is a CustomText, which is not drawn", doc.prefab),
    };
    let bar = component(reference(doc, &exp.fields, "gauge"), "Sekai.UIPartsGauge");
    let fill = reference(doc, &bar.fields, "fillImage");
    component(fill, "Sekai.UI.CustomImage");
    RankGaugeTargets { rank_text: format!("@{rank_text}"), rest_text: format!("@{rest}"), fill_image: format!("@{fill}") }
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

/// The open menu's tap map: each button's centre in window pixels (top-left
/// origin, the inverse of [`to_canvas`]), and whether it is active.
fn log_tap_map(view: &UiPrefabView, layouts: &UiLayouts, canvas: Vec2, scale: f32, window: Vec2) {
    let cells: Vec<String> = ALL_BUTTONS
        .iter()
        .map(|which| match view.rect(layouts, which.source_path(), canvas) {
            Some(rect) => {
                let centre = rect.center();
                format!(
                    "{which:?} ({:.0},{:.0}){}",
                    centre.x * scale + window.x / 2.0,
                    window.y / 2.0 - centre.y * scale,
                    if rect.active { "" } else { " inactive" }
                )
            }
            None => format!("{which:?} (no rect)"),
        })
        .collect();
    info!("[menu_dialog] open: tap map in window pixels: {}", cells.join(" · "));
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
    layouts: Res<crate::ui_layout::UiLayouts>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut screens: ResMut<ScreenManager>,
    mut sketch_modes: MessageWriter<crate::home_action::SketchModeRequest>,
    views: Query<(&crate::ui_layout::UiPrefabView, &MenuDialogRoot)>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
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
    let Ok((view, root)) = views.single() else { return; };
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    let mut close_requested = false;
    for position in &taps {
        // 模态：这一下被对话框吃掉（世界射线与外壳件都拿不到）。
        consumed.0 = true;
        let canvas = to_canvas(*position, width, height, scale);
        let hit = ALL_BUTTONS
            .into_iter()
            .find(|which| hit_test(canvas, *which, &layouts, view, size));
        match hit {
            Some(MenuButton::Close) => {
                sounds.source_button(&layouts, view.key, MenuButton::Close.source_path());
                info!(
                    "[menu_dialog] 关闭钮按下 → {}——开始窗口滑出",
                    MenuButton::Close.handler()
                );
                close_requested = true;
            }
            Some(which) => {
                let Some(setup) = root.button(which) else {
                    error!("[menu_dialog] {which:?} tapped before Setup wrote the buttons; nothing");
                    continue;
                };
                if !setup.source {
                    // A disabled CustomButton dispatches nothing.
                    info!(
                        "[menu_dialog] {which:?} ({}) tapped: disabled by the source rule {}; nothing",
                        which.field(),
                        setup.rule
                    );
                } else if let Some(missing) = setup.greyed_until {
                    // This product's adaptation: greyed until the target exists.
                    info!(
                        "[menu_dialog] {which:?} ({}) tapped: greyed until its target exists: {}; source handler {}",
                        which.field(),
                        missing.reason(),
                        which.handler()
                    );
                } else {
                    sounds.source_button(&layouts, view.key, which.source_path());
                    match which {
                        MenuButton::Chest => {
                            // OnClickChestButton: when IsActiveScreen(607) it
                            // only closes; else it reports a state to the
                            // multiplayer room (no multiplayer domain here),
                            // PushUIScreen(607) with no boot argument, then Close.
                            let caller = "MysekaiMenuDialog.OnClickChestButton";
                            if screens.is_active(MenuScreenType::MysekaiInventory) {
                                info!("[menu_dialog] {caller}: IsActiveScreen(MysekaiInventory); no push; Close");
                            } else {
                                info!("[menu_dialog] {caller}: PushUIScreen(MysekaiInventory), no boot argument (the multiplayer room report has no domain here); Close");
                                screens.push_ui_screen(MenuScreenType::MysekaiInventory, None, caller);
                            }
                            close_requested = true;
                        }
                        MenuButton::Info => {
                            // OnClickMysekaiInfoButton:
                            // MysekaiInfoUtility.TryMoveScreenLayerMysekaiInfo
                            // pushes the info screen with its boot data unless
                            // the screen is active; then Close.
                            let caller = "MysekaiMenuDialog.OnClickMysekaiInfoButton (MysekaiInfoUtility.TryMoveScreenLayerMysekaiInfo)";
                            if screens.is_active(MenuScreenType::MysekaiInfo) {
                                info!("[menu_dialog] {caller}: IsActiveScreen(MysekaiInfo); no push; Close");
                            } else {
                                info!("[menu_dialog] {caller}: PushUIScreen(MysekaiInfo, ScreenLayerMysekaiInfoBootData); Close");
                                screens.push_ui_screen(
                                    MenuScreenType::MysekaiInfo,
                                    Some("ScreenLayerMysekaiInfoBootData".to_owned()),
                                    caller,
                                );
                            }
                            close_requested = true;
                        }
                        MenuButton::WhiteBlueprintSketch => {
                            // 真源白图写生钮：Player.ChangeState(SelectSketchItem)
                            // + ChangeState(GameStateType.Sketch) → Close。他人编辑
                            // 中的警告子窗单机不可达（恒站主、无他人）。
                            info!(
                                "[menu_dialog] 白图写生钮按下 → Player.ChangeState(SelectSketchItem)                                  + ChangeState(GameStateType.Sketch) → 关框"
                            );
                            sketch_modes.write(crate::home_action::SketchModeRequest::Enter);
                            close_requested = true;
                        }
                        // A button whose adaptation line is deleted gets its
                        // source route here.
                        other => unreachable!("{other:?} has no adaptation and no route"),
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `MysekaiStaminaView.UpdateStaminaView` (with its `SetStaminaGaugeColor`
    /// and `StaminaData.IsEmptyStamina`) and `UpdateStaminaGageRate`, executed
    /// from the game's 6.8.1 binary on these inputs with the engine calls
    /// recorded (the lane harness runs the machine code; the stock count
    /// comes from the boost master's maxStamina 10000). Per row: normal,
    /// enhance, boost; the gauge coloured by palette entry 55; the gradient
    /// enabled; the boost icon sprite; the icon coloured by palette entry 5;
    /// the on-stock back enabled (the no-stock back always the opposite); the
    /// stock count sprite number or -1 for a disabled count image; and the
    /// fill as the index of the rate it took (normal 0.25, enhance 0.5,
    /// boost 0.75, full 1.0). SetNativeSize ran on the icon in every row.
    const SOURCE_ROWS: [[i32; 10]; 195] = [
    [-1, 0, -5, 1, 0, 0, 1, 0, -1, 0],
    [-1, 0, 0, 1, 0, 0, 1, 0, -1, 0],
    [-1, 0, 1, 0, 1, 1, 0, 0, -1, 2],
    [-1, 0, 99, 0, 1, 1, 0, 0, -1, 2],
    [-1, 0, 100, 0, 1, 1, 0, 0, -1, 2],
    [-1, 0, 999, 0, 1, 1, 0, 0, -1, 2],
    [-1, 0, 1000, 0, 1, 1, 0, 1, 1, 2],
    [-1, 0, 1001, 0, 1, 1, 0, 1, 1, 2],
    [-1, 0, 5000, 0, 1, 1, 0, 1, 5, 2],
    [-1, 0, 9000, 0, 1, 1, 0, 1, 9, 2],
    [-1, 0, 9999, 0, 1, 1, 0, 1, 9, 2],
    [-1, 0, 10000, 0, 1, 1, 0, 1, 9, 3],
    [-1, 0, 12000, 0, 1, 1, 0, 1, 9, 3],
    [-1, 1, -5, 0, 1, 0, 0, 0, -1, 1],
    [-1, 1, 0, 0, 1, 0, 0, 0, -1, 1],
    [-1, 1, 1, 0, 1, 1, 0, 0, -1, 2],
    [-1, 1, 99, 0, 1, 1, 0, 0, -1, 2],
    [-1, 1, 100, 0, 1, 1, 0, 0, -1, 2],
    [-1, 1, 999, 0, 1, 1, 0, 0, -1, 2],
    [-1, 1, 1000, 0, 1, 1, 0, 1, 1, 2],
    [-1, 1, 1001, 0, 1, 1, 0, 1, 1, 2],
    [-1, 1, 5000, 0, 1, 1, 0, 1, 5, 2],
    [-1, 1, 9000, 0, 1, 1, 0, 1, 9, 2],
    [-1, 1, 9999, 0, 1, 1, 0, 1, 9, 2],
    [-1, 1, 10000, 0, 1, 1, 0, 1, 9, 3],
    [-1, 1, 12000, 0, 1, 1, 0, 1, 9, 3],
    [-1, 3, -5, 0, 1, 0, 0, 0, -1, 1],
    [-1, 3, 0, 0, 1, 0, 0, 0, -1, 1],
    [-1, 3, 1, 0, 1, 1, 0, 0, -1, 2],
    [-1, 3, 99, 0, 1, 1, 0, 0, -1, 2],
    [-1, 3, 100, 0, 1, 1, 0, 0, -1, 2],
    [-1, 3, 999, 0, 1, 1, 0, 0, -1, 2],
    [-1, 3, 1000, 0, 1, 1, 0, 1, 1, 2],
    [-1, 3, 1001, 0, 1, 1, 0, 1, 1, 2],
    [-1, 3, 5000, 0, 1, 1, 0, 1, 5, 2],
    [-1, 3, 9000, 0, 1, 1, 0, 1, 9, 2],
    [-1, 3, 9999, 0, 1, 1, 0, 1, 9, 2],
    [-1, 3, 10000, 0, 1, 1, 0, 1, 9, 3],
    [-1, 3, 12000, 0, 1, 1, 0, 1, 9, 3],
    [0, 0, -5, 1, 0, 0, 1, 0, -1, 0],
    [0, 0, 0, 1, 0, 0, 1, 0, -1, 0],
    [0, 0, 1, 0, 1, 1, 0, 0, -1, 2],
    [0, 0, 99, 0, 1, 1, 0, 0, -1, 2],
    [0, 0, 100, 0, 1, 1, 0, 0, -1, 2],
    [0, 0, 999, 0, 1, 1, 0, 0, -1, 2],
    [0, 0, 1000, 0, 1, 1, 0, 1, 1, 2],
    [0, 0, 1001, 0, 1, 1, 0, 1, 1, 2],
    [0, 0, 5000, 0, 1, 1, 0, 1, 5, 2],
    [0, 0, 9000, 0, 1, 1, 0, 1, 9, 2],
    [0, 0, 9999, 0, 1, 1, 0, 1, 9, 2],
    [0, 0, 10000, 0, 1, 1, 0, 1, 9, 3],
    [0, 0, 12000, 0, 1, 1, 0, 1, 9, 3],
    [0, 1, -5, 0, 1, 0, 0, 0, -1, 1],
    [0, 1, 0, 0, 1, 0, 0, 0, -1, 1],
    [0, 1, 1, 0, 1, 1, 0, 0, -1, 2],
    [0, 1, 99, 0, 1, 1, 0, 0, -1, 2],
    [0, 1, 100, 0, 1, 1, 0, 0, -1, 2],
    [0, 1, 999, 0, 1, 1, 0, 0, -1, 2],
    [0, 1, 1000, 0, 1, 1, 0, 1, 1, 2],
    [0, 1, 1001, 0, 1, 1, 0, 1, 1, 2],
    [0, 1, 5000, 0, 1, 1, 0, 1, 5, 2],
    [0, 1, 9000, 0, 1, 1, 0, 1, 9, 2],
    [0, 1, 9999, 0, 1, 1, 0, 1, 9, 2],
    [0, 1, 10000, 0, 1, 1, 0, 1, 9, 3],
    [0, 1, 12000, 0, 1, 1, 0, 1, 9, 3],
    [0, 3, -5, 0, 1, 0, 0, 0, -1, 1],
    [0, 3, 0, 0, 1, 0, 0, 0, -1, 1],
    [0, 3, 1, 0, 1, 1, 0, 0, -1, 2],
    [0, 3, 99, 0, 1, 1, 0, 0, -1, 2],
    [0, 3, 100, 0, 1, 1, 0, 0, -1, 2],
    [0, 3, 999, 0, 1, 1, 0, 0, -1, 2],
    [0, 3, 1000, 0, 1, 1, 0, 1, 1, 2],
    [0, 3, 1001, 0, 1, 1, 0, 1, 1, 2],
    [0, 3, 5000, 0, 1, 1, 0, 1, 5, 2],
    [0, 3, 9000, 0, 1, 1, 0, 1, 9, 2],
    [0, 3, 9999, 0, 1, 1, 0, 1, 9, 2],
    [0, 3, 10000, 0, 1, 1, 0, 1, 9, 3],
    [0, 3, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1, 0, -5, 1, 0, 0, 0, 0, -1, 0],
    [1, 0, 0, 1, 0, 0, 0, 0, -1, 0],
    [1, 0, 1, 0, 1, 1, 0, 0, -1, 2],
    [1, 0, 99, 0, 1, 1, 0, 0, -1, 2],
    [1, 0, 100, 0, 1, 1, 0, 0, -1, 2],
    [1, 0, 999, 0, 1, 1, 0, 0, -1, 2],
    [1, 0, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1, 0, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1, 0, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1, 0, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1, 0, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1, 0, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1, 0, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1, 1, -5, 0, 1, 0, 0, 0, -1, 1],
    [1, 1, 0, 0, 1, 0, 0, 0, -1, 1],
    [1, 1, 1, 0, 1, 1, 0, 0, -1, 2],
    [1, 1, 99, 0, 1, 1, 0, 0, -1, 2],
    [1, 1, 100, 0, 1, 1, 0, 0, -1, 2],
    [1, 1, 999, 0, 1, 1, 0, 0, -1, 2],
    [1, 1, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1, 1, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1, 1, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1, 1, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1, 1, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1, 1, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1, 1, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1, 3, -5, 0, 1, 0, 0, 0, -1, 1],
    [1, 3, 0, 0, 1, 0, 0, 0, -1, 1],
    [1, 3, 1, 0, 1, 1, 0, 0, -1, 2],
    [1, 3, 99, 0, 1, 1, 0, 0, -1, 2],
    [1, 3, 100, 0, 1, 1, 0, 0, -1, 2],
    [1, 3, 999, 0, 1, 1, 0, 0, -1, 2],
    [1, 3, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1, 3, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1, 3, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1, 3, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1, 3, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1, 3, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1, 3, 12000, 0, 1, 1, 0, 1, 9, 3],
    [500, 0, -5, 1, 0, 0, 0, 0, -1, 0],
    [500, 0, 0, 1, 0, 0, 0, 0, -1, 0],
    [500, 0, 1, 0, 1, 1, 0, 0, -1, 2],
    [500, 0, 99, 0, 1, 1, 0, 0, -1, 2],
    [500, 0, 100, 0, 1, 1, 0, 0, -1, 2],
    [500, 0, 999, 0, 1, 1, 0, 0, -1, 2],
    [500, 0, 1000, 0, 1, 1, 0, 1, 1, 2],
    [500, 0, 1001, 0, 1, 1, 0, 1, 1, 2],
    [500, 0, 5000, 0, 1, 1, 0, 1, 5, 2],
    [500, 0, 9000, 0, 1, 1, 0, 1, 9, 2],
    [500, 0, 9999, 0, 1, 1, 0, 1, 9, 2],
    [500, 0, 10000, 0, 1, 1, 0, 1, 9, 3],
    [500, 0, 12000, 0, 1, 1, 0, 1, 9, 3],
    [500, 1, -5, 0, 1, 0, 0, 0, -1, 1],
    [500, 1, 0, 0, 1, 0, 0, 0, -1, 1],
    [500, 1, 1, 0, 1, 1, 0, 0, -1, 2],
    [500, 1, 99, 0, 1, 1, 0, 0, -1, 2],
    [500, 1, 100, 0, 1, 1, 0, 0, -1, 2],
    [500, 1, 999, 0, 1, 1, 0, 0, -1, 2],
    [500, 1, 1000, 0, 1, 1, 0, 1, 1, 2],
    [500, 1, 1001, 0, 1, 1, 0, 1, 1, 2],
    [500, 1, 5000, 0, 1, 1, 0, 1, 5, 2],
    [500, 1, 9000, 0, 1, 1, 0, 1, 9, 2],
    [500, 1, 9999, 0, 1, 1, 0, 1, 9, 2],
    [500, 1, 10000, 0, 1, 1, 0, 1, 9, 3],
    [500, 1, 12000, 0, 1, 1, 0, 1, 9, 3],
    [500, 3, -5, 0, 1, 0, 0, 0, -1, 1],
    [500, 3, 0, 0, 1, 0, 0, 0, -1, 1],
    [500, 3, 1, 0, 1, 1, 0, 0, -1, 2],
    [500, 3, 99, 0, 1, 1, 0, 0, -1, 2],
    [500, 3, 100, 0, 1, 1, 0, 0, -1, 2],
    [500, 3, 999, 0, 1, 1, 0, 0, -1, 2],
    [500, 3, 1000, 0, 1, 1, 0, 1, 1, 2],
    [500, 3, 1001, 0, 1, 1, 0, 1, 1, 2],
    [500, 3, 5000, 0, 1, 1, 0, 1, 5, 2],
    [500, 3, 9000, 0, 1, 1, 0, 1, 9, 2],
    [500, 3, 9999, 0, 1, 1, 0, 1, 9, 2],
    [500, 3, 10000, 0, 1, 1, 0, 1, 9, 3],
    [500, 3, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 0, -5, 1, 0, 0, 0, 0, -1, 0],
    [1000, 0, 0, 1, 0, 0, 0, 0, -1, 0],
    [1000, 0, 1, 0, 1, 1, 0, 0, -1, 2],
    [1000, 0, 99, 0, 1, 1, 0, 0, -1, 2],
    [1000, 0, 100, 0, 1, 1, 0, 0, -1, 2],
    [1000, 0, 999, 0, 1, 1, 0, 0, -1, 2],
    [1000, 0, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1000, 0, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1000, 0, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1000, 0, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1000, 0, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1000, 0, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 0, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 1, -5, 0, 1, 0, 0, 0, -1, 1],
    [1000, 1, 0, 0, 1, 0, 0, 0, -1, 1],
    [1000, 1, 1, 0, 1, 1, 0, 0, -1, 2],
    [1000, 1, 99, 0, 1, 1, 0, 0, -1, 2],
    [1000, 1, 100, 0, 1, 1, 0, 0, -1, 2],
    [1000, 1, 999, 0, 1, 1, 0, 0, -1, 2],
    [1000, 1, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1000, 1, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1000, 1, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1000, 1, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1000, 1, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1000, 1, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 1, 12000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 3, -5, 0, 1, 0, 0, 0, -1, 1],
    [1000, 3, 0, 0, 1, 0, 0, 0, -1, 1],
    [1000, 3, 1, 0, 1, 1, 0, 0, -1, 2],
    [1000, 3, 99, 0, 1, 1, 0, 0, -1, 2],
    [1000, 3, 100, 0, 1, 1, 0, 0, -1, 2],
    [1000, 3, 999, 0, 1, 1, 0, 0, -1, 2],
    [1000, 3, 1000, 0, 1, 1, 0, 1, 1, 2],
    [1000, 3, 1001, 0, 1, 1, 0, 1, 1, 2],
    [1000, 3, 5000, 0, 1, 1, 0, 1, 5, 2],
    [1000, 3, 9000, 0, 1, 1, 0, 1, 9, 2],
    [1000, 3, 9999, 0, 1, 1, 0, 1, 9, 2],
    [1000, 3, 10000, 0, 1, 1, 0, 1, 9, 3],
    [1000, 3, 12000, 0, 1, 1, 0, 1, 9, 3]
    ];

    #[test]
    fn stamina_view_matches_the_source_rows() {
        let rates = (0.25, 0.5, 0.75);
        for row in SOURCE_ROWS {
            let stamina = crate::server::Stamina { normal: row[0], enhance: row[1], boost: row[2] };
            let max = crate::server::StaminaMax { normal: 1000, enhance: 1000, boost: 10000 };
            let state = stamina_view_state_with(stamina, boost_stamina_stock_count(stamina.boost, max), rates);
            let fill = [0.25, 0.5, 0.75, 1.0][row[9] as usize];
            assert_eq!(
                (state.gauge_palette, state.gradient, state.icon == "icon_stamina_boost_h40", state.icon_palette,
                    state.on_stock, state.boost_count.unwrap_or(-1), state.fill),
                (row[3] == 1, row[4] == 1, row[5] == 1, row[6] == 1, row[7] == 1, row[8], fill),
                "stamina {stamina:?}"
            );
        }
    }
}
