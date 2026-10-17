//! The developer instruments the server reads from its own environment.
//!
//! A developer start (`MOLY_DEV=1`) may fix the clock, the screen pools, the
//! avatar wear, the music records and the harvest tools through the same
//! environment variables the native game reads in that mode. A normal start
//! reads none of them: the stored documents alone decide the values.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process is a developer start.
static DEV: AtomicBool = AtomicBool::new(false);

/// Reads `MOLY_DEV` once, when the server is built.
pub(crate) fn install() {
    DEV.store(dev_requested(), Ordering::Relaxed);
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_requested() -> bool {
    std::env::var("MOLY_DEV").ok().as_deref() == Some("1")
}

#[cfg(target_arch = "wasm32")]
fn dev_requested() -> bool {
    false
}

/// Whether the instruments are live.
pub fn dev() -> bool {
    DEV.load(Ordering::Relaxed)
}

/// One instrument's value; `None` outside a developer start.
pub(crate) fn instrument_env(name: &str) -> Option<String> {
    if !dev() {
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(name).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instruments_are_silent_outside_a_developer_start() {
        install();
        if !dev() {
            assert!(instrument_env("MOLY_MENU_MOCK_TOTAL_EXP").is_none());
        }
    }
}
