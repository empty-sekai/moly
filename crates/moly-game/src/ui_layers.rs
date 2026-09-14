//! 屏幕层栈与显示层槽位——本仓层序机制的唯一权威表。
//!
//! 真源的层序由两样东西给出（类与方法名直引反编译，逐体核过）：
//!
//! **一、显示层槽**——`Sekai.DisplayLayerType` 八值有序枚举（BG=0 ·
//! UI=1 · Header=2 · Scenario=3 · Dialog=4 · Loading=5 · ScreenEffect=6 ·
//! Overlay=7）。`ScreenManager` 持一张 `displayLayersCache` 表：每格一个
//! 层物体，层内次序由 `DisplayOrderInLayer`（BG=0 · UI=100 · Dialog=150 ·
//! Loading=200 · Overlay=300）加同层 `SetAsLastSibling` 决定。
//!
//! **二、屏幕层栈**——`ScreenManager.uiScreenStack`，元素是「回去的层」
//! 的三元组（层状态 · 视图栈对象 · 启动参数）。
//!
//! 两者职责不重叠：**对话框不在屏幕层栈里**（`DialogUtility` 一族直接
//! 建在 Dialog 槽上，靠 `DisableTapScreen` 阻断下层输入），
//! 屏幕层才进出栈。⇒ 本仓建**一套层槽表 + 一个屏幕层栈**，对话框占
//! Dialog 槽不入栈——不另建第二套层序机制。
//!
//! ## 层栈钩子顺序律（本模块的地基；承重句全列）
//!
//! **挂载链**（`AddScreenCore` → `OnScreenLayerBoot` 一族；一次压层把
//! 新层从 None 拉到 Playing）：
//! 存启动参数 →（实例已毁则重建；已激活则无操作返回）→ SetActive(true)
//! → 记视图栈对象 → SetAsLastSibling → `OnScreenLayerBoot`：状态=Booting
//! → **HideComponentRoot** →（有启动参数先 `ReceiveBootData`）→
//! **OnBoot** → 等装载完成 →（反向开场标志 · BGM · 背景 · 表头）→
//! `OnScreenLayerInitComponent`：状态=InitComponent → **ShowComponentRoot**
//! → **OnInitComponent** → 开场动画起（状态=Starting）→ **OnScreenStart**
//! （开场动画还在播时就发）→ 动画收尾 → **OnFinishStartAnimation**
//! （状态=Playing）→ `OnChangeUILayer` 观察者 → `InitComplete`。
//!
//! **可否决退出链**（`OnWillExit` 三态门，三个迁移共用）：换层前先问
//! 当前层 OnWillExit，回三态：None=让一帧再问 · OK=放行 · **Cancel=否决
//! 回滚**——弹层路把刚弹出的三元组压回栈、恢复待入层后中止。三个迁移
//! （压/弹/换）都在**自己**先问过 OnWillExit 之后才把旧层交给出场链；
//! 出场链本身不再问。
//!
//! **出场链**（`ExitScreen` → `OnScreenLayerExitStart` 一族）：状态=
//! Exiting → **OnExitStart** → 播出场动画 → 动画收尾 → SetActive(false)
//! → `OnScreenLayerExited`：状态=Exit → **OnExited**。
//!
//! **压层**（`PushUIScreenCore`）：同层重复压=丢弃 · 迁移中=丢弃 →
//! 当前层过 OnWillExit 门 → 三元组（当前层 · 视图栈对象 · 启动参数）→
//! 出场链退当前层 → **栈压入该三元组** → 挂载链挂新层 → 等到 Playing
//! → InitComplete。
//!
//! **弹层**（`PopUIScreenCore`）：**先弹栈再退旧层**。栈空且当前层非
//! 主场地屏 → 压主场地屏（Home）；栈空且当前就是主场地屏 → 无操作
//! 收场。非空：取出待显层 → 当前层过 OnWillExit 门（Cancel=回滚）→
//! 出场链退当前层 → **挂载链全量重挂待显层**（OnBoot 再来一遍、反向
//! 开场）→ 等到 Playing → InitComplete。
//!
//! **换层**（`ChangeUIScreenCore`）：同压层但不压栈（原地替换当前层）。
//!
//! **退场**（`ExitScene`）：对 screenMap 里全部层发 **OnExitScene**，清
//! 启动参数表。
//!
//! **单槽串行**：`ScreenManager.changeUILayerCoroutine` 只有一个槽，
//! 一次只跑一个迁移；迁移中的新请求被门丢弃。
//!
//! ## 本仓对应物与刻意的简化（改这里之前先读）
//!
//! - 钩子相对栈变更的**次序**逐句照律；律保留的是次序，不是帧节奏。
//! - 本仓层视图没有开场/出场动画 ⇒ 各等待点（装载完成/出场完成轮询）
//!   当帧通过，一次迁移在单帧内按律收尾。有动画的层接入后把等待点
//!   换成动画时长即可，次序表不变。
//! - `OnWillExit` 三态门保留：None 的「让一帧再问」在无可否决实现者时
//!   等价放行（问两次同一个实现，答案不会变）。Cancel 回滚支路照写，
//!   当前没有层会返回它。
//! - 视图栈对象/启动参数两格本仓无消费者（层视图还没有可保存的状态），
//!   栈元素只记层身份；补视图时扩栈元素，不动层序。
//! - 「站点地图」层有真视图（`crate::sitemap`）。它自己的 M 键与「点
//!   当前站收起」是直通写可见性的既有口；本模块把该槽的**视图可见性
//!   视为层状态的真值**，每帧对账：视图开了而栈没记 ⇒ 补压层梯；视图
//!   收了而栈还记着 ⇒ 补弹层梯。三条入口（按钮 · M 键 · 点当前站）
//!   走同一张梯。
//!
//! 具名挂账（本模块不实现，收工报里重列）：
//! - 除站点地图外的全部层视图（任务 · 库存 · 工坊 · 传送门 · 换装 ·
//!   邀请 · 访客 · 情报 · 变换 · 选曲 · 秘密商店）——层槽与梯在，视图
//!   不在：压层请求响亮记账后入栈，画面上没有那一层。
//! - `OnChangeUILayer` 观察者（本仓无订阅者，日志占位）。
//! - 退场（`ExitScene`）的发送方（场景退出域未建；命令口先立）。

use bevy::prelude::*;

use crate::sitemap::SitemapRoot;

// ---------------------------------------------------------------------------
// 显示层槽（真源 DisplayLayerType / DisplayOrderInLayer）
// ---------------------------------------------------------------------------

/// 显示层槽（真源 `Sekai.DisplayLayerType` 八值；值与次序都是真值）。
/// 变体全录是刻意的：这是真源层序机制的槽表，未构造的变体是还没有
/// 落位者的槽（Header/Scenario/Loading/ScreenEffect/Overlay 的视图域
/// 未建），删变体等于替真源裁表。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DisplayLayerType {
    LayerBg = 0,
    LayerUi = 1,
    LayerHeader = 2,
    LayerScenario = 3,
    LayerDialog = 4,
    LayerLoading = 5,
    LayerScreenEffect = 6,
    LayerOverlay = 7,
}

impl DisplayLayerType {
    pub(crate) fn label(self) -> &'static str {
        match self {
            DisplayLayerType::LayerBg => "BG",
            DisplayLayerType::LayerUi => "UI",
            DisplayLayerType::LayerHeader => "Header",
            DisplayLayerType::LayerScenario => "Scenario",
            DisplayLayerType::LayerDialog => "Dialog",
            DisplayLayerType::LayerLoading => "Loading",
            DisplayLayerType::LayerScreenEffect => "ScreenEffect",
            DisplayLayerType::LayerOverlay => "Overlay",
        }
    }

    /// 层内次序基准（真源 `DisplayOrderInLayer`）。那张表只有五个成员：
    /// BG=0 · UI=100 · Dialog=150 · Loading=200 · Overlay=300。Header ·
    /// Scenario · ScreenEffect 没有命名基准——返回 [`None`] 而不是拍一个
    /// 0：那张表没给它们排过序，本仓也不替它排。
    pub(crate) fn order_in_layer(self) -> Option<i32> {
        match self {
            DisplayLayerType::LayerBg => Some(0),
            DisplayLayerType::LayerUi => Some(100),
            DisplayLayerType::LayerDialog => Some(150),
            DisplayLayerType::LayerLoading => Some(200),
            DisplayLayerType::LayerOverlay => Some(300),
            DisplayLayerType::LayerHeader
            | DisplayLayerType::LayerScenario
            | DisplayLayerType::LayerScreenEffect => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 具名层槽
// ---------------------------------------------------------------------------

/// 具名层槽：动作按钮分派表会打开的那些屏幕层 + 栈底场地屏。
/// 括号里的数字是真源屏幕号（`MenuScreenType`，逐个对着压层调用核过）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LayerId {
    /// 栈底场地屏（真源 Home=2）。真源里主场地 · 自室 · 采集 · 投递是
    /// 四个场地屏；本仓站点域已把它们并成站点切换（`crate::site`），
    /// 层栈只留这一格——四层共用外壳的「共用」正是落在这一格上。
    HomeField,
    /// 小地图（641）。本仓唯一有真视图的层槽（`crate::sitemap`）。
    MysekaiSiteMap,
    /// Original furniture layout editor (605), owned by its draft reducer.
    MysekaiSiteEdit,
    /// 任务（621；外壳任务面板的压层目标）。
    MysekaiMission,
    /// 库存（607；宝箱按钮的压层目标）。
    MysekaiInventory,
    /// 工坊（615；工作台按钮的压层目标）。
    MysekaiCraft,
    /// 传送门编辑（628；门编辑按钮的压层目标，源带启动参数）。
    MysekaiGate,
    /// 换装（629）。
    MysekaiAvatarCostumeSetting,
    /// 传送门邀请（645，源带启动参数）。
    MysekaiGateInvitation,
    /// 访客一览（658）。
    MysekaiVisitTop,
    /// 家具情报（622，源带启动参数）。
    MysekaiInfo,
    /// 家具变换（627）。
    MysekaiConvert,
    /// 选曲（626，源带当前站点与家具的启动参数）。
    MysekaiBgmSelect,
    /// 秘密商店（625）。
    MysekaiSecretShop,
}

impl LayerId {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LayerId::HomeField => "场地屏",
            LayerId::MysekaiSiteMap => "站点地图",
            LayerId::MysekaiSiteEdit => "家具布局编辑",
            LayerId::MysekaiMission => "任务",
            LayerId::MysekaiInventory => "库存",
            LayerId::MysekaiCraft => "工坊",
            LayerId::MysekaiGate => "传送门编辑",
            LayerId::MysekaiAvatarCostumeSetting => "换装",
            LayerId::MysekaiGateInvitation => "传送门邀请",
            LayerId::MysekaiVisitTop => "访客一览",
            LayerId::MysekaiInfo => "家具情报",
            LayerId::MysekaiConvert => "家具变换",
            LayerId::MysekaiBgmSelect => "选曲",
            LayerId::MysekaiSecretShop => "秘密商店",
        }
    }

    /// 该层落在哪个显示层槽。屏幕层都是游戏 UI——落在 UI 槽（本仓槽位
    /// 表的指派；对话框走 Dialog 槽、全屏遮罩走 Overlay 槽，见
    /// [`crate::menu_shell`] 的对话框）。
    pub(crate) fn display_layer(self) -> DisplayLayerType {
        DisplayLayerType::LayerUi
    }

    /// 该层的视图是否已建。**唯一已建的是站点地图**；其余层槽压层请求
    /// 照走钩子梯、响亮记账「视图未建」。
    pub(crate) fn view_built(self) -> bool {
        matches!(self, LayerId::MysekaiSiteMap | LayerId::MysekaiInfo | LayerId::MysekaiSiteEdit)
    }
}

// ---------------------------------------------------------------------------
// 层状态机（真源 ScreenLayer.State 七值）
// ---------------------------------------------------------------------------

/// 层状态（真源 `ScreenLayer.State`：None=0 · Booting=1 · InitComponent=2 ·
/// Starting=3 · Playing=4 · Exiting=5 · Exit=6）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenLayerState {
    None = 0,
    Booting = 1,
    InitComponent = 2,
    Starting = 3,
    Playing = 4,
    Exiting = 5,
    Exit = 6,
}

impl ScreenLayerState {
    /// 状态名读口（迁移日志的消费者未建，口先立）。
    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            ScreenLayerState::None => "None",
            ScreenLayerState::Booting => "Booting",
            ScreenLayerState::InitComponent => "InitComponent",
            ScreenLayerState::Starting => "Starting",
            ScreenLayerState::Playing => "Playing",
            ScreenLayerState::Exiting => "Exiting",
            ScreenLayerState::Exit => "Exit",
        }
    }
}

/// OnWillExit 的三态回答（真源 `WillExitBehaviour`：None=0 · OK=1 ·
/// Cancel=2）。Cancel 的回滚支路在弹层梯里，见 [`UiLayerStack::pop`]；
/// Ok 由未来的层实现者构造（问门当前恒 None），三态全录。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WillExitBehaviour {
    None,
    Ok,
    Cancel,
}

// ---------------------------------------------------------------------------
// 层栈（真源 uiScreenStack + currentUI 的本仓对应物）
// ---------------------------------------------------------------------------

/// 屏幕层栈。`beneath` 是「回去的层」（真源栈元素只存三元组的层身份
/// 那一格）；`current` 是当前显示层（真源 currentUI）。
#[derive(Debug, Resource)]
pub(crate) struct UiLayerStack {
    beneath: Vec<LayerId>,
    current: LayerId,
    current_state: ScreenLayerState,
}

impl Default for UiLayerStack {
    fn default() -> Self {
        UiLayerStack {
            beneath: Vec::new(),
            current: LayerId::HomeField,
            current_state: ScreenLayerState::Playing,
        }
    }
}

impl UiLayerStack {
    /// 当前层读口（层视图/调试消费者未建，口先立）。
    #[allow(dead_code)]
    pub(crate) fn current(&self) -> LayerId {
        self.current
    }

    /// 当前层状态读口（消费者未建，口先立）。
    #[allow(dead_code)]
    pub(crate) fn current_state(&self) -> ScreenLayerState {
        self.current_state
    }

    /// 当前层是否场地屏（外壳跟着场地屏走：非场地屏当前层时外壳收起，
    /// 对应真源「出场链 SetActive(false) 把旧层的 UI 一并收起」）。
    pub(crate) fn on_field(&self) -> bool {
        self.current == LayerId::HomeField
    }

    /// OnWillExit 的问门。当前没有层实现它 ⇒ 恒 None（等价放行，见模块
    /// 头）。有层要否决时改这里成虚派发，三态语义已备好。
    fn will_exit_gate(&self, layer: LayerId) -> WillExitBehaviour {
        let behaviour = WillExitBehaviour::None;
        info!(
            "[ui_layers] OnWillExit → {layer:?}（{}）答 {behaviour:?}",
            layer.label()
        );
        behaviour
    }

    /// 挂载梯（AddScreenCore → OnScreenLayerBoot 一族）。钩子次序照律；
    /// 无动画 ⇒ 等待点当帧通过。
    fn mount_ladder(&mut self, layer: LayerId) {
        let slot = layer.display_layer();
        info!(
            "[ui_layers] 挂载梯起 {layer:?}（{}；显示层槽 {} 槽内基准 {:?}）",
            layer.label(),
            slot.label(),
            slot.order_in_layer()
        );
        // OnScreenLayerBoot：状态=Booting → HideComponentRoot → OnBoot。
        self.current_state = ScreenLayerState::Booting;
        info!("[ui_layers]   HideComponentRoot（装载期收起部件根）");
        info!(
            "[ui_layers]   OnBoot（{}；装载完成等待点无动画，当帧通过）",
            layer.label()
        );
        // OnScreenLayerInitComponent：ShowComponentRoot → OnInitComponent
        // → 开场动画起。
        self.current_state = ScreenLayerState::InitComponent;
        info!("[ui_layers]   ShowComponentRoot · OnInitComponent");
        self.current_state = ScreenLayerState::Starting;
        // OnScreenStart 在开场动画还在播时发；动画收尾后
        // OnFinishStartAnimation。
        info!("[ui_layers]   OnScreenStart（开场动画无，当帧收尾）");
        self.current_state = ScreenLayerState::Playing;
        info!("[ui_layers]   OnFinishStartAnimation（状态=Playing）");
        info!("[ui_layers]   OnChangeUILayer 观察者（本仓无订阅者）· InitComplete");
        if !layer.view_built() && layer != LayerId::HomeField {
            info!(
                "[ui_layers] 层槽 {}：视图未建——梯照走、层入栈，画面上没有这一层（具名挂账）",
                layer.label()
            );
        }
    }

    /// 出场梯（ExitScreen → OnScreenLayerExitStart 一族）。调用方已问过
    /// OnWillExit；这里不再问。
    fn exit_ladder(&mut self, layer: LayerId) {
        info!(
            "[ui_layers] 出场梯起 {layer:?}（{}）",
            layer.label()
        );
        self.current_state = ScreenLayerState::Exiting;
        info!("[ui_layers]   OnExitStart（出场动画无，当帧收尾）");
        // SetActive(false) → OnScreenLayerExited → OnExited。
        self.current_state = ScreenLayerState::Exit;
        info!("[ui_layers]   SetActive(false) · OnExited（状态=Exit）");
    }

    /// 压层梯（PushUIScreenCore）。
    fn push(&mut self, target: LayerId) {
        // 同层重复压=丢弃（真源记错误日志后丢弃）。
        if self.current == target {
            info!(
                "[ui_layers] 压层请求 {} 丢弃：已是当前层",
                target.label()
            );
            return;
        }
        // 当前层过 OnWillExit 门（Cancel=中止——压层路的回滚只撤待入层，
        // 栈还没动）。
        if self.will_exit_gate(self.current) == WillExitBehaviour::Cancel {
            info!("[ui_layers]   OnWillExit 答 Cancel：压层中止（栈未动）");
            return;
        }
        let old = self.current;
        // 三元组先攒好 → 出场链退当前层 → 栈压入 → 挂载链挂新层。
        self.exit_ladder(old);
        self.beneath.push(old);
        self.current = target;
        self.mount_ladder(target);
        info!(
            "[ui_layers] 压层完成：{} → {}（栈深 {}）",
            old.label(),
            target.label(),
            self.beneath.len()
        );
    }

    /// 弹层梯（PopUIScreenCore）：**先弹栈再退旧层**。
    fn pop(&mut self) {
        let Some(reveal) = self.beneath.pop() else {
            // 栈空：当前非场地屏 → 压场地屏（真源压 Home）；当前就是
            // 场地屏 → 无操作收场。
            if self.current != LayerId::HomeField {
                info!(
                    "[ui_layers] 弹层遇空栈且当前非场地屏 → 压场地屏（真源同形）"
                );
                self.push(LayerId::HomeField);
            } else {
                info!("[ui_layers] 弹层到栈底（场地屏）：无操作收场");
            }
            return;
        };
        // 当前层过 OnWillExit 门；Cancel=回滚——刚弹出的层压回栈、中止。
        if self.will_exit_gate(self.current) == WillExitBehaviour::Cancel {
            self.beneath.push(reveal);
            info!(
                "[ui_layers]   OnWillExit 答 Cancel：弹层回滚（{} 压回栈，当前层不动）",
                reveal.label()
            );
            return;
        }
        let old = self.current;
        self.exit_ladder(old);
        self.current = reveal;
        // 挂载链全量重挂待显层：OnBoot 再来一遍、反向开场（真源弹层路
        // 的 AddScreenCore 走 isForward=反向）。
        self.mount_ladder(reveal);
        info!(
            "[ui_layers] 弹层完成：{} → {}（重挂；栈深 {}）",
            old.label(),
            reveal.label(),
            self.beneath.len()
        );
    }

    /// 换层梯（ChangeUIScreenCore）：同压层但不压栈（原地替换）。
    fn change(&mut self, target: LayerId) {
        if self.current == target {
            info!(
                "[ui_layers] 换层请求 {} 丢弃：已是当前层",
                target.label()
            );
            return;
        }
        if self.will_exit_gate(self.current) == WillExitBehaviour::Cancel {
            info!("[ui_layers]   OnWillExit 答 Cancel：换层中止");
            return;
        }
        let old = self.current;
        self.exit_ladder(old);
        self.current = target;
        self.mount_ladder(target);
        info!(
            "[ui_layers] 换层完成：{} → {}（不压栈）",
            old.label(),
            target.label()
        );
    }

    /// 退场（ExitScene）：全部层发 OnExitScene，栈清回场地屏。
    fn exit_scene(&mut self) {
        info!(
            "[ui_layers] 退场：{} 与栈内 {} 层逐个 OnExitScene，栈清空",
            self.current.label(),
            self.beneath.len()
        );
        self.beneath.clear();
        self.exit_ladder(self.current);
        self.current = LayerId::HomeField;
        self.current_state = ScreenLayerState::None;
    }
}

// ---------------------------------------------------------------------------
// 命令与推进系统
// ---------------------------------------------------------------------------

/// 层栈命令（真源 PushUIScreen / BackUIScreen / ExitScene 的本仓口）。
/// ExitScene/Change 的发送方未建（场景退出域挂账；换层路本仓暂无走
/// 入口）——命令口先立全，发送方补齐即走同一条梯。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub(crate) enum LayerCommand {
    Push(LayerId),
    /// BackUIScreen。
    Pop,
    /// ExitScene（发送方未建——场景退出域挂账，命令口先立）。
    ExitScene,
    /// ChangeUIScreen（原地替换；本仓暂无发送方，口先立全）。
    Change(LayerId),
}

/// Update：层栈推进。三步——
/// 1. 站点地图槽**视图先行对账**（M 键/点当前站是直通写可见性的既有口，
///    栈跟随视图补梯）；
/// 2. 消费层命令（压/弹/换/退场，单帧内按律收尾）；
/// 3. 槽位视图落位（站点地图根可见性 = 该槽是否当前层）。
pub(crate) fn advance(
    mut commands: MessageReader<LayerCommand>,
    mut stack: ResMut<UiLayerStack>,
    mut sitemap_roots: Query<&mut Visibility, With<SitemapRoot>>,
    editor: Res<crate::fixture_edit::EditView>,
    mut editor_commands: MessageWriter<crate::fixture_edit::EditCommand>,
) {
    // The editor's draft owner is authoritative for entry/exit, including the
    // keyboard route. A Back request must first resolve its dirty-work dialog.
    let editor_mounted = stack.current == LayerId::MysekaiSiteEdit
        || stack.beneath.contains(&LayerId::MysekaiSiteEdit);
    if editor.active && !editor_mounted && stack.on_field() {
        stack.push(LayerId::MysekaiSiteEdit);
    } else if !editor.active && stack.current == LayerId::MysekaiSiteEdit {
        stack.pop();
    }
    // ---- 1. 视图先行对账（站点地图槽） ----
    let view_open = match sitemap_roots.single() {
        Ok(visible) => *visible != Visibility::Hidden,
        Err(_) => false,
    };
    let slot_open = stack.current == LayerId::MysekaiSiteMap;
    if view_open && !slot_open {
        info!("[ui_layers] 站点地图视图已开而栈未记 → 补压层梯（直通口对账）");
        stack.push(LayerId::MysekaiSiteMap);
    } else if !view_open && slot_open {
        info!("[ui_layers] 站点地图视图已收而栈还记着 → 补弹层梯（直通口对账）");
        stack.pop();
    }

    // ---- 2. 层命令 ----
    for command in commands.read() {
        match *command {
            LayerCommand::Push(layer) => stack.push(layer),
            LayerCommand::Pop if stack.current == LayerId::MysekaiSiteEdit && editor.active => {
                editor_commands.write(crate::fixture_edit::EditCommand::RequestExit);
            }
            LayerCommand::Pop => stack.pop(),
            LayerCommand::Change(layer) => stack.change(layer),
            LayerCommand::ExitScene => stack.exit_scene(),
        }
    }

    // ---- 3. 槽位视图落位 ----
    if let Ok(mut visible) = sitemap_roots.single_mut() {
        *visible = if stack.current == LayerId::MysekaiSiteMap {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
