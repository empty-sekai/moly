//! Developer scaffolding switch. The native window and the legacy browser
//! pages insert [`DevTools`]; the browser game never does. Every developer
//! surface that has no counterpart in the game registers behind
//! [`dev_tools`], or asks [`installed`] when it is a plain function.

use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Present while developer scaffolding is part of this application.
#[derive(Resource)]
pub struct DevTools;

/// Mirror of the resource for code that runs without a World borrow.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Called before `App::run`; the flag never changes in a running world.
pub fn insert_dev_tools(app: &mut App) {
    app.insert_resource(DevTools);
    INSTALLED.store(true, Ordering::Relaxed);
}

/// Run condition of every gated registration.
pub(crate) fn dev_tools(tools: Option<Res<DevTools>>) -> bool {
    tools.is_some()
}

/// Gate for plain functions that have no World access.
pub(crate) fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}
