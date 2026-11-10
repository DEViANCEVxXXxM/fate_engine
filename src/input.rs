use std::collections::HashSet;

use winit::keyboard::KeyCode;

/// Per-frame keyboard/mouse state with an action mapping layer.
///
/// The engine reads actions (`Action::MoveLeft`, ...) instead of raw keys, so
/// rebinding later is a data change, not a code change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Boost,
}

#[derive(Default)]
pub struct Input {
    pressed: HashSet<KeyCode>,
    just_pressed: HashSet<KeyCode>,
    just_released: HashSet<KeyCode>,
    bindings: Vec<(Action, KeyCode)>,
}

#[allow(dead_code)]
impl Input {
    pub fn new() -> Self {
        let mut input = Self::default();
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

    pub fn bind(&mut self, action: Action, key: KeyCode) {
        self.bindings.push((action, key));
    }

    pub fn press(&mut self, key: KeyCode) {
        if self.pressed.insert(key) {
            self.just_pressed.insert(key);
        }
    }

    pub fn release(&mut self, key: KeyCode) {
        if self.pressed.remove(&key) {
            self.just_released.insert(key);
        }
    }

    pub fn key_down(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    pub fn key_just_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed.contains(&key)
    }

    pub fn action_down(&self, action: Action) -> bool {
        self.bindings
            .iter()
            .any(|(bound, key)| *bound == action && self.pressed.contains(key))
    }

    pub fn action_just_pressed(&self, action: Action) -> bool {
        self.bindings
            .iter()
            .any(|(bound, key)| *bound == action && self.just_pressed.contains(key))
    }

    /// Call once at the end of every frame, after systems have read the input.
    pub fn end_frame(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }
}
