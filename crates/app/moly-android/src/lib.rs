//! The Android entry point: the game, which connects to a server elsewhere.
//!
//! The APK carries no game data. The first start reads the public store's
//! publication document, picks the first resource snapshot the launcher would
//! install, and points the asset reader at that snapshot's store; the reader
//! keeps what it fetches in the app-private cache. The player's documents go
//! to the app's own data directory. The game reaches its server at the
//! address `MOLY_SERVER_URL` names in the process environment, else the one
//! the build names at `MOLY_SERVER_URL`, else
//! [`moly_link::PUBLIC_SERVER_URL`]. A build that names
//! `http://127.0.0.1:15800`, with `adb reverse tcp:15800 tcp:15800`, reaches
//! a server on the developer's machine.
//!
//! Every item is behind `target_os = "android"`: elsewhere the crate is an
//! empty library.
#![cfg(target_os = "android")]

/// The tag every message this entry point writes to logcat carries.
const LOGCAT_TAG: &[u8] = b"moly\0";

/// The entry point. `#[bevy_main]` exports it as the `android_main` symbol
/// Android's game activity calls, and runs it on the activity's own thread.
#[bevy::prelude::bevy_main]
fn main() {
    install_panic_hook();
    let app = bevy::android::ANDROID_APP
        .get()
        .cloned()
        .expect("the Android runtime passes the app before main runs");
    let internal = app
        .internal_data_path()
        .unwrap_or_else(|| panic!("the app data directory: Android reports none"));
    let data = internal.join("data");
    let cache = internal.join("asset-cache");
    for directory in [&data, &cache] {
        std::fs::create_dir_all(directory).unwrap_or_else(|error| {
            panic!("the app data directory {}: {error}", directory.display())
        });
    }
    std::env::set_var("MOLY_DATA_DIR", &data);
    std::env::set_var("MOLY_ASSET_CACHE", &cache);

    let publication = moly_store::install::fetch_publication(moly_store::DEFAULT_CDN)
        .unwrap_or_else(|error| panic!("the asset store publication: {error}"));
    // The launcher's default with nothing installed: the store view lists the
    // publication's installable snapshots in the document's order, and the
    // start button downloads and launches the first of them. The phone has
    // nothing installed, so that snapshot is the one it starts with.
    let snapshot = publication.installable().next().unwrap_or_else(|| {
        panic!(
            "the asset store publication: {} publishes no installable snapshot",
            moly_store::DEFAULT_CDN
        )
    });
    let store_url = snapshot
        .store_base(moly_store::DEFAULT_CDN, &publication.prefix)
        .unwrap_or_else(|error| panic!("the asset store address: {error}"));
    let catalog = snapshot
        .asset_catalog
        .clone()
        .unwrap_or_else(|| panic!("the asset store catalog: snapshot {} names none", snapshot.id));
    std::env::set_var("MOLY_ASSET_STORE_URL", &store_url);
    std::env::set_var("MOLY_ASSET_CATALOG", &catalog);

    // The game reads the address from the environment; a build that names
    // one gives it here.
    if std::env::var_os("MOLY_SERVER_URL").is_none() {
        std::env::set_var("MOLY_SERVER_URL", built_server_url());
    }

    // The game runs on this thread, as Bevy's own Android examples do.
    moly_app::run();
}

/// The server address the build named, else the default.
fn built_server_url() -> &'static str {
    match option_env!("MOLY_SERVER_URL") {
        Some(addr) if !addr.trim().is_empty() => addr.trim(),
        _ => moly_link::PUBLIC_SERVER_URL,
    }
}

/// Writes a panic to logcat before the default hook reports it, so a refusal
/// during startup is visible in `adb logcat` rather than lost on stderr.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("unexpected panic");
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_owned());
        logcat(&format!("panic at {location}: {message}"));
        previous(info);
    }));
}

/// One line at the error priority on the `moly` tag.
fn logcat(message: &str) {
    // A message may not hold a NUL byte; a logged panic keeps its text anyway.
    let text = std::ffi::CString::new(message.replace('\0', "\u{fffd}"))
        .expect("the message is NUL-free after the replacement");
    // SAFETY: both strings are NUL-terminated and live for the call; the log
    // call only reads them.
    unsafe {
        android_log_sys::__android_log_write(
            android_log_sys::LogPriority::ERROR as i32,
            LOGCAT_TAG.as_ptr().cast(),
            text.as_ptr(),
        );
    }
}
