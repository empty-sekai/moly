use bevy::prelude::*;

/// The saved profile's total experience, seated where the rank gauge and the
/// server responses read it. [`saved_total_exp`] reads the settings document;
/// this resource holds that answer so a reader without the document's
/// ownership reads it too, and it is seated again whenever an import
/// replaces the document.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct SavedTotalExp(pub Option<i32>);
