//! The clean view: two switches that take the game's own furniture off the
//! screen for a recording or a photograph. The players and plugins ask for
//! them with `view.set`, the configuration with `display.hide_hud` and
//! `display.hide_player` (`MOLY_HIDE_HUD`, `MOLY_HIDE_PLAYER`).
//!
//! - `hide_hud` hides every UI view but the talk window, so only the scene,
//!   the in-world balloons and a conversation remain.
//! - `hide_player` hides the player's own body: the field camera stands where
//!   the player stands, so a first-person shot would have the body (and its
//!   hair) fill the frame.
//!
//! A view the switch hid is shown again when the switch goes off, and only
//! that view: the entities the switch hid are kept, so a view the game hid
//! itself stays hidden.

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use std::collections::HashSet;

use crate::player::PlayerControlled;
use crate::talk_window::TalkWindowView;
use crate::ui_layout::UiPrefabView;

/// The clean view's two switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewSwitches {
    hide_hud: bool,
    hide_player: bool,
}

impl ViewSwitches {
    /// The HUD switch: on when every HUD view but the talk window is hidden.
    pub fn hud(&self) -> bool {
        self.hide_hud
    }

    /// The player switch: on when the player's own body is hidden.
    pub fn player(&self) -> bool {
        self.hide_player
    }
}

/// The clean view as the game holds it.
#[derive(Resource, Default)]
pub struct CleanView {
    switches: ViewSwitches,
    /// The entities the switches hid.
    hidden: HashSet<Entity>,
}

impl CleanView {
    pub fn switches(&self) -> ViewSwitches {
        self.switches
    }

    /// The switches as `view.set` names them: each of `hud` and `player`
    /// changes only when given.
    pub fn set(&mut self, hud: Option<bool>, player: Option<bool>) {
        if let Some(hide) = hud {
            self.switches.hide_hud = hide;
        }
        if let Some(hide) = player {
            self.switches.hide_player = hide;
        }
    }

    /// The state as `view.set`'s note names it.
    pub fn note(&self) -> String {
        format!(
            "view: hud {}, player {}",
            if self.switches.hide_hud { "hidden" } else { "shown" },
            if self.switches.hide_player { "hidden" } else { "shown" }
        )
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<CleanView>().add_systems(
        PostUpdate,
        (hide_views, hide_player_body)
            .chain()
            .before(VisibilitySystems::VisibilityPropagate),
    );
}

/// The player's own body: a hidden root propagates to every mesh under it.
fn hide_player_body(
    mut clean: ResMut<CleanView>,
    mut players: Query<(Entity, &mut Visibility), With<PlayerControlled>>,
) {
    let hide = clean.switches.hide_player;
    let hidden = std::mem::take(&mut clean.hidden);
    let mut hidden = hidden;
    for (entity, mut visibility) in &mut players {
        apply(&mut hidden, entity, &mut visibility, hide);
    }
    clean.hidden = hidden;
}

fn hide_views(
    mut clean: ResMut<CleanView>,
    mut views: Query<(Entity, &mut Visibility), (With<UiPrefabView>, Without<TalkWindowView>)>,
) {
    let hide = clean.switches.hide_hud;
    let mut hidden = std::mem::take(&mut clean.hidden);
    for (entity, mut visibility) in &mut views {
        apply(&mut hidden, entity, &mut visibility, hide);
    }
    clean.hidden = hidden;
}

/// One entity for one switch: hidden while the switch is on, and shown again
/// once it is off when this switch hid it.
fn apply(hidden: &mut HashSet<Entity>, entity: Entity, visibility: &mut Mut<Visibility>, hide: bool) {
    if hide {
        if **visibility != Visibility::Hidden {
            hidden.insert(entity);
        }
        visibility.set_if_neq(Visibility::Hidden);
    } else if hidden.remove(&entity) {
        visibility.set_if_neq(Visibility::Inherited);
    }
}
