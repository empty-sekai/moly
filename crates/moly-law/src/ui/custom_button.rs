//! The game's `CustomButton` pointer handling and its click gate.
//!
//! A press is taken only when exactly one touch is down and no other
//! selectable holds the input manager's control, unless the button is marked
//! "absolutely press". Taking a press registers the pointer id, gives the
//! button the manager's control, and plays the press interaction. Releasing
//! with the same pointer finishes control (the release interaction plays) and
//! arms the click check. The click then needs the same pointer again; it
//! releases the registration and runs the click gate: the shared interval
//! debounce (0.2 s of real time since the last accepted click of any button
//! that uses it; the start time is 0 when the game boots), and, when not
//! blocked and the button is active and interactable, the button's sound.
//! Only when the gate passes does the engine button's click run, which again
//! requires the left button, an active object and an interactable button.
//!
//! The gate's sound is a `CustomSelectableDefine.PlaySE(se, otherSeName)`
//! call, which the host runs against its cue bank with [`play_se`]: the table
//! maps Decide / Cancel to the common cues and the three MySekai kinds to the
//! MySekai cues; Other plays the serialized cue name only when that cue exists
//! (a missing cue is logged and nothing plays); None plays nothing.

/// `SeType` in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeType {
    None,
    Decide,
    Cancel,
    Other,
    MysekaiDecide,
    MysekaiSelect,
    MysekaiCancel,
}

impl SeType {
    pub fn from_serialized(value: i64) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::Decide,
            2 => Self::Cancel,
            3 => Self::Other,
            4 => Self::MysekaiDecide,
            5 => Self::MysekaiSelect,
            6 => Self::MysekaiCancel,
            _ => return None,
        })
    }

    /// The static cue table; empty for None and Other.
    pub fn table_cue(self) -> &'static str {
        match self {
            Self::Decide => "SE_DECIDE1",
            Self::Cancel => "SE_CANCEL",
            Self::MysekaiDecide => "se_mysekai_ui_decision",
            Self::MysekaiSelect => "se_mysekai_ui_select",
            Self::MysekaiCancel => "se_mysekai_ui_cancel",
            Self::None | Self::Other => "",
        }
    }
}

/// `CustomSelectableDefine.PlaySE`: the cue that plays, if any.
pub fn play_se(se: SeType, other_se_name: &str, other_cue_exists: impl Fn(&str) -> bool) -> Option<String> {
    let cue = se.table_cue();
    if !cue.is_empty() {
        return Some(cue.to_owned());
    }
    if se != SeType::Other || !other_cue_exists(other_se_name) {
        return None;
    }
    Some(other_se_name.to_owned())
}

/// `InputManager.IntervalUseType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalUseType {
    None,
    Use,
}

impl IntervalUseType {
    pub fn from_serialized(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Use),
            _ => None,
        }
    }
}

/// `InputManager.ControlState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlState {
    #[default]
    NoControl,
    Press,
    ClickCheck,
}

/// `InputManager.INTERVAL_SEC`.
pub const INTERVAL_SEC: f32 = 0.2;

/// The input manager's state shared by every custom button.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputManager {
    /// Real time of the last accepted click (0 at boot).
    pub interval_start_time: f32,
    /// The selectable holding control.
    controlled: Option<u64>,
    /// Pointer id registered per selectable.
    pointer_ids: Vec<(u64, i32)>,
}

impl InputManager {
    /// `CheckAndResetIntervalTime`: true when the click is blocked.
    pub fn check_and_reset_interval_time(&mut self, interval: IntervalUseType, now: f32) -> bool {
        if interval == IntervalUseType::None {
            return false;
        }
        if INTERVAL_SEC <= now - self.interval_start_time {
            self.interval_start_time = now;
            false
        } else {
            true
        }
    }

    /// `ControledSelectable`.
    pub fn controlled_selectable(&self) -> bool {
        self.controlled.is_some()
    }

    /// `EnableTouchControl`: the pointer id registered for `source` matches.
    pub fn enable_touch_control(&self, source: u64, pointer_id: i32) -> bool {
        self.pointer_ids.iter().any(|(s, p)| *s == source && *p == pointer_id)
    }

    fn register_touch_control(&mut self, source: u64, pointer_id: i32) {
        if let Some(entry) = self.pointer_ids.iter_mut().find(|(s, _)| *s == source) {
            entry.1 = pointer_id;
        } else {
            self.pointer_ids.push((source, pointer_id));
        }
    }

    fn release_touch_control(&mut self, source: u64) {
        self.pointer_ids.retain(|(s, _)| *s != source);
    }

    /// `FinishControlSelectable`: returns the selectable whose finish
    /// callback runs (the one that started control), then clears control.
    fn finish_control_selectable(&mut self) -> Option<u64> {
        self.controlled.take()
    }
}

/// Serialized `CustomButton` fields the pointer handlers read.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomButtonConfig {
    pub se: SeType,
    pub other_se_name: String,
    pub interval: IntervalUseType,
    pub absolutely_press: bool,
    pub enable_long_press: bool,
    pub enable_hold_repeat: bool,
}

/// Per-button runtime state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomButtonState {
    pub control: ControlState,
    pub executed_long_press: bool,
}

/// What a handler asks the host to do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum ButtonEffect {
    /// The press interaction (override delegate, else the view interaction's press).
    PressEffect,
    /// The release interaction (override delegate, else the view interaction's release).
    ReleaseEffect,
    /// Long-press checking starts (not ported further here).
    StartLongPressCheck,
    /// Hold-repeat starts (not ported further here).
    StartHoldRepeat,
    /// `CustomSelectableDefine.PlaySE(se, otherSeName)` runs; [`play_se`]
    /// resolves it against the host's cue bank.
    PlaySe { se: SeType, other_se_name: String },
    /// The button's click event runs.
    Click,
}

/// The pointer facts the handlers read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub pointer_id: i32,
    pub left_button: bool,
}

/// The button facts the handlers read at event time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonLive {
    /// `IsActive()`.
    pub active: bool,
    /// `IsInteractable()`.
    pub interactable: bool,
}

/// `OnFinishedControlSelectable` for the button that started control.
pub fn on_finished_control_selectable(state: &mut CustomButtonState, effects: &mut Vec<ButtonEffect>) {
    if state.control == ControlState::Press {
        effects.push(ButtonEffect::ReleaseEffect);
    }
    state.control = ControlState::NoControl;
}

/// `CustomButton.OnPointerDown`.
pub fn on_pointer_down(
    manager: &mut InputManager,
    state: &mut CustomButtonState,
    config: &CustomButtonConfig,
    button: u64,
    pointer: Pointer,
    touch_count: u32,
    live: ButtonLive,
) -> Vec<ButtonEffect> {
    let mut effects = Vec::new();
    if state.control == ControlState::Press {
        return effects;
    }
    let takes = (touch_count == 1 && !manager.controlled_selectable()) || config.absolutely_press;
    if !takes {
        state.control = ControlState::NoControl;
        return effects;
    }
    if !live.interactable {
        return effects;
    }
    manager.register_touch_control(button, pointer.pointer_id);
    manager.controlled = Some(button);
    state.control = ControlState::Press;
    effects.push(ButtonEffect::PressEffect);
    if config.enable_long_press {
        state.executed_long_press = false;
        effects.push(ButtonEffect::StartLongPressCheck);
    }
    if config.enable_hold_repeat {
        effects.push(ButtonEffect::StartHoldRepeat);
    }
    effects
}

/// `CustomButton.OnPointerUp`. When control finishes, the finish callback
/// of the selectable that started control runs; for this button that is
/// [`on_finished_control_selectable`].
pub fn on_pointer_up(
    manager: &mut InputManager,
    state: &mut CustomButtonState,
    config: &CustomButtonConfig,
    button: u64,
    pointer: Pointer,
) -> Vec<ButtonEffect> {
    let mut effects = Vec::new();
    if state.control == ControlState::Press
        && (manager.enable_touch_control(button, pointer.pointer_id) || config.absolutely_press)
    {
        if manager.finish_control_selectable() == Some(button) {
            on_finished_control_selectable(state, &mut effects);
        }
        state.control = ControlState::ClickCheck;
    }
    state.executed_long_press = false;
    effects
}

/// `CustomSelectableDefine.CheckPointerClickAction`: true when not blocked.
pub fn check_pointer_click_action(
    manager: &mut InputManager,
    config: &CustomButtonConfig,
    now: f32,
    live: ButtonLive,
    effects: &mut Vec<ButtonEffect>,
) -> bool {
    let blocked = manager.check_and_reset_interval_time(config.interval, now);
    if !blocked && live.active && live.interactable {
        effects.push(ButtonEffect::PlaySe { se: config.se, other_se_name: config.other_se_name.clone() });
    }
    !blocked
}

/// `Button.OnPointerClick` then `Button.Press`.
fn engine_button_click(pointer: Pointer, live: ButtonLive, effects: &mut Vec<ButtonEffect>) {
    if !pointer.left_button || !live.active || !live.interactable {
        return;
    }
    effects.push(ButtonEffect::Click);
}

/// `CustomButton.OnPointerClick`.
pub fn on_pointer_click(
    manager: &mut InputManager,
    state: &mut CustomButtonState,
    config: &CustomButtonConfig,
    button: u64,
    pointer: Pointer,
    now: f32,
    live: ButtonLive,
) -> Vec<ButtonEffect> {
    let mut effects = Vec::new();
    let proceeds = (state.control == ControlState::ClickCheck
        && manager.enable_touch_control(button, pointer.pointer_id))
        || config.absolutely_press;
    if !proceeds {
        return effects;
    }
    manager.release_touch_control(button);
    if !config.enable_long_press {
        if check_pointer_click_action(manager, config, now, live, &mut effects) {
            engine_button_click(pointer, live, &mut effects);
        }
        return effects;
    }
    if !state.executed_long_press && check_pointer_click_action(manager, config, now, live, &mut effects) {
        engine_button_click(pointer, live, &mut effects);
    }
    state.executed_long_press = false;
    effects
}

/// `CustomButton.OnPointerExit` (after the engine selectable's exit).
pub fn on_pointer_exit(
    manager: &mut InputManager,
    state: &mut CustomButtonState,
    config: &CustomButtonConfig,
    button: u64,
    pointer: Pointer,
) -> Vec<ButtonEffect> {
    let mut effects = Vec::new();
    if state.control == ControlState::Press
        && (manager.enable_touch_control(button, pointer.pointer_id) || config.absolutely_press)
    {
        manager.release_touch_control(button);
        if manager.finish_control_selectable() == Some(button) {
            on_finished_control_selectable(state, &mut effects);
        }
        if config.enable_long_press {
            state.executed_long_press = false;
        }
    }
    effects
}
