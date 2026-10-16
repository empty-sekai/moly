//! Who holds the field's input this frame.
//!
//! The surfaces drawn over the field (the settings modal, the control
//! panel's drawer, the content library) each decide whether they take the
//! field's input as a whole, whether they take the pointer alone, and the
//! region a pointer claim covers. The field's own input routes ask this one
//! resource instead of each surface, so a surface that moves out of this
//! module takes its claim with it.
//!
//! A claim is published by the system that changes the surface's state, so a
//! reader ordered before that system sees what it saw before the change and
//! a reader ordered after it sees the change: the claim and the state it
//! stands for are always read at the same point of a frame. Two surfaces
//! keep their gate in systems of their own (the settings modal is also
//! opened by a failed colour apply, and the content library's gate is its
//! own state), so [`publish_field_gates`] copies those two into the claims
//! where the field's own input routes read them.

use bevy::prelude::*;

/// The control panel's drawer.
pub(crate) const CONTROL_PANEL: &str = "control_panel";
/// The settings modal.
pub(crate) const SETTINGS_PANEL: &str = "settings_panel";
/// The content library.
pub(crate) const CONTENT_LIBRARY: &str = "content_library";

/// The surfaces that hold the field's input, and the pointer region the
/// surface holding the pointer covers.
#[derive(Resource, Default)]
pub(crate) struct InputClaims {
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
    pub(crate) fn claim(&mut self, holder: &'static str) {
        if !self.world.contains(&holder) {
            self.world.push(holder);
        }
    }

    /// Gives the field's input back.
    pub(crate) fn release(&mut self, holder: &'static str) {
        self.world.retain(|claimed| *claimed != holder);
    }

    /// Whether `holder` holds the field's input.
    pub(crate) fn holds(&self, holder: &'static str) -> bool {
        self.world.contains(&holder)
    }

    /// Whether the field's input is free where the field's own routes read
    /// the surfaces covering it: the settings modal and the content library
    /// hold nothing. The control panel's drawer takes the field's input of
    /// its own while it covers the field, and a route that consults it asks
    /// [`Self::holds`].
    pub(crate) fn world_input_free(&self) -> bool {
        !self.holds(SETTINGS_PANEL) && !self.holds(CONTENT_LIBRARY)
    }

    /// Takes the pointer for `holder`, without taking the field's input.
    pub(crate) fn claim_pointer(&mut self, holder: &'static str) {
        if !self.pointer.contains(&holder) {
            self.pointer.push(holder);
        }
    }

    /// Gives the pointer back.
    pub(crate) fn release_pointer(&mut self, holder: &'static str) {
        self.pointer.retain(|claimed| *claimed != holder);
    }

    /// Whether the pointer is anybody's this frame.
    pub(crate) fn holds_pointer(&self) -> bool {
        !self.pointer.is_empty()
    }

    /// Records the region `holder`'s pointer claim covers, in logical
    /// pixels; `None` clears the region of that holder alone.
    pub(crate) fn hold_region(&mut self, holder: &'static str, region: Option<Rect>) {
        match region {
            Some(region) => self.region = Some((holder, region)),
            None if self.region.is_some_and(|(claimed, _)| claimed == holder) => self.region = None,
            None => {}
        }
    }

    /// Whether a window position (logical pixels) lies inside the region the
    /// pointer claim covers.
    pub(crate) fn captures(&self, position: Vec2) -> bool {
        self.region.is_some_and(|(_, region)| region.contains(position))
    }
}

/// Copies the gates of the surfaces that keep their own — the settings modal
/// and the content library, the two the field's input routes cover — into
/// the claims, at the point those routes read them. The control panel
/// publishes its own claim as it changes.
fn publish_field_gates(
    settings: Option<Res<crate::game_settings::SettingsPanel>>,
    library: Option<Res<crate::content_library::ContentLibrary>>,
    mut claims: ResMut<InputClaims>,
) {
    for (holder, held) in [
        (
            SETTINGS_PANEL,
            settings
                .as_deref()
                .is_some_and(crate::game_settings::SettingsPanel::blocks_world_input),
        ),
        (
            CONTENT_LIBRARY,
            library
                .as_deref()
                .is_some_and(crate::content_library::ContentLibrary::blocks_world_input),
        ),
    ] {
        if held {
            claims.claim(holder);
        } else {
            claims.release(holder);
        }
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<InputClaims>()
        .add_systems(Update, publish_field_gates.before(crate::site::read_switch));
}
