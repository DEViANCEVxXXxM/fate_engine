use std::sync::Arc;

use fate_engine::renderer::Gpu;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::ui::{DebugUi, theme};
use crate::{Command, DebugModel, Page};

/// A second native window on the host's winit event loop.
/// Closing it does not close the host. UI rendering has its own GPU context,
/// so its work is excluded from the host renderer's pass timestamps.
pub struct DebugWindow {
    window: Arc<Window>,
    gpu: Gpu,
    context: egui::Context,
    input: egui_winit::State,
    painter: egui_wgpu::Renderer,
    ui: DebugUi,
}

impl DebugWindow {
    pub async fn new(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn std::error::Error>> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Fate / Diagnostics")
                    .with_theme(Some(winit::window::Theme::Dark))
                    .with_inner_size(LogicalSize::new(1160.0, 800.0))
                    .with_min_inner_size(LogicalSize::new(720.0, 520.0)),
            )?,
        );
        dark_caption(&window);
        let mut gpu = Gpu::new(window.clone()).await?;
        // egui blends in gamma space. Use the corresponding non-sRGB swapchain
        // format if supported rather than paying for an intermediate texture.
        let format = gpu.config.format.remove_srgb_suffix();
        if gpu
            .surface
            .get_capabilities(&gpu.adapter)
            .formats
            .contains(&format)
        {
            gpu.config.format = format;
            gpu.reconfigure();
        }
        let context = egui::Context::default();
        theme(&context);
        let input = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let painter = egui_wgpu::Renderer::new(&gpu.device, gpu.config.format, Default::default());
        Ok(Self {
            window,
            gpu,
            context,
            input,
            painter,
            ui: DebugUi::default(),
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Native window access for host placement and lifecycle handling.
    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn request_redraw(&self) {
        if self.window.is_minimized() != Some(true) {
            self.window.request_redraw();
        }
    }

    /// Feed only events belonging to `id()`. Returns true on close request.
    pub fn on_event(&mut self, event: &WindowEvent) -> bool {
        let _ = self.input.on_window_event(&self.window, event);
        if let WindowEvent::Resized(size) = event {
            self.gpu.resize(*size);
        }
        matches!(event, WindowEvent::CloseRequested)
    }

    pub fn select_page(&mut self, page: Page) {
        self.ui.page = page;
    }

    pub fn draw(&mut self, model: &mut DebugModel) -> Vec<Command> {
        let Some(frame) = self.gpu.acquire_frame() else {
            return Vec::new();
        };
        let raw = self.input.take_egui_input(&self.window);
        let mut commands = Vec::new();
        let mut output = self.context.run_ui(raw, |root| {
            self.ui.show(root, model, &mut commands);
        });
        self.input
            .handle_platform_output(&self.window, output.platform_output);
        let jobs = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.gpu.config.width, self.gpu.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                self.painter
                    .update_texture(&self.gpu.device, &self.gpu.queue, id, &delta);
            }
        }
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let buffers = self.painter.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        let view = frame.texture.create_view(&Default::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("diagnostics UI"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.painter
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        self.gpu
            .queue
            .submit(buffers.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        for id in output.textures_delta.free.drain() {
            self.painter.free_texture(&id);
        }
        commands
    }
}

#[cfg(windows)]
fn dark_caption(window: &Window) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(handle) = window.window_handle()
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        // COLORREF is 0x00BBGGRR. Older Windows versions may reject these
        // cosmetic attributes; native controls and window behavior are retained.
        for (attribute, color) in [
            (DWMWA_CAPTION_COLOR, 0x00151a0bu32),
            (DWMWA_TEXT_COLOR, 0x00e3eee5u32),
            (DWMWA_BORDER_COLOR, 0x0039402bu32),
        ] {
            // SAFETY: winit owns this valid HWND, and the color pointer remains
            // valid for the duration of this synchronous DWM call.
            unsafe {
                let _ = DwmSetWindowAttribute(
                    handle.hwnd.get() as _,
                    attribute as _,
                    (&color as *const u32).cast(),
                    4,
                );
            }
        }
    }
}

#[cfg(not(windows))]
fn dark_caption(_window: &Window) {}
