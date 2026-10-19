//! The craft screen's side of the craft loop: the craft masters it reads and
//! the client's own checks of a craft (`MysekaiUserDataUtility`).
//!
//! - `PostUserMysekaiCraftApi` (`user/{userId}/mysekai/craft`) carries
//!   [`UserMysekaiCraftRequest`]; the craft screen and the canvas screen both
//!   send it (`MysekaiCraftService.Execute`, which sends a quantity of 1 when
//!   the caller gives none).
//! - `PostUserMysekaiHousingSketchApi` (`user/{userId}/mysekai/housing/sketch`)
//!   carries [`UserMysekaiHousingSketchRequest`].
//!
//! Both reply with `SuiteUser`: the tables the request changed, which the
//! client merges before it reads its copies (`UserDataManager.UpdateAll`).
//! The screens send the request on the server link and continue on
//! [`Answered`](crate::userdata::Answered); the reply's tables reach
//! [`ClientMysekaiInventory`](super::inventory::ClientMysekaiInventory) with
//! the arrived sections.
//!
//! The checks below are the client's; the server applies the same checks to
//! refuse a request the craft screen would not have sent.

// The craft, canvas and sketch screens read these.
#![allow(dead_code)]

pub use moly_masters::craft::{
    is_create_count_limit, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft, CraftMasters, CraftOwned, MasterBlueprint,
    CANVAS_MATERIAL_ID, CRAFT_COUNT_MAX, CRAFT_COUNT_MIN,
};
pub use moly_protocol::api::MysekaiCraftType;

pub use moly_protocol::api::UserMysekaiCraftRequest;
