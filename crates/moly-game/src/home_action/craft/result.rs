//! The craft result dialog's view: `CraftResultSubWindowDialog` (dialog
//! 377), shown after the craft sequence's hold by
//! `CraftPreview.OpenCreateResponseDialogAsync` and awaited to its close.
//!
//! What it shows (`ShowSubWindowDialog(messageBody, onClose,
//! allowCloseExternal true)`, then `Setup(CraftResultData(name, preview
//! texture, experience before, after, count, first craft), previewScale)`):
//! - the message body: `WORD_FIXTURE_CREATE_RESULT` from the fixture lists,
//!   `WORD_TOOL_CREATE_RESULT` from the tool list;
//! - `SetupCraftCount`: the count text off below 2, else
//!   `WORD_MULTIPLY_FORMAT(count)`;
//! - `SetupFirstCraftBonus`: the first-craft balloon's `Enable` is the first
//!   craft, its text `WORD_FIRST_CRAFT_BONUS(quantity)` with the quantity of
//!   the `craft_mysekai_fixture_first_bonus` row, 0 without the row;
//! - `SetupMysekaiRankGauge`: the rank view (`_rankGaugeViewRoot`, which
//!   holds the gauge and, under it, the first-craft balloon) on only when
//!   experience was obtained (`IsObtainedExp`: after minus before above 0);
//! - `balloon.Initialize()`: the name balloon off; a tap on the item button
//!   (`OnClick`) shows it with the target's name.
//!
//! `Open` plays `se_craft_result`. The sub window's close buttons (the close
//! area and the window base, `allowCloseExternal`) and the back key close
//! it; its close is `OnCloseCreateResponseDialog`.
//!
//! Named gaps: the item image is the 3D preview's render texture, and the 3D
//! preview is not built (the image is off); the rank gauge reads the rank
//! master through `MysekaiRankModel`, which the client does not carry, and
//! the prefab's gauge and experience texts are placeholders (`28`, `99999`,
//! `100`): a craft that obtained experience names it and keeps the rank view
//! off, and with it the first-craft balloon under it; the rank experience
//! text's value is lost in the reconstructed source (`SetupMysekaiRankExp`);
//! the name balloon's close on an outside tap and the sub window's fade are
//! not played.
//!
//! Instrument (off by default; game mode reads none):
//! `MOLY_CRAFT_RESULT_CLOSE_SECS` taps the close area that many seconds
//! after the dialog shows, near its top-left corner (the close area spans
//! the screen, and the item button covers its centre).

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::Value;

use super::{CraftScreen, DialogStep, Stage};
use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::DialogBackKeyEvent;
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// `CraftResultSubWindowDialog`'s document (optional).
pub(crate) const KEY: &str = "CraftResult";

/// What the shown dialog draws.
#[derive(Clone, Debug)]
pub(super) struct ResultShown {
    pub(super) body: &'static str,
    pub(super) name: String,
    pub(super) count: i32,
    pub(super) is_first_craft: bool,
    /// The `craft_mysekai_fixture_first_bonus` quantity (`?? 0`).
    pub(super) first_craft_bonus: i32,
    /// `IsObtainedExp`.
    pub(super) obtained_exp: bool,
    pub(super) shown_at: f32,
    pub(super) balloon: bool,
    pub(super) close_tapped: bool,
}

fn component<'a>(
    doc: &'a UiPrefab,
    selector: &str,
    suffix: &str,
) -> Result<&'a UiComponent, String> {
    let node = &doc.nodes[doc.find(selector)?];
    node.components
        .iter()
        .find(|c| c.class.ends_with(suffix))
        .ok_or_else(|| format!("{} lacks source component {suffix}", node.path))
}

fn pointer(value: &Value) -> Result<i64, String> {
    let p = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or("expected a decoded PPtr")?;
    if p[0].as_i64() != Some(0) {
        return Err("the PPtr is external".into());
    }
    p[1].as_i64().ok_or_else(|| "invalid PPtr path id".into())
}

fn field(fields: &Value, name: &str) -> Result<String, String> {
    match pointer(&fields[name])? {
        0 => Err(format!("required source reference {name} is null")),
        id => Ok(format!("@{id}")),
    }
}

struct Bindings {
    body: String,
    item_image: String,
    item_button: String,
    count_text: String,
    first_bonus: String,
    first_bonus_text: String,
    rank_root: String,
    name_balloon: String,
    name_text: String,
    close_buttons: Vec<String>,
}

fn bind(doc: &UiPrefab) -> Result<Bindings, String> {
    let dialog = component(doc, &doc.prefab, ".CraftResultSubWindowDialog")?
        .fields
        .clone();
    let first_bonus = field(&dialog, "firstCraftBonusBalloon")?;
    let first_fields = component(doc, &first_bonus, ".UIPartsBalloon")?
        .fields
        .clone();
    let name_balloon = field(&dialog, "balloon")?;
    let name_fields = component(doc, &name_balloon, ".UIPartsCommonBalloon")?
        .fields
        .clone();
    let window = component(
        doc,
        &field(&dialog, "subWindowComponent")?,
        ".SubWindowComponent",
    )?
    .fields
    .clone();
    let close_buttons = window["closeButtons"]
        .as_array()
        .ok_or("SubWindowComponent closeButtons")?
        .iter()
        .map(|button| match pointer(button)? {
            0 => Err("a null close button".to_owned()),
            id => Ok(format!("@{id}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Bindings {
        body: field(&dialog, "messageBodyText")?,
        item_image: field(&dialog, "itemImage")?,
        item_button: field(&dialog, "button")?,
        count_text: field(&dialog, "craftCountText")?,
        first_bonus_text: field(&first_fields, "message")?,
        first_bonus,
        rank_root: field(&dialog, "_rankGaugeViewRoot")?,
        name_text: field(&name_fields, "mainText")?,
        name_balloon,
        close_buttons,
    })
}

#[derive(Component)]
pub(crate) struct CraftResultView {
    bindings: Bindings,
}

/// Spawn the view once its document is ready (a root without it shows the
/// dialog without a view, named).
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut spawned: Local<bool>,
) {
    if *spawned || !layouts.ready(KEY, &server) {
        return;
    }
    *spawned = true;
    match bind(layouts.document(KEY).expect("ready document")) {
        Ok(bindings) => {
            let mut view = UiPrefabView::new(KEY, SITEMAP_LAYER);
            view.set_visible(&bindings.item_image, false);
            commands.spawn((
                CraftResultView { bindings },
                Visibility::Hidden,
                Transform::from_xyz(0., 0., 30.),
                RenderLayers::layer(SITEMAP_LAYER),
                view,
            ));
            info!("[craft] CraftResultSubWindowDialog view spawned");
        }
        Err(reason) => error!(
            "[craft] CraftResultSubWindowDialog could not be bound ({reason}); it is shown without a view"
        ),
    }
}

/// The view exists (the dialog is drawn and awaited to its close).
pub(super) fn has_view(world: &mut World) -> bool {
    world
        .query::<&CraftResultView>()
        .iter(world)
        .next()
        .is_some()
}

fn format_one(layouts: &UiLayouts, key: &str, arg: String) -> Option<String> {
    let format = layouts.wordings.get(key)?;
    moly_law::text::custom_text_mesh::format_wording(format, &[arg]).ok()
}

/// Draw the shown dialog; the close instrument's tap.
pub(crate) fn place(
    mut screen: ResMut<CraftScreen>,
    layouts: Res<UiLayouts>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut views: Query<(
        &CraftResultView,
        &mut Visibility,
        &mut Transform,
        &mut UiPrefabView,
    )>,
    mut injected: MessageWriter<GestureEvent>,
    mut named: Local<(bool, bool)>,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    for (result, mut visible, mut transform, mut view) in &mut views {
        transform.scale = Vec3::splat(scale);
        let Some(shown) = screen.result.as_mut() else {
            *visible = Visibility::Hidden;
            continue;
        };
        *visible = Visibility::Inherited;
        let b = &result.bindings;
        match layouts.wordings.get(shown.body) {
            Some(text) => view.set_text(&b.body, text.clone()),
            None => {
                if !named.0 {
                    named.0 = true;
                    error!(
                        "[craft] {} is not on the root; the result body keeps its prefab text",
                        shown.body
                    );
                }
            }
        }
        // SetupCraftCount.
        match (shown.count >= 2)
            .then(|| format_one(&layouts, "WORD_MULTIPLY_FORMAT", shown.count.to_string()))
            .flatten()
        {
            Some(text) => {
                view.set_visible(&b.count_text, true);
                view.set_text(&b.count_text, text);
            }
            None => view.set_visible(&b.count_text, false),
        }
        // SetupFirstCraftBonus.
        view.set_visible(&b.first_bonus, shown.is_first_craft);
        if shown.is_first_craft {
            if let Some(text) = format_one(
                &layouts,
                "WORD_FIRST_CRAFT_BONUS",
                shown.first_craft_bonus.to_string(),
            ) {
                view.set_text(&b.first_bonus_text, text);
            }
        }
        // SetupMysekaiRankGauge: the rank view is on only with experience
        // obtained, and its gauge is not built.
        view.set_visible(&b.rank_root, false);
        if shown.obtained_exp && !named.1 {
            named.1 = true;
            error!(
                "[craft] CraftResultSubWindowDialog: IsObtainedExp, and the rank gauge (MysekaiRankModel over the rank master) is not built: the rank view stays off, with the first-craft balloon under it"
            );
        }
        // The name balloon (OnClick).
        view.set_visible(&b.name_balloon, shown.balloon);
        if shown.balloon {
            view.set_text(&b.name_text, shown.name.clone());
        }
        // The close instrument: a tap inside the close area's top-left corner.
        if shown.close_tapped {
            continue;
        }
        let Some(secs) = crate::server::client::instrument_env("MOLY_CRAFT_RESULT_CLOSE_SECS")
            .and_then(|raw| raw.trim().parse::<f32>().ok())
        else {
            continue;
        };
        if time.elapsed_secs() - shown.shown_at < secs {
            continue;
        }
        let Some(rect) = b
            .close_buttons
            .first()
            .and_then(|path| view.rect(&layouts, path, size))
        else {
            continue;
        };
        let corner = -rect.size * rect.pivot + Vec2::new(8.0, rect.size.y - 8.0);
        let centre = rect.world.transform_point3(corner.extend(0.)).truncate();
        let position = Vec2::new(
            centre.x * scale + window.width() / 2.0,
            window.height() / 2.0 - centre.y * scale,
        );
        shown.close_tapped = true;
        info!(
            "[craft] instrument: tap injected at ({:.0},{:.0}) on the result dialog's close area",
            position.x, position.y
        );
        injected.write(GestureEvent {
            kind: GestureKind::Tap,
            state: GestureState::End,
            position,
            delta: Vec2::ZERO,
            ui_owned: false,
        });
    }
}

/// Taps and the back key on the shown result dialog (modal): a close button
/// or the back key closes it; the item button shows the name balloon.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut screen: ResMut<CraftScreen>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<(&CraftResultView, &UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let dialog = match screen.stage {
        Stage::Dialogs {
            next: DialogStep::Result {
                shown: Some(dialog),
            },
            ..
        } => dialog,
        _ => {
            gestures.clear();
            back_keys.clear();
            return;
        }
    };
    if screen.answer.is_some() {
        return;
    }
    if back_keys.read().any(|event| event.id == dialog) {
        info!("[craft] CraftResultSubWindowDialog: back key: Close");
        screen.answer = Some(true);
        return;
    }
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let Ok((result, view)) = views.single() else {
        return;
    };
    let size = root_canvas.size(window);
    let scale = root_canvas.scale(window);
    let b = &result.bindings;
    for position in taps {
        consumed.0 = true;
        let canvas = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        let hit = |path: &str| {
            view.rect(&layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        };
        // The item button sits over the window base: it takes the tap.
        if hit(b.item_button.as_str()) {
            if let Some(shown) = screen.result.as_mut() {
                shown.balloon = true;
                info!("[craft] CraftResultSubWindowDialog: item tap: the name balloon shows");
            }
            continue;
        }
        if b.close_buttons.iter().any(|path| hit(path.as_str())) {
            info!("[craft] CraftResultSubWindowDialog: tap on a close button: Close");
            screen.answer = Some(true);
            break;
        }
    }
}
