//! The player's avatar wear (`userAvatar`, `UserAvatar`: four nullable ids,
//! `avatarCostumeId`, `avatarAccessoryId`, `avatarSkinColorId`,
//! `avatarCoordinateId`). The wear is user data, so it lives in the server
//! document and reaches the client as its copy
//! ([`super::client::avatar::ClientUserAvatar`]); the client resolves the ids
//! through the avatar masters (`AvatarData.Build`), which the server model
//! reads and hands to the client as [`AvatarMasters`].
//!
//! Named default: every id null (the source's unset wear: the "default" skin
//! bundle, white skin, no accessory). The four masters check the ids when
//! present; each absent one is a named missing master.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::client::avatar::{AvatarMasters, CoordinateRow, UserAvatar, FIELDS};
use super::document::object;

pub(crate) const SECTION: &str = "userAvatar";

/// The slot of a field name.
fn slot<'a>(avatar: &'a mut UserAvatar, field: &str) -> Option<&'a mut Option<i32>> {
    match field {
        "avatarCostumeId" => Some(&mut avatar.costume),
        "avatarAccessoryId" => Some(&mut avatar.accessory),
        "avatarSkinColorId" => Some(&mut avatar.skin_color),
        "avatarCoordinateId" => Some(&mut avatar.coordinate),
        _ => None,
    }
}

fn nullable_id(value: &Value, at: &str) -> Result<Option<i32>, String> {
    match value {
        Value::Null => Ok(None),
        _ => value
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .map(Some)
            .ok_or_else(|| format!("{at} is neither null nor an id")),
    }
}

pub(crate) fn parse_value(value: &Value) -> Result<UserAvatar, String> {
    let row = object(value, SECTION)?;
    super::document::only(row, &FIELDS, SECTION)?;
    let mut avatar = UserAvatar::default();
    for field in FIELDS {
        let at = format!("{SECTION}.{field}");
        let id = nullable_id(row.get(field).unwrap_or(&Value::Null), &at)?;
        *slot(&mut avatar, field).expect("a known field") = id;
    }
    Ok(avatar)
}

/// The section from a document (all null when absent).
pub(crate) fn parse(doc: &Map<String, Value>) -> Result<UserAvatar, String> {
    doc.get(SECTION)
        .map_or(Ok(UserAvatar::default()), parse_value)
}

/// A `server.edit` of the section (next response); `None` for another path.
pub(crate) fn edit_path(
    avatar: &mut UserAvatar,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [SECTION] => Some(parse_value(value).map(|parsed| *avatar = parsed)),
        [SECTION, field] => Some(match slot(avatar, field) {
            Some(slot) => nullable_id(value, path).map(|id| *slot = id),
            None => Err(format!("{path} is not a UserAvatar field")),
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

/// The ids the present masters do not hold.
pub(crate) fn check(masters: &AvatarMasters, avatar: &UserAvatar) -> Result<(), String> {
    let known = |map: &Option<BTreeMap<i32, String>>, id: Option<i32>| {
        id.is_none_or(|id| map.as_ref().is_none_or(|map| map.contains_key(&id)))
    };
    let coordinate_known = avatar.coordinate.is_none_or(|id| {
        masters
            .coordinates
            .as_ref()
            .is_none_or(|map| map.contains_key(&id))
    });
    for (field, ok) in [
        (FIELDS[0], known(&masters.costumes, avatar.costume)),
        (FIELDS[1], known(&masters.accessories, avatar.accessory)),
        (FIELDS[2], known(&masters.skin_colors, avatar.skin_color)),
        (FIELDS[3], coordinate_known),
    ] {
        if !ok {
            return Err(format!("{SECTION}.{field} names no master row"));
        }
    }
    Ok(())
}

/// The master row whose value equals `wanted` (the native instrument's
/// reverse lookup from a bundle name or colour code).
fn id_of(map: &Option<BTreeMap<i32, String>>, wanted: &str) -> Option<i32> {
    map.as_ref()?
        .iter()
        .find(|(_, value)| value.as_str() == wanted)
        .map(|(id, _)| *id)
}

fn string_rows(text: &str, table: &str, column: &str) -> Result<BTreeMap<i32, String>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    super::master_rows(&value, table)?
        .iter()
        .map(|row| {
            let id = super::int32(row, "id")?;
            let text = row[column]
                .as_str()
                .ok_or_else(|| format!("{table} row {id} has no string {column}"))?;
            Ok((id, text.to_owned()))
        })
        .collect()
}

pub(crate) fn parse_costumes(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.costumes = Some(string_rows(text, "avatarCostumes", "assetbundleName")?);
    Ok(())
}

pub(crate) fn parse_accessories(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.accessories = Some(string_rows(text, "avatarAccessories", "assetbundleName")?);
    Ok(())
}

pub(crate) fn parse_skin_colors(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.skin_colors = Some(string_rows(text, "avatarSkinColors", "colorCode")?);
    Ok(())
}

pub(crate) fn parse_coordinates(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let text_of = |row: &Value, column: &str| row[column].as_str().map(str::to_owned);
    let rows = super::master_rows(&value, "avatarCoordinates")?
        .iter()
        .map(|row| {
            Ok((
                super::int32(row, "id")?,
                CoordinateRow {
                    costume_assetbundle_name: text_of(row, "costumeAssetbundleName"),
                    accessory_assetbundle_name: text_of(row, "accessoryAssetbundleName"),
                    skin_color_code: text_of(row, "skinColorCode"),
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    masters.avatar.coordinates = Some(rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// Native instrument
// ---------------------------------------------------------------------------

/// The native instrument `MOLY_AVATAR_MOCK_*`: bundle names and a colour
/// code, turned into ids once the masters resolve.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AvatarInstrument {
    pub(crate) coordinate: Option<String>,
    pub(crate) costume: Option<String>,
    pub(crate) accessory: Option<String>,
    pub(crate) skin_color: Option<String>,
}

impl AvatarInstrument {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The ids of the named rows; a value no master row holds is named.
    pub(crate) fn resolve(&self, masters: &AvatarMasters) -> (UserAvatar, Vec<String>) {
        let mut avatar = UserAvatar::default();
        let mut unresolved = Vec::new();
        let coordinates: Option<BTreeMap<i32, String>> = masters.coordinates.as_ref().map(|map| {
            map.iter()
                .filter_map(|(id, row)| {
                    row.costume_assetbundle_name.clone().map(|name| (*id, name))
                })
                .collect()
        });
        for (value, map, slot, name) in [
            (
                &self.coordinate,
                &coordinates,
                &mut avatar.coordinate,
                "MOLY_AVATAR_MOCK_COORDINATE",
            ),
            (
                &self.costume,
                &masters.costumes,
                &mut avatar.costume,
                "MOLY_AVATAR_MOCK_COSTUME",
            ),
            (
                &self.accessory,
                &masters.accessories,
                &mut avatar.accessory,
                "MOLY_AVATAR_MOCK_ACCESSORY",
            ),
            (
                &self.skin_color,
                &masters.skin_colors,
                &mut avatar.skin_color,
                "MOLY_AVATAR_MOCK_SKIN_COLOR",
            ),
        ] {
            if let Some(value) = value {
                match id_of(map, value) {
                    Some(id) => *slot = Some(id),
                    None => unresolved.push(format!("{name}={value:?} names no master row")),
                }
            }
        }
        (avatar, unresolved)
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": SECTION,
        "title": "Avatar wear",
        "delivery": "next-response",
        "fields": FIELDS.iter().map(|field| json!({
            "path": format!("{SECTION}.{field}"),
            "type": "int",
            "nullable": true,
            "note": "a master row id or null; the client resolves it through the avatar masters",
        })).collect::<Vec<_>>(),
    }])
}

#[cfg(test)]
mod tests {
    use super::super::client::avatar::value;
    use super::*;

    fn masters() -> AvatarMasters {
        AvatarMasters {
            costumes: Some(BTreeMap::from([
                (1, "default".to_owned()),
                (2, "costume_0002".to_owned()),
            ])),
            accessories: Some(BTreeMap::from([(1, "accessory_0001".to_owned())])),
            skin_colors: Some(BTreeMap::from([(3, "#f1f1f1".to_owned())])),
            coordinates: Some(BTreeMap::from([(
                9,
                CoordinateRow {
                    costume_assetbundle_name: Some("fullset_0001".to_owned()),
                    accessory_assetbundle_name: Some("fullset_0001".to_owned()),
                    skin_color_code: Some("#ffffff".to_owned()),
                },
            )])),
        }
    }

    #[test]
    fn the_section_round_trips_and_edits_by_field() {
        let avatar = UserAvatar {
            costume: Some(2),
            accessory: None,
            skin_color: Some(3),
            coordinate: None,
        };
        assert_eq!(parse_value(&value(&avatar)).unwrap(), avatar);
        assert_eq!(parse(&Map::new()).unwrap(), UserAvatar::default());
        let mut edited = avatar;
        edit_path(
            &mut edited,
            &[SECTION, "avatarCoordinateId"],
            "userAvatar.avatarCoordinateId",
            &json!(9),
        )
        .unwrap()
        .unwrap();
        assert_eq!(edited.coordinate, Some(9));
        assert!(
            edit_path(&mut edited, &[SECTION, "hat"], "userAvatar.hat", &json!(1))
                .unwrap()
                .is_err()
        );
        assert!(check(&masters(), &edited).is_ok());
        edited.accessory = Some(7);
        assert!(check(&masters(), &edited)
            .unwrap_err()
            .contains("avatarAccessoryId"));
        assert!(check(&AvatarMasters::default(), &edited).is_ok());
    }

    #[test]
    fn the_instrument_resolves_names_to_ids() {
        let instrument = AvatarInstrument {
            coordinate: Some("fullset_0001".to_owned()),
            costume: None,
            accessory: Some("accessory_0001".to_owned()),
            skin_color: Some("#000000".to_owned()),
        };
        let (avatar, unresolved) = instrument.resolve(&masters());
        assert_eq!(avatar.coordinate, Some(9));
        assert_eq!(avatar.accessory, Some(1));
        assert_eq!(avatar.skin_color, None);
        assert_eq!(unresolved.len(), 1);
    }
}
