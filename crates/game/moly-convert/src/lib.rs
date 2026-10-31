//! The convert screen: what the conversion machine offers and produces for
//! the player, and the screen layer 627 that shows it.
//!
//! Input: the convert master tables (`moly_masters::convert::ConvertMasters`),
//! the client's copies of the convert slots, item histories and convert item
//! rows (`moly_features::userdata::convert::ClientConvertSlots`), the
//! colourful pass and the machine level of the client's user data
//! (`moly_features::userdata::ClientUserData`), the holdings a consumption box
//! is paid from (`moly_features::userdata::inventory`), the screen and
//! dialog documents of the UI layout, and the server's answers on the link.
//!
//! Output: the slot page (`screen`, `slot`, `slot::cell`), the item
//! select page with its detail panel (`item_select`, `detail`) and the
//! three dialogs ([`dialogs`]: the start confirm 324, the skip 325 and the
//! item detail 284); the start and the skip requests, one in flight at a time
//! ([`start`]). [`model`] is the pure rule set behind all of them.
//!
//! Invariants: the model holds no Bevy resource, loads nothing and draws
//! nothing, so every number on screen comes from the masters, the copies and
//! the frame's clock. A dialog opens only while the layer plays and no other
//! convert dialog is up; a start or a skip request is sent only after its
//! dialog confirmed it. A request whose answer is not the convert reply runs
//! none of its callbacks. Every place where this crate has no data to draw or
//! send logs one named line instead of drawing or sending a made-up value.
//!
//! Boundaries: the crate reads the hub (`moly-features`) for the user data
//! copies and the game-wide input gate, and the lower crates for the rest
//! (the clock, the UI helpers and the master tables); it owns none of them.
//! The receive of a finished slot, the one-button pass dialogs, the crystal
//! shop, the tutorial slide and the effect prefabs are not built here.
//!
//! Frame order: the views spawn once their documents and data are in, the
//! layer's hooks follow the screen manager, then the current page's frame, the
//! back key and the pointer. The input reads the pointer events before the
//! gesture and pick steps that would otherwise take them.

pub mod model;
mod detail;
pub mod dialogs;
mod item_select;
mod screen;
mod slot;
pub mod start;

use bevy::prelude::*;

use moly_features::ui_layers;

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
                start::answers.after(moly_features::userdata::apply_arrivals),
            )
            // The dialogs' setup each frame: after their open (so the frame
            // that opens one draws it set up) and before the frame.
            .add_systems(
                Update,
                (dialogs::setup_start, dialogs::setup_shorten, dialogs::setup_detail)
                    .after(dialogs::open_start)
                    .after(dialogs::open_shorten)
                    .after(dialogs::open_detail)
                    .before(moly_ui::ui_layout::render),
            )
            // Their taps: the dialog slot is modal, so they take the pointer
            // before the shell and the screen's own pages, after the
            // gesture step.
            .add_systems(
                Update,
                dialogs::click
                    .run_if(moly_features::game_settings::scene_input_enabled)
                    .after(moly_ui::gesture::advance)
                    .before(moly_features::menu_shell::click)
                    .before(screen::input),
            )
            // Their back key: after the manager's reader wrote it, and after
            // the screen's own back key (which the dialog slot already
            // outranks).
            .add_systems(
                Update,
                dialogs::back_key
                    .after(moly_ui::ui_layers::back_key)
                    .after(screen::back_key),
            )
            // Their visibility each frame, after whatever opened or closed
            // them, before the frame.
            .add_systems(
                Update,
                dialogs::place.before(moly_ui::ui_layout::render),
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
                        .run_if(moly_features::game_settings::scene_input_enabled)
                        .after(screen::spawn_when_ready)
                        .after(moly_ui::gesture::advance)
                        .before(moly_features::menu_shell::click),
                    screen::advance
                        .after(screen::follow_screen)
                        .after(screen::input)
                        .before(moly_ui::ui_layout::render),
                ),
            );
    }
}
