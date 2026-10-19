//! The browser transport: `fetch`ed requests, one at a time, on the page's
//! event loop.
//!
//! Each request is a `POST` of the encoded message list as an `ArrayBuffer`
//! body with no `Content-Type` header, so the browser sends a simple request
//! that needs no preflight. The order of requests is the order of
//! [`crate::Schedule`]; time is the page's clock.

use std::time::Duration;

use futures_util::future::{self, Either};
use js_sys::{Promise, Uint8Array};
use moly_protocol::{decode, ServerMessage};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use crate::{LinkTransport, Next, Schedule, REQUEST_TIMEOUT};

/// The largest answer the link reads; the server's lists fit well below it.
const MAX_ANSWER: usize = 64 * 1024 * 1024;

/// Starts the transport on the page's event loop.
pub(crate) fn connect(transport: LinkTransport) {
    spawn_local(run(transport));
}

/// Milliseconds on the page's clock.
fn now_ms() -> f64 {
    js_sys::Date::now()
}

async fn run(transport: LinkTransport) {
    let url = transport.url();
    let started = now_ms();
    let elapsed = || Duration::from_secs_f64(((now_ms() - started) / 1000.0).max(0.0));
    let mut schedule = Schedule::new(transport.connect_window);
    loop {
        let call = match schedule.next(elapsed(), &transport) {
            Next::Stop => return,
            Next::Wait => {
                // Wake for the next call, or when the idle poll is due.
                let due = schedule.poll_due_in(elapsed());
                match future::select(Box::pin(transport.calls.recv()), Box::pin(sleep(due))).await {
                    Either::Left((Ok(call), _)) => Some(call),
                    Either::Left((Err(_), _)) => return,
                    Either::Right(_) => continue,
                }
            }
            Next::Sleep(duration) => {
                sleep(duration).await;
                continue;
            }
            Next::Poll => None,
            Next::Call(call) => Some(call),
        };
        let body = transport.request(call);
        let answer = match future::select(Box::pin(post(&url, body)), Box::pin(sleep(REQUEST_TIMEOUT))).await {
            Either::Left((answer, _)) => answer,
            Either::Right(_) => Err(format!("no answer within {} s", REQUEST_TIMEOUT.as_secs())),
        };
        if !schedule.answered(elapsed(), answer, &transport) {
            return;
        }
    }
}

/// Posts one request and reads its answer.
async fn post(url: &str, body: Vec<u8>) -> Result<Vec<ServerMessage>, String> {
    let window = web_sys::window().ok_or_else(|| "the page has no window".to_owned())?;
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);
    // No Content-Type header: a simple request needs no preflight.
    init.set_body(&JsValue::from(Uint8Array::from(body.as_slice()).buffer()));
    let response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|error| format!("the request failed: {}", describe(&error)))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| "the request did not return a response".to_owned())?;
    let status = response.status();
    let buffer = response
        .array_buffer()
        .map_err(|error| format!("the answer was not read: {}", describe(&error)))?;
    let bytes = JsFuture::from(buffer)
        .await
        .map_err(|error| format!("the answer was not read: {}", describe(&error)))?;
    let answer = Uint8Array::new(&bytes);
    if answer.length() as usize > MAX_ANSWER {
        return Err("the answer is larger than the link reads".to_owned());
    }
    let answer = answer.to_vec();
    if status != 200 {
        let reason = String::from_utf8_lossy(&answer);
        return Err(format!("status {status}: {}", reason.trim()));
    }
    decode::<Vec<ServerMessage>>(&answer).map_err(|error| format!("{error}"))
}

/// Resolves after `duration` by the page's timer.
async fn sleep(duration: Duration) {
    let ms = duration.as_millis().min(i32::MAX as u128) as i32;
    let wait = Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                resolve.unchecked_ref(),
                ms,
            );
        }
    });
    let _ = JsFuture::from(wait).await;
}

fn describe(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}
