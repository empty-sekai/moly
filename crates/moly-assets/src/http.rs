//! Stream bounded browser asset responses, with contextual errors and an
//! abort guard that covers both the HTTP request and response consumption.
use crate::read_limits::{self, Budget, Buffer};
use bevy::asset::io::AssetReaderError;
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, ReadableStreamDefaultReader, Request, RequestCredentials, RequestInit,
    RequestMode, RequestRedirect, Response, ResponseType,
};

fn error(message: impl Into<String>) -> AssetReaderError {
    std::io::Error::other(message.into()).into()
}
fn js_error(value: &JsValue) -> String {
    let field = |name| {
        js_sys::Reflect::get(value, &JsValue::from_str(name))
            .ok()
            .and_then(|v| v.as_string())
    };
    match (field("name"), field("message")) {
        (Some(name), Some(message)) => format!("{name}: {message}"),
        (_, Some(message)) => message,
        _ => value.as_string().unwrap_or_else(|| format!("{value:?}")),
    }
}
struct RequestGuard {
    controller: AbortController,
    window: web_sys::Window,
    timer: i32,
    _callback: Closure<dyn FnMut()>,
}
impl RequestGuard {
    fn new() -> Result<Self, AssetReaderError> {
        let window =
            web_sys::window().ok_or_else(|| error("HTTP assets require a browser window"))?;
        let controller = AbortController::new()
            .map_err(|e| error(format!("Create asset request: {}", js_error(&e))))?;
        let abort = controller.clone();
        let callback = Closure::wrap(Box::new(move || abort.abort()) as Box<dyn FnMut()>);
        let timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                60_000,
            )
            .map_err(|e| error(format!("Set asset timeout: {}", js_error(&e))))?;
        Ok(Self {
            controller,
            window,
            timer,
            _callback: callback,
        })
    }
    fn failure(&self, stage: &str, path: &str, cause: &JsValue) -> AssetReaderError {
        if self.controller.signal().aborted() {
            error(format!(
                "Asset HTTP timeout after 60 seconds ({stage}): {path}"
            ))
        } else {
            error(format!(
                "Asset HTTP {stage} failed for {path}: {}",
                js_error(cause)
            ))
        }
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.timer);
        self.controller.abort();
    }
}
struct StreamGuard(ReadableStreamDefaultReader);
impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.release_lock();
    }
}

async fn response(root: &str, path: &str) -> Result<(RequestGuard, Response), AssetReaderError> {
    let guard = RequestGuard::new()?;
    let url = format!("{root}{}", crate::http_path::encode_path(path));
    let init = RequestInit::new();
    // Root admission is owned by moly-app. Public assets may use the exact
    // configured CDN; cookies and authorization never accompany asset reads.
    init.set_mode(RequestMode::Cors);
    init.set_credentials(RequestCredentials::Omit);
    init.set_redirect(RequestRedirect::Error);
    init.set_signal(Some(&guard.controller.signal()));
    let request = Request::new_with_str_and_init(&url, &init)
        .map_err(|e| error(format!("Invalid asset URL {url}: {}", js_error(&e))))?;
    let response: Response = JsFuture::from(guard.window.fetch_with_request(&request))
        .await
        .map_err(|e| guard.failure("request", path, &e))?
        .dyn_into()
        .map_err(|_| error(format!("Invalid HTTP response for {path}")))?;
    match response.status() {
        200 => Ok((guard, response)),
        403 | 404 => Err(AssetReaderError::NotFound(path.into())),
        status => Err(error(format!("Asset HTTP {status}: {path}"))),
    }
}
fn bound(response: &Response, path: &str, limit: usize) -> Result<usize, AssetReaderError> {
    let length = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok());
    let encoding = response.headers().get("content-encoding").ok().flatten();
    crate::http_path::response_bound(
        length,
        encoding.as_deref(),
        response.type_() == ResponseType::Cors,
        limit,
    )
    .map_err(|cause| error(format!("{cause}: {path}")))
}
async fn stream(
    response: Response,
    guard: &RequestGuard,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, AssetReaderError> {
    // A loose thumbnail must not allocate MAX_ASSET_BYTES merely because that
    // is its safety ceiling. Grow only for bytes actually received, retaining
    // a capacity ceiling even as larger bodies grow geometrically.
    let mut bytes = read_limits::buffer(limit.min(64 * 1024))?;
    let Some(body) = response.body() else {
        return Ok(bytes);
    };
    let reader = StreamGuard(
        body.get_reader()
            .dyn_into::<ReadableStreamDefaultReader>()
            .map_err(|_| error(format!("Could not open asset response stream: {path}")))?,
    );
    loop {
        let chunk = JsFuture::from(reader.0.read())
            .await
            .map_err(|e| guard.failure("stream", path, &e))?;
        if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let chunk: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(|_| error(format!("Asset stream has no bytes: {path}")))?
            .dyn_into()
            .map_err(|_| error(format!("Asset stream is not binary: {path}")))?;
        let end = bytes
            .len()
            .checked_add(chunk.length() as usize)
            .filter(|end| *end <= limit)
            .ok_or_else(|| {
                error(format!(
                    "Asset HTTP response exceeds its byte limit: {path}"
                ))
            })?;
        if end > bytes.capacity() {
            let capacity = bytes.capacity().saturating_mul(2).max(end).min(limit);
            bytes
                .try_reserve_exact(capacity - bytes.len())
                .map_err(|_| error(format!("Could not allocate bounded asset response: {path}")))?;
        }
        let start = bytes.len();
        bytes.resize(end, 0);
        chunk.copy_to(&mut bytes[start..]);
    }
    Ok(bytes)
}

/// Packed callers already own a representation-specific buffer reservation.
pub(crate) async fn read_bytes(
    root: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, AssetReaderError> {
    let (guard, response) = response(root, path).await?;
    let limit = bound(&response, path, limit)?;
    stream(response, &guard, path, limit).await
}

/// Loose assets have no manifest size. Reserve their actual Content-Length
/// before reading the body, or the declared safety bound if length is unknown.
/// SharedReader releases the reservation before the loader awaits dependencies.
pub(crate) async fn read_budgeted(
    root: &str,
    path: &str,
    limit: usize,
    budget: &Arc<Budget>,
) -> Result<Buffer, AssetReaderError> {
    let (guard, response) = response(root, path).await?;
    let limit = bound(&response, path, limit)?;
    let mut reservation = budget.reserve(limit).await?;
    let bytes = stream(response, &guard, path, limit).await?;
    reservation.shrink_to(bytes.capacity());
    Ok(Buffer {
        bytes,
        _reservation: reservation,
    })
}
