//! Seams: the replaceable parts of the game.
//!
//! The game declares each seam once. The game's own implementation is the
//! default provider of every seam; a plugin that names a seam in
//! `[[provides]]` replaces it, for the whole seam or for one target.
//!
//! - An [`SeamKind::Exclusive`] seam has exactly one provider per target.
//!   Two enabled plugins providing the same seam and target is a startup
//!   refusal, unless the profile names which one wins.
//! - A [`SeamKind::Multi`] seam takes any number of providers, run in load
//!   order (event observers, tick hooks).

use std::collections::BTreeMap;

use crate::manifest::PluginId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamKind {
    Exclusive,
    Multi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeamDecl {
    pub name: &'static str,
    pub kind: SeamKind,
    /// Whether providers may name a `target` (for example one character).
    pub per_target: bool,
    pub doc: &'static str,
}

/// Who provides a seam (or one of its targets) after resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provider {
    /// The game's own implementation.
    Game,
    Plugin(PluginId),
}

impl Provider {
    /// The provider's name: the game's, or the plugin's id.
    pub fn name(&self) -> String {
        match self {
            Self::Game => "game".to_owned(),
            Self::Plugin(plugin) => plugin.to_string(),
        }
    }
}

/// The resolved providers. For an exclusive seam, `whole` is the provider of
/// the seam and `targets` the per-target overrides; for a multi seam,
/// `multi` lists the plugin providers in load order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeamTable {
    pub exclusive: BTreeMap<&'static str, ExclusiveSeam>,
    pub multi: BTreeMap<&'static str, Vec<PluginId>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExclusiveSeam {
    pub whole: Provider,
    pub targets: BTreeMap<String, Provider>,
}

impl SeamTable {
    /// The provider of an exclusive seam for `target`: the target override,
    /// else the seam's provider, else the game.
    pub fn provider(&self, seam: &str, target: Option<&str>) -> Provider {
        let Some(entry) = self.exclusive.get(seam) else {
            return Provider::Game;
        };
        target
            .and_then(|target| entry.targets.get(target))
            .unwrap_or(&entry.whole)
            .clone()
    }
}

/// The seams the game declares.
pub const SEAMS: &[SeamDecl] = &[
    SeamDecl {
        name: "companion.brain",
        kind: SeamKind::Exclusive,
        per_target: true,
        doc: "What an AI player does next while no human controls it. Target: `unit:<id>` of the character it wears.",
    },
    SeamDecl {
        name: "npc.objective",
        kind: SeamKind::Exclusive,
        per_target: true,
        doc: "A character's next objective. Target: `unit:<id>`.",
    },
    SeamDecl {
        name: "game.tick",
        kind: SeamKind::Multi,
        per_target: false,
        doc: "Called once per simulation frame.",
    },
    SeamDecl {
        name: "game.events",
        kind: SeamKind::Multi,
        per_target: false,
        doc: "Receives the game's event stream.",
    },
];

pub fn declared(name: &str) -> Option<&'static SeamDecl> {
    SEAMS.iter().find(|seam| seam.name == name)
}
