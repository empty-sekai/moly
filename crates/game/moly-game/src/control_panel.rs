//! The control panel: a drawer over the game for the control surface. Every
//! control in it sends a JSON-RPC request through the same interface the
//! external clients use (see `crate::control`) and shows the answer, so the
//! panel can do what a client can, and nothing a client cannot.
//!
//! F9 opens and closes it, Esc closes it, and while the game menu is open a
//! control panel button under it opens it where there is no keyboard. On a wide window the drawer
//! takes the taps over it and leaves the rest of the field to the game; on a
//! narrow one it covers the field and takes all of its input.

mod director;
mod methods;
mod server;
mod state;
mod widgets;

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::ui::{FocusPolicy, RelativeCursorPosition, UiTargetCamera};
use bevy::window::PrimaryWindow;
use bevy_remote::BrpResult;
use serde_json::Value;

use widgets::{button, column, row, text, Disabled, Kind, Selected, ACCENT, BAD, DRAWER, EDGE, GOOD, INK, MUTED};

/// Widths below this get the whole window.
const NARROW: f32 = 640.;
const DRAWER_WIDTH: f32 = 460.;
/// Answers older than this are dropped as lost.
const ANSWER_TIMEOUT: f64 = 10.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Director,
    Server,
    State,
    Methods,
    More,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Director, Tab::Server, Tab::State, Tab::Methods, Tab::More];

    fn title(self) -> &'static str {
        match self {
            Tab::Director => "导演",
            Tab::Server => "服务器",
            Tab::State => "状态",
            Tab::Methods => "方法",
            Tab::More => "更多",
        }
    }

    /// The tab's name in `moly.panel`.
    fn id(self) -> &'static str {
        match self {
            Tab::Director => "director",
            Tab::Server => "server",
            Tab::State => "state",
            Tab::Methods => "methods",
            Tab::More => "more",
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Open,
    Close,
    Tab(Tab),
    Director(director::Action),
    Server(server::Action),
    Methods(methods::Action),
    OpenLibrary,
}

/// What an answer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Director(director::Reply),
    Server(server::Reply),
    State,
    Methods(methods::Reply),
    /// An answer only the footer reports.
    Plain,
}

impl Reply {
    /// A repeated read: the footer reports it only when it fails.
    fn quiet(self) -> bool {
        matches!(self, Reply::State | Reply::Director(director::Reply::Snapshot))
    }
}

struct Call {
    reply: Reply,
    method: String,
    sent: f64,
    replies: async_channel::Receiver<BrpResult>,
}

#[derive(Resource, Default)]
pub(crate) struct ControlPanel {
    pub(crate) open: bool,
    tab: Tab,
    /// Frames during which the release of the tap that closed the panel
    /// still belongs to it.
    release_guard: u8,
    narrow: bool,
    /// The drawer's window rect in logical pixels, as last laid out.
    drawer: Option<Rect>,
    /// The cursor or a touch is over the open drawer this frame.
    pointer_over: bool,
    calls: Vec<Call>,
    /// The last answer: the method and how it went.
    footer: Option<(String, bool)>,
    /// Bumped whenever the shown body changes.
    revision: u64,
    director: director::View,
    server: server::View,
    state: state::View,
    methods: methods::View,
}

impl ControlPanel {
    /// Whether the field's input is the panel's: the narrow drawer covers
    /// the field, and the tap that closed the panel ends on it.
    pub(crate) fn blocks_world_input(&self) -> bool {
        (self.open && self.narrow) || self.release_guard != 0
    }

    /// Whether the pointer is the panel's this frame: over the open drawer.
    pub(crate) fn holds_pointer(&self) -> bool {
        self.pointer_over
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Sends one request; its answer comes to [`Self::answer`] in a later frame.
    fn send(&mut self, reply: Reply, method: &str, params: Option<Value>, now: f64) {
        match crate::control::call(method, params) {
            Ok(replies) => self.calls.push(Call { reply, method: method.to_owned(), sent: now, replies }),
            Err(error) => {
                self.footer = Some((format!("{method}：{error}"), false));
                self.changed();
            }
        }
    }

    /// Whether a request for `reply` is in flight.
    fn waiting(&self, reply: Reply) -> bool {
        self.calls.iter().any(|call| call.reply == reply)
    }

    fn answer(&mut self, reply: Reply, method: &str, result: BrpResult) {
        match &result {
            Err(error) => self.footer = Some((format!("{method}：{}", error.message), false)),
            Ok(_) if !reply.quiet() => self.footer = Some((format!("{method} ✓"), true)),
            Ok(_) => {}
        }
        // Each tab answers whether what it shows changed.
        let shown = match reply {
            Reply::Director(reply) => director::answer(self, reply, result),
            Reply::Server(reply) => server::answer(self, reply, result),
            Reply::State => state::answer(self, result),
            Reply::Methods(reply) => methods::answer(self, reply, result),
            Reply::Plain => false,
        };
        if shown {
            self.changed();
        }
    }

    /// Whether a window position (logical pixels) is over the open drawer.
    pub(crate) fn captures(&self, position: Vec2) -> bool {
        self.open && self.drawer.is_some_and(|drawer| drawer.contains(position))
    }

    fn close(&mut self) {
        self.open = false;
        self.release_guard = 2;
        self.changed();
    }
}

/// The parts of the drawer the systems look up.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Root,
    Drawer,
    Scroll,
    Body,
    Footer,
    /// The button shown under the open game menu.
    MenuEntry,
}

#[derive(Resource)]
struct PanelFont(Handle<Font>);

pub(crate) fn install(app: &mut App) {
    app.init_resource::<ControlPanel>()
        .add_systems(
            Startup,
            setup
                .after(crate::balloon::overlay_camera),
        )
        .add_systems(PreUpdate, (release_guard, input).chain().after(bevy::ui::UiSystems::Focus))
        .add_systems(Update, (poll, refresh, widgets::style).chain());
}

fn setup(mut c: Commands, ui_font: Res<crate::ui_font::UiFontAsset>, hud: Query<Entity, With<crate::balloon::HudCamera>>) {
    let Ok(camera) = hud.single() else {
        return;
    };
    let font = ui_font.0.clone();
    c.insert_resource(PanelFont(font.clone()));
    let root = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                display: Display::None,
                justify_content: JustifyContent::FlexEnd,
                padding: UiRect::all(px(12)),
                ..default()
            },
            // Only the drawer takes pointer focus; the field shows beside it.
            FocusPolicy::Pass,
            Part::Root,
            UiTargetCamera(camera),
            GlobalZIndex(1300),
        ))
        .id();
    let drawer = widgets::attach(
        &mut c,
        root,
        Node {
            width: px(DRAWER_WIDTH),
            max_width: percent(100),
            height: percent(100),
            min_height: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            padding: UiRect::all(px(14)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
    );
    c.entity(drawer).insert((
        Part::Drawer,
        BackgroundColor(DRAWER),
        BorderColor::all(EDGE),
        FocusPolicy::Block,
        RelativeCursorPosition::default(),
    ));
    let header = row(&mut c, drawer, 8.);
    c.entity(header).insert(Node {
        width: percent(100),
        align_items: AlignItems::Center,
        column_gap: px(8),
        flex_shrink: 0.,
        ..default()
    });
    let heading = column(&mut c, header, 2.);
    c.entity(heading).insert(Node {
        flex_grow: 1.,
        min_width: px(0),
        flex_direction: FlexDirection::Column,
        row_gap: px(2),
        ..default()
    });
    text(&mut c, heading, &font, "控制面板", 20., INK);
    text(&mut c, heading, &font, "与 agent 共用同一套控制接口 · F9", 11., MUTED);
    button(&mut c, header, &font, "关闭", PanelAction::Close, Kind::Plain);
    let tabs = row(&mut c, drawer, 6.);
    for tab in Tab::ALL {
        button(&mut c, tabs, &font, tab.title(), PanelAction::Tab(tab), Kind::Chip);
    }
    let scroll = widgets::attach(
        &mut c,
        drawer,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            ..default()
        },
    );
    c.entity(scroll).insert((Part::Scroll, RelativeCursorPosition::default(), ScrollPosition::default()));
    let body = column(&mut c, scroll, 10.);
    c.entity(body).insert(Part::Body);
    let footer = text(&mut c, drawer, &font, "", 11., MUTED);
    c.entity(footer).insert(Part::Footer);
    // The touch entry: a button under the game menu while the menu is open.
    let entry = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                bottom: px(18),
                display: Display::None,
                justify_content: JustifyContent::Center,
                ..default()
            },
            FocusPolicy::Pass,
            Part::MenuEntry,
            UiTargetCamera(camera),
            GlobalZIndex(1300),
        ))
        .id();
    button(&mut c, entry, &font, "控制面板", PanelAction::Open, Kind::Primary);
}

fn release_guard(buttons: Res<ButtonInput<MouseButton>>, touches: Res<Touches>, mut panel: ResMut<ControlPanel>) {
    if panel.release_guard > 0 && !buttons.any_pressed([MouseButton::Left, MouseButton::Right]) && touches.iter().next().is_none() {
        panel.release_guard -= 1;
    }
}

/// Where a drag over the scroll area started, and its scroll then.
#[derive(Default)]
struct Drag {
    from: Option<(f32, f32)>,
}

#[allow(clippy::too_many_arguments)]
fn input(
    mut panel: ResMut<ControlPanel>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut wheels: MessageReader<MouseWheel>,
    actions: Query<(&Interaction, &PanelAction, Has<Disabled>), Changed<Interaction>>,
    mut scroll: Query<(&Part, &RelativeCursorPosition, &ComputedNode, &mut ScrollPosition)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    time: Res<Time<Real>>,
    mut drag: Local<Drag>,
) {
    let now = time.elapsed_secs_f64();
    for event in keyboard.read() {
        if event.state != ButtonState::Pressed || event.repeat {
            continue;
        }
        match event.key_code {
            KeyCode::F9 if panel.open => panel.close(),
            KeyCode::F9 => open(&mut panel, now),
            KeyCode::Escape if panel.open => {
                panel.close();
                // The press is the panel's: the game's back key reads it
                // later this frame.
                keys.clear_just_pressed(KeyCode::Escape);
            }
            _ => {}
        }
    }
    let pressed: Vec<PanelAction> = actions
        .iter()
        .filter(|(interaction, _, disabled)| **interaction == Interaction::Pressed && !disabled)
        .map(|(_, action, _)| *action)
        .collect();
    if !panel.open && pressed.contains(&PanelAction::Open) {
        open(&mut panel, now);
        return;
    }
    let wheels: Vec<MouseWheel> = wheels.read().cloned().collect();
    let pointer = windows.single().ok().and_then(Window::cursor_position);
    let touch = touches.iter().next().map(|touch| touch.position());
    let over = touches.iter().any(|touch| panel.captures(touch.position())) || pointer.is_some_and(|at| panel.captures(at));
    if panel.pointer_over != over {
        panel.pointer_over = over;
    }
    if !panel.open {
        drag.from = None;
        return;
    }
    for action in pressed {
        apply(&mut panel, action, now);
    }
    // Wheel, mouse drag and touch drag scroll the body.
    let held = buttons.pressed(MouseButton::Left) || touch.is_some();
    for (part, cursor, computed, mut position) in &mut scroll {
        if *part != Part::Scroll {
            continue;
        }
        let maximum = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor()).max(0.);
        if cursor.cursor_over() {
            for wheel in &wheels {
                let delta = -wheel.y * if wheel.unit == MouseScrollUnit::Line { 36. } else { 1. };
                position.y = (position.y + delta).clamp(0., maximum);
            }
        }
        let at = touch.or(pointer).map(|at| at.y);
        match (held, at, drag.from) {
            (true, Some(y), None) if cursor.cursor_over() => drag.from = Some((y, position.y)),
            (true, Some(y), Some((start, scrolled))) => position.y = (scrolled + start - y).clamp(0., maximum),
            (false, _, _) => drag.from = None,
            _ => {}
        }
    }
}

fn open(panel: &mut ControlPanel, now: f64) {
    panel.open = true;
    panel.changed();
    match panel.tab {
        Tab::Director => director::enter(panel, now),
        Tab::Server => server::enter(panel, now),
        Tab::State => state::enter(panel, now),
        Tab::Methods => methods::enter(panel, now),
        Tab::More => {}
    }
}

/// `moly.panel`: opens or closes the panel and picks its tab, then answers
/// what it shows.
pub(crate) fn command(panel: &mut ControlPanel, params: &serde_json::Map<String, Value>, now: f64) -> Result<Value, String> {
    let tab = match params.get("tab") {
        None => None,
        Some(Value::String(id)) => Some(
            Tab::ALL
                .into_iter()
                .find(|tab| tab.id() == id.as_str())
                .ok_or_else(|| format!("moly.panel: no tab {id:?}"))?,
        ),
        Some(_) => return Err("moly.panel: tab must be a tab name".into()),
    };
    let wanted = match params.get("open") {
        None => None,
        Some(Value::Bool(wanted)) => Some(*wanted),
        Some(_) => return Err("moly.panel: open must be true or false".into()),
    };
    if let Some(tab) = tab {
        panel.tab = tab;
    }
    match wanted {
        Some(false) if panel.open => panel.close(),
        Some(true) if !panel.open => open(panel, now),
        // A new tab on the open panel is entered as a tap on it would be.
        _ if tab.is_some() && panel.open => open(panel, now),
        _ => {}
    }
    Ok(serde_json::json!({ "open": panel.open, "tab": panel.tab.id(), "narrow": panel.narrow }))
}

fn apply(panel: &mut ControlPanel, action: PanelAction, now: f64) {
    match action {
        PanelAction::Open => {}
        PanelAction::Close => panel.close(),
        PanelAction::Tab(tab) => {
            if panel.tab != tab {
                panel.tab = tab;
                open(panel, now);
            }
        }
        PanelAction::Director(action) => director::act(panel, action, now),
        PanelAction::Server(action) => server::act(panel, action, now),
        PanelAction::Methods(action) => methods::act(panel, action, now),
        PanelAction::OpenLibrary => {
            panel.send(
                Reply::Plain,
                "moly.library.command",
                Some(serde_json::json!({ "schemaVersion": 1, "type": "show" })),
                now,
            );
            panel.close();
        }
    }
}

/// Takes the answers that have arrived, and asks again for what the shown
/// tab keeps current.
fn poll(mut panel: ResMut<ControlPanel>, time: Res<Time<Real>>) {
    let now = time.elapsed_secs_f64();
    let mut arrived = Vec::new();
    panel.calls.retain(|call| match call.replies.try_recv() {
        Ok(result) => {
            arrived.push((call.reply, call.method.clone(), result));
            false
        }
        Err(_) => now - call.sent < ANSWER_TIMEOUT,
    });
    for (reply, method, result) in arrived {
        panel.answer(reply, &method, result);
    }
    if !panel.open {
        return;
    }
    match panel.tab {
        Tab::Director => director::tick(&mut panel, now),
        Tab::Server => server::tick(&mut panel, now),
        Tab::State => state::tick(&mut panel, now),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh(
    mut c: Commands,
    mut panel: ResMut<ControlPanel>,
    font: Option<Res<PanelFont>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    screens: Option<Res<crate::ui_layers::ScreenManager>>,
    mut parts: Query<(Entity, &Part, &mut Node, &ComputedNode, &UiGlobalTransform)>,
    mut footer: Query<(&Part, &mut Text, &mut TextColor)>,
    tabs: Query<(Entity, &PanelAction, Has<Selected>)>,
    mut shown: Local<Option<(Tab, u64)>>,
) {
    let Some(font) = font else {
        return;
    };
    let narrow = windows.single().is_ok_and(|window| window.width() < NARROW);
    if panel.narrow != narrow {
        panel.narrow = narrow;
    }
    let menu_open = screens.as_deref().is_some_and(|screens| {
        screens.dialogs().any(|(_, dialog, state)| {
            dialog == crate::ui_layers::DialogType::MysekaiMenuDialog
                && matches!(state, crate::ui_layers::DialogState::PlayOpenAnimation | crate::ui_layers::DialogState::Show)
        })
    });
    let mut body = None;
    for (entity, part, mut node, computed, transform) in &mut parts {
        match part {
            Part::MenuEntry => {
                set_if_neq!(node.display, if menu_open && !panel.open { Display::Flex } else { Display::None });
            }
            Part::Root => {
                set_if_neq!(node.display, if panel.open { Display::Flex } else { Display::None });
                set_if_neq!(node.padding, UiRect::all(px(if narrow { 0 } else { 12 })));
            }
            Part::Drawer => {
                set_if_neq!(node.width, if narrow { percent(100) } else { px(DRAWER_WIDTH) });
                set_if_neq!(node.border_radius, BorderRadius::all(px(if narrow { 0 } else { 16 })));
                let scale = computed.inverse_scale_factor();
                let drawer = (computed.size().x > 0.)
                    .then(|| Rect::from_center_size(transform.translation * scale, computed.size() * scale));
                if panel.drawer != drawer {
                    panel.drawer = drawer;
                }
            }
            Part::Body => body = Some(entity),
            _ => {}
        }
    }
    if !panel.open {
        return;
    }
    for (entity, action, selected) in &tabs {
        if let PanelAction::Tab(tab) = action {
            match (*tab == panel.tab, selected) {
                (true, false) => {
                    c.entity(entity).insert(Selected);
                }
                (false, true) => {
                    c.entity(entity).remove::<Selected>();
                }
                _ => {}
            }
        }
    }
    for (part, mut line, mut color) in &mut footer {
        if *part != Part::Footer {
            continue;
        }
        let (value, ok) = panel.footer.clone().unwrap_or_default();
        if **line != value {
            **line = value;
        }
        color.set_if_neq(TextColor(if panel.footer.is_none() { MUTED } else if ok { GOOD } else { BAD }));
    }
    let stamp = (panel.tab, panel.revision);
    if *shown == Some(stamp) {
        return;
    }
    *shown = Some(stamp);
    let Some(body) = body else {
        return;
    };
    c.entity(body).despawn_children();
    let font = &font.0;
    match panel.tab {
        Tab::Director => director::build(&mut c, body, font, &panel),
        Tab::Server => server::build(&mut c, body, font, &panel),
        Tab::State => state::build(&mut c, body, font, &panel),
        Tab::Methods => methods::build(&mut c, body, font, &panel),
        Tab::More => {
            let card = widgets::card(&mut c, body, font, Some("对话与互动"));
            text(
                &mut c,
                card,
                font,
                "浏览角色对话、家具互动与演出，并在当前场景或独立场景里重温。",
                12.,
                MUTED,
            );
            button(&mut c, card, font, "打开对话与互动", PanelAction::OpenLibrary, Kind::Primary);
            let card = widgets::card(&mut c, body, font, Some("外部客户端"));
            text(
                &mut c,
                card,
                font,
                "本面板的每个按钮都发送一条 JSON-RPC 请求。原生运行时设置 MOLY_CONTROL_PORT 即在 127.0.0.1 上开放同一套方法；浏览器页面经 control() 导出发送。",
                12.,
                MUTED,
            );
            text(&mut c, card, font, "方法清单见「方法」页。", 12., ACCENT);
        }
    }
}
