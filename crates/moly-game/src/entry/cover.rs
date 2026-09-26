//! The entry cover: `LiveTransitioner`'s `ContentRoot/Cover` graphic.
//!
//! The extracted prefab's cover is a `CustomImage` with no sprite, tinted by
//! its `ColorFader`: a full-screen quad in the fader's colour is the source
//! drawing. It sits on its own overlay camera above every field and UI
//! camera, from the first rendered frame. The loading indicator
//! (`LoadingContent`) shares the camera ([`super::indicator`]).
//!
//! The start particle: `Play` instantiates `effectPrefab`
//! (`fx_live_transition_v2_03_b`, the prefab document's second root) under
//! `effectEmitRoot` (`ContentRoot/EffectRoot`) and plays its first child's
//! system with its children; the fade-in's `onDone` (`ColorFader.Play
//! (WHITE_ALPHA_1, delay 1, duration 1)`, `UIUtility.PlayLiveTransition`'s
//! defaults) pauses them, and `Finish` resumes them; the object goes with the
//! transitioner 5 s after `Finish`. Its six systems draw through the
//! `root/pt_outside` UIParticle ([`crate::ui_particle`]). `EffectRoot` is
//! drawn before `Cover`, so while the cover is opaque nothing of it shows.
//! The MySekai scene starts with the cover already white, so the systems are
//! installed paused with the 2 s they ran before the pause
//! ([`crate::ui_particle::UiParticleHeadStart`]). Named differences: the
//! outgame frames are not known and the head start is stepped at 60 frames a
//! second (the systems seed themselves at random, as the source's
//! `autoRandomSeed` ones do); the transitioner canvas is a screen-space
//! overlay canvas with its own scaler (1920 x 1080, match width), and the
//! particle bakes against the root canvas as every UIParticle host here does.

use bevy::{
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
};

use super::law::WHITE_ALPHA_1;
use crate::ui_particle::{UiParticleHeadStart, UiParticleHost, UiParticlePaused};

const COVER_LAYER: usize = 29;
/// Above the balloon (1), site map (2), settings (100) and library (104)
/// cameras: the source cover canvas sorts at 800/1000, over every screen.
const COVER_ORDER: isize = 200;
/// The quad spans any window; the 2D camera's unit is one logical pixel.
const COVER_EXTENT: f32 = 1.0e6;

#[derive(Component)]
pub(crate) struct EntryCover;

#[derive(Component)]
pub(crate) struct EntryCoverCamera;

/// The start particle's canvas entity: the root canvas in canvas units.
#[derive(Component)]
pub(crate) struct EntryStartParticleCanvas;

/// The start particle's UIParticle host.
#[derive(Component)]
struct EntryStartParticle;

/// The extracted `LiveTransitioner` prefab document and its directory.
const DOCUMENT: &str = "moly://live-transitioner/LiveTransitioner/LiveTransitioner.json";
const DOCUMENT_DIR: &str = "live-transitioner/LiveTransitioner";
/// `LiveTransitioner.effectPrefab`'s root in the document.
const START_PARTICLE_ROOT: &str = "fx_live_transition_v2_03_b";
/// The prefab's UIParticle node (`m_Particles`: the six child systems).
const START_PARTICLE_NODE: &str = "root/pt_outside";
/// `ContentRoot`'s serialized local scale; every other node from the canvas
/// to the UIParticle is at the centre, unrotated, scale one (the UIParticle
/// node's own scale is driven).
const CONTENT_ROOT_SCALE: f32 = 0.999;
/// `UIUtility.PlayLiveTransition`'s `delay` and `duration` defaults, the
/// fade-in whose `onDone` pauses the particle.
const PLAY_FADE_DELAY: f32 = 1.0;
const PLAY_FADE_DURATION: f32 = 1.0;
/// Named: the frame of the outgame scene the head start is stepped in.
const HEAD_START_FRAME: f32 = 1.0 / 60.0;
/// `ContentRoot`'s CanvasGroup alpha times the root's (both serialized 1).
const START_PARTICLE_ALPHA: f32 = 1.0;

fn colour(value: [f32; 4]) -> Color {
    Color::srgba(value[0], value[1], value[2], value[3])
}

/// Startup: the cover is already opaque when the MySekai scene starts.
pub(crate) fn spawn(mut commands: Commands, server: Res<AssetServer>) {
    commands.spawn((
        Camera2d,
        crate::camera::MYSEKAI_CAMERA_MSAA,
        Camera {
            order: COVER_ORDER,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        // A default 2D camera tonemaps in the sprite shader, which would
        // grey the source white; the cover colour is a final value.
        Tonemapping::None,
        DebandDither::Disabled,
        RenderLayers::layer(COVER_LAYER),
        EntryCoverCamera,
    ));
    commands.spawn((
        Sprite {
            color: colour(WHITE_ALPHA_1),
            custom_size: Some(Vec2::splat(COVER_EXTENT)),
            ..default()
        },
        Transform::default(),
        RenderLayers::layer(COVER_LAYER),
        EntryCover,
    ));
    let canvas = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            EntryStartParticleCanvas,
        ))
        .id();
    let content = commands
        .spawn((
            Transform::from_scale(Vec3::splat(CONTENT_ROOT_SCALE)),
            Visibility::default(),
        ))
        .id();
    // EffectRoot is ContentRoot's first child, drawn before Cover (z 0).
    let host = commands
        .spawn((
            Transform::from_xyz(0.0, 0.0, -1.0),
            Visibility::default(),
            UiParticleHost {
                document: server.load(DOCUMENT),
                directory: DOCUMENT_DIR.to_owned(),
                root: START_PARTICLE_ROOT.to_owned(),
                node: START_PARTICLE_NODE.to_owned(),
                canvas,
                layer: COVER_LAYER,
                alpha: START_PARTICLE_ALPHA,
            },
            UiParticleHeadStart {
                seconds: PLAY_FADE_DELAY + PLAY_FADE_DURATION,
                frame: HEAD_START_FRAME,
            },
            UiParticlePaused,
            EntryStartParticle,
        ))
        .id();
    commands.entity(canvas).add_child(content);
    commands.entity(content).add_child(host);
    info!(
        "[entry-cover] start particle {START_PARTICLE_ROOT}/{START_PARTICLE_NODE}: paused after its {}s head start, under the cover",
        PLAY_FADE_DELAY + PLAY_FADE_DURATION
    );
}

/// Every frame: the start particle's canvas entity takes the root canvas
/// scale of the window.
pub(crate) fn fit_start_particle(world: &mut World) {
    let mut windows = world.query_filtered::<&Window, With<bevy::window::PrimaryWindow>>();
    let Some(scale) = windows.single(world).ok().and_then(|window| {
        world
            .get_resource::<crate::canvas::RootCanvas>()
            .map(|root| root.scale(window))
    }) else {
        return;
    };
    let mut canvases = world.query_filtered::<&mut Transform, With<EntryStartParticleCanvas>>();
    for mut transform in canvases.iter_mut(world) {
        if transform.scale != Vec3::splat(scale) {
            transform.scale = Vec3::splat(scale);
        }
    }
}

/// `Finish`: the paused start particle plays on.
pub(crate) fn resume_start_particle(world: &mut World) {
    let mut hosts = world.query_filtered::<Entity, With<EntryStartParticle>>();
    let hosts: Vec<_> = hosts.iter(world).collect();
    for host in &hosts {
        world.entity_mut(*host).remove::<UiParticlePaused>();
    }
    info!(
        "[entry-cover] start particle resumed ({} host)",
        hosts.len()
    );
}

/// Writes the fader colour; the source's `DeactiveSprite` at the fade end is
/// the hidden state here.
pub(crate) fn write(world: &mut World, value: [f32; 4]) {
    let mut covers = world.query_filtered::<(&mut Sprite, &mut Visibility), With<EntryCover>>();
    for (mut sprite, mut visibility) in covers.iter_mut(world) {
        sprite.color = colour(value);
        let wanted = if value[3] > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// `LiveTransitioner` destroys its own object: quad, camera and the start
/// particle go.
pub(crate) fn destroy(world: &mut World) {
    let mut roots = world.query_filtered::<Entity, Or<(
        With<EntryCover>,
        With<EntryCoverCamera>,
        With<EntryStartParticleCanvas>,
    )>>();
    let entities: Vec<_> = roots.iter(world).collect();
    for entity in entities {
        world.despawn(entity);
    }
}
