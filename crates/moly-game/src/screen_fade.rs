//! Two full-screen fade covers the expansion performances use:
//!
//! - `ScreenManager.fadeController`, the screen's own `ColorFader`.
//!   `FadeOut(delay, duration)` first writes black at the current alpha
//!   (`Set(0, 0, 0, current.a)`), then plays to `fadeOutColor`;
//!   `FadeIn(delay, duration)` plays to `fadeInColor`. The two colours are
//!   the manager's constructor values: `fadeInColor` (0, 0, 0, 0) and
//!   `fadeOutColor` (0, 0, 0, 1). `Play` is the fader coroutine the entry
//!   cover also runs ([`crate::entry::law::ColorFade`]); its callback fires
//!   on the frame the target colour is written.
//! - The cutscene screen's `_fadeImage` (`ScreenLayerMysekaiMysekaiCutScene`):
//!   `FadeIn(color, duration)` sets the image to the colour and tweens its
//!   alpha to 1 (`DOFade(1, duration)`); a colour whose squared length is
//!   below 1e-10 tweens the image to opaque black instead (`DOColor`).
//!   `FadeOut(duration)` tweens the alpha to 0 (`DOFade(0, duration)`).
//!   Both tweens use DOTween's default ease, OutQuad.
//!
//! Each cover is a quad in its own overlay camera above the field and the
//! HUD, below the entry cover. The covers are hidden while transparent.

use bevy::{
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
};

use crate::entry::law::{ColorFade, FadeStage};

const LAYER: usize = 27;
/// Above the wipe (150) and every HUD camera, below the entry cover (200).
const SCREEN_ORDER: isize = 160;
/// The cutscene screen sits below the screen manager's own fade panel.
const CUTSCENE_ORDER: isize = 158;
const EXTENT: f32 = 1.0e6;
/// `ScreenManager.fadeInColor` / `fadeOutColor` (constructor values).
pub(crate) const FADE_IN_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
pub(crate) const FADE_OUT_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Component)]
struct ScreenCover;

#[derive(Component)]
struct CutSceneCover;

/// The screen manager's fader.
#[derive(Resource)]
pub(crate) struct ScreenFader {
    colour: [f32; 4],
    fade: Option<ColorFade>,
    started_frame: u64,
    /// The `onFinish` of the last `Play` has fired.
    finished: bool,
    label: &'static str,
    since: f32,
}

impl Default for ScreenFader {
    fn default() -> Self {
        Self {
            colour: FADE_IN_COLOR,
            fade: None,
            started_frame: u64::MAX,
            finished: true,
            label: "",
            since: 0.0,
        }
    }
}

impl ScreenFader {
    pub(crate) fn finished(&self) -> bool {
        self.finished
    }
}

/// The cutscene screen's fade image and its tween.
#[derive(Resource)]
pub(crate) struct CutSceneFadeImage {
    colour: [f32; 4],
    tween: Option<AlphaTween>,
    started_frame: u64,
    finished: bool,
    label: &'static str,
}

impl Default for CutSceneFadeImage {
    fn default() -> Self {
        Self {
            colour: [0.0; 4],
            tween: None,
            started_frame: u64::MAX,
            finished: true,
            label: "",
        }
    }
}

impl CutSceneFadeImage {
    pub(crate) fn finished(&self) -> bool {
        self.finished
    }
}

/// A DOTween colour or alpha tween with the default ease (OutQuad).
#[derive(Clone, Copy, Debug)]
struct AlphaTween {
    from: [f32; 4],
    to: [f32; 4],
    duration: f32,
    elapsed: f32,
}

impl AlphaTween {
    fn sample(&self) -> [f32; 4] {
        let t = if self.duration > 0.0 {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let k = crate::camera::out_quad(t);
        let mut out = [0.0; 4];
        for (channel, value) in out.iter_mut().enumerate() {
            *value = self.from[channel] + (self.to[channel] - self.from[channel]) * k;
        }
        out
    }
}

fn frame(world: &World) -> u64 {
    world
        .get_resource::<bevy::diagnostic::FrameCount>()
        .map_or(0, |count| count.0 as u64)
}

fn delta(world: &World) -> f32 {
    world.get_resource::<Time>().map_or(0.0, Time::delta_secs)
}

/// `ScreenManager.FadeOut(delay, duration)`.
pub(crate) fn fade_out(world: &mut World, delay: f32, duration: f32, label: &'static str) {
    let current = world.resource::<ScreenFader>().colour;
    // Set(0, 0, 0, current.a) before Play.
    let base = [0.0, 0.0, 0.0, current[3]];
    play(world, base, FADE_OUT_COLOR, delay, duration, label);
}

/// `ScreenManager.FadeIn(delay, duration)`.
pub(crate) fn fade_in(world: &mut World, delay: f32, duration: f32, label: &'static str) {
    let base = world.resource::<ScreenFader>().colour;
    play(world, base, FADE_IN_COLOR, delay, duration, label);
}

fn play(
    world: &mut World,
    base: [f32; 4],
    target: [f32; 4],
    delay: f32,
    duration: f32,
    label: &'static str,
) {
    let dt = delta(world);
    let frame = frame(world);
    let (fade, written) = ColorFade::start(base, target, delay, duration, dt);
    let mut fader = world.resource_mut::<ScreenFader>();
    fader.colour = written.unwrap_or(base);
    fader.finished = fade.stage() == FadeStage::Done;
    fader.fade = (!fader.finished).then_some(fade);
    fader.started_frame = frame;
    fader.label = label;
    // The fader coroutine writes the colour for its current time, then adds
    // the frame's delta: the start frame's delta is already in its clock.
    fader.since = dt;
    let alpha = fader.colour[3];
    info!(
        "[screen-fade] {label}: ColorFader.Play(({:.0},{:.0},{:.0},{:.0}), delay {delay}, duration {duration}) from alpha {:.4}; t=0.000 alpha={alpha:.4}",
        target[0], target[1], target[2], target[3], base[3]
    );
    write_cover::<ScreenCover>(world, alpha_colour(world.resource::<ScreenFader>().colour));
}

/// `ScreenLayerMysekaiMysekaiCutScene.FadeIn(color, duration)`.
pub(crate) fn cutscene_fade_in(
    world: &mut World,
    color: [f32; 4],
    duration: f32,
    label: &'static str,
) {
    let mut image = world.resource::<CutSceneFadeImage>().colour;
    let squared: f32 = color.iter().map(|c| c * c).sum();
    let (from, to) = if squared < 9.999_999_4e-11 {
        // DOColor(BLACK_ALPHA_1, duration) from the image's current colour.
        (image, [0.0, 0.0, 0.0, 1.0])
    } else {
        // color = (r, g, b, current alpha), then DOFade(1).
        image = [color[0], color[1], color[2], image[3]];
        (image, [color[0], color[1], color[2], 1.0])
    };
    start_tween(world, from, to, duration, label);
}

/// `ScreenLayerMysekaiMysekaiCutScene.FadeOut(duration)`.
pub(crate) fn cutscene_fade_out(world: &mut World, duration: f32, label: &'static str) {
    let image = world.resource::<CutSceneFadeImage>().colour;
    let to = [image[0], image[1], image[2], 0.0];
    start_tween(world, image, to, duration, label);
}

fn start_tween(
    world: &mut World,
    from: [f32; 4],
    to: [f32; 4],
    duration: f32,
    label: &'static str,
) {
    let frame = frame(world);
    let tween = AlphaTween {
        from,
        to,
        duration,
        elapsed: 0.0,
    };
    let mut image = world.resource_mut::<CutSceneFadeImage>();
    image.colour = tween.sample();
    image.tween = Some(tween);
    image.finished = false;
    image.started_frame = frame;
    image.label = label;
    let colour = image.colour;
    info!(
        "[cutscene-fade] {label}: tween ({:.3},{:.3},{:.3},{:.3}) -> ({:.3},{:.3},{:.3},{:.3}) over {duration} s (OutQuad); t=0.000 alpha={:.4}",
        from[0], from[1], from[2], from[3], to[0], to[1], to[2], to[3], colour[3]
    );
    write_cover::<CutSceneCover>(world, colour);
}

/// Sets the cutscene fade image directly (the timeline's fade panel writes
/// the image every frame while one of its clips is active).
pub(crate) fn set_cutscene_image(world: &mut World, colour: [f32; 4]) {
    let mut image = world.resource_mut::<CutSceneFadeImage>();
    image.tween = None;
    image.finished = true;
    image.colour = colour;
    write_cover::<CutSceneCover>(world, colour);
}

fn alpha_colour(colour: [f32; 4]) -> [f32; 4] {
    colour
}

fn write_cover<C: Component>(world: &mut World, colour: [f32; 4]) {
    let mut covers = world.query_filtered::<(&mut Sprite, &mut Visibility), With<C>>();
    for (mut sprite, mut visibility) in covers.iter_mut(world) {
        sprite.color = Color::srgba(colour[0], colour[1], colour[2], colour[3]);
        let wanted = if colour[3] > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Startup: both covers, transparent.
pub(crate) fn spawn(mut commands: Commands) {
    for (order, cutscene) in [(SCREEN_ORDER, false), (CUTSCENE_ORDER, true)] {
        commands.spawn((
            Camera2d,
            crate::camera::MYSEKAI_CAMERA_MSAA,
            Camera {
                order,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Tonemapping::None,
            DebandDither::Disabled,
            RenderLayers::layer(LAYER),
        ));
        let sprite = (
            Sprite {
                color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                custom_size: Some(Vec2::splat(EXTENT)),
                ..default()
            },
            // Both quads share one layer; the cutscene image sits behind.
            Transform::from_xyz(0.0, 0.0, if cutscene { 0.0 } else { 1.0 }),
            RenderLayers::layer(LAYER),
            Visibility::Hidden,
        );
        if cutscene {
            commands.spawn((sprite, CutSceneCover));
        } else {
            commands.spawn((sprite, ScreenCover));
        }
    }
}

/// Update: the running fade and tween advance one frame; each frame's value
/// is logged with the time since its start.
pub(crate) fn advance(world: &mut World) {
    let dt = delta(world);
    let frame = frame(world);
    let mut screen = None;
    {
        let mut fader = world.resource_mut::<ScreenFader>();
        if fader.started_frame != frame {
            if let Some(fade) = fader.fade.as_mut() {
                let written = fade.resume(dt);
                let done = fade.stage() == FadeStage::Done;
                // The time this frame's colour was written for.
                let used = fader.since;
                fader.since += dt;
                if let Some(colour) = written {
                    fader.colour = colour;
                }
                if done {
                    fader.fade = None;
                    fader.finished = true;
                }
                screen = Some((fader.label, used, fader.colour[3], done));
            }
        }
    }
    if let Some((label, since, alpha, done)) = screen {
        info!(
            "[screen-fade] {label}: t={since:.3} alpha={alpha:.4}{}",
            if done {
                " (target written, onFinish)"
            } else {
                ""
            }
        );
        let colour = world.resource::<ScreenFader>().colour;
        write_cover::<ScreenCover>(world, colour);
    }
    let mut cutscene = None;
    {
        let mut image = world.resource_mut::<CutSceneFadeImage>();
        if image.started_frame != frame {
            if let Some(mut tween) = image.tween {
                tween.elapsed += dt;
                image.colour = tween.sample();
                let done = tween.elapsed >= tween.duration;
                image.tween = (!done).then_some(tween);
                if done {
                    image.finished = true;
                }
                cutscene = Some((
                    image.label,
                    tween.elapsed.min(tween.duration),
                    image.colour[3],
                    done,
                ));
            }
        }
    }
    if let Some((label, since, alpha, done)) = cutscene {
        info!(
            "[cutscene-fade] {label}: t={since:.3} alpha={alpha:.4}{}",
            if done { " (tween complete)" } else { "" }
        );
        let colour = world.resource::<CutSceneFadeImage>().colour;
        write_cover::<CutSceneCover>(world, colour);
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<ScreenFader>()
        .init_resource::<CutSceneFadeImage>()
        .add_systems(Startup, spawn)
        .add_systems(Update, advance);
}
