//! The transaction envelope and its results.

use moly_law::fixture::GridPosition;

use crate::anchor::Cell;
use crate::op::{LayoutOp, Piece, Rot};

/// Why a transaction was refused. The set is closed: a new rule adds a code
/// here, and every consumer still has to name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefusalCode {
    /// `out_of_bounds`: a cell of the footprint leaves the site grid.
    OutOfBounds,
    /// `overlap`: another piece occupies a cell of the footprint.
    Overlap,
    /// `wrong_layer`: the anchor's layer does not match the piece's layout type.
    WrongLayer,
    /// `not_owned`: the player's owned quantity does not cover the piece.
    NotOwned,
    /// `unavailable_zone`: the site layout's unavailable zone covers a cell.
    UnavailableZone,
    /// `stack_not_allowed`: the stacking rule does not allow this base, cell or height.
    StackNotAllowed,
    /// `blocks_path`: the optional walkability check found the piece blocking a path.
    BlocksPath,
    /// `unknown_piece`: the operation names a piece the session does not hold.
    UnknownPiece,
    /// `stale_revision`: a room transaction's `base` is not the room's current revision.
    StaleRevision,
    /// `locked`: another member holds the piece's edit lease.
    Locked,
}

/// A refusal with the piece it names and the nearest feasible cell the
/// resolver could offer. `piece` is absent when the operation names none (a
/// placement) or the named piece is unknown; `nearest` is absent when no cell
/// was searched or none was feasible. `holder` is the member holding the edit
/// lease when `code` is [`RefusalCode::Locked`]. `detail` names the failing
/// cell and reason, or the input the check found missing, when the executor
/// could read it from the validator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub piece: Option<Piece>,
    pub nearest: Option<Cell>,
    pub holder: Option<String>,
    pub detail: Option<String>,
}

impl Refusal {
    pub fn new(code: RefusalCode) -> Self {
        Self {
            code,
            piece: None,
            nearest: None,
            holder: None,
            detail: None,
        }
    }
}

/// One applied operation's placed result. The corners are grid positions, the
/// same form the served row carries. `rotation` is the operation's quarter
/// turn; the served row stores the direction index (`0..3`), so the caller
/// maps the two when it writes a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpResult {
    pub piece: Piece,
    pub minimum: GridPosition,
    pub maximum: GridPosition,
    pub center_y: i8,
    pub rotation: Rot,
}

/// A transaction request. `base` is the room revision the caller resolved its
/// anchors against; `check_paths` enables the optional walkability check and
/// is never set by a player's direct edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Apply {
    pub ops: Vec<LayoutOp>,
    pub dry_run: bool,
    pub base: Option<u32>,
    pub check_paths: bool,
}

impl Apply {
    pub fn new(ops: Vec<LayoutOp>) -> Self {
        Self {
            ops,
            dry_run: false,
            base: None,
            check_paths: false,
        }
    }
}

/// Every operation applied. A dry run returns this without changing the
/// caller's layout; `revision` is the room's new revision when the caller has
/// one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub results: Vec<OpResult>,
    pub revision: Option<u32>,
}

/// The transaction stopped at `index`; `results_before` are the operations
/// applied up to, but not including, that one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub index: usize,
    pub refusal: Refusal,
    pub results_before: Vec<OpResult>,
}

/// The all-or-nothing result of an [`Apply`].
pub type ApplyOutcome = Result<Applied, Rejected>;
