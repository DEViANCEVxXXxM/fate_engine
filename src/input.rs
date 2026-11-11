use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Platform-independent per-frame input state with a configurable action map
/// and analog axis tracking.
///
/// # Type parameters
/// - `Action` – high-level logical action (e.g. `MoveLeft`, `Jump`), mapped from
///   one or more `Button`s.
/// - `Button` – raw platform button identifier (e.g. `winit::keyboard::KeyCode`).
/// - `Axis` – analog axis identifier (e.g. `MouseX`, `MouseY`).  Defaults to
///   `()` so callers that do not need axes pay no extra cost.
pub struct Input<Action, Button, Axis = ()> {
    pressed: HashSet<Button>,
    just_pressed: HashSet<Button>,
    just_released: HashSet<Button>,
    just_pressed_actions: HashSet<Action>,
    just_released_actions: HashSet<Action>,
    bindings: Vec<(Action, Button)>,
    /// Accumulated analog deltas for the current frame.  Values are reset by
    /// [`end_frame`](Self::end_frame) and [`clear`](Self::clear).
    axes: HashMap<Axis, f32>,
}

impl<Action, Button, Axis> Default for Input<Action, Button, Axis> {
    fn default() -> Self {
        Self {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::new(),
            just_pressed_actions: HashSet::new(),
            just_released_actions: HashSet::new(),
            bindings: Vec::new(),
            axes: HashMap::new(),
        }
    }
}

#[allow(dead_code)]
impl<Action, Button, Axis> Input<Action, Button, Axis>
where
    Action: Copy + Eq + Hash,
    Button: Copy + Eq + Hash,
    Axis: Copy + Eq + Hash,
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

    // ── Analog axes ─────────────────────────────────────────────────────────

    /// Accumulate `delta` onto `axis` for the current frame.
    ///
    /// Multiple calls per frame are summed together, so raw device events can
    /// be fed directly without pre-aggregation by the caller.
    pub fn update_axis(&mut self, axis: Axis, delta: f32) {
        *self.axes.entry(axis).or_insert(0.0) += delta;
    }

    /// Return the total accumulated delta for `axis` this frame.
    ///
    /// Returns `0.0` if the axis has not been updated this frame.
    pub fn axis_delta(&self, axis: Axis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Clear held buttons and pending transitions, such as when a window loses
    /// focus.  Also resets all axis deltas.
    pub fn clear(&mut self) {
        self.pressed.clear();
        self.just_pressed.clear();
        self.just_released.clear();
        self.just_pressed_actions.clear();
        self.just_released_actions.clear();
        self.axes.clear();
    }

    /// Call once at the end of every frame, after systems have read the input.
    /// Clears edge state (just-pressed / just-released) and axis deltas.
    pub fn end_frame(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.just_pressed_actions.clear();
        self.just_released_actions.clear();
        self.axes.clear();
    }

    fn bound_actions(&self, button: Button) -> Vec<Action> {
        self.bindings
            .iter()
            .filter_map(|(action, bound_button)| (*bound_button == button).then_some(*action))
            .collect()
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Axis {
        MouseX,
        MouseY,
    }

    #[test]
    fn button_and_action_transitions_are_tracked() {
        let mut input: Input<Action, u8> = Input::new();
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
        let mut input: Input<Action, u8> = Input::new();
        input.bind(Action::ActionB, 2_u8);
        input.press(2);
        assert!(input.action_just_pressed(Action::ActionB));

        input.end_frame();
        input.end_frame();
        assert!(!input.action_just_pressed(Action::ActionB));
        assert!(!input.button_just_pressed(2));
    }

    #[test]
    fn clear_releases_held_buttons_and_edges() {
        let mut input: Input<Action, u8> = Input::new();
        input.bind(Action::ActionA, 1_u8);
        input.press(1);

        input.clear();

        assert!(!input.button_down(1));
        assert!(!input.button_just_pressed(1));
        assert!(!input.button_just_released(1));
        assert!(!input.action_down(Action::ActionA));
        assert!(!input.action_just_pressed(Action::ActionA));
        assert!(!input.action_just_released(Action::ActionA));
    }

    #[test]
    fn action_edges_follow_aggregate_state_across_multiple_buttons() {
        let mut input: Input<Action, u8> = Input::new();
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

    // ── Analog axis tests ────────────────────────────────────────────────────

    #[test]
    fn axis_delta_is_zero_when_not_updated() {
        let input: Input<Action, u8, Axis> = Input::new();
        assert_eq!(input.axis_delta(Axis::MouseX), 0.0);
        assert_eq!(input.axis_delta(Axis::MouseY), 0.0);
    }

    #[test]
    fn axis_deltas_accumulate_within_a_frame() {
        let mut input: Input<Action, u8, Axis> = Input::new();
        input.update_axis(Axis::MouseX, 3.0);
        input.update_axis(Axis::MouseX, -1.5);
        input.update_axis(Axis::MouseY, 7.0);

        assert!((input.axis_delta(Axis::MouseX) - 1.5).abs() < f32::EPSILON);
        assert!((input.axis_delta(Axis::MouseY) - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn end_frame_resets_axis_deltas() {
        let mut input: Input<Action, u8, Axis> = Input::new();
        input.update_axis(Axis::MouseX, 42.0);
        input.end_frame();

        assert_eq!(input.axis_delta(Axis::MouseX), 0.0);
    }

    #[test]
    fn clear_resets_axis_deltas() {
        let mut input: Input<Action, u8, Axis> = Input::new();
        input.update_axis(Axis::MouseY, -10.0);
        input.clear();

        assert_eq!(input.axis_delta(Axis::MouseY), 0.0);
    }

    #[test]
    fn axes_and_buttons_are_independent() {
        let mut input: Input<Action, u8, Axis> = Input::new();
        input.bind(Action::ActionA, 1_u8);
        input.press(1);
        input.update_axis(Axis::MouseX, 5.0);

        // button state unaffected by axis update
        assert!(input.button_down(1));
        assert!((input.axis_delta(Axis::MouseX) - 5.0).abs() < f32::EPSILON);

        input.end_frame();
        // button still held; axis reset
        assert!(input.button_down(1));
        assert_eq!(input.axis_delta(Axis::MouseX), 0.0);
    }
}
