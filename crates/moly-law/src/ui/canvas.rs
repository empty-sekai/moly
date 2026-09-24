//! Root canvas scaling and the root rect.
//!
//! The game's screen manager chooses the scaler's match value from the screen
//! aspect before every canvas update: it compares the base screen aspect
//! (height over width of the base screen size 1920 x 1080, stored as a float
//! pair by the screen-manager constants' static constructor) with
//! `(float)Screen.height / (float)Screen.width`. Equal or taller screens
//! match width (0); wider screens match height (1). The base screen size
//! enters only this rule.
//!
//! `CanvasScaler.HandleScaleWithScreenSize` then blends the two axes in log
//! base 2 space against the scaler's own serialized reference resolution for
//! `MatchWidthOrHeight`, or takes the min/max ratio for `Expand`/`Shrink`.
//!
//! The root rect of a screen-space-camera canvas is set by the engine's
//! native canvas update from its camera, not from the screen size: it takes
//! the camera's pixel rect, the frustum plane size at the canvas plane
//! distance (for an orthographic camera with an implicit aspect: height =
//! size + size, width = height * (pixel width / pixel height)), the frustum
//! units per pixel (|frustum height| / pixel height), and sets the root size
//! to (frustum width / units per pixel, frustum height / units per pixel)
//! divided by the scale factor. A zero or NaN pixel height skips the frustum
//! and divides the pixel rect itself by the scale factor.

use super::unity_math;

/// Base screen size of the game's screen manager (width, height).
pub const BASE_SCREEN_SIZE: [f32; 2] = [1920.0, 1080.0];

/// `CanvasScaler.kLogBase`.
const LOG_BASE: f32 = 2.0;

/// `CanvasScaler.ScreenMatchMode` in its serialized order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMatchMode {
    MatchWidthOrHeight,
    Expand,
    Shrink,
}

impl ScreenMatchMode {
    pub fn from_serialized(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::MatchWidthOrHeight),
            1 => Some(Self::Expand),
            2 => Some(Self::Shrink),
            _ => None,
        }
    }
}

/// The match value the screen manager writes: 0 when the screen is at least
/// as tall as the base aspect (equality included), otherwise 1.
pub fn screen_match(screen_width: f32, screen_height: f32) -> f32 {
    if BASE_SCREEN_SIZE[1] / BASE_SCREEN_SIZE[0] <= screen_height / screen_width {
        0.0
    } else {
        1.0
    }
}

/// `CanvasScaler.HandleScaleWithScreenSize` for the main display.
pub fn scale_with_screen_size(
    screen: [f32; 2],
    reference_resolution: [f32; 2],
    mode: ScreenMatchMode,
    match_width_or_height: f32,
) -> f32 {
    match mode {
        ScreenMatchMode::MatchWidthOrHeight => {
            let log_width = unity_math::log(screen[0] / reference_resolution[0], LOG_BASE);
            let log_height = unity_math::log(screen[1] / reference_resolution[1], LOG_BASE);
            let log_weighted_average = unity_math::lerp(log_width, log_height, match_width_or_height);
            unity_math::pow(LOG_BASE, log_weighted_average)
        }
        ScreenMatchMode::Expand => unity_math::min(
            screen[0] / reference_resolution[0],
            screen[1] / reference_resolution[1],
        ),
        ScreenMatchMode::Shrink => unity_math::max(
            screen[0] / reference_resolution[0],
            screen[1] / reference_resolution[1],
        ),
    }
}

/// The root canvas scaler's serialized inputs (`CanvasScaler` in
/// `ScaleWithScreenSize` mode).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaler {
    /// `m_ReferenceResolution`.
    pub reference_resolution: [f32; 2],
    /// `m_ScreenMatchMode`.
    pub mode: ScreenMatchMode,
}

/// The root canvas camera fields the root rect reads. Only an orthographic
/// camera with an implicit aspect over the full viewport is ported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootCamera {
    /// `orthographicSize`.
    pub orthographic_size: f32,
}

/// Scale factor and root rect size of the game's root UI canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasRoot {
    /// The screen manager's match value for this screen.
    pub match_width_or_height: f32,
    /// `Canvas.scaleFactor`.
    pub scale_factor: f32,
    /// Root rect size in canvas units.
    pub size: [f32; 2],
}

/// The native screen-space-camera root size for a camera pixel rect of
/// `pixel` (full viewport: the screen size) and a scale factor.
pub fn camera_root_size(pixel: [f32; 2], camera: &RootCamera, scale_factor: f32) -> [f32; 2] {
    let (mut width, mut height) = (pixel[0], pixel[1]);
    // A zero or unordered pixel height leaves the pixel rect as it is.
    if height != 0.0 && !height.is_nan() {
        let aspect = width / height;
        let frustum_height = camera.orthographic_size + camera.orthographic_size;
        let frustum_width = frustum_height * aspect;
        let units_per_pixel = frustum_height.abs() / height;
        assert!(
            units_per_pixel != 0.0,
            "root canvas camera frustum has no height; the engine's fallback size for that case is not ported"
        );
        height = frustum_height / units_per_pixel;
        width = frustum_width / units_per_pixel;
    }
    [width / scale_factor, height / scale_factor]
}

/// The root canvas for a screen of `screen` pixels: the game's match rule,
/// the scaler against its serialized reference, then the camera root size.
pub fn root(screen: [f32; 2], scaler: &Scaler, camera: &RootCamera) -> CanvasRoot {
    let match_width_or_height = screen_match(screen[0], screen[1]);
    let scale_factor = scale_with_screen_size(
        screen,
        scaler.reference_resolution,
        scaler.mode,
        match_width_or_height,
    );
    CanvasRoot {
        match_width_or_height,
        scale_factor,
        size: camera_root_size(screen, camera, scale_factor),
    }
}
