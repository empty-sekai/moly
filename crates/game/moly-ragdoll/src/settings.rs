//! The knobs the ragdoll is tuned with while it plays, and the range of each.
//!
//! Speeds and distances are read per metre of the character's own body height:
//! a character may stand in the world at any uniform scale (a giant ten to
//! thirty times the usual size is the usual case), so a fixed metre figure
//! would make a small character slow to settle and a large one settle before it
//! had touched the ground. Every speed here is multiplied by the body's height
//! before it is compared with a speed the physics gives, and every push the
//! methods take is read the same way.
//!
//! The joint compliance is the inverse of stiffness in the solver's units
//! (metres per newton, and newton-metres per radian). It is left unscaled on
//! purpose: the joint solver already weighs a limit by the inertia it resists,
//! so a compliant joint gives the same look on a small character as on a giant.

use bevy::prelude::*;
use serde_json::{json, Map, Value};

/// A single precision value as the decimal it was taken from: a quarter
/// reads as 0.25 rather than as the longer number nearest it is held as.
fn shorter(value: f32) -> f64 {
    f64::from(format!("{value}").parse::<f32>().unwrap_or(value))
}

/// One tunable value: its range and what it changes.
pub struct Knob {
    /// The key the methods take.
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    /// The value a ragdoll starts with.
    pub default: f32,
    pub about: &'static str,
}

impl Knob {
    /// Whether `value` is a value of this knob.
    pub fn admits(&self, value: f32) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }

    fn get(&self, settings: &RagdollSettings) -> f32 {
        match self.key {
            "gravity_scale" => settings.gravity_scale,
            "linear_damping" => settings.linear_damping,
            "angular_damping" => settings.angular_damping,
            "joint_compliance" => settings.joint_compliance,
            "mass_scale" => settings.mass_scale,
            "settle_speed" => settings.settle_speed,
            "settle_seconds" => settings.settle_seconds,
            "max_seconds" => settings.max_seconds,
            "recover_seconds" => settings.recover_seconds,
            _ => unreachable!("every knob reads a field of its own"),
        }
    }

    fn set(&self, settings: &mut RagdollSettings, value: f32) {
        match self.key {
            "gravity_scale" => settings.gravity_scale = value,
            "linear_damping" => settings.linear_damping = value,
            "angular_damping" => settings.angular_damping = value,
            "joint_compliance" => settings.joint_compliance = value,
            "mass_scale" => settings.mass_scale = value,
            "settle_speed" => settings.settle_speed = value,
            "settle_seconds" => settings.settle_seconds = value,
            "max_seconds" => settings.max_seconds = value,
            "recover_seconds" => settings.recover_seconds = value,
            _ => unreachable!("every knob writes a field of its own"),
        }
    }
}

/// Every knob, in the order the answers list them.
pub const KNOBS: &[Knob] = &[
    Knob { key: "gravity_scale", min: 0.0, max: 5.0, default: 1.0, about: "How strongly gravity pulls the parts, as a multiple of the world's gravity." },
    Knob { key: "linear_damping", min: 0.0, max: 10.0, default: 0.05, about: "How quickly a part's speed bleeds away besides the contacts." },
    Knob { key: "angular_damping", min: 0.0, max: 10.0, default: 0.5, about: "How quickly a part's spin bleeds away besides the contacts." },
    Knob { key: "joint_compliance", min: 0.0, max: 0.01, default: 0.0, about: "How soft the joint limits are: 0 holds them rigidly, larger values let them give." },
    Knob { key: "mass_scale", min: 0.1, max: 10.0, default: 1.0, about: "The weight of every part, as a multiple of its volume." },
    Knob { key: "settle_speed", min: 0.001, max: 2.0, default: 0.15, about: "The speed, in metres per second per metre of body height, below which every part counts as at rest." },
    Knob { key: "settle_seconds", min: 0.0, max: 10.0, default: 0.5, about: "How long every part has to stay at rest before the character gets up." },
    Knob { key: "max_seconds", min: 0.1, max: 60.0, default: 8.0, about: "The longest a character lies down before it gets up regardless." },
    Knob { key: "recover_seconds", min: 0.0, max: 5.0, default: 0.35, about: "How long the pose blends from where the parts fell back to the standing animation." },
];

/// The ragdoll's tuning: every part of the body reads it when it falls, and
/// the methods change it while bodies are in the air.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct RagdollSettings {
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub joint_compliance: f32,
    pub mass_scale: f32,
    pub settle_speed: f32,
    pub settle_seconds: f32,
    pub max_seconds: f32,
    pub recover_seconds: f32,
}

impl Default for RagdollSettings {
    fn default() -> Self {
        let mut settings = Self {
            gravity_scale: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            joint_compliance: 0.0,
            mass_scale: 0.0,
            settle_speed: 0.0,
            settle_seconds: 0.0,
            max_seconds: 0.0,
            recover_seconds: 0.0,
        };
        for knob in KNOBS {
            knob.set(&mut settings, knob.default);
        }
        settings
    }
}

impl RagdollSettings {
    /// The settings as the answers give them: every value as the shortest
    /// number its single precision field holds, so that the value answered
    /// reads back as the value set.
    pub fn to_json(&self) -> Value {
        let mut answer = Map::new();
        for knob in KNOBS {
            answer.insert(knob.key.to_owned(), json!(shorter(knob.get(self))));
        }
        Value::Object(answer)
    }

    /// The settings as JSON Schema: each key, its range and what it changes.
    pub fn schema() -> Value {
        let properties: Map<String, Value> = KNOBS
            .iter()
            .map(|knob| {
                (
                    knob.key.to_owned(),
                    json!({
                        "type": "number",
                        "minimum": knob.min,
                        "maximum": knob.max,
                        "default": knob.default,
                        "description": knob.about,
                    }),
                )
            })
            .collect();
        let mut schema = Map::new();
        schema.insert("type".to_owned(), json!("object"));
        schema.insert("properties".to_owned(), Value::Object(properties));
        schema.insert("additionalProperties".to_owned(), json!(false));
        Value::Object(schema)
    }

    /// Applies a partial answer: every key must be a knob and carry a value in
    /// its range. An answer with a key that is not a knob, a value that is not
    /// a number, or a number out of range is refused whole, naming the key,
    /// and the settings are left as they were.
    pub fn apply(&mut self, answer: &Map<String, Value>) -> Result<(), String> {
        let mut next = *self;
        for (key, value) in answer {
            let Some(knob) = KNOBS.iter().find(|knob| knob.key == key) else {
                let keys: Vec<&str> = KNOBS.iter().map(|knob| knob.key).collect();
                return Err(format!("\"{key}\" is not a setting; the settings are {}", keys.join(", ")));
            };
            let Some(number) = value.as_f64() else {
                return Err(format!("{key} must be a number from {} to {}", knob.min, knob.max));
            };
            let number = number as f32;
            if !knob.admits(number) {
                return Err(format!("{key} must be a number from {} to {}", knob.min, knob.max));
            }
            knob.set(&mut next, number);
        }
        *self = next;
        Ok(())
    }

    /// The settle speed in metres per second for a body `height` metres tall.
    pub fn rest_speed(&self, height: f32) -> f32 {
        self.settle_speed * height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(settings: &mut RagdollSettings, answer: Value) -> Result<(), String> {
        match answer.as_object() {
            Some(answer) => settings.apply(answer),
            None => Err("the answer is not an object".to_owned()),
        }
    }

    #[test]
    fn defaults_sit_inside_their_ranges() {
        let settings = RagdollSettings::default();
        for knob in KNOBS {
            assert!(knob.admits(knob.get(&settings)), "{} starts at {}, out of range", knob.key, knob.get(&settings));
        }
    }

    #[test]
    fn a_partial_answer_changes_only_the_keys_it_names() {
        let mut settings = RagdollSettings::default();
        apply(&mut settings, json!({ "gravity_scale": 0.5, "settle_seconds": 2.0 })).unwrap();
        assert_eq!(settings.gravity_scale, 0.5);
        assert_eq!(settings.settle_seconds, 2.0);
        assert_eq!(settings.angular_damping, RagdollSettings::default().angular_damping);
    }

    #[test]
    fn an_out_of_range_value_is_refused_by_name_and_changes_nothing() {
        let mut settings = RagdollSettings::default();
        let error = apply(&mut settings, json!({ "gravity_scale": 0.5, "mass_scale": 0.0 })).unwrap_err();
        assert_eq!(error, "mass_scale must be a number from 0.1 to 10");
        assert_eq!(settings, RagdollSettings::default());
    }

    #[test]
    fn an_unknown_key_and_a_value_that_is_not_a_number_are_refused() {
        let mut settings = RagdollSettings::default();
        let unknown = apply(&mut settings, json!({ "gravity": 1.0 })).unwrap_err();
        assert!(unknown.starts_with("\"gravity\" is not a setting; the settings are gravity_scale"), "{unknown}");
        assert_eq!(apply(&mut settings, json!({ "gravity_scale": "high" })).unwrap_err(), "gravity_scale must be a number from 0 to 5");
        assert!(apply(&mut settings, json!({ "settle_speed": 1e-9 })).is_err());
        assert_eq!(settings, RagdollSettings::default());
    }

    #[test]
    fn an_answer_is_read_back_as_it_was_set() {
        let mut settings = RagdollSettings::default();
        // A value that is not held exactly is held as the nearest single
        // precision number, and answered as a number that reads back as that.
        let nearest = 0.1f64 as f32;
        apply(&mut settings, json!({ "linear_damping": 0.1 })).unwrap();
        assert_eq!(settings.linear_damping, nearest);
        let answered = settings.to_json()["linear_damping"].as_f64().unwrap();
        let mut read_back = RagdollSettings::default();
        apply(&mut read_back, json!({ "linear_damping": answered })).unwrap();
        assert_eq!(read_back.linear_damping, nearest);
        // A number held exactly is answered exactly.
        apply(&mut settings, json!({ "settle_seconds": 0.25 })).unwrap();
        assert_eq!(settings.to_json()["settle_seconds"], json!(0.25));
    }

    #[test]
    fn the_answer_lists_every_knob_with_its_value() {
        let settings = RagdollSettings::default();
        let answer = settings.to_json();
        let object = answer.as_object().unwrap();
        assert_eq!(object.len(), KNOBS.len());
        for knob in KNOBS {
            let answered = object.get(knob.key).unwrap_or_else(|| panic!("{} is not answered", knob.key));
            // The answer reads back as the value the knob holds, so that what
            // a caller was told is what the ragdoll falls with.
            let mut read_back = RagdollSettings::default();
            apply(&mut read_back, json!({ knob.key: answered })).unwrap_or_else(|reason| panic!("the answer for {} is refused: {reason}", knob.key));
            assert_eq!(knob.get(&read_back), knob.get(&settings), "{} answers {answered}", knob.key);
        }
    }

    #[test]
    fn a_giant_is_judged_at_rest_by_its_own_height() {
        let settings = RagdollSettings::default();
        // The same shape at thirty times the size is at rest at thirty times
        // the speed: a speed in metres means nothing without the height.
        assert!((settings.rest_speed(30.0) / settings.rest_speed(1.0) - 30.0).abs() < 1e-4);
    }
}
