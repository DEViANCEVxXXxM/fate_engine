use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::ecs::{self, Position, Rotation, Spin, Velocity, World};
use crate::input::{Action, Input};
use crate::math::Vec2;
use crate::renderer::{Instance2d, Renderer};
use crate::time::Time;

const FIXED_HZ: f32 = 120.0;
/// Half of the playfield, in world units. The camera shows 20x20 by default.
const PLAYFIELD_HALF: Vec2 = Vec2::new(9.0, 8.5);
const MAX_SPEED: f32 = 18.0;
const ACCELERATION: f32 = 40.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FrameProfile {
    sim_ms: f32,
    render_ms: f32,
}

impl FrameProfile {
    fn total_ms(self) -> f32 {
        self.sim_ms + self.render_ms
    }

    fn over_budget(self) -> bool {
        self.total_ms() > Time::frame_budget_ms() as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PerformanceStats {
    recent_frame_times: [f32; 60],
    recent_count: usize,
}

impl Default for PerformanceStats {
    fn default() -> Self {
        Self {
            recent_frame_times: [0.0; 60],
            recent_count: 0,
        }
    }
}

impl PerformanceStats {
    fn add(&mut self, frame_ms: f32) {
        self.recent_frame_times[self.recent_count % self.recent_frame_times.len()] = frame_ms;
        self.recent_count = (self.recent_count + 1).min(self.recent_frame_times.len());
    }

    fn average_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        let sum: f32 = self.recent_frame_times.iter().take(count).sum();
        sum / count as f32
    }

    fn worst_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        self.recent_frame_times
            .iter()
            .take(count)
            .fold(0.0, |max, value| max.max(*value))
    }

    fn snapshot(&self) -> (f32, f32) {
        (self.average_ms(), self.worst_ms())
    }
}

impl App {
    fn performance_report(&self) -> String {
        let (avg_ms, worst_ms) = self.performance.snapshot();
        let total_ms = self.frame_profile.total_ms();
        let target_ms = Time::frame_budget_ms() as f32;
        let headroom_ms = target_ms - total_ms;

        format!(
            "sim={:.1}ms render={:.1}ms total={:.1}ms avg={:.1}ms worst={:.1}ms target={:.1}ms headroom={:.1}ms",
            self.frame_profile.sim_ms,
            self.frame_profile.render_ms,
            total_ms,
            avg_ms,
            worst_ms,
            target_ms,
            headroom_ms,
        )
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    world: World,
    input: Input,
    time: Time,
    player: ecs::Entity,
    title_timer: f32,
    frame_profile: FrameProfile,
    performance: PerformanceStats,
}

impl App {
    fn spawn_demo_player(world: &mut World) -> ecs::Entity {
        let player = world.spawn();
        world.positions.insert(player, Position(Vec2::ZERO));
        world
            .velocities
            .insert(player, Velocity(Vec2::new(4.0, 2.5)));
        world.rotations.insert(player, Rotation(0.0));
        world.spins.insert(player, Spin(0.9));
        player
    }

    fn new() -> Self {
        let mut world = World::new();
        let player = Self::spawn_demo_player(&mut world);

        Self {
            window: None,
            renderer: None,
            world,
            input: Input::new(),
            time: Time::new(FIXED_HZ),
            player,
            title_timer: 0.0,
            frame_profile: FrameProfile::default(),
            performance: PerformanceStats::default(),
        }
    }

    fn step_frame(&mut self) -> bool {
        let frame_dt = self.time.tick();
        let sim_start = Instant::now();
        while let Some(dt) = self.time.next_fixed_step() {
            self.fixed_update(dt);
        }
        let sim_ms = sim_start.elapsed().as_secs_f32() * 1000.0;

        let instances = self.instances();
        let render_start = Instant::now();
        let rendered = if let Some(renderer) = &mut self.renderer {
            renderer.render(&instances)
        } else {
            false
        };
        let render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        self.frame_profile = FrameProfile { sim_ms, render_ms };
        self.performance.add(self.frame_profile.total_ms());

        if self.frame_profile.over_budget() {
            let (avg_ms, worst_ms) = self.performance.snapshot();
            log::warn!(
                "frame budget exceeded: {} avg={:.2}ms worst={:.2}ms",
                self.performance_report(),
                avg_ms,
                worst_ms,
            );
        }

        self.update_title(frame_dt);
        self.input.end_frame();
        rendered
    }

    fn handle_key(&mut self, event_loop: &ActiveEventLoop, key: PhysicalKey, state: ElementState) {
        let PhysicalKey::Code(code) = key else {
            return;
        };

        if code == KeyCode::Escape && state == ElementState::Pressed {
            event_loop.exit();
            return;
        }

        match state {
            ElementState::Pressed => self.input.press(code),
            ElementState::Released => self.input.release(code),
        }
    }

    /// Fixed-step simulation: input, movement, spin, bounce.
    fn fixed_update(&mut self, dt: f32) {
        self.apply_input(dt);
        ecs::movement_system(&mut self.world, dt);
        ecs::spin_system(&mut self.world, dt);
        ecs::bounds_system(&mut self.world, PLAYFIELD_HALF);
    }

    fn apply_input(&mut self, dt: f32) {
        let mut direction = Vec2::ZERO;
        if self.input.action_down(Action::MoveLeft) {
            direction.x -= 1.0;
        }
        if self.input.action_down(Action::MoveRight) {
            direction.x += 1.0;
        }
        if self.input.action_down(Action::MoveDown) {
            direction.y -= 1.0;
        }
        if self.input.action_down(Action::MoveUp) {
            direction.y += 1.0;
        }

        if direction == Vec2::ZERO {
            return;
        }

        let boost = if self.input.action_down(Action::Boost) {
            2.0
        } else {
            1.0
        };
        let Some(velocity) = self.world.velocities.get_mut(self.player) else {
            return;
        };

        velocity.0 += direction.normalize() * (ACCELERATION * boost * dt);
        if velocity.0.length() > MAX_SPEED * boost {
            velocity.0 = velocity.0.normalize() * (MAX_SPEED * boost);
        }
    }

    fn instances(&self) -> Vec<Instance2d> {
        let view_size = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.camera.view_size)
            .unwrap_or(Vec2::new(20.0, 20.0));
        let half = view_size * 0.5;
        let center = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.camera.center)
            .unwrap_or(Vec2::ZERO);

        let mut instances = Vec::new();
        for (entity, position) in self.world.positions.iter() {
            let delta = position.0 - center;
            if delta.x.abs() > half.x || delta.y.abs() > half.y {
                continue;
            }

            let rotation = self.world.rotations.get(entity).map_or(0.0, |r| r.0);
            instances.push(Instance2d {
                position: position.0,
                rotation,
                scale: 1.0,
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
                "fate - {:.0} fps - {} entities - {} / budget {}",
                self.time.fps(),
                self.world.entity_count(),
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
                self.handle_key(event_loop, event.physical_key, event.state);
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

    #[test]
    fn step_frame_without_renderer_is_skipped_cleanly() {
        let mut app = App::new();

        let rendered = app.step_frame();

        assert!(!rendered);
        assert_eq!(app.world.entity_count(), 1);
        assert!(app.frame_profile.sim_ms >= 0.0);
        assert!(app.frame_profile.render_ms >= 0.0);
        assert!(app.frame_profile.total_ms() >= 0.0);
    }

    #[test]
    fn frame_profile_reports_budget_exceedance() {
        let profile = FrameProfile {
            sim_ms: 4.0,
            render_ms: 5.0,
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
    fn performance_report_contains_budget_summary() {
        let mut app = App::new();
        app.frame_profile = FrameProfile { sim_ms: 3.0, render_ms: 4.0 };
        app.performance.add(7.0);
        app.performance.add(9.0);

        let report = app.performance_report();
        assert!(report.contains("total=7.0ms"));
        assert!(report.contains("target=8.0ms"));
        assert!(report.contains("headroom=1.0ms"));
    }

    #[test]
    fn instances_only_include_entities_in_camera_view() {
        let mut app = App::new();
        let visible = app.world.spawn();
        let hidden = app.world.spawn();
        app.world.positions.insert(visible, Position(Vec2::new(1.0, 1.0)));
        app.world.positions.insert(hidden, Position(Vec2::new(30.0, 30.0)));

        let instances = app.instances();

        assert_eq!(instances.len(), 2);
        assert!(instances.iter().all(|instance| instance.position != Vec2::new(30.0, 30.0)));
    }
}
