//! The director's hand: a joystick finger and a tap finger. It writes the
//! same input streams a person's fingers make, so every gate the product
//! puts on input (the entry's control lock, a dialog over the field, a site
//! move) holds for a script as it does for a player.

use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowEvent};

use crate::joystick::HANDLE_SIZE;

/// The joystick finger. Its touches go to the touch stream the joystick
/// reads.
const STICK_FINGER: u64 = 99_101;
/// The tap finger. Its touches go to the window's event stream the gesture
/// layer reads, which the joystick does not.
const TAP_FINGER: u64 = 99_102;

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

/// The joystick finger's state across frames.
#[derive(Default)]
pub(crate) struct Hand {
    pressing: bool,
}

impl Hand {
    /// Holds the stick toward world direction `toward` (x, z). The finger
    /// goes down at the stick's base on the first frame and leans the next.
    pub(crate) fn steer(&mut self, world: &mut World, toward: Vec2) -> Result<(), &'static str> {
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
        self.lean(world, stick)
    }

    /// Holds the stick at ratio `stick` (x right, y up). The finger goes down
    /// at the stick's base on the first frame and leans the next.
    pub(crate) fn lean(&mut self, world: &mut World, stick: Vec2) -> Result<(), &'static str> {
        let (entity, window) = window(world).ok_or("no primary window")?;
        let base = stick_base(&window);
        if !self.pressing {
            touch(world, entity, STICK_FINGER, TouchPhase::Started, base);
            self.pressing = true;
            return Ok(());
        }
        let scale = world
            .get_resource::<crate::canvas::RootCanvas>()
            .map(|canvas| canvas.scale(&window))
            .ok_or("the root canvas is not loaded")?;
        let position = base + Vec2::new(stick.x, -stick.y) * HANDLE_SIZE * scale;
        touch(world, entity, STICK_FINGER, TouchPhase::Moved, position);
        Ok(())
    }

    /// Lifts the joystick finger if it is down.
    pub(crate) fn release(&mut self, world: &mut World) {
        if !self.pressing {
            return;
        }
        self.pressing = false;
        if let Some((entity, window)) = window(world) {
            touch(world, entity, STICK_FINGER, TouchPhase::Ended, stick_base(&window));
        }
    }

    pub(crate) fn pressing(&self) -> bool {
        self.pressing
    }
}

/// A tap at `position` (logical pixels, top-left origin): down and up in one
/// frame, so a slow frame cannot turn it into a long press.
pub(crate) fn tap(world: &mut World, position: Vec2) -> Result<(), &'static str> {
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

/// The window's logical size.
pub(crate) fn window_size(world: &mut World) -> Option<Vec2> {
    window(world).map(|(_, window)| Vec2::new(window.width(), window.height()))
}

/// The screen position of an action button on its laid-out view.
pub(crate) fn button_position(
    world: &mut World,
    button: moly_law::action_button::ButtonType,
) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).button_position(&window, button)
}

/// The screen position of the change-target button beside `shown`.
pub(crate) fn change_position(
    world: &mut World,
    shown: moly_law::action_button::ButtonType,
) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).change_position(&window, shown)
}

fn touch(world: &mut World, window: Entity, id: u64, phase: TouchPhase, position: Vec2) {
    world.write_message(TouchInput { phase, position, window, force: None, id });
}
