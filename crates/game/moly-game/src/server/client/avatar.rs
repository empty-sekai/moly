//! The client's copy of the player's avatar wear (`UserDataManager.UserAvatar`,
//! `UserAvatar`: four nullable ids, `avatarCostumeId`, `avatarAccessoryId`,
//! `avatarSkinColorId`, `avatarCoordinateId`) and the avatar wear masters the
//! client resolves them through (`AvatarData.Build`).
//!
//! The server model's responses set [`ClientUserAvatar`]. The server model
//! reads the four masters and inserts [`AvatarMasters`] once every master has
//! resolved, so the resource's presence means "resolved": each map is `None`
//! when the runtime root lacks its master.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::{json, Value};

/// The section's field names, in the source's order.
pub(crate) const FIELDS: [&str; 4] = [
    "avatarCostumeId",
    "avatarAccessoryId",
    "avatarSkinColorId",
    "avatarCoordinateId",
];

/// `UserAvatar`. All null is the source's unset wear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct UserAvatar {
    pub(crate) costume: Option<i32>,
    pub(crate) accessory: Option<i32>,
    pub(crate) skin_color: Option<i32>,
    pub(crate) coordinate: Option<i32>,
}

impl UserAvatar {
    /// The ids by field name, in [`FIELDS`] order.
    pub(crate) fn values(&self) -> [(&'static str, Option<i32>); 4] {
        [
            (FIELDS[0], self.costume),
            (FIELDS[1], self.accessory),
            (FIELDS[2], self.skin_color),
            (FIELDS[3], self.coordinate),
        ]
    }
}

/// The wear as the response key's JSON object.
pub(crate) fn value(avatar: &UserAvatar) -> Value {
    Value::Object(
        avatar
            .values()
            .into_iter()
            .map(|(field, id)| (field.to_owned(), json!(id)))
            .collect(),
    )
}

/// `MasterAvatarCoordinate`, the columns the player chain reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoordinateRow {
    pub(crate) costume_assetbundle_name: Option<String>,
    pub(crate) accessory_assetbundle_name: Option<String>,
    pub(crate) skin_color_code: Option<String>,
}

/// The avatar wear masters (`avatar-costumes.json`, `avatar-accessories.json`,
/// `avatar-skin-colors.json`, `avatar-coordinates.json`). A map is `None`
/// when the runtime root lacks its master (the server model names it).
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct AvatarMasters {
    /// id -> `assetbundleName`.
    pub(crate) costumes: Option<BTreeMap<i32, String>>,
    /// id -> `assetbundleName`.
    pub(crate) accessories: Option<BTreeMap<i32, String>>,
    /// id -> `colorCode`.
    pub(crate) skin_colors: Option<BTreeMap<i32, String>>,
    pub(crate) coordinates: Option<BTreeMap<i32, CoordinateRow>>,
}

/// The client's copy, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientUserAvatar {
    pub(crate) avatar: UserAvatar,
    /// Responses that carried the section (0: none yet).
    pub(crate) revision: u64,
}

impl ClientUserAvatar {
    pub(crate) fn apply(&mut self, avatar: UserAvatar) {
        self.avatar = avatar;
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wear_writes_every_field_and_nulls() {
        let avatar = UserAvatar {
            costume: Some(2),
            accessory: None,
            skin_color: Some(3),
            coordinate: None,
        };
        let text = value(&avatar);
        assert_eq!(text["avatarCostumeId"], 2);
        assert!(text["avatarAccessoryId"].is_null());
        assert_eq!(text.as_object().unwrap().len(), 4);
        let mut client = ClientUserAvatar::default();
        client.apply(avatar);
        assert_eq!((client.avatar, client.revision), (avatar, 1));
    }
}
