//! Project-specific movement and action bindings for a normalized controller.

use fate_gamepad::{GamepadReader, PadState, STICK_DEADZONE, apply_deadzone, direction};

const TRIGGER_THRESHOLD: f32 = 0.35;
const LOOK_COUNTS_PER_SECOND: f32 = 1_200.0;
const PITCH_SCALE: f32 = 0.75;
const MAX_LOOK_FRAME_DT: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GamepadUpdate {
    pub connected: bool,
    pub move_x: i8,
    pub move_y: i8,
    pub move_amount: f32,
    pub look_x: f32,
    pub look_y: f32,
    pub sprint: bool,
    pub fire: bool,
    pub reload: bool,
    pub pause: bool,
    pub restart: bool,
    pub verbose: bool,
}

impl GamepadUpdate {
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

#[derive(Clone, Copy, Default)]
struct MenuButtons {
    pause: bool,
    restart: bool,
    verbose: bool,
}

fn map_state(state: PadState, previous: &mut MenuButtons) -> GamepadUpdate {
    let (stick_x, stick_y) = apply_deadzone(state.left_x, state.left_y, STICK_DEADZONE);
    let (look_x, look_y) = apply_deadzone(state.right_x, state.right_y, STICK_DEADZONE);
    let dpad_x = f32::from(i8::from(state.dpad_right) - i8::from(state.dpad_left));
    let dpad_y = f32::from(i8::from(state.dpad_up) - i8::from(state.dpad_down));
    let move_amount = if dpad_x != 0.0 || dpad_y != 0.0 {
        1.0
    } else {
        (stick_x * stick_x + stick_y * stick_y).sqrt().min(1.0)
    };
    let current = MenuButtons {
        pause: state.start,
        restart: state.select,
        verbose: state.north,
    };
    let last = std::mem::replace(previous, current);
    GamepadUpdate {
        connected: true,
        move_x: direction(stick_x + dpad_x),
        move_y: direction(stick_y + dpad_y),
        move_amount,
        look_x,
        look_y,
        sprint: state.left_trigger >= TRIGGER_THRESHOLD || state.left_shoulder || state.left_thumb,
        fire: state.right_trigger >= TRIGGER_THRESHOLD || state.right_shoulder,
        reload: state.west,
        pause: current.pause && !last.pause,
        restart: current.restart && !last.restart,
        verbose: current.verbose && !last.verbose,
    }
}

pub struct Gamepad {
    reader: GamepadReader,
    previous: MenuButtons,
}

impl Gamepad {
    pub fn new() -> Self {
        Self {
            reader: GamepadReader::open(),
            previous: MenuButtons::default(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            reader: GamepadReader::disabled(),
            previous: MenuButtons::default(),
        }
    }

    pub fn poll(&mut self) -> GamepadUpdate {
        let Some(sample) = self.reader.poll() else {
            self.previous = MenuButtons::default();
            return GamepadUpdate::default();
        };
        if sample.changed_source {
            self.previous = MenuButtons::default();
        }
        map_state(sample.state, &mut self.previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(update.look_counts(f32::NAN).0, 0.0);
        assert!(
            (update.look_counts(10.0).0 - LOOK_COUNTS_PER_SECOND * MAX_LOOK_FRAME_DT).abs() < 1e-3
        );
    }

    #[test]
    fn pad_state_maps_buttons_triggers_and_menu_edges() {
        let mut previous = MenuButtons::default();
        let state = PadState {
            left_trigger: 0.9,
            right_shoulder: true,
            west: true,
            start: true,
            select: true,
            north: true,
            ..Default::default()
        };
        let update = map_state(state, &mut previous);
        assert!(update.sprint && update.fire && update.reload);
        assert!(update.pause && update.restart && update.verbose);
        assert_eq!((update.move_x, update.move_y), (0, 0));

        let held = map_state(state, &mut previous);
        assert!(!held.pause && !held.restart && !held.verbose);
        assert!(held.sprint && held.fire && held.reload);
    }

    #[test]
    fn stick_speed_is_analog_and_dpad_sets_full_speed() {
        let mut previous = MenuButtons::default();
        let state = PadState {
            left_x: 0.5,
            ..Default::default()
        };
        let half = map_state(state, &mut previous);
        assert_eq!((half.move_x, half.move_y), (1, 0));
        assert!((0.0..1.0).contains(&half.move_amount) && half.move_amount > 0.0);
        let diagonal = map_state(
            PadState {
                dpad_up: true,
                ..state
            },
            &mut previous,
        );
        assert_eq!((diagonal.move_x, diagonal.move_y), (1, 1));
        assert_eq!(diagonal.move_amount, 1.0);
        let full = map_state(
            PadState {
                left_y: 1.0,
                ..Default::default()
            },
            &mut previous,
        );
        assert_eq!(full.move_y, 1);
        assert!((full.move_amount - 1.0).abs() < 1e-5);
    }

    #[test]
    fn disabled_gamepad_reports_no_input() {
        assert_eq!(Gamepad::disabled().poll(), GamepadUpdate::default());
    }
}
