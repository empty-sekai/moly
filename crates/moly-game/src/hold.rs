//! Named holds for per-frame systems that yield their frame until an input
//! is in.
//!
//! A system that waits on a resource, an asset or a readiness flag returns
//! early on every frame the input is missing. When the input never arrives
//! the feature stands still with nothing on the screen or in the log to say
//! why. A [`HoldReport`] kept in the system's `Local` names the hold when its
//! reason changes, warns once when one reason has held for
//! [`WARN_AFTER_SECS`], and names the release with how long it held.

use bevy::prelude::*;

/// A hold of one reason this long is reported as a warning.
pub(crate) const WARN_AFTER_SECS: f32 = 10.0;

/// Where a hold is reported from and what it stops.
pub(crate) struct HoldSite {
    /// The log tag, without brackets.
    pub(crate) tag: &'static str,
    /// What holds, as the subject of the log line ("decisions").
    pub(crate) what: &'static str,
    /// What the user sees while it holds ("every NPC stands").
    pub(crate) effect: &'static str,
    /// Reasons expected to last (an edit session): named, never warned.
    pub(crate) quiet: &'static [&'static str],
}

/// The current hold of one system.
#[derive(Default)]
pub(crate) struct HoldReport {
    reason: Option<&'static str>,
    since: f32,
    warned: bool,
}

impl HoldReport {
    /// The system yields this frame for `reason`.
    pub(crate) fn hold(&mut self, site: &HoldSite, reason: &'static str, now: f32) {
        if self.reason != Some(reason) {
            info!("[{}] {} hold: {reason}", site.tag, site.what);
            *self = Self {
                reason: Some(reason),
                since: now,
                warned: false,
            };
        } else if !self.warned
            && !site.quiet.contains(&reason)
            && now - self.since >= WARN_AFTER_SECS
        {
            warn!(
                "[{}] {} have held for {:.0} s, {}: {reason}",
                site.tag,
                site.what,
                now - self.since,
                site.effect
            );
            self.warned = true;
        }
    }

    /// The system runs this frame.
    pub(crate) fn run(&mut self, site: &HoldSite, now: f32) {
        if let Some(reason) = self.reason.take() {
            info!(
                "[{}] {} run after holding {:.1} s: {reason}",
                site.tag,
                site.what,
                now - self.since
            );
        }
    }
}
