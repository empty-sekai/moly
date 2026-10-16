use bevy::{
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
};

#[derive(Message)]
pub(crate) struct CaptureFrame;

pub(crate) fn capture(mut commands: Commands, mut requests: MessageReader<CaptureFrame>) {
    if requests.read().next().is_none() {
        return;
    }
    requests.clear();
    #[cfg(not(target_arch = "wasm32"))]
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_millis();
    #[cfg(target_arch = "wasm32")]
    let timestamp = js_sys::Date::now() as u128;
    let filename = format!("moly-{timestamp}.png");
    info!("Capturing rendered frame: {filename}");
    #[cfg(not(target_arch = "wasm32"))]
    let output = {
        let directory = std::env::var_os("MOLY_SCREENSHOT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "screenshots".into());
        if let Err(error) = std::fs::create_dir_all(&directory) {
            error!("Cannot create screenshot directory: {error}");
            return;
        }
        directory.join(filename)
    };
    #[cfg(target_arch = "wasm32")]
    let output = std::path::PathBuf::from(filename);
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(output));
}
