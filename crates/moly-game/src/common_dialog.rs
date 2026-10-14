//! `Common1ButtonDialog` and `Common2ButtonDialog`, the dialogs
//! `ScreenManager.Show1ButtonDialog` and `Show2ButtonDialog` show for every
//! screen. A caller opens one with its body and its button labels, then
//! reads the answer; the dialog's views, its taps and the back key are here.
//!
//! The answers follow the dialogs' own handlers. Common2's OK button calls
//! `onClickOK`; its cancel button, the close button, a tap outside the
//! window (`OnCloseExternal`) and the back key (`OnHardwareBackKeyProcess`)
//! all call `onClickCancel`. Common1 has one button, and every one of these
//! closes it as its OK. Each closes the dialog afterwards.
//!
//! A tap answers only when it is pressed while the dialog is shown: the
//! press that opened the dialog does not close it on its release.
//!
//! A caller either reads the answer ([`open`], then [`answer`]) or leaves
//! the dialog to close itself ([`open_detached`]: the dialogs shown with no
//! answer callback, `DialogUtility.ShowCommon1ButtonDialog(body)`).
//!
//! One common dialog is shown at a time here; a second open while one is
//! shown is refused by name.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::server::client::instrument_env;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};

const COMMON1: &str = "Common1";
const COMMON2: &str = "Common2";
const FOOTER_BUTTON: &str = "WindowRoot/FooterButtons/UIPartsCommonButton";
const FOOTER_TEXT: &str = "WindowRoot/FooterButtons/UIPartsCommonButton/Text";
const MESSAGE_BODY: &str = "ContentRoot/Content/MessageBody";
const CLOSE_BUTTON: &str = "WindowRoot/UIPartsCloseButton";
const WINDOW_ROOT: &str = "WindowRoot";
const TABS: &str = "WindowRoot/Tabs";
const DIALOG_Z: f32 = 100.0;

/// The button labels every caller of these dialogs passes (the shell charset
/// takes them with the callers' own wordings).
pub(crate) const WORDINGS: &[&str] = &["WORD_OK", "WORD_CANCEL"];

/// The dialog's buttons, as the wording keys of their labels.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Buttons {
    One { ok: &'static str },
    Two { ok: &'static str, cancel: &'static str },
}

#[derive(Resource, Default)]
pub(crate) struct CommonDialogs {
    shown: Option<Shown>,
    /// The shown dialog's answer (true: OK), until its caller reads it.
    answered: Option<(DialogId, bool)>,
}

struct Shown {
    id: DialogId,
    key: &'static str,
    caller: &'static str,
    body: String,
    /// The label texts, `[OK]` or `[cancel, OK]` as the view's buttons.
    labels: Vec<String>,
    shown_at: f32,
    /// The native instrument that taps the OK button that many seconds
    /// after the dialog shows, and whether it has.
    instrument: Option<&'static str>,
    tapped: bool,
    /// No caller reads the answer: the dialog closes itself on it.
    detached: bool,
    /// The taps have not been read since the dialog opened (the ones read
    /// first belong to the screen under it).
    fresh: bool,
    /// A tap was pressed while the dialog is shown.
    pressed: bool,
}

/// The two dialogs' views. Common2's OK, cancel and close buttons are
/// [`crate::two_button_dialog::resolve`]'s (the footer buttons found by
/// their serialized labels); Common1's single footer button is its OK (the
/// dialogs' button fields are not decoded).
#[derive(Component)]
pub(crate) struct CommonDialogView {
    key: &'static str,
    /// Common2: `[cancel, OK]`; Common1: `[OK]`.
    buttons: Vec<String>,
    /// The buttons' label texts, in the same order.
    labels: Vec<String>,
    close: String,
}

/// Registration: the dialogs' state and views. Their taps ([`click`]) and
/// drawing ([`place`]) run in the screens' tap chain (`home_action`), before
/// the screens that read the answers and after the action buttons' tap flag
/// and the back key.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<CommonDialogs>()
        .add_systems(Update, spawn_when_ready);
}

/// Show a common dialog: `body` is its message, already formatted, and
/// `buttons` its labels. `instrument` names the environment variable of the
/// native instrument that taps its OK. `None` when the dialog cannot show
/// (the reason is logged).
pub(crate) fn open(
    world: &mut World,
    caller: &'static str,
    body: String,
    buttons: Buttons,
    instrument: Option<&'static str>,
) -> Option<DialogId> {
    show(world, caller, body, buttons, instrument, false)
}

/// Show a common dialog that closes itself on its answer (no caller reads
/// it).
pub(crate) fn open_detached(
    world: &mut World,
    caller: &'static str,
    body: String,
    buttons: Buttons,
) -> Option<DialogId> {
    show(world, caller, body, buttons, None, true)
}

/// The wording `key` on the root; an absent one is named and drawn empty.
pub(crate) fn wording(world: &World, caller: &str, key: &str) -> String {
    match world.resource::<UiLayouts>().wordings.get(key) {
        Some(text) => text.clone(),
        None => {
            error!("[common-dialog] {caller}: {key} is not on the root; it is drawn empty");
            String::new()
        }
    }
}

fn show(
    world: &mut World,
    caller: &'static str,
    body: String,
    buttons: Buttons,
    instrument: Option<&'static str>,
    detached: bool,
) -> Option<DialogId> {
    if let Some(shown) = &world.resource::<CommonDialogs>().shown {
        error!(
            "[common-dialog] {caller}: {} {:?} from {} is still shown; this dialog is refused",
            shown.key, shown.id, shown.caller
        );
        return None;
    }
    let (dialog, key, keys) = match buttons {
        Buttons::One { ok } => (DialogType::Common1ButtonDialog, COMMON1, vec![ok]),
        Buttons::Two { ok, cancel } => (DialogType::Common2ButtonDialog, COMMON2, vec![cancel, ok]),
    };
    let labels: Vec<String> = keys.iter().map(|key| wording(world, caller, key)).collect();
    let id = {
        let mut screens = world.resource_mut::<ScreenManager>();
        match screens.show_dialog(dialog, DisplayLayerType::LayerDialog, DialogBackKey::Close, caller) {
            Ok(id) => {
                screens.open_dialog(id);
                screens.dialog_open_finished(id);
                id
            }
            Err(reason) => {
                error!("[common-dialog] {caller}: {reason}");
                return None;
            }
        }
    };
    let shown_at = world.resource::<Time>().elapsed_secs();
    let mut dialogs = world.resource_mut::<CommonDialogs>();
    dialogs.answered = None;
    dialogs.shown = Some(Shown {
        id,
        key,
        caller,
        body,
        labels,
        shown_at,
        instrument,
        tapped: false,
        detached,
        fresh: true,
        pressed: false,
    });
    Some(id)
}

/// The answer of the dialog `id` (true: OK), once it has one; the dialog is
/// then closed. A dialog closed from outside answers as its cancel.
pub(crate) fn answer(world: &mut World, id: DialogId) -> Option<bool> {
    let dialogs = world.resource::<CommonDialogs>();
    let shown = dialogs.shown.as_ref().filter(|shown| shown.id == id);
    let answered = dialogs.answered.filter(|(answered, _)| *answered == id).map(|(_, ok)| ok);
    let gone = !world.resource::<ScreenManager>().dialogs().any(|(open, ..)| open == id);
    let ok = match (answered, shown) {
        (Some(ok), _) => ok,
        (None, Some(shown)) if gone => {
            warn!("[common-dialog] {} {id:?} from {} was closed from outside", shown.key, shown.caller);
            shown.key == COMMON1
        }
        (None, Some(_)) => return None,
        (None, None) => {
            warn!("[common-dialog] {id:?} is not a shown common dialog");
            return Some(false);
        }
    };
    dismiss(world, id);
    Some(ok)
}

/// Close the dialog `id` without reading an answer (its caller went away).
pub(crate) fn dismiss(world: &mut World, id: DialogId) {
    world.resource_scope(|world, mut dialogs: Mut<CommonDialogs>| {
        close(&mut dialogs, &mut world.resource_mut::<ScreenManager>(), id);
    });
}

fn close(dialogs: &mut CommonDialogs, screens: &mut ScreenManager, id: DialogId) {
    if dialogs.shown.as_ref().is_some_and(|shown| shown.id == id) {
        dialogs.shown = None;
        dialogs.answered = None;
    }
    if screens.dialogs().any(|(open, ..)| open == id) {
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
    }
}

/// Spawn the two dialogs' views once their documents are ready.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut spawned: Local<bool>,
) {
    if *spawned || ![COMMON1, COMMON2].iter().all(|key| layouts.ready(key, &server)) {
        return;
    }
    *spawned = true;
    for key in [COMMON1, COMMON2] {
        let doc = layouts.document(key).expect("ready document");
        let mut view = UiPrefabView::new(key, SITEMAP_LAYER);
        view.set_visible(TABS, false);
        let (buttons, labels, close) = if key == COMMON2 {
            match crate::two_button_dialog::resolve(doc) {
                Ok(dialog) => (
                    vec![dialog.negative, dialog.positive],
                    vec![dialog.negative_label, dialog.positive_label],
                    dialog.close,
                ),
                Err(reason) => {
                    error!("[common-dialog] {key}: {reason}; its buttons are not bound");
                    (Vec::new(), Vec::new(), CLOSE_BUTTON.to_owned())
                }
            }
        } else {
            let ids = |suffix: &str| -> Vec<String> {
                doc.nodes
                    .iter()
                    .filter(|node| node.path.ends_with(suffix))
                    .map(|node| format!("@{}", node.game_object_id))
                    .collect()
            };
            let buttons = ids(FOOTER_BUTTON);
            let mut labels = ids(FOOTER_TEXT);
            if buttons.len() != 1 || labels.len() != 1 {
                error!(
                    "[common-dialog] {key}: {} footer buttons and {} texts, not one; its button is not labelled",
                    buttons.len(),
                    labels.len()
                );
                labels.clear();
            }
            (buttons, labels, CLOSE_BUTTON.to_owned())
        };
        info!("[common-dialog] {key} view spawned: buttons {buttons:?} (the last is OK), close {close}");
        commands.spawn((
            CommonDialogView { key, buttons, labels, close },
            Visibility::Hidden,
            Transform::from_xyz(0., 0., DIALOG_Z),
            RenderLayers::layer(SITEMAP_LAYER),
            view,
        ));
    }
}

/// Draw the shown dialog; its OK instrument's tap.
pub(crate) fn place(
    mut dialogs: ResMut<CommonDialogs>,
    layouts: Res<UiLayouts>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut views: Query<(&CommonDialogView, &mut Visibility, &mut Transform, &mut UiPrefabView)>,
    mut injected: MessageWriter<GestureEvent>,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    for (common, mut visible, mut transform, mut view) in &mut views {
        crate::canvas::set_scale(&mut transform, scale);
        let Some(shown) = dialogs.shown.as_mut().filter(|shown| shown.key == common.key) else {
            visible.set_if_neq(Visibility::Hidden);
            continue;
        };
        visible.set_if_neq(Visibility::Inherited);
        view.set_text(MESSAGE_BODY, shown.body.clone());
        for (path, text) in common.labels.iter().zip(&shown.labels) {
            view.set_text(path, text.clone());
        }
        // The instrument: a tap at the OK button's drawn centre (pressed and
        // released at once, after the dialog's first taps were read).
        if shown.tapped || shown.fresh {
            continue;
        }
        let Some(secs) = shown
            .instrument
            .and_then(instrument_env)
            .and_then(|raw| raw.trim().parse::<f32>().ok())
        else {
            continue;
        };
        if time.elapsed_secs() - shown.shown_at < secs {
            continue;
        }
        let Some(rect) = common.buttons.last().and_then(|ok| view.rect(&layouts, ok, size)) else {
            continue;
        };
        let centre = rect.center();
        let position = Vec2::new(
            centre.x * scale + window.width() / 2.0,
            window.height() / 2.0 - centre.y * scale,
        );
        shown.tapped = true;
        shown.pressed = true;
        info!(
            "[common-dialog] instrument {}: tap injected at ({:.0},{:.0}) on {}'s OK",
            shown.instrument.unwrap_or_default(),
            position.x,
            position.y,
            shown.caller
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

/// Taps and the back key on the shown dialog (modal: every tap is its). A
/// detached dialog closes here on its answer, and is forgotten when it was
/// closed from outside.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut dialogs: ResMut<CommonDialogs>,
    mut screens: ResMut<ScreenManager>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<(&CommonDialogView, &UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let Some((id, key, caller, detached)) = dialogs
        .shown
        .as_ref()
        .map(|shown| (shown.id, shown.key, shown.caller, shown.detached))
    else {
        gestures.clear();
        back_keys.clear();
        return;
    };
    if detached && !screens.dialogs().any(|(open, ..)| open == id) {
        warn!("[common-dialog] {key} {id:?} from {caller} was closed from outside");
        dialogs.shown = None;
        dialogs.answered = None;
        return;
    }
    if dialogs.answered.is_some() {
        return;
    }
    let shown = dialogs.shown.as_mut().expect("shown dialog");
    if std::mem::take(&mut shown.fresh) {
        gestures.clear();
        back_keys.clear();
        return;
    }
    let mut answer = None;
    if back_keys.read().any(|event| event.id == id) {
        info!("[common-dialog] {key} ({caller}): back key (OnHardwareBackKeyProcess)");
        answer = Some(key == COMMON1);
    }
    let mut taps = Vec::new();
    for event in gestures.read() {
        if !event.kind.is_tap_family() {
            continue;
        }
        consumed.0 = true;
        match event.state {
            GestureState::Began => shown.pressed = true,
            GestureState::End if shown.pressed => taps.push(event.position),
            GestureState::End => {
                info!("[common-dialog] {key} ({caller}): the release of a tap pressed before the dialog showed: nothing");
            }
            GestureState::Moved => {}
        }
    }
    if answer.is_none() && !taps.is_empty() {
        answer = tap_answer(&layouts, &windows, root_canvas.as_deref(), &views, key, caller, &taps);
    }
    let Some(ok) = answer else {
        return;
    };
    if detached {
        info!("[common-dialog] {key} ({caller}): closed");
        close(&mut dialogs, &mut screens, id);
    } else {
        dialogs.answered = Some((id, ok));
    }
}

/// The answer of the first tap on a button, the close button or outside the
/// window (true: OK).
fn tap_answer(
    layouts: &UiLayouts,
    windows: &Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<&crate::canvas::RootCanvas>,
    views: &Query<(&CommonDialogView, &UiPrefabView)>,
    key: &'static str,
    caller: &'static str,
    taps: &[Vec2],
) -> Option<bool> {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas) else {
        return None;
    };
    let (common, view) = views.iter().find(|(common, _)| common.key == key)?;
    let size = root_canvas.size(window);
    let scale = root_canvas.scale(window);
    for position in taps {
        let canvas = Vec2::new(position.x - window.width() / 2.0, window.height() / 2.0 - position.y) / scale;
        let hit = |path: &str| {
            view.rect(layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        };
        let ok = common.buttons.last().is_some_and(|button| hit(button.as_str()));
        let cancel = key == COMMON2 && common.buttons.first().is_some_and(|button| hit(button.as_str()));
        let external = hit(common.close.as_str()) || !hit(WINDOW_ROOT);
        if ok || cancel || external {
            info!(
                "[common-dialog] {key} ({caller}): tap on {}",
                if ok {
                    "OK"
                } else if cancel {
                    "cancel"
                } else {
                    "the close button or outside (OnCloseExternal)"
                }
            );
            return Some(ok || (external && key == COMMON1));
        }
        info!("[common-dialog] {key} ({caller}): tap inside the window off its buttons: nothing");
    }
    None
}
