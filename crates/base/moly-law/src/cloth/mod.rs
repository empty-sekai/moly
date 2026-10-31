//! Cloth rules: the secondary physics of SD character hair/accessory
//! chains (the recomputable core of Magica Cloth 2 bone cloth mode).
//! Pure functions + explicit dt parameter, the same paradigm as
//! [`crate::path`]; the engine's Transform reads/writes belong to the
//! screen-wiring layer, and this module only handles flat data.
//!
//! # Basis and reference
//!
//! Mechanism (confirmed 2026-09-07): the secondary motion of SD station characters is driven
//! entirely by Magica Cloth 2 bone cloth — the animation library's 42
//! target chains contain no hair or accessory bones, and each
//! character's `sd_*.rig.json` carries all the data in its `cloth`
//! section. The reference behavior is that of `MagicaCloth.dll`.
//!
//! # Main pipeline (PhysicsManagerCompute.UpdateStartSimulation)
//!
//! ```text
//! CalcMaxUpdateCount(dt)          → substep count for this frame (90Hz fixed-step accumulator)
//! UpdateBoneToParticle            → read animated world positions
//! loop n times UpdatePhysics:     → one substep
//!     ForceAndVelocityJob         → Verlet integration (damping / velocity cap / gravity·dt²)
//!     constraint iterations ×SolverIterationCount, in registration order:
//!         ColliderCollision       → collision
//!         ClampDistance           → root distance ratio clamp (minRatio/maxRatio)
//!         RestoreDistance         → structural distance (stiffness + mass sharing)
//!         CompositeRotation       → angle limit + restore (composite for algorithm=1)
//!     FixPositionJob              → fixed-point repin / velocity bookkeeping
//! UpdateParticleToBone + ConvertWorldToLocal → write back to bone transform
//! ```
//!
//! In the registration order (the Create ctor + Add sequence)
//! Extrusion/Penetration/Spring/Twist/TriangleBend/ClampPosition are all
//! unregistered on this data set (constraintCounts all 0 or use switches
//! false); the selection survey of the 31 rigs has only
//! fixed(482)/move(990), no extend.
//!
//! # Behavior implemented by this module
//!
//! - root distance clamp correction: approaches the target per iteration
//!   by `clamp01(1-velInfl·0.5)` (converges over ×4 iterations,
//!   velInfl=0.2; the target is clamped directly to `min/maxRatio·len`);
//! - structural distance sharing: one-sided split by
//!   `(mass_j+5·infl_j)/(mass_i+5·infl_i+mass_j+5·infl_j)`, entries kept
//!   bidirectional from the rig (no fixed 50/50 split, no per-edge
//!   deduplication);
//! - angle limit parameters: `GetClampRotationAngle(algorithm=1)` =
//!   `clampRotationAngle2` (same values: 3°→20°);
//! - restore parameters: `GetRestoreRotationPower(algorithm=1)` =
//!   `restoreRotation2` (0.10→0.02; see the note in solver.rs);
//! - restore timing: CompositeRotation is registered inside the iteration
//!   loop (only part of the composite's internal formula is known, see the
//!   note in solver.rs);
//! - velocity absorption: a separate velocity buffer absorbs by
//!   `(1-velInfl)` (this module's pos/prev form, see the note in
//!   solver.rs);
//! - collision friction: deepest-hit bookkeeping within a 0.03m depth
//!   window (friction data 0.05/0.03, negligible in magnitude).
//!
//! # Wind (named conclusion, open item ②)
//!
//! Every chain has windInfluence=1.0 / windRandomScale=0.7 /
//! windSynchronization=0.6 (identical across all three units in every chain), but the wind
//! source is a MagicaWind component registered in the scene; neither the
//! SD prefab component histogram (rig `stats`) nor the station has one —
//! `windDataList` is empty ⇒ `Wind()` sums over an empty list ≡ 0.
//! Named "no wind": the parameters exist, the source does not, this
//! module never creates wind; the external-force entry
//! [`solver::EvaluatedParams::external_force`] defaults to zero.

pub mod bezier;
pub mod collider;
pub mod math;
pub mod schema;
pub mod solver;

pub use bezier::BezierParam;
pub use collider::{ColliderDef, WorldCollider};
pub use schema::{
    cloth_from_value, json_parse, params_from_value, ChainDef, ClothParamsDef, Selection,
};
pub use solver::{
    advance, build_chain, ChainTopology, ClothState, EvaluatedParams, FrameInput, StepStats,
};


