//! Xbox and DualSense controller support.
//!
//! Pads enumerated by `gilrs` are read directly. Sony pads also have a Windows
//! fallback: when the system's game-controller stack reports nothing, the pad is
//! read through the platform HID interface instead (see `crate::sony_hid`).
//!
//! Both paths produce the same normalized [`PadState`], so deadzones, digital
//! movement, trigger thresholds, and menu edge detection are shared. Sticks use
//! `-1..=1` with positive X right and positive Y up, which lets the mapping
//! treat Y as forward and up directly. The pure helpers below are covered by
//! headless tests; the device backends are optional so those tests never open a
//! controller.

use gilrs::{Axis as PadAxis, Button, EventType, GamepadId, Gilrs};

#[cfg(target_os = "windows")]
use crate::sony_hid::SonyHid;

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

/// Vendor ID shared by Sony Interactive Entertainment controllers.
const SONY_VENDOR_ID: u16 = 0x054C;

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

/// A pad's current state, normalized across backends.
///
/// Sticks use `-1..=1` with positive X right and positive Y up; triggers use
/// `0..=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PadState {
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    /// South face button: **A** on Xbox, **Cross** on DualSense.
    pub south: bool,
    /// East face button: **B** on Xbox, **Circle** on DualSense.
    pub east: bool,
    /// West face button: **X** on Xbox, **Square** on DualSense.
    pub west: bool,
    /// North face button: **Y** on Xbox, **Triangle** on DualSense.
    pub north: bool,
    pub left_shoulder: bool,
    pub right_shoulder: bool,
    pub left_thumb: bool,
    pub right_thumb: bool,
    /// **Menu** on Xbox, **Options** on DualSense.
    pub start: bool,
    /// **View** on Xbox, **Create** on DualSense.
    pub select: bool,
}

impl PadState {
    /// Map one pad state onto the slice's command fields.
    ///
    /// `previous` carries the menu button state so that `pause`, `restart`, and
    /// `verbose` only fire on the press edge.
    fn update(self, previous: &mut MenuButtons) -> GamepadUpdate {
        let (stick_x, stick_y) = apply_deadzone(self.left_x, self.left_y, STICK_DEADZONE);
        let (look_x, look_y) = apply_deadzone(self.right_x, self.right_y, STICK_DEADZONE);

        let dpad_x = f32::from(i8::from(self.dpad_right) - i8::from(self.dpad_left));
        let dpad_y = f32::from(i8::from(self.dpad_up) - i8::from(self.dpad_down));

        let current = MenuButtons {
            pause: self.start,
            restart: self.select,
            verbose: self.north,
        };
        let edges = current.edges(*previous);
        *previous = current;

        GamepadUpdate {
            connected: true,
            move_x: direction(stick_x + dpad_x),
            move_y: direction(stick_y + dpad_y),
            look_x,
            look_y,
            sprint: self.left_trigger >= TRIGGER_THRESHOLD || self.left_shoulder || self.left_thumb,
            fire: self.right_trigger >= TRIGGER_THRESHOLD || self.right_shoulder,
            reload: self.west,
            pause: edges.pause,
            restart: edges.restart,
            verbose: edges.verbose,
        }
    }
}

/// Parse a DualSense or DualSense Edge input report.
///
/// `report_id` is the HID report ID reported by the platform and `data` is the
/// report payload. The payload usually repeats the report ID as its first byte,
/// but the platform may strip it, so both layouts are accepted.
///
/// USB reports use ID `0x01` and start with the stick axes. Bluetooth reports
/// use ID `0x31` and insert one flag byte before the axes.
pub(crate) fn parse_sony_report(report_id: u8, data: &[u8]) -> Option<PadState> {
    let axes_offset = match report_id {
        0x01 => 1,
        0x31 => 2,
        _ => return None,
    };
    let start = if data.first() == Some(&report_id) {
        axes_offset
    } else {
        axes_offset - 1
    };
    let payload = data.get(start..)?;
    // The payload needs the four stick axes, both triggers, the sequence byte,
    // the face and menu button bytes, and the D-pad byte.
    if payload.len() < 10 {
        return None;
    }

    let axis = |raw: u8| ((f32::from(raw) - 128.0) / 127.0).clamp(-1.0, 1.0);
    let trigger = |raw: u8| f32::from(raw) / 255.0;

    // `buttons0` holds the D-pad in its low nibble and the face buttons in its
    // high nibble; `buttons1` holds the shoulders, menu buttons, and stick
    // clicks.
    let buttons0 = payload[7];
    let buttons1 = payload[8];
    let (dpad_left, dpad_right, dpad_up, dpad_down) = dpad_directions(buttons0 & 0x0F);

    Some(PadState {
        left_x: axis(payload[0]),
        // HID reports Y with positive down; the slice wants positive up.
        left_y: -axis(payload[1]),
        right_x: axis(payload[2]),
        right_y: -axis(payload[3]),
        left_trigger: trigger(payload[4]),
        right_trigger: trigger(payload[5]),
        dpad_left,
        dpad_right,
        dpad_up,
        dpad_down,
        south: buttons0 & 0x20 != 0,
        east: buttons0 & 0x40 != 0,
        west: buttons0 & 0x10 != 0,
        north: buttons0 & 0x80 != 0,
        left_shoulder: buttons1 & 0x01 != 0,
        right_shoulder: buttons1 & 0x02 != 0,
        left_thumb: buttons1 & 0x40 != 0,
        right_thumb: buttons1 & 0x80 != 0,
        start: buttons1 & 0x20 != 0,
        select: buttons1 & 0x10 != 0,
    })
}

/// Decode the low nibble of a DualSense D-pad byte.
fn dpad_directions(nibble: u8) -> (bool, bool, bool, bool) {
    // 0 = up, then clockwise in 45-degree steps; 8 means neutral.
    let left = matches!(nibble, 5..=7);
    let right = matches!(nibble, 1..=3);
    let up = matches!(nibble, 0 | 1 | 7);
    let down = matches!(nibble, 3..=5);
    (left, right, up, down)
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

/// Log every pad the `gilrs` backend sees, including its mapping source.
fn log_pads(gilrs: &Gilrs) {
    let mut connected = 0;
    for (id, pad) in gilrs.gamepads() {
        connected += 1;
        log::info!(
            target: "fate.combat",
            "controller {id:?}: {} (os name {:?}, vendor {:?}, product {:?}, mapping {:?} {:?})",
            pad.name(),
            pad.os_name(),
            pad.vendor_id(),
            pad.product_id(),
            pad.mapping_source(),
            pad.map_name(),
        );
    }

    if connected > 0 {
        log::info!(
            target: "fate.combat",
            "controller support ready ({connected} connected)"
        );
        return;
    }

    #[cfg(target_os = "windows")]
    log::warn!(
        target: "fate.combat",
        "Windows reports no controllers to gilrs; if a pad is connected, check that the \
         Xbox synthetic gamepad bus is running and that no HID filter driver is blocking it"
    );
    #[cfg(not(target_os = "windows"))]
    log::warn!(
        target: "fate.combat",
        "controller support ready (0 connected)"
    );
}

/// Optional `gilrs` backend, the Windows Sony HID fallback, and the last
/// reported menu button state.
pub struct Gamepad {
    backend: Option<Gilrs>,
    active: Option<GamepadId>,
    previous: MenuButtons,
    /// Which pad produced the previous frame, so switching pads drops edges.
    origin: (bool, Option<GamepadId>),
    #[cfg(target_os = "windows")]
    sony: SonyHid,
}

impl Gamepad {
    /// Open the controller backends.
    ///
    /// A missing backend is not an error: the slice stays playable with the
    /// keyboard and mouse, so failures are logged and ignored.
    pub fn new() -> Self {
        let backend = match Gilrs::new() {
            Ok(backend) => {
                log_pads(&backend);
                Some(backend)
            }
            Err(error) => {
                log::warn!(
                    target: "fate.combat",
                    "controller support unavailable: {error}"
                );
                None
            }
        };

        Self {
            backend,
            active: None,
            previous: MenuButtons::default(),
            origin: (false, None),
            #[cfg(target_os = "windows")]
            sony: SonyHid::new(),
        }
    }

    /// A backend-less instance for tests and headless runs.
    pub fn disabled() -> Self {
        Self {
            backend: None,
            active: None,
            previous: MenuButtons::default(),
            origin: (false, None),
            #[cfg(target_os = "windows")]
            sony: SonyHid::disabled(),
        }
    }

    /// Drain pending events and read the active controller's current state.
    pub fn poll(&mut self) -> GamepadUpdate {
        #[cfg(target_os = "windows")]
        let sony_state = self.sony.poll();
        #[cfg(not(target_os = "windows"))]
        let sony_state: Option<PadState> = None;

        // A Sony pad read through the HID fallback wins over the same pad seen
        // by `gilrs`, so a working system stack never double-feeds the slice.
        let gilrs_state = self.poll_gilrs(sony_state.is_some());

        let origin = (
            sony_state.is_some(),
            gilrs_state.as_ref().map(|(id, _)| *id),
        );
        if self.origin != origin {
            self.previous = MenuButtons::default();
            self.origin = origin;
        }

        let state = sony_state.or_else(|| gilrs_state.map(|(_, state)| state));
        match state {
            Some(state) => state.update(&mut self.previous),
            None => GamepadUpdate::default(),
        }
    }

    /// Drain `gilrs` events and return the active pad's state.
    ///
    /// Sony pads are skipped when `skip_sony` is set because the HID fallback
    /// already reads them.
    fn poll_gilrs(&mut self, skip_sony: bool) -> Option<(GamepadId, PadState)> {
        let gilrs = self.backend.as_mut()?;

        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    if let Some(pad) = gilrs.connected_gamepad(event.id) {
                        log::info!(
                            target: "fate.combat",
                            "controller connected: {} (vendor {:?}, product {:?})",
                            pad.name(),
                            pad.vendor_id(),
                            pad.product_id(),
                        );
                    }
                    if self.active.is_none() {
                        self.active = Some(event.id);
                    }
                }
                EventType::Disconnected if self.active == Some(event.id) => {
                    self.active = None;
                    log::info!(target: "fate.combat", "controller disconnected");
                }
                _ => {}
            }
        }

        let usable = |id: GamepadId| -> bool {
            match gilrs.connected_gamepad(id) {
                Some(pad) => !skip_sony || pad.vendor_id() != Some(SONY_VENDOR_ID),
                None => false,
            }
        };

        let active = self
            .active
            .filter(|id| usable(*id))
            .or_else(|| gilrs.gamepads().map(|(id, _)| id).find(|id| usable(*id)));
        self.active = active;

        let id = active?;
        let pad = gilrs.gamepad(id);
        Some((id, gilrs_pad_state(&pad)))
    }
}

/// Read a `gilrs` pad into the normalized pad state.
fn gilrs_pad_state(pad: &gilrs::Gamepad<'_>) -> PadState {
    let pressed = |button: Button| pad.is_pressed(button);
    let trigger = |axis: PadAxis, button: Button| {
        let value = pad.value(axis);
        if value > 0.0 {
            value
        } else if pressed(button) {
            1.0
        } else {
            0.0
        }
    };

    PadState {
        left_x: pad.value(PadAxis::LeftStickX),
        left_y: pad.value(PadAxis::LeftStickY),
        right_x: pad.value(PadAxis::RightStickX),
        right_y: pad.value(PadAxis::RightStickY),
        left_trigger: trigger(PadAxis::LeftZ, Button::LeftTrigger2),
        right_trigger: trigger(PadAxis::RightZ, Button::RightTrigger2),
        dpad_left: pressed(Button::DPadLeft),
        dpad_right: pressed(Button::DPadRight),
        dpad_up: pressed(Button::DPadUp),
        dpad_down: pressed(Button::DPadDown),
        south: pressed(Button::South),
        east: pressed(Button::East),
        west: pressed(Button::West),
        north: pressed(Button::North),
        left_shoulder: pressed(Button::LeftTrigger),
        right_shoulder: pressed(Button::RightTrigger),
        left_thumb: pressed(Button::LeftThumb),
        right_thumb: pressed(Button::RightThumb),
        start: pressed(Button::Start),
        select: pressed(Button::Select),
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

    /// A DualSense USB report with centered sticks and every button released.
    fn dualsense_report() -> [u8; 64] {
        let mut report = [0u8; 64];
        report[0] = 0x01;
        report[1] = 128;
        report[2] = 128;
        report[3] = 128;
        report[4] = 128;
        report[5] = 0;
        report[6] = 0;
        report[7] = 0;
        // Neutral D-pad, no face buttons.
        report[8] = 0x08;
        report
    }

    #[test]
    fn dualsense_usb_report_maps_sticks_triggers_and_dpad() {
        let mut report = dualsense_report();
        report[1] = 255;
        report[2] = 0;
        report[3] = 0;
        report[4] = 255;
        report[5] = 255;
        report[6] = 255;
        // Hat index 3 is down-right.
        report[8] = 0x03;

        let state = parse_sony_report(0x01, &report).expect("usb report");

        assert!((state.left_x - 1.0).abs() < 1e-5);
        assert!((state.left_y - 1.0).abs() < 1e-5, "stick up is positive");
        assert!((state.right_x + 1.0).abs() < 1e-5);
        assert!((state.right_y + 1.0).abs() < 1e-5, "stick down is negative");
        assert!((state.left_trigger - 1.0).abs() < 1e-5);
        assert!((state.right_trigger - 1.0).abs() < 1e-5);
        assert!(state.dpad_down && state.dpad_right);
        assert!(!state.dpad_up && !state.dpad_left);
    }

    #[test]
    fn dualsense_bluetooth_report_uses_the_shifted_payload() {
        let mut report = [0u8; 78];
        report[0] = 0x31;
        report[1] = 0x02;
        report[2] = 128;
        report[3] = 0;
        report[4] = 128;
        report[5] = 128;
        report[6] = 0;
        report[7] = 0;
        report[8] = 0;
        // Cross pressed with a neutral D-pad.
        report[9] = 0x28;
        report[10] = 0x00;
        report[11] = 0x00;

        let state = parse_sony_report(0x31, &report).expect("bluetooth report");

        assert!((state.left_y - 1.0).abs() < 1e-5);
        assert!(state.south, "Cross is pressed");
        assert!(!state.west && !state.east && !state.north);
        assert!(!state.dpad_up && !state.dpad_down);
    }

    #[test]
    fn dualsense_report_accepts_a_stripped_report_id() {
        let report = dualsense_report();
        let stripped = &report[1..];
        let state = parse_sony_report(0x01, stripped).expect("stripped report");

        assert!(state.left_x.abs() < 1e-5);
        assert!(state.left_y.abs() < 1e-5);
        assert!(!state.dpad_up && !state.dpad_down);
    }

    #[test]
    fn unknown_or_short_sony_reports_are_ignored() {
        assert!(parse_sony_report(0x02, &dualsense_report()).is_none());
        assert!(parse_sony_report(0x01, &[0x01, 128, 128]).is_none());
    }

    #[test]
    fn pad_state_maps_buttons_triggers_and_menu_edges() {
        let mut previous = MenuButtons::default();
        let state = PadState {
            left_trigger: 0.9,
            right_trigger: 0.0,
            right_shoulder: true,
            west: true,
            start: true,
            select: true,
            north: true,
            left_thumb: false,
            ..PadState::default()
        };

        let update = state.update(&mut previous);
        assert!(update.sprint, "left trigger sprints");
        assert!(update.fire, "right shoulder fires");
        assert!(update.reload, "west reloads");
        assert!(update.pause && update.restart && update.verbose);
        assert_eq!(update.move_x, 0);
        assert_eq!(update.move_y, 0);

        let held = state.update(&mut previous);
        assert!(!held.pause && !held.restart && !held.verbose);
        assert!(held.fire && held.sprint && held.reload);
    }

    #[test]
    fn dpad_and_stick_combine_into_diagonal_movement() {
        let mut previous = MenuButtons::default();
        let state = PadState {
            left_x: 0.9,
            dpad_up: true,
            ..PadState::default()
        };

        let update = state.update(&mut previous);
        assert_eq!(update.move_x, 1);
        assert_eq!(update.move_y, 1);
    }
}
