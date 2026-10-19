//! The current room's door as the actors use it: its inside point, opening
//! and closing it, and awaiting an opening. The site move owns the door and
//! fills [`RoomDoorPort`] when it is installed; without it a room has no
//! door, so there is no inside point, opening and closing do nothing and an
//! awaited opening returns on the frame after it starts.

use bevy::prelude::*;

/// The door operations the site move provides.
#[derive(Resource, Clone, Copy)]
pub struct RoomDoorPort {
    pub inside_point: fn(&World) -> Option<Entity>,
    pub open: fn(&mut World),
    pub close: fn(&mut World),
    /// The length of the clip the current door animator is in; 0 without a
    /// door animator or a clip played on it.
    pub current_clip_length: fn(&World) -> f32,
}

/// The door's inside point (its action point), while the room site is the
/// active one.
pub fn inside_point(world: &World) -> Option<Entity> {
    world
        .get_resource::<RoomDoorPort>()
        .and_then(|port| (port.inside_point)(world))
}

/// `RoomController.OpenDoorAsync`.
pub fn open(world: &mut World) {
    if let Some(port) = world.get_resource::<RoomDoorPort>().copied() {
        (port.open)(world);
    }
}

/// `RoomController.CloseDoorAsync().Forget()`.
pub fn close(world: &mut World) {
    if let Some(port) = world.get_resource::<RoomDoorPort>().copied() {
        (port.close)(world);
    }
}

fn current_clip_length(world: &World) -> f32 {
    world
        .get_resource::<RoomDoorPort>()
        .map_or(0.0, |port| (port.current_clip_length)(world))
}

/// An awaited `OpenDoorAsync`, started on the frame of [`open`]: one
/// `Yield`, then the length of the clip the door's animator is in on that
/// frame (`GetCurrentAnimatorStateInfo(0).length`, read once), then
/// `t += deltaTime` each frame until `t` reaches it. A room without a door
/// animator, or a zero length, ends the wait on the frame after the call.
/// The wait is the caller's own: a later `Play` on the door (another NPC's
/// open, the player's move closing it) does not shorten or extend it.
#[derive(Clone, Copy, Debug)]
pub struct OpenWait {
    started: u64,
    length: Option<f32>,
    t: f32,
}

impl OpenWait {
    pub fn new(frame: u64) -> Self {
        Self {
            started: frame,
            length: None,
            t: 0.0,
        }
    }

    /// One frame of the wait; true on the frame the await returns.
    pub fn tick(&mut self, world: &World, frame: u64, dt: f32) -> bool {
        if frame == self.started {
            return false;
        }
        let length = *self.length.get_or_insert_with(|| current_clip_length(world));
        if self.t < length {
            self.t += dt;
            false
        } else {
            true
        }
    }
}
