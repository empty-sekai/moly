//! The player's avatar wear now lives in [`crate::avatar_wear_data`], which
//! the world's avatar dressing reads; this path keeps the response writer's
//! own spelling.

pub(crate) use crate::avatar_wear_data::{value, AvatarMasters, ClientUserAvatar, UserAvatar};
