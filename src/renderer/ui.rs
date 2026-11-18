//! Screen-space rectangles for HUD panels and other 2D overlays.
//!
//! Rectangles use the same coordinate system as [`crate::renderer::TextDraw`]:
//! physical pixels with the origin at the top-left of the window. The renderer
//! draws them after the 3D scene and before text, so consumers can place a
//! translucent panel behind a label without constructing meshes or naming
//! `wgpu` types.
//!
//! Overlay records can opt into [`UiClip`] so content stays inside a panel even
//! when its geometry extends past it.

use bytemuck::{Pod, Zeroable};

use super::{Texture2d, TextureRegion};
use crate::math::Vec2;

/// Upper bound on rectangles drawn in one frame.
pub const MAX_UI_RECTS: usize = 256;
/// Upper bound on textured images drawn in one frame.
pub const MAX_UI_IMAGES: usize = 64;

/// An axis-aligned clip rectangle in physical pixels.
///
/// Attach a clip to a [`UiRect`], [`UiImage`], or
/// [`TextDraw`](crate::renderer::TextDraw) with its `with_clip` builder. Only
/// the pixels inside the clip are drawn; the rest of the record is discarded.
/// Clips are independent per record, so two panels can clip to their own
/// bounds. Clips are intersected with the window by the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiClip {
    pub position: Vec2,
    pub size: Vec2,
}

impl UiClip {
    /// A clip starting at `position` and extending right and down by `size`.
    pub const fn new(position: Vec2, size: Vec2) -> Self {
        Self { position, size }
    }

    /// Convert to a window-space scissor rect, or `None` when the clip is
    /// invalid or empty after rounding.
    pub(crate) fn to_scissor(self, width: u32, height: u32) -> Option<Scissor> {
        if !finite_rect(self.position, self.size) {
            return None;
        }
        let max_width = width as f32;
        let max_height = height as f32;
        let left = self.position.x.max(0.0).floor();
        let top = self.position.y.max(0.0).floor();
        let right = (self.position.x + self.size.x).min(max_width).ceil();
        let bottom = (self.position.y + self.size.y).min(max_height).ceil();
        if !left.is_finite() || !top.is_finite() || right <= left || bottom <= top {
            return None;
        }
        Some(Scissor {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }
}

/// A validated scissor rectangle ready for `set_scissor_rect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scissor {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// One axis-aligned screen-space rectangle, in physical pixels.
///
/// `position` is the top-left corner and `size` extends right and down. Colors
/// are in the same space as [`crate::renderer::TextDraw::color`], including
/// alpha for blending.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRect {
    pub position: Vec2,
    pub size: Vec2,
    pub color: [f32; 4],
    /// Optional bounds outside which nothing is drawn.
    pub clip: Option<UiClip>,
}

impl UiRect {
    /// A rectangle with an explicit position, size, and color.
    pub const fn new(position: Vec2, size: Vec2, color: [f32; 4]) -> Self {
        Self {
            position,
            size,
            color,
            clip: None,
        }
    }

    /// Draw only the part of this rectangle inside `clip`.
    pub const fn with_clip(mut self, clip: UiClip) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Whether this rectangle can be converted to a finite, non-empty quad.
    fn is_drawable(&self) -> bool {
        let finite = |value: f32| value.is_finite();
        finite(self.position.x)
            && finite(self.position.y)
            && finite(self.size.x)
            && finite(self.size.y)
            && self.size.x > 0.0
            && self.size.y > 0.0
            && self.color.iter().all(|channel| channel.is_finite())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct UiVertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl UiVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

    const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Copy the latest rectangle batch and the pipeline that draws it.
pub(crate) struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    scratch: Vec<UiVertex>,
    runs: Vec<DrawRun>,
    surface: (u32, u32),
    last_clipped: bool,
}

/// A contiguous vertex range that shares one clip rectangle.
#[derive(Clone, Copy, Debug)]
struct DrawRun {
    first_vertex: u32,
    vertex_count: u32,
    clip: Option<UiClip>,
}

impl UiRenderer {
    pub(crate) fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(UiVertex::layout())],
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

        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vertices"),
            size: (MAX_UI_RECTS * 6 * std::mem::size_of::<UiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertices,
            scratch: Vec::new(),
            runs: Vec::new(),
            surface: (0, 0),
            last_clipped: false,
        }
    }

    /// Build this frame's rectangle batch and upload it. Call before the pass.
    ///
    /// Returns the number of rectangles that were actually queued. Invalid
    /// rectangles and any beyond [`MAX_UI_RECTS`] are skipped.
    pub(crate) fn write(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rects: &[UiRect],
    ) -> usize {
        self.scratch.clear();
        self.runs.clear();
        self.surface = (width, height);
        let mut count = 0;
        let mut clipped = false;

        for rect in rects {
            if count >= MAX_UI_RECTS {
                clipped = true;
                break;
            }
            let first_vertex = self.scratch.len() as u32;
            if push_rect(&mut self.scratch, rect, width as f32, height as f32) {
                self.runs.push(DrawRun {
                    first_vertex,
                    vertex_count: 6,
                    clip: rect.clip,
                });
                count += 1;
            }
        }

        if clipped && !self.last_clipped {
            log::warn!(
                "ui rectangle budget exceeded: {MAX_UI_RECTS} rectangles per frame; dropping the rest of the frame's rectangles",
            );
        }
        self.last_clipped = clipped;

        if !self.scratch.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.scratch));
        }
        count
    }

    /// Record the rectangle batch into an existing pass.
    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.runs.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        let mut start = 0;
        while start < self.runs.len() {
            let clip = self.runs[start].clip;
            let first_vertex = self.runs[start].first_vertex;
            let mut end = start + 1;
            while end < self.runs.len() && self.runs[end].clip == clip {
                end += 1;
            }
            let vertex_count =
                self.runs[end - 1].first_vertex + self.runs[end - 1].vertex_count - first_vertex;
            if apply_scissor(pass, self.surface, clip) {
                pass.draw(first_vertex..first_vertex + vertex_count, 0..1);
            }
            start = end;
        }
    }
}

/// One textured screen-space image, in physical pixels.
///
/// `position` is the top-left corner and `size` extends right and down, matching
/// [`UiRect`]. By default the full texture is sampled; [`Self::with_region`]
/// selects a normalized sub-rectangle, including regions beyond `1.0` for tiled
/// sampling with a repeating texture. The sample is tinted by `color`, optionally
/// rotated around the image center with [`Self::with_rotation`], and alpha
/// blended. Images draw after rectangles and before text.
#[derive(Clone, Copy, Debug)]
pub struct UiImage<'a> {
    pub texture: &'a Texture2d,
    pub position: Vec2,
    pub size: Vec2,
    /// Normalized sub-rectangle to sample; defaults to the whole texture.
    pub region: TextureRegion,
    pub color: [f32; 4],
    /// Clockwise rotation in radians around the image center.
    pub rotation: f32,
    /// Optional bounds outside which nothing is drawn.
    pub clip: Option<UiClip>,
}

impl<'a> UiImage<'a> {
    /// A white-tinted image at `position` with the requested draw size.
    pub fn new(texture: &'a Texture2d, position: Vec2, size: Vec2) -> Self {
        Self {
            texture,
            position,
            size,
            region: TextureRegion::FULL,
            color: [1.0; 4],
            rotation: 0.0,
            clip: None,
        }
    }

    /// Draw only the part of this image inside `clip`.
    pub const fn with_clip(mut self, clip: UiClip) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Rotate clockwise by `radians` around the image center.
    ///
    /// Positive angles turn the top edge toward the right, matching screen
    /// coordinates where `y` grows downward. Rotation does not change the
    /// sampled region or the clip.
    pub const fn with_rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Sample `region` instead of the whole texture, for atlas tiles.
    pub fn with_region(mut self, region: TextureRegion) -> Self {
        self.region = region;
        self
    }

    /// Set the color multiplied into the sampled texture, including alpha.
    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    fn is_drawable(&self) -> bool {
        finite_rect(self.position, self.size)
            && self.region.is_finite()
            && self.rotation.is_finite()
            && self.color.iter().all(|channel| channel.is_finite())
    }
}

/// Screen-space content composited between the 3D scene and the frame's end.
///
/// Fields draw in order: [`rects`](Self::rects), then
/// [`images`](Self::images), then [`text`](Self::text). The default overlay is
/// empty, so consumers fill only the layers they use.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay<'a> {
    pub rects: &'a [UiRect],
    pub images: &'a [UiImage<'a>],
    pub text: &'a [super::TextDraw<'a>],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ImageVertex {
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

/// Copy the latest image batch and the pipeline that draws it.
pub(crate) struct ImageRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    scratch: Vec<ImageVertex>,
    vertex_count: u32,
    surface: (u32, u32),
    last_clipped: bool,
}

impl ImageRenderer {
    pub(crate) fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui_image.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui image bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui image pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
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

        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui image vertices"),
            size: (MAX_UI_IMAGES * 6 * std::mem::size_of::<ImageVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            layout,
            vertices,
            scratch: Vec::new(),
            vertex_count: 0,
            surface: (0, 0),
            last_clipped: false,
        }
    }

    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Build this frame's image batch and upload it. Call before the pass.
    ///
    /// Returns the number of images that were actually queued.
    pub(crate) fn write(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        images: &[UiImage<'_>],
    ) -> usize {
        self.scratch.clear();
        self.surface = (width, height);
        let mut count = 0;
        let mut clipped = false;

        for image in images {
            if count >= MAX_UI_IMAGES {
                clipped = true;
                break;
            }
            if push_image(&mut self.scratch, image, width as f32, height as f32) {
                count += 1;
            }
        }

        if clipped && !self.last_clipped {
            log::warn!(
                "ui image budget exceeded: {MAX_UI_IMAGES} images per frame; dropping the rest of the frame's images",
            );
        }
        self.last_clipped = clipped;

        self.vertex_count = self.scratch.len() as u32;
        if !self.scratch.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.scratch));
        }
        count
    }

    /// Record the image batch into an existing pass.
    pub(crate) fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        images: &[UiImage<'_>],
    ) {
        if self.vertex_count == 0 {
            return;
        }
        let mut index = 0;
        for image in images {
            if index >= MAX_UI_IMAGES {
                break;
            }
            if !image.is_drawable() || !rect_fits_surface(image.position, image.size) {
                continue;
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &image.texture.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            if !apply_scissor(pass, self.surface, image.clip) {
                index += 1;
                continue;
            }
            let start = (index * 6) as u32;
            pass.draw(start..start + 6, 0..1);
            index += 1;
        }
    }
}

fn finite_rect(position: Vec2, size: Vec2) -> bool {
    position.x.is_finite()
        && position.y.is_finite()
        && size.x.is_finite()
        && size.y.is_finite()
        && size.x > 0.0
        && size.y > 0.0
}

/// Apply `clip` to the pass, or reset to the full surface when unclipped.
///
/// Returns `false` when the clip rounds away to nothing and the caller should
/// skip the draw entirely.
pub(crate) fn apply_scissor(
    pass: &mut wgpu::RenderPass<'_>,
    surface: (u32, u32),
    clip: Option<UiClip>,
) -> bool {
    match clip {
        Some(clip) => match clip.to_scissor(surface.0, surface.1) {
            Some(rect) => {
                pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
                true
            }
            None => false,
        },
        None => {
            pass.set_scissor_rect(0, 0, surface.0.max(1), surface.1.max(1));
            true
        }
    }
}

/// The draw path normally knows the surface size; this guard only keeps a
/// deliberately huge off-screen image from producing non-finite NDC values.
fn rect_fits_surface(position: Vec2, size: Vec2) -> bool {
    (position.x + size.x).is_finite() && (position.y + size.y).is_finite()
}

/// Pixel-space quad corners of an image, clockwise from the top-left.
///
/// Returns `None` when the center or any rotated corner is not finite.
fn rotated_corners(position: Vec2, size: Vec2, rotation: f32) -> Option<[[f32; 2]; 4]> {
    let half_width = size.x * 0.5;
    let half_height = size.y * 0.5;
    let center_x = position.x + half_width;
    let center_y = position.y + half_height;
    let (sin, cos) = rotation.sin_cos();
    let corner = |dx: f32, dy: f32| {
        [
            center_x + dx * cos - dy * sin,
            center_y + dx * sin + dy * cos,
        ]
    };
    let corners = [
        corner(-half_width, -half_height),
        corner(half_width, -half_height),
        corner(half_width, half_height),
        corner(-half_width, half_height),
    ];
    corners
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(corners)
}

/// Append two triangles covering `image`, in normalized device coordinates.
fn push_image(out: &mut Vec<ImageVertex>, image: &UiImage<'_>, width: f32, height: f32) -> bool {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return false;
    }
    if !image.is_drawable() || !rect_fits_surface(image.position, image.size) {
        return false;
    }
    let Some([top_left, top_right, bottom_right, bottom_left]) =
        rotated_corners(image.position, image.size, image.rotation)
    else {
        return false;
    };

    let ndc = |[x, y]: [f32; 2]| [x / width * 2.0 - 1.0, 1.0 - y / height * 2.0];
    let color = image.color;
    let [u0, v0] = [image.region.min.x, image.region.min.y];
    let [u1, v1] = [image.region.max.x, image.region.max.y];
    let vertex = |position, uv| ImageVertex {
        position,
        uv,
        color,
    };

    out.extend_from_slice(&[
        vertex(ndc(top_left), [u0, v0]),
        vertex(ndc(bottom_left), [u0, v1]),
        vertex(ndc(bottom_right), [u1, v1]),
        vertex(ndc(top_left), [u0, v0]),
        vertex(ndc(bottom_right), [u1, v1]),
        vertex(ndc(top_right), [u1, v0]),
    ]);
    true
}

/// Append two triangles covering `rect`, in normalized device coordinates.
fn push_rect(out: &mut Vec<UiVertex>, rect: &UiRect, width: f32, height: f32) -> bool {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return false;
    }
    if !rect.is_drawable() {
        return false;
    }

    let x0 = rect.position.x / width * 2.0 - 1.0;
    let x1 = (rect.position.x + rect.size.x) / width * 2.0 - 1.0;
    let y0 = 1.0 - rect.position.y / height * 2.0;
    let y1 = 1.0 - (rect.position.y + rect.size.y) / height * 2.0;
    let color = rect.color;
    let vertex = |position| UiVertex { position, color };

    out.extend_from_slice(&[
        vertex([x0, y0]),
        vertex([x0, y1]),
        vertex([x1, y1]),
        vertex([x0, y0]),
        vertex([x1, y1]),
        vertex([x1, y0]),
    ]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices_for(rect: UiRect, width: f32, height: f32) -> Vec<UiVertex> {
        let mut out = Vec::new();
        push_rect(&mut out, &rect, width, height);
        out
    }

    fn assert_position(vertex: &UiVertex, expected: [f32; 2]) {
        assert!(
            (vertex.position[0] - expected[0]).abs() < 1e-6,
            "{vertex:?}"
        );
        assert!(
            (vertex.position[1] - expected[1]).abs() < 1e-6,
            "{vertex:?}"
        );
    }

    #[test]
    fn rect_maps_pixels_to_normalized_device_coordinates() {
        let rect = UiRect::new(
            Vec2::new(10.0, 5.0),
            Vec2::new(20.0, 10.0),
            [0.25, 0.5, 0.75, 0.5],
        );
        let vertices = vertices_for(rect, 100.0, 50.0);

        assert_eq!(vertices.len(), 6);
        assert_position(&vertices[0], [-0.8, 0.8]);
        assert_position(&vertices[1], [-0.8, 0.4]);
        assert_position(&vertices[2], [-0.4, 0.4]);
        assert_position(&vertices[4], [-0.4, 0.4]);
        assert_position(&vertices[5], [-0.4, 0.8]);
        assert!(vertices.iter().all(|vertex| vertex.color == rect.color));
    }

    #[test]
    fn full_surface_rect_covers_the_clip_space() {
        let rect = UiRect::new(Vec2::ZERO, Vec2::new(80.0, 40.0), [1.0; 4]);
        let vertices = vertices_for(rect, 80.0, 40.0);

        assert_position(&vertices[0], [-1.0, 1.0]);
        assert_position(&vertices[2], [1.0, -1.0]);
    }

    #[test]
    fn invalid_or_empty_rects_are_skipped() {
        let cases = [
            UiRect::new(Vec2::ZERO, Vec2::new(0.0, 10.0), [1.0; 4]),
            UiRect::new(Vec2::ZERO, Vec2::new(-5.0, 10.0), [1.0; 4]),
            UiRect::new(Vec2::new(f32::NAN, 0.0), Vec2::new(5.0, 10.0), [1.0; 4]),
            UiRect::new(Vec2::ZERO, Vec2::new(5.0, f32::INFINITY), [1.0; 4]),
            UiRect::new(Vec2::ZERO, Vec2::new(5.0, 10.0), [f32::NAN; 4]),
        ];

        for rect in cases {
            assert!(vertices_for(rect, 100.0, 100.0).is_empty(), "{rect:?}");
        }
        assert!(vertices_for(UiRect::new(Vec2::ZERO, Vec2::ONE, [1.0; 4]), 0.0, 10.0).is_empty());
    }

    #[test]
    fn rect_draws_offscreen() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let size = 64u32;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut ui = UiRenderer::new(&device, format);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui test target"),
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

        let rect = UiRect::new(
            Vec2::new(8.0, 8.0),
            Vec2::new(32.0, 32.0),
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(ui.write(&queue, size, size, &[rect]), 1);

        let bytes_per_row = size * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui readback"),
            size: (bytes_per_row * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ui test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui test pass"),
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
            ui.draw(&mut pass);
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
        let at = |x: u32, y: u32| pixels[(y * size + x) as usize];
        let inside = at(24, 24);
        let outside = at(2, 2);
        drop(data);
        readback.unmap();

        assert!(
            inside[0] > 200 && inside[1] < 10 && inside[2] < 10,
            "{inside:?}"
        );
        assert!(
            outside[0] < 10 && outside[1] < 10 && outside[2] < 10,
            "{outside:?}"
        );
    }

    const IMAGE_TEST_SIZE: u32 = 64;

    fn render_images(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut ImageRenderer,
        images: &[UiImage<'_>],
    ) -> Vec<[u8; 4]> {
        let size = IMAGE_TEST_SIZE;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        assert_eq!(renderer.write(queue, size, size, images), images.len());

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image test target"),
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
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bytes_per_row = size * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image readback"),
            size: (bytes_per_row * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("image test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("image test pass"),
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
            renderer.draw(&mut pass, images);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
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
        let (pixels, _) = data.as_ref().as_chunks::<4>();
        let pixels = pixels.to_vec();
        drop(data);
        readback.unmap();
        pixels
    }

    fn test_image_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut ImageRenderer,
        options: crate::renderer::TextureOptions,
    ) -> Texture2d {
        Texture2d::new(
            device,
            queue,
            renderer.layout(),
            crate::renderer::texture::TextureUpload {
                label: "test image",
                width: 2,
                height: 1,
                rgba8: &[255, 0, 0, 255, 0, 255, 0, 255],
                options,
            },
        )
        .expect("texture")
    }

    #[test]
    fn image_samples_the_texture_across_the_draw_rect() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ImageRenderer::new(&device, format);
        let texture = test_image_texture(&device, &queue, &mut renderer, Default::default());
        let size = IMAGE_TEST_SIZE as f32;
        let image = UiImage::new(&texture, Vec2::ZERO, Vec2::new(size, size));
        let pixels = render_images(&device, &queue, &mut renderer, &[image]);

        let at = |x: u32, y: u32| pixels[(y * IMAGE_TEST_SIZE + x) as usize];
        let left = at(12, IMAGE_TEST_SIZE / 2);
        let right = at(IMAGE_TEST_SIZE - 13, IMAGE_TEST_SIZE / 2);

        assert!(left[0] > left[1], "left pixel was {left:?}");
        assert!(right[1] > right[0], "right pixel was {right:?}");
    }

    #[test]
    fn image_region_samples_a_sub_rect() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ImageRenderer::new(&device, format);
        let texture = test_image_texture(&device, &queue, &mut renderer, Default::default());
        let size = IMAGE_TEST_SIZE as f32;
        let image = UiImage::new(&texture, Vec2::ZERO, Vec2::new(size, size))
            .with_region(texture.region(1, 0, 1, 1));
        let pixels = render_images(&device, &queue, &mut renderer, &[image]);

        let center =
            pixels[((IMAGE_TEST_SIZE / 2) * IMAGE_TEST_SIZE + IMAGE_TEST_SIZE / 2) as usize];
        assert!(center[1] > center[0], "center pixel was {center:?}");
    }

    #[test]
    fn repeat_addressing_tiles_a_region_wider_than_the_texture() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ImageRenderer::new(&device, format);
        let texture = test_image_texture(
            &device,
            &queue,
            &mut renderer,
            crate::renderer::TextureOptions::NEAREST_REPEAT,
        );
        let size = IMAGE_TEST_SIZE as f32;
        let image = UiImage::new(&texture, Vec2::ZERO, Vec2::new(size, size)).with_region(
            crate::renderer::TextureRegion::new(Vec2::new(0.0, 0.0), Vec2::new(2.0, 1.0)),
        );
        let pixels = render_images(&device, &queue, &mut renderer, &[image]);

        let at = |x: u32| pixels[((IMAGE_TEST_SIZE / 2) * IMAGE_TEST_SIZE + x) as usize];
        let first = at(8);
        let second = at(24);
        let third = at(40);
        let fourth = at(56);

        assert!(first[0] > first[1], "first tile was {first:?}");
        assert!(second[1] > second[0], "second tile was {second:?}");
        assert!(third[0] > third[1], "third tile was {third:?}");
        assert!(fourth[1] > fourth[0], "fourth tile was {fourth:?}");
    }

    #[test]
    fn clip_rounds_outward_to_cover_touched_pixels() {
        let clip = UiClip::new(Vec2::new(10.4, 20.6), Vec2::new(30.2, 40.9));
        assert_eq!(
            clip.to_scissor(200, 200),
            Some(Scissor {
                x: 10,
                y: 20,
                width: 31,
                height: 42
            })
        );
    }

    #[test]
    fn clip_clamps_to_the_surface_and_rejects_empty_or_invalid_bounds() {
        let straddling = UiClip::new(Vec2::new(-5.0, -5.0), Vec2::new(20.0, 20.0));
        assert_eq!(
            straddling.to_scissor(100, 100),
            Some(Scissor {
                x: 0,
                y: 0,
                width: 15,
                height: 15
            })
        );

        let outside = UiClip::new(Vec2::new(200.0, 0.0), Vec2::new(10.0, 10.0));
        assert_eq!(outside.to_scissor(100, 100), None);

        let empty = UiClip::new(Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0));
        assert_eq!(empty.to_scissor(100, 100), None);

        let invalid = UiClip::new(Vec2::ZERO, Vec2::new(f32::NAN, 4.0));
        assert_eq!(invalid.to_scissor(100, 100), None);
    }

    #[test]
    fn image_corners_rotate_clockwise_around_the_center() {
        let corners = rotated_corners(
            Vec2::new(10.0, 20.0),
            Vec2::new(4.0, 2.0),
            std::f32::consts::FRAC_PI_2,
        )
        .expect("corners");
        let expected = [[13.0, 19.0], [13.0, 23.0], [11.0, 23.0], [11.0, 19.0]];
        for (corner, expected) in corners.iter().zip(expected) {
            assert!((corner[0] - expected[0]).abs() < 1e-5, "{corner:?}");
            assert!((corner[1] - expected[1]).abs() < 1e-5, "{corner:?}");
        }
    }

    #[test]
    fn image_corners_reject_non_finite_rotation() {
        assert!(rotated_corners(Vec2::ZERO, Vec2::new(4.0, 4.0), f32::NAN).is_none());
    }

    #[test]
    fn image_clip_limits_draw_to_the_clip_rect() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ImageRenderer::new(&device, format);
        let texture = test_image_texture(
            &device,
            &queue,
            &mut renderer,
            crate::renderer::TextureOptions::NEAREST,
        );
        let size = IMAGE_TEST_SIZE as f32;
        let image = UiImage::new(&texture, Vec2::ZERO, Vec2::new(size, size))
            .with_clip(UiClip::new(Vec2::ZERO, Vec2::new(size / 2.0, size)));
        let pixels = render_images(&device, &queue, &mut renderer, &[image]);

        let at = |x: u32, y: u32| pixels[(y * IMAGE_TEST_SIZE + x) as usize];
        let inside = at(IMAGE_TEST_SIZE / 4, IMAGE_TEST_SIZE / 2);
        let outside = at(IMAGE_TEST_SIZE - 4, IMAGE_TEST_SIZE / 2);

        assert!(inside[0] > inside[1], "inside pixel was {inside:?}");
        assert!(
            outside[0] < 8 && outside[1] < 8 && outside[2] < 8,
            "outside pixel was {outside:?}"
        );
    }

    #[test]
    fn image_rotation_turns_the_texture_clockwise() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ImageRenderer::new(&device, format);
        let texture = test_image_texture(
            &device,
            &queue,
            &mut renderer,
            crate::renderer::TextureOptions::NEAREST,
        );
        let size = IMAGE_TEST_SIZE as f32;
        let image = UiImage::new(&texture, Vec2::ZERO, Vec2::new(size, size))
            .with_rotation(std::f32::consts::FRAC_PI_2);
        let pixels = render_images(&device, &queue, &mut renderer, &[image]);

        let at = |x: u32, y: u32| pixels[(y * IMAGE_TEST_SIZE + x) as usize];
        let top = at(IMAGE_TEST_SIZE / 2, 8);
        let bottom = at(IMAGE_TEST_SIZE / 2, IMAGE_TEST_SIZE - 8);

        assert!(top[0] > top[1], "top pixel was {top:?}");
        assert!(bottom[1] > bottom[0], "bottom pixel was {bottom:?}");
    }
}
