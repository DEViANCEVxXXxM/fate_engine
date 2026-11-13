//! Windowed capability showcase: deterministic simulation, 3D rendering,
//! culling, diagnostics, replay/rollback verification, and frame timing.
//!
//! This app intentionally has no gameplay loop. It advances a scripted input
//! pattern at a fixed rate, renders the resulting cube grid, and continuously
//! verifies the engine features it is demonstrating.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::demo_input::{self, Action, Axis, DemoInput};
use crate::diagnostics_panel::{ShowcaseSink, ShowcaseSource};
use crate::prototype_simulation::SIMULATION_HZ;
use crate::scene;
use crate::showcase::{ShowcaseSim, VerificationState, scripted_input};
use fate_engine::diagnostics::{Diagnostics, FrameDiagnostics, FrameReport, FrameSample};
use fate_engine::math::Vec3;
use fate_engine::renderer::{Camera3d, Mesh3d, Renderer};
use fate_engine::time::Time;

const CAMERA_LOOK_SENSITIVITY: f32 = 0.0025;
const CAMERA_MOVE_SPEED: f32 = 4.0;
const CAMERA_BOOST_SPEED: f32 = 8.0;
const MAX_CAMERA_FRAME_DELTA: f32 = 0.05;

const ORBIT_RADIUS: f32 = 11.0;
const ORBIT_HEIGHT: f32 = 7.5;
const ORBIT_SPEED: f32 = 0.15;
const ORBIT_TARGET: Vec3 = Vec3::new(0.0, 1.2, 0.0);

const TITLE_INTERVAL: f32 = 0.5;
const STATUS_LOG_INTERVAL: f32 = 2.0;

/// One frame's render work, kept separate so simulation and verification time
/// are never folded into the render phases.
#[derive(Clone, Copy, Debug, Default)]
struct RenderFrame {
    rendered: bool,
    draw_list_ms: f32,
    submit_ms: f32,
    visible: usize,
    drawn: usize,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    mesh: Option<Mesh3d>,
    sim: ShowcaseSim,
    verification: VerificationState,
    source: ShowcaseSource,
    input: DemoInput,
    time: Time,
    diagnostics: Diagnostics<ShowcaseSink>,
    last_report: FrameReport,
    last_verify_ms: f32,
    orbit_mode: bool,
    orbit_angle: f32,
    cursor_captured: bool,
    focused: bool,
    paused: bool,
    title_timer: f32,
    status_timer: f32,
}

impl App {
    fn new() -> Self {
        let sim = ShowcaseSim::new();
        let verification = VerificationState::new(&sim);
        let diagnostics = Diagnostics::new(
            FrameDiagnostics::with_default_target(Renderer::max_instance_budget()),
            ShowcaseSink::new(true),
        );
        let last_report = diagnostics.frame().report();

        Self {
            window: None,
            renderer: None,
            mesh: None,
            sim,
            verification,
            source: ShowcaseSource::default(),
            input: demo_input::new(),
            time: Time::new(SIMULATION_HZ as f32),
            diagnostics,
            last_report,
            last_verify_ms: 0.0,
            orbit_mode: true,
            orbit_angle: 0.0,
            cursor_captured: false,
            focused: true,
            paused: false,
            title_timer: 0.0,
            status_timer: 0.0,
        }
    }

    /// Advance the clock, simulation, verification, camera, and presentation.
    ///
    /// Returns whether a frame was submitted to the renderer.
    fn step_frame(&mut self) -> bool {
        let frame_dt = self.time.tick();
        let (sim_ms, verify_ms) = self.advance_simulation();
        self.last_verify_ms = verify_ms;
        self.update_camera(frame_dt);

        if !self.focused {
            self.input.end_frame();
            self.update_presentation(frame_dt);
            return false;
        }

        let alpha = self.interpolation_alpha();
        let frame = self.render_frame(alpha);

        let sample = FrameSample::new(sim_ms, frame.submit_ms, frame.drawn)
            .with_draw_list_ms(frame.draw_list_ms);
        self.last_report = self.diagnostics.record_frame(sample);
        self.refresh_source(alpha, frame.drawn, frame.visible);
        self.sample_source();

        if self.last_report.total_ms > self.last_report.target_ms {
            log::warn!(
                target: "fate.showcase",
                "frame budget exceeded: {}",
                self.last_report.summary()
            );
        }

        self.input.end_frame();
        self.update_presentation(frame_dt);
        frame.rendered
    }

    /// Run every queued fixed step, timing simulation and verification apart.
    fn advance_simulation(&mut self) -> (f32, f32) {
        let mut sim_ms = 0.0;
        let mut verify_ms = 0.0;

        if self.paused {
            return (sim_ms, verify_ms);
        }

        while self.time.next_fixed_step().is_some() {
            let (sim, verify) = self.tick_once();
            sim_ms += sim;
            verify_ms += verify;
        }

        (sim_ms, verify_ms)
    }

    /// Advance one deterministic tick and its replay/rollback/snapshot checks.
    fn tick_once(&mut self) -> (f32, f32) {
        let input = scripted_input(self.sim.tick());

        let verify_start = Instant::now();
        self.verification.begin_tick(&self.sim, input);
        let mut verify_ms = verify_start.elapsed().as_secs_f32() * 1000.0;

        let sim_start = Instant::now();
        self.sim.advance(input);
        let sim_ms = sim_start.elapsed().as_secs_f32() * 1000.0;

        let verify_start = Instant::now();
        self.verification.end_tick(&self.sim);
        verify_ms += verify_start.elapsed().as_secs_f32() * 1000.0;

        (sim_ms, verify_ms)
    }

    /// Interpolation factor between the previous and current fixed states.
    ///
    /// A paused simulation renders the latest completed tick instead of cycling
    /// through the fractional remainder, which would jitter in place.
    fn interpolation_alpha(&self) -> f32 {
        if self.paused { 1.0 } else { self.time.alpha() }
    }

    fn render_frame(&mut self, alpha: f32) -> RenderFrame {
        let Some(renderer) = &self.renderer else {
            return RenderFrame::default();
        };
        let Some(mesh) = &self.mesh else {
            return RenderFrame::default();
        };

        let draw_start = Instant::now();
        let mut instances = Vec::with_capacity(scene::ARENA.len() + self.sim.tracked_count());
        instances.extend_from_slice(&scene::ARENA);
        instances.extend(self.sim.render_instances(alpha));
        let (draws, visible) =
            scene::build_draws(mesh, &renderer.camera_3d, renderer.aspect(), &instances);
        let drawn = renderer.budgeted_instance_count(draws.len());
        let draw_list_ms = draw_start.elapsed().as_secs_f32() * 1000.0;

        let submit_start = Instant::now();
        let rendered = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.render_meshes(&draws));
        let submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;

        RenderFrame {
            rendered,
            draw_list_ms,
            submit_ms,
            visible,
            drawn,
        }
    }

    fn update_camera(&mut self, frame_dt: f32) {
        let dt = if frame_dt.is_finite() {
            frame_dt.max(0.0)
        } else {
            0.0
        };

        if self.orbit_mode {
            self.orbit_angle = (self.orbit_angle + ORBIT_SPEED * dt) % std::f32::consts::TAU;
            if let Some(renderer) = &mut self.renderer {
                renderer.camera_3d = orbit_camera(self.orbit_angle);
            }
            return;
        }

        let mouse_delta = (
            self.input.axis_delta(Axis::MouseX) as f64,
            self.input.axis_delta(Axis::MouseY) as f64,
        );
        if let Some(renderer) = &mut self.renderer {
            update_fps_camera(
                &mut renderer.camera_3d,
                FpsCameraInput {
                    forward: self.input.action_down(Action::MoveUp),
                    backward: self.input.action_down(Action::MoveDown),
                    left: self.input.action_down(Action::MoveLeft),
                    right: self.input.action_down(Action::MoveRight),
                    boost: self.input.action_down(Action::Boost),
                    mouse_delta,
                    dt,
                },
            );
        }
    }

    fn refresh_source(&mut self, alpha: f32, drawn: usize, visible: usize) {
        let world = self.sim.world_diagnostics();
        let verification = &self.verification;

        self.source = ShowcaseSource {
            tick: self.sim.tick(),
            alpha,
            paused: self.paused,
            state_hash: self.sim.state_hash(),
            verify_ms: self.last_verify_ms,
            replay_enabled: verification.replay_enabled,
            replay: verification.replay_result.label(),
            rollback: verification.rollback_result.label(),
            snapshot: verification.snapshot_result.label(),
            live_entities: world.live_entities,
            capacity: world.capacity,
            free_slots: world.free_slots,
            positions: world.positions,
            visible,
            drawn,
        };
    }

    fn sample_source(&mut self) {
        let source = &self.source;
        self.diagnostics.sample_sources(&[source]);
    }

    fn update_presentation(&mut self, frame_dt: f32) {
        let dt = if frame_dt.is_finite() {
            frame_dt.max(0.0)
        } else {
            0.0
        };
        self.title_timer += dt;
        self.status_timer += dt;

        if self.title_timer >= TITLE_INTERVAL {
            self.title_timer = 0.0;
            self.update_title();
        }
        if self.status_timer >= STATUS_LOG_INTERVAL {
            self.status_timer = 0.0;
            self.log_status();
        }
    }

    fn replay_label(&self) -> &'static str {
        if self.verification.replay_enabled {
            self.verification.replay_result.label()
        } else {
            "off"
        }
    }

    fn title_text(&self) -> String {
        let report = self.last_report;
        let camera = if self.orbit_mode { "orbit" } else { "free" };
        let state = if self.paused { "paused" } else { "running" };

        format!(
            "fate showcase | {camera} | {state} | {:.0} fps | frame {:.1}ms | tick {} | \
             sim {:.2} draw {:.2} submit {:.2} total {:.2} avg {:.2} worst {:.2} ms | \
             target {:.1} headroom {:+.2} | draw {}/{} {:.1}% tier={} | verify {:.2}ms \
             replay {} rollback {} snapshot {} | hash {:016x}",
            self.time.fps(),
            self.time.frame_time_ms(),
            self.sim.tick(),
            report.sim_ms,
            report.draw_list_ms,
            report.render_ms,
            report.total_ms,
            report.avg_ms,
            report.worst_ms,
            report.target_ms,
            report.headroom_ms,
            report.draw_count,
            report.draw_limit,
            report.draw_pressure_pct,
            report.policy.label(),
            self.last_verify_ms,
            self.replay_label(),
            self.verification.rollback_result.label(),
            self.verification.snapshot_result.label(),
            self.sim.state_hash(),
        )
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            window.set_title(&self.title_text());
        }
    }

    fn log_status(&self) {
        let report = self.last_report;
        log::info!(
            target: "fate.showcase",
            "status tick={} camera={} paused={} fps={:.0} frame_ms={:.2} sim_ms={:.3} draw_list_ms={:.3} submit_ms={:.3} verify_ms={:.3} total_ms={:.3} drawn={}/{} tier={} replay={} rollback={} snapshot={} hash={:016x}",
            self.sim.tick(),
            if self.orbit_mode { "orbit" } else { "free" },
            self.paused,
            self.time.fps(),
            self.time.frame_time_ms(),
            report.sim_ms,
            report.draw_list_ms,
            report.render_ms,
            self.last_verify_ms,
            report.total_ms,
            report.draw_count,
            report.draw_limit,
            report.policy.label(),
            self.replay_label(),
            self.verification.rollback_result.label(),
            self.verification.snapshot_result.label(),
            self.sim.state_hash(),
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
                log::error!(target: "fate.showcase", "failed to capture cursor: {error}");
                self.cursor_captured = false;
                false
            }
        }
    }

    fn release_cursor(&mut self) {
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                log::warn!(target: "fate.showcase", "failed to release cursor: {error}");
            }
        }
        self.cursor_captured = false;
    }

    fn set_orbit_mode(&mut self, orbit: bool) {
        if self.orbit_mode == orbit {
            return;
        }
        self.orbit_mode = orbit;
        if orbit {
            self.release_cursor();
        } else if self.focused {
            self.capture_cursor();
        }
        log::info!(
            target: "fate.showcase",
            "camera mode: {}",
            if orbit { "orbit" } else { "free" }
        );
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if !paused {
            // Drop wall-clock time accumulated while paused so resuming does not
            // replay a long backlog of fixed steps.
            self.time = Time::new(SIMULATION_HZ as f32);
        }
        log::info!(
            target: "fate.showcase",
            "simulation {}",
            if paused { "paused" } else { "running" }
        );
    }

    fn toggle_verbose(&mut self) {
        let verbose = !self.diagnostics.sink().is_verbose();
        self.diagnostics.sink_mut().set_verbose(verbose);
        log::info!(
            target: "fate.showcase",
            "verbose diagnostics {}",
            if verbose { "on" } else { "off" }
        );
    }

    fn toggle_replay(&mut self) {
        let enabled = !self.verification.replay_enabled;
        self.verification.set_replay_enabled(enabled, &self.sim);
        log::info!(
            target: "fate.showcase",
            "replay verification {}",
            if enabled { "on" } else { "off" }
        );
    }

    fn force_snapshot_check(&mut self) {
        let start = Instant::now();
        self.verification.force_snapshot_check(&self.sim);
        let elapsed = start.elapsed().as_secs_f32() * 1000.0;
        log::info!(
            target: "fate.showcase",
            "snapshot round-trip {} in {:.3}ms",
            self.verification.snapshot_result.label(),
            elapsed
        );
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

        if state == ElementState::Pressed && !repeat {
            match code {
                KeyCode::Escape => {
                    if self.cursor_captured {
                        self.release_cursor();
                    } else {
                        event_loop.exit();
                    }
                    return;
                }
                KeyCode::Tab => {
                    self.set_orbit_mode(!self.orbit_mode);
                    return;
                }
                KeyCode::Space => {
                    self.set_paused(!self.paused);
                    return;
                }
                KeyCode::F1 => {
                    self.toggle_verbose();
                    return;
                }
                KeyCode::F2 => {
                    self.toggle_replay();
                    return;
                }
                KeyCode::F4 => {
                    self.force_snapshot_check();
                    return;
                }
                _ => {}
            }
        }

        match state {
            ElementState::Pressed => self.input.press(code),
            ElementState::Released => self.input.release(code),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("fate showcase")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));

        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        let mut renderer = pollster::block_on(Renderer::new(window.clone()))
            .expect("failed to initialize renderer");
        renderer.camera_3d = orbit_camera(self.orbit_angle);

        let mesh = match renderer.cube_mesh() {
            Ok(mesh) => Some(mesh),
            Err(error) => {
                log::error!(target: "fate.showcase", "failed to create cube mesh: {error}");
                None
            }
        };

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.mesh = mesh;
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
                if focused && !self.orbit_mode {
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
        if !self.orbit_mode
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

fn orbit_camera(angle: f32) -> Camera3d {
    let position = Vec3::new(
        ORBIT_TARGET.x + ORBIT_RADIUS * angle.cos(),
        ORBIT_HEIGHT,
        ORBIT_TARGET.z + ORBIT_RADIUS * angle.sin(),
    );
    let direction = (ORBIT_TARGET - position).normalize();

    Camera3d {
        position,
        yaw: direction.x.atan2(-direction.z),
        pitch: direction.y.asin(),
        ..Camera3d::default()
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
    use crate::showcase::REPLAY_WINDOW;
    use fate_engine::diagnostics::BudgetPolicy;

    #[test]
    fn new_app_starts_with_the_showcase_world_and_no_renderer() {
        let app = App::new();

        assert!(app.renderer.is_none());
        assert!(app.mesh.is_none());
        assert_eq!(app.sim.tracked_count(), 161);
        assert_eq!(app.sim.tracked_count(), 161);
        assert_eq!(app.sim.tick(), 0);
        assert!(app.orbit_mode);
        assert!(!app.paused);
        assert!(app.diagnostics.sink().is_verbose());
    }

    #[test]
    fn tick_once_advances_simulation_and_verification() {
        let mut app = App::new();

        let (sim_ms, verify_ms) = app.tick_once();

        assert_eq!(app.sim.tick(), 1);
        assert!(sim_ms >= 0.0);
        assert!(verify_ms >= 0.0);
    }

    #[test]
    fn headless_step_is_skipped_cleanly_without_a_renderer() {
        let mut app = App::new();

        let rendered = app.step_frame();

        assert!(!rendered);
        let report = app.diagnostics.frame().report();
        assert_eq!(report.draw_limit, Renderer::max_instance_budget());
        assert!(report.total_ms >= 0.0);
        assert!(app.last_report.total_ms >= 0.0);
    }

    #[test]
    fn paused_steps_do_not_advance_the_simulation() {
        let mut app = App::new();
        app.set_paused(true);

        let rendered = app.step_frame();

        assert!(!rendered);
        assert_eq!(app.sim.tick(), 0);
        assert!(app.paused);
    }

    #[test]
    fn paused_simulation_renders_the_latest_completed_tick() {
        let mut app = App::new();
        assert!((0.0..=1.0).contains(&app.interpolation_alpha()));

        app.set_paused(true);

        assert_eq!(app.interpolation_alpha(), 1.0);
    }

    #[test]
    fn step_frame_clears_input_edge_state_even_without_renderer() {
        let mut app = App::new();
        app.input.press(KeyCode::KeyW);

        let _ = app.step_frame();

        assert!(!app.input.button_just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn frame_sample_reports_budget_exceedance() {
        let sample = FrameSample::new(4.0, 5.0, 3);

        assert!(sample.total_ms() > Time::frame_budget_ms() as f32);
        assert!((sample.total_ms() - 9.0).abs() < 1e-5);
    }

    #[test]
    fn frame_report_contains_budget_summary() {
        let mut app = App::new();
        let report = app.diagnostics.record_frame(FrameSample::new(3.0, 4.0, 2));

        let summary = report.summary();
        assert!(summary.contains("drawn=2/256"));
        assert!(summary.contains("load=0.8%"));
        assert!(summary.contains("total=7.0ms"));
        assert!(summary.contains("target=8.0ms"));
        assert!(summary.contains("headroom=1.0ms"));
        assert!(summary.contains("tier=healthy"));
    }

    #[test]
    fn title_summarizes_simulation_render_and_verification_state() {
        let mut app = App::new();
        app.tick_once();
        app.refresh_source(0.5, 10, 12);

        let title = app.title_text();

        assert!(title.contains("fate showcase"));
        assert!(title.contains("orbit"));
        assert!(title.contains("tick 1"));
        assert!(title.contains("tier="));
        assert!(title.contains("replay pending"));
        assert!(title.contains("hash "));
    }

    #[test]
    fn diagnostics_source_tracks_simulation_state() {
        let mut app = App::new();
        app.tick_once();
        app.refresh_source(0.25, 12, 20);

        assert_eq!(app.source.tick, 1);
        assert_eq!(
            app.source.live_entities,
            app.sim.world_diagnostics().live_entities
        );
        assert_eq!(app.source.positions, app.sim.tracked_count());
        assert_eq!(app.source.drawn, 12);
        assert_eq!(app.source.visible, 20);
        assert_eq!(app.source.replay, "pending");
    }

    #[test]
    fn verbose_toggle_flips_the_sink() {
        let mut app = App::new();
        assert!(app.diagnostics.sink().is_verbose());

        app.toggle_verbose();

        assert!(!app.diagnostics.sink().is_verbose());
    }

    #[test]
    fn replay_toggle_restarts_the_verification_window() {
        let mut app = App::new();
        app.toggle_replay();
        assert!(!app.verification.replay_enabled);
        assert!(app.title_text().contains("replay off"));

        for _ in 0..10 {
            app.tick_once();
        }
        assert_eq!(app.verification.replay_result.label(), "pending");

        app.toggle_replay();
        assert!(app.verification.replay_enabled);
        for _ in 0..REPLAY_WINDOW {
            app.tick_once();
        }
        assert_eq!(app.verification.replay_result.label(), "ok");
    }

    #[test]
    fn forced_snapshot_check_reports_ok() {
        let mut app = App::new();
        for _ in 0..30 {
            app.tick_once();
        }

        app.force_snapshot_check();

        assert_eq!(app.verification.snapshot_result.label(), "ok");
    }

    #[test]
    fn orbit_camera_faces_the_arena_center() {
        let camera = orbit_camera(0.0);

        assert!((camera.position.y - ORBIT_HEIGHT).abs() < 1e-6);
        let expected = (ORBIT_TARGET - camera.position).normalize();
        assert!((camera.forward() - expected).length() < 1e-4);
    }

    #[test]
    fn orbit_camera_moves_around_the_target() {
        let start = orbit_camera(0.0);
        let quarter = orbit_camera(std::f32::consts::FRAC_PI_2);

        assert!((start.position - quarter.position).length() > 1.0);
        assert!((quarter.position.y - ORBIT_HEIGHT).abs() < 1e-6);
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
    fn frame_diagnostics_classifies_warning_and_critical_budget_slips() {
        let mut app = App::new();
        let report = app.diagnostics.record_frame(FrameSample::new(8.0, 2.0, 4));
        assert_eq!(report.policy, BudgetPolicy::Warning);

        let report = app
            .diagnostics
            .record_frame(FrameSample::new(10.0, 10.0, 8));
        assert_eq!(report.policy, BudgetPolicy::Critical);
    }
}
