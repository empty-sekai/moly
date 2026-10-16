//! Native window entry. Browser hosts invoke `start_stage` or `start_game`.
//! WASM instantiation runs this main without starting a second event loop.
//! References retain the binding runtime and its exported entry points.

#[cfg(target_arch = "wasm32")]
fn main() {
    let _stage_referenced_not_called = moly_app::start_stage;
    let _game_referenced_not_called = moly_app::start_game;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if std::env::args().skip(1).any(|arg| matches!(arg.as_str(), "--version" | "-V")) {
        println!("moly {}", moly_game::VERSION);
        return;
    }
    moly_app::run();
}
