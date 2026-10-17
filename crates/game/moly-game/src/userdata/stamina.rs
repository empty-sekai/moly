//! The `mysekaiStaminas` master: the three pool maxima the stamina gauges
//! read. The harvest catalog reads the `StaminaMax` resource the shared
//! loader seats and records the full triple here on the way, so a gauge that
//! asks later gets it.
//!
//! The record is one-shot per process: without the table the maxima stay
//! unknown, and a gauge that needs one waits (as it does before the masters
//! resolve).

use std::sync::Mutex;

use moly_masters::StaminaMax;

static MAX: Mutex<Option<StaminaMax>> = Mutex::new(None);

/// Records the maxima the shared loader seated.
pub(crate) fn seat(max: StaminaMax) {
    if let Ok(mut slot) = MAX.lock() {
        *slot = Some(max);
    }
}

/// The maxima, once the table has resolved.
pub(crate) fn stamina_max() -> Option<StaminaMax> {
    MAX.lock().ok().and_then(|slot| *slot)
}
