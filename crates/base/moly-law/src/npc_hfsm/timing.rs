//! The NPC machine's constants: one controller timer, the transition budget
//! and the durations the state bodies compare against.
//!
//! - [`MAX_TRANSITIONS_PER_FRAME`]: the third upper transition a frame is
//!   refused and the character parks Inactive.
//! - [`INACTIVE_RECOVERY_SECONDS`]: an Inactive character decides again
//!   after this many seconds (only after a start, and only when its
//!   recovery is not suppressed).
//! - [`REST_AFTER_UPPER_SECONDS`]: the rest the presenter builds from a
//!   `Rest` upper state (the controller's own rest is `PauseTime`).
//! - [`IDLE_DURATION_MIN`] / [`IDLE_DURATION_MAX`]: the fallback idle's
//!   pause range; the fallback draws its pause when it is built.
//! - [`TWEET_SECONDS`], [`NO_TALK_WAIT_SECONDS`], [`GREETING_*`],
//!   [`ENTRY_STANDBY_SECONDS`], [`GATHERING_*`], [`RECRUIT_RETRY_*`],
//!   [`COMMUNICATION_WHILE_DOING_WAIT_TIME_KEY`] and
//!   [`SUB_COMMUNICATION_CARRY_GRACE_SECONDS`]: the lower states' own
//!   durations and the one config key among them.
//! - [`ACTION_POINT_ARRIVAL_STOPPING_DISTANCE`], [`ALREADY_ARRIVED_DISTANCE`],
//!   [`NAVMESH_WAIT_TIMEOUT_SECONDS`], [`RANDOM_WALK_*`] and [`AVOID_*`]: the
//!   upper states' mover constants.

/// The upper transitions one frame may make before the character parks.
pub const MAX_TRANSITIONS_PER_FRAME: i32 = 2;

/// How long an Inactive character waits before deciding again.
pub const INACTIVE_RECOVERY_SECONDS: f32 = 3.0;

/// The rest the presenter builds from a `Rest` upper state.
pub const REST_AFTER_UPPER_SECONDS: f32 = 5.0;

/// The fallback idle's shortest pause.
pub const IDLE_DURATION_MIN: f32 = 5.0;
/// The fallback idle's longest pause.
pub const IDLE_DURATION_MAX: f32 = 8.0;

/// How long a tweet stays up.
pub const TWEET_SECONDS: f32 = 5.0;

/// The general talk's wait when it finds no talk.
pub const NO_TALK_WAIT_SECONDS: f32 = 2.0;

/// The greeting's hold.
pub const GREETING_SECONDS: f32 = 5.0;
/// The greeting's tweet show time.
pub const GREETING_TWEET_SECONDS: f32 = 3.0;
/// The greeting's look at the player.
pub const GREETING_LOOK_SECONDS: f32 = 3.0;
/// The distance past which a greeting ends at once.
pub const GREETING_DISTANCE: f32 = 5.0;

/// The hidden entry standby's self-release.
pub const ENTRY_STANDBY_SECONDS: f32 = 30.0;

/// The multi-member gathering deadline.
pub const GATHERING_DEADLINE_SECONDS: f32 = 30.0;
/// The multi-member gathering's no-progress timeout.
pub const GATHERING_STALL_SECONDS: f32 = 15.0;
/// The recruit retry's grace after the gathering starts.
pub const RECRUIT_RETRY_GRACE_SECONDS: f32 = 10.0;
/// The recruit retry interval.
pub const RECRUIT_RETRY_INTERVAL_SECONDS: f32 = 1.0;
/// The while-doing-wait state's gathering arrival distance.
pub const GATHERING_ARRIVAL_DISTANCE: f32 = 0.25;
/// The final approach's speed.
pub const FINAL_APPROACH_SPEED: f32 = 0.3;
/// The distance below which a character looks at the group's centre.
pub const CENTER_LOOK_MIN_DISTANCE: f32 = 0.05;

/// The config key of the while-doing-wait tweet window (milliseconds).
pub const COMMUNICATION_WHILE_DOING_WAIT_TIME_KEY: i32 = 107;

/// How long a sub waits for its main to complete.
pub const SUB_COMMUNICATION_CARRY_GRACE_SECONDS: f32 = 1.0;

/// The fixture action's exact action-point arrival distance.
pub const ACTION_POINT_ARRIVAL_STOPPING_DISTANCE: f32 = 0.01;
/// `ALREADY_ARRIVED_DISTANCE`: added to the stopping distance when a fixture
/// or position move accepts its arrival.
pub const ALREADY_ARRIVED_DISTANCE: f32 = 0.3;
/// The navmesh wait before a move is abandoned.
pub const NAVMESH_WAIT_TIMEOUT_SECONDS: f32 = 5.0;
/// The random walk's radius.
pub const RANDOM_WALK_RADIUS: f32 = 5.0;
/// The random walk's attempts.
pub const RANDOM_WALK_ATTEMPTS: usize = 10;
/// The random walk's avoid attempts (the attempts past these walk plainly).
pub const RANDOM_WALK_AVOID_ATTEMPTS: usize = 5;
/// The avoid direction's near band: squared distances in
/// `(AVOID_NEAR_MIN², AVOID_NEAR_MAX²)`.
pub const AVOID_NEAR_MIN: f32 = 0.031_622_776;
/// The avoid direction's far edge.
pub const AVOID_NEAR_MAX: f32 = 1.5;

/// The action state that makes the controller skip the frame: a character
/// in the talk state does not advance its own machine.
pub const ACTION_STATE_TALK: i32 = 4;
/// The action state the lower states report as idle.
pub const ACTION_STATE_IDLE: i32 = 0;
/// The action state the rest lower reports.
pub const ACTION_STATE_REST: i32 = 7;
/// The action state the tweet phases report.
pub const ACTION_STATE_TWEET: i32 = 9;
/// The action state the greeting lower reports.
pub const ACTION_STATE_GREETING: i32 = 10;
/// The action state the fixture-action timelines report.
pub const ACTION_STATE_FIXTURE_ACTION: i32 = 11;
/// The action state the cut-scene lower reports.
pub const ACTION_STATE_CUT_SCENE: i32 = 13;
/// The action state the change-site lower reports.
pub const ACTION_STATE_CHANGE_SITE: i32 = 14;
/// The action state the photo-shot lower reports.
pub const ACTION_STATE_PHOTO_SHOT: i32 = 15;
/// The action state the main communication reports.
pub const ACTION_STATE_COMMUNICATION: i32 = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_constants_are_the_source_fields() {
        // MAX_TRANSITIONS_PER_FRAME = 2, INACTIVE_RECOVERY_SECONDS = 3.0,
        // the idle range 5–8, the presenter's rest 5 s.
        assert_eq!(MAX_TRANSITIONS_PER_FRAME, 2);
        assert_eq!(INACTIVE_RECOVERY_SECONDS, 3.0);
        assert_eq!(IDLE_DURATION_MIN, 5.0);
        assert_eq!(IDLE_DURATION_MAX, 8.0);
        assert_eq!(REST_AFTER_UPPER_SECONDS, 5.0);
    }

    #[test]
    fn talk_action_state_is_four() {
        // OnUpdate tests CurrentStateType == 4 before any phase body.
        assert_eq!(ACTION_STATE_TALK, 4);
    }

    #[test]
    fn avoid_band_is_the_source_square_band() {
        // GetAvoidDirection keeps 0.001 < d² < 2.25.
        assert!((AVOID_NEAR_MIN * AVOID_NEAR_MIN - 0.001).abs() < 1e-7);
        assert_eq!(AVOID_NEAR_MAX * AVOID_NEAR_MAX, 2.25);
    }
}
