use super::text::TextRenderer;
use super::ui::{ImageRenderer, Overlay, UiRenderer};

/// Shared depth-free overlay path for both scene pipelines.
pub(super) struct OverlayRenderer {
    rects: UiRenderer,
    images: ImageRenderer,
    text: TextRenderer,
}

impl OverlayRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            rects: UiRenderer::new(device, format),
            images: ImageRenderer::new(device, format, texture_layout),
            text: TextRenderer::new(device, queue, format),
        }
    }

    pub(super) fn sprite_batch(&self, device: &wgpu::Device) -> ImageRenderer {
        self.images.for_sprites(device)
    }

    pub(super) fn tile_batch(&self, device: &wgpu::Device) -> ImageRenderer {
        self.images.for_tiles(device)
    }

    /// Record an overlay pass only if it has work. The caller resolves the
    /// timestamp pair only when this returns true.
    pub(super) fn record(
        &mut self,
        queue: &wgpu::Queue,
        (width, height): (u32, u32),
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        overlay: &Overlay<'_>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> bool {
        let rects =
            !overlay.rects.is_empty() && self.rects.write(queue, width, height, overlay.rects) > 0;
        let images = !overlay.images.is_empty()
            && self.images.write(queue, width, height, overlay.images) > 0;
        let text = !overlay.text.is_empty() && self.text.write(queue, width, height, overlay.text);
        if !(rects || images || text) {
            return false;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("overlay pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes,
            ..Default::default()
        });
        if rects {
            self.rects.draw(&mut pass);
        }
        if images {
            self.images.draw(&mut pass, overlay.images);
        }
        if text {
            self.text.draw(&mut pass);
        }
        true
    }
}
