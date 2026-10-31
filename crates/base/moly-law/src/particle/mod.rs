//! The particle laws: Unity `ParticleSystem` semantics as pure functions with
//! `dt` passed explicitly.
//!
//! Site particles run on the engine's ParticleSystem, and the game side carries
//! only the parameters. Where a law is tied to an engine class or member, the
//! comment names that class or member and states what it does.
//!
//! # Evidence tiers (each law's comments mark one)
//!
//! | tier | meaning | example |
//! |---|---|---|
//! | `C# readable` | the formula is readable in UnityCsReference | the four-mode switch of `MinMaxCurve.Evaluate` |
//! | `C# readable (existence)` | only the presence of a field or member is established | `PlaybackState.m_ToEmitAccumulator` |
//! | `behavioural spec` | behind the extern wall (`Simulate`/`Evaluate` are all native) | the accumulator's rounding, the integration order, the death comparison |
//! | `bit-exact` | results, the order of operations and the constants match the reference bit for bit | the sampling and advance of the angular-velocity and limit-velocity families |
//!
//! A `behavioural spec` entry is implemented from a behaviour specification
//! that can be recomputed, and its comments give an Editor measurement plan;
//! until that measurement runs it is a hypothesis with a criterion, not an
//! established fact. A `bit-exact` entry is aligned by value comparison
//! (operation order and constant bit patterns compared one by one; the places
//! where a bit-exact match is out of reach by construction, such as library
//! functions and CPU-dependent operations, are named).
//!
//! # Shape
//!
//! As in the `path` family: no engine types and no global state; every random
//! draw is an explicit argument (which laws read the same `rand` is the
//! caller's decision; the laws draw nothing themselves, and one value read
//! twice is not two draws). Cross-frame state is carried by the caller.
//! A "pure hash factor" is not a draw: the angular-velocity and
//! limit-velocity factors are deterministic functions of the per-particle
//! seed and take 0 draws from a stream per frame.

mod json;

pub mod value;
pub mod shape;
pub mod shape_birth;
pub mod shape_mesh;
pub mod emit;
pub(crate) mod step;
pub(crate) mod buffer;
pub mod schema;
pub(crate) mod rotation;
pub mod rotation_by_speed;
pub(crate) mod mesh_selection;
pub(crate) mod limit_velocity;
pub mod main_duration;
pub mod velocity;
pub mod random;
pub mod curve;
pub(crate) mod device_libm;
pub mod size;
pub mod gradient;
pub mod color;
pub mod custom_data;
pub mod slot_tail;
pub mod texture_sheet;
pub mod force;
pub mod gravity;
pub mod sort;
pub mod collision_response;
pub mod collision_event;
pub mod death_event;
pub mod inherit;
pub mod collision_query;
pub mod collision_planes;
pub mod current_size;
pub mod collision_mesh;
pub mod noise;
pub mod seed_owner;
pub mod initial;
pub mod autonomous_emission;
pub mod sub_emission;
pub mod owner;
pub mod placement;
pub mod child_emit;
pub mod prewarm;
pub mod procedural;
pub mod frame_time;
pub mod culling;
pub mod noise_size;
mod armf;
pub mod trail;
pub mod trail_geometry;
pub mod mesh_transform;
pub mod stretch_geometry;

#[cfg(test)]
mod corpus;
#[cfg(test)]
mod ring_lifecycle_samples;

// The modules below are reachable from outside only through the re-exports in
// this list.
pub use buffer::{birth_capacity, compact, compact_with_side, compact_with_sides, compact_with_sides_indexed,
    finish_births, finish_births_with, RingBufferMode};
pub use emit::{accumulate_rate, burst_check, Burst, BurstOutcome, EmissionState};
pub use limit_velocity::{advance_age_percent, DragLaw, DragSize, LimitVelocity, MagnitudeLaw};
pub use rotation::{BakedCurve, RotationOverLifetime};
pub use schema::{EmitterParams, Effects, EffectsError};
pub use shape::{circle_position, euler_rotate_deg};
pub use step::{advance_lifetime, apply_gravity, integrate, LifetimeVerdict, Particle,
    StepVerdict};
pub use value::{Curve, CurveKey, Gradient, GradientAlphaKey, GradientColorKey, MinMaxCurve,
    MinMaxGradient};
