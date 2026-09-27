//! The harvest point summary: `HarvestUtility.SaveHarvestPoint`,
//! `HarvestUtility.ShowHarvestPointSummary` and the screen layer it opens,
//! `ScreenLayerMysekaiHarvestSummary` (`MysekaiHarvestSummary`, 637).
//!
//! The source:
//! - The cannon move's pre-action, leaving the home site, saves a harvest
//!   point: `SaveHarvestPoint(userGamedata.totalExp, GetCurrentEventPoint())`
//!   writes a `MysekaiHarvestPoint` (the player's total experience, the
//!   current event's point entry and its event item) to the device's
//!   persistent data.
//! - The cannon move's end action, arriving home, waits 0.5 s and calls
//!   `ShowHarvestPointSummary`:
//!   - `GetHarvestPoint` loads the saved point (none: nothing happens) and
//!     returns a new point whose experience is `totalExp - saved exp` and
//!     whose event lists are the differences to the saved ones;
//!   - the event point and event id are the last event entry's (0 and 0
//!     without one), the badge count the last event item's quantity (0);
//!   - `IfNeedShowBreakTimeNotification(wasHarvestInBreakTime, ...)`;
//!   - nothing more when `eventPoint <= 0` and `exp < 1`;
//!   - `AddScreen(MysekaiHarvestSummary)` unless that layer is active, then
//!     `GetLayerComponent` of it (a null component ends the call),
//!     `Play(exp, eventId, eventPoint, badge)`, the break-time flag cleared
//!     when it was set, and the saved point deleted.
//! - The layer: its constructor sets the hide position `(0, 400, 0)`;
//!   `OnBoot` reports the boot done and runs `Initialize`
//!   (`_animationCanvasGroup.alpha = 0`, its RectTransform's anchored
//!   position the hide position's x and y); `OnWillExit` answers OK.
//! - `Play(exp, eventId, eventPoint, badge)`: `_rankExpText.text =
//!   exp.ToString()`; `SetupEventPoint(eventPoint)` (break time and no point:
//!   the content active with a fixed text; a point above 0: the content
//!   active with the point; else the content inactive); `SetupEventBadge(
//!   eventId, badge)` (the same three cases on the badge count, with the
//!   event's badge texture); `Initialize`; the previous sequence killed; then
//!   one DOTween sequence:
//!   - `Append(DOAnchorPosY(rt, 0, 0.75).SetEase(OutCubic))`,
//!     `Join(canvasGroup.DOFade(1, 0.75))`, `Join(bgImage.DOFade(1, 0.375))`;
//!   - `AppendInterval(1.0)`;
//!   - `Append(DOAnchorPosY(rt, hide y, 0.75).SetEase(OutCubic))`,
//!     `Join(canvasGroup.DOFade(0, 0.1875))`, `Join(bgImage.DOFade(0,
//!     0.375))`;
//!   - `OnComplete(RemoveScreen(MysekaiHarvestSummary))`, `Play()`.
//!   The fades carry no ease of their own: they take the tween settings'
//!   default ease.
//!
//! Product mapping, named:
//! - The saved point lives in this resource for the session: the device's
//!   persistent data is not ported.
//! - The server-decided values come from the mock panel
//!   [`HarvestSummaryMock`]. The event path (a current event, its point and
//!   badge differences, `CalculatorHarvestEventPoint` /
//!   `CalculatorHarvestEventItem`) and the break-time notification are not
//!   ported: the mock has no current event and no break time, which is the
//!   source's path with both absent.
//! - `AddScreen` mounts at the screen manager's next step, so `Play` starts
//!   when the layer is active, not inside the call.
//! - The sequence is stepped as its position (the sum of the frame deltas)
//!   with each nested tween at the position minus its insertion time,
//!   clamped to its duration; each nested tween reads its start value on
//!   its first step. The sequence completes when its position reaches its
//!   duration (2.5 s).
//! - The Play frame's delta counts. In the source the cannon move's end
//!   action waits with `UniTask.Delay(0.5 s, PlayerLoopTiming.Update)`; the
//!   delay's promise completes inside UniTask's Update runner and runs the
//!   continuation at once, and UniTask injects that runner first in Unity's
//!   Update phase, ahead of the behaviour updates. `ShowHarvestPointSummary`
//!   and `Play` run there, so `DOTweenComponent.Update` of the same frame
//!   updates the new sequence, and `TweenManager.Update` adds the frame's
//!   whole delta to its position (no first-update skip). The sequence thus
//!   completes when the deltas from the Play frame's own sum to 2.5 s, which
//!   is less than 2.5 s of elapsed time after the Play frame: the log
//!   reports both.
//! - Drawing needs the layer's prefab on the UI root (layout key
//!   [`KEY`]); a root without it runs the calls and the sequence and draws
//!   nothing, reported once.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiPrefab;
use moly_law::ui::dotween::Ease;
use serde_json::Value;

use super::{MenuScreenType, ScreenManager};
use crate::balloon::BALLOON_LAYER;
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The layout key of the layer's prefab.
pub(crate) const KEY: &str = "HarvestSummary";
const PRESENTER: &str = "Sekai.ScreenLayerMysekaiHarvestSummary";
const SCREEN: MenuScreenType = MenuScreenType::MysekaiHarvestSummary;
/// `_hidePosition`, set by the constructor.
const HIDE_POSITION: Vec2 = Vec2::new(0.0, 400.0);
const SHOW_Y: f32 = 0.0;
/// The sequence's durations and its interval.
const MOVE_SECONDS: f32 = 0.75;
const CANVAS_IN_SECONDS: f32 = 0.75;
const BG_SECONDS: f32 = 0.375;
const INTERVAL_SECONDS: f32 = 1.0;
const CANVAS_OUT_SECONDS: f32 = 0.1875;

/// The server-decided inputs, as the mock panel sets them:
/// - `player_total_exp`: `UserGamedata.totalExp`, the player's total
///   experience (`MOLY_HARVEST_SUMMARY_MOCK_PLAYER_TOTAL_EXP`, default
///   100000, our chosen value);
/// - `trip_exp`: the experience the server's replies add to it over one
///   harvest trip, applied when a harvest site becomes current while a
///   harvest point is saved (`MOLY_HARVEST_SUMMARY_MOCK_TRIP_EXP`, default
///   120, our chosen value);
/// - no current event (`GetCurrentEventPoint` returns null) and
///   `HarvestUserDataManager._wasHarvestInBreakTime` false.
#[derive(Resource, Debug)]
pub(crate) struct HarvestSummaryMock {
    pub(crate) player_total_exp: i32,
    pub(crate) trip_exp: i32,
    pub(crate) was_harvest_in_break_time: bool,
}

impl HarvestSummaryMock {
    fn panel(name: &str, default: i32) -> i32 {
        match std::env::var(name) {
            Ok(raw) => raw.trim().parse().unwrap_or_else(|_| {
                warn!(
                    "[harvest-summary] mock panel: {name}={raw:?} is not an int; using {default}"
                );
                default
            }),
            Err(_) => default,
        }
    }
}

impl Default for HarvestSummaryMock {
    fn default() -> Self {
        Self {
            player_total_exp: Self::panel("MOLY_HARVEST_SUMMARY_MOCK_PLAYER_TOTAL_EXP", 100_000),
            trip_exp: Self::panel("MOLY_HARVEST_SUMMARY_MOCK_TRIP_EXP", 120),
            was_harvest_in_break_time: false,
        }
    }
}

/// `MysekaiHarvestPoint` (the event lists are empty: no current event).
#[derive(Clone, Copy, Debug)]
struct HarvestPoint {
    player_exp: i32,
}

/// One nested tween of the sequence.
#[derive(Clone, Copy, Debug)]
enum Channel {
    RootY,
    CanvasAlpha,
    BgAlpha,
}

#[derive(Clone, Copy, Debug)]
struct Nested {
    channel: Channel,
    at: f32,
    end: f32,
    duration: f32,
    ease: Ease,
    /// Start and change, read on the first step.
    started: Option<(f32, f32)>,
}

impl Nested {
    fn new(channel: Channel, at: f32, end: f32, duration: f32, ease: Ease) -> Self {
        Self {
            channel,
            at,
            end,
            duration,
            ease,
            started: None,
        }
    }
}

/// The running `Play` sequence.
#[derive(Debug)]
struct Sequence {
    position: f32,
    duration: f32,
    nested: Vec<Nested>,
    started_at: f64,
}

impl Sequence {
    fn new(default_ease: Ease, now: f64) -> Self {
        let out = MOVE_SECONDS + INTERVAL_SECONDS;
        let nested = vec![
            Nested::new(Channel::RootY, 0.0, SHOW_Y, MOVE_SECONDS, Ease::OutCubic),
            Nested::new(
                Channel::CanvasAlpha,
                0.0,
                1.0,
                CANVAS_IN_SECONDS,
                default_ease,
            ),
            Nested::new(Channel::BgAlpha, 0.0, 1.0, BG_SECONDS, default_ease),
            Nested::new(
                Channel::RootY,
                out,
                HIDE_POSITION.y,
                MOVE_SECONDS,
                Ease::OutCubic,
            ),
            Nested::new(
                Channel::CanvasAlpha,
                out,
                0.0,
                CANVAS_OUT_SECONDS,
                default_ease,
            ),
            Nested::new(Channel::BgAlpha, out, 0.0, BG_SECONDS, default_ease),
        ];
        let duration = nested
            .iter()
            .map(|n| n.at + n.duration)
            .fold(0.0_f32, f32::max);
        Self {
            position: 0.0,
            duration,
            nested,
            started_at: now,
        }
    }

    /// One step: the nested tweens the position has reached write their
    /// value in insertion order. Returns whether the sequence completed.
    fn step(&mut self, dt: f32, values: &mut Values) -> bool {
        self.position += dt;
        let complete = self.duration <= self.position;
        let position = if complete {
            self.duration
        } else {
            self.position
        };
        for nested in &mut self.nested {
            if position < nested.at {
                continue;
            }
            let local = (position - nested.at).min(nested.duration);
            let current = values.get(nested.channel);
            let end = nested.end;
            let (start, change) = *nested
                .started
                .get_or_insert_with(|| (current, end - current));
            values.set(
                nested.channel,
                start + change * nested.ease.evaluate(local, nested.duration),
            );
        }
        complete
    }
}

/// The three animated values.
#[derive(Clone, Copy, Debug)]
struct Values {
    root_y: f32,
    canvas_alpha: f32,
    bg_alpha: f32,
}

impl Values {
    fn get(&self, channel: Channel) -> f32 {
        match channel {
            Channel::RootY => self.root_y,
            Channel::CanvasAlpha => self.canvas_alpha,
            Channel::BgAlpha => self.bg_alpha,
        }
    }

    fn set(&mut self, channel: Channel, value: f32) {
        match channel {
            Channel::RootY => self.root_y = value,
            Channel::CanvasAlpha => self.canvas_alpha = value,
            Channel::BgAlpha => self.bg_alpha = value,
        }
    }
}

/// The prefab's bound fields.
struct Bindings {
    canvas_group: String,
    bg_graphic: i64,
    bg_color: [f32; 4],
    rank_exp_text: String,
    event_point_content: String,
    event_badge_content: String,
}

/// The summary's state.
#[derive(Resource, Default)]
pub(crate) struct HarvestSummary {
    saved: Option<HarvestPoint>,
    /// The trip's experience was added for the saved point.
    trip_granted: bool,
    /// `Play(exp, eventId, eventPoint, badge)` waiting for the layer.
    pending: Option<(i32, i32, i32, i32)>,
    sequence: Option<Sequence>,
    values: Option<Values>,
    bindings: Option<Bindings>,
    warned_prefab: bool,
    shown_at: Option<f64>,
}

#[derive(Component)]
pub(crate) struct HarvestSummaryRoot;

/// `HarvestUtility.SaveHarvestPoint(userGamedata.totalExp,
/// GetCurrentEventPoint())`, called by the cannon move's pre-action when it
/// leaves the home site.
#[allow(dead_code)]
pub(crate) fn save_harvest_point(world: &mut World, caller: &str) {
    let exp = world.resource::<HarvestSummaryMock>().player_total_exp;
    let mut summary = world.resource_mut::<HarvestSummary>();
    summary.saved = Some(HarvestPoint { player_exp: exp });
    summary.trip_granted = false;
    info!(
        "[harvest-summary] {caller}: SaveHarvestPoint(userGamedata.totalExp {exp} (mock), GetCurrentEventPoint null (mock: no current event)): saved for this session (the device's persistent data is not ported)"
    );
}

/// `HarvestUtility.ShowHarvestPointSummary`, called by the cannon move's end
/// action 0.5 s after it arrives home.
#[allow(dead_code)]
pub(crate) fn show_harvest_point_summary(world: &mut World, caller: &str) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let (total_exp, was_in_break_time) = {
        let mock = world.resource::<HarvestSummaryMock>();
        (mock.player_total_exp, mock.was_harvest_in_break_time)
    };
    let Some(saved) = world.resource::<HarvestSummary>().saved else {
        info!("[harvest-summary] t {now:.4} {caller}: ShowHarvestPointSummary: GetHarvestPoint loads no saved point; nothing");
        return;
    };
    // GetHarvestPoint: the experience is the difference to the saved one;
    // without a current event the event lists hold nothing.
    let exp = total_exp - saved.player_exp;
    let (event_point, event_id, badge) = (0, 0, 0);
    info!(
        "[harvest-summary] t {now:.4} {caller}: ShowHarvestPointSummary: GetHarvestPoint exp {total_exp} - {} = {exp}, eventPoint {event_point}, eventId {event_id}, badge {badge}; IfNeedShowBreakTimeNotification(wasHarvestInBreakTime {was_in_break_time}): nothing",
        saved.player_exp
    );
    if event_point <= 0 && exp < 1 {
        info!("[harvest-summary] eventPoint <= 0 and exp < 1: no summary");
        return;
    }
    let mut screens = world.resource_mut::<ScreenManager>();
    if !screens.is_active(SCREEN) {
        screens.add_screen(SCREEN, "HarvestUtility.ShowHarvestPointSummary");
    }
    let mut summary = world.resource_mut::<HarvestSummary>();
    summary.pending = Some((exp, event_id, event_point, badge));
    summary.saved = None;
    info!(
        "[harvest-summary] Play({exp}, {event_id}, {event_point}, {badge}) once {SCREEN:?} is active; PersistentDataUtility.Delete<MysekaiHarvestPoint>"
    );
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
    let presenter = &doc
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.class == PRESENTER)
        .unwrap_or_else(|| panic!("{KEY}: no {PRESENTER}"))
        .fields;
    let bg_graphic = pointer(&presenter["_bgImage"], "_bgImage");
    let bg = doc
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.path_id == bg_graphic)
        .unwrap_or_else(|| panic!("{KEY}: _bgImage component"));
    let bg_color = bg.fields["m_Color"]
        .as_array()
        .filter(|c| c.len() == 4)
        .map(|c| [0, 1, 2, 3].map(|k| c[k].as_f64().expect("bg colour") as f32))
        .unwrap_or_else(|| panic!("{KEY}: _bgImage has no colour"));
    let path = |field: &str| node_path(doc, pointer(&presenter[field], field));
    Bindings {
        canvas_group: path("_animationCanvasGroup"),
        bg_graphic,
        bg_color,
        rank_exp_text: path("_rankExpText"),
        event_point_content: path("_eventPointContent"),
        event_badge_content: path("_eventBadgeContent"),
    }
}

/// Update: the prefab view, once the root's prefab is ready.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut summary: ResMut<HarvestSummary>,
) {
    if summary.bindings.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let bindings = bind(layouts.document(KEY).expect("ready summary prefab"));
    info!(
        "[harvest-summary] {KEY} view: canvas group {}, bg graphic @{} colour {:?}, rank exp text {}, event point content {}, event badge content {}",
        bindings.canvas_group,
        bindings.bg_graphic,
        bindings.bg_color,
        bindings.rank_exp_text,
        bindings.event_point_content,
        bindings.event_badge_content
    );
    commands.spawn((
        HarvestSummaryRoot,
        Visibility::Hidden,
        Transform::from_xyz(0.0, 0.0, 110.0),
        RenderLayers::layer(BALLOON_LAYER),
        UiPrefabView::new(KEY, BALLOON_LAYER),
    ));
    summary.bindings = Some(bindings);
}

/// Update, after the screen manager's step: the trip's mock experience, the
/// pending `Play`, one sequence step, the view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    time: Res<Time>,
    site: Option<Res<crate::site::SiteActive>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<UiLayouts>,
    mut mock: ResMut<HarvestSummaryMock>,
    mut summary: ResMut<HarvestSummary>,
    mut screens: ResMut<ScreenManager>,
    mut roots: Query<
        (&mut Transform, &mut UiPrefabView, &mut Visibility),
        With<HarvestSummaryRoot>,
    >,
) {
    let summary = &mut *summary;
    let now = time.elapsed_secs_f64();
    if summary.saved.is_some()
        && !summary.trip_granted
        && site
            .as_deref()
            .is_some_and(|site| site.category == "harvest")
    {
        summary.trip_granted = true;
        let before = mock.player_total_exp;
        mock.player_total_exp += mock.trip_exp;
        info!(
            "[harvest-summary] mock: the server's replies over the harvest trip raise userGamedata.totalExp {before} -> {}",
            mock.player_total_exp
        );
    }
    if let Some((exp, event_id, event_point, badge)) = summary.pending {
        if screens.is_active(SCREEN) {
            summary.pending = None;
            let default_ease = if layouts.tween_defaults().is_some() {
                Ease::OutQuad
            } else {
                warn!("[harvest-summary] this UI root carries no tween defaults: the fades take OutQuad, the default of the JP root's tween settings");
                Ease::OutQuad
            };
            // Initialize: canvas alpha 0, anchored position the hide
            // position; the bg image keeps its colour until its fade starts.
            let bg_alpha = summary
                .bindings
                .as_ref()
                .map_or(1.0, |bindings| bindings.bg_color[3]);
            summary.values = Some(Values {
                root_y: HIDE_POSITION.y,
                canvas_alpha: 0.0,
                bg_alpha,
            });
            let sequence = Sequence::new(default_ease, now);
            info!(
                "[harvest-summary] t {now:.4}: ScreenLayerMysekaiHarvestSummary.Play({exp}, {event_id}, {event_point}, {badge}): rank exp text \"{exp}\", event point content inactive, event badge content inactive; Initialize (alpha 0, anchored {HIDE_POSITION}); sequence {:.4}s: y -> {SHOW_Y} over {MOVE_SECONDS} OutCubic, canvas -> 1 over {CANVAS_IN_SECONDS} and bg -> 1 over {BG_SECONDS} ({default_ease:?}); interval {INTERVAL_SECONDS}; y -> {} over {MOVE_SECONDS} OutCubic, canvas -> 0 over {CANVAS_OUT_SECONDS}, bg -> 0 over {BG_SECONDS}; OnComplete RemoveScreen({SCREEN:?})",
                sequence.duration,
                HIDE_POSITION.y
            );
            summary.sequence = Some(sequence);
            summary.shown_at = Some(now);
            if let (Some(bindings), Ok((_, mut view, _))) =
                (summary.bindings.as_ref(), roots.single_mut())
            {
                view.set_text(&bindings.rank_exp_text, exp.to_string());
                view.set_visible(&bindings.event_point_content, false);
                view.set_visible(&bindings.event_badge_content, false);
            } else if !summary.warned_prefab {
                summary.warned_prefab = true;
                error!("[harvest-summary] this UI root carries no {KEY} prefab (ScreenLayerMysekaiHarvestSummary): the calls and the sequence run, nothing is drawn (named missing input)");
            }
        }
    }
    let dt = time.delta_secs();
    if let (Some(sequence), Some(values)) = (summary.sequence.as_mut(), summary.values.as_mut()) {
        let complete = sequence.step(dt, values);
        if complete {
            info!(
                "[harvest-summary] t {now:.4}: sequence complete at position {:.4}s (the frame deltas summed from the Play frame's own; {:.4}s elapsed after the Play frame): y {:.4}, canvas alpha {:.4}, bg alpha {:.4}; OnComplete",
                sequence.position,
                now - sequence.started_at,
                values.root_y,
                values.canvas_alpha,
                values.bg_alpha
            );
            screens.remove_screen(SCREEN, "ScreenLayerMysekaiHarvestSummary.Play OnComplete");
        }
    }
    let values = summary.values;
    if summary
        .sequence
        .as_ref()
        .is_some_and(|s| s.duration <= s.position)
    {
        summary.sequence = None;
    }
    let Ok((mut transform, mut view, mut visibility)) = roots.single_mut() else {
        return;
    };
    let shown = if screens.is_active(SCREEN) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != shown {
        *visibility = shown;
    }
    if let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) {
        transform.scale = Vec3::splat(root_canvas.scale(window));
    }
    if let (Some(bindings), Some(values)) = (summary.bindings.as_ref(), values) {
        view.set_anchored_position(
            &bindings.canvas_group,
            Vec2::new(HIDE_POSITION.x, values.root_y),
        );
        view.set_alpha(&bindings.canvas_group, values.canvas_alpha);
        let [r, g, b, _] = bindings.bg_color;
        view.set_graphic_color(bindings.bg_graphic, [r, g, b, values.bg_alpha]);
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<HarvestSummaryMock>()
        .init_resource::<HarvestSummary>()
        .add_systems(
            Update,
            (spawn_when_ready, advance).chain().after(super::advance),
        );
}
