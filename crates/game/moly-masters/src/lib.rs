//! The typed master tables the game and its server both read.
//!
//! A master table is one upstream JSON array of rows. The rows' shape, the
//! parse from the rows, the pure rules over them and the loader that requests
//! them live here, so the two sides cannot disagree about a column name, a
//! rule or which asset holds what. [`load::MasterTables`] is the one request
//! for the five resources both sides read.

use bevy::prelude::Resource;
use serde_json::Value;

pub mod avatar;
pub mod clock;
pub mod craft;
pub mod delivery;
pub mod inventory;
pub mod load;
pub mod recovery;
pub mod tools;

/// The master pools' maxima (`mysekaiStaminas.maxStamina` per type).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaminaMax {
    pub normal: i32,
    pub enhance: i32,
    pub boost: i32,
}

pub use recovery::{
    boost_item_value, conversion_slider_max, converted_boost, item_select_max, BoostItemRow,
    ItemRoom, RecoveryRow,
};

/// An integer field of a master row.
pub(crate) fn int32(row: &Value, key: &str) -> Result<i32, String> {
    row[key]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("{key} of {row} is not an int"))
}

/// An integer field of a master row, 0 when the row has none (or `null`).
pub(crate) fn optional_int32(row: &Value, key: &str) -> Result<i32, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(_) => int32(row, key),
    }
}

/// A boolean field of a master row.
pub(crate) fn flag(row: &Value, key: &str) -> Result<bool, String> {
    row[key]
        .as_bool()
        .ok_or_else(|| format!("{key} of {row} is not a boolean"))
}
