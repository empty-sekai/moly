//! The line: the document's dialogue in the game's own talk window.
//!
//! The player writes the window's state directly (`TalkWindowState`): the
//! speaker's name on the name plate and the text's first `shown` characters in
//! the body, with no typewriter of the window's own, since the document's
//! reveal decides how much of the line shows at each time. The window has no
//! player or paired session behind it, so its taps set no latch, and the end
//! sign shows once the whole line is written.

use bevy::prelude::*;
use moly_features::balloon::BalloonArt;
use moly_features::talk_window::TalkWindowState;
use moly_sequence::doc::Track;
use moly_sequence::{LineFrame, Sequence};

use crate::Player;

/// Refuses a document whose names or lines hold characters the talk window
/// cannot draw: the window draws from the game's baked text atlas, which holds
/// the game's own character set.
pub(crate) fn refuse_missing_glyphs(world: &World, sequence: &Sequence) -> Result<(), String> {
    let lines: Vec<_> = sequence
        .tracks
        .iter()
        .filter_map(|track| match track {
            Track::Line(track) => Some(track),
            _ => None,
        })
        .flat_map(|track| &track.clips)
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    let Some(art) = world.get_resource::<BalloonArt>() else {
        return Err("the talk window's text atlas is not baked yet".to_owned());
    };
    let mut missing: Vec<char> = lines
        .iter()
        .flat_map(|line| line.name.chars().chain(line.text.chars()))
        .filter(|ch| *ch != '\n' && art.advance(*ch).is_none())
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} characters of the lines are outside the talk window's text atlas: {}",
        missing.len(),
        missing.iter().collect::<String>()
    ))
}

/// The line last written.
pub(crate) struct Shown {
    pub index: usize,
    pub name: String,
    pub shown: usize,
}

/// Writes the line up at this frame, or takes the window away when there is
/// none. A client without the talk window keeps playing without the line.
pub fn lay(world: &mut World, player: &mut Player, frame: Option<LineFrame<'_>>) {
    let changes = match (&player.line, &frame) {
        (Some(shown), Some(frame)) => shown.index != frame.index || shown.shown != frame.shown || shown.name != frame.name,
        (None, None) => false,
        _ => true,
    };
    if !changes {
        return;
    }
    let Some(mut window) = world.get_resource_mut::<TalkWindowState>() else {
        player.line = frame.map(|frame| Shown { index: frame.index, name: frame.name.to_owned(), shown: frame.shown });
        return;
    };
    match frame {
        Some(frame) => {
            window.show_written_line(frame.name, frame.text, frame.shown);
            player.line = Some(Shown { index: frame.index, name: frame.name.to_owned(), shown: frame.shown });
        }
        None => {
            window.withdraw_written_line();
            player.line = None;
        }
    }
}

/// Takes the window away at a seek or a stop.
pub fn hide(world: &mut World, player: &mut Player) {
    if player.line.take().is_none() {
        return;
    }
    if let Some(mut window) = world.get_resource_mut::<TalkWindowState>() {
        window.withdraw_written_line();
    }
}
