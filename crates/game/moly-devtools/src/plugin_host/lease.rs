//! The person's input behind the control lease, and the bus's events as the
//! host reads them.
//!
//! A person's input aborts a plugin's running hand step (see the module
//! docs): a key press, a mouse button, a touch that starts, a gamepad button
//! pressed or a gamepad stick moved. The director's own touches are not a
//! person's; its keys are.

use bevy::ecs::message::Messages;
use bevy::input::gamepad::{GamepadAxis, GamepadAxisChangedEvent, GamepadButtonChangedEvent};
use bevy::input::mouse::MouseButton;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::{ButtonInput, ButtonState};
use bevy::prelude::*;
use serde_json::Value;

/// How far a stick must move from rest to count as a person's input.
const STICK_THRESHOLD: f32 = 0.25;

/// The sequence number of the last event the bus holds.
pub(super) fn bus_latest() -> u64 {
    crate::bus::read(0, 1, None)["latest"].as_u64().unwrap_or(0)
}

/// The bus's events after `since`, oldest first, at most one read's worth.
pub(super) fn bus_since(since: u64) -> Vec<Value> {
    crate::bus::read(since, crate::bus::MOST_PER_READ, None)["events"].as_array().cloned().unwrap_or_default()
}

/// Whether a person acted this frame.
pub(super) fn human_input(world: &World) -> bool {
    let keys = world.resource::<ButtonInput<KeyCode>>().get_just_pressed().next().is_some();
    let mouse = world.resource::<ButtonInput<MouseButton>>().get_just_pressed().next().is_some();
    let pad = world
        .resource::<Messages<GamepadButtonChangedEvent>>()
        .iter_current_update_messages()
        .any(|event| event.state == ButtonState::Pressed);
    let stick = world.resource::<Messages<GamepadAxisChangedEvent>>().iter_current_update_messages().any(|event| {
        matches!(event.axis, GamepadAxis::LeftStickX | GamepadAxis::LeftStickY) && event.value.abs() > STICK_THRESHOLD
    });
    let touch = world
        .resource::<Messages<TouchInput>>()
        .iter_current_update_messages()
        .any(|touch| touch.phase == TouchPhase::Started);
    keys || mouse || pad || stick || touch
}
