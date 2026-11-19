//! Shared helpers for renderer tests.
//!
//! GPU tests are skipped when no adapter/device is available so the headless
//! quality gate can run on machines without graphics hardware.

/// Request a test adapter and device, or `None` when none is available.
pub(crate) fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    test_device_with_features(wgpu::Features::empty())
}

pub(crate) fn test_device_with_features(
    required_features: wgpu::Features,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    if !adapter.features().contains(required_features) {
        return None;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue))
}

/// Log that a GPU test was skipped because the host has no usable adapter.
pub(crate) fn skip_without_adapter() {
    eprintln!("skipping GPU test: no wgpu adapter/device available");
}
