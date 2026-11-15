//! Xbox and DualSense controller support through `gilrs`.
//!
//! `gilrs` normalizes sticks so positive Y is up on every backend, which lets
//! the mapping treat Y as forward and up directly. The pure helpers below are
//! covered by headless tests; the device backend is optional so those tests
//! never open a controller.

use gilrs::{Axis as PadAxis, Button, EventType, GamepadId, Gilrs};

/// Radial deadzone applied to both analog sticks.
pub const STICK_DEADZONE: f32 = 0.18;
/// Trigger deflection treated as a full press.
pub const TRIGGER_THRESHOLD: f32 = 0.35;
/// Stick deflection required before it counts as a digital move direction.
pub const MOVE_THRESHOLD: f32 = 0.25;

/// Look counts a fully deflected stick contributes per second.
///
/// The simulation consumes look in mouse counts at roughly 400 counts per
/// radian, so this is about 3 radians per second at full deflection.
pub const LOOK_COUNTS_PER_SECOND: f32 = 1_200.0;
/// Vertical look is slightly slower than horizontal look.
pub const PITCH_SCALE: f32 = 0.75;
/// Upper bound on the frame delta used to integrate stick look.
pub const MAX_LOOK_FRAME_DT: f32 = 0.1;

/// One frame of controller input.
///
/// `pause`, `restart`, and `verbose` are edge-triggered: they are true only on
/// the frame the button transitions from released to pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GamepadUpdate {
    pub connected: bool,
    /// Strafe axis in `-1..=1` (positive = right).
    pub move_x: i8,
    /// Forward axis in `-1..=1` (positive = forward).
    pub move_y: i8,
    /// Horizontal stick deflection in `-1..=1` (positive = right).
    pub look_x: f32,
    /// Vertical stick deflection in `-1..=1` (positive = up).
    pub look_y: f32,
    pub sprint: bool,
    pub fire: bool,
    pub reload: bool,
    pub pause: bool,
    pub restart: bool,
    pub verbose: bool,
}

impl GamepadUpdate {
    /// Convert stick deflection into look counts for this frame.
    pub fn look_counts(self, frame_dt: f32) -> (f32, f32) {
        let dt = if frame_dt.is_finite() {
            frame_dt.clamp(0.0, MAX_LOOK_FRAME_DT)
        } else {
            0.0
        };
        (
            self.look_x * LOOK_COUNTS_PER_SECOND * dt,
            self.look_y * LOOK_COUNTS_PER_SECOND * PITCH_SCALE * dt,
        )
    }
}

/// Scale a stick vector past a radial deadzone, preserving its direction.
pub fn apply_deadzone(x: f32, y: f32, deadzone: f32) -> (f32, f32) {
    if !x.is_finite() || !y.is_finite() {
        return (0.0, 0.0);
    }
    let deadzone = deadzone.clamp(0.0, 0.95);
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= deadzone || magnitude <= f32::EPSILON {
        return (0.0, 0.0);
    }
    let scaled = ((magnitude - deadzone) / (1.0 - deadzone)).min(1.0);
    let scale = scaled / magnitude;
    (x * scale, y * scale)
}

/// Convert an analog axis into a digital move direction.
pub fn direction(value: f32) -> i8 {
    if !value.is_finite() {
        return 0;
    }
    if value >= MOVE_THRESHOLD {
        1
    } else if value <= -MOVE_THRESHOLD {
        -1
    } else {
        0
    }
}

fn button_axis(pressed: bool) -> f32 {
    if pressed { 1.0 } else { 0.0 }
}

/// Menu buttons tracked for edge detection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MenuButtons {
    pause: bool,
    restart: bool,
    verbose: bool,
}

impl MenuButtons {
    /// Buttons that became pressed since `previous`.
    fn edges(self, previous: Self) -> Self {
        Self {
            pause: self.pause && !previous.pause,
            restart: self.restart && !previous.restart,
            verbose: self.verbose && !previous.verbose,
        }
    }
}

/// Optional `gilrs` backend plus the last reported button state.
pub struct Gamepad {
    backend: Option<Gilrs>,
    active: Option<GamepadId>,
    previous: MenuButtons,
}

impl Gamepad {
    /// Open the controller backend.
    ///
    /// A missing backend is not an error: the slice stays playable with the
    /// keyboard and mouse, so failures are logged and ignored.
    pub fn new() -> Self {
        match Gilrs::new() {
            Ok(backend) => {
                let connected = backend.gamepads().count();
                log::info!(
                    target: "fate.combat",
                    "controller support ready ({connected} connected)"
                );
                Self {
                    backend: Some(backend),
                    active: None,
                    previous: MenuButtons::default(),
                }
            }
            Err(error) => {
                log::warn!(
                    target: "fate.combat",
                    "controller support unavailable: {error}"
                );
                Self::disabled()
            }
        }
    }

    /// A backend-less instance for tests and headless runs.
    pub fn disabled() -> Self {
        Self {
            backend: None,
            active: None,
            previous: MenuButtons::default(),
        }
    }

    /// Drain pending events and read the active controller's current state.
    pub fn poll(&mut self) -> GamepadUpdate {
        let Some(gilrs) = self.backend.as_mut() else {
            return GamepadUpdate::default();
        };

        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    if let Some(pad) = gilrs.connected_gamepad(event.id) {
                        log::info!(
                            target: "fate.combat",
                            "controller connected: {}",
                            pad.name()
                        );
                    }
                    if self.active.is_none() {
                        self.active = Some(event.id);
                    }
                }
                EventType::Disconnected if self.active == Some(event.id) => {
                    self.active = None;
                    self.previous = MenuButtons::default();
                    log::info!(target: "fate.combat", "controller disconnected");
                }
                _ => {}
            }
        }

        let active = self
            .active
            .filter(|id| gilrs.connected_gamepad(*id).is_some())
            .or_else(|| gilrs.gamepads().next().map(|(id, _)| id));

        if self.active != active {
            self.previous = MenuButtons::default();
        }
        self.active = active;

        let Some(id) = active else {
            return GamepadUpdate::default();
        };
        let pad = gilrs.gamepad(id);

        let (stick_x, stick_y) = apply_deadzone(
            pad.value(PadAxis::LeftStickX),
            pad.value(PadAxis::LeftStickY),
            STICK_DEADZONE,
        );
        let (look_x, look_y) = apply_deadzone(
            pad.value(PadAxis::RightStickX),
            pad.value(PadAxis::RightStickY),
            STICK_DEADZONE,
        );

        let move_x = direction(
            stick_x + pad.value(PadAxis::DPadX) + button_axis(pad.is_pressed(Button::DPadRight))
                - button_axis(pad.is_pressed(Button::DPadLeft)),
        );
        let move_y = direction(
            stick_y + pad.value(PadAxis::DPadY) + button_axis(pad.is_pressed(Button::DPadUp))
                - button_axis(pad.is_pressed(Button::DPadDown)),
        );

        let fire = pad.value(PadAxis::RightZ) >= TRIGGER_THRESHOLD
            || pad.is_pressed(Button::RightTrigger)
            || pad.is_pressed(Button::RightTrigger2);
        let sprint = pad.value(PadAxis::LeftZ) >= TRIGGER_THRESHOLD
            || pad.is_pressed(Button::LeftTrigger)
            || pad.is_pressed(Button::LeftTrigger2)
            || pad.is_pressed(Button::LeftThumb);
        let reload = pad.is_pressed(Button::West);

        let current = MenuButtons {
            pause: pad.is_pressed(Button::Start),
            restart: pad.is_pressed(Button::Select),
            verbose: pad.is_pressed(Button::North),
        };
        let edges = current.edges(self.previous);
        self.previous = current;

        GamepadUpdate {
            connected: true,
            move_x,
            move_y,
            look_x,
            look_y,
            sprint,
            fire,
            reload,
            pause: edges.pause,
            restart: edges.restart,
            verbose: edges.verbose,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_zeroes_small_stick_values() {
        assert_eq!(apply_deadzone(0.0, 0.0, STICK_DEADZONE), (0.0, 0.0));
        assert_eq!(apply_deadzone(0.1, -0.1, STICK_DEADZONE), (0.0, 0.0));
        assert_eq!(apply_deadzone(f32::NAN, 0.5, STICK_DEADZONE), (0.0, 0.0));
    }

    #[test]
    fn deadzone_preserves_direction_and_rescales() {
        let (x, y) = apply_deadzone(0.5, 0.0, 0.2);
        assert!(x > 0.0 && y.abs() < 1e-6);
        assert!(x <= 1.0);

        let (x, y) = apply_deadzone(-0.5, -0.5, 0.2);
        assert!(x < 0.0 && y < 0.0);
        assert!((x - y).abs() < 1e-6);
    }

    #[test]
    fn deadzone_reaches_full_deflection_at_the_edge() {
        let (x, y) = apply_deadzone(0.0, 1.0, STICK_DEADZONE);
        assert!((y - 1.0).abs() < 1e-5);
        assert!(x.abs() < 1e-5);

        let (x, y) = apply_deadzone(1.0, 1.0, STICK_DEADZONE);
        assert!((x * x + y * y).sqrt() <= 1.0 + 1e-5);
    }

    #[test]
    fn direction_uses_a_press_threshold() {
        assert_eq!(direction(0.0), 0);
        assert_eq!(direction(0.2), 0);
        assert_eq!(direction(MOVE_THRESHOLD), 1);
        assert_eq!(direction(-MOVE_THRESHOLD), -1);
        assert_eq!(direction(f32::INFINITY), 0);
    }

    #[test]
    fn look_counts_scale_with_frame_time_and_axis() {
        let update = GamepadUpdate {
            look_x: 1.0,
            look_y: 1.0,
            ..GamepadUpdate::default()
        };
        let (yaw, pitch) = update.look_counts(1.0 / 120.0);
        assert!((yaw - LOOK_COUNTS_PER_SECOND / 120.0).abs() < 1e-3);
        assert!((pitch - LOOK_COUNTS_PER_SECOND * PITCH_SCALE / 120.0).abs() < 1e-3);

        let (yaw, _) = update.look_counts(f32::NAN);
        assert_eq!(yaw, 0.0);

        let (yaw, _) = update.look_counts(10.0);
        assert!((yaw - LOOK_COUNTS_PER_SECOND * MAX_LOOK_FRAME_DT).abs() < 1e-3);
    }

    #[test]
    fn menu_buttons_report_only_the_press_edge() {
        let released = MenuButtons::default();
        let pressed = MenuButtons {
            pause: true,
            restart: true,
            verbose: true,
        };

        let edges = pressed.edges(released);
        assert_eq!(edges, pressed);

        let held = pressed.edges(pressed);
        assert_eq!(held, MenuButtons::default());
    }

    #[test]
    fn disabled_gamepad_reports_no_input() {
        let mut gamepad = Gamepad::disabled();
        assert_eq!(gamepad.poll(), GamepadUpdate::default());
    }
}
