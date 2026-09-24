//! UGUI rules as pure functions: canvas scaling, Image mesh generation,
//! raycast selection and the game's button click semantics.
//!
//! Every function takes the same inputs the engine code reads and returns the
//! same numbers it produces, so a comparison harness can feed identical inputs
//! to this port and to the executing engine and compare value by value.
//! Engine arithmetic is reproduced in f32 with the engine's operation order;
//! where the engine calls into .NET double math, the port does the same.

pub mod canvas;
pub mod custom_button;
pub mod image;
pub mod raycast;
pub mod unity_math;
