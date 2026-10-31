//! Walking rule: recomputes "path corners + speed + dt" frame by frame into a
//! position and a heading, one NPC at a time. `dt` is an explicit parameter —
//! the "smoothness" of frame-to-frame movement is decided entirely by it.
//!
//! The move loop does not integrate the position itself: it configures a
//! navigation agent (`SetupNavMeshAgent`: radius, height, stopping distance,
//! speed and so on), starts it (`SetupMoveNavMeshParameter`: clears
//! `isStopped`, writes the acceleration and the selected speed), hands it a
//! destination (`SetDestinationIfMoving`), and from then on the agent's own
//! implementation advances every frame — `velocity`, `steeringTarget` and
//! `remainingDistance` are all extern bindings without a function body. That
//! opaque boundary is not re-derived here; what this module recomputes is the
//! surrounding contract:
//!
//! * The start writes the selected speed into the agent's `speed`; `speed`
//!   means the **maximum** movement speed, and `remainingDistance` measures
//!   along the current path. The strongest statement available is "along the
//!   corner polyline, at most `speed` per second"; this module walks at
//!   exactly `speed`.
//! * Arrival is the move loop's own per-frame check: once the straight-line
//!   distance to the move target is below [`ARRIVAL_DISTANCE`] the loop exits.
//!   The check runs at the start of the frame: a walker that crosses into the
//!   band mid-frame still walks that frame and reports arrival on the next
//!   one. Inside the band it holds still — this implementation does not move
//!   the walker further within the band, and the residual stays inside the
//!   band by construction.
//! * The walking heading follows `LookAtNavMeshCorner`: read one fixed
//!   position in the corner table (the entry after the walker's own entry;
//!   as a [`NpcPathWalkSlot`] row that is corner 0, and the index never
//!   advances) and normalize the current position toward it; when that corner
//!   is closer than [`DIRECTION_EPSILON`], fall back to the unit heading
//!   [`FORWARD_FALLBACK`]. The fixed-index quirk is kept as well: standing on
//!   the first corner takes the fallback instead of looking ahead, and past
//!   the first corner the walker faces back at it.
//!
//! # Acceleration and braking: the per-frame transfer of the agent ([`crowd`])
//!
//! The start writes an acceleration (8.0), and every frame the move loop
//! rewrites the automatic-braking switch when "remaining distance < 0.3".
//! These inputs are consumed by the crowd update (turning, integration,
//! movement along navmesh cells, remaining distance), implemented in
//! [`crowd`] and in the `carve` corridor; NPC per-frame
//! movement takes that route. [`advance`] in this module (constant-speed
//! polyline) is no longer the NPC movement rule; it remains for callers that
//! still recompute along a polyline.
//!
//! The heading is not gated by the avoidance type either: the heading code
//! only runs when the avoidance type is non-zero, and that type is written
//! in two places under two different value conditions, so which one holds at
//! any moment is not modelled here. Gating on it would be inventing its
//! state — the heading rule applies unconditionally while walking, and gating
//! is left to the side that consumes the heading.
//!
//! # Refusal is loud
//!
//! A negative or non-numeric speed, a corner or persistent state holding a
//! non-finite value, or a negative or non-numeric dt rejects that NPC for the
//! frame: position and heading bytes stay unchanged, the remaining distance
//! is reported as unknown (infinity, matching what the engine-side property
//! reads when the distance is unknown), no arrival is reported, and the
//! refusal flag is set. The only speeds the move
//! loop ever writes are zero (cleared on stop) or non-negative table values;
//! nothing else can occur. Once a non-finite value enters the position state,
//! later snapshots are no longer byte-identical or recomputable. Silently
//! fixing such a value would mean inventing semantics nobody wrote down, so
//! refusing loudly is preferred.

pub mod crowd;
pub mod look_at;
pub mod slot;
pub mod turn;
pub mod walk;
pub mod waypoint;

pub use slot::NpcPathWalkSlot;
pub use turn::{
    TurnMotion, ROTATE_DEGREES_PER_SECOND, angle_between, heading_yaw, make_positive_yaw,
    rotate_time, turn_angle, turn_motion,
};
pub use walk::{WalkState, WalkVerdict, advance, facing_direction};
pub use waypoint::{Waypoint, WaypointDraw, WaypointKind, WAYPOINT_SAMPLE_DISTANCE, build_waypoints};

/// Arrival threshold: the move loop's own literal — once the straight-line
/// distance to the move target is below it, the loop exits. It equals the
/// stopping distance written when configuring the agent, but these are two
/// literals at two different places (one feeds the agent's braking, the other
/// is the loop's own comparison). They are kept as separate constants so no
/// false "single constant" claim can arise.
pub const ARRIVAL_DISTANCE: f32 = 0.1;

/// 近零回退阈：目标拐点距自身不超过它时，朝向取回退值而不做除以近零
/// 模长的归一化。
pub const DIRECTION_EPSILON: f32 = 1e-5;

/// Fallback heading: the unit forward `[0, 0, 1]` used when the target
/// corner is almost coincident with the walker.
pub const FORWARD_FALLBACK: [f32; 3] = [0.0, 0.0, 1.0];
