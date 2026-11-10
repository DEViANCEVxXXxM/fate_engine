//! Engine renderer: owns the GPU context, the camera and the draw pipelines.

mod camera;
mod gpu;
mod pipeline;

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

pub use camera::Camera2d;
pub use gpu::{Gpu, GpuInitError};

use crate::math::{Mat4, Vec2};
use pipeline::TrianglePipeline;

pub const MAX_DRAWN_INSTANCES: usize = pipeline::MAX_INSTANCES;
const DRAW_WARNING_THRESHOLD: f32 = 0.75;

/// A drawable instance of the built-in mesh, in world space.
#[derive(Clone, Copy, Debug, Default)]
pub struct Instance2d {
    pub position: Vec2,
    pub rotation: f32,
    pub scale: f32,
}

impl Instance2d {
    pub fn model_matrix(&self) -> Mat4 {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        Mat4::translation(crate::math::Vec3::new(
            self.position.x,
            self.position.y,
            0.0,
        ))
        .mul(Mat4::rotation_z(self.rotation))
        .mul(Mat4::scale(crate::math::Vec3::new(scale, scale, 1.0)))
    }
}

pub struct Renderer {
    pub gpu: Gpu,
    pub camera: Camera2d,
    triangle: TrianglePipeline,
    clear_color: wgpu::Color,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        let gpu = Gpu::new(window).await?;
        let triangle = TrianglePipeline::new(&gpu.device, gpu.config.format);

        Ok(Self {
            gpu,
            camera: Camera2d::default(),
            triangle,
            clear_color: wgpu::Color {
                r: 0.04,
                g: 0.05,
                b: 0.07,
                a: 1.0,
            },
        })
    }

    pub fn budgeted_instance_count_static(instance_count: usize) -> usize {
        let limit = MAX_DRAWN_INSTANCES;
        let count = instance_count.min(limit);
        let warning_threshold = (limit as f32 * DRAW_WARNING_THRESHOLD) as usize;

        if instance_count > limit {
            log::warn!(
                "render instance budget exceeded: {instance_count} visible objects > {limit} max draw instances; clipping to {count} for the current frame",
            );
        } else if instance_count >= warning_threshold {
            log::warn!(
                "render instance budget nearing cap: {instance_count}/{limit} visible objects; draw pressure may reduce frame stability",
            );
        }

        count
    }

    pub fn budgeted_instance_count(&self, instance_count: usize) -> usize {
        Self::budgeted_instance_count_static(instance_count)
    }

    pub fn max_instance_budget() -> usize {
        MAX_DRAWN_INSTANCES
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.gpu.resize(size);
    }

    /// Draw one frame. Returns `false` when the frame was skipped because the
    /// surface was not ready (minimized, timed out, lost).
    pub fn render(&mut self, instances: &[Instance2d]) -> bool {
        use wgpu::CurrentSurfaceTexture;

        let frame = match self.gpu.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => {
                // Still drawable; fix the swapchain for the next frame.
                self.gpu.reconfigure();
                frame
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                log::warn!("surface out of date, reconfiguring");
                self.gpu.reconfigure();
                return false;
            }
            CurrentSurfaceTexture::Timeout => {
                log::debug!("surface timeout, skipping frame");
                return false;
            }
            CurrentSurfaceTexture::Occluded => return false,
            CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error, skipping frame");
                return false;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let view_proj = self.camera.view_projection(self.gpu.aspect());
        let count = self.budgeted_instance_count(instances.len());

        for (index, instance) in instances.iter().take(count).enumerate() {
            self.triangle.write_instance(
                &self.gpu.queue,
                index,
                view_proj,
                instance.model_matrix(),
            );
        }

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            for index in 0..count {
                self.triangle.draw(&mut pass, index);
            }
        }

        self.gpu.queue.submit(Some(encoder.finish()));
        self.gpu.queue.present(frame);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgeted_instance_count_clips_beyond_render_limit() {
        assert_eq!(Renderer::budgeted_instance_count_static(10), 10);
        assert_eq!(
            Renderer::budgeted_instance_count_static(MAX_DRAWN_INSTANCES + 7),
            MAX_DRAWN_INSTANCES
        );
        assert_eq!(
            Renderer::budgeted_instance_count_static((MAX_DRAWN_INSTANCES * 3) / 4),
            (MAX_DRAWN_INSTANCES * 3) / 4
        );
    }
}
