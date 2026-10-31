//! The mission screen (621) and its receive: the screen layer's view, its two
//! pages and the page turn ([`screen`]), the list cells ([`cell`]), the two
//! dialogs those cells open ([`list_dialog`], [`detail_dialog`]) and the one
//! receive request the screen sends (`MysekaiMissionService`, [`service`]).
//!
//! Input: the mission master tables
//! ([`moly_masters::mission::MissionMasters`]: sheets, missions, the
//! missions' reward rows and rank-experience rows), the client's mission copy
//! (`moly_features::userdata::mission::ClientMission`: the sheet the user is
//! on and the mission rows the server keeps) and the pure rules over them
//! ([`moly_features::userdata::mission::model`], which lives in the hub); the
//! screen, cell and dialog documents of the UI root; the pointer, the back
//! key and the server's answers on the link.
//!
//! Output: the screen view with both pages, drawn each frame; the dialog
//! slots of the list dialog (340) and the detail dialog (339); the receive
//! request and its [`service::MissionReceived`] /
//! [`service::MissionReceiveFailed`] answers; the dialog request a button or
//! row tap writes ([`cell::MissionDialogRequest`]).
//!
//! Invariants: the model decides what the screen shows and what a receive
//! asks for; it holds no Bevy resource, loads nothing and draws nothing. A
//! button or a row tap writes the dialog request the two dialogs read, which
//! push their dialog slot in the same frame and run their own setup on it
//! the next frame; their own taps and the back key run before the shell's.
//!
//! Frame order: the view spawns once its document is in; the layer's hooks
//! follow the screen manager; the receive's answers are applied to the copies
//! by the user data reader, then its callbacks run ([`service`]); the
//! screen's turn, content and buttons follow; the list cells are composed
//! after the view exists and drawn with it.
//!
//! Boundaries: the reward chain that follows a receive, the retry-or-cancel
//! dialog of a refused receive and the tutorial API variant are not built;
//! each is named where it would stand (see [`service`], [`screen`]).

pub mod cell;
pub mod detail_dialog;
pub mod list_dialog;
pub mod screen;
pub mod service;

use bevy::prelude::*;

use moly_features::ui_layers;

/// The mission screen, its list cells and its two dialogs.
pub struct MissionPlugin;

impl Plugin for MissionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<screen::MissionScreen>()
            .init_resource::<service::MysekaiMissionService>()
            .init_resource::<service::MissionLevelsBefore>()
            .init_resource::<cell::MissionCells>()
            .init_resource::<list_dialog::MissionListDialog>()
            .init_resource::<detail_dialog::MissionDetailDialog>()
            .init_resource::<detail_dialog::RewardCellSource>()
            .init_resource::<detail_dialog::CellSource>()
            .add_message::<service::MissionReceived>()
            .add_message::<service::MissionReceiveFailed>()
            .add_message::<cell::MissionDialogRequest>()
            // The levels the reply's sections compared, read after the
            // reader applied the arrival (no order against the later Update
            // systems is needed: PreUpdate runs first).
            .add_systems(
                PreUpdate,
                service::remember_levels.after(moly_features::userdata::apply_arrivals),
            )
            // The answer's callbacks: after the reader, before the frames
            // that draw the copies it changes.
            .add_systems(
                Update,
                (
                    service::answers.after(moly_features::userdata::apply_arrivals),
                    screen::spawn_when_ready,
                    screen::follow_screen.after(ui_layers::advance),
                    screen::follow_receive.after(service::answers),
                    // The pointer events the gesture step writes are read
                    // here first: a tap inside the mission screen belongs to
                    // its buttons, not to the field's own click handlers.
                    // The screen state it reads is the one `follow_screen`
                    // left: the shell's click runs before the manager steps,
                    // so this cannot wait for the step.
                    screen::input
                        .run_if(moly_features::game_settings::scene_input_enabled)
                        .after(moly_features::gesture::advance)
                        .before(moly_features::menu_shell::click),
                    screen::advance
                        .after(screen::follow_screen)
                        .after(screen::input)
                        .before(moly_features::ui_layout::render),
                ),
            )
            // The list cells: composed once the screen's view exists, drawing
            // with the screen each frame, its scroll read from the same
            // gesture step the screen's taps come from.
            .add_systems(
                Update,
                (
                    cell::spawn_cells
                        .after(screen::spawn_when_ready)
                        .before(screen::advance)
                        .before(cell::advance),
                    cell::scroll
                        .after(screen::input)
                        .before(screen::advance),
                    cell::advance
                        .after(screen::advance)
                        .after(cell::spawn_cells)
                        .after(cell::scroll)
                        .before(moly_features::ui_layout::render),
                ),
            )
            // The two dialogs: their documents install once the root has
            // them, before a tap asks for one; a request the screen's tap
            // wrote opens the slot in the same frame, before the layer stack
            // steps, and their `Setup` runs on the open slot the next frame;
            // their taps are the dialog slot's (before the shell's), their
            // back key follows the shared reader, and their visible state is
            // written after both and before the frame that draws them.
            .add_systems(
                Update,
                (
                    list_dialog::spawn_when_ready.before(list_dialog::open),
                    detail_dialog::spawn_when_ready.before(detail_dialog::open),
                    list_dialog::setup
                        .after(ui_layers::advance)
                        .after(list_dialog::spawn_when_ready)
                        .before(moly_features::ui_layout::render),
                    detail_dialog::setup
                        .after(ui_layers::advance)
                        .before(moly_features::ui_layout::render),
                    list_dialog::open
                        .after(screen::input)
                        .after(list_dialog::spawn_when_ready)
                        .after(list_dialog::click)
                        .after(list_dialog::back_key)
                        .before(ui_layers::advance),
                    detail_dialog::open
                        .after(screen::input)
                        .after(detail_dialog::spawn_when_ready)
                        .after(detail_dialog::click)
                        .after(detail_dialog::back_key)
                        .before(ui_layers::advance),
                    list_dialog::click
                        .run_if(moly_features::game_settings::scene_input_enabled)
                        .after(moly_features::action_button::click)
                        .before(moly_features::menu_shell::click),
                    detail_dialog::click
                        .run_if(moly_features::game_settings::scene_input_enabled)
                        .after(moly_features::action_button::click)
                        .before(moly_features::menu_shell::click),
                    list_dialog::back_key.after(ui_layers::back_key),
                    detail_dialog::back_key.after(ui_layers::back_key),
                ),
            )
            // The visible dialog each frame: after the request that opens it
            // and after its own close, before the frame that draws it.
            .add_systems(
                Update,
                (
                    list_dialog::place
                        .after(list_dialog::open)
                        .after(list_dialog::click)
                        .after(list_dialog::back_key)
                        .before(moly_features::ui_layout::render),
                    detail_dialog::place
                        .after(detail_dialog::open)
                        .after(detail_dialog::click)
                        .after(detail_dialog::back_key)
                        .before(moly_features::ui_layout::render),
                ),
            );
    }
}
