//! The virtual hand: a joystick finger, a tap finger, a drag finger and a
//! mouse wheel that write the same input streams a person makes, so every
//! gate the product puts on input (the entry's control lock, a dialog over
//! the field, a site move) holds for a step as it does for a player.
//!
//! The fingers are the host's own primitives: an action of any domain drives
//! the player through them. The joystick finger's state across frames is
//! [`Grip`], a resource; every operation on it takes the world and reaches
//! the finger itself, so a step calls it while holding nothing else.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowEvent};

use crate::joystick::{JoystickState, HANDLE_SIZE};

/// The joystick finger. Its touches go to the touch stream the joystick
/// reads.
const STICK_FINGER: u64 = 99_101;
/// The tap finger. Its touches go to the window's event stream the gesture
/// layer reads, which the joystick does not.
const TAP_FINGER: u64 = 99_102;
/// The drag finger, on the window's event stream like the tap finger: the
/// gesture layer turns its moves into the camera's drag.
const DRAG_FINGER: u64 = 99_103;

/// Where the joystick finger goes down: inside the stick's zone.
fn stick_base(window: &Window) -> Vec2 {
    Vec2::new(window.width() * 0.15, window.height() * 0.75)
}

fn window(world: &mut World) -> Option<(Entity, Window)> {
    world
        .query_filtered::<(Entity, &Window), With<PrimaryWindow>>()
        .single(world)
        .ok()
        .map(|(entity, window)| (entity, window.clone()))
}

/// The joystick finger across frames, and whose run and step it is.
#[derive(Resource, Default)]
pub struct Grip {
    pressing: bool,
    /// The run whose step put the joystick finger down, and the step.
    owner: Option<(u64, String)>,
}

/// Holds the stick toward world direction `toward` (x, z) for the step
/// `path` of the run `ticket`. The finger goes down at the stick's base on
/// the first frame and leans the next.
pub fn steer(world: &mut World, ticket: u64, path: &str, toward: Vec2) -> Result<(), &'static str> {
    let camera = world
        .query_filtered::<&GlobalTransform, With<Camera3d>>()
        .single(world)
        .map_err(|_| "no single 3D camera")?;
    let (forward, right) = (camera.forward(), camera.right());
    let toward = toward.normalize_or(Vec2::X);
    // The joystick's bake run backwards: the stick ratio whose
    // camera-relative direction is `toward`.
    let forward_flat = Vec2::new(forward.x, forward.z).normalize_or_zero();
    let stick = Vec2::new(toward.dot(Vec2::new(right.x, right.z)), toward.dot(forward_flat));
    lean(world, ticket, path, stick)
}

/// Holds the stick at ratio `stick` (x right, y up) for the step `path` of
/// the run `ticket`. The finger goes down at the stick's base on the first
/// frame and leans the next; a press the joystick did not capture is lifted
/// and pressed again once no dialog holds the field. A finger another step
/// of the same run put down is taken over; one another run put down is
/// lifted: a run leans only on its own.
pub fn lean(world: &mut World, ticket: u64, path: &str, stick: Vec2) -> Result<(), &'static str> {
    let (entity, window) = window(world).ok_or("no primary window")?;
    let base = stick_base(&window);
    // A press the joystick did not capture (a dialog over the field
    // refused it, or a gate dropped it while the finger was down) is
    // gone: a player lifts the finger and presses again.
    let captured = world.get_resource::<JoystickState>().map(|state| state.captured == Some(STICK_FINGER));
    let (pressing, other_run) = {
        let mut grip = world.resource_mut::<Grip>();
        if grip.pressing && captured == Some(false) {
            grip.pressing = false;
            grip.owner = None;
        }
        let other_run = grip.pressing && grip.owner.as_ref().is_some_and(|(owner, _)| *owner != ticket);
        if other_run {
            grip.pressing = false;
            grip.owner = None;
        }
        (grip.pressing, other_run)
    };
    if other_run {
        // A finger another run put down is lifted before this one leans.
        touch(world, entity, STICK_FINGER, TouchPhase::Ended, base);
        return lean(world, ticket, path, stick);
    }
    if !pressing {
        // While a dialog holds the field a new press would be refused too,
        // so wait for it.
        let dialog = world
            .get_resource::<crate::ui_layers::ScreenManager>()
            .is_some_and(crate::ui_layers::ScreenManager::exists_dialog);
        if dialog {
            return Ok(());
        }
        {
            let mut grip = world.resource_mut::<Grip>();
            grip.pressing = true;
            grip.owner = Some((ticket, path.to_owned()));
        }
        touch(world, entity, STICK_FINGER, TouchPhase::Started, base);
        return Ok(());
    }
    let scale = world
        .get_resource::<crate::canvas::RootCanvas>()
        .map(|canvas| canvas.scale(&window))
        .ok_or("the root canvas is not loaded")?;
    // A window with no drawable area has no canvas scale, and the reach of
    // the finger is that scale times the handle: there is no position to
    // put the finger at, so the lean is refused and the caller waits.
    if !(scale.is_finite() && scale > 0.0) {
        return Err("the window has no drawable area (minimized)");
    }
    {
        let mut grip = world.resource_mut::<Grip>();
        if grip.owner.as_ref().is_none_or(|(owner, step)| *owner != ticket || step != path) {
            grip.owner = Some((ticket, path.to_owned()));
        }
    }
    let position = base + Vec2::new(stick.x, -stick.y) * HANDLE_SIZE * scale;
    touch(world, entity, STICK_FINGER, TouchPhase::Moved, position);
    Ok(())
}

/// Lifts the joystick finger if it is down.
pub fn release(world: &mut World) {
    {
        let mut grip = world.resource_mut::<Grip>();
        if !grip.pressing {
            return;
        }
        grip.pressing = false;
        grip.owner = None;
    }
    if let Some((entity, window)) = window(world) {
        touch(world, entity, STICK_FINGER, TouchPhase::Ended, stick_base(&window));
    }
}

/// Lifts the joystick finger when the step `path` of the run `ticket` holds
/// it; `path` empty lifts it whatever step of the run holds it.
pub fn release_of(world: &mut World, ticket: u64, path: &str) {
    let owns = world
        .resource::<Grip>()
        .owner
        .as_ref()
        .is_some_and(|(owner, step)| *owner == ticket && (path.is_empty() || step == path));
    if owns {
        release(world);
    }
}

/// Whether the step `path` of the run `ticket` has the joystick finger down.
pub fn pressing(world: &World, ticket: u64, path: &str) -> bool {
    let grip = world.resource::<Grip>();
    grip.pressing && grip.owner.as_ref().is_some_and(|(owner, step)| *owner == ticket && step == path)
}

/// The run whose step has the joystick finger down, and that step's path.
pub fn leaning(world: &World) -> Option<(u64, &str)> {
    let grip = world.resource::<Grip>();
    if !grip.pressing {
        return None;
    }
    grip.owner.as_ref().map(|(owner, step)| (*owner, step.as_str()))
}

/// A tap at `position` (logical pixels, top-left origin): down and up in one
/// frame, so a slow frame cannot turn it into a long press.
pub fn tap(world: &mut World, position: Vec2) -> Result<(), &'static str> {
    let (entity, _) = window(world).ok_or("no primary window")?;
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        world.write_message(WindowEvent::TouchInput(TouchInput {
            phase,
            position,
            window: entity,
            force: None,
            id: TAP_FINGER,
        }));
    }
    Ok(())
}

/// One press and release of the back key (Escape) in one frame, as a
/// keyboard sends them.
pub fn back_key(world: &mut World) -> Result<(), &'static str> {
    let (entity, _) = window(world).ok_or("no primary window")?;
    for state in [ButtonState::Pressed, ButtonState::Released] {
        world.write_message(KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
            window: entity,
        });
    }
    Ok(())
}

/// The drag finger at `position` in `phase`.
pub fn drag_touch(world: &mut World, phase: TouchPhase, position: Vec2) -> Result<(), &'static str> {
    let (entity, _) = window(world).ok_or("no primary window")?;
    world.write_message(WindowEvent::TouchInput(TouchInput {
        phase,
        position,
        window: entity,
        force: None,
        id: DRAG_FINGER,
    }));
    Ok(())
}

/// A wheel turn of `pixels` (up positive), as a touchpad scrolls.
pub fn wheel(world: &mut World, pixels: f32) -> Result<(), &'static str> {
    let (entity, _) = window(world).ok_or("no primary window")?;
    world.write_message(MouseWheel {
        unit: MouseScrollUnit::Pixel,
        x: 0.0,
        y: pixels,
        window: entity,
        phase: TouchPhase::Moved,
    });
    Ok(())
}

/// The window's logical size.
pub fn window_size(world: &mut World) -> Option<Vec2> {
    window(world).map(|(_, window)| Vec2::new(window.width(), window.height()))
}

/// The window point of the node at `path` on the shown UI view `view` (its
/// document key), in logical pixels with a top-left origin, that the view's
/// own press raycast gives to the node: its centre when the raycast there
/// reaches the node, else another point of its rect. Several views can share
/// a document (a host's screen and a layout read for geometry only); the
/// first whose node is active is taken. A view that is not shown is named
/// with the views that are; a node active on none is refused; a node whose
/// whole rect is covered by other controls is refused with their paths.
pub fn ui_point(world: &mut World, view: &str, path: &str) -> Result<Vec2, String> {
    crate::act::ui_view::point(world, view, path)
}

/// The screen position of an action button on its laid-out view.
pub fn button_position(world: &mut World, button: moly_law::action_button::ButtonType) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).expect("system params").button_position(&window, button)
}

/// The screen position of the change-target button beside `shown`.
pub fn change_position(world: &mut World, shown: moly_law::action_button::ButtonType) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).expect("system params").change_position(&window, shown)
}

fn touch(world: &mut World, window: Entity, id: u64, phase: TouchPhase, position: Vec2) {
    world.write_message(TouchInput { phase, position, window, force: None, id });
}
