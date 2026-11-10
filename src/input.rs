use std::collections::HashSet;
use std::hash::Hash;

/// Platform-independent per-frame button state with a configurable action map.
pub struct Input<Action, Button> {
    pressed: HashSet<Button>,
    just_pressed: HashSet<Button>,
    just_released: HashSet<Button>,
    just_pressed_actions: HashSet<Action>,
    just_released_actions: HashSet<Action>,
    bindings: Vec<(Action, Button)>,
}

impl<Action, Button> Default for Input<Action, Button> {
    fn default() -> Self {
        Self {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::new(),
            just_pressed_actions: HashSet::new(),
            just_released_actions: HashSet::new(),
            bindings: Vec::new(),
        }
    }
}

#[allow(dead_code)]
impl<Action, Button> Input<Action, Button>
where
    Action: Copy + Eq + Hash,
    Button: Copy + Eq + Hash,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, action: Action, button: Button) {
        self.bindings.push((action, button));
    }

    pub fn press(&mut self, button: Button) {
        if self.pressed.contains(&button) {
            return;
        }
        let newly_pressed_actions: Vec<_> = self
            .bound_actions(button)
            .into_iter()
            .filter(|action| !self.action_down(*action))
            .collect();
        self.pressed.insert(button);
        self.just_pressed.insert(button);
        self.just_pressed_actions.extend(newly_pressed_actions);
    }

    pub fn release(&mut self, button: Button) {
        if !self.pressed.contains(&button) {
            return;
        }
        let affected_actions = self.bound_actions(button);
        self.pressed.remove(&button);
        self.just_released.insert(button);
        let newly_released_actions: Vec<_> = affected_actions
            .into_iter()
            .filter(|action| !self.action_down(*action))
            .collect();
        self.just_released_actions.extend(newly_released_actions);
    }

    pub fn button_down(&self, button: Button) -> bool {
        self.pressed.contains(&button)
    }

    pub fn button_just_pressed(&self, button: Button) -> bool {
        self.just_pressed.contains(&button)
    }

    pub fn button_just_released(&self, button: Button) -> bool {
        self.just_released.contains(&button)
    }

    pub fn action_down(&self, action: Action) -> bool {
        self.bindings
            .iter()
            .any(|(bound, button)| *bound == action && self.pressed.contains(button))
    }

    pub fn action_just_pressed(&self, action: Action) -> bool {
        self.just_pressed_actions.contains(&action)
    }

    pub fn action_just_released(&self, action: Action) -> bool {
        self.just_released_actions.contains(&action)
    }

    fn bound_actions(&self, button: Button) -> Vec<Action> {
        self.bindings
            .iter()
            .filter_map(|(action, bound_button)| (*bound_button == button).then_some(*action))
            .collect()
    }

    /// Call once at the end of every frame, after systems have read the input.
    pub fn end_frame(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.just_pressed_actions.clear();
        self.just_released_actions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Action {
        ActionA,
        ActionB,
    }

    #[test]
    fn button_and_action_transitions_are_tracked() {
        let mut input = Input::new();
        input.bind(Action::ActionA, 1_u8);
        input.bind(Action::ActionB, 2_u8);

        input.press(1);
        assert!(input.button_down(1));
        assert!(input.button_just_pressed(1));
        assert!(input.action_down(Action::ActionA));
        assert!(input.action_just_pressed(Action::ActionA));

        input.end_frame();
        assert!(!input.button_just_pressed(1));
        assert!(!input.action_just_pressed(Action::ActionA));

        input.release(1);
        assert!(!input.button_down(1));
        assert!(input.button_just_released(1));
        assert!(input.action_just_released(Action::ActionA));
    }

    #[test]
    fn edge_state_is_cleared_once_per_frame() {
        let mut input = Input::new();
        input.bind(Action::ActionB, 2_u8);
        input.press(2);
        assert!(input.action_just_pressed(Action::ActionB));

        input.end_frame();
        input.end_frame();
        assert!(!input.action_just_pressed(Action::ActionB));
        assert!(!input.button_just_pressed(2));
    }

    #[test]
    fn action_edges_follow_aggregate_state_across_multiple_buttons() {
        let mut input = Input::new();
        input.bind(Action::ActionA, 1_u8);
        input.bind(Action::ActionA, 2_u8);

        input.press(1);
        assert!(input.action_just_pressed(Action::ActionA));
        input.end_frame();

        input.press(2);
        assert!(input.action_down(Action::ActionA));
        assert!(!input.action_just_pressed(Action::ActionA));
        input.release(1);
        assert!(input.action_down(Action::ActionA));
        assert!(!input.action_just_released(Action::ActionA));

        input.release(2);
        assert!(!input.action_down(Action::ActionA));
        assert!(input.action_just_released(Action::ActionA));
    }
}
