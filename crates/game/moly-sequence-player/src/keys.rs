//! Writing the camera as it stands into a document: one key of a camera
//! clip's eye, and the look and the lens the key is taken with.
//!
//! [`capture`] edits a document's JSON in place and keeps every field it does
//! not write, the key order included. It writes only the key form of a shot:
//! an eye that is `keys` already keeps its points, an eye the clip leaves out
//! (which samples the live camera) becomes one `keys` point, and an eye of any
//! other shape is refused by that shape's name. A look that is a `dir` takes
//! the angles that look from the key's eye to [`Aim::target`]; a look at a
//! target is left as it is and the reply says so. The field of view is a
//! number the clip did not set, the key of a keyed field of view, or the
//! camera's value where the clip's is a fixed number.
//!
//! The caller owns the timing: it passes the camera clip the played time
//! falls in and the clip-local time of that moment, which the compiled
//! document knows from its own timing.

use bevy::math::Vec3;
use moly_sequence::CameraClip;
use serde_json::{json, Value};

/// Two keys this close together (seconds) are one key.
const SAME_KEY: f32 = 1e-3;

/// Where the camera stands when a key is taken: its eye, the point it looks
/// at and the field of view across, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov: f32,
}

/// What a captured key wrote, for the reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    /// The camera track's place in the document, such as `tracks[0]`.
    pub track: String,
    /// The clip's index in that track.
    pub clip: usize,
    /// The key's time in the clip's content.
    pub local: f32,
    /// A key within [`SAME_KEY`] of the time stood there and took the value.
    pub replaced: bool,
    /// `"dir"` when the look was written, `"at"` when it was left as it is.
    pub aim: &'static str,
    /// `"added"`, `"replaced"` or `"fixed"`: what the field of view did.
    pub fov: &'static str,
    /// The yaw and pitch in degrees written into the look.
    pub angles: Option<[f32; 2]>,
    /// The field of view the key was taken with.
    pub field_of_view: f32,
}

impl Capture {
    /// The reply as a log note.
    pub fn note(&self) -> String {
        let key = if self.replaced { "replaced" } else { "added" };
        let aim = match (self.aim, self.angles) {
            ("dir", Some([yaw, pitch])) => format!("the look is written as yaw {yaw:.2}, pitch {pitch:.2} degrees"),
            _ => "the look keeps its target".to_owned(),
        };
        let fov = match self.fov {
            "added" => format!("the field of view takes a key of {:.2} degrees", self.field_of_view),
            "replaced" => format!("the field of view's key takes {:.2} degrees", self.field_of_view),
            _ => format!("the field of view is {:.2} degrees", self.field_of_view),
        };
        format!("{} clips[{}]: key {key} at {:.3} s; {aim}; {fov}", self.track, self.clip, self.local)
    }
}

/// The yaw and pitch in degrees that look from `eye` to `target`: yaw about
/// +Y with 0 facing +Z, pitch positive upwards. A target straight above or
/// below takes pitch ±90 at yaw 0.
pub fn angles(eye: Vec3, target: Vec3) -> (f32, f32) {
    let flat = Vec3::new(target.x - eye.x, 0.0, target.z - eye.z);
    let pitch = (target.y - eye.y).atan2(flat.length()).to_degrees();
    let yaw = if flat.length_squared() > 1e-12 { flat.x.atan2(flat.z).to_degrees() } else { 0.0 };
    (yaw, pitch)
}

/// A point as a document writes it.
fn point(at: Vec3) -> Value {
    json!([at.x, at.y, at.z])
}

/// Writes the key `aim` at the clip-local time of the camera clip `clip`
/// into `document`, which it edits in place.
pub fn capture(document: &mut Value, clip: &CameraClip, aim: &Aim) -> Result<Capture, String> {
    let at = clip.local;
    let (eye, target, fov) = (aim.eye, aim.target, aim.fov);
    if !at.is_finite() {
        return Err(format!("the key's time {at} is not a number"));
    }
    if !eye.is_finite() || !target.is_finite() || !fov.is_finite() || fov <= 0.0 {
        return Err("the camera's eye, look point or field of view is not one a document takes".to_owned());
    }
    let clip_index = clip.index;
    let tracks = document.get_mut("tracks").and_then(Value::as_array_mut).ok_or("the document has no tracks")?;
    let (ti, camera) = tracks
        .iter_mut()
        .enumerate()
        .find(|(_, track)| track.get("kind").and_then(Value::as_str) == Some("camera"))
        .ok_or("the document has no camera track")?;
    let clips = camera.get_mut("clips").and_then(Value::as_array_mut).ok_or("the camera track has no clips")?;
    let count = clips.len();
    let shot = clips.get_mut(clip_index).ok_or_else(|| format!("the camera track holds {count} clips, so there is no clip {clip_index}"))?;
    let shot = shot.as_object_mut().ok_or_else(|| format!("tracks[{ti}].clips[{clip_index}] is not an object"))?;
    let place = format!("tracks[{ti}].clips[{clip_index}]");

    // The eye.
    let replaced = match shot.get_mut("eye") {
        None => {
            shot.insert("eye".to_owned(), json!({ "keys": [{ "t": at, "pos": point(eye) }] }));
            false
        }
        Some(value) => {
            let Some(written_eye) = value.as_object_mut() else {
                return Err(format!("{place}: the eye is not an object"));
            };
            if written_eye.len() != 1 || !written_eye.contains_key("keys") {
                let shape = ["at", "path", "orbit", "follow"].into_iter().find(|shape| written_eye.contains_key(*shape));
                return match shape {
                    Some(shape) => Err(format!("{place}: the eye is a \"{shape}\"; keys are written into a \"keys\" eye or one the clip leaves out")),
                    None => Err(format!("{place}: the eye names no shape a document has")),
                };
            }
            let keys = written_eye.get_mut("keys").ok_or_else(|| format!("{place}: the eye is not an object"))?;
            insert(keys, at, json!({ "t": at, "pos": point(eye) }), &format!("{place}.eye"))?
        }
    };

    // The look.
    let (yaw, pitch) = angles(eye, target);
    let mut written = None;
    let aim_word = match shot.get_mut("aim") {
        None => {
            shot.insert("aim".to_owned(), json!({ "dir": [yaw, pitch] }));
            written = Some([yaw, pitch]);
            "dir"
        }
        Some(value) => {
            let Some(goal) = value.as_object_mut() else {
                return Err(format!("{place}: the aim is not an object"));
            };
            if goal.contains_key("at") {
                "at"
            } else if goal.contains_key("dir") {
                goal.insert("dir".to_owned(), json!([yaw, pitch]));
                written = Some([yaw, pitch]);
                "dir"
            } else {
                return Err(format!("{place}: the aim names neither \"at\" nor \"dir\""));
            }
        }
    };

    // The lens.
    let fov_word = match shot.get_mut("fov") {
        None => {
            shot.insert("fov".to_owned(), json!(fov));
            "fixed"
        }
        Some(value) => {
            let keys = value.as_object_mut().filter(|object| object.len() == 1).and_then(|object| object.get_mut("keys"));
            match keys {
                Some(keys) => {
                    if insert(keys, at, json!({ "t": at, "value": fov }), &format!("{place}.fov"))? {
                        "replaced"
                    } else {
                        "added"
                    }
                }
                None => {
                    *value = json!(fov);
                    "fixed"
                }
            }
        }
    };
    Ok(Capture { track: format!("tracks[{ti}]"), clip: clip_index, local: at, replaced, aim: aim_word, fov: fov_word, angles: written, field_of_view: fov })
}

/// Puts `key` (whose `t` is `at`) in the time-ordered `keys` array, or makes
/// the array of it. A key within [`SAME_KEY`] of `at` stands where it is, keeps
/// its other fields and takes the new values. Answers whether a key was
/// replaced.
fn insert(keys: &mut Value, at: f32, key: Value, place: &str) -> Result<bool, String> {
    let array = match keys {
        Value::Array(array) => array,
        Value::Null => {
            *keys = Value::Array(Vec::new());
            keys.as_array_mut().ok_or_else(|| format!("{place} does not hold a list"))?
        }
        _ => return Err(format!("{place}: the keys are not a list")),
    };
    let time = |entry: &Value| entry.get("t").and_then(Value::as_f64).map(|t| t as f32);
    if let Some(index) = array.iter().position(|entry| time(entry).is_some_and(|t| (t - at).abs() <= SAME_KEY)) {
        let Some(entry) = array[index].as_object_mut() else {
            return Err(format!("{place}: key {index} is not an object"));
        };
        if let Some(Value::Object(fields)) = Some(key.clone()) {
            for (name, value) in fields {
                if name != "t" {
                    entry.insert(name, value);
                }
            }
        }
        return Ok(true);
    }
    let index = array.iter().position(|entry| time(entry).is_some_and(|t| t > at)).unwrap_or(array.len());
    array.insert(index, key);
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_sequence::doc::Shape;
    use moly_sequence::NoCatalog;

    /// The angle in degrees between the way `pose` looks and the way a camera at
    /// the yaw and pitch (degrees) looks.
    fn view_error(pose: moly_sequence::CameraPose, yaw: f32, pitch: f32) -> f32 {
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
        let direction = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos());
        direction.angle_between(pose.rotation * Vec3::NEG_Z).to_degrees()
    }

    /// A camera clip of index, content time and the plain timing a document
    /// gives a clip that writes no timing.
    fn clip(index: usize, local: f32) -> CameraClip {
        CameraClip { index, start: 0.0, end: 4.0, local, shape: plain() }
    }

    fn plain() -> Shape {
        Shape { clip_in: 0.0, speed: 1.0, blend_in: 0.0, blend_out: 0.0, ease_in: Default::default(), ease_out: Default::default(), before: None, after: None }
    }

    fn aim() -> Aim {
        Aim { eye: Vec3::new(1.0, 2.0, 3.0), target: Vec3::new(1.0, 2.0, 4.0), fov: 50.0 }
    }

    fn document(clip: Value) -> Value {
        serde_json::json!({
            "version": 1,
            "name": "shot",
            "tracks": [{ "kind": "camera", "clips": [clip] }]
        })
    }

    fn keys(document: &Value) -> &[Value] {
        document["tracks"][0]["clips"][0]["eye"]["keys"].as_array().map(Vec::as_slice).unwrap_or(&[])
    }

    #[test]
    fn a_clip_without_an_eye_or_a_look_takes_its_first_key() {
        let mut document = document(serde_json::json!({ "duration": 4.0 }));
        let wrote = capture(&mut document, &clip(0, 1.5), &aim()).expect("a fresh clip takes a key");
        assert!(!wrote.replaced);
        assert_eq!(wrote.aim, "dir");
        assert_eq!(wrote.fov, "fixed");
        assert_eq!(keys(&document).len(), 1);
        assert_eq!(keys(&document)[0], serde_json::json!({ "t": 1.5, "pos": [1.0, 2.0, 3.0] }));
        assert_eq!(document["tracks"][0]["clips"][0]["aim"], serde_json::json!({ "dir": [0.0, 0.0] }));
        assert_eq!(document["tracks"][0]["clips"][0]["fov"], serde_json::json!(50.0));
        assert_eq!(document["version"], serde_json::json!(1));
    }

    #[test]
    fn a_key_goes_in_time_order_and_keeps_the_other_fields() {
        let mut document = document(serde_json::json!({
            "duration": 4.0,
            "eye": { "keys": [ { "t": 0.0, "pos": [0.0, 0.0, 0.0] }, { "t": 3.0, "pos": [3.0, 0.0, 0.0] } ] },
            "aim": { "dir": [0.0, 0.0], "lag": 0.2 },
            "near": 0.5
        }));
        let wrote = capture(&mut document, &clip(0, 1.0), &aim()).expect("a keyed eye takes a key");
        assert!(!wrote.replaced);
        let times: Vec<f64> = keys(&document).iter().map(|key| key["t"].as_f64().unwrap()).collect();
        assert_eq!(times, vec![0.0, 1.0, 3.0]);
        assert_eq!(keys(&document)[1]["pos"], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(document["tracks"][0]["clips"][0]["aim"]["lag"], serde_json::json!(0.2));
        assert_eq!(document["tracks"][0]["clips"][0]["near"], serde_json::json!(0.5));
    }

    #[test]
    fn a_key_within_a_millisecond_is_replaced_and_keeps_its_ease() {
        let mut document = document(serde_json::json!({
            "duration": 4.0,
            "eye": { "keys": [ { "t": 0.0, "pos": [0.0, 0.0, 0.0] }, { "t": 2.0, "pos": [0.0, 0.0, 0.0], "ease": "smooth" } ] },
            "fov": { "keys": [ { "t": 2.0, "value": 40.0, "ease": "step" } ] },
            "aim": { "dir": [0.0, 0.0] }
        }));
        let wrote = capture(&mut document, &clip(0, 2.0005), &aim()).expect("a nearby key is replaced");
        assert!(wrote.replaced);
        assert_eq!(wrote.fov, "replaced");
        assert_eq!(keys(&document).len(), 2);
        assert_eq!(keys(&document)[1]["pos"], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(keys(&document)[1]["ease"], serde_json::json!("smooth"));
        assert_eq!(document["tracks"][0]["clips"][0]["fov"]["keys"][0]["value"], serde_json::json!(50.0));
        assert_eq!(document["tracks"][0]["clips"][0]["fov"]["keys"][0]["ease"], serde_json::json!("step"));
    }

    #[test]
    fn an_orbit_eye_is_refused_by_its_shape() {
        let shapes = [
            ("orbit", serde_json::json!({ "orbit": { "around": [0.0, 0.0, 0.0], "radius": 2.0, "from": 0.0, "to": 90.0 } })),
            ("path", serde_json::json!({ "path": { "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] } })),
            ("follow", serde_json::json!({ "follow": { "actor": "a", "offset": [0.0, 0.0, 1.0] } })),
            ("at", serde_json::json!({ "at": [0.0, 0.0, 0.0] })),
        ];
        for (shape, eye) in shapes {
            let mut document = document(serde_json::json!({ "duration": 4.0, "eye": eye }));
            let refused = capture(&mut document, &clip(0, 1.0), &aim()).expect_err("another eye shape is refused");
            assert!(refused.contains(&format!("the eye is a \"{shape}\"")), "{refused}");
        }
    }

    #[test]
    fn a_look_at_a_target_is_left_as_it_is() {
        let mut document = document(serde_json::json!({ "duration": 4.0, "aim": { "at": { "actor": "a", "part": "head" } } }));
        let wrote = capture(&mut document, &clip(0, 1.0), &aim()).expect("the look at a target is kept");
        assert_eq!(wrote.aim, "at");
        assert_eq!(wrote.angles, None);
        assert_eq!(document["tracks"][0]["clips"][0]["aim"], serde_json::json!({ "at": { "actor": "a", "part": "head" } }));
    }

    #[test]
    fn a_keyed_field_of_view_takes_a_key_and_a_fixed_one_takes_the_value() {
        let mut keyed = document(serde_json::json!({ "duration": 4.0, "fov": { "keys": [ { "t": 0.0, "value": 30.0 } ] } }));
        let wrote = capture(&mut keyed, &clip(0, 2.0), &aim()).expect("a keyed field of view takes a key");
        assert_eq!(wrote.fov, "added");
        let values = keyed["tracks"][0]["clips"][0]["fov"]["keys"].as_array().expect("keys");
        assert_eq!(values.len(), 2);
        assert_eq!(values[1], serde_json::json!({ "t": 2.0, "value": 50.0 }));
        let mut fixed = document(serde_json::json!({ "duration": 4.0, "fov": 20.0 }));
        let wrote = capture(&mut fixed, &clip(0, 2.0), &aim()).expect("a fixed field of view takes the value");
        assert_eq!(wrote.fov, "fixed");
        assert_eq!(fixed["tracks"][0]["clips"][0]["fov"], serde_json::json!(50.0));
    }

    #[test]
    fn the_written_angles_look_where_the_camera_looked() {
        let eye = Vec3::new(-1.5, 1.7, 2.25);
        let target = Vec3::new(3.0, 0.8, -4.0);
        let (yaw, pitch) = angles(eye, target);
        let pose = moly_sequence::CameraPose::aiming(eye, yaw, pitch, 45.0);
        assert!(view_error(pose, yaw, pitch) <= 1e-4);
        let view = (target - eye).normalize();
        assert!((pose.rotation * Vec3::NEG_Z).distance(view) <= 1e-4);
    }

    #[test]
    fn a_compiled_document_samples_the_keys_it_is_written_with() {
        let text = r#"{
            "version": 1,
            "tracks": [{ "kind": "camera", "clips": [{ "start": 0, "duration": 4, "eye": { "keys": [{ "t": 0, "pos": [0, 0, 0] }] }, "aim": { "dir": [0, 0] } }] }]
        }"#;
        let mut document: Value = serde_json::from_str(text).expect("the document parses");
        let compiled = moly_sequence::compile(&moly_sequence::parse(text).expect("the document reads"), &NoCatalog).expect("the document compiles");
        let found = compiled.camera_clip(1.0).expect("the clip holds 1 s");
        let aim = Aim { eye: Vec3::new(1.0, 1.5, -2.0), target: Vec3::new(4.0, 0.0, 1.0), fov: 55.0 };
        let wrote = capture(&mut document, &found, &aim).expect("the clip takes a key");
        let edited = serde_json::to_string(&document).expect("the document writes");
        let compiled = moly_sequence::compile(&moly_sequence::parse(&edited).expect("the edited document reads"), &NoCatalog).expect("the edited document compiles");
        let pose = compiled.shot(found.index, wrote.local, &KeysHost::default()).expect("the clip places the camera");
        assert!(pose.eye.distance(aim.eye) <= 1e-5);
        let [yaw, pitch] = wrote.angles.expect("the look was written");
        assert!(view_error(pose, yaw, pitch) <= 1e-4);
        assert!((pose.fov - aim.fov).abs() <= 1e-5);
    }

    /// The host a keyed eye needs: no actor is read, and the live camera
    /// only fills what the clip leaves unset.
    #[derive(Default)]
    struct KeysHost;

    impl moly_sequence::Host for KeysHost {
        fn live_camera(&self) -> moly_sequence::CameraPose {
            moly_sequence::CameraPose::aiming(Vec3::ZERO, 0.0, 0.0, 30.0)
        }

        fn part_height(&self, _unit: &str, _part: moly_sequence::doc::Part) -> f32 {
            0.0
        }

        fn rig(&self, _name: &str, _params: &Value, _local: f32) -> Option<moly_sequence::CameraPose> {
            None
        }
    }
}
