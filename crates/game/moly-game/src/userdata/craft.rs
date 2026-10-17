//! The craft screen's side of the craft loop: the masters it reads, the
//! client's own checks of a craft (`MysekaiUserDataUtility`), and the two
//! requests it sends.
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
//! [`post_craft`] and [`post_sketch`] run the endpoints the server model
//! installs; by the time they return, the reply's tables are already in
//! [`ClientMysekaiInventory`](super::inventory::ClientMysekaiInventory)
//! (the rank and total experience reach the server module's `ClientUserData`
//! with the next response delivery, as every response does). Without an
//! endpoint a request is refused by name.
//!
//! The checks below are the client's; the server model applies the same
//! checks to refuse a request the craft screen would not have sent.

// The craft, canvas and sketch screens read these.
#![allow(dead_code)]

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

use super::birthday_party::ClientBirthdayPartyData;
use super::inventory::ClientMysekaiInventory;
use super::tools::ClientMysekaiTools;

pub(crate) use moly_masters::craft::{
    is_create_count_limit, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft, CraftMasters, CraftOwned, MasterBlueprint,
    CANVAS_MATERIAL_ID, CRAFT_COUNT_MAX, CRAFT_COUNT_MIN,
};
pub(crate) use moly_protocol::api::MysekaiCraftType;

// ---------------------------------------------------------------------------
// Requests and replies
// ---------------------------------------------------------------------------

pub(crate) use moly_protocol::api::{
    UserMysekaiCraftRequest, UserMysekaiHousingSketchRequest, SuiteUserReply,
};

/// The server model's endpoints for the two requests.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct CraftEndpoints {
    pub(crate) craft: SystemId<In<UserMysekaiCraftRequest>, SuiteUserReply>,
    pub(crate) sketch: SystemId<In<UserMysekaiHousingSketchRequest>, SuiteUserReply>,
}

/// `PostUserMysekaiCraftApi`: send a craft or canvas request and read its
/// reply.
#[allow(dead_code)] // Called by the craft and canvas screens.
pub(crate) fn post_craft(world: &mut World, request: UserMysekaiCraftRequest) -> SuiteUserReply {
    let Some(endpoints) = world.get_resource::<CraftEndpoints>().copied() else {
        return refused(
            "PostUserMysekaiCraftApi",
            "no server endpoint is installed".into(),
        );
    };
    world
        .run_system_with(endpoints.craft, request)
        .unwrap_or_else(|error| refused("PostUserMysekaiCraftApi", error.to_string()))
}

/// `PostUserMysekaiHousingSketchApi`: send a sketch request and read its
/// reply.
#[allow(dead_code)] // Called by the sketch screen.
pub(crate) fn post_sketch(
    world: &mut World,
    request: UserMysekaiHousingSketchRequest,
) -> SuiteUserReply {
    let Some(endpoints) = world.get_resource::<CraftEndpoints>().copied() else {
        return refused(
            "PostUserMysekaiHousingSketchApi",
            "no server endpoint is installed".into(),
        );
    };
    world
        .run_system_with(endpoints.sketch, request)
        .unwrap_or_else(|error| refused("PostUserMysekaiHousingSketchApi", error.to_string()))
}

fn refused(api: &str, reason: String) -> SuiteUserReply {
    warn!("[server] {api} refused: {reason}");
    SuiteUserReply::refused(format!("{api}: {reason}"))
}

/// One request: the server model answers it, and the reply's sections reach
/// the client's copies before the caller reads them (the API executor merges
/// `updatedResources` before it returns).
fn run(
    api: &str,
    inventory: &mut ClientMysekaiInventory,
    birthday: &mut ClientBirthdayPartyData,
    tools: &mut ClientMysekaiTools,
    answer: impl FnOnce(&mut crate::server::ServerModel) -> crate::server::craft::Answer,
) -> SuiteUserReply {
    let answer = crate::server::with_model(|model| {
        if !model.joined {
            return crate::server::craft::Answer {
                reply: SuiteUserReply::refused(format!(
                    "{api}: the server has not joined the client yet"
                )),
                delivery: None,
                tools: None,
            };
        }
        answer(model)
    })
    .unwrap_or_else(|| crate::server::craft::Answer {
        reply: SuiteUserReply::refused(format!("{api}: the server model is not installed")),
        delivery: None,
        tools: None,
    });
    if let Some(delivery) = answer.delivery {
        birthday.apply(delivery);
    }
    if let Some(rows) = answer.tools {
        tools.apply(rows);
    }
    inventory.apply(answer.reply.updated.clone());
    answer.reply
}

/// The craft endpoint.
pub(crate) fn handle_craft(
    In(request): In<UserMysekaiCraftRequest>,
    mut inventory: ResMut<ClientMysekaiInventory>,
    mut birthday: ResMut<ClientBirthdayPartyData>,
    mut tools: ResMut<ClientMysekaiTools>,
) -> SuiteUserReply {
    run(
        "PostUserMysekaiCraftApi",
        &mut inventory,
        &mut birthday,
        &mut tools,
        |model| model.craft_reply(&request),
    )
}

/// The sketch endpoint.
pub(crate) fn handle_sketch(
    In(request): In<UserMysekaiHousingSketchRequest>,
    mut inventory: ResMut<ClientMysekaiInventory>,
    mut birthday: ResMut<ClientBirthdayPartyData>,
    mut tools: ResMut<ClientMysekaiTools>,
) -> SuiteUserReply {
    run(
        "PostUserMysekaiHousingSketchApi",
        &mut inventory,
        &mut birthday,
        &mut tools,
        |model| model.sketch_reply(&request),
    )
}

/// The endpoints and the client copy.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ClientMysekaiInventory>();
    let craft = app.world_mut().register_system(handle_craft);
    let sketch = app.world_mut().register_system(handle_sketch);
    app.insert_resource(CraftEndpoints { craft, sketch });
    super::talk_read::install(app);
    app.add_systems(
        PreUpdate,
        super::inventory::mirror_materials.after(super::deliver),
    );
}