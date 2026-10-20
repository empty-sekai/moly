//! The native transport: an HTTP/1.1 endpoint on its own thread.
//!
//! One request is one session. `POST /rpc` takes the encoded request list,
//! opens a session at the hub, reports the request's messages as frames, and
//! reads the session's outgoing frames until the terminal one arrives: the
//! reply numbered like the request's call, the refusal, or the welcome of a
//! request without a call. It then takes whatever is already queued, closes
//! the session, and answers with the encoded list. A request that gets no
//! terminal frame in [`ANSWER_TIMEOUT`] answers 504. Any other path answers
//! 404, any other method 405, and a body that is not a request list 400.
//!
//! The endpoint speaks plain HTTP/1.1 with keep-alive; TLS and HTTP/3 are a
//! reverse proxy's job in deployment. It runs on one thread hosting an
//! `async-executor` executor over `async-io`, so the server adds no async
//! runtime of its own.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use async_io::Async;
use bevy::log::{debug, warn};
use futures_util::future::{self, Either};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use moly_protocol::{encode, ServerMessage, RPC_PATH};
use smol_hyper::rt::{FuturesIo, SmolTimer};

use crate::hub::{ExchangeError, Hub};

/// How long a request waits for its session's terminal frame.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Binds `address` now (so a taken port fails here, not later) and serves
/// requests on a background thread until the process ends. Returns the bound
/// address, which names the port when `address` asked for port 0.
pub fn listen(address: &str, hub: Hub) -> std::io::Result<SocketAddr> {
    let listener = std::net::TcpListener::bind(address)?;
    let local = listener.local_addr()?;
    std::thread::Builder::new()
        .name("moly-server-http".to_owned())
        .spawn(move || {
            // One executor runs every request of every connection; each
            // accepted connection waits for its request and starts the next.
            let executor = async_executor::Executor::new();
            futures_lite::future::block_on(executor.run(async {
                accept(listener, hub, &executor).await
            }));
        })?;
    Ok(local)
}

/// Accepts connections and gives each to the executor.
async fn accept(
    listener: std::net::TcpListener,
    hub: Hub,
    executor: &async_executor::Executor<'_>,
) {
    // `async-io` puts the listener in non-blocking mode and registers it.
    let listener = match Async::new(listener) {
        Ok(listener) => listener,
        Err(error) => {
            warn!("[server] the HTTP listener is not watchable: {error}");
            return;
        }
    };
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let hub = hub.clone();
                executor.spawn(connection(stream, hub)).detach();
            }
            Err(error) => {
                // A connection that failed to arrive does not end the listener.
                warn!("[server] accept failed: {error}");
                async_io::Timer::after(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Serves the requests of one connection until it ends.
async fn connection(stream: Async<std::net::TcpStream>, hub: Hub) {
    let _ = stream.get_ref().set_nodelay(true);
    let service = service_fn(move |request: Request<Incoming>| {
        let hub = hub.clone();
        async move { Ok::<_, Infallible>(answer(request, &hub).await) }
    });
    let served = http1::Builder::new()
        .timer(SmolTimer::new())
        .serve_connection(FuturesIo::new(stream), service)
        .await;
    if let Err(error) = served {
        debug!("[server] a connection ended: {error}");
    }
}

/// Answers one request.
async fn answer(request: Request<Incoming>, hub: &Hub) -> Response<Full<Bytes>> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if path != RPC_PATH {
        return match method {
            Method::OPTIONS => options(StatusCode::NOT_FOUND),
            _ => plain(StatusCode::NOT_FOUND, &format!("no endpoint at {path}")),
        };
    }
    match method {
        Method::POST => call(request, hub).await,
        // A browser asks before a request it cannot send without asking.
        Method::OPTIONS => options(StatusCode::NO_CONTENT),
        _ => plain(StatusCode::METHOD_NOT_ALLOWED, "the endpoint takes POST"),
    }
}

/// The answer to `OPTIONS`: what a browser needs before a request.
fn options(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", static_header("*"));
    headers.insert("access-control-allow-methods", static_header("POST"));
    headers.insert("access-control-max-age", static_header("86400"));
    // A browser posts bytes it reads back as bytes.
    headers.insert(
        "access-control-expose-headers",
        static_header("content-type"),
    );
    response
}

/// A plain-text answer with a reason.
fn plain(status: StatusCode, reason: &str) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::copy_from_slice(reason.as_bytes())));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", static_header("*"));
    headers.insert("cache-control", static_header("no-store"));
    headers.insert(CONTENT_TYPE, static_header("text/plain; charset=utf-8"));
    response
}

/// A protocol answer: the encoded messages.
fn protocol(messages: Vec<ServerMessage>) -> Response<Full<Bytes>> {
    let body = encode(&messages);
    let mut response = Response::new(Full::new(Bytes::from(body)));
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", static_header("*"));
    headers.insert("cache-control", static_header("no-store"));
    headers.insert(CONTENT_TYPE, static_header("application/octet-stream"));
    response
}

fn static_header(value: &'static str) -> HeaderValue {
    HeaderValue::from_static(value)
}

/// Reads a request body whole; it is small enough to keep in memory.
async fn call(request: Request<Incoming>, hub: &Hub) -> Response<Full<Bytes>> {
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return plain(
                StatusCode::BAD_REQUEST,
                &format!("the body was not read: {error}"),
            );
        }
    };
    let answered = future::select(
        Box::pin(hub.exchange(&body)),
        Box::pin(async_io::Timer::after(ANSWER_TIMEOUT)),
    )
    .await;
    let messages = match answered {
        Either::Left((Ok(messages), _)) => messages,
        Either::Left((Err(ExchangeError::Request(error)), _)) => {
            return plain(StatusCode::BAD_REQUEST, &error);
        }
        Either::Left((Err(ExchangeError::Server(error)), _)) => {
            return plain(StatusCode::INTERNAL_SERVER_ERROR, &error);
        }
        Either::Right(_) => {
            let reason = format!(
                "the server sent no answer within {} s",
                ANSWER_TIMEOUT.as_secs()
            );
            return plain(StatusCode::GATEWAY_TIMEOUT, &reason);
        }
    };
    protocol(messages)
}
