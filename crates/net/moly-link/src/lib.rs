//! The game's connection to its server.
//!
//! [`LinkPlugin`] opens one session for one user and keeps it in
//! [`ServerLink`]. A system sends a request with [`ServerLink::call`] and
//! gets its sequence number; the answer arrives in a later frame as a
//! [`Replied`] message carrying that number. Data the server sends on its
//! own arrives as [`Pushed`]. Nothing here answers for the server: when the
//! server cannot be reached, the link says so in [`LinkState`] and every
//! call is refused by name.

#[cfg(not(target_arch = "wasm32"))]
mod native;

use bevy::prelude::*;
use moly_protocol::{
    Call, ClientMessage, PushKind, Refusal, Reply, Sections, ServerMessage, UserId,
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

/// The session.
#[derive(Resource)]
pub struct ServerLink {
    state: LinkState,
    next_seq: u32,
    outgoing: async_channel::Sender<ClientMessage>,
    incoming: async_channel::Receiver<LinkEvent>,
    /// Calls sent and not yet answered: (seq, what was asked).
    waiting: Vec<(u32, &'static str)>,
}

impl ServerLink {
    pub fn state(&self) -> &LinkState {
        &self.state
    }

    /// Sends a request and returns the number its [`Replied`] will carry.
    /// A link that is not open refuses at once, also by a [`Replied`] in
    /// the next frame, so a caller waits for its answer the same way.
    pub fn call(&mut self, call: Call) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        let seq = self.next_seq;
        let name = call_name(&call);
        self.waiting.push((seq, name));
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

    /// The calls waiting for an answer.
    pub fn waiting(&self) -> impl Iterator<Item = &(u32, &'static str)> {
        self.waiting.iter()
    }
}

/// The answer to one call.
#[derive(Message, Clone, Debug)]
pub struct Replied {
    pub seq: u32,
    pub outcome: Result<Reply, Refusal>,
    pub sections: Option<Box<Sections>>,
}

/// Data the server sent without a call.
#[derive(Message, Clone, Debug)]
pub struct Pushed {
    pub kind: PushKind,
    pub sections: Box<Sections>,
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
        native::connect(config.address.clone(), to_server, to_game);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (to_server, to_game);
        }
        app.insert_resource(ServerLink {
            state: LinkState::Connecting,
            next_seq: 0,
            outgoing,
            incoming,
            waiting: Vec::new(),
        })
        .add_message::<Replied>()
        .add_message::<Pushed>()
        .add_systems(PreUpdate, pump);
    }
}

/// Turns what the transport received into messages, in arrival order.
fn pump(
    mut link: ResMut<ServerLink>,
    mut replied: MessageWriter<Replied>,
    mut pushed: MessageWriter<Pushed>,
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
                link.waiting.retain(|(waiting, _)| *waiting != seq);
                replied.write(Replied {
                    seq,
                    outcome,
                    sections,
                });
            }
            LinkEvent::Message(ServerMessage::Push { kind, sections }) => {
                pushed.write(Pushed { kind, sections });
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
    for (seq, name) in std::mem::take(&mut link.waiting) {
        warn!("[link] {name} refused: {reason}");
        replied.write(Replied {
            seq,
            outcome: Err(Refusal::new("unreachable", reason.clone())),
            sections: None,
        });
    }
}

fn call_name(call: &Call) -> &'static str {
    match call {
        Call::Join => "join",
    }
}
