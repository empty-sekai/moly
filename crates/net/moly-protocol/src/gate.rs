//! Gate replies: the answer to a gate reserve or gate change request, as the
//! gate presenter receives it.

use serde::{Deserialize, Serialize};

use crate::rows::GateCharacter;

/// A reply's talk list (`mysekaiCharacterTalkWithReadHistories`): the rows
/// the server document states, or the talk-list policy's list, which the
/// server builds for the new visitors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplyTalkList {
    /// The stated rows (talk id, read).
    Stated(Vec<(i32, bool)>),
    /// The server's talk-list policy.
    Policy,
}

/// A gate reply for the gate presenter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Read by the gate presenter's seam.
pub enum GateReply {
    /// The reserve reply: the reserved row and the reply's talk list.
    Reserve {
        row: GateCharacter,
        talks: ReplyTalkList,
    },
    /// The change reply: the home gate's new gate and skin, its characters
    /// and the reply's talk list.
    Change {
        gate_id: i32,
        skin_id: i32,
        rows: Vec<GateCharacter>,
        talks: ReplyTalkList,
    },
}
