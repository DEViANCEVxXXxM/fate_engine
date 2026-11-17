//! Windowed playable slice: fixed-step combat, first-person cubes, replay and
//! snapshot verification, and a text HUD.
//!
//! Player and verification state is drawn in the window with the engine's text
//! renderer. The window title and `log` output are kept as a secondary
//! presentation so the slice still reports state without a HUD.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::combat_input::{self, Action, Axis, CombatInput};
use crate::gamepad::{Gamepad, GamepadUpdate};
use crate::interpolation::Interpolation;
use crate::scene;
use crate::sim::{
    MAGAZINE_SIZE, MAX_LOOK_INPUT, MOVE_SCALE_FULL, Outcome, SIMULATION_HZ, Sim, SimInput,
};
use crate::verification::Verification;
use fate_engine::audio::{AudioEngine, Tone};
use fate_engine::math::Vec2;
use fate_engine::renderer::{Mesh3d, Renderer, TextDraw, text_width};
use fate_engine::time::Time;

const TITLE_INTERVAL: f32 = 0.5;
const STATUS_LOG_INTERVAL: f32 = 2.0;
const MAX_PENDING_LOOK: f32 = MAX_LOOK_INPUT as f32 * 4.0;

/// Distance from the window edge to the HUD in physical pixels.
const HUD_MARGIN: f32 = 12.0;
/// Extra space between HUD lines beyond the font line height.
const HUD_LINE_GAP: f32 = 4.0;
const HUD_TEXT: [f32; 4] = [0.92, 0.95, 1.0, 1.0];
const HUD_DIM: [f32; 4] = [0.62, 0.68, 0.78, 1.0];
const HUD_ALERT: [f32; 4] = [1.0, 0.42, 0.38, 1.0];
const HUD_GOOD: [f32; 4] = [0.55, 1.0, 0.62, 1.0];
const HUD_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.72];
/// Down-right offset of the drop shadow, in physical pixels.
const HUD_SHADOW_OFFSET: f32 = 2.0;

/// One line of the on-screen HUD, in physical pixels.
#[derive(Clone, Debug)]
struct HudLine {
    x: f32,
    y: f32,
    text: String,
    scale: f32,
    color: [f32; 4],
}

impl HudLine {
    fn draw(&self) -> TextDraw<'_> {
        TextDraw::new(Vec2::new(self.x, self.y), &self.text)
            .with_scale(self.scale)
            .with_color(self.color)
    }

    /// A dark copy of the line offset down-right, drawn under the text so the
    /// HUD stays readable over bright or busy scene colors.
    fn shadow(&self, offset: f32) -> TextDraw<'_> {
        TextDraw::new(Vec2::new(self.x + offset, self.y + offset), &self.text)
            .with_scale(self.scale)
            .with_color(HUD_SHADOW)
    }
}

fn push_hud(
    lines: &mut Vec<HudLine>,
    y: &mut f32,
    x: f32,
    text: String,
    scale: f32,
    color: [f32; 4],
) {
    lines.push(HudLine {
        x,
        y: *y,
        text,
        scale,
        color,
    });
    *y += 8.0 * scale + HUD_LINE_GAP;
}

/// A horizontally centered HUD line at `y`.
fn centered_hud(surface_width: u32, text: String, y: f32, scale: f32, color: [f32; 4]) -> HudLine {
    let width = text_width(&text, scale);
    HudLine {
        x: ((surface_width as f32 - width) * 0.5).max(0.0),
        y,
        text,
        scale,
        color,
    }
}

const HELP_COLUMN_GAP: f32 = 40.0;
/// Extra space between help rows beyond the font line height.
const HELP_ROW_GAP: f32 = 4.0;
/// Help rows shrink below this fraction of the nominal scale only when the
/// surface is too narrow to fit both columns.
const HELP_MIN_SCALE: f32 = 0.6;

/// The controls overlay: a centered heading over two DPI-scaled columns.
///
/// The left column lists movement and combat bindings, the right column lists
/// system controls. When the surface is too narrow the row scale shrinks so the
/// columns still fit inside the margins, which keeps the overlay readable in a
/// small window. The block is positioned above the pause banner.
fn help_lines(surface_width: u32, surface_height: u32, dpi: f32) -> Vec<HudLine> {
    const LEFT_TITLE: &str = "MOVE + FIGHT";
    const LEFT: [&str; 8] = [
        "WASD / ARROWS  MOVE",
        "MOUSE  LOOK",
        "SHIFT  SPRINT",
        "SPACE / LMB  FIRE",
        "R  RELOAD",
        "PAD STICK  MOVE + LOOK",
        "PAD RT  FIRE",
        "PAD X  RELOAD",
    ];
    const RIGHT_TITLE: &str = "SYSTEM";
    const RIGHT: [&str; 8] = [
        "P / MENU  PAUSE",
        "M  MUTE AUDIO",
        "H  HIDE HELP",
        "F1 / Y  VERBOSE",
        "F2  REPLAY CHECK",
        "F4  SNAPSHOT CHECK",
        "ENTER  RESTART",
        "ESC  RELEASE / QUIT",
    ];

    let margin = HUD_MARGIN * dpi;
    let heading_scale = 2.5 * dpi;
    let mut row_scale = 2.0 * dpi;
    let column_gap = HELP_COLUMN_GAP * dpi;
    let available = (surface_width as f32 - margin * 2.0).max(1.0);
    let column_width = |title: &str, rows: &[&str], scale: f32| {
        rows.iter().fold(text_width(title, scale), |width, row| {
            width.max(text_width(row, scale))
        })
    };

    let mut left_width = column_width(LEFT_TITLE, &LEFT, row_scale);
    let mut right_width = column_width(RIGHT_TITLE, &RIGHT, row_scale);
    let natural = left_width + column_gap + right_width;
    if natural > available {
        let fitted = (available - column_gap).max(1.0) / (left_width + right_width).max(1.0);
        row_scale *= fitted.clamp(HELP_MIN_SCALE, 1.0);
        left_width = column_width(LEFT_TITLE, &LEFT, row_scale);
        right_width = column_width(RIGHT_TITLE, &RIGHT, row_scale);
    }

    let total = left_width + column_gap + right_width;
    let left_x = ((surface_width as f32 - total) * 0.5).max(margin);
    let right_x = left_x + left_width + column_gap;
    let title_y = surface_height as f32 * 0.16;
    let header_y = title_y + 8.0 * heading_scale + 6.0 * dpi;
    let row_y = header_y + 8.0 * row_scale + 6.0 * dpi;
    let row_step = 8.0 * row_scale + HELP_ROW_GAP * dpi;

    let mut lines = Vec::with_capacity(2 + LEFT.len() + RIGHT.len());
    lines.push(centered_hud(
        surface_width,
        "CONTROLS".to_string(),
        title_y,
        heading_scale,
        HUD_TEXT,
    ));
    for (x, title) in [(left_x, LEFT_TITLE), (right_x, RIGHT_TITLE)] {
        lines.push(HudLine {
            x,
            y: header_y,
            text: title.to_string(),
            scale: row_scale,
            color: HUD_GOOD,
        });
    }
    for row in 0..LEFT.len().max(RIGHT.len()) {
        let y = row_y + row_step * row as f32;
        if let Some(text) = LEFT.get(row) {
            lines.push(HudLine {
                x: left_x,
                y,
                text: (*text).to_string(),
                scale: row_scale,
                color: HUD_TEXT,
            });
        }
        if let Some(text) = RIGHT.get(row) {
            lines.push(HudLine {
                x: right_x,
                y,
                text: (*text).to_string(),
                scale: row_scale,
                color: HUD_TEXT,
            });
        }
    }

    lines
}

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
    audio: Option<AudioEngine>,
    audio_muted: bool,
    help_visible: bool,
    sim: Sim,
    verification: Verification,
    interpolation: Interpolation,
    input: CombatInput,
    gamepad: Gamepad,
    gamepad_update: GamepadUpdate,
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
            audio: None,
            audio_muted: false,
            help_visible: false,
            sim,
            verification,
            interpolation,
            input: combat_input::new(),
            gamepad: Gamepad::disabled(),
            gamepad_update: GamepadUpdate::default(),
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
        self.poll_gamepad();
        self.accumulate_look(frame_dt);
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
            let feedback = FeedbackState::capture(&self.sim);
            let sim_start = Instant::now();
            self.sim.step(input);
            sim_ms += sim_start.elapsed().as_secs_f32() * 1000.0;
            self.interpolation.end_step(&self.sim);

            for event in feedback_events(feedback, &self.sim) {
                self.play_feedback(event);
            }

            let verify_start = Instant::now();
            self.verification.end_tick(&self.sim);
            verify_ms += verify_start.elapsed().as_secs_f32() * 1000.0;

            if !self.sim.outcome().is_running() {
                break;
            }
        }

        (sim_ms, verify_ms)
    }

    /// Play one presentation cue. Audio is optional and never affects the
    /// simulation, replay, or snapshot state.
    fn play_feedback(&self, event: Feedback) {
        if let Some(audio) = &self.audio {
            audio.play(tone_for(event));
        }
    }

    /// Poll the controller and apply one-shot menu actions.
    ///
    /// The backend is still drained while the window is unfocused so events do
    /// not pile up, but its input is ignored until focus returns.
    fn poll_gamepad(&mut self) {
        let update = self.gamepad.poll();
        self.apply_gamepad_update(update);
    }

    /// Apply one frame of controller input, including one-shot menu actions.
    fn apply_gamepad_update(&mut self, update: GamepadUpdate) {
        if !self.focused {
            self.gamepad_update = GamepadUpdate::default();
            return;
        }

        self.gamepad_update = update;
        if update.pause {
            self.set_paused(!self.paused);
        }
        if update.restart && !self.sim.outcome().is_running() {
            self.restart();
        }
        if update.verbose {
            self.toggle_verbose();
        }
    }

    fn build_input(&self, look: (i16, i16)) -> SimInput {
        let keyboard_x = (self.input.action_down(Action::MoveRight) as i8)
            - (self.input.action_down(Action::MoveLeft) as i8);
        let keyboard_y = (self.input.action_down(Action::MoveForward) as i8)
            - (self.input.action_down(Action::MoveBack) as i8);
        let keyboard_active = keyboard_x != 0 || keyboard_y != 0;
        let move_x = (keyboard_x as i16 + self.gamepad_update.move_x as i16).clamp(-1, 1) as i8;
        let move_y = (keyboard_y as i16 + self.gamepad_update.move_y as i16).clamp(-1, 1) as i8;
        // Keys are digital and always mean full speed; the stick keeps its
        // deflection as an analog speed fraction.
        let move_scale = if keyboard_active {
            MOVE_SCALE_FULL
        } else if self.gamepad_update.move_amount.is_finite() {
            (self.gamepad_update.move_amount.clamp(0.0, 1.0) * f32::from(MOVE_SCALE_FULL)).round()
                as i16
        } else {
            0
        };

        SimInput {
            move_x,
            move_y,
            move_scale,
            look_yaw: look.0,
            look_pitch: look.1,
            sprint: self.input.action_down(Action::Sprint) || self.gamepad_update.sprint,
            fire: self.input.action_down(Action::Fire)
                || self.mouse_fire
                || self.gamepad_update.fire,
            reload: self.input.action_down(Action::Reload) || self.gamepad_update.reload,
        }
    }

    fn accumulate_look(&mut self, frame_dt: f32) {
        if self.paused || !self.sim.outcome().is_running() {
            return;
        }

        let mouse_x = finite_or_zero(self.input.axis_delta(Axis::MouseX));
        let mouse_y = finite_or_zero(self.input.axis_delta(Axis::MouseY));
        let (pad_yaw, pad_pitch) = self.gamepad_update.look_counts(frame_dt);

        self.pending_look.0 =
            (self.pending_look.0 + mouse_x + pad_yaw).clamp(-MAX_PENDING_LOOK, MAX_PENDING_LOOK);
        // Screen-space mouse Y grows downward, while positive pitch looks up.
        // The stick already reports positive up.
        self.pending_look.1 =
            (self.pending_look.1 - mouse_y + pad_pitch).clamp(-MAX_PENDING_LOOK, MAX_PENDING_LOOK);
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

    /// Build the on-screen HUD for a surface of `surface_width` x
    /// `surface_height` physical pixels at display scale `dpi_scale`.
    ///
    /// The layout is a pure function of app state so it can be tested without a
    /// window: status lines stack down the top-left, and outcome/pause banners
    /// are centered over the scene. Sizes are in physical pixels, so
    /// `dpi_scale` converts the logical layout to a crisp size on high-DPI
    /// displays.
    fn hud_lines(&self, surface_width: u32, surface_height: u32, dpi_scale: f32) -> Vec<HudLine> {
        let dpi = if dpi_scale.is_finite() {
            dpi_scale.clamp(0.5, 4.0)
        } else {
            1.0
        };
        let margin = HUD_MARGIN * dpi;
        let gap = HUD_LINE_GAP * dpi;
        let mut lines = Vec::new();
        let mut y = margin;
        let health = self.sim.player.health;
        let health_color = if health <= 25 { HUD_ALERT } else { HUD_TEXT };

        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!("HEALTH {health}"),
            2.0 * dpi,
            health_color,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!("AMMO {}/{}", self.sim.player.ammo, MAGAZINE_SIZE),
            2.0 * dpi,
            HUD_TEXT,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "ENEMIES {}/{}",
                self.sim.enemies_alive(),
                self.sim.enemies.len()
            ),
            2.0 * dpi,
            HUD_TEXT,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!("TIME {:.0}s", remaining_seconds(&self.sim).max(0.0)),
            2.0 * dpi,
            HUD_TEXT,
        );

        y += gap;
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!("{:.0} FPS", self.time.fps()),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "SIM {:.2} DRAW {:.2} SUBMIT {:.2} VERIFY {:.2} MS",
                self.last_sim_ms, self.last_draw_list_ms, self.last_render_ms, self.last_verify_ms
            ),
            dpi,
            HUD_DIM,
        );

        y += gap;
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!("REPLAY {}", self.replay_label().to_uppercase()),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "SNAPSHOT {}",
                self.verification.snapshot_result.label().to_uppercase()
            ),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "TICK {} HASH {:016x}",
                self.sim.tick(),
                self.sim.state_hash()
            ),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "DRAWN {}/{} VISIBLE {}",
                self.last_drawn,
                Renderer::max_instance_budget(),
                self.last_visible
            ),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            format!(
                "AUDIO {}",
                audio_status(self.audio.is_some(), self.audio_muted)
            ),
            dpi,
            HUD_DIM,
        );
        push_hud(
            &mut lines,
            &mut y,
            margin,
            if self.help_visible {
                "H HIDE HELP"
            } else {
                "H HELP"
            }
            .to_string(),
            dpi,
            HUD_DIM,
        );

        if self.help_visible && self.sim.outcome().is_running() {
            lines.extend(help_lines(surface_width, surface_height, dpi));
        }

        let banner_y = surface_height as f32 * 0.34;
        match self.sim.outcome() {
            Outcome::Running => {}
            Outcome::Victory => lines.push(centered_hud(
                surface_width,
                "VICTORY".to_string(),
                banner_y,
                4.0 * dpi,
                HUD_GOOD,
            )),
            Outcome::Defeat(reason) => lines.push(centered_hud(
                surface_width,
                format!("DEFEAT ({})", reason.label().to_uppercase()),
                banner_y,
                4.0 * dpi,
                HUD_ALERT,
            )),
        }

        if self.paused {
            lines.push(centered_hud(
                surface_width,
                "PAUSED".to_string(),
                surface_height as f32 * 0.52,
                3.0 * dpi,
                HUD_TEXT,
            ));
        }

        lines
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
        let (surface_width, surface_height) = renderer.surface_size();
        let dpi_scale = self
            .window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor() as f32);
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
        let hud = self.hud_lines(surface_width, surface_height, dpi_scale);
        let shadow_offset = HUD_SHADOW_OFFSET * dpi_scale.clamp(0.5, 4.0);
        let mut overlay: Vec<TextDraw<'_>> = Vec::with_capacity(hud.len() * 2);
        overlay.extend(hud.iter().map(|line| line.shadow(shadow_offset)));
        overlay.extend(hud.iter().map(HudLine::draw));
        let draw_list_ms = draw_start.elapsed().as_secs_f32() * 1000.0;

        let submit_start = Instant::now();
        let rendered = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.render_meshes_with_text(&draws, &overlay));
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
        self.gamepad_update = GamepadUpdate::default();
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

    fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
        log::info!(
            target: "fate.combat",
            "help overlay {}",
            if self.help_visible { "shown" } else { "hidden" }
        );
    }

    fn toggle_mute(&mut self) {
        self.audio_muted = !self.audio_muted;
        self.apply_audio_volume();
        log::info!(
            target: "fate.combat",
            "audio {}",
            if self.audio_muted { "muted" } else { "unmuted" }
        );
    }

    fn apply_audio_volume(&self) {
        if let Some(audio) = &self.audio {
            audio.set_volume(if self.audio_muted { 0.0 } else { 1.0 });
        }
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
                KeyCode::KeyM => {
                    self.toggle_mute();
                    return;
                }
                KeyCode::KeyH => {
                    self.toggle_help();
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
        self.gamepad = Gamepad::new();
        self.capture_cursor();

        self.audio = match AudioEngine::new() {
            Ok(audio) => {
                audio.set_volume(if self.audio_muted { 0.0 } else { 1.0 });
                log::info!(target: "fate.combat", "audio output started");
                Some(audio)
            }
            Err(error) => {
                log::warn!(target: "fate.combat", "audio disabled: {error}");
                None
            }
        };

        log::info!(
            target: "fate.combat",
            "controls: WASD/arrows move, mouse look, Shift sprint, Space/left mouse fire, R reload, P pause, M mute, H help, F1 verbose, F2 replay, F4 snapshot, Enter restart, Escape release/quit"
        );
        log::info!(
            target: "fate.combat",
            "controller: left stick (analog speed)/d-pad move, right stick look, RT fire, LT sprint, X/Square reload, Menu/Options pause, View/Create restart after outcome, Y/Triangle verbose"
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

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn clamp_look(value: f32) -> i16 {
    if value.is_finite() {
        value.clamp(-MAX_LOOK_INPUT as f32, MAX_LOOK_INPUT as f32) as i16
    } else {
        0
    }
}

/// Simulation values that the presentation layer needs to detect new events.
///
/// Only counters and terminal state are compared; nothing here feeds back into
/// the simulation, replay tape, or snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FeedbackState {
    shots: u32,
    hits: u32,
    kills: u32,
    health: i32,
    reload_remaining: u32,
    outcome: Outcome,
}

impl FeedbackState {
    fn capture(sim: &Sim) -> Self {
        Self {
            shots: sim.shots_fired(),
            hits: sim.hits(),
            kills: sim.kills(),
            health: sim.player.health,
            reload_remaining: sim.player.reload_remaining,
            outcome: sim.outcome(),
        }
    }
}

/// A presentation cue derived from one simulation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Feedback {
    Shot,
    Hit,
    Kill,
    Damage,
    ReloadStarted,
    ReloadFinished,
    Victory,
    Defeat,
}

/// Compare two simulation states and list the cues that just became true.
fn feedback_events(previous: FeedbackState, sim: &Sim) -> Vec<Feedback> {
    let mut events = Vec::new();

    if sim.shots_fired() > previous.shots {
        events.push(Feedback::Shot);
    }
    if sim.hits() > previous.hits {
        events.push(Feedback::Hit);
    }
    if sim.kills() > previous.kills {
        events.push(Feedback::Kill);
    }
    if sim.player.health < previous.health {
        events.push(Feedback::Damage);
    }
    if previous.reload_remaining == 0 && sim.player.reload_remaining > 0 {
        events.push(Feedback::ReloadStarted);
    }
    if previous.reload_remaining > 0 && sim.player.reload_remaining == 0 {
        events.push(Feedback::ReloadFinished);
    }
    match sim.outcome() {
        Outcome::Victory if previous.outcome != Outcome::Victory => events.push(Feedback::Victory),
        Outcome::Defeat(reason) if previous.outcome != Outcome::Defeat(reason) => {
            events.push(Feedback::Defeat);
        }
        Outcome::Running | Outcome::Victory | Outcome::Defeat(_) => {}
    }

    events
}

fn audio_status(available: bool, muted: bool) -> &'static str {
    if !available {
        "UNAVAILABLE"
    } else if muted {
        "MUTED"
    } else {
        "ON"
    }
}

/// The slice's sound design: short synthesized cues, chosen by the consumer.
fn tone_for(event: Feedback) -> Tone {
    match event {
        Feedback::Shot => Tone::square(180.0, 0.05, 0.16),
        Feedback::Hit => Tone::triangle(560.0, 0.05, 0.16),
        Feedback::Kill => Tone::triangle(880.0, 0.22, 0.20),
        Feedback::Damage => Tone::noise(0.16, 0.22),
        Feedback::ReloadStarted => Tone::square(130.0, 0.04, 0.10),
        Feedback::ReloadFinished => Tone::square(260.0, 0.05, 0.12),
        Feedback::Victory => Tone::triangle(660.0, 0.45, 0.20),
        Feedback::Defeat => Tone::triangle(130.0, 0.60, 0.20),
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
        app.accumulate_look(1.0 / 120.0);
        let (_, pitch) = app.take_look();
        assert!(pitch < 0, "mouse down should pitch down");

        let mut app = App::new();
        app.input.update_axis(Axis::MouseY, -120.0);
        app.accumulate_look(1.0 / 120.0);
        let (_, pitch) = app.take_look();
        assert!(pitch > 0, "mouse up should pitch up");
    }

    #[test]
    fn gamepad_movement_and_actions_merge_with_keyboard() {
        let mut app = App::new();
        app.input.press(KeyCode::KeyW);
        app.gamepad_update = GamepadUpdate {
            move_x: 1,
            move_amount: 1.0,
            fire: true,
            sprint: true,
            reload: true,
            ..GamepadUpdate::default()
        };

        let input = app.build_input((0, 0));

        assert_eq!(input.move_x, 1);
        assert_eq!(input.move_y, 1);
        assert_eq!(input.move_scale, MOVE_SCALE_FULL);
        assert!(input.fire);
        assert!(input.sprint);
        assert!(input.reload);
    }

    #[test]
    fn gamepad_half_stick_requests_half_speed_but_keyboard_requests_full() {
        let mut app = App::new();
        app.gamepad_update = GamepadUpdate {
            move_y: 1,
            move_amount: 0.5,
            ..GamepadUpdate::default()
        };

        let pad_only = app.build_input((0, 0));
        assert_eq!(pad_only.move_y, 1);
        assert_eq!(pad_only.move_scale, MOVE_SCALE_FULL / 2);

        app.input.press(KeyCode::KeyW);
        let merged = app.build_input((0, 0));
        assert_eq!(merged.move_scale, MOVE_SCALE_FULL);
    }

    #[test]
    fn non_finite_gamepad_move_amount_requests_no_speed() {
        let mut app = App::new();
        app.gamepad_update = GamepadUpdate {
            move_y: 1,
            move_amount: f32::NAN,
            ..GamepadUpdate::default()
        };

        let input = app.build_input((0, 0));

        assert_eq!(input.move_y, 1);
        assert_eq!(input.move_scale, 0);
    }

    #[test]
    fn gamepad_stick_look_pitches_up_and_uses_frame_time() {
        let mut app = App::new();
        app.gamepad_update = GamepadUpdate {
            look_y: 1.0,
            ..GamepadUpdate::default()
        };
        app.accumulate_look(1.0 / 120.0);
        let (yaw, pitch) = app.take_look();
        assert_eq!(yaw, 0);
        assert!(pitch > 0, "stick up should pitch up");

        app.gamepad_update = GamepadUpdate {
            look_x: 1.0,
            ..GamepadUpdate::default()
        };
        app.accumulate_look(1.0 / 120.0);
        let (yaw, _) = app.take_look();
        assert!(yaw > 0, "stick right should look right");
    }

    #[test]
    fn look_is_ignored_while_paused() {
        let mut app = App::new();
        app.set_paused(true);
        app.gamepad_update = GamepadUpdate {
            look_y: 1.0,
            ..GamepadUpdate::default()
        };

        app.accumulate_look(1.0 / 120.0);

        assert_eq!(app.pending_look, (0.0, 0.0));
    }

    #[test]
    fn gamepad_pause_edge_toggles_pause() {
        let mut app = App::new();

        app.apply_gamepad_update(GamepadUpdate {
            pause: true,
            ..GamepadUpdate::default()
        });
        assert!(app.paused);

        app.apply_gamepad_update(GamepadUpdate::default());
        assert!(app.paused);
    }

    #[test]
    fn gamepad_verbose_edge_toggles_verbose() {
        let mut app = App::new();
        let verbose = app.verbose;

        app.apply_gamepad_update(GamepadUpdate {
            verbose: true,
            ..GamepadUpdate::default()
        });

        assert_eq!(app.verbose, !verbose);
    }

    #[test]
    fn gamepad_restart_requires_a_terminal_outcome() {
        let mut app = App::new();
        app.sim.step(SimInput {
            move_y: 1,
            ..SimInput::default()
        });

        app.apply_gamepad_update(GamepadUpdate {
            restart: true,
            ..GamepadUpdate::default()
        });
        assert_eq!(app.sim.tick(), 1, "a live run should not restart");

        for enemy in &mut app.sim.enemies {
            enemy.health = 0;
        }
        app.sim.step(SimInput::default());
        assert!(!app.sim.outcome().is_running());

        app.apply_gamepad_update(GamepadUpdate {
            restart: true,
            ..GamepadUpdate::default()
        });
        assert_eq!(app.sim.tick(), 0);
        assert!(app.sim.outcome().is_running());
    }

    #[test]
    fn unfocused_gamepad_input_is_ignored() {
        let mut app = App::new();
        app.focused = false;

        app.apply_gamepad_update(GamepadUpdate {
            move_y: 1,
            pause: true,
            ..GamepadUpdate::default()
        });

        assert_eq!(app.gamepad_update, GamepadUpdate::default());
        assert!(!app.paused);
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

    fn hud_text(app: &App) -> String {
        app.hud_lines(1280, 720, 1.0)
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn find_hud_line<'a>(lines: &'a [HudLine], text: &str) -> Option<&'a HudLine> {
        lines.iter().find(|line| line.text == text)
    }

    #[test]
    fn hud_reports_health_ammo_enemies_and_time() {
        let app = App::new();
        let text = hud_text(&app);

        assert!(text.contains("HEALTH 100"), "hud was:\n{text}");
        assert!(text.contains("AMMO 10/10"), "hud was:\n{text}");
        assert!(text.contains("ENEMIES 6/6"), "hud was:\n{text}");
        assert!(text.contains("TIME 90s"), "hud was:\n{text}");
        assert!(text.contains("REPLAY "), "hud was:\n{text}");
        assert!(text.contains("SNAPSHOT "), "hud was:\n{text}");
        assert!(text.contains(&format!("TICK 0 HASH {:016x}", app.sim.state_hash())));
    }

    #[test]
    fn hud_marks_low_health_and_tracks_ammo() {
        let mut app = App::new();
        app.sim.player.health = 20;
        app.sim.player.ammo = 3;

        let lines = app.hud_lines(1280, 720, 1.0);
        let health = find_hud_line(&lines, "HEALTH 20").expect("health line");
        assert_eq!(health.color, HUD_ALERT);
        assert!(hud_text(&app).contains("AMMO 3/10"));
    }

    #[test]
    fn hud_hides_pause_banner_while_running_and_centers_it_when_paused() {
        let mut app = App::new();
        assert!(find_hud_line(&app.hud_lines(1280, 720, 1.0), "PAUSED").is_none());

        app.set_paused(true);
        let lines = app.hud_lines(1280, 720, 1.0);
        let paused = find_hud_line(&lines, "PAUSED").expect("pause banner");
        let expected = (1280.0 - text_width("PAUSED", paused.scale)) * 0.5;
        assert!((paused.x - expected).abs() < 1e-3);
    }

    #[test]
    fn hud_centers_a_victory_banner_when_the_run_ends() {
        let mut app = App::new();
        for enemy in &mut app.sim.enemies {
            enemy.health = 0;
        }
        app.sim.step(SimInput::default());

        let lines = app.hud_lines(1280, 720, 1.0);
        let victory = find_hud_line(&lines, "VICTORY").expect("victory banner");
        let expected = (1280.0 - text_width("VICTORY", victory.scale)) * 0.5;
        assert!((victory.x - expected).abs() < 1e-3);
        assert_eq!(victory.color, HUD_GOOD);
    }

    #[test]
    fn hud_lines_convert_to_text_draws_without_allocating_new_strings() {
        let mut app = App::new();
        app.set_paused(true);

        let lines = app.hud_lines(800, 600, 1.0);
        let draws: Vec<TextDraw<'_>> = lines.iter().map(HudLine::draw).collect();

        assert_eq!(draws.len(), lines.len());
        assert_eq!(draws[0].text, "HEALTH 100");
        assert_eq!(draws.last().expect("pause draw").text, "PAUSED");
    }

    #[test]
    fn hud_shadow_is_offset_and_translucent() {
        let line = HudLine {
            x: 40.0,
            y: 20.0,
            text: "OK".to_string(),
            scale: 2.0,
            color: HUD_TEXT,
        };

        let shadow = line.shadow(HUD_SHADOW_OFFSET);

        assert_eq!(shadow.text, "OK");
        assert_eq!(shadow.position, Vec2::new(42.0, 22.0));
        assert_eq!(shadow.color, HUD_SHADOW);
        assert_eq!(shadow.scale, 2.0);
    }

    #[test]
    fn hud_scales_layout_for_high_dpi_surfaces() {
        let mut app = App::new();
        app.set_paused(true);

        let lines = app.hud_lines(2560, 1440, 2.0);
        let health = find_hud_line(&lines, "HEALTH 100").expect("health line");
        assert_eq!(health.x, HUD_MARGIN * 2.0);
        assert_eq!(health.y, HUD_MARGIN * 2.0);
        assert_eq!(health.scale, 4.0);

        let paused = find_hud_line(&lines, "PAUSED").expect("pause banner");
        assert_eq!(paused.scale, 6.0);
        assert_eq!(paused.y, 1440.0 * 0.52);
        let expected = (2560.0 - text_width("PAUSED", 6.0)) * 0.5;
        assert!((paused.x - expected).abs() < 1e-3);
    }

    #[test]
    fn hud_layout_falls_back_to_unscaled_for_invalid_dpi() {
        let app = App::new();

        let scaled = app.hud_lines(1280, 720, 1.0);
        let fallback = app.hud_lines(1280, 720, f32::NAN);

        assert_eq!(scaled[0].x, fallback[0].x);
        assert_eq!(scaled[0].y, fallback[0].y);
        assert_eq!(scaled[0].scale, fallback[0].scale);
    }

    #[test]
    fn audio_status_covers_unavailable_muted_and_on() {
        assert_eq!(audio_status(false, false), "UNAVAILABLE");
        assert_eq!(audio_status(false, true), "UNAVAILABLE");
        assert_eq!(audio_status(true, true), "MUTED");
        assert_eq!(audio_status(true, false), "ON");
    }

    #[test]
    fn mute_toggle_survives_restart() {
        let mut app = App::new();
        assert!(!app.audio_muted);

        app.toggle_mute();
        assert!(app.audio_muted);

        app.restart();
        assert!(app.audio_muted);
        assert!(hud_text(&app).contains("AUDIO UNAVAILABLE"));
    }

    #[test]
    fn help_is_hidden_by_default_and_hinted_in_the_hud() {
        let app = App::new();
        let text = hud_text(&app);

        assert!(!text.contains("CONTROLS"), "hud was:\n{text}");
        assert!(text.contains("H HELP"), "hud was:\n{text}");
        assert!(!text.contains("H HIDE HELP"), "hud was:\n{text}");
    }

    #[test]
    fn help_toggle_shows_two_centered_columns_above_the_pause_banner() {
        let mut app = App::new();
        app.toggle_help();

        let lines = app.hud_lines(1280, 720, 1.0);
        let title = find_hud_line(&lines, "CONTROLS").expect("help title");
        let expected = (1280.0 - text_width("CONTROLS", title.scale)) * 0.5;
        assert!((title.x - expected).abs() < 1e-3);

        let left_header = find_hud_line(&lines, "MOVE + FIGHT").expect("left header");
        let right_header = find_hud_line(&lines, "SYSTEM").expect("right header");
        assert!(left_header.x < right_header.x);

        let left_row = find_hud_line(&lines, "WASD / ARROWS  MOVE").expect("left row");
        let right_row = find_hud_line(&lines, "P / MENU  PAUSE").expect("right row");
        assert_eq!(left_row.x, left_header.x);
        assert_eq!(right_row.x, right_header.x);
        assert_eq!(left_row.y, right_row.y);
        assert!(find_hud_line(&lines, "H HIDE HELP").is_some());

        for line in &lines {
            let bottom = line.y + 8.0 * line.scale;
            assert!(
                bottom < 720.0 * 0.52,
                "help row overlaps the pause banner: {line:?}"
            );
            let right = line.x + text_width(&line.text, line.scale);
            assert!(left_row.x >= HUD_MARGIN, "help starts inside the margin");
            assert!(
                line.x >= 0.0 && right <= 1280.0,
                "help row overflows: {line:?}"
            );
        }
    }

    #[test]
    fn help_layout_scales_with_dpi() {
        let mut app = App::new();
        app.toggle_help();

        let lines = app.hud_lines(2560, 1440, 2.0);
        let title = find_hud_line(&lines, "CONTROLS").expect("help title");
        assert_eq!(title.scale, 5.0);
        assert_eq!(title.y, 1440.0 * 0.16);
        let expected = (2560.0 - text_width("CONTROLS", 5.0)) * 0.5;
        assert!((title.x - expected).abs() < 1e-3);

        let row = find_hud_line(&lines, "M  MUTE AUDIO").expect("right row");
        assert_eq!(row.scale, 4.0);
    }

    #[test]
    fn help_rows_shrink_to_fit_a_narrow_surface() {
        let mut app = App::new();
        app.toggle_help();

        let lines = app.hud_lines(360, 640, 1.0);
        let row = find_hud_line(&lines, "WASD / ARROWS  MOVE").expect("left row");
        assert!(row.scale < 2.0, "row scale did not shrink: {}", row.scale);

        for line in &lines {
            let right = line.x + text_width(&line.text, line.scale);
            assert!(line.x >= 0.0, "row starts off-screen: {line:?}");
            assert!(right <= 360.0, "row overflows the surface: {line:?}");
        }
    }

    #[test]
    fn help_is_hidden_once_the_run_ends() {
        let mut app = App::new();
        app.toggle_help();
        assert!(hud_text(&app).contains("CONTROLS"));

        for enemy in &mut app.sim.enemies {
            enemy.health = 0;
        }
        app.sim.step(SimInput::default());

        let text = hud_text(&app);
        assert!(!text.contains("CONTROLS"), "hud was:\n{text}");
        assert!(text.contains("VICTORY"), "hud was:\n{text}");
    }

    #[test]
    fn unchanged_simulation_produces_no_feedback() {
        let sim = Sim::new();
        assert!(feedback_events(FeedbackState::capture(&sim), &sim).is_empty());
    }

    #[test]
    fn shot_and_hit_feedback_follow_the_event_counters() {
        let mut sim = Sim::new();
        let before = FeedbackState::capture(&sim);

        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });
        let events = feedback_events(before, &sim);

        assert!(events.contains(&Feedback::Shot));
        assert!(events.contains(&Feedback::Hit));
        assert!(!events.contains(&Feedback::Kill));
    }

    #[test]
    fn damage_and_defeat_feedback_are_reported_together() {
        let mut sim = Sim::new();
        sim.player.health = 1;
        sim.enemies[0].position = sim.player.position;
        let before = FeedbackState::capture(&sim);

        sim.step(SimInput::default());
        let events = feedback_events(before, &sim);

        assert!(events.contains(&Feedback::Damage));
        assert!(events.contains(&Feedback::Defeat));
    }

    #[test]
    fn victory_feedback_is_reported_when_the_last_enemy_dies() {
        let mut sim = Sim::new();
        for enemy in &mut sim.enemies {
            enemy.health = 0;
        }
        let before = FeedbackState::capture(&sim);

        sim.step(SimInput::default());

        assert_eq!(sim.outcome(), Outcome::Victory);
        assert!(feedback_events(before, &sim).contains(&Feedback::Victory));
    }

    #[test]
    fn reload_feedback_reports_start_and_finish() {
        let mut sim = Sim::new();
        sim.player.ammo = 1;
        let before = FeedbackState::capture(&sim);

        sim.step(SimInput {
            reload: true,
            ..SimInput::default()
        });
        assert!(feedback_events(before, &sim).contains(&Feedback::ReloadStarted));

        let before_finish = FeedbackState::capture(&sim);
        for _ in 0..100 {
            sim.step(SimInput::default());
        }
        let events = feedback_events(before_finish, &sim);
        assert!(events.contains(&Feedback::ReloadFinished));
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE);
    }
}
