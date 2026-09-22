//! Browser console diagnostics without an unbounded User Timing recording.
use bevy::log::tracing_subscriber::{
    filter::{filter_fn, EnvFilter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    Layer, Registry,
};

pub(crate) fn install() {
    let mut config = tracing_wasm::WASMLayerConfigBuilder::new();
    config.set_report_logs_in_timings(false);
    // tracing-wasm 0.2 records EVERY entered/exited span even with event
    // timings disabled. Bevy render spans otherwise leave millions of marks
    // and measures in Performance's entry buffer during a long-lived stage.
    // Console events retain Bevy's normal filter, including warnings/errors.
    let console = tracing_wasm::WASMLayer::new(config.build())
        .with_filter(filter_fn(|metadata| metadata.is_event()));
    Registry::default()
        .with(EnvFilter::new(format!("info,{}", bevy::log::DEFAULT_FILTER)))
        .with(console)
        .try_init()
        .expect("one browser tracing subscriber per runtime");
}
