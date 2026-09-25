//! UGUI rules as pure functions: canvas scaling, Image mesh generation,
//! raycast selection, the game's button click semantics, its button tap effect
//! and its MySekai rank gauge.
//!
//! Every function takes the same inputs the engine code reads and returns the
//! same numbers it produces, so a comparison harness can feed identical inputs
//! to this port and to the executing engine and compare value by value.
//! Engine arithmetic is reproduced in f32 with the engine's operation order;
//! where the engine calls into .NET double math, the port does the same.

pub mod canvas;
pub mod custom_button;
pub mod graphic_tap_effect;
pub mod image;
pub mod mysekai_rank;
pub mod raycast;
pub mod screen_ray;
pub mod unity_math;
