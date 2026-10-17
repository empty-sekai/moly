//! The messages the game and its server exchange.
//!
//! One connection carries one user's session:
//!
//! 1. The game opens with [`ClientMessage::Hello`], naming the protocol
//!    version and the user.
//! 2. The server answers [`ServerMessage::Welcome`], or
//!    [`ServerMessage::Refused`] and closes.
//! 3. Each [`ClientMessage::Call`] carries a sequence number that its
//!    [`ServerMessage::Reply`] repeats. Replies arrive in call order.
//! 4. [`ServerMessage::Push`] carries user data the server sends on its own,
//!    such as the sections of a new refresh window.
//!
//! Every reply and push may carry [`Sections`]: the user data tables the
//! game keeps copies of. The game writes its copies from these and from
//! nothing else.
//!
//! Frames are binary ([`encode`] / [`decode`]); [`to_json`] renders any
//! message as JSON for logs and tools.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Raised whenever a message changes shape. A peer with another version is
/// refused at the handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// The account a session belongs to. The server keeps one document per user.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UserId(pub String);

impl UserId {
    /// The user a single-player game plays as.
    pub fn local() -> Self {
        Self("local".to_owned())
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A message from the game to the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// The first message of a session.
    Hello {
        protocol: u32,
        user: UserId,
        /// The game's name and version, for the server's log.
        client: String,
    },
    /// A request. `seq` is the game's own counter; the reply repeats it.
    Call { seq: u32, call: Call },
}

/// A message from the server to the game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// The session is open.
    Welcome {
        protocol: u32,
        user: UserId,
        /// The server's name and version, for the game's log.
        server: String,
    },
    /// The session is refused; the server closes the connection after this.
    Refused { reason: String },
    /// The answer to the call numbered `seq`.
    Reply {
        seq: u32,
        outcome: Result<Reply, Refusal>,
        sections: Option<Box<Sections>>,
    },
    /// User data the server sends without a call.
    Push { kind: PushKind, sections: Box<Sections> },
}

/// Why the server did not carry out a call. The game shows or logs `reason`;
/// `code` is the server's own name for the case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: String,
    pub reason: String,
}

impl Refusal {
    pub fn new(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.reason)
    }
}

/// What a push answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushKind {
    /// The server clock entered a new refresh window.
    Refresh,
    /// The server's document was edited and the edit is delivered now.
    Sync,
    /// Rows of the birthday parties now in session.
    BirthdayPartySeat,
}

/// A request. Filled in by the API modules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Call {
    /// The join: the server seats the user and answers with every section.
    Join,
}

/// A request's own answer, beside the sections it carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    /// The join's answer.
    Joined,
}

/// The user data tables a reply or push carries. A `None` field is not
/// carried and leaves the game's copy as it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sections {
    /// The server time the sections were taken at, in milliseconds since the
    /// Unix epoch.
    pub server_date_ms: i64,
    /// Whether the server entered a new refresh window for this response.
    pub is_refreshed: bool,
}

/// A frame could not be read.
#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unreadable frame: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// A message as one binary frame.
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    bitcode::serialize(message).expect("protocol messages are always serializable")
}

/// A message from one binary frame.
pub fn decode<T: DeserializeOwned>(frame: &[u8]) -> Result<T, DecodeError> {
    bitcode::deserialize(frame).map_err(|error| DecodeError(error.to_string()))
}

/// A message as JSON, for logs and tools.
pub fn to_json<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).expect("protocol messages are always serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_round_trips_through_frames() {
        let messages = [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                user: UserId::local(),
                client: "moly-app 0.0.3".to_owned(),
            },
            ClientMessage::Call {
                seq: 7,
                call: Call::Join,
            },
        ];
        for message in messages {
            assert_eq!(decode::<ClientMessage>(&encode(&message)).unwrap(), message);
        }
        let reply = ServerMessage::Reply {
            seq: 7,
            outcome: Err(Refusal::new("not_joined", "join first")),
            sections: Some(Box::new(Sections {
                server_date_ms: 1_760_000_000_000,
                is_refreshed: true,
            })),
        };
        assert_eq!(decode::<ServerMessage>(&encode(&reply)).unwrap(), reply);
    }

    #[test]
    fn a_truncated_frame_is_refused() {
        let frame = encode(&ServerMessage::Refused {
            reason: "protocol 2 is not 1".to_owned(),
        });
        assert!(decode::<ServerMessage>(&frame[..frame.len() - 1]).is_err());
    }
}
