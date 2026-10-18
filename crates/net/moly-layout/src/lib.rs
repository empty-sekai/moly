//! The layout operation vocabulary shared by the editor, the director, the
//! MCP proxy and the room service.
//!
//! An operation names an anchor and a rotation; a resolver turns the anchor
//! into a grid cell and a direction, and a validator applies the source's
//! placement rules to the layout. A transaction is all-or-nothing and a dry
//! run changes nothing.
//!
//! The crate carries no engine dependency and no session state: the caller
//! owns the [`Layout`] and the [`Rules`], calls the validator and stores the
//! result it gets back. Editor code, the server's room service and the proxy
//! therefore run the same rules over the same data.

pub mod anchor;
pub mod apply;
pub mod layout;
pub mod op;
pub mod resolve;
pub mod result;
pub mod rules;
pub mod validate;

pub use anchor::{Anchor, Cell, Side, Wall};
pub use apply::apply;
pub use layout::{Layout, PlacedRow, SiteGrid};
pub use op::{LayoutOp, Layer, Piece, Rot};
pub use result::{
    Apply, ApplyOutcome, Applied, OpResult, Refusal, RefusalCode, Rejected,
};
pub use rules::{BundleMeta, FixtureMaster, MetaGrid, OwnedCount, Rules, Zone};
