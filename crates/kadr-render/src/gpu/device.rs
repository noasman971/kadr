//! Finding a graphics adapter and opening a device on it.

use crate::renderer::RenderError;

/// Environment variable forcing the software adapter (WARP on Windows, lavapipe
/// on Linux): set it to `software`. Used by continuous integration, which has no GPU.
pub const ADAPTER_ENVIRONMENT_VARIABLE: &str = "KADR_GPU_ADAPTER";

/// An open GPU device, with a readable description of its adapter.
pub(crate) struct OpenDevice {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) adapter_description: String,
}

/// Whether the environment asks for the software adapter.
fn software_adapter_requested() -> bool {
    std::env::var(ADAPTER_ENVIRONMENT_VARIABLE)
        .is_ok_and(|value| value.eq_ignore_ascii_case("software"))
}

/// Opens the most powerful adapter (or the software one when requested).
pub(crate) fn open_device() -> Result<OpenDevice, RenderError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: software_adapter_requested(),
        ..Default::default()
    }))
    .map_err(|error| RenderError::NoGraphicsAdapter {
        reason: error.to_string(),
    })?;

    let adapter_info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Kadr renderer"),
        // The smallest common limits, raised to the adapter's texture sizes so
        // 4K and 8K pictures fit.
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|error| RenderError::NoGraphicsAdapter {
        reason: format!("{} refused to open: {error}", adapter_info.name),
    })?;

    Ok(OpenDevice {
        device,
        queue,
        adapter_description: format!("GPU: {} ({:?})", adapter_info.name, adapter_info.backend),
    })
}
