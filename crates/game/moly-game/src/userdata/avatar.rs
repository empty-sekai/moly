//! The client's copy of the player's avatar wear (`UserDataManager.UserAvatar`,
//! `UserAvatar`: four nullable ids, `avatarCostumeId`, `avatarAccessoryId`,
//! `avatarSkinColorId`, `avatarCoordinateId`) and the avatar wear masters the
//! client resolves them through (`AvatarData.Build`).
//!
//! The server model's responses set [`ClientUserAvatar`]. The server model
//! reads the four masters and inserts [`AvatarMasters`] once every master has
//! resolved, so the resource's presence means "resolved": each map is `None`
//! when the runtime root lacks its master.

use bevy::prelude::*;

pub(crate) use moly_masters::avatar::AvatarMasters;
pub(crate) use moly_protocol::json::avatar_value as value;
pub(crate) use moly_protocol::rows::UserAvatar;

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
