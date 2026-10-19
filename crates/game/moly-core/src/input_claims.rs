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

use bevy::prelude::*;

/// The control panel's drawer.
pub const CONTROL_PANEL: &str = "control_panel";
/// The settings modal.
pub const SETTINGS_PANEL: &str = "settings_panel";
/// The content library.
pub const CONTENT_LIBRARY: &str = "content_library";

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
    /// the surfaces covering it: the settings modal and the content library
    /// hold nothing. The control panel's drawer takes the field's input of
    /// its own while it covers the field, and a route that consults it asks
    /// [`Self::holds`].
    pub fn world_input_free(&self) -> bool {
        !self.holds(SETTINGS_PANEL) && !self.holds(CONTENT_LIBRARY)
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
