//! The entry weather banner: `ScreenLayerMysekaiNotice.ShowSiteEnvironmentInfo`
//! and `HideSiteEnvironmentInfo`, drawn from the notice layer's prefab.
//!
//! The source:
//! - `ShowSiteEnvironmentInfo(delay, duration)`: the phenomenon master row of
//!   today's phenomenon id; `_topNoticeBack.SetAlpha(0)` (the back image's
//!   colour alpha); with no master row nothing else happens. Otherwise
//!   `_phenomenaRoot.anchoredPosition = (0, 256)`, both objects active,
//!   `SiteMapPhenomenaView.Setup` (the background image takes the Sprite
//!   `bg_site_info` of the site map texture bundle) and
//!   `UpdatePhenomena(master)` (the icon takes `"icon_" + iconAssetbundleName`,
//!   the two texts the name and the English name, then the layout group is
//!   rebuilt); then `DOFade(back, 0.5, duration).SetDelay(delay)` (the tween
//!   settings' default ease) and `DOAnchorPosY(root, 0, duration)
//!   .SetDelay(delay).SetEase(OutQuart)`, awaited.
//! - `HideSiteEnvironmentInfo(delay, duration)`: `DOFade(back, 0, duration)
//!   .SetDelay(delay)`, `DOAnchorPosY(root, 256, duration).SetDelay(delay)
//!   .SetEase(OutQuart)`, awaited, then both objects inactive.
//!
//! Opener: the notice layer (`MysekaiNotice`, 633) is one of the three
//! layers `SceneMysekai` adds at its setup; the screen manager
//! ([`crate::ui_layers`]) adds it, and this view draws only while that layer
//! is active. The calls reach the layer component (`GetLayerComponent`),
//! which exists once the layer is instantiated; before that a call is
//! reported and not drawn.
//!
//! Neither call kills the other's tweens: each frame the running tweens of a
//! channel step in creation order and the last write stands. A tween created
//! in a frame takes its first step in that frame, with that frame's delta.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiPrefab;
use moly_law::ui::dotween::{Ease, FloatTween};
use serde_json::Value;

use crate::balloon::BALLOON_LAYER;
use crate::ui_layout::{SpriteLayoutMetrics, UiLayouts, UiPrefabView};

pub(crate) const KEY: &str = "Notice";
const PRESENTER: &str = "Sekai.Mysekai.ScreenLayerMysekaiNotice";
const VIEW: &str = "Sekai.Mysekai.SiteMapPhenomenaView";
/// The hidden and shown y of `_phenomenaRoot`, and the back's shown alpha.
const HIDDEN_Y: f32 = 256.0;
const SHOWN_Y: f32 = 0.0;
const BACK_ALPHA: f32 = 0.5;
/// The runtime Sprites `SiteMapPhenomenaView` loads, under the UI root's
/// runtime texture prefix.
const SPRITE_PREFIX: &str = "site-map-phenomena/";
const BACKGROUND_SPRITE: &str = "bg_site_info";

/// `ScreenLayerMysekaiNotice.ShowSiteEnvironmentInfo` /
/// `HideSiteEnvironmentInfo`. `phenomenon` is today's phenomenon id.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) enum SiteEnvironmentInfo {
    Show {
        phenomenon: i32,
        delay: f32,
        duration: f32,
    },
    Hide {
        delay: f32,
        duration: f32,
    },
}

#[derive(Component)]
pub(crate) struct NoticeRoot;

struct Bindings {
    root: String,
    back_graphic: i64,
    back_path: String,
    back_color: [f32; 4],
    icon: String,
    background: String,
    jp_name: String,
    en_name: String,
}

#[derive(Default)]
struct Channel {
    value: f32,
    tweens: Vec<FloatTween>,
}

impl Channel {
    /// One tween-manager step of this channel's tweens, in creation order.
    fn step(&mut self, dt: f32) -> bool {
        let mut wrote = false;
        for tween in &mut self.tweens {
            if let Some(value) = tween.update(self.value, dt) {
                self.value = value;
                wrote = true;
            }
        }
        self.tweens.retain(|tween| !tween.is_complete());
        wrote
    }
}

#[derive(Resource, Default)]
pub(crate) struct NoticeBanner {
    bindings: Option<Bindings>,
    back_alpha: Channel,
    root_y: Channel,
    /// The awaited move tween of a hide is running.
    hiding: bool,
    shown_at: Option<f64>,
    warned_sprites: bool,
}

fn pointer(value: &Value, what: &str) -> i64 {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .unwrap_or_else(|| panic!("{KEY}: {what} is not a serialized reference"));
    assert_eq!(
        pair[0].as_i64(),
        Some(0),
        "{KEY}: {what} points outside the prefab"
    );
    pair[1]
        .as_i64()
        .filter(|id| *id != 0)
        .unwrap_or_else(|| panic!("{KEY}: {what} is null"))
}

fn node_path(doc: &UiPrefab, id: i64) -> String {
    let index = doc
        .find(&format!("@{id}"))
        .unwrap_or_else(|e| panic!("{e}"));
    format!("@{}", doc.nodes[index].game_object_id)
}

fn bind(doc: &UiPrefab) -> Bindings {
    let find = |class: &str| {
        doc.nodes
            .iter()
            .flat_map(|node| &node.components)
            .find(|c| c.class == class)
            .unwrap_or_else(|| panic!("{KEY}: no {class}"))
    };
    let presenter = &find(PRESENTER).fields;
    let view = &find(VIEW).fields;
    let back_graphic = pointer(&presenter["_topNoticeBack"], "_topNoticeBack");
    let back = doc
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == back_graphic)
        .expect("notice back graphic");
    let back_color = back.fields["m_Color"]
        .as_array()
        .filter(|c| c.len() == 4)
        .map(|c| [0, 1, 2, 3].map(|k| c[k].as_f64().expect("back colour") as f32))
        .unwrap_or_else(|| panic!("{KEY}: the back image has no colour"));
    Bindings {
        root: node_path(doc, pointer(&presenter["_phenomenaRoot"], "_phenomenaRoot")),
        back_graphic,
        // The back's colour override is keyed by its graphic; its
        // visibility shares that entry.
        back_path: format!("@{back_graphic}"),
        back_color,
        icon: node_path(
            doc,
            pointer(&view["_siteMapPhenomenaIcon"], "_siteMapPhenomenaIcon"),
        ),
        background: node_path(
            doc,
            pointer(
                &view["_siteMapPhenomenaBackground"],
                "_siteMapPhenomenaBackground",
            ),
        ),
        jp_name: node_path(
            doc,
            pointer(&view["_siteMapPhenomenaJPName"], "_siteMapPhenomenaJPName"),
        ),
        en_name: node_path(
            doc,
            pointer(&view["_siteMapPhenomenaENName"], "_siteMapPhenomenaENName"),
        ),
    }
}

/// The site map Sprites' authored metrics, when the root carries them.
fn register_sprites(layouts: &mut UiLayouts) -> usize {
    let Some(sprites) = layouts.runtime_sprites().cloned() else {
        return 0;
    };
    let mut count = 0;
    for (name, sprite) in sprites.as_object().expect("runtime Sprite map") {
        if !name.starts_with(SPRITE_PREFIX) {
            continue;
        }
        let number = |field: &str, index: usize| {
            sprite[field][index]
                .as_f64()
                .filter(|v| v.is_finite())
                .expect("source Sprite metric") as f32
        };
        layouts.set_runtime_sprite_layout(
            name,
            SpriteLayoutMetrics {
                rect_size: Vec2::new(number("rectSize", 0), number("rectSize", 1)),
                border: [
                    number("border", 0),
                    number("border", 1),
                    number("border", 2),
                    number("border", 3),
                ],
                pixels_per_unit: sprite["pixelsPerUnit"]
                    .as_f64()
                    .expect("Sprite pixelsPerUnit") as f32,
            },
        );
        count += 1;
    }
    count
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut banner: ResMut<NoticeBanner>,
) {
    if banner.bindings.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let bindings = bind(layouts.document(KEY).expect("ready notice prefab"));
    let sprites = register_sprites(&mut layouts);
    info!(
        "[notice] {KEY} view: phenomena root {}, back graphic @{} colour {:?}, view icon {} background {} texts {} / {}; {sprites} site map Sprites on this root",
        bindings.root, bindings.back_graphic, bindings.back_color, bindings.icon, bindings.background,
        bindings.jp_name, bindings.en_name
    );
    banner.back_alpha.value = bindings.back_color[3];
    banner.root_y.value = layouts
        .document(KEY)
        .and_then(|doc| {
            doc.find(&bindings.root)
                .ok()
                .map(|i| doc.nodes[i].rect.anchored_position[1])
        })
        .expect("phenomena root position");
    commands.spawn((
        NoticeRoot,
        Visibility::Inherited,
        Transform::from_xyz(0.0, 0.0, 100.0),
        RenderLayers::layer(BALLOON_LAYER),
        UiPrefabView::new(KEY, BALLOON_LAYER),
    ));
    banner.bindings = Some(bindings);
}

/// The calls, then one tween step of both channels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut calls: MessageReader<SiteEnvironmentInfo>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    layouts: Res<UiLayouts>,
    mut banner: ResMut<NoticeBanner>,
    mut roots: Query<(&mut Transform, &mut UiPrefabView, &mut Visibility), With<NoticeRoot>>,
    screens: Res<crate::ui_layers::ScreenManager>,
) {
    let mut calls: Vec<SiteEnvironmentInfo> = calls.read().copied().collect();
    let layer = crate::ui_layers::MenuScreenType::MysekaiNotice;
    if !screens.is_active(layer) {
        for call in calls.drain(..) {
            error!("[notice] {call:?}: GetLayerComponent<ScreenLayerMysekaiNotice> finds no active {layer:?} layer: not drawn");
        }
    }
    let banner = &mut *banner;
    let Some(bindings) = banner.bindings.as_ref() else {
        for call in calls {
            error!("[notice] {call:?} before the notice prefab is ready: not drawn");
        }
        return;
    };
    let Ok((mut transform, mut view, mut visibility)) = roots.single_mut() else {
        return;
    };
    let shown = if screens.is_active(layer) { Visibility::Inherited } else { Visibility::Hidden };
    if *visibility != shown {
        *visibility = shown;
    }
    if let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) {
        transform.scale = Vec3::splat(root_canvas.scale(window));
    }
    let now = time.elapsed_secs_f64();
    let ease = if layouts.tween_defaults().is_some() {
        Ease::OutQuad
    } else {
        if !calls.is_empty() {
            warn!("[notice] this UI root carries no tween defaults: the back fade takes OutQuad, the default of the JP root's tween settings");
        }
        Ease::OutQuad
    };
    for call in calls {
        match call {
            SiteEnvironmentInfo::Show {
                phenomenon,
                delay,
                duration,
            } => {
                banner.back_alpha.value = 0.0;
                let [r, g, b, _] = bindings.back_color;
                view.set_graphic_color(bindings.back_graphic, [r, g, b, 0.0]);
                let master = catalogue
                    .0
                    .iter()
                    .find_map(|option| option.metadata.as_ref().filter(|m| m.id == phenomenon));
                let Some(master) = master else {
                    error!("[notice] t {now:.4}: ShowSiteEnvironmentInfo: no phenomenon master row {phenomenon}: the back alpha is 0 and nothing is shown");
                    continue;
                };
                banner.root_y.value = HIDDEN_Y;
                view.set_anchored_position(&bindings.root, Vec2::new(0.0, HIDDEN_Y));
                view.set_visible(&bindings.root, true);
                view.set_visible(&bindings.back_path, true);
                let icon = master.icon_assetbundle_name.as_deref().unwrap_or_else(|| {
                    panic!("[notice] phenomenon {phenomenon} has no iconAssetbundleName")
                });
                let aliases = [
                    (
                        &bindings.background,
                        format!("{SPRITE_PREFIX}{BACKGROUND_SPRITE}"),
                    ),
                    (&bindings.icon, format!("{SPRITE_PREFIX}icon_{icon}")),
                ];
                for (path, alias) in aliases {
                    if layouts.has_runtime_texture(&alias) {
                        view.set_texture(path, &alias);
                    } else if !banner.warned_sprites {
                        banner.warned_sprites = true;
                        error!("[notice] this UI root carries no runtime Sprite {alias}: the banner image {path} is not drawn");
                    }
                }
                let english = master.english_name.clone().unwrap_or_default();
                view.set_text(&bindings.jp_name, master.name.clone());
                view.set_text(&bindings.en_name, english.clone());
                banner
                    .back_alpha
                    .tweens
                    .push(FloatTween::new(BACK_ALPHA, duration, ease).with_delay(delay));
                banner
                    .root_y
                    .tweens
                    .push(FloatTween::new(SHOWN_Y, duration, Ease::OutQuart).with_delay(delay));
                banner.shown_at = Some(now);
                info!(
                    "[notice] t {now:.4}: ShowSiteEnvironmentInfo(delay {delay}, duration {duration}) phenomenon {phenomenon} (\"{}\", \"{english}\", icon_{icon}): back alpha 0, root y {HIDDEN_Y}, both active; fade back to {BACK_ALPHA} ({ease:?}), root y to {SHOWN_Y} (OutQuart)",
                    master.name
                );
            }
            SiteEnvironmentInfo::Hide { delay, duration } => {
                banner
                    .back_alpha
                    .tweens
                    .push(FloatTween::new(0.0, duration, ease).with_delay(delay));
                banner
                    .root_y
                    .tweens
                    .push(FloatTween::new(HIDDEN_Y, duration, Ease::OutQuart).with_delay(delay));
                banner.hiding = true;
                info!(
                    "[notice] t {now:.4}: HideSiteEnvironmentInfo(delay {delay}, duration {duration}): fade back to 0 ({ease:?}), root y to {HIDDEN_Y} (OutQuart){}",
                    banner.shown_at.map(|t| format!(", {:.4}s after the show", now - t)).unwrap_or_default()
                );
            }
        }
    }
    let dt = time.delta_secs();
    let had_move = !banner.root_y.tweens.is_empty();
    if banner.back_alpha.step(dt) {
        let [r, g, b, _] = bindings.back_color;
        view.set_graphic_color(bindings.back_graphic, [r, g, b, banner.back_alpha.value]);
    }
    if banner.root_y.step(dt) {
        view.set_anchored_position(&bindings.root, Vec2::new(0.0, banner.root_y.value));
    }
    if had_move && banner.root_y.tweens.is_empty() {
        info!(
            "[notice] t {now:.4}: banner tweens done: back alpha {:.4}, root y {:.4}{}",
            banner.back_alpha.value,
            banner.root_y.value,
            banner
                .shown_at
                .map(|t| format!(", {:.4}s after the show", now - t))
                .unwrap_or_default()
        );
        if banner.hiding {
            // The awaited hide returns: both objects inactive.
            banner.hiding = false;
            view.set_visible(&bindings.root, false);
            view.set_visible(&bindings.back_path, false);
            info!("[notice] HideSiteEnvironmentInfo done: phenomena root and back inactive");
        }
    }
}
