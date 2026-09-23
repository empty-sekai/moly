//! Product-owned settings UI and graphics application, shared by native/wasm.
//! The source Option UI and this panel share LocalVolumeSettings and VolumeBus.

use bevy::{
    anti_alias::fxaa::Fxaa,
    camera::{visibility::RenderLayers, ImageRenderTarget, RenderTarget},
    image::ImageSampler,
    input::touch::Touches,
    prelude::*,
    render::render_resource::TextureFormat,
    ui::{FocusPolicy, UiTargetCamera},
    window::PrimaryWindow,
};
use serde_json::{json, Value};

use crate::{
    audio::{self, LocalVolumeSettings, VolumeBus, VolumeSettingData},
    settings_store::SettingsStore,
};

const COMPOSITE_LAYER: usize = 30;
const UI_ORDER: isize = 100;
const FONT: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GraphicsSettings {
    /// Frame limit. The source offers `MysekaiFpsQualityType` High (60) and
    /// Normal (30); `MysekaiOptionSettingData` leaves the field zero, i.e. High.
    pub(crate) frame_rate: u16,
    /// Scene camera render scale chosen in the product panel; `None` applies
    /// the source DPI law ([`source_render_scale`]).
    pub(crate) render_scale: Option<f32>,
    /// `SetImageQuality` target DPI of the current image quality.
    pub(crate) target_dpi: u16,
    pub(crate) fxaa: bool,
}

/// Frame limits the source offers (High, Normal).
const FRAME_RATES: [u16; 2] = [60, 30];

/// `SetImageQuality` target DPI of Normal, the image quality the source
/// constructs by default (High is 299, Low 180).
pub(crate) const SOURCE_DEFAULT_TARGET_DPI: u16 = 200;

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            frame_rate: FRAME_RATES[0],
            // The DPI law needs the display density. A page provides it as
            // devicePixelRatio; the native window keeps its full resolution.
            render_scale: if cfg!(target_arch = "wasm32") { None } else { Some(1.0) },
            target_dpi: SOURCE_DEFAULT_TARGET_DPI,
            fxaa: true,
        }
    }
}

/// Unity's `Screen.dpi` on Android is `DisplayMetrics.densityDpi`, and an
/// Android browser reports `devicePixelRatio` as `densityDpi / 160`. The
/// window scale factor (the page's devicePixelRatio) times 160 is therefore
/// the page's equivalent of the source's screen DPI.
const SCREEN_DPI_PER_SCALE_FACTOR: f32 = 160.;

/// Scene camera render scale of the source for a target DPI and a window
/// scale factor. `MysekaiQualitySettings.SetTargetDpi` stores
/// `clamp01(targetDpi / Screen.dpi)` as `MysekaiRenderSettings.RenderScale`
/// (an unknown density of 0 gives 1). `SceneMysekai.UpdateResolution` hands
/// it to ResoDynamix as the base camera scale, which writes it to the URP
/// asset, whose setter clamps to [0.1, 2]; URP renders at full size when the
/// scale is within 0.05 of 1.
pub(crate) fn source_render_scale(target_dpi: u16, scale_factor: f32) -> f32 {
    let screen_dpi = SCREEN_DPI_PER_SCALE_FACTOR * scale_factor;
    if !(screen_dpi > 0.) {
        return 1.;
    }
    let render_scale = (f32::from(target_dpi) / screen_dpi).clamp(0., 1.);
    let asset_scale = render_scale.clamp(0.1, 2.);
    if (1. - asset_scale).abs() < 0.05 {
        1.
    } else {
        asset_scale
    }
}

fn scene_render_scale(graphics: &GraphicsSettings, window: &Window) -> f32 {
    match graphics.render_scale {
        Some(fixed) => fixed.clamp(0.5, 1.),
        None => source_render_scale(graphics.target_dpi, window.resolution.base_scale_factor()),
    }
}

#[derive(Resource, Default)]
pub(crate) struct GameSettings {
    pub(crate) graphics: GraphicsSettings,
}

/// The host input routes must consult this gate before processing scene input.
/// The release guard also consumes the gesture that closes the modal.
#[derive(Resource, Default)]
pub(crate) struct SettingsPanel {
    pub(crate) open: bool,
    pub(crate) player_data: bool,
    /// Edge-triggered request consumed by fixture colour/cache ownership.
    /// The panel never removes an asset directly and never reports RSS.
    pub(crate) resource_clear_requested: bool,
    pub(crate) resource_status: String,
    release_guard: u8,
    status: String,
}

impl SettingsPanel {
    pub(crate) fn close_after_import(&mut self) {
        self.open = false;
        self.release_guard = 2;
    }

    pub(crate) fn blocks_world_input(&self) -> bool {
        self.open || self.release_guard != 0
    }
}

#[derive(Message)]
pub(crate) enum SettingsPanelRequest {
    Open,
    Close,
    Toggle,
}

#[derive(Component)]
pub(crate) struct PanelRoot;
#[derive(Component)]
pub(crate) struct SettingsBody;
#[derive(Component)]
struct SettingsUiCamera;
#[derive(Component)]
pub(crate) struct CompositeCamera;
#[derive(Component)]
pub(crate) struct CompositeQuad;
#[derive(Component)]
pub(crate) struct OriginalSceneOutput {
    target: RenderTarget,
    order: isize,
}

#[derive(Resource, Default)]
pub(crate) struct SceneSurface {
    image: Option<Handle<Image>>,
    physical_size: UVec2,
    /// Scene camera render scale applied by the last `apply_graphics`.
    applied_scale: f32,
}

#[derive(Debug, Clone, Copy, Component)]
pub(crate) enum Action {
    Toggle,
    Close,
    Volume(usize, f32),
    Fps(u16),
    Scale(Option<f32>),
    Fxaa,
    AudioOptions,
    Save,
    Defaults,
    Capture,
    PlayerData,
    SettingsPage,
    ClearInactiveResources,
}

#[derive(Component)]
pub(crate) enum ValueLabel {
    Performance,
    Volume(usize),
    Fps,
    Scale,
    Fxaa,
    Status,
    Residency,
}

/// Register once in the shared app assembly, after DefaultPlugins.
pub(crate) fn install(app: &mut App) {
    app.add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default());
    app.init_resource::<GameSettings>()
        .init_resource::<SettingsStore>()
        .init_resource::<SettingsPanel>()
        .init_resource::<SceneSurface>()
        .add_message::<SettingsPanelRequest>();
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Startup, pacing::start)
        .add_systems(First, pacing::note_update);
}

/// Per-frame counters for the browser host: engine assets and every game
/// material type visible to this module. The fixture surface and tree
/// materials and the UI clip material are private to their modules and are
/// not counted.
pub(crate) fn perf_plugin() -> moly_perf::PerfPlugin {
    moly_perf::PerfPlugin::default()
        .track::<Mesh>("Mesh")
        .track::<Image>("Image")
        .track::<StandardMaterial>("StandardMaterial")
        .track::<crate::avatar_material::AvatarMaterial>("AvatarMaterial")
        .track::<crate::character_material::CharacterMaterial>("CharacterMaterial")
        .track::<crate::emoticon::EmoticonMaterial>("EmoticonMaterial")
        .track::<crate::fixture_material::FixtureMaterial>("FixtureMaterial")
        .track::<crate::fixture::road::RoadMaterial>("RoadMaterial")
        .track::<crate::site_material::SiteMaterial>("SiteMaterial")
        .track::<crate::sky::SkyGradient>("SkyGradient")
        .track::<crate::uber_particle::UberParticleMaterial>("UberParticleMaterial")
}

pub(crate) fn scene_input_enabled(
    panel: Res<SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
) -> bool {
    !panel.blocks_world_input() && !library.blocks_world_input()
}

pub(crate) fn camera_input_enabled(
    panel: Res<SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
) -> bool {
    !panel.blocks_world_input() && !library.blocks_camera_input()
}

pub(crate) fn talk_input_enabled(
    panel: Res<SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
) -> bool {
    !panel.blocks_world_input() && !library.blocks_talk_input()
}

fn graphics_from_document(document: &Value) -> GraphicsSettings {
    let mut value = GraphicsSettings::default();
    let fields = &document["GameSettings"]["graphics"];
    if let Some(rate) = fields["frameRate"].as_u64() {
        if let Some(&rate) = FRAME_RATES.iter().find(|&&offered| u64::from(offered) == rate) {
            value.frame_rate = rate;
        }
    }
    // A stored null selects the source DPI law; an absent field keeps the default.
    match fields.get("renderScale") {
        Some(Value::Null) => value.render_scale = None,
        Some(scale) => {
            if let Some(scale) = scale.as_f64().filter(|scale| scale.is_finite()) {
                value.render_scale = Some((scale as f32).clamp(0.5, 1.0));
            }
        }
        None => {}
    }
    if let Some(fxaa) = fields["fxaa"].as_bool() {
        value.fxaa = fxaa;
    }
    value
}

pub(crate) fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    mut store: ResMut<SettingsStore>,
    mut settings: ResMut<GameSettings>,
    mut panel: ResMut<SettingsPanel>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    if let Some(document) = store.load() {
        settings.graphics = GraphicsSettings {
            // Applied by the source image-quality option at startup; not stored here.
            target_dpi: settings.graphics.target_dpi,
            ..graphics_from_document(&document)
        };
        panel.status = "Changes apply immediately. Save to keep them after restart.".into();
    } else {
        panel.status = "Storage unavailable. Changes will apply to this session.".into();
    }
    if stage.is_some() { return; }
    let font = fonts.add(Font::try_from_bytes(FONT.to_vec()).expect("bundled open font is valid"));
    let camera = commands
        .spawn((
            Camera2d,
            crate::camera::MYSEKAI_CAMERA_MSAA,
            Camera {
                order: UI_ORDER,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::none(),
            SettingsUiCamera,
        ))
        .id();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let trigger = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(14),
                    bottom: px(14),
                    ..default()
                },
                UiTargetCamera(camera),
                GlobalZIndex(1000),
            ))
            .id();
        add_button(
            &mut commands,
            trigger,
            &font,
            "Settings  F10",
            Action::Toggle,
        );
    }

    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.04, 0.08, 0.76)),
            FocusPolicy::Block,
            UiTargetCamera(camera),
            GlobalZIndex(1100),
            PanelRoot,
        ))
        .id();
    let content = commands
        .spawn((
            Node {
                width: percent(92),
                max_width: px(660),
                padding: UiRect::all(px(24)),
                flex_direction: FlexDirection::Column,
                row_gap: px(12),
                border_radius: BorderRadius::all(px(16)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.09, 0.12, 0.19)),
        ))
        .id();
    commands.entity(root).add_child(content);
    let heading = row(&mut commands, content);
    add_text(
        &mut commands,
        heading,
        &font,
        &format!("moly v{}", crate::VERSION),
        23.,
        None,
    );
    add_button(
        &mut commands,
        heading,
        &font,
        "Audio/video",
        Action::SettingsPage,
    );
    add_button(
        &mut commands,
        heading,
        &font,
        "Player data",
        Action::PlayerData,
    );
    add_button(&mut commands, heading, &font, "Close", Action::Close);
    crate::player_data_ui::spawn(&mut commands, content, &font);
    let settings_body = commands
        .spawn((
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(12),
                ..default()
            },
            SettingsBody,
        ))
        .id();
    commands.entity(content).add_child(settings_body);
    let content = settings_body;
    for (index, title) in ["Music", "Sound effects", "Voice"].into_iter().enumerate() {
        let line = row(&mut commands, content);
        add_text(&mut commands, line, &font, title, 18., None);
        add_button(
            &mut commands,
            line,
            &font,
            "-",
            Action::Volume(index, -0.05),
        );
        add_text(
            &mut commands,
            line,
            &font,
            "100%",
            18.,
            Some(ValueLabel::Volume(index)),
        );
        add_button(&mut commands, line, &font, "+", Action::Volume(index, 0.05));
    }
    let fps = row(&mut commands, content);
    add_text(
        &mut commands,
        fps,
        &font,
        "Frame limit",
        18.,
        Some(ValueLabel::Fps),
    );
    for rate in [FRAME_RATES[1], FRAME_RATES[0]] {
        add_button(
            &mut commands,
            fps,
            &font,
            &rate.to_string(),
            Action::Fps(rate),
        );
    }
    let scale = row(&mut commands, content);
    add_text(
        &mut commands,
        scale,
        &font,
        "Scene resolution",
        18.,
        Some(ValueLabel::Scale),
    );
    for (label, value) in [
        ("Auto", None),
        ("50%", Some(0.5)),
        ("67%", Some(0.67)),
        ("75%", Some(0.75)),
        ("100%", Some(1.)),
    ] {
        add_button(&mut commands, scale, &font, label, Action::Scale(value));
    }
    let fxaa = row(&mut commands, content);
    add_text(
        &mut commands,
        fxaa,
        &font,
        "FXAA",
        18.,
        Some(ValueLabel::Fxaa),
    );
    add_button(&mut commands, fxaa, &font, "Toggle", Action::Fxaa);
    let footer = row(&mut commands, content);
    add_button(
        &mut commands,
        footer,
        &font,
        "Audio options",
        Action::AudioOptions,
    );
    add_button(&mut commands, footer, &font, "Reset", Action::Defaults);
    add_button(&mut commands, footer, &font, "Save", Action::Save);
    add_button(
        &mut commands,
        footer,
        &font,
        "Screenshot  F12",
        Action::Capture,
    );
    let residency = row(&mut commands, content);
    add_text(
        &mut commands,
        residency,
        &font,
        "资源管理：测量中…",
        14.,
        Some(ValueLabel::Residency),
    );
    add_button(
        &mut commands,
        residency,
        &font,
        "清理闲置家具换色缓存",
        Action::ClearInactiveResources,
    );
    add_text(
        &mut commands,
        content,
        &font,
        "内存数字只覆盖可测的图片与网格；GPU 数字不含驱动与渲染目标。硬盘资源缓存请在网站的资源管理页清理。",
        13.,
        None,
    );
    add_text(
        &mut commands,
        content,
        &font,
        "",
        14.,
        Some(ValueLabel::Status),
    );
    add_text(
        &mut commands,
        content,
        &font,
        "Measuring frame time...",
        14.,
        Some(ValueLabel::Performance),
    );
}

fn row(commands: &mut Commands, parent: Entity) -> Entity {
    let entity = commands
        .spawn(Node {
            width: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(8),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(5),
            ..default()
        })
        .id();
    commands.entity(parent).add_child(entity);
    entity
}

fn add_text(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    text: &str,
    size: f32,
    value: Option<ValueLabel>,
) {
    let mut entity = commands.spawn((
        Text::new(text),
        TextFont {
            font: font.clone(),
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.94, 1.)),
    ));
    if let Some(value) = value {
        entity.insert(value);
    }
    let id = entity.id();
    commands.entity(parent).add_child(id);
}

fn add_button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    text: &str,
    action: Action,
) {
    let entity = commands
        .spawn((
            Button,
            action,
            Node {
                min_width: px(40),
                min_height: px(34),
                padding: UiRect::axes(px(12), px(7)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.19, 0.26, 0.38)),
        ))
        .id();
    commands.entity(parent).add_child(entity);
    add_text(commands, entity, font, text, 16., None);
}

/// Input ownership must expire even when the embedded host owns the settings UI.
pub(crate) fn release_input_guard(
    buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut panel: ResMut<SettingsPanel>,
) {
    if panel.release_guard > 0
        && !buttons.any_pressed([MouseButton::Left, MouseButton::Right])
        && touches.iter().next().is_none()
    {
        panel.release_guard -= 1;
    }
}

pub(crate) fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<SettingsPanelRequest>,
    actions: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut panel: ResMut<SettingsPanel>,
    mut settings: ResMut<GameSettings>,
    mut volumes: ResMut<LocalVolumeSettings>,
    mut bus: ResMut<VolumeBus>,
    mut store: ResMut<SettingsStore>,
    mut captures: MessageWriter<crate::frame_capture::CaptureFrame>,
    mut dialogs: ResMut<crate::menu_shell::ShellDialogState>,
) {
    let mut queued = Vec::new();
    if keys.just_pressed(KeyCode::F12) {
        queued.push(Action::Capture);
    }
    if keys.just_pressed(KeyCode::F10) {
        queued.push(Action::Toggle);
    }
    if panel.open && keys.just_pressed(KeyCode::Escape) {
        queued.push(Action::Close);
    }
    if panel.open
        && keys.just_pressed(KeyCode::KeyS)
        && (keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
            || keys.any_just_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]))
    {
        queued.push(Action::Save);
    }
    for request in requests.read() {
        match request {
            SettingsPanelRequest::Open => {
                panel.open = true;
            }
            SettingsPanelRequest::Close => queued.push(Action::Close),
            SettingsPanelRequest::Toggle => queued.push(Action::Toggle),
        }
    }
    for (interaction, action) in &actions {
        if *interaction == Interaction::Pressed {
            queued.push(*action);
        }
    }
    for action in queued {
        info!("[game-settings] {action:?}");
        match action {
            Action::Toggle => {
                panel.open = !panel.open;
                panel.release_guard = 2;
            }
            Action::Close => {
                panel.open = false;
                panel.release_guard = 2;
            }
            Action::Capture => {
                captures.write(crate::frame_capture::CaptureFrame);
            }
            _ if !panel.open => continue,
            Action::PlayerData => panel.player_data = true,
            Action::SettingsPage => panel.player_data = false,
            Action::ClearInactiveResources => {
                panel.resource_clear_requested = true;
                panel.resource_status =
                    "正在检查家具换色句柄…".into();
            }
            Action::AudioOptions => {
                dialogs.menu_open = false;
                dialogs.option_open = true;
                panel.open = false;
                panel.release_guard = 2;
            }
            Action::Volume(index, delta) => {
                let value = match index {
                    0 => &mut volumes.system.bgm,
                    1 => &mut volumes.system.se,
                    _ => &mut volumes.system.voice,
                };
                *value = ((*value + delta) * 100.).round().clamp(0., 100.) / 100.;
                audio::apply_system_volume(&mut bus, &volumes.system, "Game settings");
                panel.status = "Applied. Save to keep these changes.".into();
            }
            Action::Fps(rate) => {
                settings.graphics.frame_rate = rate;
                panel.status = "Frame limit applied.".into();
            }
            Action::Scale(scale) => {
                settings.graphics.render_scale = scale;
                panel.status = "Scene resolution applied; UI stays at display resolution.".into();
            }
            Action::Fxaa => {
                settings.graphics.fxaa = !settings.graphics.fxaa;
                panel.status = "Scene FXAA updated.".into();
            }
            Action::Defaults => {
                // The target DPI follows the source image-quality option, not this panel.
                settings.graphics = GraphicsSettings {
                    target_dpi: settings.graphics.target_dpi,
                    ..GraphicsSettings::default()
                };
                volumes.system = VolumeSettingData::default();
                audio::apply_system_volume(&mut bus, &volumes.system, "Reset game settings");
                panel.status = "Defaults applied. Save to keep these changes.".into();
            }
            Action::Save => {
                let g = settings.graphics;
                let mut sections = audio::settings_sections(&volumes).to_vec();
                sections.push(("GameSettings",json!({"version":2,"graphics":{"frameRate":g.frame_rate,"renderScale":g.render_scale,"fxaa":g.fxaa}})));
                panel.status = if store.save(&sections) {
                    "Saved.".into()
                } else {
                    format!(
                        "Save failed: {}. Session values are still active.",
                        store.last_error.as_deref().unwrap_or("storage unavailable")
                    )
                };
                if let Some(error) = &store.last_error {
                    warn!("[game-settings] {error}");
                }
            }
        }
    }
}

pub(crate) fn refresh_ui(
    panel: Res<SettingsPanel>,
    settings: Res<GameSettings>,
    volumes: Res<LocalVolumeSettings>,
    mut roots: Query<&mut Node, With<PanelRoot>>,
    mut bodies: Query<&mut Node, (With<SettingsBody>, Without<PanelRoot>)>,
    mut labels: Query<(&ValueLabel, &mut Text)>,
    images: Res<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    asset_server: Res<AssetServer>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    surface: Res<SceneSurface>,
    time: Res<Time<Real>>,
    mut last_performance: Local<f64>,
    mut residency_sample: Local<Option<(f64, u64, u64, u64)>>,
) {
    for mut node in &mut roots {
        node.display = if panel.open {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !panel.open {
        return;
    }
    for mut body in &mut bodies {
        body.display = if panel.player_data {
            Display::None
        } else {
            Display::Flex
        };
    }
    let refresh_performance = time.elapsed_secs_f64() - *last_performance >= 0.5;
    if refresh_performance {
        *last_performance = time.elapsed_secs_f64();
    }
    for (label, mut text) in &mut labels {
        let next = match label {
            ValueLabel::Performance => {
                if !refresh_performance {
                    continue;
                }
                use bevy::diagnostic::FrameTimeDiagnosticsPlugin as Frames;
                match diagnostics
                    .get(&Frames::FRAME_TIME)
                    .and_then(|d| d.average())
                {
                    Some(ms) if ms > 0. => {
                        format!("Actual: {:.1} FPS | {:.1} ms/frame", 1000. / ms, ms)
                    }
                    _ => "Measuring frame time...".into(),
                }
            }
            ValueLabel::Volume(index) => format!(
                "{:.0}%",
                100. * match index {
                    0 => volumes.system.bgm,
                    1 => volumes.system.se,
                    _ => volumes.system.voice,
                }
            ),
            ValueLabel::Fps => format!("Frame limit: {}", settings.graphics.frame_rate),
            ValueLabel::Scale => match settings.graphics.render_scale {
                Some(scale) => format!("Scene: {:.0}%", 100. * scale),
                None => format!("Scene: Auto {:.0}%", 100. * surface.applied_scale),
            },
            ValueLabel::Fxaa => format!(
                "FXAA: {}",
                if settings.graphics.fxaa { "On" } else { "Off" }
            ),
            ValueLabel::Status => panel.status.clone(),
            ValueLabel::Residency => {
                let now = time.elapsed_secs_f64();
                if residency_sample.as_ref().is_none_or(|(at, _, _, _)| now - *at >= 1.0) {
                    let value = crate::content_library::resource_residency_summary(
                        &images,
                        &meshes,
                        &asset_server,
                    );
                    *residency_sample = Some((
                        now,
                        value["cpuImageBytes"].as_u64().unwrap_or(0)
                            + value["cpuMeshBufferBytes"].as_u64().unwrap_or(0),
                        value["estimatedGpuTextureBytes"].as_u64().unwrap_or(0),
                        value["cpuAndGpuImageCount"].as_u64().unwrap_or(0),
                    ));
                }
                let (_, cpu, gpu, both) = residency_sample.expect("sampled above");
                let format_bytes = |bytes: u64| {
                    if bytes >= 1024 * 1024 {
                        format!("{:.1} MiB", bytes as f64 / (1024. * 1024.))
                    } else {
                        format!("{:.0} KiB", bytes as f64 / 1024.)
                    }
                };
                if panel.resource_status.is_empty() {
                    format!(
                        "资源管理：CPU 可测 {} · GPU 可测估算 {} · 双份纹理 {}",
                        format_bytes(cpu),
                        format_bytes(gpu),
                        both
                    )
                } else {
                    format!(
                        "资源管理：CPU 可测 {} · GPU 可测估算 {} · {}",
                        format_bytes(cpu),
                        format_bytes(gpu),
                        panel.resource_status
                    )
                }
            }
        };
        if **text != next {
            **text = next;
        }
    }
}

/// At 100%, retain the original direct-to-window camera target. Reduced
/// resolutions use a single scene image; source overlay cameras stay native size.
pub(crate) fn apply_graphics(
    mut commands: Commands,
    settings: Res<GameSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera,
            &mut RenderTarget,
            &mut Projection,
            Option<&mut Fxaa>,
            Option<&OriginalSceneOutput>,
        ),
        With<Camera3d>,
    >,
    mut surface: ResMut<SceneSurface>,
    mut images: ResMut<Assets<Image>>,
    composites: Query<Entity, With<CompositeCamera>>,
    mut quads: Query<(Entity, &mut Sprite), With<CompositeQuad>>,
    mut last_rate: Local<Option<u16>>,
) {
    let graphics = settings.graphics;
    if *last_rate != Some(graphics.frame_rate) {
        // Browser: updates paced by animation frames (see `pacing`).
        #[cfg(target_arch = "wasm32")]
        {
            if last_rate.is_none() {
                commands.insert_resource(pacing::winit_settings());
            }
            pacing::set_frame_rate(graphics.frame_rate);
        }
        // Native: the engine's reactive wait between updates.
        #[cfg(not(target_arch = "wasm32"))]
        {
            use bevy::winit::{UpdateMode, WinitSettings};
            let mode = UpdateMode::Reactive {
                wait: std::time::Duration::from_secs_f64(
                    1. / f64::from(graphics.frame_rate.max(1)),
                ),
                react_to_device_events: false,
                react_to_user_events: false,
                react_to_window_events: false,
            };
            commands.insert_resource(WinitSettings {
                focused_mode: mode,
                unfocused_mode: mode,
            });
        }
        *last_rate = Some(graphics.frame_rate);
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = scene_render_scale(&graphics, window);
    if surface.applied_scale != scale {
        surface.applied_scale = scale;
    }
    for (entity, mut camera, mut target, mut projection, fxaa, original) in &mut cameras {
        if let Some(mut fxaa) = fxaa {
            if fxaa.enabled != graphics.fxaa {
                fxaa.enabled = graphics.fxaa;
            }
        } else {
            commands.entity(entity).insert(Fxaa {
                enabled: graphics.fxaa,
                ..default()
            });
        }
        if scale >= 1. {
            if let Some(original) = original {
                *target = original.target.clone();
                camera.order = original.order;
                // Camera target changes do not invalidate Bevy's cached dimensions.
                // Updating the projection refreshes color/depth sizes together.
                projection.set_changed();
                commands.entity(entity).remove::<OriginalSceneOutput>();
            }
            continue;
        }
        // URP sizes a scaled camera target as (int)(pixel size * scale), at least 1.
        let size = UVec2::new(
            ((window.resolution.physical_width() as f32 * scale) as u32).max(1),
            ((window.resolution.physical_height() as f32 * scale) as u32).max(1),
        );
        let factor = size.x as f32 / window.width().max(1.);
        if surface.image.is_none() || surface.physical_size != size {
            let mut image =
                Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
            image.sampler = ImageSampler::linear();
            if let Some(handle) = &surface.image {
                if let Some(current) = images.get_mut(handle) {
                    *current = image;
                }
            } else {
                surface.image = Some(images.add(image));
            }
            surface.physical_size = size;
        }
        let image = surface
            .image
            .as_ref()
            .expect("scene image created above")
            .clone();
        if original.is_none() {
            commands.entity(entity).insert(OriginalSceneOutput {
                target: target.clone(),
                order: camera.order,
            });
        }
        camera.order = -1;
        let unchanged = matches!(&*target, RenderTarget::Image(current) if current.handle == image && current.scale_factor == factor);
        if !unchanged {
            *target = RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: factor,
            });
            projection.set_changed();
        }
        if composites.is_empty() {
            commands.spawn((
                Camera2d,
                crate::camera::MYSEKAI_CAMERA_MSAA,
                Camera {
                    order: 0,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderLayers::layer(COMPOSITE_LAYER),
                CompositeCamera,
            ));
            commands.spawn((
                Sprite {
                    image,
                    custom_size: Some(Vec2::new(window.width(), window.height())),
                    ..default()
                },
                Transform::default(),
                RenderLayers::layer(COMPOSITE_LAYER),
                CompositeQuad,
            ));
        }
    }
    if scale >= 1. {
        for entity in &composites {
            commands.entity(entity).despawn();
        }
        for (entity, _) in &mut quads {
            commands.entity(entity).despawn();
        }
        surface.image = None;
        surface.physical_size = UVec2::ZERO;
    } else {
        for (_, mut sprite) in &mut quads {
            sprite.custom_size = Some(Vec2::new(window.width(), window.height()));
        }
    }
}

/// Browser frame pacing for the frame limit.
///
/// The source limits the frame rate with `Application.targetFrameRate`. A
/// page presents only at the display's refresh, so every update runs inside
/// an animation frame, and frames that arrive before the limit's interval has
/// elapsed are skipped; the average rate equals the limit.
///
/// The engine loop is reactive: its wait is an hour and it ignores window and
/// device events, so it updates only when this pacer wakes it through the
/// event-loop proxy. The winit runner handles that wake-up in a microtask of
/// the same animation frame. The runner updates only after a redraw since
/// its previous update and requests its own animation frame at the end of
/// every update, so the pacer registers its next frame from a microtask
/// queued behind the wake-up; winit's redraw then precedes the pacer in the
/// next frame. A wake-up that finds no redraw yet is repeated next frame.
#[cfg(target_arch = "wasm32")]
mod pacing {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use bevy::prelude::*;
    use bevy::winit::{
        EventLoopProxy, EventLoopProxyWrapper, UpdateMode, WinitSettings, WinitUserEvent,
    };
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    struct Pacer {
        window: web_sys::Window,
        proxy: EventLoopProxy<WinitUserEvent>,
        interval_ms: Cell<f64>,
        due_ms: Cell<f64>,
        last_frame_ms: Cell<f64>,
        /// Smoothed display frame interval from animation-frame timestamps.
        display_ms: Cell<f64>,
        /// A wake-up was sent and no update has run since.
        pending: Cell<bool>,
        on_frame: RefCell<Option<Closure<dyn FnMut(f64)>>>,
        after_wake: RefCell<Option<Closure<dyn FnMut()>>>,
    }

    thread_local! {
        static PACER: RefCell<Option<Rc<Pacer>>> = const { RefCell::new(None) };
    }

    /// The engine loop waits for the pacer's wake-ups only.
    pub(super) fn winit_settings() -> WinitSettings {
        let mode = UpdateMode::Reactive {
            wait: Duration::from_secs(3600),
            react_to_device_events: false,
            react_to_user_events: true,
            react_to_window_events: false,
        };
        WinitSettings {
            focused_mode: mode,
            unfocused_mode: mode,
        }
    }

    pub(super) fn start(proxy: Res<EventLoopProxyWrapper>) {
        // Without the pacer the engine loop would never wake again.
        let window = web_sys::window().expect("frame pacing needs the page window");
        let pacer = Rc::new(Pacer {
            window,
            proxy: (**proxy).clone(),
            interval_ms: Cell::new(1000. / f64::from(super::FRAME_RATES[0])),
            due_ms: Cell::new(0.),
            last_frame_ms: Cell::new(0.),
            display_ms: Cell::new(1000. / 60.),
            pending: Cell::new(false),
            on_frame: RefCell::new(None),
            after_wake: RefCell::new(None),
        });
        // The closures own the pacer for the lifetime of the page.
        let frame_pacer = pacer.clone();
        *pacer.on_frame.borrow_mut() =
            Some(Closure::new(move |now: f64| on_frame(&frame_pacer, now)));
        let wake_pacer = pacer.clone();
        *pacer.after_wake.borrow_mut() = Some(Closure::new(move || request_frame(&wake_pacer)));
        request_frame(&pacer);
        PACER.with_borrow_mut(|slot| *slot = Some(pacer));
    }

    pub(super) fn set_frame_rate(rate: u16) {
        PACER.with_borrow(|pacer| {
            if let Some(pacer) = pacer {
                pacer.interval_ms.set(1000. / f64::from(rate.max(1)));
            }
        });
    }

    /// Runs in `First` of every update.
    pub(super) fn note_update() {
        PACER.with_borrow(|pacer| {
            if let Some(pacer) = pacer {
                pacer.pending.set(false);
            }
        });
    }

    fn request_frame(pacer: &Pacer) {
        if let Some(callback) = pacer.on_frame.borrow().as_ref() {
            let _ = pacer
                .window
                .request_animation_frame(callback.as_ref().unchecked_ref());
        }
    }

    fn on_frame(pacer: &Pacer, now: f64) {
        let last = pacer.last_frame_ms.replace(now);
        let delta = now - last;
        if last > 0. && delta > 2. && delta < 50. {
            pacer.display_ms.set(0.9 * pacer.display_ms.get() + 0.1 * delta);
        }
        if !pacer.pending.get() {
            let interval = pacer.interval_ms.get();
            // Fire on the display frame nearest to the due time.
            let tolerance = 0.5 * pacer.display_ms.get().min(interval);
            let due = pacer.due_ms.get();
            if now + tolerance >= due {
                // Phase-locked, so the average rate is the limit; after a
                // stall the schedule restarts instead of catching up.
                let next = if due + interval <= now {
                    now + interval
                } else {
                    due + interval
                };
                pacer.due_ms.set(next);
                pacer.pending.set(true);
            }
        }
        if pacer.pending.get() {
            let _ = pacer.proxy.send_event(WinitUserEvent::WakeUp);
            if let Some(after_wake) = pacer.after_wake.borrow().as_ref() {
                pacer
                    .window
                    .queue_microtask(after_wake.as_ref().unchecked_ref());
            }
        } else {
            request_frame(pacer);
        }
    }
}
