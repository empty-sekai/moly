//! The avatar wear masters: the costume, accessory, skin-colour and
//! coordinate rows the client resolves `userAvatar`'s ids through
//! (`AvatarData.Build`).

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::Value;

pub use moly_protocol::rows::CoordinateRow;

/// The avatar wear masters (`avatar-costumes.json`, `avatar-accessories.json`,
/// `avatar-skin-colors.json`, `avatar-coordinates.json`). A map is `None`
/// when the runtime root lacks its master.
#[derive(Resource, Clone, Debug, Default)]
pub struct AvatarMasters {
    /// id -> `assetbundleName`.
    pub costumes: Option<BTreeMap<i32, String>>,
    /// id -> `assetbundleName`.
    pub accessories: Option<BTreeMap<i32, String>>,
    /// id -> `colorCode`.
    pub skin_colors: Option<BTreeMap<i32, String>>,
    pub coordinates: Option<BTreeMap<i32, CoordinateRow>>,
}

/// A table of an id and one string column (`assetbundleName`, `colorCode`).
fn string_rows(rows: &[Value], table: &str, column: &str) -> Result<BTreeMap<i32, String>, String> {
    rows.iter()
        .map(|row| {
            let id = crate::int32(row, "id")?;
            let text = row[column]
                .as_str()
                .ok_or_else(|| format!("{table} row {id} has no string {column}"))?;
            Ok((id, text.to_owned()))
        })
        .collect()
}

/// `avatarCostumes`.
pub fn parse_costumes(rows: &[Value]) -> Result<BTreeMap<i32, String>, String> {
    string_rows(rows, "avatarCostumes", "assetbundleName")
}

/// `avatarAccessories`.
pub fn parse_accessories(rows: &[Value]) -> Result<BTreeMap<i32, String>, String> {
    string_rows(rows, "avatarAccessories", "assetbundleName")
}

/// `avatarSkinColors`.
pub fn parse_skin_colors(rows: &[Value]) -> Result<BTreeMap<i32, String>, String> {
    string_rows(rows, "avatarSkinColors", "colorCode")
}

/// `avatarCoordinates`.
pub fn parse_coordinates(rows: &[Value]) -> Result<BTreeMap<i32, CoordinateRow>, String> {
    let text_of = |row: &Value, column: &str| row[column].as_str().map(str::to_owned);
    rows.iter()
        .map(|row| {
            Ok((
                crate::int32(row, "id")?,
                CoordinateRow {
                    costume_assetbundle_name: text_of(row, "costumeAssetbundleName"),
                    accessory_assetbundle_name: text_of(row, "accessoryAssetbundleName"),
                    skin_color_code: text_of(row, "skinColorCode"),
                },
            ))
        })
        .collect()
}
