//! A WebSocket client against the native listener: the handshake, one call
//! and its reply, and the refusal of another protocol version.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use moly_protocol::{
    decode, encode, Call, ClientMessage, Reply, ServerMessage, UserId, PROTOCOL_VERSION,
};
use moly_server::Masters;
use tungstenite::Message;

fn server() -> (App, std::net::SocketAddr) {
    let mut app = App::new();
    // A harness seats the masters it drives the model with; the server then
    // loads none.
    app.insert_resource(Masters::fixture());
    app.add_plugins(MinimalPlugins)
        .add_plugins(moly_server::ServerPlugin);
    let address = moly_server::ws::listen("127.0.0.1:0", moly_server::hub(&app)).unwrap();
    (app, address)
}

/// Runs the server until the client thread has finished.
fn drive<T: Send + 'static>(app: &mut App, client: std::thread::JoinHandle<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.is_finished() {
        assert!(Instant::now() < deadline, "the client did not finish in time");
        app.update();
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap()
}

fn read(socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>) -> Option<ServerMessage> {
    loop {
        match socket.read() {
            Ok(Message::Binary(bytes)) => return Some(decode(&bytes).unwrap()),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

fn hello(protocol: u32) -> Message {
    Message::Binary(
        encode(&ClientMessage::Hello {
            protocol,
            user: UserId::local(),
            client: "test".to_owned(),
        })
        .into(),
    )
}

#[test]
fn a_session_says_hello_and_joins() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || {
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}")).unwrap();
        socket.send(hello(PROTOCOL_VERSION)).unwrap();
        let welcome = read(&mut socket);
        let call = ClientMessage::Call {
            seq: 3,
            call: Call::Join,
        };
        socket.send(Message::Binary(encode(&call).into())).unwrap();
        let reply = read(&mut socket);
        (welcome, reply)
    });
    let (welcome, reply) = drive(&mut app, client);
    assert!(
        matches!(welcome, Some(ServerMessage::Welcome { protocol: PROTOCOL_VERSION, ref user, .. }) if *user == UserId::local()),
        "{welcome:?}"
    );
    match reply {
        Some(ServerMessage::Reply {
            seq: 3,
            outcome: Ok(Reply::Joined),
            sections: Some(sections),
        }) => assert!(sections.server_date_ms > 0),
        other => panic!("not the join's reply: {other:?}"),
    }
}

#[test]
fn another_protocol_version_is_refused_and_closed() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || {
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}")).unwrap();
        socket.send(hello(PROTOCOL_VERSION + 1)).unwrap();
        let first = read(&mut socket);
        let after = read(&mut socket);
        (first, after)
    });
    let (first, after) = drive(&mut app, client);
    match first {
        Some(ServerMessage::Refused { reason }) => {
            assert!(reason.contains(&format!("protocol {}", PROTOCOL_VERSION + 1)), "{reason}")
        }
        other => panic!("not a refusal: {other:?}"),
    }
    assert_eq!(after, None, "the connection stays open after the refusal");
}

#[test]
fn a_call_before_the_hello_is_refused() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || {
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}")).unwrap();
        let call = ClientMessage::Call {
            seq: 1,
            call: Call::Join,
        };
        socket.send(Message::Binary(encode(&call).into())).unwrap();
        read(&mut socket)
    });
    let first = drive(&mut app, client);
    assert!(
        matches!(&first, Some(ServerMessage::Refused { reason }) if reason.contains("before the hello")),
        "{first:?}"
    );
}
