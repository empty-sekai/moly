//! The harvest stamina gauge: `ScreenLayerMysekaiHUD`'s
//! `HarvestPlayerHeadUpDisplay`, whose `_staminaView` is the
//! `MysekaiStaminaView` on its StaminaGage child, drawn from the HUD
//! document (layer 601).
//!
//! The presenter's calls, read in the game binary:
//! - `HarvestPresenter.Initialize` (each harvest site): the HUD's `Init`,
//!   `UpdateStaminaDecreaseGage(rate)` with rate = (colorful pass ? (boost >
//!   0 ? boost rate : enhance rate) : (boost > 0 ? boost rate : normal
//!   rate)) of `GetStaminaGageRate(UserMysekaiStamina)`, then
//!   `SetupStaminaGage(UserMysekaiStamina)` = `UpdateStaminaView(stamina,
//!   stock, false)` + `UpdateStaminaGageRate(stamina, rates, false)` (the
//!   menu dialog's stamina cell writes, reused).
//! - `UpdateHarvestUI`: `Show()` when the stamina is empty.
//! - `PlayHarvestAction`: `LackOfStamina()` on a failed stamina gate; before
//!   each swing's `OnPlayerActionStart`, the gauge animation's cancellation
//!   and `Show()`.
//! - `OnAnimationEvent` (each hit): `UpdateStaminaViewAnimation` (the view
//!   from the stamina before the hit to the one after) and
//!   `DecreaseStaminaAnimation(StaminaUIAnimationAwaitTime)`: a delay of that
//!   many seconds (FloatConfigs key 92), the decrease bar's animation to the
//!   stamina after, `ResetActiveStamina`, `Hide()`. A swing's cancellation
//!   drops a delay still running.
//! - The last contact leaving, and an action ending without a target:
//!   `Hide()`.
//!
//! `HarvestPlayerHeadUpDisplay.Show()`: `HeadUpDisplayBase.Show` (the HUD's
//! GameObject on), then `MysekaiStaminaView.Show`: `alphaTweener.Kill(true)`,
//! the `_canvasGroup`'s GameObject on, `DOFade(1, 0.1)`. `Hide()`:
//! `MysekaiStaminaView.Hide(onComplete)`: `alphaTweener.Kill(true)`,
//! `DOFade(0, 0.1)`, whose completion turns the `_canvasGroup`'s GameObject
//! off and runs `onComplete` = `HeadUpDisplayBase.Hide` (the HUD's GameObject
//! off). A `Show` within 0.1 s of a `Hide` therefore completes the hide
//! first, turning the HUD's GameObject off after `Show` turned it on, and the
//! gauge stays hidden until the next `Show`, as in the source. Fades take the
//! tween settings' default ease (OutQuad). `LackOfStamina()`: `Show()`, the
//! running shake killed where it stands (`Kill(false)`: its offset stays and
//! the next shake starts from it), then `_staminaView.transform
//! .DOShakePosition(0.3, (20, 0, 0), 115, 90, false, true)` (the vector shake
//! of [`law::vector_shake_points`], linear segments on the local position;
//! the draws are this product's generator, not Unity's).
//!
//! The gauge stays at its document place (the screen centre plus (140, 50)),
//! because nothing calls `UpdatePosition` on it. `UpdatePosition` is virtual
//! (vtable slot 7: method pointer at klass + 0x1a8, MethodInfo at + 0x1b0);
//! `HeadUpDisplayBase` and `HarvestPlayerHeadUpDisplay` have no `Update`,
//! `LateUpdate` or `OnEnable`. The paths checked in the game binary: every
//! method of the classes that hold a reference to this HUD (by field or local
//! type: `HarvestPlayerHeadUpDisplay` and its state machines,
//! `HeadUpDisplayBase`, `MysekaiStaminaView`, `HarvestPresenter`,
//! `JoinMysekaiActionState`, `ScreenLayerMysekaiHUD`, 240 methods) has no
//! direct or tail call to `HeadUpDisplayBase.UpdatePosition` and no load at
//! klass + 0x1a8 or + 0x1b0 (a slot-7 dispatch, or the `ldvirtftn` a delegate
//! over it would need). The scan finds the known slot-7 calls in
//! `TweetHeadUpDisplay.Update`, `MysekaiPhotoShotView.Update` and
//! `SelectFixtureHeadUpDisplayView.UpdateFollowTargetPos`, and the direct tail
//! call in `SelectFixtureHeadUpDisplayView.UpdatePosition`. No field or
//! collection is typed `HeadUpDisplayBase`, so no manager reaches this HUD
//! through the base type. No coroutine or callback registers it: there is no
//! method-group reference to it; the metadata has no MethodInfo usage
//! for it (control: `<Hide>b__15_0`, the Hide callback, is present) and no
//! string literal `UpdatePosition` (control: `MSG_NOT_HAVE_TOOL` is
//! present); and a persistent UnityEvent call cannot carry its `Vector3`
//! argument.
//!
//! Named gaps: the fill tweens are not ported (`StaminaViewAnimation`'s
//! 0.5 s gauge change with its boost-stock crossing stages, one of which
//! writes the decrease bar, and `DecreaseStaminaView`'s 0.5 s): at a hit the
//! gauge takes the state after the hit at once, and when the delay ends the
//! decrease bar takes the fill after it at once; the sprites `SpriteName`
//! switches to (`icon_stamina_boost_h40` while boost is held, `txt_stamina_2`
//! to `_9` for a stock of 2 or more) are not drawn: no UI document references
//! them and the atlas crops do not include them, so they exist on the roots
//! only as rects in the MysekaiAtlas page, which this view cannot draw (the
//! serialized sprite stays, named once); the gradient's soft mask is not
//! ported; a new harvest site starts the view at its serialized state
//! (whether a site move keeps the HUD screen alive is not read); the recover
//! dialog's and the join refresh's calls are not built here.
//!
//! `SpriteName = name` resolves the sprite through the UI root's runtime
//! texture inventory, else through the sprite a UI document references by that
//! name (its exported image; the HUD and menu documents reference
//! `icon_stamina_normal_h40` and `txt_stamina_1`, the gauge's own serialized
//! sprites).

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::action::HarvestPlayerModel;
use super::law::{self, PointTween, SegmentEase, Stamina};
use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::menu_dialog::{
    boost_stamina_stock_count, reference, stamina_gage_rate, stamina_view_state, StaminaViewState,
    STAMINA_EMPTY_ICON_COLOR_ENTRY, STAMINA_GAUGE_COLOR_ENTRY,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use moly_assets::ui_layout::UiPrefab;

const KEY: &str = "HUD";
/// ⚠ This view claims this subtree of the HUD document (layer 601): a later
/// view of the whole 601 layer must leave it to this view. The draw order is
/// the 601 layer's: the HUD screen is on display layer 0 (BG), under the
/// harvest screen's display layer 1 (UI), and inside the document the Player
/// root follows the tweet roots. [`view_z`] puts this view under every view
/// at z 0 on the same camera layer and above the tweet balloons (z < 0).
const ROOT: &str = "ComponentRoot/Player/HarvestPlayerHeadUpDisplay";
const FADE: f32 = 0.1;
const SHAKE_DURATION: f32 = 0.3;
const SHAKE_STRENGTH: Vec3 = Vec3::new(20.0, 0.0, 0.0);
const SHAKE_VIBRATO: i32 = 115;
const SHAKE_RANDOMNESS: f32 = 90.0;

/// A presenter call, applied in order by [`advance`].
#[derive(Debug, Clone, Copy)]
enum Call {
    Show,
    Hide,
    LackOfStamina,
    /// The gauge animation's cancellation, then `Show()`.
    ActionStart,
    /// A hit: the stamina before and after it.
    Hit(Stamina, Stamina),
}

#[derive(Debug, Clone, Copy)]
struct Fade {
    /// Captured at the tween's first update.
    from: Option<f32>,
    to: f32,
    elapsed: f32,
    /// The hide's completion: the view's and the HUD's GameObjects off.
    hides: bool,
}

/// The gauge's state as the harvest presenter writes it.
#[derive(Resource, Default)]
pub(crate) struct HarvestStaminaHud {
    calls: Vec<Call>,
    /// The harvest site of this screen instance.
    screen_site: Option<u32>,
    /// The view state is seated from the document.
    seated: bool,
    /// `Initialize` ran for this site.
    initialized: bool,
    /// The HUD's GameObject active.
    active: bool,
    /// The `_canvasGroup`'s GameObject active and its alpha.
    view_active: bool,
    alpha: f32,
    fade: Option<Fade>,
    gauge: Option<StaminaViewState>,
    decrease_fill: Option<f32>,
    /// The view's local position offset from its serialized place, and the
    /// running shake.
    offset: Vec3,
    shake: Option<PointTween>,
    shakes: u64,
    /// `DecreaseStaminaAnimation`'s delay end and the stamina after the hit.
    pending: Option<(f64, Stamina)>,
}

impl HarvestStaminaHud {
    pub(crate) fn show(&mut self) {
        self.calls.push(Call::Show);
    }
    pub(crate) fn hide(&mut self) {
        self.calls.push(Call::Hide);
    }
    pub(crate) fn lack_of_stamina(&mut self) {
        self.calls.push(Call::LackOfStamina);
    }
    pub(crate) fn action_start(&mut self) {
        self.calls.push(Call::ActionStart);
    }
    pub(crate) fn hit(&mut self, before: Stamina, after: Stamina) {
        self.calls.push(Call::Hit(before, after));
    }

    fn do_show(&mut self) {
        self.active = true;
        self.complete_fade();
        self.view_active = true;
        self.fade = Some(Fade {
            from: None,
            to: 1.0,
            elapsed: 0.0,
            hides: false,
        });
    }

    fn do_hide(&mut self) {
        self.complete_fade();
        self.fade = Some(Fade {
            from: None,
            to: 0.0,
            elapsed: 0.0,
            hides: true,
        });
    }

    /// `alphaTweener.Kill(complete: true)`.
    fn complete_fade(&mut self) {
        if let Some(fade) = self.fade.take() {
            self.alpha = fade.to;
            if fade.hides {
                self.view_active = false;
                self.active = false;
            }
        }
    }

    /// Advance the running tweens by `dt` (a tween made by a call takes its
    /// first update the frame after, so the tweens step before the calls).
    fn step(&mut self, dt: f32) {
        if let Some(fade) = self.fade.as_mut() {
            let from = *fade.from.get_or_insert(self.alpha);
            fade.elapsed += dt;
            let u = (fade.elapsed / FADE).clamp(0.0, 1.0);
            self.alpha = from + (fade.to - from) * (-u * (u - 2.0));
            if fade.elapsed >= FADE {
                self.complete_fade();
            }
        }
        if let Some(shake) = self.shake.as_mut() {
            self.offset = shake.advance(self.offset, dt);
            if shake.done() {
                self.shake = None;
            }
        }
    }
}

fn server_stamina(stamina: Stamina) -> crate::server::Stamina {
    crate::server::Stamina {
        normal: stamina.normal,
        enhance: stamina.enhance,
        boost: stamina.boost,
    }
}

/// The view's depth: the UI painter gives the node at `index` the depth
/// `20 + 0.02 * index` inside its view, scaled with the view; this view's
/// whole band sits under `20 * scale` (the lowest part of any view at z 0)
/// and far above the tweet balloons below z 0.
fn view_z(nodes: usize, scale: f32) -> f32 {
    -(nodes as f32 * 0.02 + 0.02) * scale
}

/// The documents that reference the stamina sprites (the two with a
/// `MysekaiStaminaView`).
const SPRITE_DOCUMENTS: [&str; 2] = [KEY, "Menu"];

/// The exported image of the sprite `doc` references by `name`.
fn document_sprite_image(doc: &UiPrefab, name: &str) -> Option<String> {
    doc.nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .filter_map(|component| component.sprite.as_ref())
        .find(|sprite| sprite["name"].as_str() == Some(name))
        .and_then(|sprite| sprite["image"].as_str())
        .map(str::to_owned)
}

/// `CustomImage.SpriteName = name` on the image `image`: the resolved sprite,
/// or nothing when the serialized sprite already is that sprite; any other
/// name is a sprite this view cannot draw (named at spawn).
fn set_sprite(
    view: &mut UiPrefabView,
    doc: &UiPrefab,
    marks: &StaminaHudView,
    image: i64,
    name: &str,
) {
    if let Some(alias) = marks.sprites.get(name) {
        view.set_texture(&format!("@{image}"), alias);
        return;
    }
    let serialized = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|component| component.path_id == image)
        .and_then(|component| component.sprite.as_ref())
        .and_then(|sprite| sprite["name"].as_str());
    if serialized != Some(name) {
        warn_once!(
            "[harvest-hud] SpriteName {name} on image @{image} is not drawable on this root; the serialized {serialized:?} stays"
        );
    }
}

/// The view's targets, as selectors.
#[derive(Component)]
pub(crate) struct StaminaHudView {
    root: String,
    /// `_canvasGroup`'s node.
    group: String,
    /// The `MysekaiStaminaView`'s node, and its serialized anchored position.
    view: String,
    anchored: Vec2,
    serialized_active: bool,
    serialized_alpha: f32,
    gauge: i64,
    decrease: i64,
    gradient: String,
    icon: i64,
    on_stock: String,
    on_no_stock: String,
    boost_count: i64,
    boost_count_node: String,
    /// The sprites `SpriteName` can name, by name: the runtime texture alias
    /// that draws each one found on the root.
    sprites: HashMap<String, String>,
}

impl StaminaHudView {
    fn from_document(doc: &UiPrefab) -> Self {
        let root = doc.find(ROOT).unwrap_or_else(|error| panic!("{error}"));
        let root_path = doc.nodes[root].path.clone();
        let (view_index, view) = doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.path.starts_with(&format!("{root_path}/")))
            .find_map(|(i, node)| {
                node.components
                    .iter()
                    .find(|c| c.class == "Sekai.Mysekai.MysekaiStaminaView")
                    .map(|c| (i, c))
            })
            .unwrap_or_else(|| panic!("{}: no MysekaiStaminaView under {ROOT}", doc.prefab));
        // A Graphic's `enabled` as its GameObject's active flag: each of these
        // nodes carries only that Graphic and has no children (the menu
        // dialog's rule for the same component).
        let graphic_node = |field: &str| -> String {
            let id = reference(doc, &view.fields, field);
            let index = doc
                .find(&format!("@{id}"))
                .unwrap_or_else(|error| panic!("{error}"));
            let node = &doc.nodes[index];
            assert!(
                doc.nodes
                    .iter()
                    .all(|n| n.parent_transform_id != node.transform_id),
                "{}: MysekaiStaminaView {field} is not a leaf node",
                doc.prefab
            );
            format!("@{}", node.game_object_id)
        };
        let group_id = reference(doc, &view.fields, "_canvasGroup");
        let group_index = doc
            .find(&format!("@{group_id}"))
            .unwrap_or_else(|error| panic!("{error}"));
        let group_node = &doc.nodes[group_index];
        let group = group_node
            .components
            .iter()
            .find(|c| c.path_id == group_id && c.class == "UnityEngine.CanvasGroup")
            .unwrap_or_else(|| {
                panic!(
                    "{}: MysekaiStaminaView _canvasGroup {group_id} is not a CanvasGroup",
                    doc.prefab
                )
            });
        let serialized_alpha = group.fields["m_Alpha"]
            .as_f64()
            .unwrap_or_else(|| panic!("{}: CanvasGroup {group_id} has no m_Alpha", doc.prefab))
            as f32;
        let anchored = doc.nodes[view_index].rect.anchored_position;
        StaminaHudView {
            root: format!("@{}", doc.nodes[root].game_object_id),
            group: format!("@{}", group_node.game_object_id),
            view: format!("@{}", doc.nodes[view_index].game_object_id),
            anchored: Vec2::new(anchored[0], anchored[1]),
            serialized_active: group_node.active,
            serialized_alpha,
            gauge: reference(doc, &view.fields, "_staminaGageImage"),
            decrease: reference(doc, &view.fields, "_staminaDecreaseGageImage"),
            gradient: graphic_node("_staminaGradiantImage"),
            icon: reference(doc, &view.fields, "_staminaIcon"),
            on_stock: graphic_node("_staminaGageImageOnStock"),
            on_no_stock: graphic_node("_staminaGageImageOnNoStock"),
            boost_count: reference(doc, &view.fields, "_boostStaminaCount"),
            boost_count_node: graphic_node("_boostStaminaCount"),
            sprites: HashMap::new(),
        }
    }
}

#[derive(Resource)]
pub(crate) struct StaminaHudSpawned;

/// Update: build the view of the claimed subtree once the HUD document is
/// loaded.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<StaminaHudSpawned>>,
) {
    if spawned.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    // The sprites the gauge's writes can name, resolved before the view
    // borrows the document: a runtime texture of that name, else the image of
    // the sprite a UI document references by that name.
    let names: Vec<String> = ["icon_stamina_normal_h40", "icon_stamina_boost_h40"]
        .into_iter()
        .map(str::to_owned)
        .chain((1..=9).map(|n| format!("txt_stamina_{n}")))
        .collect();
    let mut sprites = HashMap::new();
    let mut referenced = Vec::new();
    let mut unresolved = Vec::new();
    for name in names {
        if layouts.has_runtime_texture(&name) {
            sprites.insert(name.clone(), name);
            continue;
        }
        let image = SPRITE_DOCUMENTS
            .iter()
            .filter_map(|key| layouts.document(key))
            .find_map(|doc| document_sprite_image(doc, &name));
        match image {
            Some(image) => referenced.push((name, image)),
            None => unresolved.push(name),
        }
    }
    for (name, image) in referenced {
        let alias = format!("harvest-hud-sprite/{name}");
        layouts.register_runtime_texture(&alias, &image, &server);
        sprites.insert(name, alias);
    }
    let doc = layouts.document(KEY).expect("ready HUD document");
    let mut marks = StaminaHudView::from_document(doc);
    marks.sprites = sprites;
    let root = doc.find(ROOT).unwrap_or_else(|error| panic!("{error}"));
    let root_path = doc.nodes[root].path.clone();
    // The claim (see ROOT): the subtree and its ancestors, nothing else.
    let mut view = UiPrefabView::new(KEY, BALLOON_LAYER);
    let mut hidden = 0usize;
    for node in &doc.nodes {
        let kept = root_path.starts_with(&format!("{}/", node.path))
            || node.path == root_path
            || node.path.starts_with(&format!("{root_path}/"));
        if !kept {
            view.set_visible(&format!("@{}", node.game_object_id), false);
            hidden += 1;
        }
    }
    view.set_visible(&marks.root, false);
    info!(
        "[harvest-hud] stamina gauge view: {ROOT} of the HUD document ({hidden} other nodes hidden); gauge node {} serialized active {} alpha {}",
        marks.view, marks.serialized_active, marks.serialized_alpha
    );
    let mut found: Vec<&String> = marks.sprites.keys().collect();
    found.sort();
    info!("[harvest-hud] stamina sprites resolved on this UI root: {found:?}");
    if !unresolved.is_empty() {
        warn!(
            "[harvest-hud] stamina sprites {unresolved:?}: no runtime texture and no UI document sprite of that name on this root (the atlas page rects are not drawable by this view); the serialized sprite stays when SpriteName names them"
        );
    }
    commands.spawn((
        marks,
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
        view,
    ));
    commands.insert_resource(StaminaHudSpawned);
}

/// Update: `Initialize` once per harvest site, the running tweens, the
/// presenter's calls in order and the delayed decrease; then dress the view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    time: Res<Time>,
    real: Res<Time<Real>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    layouts: Res<UiLayouts>,
    model: Option<Res<HarvestPlayerModel>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    user: Option<Res<crate::server::ClientUserData>>,
    (stack, entry, site): (
        Res<crate::ui_layers::UiLayerStack>,
        Option<Res<crate::entry::EntrySequence>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    mut hud: ResMut<HarvestStaminaHud>,
    mut views: Query<(
        &StaminaHudView,
        &mut Visibility,
        &mut Transform,
        &mut UiPrefabView,
    )>,
) {
    let now = time.elapsed_secs_f64();
    let harvest_site = site
        .as_deref()
        .filter(|site| site.category == "harvest")
        .map(|site| site.site_id);
    let hud = &mut *hud;
    if harvest_site != hud.screen_site {
        // A new screen instance (the calls queued this frame are its own).
        let calls = std::mem::take(&mut hud.calls);
        let shakes = hud.shakes;
        *hud = HarvestStaminaHud {
            calls,
            shakes,
            screen_site: harvest_site,
            ..Default::default()
        };
    }
    if !hud.seated {
        let Some((marks, ..)) = views.iter().next() else {
            return;
        };
        hud.view_active = marks.serialized_active;
        hud.alpha = marks.serialized_alpha;
        hud.seated = true;
    }
    let max = crate::server::stamina_max();
    if let (Some(site_id), false) = (harvest_site, hud.initialized) {
        match (model.as_deref(), max) {
            (Some(model), Some(max)) => {
                let stamina = server_stamina(model.stamina);
                let (normal, enhance, boost) = stamina_gage_rate(stamina, max);
                let pass = user
                    .as_deref()
                    .is_some_and(|user| user.has_mysekai_colorful_pass(real.elapsed_secs()));
                let rate = match (pass, stamina.boost > 0) {
                    (_, true) => boost,
                    (true, false) => enhance,
                    (false, false) => normal,
                };
                hud.decrease_fill = Some(rate);
                hud.gauge = Some(stamina_view_state(stamina, max));
                hud.initialized = true;
                info!(
                    "[harvest-hud] Initialize site {site_id}: UpdateStaminaDecreaseGage({rate:.3}) (colorful pass {pass}), SetupStaminaGage({stamina:?}) over maxima {max:?}, stock {}",
                    boost_stamina_stock_count(stamina.boost, max)
                );
            }
            (model, max) => {
                warn_once!(
                    "[harvest-hud] Initialize waits: harvest model {}, stamina masters {}",
                    model.is_some(),
                    max.is_some()
                );
            }
        }
    }
    hud.step(time.delta_secs());
    for call in std::mem::take(&mut hud.calls) {
        match call {
            Call::Show => hud.do_show(),
            Call::Hide => hud.do_hide(),
            Call::ActionStart => {
                hud.pending = None;
                hud.do_show();
            }
            Call::LackOfStamina => {
                hud.do_show();
                let mut rng = super::Rng(0x5354_414D_494E_4100 ^ hud.shakes);
                hud.shakes = hud.shakes.wrapping_add(1);
                let points = law::vector_shake_points(
                    SHAKE_DURATION,
                    SHAKE_STRENGTH,
                    SHAKE_VIBRATO,
                    SHAKE_RANDOMNESS,
                    true,
                    |min, max| min + rng.next_f32() * (max - min),
                );
                hud.shake = Some(PointTween::new(points, SegmentEase::Linear));
                info!(
                    "[harvest-hud] LackOfStamina: Show, DOShakePosition(0.3, (20, 0, 0), 115, 90, false, true) from offset ({:.2}, {:.2})",
                    hud.offset.x, hud.offset.y
                );
            }
            Call::Hit(before, after) => {
                let Some(max) = max else {
                    error!("[harvest-hud] a hit: the stamina masters are not loaded; the gauge keeps its state");
                    continue;
                };
                let Some(configs) = configs.as_deref() else {
                    error!("[harvest-hud] a hit: the client config panel is not loaded; DecreaseStaminaAnimation does not run");
                    continue;
                };
                let wait = configs.float(crate::client_config::KEY_STAMINA_UI_ANIMATION_AWAIT_TIME);
                let state = stamina_view_state(server_stamina(after), max);
                info!(
                    "[harvest-hud] hit: stamina {before:?} -> {after:?}: the gauge at the state after (fill {:.3}, the 0.5 s change not ported); DecreaseStaminaAnimation after {wait} s",
                    state.fill
                );
                hud.gauge = Some(state);
                hud.pending = Some((now + f64::from(wait), after));
            }
        }
    }
    if let Some((due, after)) = hud.pending {
        if now >= due {
            hud.pending = None;
            if let Some(max) = max {
                hud.decrease_fill = Some(stamina_view_state(server_stamina(after), max).fill);
            }
            info!(
                "[harvest-hud] DecreaseStaminaAnimation: the delay passed; the decrease bar at the fill after the hit ({:?}, the 0.5 s change not ported), ResetActiveStamina, Hide",
                hud.decrease_fill
            );
            hud.do_hide();
        }
    }
    let (Ok(window), Some(root_canvas), Some(doc)) = (
        windows.single(),
        root_canvas.as_deref(),
        layouts.document(KEY),
    ) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let visible =
        crate::entry::hud_open(entry.as_deref()) && stack.on_field() && harvest_site.is_some();
    for (marks, mut visibility, mut transform, mut view) in &mut views {
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
        transform.translation.z = view_z(doc.nodes.len(), scale);
        view.set_visible(&marks.root, hud.active);
        view.set_visible(&marks.group, hud.view_active);
        view.set_alpha(&marks.group, hud.alpha);
        view.set_anchored_position(&marks.view, marks.anchored + hud.offset.truncate());
        if let Some(rate) = hud.decrease_fill {
            view.set_fill(&format!("@{}", marks.decrease), rate);
        }
        if let Some(state) = hud.gauge.as_ref() {
            apply_state(&mut view, &layouts, doc, marks, state);
        }
    }
}

/// `UpdateStaminaView` + `UpdateStaminaGageRate` on the view (the menu
/// dialog's writes of the same component).
fn apply_state(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    doc: &UiPrefab,
    marks: &StaminaHudView,
    state: &StaminaViewState,
) {
    let set_color = |view: &mut UiPrefabView, graphic: i64, entry: Option<usize>| {
        let color = match entry {
            None => [1.0; 4],
            Some(entry) => match layouts.palette_color(entry) {
                Some(color) => color,
                None => {
                    error_once!(
                        "[harvest-hud] {}: palette entry {entry} is not on this UI root; the stamina colours stay as serialized",
                        doc.prefab
                    );
                    return;
                }
            },
        };
        view.set_graphic_color(graphic, color);
    };
    set_color(
        view,
        marks.gauge,
        state.gauge_palette.then_some(STAMINA_GAUGE_COLOR_ENTRY),
    );
    view.set_visible(&marks.gradient, state.gradient);
    set_sprite(view, doc, marks, marks.icon, state.icon);
    set_color(
        view,
        marks.icon,
        state.icon_palette.then_some(STAMINA_EMPTY_ICON_COLOR_ENTRY),
    );
    view.set_visible(&marks.on_stock, state.on_stock);
    view.set_visible(&marks.on_no_stock, !state.on_stock);
    view.set_visible(&marks.boost_count_node, state.boost_count.is_some());
    if let Some(count) = state.boost_count {
        set_sprite(
            view,
            doc,
            marks,
            marks.boost_count,
            &format!("txt_stamina_{count}"),
        );
    }
    view.set_fill(&format!("@{}", marks.gauge), state.fill);
}
