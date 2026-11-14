//! Windowed playable slice: fixed-step combat, first-person cubes, replay and
//! snapshot verification, and a log/title presentation.
//!
//! The engine has no text renderer or HUD, so player state and verification
//! results are published through the window title and the `log` facade.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::combat_input::{self, Action, Axis, CombatInput};
use crate::interpolation::Interpolation;
use crate::scene;
use crate::sim::{MAGAZINE_SIZE, MAX_LOOK_INPUT, Outcome, SIMULATION_HZ, Sim, SimInput};
use crate::verification::Verification;
use fate_engine::renderer::{Mesh3d, Renderer};
use fate_engine::time::Time;

const TITLE_INTERVAL: f32 = 0.5;
const STATUS_LOG_INTERVAL: f32 = 2.0;
const MAX_PENDING_LOOK: f32 = MAX_LOOK_INPUT as f32 * 4.0;

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
    sim: Sim,
    verification: Verification,
    interpolation: Interpolation,
    input: CombatInput,
    time: Time,
    mouse_fire: bool,
    pending_look: (f32, f32),
    cursor_captured: bool,
    focused: bool,
    paused: bool,
    verbose: bool,
    last_outcome: Outcome,
    last_sim_ms: f32,
    last_verify_ms: f32,
    last_render_ms: f32,
    last_draw_list_ms: f32,
    last_visible: usize,
    last_drawn: usize,
    title_timer: f32,
    status_timer: f32,
}

impl App {
    fn new() -> Self {
        let sim = Sim::new();
        let verification = Verification::new(&sim);
        let interpolation = Interpolation::new(&sim);

        Self {
            window: None,
            renderer: None,
            mesh: None,
            sim,
            verification,
            interpolation,
            input: combat_input::new(),
            time: Time::new(SIMULATION_HZ as f32),
            mouse_fire: false,
            pending_look: (0.0, 0.0),
            cursor_captured: false,
            focused: true,
            paused: false,
            verbose: true,
            last_outcome: Outcome::Running,
            last_sim_ms: 0.0,
            last_verify_ms: 0.0,
            last_render_ms: 0.0,
            last_draw_list_ms: 0.0,
            last_visible: 0,
            last_drawn: 0,
            title_timer: 0.0,
            status_timer: 0.0,
        }
    }

    /// Advance the clock, simulation, camera, and presentation.
    ///
    /// Returns whether a frame was submitted to the renderer.
    fn step_frame(&mut self) -> bool {
        let frame_dt = self.time.tick();
        self.accumulate_look();
        let (sim_ms, verify_ms) = self.advance_simulation();
        self.last_sim_ms = sim_ms;
        self.last_verify_ms = verify_ms;
        self.update_camera();

        if !self.focused {
            self.input.end_frame();
            self.pending_look = (0.0, 0.0);
            self.update_presentation(frame_dt);
            return false;
        }

        let frame = self.render_frame(self.interpolation_alpha());
        self.last_draw_list_ms = frame.draw_list_ms;
        self.last_render_ms = frame.submit_ms;
        self.last_visible = frame.visible;
        self.last_drawn = frame.drawn;

        self.input.end_frame();
        self.update_presentation(frame_dt);
        frame.rendered
    }

    /// Run every queued fixed step, timing simulation and verification apart.
    fn advance_simulation(&mut self) -> (f32, f32) {
        if self.paused || !self.sim.outcome().is_running() {
            return (0.0, 0.0);
        }

        let mut sim_ms = 0.0;
        let mut verify_ms = 0.0;

        while self.time.next_fixed_step().is_some() {
            let look = self.take_look();
            let input = self.build_input(look);

            let verify_start = Instant::now();
            self.verification.begin_tick(&self.sim, input);
            verify_ms += verify_start.elapsed().as_secs_f32() * 1000.0;

            self.interpolation.begin_step();
            let sim_start = Instant::now();
            self.sim.step(input);
            sim_ms += sim_start.elapsed().as_secs_f32() * 1000.0;
            self.interpolation.end_step(&self.sim);

            let verify_start = Instant::now();
            self.verification.end_tick(&self.sim);
            verify_ms += verify_start.elapsed().as_secs_f32() * 1000.0;

            if !self.sim.outcome().is_running() {
                break;
            }
        }

        (sim_ms, verify_ms)
    }

    fn build_input(&self, look: (i16, i16)) -> SimInput {
        let move_x = (self.input.action_down(Action::MoveRight) as i8)
            - (self.input.action_down(Action::MoveLeft) as i8);
        let move_y = (self.input.action_down(Action::MoveForward) as i8)
            - (self.input.action_down(Action::MoveBack) as i8);

        SimInput {
            move_x,
            move_y,
            look_yaw: look.0,
            look_pitch: look.1,
            sprint: self.input.action_down(Action::Sprint),
            fire: self.input.action_down(Action::Fire) || self.mouse_fire,
            reload: self.input.action_down(Action::Reload),
        }
    }

    fn accumulate_look(&mut self) {
        let delta_x = self.input.axis_delta(Axis::MouseX);
        let delta_y = self.input.axis_delta(Axis::MouseY);
        if delta_x.is_finite() {
            self.pending_look.0 =
                (self.pending_look.0 + delta_x).clamp(-MAX_PENDING_LOOK, MAX_PENDING_LOOK);
        }
        if delta_y.is_finite() {
            // Screen-space Y grows downward, while positive pitch looks up.
            self.pending_look.1 =
                (self.pending_look.1 - delta_y).clamp(-MAX_PENDING_LOOK, MAX_PENDING_LOOK);
        }
    }

    fn take_look(&mut self) -> (i16, i16) {
        let yaw = clamp_look(self.pending_look.0);
        let pitch = clamp_look(self.pending_look.1);
        self.pending_look.0 -= yaw as f32;
        self.pending_look.1 -= pitch as f32;
        (yaw, pitch)
    }

    fn interpolation_alpha(&self) -> f32 {
        if self.paused || !self.sim.outcome().is_running() {
            1.0
        } else {
            self.time.alpha()
        }
    }

    fn update_camera(&mut self) {
        let eye = self
            .interpolation
            .player_position(self.interpolation_alpha());
        if let Some(renderer) = &mut self.renderer {
            renderer.camera_3d = scene::camera_at(eye, &self.sim);
        }
    }

    fn render_frame(&mut self, alpha: f32) -> RenderFrame {
        let Some(renderer) = &self.renderer else {
            return RenderFrame::default();
        };
        let Some(mesh) = &self.mesh else {
            return RenderFrame::default();
        };

        let draw_start = Instant::now();
        let camera = renderer.camera_3d;
        let dynamic = scene::dynamic_instances(&self.sim, &self.interpolation, alpha);
        let mut instances = Vec::with_capacity(scene::ARENA.len() + dynamic.len());
        instances.extend_from_slice(&scene::ARENA);
        instances.extend(dynamic);
        let (world_draws, world_visible) =
            scene::build_draws(mesh, &camera, renderer.aspect(), &instances);
        let weapon_draws = scene::weapon_draws(mesh, &camera, &self.sim);
        let visible = world_visible + weapon_draws.len();
        // Keep the viewmodel first so draw-budget truncation can never drop it.
        let mut draws = weapon_draws;
        draws.extend(world_draws);
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

    fn update_presentation(&mut self, frame_dt: f32) {
        let dt = if frame_dt.is_finite() {
            frame_dt.max(0.0)
        } else {
            0.0
        };
        self.title_timer += dt;
        self.status_timer += dt;

        let outcome = self.sim.outcome();
        if outcome != self.last_outcome {
            self.last_outcome = outcome;
            match outcome {
                Outcome::Running => {
                    log::info!(target: "fate.combat", "slice restarted");
                }
                Outcome::Victory => {
                    log::info!(
                        target: "fate.combat",
                        "victory at tick {} with {} health and {:.1}s remaining",
                        self.sim.tick(),
                        self.sim.player.health,
                        remaining_seconds(&self.sim)
                    );
                }
                Outcome::Defeat(reason) => {
                    log::info!(
                        target: "fate.combat",
                        "defeat at tick {}: {}",
                        self.sim.tick(),
                        reason.label()
                    );
                }
            }
        }

        if self.title_timer >= TITLE_INTERVAL {
            self.title_timer = 0.0;
            self.update_title();
        }
        if self.status_timer >= STATUS_LOG_INTERVAL {
            self.status_timer = 0.0;
            self.log_status();
        }
    }

    fn title_text(&self) -> String {
        format!(
            "fate combat slice | {} | health {} | ammo {}/{} | enemies {}/{} | tick {} | \
             {:.0} fps | sim {:.2} draw {:.2} submit {:.2} verify {:.2} ms | \
             drawn {}/{} visible {} | replay {} snapshot {} | hash {:016x}",
            outcome_text(self.sim.outcome()),
            self.sim.player.health,
            self.sim.player.ammo,
            MAGAZINE_SIZE,
            self.sim.enemies_alive(),
            self.sim.enemies.len(),
            self.sim.tick(),
            self.time.fps(),
            self.last_sim_ms,
            self.last_draw_list_ms,
            self.last_render_ms,
            self.last_verify_ms,
            self.last_drawn,
            Renderer::max_instance_budget(),
            self.last_visible,
            self.replay_label(),
            self.verification.snapshot_result.label(),
            self.sim.state_hash(),
        )
    }

    fn replay_label(&self) -> &'static str {
        if self.verification.replay_enabled {
            self.verification.replay_result.label()
        } else {
            "off"
        }
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            window.set_title(&self.title_text());
        }
    }

    fn log_status(&self) {
        let outcome = self.sim.outcome();
        if self.verbose {
            log::info!(
                target: "fate.combat",
                "status tick={} outcome={} health={} ammo={}/{} enemies={} shots={} hits={} kills={} paused={} fps={:.0} sim_ms={:.3} draw_list_ms={:.3} submit_ms={:.3} verify_ms={:.3} drawn={}/{} visible={} replay={} snapshot={} hash={:016x}",
                self.sim.tick(),
                outcome.label(),
                self.sim.player.health,
                self.sim.player.ammo,
                MAGAZINE_SIZE,
                self.sim.enemies_alive(),
                self.sim.shots_fired(),
                self.sim.hits(),
                self.sim.kills(),
                self.paused,
                self.time.fps(),
                self.last_sim_ms,
                self.last_draw_list_ms,
                self.last_render_ms,
                self.last_verify_ms,
                self.last_drawn,
                Renderer::max_instance_budget(),
                self.last_visible,
                self.replay_label(),
                self.verification.snapshot_result.label(),
                self.sim.state_hash(),
            );
        } else {
            log::debug!(
                target: "fate.combat",
                "status tick={} outcome={} health={} ammo={}/{} enemies={} hash={:016x}",
                self.sim.tick(),
                outcome.label(),
                self.sim.player.health,
                self.sim.player.ammo,
                MAGAZINE_SIZE,
                self.sim.enemies_alive(),
                self.sim.state_hash(),
            );
        }
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
                log::error!(target: "fate.combat", "failed to capture cursor: {error}");
                self.cursor_captured = false;
                false
            }
        }
    }

    fn release_cursor(&mut self) {
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                log::warn!(target: "fate.combat", "failed to release cursor: {error}");
            }
        }
        self.cursor_captured = false;
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        self.pending_look = (0.0, 0.0);
        self.mouse_fire = false;
        if !paused {
            // Drop wall-clock time accumulated while paused so resuming does not
            // replay a long backlog of fixed steps.
            self.time = Time::new(SIMULATION_HZ as f32);
        }
        log::info!(
            target: "fate.combat",
            "simulation {}",
            if paused { "paused" } else { "running" }
        );
    }

    fn restart(&mut self) {
        self.sim = Sim::new();
        self.verification = Verification::new(&self.sim);
        self.interpolation = Interpolation::new(&self.sim);
        self.last_outcome = Outcome::Running;
        self.paused = false;
        self.mouse_fire = false;
        self.pending_look = (0.0, 0.0);
        self.time = Time::new(SIMULATION_HZ as f32);
        self.input.clear();
        log::info!(target: "fate.combat", "slice restarted");
    }

    fn toggle_verbose(&mut self) {
        self.verbose = !self.verbose;
        log::info!(
            target: "fate.combat",
            "verbose status logging {}",
            if self.verbose { "on" } else { "off" }
        );
    }

    fn toggle_replay(&mut self) {
        let enabled = !self.verification.replay_enabled;
        self.verification.set_replay_enabled(enabled, &self.sim);
        log::info!(
            target: "fate.combat",
            "replay verification {}",
            if enabled { "on" } else { "off" }
        );
    }

    fn force_snapshot_check(&mut self) {
        let start = Instant::now();
        self.verification.force_snapshot_check(&self.sim);
        let elapsed = start.elapsed().as_secs_f32() * 1000.0;
        log::info!(
            target: "fate.combat",
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
                KeyCode::KeyP => {
                    self.set_paused(!self.paused);
                    return;
                }
                KeyCode::Enter => {
                    if !self.sim.outcome().is_running() {
                        self.restart();
                    }
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
            .with_title("fate combat slice")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));

        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        let mut renderer = pollster::block_on(Renderer::new(window.clone()))
            .expect("failed to initialize renderer");
        renderer.camera_3d = scene::camera_for(&self.sim);

        let mesh = match renderer.cube_mesh() {
            Ok(mesh) => Some(mesh),
            Err(error) => {
                log::error!(target: "fate.combat", "failed to create cube mesh: {error}");
                None
            }
        };

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.mesh = mesh;
        self.capture_cursor();

        log::info!(
            target: "fate.combat",
            "controls: WASD/arrows move, mouse look, Shift sprint, Space/left mouse fire, R reload, P pause, F1 verbose, F2 replay, F4 snapshot, Enter restart, Escape release/quit"
        );
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
                if focused {
                    self.capture_cursor();
                } else {
                    self.release_cursor();
                    self.input.clear();
                    self.mouse_fire = false;
                    self.pending_look = (0.0, 0.0);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key(event_loop, event.physical_key, event.state, event.repeat);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    if state == ElementState::Pressed && !self.cursor_captured {
                        self.capture_cursor();
                    } else {
                        self.mouse_fire = state == ElementState::Pressed;
                    }
                }
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
        if self.cursor_captured
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

fn clamp_look(value: f32) -> i16 {
    if value.is_finite() {
        value.clamp(-MAX_LOOK_INPUT as f32, MAX_LOOK_INPUT as f32) as i16
    } else {
        0
    }
}

fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Running => "running".to_string(),
        Outcome::Victory => "victory".to_string(),
        Outcome::Defeat(reason) => format!("defeat ({})", reason.label()),
    }
}

fn remaining_seconds(sim: &Sim) -> f32 {
    crate::sim::TIME_LIMIT_TICKS.saturating_sub(sim.tick()) as f32 / SIMULATION_HZ as f32
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
    use fate_engine::renderer::Camera3d;

    #[test]
    fn new_app_starts_with_a_live_slice_and_no_renderer() {
        let app = App::new();

        assert!(app.renderer.is_none());
        assert!(app.mesh.is_none());
        assert_eq!(app.sim.tick(), 0);
        assert_eq!(app.sim.outcome(), Outcome::Running);
        assert_eq!(app.sim.enemies_alive(), app.sim.enemies.len());
        assert!(app.verbose);
        assert!(!app.paused);
    }

    #[test]
    fn paused_simulation_does_not_advance() {
        let mut app = App::new();
        app.set_paused(true);

        let (sim_ms, verify_ms) = app.advance_simulation();

        assert_eq!((sim_ms, verify_ms), (0.0, 0.0));
        assert_eq!(app.sim.tick(), 0);
    }

    #[test]
    fn restart_resets_simulation_verification_and_timers() {
        let mut app = App::new();
        app.sim.step(SimInput {
            move_y: 1,
            ..SimInput::default()
        });
        app.paused = true;
        app.pending_look = (25.0, -10.0);
        app.mouse_fire = true;

        app.restart();

        assert_eq!(app.sim.tick(), 0);
        assert_eq!(app.sim.state_bytes(), Sim::new().state_bytes());
        assert!(!app.paused);
        assert!(!app.mouse_fire);
        assert_eq!(app.pending_look, (0.0, 0.0));
    }

    #[test]
    fn forced_snapshot_check_reports_ok() {
        let mut app = App::new();
        app.sim.step(SimInput {
            move_y: 1,
            ..SimInput::default()
        });

        app.force_snapshot_check();

        assert_eq!(app.verification.snapshot_result.label(), "ok");
    }

    #[test]
    fn replay_toggle_restarts_the_verification_window() {
        let mut app = App::new();
        app.toggle_replay();
        assert!(!app.verification.replay_enabled);
        assert!(app.title_text().contains("replay off"));

        app.toggle_replay();
        assert!(app.verification.replay_enabled);
    }

    #[test]
    fn title_summarizes_combat_and_verification_state() {
        let app = App::new();
        let title = app.title_text();

        assert!(title.contains("fate combat slice"));
        assert!(title.contains("health 100"));
        assert!(title.contains("ammo 10/10"));
        assert!(title.contains("running"));
        assert!(title.contains("replay pending"));
        assert!(title.contains("hash "));
    }

    #[test]
    fn outcome_text_labels_each_terminal_state() {
        assert_eq!(outcome_text(Outcome::Running), "running");
        assert_eq!(outcome_text(Outcome::Victory), "victory");
        assert_eq!(
            outcome_text(Outcome::Defeat(crate::sim::DefeatReason::TimeExpired)),
            "defeat (time expired)"
        );
    }

    #[test]
    fn look_clamping_rejects_non_finite_values() {
        assert_eq!(clamp_look(f32::NAN), 0);
        assert_eq!(clamp_look(f32::INFINITY), 0);
        assert_eq!(clamp_look(-f32::INFINITY), 0);
        assert_eq!(clamp_look(12.6), 12);
        assert_eq!(clamp_look(-12.6), -12);
        assert_eq!(clamp_look(MAX_LOOK_INPUT as f32 * 2.0), MAX_LOOK_INPUT);
    }

    #[test]
    fn vertical_mouse_motion_is_not_inverted() {
        let mut app = App::new();
        app.input.update_axis(Axis::MouseY, 120.0);
        app.accumulate_look();
        let (_, pitch) = app.take_look();
        assert!(pitch < 0, "mouse down should pitch down");

        let mut app = App::new();
        app.input.update_axis(Axis::MouseY, -120.0);
        app.accumulate_look();
        let (_, pitch) = app.take_look();
        assert!(pitch > 0, "mouse up should pitch up");
    }

    #[test]
    fn headless_frame_step_clears_input_edges_without_a_renderer() {
        let mut app = App::new();
        app.input.press(KeyCode::KeyW);

        let rendered = app.step_frame();

        assert!(!rendered);
        assert!(!app.input.button_just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn camera_helper_tracks_the_player() {
        let sim = Sim::new();
        let camera: Camera3d = scene::camera_for(&sim);

        assert!((camera.position.y - 1.6).abs() < 1e-6);
        assert!(camera.yaw.is_finite());
        assert!(camera.pitch.is_finite());
    }
}
