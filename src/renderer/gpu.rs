use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Everything that talks to the GPU: instance, surface, device, queue and the
/// swapchain configuration.
pub struct Gpu {
    /// Kept alive so the surface can be recreated after loss.
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    /// Useful for querying capabilities in later milestones.
    #[allow(dead_code)]
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    window: Arc<Window>,
    suspended: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum ResizeAction {
    Suspend,
    Keep,
    Configure,
}

fn resize_action(
    configured: PhysicalSize<u32>,
    suspended: bool,
    requested: PhysicalSize<u32>,
) -> ResizeAction {
    if requested.width == 0 || requested.height == 0 {
        ResizeAction::Suspend
    } else if !suspended && requested == configured {
        ResizeAction::Keep
    } else {
        ResizeAction::Configure
    }
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
        let suspended = size.width == 0 || size.height == 0;
        if !suspended {
            surface.configure(&device, &config);
        }

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
            window,
            suspended,
        })
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// Reconfigure after a resize. A zero dimension suspends acquisition until
    /// a nonzero size is supplied, without discarding the last valid size.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let configured = PhysicalSize::new(self.config.width, self.config.height);
        match resize_action(configured, self.suspended, size) {
            ResizeAction::Suspend => self.suspended = true,
            ResizeAction::Keep => {}
            ResizeAction::Configure => {
                self.config.width = size.width;
                self.config.height = size.height;
                self.suspended = false;
                self.reconfigure();
            }
        }
    }

    /// Reconfigure the current surface. No acquired frame may still be live.
    pub fn reconfigure(&self) {
        if !self.suspended {
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub(crate) fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture;

        if self.suspended || self.window.is_minimized() == Some(true) {
            return None;
        }

        match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => Some(frame),
            CurrentSurfaceTexture::Suboptimal(frame) => {
                // Surface::configure forbids a live SurfaceTexture.
                drop(frame);
                self.reconfigure();
                None
            }
            CurrentSurfaceTexture::Outdated => {
                self.reconfigure();
                None
            }
            CurrentSurfaceTexture::Lost => {
                if let Err(error) = self.recreate_surface() {
                    log::warn!("could not recover lost surface: {error}");
                }
                None
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => None,
            CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error, skipping frame");
                None
            }
        }
    }

    fn recreate_surface(&mut self) -> Result<(), GpuInitError> {
        let surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(GpuInitError::Surface)?;
        let caps = surface.get_capabilities(&self.adapter);
        // Keep the render pipelines and depth target compatible with the new
        // surface. This recovers surface loss, not device or format changes.
        if !caps.formats.contains(&self.config.format)
            || !caps.alpha_modes.contains(&self.config.alpha_mode)
            || !caps.usages.contains(self.config.usage)
        {
            return Err(GpuInitError::UnsupportedSurface);
        }
        self.surface = surface;
        self.reconfigure();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_distinguishes_zero_size_duplicate_resize_and_same_size_restore() {
        let size = PhysicalSize::new(640, 480);
        for zero in [PhysicalSize::new(0, 480), PhysicalSize::new(640, 0)] {
            assert_eq!(resize_action(size, false, zero), ResizeAction::Suspend);
            assert_eq!(resize_action(size, true, zero), ResizeAction::Suspend);
        }
        assert_eq!(resize_action(size, false, size), ResizeAction::Keep);
        assert_eq!(resize_action(size, true, size), ResizeAction::Configure);
        assert_eq!(
            resize_action(size, false, PhysicalSize::new(800, 600)),
            ResizeAction::Configure
        );
    }
}
