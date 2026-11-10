use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Everything that talks to the GPU: instance, surface, device, queue and the
/// swapchain configuration.
pub struct Gpu {
    /// Kept alive so the surface can be recreated later.
    #[allow(dead_code)]
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    /// Useful for querying capabilities in later milestones.
    #[allow(dead_code)]
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}

#[derive(Debug)]
pub enum GpuInitError {
    Surface(wgpu::CreateSurfaceError),
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    UnsupportedSurface,
}

impl std::fmt::Display for GpuInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(err) => write!(f, "failed to create surface: {err}"),
            Self::Adapter(err) => write!(f, "no suitable GPU adapter: {err}"),
            Self::Device(err) => write!(f, "failed to create device: {err}"),
            Self::UnsupportedSurface => write!(f, "surface is not supported by the adapter"),
        }
    }
}

impl std::error::Error for GpuInitError {}

impl Gpu {
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        // `Arc<Window>` is `'static`, so the surface outlives the window handle.
        let surface = instance
            .create_surface(window.clone())
            .map_err(GpuInitError::Surface)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(GpuInitError::Adapter)?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("engine device"),
                ..Default::default()
            })
            .await
            .map_err(GpuInitError::Device)?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(GpuInitError::UnsupportedSurface)?;

        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        log::info!(
            "GPU: {} ({:?}), surface {:?}",
            adapter.get_info().name,
            adapter.get_info().backend,
            config.format
        );

        Ok(Self {
            instance,
            surface,
            adapter,
            device,
            queue,
            config,
        })
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// Reconfigure the swapchain after a window resize. Zero-sized surfaces
    /// (minimized windows) are ignored until they come back.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        if size.width == self.config.width && size.height == self.config.height {
            return;
        }

        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Recreate the surface and swapchain, e.g. after `SurfaceStatus::Lost`.
    pub fn reconfigure(&self) {
        self.surface.configure(&self.device, &self.config);
    }
}
