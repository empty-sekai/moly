//! Root UI canvas: the game's scaler match rule, the engine scaler with the
//! host canvas' own serialized scaler fields, and the camera root rect.
//!
//! Every UI layer in this host is laid out in canvas units. The rules take
//! the window's physical pixel size, as the engine reads the integer screen
//! size; the host UI cameras draw one camera unit per logical pixel, so the
//! drawn scale is the scale factor divided by the window's device pixel
//! ratio. The scaler's reference resolution and match mode are the host
//! canvas scaler's serialized fields; the base screen size of the screen
//! manager feeds only its match rule.

use bevy::math::Vec2;
use bevy::prelude::{warn, Resource};
use bevy::window::Window;
use moly_law::ui::canvas;
use serde_json::Value;

/// The screen manager's base screen width (its match rule only).
pub(crate) const CANVAS_REF_W: f32 = canvas::BASE_SCREEN_SIZE[0];
/// The screen manager's base screen height (its match rule only).
pub(crate) const CANVAS_REF_H: f32 = canvas::BASE_SCREEN_SIZE[1];

/// The host root canvas' scaler and camera inputs, from the host canvas
/// document. Present once that document has been parsed; UI layers wait for it.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub(crate) struct RootCanvas {
    scaler: canvas::Scaler,
    camera: canvas::RootCamera,
}

fn number(value: &Value, what: &str) -> f32 {
    let n = value.as_f64().unwrap_or_else(|| panic!("host canvas {what} is missing or not a number")) as f32;
    assert!(n.is_finite(), "host canvas {what} is not finite");
    n
}

/// Whether a host canvas document must carry the root canvas camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootCameraFields {
    /// A region root's document: the camera fields are required.
    Required,
    /// The shared root's document was exported before the camera fields
    /// were; it may lack them (one warning, and the scene camera's size).
    SharedRootMayLack,
}

impl RootCanvas {
    /// Reads the root scaler's `m_UiScaleMode`, `m_ReferenceResolution` and
    /// `m_ScreenMatchMode`, the root Canvas' render mode, and the root
    /// canvas camera's projection fields.
    pub(crate) fn from_host_canvas(host: &Value, camera_fields: RootCameraFields) -> Self {
        let scaler = &host["scalerFields"];
        assert_eq!(
            scaler["m_UiScaleMode"].as_i64(),
            Some(1),
            "host canvas scaler: only ScaleWithScreenSize is ported"
        );
        let reference = scaler["m_ReferenceResolution"]
            .as_array()
            .filter(|v| v.len() == 2)
            .expect("host canvas scaler m_ReferenceResolution");
        let reference_resolution = [
            number(&reference[0], "scaler reference width"),
            number(&reference[1], "scaler reference height"),
        ];
        let mode = scaler["m_ScreenMatchMode"]
            .as_i64()
            .and_then(canvas::ScreenMatchMode::from_serialized)
            .expect("host canvas scaler m_ScreenMatchMode");
        assert_eq!(
            host["rootCanvasFields"]["m_RenderMode"].as_i64(),
            Some(1),
            "host root Canvas: only the screen-space-camera root rect is ported"
        );
        let camera = match host.get("rootCameraFields") {
            Some(fields) => {
                assert_eq!(
                    fields["orthographic"].as_bool(),
                    Some(true),
                    "host root canvas camera: only an orthographic camera is ported"
                );
                let viewport = &fields["m_NormalizedViewPortRect"];
                let full = ["x", "y", "width", "height"]
                    .map(|axis| number(&viewport[axis], "camera viewport rect"));
                assert_eq!(full, [0.0, 0.0, 1.0, 1.0], "host root canvas camera: only a full viewport is ported");
                canvas::RootCamera {
                    orthographic_size: number(&fields["orthographic size"], "camera orthographic size"),
                }
            }
            None => {
                assert_eq!(
                    camera_fields,
                    RootCameraFields::SharedRootMayLack,
                    "host canvas document of a region UI root has no root canvas camera fields                      (rootCameraFields); the root rect needs the camera's orthographic size and viewport"
                );
                // The shared root's document does not carry the root canvas
                // camera. The Mysekai scene's UI camera is orthographic over
                // the full viewport with size 1.0; any power of two gives the
                // same root size bit for bit, other sizes can differ in the
                // last place.
                warn!(
                    "host canvas document has no root canvas camera fields; the root rect assumes \
                     an orthographic full-viewport camera of size 1.0 until they are extracted"
                );
                canvas::RootCamera { orthographic_size: 1.0 }
            }
        };
        Self { scaler: canvas::Scaler { reference_resolution, mode }, camera }
    }

    /// The scaler inputs read from the host canvas.
    #[cfg(test)]
    pub(crate) fn scaler(&self) -> canvas::Scaler {
        self.scaler
    }

    /// The root canvas for a screen of this many physical pixels.
    pub(crate) fn for_pixels(&self, pixels: [f32; 2]) -> canvas::CanvasRoot {
        canvas::root(pixels, &self.scaler, &self.camera)
    }

    fn for_window(&self, window: &Window) -> canvas::CanvasRoot {
        self.for_pixels([window.physical_width() as f32, window.physical_height() as f32])
    }

    /// Logical window pixels per canvas unit.
    pub(crate) fn scale(&self, window: &Window) -> f32 {
        self.for_window(window).scale_factor / window.scale_factor()
    }

    /// The root canvas rect size in canvas units.
    pub(crate) fn size(&self, window: &Window) -> Vec2 {
        Vec2::from_array(self.for_window(window).size)
    }
}
