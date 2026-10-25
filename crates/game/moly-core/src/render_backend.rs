//! Consume the backend selected by the browser's asynchronous preflight.

use bevy::render::settings::{Backends, WgpuLimits, WgpuSettings};

pub fn settings(backend: &str, supports_webgpu: bool) -> Result<WgpuSettings, String> {
    if (backend == "webgpu") != supports_webgpu {
        return Err("The selected renderer requires its matching browser module".into());
    }
    match backend {
        "webgpu" => Ok(WgpuSettings {
            backends: Some(Backends::BROWSER_WEBGPU),
            ..Default::default()
        }),
        "webgl2" => Ok(WgpuSettings {
            backends: Some(Backends::GL),
            limits: WgpuLimits::downlevel_webgl2_defaults(),
            ..Default::default()
        }),
        _ => Err("A preflighted webgpu or webgl2 backend is required".into()),
    }
}
