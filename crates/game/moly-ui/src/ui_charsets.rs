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
