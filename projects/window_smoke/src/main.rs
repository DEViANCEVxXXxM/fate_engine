//! Minimal windowed consumer for the renderer's public seam.
//!
//! This app exists to answer one question: can a project that is not the demo
//! create a window, own the surface, upload GPU resources, and submit a draw
//! list using only the documented `fate_engine` API? It intentionally does not
//! depend on `wgpu` directly.
//!
//! It exits after a fixed number of redraws so it can run unattended.

mod lifecycle;

use std::sync::Arc;
use std::time::{Duration, Instant};

use fate_engine::math::{Mat4, Vec2, Vec3};
use fate_engine::renderer::{
    Draw3d, Mesh3d, MeshVertex, Overlay, Renderer, SceneDraw3d, TextDraw, Texture2d,
    TextureOptions, TextureRegion, UiClip, UiImage, UiRect, UvMeshVertex, text_width, wrap_text,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use lifecycle::Lifecycle;

const FRAME_LIMIT: u32 = 180;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SPIN_RATE: f32 = 0.05;
const GRID_SIDE: usize = 16;
const GRID_DRAWS: usize = GRID_SIDE * GRID_SIDE;
const LOAD_WARMUP: u32 = 30;

const HUD_MARGIN: f32 = 24.0;
const HUD_LINE_GAP: f32 = 8.0;
const HUD_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const HUD_DIM: [f32; 4] = [0.75, 0.8, 0.85, 1.0];
const HUD_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.65];
const PANEL_FILL: [f32; 4] = [0.03, 0.04, 0.06, 0.78];
const PANEL_ACCENT: [f32; 4] = [0.35, 0.6, 1.0, 0.95];
const PANEL_ACCENT_WIDTH: f32 = 4.0;
const ATLAS_WIDTH: u32 = 32;
const ATLAS_HEIGHT: u32 = 16;
const ATLAS_TILE: u32 = 16;
const IMAGE_SIZE: f32 = 56.0;
const IMAGE_GAP: f32 = 10.0;
const TILE_STRIP_HEIGHT: f32 = 12.0;
const TILE_REPEATS: f32 = 8.0;
const CLIP_PREVIEW: f32 = 40.0;
const CLIP_OVERFLOW: f32 = 12.0;
/// Slight clockwise tilt so the checker icon shows rotated geometry.
const ICON_ROTATION: f32 = 0.16;

const PYRAMID_VERTICES: &[MeshVertex] = &[
    MeshVertex {
        position: [0.0, 0.7, 0.0],
        color: [1.0, 0.85, 0.2],
    },
    MeshVertex {
        position: [-0.6, -0.4, 0.6],
        color: [0.2, 0.6, 1.0],
    },
    MeshVertex {
        position: [0.6, -0.4, 0.6],
        color: [0.2, 1.0, 0.6],
    },
    MeshVertex {
        position: [0.6, -0.4, -0.6],
        color: [1.0, 0.3, 0.3],
    },
    MeshVertex {
        position: [-0.6, -0.4, -0.6],
        color: [0.8, 0.4, 1.0],
    },
];

const PYRAMID_INDICES: &[u32] = &[
    0, 1, 2, // front
    0, 2, 3, // right
    0, 3, 4, // back
    0, 4, 1, // left
    1, 4, 3, // base a
    1, 3, 2, // base b
];

fn upload_textured_pyramid(renderer: &Renderer) -> Mesh3d {
    // Each face has its own corners so the checker tile can start over at
    // each edge. Sample its texel centers to stay clear of the next atlas tile.
    let left = 0.5 / ATLAS_WIDTH as f32;
    let right = (ATLAS_TILE as f32 - 0.5) / ATLAS_WIDTH as f32;
    let top = 0.5 / ATLAS_HEIGHT as f32;
    let bottom = 1.0 - top;
    let (faces, remainder) = PYRAMID_INDICES.as_chunks::<3>();
    assert!(remainder.is_empty());
    let vertices: Vec<_> = faces
        .iter()
        .enumerate()
        .flat_map(|(face, indices)| {
            let uv = match face {
                4 => [[left, bottom], [left, top], [right, top]],
                5 => [[left, bottom], [right, top], [right, bottom]],
                _ => [[(left + right) * 0.5, top], [left, bottom], [right, bottom]],
            };
            indices.iter().zip(uv).map(|(&index, uv)| UvMeshVertex {
                position: PYRAMID_VERTICES[index as usize].position,
                color: [1.0; 3],
                uv,
            })
        })
        .collect();
    let indices: Vec<_> = (0..PYRAMID_INDICES.len() as u32).collect();
    renderer
        .create_mesh_with_uvs("textured smoke pyramid", &vertices, &indices)
        .expect("failed to create textured mesh")
}

/// Procedural RGBA8 atlas: a checker tile and a target tile side by side.
fn atlas_rgba(width: u32, height: u32, tile: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let local_x = x % tile;
            let local_y = y % tile;
            let color = if x / tile == 0 {
                let light = ((local_x / 4) + (local_y / 4)).is_multiple_of(2);
                let value = if light { 235 } else { 70 };
                [value / 3, value / 2, value, 255]
            } else {
                let border =
                    local_x < 2 || local_y < 2 || local_x + 2 >= tile || local_y + 2 >= tile;
                let cross = (local_x as i32 - tile as i32 / 2).abs() <= 1
                    || (local_y as i32 - tile as i32 / 2).abs() <= 1;
                if border || cross {
                    [235, 245, 255, 255]
                } else {
                    [28, 76, 196, 255]
                }
            };
            pixels.extend_from_slice(&color);
        }
    }
    pixels
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Showcase,
    Lifecycle,
    MeshLoad,
}

struct LoadSample {
    prepare: Duration,
    record: Duration,
    render_call: Duration,
}

fn main() {
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => Mode::Showcase,
        [mode] if mode == "--surface-lifecycle" => Mode::Lifecycle,
        [mode] if mode == "--mesh-load" => Mode::MeshLoad,
        _ => {
            eprintln!("usage: fate_window_smoke [--surface-lifecycle | --mesh-load]");
            std::process::exit(2);
        }
    };
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = SmokeApp {
        mode,
        ..Default::default()
    };
    event_loop.run_app(&mut app).expect("event loop failed");
    if mode == Mode::Lifecycle {
        assert!(
            app.lifecycle.is_some_and(|test| test.is_complete()),
            "surface lifecycle smoke exited before it completed"
        );
        println!("surface lifecycle completed: restored window; 2D and 3D submitted");
        return;
    }
    println!(
        "window smoke completed: {} redraws, {} submitted, {} skipped",
        app.redraws, app.submitted, app.skipped
    );
    if mode == Mode::MeshLoad {
        assert_eq!(app.redraws, FRAME_LIMIT, "mesh load ended early");
        assert!(
            !app.load_samples.is_empty(),
            "no mesh load frames submitted after warm-up"
        );
        let summary = |sample: fn(&LoadSample) -> Duration| {
            let mut times: Vec<_> = app.load_samples.iter().map(sample).collect();
            times.sort_unstable();
            let ms = |time: Duration| time.as_secs_f64() * 1000.0;
            (
                ms(times[(times.len() - 1) / 2]),
                ms(times[(times.len() * 95).div_ceil(100) - 1]),
            )
        };
        let (prepare_p50, prepare_p95) = summary(|sample| sample.prepare);
        let (record_p50, record_p95) = summary(|sample| sample.record);
        let (render_p50, render_p95) = summary(|sample| sample.render_call);
        println!(
            "mesh load: {GRID_DRAWS} instances, 1 indexed draw per frame, {} samples; p50/p95 CPU prep {prepare_p50:.3}/{prepare_p95:.3} ms, CPU recording {record_p50:.3}/{record_p95:.3} ms, render call {render_p50:.3}/{render_p95:.3} ms (includes surface acquire/present, not GPU time)",
            app.load_samples.len()
        );
    }
}

#[derive(Default)]
struct SmokeApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    mesh: Option<Mesh3d>,
    textured_mesh: Option<Mesh3d>,
    texture: Option<Texture2d>,
    mode: Mode,
    lifecycle: Option<Lifecycle>,
    load_samples: Vec<LoadSample>,
    redraws: u32,
    submitted: u32,
    skipped: u32,
}

impl SmokeApp {
    fn render_mesh_load(&mut self) -> bool {
        let (Some(renderer), Some(mesh)) = (self.renderer.as_mut(), self.mesh.as_ref()) else {
            return false;
        };
        let started = Instant::now();
        let angle = self.redraws as f32 * SPIN_RATE;
        let shape = Mat4::rotation_y(angle).mul(Mat4::scale(Vec3::new(0.16, 0.16, 0.16)));
        let draws: [Draw3d<'_>; GRID_DRAWS] = std::array::from_fn(|index| {
            let x = (index % GRID_SIDE) as f32 - (GRID_SIDE - 1) as f32 * 0.5;
            let y = (index / GRID_SIDE) as f32 - (GRID_SIDE - 1) as f32 * 0.5;
            Draw3d {
                mesh,
                model: Mat4::translation(Vec3::new(x * 0.29, y * 0.29, -7.0)).mul(shape),
                tint: [1.0, 0.65 + 0.35 * (index % 2) as f32, 1.0],
            }
        });
        let preparing = started.elapsed();
        let rendering = Instant::now();
        let submission = renderer.render_meshes_with_overlay_report(&draws, &Overlay::default());
        let render_call = rendering.elapsed();
        let Some(submission) = submission else {
            return false;
        };
        assert_eq!(submission.instances, GRID_DRAWS);
        assert_eq!(submission.draw_calls, 1);
        if self.redraws >= LOAD_WARMUP {
            self.load_samples.push(LoadSample {
                prepare: preparing,
                record: submission.cpu_record_time,
                render_call,
            });
        }
        true
    }

    fn render_once(&mut self) -> bool {
        let (Some(renderer), Some(mesh), Some(textured_mesh), Some(texture)) = (
            self.renderer.as_mut(),
            self.mesh.as_ref(),
            self.textured_mesh.as_ref(),
            self.texture.as_ref(),
        ) else {
            return false;
        };

        let dpi = self
            .window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor() as f32);
        let dpi = if dpi.is_finite() {
            dpi.clamp(0.5, 4.0)
        } else {
            1.0
        };
        let (surface_width, surface_height) = renderer.surface_size();

        let draws: [SceneDraw3d<'_>; 3] = std::array::from_fn(|index| {
            let offset = index as f32 * 2.2 - 2.2;
            let angle = self.redraws as f32 * SPIN_RATE + index as f32 * 1.3;
            let model = Mat4::translation(Vec3::new(offset, 0.0, -7.0))
                .mul(Mat4::rotation_y(angle))
                .mul(Mat4::rotation_x(angle * 0.5));
            let draw = Draw3d {
                mesh,
                model,
                tint: [1.0, 1.0, 1.0],
            };
            if index == 1 {
                Draw3d {
                    mesh: textured_mesh,
                    model: model.mul(Mat4::scale(Vec3::new(1.5, 1.5, 1.5))),
                    ..draw
                }
                .with_texture(texture)
            } else {
                draw.into()
            }
        });

        let margin = HUD_MARGIN * dpi;
        let gap = HUD_LINE_GAP * dpi;
        let line_scale = 2.0 * dpi;
        let line_step = 8.0 * line_scale + gap;
        let panel_pad = margin * 0.5;
        let panel_width = (surface_width as f32 - panel_pad * 2.0).max(1.0);
        let title = "FATE WINDOW SMOKE";
        let title_scale = 3.0 * dpi;
        let title_x = ((surface_width as f32 - text_width(title, title_scale)) * 0.5).max(0.0);
        let title_y = margin;
        let frame_y = title_y + 8.0 * title_scale + gap;
        let counters_y = frame_y + line_step;
        let seam_y = counters_y + line_step;
        let frame = format!("FRAME {}", self.redraws);
        let counters = format!("SUBMITTED {} SKIPPED {}", self.submitted, self.skipped);
        let seam = "TEXTURED 3D + HUD OVERLAY";
        let copy = "WRAPPED TEXT STAYS INSIDE THE PANEL NO MATTER HOW NARROW THE WINDOW GETS";
        // Wrap to a deliberately narrow column so the screenshot shows more than
        // one line even on a wide window.
        let wrap_width = (panel_width * 0.55 - margin * 2.0).max(1.0);
        let wrapped = wrap_text(copy, wrap_width, line_scale);
        let copy_y = seam_y + line_step;

        let mut lines: Vec<(&str, f32, f32, f32, [f32; 4])> = vec![
            (title, title_x, title_y, title_scale, HUD_TEXT),
            (frame.as_str(), margin, frame_y, line_scale, HUD_DIM),
            (counters.as_str(), margin, counters_y, line_scale, HUD_DIM),
            (seam, margin, seam_y, line_scale, HUD_DIM),
        ];
        lines.extend(wrapped.iter().enumerate().map(|(index, line)| {
            (
                line.as_str(),
                margin,
                copy_y + index as f32 * line_step,
                line_scale,
                HUD_DIM,
            )
        }));

        let strip_height = TILE_STRIP_HEIGHT * dpi;
        let preview_size = CLIP_PREVIEW * dpi;
        let bottom_height = strip_height.max(preview_size);
        let copy_bottom =
            copy_y + (wrapped.len().saturating_sub(1)) as f32 * line_step + 8.0 * line_scale;
        let panel_height = (copy_bottom + bottom_height + margin)
            .min((surface_height as f32 - panel_pad * 2.0).max(1.0))
            .max(1.0);
        let panel = UiRect::new(
            Vec2::new(panel_pad, panel_pad),
            Vec2::new(panel_width, panel_height),
            PANEL_FILL,
        );
        let accent = UiRect::new(
            panel.position,
            Vec2::new(PANEL_ACCENT_WIDTH * dpi, panel.size.y),
            PANEL_ACCENT,
        );
        let rects = [panel, accent];

        let image_size = IMAGE_SIZE * dpi;
        let image_gap = IMAGE_GAP * dpi;
        let icon_y = panel.position.y + margin;
        let target_x = (panel.position.x + panel.size.x - image_size - margin).max(margin);
        let checker_x = (target_x - image_size - image_gap).max(margin);
        let bottom_y = panel.position.y + panel.size.y - bottom_height - margin * 0.5;
        let strip_y = bottom_y + (bottom_height - strip_height) * 0.5;
        let preview_x = panel.position.x + panel.size.x - preview_size - margin;
        let strip_width = (preview_x - image_gap - (panel.position.x + margin)).max(1.0);
        let overflow = CLIP_OVERFLOW * dpi;
        let preview_clip = UiClip::new(
            Vec2::new(preview_x, bottom_y),
            Vec2::new(preview_size, preview_size),
        );
        let images = [
            UiImage::new(
                texture,
                Vec2::new(checker_x, icon_y),
                Vec2::new(image_size, image_size),
            )
            .with_region(texture.region(0, 0, ATLAS_TILE, ATLAS_TILE))
            .with_rotation(ICON_ROTATION),
            UiImage::new(
                texture,
                Vec2::new(target_x, icon_y),
                Vec2::new(image_size, image_size),
            )
            .with_region(texture.region(ATLAS_TILE, 0, ATLAS_TILE, ATLAS_TILE)),
            // A region wider than the texture plus repeat addressing tiles the
            // checker half of the atlas across the strip.
            UiImage::new(
                texture,
                Vec2::new(panel.position.x + margin, strip_y),
                Vec2::new(strip_width, strip_height),
            )
            .with_region(TextureRegion::new(
                Vec2::new(0.0, 0.0),
                Vec2::new(0.5 * TILE_REPEATS, 1.0),
            ))
            .with_color([0.55, 0.7, 1.0, 0.75]),
            // Deliberately oversized and offset, then clipped to a slot so the
            // cropped edges are visible.
            UiImage::new(
                texture,
                Vec2::new(preview_x - overflow, bottom_y - overflow),
                Vec2::new(preview_size + overflow * 2.0, preview_size + overflow * 2.0),
            )
            .with_region(texture.region(ATLAS_TILE, 0, ATLAS_TILE, ATLAS_TILE))
            .with_clip(preview_clip),
        ];

        let shadow_offset = 2.0 * dpi;
        let mut text_draws: Vec<TextDraw<'_>> = Vec::with_capacity(lines.len() * 2);
        text_draws.extend(lines.iter().map(|(text, x, y, scale, _)| {
            TextDraw::new(Vec2::new(x + shadow_offset, y + shadow_offset), text)
                .with_scale(*scale)
                .with_color(HUD_SHADOW)
        }));
        text_draws.extend(lines.iter().map(|(text, x, y, scale, color)| {
            TextDraw::new(Vec2::new(*x, *y), text)
                .with_scale(*scale)
                .with_color(*color)
        }));

        renderer.render_scene(
            &draws,
            &Overlay {
                rects: &rects,
                images: &images,
                text: &text_draws,
            },
        )
    }
}

impl ApplicationHandler for SmokeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("fate window smoke")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        let renderer = pollster::block_on(Renderer::new(window.clone()))
            .expect("failed to initialize renderer");
        let mesh = renderer
            .create_mesh("smoke pyramid", PYRAMID_VERTICES, PYRAMID_INDICES)
            .expect("failed to create mesh");
        let textured_mesh = upload_textured_pyramid(&renderer);
        let texture = renderer
            .create_texture_with_options(
                "smoke atlas",
                ATLAS_WIDTH,
                ATLAS_HEIGHT,
                &atlas_rgba(ATLAS_WIDTH, ATLAS_HEIGHT, ATLAS_TILE),
                TextureOptions::NEAREST_REPEAT,
            )
            .expect("failed to create texture");

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.mesh = Some(mesh);
        self.textured_mesh = Some(textured_mesh);
        self.texture = Some(texture);
        if self.mode == Mode::Lifecycle {
            self.lifecycle = Some(Lifecycle::new());
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested if self.mode != Mode::Lifecycle => {
                let submitted = if self.mode == Mode::MeshLoad {
                    self.render_mesh_load()
                } else {
                    self.render_once()
                };
                if submitted {
                    self.submitted += 1;
                } else {
                    self.skipped += 1;
                }
                self.redraws += 1;
                if self.redraws >= FRAME_LIMIT {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
        if let (Some(test), Some(window), Some(renderer), Some(mesh)) = (
            self.lifecycle.as_mut(),
            self.window.as_ref(),
            self.renderer.as_mut(),
            self.mesh.as_ref(),
        ) {
            // A minimized window may stop receiving RedrawRequested.
            test.tick(window, renderer, mesh);
            if test.is_complete() {
                event_loop.exit();
            }
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
