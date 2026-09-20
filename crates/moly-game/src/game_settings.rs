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
    winit::{UpdateMode, WinitSettings},
};
use serde_json::{json, Value};
use std::time::Duration;

use crate::{
    audio::{self, LocalVolumeSettings, VolumeBus, VolumeSettingData},
    settings_store::SettingsStore,
};

const COMPOSITE_LAYER: usize = 30;
const UI_ORDER: isize = 100;
const FONT: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GraphicsSettings {
    pub(crate) frame_rate: u16,
    pub(crate) render_scale: f32,
    pub(crate) fxaa: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            frame_rate: if cfg!(target_arch = "wasm32") { 30 } else { 60 },
            render_scale: 1.0,
            fxaa: true,
        }
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
}

#[derive(Debug, Clone, Copy, Component)]
pub(crate) enum Action {
    Toggle,
    Close,
    Volume(usize, f32),
    Fps(u16),
    Scale(f32),
    Fxaa,
    AudioOptions,
    Save,
    Defaults,
    Capture,
    PlayerData,
    SettingsPage,
}

#[derive(Component)]
pub(crate) enum ValueLabel {
    Performance,
    Volume(usize),
    Fps,
    Scale,
    Fxaa,
    Status,
}

/// Register once in the shared app assembly, after DefaultPlugins.
pub(crate) fn install(app: &mut App) {
    app.add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default());
    app.init_resource::<GameSettings>()
        .init_resource::<SettingsStore>()
        .init_resource::<SettingsPanel>()
        .init_resource::<SceneSurface>()
        .add_message::<SettingsPanelRequest>();
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
        if matches!(rate, 30 | 60 | 120) {
            value.frame_rate = rate as u16;
        }
    }
    if let Some(scale) = fields["renderScale"].as_f64() {
        if scale.is_finite() {
            value.render_scale = (scale as f32).clamp(0.5, 1.0);
        }
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
        settings.graphics = graphics_from_document(&document);
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
    for rate in [30, 60, 120] {
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
    for (label, value) in [("50%", 0.5), ("67%", 0.67), ("75%", 0.75), ("100%", 1.)] {
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
                settings.graphics = GraphicsSettings::default();
                volumes.system = VolumeSettingData::default();
                audio::apply_system_volume(&mut bus, &volumes.system, "Reset game settings");
                panel.status = "Defaults applied. Save to keep these changes.".into();
            }
            Action::Save => {
                let g = settings.graphics;
                let mut sections = audio::settings_sections(&volumes).to_vec();
                sections.push(("GameSettings",json!({"version":1,"graphics":{"frameRate":g.frame_rate,"renderScale":g.render_scale,"fxaa":g.fxaa}})));
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
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    time: Res<Time<Real>>,
    mut last_performance: Local<f64>,
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
            ValueLabel::Scale => format!("Scene: {:.0}%", 100. * settings.graphics.render_scale),
            ValueLabel::Fxaa => format!(
                "FXAA: {}",
                if settings.graphics.fxaa { "On" } else { "Off" }
            ),
            ValueLabel::Status => panel.status.clone(),
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
        let mode = UpdateMode::Reactive {
            wait: Duration::from_secs_f64(1. / f64::from(graphics.frame_rate.max(1))),
            react_to_device_events: false,
            react_to_user_events: false,
            react_to_window_events: false,
        };
        commands.insert_resource(WinitSettings {
            focused_mode: mode,
            unfocused_mode: mode,
        });
        *last_rate = Some(graphics.frame_rate);
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = graphics.render_scale.clamp(0.5, 1.);
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
        let size = UVec2::new(
            (window.resolution.physical_width() as f32 * scale)
                .round()
                .max(1.) as u32,
            (window.resolution.physical_height() as f32 * scale)
                .round()
                .max(1.) as u32,
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
