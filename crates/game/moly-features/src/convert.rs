//! The convert screen's data model and its screen: the pure rules over the
//! convert master tables (`moly_masters::convert::ConvertMasters`) and a
//! plain user state ([`model`]), and the screen layer 627 itself
//! ([`screen`], [`slot`], [`item_select`] and [`detail`]). The start and the
//! skip chains and the one request each sends are [`start`]; the screen's
//! dialogs (284, 324, 325) and its receive chain are not built yet: each call
//! site stands as one named line.
//!
//! The model decides what the screen shows and what a start asks for; it
//! holds no Bevy resource, loads nothing and draws nothing. The user data is
//! the slot rows, the items' histories and the item rows the server sends
//! (the client's copy of them is `userdata::convert::ClientConvertSlots`),
//! plus what a convert spends (`userdata::client::inventory`).
//!
//! Order inside a frame: the views spawn once their documents and data are
//! in, the layer's hooks follow the screen manager, then the current page's
//! frame, the back key and the pointer. The input reads the pointer events
//! before the gesture and pick steps that would otherwise take them.

pub mod model;
mod detail;
pub mod dialogs;
mod item_select;
mod screen;
mod slot;
pub mod start;

use bevy::prelude::*;

use crate::ui_layers;

/// The convert screen (627).
pub struct ConvertPlugin;

impl Plugin for ConvertPlugin {
    fn build(&self, app: &mut App) {
        // The back key answers before the manager's own back-key path (which
        // writes its pop request) and before the manager steps.
        app.add_systems(
            Update,
            screen::back_key
                .before(ui_layers::back_key)
                .before(ui_layers::advance),
        );
        app.init_resource::<screen::ConvertScreen>()
            .init_resource::<screen::ConvertPress>()
            .init_resource::<start::MysekaiConvertService>()
            .init_resource::<dialogs::ConvertDialogs>()
            // The dialogs' documents install once the root has them: one
            // system, ordered after nothing that runs after the layer step,
            // so the dialogs' `open` (before the step) cannot form a cycle
            // with it.
            .add_systems(Update, dialogs::spawn_when_ready.before(dialogs::open_start))
            // The dialogs open only while the layer plays, which is only ever
            // true after the step (`update`): each `open` needs no order
            // against `spawn_when_ready` and takes its order from the frame.
            .add_systems(
                Update,
                (
                    dialogs::open_start,
                    dialogs::open_shorten,
                    dialogs::open_detail,
                ),
            )
            // The start or the skip a dialog's OK asked for is sent after the
            // screen's own taps wrote the request and after the dialogs that
            // answer them, so a pass dialog the start shows waits for it.
            .add_systems(
                Update,
                start::send_pending
                    .after(screen::input)
                    .after(dialogs::open_start)
                    .after(dialogs::open_shorten),
            )
            // The requests' answers: after the reader applied the answer's
            // sections to the copies, before the frames that draw them.
            .add_systems(
                Update,
                start::answers.after(crate::userdata::apply_arrivals),
            )
            // The dialogs' setup each frame: after their open (so the frame
            // that opens one draws it set up) and before the frame.
            .add_systems(
                Update,
                (dialogs::setup_start, dialogs::setup_shorten, dialogs::setup_detail)
                    .after(dialogs::open_start)
                    .after(dialogs::open_shorten)
                    .after(dialogs::open_detail)
                    .before(crate::ui_layout::render),
            )
            // Their taps: the dialog slot is modal, so they take the pointer
            // before the shell and the screen's own pages, after the
            // gesture step.
            .add_systems(
                Update,
                dialogs::click
                    .run_if(crate::game_settings::scene_input_enabled)
                    .after(crate::gesture::advance)
                    .before(crate::menu_shell::click)
                    .before(screen::input),
            )
            // Their back key: after the manager's reader wrote it, and after
            // the screen's own back key (which the dialog slot already
            // outranks).
            .add_systems(
                Update,
                dialogs::back_key
                    .after(crate::ui_layers::back_key)
                    .after(screen::back_key),
            )
            // Their visibility each frame, after whatever opened or closed
            // them, before the frame.
            .add_systems(
                Update,
                dialogs::place.before(crate::ui_layout::render),
            )
            .add_systems(
                Update,
                (
                    screen::spawn_when_ready,
                    screen::follow_screen
                        .after(screen::spawn_when_ready)
                        .after(ui_layers::advance),
                    // The pointer events the gesture step writes are read
                    // here first: a tap inside the convert screen belongs to
                    // its slot page, not to the field's own click handlers.
                    // The screen state it reads is the one `follow_screen`
                    // left: the shell's click runs before the manager steps,
                    // so this cannot wait for the step.
                    screen::input
                        .run_if(crate::game_settings::scene_input_enabled)
                        .after(screen::spawn_when_ready)
                        .after(crate::gesture::advance)
                        .before(crate::menu_shell::click),
                    screen::advance
                        .after(screen::follow_screen)
                        .after(screen::input)
                        .before(crate::ui_layout::render),
                ),
            );
    }
}
