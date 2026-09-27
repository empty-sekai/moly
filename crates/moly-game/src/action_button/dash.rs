//! The dash button (`MysekaiDashActionButton`): the HUD control that turns
//! the player's `IsDashMode` on and off.
//!
//! Where it shows. The home, harvest and delivery field screens carry it
//! (`UIPartsDashButton` in each prefab). Its Setup shows it when
//! `MysekaiUtility.IsAllowedToChangeDashMode` holds: the current site's
//! master row exists and its category is not 1 (the room). The room screen's
//! presenter also turns dash mode off each time it is set up. The home
//! screen's presenter runs Setup and then shows it as an added button; the
//! harvest and delivery screens run Setup and then `ShowWithAnimation`, which
//! fades the button's canvas group from 0 to 1 over 0.15 s (Linear). While a
//! timeline fixture plays, the home screen hides the whole action-buttons
//! view, the dash button with it.
//!
//! A press. The listener flips `IsDashMode`, runs the screen's action and then
//! `UpdateState`. On the home screen the action is `OnDash`: in Move the
//! player changes to Dash, in Dash to Move, in any other state nothing. The
//! harvest and delivery screens pass no action. The click wrapper's
//! `IsResetJoyStick` does not reset the stick for this button: its type bit
//! is 0 and each prefab's own `_isResetJoyStick` is 0 (read from the player
//! data of all four screen prefabs); the layout documents do not carry that
//! flag yet, so it is read when present and otherwise taken as that 0.
//!
//! The look. `SetState` does nothing when the state is unchanged; otherwise
//! `EffectActivate` or `EffectDeactivate`, immediately when asked (Setup asks).
//! * The four icon images (Base, IconLine (2), IconLine (1), Icon) take the
//!   state's colour and keep their own alpha: white when enabled; when active,
//!   `Sekai.ColorUtility`'s colour built from `PaletteUtility.GetColorCode(44)`
//!   (a hex round trip, so each channel is a multiple of 1/255).
//! * Activate: the effect root (RotRoot) goes active at scale 0.7 and scales
//!   to 1 over 0.2 s (OutBack) while the ring fades to alpha 1 over 0.13 s;
//!   the root spins by (0, 0, -360) every 0.8 s (Linear, endless restarts,
//!   started once and never stopped); the icon lines blink on an endless
//!   0.4 s value tween 0 -> 180 on the default ease: IconLine (2)'s alpha is
//!   |sin(v degrees)| and IconLine (1)'s is one minus it.
//! * Deactivate: the root goes back to scale 1 and shrinks to 0.7 over 0.1 s
//!   (InBack) while the ring fades to 1 over 0.1 s, then goes inactive; the
//!   blink pauses and the lines reset to 0 and 1.
//!
//! The four images are drawn by the prefab view; the ring is drawn here as
//! its own sprite, because the view has no rotation or scale for a node.
//! Not ported: the press-down and release colour overrides of the internal
//! button, and the harvest and delivery screens' own hide and show calls
//! (they are delegates whose owners are not read), so there the button is
//! shown whenever the field screen is.

use bevy::asset::AssetPath;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_law::action_button::ButtonType;
use moly_law::ui::dotween::{Ease, FloatTween};

use super::{ActionButtonState, ActionTapConsumed};
use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::player::{DashMode, PlayerControlled};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The dash button's node in each field screen prefab that carries it.
const NODE: &str = "UIPartsDashButton";

/// The field screens whose dash button Setup shows, by site category (the
/// room, category 1, is the one it hides on), with their layout keys.
const HOSTS: [(&str, &str); 3] = [
    ("housing_home", "ShellHome"),
    ("harvest", "ShellHarvest"),
    ("delivery", "ShellDelivery"),
];

/// The room's site category: its screen presenter turns dash mode off.
const ROOM_CATEGORY: &str = "housing_room";

/// `_effectStartScale`: the constructor writes 0.7 to all three axes.
const EFFECT_START_SCALE: f32 = 0.7;
/// EffectActivate's DOScale(one, 0.2) and the ring's DOFade(1, 0.13).
const ACTIVATE_SCALE_SECONDS: f32 = 0.2;
const ACTIVATE_RING_SECONDS: f32 = 0.13;
/// EffectDeactivate's DOScale(_effectStartScale, 0.1) and DOFade(1, 0.1).
const DEACTIVATE_SECONDS: f32 = 0.1;
/// The root's DORotate((0, 0, -360), 0.8, FastBeyond360), Linear.
const ROTATE_SECONDS: f32 = 0.8;
const ROTATE_DEGREES: f32 = -360.0;
/// The icon lines' DOVirtual.Float(0, 180, 0.4) on the default ease.
const BLINK_SECONDS: f32 = 0.4;
const BLINK_TO: f32 = 180.0;
/// The callback's degree-to-radian factor (the float constant it loads).
const DEG_TO_RAD_BITS: u32 = 0x3c8e_fa35;
/// ShowWithAnimation's DOFade(1, 0.15) on the canvas group, Linear.
const SHOW_FADE_SECONDS: f32 = 0.15;
/// The palette entry of the active icon colour.
const ACTIVE_COLOR_ENTRY: usize = 44;
/// DOTween's default overshoot of the Back eases.
const BACK_OVERSHOOT: f32 = 1.701_58;

/// `EaseManager.Evaluate` OutBack: `(t = t/d - 1) * t * ((s + 1) * t + s) + 1`.
fn out_back(time: f32, duration: f32) -> f32 {
    let t = time / duration - 1.0;
    t * t * ((BACK_OVERSHOOT + 1.0) * t + BACK_OVERSHOOT) + 1.0
}

/// `EaseManager.Evaluate` InBack: `(t /= d) * t * ((s + 1) * t - s)`.
fn in_back(time: f32, duration: f32) -> f32 {
    let t = time / duration;
    t * t * ((BACK_OVERSHOOT + 1.0) * t - BACK_OVERSHOOT)
}

/// The icon lines' alphas at one value of the blink tween: IconLine (2) takes
/// `|sinf(v * factor)|`, IconLine (1) one minus what IconLine (2) then holds.
fn blink_alphas(value: f32) -> [f32; 2] {
    let first = (value * f32::from_bits(DEG_TO_RAD_BITS)).sin().abs();
    [first, 1.0 - first]
}

/// A colour through `ColorUtility.ToHtmlString` and `TryParseHtmlString`:
/// each channel rounded to a multiple of 1/255.
fn hex_round_trip(channel: f32) -> f32 {
    (channel.clamp(0.0, 1.0) * 255.0).round_ties_even() / 255.0
}

/// One `_effectSequence`: the root's scale tween with the ring's fade joined.
#[derive(Clone, Copy, Debug)]
struct EffectSequence {
    scale_from: f32,
    scale_to: f32,
    scale_seconds: f32,
    /// OutBack for the activation, InBack for the deactivation.
    grows: bool,
    ring_from: f32,
    ring_seconds: f32,
    position: f32,
    /// EffectDeactivate's OnComplete: the root goes inactive.
    hides_root: bool,
}

impl EffectSequence {
    fn scale_at(&self, position: f32) -> f32 {
        let time = position.min(self.scale_seconds);
        let eased = if self.grows {
            out_back(time, self.scale_seconds)
        } else {
            in_back(time, self.scale_seconds)
        };
        self.scale_from + (self.scale_to - self.scale_from) * eased
    }

    fn ring_at(&self, position: f32) -> f32 {
        let time = position.min(self.ring_seconds);
        self.ring_from + (1.0 - self.ring_from) * Ease::OutQuad.evaluate(time, self.ring_seconds)
    }
}

/// The button's runtime state: the component fields and its tweens.
#[derive(Resource)]
pub(crate) struct DashButton {
    /// The layout of the field screen whose button is mounted.
    host: Option<&'static str>,
    /// `_currentState`: `Some(true)` Active, `Some(false)` Enable.
    current: Option<bool>,
    icons_active: bool,
    root_active: bool,
    root_scale: f32,
    ring_alpha: f32,
    sequence: Option<EffectSequence>,
    /// `_effectLoopTweener1`'s elapsed time, once created.
    rotation: Option<f32>,
    /// `_effectLoopTweener2`: elapsed time and whether it is paused.
    blink: Option<(f32, bool)>,
    /// IconLine (2) and IconLine (1) alphas.
    lines: [f32; 2],
    canvas_alpha: f32,
    fade: Option<FloatTween>,
    /// The field screen was current on the previous frame.
    was_mounted: bool,
    warned_palette: bool,
    warned_flag: bool,
}

impl Default for DashButton {
    fn default() -> Self {
        Self {
            host: None,
            current: None,
            icons_active: false,
            root_active: true,
            root_scale: 1.0,
            ring_alpha: 1.0,
            sequence: None,
            rotation: None,
            blink: None,
            // Awake: ResetLineEffectAlpha.
            lines: [0.0, 1.0],
            canvas_alpha: 1.0,
            fade: None,
            was_mounted: false,
            warned_palette: false,
            warned_flag: false,
        }
    }
}

impl DashButton {
    fn reset_lines(&mut self) {
        self.lines = [0.0, 1.0];
    }

    fn complete_sequence(&mut self) {
        if let Some(sequence) = self.sequence.take() {
            self.root_scale = sequence.scale_to;
            self.ring_alpha = 1.0;
            if sequence.hides_root {
                self.root_active = false;
            }
        }
    }

    fn effect_activate(&mut self, immediately: bool) {
        self.icons_active = true;
        self.root_active = true;
        self.root_scale = EFFECT_START_SCALE;
        self.sequence = Some(EffectSequence {
            scale_from: self.root_scale,
            scale_to: 1.0,
            scale_seconds: ACTIVATE_SCALE_SECONDS,
            grows: true,
            ring_from: self.ring_alpha,
            ring_seconds: ACTIVATE_RING_SECONDS,
            position: 0.0,
            hides_root: false,
        });
        if immediately {
            self.complete_sequence();
        }
        // An endless loop stays active once created, so it is never restarted.
        self.rotation.get_or_insert(0.0);
        self.blink = Some((0.0, false));
    }

    fn effect_deactivate(&mut self, immediately: bool) {
        self.icons_active = false;
        self.root_scale = 1.0;
        self.sequence = Some(EffectSequence {
            scale_from: self.root_scale,
            scale_to: EFFECT_START_SCALE,
            scale_seconds: DEACTIVATE_SECONDS,
            grows: false,
            ring_from: self.ring_alpha,
            ring_seconds: DEACTIVATE_SECONDS,
            position: 0.0,
            hides_root: true,
        });
        if let Some((_, paused)) = self.blink.as_mut() {
            *paused = true;
        }
        self.reset_lines();
        if immediately {
            self.complete_sequence();
        }
    }

    fn set_state(&mut self, active: bool, immediately: bool) {
        if self.current == Some(active) {
            return;
        }
        if active {
            self.effect_activate(immediately);
        } else {
            self.effect_deactivate(immediately);
        }
        self.current = Some(active);
    }

    /// Setup on a newly mounted screen, then that screen's show.
    fn mount(&mut self, host: &'static str, dash: bool) {
        if self.host != Some(host) {
            // Another screen's prefab: its own button instance.
            *self = Self {
                warned_palette: self.warned_palette,
                warned_flag: self.warned_flag,
                ..Self::default()
            };
            self.host = Some(host);
        }
        self.set_state(dash, true);
        self.reset_lines();
        if host != "ShellHome" {
            // ShowWithAnimation: the effect by the mode, not through SetState.
            if dash {
                self.effect_activate(false);
            } else {
                self.effect_deactivate(false);
            }
            self.canvas_alpha = 0.0;
            self.fade = Some(FloatTween::new(1.0, SHOW_FADE_SECONDS, Ease::Linear));
        }
    }

    /// One tween step of `delta` seconds.
    fn advance(&mut self, delta: f32) {
        if let Some(mut sequence) = self.sequence {
            sequence.position += delta;
            self.root_scale = sequence.scale_at(sequence.position);
            self.ring_alpha = sequence.ring_at(sequence.position);
            self.sequence = Some(sequence);
            if sequence.position >= sequence.scale_seconds {
                self.complete_sequence();
            }
        }
        if let Some(elapsed) = self.rotation.as_mut() {
            *elapsed += delta;
        }
        if let Some((elapsed, false)) = self.blink.as_mut() {
            *elapsed += delta;
            let time = elapsed.rem_euclid(BLINK_SECONDS);
            self.lines = blink_alphas(BLINK_TO * Ease::OutQuad.evaluate(time, BLINK_SECONDS));
        }
        if let Some(fade) = self.fade.as_mut() {
            if let Some(value) = fade.update(self.canvas_alpha, delta) {
                self.canvas_alpha = value;
            }
            if fade.is_complete() {
                self.fade = None;
            }
        }
    }

    fn rotation_degrees(&self) -> f32 {
        self.rotation.map_or(0.0, |elapsed| {
            ROTATE_DEGREES * elapsed.rem_euclid(ROTATE_SECONDS) / ROTATE_SECONDS
        })
    }
}

/// One field screen's dash button view: the prefab view of the button's
/// subtree, with the identities this module writes.
#[derive(Component)]
pub(crate) struct DashButtonView {
    key: &'static str,
    /// `@` identity of UIPartsDashButton (its canvas group and hit rect).
    button: String,
    /// Base, IconLine (2), IconLine (1), Icon: image component ids and their
    /// serialized colours.
    icons: [(i64, [f32; 4]); 4],
    /// The `@` identities of RotRoot and Circle.
    rot_root: String,
    circle: String,
    /// The serialized alpha of the canvas groups above the button.
    parent_alpha: f32,
}

/// The ring sprite, a child of its view's root.
#[derive(Component)]
pub(crate) struct DashRing {
    /// The Circle's quad inside its rect (Image Simple on its sprite's
    /// padding), in the rect's local units: centre offset and size.
    quad_center: Vec2,
    quad_size: Vec2,
    color: [f32; 4],
}

/// Serialized `m_Color` of an image component.
fn image_color(component: &moly_assets::ui_layout::UiComponent) -> [f32; 4] {
    let values = component.fields["m_Color"]
        .as_array()
        .expect("source UI colour");
    std::array::from_fn(|i| values[i].as_f64().expect("source UI colour channel") as f32)
}

fn array(value: &serde_json::Value, len: usize) -> Option<Vec<f32>> {
    let values = value.as_array()?;
    (values.len() == len)
        .then(|| {
            values
                .iter()
                .map(|v| v.as_f64().map(|v| v as f32))
                .collect()
        })
        .flatten()
}

/// Update: once the three field-screen layouts are loaded, one view per
/// screen showing only its dash button's subtree, with the ring hidden in the
/// view and drawn by its own sprite.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Option<Res<UiLayouts>>,
    server: Res<AssetServer>,
    views: Query<(), With<DashButtonView>>,
) {
    if !views.is_empty() {
        return;
    }
    let Some(layouts) = layouts else { return };
    if HOSTS.iter().any(|(_, key)| layouts.document(key).is_none()) {
        return;
    }
    for (_, key) in HOSTS {
        let doc = layouts.document(key).expect("checked above");
        let index = doc.find(NODE).unwrap_or_else(|error| panic!("{error}"));
        let button_path = doc.nodes[index].path.clone();
        let child = |name: &str| -> usize {
            let path = format!("{button_path}/{name}");
            doc.nodes
                .iter()
                .position(|node| node.path == path)
                .unwrap_or_else(|| panic!("dash button child missing in {key}: {name}"))
        };
        let image = |index: usize| {
            doc.nodes[index]
                .components
                .iter()
                .find(|component| component.enabled && component.class.ends_with("Image"))
                .unwrap_or_else(|| {
                    panic!(
                        "dash button image missing in {key}: {}",
                        doc.nodes[index].path
                    )
                })
        };
        let icons = ["Base", "IconLine (2)", "IconLine (1)", "Icon"].map(|name| {
            let component = image(child(name));
            (component.path_id, image_color(component))
        });
        let rot_root = child("RotRoot");
        let circle = child("RotRoot/Circle");
        let identity = |index: usize| format!("@{}", doc.nodes[index].game_object_id);
        let mut view = UiPrefabView::new(key, BALLOON_LAYER);
        let mut parent_alpha = 1.0;
        for node in &doc.nodes {
            let ancestor = button_path.starts_with(&format!("{}/", node.path));
            let inside =
                node.path == button_path || node.path.starts_with(&format!("{button_path}/"));
            if ancestor {
                for group in node
                    .components
                    .iter()
                    .filter(|c| c.enabled && c.class == "UnityEngine.CanvasGroup")
                {
                    parent_alpha *=
                        group.fields["m_Alpha"].as_f64().expect("CanvasGroup alpha") as f32;
                }
            }
            if !ancestor && !inside {
                view.set_visible(&format!("@{}", node.game_object_id), false);
            }
        }
        // The prefab serializes RotRoot active; its visibility is the
        // effect root's, drawn by the ring sprite.
        view.set_visible(&identity(rot_root), false);
        // The ring: an Image Simple quad on its sprite's padding.
        let ring = image(circle);
        let sprite = ring
            .sprite
            .as_ref()
            .unwrap_or_else(|| panic!("dash ring sprite missing in {key}"));
        let rect = array(&sprite["rect"], 4).expect("dash ring sprite rect");
        let exported = array(&sprite["size"], 2).expect("dash ring exported size");
        let (texture_rect, offset) = match (
            array(&sprite["textureRect"], 4),
            array(&sprite["textureRectOffset"], 2),
        ) {
            (Some(texture_rect), Some(offset)) => (texture_rect, offset),
            _ => (vec![0.0, 0.0, rect[2], rect[3]], vec![0.0, 0.0]),
        };
        let node_size = Vec2::from_array(doc.nodes[circle].rect.size_delta);
        let pivot = Vec2::from_array(doc.nodes[circle].rect.pivot);
        let (sprite_w, sprite_h) = (rect[2].round(), rect[3].round());
        let padding = [
            offset[0],
            offset[1],
            rect[2] - texture_rect[2] - offset[0],
            rect[3] - texture_rect[3] - offset[1],
        ];
        let lower = Vec2::new(padding[0] / sprite_w, padding[1] / sprite_h);
        let upper = Vec2::new(
            (sprite_w - padding[2]) / sprite_w,
            (sprite_h - padding[3]) / sprite_h,
        );
        let origin = -pivot * node_size;
        let quad_min = origin + node_size * lower;
        let quad_max = origin + node_size * upper;
        // The exported image is the texture rect cropped at its rounded origin.
        let crop = Vec2::new(texture_rect[0].round(), texture_rect[1].round());
        let u0 = texture_rect[0] - crop.x;
        let v_top = exported[1] - (texture_rect[1] + texture_rect[3] - crop.y);
        let uv_rect = Rect::new(u0, v_top, u0 + texture_rect[2], v_top + texture_rect[3]);
        let image_path = sprite["image"].as_str().expect("dash ring sprite image");
        let handle = moly_assets::residency::load_image(
            &server,
            AssetPath::from(crate::ui_layout::image_asset_path(image_path)),
        );
        let marks = DashButtonView {
            key,
            button: identity(index),
            icons,
            rot_root: identity(rot_root),
            circle: identity(circle),
            parent_alpha,
        };
        let ring_entity = commands
            .spawn((
                DashRing {
                    quad_center: (quad_min + quad_max) * 0.5,
                    quad_size: quad_max - quad_min,
                    color: image_color(ring),
                },
                Sprite {
                    image: handle,
                    custom_size: Some(quad_max - quad_min),
                    rect: Some(uv_rect),
                    ..default()
                },
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(BALLOON_LAYER),
            ))
            .id();
        commands
            .spawn((
                marks,
                Visibility::Hidden,
                Transform::default(),
                RenderLayers::layer(BALLOON_LAYER),
                view,
            ))
            .add_child(ring_entity);
        info!(
            "[dash-button] view of {key}: {button_path}; ring quad {:?}..{:?}",
            quad_min, quad_max
        );
    }
}

/// The layout of the field screen that is current now, if it shows the
/// dash button; and whether the current site is the room.
fn current_host(
    state: &ActionButtonState,
    site: Option<&crate::site::SiteActive>,
) -> (Option<&'static str>, bool) {
    let Some(site) = site else {
        return (None, false);
    };
    if !state.home_mounted {
        return (None, false);
    }
    let host = HOSTS
        .iter()
        .find(|(category, _)| *category == site.category)
        .map(|(_, key)| *key);
    (host, site.category == ROOM_CATEGORY)
}

/// Update, after the action-button click: advance the button's tweens, run
/// Setup when its screen mounts, and dress the view.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn place(
    time: Res<Time>,
    state: Res<ActionButtonState>,
    site: Option<Res<crate::site::SiteActive>>,
    layouts: Option<Res<UiLayouts>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut dash: ResMut<DashButton>,
    mut players: Query<&mut DashMode, With<PlayerControlled>>,
    mut views: Query<(
        &DashButtonView,
        &mut Visibility,
        &mut Transform,
        &mut UiPrefabView,
        &Children,
    )>,
    mut rings: Query<
        (&DashRing, &mut Transform, &mut Sprite, &mut Visibility),
        Without<DashButtonView>,
    >,
) {
    dash.advance(time.delta_secs());
    let (host, in_room) = current_host(&state, site.as_deref());
    let mounted = host.is_some() || in_room;
    if mounted && !dash.was_mounted {
        if in_room {
            // ScreenLayerMysekaiMyRoomPresenter.SetupPresenter.
            for mut mode in &mut players {
                if mode.0 {
                    info!("[dash-button] room screen set up: dash mode off");
                }
                mode.0 = false;
            }
        }
        if let Some(host) = host {
            let on = players.iter().any(|mode| mode.0);
            dash.mount(host, on);
            info!(
                "[dash-button] {host} set up: state {}",
                if on { "Active" } else { "Enable" }
            );
        }
    }
    dash.was_mounted = mounted;
    let (Some(layouts), Ok(window), Some(root_canvas)) =
        (layouts, windows.single(), root_canvas.as_deref())
    else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let active_rgb = match layouts.palette_color(ACTIVE_COLOR_ENTRY) {
        Some(color) => [color[0], color[1], color[2]].map(hex_round_trip),
        None => {
            if !dash.warned_palette {
                dash.warned_palette = true;
                error!("[dash-button] the UI root carries no palette: the active icon colour (entry {ACTIVE_COLOR_ENTRY}) is not drawn");
            }
            [1.0; 3]
        }
    };
    let shown_host = host.filter(|key| *key != "ShellHome" || state.timeline.is_none());
    for (marks, mut visibility, mut transform, mut view, children) in &mut views {
        let shown = shown_host == Some(marks.key) && dash.host == Some(marks.key);
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
        if !shown {
            continue;
        }
        view.set_alpha(&marks.button, dash.canvas_alpha);
        for (slot, (graphic, serialized)) in marks.icons.iter().enumerate() {
            let rgb = if dash.icons_active {
                active_rgb
            } else {
                [1.0; 3]
            };
            let alpha = match slot {
                1 => dash.lines[0],
                2 => dash.lines[1],
                _ => serialized[3],
            };
            view.set_graphic_color(*graphic, [rgb[0], rgb[1], rgb[2], alpha]);
        }
        let (Some(root_rect), Some(circle_rect)) = (
            view.rect(&layouts, &marks.rot_root, canvas),
            view.rect(&layouts, &marks.circle, canvas),
        ) else {
            continue;
        };
        for child in children.iter() {
            let Ok((ring, mut ring_transform, mut sprite, mut ring_visibility)) =
                rings.get_mut(child)
            else {
                continue;
            };
            *ring_visibility = if dash.root_active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            // The root's pivot, and its runtime rotation and scale about it.
            let pivot = root_rect.world.transform_point3(Vec3::ZERO);
            let spin = Quat::from_rotation_z(dash.rotation_degrees().to_radians());
            let (circle_scale, circle_rotation, _) =
                circle_rect.world.to_scale_rotation_translation();
            let quad = circle_rect
                .world
                .transform_point3(ring.quad_center.extend(0.0));
            let placed = pivot + spin * ((quad - pivot) * dash.root_scale);
            let depth = 20.0
                + layouts
                    .document(marks.key)
                    .map_or(0, |doc| doc.find(&marks.circle).unwrap_or(0)) as f32
                    * 0.02;
            *ring_transform = Transform {
                translation: placed.truncate().extend(depth),
                rotation: spin * circle_rotation,
                scale: circle_scale * dash.root_scale,
            };
            sprite.custom_size = Some(ring.quad_size);
            let alpha = ring.color[3] * dash.ring_alpha * dash.canvas_alpha * marks.parent_alpha;
            sprite.color = Color::srgba(ring.color[0], ring.color[1], ring.color[2], alpha);
        }
    }
}

/// Update, after the action-button click and before the world pick: a tap
/// on the shown dash button is its click.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut dash: ResMut<DashButton>,
    layouts: Option<Res<UiLayouts>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    views: Query<(&DashButtonView, &Visibility, &UiPrefabView)>,
    mut players: Query<&mut DashMode, With<PlayerControlled>>,
    mut states: ResMut<PlayerAvatarStates>,
    mut resets: MessageWriter<crate::joystick::ForceResetJoystick>,
) {
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| {
            matches!(event.kind, GestureKind::Tap | GestureKind::DoubleTap)
                && event.state == GestureState::End
        })
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Some(layouts), Ok(window), Some(root_canvas)) =
        (layouts, windows.single(), root_canvas.as_deref())
    else {
        return;
    };
    let Some((marks, view)) = views
        .iter()
        .find(|(marks, visibility, _)| {
            **visibility != Visibility::Hidden && dash.host == Some(marks.key)
        })
        .map(|(marks, _, view)| (marks, view))
    else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let Some(rect) = view.rect(&layouts, &marks.button, canvas) else {
        return;
    };
    for tap in taps {
        let point = Vec2::new(tap.x - window.width() * 0.5, window.height() * 0.5 - tap.y) / scale;
        if !(rect.active && rect.contains(point)) {
            continue;
        }
        consumed.0 = true;
        // The click wrapper: IsResetJoyStick with the prefab's own flag.
        let flag = reset_flag(&layouts, marks.key);
        if flag.is_none() && !dash.warned_flag {
            dash.warned_flag = true;
            warn!("[dash-button] the layout document does not carry the dash button's _isResetJoyStick; taken as the 0 all four prefabs serialize");
        }
        match ButtonType::Dash.resets_joystick(flag.unwrap_or(false)) {
            Some(true) => {
                resets.write(crate::joystick::ForceResetJoystick {
                    reason: "dash button press",
                });
            }
            Some(false) => {}
            None => {
                error!("[dash-button] IsResetJoyStick: the dash type is not in the check's table")
            }
        }
        let mut on = false;
        for mut mode in &mut players {
            mode.0 = !mode.0;
            on = mode.0;
        }
        if marks.key == "ShellHome" {
            // OnDash.
            match states.current {
                PlayerActionState::Move => states.change_status(PlayerActionState::Dash),
                PlayerActionState::Dash => states.change_status(PlayerActionState::Move),
                _ => {}
            }
        }
        dash.set_state(on, false);
        info!(
            "[dash-button] pressed on {}: dash mode {}, player state {:?}",
            marks.key,
            if on { "on" } else { "off" },
            states.current
        );
    }
}

/// The prefab's `_isResetJoyStick`, when the layout document decodes the
/// dash button component.
fn reset_flag(layouts: &UiLayouts, key: &str) -> Option<bool> {
    let doc = layouts.document(key)?;
    let index = doc.find(NODE).ok()?;
    doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class.ends_with("MysekaiDashActionButton"))?
        .fields
        .get("_isResetJoyStick")?
        .as_bool()
}
