//! The ragdoll's methods of the control surface: `moly.ragdoll.drop`,
//! `moly.ragdoll.recover`, `moly.ragdoll.status`, `moly.ragdoll.settings` and
//! `moly.ragdoll.schema`.
//!
//! The methods join the protocol's table at `Startup`, once the control
//! surface has built it. A method that changes a character runs on the world
//! at once, in the caller's frame, and answers what it did or a refusal by
//! name; `status`, `settings` and `schema` answer at once. They are registered
//! only where the protocol table exists.

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use crate::description::Role;
use crate::drop::Drop;
use crate::RagdollSettings;
use moly_features::npc::CharacterUnitId;

/// The methods, by name.
const METHODS: &[&str] = &["moly.ragdoll.schema", "moly.ragdoll.drop", "moly.ragdoll.recover", "moly.ragdoll.status", "moly.ragdoll.settings"];

/// The keys the settings method takes, in the order the answers list them.
const SETTINGS_KEYS: &[&str] = &["gravity_scale", "linear_damping", "angular_damping", "joint_compliance", "mass_scale", "settle_speed", "settle_seconds", "max_seconds", "recover_seconds"];

#[cfg(not(target_arch = "wasm32"))]
use bevy_remote::{error_codes, BrpError, BrpResult, RemoteMethodSystemId};

/// Registers the methods on a native host's control surface.
pub fn install(_app: &mut App) {
    #[cfg(not(target_arch = "wasm32"))]
    _app.add_systems(Startup, register);
}

#[cfg(not(target_arch = "wasm32"))]
fn register(world: &mut World) {
    if !world.contains_resource::<bevy_remote::RemoteMethods>() {
        return;
    }
    let drop = world.register_system(native::drop_method);
    let recover = world.register_system(native::recover_method);
    let status = world.register_system(native::status);
    let settings = world.register_system(native::settings);
    let schema = world.register_system(native::schema);
    let mut methods = world.resource_mut::<bevy_remote::RemoteMethods>();
    methods.insert("moly.ragdoll.drop", RemoteMethodSystemId::Instant(drop));
    methods.insert("moly.ragdoll.recover", RemoteMethodSystemId::Instant(recover));
    methods.insert("moly.ragdoll.status", RemoteMethodSystemId::Instant(status));
    methods.insert("moly.ragdoll.settings", RemoteMethodSystemId::Instant(settings));
    methods.insert("moly.ragdoll.schema", RemoteMethodSystemId::Instant(schema));
}

/// The params as an object; none reads as an empty one. A key the method does
/// not take is refused rather than ignored.
fn fields(params: Option<Value>, method: &str, takes: &[&str]) -> Result<Map<String, Value>, String> {
    let params = match params {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(format!("{method}: params must be an object")),
    };
    if let Some(key) = params.keys().find(|key| !takes.contains(&key.as_str())) {
        let takes = if takes.is_empty() { "no parameters".to_owned() } else { takes.join(", ") };
        return Err(format!("{method}: unknown parameter \"{key}\"; it takes {takes}"));
    }
    Ok(params)
}

/// The unit a method names.
fn unit(params: &Map<String, Value>, method: &str) -> Result<u32, String> {
    params
        .get("unit")
        .and_then(Value::as_u64)
        .and_then(|unit| u32::try_from(unit).ok())
        .ok_or_else(|| format!("{method}: unit must be a whole number of at least 0"))
}

/// A push, as three finite numbers.
fn impulse(params: &Map<String, Value>, method: &str) -> Result<Option<Vec3>, String> {
    let Some(value) = params.get("impulse") else {
        return Ok(None);
    };
    let Some(numbers) = value.as_array().filter(|numbers| numbers.len() == 3) else {
        return Err(format!("{method}: impulse must be three numbers, [x, y, z]"));
    };
    let mut push = Vec3::ZERO;
    for (axis, number) in numbers.iter().enumerate() {
        let Some(number) = number.as_f64().filter(|number| number.is_finite()) else {
            return Err(format!("{method}: impulse must be three numbers, [x, y, z]"));
        };
        push[axis] = number as f32;
    }
    Ok(Some(push))
}

/// The bone a part follows, as its bone's name.
fn bone_name(world: &World, entity: Entity) -> Value {
    world.get::<Name>(entity).map_or(Value::Null, |name| json!(name.as_str()))
}

/// The status of every character that is falling. The world is lent mutable
/// because a query's first walk readies it.
pub fn status_of(world: &mut World) -> Value {
    let falling: Vec<Entity> = world.query_filtered::<Entity, (With<CharacterUnitId>, With<Drop>)>().iter(world).collect();
    let mut list = Vec::new();
    for member in falling {
        let unit_number = world.get::<CharacterUnitId>(member).map_or(0, |unit| unit.0);
        let Some(drop) = world.get::<Drop>(member) else {
            continue;
        };
        let phase = if drop.getting_up.is_some() { "recovering" } else { "simulating" };
        let recovered = drop.getting_up.as_ref().map_or(0.0, |getting| world.get_resource::<Time>().map_or(0.0, |time| time.elapsed_secs()) - getting.started);
        let pelvis = drop.of_role(Role::Pelvis).map(|part| part.frame(world).map_or(part.position, |(position, _)| position));
        let owner = world.get::<moly_features::character::MotionDriver>(member).and_then(|driver| driver.held_by());
        let parts: Vec<Value> = drop
            .all_parts()
            .enumerate()
            .map(|(index, part)| {
                let (position, rotation) = part.frame(world).unwrap_or((part.position, part.rotation));
                json!({ "role": part.role.word(), "bone": bone_name(world, part.bone), "position": position.to_array(), "rotation": rotation.to_array(), "speed": drop.seen.speed(index) })
            })
            .collect();
        list.push(json!({
            "unit": unit_number,
            "entity": format!("{member:?}"),
            "phase": phase,
            "elapsed": drop.how_long,
            "recovering_for": recovered,
            "height": drop.height,
            "floor": drop.floor,
            "at": drop.at,
            "pelvis": pelvis.map_or(Value::Null, |position| json!(position.to_array())),
            "fastest": drop.seen.speed(0).map(|_| drop.fastest()),
            "rest_speed": world.get_resource::<crate::RagdollSettings>().map_or(0.0, |settings| settings.rest_speed(drop.height)),
            "body_owner": owner.map_or("no state".to_owned(), |owner| owner.name().to_owned()),
            "parts": parts.len(),
            "joints": drop.joints.len(),
            "poses": parts,
        }));
    }
    list.sort_by_key(|entry| entry["unit"].as_u64());
    json!({ "ragdolls": list })
}

/// The params each method takes, as JSON Schema.
fn params_schema() -> Value {
    let unit = json!({ "type": "integer", "minimum": 0, "description": "The character's unit number." });
    json!({
        "moly.ragdoll.drop": {
            "type": "object",
            "properties": {
                "unit": unit,
                "impulse": {
                    "type": "array",
                    "items": { "type": "number" },
                    "minItems": 3,
                    "maxItems": 3,
                    "description": "A push in metres per second per metre of the character's height, applied at the named bone."
                },
                "at": { "type": "string", "description": "The bone the push lands on; the pelvis when absent." }
            },
            "required": ["unit"],
            "additionalProperties": false
        },
        "moly.ragdoll.recover": {
            "type": "object",
            "properties": { "unit": unit },
            "required": ["unit"],
            "additionalProperties": false
        },
        "moly.ragdoll.status": { "type": "object", "additionalProperties": false },
        "moly.ragdoll.settings": RagdollSettings::schema(),
        "moly.ragdoll.schema": { "type": "object", "additionalProperties": false }
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;

    fn invalid(message: impl Into<String>) -> BrpError {
        BrpError { code: error_codes::INVALID_PARAMS, message: message.into(), data: None }
    }

    fn refused(message: impl Into<String>) -> BrpError {
        BrpError { code: error_codes::RESOURCE_NOT_PRESENT, message: message.into(), data: None }
    }

    pub(super) fn drop_method(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
        let taken = super::fields(params, "moly.ragdoll.drop", &["unit", "impulse", "at"]).map_err(invalid)?;
        let unit = super::unit(&taken, "moly.ragdoll.drop").map_err(invalid)?;
        let push = super::impulse(&taken, "moly.ragdoll.drop").map_err(invalid)?;
        let at = taken
            .get("at")
            .map(|value| value.as_str().map(str::to_owned).ok_or_else(|| invalid("moly.ragdoll.drop: at must be a bone's name")))
            .transpose()?;
        let (reply, body) = crate::drop::drop_now(world, unit, push, at)
            .map(|answer| (answer.note, answer.body))
            .map_err(|reason| refused(format!("moly.ragdoll.drop: {reason}")))?;
        Ok(json!({ "unit": unit, "phase": "simulating", "reply": reply, "body": body, "status": super::status_of(world) }))
    }

    pub(super) fn recover_method(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
        let taken = super::fields(params, "moly.ragdoll.recover", &["unit"]).map_err(invalid)?;
        let unit = super::unit(&taken, "moly.ragdoll.recover").map_err(invalid)?;
        let reply = crate::drop::recover_now(world, unit)
            .map(|answer| answer.note)
            .map_err(|reason| refused(format!("moly.ragdoll.recover: {reason}")))?;
        Ok(json!({ "unit": unit, "phase": "recovering", "reply": reply, "status": super::status_of(world) }))
    }

    pub(super) fn status(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
        super::fields(params, "moly.ragdoll.status", &[]).map_err(invalid)?;
        Ok(super::status_of(world))
    }

    pub(super) fn settings(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
        let taken = super::fields(params, "moly.ragdoll.settings", super::SETTINGS_KEYS).map_err(invalid)?;
        let mut settings = *world.resource::<RagdollSettings>();
        settings.apply(&taken).map_err(|reason| invalid(format!("moly.ragdoll.settings: {reason}")))?;
        *world.resource_mut::<RagdollSettings>() = settings;
        Ok(settings.to_json())
    }

    pub(super) fn schema(In(params): In<Option<Value>>, _world: &mut World) -> BrpResult {
        super::fields(params, "moly.ragdoll.schema", &[]).map_err(invalid)?;
        Ok(json!({ "methods": super::METHODS, "params": super::params_schema() }))
    }
}
