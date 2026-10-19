//! An HTTP client against the native endpoint: the handshake, one call and
//! its reply, a refusal, and the pushes a request carries for its user.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use moly_protocol::{
    decode, encode, Call, ClientMessage, Reply, ServerMessage, UserId, PROTOCOL_VERSION,
    RPC_PATH,
};
use moly_server::Masters;

fn server() -> (App, SocketAddr) {
    let mut app = App::new();
    // A harness seats the masters it drives the model with; the server then
    // loads none.
    app.insert_resource(Masters::fixture());
    app.add_plugins(MinimalPlugins)
        .add_plugins(moly_server::ServerPlugin);
    let address = moly_server::http::listen("127.0.0.1:0", moly_server::hub(&app)).unwrap();
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

/// One request and the whole response: the status, the plain reason of a
/// failed status, and the messages of a protocol answer.
struct Answer {
    status: u16,
    reason: String,
    messages: Vec<ServerMessage>,
}

fn request(address: SocketAddr, method: &str, path: &str, body: &[u8]) -> Answer {
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let sent = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(sent.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a response head");
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    // The head names the length of the body that follows it.
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        // A response without a body (204) names no length.
        .unwrap_or(0);
    let body = response[split + 4..].get(..length).expect("the whole body");
    if status == 200 {
        Answer {
            status,
            reason: String::new(),
            messages: decode(body).expect("a protocol answer"),
        }
    } else {
        Answer {
            status,
            reason: String::from_utf8(body.to_vec()).unwrap(),
            messages: Vec::new(),
        }
    }
}

fn post(address: SocketAddr, messages: &[ClientMessage]) -> Answer {
    request(address, "POST", RPC_PATH, &encode(&messages.to_vec()))
}

fn hello() -> ClientMessage {
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        user: UserId::local(),
        client: "test".to_owned(),
    }
}

fn call(seq: u32, call: Call) -> ClientMessage {
    ClientMessage::Call { seq, call }
}

fn kinds(messages: &[ServerMessage]) -> Vec<&'static str> {
    messages
        .iter()
        .map(|message| match message {
            ServerMessage::Welcome { .. } => "welcome",
            ServerMessage::Refused { .. } => "refused",
            ServerMessage::Reply { .. } => "reply",
            ServerMessage::Push { .. } => "push",
        })
        .collect()
}

#[test]
fn a_request_without_a_call_answers_the_welcome_only() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || post(address, &[hello()]));
    let answer = drive(&mut app, client);
    assert_eq!(answer.status, 200);
    assert_eq!(kinds(&answer.messages), ["welcome"]);
    assert!(matches!(
        &answer.messages[0],
        ServerMessage::Welcome { protocol: PROTOCOL_VERSION, user, .. } if *user == UserId::local()
    ));
}

#[test]
fn a_call_answers_the_welcome_then_its_reply() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || post(address, &[hello(), call(3, Call::Join)]));
    let answer = drive(&mut app, client);
    assert_eq!(answer.status, 200);
    assert_eq!(kinds(&answer.messages), ["welcome", "reply"]);
    match &answer.messages[1] {
        ServerMessage::Reply {
            seq: 3,
            outcome: Ok(Reply::Joined),
            sections: Some(sections),
        } => assert!(sections.server_date_ms > 0),
        other => panic!("not the join's reply: {other:?}"),
    }
}

#[test]
fn a_refusal_is_a_message_with_the_welcome() {
    let (mut app, address) = server();
    let client = std::thread::spawn(move || {
        let messages = [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION + 1,
                user: UserId::local(),
                client: "test".to_owned(),
            },
            call(1, Call::Join),
        ];
        post(address, &messages)
    });
    let answer = drive(&mut app, client);
    assert_eq!(answer.status, 200);
    assert_eq!(kinds(&answer.messages), ["refused"]);
    match &answer.messages[0] {
        ServerMessage::Refused { reason } => {
            assert!(reason.contains(&format!("protocol {}", PROTOCOL_VERSION + 1)), "{reason}")
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

#[test]
fn a_user_id_that_could_name_another_file_is_refused() {
    for user in ["../local", "a/b", "", &"x".repeat(UserId::MAX_LEN + 1)] {
        let (mut app, address) = server();
        let user = UserId(user.to_owned());
        let client = std::thread::spawn(move || {
            let messages = [
                ClientMessage::Hello { protocol: PROTOCOL_VERSION, user, client: "test".to_owned() },
                call(1, Call::Join),
            ];
            post(address, &messages)
        });
        let answer = drive(&mut app, client);
        assert_eq!(kinds(&answer.messages), ["refused"]);
    }
}

#[test]
fn a_push_made_without_a_session_arrives_with_the_next_request() {
    // A party that started a minute ago on the server clock, with the
    // delivery item stock it seats: written as a reader of the master table
    // reads it.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let parties = format!(
        r#"[{{"id": 1, "assetbundleName": "party-1", "startAt": {}, "closedAt": 0,
             "deliveryItemMaterialId": 11, "deliveryRewardMysekaiMaterialId": 12}}]"#,
        now - 60_000
    );
    let mut app = App::new();
    app.insert_resource(Masters::fixture());
    // No party is known when the user joins.
    app.insert_resource(moly_server::Parties::fixture("[]"));
    app.add_plugins(MinimalPlugins)
        .add_plugins(moly_server::ServerPlugin);
    let address = moly_server::http::listen("127.0.0.1:0", moly_server::hub(&app)).unwrap();
    // The user joins: the join seats the party in its reply.
    let client = std::thread::spawn(move || post(address, &[hello(), call(1, Call::Join)]));
    let answer = drive(&mut app, client);
    assert_eq!(kinds(&answer.messages), ["welcome", "reply"]);
    assert_eq!(answer.messages.len(), 2, "{:?}", answer.messages);

    // The party table arrives while the user has no session open: the
    // server seats the party and its stock and queues them as a push.
    app.insert_resource(moly_server::Parties::fixture(&parties));
    for _ in 0..5 {
        app.update();
    }
    // The next request takes the push under its welcome.
    let client = std::thread::spawn(move || post(address, &[hello()]));
    let answer = drive(&mut app, client);
    assert_eq!(answer.status, 200);
    assert_eq!(kinds(&answer.messages)[0], "welcome");
    let pushed: Vec<&ServerMessage> = answer
        .messages
        .iter()
        .filter(|message| matches!(message, ServerMessage::Push { .. }))
        .collect();
    assert_eq!(pushed.len(), 1, "{:?}", answer.messages);
    match pushed[0] {
        ServerMessage::Push { kind, sections } => {
            assert_eq!(*kind, moly_protocol::PushKind::BirthdayPartySeat);
            // The party row the party master was seated with, and the stock
            // of its delivery item.
            let Some(parties) = &sections.delivery.parties else {
                panic!("the push carries no parties: {sections:?}");
            };
            assert_eq!(parties.len(), 1, "{parties:?}");
            assert_eq!(parties[0].birthday_party_id, 1);
            let materials = sections
                .delivery
                .materials
                .as_ref()
                .expect("the push carries the delivery item stock");
            assert!(materials.contains_key(&11), "{materials:?}");
        }
        other => panic!("not a push: {other:?}"),
    }
}

#[test]
fn another_path_and_another_method_are_refused() {
    let (_app, address) = server();
    let elsewhere = request(address, "POST", "/elsewhere", &[]);
    assert_eq!(elsewhere.status, 404);
    assert!(!elsewhere.reason.is_empty());
    let read = request(address, "GET", RPC_PATH, &[]);
    assert_eq!(read.status, 405);
    assert!(!read.reason.is_empty());
    let asked = request(address, "OPTIONS", RPC_PATH, &[]);
    assert_eq!(asked.status, 204);
}

#[test]
fn a_body_that_is_not_a_request_is_refused() {
    let (_app, address) = server();
    let answer = request(address, "POST", RPC_PATH, b"not a request");
    assert_eq!(answer.status, 400);
    assert!(!answer.reason.is_empty());
}
