//! The link against a running server, and against no server at all.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use moly_link::{Arrival, LinkConfig, LinkPlugin, LinkState, ServerArrived, ServerLink};
use moly_protocol::{Call, CallKind, Reply, UserId};

#[derive(Resource, Default)]
struct Answers(Vec<ServerArrived>);

fn collect(mut arrived: MessageReader<ServerArrived>, mut answers: ResMut<Answers>) {
    answers.0.extend(arrived.read().cloned());
}

fn game(address: String) -> App {
    game_with_window(address, LinkConfig::CONNECT_WINDOW)
}

fn game_with_window(address: String, connect_window: Duration) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(LinkPlugin(LinkConfig {
            address,
            user: UserId::local(),
            client: "link test".to_owned(),
            connect_window,
        }))
        .init_resource::<Answers>()
        .add_systems(Update, collect);
    app
}

/// Updates both apps until `done` holds for the game.
fn run_until(game: &mut App, server: Option<&mut App>, done: impl Fn(&World) -> bool) {
    let mut server = server;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(game.world()) {
        assert!(Instant::now() < deadline, "timed out");
        if let Some(server) = server.as_deref_mut() {
            server.update();
        }
        game.update();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_join_is_answered_by_the_server() {
    let mut server = App::new();
    // The server needs its master tables before it answers; a test seats the
    // synthetic set instead of loading them through an asset source.
    server
        .insert_resource(moly_server::Masters::fixture())
        .add_plugins(MinimalPlugins)
        .add_plugins(moly_server::ServerPlugin);
    let address = moly_server::ws::listen("127.0.0.1:0", moly_server::hub(&server)).unwrap();
    let mut game = game(format!("ws://{address}"));
    run_until(&mut game, Some(&mut server), |world| {
        world.resource::<ServerLink>().state().is_open()
    });
    let seq = game.world_mut().resource_mut::<ServerLink>().call(Call::Join);
    run_until(&mut game, Some(&mut server), |world| {
        !world.resource::<Answers>().0.is_empty()
    });
    let answers = &game.world().resource::<Answers>().0;
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].seq, Some(seq));
    match &answers[0].event {
        Arrival::Reply { call, outcome, .. } => {
            assert_eq!(*call, CallKind::Join);
            assert_eq!(outcome, &Ok(Reply::Joined));
        }
        other => panic!("not a reply: {other:?}"),
    }
    assert_eq!(game.world().resource::<ServerLink>().waiting().count(), 0);
}

#[test]
fn without_a_server_the_link_is_lost_and_calls_are_refused_by_name() {
    // A port nothing listens on: bind, read the port, release it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut game = game_with_window(format!("ws://127.0.0.1:{port}"), Duration::from_millis(300));
    let seq = game.world_mut().resource_mut::<ServerLink>().call(Call::Join);
    run_until(&mut game, None, |world| {
        !world.resource::<Answers>().0.is_empty()
    });
    assert!(matches!(
        game.world().resource::<ServerLink>().state(),
        LinkState::Lost { .. }
    ));
    let answer = &game.world().resource::<Answers>().0[0];
    assert_eq!(answer.seq, Some(seq));
    match &answer.event {
        Arrival::Reply { outcome, .. } => {
            let refusal = outcome.as_ref().unwrap_err();
            assert_eq!(refusal.code, "link_lost");
            assert!(refusal.reason.contains("not reachable"), "{refusal}");
        }
        other => panic!("not a reply: {other:?}"),
    }
}

#[test]
fn a_server_that_starts_after_the_game_is_reached() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut game = game(format!("ws://127.0.0.1:{port}"));
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(500) {
        game.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*game.world().resource::<ServerLink>().state(), LinkState::Connecting);
    let mut server = App::new();
    server
        .insert_resource(moly_server::Masters::fixture())
        .add_plugins(MinimalPlugins)
        .add_plugins(moly_server::ServerPlugin);
    moly_server::ws::listen(&format!("127.0.0.1:{port}"), moly_server::hub(&server)).unwrap();
    run_until(&mut game, Some(&mut server), |world| {
        world.resource::<ServerLink>().state().is_open()
    });
}
