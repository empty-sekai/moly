//! Stream bounded pack responses; do not materialize an unbounded ArrayBuffer.

use bevy::asset::io::AssetReaderError;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, ReadableStreamDefaultReader, Request, RequestInit, RequestMode,
    RequestRedirect, Response,
};

fn error(message: impl Into<String>) -> AssetReaderError {
    std::io::Error::other(message.into()).into()
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
            web_sys::window().ok_or_else(|| error("HTTP asset reads require a browser window"))?;
        let controller =
            AbortController::new().map_err(|e| error(format!("Create asset request: {e:?}")))?;
        let abort = controller.clone();
        let callback = Closure::wrap(Box::new(move || abort.abort()) as Box<dyn FnMut()>);
        let timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                60_000,
            )
            .map_err(|e| error(format!("Set asset request timeout: {e:?}")))?;
        Ok(Self {
            controller,
            window,
            timer,
            _callback: callback,
        })
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.timer);
        self.controller.abort();
    }
}

pub(crate) async fn read_bytes(
    root: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, AssetReaderError> {
    let guard = RequestGuard::new()?;
    let encoded = path
        .split('/')
        .map(|part| String::from(js_sys::encode_uri_component(part)))
        .collect::<Vec<_>>()
        .join("/");
    let url = format!("{root}{encoded}");
    let init = RequestInit::new();
    init.set_mode(RequestMode::SameOrigin);
    init.set_redirect(RequestRedirect::Error);
    init.set_signal(Some(&guard.controller.signal()));
    let request = Request::new_with_str_and_init(&url, &init)
        .map_err(|e| error(format!("Invalid asset request: {e:?}")))?;
    let response: Response = JsFuture::from(guard.window.fetch_with_request(&request))
        .await
        .map_err(|e| error(format!("Asset HTTP request failed: {e:?}")))?
        .dyn_into()
        .map_err(|_| error("Asset HTTP response is invalid"))?;
    match response.status() {
        200 => {}
        403 | 404 => return Err(AssetReaderError::NotFound(path.into())),
        status => return Err(AssetReaderError::HttpError(status)),
    }
    if response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|bytes| bytes > limit as u64)
    {
        return Err(error(format!(
            "Asset HTTP response exceeds its byte limit: {path}"
        )));
    }
    let mut bytes = crate::read_limits::buffer(
        limit
            .checked_add(1)
            .ok_or_else(|| error("Asset limit overflow"))?,
    )?;
    let Some(body) = response.body() else {
        return Ok(bytes);
    };
    let reader: ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| error("Could not open asset response stream"))?;
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| error(format!("Read asset response: {e:?}")))?;
        if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let chunk: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(|_| error("Asset stream has no bytes"))?
            .dyn_into()
            .map_err(|_| error("Asset stream is not binary"))?;
        let end = bytes
            .len()
            .checked_add(chunk.length() as usize)
            .filter(|end| *end <= limit)
            .ok_or_else(|| {
                error(format!(
                    "Asset HTTP response exceeds its byte limit: {path}"
                ))
            })?;
        let start = bytes.len();
        bytes.resize(end, 0);
        chunk.copy_to(&mut bytes[start..]);
    }
    reader.release_lock();
    Ok(bytes)
}
