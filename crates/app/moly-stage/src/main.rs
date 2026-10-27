//! WASM exports start the stage only after browser preflight and a gesture.
#[cfg(target_arch = "wasm32")]
fn main() {
    let _entry_referenced_not_called = moly_stage::start_stage;
}
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if !std::env::args().any(|arg| arg == "--check-systems") {
        eprintln!("moly-stage is an embedded browser runtime; --check-systems audits its assembly");
        return;
    }
    // Read-only assembly audit using the same source admission and field
    // plugins. It never starts a visible interactive window or writes saves.
    std::env::set_var("MOLY_WINDOW_HIDDEN", "1");
    let source = moly_assets::asset_source::resolve().expect("stage asset source");
    let remote = moly_assets::asset_source::resolve_remote().expect("stage remote sources");
    let site = moly_features::site_request::resolve().expect("stage site request");
    let mut app = moly_stage::app(source, site);
    moly_assets::remote::admit(&mut app, remote);
    app.add_plugins(moly_link::LocalLinkPlugin {
        user: moly_protocol::UserId::local(),
        client: format!("moly-stage audit {}", moly_stage::VERSION),
    });
    let (count, errors) = moly_core::system_access::conflicts(&mut app);
    for error in &errors {
        eprintln!("[stage-system-access] {error}");
    }
    eprintln!(
        "[stage-system-access] {count} systems, {} rejected",
        errors.len()
    );
    std::process::exit(if errors.is_empty() { 0 } else { 1 });
}
