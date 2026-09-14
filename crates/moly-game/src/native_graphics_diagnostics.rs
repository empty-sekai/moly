//! Record native device loss before a later presentation error hides its origin.
//!
//! Registered after DefaultPlugins: RenderPlugin::finish installs RenderDevice
//! before this hook, and gameplay RenderStartup has not allocated resources yet.
//! This observes failure only; it does not change the backend or recover a device.

use bevy::{prelude::*, render::renderer::RenderDevice};
use std::{
    backtrace::Backtrace,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) struct NativeGraphicsDiagnosticsPlugin;

impl Plugin for NativeGraphicsDiagnosticsPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        let device = app.world().resource::<RenderDevice>().wgpu_device();
        let process_id = std::process::id();
        let installed_at = Instant::now();

        // wgpu 27 does not pass core DeviceLost errors to the ordinary error
        // handler; it expects this callback to surface them. Do not access ECS
        // or GPU state, submit work, recover the device, or panic in this hook.
        device.set_device_lost_callback(move |reason, message| {
            let unix_ms = unix_millis();
            let elapsed_ms = installed_at.elapsed().as_millis();
            let backtrace = Backtrace::force_capture();
            eprintln!(
                "[native-graphics/device-lost] unix_ms={unix_ms} elapsed_ms={elapsed_ms} pid={process_id} reason={reason:?}\nmessage={message}\n{backtrace}"
            );
        });

        // Preserve wgpu's default fatal handling of uncaptured validation,
        // OOM and internal errors. Device loss is a separate reporting path.
        eprintln!(
            "[native-graphics/installed] unix_ms={} pid={process_id} uncaptured_error_handler=unchanged",
            unix_millis()
        );
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
