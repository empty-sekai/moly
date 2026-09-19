//! Source particle velocity curves, transient orbital displacement and sampling.
mod motion;
mod sampling;
pub use motion::{OrbitalMotion, animated_velocity};
pub use sampling::{VelocityOverLifetime, VelocitySample};
