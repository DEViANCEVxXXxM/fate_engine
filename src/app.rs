use std::sync::Arc;

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

const FIXED_HZ: f32 = 60.0;
/// Half of the playfield, in world units. The camera shows 20x20 by default.
const PLAYFIELD_HALF: Vec2 = Vec2::new(9.0, 8.5);
const MAX_SPEED: f32 = 18.0;
const ACCELERATION: f32 = 40.0;

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    world: World,
    input: Input,
    time: Time,
    player: ecs::Entity,
    title_timer: f32,
}

impl App {
    fn new() -> Self {
        let mut world = World::new();

        let player = world.spawn();
        world.positions.insert(player, Position(Vec2::ZERO));
        world.velocities.insert(player, Velocity(Vec2::new(4.0, 2.5)));
        world.rotations.insert(player, Rotation(0.0));
        world.spins.insert(player, Spin(0.9));

        Self {
            window: None,
            renderer: None,
            world,
            input: Input::new(),
            time: Time::new(FIXED_HZ),
            player,
            title_timer: 0.0,
        }
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

        let boost = if self.input.action_down(Action::Boost) { 2.0 } else { 1.0 };
        let Some(velocity) = self.world.velocities.get_mut(self.player) else {
            return;
        };

        velocity.0 += direction.normalize() * (ACCELERATION * boost * dt);
        if velocity.0.length() > MAX_SPEED * boost {
            velocity.0 = velocity.0.normalize() * (MAX_SPEED * boost);
        }
    }

    fn instances(&self) -> Vec<Instance2d> {
        let mut instances = Vec::new();
        for (entity, position) in self.world.positions.iter() {
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
            window.set_title(&format!(
                "fate - {:.0} fps - {} entities",
                self.time.fps(),
                self.world.entity_count()
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
                let frame_dt = self.time.tick();
                while let Some(dt) = self.time.next_fixed_step() {
                    self.fixed_update(dt);
                }

                let instances = self.instances();
                if let Some(renderer) = &mut self.renderer {
                    renderer.render(&instances);
                }

                self.update_title(frame_dt);
                self.input.end_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

pub async fn run() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
