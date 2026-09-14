//! 选项对话框（真源 `OptionDialog`，DialogType 82，Dialog 槽——全局设置的
//! 对话框体，Common1ButtonDialog 一钮形）——本仓建它的**音量页**，其余
//! 三页置灰未建。
//!
//! ## 入口
//!
//! The product settings panel opens this source dialog. It does not add a
//! hand-positioned button to the field chrome.
//!
//! ## 页结构（四页，本仓建一页）
//!
//! 页枚举：Live=0 · Volume=1 · System=2 · Communication=3（另一语言版本
//! 多一页 CustomScore，音量页两版一致）。本仓只建音量页；开局页取音量页
//! （真源开局页 Live=0 属演出域未建——本仓选值，具名）。
//!
//! ## 六滑杆（SetupObject 逐个，方法体直读）
//!
//! | 滑杆 | 组 | 声型 | 预览 cue |
//! |---|---|---|---|
//! | Live BGM | Live | 型 0（BGM） | 无（cue 空 ⇒ 预览静默） |
//! | Live SE | Live | 型 3（InGameSE） | `SE_VOLCHANGE_SE_INGAME` |
//! | Live Voice | Live | 型 4（InGameVoice） | `SE_VOLCHANGE_VOX_INGAME` |
//! | System BGM | System | 型 0 | 无 |
//! | System SE | System | 型 1（SE） | `SE_VOLCHANGE_SE_UI` |
//! | System Voice | System | 型 2（Voice） | `SE_VOLCHANGE_VOX_SCENARIO` |
//!
//! 控件是整数选择器（ViewData 的 SelectedNum/MinNum/MaxNum 全 int，
//! 0..100 步 1）：±钮 = `ChangeItemSelectNum` ±1 界内钳后写值并回调；
//! 置灰律 = 减量钮 Selected>Min 才亮、增量钮 Selected<Max 才亮
//! （`UpdateButtonEnable` 方法体直读）。滑杆本体是 Unity Slider 子类
//! （`CustomSlider`）：按下即落值到按点位、拖动续值；IsPlaySliderSe=
//! false ⇒ 拖动不播逐档 SE（`OnValueChange` 的逐档 SE 支整体被该开关
//! 关掉，方法体直读）。
//!
//! ## 变更链（b__0/b__1 两拍，方法体直读）
//!
//! 滑杆变更回调 b__0：取消在途延时 → 重排 0.15s 延时预览 → **立即**
//! `UpdateVolume`。UpdateVolume 只读**系统三滑杆** →
//! `SetupVolume(1.0, Bgm, Se, Voice)` → 三播放器 UpdateAll（即刻生效）
//! ——**游戏外预览律：这里没有 BGM×0.7**；mysekai 场内的 0.7 落在 BGM
//! 消费侧（进场施加同源，见 `crate::audio` 的 BGM 通道）。Live 组滑杆
//! 不进施加（档值在保存关闭时随 Live 组落盘）。
//!
//! 延时预览 b__1（0.15s 到点）：cue 非空 ⇒ 先 `StopVoiceAll` 再按型播
//! 预览声——型 1 PlaySEOneShot · 型 2 PlayVoice(1.0) · 型 3
//! SamplePlaySE(值×0.01) · 型 4 PlayVoiceFixedVolume(值×0.01)。本仓
//! 对应物：**型 1 走真 SE 请求队列**（cue 未提取 ⇒ 通道侧每 cue 一次的
//! 缺流告警就是诚实行）；型 2/3/4 **无渠道对应物**（voice 通道词表只有
//! 对话行；SE 请求无逐请求音量位）——具名不播，记行不静默。
//!
//! ## 落盘与弃置律
//!
//! - **OK / 合法框外点按**：`UpdateLoacalData`（六值 ×0.01 写档对象）→ 落盘读回校验
//!   （`crate::audio::save_volume_settings`）→ 关框。
//! - **关闭钮**：保留当前草稿弃置行为、
//!   **已施加的音量不回滚**（施加即时、无回滚支，真源同形）、档对象与
//!   盘上档都不动。
//!
//! 持久化形态在 `crate::audio` 的本地档段（ApplicationLocalSettings 音量
//! 半：native 用户数据目录文件 / wasm localStorage；键形照真源序列化键
//! Bgm/Se/Voice 与字段名 LiveVolume/SystemVolume）。
//!
//! ## 附加件收口
//!
//! 静音/独奏/恢复默认：真源音量页**无**这三样（SoundManager 的
//! SetMute/SetCategoryVolume 写者只在 streaming live 域——逐调用点文件
//! 穷举；MysekaiPlayerInfo.SetMute(bool) 是同名异物：多人聊天的玩家
//! userId 静音，不碰音频；恢复默认无对应方法）⇒ 具名不做。
//!
//! ## 我方选值（改这里之前先读）
//!
//! 预制体与资产字符串未提取 ⇒ 摆位、配色、文案全是**我方选值**（页签/
//! 组头/滑杆标签为自写文案，提取到后整组替换）。参照画布 1920×1080，
//! y 向上；对话框渲染走 order-2 覆盖相机（Dialog 槽，件 z ≥ 20，盖过
//! 小地图内容 z ≤ 1）。
//!
//! ## 模态与已知偏差
//!
//! 开着时点按族 End 先过本模块（Dialog 槽 blockRaycasts 同形；外壳与
//! 菜单对话框的点按门读同一个 option_open 位）。已知偏差与菜单对话框
//! 同款（既有具名挂账家族「输入让位门在各自域」）：相机拖拽、摇杆、
//! 对话 tap 族不读对话框态，框开着照样收输入——本模块不动各域判定面；
//! 动作按钮的点按消费先于本模块（同菜单对话框）。
//!
//! ## 冒烟口
//!
//! `MOLY_OPTION_AUTOSMOKE_SECS` 给秒数后按 2 秒一拍七步：开框 → 系统
//! SE 减一（施加 + 预览入队）→ 系统 BGM 减一（施加；BGM 账目行随周期
//! 账目现新值）→ Live SE 减一（无总线效果 + 预览无渠道行）→ OK（落盘
//! + 读回一致 + 关框）→ 再开框（草稿从档取——档内值现于开框行 = 当跑
//! 读回）→ 框外点按（提交关框）。**两跑重启读回法**：`MOLY_SETTINGS_FILE`
//! 指到隔离路径跑一遍（OK 落盘），同路径再跑一遍——第二跑的装载行现出
//! 第一跑存的值，即重启读回判据。

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::action_button::ActionTapConsumed;
use crate::audio::{
    apply_system_volume, save_volume_settings, stop_voice_all, LocalVolumeSettings, SeClass,
    SeRequest, SeRequests, VoiceChannel, VolumeBus, VolumeSettingData,
};
use crate::balloon::canvas_scale;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;

// ---------------------------------------------------------------------------
// 身份件：滑杆 · 页签 · 组（屏位与真源参数）
// ---------------------------------------------------------------------------

/// 六滑杆之一（SetupObject 逐个对应，见模块头表）。序即草稿下标序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VolumeSlider {
    LiveBgm,
    LiveSe,
    LiveVoice,
    SystemBgm,
    SystemSe,
    SystemVoice,
}

/// 六滑杆全集（序即草稿下标）。
const SLIDERS: [VolumeSlider; 6] = [
    VolumeSlider::LiveBgm,
    VolumeSlider::LiveSe,
    VolumeSlider::LiveVoice,
    VolumeSlider::SystemBgm,
    VolumeSlider::SystemSe,
    VolumeSlider::SystemVoice,
];

/// 组（本地档两组：LiveVolume / SystemVolume）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    Live,
    System,
}

impl Group {
    /// 组头文案（字段名族的自写标签，资产字符串未提取）。
    fn header(self) -> &'static str {
        match self {
            Group::Live => "Live",
            Group::System => "System",
        }
    }

    /// 组在本地档里的那一片。
    fn data<'a>(self, settings: &'a LocalVolumeSettings) -> &'a VolumeSettingData {
        match self {
            Group::Live => &settings.live,
            Group::System => &settings.system,
        }
    }
}

/// 滑杆的三类之一（BGM/SE/Voice——两组各三杆）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    Bgm,
    Se,
    Voice,
}

/// 预览声型（SetupObject 的型值，方法体直读）：0 BGM（cue 空，静默）·
/// 1 SE（PlaySEOneShot）· 2 Voice（PlayVoice）· 3 InGameSE（SamplePlaySE，
/// 音量 = 值×0.01）· 4 InGameVoice（PlayVoiceFixedVolume，音量 = 值×0.01）。
enum PreviewKind {
    Bgm,
    Se,
    Voice,
    IngameSe,
    IngameVoice,
}

impl PreviewKind {
    /// 账目行用的型名。
    fn label(self) -> &'static str {
        match self {
            PreviewKind::Bgm => "型 0 BGM",
            PreviewKind::Se => "型 1 SE",
            PreviewKind::Voice => "型 2 Voice",
            PreviewKind::IngameSe => "型 3 InGameSE",
            PreviewKind::IngameVoice => "型 4 InGameVoice",
        }
    }
}

impl VolumeSlider {
    fn group(self) -> Group {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::LiveSe | VolumeSlider::LiveVoice => Group::Live,
            VolumeSlider::SystemBgm | VolumeSlider::SystemSe | VolumeSlider::SystemVoice => {
                Group::System
            }
        }
    }

    fn kind(self) -> SliderKind {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => SliderKind::Bgm,
            VolumeSlider::LiveSe | VolumeSlider::SystemSe => SliderKind::Se,
            VolumeSlider::LiveVoice | VolumeSlider::SystemVoice => SliderKind::Voice,
        }
    }

    /// 杆标签（组内三类名，自写）。
    fn kind_label(self) -> &'static str {
        match self.kind() {
            SliderKind::Bgm => "BGM",
            SliderKind::Se => "SE",
            SliderKind::Voice => "Voice",
        }
    }

    /// 草稿下标。
    fn index(self) -> usize {
        match self {
            VolumeSlider::LiveBgm => 0,
            VolumeSlider::LiveSe => 1,
            VolumeSlider::LiveVoice => 2,
            VolumeSlider::SystemBgm => 3,
            VolumeSlider::SystemSe => 4,
            VolumeSlider::SystemVoice => 5,
        }
    }

    /// 延时预览的 cue（SetupObject 逐个；BGM 两杆 cue 空 ⇒ 预览静默）。
    fn cue(self) -> Option<&'static str> {
        match self {
            VolumeSlider::LiveSe => Some("SE_VOLCHANGE_SE_INGAME"),
            VolumeSlider::LiveVoice => Some("SE_VOLCHANGE_VOX_INGAME"),
            VolumeSlider::SystemSe => Some("SE_VOLCHANGE_SE_UI"),
            VolumeSlider::SystemVoice => Some("SE_VOLCHANGE_VOX_SCENARIO"),
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => None,
        }
    }

    fn preview_kind(self) -> PreviewKind {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => PreviewKind::Bgm,
            VolumeSlider::LiveSe => PreviewKind::IngameSe,
            VolumeSlider::LiveVoice => PreviewKind::IngameVoice,
            VolumeSlider::SystemSe => PreviewKind::Se,
            VolumeSlider::SystemVoice => PreviewKind::Voice,
        }
    }

    /// 开框装值：档值 ×100 取整（真源 Setup：滑杆装 `(int)(值×100)` 的
    /// 整数位）。
    fn initial(self, settings: &LocalVolumeSettings) -> u8 {
        let value = match self.kind() {
            SliderKind::Bgm => self.group().data(settings).bgm,
            SliderKind::Se => self.group().data(settings).se,
            SliderKind::Voice => self.group().data(settings).voice,
        };
        (value * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// 草稿的一组 → 档值（OK 的 UpdateLoacalData：六值 ×0.01 写档对象）。
fn draft_group(draft: &[u8; 6], group: Group) -> VolumeSettingData {
    let (bgm, se, voice) = match group {
        Group::Live => (
            draft[VolumeSlider::LiveBgm.index()],
            draft[VolumeSlider::LiveSe.index()],
            draft[VolumeSlider::LiveVoice.index()],
        ),
        Group::System => (
            draft[VolumeSlider::SystemBgm.index()],
            draft[VolumeSlider::SystemSe.index()],
            draft[VolumeSlider::SystemVoice.index()],
        ),
    };
    VolumeSettingData {
        bgm: bgm as f32 * 0.01,
        se: se as f32 * 0.01,
        voice: voice as f32 * 0.01,
    }
}

/// 延时预览的延迟（真源 CP_DelayCall 的 0.15s 字面量）：每拍滑杆变更
/// 重排（b__0 先取消在途再重排）——停手 0.15s 后才响预览声。
const PREVIEW_DELAY_SECONDS: f32 = 0.15;

/// 页签（页枚举 Live=0 · Volume=1 · System=2 · Communication=3；序即
/// 摆位序，本仓只建音量页）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OptionTab {
    Live,
    Volume,
    System,
    Communication,
}

/// 页签全集（摆位序）。
const TABS: [OptionTab; 4] = [
    OptionTab::Live,
    OptionTab::Volume,
    OptionTab::System,
    OptionTab::Communication,
];

impl OptionTab {
    /// 页名（真源页枚举名，账目行用）。
    fn source_name(self) -> &'static str {
        match self {
            OptionTab::Live => "Live",
            OptionTab::Volume => "Volume",
            OptionTab::System => "System",
            OptionTab::Communication => "Communication",
        }
    }

    /// 页签文案（自写：资产字符串未提取）。
    fn label(self) -> &'static str {
        match self {
            OptionTab::Live => "演出",
            OptionTab::Volume => "音量",
            OptionTab::System => "系统",
            OptionTab::Communication => "通信",
        }
    }

    /// 该页在本仓建了没有（当前唯一已建：音量页）。
    fn built(self) -> bool {
        matches!(self, OptionTab::Volume)
    }
}

/// 字符集（图集的**第七个**消费者）。动态值只有 0..100 的整数，无自由
/// 文本 ⇒ 装载期可枚举全量。
pub(crate) const FIXED_TEXTS: &[&str] = &[
    "设置",
    "选项",
    "演出",
    "音量",
    "系统",
    "通信",
    "Live",
    "System",
    "BGM",
    "SE",
    "Voice",
    "-",
    "+",
    "OK",
    "关闭",
    "0123456789",
];

// ---------------------------------------------------------------------------
// 实体与资源
// ---------------------------------------------------------------------------

/// 对话框总根（Dialog 槽：order-2 覆盖相机，件 z ≥ 20；开 = 外壳对话框
/// 态的 option_open 位）。
#[derive(Component)]
pub(crate) struct OptionDialogRoot;

/// 选项对话框运行态。
#[derive(Resource, Default)]
pub(crate) struct OptionDialogState {
    /// 六滑杆的整数草稿（0..=100；开框沿从档值 ×100 取整初始化）。
    draft: [u8; 6],
    /// 在途的延时预览（b__0 的重排结果）：(到点 elapsed 秒, 滑杆)。
    pending: Option<(f32, VolumeSlider)>,
    /// 正在拖动的滑杆（Drag Began 在轨道上 → Some；End → None）。
    dragging: Option<VolumeSlider>,
}

/// 铺装闩（视图一次铺成后置）。
#[derive(Resource, Default)]
pub(crate) struct OptionDialogSpawned;

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

pub(crate) fn init(mut commands: Commands) {
    commands.init_resource::<OptionDialogState>();
}

// ---------------------------------------------------------------------------
// Update：铺件（图集到齐一次）
// ---------------------------------------------------------------------------

/// Build the source dialog once; the product settings panel owns its entry.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<crate::ui_layout::UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<OptionDialogSpawned>>,
) {
    if spawned.is_some() || !layouts.ready("Option", &server) {
        return;
    }
    commands.spawn((OptionDialogRoot, Visibility::Hidden, Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER), crate::ui_layout::UiPrefabView::new("Option", SITEMAP_LAYER)));
    commands.insert_resource(OptionDialogSpawned);
}

// ---------------------------------------------------------------------------
// Update：摆位与开关沿
// ---------------------------------------------------------------------------

/// 开框沿（Setup 的同形日志）：开局页 · 页签装值 · 六滑杆逐杆装值（档值
/// ×100 取整 + 声型与预览 cue）。
fn on_open(settings: &LocalVolumeSettings, draft: &[u8; 6]) {
    info!(
        "[option] 开框：选项对话框（Dialog 槽，Common1ButtonDialog 一钮形，allowCloseExternal=true）\
         → Setup(tabIndex=1 音量页)——开局页取音量页是本仓选值（真源开局页 Live=0，演出域未建）"
    );
    info!(
        "[option] 页签装值：演出(Live=0) 置灰未建 · 音量(Volume=1) 当前页 · 系统(System=2) 置灰未建 · \
         通信(Communication=3) 置灰未建（本仓只建音量页，其余三页具名挂账）"
    );
    for which in SLIDERS {
        let stored = match which.kind() {
            SliderKind::Bgm => which.group().data(settings).bgm,
            SliderKind::Se => which.group().data(settings).se,
            SliderKind::Voice => which.group().data(settings).voice,
        };
        info!(
            "[option]   滑杆 {}·{}：装值 {}（档 {} 组 {:.2} ×100 取整 · {} · 预览 cue {}）",
            which.group().header(),
            which.kind_label(),
            draft[which.index()],
            which.group().header(),
            stored,
            which.preview_kind().label(),
            which.cue().unwrap_or("空（型 0 静默）"),
        );
    }
}

/// Update：摆位与逐帧状态。每帧——
/// 1. 开关沿（开框沿：草稿从档取整 + Setup 行组；关框沿：清拖动）；
/// 2. 在途延时预览到点即放（b__1 的 0.15s；不问开关态——真源 DelayCall
///    不随框撤，此处开框沿清拍为简化，具名）；
/// 3. 对话框根可见性 = option_open 位；
/// 4. 逐件：填充/柄位按草稿摆，±钮按界置灰，页签按已建置灰，文案变更
///    整组重建。
#[allow(clippy::type_complexity)]
pub(crate) fn place(
    mut commands: Commands, windows: Query<&Window, With<PrimaryWindow>>, time: Res<Time>,
    dialog: Res<ShellDialogState>, settings: Res<LocalVolumeSettings>,
    mut state: ResMut<OptionDialogState>, mut voice: ResMut<VoiceChannel>, mut se_requests: ResMut<SeRequests>,
    mut roots: Query<(&mut Visibility, &mut Transform, &mut crate::ui_layout::UiPrefabView), With<OptionDialogRoot>>,
    mut was_open: Local<bool>,
) {
    let open = dialog.option_open;
    match (*was_open, open) {
        (false, true) => {
            // 开框沿：草稿从档取整（真源 Setup：滑杆装档值 ×100）。
            for which in SLIDERS {
                state.draft[which.index()] = which.initial(&settings);
            }
            // 弃置在途预览与拖动：重开框的新 Setup 不受旧拍尾巴干扰
            //（真源 DelayCall 挂管理器不随框撤，此处简化，具名）。
            state.pending = None;
            state.dragging = None;
            on_open(&settings, &state.draft);
        }
        (true, false) => {
            state.dragging = None;
        }
        _ => {}
    }
    *was_open = open;

    // 在途延时预览到点（b__1 的 0.15s）。
    if let Some((at, which)) = state.pending {
        if time.elapsed_secs() >= at {
            state.pending = None;
            fire_preview(
                &mut commands,
                which,
                state.draft[which.index()],
                &mut voice,
                &mut se_requests,
            );
        }
    }

    let Ok(window) = windows.single() else { return; };
    let scale = canvas_scale(window.width(),window.height());
    for (mut visible,mut transform,mut view) in &mut roots {
        *visible = if open {Visibility::Inherited}else{Visibility::Hidden}; transform.scale=Vec3::splat(scale);
        for tab in TABS {view.set_visible(&format!("ContentRoot/Content/{}",tab.source_name()),tab==OptionTab::Volume);}
        for (line,on) in [("@135036",true),("@68777",false),("@62301",true),("@44320",true)] {view.set_visible(line,on);}
        for (cover,on) in [("@128425",true),("@57742",false),("@72401",true),("@50041",true)] {view.set_visible(cover,on);}
        for which in SLIDERS {
            let path=slider_root(which); let value=state.draft[which.index()];
            view.set_text(&format!("{path}/NumText"),value.to_string());
            view.set_slider(&format!("{path}/UIPartsSlider"),value as f32/100.0);
            view.set_visible(&format!("{path}/UIPartsDecrementButton/Cover"),value==0);
            view.set_visible(&format!("{path}/UIPartsIncrementButton/Cover"),value==100);
        }
    }
}

/// 延时预览到点的一拍（b__1）：cue 空（BGM 两杆）静默返回；cue 非空先
/// `StopVoiceAll` 再按型播——型 1 走真 SE 队列，型 2/3/4 无渠道对应物
/// 具名不播（见模块头）。
fn fire_preview(
    commands: &mut Commands,
    which: VolumeSlider,
    value: u8,
    voice: &mut VoiceChannel,
    se_requests: &mut SeRequests,
) {
    let Some(cue) = which.cue() else {
        return; // 型 0（BGM 两杆）：cue 空 ⇒ 预览静默返回（真源同支）
    };
    // StopVoiceAll 先于预览声（真源序：cue 非空即先停语音，型别在后）。
    stop_voice_all(voice, commands);
    let who = format!("{}·{}", which.group().header(), which.kind_label());
    match which.preview_kind() {
        PreviewKind::Se => {
            // 型 1（PlaySEOneShot）⇒ 真 SE 请求队列。cue 未提取 ⇒ 通道侧
            // 每 cue 一次的缺流告警就是它的诚实行（ExistsCueName
            // fail-closed 同款）。
            se_requests.0.push(SeRequest {
                cue: cue.into(),
                class: SeClass::Ui,
                source: "option_preview",
            });
            info!(
                "[option] 延时预览（0.15s 到点，b__1）：{who} → 型 1 PlaySEOneShot({cue}) 入 SE 队列\
                 （值 {value}；cue 未提取 ⇒ 通道侧具名跳过）"
            );
        }
        PreviewKind::Voice => {
            info!(
                "[option] 延时预览（0.15s 到点，b__1）：{who} → 型 2 PlayVoice({cue}, 1.0)——voice 通道\
                 词表只有对话行（talk voice/partvoice），无对应渠道，具名不播（值 {value}）"
            );
        }
        PreviewKind::IngameSe => {
            info!(
                "[option] 延时预览（0.15s 到点，b__1）：{who} → 型 3 SamplePlaySE({cue}, {:.2})——SE 请求\
                 无逐请求音量位，无对应渠道，具名不播",
                value as f32 * 0.01
            );
        }
        PreviewKind::IngameVoice => {
            info!(
                "[option] 延时预览（0.15s 到点，b__1）：{who} → 型 4 PlayVoiceFixedVolume({cue}, {:.2})——\
                 同型 3，无对应渠道，具名不播",
                value as f32 * 0.01
            );
        }
        PreviewKind::Bgm => unreachable!("cue 非空的滑杆不会落在型 0"),
    }
}

// ---------------------------------------------------------------------------
// Update：点按与拖动（模态先手）
// ---------------------------------------------------------------------------

/// 点按屏位（顶为原点）→ 参照画布坐标。
fn to_canvas(position: Vec2, width: f32, height: f32, scale: f32) -> Vec2 {
    Vec2::new(position.x - width / 2.0, height / 2.0 - position.y) / scale
}

fn slider_root(which: VolumeSlider) -> String {
    let group=if which.index()<3 {"LiveVolume"}else{"SystemVolume"};
    let suffix=match which.index()%3 {0=>"",1=>" (1)",_=>" (2)"};
    format!("Volume/ScorollView/Viewport/Content/{group}/UIPartsSliderLabelContent{suffix}/UIPartsSelectCost")
}
fn source_hit(canvas:Vec2,path:&str,layouts:&crate::ui_layout::UiLayouts,view:&crate::ui_layout::UiPrefabView,size:Vec2)->bool {
    view.rect(layouts,path,size).is_some_and(|r|r.active&&r.contains(canvas))
}
fn track_slider_at(canvas: Vec2, layouts:&crate::ui_layout::UiLayouts,view:&crate::ui_layout::UiPrefabView,size:Vec2) -> Option<VolumeSlider> {
    SLIDERS.iter().copied().find(|which|source_hit(canvas,&format!("{}/UIPartsSlider",slider_root(*which)),layouts,view,size))
}
fn value_from_x(which: VolumeSlider, x: f32, layouts:&crate::ui_layout::UiLayouts,view:&crate::ui_layout::UiPrefabView,size:Vec2) -> u8 {
    let Some(rect)=view.rect(layouts,&format!("{}/UIPartsSlider/HandleSlideArea",slider_root(which)),size) else {return 0;};
    let t=(x-(rect.center().x-rect.size.x*0.5))/rect.size.x;
    (t*100.0).round().clamp(0.0,100.0) as u8
}

/// 滑杆变更的整链（b__0）：写草稿 → 重排 0.15s 延时预览 → UpdateVolume
/// 立即（只读系统三滑杆——Live 杆也照走这条，总线值不变即无施加行）。
fn set_slider(
    state: &mut OptionDialogState,
    bus: &mut VolumeBus,
    now: f32,
    which: VolumeSlider,
    value: u8,
    cause: &str,
) {
    let old = state.draft[which.index()];
    if old == value {
        return; // 整数控件同值不回调（Unity Slider 同值不派发）
    }
    state.draft[which.index()] = value;
    state.pending = Some((now + PREVIEW_DELAY_SECONDS, which));
    let system = draft_group(&state.draft, Group::System);
    apply_system_volume(
        bus,
        &system,
        "选项页 UpdateVolume（b__0：重排延时预览后立即施加，只读系统三滑杆）",
    );
    if which.group() == Group::Live {
        info!(
            "[option] {}·{} 滑杆：{old} → {value}（{cause}）——LiveVolume 不进施加（UpdateVolume 只读\
             系统三滑杆；档值在保存关闭时随 Live 组落盘）",
            which.group().header(),
            which.kind_label()
        );
    } else {
        info!(
            "[option] {}·{} 滑杆：{old} → {value}（{cause}）",
            which.group().header(),
            which.kind_label()
        );
    }
}

/// 保存关闭入口共用六值提交，不改变音量施加与存储失败策略。
fn commit_draft(settings: &mut LocalVolumeSettings, draft: &[u8; 6], cause: &str) {
    let next = LocalVolumeSettings {
        live: draft_group(draft, Group::Live),
        system: draft_group(draft, Group::System),
    };
    info!(
        "[option] {cause} → UpdateLoacalData：六值 ×0.01 写档对象（Live{{bgm {:.2}, se {:.2}, voice {:.2}}} · \
         System{{bgm {:.2}, se {:.2}, voice {:.2}}}）→ 落盘 → 关框",
        next.live.bgm,
        next.live.se,
        next.live.voice,
        next.system.bgm,
        next.system.se,
        next.system.voice,
    );
    *settings = next;
    save_volume_settings(&next);
}

/// 框内点按分派（模态消费在外层记）。命中优先序：关闭 → OK → 页签 →
/// 六杆的 ±钮 → 轨道/柄（点按落值）→ 面板空白（吃掉不动作）→ 框外
/// （提交收框）。
fn dispatch_tap(
    layouts: &crate::ui_layout::UiLayouts, view:&crate::ui_layout::UiPrefabView, size:Vec2,
    canvas: Vec2,
    now: f32,
    settings: &mut LocalVolumeSettings,
    state: &mut OptionDialogState,
    bus: &mut VolumeBus,
    close: &mut Option<&'static str>,
) {
    if source_hit(canvas,"WindowRoot/UIPartsCloseButton",layouts,view,size) {
        info!(
            "[option] 关闭钮按下 → 关框——草稿弃置、已施加的音量不回滚（施加即时无回滚支，真源同形）、档不动"
        );
        *close = Some("关闭钮");
        return;
    }
    if source_hit(canvas,"FooterButtons/UIPartsCommonButton",layouts,view,size) {
        commit_draft(settings, &state.draft, "OK 按下");
        *close = Some("OK");
        return;
    }
    for tab in TABS {
        if source_hit(canvas,match tab {OptionTab::Live=>"@15056",OptionTab::Volume=>"@79073",OptionTab::System=>"@85519",OptionTab::Communication=>"@105193"},layouts,view,size) {
            if tab.built() {
                info!("[option] 页签「{}」按下：当前页（音量页已建）", tab.label());
            } else {
                info!(
                    "[option] 页签「{}」按下：置灰不响应（{} 页未建，具名挂账——本仓只建音量页）",
                    tab.label(),
                    tab.source_name()
                );
            }
            return;
        }
    }
    for which in SLIDERS {
        let value = state.draft[which.index()];
        if source_hit(canvas,&format!("{}/UIPartsDecrementButton",slider_root(which)),layouts,view,size) {
            if value > 0 {
                set_slider(state, bus, now, which, value - 1, "减量钮 ChangeItemSelectNum(-1) 界内钳");
            } else {
                info!(
                    "[option] {}·{} 减量钮置灰不响应（UpdateButtonEnable：Selected>Min 才亮，已在 0）",
                    which.group().header(),
                    which.kind_label()
                );
            }
            return;
        }
        if source_hit(canvas,&format!("{}/UIPartsIncrementButton",slider_root(which)),layouts,view,size) {
            if value < 100 {
                set_slider(state, bus, now, which, value + 1, "增量钮 ChangeItemSelectNum(+1) 界内钳");
            } else {
                info!(
                    "[option] {}·{} 增量钮置灰不响应（UpdateButtonEnable：Selected<Max 才亮，已在 100）",
                    which.group().header(),
                    which.kind_label()
                );
            }
            return;
        }
    }
    // 轨道/柄点按：落值到按点位（Unity Slider 按下落值同形；柄内按下
    // 不跳变的细枝不设，具名）。
    if let Some(which) = track_slider_at(canvas,layouts,view,size) {
        set_slider(state, bus, now, which, value_from_x(which, canvas.x,layouts,view,size), "轨道点按落值");
        return;
    }
    if source_hit(canvas,"WindowRoot",layouts,view,size) {
        return; // 面板空白：吃掉不动作（模态消费已在外层记）
    }
    commit_draft(
        settings,
        &state.draft,
        "框外点按收框（模态；allowCloseExternal=true）",
    );
    *close = Some("框外点按");
}

/// 点按与拖动分派。对话框开着 ⇒ 模态（点按族 End 全被本模块吃掉，真源
/// Dialog 槽 blockRaycasts 同形；外壳与菜单对话框的门都读同一个
/// option_open 位）；拖动在轨道上起拖落值、拖动中续值。
pub(crate) fn click(
    layouts: Res<crate::ui_layout::UiLayouts>,
    views: Query<&crate::ui_layout::UiPrefabView,With<OptionDialogRoot>>,
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    mut dialog: ResMut<ShellDialogState>,
    mut settings: ResMut<LocalVolumeSettings>,
    mut bus: ResMut<VolumeBus>,
    mut state: ResMut<OptionDialogState>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let events: Vec<GestureEvent> = gestures.read().cloned().collect();
    if events.is_empty() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = canvas_scale(width, height);
    let now = time.elapsed_secs();

    if !dialog.option_open { return; }
    let Ok(view)=views.single() else {return;};
    let size=Vec2::new(width,height)/scale;
    // ---- 开态：模态 ----
    let mut close = None::<&'static str>;
    for event in &events {
        let canvas = to_canvas(event.position, width, height, scale);
        match event.kind {
            // 拖动：轨道上起拖落值，拖动中续值，抬手收拖（CustomSlider =
            // Unity Slider 子类：按下落值到按点位、拖动续值）。
            GestureKind::Drag => match event.state {
                GestureState::Began => {
                    if let Some(which) = track_slider_at(canvas,&layouts,view,size) {
                        state.dragging = Some(which);
                        set_slider(
                            &mut state,
                            &mut bus,
                            now,
                            which,
                            value_from_x(which, canvas.x,&layouts,view,size),
                            "轨道按下落值",
                        );
                    }
                }
                GestureState::Moved => {
                    if let Some(which) = state.dragging {
                        set_slider(
                            &mut state,
                            &mut bus,
                            now,
                            which,
                            value_from_x(which, canvas.x,&layouts,view,size),
                            "拖动续值",
                        );
                    }
                }
                GestureState::End => {
                    if let Some(which) = state.dragging.take() {
                        set_slider(
                            &mut state,
                            &mut bus,
                            now,
                            which,
                            value_from_x(which, canvas.x,&layouts,view,size),
                            "拖动收尾",
                        );
                    }
                }
            },
            // 点按族 End：模态吃掉 + 分派（每拍物理交互恰一事件，双击的
            // 第二拍以 DoubleTap 型到达，不与 Tap 重复）。
            GestureKind::Tap | GestureKind::DoubleTap | GestureKind::LongTouch
                if event.state == GestureState::End =>
            {
                consumed.0 = true;
                dispatch_tap(&layouts,view,size,canvas, now, &mut settings, &mut state, &mut bus, &mut close);
            }
            _ => {}
        }
    }
    if close.is_some() {
        dialog.option_open = false;
    }
}
