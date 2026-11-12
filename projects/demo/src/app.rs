use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::demo_input::{self, Action, Axis, DemoInput};
use crate::demo_simulation::DemoSimulation;
use crate::prototype_simulation::{SIMULATION_HZ, SimulationInput};
use fate_engine::diagnostics::{
    DiagnosticSink, DiagnosticSource, DiagnosticValue, FrameDiagnostics, FrameSample,
};
use fate_engine::math::{Mat4, Vec2, Vec3};
use fate_engine::renderer::{Bounds2d, Camera3d, Draw3d, Instance2d, Mesh3d, Renderer};
use fate_engine::time::Time;
const CAMERA_LOOK_SENSITIVITY: f32 = 0.0025;
const CAMERA_MOVE_SPEED: f32 = 4.0;
const CAMERA_BOOST_SPEED: f32 = 8.0;
const MAX_CAMERA_FRAME_DELTA: f32 = 0.05;

impl App {
    fn telemetry(&self) -> fate_engine::diagnostics::FrameReport {
        self.frame_diagnostics.report()
    }

    fn performance_report(&self) -> String {
        self.telemetry().summary()
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    demo_mesh: Option<Mesh3d>,
    three_d_mode: bool,
    cursor_captured: bool,
    focused: bool,
    core: DemoSimulation,
    input: DemoInput,
    time: Time,
    frame_diagnostics: FrameDiagnostics,
    title_timer: f32,
}

/// Demonstrates the engine's diagnostic-source seam without making the engine
/// depend on demo types.
impl DiagnosticSource for App {
    fn record_diagnostics(&self, sink: &mut dyn DiagnosticSink) {
        let world = self.core.world.diagnostics();
        sink.record(
            "world",
            "live_entities",
            DiagnosticValue::Count(world.live_entities as u64),
        );
        sink.record(
            "world",
            "capacity",
            DiagnosticValue::Count(world.capacity as u64),
        );
        sink.record(
            "world",
            "free_slots",
            DiagnosticValue::Count(world.free_slots as u64),
        );

        let frame = self.frame_diagnostics.report();
        sink.record("frame", "total_ms", DiagnosticValue::Millis(frame.total_ms));
        sink.record(
            "frame",
            "drawn",
            DiagnosticValue::Count(frame.draw_count as u64),
        );
        sink.record("frame", "tier", DiagnosticValue::Text(frame.policy.label()));
    }
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            demo_mesh: None,
            three_d_mode: false,
            cursor_captured: false,
            focused: true,
            core: DemoSimulation::new(),
            input: demo_input::new(),
            time: Time::new(SIMULATION_HZ as f32),
            frame_diagnostics: FrameDiagnostics::with_default_target(
                Renderer::max_instance_budget(),
            ),
            title_timer: 0.0,
        }
    }

    fn step_frame(&mut self) -> bool {
        let frame_dt = self.time.tick();
        let input = self.simulation_input();
        let sim_start = Instant::now();
        while self.time.next_fixed_step().is_some() {
            self.core.fixed_update(input);
        }
        let sim_ms = sim_start.elapsed().as_secs_f32() * 1000.0;
        self.update_3d_camera(frame_dt);
        self.input.end_frame();
        self.update_camera();

        if !self.focused {
            self.update_title(frame_dt);
            return false;
        }

        let render_start = Instant::now();
        let (rendered, draw_budget) = if self.three_d_mode {
            if let (Some(renderer), Some(mesh)) = (&mut self.renderer, &self.demo_mesh) {
                let draws = Self::three_d_demo_draws(mesh, &renderer.camera_3d, renderer.aspect());
                let count = renderer.budgeted_instance_count(draws.len());
                (renderer.render_meshes(&draws), count)
            } else {
                (false, 0)
            }
        } else {
            let instances = self.instances();
            let count = self
                .renderer
                .as_ref()
                .map(|renderer| renderer.budgeted_instance_count(instances.len()))
                .unwrap_or(0);
            let rendered = if let Some(renderer) = &mut self.renderer {
                renderer.render(&instances)
            } else {
                false
            };
            (rendered, count)
        };
        let render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        let sample = FrameSample::new(sim_ms, render_ms, draw_budget);
        let report = self.frame_diagnostics.record(sample);

        if sample.total_ms() > Time::frame_budget_ms() as f32 {
            log::warn!("frame budget exceeded: {}", report.summary());
        }

        self.update_title(frame_dt);
        rendered
    }

    #[allow(dead_code)]
    fn headless_step(&mut self) -> bool {
        self.step_frame()
    }

    fn simulation_input(&self) -> SimulationInput {
        if self.three_d_mode {
            return SimulationInput::default();
        }
        let axis = |positive, negative| {
            self.input.action_down(positive) as i8 - self.input.action_down(negative) as i8
        };
        SimulationInput {
            move_x: axis(Action::MoveRight, Action::MoveLeft),
            move_y: axis(Action::MoveUp, Action::MoveDown),
            boost: self.input.action_down(Action::Boost),
        }
    }

    fn player_camera_target(&self) -> Option<Vec2> {
        self.core
            .world
            .positions()
            .get(self.core.player)
            .map(|position| Vec2::new(position.0.x.to_f32(), position.0.y.to_f32()))
    }

    fn update_camera(&mut self) {
        let Some(target) = self.player_camera_target() else {
            return;
        };
        if let Some(renderer) = &mut self.renderer {
            renderer.camera.center = target;
        }
    }

    fn three_d_demo_draws<'a>(mesh: &'a Mesh3d, camera: &Camera3d, aspect: f32) -> Vec<Draw3d<'a>> {
        let frustum = camera.frustum(aspect);
        let mut draws = Vec::with_capacity(19);

        let mut add_box = |position: Vec3, scale: Vec3, rotation: f32, tint: [f32; 3]| {
            // The demo mesh is a unit cube, so this sphere encloses the box
            // regardless of rotation.
            let radius = scale.length() * 0.5;
            if !frustum.intersects_sphere(position, radius) {
                return;
            }

            let model = Mat4::translation(position)
                .mul(Mat4::rotation_y(rotation))
                .mul(Mat4::rotation_x(rotation * 0.5))
                .mul(Mat4::scale(scale));
            draws.push(Draw3d { mesh, model, tint });
        };

        add_box(
            Vec3::new(0.0, -0.15, -1.0),
            Vec3::new(12.0, 0.3, 16.0),
            0.0,
            [0.22, 0.28, 0.38],
        );
        add_box(
            Vec3::new(0.0, 1.5, -8.0),
            Vec3::new(12.0, 3.0, 0.3),
            0.0,
            [0.30, 0.40, 0.55],
        );
        add_box(
            Vec3::new(-6.0, 1.5, -1.0),
            Vec3::new(0.3, 3.0, 14.0),
            0.0,
            [0.30, 0.40, 0.55],
        );
        add_box(
            Vec3::new(6.0, 1.5, -1.0),
            Vec3::new(0.3, 3.0, 14.0),
            0.0,
            [0.30, 0.40, 0.55],
        );

        for x in [-4.0, 4.0] {
            for z in [-6.0, -2.0, 2.0] {
                add_box(
                    Vec3::new(x, 1.0, z),
                    Vec3::new(0.8, 2.0, 0.8),
                    0.0,
                    [0.20, 0.62, 0.58],
                );
            }
        }

        let cube_colors = [
            [0.95, 0.28, 0.22],
            [0.98, 0.58, 0.18],
            [0.95, 0.82, 0.22],
            [0.34, 0.78, 0.34],
            [0.18, 0.72, 0.78],
            [0.22, 0.45, 0.95],
            [0.55, 0.32, 0.90],
            [0.90, 0.30, 0.62],
            [0.88, 0.88, 0.92],
        ];
        for row in 0..3 {
            for column in 0..3 {
                let x = (column as f32 - 1.0) * 1.6;
                let z = -(row as f32) * 1.6;
                add_box(
                    Vec3::new(x, 0.8, z),
                    Vec3::new(0.9, 1.2, 0.9),
                    (row + column) as f32 * 0.18,
                    cube_colors[row * 3 + column],
                );
            }
        }
        draws
    }

    fn update_3d_camera(&mut self, dt: f32) {
        let mouse_dx = self.input.axis_delta(Axis::MouseX) as f64;
        let mouse_dy = self.input.axis_delta(Axis::MouseY) as f64;
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if !self.three_d_mode {
            return;
        }

        update_fps_camera(
            &mut renderer.camera_3d,
            FpsCameraInput {
                forward: self.input.button_down(KeyCode::KeyW)
                    || self.input.button_down(KeyCode::ArrowUp),
                backward: self.input.button_down(KeyCode::KeyS)
                    || self.input.button_down(KeyCode::ArrowDown),
                left: self.input.button_down(KeyCode::KeyA)
                    || self.input.button_down(KeyCode::ArrowLeft),
                right: self.input.button_down(KeyCode::KeyD)
                    || self.input.button_down(KeyCode::ArrowRight),
                boost: self.input.action_down(Action::Boost),
                mouse_delta: (mouse_dx, mouse_dy),
                dt,
            },
        );
    }

    fn capture_cursor(&mut self) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let result = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
        match result {
            Ok(()) => {
                window.set_cursor_visible(false);
                self.cursor_captured = true;
                true
            }
            Err(error) => {
                log::error!("failed to capture cursor for 3d camera: {error}");
                self.cursor_captured = false;
                false
            }
        }
    }

    fn release_cursor(&mut self) {
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                log::warn!("failed to release cursor: {error}");
            }
        }
        self.cursor_captured = false;
    }

    fn set_3d_mode(&mut self, enabled: bool) {
        if enabled {
            if self.demo_mesh.is_none() {
                log::warn!("3d demo mesh is unavailable");
                return;
            }
            self.three_d_mode = true;
            if self.focused {
                self.capture_cursor();
            }
        } else {
            self.three_d_mode = false;
            self.release_cursor();
        }
    }

    fn handle_key(
        &mut self,
        event_loop: &ActiveEventLoop,
        key: PhysicalKey,
        state: ElementState,
        repeat: bool,
    ) {
        let PhysicalKey::Code(code) = key else {
            return;
        };

        if code == KeyCode::Escape && state == ElementState::Pressed {
            if self.three_d_mode {
                self.set_3d_mode(false);
            } else {
                event_loop.exit();
            }
            return;
        }

        if code == KeyCode::F2 && state == ElementState::Pressed && !repeat {
            let enable_stress = !self.core.demo_stress_enabled();
            self.core.set_demo_stress(enable_stress);
            return;
        }

        if code == KeyCode::F3 && state == ElementState::Pressed && !repeat {
            self.set_3d_mode(!self.three_d_mode);
            return;
        }

        match state {
            ElementState::Pressed => self.input.press(code),
            ElementState::Released => self.input.release(code),
        }
    }

    fn instances(&self) -> Vec<Instance2d> {
        let bounds = match &self.renderer {
            Some(renderer) => renderer.camera.visible_bounds(renderer.gpu.aspect()),
            None => Bounds2d::new(Vec2::ZERO, Vec2::new(10.0, 10.0)),
        };

        let mut instances = Vec::new();
        for (entity, position) in self.core.world.positions().iter() {
            let render_position = Vec2::new(position.0.x.to_f32(), position.0.y.to_f32());
            if !bounds.contains_point(render_position) {
                continue;
            }

            let rotation = self
                .core
                .world
                .rotations()
                .get(entity)
                .map_or(0.0, |r| r.0.to_f32());
            instances.push(Instance2d {
                position: render_position,
                rotation,
                scale: if entity == self.core.player { 1.7 } else { 1.0 },
            });
        }
        instances
    }

    fn update_title(&mut self, dt: f32) {
        self.title_timer += dt;
        if self.title_timer < 0.5 {
            return;
        }
        self.title_timer = 0.0;

        if let Some(window) = &self.window {
            let headroom = self.time.budget_headroom_ms();
            let mode_text = if self.three_d_mode {
                "3d mesh demo"
            } else {
                "2d simulation demo"
            };
            let budget_text = if headroom >= 0.0 {
                format!("+{headroom:.1}ms")
            } else {
                format!("{headroom:.1}ms")
            };

            window.set_title(&format!(
                "fate | {mode_text} | WASD/arrows move, Shift boost, F2 stress {}, F3 3d | {:.0} fps - {} entities - {} / budget {}",
                if self.core.demo_stress_enabled() { "on" } else { "off" },
                self.time.fps(),
                self.core.world.entity_count(),
                self.performance_report(),
                budget_text,
            ));
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("fate")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));

        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        let renderer = pollster::block_on(Renderer::new(window.clone()))
            .expect("failed to initialize renderer");
        let demo_mesh = match renderer.cube_mesh() {
            Ok(mesh) => Some(mesh),
            Err(error) => {
                log::error!("failed to create 3d demo mesh: {error}");
                None
            }
        };

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.demo_mesh = demo_mesh;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if focused && self.three_d_mode {
                    self.capture_cursor();
                } else if !focused {
                    self.release_cursor();
                    self.input.clear();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key(event_loop, event.physical_key, event.state, event.repeat);
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                let _ = self.step_frame();
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if self.three_d_mode
            && self.cursor_captured
            && let DeviceEvent::MouseMotion { delta } = event
        {
            self.input.update_axis(Axis::MouseX, delta.0 as f32);
            self.input.update_axis(Axis::MouseY, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let frame_budget = Duration::from_millis(if self.focused {
            Time::frame_budget_ms()
        } else {
            50
        });
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame_budget));

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[derive(Clone, Copy)]
struct FpsCameraInput {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    boost: bool,
    mouse_delta: (f64, f64),
    dt: f32,
}

fn update_fps_camera(camera: &mut Camera3d, input: FpsCameraInput) {
    let dt = if input.dt.is_finite() {
        input.dt.clamp(0.0, MAX_CAMERA_FRAME_DELTA)
    } else {
        0.0
    };
    if input.mouse_delta.0.is_finite() {
        camera.yaw +=
            (input.mouse_delta.0 as f32).clamp(-1_000.0, 1_000.0) * CAMERA_LOOK_SENSITIVITY;
    }
    if input.mouse_delta.1.is_finite() {
        camera.pitch -=
            (input.mouse_delta.1 as f32).clamp(-1_000.0, 1_000.0) * CAMERA_LOOK_SENSITIVITY;
    }

    let forward = camera.forward();
    let horizontal_forward = Vec3::new(forward.x, 0.0, forward.z).normalize();
    let right = Vec3::new(-horizontal_forward.z, 0.0, horizontal_forward.x);
    let mut movement = Vec3::ZERO;
    if input.forward {
        movement += horizontal_forward;
    }
    if input.backward {
        movement += -horizontal_forward;
    }
    if input.right {
        movement += right;
    }
    if input.left {
        movement += -right;
    }
    if movement.length_squared() > 0.0 {
        let speed = if input.boost {
            CAMERA_BOOST_SPEED
        } else {
            CAMERA_MOVE_SPEED
        };
        camera.position += movement.normalize() * (speed * dt);
    }
}

pub async fn run() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::WaitUntil(
        Instant::now() + Duration::from_millis(Time::frame_budget_ms()),
    ));

    let mut app = App::new();
    event_loop.run_app(&mut app).expect("event loop error");
}

#[cfg(test)]
mod tests {
    use super::*;
    use fate_engine::diagnostics::BudgetPolicy;
    use fate_engine::renderer::Camera3d;
    use fate_sim::ecs::Position;
    use fate_sim::math::{Fixed, FixedVec2};

    #[test]
    fn step_frame_without_renderer_is_skipped_cleanly() {
        let mut app = App::new();

        let rendered = app.step_frame();

        assert!(!rendered);
        assert_eq!(app.core.world.entity_count(), 40);
        let report = app.frame_diagnostics.report();
        assert!(report.sim_ms >= 0.0);
        assert!(report.render_ms >= 0.0);
        assert!(report.total_ms >= 0.0);
    }

    #[test]
    fn frame_sample_reports_budget_exceedance() {
        let sample = FrameSample::new(4.0, 5.0, 3);

        assert!(sample.total_ms() > Time::frame_budget_ms() as f32);
        assert!((sample.total_ms() - 9.0).abs() < 1e-5);
    }

    #[test]
    fn step_frame_clears_input_edge_state_even_without_renderer() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyA);
        app.input.end_frame();

        app.input.press(winit::keyboard::KeyCode::KeyA);
        let _ = app.step_frame();

        assert!(
            !app.input
                .button_just_pressed(winit::keyboard::KeyCode::KeyA)
        );
    }

    #[test]
    fn platform_key_bindings_become_platform_independent_simulation_input() {
        let mut app = App::new();
        app.input.press(KeyCode::ArrowRight);
        app.input.press(KeyCode::KeyW);
        app.input.press(KeyCode::ShiftLeft);

        assert_eq!(
            app.simulation_input(),
            SimulationInput {
                move_x: 1,
                move_y: 1,
                boost: true,
            }
        );
    }

    #[test]
    fn frame_diagnostics_roll_and_average_recent_frames() {
        let mut diagnostics =
            FrameDiagnostics::with_default_target(Renderer::max_instance_budget());
        diagnostics.record(FrameSample::new(4.0, 0.0, 0));
        diagnostics.record(FrameSample::new(6.0, 0.0, 0));
        let report = diagnostics.record(FrameSample::new(8.0, 0.0, 0));

        assert!((report.avg_ms - 6.0).abs() < 1e-5);
        assert!((report.worst_ms - 8.0).abs() < 1e-5);
        assert_eq!(diagnostics.stats().recent_count(), 3);
    }

    #[test]
    fn frame_diagnostics_track_budget_policy_breakdown() {
        let mut diagnostics = FrameDiagnostics::new(8.0, 256);

        diagnostics.record(FrameSample::new(7.0, 0.0, 64));
        diagnostics.record(FrameSample::new(9.0, 0.0, 204));
        let report = diagnostics.record(FrameSample::new(12.0, 0.0, 256));

        assert_eq!(
            (
                report.healthy_frames,
                report.warning_frames,
                report.critical_frames
            ),
            (1, 1, 1)
        );
        assert!((diagnostics.stats().peak_draw_pressure_pct() - 100.0).abs() < 1e-5);
    }

    #[test]
    fn frame_report_contains_budget_summary() {
        let mut app = App::new();
        let report = app.frame_diagnostics.record(FrameSample::new(3.0, 4.0, 2));

        let summary = report.summary();
        assert!(summary.contains("drawn=2/256"));
        assert!(summary.contains("load=0.8%"));
        assert!(summary.contains("total=7.0ms"));
        assert!(summary.contains("target=8.0ms"));
        assert!(summary.contains("headroom=1.0ms"));
        assert!(summary.contains("tier=healthy"));
    }

    #[test]
    fn frame_diagnostics_classifies_warning_and_critical_budget_slips() {
        let mut app = App::new();
        let report = app.frame_diagnostics.record(FrameSample::new(8.0, 2.0, 4));
        assert_eq!(report.policy, BudgetPolicy::Warning);

        let report = app
            .frame_diagnostics
            .record(FrameSample::new(10.0, 10.0, 8));
        assert_eq!(report.policy, BudgetPolicy::Critical);
    }

    #[test]
    fn draw_pressure_tracks_budget_usage() {
        let mut app = App::new();
        app.frame_diagnostics
            .record(FrameSample::new(0.0, 0.0, 192));
        assert!((app.frame_diagnostics.draw_pressure() - 0.75).abs() < 1e-5);

        app.frame_diagnostics
            .record(FrameSample::new(0.0, 0.0, 256));
        assert!((app.frame_diagnostics.draw_pressure() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn telemetry_snapshot_exposes_machine_readable_budget_data() {
        let mut app = App::new();
        app.frame_diagnostics
            .record(FrameSample::new(3.0, 4.0, 128));

        let telemetry = app.telemetry();
        assert_eq!(telemetry.draw_count, 128);
        assert_eq!(telemetry.draw_limit, Renderer::max_instance_budget());
        assert!(telemetry.draw_pressure_pct > 0.0);
        assert_eq!(telemetry.policy, BudgetPolicy::Healthy);
        assert!(telemetry.summary().contains("tier=healthy"));
    }

    #[test]
    fn headless_simulation_runs_without_window_or_renderer() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyA);

        let rendered = app.headless_step();

        assert!(!rendered);
        assert!(app.core.world.entity_count() >= 1);
        assert!(app.frame_diagnostics.report().total_ms >= 0.0);
        assert!(
            !app.input
                .button_just_pressed(winit::keyboard::KeyCode::KeyA)
        );
    }

    #[test]
    fn app_can_step_through_headless_simulation_isolation() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyD);

        let rendered = app.headless_step();

        assert!(!rendered);
        assert_eq!(app.core.world.entity_count(), 40);
        assert!(app.frame_diagnostics.report().total_ms >= 0.0);
    }

    #[test]
    fn instances_only_include_entities_in_camera_view() {
        let mut app = App::new();
        let visible = app.core.world.spawn();
        let hidden = app.core.world.spawn();
        app.core
            .world
            .set_position(visible, Position(FixedVec2::new(Fixed::ONE, Fixed::ONE)))
            .unwrap();
        app.core
            .world
            .set_position(
                hidden,
                Position(FixedVec2::new(Fixed::from_int(30), Fixed::from_int(30))),
            )
            .unwrap();

        let instances = app.instances();

        assert_eq!(instances.len(), 33);
        assert!(
            instances
                .iter()
                .all(|instance| instance.position != Vec2::new(30.0, 30.0))
        );
    }

    #[test]
    fn demo_stress_mode_shows_culling_and_budget_warning() {
        let mut app = App::new();
        assert_eq!(app.instances().len(), 32);

        app.core.set_demo_stress(true);

        assert_eq!(app.instances().len(), 192);
        assert_eq!(app.core.world.entity_count(), 200);
        let drawn = app.instances().len();
        app.frame_diagnostics
            .record(FrameSample::new(0.0, 0.0, drawn));
        assert!((app.frame_diagnostics.draw_pressure() - 0.75).abs() < 1e-5);
        assert_eq!(app.frame_diagnostics.policy(), BudgetPolicy::Warning);

        app.core.set_demo_stress(false);
        assert_eq!(app.instances().len(), 32);
        assert_eq!(app.core.world.entity_count(), 40);
    }

    #[test]
    fn player_camera_target_tracks_player_position() {
        let mut app = App::new();
        let player = app.core.player;
        let target = FixedVec2::new(Fixed::from_int(4), Fixed::from_int(-3));
        app.core
            .world
            .set_position(player, Position(target))
            .unwrap();

        assert_eq!(
            app.player_camera_target(),
            Some(Vec2::new(target.x.to_f32(), target.y.to_f32()))
        );
    }

    #[test]
    fn player_is_visually_distinguished_from_demo_swarm() {
        let app = App::new();
        let instances = app.instances();
        let fixed_player_position = app.core.world.positions().get(app.core.player).unwrap().0;
        let player_position = Vec2::new(
            fixed_player_position.x.to_f32(),
            fixed_player_position.y.to_f32(),
        );
        let player_instance = instances
            .iter()
            .find(|instance| instance.position == player_position)
            .unwrap();

        assert_eq!(player_instance.scale, 1.7);
        assert!(
            instances
                .iter()
                .filter(|instance| instance.position != player_position)
                .all(|instance| instance.scale == 1.0)
        );
    }

    #[test]
    fn fps_camera_moves_in_facing_direction_and_sprint_changes_speed() {
        let mut camera = Camera3d::default();
        update_fps_camera(
            &mut camera,
            FpsCameraInput {
                forward: true,
                backward: false,
                left: false,
                right: false,
                boost: false,
                mouse_delta: (0.0, 0.0),
                dt: 0.5,
            },
        );
        assert!((camera.position.z - 4.8).abs() < 1e-5);

        let start = camera.position;
        update_fps_camera(
            &mut camera,
            FpsCameraInput {
                forward: true,
                backward: false,
                left: false,
                right: false,
                boost: true,
                mouse_delta: (0.0, 0.0),
                dt: 0.1,
            },
        );
        assert!((start.z - camera.position.z - 0.4).abs() < 1e-5);
    }

    #[test]
    fn fps_camera_strafe_is_relative_to_yaw_without_changing_height() {
        let mut camera = Camera3d {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let initial_height = camera.position.y;
        update_fps_camera(
            &mut camera,
            FpsCameraInput {
                forward: false,
                backward: false,
                left: false,
                right: true,
                boost: false,
                mouse_delta: (0.0, 0.0),
                dt: 0.25,
            },
        );

        assert!(camera.position.x < 0.0);
        assert!((camera.position.y - initial_height).abs() < 1e-5);
        assert!(camera.position.z.is_finite());
    }

    #[test]
    fn fps_camera_look_and_diagonal_movement_are_applied_safely() {
        let mut camera = Camera3d::default();
        let initial_position = camera.position;
        update_fps_camera(
            &mut camera,
            FpsCameraInput {
                forward: true,
                backward: false,
                left: false,
                right: true,
                boost: false,
                mouse_delta: (100.0, -50.0),
                dt: 0.1,
            },
        );
        assert!((camera.yaw - 0.25).abs() < 1e-5);
        assert!((camera.pitch - 0.125).abs() < 1e-5);
        assert!((camera.position - initial_position).length() - 0.4 < 1e-5);

        update_fps_camera(
            &mut camera,
            FpsCameraInput {
                forward: false,
                backward: false,
                left: false,
                right: false,
                boost: false,
                mouse_delta: (f64::MAX, f64::NAN),
                dt: f32::INFINITY,
            },
        );
        assert!(camera.yaw.is_finite());
        assert!(camera.pitch.is_finite());
        assert!(camera.position.x.is_finite());
        assert!(camera.position.z.is_finite());
    }

    #[test]
    fn first_person_controls_do_not_feed_the_2d_simulation() {
        let mut app = App::new();
        app.three_d_mode = true;
        app.input.press(KeyCode::KeyW);

        assert_eq!(app.simulation_input(), SimulationInput::default());
    }

    #[derive(Default)]
    struct TestSink {
        entries: Vec<(&'static str, &'static str, DiagnosticValue)>,
    }

    impl DiagnosticSink for TestSink {
        fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
            self.entries.push((group, name, value));
        }
    }

    #[test]
    fn app_exposes_diagnostics_through_the_source_seam() {
        let mut app = App::new();
        app.frame_diagnostics.record(FrameSample::new(3.0, 4.0, 32));

        let mut sink = TestSink::default();
        app.record_diagnostics(&mut sink);

        assert!(sink.entries.contains(&(
            "world",
            "live_entities",
            DiagnosticValue::Count(app.core.world.entity_count() as u64)
        )));
        assert!(
            sink.entries
                .contains(&("frame", "total_ms", DiagnosticValue::Millis(7.0)))
        );
        assert!(
            sink.entries
                .contains(&("frame", "tier", DiagnosticValue::Text("healthy")))
        );
    }
}
