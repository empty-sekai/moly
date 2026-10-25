//! Plugins for the native game.
//!
//! A plugin is a directory with a `plugin.toml` ([`manifest`]). The profile
//! ([`profile`]) lists which plugins are enabled and their settings. At
//! startup [`resolve`] checks the enabled set against the running moly
//! version, dependencies, conflicts and the seams each plugin provides
//! ([`seam`]), and either returns the load order or refuses with every
//! problem named. Luau plugins then run in a sandbox ([`luau`]) that exposes
//! only the game's plugin interface.

pub mod manifest;
pub mod profile;
pub mod resolve;
pub mod seam;

#[cfg(not(target_arch = "wasm32"))]
pub mod luau;

pub use manifest::{Manifest, ManifestError, PluginId, PluginKind};
pub use profile::Profile;
pub use resolve::{resolve, Resolved, ResolveError};
pub use seam::{SeamDecl, SeamKind, SeamTable};
