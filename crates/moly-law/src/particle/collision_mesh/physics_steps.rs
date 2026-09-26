//! The physics steps the engine runs in a frame. Each one is a simulation of
//! the physics scene, and so one rebuild step and a commit of its static
//! pruner (the scene query's update after the simulation's results).
//!
//! The engine simulates the scene from three places only:
//! - the fixed-update physics step, which simulates only in the fixed-update
//!   simulation mode;
//! - the late update, which simulates only in the update mode, and only on
//!   a frame with a positive delta time;
//! - the scripting call, which the game's code never makes (its compiled
//!   internal calls into the physics module hold no simulate entry).
//!
//! The project's physics settings put the simulation in script mode, so no
//! frame of the game runs a physics step. The static pruner is then never
//! stepped, and only the flushes of the scene queries commit it.

/// The physics settings' simulation mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationMode {
    FixedUpdate = 0,
    Update = 1,
    Script = 2,
}

/// Why the steps of a frame cannot be given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmodeled(pub &'static str);

/// The physics steps of one frame with delta time `delta`.
pub fn frame_steps(mode: SimulationMode, delta: f32) -> Result<u32, Unmodeled> {
    match mode {
        SimulationMode::Script => Ok(0),
        SimulationMode::Update => Ok(u32::from(delta > 0.0)),
        SimulationMode::FixedUpdate => Err(Unmodeled("the fixed-update step count is not ported")),
    }
}
