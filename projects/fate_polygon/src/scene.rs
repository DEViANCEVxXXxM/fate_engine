use std::time::Instant;

use fate_engine::math::{Mat4, Vec2, Vec3};
use fate_engine::renderer::{
    Bounds3d, Camera3d, Draw3d, FlatLight3d, Frustum3d, GpuFrameTiming, Mesh3d, MeshCreateError,
    Overlay, RenderDiagnosticEvent, RenderDiagnosticReceiver, Renderer, SceneDraw3d, TextDraw,
    Texture2d, UiRect, UvMeshVertex,
};

use crate::{lab::Lab, settings::Settings};

const PREVIEW_PER_WORKLOAD: usize = 40;

pub struct Viewport {
    pub renderer: Renderer,
    telemetry: RenderDiagnosticReceiver,
    mesh: Mesh3d,
    marker: Mesh3d,
    white: Texture2d,
    detail: u32,
    orbit: f32,
}

pub struct RenderWork {
    pub prepare_ms: f32,
    pub candidates: usize,
    pub event: Option<fate_engine::renderer::RenderFrameDiagnostics>,
    pub gpu: Vec<GpuFrameTiming>,
}

impl Viewport {
    pub fn new(mut renderer: Renderer, detail: u32) -> Self {
        let telemetry = renderer.subscribe_diagnostics();
        let mesh = deck_mesh(&renderer, detail).expect("load polygon stress mesh");
        let marker = renderer.cube_mesh().expect("load polygon preview marker");
        let white = renderer
            .create_texture("polygon white albedo", 1, 1, &[255; 4])
            .expect("load polygon albedo");
        renderer.mesh_light = grid_light();
        Self {
            renderer,
            telemetry,
            mesh,
            marker,
            white,
            detail,
            orbit: 0.0,
        }
    }

    pub fn render(&mut self, s: Settings, lab: &Lab, paused: bool, delta: f32) -> RenderWork {
        let prepare = Instant::now();
        if s.detail != self.detail {
            self.mesh = deck_mesh(&self.renderer, s.detail).expect("rebuild polygon stress mesh");
            self.detail = s.detail;
        }
        if s.orbit {
            self.orbit = (self.orbit + delta * 0.36).rem_euclid(std::f32::consts::TAU);
        } else {
            self.orbit = 0.0;
        }
        set_grid_camera(&mut self.renderer.camera_3d, self.orbit);
        let (width, height) = self.renderer.surface_size();
        let frustum = s.cull.then(|| {
            self.renderer
                .camera_3d
                .frustum(width as f32 / height.max(1) as f32)
        });
        let mut candidates = 0;
        let mut draws = Vec::with_capacity(s.candidates as usize + PREVIEW_PER_WORKLOAD * 2);
        let mut add = |mesh, texture, center, scale: f32, tint| {
            candidates += 1;
            if frustum.is_none_or(|frustum| {
                frustum.intersects_bounds(Bounds3d::new(center, Vec3::ONE * scale))
            }) {
                draws.push(SceneDraw3d {
                    draw: Draw3d {
                        mesh,
                        tint,
                        model: Mat4::translation(center).mul(Mat4::scale(Vec3::ONE * scale)),
                    },
                    texture,
                });
            }
        };
        if s.preview && s.physics {
            for body in lab.bodies.iter().take(PREVIEW_PER_WORKLOAD) {
                add(
                    &self.marker,
                    None,
                    Vec3::new(body.p.x.to_f32(), body.p.y.to_f32(), 1.0),
                    s.radius as f32 * 0.013,
                    [1.0, 0.56, 0.27],
                );
            }
        }
        if s.preview && s.ai {
            for agent in lab.agents.iter().take(PREVIEW_PER_WORKLOAD) {
                add(
                    &self.marker,
                    None,
                    Vec3::new(agent.p.x.to_f32(), agent.p.y.to_f32(), 1.0),
                    0.29,
                    [0.30, 0.91, 0.75],
                );
            }
        }
        if s.render {
            candidates += s.candidates as usize;
            append_stress_grid(s, frustum, &self.mesh, &self.white, &mut draws);
        }
        let prepare_ms = prepare.elapsed().as_secs_f32() * 1e3;
        let status = format!(
            "{}  /  RENDER {}  PHYSICS {}  AI {}",
            if paused { "PAUSED" } else { "RUNNING" },
            on(s.render),
            on(s.physics),
            on(s.ai)
        );
        let top = UiRect::new(
            Vec2::ZERO,
            Vec2::new(width as f32, 54.0),
            [0.021, 0.038, 0.036, 0.94],
        );
        let text = [
            TextDraw::new(Vec2::new(17.0, 9.0), "FATE POLYGON")
                .with_scale(2.0)
                .with_color([0.77, 0.95, 0.83, 1.0]),
            TextDraw::new(Vec2::new(17.0, 33.0), &status)
                .with_scale(1.25)
                .with_color([0.64, 0.77, 0.73, 1.0]),
        ];
        self.renderer.render_scene(
            &draws,
            &Overlay {
                rects: &[top],
                text: &text,
                ..Overlay::default()
            },
        );
        let mut event = None;
        let mut gpu = Vec::new();
        for sample in self.telemetry.drain() {
            match sample {
                RenderDiagnosticEvent::Frame(frame) => event = Some(frame),
                RenderDiagnosticEvent::Gpu(timing) => gpu.push(timing),
            }
        }
        RenderWork {
            prepare_ms,
            candidates,
            event,
            gpu,
        }
    }
}

fn on(flag: bool) -> &'static str {
    if flag { "ON" } else { "OFF" }
}

fn deck_mesh(renderer: &Renderer, detail: u32) -> Result<Mesh3d, MeshCreateError> {
    let (vertices, indices) = deck_geometry(detail);
    renderer.create_mesh_with_uvs("polygon load tile", &vertices, &indices)
}

pub fn grid_light() -> Option<FlatLight3d> {
    FlatLight3d::new(Vec3::new(-0.35, 0.65, 1.0), 0.48)
}

pub fn set_grid_camera(camera: &mut Camera3d, orbit: f32) {
    let (sin, cos) = orbit.sin_cos();
    camera.position = Vec3::new(sin * 30.0, 0.0, cos * 30.0);
    camera.yaw = -orbit;
}

pub fn append_stress_grid<'a>(
    s: Settings,
    frustum: Option<Frustum3d>,
    mesh: &'a Mesh3d,
    white: &'a Texture2d,
    draws: &mut Vec<SceneDraw3d<'a>>,
) {
    let side = (s.candidates as f32).sqrt().ceil() as u32;
    for i in 0..s.candidates {
        let x = (i % side) as f32 - (side - 1) as f32 * 0.5;
        let y = (i / side) as f32 - (side - 1) as f32 * 0.5;
        let center = Vec3::new(x * 1.26, y * 1.26, -3.0);
        if frustum
            .is_some_and(|frustum| !frustum.intersects_bounds(Bounds3d::new(center, Vec3::ONE)))
        {
            continue;
        }
        let textured = !(i / s.batch).is_multiple_of(2);
        draws.push(SceneDraw3d {
            draw: Draw3d {
                mesh,
                model: Mat4::translation(center),
                tint: if textured {
                    [0.49, 0.76, 0.69]
                } else {
                    [0.27, 0.56, 0.62]
                },
            },
            texture: textured.then_some(white),
        });
    }
}

pub fn deck_geometry(detail: u32) -> (Vec<UvMeshVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for y in 0..=detail {
        for x in 0..=detail {
            let (u, v) = (x as f32 / detail as f32, y as f32 / detail as f32);
            let (px, py) = (u - 0.5, v - 0.5);
            let relief = (1.0 - 2.0 * px.abs().max(py.abs())) * 0.32;
            vertices.push(UvMeshVertex {
                position: [px, py, relief],
                color: [0.9, 0.95, 1.0],
                uv: [u, v],
            });
            if x < detail && y < detail {
                let a = y * (detail + 1) + x;
                let b = a + detail + 1;
                indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tessellation_changes_actual_triangle_count() {
        for detail in [3, 10, 48] {
            let (vertices, indices) = deck_geometry(detail);
            assert_eq!(indices.len(), (detail * detail * 6) as usize);
            assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
        }
    }
}
