//! Minimal windowed consumer for the renderer's public seam.
//!
//! This app exists to answer one question: can a project that is not the demo
//! create a window, own the surface, upload GPU resources, and submit a draw
//! list using only the documented `fate_engine` API? It intentionally does not
//! depend on `wgpu` directly.
//!
//! It exits after a fixed number of redraws so it can run unattended.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fate_engine::math::{Mat4, Vec3};
use fate_engine::renderer::{Draw3d, Mesh3d, MeshVertex, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const FRAME_LIMIT: u32 = 180;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SPIN_RATE: f32 = 0.05;

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

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = SmokeApp::default();
    event_loop.run_app(&mut app).expect("event loop failed");
    println!(
        "window smoke completed: {} redraws, {} submitted, {} skipped",
        app.redraws, app.submitted, app.skipped
    );
}

#[derive(Default)]
struct SmokeApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    mesh: Option<Mesh3d>,
    redraws: u32,
    submitted: u32,
    skipped: u32,
}

impl SmokeApp {
    fn render_once(&mut self) -> bool {
        let (Some(renderer), Some(mesh)) = (self.renderer.as_mut(), self.mesh.as_ref()) else {
            return false;
        };

        let draws: Vec<Draw3d<'_>> = (0..3)
            .map(|index| {
                let offset = index as f32 * 2.2 - 2.2;
                let angle = self.redraws as f32 * SPIN_RATE + index as f32 * 1.3;
                let model = Mat4::translation(Vec3::new(offset, 0.0, -7.0))
                    .mul(Mat4::rotation_y(angle))
                    .mul(Mat4::rotation_x(angle * 0.5));
                Draw3d {
                    mesh,
                    model,
                    tint: [1.0, 1.0, 1.0],
                }
            })
            .collect();

        renderer.render_meshes(&draws)
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

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.mesh = Some(mesh);
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
            WindowEvent::RedrawRequested => {
                if self.render_once() {
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
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
