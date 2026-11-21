mod bench_compare;
mod bench_io;
mod bench_schema;
mod cli;
mod contact_grid;
mod ecs_lab;
mod headless;
mod lab;
mod navigation;
mod render_bench;
mod scene;
mod settings;

use std::sync::Arc;
use std::time::{Duration, Instant};

use fate_debug::{
    Command, Control, ControlValue, DebugModel, DebugWindow, EventLevel, FrameSample, GpuSample,
    Inspection, Page,
};
use fate_engine::renderer::{Renderer, RendererOptions};
use fate_engine::time::Time;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use lab::{Lab, Work};
use scene::Viewport;
use settings::{Preset, Settings};

const CONTROLS_INTERVAL: Duration = Duration::from_millis(50);

fn main() {
    match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Launch::Help) => cli::help(),
        Ok(cli::Launch::Live {
            settings,
            timing,
            smoke,
        }) => run_live(settings, timing, smoke),
        Ok(cli::Launch::Headless(bench)) => {
            if let Err(error) = headless::run(bench) {
                eprintln!("fate_polygon: {error}");
                std::process::exit(1);
            }
        }
        Ok(cli::Launch::Offscreen(bench)) => {
            if let Err(error) = render_bench::run(bench) {
                eprintln!("fate_polygon: {error}");
                std::process::exit(1);
            }
        }
        Ok(cli::Launch::Compare(options)) => match bench_compare::run(options) {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(error) => {
                eprintln!("fate_polygon: {error}");
                std::process::exit(2);
            }
        },
        Err(error) => {
            eprintln!("fate_polygon: {error}. Run with --help for usage.");
            std::process::exit(2);
        }
    }
}

fn run_live(settings: Settings, timing: bool, smoke: bool) {
    let mut app = App::new(settings, timing, smoke);
    EventLoop::new()
        .expect("create Polygon event loop")
        .run_app(&mut app)
        .expect("run Polygon");
    if smoke {
        assert_eq!(
            app.smoke_seen, 0b111_1111,
            "not all Polygon presets submitted and did work"
        );
        println!(
            "fate_polygon smoke: {} frames; mixed, render (with and without culling), grid physics, steering, navigation, and ECS verified",
            app.frames
        );
    }
}

struct App {
    window: Option<Arc<Window>>,
    view: Option<Viewport>,
    controls: Option<DebugWindow>,
    model: DebugModel,
    settings: Settings,
    lab: Lab,
    clock: Time,
    paused: bool,
    step_once: bool,
    work: Work,
    steps: usize,
    last_frame: FrameSample,
    gpu_requested: bool,
    smoke: bool,
    smoke_seen: u8,
    frames: u32,
    next_frame: Instant,
    next_controls: Instant,
}

impl App {
    fn new(settings: Settings, gpu_requested: bool, smoke: bool) -> Self {
        let mut lab = Lab::default();
        lab.sync_counts(settings);
        let now = Instant::now();
        Self {
            window: None,
            view: None,
            controls: None,
            model: DebugModel::default(),
            settings,
            lab,
            clock: Time::new(settings.tick_hz as f32),
            paused: false,
            step_once: false,
            work: Work::default(),
            steps: 0,
            last_frame: FrameSample::default(),
            gpu_requested,
            smoke,
            smoke_seen: 0,
            frames: 0,
            next_controls: now,
            next_frame: now,
        }
    }

    fn reset(&mut self) {
        self.lab = Lab::default();
        self.lab.sync_counts(self.settings);
        self.clock = Time::new(self.settings.tick_hz as f32);
        self.step_once = false;
    }

    fn preset(&mut self, preset: Preset) {
        self.settings = Settings::preset(preset);
        self.paused = false;
        self.reset();
    }

    fn pause(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.clock = Time::new(self.settings.tick_hz as f32);
            self.step_once = false;
        }
    }

    fn command(&mut self, Command { id, value }: Command) {
        match (id, value) {
            ("mixed", ControlValue::Action) => self.preset(Preset::Mixed),
            ("render_preset", ControlValue::Action) => self.preset(Preset::Render),
            ("physics_preset", ControlValue::Action) => self.preset(Preset::Physics),
            ("agents_preset", ControlValue::Action) => self.preset(Preset::Agents),
            ("navigation_preset", ControlValue::Action) => self.preset(Preset::Navigation),
            ("ecs_preset", ControlValue::Action) => self.preset(Preset::Ecs),
            ("reset", ControlValue::Action) => self.reset(),
            ("step", ControlValue::Action) if self.paused => self.step_once = true,
            ("paused", ControlValue::Toggle(on)) => self.pause(on),
            (id, ControlValue::Toggle(on)) => {
                self.settings.apply_toggle(id, on);
                if id == "ecs" {
                    self.lab.sync_counts(self.settings);
                }
            }
            (id, ControlValue::Number { value, .. }) => {
                let before = self.settings;
                if !self.settings.apply_number(id, value) {
                    return;
                }
                if before.seed != self.settings.seed {
                    self.reset();
                } else {
                    self.lab.sync_counts(self.settings);
                    if before.tick_hz != self.settings.tick_hz {
                        self.clock = Time::new(self.settings.tick_hz as f32);
                    }
                }
            }
            _ => {}
        }
    }

    fn refresh_inspection(&mut self) {
        let s = self.settings;
        let mut i = Inspection::default();
        i.controls.extend([
            Control::action(Page::Simulation, "mixed", "Mixed preset"),
            Control::action(Page::Simulation, "render_preset", "Render preset"),
            Control::action(Page::Simulation, "physics_preset", "Physics preset"),
            Control::action(Page::Simulation, "agents_preset", "AI preset"),
            Control::action(Page::Simulation, "navigation_preset", "Navigation preset"),
            Control::action(Page::Simulation, "ecs_preset", "ECS preset"),
            Control::toggle(Page::Simulation, "paused", "Pause fixed steps", self.paused),
            Control {
                enabled: self.paused,
                ..Control::action(Page::Simulation, "step", "Advance one tick")
            },
            Control::action(Page::Simulation, "reset", "Reset to seed"),
            Control::toggle(
                Page::Simulation,
                "physics",
                "Physics workload (lab)",
                s.physics,
            ),
            Control::toggle(Page::Simulation, "ai", "AI workload (lab)", s.ai),
            Control::toggle(Page::Simulation, "ecs", "ECS workload", s.ecs),
            Control::toggle(
                Page::Renderer,
                "render",
                "Render stress mesh grid",
                s.render,
            ),
            Control::toggle(
                Page::Renderer,
                "preview",
                "Simulation preview (up to 40 per workload)",
                s.preview,
            ),
            Control::toggle(
                Page::Renderer,
                "cull",
                "Frustum-cull mesh candidates",
                s.cull,
            ),
            Control::toggle(Page::Renderer, "orbit", "Orbit viewport camera", s.orbit),
        ]);
        i.controls.extend(s.number_controls());
        i.fact(
            Page::Simulation,
            "Workloads",
            "Lab-owned physics and AI; engine ECS and snapshots",
        );
        i.fact(Page::Simulation, "Fixed tick", self.lab.tick);
        i.fact(
            Page::Simulation,
            "Steps / queued",
            format!("{} / {}", self.steps, self.clock.pending_fixed_steps()),
        );
        i.fact(
            Page::Simulation,
            "Physics CPU / frame",
            format!("{:.3} ms", self.work.physics.as_secs_f64() * 1e3),
        );
        i.fact(
            Page::Simulation,
            "Pair tests / contacts",
            format!("{} / {}", self.work.pairs, self.work.contacts),
        );
        i.fact(
            Page::Simulation,
            "AI CPU / frame",
            format!("{:.3} ms", self.work.ai.as_secs_f64() * 1e3),
        );
        i.fact(
            Page::Simulation,
            "AI mode",
            match (s.ai, s.nav_mode) {
                (false, _) => "Off",
                (true, 0) => "Sampled steering",
                (true, 1) => "Per-agent grid search",
                (true, _) => "Shared distance field",
            },
        );
        if s.nav_mode == 0 {
            i.fact(
                Page::Simulation,
                "Decisions / neighbor tests",
                format!("{} / {}", self.work.decisions, self.work.neighbors),
            );
        } else {
            i.fact(
                Page::Simulation,
                "Decisions / route moves",
                format!("{} / {}", self.work.decisions, self.work.route.moves),
            );
            i.fact(
                Page::Simulation,
                "Searches / cells visited",
                format!("{} / {}", self.work.route.searches, self.work.route.visits),
            );
        }
        if let Some(ecs) = &self.lab.ecs {
            let (live, capacity) = ecs.live_and_capacity();
            let work = &self.work.ecs;
            i.fact(
                Page::Simulation,
                "ECS live / allocated",
                format!("{live} / {capacity}"),
            );
            i.fact(
                Page::Simulation,
                "ECS update CPU / frame",
                format!("{:.3} ms", work.update_time.as_secs_f64() * 1e3),
            );
            i.fact(
                Page::Simulation,
                "ECS spawns / despawns",
                format!("{} / {}", work.spawned, work.despawned),
            );
            i.fact(
                Page::Simulation,
                "Snapshots / encoded bytes",
                format!("{} / {}", work.snapshots, work.bytes),
            );
            i.fact(
                Page::Simulation,
                "Snapshot CPU / frame",
                format!("{:.3} ms", work.snapshot_time.as_secs_f64() * 1e3),
            );
        }
        i.fact(
            Page::Renderer,
            "Triangles per stress mesh",
            s.detail * s.detail * 2,
        );
        i.fact(
            Page::Renderer,
            "Candidates / after culling",
            format!(
                "{} / {}",
                self.last_frame.candidates, self.last_frame.visible
            ),
        );
        i.fact(
            Page::Renderer,
            "Submitted / draw calls",
            format!(
                "{} / {}",
                self.last_frame.instances, self.last_frame.draw_calls
            ),
        );
        i.fact(
            Page::Renderer,
            "Engine instance limit",
            Renderer::max_instance_budget(),
        );
        i.fact(
            Page::Renderer,
            "GPU timestamps",
            if self.model.gpu_available {
                "Enabled"
            } else if self.gpu_requested {
                "Unavailable on this device"
            } else {
                "Off; launch with --dbg to enable"
            },
        );
        i.fact(
            Page::Overview,
            "Workload controls",
            "Simulation and Renderer pages",
        );
        i.fact(
            Page::Overview,
            "Viewport keys",
            "Space pause · R reset · F3 reopen controls",
        );
        self.model.inspection = i;
    }

    fn open_controls(&mut self, loop_: &ActiveEventLoop) {
        if self.controls.is_some() {
            return;
        }
        match pollster::block_on(DebugWindow::new(loop_)) {
            Ok(mut controls) => {
                controls.window().set_title("fate_polygon / Controls");
                controls.select_page(Page::Simulation);
                if let Some(view) = &self.window
                    && let Some(monitor) =
                        view.current_monitor().or_else(|| loop_.primary_monitor())
                    && monitor.size().width >= 1600
                {
                    let origin = monitor.position();
                    let ui_width = (monitor.size().width / 2).min(1100);
                    let scale = monitor.scale_factor();
                    let _ = controls
                        .window()
                        .request_inner_size(LogicalSize::new(f64::from(ui_width) / scale, 680.0));
                    let _ = view.request_inner_size(LogicalSize::new(
                        f64::from(ui_width.saturating_sub(32)) / scale,
                        600.0,
                    ));
                    controls
                        .window()
                        .set_outer_position(PhysicalPosition::new(origin.x + 12, origin.y + 32));
                    view.set_outer_position(PhysicalPosition::new(
                        origin.x + ui_width as i32 + 24,
                        origin.y + 32,
                    ));
                }
                self.controls = Some(controls);
                self.refresh_inspection();
                self.next_controls = Instant::now();
            }
            Err(error) => {
                eprintln!("Unable to open Polygon controls (F3 retries): {error}");
                self.model.event(EventLevel::Error, error.to_string());
            }
        }
    }

    fn draw_viewport(&mut self, loop_: &ActiveEventLoop) {
        if self.smoke {
            match self.frames {
                20 => self.preset(Preset::Render),
                30 => {
                    self.settings.cull = true;
                    self.settings.candidates = 4096;
                }
                40 => {
                    self.preset(Preset::Physics);
                    self.settings.broad_phase = 1;
                }
                60 => self.preset(Preset::Agents),
                80 => self.preset(Preset::Navigation),
                100 => {
                    self.preset(Preset::Ecs);
                    self.settings.snapshot_every = 4;
                }
                _ => {}
            }
        }
        let Some(view) = &mut self.view else { return };
        self.frames += 1;
        let delta = if self.paused { 0.0 } else { self.clock.tick() };
        self.work = Work::default();
        self.steps = 0;
        let sim = Instant::now();
        if self.paused {
            if std::mem::take(&mut self.step_once) {
                self.lab.step(self.settings, &mut self.work);
                self.steps = 1;
            }
        } else {
            while self.clock.next_fixed_step().is_some() {
                self.lab.step(self.settings, &mut self.work);
                self.steps += 1;
            }
        }
        let sim_ms = sim.elapsed().as_secs_f32() * 1e3;
        let rendered = view.render(self.settings, &self.lab, self.paused, delta);
        if let Some(f) = rendered.event {
            self.last_frame = FrameSample {
                tick: self.lab.tick,
                interval_ms: f.interval.map_or(0.0, |t| t.as_secs_f32() * 1e3),
                sim_ms,
                prepare_ms: rendered.prepare_ms,
                submit_ms: f.render_time.as_secs_f32() * 1e3,
                record_ms: f.record_time.as_secs_f32() * 1e3,
                instances: f.submitted_instances,
                draw_calls: f.draw_calls,
                visible: f.requested_instances,
                candidates: rendered.candidates,
                submitted: f.submitted,
                ..FrameSample::default()
            };
            self.model.record_frame(self.last_frame);
            if self.smoke && f.submitted {
                match self.frames {
                    1..=20
                        if self.work.pairs > 0 && self.work.decisions > 0 && f.draw_calls > 0 =>
                    {
                        self.smoke_seen |= 1
                    }
                    21..=30
                        if self.work.pairs == 0
                            && self.work.decisions == 0
                            && f.submitted_instances == Renderer::max_instance_budget()
                            && f.draw_calls > 1 =>
                    {
                        self.smoke_seen |= 2
                    }
                    31..=40
                        if f.requested_instances < rendered.candidates
                            && f.submitted_instances > 0
                            && f.draw_calls > 1 =>
                    {
                        self.smoke_seen |= 64
                    }
                    41..=60
                        if self.work.pairs > 0
                            && self.work.decisions == 0
                            && self.work.pairs
                                < u64::from(
                                    self.settings.bodies * (self.settings.bodies - 1) / 2
                                        * self.settings.passes,
                                ) * self.steps as u64 =>
                    {
                        self.smoke_seen |= 4
                    }
                    61..=80 if self.work.pairs == 0 && self.work.neighbors > 0 => {
                        self.smoke_seen |= 8
                    }
                    81..=100 if self.work.route.moves > 0 && self.work.neighbors == 0 => {
                        self.smoke_seen |= 16
                    }
                    101..=120 if self.work.ecs.spawned > 0 && self.work.ecs.snapshots > 0 => {
                        self.smoke_seen |= 32
                    }
                    _ => {}
                }
            }
        }
        for g in rendered.gpu {
            self.model.record_gpu(GpuSample {
                id: g.sample_id,
                scene_ms: g.scene.as_secs_f32() * 1e3,
                overlay_ms: g.overlay.map(|t| t.as_secs_f32() * 1e3),
            });
        }
        if self.smoke && self.frames >= 120 {
            loop_.exit();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, loop_: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            loop_
                .create_window(
                    Window::default_attributes()
                        .with_title("fate_polygon / Viewport")
                        .with_inner_size(LogicalSize::new(880.0, 620.0))
                        .with_min_inner_size(LogicalSize::new(480.0, 320.0)),
                )
                .expect("open Polygon viewport"),
        );
        let renderer = pollster::block_on(Renderer::new_with_options(
            window.clone(),
            RendererOptions {
                gpu_timing: self.gpu_requested,
            },
        ))
        .expect("initialize Polygon renderer");
        self.model.gpu_available = renderer.gpu_timing_available();
        self.view = Some(Viewport::new(renderer, self.settings.detail));
        self.window = Some(window);
        self.open_controls(loop_);
        self.clock = Time::new(self.settings.tick_hz as f32);
    }

    fn window_event(&mut self, loop_: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(controls) = self.controls.as_mut().filter(|window| window.id() == id) {
            if controls.on_event(&event) {
                self.controls = None;
            } else if matches!(event, WindowEvent::RedrawRequested) {
                let commands = controls.draw(&mut self.model);
                for command in commands {
                    self.command(command);
                }
                self.refresh_inspection();
            }
            return;
        }
        if self.window.as_ref().is_none_or(|window| window.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => loop_.exit(),
            WindowEvent::Resized(size) => {
                if let Some(view) = &mut self.view {
                    view.renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => self.draw_viewport(loop_),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Space) => self.pause(!self.paused),
                    PhysicalKey::Code(KeyCode::KeyR) => self.reset(),
                    PhysicalKey::Code(KeyCode::F3) => self.open_controls(loop_),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, loop_: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            self.next_frame =
                now + Duration::from_secs_f64(1.0 / f64::from(self.settings.frame_hz));
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        if self.controls.is_some() && now >= self.next_controls {
            self.next_controls = now + CONTROLS_INTERVAL;
            self.refresh_inspection();
            self.controls
                .as_ref()
                .expect("controls still open")
                .request_redraw();
        }
        let next = if self.controls.is_some() {
            self.next_frame.min(self.next_controls)
        } else {
            self.next_frame
        };
        loop_.set_control_flow(ControlFlow::WaitUntil(next));
    }
}
