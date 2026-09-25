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
//!
//! Presses are cast as the event system casts them: the root canvas camera's
//! screen-point ray in world space. The camera is a child of the root canvas,
//! so the engine's canvas update does not move the canvas in front of it:
//! the canvas keeps its serialized position and rotation, with the uniform
//! local scale of its root rule, and the camera's world pose is its own
//! local pose taken through the canvas.

use bevy::math::Vec2;
use bevy::prelude::{warn, Resource};
use bevy::window::Window;
use moly_law::ui::{canvas, screen_ray};
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
    event_camera: Option<EventCamera>,
}

/// The root canvas camera as the event system casts from it: its clip
/// planes and viewport, its Transform's local pose under the root canvas,
/// and the root canvas Transform's serialized position and rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EventCamera {
    near: f32,
    far: f32,
    viewport: screen_ray::Rect,
    local_position: [f32; 3],
    local_rotation: [f32; 4],
    root_position: [f32; 3],
    root_rotation: [f32; 4],
}

/// The event camera's screen-point ray and the root canvas' world pose the
/// canvas corners are taken through.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventRay {
    pub(crate) ray: screen_ray::Ray,
    pub(crate) root: screen_ray::Pose,
    /// The engine's error when the point is outside the camera's view and it
    /// casts its fallback ray.
    pub(crate) error: Option<String>,
}

fn number(value: &Value, what: &str) -> f32 {
    let n = value.as_f64().unwrap_or_else(|| panic!("host canvas {what} is missing or not a number")) as f32;
    assert!(n.is_finite(), "host canvas {what} is not finite");
    n
}

fn vector3(value: &Value, what: &str) -> [f32; 3] {
    ["x", "y", "z"].map(|axis| number(&value[axis], what))
}

fn quaternion(value: &Value, what: &str) -> [f32; 4] {
    ["x", "y", "z", "w"].map(|axis| number(&value[axis], what))
}

fn path_id(value: &Value, what: &str) -> i64 {
    value.as_i64().unwrap_or_else(|| panic!("host canvas {what} is missing or not an integer"))
}

const DIRECT_CHILD_ONLY: &str =
    "host root canvas camera: only a camera Transform parented directly to the root canvas is ported";

impl EventCamera {
    /// Reads the camera's clip planes and viewport, its Transform chain up
    /// to the scene root and the root canvas Transform. Only a camera
    /// Transform parented directly to the root canvas is ported: a camera
    /// outside the canvas makes the engine move the canvas in front of it,
    /// and a deeper one needs the Transforms between them.
    fn read(host: &Value, fields: &Value, viewport: [f32; 4], chain: &Value, root_fields: &Value) -> Self {
        let root = path_id(&host["source"]["rootTransformPathId"], "source rootTransformPathId");
        let chain = chain.as_array().expect("host canvas rootCameraTransformChain is not a list");
        assert_eq!(chain.len(), 2, "{DIRECT_CHILD_ONLY}");
        let (camera, canvas) = (&chain[0], &chain[1]);
        assert_eq!(path_id(&camera["fields"]["m_Father"]["m_PathID"], "camera Transform m_Father"), root, "{DIRECT_CHILD_ONLY}");
        assert_eq!(path_id(&canvas["pathId"], "camera Transform chain link"), root, "host canvas camera chain ends off the root canvas");
        assert_eq!(
            path_id(&canvas["fields"]["m_Father"]["m_PathID"], "root canvas Transform m_Father"),
            0,
            "host root canvas Transform has a parent"
        );
        assert_eq!(&canvas["fields"], root_fields, "host canvas rootTransformFields differ from the camera chain's root link");
        Self {
            near: number(&fields["near clip plane"], "camera near clip plane"),
            far: number(&fields["far clip plane"], "camera far clip plane"),
            viewport: screen_ray::Rect { x: viewport[0], y: viewport[1], width: viewport[2], height: viewport[3] },
            local_position: vector3(&camera["fields"]["m_LocalPosition"], "camera Transform m_LocalPosition"),
            local_rotation: quaternion(&camera["fields"]["m_LocalRotation"], "camera Transform m_LocalRotation"),
            root_position: vector3(&root_fields["m_LocalPosition"], "root canvas m_LocalPosition"),
            root_rotation: quaternion(&root_fields["m_LocalRotation"], "root canvas m_LocalRotation"),
        }
    }
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
    /// canvas camera's `orthographic`, `m_projectionMatrixMode`,
    /// `m_NormalizedViewPortRect` and `orthographic size`.
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
        let mut event_camera = None;
        let camera = match host.get("rootCameraFields") {
            Some(fields) => {
                assert_eq!(
                    fields["orthographic"].as_bool(),
                    Some(true),
                    "host root canvas camera: only an orthographic camera is ported"
                );
                // Camera.ProjectionMatrixMode in declaration order: Explicit 0,
                // Implicit 1, PhysicalPropertiesBased 2. Only a projection built
                // from the camera's own fields is ported.
                assert_eq!(
                    fields["m_projectionMatrixMode"].as_i64(),
                    Some(1),
                    "host root canvas camera: only the implicit projection matrix mode is ported"
                );
                let viewport = &fields["m_NormalizedViewPortRect"];
                let full = ["x", "y", "width", "height"]
                    .map(|axis| number(&viewport[axis], "camera viewport rect"));
                assert_eq!(full, [0.0, 0.0, 1.0, 1.0], "host root canvas camera: only a full viewport is ported");
                if let (Some(chain), Some(root_fields)) =
                    (host.get("rootCameraTransformChain"), host.get("rootTransformFields"))
                {
                    event_camera = Some(EventCamera::read(host, fields, full, chain, root_fields));
                }
                canvas::RootCamera {
                    orthographic_size: number(&fields["orthographic size"], "camera orthographic size"),
                }
            }
            None => {
                assert_eq!(
                    camera_fields,
                    RootCameraFields::SharedRootMayLack,
                    "host canvas document of a region UI root has no root canvas camera fields \
                     (rootCameraFields); the root rect needs the camera's orthographic size and viewport"
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
        if event_camera.is_none() {
            assert_eq!(
                camera_fields,
                RootCameraFields::SharedRootMayLack,
                "host canvas document of a region UI root has no root canvas camera pose (rootCameraTransformChain, rootTransformFields); presses are cast from that camera"
            );
            warn!(
                "host canvas document has no root canvas camera pose; presses are tested at canvas points instead of the camera's screen-point ray until it is extracted"
            );
        }
        Self { scaler: canvas::Scaler { reference_resolution, mode }, camera, event_camera }
    }

    /// The event camera for a screen of `pixels` physical pixels and the root
    /// canvas' world pose there, when the host canvas carries the camera pose.
    pub(crate) fn event_camera(&self, pixels: [f32; 2]) -> Option<(screen_ray::OrthographicCamera, screen_ray::Pose)> {
        let event = self.event_camera?;
        let scale = self.for_pixels(pixels).local_scale;
        let root = screen_ray::Pose { position: event.root_position, rotation: event.root_rotation, scale: [scale; 3] };
        let (world_position, world_rotation) =
            screen_ray::child_world_pose(&root, event.local_position, event.local_rotation);
        let camera = screen_ray::OrthographicCamera {
            orthographic_size: self.camera.orthographic_size,
            near: event.near,
            far: event.far,
            viewport: event.viewport,
            world_position,
            world_rotation,
        };
        Some((camera, root))
    }

    /// The event camera's ray through `point` (engine screen pixels, origin
    /// bottom-left) on a screen of `pixels` physical pixels. None when the
    /// host canvas carries no camera pose.
    pub(crate) fn event_ray(&self, pixels: [f32; 2], point: [f32; 2]) -> Option<EventRay> {
        let (camera, root) = self.event_camera(pixels)?;
        let (ray, error) = screen_ray::screen_point_to_ray(&camera, pixels, point);
        Some(EventRay { ray, root, error })
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
