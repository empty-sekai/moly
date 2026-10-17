//! The layout editor's save as the client sends it:
//! `PostUserMysekaiHousingLayoutApi` with a `UserMysekaiHousingEditLayoutRequest`
//! (the site and its whole layout). The editor writes its own saved copy
//! only once the reply succeeds; a refusal keeps the draft.
//!
//! The rows are in the served layout's record form (package, master fixture
//! id, footprint corners, centre height, layout type bits, rotation, texture),
//! the form the server document's `userMysekaiSiteHousingLayouts` rows use,
//! not the source's `MysekaiSiteHousingLayout` fields. Named gaps: the
//! request's `mysekaiFixtureSurfaceAppearances` (no surface editing here) and
//! `mysekaiPhenomenaId` are not sent.
//!
//! The request is one `Call::HousingLayout` on the server link (see
//! [`crate::fixture_edit`]).

pub(crate) use moly_protocol::api::{
    HousingLayoutCorner, HousingLayoutFixture, UserMysekaiHousingEditLayoutRequest,
};
