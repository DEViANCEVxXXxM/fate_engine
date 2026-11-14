//! Keyboard and mouse bindings for the combat slice.

use fate_engine::input::Input;
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    Sprint,
    Fire,
    Reload,
}

/// Analog axes used for mouse look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Horizontal mouse movement (positive = right).
    MouseX,
    /// Vertical mouse movement (positive = down in screen space).
    MouseY,
}

pub type CombatInput = Input<Action, KeyCode, Axis>;

pub fn new() -> CombatInput {
    let mut input = CombatInput::new();
    input.bind(Action::MoveForward, KeyCode::KeyW);
    input.bind(Action::MoveForward, KeyCode::ArrowUp);
    input.bind(Action::MoveBack, KeyCode::KeyS);
    input.bind(Action::MoveBack, KeyCode::ArrowDown);
    input.bind(Action::MoveLeft, KeyCode::KeyA);
    input.bind(Action::MoveLeft, KeyCode::ArrowLeft);
    input.bind(Action::MoveRight, KeyCode::KeyD);
    input.bind(Action::MoveRight, KeyCode::ArrowRight);
    input.bind(Action::Sprint, KeyCode::ShiftLeft);
    input.bind(Action::Sprint, KeyCode::ShiftRight);
    input.bind(Action::Fire, KeyCode::Space);
    input.bind(Action::Reload, KeyCode::KeyR);
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_map_expected_keys_to_actions() {
        let mut input = new();

        input.press(KeyCode::KeyW);
        assert!(input.action_down(Action::MoveForward));
        input.press(KeyCode::ArrowLeft);
        assert!(input.action_down(Action::MoveLeft));
        input.press(KeyCode::ShiftRight);
        assert!(input.action_down(Action::Sprint));
        input.press(KeyCode::Space);
        assert!(input.action_down(Action::Fire));
        input.press(KeyCode::KeyR);
        assert!(input.action_down(Action::Reload));
    }

    #[test]
    fn unbound_keys_do_not_trigger_actions() {
        let mut input = new();
        input.press(KeyCode::KeyQ);

        assert!(!input.action_down(Action::Fire));
        assert!(!input.action_down(Action::MoveForward));
    }
}
