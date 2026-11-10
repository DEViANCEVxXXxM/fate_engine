use fate_engine::input::Input;
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Boost,
}

pub type DemoInput = Input<Action, KeyCode>;

pub fn new() -> DemoInput {
    let mut input = DemoInput::new();
    input.bind(Action::MoveLeft, KeyCode::KeyA);
    input.bind(Action::MoveLeft, KeyCode::ArrowLeft);
    input.bind(Action::MoveRight, KeyCode::KeyD);
    input.bind(Action::MoveRight, KeyCode::ArrowRight);
    input.bind(Action::MoveUp, KeyCode::KeyW);
    input.bind(Action::MoveUp, KeyCode::ArrowUp);
    input.bind(Action::MoveDown, KeyCode::KeyS);
    input.bind(Action::MoveDown, KeyCode::ArrowDown);
    input.bind(Action::Boost, KeyCode::ShiftLeft);
    input
}
