//! Sessions: the frames the transport hands in, the handshake, and the
//! frames going back out.
//!
//! The HTTP transport owns the connections and talks to the server only
//! through a [`Hub`]: for one request it opens a session with the sender its
//! outgoing frames go to, reports each frame it decoded from the request,
//! and closes the session once the response is sent. Everything after that
//! is the server's.

use std::collections::HashMap;

use bevy::prelude::*;
use moly_protocol::{
    decode, encode, Call, ClientMessage, ServerMessage, UserId, PROTOCOL_VERSION,
};

/// One request's session, numbered by its transport's hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

/// What the transport reports.
#[derive(Debug)]
pub enum HubEvent {
    /// A session opened; frames for it go to `out`.
    Open {
        session: SessionId,
        out: async_channel::Sender<Vec<u8>>,
    },
    /// One message of the request, encoded as one frame.
    Frame { session: SessionId, bytes: Vec<u8> },
    /// The session closed, or was refused and forgot.
    Close { session: SessionId },
}

/// The handle transports hold: cheap to clone, usable from any thread.
#[derive(Resource, Clone)]
pub struct Hub {
    events: async_channel::Sender<HubEvent>,
    next: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Hub {
    /// A session number no other session of this hub has.
    pub fn next_session(&self) -> SessionId {
        SessionId(
            self.next
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Hands an event to the server. Fails only once the server is gone.
    pub fn send(&self, event: HubEvent) -> Result<(), String> {
        self.events
            .try_send(event)
            .map_err(|error| format!("the server is not running: {error}"))
    }
}

#[derive(Resource)]
pub(crate) struct HubInbox(async_channel::Receiver<HubEvent>);

/// A session after its transport opened it.
struct Session {
    out: async_channel::Sender<Vec<u8>>,
    /// Set by the handshake.
    user: Option<UserId>,
}

/// The open sessions.
#[derive(Resource, Default)]
pub struct Sessions(HashMap<SessionId, Session>);

impl Sessions {
    /// Sends one message to a session. A session that closed meanwhile is
    /// skipped and named.
    pub fn send(&self, session: SessionId, message: &ServerMessage) {
        let Some(open) = self.0.get(&session) else {
            warn!("[server] session {} closed before its message: {}", session.0, kind_of(message));
            return;
        };
        if open.out.try_send(encode(message)).is_err() {
            warn!("[server] session {} stopped reading: {}", session.0, kind_of(message));
        }
    }

    /// The user a session belongs to, once it has said hello.
    pub fn user(&self, session: SessionId) -> Option<&UserId> {
        self.0.get(&session).and_then(|open| open.user.as_ref())
    }

    /// The number of open sessions.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The sessions a user has open.
    pub fn sessions_of<'a>(&'a self, user: &'a UserId) -> impl Iterator<Item = SessionId> + 'a {
        self.0.iter().filter_map(move |(session, open)| {
            (open.user.as_ref() == Some(user)).then_some(*session)
        })
    }

    /// Sends one message to every open session of a user.
    pub fn broadcast(&self, user: &UserId, message: &ServerMessage) {
        for session in self.sessions_of(user) {
            self.send(session, message);
        }
    }
}

/// A call from a session past its handshake, for the API handlers.
#[derive(Message, Clone, Debug)]
pub struct CallReceived {
    pub session: SessionId,
    pub user: UserId,
    pub seq: u32,
    pub call: Call,
}

/// A session that has said hello, for the handlers that seat its user.
#[derive(Message, Clone, Debug)]
pub struct SessionOpened {
    pub session: SessionId,
    pub user: UserId,
}

/// The server's name in its handshake.
pub fn server_name() -> String {
    format!("moly-server {}", env!("CARGO_PKG_VERSION"))
}

fn kind_of(message: &ServerMessage) -> &'static str {
    match message {
        ServerMessage::Welcome { .. } => "welcome",
        ServerMessage::Refused { .. } => "refusal",
        ServerMessage::Reply { .. } => "reply",
        ServerMessage::Push { .. } => "push",
    }
}

/// Installs the hub and returns the handle transports use.
pub(crate) fn install(app: &mut App) -> Hub {
    let (events, inbox) = async_channel::unbounded();
    let hub = Hub {
        events,
        next: Default::default(),
    };
    app.insert_resource(hub.clone())
        .insert_resource(HubInbox(inbox))
        .init_resource::<Sessions>()
        .add_message::<CallReceived>()
        .add_message::<SessionOpened>()
        .add_systems(PreUpdate, pump)
        .add_systems(PostUpdate, end_frame);
    hub
}

/// Ends every open session's frames of this update with an empty frame, so a
/// transport that answers one request knows that everything the update sent
/// the session — the replies and the pushes flushed after them — is in.
pub(crate) fn end_frame(sessions: Res<Sessions>) {
    for session in sessions.0.values() {
        let _ = session.out.try_send(Vec::new());
    }
}

/// Takes every event the transport reported since the last frame.
pub(crate) fn pump(
    inbox: Res<HubInbox>,
    mut sessions: ResMut<Sessions>,
    mut calls: MessageWriter<CallReceived>,
    mut opened: MessageWriter<SessionOpened>,
) {
    while let Ok(event) = inbox.0.try_recv() {
        match event {
            HubEvent::Open { session, out } => {
                debug!("[server] session {} opened", session.0);
                sessions.0.insert(session, Session { out, user: None });
            }
            HubEvent::Close { session } => {
                if let Some(closed) = sessions.0.remove(&session) {
                    let user = closed
                        .user
                        .map_or_else(|| "none".to_owned(), |user| format!("user {user}"));
                    debug!("[server] session {} closed ({user})", session.0);
                }
            }
            HubEvent::Frame { session, bytes } => {
                let message = match decode::<ClientMessage>(&bytes) {
                    Ok(message) => message,
                    Err(error) => {
                        refuse(&mut sessions, session, format!("{error}"));
                        continue;
                    }
                };
                match message {
                    ClientMessage::Hello {
                        protocol,
                        user,
                        client,
                    } => {
                        if protocol != PROTOCOL_VERSION {
                            refuse(
                                &mut sessions,
                                session,
                                format!(
                                    "protocol {protocol} is not this server's {PROTOCOL_VERSION}"
                                ),
                            );
                            continue;
                        }
                        if !user.is_valid() {
                            refuse(
                                &mut sessions,
                                session,
                                format!(
                                    "the user id is not 1 to {} letters, digits, _ or -",
                                    UserId::MAX_LEN
                                ),
                            );
                            continue;
                        }
                        let Some(open) = sessions.0.get_mut(&session) else {
                            continue;
                        };
                        if let Some(previous) = &open.user {
                            let reason = format!("this session already said hello as {previous}");
                            refuse(&mut sessions, session, reason);
                            continue;
                        }
                        open.user = Some(user.clone());
                        debug!("[server] session {} hello: user {user}, {client}", session.0);
                        sessions.send(
                            session,
                            &ServerMessage::Welcome {
                                protocol: PROTOCOL_VERSION,
                                user: user.clone(),
                                server: server_name(),
                            },
                        );
                        opened.write(SessionOpened { session, user });
                    }
                    ClientMessage::Call { seq, call } => {
                        let Some(user) = sessions.user(session).cloned() else {
                            refuse(&mut sessions, session, "a call before the hello".to_owned());
                            continue;
                        };
                        calls.write(CallReceived {
                            session,
                            user,
                            seq,
                            call,
                        });
                    }
                }
            }
        }
    }
}

/// Refuses a session and forgets it, so no later frame of the request is
/// read; the refusal stays in the session's outgoing frames for the
/// transport to send.
fn refuse(sessions: &mut Sessions, session: SessionId, reason: String) {
    warn!("[server] session {} refused: {reason}", session.0);
    sessions.send(session, &ServerMessage::Refused { reason });
    sessions.0.remove(&session);
}
