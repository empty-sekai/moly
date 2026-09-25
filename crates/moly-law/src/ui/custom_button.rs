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
//! The gate reads the real-time clock twice, once to compare and once more
//! to store, so it takes the clock itself, not one frame's time.
//! Only when the gate passes does the engine button's click run, which again
//! requires the left button, an active object and an interactable button.
//!
//! Taking control stores the button both as the controlling selectable and
//! as the owner of the finish callback. Finishing control (a release or an
//! exit of the pressed pointer, or the button being disabled while pressed)
//! runs the stored callback, whichever button stored it last, and clears
//! both: a second, "absolutely press" button that took control while the
//! first was pressed is the one whose release plays and whose press ends.
//!
//! Long-press checking and hold-repeat are started by a press (with the
//! button's options) and stopped again: every release stops hold-repeat and
//! cancels the long-press check; an exit stops hold-repeat, and cancels the
//! long-press check when the exit finishes a press of a long-press button; a
//! click of a long-press button cancels the check after its gate.
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
    /// The selectable holding control, which is also the owner of the stored
    /// finish callback: a custom button's `StartControlSelectable` call
    /// stores itself and its own `OnFinishedControlSelectable` together.
    controlled: Option<u64>,
    /// Pointer id registered per selectable.
    pointer_ids: Vec<(u64, i32)>,
}

impl InputManager {
    /// `CheckAndResetIntervalTime`: true when the click is blocked. `clock`
    /// is `Time.realtimeSinceStartup`: one read to compare and, when the
    /// click passes, a second read to store. The click is blocked only when
    /// the elapsed time compares less than the interval, so an unordered
    /// comparison passes and resets.
    pub fn check_and_reset_interval_time(&mut self, interval: IntervalUseType, clock: &mut impl FnMut() -> f32) -> bool {
        if interval == IntervalUseType::None {
            return false;
        }
        if clock() - self.interval_start_time < INTERVAL_SEC {
            true
        } else {
            self.interval_start_time = clock();
            false
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

    /// `FinishControlSelectable`: returns the button whose stored finish
    /// callback runs (the one that took control last, not necessarily the
    /// caller), then clears control and the callback.
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
    /// `InputManager.CancelLongPressCheck` runs.
    CancelLongPressCheck,
    /// Hold-repeat starts (not ported further here).
    StartHoldRepeat,
    /// Hold-repeat stops: the holding flag clears and its cancellation
    /// source is cancelled and released.
    StopHoldRepeat,
    /// The stored finish callback belongs to another button: the host runs
    /// [`on_finished_control_selectable`] on that button's state (and plays
    /// its release on that button's view) at this point of the sequence.
    FinishedControlOf(u64),
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

/// `OnFinishedControlSelectable` of the button whose callback is stored.
pub fn on_finished_control_selectable(state: &mut CustomButtonState, effects: &mut Vec<ButtonEffect>) {
    if state.control == ControlState::Press {
        effects.push(ButtonEffect::ReleaseEffect);
    }
    state.control = ControlState::NoControl;
}

/// `InputManager.FinishControlSelectable` called by `button`: the stored
/// callback runs, on this button's state when it is this button's, else as
/// [`ButtonEffect::FinishedControlOf`] for the host.
fn finish_control(
    manager: &mut InputManager,
    state: &mut CustomButtonState,
    button: u64,
    effects: &mut Vec<ButtonEffect>,
) {
    match manager.finish_control_selectable() {
        Some(owner) if owner == button => on_finished_control_selectable(state, effects),
        Some(owner) => effects.push(ButtonEffect::FinishedControlOf(owner)),
        None => {}
    }
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
        effects.push(ButtonEffect::CancelLongPressCheck);
        effects.push(ButtonEffect::StartLongPressCheck);
    }
    if config.enable_hold_repeat {
        effects.push(ButtonEffect::StartHoldRepeat);
    }
    effects
}

/// `CustomButton.OnPointerUp`. When control finishes, the stored finish
/// callback runs (see [`finish_control`]); then this button waits for the
/// click. Every release clears the long-press flag, cancels the long-press
/// check and stops hold-repeat.
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
        finish_control(manager, state, button, &mut effects);
        state.control = ControlState::ClickCheck;
    }
    state.executed_long_press = false;
    effects.push(ButtonEffect::CancelLongPressCheck);
    effects.push(ButtonEffect::StopHoldRepeat);
    effects
}

/// `CustomSelectableDefine.CheckPointerClickAction`: true when not blocked.
pub fn check_pointer_click_action(
    manager: &mut InputManager,
    config: &CustomButtonConfig,
    clock: &mut impl FnMut() -> f32,
    live: ButtonLive,
    effects: &mut Vec<ButtonEffect>,
) -> bool {
    let blocked = manager.check_and_reset_interval_time(config.interval, clock);
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
    clock: &mut impl FnMut() -> f32,
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
        if check_pointer_click_action(manager, config, clock, live, &mut effects) {
            engine_button_click(pointer, live, &mut effects);
        }
        return effects;
    }
    if !state.executed_long_press && check_pointer_click_action(manager, config, clock, live, &mut effects) {
        engine_button_click(pointer, live, &mut effects);
    }
    state.executed_long_press = false;
    effects.push(ButtonEffect::CancelLongPressCheck);
    effects
}

/// `CustomButton.OnPointerExit` (after the engine selectable's exit). An
/// exit of the pressed pointer releases its registration and finishes
/// control; every exit stops hold-repeat.
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
        finish_control(manager, state, button, &mut effects);
        if config.enable_long_press {
            state.executed_long_press = false;
            effects.push(ButtonEffect::CancelLongPressCheck);
        }
    }
    effects.push(ButtonEffect::StopHoldRepeat);
    effects
}

/// `CustomButton.OnDisable` (after the engine selectable's disable, the
/// cover and the view interaction's disabled hook, none of which this port
/// draws): a button disabled while pressed finishes control, which runs the
/// stored finish callback (the release, when it is this button's).
pub fn on_disable(manager: &mut InputManager, state: &mut CustomButtonState, button: u64) -> Vec<ButtonEffect> {
    let mut effects = Vec::new();
    if state.control == ControlState::Press {
        finish_control(manager, state, button, &mut effects);
    }
    effects
}

#[cfg(test)]
mod source_compare {
    use super::*;

    /// Runs the interval gate on cases given as text lines
    /// `interval start read1 read2` (interval 0 or 1, the others f32 bits in
    /// hex) and writes `blocked stored reads` per line (stored as f32 bits in
    /// hex, reads the number of clock reads), for comparison with the source
    /// method executed on the same cases.
    #[test]
    #[ignore = "research instrument: needs MOLY_INTERVAL_COMPARE_IN and MOLY_INTERVAL_COMPARE_OUT"]
    fn interval_gate_for_source_comparison() {
        let input = std::env::var("MOLY_INTERVAL_COMPARE_IN").expect("MOLY_INTERVAL_COMPARE_IN");
        let output = std::env::var("MOLY_INTERVAL_COMPARE_OUT").expect("MOLY_INTERVAL_COMPARE_OUT");
        let text = std::fs::read_to_string(&input).expect("case file");
        let mut out = String::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let words: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(words.len(), 4, "case line: {line}");
            let interval = IntervalUseType::from_serialized(words[0].parse().expect("interval")).expect("interval type");
            let bits = |w: &str| u32::from_str_radix(w.trim_start_matches("0x"), 16).expect("hex bits");
            let mut manager = InputManager { interval_start_time: f32::from_bits(bits(words[1])), ..Default::default() };
            let reads = [f32::from_bits(bits(words[2])), f32::from_bits(bits(words[3]))];
            let mut count = 0usize;
            let mut clock = || {
                let value = reads[count];
                count += 1;
                value
            };
            let blocked = manager.check_and_reset_interval_time(interval, &mut clock);
            out.push_str(&format!("{} {:#010x} {}\n", blocked, manager.interval_start_time.to_bits(), count));
        }
        std::fs::write(&output, out).expect("write report");
    }
}
