//! The harvest screen (`ScreenLayerMysekaiHarvest`) without its field menu
//! and dash button (those have their own views): the action button
//! (`HarvestActionButton` + `HarvestButtonInteraction`), its action icon
//! and tool-missing balloon, the cannot button, and the tool views
//! ([`super::tool_view`]), drawn from the serialized prefab with the other
//! subtrees hidden.
//!
//! - `CustomButton` forwards pointer-down to `OnPressed` only while it is
//!   interactable; `OnPressed` sets `IsPress` (read as
//!   `HarvestActionButton.IsLongTap`), turns `ActiveCover` on and invokes the
//!   click; `OnReleased` clears `IsPress`.
//! - `SetActive(active)`: the button's interactable and its CanvasGroup
//!   alpha (1 or 0). `ShowHarvestUI` runs `UpdateToolView`, then
//!   `UpdateToolIcon(fixtureType)` and, once that returns, `SetActive(true)`.
//!   `UpdateToolIcon` sets the raw image `icon` to the texture
//!   `GetActionButtonName(type)` names, from the screen's cache or else by
//!   loading bundle `mysekai/icon/action_icon/<name>`; the name table (a
//!   10-entry pointer table over three names, the same in both regions): wood
//!   `icon_action_tree_wh`, mineral `icon_action_rock_wh`, every
//!   other kind `icon_action_lostitem_wh`. An uncached icon therefore shows
//!   the button (and makes it interactable) only when the load returns,
//!   after `UpdateHarvestUI`'s own writes of that frame.
//! - `UpdateHarvestUI` is the other writer of interactable
//!   (`HasToolRequiredForHarvest`); the cover shows when the tool is missing
//!   or the stamina does not cover the next attack; ButtonActionCannot is
//!   active when the tool is missing, and its click runs
//!   `OnHarvestActionCannot`: the balloon (`UIPartsBalloon`, the
//!   `_toolNotHaveBalloon`) takes `MessageText = MSG_NOT_HAVE_TOOL` and
//!   `Enable = true` (alpha 1), then alpha 0 and `DOFade(1, 0.1)`, a 2 s
//!   delay, alpha 0 and `DOFade(1, 0.1)` again, then `Enable = false`
//!   (alpha 0); both fades end at 1 (the binary: `fmov s0, #1.0` at
//!   both sites, the 0.1 from the literal pool), on the tween settings'
//!   default ease (OutQuad). A second click cancels the running one's
//!   delay: the newer run governs the alpha.
//! - The F key is a named stand-in for the button on a desktop: pressing it
//!   is a pointer-down, holding it is a held button.
//!
//! The root file of the lostitem icon is spelled `icon_action_lostItem_wh`
//! (the source texture's `m_Name`; bundle asset lookup is by that name).
//! The balloon's `FitText` does nothing with the serialized `minSize` (0, 0);
//! its size is the content size fitters'.

use std::collections::HashSet;

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::tool_view::{ToolViewMarks, ToolViewState};
use crate::audio::SeRequests;
use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::ui_layout::{UiLayouts, UiPrefabView};

const KEY: &str = "ShellHarvest";
const BUTTON: &str = "ComponentRoot/RightBottom/ButtonAction";
const CANNOT: &str = "ComponentRoot/RightBottom/ButtonActionCannot";
const BALLOON_WORDING: &str = "MSG_NOT_HAVE_TOOL";
const BALLOON_FADE: f32 = 0.1;
const BALLOON_HOLD: f32 = 2.0;

/// `GetActionButtonName(type)` (see the module comment).
pub(crate) fn action_button_name(fixture_type: i32) -> &'static str {
    match fixture_type {
        0 => "icon_action_tree_wh",
        1 => "icon_action_rock_wh",
        2..=9 => "icon_action_lostitem_wh",
        other => panic!(
            "GetActionButtonName: fixture type {other} is outside the table (the source throws ArgumentOutOfRangeException)"
        ),
    }
}

/// The three icons and their files on the root.
const ACTION_ICONS: [(&str, &str); 3] = [
    ("icon_action_tree_wh", "icon_action_tree_wh"),
    ("icon_action_rock_wh", "icon_action_rock_wh"),
    ("icon_action_lostitem_wh", "icon_action_lostItem_wh"),
];

/// DOTween's OutQuad (the tween settings' default ease).
fn out_quad(t: f32) -> f32 {
    -t * (t - 2.0)
}

fn icon_alias(name: &str) -> String {
    format!("harvest-action-icon/{name}")
}

/// `OnHarvestActionCannot`'s balloon run.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum BalloonStage {
    #[default]
    Idle,
    /// The first `DOFade(1, 0.1)`.
    FadeIn,
    /// The 2 s delay.
    Hold,
    /// The second `DOFade(1, 0.1)`, then `Enable = false`.
    Refade,
}

#[derive(Debug, Default)]
struct BalloonRun {
    stage: BalloonStage,
    /// Seconds in the stage; `None` in the frame the stage started (the
    /// tween's first update is the next frame).
    elapsed: Option<f32>,
}

impl BalloonRun {
    fn start(&mut self) {
        self.stage = BalloonStage::FadeIn;
        self.elapsed = None;
    }

    /// Advance by `dt`; the balloon's CanvasGroup alpha.
    fn advance(&mut self, dt: f32) -> f32 {
        let fade = |t: f32| out_quad((t / BALLOON_FADE).clamp(0.0, 1.0));
        loop {
            let t = match self.elapsed {
                None => {
                    self.elapsed = Some(0.0);
                    return match self.stage {
                        BalloonStage::Idle => 0.0,
                        BalloonStage::Hold => 1.0,
                        BalloonStage::FadeIn | BalloonStage::Refade => 0.0,
                    };
                }
                Some(t) => t + dt,
            };
            self.elapsed = Some(t);
            match self.stage {
                BalloonStage::Idle => return 0.0,
                BalloonStage::FadeIn if t >= BALLOON_FADE => {
                    self.stage = BalloonStage::Hold;
                    self.elapsed = Some(t - BALLOON_FADE);
                    return 1.0;
                }
                BalloonStage::FadeIn => return fade(t),
                BalloonStage::Hold if t >= BALLOON_HOLD => {
                    // alpha 0, then the second fade from this frame.
                    self.stage = BalloonStage::Refade;
                    self.elapsed = None;
                    info!("[harvest-ui] tool-missing balloon: 2 s passed; alpha 0 and DOFade(1, 0.1) again");
                }
                BalloonStage::Hold => return 1.0,
                BalloonStage::Refade if t >= BALLOON_FADE => {
                    self.stage = BalloonStage::Idle;
                    info!("[harvest-ui] tool-missing balloon: Enable = false");
                    return 0.0;
                }
                BalloonStage::Refade => return fade(t),
            }
        }
    }
}

/// The screen's state as the harvest presenter writes and reads it.
#[derive(Resource, Default, Debug)]
pub(crate) struct HarvestButton {
    /// The button's `SetActive` (its CanvasGroup alpha 1).
    pub(crate) shown: bool,
    /// `SetActionButtonEnable` / `SetActive`.
    pub(crate) interactable: bool,
    /// `SetCover`.
    pub(crate) cover: bool,
    /// ButtonActionCannot active.
    pub(crate) cannot: bool,
    /// `IsPress` (`IsLongTap`).
    pub(crate) is_press: bool,
    /// `OnPressed` clicks raised this frame (drained by the action loop).
    pub(crate) presses: u32,
    /// The tool views.
    pub(crate) tools: ToolViewState,
    /// `UpdateToolIcon` awaiting its load (then `SetActive(true)`).
    icon_request: Option<&'static str>,
    /// The icon on the raw image.
    icon: Option<&'static str>,
    /// `_toolIconCache`: icons this screen instance has loaded.
    icon_cache: HashSet<&'static str>,
    /// The site of this screen instance.
    screen_site: Option<u32>,
    balloon: BalloonRun,
    cannot_presses: u32,
    pointer: bool,
    key: bool,
}

impl HarvestButton {
    /// `HideHarvestUI`: the selector off, the selected-tool view hidden, the
    /// button's `SetActive(false)`.
    pub(crate) fn hide(&mut self) {
        self.tools.hide();
        self.shown = false;
        self.interactable = false;
    }

    /// `ShowHarvestUI`'s `UpdateToolIcon(type)` and `SetActive(true)`.
    pub(crate) fn show_harvest_ui(&mut self, fixture_type: i32) {
        let name = action_button_name(fixture_type);
        if self.icon_cache.contains(name) {
            self.icon = Some(name);
            self.icon_request = None;
            self.shown = true;
            self.interactable = true;
        } else {
            self.icon_request = Some(name);
        }
    }
}

#[derive(Component)]
pub(crate) struct HarvestButtonView {
    button: String,
    active_cover: String,
    cover: String,
    cannot: String,
    icon: String,
    balloon: String,
    balloon_text: String,
    tools: ToolViewMarks,
    icons: Vec<(&'static str, Handle<Image>)>,
}

#[derive(Resource)]
pub(crate) struct HarvestButtonSpawned;

/// Update: build the view once the prefab is loaded.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<HarvestButtonSpawned>>,
) {
    if spawned.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let doc = layouts.document(KEY).expect("ready harvest prefab");
    let index = doc.find(BUTTON).unwrap_or_else(|error| panic!("{error}"));
    let button_path = doc.nodes[index].path.clone();
    let cannot_index = doc.find(CANNOT).unwrap_or_else(|error| panic!("{error}"));
    let identity = |suffix: &str| -> String {
        let path = format!("{button_path}/{suffix}");
        let node = doc
            .nodes
            .iter()
            .find(|node| node.path == path)
            .unwrap_or_else(|| panic!("harvest action button child missing: {suffix}"));
        format!("@{}", node.game_object_id)
    };
    let mut owned: Vec<String> = vec![button_path.clone(), doc.nodes[cannot_index].path.clone()];
    for subtree in super::tool_view::SUBTREES {
        let i = doc.find(subtree).unwrap_or_else(|error| panic!("{error}"));
        owned.push(doc.nodes[i].path.clone());
    }
    let mut view = UiPrefabView::new(KEY, BALLOON_LAYER);
    let mut hidden = 0usize;
    for node in &doc.nodes {
        let kept = owned.iter().any(|root| {
            root.starts_with(&format!("{}/", node.path))
                || node.path == *root
                || node.path.starts_with(&format!("{root}/"))
        });
        if !kept {
            view.set_visible(&format!("@{}", node.game_object_id), false);
            hidden += 1;
        }
    }
    let balloon_path = format!("{button_path}/Balloon");
    let balloon_text = doc
        .nodes
        .iter()
        .position(|node| node.path == format!("{balloon_path}/Text"))
        .expect("the balloon's message text");
    let tools = ToolViewMarks::from_document(doc);
    let marks = HarvestButtonView {
        button: format!("@{}", doc.nodes[index].game_object_id),
        active_cover: identity("ActiveCover"),
        cover: identity("Cover"),
        cannot: format!("@{}", doc.nodes[cannot_index].game_object_id),
        icon: identity("icon"),
        balloon: identity("Balloon"),
        balloon_text: format!("@{}", doc.nodes[balloon_text].game_object_id),
        tools,
        icons: Vec::new(),
    };
    // Awake: UIPartsBalloon.enableHideOnAwake (serialized true where the
    // component is decoded) hides the balloon; OnScreenStart: HideHarvestUI.
    view.set_alpha(&marks.balloon, 0.0);
    view.set_alpha(&marks.button, 0.0);
    view.set_visible(&marks.icon, false);
    view.set_visible(&marks.active_cover, false);
    let mut marks = marks;
    for (name, file) in ACTION_ICONS {
        let handle = layouts.register_runtime_texture(
            &icon_alias(name),
            &format!("moly://ui/action-icon/{file}.png"),
            &server,
        );
        marks.icons.push((name, handle));
    }
    info!(
        "[harvest-ui] harvest screen view: {BUTTON}, {CANNOT} and the tool views of {KEY}; {hidden} other nodes hidden; action icons {:?}",
        ACTION_ICONS.map(|(_, file)| file)
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

/// The harvest screen is up (the view is drawn).
fn screen_visible(
    stack: &crate::ui_layers::UiLayerStack,
    entry: Option<&crate::entry::EntrySequence>,
    site: Option<&crate::site::SiteActive>,
) -> bool {
    crate::entry::hud_open(entry)
        && stack.on_field()
        && site.is_some_and(|site| site.category == "harvest")
}

/// Update: pointer-down on the button and the F key raise `OnPressed`;
/// release clears `IsPress`; clicks on the cannot button and the selected
/// tool button.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_input(
    mut gestures: MessageReader<GestureEvent>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    layouts: Res<UiLayouts>,
    views: Query<(&HarvestButtonView, &UiPrefabView)>,
    (stack, entry, site): (
        Res<crate::ui_layers::UiLayerStack>,
        Option<Res<crate::entry::EntrySequence>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    eligibility: crate::interaction::InteractionEligibility,
    mut button: ResMut<HarvestButton>,
    (mut se, mut consumed): (
        ResMut<SeRequests>,
        ResMut<crate::action_button::ActionTapConsumed>,
    ),
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let visible =
        screen_visible(&stack, entry.as_deref(), site.as_deref()) && eligibility.field_input_open();
    if !visible {
        button.pointer = false;
        button.key = false;
        button.is_press = false;
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    let to_canvas = |position: Vec2| -> Vec2 {
        Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale
    };
    let over = |position: Vec2, pick: &dyn Fn(&HarvestButtonView) -> &str| -> bool {
        let canvas = to_canvas(position);
        views.iter().any(|(marks, view)| {
            view.rect(&layouts, pick(marks), size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        })
    };
    for event in &events {
        // ButtonActionCannot lies over the action button while it is active.
        let over_cannot = button.cannot && over(event.position, &|m| m.cannot.as_str());
        match event.state {
            GestureState::Began
                if event.kind == GestureKind::Tap
                    && !over_cannot
                    && over(event.position, &|m| m.button.as_str()) =>
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
            GestureState::End if event.kind.is_tap_family() && over_cannot => {
                consumed.0 = true;
                button.cannot_presses += 1;
                se.source_button(&layouts, KEY, CANNOT);
                info!("[harvest-ui] ButtonActionCannot clicked: OnHarvestActionCannot");
            }
            GestureState::End
                if event.kind.is_tap_family()
                    && button.tools.selected_active
                    && over(event.position, &|m| m.tools.selected.as_str()) =>
            {
                consumed.0 = true;
                button.tools.open_presses += 1;
                if let Some((marks, _)) = views.iter().next() {
                    se.source_button(&layouts, KEY, &marks.tools.selected);
                }
                info!("[harvest-ui] selected-tool button clicked: OnOpenToolSelector");
            }
            _ => {}
        }
    }
    // A pointer that went away without an End (focus loss, a cancelled tap)
    // is a release as well.
    if button.pointer && !mouse.pressed(MouseButton::Left) && touches.iter().next().is_none() {
        button.pointer = false;
    }
    if keys.just_pressed(KeyCode::KeyF) && button.interactable && button.shown {
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

/// Update: the icon loads that `UpdateToolIcon` awaits, then place and dress
/// the view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    time: Res<Time>,
    server: Res<AssetServer>,
    layouts: Res<UiLayouts>,
    mut button: ResMut<HarvestButton>,
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
    // A harvest site's screen is a new instance: its icon cache starts
    // empty and its balloon at rest.
    let site_id = site
        .as_deref()
        .filter(|site| site.category == "harvest")
        .map(|site| site.site_id);
    if site_id != button.screen_site {
        button.screen_site = site_id;
        button.icon_cache.clear();
        button.balloon = BalloonRun::default();
    }
    for _ in 0..std::mem::take(&mut button.cannot_presses) {
        // OnHarvestActionCannot: MessageText, Enable = true, alpha 0 and the
        // first fade (a running run's delay is cancelled).
        button.balloon.start();
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let visible = screen_visible(&stack, entry.as_deref(), site.as_deref());
    let balloon_alpha = button.balloon.advance(time.delta_secs());
    let button = &mut *button;
    for (marks, mut visibility, mut transform, mut view) in &mut views {
        if let Some(name) = button.icon_request {
            let handle = &marks
                .icons
                .iter()
                .find(|(icon, _)| *icon == name)
                .expect("every table name has a registered icon")
                .1;
            match server.load_state(handle) {
                LoadState::Loaded => {
                    button.icon_cache.insert(name);
                    button.icon = Some(name);
                    button.icon_request = None;
                    button.shown = true;
                    button.interactable = true;
                    info!("[harvest-ui] UpdateToolIcon: {name} loaded; SetActive(true)");
                }
                LoadState::Failed(error) => {
                    error!(
                        "[harvest-ui] UpdateToolIcon: {name} failed to load ({error}); the button is shown without its icon"
                    );
                    button.icon_cache.insert(name);
                    button.icon = None;
                    button.icon_request = None;
                    button.shown = true;
                    button.interactable = true;
                }
                _ => {}
            }
        }
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
        view.set_alpha(&marks.button, if button.shown { 1.0 } else { 0.0 });
        view.set_visible(&marks.active_cover, button.is_press);
        view.set_visible(&marks.cover, button.cover);
        view.set_visible(&marks.cannot, button.cannot);
        match button.icon {
            Some(name) => {
                view.set_texture(&marks.icon, &icon_alias(name));
                view.set_visible(&marks.icon, true);
            }
            None => view.set_visible(&marks.icon, false),
        }
        if button.balloon.stage != BalloonStage::Idle {
            view.set_text(&marks.balloon_text, layouts.wording(BALLOON_WORDING));
        }
        view.set_alpha(&marks.balloon, balloon_alpha);
        super::tool_view::apply(&mut button.tools, &marks.tools, &mut view, &layouts);
    }
}
