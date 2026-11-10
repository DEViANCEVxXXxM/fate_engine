use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::input::{Action, Input};
use crate::math::Vec2;
use crate::performance::{FrameBudgetPolicy, FrameProfile, FrameTelemetry, PerformanceStats};
use crate::renderer::{Instance2d, Renderer};
use crate::simulation::{EngineCore, SIMULATION_HZ, SimulationInput};
use crate::time::Time;

impl App {
    fn draw_pressure(&self) -> f32 {
        let budget = Renderer::max_instance_budget() as f32;
        if budget <= 0.0 {
            0.0
        } else {
            (self.frame_profile.drawn_instances as f32 / budget).clamp(0.0, 1.0)
        }
    }

    fn telemetry(&self) -> FrameTelemetry {
        let (avg_ms, worst_ms) = self.performance.snapshot();
        let total_ms = self.frame_profile.total_ms();
        let target_ms = Time::frame_budget_ms() as f32;
        let draw_limit = Renderer::max_instance_budget();
        let draw_pressure = self.draw_pressure();
        let policy = FrameBudgetPolicy::for_frame(total_ms, draw_pressure);
        let (healthy_frames, warning_frames, critical_frames) = self.performance.budget_breakdown();

        FrameTelemetry {
            sim_ms: self.frame_profile.sim_ms,
            render_ms: self.frame_profile.render_ms,
            total_ms,
            avg_ms,
            worst_ms,
            target_ms,
            headroom_ms: target_ms - total_ms,
            draw_count: self.frame_profile.drawn_instances,
            draw_limit,
            draw_pressure_pct: draw_pressure * 100.0,
            healthy_frames,
            warning_frames,
            critical_frames,
            policy,
        }
    }

    fn performance_tier(&self) -> FrameBudgetPolicy {
        FrameBudgetPolicy::for_frame(self.frame_profile.total_ms(), self.draw_pressure())
    }

    fn performance_report(&self) -> String {
        self.telemetry().summary()
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    core: EngineCore,
    input: Input,
    time: Time,
    frame_profile: FrameProfile,
    performance: PerformanceStats,
    title_timer: f32,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            core: EngineCore::new_demo(),
            input: Input::new(),
            time: Time::new(SIMULATION_HZ as f32),
            frame_profile: FrameProfile::default(),
            performance: PerformanceStats::default(),
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
        self.input.end_frame();
        self.update_camera();

        let instances = self.instances();
        let draw_budget = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.budgeted_instance_count(instances.len()))
            .unwrap_or(0);
        let render_start = Instant::now();
        let rendered = if let Some(renderer) = &mut self.renderer {
            renderer.render(&instances)
        } else {
            false
        };
        let render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        self.frame_profile = FrameProfile {
            sim_ms,
            render_ms,
            drawn_instances: draw_budget,
        };
        let policy = self.performance_tier();
        self.performance
            .record_budget(self.frame_profile.total_ms(), policy, self.draw_pressure());

        if self.frame_profile.over_budget() {
            let (avg_ms, worst_ms) = self.performance.snapshot();
            let tier = self.performance_tier();
            log::warn!(
                "frame budget exceeded: {} avg={:.2}ms worst={:.2}ms tier={}",
                self.performance_report(),
                avg_ms,
                worst_ms,
                tier.label(),
            );
        }

        self.update_title(frame_dt);
        rendered
    }

    #[allow(dead_code)]
    fn headless_step(&mut self) -> bool {
        self.step_frame()
    }

    fn simulation_input(&self) -> SimulationInput {
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
            .positions
            .get(self.core.player)
            .map(|position| position.0.to_vec2())
    }

    fn update_camera(&mut self) {
        let Some(target) = self.player_camera_target() else {
            return;
        };
        if let Some(renderer) = &mut self.renderer {
            renderer.camera.center = target;
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
            event_loop.exit();
            return;
        }

        if code == KeyCode::F2 && state == ElementState::Pressed && !repeat {
            let enable_stress = !self.core.demo_stress_enabled();
            self.core.set_demo_stress(enable_stress);
            return;
        }

        match state {
            ElementState::Pressed => self.input.press(code),
            ElementState::Released => self.input.release(code),
        }
    }

    fn instances(&self) -> Vec<Instance2d> {
        let view_size = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.camera.safe_visible_size())
            .unwrap_or(Vec2::new(20.0, 20.0));
        let half = view_size * 0.5;
        let center = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.camera.center)
            .unwrap_or(Vec2::ZERO);

        let mut instances = Vec::new();
        for (entity, position) in self.core.world.positions.iter() {
            let render_position = position.0.to_vec2();
            let delta = render_position - center;
            if delta.x.abs() > half.x || delta.y.abs() > half.y {
                continue;
            }

            let rotation = self
                .core
                .world
                .rotations
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
            let budget_text = if headroom >= 0.0 {
                format!("+{headroom:.1}ms")
            } else {
                format!("{headroom:.1}ms")
            };

            window.set_title(&format!(
                "fate | WASD/arrows move, Shift boost, F2 stress {} | {:.0} fps - {} entities - {} / budget {}",
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

        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let frame_budget = Duration::from_millis(Time::frame_budget_ms());
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame_budget));

        if let Some(window) = &self.window {
            window.request_redraw();
        }
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
    use crate::ecs::Position;
    use crate::math::FixedVec2;
    use crate::performance::PerformanceStats;

    #[test]
    fn step_frame_without_renderer_is_skipped_cleanly() {
        let mut app = App::new();

        let rendered = app.step_frame();

        assert!(!rendered);
        assert_eq!(app.core.world.entity_count(), 40);
        assert!(app.frame_profile.sim_ms >= 0.0);
        assert!(app.frame_profile.render_ms >= 0.0);
        assert!(app.frame_profile.total_ms() >= 0.0);
    }

    #[test]
    fn frame_profile_reports_budget_exceedance() {
        let profile = FrameProfile {
            sim_ms: 4.0,
            render_ms: 5.0,
            drawn_instances: 3,
        };

        assert!(profile.over_budget());
        assert!((profile.total_ms() - 9.0).abs() < 1e-5);
    }

    #[test]
    fn step_frame_clears_input_edge_state_even_without_renderer() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyA);
        app.input.end_frame();

        app.input.press(winit::keyboard::KeyCode::KeyA);
        let _ = app.step_frame();

        assert!(!app.input.key_just_pressed(winit::keyboard::KeyCode::KeyA));
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
    fn performance_stats_roll_and_average_recent_frames() {
        let mut stats = PerformanceStats::default();
        stats.add(4.0);
        stats.add(6.0);
        stats.add(8.0);

        assert!((stats.average_ms() - 6.0).abs() < 1e-5);
        assert!((stats.worst_ms() - 8.0).abs() < 1e-5);
        assert_eq!(stats.recent_count, 3);
        let (avg, worst) = stats.snapshot();
        assert!((avg - 6.0).abs() < 1e-5);
        assert!((worst - 8.0).abs() < 1e-5);
    }

    #[test]
    fn performance_stats_track_budget_policy_breakdown() {
        let mut stats = PerformanceStats::default();

        stats.record_budget(7.0, FrameBudgetPolicy::Healthy, 0.25);
        stats.record_budget(9.0, FrameBudgetPolicy::Warning, 0.8);
        stats.record_budget(12.0, FrameBudgetPolicy::Critical, 1.0);

        let (healthy, warning, critical) = stats.budget_breakdown();
        assert_eq!((healthy, warning, critical), (1, 1, 1));
        assert!((stats.peak_draw_pressure_pct() - 100.0).abs() < 1e-5);
    }

    #[test]
    fn performance_report_contains_budget_summary() {
        let mut app = App::new();
        app.frame_profile = FrameProfile {
            sim_ms: 3.0,
            render_ms: 4.0,
            drawn_instances: 2,
        };
        app.performance.add(7.0);
        app.performance.add(9.0);

        let report = app.performance_report();
        assert!(report.contains("drawn=2/256"));
        assert!(report.contains("load=0.8%"));
        assert!(report.contains("total=7.0ms"));
        assert!(report.contains("target=8.0ms"));
        assert!(report.contains("headroom=1.0ms"));
        assert!(report.contains("tier=healthy"));
    }

    #[test]
    fn performance_tier_classifies_warning_and_critical_budget_slips() {
        let mut app = App::new();
        app.frame_profile = FrameProfile {
            sim_ms: 8.0,
            render_ms: 2.0,
            drawn_instances: 4,
        };
        assert_eq!(app.performance_tier(), FrameBudgetPolicy::Warning);

        app.frame_profile = FrameProfile {
            sim_ms: 10.0,
            render_ms: 10.0,
            drawn_instances: 8,
        };
        assert_eq!(app.performance_tier(), FrameBudgetPolicy::Critical);
    }

    #[test]
    fn draw_pressure_tracks_budget_usage() {
        let mut app = App::new();
        app.frame_profile.drawn_instances = 192;
        assert!((app.draw_pressure() - 0.75).abs() < 1e-5);

        app.frame_profile.drawn_instances = 256;
        assert!((app.draw_pressure() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn telemetry_snapshot_exposes_machine_readable_budget_data() {
        let mut app = App::new();
        app.frame_profile = FrameProfile {
            sim_ms: 3.0,
            render_ms: 4.0,
            drawn_instances: 128,
        };
        app.performance.add(7.0);
        app.performance.add(9.0);

        let telemetry = app.telemetry();
        assert_eq!(telemetry.draw_count, 128);
        assert_eq!(telemetry.draw_limit, Renderer::max_instance_budget());
        assert!(telemetry.draw_pressure_pct > 0.0);
        assert_eq!(telemetry.policy, FrameBudgetPolicy::Healthy);
        assert!(telemetry.summary().contains("tier=healthy"));
    }

    #[test]
    fn headless_simulation_runs_without_window_or_renderer() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyA);

        let rendered = app.headless_step();

        assert!(!rendered);
        assert!(app.core.world.entity_count() >= 1);
        assert!(app.frame_profile.total_ms() >= 0.0);
        assert!(!app.input.key_just_pressed(winit::keyboard::KeyCode::KeyA));
    }

    #[test]
    fn app_can_step_through_headless_simulation_isolation() {
        let mut app = App::new();
        app.input.press(winit::keyboard::KeyCode::KeyD);

        let rendered = app.headless_step();

        assert!(!rendered);
        assert_eq!(app.core.world.entity_count(), 40);
        assert!(app.frame_profile.total_ms() >= 0.0);
    }

    #[test]
    fn instances_only_include_entities_in_camera_view() {
        let mut app = App::new();
        let visible = app.core.world.spawn();
        let hidden = app.core.world.spawn();
        app.core.world.positions.insert(
            visible,
            Position(FixedVec2::new(
                crate::math::Fixed::ONE,
                crate::math::Fixed::ONE,
            )),
        );
        app.core.world.positions.insert(
            hidden,
            Position(FixedVec2::new(
                crate::math::Fixed::from_int(30),
                crate::math::Fixed::from_int(30),
            )),
        );

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
        app.frame_profile.drawn_instances = app.instances().len();
        assert!((app.draw_pressure() - 0.75).abs() < 1e-5);
        assert_eq!(app.performance_tier(), FrameBudgetPolicy::Warning);

        app.core.set_demo_stress(false);
        assert_eq!(app.instances().len(), 32);
        assert_eq!(app.core.world.entity_count(), 40);
    }

    #[test]
    fn player_camera_target_tracks_player_position() {
        let mut app = App::new();
        let player = app.core.player;
        let target = FixedVec2::new(
            crate::math::Fixed::from_int(4),
            crate::math::Fixed::from_int(-3),
        );
        app.core.world.positions.insert(player, Position(target));

        assert_eq!(app.player_camera_target(), Some(target.to_vec2()));
    }

    #[test]
    fn player_is_visually_distinguished_from_demo_swarm() {
        let app = App::new();
        let instances = app.instances();
        let player_position = app
            .core
            .world
            .positions
            .get(app.core.player)
            .unwrap()
            .0
            .to_vec2();
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
}
