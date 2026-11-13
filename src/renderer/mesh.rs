use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::math::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl MeshVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MeshCreateError {
    EmptyVertices,
    EmptyIndices,
    InvalidTriangleIndexCount(usize),
    IndexOutOfBounds { index: u32, vertex_count: usize },
    TooManyIndices(usize),
}

impl std::fmt::Display for MeshCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyVertices => write!(f, "mesh must contain at least one vertex"),
            Self::EmptyIndices => write!(f, "mesh must contain at least one index"),
            Self::InvalidTriangleIndexCount(count) => write!(
                f,
                "triangle-list mesh index count {count} must be divisible by 3"
            ),
            Self::IndexOutOfBounds {
                index,
                vertex_count,
            } => write!(
                f,
                "mesh index {index} is outside vertex count {vertex_count}"
            ),
            Self::TooManyIndices(count) => {
                write!(f, "mesh index count {count} exceeds the supported range")
            }
        }
    }
}

impl std::error::Error for MeshCreateError {}

pub struct Mesh3d {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl Mesh3d {
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        vertices: &[MeshVertex],
        indices: &[u32],
    ) -> Result<Self, MeshCreateError> {
        if vertices.is_empty() {
            return Err(MeshCreateError::EmptyVertices);
        }
        if indices.is_empty() {
            return Err(MeshCreateError::EmptyIndices);
        }
        if !indices.len().is_multiple_of(3) {
            return Err(MeshCreateError::InvalidTriangleIndexCount(indices.len()));
        }
        if let Some(&index) = indices
            .iter()
            .find(|&&index| index as usize >= vertices.len())
        {
            return Err(MeshCreateError::IndexOutOfBounds {
                index,
                vertex_count: vertices.len(),
            });
        }
        let index_count = u32::try_from(indices.len())
            .map_err(|_| MeshCreateError::TooManyIndices(indices.len()))?;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} vertices")),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} indices")),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count,
        })
    }
}

#[derive(Clone, Copy)]
pub struct Draw3d<'a> {
    pub mesh: &'a Mesh3d,
    pub model: Mat4,
    pub tint: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    tint: [f32; 4],
}

impl Uniforms {
    fn new(view_projection: Mat4, model: Mat4, tint: [f32; 3]) -> Self {
        Self {
            view_projection: to_columns(view_projection),
            model: to_columns(model),
            tint: [tint[0], tint[1], tint[2], 1.0],
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

pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    uniform_stride: u64,
}

impl MeshPipeline {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let uniform_stride = uniform_size.div_ceil(alignment.max(1)) * alignment.max(1);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(uniform_size),
                },
                count: None,
            }],
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh draw uniforms"),
            size: uniform_stride * super::MAX_DRAWN_INSTANCES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(uniform_size),
                }),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("indexed 3d mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(MeshVertex::layout())],
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            uniform_buffer,
            bind_group,
            uniform_stride,
        }
    }

    pub fn write_draw(
        &self,
        queue: &wgpu::Queue,
        index: usize,
        view_projection: Mat4,
        model: Mat4,
        tint: [f32; 3],
    ) {
        debug_assert!(index < super::MAX_DRAWN_INSTANCES);
        let uniforms = Uniforms::new(view_projection, model, tint);
        queue.write_buffer(
            &self.uniform_buffer,
            index as u64 * self.uniform_stride,
            bytemuck::bytes_of(&uniforms),
        );
    }

    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        index: usize,
        mesh: &'pass Mesh3d,
    ) {
        let offset = (index as u64 * self.uniform_stride) as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[offset]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::renderer::Camera3d;
    use crate::renderer::test_support::{skip_without_adapter, test_device};

    #[test]
    fn mesh_creation_rejects_empty_and_out_of_bounds_data() {
        let Some((device, _queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        assert!(matches!(
            Mesh3d::new(&device, "empty", &[], &[0]),
            Err(MeshCreateError::EmptyVertices)
        ));
        let vertex = [MeshVertex {
            position: [0.0, 0.0, 0.0],
            color: [1.0, 0.0, 0.0],
        }];
        assert!(matches!(
            Mesh3d::new(&device, "empty indices", &vertex, &[]),
            Err(MeshCreateError::EmptyIndices)
        ));
        assert!(matches!(
            Mesh3d::new(&device, "invalid index", &vertex, &[1, 0, 0]),
            Err(MeshCreateError::IndexOutOfBounds {
                index: 1,
                vertex_count: 1
            })
        ));
    }

    #[test]
    fn indexed_meshes_use_depth_testing() {
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let triangle = |color| {
            [
                MeshVertex {
                    position: [-1.0, -1.0, 0.0],
                    color,
                },
                MeshVertex {
                    position: [1.0, -1.0, 0.0],
                    color,
                },
                MeshVertex {
                    position: [0.0, 1.0, 0.0],
                    color,
                },
            ]
        };
        let front = Mesh3d::new(
            &device,
            "front triangle",
            &triangle([1.0, 1.0, 1.0]),
            &[0, 1, 2],
        )
        .expect("valid front mesh");
        let back = Mesh3d::new(
            &device,
            "back triangle",
            &triangle([1.0, 1.0, 1.0]),
            &[0, 1, 2],
        )
        .expect("valid back mesh");

        let size = 64;
        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh test color"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh test depth"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline = MeshPipeline::new(&device, color_format);
        let camera = Camera3d {
            position: Vec3::ZERO,
            fov_y_radians: std::f32::consts::FRAC_PI_2,
            near: 0.1,
            far: 100.0,
            ..Default::default()
        };
        let view_projection = camera.view_projection(1.0);
        pipeline.write_draw(
            &queue,
            0,
            view_projection,
            Mat4::translation(Vec3::new(0.0, 0.0, -2.0)),
            [1.0, 0.0, 0.0],
        );
        pipeline.write_draw(
            &queue,
            1,
            view_projection,
            Mat4::translation(Vec3::new(0.0, 0.0, -3.0)),
            [0.0, 0.0, 1.0],
        );

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mesh test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
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
            pipeline.draw(&mut pass, 0, &front);
            pipeline.draw(&mut pass, 1, &back);
        }

        let bytes_per_row = size * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh test readback"),
            size: (bytes_per_row * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color_texture,
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
        let center = ((size / 2 * size + size / 2) * 4) as usize;
        assert!(
            data[center] > data[center + 2],
            "near red triangle should cover the farther blue triangle: {:?}",
            &data[center..center + 4]
        );
    }
}
