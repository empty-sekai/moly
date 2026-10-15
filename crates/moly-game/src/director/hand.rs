//! The director's hand: a joystick finger, a tap finger, a drag finger and
//! a mouse wheel. It writes the same input streams a person makes, so every
//! gate the product puts on input (the entry's control lock, a dialog over
//! the field, a site move) holds for a script as it does for a player.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
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

/// The drag finger at `position` in `phase`.
pub(crate) fn drag_touch(world: &mut World, phase: TouchPhase, position: Vec2) -> Result<(), &'static str> {
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
pub(crate) fn wheel(world: &mut World, pixels: f32) -> Result<(), &'static str> {
    let (entity, _) = window(world).ok_or("no primary window")?;
    world.write_message(MouseWheel { unit: MouseScrollUnit::Pixel, x: 0.0, y: pixels, window: entity });
    Ok(())
}

/// The window's logical size.
pub(crate) fn window_size(world: &mut World) -> Option<Vec2> {
    window(world).map(|(_, window)| Vec2::new(window.width(), window.height()))
}

/// The centre of the node at `path` on the shown UI view `view` (its
/// document key), in logical pixels with a top-left origin. Several views
/// can share a document (a host's screen and a layout read for geometry
/// only); the first whose node is active is taken. A view that is not shown
/// is named with the views that are; a node active on none is refused.
pub(crate) fn ui_point(world: &mut World, view: &str, path: &str) -> Result<Vec2, String> {
    let (_, window) = window(world).ok_or("no primary window")?;
    let root = *world.get_resource::<crate::canvas::RootCanvas>().ok_or("the root canvas is not set up")?;
    let canvas = root.size(&window);
    let mut views = world.query::<(&crate::ui_layout::UiPrefabView, Option<&InheritedVisibility>)>();
    let layouts = world.get_resource::<crate::ui_layout::UiLayouts>().ok_or("the UI layouts are not loaded")?;
    let shown: Vec<&crate::ui_layout::UiPrefabView> = views
        .iter(world)
        .filter(|(_, visibility)| visibility.is_none_or(|visibility| visibility.get()))
        .map(|(view, _)| view)
        .collect();
    let candidates: Vec<_> = shown.iter().filter(|candidate| candidate.key == view).collect();
    if candidates.is_empty() {
        let mut keys: Vec<&str> = shown.iter().map(|candidate| candidate.key).collect();
        keys.sort_unstable();
        keys.dedup();
        return Err(format!("no view {view} is shown (shown: {})", keys.join(", ")));
    }
    let (mut found, mut refused) = (None, None);
    for candidate in candidates {
        match candidate.try_rect(layouts, path, canvas) {
            Ok(rect) if rect.active => {
                found = Some(rect);
                break;
            }
            Ok(_) => {}
            Err(reason) => refused = refused.or(Some(reason)),
        }
    }
    let Some(rect) = found else {
        return Err(refused.unwrap_or_else(|| format!("{view} {path} is not active")));
    };
    Ok(root.to_window(&window, rect.center()))
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
