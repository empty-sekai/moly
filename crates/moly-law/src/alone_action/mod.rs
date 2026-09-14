//! Source-ordered Rest scripts and their presentation commands.
//!
//! ScriptState owns a single invocation: independent conditional blocks or an
//! exclusive random branch, wall-second guards, cached loop time, delayed slot
//! writes, and an unconditional tail. The host creates and drops it on actual
//! Rest entry/exit. The nominal step helpers remain available for inspecting
//! authored sequences; they do not select or schedule a second script.
pub mod row;
pub mod select;
pub mod step;

pub use row::{Program, ProgramBlock, ProgramKind};
pub use row::{RandomBranchTrigger, Scenario, SegmentSuffix, Step, TimeGatedTrigger, Trigger};
pub use select::{PercentDraw, ScriptState};
pub use step::{
    advance, effective_speed, is_finished, op_executable, schedule, span, wait_milliseconds,
    Rejection, ScheduledStep, SequenceState, StepEvent, EXCLUDED_GAME_STATE_TYPE,
    OP_STATE_TYPE_IDLE, TIME_EPSILON,
};
