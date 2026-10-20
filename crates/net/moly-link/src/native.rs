//! The native transport: one HTTP request at a time on its own thread.
//!
//! The order of requests is the order of [`crate::Schedule`]: polls until the
//! first welcome, then the queued calls in order, and a poll whenever no
//! request has been made for [`crate::POLL_INTERVAL`].

use std::time::{Duration, Instant};

use bevy::log::error;
use moly_protocol::{decode, ServerMessage};

use crate::{Endpoint, LinkTransport, Next, Schedule, REQUEST_TIMEOUT};

/// The largest answer the link reads; the server's lists fit well below it.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

/// How often an idle transport looks for a queued call.
const IDLE_TICK: Duration = Duration::from_millis(5);

/// Starts the transport thread.
pub(crate) fn connect(transport: LinkTransport) {
    let spawned = std::thread::Builder::new()
        .name("moly-link".to_owned())
        .spawn(move || run(transport));
    if let Err(spawn_error) = spawned {
        error!("[link] the link thread did not start: {spawn_error}");
    }
}

fn run(transport: LinkTransport) {
    let agent = ureq::Agent::config_builder()
        // The trust every native HTTPS client of the game uses; a status is
        // read here rather than failing the request.
        .tls_config(moly_store::tls::tls_config())
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(concat!("moly-link/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    let url = transport.url();
    let started = Instant::now();
    let mut schedule = Schedule::new(transport.connect_window);
    loop {
        let call = match schedule.next(started.elapsed(), &transport) {
            Next::Stop => return,
            Next::Wait => {
                std::thread::sleep(IDLE_TICK);
                continue;
            }
            Next::Sleep(duration) => {
                std::thread::sleep(duration);
                continue;
            }
            Next::Poll => None,
            Next::Call(call) => Some(call),
        };
        let body = transport.request(call);
        let answer = match &transport.endpoint {
            Endpoint::Http(_) => post(&agent, &url, body),
            // The app hosting the server answers in its own updates; this
            // thread waits for them.
            #[cfg(feature = "local")]
            Endpoint::Local(hub) => bevy::tasks::block_on(hub.exchange(&body))
                .map_err(|error| error.to_string()),
        };
        if !schedule.answered(started.elapsed(), answer, &transport) {
            return;
        }
    }
}

/// Posts one request and reads its answer.
fn post(agent: &ureq::Agent, url: &str, body: Vec<u8>) -> Result<Vec<ServerMessage>, String> {
    // The body is sent as it is, with no Content-Type header.
    let mut response = agent
        .post(url)
        .send(&body[..])
        .map_err(|send_error| format!("{send_error}"))?;
    let status = response.status().as_u16();
    let answer = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|read_error| format!("the answer was not read: {read_error}"))?;
    if status != 200 {
        let reason = String::from_utf8_lossy(&answer);
        return Err(format!("status {status}: {}", reason.trim()));
    }
    decode::<Vec<ServerMessage>>(&answer).map_err(|decode_error| format!("{decode_error}"))
}
