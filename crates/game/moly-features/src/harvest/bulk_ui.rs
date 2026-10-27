//! The source bulk button is a disjoint subtree of ShellHarvest. Its pointer
//! owner is independent of tap recognition so leaving cancels even at full.

use super::bulk::{BulkHarvest, Input, Phase};
use crate::{
    canvas::RootCanvas,
    gesture::{UiPointerEvent, UiPointerPhase},
    ui_layout::{UiLayouts, UiPrefabView},
};
use bevy::{camera::visibility::RenderLayers, prelude::*, window::PrimaryWindow};
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use moly_law::bulk_harvest::UnavailableReason;

const KEY: &str = "ShellHarvest";
const SOURCE_BUTTON: &str = "Sekai.Mysekai.BulkHarvestActionButton";

#[derive(Component)]
pub struct BulkButtonView {
    button: String,
    cannot: String,
    balloon: String,
    message: String,
    icon: String,
    cover: String,
    effects: Vec<TapEffect>,
    pressed: bool,
}

struct TapEffect {
    graphics: Vec<i64>,
    colors: moly_law::ui::graphic_tap_effect::TapEffectColors,
    state: moly_law::ui::graphic_tap_effect::TapEffectState,
    current: [f32; 4],
}

#[derive(Component)]
pub struct BulkGaugeView {
    root: String,
    fill: String,
    group: String,
    max: String,
    alpha: f32,
    showing: bool,
    fade: Option<moly_law::ui::graphic_tap_effect::ColorTween>,
}

#[derive(Component)]
pub struct BulkGaugeParticle {
    gauge: Entity,
}

pub fn spawn_gauge(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    views: Query<(), With<BulkGaugeView>>,
) {
    if !views.is_empty() || !layouts.ready("HUD", &server) {
        return;
    }
    let Some(doc) = layouts.document("HUD") else {
        return;
    };
    let Ok(gauge) = component(doc, "Sekai.Mysekai.BulkHarvestChargeGaugeView") else {
        return;
    };
    let root = format!("@{}", gauge.path_id);
    let root_path = &doc.nodes[doc.find(&root).expect("source gauge")].path;
    let mut view = UiPrefabView::new("HUD", crate::balloon::BALLOON_LAYER);
    for node in &doc.nodes {
        if node.path != *root_path
            && !root_path.starts_with(&format!("{}/", node.path))
            && !node.path.starts_with(&format!("{root_path}/"))
        {
            view.set_visible(&format!("@{}", node.game_object_id), false);
        }
    }
    // ShowBulkHarvestChargeGauge activates the containing head-up display.
    for node in &doc.nodes {
        if root_path.starts_with(&format!("{}/", node.path)) {
            view.set_visible(&format!("@{}", node.game_object_id), true);
        }
    }
    let marks = BulkGaugeView {
        root,
        fill: reference(doc, &gauge.fields["_gaugeImage"]).expect("source gauge image"),
        group: reference(doc, &gauge.fields["_canvasGroup"]).expect("source gauge CanvasGroup"),
        max: reference(doc, &gauge.fields["_maxEffectRoot"]).expect("source gauge max root"),
        alpha: 0.0,
        showing: false,
        fade: None,
    };
    let gauge = commands
        .spawn((
            marks,
            view,
            Transform::default(),
            Visibility::Hidden,
            RenderLayers::layer(crate::balloon::BALLOON_LAYER),
        ))
        .id();
    let document =
        server.load("moly://ui-bulkharvest/gauge/fx_mysekai_bulk_harvest_gauge_charge.json");
    // Both primary-root children are active; there is no Animator or clip.
    // UIParticle replaces its own serialized zero scale in ModifyScale.
    for (index, node) in [
        "fx_mysekai_charge_standby/UIParticle",
        "fx_mysekai_charge_complete/UIParticle",
    ]
    .into_iter()
    .enumerate()
    {
        commands.spawn((
            BulkGaugeParticle { gauge },
            ChildOf(gauge),
            Transform::from_xyz(0.0, 0.0, 0.001 + index as f32 * 0.001),
            Visibility::Hidden,
            crate::ui_particle::UiParticleHost {
                document: document.clone(),
                directory: "ui-bulkharvest/gauge".into(),
                root: "fx_mysekai_bulk_harvest_gauge_charge".into(),
                node: node.into(),
                canvas: gauge,
                layer: crate::balloon::BALLOON_LAYER,
                alpha: 0.0,
            },
        ));
    }
}

pub fn place_gauge_particles(
    state: Res<BulkHarvest>,
    time: Res<Time<Real>>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<RootCanvas>>,
    gauges: Query<(&BulkGaugeView, &UiPrefabView)>,
    mut particles: Query<(
        &BulkGaugeParticle,
        &mut crate::ui_particle::UiParticleHost,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let (Ok(window), Some(canvas)) = (windows.single(), canvas) else {
        return;
    };
    let size = canvas.size(window);
    let showing = state.phase == Phase::Charging && state.progress(time.elapsed_secs_f64()) >= 1.0;
    for (particle, mut host, mut transform, mut visibility) in &mut particles {
        let Ok((gauge, view)) = gauges.get(particle.gauge) else {
            continue;
        };
        host.alpha = if showing { gauge.alpha } else { 0.0 };
        visibility.set_if_neq(if showing {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(rect) = view.rect(&layouts, &gauge.max, size) {
            let center = rect.center();
            transform.translation.x = center.x;
            transform.translation.y = center.y;
        }
    }
}

pub fn place_gauge(
    time: Res<Time<Real>>,
    state: Res<BulkHarvest>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<RootCanvas>>,
    mut views: Query<(
        &mut BulkGaugeView,
        &mut UiPrefabView,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let (Ok(window), Some(canvas)) = (windows.single(), canvas) else {
        return;
    };
    for (mut marks, mut view, mut transform, mut visibility) in &mut views {
        let showing = state.phase == Phase::Charging;
        if marks.showing != showing {
            // Kill(true) completes the previous target before starting this fade.
            if marks.fade.is_some() {
                marks.alpha = if marks.showing { 1.0 } else { 0.0 };
            }
            marks.showing = showing;
            marks.fade = Some(moly_law::ui::graphic_tap_effect::ColorTween::new(
                [0., 0., 0., if showing { 1.0 } else { 0.0 }],
                0.1,
            ));
        }
        let current = marks.alpha;
        if let Some(fade) = marks.fade.as_mut() {
            let (color, complete) = fade.update([0., 0., 0., current], time.delta_secs());
            marks.alpha = color[3];
            if complete {
                marks.fade = None;
            }
        }
        visibility.set_if_neq(if marks.alpha > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        transform.scale = Vec3::splat(canvas.scale(window));
        let progress = state.progress(time.elapsed_secs_f64());
        view.set_visible(&marks.root, true);
        view.set_alpha(&marks.group, marks.alpha);
        view.set_fill(&marks.fill, progress);
        view.set_visible(&marks.max, showing && progress >= 1.0);
    }
}

pub fn component<'a>(doc: &'a UiPrefab, class: &str) -> Result<&'a UiComponent, String> {
    doc.nodes
        .iter()
        .flat_map(|n| &n.components)
        .find(|c| c.class == class)
        .ok_or_else(|| format!("{class} is absent from {}", doc.prefab))
}

pub fn reference(doc: &UiPrefab, value: &serde_json::Value) -> Result<String, String> {
    let (file, id) = if let Some(pair) = value.as_array() {
        (
            pair.first().and_then(|v| v.as_i64()),
            pair.get(1).and_then(|v| v.as_i64()),
        )
    } else {
        (value["m_FileID"].as_i64(), value["m_PathID"].as_i64())
    };
    if file != Some(0) {
        return Err("UI runtime reference is not local".into());
    }
    let id = id
        .filter(|id| *id != 0)
        .ok_or("UI runtime reference is null")?;
    let key = format!("@{id}");
    doc.find(&key)?;
    Ok(key)
}

pub fn referenced_component<'a>(
    doc: &'a UiPrefab,
    fields: &serde_json::Value,
    field: &str,
) -> Result<(&'a UiComponent, String), String> {
    let key = reference(doc, &fields[field])?;
    let id: i64 = key[1..].parse().map_err(|_| "invalid identity")?;
    let node = &doc.nodes[doc.find(&key)?];
    let c = node
        .components
        .iter()
        .find(|c| c.path_id == id)
        .ok_or("UI reference is not a component")?;
    Ok((c, key))
}

pub fn spawn(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    views: Query<(), With<BulkButtonView>>,
) {
    if !views.is_empty() || !layouts.ready(KEY, &server) {
        return;
    }
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    // This component does not exist before JP7; that is an ordinary absence.
    let Ok(button) = component(doc, SOURCE_BUTTON) else {
        return;
    };
    let marks = (|| -> Result<BulkButtonView, String> {
        let (_, cannot) = referenced_component(doc, &button.fields, "_cannotButton")?;
        let (balloon, balloon_node) = referenced_component(doc, &button.fields, "_cannotBalloon")?;
        let message = reference(doc, &balloon.fields["message"])?;
        let (loader, _) = referenced_component(doc, &button.fields, "_iconLoader")?;
        Ok(BulkButtonView {
            button: format!("@{}", button.path_id),
            cannot,
            balloon: balloon_node,
            message,
            icon: reference(doc, &loader.fields["targetGraphic"])?,
            cover: reference(doc, &button.fields["coverImage"])?,
            effects: crate::menu_shell::tap_effect(doc, button.path_id)
                .into_iter()
                .map(|effect| {
                    let colors = effect
                        .config
                        .colors(|entry| layouts.palette_color(entry))
                        .expect("source bulk tap colors");
                    TapEffect {
                        graphics: effect.graphics,
                        colors,
                        state: Default::default(),
                        current: colors.default,
                    }
                })
                .collect(),
            pressed: false,
        })
    })()
    .unwrap_or_else(|reason| panic!("BulkHarvestActionButton source contract: {reason}"));
    let root = doc.nodes[doc.find(&marks.button).expect("source button")]
        .path
        .clone();
    let mut view = UiPrefabView::new(KEY, crate::balloon::BALLOON_LAYER);
    for node in &doc.nodes {
        if !node.path.starts_with(&format!("{root}/"))
            && node.path != root
            && !root.starts_with(&format!("{}/", node.path))
        {
            view.set_visible(&format!("@{}", node.game_object_id), false);
        }
    }
    view.set_visible(&marks.balloon, false);
    layouts.register_runtime_texture(
        "bulk-harvest-icon",
        "moly://ui/action-icon/icon_action_allcollect_wh.png",
        &server,
    );
    view.set_texture(&marks.icon, "bulk-harvest-icon");
    commands.spawn((
        marks,
        view,
        Transform::default(),
        Visibility::Hidden,
        RenderLayers::layer(crate::balloon::BALLOON_LAYER),
    ));
}

pub fn read_input(
    mut events: MessageReader<UiPointerEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<RootCanvas>>,
    layouts: Res<UiLayouts>,
    views: Query<(&BulkButtonView, &UiPrefabView)>,
    stack: Res<crate::ui_layers::UiLayerStack>,
    eligibility: crate::interaction::InteractionEligibility,
    mut state: ResMut<BulkHarvest>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>,
) {
    let events: Vec<_> = events.read().copied().collect();
    if !state.visible || !stack.on_field() || stack.exists_dialog() {
        if let Some(press) = state.press {
            state.input.push(Input::Cancel(press.pointer));
        }
        return;
    }
    let (Ok(window), Some(canvas), Ok((marks, view))) = (windows.single(), canvas, views.single())
    else {
        return;
    };
    let size = canvas.size(window);
    let scale = canvas.scale(window);
    let over = |position: Vec2| {
        let p = Vec2::new(
            position.x - window.width() * 0.5,
            window.height() * 0.5 - position.y,
        ) / scale;
        view.rect(&layouts, &marks.button, size)
            .is_some_and(|r| r.active && r.contains(p))
    };
    for event in events {
        match event.phase {
            UiPointerPhase::Down if over(event.position) => {
                // The bulk hold blocks ordinary field input itself. Qualify
                // only a new press; keep reading release and cancellation.
                if state.phase == Phase::None && !eligibility.field_input_open() {
                    continue;
                }
                consumed.0 = true;
                if state.phase == Phase::None {
                    if state.available == UnavailableReason::None {
                        state.input.push(Input::Down(event.pointer_id));
                    } else {
                        state.input.push(Input::Cannot);
                    }
                }
            }
            UiPointerPhase::Move if !over(event.position) => {
                state.input.push(Input::Cancel(event.pointer_id))
            }
            UiPointerPhase::Up => {
                if over(event.position) {
                    state.input.push(Input::Up(event.pointer_id));
                } else {
                    state.input.push(Input::Cancel(event.pointer_id));
                }
            }
            UiPointerPhase::Cancel => state.input.push(Input::Cancel(event.pointer_id)),
            _ => {}
        }
    }
}

pub fn reason_key(reason: UnavailableReason) -> Option<&'static str> {
    Some(match reason {
        UnavailableReason::None | UnavailableReason::TreasureBoxNotSynced => return None,
        UnavailableReason::PassExpired => "MSG_MYSEKAI_BULK_HARVEST_CANNOT_PASS_EXPIRED",
        UnavailableReason::PossessionLimit => "MSG_MYSEKAI_BULK_HARVEST_CANNOT_POSSESSION_LIMIT",
        UnavailableReason::StaminaShortage => "MSG_MYSEKAI_BULK_HARVEST_CANNOT_STAMINA_SHORTAGE",
        UnavailableReason::ToolShortage => "MSG_MYSEKAI_BULK_HARVEST_CANNOT_TOOL_SHORTAGE",
        UnavailableReason::NoTarget => "MSG_MYSEKAI_BULK_HARVEST_CANNOT_NO_TARGET",
    })
}

pub fn place(
    time: Res<Time<Real>>,
    state: Res<BulkHarvest>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<RootCanvas>>,
    stack: Res<crate::ui_layers::UiLayerStack>,
    mut views: Query<(
        &mut BulkButtonView,
        &mut UiPrefabView,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let (Ok(window), Some(canvas)) = (windows.single(), canvas) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for (mut marks, mut view, mut transform, mut visible) in &mut views {
        visible.set_if_neq(if state.visible && stack.on_field() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        transform.scale = Vec3::splat(canvas.scale(window));
        view.set_visible(&marks.cannot, state.available != UnavailableReason::None);
        view.set_visible(&marks.cover, state.available != UnavailableReason::None);
        let pressed = state.phase == Phase::Charging;
        let changed = marks.pressed != pressed;
        marks.pressed = pressed;
        for effect in &mut marks.effects {
            if changed {
                if pressed {
                    effect.state.on_pressed(&effect.colors);
                } else {
                    effect.state.on_released(&effect.colors);
                }
            }
            if let Some(color) = effect.state.update(effect.current, time.delta_secs()) {
                effect.current = color;
            }
            for graphic in &effect.graphics {
                view.set_graphic_color(*graphic, effect.current);
            }
        }
        let age = state.balloon.map(|(_, at)| (now - at) as f32);
        let alpha = age
            .map_or(0.0, |age| {
                if age < 0.1 {
                    let t = age / 0.1;
                    t * (2.0 - t)
                } else if age < 2.1 {
                    1.0
                } else {
                    let t = (age - 2.1) / 0.1;
                    1.0 - t * (2.0 - t)
                }
            })
            .clamp(0.0, 1.0);
        view.set_visible(&marks.balloon, alpha > 0.0);
        view.set_alpha(&marks.balloon, alpha);
        if let Some((reason, _)) = state.balloon {
            if let Some(text) = reason_key(reason).and_then(|key| layouts.wordings.get(key)) {
                view.set_text(&marks.message, text.clone());
            }
        }
    }
}
