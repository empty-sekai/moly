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

/// What the source `SetImageQuality` writes for one image quality: the target
/// DPI it passes to `SetTargetDpi` and `IsFXAAEnable`. High is (299, on),
/// Normal (200, on) and Low (180, off). Only the source image-quality option
/// (`info`) sets it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageQualityPair {
    pub(crate) target_dpi: u16,
    pub(crate) fxaa: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GraphicsSettings {
    /// Frame limit (`Application.targetFrameRate`): `MysekaiFpsQualityType`
    /// High is 60 and Normal 30.
    pub(crate) frame_rate: u16,
    /// Scene camera render scale chosen in the product panel; `None` applies
    /// the source DPI law ([`source_render_scale`]).
    pub(crate) render_scale: Option<f32>,
    /// The current image quality's pair. Loading saved settings and Defaults
    /// keep it, so the scene never runs one quality's target DPI with another
    /// quality's FXAA.
    pub(crate) image_quality: ImageQualityPair,
    /// Scene FXAA in effect: the pair's, or the panel toggle's session-only
    /// override until the image-quality option applies a pair again.
    pub(crate) fxaa: bool,
}

/// Frame limits the source offers (High, Normal).
const FRAME_RATES: [u16; 2] = [60, 30];

/// Image quality and frame limit of the no-recommendation path.
/// `MysekaiOptionSettingData`'s constructor sets image quality Normal and
/// leaves the fps quality at High. The JP client always starts there, and so
/// does a CN player who declines the first-run recommendation: the CN
/// client's `ScreenLayerLiveTop.RecommendSetting` otherwise offers the pair
/// for the device tier its publisher SDK reports (tier 3 and up High and 60,
/// tier 2 Normal and 30, tier 1 Low and 30) and saves it when accepted. The
/// port follows the no-recommendation path because that tier is a device
/// rating from the publisher SDK, which a browser has no counterpart for.
const NO_RECOMMENDATION_IMAGE_QUALITY: ImageQualityPair = ImageQualityPair {
    target_dpi: 200,
    fxaa: true,
};
const NO_RECOMMENDATION_FRAME_RATE: u16 = FRAME_RATES[0];

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            frame_rate: NO_RECOMMENDATION_FRAME_RATE,
            // The DPI law needs the display density. A page provides it as
            // devicePixelRatio; the native window keeps its full resolution.
            render_scale: if cfg!(target_arch = "wasm32") { None } else { Some(1.0) },
            image_quality: NO_RECOMMENDATION_IMAGE_QUALITY,
            fxaa: NO_RECOMMENDATION_IMAGE_QUALITY.fxaa,
        }
    }
}

/// Unity's `Screen.dpi` on Android is `DisplayMetrics.densityDpi`, and an
/// Android browser reports `devicePixelRatio` as `densityDpi / 160`. The
/// window scale factor (the page's devicePixelRatio) times 160 is therefore
/// the page's equivalent of the source's screen DPI.
const SCREEN_DPI_PER_SCALE_FACTOR: f32 = 160.;

/// Render scale of the Mysekai pipeline asset itself. While the DPI law keeps
/// the scene at full size ResoDynamix is inactive and restores this value to
/// the asset every frame, so URP renders the scene at it. Open item: the asset
/// (`MysekaiRenderPipelineAsset`, together with its upscaling filter, which
/// only matters when this scale is not 1) is not in the extracted client data
/// yet, and 1 stands in for its value until the extraction reads it.
const PIPELINE_ASSET_RENDER_SCALE: f32 = 1.;

/// Scene camera render scale of the source for a target DPI and a window
/// scale factor. `MysekaiQualitySettings.SetTargetDpi` stores
/// `clamp01(targetDpi / Screen.dpi)` as `MysekaiRenderSettings.RenderScale`
/// (an unknown density of 0 gives 1). `SceneMysekai.UpdateResolution` hands
/// it to ResoDynamix as the base camera scale; below 1 ResoDynamix writes it
/// to the URP asset, otherwise the asset keeps its own scale. The asset's
/// setter clamps to [0.1, 2], and URP renders at full size when the scale is
/// within 0.05 of 1.
pub(crate) fn source_render_scale(target_dpi: u16, scale_factor: f32) -> f32 {
    let screen_dpi = SCREEN_DPI_PER_SCALE_FACTOR * scale_factor;
    let render_scale = if screen_dpi > 0. {
        (f32::from(target_dpi) / screen_dpi).clamp(0., 1.)
    } else {
        1.
    };
    let asset_scale = if render_scale < 1. {
        render_scale
    } else {
        PIPELINE_ASSET_RENDER_SCALE
    }
    .clamp(0.1, 2.);
    if (1. - asset_scale).abs() < 0.05 {
        1.
    } else {
        asset_scale
    }
}

fn scene_render_scale(graphics: &GraphicsSettings, window: &Window) -> f32 {
    match graphics.render_scale {
        Some(fixed) => fixed.clamp(0.5, 1.),
        None => source_render_scale(
            graphics.image_quality.target_dpi,
            window.resolution.base_scale_factor(),
        ),
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
/// material type visible to this module. `FixtureSurfaceMaterial`,
/// `FixtureTreeMaterial` and `UiClipMaterial` are private to their modules
/// and are not counted.
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
    // FXAA is half of the image-quality pair, so a stored `fxaa` is not read.
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
        // The source image-quality option applied its pair at startup; the
        // panel does not store it.
        let image_quality = settings.graphics.image_quality;
        settings.graphics = GraphicsSettings {
            image_quality,
            fxaa: image_quality.fxaa,
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
                // A session-only override: FXAA belongs to the image-quality
                // pair, which the next image-quality change, Defaults or a
                // restart applies again.
                settings.graphics.fxaa = !settings.graphics.fxaa;
                panel.status = "Scene FXAA changed for this session.".into();
            }
            Action::Defaults => {
                // The image-quality pair follows the source option, not this panel.
                let image_quality = settings.graphics.image_quality;
                settings.graphics = GraphicsSettings {
                    image_quality,
                    fxaa: image_quality.fxaa,
                    ..GraphicsSettings::default()
                };
                volumes.system = VolumeSettingData::default();
                audio::apply_system_volume(&mut bus, &volumes.system, "Reset game settings");
                panel.status = "Defaults applied. Save to keep these changes.".into();
            }
            Action::Save => {
                let g = settings.graphics;
                let mut sections = audio::settings_sections(&volumes).to_vec();
                sections.push(("GameSettings",json!({"version":2,"graphics":{"frameRate":g.frame_rate,"renderScale":g.render_scale}})));
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
        if camera.order != -1 {
            camera.order = -1;
        }
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
        let quad_size = Some(Vec2::new(window.width(), window.height()));
        for (_, mut sprite) in &mut quads {
            if sprite.custom_size != quad_size {
                sprite.custom_size = quad_size;
            }
        }
    }
}

/// Browser frame pacing for the frame limit.
///
/// The source sets `Application.targetFrameRate`, and its Android player
/// (Optimized Frame Pacing off) starts a frame on the first display refresh
/// that is at least `trunc(refresh rate / limit + 0.5)` refreshes after the
/// refresh the previous frame started on. A frame that starts late counts from
/// the refresh it started on, so a stall is never caught up. The spacing is a
/// whole number of refreshes and the rate follows the display: at a limit of
/// 60 a 120 Hz display updates on every second refresh, and so does a 144 Hz
/// one, 72 times a second.
///
/// A page sees each display refresh as an animation frame. The pacer counts
/// refreshes with `refresh_count::RefreshCount` and wakes the engine on the
/// animation frame that completes the spacing.
///
/// The engine loop is reactive: its wait is an hour and it ignores window and
/// device events, so it updates only when this pacer wakes it through the
/// event-loop proxy. The winit runner handles that wake-up in a microtask of
/// the same animation frame. The runner updates only after a redraw since
/// its previous update and requests its own animation frame at the end of
/// every update, so the pacer registers its next frame from a microtask
/// queued behind the wake-up; winit's redraw then precedes the pacer in the
/// next frame. A wake-up that finds no redraw yet is repeated next frame, and
/// the update then counts from that frame's refresh.
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

    use super::refresh_count::RefreshCount;

    struct Pacer {
        window: web_sys::Window,
        proxy: EventLoopProxy<WinitUserEvent>,
        frame_rate: Cell<u16>,
        refreshes: RefCell<RefreshCount>,
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
            frame_rate: Cell::new(super::NO_RECOMMENDATION_FRAME_RATE),
            refreshes: RefCell::new(RefreshCount::new()),
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
                pacer.frame_rate.set(rate);
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
        {
            let mut refreshes = pacer.refreshes.borrow_mut();
            refreshes.frame(now);
            if pacer.pending.get() {
                // The update asked for last frame has not run yet; it starts
                // on this refresh instead.
                refreshes.start_update();
            } else if refreshes.due(pacer.frame_rate.get()) {
                refreshes.start_update();
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

/// Display-refresh counting of the browser pacer; it uses no page API.
#[cfg(target_arch = "wasm32")]
mod refresh_count {
    /// Animation-frame intervals kept for the refresh estimate (about half a
    /// second at 60 Hz).
    const WINDOW: usize = 32;
    /// Longer intervals (a hidden page, a long stall) still advance the count
    /// but stay out of the refresh estimate.
    const MAX_SAMPLE_MS: f64 = 250.;

    /// Counts display refreshes from animation-frame timestamps and applies
    /// the source's refresh spacing between frame starts.
    pub(super) struct RefreshCount {
        intervals: [f64; WINDOW],
        stored: usize,
        next: usize,
        /// Estimated display refresh interval.
        refresh_ms: f64,
        last_frame_ms: Option<f64>,
        /// Refreshes counted since the first animation frame.
        refreshes: u64,
        /// Refresh the latest update started on.
        update_refresh: Option<u64>,
    }

    impl RefreshCount {
        pub(super) fn new() -> Self {
            Self {
                intervals: [0.; WINDOW],
                stored: 0,
                next: 0,
                refresh_ms: 1000. / 60.,
                last_frame_ms: None,
                refreshes: 0,
                update_refresh: None,
            }
        }

        /// Advances to the animation frame stamped `now_ms`. The interval
        /// since the previous animation frame counts as many refreshes as it
        /// spans; a stray callback within the same refresh counts none.
        pub(super) fn frame(&mut self, now_ms: f64) {
            let Some(last_ms) = self.last_frame_ms.replace(now_ms) else {
                return;
            };
            let interval = now_ms - last_ms;
            if !(interval > 0.) {
                return;
            }
            if interval < MAX_SAMPLE_MS {
                self.sample(interval);
            }
            self.refreshes += (interval / self.refresh_ms).round() as u64;
        }

        /// Whether the source starts a frame on the current refresh.
        pub(super) fn due(&self, frame_rate: u16) -> bool {
            self.update_refresh
                .is_none_or(|start| self.refreshes - start >= self.spacing(frame_rate))
        }

        /// Records that an update starts on the current refresh.
        pub(super) fn start_update(&mut self) {
            self.update_refresh = Some(self.refreshes);
        }

        /// Refreshes between frame starts, `trunc(refresh rate / limit + 0.5)`
        /// in single precision as the source computes it. The estimated rate
        /// is rounded to whole hertz first, so timing noise cannot flip the
        /// spacing where the ratio is exactly half-way (90 Hz at a limit of
        /// 60). The spacing is at least one, since a page updates at most once
        /// per animation frame.
        fn spacing(&self, frame_rate: u16) -> u64 {
            let refresh_rate = (1000. / self.refresh_ms).round() as f32;
            ((refresh_rate / f32::from(frame_rate.max(1)) + 0.5) as u64).max(1)
        }

        /// The refresh interval is the mean of the lowest cluster of stored
        /// intervals: the shortest interval that has another one within 25%
        /// above it, together with those. Longer intervals span skipped
        /// refreshes, and a lone shorter one is a stray callback.
        fn sample(&mut self, interval: f64) {
            self.intervals[self.next] = interval;
            self.next = (self.next + 1) % WINDOW;
            self.stored = (self.stored + 1).min(WINDOW);
            let mut sorted = self.intervals;
            let sorted = &mut sorted[..self.stored];
            sorted.sort_unstable_by(f64::total_cmp);
            let members = self.stored.min(2);
            for (index, &low) in sorted.iter().enumerate() {
                let end = sorted.partition_point(|&value| value <= 1.25 * low);
                let cluster = &sorted[index..end];
                if cluster.len() >= members {
                    self.refresh_ms = cluster.iter().sum::<f64>() / cluster.len() as f64;
                    return;
                }
            }
        }
    }
}
