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
pub struct Hand {
    pressing: bool,
    /// The run whose step put the joystick finger down.
    owner: Option<u64>,
}

impl Hand {
    /// Holds the stick toward world direction `toward` (x, z) for the run
    /// with `ticket`. The finger goes down at the stick's base on the first
    /// frame and leans the next.
    pub fn steer(&mut self, world: &mut World, ticket: u64, toward: Vec2) -> Result<(), &'static str> {
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
        self.lean(world, ticket, stick)
    }

    /// Holds the stick at ratio `stick` (x right, y up) for the run with
    /// `ticket`. The finger goes down at the stick's base on the first frame
    /// and leans the next; a press the joystick did not capture is lifted and
    /// pressed again once no dialog holds the field. A finger another run put
    /// down is lifted: a run leans only on its own.
    pub fn lean(&mut self, world: &mut World, ticket: u64, stick: Vec2) -> Result<(), &'static str> {
        let (entity, window) = window(world).ok_or("no primary window")?;
        let base = stick_base(&window);
        // A press the joystick did not capture (a dialog over the field
        // refused it, or a gate dropped it while the finger was down) is
        // gone: a player lifts the finger and presses again. While a dialog
        // holds the field a new press would be refused too, so wait for it.
        let captured = world
            .get_resource::<crate::joystick::JoystickState>()
            .map(|state| state.captured == Some(STICK_FINGER));
        if self.pressing && captured == Some(false) {
            self.pressing = false;
            self.owner = None;
        }
        if self.pressing && self.owner != Some(ticket) {
            self.release(world);
        }
        if !self.pressing {
            if world
                .get_resource::<crate::ui_layers::ScreenManager>()
                .is_some_and(crate::ui_layers::ScreenManager::exists_dialog)
            {
                return Ok(());
            }
            touch(world, entity, STICK_FINGER, TouchPhase::Started, base);
            self.pressing = true;
            self.owner = Some(ticket);
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
        let position = base + Vec2::new(stick.x, -stick.y) * HANDLE_SIZE * scale;
        touch(world, entity, STICK_FINGER, TouchPhase::Moved, position);
        Ok(())
    }

    /// Lifts the joystick finger if it is down.
    pub fn release(&mut self, world: &mut World) {
        if !self.pressing {
            return;
        }
        self.pressing = false;
        self.owner = None;
        if let Some((entity, window)) = window(world) {
            touch(world, entity, STICK_FINGER, TouchPhase::Ended, stick_base(&window));
        }
    }

    /// Whether `ticket`'s run has the joystick finger down.
    pub fn pressing(&self, ticket: u64) -> bool {
        self.pressing && self.owner == Some(ticket)
    }
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
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::ButtonState;
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
    let candidates: Vec<&crate::ui_layout::UiPrefabView> = shown
        .iter()
        .filter(|candidate| candidate.key == view)
        .copied()
        .collect();
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
                found = Some((candidate, rect));
                break;
            }
            Ok(_) => {}
            Err(reason) => refused = refused.or(Some(reason)),
        }
    }
    let Some((view, rect)) = found else {
        return Err(refused.unwrap_or_else(|| format!("{view} {path} is not active")));
    };
    let doc = layouts
        .document(view.key)
        .ok_or_else(|| format!("view {} has no document", view.key))?;
    let index = doc.find(path)?;
    ui_press_point(view, doc, layouts, &window, &root, canvas, &rect, path, index)
}

/// The point of `rect` the view's press raycast gives to `index`: the centre
/// first, then a grid across the rect's canvas bounds; every tested point is
/// the window point the tap sends, so the game's own pointer conversion and
/// raycast decide. The covering paths are collected for the refusal.
#[allow(clippy::too_many_arguments)]
fn ui_press_point(
    view: &crate::ui_layout::UiPrefabView,
    doc: &moly_assets::ui_layout::UiPrefab,
    layouts: &crate::ui_layout::UiLayouts,
    window: &Window,
    root: &crate::canvas::RootCanvas,
    canvas: Vec2,
    rect: &moly_assets::ui_layout::UiRect,
    path: &str,
    index: usize,
) -> Result<Vec2, String> {
    let mut covering = Vec::new();
    if let Some(point) = ui_press_at(view, doc, layouts, window, root, canvas, rect, index, rect.center(), &mut covering) {
        return Ok(point);
    }
    // The centre is the usual covered point; a grid across the rect's bounds
    // finds a free part of it (a fine grid so a small uncovered strip is still
    // found). A point the raycast cannot reach, such as one off the window,
    // simply wins nothing.
    let min = -rect.size * rect.pivot;
    let max = min + rect.size;
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for corner in [
        Vec2::new(min.x, min.y),
        Vec2::new(max.x, min.y),
        Vec2::new(max.x, max.y),
        Vec2::new(min.x, max.y),
    ] {
        let at = rect.world.transform_point3(corner.extend(0.0)).truncate();
        lo = lo.min(at);
        hi = hi.max(at);
    }
    let mut steps = 4u32;
    while steps <= 32 {
        for iy in 0..steps {
            for ix in 0..steps {
                let fx = (ix as f32 + 1.0) / (steps as f32 + 1.0);
                let fy = (iy as f32 + 1.0) / (steps as f32 + 1.0);
                let at = Vec2::new(lo.x + (hi.x - lo.x) * fx, lo.y + (hi.y - lo.y) * fy);
                if let Some(point) = ui_press_at(view, doc, layouts, window, root, canvas, rect, index, at, &mut covering) {
                    return Ok(point);
                }
            }
        }
        steps *= 2;
    }
    covering.sort_unstable();
    covering.dedup();
    covering.truncate(8);
    let listed = if covering.is_empty() {
        "no control is hit there".to_owned()
    } else {
        format!("covering controls: {}", covering.join(", "))
    };
    Err(format!(
        "view {} path {path}: no point of its rect gives the press to it ({listed})",
        view.key
    ))
}

/// Whether the press at `at` (a point of this view's canvas units) reaches
/// `index`: the raycast winner, or the selectable the raycast hands the press
/// to. The covering winner's path is kept for the refusal.
#[allow(clippy::too_many_arguments)]
fn ui_press_at(
    view: &crate::ui_layout::UiPrefabView,
    doc: &moly_assets::ui_layout::UiPrefab,
    layouts: &crate::ui_layout::UiLayouts,
    window: &Window,
    root: &crate::canvas::RootCanvas,
    canvas: Vec2,
    rect: &moly_assets::ui_layout::UiRect,
    index: usize,
    at: Vec2,
    covering: &mut Vec<String>,
) -> Option<Vec2> {
    if !rect.contains(at) {
        return None;
    }
    let point = root.to_window(window, at);
    if point.x < 0.0 || point.y < 0.0 || point.x > window.width() || point.y > window.height() {
        return None;
    }
    let pointer = crate::menu_shell::event_pointer(window, root, point);
    let winner = view.raycast_winner(layouts, pointer, canvas);
    if winner == Some(index) {
        return Some(point);
    }
    if view.press_target(layouts, pointer, canvas).is_some_and(|(node, _)| node == index) {
        return Some(point);
    }
    if let Some(node) = winner.and_then(|winner| doc.nodes.get(winner)) {
        covering.push(node.path.clone());
    }
    None
}

/// The screen position of an action button on its laid-out view.
pub fn button_position(
    world: &mut World,
    button: moly_law::action_button::ButtonType,
) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).expect("system params").button_position(&window, button)
}

/// The screen position of the change-target button beside `shown`.
pub fn change_position(
    world: &mut World,
    shown: moly_law::action_button::ButtonType,
) -> Option<Vec2> {
    let (_, window) = window(world)?;
    let mut screen: bevy::ecs::system::SystemState<crate::action_button::ActionButtonScreen> =
        bevy::ecs::system::SystemState::new(world);
    screen.get(world).expect("system params").change_position(&window, shown)
}

fn touch(world: &mut World, window: Entity, id: u64, phase: TouchPhase, position: Vec2) {
    world.write_message(TouchInput { phase, position, window, force: None, id });
}
