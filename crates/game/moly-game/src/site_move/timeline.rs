//! Source values of the cannon move (`MoveSiteUseCannonActionState`) as pure
//! functions: the five timing fields, the wait each step hands to
//! `UniTask.Delay`, the landing draw, the DOTween eases the move uses, and the
//! frame clocks the executor steps with. Nothing here touches the ECS.
//!
//! Where each value comes from, in words:
//! - The Core state machine stores its five floats with one wide store of four
//!   single-precision words followed by one scalar store; the words are kept
//!   bit-exact below (the decompiled C# shows zeros for them, a known artefact
//!   of that wide store, and must not be copied).
//! - Every wait is `UniTask.Delay(TimeSpan.FromSeconds(x))`. `FromSeconds`
//!   goes through `TimeSpan.Interval(value, 1000)`, which rounds to whole
//!   milliseconds (away from zero) before scaling to ticks, and the delay
//!   keeps `(float)TotalSeconds`. So the waits are millisecond-rounded:
//!   0.6666667 becomes 0.667, 0.4833333 becomes 0.483, 0.8166667 becomes
//!   0.817 and so on. [`delay_seconds`] reproduces that chain.
//! - The move time handed to the player's `DOMove` is computed in single
//!   precision as `((out + fly) + landing) + (-1.0)`, in that order.
//! - The landing draw is one `Random.Range(1, 101)` whose result is compared
//!   twice (`r < 4` for the failure flag, `r > 3` for the clip), so the flag
//!   and the clip can never disagree: failure is exactly 3 of 100 values.

/// Frames the source tween and delay clocks run on. A `UniTask.Delay` never
/// advances on the frame that creates it; each later frame adds that frame's
/// `Time.deltaTime` as a float and completes on the first frame whose sum
/// reaches the target (`elapsed >= delay`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Delay {
    pub(crate) target: f32,
    pub(crate) elapsed: f32,
    pub(crate) created_frame: u64,
}

impl Delay {
    pub(crate) fn new(target: f32, frame: u64) -> Self {
        Self {
            target,
            elapsed: 0.0,
            created_frame: frame,
        }
    }

    /// Advance on one frame; true once the delay is due. The creation frame
    /// is skipped, as `DelayPromise.MoveNext` skips it while nothing has
    /// elapsed yet.
    pub(crate) fn tick(&mut self, frame: u64, dt: f32) -> bool {
        if frame == self.created_frame {
            return false;
        }
        self.elapsed += dt;
        self.elapsed >= self.target
    }
}

/// A DOTween clock (player `DOMove`, the environment follow, the cannon's
/// `DOScale`). The product convention for tweens (see `camera::CameraTween`,
/// advanced in the frame it is inserted) is kept: the creating frame's delta
/// counts. DOTween's own first-update timing was not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TweenClock {
    pub(crate) duration: f32,
    pub(crate) elapsed: f32,
}

impl TweenClock {
    pub(crate) fn new(duration: f32) -> Self {
        Self {
            duration,
            elapsed: 0.0,
        }
    }

    /// Advance and return the normalised time in [0, 1].
    pub(crate) fn advance(&mut self, dt: f32) -> f32 {
        self.elapsed += dt;
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    pub(crate) fn done(&self) -> bool {
        self.elapsed >= self.duration
    }
}

/// `inCannonTime`: the `_s` clip is 40 frames at 60 fps.
pub(crate) const IN_CANNON_TIME: f32 = f32::from_bits(0x3f2a_aaab);
/// `outCannonTime`: the `_s2` clip is 35 frames at 60 fps.
pub(crate) const OUT_CANNON_TIME: f32 = f32::from_bits(0x3f15_5555);
/// `readyTime`: 6 frames.
pub(crate) const READY_TIME: f32 = f32::from_bits(0x3dcc_cccd);
/// `flyTime`: 10 frames.
pub(crate) const FLY_TIME: f32 = f32::from_bits(0x3e2a_aaab);
/// `landingTime`: 39 frames.
pub(crate) const LANDING_TIME: f32 = f32::from_bits(0x3f26_6666);
/// The first wait is a single-precision 0.2 widened to double.
pub(crate) const FIRST_DELAY: f32 = 0.2;
/// `OnSiteMoveEndAction` waits a double 0.5 before the harvest summary when
/// the destination is the home site.
pub(crate) const HOME_SUMMARY_DELAY: f64 = 0.5;
/// The environment cross-fade of this move comes from the graphics config
/// (`_siteTransitionData._crossFadeDuration`), carried by the weather module.
pub(crate) const ENVIRONMENT_FADE: f32 = crate::weather_transition::CANNON_SITE_FADE_SECONDS;
/// `SiteMoveCannon.End`: `DOScale(Vector3.zero, 0.5f)`.
pub(crate) const CANNON_SHRINK_TIME: f32 = 0.5;
/// Landing `ChangeAnimation` passes a 0.25 fade; the three flight clips 0.
pub(crate) const LANDING_FADE: f32 = 0.25;
/// `Random.Range(1, 101)` bounds of the landing draw.
pub(crate) const LANDING_RANGE: (i32, i32) = (1, 101);
/// The draw fails when it is below this value (`r < 4`).
pub(crate) const LANDING_FAILURE_BELOW: i32 = 4;
/// The fire clip's `PlayEffect` event (frame 50 of the cannon clip).
pub(crate) const CANNON_PLAY_EFFECT_AT: f32 = 0.833_333_3;
/// The cannon clip's three `m_IsActive` curves: each group is inactive at 0
/// and active from its key on (frames 1, 24 and 47).
pub(crate) const CANNON_GROUP_ACTIVATION: [(&str, f32); 3] = [
    ("root/joint_canon1/fx_act_cannon_appear", 0.016_666_67),
    (
        "root/joint_canon1/joint_canon2/joint_canon3/fx_act_user_boarding",
        0.400_000_006,
    ),
    (
        "root/joint_canon1/joint_canon2/joint_canon3/fx_act_user_takeoff",
        0.783_333_36,
    ),
];

/// `TimeSpan.FromSeconds(x)` as the delay sees it: `Interval(x, 1000)` turns
/// `x * 1000` into whole milliseconds, rounding half away from zero, scales
/// them to 100 ns ticks, and `TotalSeconds` multiplies the ticks by 1e-7 in
/// double; `DelayPromise` stores that as a float.
pub(crate) fn delay_seconds(x: f64) -> f32 {
    let scaled = x * 1000.0;
    let millis = (scaled + if scaled >= 0.0 { 0.5 } else { -0.5 }).trunc() as i64;
    let ticks = millis * 10_000;
    (ticks as f64 * 1e-7) as f32
}

/// The move time handed to `DOMove` and to the environment follow.
pub(crate) fn move_time() -> f32 {
    ((OUT_CANNON_TIME + FLY_TIME) + LANDING_TIME) + (-1.0)
}

/// The waits of `OnMoveActionCore`, in order, as the delays hold them.
pub(crate) struct CoreWaits {
    pub(crate) first: f32,
    pub(crate) in_cannon: f32,
    pub(crate) ready: f32,
    pub(crate) out_minus_ready: f32,
    pub(crate) fly: f32,
    pub(crate) landing: f32,
}

pub(crate) fn core_waits() -> CoreWaits {
    CoreWaits {
        first: delay_seconds(FIRST_DELAY as f64),
        in_cannon: delay_seconds(IN_CANNON_TIME as f64),
        ready: delay_seconds(READY_TIME as f64),
        // Single-precision subtraction first, then the delay's rounding.
        out_minus_ready: delay_seconds((OUT_CANNON_TIME - READY_TIME) as f64),
        fly: delay_seconds(FLY_TIME as f64),
        landing: delay_seconds(LANDING_TIME as f64),
    }
}

/// The last Core wait: `GetAnimationTime(name) - landingTime`, subtracted in
/// single precision, then handed to the delay.
pub(crate) fn final_wait(animation_length: f32) -> f32 {
    delay_seconds((animation_length - LANDING_TIME) as f64)
}

/// The player clips the Core requests by name. The names are the literal
/// strings the source passes; the lengths are the shipped clips' lengths
/// (`GetAnimationTime` returns the clip length). The camera keys its
/// per-frame branches on these names, never on the product's SD stand-ins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceClip {
    CannonS,
    CannonS2,
    CannonL,
    CannonE,
    Cannon02E,
}

impl SourceClip {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::CannonS => "mov_u000_site_cannon01_s",
            Self::CannonS2 => "mov_u000_site_cannon01_s2",
            Self::CannonL => "mov_u000_site_cannon01_l",
            Self::CannonE => "mov_u000_site_cannon01_e",
            Self::Cannon02E => "mov_u000_site_cannon02_e",
        }
    }

    /// Shipped clip length in seconds (single precision, as stored).
    pub(crate) fn length(self) -> f32 {
        match self {
            Self::CannonS => 0.666_666_7,
            Self::CannonS2 => 0.583_333_4,
            Self::CannonL => 0.25,
            Self::CannonE => 1.466_666_7,
            Self::Cannon02E => 2.800_000_2,
        }
    }

    /// `ChangeAnimation` fade time the Core passes with this clip.
    pub(crate) fn fade(self) -> f32 {
        match self {
            Self::CannonE | Self::Cannon02E => LANDING_FADE,
            _ => 0.0,
        }
    }

    /// `PublishFlyingEffectPlay` on `_s2` at frame 7.
    pub(crate) fn flying_play_event(self) -> Option<f32> {
        (self == Self::CannonS2).then_some(0.116_666_67)
    }

    /// `PublishFlyingEffectEnd` at 0 on both landing clips.
    pub(crate) fn flying_end_event(self) -> bool {
        matches!(self, Self::CannonE | Self::Cannon02E)
    }
}

/// Outcome of the one landing draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Landing {
    pub(crate) roll: i32,
    pub(crate) failure: bool,
    pub(crate) clip: SourceClip,
}

/// Both reads of the same draw: the flag (`r < 4`) and the clip (`r > 3`).
pub(crate) fn landing(roll: i32) -> Landing {
    assert!(
        (LANDING_RANGE.0..LANDING_RANGE.1).contains(&roll),
        "landing draw {roll} outside Random.Range(1, 101)"
    );
    Landing {
        roll,
        failure: roll < LANDING_FAILURE_BELOW,
        clip: if roll > 3 {
            SourceClip::CannonE
        } else {
            SourceClip::Cannon02E
        },
    }
}

/// `Random.Range(int min, int max)` maps one 32-bit draw as
/// `min + draw % (max - min)` when `min < max`.
pub(crate) fn range_int(word: u32, min: i32, max: i32) -> i32 {
    assert!(min < max, "Random.Range needs min < max");
    let span = (max - min) as u32;
    min + (word % span) as i32
}

/// DOTween `OutSine` (ease 3, the player's flight): `sin(t / d * pi/2)`,
/// t already normalised.
pub(crate) fn out_sine(t: f32) -> f32 {
    (t * std::f32::consts::FRAC_PI_2).sin()
}

/// DOTween `InBack` (ease 26, the cannon's shrink) with the default
/// overshoot 1.70158:
/// `t * t * ((s + 1) * t - s)`.
pub(crate) fn in_back(t: f32) -> f32 {
    const S: f32 = 1.701_58;
    t * t * ((S + 1.0) * t - S)
}

/// DOTween `OutQuad` (ease 6, the camera's entry tween, advanced by
/// `camera::CameraTween` as `1 - (1 - t)^2`, the same polynomial):
/// `-t * (t - 2)`.
#[cfg(test)]
pub(crate) fn out_quad(t: f32) -> f32 {
    -t * (t - 2.0)
}

/// One step of the ordered Core timeline, for the step log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Setup,
    CreateCannon,
    CannonStart,
    Clip2,
    FlyingStart,
    Fire,
    CannonPlayEffect,
    PlayerArrive,
    EnvironmentFollowEnd,
    ChangeEnvironment,
    Reveal,
    CannonDestroyed,
    LandingEffect,
    ExitStateGateOpen,
    CoreEnd,
    EndAction,
    Normal,
}

impl Step {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::CreateCannon => "create-cannon",
            Self::CannonStart => "cannon-start+_s",
            Self::Clip2 => "_s2",
            Self::FlyingStart => "flying-effect-start",
            Self::Fire => "fire",
            Self::CannonPlayEffect => "cannon-PlayEffect",
            Self::PlayerArrive => "player-tween-end(swap)",
            Self::EnvironmentFollowEnd => "environment-follow-end",
            Self::ChangeEnvironment => "change-environment+bgm+_l",
            Self::Reveal => "reveal+cannon-end+landing",
            Self::CannonDestroyed => "cannon-destroyed",
            Self::LandingEffect => "landing-effect+se",
            Self::ExitStateGateOpen => "exit-state-gate-open",
            Self::CoreEnd => "core-end",
            Self::EndAction => "end-action",
            Self::Normal => "normal",
        }
    }
}

/// The source time of a step relative to T0 (the frame the cannon starts,
/// after the 0.2 s wait and the cannon load), for the step table. `None`
/// for steps timed from the move start or the Core end instead.
pub(crate) fn source_offset_from_t0(step: Step, landing: Option<Landing>) -> Option<f64> {
    let w = core_waits();
    let fire = w.in_cannon as f64 + w.ready as f64;
    let change = fire + w.out_minus_ready as f64;
    let reveal = change + w.fly as f64;
    let land = reveal + w.landing as f64;
    Some(match step {
        Step::CannonStart => 0.0,
        Step::Clip2 => w.in_cannon as f64,
        Step::FlyingStart => {
            w.in_cannon as f64 + SourceClip::CannonS2.flying_play_event().unwrap() as f64
        }
        Step::Fire => fire,
        Step::CannonPlayEffect => CANNON_PLAY_EFFECT_AT as f64,
        Step::PlayerArrive | Step::EnvironmentFollowEnd => fire + move_time() as f64,
        Step::ChangeEnvironment => change,
        Step::Reveal => reveal,
        Step::CannonDestroyed => reveal + CANNON_SHRINK_TIME as f64,
        Step::LandingEffect => land,
        Step::ExitStateGateOpen => land + 1.0,
        Step::CoreEnd => land + final_wait(landing?.clip.length()) as f64,
        Step::Setup | Step::CreateCannon | Step::EndAction | Step::Normal => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Research instrument: every expected value below is recomputed from the
    //! source's own constants and formulas, so changing any of them in the
    //! source reading (the words, the rounding chain, the draw comparisons)
    //! turns these red.
    use super::*;

    #[test]
    fn core_fields_are_the_stored_words() {
        assert_eq!(IN_CANNON_TIME, 40.0_f32 / 60.0);
        assert_eq!(OUT_CANNON_TIME, 35.0_f32 / 60.0);
        assert_eq!(READY_TIME, 0.1_f32);
        assert_eq!(FLY_TIME, 10.0_f32 / 60.0);
        assert_eq!(LANDING_TIME, 0.65_f32);
        // The flight time: f32 sum in the source's order.
        let expected = ((35.0_f32 / 60.0 + 10.0_f32 / 60.0) + 0.65_f32) - 1.0;
        assert_eq!(move_time(), expected);
        assert!((move_time() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn waits_are_millisecond_rounded() {
        let w = core_waits();
        assert_eq!(w.first, 0.2_f32);
        assert_eq!(w.in_cannon, 0.667_f32);
        assert_eq!(w.ready, 0.1_f32);
        assert_eq!(w.out_minus_ready, 0.483_f32);
        assert_eq!(w.fly, 0.167_f32);
        assert_eq!(w.landing, 0.65_f32);
        assert_eq!(final_wait(SourceClip::CannonE.length()), 0.817_f32);
        assert_eq!(final_wait(SourceClip::Cannon02E.length()), 2.15_f32);
        assert_eq!(delay_seconds(HOME_SUMMARY_DELAY), 0.5_f32);
        // Rounding is half away from zero on the millisecond.
        assert_eq!(delay_seconds(0.0005), 0.001_f32);
        assert_eq!(delay_seconds(-0.0005), -0.001_f32);
        assert_eq!(delay_seconds(0.000_499), 0.0);
    }

    #[test]
    fn landing_draw_over_every_value() {
        let mut failures = 0;
        for roll in LANDING_RANGE.0..LANDING_RANGE.1 {
            let l = landing(roll);
            // The flag and the clip are two reads of one draw.
            assert_eq!(l.failure, l.clip == SourceClip::Cannon02E, "roll {roll}");
            failures += usize::from(l.failure);
        }
        assert_eq!(failures, 3);
        assert!(landing(1).failure && landing(3).failure && !landing(4).failure);
        assert!(!landing(100).failure);
        // Random.Range(1, 101) covers 1..=100 through `min + word % 100`.
        assert_eq!(range_int(0, 1, 101), 1);
        assert_eq!(range_int(99, 1, 101), 100);
        assert_eq!(range_int(100, 1, 101), 1);
        assert_eq!(range_int(u32::MAX, 1, 101), 1 + (u32::MAX % 100) as i32);
    }

    #[test]
    fn eases_match_dotween_closed_forms() {
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            let sine = (t * std::f64::consts::FRAC_PI_2).sin();
            let back = t * t * ((1.70158 + 1.0) * t - 1.70158);
            let quad = -t * (t - 2.0);
            assert!(
                (out_sine(t as f32) as f64 - sine).abs() < 1e-6,
                "OutSine {t}"
            );
            assert!((in_back(t as f32) as f64 - back).abs() < 1e-6, "InBack {t}");
            assert!(
                (out_quad(t as f32) as f64 - quad).abs() < 1e-6,
                "OutQuad {t}"
            );
        }
        // InBack undershoots below zero before rising to one.
        assert!(in_back(0.5) < 0.0);
        assert_eq!(in_back(1.0), 1.0);
    }

    #[test]
    fn delay_skips_its_creation_frame() {
        let mut delay = Delay::new(0.1, 7);
        assert!(!delay.tick(7, 0.05));
        assert!(!delay.tick(8, 0.05));
        assert!(delay.tick(9, 0.05));
    }

    #[test]
    fn step_offsets_follow_the_waits() {
        let s = |step| source_offset_from_t0(step, Some(landing(50))).unwrap();
        assert!((s(Step::Clip2) - 0.667).abs() < 1e-6);
        assert!((s(Step::Fire) - 0.767).abs() < 1e-6);
        assert!((s(Step::ChangeEnvironment) - 1.25).abs() < 1e-6);
        assert!((s(Step::Reveal) - 1.417).abs() < 1e-6);
        assert!((s(Step::LandingEffect) - 2.067).abs() < 1e-6);
        assert!((s(Step::CoreEnd) - 2.884).abs() < 1e-6);
        let failed = source_offset_from_t0(Step::CoreEnd, Some(landing(2))).unwrap();
        assert!((failed - 4.217).abs() < 1e-6);
        assert!((s(Step::FlyingStart) - (0.667 + 0.116_666_67)).abs() < 1e-6);
    }
}
