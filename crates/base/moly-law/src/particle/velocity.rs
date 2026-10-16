//! Source particle velocity curves, transient orbital displacement and sampling.
mod motion;
pub(crate) use motion::orbital_reciprocal;
mod sampling;
pub use motion::{OrbitalMotion, animated_velocity};
pub use sampling::{VelocityOverLifetime, VelocitySample};
