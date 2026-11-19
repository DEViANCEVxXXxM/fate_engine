use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fate_engine::audio::{AudioEngine, Tone};
use fate_engine::math::{Mat4, Vec2, Vec3};
use fate_engine::renderer::{Camera3d, Draw3d, Overlay, Renderer, TextDraw, UiRect, wrap_text};
use fate_engine::time::Time;
use fate_gamepad::GamepadReader;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::gamepad::{MenuActions, PadControls, quantize_axis};
use crate::models::Models;
use crate::replay::Replay;
use crate::sim::{
    ARENA, Command, Duel, FULL_AXIS, HEALTH, HZ, Outcome, Phase, SNAPSHOT_MAX, StepCue,
};
use crate::storage;

const CHECKPOINT_FILE: &str = "checkpoint-v5.dlab";
const REPLAY_FILE: &str = "last-replay-v5.drep";
const SHOT_Y: f32 = 0.22;

struct AimInput {
    pointer: Option<Vec2>,
    direction: [i8; 2],
}

impl Default for AimInput {
    fn default() -> Self {
        Self {
            pointer: None,
            direction: [0, -FULL_AXIS],
        }
    }
}

impl AimInput {
    fn resolve(
        &mut self,
        mut command: Command,
        camera: &Camera3d,
        viewport: (u32, u32),
        player: Vec2,
    ) -> Command {
        if (command.aim_x, command.aim_z) != (0, 0) {
            self.pointer = None;
            self.direction = [command.aim_x, command.aim_z];
        } else if let Some(pixel) = self.pointer
            && let Some((origin, ray)) = camera.ray_from_pixel(pixel, viewport)
            && ray.y < -0.001
        {
            let point = origin + ray * ((SHOT_Y - origin.y) / ray.y);
            let direction = (Vec2::new(point.x, point.z) - player).normalize();
            if direction != Vec2::ZERO {
                self.direction = [quantize_axis(direction.x), quantize_axis(direction.y)];
            }
        }
        [command.aim_x, command.aim_z] = self.direction;
        command
    }
}

fn arena_camera(phase: Phase) -> Camera3d {
    let close = phase == Phase::Close;
    Camera3d {
        position: if close {
            Vec3::new(0.0, 9.0, 8.0)
        } else {
            Vec3::new(0.0, 13.0, 6.0)
        },
        pitch: if close { -0.79 } else { -1.14 },
        ..Default::default()
    }
}

/// Preserve short taps that arrive between fixed ticks, without retriggering
/// a held button during catch-up or on a later redraw.
#[derive(Default)]
struct ActionInput {
    sampled: [bool; 3],
    pending: [bool; 3],
}

impl ActionInput {
    fn sample(&mut self, command: Command) {
        for (index, held) in [command.guard, command.dash, command.attack]
            .into_iter()
            .enumerate()
        {
            self.pending[index] |= held && !self.sampled[index];
            self.sampled[index] = held;
        }
    }

    fn for_tick(&mut self, mut command: Command) -> Command {
        let [guard, dash, attack] = std::mem::take(&mut self.pending);
        command.guard |= guard;
        command.dash |= dash;
        command.attack |= attack;
        command
    }
}

pub fn run(scripted: bool) {
    let mut lab = Lab::new(scripted, if scripted { None } else { storage::state_dir() });
    EventLoop::new().unwrap().run_app(&mut lab).unwrap();
    if scripted {
        assert_eq!(
            lab.duel.outcome,
            Outcome::Won,
            "scripted window run did not win"
        );
        assert!(
            lab.close_frames > 0
                && lab.wave_frames > 0
                && lab.shift_frames > 0
                && lab.shot_frames > 0,
            "a phase, transition, or shot never rendered"
        );
        assert!(
            lab.wave_warning_frames > 0
                && lab.close_warning_frames > 0
                && lab.lunge_warning_frames > 0,
            "a warning never rendered"
        );
        assert_eq!(
            lab.duel.hash(),
            crate::verify_script(),
            "windowed and headless runs must end in the same state"
        );
        let verified = Replay::decode(&lab.recording.encode())
            .expect("window generated a valid replay")
            .verify()
            .expect("window replay matches");
        assert_eq!(lab.duel.snapshot(), verified.snapshot());
        println!(
            "duel lab window: {} wave, {} transition and {} close frames submitted; {} frames showed player shots",
            lab.wave_frames, lab.shift_frames, lab.close_frames, lab.shot_frames
        );
        println!(
            "duel lab warnings: {} ranged, {} close and {} lunge frames submitted",
            lab.wave_warning_frames, lab.close_warning_frames, lab.lunge_warning_frames
        );
    }
}

struct Lab {
    scripted: bool,
    time: Time,
    duel: Duel,
    focused: bool,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    models: Option<Models>,
    audio: Option<AudioEngine>,
    keys: HashSet<KeyCode>,
    mouse_attack: bool,
    aim: AimInput,
    action_input: ActionInput,
    pad: GamepadReader,
    pad_controls: PadControls,
    state_dir: Option<PathBuf>,
    recording: Replay,
    notice: &'static str,
    close_frames: usize,
    wave_frames: usize,
    shift_frames: usize,
    shot_frames: usize,
    wave_warning_frames: usize,
    close_warning_frames: usize,
    lunge_warning_frames: usize,
}

impl Lab {
    fn new(scripted: bool, state_dir: Option<PathBuf>) -> Self {
        let duel = Duel::new();
        Self {
            scripted,
            time: Time::new(HZ as f32),
            recording: Replay::new(&duel),
            duel,
            focused: true,
            window: None,
            renderer: None,
            models: None,
            audio: None,
            keys: HashSet::new(),
            mouse_attack: false,
            aim: AimInput::default(),
            action_input: ActionInput::default(),
            pad: GamepadReader::disabled(),
            pad_controls: PadControls::default(),
            state_dir,
            notice: "",
            close_frames: 0,
            wave_frames: 0,
            shift_frames: 0,
            shot_frames: 0,
            wave_warning_frames: 0,
            close_warning_frames: 0,
            lunge_warning_frames: 0,
        }
    }

    fn command(&self) -> Command {
        let held = |key| self.keys.contains(&key);
        Command {
            x: (i8::from(held(KeyCode::KeyD)) - i8::from(held(KeyCode::KeyA))) * FULL_AXIS,
            z: (i8::from(held(KeyCode::KeyS)) - i8::from(held(KeyCode::KeyW))) * FULL_AXIS,
            guard: held(KeyCode::KeyJ),
            attack: held(KeyCode::KeyK) || self.mouse_attack,
            boost: held(KeyCode::KeyF),
            dash: held(KeyCode::Space),
            ..Default::default()
        }
    }

    fn key(&mut self, code: KeyCode, down: bool, repeat: bool, event_loop: &ActiveEventLoop) {
        if !down {
            self.keys.remove(&code);
            return;
        }
        self.keys.insert(code);
        if repeat {
            return;
        }
        match code {
            KeyCode::KeyJ => self.action_input.pending[0] = true,
            KeyCode::Space => self.action_input.pending[1] = true,
            KeyCode::KeyK => self.action_input.pending[2] = true,
            KeyCode::Escape => event_loop.exit(),
            KeyCode::KeyR => self.menu(MenuActions {
                retry: true,
                ..Default::default()
            }),
            KeyCode::F5 => self.menu(MenuActions {
                save: true,
                ..Default::default()
            }),
            KeyCode::F9 => self.menu(MenuActions {
                restore: true,
                ..Default::default()
            }),
            KeyCode::F6 => self.menu(MenuActions {
                export: true,
                ..Default::default()
            }),
            _ => {}
        }
    }

    fn menu(&mut self, actions: MenuActions) {
        if actions.retry {
            self.start_branch(Duel::new());
            self.notice = "";
        } else if actions.restore {
            self.restore_checkpoint();
        } else if actions.save {
            self.store(CHECKPOINT_FILE, &self.duel.snapshot(), "CHECKPOINT SAVED");
        } else if actions.export {
            self.store(REPLAY_FILE, &self.recording.encode(), "REPLAY EXPORTED");
        }
    }

    fn start_branch(&mut self, duel: Duel) {
        self.recording = Replay::new(&duel);
        self.duel = duel;
        self.time = Time::new(HZ as f32);
        self.action_input = ActionInput::default();
    }

    fn store(&mut self, file: &str, bytes: &[u8], notice: &'static str) {
        let Some(path) = self.state_dir.as_ref().map(|dir| dir.join(file)) else {
            self.notice = "SAVE DIRECTORY UNAVAILABLE";
            return;
        };
        match storage::write_atomic(&path, bytes) {
            Ok(()) => {
                self.notice = notice;
                println!("{}: {}", notice, path.display());
            }
            Err(error) => {
                self.notice = "FILE WRITE FAILED";
                eprintln!("{}: {error}", path.display());
            }
        }
    }

    fn restore_checkpoint(&mut self) {
        let Some(path) = self.state_dir.as_ref().map(|dir| dir.join(CHECKPOINT_FILE)) else {
            self.notice = "SAVE DIRECTORY UNAVAILABLE";
            return;
        };
        let bytes = match storage::read_bounded(&path, SNAPSHOT_MAX) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.notice = if error.kind() == std::io::ErrorKind::NotFound {
                    "NO CHECKPOINT"
                } else {
                    eprintln!("{}: {error}", path.display());
                    "CHECKPOINT READ FAILED"
                };
                return;
            }
        };
        match Duel::restore(&bytes) {
            Ok(duel) => {
                self.start_branch(duel);
                self.notice = "CHECKPOINT RESTORED";
            }
            Err(error) => {
                self.notice = "INVALID CHECKPOINT";
                eprintln!("{}: {error}", path.display());
            }
        }
    }

    fn tick(&mut self, command: Command) {
        let tick = self.duel.tick;
        let before = (
            self.duel.health,
            self.duel.target_health,
            self.duel.phase,
            self.duel.pressure,
        );
        let cue = self.duel.step(command);
        if tick != self.duel.tick {
            self.recording.record(command, &self.duel);
        }
        let frequency = if self.duel.pressure > before.3 {
            740.0
        } else if self.duel.health < before.0 {
            160.0
        } else if cue == StepCue::Parry {
            640.0
        } else if cue == StepCue::Dash {
            280.0
        } else if self.duel.phase != before.2 {
            470.0
        } else if cue == StepCue::Hit || self.duel.target_health < before.1 {
            400.0
        } else if cue == StepCue::Blocked {
            210.0
        } else if cue == StepCue::Shot {
            330.0
        } else if cue == StepCue::Volley {
            250.0
        } else if cue == StepCue::Lunge {
            180.0
        } else {
            return;
        };
        if let Some(audio) = &self.audio {
            audio.play(Tone::sine(frequency, 0.08, 0.12));
        }
    }

    fn render(&mut self) -> bool {
        let (Some(renderer), Some(models)) = (&mut self.renderer, &self.models) else {
            return false;
        };
        renderer.camera_3d = arena_camera(self.duel.phase);

        let arena_width = ARENA.to_f32() * 2.0;
        let cue = self.duel.telegraph();
        let wave_warning = self.duel.wave_warning();
        let close_warning = self.duel.close_warning();
        let lunge_warning = self.duel.lunge_warning();
        let x = self.duel.position.x.to_f32();
        let z = self.duel.position.y.to_f32();
        let opponent = Vec2::new(
            self.duel.opponent_position.x.to_f32(),
            self.duel.opponent_position.y.to_f32(),
        );
        let aim = Vec2::new(
            f32::from(self.aim.direction[0]),
            f32::from(self.aim.direction[1]),
        )
        .normalize();
        let toward_player = Vec2::new(x, z) - opponent;
        let facing = toward_player.x.atan2(toward_player.y);
        let lunge_angle = lunge_warning.map(|(from, to, _, _)| {
            let direction = to - from;
            direction.x.to_f32().atan2(direction.y.to_f32())
        });
        let feet_y = -0.06;
        let mut draws = Vec::with_capacity(
            5 + self.duel.bolts.len()
                + self.duel.shots.len()
                + wave_warning.map_or(0, |(_, count, _)| count)
                + usize::from(close_warning.is_some()) * CLOSE_RING_SEGMENTS
                + usize::from(lunge_warning.is_some()) * 3,
        );
        draws.push(Draw3d {
            mesh: &models.floor,
            model: Mat4::translation(Vec3::new(0.0, -0.24, 0.0)).mul(Mat4::scale(Vec3::new(
                arena_width,
                0.35,
                arena_width,
            ))),
            tint: [0.22, 0.25, 0.30],
        });
        if let Some((directions, count, remaining)) = wave_warning {
            for direction in &directions[..count] {
                let dx = direction.x.to_f32();
                let dz = direction.y.to_f32();
                let outer = ARENA.to_f32() / dx.abs().max(dz.abs());
                let inner = 0.6;
                let middle = (outer + inner) * 0.5;
                push_mark(
                    &mut draws,
                    models,
                    Vec2::new(dx * middle, dz * middle),
                    dx.atan2(dz),
                    Vec2::new(0.10, outer - inner),
                    remaining,
                );
            }
        }
        if let Some((radius, remaining)) = close_warning {
            let radius = radius.to_f32();
            let angle_step = std::f32::consts::TAU / CLOSE_RING_SEGMENTS as f32;
            let width = radius * (angle_step * 0.5).tan() * 1.8;
            for index in 0..CLOSE_RING_SEGMENTS {
                let angle = index as f32 * angle_step;
                let (sin, cos) = angle.sin_cos();
                push_mark(
                    &mut draws,
                    models,
                    opponent + Vec2::new(sin * radius, cos * radius),
                    angle,
                    Vec2::new(width, 0.09),
                    remaining,
                );
            }
        }
        if let Some((from, to, radius, remaining)) = lunge_warning {
            let start = Vec2::new(from.x.to_f32(), from.y.to_f32());
            let end = Vec2::new(to.x.to_f32(), to.y.to_f32());
            let axis = end - start;
            let direction = axis.normalize();
            let side = Vec2::new(direction.y, -direction.x) * radius.to_f32();
            let angle = direction.x.atan2(direction.y);
            for offset in [side, -side] {
                push_mark(
                    &mut draws,
                    models,
                    start + axis * 0.5 + offset,
                    angle,
                    Vec2::new(0.12, axis.length()),
                    remaining,
                );
            }
            push_mark(
                &mut draws,
                models,
                end,
                angle,
                Vec2::new(radius.to_f32() * 2.0, 0.12),
                remaining,
            );
        }
        draws.push(Draw3d {
            mesh: &models.opponent,
            model: Mat4::translation(Vec3::new(opponent.x, feet_y, opponent.y))
                .mul(Mat4::rotation_y(lunge_angle.unwrap_or(facing))),
            tint: match cue {
                "COUNTER NOW" | "TARGET OPEN" => [0.40, 1.0, 0.45],
                _ if wave_warning.is_some()
                    || close_warning.is_some()
                    || lunge_warning.is_some() =>
                {
                    [1.0, 0.6, 0.1]
                }
                _ => [0.85, 0.24, 0.34],
            },
        });
        let (dashing, guarding) = self.duel.defense();
        draws.push(Draw3d {
            mesh: &models.player,
            model: Mat4::translation(Vec3::new(x, feet_y, z)).mul(Mat4::rotation_y(
                if self.duel.phase == Phase::Waves {
                    aim.x.atan2(aim.y)
                } else {
                    facing + std::f32::consts::PI
                },
            )),
            tint: if dashing {
                [0.64, 1.0, 0.65]
            } else if guarding {
                [1.0, 0.67, 0.34]
            } else {
                [1.0; 3]
            },
        });
        for bolt in &self.duel.bolts {
            draws.push(Draw3d {
                mesh: &models.bolt,
                model: Mat4::translation(Vec3::new(
                    bolt.position.x.to_f32(),
                    0.22,
                    bolt.position.y.to_f32(),
                ))
                .mul(Mat4::rotation_y(self.duel.tick as f32 * 0.09)),
                tint: [1.0; 3],
            });
        }
        for shot in &self.duel.shots {
            draws.push(Draw3d {
                mesh: &models.shot,
                model: Mat4::translation(Vec3::new(
                    shot.position.x.to_f32(),
                    SHOT_Y,
                    shot.position.y.to_f32(),
                )),
                tint: [1.0; 3],
            });
        }
        if self.duel.phase == Phase::Waves && self.duel.outcome == Outcome::Active {
            for distance in [0.65, 1.05] {
                draws.push(Draw3d {
                    mesh: &models.shot,
                    model: Mat4::translation(Vec3::new(
                        x + aim.x * distance,
                        SHOT_Y,
                        z + aim.y * distance,
                    ))
                    .mul(Mat4::scale(Vec3::new(0.33, 0.33, 0.33))),
                    tint: [0.7, 0.85, 0.9],
                });
            }
        }

        let status = match self.duel.outcome {
            Outcome::Won => "WIN - R OR MENU/OPTIONS TO RETRY",
            Outcome::Lost => "LOSS - R OR MENU/OPTIONS TO RETRY",
            Outcome::Active if !self.focused => "PAUSED - FOCUS WINDOW",
            Outcome::Active => cue,
        };
        let title = format!(
            "{} | HP {}/{HEALTH} | TARGET {} | CHARGE {} | PRESSURE {}",
            match self.duel.phase {
                Phase::Waves => "WAVES",
                Phase::Shift => "TRANSITION",
                Phase::Close => "CLOSE",
            },
            self.duel.health,
            self.duel.target_health,
            self.duel.charge,
            self.duel.pressure,
        );
        let help = "WASD MOVE / MOUSE AIM / LMB OR K ATTACK / SPACE DASH / J PARRY / F BOOST / F5 SAVE / F9 RESTORE / F6 EXPORT / R RETRY";
        let pad_help = "PAD: LEFT STICK/D-PAD MOVE / RIGHT STICK AIM / RB/R1 ATTACK / A/CROSS DASH / LB/L1 PARRY / Y/TRIANGLE BOOST / L3 SAVE / R3 RESTORE / VIEW/CREATE EXPORT / MENU/OPTIONS RETRY";
        let (width, _) = renderer.surface_size();
        let panel_width = (width as f32 - 24.0).max(1.0);
        let scale = 1.5;
        let text = format!("{title}\n{status}  {}\n{help}\n{pad_help}", self.notice);
        let lines = wrap_text(&text, (panel_width - 20.0).max(1.0), scale);
        let body = lines.join("\n");
        let panels = [UiRect::new(
            Vec2::new(12.0, 12.0),
            Vec2::new(panel_width, 20.0 + 12.0 * lines.len() as f32),
            [0.04, 0.05, 0.08, 0.92],
        )];
        let labels = [TextDraw::new(Vec2::new(22.0, 22.0), &body).with_scale(scale)];
        let rendered = renderer.render_meshes_with_overlay(
            &draws,
            &Overlay {
                rects: &panels,
                text: &labels,
                ..Default::default()
            },
        );
        if rendered {
            self.wave_warning_frames += usize::from(wave_warning.is_some());
            self.close_warning_frames += usize::from(close_warning.is_some());
            self.lunge_warning_frames += usize::from(lunge_warning.is_some());
        }
        rendered
    }
}

const CLOSE_RING_SEGMENTS: usize = 24;

fn push_mark<'a>(
    draws: &mut Vec<Draw3d<'a>>,
    models: &'a Models,
    position: Vec2,
    angle: f32,
    size: Vec2,
    remaining: u32,
) {
    draws.push(Draw3d {
        mesh: &models.floor,
        model: Mat4::translation(Vec3::new(position.x, -0.045, position.y))
            .mul(Mat4::rotation_y(angle))
            .mul(Mat4::scale(Vec3::new(size.x, 0.02, size.y))),
        tint: if remaining <= 4 {
            [1.0, 0.52, 0.08]
        } else {
            [0.64, 0.34, 0.055]
        },
    });
}

impl ApplicationHandler for Lab {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("fate duel lab")
            .with_inner_size(LogicalSize::new(960, 600))
            .with_min_inner_size(LogicalSize::new(480, 320));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create duel lab window"),
        );
        let renderer =
            pollster::block_on(Renderer::new(window.clone())).expect("initialize renderer");
        self.models = Some(Models::upload(&renderer));
        self.renderer = Some(renderer);
        self.window = Some(window);
        if !self.scripted {
            self.audio = AudioEngine::new().ok();
            self.pad = GamepadReader::open();
        }
        self.time = Time::new(HZ as f32);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.keys.clear();
                self.mouse_attack = false;
                self.aim.pointer = None;
                self.pad_controls.reset();
                self.action_input = ActionInput::default();
                self.time = Time::new(HZ as f32);
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if !self.scripted => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(code, event.state.is_pressed(), event.repeat, event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } if !self.scripted => {
                self.aim.pointer = Some(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::CursorLeft { .. } => {
                self.aim.pointer = None;
                self.mouse_attack = false;
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if !self.scripted => {
                self.mouse_attack = state.is_pressed();
                self.action_input.pending[2] |= self.mouse_attack;
            }
            WindowEvent::RedrawRequested => {
                if self.scripted {
                    let command = crate::command_at(self.duel.tick + 1);
                    self.aim.direction = [command.aim_x, command.aim_z];
                    self.tick(command);
                } else if self.focused {
                    self.time.tick();
                    let keyboard = self.command();
                    let (command, menu) = self.pad_controls.map(self.pad.poll(), keyboard);
                    self.menu(menu);
                    self.action_input.sample(command);
                    for _ in 0..8 {
                        if self.time.next_fixed_step().is_none() {
                            break;
                        }
                        let command = self.action_input.for_tick(command);
                        let command = self.aim.resolve(
                            command,
                            &arena_camera(self.duel.phase),
                            self.renderer
                                .as_ref()
                                .map_or((0, 0), Renderer::surface_size),
                            Vec2::new(self.duel.position.x.to_f32(), self.duel.position.y.to_f32()),
                        );
                        self.tick(command);
                    }
                }
                if self.render() {
                    self.shot_frames += usize::from(!self.duel.shots.is_empty());
                    match self.duel.phase {
                        Phase::Close => self.close_frames += 1,
                        Phase::Waves => self.wave_frames += 1,
                        Phase::Shift => self.shift_frames += 1,
                    }
                }
                if self.scripted
                    && (self.duel.outcome != Outcome::Active || self.duel.tick >= crate::SCRIPT_END)
                {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(8),
        ));
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_action_tap_between_ticks_is_delivered_once() {
        let mut buffer = ActionInput::default();
        buffer.sample(Command {
            guard: true,
            dash: true,
            attack: true,
            ..Default::default()
        });
        buffer.sample(Command::default());
        let first = buffer.for_tick(Command::default());
        let second = buffer.for_tick(Command::default());
        assert!(first.guard && first.dash && first.attack);
        assert!(!second.guard && !second.dash && !second.attack);
    }

    #[test]
    fn mouse_aim_projects_while_a_released_stick_keeps_its_direction() {
        let sizes = (800, 600);
        let camera = arena_camera(Phase::Waves);
        let mut aim = AimInput::default();
        let player = Vec2::new(0.0, 2.0);
        aim.pointer = Some(Vec2::new(500.0, 300.0));
        let from_mouse = aim.resolve(Command::default(), &camera, sizes, player);
        assert!(from_mouse.aim_x > 0 && from_mouse.aim_z < 0);
        let pad = Command {
            aim_x: -FULL_AXIS,
            ..Default::default()
        };
        aim.resolve(pad, &camera, sizes, player);
        let released = aim.resolve(Command::default(), &camera, sizes, player);
        assert_eq!((released.aim_x, released.aim_z), (-FULL_AXIS, 0));
    }

    #[test]
    fn checkpoint_survives_a_new_session_and_its_branch_exports() {
        let dir = std::env::temp_dir().join(format!(
            "fate-duel-session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut original = Lab::new(false, Some(dir.clone()));
        for tick in 1..=crate::CHECKPOINT {
            original.tick(crate::command_at(tick));
        }
        assert!(!original.duel.bolts.is_empty());
        original.menu(MenuActions {
            save: true,
            ..Default::default()
        });
        assert_eq!(original.notice, "CHECKPOINT SAVED");
        let expected = original.duel.snapshot();
        drop(original);

        let mut resumed = Lab::new(false, Some(dir.clone()));
        resumed.menu(MenuActions {
            restore: true,
            ..Default::default()
        });
        assert_eq!(resumed.notice, "CHECKPOINT RESTORED");
        assert_eq!(resumed.duel.snapshot(), expected);
        for tick in (crate::CHECKPOINT + 1)..=crate::SCRIPT_END {
            resumed.tick(crate::command_at(tick));
        }
        resumed.menu(MenuActions {
            export: true,
            ..Default::default()
        });
        let replay = crate::verify_file(&dir.join(REPLAY_FILE)).unwrap();
        assert_eq!(replay.outcome, Outcome::Won);
        assert_eq!(replay.hash(), crate::verify_script());

        let checkpoint = dir.join(CHECKPOINT_FILE);
        storage::write_atomic(&checkpoint, b"broken").unwrap();
        resumed.menu(MenuActions {
            restore: true,
            ..Default::default()
        });
        assert_eq!(resumed.notice, "INVALID CHECKPOINT");
        assert_eq!(resumed.duel.snapshot(), replay.snapshot());

        std::fs::remove_file(dir.join(REPLAY_FILE)).unwrap();
        std::fs::remove_file(checkpoint).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
