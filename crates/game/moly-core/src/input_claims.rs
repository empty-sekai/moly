//! Who holds the field's input this frame.
//!
//! The surfaces drawn over the field (the settings modal, the control
//! panel's drawer, the content library) each decide whether they take the
//! field's input as a whole, whether they take the pointer alone, and the
//! region a pointer claim covers. The field's own input routes ask this one
//! resource instead of each surface, so a surface that moves out of this
//! module takes its claim with it.
//!
//! Each surface publishes its own claim from the system that changes its
//! state, ordered before the field's input routes, so a route reads one
//! frame's claims and the state they stand for at the same point of the
//! frame. A surface that moves out of this module keeps its publisher.
//!
//! Besides the whole-field claim, a holder can take single [`InputChannel`]s:
//! the field's world input, exploration (walking and picking), the camera,
//! conversation clicks, and the scene itself. A route asks
//! [`InputClaims::blocks`] for its channel, and [`InputClaims::holders`]
//! names who holds it, so a refusal can say which surface is in the way.
//! Holder names are owned strings: a surface registered at run time (a
//! plugin's panel, for example) claims channels under its own id.

use std::borrow::Cow;

use bevy::prelude::*;

/// The control panel's drawer.
pub const CONTROL_PANEL: &str = "control_panel";
/// The settings modal.
pub const SETTINGS_PANEL: &str = "settings_panel";
/// The content library.
pub const CONTENT_LIBRARY: &str = "content_library";
/// Input that the embedding page or a library host command captured for its
/// own surfaces.
pub const HOST_INPUT: &str = "host_input";
/// The channels [`HOST_INPUT`] takes while the host captures input: every
/// channel but [`InputChannel::Scene`].
pub const HOST_INPUT_CHANNELS: Channels = Channels::NONE
    .with(InputChannel::World, true)
    .with(InputChannel::Exploration, true)
    .with(InputChannel::Camera, true)
    .with(InputChannel::Talk, true);

/// The systems through which the host sets its input capture
/// ([`InputClaims::capture_host_input`]); readers that act on the same frame's
/// capture run after this set.
#[derive(bevy::ecs::schedule::SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostInputSystems;

/// One kind of the field's input a holder can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputChannel {
    /// The field's input as a whole: interactions, layout edits, site moves.
    World,
    /// Walking and picking in the field.
    Exploration,
    /// Turning and zooming the camera.
    Camera,
    /// Clicks that advance or answer a conversation.
    Talk,
    /// The scene itself: a holder stages or plays something in it, and
    /// nothing else may start a scene change.
    Scene,
}

impl InputChannel {
    pub const ALL: [InputChannel; 5] = [
        InputChannel::World,
        InputChannel::Exploration,
        InputChannel::Camera,
        InputChannel::Talk,
        InputChannel::Scene,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`InputChannel`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Channels(u8);

impl Channels {
    pub const NONE: Channels = Channels(0);

    /// The set with `channel` added when `held`.
    #[must_use]
    pub const fn with(self, channel: InputChannel, held: bool) -> Channels {
        if held {
            Channels(self.0 | channel.bit())
        } else {
            self
        }
    }

    pub const fn contains(self, channel: InputChannel) -> bool {
        self.0 & channel.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The channels in the set, in [`InputChannel::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = InputChannel> {
        InputChannel::ALL
            .into_iter()
            .filter(move |channel| self.contains(*channel))
    }
}

/// The surfaces that hold the field's input, and the pointer region the
/// surface holding the pointer covers.
#[derive(Resource, Default)]
pub struct InputClaims {
    /// The surfaces that take the field's input as a whole.
    world: Vec<&'static str>,
    /// The surfaces that hold the pointer alone.
    pointer: Vec<&'static str>,
    /// The surface whose pointer claim covers a region, and that region in
    /// logical pixels.
    region: Option<(&'static str, Rect)>,
    /// The channels each holder takes, in the order holders first published.
    channels: Vec<(Cow<'static, str>, Channels)>,
}

impl InputClaims {
    /// Takes the field's input for `holder`, as a modal or a drawer over the
    /// field does. Claiming twice changes nothing.
    pub fn claim(&mut self, holder: &'static str) {
        if !self.world.contains(&holder) {
            self.world.push(holder);
        }
    }

    /// Gives the field's input back.
    pub fn release(&mut self, holder: &'static str) {
        self.world.retain(|claimed| *claimed != holder);
    }

    /// Whether `holder` holds the field's input.
    pub fn holds(&self, holder: &'static str) -> bool {
        self.world.contains(&holder)
    }

    /// Whether the field's input is free where the field's own routes read
    /// the surfaces covering it: the settings modal holds nothing, and the
    /// content library does not hold the [`InputChannel::World`] channel. The
    /// control panel's drawer takes the field's input of its own while it
    /// covers the field, and a route that consults it asks [`Self::holds`].
    pub fn world_input_free(&self) -> bool {
        !self.holds(SETTINGS_PANEL) && !self.holds_channel(CONTENT_LIBRARY, InputChannel::World)
    }

    /// Replaces the channels `holder` takes with `channels`; an empty set
    /// removes the holder.
    pub fn publish_channels(&mut self, holder: impl Into<Cow<'static, str>>, channels: Channels) {
        let holder = holder.into();
        match self.channels.iter().position(|(name, _)| *name == holder) {
            Some(index) if channels.is_empty() => {
                self.channels.remove(index);
            }
            Some(index) => self.channels[index].1 = channels,
            None if channels.is_empty() => {}
            None => self.channels.push((holder, channels)),
        }
    }

    /// Sets whether the host captures input: [`HOST_INPUT`] takes
    /// [`HOST_INPUT_CHANNELS`], or nothing. The last call wins, whichever
    /// host surface made it.
    pub fn capture_host_input(&mut self, captured: bool) {
        let channels = if captured { HOST_INPUT_CHANNELS } else { Channels::NONE };
        if self.channels_of(HOST_INPUT) != channels {
            self.publish_channels(HOST_INPUT, channels);
        }
    }

    /// The channels `holder` takes.
    pub fn channels_of(&self, holder: &str) -> Channels {
        self.channels
            .iter()
            .find(|(name, _)| name == holder)
            .map_or(Channels::NONE, |(_, channels)| *channels)
    }

    /// Whether `holder` takes `channel`.
    pub fn holds_channel(&self, holder: &str, channel: InputChannel) -> bool {
        self.channels_of(holder).contains(channel)
    }

    /// Whether any holder takes `channel`.
    pub fn blocks(&self, channel: InputChannel) -> bool {
        self.channels
            .iter()
            .any(|(_, channels)| channels.contains(channel))
    }

    /// The holders that take `channel`, in the order they first published.
    pub fn holders(&self, channel: InputChannel) -> impl Iterator<Item = &str> {
        self.channels
            .iter()
            .filter(move |(_, channels)| channels.contains(channel))
            .map(|(name, _)| name.as_ref())
    }

    /// Takes the pointer for `holder`, without taking the field's input.
    pub fn claim_pointer(&mut self, holder: &'static str) {
        if !self.pointer.contains(&holder) {
            self.pointer.push(holder);
        }
    }

    /// Gives the pointer back.
    pub fn release_pointer(&mut self, holder: &'static str) {
        self.pointer.retain(|claimed| *claimed != holder);
    }

    /// Whether the pointer is anybody's this frame.
    pub fn holds_pointer(&self) -> bool {
        !self.pointer.is_empty()
    }

    /// Records the region `holder`'s pointer claim covers, in logical
    /// pixels; `None` clears the region of that holder alone.
    pub fn hold_region(&mut self, holder: &'static str, region: Option<Rect>) {
        match region {
            Some(region) => self.region = Some((holder, region)),
            None if self.region.is_some_and(|(claimed, _)| claimed == holder) => self.region = None,
            None => {}
        }
    }

    /// Whether a window position (logical pixels) lies inside the region the
    /// pointer claim covers.
    pub fn captures(&self, position: Vec2) -> bool {
        self.region.is_some_and(|(_, region)| region.contains(position))
    }
}

/// Publishes `holder`'s claim from the state the surface owns. Called by each
/// surface's own system, at the point its readers of [`InputClaims`] expect.
pub fn publish(claims: &mut InputClaims, holder: &'static str, held: bool) {
    if held {
        claims.claim(holder);
    } else {
        claims.release(holder);
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<InputClaims>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_name_their_holders_and_an_empty_set_removes_one() {
        let mut claims = InputClaims::default();
        let camera = Channels::NONE.with(InputChannel::Camera, true);
        claims.publish_channels(CONTENT_LIBRARY, camera.with(InputChannel::World, true));
        claims.publish_channels(String::from("plugin.panel"), camera);
        assert!(claims.blocks(InputChannel::Camera));
        assert!(!claims.blocks(InputChannel::Talk));
        assert_eq!(
            claims.holders(InputChannel::Camera).collect::<Vec<_>>(),
            [CONTENT_LIBRARY, "plugin.panel"]
        );
        assert!(!claims.world_input_free());
        claims.publish_channels(CONTENT_LIBRARY, Channels::NONE);
        assert!(claims.world_input_free());
        assert_eq!(claims.holders(InputChannel::Camera).collect::<Vec<_>>(), ["plugin.panel"]);
        claims.publish_channels("plugin.panel", Channels::NONE);
        assert!(!claims.blocks(InputChannel::Camera));
    }
}
