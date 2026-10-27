//! Browser console diagnostics without an unbounded User Timing recording.
use bevy::log::tracing_subscriber::{
    filter::{filter_fn, EnvFilter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    Layer, Registry,
};

pub fn install() {
    let mut config = tracing_wasm::WASMLayerConfigBuilder::new();
    config.set_report_logs_in_timings(false);
    // tracing-wasm 0.2 records EVERY entered/exited span even with event
    // timings disabled. Bevy render spans otherwise leave millions of marks
    // and measures in Performance's entry buffer during a long-lived stage.
    // Production pages keep warnings/errors. Detailed diagnostics are opt-in
    // per document so a long-lived scene does not flood the user's console.
    let console = tracing_wasm::WASMLayer::new(config.build())
        .with_filter(filter_fn(|metadata| metadata.is_event()));
    let filter = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("log"))
        .filter(|value| matches!(value.as_str(), "error" | "warn" | "info" | "debug" | "trace"))
        .unwrap_or_else(|| "warn".to_owned());
    Registry::default()
        .with(EnvFilter::new(format!("{filter},{}", bevy::log::DEFAULT_FILTER)))
        .with(console)
        .try_init()
        .expect("one browser tracing subscriber per runtime");
}
