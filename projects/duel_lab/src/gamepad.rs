use fate_gamepad::{PadSample, STICK_DEADZONE, apply_deadzone};

use crate::sim::{Command, FULL_AXIS};

pub(crate) fn quantize_axis(axis: f32) -> i8 {
    (axis.clamp(-1.0, 1.0) * f32::from(FULL_AXIS)).round() as i8
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MenuActions {
    pub retry: bool,
    pub save: bool,
    pub restore: bool,
    pub export: bool,
}

#[derive(Default)]
pub(crate) struct PadControls {
    previous: Option<MenuActions>,
}

impl PadControls {
    pub fn reset(&mut self) {
        self.previous = None;
    }

    pub fn map(&mut self, sample: Option<PadSample>, keyboard: Command) -> (Command, MenuActions) {
        let Some(sample) = sample else {
            self.reset();
            return (keyboard, MenuActions::default());
        };
        let state = sample.state;
        let (stick_x, stick_y) = apply_deadzone(state.left_x, state.left_y, STICK_DEADZONE);
        let (aim_x, aim_y) = apply_deadzone(state.right_x, state.right_y, STICK_DEADZONE);
        let dpad_x = i8::from(state.dpad_right) - i8::from(state.dpad_left);
        let dpad_z = i8::from(state.dpad_down) - i8::from(state.dpad_up);
        let (x, z) = if keyboard.x != 0 || keyboard.z != 0 {
            (keyboard.x, keyboard.z)
        } else if dpad_x != 0 || dpad_z != 0 {
            (dpad_x * FULL_AXIS, dpad_z * FULL_AXIS)
        } else {
            (quantize_axis(stick_x), quantize_axis(-stick_y))
        };
        let current = MenuActions {
            retry: state.start,
            save: state.left_thumb,
            restore: state.right_thumb,
            export: state.select,
        };
        let previous = self.previous.replace(current);
        let edges = match previous {
            Some(previous) if !sample.changed_source => MenuActions {
                retry: current.retry && !previous.retry,
                save: current.save && !previous.save,
                restore: current.restore && !previous.restore,
                export: current.export && !previous.export,
            },
            _ => MenuActions::default(),
        };
        (
            Command {
                x,
                z,
                aim_x: quantize_axis(aim_x),
                aim_z: quantize_axis(-aim_y),
                guard: keyboard.guard || state.left_shoulder,
                attack: keyboard.attack || state.right_shoulder,
                boost: keyboard.boost || state.north,
                dash: keyboard.dash || state.south,
            },
            edges,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fate_gamepad::PadState;

    fn sample(state: PadState) -> Option<PadSample> {
        Some(PadSample {
            state,
            changed_source: false,
        })
    }

    #[test]
    fn stick_and_dpad_map_to_arena_axes_and_ignore_drift() {
        let mut input = PadControls::default();
        let keyboard = Command::default();
        let (drift, _) = input.map(
            sample(PadState {
                left_x: 0.1,
                left_y: -0.1,
                ..Default::default()
            }),
            keyboard,
        );
        assert_eq!((drift.x, drift.z), (0, 0));
        let (stick, _) = input.map(
            sample(PadState {
                left_x: 0.9,
                left_y: 0.9,
                ..Default::default()
            }),
            keyboard,
        );
        assert!(stick.x > 0 && stick.x < FULL_AXIS);
        assert_eq!(stick.x, -stick.z);
        let (dpad, _) = input.map(
            sample(PadState {
                dpad_left: true,
                dpad_down: true,
                left_x: 1.0,
                ..Default::default()
            }),
            keyboard,
        );
        assert_eq!((dpad.x, dpad.z), (-FULL_AXIS, FULL_AXIS));
    }

    #[test]
    fn right_stick_aim_is_independent_from_movement() {
        let mut input = PadControls::default();
        let (command, _) = input.map(
            sample(PadState {
                right_y: 1.0,
                left_x: -1.0,
                ..Default::default()
            }),
            Command::default(),
        );
        assert_eq!((command.x, command.z), (-FULL_AXIS, 0));
        assert_eq!((command.aim_x, command.aim_z), (0, -FULL_AXIS));
    }

    #[test]
    fn pad_actions_merge_with_keyboard_without_fighting_its_movement() {
        let mut input = PadControls::default();
        let (merged, _) = input.map(
            sample(PadState {
                dpad_left: true,
                right_shoulder: true,
                north: true,
                south: true,
                ..Default::default()
            }),
            Command {
                x: FULL_AXIS,
                guard: true,
                ..Default::default()
            },
        );
        assert_eq!((merged.x, merged.z), (FULL_AXIS, 0));
        assert!(merged.guard && merged.attack && merged.boost && merged.dash);
        let (pad, _) = input.map(
            sample(PadState {
                left_shoulder: true,
                ..Default::default()
            }),
            Command::default(),
        );
        assert!(pad.guard && !pad.attack && !pad.boost);
    }

    #[test]
    fn menu_actions_edge_trigger_and_connection_changes_do_not_retrigger() {
        let mut input = PadControls::default();
        let keyboard = Command::default();
        let released = PadState::default();
        let pressed = PadState {
            start: true,
            left_thumb: true,
            right_thumb: true,
            select: true,
            ..released
        };
        assert_eq!(
            input.map(sample(pressed), keyboard).1,
            MenuActions::default()
        );
        input.map(sample(released), keyboard);
        assert_eq!(
            input.map(sample(pressed), keyboard).1,
            MenuActions {
                retry: true,
                save: true,
                restore: true,
                export: true,
            }
        );
        assert_eq!(
            input.map(sample(pressed), keyboard).1,
            MenuActions::default()
        );
        input.map(sample(released), keyboard);
        let switched = Some(PadSample {
            state: pressed,
            changed_source: true,
        });
        assert_eq!(input.map(switched, keyboard).1, MenuActions::default());
        let (unplugged, menu) = input.map(None, keyboard);
        assert_eq!(
            (unplugged.x, unplugged.z, menu),
            (0, 0, MenuActions::default())
        );
        assert_eq!(
            input.map(sample(pressed), keyboard).1,
            MenuActions::default()
        );
    }
}
