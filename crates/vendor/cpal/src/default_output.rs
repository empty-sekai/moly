//! A process-wide replacement for the default output device.
//!
//! A program that renders its own output (to a file, at its own pace) sets a
//! custom device here before any library opens the default output; from then
//! on every host answers `default_output_device()` with that device. Setting
//! `None` returns to the host's own default device.

use std::sync::{Mutex, PoisonError};

use crate::platform::CustomDevice;

static DEFAULT_OUTPUT: Mutex<Option<CustomDevice>> = Mutex::new(None);

/// Makes `device` the default output device of every host, or restores the
/// hosts' own defaults with `None`.
pub fn set_default_output_device(device: Option<CustomDevice>) {
    *DEFAULT_OUTPUT.lock().unwrap_or_else(PoisonError::into_inner) = device;
}

/// The device set by [`set_default_output_device`], if any.
pub(crate) fn default_output_device() -> Option<CustomDevice> {
    DEFAULT_OUTPUT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}
