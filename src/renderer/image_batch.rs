//! Shared GPU batch for textured quads.
//!
//! Overlay images, world-space sprites, and tile layers all draw as
//! alpha-blended textured quads. This module owns the vertex buffer, pipeline,
//! and run merging; callers build quads from their own geometry. Adjacent
//! quads that share a texture and scissor merge into one draw, and runs keep
//! submission order so translucent quads composite correctly.

use bytemuck::{Pod, Zeroable};

use super::ui::Scissor;
use super::{Texture2d, TextureRegion};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct ImageVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl ImageVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

    const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// A contiguous vertex range that shares one texture and scissor.
struct ImageRun {
    texture_source: usize,
    scissor: Scissor,
    vertices: std::ops::Range<u32>,
}

/// One textured quad ready to append to an [`ImageBatch`].
pub(crate) struct Quad<'a> {
    /// Index passed back to the caller's texture lookup when drawing.
    pub(crate) source: usize,
    pub(crate) texture: &'a Texture2d,
    pub(crate) scissor: Scissor,
    pub(crate) vertices: [ImageVertex; 6],
}

/// Build two triangles from clip-space corners, sampling `region`.
pub(crate) fn textured_vertices(
    [top_left, top_right, bottom_right, bottom_left]: [[f32; 2]; 4],
    region: TextureRegion,
    color: [f32; 4],
) -> [ImageVertex; 6] {
    let [u0, v0] = [region.min.x, region.min.y];
    let [u1, v1] = [region.max.x, region.max.y];
    let vertex = |position, uv| ImageVertex {
        position,
        uv,
        color,
    };

    [
        vertex(top_left, [u0, v0]),
        vertex(bottom_left, [u0, v1]),
        vertex(bottom_right, [u1, v1]),
        vertex(top_left, [u0, v0]),
        vertex(bottom_right, [u1, v1]),
        vertex(top_right, [u1, v0]),
    ]
}

/// Upload accepted quads and record adjacent quads with compatible GPU state.
pub(crate) struct ImageBatch {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    scratch: Vec<ImageVertex>,
    runs: Vec<ImageRun>,
    limit: usize,
    label: &'static str,
    last_clipped: bool,
}

impl ImageBatch {
    /// Create the overlay image batch.
    pub(crate) fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui_image.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui image pipeline layout"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui image pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(ImageVertex::layout())],
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
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self::with_pipeline(device, pipeline, super::MAX_UI_IMAGES, "overlay images")
    }

    fn with_pipeline(
        device: &wgpu::Device,
        pipeline: wgpu::RenderPipeline,
        limit: usize,
        label: &'static str,
    ) -> Self {
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (limit * 6 * std::mem::size_of::<ImageVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertices,
            scratch: Vec::with_capacity(limit * 6),
            runs: Vec::with_capacity(limit),
            limit,
            label,
            last_clipped: false,
        }
    }

    /// A batch sized for the world-space sprite budget.
    pub(super) fn for_sprites(&self, device: &wgpu::Device) -> Self {
        Self::with_pipeline(
            device,
            self.pipeline.clone(),
            super::MAX_DRAWN_INSTANCES,
            "2D sprites",
        )
    }

    /// A batch sized for every tile layer's visible-tile budget.
    pub(super) fn for_tiles(&self, device: &wgpu::Device) -> Self {
        Self::with_pipeline(
            device,
            self.pipeline.clone(),
            super::MAX_DRAWN_TILES * super::MAX_TILE_LAYERS,
            "2D tile layers",
        )
    }

    /// Maximum quads this batch may queue in one frame.
    pub(super) fn limit(&self) -> usize {
        self.limit
    }

    /// Queue one stream of quads up to the batch limit and upload it.
    ///
    /// Returns the number of quads that were actually queued.
    pub(super) fn write_quads<'a>(
        &mut self,
        queue: &wgpu::Queue,
        quads: impl Iterator<Item = Quad<'a>>,
    ) -> usize {
        let limit = self.limit;
        self.begin();
        let mut previous_texture = None;
        let (count, clipped) = self.append(quads, &mut previous_texture, limit);
        self.end(queue, clipped, limit, "quads per frame");
        count
    }

    /// Start a new frame, discarding any previous runs.
    pub(super) fn begin(&mut self) {
        self.scratch.clear();
        self.runs.clear();
    }

    /// Append at most `limit` quads, merging compatible adjacent runs.
    ///
    /// Returns the number of accepted quads and whether the limit cut the
    /// source short.
    pub(super) fn append<'a>(
        &mut self,
        quads: impl Iterator<Item = Quad<'a>>,
        previous_texture: &mut Option<usize>,
        limit: usize,
    ) -> (usize, bool) {
        let mut count = 0;
        for quad in quads {
            if count == limit {
                return (count, true);
            }
            let start = self.scratch.len() as u32;
            self.scratch.extend_from_slice(&quad.vertices);
            let end = self.scratch.len() as u32;
            // Only merge with the last accepted run: regrouping translucent
            // quads across intervening draws would change compositing order.
            let texture = quad.texture as *const Texture2d as usize;
            if let Some(run) = self.runs.last_mut()
                && run.scissor == quad.scissor
                && *previous_texture == Some(texture)
            {
                run.vertices.end = end;
            } else {
                self.runs.push(ImageRun {
                    texture_source: quad.source,
                    scissor: quad.scissor,
                    vertices: start..end,
                });
            }
            *previous_texture = Some(texture);
            count += 1;
        }
        (count, false)
    }

    /// Upload the accumulated quads and report a budget cut once per transition.
    pub(super) fn end(&mut self, queue: &wgpu::Queue, dropped: bool, limit: usize, unit: &str) {
        if dropped && !self.last_clipped {
            log::warn!(
                "{} budget exceeded: {limit} {unit}; dropping the rest",
                self.label
            );
        }
        self.last_clipped = dropped;

        if !self.scratch.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.scratch));
        }
    }

    /// Record the prepared runs, resolving each run's texture through
    /// `texture_at`. Returns the number of draws recorded.
    pub(super) fn draw_with<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        texture_at: impl Fn(usize) -> &'a Texture2d,
    ) -> usize {
        if self.runs.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for run in &self.runs {
            let texture = texture_at(run.texture_source);
            let Scissor {
                x,
                y,
                width,
                height,
            } = run.scissor;
            pass.set_bind_group(0, &texture.bind_group, &[]);
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(run.vertices.clone(), 0..1);
        }
        self.runs.len()
    }

    /// Number of recorded runs. Only used by tests to check run merging.
    #[cfg(test)]
    pub(super) fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Vertex range of one recorded run. Only used by tests.
    #[cfg(test)]
    pub(super) fn run_vertices(&self, index: usize) -> Option<std::ops::Range<u32>> {
        self.runs.get(index).map(|run| run.vertices.clone())
    }
}
