//! The harvest action button: `ScreenLayerMysekaiHarvest`'s ButtonAction
//! (`HarvestActionButton` + `HarvestButtonInteraction`), drawn from the
//! same serialized prefab as the harvest field menu, with only the button
//! subtree visible.
//!
//! - `CustomButton` forwards pointer-down to `OnPressed` only while it is
//!   interactable; `OnPressed` sets `IsPress` (read as
//!   `HarvestActionButton.IsLongTap`), turns `ActiveCover` on and invokes the
//!   click; `OnReleased` clears `IsPress`.
//! - `UpdateHarvestUI` is the only writer of interactable
//!   (`HasToolRequiredForHarvest`); the cover shows when the tool is missing
//!   or the stamina does not cover the next attack; ButtonActionCannot is
//!   active when the tool is missing.
//! - The F key is a named stand-in for the button on a desktop: pressing it
//!   is a pointer-down, holding it is a held button.
//!
//! Named gaps: the button's runtime icon (`icon`, a texture the harvest
//! screen sets per kind) and its balloon text are not supplied to this view
//! and stay hidden; the tool name, tool selector and selected-tool views of
//! the same screen belong to the harvest screen's own presenters and stay
//! hidden; ButtonActionCannot's click has no reader here.

use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::balloon::{canvas_scale, BALLOON_LAYER};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::ui_layout::{UiLayouts, UiPrefabView};

const KEY: &str = "ShellHarvest";
const BUTTON: &str = "ComponentRoot/RightBottom/ButtonAction";

/// The button's state as the harvest presenter writes and reads it.
#[derive(Resource, Default, Debug)]
pub(crate) struct HarvestButton {
    /// `ShowHarvestUI` / `HideHarvestUI`.
    pub(crate) shown: bool,
    /// `SetActionButtonEnable`.
    pub(crate) interactable: bool,
    /// `SetCover`.
    pub(crate) cover: bool,
    /// ButtonActionCannot active.
    pub(crate) cannot: bool,
    /// `IsPress` (`IsLongTap`).
    pub(crate) is_press: bool,
    /// `OnPressed` clicks raised this frame (drained by the action loop).
    pub(crate) presses: u32,
    pointer: bool,
    key: bool,
}

impl HarvestButton {
    pub(crate) fn hide(&mut self) {
        self.shown = false;
    }
}

#[derive(Component)]
pub(crate) struct HarvestButtonView {
    button: String,
    active_cover: String,
    cover: String,
    cannot: String,
}

#[derive(Resource)]
pub(crate) struct HarvestButtonSpawned;

/// Update: build the button view once the prefab is loaded.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<HarvestButtonSpawned>>,
) {
    if spawned.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let doc = layouts.document(KEY).expect("ready harvest prefab");
    let index = doc.find(BUTTON).unwrap_or_else(|error| panic!("{error}"));
    let button_path = doc.nodes[index].path.clone();
    let identity = |suffix: &str| -> String {
        let path = format!("{button_path}/{suffix}");
        let node = doc
            .nodes
            .iter()
            .find(|node| node.path == path)
            .unwrap_or_else(|| panic!("harvest action button child missing: {suffix}"));
        format!("@{}", node.game_object_id)
    };
    let mut view = UiPrefabView::new(KEY, BALLOON_LAYER);
    let mut hidden = 0usize;
    for node in &doc.nodes {
        let ancestor = button_path.starts_with(&format!("{}/", node.path));
        let inside = node.path == button_path || node.path.starts_with(&format!("{button_path}/"));
        if !ancestor && !inside {
            view.set_visible(&format!("@{}", node.game_object_id), false);
            hidden += 1;
        }
    }
    let marks = HarvestButtonView {
        button: format!("@{}", doc.nodes[index].game_object_id),
        active_cover: identity("ActiveCover"),
        cover: identity("Cover"),
        cannot: {
            let path = format!(
                "{}/ButtonActionCannot",
                button_path.rsplit_once('/').expect("button parent").0
            );
            let node = doc
                .nodes
                .iter()
                .find(|node| node.path == path)
                .expect("ButtonActionCannot beside the action button");
            format!("@{}", node.game_object_id)
        },
    };
    view.set_visible(&identity("icon"), false);
    view.set_visible(&identity("Balloon"), false);
    view.set_visible(&marks.active_cover, false);
    info!(
        "[harvest-ui] action button view: {BUTTON} of {KEY}; {hidden} other nodes hidden; icon and balloon hidden (runtime icon and wording not supplied)"
    );
    commands.spawn((
        marks,
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
        view,
    ));
    commands.insert_resource(HarvestButtonSpawned);
}

fn visible_now(
    button: &HarvestButton,
    stack: &crate::ui_layers::UiLayerStack,
    entry: Option<&crate::entry::EntrySequence>,
    site: Option<&crate::site::SiteActive>,
) -> bool {
    button.shown
        && crate::entry::hud_open(entry)
        && stack.on_field()
        && site.is_some_and(|site| site.category == "harvest")
}

/// Update: pointer-down on the button and the F key raise `OnPressed`;
/// release clears `IsPress`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_input(
    mut gestures: MessageReader<GestureEvent>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    views: Query<(&HarvestButtonView, &UiPrefabView)>,
    (stack, entry, site): (
        Res<crate::ui_layers::UiLayerStack>,
        Option<Res<crate::entry::EntrySequence>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    eligibility: crate::interaction::InteractionEligibility,
    mut button: ResMut<HarvestButton>,
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let visible = visible_now(&button, &stack, entry.as_deref(), site.as_deref())
        && eligibility.field_input_open();
    if !visible {
        button.pointer = false;
        button.key = false;
        button.is_press = false;
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = canvas_scale(window.width(), window.height());
    let size = Vec2::new(window.width(), window.height()) / scale;
    let over_button = |position: Vec2| -> bool {
        let canvas = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        views.iter().any(|(marks, view)| {
            view.rect(&layouts, &marks.button, size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        })
    };
    for event in &events {
        match event.state {
            GestureState::Began
                if event.kind == GestureKind::Tap && over_button(event.position) =>
            {
                if button.interactable {
                    button.pointer = true;
                    button.presses += 1;
                    info!(
                        "[harvest-ui] action button pressed at ({:.0},{:.0})",
                        event.position.x, event.position.y
                    );
                }
            }
            GestureState::End if button.pointer => {
                button.pointer = false;
            }
            _ => {}
        }
    }
    // A pointer that went away without an End (focus loss, a cancelled tap)
    // is a release as well.
    if button.pointer && !mouse.pressed(MouseButton::Left) && touches.iter().next().is_none() {
        button.pointer = false;
    }
    if keys.just_pressed(KeyCode::KeyF) && button.interactable {
        button.presses += 1;
        button.key = true;
        info!("[harvest-ui] action button pressed by the F key stand-in");
    }
    if !keys.pressed(KeyCode::KeyF) {
        button.key = false;
    }
    let press = button.pointer || button.key;
    if press != button.is_press {
        button.is_press = press;
    }
}

/// Update: place and dress the view.
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    button: Res<HarvestButton>,
    (stack, entry, site): (
        Res<crate::ui_layers::UiLayerStack>,
        Option<Res<crate::entry::EntrySequence>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    mut views: Query<(
        &HarvestButtonView,
        &mut Visibility,
        &mut Transform,
        &mut UiPrefabView,
    )>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = canvas_scale(window.width(), window.height());
    let visible = visible_now(&button, &stack, entry.as_deref(), site.as_deref());
    for (marks, mut visibility, mut transform, mut view) in &mut views {
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
        view.set_visible(&marks.active_cover, button.is_press);
        view.set_visible(&marks.cover, button.cover);
        view.set_visible(&marks.cannot, button.cannot);
    }
}
