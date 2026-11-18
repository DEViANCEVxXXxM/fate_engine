use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::math::Mat4;

use super::Texture2d;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

/// A mesh vertex with texture coordinates. UVs use a top-left origin; values
/// outside `0..=1` are handled by the texture's address mode.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct UvMeshVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub uv: [f32; 2],
}

impl UvMeshVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

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
    NonFiniteUv(usize),
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
            Self::NonFiniteUv(index) => write!(f, "mesh vertex {index} has non-finite UVs"),
        }
    }
}

impl std::error::Error for MeshCreateError {}

fn validate_indices(vertex_count: usize, indices: &[u32]) -> Result<u32, MeshCreateError> {
    if vertex_count == 0 {
        return Err(MeshCreateError::EmptyVertices);
    }
    if indices.is_empty() {
        return Err(MeshCreateError::EmptyIndices);
    }
    if !indices.len().is_multiple_of(3) {
        return Err(MeshCreateError::InvalidTriangleIndexCount(indices.len()));
    }
    let count =
        u32::try_from(indices.len()).map_err(|_| MeshCreateError::TooManyIndices(indices.len()))?;
    if let Some(&index) = indices
        .iter()
        .find(|&&index| index as usize >= vertex_count)
    {
        return Err(MeshCreateError::IndexOutOfBounds {
            index,
            vertex_count,
        });
    }
    Ok(count)
}

fn validate_uv_mesh(vertices: &[UvMeshVertex], indices: &[u32]) -> Result<u32, MeshCreateError> {
    let count = validate_indices(vertices.len(), indices)?;
    if let Some(index) = vertices
        .iter()
        .position(|vertex| !vertex.uv.iter().all(|uv| uv.is_finite()))
    {
        return Err(MeshCreateError::NonFiniteUv(index));
    }
    Ok(count)
}

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
        let index_count = validate_indices(vertices.len(), indices)?;
        let vertices: Vec<_> = vertices
            .iter()
            .map(|vertex| UvMeshVertex {
                position: vertex.position,
                color: vertex.color,
                uv: [0.0; 2],
            })
            .collect();
        Ok(Self::upload(device, label, &vertices, indices, index_count))
    }

    pub(crate) fn new_with_uvs(
        device: &wgpu::Device,
        label: &str,
        vertices: &[UvMeshVertex],
        indices: &[u32],
    ) -> Result<Self, MeshCreateError> {
        let index_count = validate_uv_mesh(vertices, indices)?;
        Ok(Self::upload(device, label, vertices, indices, index_count))
    }

    fn upload(
        device: &wgpu::Device,
        label: &str,
        vertices: &[UvMeshVertex],
        indices: &[u32],
        index_count: u32,
    ) -> Self {
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

        Self {
            vertex_buffer,
            index_buffer,
            index_count,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Draw3d<'a> {
    pub mesh: &'a Mesh3d,
    pub model: Mat4,
    pub tint: [f32; 3],
}

impl<'a> Draw3d<'a> {
    /// Sample `texture` using the mesh UVs. Meshes uploaded without UVs use
    /// `[0.0, 0.0]` at every vertex.
    pub const fn with_texture(self, texture: &'a Texture2d) -> SceneDraw3d<'a> {
        SceneDraw3d {
            draw: self,
            texture: Some(texture),
        }
    }
}

/// One opaque scene draw, with an optional color texture.
#[derive(Clone, Copy)]
pub struct SceneDraw3d<'a> {
    pub draw: Draw3d<'a>,
    pub texture: Option<&'a Texture2d>,
}

impl<'a> From<Draw3d<'a>> for SceneDraw3d<'a> {
    fn from(draw: Draw3d<'a>) -> Self {
        Self {
            draw,
            texture: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MeshInstance {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
}

impl MeshInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];

    const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }

    fn new(model: Mat4, tint: [f32; 3]) -> Self {
        Self {
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
    textured_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instances: Vec<MeshInstance>,
}

fn mesh_runs<'a>(
    draws: impl Iterator<Item = SceneDraw3d<'a>>,
) -> impl Iterator<Item = (std::ops::Range<u32>, SceneDraw3d<'a>)> {
    // Preserve submission order, including the winner for equal-depth draws.
    let mut draws = draws.peekable();
    let mut end = 0;
    std::iter::from_fn(move || {
        let draw = draws.next()?;
        let start = end;
        end += 1;
        while draws
            .next_if(|next| {
                std::ptr::eq(draw.draw.mesh, next.draw.mesh)
                    && draw.texture.map(std::ptr::from_ref) == next.texture.map(std::ptr::from_ref)
            })
            .is_some()
        {
            end += 1;
        }
        Some((start..end, draw))
    })
}

impl MeshPipeline {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
        let uniform_size = std::mem::size_of::<[[f32; 4]; 4]>() as u64;

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh bind group layout"),
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
            label: Some("mesh view projection"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh instances"),
            size: (std::mem::size_of::<MeshInstance>() * super::MAX_DRAWN_INSTANCES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
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

        let textured_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("textured mesh pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let make_pipeline = |layout, fragment| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("indexed 3d mesh pipeline"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(UvMeshVertex::layout()), Some(MeshInstance::layout())],
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
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        Self {
            pipeline: make_pipeline(&pipeline_layout, "fs_main"),
            textured_pipeline: make_pipeline(&textured_layout, "fs_textured"),
            uniform_buffer,
            bind_group,
            instance_buffer,
            instances: Vec::with_capacity(super::MAX_DRAWN_INSTANCES),
        }
    }

    pub fn write_draws<'a>(
        &mut self,
        queue: &wgpu::Queue,
        view_projection: Mat4,
        draws: impl Iterator<Item = SceneDraw3d<'a>>,
    ) {
        self.instances.clear();
        self.instances.extend(
            draws
                .take(super::MAX_DRAWN_INSTANCES)
                .map(|draw| MeshInstance::new(draw.draw.model, draw.draw.tint)),
        );
        if self.instances.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&to_columns(view_projection)),
        );
        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instances),
        );
    }

    pub fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draws: impl Iterator<Item = SceneDraw3d<'a>>,
    ) -> usize {
        if self.instances.is_empty() {
            return 0;
        }
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        let mut calls = 0;
        for (instances, draw) in mesh_runs(draws.take(self.instances.len())) {
            let mesh = draw.draw.mesh;
            if let Some(texture) = draw.texture {
                pass.set_pipeline(&self.textured_pipeline);
                pass.set_bind_group(1, &texture.bind_group, &[]);
            } else {
                pass.set_pipeline(&self.pipeline);
            }
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, instances);
            calls += 1;
        }
        calls
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
        assert_eq!(
            validate_indices(0, &[0]),
            Err(MeshCreateError::EmptyVertices)
        );
        assert_eq!(validate_indices(1, &[]), Err(MeshCreateError::EmptyIndices));
        assert_eq!(
            validate_indices(1, &[1, 0, 0]),
            Err(MeshCreateError::IndexOutOfBounds {
                index: 1,
                vertex_count: 1
            })
        );
    }

    #[test]
    fn uv_mesh_accepts_repeating_coordinates_but_rejects_non_finite_uvs() {
        let mut vertices = [UvMeshVertex {
            position: [0.0; 3],
            color: [1.0; 3],
            uv: [-1.0, 2.0],
        }];
        assert_eq!(validate_uv_mesh(&vertices, &[0, 0, 0]), Ok(3));
        vertices[0].uv[1] = f32::NAN;
        assert_eq!(
            validate_uv_mesh(&vertices, &[0, 0, 0]),
            Err(MeshCreateError::NonFiniteUv(0))
        );
    }

    #[test]
    fn indexed_meshes_use_depth_testing() {
        check_mesh_depth(false);
    }

    #[test]
    fn textured_mesh_samples_uvs_and_shares_depth_with_color_meshes() {
        check_mesh_depth(true);
    }

    fn check_mesh_depth(textured: bool) {
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
        let front_vertices = triangle([1.0; 3]);
        let uv_vertices = std::array::from_fn::<_, 3, _>(|index| UvMeshVertex {
            position: front_vertices[index].position,
            color: [1.0, 0.5, 1.0],
            uv: [[0.0, 1.0], [1.0, 1.0], [0.5, 0.0]][index],
        });
        let front = if textured {
            Mesh3d::new_with_uvs(&device, "front triangle", &uv_vertices, &[0, 1, 2])
        } else {
            Mesh3d::new(&device, "front triangle", &front_vertices, &[0, 1, 2])
        }
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
        let texture_layout = super::super::texture::texture_layout(&device);
        let mut pipeline = MeshPipeline::new(&device, color_format, &texture_layout);
        let texture = textured.then(|| {
            Texture2d::new(
                &device,
                &queue,
                &texture_layout,
                super::super::texture::TextureUpload {
                    label: "red and green",
                    width: 2,
                    height: 1,
                    rgba8: &[255, 0, 0, 0, 0, 255, 0, 255],
                    options: super::super::TextureOptions::NEAREST,
                },
            )
            .expect("valid mesh texture")
        });
        let camera = Camera3d {
            position: Vec3::ZERO,
            fov_y_radians: std::f32::consts::FRAC_PI_2,
            near: 0.1,
            far: 100.0,
            ..Default::default()
        };
        let view_projection = camera.view_projection(1.0);
        let draws = [
            SceneDraw3d {
                draw: Draw3d {
                    mesh: &front,
                    model: Mat4::translation(Vec3::new(0.0, 0.0, -2.0)),
                    tint: if textured {
                        [0.5, 1.0, 1.0]
                    } else {
                        [1.0, 0.0, 0.0]
                    },
                },
                texture: texture.as_ref(),
            },
            SceneDraw3d {
                draw: Draw3d {
                    mesh: &front,
                    model: Mat4::translation(Vec3::new(1.5, 0.0, -2.0)),
                    tint: [0.0, 0.5, 0.0],
                },
                texture: texture.as_ref(),
            },
            Draw3d {
                mesh: &back,
                model: Mat4::translation(Vec3::new(0.0, 0.0, -3.0)),
                tint: [0.0, 0.0, 1.0],
            }
            .into(),
        ];
        let ranges: Vec<_> = mesh_runs(draws.into_iter())
            .map(|(range, _)| range)
            .collect();
        assert_eq!(ranges, [0..2, 2..3]);
        if textured {
            let changes = [draws[0], draws[0].draw.into(), draws[1]];
            let ranges: Vec<_> = mesh_runs(changes.into_iter())
                .map(|(range, _)| range)
                .collect();
            assert_eq!(ranges, [0..1, 1..2, 2..3]);
        }
        pipeline.write_draws(&queue, view_projection, draws.into_iter());

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
            assert_eq!(pipeline.draw(&mut pass, draws.into_iter()), 2);
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
        let pixel_at = |x: u32, y: u32| {
            let start = ((y * size + x) * 4) as usize;
            &data[start..start + 4]
        };
        let second_instance = pixel_at(57, 36);
        let expected_green = if textured { 63..=65 } else { 126..=129 };
        assert!(
            expected_green.contains(&second_instance[1]),
            "second instance should use its own transform and tint: {second_instance:?}"
        );
        assert_eq!(
            [second_instance[0], second_instance[2], second_instance[3]],
            [0, 0, 255]
        );
        if textured {
            let left = pixel_at(28, 36);
            let right = pixel_at(36, 36);
            assert!((126..=129).contains(&left[0]), "{left:?}");
            assert_eq!(&left[1..], &[0, 0, 255], "{left:?}");
            assert!((126..=129).contains(&right[1]), "{right:?}");
            assert_eq!([right[0], right[2], right[3]], [0, 0, 255], "{right:?}");
            return;
        }
        let center = ((size / 2 * size + size / 2) * 4) as usize;
        assert!(
            data[center] > data[center + 2],
            "near red triangle should cover the farther blue triangle: {:?}",
            &data[center..center + 4]
        );
    }
}
