//! The game's connection to its server.
//!
//! [`LinkPlugin`] opens one session for one user and keeps it in
//! [`ServerLink`]. A system sends a request with [`ServerLink::call`] and
//! gets its sequence number; the server's answer arrives in a later frame as
//! a [`ServerArrived`] carrying that number. Everything the server sends —
//! answers and data it sends on its own — arrives in the server's order on
//! that one message. Nothing here answers for the server: when the server
//! cannot be reached, the link says so in [`LinkState`] and every call is
//! refused by name.

#[cfg(not(target_arch = "wasm32"))]
mod native;

use bevy::prelude::*;
use moly_protocol::{
    Call, CallKind, ClientMessage, PushKind, Refusal, Reply, Sections, ServerMessage, UserId,
    PROTOCOL_VERSION,
};

/// Where the server is and who plays.
#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// `ws://host:port` natively.
    pub address: String,
    pub user: UserId,
    /// The game's name and version, for the server's log.
    pub client: String,
    /// How long to keep trying to reach a server that does not answer yet
    /// (one started together with the game).
    pub connect_window: std::time::Duration,
}

impl LinkConfig {
    /// The default connect window.
    pub const CONNECT_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
}

/// The connection's state, for the game's notices and the control plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    /// The server said welcome.
    Open { server: String },
    /// The server refused the session.
    Refused { reason: String },
    /// The connection could not be made or was lost.
    Lost { reason: String },
}

impl LinkState {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

/// What the transport hands the game.
#[derive(Debug)]
pub(crate) enum LinkEvent {
    Message(ServerMessage),
    Lost(String),
}

/// One thing the server sent, in the order it sent it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arrival {
    /// The answer to the call numbered [`ServerArrived::seq`].
    Reply {
        call: CallKind,
        outcome: Result<Reply, Refusal>,
        sections: Sections,
    },
    /// User data the server sent without a call.
    Push {
        kind: PushKind,
        sections: Sections,
    },
}

/// Everything the server sent, in order. A reply carries the number of the
/// call it answers; a push has none.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ServerArrived {
    pub seq: Option<u32>,
    pub event: Arrival,
}

/// The session.
#[derive(Resource)]
pub struct ServerLink {
    state: LinkState,
    next_seq: u32,
    outgoing: async_channel::Sender<ClientMessage>,
    incoming: async_channel::Receiver<LinkEvent>,
    /// Calls sent and not yet answered: (seq, what was asked).
    waiting: Vec<(u32, CallKind)>,
    /// Calls made since the game last looked, for its event stream.
    sent: Vec<(u32, Call)>,
}

impl ServerLink {
    pub fn state(&self) -> &LinkState {
        &self.state
    }

    /// Sends a request and returns the number its [`ServerArrived`] will
    /// carry. A link that is not open refuses at once, also by a
    /// [`ServerArrived`] in the next frame, so a caller waits for its answer
    /// the same way.
    pub fn call(&mut self, call: Call) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        let seq = self.next_seq;
        let kind = call.kind();
        self.waiting.push((seq, kind));
        self.sent.push((seq, call.clone()));
        if self.state.is_open() || self.state == LinkState::Connecting {
            if self
                .outgoing
                .try_send(ClientMessage::Call { seq, call })
                .is_err()
            {
                self.state = LinkState::Lost {
                    reason: "the connection's writer stopped".to_owned(),
                };
            }
        }
        seq
    }

    /// The calls waiting for an answer, as their kind.
    pub fn waiting(&self) -> impl Iterator<Item = &(u32, CallKind)> {
        self.waiting.iter()
    }

    /// The calls made since the last look, oldest first. The game reads this
    /// once a frame for its event stream.
    pub fn take_sent(&mut self) -> Vec<(u32, Call)> {
        std::mem::take(&mut self.sent)
    }
}

/// Opens the session when the app starts.
pub struct LinkPlugin(pub LinkConfig);

impl Plugin for LinkPlugin {
    fn build(&self, app: &mut App) {
        let (outgoing, to_server) = async_channel::unbounded();
        let (to_game, incoming) = async_channel::unbounded();
        let config = self.0.clone();
        info!("[link] connecting to {} as user {}", config.address, config.user);
        let _ = outgoing.try_send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            user: config.user.clone(),
            client: config.client.clone(),
        });
        #[cfg(not(target_arch = "wasm32"))]
        let state = LinkState::Connecting;
        #[cfg(not(target_arch = "wasm32"))]
        native::connect(config.address.clone(), config.connect_window, to_server, to_game);
        #[cfg(target_arch = "wasm32")]
        let state = {
            let _ = (to_server, to_game);
            LinkState::Lost {
                reason: "no browser transport".to_owned(),
            }
        };
        app.insert_resource(ServerLink {
            state,
            next_seq: 0,
            outgoing,
            incoming,
            waiting: Vec::new(),
            sent: Vec::new(),
        })
        .add_message::<ServerArrived>()
        .add_systems(PreUpdate, pump);
    }
}

/// Turns what the transport received into messages, in arrival order.
fn pump(
    mut link: ResMut<ServerLink>,
    mut arrived: MessageWriter<ServerArrived>,
) {
    while let Ok(event) = link.incoming.try_recv() {
        match event {
            LinkEvent::Message(ServerMessage::Welcome { server, user, .. }) => {
                info!("[link] session open: {server}, user {user}");
                link.state = LinkState::Open { server };
            }
            LinkEvent::Message(ServerMessage::Refused { reason }) => {
                error!("[link] the server refused the session: {reason}");
                link.state = LinkState::Refused { reason };
            }
            LinkEvent::Message(ServerMessage::Reply {
                seq,
                outcome,
                sections,
            }) => {
                let Some(at) = link.waiting.iter().position(|(waiting, _)| *waiting == seq) else {
                    error!("[link] a reply numbered {seq} answers no call; it is dropped");
                    continue;
                };
                let (_, call) = link.waiting.remove(at);
                arrived.write(ServerArrived {
                    seq: Some(seq),
                    event: Arrival::Reply {
                        call,
                        outcome,
                        sections: sections.map(|sections| *sections).unwrap_or_default(),
                    },
                });
            }
            LinkEvent::Message(ServerMessage::Push { kind, sections }) => {
                arrived.write(ServerArrived {
                    seq: None,
                    event: Arrival::Push {
                        kind,
                        sections: *sections,
                    },
                });
            }
            LinkEvent::Lost(reason) => {
                if !matches!(link.state, LinkState::Refused { .. }) {
                    error!("[link] the server connection is lost: {reason}");
                    link.state = LinkState::Lost { reason };
                }
            }
        }
    }
    // Calls that can no longer be answered are refused by name.
    let reason = match &link.state {
        LinkState::Refused { reason } => format!("the server refused the session: {reason}"),
        LinkState::Lost { reason } => format!("the server is not reachable: {reason}"),
        LinkState::Connecting | LinkState::Open { .. } => return,
    };
    for (seq, call) in std::mem::take(&mut link.waiting) {
        warn!("[link] {} refused: {reason}", call.name());
        arrived.write(ServerArrived {
            seq: Some(seq),
            event: Arrival::Reply {
                call,
                outcome: Err(Refusal::new("link_lost", reason.clone())),
                sections: Sections::default(),
            },
        });
    }
}
