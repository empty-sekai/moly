//! The home actions' requests as the client sends them: craft and canvas
//! (`PostUserMysekaiCraftApi`) and sketch (`PostUserMysekaiHousingSketchApi`).
//! The client starts the world sequence once the reply succeeds.
//!
//! The request is one `Call::HomeAction` on the server link (see
//! [`crate::home_action`]).

pub(crate) use moly_protocol::api::HomeActionApi;
