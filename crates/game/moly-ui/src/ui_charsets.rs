//! The glyph sets the field screens add to the one shared text atlas: the
//! shell's fixed wordings and site names, the collect notice's item names and
//! counts, and the BGM select screen's record texts. Each screen fills its own
//! set once, and the atlas takes them as union members.

use bevy::prelude::*;

/// The existing glyph bake waits for this fixed set of UI characters.
#[derive(Resource)]
pub struct ShellTextCharset {
    pub chars: Vec<char>,
}

/// The characters a collect-item cell can print: the atlas's fifth member.
#[derive(Resource, Default)]
pub struct CollectNoticeCharset {
    pub chars: Vec<char>,
}

/// The characters the screen can print (record titles, artists, vocal
/// texts, its wordings): a member of the shared glyph atlas.
#[derive(Resource, Default)]
pub struct BgmSelectCharset {
    pub chars: Vec<char>,
}

/// Characters the shell's set must take from screens that install themselves
/// (a frame list, master names). A screen declares its source when it
/// installs and fills it once its texts are read; the shell builds its set
/// only when every declared source is filled, so no glyph is missing from the
/// first bake. A source that is never declared adds nothing.
#[derive(Resource, Default)]
pub struct ShellTextSources {
    sources: std::collections::BTreeMap<&'static str, Option<Vec<char>>>,
}

impl ShellTextSources {
    /// Declares `name` as a source the shell waits for. Declaring a source
    /// again keeps what it holds.
    pub fn declare(&mut self, name: &'static str) {
        self.sources.entry(name).or_insert(None);
    }

    /// Fills `name` with its characters; the source counts as declared.
    pub fn fill(&mut self, name: &'static str, chars: Vec<char>) {
        self.sources.insert(name, Some(chars));
    }

    /// Whether `name` has been filled.
    pub fn is_filled(&self, name: &str) -> bool {
        self.sources.get(name).is_some_and(Option::is_some)
    }

    /// Every source's characters, or `None` while a declared source is
    /// still empty.
    pub fn chars(&self) -> Option<impl Iterator<Item = char> + '_> {
        if self.sources.values().any(Option::is_none) {
            return None;
        }
        Some(self.sources.values().flatten().flatten().copied())
    }
}
