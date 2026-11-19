//! Showcase adapter for the reusable native diagnostics companion.

use super::*;
use fate_debug::{
    Command, Control, ControlValue, DebugModel, DebugWindow, EntityRow, EventLevel, GpuSample,
    Inspection, Page,
};

pub(super) struct DebugSession {
    pub window: Option<DebugWindow>,
    pub logs: Option<fate_debug::LogInbox>,
    model: DebugModel,
    last_frame: Instant,
    last_inspection: Instant,
    last_redraw: Instant,
    verify_labels: [String; 3],
    smoke: bool,
    smoke_frame: usize,
    ui_frames: usize,
}

impl DebugSession {
    pub fn new(smoke: bool) -> Self {
        let now = Instant::now();
        Self {
            window: None,
            logs: None,
            model: DebugModel::default(),
            last_frame: now,
            last_inspection: now,
            last_redraw: now,
            verify_labels: Default::default(),
            smoke,
            smoke_frame: 0,
            ui_frames: 0,
        }
    }
}

impl App {
    pub(super) fn open_debug(&mut self, event_loop: &ActiveEventLoop) {
        let Some(debug) = &mut self.debug else {
            return;
        };
        if debug.window.is_some() {
            return;
        }
        debug.model.source = "Capability showcase".into();
        debug.model.host_timings_available = true;
        debug.model.frame_budget_ms = Time::frame_period_ms();
        match pollster::block_on(DebugWindow::new(event_loop)) {
            Ok(window) => {
                debug.window = Some(window);
                debug.last_frame = Instant::now();
                debug.model.event(
                    EventLevel::Info,
                    "Diagnostics connected to capability showcase",
                );
                debug.model.gpu_available = self
                    .renderer
                    .as_ref()
                    .is_some_and(Renderer::gpu_timing_available);
                if !debug.model.gpu_available {
                    debug.model.event(
                        EventLevel::Warning,
                        "GPU timestamps unavailable; CPU telemetry remains active",
                    );
                }
            }
            Err(error) => {
                log::error!("Could not open diagnostics: {error}");
                debug.model.event(
                    EventLevel::Error,
                    format!("Could not open diagnostics: {error}. Press F3 to retry."),
                );
            }
        }
        self.refresh_debug_inspection();
    }

    pub(super) fn debug_event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        let Some(debug) = &mut self.debug else {
            return false;
        };
        let Some(window) = &mut debug.window else {
            return false;
        };
        if window.id() != id {
            return false;
        }
        if window.on_event(event) {
            debug.window = None;
            debug.model.event(
                EventLevel::Info,
                "Diagnostics window closed; F3 in showcase reopens it",
            );
            return true;
        }
        let commands = if matches!(event, WindowEvent::RedrawRequested) {
            debug.ui_frames += 1;
            window.draw(&mut debug.model)
        } else {
            Vec::new()
        };
        for command in commands {
            self.apply_debug_command(command);
        }
        true
    }

    pub(super) fn request_debug_redraw(&mut self) {
        if let Some(debug) = &mut self.debug
            && debug.last_redraw.elapsed() >= Duration::from_millis(50)
        {
            if let Some(window) = &debug.window {
                window.request_redraw();
            }
            debug.last_redraw = Instant::now();
        }
    }

    pub(super) fn record_debug_event(&mut self, event: &WindowEvent) {
        let Some(debug) = &mut self.debug else {
            return;
        };
        let message = match event {
            WindowEvent::KeyboardInput { event, .. } if !event.repeat => {
                format!("Input {:?} {:?}", event.physical_key, event.state)
            }
            WindowEvent::MouseInput { button, state, .. } => {
                format!("Input mouse {button:?} {state:?}")
            }
            WindowEvent::MouseWheel { delta, .. } => format!("Input wheel {delta:?}"),
            WindowEvent::Focused(focused) => format!("Showcase focus: {focused}"),
            WindowEvent::Resized(size) => {
                format!("Showcase surface: {} x {}", size.width, size.height)
            }
            WindowEvent::Occluded(occluded) => format!("Showcase occluded: {occluded}"),
            _ => return,
        };
        debug.model.event(EventLevel::Info, message);
    }

    pub(super) fn record_debug_frame(&mut self, frame: RenderFrame, sim_ms: f32) {
        let Some(debug) = &mut self.debug else {
            return;
        };
        if let Some(logs) = &debug.logs {
            logs.drain_into(&mut debug.model);
        }
        let now = Instant::now();
        let interval_ms = now.duration_since(debug.last_frame).as_secs_f32() * 1000.0;
        debug.last_frame = now;
        debug.model.record_frame(fate_debug::FrameSample {
            tick: self.sim.tick(),
            interval_ms,
            sim_ms,
            prepare_ms: frame.draw_list_ms,
            submit_ms: frame.submit_ms,
            record_ms: frame.record_ms,
            verify_ms: self.last_verify_ms,
            instances: frame.drawn,
            draw_calls: frame.draw_calls,
            visible: frame.visible,
            candidates: scene::ARENA.len() + self.sim.tracked_count(),
            submitted: frame.rendered,
            ..Default::default()
        });
        if let Some(renderer) = &mut self.renderer {
            for sample in renderer.poll_gpu_timings() {
                debug.model.record_gpu(GpuSample {
                    id: sample.sample_id,
                    scene_ms: sample.scene.as_secs_f32() * 1000.0,
                    overlay_ms: sample.overlay.map(|d| d.as_secs_f32() * 1000.0),
                });
            }
        }
        for (index, (name, result)) in [
            ("Replay", &self.verification.replay_result),
            ("Rollback", &self.verification.rollback_result),
            ("Snapshot", &self.verification.snapshot_result),
        ]
        .into_iter()
        .enumerate()
        {
            let label = result.label();
            if debug.verify_labels[index] != label {
                debug.verify_labels[index] = label.into();
                let level = if label == "failed" {
                    EventLevel::Error
                } else {
                    EventLevel::Info
                };
                debug
                    .model
                    .event(level, format!("{name} verification: {result:?}"));
            }
        }
        if debug.last_inspection.elapsed() >= Duration::from_millis(100) {
            debug.last_inspection = now;
            self.refresh_debug_inspection();
        }
    }

    fn refresh_debug_inspection(&mut self) {
        if self.debug.is_none() {
            return;
        }
        let mut inspection = Inspection::default();
        let world = self.sim.world_diagnostics();
        inspection.fact(
            Page::Overview,
            "Simulation",
            if self.paused { "Paused" } else { "Running" },
        );
        inspection.fact(Page::Overview, "Fixed tick", self.sim.tick());
        inspection.fact(Page::Overview, "Entities", world.live_entities);
        inspection.fact(
            Page::Overview,
            "Replay / rollback",
            format!(
                "{} / {}",
                self.replay_label(),
                self.verification.rollback_result.label()
            ),
        );
        inspection.fact(
            Page::Overview,
            "Snapshot check",
            self.verification.snapshot_result.label(),
        );
        inspection.fact(
            Page::Overview,
            "State hash",
            format!("{:016x}", self.sim.state_hash()),
        );
        inspection.fact(Page::Simulation, "Tick", self.sim.tick());
        inspection.fact(
            Page::Simulation,
            "Fixed rate",
            format!("{SIMULATION_HZ} Hz"),
        );
        inspection.fact(
            Page::Simulation,
            "State hash",
            format!("{:016x}", self.sim.state_hash()),
        );
        inspection.fact(
            Page::Simulation,
            "Live / capacity / free",
            format!(
                "{} / {} / {}",
                world.live_entities, world.capacity, world.free_slots
            ),
        );
        inspection.fact(Page::Simulation, "Position components", world.positions);
        inspection.fact(
            Page::Simulation,
            "Fixed-step backlog",
            self.time.pending_fixed_steps(),
        );
        inspection.fact(
            Page::Simulation,
            "Replay",
            format!("{:?}", self.verification.replay_result),
        );
        inspection.fact(
            Page::Simulation,
            "Rollback",
            format!("{:?}", self.verification.rollback_result),
        );
        inspection.fact(
            Page::Simulation,
            "Snapshot",
            format!("{:?}", self.verification.snapshot_result),
        );
        inspection.controls.extend([
            Control::toggle(Page::Simulation, "pause", "Pause simulation", self.paused),
            Control {
                enabled: self.paused,
                ..Control::action(Page::Simulation, "step", "Advance one fixed tick")
            },
            Control::toggle(
                Page::Simulation,
                "replay",
                "Replay verification",
                self.verification.replay_enabled,
            ),
            Control::action(
                Page::Simulation,
                "snapshot-check",
                "Verify snapshot round-trip",
            ),
            Control::action(Page::Simulation, "reset-sim", "Reset showcase simulation"),
            Control::toggle(
                Page::Simulation,
                "verbose",
                "Verbose console diagnostics",
                self.diagnostics.sink().is_verbose(),
            ),
            Control::toggle(Page::Renderer, "culling", "Frustum culling", self.culling),
            Control::number(
                Page::Renderer,
                "instance-limit",
                "Instance submission limit",
                self.instance_limit as f64,
                0.0..=Renderer::max_instance_budget() as f64,
            ),
            Control::action(
                Page::Capture,
                "export-state",
                "Save simulation snapshot (.bin)",
            ),
        ]);
        if let Some(renderer) = &self.renderer {
            let info = renderer.gpu.adapter.get_info();
            let limits = renderer.gpu.device.limits();
            inspection.fact(Page::Renderer, "Adapter", &info.name);
            inspection.fact(
                Page::Renderer,
                "Backend / device",
                format!("{:?} / {:?}", info.backend, info.device_type),
            );
            inspection.fact(
                Page::Renderer,
                "Driver",
                format!("{} {}", info.driver, info.driver_info),
            );
            inspection.fact(
                Page::Renderer,
                "Surface",
                format!(
                    "{} x {} / {:?}",
                    renderer.gpu.config.width,
                    renderer.gpu.config.height,
                    renderer.gpu.config.format
                ),
            );
            inspection.fact(
                Page::Renderer,
                "Presentation",
                format!("{:?}", renderer.gpu.config.present_mode),
            );
            inspection.fact(
                Page::Renderer,
                "GPU timestamps",
                if renderer.gpu_timing_available() {
                    "Available / asynchronous"
                } else {
                    "Unavailable"
                },
            );
            inspection.fact(
                Page::Renderer,
                "Instance budget",
                Renderer::max_instance_budget(),
            );
            inspection.fact(
                Page::Renderer,
                "Max texture dimension",
                limits.max_texture_dimension_2d,
            );
            inspection.fact(
                Page::Renderer,
                "Max buffer size",
                format!("{} MiB", limits.max_buffer_size / (1024 * 1024)),
            );
            inspection.fact(
                Page::Renderer,
                "Host architecture",
                format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
            );
            let camera = renderer.camera_3d;
            inspection.fact(
                Page::Camera,
                "Mode",
                if self.orbit_mode { "Orbit" } else { "Free" },
            );
            inspection.fact(
                Page::Camera,
                "Position",
                format!(
                    "{:.3}, {:.3}, {:.3}",
                    camera.position.x, camera.position.y, camera.position.z
                ),
            );
            let direction = camera.forward();
            inspection.fact(
                Page::Camera,
                "Forward",
                format!("{:.3}, {:.3}, {:.3}", direction.x, direction.y, direction.z),
            );
            inspection.fact(
                Page::Camera,
                "Projection",
                format!(
                    "FOV {:.1} deg / aspect {:.3}",
                    camera.fov_y_radians.to_degrees(),
                    renderer.aspect()
                ),
            );
            inspection.fact(
                Page::Camera,
                "Input",
                "Tab: free/orbit; WASD: move; mouse: look; Shift: boost",
            );
            inspection.controls.extend([
                Control::toggle(Page::Camera, "orbit", "Orbit camera", self.orbit_mode),
                Control::action(Page::Camera, "reset-camera", "Reset camera"),
                Control::number(
                    Page::Camera,
                    "camera-x",
                    "Position X (switches to free camera)",
                    camera.position.x as f64,
                    -1000.0..=1000.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-y",
                    "Position Y",
                    camera.position.y as f64,
                    -1000.0..=1000.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-z",
                    "Position Z",
                    camera.position.z as f64,
                    -1000.0..=1000.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-yaw",
                    "Yaw (degrees)",
                    camera.yaw.to_degrees() as f64,
                    -360.0..=360.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-pitch",
                    "Pitch (degrees)",
                    camera.pitch.to_degrees() as f64,
                    -89.0..=89.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-fov",
                    "Vertical FOV (degrees)",
                    camera.fov_y_radians.to_degrees() as f64,
                    10.0..=120.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-near",
                    "Near clip",
                    camera.near as f64,
                    0.01..=100.0,
                ),
                Control::number(
                    Page::Camera,
                    "camera-far",
                    "Far clip",
                    camera.far as f64,
                    101.0..=10000.0,
                ),
            ]);
            let mut instances = scene::ARENA.to_vec();
            instances.extend(self.sim.render_instances(self.interpolation_alpha()));
            let visible = scene::visible_indices(&camera, renderer.aspect(), &instances);
            for (index, instance) in instances.iter().enumerate() {
                let id = if index < scene::ARENA.len() {
                    format!("Arena {index}")
                } else {
                    let entity = self.sim.tracked_entities()[index - scene::ARENA.len()];
                    format!("Entity {}:{}", entity.id(), entity.generation())
                };
                inspection.entities.push(EntityRow {
                    id,
                    position: [
                        instance.position.x,
                        instance.position.y,
                        instance.position.z,
                    ],
                    visible: visible.contains(&index),
                });
            }
        }
        self.debug.as_mut().unwrap().model.inspection = inspection;
    }

    fn apply_debug_command(&mut self, command: Command) {
        let mut outcome = format!("Tool: {}", command.id);
        let toggle = match command.value {
            ControlValue::Toggle(v) => Some(v),
            _ => None,
        };
        let number = match command.value {
            ControlValue::Number { value, .. } if value.is_finite() => Some(value),
            _ => None,
        };
        match command.id {
            "pause" => {
                if let Some(value) = toggle {
                    self.set_paused(value);
                }
            }
            "step" if self.paused => {
                self.tick_once();
                outcome = format!("Advanced to tick {}", self.sim.tick());
            }
            "replay" => {
                if let Some(value) = toggle {
                    self.verification.set_replay_enabled(value, &self.sim);
                }
            }
            "snapshot-check" => {
                self.force_snapshot_check();
                outcome = format!(
                    "Snapshot round-trip: {:?}",
                    self.verification.snapshot_result
                );
            }
            "reset-sim" => {
                self.sim = ShowcaseSim::new();
                self.verification = VerificationState::new(&self.sim);
                self.time = Time::new(SIMULATION_HZ as f32);
            }
            "verbose" => {
                if let Some(value) = toggle {
                    self.diagnostics.sink_mut().set_verbose(value);
                }
            }
            "culling" => {
                if let Some(value) = toggle {
                    self.culling = value;
                }
            }
            "instance-limit" => {
                if let Some(value) = number {
                    self.instance_limit = value
                        .clamp(0.0, Renderer::max_instance_budget() as f64)
                        .round() as usize;
                }
            }
            "orbit" => {
                if let Some(value) = toggle {
                    self.orbit_mode = value;
                    self.release_cursor();
                    self.input.clear();
                }
            }
            "reset-camera" => {
                self.orbit_angle = 0.0;
                self.orbit_mode = true;
                self.release_cursor();
                if let Some(renderer) = &mut self.renderer {
                    renderer.camera_3d = orbit_camera(0.0);
                }
            }
            "export-state" => match export_state(&self.sim) {
                Ok(path) => outcome = format!("Saved simulation snapshot: {}", path.display()),
                Err(error) => {
                    if let Some(debug) = &mut self.debug {
                        debug.model.event(EventLevel::Error, format!("Snapshot export failed: {error}. Check diagnostics-captures permissions and retry."));
                    }
                    return;
                }
            },
            id if id.starts_with("camera-") => {
                if let Some(value) = number {
                    if matches!(
                        id,
                        "camera-x" | "camera-y" | "camera-z" | "camera-yaw" | "camera-pitch"
                    ) {
                        self.orbit_mode = false;
                        self.release_cursor();
                        self.input.clear();
                    }
                    if let Some(renderer) = &mut self.renderer {
                        let camera = &mut renderer.camera_3d;
                        let v = value as f32;
                        match id {
                            "camera-x" => camera.position.x = v.clamp(-1000.0, 1000.0),
                            "camera-y" => camera.position.y = v.clamp(-1000.0, 1000.0),
                            "camera-z" => camera.position.z = v.clamp(-1000.0, 1000.0),
                            "camera-yaw" => camera.yaw = v.clamp(-360.0, 360.0).to_radians(),
                            "camera-pitch" => camera.pitch = v.clamp(-89.0, 89.0).to_radians(),
                            "camera-fov" => {
                                camera.fov_y_radians = v.clamp(10.0, 120.0).to_radians()
                            }
                            "camera-near" => {
                                camera.near = v.clamp(0.01, 100.0).min(camera.far - 0.01)
                            }
                            "camera-far" => {
                                camera.far = v.clamp(101.0, 10000.0).max(camera.near + 0.01)
                            }
                            _ => return,
                        }
                    }
                }
            }
            _ => return,
        }
        if let Some(debug) = &mut self.debug {
            debug.model.event(EventLevel::Info, outcome);
        }
        self.refresh_debug_inspection();
    }

    pub(super) fn debug_smoke_step(&mut self, event_loop: &ActiveEventLoop) {
        let Some(debug) = &mut self.debug else {
            return;
        };
        if !debug.smoke {
            return;
        }
        debug.smoke_frame += 1;
        let frame = debug.smoke_frame;
        assert!(
            debug.window.is_some(),
            "diagnostics smoke requires a native companion"
        );
        let native = debug.window.as_ref().unwrap().window();
        match frame {
            120 => native.set_minimized(true),
            140 => native.set_minimized(false),
            160 => {
                let _ = native.request_inner_size(LogicalSize::new(720.0, 520.0));
            }
            180 => {
                let _ = native.request_inner_size(LogicalSize::new(1160.0, 800.0));
            }
            _ => {}
        }
        if frame.is_multiple_of(30) {
            debug
                .window
                .as_mut()
                .unwrap()
                .select_page(Page::ALL[(frame / 30 - 1) % Page::ALL.len()]);
        }
        let command = match frame {
            40 => Some(Command {
                id: "pause",
                value: ControlValue::Toggle(true),
            }),
            45 => Some(Command {
                id: "step",
                value: ControlValue::Action,
            }),
            50 => Some(Command {
                id: "snapshot-check",
                value: ControlValue::Action,
            }),
            55 => Some(Command {
                id: "pause",
                value: ControlValue::Toggle(false),
            }),
            80 => Some(Command {
                id: "culling",
                value: ControlValue::Toggle(false),
            }),
            90 => Some(Command {
                id: "culling",
                value: ControlValue::Toggle(true),
            }),
            _ => None,
        };
        if let Some(command) = command {
            self.apply_debug_command(command);
        }
        if frame == 210 {
            self.debug.as_mut().unwrap().window = None;
            self.open_debug(event_loop);
        }
        if frame == 240 {
            let debug = self.debug.as_ref().unwrap();
            assert!(debug.ui_frames >= 7);
            assert!(debug.model.submitted > 0);
            assert!(!debug.model.frames.is_empty());
            assert_eq!(debug.model.inspection.entities.len(), 168);
            if debug.model.gpu_available {
                assert!(!debug.model.gpu.is_empty());
            }
            println!(
                "diagnostics smoke: {} scene submissions, {} UI frames, {} GPU samples, 7 pages exercised",
                debug.model.submitted,
                debug.ui_frames,
                debug.model.gpu.len()
            );
            event_loop.exit();
        }
    }
}

fn export_state(sim: &ShowcaseSim) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::path::Path::new("diagnostics-captures");
    std::fs::create_dir_all(directory)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("state-tick-{}-{stamp}.bin", sim.tick()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&sim.state_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_tools_control_real_simulation_without_a_window() {
        let mut app = App::new();
        app.debug = Some(DebugSession::new(false));
        app.apply_debug_command(Command {
            id: "pause",
            value: ControlValue::Toggle(true),
        });
        app.apply_debug_command(Command {
            id: "step",
            value: ControlValue::Action,
        });
        assert_eq!(app.sim.tick(), 1);
        assert!(app.paused);
        app.apply_debug_command(Command {
            id: "snapshot-check",
            value: ControlValue::Action,
        });
        assert_eq!(app.verification.snapshot_result.label(), "ok");
        app.apply_debug_command(Command {
            id: "culling",
            value: ControlValue::Toggle(false),
        });
        assert!(!app.culling);
        app.apply_debug_command(Command {
            id: "reset-sim",
            value: ControlValue::Action,
        });
        assert_eq!(app.sim.tick(), 0);
        assert!(app.paused);
        assert!(!app.debug.as_ref().unwrap().model.events.is_empty());
    }

    #[test]
    fn step_cannot_advance_a_running_simulation_and_numeric_tools_are_bounded() {
        let mut app = App::new();
        app.apply_debug_command(Command {
            id: "step",
            value: ControlValue::Action,
        });
        assert_eq!(app.sim.tick(), 0);
        let number = |value| ControlValue::Number {
            value,
            min: 0.0,
            max: 99999.0,
            speed: 1.0,
        };
        app.apply_debug_command(Command {
            id: "instance-limit",
            value: number(99999.0),
        });
        assert_eq!(app.instance_limit, Renderer::max_instance_budget());
        app.apply_debug_command(Command {
            id: "instance-limit",
            value: number(f64::NAN),
        });
        assert_eq!(app.instance_limit, Renderer::max_instance_budget());
        app.apply_debug_command(Command {
            id: "instance-limit",
            value: number(-10.0),
        });
        assert_eq!(app.instance_limit, 0);
    }
}
