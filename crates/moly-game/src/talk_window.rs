//! 对话窗体：固定窗呈现侧（真源 `ScreenLayerMysekaiTalk` 的宿主消费）。
//!
//! 真源里窗体引擎服务**全部引擎播的对话**：玩家（SD）对话与配对剧情
//! （含多角色 fixture 对话）同走 `ShowTalkWindow`/`HideTalkWindow`/
//! `ShowTextAsync` 这扇窗；tweet 一类世界内文本不经窗体引擎、留在头顶
//! 气泡（[`crate::balloon`]）。两条链共用这扇窗与同一套状态机（开场/
//! 收场/淡变/打字机/点击闩）。本模块只做呈现与状态机半，步进/选取/
//! 驻留在消费律侧（玩家链 [`crate::player_talk`]、配对链 [`crate::talk`]）。
//!
//! **归属构造**：状态机的全部变迁方法（开场/收场/淡变/正文/名字栏/
//! 置闩/耗闩）都要一条 [`TalkSession`] 作保——构造即借用（枚举里握着
//! 会话的 `&mut`），编译器保证独占。`TalkSession` 是两条链的会话枚举
//! （见下），任一链在播都给得出所有权；两链互斥由各自入口守卫保证
//! （配对选取与双注入口都在玩家会话在场时让行，反之亦然），同一帧两
//! 链都在播是缺陷，输入面对
//! 此响亮 panic。置闩的输入面按会话在场门控：无会话的点击不进闩
//! （发起点击被发起消费；无对话期间的点击对窗体惰性）。
//!
//! **布局与样式**（canvas 单位、y 向上）：下列既有部件坐标以参考
//! 1920×1080 的屏幕中心为原点。实际摆位由提取的 talkBG 锚点决定，
//! 整组按其相对参考中心的位移移动，保持原有文字与面板的局部关系。
//! * **面板**（talkBG）：中心 (0,−320)、1600×312；源图 `bg_window`
//!   312×312、四边 border 156——border 恰为源图半边，九宫退化成
//!   「四角原生 + 上下两条横带」（中段源宽 312−2×156=0，横带按 border
//!   线上的 1px 采样列拉伸；左右边带与中心高 0，无可画）。
//! * **名字栏**（LabelText）：盒 x[−694,−118]、y[−256,−212]，字号 44
//!   （autosize 关）、色 RGB(68,68,102)、字距/词距 0。
//! * **正文**（ContentText）：盒 x[−696,684]、y[−432,−278]，字号 40
//!   （autosize 22–44 开——语料最宽行 34 字 ≈ 1373px < 盒宽 1380、
//!   三行 ≈ 115px < 盒高 154，**不触发缩字号**，按 40 定格并在超盒时
//!   响亮告警）、色 RGB(85,85,119)、字距 1.0/词距 9.73（消费式：
//!   值 × 0.01 × 字号，正交文本 ×1 ⇒ 0.4px/字、3.892px/空白）。
//!   两栏水平居左、垂直居顶、行距 −80、margin 0——行量法与摆位全按
//!   排版律（`moly_law::text`），字距增量在摆位侧逐字加。
//! * **尾标**（EndSign）：中心 (718,−434) 100×100 组，可见标记是
//!   50×44 的 `icon_pageForward_gn`（ScenarioAtlas 未解码——程序化白色
//!   下指三角替身，换图即真）；同组 12×12 星标与缩放/位移动效是装饰
//!   （替身缺动效，具名）。打字机收尾拍才激活。
//!
//! **时序**：
//! * **打字机**（`ShowTextAsync`）：第 0 字在 t=0 即现，第 k 字在 50k ms，
//!   末字后再过一拍进 DONE（尾标激活、在播清零）；空文本 t=0 即 DONE。
//!   跳过支把全文一次揭示并**清点击闩**——跳过与放行是两次点击
//!   （打字中点击只跳字，放行要等打字收尾再点）。
//! * **淡变**：窗体 CanvasGroup 序列化初值 α=1 ⇒ 对话开始的 show 步被
//!   `Approximately(α,1)` 短路（即时在场）；hide 步无条件 `DOFade` 0.5s
//!   到 0，被 hide 过再 show 走 0.3s 淡入；缓动取补间库缺省（OutQuad）。
//! * **收口**：层弹出路径不带淡出——对话收尾即时撤窗。
//!
//! **具名缺口**（真源有、此处不做假）：面板后方的全宽暗带（`bg_story_adv`，
//! ScenarioAtlas 未解码）；尾标星形动效与 Animator 状态；段落距
//! （m_paragraphSpacing 5.16——只作用于 U+2029 段落分隔，语料 0 个，
//! 恒不触发）；软换行（`wordWrapping` 开，但排版律无软换行且语料行宽
//! 全部在盒内，硬断行即全部形态）。
//!
//! 日志口径同对话域：只带 id/字数/步号/量法值——文本内容与名字栏文字
//! 不进日志（语言网关纪律）。

use bevy::asset::{AssetPath, LoadState};
use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use moly_law::text::{layout_metrics, LayoutMetrics};

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::balloon::{
    canvas_scale, walk_glyphs, BalloonArt, GlyphSpot, ASCENT_RATIO, BAKE_PPEM, BALLOON_LAYER,
    FONT_FAMILY, TEXT_SCALE,
};
use crate::gesture::{GestureEvent, GestureState};
use crate::ui_layout::UiLayouts;

// ---------------------------------------------------------------------------
// 会话归属
// ---------------------------------------------------------------------------

/// 窗体归属的会话侧：玩家对话与配对剧情两条链共用这扇窗，步进所在的
/// 链不同。变迁方法只要一条 `&mut` 作保——借用即证明独占，同一帧只有
/// 一条链能驱动窗体（两链互斥由各自入口守卫保证，见模块注释；同帧
/// 双链在播是缺陷，输入面 panic）。
pub(crate) enum TalkSession<'a> {
    Player(&'a mut crate::player_talk::PlayerTalkSession),
    Pair(&'a mut crate::talk::ActiveTalk),
}

impl TalkSession<'_> {
    /// 链名（日志用）：归属枚举的读取点——字段构造即借用（独占的证
    /// 明），这里把「哪条链、哪一场」读出来落进日志。
    pub(crate) fn chain_word(&self) -> String {
        match self {
            TalkSession::Player(session) => format!("玩家链 talk {}", session.talk_id()),
            TalkSession::Pair(talk) => format!("配对链 talk {}", talk.talk_id()),
        }
    }
}

// ---------------------------------------------------------------------------
// 常量（canvas 单位；源矩形用页图像坐标、y 从顶量——真源 textureRect 是
// 底原点，翻成顶原点后落在这里）
// ---------------------------------------------------------------------------

/// 面板源图所在页（提取产物的 UI atlas 页 0；文件名含 atlas 与页号）。
const PANEL_PAGE: &str = "moly://ui/atlas/textures/sactx-0-2048x2048-ASTC 4x4-CommonAtlas-d65f3824-00000202.png";

/// 面板：中心 (0,−320)、1600×312。
const PANEL_CENTER: Vec2 = Vec2::new(0.0, -320.0);
const PANEL_W: f32 = 1600.0;
const PANEL_H: f32 = 312.0;
const PANEL_LAYOUT: &str = "Talk";
const PANEL_NODE: &str = "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG";

/// `bg_window` 源图：312×312、四边 border 156，在页上的位置
/// (1232,1532)–(1544,1844)（页 2048×2048，textureRect (1232,204) 312×312
/// 按「页高 − (y+h)」翻成顶原点）。
const PANEL_SRC: Rect = Rect {
    min: Vec2::new(1232.0, 1532.0),
    max: Vec2::new(1544.0, 1844.0),
};
const PANEL_BORDER: f32 = 156.0;

/// 名字栏盒（LabelText）：中心 (−406,−234)、576×44。
const LABEL_RECT: Rect = Rect {
    min: Vec2::new(-694.0, -256.0),
    max: Vec2::new(-118.0, -212.0),
};
const LABEL_FONT: f32 = 44.0;
const LABEL_COLOR: [f32; 3] = [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0];

/// 正文盒（ContentText）：中心 (−6,−355)、1380×154。
const CONTENT_RECT: Rect = Rect {
    min: Vec2::new(-696.0, -432.0),
    max: Vec2::new(684.0, -278.0),
};
const CONTENT_FONT: f32 = 40.0;
const CONTENT_COLOR: [f32; 3] = [85.0 / 255.0, 85.0 / 255.0, 119.0 / 255.0];
/// 正文字距增量：`m_characterSpacing` 1.0 × 0.01 × 字号 40（正交 ×1）。
const CONTENT_CHAR_EXTRA: f32 = 0.4;
/// 正文词距增量：`m_wordSpacing` 9.73 × 0.01 × 字号 40（空白字后补一笔）。
const CONTENT_WORD_EXTRA: f32 = 3.892;

/// 两栏同值：`m_lineSpacing` −80（序列化原值，换算在行量律内部）。
const WINDOW_LINE_SPACING: f32 = -80.0;

/// 尾标可见标记：50×44，中心 (716.7,−416.7)（EndSign 中心 (718,−434)
/// 100×100 组内、pivot(0.5,0) 底托在组心下 4.7）。
const END_ICON_CENTER: Vec2 = Vec2::new(716.7, -416.7);
const END_ICON_W: f32 = 50.0;
const END_ICON_H: f32 = 44.0;

/// show 步淡入时长（readonly 字段 0.3s）。
const SHOW_FADE_SECONDS: f32 = 0.3;
/// hide 步淡出时长（readonly 字段 0.5s）。
const HIDE_FADE_SECONDS: f32 = 0.5;
/// 打字机逐字间隔（`_showingTextIntervalTimeMs` 50ms）。
const TYPE_INTERVAL: f32 = 0.05;

// ---------------------------------------------------------------------------
// 资源与组件
// ---------------------------------------------------------------------------

/// 窗体纹源：面板页（atlas 提取产物）+ 尾标替身三角（程序化）。
#[derive(Resource)]
pub(crate) struct WindowArt {
    page: Handle<Image>,
    end_mark: Handle<Image>,
}

/// 窗体状态机半：真源层的可变字段（闩、打字机、α）+ 呈现侧版本号。
/// 常驻资源（对话不在播也在——真源层对象常驻，show/hide 只动可见性）。
#[derive(Resource)]
pub(crate) struct TalkWindowState {
    /// 窗体是否在场（对话开始置位、收口撤；层压栈/弹出的宿主侧对应物）。
    present: bool,
    /// CanvasGroup α 当前值。
    alpha: f32,
    /// 进行中的淡变 (起点, 目标, 时长, 已放秒)。
    fade: Option<(f32, f32, f32, f32)>,
    /// 名字栏文本（`ResetTexts` 清空、label 步覆写；常驻到下一次覆写）。
    label: String,
    /// 正文全文（`ShowTextAsync` 一次设入，打字机只动揭示游标）。
    text: String,
    /// 已揭示的字符数（真源下标 i 的「已过」侧；换行占原文位但不揭示）。
    shown: usize,
    /// 打字机时钟（秒）。
    typing_clock: f32,
    /// `_isPlayingTextAnimation`。
    playing: bool,
    /// `_isClicked`（点击闩：跳过支与放行门共写）。
    clicked: bool,
    /// wait_click 放行沿待发（步进放行支经 consume_click 置位、本模块帧
    /// 推进取走入一次性 SE 队列——步进主循环的系统参数已满，经状态中转）。
    released: bool,
    /// `_endIconObj` 激活态。
    end_icon: bool,
    /// 正文/名字栏的改版计数（呈现侧按版本重建字形实体）。
    text_version: u64,
    label_version: u64,
    built_text: u64,
    built_label: u64,
}

impl Default for TalkWindowState {
    fn default() -> Self {
        Self {
            present: false,
            alpha: 1.0,
            fade: None,
            label: String::new(),
            text: String::new(),
            shown: 0,
            typing_clock: 0.0,
            playing: false,
            clicked: false,
            released: false,
            end_icon: false,
            text_version: 0,
            label_version: 0,
            built_text: 0,
            built_label: 0,
        }
    }
}

impl TalkWindowState {
    /// `Mathf.Approximately(α,1)` 的容差（浮点 ε×8 一档）。
    fn approximately_one(value: f32) -> bool {
        (value - 1.0).abs() <= 1e-6
    }

    /// 对话开始：层压栈即窗体在场（序列化初值 α=1，无淡入），`ResetTexts`
    /// 清两栏。首个 show 步因此被 Approximately 短路。要一条会话作保
    /// （归属构造，见模块注释——两条链的 [`TalkSession`] 任一）。
    pub(crate) fn open(&mut self, _owner: TalkSession<'_>) {
        self.present = true;
        self.alpha = 1.0;
        self.fade = None;
        self.label.clear();
        self.text.clear();
        self.shown = 0;
        self.typing_clock = 0.0;
        self.playing = false;
        self.clicked = false;
        self.end_icon = false;
        self.text_version += 1;
        self.label_version += 1;
    }

    /// 对话收口：层弹出无淡出，即时撤（α 回序列化初值，下次开场即短路）。
    /// 同上要会话作保。
    pub(crate) fn close(&mut self, _owner: TalkSession<'_>) {
        self.present = false;
        self.alpha = 1.0;
        self.fade = None;
        self.label.clear();
        self.text.clear();
        self.shown = 0;
        self.typing_clock = 0.0;
        self.playing = false;
        self.clicked = false;
        self.end_icon = false;
        self.text_version += 1;
        self.label_version += 1;
    }

    /// show 步：α 已 ≈1 时短路（返回 `None`），否则 0.3s 淡入（返回
    /// 起始 α，日志用）。同上要会话作保。
    pub(crate) fn show(&mut self, _owner: TalkSession<'_>) -> Option<f32> {
        self.present = true;
        if Self::approximately_one(self.alpha) {
            return None;
        }
        let from = self.alpha;
        self.fade = Some((from, 1.0, SHOW_FADE_SECONDS, 0.0));
        Some(from)
    }

    /// hide 步：无条件 0.5s 淡出到 0（窗体仍在场，α=0）。同上要会话作保。
    pub(crate) fn hide(&mut self, _owner: TalkSession<'_>) {
        self.fade = Some((self.alpha, 0.0, HIDE_FADE_SECONDS, 0.0));
    }

    /// 正文进窗（`ShowTextAsync` 状态-0）：闩清、在播置位、尾标收、
    /// 游标 0；第 0 字在 t=0 即揭示。空文本直接 DONE。同上要会话作保。
    pub(crate) fn show_text(&mut self, _owner: TalkSession<'_>, text: &str) {
        self.clicked = false;
        self.text.clear();
        self.text.push_str(text);
        self.playing = true;
        self.end_icon = false;
        self.typing_clock = 0.0;
        self.shown = 0;
        let len = text.chars().count();
        if len == 0 {
            self.playing = false;
            self.end_icon = true;
        } else {
            self.shown = 1;
        }
        self.text_version += 1;
    }

    /// label 步：名字栏 `SetText`（常驻到下一次覆写或开场清空）。同上
    /// 要会话作保。
    pub(crate) fn set_label(&mut self, _owner: TalkSession<'_>, name: &str) {
        self.label.clear();
        self.label.push_str(name);
        self.label_version += 1;
    }

    /// 放行门（`IsWaitClick`）：闩置位且打字机已收尾。
    pub(crate) fn click_level(&self) -> bool {
        self.clicked && !self.playing
    }

    /// 置闩（真实点击或显式启用的冒烟输入）。要一条会话作保——输入
    /// 系统按会话在场门控后，置闩只可能来自某条链在播期间。
    pub(crate) fn latch(&mut self, _owner: TalkSession<'_>) {
        self.clicked = true;
    }

    /// 闩空闲？（会话首拍用来耗尽发起点击，不将它用于正文放行。）
    pub(crate) fn latch_idle(&self) -> bool {
        !self.clicked
    }

    /// 耗尽闩（`WaitClicked` 放行后的清闩——该击即耗尽）。放行沿同时置
    /// 待发 SE 标记（帧推进取走入队）。同上要会话作保。
    pub(crate) fn consume_click(&mut self, _owner: TalkSession<'_>) {
        self.clicked = false;
        self.released = true;
    }

    /// 取走放行沿待发标记（有即返回 true 并清）。仅本模块的帧推进调。
    fn take_released(&mut self) -> bool {
        std::mem::take(&mut self.released)
    }
}

/// 窗体根（挂 canvas 缩放；子件全在 canvas 单位系）。
#[derive(Component)]
pub(crate) struct TalkWindowRoot;

/// 窗体一个 sprite 部件：基础色（α 淡变写回的底）。
#[derive(Component)]
pub(crate) struct TalkWindowPart {
    base: Color,
}

/// 正文一个字形：`order` = 原文下标（打字机按它揭示）。
#[derive(Component)]
pub(crate) struct TalkGlyph {
    order: usize,
}

/// 名字栏一个字形（无打字机，常显）。
#[derive(Component)]
pub(crate) struct TalkLabelGlyph;

/// 尾标（激活态随打字机 DONE 翻转）。
#[derive(Component)]
pub(crate) struct TalkEndMark;

// ---------------------------------------------------------------------------
// 装载与输入
// ---------------------------------------------------------------------------

/// Startup：请求面板页贴图、程序化烘尾标替身、初始化窗体状态（常驻）。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>, mut images: ResMut<Assets<Image>>) {
    let page = server.load::<Image>(AssetPath::from(PANEL_PAGE.to_owned()));
    let end_mark = end_mark_image(&mut images);
    commands.insert_resource(WindowArt { page, end_mark });
    commands.init_resource::<TalkWindowState>();
}

/// Update（对话链内、步进前）：点跳输入最小集——Space 与 tap 族的
/// 收场沿。真源对话引擎的手势门：手势 ∈ {TAP, DOUBLE_TAP, LONG_TOUCH}
/// 且态为 End 即触发，**不问屏位**（手势层是全屏的，本来就没有位
/// 门槛）。相机拖拽在手势层就被折成 DRAG、不属 tap 族——拖完松手
/// 不会误触点跳（此前读裸左键按下沿，拖拽起手即误闩，那正是要由
/// 手势层接管的分工）。真源 `OnClick` 的门（玩家数据在场、UI 未隐藏）
/// 在宿主里的对应物是**任一对话会话在场**（归属构造，见模块注释）：
/// 无会话的点击不进闩——发起点按在会话插上前发生（发起消费该击），
/// 无对话期间的点击对窗体惰性。两条链的会话互斥由入口守卫保证，
/// 同帧双链在场是守卫失守，响亮 panic。
pub(crate) fn read_click_input(
    mut state: ResMut<TalkWindowState>,
    sessions: Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: Option<ResMut<crate::talk::ActiveTalk>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut gestures: MessageReader<GestureEvent>,
) {
    let tap_family_end = gestures.read().any(|event| {
        event.kind.is_tap_family() && event.state == GestureState::End
    });
    if !(tap_family_end || keys.just_pressed(KeyCode::Space)) {
        return;
    }
    match (sessions, talks) {
        (None, None) => {}
        (Some(mut session), None) => {
            let owner = TalkSession::Player(&mut session);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 点击闩置位（{chain}；tap 族收场/Space；打字中即跳字，收尾后放行）"
            );
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 点击闩置位（{chain}；tap 族收场/Space；打字中即跳字，收尾后放行）"
            );
        }
        (Some(_), Some(_)) => {
            panic!("玩家会话与配对会话同时在播：两链互斥的入口守卫失守，fail-closed");
        }
    }
}

/// 冒烟口（`MOLY_TALK_TAP_AT_SECS`，逗号分隔的时刻表；宿主侧仪表，与
/// 采集 autohit 同款）：到点即置闩——与真点击/替身同一条路（打字中即
/// 点跳，收尾后放行）。无头窗口收不到真点击，点跳半边的 SE 验证靠它。
/// 同样按会话在场门控：时刻落在无会话的空档不消费（对窗体惰性，挂到
/// 下一场对话开场后即拍）。
pub(crate) fn smoke_tap(
    mut state: ResMut<TalkWindowState>,
    sessions: Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: Option<ResMut<crate::talk::ActiveTalk>>,
    time: Res<Time>,
    mut schedule: Local<Option<Vec<f32>>>,
    mut next: Local<usize>,
) {
    let times = schedule.get_or_insert_with(|| {
        std::env::var("MOLY_TALK_TAP_AT_SECS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|t| t.trim().parse::<f64>().ok().map(|v| v.max(0.0) as f32))
                    .collect()
            })
            .unwrap_or_default()
    });
    if times.is_empty() || *next >= times.len() {
        return;
    }
    if sessions.is_none() && talks.is_none() {
        return; // 空档：时刻不消费，等下一场对话
    }
    let now = time.elapsed_secs();
    if now < times[*next] {
        return;
    }
    *next += 1;
    match (sessions, talks) {
        (None, None) => {}
        (Some(mut session), None) => {
            let owner = TalkSession::Player(&mut session);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 冒烟点击置闩（{chain}，时刻表第 {} 拍 @ {:.2}s；打字中即点跳，收尾后放行）",
                *next, now
            );
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 冒烟点击置闩（{chain}，时刻表第 {} 拍 @ {:.2}s；打字中即点跳，收尾后放行）",
                *next, now
            );
        }
        (Some(_), Some(_)) => {
            panic!("玩家会话与配对会话同时在播：两链互斥的入口守卫失守，fail-closed");
        }
    }
}

// ---------------------------------------------------------------------------
// 帧推进
// ---------------------------------------------------------------------------

/// Update（对话链内、步进后）：状态机半（跳过/打字机/淡变）+ 呈现
/// （树生死、字形重建、揭示与 α 写回、canvas 缩放）。只在纹源到齐后
/// 动实体；装载失败响亮 panic（资产边界的拒绝点）。
#[allow(clippy::type_complexity)]
pub(crate) fn tick_window(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    server: Res<AssetServer>,
    layouts: Option<Res<UiLayouts>>,
    art: Option<Res<BalloonArt>>,
    window_art: Option<Res<WindowArt>>,
    mut state: ResMut<TalkWindowState>,
    mut se: ResMut<SeRequests>,
    mut roots: Query<(Entity, &mut Transform), With<TalkWindowRoot>>,
    mut glyphs: Query<(Entity, &TalkGlyph, &mut Visibility)>,
    label_glyphs: Query<(Entity, &TalkLabelGlyph)>,
    mut end_marks: Query<&mut Visibility, (With<TalkEndMark>, Without<TalkGlyph>)>,
    mut parts: Query<(&TalkWindowPart, &mut Sprite)>,
) {
    let dt = time.delta_secs();

    // --- 状态机半：跳过支（先于推进——真源逐字循环每拍先查闩） ---
    if state.clicked && state.playing {
        let len = state.text.chars().count();
        state.shown = len;
        state.playing = false;
        state.end_icon = true;
        state.clicked = false;
        // 点跳 SE：真源对话窗 OnClick 体 0 cue（读不出），按 mysekai UI
        // 家族的 select 档具名 mock（按下沿语义，CustomSelectableDefine
        // 同表）；2.0s 抑制在 SE 通道侧。
        se.0.push(SeRequest {
            cue: "se_mysekai_ui_select".into(),
            class: SeClass::Ui,
            source: "talk-skip",
        });
        info!(
            "[talkwin] 点跳：跳过打字机（{len} 字一次揭示），点击闩清零——放行须再点"
        );
    }

    // --- 放行沿 SE：wait_click 放行发生在步进主循环（其系统参数已满，
    // 经状态中转，见 consume_click/take_released）——decision 档具名 mock，
    // 与点跳的 select 档同表同源。 ---
    if state.take_released() {
        se.0.push(SeRequest {
            cue: "se_mysekai_ui_decision".into(),
            class: SeClass::Ui,
            source: "talk-advance",
        });
    }

    // --- 状态机半：打字机推进（第 k 字在 50k ms，DONE 在 50×len ms） ---
    if state.playing {
        state.typing_clock += dt;
        let len = state.text.chars().count();
        let due = ((state.typing_clock / TYPE_INTERVAL).floor() as usize + 1).min(len);
        state.shown = due;
        if state.typing_clock >= TYPE_INTERVAL * len as f32 {
            state.playing = false;
            state.end_icon = true;
            info!(
                "[talkwin] 打字机 DONE：{len} 字 {:.2}s（50ms/字，末字后再一拍），尾标激活，在播清零",
                state.typing_clock
            );
        }
    }

    // --- 状态机半：淡变（OutQuad——补间库缺省） ---
    if let Some((from, target, duration, elapsed)) = state.fade {
        let elapsed = elapsed + dt;
        let t = (elapsed / duration).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        state.alpha = from + (target - from) * eased;
        if elapsed >= duration {
            state.alpha = target;
            state.fade = None;
            info!(
                "[talkwin] 淡变完成：α={:.2}（{:.1}s OutQuad）",
                target, duration
            );
        } else {
            state.fade = Some((from, target, duration, elapsed));
        }
    }

    // --- 呈现：树生死（根至多一棵——只有本系统铺） ---
    let placement = layouts.as_deref().and_then(|layouts| {
        let window = windows.single().ok()?;
        window_root_transform(layouts, Vec2::new(window.width(), window.height()))
    });
    if roots.is_empty() {
        // 无树：该在场且纹源齐了才铺；铺完把两栏版本记到当版（同帧已铺）。
        if state.present {
            if let (Some(art), Some(window_art)) = (art.as_deref(), window_art.as_deref()) {
                match server.load_state(&window_art.page) {
                    LoadState::Failed(err) => {
                        panic!("对话窗面板页装载失败：{err:?}");
                    }
                    LoadState::Loaded => {
                        let Some(placement) = placement else { return; };
                        spawn_tree(&mut commands, art, window_art, &state, placement);
                        state.built_label = state.label_version;
                        state.built_text = state.text_version;
                        info!(
                            "[talkwin] 窗体树上屏：面板 {:.0}x{:.0} @({:.0},{:.0})（九宫退化：border {} = 源半边，四角 + 上下横带）",
                            PANEL_W, PANEL_H, PANEL_CENTER.x, PANEL_CENTER.y, PANEL_BORDER
                        );
                    }
                    _ => {} // 页未到：等（首次装载帧序）
                }
            }
        }
        return;
    }
    let (root, mut root_transform) = roots.single_mut().expect("TalkWindowRoot 至多一棵");
    if !state.present {
        commands.entity(root).despawn();
        return;
    }

    // The source Window is bottom-anchored. Scale alone would keep its
    // reference-frame coordinates around the center of a tall viewport.
    let Some(placement) = placement else { return; };
    *root_transform = placement;

    // --- 呈现：名字栏/正文字形重建（版本落后即重建） ---
    let Some(art) = art.as_deref() else {
        return; // 图集未烘（tweet 主表先到才烘——对话窗共用它的字形）
    };
    if state.built_label != state.label_version {
        for (entity, _) in &label_glyphs {
            commands.entity(entity).despawn();
        }
        spawn_label_glyphs(&mut commands, root, art, &state);
        state.built_label = state.label_version;
        info!(
            "[talkwin] 名字栏重建：{} 字（字号 {LABEL_FONT}，名字不进日志）",
            state.label.chars().count()
        );
    }
    if state.built_text != state.text_version {
        // Text replacement owns the previous glyph entities as well as the string.
        // Otherwise the new reveal cursor makes earlier sentences visible again.
        for (entity, _, _) in &glyphs {
            commands.entity(entity).despawn();
        }
        let (count, lines, widest) = spawn_content_glyphs(&mut commands, root, art, &state);
        state.built_text = state.text_version;
        if widest > CONTENT_RECT.width() {
            warn!(
                "[talkwin] 正文最宽行 {widest:.0}px 超盒宽 {:.0}：真源 autosize（22–44）会缩字号，本实现按 {CONTENT_FONT} 定格（语料最宽 34 字 ≈ 1373 < 1380 不触发——触发即数据超界）",
                CONTENT_RECT.width()
            );
        }
        info!(
            "[talkwin] 正文重建：{count} 字形 {lines} 行，最宽行 {widest:.0}px / 盒宽 {:.0}（打字机 {} 字，揭示 {}）",
            CONTENT_RECT.width(),
            state.text.chars().count(),
            state.shown
        );
    }

    // --- 呈现：揭示与尾标 ---
    for (_, glyph, mut visible) in &mut glyphs {
        *visible = if glyph.order < state.shown {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut visible in &mut end_marks {
        *visible = if state.end_icon {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    // --- 呈现：α 写回（淡变值乘进每个部件的基础色） ---
    let alpha = state.alpha;
    for (part, mut sprite) in &mut parts {
        sprite.color = part.base.with_alpha(part.base.alpha() * alpha);
    }
}

// ---------------------------------------------------------------------------
// 树与字形
// ---------------------------------------------------------------------------

fn window_root_transform(layouts: &UiLayouts, size: Vec2) -> Option<Transform> {
    let scale = canvas_scale(size.x, size.y);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let panel = layouts.rect(PANEL_LAYOUT, PANEL_NODE, size / scale)?;
    // Every currently rendered part belongs to this fixed-size panel subtree.
    // Keep the existing font/glyph geometry; move their common root by the
    // panel's authored anchor displacement in the current canvas.
    let translation = ((panel.center() - PANEL_CENTER) * scale).extend(0.0);
    Some(Transform::from_translation(translation).with_scale(Vec3::splat(scale)))
}

/// 铺窗体树：根 → 面板件 + 尾标 + 两栏字形（全部 canvas 单位、屏幕中心
/// 系，根上乘 canvas 缩放）。
fn spawn_tree(
    commands: &mut Commands,
    art: &BalloonArt,
    window_art: &WindowArt,
    state: &TalkWindowState,
    placement: Transform,
) {
    let root = commands
        .spawn((
            TalkWindowRoot,
            placement,
            Visibility::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    // 面板：退化九宫（四角原生 + 上下横带 1px 采样列拉伸）。
    for (rect, size, pos) in panel_pieces() {
        let piece = commands
            .spawn((
                Sprite {
                    image: window_art.page.clone(),
                    color: Color::WHITE,
                    rect: Some(rect),
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, -0.1),
                TalkWindowPart { base: Color::WHITE },
                RenderLayers::layer(BALLOON_LAYER),
            ))
            .id();
        commands.entity(root).add_child(piece);
    }
    // 尾标（替身三角；激活态随打字机，先隐）。
    let end = commands
        .spawn((
            Sprite {
                image: window_art.end_mark.clone(),
                color: Color::WHITE,
                rect: Some(Rect {
                    min: Vec2::ZERO,
                    max: Vec2::new(END_ICON_W, END_ICON_H),
                }),
                custom_size: Some(Vec2::new(END_ICON_W, END_ICON_H)),
                ..default()
            },
            Transform::from_xyz(END_ICON_CENTER.x, END_ICON_CENTER.y, 0.0),
            TalkWindowPart { base: Color::WHITE },
            TalkEndMark,
            Visibility::Hidden,
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands.entity(root).add_child(end);
    spawn_label_glyphs(commands, root, art, state);
    spawn_content_glyphs(commands, root, art, state);
}

/// 面板件清单：(源矩形（页图像坐标，y 从顶量）, 显示尺寸, 中心位置)。
/// border 156 = 源图 312 的半边 ⇒ 中段宽 0：横带取 border 线上的 1px
/// 采样列，左右边带与中心高 0 无件。
fn panel_pieces() -> Vec<(Rect, Vec2, Vec2)> {
    let b = PANEL_BORDER;
    let half_w = PANEL_W / 2.0;
    let half_h = PANEL_H / 2.0;
    let (cx, cy) = (PANEL_CENTER.x, PANEL_CENTER.y);
    let s = PANEL_SRC;
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect {
        min: Vec2::new(x0, y0),
        max: Vec2::new(x1, y1),
    };
    let band_w = PANEL_W - 2.0 * b;
    let sample_x0 = s.min.x + b;
    vec![
        // 四角（原生 156×156）。
        (
            rect(s.min.x, s.min.y, s.min.x + b, s.min.y + b),
            Vec2::splat(b),
            Vec2::new(cx - half_w + b / 2.0, cy + half_h - b / 2.0),
        ),
        (
            rect(s.max.x - b, s.min.y, s.max.x, s.min.y + b),
            Vec2::splat(b),
            Vec2::new(cx + half_w - b / 2.0, cy + half_h - b / 2.0),
        ),
        (
            rect(s.min.x, s.max.y - b, s.min.x + b, s.max.y),
            Vec2::splat(b),
            Vec2::new(cx - half_w + b / 2.0, cy - half_h + b / 2.0),
        ),
        (
            rect(s.max.x - b, s.max.y - b, s.max.x, s.max.y),
            Vec2::splat(b),
            Vec2::new(cx + half_w - b / 2.0, cy - half_h + b / 2.0),
        ),
        // 上下横带：1px 采样列拉到带宽（源中段宽 0）。
        (
            rect(sample_x0, s.min.y, sample_x0 + 1.0, s.min.y + b),
            Vec2::new(band_w, b),
            Vec2::new(cx, cy + half_h - b / 2.0),
        ),
        (
            rect(sample_x0, s.max.y - b, sample_x0 + 1.0, s.max.y),
            Vec2::new(band_w, b),
            Vec2::new(cx, cy - half_h + b / 2.0),
        ),
    ]
}

/// 铺名字栏字形（居左居顶、字距 0）。行结构对账同气泡（摆位 vs 行量律）。
fn spawn_label_glyphs(
    commands: &mut Commands,
    root: Entity,
    art: &BalloonArt,
    state: &TalkWindowState,
) {
    let text = state.label.as_str();
    let metrics = layout_metrics(
        text,
        LABEL_FONT,
        FONT_FAMILY,
        WINDOW_LINE_SPACING,
        BAKE_PPEM,
        &|ch| art.glyph_cell(ch).map(|(_, advance)| advance),
    );
    let (lines, missing) = walk_glyphs(text, art, LABEL_FONT, 0.0, 0.0);
    if missing > 0 {
        warn!("[talkwin] 名字栏缺字形 {missing} 个（label 名字的字符集已并进图集——缺即数据断点）");
    }
    if lines.len() != metrics.line_widths.len() {
        warn!(
            "[talkwin] 名字栏摆位行数 {} 与律行数 {} 不一致：摆位实现漂移",
            lines.len(),
            metrics.line_widths.len()
        );
    }
    spawn_text_glyphs(
        commands,
        root,
        art,
        state,
        &lines,
        &metrics,
        LABEL_RECT,
        LABEL_FONT,
        LABEL_COLOR,
        GlyphSlot::Label,
    );
}

/// 铺正文字形（居左居顶 + 字距/词距增量）。返回 (字形数, 行数, 最宽行
/// 估算 px)——最宽行是超盒告警的量法（每字按其字号量宽，全角即精确）。
fn spawn_content_glyphs(
    commands: &mut Commands,
    root: Entity,
    art: &BalloonArt,
    state: &TalkWindowState,
) -> (usize, usize, f32) {
    let text = state.text.as_str();
    let metrics = layout_metrics(
        text,
        CONTENT_FONT,
        FONT_FAMILY,
        WINDOW_LINE_SPACING,
        BAKE_PPEM,
        &|ch| art.glyph_cell(ch).map(|(_, advance)| advance),
    );
    let (lines, missing) = walk_glyphs(text, art, CONTENT_FONT, CONTENT_CHAR_EXTRA, CONTENT_WORD_EXTRA);
    if lines.len() != metrics.line_widths.len() {
        warn!(
            "[talkwin] 正文摆位行数 {} 与律行数 {} 不一致：摆位实现漂移",
            lines.len(),
            metrics.line_widths.len()
        );
    }
    if missing > 0 {
        warn!(
            "[talkwin] 正文缺字形 {missing} 个（候选预筛已把语料字符集并进图集——缺即数据断点）"
        );
    }
    let mut widest = 0.0f32;
    for line in &lines {
        let line_w = line
            .iter()
            .map(|(spot, pen, _)| pen + spot.size * spot.visual)
            .fold(0.0f32, f32::max);
        widest = widest.max(line_w);
    }
    let count = spawn_text_glyphs(
        commands,
        root,
        art,
        state,
        &lines,
        &metrics,
        CONTENT_RECT,
        CONTENT_FONT,
        CONTENT_COLOR,
        GlyphSlot::Content,
    );
    (count, lines.len(), widest)
}

/// 栏位标记：正文字形带打字机序，名字栏字形常显。
enum GlyphSlot {
    Label,
    Content,
}

/// 两栏共用的字形摆位：垂直居顶（首行基线 = 盒顶 − 首行字号×上行线比）、
/// 水平居左（起笔 = 盒左沿）；行推进按行量律的行偏移（2x 单位折半）。
/// 格内笔点换算与气泡摆位同式。返回字形数。
fn spawn_text_glyphs(
    commands: &mut Commands,
    root: Entity,
    art: &BalloonArt,
    state: &TalkWindowState,
    lines: &[Vec<(GlyphSpot, f32, usize)>],
    metrics: &LayoutMetrics,
    rect: Rect,
    fallback_font: f32,
    color: [f32; 3],
    slot: GlyphSlot,
) -> usize {
    let size_of_line = |i: usize| -> f32 {
        lines
            .get(i)
            .map(|line| {
                line.iter()
                    .map(|(spot, _, _)| spot.size)
                    .fold(0.0f32, f32::max)
            })
            .filter(|size| *size > 0.0)
            .unwrap_or(fallback_font)
    };
    let baseline_0 = rect.max.y - size_of_line(0) * ASCENT_RATIO;
    let (cell, pen_x, baseline_from_top) = art.cell_geometry();
    let mut count = 0usize;
    for (li, line) in lines.iter().enumerate() {
        let baseline = baseline_0 - metrics.line_offsets.get(li).copied().unwrap_or(0.0) / TEXT_SCALE;
        for (spot, pen, raw) in line {
            let Some((cell_rect, _)) = art.glyph_cell(spot.ch) else {
                continue;
            };
            let scale = spot.size * spot.visual / BAKE_PPEM;
            // 格内笔点在 (pen_x, baseline_from_top)；格中心到笔点的偏移按
            // 显示缩放折算，atlas 的 y 向下、canvas 坐标 y 向上取负。
            let dx = (cell / 2.0 - pen_x) * scale;
            let dy = (cell / 2.0 - baseline_from_top) * scale;
            let [r, g, b] = spot.color.unwrap_or(color);
            let a = spot.alpha.unwrap_or(1.0);
            let glyph_color = Color::srgba(r, g, b, a);
            let x = rect.min.x + pen + dx;
            let y = baseline - dy;
            let revealed = match slot {
                GlyphSlot::Content => *raw < state.shown,
                GlyphSlot::Label => true,
            };
            let mut entity = commands.spawn((
                Sprite {
                    image: art.glyph_image_for(spot.ch).clone(),
                    color: glyph_color,
                    rect: Some(cell_rect),
                    custom_size: Some(Vec2::splat(cell * scale)),
                    ..default()
                },
                Transform::from_xyz(x, y, 0.0),
                TalkWindowPart {
                    base: glyph_color,
                },
                Visibility::Hidden,
                RenderLayers::layer(BALLOON_LAYER),
            ));
            match slot {
                GlyphSlot::Content => {
                    entity.insert(TalkGlyph { order: *raw });
                }
                GlyphSlot::Label => {
                    entity.insert(TalkLabelGlyph);
                }
            }
            if revealed {
                entity.insert(Visibility::Visible);
            }
            let id = entity.id();
            commands.entity(root).add_child(id);
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// 程序化替身
// ---------------------------------------------------------------------------

/// 尾标替身：50×44 白色下指三角（顶边 y=8 起全宽，收到底心尖）。真件
/// `icon_pageForward_gn`（ScenarioAtlas，提取缺口）——换图即真。
fn end_mark_image(images: &mut Assets<Image>) -> Handle<Image> {
    let (w, h) = (END_ICON_W as usize, END_ICON_H as usize);
    let mut data = vec![0u8; w * h * 4];
    // 三顶点 A(0,8) B(50,8) C(25,44)；三条边的内侧重距取最小 ⇒ SDF。
    let norm = (36.0f32 * 36.0 + 25.0 * 25.0).sqrt();
    for py in 0..h {
        for px in 0..w {
            let x = px as f32 + 0.5;
            let y = py as f32 + 0.5;
            let d_top = y - 8.0;
            let d_left = (36.0 * x - 25.0 * (y - 8.0)) / norm;
            let d_right = (36.0 * (END_ICON_W - x) - 25.0 * (y - 8.0)) / norm;
            let inside = d_top.min(d_left).min(d_right);
            let sdf = -inside; // 负在内侧
            let alpha = (0.5 - sdf).clamp(0.0, 1.0);
            let at = (py * w + px) * 4;
            data[at] = 255;
            data[at + 1] = 255;
            data[at + 2] = 255;
            data[at + 3] = (alpha * 255.0).round() as u8;
        }
    }
    images.add(Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    ))
}
