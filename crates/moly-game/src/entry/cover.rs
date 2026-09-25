//! The entry cover: `LiveTransitioner`'s `ContentRoot/Cover` graphic.
//!
//! The extracted prefab's cover is a `CustomImage` with no sprite, tinted by
//! its `ColorFader`: a full-screen quad in the fader's colour is the source
//! drawing. It sits on its own overlay camera above every field and UI
//! camera, from the first rendered frame. The loading indicator
//! (`LoadingContent`) shares the camera ([`super::indicator`]); the start
//! particle (`effectPrefab`) is not drawn.

use bevy::{
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
};

use super::law::WHITE_ALPHA_1;

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

fn colour(value: [f32; 4]) -> Color {
    Color::srgba(value[0], value[1], value[2], value[3])
}

/// Startup: the cover is already opaque when the MySekai scene starts.
pub(crate) fn spawn(mut commands: Commands) {
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
    warn!(
        "[entry-cover] LiveTransitioner's start particle (effectPrefab fx_live_transition_v2_03_b) \
         is not drawn"
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

/// `LiveTransitioner` destroys its own object: quad and camera go.
pub(crate) fn destroy(world: &mut World) {
    let mut roots =
        world.query_filtered::<Entity, Or<(With<EntryCover>, With<EntryCoverCamera>)>>();
    let entities: Vec<_> = roots.iter(world).collect();
    for entity in entities {
        world.despawn(entity);
    }
}
