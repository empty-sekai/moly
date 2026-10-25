//! Application feature selection over the shared browser renderer setup.
pub(crate) fn settings(backend: &str) -> Result<bevy::render::settings::WgpuSettings, String> {
    moly_core::render_backend::settings(backend, cfg!(feature = "webgpu"))
}
