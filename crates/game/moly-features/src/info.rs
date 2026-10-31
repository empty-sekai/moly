//! Info screen layer view (matches `ScreenLayerMysekaiInfo`, screen number
//! 622) — the first screen layer view in this repository that carries
//! content: pushing the layer shows it at once, and the layer stack slot
//! drives its visibility.
//!
//! ## Layer identity and entry
//!
//! The entry is the existing action button dispatch table's `OpenMysekaiInfo`
//! slot (it writes `LayerCommand::Push(MysekaiInfo)`; this module does not
//! change the dispatch table). The push helper first checks
//! `IsActiveScreen(622)` and does not push again when the layer is already
//! open, matching the layer stack's rule that a repeated push of the same
//! layer is dropped. The boot parameter (bootData) is the rank page model,
//! which is server user state here, so the panel supplies a mock (see below).
//!
//! ## Two-page structure (page count = 2, the SetupPage nine-step assembly
//! order)
//!
//! - **Page 0, the rank page** (`MysekaiInfoRankPage.Setup`): rank text, rank gauge,
//!   and on the JP client the home and my-room put-cost limits
//!   (`FixtureHomePutLimitPutCost` as the number, `FixtureMyRoomPutLimitPutCost`
//!   through `MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST`), then the photo button
//!   and `SetupPhotoAsync`. See the rank page section below.
//! - **Page 1, the option page** (`MysekaiInfoOption1Page.Setup`): four radio
//!   toggle groups plus the voice download button.
//! - The order of the pages, the tabs and each toggle group is read from the
//!   prefab's PPtr arrays: page 0 = rank page, page 1 = option page. The page
//!   state persists across open and close (the layer instance stays alive, its
//!   serialized fields survive `SetActive(false)`, so reopening fades in the
//!   last page).
//! - Two tabs (a `CustomIndexToggleGroup` radio group whose callback
//!   `OnNoteTabSelectedIndexChanged` calls `ChangePage`); left and right arrow
//!   buttons (visible when `IsAbleToMovePrev/NextPage`, bounds clamped to
//!   0 / Length-1).
//!
//! ## The four toggle groups (option indices come from the serialized group,
//! not from node names)
//!
//! | Group | Enum | Values | Default |
//! |---|---|---|---|
//! | Access permission | `MysekaiRoomAcceptUserType` | all=0 · friend_only=1 · reject=2 · review=3 | no client default (server value, null → 0) |
//! | Image quality | `MysekaiImageQualityType` | high=0 · normal=1 · low=2 | **normal** |
//! | Frame rate | `MysekaiFpsQualityType` | high=0 · normal=1 | **high** |
//! | Convert notification | `MysekaiConvertFxitureNotificationType` | on=0 · off=1 | **on** |
//!
//! The access permission group has only three radio buttons. review=3 is a
//! server state meaning under review: the tip is shown, the index of reject
//! (2) is selected, all three buttons are disabled and the source disable
//! cover is shown; it is not a fourth option.
//!
//! Where the defaults come from: the `MysekaiOptionSettingData` constructor
//! writes only the image quality member = 1, so the default image quality is
//! normal; the frame rate and convert notification members are not written,
//! so zero initialization gives 0 (high / on). The current access permission
//! is read from `UserMysekaiVisitSetting` (server side).
//!
//! ## Switching rules (CustomIndexToggleGroup)
//!
//! Single selection; the callback fires **only when the selection
//! changes** (tapping the selected option gives no callback); the initial
//! value is written twice without notification (set_SelectedIndex + WithoutNotify
//! before the callbacks are attached, so the initial value fires none); an
//! index out of range is rejected with LogError — the toggle enums here are
//! closed sets, so construction rules out the out-of-range branch.
//!
//! ## Applying a switch at once
//!
//! - **Image quality** (`OnChangeImageQualityToggle`): writes the quality **and
//!   immediately** calls `SetImageQuality` — high → `SetTargetDpi(299)` + FXAA on ·
//!   normal → 200 + on · low → 180 + off; `SetTargetDpi` writes
//!   `clamp(target DPI / screen DPI, 0, 1)` into RenderScale. FXAA and the
//!   target DPI are written into the current GameSettings; the scene camera
//!   applies the render scale from the screen DPI derived from the window scale
//!   factor (see `game_settings::source_render_scale`).
//! - **Frame rate** (`OnChangeFpsToggle`): writes the quality and immediately calls
//!   `SetFpsQuality` — high → `Application.targetFrameRate = 60` · normal → 30;
//!   out of range → LogError; the same value without a forced update returns
//!   early (an idempotence gate). **The equivalent here is `WinitSettings`'
//!   Reactive mode**: the wait is 1/60 · 1/30 and all three reactive bits are
//!   off (tick on the beat only, buffer events to the next tick) — a quality
//!   change at run time takes effect in the same frame. This is what this
//!   layer's wiring actually does. The browser uses an animation-frame beat
//!   instead: it updates on the frame when due and skips frames that are not due
//!   (`game_settings`).
//! - **Convert notification** (`OnChangeConvertFixtureNotificationTypeToggle`):
//!   only writes the quality; no immediate side effect.
//! - **Access permission** (`OnChangeVisitSettingToggle`): outside the review
//!   state it only writes the quality; the report goes through the exit chain
//!   (`ChangeVisitType` in `OnExited`, server domain, so the panel supplies a
//!   mock).
//!
//! **Startup application**: when the field scene starts (`SceneMysekai.Start`)
//! it applies the stored qualities in full (forceUpdate=true applies both
//! qualities). The stored qualities are the local settings'
//! `MysekaiOptionSettingData` (the member of the same name in
//! `ApplicationLocalSettings`, with the three keys
//! `MysekaiImageQualityType` / `MysekaiFpsQualityType` /
//! `MysekaiConvertFxitureNotificationType`, enums stored as integers); without
//! a stored document the constructor defaults apply (image quality normal ·
//! frame rate high ⇒ 60). `init` applies them at Startup, and the first scene
//! frame applies the frame rate once more (the product panel's startup load
//! runs after `init` and writes its own saved frame rate).
//!
//! ## Page switching rules (ChangePage / MoveXxxPage)
//!
//! Tapping a tab or an arrow: first writes `_currentPageIndex` → `RefreshTab`
//! (writes the tab group without notification + `Setup` on each tab) →
//! `RefreshArrowButton` → rebuilds the cancellation token → picks the direction
//! from the difference between old and new page. The page transition is a
//! directional fade-out of the old page + the two tab particles + a directional
//! fade-in of the new page, with a single wait of `Delay(interval * 2)`
//! (interval = particle duration / 10). **The particle draw path is not
//! integrated and the two serial fades and their timing are not extracted**,
//! so a page change lands in the same frame here (the same simplification as
//! the layer view's existing lack of animation), a named gap. The page-swipe
//! gesture (`OnFlick` left = next page · right = previous page) is a named gap:
//! the gesture layer has no flick classifier and this module does not build a
//! second one.
//!
//! ## Lifecycle (hook order follows the layer stack ladder; the layer's own
//! content is here)
//!
//! `OnBoot` (bootData = rank model mock · access permission current value
//! re-read from the mock) → `OnInitComponent` (three qualities read + seven
//! Setup calls: tabs / rank info button / arrow buttons / page, four of which
//! are wired, textures / particles / page-swipe gesture are named gaps) →
//! `OnFinishStartAnimation` (all pages Hide, then the current page FadeIn) → …
//! → `OnExitStart` (current page FadeOut) → `OnExited` (the three qualities are
//! written back through SaveToStorage, merging only the
//! `MysekaiOptionSettingData` section and reading it back to check; the access
//! permission `ChangeVisitType` report is server domain ⇒ mock).
//!
//! ## Server domain and the mock panel (shaped like the audio domain's volume
//! panel: a named resource + defaults + environment variable overrides, an
//! invalid value warns loudly and falls back to the default)
//!
//! - **Rank page display items**: `MysekaiRankModel` reads user state (server
//!   side) ⇒ mock: rank · gauge ratio; the put-limit count and joint count are
//!   mock values too.
//! - **Rank info dialog** (`Show1ButtonDialog<MysekaiRankListDialog>`,
//!   dialog type `DialogType::MysekaiRankListDialog`, shown through the
//!   screen manager with the back key
//!   closing it; a tap outside closes it): the dialog body = the per-rank
//!   unlock condition list of the master table `MasterMysekaiRanks` + user
//!   rank ⇒ also served by the mock, with the body texts named as supplied by
//!   the panel.
//! - **Current access permission**: supplied by the panel as a mock (option
//!   names use the enum names).
//! - **Rank and rank gauge on the rank page**: `TryMoveScreenLayerMysekaiInfo`
//!   builds the rank page model with `new MysekaiRankModel()` (user total
//!   experience, the same mock the menu reads, see
//!   [`crate::mysekai_rank::UserTotalExp`]) wrapped in
//!   `MysekaiRankGaugeViewDataModel`; the gauge uses the menu's same
//!   `UIPartsMysekaiRankGauge.Setup`. The gauge is taken as the document's only
//!   `UIPartsMysekaiRankGauge` component (on a region layout its references are
//!   all decoded).
//!
//! Environment variables: `MOLY_INFO_MOCK_ACCESS_PERMISSION` (all/friend_only/reject/
//! review) · `MOLY_INFO_MOCK_FIXTURE_PUT_LIMIT` ·
//! `MOLY_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT`.
//!
//! ## Rank page costs, photo and voice download
//!
//! - **Put-cost limits** (JP): `GetFixturePutLimitCost` asks
//!   `GetMasterMysekaiFixturePutLimitLevelFromMysekaiRankRelease(rank,
//!   category)` (read natively): the `externalId`s of the
//!   `mysekai_fixture_put_limit` rank releases at or below the rank; among
//!   the put-limit level rows of the site category whose id is one of them,
//!   the first with the highest level; its `putCostLimit`, else 0. Both
//!   tables are master data read from the region's master mirror
//!   (`mysekaiRankReleases`, `mysekaiFixturePutLimitLevels`); a table that
//!   is absent, or not in this shape, leaves the two texts as the prefab has
//!   them, named once. The CN rank page has no put-cost texts and does not
//!   request the tables.
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
//! ## Chosen values and named gaps (read this before changing them)
//!
//! - The view and the hit testing share the source prefab layouts: Info for
//!   the main layer, Common1 for the rank info dialog. Dynamic option and user
//!   data displays are written into their own UiPrefabView.
//! - The back key: the screen manager gives it to the rank list dialog when
//!   it is open, else to the header back button (the screen pops there).
//! - While the layer is open **this layer consumes every tap** (a full-screen
//!   layer blocks world rays, as its `blockRaycasts` does); world input yields
//!   through the shared layer state and pointer ownership.

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


/// Page count (the SetupPage assembly order: rank page + option page).
const PAGE_COUNT: usize = 2;

/// 情报层全部渲染文案的字符闭包（静态标签 + 数字位）——并入外壳字符集
/// （气泡层图集的同一并集成员，图集的第五个消费者）。动态值只有闭集
/// 档名 · 整数 · 一位小数，无自由文本 ⇒ 装载期可枚举全量。
pub const FIXED_TEXTS: &[&str] = &[
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
// The four toggle groups (the enum values are checked one by one; the index
// maps to the value identically)
// ---------------------------------------------------------------------------

/// Access permission option (`MysekaiRoomAcceptUserType`). The value is
/// persisted on the server (read from `UserMysekaiVisitSetting`, null → 0), so
/// the panel supplies a mock. review is a state that locks the first three
/// options, not a selectable fourth button.
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
    /// The mock panel's option name (the enum name).
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

/// Image quality option (`MysekaiImageQualityType`). The default is normal
/// (the constructor writes it; see the module header).
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
    /// The two values `SetImageQuality` applies: the target DPI and the FXAA
    /// switch.
    fn target_dpi_and_fxaa(self) -> (u32, bool) {
        match self {
            Self::High => (299, true),
            Self::Normal => (200, true),
            Self::Low => (180, false),
        }
    }
}

/// Frame rate option (`MysekaiFpsQualityType`). The default is high (zero
/// initialization). Values: high → 60 · normal → 30
/// (`Application.targetFrameRate`).
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

/// Convert notification option (`MysekaiConvertFxitureNotificationType`; the
/// class name carries this misspelling). The default is on (zero
/// initialization).
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

/// The info screen's session option state (this repository's counterpart of
/// `MysekaiOptionSettingData`: it lives for the session, so reopening the
/// layer reads the changed values; the exit chain's OnExited writes it back to
/// the local settings). The access permission member is re-read from the mock
/// panel each time the layer opens (OnBoot reads the server's current value).
#[derive(Resource)]
pub struct InfoSettings {
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
pub struct InfoMock {
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
#[derive(Default)]
pub struct InfoPageState {
    current: usize,
}


/// In-layer dialog state (the Dialog slot: the dialog does not enter the
/// screen layer stack, and while open it blocks taps in the layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfoDialog {
    /// The rank list (`MysekaiRankListDialog`, a one-button dialog, closes on
    /// a tap outside).
    RankList,
}

/// `Show1ButtonDialog<MysekaiRankListDialog>(DialogType.MysekaiRankListDialog)`.
const RANK_LIST_DIALOG: DialogType = DialogType::MysekaiRankListDialog;

#[derive(Resource, Default)]
pub struct InfoDialogState {
    open: Option<InfoDialog>,
    /// The screen manager's handle of the open dialog.
    id: Option<DialogId>,
}

/// 铺装闩（视图一次铺成后置）。
#[derive(Resource, Default)]
pub struct InfoSpawned;

// ---------------------------------------------------------------------------
// 业务件身份
// ---------------------------------------------------------------------------

/// 单选组身份（设置页四组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToggleGroup {
    Access,
    ImageQuality,
    Fps,
    Convert,
}

/// 情报层业务件身份；点击矩形由当前 UiPrefabView 的源布局提供。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfoItem {
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
pub struct InfoRoot;

/// 层内对话框根（Dialog 槽语义：z ≥ 20，模态阻断层内点按）。
#[derive(Component)]
pub struct InfoDialogRoot {
    which: InfoDialog,
}

// ---------------------------------------------------------------------------
// Startup：资源与启动施加
// ---------------------------------------------------------------------------

/// Startup：资源落位 + 启动施加律（进场即按存量档全量施加：画质与刷新率
/// 两档读自本地设置档，无档取构造默认）。
pub fn init(mut commands: Commands, mut graphics: ResMut<crate::game_settings::GameSettings>) {
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
pub struct InfoPresentation {
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

pub fn referenced_component<'a>(
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
pub const WORDINGS: &[&str] = &["WORD_DOWNLOADED", "WORD_BULK_DOWNLOAD", MYROOM_COST_WORDING];

const MYROOM_COST_WORDING: &str = "MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST";

use crate::fixture_put_limit::{PutLimitTables, SiteCategory};

// ScreenLayerMysekaiInfo's constructor sets the page fade duration to 0.1s.
const PAGE_FADE_SECONDS: f32 = 0.1;

pub fn spawn_when_ready(
    mut commands: Commands, layouts: Res<crate::ui_layout::UiLayouts>, server: Res<AssetServer>, spawned: Option<Res<InfoSpawned>>,
    put_costs: Option<Res<crate::fixture_put_limit::PutCostMaster>>,
) {
    if spawned.is_some() || !["Info","Common1"].iter().all(|key|layouts.ready(key,&server)) {return;}
    let doc = layouts.document("Info").expect("ready Info prefab");
    // Only the put-cost texts read the two tables; a rank page without them
    // (the CN classes, whose count texts come from the panel) does not read
    // them, so a CN root's own table shape is never requested here.
    let put_limit = if info_field_set(doc).put_limit_costs {
        let Some(put_limit) = put_costs.as_deref().and_then(crate::fixture_put_limit::PutCostMaster::limits) else { return; };
        let put_limit = put_limit.clone();
        match &put_limit {
            Ok(tables) => info!("[info] put-limit tables: {} put-limit releases, {} level rows", tables.releases.len(), tables.level_count()),
            Err(_) => info!("[info] no put-limit tables: the rank page's put-cost texts are hidden"),
        }
        put_limit
    } else {
        Err("this region's rank page has no put-cost texts".to_owned())
    };
    // The room cost text is written only where the region's page declares it.
    let writes_costs = info_field_set(doc).put_limit_costs;
    let missing: Vec<&str> = WORDINGS
        .iter()
        .copied()
        .filter(|key| writes_costs || *key != MYROOM_COST_WORDING)
        .filter(|key| !layouts.wordings.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        warn!("[info] wordings missing from this root: {missing:?}; their texts follow the region's wording lookup");
    }
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

/// The open-hook sequence (beyond the layer stack's mount ladder log, the
/// layer's own content runs here; with no animation every wait point passes in
/// the same frame). `rank` is `None` when the rank model was refused (named at
/// the open edge): the rank page's model texts are hidden.
fn on_open(settings: &mut InfoSettings, mock: &InfoMock, page_state: &InfoPageState,
    rank: Option<&moly_law::ui::mysekai_rank::MysekaiRankModel>) {
    // OnBoot: bootData = the rank page model (server user state, so the panel
    // supplies it); the access permission's
    // current value is re-read (OnBoot reads it from the user data
    // manager on the server side, so the mock stands in).
    settings.access = mock.access_permission;
    if let Some(rank) = rank {
        info!(
            "[info] OnBoot: bootData = the rank page model (total exp {} (mock) looked up in the master rank table: rank {} / max {} · \
             cumulative to this rank {} · to the next rank {}; supplied by the panel: fixture put limit {} · joint put limit {}); access permission current value = {} (supplied by the panel)",
            rank.total_exp,
            rank.mysekai_rank,
            rank.max_mysekai_rank,
            rank.total_exp_to_current_rank,
            rank.total_exp_to_next_rank,
            mock.fixture_put_limit,
            mock.fixture_joint_put_limit,
            settings.access.true_name()
        );
    }
    info!(
        "[info] OnInitComponent: three qualities read (image quality {} · frame rate {} · convert notification {}); seven Setup calls -- \
         SetupTab · SetupRankInfoButton · SetupArrowButton · SetupPage wired; the view reads the source prefab; \
         SetupParticle is a named gap (particle draw path not integrated) · SetupPageFlick is a named gap \
         (the gesture layer has no flick classifier)",
        settings.image_quality.label(),
        settings.fps.label(),
        settings.convert.label()
    );
    info!(
        "[info] OnFinishStartAnimation: both pages Hide, then the current page '{}' FadeIn -- the layer view has no animation, \
         and finishes in this frame (the two serial fades are not wired)",
        page_label(page_state.current)
    );
    // The current values of the four groups and their defaults.
    info!(
        "[info] Info placed: page {}/{} · access permission = {} (supplied by the panel, no client default) · image quality = {}\
         (default {}) · frame rate = {} (default {}) · convert notification = {} (default {})",
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

/// The close-hook sequence (the layer's own content of the exit chain).
fn on_close(settings: &InfoSettings, page_state: &InfoPageState) {
    info!(
        "[info] OnExitStart: the current page '{}' fades out -- the layer view has no animation and finishes in this frame",
        page_label(page_state.current)
    );
    info!(
        "[info] OnExited: three qualities written back through SaveToStorage (image quality {} · frame rate {} · convert notification {}); access permission reported as \
         ChangeVisitType={} (server domain, so the report stops at the mock supplied by the panel)",
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
/// model built per open, the reason a rank model was last refused (named
/// once per reason), and the one-time scene start.
#[derive(Default)]
pub struct PlaceMemory {
    was_open: bool,
    rank: Option<moly_law::ui::mysekai_rank::MysekaiRankModel>,
    rank_refused: Option<String>,
    scene_started: bool,
}

/// Update：摆位与逐帧状态。每帧——
/// 1. 层开关沿（进层/退层钩子串各一串，与层栈梯日志同帧互补）；
/// 2. 根可见性 = 层栈当前层是否情报层；根缩放 = canvas 缩放；
/// 3. 源页节点按当前页显隐，对话框根按对话框态显隐；
/// 4. UiPrefabView 覆写单选勾选、箭头、量表与动态文案。
pub fn place(
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
    let PlaceMemory { was_open, rank, rank_refused, scene_started } = &mut *memory;
    if !*scene_started {
        *scene_started = true;
        apply_on_scene_start(&mut graphics, &settings);
    }
    let open=stack.current()==LayerId::MysekaiInfo;
    let opening = open && !*was_open;
    if opening {
        // TryMoveScreenLayerMysekaiInfo: a new rank model per open. Where the
        // source throws (or the table has not resolved) the rank page's model
        // texts are refused by name and the rest of the screen opens.
        let built = match ranks.as_deref() {
            Some(ranks) => ranks.try_model(total_exp.0),
            None => Err("rank model: the master rank table has not resolved when the info screen opens".to_owned()),
        };
        *rank = match built {
            Ok(model) => Some(model),
            Err(reason) => {
                if rank_refused.as_deref() != Some(reason.as_str()) {
                    error!("[info] {reason}; rank values and gauge hidden");
                    *rank_refused = Some(reason);
                }
                None
            }
        };
    }
    match (*was_open,open) {(false,true)=>on_open(&mut settings,&mock,&page_state,rank.as_ref()),(true,false)=>on_close(&settings,&page_state),_=>{}}
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
        set_if_neq!(*visible,if open {Visibility::Inherited}else{Visibility::Hidden});crate::canvas::set_scale(&mut transform,scale);
        for (index, source_page) in bindings.pages.iter().enumerate() {
            view.set_visible(&source_page.node, index == page);
            for group in &source_page.groups { view.set_alpha(group, if open && index == page { alpha } else { 0. }); }
        }
        for (index, tab) in bindings.tabs.iter().enumerate() {
            view.set_visible(&tab.on_image, page == index);
            view.set_visible(&tab.off_image, page != index);
        }
        view.set_visible(&bindings.arrow_prev,page>0);view.set_visible(&bindings.arrow_next,page+1<PAGE_COUNT);
        view.set_visible(&bindings.rank_page.rank_text, rank.is_some());
        let doc = layouts.document(view.key).expect("info layout is loaded");
        let targets = info_rank_gauge_targets(doc);
        for target in [&targets.rank_text, &targets.rest_text, &targets.fill_image] {
            view.set_visible(target, rank.is_some());
        }
        if let Some([home, room]) = &bindings.rank_page.put_limit_costs {
            view.set_visible(home, rank.is_some() && put_limit.is_ok());
            view.set_visible(room, rank.is_some() && put_limit.is_ok());
        }
        if let Some(rank) = rank.as_ref() {
            view.set_text(&bindings.rank_page.rank_text, rank.mysekai_rank.to_string());
            crate::menu_dialog::apply_rank_gauge(&mut view, &layouts, &targets, rank);
            // FixtureHomePutLimitPutCost.ToString() and
            // GetFormat(MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST, room cost).
            if let (Some([home, room]), Ok(tables)) = (&bindings.rank_page.put_limit_costs, &put_limit) {
                let rank = i64::from(rank.mysekai_rank);
                view.set_text(home, tables.cost(rank, SiteCategory::HousingHome).to_string());
                let format = layouts.wording(MYROOM_COST_WORDING);
                let room_cost = tables.cost(rank, SiteCategory::HousingRoom).to_string();
                let text = moly_law::text::custom_text_mesh::format_wording(&format, &[room_cost])
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
        view.set_text(&bindings.voice_download_text, layouts.wording(voice_key));
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
        set_if_neq!(*visible,if open&&dialog.open==Some(root.which){Visibility::Inherited}else{Visibility::Hidden});crate::canvas::set_scale(&mut transform,scale);
        view.set_visible("WindowRoot/Tabs",false);
        match root.which {
            InfoDialog::RankList=>{
                let mut lines = Vec::new();
                if let Some(rank) = rank.as_ref() {
                    lines.push(InfoItem::RankLevelText.current_label(&mock, rank.mysekai_rank).unwrap());
                }
                lines.push(InfoItem::FixturePutText.current_label(&mock, 0).unwrap());
                lines.push(InfoItem::FixtureJointText.current_label(&mock, 0).unwrap());
                view.set_text("Content/MessageBody", lines.join("\n"));
            }
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

/// Page change (matches `ChangePage`: write the page index → RefreshTab (the
/// tab group written without notification + `Setup` on each tab) → refresh the
/// arrows → direction from the old/new difference). The page transition lands
/// in the same frame (particles and fades are named gaps; see the module
/// header).
fn change_page(page_state: &mut InfoPageState, next: usize, via: &str) {
    if next == page_state.current {
        info!(
            "[info] page tap did not change the page ({via}) -- the callback fires only on a new selection, so the same value gives none (the same as the radio group)"
        );
        return;
    }
    let old = page_state.current;
    page_state.current = next;
    let direction = if next > old { "forward" } else { "back" };
    let prev_able = if next > 0 { "enabled" } else { "at bound" };
    let next_able = if next + 1 < PAGE_COUNT {
        "enabled"
    } else {
        "at bound"
    };
    info!(
        "[info] tab change {old} -> {next} ({via}): RefreshTab writes the tab group without notification and calls Setup on each tab -> \
         refresh the arrows (previous {prev_able} · next {next_able}) -> direction = {direction}; page transition = directional \
         fade-out of the old page + the two tab particles + directional fade-in of the new page (particle draw path not integrated · fades and timing not extracted => lands in the same frame, \
         a named gap)"
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

/// Option switch (the switching rules are in the module header: two groups
/// apply at once, two only write the option).
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
                info!("[info] access permission option tap did not change it ({}) -- the callback fires only on a new selection", next.label());
                return;
            }
            settings.access = next;
            info!(
                "[info] access permission -> {} (OnChangeVisitSettingToggle only writes the option, no immediate side effect; \
                 the report is in the exit chain -- server domain, so the mock supplied by the panel)",
                next.true_name()
            );
        }
        ToggleGroup::ImageQuality => {
            let Some(next) = ImageQuality::from_index(index) else {
                return;
            };
            if next == settings.image_quality && graphics.graphics.fxaa == next.target_dpi_and_fxaa().1 {
                info!("[info] image quality option tap did not change it ({}) -- the callback fires only on a new selection", next.label());
                return;
            }
            settings.image_quality = next;
            info!(
                "[info] image quality -> {} (OnChangeImageQualityToggle writes the option and applies at once)",
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
                    "[info] frame rate option tap did not change it ({}) -- the callback fires only on a new selection",
                    next.label()
                );
                return;
            }
            settings.fps = next;
            info!(
                "[info] frame rate -> {} (OnChangeFpsToggle writes the option and applies at once)",
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
                    "[info] convert notification option tap did not change it ({}) -- the callback fires only on a new selection",
                    next.label()
                );
                return;
            }
            settings.convert = next;
            info!(
                "[info] convert notification -> {} (OnChangeConvertFixtureNotificationTypeToggle only writes the option, \
                 no immediate side effect)",
                next.label()
            );
        }
    }
}

/// Tap dispatch: an open dialog comes first (modal; it gets every tap, and a
/// tap outside closes it); then hits on the layer's items. While the layer is
/// open **this layer consumes every tap** (a full-screen layer blocks world
/// rays, as its `blockRaycasts` does).
pub fn click(
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
        // The full-screen layer consumes this tap (no world ray is cast,
        // as with its `blockRaycasts`).
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
                    info!("[info] rank list close pressed -> dialog closed")
                }
                _ => info!(
                    "[info] a dialog is open; a tap outside closes it (modal; the rank list dialog closes on a tap outside, as here)"
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
            if let Some(path) = item_path(item,bindings) { layouts.queue_button_se(&mut sounds, view.key, path); }
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
                        "[info] rank info button pressed -> Show1ButtonDialog(rank list 342) -- Dialog slot, \
                         not entering the layer stack; list body = master table list + user state, so a mock supplied by the panel"
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
            info!("[info] tap inside the layer hit no item (panel background) -- consumed, not passed on");
        }
    }
}
