//! Bounded, cancellable player-data transfers. Only one transfer allocates a
//! body buffer at a time; cancelled operations release their queue position.

use async_lock::Semaphore;
use futures_util::future::{AbortHandle, Abortable};
use moly_assets::player_data::MAX_IMPORT_BYTES;
use std::{future::Future, sync::OnceLock};

pub(crate) struct ReadTask(AbortHandle);

impl Drop for ReadTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn transfers() -> &'static Semaphore {
    static TRANSFERS: OnceLock<Semaphore> = OnceLock::new();
    TRANSFERS.get_or_init(|| Semaphore::new(1))
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(future: impl Future<Output = ()> + Send + 'static) -> Result<ReadTask, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .max_blocking_threads(2)
                .thread_name("moly-import")
                .enable_all()
                .build()
                .map_err(|error| format!("Start player-data reader: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (abort, registration) = AbortHandle::new_pair();
    runtime.spawn(async move {
        let _ = Abortable::new(future, registration).await;
    });
    Ok(ReadTask(abort))
}

#[cfg(target_arch = "wasm32")]
fn spawn(future: impl Future<Output = ()> + 'static) -> Result<ReadTask, String> {
    let (abort, registration) = AbortHandle::new_pair();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = Abortable::new(future, registration).await;
    });
    Ok(ReadTask(abort))
}

pub(crate) fn fetch(
    url: String,
    complete: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<ReadTask, String> {
    spawn(async move {
        let _permit = transfers().acquire().await;
        complete(
            platform::fetch(&url, MAX_IMPORT_BYTES)
                .await
                .and_then(decode_bytes),
        );
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file(
    path: std::path::PathBuf,
    complete: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<ReadTask, String> {
    spawn(async move {
        let _permit = transfers().acquire().await;
        complete(
            platform::read_file(&path, MAX_IMPORT_BYTES)
                .await
                .and_then(decode_bytes),
        );
    })
}

pub(crate) fn choose_file(
    complete: impl FnOnce(Result<Option<String>, String>) + Send + 'static,
) -> Result<ReadTask, String> {
    let selection = platform::pick_file()?;
    spawn(async move {
        let result = match selection.await {
            Ok(Some(file)) => {
                let _permit = transfers().acquire().await;
                platform::file(&file, MAX_IMPORT_BYTES)
                    .await
                    .and_then(decode_bytes)
                    .map(Some)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        complete(result);
    })
}

fn too_large() -> String {
    "Player data exceeds the 32 MiB import limit".into()
}

fn check_length(length: Option<u64>, limit: usize) -> Result<(), String> {
    if length.is_some_and(|length| length > limit as u64) {
        Err(too_large())
    } else {
        Ok(())
    }
}

fn status(code: u16) -> Result<(), String> {
    match code {
        200 => Ok(()),
        404 => Err("Player does not exist or their Mysekai home is unavailable (HTTP 404)".into()),
        401 | 403 => Err(format!("Player API denied access (HTTP {code})")),
        429 => Err("Player API is rate limited; try again later (HTTP 429)".into()),
        code => Err(format!("Player API returned HTTP {code}")),
    }
}

struct Body {
    bytes: Vec<u8>,
    limit: usize,
}

impl Body {
    fn new(limit: usize) -> Result<Self, String> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(limit.checked_add(1).ok_or_else(too_large)?)
            .map_err(|error| format!("Allocate player-data buffer: {error}"))?;
        Ok(Self { bytes, limit })
    }

    fn remaining(&self) -> usize {
        self.limit + 1 - self.bytes.len()
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        let count = self.remaining().min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        if count != bytes.len() || self.bytes.len() > self.limit {
            return Err(too_large());
        }
        Ok(())
    }
}

pub(crate) fn decode_bytes(bytes: Vec<u8>) -> Result<String, String> {
    check_length(Some(bytes.len() as u64), MAX_IMPORT_BYTES)?;
    let mut text =
        String::from_utf8(bytes).map_err(|_| "Player data must be UTF-8 JSON".to_owned())?;
    if text.starts_with('\u{feff}') {
        text.drain(..3);
    }
    Ok(text)
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use super::*;
    use tokio::io::AsyncReadExt;

    pub(super) fn pick_file(
    ) -> Result<impl Future<Output = Result<Option<rfd::FileHandle>, String>> + Send, String> {
        let selection = rfd::AsyncFileDialog::new()
            .add_filter("Mysekai player data", &["json"])
            .pick_file();
        Ok(async move { Ok(selection.await) })
    }

    pub(super) async fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|error| format!("Create player request: {error}"))?;
        let mut response = client
            .get(url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "identity")
            .send()
            .await
            .map_err(|error| format!("Fetch player data: {error}"))?;
        status(response.status().as_u16())?;
        check_length(response.content_length(), limit)?;
        let mut body = Body::new(limit)?;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Read player data: {error}"))?
        {
            body.push(&chunk)?;
        }
        Ok(body.bytes)
    }

    pub(super) async fn file(file: &rfd::FileHandle, limit: usize) -> Result<Vec<u8>, String> {
        read_file(file.path(), limit).await
    }

    pub(super) async fn read_file(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, String> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| format!("Open player data: {error}"))?;
        let metadata = file
            .metadata()
            .await
            .map_err(|error| format!("Inspect player data: {error}"))?;
        if !metadata.is_file() {
            return Err("Choose a player-data file".into());
        }
        check_length(Some(metadata.len()), limit)?;
        let mut body = Body::new(limit)?;
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let remaining = body.remaining().min(chunk.len());
            let length = file
                .read(&mut chunk[..remaining])
                .await
                .map_err(|error| format!("Read player data: {error}"))?;
            if length == 0 {
                break;
            }
            body.push(&chunk[..length])?;
        }
        Ok(body.bytes)
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{closure::Closure, JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        AbortController, ReadableStreamDefaultReader, Request, RequestInit, RequestRedirect,
        Response,
    };

    struct FilePicker {
        input: web_sys::HtmlInputElement,
        result: Option<futures_channel::oneshot::Receiver<Option<web_sys::File>>>,
        _change: Closure<dyn FnMut(web_sys::Event)>,
        cancel: Closure<dyn FnMut(web_sys::Event)>,
    }

    impl FilePicker {
        fn new() -> Result<Self, String> {
            let document = web_sys::window()
                .and_then(|window| window.document())
                .ok_or("Browser document is unavailable")?;
            let input: web_sys::HtmlInputElement = document
                .create_element("input")
                .map_err(|_| "Create file picker")?
                .dyn_into()
                .map_err(|_| "Create file picker input")?;
            input.set_id("moly-player-data-file");
            input.set_type("file");
            input.set_accept(".json");
            input.set_hidden(true);
            let (send, result) = futures_channel::oneshot::channel();
            let send = Rc::new(RefCell::new(Some(send)));
            let change_send = send.clone();
            let selected = input.clone();
            let change = Closure::wrap(Box::new(move |_: web_sys::Event| {
                if let Some(send) = change_send.borrow_mut().take() {
                    let file = selected.files().and_then(|files| files.get(0));
                    let _ = send.send(file);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let cancel = Closure::wrap(Box::new(move |_: web_sys::Event| {
                if let Some(send) = send.borrow_mut().take() {
                    let _ = send.send(None);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            input.set_onchange(Some(change.as_ref().unchecked_ref()));
            input
                .add_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref())
                .map_err(|_| "Listen for file cancellation")?;
            let picker = Self {
                input,
                result: Some(result),
                _change: change,
                cancel,
            };
            document
                .body()
                .ok_or("Browser body is unavailable")?
                .append_child(&picker.input)
                .map_err(|_| "Attach file picker")?;
            picker.input.click();
            Ok(picker)
        }

        async fn finish(mut self) -> Result<Option<web_sys::File>, String> {
            self.result
                .take()
                .expect("one file selection")
                .await
                .map_err(|_| "File selection was interrupted".into())
        }
    }

    impl Drop for FilePicker {
        fn drop(&mut self) {
            self.input.set_onchange(None);
            let _ = self.input.remove_event_listener_with_callback(
                "cancel",
                self.cancel.as_ref().unchecked_ref(),
            );
            self.input.remove();
        }
    }

    pub(super) fn pick_file(
    ) -> Result<impl Future<Output = Result<Option<web_sys::File>, String>>, String> {
        Ok(FilePicker::new()?.finish())
    }

    struct RequestGuard {
        window: web_sys::Window,
        controller: AbortController,
        timer: i32,
        _timeout: Closure<dyn FnMut()>,
    }

    impl RequestGuard {
        fn new() -> Result<Self, String> {
            let window = web_sys::window().ok_or("Browser window is unavailable")?;
            let controller = AbortController::new()
                .map_err(|error| format!("Create player request: {error:?}"))?;
            let abort = controller.clone();
            let timeout = Closure::wrap(Box::new(move || abort.abort()) as Box<dyn FnMut()>);
            let timer = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    timeout.as_ref().unchecked_ref(),
                    60_000,
                )
                .map_err(|error| format!("Set player request timeout: {error:?}"))?;
            Ok(Self {
                window,
                controller,
                timer,
                _timeout: timeout,
            })
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
            let reader = self.0.clone();
            let cancelled = reader.cancel();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(cancelled).await;
                reader.release_lock();
            });
        }
    }

    async fn read_stream(stream: web_sys::ReadableStream, limit: usize) -> Result<Vec<u8>, String> {
        let reader = StreamGuard(
            stream
                .get_reader()
                .dyn_into()
                .map_err(|_| "Player data has no readable byte stream")?,
        );
        let mut body = Body::new(limit)?;
        loop {
            let chunk = JsFuture::from(reader.0.read())
                .await
                .map_err(|error| format!("Read player data: {error:?}"))?;
            if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
                .ok()
                .and_then(|value| value.as_bool())
                == Some(true)
            {
                break;
            }
            let bytes: js_sys::Uint8Array =
                js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
                    .map_err(|_| "Player data stream has no bytes")?
                    .dyn_into()
                    .map_err(|_| "Player data stream is not binary")?;
            let count = body.remaining().min(bytes.length() as usize);
            let offset = body.bytes.len();
            body.bytes.resize(offset + count, 0);
            bytes
                .subarray(0, count as u32)
                .copy_to(&mut body.bytes[offset..]);
            if count != bytes.length() as usize || body.bytes.len() > limit {
                return Err(too_large());
            }
        }
        Ok(body.bytes)
    }

    pub(super) async fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let guard = RequestGuard::new()?;
        let init = RequestInit::new();
        init.set_redirect(RequestRedirect::Error);
        init.set_signal(Some(&guard.controller.signal()));
        let request = Request::new_with_str_and_init(url, &init)
            .map_err(|error| format!("Invalid player request: {error:?}"))?;
        request
            .headers()
            .set("Accept", "application/json")
            .map_err(|error| format!("Set player request headers: {error:?}"))?;
        let response: Response = JsFuture::from(guard.window.fetch_with_request(&request))
            .await
            .map_err(|error| format!("Fetch player data: {error:?}"))?
            .dyn_into()
            .map_err(|_| "Invalid player response")?;
        status(response.status())?;
        let length = response
            .headers()
            .get("content-length")
            .ok()
            .flatten()
            .and_then(|value| value.parse().ok());
        check_length(length, limit)?;
        let Some(body) = response.body() else {
            return Ok(Vec::new());
        };
        read_stream(body, limit).await
    }

    pub(super) async fn file(file: &web_sys::File, limit: usize) -> Result<Vec<u8>, String> {
        read_file(file, limit).await
    }

    pub(super) async fn read_file(file: &web_sys::File, limit: usize) -> Result<Vec<u8>, String> {
        let size = file.size();
        if !size.is_finite() || size < 0.0 || size > limit as f64 {
            return Err(too_large());
        }
        read_stream(file.stream(), limit).await
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
