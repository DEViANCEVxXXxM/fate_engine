//! Screen-space text rendering.
//!
//! The renderer ships a built-in 5x7 bitmap font, so consumers can draw a HUD
//! without loading a font file or adding a dependency. Text is rasterized into
//! one coverage atlas at startup and drawn as one instanced-free quad batch in
//! physical pixels, on top of whatever the frame already drew.
//!
//! The layout helpers are pure functions with headless tests; the pipeline is
//! only built when a device exists.

use bytemuck::{Pod, Zeroable};

use super::ui::{UiClip, apply_scissor};
use crate::math::Vec2;

/// Width of one glyph cell in atlas pixels.
pub const GLYPH_WIDTH: u32 = 5;
/// Height of one glyph cell in atlas pixels.
pub const GLYPH_HEIGHT: u32 = 7;
/// Horizontal distance between glyph origins, including inter-glyph spacing.
pub const GLYPH_ADVANCE: u32 = 6;
/// Vertical distance between line origins.
pub const LINE_HEIGHT: u32 = 8;
/// First character covered by the built-in font.
pub const FIRST_GLYPH: u32 = 0x20;
/// Number of characters covered by the built-in font: printable ASCII.
pub const GLYPH_COUNT: usize = 95;
/// Upper bound on glyph quads submitted in one frame.
pub const MAX_TEXT_GLYPHS: usize = 2_048;

const ATLAS_COLUMNS: u32 = 16;
const ATLAS_ROWS: u32 = 6;
const CELL_WIDTH: u32 = 8;
const CELL_HEIGHT: u32 = 8;
const ATLAS_WIDTH: u32 = ATLAS_COLUMNS * CELL_WIDTH;
const ATLAS_HEIGHT: u32 = ATLAS_ROWS * CELL_HEIGHT;

/// One run of text drawn in one frame.
///
/// `position` is the top-left corner of the first glyph in physical pixels,
/// measured from the top-left of the window. `scale` multiplies every atlas
/// pixel, so a scale of 2 draws the 5x7 glyph as 10x14 pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDraw<'a> {
    pub position: Vec2,
    pub text: &'a str,
    pub scale: f32,
    pub color: [f32; 4],
    /// Optional bounds outside which nothing is drawn.
    pub clip: Option<UiClip>,
}

impl<'a> TextDraw<'a> {
    /// A white, unscaled run of text at `position`.
    pub fn new(position: Vec2, text: &'a str) -> Self {
        Self {
            position,
            text,
            scale: 1.0,
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
        }
    }

    /// Set the pixel scale.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Set the color, including alpha.
    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// Draw only the glyph pixels inside `clip`.
    pub fn with_clip(mut self, clip: UiClip) -> Self {
        self.clip = Some(clip);
        self
    }
}

/// Where one glyph lands on screen, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlyphPlacement {
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// `min_u, min_v, max_u, max_v` in atlas texture space.
    pub uv: [f32; 4],
}

/// The built-in font, one entry per printable ASCII character from `0x20`.
///
/// Each entry is seven five-wide rows separated by `/`, with `#` for a lit
/// pixel. Keeping the glyphs visible in source makes the font easy to correct.
const FONT: [&str; GLYPH_COUNT] = [
    "...../...../...../...../...../...../.....", // space
    "..#../..#../..#../..#../..#../...../..#..", // !
    ".#.#./.#.#./...../...../...../...../.....", // "
    ".#.#./.#.#./#####/.#.#./#####/.#.#./.#.#.", // #
    "..#../.####/#.#../.###./..#.#/####./..#..", // $
    "##.../##..#/...#./..#../.#.../#..##/...##", // %
    ".##../#..#./#.#../.#.../#.#.#/#..#./.##.#", // &
    "..#../..#../...../...../...../...../.....", // '
    "...#./..#../.#.../.#.../.#.../..#../...#.", // (
    ".#.../..#../...#./...#./...#./..#../.#...", // )
    "...../#.#.#/.###./#####/.###./#.#.#/.....", // *
    "...../..#../..#../#####/..#../..#../.....", // +
    "...../...../...../...../..##./..#../.#...", // ,
    "...../...../...../#####/...../...../.....", // -
    "...../...../...../...../...../.##../.##..", // .
    "....#/...#./...#./..#../.#.../.#.../#....", // /
    ".###./#...#/#..##/#.#.#/##..#/#...#/.###.", // 0
    "..#../.##../..#../..#../..#../..#../.###.", // 1
    ".###./#...#/....#/...#./..#../.#.../#####", // 2
    "#####/...#./..#../...#./....#/#...#/.###.", // 3
    "...#./..##./.#.#./#..#./#####/...#./...#.", // 4
    "#####/#..../####./....#/....#/#...#/.###.", // 5
    "..##./.#.../#..../####./#...#/#...#/.###.", // 6
    "#####/....#/...#./..#../.#.../.#.../.#...", // 7
    ".###./#...#/#...#/.###./#...#/#...#/.###.", // 8
    ".###./#...#/#...#/.####/....#/...#./.##..", // 9
    "...../.##../.##../...../.##../.##../.....", // :
    "...../.##../.##../...../.##../..#../.#...", // ;
    "...#./..#../.#.../#..../.#.../..#../...#.", // <
    "...../...../#####/...../#####/...../.....", // =
    ".#.../..#../...#./....#/...#./..#../.#...", // >
    ".###./#...#/....#/...#./..#../...../..#..", // ?
    ".###./#...#/#.###/#.#.#/#.###/#..../.###.", // @
    ".###./#...#/#...#/#####/#...#/#...#/#...#", // A
    "####./#...#/#...#/####./#...#/#...#/####.", // B
    ".###./#...#/#..../#..../#..../#...#/.###.", // C
    "###../#..#./#...#/#...#/#...#/#..#./###..", // D
    "#####/#..../#..../####./#..../#..../#####", // E
    "#####/#..../#..../####./#..../#..../#....", // F
    ".###./#...#/#..../#.###/#...#/#...#/.###.", // G
    "#...#/#...#/#...#/#####/#...#/#...#/#...#", // H
    ".###./..#../..#../..#../..#../..#../.###.", // I
    "..###/...#./...#./...#./...#./#..#./.##..", // J
    "#...#/#..#./#.#../##.../#.#../#..#./#...#", // K
    "#..../#..../#..../#..../#..../#..../#####", // L
    "#...#/##.##/#.#.#/#.#.#/#...#/#...#/#...#", // M
    "#...#/##..#/#.#.#/#..##/#...#/#...#/#...#", // N
    ".###./#...#/#...#/#...#/#...#/#...#/.###.", // O
    "####./#...#/#...#/####./#..../#..../#....", // P
    ".###./#...#/#...#/#...#/#.#.#/#..#./.##.#", // Q
    "####./#...#/#...#/####./#.#../#..#./#...#", // R
    ".####/#..../#..../.###./....#/....#/####.", // S
    "#####/..#../..#../..#../..#../..#../..#..", // T
    "#...#/#...#/#...#/#...#/#...#/#...#/.###.", // U
    "#...#/#...#/#...#/#...#/#...#/.#.#./..#..", // V
    "#...#/#...#/#...#/#.#.#/#.#.#/##.##/#...#", // W
    "#...#/#...#/.#.#./..#../.#.#./#...#/#...#", // X
    "#...#/#...#/.#.#./..#../..#../..#../..#..", // Y
    "#####/....#/...#./..#../.#.../#..../#####", // Z
    ".###./.#.../.#.../.#.../.#.../.#.../.###.", // [
    "#..../#..../.#.../..#../...#./....#/....#", // \
    "###../..#../..#../..#../..#../..#../###..", // ]
    "..#../.#.#./#...#/...../...../...../.....", // ^
    "...../...../...../...../...../...../#####", // _
    ".#.../..#../...../...../...../...../.....", // `
    "...../...../.###./....#/.####/#...#/.####", // a
    "#..../#..../####./#...#/#...#/#...#/####.", // b
    "...../...../.###./#..../#..../#...#/.###.", // c
    "....#/....#/.####/#...#/#...#/#...#/.####", // d
    "...../...../.###./#...#/#####/#..../.###.", // e
    "..##./.#..#/.#.../###../.#.../.#.../.#...", // f
    "...../.####/#...#/#...#/.####/....#/.###.", // g
    "#..../#..../####./#...#/#...#/#...#/#...#", // h
    "..#../...../.##../..#../..#../..#../.###.", // i
    "...#./...../..##./...#./...#./#..#./.##..", // j
    "#..../#..../#..#./#.#../##.../#.#../#..#.", // k
    ".##../..#../..#../..#../..#../..#../.###.", // l
    "...../...../##.#./#.#.#/#.#.#/#...#/#...#", // m
    "...../...../####./#...#/#...#/#...#/#...#", // n
    "...../...../.###./#...#/#...#/#...#/.###.", // o
    "...../####./#...#/#...#/####./#..../#....", // p
    "...../.####/#...#/#...#/.####/....#/....#", // q
    "...../...../#.##./##..#/#..../#..../#....", // r
    "...../...../.###./#..../.###./....#/####.", // s
    ".#.../.#.../###../.#.../.#.../.#..#/..##.", // t
    "...../...../#...#/#...#/#...#/#..##/.##.#", // u
    "...../...../#...#/#...#/#...#/.#.#./..#..", // v
    "...../...../#...#/#...#/#.#.#/#.#.#/.#.#.", // w
    "...../...../#...#/.#.#./..#../.#.#./#...#", // x
    "...../#...#/#...#/#...#/.####/....#/.###.", // y
    "...../...../#####/...#./..#../.#.../#####", // z
    "...#./..#../..#../.#.../..#../..#../...#.", // {
    "..#../..#../..#../..#../..#../..#../..#..", // |
    ".#.../..#../..#../...#./..#../..#../.#...", // }
    "...../...../.#..#/#.#.#/#..#./...../.....", // ~
];

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl TextVertex {
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextUniforms {
    screen_size: [f32; 2],
    _padding: [f32; 2],
}

impl TextUniforms {
    fn new(width: f32, height: f32) -> Self {
        Self {
            screen_size: [width.max(1.0), height.max(1.0)],
            _padding: [0.0; 2],
        }
    }
}

/// Map a character onto an atlas cell, substituting `?` for anything the font
/// does not cover.
pub(crate) fn glyph_index(ch: char) -> usize {
    let code = u32::from(ch);
    if (FIRST_GLYPH..FIRST_GLYPH + GLYPH_COUNT as u32).contains(&code) {
        (code - FIRST_GLYPH) as usize
    } else {
        (u32::from('?') - FIRST_GLYPH) as usize
    }
}

/// Whether a glyph has no lit pixels, so it can skip its quad.
fn is_blank(index: usize) -> bool {
    !FONT[index].contains('#')
}

/// Rasterize the built-in font into an 8-bit coverage atlas.
///
/// One byte per texel; lit glyph pixels are `255` and everything else is `0`.
pub(crate) fn build_atlas() -> Vec<u8> {
    let mut pixels = vec![0u8; (ATLAS_WIDTH * ATLAS_HEIGHT) as usize];
    for (index, glyph) in FONT.iter().enumerate() {
        let cell_x = (index as u32 % ATLAS_COLUMNS) * CELL_WIDTH;
        let cell_y = (index as u32 / ATLAS_COLUMNS) * CELL_HEIGHT;
        for (row, line) in glyph.split('/').enumerate() {
            debug_assert_eq!(line.len(), GLYPH_WIDTH as usize, "font row width");
            for (column, pixel) in line.bytes().enumerate() {
                if pixel == b'#' {
                    let x = cell_x + column as u32;
                    let y = cell_y + row as u32;
                    pixels[(y * ATLAS_WIDTH + x) as usize] = 0xFF;
                }
            }
        }
        debug_assert_eq!(glyph.split('/').count(), GLYPH_HEIGHT as usize);
    }
    pixels
}

/// Width of the widest line in `text` at `scale`, in physical pixels.
///
/// Trailing inter-glyph spacing is excluded, so the result is the width the
/// drawn pixels occupy.
pub fn text_width(text: &str, scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0.0;
    }
    let mut widest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        match ch {
            '\n' => {
                widest = widest.max(current);
                current = 0;
            }
            '\r' => {}
            '\t' => current += 4,
            _ => current += 1,
        }
    }
    glyph_span_width(widest.max(current), scale)
}

fn glyph_span_width(count: usize, scale: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    (count as f32 * GLYPH_ADVANCE as f32 - (GLYPH_ADVANCE - GLYPH_WIDTH) as f32) * scale
}

/// Split `text` into lines no wider than `max_width` at `scale`.
///
/// Words wrap at ASCII whitespace, explicit `\n` starts a new line, and blank
/// paragraphs produce empty strings. A single word wider than `max_width` is
/// hard-broken at character boundaries, so the result stays within the limit
/// unless one glyph alone is wider. Whitespace inside a paragraph is collapsed
/// to single spaces.
///
/// Returns the original paragraph structure when `max_width` or `scale` is not
/// positive and finite, so callers can render the unwrapped fallback.
pub fn wrap_text(text: &str, max_width: f32, scale: f32) -> Vec<String> {
    let paragraphs = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    if !max_width.is_finite() || max_width <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return paragraphs.map(str::to_owned).collect();
    }

    let fits = |count| glyph_span_width(count, scale) <= max_width;
    let mut lines = Vec::new();
    for paragraph in paragraphs {
        let mut current = String::new();
        let mut count = 0;
        for word in paragraph.split_ascii_whitespace() {
            let word_count = word.chars().count();
            let candidate = count + usize::from(count != 0) + word_count;
            if fits(candidate) {
                if count != 0 {
                    current.push(' ');
                }
                current.push_str(word);
                count = candidate;
                continue;
            }
            if count != 0 {
                lines.push(std::mem::take(&mut current));
                count = 0;
            }
            if fits(word_count) {
                current.push_str(word);
                count = word_count;
                continue;
            }
            // The word does not fit on an empty line, so break it by glyphs.
            for ch in word.chars() {
                if count != 0 && !fits(count + 1) {
                    lines.push(std::mem::take(&mut current));
                    count = 0;
                }
                current.push(ch);
                count += 1;
            }
        }
        lines.push(current);
    }
    lines
}

/// Lay out one run of text into glyph placements.
///
/// `out` is cleared first so callers can reuse its allocation. Runs whose scale
/// is not a positive finite number produce no placements.
pub(crate) fn layout_text(text: &str, origin: Vec2, scale: f32, out: &mut Vec<GlyphPlacement>) {
    out.clear();
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }

    let advance = GLYPH_ADVANCE as f32 * scale;
    let line_height = LINE_HEIGHT as f32 * scale;
    let glyph_size = [GLYPH_WIDTH as f32 * scale, GLYPH_HEIGHT as f32 * scale];
    let mut cursor = origin;

    for ch in text.chars() {
        match ch {
            '\n' => {
                cursor.x = origin.x;
                cursor.y += line_height;
                continue;
            }
            '\r' => continue,
            '\t' => {
                cursor.x += advance * 4.0;
                continue;
            }
            _ => {}
        }

        let index = glyph_index(ch);
        if !is_blank(index) {
            let cell_x = (index as u32 % ATLAS_COLUMNS) * CELL_WIDTH;
            let cell_y = (index as u32 / ATLAS_COLUMNS) * CELL_HEIGHT;
            let atlas_width = ATLAS_WIDTH as f32;
            let atlas_height = ATLAS_HEIGHT as f32;
            out.push(GlyphPlacement {
                position: [cursor.x, cursor.y],
                size: glyph_size,
                uv: [
                    cell_x as f32 / atlas_width,
                    cell_y as f32 / atlas_height,
                    (cell_x + GLYPH_WIDTH) as f32 / atlas_width,
                    (cell_y + GLYPH_HEIGHT) as f32 / atlas_height,
                ],
            });
        }
        cursor.x += advance;
    }
}

/// Copy the latest glyph batch, the atlas and the text pipeline.
pub(crate) struct TextRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    placements: Vec<GlyphPlacement>,
    scratch: Vec<TextVertex>,
    runs: Vec<TextRun>,
    surface: (u32, u32),
    last_clipped: bool,
}

/// A contiguous glyph-vertex range that shares one clip rectangle.
#[derive(Clone, Copy, Debug)]
struct TextRun {
    first_vertex: u32,
    vertex_count: u32,
    clip: Option<UiClip>,
}

impl TextRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let atlas = build_atlas();
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ATLAS_WIDTH),
                rows_per_image: Some(ATLAS_HEIGHT),
            },
            wgpu::Extent3d {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("text.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<TextUniforms>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text uniforms"),
            size: std::mem::size_of::<TextUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(TextVertex::layout())],
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
            label: Some("text vertices"),
            size: (MAX_TEXT_GLYPHS * 6 * std::mem::size_of::<TextVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertices,
            uniforms,
            bind_group,
            placements: Vec::new(),
            scratch: Vec::new(),
            runs: Vec::new(),
            surface: (0, 0),
            last_clipped: false,
        }
    }

    /// Build this frame's glyph batch and upload it. Call before the pass.
    pub(crate) fn write(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        draws: &[TextDraw<'_>],
    ) {
        self.scratch.clear();
        self.runs.clear();
        self.surface = (width, height);
        let mut clipped = false;

        'draws: for draw in draws {
            layout_text(draw.text, draw.position, draw.scale, &mut self.placements);
            let first_vertex = self.scratch.len() as u32;
            for placement in &self.placements {
                if self.scratch.len() / 6 >= MAX_TEXT_GLYPHS {
                    clipped = true;
                    let vertex_count = self.scratch.len() as u32 - first_vertex;
                    if vertex_count > 0 {
                        self.runs.push(TextRun {
                            first_vertex,
                            vertex_count,
                            clip: draw.clip,
                        });
                    }
                    break 'draws;
                }
                push_quad(&mut self.scratch, placement, draw.color);
            }
            let vertex_count = self.scratch.len() as u32 - first_vertex;
            if vertex_count > 0 {
                self.runs.push(TextRun {
                    first_vertex,
                    vertex_count,
                    clip: draw.clip,
                });
            }
        }

        if clipped && !self.last_clipped {
            log::warn!(
                "text budget exceeded: {MAX_TEXT_GLYPHS} glyphs per frame; dropping the rest of the frame's text",
            );
        }
        self.last_clipped = clipped;

        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&TextUniforms::new(width as f32, height as f32)),
        );
        if !self.scratch.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.scratch));
        }
    }

    /// Record the glyph batch into an existing pass.
    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.runs.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
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

/// Append two triangles covering one glyph placement.
fn push_quad(out: &mut Vec<TextVertex>, placement: &GlyphPlacement, color: [f32; 4]) {
    let [x, y] = placement.position;
    let [width, height] = placement.size;
    let [u0, v0, u1, v1] = placement.uv;
    let corners = [
        ([x, y], [u0, v0]),
        ([x + width, y], [u1, v0]),
        ([x + width, y + height], [u1, v1]),
        ([x, y + height], [u0, v1]),
    ];
    for index in [0usize, 1, 2, 0, 2, 3] {
        let (position, uv) = corners[index];
        out.push(TextVertex {
            position,
            uv,
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(text: &str, scale: f32) -> Vec<GlyphPlacement> {
        let mut out = Vec::new();
        layout_text(text, Vec2::new(0.0, 0.0), scale, &mut out);
        out
    }

    #[test]
    fn atlas_lights_glyph_pixels_and_leaves_space_blank() {
        let atlas = build_atlas();
        assert_eq!(atlas.len(), (ATLAS_WIDTH * ATLAS_HEIGHT) as usize);

        let index = glyph_index('A');
        let cell_x = (index as u32 % ATLAS_COLUMNS) * CELL_WIDTH;
        let cell_y = (index as u32 / ATLAS_COLUMNS) * CELL_HEIGHT;
        let lit = (0..GLYPH_HEIGHT)
            .flat_map(|row| (0..GLYPH_WIDTH).map(move |column| (column, row)))
            .filter(|(column, row)| {
                atlas[((cell_y + row) * ATLAS_WIDTH + cell_x + column) as usize] != 0
            })
            .count();
        assert!(lit > 10, "expected 'A' to have lit pixels, got {lit}");

        let space = glyph_index(' ');
        let cell_x = (space as u32 % ATLAS_COLUMNS) * CELL_WIDTH;
        let cell_y = (space as u32 / ATLAS_COLUMNS) * CELL_HEIGHT;
        let lit = (0..GLYPH_HEIGHT)
            .flat_map(|row| (0..GLYPH_WIDTH).map(move |column| (column, row)))
            .filter(|(column, row)| {
                atlas[((cell_y + row) * ATLAS_WIDTH + cell_x + column) as usize] != 0
            })
            .count();
        assert_eq!(lit, 0);
    }

    #[test]
    fn layout_advances_per_glyph_and_wraps_lines() {
        let placements = layout("AB\nC", 2.0);
        // The newline itself draws nothing, so only three glyphs remain.
        assert_eq!(placements.len(), 3);
        assert_eq!(placements[0].position, [0.0, 0.0]);
        assert_eq!(placements[1].position, [GLYPH_ADVANCE as f32 * 2.0, 0.0]);
        assert_eq!(placements[2].position, [0.0, LINE_HEIGHT as f32 * 2.0]);
        assert_eq!(placements[0].size, [10.0, 14.0]);
    }

    #[test]
    fn layout_skips_blanks_and_ignores_non_positive_scale() {
        let placements = layout("A A", 1.0);
        assert_eq!(placements.len(), 2, "the space draws no quad");
        assert_eq!(placements[1].position[0], GLYPH_ADVANCE as f32 * 2.0);

        assert!(layout("A", 0.0).is_empty());
        assert!(layout("A", f32::NAN).is_empty());
        assert!(layout("A", -1.0).is_empty());
    }

    #[test]
    fn tabs_advance_four_cells() {
        let placements = layout("\tA", 1.0);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].position[0], GLYPH_ADVANCE as f32 * 4.0);
    }

    #[test]
    fn unknown_characters_fall_back_to_a_question_mark() {
        assert_eq!(glyph_index('é'), glyph_index('?'));
        assert_eq!(glyph_index('\u{1F600}'), glyph_index('?'));
        assert_eq!(glyph_index('~'), (u32::from('~') - FIRST_GLYPH) as usize);
    }

    #[test]
    fn text_width_excludes_trailing_spacing() {
        assert_eq!(text_width("", 1.0), 0.0);
        assert_eq!(text_width("A", 1.0), GLYPH_WIDTH as f32);
        assert_eq!(text_width("AB", 1.0), (GLYPH_ADVANCE * 2 - 1) as f32);
        assert_eq!(text_width("AB", 2.0), ((GLYPH_ADVANCE * 2 - 1) * 2) as f32);
        assert_eq!(text_width("AB\nABCD", 1.0), (GLYPH_ADVANCE * 4 - 1) as f32);
        assert_eq!(text_width("A", 0.0), 0.0);
    }

    #[test]
    fn wrap_text_breaks_at_word_boundaries() {
        let lines = wrap_text("AA AA", 11.0, 1.0);
        assert_eq!(lines, ["AA", "AA"]);
        assert!(lines.iter().all(|line| text_width(line, 1.0) <= 11.0));
    }

    #[test]
    fn wrap_text_hard_breaks_a_word_that_cannot_fit() {
        let lines = wrap_text("AAAAA", 11.0, 1.0);
        assert_eq!(lines, ["AA", "AA", "A"]);
    }

    #[test]
    fn wrap_text_keeps_exact_fits_at_fractional_scale() {
        let scale = 0.41;
        assert_eq!(wrap_text("A B", text_width("A B", scale), scale), ["A B"]);
        assert_eq!(
            wrap_text("ABC", text_width("AB", scale), scale),
            ["AB", "C"]
        );
    }

    #[test]
    fn wrap_text_collapses_ascii_whitespace_but_keeps_non_ascii_characters() {
        let text = " \tA\u{a0}B\t C ";
        assert_eq!(wrap_text(text, 29.0, 1.0), ["A\u{a0}B C"]);
        assert_eq!(wrap_text(text, 23.0, 1.0), ["A\u{a0}B", "C"]);
        assert_eq!(wrap_text(text, 11.0, 1.0), ["A\u{a0}", "B", "C"]);
    }

    #[test]
    fn wrap_text_makes_progress_when_even_one_glyph_is_too_wide() {
        assert_eq!(wrap_text("ABC", 1.0, 1.0), ["A", "B", "C"]);
    }

    #[test]
    fn wrap_text_preserves_paragraphs_and_blank_lines() {
        let lines = wrap_text("AB\r\n\nCD", 100.0, 1.0);
        assert_eq!(lines, ["AB", "", "CD"]);
    }

    #[test]
    fn wrap_text_keeps_the_fallback_when_the_limit_is_invalid() {
        assert_eq!(wrap_text("AB CD", 0.0, 1.0), ["AB CD"]);
        assert_eq!(wrap_text("AB CD", 100.0, f32::NAN), ["AB CD"]);
        assert_eq!(wrap_text("AB\nCD", f32::INFINITY, 1.0), ["AB", "CD"]);
    }

    #[test]
    fn wrap_text_keeps_every_line_within_the_limit() {
        let lines = wrap_text(
            "WRAP THESE WORDS INTO SEVERAL HUD LINES WITHOUT OVERFLOW",
            80.0,
            1.0,
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| text_width(line, 1.0) <= 80.0));
    }

    #[test]
    fn quad_push_emits_two_triangles_per_glyph() {
        let placements = layout("AB", 1.0);
        let mut vertices = Vec::new();
        for placement in &placements {
            push_quad(&mut vertices, placement, [1.0, 0.5, 0.25, 1.0]);
        }
        assert_eq!(vertices.len(), 12);
        assert_eq!(vertices[0].position, [0.0, 0.0]);
        assert_eq!(vertices[0].color, [1.0, 0.5, 0.25, 1.0]);
    }

    /// Draw text into an offscreen target and count lit pixels, so the atlas,
    /// shader and pipeline are covered as a whole.
    #[test]
    fn text_is_visible_offscreen() {
        use crate::renderer::test_support::{skip_without_adapter, test_device};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };

        let width = 128u32;
        let height = 32u32;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut text = TextRenderer::new(&device, &queue, format);
        text.write(
            &queue,
            width,
            height,
            &[TextDraw::new(Vec2::new(4.0, 4.0), "Hi").with_scale(3.0)],
        );

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text test target"),
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

        let bytes_per_row = width * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("text test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text test pass"),
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
            text.draw(&mut pass);
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
        let (pixels, _) = data.as_ref().as_chunks::<4>();
        let lit = pixels
            .iter()
            .filter(|px| px[0] > 10 || px[1] > 10 || px[2] > 10)
            .count();
        drop(data);
        readback.unmap();

        assert!(lit > 50, "expected text pixels, got {lit} lit pixels");
    }
}
