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

use crate::balloon::{canvas_scale, BALLOON_LAYER};
use crate::camera::{CameraSetting, FieldCameraModel};
use crate::frame_capture::CaptureFrame;
use crate::gesture::{GestureEvent, GestureState};
use crate::site::SiteActive;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use crate::ui_layout::{UiLayouts, UiPrefabView};

const SITES_DATA: &str = "moly://site/sites.json";
const CAMERA_RESET_DURATION: f32 = 0.25;
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
    leave_context: Option<LeaveContext>,
}

impl ShellDialogState {
    pub(crate) fn blocks_field_input(&self) -> bool {
        self.leave_confirm || self.menu_open || self.menu_closing || self.option_open || self.get_resource_open
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
pub(crate) struct CameraResetTween {
    elapsed: f32,
    from_pitch: f32,
    from_distance: f32,
    duration: f32,
}

#[derive(Resource)]
pub(crate) struct ShellSpawned;

#[derive(Component)]
pub(crate) struct MenuShellRoot {
    host: ShellHost,
    bindings: ShellBindings,
    motion: ChromeMotion,
}

#[derive(Component)]
pub(crate) struct ShellDialogRoot;

/// These are identities, not a second copy of layout data.
struct ShellBindings {
    refs: std::collections::HashMap<&'static str, String>,
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
            // CN 6.0.0 HomeView.Awake / MyRoomView.Awake find the direct
            // "Text" child and assign TMP TextAlignmentOptions.Center (514).
            // Both prefabs serialize Left (513); keep their authored rects.
            let label_index = doc.find(&format!("{}/Text", node.path))
                .expect("housing host edit-button Text child");
            let label = doc.nodes[label_index].components.iter()
                .find(|component| component.class == "Sekai.UI.CustomTextMesh")
                .expect("housing host edit-button CustomTextMesh");
            view.set_text_alignment(&format!("@{}", label.path_id), 514);
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
        let result = Self { refs };
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
) {
    let Some(handle) = handle else { return; };
    if let LoadState::Failed(error) = server.load_state(&handle.0) {
        panic!("field menu site names failed: {error:?}");
    }
    let Some(asset) = jsons.get(&handle.0) else { return; };
    if ShellHost::ALL.iter().any(|host| layouts.document(host.key()).is_none())
        || layouts.document("Common2").is_none() {
        return;
    }
    let parsed: Value = serde_json::from_str(&asset.0).expect("site name document");
    let rows = parsed["sites"].as_array().expect("site name rows");
    let mut chars = layouts.text_chars();
    for row in rows {
        chars.extend(row["name"].as_str().expect("site name").chars());
    }
    for wording in ["WORD_LEFT_ROOM", "WORD_CANCEL", "MSG_CONFIRM_LEAVE_MYSEKAI",
        "WORD_NOT_SAVE_RETURN", "WORD_SAVE_RETURN", "WORD_EDIT_SAVE_CONFIRMATION"] {
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
        let bindings = ShellBindings::from_document(doc, &mut view);
        let motion = ChromeMotion::new(doc, &bindings);
        commands.spawn((
            MenuShellRoot { host, bindings, motion },
            Visibility::Hidden, Transform::default(),
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
    mut roots: Query<(&mut MenuShellRoot, &mut Visibility, &mut Transform, &mut UiPrefabView), Without<ShellDialogRoot>>,
    mut dialogs: Query<(&mut Visibility, &mut Transform, &mut UiPrefabView), (With<ShellDialogRoot>, Without<MenuShellRoot>)>,
) {
    let Ok(window) = windows.single() else { return; };
    let scale = canvas_scale(window.width(), window.height());
    let host = active.as_deref().map(ShellHost::for_site);
    for (mut root, mut visibility, mut transform, mut view) in &mut roots {
        let visible = stack.on_field() && host == Some(root.host);
        *visibility = if visible { Visibility::Inherited } else { Visibility::Hidden };
        transform.scale = Vec3::splat(scale);
        if !visible { continue; }
        for action in ShellAction::ALL {
            if let Some(path) = root.bindings.refs.get(action.field()) {
                view.set_visible(path, action.visible(root.host, ui.hidden));
            }
        }
        let doc = layouts.document(root.host.key()).expect("spawned field prefab");
        let MenuShellRoot { bindings, motion, .. } = &mut *root;
        motion.apply(ui.hidden, time.delta_secs(), doc, bindings, &mut view);
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
) {
    for request in requests.read() {
        match request {
            ShellDialogRequest::LeaveConfirm { owner_name, on_confirm } => {
                if !dialog.leave_confirm {
                    dialog.leave_context = Some(LeaveContext {
                        owner_name: owner_name.clone(), on_confirm: *on_confirm,
                    });
                    dialog.leave_confirm = true;
                }
            }
            ShellDialogRequest::MysekaiMenu => dialog.menu_open = true,
        }
    }
}

pub(crate) fn advance_camera_reset(
    mut commands: Commands,
    time: Res<Time>,
    mut reset: Option<ResMut<CameraResetTween>>,
    camera_model: Option<ResMut<FieldCameraModel>>,
    camera_setting: Option<Res<CameraSetting>>,
) {
    let Some(reset) = reset.as_deref_mut() else { return; };
    let (Some(mut model), Some(setting)) = (camera_model, camera_setting) else {
        commands.remove_resource::<CameraResetTween>();
        return;
    };
    reset.elapsed += time.delta_secs();
    let t = (reset.elapsed / reset.duration).clamp(0., 1.);
    model.pitch = reset.from_pitch + (setting.init_pitch - reset.from_pitch) * t;
    model.distance = reset.from_distance + (setting.distance - reset.from_distance) * t;
    model.gestured_distance = model.distance;
    if t >= 1. { commands.remove_resource::<CameraResetTween>(); }
}

fn to_canvas(position: Vec2, window: &Window) -> Vec2 {
    Vec2::new(position.x - window.width() / 2., window.height() / 2. - position.y)
        / canvas_scale(window.width(), window.height())
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
    stack: Res<UiLayerStack>,
    active: Option<Res<SiteActive>>,
    roots: Query<(&MenuShellRoot, &UiPrefabView)>,
    dialogs: Query<&UiPrefabView, With<ShellDialogRoot>>,
    camera_model: Option<Res<FieldCameraModel>>,
    camera_setting: Option<Res<CameraSetting>>,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    let (mut captures, mut edit_commands) = requests;
    let taps: Vec<Vec2> = gestures.read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position).collect();
    if taps.is_empty() || consumed.0 { return; }
    let Ok(window) = windows.single() else { return; };
    let scale = canvas_scale(window.width(), window.height());
    let size = Vec2::new(window.width(), window.height()) / scale;
    if dialog.menu_open || dialog.menu_closing || dialog.option_open || dialog.get_resource_open { return; }
    if dialog.leave_confirm {
        // Read ownership before closing. A close must not replay against the
        // field controls later in this gesture dispatch.
        consumed.0 = true;
        let Some(context) = dialog.leave_context.as_ref() else { return; };
        let on_confirm = context.on_confirm;
        let Ok(view) = dialogs.single() else { return; };
        for position in taps {
            let canvas = to_canvas(position, window);
            let accept = hit(view, &layouts, "@137046", canvas, size);
            let cancel = hit(view, &layouts, "@113613", canvas, size)
                || hit(view, &layouts, "WindowRoot/UIPartsCloseButton", canvas, size)
                || !hit(view, &layouts, "WindowRoot", canvas, size);
            if accept || cancel {
                dialog.leave_confirm = false;
                dialog.leave_context = None;
                if accept { commands.run_system(on_confirm); }
                break;
            }
        }
        return;
    }
    if !stack.on_field() { return; }
    let Some(active) = active else { return; };
    let host = ShellHost::for_site(&active);
    let Some((root, view)) = roots.iter().find(|(root, _)| root.host == host) else { return; };
    for position in taps {
        let canvas = to_canvas(position, window);
        if let Some(action) = ShellAction::ALL.into_iter().find(|action| {
            action.visible(host, ui.hidden)
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
                    if let (Some(model), Some(setting)) = (camera_model.as_deref(), camera_setting.as_deref()) {
                        let mut next = model.clone();
                        next.min_pitch = setting.min_pitch;
                        next.max_pitch = setting.max_pitch;
                        commands.insert_resource(next);
                        commands.insert_resource(CameraResetTween {
                            elapsed: 0., from_pitch: model.pitch, from_distance: model.distance,
                            duration: CAMERA_RESET_DURATION,
                        });
                    }
                }
            }
            break;
        }
    }
}
