//! The native transport: a WebSocket client on its own thread.

use futures_util::{SinkExt, StreamExt};
use moly_protocol::{decode, encode, ClientMessage, ServerMessage};
use tokio_tungstenite::tungstenite::Message;

use crate::LinkEvent;

pub(crate) fn connect(
    address: String,
    outgoing: async_channel::Receiver<ClientMessage>,
    incoming: async_channel::Sender<LinkEvent>,
) {
    let spawned = std::thread::Builder::new()
        .name("moly-link".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
                .expect("the link thread builds its runtime");
            let lost = runtime.block_on(session(&address, outgoing, &incoming));
            let _ = incoming.try_send(LinkEvent::Lost(lost));
        });
    if let Err(error) = spawned {
        bevy::log::error!("[link] the link thread did not start: {error}");
    }
}

/// Runs the session until it ends; returns why it ended.
async fn session(
    address: &str,
    outgoing: async_channel::Receiver<ClientMessage>,
    incoming: &async_channel::Sender<LinkEvent>,
) -> String {
    let socket = match tokio_tungstenite::connect_async(address).await {
        Ok((socket, _)) => socket,
        Err(error) => return format!("{address}: {error}"),
    };
    let (mut sink, mut source) = socket.split();
    let writer = async {
        while let Ok(message) = outgoing.recv().await {
            if let Err(error) = sink.send(Message::Binary(encode(&message).into())).await {
                return format!("sending failed: {error}");
            }
        }
        "the game closed the link".to_owned()
    };
    let reader = async {
        while let Some(message) = source.next().await {
            match message {
                Ok(Message::Binary(bytes)) => match decode::<ServerMessage>(&bytes) {
                    Ok(message) => {
                        if incoming.try_send(LinkEvent::Message(message)).is_err() {
                            return "the game stopped reading".to_owned();
                        }
                    }
                    Err(error) => return format!("{error}"),
                },
                Ok(Message::Close(frame)) => {
                    return frame.map_or_else(
                        || "the server closed the connection".to_owned(),
                        |frame| format!("the server closed the connection: {}", frame.reason),
                    )
                }
                Ok(_) => {}
                Err(error) => return format!("receiving failed: {error}"),
            }
        }
        "the server closed the connection".to_owned()
    };
    tokio::select! {
        reason = writer => reason,
        reason = reader => reason,
    }
}
