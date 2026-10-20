//! The game's connection to its server.
//!
//! [`LinkPlugin`] keeps one link to one user of a server in [`ServerLink`].
//! A system sends a request with [`ServerLink::call`] and gets its sequence
//! number; the server's answer arrives in a later frame as a
//! [`ServerArrived`] carrying that number. Everything the server sends —
//! answers and data it sends on its own — arrives in the server's order.
//! Nothing here answers for the server: when the server cannot be reached,
//! the link says so in [`LinkState`] and every call is refused by name.
//!
//! The link speaks the server's one HTTP endpoint. Each request is the
//! encoded [`ClientMessage`] list `[Hello, Call]` for a call or `[Hello]`
//! alone for a poll, and each answer is the encoded [`ServerMessage`] list.
//! One request is in flight at a time; calls wait in a queue and are sent in
//! the order they were made. A welcome in an answer opens the link, and a
//! later answer's welcome opens it again. While no call is queued, the link
//! polls once no request has been made for [`POLL_INTERVAL`], so the pushes
//! the server queued for the user arrive.
//!
//! With the `local` feature, `LocalLinkPlugin` hosts the server in the
//! game's own app and sends the same requests to it in memory.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

mod address;

use bevy::prelude::*;
use moly_protocol::{
    Call, CallKind, ClientMessage, PushKind, Refusal, Reply, Sections, ServerMessage, UserId,
    PROTOCOL_VERSION, RPC_PATH,
};

pub use address::{is_server_address, DEFAULT_SERVER_URL, PUBLIC_SERVER_URL};

/// How long the link waits for the first welcome: a server started together
/// with the game may not listen yet.
pub const CONNECT_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// The link polls when no request has been made for this long, so the pushes
/// the server queued arrive.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// How long one request may wait for its answer before the link reports the
/// server lost.
pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long after a failed request the link tries the server again.
pub const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Where the server is and who plays.
#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// `http://host:port` or `https://host[:port]`, as
    /// [`is_server_address`] admits.
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
    pub const CONNECT_WINDOW: std::time::Duration = CONNECT_WINDOW;
}

/// The connection's state, for the game's notices and the control plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    /// An answer said welcome.
    Open { server: String },
    /// An answer refused the session.
    Refused { reason: String },
    /// A request could not be made or answered.
    Lost { reason: String },
    /// The app links to no server ([`OfflineLinkPlugin`]); every call is
    /// refused.
    Offline { reason: String },
}

impl LinkState {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

/// What the transport hands the game.
#[derive(Debug)]
pub(crate) enum LinkEvent {
    /// Every message of one answer, in the server's order.
    Answer(Vec<ServerMessage>),
    /// The request failed; the string names the address and what happened.
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

/// The link.
#[derive(Resource)]
pub struct ServerLink {
    state: LinkState,
    next_seq: u32,
    /// The calls for the transport, in the order they were made.
    outgoing: async_channel::Sender<(u32, Call)>,
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
        if (self.state.is_open() || self.state == LinkState::Connecting)
            && self.outgoing.try_send((seq, call)).is_err()
        {
            self.state = LinkState::Lost {
                reason: "the link's transport stopped".to_owned(),
            };
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

/// Opens the link when the app starts.
pub struct LinkPlugin(pub LinkConfig);

impl Plugin for LinkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.0.clone();
        info!("[link] connecting to {} as user {}", config.address, config.user);
        install(
            app,
            Endpoint::Http(config.address),
            config.user,
            config.client,
            config.connect_window,
        );
    }
}

/// Hosts the server in this app and links to it in memory: the server's
/// plugins are added to the app, read the masters and the default document
/// through the app's own asset source, and answer the link's requests
/// through their hub. The requests and answers are those of the HTTP
/// endpoint.
#[cfg(feature = "local")]
pub struct LocalLinkPlugin {
    pub user: UserId,
    /// The game's name and version, for the server's log.
    pub client: String,
}

#[cfg(feature = "local")]
impl Plugin for LocalLinkPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<moly_server::MasterLoadPlugin>() {
            app.add_plugins(moly_server::MasterLoadPlugin);
        }
        app.add_plugins(moly_server::ServerPlugin);
        info!("[link] the server runs in this app; user {}", self.user);
        install(
            app,
            Endpoint::Local(moly_server::hub(app)),
            self.user.clone(),
            self.client.clone(),
            CONNECT_WINDOW,
        );
    }
}

/// Starts a transport to `endpoint` and installs the link.
fn install(
    app: &mut App,
    endpoint: Endpoint,
    user: UserId,
    client: String,
    connect_window: std::time::Duration,
) {
    let (outgoing, calls) = async_channel::unbounded();
    let (to_game, incoming) = async_channel::unbounded();
    let transport = LinkTransport {
        endpoint,
        user,
        client,
        connect_window,
        calls,
        to_game,
    };
    #[cfg(not(target_arch = "wasm32"))]
    native::connect(transport);
    #[cfg(target_arch = "wasm32")]
    web::connect(transport);
    app.insert_resource(ServerLink {
        state: LinkState::Connecting,
        next_seq: 0,
        outgoing,
        incoming,
        waiting: Vec::new(),
        sent: Vec::new(),
    })
    .add_message::<ServerArrived>()
    .add_systems(PreUpdate, pump);
}

/// A link that never connects: the app plays without a server, and every
/// call is refused by name in the next frame. The string says why.
pub struct OfflineLinkPlugin(pub String);

impl Plugin for OfflineLinkPlugin {
    fn build(&self, app: &mut App) {
        // No transport takes the calls: the state is never open, so `call`
        // queues nothing and `pump` refuses what waits.
        let (outgoing, _) = async_channel::unbounded();
        let (_, incoming) = async_channel::unbounded();
        info!("[link] no server: {}", self.0);
        app.insert_resource(ServerLink {
            state: LinkState::Offline {
                reason: self.0.clone(),
            },
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

/// Where a transport's requests go.
#[derive(Clone)]
pub(crate) enum Endpoint {
    /// The server's HTTP endpoint at this `http://host:port` address.
    Http(String),
    /// A server hosted in this app, through its hub.
    #[cfg(feature = "local")]
    Local(moly_server::Hub),
}

/// What a transport needs: where the server is, who says hello, how long it
/// retries before the first welcome, the calls to send and the game's inbox.
pub(crate) struct LinkTransport {
    pub(crate) endpoint: Endpoint,
    user: UserId,
    client: String,
    pub(crate) connect_window: std::time::Duration,
    pub(crate) calls: async_channel::Receiver<(u32, Call)>,
    pub(crate) to_game: async_channel::Sender<LinkEvent>,
}

impl LinkTransport {
    /// Where every request goes, as the link's messages name it: the HTTP
    /// endpoint's URL, or the server in this app.
    pub(crate) fn url(&self) -> String {
        match &self.endpoint {
            Endpoint::Http(address) => format!("{address}{RPC_PATH}"),
            #[cfg(feature = "local")]
            Endpoint::Local(_) => "the server in this app".to_owned(),
        }
    }

    /// The body of one request: the hello, then the call when there is one.
    pub(crate) fn request(&self, call: Option<(u32, Call)>) -> Vec<u8> {
        let mut messages = vec![ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            user: self.user.clone(),
            client: self.client.clone(),
        }];
        if let Some((seq, call)) = call {
            messages.push(ClientMessage::Call { seq, call });
        }
        moly_protocol::encode(&messages)
    }

    /// Hands one answer, or the failure of its request, to the game. False
    /// once the game is gone.
    pub(crate) fn deliver(&self, answer: Result<Vec<ServerMessage>, String>) -> bool {
        let event = match answer {
            Ok(messages) => LinkEvent::Answer(messages),
            Err(reason) => LinkEvent::Lost(format!("{}: {reason}", self.url())),
        };
        self.to_game.try_send(event).is_ok()
    }

    /// Drops the calls queued while the link could not send them: the game
    /// refuses each by name once it reads the failure.
    pub(crate) fn drop_queued(&self) {
        while self.calls.try_recv().is_ok() {}
    }
}

/// Whether an answer said welcome.
pub(crate) fn welcomed(answer: &Result<Vec<ServerMessage>, String>) -> bool {
    answer.as_ref().is_ok_and(|messages| {
        messages
            .iter()
            .any(|message| matches!(message, ServerMessage::Welcome { .. }))
    })
}

/// How long after a failed request an open link tries the server again.
pub(crate) const RECOVER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// What a transport does next.
pub(crate) enum Next {
    /// Send this call.
    Call((u32, Call)),
    /// Send a poll.
    Poll,
    /// Nothing is due: look again shortly, or wait for a call.
    Wait,
    /// A retry is due after this long.
    Sleep(std::time::Duration),
    /// The game is gone or the server refused the session.
    Stop,
}

/// The order of a transport's requests, the same on every target. Times are
/// the transport's own clock since it started.
///
/// Until the first welcome the transport polls every [`RETRY_INTERVAL`] for
/// the connect window, keeping the calls made meanwhile; then it sends the
/// queued calls in order and polls once no request has been made for
/// [`POLL_INTERVAL`]. After a failed request the game reads the link as lost
/// and refuses its waiting calls, so the transport drops every call queued
/// until a later poll is welcomed again, and polls every
/// [`RECOVER_INTERVAL`].
pub(crate) struct Schedule {
    connect_window: std::time::Duration,
    /// An answer said welcome and no request failed since.
    open: bool,
    /// A request failed (or the connect window ran out) and no answer has
    /// said welcome since.
    recovering: bool,
    /// Some answer has said welcome.
    ever_open: bool,
    stopped: bool,
    last_request: std::time::Duration,
    retry_at: Option<std::time::Duration>,
}

impl Schedule {
    pub(crate) fn new(connect_window: std::time::Duration) -> Self {
        Self {
            connect_window,
            open: false,
            recovering: false,
            ever_open: false,
            stopped: false,
            last_request: std::time::Duration::ZERO,
            retry_at: None,
        }
    }

    /// What to do at `now`.
    pub(crate) fn next(&mut self, now: std::time::Duration, transport: &LinkTransport) -> Next {
        if self.stopped || transport.to_game.is_closed() {
            return Next::Stop;
        }
        if self.recovering {
            transport.drop_queued();
        }
        if let Some(at) = self.retry_at {
            if now < at {
                return Next::Sleep(at - now);
            }
            self.retry_at = None;
            return Next::Poll;
        }
        if !self.open {
            return Next::Poll;
        }
        match transport.calls.try_recv() {
            Ok(call) => Next::Call(call),
            Err(async_channel::TryRecvError::Closed) => Next::Stop,
            Err(async_channel::TryRecvError::Empty) if self.poll_due_in(now).is_zero() => Next::Poll,
            Err(async_channel::TryRecvError::Empty) => Next::Wait,
        }
    }

    /// How long until an idle open link polls.
    pub(crate) fn poll_due_in(&self, now: std::time::Duration) -> std::time::Duration {
        (self.last_request + POLL_INTERVAL).saturating_sub(now)
    }

    /// Records the answer of the request made just before `now` and hands it
    /// to the game. False once the transport should stop.
    pub(crate) fn answered(
        &mut self,
        now: std::time::Duration,
        answer: Result<Vec<ServerMessage>, String>,
        transport: &LinkTransport,
    ) -> bool {
        self.last_request = now;
        match &answer {
            Ok(messages) => {
                if messages
                    .iter()
                    .any(|message| matches!(message, ServerMessage::Refused { .. }))
                {
                    self.stopped = true;
                } else if welcomed(&answer) {
                    self.open = true;
                    self.ever_open = true;
                    self.recovering = false;
                }
            }
            Err(_) => {
                self.open = false;
                if !self.ever_open && !self.recovering && now < self.connect_window {
                    // A server started together with the game may not listen
                    // yet: try again without telling the game.
                    self.retry_at = Some(now + RETRY_INTERVAL);
                    return true;
                }
                self.recovering = true;
                transport.drop_queued();
                self.retry_at = Some(now + RECOVER_INTERVAL);
            }
        }
        transport.deliver(answer) && !self.stopped
    }
}

/// Turns what the transport received into messages, in arrival order.
fn pump(mut link: ResMut<ServerLink>, mut arrived: MessageWriter<ServerArrived>) {
    while let Ok(event) = link.incoming.try_recv() {
        match event {
            LinkEvent::Answer(messages) => {
                for message in messages {
                    match message {
                        ServerMessage::Welcome { server, user, .. } => {
                            // One line when the link opens; the welcome of
                            // every later answer keeps it open.
                            if !link.state.is_open() {
                                info!("[link] session open: {server}, user {user}");
                            }
                            link.state = LinkState::Open { server };
                        }
                        ServerMessage::Refused { reason } => {
                            error!("[link] the server refused the session: {reason}");
                            link.state = LinkState::Refused { reason };
                        }
                        ServerMessage::Reply {
                            seq,
                            outcome,
                            sections,
                        } => {
                            let Some(at) = link
                                .waiting
                                .iter()
                                .position(|(waiting, _)| *waiting == seq)
                            else {
                                error!("[link] a reply numbered {seq} answers no call; it is dropped");
                                continue;
                            };
                            let (_, call) = link.waiting.remove(at);
                            arrived.write(ServerArrived {
                                seq: Some(seq),
                                event: Arrival::Reply {
                                    call,
                                    outcome,
                                    sections: sections
                                        .map(|sections| *sections)
                                        .unwrap_or_default(),
                                },
                            });
                        }
                        ServerMessage::Push { kind, sections } => {
                            arrived.write(ServerArrived {
                                seq: None,
                                event: Arrival::Push {
                                    kind,
                                    sections: *sections,
                                },
                            });
                        }
                    }
                }
            }
            LinkEvent::Lost(reason) => {
                if !matches!(link.state, LinkState::Refused { .. } | LinkState::Offline { .. }) {
                    // A link that keeps failing the same way says so once.
                    let known = matches!(&link.state, LinkState::Lost { reason: known } if *known == reason);
                    if !known {
                        error!("[link] the server is not reachable: {reason}");
                    }
                    link.state = LinkState::Lost { reason };
                }
            }
        }
    }
    // Calls that can no longer be answered are refused by name.
    let reason = match &link.state {
        LinkState::Refused { reason } => format!("the server refused the session: {reason}"),
        LinkState::Lost { reason } => format!("the server is not reachable: {reason}"),
        LinkState::Offline { reason } => format!("this app links to no server: {reason}"),
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
