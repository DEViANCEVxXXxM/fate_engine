use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::math::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl Vertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Model {
    columns: [[f32; 4]; 4],
}

impl Model {
    const ATTRIBS: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4];

    const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

fn to_columns(matrix: Mat4) -> [[f32; 4]; 4] {
    let m = matrix.to_cols_array();
    [
        [m[0], m[1], m[2], m[3]],
        [m[4], m[5], m[6], m[7]],
        [m[8], m[9], m[10], m[11]],
        [m[12], m[13], m[14], m[15]],
    ]
}

/// Shared upper bound on scene instances drawn per frame.
pub const MAX_INSTANCES: usize = 256;

/// Static triangle mesh, shared camera uniform, and streamed model matrices.
pub struct TrianglePipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    models: Vec<Model>,
    vertex_count: u32,
}

impl TrianglePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let uniform_size = std::mem::size_of::<[[f32; 4]; 4]>() as u64;

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("triangle bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(uniform_size),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle camera uniform"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("triangle bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("triangle pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::layout()), Some(Model::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let vertices = [
            Vertex {
                position: [0.0, 0.75, 0.0],
                color: [1.0, 0.35, 0.35],
            },
            Vertex {
                position: [-0.65, -0.6, 0.0],
                color: [0.35, 1.0, 0.45],
            },
            Vertex {
                position: [0.65, -0.6, 0.0],
                color: [0.4, 0.55, 1.0],
            },
        ];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("triangle vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle instance models"),
            size: (MAX_INSTANCES * std::mem::size_of::<Model>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            instance_buffer,
            models: Vec::with_capacity(MAX_INSTANCES),
            vertex_count: vertices.len() as u32,
        }
    }

    pub fn write_instances(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        models: impl IntoIterator<Item = Mat4>,
    ) {
        self.models.clear();
        self.models
            .extend(models.into_iter().take(MAX_INSTANCES).map(|model| Model {
                columns: to_columns(model),
            }));
        if !self.models.is_empty() {
            queue.write_buffer(
                &self.uniform_buffer,
                0,
                bytemuck::bytes_of(&to_columns(view_proj)),
            );
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.models));
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) -> usize {
        if self.models.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..self.models.len() as u32);
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Mat4, Vec3};
    use crate::renderer::test_support::{skip_without_adapter, test_device};

    #[test]
    fn instance_layout_matches_shader_model_columns() {
        let layout = Model::layout();
        assert_eq!(layout.array_stride, 64);
        assert_eq!(layout.step_mode, wgpu::VertexStepMode::Instance);
        for (i, attr) in layout.attributes.iter().enumerate() {
            assert_eq!(attr.shader_location, 2 + i as u32);
            assert_eq!(attr.offset, 16 * i as u64);
            assert_eq!(attr.format, wgpu::VertexFormat::Float32x4);
        }
        let model = Mat4::translation(Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(to_columns(model)[3], [2.0, 3.0, 4.0, 1.0]);
    }

    /// Verify two different matrices are fetched in the same instanced draw.
    #[test]
    fn two_instances_render_in_one_offscreen_draw() {
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let size: u32 = 64;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut pipeline = TrianglePipeline::new(&device, format);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test target"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let model = |x| {
            Mat4::translation(Vec3::new(x, 0.0, 0.0)).mul(Mat4::scale(Vec3::new(0.55, 0.55, 1.0)))
        };
        pipeline.write_instances(&queue, Mat4::IDENTITY, [model(-0.5), model(0.5)]);

        let bytes_per_row = size * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            assert_eq!(pipeline.draw(&mut pass), 1);
            pipeline.write_instances(&queue, Mat4::IDENTITY, []);
            assert_eq!(pipeline.draw(&mut pass), 0);
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
        receiver
            .recv()
            .expect("map callback never ran")
            .expect("buffer mapping failed");

        let data = slice.get_mapped_range().expect("mapped range");
        let bytes = data.as_ref();
        let (pixels, _) = bytes.as_chunks::<4>();
        let lit = |x: usize, y: usize| pixels[y * size as usize + x][..3].iter().any(|&v| v > 10);
        assert!(
            lit(16, 32) && lit(48, 32),
            "both instance centers should be lit"
        );
        assert!(
            !lit(32, 32),
            "the gap between instances should remain clear"
        );
        drop(data);
        readback.unmap();
    }
}

#[cfg(test)]
mod camera_scale_tests {
    use super::*;
    use crate::renderer::Camera2d;
    use crate::renderer::Instance2d;
    use crate::renderer::test_support::{skip_without_adapter, test_device};

    /// Same matrices the app uses on the first frame: default camera, one
    /// instance near the origin. The triangle is small in world units, so this
    /// guards against the ortho projection being wrong.
    #[test]
    fn default_camera_draws_the_triangle() {
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let width = 1280u32;
        let height = 720u32;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut pipeline = TrianglePipeline::new(&device, format);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let camera = Camera2d::default();
        let instance = Instance2d {
            position: crate::math::Vec2::new(0.4, 0.25),
            rotation: 0.1,
            scale: 1.0,
        };
        pipeline.write_instances(
            &queue,
            camera.view_projection(width as f32 / height as f32),
            [instance.model_matrix()],
        );

        let bytes_per_row = width * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            assert_eq!(pipeline.draw(&mut pass), 1);
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
        receiver
            .recv()
            .expect("map callback never ran")
            .expect("buffer mapping failed");

        let data = slice.get_mapped_range().expect("mapped range");
        let bytes = data.as_ref();
        let (pixels, _) = bytes.as_chunks::<4>();
        let lit_pixels = pixels
            .iter()
            .filter(|px| px[0] > 10 || px[1] > 10 || px[2] > 10)
            .count();
        drop(data);
        readback.unmap();

        assert!(
            lit_pixels > 500,
            "expected a visible triangle from the default camera, got {lit_pixels} lit pixels"
        );
    }
}
