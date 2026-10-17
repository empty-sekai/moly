//! The native transport: a WebSocket listener on its own thread. Each
//! binary message is one protocol frame.

use std::net::SocketAddr;

use bevy::log::{info, warn};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::hub::{Hub, HubEvent};

/// Binds `address` now (so a taken port fails here, not later) and serves
/// connections on a background thread until the process ends. Returns the
/// bound address, which names the port when `address` asked for port 0.
pub fn listen(address: &str, hub: Hub) -> std::io::Result<SocketAddr> {
    let listener = std::net::TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    std::thread::Builder::new()
        .name("moly-server-ws".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
                .expect("the WebSocket thread builds its runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("the bound listener moves onto the runtime");
                loop {
                    match listener.accept().await {
                        Ok((stream, peer)) => {
                            let _ = stream.set_nodelay(true);
                            tokio::spawn(connection(stream, peer, hub.clone()));
                        }
                        Err(error) => warn!("[server] accept failed: {error}"),
                    }
                }
            });
        })?;
    Ok(local)
}

async fn connection(stream: tokio::net::TcpStream, peer: SocketAddr, hub: Hub) {
    let socket = match tokio_tungstenite::accept_async(stream).await {
        Ok(socket) => socket,
        Err(error) => {
            warn!("[server] {peer}: WebSocket handshake failed: {error}");
            return;
        }
    };
    let session = hub.next_session();
    let (out, outgoing) = async_channel::unbounded::<Vec<u8>>();
    if hub.send(HubEvent::Open { session, out }).is_err() {
        return;
    }
    info!("[server] {peer}: connected as session {}", session.0);
    let (mut sink, mut source) = socket.split();
    // The writer drains every queued frame before it closes, so a refusal
    // reaches the peer before the connection does.
    let writer = async move {
        while let Ok(frame) = outgoing.recv().await {
            if sink.send(Message::Binary(frame.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    };
    let reader = async {
        while let Some(message) = source.next().await {
            match message {
                Ok(Message::Binary(bytes)) => {
                    if hub
                        .send(HubEvent::Frame {
                            session,
                            bytes: bytes.to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                // Pings are answered by the library; text frames are not
                // protocol frames.
                Ok(_) => {}
            }
        }
    };
    tokio::select! {
        () = writer => {}
        () = reader => {}
    }
    let _ = hub.send(HubEvent::Close { session });
}
