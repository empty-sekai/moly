//! Field menu chrome backed by the serialized host prefab.
//!
//! Runtime bindings come from MysekaiMenuUIContent's component references.
//! Rendering and input share UiPrefabView's resolved geometry; site transitions,
//! menus, camera reset and frame capture retain their existing owners.

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::{json::JsonAsset, ui_layout::UiPrefab};
use serde_json::Value;

use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::frame_capture::CaptureFrame;
use crate::gesture::{GestureEvent, GestureState, UiPointerEvent, UiPointerPhase};
use moly_law::ui::custom_button as button_rule;
use moly_law::ui::graphic_tap_effect as tap_rule;
use crate::site::SiteActive;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use crate::ui_layout::{Pointer, UiLayouts, UiPrefabView};

const SITES_DATA: &str = "moly://site/sites.json";
const CHROME_MOVE_DURATION: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellHost {
    Home,
    MyRoom,
    Harvest,
    Delivery,
}

impl ShellHost {
    const ALL: [Self; 4] = [Self::Home, Self::MyRoom, Self::Harvest, Self::Delivery];

    fn for_site(site: &SiteActive) -> Self {
        match site.category.as_str() {
            "housing_home" => Self::Home,
            "housing_room" => Self::MyRoom,
            "harvest" => Self::Harvest,
            "delivery" => Self::Delivery,
            category => panic!("field menu has no host for site category {category}"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Home => "ShellHome",
            Self::MyRoom => "ShellMyRoom",
            Self::Harvest => "ShellHarvest",
            Self::Delivery => "ShellDelivery",
        }
    }

    fn shows_site_map(self) -> bool {
        self != Self::MyRoom
    }

    /// The host of the harvest site category (MysekaiSiteCategory 2); the
    /// other three hosts are housing_home 0, housing_room 1 and delivery 3.
    fn is_harvest(self) -> bool {
        self == Self::Harvest
    }
}

/// UIPartsEventBreakTimeGauge's ANIM_OFFSET_Y: a hidden gauge sits this far
/// above its cached anchored position.
const GAUGE_HIDE_OFFSET_Y: f32 = 400.;

/// UIPartsEventBreakTimeGauge's position tween duration.
const GAUGE_SLIDE_SECONDS: f32 = 0.3;

/// The server's event state the break-time gauge reads.
///
/// `EventBreakTimeUtility.IsSetEventBreakTimeMaster` asks
/// `EventUtility.GetMasterEventInSession` for the master event whose session
/// holds the server clock, and `EventBreakTimeModel.Initialize` reads the
/// user's break-time record (`UserDataManager.userEventBreakTime`). The event
/// schedule and the user's record are the server's; the server model carries
/// neither yet, and the runtime roots carry no events master. Until they do,
/// [`EVENT_BREAK_TIME_SERVER`] is the one named value.
pub(crate) struct EventBreakTimeServer {
    /// The master event in session, `None` for no event.
    pub(crate) event_in_session: Option<SessionEvent>,
    /// The user's break-time record. It has no value: the gauge view the
    /// model sets up is not built, so a record cannot be supplied.
    pub(crate) user_break_time: Option<NoUserBreakTime>,
}

/// The fields of a master event `IsSetEventBreakTimeMaster` reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionEvent {
    /// `MasterEvent.startAt` (epoch ms).
    pub(crate) start_at: i64,
    /// `MasterEvent.aggregateAt` (epoch ms).
    pub(crate) aggregate_at: i64,
    /// `MasterEvent.eventBreakTimeId` (nullable).
    pub(crate) event_break_time_id: Option<i32>,
}

/// A user break-time record; uninhabited until the gauge view exists.
#[derive(Clone, Copy, Debug)]
pub(crate) enum NoUserBreakTime {}

/// No event in session, no user record (named server value; see
/// [`EventBreakTimeServer`]).
pub(crate) const EVENT_BREAK_TIME_SERVER: EventBreakTimeServer =
    EventBreakTimeServer { event_in_session: None, user_break_time: None };

/// `EventBreakTimeUtility.IsSetEventBreakTimeMaster`: an event in session
/// with a break time, started (`startAt <= now`) and before its aggregation
/// (`now < aggregateAt`).
fn is_set_event_break_time_master(event: Option<SessionEvent>, now_ms: impl FnOnce() -> i64) -> bool {
    let Some(event) = event else { return false; };
    if event.event_break_time_id.is_none() {
        return false;
    }
    let now = now_ms();
    event.start_at <= now && now < event.aggregate_at
}

/// The gauge behind `MysekaiMenuUIContent._eventBreakTimeController`: the
/// controller's serialized `_gauge` (a UIPartsEventBreakTimeGauge), the
/// anchored position its first CacheInitialPosition call keeps, the position
/// this host last wrote, and the gauge's running position tween.
struct BreakTimeGauge {
    gauge: String,
    initial: Vec2,
    position: Vec2,
    slide: Option<GaugeSlide>,
}

/// The hide tween: DOAnchorPosY from the y at its start to the hidden y over
/// 0.3 s with Ease 5 (InQuad); x stays where it is.
struct GaugeSlide {
    from_y: f32,
    to_y: f32,
    elapsed: f32,
}

impl BreakTimeGauge {
    fn from_document(doc: &UiPrefab, content: &moly_assets::ui_layout::UiComponent) -> Option<Self> {
        // Whether the region's MysekaiMenuUIContent declares the field, by the
        // layout's region tag: the shared root's untagged CN extractions have
        // no break-time controller (nor gauge); the JP class serializes it.
        let declared = match doc.source.region.as_deref() {
            None => false,
            Some("jp") => true,
            Some(region) => panic!("MysekaiMenuUIContent: no declared field set for region {region}"),
        };
        let reference = content.fields.get("_eventBreakTimeController");
        if !declared {
            assert!(
                reference.is_none(),
                "{}: MysekaiMenuUIContent serializes _eventBreakTimeController, which its region's class does not declare",
                doc.prefab
            );
            return None;
        }
        let reference = reference.unwrap_or_else(|| {
            panic!("{}: MysekaiMenuUIContent lacks its declared _eventBreakTimeController", doc.prefab)
        });
        let controller = local_reference(reference, "_eventBreakTimeController")?;
        let component = doc.nodes.iter().flat_map(|node| node.components.iter())
            .find(|c| c.path_id == controller)
            .expect("field menu break-time controller component");
        assert_eq!(component.class, "Sekai.EventBreakTimeController", "field menu break-time controller class");
        let gauge = component.fields.get("_gauge")
            .expect("EventBreakTimeController._gauge is not decoded in this layout");
        // HideGauge does nothing when the gauge reference is null.
        let gauge = local_reference(gauge, "_gauge")?;
        let index = doc.find(&format!("@{gauge}")).unwrap_or_else(|error| panic!("{error}"));
        let initial = Vec2::from_array(doc.nodes[index].rect.anchored_position);
        Some(Self { gauge: format!("@{gauge}"), initial, position: initial, slide: None })
    }

    /// The last branch of MysekaiMenuUIContent.Setup, which runs when this
    /// host's screen is set up: on the harvest site it calls
    /// Initialize(false, true, 0), everywhere else HideGauge(false) and
    /// StopUpdate (which only stops the controller's per-frame refresh).
    fn setup(&mut self, host: ShellHost, server: &EventBreakTimeServer, now_ms: impl FnOnce() -> i64, view: &mut UiPrefabView) {
        if host.is_harvest() {
            self.initialize(server, now_ms, view);
        } else {
            self.hide(false, view);
        }
    }

    /// EventBreakTimeController.Initialize(useInfoButton false, useAnimation
    /// true, display mode 0): when an event with a break time is in session
    /// and `InitializeModel` succeeds, the display mode is kept and the gauge
    /// view is set up; otherwise `HideGauge(useAnimation)` and the refresh
    /// stops. `InitializeModel` creates the model and runs its `Initialize`,
    /// which fails without the user's break-time record (then the
    /// controller logs its failure line), and otherwise looks up the event
    /// and its break-time master rows.
    fn initialize(&mut self, server: &EventBreakTimeServer, now_ms: impl FnOnce() -> i64, view: &mut UiPrefabView) {
        if is_set_event_break_time_master(server.event_in_session, now_ms) {
            match server.user_break_time {
                None => error!("[EventBreakTimeController] EventBreakTimeModel初期化失敗"),
                Some(record) => match record {},
            }
        }
        self.hide(true, view);
    }

    /// UIPartsEventBreakTimeGauge.Hide(useAnimation): its tweens are killed,
    /// then SetActive(false, useAnimation) sets the GameObject active and
    /// moves the gauge to the cached position raised by ANIM_OFFSET_Y, at
    /// once or with the slide.
    fn hide(&mut self, use_animation: bool, view: &mut UiPrefabView) {
        self.slide = None;
        view.set_visible(&self.gauge, true);
        let hidden_y = self.initial.y + GAUGE_HIDE_OFFSET_Y;
        if use_animation {
            self.slide = Some(GaugeSlide { from_y: self.position.y, to_y: hidden_y, elapsed: 0. });
        } else {
            self.position = Vec2::new(self.initial.x, hidden_y);
            view.set_anchored_position(&self.gauge, self.position);
        }
    }

    /// Advances the running slide: DOTween InQuad evaluates `(t/d) * (t/d)`.
    fn advance(&mut self, delta: f32, view: &mut UiPrefabView) {
        let Some(slide) = self.slide.as_mut() else { return; };
        slide.elapsed = (slide.elapsed + delta).min(GAUGE_SLIDE_SECONDS);
        let t = slide.elapsed / GAUGE_SLIDE_SECONDS;
        self.position.y = slide.from_y + (slide.to_y - slide.from_y) * (t * t);
        view.set_anchored_position(&self.gauge, self.position);
        if slide.elapsed >= GAUGE_SLIDE_SECONDS {
            self.slide = None;
        }
    }
}

/// A same-document reference: `None` for the null reference.
fn local_reference(value: &Value, field: &str) -> Option<i64> {
    assert_eq!(value[0].as_i64(), Some(0), "field menu reference must be local: {field}");
    let id = value[1].as_i64().unwrap_or_else(|| panic!("field menu reference missing: {field}"));
    (id != 0).then_some(id)
}

#[derive(Debug, Clone, Copy)]
enum ShellAction {
    SiteMap,
    FixtureEdit,
    Menu,
    ScreenShot,
    CameraReset,
    UiHide,
}

impl ShellAction {
    const ALL: [Self; 6] = [
        Self::SiteMap, Self::FixtureEdit, Self::Menu, Self::ScreenShot, Self::CameraReset, Self::UiHide,
    ];

    fn field(self) -> &'static str {
        match self {
            Self::SiteMap => "_siteMapButton",
            Self::FixtureEdit => "_fixtureEditButton",
            Self::Menu => "_menuButton",
            Self::ScreenShot => "_screenShotButton",
            Self::CameraReset => "_cameraResetButton",
            Self::UiHide => "_uiDisableButton",
        }
    }

    fn visible(self, host: ShellHost, hidden: bool) -> bool {
        match self {
            Self::SiteMap => host.shows_site_map() && !hidden,
            Self::FixtureEdit => matches!(host, ShellHost::Home | ShellHost::MyRoom) && !hidden,
            Self::ScreenShot | Self::UiHide => true,
            Self::Menu | Self::CameraReset => !hidden,
        }
    }
}

/// The existing glyph bake waits for this fixed set of UI characters.
#[derive(Resource)]
pub(crate) struct ShellTextCharset {
    pub(crate) chars: Vec<char>,
}

#[derive(Resource)]
pub(crate) struct ShellNamesHandle(Handle<JsonAsset>);

#[derive(Resource, Default)]
pub(crate) struct ShellUiState {
    hidden: bool,
}

#[derive(Resource, Default)]
pub(crate) struct ShellDialogState {
    pub(crate) leave_confirm: bool,
    pub(crate) menu_open: bool,
    /// The closing window still occupies the dialog input layer until its slide ends.
    pub(crate) menu_closing: bool,
    pub(crate) option_open: bool,
    pub(crate) get_resource_open: bool,
    /// The learn-phenomenon dialog, from its request to the end of its close animation.
    pub(crate) learn_phenomena_open: bool,
    leave_context: Option<LeaveContext>,
    /// The screen manager's instance of the leave confirm.
    leave_dialog: Option<crate::ui_layers::DialogId>,
}

impl ShellDialogState {
    pub(crate) fn blocks_field_input(&self) -> bool {
        self.leave_confirm || self.menu_open || self.menu_closing || self.option_open || self.get_resource_open
            || self.learn_phenomena_open
    }
}

struct LeaveContext {
    owner_name: String,
    on_confirm: SystemId,
}

/// A visitor flow must supply both the world owner's name and its actual join
/// operation. Merely selecting a local site is not a change of world.
#[derive(Debug, Message)]
pub(crate) enum ShellDialogRequest {
    LeaveConfirm {
        owner_name: String,
        on_confirm: SystemId,
    },
    MysekaiMenu,
}

#[derive(Resource)]
pub(crate) struct ShellSpawned;

#[derive(Component)]
pub(crate) struct MenuShellRoot {
    host: ShellHost,
    bindings: ShellBindings,
    motion: ChromeMotion,
    /// This host is the active site's host (its screen has been set up).
    active: bool,
}

impl MenuShellRoot {
    fn new(host: ShellHost, doc: &UiPrefab, view: &mut UiPrefabView) -> Self {
        let bindings = ShellBindings::from_document(doc, view);
        let motion = ChromeMotion::new(doc, &bindings);
        Self { host, bindings, motion, active: false }
    }

    /// One frame of this host's view state. The gauge's tween runs whether or
    /// not the chrome is drawn, and the host's screen setup runs when its site
    /// has just become the active one; while the chrome is drawn, the actions
    /// visible for this host and the chrome motion are applied.
    #[allow(clippy::too_many_arguments)]
    fn advance_view(
        &mut self,
        set_up: bool,
        drawn: bool,
        hidden: bool,
        delta: f32,
        now_ms: impl FnOnce() -> i64,
        doc: &UiPrefab,
        view: &mut UiPrefabView,
    ) {
        let host = self.host;
        if let Some(gauge) = self.bindings.gauge.as_mut() {
            gauge.advance(delta, view);
            if set_up {
                gauge.setup(host, &EVENT_BREAK_TIME_SERVER, now_ms, view);
            }
        }
        if !drawn { return; }
        for action in ShellAction::ALL {
            if let Some(path) = self.bindings.refs.get(action.field()) {
                view.set_visible(path, action.visible(host, hidden));
            }
        }
        self.motion.apply(hidden, delta, doc, &self.bindings, view);
    }
}

/// A field-menu host's view as `place` leaves it once the host's site is
/// active, its screen has been set up with no current event and the chrome
/// is shown and at rest: the research instrument measures this state.
#[cfg(test)]
pub(crate) fn settled_host_view(doc: &UiPrefab, key: &str) -> UiPrefabView {
    let host = ShellHost::ALL.into_iter().find(|host| host.key() == key)
        .unwrap_or_else(|| panic!("{key} is not a field-menu host"));
    let mut view = UiPrefabView::new(host.key(), BALLOON_LAYER);
    let mut root = MenuShellRoot::new(host, doc, &mut view);
    let no_clock = || unreachable!("no event is in session");
    root.advance_view(true, true, false, 0., no_clock, doc, &mut view);
    // Long enough for the gauge slide and the chrome motion to end.
    root.advance_view(false, true, false, GAUGE_SLIDE_SECONDS.max(CHROME_MOVE_DURATION), no_clock, doc, &mut view);
    view
}

#[derive(Component)]
pub(crate) struct ShellDialogRoot;

/// These are identities, not a second copy of layout data.
struct ShellBindings {
    refs: std::collections::HashMap<&'static str, String>,
    gauge: Option<BreakTimeGauge>,
}

impl ShellBindings {
    fn from_document(doc: &UiPrefab, view: &mut UiPrefabView) -> Self {
        let (root, component) = doc.nodes.iter().enumerate().find_map(|(i, node)| {
            node.components.iter()
                .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuUIContent")
                .map(|c| (i, c))
        }).expect("field prefab must contain MysekaiMenuUIContent");
        let mut refs = std::collections::HashMap::new();
        for field in [
            "_siteMapButton", "_leaveMysekaiButton", "_homeAreaInfo", "_siteNameIconImage",
            "_screenShotButton", "_screenShotButtonCanvasGroup", "_screenShotUx",
            "_cameraResetButton", "_menuButton", "_uiDisableButton",
            "_uiDisableButtonCanvasGroup", "_sekaiMissionHomePanel",
            "_uiDisableButtonOnPosition", "_uiDisableButtonOffPosition",
            "_screenShotButtonOnPosition", "_screenShotButtonOffPosition", "_backButton",
        ] {
            let value = &component.fields[field];
            assert_eq!(value[0].as_i64(), Some(0), "field menu reference must be local: {field}");
            let id = value[1].as_i64().unwrap_or_else(|| panic!("field menu reference missing: {field}"));
            let identity = format!("@{id}");
            doc.find(&identity).unwrap_or_else(|error| panic!("{error}"));
            refs.insert(field, identity);
        }
        // The housing entry is an authored sibling of the common chrome, not
        // part of MysekaiMenuUIContent. Reuse that actual host button; its
        // screen-605 editor is a separate presenter behind EditCommand.
        let edit_root = if matches!(doc.prefab.as_str(), "ScreenLayerMysekaiHome" | "ScreenLayerMysekaiMyRoom") {
            let index = doc.find("ComponentRoot/LeftTop/EditButtonRoot")
                .expect("housing host edit-button root");
            let node = &doc.nodes[index];
            let button = node.components.iter()
                .find(|component| component.class == "Sekai.UI.CustomButton")
                .expect("housing host edit button");
            refs.insert("_fixtureEditButton", format!("@{}", button.path_id));
            // The CN client's HomeView.Awake / MyRoomView.Awake find the direct
            // "Text" child and assign TMP TextAlignmentOptions.Center (514).
            // Both prefabs serialize Left (513); keep their authored rects.
            let label_index = doc.find(&format!("{}/Text", node.path))
                .expect("housing host edit-button Text child");
            let label = doc.nodes[label_index].components.iter()
                .find(|component| component.class == "Sekai.UI.CustomTextMesh")
                .expect("housing host edit-button CustomTextMesh");
            // That Awake is the CN client's. The JP views declare no Awake and
            // never set this alignment, so a JP layout keeps its serialized
            // one. Layouts extracted before region tagging are the CN ones.
            if doc.source.region.is_none() {
                view.set_text_alignment(&format!("@{}", label.path_id), 514);
            }
            Some(node.path.as_str())
        } else { None };
        // Keep the common chrome and the real housing entry only. Selection,
        // birthday and the editor screen still have their own presenters.
        let path = &doc.nodes[root].path;
        for node in &doc.nodes {
            let is_ancestor = path.starts_with(&format!("{}/", node.path));
            let is_descendant = node.path.starts_with(&format!("{path}/"));
            let is_edit = edit_root.is_some_and(|edit| node.path == edit
                || edit.starts_with(&format!("{}/", node.path))
                || node.path.starts_with(&format!("{edit}/")));
            if node.path != *path && !is_ancestor && !is_descendant && !is_edit {
                view.set_visible(&format!("@{}", node.game_object_id), false);
            }
        }
        let result = Self { refs, gauge: BreakTimeGauge::from_document(doc, component) };
        // The currently mounted local world is owned by the player. Visitor
        // headers and leave controls require a distinct world context.
        for field in ["_leaveMysekaiButton", "_homeAreaInfo", "_backButton"] {
            view.set_visible(result.get(field), false);
        }
        // Mission rows have no account producer in this host yet; serialized
        // preview progress must not become a runtime mission.
        view.set_visible(result.get("_sekaiMissionHomePanel"), false);
        result
    }

    fn get(&self, field: &'static str) -> &str {
        self.refs[field].as_str()
    }

    fn position(&self, doc: &UiPrefab, field: &'static str) -> Vec2 {
        let index = doc.find(self.get(field)).unwrap_or_else(|error| panic!("{error}"));
        Vec2::from_array(doc.nodes[index].rect.anchored_position)
    }
}

struct ChromeMotion {
    hidden: bool,
    elapsed: f32,
    from_hide: Vec2,
    from_shot: Vec2,
    hide: Vec2,
    shot: Vec2,
}

impl ChromeMotion {
    fn new(doc: &UiPrefab, bindings: &ShellBindings) -> Self {
        let hide = bindings.position(doc, "_uiDisableButtonOnPosition");
        let shot = bindings.position(doc, "_screenShotButtonOnPosition");
        Self { hidden: false, elapsed: CHROME_MOVE_DURATION, from_hide: hide, from_shot: shot, hide, shot }
    }

    fn apply(&mut self, hidden: bool, delta: f32, doc: &UiPrefab, bindings: &ShellBindings, view: &mut UiPrefabView) {
        if self.hidden != hidden {
            self.hidden = hidden;
            self.elapsed = 0.;
            self.from_hide = self.hide;
            self.from_shot = self.shot;
        } else {
            self.elapsed = (self.elapsed + delta).min(CHROME_MOVE_DURATION);
        }
        let t = self.elapsed / CHROME_MOVE_DURATION;
        let ease = 1. - (1. - t).powi(3);
        let target_hide = bindings.position(doc, if hidden { "_uiDisableButtonOffPosition" } else { "_uiDisableButtonOnPosition" });
        let target_shot = bindings.position(doc, if hidden { "_screenShotButtonOffPosition" } else { "_screenShotButtonOnPosition" });
        self.hide = self.from_hide.lerp(target_hide, ease);
        self.shot = self.from_shot.lerp(target_shot, ease);
        view.set_anchored_position(bindings.get("_uiDisableButton"), self.hide);
        view.set_anchored_position(bindings.get("_screenShotButton"), self.shot);
        let alpha = if hidden { 0.5 } else { 1.0 };
        view.set_alpha(bindings.get("_uiDisableButtonCanvasGroup"), alpha);
        view.set_alpha(bindings.get("_screenShotButtonCanvasGroup"), alpha);
    }
}

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ShellNamesHandle(server.load(SITES_DATA)));
    commands.init_resource::<ShellUiState>();
    commands.init_resource::<ShellDialogState>();
}

pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<ShellNamesHandle>>,
    layouts: Res<UiLayouts>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
    phenomena: Option<Res<crate::learn_phenomena_dialog::PhenomenonGlyphs>>,
) {
    let Some(handle) = handle else { return; };
    // A stage does not render prefab-based menus or their fixed button labels.
    // Its visible text comes from TalkCharset and TweetMaster, which still feed
    // the one shared atlas. Do not demand unused localized menu wordings here.
    if stage.is_some() {
        if layouts.document("Talk").is_none() { return; }
        commands.insert_resource(ShellTextCharset { chars: Vec::new() });
        commands.remove_resource::<ShellNamesHandle>();
        return;
    }
    if let LoadState::Failed(error) = server.load_state(&handle.0) {
        panic!("field menu site names failed: {error:?}");
    }
    let Some(asset) = jsons.get(&handle.0) else { return; };
    if ShellHost::ALL.iter().any(|host| layouts.document(host.key()).is_none())
        || layouts.document("Common2").is_none() {
        return;
    }
    // The phenomenon names the learn dialog and the notice banner print.
    let Some(phenomenon_names) = phenomena.as_deref().and_then(|p| p.texts.as_ref()) else { return; };
    let parsed: Value = serde_json::from_str(&asset.0).expect("site name document");
    let rows = parsed["sites"].as_array().expect("site name rows");
    let mut chars = layouts.text_chars();
    for row in rows {
        chars.extend(row["name"].as_str().expect("site name").chars());
    }
    for name in phenomenon_names {
        chars.extend(name.chars());
    }
    for wording in ["WORD_LEFT_ROOM", "WORD_CANCEL", "MSG_CONFIRM_LEAVE_MYSEKAI",
        "WORD_NOT_SAVE_RETURN", "WORD_SAVE_RETURN", "WORD_EDIT_SAVE_CONFIRMATION", "MSG_LEARN_PHENOMENA"]
        .into_iter().chain(crate::menu_dialog::RANK_GAUGE_WORDINGS) {
        chars.extend(layouts.wordings.get(wording).unwrap_or_else(|| panic!("UI wording missing: {wording}")).chars());
    }
    for texts in [
        crate::info::FIXED_TEXTS, crate::menu_dialog::FIXED_TEXTS,
        crate::get_resource::FIXED_TEXTS, crate::option_dialog::FIXED_TEXTS,
        crate::fixture_edit_ui::FIXED_TEXTS,
    ] {
        for text in texts { chars.extend(text.chars()); }
    }
    chars.sort_unstable();
    chars.dedup();
    commands.insert_resource(ShellTextCharset { chars });
    commands.remove_resource::<ShellNamesHandle>();
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    spawned: Option<Res<ShellSpawned>>,
) {
    if spawned.is_some() || !layouts.ready("Common2", &server)
        || ShellHost::ALL.iter().any(|host| !layouts.ready(host.key(), &server)) {
        return;
    }
    for host in ShellHost::ALL {
        let doc = layouts.document(host.key()).expect("ready field prefab");
        let mut view = UiPrefabView::new(host.key(), BALLOON_LAYER);
        let root = MenuShellRoot::new(host, doc, &mut view);
        commands.spawn((
            root, Visibility::Hidden, Transform::default(),
            RenderLayers::layer(BALLOON_LAYER), view,
        ));
    }
    let mut view = UiPrefabView::new("Common2", SITEMAP_LAYER);
    view.set_visible("WindowRoot/Tabs", false);
    view.set_text("@34358", layouts.wordings["WORD_CANCEL"].clone());
    view.set_text("@11788", layouts.wordings["WORD_LEFT_ROOM"].clone());
    commands.spawn((
        ShellDialogRoot, Visibility::Hidden, Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER), view,
    ));
    commands.insert_resource(ShellSpawned);
}

pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    layouts: Res<UiLayouts>,
    active: Option<Res<SiteActive>>,
    stack: Res<UiLayerStack>,
    ui: Res<ShellUiState>,
    dialog: Res<ShellDialogState>,
    entry: Option<Res<crate::entry::EntrySequence>>,
    mut roots: Query<(&mut MenuShellRoot, &mut Visibility, &mut Transform, &mut UiPrefabView), Without<ShellDialogRoot>>,
    mut dialogs: Query<(&mut Visibility, &mut Transform, &mut UiPrefabView), (With<ShellDialogRoot>, Without<MenuShellRoot>)>,
    mut solved: Local<std::collections::HashSet<&'static str>>,
    root_canvas: Option<Res<RootCanvas>>,
    (user, real): (Option<Res<crate::server::ClientUserData>>, Res<Time<Real>>),
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else { return; };
    let scale = root_canvas.scale(window);
    let host = active.as_deref().map(ShellHost::for_site);
    for (mut root, mut visibility, mut transform, mut view) in &mut roots {
        let is_host = host == Some(root.host);
        let set_up = is_host && !root.active;
        root.active = is_host;
        // The home HUD appears in the entry's OnFinishEnterAsync.
        let visible = crate::entry::hud_open(entry.as_deref()) && stack.on_field() && is_host;
        *visibility = if visible { Visibility::Inherited } else { Visibility::Hidden };
        transform.scale = Vec3::splat(scale);
        let doc = layouts.document(root.host.key()).expect("spawned field prefab");
        // TimeUtility.GetCurrentTimestamp: the client's server date plus the
        // real time since it.
        let now_ms = || {
            user.as_deref()
                .expect("TimeUtility.GetCurrentTimestamp before the server model's first response")
                .current_timestamp(real.elapsed_secs())
        };
        root.advance_view(set_up, visible, ui.hidden, time.delta_secs(), now_ms, doc, &mut view);
        if !visible { continue; }
        let canvas = root_canvas.size(window);
        if !solved.contains(view.key) {
            if let Some(check) = view.solve_check(&layouts, canvas) {
                solved.insert(view.key);
                if check.non_finite.is_empty() {
                    info!("UI {}: {} nodes solved at canvas {canvas}, every world corner finite", view.key, check.nodes);
                } else {
                    error!(
                        "UI {}: {} of {} nodes have a non-finite world corner at canvas {canvas}: {:?}",
                        view.key, check.non_finite.len(), check.nodes, check.non_finite
                    );
                }
                if !check.negative_size.is_empty() {
                    warn!(
                        "UI {}: {} nodes have a negative width or height at canvas {canvas}: {:?}",
                        view.key, check.negative_size.len(), check.negative_size
                    );
                }
            }
        }
    }
    for (mut visibility, mut transform, mut view) in &mut dialogs {
        let context = dialog.leave_context.as_ref().filter(|_| dialog.leave_confirm);
        *visibility = if context.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        transform.scale = Vec3::splat(scale);
        if let Some(context) = context {
            view.set_text("Content/MessageBody", layouts.wordings["MSG_CONFIRM_LEAVE_MYSEKAI"]
                .replace("{0}", &context.owner_name));
        }
    }
}

pub(crate) fn advance_dialogs(
    mut requests: MessageReader<ShellDialogRequest>,
    mut dialog: ResMut<ShellDialogState>,
    mut screens: ResMut<crate::ui_layers::ScreenManager>,
    mut back_keys: MessageReader<crate::ui_layers::DialogBackKeyEvent>,
) {
    for request in requests.read() {
        match request {
            ShellDialogRequest::LeaveConfirm { owner_name, on_confirm } => {
                if !dialog.leave_confirm {
                    // MysekaiLeaveConfirmDialog.Show: ShowDialog(type 465),
                    // Initialize, Open. The view draws no open animation, so
                    // the open finishes at once.
                    match screens.show_dialog(
                        crate::ui_layers::DialogType::MysekaiLeaveConfirmDialog,
                        crate::ui_layers::DisplayLayerType::LayerDialog,
                        crate::ui_layers::DialogBackKey::Close,
                        "MysekaiLeaveConfirmDialog.Show",
                    ) {
                        Ok(id) => {
                            screens.open_dialog(id);
                            screens.dialog_open_finished(id);
                            dialog.leave_dialog = Some(id);
                        }
                        Err(error) => {
                            error!("[field menu] {error}: the leave confirm does not open");
                            continue;
                        }
                    }
                    dialog.leave_context = Some(LeaveContext {
                        owner_name: owner_name.clone(), on_confirm: *on_confirm,
                    });
                    dialog.leave_confirm = true;
                }
            }
            ShellDialogRequest::MysekaiMenu => dialog.menu_open = true,
        }
    }
    // The back key the manager hands the leave confirm: its
    // OnHardwareBackKeyProcess makes one virtual call on the dialog, drawn
    // here as the close button's path (no leave).
    for key in back_keys.read() {
        if dialog.leave_dialog == Some(key.id) {
            info!("[field menu] back key on the leave confirm: closed without leaving");
            close_leave_confirm(&mut dialog, &mut screens);
        }
    }
}

/// The leave confirm's close: `DialogBase.Close`, then its destruction (the
/// view draws no close animation).
fn close_leave_confirm(dialog: &mut ShellDialogState, screens: &mut crate::ui_layers::ScreenManager) {
    dialog.leave_confirm = false;
    dialog.leave_context = None;
    if let Some(id) = dialog.leave_dialog.take() {
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
    }
}

/// A window position (logical pixels, origin top-left) in root canvas units
/// (origin at the canvas centre, y up).
fn to_canvas(position: Vec2, window: &Window, root: &RootCanvas) -> Vec2 {
    Vec2::new(position.x - window.width() / 2., window.height() / 2. - position.y) / root.scale(window)
}

/// A window position (logical pixels, origin top-left) as the engine's
/// screen point (physical pixels, origin bottom-left).
fn to_screen_point(position: Vec2, window: &Window) -> [f32; 2] {
    let ratio = window.scale_factor();
    [position.x * ratio, window.physical_height() as f32 - position.y * ratio]
}

/// The event camera's ray through an engine screen point, as the raycast
/// pointer; None when the host canvas carries no camera pose. A point the
/// camera cannot see gets the engine's fallback ray and its error is logged.
pub(crate) fn screen_pointer(window: &Window, root: &RootCanvas, screen: [f32; 2]) -> Option<Pointer> {
    let pixels = [window.physical_width() as f32, window.physical_height() as f32];
    let event = root.event_ray(pixels, screen)?;
    if let Some(error) = &event.error {
        warn!("{error}");
    }
    Some(Pointer::Ray { ray: event.ray, root: event.root })
}

/// The raycast pointer of a window position: the event camera's ray, or the
/// point in canvas units when the host canvas carries no camera pose.
pub(crate) fn event_pointer(window: &Window, root: &RootCanvas, position: Vec2) -> Pointer {
    screen_pointer(window, root, to_screen_point(position, window))
        .unwrap_or_else(|| Pointer::Canvas(to_canvas(position, window, root)))
}

/// The press chain of the field camera reset button for one pointer: the
/// pointer, the selectable the source raycast gives a press there to,
/// whether the button's node is the pointer's enter target or an ancestor
/// of it, and the button's `IsActive()` and `IsInteractable()`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraResetHit {
    // The pointer and the target are read by the research instrument only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) pointer: Pointer,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) target: Option<(usize, i64)>,
    pub(crate) over_button: bool,
    pub(crate) hovered: bool,
    pub(crate) active: bool,
    pub(crate) interactable: bool,
}

/// The camera reset press chain for a pointer at a window position.
pub(crate) fn camera_reset_hit(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    window: &Window,
    root: &RootCanvas,
    position: Vec2,
    node: usize,
    button: i64,
) -> CameraResetHit {
    camera_reset_press(view, layouts, root.size(window), event_pointer(window, root, position), node, button)
}

/// The camera reset press chain for a raycast pointer over a root canvas of
/// `canvas` units.
pub(crate) fn camera_reset_press(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
    pointer: Pointer,
    node: usize,
    button: i64,
) -> CameraResetHit {
    let target = view.press_target(layouts, pointer, canvas);
    let hovered = view.hovers(layouts, pointer, canvas, node);
    let (active, interactable) = match view.selectable_live(layouts, canvas, node, button) {
        Some(live) => live,
        None => {
            // A canvas without area resolves and raycasts nothing, so the
            // pointer is over no button and these two are never read.
            assert!(target.is_none() && !hovered, "a canvas without area produced a raycast target");
            (false, false)
        }
    };
    CameraResetHit { pointer, target, over_button: target.is_some_and(|(_, id)| id == button), hovered, active, interactable }
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, canvas: Vec2, size: Vec2) -> bool {
    view.rect(layouts, path, size).is_some_and(|rect| rect.active && rect.contains(canvas))
}

pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>,
    mut ui: ResMut<ShellUiState>,
    mut dialog: ResMut<ShellDialogState>,
    mut layer_commands: MessageWriter<LayerCommand>,
    requests: (MessageWriter<CaptureFrame>, MessageWriter<crate::fixture_edit::EditCommand>),
    // Paired in one parameter: this system is at the parameter-count limit.
    (mut stack, entry): (ResMut<UiLayerStack>, Option<Res<crate::entry::EntrySequence>>),
    active: Option<Res<SiteActive>>,
    roots: Query<(&MenuShellRoot, &UiPrefabView)>,
    dialogs: Query<&UiPrefabView, With<ShellDialogRoot>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    let (mut captures, mut edit_commands) = requests;
    let taps: Vec<Vec2> = gestures.read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position).collect();
    if taps.is_empty() || consumed.0 { return; }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else { return; };
    let size = root_canvas.size(window);
    if dialog.menu_open || dialog.menu_closing || dialog.option_open || dialog.get_resource_open
        || dialog.learn_phenomena_open { return; }
    if dialog.leave_confirm {
        // Read ownership before closing. A close must not replay against the
        // field controls later in this gesture dispatch.
        consumed.0 = true;
        let Some(context) = dialog.leave_context.as_ref() else { return; };
        let on_confirm = context.on_confirm;
        let Ok(view) = dialogs.single() else { return; };
        for position in taps {
            let canvas = to_canvas(position, window, root_canvas);
            let accept = hit(view, &layouts, "@137046", canvas, size);
            let cancel = hit(view, &layouts, "@113613", canvas, size)
                || hit(view, &layouts, "WindowRoot/UIPartsCloseButton", canvas, size)
                || !hit(view, &layouts, "WindowRoot", canvas, size);
            if accept || cancel {
                close_leave_confirm(&mut dialog, &mut stack);
                if accept { commands.run_system(on_confirm); }
                break;
            }
        }
        return;
    }
    if !stack.on_field() || !crate::entry::hud_open(entry.as_deref()) { return; }
    let Some(active) = active else { return; };
    let host = ShellHost::for_site(&active);
    let Some((root, view)) = roots.iter().find(|(root, _)| root.host == host) else { return; };
    for position in taps {
        let canvas = to_canvas(position, window, root_canvas);
        if let Some(action) = ShellAction::ALL.into_iter().find(|action| {
            !matches!(action, ShellAction::CameraReset)
                && action.visible(host, ui.hidden)
                && hit(view, &layouts, root.bindings.get(action.field()), canvas, size)
        }) {
            consumed.0 = true;
            sounds.source_button(&layouts, view.key, root.bindings.get(action.field()));
            match action {
                ShellAction::SiteMap => { layer_commands.write(LayerCommand::Push(LayerId::MysekaiSiteMap)); }
                ShellAction::FixtureEdit => { edit_commands.write(crate::fixture_edit::EditCommand::Enter); }
                ShellAction::Menu => { dialog.menu_open = true; }
                ShellAction::ScreenShot => { captures.write(CaptureFrame); }
                ShellAction::UiHide => { ui.hidden = !ui.hidden; }
                ShellAction::CameraReset => {
                    unreachable!("the camera reset button is driven by its source pointer handlers")
                }
            }
            break;
        }
    }
}

/// The CustomButton behind `MysekaiMenuUIContent._cameraResetButton`: the
/// field holds the MySekai button wrapper, whose `_button` is the CustomButton.
pub(crate) fn camera_reset_button(doc: &UiPrefab) -> Option<i64> {
    let content = doc.nodes.iter()
        .flat_map(|node| node.components.iter())
        .find(|c| c.class == "Sekai.Mysekai.MysekaiMenuUIContent")?;
    let reference = content.fields["_cameraResetButton"].as_array()?;
    if reference.first()?.as_i64() != Some(0) {
        return None;
    }
    let id = reference.get(1)?.as_i64()?;
    let component = doc.nodes.iter().flat_map(|node| node.components.iter()).find(|c| c.path_id == id)?;
    if component.fields.get("se").is_some() {
        return Some(id);
    }
    let inner = component.fields["_button"].as_array()?;
    (inner.first()?.as_i64() == Some(0)).then(|| inner.get(1)?.as_i64()).flatten()
}

/// Whether this layout's CustomButton class serializes `enableHoldRepeat`,
/// by the layout's region tag. Layouts without a tag are the shared root's
/// CN extractions, whose CustomButton has no hold repeat at all; the JP
/// class serializes it.
fn declares_hold_repeat(doc: &UiPrefab) -> bool {
    match doc.source.region.as_deref() {
        None => false,
        Some("jp") => true,
        Some(region) => panic!("CustomButton: no declared field set for region {region}"),
    }
}

/// The CustomButton's serialized click fields. A field the region's class
/// declares must be in the layout, and one it does not declare must not be.
fn button_config(doc: &UiPrefab, fields: &Value) -> Option<button_rule::CustomButtonConfig> {
    let enable_hold_repeat = if declares_hold_repeat(doc) {
        fields["enableHoldRepeat"].as_bool()?
    } else {
        assert!(
            fields.get("enableHoldRepeat").is_none(),
            "{}: CustomButton serializes enableHoldRepeat, which its region's class does not declare",
            doc.prefab
        );
        false
    };
    Some(button_rule::CustomButtonConfig {
        se: button_rule::SeType::from_serialized(fields["se"].as_i64()?)?,
        other_se_name: fields["otherSeName"].as_str()?.to_owned(),
        interval: button_rule::IntervalUseType::from_serialized(fields["interval"].as_i64()?)?,
        absolutely_press: fields["absolutelyPress"].as_bool()?,
        enable_long_press: fields["enableLongPress"].as_bool()?,
        enable_hold_repeat,
    })
}

/// The game's input manager state shared by source buttons.
#[derive(Resource, Default)]
pub(crate) struct SourceInputManager(pub(crate) button_rule::InputManager);

/// The camera reset button's CustomButton state, whether the event
/// system's current press went to it, the held pointer (its id and latest
/// position, from press to release), whether the button is in that
/// pointer's hover chain, and the press/release interactions requested this
/// frame for the tween update that follows the event system.
#[derive(Resource, Default)]
pub(crate) struct CameraResetPress {
    state: button_rule::CustomButtonState,
    pressed: bool,
    held: Option<(i32, Vec2)>,
    entered: bool,
    interactions: Vec<(ShellHost, ViewInteraction)>,
    /// The host view and the button (its CustomButton's id) that took the
    /// current press.
    press_host: Option<(ShellHost, u64)>,
}

/// The host's handling of one button's effects that are not the press
/// effect, release effect, sound or click: long-press and hold-repeat are
/// not driven by this button's options (both off), and only this one source
/// button is driven, so no other button can own the stored finish callback.
fn unused_button_effect(effect: &button_rule::ButtonEffect) {
    match effect {
        button_rule::ButtonEffect::StartLongPressCheck
        | button_rule::ButtonEffect::CancelLongPressCheck
        | button_rule::ButtonEffect::StartHoldRepeat
        | button_rule::ButtonEffect::StopHoldRepeat => {}
        button_rule::ButtonEffect::FinishedControlOf(owner) => error!(
            "camera reset: the input manager's finish callback belongs to source button {owner}, \
             which this host does not drive; its release is not played"
        ),
        other => unreachable!("handled by the caller: {other:?}"),
    }
}

/// `CustomButton.PlayPressEffect` / `PlayReleaseEffect` on a button whose
/// override delegates are unset: the view interaction's `OnPressed` /
/// `OnReleased`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewInteraction {
    Pressed,
    Released,
}

/// Source press/release/click for the field camera reset button: the press
/// goes to the selectable under the pointer by the source raycast; release
/// finishes it; the click needs the release over the same button and then
/// passes the CustomButton gate (shared 0.2 s interval on the real-time
/// clock, sound), and its click event runs
/// `MysekaiMenuUIContent.ResetCameraStatus`, the field camera's
/// `ResetCameraSetting`. The press and release interactions are handed to
/// [`camera_reset_tap_effect`], which runs after this system as the tween
/// manager runs after the event system. A press the handlers cannot resolve
/// is logged.
///
/// A touch the platform cancels is a release to the event system's touch
/// path, but its raycast is cleared first (`GetTouchPointerEventData` sets an
/// empty `pointerCurrentRaycast` for `TouchPhase.Canceled`), so the release
/// finds no click handler and never clicks: the up and the exit run, the
/// click does not. The gesture layer reports a cancelled touch as `Cancel`,
/// which this system handles exactly so.
///
/// Enter and exit follow the event system's touch path: the press enters
/// the pressed object's hover chain; every frame a held pointer's current
/// position is raycast again, and leaving the button's chain sends it
/// `OnPointerExit` (coming back sends `OnPointerEnter`); a release sends
/// the up and the click first and then the exit. CustomButton overrides
/// only the exit: a held press that leaves finishes control, so the
/// release no longer clicks. The enter it inherits changes only the
/// selectable's transition state, which is not drawn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn camera_reset_input(
    mut commands: Commands,
    mut pointers: MessageReader<UiPointerEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    real: Res<Time<Real>>,
    stack: Res<UiLayerStack>,
    dialog: Res<ShellDialogState>,
    active: Option<Res<SiteActive>>,
    roots: Query<(&MenuShellRoot, &UiPrefabView)>,
    mut manager: ResMut<SourceInputManager>,
    mut press: ResMut<CameraResetPress>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>,
    mut camera: crate::camera::CameraReset,
    root_canvas: Option<Res<RootCanvas>>,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    let events: Vec<UiPointerEvent> = pointers.read().copied().collect();
    if events.is_empty() && press.held.is_none() { return; }
    let Ok(window) = windows.single() else {
        info!("camera reset input: {} pointer events without a single primary window", events.len());
        return;
    };
    let Some(root_canvas) = root_canvas.as_deref() else {
        info!("camera reset input: pointer events before the host canvas loaded");
        return;
    };
    let Some(active) = active else {
        info!("camera reset input: pointer events before a site is active");
        return;
    };
    let host = ShellHost::for_site(&active);
    let Some((_, view)) = roots.iter().find(|(root, _)| root.host == host) else {
        info!("camera reset input: pointer events before the {host:?} field menu is spawned");
        return;
    };
    let Some(doc) = layouts.document(view.key) else {
        info!("camera reset input: the {} layout is not loaded", view.key);
        return;
    };
    let Some(button) = camera_reset_button(doc) else {
        info!("camera reset input: {} has no camera reset CustomButton", view.key);
        return;
    };
    let node = doc.find(&format!("@{button}")).unwrap_or_else(|error| panic!("{error}"));
    let component = doc.nodes[node].components.iter().find(|c| c.path_id == button)
        .expect("camera reset CustomButton component");
    let config = button_config(doc, &component.fields).expect("camera reset CustomButton fields");
    let blocked = dialog.blocks_field_input() || !stack.on_field();
    let key = button as u64;
    // `Time.realtimeSinceStartup`, read live at each call.
    let startup = real.startup();
    let mut clock = move || startup.elapsed().as_secs_f32();
    let mut interactions = Vec::new();
    let mut run = |effects: Vec<button_rule::ButtonEffect>, commands: &mut Commands| {
        for effect in effects {
            match effect {
                button_rule::ButtonEffect::PlaySe { se, other_se_name } => sounds.button(se, other_se_name),
                button_rule::ButtonEffect::Click => {
                    consumed.0 = true;
                    camera.reset_camera_setting(commands);
                }
                button_rule::ButtonEffect::PressEffect => interactions.push((host, ViewInteraction::Pressed)),
                button_rule::ButtonEffect::ReleaseEffect => interactions.push((host, ViewInteraction::Released)),
                other => unused_button_effect(&other),
            }
        }
    };
    for event in events {
        let pointer = button_rule::Pointer { pointer_id: event.pointer_id, left_button: true };
        if event.phase == UiPointerPhase::Move {
            if let Some(held) = press.held.as_mut().filter(|(id, _)| *id == event.pointer_id) {
                held.1 = event.position;
            }
            continue;
        }
        let hit = camera_reset_hit(view, &layouts, window, root_canvas, event.position, node, button);
        let over_button = !blocked && hit.over_button;
        let live = button_rule::ButtonLive { active: hit.active, interactable: hit.interactable };
        let mut effects = Vec::new();
        let CameraResetPress { state, pressed, held, entered, press_host, .. } = &mut *press;
        // The event system delivers a handler only to a component that is
        // active and enabled at that moment.
        match event.phase {
            UiPointerPhase::Down => {
                *held = Some((event.pointer_id, event.position));
                *entered = !blocked && hit.hovered;
                *pressed = over_button;
                if *pressed {
                    *press_host = Some((host, key));
                    effects = button_rule::on_pointer_down(&mut manager.0, state, &config, key, pointer, event.touch_count, live);
                }
            }
            UiPointerPhase::Up | UiPointerPhase::Cancel => {
                if *pressed && live.active {
                    effects = button_rule::on_pointer_up(&mut manager.0, state, &config, key, pointer);
                    if event.phase == UiPointerPhase::Up && over_button {
                        effects.extend(button_rule::on_pointer_click(
                            &mut manager.0, state, &config, key, pointer, &mut clock, live,
                        ));
                    }
                }
                if *entered && live.active {
                    effects.extend(button_rule::on_pointer_exit(&mut manager.0, state, &config, key, pointer));
                }
                *pressed = false;
                *held = None;
                *entered = false;
            }
            UiPointerPhase::Move => unreachable!("moves only update the held position"),
        }
        run(effects, &mut commands);
    }
    // The input module's per-frame move of a held pointer.
    let CameraResetPress { state, held, entered, .. } = &mut *press;
    if let Some((pointer_id, position)) = *held {
        let hit = camera_reset_hit(view, &layouts, window, root_canvas, position, node, button);
        let hovered = !blocked && hit.hovered;
        if *entered && !hovered && hit.active {
            let pointer = button_rule::Pointer { pointer_id, left_button: true };
            run(button_rule::on_pointer_exit(&mut manager.0, state, &config, key, pointer), &mut commands);
        }
        *entered = hovered;
    }
    press.interactions.extend(interactions);
}

/// `CustomButton.OnDisable` of the camera reset button: a button that holds
/// a press and is no longer active and enabled (its host view is gone, or
/// the button or an ancestor was deactivated) finishes control, so its
/// release interaction plays and its press ends. Runs every frame, input
/// enabled or not, before the tap effect's update (the fade-out starts on
/// this frame's step).
pub(crate) fn camera_reset_disable(
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    roots: Query<(&MenuShellRoot, &UiPrefabView)>,
    root_canvas: Option<Res<RootCanvas>>,
    mut manager: ResMut<SourceInputManager>,
    mut press: ResMut<CameraResetPress>,
) {
    if press.state.control != button_rule::ControlState::Press {
        return;
    }
    let Some((host, key)) = press.press_host else { return; };
    let enabled = match roots.iter().find(|(root, _)| root.host == host) {
        None => false,
        Some((_, view)) => {
            let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else { return; };
            let Some(doc) = layouts.document(view.key) else { return; };
            let Some(button) = camera_reset_button(doc) else { return; };
            let node = doc.find(&format!("@{button}")).unwrap_or_else(|error| panic!("{error}"));
            // A canvas without area resolves nothing: the button's state is
            // unknown there, not disabled.
            match view.selectable_live(&layouts, root_canvas.size(window), node, button) {
                Some((active, _)) => active,
                None => return,
            }
        }
    };
    if enabled {
        return;
    }
    let CameraResetPress { state, interactions, .. } = &mut *press;
    for effect in button_rule::on_disable(&mut manager.0, state, key) {
        match effect {
            button_rule::ButtonEffect::ReleaseEffect => interactions.push((host, ViewInteraction::Released)),
            other => unused_button_effect(&other),
        }
    }
}

/// One `GraphicButtonTapEffect` a button's view interaction drives: its
/// component id, the Graphic components it fades (`_effectGraphic`, and
/// `_effectGraphicIcon` when set) and its serialized colour fields.
struct GraphicTapEffect {
    id: i64,
    graphics: Vec<i64>,
    config: tap_rule::TapEffectConfig,
}

/// The camera reset button's view interaction: the CustomButton's
/// `buttonViewInteraction`, as the `GraphicButtonTapEffect`s it drives, in
/// call order. Empty when the button has no view interaction (the press and
/// release then play nothing). A `MultiButtonTapEffect` forwards each call to
/// the entries of its `_tapEffectList` in order (nothing when the list is
/// empty), so its entries are expanded in place. A class that is not ported
/// (`CoverButtonTapEffect`, `HarvestButtonInteraction`; none is the camera
/// reset button's on any root) is reported and drives nothing; a missing
/// field is refused.
fn tap_effect(doc: &UiPrefab, button: i64) -> Vec<GraphicTapEffect> {
    let components = || doc.nodes.iter().flat_map(|node| node.components.iter());
    let component = |id: i64| components().find(|c| c.path_id == id)
        .unwrap_or_else(|| panic!("{}: component {id} is not in the layout", doc.prefab));
    let reference = |fields: &Value, name: &str| -> i64 {
        let pointer = fields[name].as_array().filter(|p| p.len() == 2)
            .unwrap_or_else(|| panic!("{}: {name} is not a reference", doc.prefab));
        assert_eq!(pointer[0].as_i64(), Some(0), "{}: {name} points outside the layout file", doc.prefab);
        pointer[1].as_i64().unwrap_or_else(|| panic!("{}: {name} has no path id", doc.prefab))
    };
    let interaction = reference(&component(button).fields, "buttonViewInteraction");
    let mut effects = Vec::new();
    let mut pending = vec![interaction];
    while let Some(id) = pending.pop() {
        if id == 0 {
            continue;
        }
        let effect = component(id);
        match effect.class.as_str() {
            "Sekai.UI.GraphicButtonTapEffect" => effects.push(graphic_tap_effect(doc, effect, &reference)),
            "Sekai.UI.MultiButtonTapEffect" => {
                let list = effect.fields["_tapEffectList"].as_array()
                    .unwrap_or_else(|| panic!("{}: MultiButtonTapEffect {id} has no _tapEffectList", doc.prefab));
                // Multi calls each entry in list order; a null entry throws
                // on the press.
                for entry in list.iter().rev() {
                    let pointer = entry.as_array().filter(|p| p.len() == 2 && p[0].as_i64() == Some(0))
                        .unwrap_or_else(|| panic!("{}: MultiButtonTapEffect {id} entry is not a local reference", doc.prefab));
                    let entry_id = pointer[1].as_i64().filter(|id| *id != 0)
                        .unwrap_or_else(|| panic!("{}: MultiButtonTapEffect {id} has a null entry", doc.prefab));
                    pending.push(entry_id);
                }
            }
            other => error_once!(
                "{}: button {button}'s view interaction {id} is a {other}, which is not ported; its press and release play nothing",
                doc.prefab
            ),
        }
    }
    effects
}

/// A `GraphicButtonTapEffect`'s graphics and serialized colour fields.
fn graphic_tap_effect(
    doc: &UiPrefab,
    effect: &moly_assets::ui_layout::UiComponent,
    reference: &dyn Fn(&Value, &str) -> i64,
) -> GraphicTapEffect {
    let f = &effect.fields;
    let number = |name: &str| f[name].as_f64()
        .unwrap_or_else(|| panic!("{}: GraphicButtonTapEffect {name} is missing", doc.prefab));
    let flag = |name: &str| f[name].as_bool()
        .unwrap_or_else(|| panic!("{}: GraphicButtonTapEffect {name} is missing", doc.prefab));
    let entry = |name: &str| usize::try_from(f[name].as_i64()
        .unwrap_or_else(|| panic!("{}: GraphicButtonTapEffect {name} is missing", doc.prefab)))
        .unwrap_or_else(|_| panic!("{}: GraphicButtonTapEffect {name} is negative", doc.prefab));
    let graphics: Vec<i64> = ["_effectGraphic", "_effectGraphicIcon"].iter()
        .map(|name| reference(f, name))
        .filter(|id| *id != 0)
        .collect();
    let config = tap_rule::TapEffectConfig {
        default_palette: entry("_defaultColorPalette"),
        effect_palette: entry("_effectColorPalette"),
        use_custom_alpha: flag("_useCustomAlpha"),
        custom_default_alpha: number("_customDefaultAlpha") as f32,
        custom_effect_alpha: number("_customEffectAlpha") as f32,
        refresh_color_when_awake: flag("_refreshColorWhenAwake"),
    };
    GraphicTapEffect { id: effect.path_id, graphics, config }
}

/// Per tap effect of a host view: whether Awake ran, the running fade and the
/// effect graphics' current colours.
#[derive(Default)]
struct TapEffectView {
    awake: bool,
    state: tap_rule::TapEffectState,
    colors: std::collections::HashMap<i64, [f32; 4]>,
}

/// The camera reset buttons' tap effects, by host view and effect component.
#[derive(Resource, Default)]
pub(crate) struct CameraResetTapEffect {
    views: std::collections::HashMap<(&'static str, i64), TapEffectView>,
    unported_root_reported: bool,
}

/// `GraphicButtonTapEffect` of the field camera reset button: Awake assigns
/// the default palette colour to the effect graphics once the host view
/// exists; each press/release from [`camera_reset_input`] starts its fade;
/// then the running fade takes this frame's step, as the tween manager's
/// update does after the event system in the same frame (the event system's
/// script execution order is -1000, the tween manager's 0, and a new tween
/// is updated in the frame it was created, on the scaled frame delta).
pub(crate) fn camera_reset_tap_effect(
    time: Res<Time>,
    layouts: Res<UiLayouts>,
    mut press: ResMut<CameraResetPress>,
    mut effects: ResMut<CameraResetTapEffect>,
    mut roots: Query<(&MenuShellRoot, &mut UiPrefabView)>,
) {
    let interactions = std::mem::take(&mut press.interactions);
    if layouts.tween_defaults().is_none() {
        if !interactions.is_empty() && !effects.unported_root_reported {
            effects.unported_root_reported = true;
            warn!("camera reset: the UI root carries no palette or tween defaults; the press effect is not drawn");
        }
        return;
    }
    for (root, mut view) in &mut roots {
        let Some(doc) = layouts.document(view.key) else { continue; };
        let Some(button) = camera_reset_button(doc) else { continue; };
        for GraphicTapEffect { id, graphics, config } in tap_effect(doc, button) {
        let colors = config.colors(|entry| layouts.palette_color(entry))
            .unwrap_or_else(|error| panic!("{}: camera reset tap effect: {error}", doc.prefab));
        let entry = effects.views.entry((view.key, id)).or_default();
        if !entry.awake {
            entry.awake = true;
            for &graphic in &graphics {
                let node = doc.find(&format!("@{graphic}")).unwrap_or_else(|error| panic!("{error}"));
                let image = doc.nodes[node].components.iter().find(|c| c.path_id == graphic)
                    .unwrap_or_else(|| panic!("{}: effect graphic {graphic} missing", doc.prefab));
                let channels = image.fields["m_Color"].as_array().filter(|v| v.len() == 4)
                    .unwrap_or_else(|| panic!("{}: effect graphic {graphic} has no m_Color", doc.prefab));
                let mut color = [0.0; 4];
                for (channel, value) in color.iter_mut().zip(channels) {
                    *channel = value.as_f64().expect("m_Color channel") as f32;
                }
                entry.colors.insert(graphic, color);
            }
            if let Some(color) = entry.state.awake(&config, &colors) {
                for &graphic in &graphics {
                    entry.colors.insert(graphic, color);
                    view.set_graphic_color(graphic, color);
                }
            }
        }
        for (host, interaction) in &interactions {
            if *host != root.host {
                continue;
            }
            match interaction {
                ViewInteraction::Pressed => entry.state.on_pressed(&colors),
                ViewInteraction::Released => entry.state.on_released(&colors),
            }
        }
        // The effect graphic's tween and the icon graphic's tween are the
        // same fade on two graphics; the first graphic's colour starts it.
        let Some(&first) = graphics.first() else { continue; };
        let current = entry.colors[&first];
        if let Some(color) = entry.state.update(current, time.delta_secs()) {
            for &graphic in &graphics {
                entry.colors.insert(graphic, color);
                view.set_graphic_color(graphic, color);
            }
        }
        }
    }
}
