//! Engine renderer: owns the GPU context, the camera and the draw pipelines.

mod camera;
mod gpu;
mod mesh;
mod pipeline;

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

pub use camera::{Camera2d, Camera3d};
pub use gpu::{Gpu, GpuInitError};
pub use mesh::{Draw3d, Mesh3d, MeshCreateError, MeshVertex};

use crate::math::{Mat4, Vec2};
use mesh::MeshPipeline;
use pipeline::TrianglePipeline;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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
    #[allow(dead_code)]
    pub camera_3d: Camera3d,
    triangle: TrianglePipeline,
    mesh_pipeline: MeshPipeline,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    clear_color: wgpu::Color,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        let gpu = Gpu::new(window).await?;
        let triangle = TrianglePipeline::new(&gpu.device, gpu.config.format);
        let mesh_pipeline = MeshPipeline::new(&gpu.device, gpu.config.format);
        let (depth_texture, depth_view) = Self::create_depth_target(&gpu);

        Ok(Self {
            gpu,
            camera: Camera2d::default(),
            camera_3d: Camera3d::default(),
            triangle,
            mesh_pipeline,
            _depth_texture: depth_texture,
            depth_view,
            clear_color: wgpu::Color {
                r: 0.04,
                g: 0.05,
                b: 0.07,
                a: 1.0,
            },
        })
    }

    fn create_depth_target(gpu: &Gpu) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("main depth target"),
            size: wgpu::Extent3d {
                width: gpu.config.width,
                height: gpu.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
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
        let previous_size = (self.gpu.config.width, self.gpu.config.height);
        self.gpu.resize(size);
        if size.width > 0
            && size.height > 0
            && previous_size != (self.gpu.config.width, self.gpu.config.height)
        {
            let (texture, view) = Self::create_depth_target(&self.gpu);
            self._depth_texture = texture;
            self.depth_view = view;
        }
    }

    /// Upload and draw indexed meshes using the perspective camera and a depth
    /// buffer. Meshes and transforms remain owned by the caller.
    pub fn render_meshes(&mut self, draws: &[Draw3d<'_>]) -> bool {
        use wgpu::CurrentSurfaceTexture;

        let (frame, suboptimal) = match self.gpu.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                log::warn!("surface out of date, reconfiguring");
                self.gpu.reconfigure();
                return false;
            }
            CurrentSurfaceTexture::Timeout => {
                log::debug!("surface timeout, skipping 3d frame");
                return false;
            }
            CurrentSurfaceTexture::Occluded => return false,
            CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error, skipping 3d frame");
                return false;
            }
        };
        if suboptimal {
            self.gpu.reconfigure();
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let count = self.budgeted_instance_count(draws.len());
        let view_projection = self.camera_3d.view_projection(self.gpu.aspect());
        for (index, draw) in draws.iter().take(count).enumerate() {
            self.mesh_pipeline.write_draw(
                &self.gpu.queue,
                index,
                view_projection,
                draw.model,
                draw.tint,
            );
        }

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("3d frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for (index, draw) in draws.iter().take(count).enumerate() {
                self.mesh_pipeline.draw(&mut pass, index, draw.mesh);
            }
        }

        self.gpu.queue.submit(Some(encoder.finish()));
        self.gpu.queue.present(frame);
        true
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

pub fn cube_mesh(device: &wgpu::Device) -> Result<Mesh3d, MeshCreateError> {
    let vertices = [
        MeshVertex {
            position: [-0.5, -0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, -0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, 0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, 0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, -0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, -0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, 0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, 0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
    ];
    let indices: [u32; 36] = [
        0, 2, 1, 0, 3, 2, // back
        4, 5, 6, 4, 6, 7, // front
        0, 1, 5, 0, 5, 4, // bottom
        3, 7, 6, 3, 6, 2, // top
        1, 2, 6, 1, 6, 5, // right
        0, 4, 7, 0, 7, 3, // left
    ];
    Mesh3d::new(device, "unit cube", &vertices, &indices)
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
