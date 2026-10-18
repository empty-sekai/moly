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
//! while typing is the role's `IsClicked`: it reveals the whole text and
//! clears. `WaitClicked` tests `IsWaitClick` once before it waits, so a click
//! the session cannot use is dropped there. Fade: the window's CanvasGroup
//! starts at alpha 1, so the opening show is instantaneous; hide fades to 0
//! over 0.5 s and a later show fades in over 0.3 s (OutQuad). Closing is
//! immediate.
//!
//! **Auto.** The prefab's Auto button toggles the source's `AutoMode`. While
//! it is on, `IsNextTalkIntervalEnd` accumulates `_nextTalkIntervalTime` (2.0
//! s) only while no talk voice plays and the text is done, and its end is the
//! automatic release `IsWaitClick` reports; a voice playing blocks that
//! release (`_isPlayingVoice`) and zeroes the accumulator. [`bind_voice`]
//! reports the talk's playing voice from the voice bus.
//!
//! **Menu and buttons.** The menu button's group fades out over 0.1 s when the
//! scenario menu opens and back in when it closes; the menu's own group fades
//! in over 0.1 s and out over 0.1 s (its serialized animator states are not
//! extracted), and takes no taps while it closes. The all-skip button turns
//! auto off and shows `ScenarioAllSkipConfirmDialog` (type 176, the common
//! two-button window) from [`dialog`]; OK runs the talk's `SkipEvent` (the
//! window ends the wait it holds; the script's own end stays the chain's) and
//! either answer closes the menu. The hide-UI button hides the window and the menu button,
//! turns auto on and makes taps stop releasing the talk, until the next
//! `ShowTalkWindow` (only a hidden UI's menu button then shows it). The
//! screenshot button is hidden: the host has no capture surface. The end sign
//! shows when the typewriter ends; its `PageForward` loop is the star child's
//! `TweenPosition` (0.75 s, y 34 ⇄ 36); its `TweenScale` (0.5 s, 1.1 → 0.8)
//! needs a node scale the shared view does not override.
//!
//! **Named gaps.** The body's TMP auto-size search does not run in the shared
//! renderer (the document's start size is drawn; this view's font size feeds
//! the QA metrics only). A talk voice counts as playing until its player is
//! gone, which includes its asset load. The body uses the shared open-font
//! atlas, not the source's two TMP font assets. An Image whose component is
//! disabled still draws (the window's disabled `bg_story_adv` strip draws as
//! an opaque image at the window's bottom edge). The pair chain's all-skip
//! does not jump its script yet: its `SkipEvent` only ends the wait the window
//! holds.

use bevy::audio::AudioSink;
use bevy::camera::visibility::RenderLayers;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde_json::Value;

use crate::gesture::{GestureEvent, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layout::{UiLayouts, UiPrefabView};

pub(crate) mod dialog;
mod stage_layout;
pub(crate) use stage_layout::ResponsiveDialogueMetrics;
use dialog::AllSkipConfirm;

/// One tap on the window.
enum Tap {
    /// The control took it.
    Consumed,
    /// The all-skip button: the confirmation opens.
    AllSkip,
    /// No control: the tap is the talk's.
    Passed,
}

fn chain_word(
    sessions: &Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: &Option<ResMut<crate::talk::ActiveTalk>>,
) -> String {
    match (sessions, talks) {
        (Some(session), None) => format!("player chain talk {}", session.talk_id()),
        (None, Some(talk)) => format!("pair chain talk {}", talk.talk_id()),
        _ => "no talk".to_owned(),
    }
}

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
/// The window group (the layer's `_talkWindowCanvasGroup`): the fade writes
/// its alpha and `SetHideUI` hides its game object.
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
/// The star `EndSign`'s animator shows; its own prefab tweens loop.
const END_STAR_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/EndSign/star";
/// The auto-mode marker (the layer's `_autoSignal`).
const AUTO_SIGNAL_NODE: &str = "ComponentRoot/TalkWindow/Window/AutoSignal";
/// The full-screen click area (the layer's `_clickAreaInOutGame`).
const CLICK_AREA_NODE: &str = "ComponentRoot/TalkWindow/ClickArea";
/// The scenario menu and its parts.
const MENU_PANEL: &str = "ComponentRoot/RightTop/ScenarioMenu/Base";
const MENU_CONTENT: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content";
const MENU_CLOSE: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/UIPartsCloseButton";
const MENU_AUTO: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/Auto";
const MENU_SKIP: &str = "ComponentRoot/RightTop/ScenarioMenu/Base/Content/Skip";
/// The menu button's group (the layer's `_menuButtonCanvasGroup`, the
/// `MenuButtons` node) and the button node inside it.
const MENU_BUTTON_GROUP: &str = "ComponentRoot/RightTop/MenuButtons";
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
/// The menu button's fade of `ShowMenuButton`/`HideMenuButton` when not
/// immediate: `0x3dcccccd` (0.1 s).
const MENU_BUTTON_FADE_SECONDS: f32 = 0.1;
/// The scenario menu's open/close fade (`ScenarioMenu.Open`/`Close` play
/// "OpenSettings"/"CloseEx" over their CanvasGroup).
const MENU_FADE_SECONDS: f32 = 0.1;
/// `EndSign/star`'s `TweenPosition` loop: 0.75 s between (x, y) and (x, y+2).
const END_STAR_POSITION_SECONDS: f32 = 0.75;
/// The key that reveals the whole line (`ShowTextAsync`'s `_isClicked`; the
/// role's click area is inactive in the game's scene). After the typewriter,
/// it is a tap.
const SKIP_KEY: KeyCode = KeyCode::Space;

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
    /// `ShowTextAsync`'s `_isClicked`: the next typed line is revealed whole.
    skip_text: bool,
    /// The manual click latch the chains' `WaitClicked` reads.
    clicked: bool,
    /// `_endIconObj`'s active state.
    end_icon: bool,
    /// `playerData.AutoMode` and its marker.
    auto: bool,
    /// The auto interval accumulator (`_nextTalkInterval`).
    auto_wait: f32,
    /// The automatic release (`IsNextTalkIntervalEnd` at `WaitClicked`).
    auto_release: bool,
    /// `_isPlayingVoice`: the talk's voice plays.
    voice_playing: bool,
    /// The scenario menu is open (or closing).
    menu_open: bool,
    /// The menu's group alpha: 0 closed, 1 open, in between while it fades.
    menu_alpha: f32,
    /// The scenario menu takes no taps until this time (it closes).
    closing_until: f32,
    /// The menu button's group alpha (`_menuButtonCanvasGroup`).
    menu_button_alpha: f32,
    /// The alpha the menu button fades to.
    menu_button_target: f32,
    /// `playerData.IsHideUI`.
    hide_ui: bool,
    /// The end star's loop clock (seconds since the end sign showed).
    end_star_clock: f32,
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
            skip_text: false,
            clicked: false,
            end_icon: false,
            auto: false,
            auto_wait: 0.0,
            auto_release: false,
            voice_playing: false,
            menu_open: false,
            menu_alpha: 0.0,
            closing_until: 0.0,
            menu_button_alpha: 1.0,
            menu_button_target: 1.0,
            hide_ui: false,
            end_star_clock: 0.0,
        }
    }
}

impl TalkWindowState {
    /// Read-only visible transcript for accessibility/acceptance tools.
    pub(crate) fn transcript(&self) -> (&str, &str, bool, bool) {
        let displaying = self.present
            && !self.hide_ui
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
        self.skip_text = false;
        self.clicked = false;
        self.end_icon = false;
        self.auto_wait = 0.0;
        self.auto_release = false;
        self.menu_open = false;
        self.menu_alpha = 0.0;
        self.closing_until = 0.0;
        // Auto off and the menu and its button back, as the screen sets them
        // up (`ShowMenuButton`; `OnClickAllSkip` turns auto off).
        self.auto = false;
        self.menu_button_alpha = 1.0;
        self.menu_button_target = 1.0;
        self.hide_ui = false;
        self.end_star_clock = 0.0;
    }

    /// The talk ends: the window is withdrawn with no fade.
    pub(crate) fn close(&mut self, _owner: TalkSession<'_>) {
        self.reset_for_admission(false);
    }

    /// `ShowTalkWindow`: alpha 1 short-circuits, else a 0.3 s fade-in. The
    /// window's game object is made active again, so a hidden UI shows.
    pub(crate) fn show(&mut self, _owner: TalkSession<'_>) -> Option<f32> {
        self.present = true;
        self.hide_ui = false;
        if Self::approximately_one(self.alpha) {
            return None;
        }
        let from = self.alpha;
        self.fade = Some((from, 1.0, SHOW_FADE_SECONDS, 0.0));
        Some(from)
    }

    /// `HideTalkWindow`: an unconditional 0.5 s fade to 0 (the window stays
    /// present).
    pub(crate) fn hide(&mut self, _owner: TalkSession<'_>) {
        self.fade = Some((self.alpha, 0.0, HIDE_FADE_SECONDS, 0.0));
    }

    /// A body line enters (`ShowTextAsync` state 0): the click and the
    /// typewriter start, the end sign and the click latch reset.
    pub(crate) fn show_text(&mut self, _owner: TalkSession<'_>, text: &str) {
        self.clicked = false;
        self.auto_release = false;
        self.auto_wait = 0.0;
        self.skip_text = false;
        self.text.clear();
        self.text.push_str(text);
        self.playing = true;
        self.end_icon = false;
        self.end_star_clock = 0.0;
        self.typing_clock = 0.0;
        // The loop types no character of an empty text before it ends.
        self.shown = usize::from(!text.is_empty());
    }

    /// The name plate step.
    pub(crate) fn set_label(&mut self, _owner: TalkSession<'_>, name: &str) {
        self.label.clear();
        self.label.push_str(name);
    }

    /// `IsWaitClick`: a click (manual, or `IsNextTalkIntervalEnd`'s automatic
    /// release) with the typewriter done.
    pub(crate) fn click_level(&self) -> bool {
        (self.clicked || self.auto_release) && !self.playing
    }

    /// Set the manual latch (a tap, `SetClicked`).
    pub(crate) fn latch(&mut self, _owner: TalkSession<'_>) {
        self.clicked = true;
    }

    /// Whether the release latch is idle (the chains drain the opening click
    /// with it).
    pub(crate) fn latch_idle(&self) -> bool {
        !self.clicked && !self.auto_release
    }

    /// Drain the release latch (`WaitClicked` clears the click it released).
    pub(crate) fn consume_click(&mut self, _owner: TalkSession<'_>) {
        self.clicked = false;
        self.auto_release = false;
    }

    /// The talk's `SkipEvent`: `ExecuteAllSkip` stops the player and the dialog
    /// layer's callback disposes of the talk, so whatever wait it holds ends —
    /// the whole line is shown and the latches that would release it again are
    /// cleared. (The player chain owns the session's end: it steps on when the
    /// latches are idle and the typewriter is done.)
    pub(crate) fn skip_event(&mut self) {
        self.auto = false;
        self.auto_release = false;
        self.auto_wait = 0.0;
        self.clicked = false;
        self.skip_text = false;
        if self.playing {
            self.shown = self.text.chars().count();
            self.playing = false;
            self.end_icon = true;
            self.end_star_clock = 0.0;
        }
    }

    /// `SetHideUI`: hiding hides the window (`_talkWindowCanvasGroup`'s game
    /// object), turns auto on (`SetAutoMode`) and fades the menu button out
    /// (`HideMenuButton`); showing fades the menu button back in
    /// (`ShowMenuButton`).
    fn set_hide_ui(&mut self, hide: bool) {
        self.hide_ui = hide;
        if hide {
            self.auto = true;
            self.auto_wait = 0.0;
            self.menu_open = false;
            self.menu_alpha = 0.0;
            self.closing_until = 0.0;
            self.menu_button_alpha = 0.0;
            self.menu_button_target = 0.0;
        } else {
            self.menu_button_alpha = 1.0;
            self.menu_button_target = 1.0;
        }
    }

    /// `HideMenuAndShowMenuButton(false)`: the menu fades out, then takes no
    /// taps, and the menu button fades in after it.
    fn close_menu(&mut self, now: f32) {
        self.menu_open = false;
        self.closing_until = now + MENU_FADE_SECONDS;
        self.menu_button_alpha = 0.0;
        self.menu_button_target = 1.0;
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

/// Which talk owns the playing voice (`MySekaiTalkEngine`: `SetIsPlayingVoice`
/// true before a voice line starts and false once it ends).
#[derive(Resource, Default)]
pub(crate) struct TalkVoiceBinding {
    player: Option<Entity>,
}

// ---------------------------------------------------------------------------
// Load and input
// ---------------------------------------------------------------------------

/// Startup: the window's persistent state.
pub(crate) fn load(mut commands: Commands) {
    commands.init_resource::<TalkWindowState>();
    commands.init_resource::<AllSkipConfirm>();
    commands.init_resource::<TalkVoiceBinding>();
}

/// Registration of what the shared schedule does not hold: the talk voice's
/// flag before the tap handler, and the all-skip confirmation's own systems.
/// `load`, `read_click_input` and `tick_window` are placed in `schedule.rs`;
/// adding them here too would make their ordering targets ambiguous.
pub(crate) fn install(app: &mut App) {
    app.add_systems(Update, bind_voice.before(read_click_input));
    dialog::install(app);
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

// ---------------------------------------------------------------------------
// Talk voice
// ---------------------------------------------------------------------------

/// Update (before the click input): whether the talk's voice plays. The voice
/// bus spawns one player per line and keeps it until the line ends; the
/// engine sets `SetIsPlayingVoice(true)` when a line starts and `false` once
/// it stopped and disposed of it, so a live talk-voice player is the flag.
pub(crate) fn bind_voice(
    channel: Res<crate::audio::VoiceChannel>,
    sources: Query<&crate::voice_pcm::VoiceSource>,
    sinks: Query<&AudioSink>,
    mut state: ResMut<TalkWindowState>,
    mut binding: ResMut<TalkVoiceBinding>,
) {
    let sink = channel.active_sink().filter(|sink| sources.contains(*sink));
    let Some(sink) = sink else {
        if binding.player.take().is_some() {
            info!("[talkwin] the talk voice ended (SetIsPlayingVoice(false))");
        }
        state.voice_playing = false;
        return;
    };
    if binding.player != Some(sink) {
        binding.player = Some(sink);
        info!("[talkwin] the talk started a voice line (SetIsPlayingVoice(true): auto mode waits for it)");
    }
    // A player still reading its asset has no sink yet and counts as playing;
    // a sink that is empty or stopped has played the line out, which the line's
    // reap ends.
    state.voice_playing = match sinks.get(sink) {
        Ok(sink) => !sink.empty(),
        Err(_) => true,
    };
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// One tap on the window's controls: the menu button, the menu's close, auto
/// and skip buttons, the hide-UI button and the clicks that turn the menu
/// button off. Returns whether the tap was consumed (so it does not also
/// release the talk).
fn window_control_tap(
    state: &mut TalkWindowState,
    layouts: &UiLayouts,
    view: &UiPrefabView,
    point: Vec2,
    canvas: Vec2,
    now: f32,
) -> Tap {
    // A hidden UI shows no window; its menu button is the way back.
    if state.hide_ui {
        if hit(view, layouts, MENU_BUTTON, point, canvas) {
            state.set_hide_ui(false);
            info!("[talkwin] menu button with the UI hidden -> SetHideUI(false) (ShowMenuButton)");
            return Tap::Consumed;
        }
        return Tap::Passed;
    }
    if !state.menu_open && hit(view, layouts, MENU_BUTTON, point, canvas) {
        // `<InitializeMenu>b__51_1`: `ShowMenuAndHideMenuButton(false)`.
        state.menu_open = true;
        state.menu_alpha = 0.0;
        state.closing_until = 0.0;
        state.menu_button_alpha = 0.0;
        state.menu_button_target = 0.0;
        info!("[talkwin] menu button -> the scenario menu opens (menu button out 0.1s, menu in 0.1s)");
        return Tap::Consumed;
    }
    if state.menu_open {
        // A closing menu is not interactable.
        if now < state.closing_until {
            return if hit(view, layouts, MENU_CLOSE, point, canvas)
                || hit(view, layouts, MENU_AUTO, point, canvas)
                || hit(view, layouts, MENU_SKIP, point, canvas)
            {
                Tap::Consumed
            } else {
                Tap::Passed
            };
        }
        if hit(view, layouts, MENU_CLOSE, point, canvas) {
            // `<InitializeMenu>b__51_0`: `HideMenuAndShowMenuButton(false)`.
            info!("[talkwin] menu close -> the scenario menu closes (out 0.1s, menu button in 0.1s)");
            state.close_menu(now);
            return Tap::Consumed;
        }
        if hit(view, layouts, MENU_AUTO, point, canvas) {
            // `OnClickAutoMode`: `SetAutoMode(!IsAutoMode)`; the interval
            // carries over.
            state.auto = !state.auto;
            info!(
                "[talkwin] auto button -> AutoMode {} (release every {AUTO_INTERVAL_SECONDS:.1}s while the text is done and no voice plays)",
                state.auto
            );
            return Tap::Consumed;
        }
        if hit(view, layouts, MENU_SKIP, point, canvas) {
            // `OnClickAllSkip`: auto off, then the confirmation.
            state.auto = false;
            state.auto_wait = 0.0;
            return Tap::AllSkip;
        }
        // The menu's own background eats taps: it is UI, not the talk.
        if hit(view, layouts, MENU_PANEL, point, canvas) {
            return Tap::Consumed;
        }
    }
    if hit(view, layouts, HIDE_UI_BUTTON, point, canvas) {
        // `OnClickHideUI`: `SetHideUI(!IsHideUI)`. The window goes, so only a
        // hidden UI's menu button can show it again.
        state.set_hide_ui(true);
        info!("[talkwin] hide-UI button -> SetHideUI(true): window and menu button hidden, auto on; taps do not release until ShowTalkWindow");
        return Tap::Consumed;
    }
    Tap::Passed
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
    mut confirm: ResMut<AllSkipConfirm>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Option<Res<UiLayouts>>,
    views: Query<(&UiPrefabView, Option<&InheritedVisibility>), With<TalkWindowView>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    screens: Option<Res<crate::ui_layers::ScreenManager>>,
    mut gestures: MessageReader<GestureEvent>,
) {
    let now = time.elapsed_secs();
    // The confirmation's answer: OK runs the talk's `SkipEvent`, either
    // answer `HideMenuAndShowMenuButton(false)`.
    while let Some(ok) = confirm.take_answer() {
        if ok {
            state.skip_event();
            info!(
                "[talkwin] all skip confirmed: SkipEvent for {} (the wait it holds ends)",
                chain_word(&sessions, &talks)
            );
        } else {
            info!("[talkwin] all skip cancelled");
        }
        state.close_menu(now);
    }

    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();

    // While the confirmation is up the taps are its (modal); it answers through
    // the common dialog's own handler, and the answer is read here.
    let taps: Vec<Vec2> = if confirm.is_open() {
        if let Some(screens) = screens.as_deref() {
            if let Some(ok) = confirm.settle(screens) {
                if ok {
                    state.skip_event();
                    info!(
                        "[talkwin] all skip: SkipEvent for {} (the wait it holds ends)",
                        chain_word(&sessions, &talks)
                    );
                }
                state.close_menu(now);
            }
        }
        Vec::new()
    } else {
        taps
    };

    // `IsNextTalkIntervalEnd`, once per frame as `WaitClicked` consults it: a
    // voice playing or the typewriter running zeroes the interval (and the
    // release it may hold); otherwise it accumulates and its end is the
    // release.
    if state.present && state.auto {
        if state.voice_playing || state.playing {
            state.auto_wait = 0.0;
            state.auto_release = false;
        } else if state.auto_release {
            state.auto_wait = AUTO_INTERVAL_SECONDS;
        } else {
            state.auto_wait += time.delta_secs();
            if state.auto_wait >= AUTO_INTERVAL_SECONDS {
                state.auto_wait = AUTO_INTERVAL_SECONDS;
                state.auto_release = true;
                info!("[talkwin] auto mode: IsNextTalkIntervalEnd ({AUTO_INTERVAL_SECONDS:.1}s, no voice), release set");
            }
        }
    } else if !state.auto {
        state.auto_wait = 0.0;
        state.auto_release = false;
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
    let mut all_skip = false;
    for position in &taps {
        let mut tap = Tap::Passed;
        if let (Some(layouts), Some((view, visibility))) = (layouts.as_deref(), view) {
            if state.present
                && visibility.is_none_or(|visibility| visibility.get())
                && scale.is_finite()
                && scale > 0.0
            {
                let point = canvas_position(*position, size, scale);
                tap = window_control_tap(&mut state, layouts, view, point, canvas, now);
            }
        }
        match tap {
            Tap::Consumed => {}
            Tap::AllSkip => all_skip = true,
            Tap::Passed => latch = true,
        }
    }
    // `OnClick`: a hidden UI swallows the click.
    if state.hide_ui {
        latch = false;
    }
    if keys.just_pressed(SKIP_KEY) {
        // `ShowTextAsync`'s `_isClicked`: while typing, the line is revealed;
        // after it, this is a tap.
        if state.playing {
            state.skip_text = true;
        } else {
            latch = true;
        }
    }
    if all_skip {
        // `OnClickAllSkip`: the confirmation, over the window; its system
        // opens it before the common dialog's tap handler.
        match layouts.as_deref() {
            Some(layouts) => confirm.request(AllSkipConfirm::message(layouts)),
            None => error!("[talkwin] all skip: the UI root is not loaded; the confirmation was not requested"),
        }
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

    // --- the skip arm (`ShowTextAsync`'s `_isClicked`, before each
    // character) ---
    if state.skip_text && state.playing {
        let len = state.text.chars().count();
        state.shown = len;
        state.playing = false;
        state.end_icon = true;
        state.end_star_clock = 0.0;
        // `_contentText.SetText(text); _isClicked = false;`
        state.skip_text = false;
        state.clicked = false;
        info!("[talkwin] tap skip: the whole line is revealed ({len} chars), the click clears");
    }
    state.skip_text &= state.playing;

    // --- typewriter: character k at 50k ms, DONE one tick after the last ---
    if state.playing {
        state.typing_clock += dt;
        let len = state.text.chars().count();
        let due = ((state.typing_clock / TYPE_INTERVAL).floor() as usize + 1).min(len);
        state.shown = due;
        if state.typing_clock >= TYPE_INTERVAL * len as f32 {
            state.playing = false;
            state.end_icon = true;
            state.end_star_clock = 0.0;
            info!(
                "[talkwin] typewriter DONE: {len} chars in {:.2}s (50ms/char), end sign active",
                state.typing_clock
            );
        }
    }
    if state.end_icon {
        state.end_star_clock += dt;
    }

    // --- the menu button's fade (ShowMenuButton/HideMenuButton) ---
    let step = (dt / MENU_BUTTON_FADE_SECONDS).min(1.0);
    state.menu_button_alpha = if state.menu_button_alpha < state.menu_button_target {
        (state.menu_button_alpha + step).min(state.menu_button_target)
    } else {
        (state.menu_button_alpha - step).max(state.menu_button_target)
    };
    // --- the menu's fade (ScenarioMenu.Open/Close) ---
    let menu_step = (dt / MENU_FADE_SECONDS).min(1.0);
    let menu_target = if state.menu_open { 1.0 } else { 0.0 };
    state.menu_alpha = if state.menu_alpha < menu_target {
        (state.menu_alpha + menu_step).min(menu_target)
    } else {
        (state.menu_alpha - menu_step).max(menu_target)
    };

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
    if !state.present || state.hide_ui {
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
    // `ShowTextAsync` writes the typed prefix, except the empty line's "\n",
    // whose typed form is "", and a reveal (Skip) writes the whole line.
    if state.shown == 0 || state.shown >= state.text.chars().count() {
        view.set_text(CONTENT_NODE, state.text.clone());
    } else {
        let revealed: String = state.text.chars().take(state.shown).collect();
        view.set_text(CONTENT_NODE, revealed);
    }
    view.set_visible(END_SIGN_NODE, state.end_icon);
    // `EndSign`'s animator plays the star: its own prefab tweens loop (the
    // node is serialized inactive; the animator shows it).
    view.set_visible(END_STAR_NODE, state.end_icon);
    if state.end_icon {
        // `TweenPosition`: 0.75 s between y 34 and 36 (x stays -21), looping.
        let t = (state.end_star_clock / END_STAR_POSITION_SECONDS).fract();
        let rising = if t < 0.5 { t } else { 1.0 - t };
        view.set_anchored_position(
            END_STAR_NODE,
            Vec2::new(-21.0, 34.0 + 2.0 * (1.0 - (1.0 - rising) * (1.0 - rising))),
        );
    }
    view.set_visible(AUTO_SIGNAL_NODE, state.auto);
    // The menu's own group is its `_menu` CanvasGroup, faded by Open/Close;
    // it is inactive (and unscaled) while closed.
    view.set_alpha(MENU_CONTENT, state.menu_alpha);
    view.set_visible(MENU_PANEL, state.menu_alpha > 0.0);
    // `_menuButtonCanvasGroup` (`MenuButtons`) fades between 0 and 1.
    view.set_alpha(MENU_BUTTON_GROUP, state.menu_button_alpha);
    view.set_alpha(MENU_BUTTON, state.menu_button_alpha);
    view.set_visible(MENU_BUTTON_GROUP, true);
    view.set_visible(MENU_BUTTON, true);
    // `SetupOutGame`: in the field the hide-UI button shows; the screenshot
    // button needs a capture surface this host does not have.
    view.set_visible(HIDE_UI_BUTTON, true);
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
