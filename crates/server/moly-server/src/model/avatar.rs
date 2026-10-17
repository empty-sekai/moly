//! The player's avatar wear (`userAvatar`, `UserAvatar`: four nullable ids,
//! `avatarCostumeId`, `avatarAccessoryId`, `avatarSkinColorId`,
//! `avatarCoordinateId`). The wear is user data, so it lives in the server
//! document and reaches the client as its copy (`ClientUserAvatar`); the
//! client resolves the ids through the avatar masters (`AvatarData.Build`),
//! which the server model reads and hands to the client as
//! `AvatarMasters`.
//!
//! Named default: every id null (the source's unset wear: the "default" skin
//! bundle, white skin, no accessory). The four masters check the ids when
//! present; each absent one is a named missing master.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use moly_masters::avatar::AvatarMasters;
pub(crate) use moly_protocol::json::SECTION_AVATAR as SECTION;
use moly_protocol::json::{parse_avatar, AVATAR_FIELDS as FIELDS};
use moly_protocol::rows::UserAvatar;

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

/// The section from a document (all null when absent).
pub(crate) fn parse(doc: &Map<String, Value>) -> Result<UserAvatar, String> {
    doc.get(SECTION)
        .map_or(Ok(UserAvatar::default()), parse_avatar)
}

/// A `server.edit` of the section (next response); `None` for another path.
pub(crate) fn edit_path(
    avatar: &mut UserAvatar,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [SECTION] => Some(parse_avatar(value).map(|parsed| *avatar = parsed)),
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

fn master_rows(text: &str, table: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    super::master_rows(&value, table)
}

pub(crate) fn parse_costumes(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.costumes = Some(moly_masters::avatar::parse_costumes(&master_rows(
        text,
        "avatarCostumes",
    )?)?);
    Ok(())
}

pub(crate) fn parse_accessories(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.accessories = Some(moly_masters::avatar::parse_accessories(&master_rows(
        text,
        "avatarAccessories",
    )?)?);
    Ok(())
}

pub(crate) fn parse_skin_colors(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.skin_colors = Some(moly_masters::avatar::parse_skin_colors(&master_rows(
        text,
        "avatarSkinColors",
    )?)?);
    Ok(())
}

pub(crate) fn parse_coordinates(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    masters.avatar.coordinates = Some(moly_masters::avatar::parse_coordinates(&master_rows(
        text,
        "avatarCoordinates",
    )?)?);
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
