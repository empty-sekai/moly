//! The player view's record (`MysekaiBGMRecordPlayer`) and the record
//! animation it instantiates (`fx_mysekai_bgm_select_record`).
//!
//! The source loads the animation prefab from the player view's package,
//! adds `MysekaiBGMRecordPlayer` to it and parents it below the player view's
//! `_recordAnimationTransform`, then drives it through its `Animator`'s
//! states: `None`, `SlideIn`, `SlideOut`, `PlayRecord`, `StopRecord` and
//! `LoopRecord`. Each state request crossfades in fixed time and cancels the
//! request before it. The prefab's clips and their events are not in the
//! public asset root, so the record's motion is this port's own, in the
//! states' order and with their events:
//! - `PlayRecordAsync` (from `None`), which `SetupBGMPlayerView` starts when
//!   the player view is set up with a record: the record slides in and
//!   turns, then the turntable loops its turn; the animation's `play` event,
//!   which `MysekaiBGMRecordPlayer.OnCallback` answers, starts the music;
//! - `PlayResumeAsync` / `SetPlaying` / `LoopRecord`: the turn loops;
//! - `PlayPauseAsync` / `StopRecordAsync`: the turn comes to rest.
//!
//! Named stand-ins: the duration of each part and the turn's speeds (the
//! prefab's clips carry the real ones). The events and the state order are
//! the source's.

use bevy::prelude::*;

use super::model::Presenter;
use super::UiPrefabView;

/// The animation prefab's name in the player view's package
/// (`MysekaiBGMPlayerView.RECORD_ANIMATION_NAME`).
const RECORD_ANIMATION_NAME: &str = "fx_mysekai_bgm_select_record";
/// How long the slide-in runs before the turn loops (this port's value: the
/// prefab's clips are not in the public asset root).
const SLIDE_IN_SECONDS: f32 = 0.4;
/// The record's turn while it plays, degrees per second (this port's value),
/// and how fast it starts and stops (degrees per second, per second).
const TURN_DEGREES_PER_SECOND: f32 = 90.;
const TURN_RAMP_DEGREES_PER_SECOND: f32 = 180.;

/// `MysekaiBGMRecordPlayer.StateNameType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    None,
    SlideIn,
    SlideOut,
    PlayRecord,
    StopRecord,
    LoopRecord,
}

impl State {
    /// The animation's state name, as `Enum.ToString` spells it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            State::None => "None",
            State::SlideIn => "SlideIn",
            State::SlideOut => "SlideOut",
            State::PlayRecord => "PlayRecord",
            State::StopRecord => "StopRecord",
            State::LoopRecord => "LoopRecord",
        }
    }
}

/// What the record player did on one frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Events {
    /// The animation's `play` event (`OnCallback`: the music starts).
    pub(crate) played: bool,
    /// The animation's `loop` event (the turn passed its first full turn).
    pub(crate) looped: bool,
}

/// The record's motion in the player view.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Player {
    state: Option<State>,
    /// The current state's time.
    time: f32,
    /// The turn's angle and speed (degrees per second), whether it loops,
    /// and whether a turn has passed (the animation's `loop` event).
    angle: f32,
    turn: f32,
    looping: bool,
    turned: bool,
}

impl Player {
    /// The state the animation is in.
    pub(crate) fn state(&self) -> Option<State> {
        self.state
    }

    fn enter(&mut self, state: State) -> bool {
        if self.state == Some(state) {
            return false;
        }
        self.state = Some(state);
        self.time = 0.;
        true
    }

    /// `PlayRecordAsync`: the slide-in, then the turn loops.
    fn play_record(&mut self) -> bool {
        self.looping = false;
        self.turned = false;
        self.enter(State::PlayRecord)
    }

    /// `PlayResumeAsync` / `SetPlaying` / `LoopRecord`.
    fn resume(&mut self) -> bool {
        self.looping = true;
        self.enter(State::LoopRecord)
    }

    /// `PlayPauseAsync` / `StopRecordAsync`.
    fn stop(&mut self) -> bool {
        self.looping = false;
        if self.state.is_none() {
            return false;
        }
        self.enter(State::StopRecord)
    }
}

/// One frame of the record player: the state the record's playback asks for
/// (`PlayRecordAsync` when a record starts playing, `PlayPauseAsync` when it
/// stops), the slide-in handing over to the turn, and the turn itself.
/// Returns whether a state changed (for the log) and the events.
pub(crate) fn frame(player: &mut Player, playing: bool, record: Option<i32>, dt: f32) -> (bool, Events) {
    let mut events = Events::default();
    if record.is_none() {
        player.state = None;
        player.looping = false;
        return (false, events);
    }
    let before = player.state;
    if playing && !matches!(before, Some(State::PlayRecord) | Some(State::LoopRecord)) {
        player.play_record();
    } else if !playing && matches!(before, Some(State::PlayRecord) | Some(State::LoopRecord)) {
        player.stop();
    }
    player.time += dt;
    if player.state == Some(State::PlayRecord) && player.time >= SLIDE_IN_SECONDS {
        player.resume();
        events.played = true;
    }
    if player.state == Some(State::StopRecord) && player.time >= SLIDE_IN_SECONDS {
        player.state = Some(State::None);
    }
    if player.looping {
        if player.turn < TURN_DEGREES_PER_SECOND {
            player.turn = (player.turn + TURN_RAMP_DEGREES_PER_SECOND * dt)
                .min(TURN_DEGREES_PER_SECOND);
        }
    } else {
        player.turn = (player.turn - TURN_RAMP_DEGREES_PER_SECOND * dt).max(0.);
    }
    let was = player.angle;
    player.angle = (player.angle + player.turn * dt).rem_euclid(360.);
    if player.angle < was && !player.turned {
        player.turned = true;
        events.looped = true;
    }
    (before != player.state, events)
}

/// One frame of the record player, written into the view: the record turns
/// about its rect's centre while the turntable runs. `playing` is the player
/// view's record playing (`PlaceSelectBgm` sets it; `isSettingMusic` reads
/// it).
pub(crate) fn animate(
    presenter: &Presenter,
    player: &mut Player,
    view: &mut UiPrefabView,
    node: &str,
    dt: f32,
) -> Events {
    let (changed, events) = frame(player, presenter.is_setting_music(), presenter.select, dt);
    if changed {
        info!(
            "[bgm-select] record animation: {RECORD_ANIMATION_NAME} animator enters {}",
            player.state().map_or(State::None.name(), State::name)
        );
    }
    view.set_rotation(node, player.angle);
    events
}

/// `MysekaiBGMRecordPlayer.OnCallback`: the animation's `play` and `loop`
/// events name the callback it calls.
pub(crate) fn callback(event: &str) -> String {
    format!(
        "{RECORD_ANIMATION_NAME}: {event} event -> MysekaiBGMRecordPlayer.OnCallback(\"{event}\")"
    )
}
