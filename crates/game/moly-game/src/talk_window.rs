//! The talk window: the fixed window of `ScreenLayerMysekaiTalk` (screen 619),
//! drawn from the extracted prefab as one `UiPrefabView`.
//!
//! The source's window serves every dialogue the engine plays — the player's
//! (SD) talks and the paired fixture talks — through
//! `ShowTalkWindow`/`HideTalkWindow`/`ShowTextAsync`. World text such as tweets
//! does not go through it. Both chains keep their own stepping and selection
//! ([`crate::player_talk`], [`crate::talk`]); this module is the presentation
//! half and the window's own state machine (open/close/fade/typewriter/click
//! latch/auto/menu).
//!
//! **Ownership.** Every state transition takes a [`TalkSession`] (player or
//! pair) by `&mut`; the two chains are mutually exclusive at their entrances,
//! so a same-frame double ownership is a defect and panics.
//!
//! **Layout.** The prefab document `Talk` is drawn by [`UiPrefabView`]:
//! geometry, images, the name plate and the body text come from the document;
//! this module only writes the body text (by revealed characters), the name,
//! the window alpha and node visibility. `moly.ui.views` lists the view and
//! `moly.ui.tap` can press its controls (the click area, the menu button, the
//! menu's close/auto/skip buttons).
//!
//! **Timing.** `ShowTextAsync`: character 0 at t=0, character k at 50k ms,
//! DONE one tick after the last character; an empty text is DONE at t=0. A tap
//! while typing reveals the whole text and clears the click latch (skip and
//! release are two clicks). Fade: the window's CanvasGroup starts at alpha 1,
//! so the opening show is instantaneous; hide fades to 0 over 0.5 s and a
//! later show fades in over 0.3 s (OutQuad). Closing is immediate.
//!
//! **Auto.** The prefab's Auto button toggles the source's `AutoMode`: while
//! it is on and the text is done, the release latch is set every 2.0 s
//! (`_nextTalkIntervalTime`), which the chains read as a click.
//!
//! **Named gaps.** Auto mode does not wait for a playing voice
//! (`_isPlayingVoice` is not tracked by the window), so its 2 s interval can
//! run under a voice line. The body's TMP auto-size search is not run (the
//! document's start size is used). The end sign is shown/hidden at the
//! typewriter's end without the source's `PageForward` bounce (no scale
//! override in the view). The menu's open/close animation is a visibility
//! switch. The all-skip button is tappable but its
//! `ScenarioAllSkipConfirmDialog` (176) and the talk's `SkipEvent` are not
//! built. The hide-UI and screenshot buttons are hidden (their behaviours are
//! not built). The body uses the shared open-font atlas, not the source's two
//! TMP font assets.

use bevy::camera::visibility::RenderLayers;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiRect;
use serde_json::Value;

use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layout::{UiLayouts, UiPrefabView};

mod stage_layout;
pub(crate) use stage_layout::ResponsiveDialogueMetrics;

// ---------------------------------------------------------------------------
// Session ownership
// ---------------------------------------------------------------------------

/// The chain the window belongs to: the player talk and the paired talk share
/// the window and differ in which chain steps it. Every transition takes one
/// by `&mut`, so the borrow proves single ownership.
pub(crate) enum TalkSession<'a> {
    Player(&'a mut crate::player_talk::PlayerTalkSession),
    Pair(&'a mut crate::talk::ActiveTalk),
}

impl TalkSession<'_> {
    /// Which chain and talk this is (for logs).
    pub(crate) fn chain_word(&self) -> String {
        match self {
            TalkSession::Player(session) => format!("player chain talk {}", session.talk_id()),
            TalkSession::Pair(talk) => format!("pair chain talk {}", talk.talk_id()),
        }
    }
}

// ---------------------------------------------------------------------------
// The prefab's nodes (path suffixes, unique in the document)
// ---------------------------------------------------------------------------

/// The talk layer's layout document.
const LAYOUT: &str = "Talk";
/// The window group (the layer's `_talkWindowCanvasGroup`): the fade writes its
/// alpha.
const WINDOW_NODE: &str = "ComponentRoot/TalkWindow";
/// The body panel (the window's `talkBG`): the geometry the QA layout metrics
/// report.
const PANEL_NODE: &str = "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG";
/// The name plate (the layer's `_labelText`).
const LABEL_NODE: &str = "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText";
/// The body (the layer's `_contentText`).
const CONTENT_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/ContentText";
/// The end sign group (the layer's `_endIconObj`).
const END_SIGN_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/EndSign";
/// The auto-mode marker (the layer's `_autoSignal`).
const AUTO_SIGNAL_NODE: &str = "ComponentRoot/TalkWindow/Window/AutoSignal";
/// The full-screen click area (the layer's `_clickAreaInOutGame`).
const CLICK_AREA_NODE: &str = "ComponentRoot/TalkWindow/ClickArea";
/// The scenario menu and its parts.
const MENU_PANEL: &str = "ComponentRoot/RightTop/ScenarioMenu/Base";
const MENU_CLOSE: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/UIPartsCloseButton";
const MENU_AUTO: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/Auto";
const MENU_SKIP: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/Skip";
const MENU_BUTTON: &str = "ComponentRoot/RightTop/MenuButtons/UIPartsMenuButton";
const HIDE_UI_BUTTON: &str = "ComponentRoot/RightTop/MenuButtons/HideUIButton";
const SCREENSHOT_BUTTON: &str = "ComponentRoot/RightTop/MenuButtons/ScreenshotButton";

/// `_showingTextIntervalTimeMs` = 50 ms per character (the layer's ctor).
const TYPE_INTERVAL: f32 = 0.05;
/// `_showTalkWindowFadeTime` (the layer's ctor).
const SHOW_FADE_SECONDS: f32 = 0.3;
/// `_hideTalkWindowFadeTime` (the layer's ctor).
const HIDE_FADE_SECONDS: f32 = 0.5;
/// `_nextTalkIntervalTime` = 2.0 s (the layer's ctor, `+0x128` low float).
const AUTO_INTERVAL_SECONDS: f32 = 2.0;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The window's mutable half: the source layer's fields (latch, typewriter,
/// alpha, auto, menu) as the chains and the view read them. Persistent: the
/// layer object outlives a talk and show/hide only change visibility.
#[derive(Resource)]
pub(crate) struct TalkWindowState {
    /// The window is present (opened at the talk's start, closed at its end).
    present: bool,
    /// CanvasGroup alpha.
    alpha: f32,
    /// A fade in flight: (from, target, seconds, elapsed).
    fade: Option<(f32, f32, f32, f32)>,
    /// The name plate text.
    label: String,
    /// The body text (the whole line; the typewriter moves `shown`).
    text: String,
    /// Revealed characters.
    shown: usize,
    /// Typewriter clock (seconds).
    typing_clock: f32,
    /// `_isPlayingTextAnimation`.
    playing: bool,
    /// `_isClicked` (the manual click latch).
    clicked: bool,
    /// `_endIconObj`'s active state.
    end_icon: bool,
    /// `playerData.AutoMode` and its marker.
    auto: bool,
    /// The auto interval accumulator (`_nextTalkInterval`).
    auto_wait: f32,
    /// The auto release for this frame (the source's `IsWaitClick` auto arm).
    auto_release: bool,
    /// The scenario menu is open.
    menu_open: bool,
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
            end_icon: false,
            auto: false,
            auto_wait: 0.0,
            auto_release: false,
            menu_open: false,
        }
    }
}

impl TalkWindowState {
    /// Read-only visible transcript for accessibility/acceptance tools.
    pub(crate) fn transcript(&self) -> (&str, &str, bool, bool) {
        let displaying = self.present
            && self
                .fade
                .as_ref()
                .map_or(self.alpha > 0.01, |(_, target, _, _)| *target > 0.01);
        (&self.label, &self.text, displaying, self.playing)
    }

    fn approximately_one(value: f32) -> bool {
        (value - 1.0).abs() <= 1e-6
    }

    /// The talk starts: the window is present (the serialized alpha is 1, so
    /// the first show is instantaneous) and both texts are cleared.
    pub(crate) fn open(&mut self, _owner: TalkSession<'_>) {
        self.reset_for_admission(true);
    }

    /// A fixture pre-action owns its cast but has not entered the talk body
    /// yet: clear the previous prompt without showing an empty panel.
    pub(crate) fn prepare(&mut self, _owner: TalkSession<'_>) {
        self.reset_for_admission(false);
    }

    fn reset_for_admission(&mut self, present: bool) {
        self.present = present;
        self.alpha = 1.0;
        self.fade = None;
        self.label.clear();
        self.text.clear();
        self.shown = 0;
        self.typing_clock = 0.0;
        self.playing = false;
        self.clicked = false;
        self.end_icon = false;
        self.auto_wait = 0.0;
        self.auto_release = false;
        self.menu_open = false;
    }

    /// The talk ends: the window is withdrawn with no fade.
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
        self.auto_wait = 0.0;
        self.auto_release = false;
        self.menu_open = false;
    }

    /// The show step: alpha 1 short-circuits, else a 0.3 s fade-in.
    pub(crate) fn show(&mut self, _owner: TalkSession<'_>) -> Option<f32> {
        self.present = true;
        if Self::approximately_one(self.alpha) {
            return None;
        }
        let from = self.alpha;
        self.fade = Some((from, 1.0, SHOW_FADE_SECONDS, 0.0));
        Some(from)
    }

    /// The hide step: an unconditional 0.5 s fade to 0 (the window stays
    /// present).
    pub(crate) fn hide(&mut self, _owner: TalkSession<'_>) {
        self.fade = Some((self.alpha, 0.0, HIDE_FADE_SECONDS, 0.0));
    }

    /// A body line enters (`ShowTextAsync` state 0): the latch clears, the
    /// typewriter starts, the end sign and the click latch reset.
    pub(crate) fn show_text(&mut self, _owner: TalkSession<'_>, text: &str) {
        self.clicked = false;
        self.auto_release = false;
        self.auto_wait = 0.0;
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
    }

    /// The name plate step.
    pub(crate) fn set_label(&mut self, _owner: TalkSession<'_>, name: &str) {
        self.label.clear();
        self.label.push_str(name);
    }

    /// `IsWaitClick`: a manual or automatic release and the typewriter done.
    pub(crate) fn click_level(&self) -> bool {
        (self.clicked || self.auto_release) && !self.playing
    }

    /// Set the manual latch.
    pub(crate) fn latch(&mut self, _owner: TalkSession<'_>) {
        self.clicked = true;
    }

    /// Whether the latch is idle (the chains drain the opening click with it).
    pub(crate) fn latch_idle(&self) -> bool {
        !self.clicked && !self.auto_release
    }

    /// Drain the release latch (`WaitClicked` clears the click it released).
    pub(crate) fn consume_click(&mut self, _owner: TalkSession<'_>) {
        self.clicked = false;
        self.auto_release = false;
    }
}

/// The window view. One persistent entity; `present` shows it.
#[derive(Component)]
pub(crate) struct TalkWindowView;

/// The view has been spawned.
#[derive(Resource)]
pub(crate) struct TalkWindowSpawned;

/// Kept for the chains' teardown queries: the view is persistent now, so no
/// entity carries it and the despawn loops are empty.
#[derive(Component)]
pub(crate) struct TalkWindowRoot;

// ---------------------------------------------------------------------------
// Load and input
// ---------------------------------------------------------------------------

/// Startup: the window's persistent state.
pub(crate) fn load(mut commands: Commands) {
    commands.init_resource::<TalkWindowState>();
}

fn canvas_position(position: Vec2, window_size: Vec2, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window_size.x * 0.5,
        window_size.y * 0.5 - position.y,
    ) / scale
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.try_rect(layouts, path, canvas)
        .is_ok_and(|rect| rect.active && rect.contains(point))
}

fn content_font_size(layouts: &UiLayouts) -> Option<f32> {
    let doc = layouts.document(LAYOUT)?;
    let index = doc.find(CONTENT_NODE).ok()?;
    let text = doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class.ends_with(".CustomTextMesh"))?;
    let number = |key: &str| text.fields.get(key).and_then(Value::as_f64);
    number("m_fontSizeBase")
        .or_else(|| number("m_fontSize"))
        .map(|value| value as f32)
}

/// One tap on the window's controls: the menu button, and the menu's close,
/// auto and skip buttons. Returns whether the tap was consumed (so it does not
/// also release the talk).
fn window_control_tap(
    state: &mut TalkWindowState,
    layouts: &UiLayouts,
    view: &UiPrefabView,
    point: Vec2,
    canvas: Vec2,
) -> bool {
    if !state.menu_open && hit(view, layouts, MENU_BUTTON, point, canvas) {
        state.menu_open = true;
        info!("[talkwin] menu button -> the scenario menu opens");
        return true;
    }
    if state.menu_open {
        if hit(view, layouts, MENU_CLOSE, point, canvas) {
            state.menu_open = false;
            info!("[talkwin] menu close -> the scenario menu closes");
            return true;
        }
        if hit(view, layouts, MENU_AUTO, point, canvas) {
            state.auto = !state.auto;
            state.auto_wait = 0.0;
            info!(
                "[talkwin] auto button -> AutoMode {} (release every {AUTO_INTERVAL_SECONDS:.1}s while the text is done)",
                state.auto
            );
            return true;
        }
        if hit(view, layouts, MENU_SKIP, point, canvas) {
            state.auto = false;
            warn!("[talkwin] all-skip button: ScenarioAllSkipConfirmDialog (176) and the talk SkipEvent are not built; the talk continues");
            return true;
        }
        // The menu's own background eats taps: it is UI, not the talk.
        if hit(view, layouts, MENU_PANEL, point, canvas) {
            return true;
        }
    }
    false
}

/// Update (before the chains step): the click latch, the auto release and the
/// window's UI buttons. A manual tap releases the talk unless it landed on one
/// of the menu's controls.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_click_input(
    mut state: ResMut<TalkWindowState>,
    sessions: Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: Option<ResMut<crate::talk::ActiveTalk>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Option<Res<UiLayouts>>,
    views: Query<(&UiPrefabView, Option<&InheritedVisibility>), With<TalkWindowView>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut gestures: MessageReader<GestureEvent>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();

    // `IsNextTalkIntervalEnd`: the auto release counts while the text is done
    // and no release is pending.
    if state.present && state.auto {
        if state.playing || state.clicked || state.auto_release {
            state.auto_wait = 0.0;
        } else {
            state.auto_wait += time.delta_secs();
            if state.auto_wait >= AUTO_INTERVAL_SECONDS {
                state.auto_wait = 0.0;
                state.auto_release = true;
                info!(
                    "[talkwin] auto mode: {AUTO_INTERVAL_SECONDS:.1}s elapsed, release latch set"
                );
            }
        }
    }

    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0.0 {
        return;
    }
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let view = views.single().ok();
    let mut latch = false;
    for position in &taps {
        let mut handled = false;
        if let (Some(layouts), Some((view, visibility))) = (layouts.as_deref(), view) {
            if state.present
                && visibility.is_none_or(|visibility| visibility.get())
                && scale.is_finite()
                && scale > 0.0
            {
                let point = canvas_position(*position, size, scale);
                handled = window_control_tap(&mut state, layouts, view, point, canvas);
            }
        }
        if !handled {
            latch = true;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        latch = true;
    }
    if !latch {
        return;
    }
    match (sessions, talks) {
        (None, None) => {}
        (Some(mut session), None) => {
            let owner = TalkSession::Player(&mut session);
            let chain = owner.chain_word();
            state.latch(owner);
            info!("[talkwin] click latch set ({chain}; a tap while typing reveals the text, a tap after it releases)");
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!("[talkwin] click latch set ({chain}; a tap while typing reveals the text, a tap after it releases)");
        }
        (Some(_), Some(_)) => {
            panic!("a player session and a paired session are both playing: the entrance guards failed (fail-closed)");
        }
    }
}

/// The smoke input (`MOLY_TALK_TAP_AT_SECS`): at the named seconds, set the
/// same latch a real tap does (the headless window takes no real clicks).
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
        return;
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
                "[talkwin] smoke click latch ({chain}, step {} @ {now:.2}s)",
                *next
            );
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] smoke click latch ({chain}, step {} @ {now:.2}s)",
                *next
            );
        }
        (Some(_), Some(_)) => {
            panic!("a player session and a paired session are both playing: the entrance guards failed (fail-closed)");
        }
    }
}

// ---------------------------------------------------------------------------
// Frame advance and drawing
// ---------------------------------------------------------------------------

/// Update (after the chains): the state machine half (skip, typewriter, fade)
/// and the view (visibility, alpha, texts, menu/auto/end sign, scale).
pub(crate) fn tick_window(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    server: Res<AssetServer>,
    layouts: Option<Res<UiLayouts>>,
    mut state: ResMut<TalkWindowState>,
    mut views: Query<
        (
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
            &mut ResponsiveDialogueMetrics,
        ),
        With<TalkWindowView>,
    >,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    spawned: Option<Res<TalkWindowSpawned>>,
) {
    let dt = time.delta_secs();

    // --- the skip arm (checked before the typewriter advances) ---
    if state.clicked && state.playing {
        let len = state.text.chars().count();
        state.shown = len;
        state.playing = false;
        state.end_icon = true;
        state.clicked = false;
        info!("[talkwin] tap skip: the whole line is revealed ({len} chars), the click latch clears");
    }

    // --- typewriter: character k at 50k ms, DONE one tick after the last ---
    if state.playing {
        state.typing_clock += dt;
        let len = state.text.chars().count();
        let due = ((state.typing_clock / TYPE_INTERVAL).floor() as usize + 1).min(len);
        state.shown = due;
        if state.typing_clock >= TYPE_INTERVAL * len as f32 {
            state.playing = false;
            state.end_icon = true;
            info!(
                "[talkwin] typewriter DONE: {len} chars in {:.2}s (50ms/char), end sign active",
                state.typing_clock
            );
        }
    }

    // --- fade (OutQuad, the tween default) ---
    if let Some((from, target, duration, elapsed)) = state.fade {
        let elapsed = elapsed + dt;
        let t = (elapsed / duration).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        state.alpha = from + (target - from) * eased;
        if elapsed >= duration {
            state.alpha = target;
            state.fade = None;
            info!("[talkwin] fade done: alpha={target:.2} ({duration:.1}s)");
        } else {
            state.fade = Some((from, target, duration, elapsed));
        }
    }

    // The view is spawned once, after the layout root carries the document
    // and its images.
    if spawned.is_none() {
        let Some(layouts) = layouts.as_deref() else { return };
        if !layouts.ready(LAYOUT, &server) {
            return;
        }
        let font = content_font_size(layouts).unwrap_or(0.0);
        commands.spawn((
            TalkWindowView,
            Visibility::Hidden,
            Transform::from_xyz(0., 0., 100.),
            RenderLayers::layer(SITEMAP_LAYER),
            UiPrefabView::new(LAYOUT, SITEMAP_LAYER),
            ResponsiveDialogueMetrics {
                font_px: font,
                line_count: 0,
                bounds: Rect::from_corners(Vec2::ZERO, Vec2::ZERO),
            },
        ));
        commands.insert_resource(TalkWindowSpawned);
        return;
    }
    let Ok((mut view, mut visibility, mut transform, mut metrics)) = views.single_mut() else {
        return;
    };
    if !state.present {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    }
    let (Some(layouts), Some(root_canvas), Ok(window)) =
        (layouts.as_deref(), root_canvas.as_deref(), windows.single())
    else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    if !scale.is_finite() || scale <= 0.0 || !canvas.is_finite() || canvas.min_element() <= 0.0 {
        return;
    }
    crate::canvas::set_scale(&mut transform, scale);
    visibility.set_if_neq(Visibility::Inherited);
    view.set_alpha(WINDOW_NODE, state.alpha);
    view.set_text(LABEL_NODE, state.label.clone());
    let revealed: String = state.text.chars().take(state.shown).collect();
    view.set_text(CONTENT_NODE, revealed);
    view.set_visible(END_SIGN_NODE, state.end_icon);
    view.set_visible(AUTO_SIGNAL_NODE, state.auto);
    view.set_visible(MENU_PANEL, state.menu_open);
    view.set_visible(MENU_BUTTON, !state.menu_open);
    view.set_visible(HIDE_UI_BUTTON, false);
    view.set_visible(SCREENSHOT_BUTTON, false);
    // The source's click area is inactive in the MySekai scene (the layer
    // reads gestures itself); activating it gives the view a full-screen
    // control the agent can press, which the same latch handles.
    view.set_visible(CLICK_AREA_NODE, true);
    if let Some(font) = content_font_size(layouts) {
        if let Ok(panel) = view.try_rect(layouts, PANEL_NODE, canvas) {
            *metrics = stage_layout::source_metrics(scale, &panel, font, &state.text);
        }
    }
}
